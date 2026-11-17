//! The quick chat of Inner Word's assistant: a small popover at the
//! selection or the caret, for a word, a sentence or a paragraph.
//!
//! The engine is `innerword-assistant` (no GPUI, nothing in it assumes a
//! particular agent); the message list, the composer, the pickers and the
//! notices are the shared components of `innerword-assistant-ui`. This crate
//! is the popover around them and what it does to the text.
//!
//! # What it is
//!
//! - It opens at the selection, or at the caret when nothing is selected,
//!   and stays inside the editor's viewport, beside the text it was opened
//!   on, while the text scrolls ([`place`]).
//! - What the assistant is given is exactly the selected text, captured
//!   when the popover opens ([`Capture`]), and what the user types. With
//!   no selection it is the question alone. The document, its title and
//!   the library are never sent, and the quick chat has no tools.
//! - The keyboard is in the composer as soon as it opens. `enter` sends,
//!   the answer streams in above the composer, and the conversation goes
//!   on in the popover, which grows to a limit and then scrolls.
//! - Each answer has Replace Selection (the captured text, wherever the
//!   caret is now; not offered once that text has changed), Insert Below
//!   and Copy ([`apply`], [`QuickAction`]). After Replace or Insert the
//!   popover closes and the text that changed is selected; one `cmd-z`
//!   takes it back.
//! - `escape` stops an answer that is being written, and closes the
//!   popover otherwise. A press outside closes it, and so does the
//!   keyboard going anywhere else in the window. Closing ends the
//!   conversation; opening again starts a fresh one.
//!
//! # Mounting it (the app)
//!
//! ```ignore
//! // Once at startup, after innerword_ui::init, innerword_editor::init, the
//! // editor tools and innerword_assistant_ui::init:
//! innerword_assistant_quick::init(cx);
//!
//! // Per editor, kept for as long as the editor lives. `backend` is the
//! // app's engine: an `Arc<dyn innerword_assistant::Backend>`.
//! let quick = cx.new(|cx| QuickChat::new(editor.clone(), backend.clone(), window, cx));
//! quick.update(cx, |quick, cx| {
//!     quick.set_selection(remembered_model_and_effort, cx);
//!     quick.set_privacy_notice(!notice_was_acknowledged, cx);
//! });
//! cx.subscribe_in(&quick, window, |this, _, event: &QuickChatEvent, window, cx| match event {
//!     QuickChatEvent::SelectionChanged(selection) => this.remember_quick_selection(selection, cx),
//!     QuickChatEvent::PrivacyNoticeAcknowledged => this.remember_notice_acknowledged(cx),
//!     QuickChatEvent::AvailabilityChanged(found) => this.assistant_found(found.clone(), cx),
//!     _ => {}
//! })
//! .detach();
//!
//! // In the workspace's render, anywhere AFTER the editor (it takes no
//! // room; it has to be prepainted after the editor to follow its
//! // scrolling in the same frame) and inside the element that tracks the
//! // workspace's focus:
//! root.child(editor_tools.clone()).child(quick.clone())
//!
//! // The app's own action for it (a key, the text menu, the format bar):
//! .on_action(cx.listener(|this, _: &AskAssistant, window, cx| {
//!     this.quick.update(cx, |quick, cx| quick.toggle(window, cx));
//! }))
//! ```
//!
//! The window needs a `innerword_ui::ToastLayer` (or `ContextMenuLayer`) for
//! the menus of the pickers, as every window of the app has.
//!
//! Opening takes the keyboard from the text, which is what closes the
//! format bar and any popup tool of `innerword-editor-tools` (they close when
//! the text loses the keyboard). Nothing here depends on that crate.
//!
//! # What it looks like and how it grows
//!
//! A header (the agent's name, the model and effort pickers, close), a
//! quote of the selection until the first question shows it, the
//! conversation, the notices (not signed in, where the text goes, what
//! the agent wants the user to know) and the composer. With nothing said
//! it is as small as its composer. The conversation is given the height
//! it needs, measured before each frame that changed it, until the
//! popover reaches its limit ([`place::FULL_HEIGHT`], or the room beside
//! the text when that is less); from then on the conversation scrolls
//! and follows the answer.
//!
//! # Tests and scripted runs
//!
//! [`QuickChat::describe`] is one line about the open popover.
//! [`QuickChat::ask`], [`QuickChat::apply_answer`], [`QuickChat::dismiss`]
//! and [`QuickChat::press_notice`] do what the composer, the buttons,
//! `escape` and a notice's button do. Debug selectors: `quick-chat` (the
//! popover), `quick-chat-header`, `quick-chat-quote`,
//! `quick-chat-conversation`, `quick-chat-close`, the notices
//! ([`QuickNotice::selector`], with `-action` for the button), and those
//! of the shared components (`assistant-model`, `assistant-effort`,
//! `assistant-composer`, `assistant-send`, `assistant-stop`,
//! `assistant-action-<replace|insert|copy>-<message id>`,
//! `assistant-failed-<message id>`).
//!
//! # Modules
//!
//! | Module | Contents |
//! |---|---|
//! | [`popover`] | [`QuickChat`]: the entity, its events, what it shows |
//! | [`actions`] | the keys while the keyboard is in the popover |
//! | [`capture`] | the selection as it was when the chat opened, and where it is now |
//! | [`apply`] | Replace Selection and Insert Below: an answer (Markdown) into the text, one undo step |
//! | [`place`] | where the popover goes: beside its text, inside the editor's viewport |

pub mod actions;
pub mod apply;
pub mod capture;
mod layer;
pub mod place;
pub mod popover;

pub use actions::{EDITOR_FLAG, InsertBelow, KEY_CONTEXT, key_bindings};
pub use capture::Capture;
pub use popover::{Drawn, QuickAction, QuickChat, QuickChatEvent, QuickNotice, ReplaceState};

/// Installs the key bindings of the popover.
///
/// Call once at startup, after `innerword_ui::init`, `innerword_editor::init`,
/// the editor tools and `innerword_assistant_ui::init`: the keys of the
/// chat's editors (`cmd-enter`, `cmd-shift-enter`) are the editor's own
/// otherwise, and of two bindings at the same depth the later one wins.
pub fn init(cx: &mut gpui::App) {
    cx.bind_keys(actions::key_bindings());
}
