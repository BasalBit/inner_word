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
    /// A `mailto:` or `tel:` link is written as its address, and a space
    /// inside a URL as `%20`.
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
    /// characters stay as they are. Links, e-mail addresses, `@mentions`
    /// and `#hashtags` in styled text keep their plain letters, so they
    /// still work. Off by default: screen readers and search do not treat
    /// these characters as letters.
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
        let label = plain.trim();
        let same = label.is_empty() || {
            let url = url.trim();
            label == url
                || address(url) == Some(label)
                || ["https://", "http://"].iter().any(|s| {
                    url.strip_prefix(s).map(|u| u.trim_end_matches('/'))
                        == Some(label.trim_end_matches('/'))
                })
        };
        let url = &written_url(url);
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
                out.push_str(url);
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

/// The address of a `mailto:` or `tel:` target: what a reader of plain
/// text can use.
fn address(url: &str) -> Option<&str> {
    ["mailto:", "tel:"].iter().find_map(|scheme| {
        let head = url.get(..scheme.len())?;
        head.eq_ignore_ascii_case(scheme)
            .then(|| url[scheme.len()..].split('?').next().unwrap_or("").trim())
            .filter(|a| !a.is_empty())
    })
}

/// A link target as plain text shows it: a `mailto:` or `tel:` target is
/// its address, and white space inside any other target is
/// percent-encoded, because a URL ends at the first space wherever plain
/// text is turned into links.
fn written_url(url: &str) -> String {
    let url = url.trim();
    if let Some(address) = address(url) {
        return address.to_string();
    }
    let mut out = String::with_capacity(url.len());
    for c in url.chars() {
        match c {
            ' ' | '\u{a0}' => out.push_str("%20"),
            '\t' => out.push_str("%09"),
            '\n' | '\r' => {}
            c => out.push(c),
        }
    }
    out
}

/// Whether a word must keep its plain letters to keep working where the
/// text is posted: a link, an e-mail address, a `@mention` or a
/// `#hashtag`. The styled alphabets are different characters, and a
/// network does not recognise a link or a tag written in them.
fn is_functional(word: &str) -> bool {
    let word = word.trim_start_matches(|c: char| !c.is_alphanumeric() && !matches!(c, '@' | '#'));
    let tag = |marker: char| {
        word.strip_prefix(marker)
            .and_then(|rest| rest.chars().next())
            .is_some_and(|c| c.is_alphanumeric() || c == '_')
    };
    tag('@')
        || tag('#')
        || !crate::social::find_links(word, true).is_empty()
        || word.split_once('@').is_some_and(|(name, host)| {
            name.chars().next_back().is_some_and(char::is_alphanumeric)
                && host.contains('.')
                && host.starts_with(|c: char| c.is_alphanumeric())
        })
}

fn push_styled(
    out: &mut String,
    text: &str,
    options: &PlainTextOptions,
    bold: bool,
    italic: bool,
    code: bool,
) {
    if !(options.unicode_styles && !code && (bold || italic)) {
        out.push_str(text);
        return;
    }
    // Word by word, so that links, addresses, mentions and hashtags in
    // styled text keep their plain letters.
    let mut rest = text;
    while !rest.is_empty() {
        let space = rest.len() - rest.trim_start().len();
        out.push_str(&rest[..space]);
        rest = &rest[space..];
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let word = &rest[..end];
        if is_functional(word) {
            out.push_str(word);
        } else {
            out.push_str(&style_text(word, bold, italic));
        }
        rest = &rest[end..];
    }
}

/// `text` with ASCII letters and digits replaced by their mathematical
/// sans-serif bold, italic or bold italic forms.
///
/// There are no italic digits in Unicode: they stay plain in italic text and
/// are bold in bold italic text. Characters without a styled form (accented
/// letters, other scripts, punctuation) are returned unchanged.
///
/// This styles every letter it is given. [`to_plain_text`] with
/// [`PlainTextOptions::unicode_styles`] leaves links, e-mail addresses,
/// `@mentions` and `#hashtags` inside bold or italic text as they are,
/// because they stop working when written in these alphabets.
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
        // An address behind other text: without the scheme.
        assert_eq!(
            plain(vec![
                Block::paragraph("Write to ")
                    .with_span("me", link("mailto:me@example.com?subject=Hi"))
                    .with_span(" or call ", InlineAttrs::default())
                    .with_span("the office", link("tel:+1-555-0100"))
            ]),
            "Write to me (me@example.com) or call the office (+1-555-0100)\n"
        );
        // A space would end the URL wherever the text is posted.
        assert_eq!(
            plain(vec![
                Block::paragraph("The ")
                    .with_span("file", link(" https://example.com/my file.pdf "))
            ]),
            "The file (https://example.com/my%20file.pdf)\n"
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
    fn unicode_styles_leave_links_mentions_and_hashtags_working() {
        let bold = || InlineAttrs::default().with(InlineStyle::Bold);
        let social = |blocks: Vec<Block>| {
            let doc = Document::from_blocks(blocks);
            to_plain_text(&ExportSource::from(&doc), &PlainTextOptions::social())
        };
        assert_eq!(
            social(vec![Block::new(BlockKind::Paragraph).with_span(
                "Read https://example.com/Post_1 now, thanks @ada_l and #RustLang!",
                bold()
            )]),
            "𝗥𝗲𝗮𝗱 https://example.com/Post_1 𝗻𝗼𝘄, 𝘁𝗵𝗮𝗻𝗸𝘀 @ada_l 𝗮𝗻𝗱 #RustLang!\n"
        );
        // Bare domains, addresses, and tags behind punctuation.
        assert_eq!(
            social(vec![Block::new(BlockKind::Paragraph).with_span(
                "See example.com/a (or www.example.org), mail me@example.com. Tags: (#one) “@two”",
                InlineAttrs::default().with(InlineStyle::Italic)
            )]),
            "𝘚𝘦𝘦 example.com/a (𝘰𝘳 www.example.org), 𝘮𝘢𝘪𝘭 me@example.com. 𝘛𝘢𝘨𝘴: (#one) “@two”\n"
        );
        // A heading is bold: its link and tag survive too.
        assert_eq!(
            social(vec![Block::with_text(
                BlockKind::heading(1),
                "Launch #1 at https://example.com"
            )]),
            "𝗟𝗮𝘂𝗻𝗰𝗵 #1 𝗮𝘁 https://example.com\n"
        );
        // Not tags: a lone marker, a marker inside a word, a price.
        assert_eq!(
            social(vec![
                Block::new(BlockKind::Paragraph).with_span("# 1 a#b @ c 5.5 v1.2", bold())
            ]),
            "# 𝟭 𝗮#𝗯 @ 𝗰 𝟱.𝟱 𝘃𝟭.𝟮\n"
        );
        // A bold link whose text is its URL is the plain URL.
        assert_eq!(
            social(vec![Block::new(BlockKind::Paragraph).with_span(
                "https://example.com/x",
                InlineAttrs {
                    bold: true,
                    ..link("https://example.com/x")
                }
            )]),
            "https://example.com/x\n"
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
