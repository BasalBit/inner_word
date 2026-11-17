//! A small copy of innerword-app's visual-QA hook (innerword-spell cannot depend
//! on innerword-app). Same script syntax, a subset of its steps and a few of
//! its own:
//! `wait:<ms>`, `keys:<keystrokes>`, `type:<text>`, `typeslow:<text>`,
//! `choose:<label>` (that item of the open context menu),
//! `theme:<light|dark|system>`, `screenshot:<path>`, `log:<text>`, `state`
//! (one line about the checker, the selection and the open menu),
//! `lang:<auto|identifier>`, `book:<blocks>` (opens a generated book and
//! prints when what is on screen was marked), `settle` (waits until every
//! block is checked and prints how long that took and what the slices
//! cost) and `quit`.
//! Steps are separated by `;` or newlines (write the key `cmd-;` as
//! `cmd-\;`); `INNER_WORD_AUTOMATION=@file` reads a file. `typeslow` types one character per frame and prints what
//! a key costs: dispatching it (handlers and events) and that plus the
//! frame it causes.

use std::{
    path::PathBuf,
    process::Command,
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result, anyhow, bail};
use gpui::{AnyWindowHandle, App, Keystroke, Modifiers, Window};
use innerword_ui::theme::{self, ThemeMode};

pub const ENV: &str = "INNER_WORD_AUTOMATION";

#[derive(Debug)]
enum Step {
    Wait(Duration),
    Keys(Vec<Keystroke>),
    Type(String),
    TypeSlow(String),
    Choose(String),
    Theme(ThemeMode),
    Screenshot(PathBuf),
    Log(String),
    State,
    Language(String),
    Book(usize),
    Settle,
    Quit,
}

fn parse(source: &str) -> Result<Vec<Step>> {
    let mut steps = Vec::new();
    // `;` separates steps unless it is written `\;` (the key `cmd-;`).
    let source = source.replace("\\;", "\u{1}");
    for raw in source.split(['\n', ';']) {
        let raw = raw.replace('\u{1}', ";");
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
            "choose" => Step::Choose(arg.trim().to_string()),
            "lang" => Step::Language(arg.trim().to_string()),
            "book" => Step::Book(arg.trim().parse()?),
            "settle" => Step::Settle,
            "theme" => {
                Step::Theme(ThemeMode::parse(arg).ok_or_else(|| anyhow!("unknown theme {arg:?}"))?)
            }
            "screenshot" => Step::Screenshot(PathBuf::from(arg.trim())),
            "log" => Step::Log(arg.to_string()),
            "state" => Step::State,
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

/// Runs the script from `INNER_WORD_AUTOMATION`, if set.
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
                    Step::Log(text) => eprintln!("[automation] {text}"),
                    Step::State => {
                        window.update(cx, |_, window, _| window.refresh())?;
                        draw_frame(window, view, cx);
                        let line = window.update(cx, |root, window, cx| {
                            let menu = innerword_ui::ContextMenu::current(window, cx)
                                .map(|menu| menu.read(cx).labels());
                            root.downcast::<crate::Demo>()
                                .map(|demo| demo.read(cx).state_line(menu, cx))
                                .unwrap_or_default()
                        })?;
                        eprintln!("[automation] state {line}");
                    }
                    Step::Choose(title) => {
                        let chosen = window.update(cx, |_, window, cx| {
                            innerword_ui::ContextMenu::choose(&title, window, cx)
                        })?;
                        draw_frame(window, view, cx);
                        eprintln!("[automation] choose {title:?}: {chosen}");
                    }
                    Step::Language(name) => {
                        let language = innerword_spell::SpellLanguage::from_setting(&name);
                        window.update(cx, |root, _, cx| {
                            if let Ok(demo) = root.downcast::<crate::Demo>() {
                                demo.update(cx, |demo, cx| demo.set_language(language, cx));
                            }
                        })?;
                        draw_frame(window, view, cx);
                    }
                    Step::Book(blocks) => {
                        let started = Instant::now();
                        let document = crate::book(blocks);
                        let built = started.elapsed();
                        let demo = window.update(cx, |root, _, _| root.downcast::<crate::Demo>())?;
                        let Ok(demo) = demo else {
                            bail!("no demo view");
                        };
                        let started = Instant::now();
                        demo.update(cx, |demo, cx| {
                            demo.editor.update(cx, |editor, cx| editor.set_document(document, cx));
                            demo.checker.update(cx, |checker, _| checker.reset_slice_stats());
                        })?;
                        draw_frame(window, view, cx);
                        let opened = started.elapsed();
                        // Until what is on screen is marked.
                        let mut marked_after = None;
                        for _ in 0..400 {
                            let marks = demo.update(cx, |demo, cx| {
                                let editor = demo.editor.read(cx);
                                let visible = editor.visible_range().map(|range| range.start.block..range.end.block + 1);
                                visible.is_some_and(|visible| {
                                    editor.first_unchecked_spelling_block(visible).is_none()
                                })
                            })?;
                            if marks {
                                marked_after = Some(started.elapsed());
                                break;
                            }
                            cx.background_executor().timer(Duration::from_millis(1)).await;
                        }
                        draw_frame(window, view, cx);
                        eprintln!(
                            "[automation] book {blocks}: built in {built:?}, set and first frame {opened:?}, \
                             what is on screen checked after {marked_after:?}"
                        );
                    }
                    Step::Settle => {
                        let demo = window.update(cx, |root, _, _| root.downcast::<crate::Demo>())?;
                        let Ok(demo) = demo else {
                            bail!("no demo view");
                        };
                        let started = Instant::now();
                        loop {
                            let settled = demo.update(cx, |demo, cx| {
                                let checker = demo.checker.read(cx);
                                checker.is_settled(cx) || !checker.is_active(cx)
                            })?;
                            if settled || started.elapsed() > Duration::from_secs(120) {
                                break;
                            }
                            cx.background_executor().timer(Duration::from_millis(20)).await;
                        }
                        let (slices, longest, slow, marks) = demo.update(cx, |demo, cx| {
                            let checker = demo.checker.read(cx);
                            let (slices, longest) = checker.slice_stats();
                            (
                                slices,
                                longest,
                                checker.slow_slice_count(),
                                checker.misspelling_count(cx),
                            )
                        })?;
                        draw_frame(window, view, cx);
                        eprintln!(
                            "[automation] settle: {:?}, {slices} slices, the longest {longest:?}, \
                             {slow} over twice the budget, {marks} marked",
                            started.elapsed()
                        );
                        demo.update(cx, |demo, cx| {
                            demo.checker.update(cx, |checker, _| checker.reset_slice_stats())
                        })?;
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
/// for a synchronous frame (same as innerword-app's `screenshot::force_frame`).
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
