//! Undo / redo history.
//!
//! Every transaction records the inverse of each primitive change it made.
//! There are only two primitives: replacing a range of blocks
//! ([`Op::Splice`]) and setting a comment thread ([`Op::Thread`]). Applying
//! an op returns its own inverse, so undo and redo are symmetric and restore
//! the exact blocks (ids and revision stamps included).

use std::collections::VecDeque;

use crate::attrs::CommentId;
use crate::block::Block;
use crate::comments::CommentThread;
use crate::position::Selection;
use crate::text::CharClass;

/// Default maximum number of undo steps.
pub const DEFAULT_UNDO_LIMIT: usize = 1000;

/// A primitive change. Stored in history as the inverse of what was done.
#[derive(Clone, Debug)]
pub(crate) enum Op {
    /// Replace `remove` blocks at `index` with `insert`.
    Splice {
        index: usize,
        remove: usize,
        insert: Vec<Block>,
    },
    /// Set (or delete, with `None`) a comment thread.
    Thread {
        id: CommentId,
        thread: Option<CommentThread>,
    },
}

/// What kind of command produced a transaction; consecutive transactions of
/// the same coalescing kind merge into one undo step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EditKind {
    Typing,
    Backspace,
    DeleteForward,
    Other,
}

impl EditKind {
    fn coalesces(self) -> bool {
        !matches!(self, EditKind::Other)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Entry {
    /// Inverse ops in the order they were recorded; undo applies them last
    /// to first.
    pub ops: Vec<Op>,
    pub before: Selection,
    pub after: Selection,
    pub kind: EditKind,
    pub first_class: Option<CharClass>,
    pub last_class: Option<CharClass>,
    /// Whether later transactions may still merge into this entry.
    pub open: bool,
}

/// Appends an inverse op, merging consecutive single-block replacements of
/// the same block (only the oldest saved version matters for undo).
pub(crate) fn push_op(ops: &mut Vec<Op>, op: Op) {
    if let (
        Some(Op::Splice {
            index: i0,
            remove: 1,
            insert: ins0,
        }),
        Op::Splice {
            index: i1,
            remove: 1,
            insert: ins1,
        },
    ) = (ops.last(), &op)
        && i0 == i1
        && ins0.len() == 1
        && ins1.len() == 1
    {
        return;
    }
    ops.push(op);
}

/// Whether a transaction starting with `first` must not merge into a group
/// whose last character was `last`: undo never gets coarser than a word.
fn breaks_word(kind: EditKind, last: Option<CharClass>, first: Option<CharClass>) -> bool {
    match (kind, last, first) {
        (EditKind::Typing, Some(CharClass::Space), Some(c)) => c != CharClass::Space,
        (EditKind::Backspace | EditKind::DeleteForward, Some(l), Some(CharClass::Space)) => {
            l != CharClass::Space
        }
        _ => false,
    }
}

#[derive(Clone, Debug)]
pub(crate) struct History {
    pub undo: VecDeque<Entry>,
    pub redo: Vec<Entry>,
    pub limit: usize,
    pub group_depth: usize,
    pub group_started: bool,
}

impl Default for History {
    fn default() -> History {
        History {
            undo: VecDeque::new(),
            redo: Vec::new(),
            limit: DEFAULT_UNDO_LIMIT,
            group_depth: 0,
            group_started: false,
        }
    }
}

impl History {
    /// Records a new transaction (clears redo).
    pub fn push(&mut self, mut entry: Entry) {
        self.redo.clear();
        if self.group_depth > 0 {
            if self.group_started
                && let Some(top) = self.undo.back_mut()
            {
                for op in entry.ops {
                    push_op(&mut top.ops, op);
                }
                top.after = entry.after;
                return;
            }
            entry.open = false;
            self.group_started = true;
            self.undo.push_back(entry);
            self.trim();
            return;
        }
        if let Some(top) = self.undo.back_mut() {
            if top.open
                && entry.kind.coalesces()
                && top.kind == entry.kind
                && top.after == entry.before
                && !breaks_word(entry.kind, top.last_class, entry.first_class)
            {
                for op in entry.ops {
                    push_op(&mut top.ops, op);
                }
                top.after = entry.after;
                if entry.last_class.is_some() {
                    top.last_class = entry.last_class;
                }
                return;
            }
            top.open = false;
        }
        entry.open = entry.kind.coalesces();
        self.undo.push_back(entry);
        self.trim();
    }

    /// Pushes an entry produced by redo (keeps the redo stack).
    pub fn push_redone(&mut self, mut entry: Entry) {
        entry.open = false;
        self.undo.push_back(entry);
        self.trim();
    }

    pub fn trim(&mut self) {
        let limit = self.limit.max(1);
        while self.undo.len() > limit {
            self.undo.pop_front();
        }
    }

    /// Stops the newest entry from absorbing later transactions.
    pub fn close(&mut self) {
        if let Some(top) = self.undo.back_mut() {
            top.open = false;
        }
    }

    pub fn begin_group(&mut self) {
        if self.group_depth == 0 {
            self.close();
            self.group_started = false;
        }
        self.group_depth += 1;
    }

    pub fn end_group(&mut self) {
        if self.group_depth > 0 {
            self.group_depth -= 1;
            if self.group_depth == 0 {
                self.close();
                self.group_started = false;
            }
        }
    }

    pub fn force_end_group(&mut self) {
        if self.group_depth > 0 {
            self.group_depth = 0;
            self.close();
            self.group_started = false;
        }
    }
}
