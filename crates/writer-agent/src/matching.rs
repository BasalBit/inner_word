//! Finding a piece of text in a document: what an edit's `old_text` and a comment's
//! `quote` are matched against.
//!
//! The text of a document, for this purpose, is the text of its blocks with one line
//! break between two blocks (a soft line break inside a block is a line break too). A
//! match may run across blocks.
//!
//! An image is a block without text: an empty line in that text. So a text that names
//! the paragraph before an image and the one after it, with an empty line between them,
//! is found across the image, and replacing it would take the image away without a word
//! about it. An edit therefore asks [`across_an_image`] of every match it is about to
//! replace. A text read as Markdown that names the image where it stands
//! (`![…](image:<id>)`) is found there too, and only there: then the image is part of
//! what was asked for.

use writer_core::{Block, Document, ImageId, Position, TextRange, text};

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
        let read = markdown_in::read_against(needle, document);
        let blocks = read.blocks();
        let as_text = blocks
            .iter()
            .map(|block| block.text())
            .collect::<Vec<_>>()
            .join("\n");
        let as_text = as_text.trim_matches('\n');
        let named = Named::of(blocks);
        let ranges: Vec<TextRange> = if as_text.is_empty() {
            // Nothing but image lines: the places where those images stand in a row.
            rows(document, &named.before)
        } else if as_text != needle {
            flat.find(document, as_text)
                .into_iter()
                .filter_map(|range| named.at(document, range))
                .collect()
        } else {
            Vec::new()
        };
        if !ranges.is_empty() {
            return Ok(Matches {
                ranges,
                matched_as: Some(MatchedAs::Markdown),
            });
        }
    }
    Ok(Matches {
        ranges: Vec::new(),
        matched_as: None,
    })
}

/// The pictures of the image blocks among `blocks`, in order.
fn images_of(blocks: &[Block]) -> Vec<ImageId> {
    blocks
        .iter()
        .filter_map(|block| block.as_image().map(|image| image.id))
        .collect()
}

/// The pictures of the image blocks a match runs across, in order. (A match of text
/// never begins or ends on one: an image has no text, and the text that is looked for
/// has no line break at its ends.)
fn images_in(document: &Document, range: &TextRange) -> Vec<ImageId> {
    let last = range.end.block.min(document.block_count() - 1);
    images_of(&document.blocks()[range.start.block.min(last)..=last])
}

/// A block without text, as a text that was read as Markdown names it: the image with
/// this picture, or (`None`) anything else without text (a divider, an empty line).
type Line = Option<ImageId>;

fn line_of(block: &Block) -> Line {
    block.as_image().map(|image| image.id)
}

/// Whether the blocks of `document` from `at` on are `lines`, one for one.
fn stand(document: &Document, at: usize, lines: &[Line]) -> bool {
    lines.iter().enumerate().all(|(offset, line)| {
        document
            .blocks()
            .get(at + offset)
            .is_some_and(|block| block.is_empty() && line_of(block) == *line)
    })
}

/// The places where `lines` stand as blocks in a row: each from the first of them to
/// the last.
fn rows(document: &Document, lines: &[Line]) -> Vec<TextRange> {
    let mut found = Vec::new();
    let mut at = 0;
    while !lines.is_empty() && at + lines.len() <= document.block_count() {
        if stand(document, at, lines) {
            found.push(TextRange::new(
                Position::new(at, 0),
                Position::new(at + lines.len() - 1, 0),
            ));
            at += lines.len();
        } else {
            at += 1;
        }
    }
    found
}

/// The images a text that was read as Markdown names, by where they stand in it.
struct Named {
    /// The blocks without text before its first line of text, from the first image
    /// among them on (none when there is no image there: empty lines at the ends of a
    /// text do not count). For a text without any line of text: from its first image
    /// to its last.
    before: Vec<Line>,
    /// The images between its lines of text.
    inside: Vec<ImageId>,
    /// The blocks without text after its last line of text, up to the last image
    /// among them.
    after: Vec<Line>,
}

impl Named {
    fn of(blocks: &[Block]) -> Named {
        let first = blocks.iter().position(|block| !block.is_empty());
        let last = blocks.iter().rposition(|block| !block.is_empty());
        let (head, core, tail) = match (first, last) {
            (Some(first), Some(last)) => {
                (&blocks[..first], &blocks[first..=last], &blocks[last + 1..])
            }
            _ => (blocks, &blocks[..0], &blocks[..0]),
        };
        // Without a line of text the images are a row of their own: from the first
        // to the last.
        let head = match (
            first,
            head.iter().rposition(|block| block.kind().is_image()),
        ) {
            (None, Some(last_image)) => &head[..=last_image],
            _ => head,
        };
        let before = match head.iter().position(|block| block.kind().is_image()) {
            Some(image) => head[image..].iter().map(line_of).collect(),
            None => Vec::new(),
        };
        let after = match tail.iter().rposition(|block| block.kind().is_image()) {
            Some(image) => tail[..=image].iter().map(line_of).collect(),
            None => Vec::new(),
        };
        Named {
            before,
            inside: images_of(core),
            after,
        }
    }

    /// The match `range` of the text's own lines as a match of the whole text, the
    /// image lines at its ends included; `None` when the images are not there as the
    /// text names them. Exactly the images the text names between its lines stand inside
    /// the match (so a text that names none is never found across one), and the image
    /// lines at its ends are the blocks right before and right after it.
    fn at(&self, document: &Document, mut range: TextRange) -> Option<TextRange> {
        if images_in(document, &range) != self.inside {
            return None;
        }
        if !self.before.is_empty() {
            let first = range.start.block.checked_sub(self.before.len())?;
            if range.start.offset != 0 || !stand(document, first, &self.before) {
                return None;
            }
            range.start = Position::new(first, 0);
        }
        if !self.after.is_empty() {
            let last = range.end.block;
            let whole = range.end.offset == document.blocks()[last].len();
            if !whole || !stand(document, last + 1, &self.after) {
                return None;
            }
            range.end = Position::new(last + self.after.len(), 0);
        }
        Some(range)
    }
}

/// Whether replacing the match `range` would take an image with it that the text of the
/// match does not name: the match was found as plain text (`matched_as`) and an image
/// stands inside it. What was found as Markdown names every image in it, and no other
/// stands there ([`find`]).
pub(crate) fn across_an_image(
    document: &Document,
    range: &TextRange,
    matched_as: Option<MatchedAs>,
) -> bool {
    matched_as != Some(MatchedAs::Markdown) && !images_in(document, range).is_empty()
}

/// What an edit is told when its old text runs across an image.
pub(crate) const ACROSS_AN_IMAGE: &str = "the text to replace runs across an image; read the place again and replace the text \
     before and after the image separately, or name the image as it reads in Markdown";

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

    /// A document with two pictures: `[Harbour] [A] [One.] [---] [B] [B] [Two.] [A]`.
    fn pictures() -> (Document, String, String) {
        use writer_core::{Image, ImageId};
        let id = |digit: char| ImageId::parse(&digit.to_string().repeat(32)).unwrap();
        let image = |digit: char| Block::image(Image::new(id(digit), (40, 30), 72));
        let document = Document::from_blocks(vec![
            Block::with_text(BlockKind::heading(1), "Harbour"),
            image('a'),
            Block::paragraph("One."),
            Block::new(BlockKind::Divider),
            image('b'),
            image('b'),
            Block::paragraph("Two."),
            image('a'),
        ]);
        let line = |digit: char| format!("![](image:{})", id(digit));
        (document, line('a'), line('b'))
    }

    #[test]
    fn an_image_is_an_empty_line_and_a_text_that_names_it_is_found_where_it_stands() {
        let (document, a, b) = pictures();
        let at = |first: (usize, usize), last: (usize, usize)| {
            TextRange::new(
                Position::new(first.0, first.1),
                Position::new(last.0, last.1),
            )
        };
        let found = |wanted: &str| find(&document, wanted, false).unwrap();

        // As plain text, the lines before and after an image are found across it: the
        // empty line between them is the image. An edit asks, and refuses.
        let across = found("Harbour\n\nOne.");
        assert_eq!(across.matched_as, Some(MatchedAs::Text));
        assert_eq!(across.ranges, [at((0, 0), (2, 4))]);
        assert!(across_an_image(
            &document,
            &across.ranges[0],
            across.matched_as
        ));
        let beside = found("One.");
        assert!(!across_an_image(
            &document,
            &beside.ranges[0],
            beside.matched_as
        ));

        // As Markdown that names the image between its lines: found, and for an edit
        // to replace.
        let named = found(&format!("# Harbour\n\n{a}\n\nOne."));
        assert_eq!(named.matched_as, Some(MatchedAs::Markdown));
        assert_eq!(named.ranges, [at((0, 0), (2, 4))]);
        assert!(!across_an_image(
            &document,
            &named.ranges[0],
            named.matched_as
        ));
        // Another image, no image, or something else without text in its place: not there.
        for other in [
            format!("# Harbour\n\n{b}\n\nOne."),
            "# Harbour\n\n---\n\nOne.".to_string(),
            format!("# Harbour\n\n{a}\n\n{a}\n\nOne."),
        ] {
            assert_eq!(found(&other).ranges, [], "{other}");
        }
        // Markdown that names no image is never found across one.
        assert_eq!(found("**One.**\n\n---\n\n---\n\n---\n\nTwo.").ranges, []);

        // The image lines a text begins or ends with are the blocks right beside its
        // own lines, and the match runs from the first to the last of them.
        assert_eq!(found(&format!("{a}\n\nOne.")).ranges, [at((1, 0), (2, 4))]);
        assert_eq!(
            found(&format!("# Harbour\n\n{a}")).ranges,
            [at((0, 0), (1, 0))]
        );
        assert_eq!(
            found(&format!("One.\n\n---\n\n{b}\n\n{b}")).ranges,
            [at((2, 0), (5, 0))]
        );
        assert_eq!(
            found(&format!("{b}\n\nTwo.\n\n{a}")).ranges,
            [at((5, 0), (7, 0))]
        );
        for not_beside in [
            // The image is not the block before, a divider is between, or the text
            // does not begin or end where its block does.
            format!("{b}\n\nOne."),
            format!("One.\n\n{b}"),
            format!("{a}\n\nne."),
            format!("# Harb\n\n{a}"),
            format!("{a}\n\n{a}\n\nOne."),
        ] {
            assert_eq!(found(&not_beside).ranges, [], "{not_beside}");
        }

        // Image lines and nothing else: the image blocks themselves, wherever they
        // stand in that row.
        let alone = found(&a);
        assert_eq!(alone.matched_as, Some(MatchedAs::Markdown));
        assert_eq!(alone.ranges, [at((1, 0), (1, 0)), at((7, 0), (7, 0))]);
        assert_eq!(found(&b).ranges, [at((4, 0), (4, 0)), at((5, 0), (5, 0))]);
        assert_eq!(found(&format!("{b}\n\n{b}")).ranges, [at((4, 0), (5, 0))]);
        assert_eq!(found(&format!("{b}\n\n{a}")).ranges, []);
        // What is no image and has no text is not looked for (as before images).
        assert_eq!(found("---").ranges, []);
        // Taken literally, the line is the characters it is, which stand nowhere.
        assert_eq!(find(&document, &a, true).unwrap().ranges, []);
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
