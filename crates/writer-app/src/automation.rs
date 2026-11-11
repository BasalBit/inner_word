//! Debug automation hook: drive the running app from a tiny script.
//!
//! When `THE_WRITER_AUTOMATION` is set, the app runs the script against its
//! main window after startup, dispatching keystrokes and text through GPUI
//! exactly as if they were typed (bindings first, then the focused input
//! handler). Progress and errors go to stderr, prefixed with `[automation]`.
//!
//! # Grammar
//!
//! ```text
//! script  := step ((';' | newline) step)*
//! step    := name [':' argument]          leading/trailing whitespace ignored
//! comment := '#' ... end of line          only at the start of a line
//! ```
//!
//! | step                  | effect                                                       |
//! |-----------------------|--------------------------------------------------------------|
//! | `wait:<ms>`           | sleep                                                        |
//! | `keys:<keystrokes>`   | GPUI keystrokes separated by spaces: `keys:cmd-shift-p escape` |
//! | `type:<text>`         | types each character (escapes: `\;` `\n` `\t` `\s` `\\`)     |
//! | `action:<name>`       | dispatches a registered action by name: `action:app::Quit`   |
//! | `resize:<w>x<h>`      | resizes the window content                                   |
//! | `theme:<mode>`        | `light`, `dark` or `system`                                  |
//! | `screenshot:<path>`   | captures only this window to a PNG (waits for a fresh frame) |
//! | `log:<text>`          | prints the text                                              |
//! | `info`                | prints window bounds, scale factor, appearance, theme, focus |
//! | `quit`                | quits the app (runs the quit hooks)                          |
//!
//! `THE_WRITER_AUTOMATION=@/path/to/script.txt` reads the script from a file.
//! A step that fails is reported and the script continues; `quit` still runs.
//!
//! There are no mouse steps: `Window::dispatch_event` is public in gpui 0.2.2
//! but returns a crate-private type, so it cannot be called from outside gpui.
//! Drive everything through the keyboard (which the product requires anyway).
//!
//! Example:
//!
//! ```sh
//! THE_WRITER_DATA_DIR=$(mktemp -d) \
//! THE_WRITER_AUTOMATION='wait:800; type:Hello world; keys:cmd-a; screenshot:/tmp/tw.png; quit' \
//! cargo run -p writer-app
//! ```

use std::{path::PathBuf, time::Duration};

use anyhow::{Context as _, Result, anyhow, bail};
use gpui::{
    AnyWindowHandle, App, AsyncApp, Keystroke, Modifiers, px, size,
};
use writer_ui::theme::{self, ActiveTheme, ThemeMode};

use crate::screenshot;

/// Environment variable holding the script (or `@path`).
pub const AUTOMATION_ENV: &str = "THE_WRITER_AUTOMATION";

/// One parsed step.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Wait(Duration),
    Keys(Vec<Keystroke>),
    Type(String),
    Action(String),
    Resize(f32, f32),
    Theme(ThemeMode),
    Screenshot(PathBuf),
    Log(String),
    Info,
    Quit,
}

/// A parsed script.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Script {
    pub steps: Vec<Step>,
}

impl Script {
    /// Reads `THE_WRITER_AUTOMATION`. `Ok(None)` when unset or empty.
    pub fn from_env() -> Result<Option<Self>> {
        let Some(raw) = std::env::var_os(AUTOMATION_ENV) else {
            return Ok(None);
        };
        let raw = raw
            .into_string()
            .map_err(|_| anyhow!("{AUTOMATION_ENV} is not valid UTF-8"))?;
        if raw.trim().is_empty() {
            return Ok(None);
        }
        let source = match raw.trim().strip_prefix('@') {
            Some(path) => std::fs::read_to_string(path)
                .with_context(|| format!("reading automation script {path}"))?,
            None => raw,
        };
        Self::parse(&source).map(Some)
    }

    /// Parses the grammar described in the module docs.
    pub fn parse(source: &str) -> Result<Self> {
        let mut steps = Vec::new();
        for (ix, raw_step) in split_steps(source).into_iter().enumerate() {
            let step = raw_step.trim();
            if step.is_empty() {
                continue;
            }
            let (name, arg) = match step.split_once(':') {
                Some((name, arg)) => (name.trim(), arg.trim()),
                None => (step, ""),
            };
            let parsed = parse_step(name, arg)
                .with_context(|| format!("automation step {} ({step:?})", ix + 1))?;
            steps.push(parsed);
        }
        Ok(Self { steps })
    }
}

/// Splits on unescaped `;` and newlines, drops `#` comment lines, and keeps
/// escape sequences for [`unescape`].
fn split_steps(source: &str) -> Vec<String> {
    let mut steps = Vec::new();
    for line in source.lines() {
        if line.trim_start().starts_with('#') {
            continue;
        }
        let mut current = String::new();
        let mut chars = line.chars();
        while let Some(ch) = chars.next() {
            match ch {
                '\\' => {
                    current.push('\\');
                    if let Some(next) = chars.next() {
                        current.push(next);
                    }
                }
                ';' => steps.push(std::mem::take(&mut current)),
                _ => current.push(ch),
            }
        }
        steps.push(current);
    }
    steps
}

fn unescape(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('s') => out.push(' '),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

fn parse_step(name: &str, arg: &str) -> Result<Step> {
    Ok(match name.to_ascii_lowercase().as_str() {
        "wait" | "sleep" => Step::Wait(Duration::from_millis(
            arg.parse().context("expected milliseconds")?,
        )),
        "keys" | "key" => {
            if arg.is_empty() {
                bail!("expected keystrokes");
            }
            let keys = arg
                .split_whitespace()
                .map(|key| Keystroke::parse(key).map_err(|err| anyhow!("{err}")))
                .collect::<Result<Vec<_>>>()?;
            Step::Keys(keys)
        }
        "type" | "text" => Step::Type(unescape(arg)),
        "action" => {
            if arg.is_empty() {
                bail!("expected an action name");
            }
            Step::Action(arg.to_string())
        }
        "resize" => {
            let (w, h) = arg
                .split_once(['x', 'X'])
                .ok_or_else(|| anyhow!("expected <width>x<height>"))?;
            Step::Resize(w.trim().parse()?, h.trim().parse()?)
        }
        "theme" => Step::Theme(
            ThemeMode::parse(arg).ok_or_else(|| anyhow!("expected light, dark or system"))?,
        ),
        "screenshot" | "capture" => {
            if arg.is_empty() {
                bail!("expected a file path");
            }
            Step::Screenshot(PathBuf::from(unescape(arg)))
        }
        "log" | "echo" => Step::Log(unescape(arg)),
        "info" => Step::Info,
        "quit" | "exit" => Step::Quit,
        other => bail!("unknown step {other:?}"),
    })
}

/// The keystroke GPUI would see when typing `ch` on a US layout.
pub fn keystroke_for_char(ch: char) -> Keystroke {
    match ch {
        '\n' | '\r' => Keystroke::parse("enter").expect("valid keystroke"),
        '\t' => Keystroke::parse("tab").expect("valid keystroke"),
        ' ' => Keystroke::parse("space").expect("valid keystroke"),
        _ => {
            let shift = ch.is_uppercase();
            Keystroke {
                modifiers: Modifiers {
                    shift,
                    ..Default::default()
                },
                key: ch.to_lowercase().collect(),
                key_char: Some(ch.to_string()),
            }
        }
    }
}

/// Starts running `script` against `window`. Returns immediately.
pub fn run(script: Script, window: AnyWindowHandle, cx: &mut App) {
    eprintln!("[automation] running {} steps", script.steps.len());
    cx.spawn(async move |cx| {
        for (ix, step) in script.steps.into_iter().enumerate() {
            let label = format!("{step:?}");
            match execute(step, window, cx).await {
                Ok(StepOutcome::Continue) => {}
                Ok(StepOutcome::Quit) => {
                    eprintln!("[automation] step {}: quit", ix + 1);
                    cx.update(|cx| cx.quit()).ok();
                    return;
                }
                Err(err) => eprintln!("[automation] step {} {label} failed: {err:#}", ix + 1),
            }
        }
        eprintln!("[automation] done");
    })
    .detach();
}

enum StepOutcome {
    Continue,
    Quit,
}

async fn execute(step: Step, window: AnyWindowHandle, cx: &mut AsyncApp) -> Result<StepOutcome> {
    match step {
        Step::Wait(duration) => cx.background_executor().timer(duration).await,
        Step::Keys(keys) => {
            for keystroke in keys {
                let text = keystroke.unparse();
                let handled =
                    window.update(cx, |_, window, cx| window.dispatch_keystroke(keystroke, cx))?;
                eprintln!(
                    "[automation] keys {text}: {}",
                    if handled { "handled" } else { "not handled" }
                );
                frame_pause(cx).await;
            }
        }
        Step::Type(text) => {
            // One update per character, like real typing: effects (notify,
            // redraw) are flushed between keystrokes.
            for ch in text.chars() {
                window.update(cx, |_, window, cx| {
                    window.dispatch_keystroke(keystroke_for_char(ch), cx);
                })?;
            }
            eprintln!("[automation] typed {} characters", text.chars().count());
            frame_pause(cx).await;
        }
        Step::Action(name) => {
            window.update(cx, |_, window, cx| -> Result<()> {
                let action = cx
                    .build_action(&name, None)
                    .map_err(|err| anyhow!("{err}"))?;
                window.dispatch_action(action, cx);
                Ok(())
            })??;
            eprintln!("[automation] dispatched {name}");
            frame_pause(cx).await;
        }
        Step::Resize(width, height) => {
            window.update(cx, |_, window, _| window.resize(size(px(width), px(height))))?;
            cx.background_executor()
                .timer(Duration::from_millis(150))
                .await;
        }
        Step::Theme(mode) => {
            cx.update(|cx| theme::set_theme_mode(mode, cx))?;
            frame_pause(cx).await;
        }
        Step::Screenshot(path) => {
            // Make sure the latest state is on screen before capturing.
            window.update(cx, |_, window, _| window.refresh())?;
            cx.background_executor()
                .timer(Duration::from_millis(250))
                .await;
            let number = window.update(cx, |_, window, _| screenshot::window_number(window))??;
            let path = absolute(path);
            let task = cx
                .background_executor()
                .spawn(async move { screenshot::capture_window(number, &path) });
            let written = task.await?;
            eprintln!(
                "[automation] screenshot of window {number} written to {}",
                written.display()
            );
        }
        Step::Log(text) => eprintln!("[automation] {text}"),
        Step::Info => {
            let info = window.update(cx, |_, window, cx| {
                format!(
                    "window bounds {:?}, viewport {:?}, scale {}, appearance {:?}, active {}, \
                     focused {}, theme {} ({})",
                    window.bounds(),
                    window.viewport_size(),
                    window.scale_factor(),
                    window.appearance(),
                    window.is_window_active(),
                    window.focused(cx).is_some(),
                    cx.theme().name,
                    theme::theme_mode(cx).as_str(),
                )
            })?;
            eprintln!("[automation] {info}");
        }
        Step::Quit => return Ok(StepOutcome::Quit),
    }
    Ok(StepOutcome::Continue)
}

/// Gives the window a chance to render between steps (about two frames).
async fn frame_pause(cx: &mut AsyncApp) {
    cx.background_executor()
        .timer(Duration::from_millis(32))
        .await;
}

fn absolute(path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .map(|dir| dir.join(&path))
            .unwrap_or(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_example_script() {
        let script = Script::parse(
            "wait:500; keys:cmd-n; type:Hello world; keys:cmd-b; screenshot:/tmp/x.png; quit",
        )
        .unwrap();
        assert_eq!(
            script.steps,
            vec![
                Step::Wait(Duration::from_millis(500)),
                Step::Keys(vec![Keystroke::parse("cmd-n").unwrap()]),
                Step::Type("Hello world".into()),
                Step::Keys(vec![Keystroke::parse("cmd-b").unwrap()]),
                Step::Screenshot("/tmp/x.png".into()),
                Step::Quit,
            ]
        );
    }

    #[test]
    fn multi_line_comments_and_escapes() {
        let script = Script::parse(
            "# a comment\nkeys: cmd-shift-p escape\ntype:a\\;b\\nc\\s\nresize:800x600\ntheme:dark\naction:app::Quit\n\n",
        )
        .unwrap();
        assert_eq!(
            script.steps,
            vec![
                Step::Keys(vec![
                    Keystroke::parse("cmd-shift-p").unwrap(),
                    Keystroke::parse("escape").unwrap()
                ]),
                Step::Type("a;b\nc ".into()),
                Step::Resize(800., 600.),
                Step::Theme(ThemeMode::Dark),
                Step::Action("app::Quit".into()),
            ]
        );
    }

    #[test]
    fn rejects_bad_steps() {
        assert!(Script::parse("wait:soon").is_err());
        assert!(Script::parse("keys:cmd-opt-s").is_err());
        assert!(Script::parse("dance").is_err());
        assert!(Script::parse("theme:blue").is_err());
    }

    #[test]
    fn keystrokes_for_characters() {
        let upper = keystroke_for_char('H');
        assert!(upper.modifiers.shift);
        assert_eq!(upper.key, "h");
        assert_eq!(upper.key_char.as_deref(), Some("H"));
        assert_eq!(keystroke_for_char(' ').key, "space");
        assert_eq!(keystroke_for_char('\n').key, "enter");
        assert_eq!(keystroke_for_char('ö').key_char.as_deref(), Some("ö"));
    }
}
