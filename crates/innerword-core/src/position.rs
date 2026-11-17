//! Positions, ranges and selections.

/// A position in a document: a block index and a byte offset into that
/// block's text.
///
/// Valid positions are on char boundaries, and caret positions are also on
/// grapheme cluster boundaries. Void blocks have the single position offset 0.
/// Positions order by block, then offset.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Position {
    /// Index of the block in the document.
    pub block: usize,
    /// Byte offset into the block's text.
    pub offset: usize,
}

impl Position {
    /// Creates a position.
    pub const fn new(block: usize, offset: usize) -> Position {
        Position { block, offset }
    }

    /// The start of the document.
    pub const ZERO: Position = Position::new(0, 0);
}

/// An ordered range of positions, `start <= end`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TextRange {
    /// Inclusive start.
    pub start: Position,
    /// Exclusive end.
    pub end: Position,
}

impl TextRange {
    /// Creates a range from two positions in any order.
    pub fn new(a: Position, b: Position) -> TextRange {
        if a <= b {
            TextRange { start: a, end: b }
        } else {
            TextRange { start: b, end: a }
        }
    }

    /// A range inside one block.
    pub fn in_block(block: usize, start: usize, end: usize) -> TextRange {
        TextRange::new(Position::new(block, start), Position::new(block, end))
    }

    /// An empty range at `p`.
    pub fn empty(p: Position) -> TextRange {
        TextRange { start: p, end: p }
    }

    /// Whether `start == end`.
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// Whether `p` lies in `start..end` (start inclusive, end exclusive).
    pub fn contains(&self, p: Position) -> bool {
        self.start <= p && p < self.end
    }

    /// Whether the two ranges share at least one position strictly inside both.
    pub fn overlaps(&self, other: &TextRange) -> bool {
        self.start < other.end && other.start < self.end
    }
}

/// A selection: the fixed `anchor` and the moving `head` (where the caret is
/// drawn). A collapsed selection (`anchor == head`) is a caret.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Selection {
    /// The end that stays put when extending.
    pub anchor: Position,
    /// The end that moves; the caret.
    pub head: Position,
}

impl Selection {
    /// A selection from `anchor` to `head`.
    pub const fn new(anchor: Position, head: Position) -> Selection {
        Selection { anchor, head }
    }

    /// A caret at `p`.
    pub const fn caret(p: Position) -> Selection {
        Selection { anchor: p, head: p }
    }

    /// A selection covering `range`, head at the end.
    pub fn from_range(range: TextRange) -> Selection {
        Selection::new(range.start, range.end)
    }

    /// Whether this is a caret.
    pub fn is_collapsed(&self) -> bool {
        self.anchor == self.head
    }

    /// Whether the head is before the anchor.
    pub fn is_reversed(&self) -> bool {
        self.head < self.anchor
    }

    /// The earlier end.
    pub fn start(&self) -> Position {
        self.anchor.min(self.head)
    }

    /// The later end.
    pub fn end(&self) -> Position {
        self.anchor.max(self.head)
    }

    /// The ordered range covered.
    pub fn range(&self) -> TextRange {
        TextRange::new(self.anchor, self.head)
    }
}
