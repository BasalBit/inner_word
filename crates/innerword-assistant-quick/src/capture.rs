//! What the quick chat was opened on: the selection (or the caret) as it
//! was at that moment.
//!
//! Positions are block index plus byte offset and go stale with every
//! edit, so a capture also remembers the ids of the blocks it starts and
//! ends in and the text it covered. With those it can say, at any later
//! moment, where its text is now ([`Capture::resolve`]) and whether it is
//! still the text that was asked about ([`Capture::is_intact`]).

use innerword_core::{BlockId, Document, Position, TextRange};

/// How much of the selection the popover quotes.
pub const QUOTE_CHARS: usize = 120;

/// The selection of an editor at the moment the quick chat opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capture {
    /// The range as it was (ordered; empty for a caret).
    range: TextRange,
    /// The text in it, blocks joined by a line break.
    text: String,
    /// The block the range starts in.
    start_block: BlockId,
    /// The block the range ends in.
    end_block: BlockId,
}

impl Capture {
    /// Captures `range` of `document` (clamped and ordered).
    pub fn new(document: &Document, range: TextRange) -> Capture {
        let range = TextRange::new(
            document.clamp_position(range.start),
            document.clamp_position(range.end),
        );
        let id = |position: Position| {
            document
                .block(position.block)
                .map(|block| block.id())
                .unwrap_or(BlockId::UNASSIGNED)
        };
        Capture {
            range,
            text: document.text_in(range),
            start_block: id(range.start),
            end_block: id(range.end),
        }
    }

    /// Whether text was selected (otherwise this is a caret).
    pub fn has_selection(&self) -> bool {
        !self.range.is_empty()
    }

    /// The selected text, exactly: the blocks of a selection over several
    /// are joined by a line break. Empty for a caret.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The range as it was when the capture was taken.
    pub fn original_range(&self) -> TextRange {
        self.range
    }

    /// The selected text for showing: on one line, cut to
    /// [`QUOTE_CHARS`] characters with an ellipsis. `None` for a caret.
    pub fn quote(&self) -> Option<String> {
        self.has_selection().then(|| quote(&self.text, QUOTE_CHARS))
    }

    /// Where the captured range is in `document` now: the same offsets in
    /// the blocks it started and ended in, wherever those blocks have
    /// moved to. `None` when one of the blocks is gone or has become too
    /// short. This does not say that the text is unchanged (see
    /// [`Capture::is_intact`]).
    pub fn resolve(&self, document: &Document) -> Option<TextRange> {
        let start = position_in(document, self.start_block, self.range.start)?;
        let end = position_in(document, self.end_block, self.range.end)?;
        (start <= end).then(|| TextRange::new(start, end))
    }

    /// Where the captured text is now, when it still is the text that was
    /// captured. `None` for a caret and when anything in the range has
    /// changed.
    pub fn intact_range(&self, document: &Document) -> Option<TextRange> {
        if !self.has_selection() {
            return None;
        }
        let range = self.resolve(document)?;
        // The same number of blocks, and the same text in them.
        let blocks = |range: TextRange| range.end.block - range.start.block;
        (blocks(range) == blocks(self.range) && document.text_in(range) == self.text)
            .then_some(range)
    }

    /// Whether the captured text is still in the document as it was.
    pub fn is_intact(&self, document: &Document) -> bool {
        self.intact_range(document).is_some()
    }

    /// The index of the block the capture ends in, as the document is now.
    /// When that block is gone, `None`.
    pub fn end_block_index(&self, document: &Document) -> Option<usize> {
        document.block_index(self.end_block)
    }
}

/// `position` moved to wherever the block `id` is now.
fn position_in(document: &Document, id: BlockId, position: Position) -> Option<Position> {
    // The common case first: nothing moved.
    let index = match document.block(position.block) {
        Some(block) if block.id() == id => position.block,
        _ => document.block_index(id)?,
    };
    let block = document.block(index)?;
    (position.offset <= block.len() && block.text().is_char_boundary(position.offset))
        .then(|| Position::new(index, position.offset))
}

/// `text` on one line (every run of white space is one space) and cut to
/// `limit` characters, with an ellipsis where something was left out.
pub fn quote(text: &str, limit: usize) -> String {
    let mut out = String::new();
    let mut count = 0;
    let mut words = text.split_whitespace().peekable();
    while let Some(word) = words.next() {
        for character in word.chars() {
            if count == limit {
                let kept = out.trim_end().len();
                out.truncate(kept);
                out.push('…');
                return out;
            }
            out.push(character);
            count += 1;
        }
        if words.peek().is_some() {
            if count == limit {
                out.push('…');
                return out;
            }
            out.push(' ');
            count += 1;
        }
    }
    out
}
