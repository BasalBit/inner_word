//! One conversation: a thread that owns one `claude` process and speaks its
//! `stream-json` protocol.
//!
//! The thread takes the commands of the [`Conversation`](crate::Conversation) and the
//! lines of the process from one queue, so nothing here needs a lock and nothing waits
//! on the main thread. The process is started with the first message (or with
//! `prepare`), lives as long as the conversation, and ends when its input is closed.
//!
//! The rules it keeps:
//!
//! * One message at a time is given to the program. A message sent while another is
//!   being answered waits here, so the answers come one by one in the order of the
//!   questions (the program would merge messages that arrive mid-answer).
//! * Stop interrupts the answer through the program's own `interrupt` request: the turn
//!   finishes as stopped at once, and the next message goes out when the program has
//!   confirmed. A program that does not confirm within [`STOP_PATIENCE`] is ended.
//! * Every turn is finished exactly once: by its `result`, by Stop, by Close, or by the
//!   end of the process, which fails the open turn and everything queued and marks the
//!   conversation dead. The session starts a new one with the history.
//! * Lines that are not JSON, lines of unknown kinds and over-long lines are skipped.

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::args::{bare_tool, base_command, conversation_args};
use super::run::{end_group, wait_until};
use super::stream::{
    Line, ResultLine, ToolUse, classify, control_refusal, control_request, parse_line, usage_of,
    user_message,
};
use crate::backend::{
    AssistantError, AssistantEvent, Command, ConversationOptions, EffortChoice, ModelChoice,
    Selection, ToolActivity, ToolState, TurnEnd, TurnId, one_line,
};
use crate::context::{HISTORY_LIMIT, bounded_history, with_history};
use crate::events::EventSink;
use crate::time::{describe_moment, now_seconds};
use crate::tools::describe_unknown_tool;

/// How long the program gets to confirm an interrupt before it is ended.
pub const STOP_PATIENCE: Duration = Duration::from_secs(5);
/// How long the program gets to answer a change of model or effort.
pub const SELECT_PATIENCE: Duration = Duration::from_secs(20);
/// How long the program gets to end by itself once its input is closed.
pub const CLOSE_PATIENCE: Duration = Duration::from_millis(1500);
/// The longest line of output that is read. A longer one is skipped: the final `result`
/// of an enormous answer repeats text that already arrived in pieces.
pub const MAX_LINE: usize = 32 * 1024 * 1024;

/// What the thread is told.
pub(super) enum Input {
    /// A command of the conversation.
    Command(Command),
    /// A line the process of this generation printed.
    Line(u64, String),
    /// The process of this generation closed its output.
    Eof(u64),
    /// The process of this generation does not take input any more.
    Deaf(u64),
}

/// Where the program is and whether it can be used: asked when a process is to be
/// started. The answer may come from a cache; `fresh` asks again.
pub(super) trait Launcher: Send + 'static {
    /// The program to run, or why it cannot be.
    fn program(&self, fresh: bool) -> Result<PathBuf, AssistantError>;
}

/// A running process.
struct Proc {
    child: Child,
    /// The lines for the process's input. They are written by a thread of their own:
    /// a process that has stopped reading must not stop this thread, which still has
    /// to hear Stop and Close. Dropping the sender closes the process's input.
    stdin: Option<Sender<String>>,
    generation: u64,
    stderr: Arc<Mutex<String>>,
    /// The total cost the program last reported.
    cost: f64,
}

/// A tool call of the open turn.
struct Call {
    tool: String,
    input: Value,
    description: String,
}

/// The turn that is being answered.
struct Turn {
    id: TurnId,
    text: String,
    /// Whether text of the model's current message arrived in pieces.
    streamed: bool,
    /// Whether a status line is showing.
    status: bool,
    /// The program's report of a failed request: its category and its sentence.
    error: Option<(Option<String>, String)>,
    resets_at: Option<i64>,
    /// Whether the program reported a try that got no answer from the service.
    no_answer: bool,
    calls: HashMap<String, Call>,
}

impl Turn {
    fn new(id: TurnId) -> Self {
        Turn {
            id,
            text: String::new(),
            streamed: false,
            status: false,
            error: None,
            resets_at: None,
            no_answer: false,
            calls: HashMap::new(),
        }
    }
}

/// A request to the program that is waiting for its answer.
enum Pending {
    Interrupt,
    Model(ModelChoice),
    Effort(EffortChoice),
}

pub(super) struct Worker<L: Launcher> {
    launcher: L,
    options: ConversationOptions,
    sink: EventSink,
    alive: Arc<AtomicBool>,
    inbox: Receiver<Input>,
    inbox_tx: Sender<Input>,
    proc: Option<Proc>,
    generation: u64,
    /// Whether a message was given to a process yet (the first carries the history).
    spoken: bool,
    queue: VecDeque<(TurnId, String)>,
    current: Option<Turn>,
    /// An interrupted turn whose `result` is awaited, and until when.
    draining: Option<Instant>,
    /// The selection the user wants, and the one the running process has.
    wanted: Selection,
    actual: Selection,
    /// Until when the answers to a change of selection are awaited.
    selecting: Option<Instant>,
    /// Why the program refused the wanted selection.
    rejection: Option<String>,
    pending: HashMap<String, Pending>,
    request_seq: u64,
    model: Option<String>,
    warned: bool,
    tools_warned: bool,
    /// The process is gone for good: the conversation takes no more messages.
    dead: bool,
    last_noise: String,
}

impl<L: Launcher> Worker<L> {
    pub(super) fn new(
        launcher: L,
        options: ConversationOptions,
        sink: EventSink,
        alive: Arc<AtomicBool>,
        inbox: Receiver<Input>,
        inbox_tx: Sender<Input>,
    ) -> Self {
        let selection = options.selection.clone();
        Worker {
            launcher,
            options,
            sink,
            alive,
            inbox,
            inbox_tx,
            proc: None,
            generation: 0,
            spoken: false,
            queue: VecDeque::new(),
            current: None,
            draining: None,
            wanted: selection.clone(),
            actual: selection,
            selecting: None,
            rejection: None,
            pending: HashMap::new(),
            request_seq: 0,
            model: None,
            warned: false,
            tools_warned: false,
            dead: false,
            last_noise: String::new(),
        }
    }

    /// Runs until the conversation is closed.
    pub(super) fn run(mut self) {
        loop {
            let deadline = match (self.draining, self.selecting) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
            let input = match deadline {
                None => match self.inbox.recv() {
                    Ok(input) => input,
                    Err(_) => break,
                },
                Some(deadline) => {
                    let wait = deadline.saturating_duration_since(Instant::now());
                    match self.inbox.recv_timeout(wait) {
                        Ok(input) => input,
                        Err(RecvTimeoutError::Timeout) => {
                            self.out_of_patience();
                            continue;
                        }
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
            };
            match input {
                Input::Command(Command::Close) => break,
                Input::Command(Command::Prepare) => {
                    if !self.dead && self.proc.is_none() {
                        // A failure shows with the first message.
                        let _ = self.start_process();
                    }
                }
                Input::Command(Command::Send { turn, text }) => {
                    self.queue.push_back((turn, text));
                    self.pump();
                }
                Input::Command(Command::Select(selection)) => {
                    self.wanted = selection;
                    self.rejection = None;
                    self.pump();
                }
                Input::Command(Command::Stop) => self.stop(),
                Input::Line(generation, line) => {
                    if self
                        .proc
                        .as_ref()
                        .is_some_and(|proc| proc.generation == generation)
                    {
                        self.on_line(&line);
                    }
                }
                Input::Eof(generation) | Input::Deaf(generation) => {
                    if self
                        .proc
                        .as_ref()
                        .is_some_and(|proc| proc.generation == generation)
                    {
                        self.on_exit();
                    }
                }
            }
        }
        self.close();
        // A message that came in while the process was being ended is still answered.
        let late = AssistantError::Exited {
            code: None,
            detail: "the conversation has ended".to_string(),
        };
        while let Ok(input) = self.inbox.try_recv() {
            if let Input::Command(Command::Send { turn, .. }) = input {
                self.finish(turn, None, TurnEnd::Failed(late.clone()), None);
            }
        }
    }

    fn emit(&self, event: AssistantEvent) {
        self.sink.send(event);
    }

    fn finish(
        &self,
        turn: TurnId,
        text: Option<String>,
        end: TurnEnd,
        usage: Option<crate::Usage>,
    ) {
        self.emit(AssistantEvent::Finished {
            turn,
            text,
            end,
            usage,
        });
    }

    /// Fails every waiting message with `error`.
    fn fail_queue(&mut self, error: &AssistantError) {
        for (turn, _) in std::mem::take(&mut self.queue) {
            self.finish(turn, None, TurnEnd::Failed(error.clone()), None);
        }
    }

    // ----- the process -----------------------------------------------------------------

    /// Starts the process with the wanted selection.
    fn start_process(&mut self) -> Result<(), AssistantError> {
        let program = match self.launcher.program(false) {
            Ok(program) => program,
            // The user may have installed it or signed in since the last look.
            Err(_) => self.launcher.program(true)?,
        };
        let args = conversation_args(
            &self.options.system_prompt,
            &self.wanted,
            self.options.tools.as_ref(),
        );
        let mut command = base_command(&program, self.options.working_dir.as_deref());
        command
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0);
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // It was there when it was looked for: it is not any more.
                let _ = self.launcher.program(true);
                return Err(AssistantError::NotInstalled);
            }
            Err(error) => {
                return Err(AssistantError::Exited {
                    code: None,
                    detail: format!("it could not be started: {error}"),
                });
            }
        };
        self.generation += 1;
        let generation = self.generation;
        let stdin = child.stdin.take().map(|pipe| {
            let (lines, queue) = std::sync::mpsc::channel::<String>();
            let inbox = self.inbox_tx.clone();
            let _ = std::thread::Builder::new()
                .name("innerword-assistant-claude-in".into())
                .spawn(move || write_lines(pipe, queue, generation, inbox));
            lines
        });
        if let Some(stdout) = child.stdout.take() {
            let inbox = self.inbox_tx.clone();
            let _ = std::thread::Builder::new()
                .name("innerword-assistant-claude-out".into())
                .spawn(move || read_lines(stdout, generation, inbox));
        }
        let stderr = Arc::new(Mutex::new(String::new()));
        if let Some(pipe) = child.stderr.take() {
            let tail = stderr.clone();
            let _ = std::thread::Builder::new()
                .name("innerword-assistant-claude-err".into())
                .spawn(move || keep_tail(pipe, tail));
        }
        self.proc = Some(Proc {
            child,
            stdin,
            generation,
            stderr,
            cost: 0.0,
        });
        self.actual = self.wanted.clone();
        self.rejection = None;
        self.pending.clear();
        self.selecting = None;
        Ok(())
    }

    /// Writes a line to the process. `false` when it does not take it any more.
    fn write(&mut self, line: &str) -> bool {
        let Some(stdin) = self.proc.as_ref().and_then(|proc| proc.stdin.as_ref()) else {
            return false;
        };
        stdin.send(line.to_string()).is_ok()
    }

    fn request(&mut self, subtype: &str, fields: Value, pending: Pending) -> bool {
        self.request_seq += 1;
        let id = format!("writer-{}-{}", self.generation, self.request_seq);
        if self.write(&control_request(&id, subtype, fields)) {
            self.pending.insert(id, pending);
            true
        } else {
            false
        }
    }

    // ----- sending -----------------------------------------------------------------------

    /// Gives the program the next waiting message, when it is free to take one.
    fn pump(&mut self) {
        loop {
            if self.current.is_some() || self.draining.is_some() || self.selecting.is_some() {
                return;
            }
            if self.dead {
                let error = AssistantError::Exited {
                    code: None,
                    detail: "the conversation has ended".to_string(),
                };
                self.fail_queue(&error);
                return;
            }
            if self.proc.is_some() && self.wanted != self.actual && self.rejection.is_none() {
                self.change_selection();
                if self.selecting.is_some() || self.dead {
                    continue;
                }
            }
            if self.queue.is_empty() {
                return;
            }
            if self.proc.is_none()
                && let Err(error) = self.start_process()
            {
                self.fail_queue(&error);
                return;
            }
            if let Some(detail) = self.rejection.clone() {
                self.fail_queue(&AssistantError::Rejected { detail });
                return;
            }
            let Some((turn, text)) = self.queue.pop_front() else {
                return;
            };
            let text = if self.spoken {
                text
            } else {
                with_history(
                    &bounded_history(&self.options.history, HISTORY_LIMIT),
                    &text,
                )
            };
            self.emit(AssistantEvent::Started { turn });
            if self.write(&user_message(&text)) {
                self.spoken = true;
                self.current = Some(Turn::new(turn));
                return;
            }
            // The process is gone before it took the message: the end of its output
            // says why, and fails this turn with everything queued.
            self.current = Some(Turn::new(turn));
            self.on_exit();
        }
    }

    /// Asks the running program to answer with the wanted model and effort from now on.
    fn change_selection(&mut self) {
        let mut asked = false;
        if self.wanted.model != self.actual.model {
            let model = self.wanted.model.id().unwrap_or("default").to_string();
            asked |= self.request(
                "set_model",
                json!({ "model": model }),
                Pending::Model(self.wanted.model.clone()),
            );
        }
        if self.wanted.effort != self.actual.effort {
            let level = match self.wanted.effort.id() {
                Some(level) => json!(level),
                None => Value::Null,
            };
            asked |= self.request(
                "apply_flag_settings",
                json!({ "settings": { "effortLevel": level } }),
                Pending::Effort(self.wanted.effort.clone()),
            );
        }
        if asked {
            self.selecting = Some(Instant::now() + SELECT_PATIENCE);
        } else {
            // The process does not take input any more.
            self.on_exit();
        }
    }

    fn selection_settled(&mut self) {
        let waiting = self
            .pending
            .values()
            .any(|pending| matches!(pending, Pending::Model(_) | Pending::Effort(_)));
        if !waiting {
            self.selecting = None;
            self.pump();
        }
    }

    // ----- stopping ----------------------------------------------------------------------

    fn stop(&mut self) {
        for (turn, _) in std::mem::take(&mut self.queue) {
            self.finish(turn, None, TurnEnd::Stopped, None);
        }
        let Some(turn) = self.current.take() else {
            return;
        };
        self.finish(turn.id, None, TurnEnd::Stopped, None);
        if self.proc.is_none() {
            return;
        }
        if self.request("interrupt", Value::Null, Pending::Interrupt) {
            self.draining = Some(Instant::now() + STOP_PATIENCE);
        } else {
            self.on_exit();
        }
    }

    /// A deadline passed: the program did not confirm an interrupt, or did not answer a
    /// change of selection. Either way it is not to be trusted with the next message.
    fn out_of_patience(&mut self) {
        let now = Instant::now();
        let stuck = self.draining.is_some_and(|deadline| deadline <= now)
            || self.selecting.is_some_and(|deadline| deadline <= now);
        if stuck {
            self.draining = None;
            self.selecting = None;
            self.end_process("it did not answer in time");
        }
    }

    /// Ends the process and the conversation with it: what is open fails.
    fn end_process(&mut self, why: &str) {
        // Said first: whoever asks from now on starts a new conversation instead of
        // sending a message to one that is going.
        self.alive.store(false, Ordering::SeqCst);
        let code = self.proc.take().and_then(|mut proc| {
            // It ends by itself when its input closes, if it is still listening.
            drop(proc.stdin.take());
            match wait_until(&mut proc.child, Instant::now() + Duration::from_millis(400)) {
                Some(code) => {
                    super::run::signal_group(&proc.child, libc::SIGKILL);
                    code
                }
                None => end_group(&mut proc.child, Duration::from_millis(300)),
            }
        });
        self.died(code, why.to_string());
    }

    /// The process closed its output: it has ended, or is about to.
    fn on_exit(&mut self) {
        let Some(mut proc) = self.proc.take() else {
            return;
        };
        drop(proc.stdin.take());
        let code = match wait_until(&mut proc.child, Instant::now() + CLOSE_PATIENCE) {
            Some(code) => code,
            None => end_group(&mut proc.child, Duration::from_millis(300)),
        };
        // What it started (the tools' server) goes with it.
        super::run::signal_group(&proc.child, libc::SIGKILL);
        // The reader of its errors needs a moment to have the last of them.
        std::thread::sleep(Duration::from_millis(20));
        let said = proc
            .stderr
            .lock()
            .map(|tail| tail.trim().to_string())
            .unwrap_or_default();
        let detail = if said.is_empty() {
            self.last_noise.clone()
        } else {
            said
        };
        self.died(code, one_line(&detail, 400));
    }

    fn died(&mut self, code: Option<i32>, detail: String) {
        self.dead = true;
        self.alive.store(false, Ordering::SeqCst);
        self.draining = None;
        self.selecting = None;
        self.pending.clear();
        let lower = detail.to_lowercase();
        let error = if lower.contains("not logged in") || lower.contains("/login") {
            AssistantError::NotSignedIn
        } else {
            AssistantError::Exited { code, detail }
        };
        if let Some(turn) = self.current.take() {
            self.finish(turn.id, None, TurnEnd::Failed(error.clone()), None);
        }
        self.fail_queue(&error);
    }

    /// The conversation is closed: what is open is stopped, the process is ended.
    fn close(&mut self) {
        self.dead = true;
        self.alive.store(false, Ordering::SeqCst);
        let mut busy = self.draining.is_some();
        if let Some(turn) = self.current.take() {
            busy = true;
            self.finish(turn.id, None, TurnEnd::Stopped, None);
        }
        for (turn, _) in std::mem::take(&mut self.queue) {
            self.finish(turn, None, TurnEnd::Stopped, None);
        }
        if let Some(mut proc) = self.proc.take() {
            // The program ends by itself when its input closes; a turn in flight is
            // not waited for.
            drop(proc.stdin.take());
            let patience = if busy {
                Duration::from_millis(100)
            } else {
                CLOSE_PATIENCE
            };
            if wait_until(&mut proc.child, Instant::now() + patience).is_none() {
                end_group(&mut proc.child, Duration::from_millis(300));
            }
            super::run::signal_group(&proc.child, libc::SIGKILL);
        }
    }

    // ----- the program's output ------------------------------------------------------------

    fn on_line(&mut self, line: &str) {
        match parse_line(line) {
            Line::NotJson => {
                let line = line.trim();
                if !line.is_empty() {
                    self.last_noise = one_line(line, 400);
                }
            }
            Line::Other => {}
            Line::Init {
                model,
                failed_servers,
                ..
            } => {
                if model.is_some() && model != self.model {
                    self.model = model.clone();
                    self.emit(AssistantEvent::Ready { model });
                }
                if !failed_servers.is_empty() && !self.tools_warned {
                    self.tools_warned = true;
                    self.emit(AssistantEvent::Notice {
                        text: "The assistant's tools did not start: in this conversation it \
                               cannot look anything up and works from what it is sent."
                            .to_string(),
                    });
                }
            }
            Line::Retry {
                attempt,
                max,
                status,
                ..
            } => {
                if let Some(turn) = &mut self.current {
                    turn.no_answer |= status.is_none();
                    turn.status = true;
                    let what = match status {
                        None => "No connection",
                        Some(401) | Some(403) => "Signing in",
                        Some(_) => "The service is busy",
                    };
                    let text = if max > 0 && attempt > 0 {
                        format!("{what}, trying again ({attempt} of {max})")
                    } else {
                        format!("{what}, trying again")
                    };
                    let turn = turn.id;
                    self.emit(AssistantEvent::Status { turn, text });
                }
            }
            Line::Compacting => self.status("Making room in a long conversation"),
            Line::ThinkingStart => self.status("Thinking"),
            Line::MessageStart => {
                if let Some(turn) = &mut self.current {
                    turn.streamed = false;
                    if turn.status {
                        turn.status = false;
                        let turn = turn.id;
                        self.emit(AssistantEvent::Status {
                            turn,
                            text: String::new(),
                        });
                    }
                }
            }
            Line::TextStart => {
                // The model's words before and after a tool call are separate blocks.
                if let Some(turn) = &mut self.current
                    && !turn.text.is_empty()
                    && !turn.text.ends_with("\n\n")
                {
                    let gap = if turn.text.ends_with('\n') {
                        "\n"
                    } else {
                        "\n\n"
                    };
                    turn.text.push_str(gap);
                    let turn = turn.id;
                    self.emit(AssistantEvent::Text {
                        turn,
                        text: gap.to_string(),
                    });
                }
            }
            Line::TextDelta(text) => {
                if let Some(turn) = &mut self.current {
                    turn.streamed = true;
                    turn.status = false;
                    turn.text.push_str(&text);
                    let turn = turn.id;
                    self.emit(AssistantEvent::Text { turn, text });
                }
            }
            // The call is shown when its arguments are all there (the complete message
            // follows the pieces at once): they say what is being read.
            Line::ToolStart(_) => {}
            Line::Assistant {
                texts,
                tools,
                error,
                synthetic,
            } => {
                if error.is_some() || synthetic {
                    if let Some(turn) = &mut self.current {
                        turn.error = Some((error, texts.join(" ")));
                    }
                    return;
                }
                let streamed = self.current.as_ref().is_some_and(|turn| turn.streamed);
                if !streamed {
                    // No pieces came for this message (a program that does not stream):
                    // its text arrives whole.
                    for text in texts.into_iter().filter(|text| !text.is_empty()) {
                        if let Some(turn) = &mut self.current {
                            let mut piece = String::new();
                            if !turn.text.is_empty() && !turn.text.ends_with("\n\n") {
                                piece.push_str(if turn.text.ends_with('\n') {
                                    "\n"
                                } else {
                                    "\n\n"
                                });
                            }
                            piece.push_str(&text);
                            turn.text.push_str(&piece);
                            let turn = turn.id;
                            self.emit(AssistantEvent::Text { turn, text: piece });
                        }
                    }
                }
                for call in tools {
                    self.tool_called(call);
                }
            }
            Line::ToolResults(results) => {
                for result in results {
                    let Some(turn) = &mut self.current else {
                        return;
                    };
                    let Some(call) = turn.calls.get(&result.tool_use_id) else {
                        continue;
                    };
                    let activity = ToolActivity {
                        id: result.tool_use_id.clone(),
                        tool: call.tool.clone(),
                        description: call.description.clone(),
                        state: if result.is_error {
                            ToolState::Failed
                        } else {
                            ToolState::Done
                        },
                    };
                    if !result.is_error
                        && let Some(observe) = self
                            .options
                            .tools
                            .as_ref()
                            .and_then(|tools| tools.observe.as_ref())
                    {
                        observe(&call.tool, &call.input, &result.structured);
                    }
                    let turn = turn.id;
                    self.emit(AssistantEvent::Tool { turn, activity });
                }
            }
            Line::RateLimit {
                status,
                resets_at,
                kind,
            } => {
                if status == "rejected" {
                    if let Some(turn) = &mut self.current {
                        turn.resets_at = resets_at;
                    }
                } else if status.contains("warning") && !self.warned {
                    self.warned = true;
                    let window = match kind.as_deref() {
                        Some("five_hour") => "five-hour ",
                        Some("seven_day") => "weekly ",
                        _ => "",
                    };
                    let mut text = format!(
                        "You are close to the {window}usage limit of your Claude Code plan."
                    );
                    if let Some(at) = resets_at {
                        text.push_str(&format!(
                            " It resets {}.",
                            describe_moment(at, now_seconds())
                        ));
                    }
                    self.emit(AssistantEvent::Notice { text });
                }
            }
            Line::Result(result) => self.on_result(result),
            Line::ControlResponse {
                request_id,
                ok,
                error,
                ..
            } => match self.pending.remove(&request_id) {
                None | Some(Pending::Interrupt) => {}
                Some(Pending::Model(model)) => {
                    self.selection_answered(ok, error, |actual| actual.model = model);
                }
                Some(Pending::Effort(effort)) => {
                    self.selection_answered(ok, error, |actual| actual.effort = effort);
                }
            },
            Line::ControlRequest {
                request_id,
                subtype,
            } => {
                self.write(&control_refusal(&request_id, &subtype));
            }
        }
    }

    fn status(&mut self, text: &str) {
        if let Some(turn) = &mut self.current {
            turn.status = true;
            let turn = turn.id;
            self.emit(AssistantEvent::Status {
                turn,
                text: text.to_string(),
            });
        }
    }

    /// The program answered a change of model or effort.
    fn selection_answered(
        &mut self,
        ok: bool,
        error: Option<String>,
        apply: impl FnOnce(&mut Selection),
    ) {
        if ok {
            apply(&mut self.actual);
            self.selection_settled();
            return;
        }
        let error = error.unwrap_or_default();
        if error.to_lowercase().contains("unsupported control request") {
            // This version cannot change it while it runs: the conversation ends here
            // and the session starts a new one with the selection as flags.
            self.end_process("this version of Claude Code cannot change the model while it runs");
            return;
        }
        self.rejection = Some(if error.is_empty() {
            "Claude Code could not change the model or effort.".to_string()
        } else {
            one_line(&error, 300)
        });
        self.selection_settled();
    }

    /// The model calls a tool; its arguments are all there.
    fn tool_called(&mut self, call: ToolUse) {
        let Some(turn) = &mut self.current else {
            return;
        };
        let server = self.options.tools.as_ref();
        let tool = bare_tool(server.map(|server| server.name.as_str()), &call.name).to_string();
        let described = server
            .and_then(|server| server.describe.as_ref())
            .and_then(|describe| describe(&tool, &call.input))
            .unwrap_or_else(|| describe_unknown_tool(&tool));
        if turn.calls.contains_key(&call.id) {
            // The program repeats a message it already gave.
            return;
        }
        turn.status = false;
        turn.calls.insert(
            call.id.clone(),
            Call {
                tool: tool.clone(),
                input: call.input,
                description: described.clone(),
            },
        );
        let turn = turn.id;
        self.emit(AssistantEvent::Tool {
            turn,
            activity: ToolActivity {
                id: call.id,
                tool,
                description: described,
                state: ToolState::Running,
            },
        });
    }

    fn on_result(&mut self, result: ResultLine) {
        let cost_before = self.proc.as_ref().map(|proc| proc.cost).unwrap_or(0.0);
        if let (Some(proc), Some(total)) = (self.proc.as_mut(), result.total_cost_usd) {
            proc.cost = total;
        }
        if self.draining.take().is_some() {
            // The interrupted turn, which was finished when Stop was pressed.
            self.pump();
            return;
        }
        let Some(turn) = self.current.take() else {
            return;
        };
        let usage = usage_of(&result, cost_before);
        if result.was_aborted() {
            self.finish(turn.id, None, TurnEnd::Stopped, usage);
        } else if turn.error.is_some() || result.is_error {
            let (category, text) = match turn.error {
                Some((category, text)) if !text.trim().is_empty() => (category, text),
                Some((category, _)) => (category, result.text.clone().unwrap_or_default()),
                None => (
                    None,
                    result
                        .text
                        .clone()
                        .filter(|text| !text.trim().is_empty())
                        .unwrap_or_else(|| result.errors.join(" ")),
                ),
            };
            let error = classify(
                category.as_deref(),
                result.api_error_status,
                &text,
                turn.resets_at,
                turn.no_answer,
            );
            if error == AssistantError::NotSignedIn {
                // What was known about the sign-in is out of date: look again, so
                // that the app shows it.
                let _ = self.launcher.program(true);
            }
            self.finish(turn.id, None, TurnEnd::Failed(error), usage);
        } else {
            // The final text is the answer where nothing arrived in pieces. Where
            // pieces arrived they are the answer: with a tool call in the middle the
            // final text is only the last of the model's messages.
            let text = if turn.text.trim().is_empty() {
                result.text.filter(|text| !text.trim().is_empty())
            } else {
                None
            };
            self.finish(turn.id, text, TurnEnd::Complete, usage);
        }
        self.pump();
    }
}

/// Reads the lines of the process's output into the thread's queue, skipping a line
/// longer than [`MAX_LINE`].
fn read_lines(stdout: impl Read, generation: u64, inbox: Sender<Input>) {
    let mut reader = BufReader::with_capacity(64 * 1024, stdout);
    let mut line: Vec<u8> = Vec::new();
    let mut too_long = false;
    loop {
        let (used, complete) = {
            let buffer = match reader.fill_buf() {
                Ok(buffer) if !buffer.is_empty() => buffer,
                _ => break,
            };
            match buffer.iter().position(|byte| *byte == b'\n') {
                Some(at) => {
                    if !too_long {
                        line.extend_from_slice(&buffer[..at]);
                    }
                    (at + 1, true)
                }
                None => {
                    if !too_long {
                        line.extend_from_slice(buffer);
                    }
                    (buffer.len(), false)
                }
            }
        };
        reader.consume(used);
        if line.len() > MAX_LINE {
            too_long = true;
            line.clear();
        }
        if complete {
            if !too_long
                && !line.is_empty()
                && inbox
                    .send(Input::Line(
                        generation,
                        String::from_utf8_lossy(&line).into_owned(),
                    ))
                    .is_err()
            {
                return;
            }
            line.clear();
            too_long = false;
        }
    }
    if !too_long && !line.is_empty() {
        let _ = inbox.send(Input::Line(
            generation,
            String::from_utf8_lossy(&line).into_owned(),
        ));
    }
    let _ = inbox.send(Input::Eof(generation));
}

/// Writes the lines of the queue to the process's input, one a line, until the queue is
/// closed (which closes the input) or the process does not take them any more.
fn write_lines(
    mut pipe: ChildStdin,
    queue: Receiver<String>,
    generation: u64,
    inbox: Sender<Input>,
) {
    for line in queue {
        let written = pipe
            .write_all(line.as_bytes())
            .and_then(|_| pipe.write_all(b"\n"))
            .and_then(|_| pipe.flush());
        if written.is_err() {
            let _ = inbox.send(Input::Deaf(generation));
            return;
        }
    }
}

/// Keeps the last of what the process writes to its standard error.
fn keep_tail(pipe: impl Read, tail: Arc<Mutex<String>>) {
    const KEEP: usize = 4000;
    let reader = BufReader::new(pipe);
    for line in reader.lines() {
        let Ok(line) = line else {
            break;
        };
        if let Ok(mut tail) = tail.lock() {
            tail.push_str(&line);
            tail.push('\n');
            if tail.len() > KEEP * 2 {
                let cut = tail
                    .char_indices()
                    .map(|(at, _)| at)
                    .find(|at| tail.len() - at <= KEEP)
                    .unwrap_or(0);
                tail.drain(..cut);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    fn lines_of(bytes: &[u8]) -> Vec<String> {
        let (tx, rx) = channel();
        read_lines(bytes, 7, tx);
        let mut lines = Vec::new();
        let mut ended = false;
        for input in rx.try_iter() {
            match input {
                Input::Line(7, line) => lines.push(line),
                Input::Eof(7) => ended = true,
                _ => panic!("another generation"),
            }
        }
        assert!(ended, "the end of the output is reported");
        lines
    }

    #[test]
    fn lines_are_read_whole_whatever_their_length() {
        let long = "x".repeat(300_000);
        let text = format!("first\n\n{long}\nlast without a newline");
        assert_eq!(
            lines_of(text.as_bytes()),
            vec![
                "first".to_string(),
                long,
                "last without a newline".to_string()
            ]
        );
        assert!(lines_of(b"").is_empty());
    }

    #[test]
    fn bytes_that_are_not_text_do_not_stop_the_reading() {
        let lines = lines_of(b"\xff\xfe broken\n{\"type\":\"result\"}\n");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1], "{\"type\":\"result\"}");
    }

    #[test]
    fn the_tail_of_the_errors_is_kept_bounded() {
        let tail = Arc::new(Mutex::new(String::new()));
        let mut text = String::new();
        for line in 0..2000 {
            text.push_str(&format!("line {line} of the errors\n"));
        }
        keep_tail(text.as_bytes(), tail.clone());
        let kept = tail.lock().unwrap().clone();
        assert!(kept.len() <= 8100, "{}", kept.len());
        assert!(kept.trim_end().ends_with("line 1999 of the errors"));
    }
}
