//! Resolving, reopening and deleting threads, and undo from the list.

use gpui::{Context, Window};

use super::{CommentsPanel, DeleteTarget, PendingDelete};

impl CommentsPanel {
    /// Resolves the active thread, or reopens it when it is resolved. A
    /// resolved thread leaves the list unless resolved threads are shown;
    /// with focus in the list, its neighbour becomes active.
    pub fn toggle_resolved(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.active_entry() else {
            return;
        };
        let id = entry.id;
        let resolved = entry.is_resolved();
        let position = self.listed_ids().iter().position(|listed| *listed == id);
        self.mutate(cx, |state| {
            if resolved {
                state.reopen_comment(id)
            } else {
                state.resolve_comment(id)
            }
        });
        self.activate_neighbour(position, window, cx);
        self.repair_focus(window, cx);
    }

    /// After the active thread left the list: with focus in the list, the
    /// thread that took its place (or the last one) becomes active.
    fn activate_neighbour(
        &mut self,
        position: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active.is_some() || !self.focus_handle.is_focused(window) {
            return;
        }
        let ids = self.listed_ids();
        let neighbour = position
            .and_then(|position| ids.get(position))
            .or(ids.last())
            .copied();
        if neighbour.is_some() {
            self.set_active(neighbour, true, cx);
        }
    }

    /// Asks before deleting the active thread, or the selected reply: the
    /// thread shows a confirmation, the list takes focus, `enter` deletes
    /// and `escape` keeps. See [`CommentsPanel::confirm_delete`].
    pub fn request_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only(cx) {
            return;
        }
        let Some(thread) = self.active else {
            return;
        };
        let target = match self.selected_reply {
            Some(reply) => DeleteTarget::Reply(thread, reply),
            None => DeleteTarget::Thread(thread),
        };
        let return_to_editor = match self.confirming {
            Some(pending) => pending.return_to_editor,
            None => !self.has_focus(window, cx),
        };
        self.discard_draft(cx);
        self.confirming = Some(PendingDelete {
            target,
            return_to_editor,
        });
        window.focus(&self.focus_handle);
        self.scroll_list_to_active();
        cx.notify();
    }

    /// Deletes what [`CommentsPanel::request_delete`] asked about: the
    /// thread with its replies and its anchor in the text, or the one
    /// reply. One undo step.
    pub fn confirm_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pending) = self.confirming.take() else {
            return;
        };
        match pending.target {
            DeleteTarget::Thread(thread) => {
                let position = self.listed_ids().iter().position(|id| *id == thread);
                self.mutate(cx, |state| state.delete_comment(thread));
                if !pending.return_to_editor {
                    self.activate_neighbour(position, window, cx);
                }
            }
            DeleteTarget::Reply(thread, reply) => {
                self.mutate(cx, |state| state.delete_reply(thread, reply));
            }
        }
        if pending.return_to_editor {
            self.editor.read(cx).focus(window);
        } else {
            self.repair_focus(window, cx);
        }
        cx.notify();
    }

    /// Dismisses the delete confirmation and keeps the comment.
    pub fn cancel_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pending) = self.confirming.take() else {
            return;
        };
        if pending.return_to_editor {
            self.editor.read(cx).focus(window);
        }
        cx.notify();
    }

    /// Undoes the last change of the document (comment changes are part of
    /// its history). Bound to `cmd-z` in the thread list, so a resolve or a
    /// delete can be taken back without leaving the panel.
    pub fn undo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.mutate(cx, |state| state.undo());
        self.repair_focus(window, cx);
    }

    /// Redoes the last undone change of the document.
    pub fn redo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.mutate(cx, |state| state.redo());
        self.repair_focus(window, cx);
    }
}
