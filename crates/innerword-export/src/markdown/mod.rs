//! Markdown export: CommonMark with the common GitHub extensions
//! (strikethrough, task lists, footnotes).
//!
//! What maps directly: headings, bold, italic, strikethrough, inline code,
//! links, bullet / numbered / to-do lists nested by indent level (numbered
//! items carry their real numbers), block quotes, fenced code blocks with
//! their language, dividers and line breaks (written as hard breaks).
//!
//! What Markdown has no syntax for follows [`MarkdownOptions`]: underline
//! and highlight are inline HTML by default, text colour, font and
//! alignment are dropped by default, a page break is an HTML comment, and
//! comments are left out unless asked for.
//!
//! An image is a line of its own, `![description](image:<id>)`: a
//! reference to the stored picture, which is what an agent reads and
//! writes back. For a file somebody else will open,
//! [`MarkdownImages::Files`] links picture files instead and
//! [`export_package`](crate::export_package) returns them. The width of an
//! image is not written: Markdown has no syntax for it.
//!
//! The output is tidy: one blank line between blocks, none between the
//! items of a list, exactly one newline at the end. Literal text is escaped
//! only where it would otherwise be read as markup (`[1]`, `5 * 3`,
//! `_config.yml` and `a<b` stay as they are). Whitespace at the edges
//! of bold, italic and other marks is moved outside the delimiters, and a
//! mark whose delimiters CommonMark would not pair (for example bold that
//! ends in punctuation right before a letter) is written with an HTML tag
//! (`<strong>`, `<em>`, `<del>`) instead.
//!
//! Two rare constructs also use a little HTML because no Markdown spelling
//! reads the same in every parser: a to-do item without text that has
//! nested blocks is written `- [ ] <!-- -->`, and inline code holding `]:`
//! inside a link that starts a block is written with `<code>`.
//!
//! ```
//! use innerword_core::{Block, BlockKind, Document, InlineAttrs, InlineStyle};
//! use innerword_export::{ExportSource, MarkdownOptions, to_markdown};
//!
//! let doc = Document::from_blocks(vec![
//!     Block::with_text(BlockKind::heading(2), "Plan"),
//!     Block::paragraph("Ship ")
//!         .with_span("soon ", InlineAttrs::default().with(InlineStyle::Bold))
//!         .with_span("(1 * 2)", InlineAttrs::default()),
//!     Block::with_text(BlockKind::NumberedItem, "Write"),
//!     Block::with_text(BlockKind::TodoItem { checked: true }, "Outline").with_indent(1),
//!     Block::with_text(BlockKind::NumberedItem, "Edit"),
//! ]);
//! let md = to_markdown(&ExportSource::from(&doc), &MarkdownOptions::default());
//! assert_eq!(md, "## Plan\n\nShip **soon** (1 * 2)\n\n1. Write\n   - [x] Outline\n2. Edit\n");
//! ```

mod emphasis;
mod escape;
mod inline;

use std::collections::{BTreeMap, HashMap, HashSet};

use innerword_core::{Alignment, Block, BlockKind, CommentThread, Document, Image, ImageId};

use crate::common::{
    first_comment_ranges, is_blank_text, ordinals, part_needs_title, thread_entries,
};
use crate::{ExportAsset, ExportSource, PartTitles};
use escape::{
    TextContext, escape_line, escape_line_in, file_path_in_link, image_alt, image_destination,
};
use inline::{Anchor, render_inline};

/// What happens to comment threads in Markdown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MarkdownComments {
    /// Comments are left out. The default.
    #[default]
    Omit,
    /// Each thread becomes an HTML comment at the end of the text it is
    /// anchored on: `<!-- Ada: Check this. | Ben: Done. -->`. Invisible when
    /// rendered.
    HtmlComments,
    /// Each thread becomes a footnote: a reference `[^1]` at the end of the
    /// anchored text and the comment and its replies as the note, at the
    /// end of the file.
    Footnotes,
}

/// How highlighted text is written in Markdown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MarkdownHighlight {
    /// `<mark>text</mark>`. The default; the colour is not kept.
    #[default]
    Html,
    /// `==text==`, which Obsidian, Typora and some other tools understand.
    /// Not CommonMark: other renderers show the equals signs.
    Equals,
    /// The highlight is dropped.
    Drop,
}

/// How a page break is written in Markdown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MarkdownPageBreak {
    /// `<!-- pagebreak -->`. The default; invisible when rendered.
    #[default]
    Comment,
    /// A thematic break, `---`.
    Rule,
    /// `<div style="page-break-after: always"></div>`, which breaks the
    /// page when the rendered HTML is printed.
    Html,
    /// Nothing.
    Omit,
}

/// How an image is written in Markdown.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum MarkdownImages {
    /// `![description](image:<id>)`: a reference to the stored image. The
    /// default: what agents, the assistant and the clipboard get, and what
    /// the importer reads back as the same image.
    #[default]
    References,
    /// `![description](<dir>/image-<first 8 hex of the id>.<ext>)`: a link
    /// to a picture file in the folder `dir` beside the Markdown file. The
    /// files are returned by [`export_package`](crate::export_package) as
    /// its assets, one per picture however often it is used
    /// ([`to_markdown`] writes the same text and returns no files).
    ///
    /// Two different images of one export whose ids begin with the same
    /// eight digits get 12, 16, ... 32 digits (the later one in the text),
    /// so no file is written over. An image whose bytes the source does
    /// not have is written as a reference, as with
    /// [`References`](MarkdownImages::References).
    Files {
        /// The folder, relative to the Markdown file, with `/` between its
        /// parts; [`images_dir_name`](crate::images_dir_name) names it
        /// after the file. Empty: beside the file.
        dir: String,
    },
}

/// Options for [`to_markdown`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkdownOptions {
    /// Write underlined text as `<u>text</u>` (default). When off,
    /// underline is dropped.
    pub underline_html: bool,
    /// How highlighted text is written.
    pub highlight: MarkdownHighlight,
    /// Keep text colours as `<span style="color:#rrggbb">`. Off by default:
    /// colours are dropped.
    pub colors: bool,
    /// Keep font families as `<span style="font-family:'Name'">`. Off by
    /// default: fonts are dropped.
    pub fonts: bool,
    /// Keep the alignment of centred, right-aligned and justified top-level
    /// paragraphs and headings, and of centred and right-aligned images,
    /// by wrapping them in `<div align="…">`. Off by default: alignment is
    /// dropped.
    pub alignment: bool,
    /// How a page break is written.
    pub page_break: MarkdownPageBreak,
    /// What happens to comment threads.
    pub comments: MarkdownComments,
    /// Whether the documents of a collection get their title as a level-1
    /// heading.
    pub part_titles: PartTitles,
    /// Separate the documents of a collection with a thematic break
    /// (default). When off, only a blank line separates them.
    pub part_rule: bool,
    /// How an image is written.
    pub images: MarkdownImages,
}

impl Default for MarkdownOptions {
    fn default() -> MarkdownOptions {
        MarkdownOptions {
            underline_html: true,
            highlight: MarkdownHighlight::default(),
            colors: false,
            fonts: false,
            alignment: false,
            page_break: MarkdownPageBreak::default(),
            comments: MarkdownComments::default(),
            part_titles: PartTitles::default(),
            part_rule: true,
            images: MarkdownImages::default(),
        }
    }
}

impl MarkdownOptions {
    /// Options that write nothing but CommonMark and GitHub syntax: no
    /// inline HTML for underline, highlight or page breaks (emphasis that
    /// only HTML can express still uses a tag).
    pub fn strict() -> MarkdownOptions {
        MarkdownOptions {
            underline_html: false,
            highlight: MarkdownHighlight::Drop,
            page_break: MarkdownPageBreak::Rule,
            ..MarkdownOptions::default()
        }
    }
}

/// Exports `source` as Markdown. The result ends with exactly one newline,
/// or is empty when there is nothing to write.
///
/// With [`MarkdownImages::Files`] the text links picture files that this
/// function does not return: use [`export_package`](crate::export_package)
/// to get them with the text.
pub fn to_markdown(source: &ExportSource<'_>, options: &MarkdownOptions) -> String {
    to_markdown_with_assets(source, options).0
}

/// The Markdown of `source` and the picture files it links.
pub(crate) fn to_markdown_with_assets(
    source: &ExportSource<'_>,
    options: &MarkdownOptions,
) -> (String, Vec<ExportAsset>) {
    let mut footnotes: Vec<String> = Vec::new();
    let mut pictures = Pictures::new(source, &options.images);
    let mut parts: Vec<String> = Vec::new();
    for part in source.parts() {
        let mut writer = Writer::new(part.document, options, &mut footnotes, &mut pictures);
        if part_needs_title(source, part, options.part_titles) {
            writer.push_other(format!("# {}", escape_line(&part.title, true)));
        }
        writer.document();
        let text = writer.finish();
        if !text.is_empty() {
            parts.push(text);
        }
    }
    let separator = if options.part_rule {
        "\n\n---\n\n"
    } else {
        "\n\n"
    };
    let mut out = parts.join(separator);
    for note in &footnotes {
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(note);
    }
    if !out.is_empty() {
        out.push('\n');
    }
    (out, pictures.assets)
}

/// The pictures of one export: what each image is written as, and the
/// files that go beside the Markdown.
struct Pictures<'a> {
    source: &'a ExportSource<'a>,
    /// The folder of the picture files; `None` writes references.
    dir: Option<String>,
    /// What the link of each image met so far points at.
    destinations: HashMap<ImageId, String>,
    /// The digits of the file names given out.
    taken: HashSet<String>,
    assets: Vec<ExportAsset>,
}

impl<'a> Pictures<'a> {
    fn new(source: &'a ExportSource<'a>, images: &MarkdownImages) -> Pictures<'a> {
        let dir = match images {
            MarkdownImages::References => None,
            MarkdownImages::Files { dir } => {
                Some(dir.trim().trim_end_matches(['/', '\\']).replace('\\', "/"))
            }
        };
        Pictures {
            source,
            dir,
            destinations: HashMap::new(),
            taken: HashSet::new(),
            assets: Vec::new(),
        }
    }

    /// What the link of `image` points at: its file, written as an asset
    /// the first time it is asked for, or the reference to the stored
    /// image where there is no file to write.
    fn destination(&mut self, image: &Image) -> String {
        let id = image.id;
        if let Some(known) = self.destinations.get(&id) {
            return known.clone();
        }
        let file = self
            .dir
            .as_ref()
            .and_then(|dir| Some((dir, self.source.image(id)?)));
        let destination = match file {
            None => format!("image:{id}"),
            Some((dir, data)) => {
                // A name is the first digits of the id: as many as it
                // takes to differ from every name given out before.
                let hex = id.to_string();
                let mut digits = 8;
                while digits < hex.len() && self.taken.contains(&hex[..digits]) {
                    digits += 4;
                }
                let digits = digits.min(hex.len());
                self.taken.insert(hex[..digits].to_string());
                let name = format!("image-{}.{}", &hex[..digits], data.format.extension());
                let path = if dir.is_empty() {
                    name
                } else {
                    format!("{dir}/{name}")
                };
                let link = file_path_in_link(&path);
                self.assets.push(ExportAsset {
                    path,
                    bytes: data.bytes.into_owned(),
                });
                link
            }
        };
        self.destinations.insert(id, destination.clone());
        destination
    }
}

/// The two kinds of Markdown list. Bullets and to-dos share one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Family {
    Bullet,
    Ordered,
}

/// An open list item.
#[derive(Clone, Debug)]
struct Level {
    /// The document indent level of the item.
    indent: u8,
    /// What the lines of the item's children start with.
    prefix: String,
    family: Family,
    /// Which of the two markers the list uses (`-` or `*`, `.` or `)`).
    alternate: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ChunkKind {
    /// A list item; `separate` puts a blank line before it.
    Item {
        separate: bool,
    },
    /// A quote written with this prefix, from the block at this index.
    Quote {
        prefix: String,
        block: usize,
    },
    Other,
}

struct Chunk {
    text: String,
    kind: ChunkKind,
    /// Join to the previous chunk with a single newline.
    glue: bool,
}

struct Writer<'a, 'p> {
    doc: &'a Document,
    options: &'a MarkdownOptions,
    footnotes: &'a mut Vec<String>,
    pictures: &'a mut Pictures<'p>,
    chunks: Vec<Chunk>,
    stack: Vec<Level>,
    /// The most recent list at each depth, while nothing has broken it.
    last_list: Vec<Option<(Family, bool)>>,
    /// The depth of the previous chunk when it is a list item, and whether
    /// that item has no text.
    prev_item: Option<(usize, bool)>,
    /// The depth of the previous chunk when it is a to-do item without
    /// text, which needs a filler before it can have children.
    bare_todo: Option<usize>,
    /// Comment anchors per block: inside the text, or after the block.
    inline_anchors: BTreeMap<usize, Vec<Anchor>>,
    block_anchors: BTreeMap<usize, Vec<String>>,
}

impl<'a, 'p> Writer<'a, 'p> {
    fn new(
        doc: &'a Document,
        options: &'a MarkdownOptions,
        footnotes: &'a mut Vec<String>,
        pictures: &'a mut Pictures<'p>,
    ) -> Writer<'a, 'p> {
        let mut writer = Writer {
            doc,
            options,
            footnotes,
            pictures,
            chunks: Vec::new(),
            stack: Vec::new(),
            last_list: Vec::new(),
            prev_item: None,
            bare_todo: None,
            inline_anchors: BTreeMap::new(),
            block_anchors: BTreeMap::new(),
        };
        writer.collect_comments();
        writer
    }

    fn collect_comments(&mut self) {
        if self.options.comments == MarkdownComments::Omit {
            return;
        }
        for (id, range) in first_comment_ranges(self.doc) {
            let Some(thread) = self.doc.comment(id) else {
                continue;
            };
            // The note goes at the end of the commented text. When the
            // range ends in a block with nothing to show, step back to the
            // last block of the range that has text.
            let visible = |b: &Block| !b.is_void() && !is_blank_text(b);
            let mut block = range.end.block;
            let mut offset = range.end.offset;
            while block > range.start.block && !self.doc.block(block).is_some_and(visible) {
                block -= 1;
                offset = self.doc.block(block).map_or(0, Block::len);
            }
            let Some(target) = self.doc.block(block).filter(|b| visible(b)) else {
                continue;
            };
            let raw = match self.options.comments {
                MarkdownComments::Omit => continue,
                MarkdownComments::HtmlComments => html_comment(thread),
                MarkdownComments::Footnotes => {
                    let label = self.footnotes.len() + 1;
                    self.footnotes.push(footnote(label, thread));
                    format!("[^{label}]")
                }
            };
            if target.kind().is_code() {
                self.block_anchors.entry(block).or_default().push(raw);
            } else {
                self.inline_anchors
                    .entry(block)
                    .or_default()
                    .push(Anchor { offset, raw });
            }
        }
    }

    /// Called before a chunk that sits `depth` list items deep is written.
    ///
    /// A to-do without text is just `- [ ]`. Parsers disagree on what
    /// that is: for some the brackets are the item's text (a line right
    /// below continues or underlines it), for others the item is empty (a
    /// blank line ends it). When such an item gets a child, an empty HTML
    /// comment after the marker makes it an item with content for all of
    /// them.
    fn before_chunk(&mut self, depth: usize) {
        if let Some(todo_depth) = self.bare_todo.take()
            && depth == todo_depth + 1
            && let Some(chunk) = self.chunks.last_mut()
        {
            chunk.text.push_str(" <!-- -->");
        }
    }

    fn push_other(&mut self, text: String) {
        self.before_chunk(self.stack.len());
        let glue = self.glue_to_empty_item();
        self.chunks.push(Chunk {
            text,
            kind: ChunkKind::Other,
            glue,
        });
        self.prev_item = None;
    }

    /// Whether the next block is the first child of an item without text.
    /// A blank line after such an item would end it ("a list item can begin
    /// with at most one blank line"), so its first child follows directly.
    fn glue_to_empty_item(&self) -> bool {
        matches!(self.prev_item, Some((depth, true)) if depth + 1 == self.stack.len())
    }

    fn finish(self) -> String {
        let mut out = String::new();
        for (i, chunk) in self.chunks.iter().enumerate() {
            if i > 0 {
                let tight = chunk.glue
                    || (matches!(chunk.kind, ChunkKind::Item { separate: false })
                        && matches!(self.chunks[i - 1].kind, ChunkKind::Item { .. }));
                out.push_str(if tight { "\n" } else { "\n\n" });
            }
            out.push_str(&chunk.text);
        }
        out
    }

    /// Closes the list items that a block at `indent` does not belong to
    /// and returns the prefix of the block's lines.
    fn close_levels(&mut self, indent: u8) -> String {
        while self.stack.last().is_some_and(|l| l.indent >= indent) {
            self.stack.pop();
        }
        self.stack
            .last()
            .map(|l| l.prefix.clone())
            .unwrap_or_default()
    }

    fn document(&mut self) {
        let ordinals = ordinals(self.doc);
        let doc = self.doc;
        for (index, block) in doc.blocks().iter().enumerate() {
            match block.kind() {
                BlockKind::BulletItem | BlockKind::NumberedItem | BlockKind::TodoItem { .. } => {
                    self.list_item(index, block, ordinals[index]);
                }
                _ => self.block(index, block),
            }
            if let Some(raws) = self.block_anchors.remove(&index) {
                let prefix = self
                    .stack
                    .last()
                    .map(|l| l.prefix.clone())
                    .unwrap_or_default();
                self.push_other(format!("{prefix}{}", raws.concat()));
            }
        }
    }

    fn inline(&self, index: usize, block: &Block, heading: bool) -> String {
        let none = Vec::new();
        let anchors = self.inline_anchors.get(&index).unwrap_or(&none);
        render_inline(block, anchors, self.options, heading)
    }

    fn list_item(&mut self, index: usize, block: &Block, ordinal: Option<usize>) {
        let family = if matches!(block.kind(), BlockKind::NumberedItem) {
            Family::Ordered
        } else {
            Family::Bullet
        };
        // The item that was open at the depth this one lands on, if any.
        let before: Vec<Level> = self.stack.clone();
        let parent_prefix = self.close_levels(block.indent());
        let depth = self.stack.len();
        // A numbered item that the document counts as "1" starts a new list
        // even when an item of the same kind is open at this depth (items
        // at different indent levels can land on the same depth).
        let restarts = family == Family::Ordered && ordinal == Some(1);
        let sibling = before
            .get(depth)
            .filter(|l| l.family == family && !restarts);
        if self.last_list.len() <= depth {
            self.last_list.resize(depth + 1, None);
        }
        let (alternate, new_list) = match sibling {
            Some(s) => (s.alternate, false),
            // A list of the same kind is still open at this depth as far as
            // a parser can tell (nothing but deeper blocks came since): the
            // new list takes the other marker so the two are not merged.
            None => match self.last_list[depth] {
                Some((f, alt)) if f == family => (!alt, true),
                _ => (false, true),
            },
        };
        let marker = match (family, block.kind()) {
            (Family::Ordered, _) => format!(
                "{}{}",
                ordinal.unwrap_or(1),
                if alternate { ')' } else { '.' }
            ),
            _ => if alternate { "*" } else { "-" }.to_string(),
        };
        let task = match block.kind() {
            BlockKind::TodoItem { checked: true } => "[x] ",
            BlockKind::TodoItem { checked: false } => "[ ] ",
            _ => "",
        };
        let child_prefix = format!("{parent_prefix}{}", " ".repeat(marker.len() + 1));
        let mut text = self.inline(index, block, false);
        // `- |` right under the text of the item above is the delimiter
        // row of a table to a parser, with that text as the header row.
        if marker == "-" && task.is_empty() && depth > 0 {
            let first_line = text.split('\n').next().unwrap_or("");
            if first_line.contains('|')
                && first_line
                    .chars()
                    .all(|c| matches!(c, '|' | ':' | '-' | ' ' | '\t'))
            {
                text = text.replacen('|', "\\|", 1);
            }
        }
        let line = format!(
            "{parent_prefix}{marker} {task}{}",
            text.replace('\n', &format!("\n{child_prefix}"))
        );
        // An empty item directly under its parent's text would be read as
        // the underline of a setext heading ("-") or as more text ("1.").
        // A to-do without text is not empty to a parser: its "[ ]" is the
        // text of a paragraph, which a child directly below would continue
        // or underline.
        self.before_chunk(depth);
        let empty = text.is_empty() && task.is_empty();
        let empty_first_child =
            empty && self.prev_item.is_some_and(|(d, empty)| d < depth && !empty);
        self.chunks.push(Chunk {
            text: line.trim_end().to_string(),
            kind: ChunkKind::Item {
                separate: empty_first_child || (new_list && depth == 0 && self.prev_item.is_some()),
            },
            glue: false,
        });
        self.stack.push(Level {
            indent: block.indent(),
            prefix: child_prefix,
            family,
            alternate,
        });
        self.last_list.truncate(depth + 1);
        self.last_list[depth] = Some((family, alternate));
        self.prev_item = Some((depth, empty));
        self.bare_todo = (text.is_empty() && !task.is_empty()).then_some(depth);
    }

    fn block(&mut self, index: usize, block: &Block) {
        let prefix = self.close_levels(block.indent());
        let depth = self.stack.len();
        if is_blank_text(block) {
            // Nothing is written, but the lists around it stay separate.
            return;
        }
        let text = match block.kind() {
            BlockKind::Divider => format!("{prefix}---"),
            BlockKind::PageBreak => match self.options.page_break {
                MarkdownPageBreak::Comment => format!("{prefix}<!-- pagebreak -->"),
                MarkdownPageBreak::Rule => format!("{prefix}---"),
                MarkdownPageBreak::Html => {
                    format!("{prefix}<div style=\"page-break-after: always\"></div>")
                }
                MarkdownPageBreak::Omit => return,
            },
            BlockKind::CodeBlock { language } => {
                code_block(block.text(), language.as_deref(), &prefix)
            }
            BlockKind::Image(image) => {
                // One line. The width is not written: Markdown has no
                // syntax for it.
                let destination = self.pictures.destination(image);
                let line = format!(
                    "{prefix}![{}]({})",
                    image_alt(&image.description),
                    image_destination(&destination)
                );
                let align = match block.alignment() {
                    Alignment::Center => Some("center"),
                    Alignment::Right => Some("right"),
                    Alignment::Left | Alignment::Justify => None,
                };
                match align {
                    Some(a) if self.options.alignment && depth == 0 => {
                        format!("<div align=\"{a}\">\n\n{line}\n\n</div>")
                    }
                    _ => line,
                }
            }
            BlockKind::Quote => {
                let quote_prefix = format!("{prefix}> ");
                let text = self.inline(index, block, false);
                let lines = format!(
                    "{quote_prefix}{}",
                    text.replace('\n', &format!("\n{quote_prefix}"))
                );
                // Quote blocks in a row are the paragraphs of one quote.
                if let Some(Chunk {
                    text: previous,
                    kind:
                        ChunkKind::Quote {
                            prefix: p,
                            block: b,
                        },
                    ..
                }) = self.chunks.last_mut()
                    && *p == quote_prefix
                    && *b + 1 == index
                {
                    previous.push_str(&format!("\n{}\n{lines}", quote_prefix.trim_end()));
                    *b = index;
                    return;
                }
                self.before_chunk(depth);
                let glue = self.glue_to_empty_item();
                self.chunks.push(Chunk {
                    text: lines,
                    kind: ChunkKind::Quote {
                        prefix: quote_prefix,
                        block: index,
                    },
                    glue,
                });
                self.prev_item = None;
                self.last_list.truncate(depth);
                return;
            }
            kind => {
                let level = kind.heading_level();
                let text = self.inline(index, block, level.is_some());
                let first = match level {
                    Some(n) => format!("{prefix}{} ", "#".repeat(n as usize)),
                    None => prefix.clone(),
                };
                let body = format!("{first}{}", text.replace('\n', &format!("\n{prefix}")));
                let align = match block.alignment() {
                    Alignment::Left => None,
                    Alignment::Center => Some("center"),
                    Alignment::Right => Some("right"),
                    Alignment::Justify => Some("justify"),
                };
                match align {
                    Some(a) if self.options.alignment && depth == 0 => {
                        format!("<div align=\"{a}\">\n\n{body}\n\n</div>")
                    }
                    _ => body,
                }
            }
        };
        self.push_other(text);
        self.last_list.truncate(depth);
    }
}

/// A fenced code block. The fence is longer than any run of backticks in
/// the code.
fn code_block(code: &str, language: Option<&str>, prefix: &str) -> String {
    let mut longest = 0;
    let mut run = 0;
    for c in code.chars() {
        run = if c == '`' { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    let fence = "`".repeat((longest + 1).max(3));
    // The info string: the first word of the language, without characters
    // that mean something there.
    let info: String = language
        .unwrap_or("")
        .split_whitespace()
        .next()
        .unwrap_or("")
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, '+' | '-' | '_' | '#' | '.'))
        .collect();
    let mut out = format!("{prefix}{fence}{info}");
    if !code.is_empty() {
        for line in code.split('\n') {
            out.push('\n');
            if line.is_empty() {
                out.push_str(prefix.trim_end());
            } else {
                out.push_str(prefix);
                out.push_str(line);
            }
        }
    }
    out.push_str(&format!("\n{prefix}{fence}"));
    out
}

/// One line of comment text, safe inside `<!-- -->`.
fn comment_safe(text: &str) -> String {
    let mut s = text.split_whitespace().collect::<Vec<_>>().join(" ");
    while s.contains("--") {
        s = s.replace("--", "- -");
    }
    s
}

fn html_comment(thread: &CommentThread) -> String {
    let entries: Vec<String> = thread_entries(thread)
        .into_iter()
        .map(|(author, body)| {
            let body = comment_safe(body);
            let author = comment_safe(author);
            if author.is_empty() {
                body
            } else {
                format!("{author}: {body}")
            }
        })
        .collect();
    let resolved = if thread.resolved { " (resolved)" } else { "" };
    format!("<!-- {}{resolved} -->", entries.join(" | "))
}

/// The footnote definition of a thread: the comment, then one paragraph
/// per reply.
fn footnote(label: usize, thread: &CommentThread) -> String {
    let mut paragraphs: Vec<String> = Vec::new();
    for (i, (author, body)) in thread_entries(thread).into_iter().enumerate() {
        // The lines of a body are one paragraph, which also holds the
        // delimiters around the author's name.
        let ctx = TextContext {
            stars: true,
            ..TextContext::for_text(&format!("{author}\n{body}"))
        };
        // Every kind of white space is trimmed here (a line of no-break
        // spaces would otherwise be a paragraph of nothing).
        let mut lines: Vec<&str> = body.split('\n').map(str::trim).collect();
        while lines.last().is_some_and(|l| l.is_empty()) {
            lines.pop();
        }
        let lead = lines.iter().take_while(|l| l.is_empty()).count();
        lines.drain(..lead);
        let lines: Vec<String> = lines
            .into_iter()
            .map(|l| escape_line_in(l, false, false, ctx))
            .collect();
        let mut text = lines.join("\\\n    ");
        let resolved = if i == 0 && thread.resolved {
            " (resolved)"
        } else {
            ""
        };
        let author = author.split_whitespace().collect::<Vec<_>>().join(" ");
        if !author.is_empty() {
            let name = escape_line_in(&author, false, false, ctx);
            // The name is its own chunk; the delimiters sit against a line
            // start and a colon, which always pair.
            let head = format!("**{name}{resolved}:**");
            text = if text.is_empty() {
                head
            } else {
                format!("{head} {text}")
            };
        } else if !resolved.is_empty() {
            text = format!("{text}{resolved}").trim().to_string();
        }
        if !text.is_empty() {
            paragraphs.push(text);
        }
    }
    if paragraphs.is_empty() {
        paragraphs.push("(empty comment)".to_string());
    }
    format!("[^{label}]: {}", paragraphs.join("\n\n    "))
}
