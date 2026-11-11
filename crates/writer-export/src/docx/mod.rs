//! DOCX export: an Office Open XML package that Word, Pages and Google
//! Docs open.
//!
//! # What is written
//!
//! - **Styles.** Normal takes the document typography (body font, base
//!   size, line height, paragraph spacing); Heading 1 to 6 use the heading
//!   sizes of [`Typography::size_for`](writer_core::Typography::size_for);
//!   Quote, Code, List Paragraph and Hyperlink are defined too, so the
//!   result can be restyled in Word by changing a style.
//! - **Blocks.** Headings, paragraphs, quotes and code blocks use those
//!   styles. Bullet and numbered items use real Word numbering, nested by
//!   indent level, and every separate numbered list restarts at 1: a list
//!   that restarts is a numbering instance with a start override, and
//!   instances are reused wherever a reader restarts or continues the
//!   count by itself. Word cannot address more than 2,046 instances, so at
//!   most 1,000 are written, over at most 128 list definitions, however
//!   long the document is; numbered lists beyond that carry their numbers
//!   as text. To-dos
//!   are list paragraphs that start with a ballot box character (☐ / ☑). A
//!   divider is an empty paragraph with a bottom border, a page break
//!   makes the next paragraph start on a new page (`w:pageBreakBefore`). Code blocks keep
//!   their whitespace and line breaks. Alignment and indent levels are
//!   kept. Indents and spacing follow the editor's page view: 24 pt per
//!   indent level, list items closer together than paragraphs.
//! - **Runs.** Bold, italic, underline, strikethrough, highlight (as run
//!   shading in the highlight's reference colour), text colour, font
//!   family and the block's text size; inline code in the monospace font
//!   with shading; links as real hyperlinks; soft line breaks.
//!   Right-to-left paragraphs and runs are marked as such.
//! - **Page setup.** Paper size, orientation and margins of the document
//!   settings, and optionally a centred page-number footer. Word's largest
//!   page is 22 inches each way; a larger custom page is cut to that.
//! - **Comments.** With [`DocxOptions::comments`], every anchored thread is
//!   a Word comment on its text range, with its replies threaded under it
//!   and resolved threads marked done.
//! - **Collections.** Every document is its own section starting on a new
//!   page, with its own page setup and its own typography. The styles
//!   are written from the first document; a document whose font, text
//!   size, line height or paragraph spacing differ carries them on its
//!   own paragraphs and runs, so that it looks in the collection exactly
//!   as it does exported alone, and its headings are still headings.
//!   Page breaks at the very start or end of a document, where the next
//!   section starts a new page anyway, are dropped so that no empty page
//!   appears between two documents.
//!
//! # Sizes
//!
//! The document stores sizes in points (on screen one point is one logical
//! pixel), and DOCX uses points too, so sizes map one to one, multiplied by
//! [`DocxOptions::text_scale`]: 17 pt body text is `w:sz="34"` (half
//! points), a 72 pt margin is 1440 twips. Line height is a multiple of the
//! font size in the document but a multiple of the font's own line height
//! in Word, which is about 1.2 times the size, so a line height of 1.6
//! becomes Word line spacing 1.6 / 1.2 = 1.33 (`w:line="320"`).
//!
//! ```
//! use writer_core::{Block, Document};
//! use writer_export::{DocxOptions, ExportSource, to_docx};
//!
//! let doc = Document::from_blocks(vec![Block::paragraph("Hello")]);
//! let options = DocxOptions {
//!     author: "Ada".into(),
//!     created: 1_794_391_505_000, // milliseconds since the Unix epoch
//!     modified: 1_794_391_505_000,
//!     ..DocxOptions::default()
//! };
//! let bytes = to_docx(&ExportSource::single("Greeting", &doc), &options);
//! assert!(bytes.starts_with(b"PK"));
//! ```

mod body;
mod numbering;
mod styles;
mod xml;
mod zip;

use writer_core::{DocumentSettings, Timestamp};

use crate::common::iso_datetime;
use crate::{ExportSource, PartTitles};
use xml::{DECLARATION, attr, text};

pub(crate) const W_NS: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
pub(crate) const R_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const W14_NS: &str = "http://schemas.microsoft.com/office/word/2010/wordml";
const W15_NS: &str = "http://schemas.microsoft.com/office/word/2012/wordml";
const MC_NS: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
const REL_BASE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const WML: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml";

/// The file names inside the package, in the order they are written.
pub mod parts {
    /// Content types of all parts.
    pub const CONTENT_TYPES: &str = "[Content_Types].xml";
    /// Package relationships.
    pub const ROOT_RELS: &str = "_rels/.rels";
    /// Core properties: title, author, timestamps.
    pub const CORE: &str = "docProps/core.xml";
    /// Extended properties: the application name.
    pub const APP: &str = "docProps/app.xml";
    /// The document body.
    pub const DOCUMENT: &str = "word/document.xml";
    /// Relationships of the document body.
    pub const DOCUMENT_RELS: &str = "word/_rels/document.xml.rels";
    /// Styles.
    pub const STYLES: &str = "word/styles.xml";
    /// Numbering definitions.
    pub const NUMBERING: &str = "word/numbering.xml";
    /// Document settings.
    pub const SETTINGS: &str = "word/settings.xml";
    /// The page-number footer (only with [`DocxOptions::page_numbers`](super::DocxOptions::page_numbers)).
    pub const FOOTER: &str = "word/footer1.xml";
    /// Comments (only when comments are exported).
    pub const COMMENTS: &str = "word/comments.xml";
    /// Comment threading and resolved state (only when comments are exported).
    pub const COMMENTS_EXTENDED: &str = "word/commentsExtended.xml";
}

/// Options for [`to_docx`].
#[derive(Clone, Debug, PartialEq)]
pub struct DocxOptions {
    /// Export comment threads as Word comments (default on). Threads whose
    /// text was deleted have no place in the document and are left out.
    pub comments: bool,
    /// Add a footer with the page number, centred (default on, as in the
    /// app's page view).
    pub page_numbers: bool,
    /// The author written to the document properties. May be empty.
    pub author: String,
    /// Creation time for the document properties, in milliseconds since
    /// the Unix epoch. The caller supplies it: this crate has no clock.
    pub created: Timestamp,
    /// Modification time, in milliseconds since the Unix epoch. Also the
    /// time stamp of the files inside the package.
    pub modified: Timestamp,
    /// Multiplies every text size, the paragraph spacing and the list
    /// indents (default 1.0). The document's 17 pt screen size is large on
    /// paper; `12.0 / 17.0` gives a conventional 12 pt manuscript.
    pub text_scale: f32,
    /// The font of code blocks and inline code (default `"Menlo"`).
    pub code_font: String,
    /// Whether the documents of a collection get their title as a
    /// Heading 1.
    pub part_titles: PartTitles,
}

impl Default for DocxOptions {
    fn default() -> DocxOptions {
        DocxOptions {
            comments: true,
            page_numbers: true,
            author: String::new(),
            created: 0,
            modified: 0,
            text_scale: 1.0,
            code_font: "Menlo".to_string(),
            part_titles: PartTitles::default(),
        }
    }
}

/// Unit conversions shared by the parts.
pub(crate) struct Metrics {
    scale: f32,
    pub code_font: String,
}

/// The indent of one level, the room left of a list item's text for its
/// marker, and the room left of a quote's text for its bar, in points.
/// These are the editor's own measures, so the export looks like the page
/// view.
const INDENT_STEP: f32 = 24.0;
const MARKER_GUTTER: f32 = 26.0;
const QUOTE_GUTTER: f32 = 20.0;
/// Room between the end of a list number and the item's text.
const NUMBER_GAP: f32 = 7.0;
/// Inner padding of a code block: left and right, top and bottom.
const CODE_PADDING: f32 = 14.0;
const CODE_PADDING_V: f32 = 10.0;
/// Headings are set tighter than body text.
const HEADING_LINE_HEIGHT: f32 = 1.3;

impl Metrics {
    fn new(options: &DocxOptions) -> Metrics {
        let scale = if options.text_scale.is_finite() {
            options.text_scale.clamp(0.1, 10.0)
        } else {
            1.0
        };
        let code_font = options.code_font.trim();
        Metrics {
            scale,
            code_font: if code_font.is_empty() {
                "Menlo".to_string()
            } else {
                code_font.to_string()
            },
        }
    }

    /// The left indent of a block at indent `level`, in twips.
    pub fn indent(&self, level: u8) -> i32 {
        self.twips(INDENT_STEP * f32::from(level))
    }

    /// The hanging indent of a list item (where its marker sits), in twips.
    pub fn marker_gutter(&self) -> i32 {
        self.twips(MARKER_GUTTER)
    }

    /// Where a list number ends, measured back from the item's text, in
    /// twips.
    pub fn number_gap(&self) -> i32 {
        self.twips(NUMBER_GAP)
    }

    /// Where the middle of a bullet sits, measured back from the item's
    /// text, in twips.
    pub fn bullet_offset(&self) -> i32 {
        self.twips(MARKER_GUTTER / 2.0)
    }

    /// The inner padding of a code block in points: `(sides, top and
    /// bottom)`. Paragraph borders carry it, and their distance from the
    /// text is limited to 31 pt.
    pub fn code_padding(&self) -> (i32, i32) {
        let pt = |v: f32| ((v * self.scale).round() as i32).clamp(1, 31);
        (pt(CODE_PADDING), pt(CODE_PADDING_V))
    }

    /// The height of a body line in points: the unit the editor measures
    /// heading and divider spacing in.
    pub fn base_line(&self, typography: &writer_core::Typography) -> f32 {
        (typography.base_size * typography.line_height)
            .round()
            .max(1.0)
    }

    /// The space after a paragraph in twips.
    pub fn paragraph_gap(&self, typography: &writer_core::Typography) -> i32 {
        self.twips(typography.paragraph_spacing.round())
    }

    /// The space between two neighbouring list items in twips.
    pub fn list_gap(&self, typography: &writer_core::Typography) -> i32 {
        self.twips((typography.paragraph_spacing.round() * 0.33).round())
    }

    /// The space between two neighbouring quote paragraphs in twips.
    pub fn quote_gap(&self, typography: &writer_core::Typography) -> i32 {
        self.twips((typography.paragraph_spacing.round() * 0.5).round())
    }

    /// The indent of a quote's text from its bar, in twips.
    pub fn quote_gutter(&self) -> i32 {
        self.twips(QUOTE_GUTTER)
    }

    /// A text size in points as scaled half-points (`w:sz`).
    pub fn half_points(&self, points: f32) -> u32 {
        let half = (points * self.scale * 2.0).round();
        if half.is_finite() {
            half.clamp(2.0, 3276.0) as u32
        } else {
            24
        }
    }

    /// A text-related length in points as scaled twips.
    pub fn twips(&self, points: f32) -> i32 {
        (points * self.scale * 20.0).round() as i32
    }

    /// The line height of text of `size` points in `font`, set at
    /// `ratio` times its size, as `w:line` for `lineRule="auto"`.
    ///
    /// The editor makes a line `size * ratio` high, rounded to whole
    /// points, whatever the font. Word's unit (240) is the font's own
    /// line height instead, which differs from font to font
    /// ([`natural_line_height`]).
    pub fn line(&self, font: &str, size: f32, ratio: f32) -> i32 {
        let size = if size.is_finite() && size > 0.0 {
            size
        } else {
            12.0
        };
        let pitch = (size * ratio).round().max(1.0);
        let multiple = pitch / (size * natural_line_height(font));
        if multiple.is_finite() {
            (multiple * 240.0).round().clamp(120.0, 1200.0) as i32
        } else {
            240
        }
    }

    /// `w:line` of a heading of `size` points: headings are set tighter
    /// than body text.
    pub fn heading_line(&self, typography: &writer_core::Typography, size: f32) -> i32 {
        self.line(
            &typography.body_font,
            size,
            typography.line_height.min(HEADING_LINE_HEIGHT),
        )
    }
}

/// A font's own line height as a multiple of its size: ascent plus descent
/// plus line gap of its `hhea` table, which is what Word and LibreOffice
/// call "single" spacing. Measured from the fonts macOS ships (the ones
/// the app offers, and the common ones besides); 1.2 for any other font.
pub(crate) fn natural_line_height(font: &str) -> f32 {
    const KNOWN: &[(&str, f32)] = &[
        ("charter", 1.2202),
        ("iowan old style", 1.3652),
        ("georgia", 1.1362),
        ("palatino", 1.1001),
        ("baskerville", 1.144),
        ("hoefler text", 1.0),
        ("times new roman", 1.1499),
        ("times", 1.0),
        ("new york", 1.1934),
        ("helvetica neue", 1.193),
        ("helvetica", 1.0),
        ("arial", 1.1499),
        ("avenir next", 1.366),
        ("avenir", 1.366),
        ("menlo", 1.1641),
        ("monaco", 1.3335),
        ("courier new", 1.1328),
        ("verdana", 1.2153),
        ("optima", 1.212),
        ("gill sans", 1.1484),
        ("didot", 1.264),
        ("cochin", 1.147),
        ("athelas", 1.12),
        ("seravek", 1.227),
        ("pt serif", 1.294),
        ("pt sans", 1.295),
        ("american typewriter", 1.154),
        ("futura", 1.3281),
    ];
    let name = font.trim().to_lowercase();
    KNOWN
        .iter()
        .find(|(known, _)| *known == name)
        .map_or(1.2, |(_, height)| *height)
}

/// A page length in points as twips. Page geometry is never scaled.
pub(crate) fn page_twips(points: f32) -> i32 {
    (points * 20.0).round() as i32
}

/// Relationship ids of the fixed parts. Hyperlinks follow from
/// [`FIRST_LINK_REL`].
const STYLES_REL: &str = "rId1";
const NUMBERING_REL: &str = "rId2";
const SETTINGS_REL: &str = "rId3";
pub(crate) const FOOTER_REL: &str = "rId4";
const COMMENTS_REL: &str = "rId5";
const COMMENTS_EXTENDED_REL: &str = "rId6";
pub(crate) const FIRST_LINK_REL: usize = 7;

/// Exports `source` as a DOCX package and returns the file's bytes.
///
/// The output depends only on the arguments: the same source and options
/// give the same bytes.
pub fn to_docx(source: &ExportSource<'_>, options: &DocxOptions) -> Vec<u8> {
    let default_settings = DocumentSettings::default();
    let first_settings = source
        .parts()
        .first()
        .map(|p| p.document.settings().clone().sanitized())
        .unwrap_or(default_settings);
    let metrics = Metrics::new(options);
    let body = body::build(source, options, &metrics);
    let has_comments = !body.comments.is_empty();

    let mut zip = zip::ZipWriter::new(options.modified);
    zip.add(
        parts::CONTENT_TYPES,
        content_types(options.page_numbers, has_comments).as_bytes(),
    );
    zip.add(parts::ROOT_RELS, root_rels().as_bytes());
    zip.add(
        parts::CORE,
        core_properties(source.title(), options).as_bytes(),
    );
    zip.add(parts::APP, app_properties().as_bytes());
    zip.add(parts::DOCUMENT, body.document.as_bytes());
    zip.add(
        parts::DOCUMENT_RELS,
        document_rels(options.page_numbers, has_comments, &body.links).as_bytes(),
    );
    zip.add(
        parts::STYLES,
        styles::styles_xml(&first_settings.typography, &metrics).as_bytes(),
    );
    zip.add(
        parts::NUMBERING,
        styles::numbering_xml(&metrics, &body.numbered).as_bytes(),
    );
    zip.add(parts::SETTINGS, settings_xml().as_bytes());
    if options.page_numbers {
        zip.add(parts::FOOTER, footer_xml().as_bytes());
    }
    if has_comments {
        zip.add(parts::COMMENTS, comments_xml(&body.comments).as_bytes());
        zip.add(
            parts::COMMENTS_EXTENDED,
            comments_extended_xml(&body.comments).as_bytes(),
        );
    }
    zip.finish()
}

fn content_types(footer: bool, comments: bool) -> String {
    let mut s = String::from(DECLARATION);
    s.push_str(
        "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
         <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
         <Default Extension=\"xml\" ContentType=\"application/xml\"/>",
    );
    let mut part = |name: &str, content_type: &str| {
        s.push_str(&format!(
            "<Override PartName=\"/{name}\" ContentType=\"{content_type}\"/>"
        ));
    };
    part(parts::DOCUMENT, &format!("{WML}.document.main+xml"));
    part(parts::STYLES, &format!("{WML}.styles+xml"));
    part(parts::NUMBERING, &format!("{WML}.numbering+xml"));
    part(parts::SETTINGS, &format!("{WML}.settings+xml"));
    if footer {
        part(parts::FOOTER, &format!("{WML}.footer+xml"));
    }
    if comments {
        part(parts::COMMENTS, &format!("{WML}.comments+xml"));
        part(
            parts::COMMENTS_EXTENDED,
            &format!("{WML}.commentsExtended+xml"),
        );
    }
    part(
        parts::CORE,
        "application/vnd.openxmlformats-package.core-properties+xml",
    );
    part(
        parts::APP,
        "application/vnd.openxmlformats-officedocument.extended-properties+xml",
    );
    s.push_str("</Types>");
    s
}

const RELATIONSHIPS_OPEN: &str =
    "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">";

fn root_rels() -> String {
    format!(
        "{DECLARATION}{RELATIONSHIPS_OPEN}\
         <Relationship Id=\"rId1\" Type=\"{REL_BASE}/officeDocument\" Target=\"word/document.xml\"/>\
         <Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/>\
         <Relationship Id=\"rId3\" Type=\"{REL_BASE}/extended-properties\" Target=\"docProps/app.xml\"/>\
         </Relationships>"
    )
}

fn document_rels(footer: bool, comments: bool, links: &[String]) -> String {
    let mut s = format!(
        "{DECLARATION}{RELATIONSHIPS_OPEN}\
         <Relationship Id=\"{STYLES_REL}\" Type=\"{REL_BASE}/styles\" Target=\"styles.xml\"/>\
         <Relationship Id=\"{NUMBERING_REL}\" Type=\"{REL_BASE}/numbering\" Target=\"numbering.xml\"/>\
         <Relationship Id=\"{SETTINGS_REL}\" Type=\"{REL_BASE}/settings\" Target=\"settings.xml\"/>"
    );
    if footer {
        s.push_str(&format!(
            "<Relationship Id=\"{FOOTER_REL}\" Type=\"{REL_BASE}/footer\" Target=\"footer1.xml\"/>"
        ));
    }
    if comments {
        s.push_str(&format!(
            "<Relationship Id=\"{COMMENTS_REL}\" Type=\"{REL_BASE}/comments\" Target=\"comments.xml\"/>\
             <Relationship Id=\"{COMMENTS_EXTENDED_REL}\" Type=\"http://schemas.microsoft.com/office/2011/relationships/commentsExtended\" Target=\"commentsExtended.xml\"/>"
        ));
    }
    for (i, target) in links.iter().enumerate() {
        s.push_str(&format!(
            "<Relationship Id=\"rId{}\" Type=\"{REL_BASE}/hyperlink\" Target=\"{}\" TargetMode=\"External\"/>",
            FIRST_LINK_REL + i,
            attr(target),
        ));
    }
    s.push_str("</Relationships>");
    s
}

fn core_properties(title: &str, options: &DocxOptions) -> String {
    let author = text(options.author.trim());
    format!(
        "{DECLARATION}<cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" \
         xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:dcterms=\"http://purl.org/dc/terms/\" \
         xmlns:dcmitype=\"http://purl.org/dc/dcmitype/\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\
         <dc:title>{}</dc:title><dc:creator>{author}</dc:creator><cp:lastModifiedBy>{author}</cp:lastModifiedBy>\
         <dcterms:created xsi:type=\"dcterms:W3CDTF\">{}</dcterms:created>\
         <dcterms:modified xsi:type=\"dcterms:W3CDTF\">{}</dcterms:modified></cp:coreProperties>",
        text(title),
        iso_datetime(options.created.max(0)),
        iso_datetime(options.modified.max(0)),
    )
}

fn app_properties() -> String {
    format!(
        "{DECLARATION}<Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\">\
         <Application>The Writer</Application></Properties>"
    )
}

/// `word/settings.xml`.
///
/// `doNotUseHTMLParagraphAutoSpacing` makes the space after a paragraph
/// and the space before the next one add up, as they do in the editor.
/// Without it Word and LibreOffice use only the larger of the two, and a
/// heading sits a paragraph's space closer to the text above it.
fn settings_xml() -> String {
    format!(
        "{DECLARATION}<w:settings xmlns:w=\"{W_NS}\"><w:zoom w:percent=\"100\"/><w:defaultTabStop w:val=\"720\"/>\
         <w:characterSpacingControl w:val=\"doNotCompress\"/><w:compat>\
         <w:doNotUseHTMLParagraphAutoSpacing/>\
         <w:compatSetting w:name=\"compatibilityMode\" w:uri=\"http://schemas.microsoft.com/office/word\" w:val=\"15\"/>\
         </w:compat></w:settings>"
    )
}

fn footer_xml() -> String {
    format!(
        "{DECLARATION}<w:ftr xmlns:w=\"{W_NS}\" xmlns:r=\"{R_NS}\"><w:p><w:pPr><w:pStyle w:val=\"Footer\"/><w:jc w:val=\"center\"/></w:pPr>\
         <w:r><w:fldChar w:fldCharType=\"begin\"/></w:r><w:r><w:instrText xml:space=\"preserve\"> PAGE </w:instrText></w:r>\
         <w:r><w:fldChar w:fldCharType=\"separate\"/></w:r><w:r><w:rPr><w:noProof/></w:rPr><w:t>1</w:t></w:r>\
         <w:r><w:fldChar w:fldCharType=\"end\"/></w:r></w:p></w:ftr>"
    )
}

/// One Word comment: the opening comment of a thread or a reply.
pub(crate) struct Comment {
    pub id: u32,
    pub author: String,
    pub date: Timestamp,
    pub body: String,
    /// Id of the opening comment when this is a reply.
    pub parent: Option<u32>,
    pub resolved: bool,
}

impl Comment {
    /// The id of the comment's paragraph, which the threading part refers to.
    fn para_id(id: u32) -> String {
        format!("{:08X}", 0x1000_0000 + id)
    }
}

fn initials(author: &str) -> String {
    author
        .split_whitespace()
        .filter_map(|w| w.chars().find(|c| c.is_alphanumeric()))
        .take(3)
        .flat_map(char::to_uppercase)
        .collect()
}

fn comments_xml(comments: &[Comment]) -> String {
    let mut s = format!(
        "{DECLARATION}<w:comments xmlns:w=\"{W_NS}\" xmlns:w14=\"{W14_NS}\" xmlns:w15=\"{W15_NS}\" xmlns:mc=\"{MC_NS}\" mc:Ignorable=\"w14 w15\">"
    );
    for c in comments {
        // A comment without a usable time (zero or before 1970) gets no
        // date at all rather than a wrong one.
        let date = if c.date > 0 {
            format!(" w:date=\"{}\"", iso_datetime(c.date))
        } else {
            String::new()
        };
        s.push_str(&format!(
            "<w:comment w:id=\"{}\" w:author=\"{}\"{date} w:initials=\"{}\"><w:p w14:paraId=\"{}\" w14:textId=\"77777777\">\
             <w:pPr><w:pStyle w:val=\"CommentText\"/></w:pPr><w:r><w:rPr><w:rStyle w:val=\"CommentReference\"/></w:rPr><w:annotationRef/></w:r>",
            c.id,
            attr(&c.author),
            attr(&initials(&c.author)),
            Comment::para_id(c.id),
        ));
        for (i, line) in c.body.split('\n').enumerate() {
            s.push_str("<w:r>");
            if i > 0 {
                s.push_str("<w:br/>");
            }
            s.push_str(&format!(
                "<w:t xml:space=\"preserve\">{}</w:t></w:r>",
                text(&line.replace('\t', " "))
            ));
        }
        s.push_str("</w:p></w:comment>");
    }
    s.push_str("</w:comments>");
    s
}

fn comments_extended_xml(comments: &[Comment]) -> String {
    let mut s = format!(
        "{DECLARATION}<w15:commentsEx xmlns:w15=\"{W15_NS}\" xmlns:mc=\"{MC_NS}\" mc:Ignorable=\"w15\">"
    );
    for c in comments {
        let parent = c
            .parent
            .map(|p| format!(" w15:paraIdParent=\"{}\"", Comment::para_id(p)))
            .unwrap_or_default();
        s.push_str(&format!(
            "<w15:commentEx w15:paraId=\"{}\"{parent} w15:done=\"{}\"/>",
            Comment::para_id(c.id),
            u8::from(c.resolved),
        ));
    }
    s.push_str("</w15:commentsEx>");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics() {
        let m = Metrics::new(&DocxOptions::default());
        assert_eq!(m.half_points(17.0), 34);
        assert_eq!(m.twips(12.0), 240);
        // 17 pt Charter at 1.6 is a 27 pt line; Charter's own line is
        // 1.2202 times its size.
        assert_eq!(m.line("Charter", 17.0, 1.6), 312);
        assert_eq!(m.line(" charter ", 17.0, 1.6), 312);
        // An unknown font is taken as 1.2: 27 / (17 * 1.2) * 240.
        assert_eq!(m.line("No Such Font", 17.0, 1.6), 318);
        assert_eq!(m.line("No Such Font", 10.0, 1.2), 240);
        assert_eq!(m.line("Avenir Next", 17.0, 1.6), 279);
        // A size that is not a number counts as 12 pt.
        assert_eq!(m.line("Charter", f32::NAN, 1.6), 311);
        assert_eq!(m.line("Charter", 17.0, 100.0), 1200);
        // Headings never go above 1.3: 34 pt gives a 44 pt line.
        let t = writer_core::Typography::default();
        assert_eq!(m.heading_line(&t, 34.0), 255);
        assert_eq!(
            (
                m.indent(0),
                m.indent(2),
                m.marker_gutter(),
                m.quote_gutter()
            ),
            (0, 960, 520, 400)
        );
        assert_eq!((m.number_gap(), m.bullet_offset()), (140, 260));
        assert_eq!(m.code_padding(), (14, 10));
        let t = writer_core::Typography::default();
        assert_eq!(m.base_line(&t), (t.base_size * t.line_height).round());
        assert_eq!(
            m.list_gap(&t),
            m.twips((t.paragraph_spacing.round() * 0.33).round())
        );
        assert_eq!(page_twips(595.276), 11906);
        assert_eq!(page_twips(841.89), 16838);
        let scaled = Metrics::new(&DocxOptions {
            text_scale: 12.0 / 17.0,
            ..Default::default()
        });
        assert_eq!(scaled.half_points(17.0), 24);
        let bad = Metrics::new(&DocxOptions {
            text_scale: f32::NAN,
            code_font: "  ".into(),
            ..Default::default()
        });
        assert_eq!(bad.half_points(17.0), 34);
        assert_eq!(bad.code_font, "Menlo");
    }

    #[test]
    fn author_initials() {
        assert_eq!(initials("Ada Lovelace"), "AL");
        assert_eq!(initials("  émile  zola jr third"), "ÉZJ");
        assert_eq!(initials(""), "");
        assert_eq!(initials("\"quoted\" <x>"), "QX");
    }
}
