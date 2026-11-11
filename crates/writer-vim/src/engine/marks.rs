//! Marks and the jump list.
//!
//! Positions in a document are block indices, which shift whenever blocks
//! are inserted or removed, so marks are stored against the stable block id.
//! A mark whose block was deleted is gone (and comes back if undo restores
//! the block); an offset past the end of the block is clamped.

use writer_core::{BlockId, Document, Position};

use super::{Cx, Vim};

/// A position that survives edits elsewhere in the document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Mark {
    pub id: BlockId,
    pub offset: usize,
}

/// A mark that falls back on its plain position when its block is gone:
/// Vim's `'<` and `'>` stay on the same line number when the line they were
/// on is deleted, so `gv` after `Vd` selects the line that moved up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PlaceMark {
    pub mark: Mark,
    pub place: Position,
}

impl PlaceMark {
    pub fn at(doc: &Document, p: Position) -> PlaceMark {
        PlaceMark {
            mark: Mark::at(doc, p),
            place: doc.clamp_position(p),
        }
    }

    /// A mark that is only a place.
    pub fn placed(p: Position) -> PlaceMark {
        PlaceMark {
            mark: Mark {
                id: BlockId::default(),
                offset: p.offset,
            },
            place: p,
        }
    }

    /// The position; `None` when the block is gone and the place lies
    /// below the last block.
    pub fn resolve(&self, doc: &Document) -> Option<Position> {
        match self.mark.resolve(doc) {
            Some(p) => Some(p),
            None if self.place.block < doc.block_count() => Some(doc.clamp_position(self.place)),
            None => None,
        }
    }
}

impl Mark {
    pub fn at(doc: &Document, p: Position) -> Mark {
        let p = doc.clamp_position(p);
        Mark {
            id: doc.blocks()[p.block].id(),
            offset: p.offset,
        }
    }

    pub fn resolve(&self, doc: &Document) -> Option<Position> {
        let block = doc.block_index(self.id)?;
        Some(doc.clamp_position(Position::new(block, self.offset)))
    }
}

const MAX_JUMPS: usize = 100;

/// Vim's jump list: `ctrl-o` goes back, `ctrl-i` forward.
#[derive(Clone, Debug, Default)]
pub(crate) struct JumpList {
    entries: Vec<Mark>,
    /// Index of the current entry; `entries.len()` when not navigating.
    index: usize,
}

impl JumpList {
    /// Records the position a jump starts from.
    pub fn push(&mut self, doc: &Document, from: Position) {
        self.entries.push(Mark::at(doc, from));
        if self.entries.len() > MAX_JUMPS {
            let extra = self.entries.len() - MAX_JUMPS;
            self.entries.drain(..extra);
        }
        self.index = self.entries.len();
    }

    /// Drops entries whose block is gone, and all but the newest entry of
    /// each block (Vim keeps one entry per line, and tidies up only when
    /// the list is used).
    fn cleanup(&mut self, doc: &Document) {
        let at_end = self.index >= self.entries.len();
        let mut kept: Vec<Mark> = Vec::with_capacity(self.entries.len());
        let mut index = self.index;
        for (i, e) in self.entries.iter().enumerate() {
            if self.index == i {
                index = kept.len();
            }
            let gone = doc.block_index(e.id).is_none();
            let newer = self.entries[i + 1..].iter().any(|l| l.id == e.id);
            if !gone && !newer {
                kept.push(*e);
            }
        }
        self.entries = kept;
        self.index = if at_end {
            self.entries.len()
        } else {
            index.min(self.entries.len())
        };
    }

    /// Whether no `ctrl-o` is underway: the list is at its newest end.
    pub fn at_newest(&self) -> bool {
        self.index >= self.entries.len()
    }

    /// `ctrl-o`: the `count`th older position.
    pub fn back(&mut self, doc: &Document, current: Position, count: usize) -> Option<Position> {
        self.cleanup(doc);
        if self.index == self.entries.len() {
            // Remember where we are so ctrl-i can come back.
            self.entries.push(Mark::at(doc, current));
            self.index = self.entries.len() - 1;
        }
        if self.index < count {
            return None;
        }
        self.index -= count;
        self.entries[self.index].resolve(doc)
    }

    /// `ctrl-i`: the `count`th newer position.
    pub fn forward(&mut self, doc: &Document, count: usize) -> Option<Position> {
        self.cleanup(doc);
        if self.entries.is_empty() || self.index + count >= self.entries.len() {
            return None;
        }
        self.index += count;
        self.entries[self.index].resolve(doc)
    }
}

impl Vim {
    pub(crate) fn set_mark(&mut self, cx: &Cx, name: char, p: Position) {
        self.marks.insert(name, Mark::at(cx.ed.document(), p));
    }

    /// The position of mark `name`. `'` and `` ` `` are the same mark (the
    /// position before the last jump).
    pub(crate) fn mark_position(&self, cx: &Cx, name: char) -> Option<Position> {
        let name = if name == '`' { '\'' } else { name };
        self.marks.get(&name)?.resolve(cx.ed.document())
    }

    /// Records `from` as the origin of a jump: it becomes the `''` mark and
    /// the newest jump list entry.
    pub(crate) fn push_jump(&mut self, cx: &Cx, from: Position) {
        self.set_context_mark(cx, from);
        self.jumps.push(cx.ed.document(), from);
    }

    /// Sets the `''` mark, remembering the one before: a jump that ends
    /// where it started gets the old mark back (see `check_context_mark`).
    pub(crate) fn set_context_mark(&mut self, cx: &Cx, at: Position) {
        self.prev_context = self.marks.get(&'\'').copied();
        self.marks.insert('\'', Mark::at(cx.ed.document(), at));
    }

    /// After a command: the `''` mark never sits on the cursor itself. If
    /// the command set it there (a jump that did not move), the mark from
    /// before comes back, as in Vim.
    pub(crate) fn check_context_mark(&mut self, cx: &Cx) {
        let Some(prev) = self.prev_context else {
            return;
        };
        let doc = cx.ed.document();
        let cur = self.cursor(cx);
        if self.marks.get(&'\'').and_then(|m| m.resolve(doc)) == Some(cur) {
            self.marks.insert('\'', prev);
        }
    }
}
