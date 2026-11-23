//! The `board:` steps of a scripted run (`crate::automation`; the grammar is
//! in `docs/whiteboard-design.md`, section 6.9).
//!
//! | Step | What it does |
//! |---|---|
//! | `board:new[:<title>]` | makes a whiteboard as New Whiteboard does, named `<title>`, and opens it; logs `board: made board:12 “Plan”` |
//! | `board:wait[:<ms>]` | waits until the open whiteboard's page posted `ready` (10 s unless given) |
//! | `board:scene` | asks the page for what it has not posted, saves it, and logs the listing of the whiteboard as the app holds it |
//! | `board:draw:<json>` | converts skeletons (an array, or one) with the converter agents use and gives them to the page as the writer's drawing; logs the ids |
//! | `board:press:<keys>` | sends real `keyDown:`/`keyUp:` events to the web view (`shift-h i`), as AppKit hands them to the first responder; not the menu bar's half |
//! | `board:js:<script>` | evaluates a script in the page and logs the answer |
//! | `board:pdf:<path>` | writes the whiteboard's PDF and logs its size |
//! | `board:menu` | opens the whiteboard's menu, as `ctrl-enter` |
//! | `board:library` | logs the shape library as the app holds it: `board: library 3 items, saved` |
//! | `board:mermaid:<definition>` | converts a Mermaid definition (`\n` for a new line) as the dialog's Insert does; logs the ids |

use std::{path::PathBuf, time::Duration};

use anyhow::{Context as _, Result, anyhow, bail};
use gpui::{AnyWindowHandle, AsyncApp};

/// What a `board:` step does.
#[derive(Clone, Debug, PartialEq)]
pub enum BoardStep {
    New(Option<String>),
    Wait(Duration),
    Scene,
    Draw(String),
    Press(String),
    Js(String),
    Pdf(PathBuf),
    Menu,
    Library,
    Mermaid(String),
}

/// How long `board:wait` waits unless the step says otherwise.
pub const BOARD_WAIT: Duration = Duration::from_secs(10);

/// Reads the part of a `board:` step after `board:`.
pub fn parse(arg: &str, unescape: impl Fn(&str) -> String) -> Result<BoardStep> {
    let (what, rest) = arg.split_once(':').unwrap_or((arg, ""));
    Ok(match what.trim() {
        "new" => BoardStep::New(Some(unescape(rest.trim())).filter(|title| !title.is_empty())),
        "wait" => BoardStep::Wait(if rest.trim().is_empty() {
            BOARD_WAIT
        } else {
            Duration::from_millis(rest.trim().parse().context("expected milliseconds")?)
        }),
        "scene" => BoardStep::Scene,
        "draw" if !rest.trim().is_empty() => BoardStep::Draw(rest.to_string()),
        "press" if !rest.trim().is_empty() => BoardStep::Press(rest.trim().to_string()),
        "js" if !rest.trim().is_empty() => BoardStep::Js(unescape(rest)),
        "pdf" if !rest.trim().is_empty() => BoardStep::Pdf(PathBuf::from(unescape(rest.trim()))),
        "menu" => BoardStep::Menu,
        "library" => BoardStep::Library,
        "mermaid" if !rest.trim().is_empty() => BoardStep::Mermaid(unescape(rest)),
        other => bail!(
            "unknown board step {other:?} (new, wait, scene, draw:<json>, press:<keys>, \
             js:<script>, pdf:<path>, menu, library, mermaid:<definition>)"
        ),
    })
}

/// Carries out one `board:` step.
pub async fn run(step: BoardStep, window: AnyWindowHandle, cx: &mut AsyncApp) -> Result<()> {
    let workspace = cx
        .update(|_| window.downcast::<crate::Workspace>())?
        .ok_or_else(|| anyhow!("the window does not show a workspace"))?;
    match step {
        BoardStep::New(title) => {
            let made = workspace.update(cx, |workspace, window, cx| {
                workspace.new_board(title.as_deref(), window, cx)
            })?;
            let Some(id) = made else {
                bail!("board:new: no whiteboard was made");
            };
            let title = workspace.update(cx, |workspace, _, _| {
                workspace.page_title().unwrap_or_default().to_string()
            })?;
            eprintln!(
                "[automation] board: made board:{} \u{201c}{}\u{201d}",
                id.0,
                crate::workspace::session_title(&title)
            );
        }
        BoardStep::Wait(limit) => {
            let started = std::time::Instant::now();
            let ready = workspace.update(cx, |workspace, _, _| workspace.board_ready_answer())?;
            let Some(ready) = ready else {
                bail!("board:wait: no whiteboard is open");
            };
            let timer = cx.background_executor().timer(limit);
            match crate::pages::within(ready, timer).await.flatten() {
                Some(()) => eprintln!(
                    "[automation] board: ready after {:.0} ms",
                    started.elapsed().as_secs_f64() * 1000.
                ),
                None => {
                    let state = workspace.update(cx, |workspace, _, _| workspace.page_state())?;
                    bail!(
                        "board:wait: the whiteboard was not ready within {} ms ({state:?})",
                        limit.as_millis()
                    );
                }
            }
        }
        BoardStep::Scene => {
            let collect = workspace.update(cx, |workspace, _, cx| {
                workspace
                    .open_board_id()
                    .map(|id| workspace.collect_board(id, cx))
            })?;
            let Some(collect) = collect else {
                bail!("board:scene: no whiteboard is open");
            };
            let saved = collect.await;
            let listing =
                workspace.update(cx, |workspace, _, cx| workspace.board_listing_text(cx))?;
            eprintln!(
                "[automation] board: scene{}\n{listing}",
                if saved { "" } else { " (not saved)" }
            );
        }
        BoardStep::Draw(json) => {
            let value: serde_json::Value =
                serde_json::from_str(&json).context("board:draw: the skeletons are no JSON")?;
            let skeletons = match value {
                serde_json::Value::Array(items) => items,
                one => vec![one],
            };
            let drawn = workspace.update(cx, |workspace, window, cx| {
                workspace.draw_on_board(skeletons, false, window, cx)
            })?;
            match drawn {
                Ok(ids) => eprintln!("[automation] board: drew {}", ids.join(", ")),
                Err(why) => bail!("board:draw: {why}"),
            }
        }
        BoardStep::Press(keys) => {
            let view = workspace.update(cx, |workspace, _, _| workspace.board_native_view())?;
            let Some(view) = view else {
                bail!("board:press: no whiteboard is shown in WebKit");
            };
            for stroke in keys.split_whitespace() {
                crate::board_steps::press(view, stroke)?;
                cx.background_executor()
                    .timer(Duration::from_millis(30))
                    .await;
            }
        }
        BoardStep::Js(script) => {
            let answer =
                workspace.update(cx, |workspace, _, _| workspace.evaluate_in_page(&script))?;
            let Some(answer) = answer else {
                bail!("board:js: no whiteboard is shown");
            };
            match answer.await {
                Some(Ok(value)) => eprintln!("[automation] board: js answered {value:?}"),
                Some(Err(error)) => eprintln!("[automation] board: js failed: {error}"),
                None => eprintln!("[automation] board: js got no answer"),
            }
        }
        BoardStep::Pdf(path) => {
            let started = std::time::Instant::now();
            let job = workspace.update(cx, |workspace, window, cx| {
                workspace.board_pdf_now(window, cx)
            })?;
            let printed = job
                .await
                .map_err(|why| anyhow!("board:pdf: the PDF was not made: {why}"))?;
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("creating {}", parent.display()))?;
            }
            std::fs::write(&path, &printed.bytes)
                .with_context(|| format!("writing {}", path.display()))?;
            let pictures = workspace.update(cx, |workspace, _, _| {
                workspace
                    .board_scene()
                    .map(|scene| scene.summary().pictures)
                    .unwrap_or(0)
            })?;
            eprintln!(
                "[automation] board: pdf {} page{}, {:.0} x {:.0} pt, {pictures} picture{}, made in {:.2} s",
                printed.pages,
                if printed.pages == 1 { "" } else { "s" },
                printed.paper.0,
                printed.paper.1,
                if pictures == 1 { "" } else { "s" },
                started.elapsed().as_secs_f64()
            );
        }
        BoardStep::Menu => {
            workspace.update(cx, |workspace, window, cx| {
                if let Some(focus) = workspace.page_focus_handle() {
                    window.focus(&focus);
                }
                window.dispatch_action(Box::new(crate::workspace::OpenBoardContextMenu), cx);
            })?;
        }
        BoardStep::Library => {
            let line =
                workspace.update(cx, |workspace, _, _| workspace.describe_shape_library())?;
            eprintln!("[automation] board: library {line}");
        }
        BoardStep::Mermaid(definition) => {
            let answer = workspace.update(cx, |workspace, _, _| {
                workspace.open_board_id().and_then(|id| {
                    workspace.board_call(
                        id,
                        innerword_board_web::calls::MERMAID,
                        innerword_board_web::calls::mermaid_argument(Some(&definition)),
                    )
                })
            })?;
            let Some(answer) = answer else {
                bail!("board:mermaid: no whiteboard is shown");
            };
            match answer.await {
                Some(Ok(text)) => {
                    let ids = innerword_board_web::calls::answer::<
                        innerword_board_web::calls::Inserted,
                    >(&text)
                    .map(|inserted| inserted.ids)
                    .unwrap_or_default();
                    eprintln!("[automation] board: mermaid made {}", ids.join(", "));
                }
                Some(Err(error)) => bail!("board:mermaid: {error}"),
                None => bail!("board:mermaid: no answer"),
            }
        }
    }
    Ok(())
}

/// Sends one keystroke (`cmd-a`, `shift-h`, `escape`) to the web view at
/// `view` as AppKit hands a key to the first responder: `keyDown:` then
/// `keyUp:` (a `cmd` chord is offered as a key equivalent first).
#[allow(unexpected_cfgs)]
pub fn press(view: *mut std::ffi::c_void, stroke: &str) -> Result<()> {
    use objc::{
        class, msg_send,
        runtime::{BOOL, NO, Object},
        sel, sel_impl,
    };
    let keystroke =
        gpui::Keystroke::parse(stroke).map_err(|_| anyhow!("board:press: {stroke:?} is no key"))?;
    let (code, characters) = mac_key(&keystroke.key, keystroke.modifiers.shift)
        .ok_or_else(|| anyhow!("board:press: the key {:?} is not known", keystroke.key))?;
    let mut flags: usize = 0;
    if keystroke.modifiers.shift {
        flags |= 1 << 17;
    }
    if keystroke.modifiers.control {
        flags |= 1 << 18;
    }
    if keystroke.modifiers.alt {
        flags |= 1 << 19;
    }
    if keystroke.modifiers.platform {
        flags |= 1 << 20;
    }
    if matches!(code, 123..=126) {
        // The arrows are function keys on the numeric pad, to AppKit.
        flags |= (1 << 23) | (1 << 21);
    }
    let view = view as *mut Object;
    // SAFETY: AppKit's own classes and selectors, on the main thread, with
    // a live web view; the strings live until the events are made.
    unsafe {
        let window: *mut Object = msg_send![view, window];
        if window.is_null() {
            bail!("board:press: the web view has no window");
        }
        let number: isize = msg_send![window, windowNumber];
        let text = ns_string(&characters);
        let uptime: f64 = {
            let info: *mut Object = msg_send![class!(NSProcessInfo), processInfo];
            msg_send![info, systemUptime]
        };
        let make = |kind: usize| -> *mut Object {
            msg_send![class!(NSEvent),
                keyEventWithType: kind
                location: CGPoint { x: 0.0, y: 0.0 }
                modifierFlags: flags
                timestamp: uptime
                windowNumber: number
                context: std::ptr::null_mut::<Object>()
                characters: text
                charactersIgnoringModifiers: text
                isARepeat: NO
                keyCode: code]
        };
        let down = make(10);
        let up = make(11);
        let mut handled: BOOL = NO;
        if keystroke.modifiers.platform {
            handled = msg_send![view, performKeyEquivalent: down];
        }
        if handled == NO {
            let _: () = msg_send![view, keyDown: down];
        }
        let _: () = msg_send![view, keyUp: up];
    }
    Ok(())
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[allow(unexpected_cfgs)]
unsafe impl objc::Encode for CGPoint {
    fn encode() -> objc::Encoding {
        // SAFETY: the encoding of `{CGPoint=dd}`.
        unsafe { objc::Encoding::from_str("{CGPoint=dd}") }
    }
}

#[allow(unexpected_cfgs)]
fn ns_string(text: &str) -> *mut objc::runtime::Object {
    use objc::{class, msg_send, sel, sel_impl};
    let bytes = std::ffi::CString::new(text).unwrap_or_default();
    // SAFETY: Foundation's own constructor, with a NUL-terminated UTF-8
    // string; the object is autoreleased.
    unsafe { msg_send![class!(NSString), stringWithUTF8String: bytes.as_ptr()] }
}

/// The virtual key code of `key` on a US keyboard and the characters it
/// types (with `shift`).
fn mac_key(key: &str, shift: bool) -> Option<(u16, String)> {
    let named = match key {
        "enter" => Some((36, "\r")),
        "tab" => Some((48, "\t")),
        "space" => Some((49, " ")),
        "backspace" => Some((51, "\u{7f}")),
        "escape" => Some((53, "\u{1b}")),
        "delete" => Some((117, "\u{F728}")),
        "left" => Some((123, "\u{F702}")),
        "right" => Some((124, "\u{F703}")),
        "down" => Some((125, "\u{F701}")),
        "up" => Some((126, "\u{F700}")),
        _ => None,
    };
    if let Some((code, text)) = named {
        return Some((code, text.to_string()));
    }
    let table: [(char, u16); 47] = [
        ('a', 0),
        ('s', 1),
        ('d', 2),
        ('f', 3),
        ('h', 4),
        ('g', 5),
        ('z', 6),
        ('x', 7),
        ('c', 8),
        ('v', 9),
        ('b', 11),
        ('q', 12),
        ('w', 13),
        ('e', 14),
        ('r', 15),
        ('y', 16),
        ('t', 17),
        ('1', 18),
        ('2', 19),
        ('3', 20),
        ('4', 21),
        ('6', 22),
        ('5', 23),
        ('=', 24),
        ('9', 25),
        ('7', 26),
        ('-', 27),
        ('8', 28),
        ('0', 29),
        (']', 30),
        ('o', 31),
        ('u', 32),
        ('[', 33),
        ('i', 34),
        ('p', 35),
        ('l', 37),
        ('j', 38),
        ('\'', 39),
        ('k', 40),
        (';', 41),
        ('\\', 42),
        (',', 43),
        ('/', 44),
        ('n', 45),
        ('m', 46),
        ('.', 47),
        ('`', 50),
    ];
    let mut chars = key.chars();
    let ch = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    let lower = ch.to_ascii_lowercase();
    let code = table
        .iter()
        .find(|(known, _)| *known == lower)
        .map(|(_, code)| *code)?;
    let text = if shift {
        ch.to_ascii_uppercase().to_string()
    } else {
        ch.to_string()
    };
    Some((code, text))
}
