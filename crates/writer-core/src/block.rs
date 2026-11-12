//! Blocks: the paragraphs, headings, list items and so on of a document.

use std::collections::BTreeSet;
use std::fmt;
use std::ops::Range;

use crate::attrs::{CommentId, InlineAttrs};
use crate::runs::{self, Run};
use crate::text::{self, CharFix};

/// Stable identifier of a block, unique within its document.
///
/// Ids survive edits: typing, formatting and moving keep the id; splitting a
/// block keeps the id on the first half and mints a new one for the second;
/// merging keeps the id of the first block. Ids are minted by the document
/// and never reused within an editing session.
/// [`BlockId::UNASSIGNED`] (0) marks blocks that are not part of a document
/// yet (for example the blocks of a [`Fragment`](crate::Fragment)).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockId(pub u64);

impl BlockId {
    /// The id of a block that does not belong to a document.
    pub const UNASSIGNED: BlockId = BlockId(0);
}

impl fmt::Display for BlockId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "b{}", self.0)
    }
}

/// The kind of a block.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum BlockKind {
    /// Body text.
    #[default]
    Paragraph,
    /// A heading; `level` is always in `1..=6`.
    Heading {
        /// Heading level, 1 (largest) to 6.
        level: u8,
    },
    /// A bulleted list item.
    BulletItem,
    /// A numbered list item; see [`Document::numbered_ordinal`](crate::Document::numbered_ordinal).
    NumberedItem,
    /// A to-do list item.
    TodoItem {
        /// Whether the item is ticked.
        checked: bool,
    },
    /// A block quote.
    Quote,
    /// A code block: one block whose text holds several lines separated by
    /// `\n`. Its runs carry no formatting, only comment anchors.
    CodeBlock {
        /// Optional language tag (for example `"rust"`).
        language: Option<String>,
    },
    /// A horizontal rule. A void block: its text is always empty.
    Divider,
    /// A page break. A void block: its text is always empty.
    PageBreak,
}

impl BlockKind {
    /// A heading of the given level (clamped to `1..=6`).
    pub fn heading(level: u8) -> BlockKind {
        BlockKind::Heading {
            level: level.clamp(1, 6),
        }
    }

    /// An unchecked to-do item.
    pub fn todo() -> BlockKind {
        BlockKind::TodoItem { checked: false }
    }

    /// A code block without a language.
    pub fn code() -> BlockKind {
        BlockKind::CodeBlock { language: None }
    }

    /// Void blocks (divider, page break) have no text.
    pub fn is_void(&self) -> bool {
        matches!(self, BlockKind::Divider | BlockKind::PageBreak)
    }

    /// Bullet, numbered and to-do items.
    pub fn is_list_item(&self) -> bool {
        matches!(
            self,
            BlockKind::BulletItem | BlockKind::NumberedItem | BlockKind::TodoItem { .. }
        )
    }

    /// Whether this is a code block.
    pub fn is_code(&self) -> bool {
        matches!(self, BlockKind::CodeBlock { .. })
    }

    /// The heading level, if this is a heading.
    pub fn heading_level(&self) -> Option<u8> {
        match self {
            BlockKind::Heading { level } => Some(*level),
            _ => None,
        }
    }

    /// Whether two kinds are "the same kind" for toggling: equal variants,
    /// with the same heading level. The to-do checked flag and the code
    /// language are ignored.
    pub fn same_type(&self, other: &BlockKind) -> bool {
        match (self, other) {
            (BlockKind::Heading { level: a }, BlockKind::Heading { level: b }) => a == b,
            (BlockKind::TodoItem { .. }, BlockKind::TodoItem { .. }) => true,
            (BlockKind::CodeBlock { .. }, BlockKind::CodeBlock { .. }) => true,
            _ => std::mem::discriminant(self) == std::mem::discriminant(other),
        }
    }

    /// A short stable name (`"paragraph"`, `"heading"`, `"bullet"`,
    /// `"numbered"`, `"todo"`, `"quote"`, `"code"`, `"divider"`,
    /// `"page_break"`), used in the serialised format.
    pub fn name(&self) -> &'static str {
        match self {
            BlockKind::Paragraph => "paragraph",
            BlockKind::Heading { .. } => "heading",
            BlockKind::BulletItem => "bullet",
            BlockKind::NumberedItem => "numbered",
            BlockKind::TodoItem { .. } => "todo",
            BlockKind::Quote => "quote",
            BlockKind::CodeBlock { .. } => "code",
            BlockKind::Divider => "divider",
            BlockKind::PageBreak => "page_break",
        }
    }

    /// Repairs out-of-range data (heading level, empty language).
    pub(crate) fn sanitized(self) -> BlockKind {
        match self {
            BlockKind::Heading { level } => BlockKind::heading(level),
            BlockKind::CodeBlock { language } => BlockKind::CodeBlock {
                language: language
                    .map(|l| l.trim().to_string())
                    .filter(|l| !l.is_empty()),
            },
            k => k,
        }
    }
}

/// Horizontal alignment of a block.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Alignment {
    /// Flush left (the default).
    #[default]
    Left,
    /// Centred.
    Center,
    /// Flush right.
    Right,
    /// Justified.
    Justify,
}

impl Alignment {
    /// Stable lower-case name used in the serialised format.
    pub fn name(self) -> &'static str {
        match self {
            Alignment::Left => "left",
            Alignment::Center => "center",
            Alignment::Right => "right",
            Alignment::Justify => "justify",
        }
    }

    /// Parses a name produced by [`Alignment::name`].
    pub fn from_name(s: &str) -> Option<Alignment> {
        match s.trim().to_ascii_lowercase().as_str() {
            "left" => Some(Alignment::Left),
            "center" | "centre" => Some(Alignment::Center),
            "right" => Some(Alignment::Right),
            "justify" => Some(Alignment::Justify),
            _ => None,
        }
    }
}

/// Maximum indent level of a block.
pub const MAX_INDENT: u8 = 8;
/// Smallest allowed block text size override, in points.
pub const MIN_TEXT_SIZE: f32 = 6.0;
/// Largest allowed block text size override, in points.
pub const MAX_TEXT_SIZE: f32 = 144.0;
/// The sizes `step_text_size` walks through, in points.
pub const TEXT_SIZE_SCALE: [f32; 22] = [
    8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0, 17.0, 18.0, 20.0, 22.0, 24.0, 28.0, 32.0,
    36.0, 40.0, 48.0, 56.0, 64.0, 72.0,
];

/// Clamps a text size override; non-finite values become `None`.
pub fn sanitize_text_size(size: Option<f32>) -> Option<f32> {
    size.filter(|s| s.is_finite())
        .map(|s| s.clamp(MIN_TEXT_SIZE, MAX_TEXT_SIZE))
}

/// One block of a document.
///
/// Fields are private so that the invariants hold: text is normalised (no
/// control characters other than `\n` and `\t`), runs are normalised, void
/// blocks have no text, code blocks carry no formatting, the indent is at
/// most [`MAX_INDENT`].
///
/// Blocks are built with the constructors and `with_*` builders and turned
/// into a document with [`Document::from_blocks`](crate::Document::from_blocks),
/// which assigns ids. After that, all changes go through
/// [`EditorState`](crate::EditorState).
#[derive(Clone, Debug)]
pub struct Block {
    pub(crate) id: BlockId,
    pub(crate) revision: u64,
    pub(crate) kind: BlockKind,
    pub(crate) alignment: Alignment,
    pub(crate) indent: u8,
    pub(crate) text_size: Option<f32>,
    pub(crate) text: String,
    pub(crate) runs: Vec<Run>,
}

/// Content equality: compares everything except the revision stamp.
impl PartialEq for Block {
    fn eq(&self, other: &Block) -> bool {
        self.id == other.id
            && self.kind == other.kind
            && self.alignment == other.alignment
            && self.indent == other.indent
            && self.text_size == other.text_size
            && self.text == other.text
            && self.runs == other.runs
    }
}

impl Default for Block {
    fn default() -> Block {
        Block::new(BlockKind::Paragraph)
    }
}

impl Block {
    /// An empty block of the given kind.
    pub fn new(kind: BlockKind) -> Block {
        Block {
            id: BlockId::UNASSIGNED,
            revision: 0,
            kind: kind.sanitized(),
            alignment: Alignment::Left,
            indent: 0,
            text_size: None,
            text: String::new(),
            runs: Vec::new(),
        }
    }

    /// A block of `kind` holding plain `text` (normalised; ignored for void kinds).
    pub fn with_text(kind: BlockKind, text: &str) -> Block {
        Block::new(kind).with_span(text, InlineAttrs::default())
    }

    /// A paragraph holding plain `text`.
    pub fn paragraph(text: &str) -> Block {
        Block::with_text(BlockKind::Paragraph, text)
    }

    /// Builder: appends `text` with `attrs` (normalised; code blocks keep only
    /// comment anchors; ignored for void kinds).
    pub fn with_span(mut self, text: &str, attrs: InlineAttrs) -> Block {
        if self.kind.is_void() {
            return self;
        }
        let text = text::normalize_input(text);
        let mut attrs = attrs;
        attrs.sanitize();
        let at = self.text.len();
        self.insert(at, &text, &attrs);
        self
    }

    /// Builder: sets the alignment.
    pub fn with_alignment(mut self, alignment: Alignment) -> Block {
        self.alignment = alignment;
        self
    }

    /// Builder: sets the indent level (clamped to [`MAX_INDENT`]).
    pub fn with_indent(mut self, indent: u8) -> Block {
        self.indent = indent.min(MAX_INDENT);
        self
    }

    /// Builder: sets the text size override (clamped).
    pub fn with_text_size(mut self, size: Option<f32>) -> Block {
        self.text_size = sanitize_text_size(size);
        self
    }

    /// The stable id ([`BlockId::UNASSIGNED`] outside a document).
    pub fn id(&self) -> BlockId {
        self.id
    }

    /// A cheap stamp that changes whenever the block's content or attributes
    /// change. `(id, revision)` identifies one exact version of a block within
    /// one document session, so the UI can cache layout on it. Undo restores
    /// the stamp of the restored version. The stamp is not serialised: a
    /// freshly loaded document numbers its blocks from 1 again.
    ///
    /// Moving a block keeps its stamp. Things that depend on neighbours, such
    /// as a numbered item's ordinal, are not covered by it: take them from
    /// [`Document::numbered_ordinals`](crate::Document::numbered_ordinals)
    /// for the visible range instead of caching them with the layout.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// The block kind.
    pub fn kind(&self) -> &BlockKind {
        &self.kind
    }

    /// The alignment.
    pub fn alignment(&self) -> Alignment {
        self.alignment
    }

    /// The indent level, `0..=MAX_INDENT`.
    pub fn indent(&self) -> u8 {
        self.indent
    }

    /// The block-level text size override in points, if any. See
    /// [`Typography::size_for`](crate::Typography::size_for) for the size
    /// without an override.
    pub fn text_size(&self) -> Option<f32> {
        self.text_size
    }

    /// The text. `\n` is a soft line break (a new line inside a code block).
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Length of the text in bytes.
    pub fn len(&self) -> usize {
        self.text.len()
    }

    /// Whether the text is empty.
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Whether this is a void block (divider, page break).
    pub fn is_void(&self) -> bool {
        self.kind.is_void()
    }

    /// The normalised attribute runs.
    pub fn runs(&self) -> &[Run] {
        &self.runs
    }

    /// Iterates `(byte range, attrs)` over the runs, in order.
    pub fn spans(&self) -> impl Iterator<Item = (Range<usize>, &InlineAttrs)> {
        runs::spans(&self.runs)
    }

    /// Iterates `(byte range, text, attrs)` over the runs, in order.
    pub fn text_spans(&self) -> impl Iterator<Item = (Range<usize>, &str, &InlineAttrs)> {
        self.spans().map(|(r, a)| (r.clone(), &self.text[r], a))
    }

    /// The attributes of the character starting at `offset`, if any.
    pub fn attrs_at(&self, offset: usize) -> Option<&InlineAttrs> {
        runs::attrs_at(&self.runs, offset)
    }

    /// The attributes of the character ending at `offset`, if any.
    pub fn attrs_before(&self, offset: usize) -> Option<&InlineAttrs> {
        let o = text::floor_char_boundary(&self.text, offset);
        if o == 0 {
            return None;
        }
        let p = text::floor_char_boundary(&self.text, o - 1);
        self.attrs_at(p)
    }

    /// Every comment id anchored somewhere in this block.
    pub fn comment_ids(&self) -> BTreeSet<CommentId> {
        let mut ids = BTreeSet::new();
        for r in &self.runs {
            ids.extend(r.attrs.comments.iter().copied());
        }
        ids
    }

    /// Converts a byte offset in this block to a UTF-16 offset.
    pub fn offset_to_utf16(&self, offset: usize) -> usize {
        text::utf8_to_utf16(&self.text, offset)
    }

    /// Converts a UTF-16 offset in this block to a byte offset.
    pub fn utf16_to_offset(&self, offset16: usize) -> usize {
        text::utf16_to_utf8(&self.text, offset16)
    }

    /// The hard line (between `\n`s) containing `offset`.
    pub fn line_range(&self, offset: usize) -> Range<usize> {
        text::line_start(&self.text, offset)..text::line_end(&self.text, offset)
    }

    // ----- crate-private mutation -------------------------------------------------

    pub(crate) fn mark_dirty(&mut self) {
        self.revision = 0;
    }

    /// Changes the kind, keeping text; void kinds drop the text, code blocks
    /// drop formatting.
    pub(crate) fn set_kind(&mut self, kind: BlockKind) {
        self.kind = kind.sanitized();
        if self.kind.is_void() {
            self.text.clear();
            self.runs.clear();
        } else if self.kind.is_code() {
            for r in &mut self.runs {
                r.attrs = r.attrs.code_block_subset();
            }
            runs::normalize(&mut self.runs);
        }
        self.mark_dirty();
    }

    pub(crate) fn set_alignment(&mut self, a: Alignment) {
        self.alignment = a;
        self.mark_dirty();
    }

    pub(crate) fn set_indent(&mut self, indent: u8) {
        self.indent = indent.min(MAX_INDENT);
        self.mark_dirty();
    }

    pub(crate) fn set_text_size(&mut self, size: Option<f32>) {
        self.text_size = sanitize_text_size(size);
        self.mark_dirty();
    }

    /// Inserts already-normalised `text` with `attrs` at a char boundary.
    pub(crate) fn insert(&mut self, offset: usize, text: &str, attrs: &InlineAttrs) {
        if text.is_empty() || self.kind.is_void() {
            return;
        }
        let offset = text::floor_char_boundary(&self.text, offset);
        self.text.insert_str(offset, text);
        if self.kind.is_code() {
            runs::insert(
                &mut self.runs,
                offset,
                text.len(),
                &attrs.code_block_subset(),
            );
        } else {
            runs::insert(&mut self.runs, offset, text.len(), attrs);
        }
        self.mark_dirty();
    }

    /// Inserts the text and runs of `content` (another block's content) at `offset`.
    pub(crate) fn insert_content(&mut self, offset: usize, content: &Block) {
        if content.text.is_empty() || self.kind.is_void() {
            return;
        }
        let offset = text::floor_char_boundary(&self.text, offset);
        self.text.insert_str(offset, &content.text);
        if self.kind.is_code() {
            let stripped: Vec<Run> = content
                .runs
                .iter()
                .map(|r| Run {
                    len: r.len,
                    attrs: r.attrs.code_block_subset(),
                })
                .collect();
            runs::insert_runs(&mut self.runs, offset, &stripped);
        } else {
            runs::insert_runs(&mut self.runs, offset, &content.runs);
        }
        self.mark_dirty();
    }

    /// Removes the bytes in `range` (clamped, char boundaries).
    pub(crate) fn remove(&mut self, range: Range<usize>) {
        let s = text::floor_char_boundary(&self.text, range.start);
        let e = text::floor_char_boundary(&self.text, range.end);
        if s >= e {
            return;
        }
        self.text.replace_range(s..e, "");
        runs::remove(&mut self.runs, s..e);
        self.mark_dirty();
    }

    /// Replaces `range` (char boundaries) with already-normalised `rep`.
    /// When `rep` has as many chars as the replaced text, each new char
    /// takes the attributes of the old char at the same index, so a case
    /// change or a same-length correction keeps per-character formatting;
    /// otherwise the whole replacement takes `fallback`.
    pub(crate) fn replace_text(&mut self, range: Range<usize>, rep: &str, fallback: &InlineAttrs) {
        let s = text::floor_char_boundary(&self.text, range.start);
        let e = text::floor_char_boundary(&self.text, range.end).max(s);
        let old = &self.text[s..e];
        if !old.is_empty() && old.chars().count() == rep.chars().count() {
            let mut piece = Block::new(BlockKind::Paragraph);
            piece.text = rep.to_string();
            for ((i, _), c) in old.char_indices().zip(rep.chars()) {
                let attrs = runs::attrs_at(&self.runs, s + i)
                    .cloned()
                    .unwrap_or_default();
                match piece.runs.last_mut() {
                    Some(last) if last.attrs == attrs => last.len += c.len_utf8(),
                    _ => piece.runs.push(Run {
                        len: c.len_utf8(),
                        attrs,
                    }),
                }
            }
            self.remove(s..e);
            self.insert_content(s, &piece);
        } else {
            self.remove(s..e);
            self.insert(s, rep, fallback);
        }
        self.mark_dirty();
    }

    /// Splits at `offset`: `self` keeps the head (and its id), the returned
    /// block holds the tail with the same kind and properties and no id.
    pub(crate) fn split_off(&mut self, offset: usize) -> Block {
        let o = text::floor_char_boundary(&self.text, offset);
        let tail_text = self.text.split_off(o);
        let tail_runs = runs::slice(&self.runs, o..o + tail_text.len());
        runs::remove(&mut self.runs, o..o + tail_text.len());
        self.mark_dirty();
        Block {
            id: BlockId::UNASSIGNED,
            revision: 0,
            kind: self.kind.clone(),
            alignment: self.alignment,
            indent: self.indent,
            text_size: self.text_size,
            text: tail_text,
            runs: tail_runs,
        }
    }

    /// Appends the content of `other` (kind and properties of `self` stay).
    pub(crate) fn append(&mut self, other: &Block) {
        let at = self.text.len();
        self.insert_content(at, other);
    }

    /// A copy of the content in `range` with the same kind and properties
    /// and no id.
    pub(crate) fn slice(&self, range: Range<usize>) -> Block {
        let s = text::floor_char_boundary(&self.text, range.start);
        let e = text::floor_char_boundary(&self.text, range.end).max(s);
        Block {
            id: BlockId::UNASSIGNED,
            revision: 0,
            kind: self.kind.clone(),
            alignment: self.alignment,
            indent: self.indent,
            text_size: self.text_size,
            text: self.text[s..e].to_string(),
            runs: runs::slice(&self.runs, s..e),
        }
    }

    /// Applies `f` to the attributes of `range`. Code blocks only accept
    /// changes to comment anchors.
    pub(crate) fn apply_attrs(&mut self, range: Range<usize>, f: impl FnMut(&mut InlineAttrs)) {
        let s = text::floor_char_boundary(&self.text, range.start);
        let e = text::floor_char_boundary(&self.text, range.end);
        if s >= e {
            return;
        }
        runs::apply(&mut self.runs, s..e, f);
        if self.kind.is_code() {
            for r in &mut self.runs {
                if !(r.attrs.code_block_subset() == r.attrs) {
                    r.attrs = r.attrs.code_block_subset();
                }
            }
            runs::normalize(&mut self.runs);
        }
        self.mark_dirty();
    }

    /// Removes `id` from every run. Returns whether anything changed.
    pub(crate) fn remove_comment(&mut self, id: CommentId) -> bool {
        let mut changed = false;
        for r in &mut self.runs {
            changed |= r.attrs.comments.remove(&id);
        }
        if changed {
            runs::normalize(&mut self.runs);
            self.mark_dirty();
        }
        changed
    }

    /// Removes every comment anchor.
    pub(crate) fn strip_comments(&mut self) {
        if self.runs.iter().any(|r| !r.attrs.comments.is_empty()) {
            for r in &mut self.runs {
                r.attrs.comments.clear();
            }
            runs::normalize(&mut self.runs);
            self.mark_dirty();
        }
    }

    /// Repairs every invariant of a block that may come from untrusted
    /// input. A void block that carries text keeps the void and moves the
    /// text into a following paragraph, which is returned.
    pub(crate) fn repair(&mut self) -> Option<Block> {
        self.kind = self.kind.clone().sanitized();
        self.indent = self.indent.min(MAX_INDENT);
        self.text_size = sanitize_text_size(self.text_size);
        let (text, runs) = rebuild(&self.text, &self.runs);
        self.text = text;
        self.runs = runs;
        for r in &mut self.runs {
            r.attrs.sanitize();
        }
        runs::normalize(&mut self.runs);
        self.mark_dirty();
        if self.kind.is_void() && !self.text.is_empty() {
            let mut p = Block::new(BlockKind::Paragraph);
            p.text = std::mem::take(&mut self.text);
            p.runs = std::mem::take(&mut self.runs);
            p.alignment = self.alignment;
            p.indent = self.indent;
            return Some(p);
        }
        if self.kind.is_code() {
            let kind = self.kind.clone();
            self.set_kind(kind);
        }
        None
    }

    /// Checks the block invariants.
    pub(crate) fn check(&self) -> Result<(), String> {
        if !text::is_normalized(&self.text) {
            return Err(format!(
                "block {}: text contains control characters",
                self.id
            ));
        }
        if !runs::is_normalized(&self.runs, self.text.len()) {
            return Err(format!("block {}: runs are not normalised", self.id));
        }
        let mut pos = 0;
        for r in &self.runs {
            pos += r.len;
            if !self.text.is_char_boundary(pos) {
                return Err(format!(
                    "block {}: run boundary {pos} inside a char",
                    self.id
                ));
            }
            if r.attrs
                .font
                .as_deref()
                .is_some_and(|f| f.trim().is_empty() || f.trim() != f)
                || r.attrs
                    .link
                    .as_deref()
                    .is_some_and(|l| l.trim().is_empty() || l.trim() != l)
            {
                return Err(format!("block {}: unsanitised font or link", self.id));
            }
        }
        if self.kind.is_void() && !self.text.is_empty() {
            return Err(format!("block {}: void block has text", self.id));
        }
        if self.kind.is_code()
            && self
                .runs
                .iter()
                .any(|r| r.attrs != r.attrs.code_block_subset())
        {
            return Err(format!("block {}: code block carries formatting", self.id));
        }
        if self.indent > MAX_INDENT {
            return Err(format!("block {}: indent too large", self.id));
        }
        if let BlockKind::Heading { level } = self.kind
            && !(1..=6).contains(&level)
        {
            return Err(format!("block {}: bad heading level", self.id));
        }
        if self.text_size != sanitize_text_size(self.text_size) {
            return Err(format!("block {}: bad text size", self.id));
        }
        if self.kind != self.kind.clone().sanitized() {
            return Err(format!("block {}: unsanitised kind", self.id));
        }
        Ok(())
    }
}

/// Rebuilds possibly malformed text and runs: normalises control characters
/// and makes the runs cover the text exactly, each character taking the
/// attributes of the run containing its first byte.
pub(crate) fn rebuild(text: &str, src_runs: &[Run]) -> (String, Vec<Run>) {
    let mut out = String::with_capacity(text.len());
    let mut out_runs: Vec<Run> = Vec::new();
    // Cursor over source runs by byte position.
    let mut run_idx = 0usize;
    let mut run_end = src_runs.first().map_or(0, |r| r.len);
    let fallback = InlineAttrs::default();
    let attrs_for = |byte: usize, run_idx: &mut usize, run_end: &mut usize| -> InlineAttrs {
        while *run_idx < src_runs.len() && byte >= *run_end {
            *run_idx += 1;
            if *run_idx < src_runs.len() {
                *run_end = run_end.saturating_add(src_runs[*run_idx].len);
            }
        }
        if *run_idx < src_runs.len() {
            src_runs[*run_idx].attrs.clone()
        } else {
            src_runs
                .last()
                .map_or_else(|| fallback.clone(), |r| r.attrs.clone())
        }
    };
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        let piece: Option<char> = match text::normalize_char(c) {
            CharFix::Keep => Some(c),
            CharFix::Drop => None,
            CharFix::Newline => {
                if c == '\r' && chars.peek().map(|(_, n)| *n) == Some('\n') {
                    None
                } else {
                    Some('\n')
                }
            }
        };
        if let Some(p) = piece {
            let attrs = attrs_for(i, &mut run_idx, &mut run_end);
            out.push(p);
            let len = p.len_utf8();
            match out_runs.last_mut() {
                Some(last) if last.attrs == attrs => last.len += len,
                _ => out_runs.push(Run { len, attrs }),
            }
        }
    }
    (out, out_runs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builders_normalize() {
        let b = Block::paragraph("a\r\nb\u{7}c");
        assert_eq!(b.text(), "a\nbc");
        assert!(b.check().is_ok());
        let d = Block::with_text(BlockKind::Divider, "ignored");
        assert!(d.is_empty());
        let bold = InlineAttrs {
            bold: true,
            ..Default::default()
        };
        let b = Block::paragraph("x")
            .with_span("yz", bold.clone())
            .with_indent(99);
        assert_eq!(b.runs().len(), 2);
        assert_eq!(b.indent(), MAX_INDENT);
        let c = Block::with_text(BlockKind::code(), "fn").with_span("()", bold);
        assert_eq!(c.runs().len(), 1);
        assert!(c.check().is_ok());
        assert_eq!(BlockKind::heading(9), BlockKind::Heading { level: 6 });
    }

    #[test]
    fn split_and_append() {
        let bold = InlineAttrs {
            bold: true,
            ..Default::default()
        };
        let mut b = Block::paragraph("hello ").with_span("world", bold);
        let tail = b.split_off(3);
        assert_eq!(b.text(), "hel");
        assert_eq!(tail.text(), "lo world");
        assert_eq!(tail.runs().len(), 2);
        b.append(&tail);
        assert_eq!(b.text(), "hello world");
        assert_eq!(b.runs().len(), 2);
        assert!(b.check().is_ok());
    }

    #[test]
    fn rebuild_repairs_runs() {
        let bold = InlineAttrs {
            bold: true,
            ..Default::default()
        };
        // Runs too short, boundary inside a multi-byte char, control chars.
        let runs = vec![
            Run {
                len: 2,
                attrs: bold.clone(),
            },
            Run {
                len: 0,
                attrs: InlineAttrs::default(),
            },
        ];
        let (t, r) = rebuild("aé\r\nz\u{1}", &runs);
        assert_eq!(t, "aé\nz");
        assert!(runs::is_normalized(&r, t.len()));
        let (t, r) = rebuild("abc", &[]);
        assert_eq!(t, "abc");
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn void_with_text_is_repaired() {
        let mut d = Block::new(BlockKind::Divider);
        d.text = "oops".into();
        d.runs = vec![Run {
            len: 4,
            attrs: InlineAttrs::default(),
        }];
        let extra = d.repair().expect("text moved out");
        assert!(d.is_empty());
        assert_eq!(extra.text(), "oops");
    }
}
