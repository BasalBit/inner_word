//! The document: blocks, comment threads and settings.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::ops::Range;

use crate::attrs::CommentId;
use crate::block::{Block, BlockId, BlockKind};
use crate::comments::CommentThread;
use crate::position::{Position, Selection, TextRange};
use crate::settings::DocumentSettings;
use crate::text;

/// Maximum length of [`Document::title`] in characters.
pub const TITLE_MAX_CHARS: usize = 80;
/// Longest first sentence [`Document::title`] takes whole as the title of a
/// document without a heading.
pub const TITLE_SENTENCE_CHARS: usize = 50;
/// Longest title [`Document::title`] cuts out of text that is not a heading
/// (the ellipsis included).
pub const TITLE_TEXT_CHARS: usize = 48;

/// `text` when it has at most `max` characters, else its start: cut at the
/// last space where that keeps more than half, never inside a grapheme,
/// without dangling punctuation, and ending with an ellipsis (which counts
/// towards `max`).
fn shortened_title(text: &str, max: usize) -> String {
    use unicode_segmentation::UnicodeSegmentation;
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut cut = 0;
    let mut chars = 0;
    for (i, g) in text.grapheme_indices(true) {
        let n = g.chars().count();
        // One character is kept for the ellipsis.
        if chars + n > max.saturating_sub(1) {
            break;
        }
        chars += n;
        cut = i + g.len();
    }
    let head = &text[..cut];
    // A cut that falls right after a whole word keeps it.
    let whole_word = text[cut..].starts_with(' ');
    let head = match head.rfind(' ') {
        Some(space) if !whole_word && space > cut / 2 => &head[..space],
        _ => head,
    };
    let head = head.trim_end_matches(|c: char| {
        c.is_whitespace() || matches!(c, ',' | ';' | ':' | '.' | '-' | '\u{2013}' | '\u{2014}' | '(' | '\u{201c}' | '"')
    });
    format!("{head}\u{2026}")
}

/// The byte offset just after the first sentence of `text` (after its
/// closing mark and any quote or bracket that closes with it), or `None`
/// when the text has no sentence end.
///
/// A sentence ends at `.`, `!`, `?` or an ellipsis that is followed by a
/// space or by nothing. A full stop after an initial or a short capitalised word ("J.",
/// "Dr.", "St.") or after a word with a stop inside ("e.g.") is taken for
/// an abbreviation.
fn first_sentence_end(text: &str) -> Option<usize> {
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if !matches!(c, '.' | '!' | '?' | '\u{2026}') {
            continue;
        }
        // Marks that come together ("?!", "...") and what closes with them.
        let mut end = i + c.len_utf8();
        while let Some(&(j, next)) = chars.peek() {
            if matches!(next, '.' | '!' | '?' | '\u{2026}' | '"' | '\'' | ')' | ']' | '\u{201d}' | '\u{2019}' | '\u{00bb}') {
                end = j + next.len_utf8();
                chars.next();
            } else {
                break;
            }
        }
        if end < text.len() && !text[end..].starts_with(' ') {
            // Inside a number or a name ("3.50", "example.com").
            continue;
        }
        if c == '.' && end == i + 1 {
            let word = text[..i].rsplit(' ').next().unwrap_or("");
            let letters = word.chars().filter(|c| c.is_alphabetic()).count();
            let capitalised = word.chars().next().is_some_and(char::is_uppercase);
            if word.contains('.') || letters <= 1 || (capitalised && letters <= 3) {
                continue;
            }
        }
        return Some(end);
    }
    None
}

/// Ids at or above this value in loaded data are treated as invalid and
/// replaced, so id counters can never overflow.
pub(crate) const MAX_ID: u64 = 1 << 52;

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
        let valid = |id: u64| id != 0 && id < MAX_ID;
        let max_id = out.iter().map(|b| b.id.0).filter(|&i| valid(i)).max().unwrap_or(0);
        let mut next_block_id = max_id + 1;
        let mut seen = HashSet::with_capacity(out.len());
        for b in &mut out {
            if !valid(b.id.0) || !seen.insert(b.id) {
                b.id = BlockId(next_block_id);
                next_block_id += 1;
                seen.insert(b.id);
            }
        }
        // Threads.
        let max_cid = threads.iter().map(|t| t.id.0).filter(|&i| valid(i)).max().unwrap_or(0);
        let mut next_comment_id = max_cid + 1;
        let mut comments = BTreeMap::new();
        for mut t in threads {
            t.repair();
            if !valid(t.id.0) || comments.contains_key(&t.id) {
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

    /// Clamps both ends of `selection` (see [`Document::clamp_position`]).
    pub fn clamp_selection(&self, selection: Selection) -> Selection {
        Selection::new(
            self.clamp_position(selection.anchor),
            self.clamp_position(selection.head),
        )
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

    /// The whole text of block `index` (clamped) as a range: from its
    /// offset 0 to its end.
    pub fn block_range(&self, index: usize) -> TextRange {
        let i = index.min(self.blocks.len() - 1);
        TextRange::in_block(i, 0, self.blocks[i].len())
    }

    /// The part of `range` (clamped) inside block `index`, as a byte range
    /// of that block: `None` when the block lies outside the range. Blocks
    /// strictly inside get their whole text (`0..0` for a void block, which
    /// the UI shows as selected); a void block at either end of a
    /// multi-block range is included too, matching what deletion and copy
    /// do. For drawing selections, search matches and comment ranges.
    pub fn range_in_block(&self, range: TextRange, index: usize) -> Option<Range<usize>> {
        let s = self.clamp_position(range.start);
        let e = self.clamp_position(range.end);
        let (s, e) = if s <= e { (s, e) } else { (e, s) };
        if index < s.block || index > e.block || index >= self.blocks.len() {
            return None;
        }
        let len = self.blocks[index].len();
        let from = if index == s.block { s.offset } else { 0 };
        let to = if index == e.block { e.offset } else { len };
        if s.block == e.block && from == to {
            return None;
        }
        if index == e.block && to == 0 && index > s.block && !self.blocks[index].is_void() {
            // The range ends at the start of this text block: nothing of it.
            return None;
        }
        Some(from..to)
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

    /// A title for the document: the first heading's text, else one taken
    /// from the first non-empty block; the first line only, whitespace
    /// collapsed. `None` when the document has no text.
    ///
    /// A heading is a title someone wrote: it is kept whole up to
    /// [`TITLE_MAX_CHARS`] characters. Text that merely starts a document
    /// (a social post, a note) is not, so the title made from it stays
    /// short: its first sentence when that ends within
    /// [`TITLE_SENTENCE_CHARS`] characters (without the full stop), else
    /// its first [`TITLE_TEXT_CHARS`] characters. Whatever is cut is cut
    /// at a word boundary where there is one, never inside a character,
    /// and ends with an ellipsis.
    pub fn title(&self) -> Option<String> {
        let heading = self
            .blocks
            .iter()
            .find(|b| b.kind.heading_level().is_some() && !b.text.trim().is_empty());
        let pick = heading.or_else(|| self.blocks.iter().find(|b| !b.text.trim().is_empty()))?;
        let line = pick.text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
        let collapsed: String = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if heading.is_some() {
            return Some(shortened_title(&collapsed, TITLE_MAX_CHARS));
        }
        if let Some(end) = first_sentence_end(&collapsed)
            && collapsed[..end].chars().count() <= TITLE_SENTENCE_CHARS
        {
            let sentence = &collapsed[..end];
            // A title does not end with a full stop (an ellipsis, a
            // question or an exclamation mark belong to it).
            let sentence = match sentence.strip_suffix('.') {
                Some(rest) if !rest.ends_with('.') => rest,
                _ => sentence,
            };
            return Some(sentence.to_string());
        }
        Some(shortened_title(&collapsed, TITLE_TEXT_CHARS))
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
    /// id). A thread may have several ranges. A range that runs from the end
    /// of one block into the start of a later one is reported as one range
    /// when only blocks without text (empty paragraphs, dividers) lie in
    /// between, as happens when a comment is added on a selection across
    /// them.
    ///
    /// This walks the runs of every block (not the text), so it is cheap
    /// enough for the comments panel and for comment navigation, but the
    /// editor should draw comment highlights from the runs of the visible
    /// blocks ([`InlineAttrs::comments`](crate::InlineAttrs::comments)).
    pub fn all_comment_ranges(&self) -> Vec<(CommentId, TextRange)> {
        let mut out: Vec<(CommentId, TextRange)> = Vec::new();
        // id -> index in `out` of its last range.
        let mut last: BTreeMap<CommentId, usize> = BTreeMap::new();
        // The last block with text before the current one.
        let mut prev_text: Option<usize> = None;
        for (bi, b) in self.blocks.iter().enumerate() {
            if b.runs.iter().all(|r| r.attrs.comments.is_empty()) {
                if !b.text.is_empty() {
                    prev_text = Some(bi);
                }
                continue;
            }
            for (r, a) in b.spans() {
                for &id in &a.comments {
                    if let Some(&li) = last.get(&id) {
                        let range = &mut out[li].1;
                        let contiguous_in_block =
                            range.end.block == bi && range.end.offset == r.start;
                        let contiguous_across = r.start == 0
                            && prev_text == Some(range.end.block)
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
            prev_text = Some(bi);
        }
        out.sort_by_key(|a| (a.1.start, a.0));
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
        assert!(t.chars().count() <= TITLE_TEXT_CHARS);
        assert!(t.ends_with("word\u{2026}"), "{t}");
    }

    #[test]
    fn title_of_a_document_without_a_heading_is_short_and_whole() {
        let title = |text: &str| {
            Document::from_blocks(vec![Block::paragraph(text)])
                .title()
                .unwrap()
        };
        // The first sentence, without its full stop.
        assert_eq!(
            title(
                "The Salt Road is out next month. It is a small book about a harbour town, a \
                 letter and a debt. I would love for you to read it."
            ),
            "The Salt Road is out next month"
        );
        assert_eq!(title("Why not? Because."), "Why not?");
        assert_eq!(title("Stop! Nobody moves."), "Stop!");
        assert_eq!(title("\u{201c}Go on.\u{201d} She did not."), "\u{201c}Go on.\u{201d}");
        assert_eq!(title("Wait\u{2026} what was that? Nothing."), "Wait\u{2026}");
        // Short text is the title as it is; a lone sentence loses its
        // stop too, so the title does not change when a second one is
        // written after it.
        assert_eq!(title("Shopping list"), "Shopping list");
        assert_eq!(title("A note to self."), "A note to self");
        assert_eq!(title("Really?"), "Really?");
        assert_eq!(title("And then..."), "And then...");
        assert_eq!(title("Dr."), "Dr.");
        // Abbreviations, initials, numbers and addresses do not end it.
        assert_eq!(title("Dr. Moreau came at nine. He left at ten."), "Dr. Moreau came at nine");
        assert_eq!(title("J. R. R. Tolkien wrote it. So?"), "J. R. R. Tolkien wrote it");
        assert_eq!(title("It costs 3.50 at example.com today. Buy."), "It costs 3.50 at example.com today");
        assert_eq!(title("See e.g. the notes. Then go."), "See e.g. the notes");
        // No sentence end in reach: a cut at a word, with an ellipsis and
        // without the comma or the dash it would end on.
        let t = title(
            "A small book about a harbour town, a letter, and a debt nobody ever paid back in full",
        );
        assert_eq!(t, "A small book about a harbour town, a letter\u{2026}");
        assert!(t.chars().count() <= TITLE_TEXT_CHARS);
        let t = title(
            "One sentence that goes on for much longer than a title should. Then a second one.",
        );
        assert_eq!(t, "One sentence that goes on for much longer than\u{2026}");
        // A single unbroken word is cut inside, never inside a character.
        let t = title(&"\u{00e9}".repeat(200));
        assert_eq!(t.chars().count(), TITLE_TEXT_CHARS);
        assert!(t.ends_with('\u{2026}'));
        let family = "\u{1F469}\u{200D}\u{1F469}\u{200D}\u{1F467}".repeat(40);
        let t = title(&family);
        assert!(t.chars().count() <= TITLE_TEXT_CHARS);
        assert!(t.trim_end_matches('\u{2026}').chars().count() % 5 == 0, "{t:?}");

        // A heading is kept whole up to the longer limit, then cut the
        // same way.
        let heading = |text: &str| {
            Document::from_blocks(vec![Block::with_text(BlockKind::heading(1), text)])
                .title()
                .unwrap()
        };
        let sixty = "A heading that is itself a sentence. It stays as written here.";
        assert_eq!(heading(sixty), sixty);
        let t = heading(&"heading ".repeat(30));
        assert!(t.chars().count() <= TITLE_MAX_CHARS);
        assert!(t.ends_with("heading\u{2026}"), "{t}");
    }

    #[test]
    fn range_in_block_parts() {
        let d = Document::from_blocks(vec![
            Block::paragraph("hello"),
            Block::new(BlockKind::Divider),
            Block::paragraph("world"),
            Block::paragraph("next"),
        ]);
        let r = TextRange::new(Position::new(0, 2), Position::new(3, 0));
        assert_eq!(d.range_in_block(r, 0), Some(2..5));
        assert_eq!(d.range_in_block(r, 1), Some(0..0));
        assert_eq!(d.range_in_block(r, 2), Some(0..5));
        assert_eq!(d.range_in_block(r, 3), None);
        let r = TextRange::new(Position::new(0, 5), Position::new(1, 0));
        assert_eq!(d.range_in_block(r, 0), Some(5..5));
        assert_eq!(d.range_in_block(r, 1), Some(0..0));
        assert_eq!(d.range_in_block(TextRange::empty(Position::new(2, 1)), 2), None);
        assert_eq!(d.range_in_block(TextRange::in_block(2, 1, 3), 2), Some(1..3));
        assert_eq!(d.range_in_block(TextRange::in_block(2, 1, 3), 9), None);
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
