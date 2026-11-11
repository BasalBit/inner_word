//! The comment actions, their key bindings and the command list.
//!
//! Four key contexts are involved:
//!
//! | Context | Where | Keys |
//! |---|---|---|
//! | none | anywhere in the window | add comment, next / previous comment, show or hide comments, focus comments |
//! | [`THREADS_CONTEXT`] | the thread list has focus | thread and reply navigation, reply, edit, resolve, delete, show resolved, undo, back to the editor |
//! | [`THREADS_CONTEXT`] with `confirming` | a delete confirmation is showing | confirm, cancel |
//! | [`COMPOSER_CONTEXT`] | the composer has focus | submit, cancel, on to the thread list |
//!
//! The list bindings are disabled while a composer is open inside the list
//! (the `composing` flag), so letters typed into a reply are text, not
//! commands.

use gpui::{KeyBinding, actions};
use writer_ui::CommandDescriptor;

actions!(
    comments,
    [
        // --- Anywhere in the window ---
        /// Start a comment on the selected text, or on the word at the caret.
        AddComment,
        /// Move the caret to the next comment and make its thread active.
        NextComment,
        /// Move the caret to the previous comment and make its thread active.
        PreviousComment,
        /// Show or hide the comments panel and the comment tints in the text.
        ToggleComments,
        /// Move keyboard focus to the comments panel.
        FocusComments,
        /// Show or hide resolved threads in the comments panel.
        ToggleShowResolved,
        // --- The thread list ---
        /// Make the next thread in the panel active.
        SelectNextThread,
        /// Make the previous thread in the panel active.
        SelectPreviousThread,
        /// Make the first thread in the panel active.
        SelectFirstThread,
        /// Make the last thread in the panel active.
        SelectLastThread,
        /// Select the next reply of the active thread.
        SelectNextReply,
        /// Select the previous reply of the active thread, then the comment itself.
        SelectPreviousReply,
        /// Reply to the active thread.
        ReplyToThread,
        /// Edit the active comment, or the selected reply.
        EditComment,
        /// Resolve the active thread, or reopen it when it is resolved.
        ToggleResolved,
        /// Delete the active thread, or the selected reply, after a confirmation.
        DeleteComment,
        /// Confirm the pending delete.
        ConfirmDelete,
        /// Keep the comment: dismiss the delete confirmation.
        CancelDelete,
        /// Leave the comments panel and return focus to the editor.
        FocusEditor,
        /// Move the keyboard between an open composer and the thread list.
        CycleFocus,
        // --- The composer ---
        /// Save the comment, reply or edit being written.
        SubmitComment,
        /// Discard the comment, reply or edit being written.
        CancelComment,
    ]
);

/// Key context of the panel's root element.
pub const PANEL_CONTEXT: &str = "CommentsPanel";
/// Key context of the thread list (the element that holds focus while the
/// user moves between threads). It also carries the flags `confirming`
/// (a delete confirmation is showing) and `composing` (a reply or an edit
/// is being written inside the list).
pub const THREADS_CONTEXT: &str = "CommentThreads";
/// Key context around the composer (a compact `writer_editor::Editor`).
pub const COMPOSER_CONTEXT: &str = "CommentComposer";

/// Category of every command in [`commands`].
pub const CATEGORY: &str = "Comments";

/// The composer's own level: the compact editor inside the composer box.
/// The editor binds `tab` at this level too (in a compact editor it moves
/// to the next tab stop of the window, which from a composer can be
/// anywhere); of two bindings at the focused element's level the one added
/// later wins, and [`crate::init`] runs after `writer_editor::init`.
const COMPOSER_FIELD: &str = "CommentComposer > Editor";
const THREADS_IDLE: &str = "CommentThreads && !confirming && !composing";
const THREADS_CONFIRMING: &str = "CommentThreads && confirming";

/// The default key bindings. [`crate::init`] installs them.
///
/// Alternatives come first and the canonical key last, because key hints
/// show the binding that was added last.
pub fn key_bindings() -> Vec<KeyBinding> {
    let threads = Some(THREADS_IDLE);
    let confirming = Some(THREADS_CONFIRMING);
    let composer = Some(COMPOSER_CONTEXT);
    vec![
        // Anywhere in the window.
        KeyBinding::new("cmd-alt-m", AddComment, None),
        KeyBinding::new("cmd-alt-]", NextComment, None),
        KeyBinding::new("cmd-alt-[", PreviousComment, None),
        KeyBinding::new("cmd-alt-c", ToggleComments, None),
        KeyBinding::new("cmd-2", FocusComments, None),
        // The thread list.
        KeyBinding::new("j", SelectNextThread, threads),
        KeyBinding::new("ctrl-n", SelectNextThread, threads),
        KeyBinding::new("down", SelectNextThread, threads),
        KeyBinding::new("k", SelectPreviousThread, threads),
        KeyBinding::new("ctrl-p", SelectPreviousThread, threads),
        KeyBinding::new("up", SelectPreviousThread, threads),
        KeyBinding::new("cmd-up", SelectFirstThread, threads),
        KeyBinding::new("home", SelectFirstThread, threads),
        KeyBinding::new("cmd-down", SelectLastThread, threads),
        KeyBinding::new("end", SelectLastThread, threads),
        KeyBinding::new("l", SelectNextReply, threads),
        KeyBinding::new("right", SelectNextReply, threads),
        KeyBinding::new("h", SelectPreviousReply, threads),
        KeyBinding::new("left", SelectPreviousReply, threads),
        KeyBinding::new("r", ReplyToThread, threads),
        KeyBinding::new("enter", ReplyToThread, threads),
        KeyBinding::new("e", EditComment, threads),
        KeyBinding::new("x", ToggleResolved, threads),
        KeyBinding::new("cmd-backspace", DeleteComment, threads),
        KeyBinding::new("s", ToggleShowResolved, threads),
        KeyBinding::new("escape", FocusEditor, threads),
        // Back into a composer that was left open.
        KeyBinding::new("shift-tab", CycleFocus, threads),
        KeyBinding::new("tab", CycleFocus, threads),
        // The document's undo history holds the comment changes, so undo
        // must work without going back to the text first.
        KeyBinding::new("cmd-z", writer_editor::actions::Undo, threads),
        KeyBinding::new("cmd-shift-z", writer_editor::actions::Redo, threads),
        // A delete confirmation.
        KeyBinding::new("enter", ConfirmDelete, confirming),
        KeyBinding::new("escape", CancelDelete, confirming),
        // The composer. `enter` submits through the compact editor's own
        // submit-on-enter; these two are also caught by a key interceptor on
        // the composer, because the editor binds `cmd-enter` itself.
        KeyBinding::new("cmd-enter", SubmitComment, composer),
        KeyBinding::new("escape", CancelComment, composer),
        // A comment is plain text, so `tab` is free: it leads to the thread
        // list and leaves the draft open.
        KeyBinding::new("shift-tab", CycleFocus, Some(COMPOSER_FIELD)),
        KeyBinding::new("tab", CycleFocus, Some(COMPOSER_FIELD)),
    ]
}

/// The comment commands for the command palette and the shortcuts overlay.
///
/// The first six work from anywhere. The last four act on the active
/// thread, which is the one the caret is in or the one chosen in the panel.
pub fn commands() -> Vec<CommandDescriptor> {
    vec![
        CommandDescriptor::new("Add comment", CATEGORY, AddComment)
            .keywords(["note", "annotate", "remark"]),
        CommandDescriptor::new("Next comment", CATEGORY, NextComment).keywords(["jump", "go to"]),
        CommandDescriptor::new("Previous comment", CATEGORY, PreviousComment)
            .keywords(["jump", "go to"]),
        CommandDescriptor::new("Show or hide comments", CATEGORY, ToggleComments)
            .keywords(["toggle", "panel", "sidebar"]),
        CommandDescriptor::new("Focus comments", CATEGORY, FocusComments)
            .keywords(["panel", "threads"]),
        CommandDescriptor::new(
            "Show or hide resolved comments",
            CATEGORY,
            ToggleShowResolved,
        )
        .keywords(["toggle", "done", "closed"]),
        CommandDescriptor::new("Reply to comment", CATEGORY, ReplyToThread)
            .keywords(["answer", "thread"]),
        CommandDescriptor::new("Edit comment", CATEGORY, EditComment).keywords(["change"]),
        CommandDescriptor::new("Resolve or reopen comment", CATEGORY, ToggleResolved).keywords([
            "done",
            "close",
            "unresolve",
        ]),
        CommandDescriptor::new("Delete comment", CATEGORY, DeleteComment)
            .keywords(["remove", "thread"]),
    ]
}
