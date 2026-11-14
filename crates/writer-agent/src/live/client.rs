//! The tool's end of the channel: sending one request to the app and reading its answer.

use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::Value;
use writer_store::instance_lock;

use super::socket::{line_of, read_line, write_line, write_text};
use super::{ANSWER_GRACE, Answer, AppState, Operation, PROTOCOL_VERSION, RequestLine};
use crate::error::{Error, Refusal};

/// What became of a request that was sent to the app.
pub(crate) enum Sent {
    /// The app carried it out; this is the result.
    Done(Value),
    /// The app carried it out and it failed, as it would have failed for the tool.
    Failed(Error),
    /// The app did not carry it out, or it is not known whether it did.
    Refused(Refusal),
    /// Nobody listens there: the app is gone, or on its way. Nothing was sent that
    /// could be carried out.
    Gone,
}

/// How long the tool listens for a word from an app that hung up on its request.
const GONE_WORD: Duration = Duration::from_millis(200);

/// The user this process runs as.
fn user_id() -> u32 {
    // SAFETY: `getuid` has no arguments and cannot fail.
    unsafe { libc::getuid() }
}

/// Sends `operation` to the app that listens at `socket` and waits for its answer: up to
/// `patience` for the app to take the request up (the app itself says so when that time
/// is over), and a little longer for the answer of one that did.
pub(crate) fn change(socket: &Path, operation: &Operation, patience: Duration) -> Sent {
    let started = Instant::now();
    let waited = |started: Instant| started.elapsed().as_secs().max(1);
    // A socket of another user is not the app's.
    if !std::fs::metadata(socket).is_ok_and(|found| found.uid() == user_id()) {
        return Sent::Gone;
    }
    // The whole line is written before the connection is made: the listener gives a
    // connection up that sends nothing for a second, and the line of a page of 64 MB
    // takes longer than that to write (as base64, in a build without optimisations).
    let line = RequestLine {
        version: PROTOCOL_VERSION,
        wait_ms: patience.as_millis().try_into().unwrap_or(u64::MAX),
        operation,
    };
    let line = match line_of(&line) {
        Ok(line) => line,
        Err(error) => {
            // Nothing was sent.
            return Sent::Failed(Error::failed(format!(
                "the change cannot be written for The Writer ({error}); nothing was changed"
            )));
        }
    };
    let Ok(mut stream) = UnixStream::connect(socket) else {
        return Sent::Gone;
    };
    // The app says so itself when it could not take the request up in time. One that did
    // take it up is given a little longer for its answer (never more than the patience
    // itself, so that a caller in a hurry is not kept).
    let wait = patience + ANSWER_GRACE.min(patience);
    if write_text(&mut stream, &line, patience.max(Duration::from_secs(1))).is_err() {
        // The app closed its end before it had the whole request. It has carried
        // nothing out; it may have said why (it is quitting).
        let said = read_line(&mut stream, GONE_WORD, Instant::now() + GONE_WORD);
        return match said.ok().flatten().as_deref().and_then(read_answer) {
            Some(Answer::Refused { refusal }) => Sent::Refused(refusal),
            _ => Sent::Gone,
        };
    }
    let deadline = Instant::now() + wait;
    let line = match read_line(&mut stream, wait, deadline) {
        Ok(Some(line)) => line,
        Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {
            return Sent::Refused(Refusal::Unanswered {
                seconds: waited(started),
            });
        }
        // The connection ended without an answer: the app went down with the request.
        Ok(None) | Err(_) => return Sent::Refused(Refusal::AnswerLost),
    };
    match read_answer(&line) {
        Some(Answer::Done { result }) => Sent::Done(result),
        Some(Answer::Failed { error }) => Sent::Failed(error.into_error()),
        Some(Answer::Refused {
            refusal: Refusal::TooLate,
        }) => Sent::Refused(Refusal::NoAnswer {
            seconds: waited(started),
        }),
        Some(Answer::Refused { refusal }) => Sent::Refused(refusal),
        None => Sent::Refused(Refusal::AnswerLost),
    }
}

/// An answer, as far as this version reads it. What every version reads of every other
/// is whether the app refused, and why.
fn read_answer(line: &str) -> Option<Answer> {
    let value: Value = serde_json::from_str(line).ok()?;
    if let Ok(answer) = serde_json::from_value::<Answer>(value.clone()) {
        return Some(answer);
    }
    match value.get("answer").and_then(Value::as_str) {
        Some("refused") => Some(Answer::refused(
            value
                .get("refusal")
                .cloned()
                .and_then(|refusal| serde_json::from_value(refusal).ok())
                .unwrap_or(Refusal::Unknown),
        )),
        _ => None,
    }
}

/// Asks the app that listens at `socket` to save what is not saved yet, and waits for it
/// up to `patience`. `None` when it did not answer in time (or not at all): the read
/// goes ahead on what is saved.
pub(crate) fn save(socket: &Path, patience: Duration) -> Option<AppState> {
    if !std::fs::metadata(socket).is_ok_and(|found| found.uid() == user_id()) {
        return None;
    }
    let mut stream = UnixStream::connect(socket).ok()?;
    let line = RequestLine {
        version: PROTOCOL_VERSION,
        wait_ms: patience.as_millis().try_into().unwrap_or(u64::MAX),
        operation: &Operation::Flush,
    };
    let deadline = Instant::now() + patience;
    write_line(&mut stream, &line, patience).ok()?;
    let line = read_line(&mut stream, patience, deadline).ok()??;
    match read_answer(&line)? {
        Answer::Done { result } => serde_json::from_value(result).ok(),
        _ => None,
    }
}

/// Waits up to `patience` for something else than an app that names `socket` and does not
/// listen there: for that app to be gone, or for the lock file of `data_dir` to name
/// another channel (an app that has just started, where a file left by a crash named the
/// old one for an instant). Says whether there is something new to look at.
pub(crate) fn wait_for_another(data_dir: &Path, socket: &Path, patience: Duration) -> bool {
    let started = Instant::now();
    loop {
        match instance_lock::app_channel(data_dir) {
            Ok(Some(channel)) if channel.path == socket => {}
            _ => return true,
        }
        if started.elapsed() >= patience {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Waits up to `patience` for the app that has `data_dir` open to be gone. Says whether
/// it is.
pub(crate) fn wait_for_quit(data_dir: &Path, patience: Duration) -> bool {
    let started = Instant::now();
    loop {
        if !instance_lock::app_is_running(data_dir).unwrap_or(true) {
            return true;
        }
        if started.elapsed() >= patience {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
