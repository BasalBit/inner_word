//! Markdown, plain-text and DOCX exporters for The Writer, and the
//! importers that read Markdown and plain text back.
//!
//! Pure Rust over [`writer_core`]: no UI, no file dialogs, no clock. The app
//! picks a destination and writes the bytes this crate returns.
//!
//! # What gets exported
//!
//! Every exporter takes an [`ExportSource`]: one document, or an ordered
//! list of titled documents exported as one file (a folder, for example the
//! chapters of a book). In a collection each document starts on a new page
//! in DOCX and is separated by a rule in Markdown and plain text; a
//! document's title is written as a heading when the document does not
//! already open with it ([`PartTitles`]).
//!
//! # Formats
//!
//! - [`to_markdown`]: CommonMark with the GitHub extensions (strikethrough,
//!   task lists, footnotes for comments). See [`MarkdownOptions`].
//! - [`to_plain_text`]: text for places that do not render Markdown, such as
//!   social posts. See [`PlainTextOptions`], and [`social`] for the
//!   character counter with per-network limits.
//! - [`to_docx`]: an Office Open XML package for Word, Pages and Google
//!   Docs. See [`DocxOptions`].
//!
//! [`export`] dispatches on an [`ExportFormat`] and always returns bytes;
//! [`suggested_file_name`] turns a title into a safe file name.
//!
//! # Importing
//!
//! - [`from_markdown`]: Markdown to a document, the inverse of
//!   [`to_markdown`]. This is how anything that does not type writes into
//!   the library (an agent creating or rewriting a document). See
//!   [`MarkdownImportOptions`] and the [`import`] module.
//! - [`from_plain_text`]: paragraphs separated by blank lines.
//!
//! Both return an [`Imported`]: the document, a [`Fragment`] of the same
//! blocks to insert into a document that exists, the title the text
//! names, and an [`ImportWarning`] for everything the document cannot hold
//! as written (tables, images, raw HTML), which is kept as readable text.
//!
//! [`Fragment`]: writer_core::Fragment
//!
//! # Example
//!
//! ```
//! use writer_core::{Block, BlockKind, Document};
//! use writer_export::{ExportFormat, ExportOptions, ExportSource, export, suggested_file_name};
//!
//! let doc = Document::from_blocks(vec![
//!     Block::with_text(BlockKind::heading(1), "Notes: draft 2"),
//!     Block::paragraph("Hello, world."),
//! ]);
//! let source = ExportSource::single("Notes: draft 2", &doc);
//!
//! let md = writer_export::to_markdown(&source, &Default::default());
//! assert_eq!(md, "# Notes: draft 2\n\nHello, world.\n");
//!
//! let options = ExportOptions::default();
//! let bytes = export(&source, ExportFormat::Docx, &options);
//! assert_eq!(&bytes[..2], b"PK");
//! assert_eq!(suggested_file_name(source.title(), ExportFormat::Docx), "Notes - draft 2.docx");
//! ```

#![warn(missing_docs)]

mod common;
pub mod docx;
pub mod import;
pub mod markdown;
pub mod plain;
pub mod social;

use writer_core::Document;

pub use docx::{DocxOptions, to_docx};
pub use import::{
    ImportWarning, ImportWarningKind, Imported, MarkdownFrontMatter, MarkdownImportOptions,
    MarkdownLeadingHeading, MarkdownSoftBreaks, PlainTextImportOptions, PlainTextLines,
    from_markdown, from_plain_text, summarize_warnings,
};
pub use markdown::{
    MarkdownComments, MarkdownHighlight, MarkdownOptions, MarkdownPageBreak, to_markdown,
};
pub use plain::{PlainTextLinks, PlainTextOptions, to_plain_text};
pub use social::{SocialCount, SocialNetwork, social_length};

/// The title used when a document or collection has none.
pub const UNTITLED: &str = "Untitled";

/// An export file format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ExportFormat {
    /// Markdown (`.md`): CommonMark with the GitHub extensions.
    Markdown,
    /// Plain text (`.txt`), for example for social posts.
    PlainText,
    /// Word document (`.docx`), Office Open XML.
    Docx,
}

impl ExportFormat {
    /// Every format, in menu order.
    pub const ALL: [ExportFormat; 3] = [
        ExportFormat::Markdown,
        ExportFormat::Docx,
        ExportFormat::PlainText,
    ];

    /// The file extension without the dot: `"md"`, `"txt"` or `"docx"`.
    pub fn extension(self) -> &'static str {
        match self {
            ExportFormat::Markdown => "md",
            ExportFormat::PlainText => "txt",
            ExportFormat::Docx => "docx",
        }
    }

    /// The name shown in the export menu.
    pub fn display_name(self) -> &'static str {
        match self {
            ExportFormat::Markdown => "Markdown",
            ExportFormat::PlainText => "Plain text",
            ExportFormat::Docx => "Word document",
        }
    }

    /// The MIME type of the exported file.
    pub fn mime_type(self) -> &'static str {
        match self {
            ExportFormat::Markdown => "text/markdown",
            ExportFormat::PlainText => "text/plain",
            ExportFormat::Docx => {
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
            }
        }
    }

    /// Whether the exported bytes are UTF-8 text (Markdown, plain text) and
    /// can also go to the clipboard as a string.
    pub fn is_text(self) -> bool {
        !matches!(self, ExportFormat::Docx)
    }

    /// The format with the given extension (case-insensitive, with or
    /// without the dot; `"markdown"` and `"text"` are accepted too).
    pub fn from_extension(extension: &str) -> Option<ExportFormat> {
        let e = extension
            .trim()
            .trim_start_matches('.')
            .to_ascii_lowercase();
        match e.as_str() {
            "md" | "markdown" => Some(ExportFormat::Markdown),
            "txt" | "text" => Some(ExportFormat::PlainText),
            "docx" => Some(ExportFormat::Docx),
            _ => None,
        }
    }
}

/// One titled document inside an [`ExportSource`].
#[derive(Clone, Debug)]
pub struct ExportPart<'a> {
    /// The document's title as the library shows it.
    pub title: String,
    /// The document.
    pub document: &'a Document,
}

/// What to export: one document, or an ordered list of titled documents
/// written to a single file.
#[derive(Clone, Debug)]
pub struct ExportSource<'a> {
    title: String,
    parts: Vec<ExportPart<'a>>,
    collection: bool,
}

impl<'a> ExportSource<'a> {
    /// A single document. `title` names the file and the DOCX title
    /// property; it is not written into the text.
    pub fn single(title: impl Into<String>, document: &'a Document) -> ExportSource<'a> {
        let title = clean_title(&title.into());
        ExportSource {
            parts: vec![ExportPart {
                title: title.clone(),
                document,
            }],
            title,
            collection: false,
        }
    }

    /// Several documents exported as one file, in the given order, for
    /// example the documents of a folder. `title` is the name of the whole
    /// (the folder name); each part carries its own title.
    pub fn collection<S: Into<String>>(
        title: impl Into<String>,
        parts: impl IntoIterator<Item = (S, &'a Document)>,
    ) -> ExportSource<'a> {
        ExportSource {
            title: clean_title(&title.into()),
            parts: parts
                .into_iter()
                .map(|(t, document)| ExportPart {
                    title: clean_title(&t.into()),
                    document,
                })
                .collect(),
            collection: true,
        }
    }

    /// The title of the whole export.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The documents, in order. Empty only for an empty collection.
    pub fn parts(&self) -> &[ExportPart<'a>] {
        &self.parts
    }

    /// Whether this was built with [`ExportSource::collection`].
    pub fn is_collection(&self) -> bool {
        self.collection
    }
}

/// A single document titled with [`Document::title`] (or [`UNTITLED`]).
impl<'a> From<&'a Document> for ExportSource<'a> {
    fn from(document: &'a Document) -> ExportSource<'a> {
        ExportSource::single(document.title().unwrap_or_default(), document)
    }
}

fn clean_title(title: &str) -> String {
    let t = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.is_empty() {
        UNTITLED.to_string()
    } else {
        t
    }
}

/// When the title of a document in a collection is written as a level-1
/// heading above its text. Single documents never get one: their text is
/// exported as it is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PartTitles {
    /// Never write titles.
    Never,
    /// Write the title unless the document already shows it: one of its
    /// headings, or its first line, starts with the title. This is the
    /// default, and the right choice when titles are derived from content.
    #[default]
    WhenMissing,
    /// Always write the title.
    Always,
}

/// Options for every format, for [`export`].
#[derive(Clone, Debug, Default)]
pub struct ExportOptions {
    /// Markdown options.
    pub markdown: MarkdownOptions,
    /// Plain-text options.
    pub plain_text: PlainTextOptions,
    /// DOCX options. Set [`DocxOptions::created`] and
    /// [`DocxOptions::modified`]; this crate never reads the clock.
    pub docx: DocxOptions,
}

/// Exports `source` in `format` and returns the file's bytes (UTF-8 text for
/// Markdown and plain text, a ZIP package for DOCX).
pub fn export(source: &ExportSource<'_>, format: ExportFormat, options: &ExportOptions) -> Vec<u8> {
    match format {
        ExportFormat::Markdown => to_markdown(source, &options.markdown).into_bytes(),
        ExportFormat::PlainText => to_plain_text(source, &options.plain_text).into_bytes(),
        ExportFormat::Docx => to_docx(source, &options.docx),
    }
}

/// Exports `source` as text. `None` for [`ExportFormat::Docx`], which is
/// binary.
pub fn export_text(
    source: &ExportSource<'_>,
    format: ExportFormat,
    options: &ExportOptions,
) -> Option<String> {
    match format {
        ExportFormat::Markdown => Some(to_markdown(source, &options.markdown)),
        ExportFormat::PlainText => Some(to_plain_text(source, &options.plain_text)),
        ExportFormat::Docx => None,
    }
}

/// The longest file-name stem [`suggested_file_name`] produces, in
/// characters.
pub const MAX_FILE_STEM_CHARS: usize = 100;

/// A file name for `title` in `format`, safe on macOS, Windows and Linux
/// file systems: `"Chapter 1: The End?"` becomes `"Chapter 1 - The End.md"`.
///
/// Path separators and the characters Windows forbids are replaced, control
/// characters are removed, whitespace is collapsed, leading dots (hidden
/// files) and trailing dots and spaces are dropped, and the stem is cut to
/// [`MAX_FILE_STEM_CHARS`] characters on a grapheme boundary. An empty
/// result becomes [`UNTITLED`].
pub fn suggested_file_name(title: &str, format: ExportFormat) -> String {
    format!("{}.{}", file_stem(title), format.extension())
}

fn file_stem(title: &str) -> String {
    use unicode_segmentation::UnicodeSegmentation;

    let mut s = String::with_capacity(title.len());
    for c in title.chars() {
        match c {
            ':' => s.push_str(" - "),
            '/' | '\\' | '|' => s.push('-'),
            '*' | '?' | '"' | '<' | '>' => {}
            c if c.is_control() => s.push(' '),
            // Bidi controls and other invisible format characters make file
            // names misleading.
            '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' => {}
            '\u{feff}' => {}
            c => s.push(c),
        }
    }
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let s = s.trim_matches(['.', ' ', '-']);
    let mut out = String::new();
    let mut chars = 0;
    for g in s.graphemes(true) {
        let n = g.chars().count();
        if chars + n > MAX_FILE_STEM_CHARS {
            break;
        }
        chars += n;
        out.push_str(g);
    }
    let out = out.trim_end_matches(['.', ' ', '-']);
    // Device names Windows reserves, with or without an extension.
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if out.is_empty() {
        UNTITLED.to_string()
    } else if RESERVED.iter().any(|r| r.eq_ignore_ascii_case(out)) {
        format!("{out} document")
    } else {
        out.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use writer_core::{Block, BlockKind};

    #[test]
    fn formats() {
        assert_eq!(ExportFormat::Markdown.extension(), "md");
        assert_eq!(ExportFormat::PlainText.extension(), "txt");
        assert_eq!(ExportFormat::Docx.extension(), "docx");
        for f in ExportFormat::ALL {
            assert_eq!(ExportFormat::from_extension(f.extension()), Some(f));
            assert!(!f.display_name().is_empty());
            assert!(f.mime_type().contains('/'));
        }
        assert_eq!(
            ExportFormat::from_extension(".MD"),
            Some(ExportFormat::Markdown)
        );
        assert_eq!(ExportFormat::from_extension("pdf"), None);
        assert!(ExportFormat::Markdown.is_text());
        assert!(!ExportFormat::Docx.is_text());
    }

    #[test]
    fn file_names() {
        let f = |t: &str| suggested_file_name(t, ExportFormat::Markdown);
        assert_eq!(f("Chapter 1: The End?"), "Chapter 1 - The End.md");
        assert_eq!(f("a/b\\c|d"), "a-b-c-d.md");
        assert_eq!(f("  spaced \t out\n"), "spaced out.md");
        assert_eq!(f(""), "Untitled.md");
        assert_eq!(f("???"), "Untitled.md");
        assert_eq!(f("..hidden."), "hidden.md");
        assert_eq!(f("con"), "con document.md");
        assert_eq!(f("Émile, 東京, שלום"), "Émile, 東京, שלום.md");
        assert_eq!(f("a\u{0}b\u{7}c"), "a b c.md");
        assert_eq!(f("evil\u{202e}txt.exe"), "eviltxt.exe.md");
        let long = "word ".repeat(100);
        let name = f(&long);
        assert!(name.chars().count() <= MAX_FILE_STEM_CHARS + 3);
        assert!(name.ends_with("word.md"));
        // A family emoji is one grapheme and is never cut in half.
        let fam = "👨‍👩‍👧‍👦".repeat(40);
        let name = suggested_file_name(&fam, ExportFormat::Docx);
        let stem = name.strip_suffix(".docx").unwrap();
        assert_eq!(stem.chars().count() % 7, 0);
        assert!(!stem.is_empty());
    }

    #[test]
    fn sources() {
        let doc = Document::from_blocks(vec![Block::with_text(BlockKind::heading(1), "Title")]);
        let s = ExportSource::from(&doc);
        assert_eq!(s.title(), "Title");
        assert!(!s.is_collection());
        assert_eq!(s.parts().len(), 1);
        let empty = Document::new();
        assert_eq!(ExportSource::from(&empty).title(), UNTITLED);
        let c = ExportSource::collection("Book", [("  One ", &doc), ("", &empty)]);
        assert!(c.is_collection());
        assert_eq!(c.parts()[0].title, "One");
        assert_eq!(c.parts()[1].title, UNTITLED);
    }

    #[test]
    fn export_dispatch() {
        let doc = Document::from_blocks(vec![Block::paragraph("Hi")]);
        let s = ExportSource::single("Hi", &doc);
        let o = ExportOptions::default();
        assert_eq!(export(&s, ExportFormat::Markdown, &o), b"Hi\n");
        assert_eq!(export(&s, ExportFormat::PlainText, &o), b"Hi\n");
        assert_eq!(&export(&s, ExportFormat::Docx, &o)[..4], b"PK\x03\x04");
        assert_eq!(
            export_text(&s, ExportFormat::Markdown, &o).as_deref(),
            Some("Hi\n")
        );
        assert_eq!(export_text(&s, ExportFormat::Docx, &o), None);
    }
}
