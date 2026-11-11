//! Helpers shared by the exporters.

use std::collections::BTreeSet;

use writer_core::{Block, BlockKind, CommentId, CommentThread, Document, TextRange};

use crate::{ExportPart, ExportSource, PartTitles};

/// Whether `part`'s title should be written as a heading above its text.
pub(crate) fn part_needs_title(
    source: &ExportSource<'_>,
    part: &ExportPart<'_>,
    mode: PartTitles,
) -> bool {
    if !source.is_collection() {
        return false;
    }
    match mode {
        PartTitles::Never => false,
        PartTitles::Always => true,
        PartTitles::WhenMissing => !shows_title(part.document, &part.title),
    }
}

fn fold(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Whether a heading of `doc`, or its first line of text, starts with
/// `title` (case and whitespace insensitive).
fn shows_title(doc: &Document, title: &str) -> bool {
    let title = fold(title);
    if title.is_empty() {
        return true;
    }
    let first_line = |b: &Block| fold(b.text().lines().find(|l| !l.trim().is_empty()).unwrap_or(""));
    let mut first_text = true;
    for b in doc.blocks() {
        if b.text().trim().is_empty() {
            continue;
        }
        let is_heading = b.kind().heading_level().is_some();
        if (is_heading || first_text) && first_line(b).starts_with(&title) {
            return true;
        }
        first_text = false;
    }
    false
}

/// The first anchor range of every anchored comment thread, in document
/// order. A thread anchored on several separate ranges is exported once, on
/// its first range. Orphaned threads have no range and are not listed.
pub(crate) fn first_comment_ranges(doc: &Document) -> Vec<(CommentId, TextRange)> {
    let mut seen = BTreeSet::new();
    doc.all_comment_ranges()
        .into_iter()
        .filter(|(id, _)| seen.insert(*id))
        .collect()
}

/// The entries of a thread as `(author, body)`: the opening comment, then
/// the replies in order.
pub(crate) fn thread_entries(thread: &CommentThread) -> Vec<(&str, &str)> {
    let mut v = vec![(thread.author.trim(), thread.body.as_str())];
    v.extend(thread.replies.iter().map(|r| (r.author.trim(), r.body.as_str())));
    v
}

/// The lines of `text` with spaces and tabs trimmed from both ends of every
/// line and empty lines removed from the start and the end.
pub(crate) fn trimmed_lines(text: &str) -> Vec<&str> {
    let mut lines: Vec<&str> = text
        .split('\n')
        .map(|l| l.trim_matches(|c| c == ' ' || c == '\t'))
        .collect();
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    let lead = lines.iter().take_while(|l| l.is_empty()).count();
    lines.drain(..lead);
    lines
}

/// Whether the block is a list item of any kind.
pub(crate) fn is_list_item(block: &Block) -> bool {
    block.kind().is_list_item()
}

/// The numbered-list ordinal of every block of `doc`.
pub(crate) fn ordinals(doc: &Document) -> Vec<Option<usize>> {
    doc.numbered_ordinals(0..doc.block_count())
}

/// Whether the block has no visible text (void blocks are not "blank").
pub(crate) fn is_blank_text(block: &Block) -> bool {
    !block.is_void() && !matches!(block.kind(), BlockKind::CodeBlock { .. }) && block.text().trim().is_empty()
}

/// Civil date and time (UTC) of a timestamp in milliseconds since the Unix
/// epoch: `(year, month, day, hour, minute, second)`.
pub(crate) fn civil_from_millis(ms: i64) -> (i64, u32, u32, u32, u32, u32) {
    let secs = ms.div_euclid(1000);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400) as u32;
    // Howard Hinnant's days-to-civil algorithm.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day, rem / 3600, rem % 3600 / 60, rem % 60)
}

/// `2026-01-31T09:05:00Z` for a timestamp in milliseconds since the epoch.
/// Years are clamped to `0001..=9999` so the result is always a valid W3CDTF
/// date.
pub(crate) fn iso_datetime(ms: i64) -> String {
    const MIN: i64 = -62_135_596_800_000; // 0001-01-01T00:00:00Z
    const MAX: i64 = 253_402_300_799_000; // 9999-12-31T23:59:59Z
    let (y, mo, d, h, mi, s) = civil_from_millis(ms.clamp(MIN, MAX));
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(iso_datetime(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_datetime(951_782_400_000), "2000-02-29T00:00:00Z");
        assert_eq!(iso_datetime(1_794_391_505_999), "2026-11-11T10:05:05Z");
        assert_eq!(iso_datetime(-1), "1969-12-31T23:59:59Z");
        assert_eq!(iso_datetime(i64::MIN), "0001-01-01T00:00:00Z");
        assert_eq!(iso_datetime(i64::MAX), "9999-12-31T23:59:59Z");
    }

    #[test]
    fn lines() {
        assert_eq!(trimmed_lines("  a \n\n b\t\n \n"), vec!["a", "", "b"]);
        assert_eq!(trimmed_lines(" \n \n"), Vec::<&str>::new());
        assert_eq!(trimmed_lines("\u{a0}x"), vec!["\u{a0}x"]);
    }

    #[test]
    fn titles() {
        let doc = Document::from_blocks(vec![
            Block::paragraph(""),
            Block::paragraph("Opening  Line and more"),
            Block::with_text(BlockKind::heading(2), "Chapter One"),
            Block::paragraph("Body"),
        ]);
        assert!(shows_title(&doc, "chapter one"));
        assert!(shows_title(&doc, "Opening line"));
        assert!(!shows_title(&doc, "Body"));
        assert!(!shows_title(&doc, "Something else"));
        assert!(shows_title(&doc, "  "));
    }
}
