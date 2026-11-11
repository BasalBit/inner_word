//! Word counts and reading time.

use std::ops::{Add, AddAssign};

use unicode_segmentation::UnicodeSegmentation;

use crate::document::Document;
use crate::position::TextRange;

/// Reading speed used for [`Stats::reading_minutes`].
pub const WORDS_PER_MINUTE: usize = 230;

/// Text statistics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Words (Unicode word segmentation, segments containing a letter or digit).
    pub words: usize,
    /// User-perceived characters (grapheme clusters), including whitespace
    /// and soft line breaks, excluding the breaks between blocks.
    pub characters: usize,
    /// Grapheme clusters that are not whitespace.
    pub characters_no_spaces: usize,
    /// Non-void blocks containing non-whitespace text.
    pub paragraphs: usize,
    /// Estimated reading time in whole minutes, rounded up
    /// ([`WORDS_PER_MINUTE`]); 0 only for zero words.
    pub reading_minutes: usize,
}

impl Stats {
    /// Statistics of a single piece of text (counted as one paragraph if it
    /// has non-whitespace content).
    pub fn of_text(text: &str) -> Stats {
        let mut s = Stats::default();
        for g in text.graphemes(true) {
            s.characters += 1;
            if !g.chars().all(char::is_whitespace) {
                s.characters_no_spaces += 1;
            }
        }
        s.words = text.unicode_words().count();
        s.paragraphs = usize::from(s.characters_no_spaces > 0);
        s.finish()
    }

    /// Recomputes `reading_minutes` from `words`.
    pub(crate) fn finish(mut self) -> Stats {
        self.reading_minutes = self.words.div_ceil(WORDS_PER_MINUTE);
        self
    }
}

impl Add for Stats {
    type Output = Stats;
    fn add(self, o: Stats) -> Stats {
        Stats {
            words: self.words + o.words,
            characters: self.characters + o.characters,
            characters_no_spaces: self.characters_no_spaces + o.characters_no_spaces,
            paragraphs: self.paragraphs + o.paragraphs,
            reading_minutes: 0,
        }
        .finish()
    }
}

impl AddAssign for Stats {
    fn add_assign(&mut self, o: Stats) {
        *self = *self + o;
    }
}

impl Document {
    /// Statistics of the whole document (linear in its size).
    pub fn stats(&self) -> Stats {
        let mut s = Stats::default();
        for b in &self.blocks {
            s += Stats::of_text(&b.text);
        }
        s.finish()
    }

    /// Statistics of the text in `range`.
    pub fn stats_in(&self, range: TextRange) -> Stats {
        let r = TextRange::new(self.clamp_position(range.start), self.clamp_position(range.end));
        let mut s = Stats::default();
        for i in r.start.block..=r.end.block {
            let b = &self.blocks[i];
            let from = if i == r.start.block { r.start.offset } else { 0 };
            let to = if i == r.end.block { r.end.offset } else { b.len() };
            if from < to {
                s += Stats::of_text(&b.text[from..to]);
            }
        }
        s.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::{Block, BlockKind};
    use crate::position::Position;

    #[test]
    fn counts() {
        let d = Document::from_blocks(vec![
            Block::paragraph("Hello, world! It's 2 words... no, 5."),
            Block::new(BlockKind::Divider),
            Block::paragraph("   "),
            Block::paragraph("中文 e\u{301}"),
        ]);
        let s = d.stats();
        assert_eq!(s.paragraphs, 2);
        assert_eq!(s.words, 7 + 3);
        assert_eq!(s.reading_minutes, 1);
        let t = Stats::of_text("a b");
        assert_eq!((t.characters, t.characters_no_spaces, t.words), (3, 2, 2));
        let e = Stats::of_text("e\u{301}");
        assert_eq!(e.characters, 1);
        assert_eq!(Document::new().stats(), Stats::default());
        let sel = d.stats_in(TextRange::new(Position::new(0, 0), Position::new(0, 5)));
        assert_eq!(sel.words, 1);
        assert_eq!(sel.characters, 5);
        let long = Stats::of_text(&"word ".repeat(231));
        assert_eq!(long.reading_minutes, 2);
    }
}
