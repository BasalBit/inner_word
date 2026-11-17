//! Markdown import: CommonMark with the GitHub extensions to a
//! [`Document`], the inverse of [`to_markdown`](crate::to_markdown).
//!
//! # What maps directly
//!
//! - Headings 1 to 6 (`#` and underlined), paragraphs.
//! - Bold, italic, strikethrough, inline code, links (inline, reference
//!   and `<autolinks>`), in any nesting.
//! - Bullet, numbered and task lists. Nesting becomes the indent level of
//!   the items; a paragraph, quote or code block inside an item is a block
//!   one level deeper than the item, which is how the document (and the
//!   exporter) say "belongs to the item above".
//! - Block quotes: every paragraph of a quote is a quote block.
//! - Fenced and indented code blocks; the first word after the fence is
//!   the language.
//! - Thematic breaks are dividers.
//! - A hard line break (a backslash or two spaces before the line end) is
//!   a line break inside the block. A plain line end inside a paragraph is
//!   a space, as in every Markdown renderer
//!   ([`MarkdownSoftBreaks`] changes that).
//!
//! # The exporter's conventions, read back
//!
//! - Underline `<u>`, highlight `<mark>`, and the tags the exporter falls
//!   back to where Markdown delimiters cannot pair: `<strong>`, `<em>`,
//!   `<del>`, `<code>`. Their common synonyms are read too (`<b>`, `<i>`,
//!   `<s>`, `<ins>`, `<kbd>`), as is `<br>` (a line break) and
//!   `<a href="…">`.
//! - `<span style="color:#rrggbb">` and `<span style="font-family:'Name'">`
//!   (text colour and font), and in a `style` also `background-color`
//!   (highlight), `font-weight: bold`, `font-style: italic` and
//!   `text-decoration: underline` or `line-through`.
//! - `<!-- pagebreak -->` and `<div style="page-break-after: always"></div>`
//!   on a line of their own are page breaks.
//! - `<div align="center">` … `</div>` around blocks (blank lines inside,
//!   as the exporter writes it) sets their alignment.
//! - `![description](image:<id>)` on a line of its own is an image, when
//!   the caller can say which one (see "Images" below).
//! - `==text==` is a highlight when
//!   [`MarkdownImportOptions::highlight_equals`] is on.
//! - `<!-- -->`, the exporter's filler in an empty to-do item, leaves
//!   nothing.
//!
//! Comments are not read back: the exporter can write them as HTML comments
//! (left out here, with a warning) or as footnotes (kept as literal text,
//! like any footnote). Text size is not written by the exporter either.
//!
//! # What the document cannot hold
//!
//! Each of these is kept as readable text and reported as an
//! [`ImportWarning`]:
//!
//! - A table becomes a code block holding the table as aligned text.
//! - An image that nobody resolved (below) becomes a link to the image;
//!   its text is the alternative text, or the address when there is none.
//! - A footnote reference stays `[^1]` in the text, and the note becomes a
//!   paragraph `[^1]: …` where it was defined.
//! - A block of raw HTML becomes a code block with the language `html`. An
//!   inline tag that is not listed above stays in the text as it was
//!   typed. HTML comments are left out.
//! - A numbered list is numbered from 1 whatever its first number.
//! - Lists inside a quote, quotes inside a quote and nesting beyond nine
//!   levels are flattened as [`ImportWarningKind`] describes.
//!
//! # Images
//!
//! A document holds an image as a block of its own that names stored
//! bytes. Markdown names an address. [`from_markdown_with_images`] asks an
//! [`ImageResolver`] what each address is, once for every image and in the
//! order of the text; [`from_markdown`] asks nobody, and every image is a
//! link. This module stores nothing and reads no file: a resolver does
//! what its caller allows ([`markdown_image_sources`] lists the addresses
//! beforehand, for a caller that has files to prepare).
//!
//! - An image the resolver says is stored, **alone in its paragraph**,
//!   becomes an image block. Its description is the alternative text (or,
//!   when there is none, the title, or what the stored image had), its
//!   width what the resolver says or its original size on the page of
//!   [`MarkdownImportOptions::settings`], its alignment what a
//!   `<div align="…">` around it says, or else what the resolver says.
//! - **Inside a line of text** it is put on a line of its own: the text
//!   before it, the image and the text after it are three blocks
//!   ([`ImportWarningKind::ImageInText`]). In a bullet or numbered item
//!   that holds nothing else, the image takes the item's place.
//! - A link around it is left out ([`ImportWarningKind::ImageLink`]).
//! - In a heading, a quote or a table there is no place for an image, and
//!   it stays a link ([`ImportWarningKind::Image`]). So does an image at an
//!   address on the network, one whose file was not found
//!   ([`ImportWarningKind::ImageNotFound`]) and one whose file is no image
//!   the library holds ([`ImportWarningKind::ImageUnsupported`]; the
//!   reasons are in [`Imported::image_problems`]).
//!
//! # What is put into the document's own form
//!
//! - Two lists of one sort in a row, or two quotes in a row, are one list
//!   or one quote to the document. Where Markdown has two, an empty
//!   paragraph goes between them, which is how one keeps them apart in
//!   the app.
//! - A bullet or numbered item without text that is followed by a
//!   paragraph of its own is an item with that text (`-` and, on the next
//!   line, indented text).
//! - White space at the ends of a line is dropped, as a renderer would;
//!   inside a line it is kept, tabs and no-break spaces included.
//! - Characters a document cannot hold (control characters) are dropped,
//!   and a U+FEFF that opens the text is taken for the byte order mark it
//!   would be in a file.
//!
//! # Round trips
//!
//! Exporting a document and importing the result gives the same blocks,
//! text and character formatting, up to what the exporter itself
//! normalises (white space at the edges of lines, empty paragraphs, the
//! colour of a highlight, anything the export options leave out).
//! Importing any Markdown and exporting it gives Markdown that imports and
//! exports to itself. The tests hold both over generated documents and
//! generated Markdown.
//!
//! An image comes back as the image it was (its stored bytes, its width,
//! its alignment) when the import is given the images of the document
//! ([`KnownImages`](super::KnownImages)): the Markdown says which image
//! and what it shows, and nothing of how it is shown.
//!
//! The parser is [pulldown-cmark](https://docs.rs/pulldown-cmark), the
//! one the exporter's output is checked against. Bare addresses in text
//! (`https://example.com` without angle brackets) are text, as in
//! CommonMark; GitHub's alert quotes, definition lists and math are not
//! read as such and stay as typed.
//!
//! ```
//! use innerword_core::BlockKind;
//! use innerword_export::{ImportWarningKind, MarkdownImportOptions, from_markdown};
//!
//! let imported = from_markdown(
//!     "Costs:\n\n| Item | Price |\n|------|------:|\n| Ink  | 12 |\n",
//!     &MarkdownImportOptions::default(),
//! );
//! let table = &imported.document.blocks()[1];
//! assert_eq!(table.kind(), &BlockKind::code());
//! assert_eq!(table.text(), "| Item | Price |\n| ---- | ----: |\n| Ink  |    12 |");
//! assert_eq!(imported.warnings[0].kind, ImportWarningKind::Table);
//! assert_eq!(imported.warnings[0].line, 3);
//! ```

use std::borrow::Cow;

use innerword_core::{
    Alignment, Block, BlockKind, Document, DocumentSettings, Highlight, Image, ImageWidth,
    InlineAttrs, MAX_INDENT, Rgb, text::normalize_input,
};
use pulldown_cmark::{
    Alignment as ColumnAlignment, CodeBlockKind, Event, LinkType, Options, Parser, Tag, TagEnd,
};

use super::html::{self, Tag as HtmlTag};
use super::images::NoImages;
use super::{
    ImageProblem, ImageResolution, ImageResolver, ImportWarning, ImportWarningKind, Imported,
};

/// What happens to a level-1 heading that opens the Markdown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MarkdownLeadingHeading {
    /// It stays in the text, as the document's first block. The default:
    /// the app derives a document's title from its first heading, so the
    /// heading is the title already. [`Imported::title`] reports its text.
    #[default]
    Keep,
    /// It is taken out of the text and returned in [`Imported::title`], for
    /// a caller that keeps the title somewhere else. Only a level-1 heading
    /// that is the very first block (after front matter) is taken.
    Title,
}

/// What happens to front matter: a block of YAML between two `---` lines
/// (or of TOML between two `+++` lines) at the very start of the text.
///
/// A leading `---` line only counts as front matter when the line right
/// after it looks like a key (`title: …`) and a closing line follows;
/// anything else is ordinary Markdown (a rule, then text).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MarkdownFrontMatter {
    /// It is removed from the text, returned in
    /// [`Imported::front_matter`] and reported as a warning. The default:
    /// front matter is data about the text, not part of it. Its `title`
    /// key, if any, is reported in [`Imported::title`].
    #[default]
    Remove,
    /// It is kept as a code block (language `yaml` or `toml`) at the start
    /// of the document, and also returned in [`Imported::front_matter`].
    CodeBlock,
    /// Nothing is recognised: the lines are read as Markdown like the rest.
    Markdown,
}

/// What a plain line end inside a paragraph becomes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MarkdownSoftBreaks {
    /// A space, as Markdown defines it. The default.
    #[default]
    Space,
    /// A line break inside the block, as chat applications and GitHub
    /// comments show it. For text whose line ends are meant.
    LineBreak,
}

/// Options for [`from_markdown`]. The defaults read Markdown as a renderer
/// would and keep everything in the text.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MarkdownImportOptions {
    /// What happens to a level-1 heading that opens the text.
    pub leading_heading: MarkdownLeadingHeading,
    /// What happens to front matter.
    pub front_matter: MarkdownFrontMatter,
    /// What a plain line end inside a paragraph becomes.
    pub soft_breaks: MarkdownSoftBreaks,
    /// Read `==text==` as a highlight, the counterpart of
    /// [`MarkdownHighlight::Equals`](crate::MarkdownHighlight::Equals). Off
    /// by default: it is not Markdown, and `a == b` is common in text
    /// about code.
    pub highlight_equals: bool,
    /// The settings of the imported document (page setup, typography,
    /// language). Pass the settings of the document whose content is being
    /// replaced to keep them. An image that comes in without a width gets
    /// its original size on this page.
    pub settings: DocumentSettings,
}

/// Imports Markdown. See the [module documentation](self) for the mapping
/// and [`Imported`] for what comes back; this cannot fail.
///
/// Line endings of every kind are understood, a byte order mark is
/// skipped, control characters are removed.
///
/// ```
/// use innerword_core::BlockKind;
/// use innerword_export::{MarkdownImportOptions, MarkdownLeadingHeading, from_markdown};
///
/// let options = MarkdownImportOptions {
///     leading_heading: MarkdownLeadingHeading::Title,
///     ..Default::default()
/// };
/// let imported = from_markdown("# The Salt Road\n\n> *Day one.*\n", &options);
/// assert_eq!(imported.title.as_deref(), Some("The Salt Road"));
/// let blocks = imported.document.blocks();
/// assert_eq!(blocks.len(), 1);
/// assert_eq!(blocks[0].kind(), &BlockKind::Quote);
/// assert!(blocks[0].attrs_at(0).unwrap().italic);
/// ```
pub fn from_markdown(markdown: &str, options: &MarkdownImportOptions) -> Imported {
    from_markdown_with_images(markdown, options, &mut NoImages)
}

/// The extensions of Markdown the importer reads.
fn parser_options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
}

/// The destinations of the images in `markdown`, in the order of the text
/// and as [`ImageResolver::resolve`] is given them (without the angle
/// brackets and backslashes of the syntax), so that a caller can prepare
/// local files before it imports. Every image is listed, also one in a
/// place where it will stay a link, and one address as often as it is
/// used.
///
/// Front matter is skipped, as an import with the default options skips
/// it.
///
/// ```
/// use innerword_export::markdown_image_sources;
///
/// let markdown = "![A](one.png) and ![B](<my pictures/two.jpg>)\n\n# ![C](image:00ff)\n";
/// assert_eq!(
///     markdown_image_sources(markdown),
///     ["one.png", "my pictures/two.jpg", "image:00ff"]
/// );
/// ```
pub fn markdown_image_sources(markdown: &str) -> Vec<String> {
    let source = normalize_source(markdown);
    let body_start = find_front_matter(&source).map_or(0, |front| front.end);
    Parser::new_ext(&source[body_start..], parser_options())
        .filter_map(|event| match event {
            Event::Start(Tag::Image { dest_url, .. }) => Some(dest_url.to_string()),
            _ => None,
        })
        .collect()
}

/// Imports Markdown as [`from_markdown`] does, with someone to say which
/// stored image each image of the text is. See "Images" in the
/// [module documentation](self) for what becomes of an image, and
/// [`KnownImages`](super::KnownImages) for the resolver that knows the
/// images of a document.
///
/// `images` is asked once for every image of the Markdown, in the order of
/// the text.
///
/// ```
/// use innerword_core::{Image, ImageId};
/// use innerword_export::{
///     ImageResolution, ImageResolver, MarkdownImportOptions, from_markdown_with_images,
/// };
///
/// /// A library of one picture.
/// struct OnePicture(Image);
/// impl ImageResolver for OnePicture {
///     fn resolve(&mut self, source: &str) -> ImageResolution {
///         if source == "harbour.jpg" {
///             ImageResolution::Stored { image: self.0.clone(), width: None, alignment: None }
///         } else {
///             ImageResolution::NotFound
///         }
///     }
/// }
///
/// let id = ImageId::parse("3f2a9c1b5d6e7f80a1b2c3d4e5f60718").unwrap();
/// let mut library = OnePicture(Image::new(id, (800, 500), 144));
/// let imported = from_markdown_with_images(
///     "Arrival.\n\n![The harbour](harbour.jpg)\n\n![The ship](ship.jpg)\n",
///     &MarkdownImportOptions::default(),
///     &mut library,
/// );
/// let blocks = imported.document.blocks();
/// let harbour = blocks[1].as_image().unwrap();
/// assert_eq!((harbour.id, harbour.description.as_str()), (id, "The harbour"));
/// // 400 points wide in a column of 451: its original size.
/// assert_eq!(harbour.width.thousandths(), 886);
/// // The picture that is not there stays a link, and the caller is told.
/// assert_eq!(blocks[2].text(), "The ship");
/// assert_eq!(imported.image_problems[0].source, "ship.jpg");
/// assert_eq!(imported.image_problems[0].line, 5);
/// ```
pub fn from_markdown_with_images(
    markdown: &str,
    options: &MarkdownImportOptions,
    images: &mut dyn ImageResolver,
) -> Imported {
    let source = normalize_source(markdown);
    let mut warnings: Vec<ImportWarning> = Vec::new();
    let mut drafts: Vec<Draft> = Vec::new();
    let mut front_text: Option<String> = None;
    let mut front_title: Option<String> = None;
    let mut body_start = 0;
    if options.front_matter != MarkdownFrontMatter::Markdown
        && let Some(front) = find_front_matter(&source)
    {
        let content = source[front.content.clone()].trim_end().to_string();
        front_title = front_matter_title(&content, front.toml);
        body_start = front.end;
        match options.front_matter {
            MarkdownFrontMatter::CodeBlock => drafts.push(Draft {
                spans: vec![(content.clone(), InlineAttrs::default())],
                ..Draft::new(BlockKind::CodeBlock {
                    language: Some(if front.toml { "toml" } else { "yaml" }.to_string()),
                })
            }),
            _ => warnings.push(ImportWarning {
                kind: ImportWarningKind::FrontMatter,
                line: 1,
            }),
        }
        front_text = Some(content);
    }
    let front_blocks = drafts.len();

    let mut importer = Importer::new(&source, options, drafts, warnings, images);
    for (event, range) in
        Parser::new_ext(&source[body_start..], parser_options()).into_offset_iter()
    {
        importer.event(event, body_start + range.start);
    }
    // (A text that ends inside a block has ended it.)
    importer.settle_images();
    let Importer {
        mut drafts,
        warnings,
        problems,
        ..
    } = importer;

    // What an image left empty when it took a line of its own (a bullet or
    // numbered item that held nothing else: the image stands in its place).
    drafts.retain(|draft| !draft.left_to_an_image);
    let mut drafts = join_empty_items(drafts);

    // The title the text names.
    let heading_title = drafts
        .get(front_blocks)
        .filter(|d| d.kind == BlockKind::heading(1))
        .map(Draft::one_line)
        .filter(|t| !t.is_empty());
    let title = match options.leading_heading {
        MarkdownLeadingHeading::Title if heading_title.is_some() => {
            drafts.remove(front_blocks);
            heading_title
        }
        MarkdownLeadingHeading::Title => front_title,
        MarkdownLeadingHeading::Keep => front_title.or(heading_title),
    };

    let mut blocks: Vec<Block> = Vec::with_capacity(drafts.len());
    // Whether nothing that shows has been imported yet.
    let mut opening = true;
    for draft in drafts {
        if let Some(block) = draft.finish(options, opening) {
            opening &= !block.kind().is_list_item()
                && !block.is_void()
                && !block.kind().is_code()
                && block.text().trim().is_empty();
            blocks.push(block);
        }
    }

    let has_content = !blocks.is_empty();
    Imported {
        document: Document::from_blocks(blocks).with_settings(options.settings.clone()),
        title,
        front_matter: front_text,
        warnings,
        image_problems: problems,
        has_content,
    }
}

/// The text without a byte order mark and with `\n` line ends, so that no
/// later step meets half of a `\r\n`.
fn normalize_source(markdown: &str) -> Cow<'_, str> {
    let text = markdown.strip_prefix('\u{feff}').unwrap_or(markdown);
    if text.contains('\r') {
        Cow::Owned(text.replace("\r\n", "\n").replace('\r', "\n"))
    } else {
        Cow::Borrowed(text)
    }
}

// ----- front matter -----------------------------------------------------------------------

struct FrontMatter {
    /// The bytes between the delimiter lines.
    content: std::ops::Range<usize>,
    /// Where the Markdown goes on, after the closing line.
    end: usize,
    toml: bool,
}

fn find_front_matter(source: &str) -> Option<FrontMatter> {
    let first_end = source.find('\n')?;
    let toml = match source[..first_end].trim_end() {
        "---" => false,
        "+++" => true,
        _ => return None,
    };
    let content_start = first_end + 1;
    let mut at = content_start;
    let mut first_line = true;
    loop {
        if at >= source.len() {
            return None;
        }
        let line_end = source[at..].find('\n').map_or(source.len(), |i| at + i);
        let line = source[at..line_end].trim_end();
        let closes = if toml {
            line == "+++"
        } else {
            line == "---" || line == "..."
        };
        if closes {
            if first_line {
                // Nothing between the lines: two thematic breaks.
                return None;
            }
            return Some(FrontMatter {
                content: content_start..at,
                end: (line_end + 1).min(source.len()),
                toml,
            });
        }
        if first_line {
            // Front matter starts on the very next line. A rule followed
            // by a blank line, or by text, is a rule.
            if !looks_like_a_key(line, toml) {
                return None;
            }
            first_line = false;
        }
        at = line_end + 1;
    }
}

/// Whether the first line of a block is plausibly YAML or TOML: a key or
/// (TOML) a table header.
fn looks_like_a_key(line: &str, toml: bool) -> bool {
    if toml && line.starts_with('[') {
        return true;
    }
    let separator = if toml { '=' } else { ':' };
    let Some((key, rest)) = line.split_once(separator) else {
        return false;
    };
    let key = key.trim_end();
    let key = key
        .strip_prefix('"')
        .and_then(|k| k.strip_suffix('"'))
        .unwrap_or(key);
    !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | ' '))
        && !key.starts_with(' ')
        && (toml || rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t'))
}

/// The value of a top-level `title` key, when it is a plain or quoted
/// string on one line.
fn front_matter_title(content: &str, toml: bool) -> Option<String> {
    let separator = if toml { '=' } else { ':' };
    for line in content.lines() {
        let Some((key, value)) = line.split_once(separator) else {
            continue;
        };
        if !key.trim_end().eq_ignore_ascii_case("title") {
            continue;
        }
        let mut value = value.trim();
        let quoted = ['"', '\''].into_iter().find_map(|q| {
            let inner = value.strip_prefix(q)?;
            let end = inner.find(q)?;
            Some(&inner[..end])
        });
        match quoted {
            Some(inner) => value = inner,
            None => {
                // A YAML comment ends a plain value; `|` and `>` start a
                // block of text on the following lines.
                if let Some(hash) = value.find(" #") {
                    value = value[..hash].trim_end();
                }
                if matches!(value, "|" | ">" | "|-" | ">-" | "|+" | ">+") {
                    return None;
                }
            }
        }
        let title = value.split_whitespace().collect::<Vec<_>>().join(" ");
        return (!title.is_empty()).then_some(title);
    }
    None
}

// ----- drafts -----------------------------------------------------------------------------

/// A block being built.
struct Draft {
    kind: BlockKind,
    indent: u8,
    alignment: Alignment,
    spans: Vec<(String, InlineAttrs)>,
    /// An empty paragraph that is there on purpose (it keeps two lists
    /// apart).
    keep_empty: bool,
    /// What an image left empty when it went onto a line of its own: a
    /// bullet or numbered item whose own text was the image and nothing
    /// else, or the paragraph for the rest of a line that had no rest.
    /// Such a block is not part of the text.
    left_to_an_image: bool,
}

impl Draft {
    fn new(kind: BlockKind) -> Draft {
        Draft {
            kind,
            indent: 0,
            alignment: Alignment::Left,
            spans: Vec::new(),
            keep_empty: false,
            left_to_an_image: false,
        }
    }

    /// Adds text. It is brought into the form a document holds here
    /// (control characters removed, the Unicode line separators as line
    /// breaks), so that what is tidied afterwards is the final text.
    fn push(&mut self, text: &str, attrs: InlineAttrs) {
        let mut text = normalize_input(text);
        if attrs.code && text.contains('\n') {
            // Inline code is one line (a line end in a code span is a
            // space to Markdown too).
            text = text.replace('\n', " ").into();
        }
        if text.is_empty() {
            return;
        }
        match self.spans.last_mut() {
            Some((last, a)) if *a == attrs => last.push_str(&text),
            _ => self.spans.push((text.into_owned(), attrs)),
        }
    }

    /// The text on one line, white space collapsed.
    fn one_line(&self) -> String {
        let text: String = self.spans.iter().map(|(t, _)| t.as_str()).collect();
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    fn is_item(&self) -> bool {
        self.kind.is_list_item()
    }

    /// Whether the block shows nothing: no text, or white space only.
    fn is_blank(&self) -> bool {
        self.spans
            .iter()
            .all(|(text, attrs)| !attrs.code && text.chars().all(char::is_whitespace))
    }

    /// The finished block, or `None` for a paragraph that ended up with
    /// nothing in it. `opening` says that the block opens the text.
    fn finish(mut self, options: &MarkdownImportOptions, opening: bool) -> Option<Block> {
        let mut block = Block::new(self.kind.clone())
            .with_indent(self.indent)
            .with_alignment(self.alignment);
        if self.kind.is_void() {
            return Some(block);
        }
        if self.kind.is_code() {
            let mut text: String = self.spans.into_iter().map(|(t, _)| t).collect();
            // The line end before the closing fence belongs to the fence.
            if text.ends_with('\n') {
                text.pop();
            }
            return Some(block.with_span(&text, InlineAttrs::default()));
        }
        if options.highlight_equals {
            highlight_equals(&mut self.spans);
        }
        tidy_edges(&mut self.spans);
        if opening && self.kind == BlockKind::Paragraph && strip_opening_mark(&mut self.spans) {
            tidy_edges(&mut self.spans);
        }
        if self.spans.is_empty()
            && matches!(self.kind, BlockKind::Paragraph | BlockKind::Quote)
            && !self.keep_empty
        {
            return None;
        }
        for (text, attrs) in self.spans {
            block = block.with_span(&text, attrs);
        }
        Some(block)
    }
}

/// A bullet or numbered item without text takes the paragraph that
/// follows it as its first child: Markdown cannot tell the two apart (the
/// exporter writes both as `- text`), so there is one form for them.
/// Blocks that show nothing between the two do not count, as they do not
/// for the exporter.
fn join_empty_items(drafts: Vec<Draft>) -> Vec<Draft> {
    let mut out: Vec<Draft> = Vec::with_capacity(drafts.len());
    // The last block that is not an invisible child: where an item that
    // could take a paragraph would be.
    let mut last_solid: Option<usize> = None;
    for draft in drafts {
        let blank = draft.is_blank();
        if draft.kind == BlockKind::Paragraph
            && !blank
            && let Some(item) = last_solid.map(|i| &mut out[i])
            && matches!(item.kind, BlockKind::BulletItem | BlockKind::NumberedItem)
            && item.is_blank()
            && draft.indent == item.indent + 1
        {
            item.spans = draft.spans;
            continue;
        }
        let invisible_child = blank
            && !draft.is_item()
            && !draft.kind.is_void()
            && !draft.kind.is_code()
            && !draft.keep_empty
            && last_solid.is_some_and(|i| draft.indent == out[i].indent + 1);
        if !invisible_child {
            last_solid = Some(out.len());
        }
        out.push(draft);
    }
    out
}

/// One character of a block with the index of the span it came from.
type Cell = (char, usize);

fn cells_of(spans: &[(String, InlineAttrs)]) -> Vec<Cell> {
    spans
        .iter()
        .enumerate()
        .flat_map(|(i, (text, _))| text.chars().map(move |c| (c, i)))
        .collect()
}

fn spans_of(cells: &[(char, InlineAttrs)]) -> Vec<(String, InlineAttrs)> {
    let mut out: Vec<(String, InlineAttrs)> = Vec::new();
    for (c, attrs) in cells {
        match out.last_mut() {
            Some((text, a)) if a == attrs => text.push(*c),
            _ => out.push((c.to_string(), attrs.clone())),
        }
    }
    out
}

/// Removes spaces and tabs at both ends of every line, and empty lines at
/// both ends of the block. A parser has done this already for what the
/// Markdown itself holds; what is left comes from `<br>` and from things
/// that were left out. Code spans are not touched.
fn tidy_edges(spans: &mut Vec<(String, InlineAttrs)>) {
    let blank = |c: char| c == ' ' || c == '\t';
    let edge = |c: char| blank(c) || c == '\n';
    let untidy = {
        let first = spans.first().filter(|(_, a)| !a.code);
        let last = spans.last().filter(|(_, a)| !a.code);
        first.is_some_and(|(t, _)| t.starts_with(edge))
            || last.is_some_and(|(t, _)| t.ends_with(edge))
            || spans.iter().any(|(t, a)| {
                !a.code
                    && (t.contains(" \n")
                        || t.contains("\t\n")
                        || t.contains("\n ")
                        || t.contains("\n\t")
                        || t.starts_with('\n')
                        || t.ends_with('\n'))
            })
    };
    if !untidy {
        return;
    }
    let cells = cells_of(spans);
    let n = cells.len();
    let trimmable = |cell: &Cell| blank(cell.0) && !spans[cell.1].1.code;
    let mut keep = vec![true; n];
    let mut line_start = 0;
    for i in 0..=n {
        if i == n || cells[i].0 == '\n' {
            let mut a = line_start;
            while a < i && trimmable(&cells[a]) {
                keep[a] = false;
                a += 1;
            }
            let mut b = i;
            while b > a && trimmable(&cells[b - 1]) {
                keep[b - 1] = false;
                b -= 1;
            }
            line_start = i + 1;
        }
    }
    for i in 0..n {
        if keep[i] && cells[i].0 != '\n' {
            break;
        }
        keep[i] = false;
    }
    for i in (0..n).rev() {
        if keep[i] && cells[i].0 != '\n' {
            break;
        }
        keep[i] = false;
    }
    let kept: Vec<(char, InlineAttrs)> = cells
        .iter()
        .zip(&keep)
        .filter(|(_, keep)| **keep)
        .map(|((c, span), _)| (*c, spans[*span].1.clone()))
        .collect();
    *spans = spans_of(&kept);
}

/// Removes a U+FEFF that opens a paragraph, with the white space before
/// it, and says whether it did. Such a paragraph at the start of a text
/// (the character written as a reference, say) would export to a text that
/// opens with a byte order mark. The character shows nothing.
fn strip_opening_mark(spans: &mut Vec<(String, InlineAttrs)>) -> bool {
    const MARK: char = '\u{feff}';
    // Formatting that puts markup before the character when exported.
    let bare = |a: &InlineAttrs| {
        !(a.bold || a.italic || a.underline || a.strikethrough || a.code)
            && a.highlight.is_none()
            && a.link.is_none()
    };
    let mut stripped = false;
    loop {
        let first = spans.iter().enumerate().find_map(|(i, (text, attrs))| {
            text.char_indices()
                .find(|(_, c)| attrs.code || !c.is_whitespace())
                .map(|(at, c)| (i, at, c == MARK && bare(attrs)))
        });
        let Some((span, at, true)) = first else {
            return stripped;
        };
        spans[span].0.replace_range(..at + MARK.len_utf8(), "");
        spans.drain(..span);
        if spans[0].0.is_empty() {
            spans.remove(0);
        }
        stripped = true;
    }
}

/// Reads `==text==` as a highlight: the opening `==` is followed by
/// something that is not white space, the closing one preceded by it, both
/// outside code and on one line.
fn highlight_equals(spans: &mut Vec<(String, InlineAttrs)>) {
    if !spans.iter().any(|(t, a)| !a.code && t.contains("==")) {
        return;
    }
    let cells = cells_of(spans);
    let n = cells.len();
    let code = |i: usize| spans[cells[i].1].1.code;
    let pair_at = |i: usize| {
        i + 1 < n && cells[i].0 == '=' && cells[i + 1].0 == '=' && !code(i) && !code(i + 1)
    };
    // A run of three or more equals signs is a rule someone typed, not a
    // delimiter.
    let lone_pair = |i: usize| {
        pair_at(i) && !(i > 0 && cells[i - 1].0 == '=') && !(i + 2 < n && cells[i + 2].0 == '=')
    };
    let mut remove = vec![false; n];
    let mut marked = vec![false; n];
    let mut i = 0;
    while i < n {
        if !(lone_pair(i) && i + 2 < n && !cells[i + 2].0.is_whitespace()) {
            i += 1;
            continue;
        }
        let mut close = None;
        let mut j = i + 3;
        while j < n && cells[j].0 != '\n' {
            if lone_pair(j) && !cells[j - 1].0.is_whitespace() {
                close = Some(j);
                break;
            }
            j += 1;
        }
        let Some(close) = close else {
            i += 2;
            continue;
        };
        for k in [i, i + 1, close, close + 1] {
            remove[k] = true;
        }
        for flag in &mut marked[i + 2..close] {
            *flag = true;
        }
        i = close + 2;
    }
    if !remove.iter().any(|r| *r) {
        return;
    }
    let kept: Vec<(char, InlineAttrs)> = (0..n)
        .filter(|&i| !remove[i])
        .map(|i| {
            let mut attrs = spans[cells[i].1].1.clone();
            if marked[i] && attrs.highlight.is_none() {
                attrs.highlight = Some(Highlight::Yellow);
            }
            (cells[i].0, attrs)
        })
        .collect();
    *spans = spans_of(&kept);
}

// ----- the event walk ---------------------------------------------------------------------

/// What encloses the block being read. Each remembers the state it
/// replaced.
enum Container {
    Quote {
        was_quoted: bool,
    },
    List {
        ordered: bool,
        /// The next item is the list's first.
        first: bool,
    },
    Item {
        block: usize,
        was_quoted: bool,
    },
    Footnote,
}

/// What an open inline HTML tag does to the text inside it.
enum Effect {
    Bold,
    Italic,
    Underline,
    Strike,
    Code,
    Highlight(Highlight),
    Link(String),
    Style(Vec<Effect>),
    Color(Rgb),
    Font(String),
}

impl Effect {
    fn apply(&self, attrs: &mut InlineAttrs) {
        match self {
            Effect::Bold => attrs.bold = true,
            Effect::Italic => attrs.italic = true,
            Effect::Underline => attrs.underline = true,
            Effect::Strike => attrs.strikethrough = true,
            Effect::Code => attrs.code = true,
            Effect::Highlight(h) => attrs.highlight = Some(*h),
            Effect::Link(url) => attrs.link = Some(url.clone()),
            Effect::Color(c) => attrs.color = Some(*c),
            Effect::Font(f) => attrs.font = Some(f.clone()),
            Effect::Style(effects) => effects.iter().for_each(|e| e.apply(attrs)),
        }
    }
}

/// The effect of an opening tag, if the document has formatting for it.
fn effect_of(tag: &HtmlTag) -> Option<Effect> {
    let HtmlTag::Open { name, .. } = tag else {
        return None;
    };
    let style_effects = |style: &str| {
        let mut effects = Vec::new();
        for (property, value) in html::style_declarations(style) {
            let v = value.to_ascii_lowercase();
            match property.as_str() {
                "color" => effects.extend(html::css_color(&value).map(Effect::Color)),
                "font-family" => effects.extend(html::css_font(&value).map(Effect::Font)),
                "background-color" | "background" => {
                    effects.push(Effect::Highlight(html::css_highlight(&value)));
                }
                "font-weight" => {
                    let heavy =
                        v == "bold" || v == "bolder" || v.parse::<u32>().is_ok_and(|w| w >= 600);
                    effects.extend(heavy.then_some(Effect::Bold));
                }
                "font-style" => {
                    effects.extend((v == "italic" || v == "oblique").then_some(Effect::Italic));
                }
                "text-decoration" | "text-decoration-line" => {
                    effects.extend(v.contains("underline").then_some(Effect::Underline));
                    effects.extend(v.contains("line-through").then_some(Effect::Strike));
                }
                _ => {}
            }
        }
        effects
    };
    match name.as_str() {
        "b" | "strong" => Some(Effect::Bold),
        "i" | "em" => Some(Effect::Italic),
        "u" | "ins" => Some(Effect::Underline),
        "s" | "del" | "strike" => Some(Effect::Strike),
        "code" | "kbd" | "samp" | "tt" => Some(Effect::Code),
        "mark" => {
            let named = ["class", "data-color", "color"]
                .into_iter()
                .filter_map(|a| tag.attr(a))
                .find_map(Highlight::from_name);
            let styled = tag.attr("style").and_then(|style| {
                style_effects(style).into_iter().find_map(|e| match e {
                    Effect::Highlight(h) => Some(h),
                    _ => None,
                })
            });
            Some(Effect::Highlight(
                named.or(styled).unwrap_or(Highlight::Yellow),
            ))
        }
        "a" => {
            let href = tag.attr("href")?.trim();
            (!href.is_empty()).then(|| Effect::Link(href.to_string()))
        }
        "span" => {
            let effects = style_effects(tag.attr("style")?);
            (!effects.is_empty()).then_some(Effect::Style(effects))
        }
        _ => None,
    }
}

/// A table being read.
struct Table {
    columns: Vec<ColumnAlignment>,
    rows: Vec<Vec<String>>,
    cell: String,
    /// Open links: the address and where the link text starts in `cell`.
    links: Vec<(String, usize)>,
}

impl Table {
    /// The table as aligned text: a header row, a rule that carries the
    /// column alignment, the rows.
    fn render(&self) -> String {
        let columns = self
            .rows
            .iter()
            .map(Vec::len)
            .max()
            .unwrap_or(0)
            .max(self.columns.len());
        let width = |s: &str| s.chars().count();
        let mut widths = vec![3usize; columns];
        for row in &self.rows {
            for (i, cell) in row.iter().enumerate() {
                widths[i] = widths[i].max(width(cell));
            }
        }
        let alignment = |i: usize| {
            self.columns
                .get(i)
                .copied()
                .unwrap_or(ColumnAlignment::None)
        };
        let mut lines: Vec<String> = Vec::new();
        for (r, row) in self.rows.iter().enumerate() {
            let mut line = String::from("|");
            for (i, w) in widths.iter().enumerate() {
                let cell = row.get(i).map_or("", String::as_str);
                let pad = w - width(cell);
                let (left, right) = match alignment(i) {
                    ColumnAlignment::Right => (pad, 0),
                    ColumnAlignment::Center => (pad / 2, pad - pad / 2),
                    _ => (0, pad),
                };
                line.push(' ');
                line.push_str(&" ".repeat(left));
                line.push_str(cell);
                line.push_str(&" ".repeat(right));
                line.push_str(" |");
            }
            lines.push(line);
            if r == 0 {
                let mut rule = String::from("|");
                for (i, w) in widths.iter().enumerate() {
                    let (left, right) = match alignment(i) {
                        ColumnAlignment::Left => (":", "-"),
                        ColumnAlignment::Right => ("-", ":"),
                        ColumnAlignment::Center => (":", ":"),
                        ColumnAlignment::None => ("-", "-"),
                    };
                    rule.push_str(&format!(" {left}{}{right} |", "-".repeat(w - 2)));
                }
                lines.push(rule);
            }
        }
        lines.join("\n")
    }
}

/// What becomes of an image that is being read.
enum Fate {
    /// It stays a link, as every image did before documents had images: the
    /// text inside it is the link's text.
    Link,
    /// It becomes a block. The text inside it is its description.
    Block(Box<Placed>),
    /// It stands inside the alternative text of an image that becomes a
    /// block: only its text counts, as part of that description.
    Inside,
}

/// An image that becomes a block, while its alternative text is read.
struct Placed {
    image: Image,
    width: Option<ImageWidth>,
    alignment: Option<Alignment>,
    /// The alternative text so far.
    alt: String,
    title: String,
    /// Where the image begins in the source.
    offset: usize,
    /// It stands inside a link.
    linked: bool,
}

/// An image that has begun and not ended.
struct OpenImage {
    /// How many pieces of text had been written when it began.
    texts: u64,
    fate: Fate,
}

/// The images that one run of inline content (a paragraph, the text of a
/// list item) was split at.
struct Split {
    /// The draft that holds what came before the first image.
    first: usize,
    /// For each image: where its warning goes among the warnings, should
    /// the run turn out to hold text as well, and where the image begins.
    images: Vec<(usize, usize)>,
}

struct Importer<'a> {
    source: &'a str,
    options: &'a MarkdownImportOptions,
    /// Who says what the images are.
    resolver: &'a mut dyn ImageResolver,
    /// The images whose files could not be used.
    problems: Vec<ImageProblem>,
    /// The byte offset of every line start, for line numbers.
    line_starts: Vec<usize>,
    drafts: Vec<Draft>,
    warnings: Vec<ImportWarning>,
    /// The draft that takes inline content.
    cur: Option<usize>,
    /// `cur` is a list item nothing has been put into yet: its first
    /// paragraph is its own text.
    fresh_item: bool,
    stack: Vec<Container>,
    /// How many list items enclose the current block.
    items: usize,
    /// The innermost quote or list item around the current block is a
    /// quote.
    quoted: bool,
    /// A quote that is not inside another has begun and holds no block
    /// yet.
    fresh_quote: bool,
    bold: u32,
    italic: u32,
    strike: u32,
    links: Vec<String>,
    /// Open inline HTML tags, by name.
    html: Vec<(String, Effect)>,
    /// The images that have begun and not ended, the outermost first.
    images: Vec<OpenImage>,
    /// Where the block being read was split at images.
    split: Option<Split>,
    texts: u64,
    /// The alignment set by an enclosing `<div align>` and the tag name
    /// that ends it.
    align: Option<(Alignment, String)>,
    html_block: Option<(String, usize)>,
    in_code_block: bool,
    table: Option<Table>,
    /// The label of a footnote definition, to put before its first block.
    footnote: Option<String>,
}

impl<'a> Importer<'a> {
    fn new(
        source: &'a str,
        options: &'a MarkdownImportOptions,
        drafts: Vec<Draft>,
        warnings: Vec<ImportWarning>,
        resolver: &'a mut dyn ImageResolver,
    ) -> Importer<'a> {
        let mut line_starts = vec![0];
        line_starts.extend(source.match_indices('\n').map(|(i, _)| i + 1));
        Importer {
            source,
            options,
            resolver,
            problems: Vec::new(),
            line_starts,
            drafts,
            warnings,
            cur: None,
            fresh_item: false,
            stack: Vec::new(),
            items: 0,
            quoted: false,
            fresh_quote: false,
            bold: 0,
            italic: 0,
            strike: 0,
            links: Vec::new(),
            html: Vec::new(),
            images: Vec::new(),
            split: None,
            texts: 0,
            align: None,
            html_block: None,
            in_code_block: false,
            table: None,
            footnote: None,
        }
    }

    fn line_of(&self, offset: usize) -> usize {
        self.line_starts.partition_point(|start| *start <= offset)
    }

    fn warn(&mut self, kind: ImportWarningKind, offset: usize) {
        let line = self.line_of(offset.min(self.source.len()));
        // One tag pair or one nested structure on a line is one thing.
        if self
            .warnings
            .last()
            .is_some_and(|w| w.kind == kind && w.line == line)
        {
            return;
        }
        self.warnings.push(ImportWarning { kind, line });
    }

    // ----- blocks ---------------------------------------------------------------------

    /// The indent of a block that is not a list item: one level inside
    /// each enclosing item.
    fn child_indent(&self) -> u8 {
        self.items.min(MAX_INDENT as usize) as u8
    }

    /// Ends the block that takes inline content.
    fn close_inline(&mut self) {
        self.settle_images();
        self.cur = None;
        self.fresh_item = false;
        self.html.clear();
    }

    /// Starts a block in the current place and returns its index.
    fn start_block(&mut self, kind: BlockKind) -> usize {
        self.close_inline();
        let text_block = !kind.is_void() && !kind.is_code();
        // The label of a footnote goes before the first paragraph of the
        // note, or stands alone when the note opens with something else.
        let label = match &kind {
            BlockKind::Paragraph | BlockKind::Quote => self.footnote.take(),
            _ => {
                self.flush_footnote();
                None
            }
        };
        let mut draft = Draft::new(kind);
        draft.indent = self.child_indent();
        // The document reads quote blocks in a row as the paragraphs of
        // one quote. Where Markdown has two quotes, an empty paragraph
        // keeps them apart.
        if std::mem::take(&mut self.fresh_quote)
            && draft.kind == BlockKind::Quote
            && self
                .drafts
                .last()
                .is_some_and(|d| d.kind == BlockKind::Quote && d.indent == draft.indent)
        {
            let mut gap = Draft::new(BlockKind::Paragraph);
            gap.indent = draft.indent;
            gap.keep_empty = true;
            self.drafts.push(gap);
        }
        if let Some(label) = label {
            draft.push(&format!("{label} "), InlineAttrs::default());
        }
        if text_block && let Some((alignment, _)) = &self.align {
            draft.alignment = *alignment;
        }
        self.drafts.push(draft);
        self.drafts.len() - 1
    }

    /// Writes the label of a footnote definition as a paragraph of its own.
    fn flush_footnote(&mut self) {
        if let Some(label) = self.footnote.take() {
            let mut note = Draft::new(BlockKind::Paragraph);
            note.indent = self.child_indent();
            note.push(&label, InlineAttrs::default());
            self.drafts.push(note);
        }
    }

    /// A paragraph in the current place: a quote paragraph inside a quote.
    fn start_paragraph(&mut self) -> usize {
        let kind = if self.quoted {
            BlockKind::Quote
        } else {
            BlockKind::Paragraph
        };
        self.start_block(kind)
    }

    /// A block other than a paragraph begins: inside a quote it loses the
    /// quote.
    fn note_quoted_block(&mut self, offset: usize) {
        if self.quoted {
            self.warn(ImportWarningKind::QuotedBlock, offset);
        }
    }

    fn start_item(&mut self, offset: usize) {
        let (ordered, first) = match self.stack.last_mut() {
            Some(Container::List { ordered, first }) => (*ordered, std::mem::take(first)),
            _ => (false, false),
        };
        self.close_inline();
        self.flush_footnote();
        self.fresh_quote = false;
        let too_deep = self.items > MAX_INDENT as usize;
        if too_deep {
            self.warn(ImportWarningKind::NestingTooDeep, offset);
        }
        let indent = self.child_indent();
        // The document runs a list item together with a list item of the
        // same sort before it at the same level. Where Markdown has two
        // lists there, an empty paragraph keeps them apart.
        if first && !too_deep {
            let before = self.drafts.iter().rev().find(|d| d.indent <= indent);
            let same_list = before.is_some_and(|d| {
                d.indent == indent && d.is_item() && (d.kind == BlockKind::NumberedItem) == ordered
            });
            if same_list {
                let mut gap = Draft::new(BlockKind::Paragraph);
                gap.indent = indent;
                gap.keep_empty = true;
                self.drafts.push(gap);
            }
        }
        let mut draft = Draft::new(if ordered {
            BlockKind::NumberedItem
        } else {
            BlockKind::BulletItem
        });
        draft.indent = indent;
        self.drafts.push(draft);
        let block = self.drafts.len() - 1;
        self.stack.push(Container::Item {
            block,
            was_quoted: self.quoted,
        });
        self.items += 1;
        self.quoted = false;
        self.cur = Some(block);
        self.fresh_item = true;
    }

    fn raw_html_block(&mut self, raw: &str, offset: usize) {
        let text = raw.trim();
        if text.is_empty() {
            return;
        }
        let whole = |tag: &(HtmlTag, usize)| tag.1 == text.len();
        let first = html::parse_tag(text);
        match &first {
            Some(tag @ (HtmlTag::Comment(inner), _)) if whole(tag) => {
                let name: String = inner
                    .chars()
                    .filter(|c| !c.is_whitespace() && *c != '-' && *c != '_')
                    .collect();
                if name.eq_ignore_ascii_case("pagebreak") {
                    self.start_block(BlockKind::PageBreak);
                    self.close_inline();
                } else if !inner.is_empty() {
                    self.warn(ImportWarningKind::HtmlComment, offset);
                }
                return;
            }
            Some(tag @ (HtmlTag::Open { name, .. }, _)) if whole(tag) => {
                let alignment = tag
                    .0
                    .attr("align")
                    .and_then(html::html_alignment)
                    .or_else(|| {
                        html::style_declarations(tag.0.attr("style")?)
                            .into_iter()
                            .find(|(p, _)| p == "text-align")
                            .and_then(|(_, v)| html::html_alignment(&v))
                    })
                    .or((name == "center").then_some(Alignment::Center));
                if let Some(alignment) = alignment
                    && matches!(name.as_str(), "div" | "p" | "center")
                    && self.align.is_none()
                {
                    self.align = Some((alignment, name.clone()));
                    return;
                }
            }
            Some(tag @ (HtmlTag::Close { name }, _)) if whole(tag) => {
                if self.align.as_ref().is_some_and(|(_, n)| n == name) {
                    self.align = None;
                    return;
                }
            }
            // `<div style="page-break-after: always"></div>`.
            Some((open @ HtmlTag::Open { name, .. }, used)) => {
                let closes = matches!(
                    html::parse_tag(text[*used..].trim_start()),
                    Some((HtmlTag::Close { name: closing }, rest))
                        if closing == *name && rest == text[*used..].trim_start().len()
                );
                let style = open.attr("style").unwrap_or("").to_ascii_lowercase();
                let class = open.attr("class").unwrap_or("").to_ascii_lowercase();
                let breaks = style.contains("page-break-after")
                    || style.contains("page-break-before")
                    || style.contains("break-after")
                    || style.contains("break-before")
                    || class.contains("page-break")
                    || class.contains("pagebreak");
                if closes && breaks {
                    self.start_block(BlockKind::PageBreak);
                    self.close_inline();
                    return;
                }
            }
            _ => {}
        }
        self.warn(ImportWarningKind::HtmlBlock, offset);
        self.note_quoted_block(offset);
        let block = self.start_block(BlockKind::CodeBlock {
            language: Some("html".to_string()),
        });
        self.drafts[block].push(raw.trim_end(), InlineAttrs::default());
        self.close_inline();
    }

    // ----- inline content -------------------------------------------------------------

    fn attrs(&self, code: bool) -> InlineAttrs {
        let mut attrs = InlineAttrs {
            bold: self.bold > 0,
            italic: self.italic > 0,
            strikethrough: self.strike > 0,
            code,
            link: self.links.last().filter(|l| !l.is_empty()).cloned(),
            ..InlineAttrs::default()
        };
        for (_, effect) in &self.html {
            effect.apply(&mut attrs);
        }
        attrs
    }

    /// Adds text to the open block, opening a paragraph when there is
    /// none (text directly inside a list item after its first block).
    fn text(&mut self, text: &str, code: bool) {
        if let Some(alt) = self.open_description() {
            alt.push_str(text);
            return;
        }
        let block = match self.cur {
            Some(block) => block,
            None => {
                let block = self.start_paragraph();
                self.cur = Some(block);
                block
            }
        };
        let attrs = self.attrs(code);
        self.drafts[block].push(text, attrs);
        self.texts += 1;
    }

    fn inline_html(&mut self, raw: &str, offset: usize) {
        if let Some(alt) = self.open_description() {
            // A description is plain text: of a tag only a line break
            // leaves something behind.
            if matches!(html::parse_tag(raw), Some((HtmlTag::Open { name, .. }, _)) if name == "br")
            {
                alt.push(' ');
            }
            return;
        }
        match html::parse_tag(raw) {
            Some((HtmlTag::Comment(inner), used)) if used == raw.len() => {
                if !inner.is_empty() {
                    self.warn(ImportWarningKind::HtmlComment, offset);
                }
                return;
            }
            Some((tag @ HtmlTag::Open { .. }, used)) if used == raw.len() => {
                let HtmlTag::Open { name, .. } = &tag else {
                    return;
                };
                if name == "br" {
                    self.text("\n", false);
                    return;
                }
                if let Some(effect) = effect_of(&tag) {
                    self.html.push((name.clone(), effect));
                    return;
                }
            }
            Some((HtmlTag::Close { name }, used)) if used == raw.len() => {
                if let Some(open) = self.html.iter().rposition(|(n, _)| *n == name) {
                    self.html.remove(open);
                    return;
                }
            }
            _ => {}
        }
        self.warn(ImportWarningKind::InlineHtml, offset);
        self.text(raw, false);
    }

    // ----- images ---------------------------------------------------------------------

    /// The description being read: the alternative text of the outermost
    /// open image that becomes a block.
    fn open_description(&mut self) -> Option<&mut String> {
        self.images
            .iter_mut()
            .find_map(|open| match &mut open.fate {
                Fate::Block(placed) => Some(&mut placed.alt),
                _ => None,
            })
    }

    /// Whether an image can stand where the text is now: in a paragraph or
    /// in the text of a list item, and not inside another image.
    fn image_can_stand_here(&mut self) -> bool {
        if !self.images.is_empty() {
            return false;
        }
        let block = match self.cur {
            Some(block) => block,
            // Directly inside a list item, after its first block.
            None => {
                let block = self.start_paragraph();
                self.cur = Some(block);
                block
            }
        };
        let kind = &self.drafts[block].kind;
        *kind == BlockKind::Paragraph || kind.is_list_item()
    }

    fn start_image(&mut self, destination: &str, title: &str, offset: usize) {
        // Every image of the text is asked for, in its order, whatever
        // becomes of it.
        let resolution = self.resolver.resolve(destination);
        let inside = self
            .images
            .iter()
            .any(|open| !matches!(open.fate, Fate::Link));
        let fate = if inside {
            Fate::Inside
        } else {
            match resolution {
                ImageResolution::Stored {
                    image,
                    width,
                    alignment,
                } if self.image_can_stand_here() => {
                    let linked = self.links.iter().any(|link| !link.is_empty())
                        || self.attrs(false).link.is_some();
                    Fate::Block(Box::new(Placed {
                        image,
                        width,
                        alignment,
                        alt: String::new(),
                        title: title.to_string(),
                        offset,
                        linked,
                    }))
                }
                // Stored, where no image can stand; or not for us to read.
                ImageResolution::Stored { .. } | ImageResolution::Remote => {
                    self.warn(ImportWarningKind::Image, offset);
                    Fate::Link
                }
                ImageResolution::NotFound => {
                    self.warn(ImportWarningKind::ImageNotFound, offset);
                    self.problem(destination, offset, None);
                    Fate::Link
                }
                ImageResolution::Unsupported(sentence) => {
                    self.warn(ImportWarningKind::ImageUnsupported, offset);
                    self.problem(destination, offset, Some(sentence));
                    Fate::Link
                }
            }
        };
        if matches!(fate, Fate::Link) {
            self.links.push(destination.trim().to_string());
        }
        self.images.push(OpenImage {
            texts: self.texts,
            fate,
        });
    }

    fn problem(&mut self, destination: &str, offset: usize, sentence: Option<String>) {
        self.problems.push(ImageProblem {
            source: destination.to_string(),
            line: self.line_of(offset.min(self.source.len())),
            sentence,
        });
    }

    fn end_image(&mut self) {
        let Some(open) = self.images.pop() else {
            return;
        };
        match open.fate {
            Fate::Inside => {}
            Fate::Link => {
                // An image without alternative text shows its address.
                let url = self.links.last().cloned().unwrap_or_default();
                if open.texts == self.texts && !url.is_empty() {
                    self.text(&url, false);
                }
                self.links.pop();
            }
            Fate::Block(placed) => self.place_image(*placed),
        }
    }

    /// Puts an image on a line of its own: the block that was being read
    /// ends before it, and what follows goes into a new paragraph after it.
    fn place_image(&mut self, placed: Placed) {
        let Placed {
            image,
            width,
            alignment,
            alt,
            title,
            offset,
            linked,
        } = placed;
        // What the text says about the image wins over what the resolver
        // knows of it; a text that says nothing keeps that.
        // (The description is kept character for character, so that an
        // image that is written out and read back is the image it was; the
        // block cleans what a description cannot hold.)
        let description = if !alt.trim().is_empty() {
            if !title.trim().is_empty() {
                self.warn(ImportWarningKind::LinkTitle, offset);
            }
            alt
        } else if !title.trim().is_empty() {
            title
        } else {
            image.description.clone()
        };
        let width = width.unwrap_or_else(|| image.original_width(&self.options.settings.page));
        let around = self
            .align
            .as_ref()
            .map(|(alignment, _)| *alignment)
            .filter(|a| *a != Alignment::Justify);
        let alignment = around.or(alignment).unwrap_or(Alignment::Left);
        if linked {
            self.warn(ImportWarningKind::ImageLink, offset);
        }

        let before = self.cur;
        let mut block = Draft::new(BlockKind::Image(Image {
            width,
            description,
            ..image
        }));
        block.alignment = alignment;
        self.drafts.push(block);
        // The rest of the line. A paragraph that nothing is put into is
        // dropped when the blocks are made.
        let mut rest = Draft::new(BlockKind::Paragraph);
        rest.indent = self.child_indent();
        rest.alignment = match before.map(|b| &self.drafts[b]) {
            Some(draft) if !draft.is_item() => draft.alignment,
            _ => self.align.as_ref().map_or(Alignment::Left, |(a, _)| *a),
        };
        self.drafts.push(rest);
        self.cur = Some(self.drafts.len() - 1);
        self.fresh_item = false;

        let first = before.unwrap_or(self.drafts.len() - 2);
        let at = self.warnings.len();
        self.split
            .get_or_insert(Split {
                first,
                images: Vec::new(),
            })
            .images
            .push((at, offset));
    }

    /// Called when a run of inline content ends: says, for the images it
    /// was split at, whether they stood inside a line of text.
    fn settle_images(&mut self) {
        let Some(split) = self.split.take() else {
            return;
        };
        let mut in_text = false;
        for draft in &mut self.drafts[split.first..] {
            if draft.kind.is_void() {
                continue;
            }
            if !draft.is_blank() {
                in_text = true;
            } else if draft.kind == BlockKind::Paragraph && !draft.is_item() {
                // Nothing came before the image, or after it.
                draft.left_to_an_image = !draft.keep_empty;
            }
        }
        let first = &mut self.drafts[split.first];
        if first.is_item() && first.is_blank() {
            match first.kind {
                // The image was all the item held: it takes its place.
                BlockKind::BulletItem | BlockKind::NumberedItem => first.left_to_an_image = true,
                // A to-do keeps its box, on a line without the image.
                _ => in_text = true,
            }
        }
        if !in_text {
            return;
        }
        // Each warning goes where the warnings stood when its image was
        // read, so that they stay in the order of the text.
        let mut last_line = None;
        let mut added: Vec<(usize, ImportWarning)> = Vec::new();
        for (at, offset) in split.images {
            let line = self.line_of(offset.min(self.source.len()));
            // Two images on one line are one thing to tell.
            if last_line == Some(line) {
                continue;
            }
            last_line = Some(line);
            added.push((
                at,
                ImportWarning {
                    kind: ImportWarningKind::ImageInText,
                    line,
                },
            ));
        }
        for (at, warning) in added.into_iter().rev() {
            self.warnings.insert(at.min(self.warnings.len()), warning);
        }
    }

    // ----- tables ---------------------------------------------------------------------

    /// Handles an event inside a table. Returns whether the table ended.
    fn table_event(&mut self, event: Event<'_>) -> bool {
        let Some(table) = &mut self.table else {
            return true;
        };
        match event {
            Event::Start(Tag::TableHead | Tag::TableRow) => table.rows.push(Vec::new()),
            Event::Start(Tag::TableCell) => {
                table.cell.clear();
                table.links.clear();
            }
            Event::End(TagEnd::TableCell) => {
                let cell = table.cell.split_whitespace().collect::<Vec<_>>().join(" ");
                if let Some(row) = table.rows.last_mut() {
                    row.push(cell);
                }
            }
            Event::End(TagEnd::Table) => return true,
            Event::Text(text) | Event::InlineHtml(text) | Event::Html(text) => {
                table.cell.push_str(&text);
            }
            Event::Code(code) => {
                table.cell.push('`');
                table.cell.push_str(&code);
                table.cell.push('`');
            }
            Event::SoftBreak | Event::HardBreak => table.cell.push(' '),
            Event::FootnoteReference(label) => {
                table.cell.push_str(&format!("[^{label}]"));
            }
            Event::Start(Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. }) => {
                table.links.push((dest_url.to_string(), table.cell.len()));
            }
            Event::End(TagEnd::Link | TagEnd::Image) => {
                if let Some((url, from)) = table.links.pop() {
                    let text = table.cell[from.min(table.cell.len())..].trim();
                    if text.is_empty() {
                        table.cell.push_str(&url);
                    } else if text != url && !url.is_empty() {
                        table.cell.push_str(&format!(" ({url})"));
                    }
                }
            }
            _ => {}
        }
        false
    }

    // ----- events ---------------------------------------------------------------------

    fn event(&mut self, event: Event<'_>, offset: usize) {
        if self.table.is_some() {
            // A table is kept as text, its images with it. The resolver is
            // asked all the same: it hears of every image of the text.
            if let Event::Start(Tag::Image { dest_url, .. }) = &event {
                self.resolver.resolve(dest_url);
            }
            if self.table_event(event)
                && let Some(table) = self.table.take()
            {
                let block = self.start_block(BlockKind::code());
                self.drafts[block].push(&table.render(), InlineAttrs::default());
                self.close_inline();
            }
            return;
        }
        match event {
            Event::Start(tag) => self.start(tag, offset),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => {
                if let Some((html, _)) = &mut self.html_block {
                    html.push_str(&text);
                } else if self.in_code_block {
                    if let Some(block) = self.cur {
                        self.drafts[block].push(&text, InlineAttrs::default());
                    }
                } else {
                    self.text(&text, false);
                }
            }
            Event::Code(code) => self.text(&code, true),
            Event::InlineMath(math) => self.text(&format!("${math}$"), false),
            Event::DisplayMath(math) => self.text(&format!("$${math}$$"), false),
            Event::Html(raw) => match &mut self.html_block {
                Some((html, _)) => html.push_str(&raw),
                None => self.raw_html_block(&raw, offset),
            },
            Event::InlineHtml(raw) => self.inline_html(&raw, offset),
            Event::FootnoteReference(label) => {
                self.warn(ImportWarningKind::Footnote, offset);
                self.text(&format!("[^{label}]"), false);
            }
            Event::SoftBreak => match self.options.soft_breaks {
                MarkdownSoftBreaks::Space => self.text(" ", false),
                MarkdownSoftBreaks::LineBreak => self.text("\n", false),
            },
            Event::HardBreak => self.text("\n", false),
            Event::Rule => {
                self.note_quoted_block(offset);
                self.start_block(BlockKind::Divider);
                self.close_inline();
            }
            Event::TaskListMarker(checked) => {
                let item = self.stack.iter().rev().find_map(|c| match c {
                    Container::Item { block, .. } => Some(*block),
                    _ => None,
                });
                if let Some(block) = item {
                    self.drafts[block].kind = BlockKind::TodoItem { checked };
                }
            }
        }
    }

    fn start(&mut self, tag: Tag<'_>, offset: usize) {
        match tag {
            Tag::Paragraph => {
                if self.fresh_item && self.cur.is_some() {
                    self.fresh_item = false;
                } else {
                    let block = self.start_paragraph();
                    self.cur = Some(block);
                }
            }
            Tag::Heading { level, .. } => {
                self.note_quoted_block(offset);
                let block = self.start_block(BlockKind::heading(level as u8));
                self.cur = Some(block);
            }
            Tag::BlockQuote(_) => {
                self.close_inline();
                if self.quoted {
                    self.warn(ImportWarningKind::NestedQuote, offset);
                }
                self.stack.push(Container::Quote {
                    was_quoted: self.quoted,
                });
                self.fresh_quote = !self.quoted;
                self.quoted = true;
            }
            Tag::CodeBlock(kind) => {
                self.note_quoted_block(offset);
                let language = match kind {
                    CodeBlockKind::Fenced(info) => info
                        .split_whitespace()
                        .next()
                        .filter(|word| !word.is_empty())
                        .map(str::to_string),
                    CodeBlockKind::Indented => None,
                };
                let block = self.start_block(BlockKind::CodeBlock { language });
                self.cur = Some(block);
                self.in_code_block = true;
            }
            Tag::HtmlBlock => {
                self.close_inline();
                self.html_block = Some((String::new(), offset));
            }
            Tag::List(start) => {
                self.close_inline();
                self.note_quoted_block(offset);
                if start.is_some_and(|n| n != 1) {
                    self.warn(ImportWarningKind::ListStart, offset);
                }
                self.stack.push(Container::List {
                    ordered: start.is_some(),
                    first: true,
                });
            }
            Tag::Item => self.start_item(offset),
            Tag::FootnoteDefinition(label) => {
                self.close_inline();
                self.warn(ImportWarningKind::Footnote, offset);
                self.footnote = Some(format!("[^{label}]:"));
                self.stack.push(Container::Footnote);
            }
            Tag::Table(columns) => {
                self.close_inline();
                self.warn(ImportWarningKind::Table, offset);
                self.note_quoted_block(offset);
                self.table = Some(Table {
                    columns,
                    rows: Vec::new(),
                    cell: String::new(),
                    links: Vec::new(),
                });
            }
            Tag::Emphasis => self.italic += 1,
            Tag::Strong => self.bold += 1,
            Tag::Strikethrough => self.strike += 1,
            Tag::Link {
                link_type,
                dest_url,
                title,
                ..
            } => {
                if !title.is_empty() {
                    self.warn(ImportWarningKind::LinkTitle, offset);
                }
                let url = dest_url.trim();
                self.links.push(match link_type {
                    LinkType::Email if !url.is_empty() => format!("mailto:{url}"),
                    _ => url.to_string(),
                });
            }
            Tag::Image {
                dest_url, title, ..
            } => self.start_image(&dest_url, &title, offset),
            // Extensions that are not switched on; their content is read
            // as what it holds.
            Tag::TableHead
            | Tag::TableRow
            | Tag::TableCell
            | Tag::DefinitionList
            | Tag::DefinitionListTitle
            | Tag::DefinitionListDefinition
            | Tag::Superscript
            | Tag::Subscript
            | Tag::MetadataBlock(_) => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph | TagEnd::Heading(_) => self.close_inline(),
            TagEnd::CodeBlock => {
                self.in_code_block = false;
                self.close_inline();
            }
            TagEnd::HtmlBlock => {
                if let Some((raw, start)) = self.html_block.take() {
                    self.raw_html_block(&raw, start);
                }
            }
            TagEnd::BlockQuote(_) => {
                self.close_inline();
                if let Some(Container::Quote { was_quoted }) = self.stack.pop() {
                    self.quoted = was_quoted;
                }
            }
            TagEnd::List(_) => {
                self.close_inline();
                self.stack.pop();
            }
            TagEnd::Item => {
                self.close_inline();
                if let Some(Container::Item { was_quoted, .. }) = self.stack.pop() {
                    self.quoted = was_quoted;
                    self.items = self.items.saturating_sub(1);
                }
            }
            TagEnd::FootnoteDefinition => {
                // A note with nothing in it is its label alone.
                self.flush_footnote();
                self.close_inline();
                self.stack.pop();
            }
            TagEnd::Emphasis => self.italic = self.italic.saturating_sub(1),
            TagEnd::Strong => self.bold = self.bold.saturating_sub(1),
            TagEnd::Strikethrough => self.strike = self.strike.saturating_sub(1),
            TagEnd::Link => {
                self.links.pop();
            }
            TagEnd::Image => self.end_image(),
            TagEnd::Table
            | TagEnd::TableHead
            | TagEnd::TableRow
            | TagEnd::TableCell
            | TagEnd::DefinitionList
            | TagEnd::DefinitionListTitle
            | TagEnd::DefinitionListDefinition
            | TagEnd::Superscript
            | TagEnd::Subscript
            | TagEnd::MetadataBlock(_) => {}
        }
    }
}
