//! What the process-level tests share: a temporary folder with a script for the
//! stand-in program, the log the stand-in keeps, and the collecting of events.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use innerword_assistant::{
    AssistantError, AssistantEvent, Backend, ClaudeCode, Conversation, ConversationOptions,
    EventStream, TurnEnd, TurnId, event_channel,
};
use serde_json::{Value, json};

/// How long a test waits for something the stand-in does at once.
pub const PATIENCE: Duration = Duration::from_secs(20);

/// A recording of the real program.
pub fn fixture(name: &str) -> String {
    format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))
}

/// A stand-in `claude` with its script and its log, in a folder of its own.
pub struct Stage {
    dir: tempfile::TempDir,
    /// The program to hand to [`ClaudeCode::with_program`].
    pub program: PathBuf,
}

impl Stage {
    /// A stand-in that follows `script` (see `tests/fake_claude/main.rs`). Each test has
    /// its own: the script reaches the stand-in through a wrapper, not through the
    /// environment of the test process, so tests run side by side.
    pub fn new(mut script: Value) -> Stage {
        // A space in the path, as "Application Support" has.
        let dir = tempfile::Builder::new()
            .prefix("innerword assistant ")
            .tempdir()
            .expect("a temporary folder");
        let log = dir.path().join("log.jsonl");
        script["log"] = json!(log.to_string_lossy());
        let script_path = dir.path().join("script.json");
        std::fs::write(&script_path, script.to_string()).unwrap();
        let program = dir.path().join("bin").join("claude");
        std::fs::create_dir_all(program.parent().unwrap()).unwrap();
        std::fs::write(
            &program,
            format!(
                "#!/bin/sh\nFAKE_CLAUDE_SCRIPT='{}' exec '{}' \"$@\"\n",
                script_path.display(),
                env!("CARGO_BIN_EXE_fake-claude"),
            ),
        )
        .unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        Stage { dir, program }
    }

    /// The backend on the stand-in.
    pub fn backend(&self) -> ClaudeCode {
        ClaudeCode::with_program(&self.program)
    }

    /// The folder of the stage.
    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// Everything the stand-in logged, oldest first.
    pub fn log(&self) -> Vec<Value> {
        std::fs::read_to_string(self.dir.path().join("log.jsonl"))
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    }

    /// The command lines the stand-in was started with, oldest first.
    pub fn launches(&self) -> Vec<Vec<String>> {
        self.log()
            .iter()
            .filter_map(|entry| entry.get("argv"))
            .map(|argv| {
                argv.as_array()
                    .unwrap()
                    .iter()
                    .map(|arg| arg.as_str().unwrap().to_string())
                    .collect()
            })
            .collect()
    }

    /// The launches that are conversations (not `--version`, not `auth status`).
    pub fn conversations(&self) -> Vec<Vec<String>> {
        self.launches()
            .into_iter()
            .filter(|argv| argv.first().map(String::as_str) == Some("-p"))
            .collect()
    }

    /// The log entry of the first conversation launch: its `argv`, `env` and `cwd`.
    pub fn first_conversation(&self) -> Value {
        self.log()
            .into_iter()
            .find(|entry| entry["argv"][0] == "-p")
            .expect("a conversation was started")
    }

    /// Every line the stand-in was sent, oldest first.
    pub fn received(&self) -> Vec<Value> {
        self.log()
            .iter()
            .filter_map(|entry| entry.get("stdin").cloned())
            .collect()
    }

    /// The text of every user message the stand-in was sent.
    pub fn messages(&self) -> Vec<String> {
        self.received()
            .iter()
            .filter(|line| line["type"] == "user")
            .map(|line| {
                line["message"]["content"][0]["text"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect()
    }

    /// The subtype of every control request the stand-in was sent.
    pub fn requests(&self) -> Vec<String> {
        self.received()
            .iter()
            .filter(|line| line["type"] == "control_request")
            .map(|line| line["request"]["subtype"].as_str().unwrap().to_string())
            .collect()
    }

    /// The events the stand-in noted of itself ("answered", "died", "input closed").
    pub fn notes(&self) -> Vec<String> {
        self.log()
            .iter()
            .filter_map(|entry| {
                entry
                    .get("event")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .collect()
    }

    /// Waits until no conversation process of this stage is running any more.
    pub fn wait_until_processes_are_gone(&self) {
        let pids: Vec<u64> = self
            .log()
            .iter()
            .filter(|entry| entry["argv"][0] == "-p")
            .filter_map(|entry| entry["pid"].as_u64())
            .collect();
        let deadline = std::time::Instant::now() + PATIENCE;
        loop {
            let running: Vec<u64> = pids
                .iter()
                .copied()
                .filter(|pid| {
                    // `kill -0` asks whether the process exists and signals nothing.
                    std::process::Command::new("/bin/kill")
                        .args(["-0", &pid.to_string()])
                        .stderr(std::process::Stdio::null())
                        .status()
                        .is_ok_and(|status| status.success())
                })
                .collect();
            if running.is_empty() {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "processes of the stand-in are still running: {running:?}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Waits until the stand-in's log satisfies `done`.
    pub fn wait_for_log(&self, what: &str, done: impl Fn(&Stage) -> bool) {
        let deadline = std::time::Instant::now() + PATIENCE;
        while !done(self) {
            assert!(
                std::time::Instant::now() < deadline,
                "the stand-in never logged: {what}\nlog: {:#?}",
                self.log()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

/// A conversation on a stage, with its events.
pub struct Talk {
    pub conversation: Conversation,
    pub events: EventStream,
}

impl Talk {
    /// Starts a conversation on `backend`.
    pub fn start(backend: &dyn Backend, options: ConversationOptions) -> Talk {
        let (sink, events) = event_channel();
        Talk {
            conversation: backend.start(options, sink),
            events,
        }
    }

    /// Waits for the end of `turn` and returns every event up to it.
    pub fn until_finished(&self, turn: TurnId) -> Vec<AssistantEvent> {
        let (events, done) = self.events.wait_until(
            PATIENCE,
            |event| matches!(event, AssistantEvent::Finished { turn: finished, .. } if *finished == turn),
        );
        assert!(done, "turn {turn:?} never finished; events: {events:#?}");
        events
    }

    /// Sends `text` and waits for its answer.
    pub fn ask(&self, text: &str) -> Answer {
        let turn = self.conversation.send(text);
        Answer::of(turn, &self.until_finished(turn))
    }
}

/// What a turn came to.
#[derive(Debug)]
pub struct Answer {
    /// The pieces of text, joined.
    pub text: String,
    /// The whole text of the last event, when it carried one.
    pub final_text: Option<String>,
    pub end: TurnEnd,
    pub usage: Option<innerword_assistant::Usage>,
    /// Every event of the turn, in order.
    pub events: Vec<AssistantEvent>,
}

impl Answer {
    /// Reads the events of `turn` out of `events`.
    pub fn of(turn: TurnId, events: &[AssistantEvent]) -> Answer {
        let mut text = String::new();
        let mut finished = None;
        let mut own = Vec::new();
        for event in events {
            if event.turn() != Some(turn) {
                continue;
            }
            own.push(event.clone());
            match event {
                AssistantEvent::Text { text: piece, .. } => text.push_str(piece),
                AssistantEvent::Finished {
                    text, end, usage, ..
                } => finished = Some((text.clone(), end.clone(), usage.clone())),
                _ => {}
            }
        }
        let (final_text, end, usage) = finished.expect("the turn finished");
        Answer {
            text,
            final_text,
            end,
            usage,
            events: own,
        }
    }

    /// The error of a failed turn.
    pub fn error(&self) -> &AssistantError {
        match &self.end {
            TurnEnd::Failed(error) => error,
            other => panic!("the turn did not fail: {other:?}"),
        }
    }

    /// The tool activity of the turn: (description, state) in order.
    pub fn tools(&self) -> Vec<(String, innerword_assistant::ToolState)> {
        self.events
            .iter()
            .filter_map(|event| match event {
                AssistantEvent::Tool { activity, .. } => {
                    Some((activity.description.clone(), activity.state))
                }
                _ => None,
            })
            .collect()
    }

    /// The status lines of the turn, in order.
    pub fn statuses(&self) -> Vec<String> {
        self.events
            .iter()
            .filter_map(|event| match event {
                AssistantEvent::Status { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }
}

/// The value that follows `flag` in a command line.
pub fn value_after<'a>(argv: &'a [String], flag: &str) -> Option<&'a str> {
    let at = argv.iter().position(|arg| arg == flag)?;
    argv.get(at + 1).map(String::as_str)
}
