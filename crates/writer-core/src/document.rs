//! The document: blocks, comment threads and settings.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::ops::Range;

use crate::attrs::CommentId;
use crate::block::{Block, BlockId, BlockKind};
use crate::comments::CommentThread;
use crate::position::{Position, TextRange};
use crate::settings::DocumentSettings;
use crate::text;

/// Maximum length of [`Document::title`] in characters.
pub const TITLE_MAX_CHARS: usize = 80;

/// A rich-text document.
///
/// Invariants (checked by [`Document::check_invariants`]):
/// - there is always at least one block;
/// - block ids are unique and assigned;
/// - every block satisfies the block invariants (normalised text and runs,
///   void blocks empty, code blocks without formatting, indent `<= 8`);
/// - every comment anchor refers to an existing thread.
///
/// A `Document` is read-only from the outside; edits go through
/// [`EditorState`](crate::EditorState). Equality compares content (blocks
/// including ids, threads, settings) and ignores revision stamps.
#[derive(Clone, Debug)]
pub struct Document {
    pub(crate) blocks: Vec<Block>,
    pub(crate) comments: BTreeMap<CommentId, CommentThread>,
    pub(crate) settings: DocumentSettings,
    pub(crate) next_block_id: u64,
    pub(crate) next_comment_id: u64,
    pub(crate) next_stamp: u64,
    /// For each thread, the number of blocks carrying its anchor.
    pub(crate) anchor_counts: BTreeMap<CommentId, usize>,
}

impl PartialEq for Document {
    fn eq(&self, other: &Document) -> bool {
        self.blocks == other.blocks
            && self.comments == other.comments
            && self.settings == other.settings
    }
}

impl Default for Document {
    fn default() -> Document {
        Document::new()
    }
}

impl Document {
    /// A document with one empty paragraph and default settings.
    pub fn new() -> Document {
        Document::from_blocks(vec![Block::new(BlockKind::Paragraph)])
    }

    /// Builds a document from blocks, repairing anything invalid: an empty
    /// list becomes one empty paragraph, unassigned or duplicate ids get fresh
    /// ids, comment anchors (there are no threads yet) are removed.
    pub fn from_blocks(blocks: Vec<Block>) -> Document {
        Document::from_parts(blocks, Vec::new(), DocumentSettings::default())
    }

    /// Builder: replaces the settings (sanitised).
    pub fn with_settings(mut self, settings: DocumentSettings) -> Document {
        self.settings = settings.sanitized();
        self
    }

    /// Builds a document from untrusted parts, repairing every invariant.
    pub(crate) fn from_parts(
        blocks: Vec<Block>,
        threads: Vec<CommentThread>,
        settings: DocumentSettings,
    ) -> Document {
        let mut out: Vec<Block> = Vec::with_capacity(blocks.len().max(1));
        for mut b in blocks {
            let extra = b.repair();
            out.push(b);
            if let Some(e) = extra {
                out.push(e);
            }
        }
        if out.is_empty() {
            out.push(Block::new(BlockKind::Paragraph));
        }
        // Ids.
        let max_id = out.iter().map(|b| b.id.0).max().unwrap_or(0);
        let mut next_block_id = max_id + 1;
        let mut seen = HashSet::with_capacity(out.len());
        for b in &mut out {
            if b.id == BlockId::UNASSIGNED || !seen.insert(b.id) {
                b.id = BlockId(next_block_id);
                next_block_id += 1;
                seen.insert(b.id);
            }
        }
        // Threads.
        let max_cid = threads.iter().map(|t| t.id.0).max().unwrap_or(0);
        let mut next_comment_id = max_cid + 1;
        let mut comments = BTreeMap::new();
        for mut t in threads {
            t.repair();
            if t.id.0 == 0 || comments.contains_key(&t.id) {
                t.id = CommentId(next_comment_id);
                next_comment_id += 1;
            }
            comments.insert(t.id, t);
        }
        // Anchors must refer to existing threads.
        for b in &mut out {
            let unknown: Vec<CommentId> = b
                .comment_ids()
                .into_iter()
                .filter(|id| !comments.contains_key(id))
                .collect();
            for id in unknown {
                b.remove_comment(id);
            }
        }
        let mut doc = Document {
            blocks: out,
            comments,
            settings: settings.sanitized(),
            next_block_id,
            next_comment_id,
            next_stamp: 1,
            anchor_counts: BTreeMap::new(),
        };
        for i in 0..doc.blocks.len() {
            let s = doc.mint_stamp();
            doc.blocks[i].revision = s;
        }
        doc.anchor_counts = doc.compute_anchor_counts();
        doc
    }

    // ----- read API ---------------------------------------------------------------

    /// All blocks in order (never empty).
    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    /// The block at `index`.
    pub fn block(&self, index: usize) -> Option<&Block> {
        self.blocks.get(index)
    }

    /// Number of blocks (at least 1).
    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }

    /// The index of the block with `id`. This is a linear scan.
    pub fn block_index(&self, id: BlockId) -> Option<usize> {
        self.blocks.iter().position(|b| b.id == id)
    }

    /// The document settings.
    pub fn settings(&self) -> &DocumentSettings {
        &self.settings
    }

    /// The comment thread with `id`.
    pub fn comment(&self, id: CommentId) -> Option<&CommentThread> {
        self.comments.get(&id)
    }

    /// All comment threads, ordered by id (creation order). See
    /// [`Document::threads_in_order`] for document order.
    pub fn comments(&self) -> impl Iterator<Item = &CommentThread> {
        self.comments.values()
    }

    /// Number of comment threads.
    pub fn comment_count(&self) -> usize {
        self.comments.len()
    }

    /// The first position of the document.
    pub fn start(&self) -> Position {
        Position::ZERO
    }

    /// The last position of the document.
    pub fn end(&self) -> Position {
        let last = self.blocks.len() - 1;
        Position::new(last, self.blocks[last].len())
    }

    /// Clamps `pos` to a valid caret position: block index into range,
    /// offset into the block, rounded down to a char and grapheme boundary
    /// (always 0 on void blocks).
    pub fn clamp_position(&self, pos: Position) -> Position {
        let block = pos.block.min(self.blocks.len() - 1);
        let b = &self.blocks[block];
        let offset = if b.is_void() {
            0
        } else {
            text::floor_grapheme_boundary(&b.text, pos.offset)
        };
        Position::new(block, offset)
    }

    /// Whether `pos` is a valid caret position (see [`Document::clamp_position`]).
    pub fn is_valid_position(&self, pos: Position) -> bool {
        self.clamp_position(pos) == pos
    }

    /// The text in `range`, blocks joined by `\n` (void blocks contribute an
    /// empty line).
    pub fn text_in(&self, range: TextRange) -> String {
        let s = self.clamp_position(range.start);
        let e = self.clamp_position(range.end);
        let (s, e) = if s <= e { (s, e) } else { (e, s) };
        let mut out = String::new();
        for i in s.block..=e.block {
            let b = &self.blocks[i];
            let from = if i == s.block { s.offset } else { 0 };
            let to = if i == e.block { e.offset } else { b.len() };
            if i > s.block {
                out.push('\n');
            }
            out.push_str(&b.text[from..to.max(from)]);
        }
        out
    }

    /// The byte offset of `pos` (clamped) in [`Document::plain_text`]
    /// (blocks joined by `\n`). Linear in the number of blocks before `pos`;
    /// meant for occasional use (Vim search, find-in-document), not per frame.
    pub fn flat_offset(&self, pos: Position) -> usize {
        let pos = self.clamp_position(pos);
        let before: usize = self.blocks[..pos.block].iter().map(|b| b.len() + 1).sum();
        before + pos.offset
    }

    /// The position of byte `offset` of [`Document::plain_text`], clamped
    /// to the end. An offset on a block separator maps to the end of the
    /// block before it. Linear in the number of blocks.
    pub fn position_from_flat(&self, offset: usize) -> Position {
        let mut start = 0;
        for (i, b) in self.blocks.iter().enumerate() {
            if offset <= start + b.len() {
                return self.clamp_position(Position::new(i, offset - start));
            }
            start += b.len() + 1;
        }
        self.end()
    }

    /// Plain text of the whole document: blocks joined by `\n`, void blocks
    /// as empty lines. Intended for search indexing.
    pub fn plain_text(&self) -> String {
        let mut out = String::with_capacity(self.blocks.iter().map(|b| b.len() + 1).sum());
        for (i, b) in self.blocks.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            out.push_str(&b.text);
        }
        out
    }

    /// A title for the document: the first heading's text, else the first
    /// non-empty block's text; the first line only, whitespace collapsed,
    /// truncated to [`TITLE_MAX_CHARS`] characters on a grapheme boundary
    /// (preferring a word boundary). `None` when the document has no text.
    pub fn title(&self) -> Option<String> {
        let pick = self
            .blocks
            .iter()
            .find(|b| b.kind.heading_level().is_some() && !b.text.trim().is_empty())
            .or_else(|| self.blocks.iter().find(|b| !b.text.trim().is_empty()))?;
        let line = pick.text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
        let collapsed: String = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if collapsed.chars().count() <= TITLE_MAX_CHARS {
            return Some(collapsed);
        }
        use unicode_segmentation::UnicodeSegmentation;
        let mut cut = 0;
        let mut chars = 0;
        for (i, g) in collapsed.grapheme_indices(true) {
            let n = g.chars().count();
            if chars + n > TITLE_MAX_CHARS {
                break;
            }
            chars += n;
            cut = i + g.len();
        }
        let head = &collapsed[..cut];
        let trimmed = match head.rfind(' ') {
            Some(sp) if sp > cut / 2 => &head[..sp],
            _ => head,
        };
        Some(trimmed.trim_end().to_string())
    }

    // ----- numbering ----------------------------------------------------------------

    /// The ordinal (1-based) of the numbered list item at `index`, or `None`
    /// if that block is not a [`BlockKind::NumberedItem`].
    ///
    /// A contiguous run of numbered items at the same indent counts up.
    /// Blocks indented deeper are skipped (nested lists number independently
    /// and do not break the outer run); any other block at the same or a
    /// shallower indent restarts the count.
    pub fn numbered_ordinal(&self, index: usize) -> Option<usize> {
        let b = self.blocks.get(index)?;
        if b.kind != BlockKind::NumberedItem {
            return None;
        }
        let level = b.indent;
        let mut n = 1;
        for j in (0..index).rev() {
            let p = &self.blocks[j];
            if p.indent > level {
                continue;
            }
            if p.indent == level && p.kind == BlockKind::NumberedItem {
                n += 1;
            } else {
                break;
            }
        }
        Some(n)
    }

    /// [`Document::numbered_ordinal`] for every block in `range` (clamped),
    /// computed in one pass. Use this when laying out many blocks.
    pub fn numbered_ordinals(&self, range: Range<usize>) -> Vec<Option<usize>> {
        let end = range.end.min(self.blocks.len());
        let start = range.start.min(end);
        if start == end {
            return Vec::new();
        }
        // Walk back to a block that resets every level.
        let mut k = start;
        while k > 0 {
            let p = &self.blocks[k - 1];
            if p.indent == 0 && p.kind != BlockKind::NumberedItem {
                break;
            }
            k -= 1;
        }
        let mut counters = [0usize; crate::block::MAX_INDENT as usize + 1];
        let mut out = Vec::with_capacity(end - start);
        for i in k..end {
            let b = &self.blocks[i];
            let level = b.indent as usize;
            for c in counters.iter_mut().skip(level + 1) {
                *c = 0;
            }
            let ord = if b.kind == BlockKind::NumberedItem {
                counters[level] += 1;
                Some(counters[level])
            } else {
                counters[level] = 0;
                None
            };
            if i >= start {
                out.push(ord);
            }
        }
        out
    }

    // ----- comments -------------------------------------------------------------

    /// Whether thread `id` exists and none of its anchored text is left.
    pub fn is_orphaned(&self, id: CommentId) -> bool {
        self.comments.contains_key(&id) && self.anchor_counts.get(&id).copied().unwrap_or(0) == 0
    }

    /// Comment threads anchored at `pos`: on the character before or the
    /// character after it. Ordered by id.
    pub fn comments_at(&self, pos: Position) -> Vec<CommentId> {
        let pos = self.clamp_position(pos);
        let b = &self.blocks[pos.block];
        let mut ids = BTreeSet::new();
        if let Some(a) = b.attrs_before(pos.offset) {
            ids.extend(a.comments.iter().copied());
        }
        if let Some(a) = b.attrs_at(pos.offset) {
            ids.extend(a.comments.iter().copied());
        }
        ids.into_iter().collect()
    }

    /// Every anchor range of every thread, sorted by start position (then
    /// id). A thread may have several ranges; a range that runs from the end
    /// of one block into the start of the next is reported as one range.
    pub fn all_comment_ranges(&self) -> Vec<(CommentId, TextRange)> {
        let mut out: Vec<(CommentId, TextRange)> = Vec::new();
        // id -> index in `out` of its last range.
        let mut last: BTreeMap<CommentId, usize> = BTreeMap::new();
        for (bi, b) in self.blocks.iter().enumerate() {
            if b.runs.iter().all(|r| r.attrs.comments.is_empty()) {
                continue;
            }
            for (r, a) in b.spans() {
                for &id in &a.comments {
                    if let Some(&li) = last.get(&id) {
                        let range = &mut out[li].1;
                        let contiguous_in_block =
                            range.end.block == bi && range.end.offset == r.start;
                        let contiguous_across = r.start == 0
                            && range.end.block + 1 == bi
                            && range.end.offset == self.blocks[range.end.block].len();
                        if contiguous_in_block || contiguous_across {
                            range.end = Position::new(bi, r.end);
                            continue;
                        }
                    }
                    out.push((id, TextRange::in_block(bi, r.start, r.end)));
                    last.insert(id, out.len() - 1);
                }
            }
        }
        out.sort_by(|a, b| (a.1.start, a.0).cmp(&(b.1.start, b.0)));
        out
    }

    /// The anchor ranges of thread `id` in document order (empty when orphaned).
    pub fn comment_ranges(&self, id: CommentId) -> Vec<TextRange> {
        if self.anchor_counts.get(&id).copied().unwrap_or(0) == 0 {
            return Vec::new();
        }
        self.all_comment_ranges()
            .into_iter()
            .filter(|(c, _)| *c == id)
            .map(|(_, r)| r)
            .collect()
    }

    /// Thread ids in document order of their first anchor; orphaned threads
    /// follow, ordered by id.
    pub fn threads_in_order(&self) -> Vec<CommentId> {
        let mut seen = BTreeSet::new();
        let mut out = Vec::with_capacity(self.comments.len());
        for (id, _) in self.all_comment_ranges() {
            if seen.insert(id) {
                out.push(id);
            }
        }
        for id in self.comments.keys() {
            if !seen.contains(id) {
                out.push(*id);
            }
        }
        out
    }

    /// The first anchor range starting strictly after `from`. With `wrap`,
    /// continues from the start of the document.
    pub fn next_comment(&self, from: Position, wrap: bool) -> Option<(CommentId, TextRange)> {
        let all = self.all_comment_ranges();
        all.iter()
            .find(|(_, r)| r.start > from)
            .or_else(|| if wrap { all.first() } else { None })
            .copied()
    }

    /// The last anchor range starting strictly before `from`. With `wrap`,
    /// continues from the end of the document. (Navigating places the caret
    /// at a range's start, so repeated calls walk backwards.)
    pub fn prev_comment(&self, from: Position, wrap: bool) -> Option<(CommentId, TextRange)> {
        let all = self.all_comment_ranges();
        all.iter()
            .rev()
            .find(|(_, r)| r.start < from)
            .or_else(|| if wrap { all.last() } else { None })
            .copied()
    }

    // ----- invariants -----------------------------------------------------------

    /// Checks every invariant of the document. Linear in the document size;
    /// meant for tests and debugging.
    pub fn check_invariants(&self) -> Result<(), String> {
        if self.blocks.is_empty() {
            return Err("document has no blocks".into());
        }
        let mut ids = HashSet::with_capacity(self.blocks.len());
        for (i, b) in self.blocks.iter().enumerate() {
            if b.id == BlockId::UNASSIGNED {
                return Err(format!("block {i} has no id"));
            }
            if b.id.0 >= self.next_block_id {
                return Err(format!("block {i} id {} not below the id counter", b.id));
            }
            if !ids.insert(b.id) {
                return Err(format!("duplicate block id {}", b.id));
            }
            if b.revision == 0 || b.revision >= self.next_stamp {
                return Err(format!("block {i} has an invalid revision stamp"));
            }
            b.check().map_err(|e| format!("block index {i}: {e}"))?;
            for id in b.comment_ids() {
                if !self.comments.contains_key(&id) {
                    return Err(format!("block {i} anchors missing thread {id}"));
                }
            }
        }
        for (id, t) in &self.comments {
            if t.id != *id {
                return Err(format!("thread key {id} does not match its id {}", t.id));
            }
            if id.0 == 0 || id.0 >= self.next_comment_id {
                return Err(format!("thread id {id} not below the id counter"));
            }
            let mut rs = BTreeSet::new();
            for r in &t.replies {
                if r.id.0 == 0 || !rs.insert(r.id) {
                    return Err(format!("thread {id} has duplicate or zero reply ids"));
                }
            }
        }
        if self.compute_anchor_counts() != self.anchor_counts {
            return Err("anchor index out of date".into());
        }
        if self.settings != self.settings.clone().sanitized() {
            return Err("settings out of range".into());
        }
        Ok(())
    }

    // ----- crate-private mutation -------------------------------------------------

    fn compute_anchor_counts(&self) -> BTreeMap<CommentId, usize> {
        let mut counts = BTreeMap::new();
        for b in &self.blocks {
            for id in b.comment_ids() {
                *counts.entry(id).or_insert(0) += 1;
            }
        }
        counts
    }

    pub(crate) fn mint_stamp(&mut self) -> u64 {
        let s = self.next_stamp;
        self.next_stamp += 1;
        s
    }

    pub(crate) fn mint_block_id(&mut self) -> BlockId {
        let id = BlockId(self.next_block_id);
        self.next_block_id += 1;
        id
    }

    pub(crate) fn mint_comment_id(&mut self) -> CommentId {
        let id = CommentId(self.next_comment_id);
        self.next_comment_id += 1;
        id
    }

    fn count_anchors(&mut self, b: &Block, delta: isize) {
        if b.runs.iter().all(|r| r.attrs.comments.is_empty()) {
            return;
        }
        for id in b.comment_ids() {
            let c = self.anchor_counts.entry(id).or_insert(0);
            *c = c.saturating_add_signed(delta);
            if *c == 0 {
                self.anchor_counts.remove(&id);
            }
        }
    }

    /// Replaces `blocks[index..index + remove]` with `insert`, returning the
    /// removed blocks. Blocks without an id get a fresh id, blocks with a zero
    /// revision get a fresh stamp. Keeps the anchor index up to date.
    pub(crate) fn splice_raw(&mut self, index: usize, remove: usize, mut insert: Vec<Block>) -> Vec<Block> {
        for b in &mut insert {
            if b.id == BlockId::UNASSIGNED {
                b.id = self.mint_block_id();
            }
            if b.revision == 0 {
                b.revision = self.mint_stamp();
            }
        }
        for b in &insert {
            self.count_anchors(b, 1);
        }
        let removed: Vec<Block> = self.blocks.splice(index..index + remove, insert).collect();
        for b in &removed {
            self.count_anchors(b, -1);
        }
        debug_assert!(!self.blocks.is_empty());
        removed
    }

    /// Sets or removes thread `id`, returning the previous value.
    pub(crate) fn set_thread_raw(
        &mut self,
        id: CommentId,
        thread: Option<CommentThread>,
    ) -> Option<CommentThread> {
        match thread {
            Some(t) => self.comments.insert(id, t),
            None => self.comments.remove(&id),
        }
    }

    pub(crate) fn set_settings_raw(&mut self, settings: DocumentSettings) {
        self.settings = settings.sanitized();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attrs::InlineAttrs;

    fn numbered(indent: u8) -> Block {
        Block::with_text(BlockKind::NumberedItem, "x").with_indent(indent)
    }

    #[test]
    fn from_blocks_repairs() {
        let d = Document::from_blocks(vec![]);
        assert_eq!(d.block_count(), 1);
        assert!(d.check_invariants().is_ok());
        let mut a = Block::paragraph("a");
        a.id = BlockId(5);
        let mut b = Block::paragraph("b");
        b.id = BlockId(5);
        let mut anchored = InlineAttrs::default();
        anchored.comments.insert(CommentId(9));
        let c = Block::paragraph("").with_span("c", anchored);
        let d = Document::from_blocks(vec![a, b, c]);
        assert!(d.check_invariants().is_ok());
        assert_eq!(d.blocks()[0].id(), BlockId(5));
        assert_ne!(d.blocks()[1].id(), BlockId(5));
        assert!(d.blocks()[2].comment_ids().is_empty());
    }

    #[test]
    fn numbering_rules() {
        let blocks = vec![
            numbered(0),                  // 1
            numbered(0),                  // 2
            numbered(1),                  // 1 (nested)
            Block::paragraph("p").with_indent(1), // deeper: does not break outer
            numbered(1),                  // 1 (restarted by paragraph at its level)
            numbered(0),                  // 3
            Block::paragraph("break"),
            numbered(0), // 1
            numbered(2), // 1
            numbered(1), // 1
            numbered(2), // 1 (restarted by the shallower item)
        ];
        let d = Document::from_blocks(blocks);
        let expect = [
            Some(1),
            Some(2),
            Some(1),
            None,
            Some(1),
            Some(3),
            None,
            Some(1),
            Some(1),
            Some(1),
            Some(1),
        ];
        for (i, e) in expect.iter().enumerate() {
            assert_eq!(d.numbered_ordinal(i), *e, "block {i}");
        }
        assert_eq!(d.numbered_ordinals(0..d.block_count()), expect.to_vec());
        assert_eq!(d.numbered_ordinals(4..6), expect[4..6].to_vec());
        assert_eq!(d.numbered_ordinals(5..100), expect[5..].to_vec());
        assert_eq!(d.numbered_ordinals(50..60), Vec::<Option<usize>>::new());
    }

    #[test]
    fn title_heuristic() {
        let d = Document::from_blocks(vec![
            Block::paragraph("  "),
            Block::paragraph("Intro text"),
            Block::with_text(BlockKind::heading(2), "  The   Real Title \nsecond"),
        ]);
        assert_eq!(d.title().as_deref(), Some("The Real Title"));
        let d = Document::from_blocks(vec![Block::paragraph(""), Block::paragraph("First words")]);
        assert_eq!(d.title().as_deref(), Some("First words"));
        assert_eq!(Document::new().title(), None);
        let long = "word ".repeat(40);
        let d = Document::from_blocks(vec![Block::paragraph(&long)]);
        let t = d.title().unwrap();
        assert!(t.chars().count() <= TITLE_MAX_CHARS);
        assert!(t.ends_with("word"));
    }

    #[test]
    fn text_in_and_clamp() {
        let d = Document::from_blocks(vec![
            Block::paragraph("hello"),
            Block::new(BlockKind::Divider),
            Block::paragraph("e\u{301}x"),
        ]);
        assert_eq!(d.text_in(TextRange::new(Position::new(0, 2), Position::new(2, 3))), "llo\n\ne\u{301}");
        assert_eq!(d.plain_text(), "hello\n\ne\u{301}x");
        assert_eq!(d.clamp_position(Position::new(9, 9)), Position::new(2, 4));
        assert_eq!(d.clamp_position(Position::new(1, 3)), Position::new(1, 0));
        assert_eq!(d.clamp_position(Position::new(2, 2)), Position::new(2, 0));
        assert!(!d.is_valid_position(Position::new(2, 1)));
        let text = d.plain_text();
        for (i, _) in text.char_indices() {
            let p = d.position_from_flat(i);
            assert!(d.flat_offset(p) <= i);
            if text::is_grapheme_boundary(&text, i) {
                assert_eq!(d.flat_offset(p), i);
            }
        }
        assert_eq!(d.position_from_flat(5), Position::new(0, 5));
        assert_eq!(d.position_from_flat(6), Position::new(1, 0));
        assert_eq!(d.position_from_flat(7), Position::new(2, 0));
        assert_eq!(d.position_from_flat(999), d.end());
        assert_eq!(d.flat_offset(Position::new(2, 1)), 7);
        assert_eq!(d.flat_offset(d.end()), text.len());
    }
}
