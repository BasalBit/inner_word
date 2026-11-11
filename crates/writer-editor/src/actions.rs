//! Every editor action and the default key bindings.
//!
//! The actions are plain unit structs in the `editor` namespace
//! (`editor::ToggleBold`), so they can be dispatched by name, listed by the
//! command palette (see [`crate::commands`]) and looked up in the keymap for
//! key hints. The editor handles them while it is focused; from outside,
//! dispatch one with `editor.focus_handle(cx).dispatch_action(&action,
//! window, cx)` or `window.dispatch_action(Box::new(action), cx)`.
//!
//! [`key_bindings`] binds them in the [`KEY_CONTEXT`] key context.
//! Alternatives are listed before the canonical key, because key hints show
//! the binding added last.

use gpui::{KeyBinding, actions};

/// The key context of a focused editor. The context also carries
/// `mode=full` or `mode=compact`, `read_only` when read-only, and whatever
/// consumers add with [`crate::Editor::set_key_context_value`] /
/// [`crate::Editor::add_key_context_flag`].
pub const KEY_CONTEXT: &str = "Editor";

actions!(
    editor,
    [
        // --- Movement ---
        /// Move the caret one character left.
        MoveLeft,
        /// Move the caret one character right.
        MoveRight,
        /// Move the caret one visual row up.
        MoveUp,
        /// Move the caret one visual row down.
        MoveDown,
        /// Move the caret to the previous word start.
        MoveWordLeft,
        /// Move the caret to the next word end.
        MoveWordRight,
        /// Move the caret to the start of the line.
        MoveToLineStart,
        /// Move the caret to the end of the line.
        MoveToLineEnd,
        /// Move the caret to the start of the document.
        MoveToDocumentStart,
        /// Move the caret to the end of the document.
        MoveToDocumentEnd,
        /// Move the caret one viewport page up.
        MovePageUp,
        /// Move the caret one viewport page down.
        MovePageDown,
        // --- Selection ---
        /// Extend the selection one character left.
        SelectLeft,
        /// Extend the selection one character right.
        SelectRight,
        /// Extend the selection one visual row up.
        SelectUp,
        /// Extend the selection one visual row down.
        SelectDown,
        /// Extend the selection to the previous word start.
        SelectWordLeft,
        /// Extend the selection to the next word end.
        SelectWordRight,
        /// Extend the selection to the start of the line.
        SelectToLineStart,
        /// Extend the selection to the end of the line.
        SelectToLineEnd,
        /// Extend the selection to the start of the document.
        SelectToDocumentStart,
        /// Extend the selection to the end of the document.
        SelectToDocumentEnd,
        /// Extend the selection one viewport page up.
        SelectPageUp,
        /// Extend the selection one viewport page down.
        SelectPageDown,
        /// Select the whole document.
        SelectAll,
        // --- Deletion ---
        /// Delete the character before the caret, or the selection.
        Backspace,
        /// Delete the character after the caret, or the selection.
        Delete,
        /// Delete back to the previous word start.
        DeleteWordBackward,
        /// Delete forward to the next word end.
        DeleteWordForward,
        /// Delete back to the start of the line.
        DeleteToLineStart,
        /// Delete forward to the end of the line.
        DeleteToLineEnd,
        // --- Insertion ---
        /// Start a new paragraph (in compact mode with submit-on-enter:
        /// submit).
        Newline,
        /// Insert a line break inside the block.
        InsertSoftBreak,
        /// Insert a page break.
        InsertPageBreak,
        /// Insert a divider.
        InsertDivider,
        // --- Inline formatting ---
        /// Toggle bold.
        ToggleBold,
        /// Toggle italic.
        ToggleItalic,
        /// Toggle underline.
        ToggleUnderline,
        /// Toggle strikethrough.
        ToggleStrikethrough,
        /// Toggle inline code.
        ToggleInlineCode,
        /// Toggle the yellow highlight.
        ToggleHighlight,
        /// Remove all inline formatting from the selection.
        ClearFormatting,
        // --- Block kinds ---
        /// Turn the selected blocks into paragraphs.
        SetParagraph,
        /// Turn the selected blocks into level 1 headings.
        SetHeading1,
        /// Turn the selected blocks into level 2 headings.
        SetHeading2,
        /// Turn the selected blocks into level 3 headings.
        SetHeading3,
        /// Turn the selected blocks into level 4 headings.
        SetHeading4,
        /// Turn the selected blocks into level 5 headings.
        SetHeading5,
        /// Turn the selected blocks into level 6 headings.
        SetHeading6,
        /// Turn the selected blocks into numbered list items, or back into
        /// paragraphs.
        ToggleNumberedList,
        /// Turn the selected blocks into bullet list items, or back into
        /// paragraphs.
        ToggleBulletList,
        /// Turn the selected blocks into to-do items, or back into
        /// paragraphs.
        ToggleTodoList,
        /// Turn the selected blocks into quotes, or back into paragraphs.
        ToggleQuote,
        /// Turn the selected blocks into a code block, or back into
        /// paragraphs.
        ToggleCodeBlock,
        /// Check or uncheck the selected to-do items.
        ToggleTodoChecked,
        // --- Block layout ---
        /// Align the selected blocks left.
        AlignLeft,
        /// Centre the selected blocks.
        AlignCenter,
        /// Align the selected blocks right.
        AlignRight,
        /// Justify the selected blocks.
        AlignJustify,
        /// Make the text of the selected blocks one step larger.
        IncreaseTextSize,
        /// Make the text of the selected blocks one step smaller.
        DecreaseTextSize,
        /// Give the selected blocks their default text size again.
        ResetTextSize,
        /// Indent the selected blocks.
        Indent,
        /// Outdent the selected blocks.
        Outdent,
        /// Move the selected blocks up.
        MoveBlockUp,
        /// Move the selected blocks down.
        MoveBlockDown,
        // --- History and clipboard ---
        /// Undo the last change.
        Undo,
        /// Redo the last undone change.
        Redo,
        /// Copy the selection.
        Copy,
        /// Cut the selection.
        Cut,
        /// Paste the clipboard.
        Paste,
        /// Paste the clipboard as plain text.
        PasteAsPlainText,
    ]
);

/// The default key bindings of the editor, all in the [`KEY_CONTEXT`]
/// context. [`crate::init`] installs them.
pub fn key_bindings() -> Vec<KeyBinding> {
    let c = Some(KEY_CONTEXT);
    vec![
        // Movement (Emacs alternatives first, canonical keys last).
        KeyBinding::new("ctrl-b", MoveLeft, c),
        KeyBinding::new("left", MoveLeft, c),
        KeyBinding::new("ctrl-f", MoveRight, c),
        KeyBinding::new("right", MoveRight, c),
        KeyBinding::new("ctrl-p", MoveUp, c),
        KeyBinding::new("up", MoveUp, c),
        KeyBinding::new("ctrl-n", MoveDown, c),
        KeyBinding::new("down", MoveDown, c),
        KeyBinding::new("alt-left", MoveWordLeft, c),
        KeyBinding::new("alt-right", MoveWordRight, c),
        KeyBinding::new("ctrl-a", MoveToLineStart, c),
        KeyBinding::new("home", MoveToLineStart, c),
        KeyBinding::new("cmd-left", MoveToLineStart, c),
        KeyBinding::new("ctrl-e", MoveToLineEnd, c),
        KeyBinding::new("end", MoveToLineEnd, c),
        KeyBinding::new("cmd-right", MoveToLineEnd, c),
        KeyBinding::new("cmd-up", MoveToDocumentStart, c),
        KeyBinding::new("cmd-down", MoveToDocumentEnd, c),
        KeyBinding::new("pageup", MovePageUp, c),
        KeyBinding::new("pagedown", MovePageDown, c),
        // Selection.
        KeyBinding::new("shift-left", SelectLeft, c),
        KeyBinding::new("shift-right", SelectRight, c),
        KeyBinding::new("shift-up", SelectUp, c),
        KeyBinding::new("shift-down", SelectDown, c),
        KeyBinding::new("alt-shift-left", SelectWordLeft, c),
        KeyBinding::new("alt-shift-right", SelectWordRight, c),
        KeyBinding::new("shift-home", SelectToLineStart, c),
        KeyBinding::new("cmd-shift-left", SelectToLineStart, c),
        KeyBinding::new("shift-end", SelectToLineEnd, c),
        KeyBinding::new("cmd-shift-right", SelectToLineEnd, c),
        KeyBinding::new("cmd-shift-up", SelectToDocumentStart, c),
        KeyBinding::new("cmd-shift-down", SelectToDocumentEnd, c),
        KeyBinding::new("shift-pageup", SelectPageUp, c),
        KeyBinding::new("shift-pagedown", SelectPageDown, c),
        KeyBinding::new("cmd-a", SelectAll, c),
        // Deletion.
        KeyBinding::new("ctrl-h", Backspace, c),
        KeyBinding::new("shift-backspace", Backspace, c),
        KeyBinding::new("backspace", Backspace, c),
        KeyBinding::new("ctrl-d", Delete, c),
        KeyBinding::new("delete", Delete, c),
        KeyBinding::new("alt-backspace", DeleteWordBackward, c),
        KeyBinding::new("alt-delete", DeleteWordForward, c),
        KeyBinding::new("cmd-backspace", DeleteToLineStart, c),
        KeyBinding::new("ctrl-k", DeleteToLineEnd, c),
        KeyBinding::new("cmd-delete", DeleteToLineEnd, c),
        // Insertion.
        KeyBinding::new("enter", Newline, c),
        KeyBinding::new("shift-enter", InsertSoftBreak, c),
        KeyBinding::new("cmd-enter", InsertPageBreak, c),
        // Inline formatting.
        KeyBinding::new("cmd-b", ToggleBold, c),
        KeyBinding::new("cmd-i", ToggleItalic, c),
        KeyBinding::new("cmd-u", ToggleUnderline, c),
        KeyBinding::new("cmd-shift-x", ToggleStrikethrough, c),
        KeyBinding::new("cmd-e", ToggleInlineCode, c),
        KeyBinding::new("cmd-shift-h", ToggleHighlight, c),
        KeyBinding::new("cmd-\\", ClearFormatting, c),
        // Block kinds.
        KeyBinding::new("cmd-alt-0", SetParagraph, c),
        KeyBinding::new("cmd-alt-1", SetHeading1, c),
        KeyBinding::new("cmd-alt-2", SetHeading2, c),
        KeyBinding::new("cmd-alt-3", SetHeading3, c),
        KeyBinding::new("cmd-alt-4", SetHeading4, c),
        KeyBinding::new("cmd-alt-5", SetHeading5, c),
        KeyBinding::new("cmd-alt-6", SetHeading6, c),
        // Shift plus a digit or a punctuation key arrives from a real
        // keyboard as the shifted character without the shift flag (GPUI's
        // macOS key translation: `cmd-shift-7` is `cmd-&` on a US layout),
        // so those are bound too. The spelled-out form comes last: it is
        // the one key hints show, and what scripted keystrokes send.
        KeyBinding::new("cmd-&", ToggleNumberedList, c),
        KeyBinding::new("cmd-shift-7", ToggleNumberedList, c),
        KeyBinding::new("cmd-*", ToggleBulletList, c),
        KeyBinding::new("cmd-shift-8", ToggleBulletList, c),
        KeyBinding::new("cmd-(", ToggleTodoList, c),
        KeyBinding::new("cmd-shift-9", ToggleTodoList, c),
        KeyBinding::new("cmd-'", ToggleQuote, c),
        KeyBinding::new("cmd-\"", ToggleCodeBlock, c),
        KeyBinding::new("cmd-shift-'", ToggleCodeBlock, c),
        KeyBinding::new("cmd-shift-enter", ToggleTodoChecked, c),
        // Block layout.
        KeyBinding::new("cmd-shift-l", AlignLeft, c),
        KeyBinding::new("cmd-shift-e", AlignCenter, c),
        KeyBinding::new("cmd-shift-r", AlignRight, c),
        KeyBinding::new("cmd-shift-j", AlignJustify, c),
        KeyBinding::new("cmd->", IncreaseTextSize, c),
        KeyBinding::new("cmd-shift-.", IncreaseTextSize, c),
        KeyBinding::new("cmd-<", DecreaseTextSize, c),
        KeyBinding::new("cmd-shift-,", DecreaseTextSize, c),
        KeyBinding::new("cmd-]", Indent, c),
        KeyBinding::new("tab", Indent, c),
        KeyBinding::new("cmd-[", Outdent, c),
        KeyBinding::new("shift-tab", Outdent, c),
        KeyBinding::new("alt-up", MoveBlockUp, c),
        KeyBinding::new("alt-down", MoveBlockDown, c),
        // History and clipboard.
        KeyBinding::new("cmd-z", Undo, c),
        KeyBinding::new("cmd-shift-z", Redo, c),
        KeyBinding::new("cmd-c", Copy, c),
        KeyBinding::new("cmd-x", Cut, c),
        KeyBinding::new("cmd-v", Paste, c),
        KeyBinding::new("cmd-shift-v", PasteAsPlainText, c),
    ]
}
