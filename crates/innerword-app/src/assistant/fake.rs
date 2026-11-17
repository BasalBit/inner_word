//! The fake engine in the real app: for scripted runs and screenshots.
//!
//! A scripted run of the binary (`INNER_WORD_AUTOMATION`) never gets Claude
//! Code: a script could send messages, and every message spends the user's
//! own allowance. It gets no assistant at all, unless [`FAKE_ENV`] asks
//! for the engine crate's fake backend, which runs no program and answers
//! from a script. That is how the assistant is looked at and captured
//! without spending anything:
//!
//! ```sh
//! INNER_WORD_DATA_DIR=/some/tmp INNER_WORD_SEED_DEMO=1 \
//! INNER_WORD_ASSISTANT_FAKE=ready \
//! INNER_WORD_ASSISTANT_FAKE_REPLIES=/some/tmp/replies.txt \
//! INNER_WORD_AUTOMATION="wait:1200; keys:cmd-3; type:What is missing?; keys:enter; wait:300; screenshot:/some/tmp/side.png; quit" \
//! ./target/debug/inner-word
//! ```
//!
//! # The replies file
//!
//! [`REPLIES_ENV`] names a text file with the answers, in the order the
//! messages are sent; a line of four or more equals signs (`====`) ends an
//! answer. Lines that begin an answer and start with `!` are directions:
//!
//! | Line | Effect |
//! |---|---|
//! | `!tool <name> <words>` | the assistant uses a tool before it answers: `!tool read_document Reading Chapter 2` |
//! | `!error <kind>` | the answer fails (after whatever text follows): `not-signed-in`, `usage-limit`, `network`, `exited`, `not-installed` |
//!
//! Everything else is the Markdown of the answer. Messages beyond the end
//! of the file are answered with [`FALLBACK_REPLY`].
//!
//! # While an answer is being written
//!
//! The automation steps `fake:hold`, `fake:advance:<events>` and
//! `fake:release` hold the fake's events back and let them through, so
//! that a capture can show an answer half written, with its Stop control.

use std::sync::Arc;

use gpui::{App, Global};
use innerword_assistant::{AssistantError, FakeBackend, FakeReply};

pub use super::host::FAKE_ENV;

/// Environment variable of a scripted run with the fake engine: the file
/// its answers are read from. See the [module docs](self).
pub const REPLIES_ENV: &str = "INNER_WORD_ASSISTANT_FAKE_REPLIES";

/// What the fake engine of a scripted run answers when its replies file
/// has nothing (more) to say.
pub const FALLBACK_REPLY: &str =
    "This is the fake engine of a scripted run. It answers every message with this sentence.";

struct FakeGlobal(FakeBackend);

impl Global for FakeGlobal {}

/// Installs `fake` as the engine of the assistant, and keeps it where the
/// automation steps find it ([`installed`]).
pub fn install(fake: FakeBackend, cx: &mut App) {
    super::host::set_backend(Arc::new(fake.clone()), cx);
    cx.set_global(FakeGlobal(fake));
}

/// The fake engine a scripted run was given, if it was.
pub fn installed(cx: &App) -> Option<FakeBackend> {
    cx.try_global::<FakeGlobal>().map(|global| global.0.clone())
}

/// The fake engine [`FAKE_ENV`] asks for, with the answers of
/// [`REPLIES_ENV`]; `None` when the variable is not set (or empty).
pub fn from_env() -> Option<FakeBackend> {
    let state = std::env::var(FAKE_ENV).ok()?;
    let state = state.trim();
    if state.is_empty() {
        return None;
    }
    let fake = match state {
        "missing" | "not-installed" => FakeBackend::not_installed(),
        "signed-out" => FakeBackend::signed_out(),
        _ => FakeBackend::new(),
    };
    fake.set_fallback(FakeReply::text(FALLBACK_REPLY));
    if let Some(path) = std::env::var_os(REPLIES_ENV).filter(|path| !path.is_empty()) {
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                for reply in parse_replies(&text) {
                    fake.push(reply);
                }
            }
            Err(error) => eprintln!(
                "Inner Word: cannot read {} ({}): {error}",
                REPLIES_ENV,
                std::path::Path::new(&path).display()
            ),
        }
    }
    Some(fake)
}

/// The answers of a replies file, in order. See the [module docs](self).
pub fn parse_replies(text: &str) -> Vec<FakeReply> {
    let mut replies = Vec::new();
    let mut lines: Vec<&str> = Vec::new();
    let mut flush = |lines: &mut Vec<&str>| {
        if lines.iter().any(|line| !line.trim().is_empty()) {
            replies.push(parse_reply(lines));
        }
        lines.clear();
    };
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.len() >= 4 && trimmed.chars().all(|ch| ch == '=') {
            flush(&mut lines);
        } else {
            lines.push(line);
        }
    }
    flush(&mut lines);
    replies
}

fn parse_reply(lines: &[&str]) -> FakeReply {
    let mut tools: Vec<(String, String)> = Vec::new();
    let mut error: Option<AssistantError> = None;
    let mut body_from = 0;
    for (ix, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() && tools.is_empty() && error.is_none() {
            body_from = ix + 1;
            continue;
        }
        let Some(direction) = trimmed.strip_prefix('!') else {
            break;
        };
        let (word, rest) = direction
            .split_once(char::is_whitespace)
            .unwrap_or((direction, ""));
        match word {
            "tool" => {
                let (name, words) = rest
                    .trim()
                    .split_once(char::is_whitespace)
                    .unwrap_or((rest.trim(), ""));
                tools.push((name.to_string(), words.trim().to_string()));
            }
            "error" => error = Some(error_named(rest.trim())),
            _ => break,
        }
        body_from = ix + 1;
    }
    let body = lines[body_from.min(lines.len())..].join("\n");
    let body = body.trim();
    let mut reply = match (&error, body.is_empty()) {
        (Some(error), true) => FakeReply::error(error.clone()),
        _ => FakeReply::text(body),
    };
    // Each call goes in front of the answer's text, behind the calls
    // before it.
    for (name, words) in tools {
        reply = reply.with_tool(name, words);
    }
    match (error, body.is_empty()) {
        (Some(error), false) => reply.then_fail(error),
        _ => reply,
    }
}

/// The error a replies file names.
fn error_named(kind: &str) -> AssistantError {
    match kind {
        "not-signed-in" | "signed-out" => AssistantError::NotSignedIn,
        "not-installed" | "missing" => AssistantError::NotInstalled,
        "usage-limit" | "limit" => AssistantError::UsageLimit {
            // Three hours from now, so that the message names a time.
            resets_at: Some(innerword_assistant::time::now_seconds() + 3 * 60 * 60),
            detail: String::new(),
        },
        "network" => AssistantError::Network {
            detail: String::new(),
        },
        "exited" | "crash" => AssistantError::Exited {
            code: Some(1),
            detail: String::new(),
        },
        other => AssistantError::Other {
            detail: other.to_string(),
        },
    }
}
