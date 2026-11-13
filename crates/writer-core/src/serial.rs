//! JSON serialisation of documents and fragments.
//!
//! # Format (versions 1 and 2)
//!
//! ```json
//! {
//!   "format": "the-writer-document",
//!   "version": 1,
//!   "settings": { "page_view": true },
//!   "blocks": [
//!     { "id": 1, "type": "heading", "level": 1, "text": "Title" },
//!     { "id": 2, "text": "Hello world", "runs": [{ "len": 6 }, { "len": 5, "bold": true }] },
//!     { "id": 3, "type": "divider" }
//!   ],
//!   "comments": [
//!     { "id": 1, "body": "Check this", "author": "Ann", "created": 1700000000000 }
//!   ]
//! }
//! ```
//!
//! Everything that has its default value is omitted: a paragraph has no
//! `"type"`, plain text has no `"runs"`, default settings have no
//! `"settings"`. Run lengths are UTF-8 byte lengths. Block keys: `id`,
//! `type` (`paragraph`, `heading`, `bullet`, `numbered`, `todo`, `quote`,
//! `code`, `divider`, `page_break`, `image`), `level`, `checked`, `lang`,
//! `image`, `px`, `ppi`, `width`, `alt`, `align`, `indent`, `size`, `text`,
//! `runs`. Run keys: `len`, `bold`, `italic`,
//! `underline`, `strike`, `code`, `highlight`, `color` (`#rrggbb`), `font`,
//! `link`, `comments` (thread ids). Settings keys: `page` (`paper`, `width`,
//! `height`, `orientation`, `margins`), `typography` (`font`, `size`,
//! `line_height`, `paragraph_spacing`), `page_view`, and `spelling_language`
//! (`"auto"` or a language identifier such as `"en_GB"`; absent when the
//! document follows the app's setting, which is also how a document written
//! before the key existed reads).
//!
//! # Images (version 2)
//!
//! ```json
//! { "id": 7, "type": "image", "image": "3f2a9c1b5d6e7f80a1b2c3d4e5f60718",
//!   "px": [1600, 1067], "ppi": 144, "width": 500, "alt": "The harbour at dusk", "align": "center" }
//! ```
//!
//! `image` is the [`ImageId`] of the stored bytes (32 lowercase hex
//! digits), `px` their pixel size, `ppi` their density (omitted when 72),
//! `width` the share of the column in thousandths (omitted when 1000) and
//! `alt` the description (omitted when empty). An image block has no
//! `indent`, `size`, `text` or `runs`, and its `align` is never
//! `"justify"`.
//!
//! The compact form holds the token `"image":"<32 hex digits>"` exactly so,
//! without a space: whoever keeps the bytes finds the images a stored
//! document names by looking for that token, without reading the document.
//! The key must keep its name.
//!
//! A document or a fragment is written with `"version": 2` only when it
//! holds an image, and with `"version": 1` otherwise, exactly as before
//! images existed. A build from before them refuses a version 2 document
//! as "newer than this app supports", which is the point: it would read an
//! image as an empty paragraph and write that back. A document without an
//! image is still read by every build.
//!
//! Reading is lenient: unknown keys are ignored, values of the wrong type
//! fall back to defaults, and every model invariant is repaired (see
//! [`Document::from_blocks`]). An image block whose `image` is not an id
//! becomes a paragraph holding its `alt`; a missing or unreadable `px`
//! reads as `[1, 1]`. Reading fails only when the input is not
//! JSON, is not an object with a `blocks` array, names another format, or
//! has a newer version than this build understands.

use std::fmt;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

use crate::attrs::{CommentId, Highlight, InlineAttrs, Rgb};
use crate::block::{Alignment, Block, BlockId, BlockKind, Image, ImageId, ImageWidth};
use crate::comments::{CommentThread, Reply, ReplyId};
use crate::document::Document;
use crate::fragment::Fragment;
use crate::runs::Run;
use crate::settings::{
    DocumentSettings, Margins, Orientation, PageSetup, PaperSize, SpellingLanguage, Typography,
};

/// The `"format"` tag of serialised documents.
pub const DOCUMENT_FORMAT: &str = "the-writer-document";
/// The `"format"` tag of serialised fragments.
pub const FRAGMENT_FORMAT: &str = "the-writer-fragment";
/// The newest format version this build reads, and the one it writes for a
/// document or a fragment that holds an image. Without an image it writes
/// [`FORMAT_VERSION_WITHOUT_IMAGES`].
pub const FORMAT_VERSION: u64 = 2;
/// The format version written for a document or a fragment without an
/// image: the format as it was before images, which every build reads.
pub const FORMAT_VERSION_WITHOUT_IMAGES: u64 = 1;

/// The version to write `blocks` with: 2 as soon as one of them is an
/// image, so that a build that does not know images refuses the document
/// instead of reading the image as an empty paragraph.
fn version_for(blocks: &[Block]) -> u64 {
    if blocks.iter().any(|b| b.kind.is_image()) {
        FORMAT_VERSION
    } else {
        FORMAT_VERSION_WITHOUT_IMAGES
    }
}

/// Why a document or fragment could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadError {
    /// The input is not valid JSON.
    Json(String),
    /// The JSON is not a document or fragment of this format.
    NotADocument(String),
    /// The input was written by a newer version of the format.
    UnsupportedVersion(u64),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::Json(e) => write!(f, "invalid JSON: {e}"),
            LoadError::NotADocument(e) => write!(f, "not a document: {e}"),
            LoadError::UnsupportedVersion(v) => {
                write!(
                    f,
                    "document format version {v} is newer than this app supports"
                )
            }
        }
    }
}

impl std::error::Error for LoadError {}

// ----- writing ----------------------------------------------------------------------

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Serialize)]
struct RunOut<'a> {
    len: usize,
    #[serde(skip_serializing_if = "is_false")]
    bold: bool,
    #[serde(skip_serializing_if = "is_false")]
    italic: bool,
    #[serde(skip_serializing_if = "is_false")]
    underline: bool,
    #[serde(skip_serializing_if = "is_false")]
    strike: bool,
    #[serde(skip_serializing_if = "is_false")]
    code: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    highlight: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    font: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    link: Option<&'a str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    comments: Vec<u64>,
}

#[derive(Serialize)]
struct BlockOut<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<u64>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    kind: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    level: Option<u8>,
    #[serde(skip_serializing_if = "is_false")]
    checked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    lang: Option<&'a str>,
    /// The id of an image's stored bytes. Written compact this is the
    /// token `"image":"<32 hex digits>"` the library looks for: the key
    /// keeps its name.
    #[serde(skip_serializing_if = "Option::is_none")]
    image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    px: Option<[u32; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ppi: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    width: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    alt: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    align: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    indent: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    size: Option<f32>,
    #[serde(skip_serializing_if = "str::is_empty")]
    text: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    runs: Option<Vec<RunOut<'a>>>,
}

impl<'a> BlockOut<'a> {
    fn new(b: &'a Block, with_id: bool) -> BlockOut<'a> {
        let runs = if b.runs.is_empty() || (b.runs.len() == 1 && b.runs[0].attrs.is_plain()) {
            None
        } else {
            Some(
                b.runs
                    .iter()
                    .map(|r| RunOut {
                        len: r.len,
                        bold: r.attrs.bold,
                        italic: r.attrs.italic,
                        underline: r.attrs.underline,
                        strike: r.attrs.strikethrough,
                        code: r.attrs.code,
                        highlight: r.attrs.highlight.map(Highlight::name),
                        color: r.attrs.color.map(Rgb::to_hex),
                        font: r.attrs.font.as_deref(),
                        link: r.attrs.link.as_deref(),
                        comments: r.attrs.comments.iter().map(|c| c.0).collect(),
                    })
                    .collect(),
            )
        };
        let image = b.kind.image();
        BlockOut {
            id: with_id.then_some(b.id.0),
            kind: (b.kind != BlockKind::Paragraph).then(|| b.kind.name()),
            level: b.kind.heading_level(),
            checked: matches!(b.kind, BlockKind::TodoItem { checked: true }),
            lang: match &b.kind {
                BlockKind::CodeBlock { language } => language.as_deref(),
                _ => None,
            },
            image: image.map(|i| i.id.to_string()),
            px: image.map(|i| [i.pixels.0, i.pixels.1]),
            ppi: image
                .map(|i| i.density)
                .filter(|d| *d != Image::DEFAULT_DENSITY),
            width: image
                .map(|i| i.width.thousandths())
                .filter(|w| *w != ImageWidth::FULL.thousandths()),
            alt: image
                .map(|i| i.description.as_str())
                .filter(|d| !d.is_empty()),
            align: (b.alignment != Alignment::Left).then(|| b.alignment.name()),
            indent: (b.indent != 0).then_some(b.indent),
            size: b.text_size,
            text: &b.text,
            runs,
        }
    }
}

#[derive(Serialize)]
struct ReplyOut<'a> {
    id: u64,
    body: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    author: &'a str,
    created: i64,
}

#[derive(Serialize)]
struct ThreadOut<'a> {
    id: u64,
    body: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    author: &'a str,
    created: i64,
    #[serde(skip_serializing_if = "is_false")]
    resolved: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    replies: Vec<ReplyOut<'a>>,
}

#[derive(Serialize)]
struct MarginsOut {
    top: f32,
    right: f32,
    bottom: f32,
    left: f32,
}

#[derive(Serialize)]
struct PageOut {
    paper: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    width: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    height: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    orientation: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    margins: Option<MarginsOut>,
}

#[derive(Serialize)]
struct TypographyOut<'a> {
    font: &'a str,
    size: f32,
    line_height: f32,
    paragraph_spacing: f32,
}

#[derive(Serialize)]
struct SettingsOut<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    page: Option<PageOut>,
    #[serde(skip_serializing_if = "Option::is_none")]
    typography: Option<TypographyOut<'a>>,
    #[serde(skip_serializing_if = "is_false")]
    page_view: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    spelling_language: Option<&'a str>,
}

impl<'a> SettingsOut<'a> {
    fn new(s: &'a DocumentSettings) -> Option<SettingsOut<'a>> {
        if *s == DocumentSettings::default() {
            return None;
        }
        let page = (s.page != PageSetup::default()).then(|| {
            let (paper, width, height) = match s.page.paper {
                PaperSize::A4 => ("a4", None, None),
                PaperSize::UsLetter => ("letter", None, None),
                PaperSize::Custom { width, height } => ("custom", Some(width), Some(height)),
            };
            PageOut {
                paper,
                width,
                height,
                orientation: (s.page.orientation == Orientation::Landscape).then_some("landscape"),
                margins: (s.page.margins != Margins::default()).then_some(MarginsOut {
                    top: s.page.margins.top,
                    right: s.page.margins.right,
                    bottom: s.page.margins.bottom,
                    left: s.page.margins.left,
                }),
            }
        });
        let typography = (s.typography != Typography::default()).then(|| TypographyOut {
            font: &s.typography.body_font,
            size: s.typography.base_size,
            line_height: s.typography.line_height,
            paragraph_spacing: s.typography.paragraph_spacing,
        });
        Some(SettingsOut {
            page,
            typography,
            page_view: s.page_view,
            spelling_language: s
                .spelling_language
                .as_ref()
                .map(SpellingLanguage::as_setting_str),
        })
    }
}

#[derive(Serialize)]
struct DocOut<'a> {
    format: &'static str,
    version: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    settings: Option<SettingsOut<'a>>,
    blocks: Vec<BlockOut<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    comments: Vec<ThreadOut<'a>>,
}

impl Serialize for Document {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        DocOut {
            format: DOCUMENT_FORMAT,
            version: version_for(&self.blocks),
            settings: SettingsOut::new(&self.settings),
            blocks: self.blocks.iter().map(|b| BlockOut::new(b, true)).collect(),
            comments: self
                .comments
                .values()
                .map(|t| ThreadOut {
                    id: t.id.0,
                    body: &t.body,
                    author: &t.author,
                    created: t.created_at,
                    resolved: t.resolved,
                    replies: t
                        .replies
                        .iter()
                        .map(|r| ReplyOut {
                            id: r.id.0,
                            body: &r.body,
                            author: &r.author,
                            created: r.created_at,
                        })
                        .collect(),
                })
                .collect(),
        }
        .serialize(serializer)
    }
}

#[derive(Serialize)]
struct FragmentOut<'a> {
    format: &'static str,
    version: u64,
    blocks: Vec<BlockOut<'a>>,
}

impl Serialize for Fragment {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        FragmentOut {
            format: FRAGMENT_FORMAT,
            version: version_for(&self.blocks),
            blocks: self
                .blocks
                .iter()
                .map(|b| BlockOut::new(b, false))
                .collect(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Document {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Document, D::Error> {
        let v = Value::deserialize(deserializer)?;
        parse_document(&v).map_err(D::Error::custom)
    }
}

impl<'de> Deserialize<'de> for Fragment {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Fragment, D::Error> {
        let v = Value::deserialize(deserializer)?;
        parse_fragment(&v).map_err(D::Error::custom)
    }
}

impl Document {
    /// Serialises to compact JSON (see the [`serial`](crate::serial) module
    /// for the format).
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// Serialises to indented JSON.
    pub fn to_json_pretty(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// Reads a document, repairing invariants. Fails only on unrecoverable
    /// input (see [`LoadError`]).
    pub fn from_json(json: &str) -> Result<Document, LoadError> {
        let v: Value = serde_json::from_str(json).map_err(|e| LoadError::Json(e.to_string()))?;
        parse_document(&v)
    }
}

impl Fragment {
    /// Serialises to compact JSON (same block format as documents, no ids).
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// Reads a fragment. Also accepts a whole serialised document (its
    /// blocks become the fragment).
    pub fn from_json(json: &str) -> Result<Fragment, LoadError> {
        let v: Value = serde_json::from_str(json).map_err(|e| LoadError::Json(e.to_string()))?;
        parse_fragment(&v)
    }
}

// ----- reading ----------------------------------------------------------------------

fn get_str<'a>(o: &'a Map<String, Value>, k: &str) -> Option<&'a str> {
    o.get(k).and_then(Value::as_str)
}

fn get_bool(o: &Map<String, Value>, k: &str) -> bool {
    match o.get(k) {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|v| v != 0.0),
        _ => false,
    }
}

fn get_u64(o: &Map<String, Value>, k: &str) -> Option<u64> {
    let v = o.get(k)?;
    v.as_u64().or_else(|| {
        v.as_f64()
            .filter(|f| f.is_finite() && *f >= 0.0)
            .map(|f| f as u64)
    })
}

fn get_i64(o: &Map<String, Value>, k: &str) -> Option<i64> {
    let v = o.get(k)?;
    v.as_i64()
        .or_else(|| v.as_f64().filter(|f| f.is_finite()).map(|f| f as i64))
}

fn get_f32(o: &Map<String, Value>, k: &str) -> Option<f32> {
    o.get(k)
        .and_then(Value::as_f64)
        .map(|f| f as f32)
        .filter(|f| f.is_finite())
}

/// Checks the header and returns the object.
fn header<'a>(v: &'a Value, format: &str) -> Result<&'a Map<String, Value>, LoadError> {
    header_read_by(v, format, FORMAT_VERSION)
}

/// [`header`] as a build reads it whose newest version is `newest`: the
/// whole of what decides whether a build takes a document or refuses it as
/// too new.
fn header_read_by<'a>(
    v: &'a Value,
    format: &str,
    newest: u64,
) -> Result<&'a Map<String, Value>, LoadError> {
    let o = v
        .as_object()
        .ok_or_else(|| LoadError::NotADocument("expected a JSON object".into()))?;
    if let Some(f) = get_str(o, "format")
        && f != format
        && !(format == FRAGMENT_FORMAT && f == DOCUMENT_FORMAT)
    {
        return Err(LoadError::NotADocument(format!("unknown format {f:?}")));
    }
    let version = get_u64(o, "version").unwrap_or(newest).max(1);
    if version > newest {
        return Err(LoadError::UnsupportedVersion(version));
    }
    // Future migrations from older versions go here.
    if !o.get("blocks").is_some_and(Value::is_array) {
        return Err(LoadError::NotADocument("missing \"blocks\" array".into()));
    }
    Ok(o)
}

fn parse_document(v: &Value) -> Result<Document, LoadError> {
    let o = header(v, DOCUMENT_FORMAT)?;
    let blocks = parse_blocks(o);
    let threads = o
        .get("comments")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(parse_thread).collect())
        .unwrap_or_default();
    let settings = o
        .get("settings")
        .and_then(Value::as_object)
        .map(parse_settings)
        .unwrap_or_default();
    Ok(Document::from_parts(blocks, threads, settings))
}

fn parse_fragment(v: &Value) -> Result<Fragment, LoadError> {
    let o = header(v, FRAGMENT_FORMAT)?;
    Ok(Fragment::new(parse_blocks(o)))
}

fn parse_blocks(o: &Map<String, Value>) -> Vec<Block> {
    o.get("blocks")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(parse_block).collect())
        .unwrap_or_default()
}

fn parse_kind(o: &Map<String, Value>) -> BlockKind {
    match get_str(o, "type").unwrap_or("paragraph") {
        "heading" => BlockKind::heading(get_u64(o, "level").unwrap_or(1).min(6) as u8),
        "bullet" => BlockKind::BulletItem,
        "numbered" => BlockKind::NumberedItem,
        "todo" => BlockKind::TodoItem {
            checked: get_bool(o, "checked"),
        },
        "quote" => BlockKind::Quote,
        "code" => BlockKind::CodeBlock {
            language: get_str(o, "lang").map(str::to_string),
        },
        "divider" => BlockKind::Divider,
        "page_break" => BlockKind::PageBreak,
        // Without an id there is no image to show: `parse_block` keeps the
        // description as a paragraph.
        "image" => parse_image(o).map_or(BlockKind::Paragraph, BlockKind::Image),
        _ => BlockKind::Paragraph,
    }
}

/// The image of a block of type `image`; `None` when `image` is not an id.
/// Everything else has a fallback: a missing or unreadable `px` is 1 × 1,
/// `ppi` 72, `width` the whole column, `alt` empty. Out-of-range values
/// are repaired with the block.
fn parse_image(o: &Map<String, Value>) -> Option<Image> {
    let id = get_str(o, "image").and_then(ImageId::parse)?;
    let pixel = |v: &Value| -> Option<u32> {
        let n = v.as_u64().or_else(|| {
            v.as_f64()
                .filter(|f| f.is_finite() && *f >= 0.0)
                .map(|f| f as u64)
        })?;
        Some(n.min(u64::from(u32::MAX)) as u32)
    };
    let pixels = match o.get("px").and_then(Value::as_array).map(Vec::as_slice) {
        Some([w, h]) => pixel(w).zip(pixel(h)),
        _ => None,
    }
    .unwrap_or((1, 1));
    let small = |k: &str| get_u64(o, k).map(|n| n.min(u64::from(u16::MAX)) as u16);
    Some(Image {
        id,
        pixels,
        density: small("ppi").unwrap_or(Image::DEFAULT_DENSITY),
        width: small("width").map_or(ImageWidth::FULL, ImageWidth::new),
        description: get_str(o, "alt").unwrap_or("").to_string(),
    })
}

fn parse_block(v: &Value) -> Option<Block> {
    let o = v.as_object()?;
    let kind = parse_kind(o);
    let mut text = get_str(o, "text").unwrap_or("").to_string();
    if text.is_empty() && kind == BlockKind::Paragraph && get_str(o, "type") == Some("image") {
        // An image that names no stored bytes: what is left of it is its
        // description, and that is kept as text.
        text = get_str(o, "alt").unwrap_or("").to_string();
    }
    let runs: Vec<Run> = match o.get("runs").and_then(Value::as_array) {
        Some(a) => a.iter().filter_map(parse_run).collect(),
        None if text.is_empty() => Vec::new(),
        None => vec![Run {
            len: text.len(),
            attrs: InlineAttrs::default(),
        }],
    };
    let mut b = Block::new(kind);
    b.id = BlockId(get_u64(o, "id").unwrap_or(0));
    b.alignment = get_str(o, "align")
        .and_then(Alignment::from_name)
        .unwrap_or_default();
    b.indent = get_u64(o, "indent").unwrap_or(0).min(u8::MAX as u64) as u8;
    b.text_size = get_f32(o, "size");
    b.text = text;
    b.runs = runs;
    // `Document::from_parts` / `Fragment::new` repair the rest.
    Some(b)
}

fn parse_run(v: &Value) -> Option<Run> {
    let o = v.as_object()?;
    let len = get_u64(o, "len").unwrap_or(0) as usize;
    let attrs = InlineAttrs {
        bold: get_bool(o, "bold"),
        italic: get_bool(o, "italic"),
        underline: get_bool(o, "underline"),
        strikethrough: get_bool(o, "strike"),
        code: get_bool(o, "code"),
        highlight: get_str(o, "highlight").and_then(Highlight::from_name),
        color: get_str(o, "color").and_then(Rgb::from_hex),
        font: get_str(o, "font").map(str::to_string),
        link: get_str(o, "link").map(str::to_string),
        comments: o
            .get("comments")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_u64).map(CommentId).collect())
            .unwrap_or_default(),
    };
    Some(Run { len, attrs })
}

fn parse_thread(v: &Value) -> Option<CommentThread> {
    let o = v.as_object()?;
    let replies = o
        .get("replies")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|r| {
                    let r = r.as_object()?;
                    Some(Reply {
                        id: ReplyId(get_u64(r, "id").unwrap_or(0)),
                        body: get_str(r, "body").unwrap_or("").to_string(),
                        author: get_str(r, "author").unwrap_or("").to_string(),
                        created_at: get_i64(r, "created").unwrap_or(0),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Some(CommentThread {
        id: CommentId(get_u64(o, "id").unwrap_or(0)),
        body: get_str(o, "body").unwrap_or("").to_string(),
        author: get_str(o, "author").unwrap_or("").to_string(),
        created_at: get_i64(o, "created").unwrap_or(0),
        resolved: get_bool(o, "resolved"),
        replies,
    })
}

fn parse_settings(o: &Map<String, Value>) -> DocumentSettings {
    let mut s = DocumentSettings::default();
    if let Some(p) = o.get("page").and_then(Value::as_object) {
        s.page.paper = match get_str(p, "paper") {
            Some("letter") => PaperSize::UsLetter,
            Some("custom") => PaperSize::Custom {
                width: get_f32(p, "width").unwrap_or(595.276),
                height: get_f32(p, "height").unwrap_or(841.89),
            },
            _ => PaperSize::A4,
        };
        if get_str(p, "orientation") == Some("landscape") {
            s.page.orientation = Orientation::Landscape;
        }
        if let Some(m) = p.get("margins").and_then(Value::as_object) {
            let d = Margins::default();
            s.page.margins = Margins {
                top: get_f32(m, "top").unwrap_or(d.top),
                right: get_f32(m, "right").unwrap_or(d.right),
                bottom: get_f32(m, "bottom").unwrap_or(d.bottom),
                left: get_f32(m, "left").unwrap_or(d.left),
            };
        }
    }
    if let Some(t) = o.get("typography").and_then(Value::as_object) {
        let d = Typography::default();
        s.typography = Typography {
            body_font: get_str(t, "font").map_or(d.body_font, str::to_string),
            base_size: get_f32(t, "size").unwrap_or(d.base_size),
            line_height: get_f32(t, "line_height").unwrap_or(d.line_height),
            paragraph_spacing: get_f32(t, "paragraph_spacing").unwrap_or(d.paragraph_spacing),
        };
    }
    s.page_view = get_bool(o, "page_view");
    s.spelling_language =
        get_str(o, "spelling_language").and_then(SpellingLanguage::from_setting_str);
    s.sanitized()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_document_is_compact() {
        let d = Document::new();
        assert_eq!(
            d.to_json(),
            r#"{"format":"the-writer-document","version":1,"blocks":[{"id":1}]}"#
        );
        assert_eq!(Document::from_json(&d.to_json()).unwrap(), d);
    }

    #[test]
    fn round_trip_everything() {
        let bold = InlineAttrs {
            bold: true,
            highlight: Some(Highlight::Pink),
            color: Some(Rgb::new(1, 2, 3)),
            font: Some("Menlo".into()),
            link: Some("https://x.y".into()),
            ..Default::default()
        };
        let blocks = vec![
            Block::with_text(BlockKind::heading(3), "T"),
            Block::paragraph("plain ")
                .with_span("bold", bold)
                .with_alignment(Alignment::Justify),
            Block::with_text(BlockKind::TodoItem { checked: true }, "do").with_indent(2),
            Block::with_text(
                BlockKind::CodeBlock {
                    language: Some("rust".into()),
                },
                "fn main() {}\n",
            ),
            Block::new(BlockKind::PageBreak),
            Block::paragraph("big").with_text_size(Some(23.5)),
        ];
        let mut settings = DocumentSettings::default();
        settings.page.paper = PaperSize::Custom {
            width: 400.0,
            height: 500.5,
        };
        settings.page.orientation = Orientation::Landscape;
        settings.typography.line_height = 1.45;
        settings.page_view = true;
        let threads = vec![CommentThread {
            id: CommentId(4),
            body: "b".into(),
            author: "a".into(),
            created_at: 99,
            resolved: true,
            replies: vec![Reply {
                id: ReplyId(1),
                body: "r".into(),
                author: "z".into(),
                created_at: 100,
            }],
        }];
        let d = Document::from_parts(blocks, threads, settings);
        let json = d.to_json();
        let back = Document::from_json(&json).unwrap();
        assert_eq!(back, d);
        assert!(back.check_invariants().is_ok());
        // serde path
        let via_serde: Document = serde_json::from_str(&json).unwrap();
        assert_eq!(via_serde, d);
    }

    #[test]
    fn spelling_language_round_trips_compactly() {
        let with = |l: Option<SpellingLanguage>| {
            Document::new().with_settings(DocumentSettings {
                spelling_language: l,
                ..Default::default()
            })
        };
        // Not set: nothing is written, exactly as version 0.1.0 wrote it.
        assert_eq!(
            with(None).to_json(),
            r#"{"format":"the-writer-document","version":1,"blocks":[{"id":1}]}"#
        );
        let fixed = with(Some(SpellingLanguage::Language("en_GB".into())));
        assert_eq!(
            fixed.to_json(),
            r#"{"format":"the-writer-document","version":1,"settings":{"spelling_language":"en_GB"},"blocks":[{"id":1}]}"#
        );
        assert_eq!(Document::from_json(&fixed.to_json()).unwrap(), fixed);
        let auto = with(Some(SpellingLanguage::Automatic));
        assert_eq!(
            auto.to_json(),
            r#"{"format":"the-writer-document","version":1,"settings":{"spelling_language":"auto"},"blocks":[{"id":1}]}"#
        );
        assert_eq!(Document::from_json(&auto.to_json()).unwrap(), auto);
        let via_serde: Document = serde_json::from_str(&fixed.to_json()).unwrap();
        assert_eq!(via_serde, fixed);
    }

    #[test]
    fn spelling_language_is_read_leniently() {
        let read = |settings: &str| {
            let json = format!(r#"{{"blocks":[{{"id":1,"text":"x"}}],"settings":{settings}}}"#);
            let d = Document::from_json(&json).unwrap();
            d.check_invariants().unwrap();
            d.settings().clone()
        };
        // A document of version 0.1.0: the key is not there.
        let old = read(r#"{"page_view":true,"typography":{"size":19}}"#);
        assert_eq!(old.spelling_language, None);
        assert!(old.page_view);
        assert_eq!(old.typography.base_size, 19.0);
        // Trimmed; "auto" in any case; the other settings are untouched.
        let s = read(r#"{"page_view":true,"spelling_language":"  fr_CA "}"#);
        assert_eq!(
            s.spelling_language,
            Some(SpellingLanguage::Language("fr_CA".into()))
        );
        assert!(s.page_view);
        assert_eq!(
            read(r#"{"spelling_language":"Automatic"}"#).spelling_language,
            Some(SpellingLanguage::Automatic)
        );
        // Anything that is not a usable identifier reads as "not set".
        for bad in [
            r#""""#,
            r#""  ""#,
            r#""en GB""#,
            "7",
            "null",
            "true",
            r#"["en"]"#,
            r#"{"id":"en"}"#,
            r#""<script>""#,
        ] {
            let s = read(&format!(
                r#"{{"spelling_language":{bad},"page_view":true}}"#
            ));
            assert_eq!(s.spelling_language, None, "{bad}");
            assert!(s.page_view, "{bad}");
        }
        let long = format!(r#"{{"spelling_language":"{}"}}"#, "x".repeat(4000));
        assert_eq!(read(&long).spelling_language, None);
    }

    #[test]
    fn lenient_reading() {
        let json = r#"{
            "blocks": [
                {"id": 3, "type": "heading", "level": 42, "text": "Hi\r\nthere", "runs": [{"len": 1, "bold": true}]},
                {"id": 3, "type": "weird", "text": "dup id", "indent": 99, "align": "sideways"},
                {"type": "divider", "text": "lost?"},
                "not a block",
                {"text": "é", "runs": [{"len": 1, "italic": true}, {"len": 7}]},
                {"text": "x", "runs": [{"len": 1, "comments": [77]}], "size": "big"}
            ],
            "comments": [{"id": 0, "body": 5}, {"id": 2, "replies": [{"id": 1}, {"id": 1}]}],
            "settings": {"typography": {"size": -3}, "page": {"margins": {"top": "x"}}}
        }"#;
        let d = Document::from_json(json).unwrap();
        assert!(d.check_invariants().is_ok(), "{:?}", d.check_invariants());
        assert_eq!(d.blocks()[0].kind(), &BlockKind::heading(6));
        assert_eq!(d.blocks()[0].text(), "Hi\nthere");
        assert_eq!(d.blocks()[1].kind(), &BlockKind::Paragraph);
        assert_eq!(d.blocks()[1].indent(), 8);
        assert!(d.blocks()[2].is_void());
        assert_eq!(d.blocks()[3].text(), "lost?");
        assert_eq!(d.blocks()[4].text(), "é");
        assert!(d.blocks()[5].comment_ids().is_empty());
        assert_eq!(d.comment_count(), 2);
        assert_eq!(d.settings().typography.base_size, 6.0);
    }

    #[test]
    fn extreme_numbers_are_repaired() {
        let json = format!(
            r#"{{"blocks": [
                {{"id": {max}, "text": "ab", "runs": [{{"len": 1e300, "bold": true}}, {{"len": {max}}}]}},
                {{"id": 18446744073709551615, "indent": 1e9, "size": 1e39, "text": "c"}}
            ],
            "comments": [{{"id": {max}, "created": -1e300, "replies": [{{"id": {max}}}]}}]}}"#,
            max = u64::MAX
        );
        let d = Document::from_json(&json).unwrap();
        d.check_invariants().unwrap();
        assert!(d.blocks()[0].attrs_at(1).unwrap().bold);
        assert_eq!(d.blocks()[1].indent(), crate::MAX_INDENT);
        let mut e = crate::EditorState::new(d);
        e.select_all();
        let id = e.add_comment("x", "y", 0).unwrap();
        let t = e.document().comments().next().unwrap().id;
        assert!(e.add_reply(t, "r", "a", 0).is_some());
        assert_ne!(id, t);
        e.insert_paragraph_break();
        e.check_invariants().unwrap();
    }

    #[test]
    fn unrecoverable_inputs() {
        assert!(matches!(
            Document::from_json("nope"),
            Err(LoadError::Json(_))
        ));
        assert!(matches!(
            Document::from_json("[]"),
            Err(LoadError::NotADocument(_))
        ));
        assert!(matches!(
            Document::from_json("{}"),
            Err(LoadError::NotADocument(_))
        ));
        assert!(matches!(
            Document::from_json(r#"{"version": 7, "blocks": []}"#),
            Err(LoadError::UnsupportedVersion(7))
        ));
        assert!(matches!(
            Document::from_json(r#"{"format": "other", "blocks": []}"#),
            Err(LoadError::NotADocument(_))
        ));
        // Empty block list is repaired, not rejected.
        assert_eq!(
            Document::from_json(r#"{"blocks": []}"#)
                .unwrap()
                .block_count(),
            1
        );
    }

    #[test]
    fn fragment_round_trip() {
        let f = Fragment::new(vec![
            Block::with_text(BlockKind::Quote, "q"),
            Block::paragraph("p").with_span(
                "i",
                InlineAttrs {
                    italic: true,
                    ..Default::default()
                },
            ),
        ]);
        let json = f.to_json();
        assert!(!json.contains("\"id\""));
        assert_eq!(Fragment::from_json(&json).unwrap(), f);
        let doc_json = Document::new().to_json();
        assert_eq!(Fragment::from_json(&doc_json).unwrap().blocks().len(), 1);
    }

    // ----- images -----------------------------------------------------------------

    const HARBOUR: &str = "3f2a9c1b5d6e7f80a1b2c3d4e5f60718";

    /// The example of the module documentation, as it is written compact.
    const HARBOUR_BLOCK: &str = r#"{"id":7,"type":"image","image":"3f2a9c1b5d6e7f80a1b2c3d4e5f60718","px":[1600,1067],"ppi":144,"width":500,"alt":"The harbour at dusk","align":"center"}"#;

    fn harbour() -> Image {
        Image {
            width: ImageWidth::new(500),
            description: "The harbour at dusk".into(),
            ..Image::new(ImageId::parse(HARBOUR).unwrap(), (1600, 1067), 144)
        }
    }

    fn document_of(blocks: &str) -> String {
        format!(r#"{{"format":"the-writer-document","version":2,"blocks":[{blocks}]}}"#)
    }

    #[test]
    fn an_image_round_trips_byte_for_byte() {
        let json = document_of(HARBOUR_BLOCK);
        let d = Document::from_json(&json).unwrap();
        d.check_invariants().unwrap();
        assert_eq!(d.block_count(), 1);
        let b = &d.blocks()[0];
        assert_eq!(b.id(), BlockId(7));
        assert_eq!(b.as_image(), Some(&harbour()));
        assert_eq!(b.alignment(), Alignment::Center);
        assert_eq!(d.to_json(), json);
        // The same through serde, and the pretty form reads back equal.
        let via_serde: Document = serde_json::from_str(&json).unwrap();
        assert_eq!(via_serde, d);
        assert_eq!(serde_json::to_string(&via_serde).unwrap(), json);
        assert_eq!(Document::from_json(&d.to_json_pretty()).unwrap(), d);
        // The documentation's example, with its spaces and line break.
        let spaced = r#"{ "blocks": [
            { "id": 7, "type": "image", "image": "3f2a9c1b5d6e7f80a1b2c3d4e5f60718",
              "px": [1600, 1067], "ppi": 144, "width": 500, "alt": "The harbour at dusk", "align": "center" }
        ] }"#;
        assert_eq!(Document::from_json(spaced).unwrap().to_json(), json);
        // Built from the model it is the same text.
        let built = Document::from_blocks(vec![
            Block::image(harbour()).with_alignment(Alignment::Center),
        ]);
        assert_eq!(
            built.to_json(),
            document_of(&HARBOUR_BLOCK.replace(r#""id":7"#, r#""id":1"#))
        );
    }

    #[test]
    fn an_image_omits_what_has_its_default() {
        let plain = Image::new(ImageId::parse(HARBOUR).unwrap(), (640, 480), 72);
        let d = Document::from_blocks(vec![Block::image(plain.clone())]);
        let json = d.to_json();
        assert_eq!(
            json,
            document_of(
                r#"{"id":1,"type":"image","image":"3f2a9c1b5d6e7f80a1b2c3d4e5f60718","px":[640,480]}"#
            )
        );
        let back = Document::from_json(&json).unwrap();
        assert_eq!(back, d);
        assert_eq!(back.blocks()[0].as_image(), Some(&plain));
        assert_eq!(back.blocks()[0].alignment(), Alignment::Left);
        // Each key appears as soon as its value is not the default, and
        // only then.
        let with = |change: &dyn Fn(&mut Image)| {
            let mut image = plain.clone();
            change(&mut image);
            Document::from_blocks(vec![Block::image(image)]).to_json()
        };
        let ppi = with(&|i| i.density = 144);
        assert!(ppi.contains(r#""ppi":144"#), "{ppi}");
        assert!(!ppi.contains("width") && !ppi.contains("alt"), "{ppi}");
        let width = with(&|i| i.width = ImageWidth::new(999));
        assert!(width.contains(r#""width":999"#), "{width}");
        assert!(!width.contains("ppi") && !width.contains("alt"), "{width}");
        let alt = with(&|i| i.description = "A \"quoted\" boat".into());
        assert!(alt.contains(r#""alt":"A \"quoted\" boat""#), "{alt}");
        assert!(!alt.contains("ppi") && !alt.contains("width"), "{alt}");
        // No key of a text block is written for an image.
        for key in ["indent", "size", "text", "runs", "level", "lang", "checked"] {
            assert!(!json.contains(&format!("\"{key}\"")), "{key}: {json}");
        }
    }

    #[test]
    fn the_token_the_library_looks_for_is_written_without_a_space() {
        let d = Document::from_blocks(vec![
            Block::paragraph("before"),
            Block::image(harbour()),
            Block::paragraph("after"),
        ]);
        let token = format!(r#""image":"{HARBOUR}""#);
        assert_eq!(d.to_json().matches(&token).count(), 1, "{}", d.to_json());
        // A fragment holds the same token.
        let f = d.to_fragment();
        assert_eq!(f.to_json().matches(&token).count(), 1, "{}", f.to_json());
        // Text that looks like the token is written as text, with its
        // quotes escaped: it cannot be mistaken for an image.
        let fake = Document::from_blocks(vec![Block::paragraph(&token)]);
        assert!(!fake.to_json().contains(&token), "{}", fake.to_json());
    }

    #[test]
    fn the_version_is_two_only_with_an_image() {
        let text =
            Document::from_blocks(vec![Block::paragraph("a"), Block::new(BlockKind::Divider)]);
        assert!(
            text.to_json()
                .starts_with(r#"{"format":"the-writer-document","version":1,"#),
            "{}",
            text.to_json()
        );
        assert!(
            text.to_fragment()
                .to_json()
                .starts_with(r#"{"format":"the-writer-fragment","version":1,"#)
        );
        let with = Document::from_blocks(vec![Block::paragraph("a"), Block::image(harbour())]);
        assert!(
            with.to_json()
                .starts_with(r#"{"format":"the-writer-document","version":2,"#),
            "{}",
            with.to_json()
        );
        assert!(
            with.to_fragment()
                .to_json()
                .starts_with(r#"{"format":"the-writer-fragment","version":2,"#)
        );
        assert_eq!(Document::from_json(&with.to_json()).unwrap(), with);
        assert_eq!(
            Fragment::from_json(&with.to_fragment().to_json()).unwrap(),
            with.to_fragment()
        );
        // A fragment of the text alone is version 1 again, and so is the
        // document once the image is gone.
        assert!(
            with.block_fragment(0..1)
                .to_json()
                .starts_with(r#"{"format":"the-writer-fragment","version":1,"#)
        );
        let mut e = crate::EditorState::new(with);
        e.set_caret(crate::Position::new(1, 0));
        assert!(e.backspace());
        assert!(
            e.document()
                .to_json()
                .starts_with(r#"{"format":"the-writer-document","version":1,"#)
        );
    }

    #[test]
    fn a_build_from_before_images_refuses_a_document_with_one() {
        let with: Value = serde_json::from_str(&document_of(HARBOUR_BLOCK)).unwrap();
        // The reader as it was while the newest version was 1.
        assert_eq!(
            header_read_by(&with, DOCUMENT_FORMAT, 1).err(),
            Some(LoadError::UnsupportedVersion(2))
        );
        assert_eq!(
            header_read_by(&with, FRAGMENT_FORMAT, 1).err(),
            Some(LoadError::UnsupportedVersion(2))
        );
        // It still takes every document without an image this build writes.
        let without: Value = serde_json::from_str(&Document::new().to_json()).unwrap();
        assert!(header_read_by(&without, DOCUMENT_FORMAT, 1).is_ok());
        // And this build refuses what comes after it, in the same words.
        assert!(header(&with, DOCUMENT_FORMAT).is_ok());
        let later = r#"{"version": 3, "blocks": []}"#;
        assert_eq!(
            Document::from_json(later).err(),
            Some(LoadError::UnsupportedVersion(3))
        );
        assert_eq!(
            Fragment::from_json(later).err(),
            Some(LoadError::UnsupportedVersion(3))
        );
        assert_eq!(
            LoadError::UnsupportedVersion(2).to_string(),
            "document format version 2 is newer than this app supports"
        );
        assert_eq!(FORMAT_VERSION, 2);
        assert_eq!(FORMAT_VERSION_WITHOUT_IMAGES, 1);
    }

    #[test]
    fn an_image_is_read_leniently() {
        let read = |block: &str| {
            let d = Document::from_json(&format!(r#"{{"blocks":[{block}]}}"#)).unwrap();
            d.check_invariants().unwrap();
            d
        };
        // Without an id that can be read there is no image: the description
        // stays, as a paragraph.
        for bad in [
            r#""image":"nope","#,
            r#""image":"3f2a9c1b5d6e7f80a1b2c3d4e5f6071","#,
            r#""image":"3f2a9c1b5d6e7f80a1b2c3d4e5f607180","#,
            r#""image":"3f2a9c1b5d6e7f80a1b2c3d4e5f6071g","#,
            r#""image":7,"#,
            r#""image":null,"#,
            r#""image":["3f2a9c1b5d6e7f80a1b2c3d4e5f60718"],"#,
            "",
        ] {
            let d = read(&format!(
                r#"{{"id":4,"type":"image",{bad}"px":[10,10],"alt":"The harbour at dusk","align":"center"}}"#
            ));
            assert_eq!(d.block_count(), 1, "{bad}");
            let b = &d.blocks()[0];
            assert_eq!(b.kind(), &BlockKind::Paragraph, "{bad}");
            assert_eq!(b.text(), "The harbour at dusk", "{bad}");
            assert_eq!(b.id(), BlockId(4), "{bad}");
            assert_eq!(b.alignment(), Alignment::Center, "{bad}");
            assert!(!d.has_images());
            // What is written then is a document without an image.
            assert!(d.to_json().contains(r#""version":1"#), "{bad}");
        }
        // Nothing at all to keep: an empty paragraph.
        let d = read(r#"{"type":"image"}"#);
        assert_eq!(d.blocks()[0].kind(), &BlockKind::Paragraph);
        assert_eq!(d.blocks()[0].text(), "");

        // An id in capitals is the same id.
        let d = read(&format!(
            r#"{{"type":"image","image":"{}","px":[3,4]}}"#,
            HARBOUR.to_uppercase()
        ));
        let image = d.blocks()[0].as_image().unwrap();
        assert_eq!(image.id.to_string(), HARBOUR);
        assert_eq!(image.pixels, (3, 4));

        // A missing or unreadable pixel size is 1 x 1; the image stays.
        for px in [
            "",
            r#""px":null,"#,
            r#""px":"1600x1067","#,
            r#""px":[1600],"#,
            r#""px":[1600,1067,3],"#,
            r#""px":[1600,"x"],"#,
            r#""px":[-5,10],"#,
            r#""px":{"w":1,"h":2},"#,
        ] {
            let d = read(&format!(r#"{{"type":"image",{px}"image":"{HARBOUR}"}}"#));
            let image = d.blocks()[0].as_image().expect(px);
            assert_eq!(image.pixels, (1, 1), "{px}");
            assert_eq!(image.density, 72, "{px}");
            assert_eq!(image.width, ImageWidth::FULL, "{px}");
            assert_eq!(image.description, "", "{px}");
        }
        // Zero is not a size, and numbers beyond the type are its largest.
        let d = read(&format!(
            r#"{{"type":"image","image":"{HARBOUR}","px":[0,1e300]}}"#
        ));
        assert_eq!(d.blocks()[0].as_image().unwrap().pixels, (1, u32::MAX));
        let d = read(&format!(
            r#"{{"type":"image","image":"{HARBOUR}","px":[640.9,480]}}"#
        ));
        assert_eq!(d.blocks()[0].as_image().unwrap().pixels, (640, 480));

        // Everything else is brought into range, as for every block.
        let d = read(&format!(
            r#"{{"type":"image","image":"{HARBOUR}","px":[8,6],"ppi":5,"width":7,
                "alt":"  two\r\nlines\t ","align":"justify","indent":3,"size":30}}"#
        ));
        let b = &d.blocks()[0];
        let image = b.as_image().unwrap();
        assert_eq!(image.density, Image::MIN_DENSITY);
        assert_eq!(image.width, ImageWidth::MIN);
        assert_eq!(image.description, "two lines");
        assert_eq!(b.alignment(), Alignment::Left);
        assert_eq!(b.indent(), 0);
        assert_eq!(b.text_size(), None);
        let d = read(&format!(
            r#"{{"type":"image","image":"{HARBOUR}","px":[8,6],"ppi":1e9,"width":70000,"alt":7,"align":"right"}}"#
        ));
        let b = &d.blocks()[0];
        let image = b.as_image().unwrap();
        assert_eq!(image.density, Image::MAX_DENSITY);
        assert_eq!(image.width, ImageWidth::FULL);
        assert_eq!(image.description, "");
        assert_eq!(b.alignment(), Alignment::Right);
        // Values of the wrong type fall back to the defaults.
        let d = read(&format!(
            r#"{{"type":"image","image":"{HARBOUR}","px":[8,6],"ppi":"dense","width":"half"}}"#
        ));
        let image = d.blocks()[0].as_image().unwrap();
        assert_eq!(image.density, 72);
        assert_eq!(image.width, ImageWidth::FULL);

        // Text an image block came with is not lost: it follows the image
        // in a paragraph of its own.
        let d = read(&format!(
            r#"{{"id":2,"type":"image","image":"{HARBOUR}","px":[8,6],"text":"caption","align":"center"}}"#
        ));
        assert_eq!(d.block_count(), 2);
        assert!(d.blocks()[0].kind().is_image());
        assert_eq!(d.blocks()[0].id(), BlockId(2));
        assert_eq!(d.blocks()[1].text(), "caption");

        // The keys of an image mean nothing on another kind of block.
        let d = read(&format!(
            r#"{{"text":"plain","image":"{HARBOUR}","px":[8,6],"alt":"not this"}}"#
        ));
        assert_eq!(d.blocks()[0].kind(), &BlockKind::Paragraph);
        assert_eq!(d.blocks()[0].text(), "plain");
        assert!(d.to_json().contains(r#""version":1"#));
    }

    #[test]
    fn a_fragment_keeps_an_image_whole() {
        let f = Fragment::new(vec![
            Block::paragraph("above"),
            Block::image(harbour()).with_alignment(Alignment::Right),
        ]);
        let json = f.to_json();
        assert!(!json.contains("\"id\""), "{json}");
        let back = Fragment::from_json(&json).unwrap();
        assert_eq!(back, f);
        assert_eq!(back.blocks()[1].as_image(), Some(&harbour()));
        assert_eq!(back.blocks()[1].alignment(), Alignment::Right);
        // A whole document read as a fragment brings its images along.
        let d = Document::from_json(&document_of(HARBOUR_BLOCK)).unwrap();
        let from_document = Fragment::from_json(&d.to_json()).unwrap();
        assert_eq!(from_document.blocks()[0].as_image(), Some(&harbour()));
    }
}
