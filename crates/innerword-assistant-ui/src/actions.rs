//! The actions of the chat, their key contexts and their keys.
//!
//! # Key contexts
//!
//! | Context | Where the keyboard is |
//! |---|---|
//! | `Editor && assistant_composer` | the composer of a chat (a compact editor) |
//! | `Editor && assistant_answer` | the text of an answer (a read-only editor: select and copy) |
//! | [`MESSAGES_CONTEXT`] | the message list |
//! | [`HEADER_CONTEXT`] | the controls of the side chat's header |
//! | [`CONTEXT_ROW_CONTEXT`] | the context chips of the side chat |
//! | [`PANEL_CONTEXT`] | anywhere in the side chat (the root of [`crate::AssistantPanel`]) |
//!
//! Both kinds of editor also carry the flag [`CHAT_EDITOR_FLAG`]
//! (`assistant`), and the flag their host gave them
//! ([`crate::ChatKeys::editor_flag`]): the side chat's is
//! [`SIDE_EDITOR_FLAG`]. A host binds a key that must win over one of the
//! editor's own keys (`cmd-enter` is a page break there) with the predicate
//! `Editor && <its flag>`, and adds the binding after `innerword_editor::init`:
//! of two bindings at the same depth the later one wins.

use gpui::{KeyBinding, actions};
use innerword_ui::{CommandDescriptor, context_menu::OpenContextMenu};

/// The key context of the side chat's root.
pub const PANEL_CONTEXT: &str = "AssistantPanel";
/// The key context of a message list while it has the keyboard.
pub const MESSAGES_CONTEXT: &str = "AssistantMessages";
/// The key context around a composer.
pub const COMPOSER_CONTEXT: &str = "AssistantComposer";
/// The key context of the side chat's header while it has the keyboard.
pub const HEADER_CONTEXT: &str = "AssistantHeader";
/// The key context of the side chat's context row while it has the
/// keyboard.
pub const CONTEXT_ROW_CONTEXT: &str = "AssistantContext";

/// The flag every editor of a chat carries in its key context (the
/// composer and the answers).
pub const CHAT_EDITOR_FLAG: &str = "assistant";
/// The flag of a composer's editor.
pub const COMPOSER_EDITOR_FLAG: &str = "assistant_composer";
/// The flag of an answer's editor.
pub const ANSWER_EDITOR_FLAG: &str = "assistant_answer";
/// The flag of the editors of the side chat.
pub const SIDE_EDITOR_FLAG: &str = "assistant_side";

/// The category of the commands in the palette.
pub const CATEGORY: &str = "Assistant";

actions!(
    assistant,
    [
        /// Stop the answer that is being written.
        StopAnswer,
        /// Leave the chat: the keyboard goes back to the text.
        DismissChat,
        /// Move the keyboard to the next part of the chat (composer,
        /// messages, context, header).
        FocusNextArea,
        /// Move the keyboard to the previous part of the chat.
        FocusPreviousArea,
        /// Move the keyboard to the composer.
        FocusComposer,
        /// Mark the next message of the list.
        SelectNextMessage,
        /// Mark the previous message of the list.
        SelectPreviousMessage,
        /// Mark the first message of the list.
        SelectFirstMessage,
        /// Mark the last message of the list.
        SelectLastMessage,
        /// Copy the marked message.
        CopyMessage,
        /// Start a new conversation: no messages, only the open document as
        /// context.
        NewChat,
        /// Remove the messages of the conversation; what was added as
        /// context stays.
        ClearChat,
        /// Choose the model that answers.
        ChooseModel,
        /// Choose how much effort the model spends.
        ChooseEffort,
        /// Add a document or a folder of the library as context.
        AddContext,
        /// Insert the answer at the caret of the document.
        InsertAtCaret,
        /// Replace the selection of the document with the answer.
        ReplaceSelection,
        /// Copy the answer.
        CopyAnswer,
        /// Draw the answer's drawing: on the open whiteboard, or as a new
        /// whiteboard beside the open document (the host says which,
        /// [`crate::AssistantPanel::set_drawing_target`]).
        DrawAnswer,
        /// Close the side chat.
        CloseAssistant,
        /// Move to the next control of the header or the context row.
        NextControl,
        /// Move to the previous control of the header or the context row.
        PreviousControl,
        /// Press the marked control of the header or the context row.
        PressControl,
        /// Remove the marked context chip.
        RemoveContext,
        /// What `enter` does in the message list: the menu of the marked
        /// message, or, in an empty conversation, the marked example
        /// prompt goes into the composer.
        ActivateSelected,
    ]
);

/// Every key binding of this crate. Add them after `innerword_ui::init`,
/// `innerword_editor::init` and the editor tools: the keys of the chat's
/// editors must come later than the editor's own (see the
/// [module docs](self)).
pub fn key_bindings() -> Vec<KeyBinding> {
    let chat_editor = Some("Editor && assistant");
    let side_editor = Some("Editor && assistant_side");
    let panel = Some(PANEL_CONTEXT);
    let messages = Some(MESSAGES_CONTEXT);
    let header = Some(HEADER_CONTEXT);
    let context_row = Some(CONTEXT_ROW_CONTEXT);
    vec![
        // Anywhere in the side chat. (Shallow bindings: a key the focused
        // element binds itself wins over these.)
        KeyBinding::new("escape", DismissChat, panel),
        KeyBinding::new("tab", FocusNextArea, panel),
        KeyBinding::new("shift-tab", FocusPreviousArea, panel),
        KeyBinding::new("cmd-.", StopAnswer, panel),
        KeyBinding::new("cmd-alt-n", NewChat, panel),
        KeyBinding::new("cmd-alt-k", ClearChat, panel),
        KeyBinding::new("cmd-shift-m", ChooseModel, panel),
        KeyBinding::new("cmd-alt-e", ChooseEffort, panel),
        KeyBinding::new("cmd-shift-a", AddContext, panel),
        KeyBinding::new("cmd-enter", InsertAtCaret, panel),
        KeyBinding::new("cmd-shift-enter", ReplaceSelection, panel),
        KeyBinding::new("cmd-shift-c", CopyAnswer, panel),
        // In the panel only: its editors bind nothing on it (InsertAtCaret
        // is bound a second time below because the editor binds
        // `cmd-enter` itself), so this binding reaches the composer and the
        // answers too.
        KeyBinding::new("cmd-shift-d", DrawAnswer, panel),
        // The editors of any chat: these keys are the editor's own
        // otherwise (`tab` moves to the next field of the window, `cmd-.`
        // is the format menu).
        KeyBinding::new("escape", DismissChat, chat_editor),
        KeyBinding::new("tab", FocusNextArea, chat_editor),
        KeyBinding::new("shift-tab", FocusPreviousArea, chat_editor),
        KeyBinding::new("cmd-.", StopAnswer, chat_editor),
        // The editors of the side chat: `cmd-enter` is a page break and
        // `cmd-shift-enter` ticks a to-do item in an editor.
        KeyBinding::new("cmd-enter", InsertAtCaret, side_editor),
        KeyBinding::new("cmd-shift-enter", ReplaceSelection, side_editor),
        // The message list of any chat.
        KeyBinding::new("escape", DismissChat, messages),
        KeyBinding::new("tab", FocusNextArea, messages),
        KeyBinding::new("shift-tab", FocusPreviousArea, messages),
        KeyBinding::new("cmd-.", StopAnswer, messages),
        KeyBinding::new("ctrl-p", SelectPreviousMessage, messages),
        KeyBinding::new("ctrl-n", SelectNextMessage, messages),
        KeyBinding::new("up", SelectPreviousMessage, messages),
        KeyBinding::new("down", SelectNextMessage, messages),
        KeyBinding::new("home", SelectFirstMessage, messages),
        KeyBinding::new("end", SelectLastMessage, messages),
        KeyBinding::new("cmd-up", SelectFirstMessage, messages),
        KeyBinding::new("cmd-down", SelectLastMessage, messages),
        KeyBinding::new("cmd-c", CopyMessage, messages),
        KeyBinding::new("shift-f10", OpenContextMenu, messages),
        KeyBinding::new("ctrl-enter", OpenContextMenu, messages),
        KeyBinding::new("enter", ActivateSelected, messages),
        // The header and the context row: one stop of the keyboard each,
        // the arrows move between their controls.
        KeyBinding::new("left", PreviousControl, header),
        KeyBinding::new("right", NextControl, header),
        KeyBinding::new("space", PressControl, header),
        KeyBinding::new("enter", PressControl, header),
        KeyBinding::new("left", PreviousControl, context_row),
        KeyBinding::new("right", NextControl, context_row),
        KeyBinding::new("space", PressControl, context_row),
        KeyBinding::new("enter", PressControl, context_row),
        KeyBinding::new("delete", RemoveContext, context_row),
        KeyBinding::new("backspace", RemoveContext, context_row),
    ]
}

/// The title of [`DrawAnswer`] in the list of shortcuts, which searches
/// titles only: it names both of the key's meanings, as the zoom's
/// "Automatic" does. The palette names the one that applies ("Draw answer on
/// whiteboard" on a whiteboard, [`NEW_BOARD_FROM_ANSWER_TITLE`] over a
/// document), with this title among its keywords.
pub const DRAW_ANSWER_TITLE: &str = "Draw answer (on the whiteboard, or as a new whiteboard)";
/// The palette's title of [`DrawAnswer`] over a document.
pub const NEW_BOARD_FROM_ANSWER_TITLE: &str = "New whiteboard from answer";

/// The commands of the side chat for the command palette and the shortcuts
/// overlay. They act on the side chat of the window, wherever the keyboard
/// is, once the app has made them reach it (see the crate docs).
pub fn commands() -> Vec<CommandDescriptor> {
    vec![
        CommandDescriptor::new("New chat", CATEGORY, NewChat).keywords([
            "assistant",
            "conversation",
            "start",
        ]),
        CommandDescriptor::new("Clear chat", CATEGORY, ClearChat).keywords([
            "assistant",
            "conversation",
            "messages",
            "empty",
        ]),
        CommandDescriptor::new("Stop the answer", CATEGORY, StopAnswer).keywords([
            "assistant",
            "cancel",
            "interrupt",
        ]),
        CommandDescriptor::new("Choose model", CATEGORY, ChooseModel)
            .keywords(["assistant", "model"]),
        CommandDescriptor::new("Choose effort", CATEGORY, ChooseEffort).keywords([
            "assistant",
            "effort",
            "thinking",
        ]),
        CommandDescriptor::new("Add context", CATEGORY, AddContext).keywords([
            "assistant",
            "document",
            "folder",
            "attach",
        ]),
        CommandDescriptor::new("Insert answer at caret", CATEGORY, InsertAtCaret).keywords([
            "assistant",
            "apply",
            "paste",
        ]),
        CommandDescriptor::new("Replace selection with answer", CATEGORY, ReplaceSelection)
            .keywords(["assistant", "apply"]),
        CommandDescriptor::new(DRAW_ANSWER_TITLE, CATEGORY, DrawAnswer).keywords([
            "assistant",
            "draw",
            "drawing",
            "diagram",
            "whiteboard",
            "apply",
        ]),
        CommandDescriptor::new("Copy answer", CATEGORY, CopyAnswer)
            .keywords(["assistant", "clipboard"]),
    ]
}
