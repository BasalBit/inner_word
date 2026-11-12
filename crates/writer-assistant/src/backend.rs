//! The interface to a terminal agent.
//!
//! Nothing here assumes a particular agent: a [`Backend`] says what it is called, whether
//! it is installed and signed in, which models and effort levels it offers, and starts
//! [`Conversation`]s. A conversation takes user messages and reports what happens as
//! [`AssistantEvent`]s through an [`EventSink`]. Claude Code is the
//! first implementation ([`ClaudeCode`](crate::ClaudeCode)); the tests of every crate use
//! [`FakeBackend`](crate::FakeBackend).
//!
//! # Threads
//!
//! * [`Backend::start`], [`Conversation::send`], [`Conversation::stop`] and
//!   [`Conversation::close`] never block: call them on the main thread.
//! * [`Backend::availability`], [`Backend::recheck`] and [`Backend::refresh_catalog`] may
//!   run the agent's program and wait for it (tens of milliseconds to a few seconds):
//!   call them on a background thread. [`Backend::cached_availability`] and
//!   [`Backend::catalog`] never block.
//! * Events arrive on the [`EventStream`](crate::EventStream), which is read on the main
//!   thread without blocking.
//!
//! # The contract of a turn
//!
//! Every [`Conversation::send`] returns a [`TurnId`] and is answered by exactly one
//! [`AssistantEvent::Finished`] with that id, whatever happens: the answer completed, it
//! was stopped, the agent failed, the program died, or the conversation was closed.
//! Before it come any number of [`AssistantEvent::Text`] and [`AssistantEvent::Tool`]
//! events of the same turn. A message sent while another is being answered waits its
//! turn; the answers come in the order of the questions.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::events::EventSink;

/// A terminal agent the assistant can run on.
pub trait Backend: Send + Sync + 'static {
    /// What the agent is called and what to tell the user about installing it and
    /// signing in. Never blocks.
    fn info(&self) -> BackendInfo;

    /// Whether the agent is installed and the user signed in. The first call looks (it
    /// may run the agent's program: call it on a background thread); later calls answer
    /// from the cache.
    fn availability(&self) -> Availability;

    /// Looks again, whatever the cache says, and returns the fresh answer. Blocks like
    /// the first [`availability`](Self::availability).
    fn recheck(&self) -> Availability;

    /// The last answer of [`availability`](Self::availability) or
    /// [`recheck`](Self::recheck), if either has run. Never blocks.
    fn cached_availability(&self) -> Option<Availability>;

    /// The models and effort levels to offer, with the defaults of each mode. Never
    /// blocks: it is the list last learned from the agent, or the built-in one.
    fn catalog(&self) -> Catalog;

    /// Asks the agent itself which models and effort levels it offers, where the agent
    /// can say, and returns the result (also kept for [`catalog`](Self::catalog)). May
    /// run the agent's program: call it on a background thread. It never sends a message
    /// and costs the user nothing.
    fn refresh_catalog(&self) -> Catalog;

    /// Starts a conversation. Never blocks and never fails here: when the agent is
    /// missing, signed out or cannot start, the first message sent is answered with
    /// [`AssistantEvent::Finished`] carrying the error.
    ///
    /// Nothing is sent anywhere until [`Conversation::send`] is called.
    fn start(&self, options: ConversationOptions, events: EventSink) -> Conversation;
}

/// What an agent is called, and the words for the two things a user may have to do.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendInfo {
    /// A stable key: `claude-code`. The keys are those of
    /// `writer_agent_setup::AgentId::key` where the agent is in that table.
    pub id: String,
    /// The name to show: "Claude Code".
    pub name: String,
    /// Who makes it and who receives what is sent: "Anthropic".
    pub vendor: String,
    /// One sentence that says how to install it.
    pub install_help: String,
    /// Where the installation is described.
    pub install_url: String,
    /// One sentence that says how to sign in.
    pub sign_in_help: String,
}

/// Whether an agent can be used on this Mac.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Availability {
    /// The program was not found: the feature is not offered.
    NotInstalled,
    /// The program is there.
    Installed(Installation),
}

/// An installed agent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Installation {
    /// The program that was found.
    pub program: PathBuf,
    /// Its version as it prints it ("2.1.322"), when it could be read.
    pub version: Option<String>,
    /// Whether the user is signed in.
    pub sign_in: SignIn,
}

/// Whether the user is signed in to an agent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignIn {
    /// Signed in. `account` is a short line for a person, such as "Claude Max plan" or
    /// "API key"; never an e-mail address.
    SignedIn { account: Option<String> },
    /// Not signed in: the feature is shown and says how to sign in.
    SignedOut,
    /// The agent could not say (its status command failed or is unknown to this
    /// version). Treated as usable: a message will tell.
    Unknown,
}

impl Availability {
    /// Whether the program is there.
    pub fn is_installed(&self) -> bool {
        matches!(self, Availability::Installed(_))
    }

    /// Whether a message can be sent: installed, and not known to be signed out.
    pub fn is_ready(&self) -> bool {
        matches!(self, Availability::Installed(installation) if installation.sign_in != SignIn::SignedOut)
    }

    /// Whether the program is there and says the user is not signed in.
    pub fn is_signed_out(&self) -> bool {
        matches!(self, Availability::Installed(installation) if installation.sign_in == SignIn::SignedOut)
    }

    /// The installation, when there is one.
    pub fn installation(&self) -> Option<&Installation> {
        match self {
            Availability::Installed(installation) => Some(installation),
            Availability::NotInstalled => None,
        }
    }

    /// The version of the program, when it is installed and said.
    pub fn version(&self) -> Option<&str> {
        self.installation()?.version.as_deref()
    }

    /// The short account line of a signed-in user ("Claude Max plan").
    pub fn account(&self) -> Option<&str> {
        match &self.installation()?.sign_in {
            SignIn::SignedIn { account } => account.as_deref(),
            _ => None,
        }
    }
}

/// The two places the assistant appears in. They differ in what the assistant is told,
/// what it may read and which model answers by default.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// The small popover at the selection: a word, a sentence, a paragraph. No tools.
    Quick,
    /// The panel beside the document: the whole document and the library.
    Side,
}

/// Which model answers.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelChoice {
    /// Whatever the agent uses when it is not told: the user's own default.
    #[default]
    AgentDefault,
    /// A model by the id the agent accepts ([`ModelOption::id`]).
    Named(String),
}

/// How much effort the model spends.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffortChoice {
    /// Whatever the agent uses when it is not told.
    #[default]
    AgentDefault,
    /// A level by the id the agent accepts ([`EffortOption::id`]).
    Named(String),
}

impl ModelChoice {
    /// A model by id.
    pub fn named(id: impl Into<String>) -> Self {
        ModelChoice::Named(id.into())
    }

    /// The id, unless the choice is the agent's default.
    pub fn id(&self) -> Option<&str> {
        match self {
            ModelChoice::Named(id) => Some(id),
            ModelChoice::AgentDefault => None,
        }
    }
}

impl EffortChoice {
    /// A level by id.
    pub fn named(id: impl Into<String>) -> Self {
        EffortChoice::Named(id.into())
    }

    /// The id, unless the choice is the agent's default.
    pub fn id(&self) -> Option<&str> {
        match self {
            EffortChoice::Named(id) => Some(id),
            EffortChoice::AgentDefault => None,
        }
    }
}

/// A model and an effort level: what the two pickers choose, what the settings remember
/// for each mode, and what a message was answered with.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Selection {
    /// The model.
    #[serde(default)]
    pub model: ModelChoice,
    /// The effort level.
    #[serde(default)]
    pub effort: EffortChoice,
}

impl Selection {
    /// The agent's own defaults for both.
    pub fn agent_default() -> Self {
        Selection::default()
    }

    /// A model and a level by their ids.
    pub fn named(model: impl Into<String>, effort: impl Into<String>) -> Self {
        Selection {
            model: ModelChoice::named(model),
            effort: EffortChoice::named(effort),
        }
    }
}

/// A model an agent offers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelOption {
    /// What the agent accepts: an alias such as `sonnet` or a full model name.
    pub id: String,
    /// The name to show in the picker.
    pub name: String,
    /// One line about it, when the agent or the built-in table has one.
    pub description: String,
    /// The effort levels this model takes, by id. `None`: every level of the catalog.
    /// An empty list: the model has no effort setting.
    pub efforts: Option<Vec<String>>,
}

/// An effort level an agent offers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffortOption {
    /// What the agent accepts: `low`.
    pub id: String,
    /// The name to show in the picker: "Low".
    pub name: String,
}

/// Where a [`Catalog`] comes from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CatalogSource {
    /// The table built into this version of the app. `as_of` is the day it was read from
    /// the agent's documentation and program.
    BuiltIn { as_of: String },
    /// The agent's program said so itself. `version` is the program's version.
    Agent { version: Option<String> },
}

/// The models and effort levels an agent offers, and the defaults of each mode.
///
/// The pickers show "the agent's default" first (see [`Catalog::agent_default_label`])
/// and then [`models`](Self::models) or [`efforts`](Self::efforts).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Catalog {
    /// The models, in the order to show them.
    pub models: Vec<ModelOption>,
    /// The effort levels, lowest first.
    pub efforts: Vec<EffortOption>,
    /// What the quick chat uses until the user chooses: fast and cheap.
    pub quick_default: Selection,
    /// What the side chat uses until the user chooses: more capable.
    pub side_default: Selection,
    /// The label of the choice "whatever the agent uses" for the model: "Claude Code's
    /// default", with the model that is where the agent says which ("Claude Code's
    /// default (Fable 5.2)").
    pub agent_default_label: String,
    /// The same choice for the effort, where the name of a model has no place:
    /// "Claude Code's default". Empty (as in a catalog stored before this field
    /// existed): [`agent_default_label`](Self::agent_default_label) is shown.
    #[serde(default)]
    pub agent_default_effort_label: String,
    /// Where the list comes from.
    pub source: CatalogSource,
}

impl Catalog {
    /// The default of a mode.
    pub fn default_for(&self, mode: Mode) -> Selection {
        match mode {
            Mode::Quick => self.quick_default.clone(),
            Mode::Side => self.side_default.clone(),
        }
    }

    /// The model with this id.
    pub fn model(&self, id: &str) -> Option<&ModelOption> {
        self.models.iter().find(|model| model.id == id)
    }

    /// The effort level with this id.
    pub fn effort(&self, id: &str) -> Option<&EffortOption> {
        self.efforts.iter().find(|effort| effort.id == id)
    }

    /// The name to show for a model choice: the model's name, the id itself for a model
    /// that is not in the list (a remembered choice the agent no longer offers is still
    /// passed on as it is), or the agent-default label.
    pub fn model_label(&self, choice: &ModelChoice) -> String {
        match choice {
            ModelChoice::AgentDefault => self.agent_default_label.clone(),
            ModelChoice::Named(id) => self
                .model(id)
                .map(|model| model.name.clone())
                .unwrap_or_else(|| id.clone()),
        }
    }

    /// The name to show for an effort choice.
    pub fn effort_label(&self, choice: &EffortChoice) -> String {
        match choice {
            EffortChoice::AgentDefault => {
                if self.agent_default_effort_label.is_empty() {
                    self.agent_default_label.clone()
                } else {
                    self.agent_default_effort_label.clone()
                }
            }
            EffortChoice::Named(id) => self
                .effort(id)
                .map(|effort| effort.name.clone())
                .unwrap_or_else(|| id.clone()),
        }
    }

    /// The effort levels to offer for a model choice: the model's own list when it has
    /// one, else all of them.
    pub fn efforts_for(&self, choice: &ModelChoice) -> Vec<EffortOption> {
        let own = choice
            .id()
            .and_then(|id| self.model(id))
            .and_then(|model| model.efforts.as_ref());
        match own {
            Some(ids) => self
                .efforts
                .iter()
                .filter(|effort| ids.contains(&effort.id))
                .cloned()
                .collect(),
            None => self.efforts.clone(),
        }
    }

    /// `selection` with an effort the chosen model does not take replaced by the agent's
    /// default. A model that is not in the list is left alone.
    pub fn normalized(&self, selection: &Selection) -> Selection {
        let mut selection = selection.clone();
        if let EffortChoice::Named(effort) = &selection.effort {
            let offered = self.efforts_for(&selection.model);
            if !offered.iter().any(|option| &option.id == effort) {
                selection.effort = EffortChoice::AgentDefault;
            }
        }
        selection
    }
}

/// An earlier message of a conversation that is carried into a new one: after the app
/// was restarted, or when the model was changed and the agent has to be started again.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryMessage {
    /// Who said it.
    pub role: Role,
    /// What was said.
    pub text: String,
}

/// Who a message is from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// The person writing.
    User,
    /// The assistant.
    Assistant,
}

/// Says what a tool call is doing, in words for a person ("Reading Chapter 2"), from the
/// tool's name (as the server names it, without any prefix an agent adds) and its
/// arguments. `None`: no better words than the tool's name.
pub type ToolDescriber = Arc<dyn Fn(&str, &serde_json::Value) -> Option<String> + Send + Sync>;

/// Is told what a tool call returned: the tool's name (as the server names it), its
/// arguments and its structured result (JSON `null` when the tool gave none). Used to
/// learn the titles behind ids, so that a later call reads "Reading Chapter 2".
pub type ToolObserver = Arc<dyn Fn(&str, &serde_json::Value, &serde_json::Value) + Send + Sync>;

/// A set of tools given to the agent as an MCP server it starts itself (over stdio).
///
/// The assistant only ever reads: `allowed` are the tools the agent may call without
/// asking, `denied` are the tools of the server it must not see at all. A backend that
/// cannot enforce both must not offer the server.
///
/// Made with [`ToolServer::new`], or for the library of the app with
/// [`library_tools`](crate::library_tools).
#[derive(Clone)]
#[non_exhaustive]
pub struct ToolServer {
    /// The name the server is registered under: `the-writer`.
    pub name: String,
    /// The program to run.
    pub command: PathBuf,
    /// Its arguments.
    pub args: Vec<String>,
    /// Variables added to its environment.
    pub env: Vec<(String, String)>,
    /// The tools that may be called without asking, by the server's own names.
    pub allowed: Vec<String>,
    /// The tools that are withheld from the agent, by the server's own names.
    pub denied: Vec<String>,
    /// Words for a tool call, for the activity line.
    pub describe: Option<ToolDescriber>,
    /// Is told the result of every tool call.
    pub observe: Option<ToolObserver>,
}

impl ToolServer {
    /// A server registered as `name` that is started by running `command`, with no
    /// arguments, no tools allowed and none withheld yet.
    pub fn new(name: impl Into<String>, command: impl Into<PathBuf>) -> Self {
        ToolServer {
            name: name.into(),
            command: command.into(),
            args: Vec::new(),
            env: Vec::new(),
            allowed: Vec::new(),
            denied: Vec::new(),
            describe: None,
            observe: None,
        }
    }

    /// With these arguments.
    pub fn with_args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }

    /// With the tools that may be called without asking.
    pub fn allowing<I, S>(mut self, tools: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.allowed = tools.into_iter().map(Into::into).collect();
        self
    }

    /// With the tools that are withheld.
    pub fn denying<I, S>(mut self, tools: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.denied = tools.into_iter().map(Into::into).collect();
        self
    }
}

impl std::fmt::Debug for ToolServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolServer")
            .field("name", &self.name)
            .field("command", &self.command)
            .field("args", &self.args)
            .field("env", &self.env)
            .field("allowed", &self.allowed)
            .field("denied", &self.denied)
            .field("describe", &self.describe.is_some())
            .field("observe", &self.observe.is_some())
            .finish()
    }
}

impl PartialEq for ToolServer {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.command == other.command
            && self.args == other.args
            && self.env == other.env
            && self.allowed == other.allowed
            && self.denied == other.denied
    }
}

/// What a conversation is started with.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ConversationOptions {
    /// What the assistant is told it is. It replaces whatever the agent would say of
    /// itself.
    pub system_prompt: String,
    /// The model and the effort level.
    pub selection: Selection,
    /// The tools, if any. `None`: the assistant can do nothing but answer.
    pub tools: Option<ToolServer>,
    /// The directory the agent runs in. `None`: a neutral, empty directory the backend
    /// keeps for this purpose (never the user's home or a project).
    pub working_dir: Option<PathBuf>,
    /// The conversation so far, when this one continues an earlier one.
    pub history: Vec<HistoryMessage>,
}

impl ConversationOptions {
    /// Options with this system prompt, the agent's default model and effort, no tools
    /// and no history.
    pub fn new(system_prompt: impl Into<String>) -> Self {
        ConversationOptions {
            system_prompt: system_prompt.into(),
            selection: Selection::default(),
            tools: None,
            working_dir: None,
            history: Vec::new(),
        }
    }

    /// With this model and effort.
    pub fn with_selection(mut self, selection: Selection) -> Self {
        self.selection = selection;
        self
    }

    /// With these tools.
    pub fn with_tools(mut self, tools: Option<ToolServer>) -> Self {
        self.tools = tools;
        self
    }

    /// With this working directory.
    pub fn with_working_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.working_dir = Some(dir.into());
        self
    }

    /// With the conversation so far.
    pub fn with_history(mut self, history: Vec<HistoryMessage>) -> Self {
        self.history = history;
        self
    }
}

/// Names one question and its answer within the process: every event of the answer
/// carries it. Unique across all conversations of a run of the app.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TurnId(pub u64);

static NEXT_TURN: AtomicU64 = AtomicU64::new(1);

impl TurnId {
    /// A new id.
    pub fn next() -> TurnId {
        TurnId(NEXT_TURN.fetch_add(1, Ordering::Relaxed))
    }
}

/// What a [`Conversation`] asks of its backend. Only backends read these.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Command {
    /// Starts the agent now, without sending anything, so that the first answer does not
    /// wait for the program to start.
    Prepare,
    /// A user message.
    Send { turn: TurnId, text: String },
    /// Answer the messages from now on with this model and effort. Sent only to a
    /// conversation that said it can change them while it runs
    /// ([`Conversation::with_live_selection`]).
    Select(Selection),
    /// Stop the answer that is being written (and drop the messages waiting behind it).
    Stop,
    /// End the conversation.
    Close,
}

/// A running conversation with an agent. Dropping it closes it.
///
/// ```
/// use std::time::Duration;
/// use writer_assistant::{
///     AssistantEvent, Backend, ConversationOptions, FakeBackend, TurnEnd, event_channel,
/// };
///
/// let backend = FakeBackend::new();
/// backend.reply("A **heron** is a wading bird.");
///
/// let (sink, events) = event_channel();
/// let conversation = backend.start(ConversationOptions::new("You help with writing."), sink);
/// let turn = conversation.send("What is a heron?");
///
/// let mut answer = String::new();
/// while let Some(event) = events.wait(Duration::from_secs(5)) {
///     match event {
///         AssistantEvent::Text { text, .. } => answer.push_str(&text),
///         AssistantEvent::Finished { turn: finished, end, .. } => {
///             assert_eq!(finished, turn);
///             assert_eq!(end, TurnEnd::Complete);
///             break;
///         }
///         _ => {}
///     }
/// }
/// assert_eq!(answer, "A **heron** is a wading bird.");
/// ```
pub struct Conversation {
    control: Box<dyn Fn(Command) + Send + Sync>,
    alive: Arc<AtomicBool>,
    live_selection: bool,
}

impl Conversation {
    /// For backends: a conversation whose commands go to `control`. `alive` is cleared by
    /// the backend when the conversation can take no more messages (the program is gone,
    /// or it was closed).
    pub fn new(
        control: impl Fn(Command) + Send + Sync + 'static,
        alive: Arc<AtomicBool>,
    ) -> Conversation {
        Conversation {
            control: Box::new(control),
            alive,
            live_selection: false,
        }
    }

    /// For backends: the same conversation, declared able to change its model and
    /// effort while it runs ([`Command::Select`]).
    pub fn with_live_selection(mut self) -> Conversation {
        self.live_selection = true;
        self
    }

    /// Changes the model and effort for the messages from now on, keeping everything
    /// the agent knows of the conversation. Returns `false`, and does nothing, when
    /// this conversation cannot change them while it runs: then start a new one with
    /// the other selection and hand it the history. Never blocks.
    ///
    /// A selection the agent refuses (a model it does not know) shows as the failure of
    /// the next message, with [`AssistantError::Rejected`].
    pub fn select(&self, selection: Selection) -> bool {
        if !self.live_selection || !self.is_alive() {
            return false;
        }
        (self.control)(Command::Select(selection));
        true
    }

    /// Sends a user message and returns the id its answer will carry. Never blocks. A
    /// message sent while another is being answered waits for that answer to finish.
    pub fn send(&self, text: impl Into<String>) -> TurnId {
        let turn = TurnId::next();
        (self.control)(Command::Send {
            turn,
            text: text.into(),
        });
        turn
    }

    /// Starts the agent's program now without sending it anything, so that the first
    /// answer does not also wait for the program to start. Optional; never blocks.
    pub fn prepare(&self) {
        (self.control)(Command::Prepare);
    }

    /// Stops the answer that is being written: its turn finishes with
    /// [`TurnEnd::Stopped`] and whatever text had arrived. Messages waiting behind it
    /// finish as stopped too, without being sent. The conversation goes on. Never blocks.
    pub fn stop(&self) {
        (self.control)(Command::Stop);
    }

    /// Ends the conversation and the agent's process. A turn that was open finishes as
    /// stopped. Never blocks.
    pub fn close(&self) {
        (self.control)(Command::Close);
    }

    /// Whether the conversation can take another message. `false` after the agent's
    /// program died or the conversation was closed: start a new one and hand it the
    /// history ([`ConversationOptions::with_history`]).
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }
}

impl Drop for Conversation {
    fn drop(&mut self) {
        (self.control)(Command::Close);
    }
}

impl std::fmt::Debug for Conversation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Conversation")
            .field("alive", &self.is_alive())
            .finish()
    }
}

/// What a conversation reports.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum AssistantEvent {
    /// The agent's program is running and ready. `model` is the model that will answer,
    /// as the agent names it, when it says.
    Ready { model: Option<String> },
    /// The agent has begun on this message.
    Started { turn: TurnId },
    /// A piece of the answer, to be appended to what came before.
    Text { turn: TurnId, text: String },
    /// The assistant uses a tool, or has finished using it (the same
    /// [`ToolActivity::id`] with another state).
    Tool {
        turn: TurnId,
        activity: ToolActivity,
    },
    /// What is happening while no text arrives, for the line under the answer:
    /// "Thinking", "No connection, trying again (2 of 4)". It holds until the next
    /// event of the turn; an empty text clears it.
    Status { turn: TurnId, text: String },
    /// The answer to this message is over. Sent exactly once for every message.
    Finished {
        turn: TurnId,
        /// The whole answer as the agent gave it at the end, when it did; else `None`
        /// and the answer is the pieces that arrived.
        text: Option<String>,
        /// How it ended.
        end: TurnEnd,
        /// What it cost, when the agent says.
        usage: Option<Usage>,
    },
    /// Something the user should know that is not the end of an answer: the usage limit
    /// is close, for example.
    Notice { text: String },
}

impl AssistantEvent {
    /// The turn the event belongs to, when it belongs to one.
    pub fn turn(&self) -> Option<TurnId> {
        match self {
            AssistantEvent::Started { turn }
            | AssistantEvent::Text { turn, .. }
            | AssistantEvent::Tool { turn, .. }
            | AssistantEvent::Status { turn, .. }
            | AssistantEvent::Finished { turn, .. } => Some(*turn),
            AssistantEvent::Ready { .. } | AssistantEvent::Notice { .. } => None,
        }
    }
}

/// How an answer ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnEnd {
    /// The assistant finished what it had to say.
    Complete,
    /// The user stopped it, or the conversation was closed.
    Stopped,
    /// It failed.
    Failed(AssistantError),
}

/// The assistant using a tool.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolActivity {
    /// Names this call within its answer.
    pub id: String,
    /// The tool, by the server's own name: `read_document`.
    pub tool: String,
    /// What is happening, for a person: "Reading Chapter 2".
    pub description: String,
    /// Where the call stands.
    pub state: ToolState,
}

/// Where a tool call stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolState {
    /// The tool is running.
    Running,
    /// It answered.
    Done,
    /// It failed or was refused.
    Failed,
}

/// What an answer cost, as far as the agent reports it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    /// Tokens read by the model (context written to and read from a cache included).
    #[serde(default)]
    pub input_tokens: Option<u64>,
    /// Tokens written by the model.
    #[serde(default)]
    pub output_tokens: Option<u64>,
    /// What the agent says the answer cost in US dollars (with a subscription this is
    /// what it would have cost, not what was charged).
    #[serde(default)]
    pub cost_usd: Option<f64>,
    /// How long the agent worked on it, in milliseconds.
    #[serde(default)]
    pub duration_ms: Option<u64>,
}

/// What can go wrong, in kinds the interface can act on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum AssistantError {
    /// The agent's program is not on this Mac (or no longer where it was).
    #[error("the agent is not installed")]
    NotInstalled,
    /// The user is not signed in to the agent, or the sign-in has expired.
    #[error("not signed in to the agent")]
    NotSignedIn,
    /// The installed agent is older than the assistant can safely use. `found` is its
    /// version, `needed` the oldest one that will do.
    #[error("the agent is too old: {found}, {needed} is needed")]
    Outdated { found: String, needed: String },
    /// The user's allowance is used up. `resets_at` is when it comes back, in seconds
    /// since 1970 (UTC), when the agent says; `detail` is the agent's own sentence.
    #[error("the usage limit is reached: {detail}")]
    UsageLimit {
        resets_at: Option<i64>,
        detail: String,
    },
    /// The agent could not reach its service.
    #[error("no connection: {detail}")]
    Network { detail: String },
    /// The agent's service is there and cannot answer now (overloaded, an internal
    /// error). Trying again later may work.
    #[error("the service cannot answer now: {detail}")]
    Unavailable { detail: String },
    /// The agent's program ended when it should not have. `code` is its exit code, when
    /// it has one; `detail` is the last of what it printed.
    #[error("the agent stopped unexpectedly: {detail}")]
    Exited { code: Option<i32>, detail: String },
    /// The agent refused the request as it was made: a model it does not know, an option
    /// it does not take.
    #[error("the agent did not accept the request: {detail}")]
    Rejected { detail: String },
    /// Anything else the agent reported.
    #[error("{detail}")]
    Other { detail: String },
}

/// An error in words for the person writing: what happened, and what to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ErrorText {
    /// What happened, one sentence.
    pub headline: String,
    /// What to do about it, one or two sentences. May be empty.
    pub advice: String,
}

impl ErrorText {
    /// Both as one line.
    pub fn line(&self) -> String {
        if self.advice.is_empty() {
            self.headline.clone()
        } else {
            format!("{} {}", self.headline, self.advice)
        }
    }
}

impl AssistantError {
    /// The error in plain words, for the agent described by `info`.
    pub fn text(&self, info: &BackendInfo) -> ErrorText {
        let name = &info.name;
        match self {
            AssistantError::NotInstalled => ErrorText {
                headline: format!("{name} is not installed on this Mac."),
                advice: info.install_help.clone(),
            },
            AssistantError::NotSignedIn => ErrorText {
                headline: format!("You are not signed in to {name}."),
                advice: format!("{} Then send the message again.", info.sign_in_help),
            },
            AssistantError::Outdated { found, needed } => ErrorText {
                headline: format!("{name} {found} is too old for the assistant."),
                advice: format!(
                    "Update {name} to version {needed} or newer, then send the message again."
                ),
            },
            AssistantError::UsageLimit { resets_at, detail } => ErrorText {
                headline: format!("You have reached the usage limit of your {name} plan."),
                advice: match resets_at {
                    Some(at) => format!(
                        "It resets {}. Until then the assistant cannot answer.",
                        crate::time::describe_moment(*at, crate::time::now_seconds())
                    ),
                    None if !detail.is_empty() => {
                        format!("{name} says: {}", one_line(detail, 200))
                    }
                    None => "Try again later.".to_string(),
                },
            },
            AssistantError::Network { .. } => ErrorText {
                headline: format!("{name} could not reach {}.", info.vendor),
                advice: "Check the internet connection, then send the message again.".to_string(),
            },
            AssistantError::Unavailable { detail } => ErrorText {
                headline: format!("{} cannot answer at the moment.", info.vendor),
                advice: if detail.is_empty() {
                    "Send the message again in a little while.".to_string()
                } else {
                    format!(
                        "Send the message again in a little while. {name} says: {}",
                        one_line(detail, 200)
                    )
                },
            },
            AssistantError::Exited { detail, .. } => ErrorText {
                headline: format!("{name} stopped unexpectedly."),
                advice: if detail.is_empty() {
                    "Send the message again; the conversation is kept.".to_string()
                } else {
                    format!(
                        "Send the message again; the conversation is kept. It said: {}",
                        one_line(detail, 200)
                    )
                },
            },
            AssistantError::Rejected { detail } => ErrorText {
                headline: format!("{name} did not accept the request."),
                advice: if detail.is_empty() {
                    "Choose another model or effort level and send the message again.".to_string()
                } else {
                    format!(
                        "{} Choose another model or effort level and send the message again.",
                        sentence(&one_line(detail, 200))
                    )
                },
            },
            AssistantError::Other { detail } => ErrorText {
                headline: format!("{name} could not answer."),
                advice: sentence(&one_line(detail, 300)),
            },
        }
    }

    /// Whether sending the same message again may work without the user doing anything
    /// first.
    pub fn may_retry(&self) -> bool {
        matches!(
            self,
            AssistantError::Network { .. }
                | AssistantError::Unavailable { .. }
                | AssistantError::Exited { .. }
                | AssistantError::Other { .. }
        )
    }
}

/// `text` on one line, cut to at most `limit` characters.
pub(crate) fn one_line(text: &str, limit: usize) -> String {
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if line.chars().count() > limit {
        let cut: String = line.chars().take(limit.saturating_sub(1)).collect();
        format!("{}…", cut.trim_end())
    } else {
        line
    }
}

/// `text` ending as a sentence does.
fn sentence(text: &str) -> String {
    let text = text.trim();
    if text.is_empty() || text.ends_with(['.', '!', '?', '…']) {
        text.to_string()
    } else {
        format!("{text}.")
    }
}
