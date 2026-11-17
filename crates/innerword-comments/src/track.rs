//! Following a range of text through edits of the document.
//!
//! The target of a comment that is still being written is a plain
//! [`TextRange`]: nothing in the document marks it, so nothing moves it when
//! the text around it changes. A [`RangeTracker`] does that from outside.
//! It remembers the text of the blocks around both ends of the range, and
//! after every change of the document ([`RangeTracker::update`], with the
//! block splice the editor reports) it works out where the ends are now:
//!
//! - a change in other blocks only shifts block indices;
//! - a change in the blocks at an end is compared as text (what stayed the
//!   same before and after the changed stretch), which covers typing,
//!   deleting, splitting and joining paragraphs;
//! - text typed right before or right after the range stays outside it;
//! - when all of the range's text is deleted, the range collapses to the
//!   place where it was, and comes back when that text returns (an undo).
//!
//! No GPUI in here.

use innerword_core::{BlockId, BlockSplice, Document, Position, TextRange};

/// A [`TextRange`] that follows its text through edits. See the
/// [module documentation](self).
#[derive(Clone, Debug)]
pub struct RangeTracker {
    range: TextRange,
    /// The text of the range, as of the last update (`Document::text_in`).
    text: String,
    /// The text the range had before it collapsed (all of it was deleted).
    lost: Option<String>,
    /// The blocks at and next to both ends of the range, before the next
    /// change.
    snapshot: Vec<Snap>,
}

#[derive(Clone, Debug)]
struct Snap {
    index: usize,
    id: BlockId,
    text: String,
}

#[derive(Clone, Copy, PartialEq)]
enum End {
    Start,
    End,
}

/// What changed around one block, as two strings: the blocks of the changed
/// stretch before and after, joined by `\n`, and where they differ.
struct Diff {
    old: String,
    new: String,
    /// Index (before the change) of the block `old` starts with.
    old_first: usize,
    /// Index (after the change) of the block `new` starts with.
    new_first: usize,
    /// Length of the common prefix of `old` and `new`.
    prefix: usize,
    /// End of the changed stretch in `old`.
    old_end: usize,
    /// End of the changed stretch in `new`.
    new_end: usize,
    /// The lengths of the blocks `new` was joined from.
    new_lens: Vec<usize>,
}

impl RangeTracker {
    /// Starts following `range` (clamped) in `document`.
    pub fn new(document: &Document, range: TextRange) -> Self {
        let mut tracker = Self {
            range,
            text: String::new(),
            lost: None,
            snapshot: Vec::new(),
        };
        tracker.settle(document);
        tracker
    }

    /// Where the range is now. Empty when its text was deleted.
    pub fn range(&self) -> TextRange {
        self.range
    }

    /// The text of the range (blocks joined by `\n`), as of the last update.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Whether the text of the range was deleted: the range is collapsed at
    /// the place where the text was.
    pub fn is_lost(&self) -> bool {
        self.range.is_empty()
    }

    /// Brings the range up to date after a change of the document.
    /// `document` is the document after the change, `splice` the blocks the
    /// change replaced (`Change::blocks`; several changes composed with
    /// `BlockSplice::then`), `None` when no block changed.
    pub fn update(&mut self, document: &Document, splice: Option<BlockSplice>) {
        if let Some(splice) = splice {
            if self.range.is_empty() {
                // The text is gone: stay where it was, and take the range
                // back when the text returns.
                let at = self.map(document, splice, self.range.start, End::Start, true);
                self.range = self
                    .lost
                    .as_deref()
                    .and_then(|lost| self.find(document, splice, lost))
                    .unwrap_or(TextRange::new(at, at));
            } else {
                let start = self.map(document, splice, self.range.start, End::Start, false);
                let end = self.map(document, splice, self.range.end, End::End, false);
                self.range = if start < end {
                    TextRange::new(start, end)
                } else {
                    // The change swallowed both ends. A large change (an
                    // undo of several steps) may still contain the text.
                    self.find(document, splice, &self.text)
                        .unwrap_or(TextRange::new(end, end))
                };
            }
        }
        self.settle(document);
    }

    /// Clamps the range to `document` and remembers the text around it.
    fn settle(&mut self, document: &Document) {
        let start = document.clamp_position(self.range.start);
        let end = document.clamp_position(self.range.end);
        self.range = TextRange::new(start.min(end), start.max(end));
        if self.range.is_empty() {
            if !self.text.is_empty() {
                self.lost = Some(std::mem::take(&mut self.text));
            }
        } else {
            self.text = document.text_in(self.range);
            self.lost = None;
        }

        let blocks = document.blocks();
        let mut indices: Vec<usize> = Vec::with_capacity(6);
        for block in [self.range.start.block, self.range.end.block] {
            for index in block.saturating_sub(1)..=block + 1 {
                if index < blocks.len() && !indices.contains(&index) {
                    indices.push(index);
                }
            }
        }
        indices.sort_unstable();
        self.snapshot = indices
            .into_iter()
            .map(|index| Snap {
                index,
                id: blocks[index].id(),
                text: blocks[index].text().to_string(),
            })
            .collect();
    }

    fn snap(&self, index: usize) -> Option<&Snap> {
        self.snapshot.iter().find(|snap| snap.index == index)
    }

    /// Where `position` (before the change) is after it.
    fn map(
        &self,
        document: &Document,
        splice: BlockSplice,
        position: Position,
        end: End,
        collapsed: bool,
    ) -> Position {
        let block = position.block;
        if block < splice.start {
            return position;
        }
        if block >= splice.start + splice.old_len {
            return Position::new(block - splice.old_len + splice.new_len, position.offset);
        }
        let Some(diff) = self.diff(document, splice, block) else {
            // Nothing to compare with: the start of what changed.
            return document.clamp_position(Position::new(splice.start, 0));
        };
        let offset = diff.old_offset(self, position);
        let mapped = diff.map_offset(offset, end, collapsed);
        diff.position(document, mapped)
    }

    /// The changed stretch around `block` (an index before the change),
    /// before and after.
    fn diff(&self, document: &Document, splice: BlockSplice, block: usize) -> Option<Diff> {
        let blocks = document.blocks();
        let old = self.snap(block)?;
        let new_range =
            splice.start.min(blocks.len())..(splice.start + splice.new_len).min(blocks.len());
        let same_block = || {
            new_range
                .clone()
                .find(|index| blocks[*index].id() == old.id)
        };
        let single = |index: usize| {
            let new = blocks[index].text();
            Diff::new(
                old.text.clone(),
                new.to_string(),
                block,
                index,
                vec![new.len()],
            )
        };
        // The same blocks, changed in place or reordered: follow the block.
        if splice.old_len == splice.new_len
            && let Some(index) = same_block()
        {
            return Some(single(index));
        }
        // Paragraphs split, joined, removed or added: compare the whole
        // stretch as one text.
        if !new_range.is_empty() && splice.old_range().all(|index| self.snap(index).is_some()) {
            let old = splice
                .old_range()
                .filter_map(|index| self.snap(index))
                .map(|snap| snap.text.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            let new: Vec<&str> = new_range
                .clone()
                .map(|index| blocks[index].text())
                .collect();
            let new_lens = new.iter().map(|text| text.len()).collect();
            return Some(Diff::new(
                old,
                new.join("\n"),
                splice.start,
                splice.start,
                new_lens,
            ));
        }
        // A change larger than what is remembered: the block itself, if it
        // is still there.
        same_block().map(single)
    }

    /// Where `text` (the text of the range) is in what the change put
    /// into the document, if it is there.
    fn find(&self, document: &Document, splice: BlockSplice, lost: &str) -> Option<TextRange> {
        if lost.is_empty() {
            return None;
        }
        let block = self.range.start.block;
        if !splice.old_range().contains(&block) {
            return None;
        }
        let diff = self.diff(document, splice, block)?;
        let inserted = diff.new.get(diff.prefix..diff.new_end)?;
        // Prefer the place where the range collapsed.
        let at = diff
            .old_offset(self, self.range.start)
            .clamp(diff.prefix, diff.new_end)
            - diff.prefix;
        let found = match inserted.get(at..) {
            Some(rest) if rest.starts_with(lost) => at,
            _ => inserted.find(lost)?,
        };
        let start = diff.prefix + found;
        Some(TextRange::new(
            diff.position(document, start),
            diff.position(document, start + lost.len()),
        ))
    }
}

impl Diff {
    fn new(
        old: String,
        new: String,
        old_first: usize,
        new_first: usize,
        new_lens: Vec<usize>,
    ) -> Self {
        let mut prefix = old
            .bytes()
            .zip(new.bytes())
            .take_while(|(a, b)| a == b)
            .count();
        while !old.is_char_boundary(prefix) {
            prefix -= 1;
        }
        let mut suffix = old.as_bytes()[prefix..]
            .iter()
            .rev()
            .zip(new.as_bytes()[prefix..].iter().rev())
            .take_while(|(a, b)| a == b)
            .count();
        while !old.is_char_boundary(old.len() - suffix) {
            suffix -= 1;
        }
        Self {
            prefix,
            old_end: old.len() - suffix,
            new_end: new.len() - suffix,
            old,
            new,
            old_first,
            new_first,
            new_lens,
        }
    }

    /// `position` (before the change) as an offset into `old`.
    fn old_offset(&self, tracker: &RangeTracker, position: Position) -> usize {
        let before: usize = (self.old_first..position.block)
            .filter_map(|index| tracker.snap(index))
            .map(|snap| snap.text.len() + 1)
            .sum();
        (before + position.offset).min(self.old.len())
    }

    /// An offset into `old` as an offset into `new`.
    ///
    /// Outside the changed stretch the text is the same and the offset only
    /// shifts. At its edges: text put right before the range's start or
    /// right after its end stays outside the range; a replacement that
    /// begins at the start or stops at the end is part of it. Inside the
    /// changed stretch, the start moves behind it and the end before it:
    /// what was replaced is no longer commented text.
    fn map_offset(&self, offset: usize, end: End, collapsed: bool) -> usize {
        let (prefix, old_end, new_end) = (self.prefix, self.old_end, self.new_end);
        if offset < prefix {
            return offset;
        }
        if offset > old_end {
            return offset - old_end + new_end;
        }
        let inserted_only = old_end == prefix;
        if collapsed {
            return prefix;
        }
        match end {
            End::Start => {
                if offset == prefix && !inserted_only {
                    prefix
                } else {
                    new_end
                }
            }
            End::End => {
                if offset == old_end && !inserted_only {
                    new_end
                } else {
                    prefix
                }
            }
        }
    }

    /// An offset into `new` as a position in the document after the change.
    fn position(&self, document: &Document, offset: usize) -> Position {
        let mut rest = offset.min(self.new.len());
        let last = self.new_lens.len().saturating_sub(1);
        for (ix, len) in self.new_lens.iter().enumerate() {
            if rest <= *len || ix == last {
                return document.clamp_position(Position::new(self.new_first + ix, rest.min(*len)));
            }
            rest -= len + 1;
        }
        document.clamp_position(Position::new(self.new_first, rest))
    }
}
