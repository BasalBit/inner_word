//! Spelling marks: the seam a spell checker plugs into.
//!
//! The editor does not check spelling. A checker (innerword-spell) reads the
//! text of a block, asks its service which words are misspelled and hands
//! the editor the byte ranges of those words for that block **and the
//! block revision it read**. The editor keeps the ranges per block, draws
//! a wavy underline in the theme's `spelling` colour under them, keeps
//! them in step with edits until the checker's next result replaces them,
//! and answers the questions a menu or a "next misspelling" command asks.
//!
//! ```ignore
//! // The checker, for one block (`index` is a block index):
//! let (id, revision, text) = editor.read(cx).document().block(index)
//!     .map(|block| (block.id(), block.revision(), block.text().to_string()))?;
//! let misspelled: Vec<Range<usize>> = service.check(&text, language); // byte ranges
//! editor.update(cx, |editor, cx| {
//!     match editor.set_spelling_marks(id, revision, misspelled, cx) {
//!         SpellingMarksOutcome::Applied => {}
//!         // The block was edited (or went) while the answer was on its
//!         // way: nothing was stored. Check the block again.
//!         SpellingMarksOutcome::Stale | SpellingMarksOutcome::Missing => recheck(id),
//!     }
//! });
//!
//! // What to check, cheaply, also in a book of 10,000 blocks:
//! let visible = editor.visible_range().map(|range| range.start.block..range.end.block + 1);
//! let next = editor.first_unchecked_spelling_block(visible)      // what is on screen first
//!     .or_else(|| editor.first_unchecked_spelling_block(cursor..count)); // then the rest
//! // A block that is skipped (a code block, a divider) gets an empty
//! // result, so it counts as checked:
//! editor.set_spelling_marks(id, revision, [], cx);
//!
//! // On EditorEvent::Edited { change }: the blocks of `change.blocks`'s new
//! // range are unchecked again (their marks have followed the edit and are
//! // still drawn); check them after the pause you want.
//!
//! // Menus and commands:
//! editor.spelling_mark_at(position)           // the marked word the caret is in or at
//! editor.next_spelling_mark(selection.end())  // wraps around the document
//! editor.previous_spelling_mark(selection.start())
//! editor.spelling_mark_count()
//! // Language changed, a word was learned: every block is checked again,
//! // the marks stay on screen meanwhile.
//! editor.invalidate_spelling_marks();
//! // Checking turned off:
//! editor.clear_spelling_marks(cx);            // or set_spelling_marks_visible(false, cx)
//! ```
//!
//! # Between a keystroke and the next result
//!
//! Every change of a block makes its result *unchecked*
//! ([`Editor::spelling_checked`] is false, also after an undo that brings
//! an old revision back) but keeps its marks, moved with the text:
//!
//! - a mark behind the changed text shifts with it, a mark before it stays;
//! - a mark the change reaches into is dropped (the word is being edited);
//! - a mark the change only touches at one end is dropped when the
//!   character that now adjoins it there continues the word (a letter, a
//!   digit, an apostrophe, a combining mark) and kept when it does not (a
//!   space or a full stop typed after a marked word leaves it marked);
//! - when a block is split, the marks behind the split go to the new block;
//!   when two blocks are joined, the marks of the second go to the first;
//!   when a block goes, its marks go.
//!
//! Marks never point outside the text or into the middle of a character.
//!
//! # Cost
//!
//! One hash map entry per checked block, plus a copy of the text of the
//! blocks that have marks (what the marks are moved by when the block
//! changes). Setting a result, the queries at a position and drawing cost
//! the block they are about; an edit costs the blocks it changed;
//! [`Editor::next_spelling_mark`] and its twin cost a lookup per block
//! until they find one (about 0.2 ms across 10,000 blocks without a mark).

use std::{cell::Cell, collections::HashMap, collections::HashSet, ops::Range};

use gpui::{Context, Pixels, Point, px};
use innerword_core::{Block, BlockId, BlockSplice, Document, Position, TextRange};

use crate::editor::Editor;

/// What [`Editor::set_spelling_marks`] did with a result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpellingMarksOutcome {
    /// The marks of the block are now the given ones, and the block counts
    /// as checked at its current revision.
    Applied,
    /// The block has another revision than the result was made for: it was
    /// edited meanwhile. Nothing was stored; check the block again.
    Stale,
    /// The document has no block with that id (any more). Nothing was
    /// stored.
    Missing,
}

impl SpellingMarksOutcome {
    /// Whether the result was stored.
    pub fn is_applied(self) -> bool {
        self == SpellingMarksOutcome::Applied
    }
}

/// The marks of one block.
struct Entry {
    /// The block revision the checker's result was made for. `None` once
    /// the marks have only been carried along by an edit.
    checked: Option<u64>,
    /// The block revision the ranges fit.
    revision: u64,
    /// Sorted, apart from each other, never empty, on character
    /// boundaries of the block's text at `revision`.
    ranges: Vec<Range<usize>>,
    /// The block's text at `revision`, kept while there are ranges: what
    /// the ranges are moved by when the block changes.
    text: Option<Box<str>>,
}

impl Entry {
    /// Brings the entry to the block as it is now.
    fn follow(&mut self, block: &Block) {
        if let Some(old) = self.text.take() {
            follow_edit(&mut self.ranges, &old, block.text());
        }
        self.text = (!self.ranges.is_empty()).then(|| block.text().into());
        self.revision = block.revision();
        self.checked = None;
    }
}

/// All spelling marks of an editor.
pub(crate) struct SpellingMarks {
    entries: HashMap<BlockId, Entry>,
    /// The ids of the document's blocks, in order: how an edit's removed
    /// blocks are known, and how a block is found by its id.
    order: Vec<BlockId>,
    /// Number of ranges in all entries.
    count: usize,
    /// Whether the marks are drawn.
    pub(crate) visible: bool,
    /// Where the last block was found by id: the checker goes through the
    /// blocks in order.
    hint: Cell<usize>,
    /// The blocks of the last frame.
    pub(crate) on_screen: Range<usize>,
}

impl SpellingMarks {
    pub(crate) fn new(document: &Document) -> Self {
        Self {
            entries: HashMap::new(),
            order: document.blocks().iter().map(Block::id).collect(),
            count: 0,
            visible: true,
            hint: Cell::new(0),
            on_screen: 0..0,
        }
    }

    /// Starts over for a replaced document. The drawing switch stays.
    pub(crate) fn reset(&mut self, document: &Document) {
        self.entries.clear();
        self.count = 0;
        self.order.clear();
        self.order.extend(document.blocks().iter().map(Block::id));
        self.hint.set(0);
    }

    /// Drops every mark and every result. Returns whether there were marks.
    pub(crate) fn clear(&mut self) -> bool {
        let had = self.count > 0;
        self.entries.clear();
        self.count = 0;
        had
    }

    /// Makes every block unchecked; the marks stay.
    pub(crate) fn invalidate(&mut self) {
        for entry in self.entries.values_mut() {
            entry.checked = None;
        }
    }

    pub(crate) fn count(&self) -> usize {
        self.count
    }

    /// The index of the block with `id`.
    fn index_of(&self, id: BlockId) -> Option<usize> {
        let hint = self.hint.get();
        let found = [hint, hint + 1]
            .into_iter()
            .find(|index| self.order.get(*index) == Some(&id))
            .or_else(|| self.order.iter().position(|other| *other == id))?;
        self.hint.set(found);
        Some(found)
    }

    /// Stores a checker's result. Returns the outcome, the block's index
    /// and whether the block's marks are different now.
    pub(crate) fn set(
        &mut self,
        document: &Document,
        id: BlockId,
        revision: u64,
        ranges: impl IntoIterator<Item = Range<usize>>,
    ) -> (SpellingMarksOutcome, usize, bool) {
        let mut index = self.index_of(id);
        let in_step = self.order.len() == document.block_count()
            && index
                .is_none_or(|index| document.block(index).is_some_and(|block| block.id() == id));
        if !in_step {
            // Should not happen (every change goes through `apply_change`);
            // the document is the truth.
            self.resync(document);
            index = self.index_of(id);
        }
        let Some((index, block)) = index.and_then(|index| Some((index, document.block(index)?)))
        else {
            return (SpellingMarksOutcome::Missing, 0, false);
        };
        if block.revision() != revision {
            return (SpellingMarksOutcome::Stale, index, false);
        }
        let ranges = normalized(ranges, block.text());
        let old = self.entries.get(&id);
        let changed = old.map_or(!ranges.is_empty(), |old| old.ranges != ranges);
        self.count -= old.map_or(0, |old| old.ranges.len());
        self.count += ranges.len();
        self.entries.insert(
            id,
            Entry {
                checked: Some(revision),
                revision,
                text: (!ranges.is_empty()).then(|| block.text().into()),
                ranges,
            },
        );
        (SpellingMarksOutcome::Applied, index, changed)
    }

    /// Follows a change of the document: the marks of changed blocks move
    /// with their text, the marks behind a split or a join go where their
    /// text went, removed blocks lose their entry.
    pub(crate) fn apply_change(&mut self, document: &Document, splice: BlockSplice) {
        let old_range = splice.old_range();
        let new_range = splice.new_range();
        if old_range.end > self.order.len()
            || self.order.len() - splice.old_len + splice.new_len != document.block_count()
            || new_range.end > document.block_count()
        {
            self.resync(document);
            return;
        }
        let new_blocks = &document.blocks()[new_range];
        // The usual edit: one block, changed in place.
        let in_place = splice.old_len == 1
            && splice.new_len == 1
            && self.order[splice.start] == new_blocks[0].id();
        if self.entries.is_empty() {
            if !in_place {
                self.order
                    .splice(old_range, new_blocks.iter().map(Block::id));
            }
            return;
        }

        // The text behind a split or of a joined block ends up at the end
        // of the last new block: remember the marks of the last old block
        // before they follow its own change.
        let tail = (!in_place && splice.old_len > 0 && splice.new_len > 0)
            .then(|| self.order[old_range.end - 1])
            .filter(|source| new_blocks.last().is_some_and(|last| last.id() != *source))
            .and_then(|source| {
                let entry = self.entries.get(&source)?;
                let text = entry.text.clone()?;
                Some((source, entry.revision, entry.ranges.clone(), text))
            });

        for block in new_blocks {
            if let Some(entry) = self.entries.get_mut(&block.id())
                && entry.revision != block.revision()
            {
                self.count -= entry.ranges.len();
                entry.follow(block);
                self.count += entry.ranges.len();
            }
        }

        if let Some((source, source_revision, ranges, text)) = tail
            && let Some(target) = new_blocks.last()
        {
            // What of the source's text can have moved: all of it when the
            // block is gone, what was cut off its end when it is still
            // there.
            let available_from = match new_blocks.iter().find(|block| block.id() == source) {
                None => Some(0),
                Some(block) if block.revision() == source_revision => None,
                Some(block) => {
                    let kept = common_prefix(&text, block.text());
                    (kept == block.text().len()).then_some(kept)
                }
            };
            if let Some(from) = available_from {
                self.carry_tail(target, &ranges, &text, from);
            }
        }

        if !in_place {
            let removed: Vec<BlockId> = if new_blocks.len() <= 16 {
                self.order[old_range.clone()]
                    .iter()
                    .filter(|id| !new_blocks.iter().any(|block| block.id() == **id))
                    .copied()
                    .collect()
            } else {
                let kept: HashSet<BlockId> = new_blocks.iter().map(Block::id).collect();
                self.order[old_range.clone()]
                    .iter()
                    .filter(|id| !kept.contains(id))
                    .copied()
                    .collect()
            };
            for id in removed {
                if let Some(entry) = self.entries.remove(&id) {
                    self.count -= entry.ranges.len();
                }
            }
            self.order
                .splice(old_range, new_blocks.iter().map(Block::id));
        }
    }

    /// Gives `target` the marks of the end of `text` (from byte `from` on)
    /// where `target`'s text ends with the same characters.
    fn carry_tail(&mut self, target: &Block, ranges: &[Range<usize>], text: &str, from: usize) {
        let target_text = target.text();
        let shared = common_suffix(&text[from..], target_text);
        if shared == 0 {
            return;
        }
        let cut = text.len() - shared;
        // Source offsets from `cut` on are target offsets from `landing` on.
        let landing = target_text.len() - shared;
        let entry = self.entries.get(&target.id());
        // Never over what the target has kept of its own.
        let floor = entry
            .filter(|entry| entry.revision == target.revision())
            .and_then(|entry| entry.ranges.last())
            .map_or(0, |last| last.end + 1);
        let carried: Vec<Range<usize>> = ranges
            .iter()
            .filter(|range| {
                range.start > cut
                    || (range.start == cut
                        && !target_text[..landing]
                            .chars()
                            .next_back()
                            .is_some_and(continues_word))
            })
            .map(|range| range.start - cut + landing..range.end - cut + landing)
            .filter(|range| range.start >= floor)
            .collect();
        if carried.is_empty() {
            return;
        }
        self.count += carried.len();
        match self.entries.get_mut(&target.id()) {
            Some(entry) if entry.revision == target.revision() => {
                entry.ranges.extend(carried);
                entry.text.get_or_insert_with(|| target_text.into());
            }
            _ => {
                if let Some(stale) = self.entries.insert(
                    target.id(),
                    Entry {
                        checked: None,
                        revision: target.revision(),
                        ranges: carried,
                        text: Some(target_text.into()),
                    },
                ) {
                    self.count -= stale.ranges.len();
                }
            }
        }
    }

    /// Brings the store in line with the document when a change did not
    /// describe it: entries of blocks that are gone or have another
    /// revision are dropped.
    fn resync(&mut self, document: &Document) {
        self.order.clear();
        self.order.extend(document.blocks().iter().map(Block::id));
        self.hint.set(0);
        if self.entries.is_empty() {
            return;
        }
        let revisions: HashMap<BlockId, u64> = document
            .blocks()
            .iter()
            .map(|block| (block.id(), block.revision()))
            .collect();
        self.entries
            .retain(|id, entry| revisions.get(id) == Some(&entry.revision));
        self.count = self.entries.values().map(|entry| entry.ranges.len()).sum();
    }

    /// The marks of block `index`.
    pub(crate) fn ranges_at(&self, document: &Document, index: usize) -> &[Range<usize>] {
        if self.count == 0 {
            return &[];
        }
        document
            .block(index)
            .and_then(|block| {
                self.entries
                    .get(&block.id())
                    .filter(|entry| entry.revision == block.revision())
            })
            .map_or(&[], |entry| &entry.ranges)
    }

    /// Whether block `index` has a result for its current revision.
    pub(crate) fn checked(&self, document: &Document, index: usize) -> bool {
        document.block(index).is_some_and(|block| {
            self.entries
                .get(&block.id())
                .is_some_and(|entry| entry.checked == Some(block.revision()))
        })
    }

    pub(crate) fn first_unchecked(
        &self,
        document: &Document,
        blocks: Range<usize>,
    ) -> Option<usize> {
        let end = blocks.end.min(document.block_count());
        (blocks.start..end).find(|index| !self.checked(document, *index))
    }

    /// The mark `position` is in or at an end of.
    pub(crate) fn mark_at(&self, document: &Document, position: Position) -> Option<TextRange> {
        let ranges = self.ranges_at(document, position.block);
        let first = ranges.partition_point(|range| range.end < position.offset);
        ranges
            .get(first)
            .filter(|range| range.start <= position.offset)
            .map(|range| TextRange::in_block(position.block, range.start, range.end))
    }

    /// The first mark that starts at or after `from`, going round the end
    /// of the document.
    pub(crate) fn next(&self, document: &Document, from: Position) -> Option<TextRange> {
        if self.count == 0 {
            return None;
        }
        let count = document.block_count();
        let from = document.clamp_position(from);
        (0..=count).find_map(|step| {
            let index = (from.block + step) % count;
            let ranges = self.ranges_at(document, index);
            let found = if step == 0 {
                ranges.get(ranges.partition_point(|range| range.start < from.offset))
            } else {
                ranges.first()
            };
            found.map(|range| TextRange::in_block(index, range.start, range.end))
        })
    }

    /// The last mark that starts before `from`, going round the start of
    /// the document.
    pub(crate) fn previous(&self, document: &Document, from: Position) -> Option<TextRange> {
        if self.count == 0 {
            return None;
        }
        let count = document.block_count();
        let from = document.clamp_position(from);
        (0..=count).find_map(|step| {
            let index = (from.block + count - step % count) % count;
            let ranges = self.ranges_at(document, index);
            let found = if step == 0 {
                ranges[..ranges.partition_point(|range| range.start < from.offset)].last()
            } else {
                ranges.last()
            };
            found.map(|range| TextRange::in_block(index, range.start, range.end))
        })
    }

    /// Checks what the store promises. For tests.
    #[cfg(test)]
    pub(crate) fn check_invariants(&self, document: &Document) -> Result<(), String> {
        let ids: Vec<BlockId> = document.blocks().iter().map(Block::id).collect();
        if ids != self.order {
            return Err("the order of the blocks is not the document's".into());
        }
        let mut count = 0;
        for (id, entry) in &self.entries {
            let Some(block) = document.blocks().iter().find(|block| block.id() == *id) else {
                return Err(format!("an entry for {id:?}, which is not in the document"));
            };
            if entry.revision != block.revision() {
                return Err(format!("the entry of {id:?} is for another revision"));
            }
            if entry
                .checked
                .is_some_and(|checked| checked != entry.revision)
            {
                return Err(format!("{id:?} is checked at a revision it does not have"));
            }
            if entry.ranges.is_empty() != entry.text.is_none() {
                return Err(format!(
                    "{id:?}: text is kept exactly while there are marks"
                ));
            }
            if entry
                .text
                .as_deref()
                .is_some_and(|text| text != block.text())
            {
                return Err(format!("{id:?}: the kept text is not the block's"));
            }
            let text = block.text();
            let mut last_end = None;
            for range in &entry.ranges {
                if range.start >= range.end
                    || range.end > text.len()
                    || !text.is_char_boundary(range.start)
                    || !text.is_char_boundary(range.end)
                {
                    return Err(format!("{id:?}: {range:?} does not fit {text:?}"));
                }
                if last_end.is_some_and(|end| range.start <= end) {
                    return Err(format!(
                        "{id:?}: {range:?} is not apart from the one before"
                    ));
                }
                last_end = Some(range.end);
            }
            count += entry.ranges.len();
        }
        if count != self.count {
            return Err(format!("{count} marks, counted as {}", self.count));
        }
        Ok(())
    }
}

/// The ranges as the store keeps them: inside `text`, on character
/// boundaries, sorted, and ranges that overlap or touch made one.
fn normalized(ranges: impl IntoIterator<Item = Range<usize>>, text: &str) -> Vec<Range<usize>> {
    let mut ranges: Vec<Range<usize>> = ranges
        .into_iter()
        .filter(|range| {
            range.start < range.end
                && range.end <= text.len()
                && text.is_char_boundary(range.start)
                && text.is_char_boundary(range.end)
        })
        .collect();
    ranges.sort_by_key(|range| (range.start, range.end));
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
    for range in ranges {
        match merged.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => merged.push(range),
        }
    }
    merged
}

/// Length of the longest common prefix of `a` and `b` that ends on a
/// character boundary.
fn common_prefix(a: &str, b: &str) -> usize {
    const CHUNK: usize = 64;
    let (a_bytes, b_bytes) = (a.as_bytes(), b.as_bytes());
    let limit = a_bytes.len().min(b_bytes.len());
    let mut same = 0;
    // Whole chunks first (a long paragraph is compared at the speed of
    // `memcmp`), then byte by byte.
    while same + CHUNK <= limit && a_bytes[same..same + CHUNK] == b_bytes[same..same + CHUNK] {
        same += CHUNK;
    }
    while same < limit && a_bytes[same] == b_bytes[same] {
        same += 1;
    }
    while !a.is_char_boundary(same) {
        same -= 1;
    }
    same
}

/// Length of the longest common suffix of `a` and `b` that starts on a
/// character boundary.
fn common_suffix(a: &str, b: &str) -> usize {
    const CHUNK: usize = 64;
    let (a_bytes, b_bytes) = (a.as_bytes(), b.as_bytes());
    let (a_len, b_len) = (a_bytes.len(), b_bytes.len());
    let limit = a_len.min(b_len);
    let mut same = 0;
    while same + CHUNK <= limit
        && a_bytes[a_len - same - CHUNK..a_len - same]
            == b_bytes[b_len - same - CHUNK..b_len - same]
    {
        same += CHUNK;
    }
    while same < limit && a_bytes[a_len - same - 1] == b_bytes[b_len - same - 1] {
        same += 1;
    }
    while !a.is_char_boundary(a_len - same) || !b.is_char_boundary(b_len - same) {
        same -= 1;
    }
    same
}

/// Whether `c` next to a word makes that word longer: a letter, a digit, an
/// apostrophe or a combining mark. A mark that an edit only touches is
/// dropped when its new neighbour is one of these.
fn continues_word(c: char) -> bool {
    c.is_alphanumeric()
        || matches!(c, '\'' | '\u{2019}' | '\u{200d}')
        || matches!(
            c,
            '\u{0300}'..='\u{036f}'
                | '\u{1ab0}'..='\u{1aff}'
                | '\u{1dc0}'..='\u{1dff}'
                | '\u{20d0}'..='\u{20ff}'
                | '\u{fe20}'..='\u{fe2f}'
        )
}

/// Moves marks made for `old` onto `new`, taking the difference between
/// the two as one replaced stretch (what is between their common start and
/// their common end).
fn follow_edit(ranges: &mut Vec<Range<usize>>, old: &str, new: &str) {
    let prefix = common_prefix(old, new);
    let suffix = common_suffix(&old[prefix..], &new[prefix..]);
    let old_end = old.len() - suffix;
    let new_end = new.len() - suffix;
    if prefix == old_end && prefix == new_end {
        return;
    }
    // The characters that adjoin the stretch from outside, in the new text.
    let after_prefix = new[prefix..].chars().next();
    let before_suffix = new[..new_end].chars().next_back();
    ranges.retain_mut(|range| {
        if range.end < prefix {
            true
        } else if range.end == prefix {
            !after_prefix.is_some_and(continues_word)
        } else if range.start >= old_end {
            if range.start == old_end && before_suffix.is_some_and(continues_word) {
                return false;
            }
            range.start = range.start - old_end + new_end;
            range.end = range.end - old_end + new_end;
            true
        } else {
            false
        }
    });
}

/// One wavy underline of a frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PlannedWave {
    /// Window position of the top left of the band the wave runs in.
    pub(crate) origin: Point<Pixels>,
    pub(crate) width: Pixels,
    /// Thickness of the line; the band is three times as high.
    pub(crate) thickness: Pixels,
}

/// How thick the line of the wave is: the same under every text, small or
/// large, plain or bold, so the marks of a page look like one kind of
/// thing. GPUI derives the shape from it: the wave is nine times as long
/// as it is thick and swings 0.8 of it to either side.
pub(crate) const WAVE_THICKNESS: Pixels = px(1.);

/// How far under the baseline the band of the wave starts: one pixel under
/// body text (the wave then touches the feet of the letters without
/// running through them, and crosses descenders where a native text
/// view's dots do), a little more under large type, whose baseline is a
/// heavier line.
pub(crate) fn wave_offset(font_size: Pixels) -> Pixels {
    px((f32::from(font_size) * 0.06).round().clamp(1., 3.))
}

impl Editor {
    /// Sets the misspelled ranges of one block: `ranges` are byte ranges in
    /// the text of the block with id `block`, as it was at block revision
    /// `revision` ([`innerword_core::Block::revision`]). They replace the
    /// block's marks, and the block counts as checked
    /// ([`Editor::spelling_checked`]) until it changes.
    ///
    /// A result for a revision the block has left is not stored
    /// ([`SpellingMarksOutcome::Stale`]): check the block again. An empty
    /// `ranges` is a result too ("nothing misspelled", or "not checked on
    /// purpose" for a code block). Ranges that do not fit the text (past
    /// its end, inside a character, empty) are left out; overlapping ones
    /// become one.
    ///
    /// Cheap: finding the block costs nothing when blocks are set in
    /// document order (a scan over the block ids otherwise), and the
    /// editor repaints only when a block on screen got different marks.
    ///
    /// ```ignore
    /// let block = editor.document().block(index)?;
    /// let (id, revision) = (block.id(), block.revision());
    /// let ranges = service.misspelled(block.text());
    /// if !editor.set_spelling_marks(id, revision, ranges, cx).is_applied() {
    ///     // edited meanwhile: check again
    /// }
    /// ```
    pub fn set_spelling_marks(
        &mut self,
        block: BlockId,
        revision: u64,
        ranges: impl IntoIterator<Item = Range<usize>>,
        cx: &mut Context<Self>,
    ) -> SpellingMarksOutcome {
        let (outcome, index, changed) =
            self.spelling
                .set(self.state.document(), block, revision, ranges);
        if changed && self.spelling.visible && self.spelling.on_screen.contains(&index) {
            cx.notify();
        }
        outcome
    }

    /// Removes every spelling mark and forgets every result: no block
    /// counts as checked afterwards. For "checking was turned off".
    pub fn clear_spelling_marks(&mut self, cx: &mut Context<Self>) {
        if self.spelling.clear() {
            cx.notify();
        }
    }

    /// Makes every block unchecked ([`Editor::spelling_checked`]) without
    /// touching the marks: they stay on screen, and in the answers of the
    /// queries, until the checker's new results replace them block by
    /// block. For "the language changed" and "a word was learned or
    /// ignored", where a [`Editor::clear_spelling_marks`] would make the
    /// marks of a long document disappear until it has been checked again.
    pub fn invalidate_spelling_marks(&mut self) {
        self.spelling.invalidate();
    }

    /// Draws the spelling marks or hides them. Hidden marks are kept, kept
    /// in step with edits and still answered by the queries; only
    /// [`crate::ContextMenuRequest::spelling_mark`] follows what is drawn.
    /// On by default; the switch survives [`Editor::set_document`].
    pub fn set_spelling_marks_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.spelling.visible != visible {
            self.spelling.visible = visible;
            cx.notify();
        }
    }

    /// Whether spelling marks are drawn.
    pub fn spelling_marks_visible(&self) -> bool {
        self.spelling.visible
    }

    /// The marked byte ranges of the block at index `block`, in order.
    /// Empty for a block without marks or an index past the end.
    pub fn spelling_marks(&self, block: usize) -> &[Range<usize>] {
        self.spelling.ranges_at(self.state.document(), block)
    }

    /// Whether the block at index `block` has a result for its current
    /// revision: it was set with [`Editor::set_spelling_marks`] and has
    /// not changed since. False after every change of the block (its marks
    /// are then only carried along), after an undo or redo that brought
    /// another revision of it, after [`Editor::invalidate_spelling_marks`]
    /// and [`Editor::clear_spelling_marks`], and for an index past the end.
    pub fn spelling_checked(&self, block: usize) -> bool {
        self.spelling.checked(self.state.document(), block)
    }

    /// The first block index in `blocks` that is not
    /// [`Editor::spelling_checked`]: what to check next. Costs a lookup
    /// per block it passes; give it the visible blocks first, then a range
    /// that starts where the last call ended.
    pub fn first_unchecked_spelling_block(&self, blocks: Range<usize>) -> Option<usize> {
        self.spelling.first_unchecked(self.state.document(), blocks)
    }

    /// The marked range `position` is in. Its two ends count: a caret
    /// right before or right after a marked word is at that word.
    pub fn spelling_mark_at(&self, position: Position) -> Option<TextRange> {
        self.spelling.mark_at(self.state.document(), position)
    }

    /// The first marked range that starts at or after `from`; after the
    /// last one of the document the search goes on from its start, so the
    /// answer is `None` only when nothing is marked. Give it the end of
    /// the selection: with the caret before or inside a marked word that
    /// is the word itself or the next one, and with a marked word
    /// selected it is the one after it.
    pub fn next_spelling_mark(&self, from: Position) -> Option<TextRange> {
        self.spelling.next(self.state.document(), from)
    }

    /// The last marked range that starts before `from`; before the first
    /// one of the document the search goes on from its end. Give it the
    /// start of the selection: with a marked word selected that is the
    /// one before it, with the caret inside or after a marked word it is
    /// that word.
    pub fn previous_spelling_mark(&self, from: Position) -> Option<TextRange> {
        self.spelling.previous(self.state.document(), from)
    }

    /// The number of marked ranges in the whole document.
    pub fn spelling_mark_count(&self) -> usize {
        self.spelling.count()
    }

    /// The mark a context menu at `position` is about: only what is drawn.
    pub(crate) fn drawn_spelling_mark_at(&self, position: Position) -> Option<TextRange> {
        self.spelling
            .visible
            .then(|| self.spelling_mark_at(position))
            .flatten()
    }
}

#[cfg(test)]
// A list of one range is what these tests mean by `[2..5]`.
#[allow(clippy::single_range_in_vec_init)]
mod tests {
    use super::*;

    const NONE: [Range<usize>; 0] = [];

    fn followed(old: &str, marks: &[Range<usize>], new: &str) -> Vec<Range<usize>> {
        let mut ranges = marks.to_vec();
        follow_edit(&mut ranges, old, new);
        for range in &ranges {
            assert!(new.is_char_boundary(range.start) && new.is_char_boundary(range.end));
            assert!(range.start < range.end && range.end <= new.len());
        }
        ranges
    }

    #[test]
    fn prefixes_and_suffixes_end_on_characters() {
        assert_eq!(common_prefix("abc", "abd"), 2);
        assert_eq!(common_prefix("abc", "abc"), 3);
        assert_eq!(common_prefix("", "abc"), 0);
        // é is C3 A9, è is C3 A8: one byte in common, no character.
        assert_eq!(common_prefix("é", "è"), 0);
        assert_eq!(common_prefix("aé", "aè"), 1);
        assert_eq!(common_suffix("xab", "yab"), 2);
        // é is C3 A9, © is C2 A9.
        assert_eq!(common_suffix("é", "©"), 0);
        assert_eq!(common_suffix("é!", "©!"), 1);
        assert_eq!(common_suffix("abc", "abc"), 3);
        // Longer than a chunk.
        let long = "word ".repeat(100);
        assert_eq!(common_prefix(&long, &format!("{long}x")), long.len());
        assert_eq!(common_suffix(&long, &format!("x{long}")), long.len());
        let changed = format!("{}X{}", &long[..333], &long[334..]);
        assert_eq!(common_prefix(&long, &changed), 333);
        assert_eq!(common_suffix(&long, &changed), long.len() - 334);
    }

    #[test]
    fn marks_follow_an_edit() {
        let old = "a teh b wrld c";
        let marks = [2..5, 8..12];
        // Typed before both: both shift.
        assert_eq!(followed(old, &marks, "xx a teh b wrld c"), [5..8, 11..15]);
        // Typed between them: the first stays, the second shifts.
        assert_eq!(followed(old, &marks, "a teh bb wrld c"), [2..5, 9..13]);
        // Typed after both: nothing moves.
        assert_eq!(followed(old, &marks, "a teh b wrld cc"), [2..5, 8..12]);
        // Typed inside the first: it goes.
        assert_eq!(followed(old, &marks, "a tech b wrld c"), [9..13]);
        // A letter typed at its end or at its start: it goes.
        assert_eq!(followed(old, &marks, "a tehx b wrld c"), [9..13]);
        assert_eq!(followed(old, &marks, "a xteh b wrld c"), [9..13]);
        // A space or a full stop there leaves it.
        assert_eq!(followed(old, &marks, "a teh. b wrld c"), [2..5, 9..13]);
        assert_eq!(followed(old, &marks, "a  teh b wrld c"), [3..6, 9..13]);
        // Deleting the space that parted it from a letter joins two words.
        assert_eq!(followed(old, &marks, "a tehb wrld c"), [7..11]);
        assert_eq!(followed(old, &marks, "ateh b wrld c"), [7..11]);
        // Deleting across both.
        assert_eq!(followed(old, &marks, "a tld c"), NONE);
        // Everything replaced.
        assert_eq!(followed(old, &marks, ""), NONE);
        assert_eq!(followed(old, &marks, "zzz"), NONE);
        // Nothing changed.
        assert_eq!(followed(old, &marks, old), marks);
    }

    #[test]
    fn marks_follow_edits_in_multi_byte_text() {
        let old = "é naïf ö";
        // "naïf" is bytes 3..8.
        assert_eq!(&old[3..8], "naïf");
        let marks = [3..8];
        assert_eq!(followed(old, &marks, "éé naïf ö"), [5..10]);
        // é replaced by è: one byte differs, the character is the change.
        assert_eq!(followed(old, &marks, "è naïf ö"), [3..8]);
        // ï replaced by i inside the word.
        assert_eq!(followed(old, &marks, "é naif ö"), NONE);
        // A combining accent typed after the word continues it.
        assert_eq!(followed(old, &marks, "é naïf\u{301} ö"), NONE);
        // An emoji typed after it does not.
        assert_eq!(followed(old, &marks, "é naïf🙂 ö"), [3..8]);
        // ö replaced by ü behind it.
        assert_eq!(followed(old, &marks, "é naïf ü"), [3..8]);
    }

    #[test]
    fn results_are_normalized() {
        let text = "é abc déf";
        let backwards = Range { start: 10, end: 8 };
        assert_eq!(
            normalized([7..9, 3..6, 4..5, 0..1, 9..9, 6..99, backwards, 6..7], text),
            [3..7]
        );
        assert_eq!(normalized([0..2, 3..6], text), [0..2, 3..6]);
        // Touching ranges are one mark.
        assert_eq!(normalized([3..4, 4..6], text), [3..6]);
    }
}
