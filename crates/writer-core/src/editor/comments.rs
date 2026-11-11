//! Comment thread commands. Every one is an undoable transaction.

use crate::attrs::CommentId;
use crate::comments::{CommentThread, Reply, ReplyId, Timestamp};
use crate::history::EditKind;
use crate::text;

use super::EditorState;

fn clean(body: &str) -> String {
    text::normalize_input(body).into_owned()
}

impl EditorState {
    /// Starts a comment thread on the selected text (void blocks skipped;
    /// code blocks included). Returns `None` when no text is selected.
    pub fn add_comment(&mut self, body: &str, author: &str, now: Timestamp) -> Option<CommentId> {
        let ranges = self.selected_ranges(true);
        if ranges.is_empty() {
            return None;
        }
        let id = self.doc.mint_comment_id();
        let thread = CommentThread {
            id,
            body: clean(body),
            author: author.trim().to_string(),
            created_at: now,
            resolved: false,
            replies: Vec::new(),
        };
        self.edit(EditKind::Other, |ed| {
            ed.set_thread(id, Some(thread));
            for (i, r) in ranges {
                ed.update_block(i, |b| {
                    b.apply_attrs(r, |a| {
                        a.comments.insert(id);
                    })
                });
            }
        });
        Some(id)
    }

    /// Applies `f` to a copy of thread `id` and stores it if it changed.
    fn modify_thread(&mut self, id: CommentId, f: impl FnOnce(&mut CommentThread)) -> bool {
        let Some(mut t) = self.doc.comment(id).cloned() else {
            return false;
        };
        f(&mut t);
        if Some(&t) == self.doc.comment(id) {
            return false;
        }
        self.edit(EditKind::Other, |ed| ed.set_thread(id, Some(t)))
    }

    /// Replaces the body of thread `id`.
    pub fn edit_comment(&mut self, id: CommentId, body: &str) -> bool {
        let body = clean(body);
        self.modify_thread(id, |t| t.body = body)
    }

    /// Marks thread `id` resolved. Its anchors stay.
    pub fn resolve_comment(&mut self, id: CommentId) -> bool {
        self.modify_thread(id, |t| t.resolved = true)
    }

    /// Reopens a resolved thread.
    pub fn reopen_comment(&mut self, id: CommentId) -> bool {
        self.modify_thread(id, |t| t.resolved = false)
    }

    /// Adds a reply to thread `id`; returns the new reply's id.
    pub fn add_reply(
        &mut self,
        id: CommentId,
        body: &str,
        author: &str,
        now: Timestamp,
    ) -> Option<ReplyId> {
        let rid = self.doc.comment(id)?.next_reply_id();
        let reply = Reply {
            id: rid,
            body: clean(body),
            author: author.trim().to_string(),
            created_at: now,
        };
        self.modify_thread(id, |t| t.replies.push(reply)).then_some(rid)
    }

    /// Replaces the body of a reply.
    pub fn edit_reply(&mut self, id: CommentId, reply: ReplyId, body: &str) -> bool {
        let body = clean(body);
        self.modify_thread(id, |t| {
            if let Some(r) = t.replies.iter_mut().find(|r| r.id == reply) {
                r.body = body;
            }
        })
    }

    /// Deletes a reply.
    pub fn delete_reply(&mut self, id: CommentId, reply: ReplyId) -> bool {
        self.modify_thread(id, |t| t.replies.retain(|r| r.id != reply))
    }

    /// Deletes thread `id` and removes its anchors from the text. Undo
    /// restores both.
    pub fn delete_comment(&mut self, id: CommentId) -> bool {
        if self.doc.comment(id).is_none() {
            return false;
        }
        let blocks: Vec<usize> = if self.doc.anchor_counts.contains_key(&id) {
            self.doc
                .blocks
                .iter()
                .enumerate()
                .filter(|(_, b)| b.runs.iter().any(|r| r.attrs.comments.contains(&id)))
                .map(|(i, _)| i)
                .collect()
        } else {
            Vec::new()
        };
        let changed = self.edit(EditKind::Other, |ed| {
            for i in blocks {
                ed.update_block(i, |b| {
                    b.remove_comment(id);
                });
            }
            ed.set_thread(id, None);
        });
        if let Some(p) = &mut self.pending {
            p.comments.remove(&id);
        }
        changed
    }
}
