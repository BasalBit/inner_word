//! Importers: Markdown and plain text to a [`Document`] or a [`Fragment`].
//!
//! The inverse of the exporters, for everything that writes text into the
//! library without typing it: an agent creating a document from Markdown,
//! replacing its content, appending a section.
//!
//! - [`from_markdown`] reads CommonMark with the GitHub extensions the
//!   exporter writes, and the exporter's own conventions for what Markdown
//!   has no syntax for. See [`markdown`] for the mapping.
//! - [`from_plain_text`] reads paragraphs separated by blank lines.
//!
//! Both return an [`Imported`]: the document, the title and front matter the
//! text named, and [`ImportWarning`]s for everything the document model
//! could not hold as written (a table, an image, raw HTML). Nothing is
//! dropped silently: such constructs are kept as readable text and
//! reported, so the caller can tell whoever wrote the text.
//!
//! No importer can fail: any string is some Markdown.
//!
//! ```
//! use writer_core::BlockKind;
//! use writer_export::{MarkdownImportOptions, from_markdown, to_markdown};
//!
//! let markdown = "# Plan\n\nShip **soon**.\n\n1. Write\n   - [x] Outline\n2. Edit\n";
//! let imported = from_markdown(markdown, &MarkdownImportOptions::default());
//! assert!(imported.warnings.is_empty());
//! assert_eq!(imported.title.as_deref(), Some("Plan"));
//!
//! let doc = &imported.document;
//! assert_eq!(doc.blocks()[0].kind(), &BlockKind::heading(1));
//! assert!(doc.blocks()[1].attrs_at(5).unwrap().bold);
//! assert_eq!(doc.blocks()[3].kind(), &BlockKind::TodoItem { checked: true });
//! assert_eq!(doc.blocks()[3].indent(), 1);
//!
//! // Exporting gives the same Markdown back.
//! assert_eq!(to_markdown(&doc.into(), &Default::default()), markdown);
//!
//! // The same blocks, to insert into a document that exists
//! // (`EditorState::insert_blocks_at` or `EditorState::paste`).
//! assert_eq!(imported.fragment().blocks().len(), 5);
//! ```

mod html;
pub mod markdown;
pub mod plain;

use std::fmt;

use writer_core::{Document, Fragment};

pub use markdown::{
    MarkdownFrontMatter, MarkdownImportOptions, MarkdownLeadingHeading, MarkdownSoftBreaks,
    from_markdown,
};
pub use plain::{PlainTextImportOptions, PlainTextLines, from_plain_text};

/// The result of an import.
#[derive(Clone, Debug, PartialEq)]
pub struct Imported {
    /// The imported document, with the settings the options named. A text
    /// with nothing in it gives the empty document (one empty paragraph).
    pub document: Document,
    /// The title the text names, if it names one: for Markdown the text of
    /// a level-1 heading that opens it, or the `title` of its front matter
    /// (see [`MarkdownLeadingHeading`]). Plain text names none; use
    /// [`Document::title`] for a title derived from the content.
    pub title: Option<String>,
    /// The front matter of a Markdown text, without its delimiter lines,
    /// when the text opened with one and the options did not ask for it to
    /// be read as ordinary Markdown.
    pub front_matter: Option<String>,
    /// What could not be imported as written, in the order of the text.
    pub warnings: Vec<ImportWarning>,
    /// Whether the text produced any block at all.
    pub(crate) has_content: bool,
}

impl Imported {
    /// The imported blocks as a [`Fragment`], to insert into an existing
    /// document. Empty (no blocks) when the text held nothing, where
    /// [`Imported::document`] has its one empty paragraph.
    ///
    /// A fragment of a single text block is inline to
    /// [`EditorState::paste`](writer_core::EditorState::paste): its text
    /// joins the block the caret is in. Use
    /// [`EditorState::insert_blocks_at`](writer_core::EditorState::insert_blocks_at)
    /// to insert whole blocks whatever their number.
    pub fn fragment(&self) -> Fragment {
        if self.has_content {
            self.document.to_fragment()
        } else {
            Fragment::default()
        }
    }

    /// Whether the text produced no block: it was empty, or held only
    /// things that leave nothing behind (white space, removed front
    /// matter, HTML comments).
    pub fn is_empty(&self) -> bool {
        !self.has_content
    }
}

/// What kind of thing an [`ImportWarning`] is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum ImportWarningKind {
    /// A table. Kept as an aligned text table in a code block.
    Table,
    /// An image. Kept as a link to the image, with its alternative text
    /// (or its address) as the link text.
    Image,
    /// A footnote reference or definition. Kept as literal text: `[^1]` in
    /// the text and a paragraph `[^1]: …` where the note was defined.
    Footnote,
    /// A block of raw HTML. Kept as a code block with the language `html`.
    HtmlBlock,
    /// An inline HTML tag the document has no formatting for (anything but
    /// the tags listed in [`markdown`]). Kept as literal
    /// text.
    InlineHtml,
    /// An HTML comment. Left out, as every renderer does.
    HtmlComment,
    /// Front matter at the start of the text. Removed from the text and
    /// returned in [`Imported::front_matter`].
    FrontMatter,
    /// A numbered list that starts at a number other than 1. The document
    /// numbers every list from 1.
    ListStart,
    /// Lists nested deeper than the document's nine indent levels. The
    /// deeper items are kept at the deepest level.
    NestingTooDeep,
    /// A list, heading, code block, divider or table inside a block quote.
    /// Kept as what it is, without the quote.
    QuotedBlock,
    /// A block quote inside a block quote. Kept as one level of quote.
    NestedQuote,
    /// The title of a link (`[text](url "title")`). Left out.
    LinkTitle,
}

impl ImportWarningKind {
    /// A short stable name for machine-readable output: `"table"`,
    /// `"image"`, `"footnote"`, `"html_block"`, `"inline_html"`,
    /// `"html_comment"`, `"front_matter"`, `"list_start"`,
    /// `"nesting_too_deep"`, `"quoted_block"`, `"nested_quote"`,
    /// `"link_title"`.
    pub fn code(self) -> &'static str {
        match self {
            ImportWarningKind::Table => "table",
            ImportWarningKind::Image => "image",
            ImportWarningKind::Footnote => "footnote",
            ImportWarningKind::HtmlBlock => "html_block",
            ImportWarningKind::InlineHtml => "inline_html",
            ImportWarningKind::HtmlComment => "html_comment",
            ImportWarningKind::FrontMatter => "front_matter",
            ImportWarningKind::ListStart => "list_start",
            ImportWarningKind::NestingTooDeep => "nesting_too_deep",
            ImportWarningKind::QuotedBlock => "quoted_block",
            ImportWarningKind::NestedQuote => "nested_quote",
            ImportWarningKind::LinkTitle => "link_title",
        }
    }

    /// What happened, as a sentence without a full stop: "a table was kept
    /// as text in a code block".
    pub fn description(self) -> &'static str {
        match self {
            ImportWarningKind::Table => {
                "a table was kept as text in a code block (documents have no tables)"
            }
            ImportWarningKind::Image => {
                "an image was kept as a link to it (documents have no images)"
            }
            ImportWarningKind::Footnote => {
                "a footnote was kept as literal text (documents have no footnotes)"
            }
            ImportWarningKind::HtmlBlock => "a block of raw HTML was kept as a code block",
            ImportWarningKind::InlineHtml => {
                "an HTML tag without a matching format was kept as literal text"
            }
            ImportWarningKind::HtmlComment => "an HTML comment was left out",
            ImportWarningKind::FrontMatter => "the front matter was left out of the text",
            ImportWarningKind::ListStart => {
                "a numbered list that starts at another number is numbered from 1"
            }
            ImportWarningKind::NestingTooDeep => {
                "list items nested deeper than nine levels were kept at the ninth"
            }
            ImportWarningKind::QuotedBlock => {
                "a list, heading, code block or divider inside a quote was kept without the quote"
            }
            ImportWarningKind::NestedQuote => "a quote inside a quote was kept as one quote",
            ImportWarningKind::LinkTitle => "the title of a link was left out",
        }
    }

    /// The same for several: "3 tables were kept as text in code blocks".
    fn description_of(self, count: usize) -> String {
        if count == 1 {
            return self.description().to_string();
        }
        let rest = match self {
            ImportWarningKind::Table => {
                "tables were kept as text in code blocks (documents have no tables)"
            }
            ImportWarningKind::Image => {
                "images were kept as links to them (documents have no images)"
            }
            ImportWarningKind::Footnote => {
                "footnote marks and notes were kept as literal text (documents have no footnotes)"
            }
            ImportWarningKind::HtmlBlock => "blocks of raw HTML were kept as code blocks",
            ImportWarningKind::InlineHtml => {
                "HTML tags without a matching format were kept as literal text"
            }
            ImportWarningKind::HtmlComment => "HTML comments were left out",
            ImportWarningKind::FrontMatter => "front matter blocks were left out of the text",
            ImportWarningKind::ListStart => {
                "numbered lists that start at another number are numbered from 1"
            }
            ImportWarningKind::NestingTooDeep => {
                "list items nested deeper than nine levels were kept at the ninth"
            }
            ImportWarningKind::QuotedBlock => {
                "lists, headings, code blocks or dividers inside quotes were kept without the quote"
            }
            ImportWarningKind::NestedQuote => "quotes inside quotes were kept as one quote",
            ImportWarningKind::LinkTitle => "link titles were left out",
        };
        format!("{count} {rest}")
    }
}

/// One thing an importer could not bring over as it was written.
///
/// `Display` gives a sentence with the line: `line 12: a table was kept as
/// text in a code block (documents have no tables)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportWarning {
    /// What it is about.
    pub kind: ImportWarningKind,
    /// The line of the imported text it is on, counted from 1.
    pub line: usize,
}

impl ImportWarning {
    /// What happened, as a sentence without the line and without a full
    /// stop ([`ImportWarningKind::description`]).
    pub fn message(&self) -> &'static str {
        self.kind.description()
    }
}

impl fmt::Display for ImportWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.kind.description())
    }
}

/// How many line numbers [`summarize_warnings`] lists per kind before it
/// says "and N more".
const SUMMARY_LINES: usize = 8;

/// The warnings as a few sentences, one per kind in the order each kind
/// first appears: `"2 images were kept as links to them (documents have no
/// images): lines 4 and 9"`. For a caller that reports to a person or an
/// agent and does not want one line per occurrence.
///
/// ```
/// use writer_export::{MarkdownImportOptions, from_markdown, summarize_warnings};
///
/// let imported = from_markdown(
///     "![a](a.png)\n\ntext\n\n![b](b.png)\n",
///     &MarkdownImportOptions::default(),
/// );
/// assert_eq!(
///     summarize_warnings(&imported.warnings),
///     ["2 images were kept as links to them (documents have no images): lines 1 and 5"]
/// );
/// ```
pub fn summarize_warnings(warnings: &[ImportWarning]) -> Vec<String> {
    let mut kinds: Vec<(ImportWarningKind, Vec<usize>)> = Vec::new();
    for w in warnings {
        match kinds.iter_mut().find(|(k, _)| *k == w.kind) {
            Some((_, lines)) => lines.push(w.line),
            None => kinds.push((w.kind, vec![w.line])),
        }
    }
    kinds
        .into_iter()
        .map(|(kind, lines)| {
            let count = lines.len();
            let mut distinct = lines;
            distinct.dedup();
            let shown: Vec<String> = distinct
                .iter()
                .take(SUMMARY_LINES)
                .map(usize::to_string)
                .collect();
            let hidden = distinct.len() - shown.len();
            let list = match (shown.len(), hidden) {
                (1, 0) => format!("line {}", shown[0]),
                (n, 0) => format!("lines {} and {}", shown[..n - 1].join(", "), shown[n - 1]),
                _ => format!("lines {} and {hidden} more", shown.join(", ")),
            };
            format!("{}: {list}", kind.description_of(count))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(kind: ImportWarningKind, line: usize) -> ImportWarning {
        ImportWarning { kind, line }
    }

    #[test]
    fn warnings_read_as_sentences() {
        let one = w(ImportWarningKind::Table, 12);
        assert_eq!(
            one.to_string(),
            "line 12: a table was kept as text in a code block (documents have no tables)"
        );
        assert_eq!(one.message(), ImportWarningKind::Table.description());
        assert_eq!(ImportWarningKind::HtmlBlock.code(), "html_block");
    }

    #[test]
    fn summaries_group_by_kind() {
        assert!(summarize_warnings(&[]).is_empty());
        let warnings = [
            w(ImportWarningKind::Image, 3),
            w(ImportWarningKind::Table, 5),
            w(ImportWarningKind::Image, 9),
            w(ImportWarningKind::Image, 9),
        ];
        assert_eq!(
            summarize_warnings(&warnings),
            [
                "3 images were kept as links to them (documents have no images): lines 3 and 9",
                "a table was kept as text in a code block (documents have no tables): line 5",
            ]
        );
        let many: Vec<ImportWarning> = (1..=20)
            .map(|i| w(ImportWarningKind::HtmlComment, i))
            .collect();
        assert_eq!(
            summarize_warnings(&many),
            ["20 HTML comments were left out: lines 1, 2, 3, 4, 5, 6, 7, 8 and 12 more"]
        );
    }
}
