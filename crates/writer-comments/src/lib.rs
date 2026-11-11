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
//! // Per window, for the editor of that window.
//! let comments = cx.new(|cx| CommentsPanel::new(editor.clone(), author_name, window, cx));
//! cx.subscribe(&comments, |workspace: &mut Workspace, comments, event: &CommentsEvent, cx| {
//!     match event {
//!         // Show the panel's container only while there is something in it.
//!         CommentsEvent::OpenChanged(open) => {
//!             workspace.set_right_panel(open.then(|| comments.clone().into()), cx)
//!         }
//!         // The flag the app persists.
//!         CommentsEvent::VisibilityChanged(visible) => workspace.save_comments_visible(*visible, cx),
//!         CommentsEvent::ShowResolvedChanged(show) => workspace.save_show_resolved(*show, cx),
//!         _ => {}
//!     }
//! })
//! .detach();
//! // The document that was open before the subscription existed:
//! if comments.read(cx).is_open() {
//!     workspace.set_right_panel(Some(comments.clone().into()), cx);
//! }
//!
//! // Palette and shortcuts overlay.
//! let commands = writer_comments::commands();
//! ```
//!
//! - **Layout.** The panel fills its container (`size_full`) and draws no
//!   outer border. The container gives it a width (about 300 to 320 px)
//!   and the hairline beside the text, and is shown only while
//!   [`CommentsPanel::is_open`]: comments are visible and there is a thread
//!   or an open composer. A document without comments has no panel beside
//!   it. (If the container is shown anyway, the panel says "No comments in
//!   this document" there.)
//! - **Actions.** [`init`] installs global listeners for the actions that
//!   work from anywhere in a window. Whatever has focus (the editor, the
//!   library, nothing), an action that no element handled reaches the
//!   panel that was created for the active window. No listener is needed
//!   on an ancestor of the editor.
//! - **Visibility.** `EditorConfig::comments_visible` of the editor and
//!   [`CommentsPanel::is_visible`] are one flag. The panel starts with the
//!   editor's value, writes it when it shows or hides comments
//!   ([`ToggleComments`], [`AddComment`] while hidden), and follows it when
//!   someone else changes the editor's configuration. Either way it emits
//!   [`CommentsEvent::VisibilityChanged`]; the app saves the flag on that
//!   event and restores it through the editor configuration or
//!   [`CommentsPanel::set_visible`].
//! - **Another document.** With one editor entity per window and
//!   `Editor::set_document`, there is nothing to do: the panel follows
//!   `EditorEvent::DocumentReplaced`. For another editor entity, call
//!   [`CommentsPanel::set_editor`]. A comment that was being written is
//!   not thrown away and not applied to the other document: the panel
//!   keeps it ([`CommentsPanel::kept_drafts`]), says so in a toast, and
//!   reopens its composer when the document is open again.
//! - **Author and time.** The author name is the constructor argument
//!   ([`CommentsPanel::set_author`] changes it). Timestamps are read from
//!   the system clock ([`time::now`]).
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
//! | composer, thread list | `tab`, `shift-tab` | [`CycleFocus`]: between an open composer and the thread list |
//! | thread list | `up` `down`, `k` `j`, `home` `end` | move between threads; the editor scrolls to the anchor |
//! | thread list | `right` `left`, `l` `h` | step through the replies of the active thread |
//! | thread list | `enter` or `r` | [`ReplyToThread`] |
//! | thread list | `e` | [`EditComment`]: the comment, or the selected reply |
//! | thread list | `x` | [`ToggleResolved`] |
//! | thread list | `cmd-backspace`, then `enter` / `escape` | [`DeleteComment`], [`ConfirmDelete`] / [`CancelDelete`] |
//! | thread list | `s` | [`ToggleShowResolved`] |
//! | thread list | `cmd-z` / `cmd-shift-z` | undo / redo in the document; the text scrolls to what changed, a changed thread becomes active |
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
//! hidden), focus returns to the editor. `tab` moves between an open
//! composer and the thread list ([`CycleFocus`]).
//!
//! While focus is in the editor, the thread the caret is in is the active
//! one. While focus is in the panel, the panel leads: the active thread is
//! the one chosen there and the editor scrolls to it without moving the
//! caret.
//!
//! # Drafts
//!
//! What is typed into a composer is discarded by its own cancel key, and
//! with the thread it belongs to (a reply to a thread that is deleted or
//! resolved out of the list). Nothing else throws it away.
//!
//! - The target of a new comment follows its text through edits of the
//!   document made while the composer is open ([`track::RangeTracker`]).
//!   If the text is deleted, the composer stays open and says so
//!   ([`CommentsPanel::draft_target_lost`]); undo brings the target back,
//!   and [`AddComment`] points the composer at the current selection.
//! - When comments are hidden, the document is switched, or another
//!   composer opens over it, a composer with text in it is set aside
//!   ([`CommentsPanel::kept_drafts`]) and reopens, without taking the
//!   keyboard, when comments are shown, the document is back (the same
//!   text at the same place, or the same comment), or the other composer
//!   closes. Kept drafts live in the panel, not in the document: they do
//!   not survive closing the window.

#![warn(missing_docs)]

mod actions;
mod controls;
mod panel;
mod render;
pub mod threads;
pub mod time;
pub mod track;

pub use actions::{
    AddComment, CATEGORY, COMPOSER_CONTEXT, CancelComment, CancelDelete, ConfirmDelete,
    CopyComment, CycleFocus, DeleteComment, EditComment, FocusComments, FocusEditor, GoToThread,
    NextComment, PANEL_CONTEXT, PreviousComment, ReplyToThread, SelectFirstThread,
    SelectLastThread, SelectNextReply, SelectNextThread, SelectPreviousReply, SelectPreviousThread,
    SubmitComment, THREADS_CONTEXT, ToggleComments, ToggleResolved, ToggleShowResolved, commands,
    key_bindings,
};
pub use panel::{
    CommentMenuTarget, CommentsEvent, CommentsPanel, DRAFT_HIGHLIGHT_LAYER, DeleteTarget, DraftKind,
};
pub use threads::ThreadEntry;

use gpui::App;

/// Installs the comment key bindings and the global listeners for the
/// actions that work from anywhere in a window ([`AddComment`],
/// [`NextComment`], [`PreviousComment`], [`ToggleComments`],
/// [`FocusComments`], [`ToggleShowResolved`], and the commands on the active
/// thread: [`ReplyToThread`], [`EditComment`], [`ToggleResolved`],
/// [`DeleteComment`]). They reach the [`CommentsPanel`] of the active window
/// whenever no element on the focus path handled them, so the panel needs
/// no listener on any ancestor of the editor.
///
/// Call once at startup, after `writer_ui::init` and `writer_editor::init`.
pub fn init(cx: &mut App) {
    cx.bind_keys(actions::key_bindings());
    render::global_actions(cx);
}
