//! A small copy of writer-app's visual-QA hook (writer-editor cannot depend on
//! writer-app). Same script syntax, subset of steps:
//! `wait:<ms>`, `keys:<keystrokes>`, `type:<text>`, `typeslow:<text>`,
//! `bench:<count> <keystrokes>`, `theme:<light|dark|system>`,
//! `screenshot:<path>`, `print:<path>` (the document as a PDF, as the
//! print pass lays it out), `log:<text>`, `quit`, and for runs that post real
//! mouse events to the process: `frame` (draw one; a background window
//! draws none by itself), `state` (selection, selected text, scroll,
//! link tooltip, to-dos, last event) and `where:<block>:<offset>` (the
//! window point of a position, which is what a posted click needs).
//! Steps are separated by `;` or newlines; `THE_WRITER_AUTOMATION=@file`
//! reads a file. `typeslow` types one character per frame and `bench`
//! presses a sequence of keys `count` times; both draw a frame after
//! every key and print what a key costs: dispatching it (handlers and
//! events) and that plus the frame it causes.

use std::{
    path::PathBuf,
    process::Command,
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result, anyhow, bail};
use gpui::{AnyWindowHandle, App, Keystroke, Modifiers, Window};
use writer_ui::theme::{self, ThemeMode};

pub const ENV: &str = "THE_WRITER_AUTOMATION";

#[derive(Debug)]
enum Step {
    Wait(Duration),
    Keys(Vec<Keystroke>),
    Type(String),
    TypeSlow(String),
    Bench(usize, Vec<Keystroke>),
    Theme(ThemeMode),
    Screenshot(PathBuf),
    Print(PathBuf),
    Log(String),
    Frame,
    State,
    Where(usize, usize),
    Quit,
}

fn parse(source: &str) -> Result<Vec<Step>> {
    let mut steps = Vec::new();
    for raw in source.split(['\n', ';']) {
        let raw = raw.trim();
        if raw.is_empty() || raw.starts_with('#') {
            continue;
        }
        let (name, arg) = raw.split_once(':').unwrap_or((raw, ""));
        steps.push(match name.trim() {
            "wait" => Step::Wait(Duration::from_millis(arg.trim().parse()?)),
            "keys" => Step::Keys(
                arg.split_whitespace()
                    .map(|k| Keystroke::parse(k).map_err(|e| anyhow!("{e}")))
                    .collect::<Result<_>>()?,
            ),
            "type" => Step::Type(arg.replace("\\s", " ")),
            "typeslow" => Step::TypeSlow(arg.replace("\\s", " ")),
            "bench" => {
                let mut words = arg.split_whitespace();
                let count = words
                    .next()
                    .ok_or_else(|| anyhow!("bench needs a count"))?
                    .parse()?;
                Step::Bench(
                    count,
                    words
                        .map(|k| Keystroke::parse(k).map_err(|e| anyhow!("{e}")))
                        .collect::<Result<_>>()?,
                )
            }
            "theme" => {
                Step::Theme(ThemeMode::parse(arg).ok_or_else(|| anyhow!("unknown theme {arg:?}"))?)
            }
            "screenshot" => Step::Screenshot(PathBuf::from(arg.trim())),
            "print" => Step::Print(PathBuf::from(arg.trim())),
            "log" => Step::Log(arg.to_string()),
            "frame" => Step::Frame,
            "state" => Step::State,
            "where" => {
                let (block, offset) = arg
                    .split_once(':')
                    .ok_or_else(|| anyhow!("where needs <block>:<offset>"))?;
                Step::Where(block.trim().parse()?, offset.trim().parse()?)
            }
            "quit" => Step::Quit,
            other => bail!("unknown step {other:?}"),
        });
    }
    Ok(steps)
}

fn keystroke_for_char(ch: char) -> Keystroke {
    match ch {
        ' ' => Keystroke::parse("space").unwrap(),
        _ => Keystroke {
            modifiers: Modifiers {
                shift: ch.is_uppercase(),
                ..Default::default()
            },
            key: ch.to_lowercase().collect(),
            key_char: Some(ch.to_string()),
        },
    }
}

/// Runs the script from `THE_WRITER_AUTOMATION`, if set.
pub fn run_from_env(window: AnyWindowHandle, cx: &mut App) {
    let Ok(source) = std::env::var(ENV) else {
        return;
    };
    let source = match source.strip_prefix('@') {
        Some(path) => match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) => return eprintln!("[automation] reading {path}: {err}"),
        },
        None => source,
    };
    let steps = match parse(&source) {
        Ok(steps) => steps,
        Err(err) => return eprintln!("[automation] bad script: {err:#}"),
    };
    cx.spawn(async move |cx| {
        // The view to draw through while the window is covered.
        let view = window
            .update(cx, |_, window, _| native_view(window).ok())
            .ok()
            .flatten();
        for step in steps {
            let label = format!("{step:?}");
            let result: Result<bool> = async {
                match step {
                    Step::Wait(d) => cx.background_executor().timer(d).await,
                    Step::Keys(keys) => {
                        for key in keys {
                            let text = key.unparse();
                            let started = Instant::now();
                            let handled = window
                                .update(cx, |_, window, cx| window.dispatch_keystroke(key, cx))?;
                            let dispatch = started.elapsed();
                            draw_frame(window, view, cx);
                            eprintln!(
                                "[automation] keys {text}: handled={handled} \
                                 (dispatch {dispatch:?}, with frame {:?})",
                                started.elapsed()
                            );
                            cx.background_executor()
                                .timer(Duration::from_millis(60))
                                .await;
                        }
                    }
                    Step::Type(text) => {
                        for ch in text.chars() {
                            window.update(cx, |_, window, cx| {
                                window.dispatch_keystroke(keystroke_for_char(ch), cx);
                            })?;
                            draw_frame(window, view, cx);
                        }
                        cx.background_executor()
                            .timer(Duration::from_millis(60))
                            .await;
                    }
                    Step::TypeSlow(text) => {
                        // What one key costs: dispatching it (handlers
                        // and events), and that plus the frame it causes.
                        let mut costs = Vec::new();
                        let mut totals = Vec::new();
                        for ch in text.chars() {
                            let started = Instant::now();
                            window.update(cx, |_, window, cx| {
                                window.dispatch_keystroke(keystroke_for_char(ch), cx);
                            })?;
                            costs.push(started.elapsed());
                            draw_frame(window, view, cx);
                            totals.push(started.elapsed());
                            cx.background_executor()
                                .timer(Duration::from_millis(24))
                                .await;
                        }
                        costs.sort();
                        totals.sort();
                        eprintln!(
                            "[automation] typeslow {} keys: dispatch median {:?}, max {:?}; \
                             key to frame done median {:?}, max {:?}",
                            costs.len(),
                            costs.get(costs.len() / 2).copied().unwrap_or_default(),
                            costs.last().copied().unwrap_or_default(),
                            totals.get(totals.len() / 2).copied().unwrap_or_default(),
                            totals.last().copied().unwrap_or_default(),
                        );
                    }
                    Step::Bench(count, keys) => {
                        let mut costs = Vec::new();
                        let mut totals = Vec::new();
                        for _ in 0..count {
                            for key in &keys {
                                let key = key.clone();
                                let started = Instant::now();
                                window.update(cx, |_, window, cx| {
                                    window.dispatch_keystroke(key, cx);
                                })?;
                                costs.push(started.elapsed());
                                draw_frame(window, view, cx);
                                totals.push(started.elapsed());
                                // Let timers and background work run as
                                // they would between two real keys.
                                cx.background_executor()
                                    .timer(Duration::from_millis(12))
                                    .await;
                            }
                        }
                        costs.sort();
                        totals.sort();
                        let label: Vec<String> = keys.iter().map(|key| key.unparse()).collect();
                        eprintln!(
                            "[automation] bench {} x {count}: dispatch median {:?}, max {:?}; \
                             key to frame done median {:?}, p95 {:?}, max {:?}",
                            label.join(" "),
                            costs.get(costs.len() / 2).copied().unwrap_or_default(),
                            costs.last().copied().unwrap_or_default(),
                            totals.get(totals.len() / 2).copied().unwrap_or_default(),
                            totals
                                .get(totals.len() * 95 / 100)
                                .copied()
                                .unwrap_or_default(),
                            totals.last().copied().unwrap_or_default(),
                        );
                    }
                    Step::Theme(mode) => {
                        cx.update(|cx| theme::set_theme_mode(mode, cx))?;
                        cx.background_executor()
                            .timer(Duration::from_millis(60))
                            .await;
                    }
                    Step::Screenshot(path) => {
                        window.update(cx, |_, window, _| window.refresh())?;
                        draw_frame(window, view, cx);
                        cx.background_executor()
                            .timer(Duration::from_millis(300))
                            .await;
                        let number = window.update(cx, |_, window, _| window_number(window))??;
                        cx.background_executor()
                            .spawn(async move { capture(number, &path) })
                            .await?;
                    }
                    Step::Print(path) => {
                        let line = window.update(cx, |root, window, cx| {
                            root.downcast::<crate::Demo>()
                                .map(|demo| demo.read(cx).print_to(&path, window, cx))
                                .unwrap_or_default()
                        })?;
                        eprintln!("[automation] print {}: {line}", path.display());
                    }
                    Step::Log(text) => eprintln!("[automation] {text}"),
                    Step::Frame => {
                        window.update(cx, |_, window, _| window.refresh())?;
                        draw_frame(window, view, cx);
                    }
                    Step::State => {
                        window.update(cx, |_, window, _| window.refresh())?;
                        draw_frame(window, view, cx);
                        let line = window.update(cx, |root, _, cx| {
                            root.downcast::<crate::Demo>()
                                .map(|demo| demo.read(cx).state_line(cx))
                                .unwrap_or_default()
                        })?;
                        eprintln!("[automation] state {line}");
                    }
                    Step::Where(block, offset) => {
                        let at = window.update(cx, |root, _, cx| {
                            root.downcast::<crate::Demo>()
                                .ok()
                                .and_then(|demo| demo.read(cx).where_is(block, offset, cx))
                        })?;
                        match at {
                            Some((x, y)) => {
                                eprintln!("[automation] where {block}:{offset} {x:.1} {y:.1}")
                            }
                            None => eprintln!("[automation] where {block}:{offset} unknown"),
                        }
                    }
                    Step::Quit => return Ok(true),
                }
                Ok(false)
            }
            .await;
            match result {
                Ok(true) => {
                    cx.update(|cx| cx.quit()).ok();
                    return;
                }
                Ok(false) => {}
                Err(err) => eprintln!("[automation] {label} failed: {err:#}"),
            }
        }
    })
    .detach();
}

/// The native view behind the window, for [`draw_frame`].
#[derive(Clone, Copy)]
struct NativeView(usize);

#[cfg(target_os = "macos")]
fn native_view(window: &Window) -> Result<NativeView> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let handle = HasWindowHandle::window_handle(window).map_err(|e| anyhow!("{e}"))?;
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        bail!("not an AppKit window");
    };
    Ok(NativeView(appkit.ns_view.as_ptr() as usize))
}

#[cfg(not(target_os = "macos"))]
fn native_view(_: &Window) -> Result<NativeView> {
    bail!("macOS only")
}

/// Draws a frame now if the window is dirty. GPUI draws from a display
/// link, which macOS stops while a window is completely covered by other
/// windows; a scripted run stays in the background, where that happens
/// easily. Sending the view `displayLayer:` is the path AppKit itself uses
/// for a synchronous frame (same as writer-app's `screenshot::force_frame`).
/// Must be called outside any GPUI update.
#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)]
fn draw_frame(window: AnyWindowHandle, view: Option<NativeView>, cx: &mut gpui::AsyncApp) {
    use objc::{msg_send, runtime::Object, sel, sel_impl};
    let Some(view) = view else {
        return;
    };
    // The view pointer is only good while the window is open.
    let open = cx
        .update(|cx| cx.windows().contains(&window))
        .unwrap_or(false);
    if !open {
        return;
    }
    let ns_view = view.0 as *mut Object;
    // SAFETY: the NSView of a window that is still open, on the main
    // thread; `layer` is a getter and `displayLayer:` is implemented by
    // GPUI's view class.
    unsafe {
        let layer: *mut Object = msg_send![ns_view, layer];
        let _: () = msg_send![ns_view, displayLayer: layer];
    }
}

#[cfg(not(target_os = "macos"))]
fn draw_frame(_: AnyWindowHandle, _: Option<NativeView>, _: &mut gpui::AsyncApp) {}

#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)]
fn window_number(window: &Window) -> Result<u32> {
    use objc::{msg_send, runtime::Object, sel, sel_impl};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let handle = HasWindowHandle::window_handle(window).map_err(|e| anyhow!("{e}"))?;
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        bail!("not an AppKit window");
    };
    let ns_view = appkit.ns_view.as_ptr() as *mut Object;
    // SAFETY: live NSView of this window; plain AppKit getters on the main thread.
    let number: isize = unsafe {
        let ns_window: *mut Object = msg_send![ns_view, window];
        if ns_window.is_null() {
            bail!("view not in a window");
        }
        msg_send![ns_window, windowNumber]
    };
    Ok(number as u32)
}

#[cfg(not(target_os = "macos"))]
fn window_number(_: &Window) -> Result<u32> {
    bail!("macOS only")
}

fn capture(number: u32, path: &PathBuf) -> Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let output = Command::new("/usr/sbin/screencapture")
        .args(["-x", "-o", &format!("-l{number}"), "-t", "png"])
        .arg(path)
        .output()
        .context("running screencapture")?;
    if !output.status.success() {
        bail!("screencapture: {}", String::from_utf8_lossy(&output.stderr));
    }
    eprintln!("[automation] screenshot {}", path.display());
    Ok(())
}
