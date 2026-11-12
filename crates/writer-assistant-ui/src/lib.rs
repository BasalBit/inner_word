//! The assistant inside The Writer, as it is seen: the shared chat
//! components and the side chat panel. The engine is `writer-assistant`
//! (no GPUI); the quick chat popover is `writer-assistant-quick`, built on
//! the components here.
//!
//! # The shared components
//!
//! | What | Where |
//! |---|---|
//! | The conversation: the user's messages, the answers as formatted text while they are written, the line that says what the assistant is doing, stopped and failed answers, a list that follows the answer | [`MessageList`], fed with the engine's `Transcript::messages` |
//! | Where a message is written: grows to a limit, `enter` sends, `shift-enter` is a new line, `escape` hands the keyboard back, Stop in place of Send | [`Composer`] |
//! | The model and effort pickers: small controls that show the choice and open a menu of the engine's catalog | [`picker_button`], [`open_picker_menu`], [`PickerKind`], [`ControlBounds`] |
//! | The actions of an answer, which the host supplies and carries out | [`AnswerAction`], [`MessageListEvent::ActionInvoked`] |
//! | Errors and notices with one action each | [`Notice`], [`notice_row`] |
//! | Small buttons for the pointer | [`controls`] |
//! | Markdown to the document an answer shows | [`markdown`] |
//!
//! A chat owns a `writer_assistant::ChatSession` and puts the pieces
//! together (this is all the quick chat needs; the side chat is
//! [`AssistantPanel`]):
//!
//! ```ignore
//! // Once at startup, after writer_ui::init, writer_editor::init and the
//! // editor tools (the keys of the chat's editors must be added later than
//! // the editor's own):
//! writer_assistant_ui::init(cx);
//!
//! struct QuickChat {
//!     session: ChatSession,
//!     messages: Entity<MessageList>,
//!     composer: Entity<Composer>,
//!     model_bounds: ControlBounds,
//!     effort_bounds: ControlBounds,
//! }
//!
//! // Making it:
//! let keys = ChatKeys::new("assistant_quick");   // for `Editor && assistant_quick` bindings
//! let messages = cx.new(|cx| {
//!     MessageList::new(
//!         MessageListOptions::new(backend.info()).style(AnswerStyle::small()).keys(keys.clone()),
//!         window, cx,
//!     )
//! });
//! messages.update(cx, |list, cx| list.set_actions(vec![
//!     AnswerAction::new("replace", "Replace").tooltip("Replace Selection").key_of(ReplaceWithAnswer),
//!     AnswerAction::new("insert", "Insert Below").key_of(InsertAnswerBelow),
//!     AnswerAction::new("copy", "Copy").key_of(CopyQuickAnswer),
//! ], cx));
//! let composer = cx.new(|cx| Composer::new(ComposerOptions::new("Ask about the selection").keys(keys), window, cx));
//! cx.subscribe_in(&composer, window, |this, _, event: &ComposerEvent, window, cx| match event {
//!     ComposerEvent::Submitted(text) => {
//!         this.session.ask_quick(text, this.selected_text.as_deref());
//!         this.sync(window, cx);
//!     }
//!     ComposerEvent::StopRequested => this.session.stop(),
//!     ComposerEvent::Dismissed => this.close(window, cx),
//!     ComposerEvent::Changed => {}
//! }).detach();
//! cx.subscribe_in(&messages, window, |this, _, event: &MessageListEvent, window, cx| match event {
//!     MessageListEvent::ActionInvoked { action, message } => this.apply(action, *message, window, cx),
//!     MessageListEvent::CopyRequested { message } => this.copy(*message, cx),
//!     MessageListEvent::RetryRequested { message } => this.ask_again(*message, window, cx),
//!     MessageListEvent::ExampleChosen(text) => this.composer.update(cx, |c, cx| c.set_text(text, cx)),
//!     MessageListEvent::Dismissed => this.close(window, cx),
//! }).detach();
//!
//! // The events of the session, a batch at a time:
//! let events = session.events();
//! cx.spawn_in(window, async move |this, cx| {
//!     while let Some(batch) = events.next_batch().await {
//!         let alive = this.update_in(cx, |this, window, cx| {
//!             let mut changed = false;
//!             for event in batch {
//!                 changed |= this.session.apply(event);
//!             }
//!             if changed {
//!                 this.sync(window, cx);
//!             }
//!         });
//!         if alive.is_err() {
//!             break;
//!         }
//!     }
//! }).detach();
//!
//! // After every change of the session:
//! fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
//!     let transcript = self.session.transcript();
//!     self.messages.update(cx, |list, cx| list.set_messages(&transcript.messages, window, cx));
//!     let busy = self.session.is_answering();
//!     self.composer.update(cx, |composer, cx| composer.set_busy(busy, cx));
//!     cx.notify();
//! }
//!
//! // Drawing it (a window needs a writer_ui::ToastLayer or ContextMenuLayer
//! // for the menus):
//! div()
//!     .on_action(cx.listener(|this, _: &FocusNextArea, window, cx| this.cycle_focus(true, window, cx)))
//!     .on_action(cx.listener(|this, _: &FocusPreviousArea, window, cx| this.cycle_focus(false, window, cx)))
//!     .on_action(cx.listener(|this, _: &StopAnswer, _, _| this.session.stop()))
//!     .child(div().flex_1().min_h_0().child(self.messages.clone()))
//!     .child(self.composer.clone())
//!     .child(
//!         picker_button(PickerKind::Model, &catalog, self.session.selection(), &self.model_bounds, false, &colors, cx)
//!             .on_click(cx.listener(|this, _, window, cx| this.open_picker(PickerKind::Model, false, window, cx))),
//!     )
//! ```
//!
//! # Keys the components bring
//!
//! [`init`] binds them; see [`actions`] for the contexts. In an editor of a
//! chat (the composer, the text of an answer): `escape` is [`DismissChat`],
//! `tab` / `shift-tab` are [`FocusNextArea`] / [`FocusPreviousArea`] (the
//! host moves the keyboard: handle the two actions on an ancestor),
//! `cmd-.` is [`StopAnswer`]. In the message list: the arrows, `home`,
//! `end`, `enter` and `ctrl-enter` (the menu), `cmd-c`.

#![warn(missing_docs)]

pub mod actions;
pub mod composer;
pub mod controls;
pub mod markdown;
pub mod messages;
pub mod notice;
pub mod pickers;

pub use actions::{
    ANSWER_EDITOR_FLAG, ActivateSelected, AddContext, CATEGORY, CHAT_EDITOR_FLAG, COMPOSER_CONTEXT,
    COMPOSER_EDITOR_FLAG, CONTEXT_ROW_CONTEXT, ChooseEffort, ChooseModel, ClearChat,
    CloseAssistant, CopyAnswer, CopyMessage, DismissChat, FocusComposer, FocusNextArea,
    FocusPreviousArea, HEADER_CONTEXT, InsertAtCaret, MESSAGES_CONTEXT, NewChat, NextControl,
    PANEL_CONTEXT, PressControl, PreviousControl, RemoveContext, ReplaceSelection,
    SIDE_EDITOR_FLAG, SelectFirstMessage, SelectLastMessage, SelectNextMessage,
    SelectPreviousMessage, StopAnswer, commands, key_bindings,
};
pub use composer::{COMPOSER_MAX_HEIGHT, Composer, ComposerEvent, ComposerOptions};
pub use markdown::{AnswerFont, AnswerStyle, answer_document, answer_plain_text};
pub use messages::{AnswerAction, EmptyState, MessageList, MessageListEvent, MessageListOptions};
pub use notice::{Notice, NoticeKind, NoticeRow, notice_row};
pub use pickers::{
    ControlBounds, PickerKind, open_picker_menu, picker_button, picker_has_choices, picker_label,
    picker_menu_items,
};

use gpui::{App, SharedString};

/// The key context flags a host gives the editors of its chat (the
/// composer and the answers), so that it can bind keys that win over the
/// editor's own: `KeyBinding::new("cmd-enter", MyAction, Some("Editor &&
/// my_flag"))`, added after `writer_editor::init`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChatKeys {
    /// A flag added to the key context of every editor of the chat.
    pub editor_flag: Option<SharedString>,
}

impl ChatKeys {
    /// Keys with this flag on the editors of the chat.
    pub fn new(editor_flag: impl Into<SharedString>) -> Self {
        Self {
            editor_flag: Some(editor_flag.into()),
        }
    }
}

/// Installs the key bindings of the chat components and of the side chat.
///
/// Call once at startup, after `writer_ui::init`, `writer_editor::init`
/// and the editor tools: the keys of the chat's editors (`escape`, `tab`,
/// `cmd-.`, `cmd-enter`) are the editor's own otherwise, and of two
/// bindings at the same depth the later one wins.
pub fn init(cx: &mut App) {
    cx.bind_keys(actions::key_bindings());
}
