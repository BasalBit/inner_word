//! The conversation as the interface shows it and the app stores it.
//!
//! A [`Transcript`] is a list of [`Message`]s, each with its state, and the library items
//! attached to the conversation. It is plain data: it serialises to JSON
//! ([`Transcript::to_json`]), it is bounded in size ([`Transcript::MAX_MESSAGES`],
//! [`Transcript::MAX_CHARS`]), and it changes only through its own methods: a question is
//! added with [`Transcript::push_user`], the answer is opened with
//! [`Transcript::begin_answer`] and filled by [`Transcript::apply`] from the events of the
//! conversation. [`ChatSession`](crate::ChatSession) does all three; a view only reads.

use serde::{Deserialize, Serialize};

use crate::backend::{
    AssistantError, AssistantEvent, HistoryMessage, Role, Selection, ToolActivity, ToolState,
    TurnEnd, TurnId, Usage,
};
use crate::time::now_millis;

/// Names a message within its transcript.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct MessageId(pub u64);

/// Where a message stands.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageState {
    /// The answer is being written (or is awaited: the text may still be empty).
    Streaming,
    /// Complete. Every user message is in this state.
    Done,
    /// The user stopped the answer; the text is what had arrived.
    Stopped,
    /// The answer failed; the text is what had arrived, often nothing.
    Failed(AssistantError),
}

/// A document or a folder of the library.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentKind {
    /// A document.
    Document,
    /// A folder with everything in it.
    Folder,
}

/// A library document or folder the user added to a conversation as context. The
/// assistant is told its id and path and reads it through the tools.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Attachment {
    /// A document or a folder.
    pub kind: AttachmentKind,
    /// Its id in the library.
    pub id: i64,
    /// Its title or name, for the chip.
    pub title: String,
    /// Its path in the library: "The Salt Road/Part one/Chapter 1".
    pub path: String,
}

impl Attachment {
    /// A document of the library.
    pub fn document(id: i64, title: impl Into<String>, path: impl Into<String>) -> Self {
        Attachment {
            kind: AttachmentKind::Document,
            id,
            title: title.into(),
            path: path.into(),
        }
    }

    /// A folder of the library.
    pub fn folder(id: i64, title: impl Into<String>, path: impl Into<String>) -> Self {
        Attachment {
            kind: AttachmentKind::Folder,
            id,
            title: title.into(),
            path: path.into(),
        }
    }

    /// The id as the tools of the library take it: `doc:12`, `folder:3`.
    pub fn reference(&self) -> String {
        match self.kind {
            AttachmentKind::Document => format!("doc:{}", self.id),
            AttachmentKind::Folder => format!("folder:{}", self.id),
        }
    }

    /// Whether `other` is the same item (the title and the path may have changed).
    pub fn is_same_item(&self, other: &Attachment) -> bool {
        self.kind == other.kind && self.id == other.id
    }
}

/// One message of a conversation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Message {
    /// Its name in the transcript.
    pub id: MessageId,
    /// Who it is from.
    pub role: Role,
    /// A user message: what the user typed. An answer: Markdown.
    pub text: String,
    /// Where it stands.
    pub state: MessageState,
    /// A user message of the quick chat: the selected text the question is about.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote: Option<String>,
    /// A user message of the side chat: what was attached when it was sent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
    /// An answer: the model and effort it was asked with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<Selection>,
    /// An answer: the tools the assistant used for it, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub activity: Vec<ToolActivity>,
    /// An answer: what it cost, when the agent said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    /// When it was sent or begun, in milliseconds since 1970.
    #[serde(default)]
    pub at: i64,
    /// An answer in this run of the app: the turn whose events fill it.
    #[serde(skip)]
    pub turn: Option<TurnId>,
    /// An answer that is being written: what is happening while no text arrives
    /// ("Thinking", "No connection, trying again"), as the agent last said.
    #[serde(skip)]
    pub status: Option<String>,
}

impl Message {
    /// Whether the message is an answer that is still being written.
    pub fn is_streaming(&self) -> bool {
        self.state == MessageState::Streaming
    }

    /// Whether this is an answer with text the user can apply (insert, replace, copy):
    /// an answer that is not being written any more and is not empty.
    pub fn is_applicable(&self) -> bool {
        self.role == Role::Assistant && !self.is_streaming() && !self.text.trim().is_empty()
    }

    /// The error of a failed answer.
    pub fn error(&self) -> Option<&AssistantError> {
        match &self.state {
            MessageState::Failed(error) => Some(error),
            _ => None,
        }
    }

    /// What the assistant is doing right now, for the line under a streaming answer:
    /// the description of the tool call that is running ("Reading Chapter 2"), else
    /// what the agent last said of itself ("Thinking"), else nothing.
    pub fn current_activity(&self) -> Option<&str> {
        if !self.is_streaming() {
            return None;
        }
        self.activity
            .iter()
            .rev()
            .find(|activity| activity.state == ToolState::Running)
            .map(|activity| activity.description.as_str())
            .or(self.status.as_deref())
    }
}

/// A conversation: its messages and what is attached to it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Transcript {
    /// The messages, oldest first.
    #[serde(default)]
    pub messages: Vec<Message>,
    /// The library items attached to the conversation (the chips of the side chat).
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    /// How many of the oldest messages were dropped to keep the transcript bounded.
    #[serde(default)]
    pub dropped: usize,
    #[serde(default)]
    next_id: u64,
}

impl Transcript {
    /// The most messages a transcript keeps; the oldest go first.
    pub const MAX_MESSAGES: usize = 200;
    /// The most characters of message text a transcript keeps; the oldest messages go
    /// first (the last question and its answer always stay).
    pub const MAX_CHARS: usize = 400_000;
    /// The most characters one answer keeps: what arrives beyond is cut off and the
    /// answer says so. A runaway answer cannot fill the memory.
    pub const MAX_ANSWER_CHARS: usize = 200_000;
    /// The most attachments of a conversation.
    pub const MAX_ATTACHMENTS: usize = 32;

    /// An empty conversation.
    pub fn new() -> Self {
        Transcript::default()
    }

    /// Whether nothing was said yet.
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// The number of messages.
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    /// The message with this id.
    pub fn message(&self, id: MessageId) -> Option<&Message> {
        self.messages.iter().find(|message| message.id == id)
    }

    /// The answer that belongs to this turn.
    pub fn message_for_turn(&self, turn: TurnId) -> Option<&Message> {
        self.messages
            .iter()
            .rev()
            .find(|message| message.turn == Some(turn))
    }

    /// The last answer, whatever its state.
    pub fn last_answer(&self) -> Option<&Message> {
        self.messages
            .iter()
            .rev()
            .find(|message| message.role == Role::Assistant)
    }

    /// The question an answer answers: the nearest user message before it.
    pub fn question_of(&self, answer: MessageId) -> Option<&Message> {
        let index = self
            .messages
            .iter()
            .position(|message| message.id == answer)?;
        self.messages[..index]
            .iter()
            .rev()
            .find(|message| message.role == Role::User)
    }

    /// Whether an answer is being written.
    pub fn is_answering(&self) -> bool {
        self.messages.iter().any(Message::is_streaming)
    }

    fn next_id(&mut self) -> MessageId {
        self.next_id += 1;
        MessageId(self.next_id)
    }

    /// Adds a question. `quote` is the selected text it is about (the quick chat). The
    /// attachments of the conversation are noted on the message.
    pub fn push_user(&mut self, text: impl Into<String>, quote: Option<String>) -> MessageId {
        let id = self.next_id();
        self.messages.push(Message {
            id,
            role: Role::User,
            text: text.into(),
            state: MessageState::Done,
            quote,
            attachments: self.attachments.clone(),
            selection: None,
            activity: Vec::new(),
            usage: None,
            at: now_millis(),
            turn: None,
            status: None,
        });
        self.trim();
        id
    }

    /// Opens the answer of `turn`: an empty message in the state
    /// [`MessageState::Streaming`], which [`apply`](Self::apply) fills.
    pub fn begin_answer(&mut self, turn: TurnId, selection: Selection) -> MessageId {
        let id = self.next_id();
        self.messages.push(Message {
            id,
            role: Role::Assistant,
            text: String::new(),
            state: MessageState::Streaming,
            quote: None,
            attachments: Vec::new(),
            selection: Some(selection),
            activity: Vec::new(),
            usage: None,
            at: now_millis(),
            turn: Some(turn),
            status: None,
        });
        self.trim();
        id
    }

    /// Takes an event of the conversation into the transcript. Returns whether anything
    /// changed (and so whether to redraw). Events of turns the transcript does not know
    /// (a conversation that was cleared, an answer that already finished) change nothing.
    pub fn apply(&mut self, event: &AssistantEvent) -> bool {
        let Some(turn) = event.turn() else {
            return false;
        };
        let Some(message) = self
            .messages
            .iter_mut()
            .rev()
            .find(|message| message.turn == Some(turn))
        else {
            return false;
        };
        if !message.is_streaming() {
            return false;
        }
        match event {
            AssistantEvent::Text { text, .. } => {
                let cleared = message.status.take().is_some();
                push_bounded(&mut message.text, text) || cleared
            }
            AssistantEvent::Status { text, .. } => {
                let status = (!text.trim().is_empty()).then(|| text.clone());
                if message.status == status {
                    return false;
                }
                message.status = status;
                true
            }
            AssistantEvent::Tool { activity, .. } => {
                message.status = None;
                match message
                    .activity
                    .iter_mut()
                    .find(|known| known.id == activity.id)
                {
                    Some(known) => {
                        if known == activity {
                            return false;
                        }
                        known.state = activity.state;
                        if !activity.description.is_empty() {
                            known.description = activity.description.clone();
                        }
                    }
                    None => message.activity.push(activity.clone()),
                }
                true
            }
            AssistantEvent::Finished {
                text, end, usage, ..
            } => {
                // The agent's final text is the answer when nothing (or less) arrived in
                // pieces; pieces that arrived are never thrown away for an empty final.
                if let Some(text) = text
                    && text.trim().len() > message.text.trim().len()
                    && *end == TurnEnd::Complete
                {
                    message.text.clear();
                    push_bounded(&mut message.text, text);
                }
                for activity in &mut message.activity {
                    if activity.state == ToolState::Running {
                        activity.state = match end {
                            TurnEnd::Complete => ToolState::Done,
                            _ => ToolState::Failed,
                        };
                    }
                }
                message.usage = usage.clone();
                message.status = None;
                message.state = match end {
                    TurnEnd::Complete => MessageState::Done,
                    TurnEnd::Stopped => MessageState::Stopped,
                    TurnEnd::Failed(error) => MessageState::Failed(error.clone()),
                };
                self.trim();
                true
            }
            _ => false,
        }
    }

    /// Ends every answer that is still open as stopped: after a transcript was loaded
    /// (the app was quit in the middle of an answer), or when its conversation is closed.
    pub fn settle(&mut self) -> bool {
        let mut changed = false;
        for message in &mut self.messages {
            if message.is_streaming() {
                message.state = MessageState::Stopped;
                message.status = None;
                for activity in &mut message.activity {
                    if activity.state == ToolState::Running {
                        activity.state = ToolState::Failed;
                    }
                }
                changed = true;
            }
            message.turn = None;
        }
        changed
    }

    /// The conversation so far as a new conversation is told it
    /// ([`ConversationOptions::with_history`](crate::ConversationOptions::with_history)):
    /// questions and the answers that have text, without answers still being written.
    /// Bounded: the most recent messages within `max_chars` characters.
    pub fn history(&self, max_chars: usize) -> Vec<HistoryMessage> {
        let mut history = Vec::new();
        let mut used = 0usize;
        for message in self.messages.iter().rev() {
            if message.is_streaming() {
                continue;
            }
            let text = match (&message.role, &message.quote) {
                (Role::User, Some(quote)) if !quote.trim().is_empty() => {
                    crate::context::quick_prompt(&message.text, Some(quote))
                }
                _ => message.text.clone(),
            };
            if text.trim().is_empty() {
                continue;
            }
            let size = text.chars().count();
            if used + size > max_chars && !history.is_empty() {
                break;
            }
            used += size;
            history.push(HistoryMessage {
                role: message.role,
                text,
            });
        }
        history.reverse();
        // A history begins with a question.
        while history
            .first()
            .is_some_and(|message| message.role == Role::Assistant)
        {
            history.remove(0);
        }
        history
    }

    /// Removes every message; the attachments stay.
    pub fn clear_messages(&mut self) {
        self.messages.clear();
        self.dropped = 0;
    }

    /// Removes everything: a new conversation.
    pub fn reset(&mut self) {
        self.messages.clear();
        self.attachments.clear();
        self.dropped = 0;
    }

    /// Attaches a library item. Returns `false` when it is attached already (its title
    /// and path are brought up to date) or the conversation has
    /// [`MAX_ATTACHMENTS`](Self::MAX_ATTACHMENTS).
    pub fn attach(&mut self, attachment: Attachment) -> bool {
        if let Some(known) = self
            .attachments
            .iter_mut()
            .find(|known| known.is_same_item(&attachment))
        {
            *known = attachment;
            return false;
        }
        if self.attachments.len() >= Self::MAX_ATTACHMENTS {
            return false;
        }
        self.attachments.push(attachment);
        true
    }

    /// Removes an attached item. Returns whether it was attached.
    pub fn detach(&mut self, kind: AttachmentKind, id: i64) -> bool {
        let before = self.attachments.len();
        self.attachments
            .retain(|attachment| !(attachment.kind == kind && attachment.id == id));
        self.attachments.len() != before
    }

    /// The characters of all message texts.
    pub fn char_count(&self) -> usize {
        self.messages
            .iter()
            .map(|message| {
                message.text.chars().count()
                    + message.quote.as_ref().map_or(0, |q| q.chars().count())
            })
            .sum()
    }

    /// Drops the oldest messages until the transcript is within its bounds. The last
    /// question and what follows it always stay.
    fn trim(&mut self) {
        let keep_from = self
            .messages
            .iter()
            .rposition(|message| message.role == Role::User)
            .unwrap_or(0);
        let mut chars = self.char_count();
        let mut drop = 0usize;
        while drop < keep_from
            && (self.messages.len() - drop > Self::MAX_MESSAGES || chars > Self::MAX_CHARS)
        {
            let message = &self.messages[drop];
            chars = chars.saturating_sub(
                message.text.chars().count()
                    + message.quote.as_ref().map_or(0, |q| q.chars().count()),
            );
            drop += 1;
        }
        // Never begin with an answer whose question is gone.
        while drop < keep_from && self.messages[drop].role == Role::Assistant {
            drop += 1;
        }
        if drop > 0 {
            self.messages.drain(..drop);
            self.dropped += drop;
        }
    }

    /// The transcript as JSON, for the app to store.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string())
    }

    /// A stored transcript. Answers that were being written when it was stored come back
    /// as stopped; the bounds are applied.
    pub fn from_json(json: &str) -> Result<Transcript, serde_json::Error> {
        let mut transcript: Transcript = serde_json::from_str(json)?;
        let highest = transcript
            .messages
            .iter()
            .map(|message| message.id.0)
            .max()
            .unwrap_or(0);
        transcript.next_id = transcript.next_id.max(highest);
        transcript.attachments.truncate(Self::MAX_ATTACHMENTS);
        transcript.settle();
        transcript.trim();
        Ok(transcript)
    }
}

/// The note that ends an answer that was cut off at
/// [`Transcript::MAX_ANSWER_CHARS`].
pub const CUT_OFF_NOTE: &str = "\n\n*(The answer was cut off here: it is too long to show.)*";

/// Appends `piece` to `text` up to the bound of one answer. Returns whether anything
/// was appended.
fn push_bounded(text: &mut String, piece: &str) -> bool {
    if piece.is_empty() || text.ends_with(CUT_OFF_NOTE) {
        return false;
    }
    let have = text.chars().count();
    let room = Transcript::MAX_ANSWER_CHARS.saturating_sub(have);
    if piece.chars().count() <= room {
        text.push_str(piece);
    } else {
        text.extend(piece.chars().take(room));
        text.push_str(CUT_OFF_NOTE);
    }
    true
}
