//! The panel's view of a document's comment threads, as plain data.
//!
//! [`collect_threads`] turns a [`Document`] into the ordered list the panel
//! shows. It has no GPUI in it, so the ordering, quoting and "detached"
//! rules can be read (and tested) on their own.

use std::collections::BTreeMap;

use innerword_core::{CommentId, CommentThread, Document, Position, TextRange, text};

/// The longest quote shown above a thread, in characters.
pub const QUOTE_MAX_CHARS: usize = 80;

/// One comment thread as the panel shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct ThreadEntry {
    /// The thread id.
    pub id: CommentId,
    /// Every range of text the thread is anchored to, in document order.
    /// Empty when the commented text was deleted.
    pub ranges: Vec<TextRange>,
    /// The anchored text, on one line, at most [`QUOTE_MAX_CHARS`] long.
    /// Empty for a detached thread.
    pub quote: String,
    /// The thread itself: body, author, time, resolved flag, replies.
    pub thread: CommentThread,
}

impl ThreadEntry {
    /// The first anchor range: where navigation puts the caret and where
    /// the editor scrolls to. `None` for a detached thread.
    pub fn anchor(&self) -> Option<TextRange> {
        self.ranges.first().copied()
    }

    /// Whether the commented text was deleted (the thread has no anchor).
    pub fn is_detached(&self) -> bool {
        self.ranges.is_empty()
    }

    /// Whether the thread is resolved.
    pub fn is_resolved(&self) -> bool {
        self.thread.resolved
    }

    /// Whether `position` is inside or at either end of one of the anchor
    /// ranges.
    pub fn contains(&self, position: Position) -> bool {
        self.ranges
            .iter()
            .any(|range| range.start <= position && position <= range.end)
    }
}

/// Every thread of `document`: anchored threads in document order of their
/// first anchor, then detached threads in creation order.
pub fn collect_threads(document: &Document) -> Vec<ThreadEntry> {
    if document.comment_count() == 0 {
        return Vec::new();
    }
    let mut order: Vec<CommentId> = Vec::new();
    let mut ranges: BTreeMap<CommentId, Vec<TextRange>> = BTreeMap::new();
    for (id, range) in document.all_comment_ranges() {
        let entry = ranges.entry(id).or_default();
        if entry.is_empty() {
            order.push(id);
        }
        entry.push(range);
    }

    let mut threads = Vec::with_capacity(document.comment_count());
    for id in order {
        let Some(thread) = document.comment(id) else {
            continue;
        };
        let ranges = ranges.remove(&id).unwrap_or_default();
        threads.push(ThreadEntry {
            id,
            quote: quote(document, &ranges),
            ranges,
            thread: thread.clone(),
        });
    }
    let anchored = threads.len();
    for thread in document.comments() {
        if threads[..anchored]
            .iter()
            .all(|entry| entry.id != thread.id)
        {
            threads.push(ThreadEntry {
                id: thread.id,
                ranges: Vec::new(),
                quote: String::new(),
                thread: thread.clone(),
            });
        }
    }
    threads
}

/// The text of `ranges` as a one-line quote: white space collapsed, the
/// pieces of a split anchor joined with an ellipsis, cut at
/// [`QUOTE_MAX_CHARS`] characters.
pub fn quote(document: &Document, ranges: &[TextRange]) -> String {
    let mut joined = String::new();
    for range in ranges {
        let piece = document.text_in(*range);
        let piece = piece.split_whitespace().collect::<Vec<_>>().join(" ");
        if piece.is_empty() {
            continue;
        }
        if !joined.is_empty() {
            joined.push_str(" … ");
        }
        joined.push_str(&piece);
        if joined.chars().count() > QUOTE_MAX_CHARS {
            break;
        }
    }
    truncate(&joined, QUOTE_MAX_CHARS)
}

/// `text` cut to at most `max_chars` characters, ending in an ellipsis when
/// something was cut.
pub fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut cut: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    let trimmed = cut.trim_end().len();
    cut.truncate(trimmed);
    cut.push('…');
    cut
}

/// The word a collapsed caret at `position` is in or right after: what
/// "add comment" anchors to when nothing is selected. `None` when the caret
/// touches no word (white space, punctuation, an empty or void block).
pub fn word_range_at(document: &Document, position: Position) -> Option<TextRange> {
    let position = document.clamp_position(position);
    let block = document.block(position.block)?;
    let mut before = None;
    for word in text::word_ranges(block.text()) {
        if word.start > position.offset {
            break;
        }
        if position.offset < word.end {
            return Some(TextRange::in_block(position.block, word.start, word.end));
        }
        if word.end == position.offset {
            before = Some(word);
        }
    }
    before.map(|word| TextRange::in_block(position.block, word.start, word.end))
}
