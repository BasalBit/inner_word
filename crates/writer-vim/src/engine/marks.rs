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
        let m = Mark::at(doc, from);
        // One entry per block: a newer one replaces the older.
        self.entries.retain(|e| e.id != m.id);
        self.entries.push(m);
        if self.entries.len() > MAX_JUMPS {
            let extra = self.entries.len() - MAX_JUMPS;
            self.entries.drain(..extra);
        }
        self.index = self.entries.len();
    }

    /// `ctrl-o`: the `count`th older position.
    pub fn back(&mut self, doc: &Document, current: Position, count: usize) -> Option<Position> {
        self.entries.retain(|e| doc.block_index(e.id).is_some());
        self.index = self.index.min(self.entries.len());
        if self.index == self.entries.len() {
            // Remember where we are so ctrl-i can come back.
            let m = Mark::at(doc, current);
            if self.entries.last().is_none_or(|e| e.id != m.id) {
                self.entries.retain(|e| e.id != m.id);
                self.entries.push(m);
            } else if let Some(last) = self.entries.last_mut() {
                *last = m;
            }
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
        self.entries.retain(|e| doc.block_index(e.id).is_some());
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
        let doc = cx.ed.document();
        self.marks.insert('\'', Mark::at(doc, from));
        self.jumps.push(doc, from);
    }
}
