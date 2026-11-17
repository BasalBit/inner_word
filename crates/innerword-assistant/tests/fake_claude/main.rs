//! A stand-in for the `claude` program, for the process-level tests of this crate.
//!
//! It speaks the part of the program's `stream-json` protocol the assistant uses and
//! answers from a script the test wrote (`FAKE_CLAUDE_SCRIPT`, a JSON file): it replays
//! recordings of the real program (`tests/fixtures`), or makes up a turn in the same
//! shapes. It never talks to anyone. Everything it is started with and everything it is
//! sent is appended to the log the script names, one JSON object a line, so a test can
//! assert the flags, the environment and the exact messages.
//!
//! The script:
//!
//! ```json
//! {
//!   "log": "/tmp/x/log.jsonl",
//!   "version": "2.1.322 (Claude Code)",
//!   "auth": "signed-in",            // or "signed-out", "garbage", "hang"
//!   "initialize": "/path/initialize.jsonl",   // absent: the request is not known
//!   "select": "ok",                 // or "reject", "unsupported", "silent"
//!   "turns": [ ... ]                // one per user message; the last repeats
//! }
//! ```
//!
//! A turn is one of
//!
//! * `{"replay": "<recording>"}`: its lines, as they were (`"delay_ms"` between them);
//! * `{"replay": "<recording>", "hold_after_deltas": 10}`: its lines up to the tenth
//!   piece of text, then nothing until the turn is interrupted; then the rest of the
//!   recording, which is that of an interrupted turn (`"ignore_interrupt": true`:
//!   nothing, ever);
//! * `{"text": "...", "pieces": 40}`: a made-up answer in that many pieces
//!   (`"echo": true`: the answer is the message it was sent);
//! * `{"silent": true}`: no answer at all;
//! * (for the whole script) `"deaf_after_messages": 1`: after that many messages the
//!   stand-in stops reading its input for good;
//! * any of them with `"noise": true` (lines that are not JSON and lines of unknown
//!   kinds in between), `"die_after_lines": 4` with `"exit_code"` and `"stderr"` (the
//!   process ends in the middle), or `"no_partials": true` (a made-up answer without
//!   pieces, only the whole message).

use std::io::{BufRead, Write};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::Duration;

use serde_json::{Value, json};

struct Fake {
    script: Value,
    log: Option<std::fs::File>,
    input: Receiver<Option<String>>,
    /// Lines that arrived while a turn was printed and are not an interrupt.
    backlog: std::collections::VecDeque<String>,
    turn: usize,
    model: String,
    out: std::io::Stdout,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let script: Value = std::env::var("FAKE_CLAUDE_SCRIPT")
        .ok()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| json!({}));
    let mut log = script.get("log").and_then(Value::as_str).and_then(|path| {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok()
    });
    let env: serde_json::Map<String, Value> = std::env::vars()
        .filter(|(name, _)| {
            name.starts_with("CLAUDE")
                || name.starts_with("ENABLE_")
                || name.starts_with("ANTHROPIC")
                || name == "PATH"
        })
        .map(|(name, value)| (name, json!(value)))
        .collect();
    note(
        &mut log,
        json!({
            "argv": args,
            "env": env,
            "cwd": std::env::current_dir().ok().map(|dir| dir.to_string_lossy().into_owned()),
            "pid": std::process::id(),
        }),
    );

    if args.first().map(String::as_str) == Some("--version") {
        println!(
            "{}",
            script
                .get("version")
                .and_then(Value::as_str)
                .unwrap_or("2.1.322 (Claude Code)")
        );
        return;
    }
    if args.first().map(String::as_str) == Some("auth") {
        match script
            .get("auth")
            .and_then(Value::as_str)
            .unwrap_or("signed-in")
        {
            "signed-out" => {
                println!(
                    "{}",
                    json!({"loggedIn": false, "authMethod": "none", "apiProvider": "firstParty"})
                );
                std::process::exit(1);
            }
            "garbage" => {
                println!("error: unknown command 'auth'");
                std::process::exit(1);
            }
            "hang" => std::thread::sleep(Duration::from_secs(120)),
            _ => println!(
                "{}",
                json!({
                    "loggedIn": true, "authMethod": "claude.ai", "apiProvider": "firstParty",
                    "email": "someone@example.com", "orgName": "someone@example.com's Organization",
                    "subscriptionType": "max",
                })
            ),
        }
        return;
    }

    let (tx, input) = channel();
    let deaf_after = script.get("deaf_after_messages").and_then(Value::as_u64);
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut messages = 0u64;
        for line in stdin.lock().lines() {
            let Ok(line) = line else { break };
            if line.contains("\"type\":\"user\"") {
                messages += 1;
            }
            if tx.send(Some(line)).is_err() {
                return;
            }
            if deaf_after.is_some_and(|limit| messages >= limit) {
                // A program that has stopped reading: whatever is written to it now
                // fills the pipe and goes no further. (It does not outlive a test by
                // much if nobody ends it.)
                std::thread::sleep(Duration::from_secs(30));
                std::process::exit(0);
            }
        }
        let _ = tx.send(None);
    });
    let model = args
        .iter()
        .position(|arg| arg == "--model")
        .and_then(|at| args.get(at + 1))
        .cloned()
        .unwrap_or_else(|| "default".to_string());
    let mut fake = Fake {
        script,
        log,
        input,
        backlog: std::collections::VecDeque::new(),
        turn: 0,
        model,
        out: std::io::stdout(),
    };
    fake.run();
}

/// Appends one entry to the log, in one write: several stand-ins log to one file.
fn note(log: &mut Option<std::fs::File>, entry: Value) {
    if let Some(file) = log {
        let line = format!("{entry}\n");
        let _ = file.write_all(line.as_bytes());
        let _ = file.flush();
    }
}

impl Fake {
    fn print(&mut self, line: &str) {
        let mut out = self.out.lock();
        if writeln!(out, "{line}").is_err() || out.flush().is_err() {
            // Nobody reads any more.
            std::process::exit(0);
        }
    }

    fn run(&mut self) {
        loop {
            if let Some(line) = self.backlog.pop_front() {
                self.on_input(&line, None);
                continue;
            }
            match self.input.recv() {
                Ok(Some(line)) => self.on_input(&line, None),
                Ok(None) | Err(_) => break,
            }
        }
        note(&mut self.log, json!({"event": "input closed"}));
    }

    /// Handles a line of input. With `interrupted` (a turn is being printed) only an
    /// interrupt is taken, into the slot; anything else waits for the turn to end.
    fn on_input(&mut self, line: &str, interrupted: Option<&mut Option<String>>) {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            note(&mut self.log, json!({"garbage": line}));
            return;
        };
        if interrupted.is_some() && value["request"]["subtype"] != "interrupt" {
            self.backlog.push_back(line.to_string());
            return;
        }
        note(&mut self.log, json!({"stdin": value}));
        match value.get("type").and_then(Value::as_str) {
            Some("user") => {
                let text = value["message"]["content"][0]["text"]
                    .as_str()
                    .or_else(|| value["message"]["content"].as_str())
                    .unwrap_or("")
                    .to_string();
                self.answer(&text);
            }
            Some("control_request") => {
                let id = value["request_id"].as_str().unwrap_or("").to_string();
                let request = &value["request"];
                match request["subtype"].as_str().unwrap_or("") {
                    "interrupt" => {
                        if let Some(slot) = interrupted {
                            *slot = Some(id);
                        } else {
                            self.respond(&id, json!({"still_queued": []}));
                        }
                    }
                    "initialize" => {
                        let recorded = self
                            .script
                            .get("initialize")
                            .and_then(Value::as_str)
                            .and_then(|path| std::fs::read_to_string(path).ok())
                            .and_then(|text| {
                                serde_json::from_str::<Value>(text.lines().next()?).ok()
                            });
                        match recorded {
                            Some(recorded) => {
                                let response = recorded["response"]["response"].clone();
                                self.respond(&id, response);
                            }
                            None => {
                                self.refuse(&id, "Unsupported control request subtype: initialize")
                            }
                        }
                    }
                    subtype @ ("set_model" | "apply_flag_settings") => {
                        match self
                            .script
                            .get("select")
                            .and_then(Value::as_str)
                            .unwrap_or("ok")
                        {
                            "silent" => {}
                            "unsupported" => self.refuse(
                                &id,
                                &format!("Unsupported control request subtype: {subtype}"),
                            ),
                            "reject" if subtype == "set_model" => self
                                .refuse(&id, "Unable to validate model: the model was not found"),
                            _ => {
                                if subtype == "set_model" {
                                    self.model =
                                        request["model"].as_str().unwrap_or("default").to_string();
                                    let model = format!("claude-{}", self.model);
                                    self.respond(&id, json!({"model": model}));
                                } else {
                                    self.respond(&id, Value::Null);
                                }
                            }
                        }
                    }
                    other => self.refuse(
                        &id,
                        &format!("Unsupported control request subtype: {other}"),
                    ),
                }
            }
            _ => {}
        }
    }

    fn respond(&mut self, id: &str, response: Value) {
        let mut inner = json!({"subtype": "success", "request_id": id});
        if !response.is_null() {
            inner["response"] = response;
        }
        self.print(&json!({"type": "control_response", "response": inner}).to_string());
    }

    fn refuse(&mut self, id: &str, error: &str) {
        self.print(
            &json!({"type": "control_response", "response": {"subtype": "error", "request_id": id, "error": error}})
                .to_string(),
        );
    }

    /// Answers a user message with the next turn of the script.
    fn answer(&mut self, message: &str) {
        let turns = self
            .script
            .get("turns")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let spec = turns
            .get(self.turn)
            .or_else(|| turns.last())
            .cloned()
            .unwrap_or_else(|| json!({"text": "A made-up answer.", "pieces": 3}));
        self.turn += 1;
        if spec.get("silent").and_then(Value::as_bool) == Some(true) {
            return;
        }
        let recorded: Vec<String> = match spec.get("replay").and_then(Value::as_str) {
            Some(path) => std::fs::read_to_string(path)
                .unwrap_or_default()
                .lines()
                .map(str::to_string)
                .collect(),
            None => {
                let text = if spec.get("echo").and_then(Value::as_bool) == Some(true) {
                    message.to_string()
                } else {
                    spec.get("text")
                        .and_then(Value::as_str)
                        .unwrap_or("A made-up answer.")
                        .to_string()
                };
                let pieces = spec.get("pieces").and_then(Value::as_u64).unwrap_or(4) as usize;
                let partials = spec.get("no_partials").and_then(Value::as_bool) != Some(true);
                made_up_turn(&text, pieces.max(1), &self.model, partials)
            }
        };
        let delay =
            Duration::from_millis(spec.get("delay_ms").and_then(Value::as_u64).unwrap_or(0));
        let hold = spec.get("hold_after_deltas").and_then(Value::as_u64);
        let ignore = spec.get("ignore_interrupt").and_then(Value::as_bool) == Some(true);
        let noise = spec.get("noise").and_then(Value::as_bool) == Some(true);
        let die_after = spec.get("die_after_lines").and_then(Value::as_u64);
        let mut deltas = 0u64;
        let mut printed = 0u64;
        let mut index = 0usize;
        while index < recorded.len() {
            let line = &recorded[index];
            let is_response = line.contains("\"type\":\"control_response\"");
            if hold.is_some_and(|hold| deltas >= hold) && is_response {
                // The recording goes on with an interrupt: wait for the real one.
                let Some(id) = self.wait_for_interrupt(ignore) else {
                    return;
                };
                self.respond(&id, json!({"still_queued": []}));
                index += 1;
                continue;
            }
            if is_response {
                index += 1;
                continue;
            }
            if die_after.is_some_and(|limit| printed >= limit) {
                if let Some(text) = spec.get("stderr").and_then(Value::as_str) {
                    eprintln!("{text}");
                }
                note(&mut self.log, json!({"event": "died"}));
                std::process::exit(
                    spec.get("exit_code").and_then(Value::as_i64).unwrap_or(1) as i32
                );
            }
            if noise {
                self.print("npm WARN something that is not JSON at all");
                self.print("{\"type\":\"a_kind_from_the_future\",\"payload\":{\"x\":[1,2,3]}}");
                self.print("{\"type\":\"system\",\"subtype\":\"a_new_subtype\"}");
                self.print("{ this is broken JSON");
                self.print("");
            }
            self.print(line);
            printed += 1;
            if line.contains("\"type\":\"text_delta\"") {
                deltas += 1;
            }
            if !delay.is_zero() {
                std::thread::sleep(delay);
            }
            index += 1;
            if index >= recorded.len() {
                break;
            }
            // An interrupt of a turn that is not held ends it where it stands.
            match self.input.try_recv() {
                Ok(Some(input)) => {
                    let mut interrupt = None;
                    self.on_input(&input, Some(&mut interrupt));
                    if let Some(id) = interrupt {
                        self.respond(&id, json!({"still_queued": []}));
                        self.print(&aborted_result());
                        return;
                    }
                }
                Ok(None) => {
                    note(&mut self.log, json!({"event": "input closed mid-turn"}));
                    std::process::exit(0);
                }
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => {}
            }
        }
        note(&mut self.log, json!({"event": "answered"}));
    }

    /// Blocks until the turn is interrupted. `None`: the input closed first.
    fn wait_for_interrupt(&mut self, ignore: bool) -> Option<String> {
        loop {
            match self.input.recv() {
                Ok(Some(input)) => {
                    let mut interrupt = None;
                    self.on_input(&input, Some(&mut interrupt));
                    if let Some(id) = interrupt
                        && !ignore
                    {
                        return Some(id);
                    }
                }
                Ok(None) | Err(_) => {
                    note(&mut self.log, json!({"event": "input closed mid-turn"}));
                    std::process::exit(0);
                }
            }
        }
    }
}

/// The `result` of a turn that was interrupted, in the shape Claude Code 2.1.322 gives.
fn aborted_result() -> String {
    json!({
        "type": "result", "subtype": "error_during_execution", "is_error": true,
        "terminal_reason": "aborted_streaming", "stop_reason": null, "num_turns": 2,
        "errors": ["[ede_diagnostic] result_type=user last_content_type=n/a stop_reason=null"],
        "duration_ms": 12, "total_cost_usd": 0, "usage": {"input_tokens": 0, "output_tokens": 0},
        "session_id": "00000000-0000-4000-8000-00000000fake",
    })
    .to_string()
}

/// The lines of an answer `text` in `pieces` pieces, in the shapes of a recorded turn.
fn made_up_turn(text: &str, pieces: usize, model: &str, partials: bool) -> Vec<String> {
    let session = "00000000-0000-4000-8000-00000000fake";
    let model = format!("claude-{model}");
    let mut lines = vec![
        json!({
            "type": "system", "subtype": "init", "session_id": session, "tools": [], "mcp_servers": [],
            "model": model, "permissionMode": "dontAsk", "slash_commands": [], "skills": [],
            "claude_code_version": "2.1.322",
        })
        .to_string(),
        json!({"type": "system", "subtype": "status", "status": "requesting", "session_id": session}).to_string(),
    ];
    let event = |event: Value| {
        json!({"type": "stream_event", "event": event, "session_id": session, "parent_tool_use_id": null})
            .to_string()
    };
    if partials {
        lines.push(event(json!({"type": "message_start", "message": {"id": "msg_fake", "role": "assistant", "model": model, "content": []}})));
        lines.push(event(json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}})));
        let chars: Vec<char> = text.chars().collect();
        let size = chars.len().div_ceil(pieces).max(1);
        for piece in chars.chunks(size) {
            let piece: String = piece.iter().collect();
            lines.push(event(json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": piece}})));
        }
    }
    lines.push(
        json!({
            "type": "assistant", "session_id": session, "parent_tool_use_id": null,
            "message": {"id": "msg_fake", "role": "assistant", "model": model, "type": "message",
                        "content": [{"type": "text", "text": text}], "stop_reason": null},
        })
        .to_string(),
    );
    if partials {
        lines.push(event(json!({"type": "content_block_stop", "index": 0})));
        lines.push(event(json!({"type": "message_delta", "delta": {"stop_reason": "end_turn", "stop_sequence": null}})));
        lines.push(event(json!({"type": "message_stop"})));
    }
    lines.push(
        json!({
            "type": "result", "subtype": "success", "is_error": false, "result": text,
            "stop_reason": "end_turn", "terminal_reason": "completed", "num_turns": 1,
            "duration_ms": 21, "total_cost_usd": 0.001,
            "usage": {"input_tokens": 3, "cache_creation_input_tokens": 0, "cache_read_input_tokens": 40, "output_tokens": 7},
            "session_id": session,
        })
        .to_string(),
    );
    lines
}
