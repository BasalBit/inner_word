//! The matches of the engine's search pattern, kept in step with the
//! document so the editor's highlight layer always shows the current text.
//!
//! The editor's highlight ranges do not follow edits, so they are set again
//! after every change. Searching the whole document on each keystroke would
//! cost milliseconds in a book-length document; instead only the blocks a
//! change touched are searched again and the ranges after them are shifted.

use innerword_core::{BlockSplice, Change, Document, TextRange};
use innerword_vim::SearchPattern;

/// The highlighted pattern and where it matches.
#[derive(Default)]
pub(crate) struct SearchMatches {
    pattern: Option<SearchPattern>,
    /// Sorted by start; every range lies inside one block.
    ranges: Vec<TextRange>,
    /// The document revision the ranges are for.
    revision: u64,
}

impl SearchMatches {
    /// The pattern being highlighted.
    pub(crate) fn pattern(&self) -> Option<&SearchPattern> {
        self.pattern.as_ref()
    }

    /// The matches, sorted by start.
    pub(crate) fn ranges(&self) -> &[TextRange] {
        &self.ranges
    }

    /// Highlights `pattern` (or nothing) in `document`.
    pub(crate) fn set(
        &mut self,
        pattern: Option<SearchPattern>,
        document: &Document,
        revision: u64,
    ) {
        self.pattern = pattern;
        self.recompute(document, revision);
    }

    /// Searches the whole document again.
    fn recompute(&mut self, document: &Document, revision: u64) {
        self.ranges.clear();
        self.revision = revision;
        let Some(pattern) = &self.pattern else {
            return;
        };
        for index in 0..document.block_count() {
            push_block_matches(&mut self.ranges, pattern, document, index);
        }
    }

    /// Brings the matches up to date after the editor reported `change`.
    /// `revision` is the document's revision now. Returns whether the
    /// ranges may have changed.
    pub(crate) fn document_changed(
        &mut self,
        change: &Change,
        document: &Document,
        revision: u64,
    ) -> bool {
        if self.pattern.is_none() || self.revision == revision {
            return false;
        }
        match change.blocks {
            // The latest change, on top of ranges that were current before
            // it: touch only what it touched.
            Some(splice) if change.revision == revision => self.splice(splice, document),
            // An older report (more changes followed before it arrived):
            // start over from the document as it is now.
            _ => self.recompute(document, revision),
        }
        self.revision = revision;
        true
    }

    fn splice(&mut self, splice: BlockSplice, document: &Document) {
        let Some(pattern) = self.pattern.clone() else {
            return;
        };
        let old = splice.old_range();
        let new = splice.new_range();
        let head = self
            .ranges
            .partition_point(|range| range.start.block < old.start);
        let tail = self
            .ranges
            .partition_point(|range| range.start.block < old.end);
        let shift = |block: usize| block - old.end + new.end;
        let moved: Vec<TextRange> = self.ranges[tail..]
            .iter()
            .map(|range| {
                TextRange::in_block(
                    shift(range.start.block),
                    range.start.offset,
                    range.end.offset,
                )
            })
            .collect();
        self.ranges.truncate(head);
        for index in new {
            push_block_matches(&mut self.ranges, &pattern, document, index);
        }
        self.ranges.extend(moved);
    }
}

fn push_block_matches(
    ranges: &mut Vec<TextRange>,
    pattern: &SearchPattern,
    document: &Document,
    index: usize,
) {
    if index >= document.block_count() {
        return;
    }
    ranges.extend(
        pattern
            .matches_in_block(document, index)
            .into_iter()
            .map(|found| TextRange::in_block(index, found.start, found.end)),
    );
}
