//! Moving between threads: in the list, through the replies of a thread,
//! and through the comments in the text.

use gpui::{Context, Window};
use writer_core::{CommentId, TextRange};
use writer_editor::Autoscroll;

use super::CommentsPanel;

impl CommentsPanel {
    /// Makes the next listed thread active (no wrap) and scrolls the editor
    /// to its anchor.
    pub fn select_next_thread(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.step_thread(1, cx);
    }

    /// Makes the previous listed thread active.
    pub fn select_previous_thread(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.step_thread(-1, cx);
    }

    /// Makes the first listed thread active.
    pub fn select_first_thread(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(first) = self.listed_ids().first() {
            self.set_active(Some(*first), true, cx);
        }
    }

    /// Makes the last listed thread active.
    pub fn select_last_thread(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(last) = self.listed_ids().last() {
            self.set_active(Some(*last), true, cx);
        }
    }

    fn step_thread(&mut self, delta: isize, cx: &mut Context<Self>) {
        let ids = self.listed_ids();
        if ids.is_empty() {
            return;
        }
        let current = self
            .active
            .and_then(|active| ids.iter().position(|id| *id == active));
        let ix = match current {
            Some(ix) => (ix as isize + delta).clamp(0, ids.len() as isize - 1) as usize,
            None if delta > 0 => 0,
            None => ids.len() - 1,
        };
        self.set_active(Some(ids[ix]), true, cx);
    }

    /// Steps into the replies of the active thread: selects its first
    /// reply, then the next one. [`CommentsPanel::edit`] and
    /// [`CommentsPanel::request_delete`] act on the selected reply.
    pub fn select_next_reply(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.active_entry() else {
            return;
        };
        let replies = &entry.thread.replies;
        let current = self
            .selected_reply
            .and_then(|selected| replies.iter().position(|reply| reply.id == selected));
        let next = match current {
            Some(ix) => replies.get(ix + 1).or(replies.last()),
            None => replies.first(),
        }
        .map(|reply| reply.id);
        if next != self.selected_reply {
            self.selected_reply = next;
            cx.notify();
        }
    }

    /// Selects the previous reply of the active thread; from the first
    /// reply, back to the opening comment.
    pub fn select_previous_reply(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.active_entry() else {
            return;
        };
        let replies = &entry.thread.replies;
        let current = self
            .selected_reply
            .and_then(|selected| replies.iter().position(|reply| reply.id == selected));
        let previous = match current {
            Some(ix) if ix > 0 => Some(replies[ix - 1].id),
            _ => None,
        };
        if previous != self.selected_reply {
            self.selected_reply = previous;
            cx.notify();
        }
    }

    /// Moves the caret to the start of the next comment in the text
    /// (wrapping to the first) and makes its thread active in the panel and
    /// in the editor. Focus stays where it is. Resolved threads are skipped
    /// unless they are listed; hidden comments are shown first.
    pub fn next_comment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.step_comment(true, window, cx);
    }

    /// Like [`CommentsPanel::next_comment`], backwards.
    pub fn previous_comment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.step_comment(false, window, cx);
    }

    fn step_comment(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.set_visible(true, window, cx);
        let stops: Vec<(CommentId, TextRange)> = self
            .listed_threads()
            .filter_map(|entry| entry.anchor().map(|anchor| (entry.id, anchor)))
            .collect();
        if stops.is_empty() {
            self.notice("This document has no comments to go to.", cx);
            return;
        }
        let head = self.editor.read(cx).selection().head;
        // Standing in the active thread: step from it, so threads that
        // share a start position are all reachable.
        let current = self.active.and_then(|active| {
            stops.iter().position(|(id, anchor)| {
                *id == active && anchor.start <= head && head <= anchor.end
            })
        });
        let count = stops.len();
        let ix = match (current, forward) {
            (Some(ix), true) => (ix + 1) % count,
            (Some(ix), false) => (ix + count - 1) % count,
            (None, true) => stops
                .iter()
                .position(|(_, anchor)| anchor.start > head)
                .unwrap_or(0),
            (None, false) => stops
                .iter()
                .rposition(|(_, anchor)| anchor.start < head)
                .unwrap_or(count - 1),
        };
        let (id, anchor) = stops[ix];
        self.editor.update(cx, |editor, cx| {
            editor.edit_with(Autoscroll::None, |state| state.set_caret(anchor.start), cx);
            editor.scroll_to_position(anchor.start, Autoscroll::Center, cx);
        });
        self.set_active(Some(id), false, cx);
    }
}
