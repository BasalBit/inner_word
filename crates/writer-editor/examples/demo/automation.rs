//! A small copy of writer-app's visual-QA hook (writer-editor cannot depend on
//! writer-app). Same script syntax, subset of steps:
//! `wait:<ms>`, `keys:<keystrokes>`, `type:<text>`, `theme:<light|dark|system>`,
//! `screenshot:<path>`, `log:<text>`, `quit`. Steps are separated by `;` or
//! newlines; `THE_WRITER_AUTOMATION=@file` reads a file.

use std::{path::PathBuf, process::Command, time::Duration};

use anyhow::{Context as _, Result, anyhow, bail};
use gpui::{AnyWindowHandle, App, Keystroke, Modifiers, Window};
use writer_ui::theme::{self, ThemeMode};

pub const ENV: &str = "THE_WRITER_AUTOMATION";

#[derive(Debug)]
enum Step {
    Wait(Duration),
    Keys(Vec<Keystroke>),
    Type(String),
    Theme(ThemeMode),
    Screenshot(PathBuf),
    Log(String),
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
            "theme" => {
                Step::Theme(ThemeMode::parse(arg).ok_or_else(|| anyhow!("unknown theme {arg:?}"))?)
            }
            "screenshot" => Step::Screenshot(PathBuf::from(arg.trim())),
            "log" => Step::Log(arg.to_string()),
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
        for step in steps {
            let label = format!("{step:?}");
            let result: Result<bool> = async {
                match step {
                    Step::Wait(d) => cx.background_executor().timer(d).await,
                    Step::Keys(keys) => {
                        for key in keys {
                            let text = key.unparse();
                            let handled = window
                                .update(cx, |_, window, cx| window.dispatch_keystroke(key, cx))?;
                            eprintln!("[automation] keys {text}: handled={handled}");
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
                        }
                        cx.background_executor()
                            .timer(Duration::from_millis(60))
                            .await;
                    }
                    Step::Theme(mode) => {
                        cx.update(|cx| theme::set_theme_mode(mode, cx))?;
                        cx.background_executor()
                            .timer(Duration::from_millis(60))
                            .await;
                    }
                    Step::Screenshot(path) => {
                        window.update(cx, |_, window, _| window.refresh())?;
                        cx.background_executor()
                            .timer(Duration::from_millis(300))
                            .await;
                        let number = window.update(cx, |_, window, _| window_number(window))??;
                        cx.background_executor()
                            .spawn(async move { capture(number, &path) })
                            .await?;
                    }
                    Step::Log(text) => eprintln!("[automation] {text}"),
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
