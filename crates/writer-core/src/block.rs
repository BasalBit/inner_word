//! Blocks: the paragraphs, headings, list items and so on of a document.

use std::collections::BTreeSet;
use std::fmt;
use std::ops::Range;

use crate::attrs::{CommentId, InlineAttrs};
use crate::runs::{self, Run};
use crate::settings::PageSetup;
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

/// Stored image bytes, named by their content: the first 16 bytes of the
/// SHA-256 of the stored bytes. Written as 32 lowercase hex digits (in the
/// serialised document, in Markdown, in the library).
///
/// This crate never sees the bytes: it has no IO. Whoever stores them makes
/// the id, and the same bytes always have the same id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ImageId([u8; 16]);

impl ImageId {
    /// The id with these 16 bytes.
    pub fn from_bytes(id: [u8; 16]) -> ImageId {
        ImageId(id)
    }

    /// Reads an id as [`Display`](fmt::Display) writes it: exactly 32 hex
    /// digits, in either case, and nothing else (no spaces, no prefix).
    pub fn parse(text: &str) -> Option<ImageId> {
        let digits = text.as_bytes();
        if digits.len() != 32 {
            return None;
        }
        let value = |digit: u8| -> Option<u8> {
            match digit {
                b'0'..=b'9' => Some(digit - b'0'),
                b'a'..=b'f' => Some(digit - b'a' + 10),
                b'A'..=b'F' => Some(digit - b'A' + 10),
                _ => None,
            }
        };
        let mut id = [0u8; 16];
        for (byte, pair) in id.iter_mut().zip(digits.chunks_exact(2)) {
            *byte = value(pair[0])? << 4 | value(pair[1])?;
        }
        Some(ImageId(id))
    }

    /// The 16 bytes.
    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

/// 32 lowercase hex digits.
impl fmt::Display for ImageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in &self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// How wide an image is drawn: a share of the text column, in thousandths
/// (`500` is half the column). Always within [`ImageWidth::MIN`] and
/// [`ImageWidth::FULL`].
///
/// A share and not a length, because the column is not the same everywhere
/// (the two views, another paper, an export): half the column is half the
/// column in all of them. An integer, because [`BlockKind`] is hashed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ImageWidth(u16);

impl ImageWidth {
    /// The narrowest an image is drawn: a twentieth of the column.
    pub const MIN: ImageWidth = ImageWidth(50);
    /// The whole column.
    pub const FULL: ImageWidth = ImageWidth(1000);
    /// What "larger" and "smaller" step through, in thousandths.
    pub const STEPS: [u16; 13] = [
        100, 150, 200, 250, 333, 400, 500, 600, 667, 750, 800, 900, 1000,
    ];

    /// A width of so many thousandths of the column, clamped to
    /// [`MIN`](ImageWidth::MIN)..=[`FULL`](ImageWidth::FULL).
    pub fn new(thousandths: u16) -> ImageWidth {
        ImageWidth(thousandths.clamp(ImageWidth::MIN.0, ImageWidth::FULL.0))
    }

    /// A width from a share of the column (`0.5` is half of it), rounded to
    /// a thousandth and clamped. Something that is no number is the whole
    /// column.
    pub fn from_fraction(fraction: f32) -> ImageWidth {
        if fraction.is_nan() {
            return ImageWidth::FULL;
        }
        // The cast saturates, so any share beyond the ends is clamped.
        ImageWidth::new((f64::from(fraction) * 1000.0).round() as u16)
    }

    /// A width from a percentage of the column, clamped.
    pub fn from_percent(percent: u16) -> ImageWidth {
        ImageWidth::new(percent.saturating_mul(10))
    }

    /// The width in thousandths of the column, `50..=1000`.
    pub fn thousandths(self) -> u16 {
        self.0
    }

    /// The width as a share of the column, `0.05..=1.0`.
    pub fn fraction(self) -> f32 {
        f32::from(self.0) / 1000.0
    }

    /// The width as a percentage of the column, rounded: what is shown as
    /// "62 %".
    pub fn percent(self) -> u16 {
        (self.0 + 5) / 10
    }

    /// The share at which something `natural` points wide is drawn at its
    /// own size in a column of `column` points; [`FULL`](ImageWidth::FULL)
    /// when it is wider than the column (or the column has no width).
    pub fn natural(natural: f32, column: f32) -> ImageWidth {
        if !column.is_finite() || column <= 0.0 {
            return ImageWidth::FULL;
        }
        ImageWidth::from_fraction(natural / column)
    }

    /// The next of [`STEPS`](ImageWidth::STEPS) above (`steps > 0`) or below
    /// (`steps < 0`) this width, as many times as `steps` says. It stays at
    /// the ends: nothing is above the last step, and nothing below the
    /// first (a width that is narrower than the first step already stays
    /// what it is). From a width between two steps the first step lands on
    /// the neighbour.
    pub fn stepped(self, steps: i32) -> ImageWidth {
        let mut width = self.0;
        for _ in 0..steps.unsigned_abs() {
            let next = if steps > 0 {
                ImageWidth::STEPS.iter().copied().find(|&s| s > width)
            } else {
                ImageWidth::STEPS.iter().copied().rev().find(|&s| s < width)
            };
            match next {
                Some(next) => width = next,
                None => break,
            }
        }
        ImageWidth::new(width)
    }
}

/// An image in a document: which stored bytes, what is known about them
/// without reading them, and how they are shown.
///
/// The bytes are not here. They are kept by whoever keeps the document
/// (the library), under [`Image::id`]; the pixel size and the density are
/// copied into the document so that layout, page breaks and exports never
/// have to ask for the bytes to know how large the picture is. The id is
/// made from the bytes, so what is copied here cannot go stale.
///
/// An image is the [`BlockKind::Image`] of a block on a line of its own;
/// the block's [`Alignment`] says where on the line it stands.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Image {
    /// The stored bytes.
    pub id: ImageId,
    /// Pixel size of the stored bytes, upright, `(width, height)`. Never
    /// zero. Layout takes the aspect ratio from here and never asks the
    /// library.
    pub pixels: (u32, u32),
    /// Pixels per inch the file declares (72 when it says nothing; 144 for
    /// a Retina screenshot). Within
    /// [`MIN_DENSITY`](Image::MIN_DENSITY)..=[`MAX_DENSITY`](Image::MAX_DENSITY).
    pub density: u16,
    /// How wide the image is drawn.
    pub width: ImageWidth,
    /// What the image shows, for exports, agents and whoever cannot see it.
    /// One line, no control characters, at most
    /// [`MAX_DESCRIPTION_CHARS`](Image::MAX_DESCRIPTION_CHARS) characters,
    /// trimmed; may be empty.
    pub description: String,
}

impl Image {
    /// The density of a file that declares none, in pixels per inch.
    pub const DEFAULT_DENSITY: u16 = 72;
    /// The lowest density an image keeps, in pixels per inch.
    pub const MIN_DENSITY: u16 = 36;
    /// The highest density an image keeps, in pixels per inch.
    pub const MAX_DENSITY: u16 = 1200;
    /// The longest description, in characters.
    pub const MAX_DESCRIPTION_CHARS: usize = 1000;

    /// An image as wide as the column and without a description. A pixel
    /// size of zero becomes one, the density is clamped.
    pub fn new(id: ImageId, pixels: (u32, u32), density: u16) -> Image {
        Image {
            id,
            pixels,
            density,
            width: ImageWidth::FULL,
            description: String::new(),
        }
        .sanitized()
    }

    /// The size of the image at its own density, in points
    /// (`pixels * 72 / density`): 1600 × 1067 pixels at 144 to the inch
    /// are 800 × 533.5 points.
    pub fn natural_points(&self) -> (f32, f32) {
        let density = self.density.clamp(Image::MIN_DENSITY, Image::MAX_DENSITY);
        let points = |pixels: u32| (f64::from(pixels.max(1)) * 72.0 / f64::from(density)) as f32;
        (points(self.pixels.0), points(self.pixels.1))
    }

    /// Height over width.
    pub fn aspect(&self) -> f32 {
        (f64::from(self.pixels.1.max(1)) / f64::from(self.pixels.0.max(1))) as f32
    }

    /// The widest this image is drawn on `page`: the whole column, or less
    /// where it would then be taller than the page's text area (an image
    /// is never cut in two by a page break, so it has to fit on one page).
    ///
    /// With `(w, h)` the text area of the page
    /// ([`PageSetup::content_size`]) the width is
    /// `floor(1000 * min(1, floor(h) / (aspect * w)))` thousandths, and at
    /// least [`ImageWidth::MIN`]. Both roundings go down, so the image is
    /// a little lower than the text area and never a little higher.
    pub fn max_width(&self, page: &PageSetup) -> ImageWidth {
        let (w, h) = page.content_size();
        let height_at_full = f64::from(self.aspect()) * f64::from(w);
        if height_at_full.is_nan() || height_at_full <= 0.0 {
            return ImageWidth::FULL;
        }
        let share = (f64::from(h.floor()) / height_at_full).min(1.0);
        ImageWidth::new((1000.0 * share).floor() as u16)
    }

    /// "Original size": the share of the column at which the image has its
    /// own size on `page` ([`ImageWidth::natural`] of its natural width in
    /// the page's text area), and not more than
    /// [`max_width`](Image::max_width). The one definition for the app, the
    /// importer and agents.
    pub fn original_width(&self, page: &PageSetup) -> ImageWidth {
        let (w, _) = page.content_size();
        ImageWidth::natural(self.natural_points().0, w).min(self.max_width(page))
    }

    /// Repairs out-of-range data: a pixel size of zero, the density, the
    /// width, the description.
    pub(crate) fn sanitized(mut self) -> Image {
        self.pixels = (self.pixels.0.max(1), self.pixels.1.max(1));
        self.density = self.density.clamp(Image::MIN_DENSITY, Image::MAX_DENSITY);
        self.width = ImageWidth::new(self.width.0);
        if !is_clean_description(&self.description) {
            self.description = clean_description(&self.description);
        }
        self
    }
}

/// A description as an image keeps it: one line (every control character,
/// line and paragraph separators among them, becomes a space, several in a
/// row one space), trimmed, and cut to
/// [`Image::MAX_DESCRIPTION_CHARS`] characters.
fn clean_description(text: &str) -> String {
    let mut line = String::with_capacity(text.len());
    let mut after_control = false;
    for c in text.chars() {
        if breaks_a_description(c) {
            if !after_control {
                line.push(' ');
            }
            after_control = true;
        } else {
            line.push(c);
            after_control = false;
        }
    }
    let line = line.trim();
    let cut = match line.char_indices().nth(Image::MAX_DESCRIPTION_CHARS) {
        Some((end, _)) => &line[..end],
        None => line,
    };
    cut.trim_end().to_string()
}

/// Whether [`clean_description`] would leave `text` as it is.
fn is_clean_description(text: &str) -> bool {
    text.trim() == text
        && !text.chars().any(breaks_a_description)
        && text.chars().count() <= Image::MAX_DESCRIPTION_CHARS
}

/// The characters a description cannot hold: the control characters and
/// the two separators that end a line.
fn breaks_a_description(c: char) -> bool {
    c.is_control() || matches!(c, '\u{2028}' | '\u{2029}')
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
    /// An image on a line of its own. A void block: its text is always
    /// empty, and so are its indent and its text size; its alignment is
    /// left, centre or right.
    Image(Image),
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

    /// Void blocks (divider, page break, image) have no text.
    pub fn is_void(&self) -> bool {
        matches!(
            self,
            BlockKind::Divider | BlockKind::PageBreak | BlockKind::Image(_)
        )
    }

    /// Whether this is an image.
    pub fn is_image(&self) -> bool {
        matches!(self, BlockKind::Image(_))
    }

    /// The image, if this is one.
    pub fn image(&self) -> Option<&Image> {
        match self {
            BlockKind::Image(image) => Some(image),
            _ => None,
        }
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
    /// `"page_break"`, `"image"`), used in the serialised format.
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
            BlockKind::Image(_) => "image",
        }
    }

    /// Repairs out-of-range data (heading level, empty language, what an
    /// image holds).
    pub(crate) fn sanitized(self) -> BlockKind {
        match self {
            BlockKind::Heading { level } => BlockKind::heading(level),
            BlockKind::Image(image) => BlockKind::Image(image.sanitized()),
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
/// most [`MAX_INDENT`]. An image block has, besides no text, no indent and
/// no text size, and is never justified (asked for, that is left).
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

    /// An image block (flush left; see [`Block::with_alignment`]).
    pub fn image(image: Image) -> Block {
        Block::new(BlockKind::Image(image))
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

    /// Builder: sets the alignment (an image cannot be justified: it is
    /// then flush left).
    pub fn with_alignment(mut self, alignment: Alignment) -> Block {
        self.alignment = alignment;
        self.hold_image_invariants();
        self
    }

    /// Builder: sets the indent level (clamped to [`MAX_INDENT`]; an image
    /// has none).
    pub fn with_indent(mut self, indent: u8) -> Block {
        self.indent = indent.min(MAX_INDENT);
        self.hold_image_invariants();
        self
    }

    /// Builder: sets the text size override (clamped; an image has none).
    pub fn with_text_size(mut self, size: Option<f32>) -> Block {
        self.text_size = sanitize_text_size(size);
        self.hold_image_invariants();
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

    /// Whether this is a void block (divider, page break, image).
    pub fn is_void(&self) -> bool {
        self.kind.is_void()
    }

    /// The image, if this is an image block.
    pub fn as_image(&self) -> Option<&Image> {
        self.kind.image()
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

    /// What an image block cannot have, besides text: an indent, a text
    /// size, justified alignment. Called wherever one of the three, or the
    /// kind, is set; does nothing to any other block.
    fn hold_image_invariants(&mut self) {
        if self.kind.is_image() {
            self.indent = 0;
            self.text_size = None;
            if self.alignment == Alignment::Justify {
                self.alignment = Alignment::Left;
            }
        }
    }

    /// Changes the kind, keeping text; void kinds drop the text, code blocks
    /// drop formatting, an image also drops the indent and the text size
    /// and is flush left where the block was justified.
    pub(crate) fn set_kind(&mut self, kind: BlockKind) {
        self.kind = kind.sanitized();
        if self.kind.is_void() {
            self.text.clear();
            self.runs.clear();
            self.hold_image_invariants();
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
        self.hold_image_invariants();
        self.mark_dirty();
    }

    pub(crate) fn set_indent(&mut self, indent: u8) {
        self.indent = indent.min(MAX_INDENT);
        self.hold_image_invariants();
        self.mark_dirty();
    }

    pub(crate) fn set_text_size(&mut self, size: Option<f32>) {
        self.text_size = sanitize_text_size(size);
        self.hold_image_invariants();
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
    /// text into a following paragraph, which is returned (with the
    /// alignment and the indent the block came with). An image block is
    /// then left without indent and text size, and flush left where it was
    /// justified.
    pub(crate) fn repair(&mut self) -> Option<Block> {
        self.kind = self.kind.clone().sanitized();
        self.indent = self.indent.min(MAX_INDENT);
        self.text_size = sanitize_text_size(self.text_size);
        // A block the app wrote itself needs no rebuilding, and going
        // through its text character by character is most of what reading
        // a book costs: see `needs_rebuild`.
        if needs_rebuild(&self.text, &self.runs) {
            let (text, runs) = rebuild(&self.text, &self.runs);
            self.text = text;
            self.runs = runs;
        }
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
            self.hold_image_invariants();
            return Some(p);
        }
        self.hold_image_invariants();
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
        if let BlockKind::Image(image) = &self.kind {
            if self.indent != 0 {
                return Err(format!("block {}: image block has an indent", self.id));
            }
            if self.text_size.is_some() {
                return Err(format!("block {}: image block has a text size", self.id));
            }
            if self.alignment == Alignment::Justify {
                return Err(format!("block {}: image block is justified", self.id));
            }
            if image.pixels.0 == 0 || image.pixels.1 == 0 {
                return Err(format!("block {}: image has no pixels", self.id));
            }
            if !(Image::MIN_DENSITY..=Image::MAX_DENSITY).contains(&image.density) {
                return Err(format!("block {}: image density out of range", self.id));
            }
            if image.width < ImageWidth::MIN || image.width > ImageWidth::FULL {
                return Err(format!("block {}: image width out of range", self.id));
            }
            if !is_clean_description(&image.description) {
                return Err(format!(
                    "block {}: image description is not one clean line",
                    self.id
                ));
            }
        }
        if self.kind != self.kind.clone().sanitized() {
            return Err(format!("block {}: unsanitised kind", self.id));
        }
        Ok(())
    }
}

/// Whether [`rebuild`] would change anything that [`runs::normalize`] does
/// not put right by itself: the text has a character to drop or to turn
/// into a line break, or the runs do not cover it exactly, or a run ends
/// inside a character. Where none of that is so, every character already
/// has the attributes of the run it lies in, and rebuilding would give the
/// same text and the same runs, less the empty ones and with equal
/// neighbours joined, which is what normalising does.
pub(crate) fn needs_rebuild(text: &str, runs: &[Run]) -> bool {
    let mut end = 0usize;
    for run in runs {
        end = match end.checked_add(run.len) {
            Some(end) if end <= text.len() && text.is_char_boundary(end) => end,
            _ => return true,
        };
    }
    end != text.len() || !text::is_normalized(text)
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

    /// `repair` leaves a block that needs no rebuilding as it is. That is
    /// only right when rebuilding it would have given the same block: here
    /// both ways are taken on thousands of texts with runs, well formed
    /// and not, and compared.
    #[test]
    fn a_block_that_is_not_rebuilt_is_what_rebuilding_would_give() {
        // The repair of text and runs as it was before the short cut.
        let the_long_way = |text: &str, runs: &[Run]| -> (String, Vec<Run>) {
            let (text, mut runs) = rebuild(text, runs);
            for run in &mut runs {
                run.attrs.sanitize();
            }
            runs::normalize(&mut runs);
            (text, runs)
        };
        let attrs = |n: u64| InlineAttrs {
            bold: n & 1 == 1,
            italic: n & 2 == 2,
            font: match n % 5 {
                0 => Some("Georgia".to_string()),
                // Sanitised away, so two runs that differ only in this
                // are one run afterwards.
                1 => Some("  ".to_string()),
                2 => Some(" Georgia ".to_string()),
                _ => None,
            },
            link: n
                .is_multiple_of(7)
                .then(|| " https://example.org".to_string()),
            ..Default::default()
        };
        let pieces = [
            "a",
            "b",
            " ",
            "é",
            "中",
            "\u{1F469}\u{200D}\u{1F467}",
            "e\u{301}",
            "\n",
            "\t",
            "word",
        ];
        let bad = ["\r", "\r\n", "\u{7}", "\u{85}", "\u{2028}", "\u{0}"];
        // xorshift64*, as in tests/random.rs.
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = move || {
            state ^= state >> 12;
            state ^= state << 25;
            state ^= state >> 27;
            state.wrapping_mul(0x2545_F491_4F6C_DD1D)
        };
        let (mut kept, mut rebuilt) = (0, 0);
        for round in 0..6000u32 {
            // Text in runs that fit it, some of them empty, some equal to
            // their neighbour.
            let mut text = String::new();
            let mut runs: Vec<Run> = Vec::new();
            for _ in 0..next() % 6 {
                let mut len = 0;
                for _ in 0..next() % 4 {
                    let piece = pieces[(next() % pieces.len() as u64) as usize];
                    text.push_str(piece);
                    len += piece.len();
                }
                runs.push(Run {
                    len,
                    attrs: attrs(next() % 4),
                });
            }
            // Every third round something is wrong with it.
            if round.is_multiple_of(3) {
                match next() % 6 {
                    // A character that does not belong in a text, in a
                    // run of its own, so only the text is wrong.
                    0 => {
                        let piece = bad[(next() % bad.len() as u64) as usize];
                        text.push_str(piece);
                        runs.push(Run {
                            len: piece.len(),
                            attrs: attrs(next() % 4),
                        });
                    }
                    1 => {
                        let piece = bad[(next() % bad.len() as u64) as usize];
                        text.insert_str(0, piece);
                        runs.insert(
                            0,
                            Run {
                                len: piece.len(),
                                attrs: attrs(next() % 4),
                            },
                        );
                    }
                    2 => text.push_str("more text than the runs cover"),
                    3 => runs.push(Run {
                        len: 1 + (next() % 9) as usize,
                        attrs: attrs(next() % 4),
                    }),
                    4 => {
                        // A run that ends inside a character.
                        text.insert(0, 'é');
                        runs.insert(
                            0,
                            Run {
                                len: 1,
                                attrs: attrs(1),
                            },
                        );
                        runs.insert(
                            1,
                            Run {
                                len: 1,
                                attrs: attrs(2),
                            },
                        );
                    }
                    _ => runs.push(Run {
                        len: usize::MAX,
                        attrs: attrs(0),
                    }),
                }
            }
            let expected = the_long_way(&text, &runs);
            if needs_rebuild(&text, &runs) {
                rebuilt += 1;
            } else {
                kept += 1;
            }
            let mut block = Block::new(BlockKind::Paragraph);
            block.text = text.clone();
            block.runs = runs.clone();
            assert!(block.repair().is_none());
            assert_eq!(
                (block.text.clone(), block.runs.clone()),
                expected,
                "round {round}: {text:?} with {runs:?}"
            );
            assert!(block.check().is_ok(), "round {round}: {:?}", block.check());
        }
        // Both ways were taken often.
        assert!(
            kept > 2000 && rebuilt > 1000,
            "{kept} kept, {rebuilt} rebuilt"
        );
        // What is wrong is seen, each kind of it.
        let plain = |len| Run {
            len,
            attrs: InlineAttrs::default(),
        };
        assert!(!needs_rebuild("", &[]));
        assert!(!needs_rebuild("aé", &[plain(1), plain(0), plain(2)]));
        assert!(needs_rebuild("aé", &[plain(2), plain(1)]), "inside é");
        assert!(needs_rebuild("abc", &[]), "no runs");
        assert!(needs_rebuild("abc", &[plain(2)]), "too short");
        assert!(needs_rebuild("abc", &[plain(4)]), "too long");
        assert!(needs_rebuild("abc", &[plain(usize::MAX), plain(4)]));
        assert!(needs_rebuild("a\rb", &[plain(3)]), "a carriage return");
        assert!(needs_rebuild("a\u{7}b", &[plain(3)]), "a control character");
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

    fn an_id() -> ImageId {
        ImageId::parse("3f2a9c1b5d6e7f80a1b2c3d4e5f60718").expect("an id")
    }

    /// 1600 × 1067 at 144 to the inch: a photograph, 800 points wide.
    fn wide() -> Image {
        Image::new(an_id(), (1600, 1067), 144)
    }

    /// 300 × 200 at 72: narrower than any column.
    fn small() -> Image {
        Image::new(an_id(), (300, 200), 72)
    }

    /// 1170 × 2532 at 72: a phone screenshot, far taller than wide.
    fn phone() -> Image {
        Image::new(an_id(), (1170, 2532), 72)
    }

    #[test]
    fn an_image_id_is_32_hex_digits_and_nothing_else() {
        let text = "3f2a9c1b5d6e7f80a1b2c3d4e5f60718";
        let id = ImageId::parse(text).expect("32 hex digits");
        assert_eq!(id.to_string(), text);
        assert_eq!(
            id.as_bytes(),
            &[
                0x3f, 0x2a, 0x9c, 0x1b, 0x5d, 0x6e, 0x7f, 0x80, 0xa1, 0xb2, 0xc3, 0xd4, 0xe5, 0xf6,
                0x07, 0x18
            ]
        );
        assert_eq!(ImageId::from_bytes(*id.as_bytes()), id);
        // Either case is read, lower case is written.
        let upper = ImageId::parse("3F2A9C1B5D6E7F80A1B2C3D4E5F60718").expect("upper case");
        assert_eq!(upper, id);
        assert_eq!(upper.to_string(), text);
        assert_eq!(
            ImageId::parse("3f2A9c1B5d6E7f80a1B2c3D4e5F60718"),
            Some(id),
            "mixed case"
        );
        // Every byte value comes back as it went.
        let mut bytes = [0u8; 16];
        for first in 0..=255u8 {
            for (i, byte) in bytes.iter_mut().enumerate() {
                *byte = first.wrapping_add((i as u8).wrapping_mul(37));
            }
            let id = ImageId::from_bytes(bytes);
            let written = id.to_string();
            assert_eq!(written.len(), 32);
            assert!(
                written
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                "{written}"
            );
            assert_eq!(ImageId::parse(&written), Some(id));
        }
        assert_eq!(
            ImageId::from_bytes([0; 16]).to_string(),
            "00000000000000000000000000000000"
        );
        // Nothing else is an id.
        for not_an_id in [
            "",
            "3f2a9c1b5d6e7f80a1b2c3d4e5f6071",        // 31 digits
            "3f2a9c1b5d6e7f80a1b2c3d4e5f607180",      // 33
            "3f2a9c1b5d6e7f80a1b2c3d4e5f6071g",       // not hex
            " 3f2a9c1b5d6e7f80a1b2c3d4e5f6071",       // a space for a digit
            "3f2a9c1b5d6e7f80a1b2c3d4e5f60718 ",      // a space after 32 digits
            "0x3f2a9c1b5d6e7f80a1b2c3d4e5f607",       // a prefix
            "+f2a9c1b5d6e7f80a1b2c3d4e5f60718",       // a sign, which a number parser takes
            "3f2a9c1b-5d6e-7f80-a1b2-c3d4e5f6",       // 32 bytes of a UUID
            "3f2a9c1b5d6e7f80a1b2c3d4e5f6071\u{663}", // 32 characters, the last an Arabic digit
            "\u{ff13}f2a9c1b5d6e7f80a1b2c3d4e5f6071", // a full-width digit: 32 bytes are not at issue
            "éf2a9c1b5d6e7f80a1b2c3d4e5f6071",        // 32 bytes, 31 characters
        ] {
            assert_eq!(ImageId::parse(not_an_id), None, "{not_an_id:?}");
        }
    }

    #[test]
    fn an_image_width_is_a_share_of_the_column_within_its_ends() {
        assert_eq!(ImageWidth::MIN.thousandths(), 50);
        assert_eq!(ImageWidth::FULL.thousandths(), 1000);
        assert_eq!(ImageWidth::new(500).thousandths(), 500);
        assert_eq!(ImageWidth::new(0), ImageWidth::MIN);
        assert_eq!(ImageWidth::new(49), ImageWidth::MIN);
        assert_eq!(ImageWidth::new(50), ImageWidth::MIN);
        assert_eq!(ImageWidth::new(1000), ImageWidth::FULL);
        assert_eq!(ImageWidth::new(1001), ImageWidth::FULL);
        assert_eq!(ImageWidth::new(u16::MAX), ImageWidth::FULL);
        assert!(ImageWidth::new(333) < ImageWidth::new(334));

        assert_eq!(ImageWidth::from_fraction(0.5).thousandths(), 500);
        assert_eq!(ImageWidth::from_fraction(0.6204).thousandths(), 620);
        assert_eq!(ImageWidth::from_fraction(0.6206).thousandths(), 621);
        assert_eq!(ImageWidth::from_fraction(1.0), ImageWidth::FULL);
        assert_eq!(ImageWidth::from_fraction(3.0), ImageWidth::FULL);
        assert_eq!(ImageWidth::from_fraction(1.0e30), ImageWidth::FULL);
        assert_eq!(ImageWidth::from_fraction(f32::INFINITY), ImageWidth::FULL);
        assert_eq!(ImageWidth::from_fraction(0.0), ImageWidth::MIN);
        assert_eq!(ImageWidth::from_fraction(0.01), ImageWidth::MIN);
        assert_eq!(ImageWidth::from_fraction(-2.0), ImageWidth::MIN);
        assert_eq!(
            ImageWidth::from_fraction(f32::NEG_INFINITY),
            ImageWidth::MIN
        );
        assert_eq!(ImageWidth::from_fraction(f32::NAN), ImageWidth::FULL);

        assert_eq!(ImageWidth::from_percent(62).thousandths(), 620);
        assert_eq!(ImageWidth::from_percent(100), ImageWidth::FULL);
        assert_eq!(ImageWidth::from_percent(250), ImageWidth::FULL);
        assert_eq!(ImageWidth::from_percent(u16::MAX), ImageWidth::FULL);
        assert_eq!(ImageWidth::from_percent(5), ImageWidth::MIN);
        assert_eq!(ImageWidth::from_percent(0), ImageWidth::MIN);

        assert_eq!(ImageWidth::new(500).fraction(), 0.5);
        assert_eq!(ImageWidth::FULL.fraction(), 1.0);
        assert_eq!(ImageWidth::MIN.fraction(), 0.05);
        assert_eq!(ImageWidth::new(620).percent(), 62);
        assert_eq!(ImageWidth::new(624).percent(), 62);
        assert_eq!(ImageWidth::new(625).percent(), 63);
        assert_eq!(ImageWidth::new(333).percent(), 33);
        assert_eq!(ImageWidth::new(667).percent(), 67);
        assert_eq!(ImageWidth::FULL.percent(), 100);
        assert_eq!(ImageWidth::MIN.percent(), 5);
        // A percentage shown is the percentage taken.
        for percent in 5..=100 {
            assert_eq!(ImageWidth::from_percent(percent).percent(), percent);
        }
        // A share and its thousandths say the same.
        for thousandths in 50..=1000 {
            let width = ImageWidth::new(thousandths);
            assert_eq!(ImageWidth::from_fraction(width.fraction()), width);
        }
    }

    #[test]
    fn the_natural_width_is_the_share_of_the_column_a_length_takes() {
        assert_eq!(ImageWidth::natural(350.0, 700.0).thousandths(), 500);
        assert_eq!(ImageWidth::natural(300.0, 451.276).thousandths(), 665);
        assert_eq!(ImageWidth::natural(233.0, 700.0).thousandths(), 333);
        // As wide as the column or wider: the whole column.
        assert_eq!(ImageWidth::natural(700.0, 700.0), ImageWidth::FULL);
        assert_eq!(ImageWidth::natural(800.0, 451.276), ImageWidth::FULL);
        // A speck is still to be seen.
        assert_eq!(ImageWidth::natural(1.0, 700.0), ImageWidth::MIN);
        assert_eq!(ImageWidth::natural(0.0, 700.0), ImageWidth::MIN);
        assert_eq!(ImageWidth::natural(-5.0, 700.0), ImageWidth::MIN);
        // A column without a width has no share to give.
        assert_eq!(ImageWidth::natural(300.0, 0.0), ImageWidth::FULL);
        assert_eq!(ImageWidth::natural(300.0, -1.0), ImageWidth::FULL);
        assert_eq!(ImageWidth::natural(300.0, f32::NAN), ImageWidth::FULL);
        assert_eq!(ImageWidth::natural(300.0, f32::INFINITY), ImageWidth::FULL);
        assert_eq!(ImageWidth::natural(f32::NAN, 700.0), ImageWidth::FULL);
    }

    #[test]
    fn larger_and_smaller_step_through_the_steps() {
        let stepped = |from: u16, steps: i32| ImageWidth::new(from).stepped(steps).thousandths();
        // The steps are in order and within the ends, the last is the whole column.
        assert!(ImageWidth::STEPS.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(ImageWidth::STEPS[0] >= ImageWidth::MIN.thousandths());
        assert_eq!(ImageWidth::STEPS[12], ImageWidth::FULL.thousandths());
        // From a step: its neighbours.
        assert_eq!(stepped(500, 1), 600);
        assert_eq!(stepped(500, -1), 400);
        assert_eq!(stepped(333, 1), 400);
        assert_eq!(stepped(333, -1), 250);
        assert_eq!(stepped(500, 2), 667);
        assert_eq!(stepped(500, -3), 250);
        assert_eq!(stepped(500, 0), 500);
        // Up through all of them and down again.
        let mut width = ImageWidth::new(ImageWidth::STEPS[0]);
        for &step in &ImageWidth::STEPS[1..] {
            width = width.stepped(1);
            assert_eq!(width.thousandths(), step);
        }
        for &step in ImageWidth::STEPS[..12].iter().rev() {
            width = width.stepped(-1);
            assert_eq!(width.thousandths(), step);
        }
        // From between two steps the first step lands on the neighbour.
        assert_eq!(stepped(550, 1), 600);
        assert_eq!(stepped(550, -1), 500);
        assert_eq!(stepped(665, 1), 667);
        assert_eq!(stepped(665, -1), 600);
        assert_eq!(stepped(713, 1), 750);
        assert_eq!(stepped(713, -1), 667);
        assert_eq!(stepped(550, 2), 667);
        assert_eq!(stepped(550, -2), 400);
        assert_eq!(stepped(999, 1), 1000);
        assert_eq!(stepped(101, -1), 100);
        // It stays at the ends.
        assert_eq!(stepped(1000, 1), 1000);
        assert_eq!(stepped(1000, 5), 1000);
        assert_eq!(stepped(900, 5), 1000);
        assert_eq!(stepped(100, -1), 100);
        assert_eq!(stepped(150, -5), 100);
        assert_eq!(stepped(500, i32::MAX), 1000);
        assert_eq!(stepped(500, i32::MIN), 100);
        // Narrower than the first step: larger goes to it, smaller changes nothing.
        assert_eq!(stepped(50, 1), 100);
        assert_eq!(stepped(75, 1), 100);
        assert_eq!(stepped(50, -1), 50);
        assert_eq!(stepped(75, -1), 75);
        assert_eq!(stepped(75, -4), 75);
    }

    #[test]
    fn a_new_image_is_as_wide_as_the_column_and_within_range() {
        let image = wide();
        assert_eq!(image.id, an_id());
        assert_eq!(image.pixels, (1600, 1067));
        assert_eq!(image.density, 144);
        assert_eq!(image.width, ImageWidth::FULL);
        assert_eq!(image.description, "");
        assert_eq!(image.natural_points(), (800.0, 533.5));
        assert_eq!(image.aspect(), (1067.0_f64 / 1600.0) as f32);
        assert_eq!(small().natural_points(), (300.0, 200.0));
        assert_eq!(small().aspect(), (200.0_f64 / 300.0) as f32);
        assert_eq!(
            Image::new(an_id(), (600, 300), 300).natural_points(),
            (144.0, 72.0)
        );
        // What a file cannot mean is put right.
        assert_eq!(Image::new(an_id(), (0, 0), 72).pixels, (1, 1));
        assert_eq!(Image::new(an_id(), (0, 40), 72).pixels, (1, 40));
        assert_eq!(Image::new(an_id(), (40, 0), 72).pixels, (40, 1));
        assert_eq!(Image::new(an_id(), (10, 10), 0).density, Image::MIN_DENSITY);
        assert_eq!(Image::new(an_id(), (10, 10), 35).density, 36);
        assert_eq!(Image::new(an_id(), (10, 10), 36).density, 36);
        assert_eq!(Image::new(an_id(), (10, 10), 1200).density, 1200);
        assert_eq!(Image::new(an_id(), (10, 10), u16::MAX).density, 1200);
        assert_eq!(Image::DEFAULT_DENSITY, 72);
        // The largest pixel size there is still has a size and a shape.
        let huge = Image::new(an_id(), (u32::MAX, 1), 72);
        assert!(huge.natural_points().0.is_finite());
        assert!(huge.aspect() > 0.0);
        let tall = Image::new(an_id(), (1, u32::MAX), 72);
        assert!(tall.aspect().is_finite());
    }

    #[test]
    fn the_widest_an_image_is_drawn_fits_on_one_page() {
        use crate::settings::{Margins, Orientation, PaperSize};
        // A4 with an inch on every side: a text area of 451.276 × 697.89 points.
        let page = PageSetup::default();
        assert_eq!(page.paper, PaperSize::A4);
        assert_eq!(page.margins, Margins::uniform(72.0));
        let (w, h) = page.content_size();
        assert!((w - 451.276).abs() < 0.001 && (h - 697.89).abs() < 0.001);
        let height_at = |image: &Image, width: ImageWidth, page: &PageSetup| {
            width.fraction() * page.content_size().0 * image.aspect()
        };

        // A photograph: the whole column is 301 points high.
        assert_eq!(wide().max_width(&page), ImageWidth::FULL);
        // It is 800 points wide by itself, more than the column.
        assert_eq!(wide().original_width(&page), ImageWidth::FULL);

        // A small picture may be as wide as the column, and has its own
        // size at 300 of 451.276 points.
        assert_eq!(small().max_width(&page), ImageWidth::FULL);
        assert_eq!(small().original_width(&page).thousandths(), 665);

        // A phone screenshot as wide as the column would be 977 points
        // high. The widest it goes is what makes it 697 points high (the
        // text area in whole points), rounded down: 713 thousandths.
        assert_eq!(phone().max_width(&page).thousandths(), 713);
        let widest = phone().max_width(&page);
        assert!(height_at(&phone(), widest, &page) <= 697.0);
        assert!(height_at(&phone(), ImageWidth::new(714), &page) > 697.0);
        // By itself it is 1170 points wide: original size is the widest it goes.
        assert_eq!(phone().original_width(&page).thousandths(), 713);
        // At a density that makes it narrower than that, its own size.
        let dense = Image::new(an_id(), (1170, 2532), 460);
        assert_eq!(dense.natural_points().0, (1170.0_f64 * 72.0 / 460.0) as f32);
        assert_eq!(dense.max_width(&page).thousandths(), 713);
        assert_eq!(dense.original_width(&page).thousandths(), 406);

        // After a change of margins: half an inch, a text area of
        // 523.276 × 769.89 points.
        let narrow_margins = PageSetup {
            margins: Margins::uniform(36.0),
            ..page
        };
        assert_eq!(wide().max_width(&narrow_margins), ImageWidth::FULL);
        assert_eq!(wide().original_width(&narrow_margins), ImageWidth::FULL);
        assert_eq!(small().original_width(&narrow_margins).thousandths(), 573);
        assert_eq!(phone().max_width(&narrow_margins).thousandths(), 679);
        assert_eq!(phone().original_width(&narrow_margins).thousandths(), 679);
        assert!(height_at(&phone(), ImageWidth::new(679), &narrow_margins) <= 769.0);
        assert!(height_at(&phone(), ImageWidth::new(680), &narrow_margins) > 769.0);
        // Wide margins at the sides only: the column is narrow and the
        // screenshot fits whole.
        let narrow_column = PageSetup {
            margins: Margins {
                left: 150.0,
                right: 150.0,
                ..Margins::uniform(72.0)
            },
            ..page
        };
        assert_eq!(phone().max_width(&narrow_column), ImageWidth::FULL);

        // The sheet on its side: the photograph no longer fits at the whole
        // column (697.89 points wide would be 465 high, the text area is 451).
        let landscape = PageSetup {
            orientation: Orientation::Landscape,
            ..page
        };
        assert_eq!(wide().max_width(&landscape).thousandths(), 969);
        assert_eq!(wide().original_width(&landscape).thousandths(), 969);
        assert_eq!(small().original_width(&landscape).thousandths(), 430);

        // Never narrower than the narrowest, however tall the picture.
        let strip = Image::new(an_id(), (10, 100_000), 72);
        assert_eq!(strip.max_width(&page), ImageWidth::MIN);
        assert_eq!(strip.original_width(&page), ImageWidth::MIN);
        // A width on every paper the settings can hold, for every shape.
        for paper in [
            PaperSize::A4,
            PaperSize::UsLetter,
            PaperSize::Custom {
                width: 72.0,
                height: 72.0,
            },
            PaperSize::Custom {
                width: 5000.0,
                height: 100.0,
            },
        ] {
            for margins in [0.0, 36.0, 72.0, 400.0] {
                let page = PageSetup {
                    paper,
                    margins: Margins::uniform(margins),
                    ..page
                }
                .sanitized();
                for image in [wide(), small(), phone(), strip.clone()] {
                    let widest = image.max_width(&page);
                    let original = image.original_width(&page);
                    assert!(widest >= ImageWidth::MIN && widest <= ImageWidth::FULL);
                    assert!(original >= ImageWidth::MIN && original <= widest);
                    // (A hundredth of a point for the arithmetic of the
                    // check itself, where the text area is whole points.)
                    assert!(
                        widest == ImageWidth::MIN
                            || height_at(&image, widest, &page) <= page.content_size().1 + 0.01,
                        "{image:?} on {page:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_description_is_one_clean_line() {
        let described = |text: &str| {
            let mut image = small();
            image.description = text.to_string();
            match Block::image(image).kind() {
                BlockKind::Image(image) => image.description.clone(),
                other => panic!("{other:?}"),
            }
        };
        assert_eq!(described("The harbour at dusk"), "The harbour at dusk");
        assert_eq!(described("  The harbour at dusk \n"), "The harbour at dusk");
        assert_eq!(described("The harbour\nat dusk"), "The harbour at dusk");
        assert_eq!(described("The harbour\r\nat dusk"), "The harbour at dusk");
        assert_eq!(described("The\tharbour"), "The harbour");
        assert_eq!(
            described("a\u{2028}b\u{2029}c\u{85}d\u{0}e\u{7}f"),
            "a b c d e f"
        );
        assert_eq!(described("\n\n"), "");
        assert_eq!(described(""), "");
        // What is no control character stays: two spaces, an emoji, an accent.
        assert_eq!(described("a  b"), "a  b");
        assert_eq!(
            described("Zoë \u{1F469}\u{200D}\u{1F467}"),
            "Zoë \u{1F469}\u{200D}\u{1F467}"
        );
        // Cut to a thousand characters, counted as characters.
        let long = "é".repeat(1500);
        let cut = described(&long);
        assert_eq!(cut.chars().count(), 1000);
        assert_eq!(cut.len(), 2000);
        assert_eq!(described(&"x".repeat(1000)).len(), 1000);
        // A cut that ends on a space leaves no space at the end.
        let spaced = format!("{} tail", "y".repeat(999));
        assert_eq!(described(&spaced), "y".repeat(999));
        // Cleaning twice changes nothing more.
        for text in [
            "  a\r\n\r\nb  ",
            "\u{7}",
            &long,
            &spaced,
            " \u{2028} x \t ",
            "plain",
        ] {
            let once = clean_description(text);
            assert!(is_clean_description(&once), "{once:?}");
            assert_eq!(clean_description(&once), once);
        }
    }

    #[test]
    fn an_image_block_holds_no_text_indent_text_size_or_justify() {
        let bold = InlineAttrs {
            bold: true,
            ..Default::default()
        };
        let as_built = |block: &Block| {
            assert!(block.check().is_ok(), "{:?}", block.check());
            assert!(block.is_void());
            assert!(block.is_empty());
            assert!(block.runs().is_empty());
            assert_eq!(block.indent(), 0);
            assert_eq!(block.text_size(), None);
            assert_ne!(block.alignment(), Alignment::Justify);
        };

        let block = Block::image(wide());
        assert_eq!(block.kind(), &BlockKind::Image(wide()));
        assert_eq!(block.as_image(), Some(&wide()));
        assert_eq!(block.kind().image(), Some(&wide()));
        assert!(block.kind().is_image());
        assert!(block.kind().is_void());
        assert!(!block.kind().is_list_item() && !block.kind().is_code());
        assert_eq!(block.kind().name(), "image");
        assert_eq!(block.alignment(), Alignment::Left);
        as_built(&block);
        // No other kind is an image.
        for kind in [
            BlockKind::Paragraph,
            BlockKind::heading(2),
            BlockKind::BulletItem,
            BlockKind::NumberedItem,
            BlockKind::todo(),
            BlockKind::Quote,
            BlockKind::code(),
            BlockKind::Divider,
            BlockKind::PageBreak,
        ] {
            assert!(!kind.is_image(), "{kind:?}");
            assert_eq!(kind.image(), None);
            assert_eq!(Block::new(kind.clone()).as_image(), None);
            assert_ne!(kind.name(), "image");
            assert!(!kind.same_type(&BlockKind::Image(wide())));
        }
        // Two images are the same kind of block, whatever they show.
        assert!(BlockKind::Image(wide()).same_type(&BlockKind::Image(phone())));

        // The builders.
        let block = Block::with_text(BlockKind::Image(wide()), "a caption")
            .with_span("more", bold.clone())
            .with_indent(3)
            .with_text_size(Some(24.0))
            .with_alignment(Alignment::Justify);
        as_built(&block);
        assert_eq!(block.alignment(), Alignment::Left);
        for alignment in [Alignment::Left, Alignment::Center, Alignment::Right] {
            let block = Block::image(wide()).with_alignment(alignment);
            assert_eq!(block.alignment(), alignment);
            as_built(&block);
        }

        // The setters the editing engine uses.
        let mut block = Block::image(wide()).with_alignment(Alignment::Center);
        block.insert(0, "text", &bold);
        block.insert_content(0, &Block::paragraph("content"));
        block.append(&Block::paragraph("more"));
        block.replace_text(0..0, "typed", &bold);
        block.set_indent(4);
        block.set_text_size(Some(30.0));
        as_built(&block);
        assert_eq!(block.alignment(), Alignment::Center);
        block.set_alignment(Alignment::Right);
        assert_eq!(block.alignment(), Alignment::Right);
        block.set_alignment(Alignment::Justify);
        assert_eq!(block.alignment(), Alignment::Left);
        as_built(&block);
        // Its halves and its copies are images too, with nothing more.
        as_built(&block.slice(0..0));
        as_built(&block.split_off(0));
        as_built(&block);

        // A block of text that becomes an image loses all four.
        let mut block = Block::paragraph("A paragraph ")
            .with_span("with bold", bold.clone())
            .with_indent(2)
            .with_text_size(Some(20.0))
            .with_alignment(Alignment::Justify);
        block.set_kind(BlockKind::Image(wide()));
        as_built(&block);
        assert_eq!(block.alignment(), Alignment::Left);
        assert_eq!(block.as_image(), Some(&wide()));
        // Centred or flush right, it stays so.
        let mut block = Block::paragraph("x").with_alignment(Alignment::Right);
        block.set_kind(BlockKind::Image(wide()));
        assert_eq!(block.alignment(), Alignment::Right);
        // And an image that becomes a paragraph is an empty one that can
        // have all of them again.
        block.set_kind(BlockKind::Paragraph);
        block.set_indent(2);
        block.set_text_size(Some(20.0));
        block.set_alignment(Alignment::Justify);
        block.insert(0, "text", &bold);
        assert_eq!(
            (
                block.indent(),
                block.text_size(),
                block.alignment(),
                block.text()
            ),
            (2, Some(20.0), Alignment::Justify, "text")
        );
        assert!(block.check().is_ok());

        // What an image holds is put right on the way into a block: by
        // the constructor and by a change of kind.
        let broken = Image {
            id: an_id(),
            pixels: (0, 0),
            density: 5,
            width: ImageWidth(0),
            description: " two\nlines ".to_string(),
        };
        let mended = Image {
            id: an_id(),
            pixels: (1, 1),
            density: 36,
            width: ImageWidth::MIN,
            description: "two lines".to_string(),
        };
        assert_eq!(Block::image(broken.clone()).as_image(), Some(&mended));
        let mut block = Block::paragraph("x");
        block.set_kind(BlockKind::Image(broken));
        assert_eq!(block.as_image(), Some(&mended));
        as_built(&block);

        // The other void kinds keep an indent as they did.
        let divider = Block::new(BlockKind::Divider)
            .with_indent(2)
            .with_alignment(Alignment::Justify);
        assert_eq!(
            (divider.indent(), divider.alignment()),
            (2, Alignment::Justify)
        );
    }

    #[test]
    fn repair_mends_every_broken_invariant_of_an_image_and_check_names_it() {
        let good = || Block::image(wide()).with_alignment(Alignment::Center);
        assert!(good().check().is_ok());
        let named = |block: &Block, words: &str| {
            let said = block.check().expect_err(words);
            assert!(said.contains(words), "{said:?} does not say {words:?}");
        };
        let mended = |mut block: Block| {
            assert!(block.repair().is_none());
            assert!(block.check().is_ok(), "{:?}", block.check());
            block
        };

        // An indent.
        let mut block = good();
        block.indent = 3;
        named(&block, "image block has an indent");
        let block = mended(block);
        assert_eq!(block.indent(), 0);
        assert_eq!(block, good());

        // A text size.
        let mut block = good();
        block.text_size = Some(24.0);
        named(&block, "image block has a text size");
        assert_eq!(mended(block), good());

        // Justified: read as left.
        let mut block = good();
        block.alignment = Alignment::Justify;
        named(&block, "image block is justified");
        let block = mended(block);
        assert_eq!(block.alignment(), Alignment::Left);
        assert_eq!(block.as_image(), Some(&wide()));

        // No pixels.
        for pixels in [(0, 0), (0, 5), (5, 0)] {
            let mut block = good();
            block.kind = BlockKind::Image(Image { pixels, ..wide() });
            named(&block, "image has no pixels");
            let block = mended(block);
            let now = block.as_image().expect("an image").pixels;
            assert_eq!(now, (pixels.0.max(1), pixels.1.max(1)));
        }

        // A density no file has.
        for (density, repaired) in [(0, 36), (35, 36), (1201, 1200), (u16::MAX, 1200)] {
            let mut block = good();
            block.kind = BlockKind::Image(Image { density, ..wide() });
            named(&block, "image density out of range");
            assert_eq!(
                mended(block).as_image().expect("an image").density,
                repaired
            );
        }

        // A width beyond the ends.
        for (width, repaired) in [(0, 50), (49, 50), (1001, 1000), (u16::MAX, 1000)] {
            let mut block = good();
            block.kind = BlockKind::Image(Image {
                width: ImageWidth(width),
                ..wide()
            });
            named(&block, "image width out of range");
            let block = mended(block);
            assert_eq!(
                block.as_image().expect("an image").width.thousandths(),
                repaired
            );
        }

        // A description that is not one clean line.
        for (description, repaired) in [
            ("two\nlines", "two lines".to_string()),
            (" spaces around ", "spaces around".to_string()),
            ("a bell\u{7}", "a bell".to_string()),
            (&"z".repeat(1001) as &str, "z".repeat(1000)),
        ] {
            let mut block = good();
            block.kind = BlockKind::Image(Image {
                description: description.to_string(),
                ..wide()
            });
            named(&block, "image description is not one clean line");
            let block = mended(block);
            assert_eq!(block.as_image().expect("an image").description, repaired);
        }

        // Text: the image stays, the text goes into a paragraph after it.
        let mut block = good();
        block.text = "a caption".to_string();
        block.runs = vec![Run {
            len: 9,
            attrs: InlineAttrs::default(),
        }];
        named(&block, "void block has text");
        let after = block.repair().expect("the text moved out");
        assert!(block.check().is_ok(), "{:?}", block.check());
        assert_eq!(block, good());
        assert_eq!(after.kind(), &BlockKind::Paragraph);
        assert_eq!(after.text(), "a caption");
        assert!(after.check().is_ok());

        // Everything at once, as a file written by something else could
        // have it. The paragraph takes the alignment and the indent the
        // block came with, as it does after a divider.
        let mut block = Block::image(wide());
        block.kind = BlockKind::Image(Image {
            id: an_id(),
            pixels: (0, 7),
            density: 9000,
            width: ImageWidth(7),
            description: "\tA\r\nB ".to_string(),
        });
        block.text = "text".to_string();
        block.runs = vec![Run {
            len: 4,
            attrs: InlineAttrs::default(),
        }];
        block.indent = 200;
        block.text_size = Some(f32::NAN);
        block.alignment = Alignment::Justify;
        assert!(block.check().is_err());
        let after = block.repair().expect("the text moved out");
        assert!(block.check().is_ok(), "{:?}", block.check());
        assert_eq!(
            block.as_image(),
            Some(&Image {
                id: an_id(),
                pixels: (1, 7),
                density: 1200,
                width: ImageWidth::MIN,
                description: "A B".to_string(),
            })
        );
        assert_eq!(
            (
                block.indent(),
                block.text_size(),
                block.alignment(),
                block.text()
            ),
            (0, None, Alignment::Left, "")
        );
        assert_eq!(
            (after.text(), after.indent(), after.alignment()),
            ("text", MAX_INDENT, Alignment::Justify)
        );
        assert!(after.check().is_ok());

        // An image that is as it should be is left as it is.
        let mut block = good();
        assert!(block.repair().is_none());
        assert_eq!(block, good());
    }
}
