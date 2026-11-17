//! Resolving, reopening and deleting threads, and undo from the list.

use gpui::{Context, Window};
use innerword_core::{Change, CommentId, Position};
use innerword_editor::Autoscroll;

use super::{CommentsPanel, DeleteTarget, PendingDelete};
use crate::threads::ThreadEntry;

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
        self.leave_empty_list(window, cx);
    }

    /// After a command emptied the list while threads remain that are not
    /// listed (the last open thread was resolved or deleted, resolved ones
    /// are hidden): nothing is left to act on, so the keyboard goes back
    /// to the text instead of staying on a list without a thread.
    fn leave_empty_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.focus_handle.is_focused(window)
            && self.draft.is_none()
            && self.listed_threads().next().is_none()
        {
            self.editor.read(cx).focus(window);
            cx.notify();
        }
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
        self.keep_draft(true, cx);
        self.confirming = Some(PendingDelete {
            target,
            return_to_editor,
        });
        window.focus(&self.focus_handle);
        self.scroll_list_to_active(cx);
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
        self.restore_kept_draft(cx);
        self.leave_empty_list(window, cx);
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
        self.restore_kept_draft(cx);
        cx.notify();
    }

    /// Undoes the last change of the document (comment changes are part of
    /// its history). Bound to `cmd-z` in the thread list, so a resolve or a
    /// delete can be taken back without leaving the panel.
    ///
    /// The step that is undone may be something else than a comment change,
    /// and may be anywhere in the document. It is never undone out of
    /// sight: a thread that changed, came back or was reopened becomes the
    /// active one (the list and the text scroll to it), and for any other
    /// change the text scrolls to the place that changed.
    pub fn undo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.step_history(false, window, cx);
    }

    /// Redoes the last undone change of the document, and shows where it
    /// was like [`CommentsPanel::undo`].
    pub fn redo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.step_history(true, window, cx);
    }

    fn step_history(&mut self, redo: bool, window: &mut Window, cx: &mut Context<Self>) {
        let before = self.threads.clone();
        let change = self.mutate(cx, |state| {
            let stepped = if redo { state.redo() } else { state.undo() };
            stepped.then(|| state.last_change()).flatten()
        });
        if let Some(Some(change)) = change {
            self.show_change(&before, change, cx);
        }
        self.repair_focus(window, cx);
        // The active thread was the thing undone: the list keeps a thread
        // to work on, without moving the text away from what changed.
        if self.active.is_none() && self.focus_handle.is_focused(window) {
            let pick = self.thread_near_caret(cx);
            if pick.is_some() {
                self.set_active(pick, false, cx);
            }
        }
    }

    /// Brings what an undo or redo changed into view. `before` is the
    /// thread list before the step.
    fn show_change(&mut self, before: &[ThreadEntry], change: Change, cx: &mut Context<Self>) {
        let was = |id: CommentId| before.iter().find(|entry| entry.id == id);
        // A thread that is new, has other content, or lost or regained its
        // text. (Anchors that merely moved with the text do not count.)
        let touched = self
            .threads
            .iter()
            .find(|entry| {
                was(entry.id).is_none_or(|old| {
                    old.thread != entry.thread || old.is_detached() != entry.is_detached()
                })
            })
            .map(|entry| (entry.id, entry.anchor()));
        if let Some((id, anchor)) = touched {
            if self.is_listed(id) {
                self.set_active(Some(id), true, cx);
            } else if let Some(anchor) = anchor {
                // Resolved and not listed: its place in the text.
                self.scroll_text_to(anchor.start, cx);
            }
            return;
        }
        // A thread that is gone: where it was.
        let removed = before
            .iter()
            .find(|old| self.thread(old.id).is_none())
            .and_then(ThreadEntry::anchor);
        if let Some(anchor) = removed {
            self.scroll_text_to(anchor.start, cx);
            return;
        }
        // Text: the caret is put where the change was; failing that, the
        // first block that changed.
        if let Some(splice) = change.blocks {
            let head = self.editor.read(cx).selection().head;
            let position = if splice.new_range().contains(&head.block) {
                head
            } else {
                Position::new(splice.start, 0)
            };
            self.scroll_text_to(position, cx);
        }
    }

    fn scroll_text_to(&mut self, position: Position, cx: &mut Context<Self>) {
        self.editor.update(cx, |editor, cx| {
            let position = editor.document().clamp_position(position);
            editor.scroll_to_position(position, Autoscroll::Fit, cx);
        });
    }
}
