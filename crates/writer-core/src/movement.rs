//! Caret movements that need no layout.

use crate::document::Document;
use crate::position::Position;
use crate::text;

/// A caret movement that can be computed without layout. Vertical movement
/// and visual-row movement are the UI's job; it computes the target and calls
/// [`EditorState::set_selection`](crate::EditorState::set_selection).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Movement {
    /// One grapheme left, crossing into the end of the previous block.
    Left,
    /// One grapheme right, crossing into the start of the next block.
    Right,
    /// macOS option-left: to the start of the current or previous word,
    /// crossing blocks when no word precedes the caret in its block.
    WordLeft,
    /// macOS option-right: to the end of the current or next word,
    /// crossing blocks when no word follows the caret in its block.
    WordRight,
    /// Start of the hard line (after the previous soft break) in the block.
    LineStart,
    /// End of the hard line (before the next soft break) in the block.
    LineEnd,
    /// Start of the block.
    BlockStart,
    /// End of the block.
    BlockEnd,
    /// Start of the document.
    DocStart,
    /// End of the document.
    DocEnd,
}

impl Movement {
    /// Whether the movement goes towards the start of the document. Used to
    /// decide which end of a selection a non-extending move starts from.
    pub fn is_backward(self) -> bool {
        matches!(
            self,
            Movement::Left
                | Movement::WordLeft
                | Movement::LineStart
                | Movement::BlockStart
                | Movement::DocStart
        )
    }
}

impl Document {
    /// Where `movement` takes a caret at `pos` (clamped first).
    pub fn moved(&self, pos: Position, movement: Movement) -> Position {
        let pos = self.clamp_position(pos);
        let blocks = &self.blocks;
        let b = &blocks[pos.block];
        match movement {
            Movement::Left => {
                if pos.offset > 0 {
                    Position::new(pos.block, text::prev_grapheme_boundary(&b.text, pos.offset))
                } else if pos.block > 0 {
                    Position::new(pos.block - 1, blocks[pos.block - 1].len())
                } else {
                    pos
                }
            }
            Movement::Right => {
                if pos.offset < b.len() {
                    Position::new(pos.block, text::next_grapheme_boundary(&b.text, pos.offset))
                } else if pos.block + 1 < blocks.len() {
                    Position::new(pos.block + 1, 0)
                } else {
                    pos
                }
            }
            Movement::WordLeft => {
                let mut i = pos.block;
                let mut off = pos.offset;
                loop {
                    if let Some(s) = text::word_start_before(&blocks[i].text, off) {
                        return Position::new(i, s);
                    }
                    if i == 0 {
                        return Position::ZERO;
                    }
                    i -= 1;
                    off = blocks[i].len();
                }
            }
            Movement::WordRight => {
                let mut i = pos.block;
                let mut off = pos.offset;
                loop {
                    if let Some(e) = text::word_end_after(&blocks[i].text, off) {
                        return Position::new(i, e);
                    }
                    if i + 1 >= blocks.len() {
                        return self.end();
                    }
                    i += 1;
                    off = 0;
                }
            }
            Movement::LineStart => Position::new(pos.block, text::line_start(&b.text, pos.offset)),
            Movement::LineEnd => Position::new(pos.block, text::line_end(&b.text, pos.offset)),
            Movement::BlockStart => Position::new(pos.block, 0),
            Movement::BlockEnd => Position::new(pos.block, b.len()),
            Movement::DocStart => Position::ZERO,
            Movement::DocEnd => self.end(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::{Block, BlockKind};

    fn doc() -> Document {
        Document::from_blocks(vec![
            Block::paragraph("Hello, wo\u{301}rld"),
            Block::new(BlockKind::Divider),
            Block::paragraph("  "),
            Block::paragraph("next line\nsoft"),
        ])
    }

    #[test]
    fn grapheme_left_right_cross_blocks() {
        let d = doc();
        let p = |b, o| Position::new(b, o);
        assert_eq!(d.moved(p(0, 0), Movement::Left), p(0, 0));
        let len0 = d.blocks()[0].len();
        assert_eq!(d.moved(p(0, len0), Movement::Right), p(1, 0));
        assert_eq!(d.moved(p(1, 0), Movement::Right), p(2, 0));
        assert_eq!(d.moved(p(1, 0), Movement::Left), p(0, len0));
        assert_eq!(d.moved(p(2, 0), Movement::Left), p(1, 0));
        // "o\u{301}" is one grapheme at offset 8..11
        assert_eq!(d.moved(p(0, 8), Movement::Right), p(0, 11));
        assert_eq!(d.moved(p(0, 11), Movement::Left), p(0, 8));
        assert_eq!(d.moved(d.end(), Movement::Right), d.end());
    }

    #[test]
    fn word_moves() {
        let d = doc();
        let p = |b, o| Position::new(b, o);
        assert_eq!(d.moved(p(0, 0), Movement::WordRight), p(0, 5));
        assert_eq!(
            d.moved(p(0, 5), Movement::WordRight),
            p(0, d.blocks()[0].len())
        );
        // From the end of block 0 it skips the divider and the blank block.
        let len0 = d.blocks()[0].len();
        assert_eq!(d.moved(p(0, len0), Movement::WordRight), p(3, 4));
        assert_eq!(d.moved(p(3, 0), Movement::WordLeft), p(0, 7));
        assert_eq!(d.moved(p(0, 3), Movement::WordLeft), p(0, 0));
        assert_eq!(d.moved(d.end(), Movement::WordRight), d.end());
    }

    #[test]
    fn line_and_block_moves() {
        let d = doc();
        let p = |b, o| Position::new(b, o);
        assert_eq!(d.moved(p(3, 12), Movement::LineStart), p(3, 10));
        assert_eq!(d.moved(p(3, 2), Movement::LineEnd), p(3, 9));
        assert_eq!(d.moved(p(3, 2), Movement::BlockEnd), p(3, 14));
        assert_eq!(d.moved(p(3, 12), Movement::BlockStart), p(3, 0));
        assert_eq!(d.moved(p(2, 1), Movement::DocStart), Position::ZERO);
        assert_eq!(d.moved(p(0, 1), Movement::DocEnd), d.end());
        // Out of range input is clamped.
        assert_eq!(d.moved(p(99, 99), Movement::BlockStart), p(3, 0));
    }
}
