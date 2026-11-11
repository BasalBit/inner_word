//! The comments panel of The Writer.
//!
//! [`CommentsPanel`] lists the comment threads of the document in an
//! [`Editor`](writer_editor::Editor), beside the text: a short quote of the
//! anchored text, the comment, its author and a relative time, replies
//! indented beneath. Resolved threads are hidden behind a "show resolved"
//! toggle; threads whose text was deleted are grouped under "Detached".
//! Everything is driven from the keyboard.
//!
//! The threads themselves live in the document (`writer-core`). The panel
//! reads them after every change and makes its own changes through
//! `Editor::edit_with`, so every one is an undo step of the document.
//!
//! # Mounting it
//!
//! ```ignore
//! // Once at startup, after writer_ui::init and writer_editor::init.
//! writer_comments::init(cx);
//!
//! // Per window, next to the editor.
//! let comments = cx.new(|cx| CommentsPanel::new(editor.clone(), author_name, window, cx));
//! comments.update(cx, |comments, cx| comments.set_visible(saved_flag, window, cx));
//! cx.subscribe(&comments, |this: &mut Workspace, _, event: &CommentsEvent, cx| match event {
//!     CommentsEvent::VisibilityChanged(visible) => this.save_setting("comments.visible", visible, cx),
//!     CommentsEvent::ShowResolvedChanged(show) => this.save_setting("comments.show_resolved", show, cx),
//!     _ => {}
//! }).detach();
//!
//! // In the workspace's render. The panel sizes itself: it is
//! // `CommentsPanel::width` wide while it has something to show and takes
//! // no space otherwise, so it is always a child.
//! writer_comments::workspace_actions(div().key_context("Workspace").flex().size_full(), &comments)
//!     .child(library_sidebar)
//!     .child(div().flex_1().min_w_0().child(editor.clone()))
//!     .child(comments.clone())
//!
//! // Another document in the same editor entity (Editor::set_document):
//! // nothing to do, the panel follows `EditorEvent::DocumentReplaced`.
//! // Another editor entity:
//! comments.update(cx, |comments, cx| comments.set_editor(new_editor, window, cx));
//!
//! // Palette and shortcuts overlay.
//! let commands = writer_comments::commands();
//! ```
//!
//! [`workspace_actions`] is what makes `cmd-alt-m`, `cmd-alt-]`,
//! `cmd-alt-[`, `cmd-alt-c` and `cmd-2` work while focus is in the editor
//! (or anywhere else under that element): it attaches the listeners for the
//! window-wide actions to an ancestor of both the editor and the panel.
//! Inside the panel, the panel handles its actions itself.
//!
//! # Keys
//!
//! | Where | Keys | Action |
//! |---|---|---|
//! | anywhere | `cmd-alt-m` | [`AddComment`]: composer for the selection, or the word at the caret |
//! | anywhere | `cmd-alt-]` / `cmd-alt-[` | [`NextComment`] / [`PreviousComment`]: caret to the anchor, thread active |
//! | anywhere | `cmd-alt-c` | [`ToggleComments`]: panel and tints |
//! | anywhere | `cmd-2` | [`FocusComments`]: focus the thread list |
//! | composer | `enter`, `cmd-enter` | [`SubmitComment`] (`shift-enter` is a line break) |
//! | composer | `escape` | [`CancelComment`]: back to where the draft was started |
//! | thread list | `up` `down`, `k` `j`, `home` `end` | move between threads; the editor scrolls to the anchor |
//! | thread list | `right` `left`, `l` `h` | step through the replies of the active thread |
//! | thread list | `enter` or `r` | [`ReplyToThread`] |
//! | thread list | `e` | [`EditComment`]: the comment, or the selected reply |
//! | thread list | `x` | [`ToggleResolved`] |
//! | thread list | `cmd-backspace`, then `enter` / `escape` | [`DeleteComment`], [`ConfirmDelete`] / [`CancelDelete`] |
//! | thread list | `s` | [`ToggleShowResolved`] |
//! | thread list | `cmd-z` / `cmd-shift-z` | undo / redo in the document |
//! | thread list | `escape` | [`FocusEditor`] |
//!
//! # Focus
//!
//! The panel has two things that take focus: the thread list
//! ([`CommentsPanel::focus_panel`]) and the composer (a compact
//! `writer_editor::Editor`). A draft remembers where it was started and
//! gives focus back there when it closes: the editor for a new comment, the
//! list for a reply or an edit made in the list. When the last thing that
//! could hold focus disappears (the last thread is deleted, comments are
//! hidden), focus returns to the editor.
//!
//! While focus is in the editor, the thread the caret is in is the active
//! one. While focus is in the panel, the panel leads: the active thread is
//! the one chosen there and the editor scrolls to it without moving the
//! caret.

#![warn(missing_docs)]

mod actions;
mod panel;
mod render;
pub mod threads;
pub mod time;

pub use actions::{
    AddComment, CATEGORY, COMPOSER_CONTEXT, CancelComment, CancelDelete, ConfirmDelete,
    DeleteComment, EditComment, FocusComments, FocusEditor, NextComment, PANEL_CONTEXT,
    PreviousComment, ReplyToThread, SelectFirstThread, SelectLastThread, SelectNextReply,
    SelectNextThread, SelectPreviousReply, SelectPreviousThread, SubmitComment, THREADS_CONTEXT,
    ToggleComments, ToggleResolved, ToggleShowResolved, commands, key_bindings,
};
pub use panel::{
    CommentsEvent, CommentsPanel, DRAFT_HIGHLIGHT_LAYER, DeleteTarget, DraftKind, PANEL_WIDTH,
};
pub use threads::ThreadEntry;

use gpui::{App, Entity, InteractiveElement};

/// Installs the comment key bindings. Call once at startup, after
/// `writer_ui::init` and `writer_editor::init`.
pub fn init(cx: &mut App) {
    cx.bind_keys(actions::key_bindings());
}

/// Attaches the listeners for the comment actions that work from anywhere
/// in the window to `element`, which must be an ancestor of the editor (the
/// workspace root is the natural place): [`AddComment`], [`NextComment`],
/// [`PreviousComment`], [`ToggleComments`], [`FocusComments`],
/// [`ToggleShowResolved`], and the commands on the active thread
/// ([`ReplyToThread`], [`EditComment`], [`ToggleResolved`],
/// [`DeleteComment`]) so that the command palette can run them.
///
/// ```ignore
/// writer_comments::workspace_actions(div().key_context("Workspace").size_full(), &self.comments)
///     .child(self.editor.clone())
///     .child(self.comments.clone())
/// ```
pub fn workspace_actions<E: InteractiveElement>(element: E, panel: &Entity<CommentsPanel>) -> E {
    render::window_actions(element, &panel.downgrade())
}
