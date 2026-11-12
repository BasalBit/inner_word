//! The assistant in the app: an optional writing assistant that runs on a
//! terminal agent the user has installed and signed in to (Claude Code
//! first).
//!
//! | Module | Contents |
//! |---|---|
//! | [`host`] | which engine the app uses, and whether it is installed and signed in (looked up in the background) |
//! | [`choices`] | the model and the effort each of the two chats asks with, as the settings keep them |
//! | [`conversations`] | the side chat's conversations, one per document, stored in the library |
//! | [`fake`] | the fake engine in a scripted run of the real app, for screenshots that cost nothing |
//! | [`apply`] | what the assistant reads from the text (the document as Markdown, the selection) and what the user applies to it (insert, replace, as one undo step) |

pub mod apply;
pub mod choices;
pub mod conversations;
pub mod fake;
pub mod host;

pub use host::{
    FAKE_ENV, Host, HostEvent, REAL_ENV, clear_backend, host, real_engine_requested, set_backend,
    use_claude_code,
};
