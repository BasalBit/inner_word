//! The engine of the assistant inside The Writer.
//!
//! The assistant is an optional feature that runs on a terminal agent the user has
//! installed and signed in to (Claude Code first; others will follow). This crate has no
//! GPUI in it. It holds:
//!
//! * [`Backend`], the interface to a terminal agent, with nothing in it that assumes a
//!   particular one: availability, models and effort levels, conversations, events.
//! * [`ClaudeCode`], the backend for Claude Code: one long-lived `claude` process per
//!   conversation, isolated from the user's coding setup.
//! * [`FakeBackend`], a scripted backend without a process, for the tests of every
//!   crate.
//! * [`Transcript`], the conversation as it is shown and stored, and [`ChatSession`],
//!   which a view owns: transcript, conversation and the rules between them.
//! * The context of a message ([`quick_prompt`], [`side_prompt`], [`SideContext`]) and
//!   the system prompts ([`prompts`]).
//! * The tools of the side chat ([`library_tools`]): the app's own MCP server, read-only.
//!
//! # Driving a conversation
//!
//! A view owns a [`ChatSession`], sends questions and applies the events it reads from
//! the session's stream. Nothing here blocks the calling thread.
//!
//! ```
//! use std::sync::Arc;
//! use std::time::Duration;
//! use writer_assistant::{ChatSession, FakeBackend, MessageState, Role};
//!
//! let fake = FakeBackend::new();
//! fake.reply("exhausted, drained, *spent*");
//!
//! let mut chat = ChatSession::quick(Arc::new(fake.clone()));
//! let events = chat.events();
//! chat.ask_quick("Another word for this?", Some("very tired"));
//!
//! // A GPUI view awaits `events.next_batch()` instead (see `ChatSession`).
//! while chat.is_answering() {
//!     let event = events.wait(Duration::from_secs(5)).expect("the answer arrives");
//!     chat.apply(event);
//! }
//!
//! let messages = &chat.transcript().messages;
//! assert_eq!(messages[0].role, Role::User);
//! assert_eq!(messages[0].quote.as_deref(), Some("very tired"));
//! assert_eq!(messages[1].text, "exhausted, drained, *spent*");
//! assert_eq!(messages[1].state, MessageState::Done);
//! // The agent received the selection and the question, and nothing else.
//! assert!(fake.last_sent().unwrap().contains("<selected_text>\nvery tired\n</selected_text>"));
//! ```
//!
//! With the real agent the only difference is the backend:
//! `ChatSession::quick(Arc::new(ClaudeCode::new()))`.

pub mod backend;
pub mod claude;
pub mod context;
pub mod events;
pub mod fake;
pub mod prompts;
pub mod session;
pub mod time;
pub mod tools;
pub mod transcript;

pub use backend::{
    AssistantError, AssistantEvent, Availability, Backend, BackendInfo, Catalog, CatalogSource,
    Command, Conversation, ConversationOptions, EffortChoice, EffortOption, ErrorText,
    HistoryMessage, Installation, Mode, ModelChoice, ModelOption, Role, Selection, SignIn,
    ToolActivity, ToolDescriber, ToolServer, ToolState, TurnEnd, TurnId, Usage,
};
pub use claude::ClaudeCode;
pub use context::{
    AttachedPart, DocumentPart, HISTORY_LIMIT, OpenDocument, OutlineEntry, SELECTION_LIMIT,
    SideContext, WHOLE_DOCUMENT_LIMIT, bounded_history, outline, quick_prompt, side_prompt,
    with_history,
};
pub use events::{EventSink, EventStream, event_channel};
pub use fake::{FakeBackend, FakeReply, FakeSent, fake_catalog, fake_info};
pub use prompts::{QUICK_SYSTEM_PROMPT, SIDE_SYSTEM_PROMPT, side_system_prompt, system_prompt};
pub use session::{ChatSession, privacy_notice};
pub use tools::{
    LIBRARY_SERVER_NAME, NameBook, describe_library_tool, library_tools, library_tools_named,
    read_only_tools, withheld_tools,
};
pub use transcript::{
    Attachment, AttachmentKind, CUT_OFF_NOTE, Message, MessageId, MessageState, Transcript,
};
