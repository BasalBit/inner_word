//! What the mouse does in the panel, and the commands that name their
//! thread.
//!
//! The keys of the thread list act on the active thread. A button on a card
//! and an item of a context menu act on the card they belong to, whichever
//! thread is active: each command here makes its thread the active one
//! first and then does exactly what the key does. The app uses the same
//! commands for the comment items it puts into the text's context menu
//! ("Go to Comment" on commented text is [`CommentsPanel::go_to_thread`]).

use gpui::{ClipboardItem, Context, Window};
use writer_core::{CommentId, ReplyId};

use super::CommentsPanel;

impl CommentsPanel {
    /// The thread card the pointer is over. It shows its buttons (reply,
    /// resolve, more), as the active card does.
    pub fn hovered_thread(&self) -> Option<CommentId> {
        self.hovered
    }

    /// The reply the pointer is over. It shows its "more" button, as the
    /// selected reply does.
    pub fn hovered_reply(&self) -> Option<(CommentId, ReplyId)> {
        self.hovered_reply
    }

    pub(crate) fn hover_thread(
        &mut self,
        thread: CommentId,
        hovered: bool,
        cx: &mut Context<Self>,
    ) {
        let next = if hovered {
            Some(thread)
        } else if self.hovered == Some(thread) {
            None
        } else {
            return;
        };
        if self.hovered != next {
            self.hovered = next;
            cx.notify();
        }
    }

    pub(crate) fn hover_reply(
        &mut self,
        thread: CommentId,
        reply: ReplyId,
        hovered: bool,
        cx: &mut Context<Self>,
    ) {
        let next = if hovered {
            Some((thread, reply))
        } else if self.hovered_reply == Some((thread, reply)) {
            None
        } else {
            return;
        };
        if self.hovered_reply != next {
            self.hovered_reply = next;
            cx.notify();
        }
    }

    /// Makes `thread` the active one and selects `reply` in it (`None`: the
    /// opening comment), for a command that names what it acts on. `false`
    /// when the panel does not list the thread or the reply is gone.
    pub(super) fn aim(
        &mut self,
        thread: CommentId,
        reply: Option<ReplyId>,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.is_listed(thread) {
            return false;
        }
        if let Some(reply) = reply
            && !self
                .thread(thread)
                .is_some_and(|entry| entry.thread.reply(reply).is_some())
        {
            return false;
        }
        self.set_active(Some(thread), true, cx);
        if self.selected_reply != reply {
            self.selected_reply = reply;
            cx.notify();
        }
        true
    }

    /// A click on a thread: it becomes active, the editor scrolls to its
    /// anchor, and the list takes the keyboard (unless a composer is open,
    /// which keeps it). A reply that was selected in it no longer is: the
    /// click was on the thread.
    pub(crate) fn click_thread(
        &mut self,
        thread: CommentId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.aim(thread, None, cx) {
            self.focus_list_after_click(window, cx);
        }
    }

    /// A click on a reply: its thread becomes active and the reply is the
    /// selected one, as after stepping to it with the arrow keys.
    pub fn select_reply(
        &mut self,
        thread: CommentId,
        reply: ReplyId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.aim(thread, Some(reply), cx) {
            self.focus_list_after_click(window, cx);
        }
    }

    fn focus_list_after_click(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // A composer that was left open keeps the keyboard if it has it; a
        // press inside the list has already moved it to the list otherwise.
        let composing = self.draft.is_some() && self.composer_focused(window, cx);
        if !composing {
            window.focus(&self.focus_handle);
        }
        cx.notify();
    }

    fn composer_focused(&self, window: &Window, cx: &gpui::App) -> bool {
        gpui::Focusable::focus_handle(self.composer.read(cx), cx).is_focused(window)
    }

    /// Shows `thread` in the panel and its text in the editor: comments are
    /// shown if they were hidden, resolved threads are listed if the thread
    /// is one of them, the thread becomes the active one (the text scrolls
    /// to its anchor, the list to its card) and the thread list takes the
    /// keyboard, so the keys of the list act on it. An open composer is
    /// left as it is.
    ///
    /// This is "Go to Comment" for a thread the app knows the id of, for
    /// example from `ContextMenuRequest::comments` of a right-click on
    /// commented text. The [`crate::GoToThread`] action does the same from
    /// a menu item. Returns `false` when the document has no such thread.
    pub fn go_to_thread(
        &mut self,
        thread: CommentId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.thread(thread).is_none() {
            return false;
        }
        self.set_visible(true, window, cx);
        if !self.is_listed(thread) {
            self.set_show_resolved(true, window, cx);
        }
        if !self.aim(thread, None, cx) {
            return false;
        }
        window.focus(&self.focus_handle);
        cx.notify();
        true
    }

    /// [`CommentsPanel::reply`] to `thread`: opens the reply composer in
    /// its card.
    pub fn reply_to(&mut self, thread: CommentId, window: &mut Window, cx: &mut Context<Self>) {
        if self.aim(thread, None, cx) {
            self.reply(window, cx);
        }
    }

    /// [`CommentsPanel::edit`] for the opening comment of `thread`.
    pub fn edit_comment(&mut self, thread: CommentId, window: &mut Window, cx: &mut Context<Self>) {
        if self.aim(thread, None, cx) {
            self.edit(window, cx);
        }
    }

    /// [`CommentsPanel::edit`] for one reply.
    pub fn edit_reply(
        &mut self,
        thread: CommentId,
        reply: ReplyId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.aim(thread, Some(reply), cx) {
            self.edit(window, cx);
        }
    }

    /// [`CommentsPanel::toggle_resolved`] for `thread`: resolves it, or
    /// reopens it when it is resolved.
    pub fn toggle_resolved_of(
        &mut self,
        thread: CommentId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.aim(thread, None, cx) {
            self.toggle_resolved(window, cx);
        }
    }

    /// [`CommentsPanel::request_delete`] for `thread`, or for one reply of
    /// it: shows the confirmation in its card.
    pub fn request_delete_of(
        &mut self,
        thread: CommentId,
        reply: Option<ReplyId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.aim(thread, reply, cx) {
            self.request_delete(window, cx);
        }
    }

    /// The words of a comment: the opening comment of `thread`, or one
    /// reply of it.
    pub fn comment_text(&self, thread: CommentId, reply: Option<ReplyId>) -> Option<&str> {
        let thread = &self.thread(thread)?.thread;
        match reply {
            Some(reply) => thread.reply(reply).map(|reply| reply.body.as_str()),
            None => Some(thread.body.as_str()),
        }
    }

    /// Puts the words of a comment on the clipboard ("Copy Text" in a
    /// thread's menu). Returns whether there was such a comment.
    pub fn copy_comment_text(
        &mut self,
        thread: CommentId,
        reply: Option<ReplyId>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(text) = self.comment_text(thread, reply).map(str::to_string) else {
            return false;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        true
    }

    /// Puts the words of the selected reply, or of the active comment, on
    /// the clipboard (`cmd-c` in the thread list, "Copy Text" in a menu)
    /// and says so.
    pub fn copy_active(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some(thread) = self.active else {
            return;
        };
        if self.copy_comment_text(thread, self.selected_reply, cx) {
            self.notice("Copied the comment's text.", cx);
        }
    }

    /// Whether the composer has something that [`CommentsPanel::submit`]
    /// would save: a draft is open, there is text in it, and a new comment
    /// still has its text in the document.
    pub fn can_submit(&self, cx: &gpui::App) -> bool {
        self.draft.is_some()
            && !self.composer_empty
            && !self.draft_target_lost()
            && !self.read_only(cx)
    }
}
