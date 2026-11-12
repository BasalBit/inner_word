//! Finding a piece of text in a document: what an edit's `old_text` and a comment's
//! `quote` are matched against.
//!
//! The text of a document, for this purpose, is the text of its blocks with one line
//! break between two blocks (a soft line break inside a block is a line break too). A
//! match may run across blocks.

use writer_core::{Document, Position, TextRange, text};

use crate::error::{Error, Occurrence, Result};
use crate::markdown_in;
use crate::sections::heading_above;
use crate::types::MatchedAs;

/// How many occurrences an error or a listing names.
pub(crate) const SHOWN_OCCURRENCES: usize = 10;

/// The text of a document as one string, and the way back to positions.
pub(crate) struct Flat {
    text: String,
    /// The offset in `text` at which each block starts.
    starts: Vec<usize>,
}

impl Flat {
    /// The text of `document`.
    pub(crate) fn of(document: &Document) -> Flat {
        let mut starts = Vec::with_capacity(document.block_count());
        let mut at = 0;
        for block in document.blocks() {
            starts.push(at);
            at += block.len() + 1;
        }
        Flat {
            text: document.plain_text(),
            starts,
        }
    }

    /// The position of offset `offset` of the text. An offset on the line break between
    /// two blocks is the end of the first.
    fn position(&self, offset: usize) -> Position {
        let block = self.starts.partition_point(|&start| start <= offset) - 1;
        Position::new(block, offset - self.starts[block])
    }

    /// Every place where `needle` stands, left to right, without overlaps. A match starts
    /// and ends between two characters as a reader sees them (never inside a letter with
    /// its accent, or an emoji made of several code points).
    fn find(&self, document: &Document, needle: &str) -> Vec<TextRange> {
        let mut found = Vec::new();
        if needle.is_empty() {
            return found;
        }
        let mut from = 0;
        while from <= self.text.len() {
            let Some(at) = self.text[from..].find(needle) else {
                break;
            };
            let start = self.position(from + at);
            let end = self.position(from + at + needle.len());
            let whole =
                text::is_grapheme_boundary(document.blocks()[start.block].text(), start.offset)
                    && text::is_grapheme_boundary(document.blocks()[end.block].text(), end.offset);
            if whole {
                found.push(TextRange::new(start, end));
                from += at + needle.len();
            } else {
                from = text::ceil_char_boundary(&self.text, from + at + 1);
            }
        }
        found
    }
}

/// Where a piece of text was found.
pub(crate) struct Matches {
    /// The places, in the order of the document.
    pub(crate) ranges: Vec<TextRange>,
    /// How the text was found. `None` when it was not.
    pub(crate) matched_as: Option<MatchedAs>,
}

/// Looks for `wanted` in `document`: as it stands, and when that finds nothing and
/// `literal` is not set, read as Markdown (so a piece copied from the Markdown form of
/// the document finds its place). Line breaks at the ends of `wanted` do not count.
pub(crate) fn find(document: &Document, wanted: &str, literal: bool) -> Result<Matches> {
    let normalized = text::normalize_input(wanted);
    let needle = normalized.trim_matches('\n');
    if needle.is_empty() {
        return Err(Error::invalid("the text to look for is empty"));
    }
    let flat = Flat::of(document);
    let ranges = flat.find(document, needle);
    if !ranges.is_empty() {
        return Ok(Matches {
            ranges,
            matched_as: Some(MatchedAs::Text),
        });
    }
    if !literal {
        let read = markdown_in::import(needle, document.settings());
        let as_text = read
            .blocks()
            .iter()
            .map(|block| block.text())
            .collect::<Vec<_>>()
            .join("\n");
        let as_text = as_text.trim_matches('\n');
        if !as_text.is_empty() && as_text != needle {
            let ranges = flat.find(document, as_text);
            if !ranges.is_empty() {
                return Ok(Matches {
                    ranges,
                    matched_as: Some(MatchedAs::Markdown),
                });
            }
        }
    }
    Ok(Matches {
        ranges: Vec::new(),
        matched_as: None,
    })
}

/// The matches a request means: the only one, the one `occurrence` numbers (from 1), or
/// with `all` every one. Fails with [`Error::NoMatch`] and [`Error::NotUnique`].
pub(crate) fn choose(
    document: &Document,
    wanted: &str,
    matches: &Matches,
    all: bool,
    occurrence: Option<usize>,
) -> Result<Vec<TextRange>> {
    let count = matches.ranges.len();
    if count == 0 {
        return Err(Error::NoMatch {
            text: wanted.to_string(),
            hint: near_miss(document, wanted),
        });
    }
    if let Some(number) = occurrence {
        return match matches.ranges.get(number.wrapping_sub(1)) {
            Some(range) => Ok(vec![*range]),
            None => Err(Error::invalid(format!(
                "the text occurs {count} time{} in the document, so there is no occurrence \
                 {number}",
                if count == 1 { "" } else { "s" }
            ))),
        };
    }
    if all || count == 1 {
        return Ok(matches.ranges.clone());
    }
    Err(Error::NotUnique {
        text: wanted.to_string(),
        count,
        occurrences: occurrences(document, &matches.ranges, SHOWN_OCCURRENCES),
    })
}

/// The first `limit` matches as data: number, block, heading and the line around each.
pub(crate) fn occurrences(
    document: &Document,
    ranges: &[TextRange],
    limit: usize,
) -> Vec<Occurrence> {
    ranges
        .iter()
        .take(limit)
        .enumerate()
        .map(|(index, range)| Occurrence {
            occurrence: index + 1,
            block: range.start.block,
            heading: heading_above(document, range.start.block),
            context: context(document, range),
        })
        .collect()
}

/// The line a match is on, shortened around the match.
fn context(document: &Document, range: &TextRange) -> String {
    const AROUND: usize = 40;
    let block = document.blocks()[range.start.block].text();
    let start = range.start.offset.min(block.len());
    let end = if range.end.block == range.start.block {
        range.end.offset.min(block.len())
    } else {
        block.len()
    };
    let before: Vec<char> = block[..start].chars().collect();
    let after: Vec<char> = block[end..].chars().collect();
    let mut line = String::new();
    if before.len() > AROUND {
        line.push('\u{2026}');
    }
    line.extend(&before[before.len().saturating_sub(AROUND)..]);
    line.push_str(&block[start..end]);
    if range.end.block != range.start.block {
        line.push('\u{23ce}');
        line.push('\u{2026}');
    } else {
        line.extend(&after[..after.len().min(AROUND)]);
        if after.len() > AROUND {
            line.push('\u{2026}');
        }
    }
    line.replace('\n', "\u{23ce}")
}

/// For a text that was not found: the place where it stands with other quotation marks,
/// dashes, spaces or capitals, when there is one.
fn near_miss(document: &Document, wanted: &str) -> Option<String> {
    fn fold(c: char) -> char {
        match c {
            '\u{2018}' | '\u{2019}' | '\u{201a}' | '\u{2032}' => '\'',
            '\u{201c}' | '\u{201d}' | '\u{201e}' | '\u{2033}' => '"',
            '\u{2010}'..='\u{2015}' | '\u{2212}' => '-',
            c if c.is_whitespace() => ' ',
            c => c.to_lowercase().next().unwrap_or(c),
        }
    }
    let needle: String = wanted.trim().chars().map(fold).collect();
    if needle.is_empty() {
        return None;
    }
    let text = document.plain_text();
    let folded: String = text.chars().map(fold).collect();
    let at = folded.find(&needle)?;
    // Every character folds to one character: count characters to find the place in the
    // text as it is.
    let first = folded[..at].chars().count();
    let length = needle.chars().count();
    let found: String = text.chars().skip(first).take(length).collect();
    Some(format!(
        "The document has it with other quotation marks, dashes, spaces or capitals: {}. \
         Give the text exactly as it stands there",
        crate::error::quoted_excerpt(&found)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use writer_core::{Block, BlockKind, InlineAttrs, InlineStyle};

    fn sample() -> Document {
        Document::from_blocks(vec![
            Block::with_text(BlockKind::heading(1), "Harbour"),
            Block::paragraph("The ")
                .with_span("Maren", InlineAttrs::default().with(InlineStyle::Italic))
                .with_span(" left at dawn. The gulls left too.", InlineAttrs::default()),
            Block::paragraph("It\u{2019}s caf\u{e9} time, e\u{301}t\u{e9}."),
            Block::with_text(BlockKind::BulletItem, "one"),
            Block::with_text(BlockKind::BulletItem, "two"),
        ])
    }

    #[test]
    fn text_is_found_as_it_stands_and_across_blocks() {
        let document = sample();
        let found = find(&document, "left", false).unwrap();
        assert_eq!(found.matched_as, Some(MatchedAs::Text));
        assert_eq!(
            found.ranges,
            [
                TextRange::in_block(1, 10, 14),
                TextRange::in_block(1, 34, 38)
            ]
        );
        // Across two blocks, and with the line breaks an agent leaves at the ends.
        let found = find(&document, "\nHarbour\nThe Maren\n", false).unwrap();
        assert_eq!(
            found.ranges,
            [TextRange::new(Position::new(0, 0), Position::new(1, 9))]
        );
        assert!(find(&document, "Left", false).unwrap().ranges.is_empty());
        assert!(matches!(
            find(&document, "\n", false),
            Err(Error::Invalid { .. })
        ));
        // Never inside a letter and its accent.
        assert!(find(&document, ", e", false).unwrap().ranges.is_empty());
        assert_eq!(find(&document, "caf\u{e9}", false).unwrap().ranges.len(), 1);
    }

    #[test]
    fn markdown_is_read_when_the_text_does_not_stand_as_given() {
        let document = sample();
        let found = find(&document, "The *Maren* left", false).unwrap();
        assert_eq!(found.matched_as, Some(MatchedAs::Markdown));
        assert_eq!(found.ranges, [TextRange::in_block(1, 0, 14)]);
        let found = find(&document, "- one\n- two", false).unwrap();
        assert_eq!(found.matched_as, Some(MatchedAs::Markdown));
        assert_eq!(
            found.ranges,
            [TextRange::new(Position::new(3, 0), Position::new(4, 3))]
        );
        // A blank line between two paragraphs is one line break in the document.
        let found = find(&document, "# Harbour\n\nThe *Maren*", false).unwrap();
        assert_eq!(found.ranges.len(), 1);
        // Taken literally, none of this is there.
        let found = find(&document, "The *Maren* left", true).unwrap();
        assert!(found.ranges.is_empty());
        assert_eq!(found.matched_as, None);
    }

    #[test]
    fn one_match_or_a_choice() {
        let document = sample();
        let two = find(&document, "left", false).unwrap();
        match choose(&document, "left", &two, false, None) {
            Err(Error::NotUnique {
                count, occurrences, ..
            }) => {
                assert_eq!(count, 2);
                assert_eq!(occurrences[0].occurrence, 1);
                assert_eq!(occurrences[0].block, 1);
                assert_eq!(occurrences[0].heading.as_deref(), Some("Harbour"));
                assert_eq!(
                    occurrences[1].context,
                    "The Maren left at dawn. The gulls left too."
                );
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            choose(&document, "left", &two, true, None).unwrap().len(),
            2
        );
        assert_eq!(
            choose(&document, "left", &two, false, Some(2)).unwrap(),
            [TextRange::in_block(1, 34, 38)]
        );
        assert!(matches!(
            choose(&document, "left", &two, false, Some(3)),
            Err(Error::Invalid { .. })
        ));
        assert!(matches!(
            choose(&document, "left", &two, false, Some(0)),
            Err(Error::Invalid { .. })
        ));
        let none = find(&document, "It's cafe", false).unwrap();
        assert!(matches!(
            choose(&document, "It's cafe", &none, false, None),
            Err(Error::NoMatch { hint: None, .. })
        ));
        let none = find(&document, "it's caf\u{e9}  time", false).unwrap();
        match choose(&document, "it's caf\u{e9}", &none, false, None) {
            Err(Error::NoMatch {
                hint: Some(hint), ..
            }) => {
                assert!(hint.contains("It\u{2019}s caf\u{e9}"), "{hint}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn long_lines_are_shortened_around_the_match() {
        let document = Document::from_blocks(vec![Block::paragraph(&format!(
            "{}needle{}",
            "a ".repeat(100),
            " b".repeat(100)
        ))]);
        let found = find(&document, "needle", false).unwrap();
        let shown = occurrences(&document, &found.ranges, 5);
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].context.chars().count(), 40 + 6 + 40 + 2);
        assert!(shown[0].context.starts_with('\u{2026}'));
        assert!(shown[0].context.ends_with('\u{2026}'));
        assert_eq!(shown[0].heading, None);
    }
}
