//! A backend for tests: scripted answers, no process, no thread.
//!
//! [`FakeBackend`] answers every message from a script the test wrote
//! ([`FakeBackend::reply`], [`FakeBackend::push`]) and records everything it was asked
//! ([`FakeBackend::started`], [`FakeBackend::sent`]). It runs no thread: the events of an
//! answer are queued on the calling thread, inside [`Conversation::send`], so a GPUI test
//! sees them after `cx.run_until_parked()` without any waiting.
//!
//! To see an answer while it is being written, or to stop one, hold the events back:
//!
//! ```
//! use writer_assistant::{AssistantEvent, Backend, ConversationOptions, FakeBackend, FakeReply,
//!     TurnEnd, event_channel};
//!
//! let fake = FakeBackend::new();
//! fake.hold();                                       // nothing arrives until the test says
//! fake.push(FakeReply::pieces(["One ", "two ", "three."]));
//!
//! let (sink, events) = event_channel();
//! let conversation = fake.start(ConversationOptions::new("system"), sink);
//! let turn = conversation.send("Count.");
//! assert!(events.drain().is_empty());
//!
//! fake.advance(3);                                   // Ready, Started, "One "
//! assert!(matches!(events.drain().last(), Some(AssistantEvent::Text { text, .. }) if text == "One "));
//!
//! conversation.stop();                               // the user presses Stop
//! assert!(matches!(
//!     events.drain().last(),
//!     Some(AssistantEvent::Finished { turn: stopped, end: TurnEnd::Stopped, .. }) if *stopped == turn
//! ));
//! ```

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::backend::{
    AssistantError, AssistantEvent, Availability, Backend, BackendInfo, Catalog, CatalogSource,
    Command, Conversation, ConversationOptions, EffortOption, Installation, ModelOption, Selection,
    SignIn, ToolActivity, ToolState, TurnEnd, TurnId, Usage,
};
use crate::events::EventSink;

/// One scripted answer: pieces of text, tool calls, and how it ends.
#[derive(Clone, Debug, PartialEq)]
pub struct FakeReply {
    steps: Vec<Step>,
    end: TurnEnd,
    usage: Option<Usage>,
    final_text: bool,
}

#[derive(Clone, Debug, PartialEq)]
enum Step {
    Text(String),
    Tool(ToolActivity),
    Status(String),
}

impl FakeReply {
    /// An answer that arrives word by word and completes.
    pub fn text(answer: impl AsRef<str>) -> Self {
        let mut pieces = Vec::new();
        let mut piece = String::new();
        for ch in answer.as_ref().chars() {
            piece.push(ch);
            if ch.is_whitespace() {
                pieces.push(std::mem::take(&mut piece));
            }
        }
        if !piece.is_empty() {
            pieces.push(piece);
        }
        FakeReply::pieces(pieces)
    }

    /// An answer that arrives in exactly these pieces and completes.
    pub fn pieces<I, S>(pieces: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        FakeReply {
            steps: pieces
                .into_iter()
                .map(|piece| Step::Text(piece.into()))
                .collect(),
            end: TurnEnd::Complete,
            usage: None,
            final_text: true,
        }
    }

    /// An answer that fails at once with `error`.
    pub fn error(error: AssistantError) -> Self {
        FakeReply {
            steps: Vec::new(),
            end: TurnEnd::Failed(error),
            usage: None,
            final_text: false,
        }
    }

    /// The same answer, failing with `error` after its pieces instead of completing.
    pub fn then_fail(mut self, error: AssistantError) -> Self {
        self.end = TurnEnd::Failed(error);
        self.final_text = false;
        self
    }

    /// The same answer with a tool call before what it has so far: the tool starts
    /// (one event), then finishes (another).
    pub fn with_tool(mut self, tool: impl Into<String>, description: impl Into<String>) -> Self {
        self.insert_tool(tool.into(), description.into(), ToolState::Done);
        self
    }

    /// Like [`with_tool`](Self::with_tool), with a call that fails.
    pub fn with_failed_tool(
        mut self,
        tool: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        self.insert_tool(tool.into(), description.into(), ToolState::Failed);
        self
    }

    fn insert_tool(&mut self, tool: String, description: String, outcome: ToolState) {
        let calls = self
            .steps
            .iter()
            .filter(
                |step| matches!(step, Step::Tool(activity) if activity.state == ToolState::Running),
            )
            .count();
        let id = format!("fake-tool-{}", calls + 1);
        let at = self
            .steps
            .iter()
            .position(|step| matches!(step, Step::Text(_)))
            .unwrap_or(self.steps.len());
        let running = ToolActivity {
            id,
            tool,
            description,
            state: ToolState::Running,
        };
        let finished = ToolActivity {
            state: outcome,
            ..running.clone()
        };
        self.steps.insert(at, Step::Tool(finished));
        self.steps.insert(at, Step::Tool(running));
    }

    /// The same answer with a status line ("Thinking") before what it has so far.
    pub fn with_status(mut self, text: impl Into<String>) -> Self {
        self.steps.insert(0, Step::Status(text.into()));
        self
    }

    /// The same answer, reporting this usage when it ends.
    pub fn with_usage(mut self, usage: Usage) -> Self {
        self.usage = Some(usage);
        self
    }

    /// The same answer without the whole text in its last event: the answer is only the
    /// pieces (as an agent that streams and gives no final text).
    pub fn without_final_text(mut self) -> Self {
        self.final_text = false;
        self
    }

    /// The whole text of the answer.
    pub fn full_text(&self) -> String {
        self.steps
            .iter()
            .filter_map(|step| match step {
                Step::Text(text) => Some(text.as_str()),
                Step::Tool(_) | Step::Status(_) => None,
            })
            .collect()
    }
}

impl From<&str> for FakeReply {
    fn from(answer: &str) -> Self {
        FakeReply::text(answer)
    }
}

impl From<String> for FakeReply {
    fn from(answer: String) -> Self {
        FakeReply::text(answer)
    }
}

/// A message a [`FakeBackend`] was sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FakeSent {
    /// The conversation, by the order they were started in, from 0.
    pub conversation: usize,
    /// The turn the message opened.
    pub turn: TurnId,
    /// The text exactly as the agent would have received it.
    pub text: String,
}

struct Turn {
    turn: TurnId,
    started: bool,
    events: VecDeque<AssistantEvent>,
}

struct Talk {
    sink: EventSink,
    alive: Arc<AtomicBool>,
    ready: bool,
    turns: VecDeque<Turn>,
}

impl Talk {
    /// Ends every open turn with `end` and forgets what was left of them.
    fn end_all(&mut self, end: &TurnEnd) {
        for turn in self.turns.drain(..) {
            self.sink.send(AssistantEvent::Finished {
                turn: turn.turn,
                text: None,
                end: end.clone(),
                usage: None,
            });
        }
    }
}

struct State {
    info: BackendInfo,
    availability: Availability,
    checked: bool,
    checks: usize,
    catalog: Catalog,
    catalog_refreshes: usize,
    refreshed_catalog: Option<Catalog>,
    script: VecDeque<FakeReply>,
    fallback: Option<FakeReply>,
    hold: bool,
    live_selection: bool,
    selections: Vec<(usize, Selection)>,
    started: Vec<ConversationOptions>,
    talks: Vec<Talk>,
    sent: Vec<FakeSent>,
    stops: usize,
    closes: usize,
    prepares: usize,
}

/// A scripted agent. Clones share one state: keep one to script and inspect, hand the
/// other to the code under test as `Arc<dyn Backend>`.
#[derive(Clone)]
pub struct FakeBackend {
    state: Arc<Mutex<State>>,
}

impl Default for FakeBackend {
    fn default() -> Self {
        FakeBackend::new()
    }
}

impl std::fmt::Debug for FakeBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FakeBackend")
    }
}

impl FakeBackend {
    /// An agent called "Fake Agent" that is installed and signed in, with a small
    /// catalog of its own (see [`fake_catalog`]). Without a script it answers every
    /// message with `Answer to: <the message>`.
    pub fn new() -> Self {
        FakeBackend {
            state: Arc::new(Mutex::new(State {
                info: fake_info(),
                availability: Availability::Installed(Installation {
                    program: PathBuf::from("/fake/bin/agent"),
                    version: Some("1.0.0".to_string()),
                    sign_in: SignIn::SignedIn {
                        account: Some("Fake plan".to_string()),
                    },
                }),
                checked: false,
                checks: 0,
                catalog: fake_catalog(),
                catalog_refreshes: 0,
                refreshed_catalog: None,
                script: VecDeque::new(),
                fallback: None,
                hold: false,
                live_selection: false,
                selections: Vec::new(),
                started: Vec::new(),
                talks: Vec::new(),
                sent: Vec::new(),
                stops: 0,
                closes: 0,
                prepares: 0,
            })),
        }
    }

    /// An agent that is not installed.
    pub fn not_installed() -> Self {
        let fake = FakeBackend::new();
        fake.set_availability(Availability::NotInstalled);
        fake
    }

    /// An agent that is installed and signed out.
    pub fn signed_out() -> Self {
        let fake = FakeBackend::new();
        fake.set_sign_in(SignIn::SignedOut);
        fake
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    // ----- what the agent is ---------------------------------------------------------

    /// Changes what [`Backend::info`] says.
    pub fn set_info(&self, info: BackendInfo) {
        self.lock().info = info;
    }

    /// Changes what the next [`Backend::recheck`] finds (and what
    /// [`Backend::availability`] says once it has looked).
    pub fn set_availability(&self, availability: Availability) {
        self.lock().availability = availability;
    }

    /// Keeps the agent installed and changes its sign-in state.
    pub fn set_sign_in(&self, sign_in: SignIn) {
        let mut state = self.lock();
        let installation = match &state.availability {
            Availability::Installed(installation) => Installation {
                sign_in,
                ..installation.clone()
            },
            Availability::NotInstalled => Installation {
                program: PathBuf::from("/fake/bin/agent"),
                version: Some("1.0.0".to_string()),
                sign_in,
            },
        };
        state.availability = Availability::Installed(installation);
    }

    /// Changes what [`Backend::catalog`] says.
    pub fn set_catalog(&self, catalog: Catalog) {
        self.lock().catalog = catalog;
    }

    /// What [`Backend::refresh_catalog`] will find (it then becomes the catalog).
    /// Without it a refresh finds the catalog unchanged.
    pub fn set_refreshed_catalog(&self, catalog: Catalog) {
        self.lock().refreshed_catalog = Some(catalog);
    }

    /// How often the availability was looked up for real (the first
    /// [`Backend::availability`] and every [`Backend::recheck`]).
    pub fn availability_checks(&self) -> usize {
        self.lock().checks
    }

    /// How often [`Backend::refresh_catalog`] ran.
    pub fn catalog_refreshes(&self) -> usize {
        self.lock().catalog_refreshes
    }

    // ----- the script ----------------------------------------------------------------

    /// Scripts the answer to the next message that has none yet: `text`, arriving word
    /// by word. Answers are used in the order they were scripted.
    pub fn reply(&self, text: impl AsRef<str>) {
        self.push(FakeReply::text(text));
    }

    /// Scripts the answer to the next message that has none yet.
    pub fn push(&self, reply: FakeReply) {
        self.lock().script.push_back(reply);
    }

    /// Scripts a failure as the answer to the next message.
    pub fn fail(&self, error: AssistantError) {
        self.push(FakeReply::error(error));
    }

    /// The answer to every message the script has nothing for. Without it the answer is
    /// `Answer to: <the message>`.
    pub fn set_fallback(&self, reply: FakeReply) {
        self.lock().fallback = Some(reply);
    }

    /// Whether conversations started from now on can change their model and effort
    /// while they run ([`Conversation::select`]), as Claude Code can. Off by default: a
    /// change of selection then starts a new conversation with the history.
    pub fn set_live_selection(&self, live: bool) {
        self.lock().live_selection = live;
    }

    /// Every change of model and effort a running conversation was asked for, with the
    /// index of the conversation.
    pub fn selections(&self) -> Vec<(usize, Selection)> {
        self.lock().selections.clone()
    }

    /// Holds every event back until [`advance`](Self::advance),
    /// [`finish`](Self::finish) or [`release`](Self::release) lets it through.
    pub fn hold(&self) {
        self.lock().hold = true;
    }

    /// Lets the next `count` held events through, oldest conversation first. Returns how
    /// many there were to let through.
    pub fn advance(&self, count: usize) -> usize {
        let mut state = self.lock();
        let mut delivered = 0;
        while delivered < count {
            if !deliver_one(&mut state) {
                break;
            }
            delivered += 1;
        }
        delivered
    }

    /// Lets every held event through; events stay held for messages sent later.
    pub fn finish(&self) -> usize {
        let mut state = self.lock();
        let mut delivered = 0;
        while deliver_one(&mut state) {
            delivered += 1;
        }
        delivered
    }

    /// Stops holding: lets every held event through, and events of later messages
    /// arrive at once again.
    pub fn release(&self) {
        let mut state = self.lock();
        state.hold = false;
        while deliver_one(&mut state) {}
    }

    /// How many events are held back.
    pub fn held(&self) -> usize {
        let state = self.lock();
        state
            .talks
            .iter()
            .map(|talk| {
                talk.turns
                    .iter()
                    .map(|turn| turn.events.len() + usize::from(!turn.started))
                    .sum::<usize>()
            })
            .sum()
    }

    /// The agent's program dies: every open answer fails with
    /// [`AssistantError::Exited`] and no conversation is alive any more.
    pub fn crash(&self) {
        let mut state = self.lock();
        let end = TurnEnd::Failed(AssistantError::Exited {
            code: Some(1),
            detail: "the fake agent crashed".to_string(),
        });
        for talk in &mut state.talks {
            talk.end_all(&end);
            talk.alive.store(false, Ordering::SeqCst);
        }
    }

    /// Sends a notice into every conversation that is alive.
    pub fn notice(&self, text: impl Into<String>) {
        let text = text.into();
        let state = self.lock();
        for talk in &state.talks {
            if talk.alive.load(Ordering::SeqCst) {
                talk.sink
                    .send(AssistantEvent::Notice { text: text.clone() });
            }
        }
    }

    // ----- what was asked of it ------------------------------------------------------

    /// The options of every conversation that was started, in order.
    pub fn started(&self) -> Vec<ConversationOptions> {
        self.lock().started.clone()
    }

    /// The options of the conversation started last.
    pub fn last_started(&self) -> Option<ConversationOptions> {
        self.lock().started.last().cloned()
    }

    /// Every message that was sent, in order.
    pub fn sent(&self) -> Vec<FakeSent> {
        self.lock().sent.clone()
    }

    /// The text of every message that was sent, in order.
    pub fn sent_texts(&self) -> Vec<String> {
        self.lock()
            .sent
            .iter()
            .map(|sent| sent.text.clone())
            .collect()
    }

    /// The text of the message sent last.
    pub fn last_sent(&self) -> Option<String> {
        self.lock().sent.last().map(|sent| sent.text.clone())
    }

    /// How often an answer was stopped.
    pub fn stops(&self) -> usize {
        self.lock().stops
    }

    /// How many conversations were closed (or dropped).
    pub fn closes(&self) -> usize {
        self.lock().closes
    }

    /// How often a conversation was asked to start its program ahead of the first
    /// message.
    pub fn prepares(&self) -> usize {
        self.lock().prepares
    }

    /// How many conversations are alive.
    pub fn alive(&self) -> usize {
        self.lock()
            .talks
            .iter()
            .filter(|talk| talk.alive.load(Ordering::SeqCst))
            .count()
    }

    fn command(&self, index: usize, command: Command) {
        let mut state = self.lock();
        match command {
            Command::Prepare => {
                state.prepares += 1;
                let talk = &mut state.talks[index];
                if talk.alive.load(Ordering::SeqCst) && !talk.ready {
                    talk.ready = true;
                    talk.sink.send(AssistantEvent::Ready {
                        model: Some("fake-model".to_string()),
                    });
                }
            }
            Command::Send { turn, text } => {
                state.sent.push(FakeSent {
                    conversation: index,
                    turn,
                    text: text.clone(),
                });
                if !state.talks[index].alive.load(Ordering::SeqCst) {
                    state.talks[index].sink.send(AssistantEvent::Finished {
                        turn,
                        text: None,
                        end: TurnEnd::Failed(AssistantError::Exited {
                            code: None,
                            detail: "the conversation is closed".to_string(),
                        }),
                        usage: None,
                    });
                    return;
                }
                let availability = state.availability.clone();
                let reply = match availability {
                    Availability::NotInstalled => FakeReply::error(AssistantError::NotInstalled),
                    Availability::Installed(Installation {
                        sign_in: SignIn::SignedOut,
                        ..
                    }) => FakeReply::error(AssistantError::NotSignedIn),
                    Availability::Installed(_) => state
                        .script
                        .pop_front()
                        .or_else(|| state.fallback.clone())
                        .unwrap_or_else(|| {
                            FakeReply::text(format!(
                                "Answer to: {}",
                                crate::backend::one_line(&text, 80)
                            ))
                        }),
                };
                let mut events = VecDeque::new();
                for step in &reply.steps {
                    events.push_back(match step {
                        Step::Text(text) => AssistantEvent::Text {
                            turn,
                            text: text.clone(),
                        },
                        Step::Tool(activity) => AssistantEvent::Tool {
                            turn,
                            activity: activity.clone(),
                        },
                        Step::Status(text) => AssistantEvent::Status {
                            turn,
                            text: text.clone(),
                        },
                    });
                }
                events.push_back(AssistantEvent::Finished {
                    turn,
                    text: reply.final_text.then(|| reply.full_text()),
                    end: reply.end.clone(),
                    usage: reply.usage.clone(),
                });
                state.talks[index].turns.push_back(Turn {
                    turn,
                    started: false,
                    events,
                });
                if !state.hold {
                    while deliver_one(&mut state) {}
                }
            }
            Command::Select(selection) => {
                state.selections.push((index, selection));
            }
            Command::Stop => {
                state.stops += 1;
                state.talks[index].end_all(&TurnEnd::Stopped);
            }
            Command::Close => {
                let talk = &mut state.talks[index];
                if talk.alive.swap(false, Ordering::SeqCst) {
                    talk.end_all(&TurnEnd::Stopped);
                    state.closes += 1;
                }
            }
        }
    }
}

/// Lets the next held event through. Returns whether there was one.
fn deliver_one(state: &mut State) -> bool {
    for talk in &mut state.talks {
        let Some(turn) = talk.turns.front_mut() else {
            continue;
        };
        if !talk.ready {
            talk.ready = true;
            talk.sink.send(AssistantEvent::Ready {
                model: Some("fake-model".to_string()),
            });
            return true;
        }
        if !turn.started {
            turn.started = true;
            talk.sink.send(AssistantEvent::Started { turn: turn.turn });
            return true;
        }
        if let Some(event) = turn.events.pop_front() {
            talk.sink.send(event);
        }
        if turn.events.is_empty() {
            talk.turns.pop_front();
        }
        return true;
    }
    false
}

impl Backend for FakeBackend {
    fn info(&self) -> BackendInfo {
        self.lock().info.clone()
    }

    fn availability(&self) -> Availability {
        let mut state = self.lock();
        if !state.checked {
            state.checked = true;
            state.checks += 1;
        }
        state.availability.clone()
    }

    fn recheck(&self) -> Availability {
        let mut state = self.lock();
        state.checked = true;
        state.checks += 1;
        state.availability.clone()
    }

    fn cached_availability(&self) -> Option<Availability> {
        let state = self.lock();
        state.checked.then(|| state.availability.clone())
    }

    fn catalog(&self) -> Catalog {
        self.lock().catalog.clone()
    }

    fn refresh_catalog(&self) -> Catalog {
        let mut state = self.lock();
        state.catalog_refreshes += 1;
        if let Some(catalog) = state.refreshed_catalog.take() {
            state.catalog = catalog;
        }
        state.catalog.clone()
    }

    fn start(&self, options: ConversationOptions, events: EventSink) -> Conversation {
        let alive = Arc::new(AtomicBool::new(true));
        let (index, live) = {
            let mut state = self.lock();
            let live = state.live_selection;
            state.started.push(options);
            state.talks.push(Talk {
                sink: events,
                alive: alive.clone(),
                ready: false,
                turns: VecDeque::new(),
            });
            (state.talks.len() - 1, live)
        };
        let fake = self.clone();
        let conversation = Conversation::new(move |command| fake.command(index, command), alive);
        if live {
            conversation.with_live_selection()
        } else {
            conversation
        }
    }
}

/// What the fake agent says of itself: "Fake Agent" by "Fake Vendor".
pub fn fake_info() -> BackendInfo {
    BackendInfo {
        id: "fake-agent".to_string(),
        name: "Fake Agent".to_string(),
        vendor: "Fake Vendor".to_string(),
        install_help: "Install Fake Agent, then open the settings again.".to_string(),
        install_url: "https://example.com/fake-agent".to_string(),
        sign_in_help: "Open Terminal, run fake-agent and sign in.".to_string(),
    }
}

/// The catalog of the fake agent: the models `swift` ("Swift") and `deep` ("Deep"; it
/// takes every level), the levels `low`, `medium` and `high`; `swift` takes only `low`
/// and `medium`. The quick chat defaults to `swift` at `low`, the side chat to `deep` at
/// `medium`.
pub fn fake_catalog() -> Catalog {
    Catalog {
        models: vec![
            ModelOption {
                id: "swift".to_string(),
                name: "Swift".to_string(),
                description: "Fast answers".to_string(),
                efforts: Some(vec!["low".to_string(), "medium".to_string()]),
            },
            ModelOption {
                id: "deep".to_string(),
                name: "Deep".to_string(),
                description: "Careful answers".to_string(),
                efforts: None,
            },
        ],
        efforts: vec![
            EffortOption {
                id: "low".to_string(),
                name: "Low".to_string(),
            },
            EffortOption {
                id: "medium".to_string(),
                name: "Medium".to_string(),
            },
            EffortOption {
                id: "high".to_string(),
                name: "High".to_string(),
            },
        ],
        quick_default: Selection::named("swift", "low"),
        side_default: Selection::named("deep", "medium"),
        agent_default_label: "Fake Agent's default".to_string(),
        source: CatalogSource::BuiltIn {
            as_of: "2026-11-12".to_string(),
        },
    }
}
