//! Plain-text export, for places where Markdown is not rendered (social
//! posts, plain e-mail).
//!
//! Paragraphs are separated by blank lines, list items sit on consecutive
//! lines, bullets are a bullet character, numbered items carry their real
//! numbers, to-dos a ballot box, quotes a `>` prefix, and links are written
//! as `text (url)`. Formatting that plain text cannot carry is dropped;
//! optionally bold and italic become the Unicode mathematical alphabets
//! that LinkedIn and X posts commonly use.
//!
//! ```
//! use writer_core::{Block, BlockKind, Document, InlineAttrs, InlineStyle};
//! use writer_export::{ExportSource, PlainTextOptions, to_plain_text};
//!
//! let doc = Document::from_blocks(vec![
//!     Block::paragraph("Three things:"),
//!     Block::with_text(BlockKind::BulletItem, "one"),
//!     Block::with_text(BlockKind::BulletItem, "two"),
//!     Block::paragraph("Read ").with_span("more", InlineAttrs {
//!         link: Some("https://example.com".into()),
//!         ..Default::default()
//!     }),
//! ]);
//! let text = to_plain_text(&ExportSource::from(&doc), &PlainTextOptions::default());
//! assert_eq!(text, "Three things:\n\n• one\n• two\n\nRead more (https://example.com)\n");
//! ```

use writer_core::{Block, BlockKind, Document};

use crate::common::{is_list_item, ordinals, part_needs_title, trimmed_lines};
use crate::{ExportSource, PartTitles};

/// How links are written in plain text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlainTextLinks {
    /// `text (url)`; only the URL when the text is the URL. The default.
    #[default]
    TextAndUrl,
    /// Only the link text.
    TextOnly,
    /// Only the URL.
    UrlOnly,
}

/// Options for [`to_plain_text`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlainTextOptions {
    /// Write bold and italic text (and headings, as bold) with the Unicode
    /// mathematical sans-serif alphabets, which social networks show as
    /// styled text. Only ASCII letters and digits have such forms; other
    /// characters stay as they are. Off by default: screen readers and
    /// search do not treat these characters as letters.
    pub unicode_styles: bool,
    /// The marker of a bullet item (default `"•"`).
    pub bullet: String,
    /// The marker of an open to-do (default `"☐"`, U+2610 ballot box).
    pub todo_unchecked: String,
    /// The marker of a done to-do (default `"☑"`, U+2611 ballot box with check).
    pub todo_checked: String,
    /// How links are written.
    pub links: PlainTextLinks,
    /// The indent of one list level (default two spaces).
    pub indent: String,
    /// The prefix of every quoted line (default `"> "`).
    pub quote_prefix: String,
    /// The line written for a divider, and between the documents of a
    /// collection (default `"* * *"`).
    pub divider: String,
    /// Whether the documents of a collection get their title as a line of
    /// their own.
    pub part_titles: PartTitles,
}

impl Default for PlainTextOptions {
    fn default() -> PlainTextOptions {
        PlainTextOptions {
            unicode_styles: false,
            bullet: "•".to_string(),
            todo_unchecked: "☐".to_string(),
            todo_checked: "☑".to_string(),
            links: PlainTextLinks::default(),
            indent: "  ".to_string(),
            quote_prefix: "> ".to_string(),
            divider: "* * *".to_string(),
            part_titles: PartTitles::default(),
        }
    }
}

impl PlainTextOptions {
    /// The defaults with [`PlainTextOptions::unicode_styles`] on, for
    /// LinkedIn and X posts.
    pub fn social() -> PlainTextOptions {
        PlainTextOptions {
            unicode_styles: true,
            ..PlainTextOptions::default()
        }
    }
}

/// Exports `source` as plain text. The result ends with exactly one
/// newline, or is empty when there is no text at all.
pub fn to_plain_text(source: &ExportSource<'_>, options: &PlainTextOptions) -> String {
    let mut parts: Vec<String> = Vec::new();
    for part in source.parts() {
        let mut chunks = Vec::new();
        if part_needs_title(source, part, options.part_titles) {
            chunks.push(Chunk {
                text: style_text(&part.title, options.unicode_styles, false),
                list: false,
            });
        }
        document_chunks(part.document, options, &mut chunks);
        let text = join_chunks(&chunks);
        if !text.is_empty() {
            parts.push(text);
        }
    }
    if parts.is_empty() {
        return String::new();
    }
    let separator = if options.divider.trim().is_empty() {
        "\n\n".to_string()
    } else {
        format!("\n\n{}\n\n", options.divider.trim())
    };
    let mut out = parts.join(&separator);
    out.push('\n');
    out
}

struct Chunk {
    text: String,
    list: bool,
}

fn join_chunks(chunks: &[Chunk]) -> String {
    let mut out = String::new();
    for (i, c) in chunks.iter().enumerate() {
        if i > 0 {
            out.push_str(if c.list && chunks[i - 1].list {
                "\n"
            } else {
                "\n\n"
            });
        }
        out.push_str(&c.text);
    }
    out
}

fn document_chunks(doc: &Document, options: &PlainTextOptions, chunks: &mut Vec<Chunk>) {
    let ordinals = ordinals(doc);
    for (i, block) in doc.blocks().iter().enumerate() {
        let text = match block.kind() {
            BlockKind::Divider => options.divider.trim().to_string(),
            BlockKind::PageBreak => String::new(),
            BlockKind::CodeBlock { .. } => {
                let code = block.text().trim_end_matches(['\n', ' ', '\t']);
                code.split('\n')
                    .map(str::trim_end)
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            kind => {
                let heading = kind.heading_level().is_some();
                let inline = inline_text(block, options, heading);
                let lines = trimmed_lines(&inline);
                if lines.is_empty() && !is_list_item(block) {
                    String::new()
                } else {
                    let level = options.indent.repeat(block.indent() as usize);
                    let marker = match kind {
                        BlockKind::BulletItem => format!("{} ", options.bullet),
                        BlockKind::NumberedItem => format!("{}. ", ordinals[i].unwrap_or(1)),
                        BlockKind::TodoItem { checked: true } => {
                            format!("{} ", options.todo_checked)
                        }
                        BlockKind::TodoItem { checked: false } => {
                            format!("{} ", options.todo_unchecked)
                        }
                        _ => String::new(),
                    };
                    let (first, rest) = match kind {
                        BlockKind::Quote => {
                            (options.quote_prefix.clone(), options.quote_prefix.clone())
                        }
                        k if k.is_list_item() => (
                            format!("{level}{marker}"),
                            format!("{level}{}", " ".repeat(marker.chars().count())),
                        ),
                        _ => (String::new(), String::new()),
                    };
                    let lines = if lines.is_empty() { vec![""] } else { lines };
                    lines
                        .iter()
                        .enumerate()
                        .map(|(n, l)| {
                            format!("{}{l}", if n == 0 { &first } else { &rest })
                                .trim_end()
                                .to_string()
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                }
            }
        };
        if !text.is_empty() {
            chunks.push(Chunk {
                text,
                list: is_list_item(block),
            });
        }
    }
}

/// The text of a block with links written out and, optionally, Unicode
/// styles applied.
fn inline_text(block: &Block, options: &PlainTextOptions, heading: bool) -> String {
    let mut out = String::new();
    let spans: Vec<_> = block.text_spans().collect();
    let mut i = 0;
    while i < spans.len() {
        let link = spans[i].2.link.as_deref().filter(|u| !u.trim().is_empty());
        let Some(url) = link else {
            let a = spans[i].2;
            push_styled(
                &mut out,
                spans[i].1,
                options,
                a.bold || heading,
                a.italic,
                a.code,
            );
            i += 1;
            continue;
        };
        // All consecutive spans with the same target are one link.
        let mut text = String::new();
        let mut plain = String::new();
        while i < spans.len() && spans[i].2.link.as_deref() == Some(url) {
            let a = spans[i].2;
            push_styled(
                &mut text,
                spans[i].1,
                options,
                a.bold || heading,
                a.italic,
                a.code,
            );
            plain.push_str(spans[i].1);
            i += 1;
        }
        let url = url.trim();
        let label = plain.trim();
        let same = label.is_empty()
            || label == url
            || url.strip_prefix("mailto:") == Some(label)
            || ["https://", "http://"].iter().any(|s| {
                url.strip_prefix(s).map(|u| u.trim_end_matches('/'))
                    == Some(label.trim_end_matches('/'))
            });
        let lead = &plain[..plain.len() - plain.trim_start().len()];
        let trail = &plain[plain.trim_end().len()..];
        match options.links {
            PlainTextLinks::TextOnly => out.push_str(&text),
            PlainTextLinks::UrlOnly => {
                out.push_str(lead);
                out.push_str(url);
                out.push_str(trail);
            }
            PlainTextLinks::TextAndUrl if same => {
                out.push_str(lead);
                out.push_str(if url.starts_with("mailto:") {
                    label
                } else {
                    url
                });
                out.push_str(trail);
            }
            PlainTextLinks::TextAndUrl => {
                out.push_str(text.trim_end());
                out.push_str(" (");
                out.push_str(url);
                out.push(')');
                out.push_str(trail);
            }
        }
    }
    out
}

fn push_styled(
    out: &mut String,
    text: &str,
    options: &PlainTextOptions,
    bold: bool,
    italic: bool,
    code: bool,
) {
    if options.unicode_styles && !code && (bold || italic) {
        out.push_str(&style_text(text, bold, italic));
    } else {
        out.push_str(text);
    }
}

/// `text` with ASCII letters and digits replaced by their mathematical
/// sans-serif bold, italic or bold italic forms.
///
/// There are no italic digits in Unicode: they stay plain in italic text and
/// are bold in bold italic text. Characters without a styled form (accented
/// letters, other scripts, punctuation) are returned unchanged.
pub fn unicode_styled(text: &str, bold: bool, italic: bool) -> String {
    style_text(text, bold, italic)
}

fn style_text(text: &str, bold: bool, italic: bool) -> String {
    if !bold && !italic {
        return text.to_string();
    }
    // Start of the capital letters, small letters and digits of each style.
    let (upper, lower, digit): (u32, u32, Option<u32>) = match (bold, italic) {
        (true, false) => (0x1D5D4, 0x1D5EE, Some(0x1D7EC)),
        (false, true) => (0x1D608, 0x1D622, None),
        _ => (0x1D63C, 0x1D656, Some(0x1D7EC)),
    };
    text.chars()
        .map(|c| {
            let mapped = match c {
                'A'..='Z' => Some(upper + (c as u32 - 'A' as u32)),
                'a'..='z' => Some(lower + (c as u32 - 'a' as u32)),
                '0'..='9' => digit.map(|d| d + (c as u32 - '0' as u32)),
                _ => None,
            };
            mapped.and_then(char::from_u32).unwrap_or(c)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use writer_core::{InlineAttrs, InlineStyle};

    fn plain(blocks: Vec<Block>) -> String {
        let doc = Document::from_blocks(blocks);
        to_plain_text(&ExportSource::from(&doc), &PlainTextOptions::default())
    }

    fn link(url: &str) -> InlineAttrs {
        InlineAttrs {
            link: Some(url.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn paragraphs_and_blank_lines() {
        assert_eq!(plain(vec![]), "");
        assert_eq!(plain(vec![Block::paragraph("")]), "");
        assert_eq!(plain(vec![Block::paragraph("  \n ")]), "");
        assert_eq!(
            plain(vec![
                Block::paragraph("One"),
                Block::paragraph(""),
                Block::paragraph(""),
                Block::paragraph("  Two  \n  three "),
            ]),
            "One\n\nTwo\nthree\n"
        );
    }

    #[test]
    fn every_block_kind() {
        let text = plain(vec![
            Block::with_text(BlockKind::heading(1), "Title"),
            Block::paragraph("Intro"),
            Block::with_text(BlockKind::BulletItem, "a"),
            Block::with_text(BlockKind::BulletItem, "b\nmore").with_indent(1),
            Block::with_text(BlockKind::NumberedItem, "one"),
            Block::with_text(BlockKind::NumberedItem, "two"),
            Block::with_text(BlockKind::NumberedItem, "nested").with_indent(2),
            Block::with_text(BlockKind::NumberedItem, "three"),
            Block::with_text(BlockKind::TodoItem { checked: false }, "open"),
            Block::with_text(BlockKind::TodoItem { checked: true }, "done"),
            Block::with_text(BlockKind::Quote, "wise\nwords"),
            Block::with_text(BlockKind::code(), "fn main() {\n    body();\n}\n"),
            Block::new(BlockKind::Divider),
            Block::new(BlockKind::PageBreak),
            Block::paragraph("End"),
        ]);
        assert_eq!(
            text,
            "Title\n\nIntro\n\n• a\n  • b\n    more\n1. one\n2. two\n    1. nested\n3. three\n☐ open\n☑ done\n\n\
             > wise\n> words\n\nfn main() {\n    body();\n}\n\n* * *\n\nEnd\n"
        );
    }

    #[test]
    fn void_blocks_first_and_last() {
        assert_eq!(
            plain(vec![
                Block::new(BlockKind::Divider),
                Block::paragraph("x"),
                Block::new(BlockKind::PageBreak),
            ]),
            "* * *\n\nx\n"
        );
        assert_eq!(plain(vec![Block::new(BlockKind::PageBreak)]), "");
    }

    #[test]
    fn links() {
        let block = || {
            Block::paragraph("See ")
                .with_span("the ", link("https://example.com/a"))
                .with_span(
                    "docs ",
                    InlineAttrs {
                        bold: true,
                        ..link("https://example.com/a")
                    },
                )
                .with_span("now", InlineAttrs::default())
        };
        assert_eq!(
            plain(vec![block()]),
            "See the docs (https://example.com/a) now\n"
        );
        let doc = Document::from_blocks(vec![block()]);
        let mut o = PlainTextOptions {
            links: PlainTextLinks::TextOnly,
            ..Default::default()
        };
        assert_eq!(
            to_plain_text(&ExportSource::from(&doc), &o),
            "See the docs now\n"
        );
        o.links = PlainTextLinks::UrlOnly;
        assert_eq!(
            to_plain_text(&ExportSource::from(&doc), &o),
            "See https://example.com/a now\n"
        );

        // The text is the URL, with or without the scheme.
        assert_eq!(
            plain(vec![Block::new(BlockKind::Paragraph).with_span(
                "https://example.com",
                link("https://example.com")
            )]),
            "https://example.com\n"
        );
        assert_eq!(
            plain(vec![
                Block::new(BlockKind::Paragraph)
                    .with_span("example.com", link("https://example.com/"))
            ]),
            "https://example.com/\n"
        );
        assert_eq!(
            plain(vec![
                Block::new(BlockKind::Paragraph)
                    .with_span("me@example.com", link("mailto:me@example.com"))
            ]),
            "me@example.com\n"
        );
        // Two different links side by side stay two links.
        assert_eq!(
            plain(vec![
                Block::new(BlockKind::Paragraph)
                    .with_span("a", link("u:1"))
                    .with_span("b", link("u:2"))
            ]),
            "a (u:1)b (u:2)\n"
        );
    }

    #[test]
    fn unicode_styles() {
        assert_eq!(unicode_styled("Bold 123", true, false), "𝗕𝗼𝗹𝗱 𝟭𝟮𝟯");
        assert_eq!(unicode_styled("Italic 1", false, true), "𝘐𝘵𝘢𝘭𝘪𝘤 1");
        assert_eq!(unicode_styled("Both 9", true, true), "𝘽𝙤𝙩𝙝 𝟵");
        assert_eq!(unicode_styled("plain", false, false), "plain");
        assert_eq!(unicode_styled("é, 日本 — ok", true, false), "é, 日本 — 𝗼𝗸");

        let doc = Document::from_blocks(vec![
            Block::with_text(BlockKind::heading(2), "Big News"),
            Block::paragraph("We ")
                .with_span("shipped", InlineAttrs::default().with(InlineStyle::Bold))
                .with_span(" it ", InlineAttrs::default())
                .with_span("today", InlineAttrs::default().with(InlineStyle::Italic))
                .with_span(" with ", InlineAttrs::default())
                .with_span(
                    "code()",
                    InlineAttrs::default()
                        .with(InlineStyle::Code)
                        .with(InlineStyle::Bold),
                ),
        ]);
        let source = ExportSource::from(&doc);
        assert_eq!(
            to_plain_text(&source, &PlainTextOptions::social()),
            "𝗕𝗶𝗴 𝗡𝗲𝘄𝘀\n\nWe 𝘀𝗵𝗶𝗽𝗽𝗲𝗱 it 𝘵𝘰𝘥𝘢𝘺 with code()\n"
        );
        assert_eq!(
            to_plain_text(&source, &PlainTextOptions::default()),
            "Big News\n\nWe shipped it today with code()\n"
        );
    }

    #[test]
    fn hostile_text_is_untouched() {
        let s = "# not *a* `heading` [x](y) <b> &amp; 👨‍👩‍👧 שלום עולם \\ _";
        assert_eq!(plain(vec![Block::paragraph(s)]), format!("{s}\n"));
        assert_eq!(
            plain(vec![Block::with_text(BlockKind::BulletItem, "")]),
            "•\n"
        );
    }

    #[test]
    fn deeply_nested_lists() {
        let blocks: Vec<Block> = (0..=8u8)
            .map(|i| {
                let kind = match i % 3 {
                    0 => BlockKind::BulletItem,
                    1 => BlockKind::NumberedItem,
                    _ => BlockKind::TodoItem {
                        checked: i % 2 == 0,
                    },
                };
                Block::with_text(kind, &format!("level {i}")).with_indent(i)
            })
            .collect();
        let text = plain(blocks);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 9);
        assert_eq!(lines[0], "• level 0");
        assert_eq!(lines[1], "  1. level 1");
        assert_eq!(lines[2], "    ☑ level 2");
        assert_eq!(lines[8], format!("{}☑ level 8", " ".repeat(16)));
    }

    #[test]
    fn right_to_left_and_emoji() {
        let text = plain(vec![
            Block::with_text(BlockKind::BulletItem, "שלום עולם"),
            Block::with_text(BlockKind::NumberedItem, "مرحبا 👨‍👩‍👧‍👦"),
            Block::with_text(BlockKind::Quote, "日本語 😀"),
        ]);
        assert_eq!(text, "• שלום עולם\n1. مرحبا 👨‍👩‍👧‍👦\n\n> 日本語 😀\n");
    }

    #[test]
    fn every_attribute_at_once() {
        let all = InlineAttrs {
            bold: true,
            italic: true,
            underline: true,
            strikethrough: true,
            code: false,
            highlight: Some(writer_core::Highlight::Pink),
            color: Some(writer_core::Rgb::new(1, 2, 3)),
            font: Some("Menlo".into()),
            link: Some("https://x.test".into()),
            ..Default::default()
        };
        let doc = Document::from_blocks(vec![
            Block::new(BlockKind::Paragraph).with_span("All 1", all),
        ]);
        let source = ExportSource::from(&doc);
        assert_eq!(
            to_plain_text(&source, &PlainTextOptions::default()),
            "All 1 (https://x.test)\n"
        );
        assert_eq!(
            to_plain_text(&source, &PlainTextOptions::social()),
            "𝘼𝙡𝙡 𝟭 (https://x.test)\n"
        );
    }

    #[test]
    fn collections() {
        let a = Document::from_blocks(vec![
            Block::with_text(BlockKind::heading(1), "One"),
            Block::paragraph("First."),
        ]);
        let b = Document::from_blocks(vec![Block::paragraph("Second.")]);
        let empty = Document::new();
        let source =
            ExportSource::collection("Book", [("One", &a), ("Empty", &empty), ("Two", &b)]);
        assert_eq!(
            to_plain_text(&source, &PlainTextOptions::default()),
            "One\n\nFirst.\n\n* * *\n\nEmpty\n\n* * *\n\nTwo\n\nSecond.\n"
        );
        let o = PlainTextOptions {
            part_titles: PartTitles::Never,
            divider: String::new(),
            ..Default::default()
        };
        assert_eq!(to_plain_text(&source, &o), "One\n\nFirst.\n\nSecond.\n");
        assert_eq!(
            to_plain_text(
                &ExportSource::collection("x", Vec::<(String, &Document)>::new()),
                &o
            ),
            ""
        );
    }
}
