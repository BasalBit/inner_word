//! Clipboard fragments.

use crate::block::{Block, BlockId, BlockKind};
use crate::document::Document;
use crate::position::TextRange;
use crate::text;

/// A piece of rich text for the clipboard: a list of (possibly partial)
/// blocks with their kinds and inline attributes.
///
/// - A fragment copied from inside one block has one block; pasting it
///   inlines the text into the current block, which keeps its kind.
/// - A fragment spanning blocks has the tail of the first block, the whole
///   blocks in between, and the head of the last block (an empty block when
///   the copied range ended at offset 0 of a block).
///
/// **Comment anchors never travel in fragments**: copying drops them and
/// pasting strips any that are present, so a thread is never duplicated.
/// Links and all other attributes are kept. Fragment blocks carry
/// [`BlockId::UNASSIGNED`]; pasting mints fresh ids.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fragment {
    pub(crate) blocks: Vec<Block>,
}

impl Fragment {
    /// A fragment from blocks (repaired, ids cleared, anchors stripped).
    pub fn new(blocks: Vec<Block>) -> Fragment {
        let mut out = Vec::with_capacity(blocks.len());
        for mut b in blocks {
            let extra = b.repair();
            out.push(b);
            if let Some(e) = extra {
                out.push(e);
            }
        }
        for b in &mut out {
            b.id = BlockId::UNASSIGNED;
            b.strip_comments();
            b.revision = 0;
        }
        Fragment { blocks: out }
    }

    /// A fragment from plain text: each line becomes a paragraph. Empty text
    /// gives an empty fragment.
    pub fn from_plain_text(text: &str) -> Fragment {
        let text = text::normalize_input(text);
        if text.is_empty() {
            return Fragment::default();
        }
        Fragment {
            blocks: text.split('\n').map(Block::paragraph).collect(),
        }
    }

    /// The blocks of the fragment.
    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    /// Whether the fragment holds no text and no void block.
    pub fn is_empty(&self) -> bool {
        self.blocks.iter().all(|b| b.is_empty() && !b.is_void())
    }

    /// Plain text: blocks joined by `\n`; void blocks become empty lines.
    pub fn to_plain_text(&self) -> String {
        let mut out = String::new();
        for (i, b) in self.blocks.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            out.push_str(b.text());
        }
        out
    }

    /// Whether the fragment is a single non-void block (pasted inline).
    pub fn is_inline(&self) -> bool {
        self.blocks.len() == 1 && !self.blocks[0].is_void()
    }
}

impl Document {
    /// Copies `range` (clamped) as a [`Fragment`]. Comment anchors are dropped.
    pub fn fragment(&self, range: TextRange) -> Fragment {
        let s = self.clamp_position(range.start);
        let e = self.clamp_position(range.end);
        let (s, e) = if s <= e { (s, e) } else { (e, s) };
        if s == e {
            return Fragment::default();
        }
        let mut blocks = Vec::new();
        if s.block == e.block {
            blocks.push(self.blocks[s.block].slice(s.offset..e.offset));
        } else {
            let first = &self.blocks[s.block];
            blocks.push(first.slice(s.offset..first.len()));
            for b in &self.blocks[s.block + 1..e.block] {
                blocks.push(b.slice(0..b.len()));
            }
            let last = &self.blocks[e.block];
            if last.is_void() {
                // A range ending at a void block does not include it.
                blocks.push(Block::new(BlockKind::Paragraph));
            } else {
                blocks.push(last.slice(0..e.offset));
            }
        }
        for b in &mut blocks {
            b.strip_comments();
            b.revision = 0;
        }
        Fragment { blocks }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attrs::{CommentId, InlineAttrs};
    use crate::position::Position;

    #[test]
    fn plain_text_round_trip() {
        let f = Fragment::from_plain_text("a\r\nb\n");
        assert_eq!(f.blocks().len(), 3);
        assert_eq!(f.to_plain_text(), "a\nb\n");
        assert!(Fragment::from_plain_text("").is_empty());
        assert!(Fragment::from_plain_text("x").is_inline());
    }

    #[test]
    fn new_strips_anchors_and_ids() {
        let mut a = InlineAttrs::default();
        a.comments.insert(CommentId(1));
        let mut b = Block::paragraph("").with_span("x", a);
        b.id = BlockId(7);
        let f = Fragment::new(vec![b]);
        assert_eq!(f.blocks()[0].id(), BlockId::UNASSIGNED);
        assert!(f.blocks()[0].comment_ids().is_empty());
    }

    #[test]
    fn fragment_of_ranges() {
        let d = Document::from_blocks(vec![
            Block::with_text(BlockKind::heading(1), "Title"),
            Block::paragraph("body"),
            Block::new(BlockKind::Divider),
        ]);
        let f = d.fragment(TextRange::new(Position::new(0, 2), Position::new(1, 2)));
        assert_eq!(f.blocks().len(), 2);
        assert_eq!(f.blocks()[0].kind(), &BlockKind::heading(1));
        assert_eq!(f.to_plain_text(), "tle\nbo");
        let f = d.fragment(TextRange::new(Position::new(1, 0), Position::new(2, 0)));
        assert_eq!(f.to_plain_text(), "body\n");
        assert!(!f.blocks()[1].is_void());
        let f = d.fragment(TextRange::new(Position::new(1, 4), Position::new(1, 1)));
        assert_eq!(f.to_plain_text(), "ody");
    }
}
