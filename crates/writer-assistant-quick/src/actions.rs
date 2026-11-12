//! The keys of the quick chat.
//!
//! The popover is opened by the app (see [`crate::QuickChat::toggle`]); what
//! is here are the keys while the keyboard is inside it. Most of them are
//! the shared chat's own actions (`writer_assistant_ui`), bound once more
//! in the contexts of the popover, so the same key does the same thing in
//! both chats and the tooltips of the shared controls name it.
//!
//! | Key | Action | What it does |
//! |---|---|---|
//! | `enter` | (the composer) | sends |
//! | `shift-enter` | (the composer) | a new line |
//! | `escape` | `DismissChat` | stops the answer that is being written; otherwise closes |
//! | `cmd-.` | `StopAnswer` | stops the answer |
//! | `cmd-shift-enter` | `ReplaceSelection` | Replace Selection with the answer |
//! | `cmd-enter` | [`InsertBelow`] | Insert Below |
//! | `cmd-shift-c` | `CopyAnswer` | Copy the answer |
//! | `cmd-shift-m` | `ChooseModel` | the model picker |
//! | `cmd-alt-e` | `ChooseEffort` | the effort picker |
//! | `tab`, `shift-tab` | `FocusNextArea`, `FocusPreviousArea` | between the composer and the conversation |
//!
//! In the conversation (after `tab`): the arrows mark a message, `enter`
//! opens its menu, `cmd-c` copies it (the shared list's keys). The three
//! answer keys act on the marked answer there, and on the latest answer
//! while the keyboard is in the composer.

use gpui::{KeyBinding, actions};
use writer_assistant_ui::{
    ChooseEffort, ChooseModel, CopyAnswer, DismissChat, FocusNextArea, FocusPreviousArea,
    ReplaceSelection, StopAnswer,
};

/// The key context of the popover's root.
pub const KEY_CONTEXT: &str = "QuickChat";

/// The flag the editors of the quick chat carry in their key context (the
/// composer and the answers): `Editor && assistant_quick`.
pub const EDITOR_FLAG: &str = "assistant_quick";

actions!(
    quick_chat,
    [
        /// Insert the answer as a new paragraph under the block the chat
        /// was opened in.
        InsertBelow,
    ]
);

/// The key bindings of the popover. Add them after `writer_editor::init`
/// and `writer_assistant_ui::init`: in an editor of the chat `cmd-enter`
/// and `cmd-shift-enter` are the editor's own keys otherwise, and of two
/// bindings at the same depth the later one wins.
pub fn key_bindings() -> Vec<KeyBinding> {
    let popover = Some(KEY_CONTEXT);
    let editors = Some("Editor && assistant_quick");
    vec![
        // Anywhere in the popover. (Shallow bindings: a key the focused
        // element binds itself wins over these.)
        KeyBinding::new("escape", DismissChat, popover),
        KeyBinding::new("tab", FocusNextArea, popover),
        KeyBinding::new("shift-tab", FocusPreviousArea, popover),
        KeyBinding::new("cmd-.", StopAnswer, popover),
        KeyBinding::new("cmd-shift-m", ChooseModel, popover),
        KeyBinding::new("cmd-alt-e", ChooseEffort, popover),
        KeyBinding::new("cmd-shift-c", CopyAnswer, popover),
        KeyBinding::new("cmd-enter", InsertBelow, popover),
        KeyBinding::new("cmd-shift-enter", ReplaceSelection, popover),
        // The editors of the chat (the composer, the text of an answer):
        // bound at the level of the focused element, where a window-wide
        // key of the app cannot outrank them and where `cmd-enter` is a
        // page break and `cmd-shift-enter` ticks a to-do item otherwise.
        KeyBinding::new("cmd-shift-m", ChooseModel, editors),
        KeyBinding::new("cmd-alt-e", ChooseEffort, editors),
        KeyBinding::new("cmd-shift-c", CopyAnswer, editors),
        KeyBinding::new("cmd-enter", InsertBelow, editors),
        KeyBinding::new("cmd-shift-enter", ReplaceSelection, editors),
    ]
}
