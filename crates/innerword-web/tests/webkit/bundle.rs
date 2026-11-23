//! Excalidraw itself, in a whiteboard's view and on its export page: the bundle of
//! `innerword-board-web` (`dist/`, found by `Assets::find`) under the whiteboard's policies
//! (`docs/whiteboard-design.md`, 9.2 B4: "the bundle", Mermaid, the shape library).

use std::{collections::HashMap, sync::Condvar};

use super::board::{BRIDGE, call};
use super::*;

use innerword_board_web::{
    Assets, BOARD_PATH, ENTRY, EXPORT_ENTRY, LIBRARY_PATH, SCENE_PATH,
    bridge::{self, MAX_MESSAGE_BYTES, Message},
    calls,
};
use innerword_web::{
    BOARD_CONTENT_SECURITY_POLICY, Bridge, ScriptInput, ScriptOutput, board_export_policy,
    check_blocking_with_policy, run_script_blocking,
};
use serde_json::{Value, json};

/// The bundle's files, and what the app's source answers beside them (`board.json`,
/// `library.json`, `scene.json`, `_files/<id>`), which a case may change while a view runs.
/// A case can hold the files under a prefix (`hold`): their reads wait, on the scheme
/// handler's background queue, until `release` (a font load still in flight).
struct BundleSource {
    assets: Assets,
    extra: Mutex<HashMap<String, ServedFile>>,
    cache: Mutex<HashMap<String, ServedFile>>,
    asked: Mutex<Vec<String>>,
    hold: Mutex<Hold>,
    released: Condvar,
}

/// The files a [`BundleSource`] holds back: the prefix, and the paths waiting.
#[derive(Default)]
struct Hold {
    prefix: Option<String>,
    held: Vec<String>,
}

/// The longest a held read waits, so that a case that fails before its release never
/// leaves a thread of the scheme handler waiting for good.
const LONGEST_HOLD: Duration = Duration::from_secs(30);

impl BundleSource {
    fn new(assets: &Assets) -> Arc<BundleSource> {
        Arc::new(BundleSource {
            assets: assets.clone(),
            extra: Mutex::new(HashMap::new()),
            cache: Mutex::new(HashMap::new()),
            asked: Mutex::new(Vec::new()),
            hold: Mutex::new(Hold::default()),
            released: Condvar::new(),
        })
    }

    /// From now on, a read of a path under `prefix` waits until [`BundleSource::release`].
    fn hold(&self, prefix: &str) {
        self.hold.lock().unwrap().prefix = Some(prefix.to_string());
    }

    /// The paths whose reads are waiting.
    fn held(&self) -> Vec<String> {
        self.hold.lock().unwrap().held.clone()
    }

    /// Lets every held read go on, and holds nothing more.
    fn release(&self) {
        let mut hold = self.hold.lock().unwrap();
        hold.prefix = None;
        hold.held.clear();
        self.released.notify_all();
    }

    fn set(&self, path: &str, media_type: &str, bytes: &[u8]) {
        self.extra
            .lock()
            .unwrap()
            .insert(path.to_string(), file(media_type, bytes));
    }

    fn set_json(&self, path: &str, value: &Value) {
        self.set(path, "application/json", value.to_string().as_bytes());
    }
}

impl PageSource for BundleSource {
    fn file(&self, path: &str) -> Option<ServedFile> {
        self.asked.lock().unwrap().push(path.to_string());
        {
            let holds = |hold: &Hold| {
                hold.prefix
                    .as_deref()
                    .is_some_and(|prefix| path.starts_with(prefix))
            };
            let mut hold = self.hold.lock().unwrap();
            if holds(&hold) {
                hold.held.push(path.to_string());
                let _ = self
                    .released
                    .wait_timeout_while(hold, LONGEST_HOLD, |hold| holds(hold));
            }
        }
        if let Some(found) = self.extra.lock().unwrap().get(path) {
            return Some(found.clone());
        }
        if let Some(found) = self.cache.lock().unwrap().get(path) {
            return Some(found.clone());
        }
        let info = self.assets.get(path)?;
        let bytes = self.assets.read(path)?.ok()?;
        let served = file(&info.media_type, &bytes);
        self.cache
            .lock()
            .unwrap()
            .insert(path.to_string(), served.clone());
        Some(served)
    }
}

/// The scout's drawing as skeletons: a rectangle with a label, an ellipse, an arrow bound
/// between them, free text, CJK text and a frame around the first three.
const SKELETONS: &str = r##"[
  {"type":"rectangle","id":"box","x":100,"y":100,"width":220,"height":110,
   "backgroundColor":"#ffec99","fillStyle":"solid","label":{"text":"Draft chapter"}},
  {"type":"ellipse","id":"oval","x":500,"y":110,"width":180,"height":100,
   "strokeColor":"#1971c2","label":{"text":"Review"}},
  {"type":"arrow","id":"link","x":320,"y":155,"width":180,"height":0,
   "start":{"id":"box"},"end":{"id":"oval"},"label":{"text":"sends"}},
  {"type":"text","id":"note","x":110,"y":280,"text":"Free text, hand-drawn font","fontSize":24},
  {"type":"text","id":"cjk","x":500,"y":280,"text":"你好，世界","fontSize":24},
  {"type":"frame","id":"frame1","children":["box","oval","link"],"name":"Pipeline"}
]"##;

/// A stamp of the form the store gives (32 hex digits).
const STAMP: &str = "0123456789abcdef0123456789abcdef";

fn stored_scene(elements: Value) -> Value {
    json!({
        "type": "excalidraw", "version": 2, "source": "inner-word",
        "elements": elements,
        "appState": {"viewBackgroundColor": "#ffffff", "gridSize": 20, "gridStep": 5, "gridModeEnabled": false},
        "files": {}
    })
}

/// A plain rectangle as Excalidraw stores one.
fn rectangle(id: &str, x: f64, index: &str) -> Value {
    json!({
        "id": id, "type": "rectangle", "x": x, "y": 100, "width": 100, "height": 60, "angle": 0,
        "strokeColor": "#1e1e1e", "backgroundColor": "transparent", "fillStyle": "solid",
        "strokeWidth": 2, "strokeStyle": "solid", "roughness": 1, "opacity": 100,
        "groupIds": [], "frameId": null, "index": index, "roundness": null, "seed": 7,
        "version": 3, "versionNonce": 11, "isDeleted": false, "updated": 1_794_306_980_000u64,
        "link": null, "locked": false, "boundElements": []
    })
}

/// A free text as Rust would write it: its width a guess that Excalidraw measures again.
fn rust_text(id: &str, text: &str, index: &str) -> Value {
    json!({
        "id": id, "type": "text", "x": 100, "y": 400, "width": 10, "height": 25, "angle": 0,
        "strokeColor": "#1e1e1e", "backgroundColor": "transparent", "fillStyle": "solid",
        "strokeWidth": 2, "strokeStyle": "solid", "roughness": 1, "opacity": 100,
        "groupIds": [], "frameId": null, "index": index, "roundness": null, "seed": 9,
        "version": 1, "versionNonce": 13, "isDeleted": false, "updated": 1_794_306_980_000u64,
        "link": null, "locked": false, "boundElements": [],
        "text": text, "originalText": text, "fontSize": 20, "fontFamily": 5,
        "textAlign": "left", "verticalAlign": "top", "containerId": null,
        "autoResize": true, "lineHeight": 1.25
    })
}

/// The export page's own conversion of skeletons into elements.
pub(super) fn convert(assets: &Assets, rules: &Path, skeletons: &str) -> Result<Value, String> {
    let source = BundleSource::new(assets);
    let output = run_script_blocking(ScriptInput {
        source,
        entry: EXPORT_ENTRY.to_string(),
        script: calls::CONVERT_SKELETONS.to_string(),
        argument_json: calls::convert_skeletons_argument(skeletons),
        pdf: false,
        csp: board_export_policy(),
        limit: Duration::from_secs(60),
        rules_dir: rules.to_path_buf(),
    })?;
    let value: Value = serde_json::from_str(&output.value).map_err(|e| e.to_string())?;
    Ok(value["elements"].clone())
}

/// A whiteboard's view in the test's window, drawn although the window is covered (it is
/// far off screen): Excalidraw mounts in animation frames, which a covered view does not
/// run. The app does the same in a scripted run (`pages::draw_when_covered`).
fn open(
    host: &Host,
    view_host: &str,
    source: Arc<BundleSource>,
    rules: &Path,
) -> (WebPage, Rc<Events>) {
    let (view, events) = host.board_from(view_host, source, board_view_extras(), rules);
    unsafe {
        let native = view.native_view() as Id;
        let _: () = msg_send![native, _setWindowOcclusionDetectionEnabled: NO];
        let _: () = msg_send![native, setHidden: YES];
        let _: () = msg_send![native, setHidden: NO];
    }
    (view, events)
}

fn board_view_extras() -> ViewExtras {
    ViewExtras {
        bridge: Some(Bridge {
            name: BRIDGE,
            max_bytes: MAX_MESSAGE_BYTES,
        }),
        one_document: true,
        csp: Some(BOARD_CONTENT_SECURITY_POLICY),
    }
}

/// The messages of a view from `from` on, parsed (an unreadable one as an error).
fn messages(events: &Events, from: usize) -> Vec<Result<Message, String>> {
    events.messages.borrow()[from.min(events.messages.borrow().len())..]
        .iter()
        .map(|body| bridge::parse(body))
        .collect()
}

/// Waits until a message from `from` on satisfies `found`; its index.
fn wait_for(
    events: &Events,
    from: usize,
    limit: Duration,
    found: impl Fn(&Message) -> bool,
) -> Option<usize> {
    let hit = Cell::new(None);
    testing::run_until(limit, || {
        let list = messages(events, from);
        hit.set(
            list.iter()
                .position(|m| m.as_ref().is_ok_and(&found))
                .map(|at| at + from),
        );
        hit.get().is_some()
    });
    hit.get()
}

fn kind_of(message: &Result<Message, String>) -> String {
    match message {
        Ok(Message::Ready { .. }) => "ready",
        Ok(Message::Scene { .. }) => "scene",
        Ok(Message::ScenePart { .. }) => "scenePart",
        Ok(Message::File { .. }) => "file",
        Ok(Message::Status(_)) => "status",
        Ok(Message::Library { .. }) => "library",
        Ok(Message::Help) => "help",
        Ok(Message::Error { .. }) => "error",
        Err(_) => "unreadable",
    }
    .to_string()
}

/// The errors the page posted (script errors, policy reports through its own listener).
fn page_errors(events: &Events) -> Vec<String> {
    messages(events, 0)
        .into_iter()
        .filter_map(|m| match m {
            Ok(Message::Error { message }) => Some(message),
            Err(why) => Some(format!("unreadable: {why}")),
            _ => None,
        })
        .collect()
}

/// The canvas as Excalidraw has it, deleted elements included.
fn canvas(view: &WebPage) -> Vec<Value> {
    let text = call(
        view,
        "return JSON.stringify(window.innerwordBoard.api.getSceneElementsIncludingDeleted());",
        "{}",
    )
    .unwrap_or_default();
    serde_json::from_str(&text).unwrap_or_default()
}

fn element<'a>(elements: &'a [Value], id: &str) -> Option<&'a Value> {
    elements.iter().find(|e| e["id"] == id)
}

fn live(elements: &[Value], id: &str) -> bool {
    element(elements, id).is_some_and(|e| e["isDeleted"] != true)
}

/// Listens, in the page world, for the policy's reports from now on (`window.__reports`).
const WATCH_REPORTS: &str = "if (!window.__reports) { window.__reports = []; document.addEventListener('securitypolicyviolation', (e) => window.__reports.push(String(e.effectiveDirective) + ' ' + String(e.blockedURI))); } return '';";

fn reports(view: &WebPage) -> Vec<String> {
    call(view, "return JSON.stringify(window.__reports || []);", "{}")
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub(super) fn the_bundle(report: &mut Report, server: &Server, rules: &Path) {
    let assets = match Assets::find() {
        Ok(assets) => assets,
        Err(why) => {
            report.check(false, &format!("the whiteboard's bundle is found ({why})"));
            return;
        }
    };
    report.check(
        assets.get(ENTRY).is_some() && assets.get(EXPORT_ENTRY).is_some(),
        &format!("the bundle is found at {}", assets.root().display()),
    );

    loaded_offline(report, &assets, &stored_scene(json!([])), rules);
    let elements = match convert(&assets, rules, SKELETONS) {
        Ok(Value::Array(list)) if !list.is_empty() => Value::Array(list),
        other => {
            report.check(
                false,
                &format!("the export page converts the scout's skeletons ({other:?})"),
            );
            return;
        }
    };
    report.check(true, "the export page converts the scout's skeletons");
    let scene = stored_scene(elements.clone());
    loaded_offline(report, &assets, &scene, rules);
    exported(report, &assets, &scene, rules);
    in_the_view(report, &assets, &scene, rules);
    adopted(report, &assets, rules);
    mermaid(report, &assets, server, rules);
    shape_library(report, &assets, &elements, rules);
}

/// The bundle loaded through the check, under the whiteboard's policies: nothing blocked,
/// nothing missing, no script error.
fn loaded_offline(report: &mut Report, assets: &Assets, scene: &Value, rules: &Path) {
    // Xiaolai's 404s are tolerated only when the build could not apply its patch 2 (the
    // faces left registered, section 3), which BUILD-INFO.json says.
    let xiaolai_left_registered = xiaolai_left_registered(assets);
    let source = BundleSource::new(assets);
    source.set_json(
        BOARD_PATH,
        &json!({"stamp": STAMP, "scene": scene, "view": {}}),
    );
    source.set_json(LIBRARY_PATH, &json!({}));
    source.set_json(SCENE_PATH, scene);
    for (entry, csp) in [
        (ENTRY, BOARD_CONTENT_SECURITY_POLICY),
        (EXPORT_ENTRY, board_export_policy()),
    ] {
        let outcome = check_blocking_with_policy(
            CheckInput {
                source: source.clone(),
                entry: entry.to_string(),
                also: Vec::new(),
                settle_min: Duration::from_millis(1500),
                settle_max: Duration::from_millis(5000),
                limit: Duration::from_secs(60),
                rules_dir: rules.to_path_buf(),
            },
            csp,
        );
        let load = outcome.loads.first();
        let missing: Vec<&String> = load
            .map(|l| {
                l.missing
                    .iter()
                    .filter(|p| !(xiaolai_left_registered && p.starts_with("fonts/Xiaolai/")))
                    .collect()
            })
            .unwrap_or_default();
        report.check(
            outcome.failure.is_none()
                && load.is_some_and(|l| {
                    l.finished && l.blocked.is_empty() && l.script_errors.is_empty()
                })
                && missing.is_empty(),
            &format!(
                "{entry} loads offline under its policy: nothing blocked, missing or failing ({:?})",
                outcome.loads.first().map(|l| (
                    l.finished,
                    &l.blocked,
                    &l.missing,
                    &l.script_errors,
                    l.requests
                ))
            ),
        );
        if xiaolai_left_registered
            && let Some(load) = load
            && load.missing.iter().any(|p| p.starts_with("fonts/Xiaolai/"))
        {
            report.note(&format!(
                "{entry}: Xiaolai's faces were asked for and answered 404 (the build left them registered)"
            ));
        }
    }
}

/// Whether the bundle's build left Xiaolai's faces registered (patch 2 "no Xiaolai" not
/// applied, `BUILD-INFO.json`'s `patches`): only then may its faces be asked for and
/// answered 404.
fn xiaolai_left_registered(assets: &Assets) -> bool {
    let info: Value = std::fs::read_to_string(assets.root().join("BUILD-INFO.json"))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    let applied = info["patches"].as_array().and_then(|patches| {
        patches
            .iter()
            .find(|p| p["name"].as_str().is_some_and(|n| n.contains("Xiaolai")))
            .and_then(|p| p["applied"].as_u64())
    });
    applied == Some(0)
}

/// The export page makes the PDF of the scout's scene.
fn exported(report: &mut Report, assets: &Assets, scene: &Value, rules: &Path) {
    let source = BundleSource::new(assets);
    source.set_json(SCENE_PATH, scene);
    let script = format!(
        "{WATCH_REPORTS_BODY}\nconst size = JSON.parse(await (async () => {{ {body} }})());\nreturn JSON.stringify(Object.assign(size, {{ reports: window.__reports }}));",
        WATCH_REPORTS_BODY = WATCH_REPORTS.trim_end_matches("return '';"),
        body = calls::EXPORT_LAYOUT
    );
    let started = Instant::now();
    let result = run_script_blocking(ScriptInput {
        source: source.clone(),
        entry: EXPORT_ENTRY.to_string(),
        script,
        argument_json: calls::export_argument(),
        pdf: true,
        csp: board_export_policy(),
        limit: Duration::from_secs(60),
        rules_dir: rules.to_path_buf(),
    });
    report.note(&format!("the export page's PDF: {:?}", started.elapsed()));
    let (value, pdf) = match result {
        Ok(ScriptOutput {
            value,
            pdf: Some(pdf),
        }) => (value, pdf),
        other => {
            report.check(false, &format!("the export page makes a PDF ({other:?})"));
            return;
        }
    };
    let answer: Value = serde_json::from_str(&value).unwrap_or_default();
    let (width, height) = (
        answer["width"].as_f64().unwrap_or(0.),
        answer["height"].as_f64().unwrap_or(0.),
    );
    report.check(
        pdf.pages == 1
            && (pdf.paper.0 - width.ceil()).abs() < 0.01
            && (pdf.paper.1 - height.ceil()).abs() < 0.01
            && width > 600.,
        &format!(
            "the PDF is one page of the SVG's size rounded up ({width} x {height} -> {:?})",
            pdf.paper
        ),
    );
    report.check(
        answer["reports"].as_array().is_some_and(|r| r.is_empty()),
        &format!(
            "the export made no report of the policy ({})",
            answer["reports"]
        ),
    );
    let text = pdf_text(&pdf.bytes);
    report.check(
        ["Draft chapter", "Review", "sends", "Free text"]
            .iter()
            .all(|word| text.contains(word)),
        &format!("the PDF's text has the drawing's words ({text:?})"),
    );
    report.note(&format!(
        "the CJK text in the PDF ({}): {:?}",
        if text.contains("你好") {
            "there"
        } else {
            "not found"
        },
        text.lines().find(|l| l.contains('你'))
    ));
    let raw = String::from_utf8_lossy(&pdf.bytes);
    report.check(raw.contains("/FontFile"), "the PDF embeds its fonts");
    let asked = source.asked.lock().unwrap().clone();
    report.check(
        !asked.iter().any(|p| p == BOARD_PATH || p == LIBRARY_PATH),
        &format!("the export page reads the stored scene only ({asked:?})"),
    );
}

/// The live view: ready with its stamp, a scene posted, a large one in parts, collect,
/// a pasted iframe, the tooltips, `?`.
fn in_the_view(report: &mut Report, assets: &Assets, scene: &Value, rules: &Path) {
    let source = BundleSource::new(assets);
    source.set_json(
        BOARD_PATH,
        &json!({"stamp": STAMP, "scene": scene, "view": {}}),
    );
    source.set_json(LIBRARY_PATH, &json!({}));
    let host = Host::new();
    let started = Instant::now();
    let (view, events) = open(&host, "board-12-1700000000000", source.clone(), rules);
    let ready = wait_for(&events, 0, Duration::from_secs(20), |m| {
        matches!(m, Message::Ready { .. })
    });
    report.note(&format!(
        "the view from load to ready: {:?}",
        started.elapsed()
    ));
    let ready_ok = ready.is_some_and(|at| {
        matches!(&messages(&events, at)[0], Ok(Message::Ready { stamp, elements, excalidraw })
            if stamp == STAMP && *elements >= 6 && excalidraw == "0.18.1")
    });
    report.check(
        ready_ok,
        &format!(
            "a view loads a stored scene and posts ready with its stamp ({:?})",
            messages(&events, 0).iter().map(kind_of).collect::<Vec<_>>()
        ),
    );
    if ready.is_none() {
        report.note(&format!("the page's errors: {:?}", page_errors(&events)));
        return;
    }
    let _ = call(&view, WATCH_REPORTS, "{}");
    report.check(
        !messages(&events, 0)
            .iter()
            .any(|m| matches!(m, Ok(Message::Scene { .. }) | Ok(Message::ScenePart { .. }))),
        "opening saves nothing: no scene was posted at the load",
    );

    // A synthetic edit posts the scene.
    let from = events.messages.borrow().len();
    let _ = eval(
        &view,
        r#"(() => { const api = window.innerwordBoard.api;
            const els = api.getSceneElementsIncludingDeleted();
            const box = els.find((e) => e.id === 'box');
            api.updateScene({ elements: els.map((e) => e.id === 'box' ? { ...e, x: e.x + 40, version: e.version + 1, versionNonce: 5 } : e), captureUpdate: 'IMMEDIATELY' });
            return 1; })()"#,
    );
    let posted = wait_for(
        &events,
        from,
        Duration::from_secs(5),
        |m| matches!(m, Message::Scene { scene, .. } if scene.get().contains("\"box\"")),
    );
    report.check(
        posted.is_some(),
        &format!(
            "an edit posts the scene ({:?})",
            messages(&events, from)
                .iter()
                .map(kind_of)
                .collect::<Vec<_>>()
        ),
    );

    // A scene of 20 MB comes in parts, every message under the cap.
    let from = events.messages.borrow().len();
    let _ = call(
        &view,
        r#"const api = window.innerwordBoard.api;
           const els = api.getSceneElementsIncludingDeleted();
           const big = { ...els.find((e) => e.id === 'note'), id: 'big', y: 700, customData: { pad: 'p'.repeat(20 * 1024 * 1024) } };
           delete big.index;
           api.updateScene({ elements: [...els, big], captureUpdate: 'IMMEDIATELY' });
           return '';"#,
        "{}",
    );
    let last = wait_for(
        &events,
        from,
        Duration::from_secs(20),
        |m| matches!(m, Message::ScenePart { index, count, .. } if index + 1 == *count),
    );
    let bodies: Vec<String> = events.messages.borrow()[from..].to_vec();
    let parts: Vec<(u64, u32, u32, String)> = bodies
        .iter()
        .filter_map(|body| match bridge::parse(body) {
            Ok(Message::ScenePart {
                rev,
                index,
                count,
                text,
            }) => Some((rev, index, count, text)),
            _ => None,
        })
        .collect();
    let joined: String = parts.iter().map(|(_, _, _, text)| text.as_str()).collect();
    let whole: Option<Value> = serde_json::from_str(&joined).ok();
    report.check(
        last.is_some()
            && parts.len() >= 2
            && parts.iter().all(|(rev, _, count, _)| *rev == parts[0].0 && *count as usize == parts.len())
            && bodies.iter().all(|b| b.len() <= MAX_MESSAGE_BYTES)
            && whole.as_ref().is_some_and(|w| {
                w["elements"].as_array().is_some_and(|list| list.iter().any(|e| e["id"] == "big"))
            }),
        &format!(
            "a scene of 20 MB comes as scenePart messages that join into it ({} parts, largest message {} bytes)",
            parts.len(),
            bodies.iter().map(String::len).max().unwrap_or(0)
        ),
    );

    // collect answers what was not posted yet: parts for a large one, the scene for a small.
    let from = events.messages.borrow().len();
    let collected = call(
        &view,
        &format!(
            r#"const api = window.innerwordBoard.api;
               const els = api.getSceneElementsIncludingDeleted();
               api.updateScene({{ elements: els.map((e) => e.id === 'big' ? {{ ...e, x: e.x + 5, version: e.version + 1, versionNonce: 6 }} : e), captureUpdate: 'IMMEDIATELY' }});
               {}"#,
            calls::COLLECT
        ),
        "{}",
    );
    let collected: Value = collected
        .as_deref()
        .ok()
        .and_then(|t| serde_json::from_str(t).ok())
        .unwrap_or_default();
    let parts_posted = messages(&events, from)
        .iter()
        .filter(|m| matches!(m, Ok(Message::ScenePart { .. })))
        .count();
    report.check(
        collected["scene"] == "parts" && parts_posted >= 2,
        &format!(
            "collect answers a large scene not posted yet as parts posted before its answer ({}, {parts_posted} parts)",
            collected["scene"].as_str().unwrap_or("?")
        ),
    );
    let collected = call(
        &view,
        &format!(
            r#"const api = window.innerwordBoard.api;
               const els = api.getSceneElementsIncludingDeleted();
               api.updateScene({{ elements: els.map((e) => e.id === 'big' ? {{ ...e, isDeleted: true, version: e.version + 1, versionNonce: 8 }} : e.id === 'oval' ? {{ ...e, x: e.x + 3, version: e.version + 1, versionNonce: 9 }} : e), captureUpdate: 'IMMEDIATELY' }});
               {}"#,
            calls::COLLECT
        ),
        "{}",
    );
    let parsed = collected
        .as_deref()
        .map_err(|e| e.clone())
        .and_then(calls::answer::<calls::Collected>);
    report.check(
        matches!(&parsed, Ok(c) if matches!(&c.scene, calls::CollectedScene::Whole(scene) if scene.get().contains("\"oval\"") && !scene.get().contains("\"big\"")) && !c.status.holds_more()),
        &format!(
            "collect answers a change not posted yet, as the app reads it ({:?})",
            parsed.as_ref().map(|c| (&c.rev, c.status.clone()))
        ),
    );

    // A pasted iframe with a script in its srcdoc runs nothing and is gone.
    let from = events.messages.borrow().len();
    let frame_script = format!(
        "<script>top.webkit.messageHandlers.{BRIDGE}.postMessage(JSON.stringify({{v:1,kind:'help'}}))</script>"
    );
    let iframe = json!({
        "type": "excalidraw/clipboard",
        "elements": [{
            "id": "web", "type": "iframe", "x": 100, "y": 900, "width": 300, "height": 200, "angle": 0,
            "strokeColor": "#1e1e1e", "backgroundColor": "transparent", "fillStyle": "solid",
            "strokeWidth": 2, "strokeStyle": "solid", "roughness": 1, "opacity": 100,
            "groupIds": [], "frameId": null, "index": "b01", "roundness": null, "seed": 3,
            "version": 1, "versionNonce": 4, "isDeleted": false, "updated": 1, "link": null,
            "locked": false, "boundElements": [],
            "customData": {"generationData": {"status": "done", "html": frame_script}}
        }],
        "files": {}
    });
    let pasted = call(
        &view,
        r#"const data = new DataTransfer();
           data.setData('text/plain', arg);
           document.querySelector('.excalidraw-container')?.focus();
           const event = new ClipboardEvent('paste', { clipboardData: data, bubbles: true, cancelable: true });
           document.dispatchEvent(event);
           await new Promise((r) => setTimeout(r, 300));
           const api = window.innerwordBoard.api;
           // The other way one comes: an undo, a change of the scene.
           const els = api.getSceneElementsIncludingDeleted();
           const web = JSON.parse(arg).elements[0];
           api.updateScene({ elements: [...els, { ...web, id: 'web2', index: null }], captureUpdate: 'IMMEDIATELY' });
           await new Promise((r) => setTimeout(r, 300));
           return '';"#,
        &iframe.to_string(),
    );
    wait(Duration::from_millis(700));
    let now = canvas(&view);
    report.check(
        pasted.is_ok()
            && !now
                .iter()
                .any(|e| e["type"] == "iframe" && e["isDeleted"] != true)
            && !messages(&events, from)
                .iter()
                .any(|m| matches!(m, Ok(Message::Help))),
        &format!(
            "an iframe element with a script in its srcdoc runs nothing and is gone ({pasted:?}, {:?})",
            now.iter()
                .filter(|e| e["type"] == "iframe")
                .map(|e| (e["id"].clone(), e["isDeleted"].clone()))
                .collect::<Vec<_>>()
        ),
    );

    // The tooltips name no key the window takes.
    let tooltips = call(
        &view,
        r#"const keys = window.innerwordBoard.keysTheWindowTakes;
           const found = [];
           for (const el of document.querySelectorAll('[title], [aria-keyshortcuts]')) {
             const said = (el.getAttribute('title') || '') + ' ' + (el.getAttribute('aria-keyshortcuts') || '');
             for (const key of keys) {
               const at = said.indexOf(key);
               if (at >= 0 && !/[A-Za-z+]/.test(said.charAt(at + key.length) || ' ')) found.push(said.trim());
             }
           }
           return JSON.stringify({ count: document.querySelectorAll('[title]').length, found });"#,
        "{}",
    );
    let tooltips: Value = tooltips
        .as_deref()
        .ok()
        .and_then(|t| serde_json::from_str(t).ok())
        .unwrap_or_default();
    report.check(
        tooltips["count"].as_u64().unwrap_or(0) > 5
            && tooltips["found"].as_array().is_some_and(|f| f.is_empty()),
        &format!("no tooltip names a key the window takes ({tooltips})"),
    );

    // `?` posts help.
    let from = events.messages.borrow().len();
    let _ = eval(
        &view,
        "document.activeElement && document.activeElement.blur && document.activeElement.blur(); \
         (document.querySelector('.excalidraw') || document).dispatchEvent(new KeyboardEvent('keydown', { key: '?', code: 'Slash', shiftKey: true, bubbles: true, cancelable: true })); 1",
    );
    let help = wait_for(&events, from, Duration::from_secs(3), |m| {
        matches!(m, Message::Help)
    });
    report.check(help.is_some(), "? posts help");

    let violations = reports(&view);
    report.check(
        violations.is_empty() && page_errors(&events).is_empty(),
        &format!(
            "the view made no report of the policy and no script error ({violations:?}, {:?})",
            page_errors(&events)
        ),
    );

    // collect answers from a parked view and from a hidden one.
    view.park();
    for (nonce, how, hide) in [(41, "parked", false), (42, "hidden", true)] {
        view.set_hidden(hide);
        let collected = call(
            &view,
            &format!(
                r#"const api = window.innerwordBoard.api;
                   api.updateScene({{ elements: api.getSceneElementsIncludingDeleted().map((e) => e.id === 'oval' ? {{ ...e, y: e.y + 2, version: e.version + 1, versionNonce: {nonce} }} : e), captureUpdate: 'IMMEDIATELY' }});
                   {}"#,
                calls::COLLECT
            ),
            "{}",
        )
        .and_then(|text| calls::answer::<calls::Collected>(&text));
        report.check(
            matches!(&collected, Ok(c) if matches!(c.scene, calls::CollectedScene::Whole(_))),
            &format!(
                "collect answers from a {how} view ({:?})",
                collected.as_ref().map(|c| c.rev)
            ),
        );
    }
}

/// `adopt` by the base rule, every row of section 4.5's table, then `undo` and `redo`.
fn adopted(report: &mut Report, assets: &Assets, rules: &Path) {
    let scene = stored_scene(json!([
        rectangle("r1", 0., "a0"),
        rectangle("r2", 200., "a1"),
        rectangle("r3", 400., "a2"),
        rectangle("r4", 600., "a3"),
        rectangle("r5", 800., "a4"),
        // A Rust-made text Excalidraw measures again, whose index it must also repair (out
        // of order after "a4"): its load raises the text's version past the stored one.
        rust_text("t1", "Measured again", "a0"),
    ]));
    let source = BundleSource::new(assets);
    source.set_json(
        BOARD_PATH,
        &json!({"stamp": STAMP, "scene": scene, "view": {}}),
    );
    let host = Host::new();
    let (view, events) = open(&host, "board-13-1700000000000", source, rules);
    if wait_for(&events, 0, Duration::from_secs(20), |m| {
        matches!(m, Message::Ready { .. })
    })
    .is_none()
    {
        report.check(
            false,
            &format!(
                "the adopt case's view is ready ({:?})",
                page_errors(&events)
            ),
        );
        return;
    }
    let loaded = canvas(&view);
    let t1_loaded = element(&loaded, "t1").cloned().unwrap_or_default();
    // Risk 12: the glue judges "untouched" by its own record of the versions on the canvas
    // at the base, not by the stored version an agent's delta carries, which Excalidraw can
    // have raised at the load. This one it did, so a glue that judged by baseVersion would
    // call the text a conflict.
    report.check(
        t1_loaded["version"].as_u64().unwrap_or(0) > 1
            && t1_loaded["width"].as_f64().unwrap_or(10.) != 10.,
        &format!(
            "a Rust-made text is measured again at the load, its version raised past the stored 1 (width {} version {})",
            t1_loaded["width"], t1_loaded["version"]
        ),
    );

    // The writer moves r2 and r4 after the base.
    let _ = eval(
        &view,
        r#"(() => { const api = window.innerwordBoard.api;
            api.updateScene({ elements: api.getSceneElementsIncludingDeleted().map((e) => (e.id === 'r2' || e.id === 'r4') ? { ...e, y: e.y + 50, version: e.version + 1, versionNonce: 21 } : e), captureUpdate: 'IMMEDIATELY' });
            return 1; })()"#,
    );
    wait(Duration::from_millis(400));
    let moved = |id: &str, dx: f64, version: u64| {
        let mut e = rectangle(id, 0., "a0");
        let base = element(&loaded, id).cloned().unwrap_or(e.clone());
        e = base;
        e["x"] = json!(e["x"].as_f64().unwrap_or(0.) + dx);
        e["version"] = json!(version);
        e["versionNonce"] = json!(1000 + version);
        e
    };
    // The agent's copy, as Rust makes it from the stored scene (S1, where the text is at
    // version 1, which is its baseVersion below), at S2's version 2.
    let mut t1_new = rust_text("t1", "Changed by an agent", "a5");
    t1_new["version"] = json!(2);
    t1_new["versionNonce"] = json!(4242);
    let delta = json!({
        "baseRev": 0,
        "upserted": [
            {"element": moved("r1", 1000., 4), "baseVersion": 3},
            {"element": moved("r2", 1000., 4), "baseVersion": 3},
            {"element": rectangle("n1", 1200., "a6"), "baseVersion": null},
            {"element": rectangle("ghost", 1400., "a7"), "baseVersion": 3},
            {"element": rectangle("r5", 2000., "a4"), "baseVersion": null},
            {"element": t1_new, "baseVersion": 1}
        ],
        "removed": [
            {"id": "r3", "baseVersion": 3},
            {"id": "r4", "baseVersion": 3}
        ],
        "files": {},
        "undoable": true
    });
    let answer = call(&view, calls::ADOPT, &delta.to_string())
        .and_then(|text| calls::answer::<calls::Adopted>(&text));
    let after = canvas(&view);
    let x = |id: &str| {
        element(&after, id)
            .and_then(|e| e["x"].as_f64())
            .unwrap_or(f64::NAN)
    };
    let x_before = |id: &str| {
        element(&loaded, id)
            .and_then(|e| e["x"].as_f64())
            .unwrap_or(f64::NAN)
    };
    match &answer {
        Ok(adopted) => {
            let mut conflicts = adopted.conflicts.clone();
            conflicts.sort();
            report.check(
                conflicts == ["ghost", "r2", "r4", "r5"],
                &format!("the conflicts are the elements the writer changed, deleted or gave a new id ({conflicts:?})"),
            );
            report.check(
                !adopted.same,
                "with conflicts the canvas is not the stored scene (same is false)",
            );
        }
        Err(why) => report.check(false, &format!("adopt answers ({why})")),
    }
    report.check(
        (x("r1") - (x_before("r1") + 1000.)).abs() < 0.01
            && element(&after, "r1")
                .and_then(|e| e["version"].as_u64())
                .unwrap_or(0)
                >= 4,
        &format!(
            "an untouched element takes the incoming copy at a version that only grew ({:?})",
            element(&after, "r1").map(|e| (&e["x"], &e["version"]))
        ),
    );
    report.check(
        (x("r2") - x_before("r2")).abs() < 0.01,
        "an element the writer moved after the base stays the writer's",
    );
    report.check(
        !live(&after, "r3") && live(&after, "r4"),
        "a removal deletes an untouched element and leaves one the writer changed",
    );
    report.check(
        live(&after, "n1") && !live(&after, "ghost"),
        "a new element comes in; one the canvas no longer has and the base had does not",
    );
    report.check(
        (x("r5") - x_before("r5")).abs() < 0.01,
        "an incoming element with no base whose id the canvas has stays the writer's",
    );
    report.check(
        element(&after, "t1").is_some_and(|e| {
            e["text"] == "Changed by an agent"
                && e["version"].as_u64().unwrap_or(0)
                    > t1_loaded["version"].as_u64().unwrap_or(u64::MAX)
        }) && answer.as_ref().is_ok_and(|a| !a.conflicts.iter().any(|c| c == "t1")),
        &format!(
            "a Rust-made text whose version the load raised past its baseVersion counts as untouched: the agent's copy is taken, at a version above the canvas's ({:?})",
            element(&after, "t1").map(|e| (&e["text"], &e["version"]))
        ),
    );

    // undo takes the change out, redo puts it back.
    let undone = call(&view, calls::UNDO, &calls::steps_argument(1));
    wait(Duration::from_millis(300));
    let back = canvas(&view);
    let undone_ok = element(&back, "r1").and_then(|e| e["x"].as_f64()) == Some(x_before("r1"))
        && !live(&back, "n1");
    report.check(
        undone.is_ok() && undone_ok,
        &format!(
            "undo (a dispatched cmd-z) takes the adopted change out ({undone:?}; r1 {:?}, n1 {:?}, t1 {:?})",
            element(&back, "r1").map(|e| (&e["x"], &e["version"])),
            element(&back, "n1").map(|e| (&e["isDeleted"], &e["version"])),
            element(&back, "t1").map(|e| &e["text"])
        ),
    );
    let redone = call(&view, calls::REDO, &calls::steps_argument(1));
    wait(Duration::from_millis(300));
    let again = canvas(&view);
    report.check(
        redone.is_ok()
            && (element(&again, "r1")
                .and_then(|e| e["x"].as_f64())
                .unwrap_or(0.)
                - (x_before("r1") + 1000.))
                .abs()
                < 0.01,
        &format!("redo puts it back ({redone:?})"),
    );

    // Without conflicts and with nothing unposted, the canvas is the stored scene.
    let collected =
        call(&view, calls::COLLECT, "{}").and_then(|text| calls::answer::<calls::Collected>(&text));
    let rev = collected.as_ref().map(|c| c.rev).unwrap_or(0);
    let now = canvas(&view);
    let r1 = element(&now, "r1").cloned().unwrap_or_default();
    let mut r1_next = r1.clone();
    r1_next["y"] = json!(r1["y"].as_f64().unwrap_or(0.) + 30.);
    r1_next["version"] = json!(r1["version"].as_u64().unwrap_or(0) + 1);
    let clean = json!({
        "baseRev": rev,
        "upserted": [{"element": r1_next, "baseVersion": r1["version"]}],
        "removed": [],
        "files": {},
        "undoable": true
    });
    let answer = call(&view, calls::ADOPT, &clean.to_string())
        .and_then(|text| calls::answer::<calls::Adopted>(&text));
    report.check(
        matches!(&answer, Ok(a) if a.same && a.conflicts.is_empty()),
        &format!(
            "an adopt without conflicts and without unposted changes is the same scene ({answer:?})"
        ),
    );

    // A change of the writer's not posted yet: the canvas holds more than the stored scene.
    let collected =
        call(&view, calls::COLLECT, "{}").and_then(|text| calls::answer::<calls::Collected>(&text));
    let rev = collected.as_ref().map(|c| c.rev).unwrap_or(0);
    let now = canvas(&view);
    let r1 = element(&now, "r1").cloned().unwrap_or_default();
    let mut r1_next = r1.clone();
    r1_next["y"] = json!(r1["y"].as_f64().unwrap_or(0.) + 30.);
    r1_next["version"] = json!(r1["version"].as_u64().unwrap_or(0) + 1);
    r1_next["versionNonce"] = json!(777);
    let delta = json!({
        "baseRev": rev,
        "upserted": [{"element": r1_next, "baseVersion": r1["version"]}],
        "removed": [], "files": {}, "undoable": true
    });
    let answer = call(
        &view,
        r#"const api = window.innerwordBoard.api;
           api.updateScene({ elements: api.getSceneElementsIncludingDeleted().map((e) => e.id === 'n1' ? { ...e, x: e.x + 9, version: e.version + 1, versionNonce: 31 } : e), captureUpdate: 'IMMEDIATELY' });
           return await window.innerwordBoard.call('adopt', arg);"#,
        &delta.to_string(),
    )
    .and_then(|text| calls::answer::<calls::Adopted>(&text));
    report.check(
        matches!(&answer, Ok(a) if !a.same && a.conflicts.is_empty()),
        &format!("with a change of the writer's not posted yet, same is false ({answer:?})"),
    );
    report.check(
        page_errors(&events).is_empty(),
        &format!(
            "the adopt case made no script error ({:?})",
            page_errors(&events)
        ),
    );
}

/// Mermaid to drawing under the whiteboard's policy.
fn mermaid(report: &mut Report, assets: &Assets, server: &Server, rules: &Path) {
    let source = BundleSource::new(assets);
    source.set_json(
        BOARD_PATH,
        &json!({"stamp": STAMP, "scene": stored_scene(json!([])), "view": {}}),
    );
    let host = Host::new();
    let (view, events) = open(&host, "board-14-1700000000000", source, rules);
    if wait_for(&events, 0, Duration::from_secs(20), |m| {
        matches!(m, Message::Ready { .. })
    })
    .is_none()
    {
        report.check(
            false,
            &format!(
                "the Mermaid case's view is ready ({:?})",
                page_errors(&events)
            ),
        );
        return;
    }
    let _ = call(&view, WATCH_REPORTS, "{}");

    let opened = call(&view, calls::MERMAID, &calls::mermaid_argument(None));
    let dialog = eval_until(
        &view,
        "(() => { const d = document.querySelector('.ttd-dialog'); return d ? d.textContent.slice(0, 200) : ''; })()",
        Duration::from_secs(5),
    );
    report.check(
        opened.is_ok() && dialog.to_lowercase().contains("mermaid"),
        &format!("the mermaid call opens Excalidraw's dialog ({opened:?}, {dialog:?})"),
    );
    let _ = eval(
        &view,
        "(() => { window.innerwordBoard.api.updateScene({ appState: { openDialog: null }, captureUpdate: 'NEVER' }); return 1; })()",
    );

    let insert = |definition: &str| {
        let started = Instant::now();
        let answer = call(
            &view,
            calls::MERMAID,
            &calls::mermaid_argument(Some(definition)),
        )
        .and_then(|text| calls::answer::<calls::Inserted>(&text));
        (answer, started.elapsed())
    };
    let (flow, took) =
        insert("flowchart LR\n  A[Idea] --> B{Good?}\n  B -->|yes| C[Write]\n  B -->|no| D[Drop]");
    report.note(&format!("a flowchart converted in {took:?}"));
    report.check(
        matches!(&flow, Ok(i) if i.ids.len() >= 8),
        &format!("a flowchart becomes elements ({flow:?})"),
    );
    let (sequence, _) = insert("sequenceDiagram\n  Alice->>Bob: Draft\n  Bob-->>Alice: Notes");
    report.check(
        matches!(&sequence, Ok(i) if i.ids.len() >= 4),
        &format!("a sequence diagram becomes elements ({sequence:?})"),
    );
    let from = events.messages.borrow().len();
    let (pie, _) = insert("pie title Words\n  \"Draft\" : 60\n  \"Notes\" : 40");
    let elements = canvas(&view);
    let picture = pie.as_ref().ok().and_then(|i| {
        i.ids
            .iter()
            .filter_map(|id| element(&elements, id))
            .find(|e| e["type"] == "image")
            .cloned()
    });
    let file_posted = wait_for(
        &events,
        from,
        Duration::from_secs(5),
        |m| matches!(m, Message::File { data_url, .. } if data_url.starts_with("data:image/svg+xml")),
    );
    report.check(
        picture.is_some() && file_posted.is_some(),
        &format!(
            "a pie chart becomes one SVG picture, posted as a file ({pie:?}, posted: {})",
            file_posted.is_some()
        ),
    );

    // Pasted as text.
    let before = canvas(&view)
        .iter()
        .filter(|e| e["isDeleted"] != true)
        .count();
    let focused = call(
        &view,
        r#"const data = new DataTransfer();
           data.setData('text/plain', JSON.parse(arg));
           // As after a click on the canvas: Excalidraw takes a paste only with the keyboard in it.
           document.querySelector('.excalidraw-container')?.focus();
           document.dispatchEvent(new ClipboardEvent('paste', { clipboardData: data, bubbles: true, cancelable: true }));
           return String(document.activeElement && document.activeElement.className);"#,
        &serde_json::to_string("flowchart TD\n  P[Pasted] --> Q[Text]").unwrap(),
    );
    let count = || {
        canvas(&view)
            .iter()
            .filter(|e| e["isDeleted"] != true)
            .count()
    };
    testing::run_until(Duration::from_secs(6), || count() >= before + 3);
    let after = count();
    report.check(
        after >= before + 3,
        &format!("a flowchart pasted as text becomes elements ({before} -> {after}, the keyboard in {focused:?})"),
    );

    let violations = reports(&view);
    report.check(
        violations.is_empty() && page_errors(&events).is_empty(),
        &format!(
            "Mermaid made no report of the policy and no script error ({violations:?}, {:?})",
            page_errors(&events)
        ),
    );

    // An image node naming an address: refused by img-src, nothing reaches the server.
    let image = server.url("mermaid.png");
    let (_, _) = insert(&format!(
        "flowchart LR\n  A@{{ img: \"{image}\", label: \"Picture\", w: 60, h: 60 }} --> B[Next]"
    ));
    wait(Duration::from_millis(800));
    let violations = reports(&view);
    report.check(
        violations
            .iter()
            .any(|v| v.starts_with("img-src") && v.contains("127.0.0.1")),
        &format!("an image node naming an address is refused by img-src ({violations:?})"),
    );
    // A label with HTML that would post a message posts none.
    let from = events.messages.borrow().len();
    let (_, _) = insert(&format!(
        "flowchart LR\n  A[\"<img src=x onerror=window.webkit.messageHandlers.{BRIDGE}.postMessage(JSON.stringify({{v:1,kind:'help'}}))>\"] --> B"
    ));
    wait(Duration::from_millis(800));
    report.check(
        !messages(&events, from)
            .iter()
            .any(|m| matches!(m, Ok(Message::Help))),
        "a label with an image whose onerror would post posts nothing",
    );

    // The extra tools' menu: Mermaid, no web embed.
    let menu = call(
        &view,
        r#"const trigger = document.querySelector('.App-toolbar__extra-tools-trigger');
           if (!trigger) return JSON.stringify({ error: 'no trigger' });
           trigger.click();
           await new Promise((r) => setTimeout(r, 400));
           const items = [...document.querySelectorAll('.App-toolbar__extra-tools-dropdown .dropdown-menu-item')]
             .filter((i) => i.style.display !== 'none' && i.offsetParent !== null)
             .map((i) => i.textContent.trim());
           trigger.click();
           return JSON.stringify({ items });"#,
        "{}",
    );
    let menu: Value = menu
        .as_deref()
        .ok()
        .and_then(|t| serde_json::from_str(t).ok())
        .unwrap_or_default();
    let items: Vec<String> = menu["items"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|i| i.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    report.check(
        items.iter().any(|i| i.contains("Mermaid"))
            && !items.iter().any(|i| i.to_lowercase().contains("embed")),
        &format!("the extra tools' menu has Mermaid to Excalidraw and no web embed ({menu})"),
    );
}

/// The shape library: served, added to, inserted from, refused, collected, and its
/// unreachable parts not shown.
fn shape_library(report: &mut Report, assets: &Assets, elements: &Value, rules: &Path) {
    let list = elements.as_array().cloned().unwrap_or_default();
    let item = |id: &str, ids: &[&str]| {
        json!({
            "id": id, "status": "unpublished", "created": 1_794_306_980_000u64,
            "elements": list.iter().filter(|e| ids.contains(&e["id"].as_str().unwrap_or(""))).cloned().collect::<Vec<_>>()
        })
    };
    let stored = json!({
        "type": "excalidrawlib", "version": 2, "source": "inner-word",
        "libraryItems": [item("first", &["box"]), item("second", &["note"])]
    });
    let source = BundleSource::new(assets);
    let mut with_picture = list.clone();
    with_picture.push(json!({
        "id": "pic", "type": "image", "x": 900, "y": 100, "width": 40, "height": 40, "angle": 0,
        "strokeColor": "transparent", "backgroundColor": "transparent", "fillStyle": "solid",
        "strokeWidth": 2, "strokeStyle": "solid", "roughness": 1, "opacity": 100,
        "groupIds": [], "frameId": null, "index": "b00", "roundness": null, "seed": 1,
        "version": 1, "versionNonce": 1, "isDeleted": false, "updated": 1, "link": null,
        "locked": false, "boundElements": [], "fileId": "missingfile", "status": "saved",
        "scale": [1, 1], "crop": null
    }));
    source.set_json(
        BOARD_PATH,
        &json!({"stamp": STAMP, "scene": stored_scene(Value::Array(with_picture)), "view": {}}),
    );
    source.set_json(LIBRARY_PATH, &stored);
    let host = Host::new();
    let (view, events) = open(&host, "board-15-1700000000000", source, rules);
    if wait_for(&events, 0, Duration::from_secs(20), |m| {
        matches!(m, Message::Ready { .. })
    })
    .is_none()
    {
        report.check(
            false,
            &format!(
                "the shape library's view is ready ({:?})",
                page_errors(&events)
            ),
        );
        return;
    }
    let _ = call(&view, WATCH_REPORTS, "{}");
    let shown = call(
        &view,
        calls::LIBRARY,
        &calls::library_argument(&calls::LibraryAction::Show),
    );
    let units = eval_until(
        &view,
        "(() => { const n = document.querySelectorAll('.library-unit').length; return n ? String(n) : ''; })()",
        Duration::from_secs(5),
    );
    report.check(
        shown.is_ok() && units == "2",
        &format!(
            "a served library of two items shows them in the sidebar ({shown:?}, {units:?} items)"
        ),
    );
    let hidden_parts = call(
        &view,
        r#"const menu = document.querySelector('.library-menu-dropdown-container button, [data-testid="lib-dropdown--button"], .library-actions-counter, .dropdown-menu-button');
           if (menu) { menu.click(); await new Promise((r) => setTimeout(r, 300)); }
           const visible = (el) => el && el.offsetParent !== null && getComputedStyle(el).display !== 'none' && getComputedStyle(el).visibility !== 'hidden';
           const found = [];
           if ([...document.querySelectorAll('.library-menu-browse-button')].some(visible)) found.push('browse');
           for (const id of ['lib-dropdown--load', 'lib-dropdown--export']) if ([...document.querySelectorAll(`[data-testid="${id}"]`)].some(visible)) found.push(id);
           for (const el of document.querySelectorAll('button, a, .dropdown-menu-item')) {
             const text = (el.textContent || '').trim().toLowerCase();
             if (visible(el) && (text.startsWith('browse libraries') || text.startsWith('publish'))) found.push(text);
           }
           if (menu) menu.click();
           return JSON.stringify(found);"#,
        "{}",
    );
    report.check(
        hidden_parts.as_deref() == Ok("[]"),
        &format!("Browse libraries, Publish, Load and Export are not shown ({hidden_parts:?})"),
    );

    // Add the selection (a shape with its label): posted with three items.
    let from = events.messages.borrow().len();
    let _ = eval(
        &view,
        "(() => { window.innerwordBoard.api.updateScene({ appState: { selectedElementIds: { oval: true } }, captureUpdate: 'NEVER' }); return 1; })()",
    );
    // Excalidraw takes a change of its state in React's next render.
    wait(Duration::from_millis(250));
    let added = call(
        &view,
        calls::LIBRARY,
        &calls::library_argument(&calls::LibraryAction::AddSelection),
    )
    .and_then(|text| calls::answer::<calls::LibraryAnswered>(&text));
    let posted = wait_for(
        &events,
        from,
        Duration::from_secs(3),
        |m| matches!(m, Message::Library { items, .. } if serde_json::from_str::<Vec<Value>>(items.get()).is_ok_and(|i| i.len() == 3)),
    );
    report.check(
        matches!(&added, Ok(a) if a.added.is_some() && a.refused.is_none()) && posted.is_some(),
        &format!(
            "Add to library on a selection posts library with three items ({added:?}, posted: {})",
            posted.is_some()
        ),
    );

    // Insert an item at the middle of the view.
    let inserted = call(
        &view,
        calls::LIBRARY,
        &calls::library_argument(&calls::LibraryAction::Insert("second".to_string())),
    )
    .and_then(|text| calls::answer::<calls::LibraryAnswered>(&text));
    let center =
        call(&view, calls::CENTER, "{}").and_then(|text| calls::answer::<calls::Center>(&text));
    let now = canvas(&view);
    let near_middle = match (&inserted, &center) {
        (Ok(i), Ok(c)) => i.ids.iter().filter_map(|id| element(&now, id)).any(|e| {
            let mx = e["x"].as_f64().unwrap_or(0.) + e["width"].as_f64().unwrap_or(0.) / 2.;
            let my = e["y"].as_f64().unwrap_or(0.) + e["height"].as_f64().unwrap_or(0.) / 2.;
            (mx - c.x).abs() < 2. && (my - c.y).abs() < 2.
        }),
        _ => false,
    };
    report.check(
        near_middle,
        &format!(
            "the library call inserts an item at the middle of the view ({inserted:?}, {center:?})"
        ),
    );

    // A selection with a picture is refused with Excalidraw's words.
    let _ = eval(
        &view,
        "(() => { window.innerwordBoard.api.updateScene({ appState: { selectedElementIds: { box: true, pic: true } }, captureUpdate: 'NEVER' }); return 1; })()",
    );
    // Excalidraw takes a change of its state in React's next render.
    wait(Duration::from_millis(250));
    let refused = call(
        &view,
        calls::LIBRARY,
        &calls::library_argument(&calls::LibraryAction::AddSelection),
    )
    .and_then(|text| calls::answer::<calls::LibraryAnswered>(&text));
    report.check(
        matches!(&refused, Ok(a) if a.added.is_none() && a.refused.as_deref().is_some_and(|r| !r.is_empty())),
        &format!("a selection with a picture is refused with a sentence ({refused:?}; {:?})",
            canvas(&view).iter().filter(|e| e["type"] == "image").map(|e| (e["id"].clone(), e["isDeleted"].clone())).collect::<Vec<_>>()),
    );

    // collect brings a change of the library not posted yet.
    let _ = eval(
        &view,
        "(() => { window.innerwordBoard.api.updateScene({ appState: { selectedElementIds: { note: true } }, captureUpdate: 'NEVER' }); return 1; })()",
    );
    // Excalidraw takes a change of its state in React's next render.
    wait(Duration::from_millis(250));
    let collected = call(
        &view,
        &format!("await window.innerwordBoard.call('library', JSON.stringify({{ addSelection: true }}));\n{}", calls::COLLECT),
        "{}",
    )
    .and_then(|text| calls::answer::<calls::Collected>(&text));
    report.check(
        matches!(&collected, Ok(c) if c.library.as_ref().is_some_and(|l| serde_json::from_str::<Vec<Value>>(l.items.get()).is_ok_and(|i| i.len() == 4))),
        &format!(
            "collect brings a change of the library not posted yet ({:?})",
            collected.as_ref().map(|c| c.library.as_ref().map(|l| l.lrev))
        ),
    );
    let violations = reports(&view);
    report.check(
        violations.is_empty(),
        &format!("the shape library made no report of the policy ({violations:?})"),
    );
}

// ---------------------------------------------------------------------------
// The glue holds what the writer does: the cases of B3's review fixes (`adopt` that waits
// first and then merges in one stretch, `state` outside the queue, a start that cannot read
// its scene, a shape library longer than one message), in a real view under the
// whiteboard's policy. They were written as a Swift harness beside B3's fixes and are kept
// here, where they run with every other case: against the dist of before those fixes
// (5f9f3b1 and 3008498), 15 of their checks fail, at least one in each case.

/// The six cases.
pub(super) fn the_glue_keeps_what_the_writer_does(report: &mut Report, rules: &Path) {
    let assets = match Assets::find() {
        Ok(assets) => assets,
        Err(why) => {
            report.check(false, &format!("the whiteboard's bundle is found ({why})"));
            return;
        }
    };
    adopt_while_a_font_loads(report, &assets, rules);
    adopt_during_a_stroke(report, &assets, rules);
    adopt_as_a_stroke_begins(report, &assets, rules);
    adopt_while_a_new_text_is_typed(report, &assets, rules);
    a_start_without_a_stored_scene(report, &assets, rules);
    a_library_longer_than_one_message(report, &assets, rules);
}

/// Three rectangles, r1 to r3, as `board.json` serves them.
fn three_rectangles(source: &BundleSource) {
    let scene = stored_scene(json!([
        rectangle("r1", 0., "a0"),
        rectangle("r2", 200., "a1"),
        rectangle("r3", 400., "a2"),
    ]));
    source.set_json(
        BOARD_PATH,
        &json!({"stamp": STAMP, "scene": scene, "view": {}}),
    );
    source.set_json(LIBRARY_PATH, &json!({}));
}

/// A whiteboard's view of [`three_rectangles`] in `host`, once it posted `ready` and
/// settled; `None` (and a failed check) when it did not.
fn ready_view(
    report: &mut Report,
    host: &Host,
    case: &str,
    view_host: &str,
    source: Arc<BundleSource>,
    rules: &Path,
) -> Option<(WebPage, Rc<Events>)> {
    let (view, events) = open(host, view_host, source, rules);
    if wait_for(&events, 0, Duration::from_secs(20), |m| {
        matches!(m, Message::Ready { .. })
    })
    .is_none()
    {
        report.check(
            false,
            &format!("{case}: the view is ready ({:?})", page_errors(&events)),
        );
        return None;
    }
    wait(Duration::from_millis(300));
    Some((view, events))
}

/// A call whose body answers JSON, read (`null` when it did not answer JSON).
fn page_json(view: &WebPage, body: &str) -> Value {
    call(view, body, "{}")
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(Value::Null)
}

/// The x of a live element on the canvas (NaN when there is none).
fn x_on_canvas(view: &WebPage, id: &str) -> f64 {
    page_json(
        view,
        &format!(
            "const e = window.innerwordBoard.api.getSceneElements().find((e) => e.id === '{id}'); return JSON.stringify(e ? e.x : null);"
        ),
    )
    .as_f64()
    .unwrap_or(f64::NAN)
}

/// The messages of a kind the view was posted, as JSON.
fn posted(events: &Events, kind: &str) -> Vec<Value> {
    events
        .messages
        .borrow()
        .iter()
        .filter_map(|body| serde_json::from_str::<Value>(body).ok())
        .filter(|message| message["kind"] == kind)
        .collect()
}

/// The x of an element in a posted `scene`.
fn x_in_post(post: &Value, id: &str) -> Option<f64> {
    post["scene"]["elements"]
        .as_array()?
        .iter()
        .find(|e| e["id"] == id)?["x"]
        .as_f64()
}

/// An agent's delta on the canvas's rev now: r1 moved by `dx`, and `extra` beside it.
fn r1_moved(view: &WebPage, dx: f64, extra: Vec<Value>) -> String {
    let rev = page_json(view, calls::COLLECT)["rev"]
        .as_i64()
        .unwrap_or(-1);
    let r1 = element(&canvas(view), "r1").cloned().unwrap_or_default();
    let mut moved = r1.clone();
    moved["x"] = json!(r1["x"].as_f64().unwrap_or(0.) + dx);
    moved["version"] = json!(r1["version"].as_u64().unwrap_or(0) + 1);
    moved["versionNonce"] = json!(4242);
    let mut upserted = vec![json!({"element": moved, "baseVersion": r1["version"]})];
    upserted.extend(extra);
    json!({"baseRev": rev, "upserted": upserted, "removed": [], "files": {}, "undoable": true})
        .to_string()
}

/// A pointer event dispatched on Excalidraw's interactive canvas, at (x, y) from its top
/// left (the writer's hand, as far as Excalidraw can tell).
fn pointer(view: &WebPage, kind: &str, x: f64, y: f64) {
    let buttons = if kind == "pointerup" { 0 } else { 1 };
    let _ = call(
        view,
        &format!(
            "const canvas = document.querySelector('canvas.interactive') ?? document.querySelector('.excalidraw__canvas.interactive');
             const r = canvas.getBoundingClientRect();
             canvas.dispatchEvent(new PointerEvent('{kind}', {{ clientX: r.left + {x}, clientY: r.top + {y}, pointerId: 1, pointerType: 'mouse', isPrimary: true, button: 0, buttons: {buttons}, bubbles: true, cancelable: true, composed: true, view: window }}));
             return '1';"
        ),
        "{}",
    );
}

/// The last freehand stroke on the canvas: its id, points, whether deleted, its last point,
/// and how many strokes there are.
fn last_stroke(view: &WebPage) -> Value {
    page_json(
        view,
        "const f = window.innerwordBoard.api.getSceneElementsIncludingDeleted().filter((e) => e.type === 'freedraw');
         const e = f[f.length - 1];
         return JSON.stringify(e ? { id: e.id, n: e.points.length, deleted: e.isDeleted, last: e.points[e.points.length - 1], count: f.length } : {});",
    )
}

/// Zoom 100 % and Excalidraw's tool `tool`.
fn take_tool(view: &WebPage, tool: &str) {
    let _ = call(
        view,
        calls::ZOOM,
        &calls::zoom_argument(calls::Zoom::To(1.0)),
    );
    let _ = call(
        view,
        &format!("window.innerwordBoard.api.setActiveTool({{ type: '{tool}' }}); return '1';"),
        "{}",
    );
    wait(Duration::from_millis(100));
}

/// Case 1: an adopt that waits for the font of an incoming text (held in flight here), while
/// the writer nudges another shape: the nudge stays, the answer is not `same`, and the
/// merge is posted at or above the answered rev, with the nudge.
fn adopt_while_a_font_loads(report: &mut Report, assets: &Assets, rules: &Path) {
    let case = "an adopt waiting for a font";
    let source = BundleSource::new(assets);
    three_rectangles(&source);
    let host = Host::new();
    let Some((view, events)) = ready_view(
        report,
        &host,
        case,
        "board-41-1700000000000",
        source.clone(),
        rules,
    ) else {
        return;
    };
    let loaded_fonts: Vec<String> = source
        .asked
        .lock()
        .unwrap()
        .iter()
        .filter(|p| p.starts_with("fonts/"))
        .cloned()
        .collect();
    report.note(&format!(
        "{case}: the fonts asked for at the load: {loaded_fonts:?}"
    ));
    source.hold("fonts/Lilita/");
    let mut lilita = rust_text("t7", "Lilita text from an agent", "a3");
    lilita["fontFamily"] = json!(7);
    let delta = r1_moved(
        &view,
        1000.,
        vec![json!({"element": lilita, "baseVersion": null})],
    );
    let x2_before = x_on_canvas(&view, "r2");
    let slot: Rc<RefCell<Option<Result<String, String>>>> = Rc::new(RefCell::new(None));
    let answer = slot.clone();
    view.call_async(calls::ADOPT, &delta, move |result| {
        *answer.borrow_mut() = Some(result)
    });
    testing::run_until(Duration::from_secs(2), || !source.held().is_empty());
    report.check(
        !source.held().is_empty() && slot.borrow().is_none(),
        &format!(
            "{case}: the font's load is held and the adopt waits for it ({:?})",
            source.held()
        ),
    );
    // The writer nudges r2 while the adopt waits.
    let _ = call(
        &view,
        "const api = window.innerwordBoard.api;
         api.updateScene({ elements: api.getSceneElementsIncludingDeleted().map((e) => e.id === 'r2' ? { ...e, x: e.x + 37, version: e.version + 1, versionNonce: 555 } : e), captureUpdate: 'IMMEDIATELY' });
         return '1';",
        "{}",
    );
    wait(Duration::from_millis(400));
    let nudge_posted = posted(&events, "scene")
        .iter()
        .any(|post| x_in_post(post, "r2") == Some(x2_before + 37.));
    report.note(&format!(
        "{case}: the nudge was posted while the adopt waited: {nudge_posted}"
    ));
    source.release();
    testing::run_until(Duration::from_secs(20), || slot.borrow().is_some());
    let adopted = slot
        .borrow_mut()
        .take()
        .unwrap_or_else(|| Err("no answer in 20 s".to_string()))
        .and_then(|text| calls::answer::<calls::Adopted>(&text));
    report.check(
        matches!(&adopted, Ok(a) if !a.same && a.conflicts.is_empty()),
        &format!("{case}: the answer is not same (the canvas holds a change of the writer's), without conflicts ({adopted:?})"),
    );
    let x2_after = x_on_canvas(&view, "r2");
    report.check(
        x2_after == x2_before + 37.,
        &format!(
            "{case}: the writer's nudge made while the adopt waited stays (r2.x {x2_before} -> {x2_after})"
        ),
    );
    let x1 = x_on_canvas(&view, "r1");
    let t7 = live(&canvas(&view), "t7");
    report.check(
        x1 == 1000. && t7,
        &format!("{case}: the agent's change is on the canvas (r1.x {x1}, t7 {t7})"),
    );
    wait(Duration::from_millis(400));
    let rev = adopted.as_ref().map(|a| a.rev).unwrap_or(u64::MAX);
    let after: Vec<Value> = posted(&events, "scene")
        .into_iter()
        .filter(|post| post["rev"].as_u64().is_some_and(|r| r >= rev))
        .collect();
    let posted_r2 = after.last().and_then(|post| x_in_post(post, "r2"));
    report.check(
        posted_r2 == Some(x2_before + 37.),
        &format!(
            "{case}: the merge is posted at or above the answered rev {rev}, with the nudge ({} posts, r2.x {posted_r2:?})",
            after.len()
        ),
    );
    let state = page_json(&view, calls::STATE);
    report.check(
        state["rev"].is_u64() && state["rev"] == state["posted"],
        &format!("{case}: nothing is left unposted ({state})"),
    );
    report.check(
        page_errors(&events).is_empty(),
        &format!("{case}: no page error ({:?})", page_errors(&events)),
    );
}

/// Case 2: an adopt in the middle of a freehand stroke: the whole stroke is on the canvas after
/// the pointer goes up, and in the last post.
fn adopt_during_a_stroke(report: &mut Report, assets: &Assets, rules: &Path) {
    let case = "an adopt during a stroke";
    let source = BundleSource::new(assets);
    three_rectangles(&source);
    let host = Host::new();
    let Some((view, events)) =
        ready_view(report, &host, case, "board-42-1700000000000", source, rules)
    else {
        return;
    };
    take_tool(&view, "freedraw");
    pointer(&view, "pointerdown", 300., 400.);
    for i in 1..=10 {
        pointer(
            &view,
            "pointermove",
            300. + i as f64 * 10.,
            400. + i as f64 * 3.,
        );
        wait(Duration::from_millis(35));
    }
    let middle = last_stroke(&view);
    let delta = r1_moved(&view, 500., Vec::new());
    let adopted =
        call(&view, calls::ADOPT, &delta).and_then(|text| calls::answer::<calls::Adopted>(&text));
    report.check(
        matches!(&adopted, Ok(a) if a.conflicts.is_empty()),
        &format!(
            "{case}: the adopt answers while the pointer is down ({adopted:?}; stroke {middle})"
        ),
    );
    for i in 11..=20 {
        pointer(
            &view,
            "pointermove",
            300. + i as f64 * 10.,
            400. + i as f64 * 3.,
        );
        wait(Duration::from_millis(35));
    }
    pointer(&view, "pointerup", 500., 460.);
    wait(Duration::from_millis(300));
    let end = last_stroke(&view);
    let n = end["n"].as_u64().unwrap_or(0);
    let last_x = end["last"][0].as_f64().unwrap_or(-1.);
    report.check(
        n > middle["n"].as_u64().unwrap_or(0) + 5
            && end["deleted"] == false
            && (last_x - 200.).abs() < 1.5,
        &format!("{case}: the whole stroke is on the canvas after the pointer went up (in the middle {middle}, at the end {end})"),
    );
    let x1 = x_on_canvas(&view, "r1");
    report.check(
        x1 == 500.,
        &format!("{case}: the agent's change is on the canvas (r1.x {x1})"),
    );
    let in_last_post = posted(&events, "scene").last().and_then(|post| {
        post["scene"]["elements"]
            .as_array()?
            .iter()
            .find(|e| e["type"] == "freedraw")
            .and_then(|e| e["points"].as_array().map(Vec::len))
    });
    report.check(
        in_last_post == Some(n as usize),
        &format!("{case}: the last post has the whole stroke ({in_last_post:?} of {n} points)"),
    );
    report.check(
        page_errors(&events).is_empty(),
        &format!("{case}: no page error ({:?})", page_errors(&events)),
    );
}

/// Case 3: an adopt right after a stroke has begun (too small yet for Excalidraw to keep in a
/// restore): the stroke stays, and is drawn whole.
fn adopt_as_a_stroke_begins(report: &mut Report, assets: &Assets, rules: &Path) {
    let case = "an adopt as a stroke begins";
    let source = BundleSource::new(assets);
    three_rectangles(&source);
    let host = Host::new();
    let Some((view, events)) =
        ready_view(report, &host, case, "board-43-1700000000000", source, rules)
    else {
        return;
    };
    take_tool(&view, "freedraw");
    pointer(&view, "pointerdown", 300., 400.);
    wait(Duration::from_millis(40));
    let begun = last_stroke(&view);
    let delta = r1_moved(&view, 500., Vec::new());
    let adopted = call(&view, calls::ADOPT, &delta);
    let right_after = last_stroke(&view);
    report.check(
        begun["id"].is_string() && right_after["id"] == begun["id"] && right_after["deleted"] == false,
        &format!("{case}: a stroke that has just begun is still there after the adopt (before {begun}, after {right_after}, {adopted:?})"),
    );
    for i in 1..=12 {
        pointer(&view, "pointermove", 300. + i as f64 * 10., 400.);
        wait(Duration::from_millis(35));
    }
    pointer(&view, "pointerup", 420., 400.);
    wait(Duration::from_millis(300));
    let end = last_stroke(&view);
    report.check(
        end["n"].as_u64().unwrap_or(0) >= 10 && end["deleted"] == false && end["count"] == 1,
        &format!("{case}: and it is drawn whole, one stroke ({end})"),
    );
    report.check(
        page_errors(&events).is_empty(),
        &format!("{case}: no page error ({:?})", page_errors(&events)),
    );
}

/// Case 4: an adopt while a new, empty text is being typed: the text stays and stays in its
/// editor, and what is typed after the adopt lands in it.
fn adopt_while_a_new_text_is_typed(report: &mut Report, assets: &Assets, rules: &Path) {
    let case = "an adopt while a new text is typed";
    let source = BundleSource::new(assets);
    three_rectangles(&source);
    let host = Host::new();
    let Some((view, events)) =
        ready_view(report, &host, case, "board-44-1700000000000", source, rules)
    else {
        return;
    };
    take_tool(&view, "text");
    pointer(&view, "pointerdown", 600., 500.);
    pointer(&view, "pointerup", 600., 500.);
    wait(Duration::from_millis(300));
    const EDITING: &str =
        "return JSON.stringify(!!document.querySelector('textarea.excalidraw-wysiwyg'));";
    let new_text = page_json(
        &view,
        "const t = window.innerwordBoard.api.getSceneElementsIncludingDeleted().filter((e) => e.type === 'text');
         const e = t[t.length - 1];
         return JSON.stringify(e ? { id: e.id, text: e.text, deleted: e.isDeleted } : {});",
    );
    report.check(
        page_json(&view, EDITING) == true && new_text["text"] == "",
        &format!("{case}: a new, empty text is being edited ({new_text})"),
    );
    let id = new_text["id"].as_str().unwrap_or("?").to_string();
    let text_of = |view: &WebPage| {
        page_json(
            view,
            &format!(
                "const e = window.innerwordBoard.api.getSceneElementsIncludingDeleted().find((e) => e.id === '{id}');
                 return JSON.stringify(e ? {{ text: e.text, deleted: e.isDeleted }} : null);"
            ),
        )
    };
    let delta = r1_moved(&view, 500., Vec::new());
    let adopted = call(&view, calls::ADOPT, &delta);
    let after_adopt = text_of(&view);
    let still_editing = page_json(&view, EDITING) == true;
    report.check(
        after_adopt["deleted"] == false && still_editing,
        &format!("{case}: after the adopt the text is there and still being edited ({after_adopt}, editing {still_editing}, {adopted:?})"),
    );
    let _ = call(
        &view,
        "const t = document.querySelector('textarea.excalidraw-wysiwyg');
         t.value = 'Typed after the adopt';
         t.dispatchEvent(new Event('input', { bubbles: true }));
         return '1';",
        "{}",
    );
    wait(Duration::from_millis(200));
    let _ = call(
        &view,
        "document.querySelector('textarea.excalidraw-wysiwyg').dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })); return '1';",
        "{}",
    );
    wait(Duration::from_millis(300));
    let committed = text_of(&view);
    report.check(
        committed["text"] == "Typed after the adopt" && committed["deleted"] == false,
        &format!("{case}: what is typed after the adopt lands in the text ({committed})"),
    );
    let x1 = x_on_canvas(&view, "r1");
    report.check(
        x1 == 500.,
        &format!("{case}: the agent's change is on the canvas (r1.x {x1})"),
    );
    report.check(
        page_errors(&events).is_empty(),
        &format!("{case}: no page error ({:?})", page_errors(&events)),
    );
}

/// Case 5: a `board.json` that is missing or is no stored scene: the start fails with an error
/// that says so (`bridge::START_FAILED`), no `ready` comes, and `state` answers at once
/// behind a call that waits for `ready`.
fn a_start_without_a_stored_scene(report: &mut Report, assets: &Assets, rules: &Path) {
    for (n, (what, board)) in [
        ("missing", None),
        ("not a stored scene", Some(r#"{"stamp": 5, "scene": []}"#)),
    ]
    .into_iter()
    .enumerate()
    {
        let case = format!("a board.json {what}");
        let source = BundleSource::new(assets);
        source.set_json(LIBRARY_PATH, &json!({}));
        if let Some(board) = board {
            source.set(BOARD_PATH, "application/json", board.as_bytes());
        }
        let host = Host::new();
        let (view, events) = open(
            &host,
            &format!("board-{}-1700000000000", 45 + n),
            source,
            rules,
        );
        let errored = wait_for(&events, 0, Duration::from_secs(10), |m| {
            matches!(m, Message::Error { .. })
        });
        let message = errored
            .and_then(|at| match messages(&events, at).into_iter().next() {
                Some(Ok(Message::Error { message })) => Some(message),
                _ => None,
            })
            .unwrap_or_default();
        report.check(
            message.starts_with(bridge::START_FAILED),
            &format!("{case}: the start fails with an error that says so ({message:?})"),
        );
        // A call that waits for `ready`, and `state` behind it.
        view.call_async(calls::COLLECT, "{}", |_| {});
        wait(Duration::from_millis(100));
        let started = Instant::now();
        let state = call(&view, calls::STATE, "{}");
        let took = started.elapsed();
        let rev = state
            .as_ref()
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(text).ok())
            .map(|value| value["rev"].clone())
            .unwrap_or(Value::Null);
        report.check(
            took < Duration::from_millis(500) && rev.is_u64(),
            &format!(
                "{case}: state answers at once behind a call that waits for ready ({took:?}, {state:?})"
            ),
        );
        wait(Duration::from_millis(500));
        report.check(
            posted(&events, "ready").is_empty(),
            &format!("{case}: no ready is posted"),
        );
    }
}

/// Case 6: a shape library longer than one message: posted small, as too large and without its
/// items (and not counted as posted), and `collect` brings it whole.
fn a_library_longer_than_one_message(report: &mut Report, assets: &Assets, rules: &Path) {
    let case = "a library longer than one message";
    let source = BundleSource::new(assets);
    three_rectangles(&source);
    let host = Host::new();
    let Some((view, events)) =
        ready_view(report, &host, case, "board-47-1700000000000", source, rules)
    else {
        return;
    };
    let bytes = MAX_MESSAGE_BYTES + 1024 * 1024;
    let _ = call(
        &view,
        &format!(
            "const api = window.innerwordBoard.api;
             const base = api.getSceneElements()[0];
             const big = 'x'.repeat({bytes});
             await api.updateLibrary({{ libraryItems: [{{ id: 'huge', status: 'unpublished', created: 1, elements: [{{ ...base, id: 'lib1', customData: {{ big }} }}] }}], merge: false, openLibraryMenu: false }});
             return '1';"
        ),
        "{}",
    );
    wait(Duration::from_millis(800));
    let posts = posted(&events, "library");
    let last = posts.last().cloned().unwrap_or_default();
    report.check(
        last["tooLarge"] == true && last["items"].is_null(),
        &format!(
            "{case}: it is posted as too large, without its items ({} posts, the last {})",
            posts.len(),
            short_json(&last)
        ),
    );
    report.check(
        view.stats().messages_dropped == 0,
        &format!(
            "{case}: nothing the bridge would drop was posted ({:?})",
            view.stats()
        ),
    );
    let collected =
        call(&view, calls::COLLECT, "{}").and_then(|text| calls::answer::<calls::Collected>(&text));
    let items: Vec<Value> = collected
        .as_ref()
        .ok()
        .and_then(|c| c.library.as_ref())
        .and_then(|library| serde_json::from_str(library.items.get()).ok())
        .unwrap_or_default();
    report.check(
        items.len() == 1 && items[0]["id"] == "huge",
        &format!(
            "{case}: collect brings it whole ({} items{})",
            items.len(),
            collected
                .as_ref()
                .err()
                .map(|e| format!(", {e}"))
                .unwrap_or_default()
        ),
    );
}

/// A message, short enough for a line.
fn short_json(value: &Value) -> String {
    let text = value.to_string();
    if text.len() > 300 {
        let head: String = text.chars().take(300).collect();
        format!("{head}… ({} bytes)", text.len())
    } else {
        text
    }
}
