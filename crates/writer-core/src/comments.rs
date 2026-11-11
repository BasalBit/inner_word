//! Comment threads.
//!
//! A thread is stored once in the document; its anchor is the
//! [`InlineAttrs::comments`](crate::InlineAttrs::comments) attribute on the
//! commented text, so the anchor moves with edits, survives splits and
//! merges, and comes back with undo. A thread whose anchored text was all
//! deleted is kept and reported as orphaned
//! ([`Document::is_orphaned`](crate::Document::is_orphaned)).

use crate::attrs::CommentId;

/// Milliseconds since the Unix epoch. The core never reads a clock: callers
/// pass timestamps in.
pub type Timestamp = i64;

/// Identifier of a reply, unique within its thread.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReplyId(pub u64);

/// A reply in a comment thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply {
    /// Id, unique within the thread.
    pub id: ReplyId,
    /// Reply text (plain text, may contain `\n`).
    pub body: String,
    /// Author display name.
    pub author: String,
    /// Creation time.
    pub created_at: Timestamp,
}

/// A comment thread: the opening comment plus replies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommentThread {
    /// Thread id; the anchor attribute refers to it.
    pub id: CommentId,
    /// Body of the opening comment.
    pub body: String,
    /// Author of the opening comment.
    pub author: String,
    /// Creation time of the opening comment.
    pub created_at: Timestamp,
    /// Whether the thread is resolved. Resolved threads keep their anchors.
    pub resolved: bool,
    /// Replies in order.
    pub replies: Vec<Reply>,
}

impl CommentThread {
    /// The reply with `id`.
    pub fn reply(&self, id: ReplyId) -> Option<&Reply> {
        self.replies.iter().find(|r| r.id == id)
    }

    /// The next free reply id.
    pub(crate) fn next_reply_id(&self) -> ReplyId {
        ReplyId(self.replies.iter().map(|r| r.id.0).max().unwrap_or(0).saturating_add(1))
    }

    /// Makes reply ids unique and non-zero.
    pub(crate) fn repair(&mut self) {
        let mut seen = std::collections::BTreeSet::new();
        let valid = |id: u64| id != 0 && id < crate::document::MAX_ID;
        let mut next = self
            .replies
            .iter()
            .map(|r| r.id.0)
            .filter(|&id| valid(id))
            .max()
            .unwrap_or(0)
            + 1;
        for r in &mut self.replies {
            if !valid(r.id.0) || !seen.insert(r.id) {
                r.id = ReplyId(next);
                seen.insert(r.id);
                next += 1;
            }
        }
    }
}
