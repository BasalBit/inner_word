//! A whiteboard's view and its export page (`docs/whiteboard-design.md`, sections 4.2
//! and 5): the bridge, `call_async`, one document, the whiteboard's policy, `park`,
//! `run_script` and its PDF. The bundle's own cases (Excalidraw in the view) are in
//! `bundle.rs`.

use super::*;

use innerword_web::{
    BOARD_CONTENT_SECURITY_POLICY, BOARD_EXPORT_CONTENT_SECURITY_POLICY, Bridge, ScriptInput,
    ScriptOutput, board_export_policy, check_blocking_with_policy, run_script_blocking,
};

/// The bridge of a whiteboard's view, as the app makes it (with a small cap here).
pub(super) const BRIDGE: &str = "innerwordBoard";

pub(super) fn js(text: &str) -> ServedFile {
    file("text/javascript; charset=utf-8", text.as_bytes())
}

/// A whiteboard's view with the cap `max_bytes`, the whiteboard's policy (or `csp`).
fn board_extras(max_bytes: usize, csp: Option<&'static str>) -> ViewExtras {
    ViewExtras {
        bridge: Some(Bridge {
            name: BRIDGE,
            max_bytes,
        }),
        one_document: true,
        csp: Some(csp.unwrap_or(BOARD_CONTENT_SECURITY_POLICY)),
    }
}

/// A call into the page, waited for.
pub(super) fn call(page: &WebPage, body: &str, argument: &str) -> Result<String, String> {
    let slot: Rc<RefCell<Option<Result<String, String>>>> = Rc::new(RefCell::new(None));
    let answer = slot.clone();
    page.call_async(body, argument, move |result| {
        *answer.borrow_mut() = Some(result)
    });
    testing::run_until(Duration::from_secs(20), || slot.borrow().is_some());
    let result = slot.borrow_mut().take();
    result.unwrap_or_else(|| Err("no answer in 20 s".to_string()))
}

/// The kinds of the messages a view was posted, in order.
fn kinds(events: &Events) -> Vec<String> {
    events
        .messages
        .borrow()
        .iter()
        .map(|body| {
            serde_json::from_str::<serde_json::Value>(body)
                .ok()
                .and_then(|value| value["kind"].as_str().map(str::to_string))
                .unwrap_or_else(|| format!("(unreadable: {body})"))
        })
        .collect()
}

const POST: &str = "const post = (kind, extra) => window.webkit.messageHandlers.innerwordBoard.postMessage(JSON.stringify(Object.assign({ kind }, extra || {})));\n";

pub(super) fn the_bridge(report: &mut Report, rules: &Path) {
    let host = Host::new();
    let files = vec![
        (
            "index.html",
            html(
                r#"<!doctype html><title>Board</title><body><script src="app.js"></script></body>"#,
            ),
        ),
        (
            "app.js",
            js(&format!(
                "{POST}post('ready');\n\
                 // A frame the page makes itself: about:blank, of the view's origin, loaded\n\
                 // without a navigation. A message it posts through its own handlers is a\n\
                 // subframe's.\n\
                 const frame = document.createElement('iframe');\n\
                 document.body.append(frame);\n\
                 frame.contentWindow.eval(\"window.webkit.messageHandlers.innerwordBoard.postMessage(JSON.stringify({{ kind: 'from a frame' }}))\");"
            )),
        ),
    ];
    // A page's policy, so that eval runs in the frame and its message is what is refused.
    let extras = board_extras(1024, Some(innerword_web::CONTENT_SECURITY_POLICY));
    let (view, events) = host.board("board-1-1700000000000", files, extras, rules);
    testing::run_until(Duration::from_secs(5), || {
        !events.messages.borrow().is_empty() && view.stats().messages_dropped > 0
    });
    report.check(
        kinds(&events) == ["ready"],
        &format!(
            "a message of the main frame arrives as the page posted it ({:?})",
            events.messages.borrow()
        ),
    );
    report.check(
        view.stats().messages_dropped == 1,
        &format!(
            "a message from a frame of the page is dropped and counted ({:?})",
            view.stats()
        ),
    );

    // Too large, not a string, the cap in UTF-8 bytes and not in characters.
    let before = view.stats();
    let _ = eval(
        &view,
        "const h = window.webkit.messageHandlers.innerwordBoard;\n\
         h.postMessage('x'.repeat(2000));\n\
         h.postMessage('\u{e9}'.repeat(600));\n\
         h.postMessage({ kind: 'an object' });\n\
         h.postMessage(12);\n\
         h.postMessage(JSON.stringify({ kind: 'at the cap', pad: 'y'.repeat(1024 - 30) }));\n\
         1",
    );
    testing::run_until(Duration::from_secs(3), || {
        view.stats().messages_dropped >= before.messages_dropped + 4
            && events.messages.borrow().len() >= 2
    });
    let after = view.stats();
    report.check(
        after.messages_dropped == before.messages_dropped + 4,
        &format!(
            "a message too large (also in UTF-8 bytes only) or not a string is dropped and counted ({before:?} -> {after:?})"
        ),
    );
    let last = events.messages.borrow().last().cloned().unwrap_or_default();
    report.check(
        kinds(&events) == ["ready", "at the cap"] && last.len() <= 1024,
        &format!(
            "a message at the cap arrives ({} bytes, {:?})",
            last.len(),
            kinds(&events)
        ),
    );
    report.check(
        after.messages_received == 2,
        &format!("the messages that arrived are counted ({after:?})"),
    );

    // Half of a character pair (which UTF-8 cannot hold) is never a reason to refuse a
    // message: it arrives as U+FFFD, as the glue and innerword_board mend it (4.4).
    let _ = eval(
        &view,
        "window.webkit.messageHandlers.innerwordBoard.postMessage('{\"kind\":\"half\",\"text\":\"a\\ud800b\\udc00\"}'); 1",
    );
    testing::run_until(Duration::from_secs(3), || {
        events.messages.borrow().len() >= 3
    });
    let half = events.messages.borrow().get(2).cloned().unwrap_or_default();
    report.check(
        half == "{\"kind\":\"half\",\"text\":\"a\u{FFFD}b\u{FFFD}\"}"
            && view.stats().messages_dropped == after.messages_dropped,
        &format!(
            "a message with a lone surrogate arrives, mended to U+FFFD ({half:?}, {:?})",
            view.stats()
        ),
    );

    // A main frame of another origin: a sandboxed document has an opaque one.
    let sandboxed: &'static str = "default-src 'self' 'unsafe-inline'; sandbox allow-scripts";
    let (other, other_events) = host.board(
        "board-2-1700000000000",
        vec![
            (
                "index.html",
                html("<!doctype html><script>window.webkit.messageHandlers.innerwordBoard.postMessage(JSON.stringify({ kind: 'opaque' }));</script>"),
            ),
        ],
        board_extras(1024, Some(sandboxed)),
        rules,
    );
    testing::run_until(Duration::from_secs(3), || {
        other.stats().messages_dropped > 0 || !other_events.messages.borrow().is_empty()
    });
    report.check(
        other_events.messages.borrow().is_empty() && other.stats().messages_dropped == 1,
        &format!(
            "a message of a main frame whose origin is not the view's own is dropped ({:?}, {:?})",
            other_events.messages.borrow(),
            other.stats()
        ),
    );
    drop(other);

    // A page's view has no bridge.
    let (page, page_events) = host.page(
        vec![("index.html", html("<!doctype html><p>A page</p>"))],
        "index.html",
        rules,
    );
    let handlers = eval(
        &page,
        "typeof (window.webkit && window.webkit.messageHandlers && window.webkit.messageHandlers.innerwordBoard)",
    );
    report.check(
        handlers.as_deref() == Ok("undefined"),
        &format!("a page's view has no messageHandlers.innerwordBoard ({handlers:?})"),
    );
    let _ = page_events;
    drop(page);
    drop(view);

    // The handler goes with its view.
    wait(Duration::from_millis(200));
    let baseline = testing::live_bridges();
    let mut while_alive = Vec::new();
    for n in 0..20 {
        let (view, _events) = host.board(
            &format!("board-{}-1700000000000", 100 + n),
            vec![("index.html", html("<!doctype html><p>x</p>"))],
            board_extras(1024, None),
            rules,
        );
        while_alive.push(testing::live_bridges());
        drop(view);
    }
    report.check(
        while_alive.iter().all(|&alive| alive > baseline),
        &format!("each view has its handler while it lives ({while_alive:?})"),
    );
    testing::run_until(Duration::from_secs(5), || {
        testing::live_bridges() <= baseline
    });
    report.check(
        testing::live_bridges() <= baseline,
        &format!(
            "the bridge's handler goes with its view: none left of 20 views ({} alive, {baseline} before)",
            testing::live_bridges()
        ),
    );
}

/// A frame inside a whiteboard's view never reaches the bridge. The main-frame check
/// cannot see one of the view's own origin that posts through `top` (WebKit reports such a
/// message as the main frame's), so a view with the bridge loads no frame at all, whatever
/// its policy; and the whiteboard's policy itself lets no frame of a file and no inline
/// script in a frame run. The control (no bridge, a page's policy) shows each frame would
/// run and post otherwise.
///
/// The whiteboard's policy alone does **not** keep out a `srcdoc` frame that names a script
/// of the view's own origin (`<script src="inner.js">`): a `srcdoc` document inherits the
/// policy, and `script-src 'self'` lets that script run, also in a frame made by a script
/// after the load (as React makes the `iframe` element of a scene) and in one with
/// `sandbox="allow-scripts allow-same-origin"` (as Excalidraw's embeds have). That is held
/// here as a fact of WebKit, so that whoever reads the policy as a wall against frames
/// learns otherwise; what keeps those frames from the bridge is that the view loads none.
pub(super) fn frames_never_reach_the_bridge(report: &mut Report, rules: &Path) {
    let files = || {
        vec![
            (
                "index.html",
                html(
                    r#"<!doctype html><title>Frames</title><body>
<iframe id="file" src="inner.html"></iframe>
<iframe id="srcdoc" srcdoc="<script src=&quot;inner.js&quot;></script>"></iframe>
<iframe id="inline" srcdoc="<script>top.__ran = (top.__ran || []).concat(['inline']); top.webkit.messageHandlers.innerwordBoard.postMessage(JSON.stringify({ kind: 'inline' }));</script>"></iframe>
<iframe id="data" src="data:text/html,%3Cscript%3Eparent.postMessage('data', '*')%3C/script%3E"></iframe>
<script src="app.js"></script></body>"#,
                ),
            ),
            (
                "app.js",
                js(
                    "window.__ran = window.__ran || []; window.addEventListener('message', (e) => { window.__ran = window.__ran.concat([String(e.data)]); }); if (window.webkit && window.webkit.messageHandlers.innerwordBoard) window.webkit.messageHandlers.innerwordBoard.postMessage(JSON.stringify({ kind: 'ready' }));
const frame = (id, sandbox) => { const f = document.createElement('iframe'); f.id = id; if (sandbox) f.setAttribute('sandbox', 'allow-scripts allow-same-origin'); f.srcdoc = '<script src=\"inner.js\"></script>'; document.body.appendChild(f); };
frame('made', false);
frame('sandboxed', true);
setTimeout(() => { const f = document.getElementById('file'); f.removeAttribute('src'); f.id = 'later'; f.srcdoc = '<script src=\"inner.js\"></script>'; }, 300);",
                ),
            ),
            (
                "inner.html",
                html(r#"<!doctype html><script src="inner.js"></script>"#),
            ),
            (
                "inner.js",
                js(
                    "const ran = window.frameElement && window.frameElement.id !== 'file' ? window.frameElement.id : (String(location.href).startsWith('about:srcdoc') ? 'srcdoc' : 'file'); top.__ran = (top.__ran || []).concat([ran]); top.webkit.messageHandlers.innerwordBoard.postMessage(JSON.stringify({ kind: 'through top' }));",
                ),
            ),
        ]
    };
    let host = Host::new();
    let ran = |view: &WebPage| -> Vec<String> {
        let mut list: Vec<String> = eval(view, "JSON.stringify(window.__ran || [])")
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        list.sort();
        list
    };
    let cases: [(&str, ViewExtras); 4] = [
        (
            "a view with the bridge under a page's policy",
            board_extras(4096, Some(innerword_web::CONTENT_SECURITY_POLICY)),
        ),
        (
            "a view with the bridge under the whiteboard's policy",
            board_extras(4096, None),
        ),
        (
            "a view without the bridge under the whiteboard's policy",
            ViewExtras {
                bridge: None,
                one_document: true,
                csp: Some(BOARD_CONTENT_SECURITY_POLICY),
            },
        ),
        (
            "the control: no bridge, a page's policy",
            ViewExtras {
                bridge: None,
                one_document: true,
                csp: Some(innerword_web::CONTENT_SECURITY_POLICY),
            },
        ),
    ];
    for (n, (what, extras)) in cases.into_iter().enumerate() {
        let (view, events) = host.board(
            &format!("board-{}-1700000000000", 30 + n),
            files(),
            extras,
            rules,
        );
        wait(Duration::from_millis(1500));
        let ran = ran(&view);
        let stats = view.stats();
        match n {
            0 | 1 => report.check(
                kinds(&events) == ["ready"] && ran.is_empty() && stats.messages_dropped == 0,
                &format!(
                    "{what}: no frame loads, none posts (ran {ran:?}, messages {:?}, {stats:?})",
                    kinds(&events)
                ),
            ),
            2 => report.check(
                !ran.iter().any(|r| r == "file" || r == "inline" || r == "data"),
                &format!(
                    "{what}: no frame of a file, no inline script in a frame and no data: frame runs (ran {ran:?})"
                ),
            ),
            _ => report.check(
                [
                    "data",
                    "file",
                    "inline",
                    "srcdoc",
                    "made",
                    "sandboxed",
                    "later",
                ]
                .iter()
                .all(|kind| ran.iter().any(|r| r == kind)),
                &format!("{what}: every frame runs (ran {ran:?})"),
            ),
        }
        if n == 2 {
            let own_script: Vec<&String> = ran
                .iter()
                .filter(|r| ["srcdoc", "made", "sandboxed", "later"].contains(&r.as_str()))
                .collect();
            report.note(&format!(
                "{what}: a srcdoc frame that names a script of the view's own origin {} (ran {own_script:?})",
                if own_script.is_empty() {
                    "does not run it"
                } else {
                    "runs it: the policy alone does not stop it, the bridge's view loads no frame"
                }
            ));
        }
    }
}

pub(super) fn call_async(report: &mut Report, rules: &Path) {
    let host = Host::new();
    let (view, _events) = host.board(
        "board-3-1700000000000",
        vec![
            (
                "index.html",
                html("<!doctype html><title>Calls</title><script src=\"app.js\"></script>"),
            ),
            ("app.js", js("window.glue = { twice: (n) => n * 2 };")),
        ],
        board_extras(1024, None),
        rules,
    );
    let awaited = call(
        &view,
        "return await new Promise((resolve) => setTimeout(() => resolve('later: ' + JSON.parse(arg).n), 50));",
        r#"{"n": 21}"#,
    );
    report.check(
        awaited.as_deref() == Ok("later: 21"),
        &format!("call_async answers an awaited promise's value ({awaited:?})"),
    );
    let glue = call(
        &view,
        "return String(window.glue.twice(JSON.parse(arg).n));",
        r#"{"n": 4}"#,
    );
    report.check(
        glue.as_deref() == Ok("8"),
        &format!("call_async runs in the page world, where the page's own script is ({glue:?})"),
    );
    let thrown = call(&view, "throw new Error('the scene is broken');", "{}");
    report.check(
        matches!(&thrown, Err(why) if why.contains("the scene is broken")),
        &format!("call_async answers an exception's message ({thrown:?})"),
    );
    let rejected = call(
        &view,
        "await Promise.reject(new Error('refused here'));",
        "{}",
    );
    report.check(
        matches!(&rejected, Err(why) if why.contains("refused here")),
        &format!("call_async answers a rejected promise's message ({rejected:?})"),
    );
    let argument = serde_json::json!({
        "text": "quotes \" and ' and `, a back\\slash,\na line break, a tab\t, </script>, \u{1F600}"
    })
    .to_string();
    let half = call(&view, "return 'a\\ud800b';", "{}");
    report.check(
        half.as_deref() == Ok("a\u{FFFD}b"),
        &format!("an answer with a lone surrogate is read, mended to U+FFFD ({half:?})"),
    );
    let echoed = call(&view, "return arg;", &argument);
    report.check(
        echoed.as_deref() == Ok(argument.as_str()),
        &format!("an argument with quotes and line breaks arrives as it was ({echoed:?})"),
    );
    let object = call(&view, "return { a: [1, 2] };", "{}");
    report.check(
        object.as_deref() == Ok(r#"{"a":[1,2]}"#),
        &format!("an answer that is no string is written out ({object:?})"),
    );
    let started = Instant::now();
    let big = call(
        &view,
        "const n = JSON.parse(arg).n; return 'z'.repeat(n);",
        r#"{"n": 20971520}"#,
    );
    report.check(
        matches!(&big, Ok(text) if text.len() == 20 * 1024 * 1024),
        &format!(
            "an answer of 20 MB comes back whole ({:?} bytes)",
            big.as_ref().map(String::len)
        ),
    );
    report.note(&format!(
        "20 MB back through call_async: {:?}",
        started.elapsed()
    ));
    let passed = call(
        &view,
        "return String(JSON.parse(arg).length);",
        &serde_json::to_string(&"q".repeat(20 * 1024 * 1024)).unwrap(),
    );
    report.check(
        passed.as_deref() == Ok("20971520"),
        &format!("an argument of 20 MB arrives whole ({passed:?})"),
    );
}

pub(super) fn one_document(report: &mut Report, rules: &Path) {
    let host = Host::new();
    let files = || {
        vec![
            (
                "index.html",
                html(
                    "<!doctype html><title>Entry</title><a id=\"own\" href=\"other.html\">other</a>",
                ),
            ),
            ("other.html", html("<!doctype html><title>Other</title>")),
        ]
    };
    let (view, events) = host.board(
        "board-4-1700000000000",
        files(),
        board_extras(1024, None),
        rules,
    );
    let loads = events.loaded.get();
    let _ = eval(&view, "location = 'other.html'; 1");
    wait(Duration::from_millis(500));
    let _ = eval(&view, "document.getElementById('own').click(); 1");
    wait(Duration::from_millis(500));
    let _ = eval(&view, "location.reload(); 1");
    wait(Duration::from_millis(500));
    let title = eval(&view, "document.title");
    report.check(
        title.as_deref() == Ok("Entry") && events.loaded.get() == loads,
        &format!(
            "a whiteboard's view goes nowhere after its first load, not to its own files, not by reload ({title:?}, loads {loads} -> {})",
            events.loaded.get()
        ),
    );
    report.check(
        view.stats().blocked_navigations >= 3,
        &format!(
            "each is counted as a navigation cancelled ({:?})",
            view.stats()
        ),
    );
    view.reload();
    testing::run_until(Duration::from_secs(5), || events.loaded.get() > loads);
    report.check(
        events.loaded.get() == loads + 1,
        &format!(
            "the app's own reload still loads ({} loads)",
            events.loaded.get()
        ),
    );
    let _ = eval(&view, "window.alert('a message dressed as the app'); 1");
    wait(Duration::from_millis(300));
    report.check(
        events.alerts.borrow().is_empty(),
        &format!("an alert makes no event ({:?})", events.alerts.borrow()),
    );

    // A page's view, as before: its own files load, an alert is told.
    let (page, page_events) = host.page(files(), "index.html", rules);
    let _ = eval(&page, "location = 'other.html'; 1");
    let moved = eval_until(
        &page,
        "document.title === 'Other' ? 'yes' : ''",
        Duration::from_secs(5),
    );
    report.check(moved == "yes", "a page's view still goes to its own files");
    let _ = eval(&page, "window.alert('hello'); 1");
    testing::run_until(Duration::from_secs(2), || {
        !page_events.alerts.borrow().is_empty()
    });
    report.check(
        page_events.alerts.borrow().len() == 1,
        &format!(
            "a page's alert is still told ({:?})",
            page_events.alerts.borrow()
        ),
    );
}

pub(super) fn the_board_policy(report: &mut Report, server: &Server, rules: &Path) {
    // Each way would post a message. Under a page's policy (the control) they do, but the
    // two frames, which a view with the bridge never loads (`frames_never_reach_the_bridge`);
    // under the whiteboard's nothing runs but the bundle's own file.
    let image = server.url("pixel.png");
    let index = format!(
        r#"<!doctype html><title>Policy</title>
<script>window.webkit.messageHandlers.innerwordBoard.postMessage(JSON.stringify({{ kind: 'inline' }}));</script>
<img src="x" onerror="window.webkit.messageHandlers.innerwordBoard.postMessage(JSON.stringify({{ kind: 'handler' }}))">
<iframe srcdoc="<script>top.webkit.messageHandlers.innerwordBoard.postMessage(JSON.stringify({{ kind: 'srcdoc' }}))</script>"></iframe>
<iframe src="data:text/html,%3Cscript%3Ewindow.webkit.messageHandlers.innerwordBoard.postMessage(JSON.stringify(%7Bkind%3A'data'%7D))%3C/script%3E"></iframe>
<img src="{image}">
<script src="app.js"></script>"#
    );
    let app = format!(
        "{POST}post('own');\n\
         try {{ eval(\"post('eval')\"); }} catch (_) {{}}\n\
         try {{ new Function(\"post('function')\")(); }} catch (_) {{}}\n\
         try {{ setTimeout(\"post('string timer')\", 0); }} catch (_) {{}}\n\
         WebAssembly.instantiate(new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0])).then(() => post('wasm'), () => {{}});"
    );
    let files = || vec![("index.html", html(&index)), ("app.js", js(&app))];
    let host = Host::new();
    let (control, control_events) = host.board(
        "board-5-1700000000000",
        files(),
        board_extras(4096, Some(innerword_web::CONTENT_SECURITY_POLICY)),
        rules,
    );
    wait(Duration::from_millis(1500));
    let mut seen = kinds(&control_events);
    seen.sort();
    report.check(
        seen == [
            "eval",
            "function",
            "handler",
            "inline",
            "own",
            "string timer",
            "wasm",
        ] && control.stats().messages_dropped == 0,
        &format!(
            "under a page's policy every way but a frame runs (the control: {seen:?}, {:?})",
            control.stats()
        ),
    );
    drop(control);

    let (board, events) = host.board(
        "board-6-1700000000000",
        files(),
        board_extras(4096, None),
        rules,
    );
    wait(Duration::from_millis(1500));
    report.check(
        kinds(&events) == ["own"] && board.stats().messages_dropped == 0,
        &format!(
            "under the whiteboard's policy no inline script, handler, eval, Function, string timer, WebAssembly, srcdoc frame or data: frame runs ({:?}, {:?})",
            kinds(&events),
            board.stats()
        ),
    );
    drop(board);

    // The check sees what the whiteboard's policy refuses.
    let outcome = check_blocking_with_policy(
        input(source(files()), "index.html", &[], rules),
        BOARD_CONTENT_SECURITY_POLICY,
    );
    let blocked = blocked_paths(&outcome);
    report.check(
        blocked
            .iter()
            .any(|(_, directive)| directive.starts_with("script-src"))
            && blocked.iter().any(|(_, directive)| {
                directive.starts_with("frame-src") || directive.starts_with("child-src")
            })
            && blocked.iter().any(|(url, directive)| {
                url.contains("127.0.0.1") && directive.starts_with("img-src")
            }),
        &format!("check_with_policy reports what the whiteboard's policy refuses ({blocked:?})"),
    );

    // The export page's policy runs WebAssembly, the view's does not.
    let wasm = |csp: &'static str| {
        run_script_blocking(ScriptInput {
            source: source(vec![
                (
                    "index.html",
                    html("<!doctype html><script src=\"wasm.js\"></script>"),
                ),
                (
                    "wasm.js",
                    js(
                        "window.wasm = WebAssembly.instantiate(new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0])).then(() => 'ran', (e) => 'refused: ' + e.message);",
                    ),
                ),
            ]),
            entry: "index.html".to_string(),
            script: "return await window.wasm;".to_string(),
            argument_json: "{}".to_string(),
            pdf: false,
            csp,
            limit: Duration::from_secs(20),
            rules_dir: rules.to_path_buf(),
        })
    };
    let export = wasm(board_export_policy());
    report.check(
        matches!(&export, Ok(ScriptOutput { value, .. }) if value == "ran"),
        &format!("the export page's policy runs WebAssembly ({export:?})"),
    );
    let view = wasm(BOARD_CONTENT_SECURITY_POLICY);
    report.check(
        matches!(&view, Ok(ScriptOutput { value, .. }) if value.starts_with("refused")),
        &format!("the whiteboard view's policy does not ({view:?})"),
    );
    report.check(
        board_export_policy() == BOARD_EXPORT_CONTENT_SECURITY_POLICY,
        "this Mac's WebKit knows 'wasm-unsafe-eval'",
    );
}

pub(super) fn park(report: &mut Report, rules: &Path) {
    let host = Host::new();
    let (view, _events) = host.board(
        "board-7-1700000000000",
        vec![
            ("index.html", html("<!doctype html><script src=\"frames.js\"></script>")),
            (
                "frames.js",
                js("window.frames_drawn = 0; const tick = () => { window.frames_drawn += 1; requestAnimationFrame(tick); }; requestAnimationFrame(tick); window.changes_seen = 0; document.addEventListener('visibilitychange', () => { window.changes_seen += 1; });"),
            ),
        ],
        board_extras(1024, None),
        rules,
    );
    view.focus();
    view.park();
    let frame = unsafe {
        let native = view.native_view() as Id;
        let frame: NSRect = msg_send![native, frame];
        frame
    };
    report.check(
        frame.origin.x <= -100_000. && frame.size.width == 1280.,
        &format!("a parked view lies far out of the window, its size kept ({frame:?})"),
    );
    report.check(!view.is_focused(), "a parked view gives the keyboard back");
    let count = |view: &WebPage| {
        call(view, "return String(window.frames_drawn);", "{}")
            .ok()
            .and_then(|text| text.parse::<u64>().ok())
            .unwrap_or(0)
    };
    let first = count(&view);
    wait(Duration::from_millis(500));
    let second = count(&view);
    report.check(
        second > first + 5,
        &format!("a parked view still runs requestAnimationFrame ({first} -> {second})"),
    );
    let state = call(&view, "return document.visibilityState;", "{}");
    report.check(
        state.as_deref() == Ok("visible"),
        &format!(
            "a parked view is visible to its page, also in a window that is covered ({state:?})"
        ),
    );
    // A view that is shown (as in the app's window) and then parked: its page is told
    // nothing, and its frames go on.
    view.set_frame(Rect {
        x: 0.,
        y: 0.,
        width: 1280.,
        height: host.content_height(),
    });
    // Brought back, the view has its occlusion detection on again, as it was found, and
    // the test's window is far off screen: shown "as in the app's window" means drawn as a
    // view in a visible window is, which is a view with occlusion detection off here.
    unsafe {
        let native = view.native_view() as Id;
        let _: () = msg_send![native, _setWindowOcclusionDetectionEnabled: NO];
        let _: () = msg_send![native, setHidden: YES];
        let _: () = msg_send![native, setHidden: NO];
    }
    wait(Duration::from_millis(200));
    let _ = call(&view, "window.changes_seen = 0; return '';", "{}");
    view.park();
    let third = count(&view);
    wait(Duration::from_millis(500));
    let fourth = count(&view);
    let changes = call(&view, "return String(window.changes_seen);", "{}");
    report.check(
        changes.as_deref() == Ok("0") && fourth > third + 5,
        &format!(
            "parking a view that is shown tells its page nothing and its frames go on ({changes:?}, {third} -> {fourth})"
        ),
    );
    let answered = call(&view, "return 'from a parked view';", "{}");
    report.check(
        answered.as_deref() == Ok("from a parked view"),
        &format!("a call answers from a parked view ({answered:?})"),
    );
    view.set_hidden(true);
    let answered = call(&view, "return 'from a hidden view';", "{}");
    report.check(
        answered.as_deref() == Ok("from a hidden view"),
        &format!("a call answers from a hidden view ({answered:?})"),
    );
    drop(view);

    // What park found comes back with set_frame: the hidden state (the app hides a view
    // under the snapshot of an overlay) and the window occlusion detection.
    let (view, _events) = host.board(
        "board-8-1700000000000",
        vec![
            ("index.html", html("<!doctype html><script src=\"frames.js\"></script>")),
            (
                "frames.js",
                js("window.frames_drawn = 0; const tick = () => { window.frames_drawn += 1; requestAnimationFrame(tick); }; requestAnimationFrame(tick);"),
            ),
        ],
        board_extras(1024, None),
        rules,
    );
    let native = view.native_view() as Id;
    let shown = Rect {
        x: 0.,
        y: 0.,
        width: 1280.,
        height: host.content_height(),
    };
    let hidden = || unsafe {
        let hidden: BOOL = msg_send![native, isHidden];
        hidden != NO
    };
    let occlusion = || unsafe {
        let answers: BOOL =
            msg_send![native, respondsToSelector: sel!(_windowOcclusionDetectionEnabled)];
        (answers != NO).then(|| {
            let on: BOOL = msg_send![native, _windowOcclusionDetectionEnabled];
            on != NO
        })
    };
    let set_occlusion = |on: bool| unsafe {
        let _: () =
            msg_send![native, _setWindowOcclusionDetectionEnabled: if on { YES } else { NO }];
    };
    view.set_frame(shown);
    set_occlusion(true);
    report.check(
        occlusion() == Some(true),
        &format!(
            "this macOS answers whether a view's occlusion detection is on ({:?})",
            occlusion()
        ),
    );
    view.set_hidden(true);
    view.park();
    let first = count(&view);
    wait(Duration::from_millis(500));
    let second = count(&view);
    report.check(
        view.is_parked() && !hidden() && occlusion() == Some(false) && second > first + 5,
        &format!(
            "a hidden view that is parked is drawn, out of sight ({}, hidden {}, occlusion {:?}, {first} -> {second})",
            view.is_parked(),
            hidden(),
            occlusion()
        ),
    );
    view.set_frame(shown);
    report.check(
        !view.is_parked() && hidden() && occlusion() == Some(true),
        &format!(
            "brought back, it is hidden again and its occlusion detection on again, as park found them (hidden {}, occlusion {:?})",
            hidden(),
            occlusion()
        ),
    );
    // A view drawn although covered (a scripted run's, occlusion detection off) and shown.
    view.set_hidden(false);
    set_occlusion(false);
    view.park();
    view.park();
    view.set_frame(shown);
    report.check(
        !hidden() && occlusion() == Some(false),
        &format!(
            "a view shown with occlusion detection off comes back so, parked twice (hidden {}, occlusion {:?})",
            hidden(),
            occlusion()
        ),
    );
    // Hidden while it was parked: hidden at once, and when it is brought back.
    view.park();
    view.set_hidden(true);
    let parked_hidden = hidden();
    view.set_frame(shown);
    report.check(
        parked_hidden && hidden(),
        &format!(
            "a parked view hidden meanwhile is hidden, and stays so when it is brought back ({parked_hidden}, {})",
            hidden()
        ),
    );
    // Shown with occlusion detection on in a covered window (the test's is far off
    // screen): parked it is drawn; brought back WebKit sees the window covered again and
    // stops drawing it, as before it was parked (a whiteboard opened again in a covered
    // window does not go on drawing).
    view.set_hidden(false);
    set_occlusion(true);
    view.park();
    wait(Duration::from_millis(200));
    let while_parked = call(&view, "return document.visibilityState;", "{}");
    view.set_frame(shown);
    wait(Duration::from_millis(500));
    let back = call(&view, "return document.visibilityState;", "{}");
    report.check(
        while_parked.as_deref() == Ok("visible") && back.as_deref() == Ok("hidden"),
        &format!(
            "brought back into a covered window with its occlusion detection on, it stops being drawn again ({while_parked:?} -> {back:?})"
        ),
    );
}

pub(super) fn run_script(report: &mut Report, rules: &Path) {
    let page = r#"<!doctype html><html><head><style>
html, body { margin: 0; background: white; }
body { width: 3000px; height: 2001px; position: relative; font: 24px Helvetica, sans-serif; }
#near { position: absolute; left: 20px; top: 20px; }
#far { position: absolute; left: 2700px; top: 1900px; }
</style></head><body><div id="near">Near words</div><div id="far">Far corner</div></body></html>"#;
    let started = Instant::now();
    let result = run_script_blocking(ScriptInput {
        source: source(vec![("index.html", html(page))]),
        entry: "index.html".to_string(),
        script: "const size = JSON.parse(arg); return JSON.stringify(size);".to_string(),
        argument_json: r#"{"width": 2999.5, "height": 2000.25}"#.to_string(),
        pdf: true,
        csp: board_export_policy(),
        limit: Duration::from_secs(30),
        rules_dir: rules.to_path_buf(),
    });
    report.note(&format!("run_script with a PDF: {:?}", started.elapsed()));
    match &result {
        Ok(ScriptOutput {
            value,
            pdf: Some(pdf),
        }) => {
            report.check(
                value == r#"{"width":2999.5,"height":2000.25}"#,
                &format!("run_script answers the script's value ({value})"),
            );
            report.check(
                pdf.pages == 1 && testing::pdf_pages(&pdf.bytes).map(|(pages, _)| pages) == Some(1),
                &format!("the PDF is one page ({pdf:?})"),
            );
            report.check(
                (pdf.paper.0 - 3000.).abs() < 0.01 && (pdf.paper.1 - 2001.).abs() < 0.01,
                &format!(
                    "its page is the rect, a fractional size rounded up ({:?})",
                    pdf.paper
                ),
            );
            let text = pdf_text(&pdf.bytes);
            report.check(
                text.contains("Near words") && text.contains("Far corner"),
                &format!("its text can be read, also beyond the view's bounds ({text:?})"),
            );
        }
        other => report.check(false, &format!("run_script makes a PDF ({other:?})")),
    }

    let plain = run_script_blocking(ScriptInput {
        source: source(vec![(
            "index.html",
            html("<!doctype html><title>T</title>"),
        )]),
        entry: "index.html".to_string(),
        script: "return document.title + ' ' + JSON.parse(arg).x;".to_string(),
        argument_json: r#"{"x": "and more"}"#.to_string(),
        pdf: false,
        csp: BOARD_CONTENT_SECURITY_POLICY,
        limit: Duration::from_secs(20),
        rules_dir: rules.to_path_buf(),
    });
    report.check(
        matches!(&plain, Ok(ScriptOutput { value, pdf: None }) if value == "T and more"),
        &format!("run_script without a PDF answers the value alone ({plain:?})"),
    );

    let base = |entry: &str, script: &str, limit: Duration| ScriptInput {
        source: source(vec![(
            "index.html",
            html("<!doctype html><title>T</title>"),
        )]),
        entry: entry.to_string(),
        script: script.to_string(),
        argument_json: "{}".to_string(),
        pdf: false,
        csp: BOARD_CONTENT_SECURITY_POLICY,
        limit,
        rules_dir: rules.to_path_buf(),
    };
    let missing = run_script_blocking(base("missing.html", "return '';", Duration::from_secs(10)));
    report.check(
        matches!(&missing, Err(why) if why.contains("missing.html")),
        &format!("an entry the source does not have fails ({missing:?})"),
    );
    let thrown = run_script_blocking(base(
        "index.html",
        "throw new Error('no scene here');",
        Duration::from_secs(10),
    ));
    report.check(
        matches!(&thrown, Err(why) if why.contains("no scene here")),
        &format!("a script error fails with its message ({thrown:?})"),
    );
    let started = Instant::now();
    let slow = run_script_blocking(base(
        "index.html",
        "await new Promise(() => {});",
        Duration::from_secs(2),
    ));
    report.check(
        matches!(&slow, Err(why) if why.contains("2 s"))
            && started.elapsed() < Duration::from_secs(8),
        &format!(
            "a script that never answers fails at the limit ({slow:?}, {:?})",
            started.elapsed()
        ),
    );
    let no_size = run_script_blocking(ScriptInput {
        pdf: true,
        ..base(
            "index.html",
            "return 'not a size';",
            Duration::from_secs(10),
        )
    });
    report.check(
        matches!(&no_size, Err(why) if why.contains("not a size")),
        &format!("an answer that is no size makes no PDF ({no_size:?})"),
    );
    // A page that answers a side far past the limit (a broken or forged export page): the
    // PDF's side is the limit, whatever it said.
    let huge = run_script_blocking(ScriptInput {
        pdf: true,
        ..base(
            "index.html",
            "return JSON.stringify({ width: 1e7, height: 200.5 });",
            Duration::from_secs(30),
        )
    });
    report.check(
        matches!(&huge, Ok(ScriptOutput { pdf: Some(pdf), .. })
            if pdf.pages == 1
                && (pdf.paper.0 - innerword_web::MAX_PDF_SIDE).abs() < 0.01
                && (pdf.paper.1 - 201.).abs() < 0.01),
        &format!(
            "a side the page answers past {} pt is cut to it ({:?})",
            innerword_web::MAX_PDF_SIDE,
            huge.as_ref().map(|o| o.pdf.as_ref().map(|p| p.paper))
        ),
    );
}
