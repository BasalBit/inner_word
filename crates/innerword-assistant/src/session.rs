//! One chat as a view holds it: the transcript, the conversation with the agent, and the
//! rules that connect the two.
//!
//! A [`ChatSession`] is what the quick chat popover and the side chat panel each own. It
//! starts the agent only when the first message is sent, assembles what each message
//! carries ([`ChatSession::ask_quick`], [`ChatSession::ask_side`]), fills the transcript
//! from the events ([`ChatSession::apply`]), and carries the conversation over into a new
//! process of the agent when one is needed: after the model or the effort was changed,
//! after the program died, and after the app was restarted with a stored transcript.
//!
//! Every method returns at once; none blocks the main thread.
//!
//! # In a GPUI view
//!
//! ```ignore
//! struct QuickChat { session: ChatSession, /* ... */ }
//!
//! // Once, when the view is made: pump the events into the session.
//! let events = self.session.events();
//! cx.spawn(async move |this, cx| {
//!     while let Some(batch) = events.next_batch().await {
//!         let alive = this.update(cx, |this, cx| {
//!             let mut changed = false;
//!             for event in batch {
//!                 changed |= this.session.apply(event);
//!             }
//!             if changed {
//!                 cx.notify();
//!             }
//!         });
//!         if alive.is_err() {
//!             break; // the view is gone: the session and its conversation go with it
//!         }
//!     }
//! })
//! .detach();
//!
//! // When the user presses enter:
//! self.session.ask_quick(&question, selected_text.as_deref());
//! cx.notify();
//!
//! // In render: self.session.transcript().messages, self.session.is_answering().
//! // Stop button: self.session.stop().
//! ```
//!
//! The stream of a session lives as long as the session: it is the same stream across
//! new conversations, model changes and restarts of the agent.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::backend::{
    AssistantEvent, Backend, Conversation, ConversationOptions, Mode, Selection, ToolServer, TurnId,
};
use crate::context::{HISTORY_LIMIT, OpenDocument, SideContext, quick_prompt};
use crate::drawing::DrawingTarget;
use crate::events::{EventSink, EventStream, event_channel};
use crate::prompts;
use crate::tools::{LIBRARY_SERVER_NAME, NameBook, library_describer, library_observer};
use crate::transcript::{Attachment, AttachmentKind, MessageId, Transcript};

/// A conversation that is running, and what it was started with.
struct Running {
    conversation: Conversation,
    selection: Selection,
    generation: u64,
    /// The drawings its system prompt offers.
    target: DrawingTarget,
}

/// One chat: a transcript and the conversation with the agent behind it.
pub struct ChatSession {
    backend: Arc<dyn Backend>,
    mode: Mode,
    transcript: Transcript,
    selection: Selection,
    tools: Option<ToolServer>,
    working_dir: Option<PathBuf>,
    system_prompt: Option<String>,
    running: Option<Running>,
    /// Conversations that were replaced while an answer of theirs was still open: kept
    /// until that answer has finished.
    retiring: Vec<Running>,
    /// The open turns and the generation of the conversation each belongs to.
    open_turns: HashMap<TurnId, u64>,
    generation: u64,
    side: SideContext,
    names: NameBook,
    sink: EventSink,
    stream: EventStream,
    model_in_use: Option<String>,
    notice: Option<String>,
    /// What drawings the side chat offers: its system prompt follows.
    drawing_target: DrawingTarget,
}

impl ChatSession {
    /// A chat of `mode` on `backend`, with an empty transcript and the default model and
    /// effort of the mode ([`Catalog::default_for`](crate::Catalog::default_for)).
    /// Nothing is started and nothing is sent until the first message.
    pub fn new(backend: Arc<dyn Backend>, mode: Mode) -> Self {
        let selection = backend.catalog().default_for(mode);
        let (sink, stream) = event_channel();
        ChatSession {
            backend,
            mode,
            transcript: Transcript::new(),
            selection,
            tools: None,
            working_dir: None,
            system_prompt: None,
            running: None,
            retiring: Vec::new(),
            open_turns: HashMap::new(),
            generation: 0,
            side: SideContext::new(),
            names: NameBook::new(),
            sink,
            stream,
            model_in_use: None,
            notice: None,
            drawing_target: DrawingTarget::None,
        }
    }

    /// A quick chat: no tools, the concise system prompt.
    pub fn quick(backend: Arc<dyn Backend>) -> Self {
        ChatSession::new(backend, Mode::Quick)
    }

    /// A side chat. `tools` is the library's MCP server
    /// ([`library_tools`](crate::library_tools)), or `None` when the `innerword` tool is not
    /// there: the chat then works on the open document alone.
    pub fn side(backend: Arc<dyn Backend>, tools: Option<ToolServer>) -> Self {
        ChatSession::new(backend, Mode::Side).with_tools(tools)
    }

    /// The same chat continuing a stored transcript. The agent is told what was said
    /// when the next message is sent.
    pub fn with_transcript(mut self, mut transcript: Transcript) -> Self {
        transcript.settle();
        self.transcript = transcript;
        self
    }

    /// The same chat with this model and effort (what the settings remember for the
    /// mode).
    pub fn with_selection(mut self, selection: Selection) -> Self {
        self.selection = selection;
        self
    }

    /// The same chat with these tools.
    pub fn with_tools(mut self, tools: Option<ToolServer>) -> Self {
        self.set_tools(tools);
        self
    }

    /// The same chat with the agent running in this directory instead of the backend's
    /// neutral one.
    pub fn with_working_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.working_dir = Some(dir.into());
        self
    }

    /// The same chat with another system prompt than the mode's own.
    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    /// The same chat offering the drawings of `target` (its system prompt follows).
    pub fn with_drawing_target(mut self, target: DrawingTarget) -> Self {
        self.set_drawing_target(target);
        self
    }

    // ----- reading ---------------------------------------------------------------------

    /// The backend the chat runs on.
    pub fn backend(&self) -> &Arc<dyn Backend> {
        &self.backend
    }

    /// The mode.
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// The conversation as it is to be shown and stored.
    pub fn transcript(&self) -> &Transcript {
        &self.transcript
    }

    /// The events of the chat. Read them and hand each to [`apply`](Self::apply); see
    /// the module documentation for the loop of a GPUI view. Every call returns the same
    /// stream.
    pub fn events(&self) -> EventStream {
        self.stream.clone()
    }

    /// Whether an answer is being written (show Stop instead of Send).
    pub fn is_answering(&self) -> bool {
        self.transcript.is_answering()
    }

    /// The model and effort the next message is sent with.
    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    /// The model that answers, as the agent names it, once it has said.
    pub fn model_in_use(&self) -> Option<&str> {
        self.model_in_use.as_deref()
    }

    /// The last notice of the agent that is not the end of an answer ("the usage limit
    /// is close"), until [`dismiss_notice`](Self::dismiss_notice).
    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    /// Forgets the notice.
    pub fn dismiss_notice(&mut self) {
        self.notice = None;
    }

    /// The library items attached to the conversation.
    pub fn attachments(&self) -> &[Attachment] {
        &self.transcript.attachments
    }

    /// What the chat offers ([`DrawingTarget::None`] unless told).
    pub fn drawing_target(&self) -> DrawingTarget {
        self.drawing_target
    }

    /// The target the system prompt is made for: the chat's own in a side chat that has
    /// the mode's prompt; none in a quick chat or with a prompt of the caller's.
    fn prompt_target(&self) -> DrawingTarget {
        if self.mode == Mode::Side && self.system_prompt.is_none() {
            self.drawing_target
        } else {
            DrawingTarget::None
        }
    }

    /// Whether the chat has the tools of the library.
    pub fn has_tools(&self) -> bool {
        self.tools.is_some()
    }

    // ----- changing --------------------------------------------------------------------

    /// Chooses the model and effort for the messages from now on. An answer that is
    /// being written goes on as it was asked. The conversation continues: an agent that
    /// can change them while it runs does (it keeps everything it knows); for another
    /// the agent is started anew with the next message and told what was said so far.
    pub fn set_selection(&mut self, selection: Selection) {
        if self.selection == selection {
            return;
        }
        self.selection = selection;
        if let Some(running) = &mut self.running
            && running.selection != self.selection
            && running.conversation.select(self.selection.clone())
        {
            running.selection = self.selection.clone();
        }
    }

    /// Gives the chat other tools (or none), from the next message on.
    pub fn set_tools(&mut self, tools: Option<ToolServer>) {
        let tools = tools.map(|mut tools| {
            if tools.name == LIBRARY_SERVER_NAME {
                tools.describe = Some(library_describer(&self.names));
                tools.observe = Some(library_observer(&self.names));
            }
            tools
        });
        if self.tools != tools {
            self.tools = tools;
            // The next message starts a conversation that has them.
            self.retire_running();
        }
    }

    /// Offers the drawings of `target` from the next message on: a running conversation
    /// started for another target is retired, and the next message starts one with the
    /// conversation so far, as a change of tools does.
    pub fn set_drawing_target(&mut self, target: DrawingTarget) {
        if self.drawing_target == target {
            return;
        }
        self.drawing_target = target;
        let target = self.prompt_target();
        if self
            .running
            .as_ref()
            .is_some_and(|running| running.target != target)
        {
            self.retire_running();
        }
    }

    /// Records that answer `id` was drawn at `at` (milliseconds since 1970):
    /// [`Message::drawn_at`](crate::Message::drawn_at). Returns whether there was such an
    /// answer.
    pub fn mark_drawn(&mut self, id: MessageId, at: i64) -> bool {
        self.transcript.mark_drawn(id, at)
    }

    /// Attaches a library item. Returns `false` when it was attached already or the
    /// conversation has as many as it takes.
    pub fn attach(&mut self, attachment: Attachment) -> bool {
        self.names
            .learn(attachment.reference(), attachment.title.clone());
        self.transcript.attach(attachment)
    }

    /// Removes an attached item. Returns whether it was attached.
    pub fn detach(&mut self, kind: AttachmentKind, id: i64) -> bool {
        self.transcript.detach(kind, id)
    }

    /// Starts the agent's program now, without sending it anything, so that the first
    /// answer does not also wait for the program to start (about a second). Nothing of
    /// the user's text leaves the Mac before the first message. Optional.
    pub fn prepare(&mut self) {
        self.ensure_running();
        if let Some(running) = &self.running {
            running.conversation.prepare();
        }
    }

    /// Sends a question of the quick chat: `selection` is the selected text it is about
    /// (`None` or empty: the question alone, as typed). Returns the id of the user
    /// message; the answer is the message after it.
    pub fn ask_quick(&mut self, question: &str, selection: Option<&str>) -> MessageId {
        let selection = selection.filter(|selection| !selection.trim().is_empty());
        self.ensure_running();
        let prompt = quick_prompt(question, selection);
        self.dispatch(question, selection.map(str::to_string), prompt)
    }

    /// Sends a question of the side chat with its context: the open document as it is
    /// now (sent only when it changed since the last message of this conversation) and
    /// the attached items. Returns the id of the user message.
    pub fn ask_side(&mut self, question: &str, document: Option<&OpenDocument>) -> MessageId {
        // A new process knows nothing of what was sent: `ensure_running` resets the
        // context first, so the document goes along again.
        self.ensure_running();
        if let Some(document) = document
            && let Some(reference) = document.reference()
        {
            self.names.learn(reference, document.title.clone());
        }
        for attachment in &self.transcript.attachments {
            self.names
                .learn(attachment.reference(), attachment.title.clone());
        }
        let prompt = self.side.compose(
            question,
            document,
            &self.transcript.attachments,
            self.tools.is_some(),
        );
        self.dispatch(question, None, prompt)
    }

    /// Sends `prompt` to the agent as it is and shows `shown` as the user's message.
    /// For callers that assemble the context themselves.
    pub fn ask_with(&mut self, shown: &str, prompt: impl Into<String>) -> MessageId {
        self.ensure_running();
        self.dispatch(shown, None, prompt.into())
    }

    fn dispatch(&mut self, shown: &str, quote: Option<String>, prompt: String) -> MessageId {
        let message = self.transcript.push_user(shown.trim(), quote);
        let running = self
            .running
            .as_ref()
            .expect("ensure_running leaves a conversation");
        let turn = running.conversation.send(prompt);
        self.open_turns.insert(turn, running.generation);
        self.transcript
            .begin_answer(turn, running.selection.clone());
        message
    }

    /// Makes sure a conversation is running that fits the chat as it is now: alive, with
    /// the chosen model and effort. A new one is told what was said so far.
    fn ensure_running(&mut self) {
        let target = self.prompt_target();
        let fits = self.running.as_ref().is_some_and(|running| {
            running.conversation.is_alive()
                && running.selection == self.selection
                && running.target == target
        });
        if fits {
            return;
        }
        self.retire_running();
        let system_prompt = self
            .system_prompt
            .clone()
            .unwrap_or_else(|| match self.mode {
                Mode::Quick => prompts::QUICK_SYSTEM_PROMPT.to_string(),
                Mode::Side => prompts::side_system_prompt(self.tools.is_some(), target),
            });
        let mut options = ConversationOptions::new(system_prompt)
            .with_selection(self.selection.clone())
            .with_tools(self.tools.clone())
            .with_history(self.transcript.history(HISTORY_LIMIT));
        options.working_dir = self.working_dir.clone();
        self.generation += 1;
        self.side.reset();
        self.running = Some(Running {
            conversation: self.backend.start(options, self.sink.clone()),
            selection: self.selection.clone(),
            generation: self.generation,
            target,
        });
    }

    /// Takes the running conversation out of use. It is closed at once unless an answer
    /// of it is still open; then it is kept until that answer has finished.
    fn retire_running(&mut self) {
        if let Some(running) = self.running.take() {
            let busy = self
                .open_turns
                .values()
                .any(|generation| *generation == running.generation);
            if busy && running.conversation.is_alive() {
                self.retiring.push(running);
            }
        }
    }

    /// Takes an event of the stream into the chat. Returns whether anything changed that
    /// is shown.
    pub fn apply(&mut self, event: AssistantEvent) -> bool {
        match &event {
            AssistantEvent::Ready { model } => {
                if model.is_some() && self.model_in_use != *model {
                    self.model_in_use = model.clone();
                    return true;
                }
                false
            }
            AssistantEvent::Notice { text } => {
                self.notice = Some(text.clone());
                true
            }
            AssistantEvent::Finished { turn, .. } => {
                let changed = self.transcript.apply(&event);
                if let Some(generation) = self.open_turns.remove(turn) {
                    let still_busy = self.open_turns.values().any(|open| *open == generation);
                    if !still_busy {
                        self.retiring
                            .retain(|retired| retired.generation != generation);
                    }
                }
                changed
            }
            _ => self.transcript.apply(&event),
        }
    }

    /// Stops the answer that is being written; what had arrived stays. The conversation
    /// goes on.
    pub fn stop(&mut self) {
        for retired in &self.retiring {
            retired.conversation.stop();
        }
        if let Some(running) = &self.running {
            running.conversation.stop();
        }
    }

    /// Clears the conversation: the messages go, the attached items stay, and the next
    /// message starts afresh with the agent.
    pub fn clear(&mut self) {
        self.end_conversations();
        self.transcript.clear_messages();
    }

    /// A new conversation: the messages and the attached items go.
    pub fn start_new(&mut self) {
        self.end_conversations();
        self.transcript.reset();
    }

    /// Replaces the transcript with a stored one (the side chat when another document
    /// is opened): the running conversation ends, and the next message continues the
    /// stored one.
    pub fn replace_transcript(&mut self, mut transcript: Transcript) -> Transcript {
        self.end_conversations();
        transcript.settle();
        for attachment in &transcript.attachments {
            self.names
                .learn(attachment.reference(), attachment.title.clone());
        }
        std::mem::replace(&mut self.transcript, transcript)
    }

    /// Ends the agent's process; the transcript stays. An answer that was being written
    /// is stopped. The next message starts the agent again and continues.
    pub fn close(&mut self) {
        self.end_conversations();
    }

    fn end_conversations(&mut self) {
        // Dropping a conversation closes it; the events of its open turns that still
        // arrive find no turn in the transcript, which was settled here.
        self.running = None;
        self.retiring.clear();
        self.open_turns.clear();
        self.transcript.settle();
        self.side.reset();
        self.notice = None;
    }
}

impl std::fmt::Debug for ChatSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatSession")
            .field("mode", &self.mode)
            .field("messages", &self.transcript.len())
            .field("selection", &self.selection)
            .field("running", &self.running.is_some())
            .finish()
    }
}

/// What the user is told once, before the first message is sent: where the text goes
/// and what it costs. `info` is the backend's.
pub fn privacy_notice(info: &crate::backend::BackendInfo, mode: Mode) -> String {
    let what = match mode {
        Mode::Quick => "Your question and the text you have selected are",
        Mode::Side => {
            "Your messages, the open document and what the assistant reads from your library are"
        }
    };
    format!(
        "{what} sent to {vendor} through your {name} sign-in, and each message counts \
         against your {name} plan. Nothing is sent before you send a message.",
        vendor = info.vendor,
        name = info.name,
    )
}
