//! Markdown export tests: exact output for every construct, and a round
//! trip through an independent CommonMark parser (pulldown-cmark) that must
//! give back the structure and the text of the document.

use std::borrow::Cow;
use std::collections::BTreeMap;

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use innerword_core::{
    Alignment, Block, BlockKind, Document, EditorState, Highlight, Image, ImageId, ImageWidth,
    InlineAttrs, InlineStyle, Position, Rgb, Selection,
};
use innerword_export::{
    ExportAsset, ExportFormat, ExportOptions, ExportSource, ImageData, ImageFileFormat,
    ImageSource, MarkdownComments, MarkdownHighlight, MarkdownImages, MarkdownOptions,
    MarkdownPageBreak, PartTitles, export, export_package, images_dir_name, to_markdown,
    to_plain_text,
};

// ----- builders ---------------------------------------------------------------------------

fn plain() -> InlineAttrs {
    InlineAttrs::default()
}
fn bold() -> InlineAttrs {
    plain().with(InlineStyle::Bold)
}
fn italic() -> InlineAttrs {
    plain().with(InlineStyle::Italic)
}
fn strike() -> InlineAttrs {
    plain().with(InlineStyle::Strikethrough)
}
fn code() -> InlineAttrs {
    plain().with(InlineStyle::Code)
}
fn underline() -> InlineAttrs {
    plain().with(InlineStyle::Underline)
}
fn link(url: &str) -> InlineAttrs {
    InlineAttrs {
        link: Some(url.to_string()),
        ..plain()
    }
}
fn para() -> Block {
    Block::new(BlockKind::Paragraph)
}
fn p(spans: &[(&str, InlineAttrs)]) -> Block {
    spans
        .iter()
        .fold(para(), |b, (t, a)| b.with_span(t, a.clone()))
}
fn md_with(blocks: Vec<Block>, options: &MarkdownOptions) -> String {
    let doc = Document::from_blocks(blocks);
    to_markdown(&ExportSource::from(&doc), options)
}
fn md(blocks: Vec<Block>) -> String {
    md_with(blocks, &MarkdownOptions::default())
}
fn one(spans: &[(&str, InlineAttrs)]) -> String {
    let out = md(vec![p(spans)]);
    out.strip_suffix('\n').unwrap_or(&out).to_string()
}

/// An image id made for a test: `first` in hex, then zeros up to 32
/// digits (`"3f2a"` is `3f2a0000…`).
fn image_id(first: &str) -> ImageId {
    ImageId::parse(&format!("{first:0<32}")).expect("hex digits")
}

/// An image of 1600 × 1200 pixels at 144 to the inch (800 × 600 points)
/// with a description, as wide as the column.
fn image(first: &str, description: &str) -> Image {
    Image {
        description: description.to_string(),
        ..Image::new(image_id(first), (1600, 1200), 144)
    }
}

fn image_block(first: &str, description: &str) -> Block {
    Block::image(image(first, description))
}

/// The stored images of a test: what a library would hold.
#[derive(Default)]
struct Pictures(BTreeMap<ImageId, (Vec<u8>, ImageFileFormat)>);

impl Pictures {
    fn with(mut self, first: &str, format: ImageFileFormat, bytes: &[u8]) -> Pictures {
        self.0.insert(image_id(first), (bytes.to_vec(), format));
        self
    }
}

impl ImageSource for Pictures {
    fn image(&self, id: ImageId) -> Option<ImageData<'_>> {
        self.0.get(&id).map(|(bytes, format)| ImageData {
            bytes: Cow::Borrowed(bytes),
            format: *format,
        })
    }
}

// ----- the parser side: Markdown to a flat, comparable form -------------------------------

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Style {
    bold: bool,
    italic: bool,
    strike: bool,
    code: bool,
    underline: bool,
    highlight: bool,
    link: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Kind {
    Para,
    Heading(u8),
    Bullet,
    Numbered(u64),
    Todo(bool),
    Quote,
    Code(String),
    Rule,
    Html(String),
    /// An image alone in its paragraph, with its destination. Its text is
    /// the alternative text.
    Image(String),
}

/// One block of the document in reading order: its kind, how many list
/// items enclose it, and its characters with their styles.
#[derive(Clone, PartialEq, Eq)]
struct Line {
    kind: Kind,
    depth: usize,
    text: Vec<(char, Style)>,
}

impl std::fmt::Debug for Line {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?} depth {} ", self.kind, self.depth)?;
        let mut i = 0;
        while i < self.text.len() {
            let style = &self.text[i].1;
            let mut s = String::new();
            while i < self.text.len() && &self.text[i].1 == style {
                s.push(self.text[i].0);
                i += 1;
            }
            let mut tags = String::new();
            for (on, t) in [
                (style.bold, 'B'),
                (style.italic, 'I'),
                (style.strike, 'S'),
                (style.code, 'C'),
                (style.underline, 'U'),
                (style.highlight, 'H'),
            ] {
                if on {
                    tags.push(t);
                }
            }
            if let Some(l) = &style.link {
                tags.push_str(&format!("L<{l}>"));
            }
            write!(f, "[{tags}|{s:?}]")?;
        }
        Ok(())
    }
}

/// Styles on whitespace cannot be seen and are not compared.
fn normalize(text: &mut [(char, Style)]) {
    for (c, s) in text.iter_mut() {
        if c.is_whitespace() && !s.code {
            *s = Style::default();
        }
    }
}

fn parser_options() -> Options {
    Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
}

fn parse(markdown: &str) -> Vec<Line> {
    let mut lines: Vec<Line> = Vec::new();
    let mut cur: Option<usize> = None;
    let mut fresh_item = false;
    let mut lists: Vec<Option<u64>> = Vec::new();
    let mut items = 0usize;
    let mut quotes = 0usize;
    let (mut bold, mut italic, mut strike, mut underline, mut highlight) =
        (0i32, 0i32, 0i32, 0i32, 0i32);
    let mut html_code = 0i32;
    let mut links: Vec<String> = Vec::new();

    let mut in_footnote = false;
    let mut in_image = false;
    for event in Parser::new_ext(markdown, parser_options()) {
        // Footnote definitions hold comments, not document text.
        match event {
            Event::Start(Tag::FootnoteDefinition(_)) => in_footnote = true,
            Event::End(TagEnd::FootnoteDefinition) => {
                in_footnote = false;
                continue;
            }
            _ => {}
        }
        if in_footnote {
            continue;
        }
        let style = |code: bool| Style {
            bold: bold > 0,
            italic: italic > 0,
            strike: strike > 0,
            code,
            underline: underline > 0,
            highlight: highlight > 0,
            link: links.last().cloned(),
        };
        let start = |lines: &mut Vec<Line>, kind: Kind| {
            lines.push(Line {
                kind,
                depth: items,
                text: Vec::new(),
            });
            lines.len() - 1
        };
        match event {
            Event::Start(Tag::Paragraph) => {
                if fresh_item && cur.is_some() {
                    fresh_item = false;
                } else {
                    let kind = if quotes > 0 { Kind::Quote } else { Kind::Para };
                    cur = Some(start(&mut lines, kind));
                }
            }
            Event::End(TagEnd::Paragraph) => cur = None,
            Event::Start(Tag::Heading { level, .. }) => {
                cur = Some(start(&mut lines, Kind::Heading(level as u8)));
            }
            Event::End(TagEnd::Heading(_)) => cur = None,
            Event::Start(Tag::BlockQuote(_)) => {
                quotes += 1;
                fresh_item = false;
                cur = None;
            }
            Event::End(TagEnd::BlockQuote(_)) => quotes -= 1,
            Event::Start(Tag::CodeBlock(kind)) => {
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => info.to_string(),
                    CodeBlockKind::Indented => "<indented>".to_string(),
                };
                fresh_item = false;
                cur = Some(start(&mut lines, Kind::Code(lang)));
            }
            Event::End(TagEnd::CodeBlock) => cur = None,
            Event::Start(Tag::HtmlBlock) => {
                fresh_item = false;
                cur = Some(start(&mut lines, Kind::Html(String::new())));
            }
            Event::End(TagEnd::HtmlBlock) => {
                if let Some(i) = cur {
                    let html: String = lines[i].text.drain(..).map(|(c, _)| c).collect();
                    lines[i].kind = Kind::Html(html.trim().to_string());
                }
                cur = None;
            }
            Event::Html(s) => {
                if let Some(i) = cur {
                    lines[i]
                        .text
                        .extend(s.chars().map(|c| (c, Style::default())));
                }
            }
            Event::Rule => {
                fresh_item = false;
                start(&mut lines, Kind::Rule);
                cur = None;
            }
            Event::Start(Tag::List(first)) => {
                lists.push(first);
                fresh_item = false;
                cur = None;
            }
            Event::End(TagEnd::List(_)) => {
                lists.pop();
            }
            Event::Start(Tag::Item) => {
                let kind = match lists.last_mut() {
                    Some(Some(n)) => {
                        *n += 1;
                        Kind::Numbered(*n - 1)
                    }
                    _ => Kind::Bullet,
                };
                cur = Some(start(&mut lines, kind));
                items += 1;
                fresh_item = true;
            }
            Event::End(TagEnd::Item) => {
                items -= 1;
                cur = None;
                fresh_item = false;
            }
            Event::TaskListMarker(done) => {
                if let Some(i) = cur {
                    lines[i].kind = Kind::Todo(done);
                }
            }
            Event::Start(Tag::Strong) => bold += 1,
            Event::End(TagEnd::Strong) => bold -= 1,
            Event::Start(Tag::Emphasis) => italic += 1,
            Event::End(TagEnd::Emphasis) => italic -= 1,
            Event::Start(Tag::Strikethrough) => strike += 1,
            Event::End(TagEnd::Strikethrough) => strike -= 1,
            Event::Start(Tag::Link { dest_url, .. }) => links.push(dest_url.to_string()),
            Event::End(TagEnd::Link) => {
                links.pop();
            }
            // The exporter writes an image as a paragraph that holds
            // nothing else.
            Event::Start(Tag::Image {
                dest_url, title, ..
            }) => {
                let i = cur.unwrap_or_else(|| panic!("an image outside a block in:\n{markdown}"));
                assert!(
                    lines[i].kind == Kind::Para && lines[i].text.is_empty() && title.is_empty(),
                    "an image that is not alone in its paragraph in:\n{markdown}"
                );
                lines[i].kind = Kind::Image(dest_url.to_string());
                in_image = true;
            }
            Event::End(TagEnd::Image) => in_image = false,
            Event::InlineHtml(tag) => match &*tag {
                "<u>" => underline += 1,
                "</u>" => underline -= 1,
                "<mark>" => highlight += 1,
                "</mark>" => highlight -= 1,
                "<strong>" => bold += 1,
                "</strong>" => bold -= 1,
                "<em>" => italic += 1,
                "</em>" => italic -= 1,
                "<code>" => html_code += 1,
                "</code>" => html_code -= 1,
                "<del>" => strike += 1,
                "</del>" => strike -= 1,
                t if t.starts_with("<span ") || t == "</span>" || t.starts_with("<!--") => {}
                other => panic!("unexpected inline HTML {other:?} in:\n{markdown}"),
            },
            Event::Text(s) => {
                let i = cur.unwrap_or_else(|| panic!("text {s:?} outside a block in:\n{markdown}"));
                assert!(
                    in_image || !matches!(lines[i].kind, Kind::Image(_)),
                    "text {s:?} beside an image in:\n{markdown}"
                );
                let st = style(html_code > 0);
                lines[i].text.extend(s.chars().map(|c| (c, st.clone())));
            }
            Event::Code(s) => {
                let i = cur.unwrap_or_else(|| panic!("code {s:?} outside a block in:\n{markdown}"));
                let st = style(true);
                lines[i].text.extend(s.chars().map(|c| (c, st.clone())));
            }
            Event::HardBreak => {
                if let Some(i) = cur {
                    lines[i].text.push(('\n', Style::default()));
                }
            }
            Event::SoftBreak => panic!("soft break in:\n{markdown}"),
            Event::FootnoteReference(_) => {}
            other => panic!("unexpected event {other:?} in:\n{markdown}"),
        }
    }
    for l in &mut lines {
        normalize(&mut l.text);
    }
    lines
}

// ----- the document side: what the parser should find -------------------------------------

fn style_of(attrs: &InlineAttrs, options: &MarkdownOptions) -> Style {
    Style {
        bold: attrs.bold,
        italic: attrs.italic,
        strike: attrs.strikethrough,
        code: attrs.code,
        underline: attrs.underline && options.underline_html,
        highlight: attrs.highlight.is_some() && options.highlight == MarkdownHighlight::Html,
        link: attrs
            .link
            .as_deref()
            .map(str::trim)
            .filter(|u| !u.is_empty())
            .map(expected_destination),
    }
}

/// A link target as a parser reports it: the exporter percent-encodes
/// whitespace and control characters, which a destination cannot hold.
fn expected_destination(url: &str) -> String {
    let mut out = String::new();
    for c in url.chars() {
        if c.is_whitespace() || c.is_control() {
            let mut buf = [0u8; 4];
            for b in c.encode_utf8(&mut buf).bytes() {
                out.push_str(&format!("%{b:02X}"));
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn block_text(block: &Block, options: &MarkdownOptions, heading: bool) -> Vec<(char, Style)> {
    let mut cells: Vec<(char, Style)> = Vec::new();
    for (_, text, attrs) in block.text_spans() {
        let style = style_of(attrs, options);
        for c in text.chars() {
            let mut s = style.clone();
            if c == '\n' {
                s = Style::default();
            }
            cells.push((c, s));
        }
    }
    // Trim spaces and tabs at line edges, then empty lines at both ends.
    let trimmable = |c: &(char, Style)| !c.1.code && c.0.is_whitespace();
    let mut out: Vec<(char, Style)> = Vec::new();
    for line in cells.split(|c| c.0 == '\n') {
        let a = line.iter().take_while(|c| trimmable(c)).count();
        let b = line.len() - line[a..].iter().rev().take_while(|c| trimmable(c)).count();
        out.extend_from_slice(&line[a..b]);
        out.push(('\n', Style::default()));
    }
    while out.last().is_some_and(|c| c.0 == '\n') {
        out.pop();
    }
    let lead = out.iter().take_while(|c| c.0 == '\n').count();
    out.drain(..lead);
    if heading {
        let mut joined: Vec<(char, Style)> = Vec::new();
        for c in out {
            if c.0 == '\n' {
                if joined.last().is_some_and(|l| l.0 == ' ' && !l.1.code) {
                    continue;
                }
                joined.push((' ', Style::default()));
            } else {
                joined.push(c);
            }
        }
        out = joined;
    }
    normalize(&mut out);
    out
}

fn expected(doc: &Document, options: &MarkdownOptions) -> Vec<Line> {
    let ordinals = doc.numbered_ordinals(0..doc.block_count());
    let mut stack: Vec<u8> = Vec::new();
    let mut lines = Vec::new();
    for (i, block) in doc.blocks().iter().enumerate() {
        while stack.last().is_some_and(|l| *l >= block.indent()) {
            stack.pop();
        }
        let depth = stack.len();
        let kind = match block.kind() {
            BlockKind::Paragraph => Kind::Para,
            BlockKind::Heading { level } => Kind::Heading(*level),
            BlockKind::BulletItem => Kind::Bullet,
            BlockKind::NumberedItem => Kind::Numbered(ordinals[i].unwrap() as u64),
            BlockKind::TodoItem { checked } => Kind::Todo(*checked),
            BlockKind::Quote => Kind::Quote,
            BlockKind::CodeBlock { language } => {
                // The info string is the first word of the language, without
                // characters that mean something in an info string.
                let word = language
                    .as_deref()
                    .unwrap_or("")
                    .split_whitespace()
                    .next()
                    .unwrap_or("");
                Kind::Code(
                    word.chars()
                        .filter(|c| c.is_alphanumeric() || "+-_#.".contains(*c))
                        .collect(),
                )
            }
            BlockKind::Divider => Kind::Rule,
            BlockKind::PageBreak => match options.page_break {
                MarkdownPageBreak::Comment => Kind::Html("<!-- pagebreak -->".to_string()),
                MarkdownPageBreak::Rule => Kind::Rule,
                MarkdownPageBreak::Html => {
                    Kind::Html("<div style=\"page-break-after: always\"></div>".to_string())
                }
                MarkdownPageBreak::Omit => continue,
            },
            // A reference to the stored image: no test that compares with
            // this writes picture files.
            BlockKind::Image(image) => Kind::Image(format!("image:{}", image.id)),
        };
        let text = match block.kind() {
            BlockKind::CodeBlock { .. } => {
                let mut t = block.text().to_string();
                if !t.is_empty() {
                    t.push('\n');
                }
                t.chars().map(|c| (c, Style::default())).collect()
            }
            BlockKind::Image(image) => {
                let mut text: Vec<(char, Style)> = image
                    .description
                    .chars()
                    .map(|c| (c, Style::default()))
                    .collect();
                normalize(&mut text);
                text
            }
            k => block_text(block, options, k.heading_level().is_some()),
        };
        if block.kind().is_list_item() {
            stack.push(block.indent());
            lines.push(Line { kind, depth, text });
            continue;
        }
        let blank = !block.is_void() && !block.kind().is_code() && block.text().trim().is_empty();
        if !blank {
            lines.push(Line { kind, depth, text });
        }
    }
    // A paragraph that is the first child of an item without text is, to a
    // Markdown parser, the text of that item.
    let mut i = 0;
    while i + 1 < lines.len() {
        // (Not of a to-do: its marker is text already, so its children are
        // written after a blank line and stay children.)
        let item = matches!(lines[i].kind, Kind::Bullet | Kind::Numbered(_));
        if item
            && lines[i].text.is_empty()
            && lines[i + 1].kind == Kind::Para
            && lines[i + 1].depth == lines[i].depth + 1
        {
            let child = lines.remove(i + 1);
            lines[i].text = child.text;
        }
        i += 1;
    }
    lines
}

/// Exports `doc`, parses the result and compares it with the document.
#[track_caller]
fn round_trip_with(doc: &Document, options: &MarkdownOptions) -> String {
    let out = to_markdown(&ExportSource::from(doc), options);
    let got = parse(&out);
    let want = expected(doc, options);
    if got != want {
        let mut msg = format!("round trip mismatch\n--- markdown ---\n{out}\n--- document ---\n");
        for b in doc.blocks() {
            msg.push_str(&format!(
                "{:?} indent {} {:?}\n",
                b.kind(),
                b.indent(),
                b.text()
            ));
            for (r, a) in b.spans() {
                msg.push_str(&format!("    {r:?} {a:?}\n"));
            }
        }
        msg.push_str("--- want / got ---\n");
        for i in 0..want.len().max(got.len()) {
            let (w, g) = (want.get(i), got.get(i));
            let mark = if w == g { " " } else { "!" };
            msg.push_str(&format!("{mark} want {w:?}\n{mark} got  {g:?}\n"));
        }
        panic!("{msg}");
    }
    // Tidy output: one final newline, no trailing blank lines, no triple newlines.
    if !out.is_empty() {
        assert!(out.ends_with('\n') && !out.ends_with("\n\n"), "{out:?}");
    }
    assert!(
        !out.contains("\n\n\n") || doc.blocks().iter().any(|b| b.kind().is_code()),
        "{out:?}"
    );
    out
}

#[track_caller]
fn round_trip(blocks: Vec<Block>) -> String {
    round_trip_with(&Document::from_blocks(blocks), &MarkdownOptions::default())
}

// ----- exact output: blocks ---------------------------------------------------------------

#[test]
fn empty_documents() {
    assert_eq!(md(vec![]), "");
    assert_eq!(md(vec![para()]), "");
    assert_eq!(
        md(vec![
            para(),
            Block::paragraph("  "),
            Block::with_text(BlockKind::heading(1), "")
        ]),
        ""
    );
    assert_eq!(
        to_markdown(
            &ExportSource::from(&Document::new()),
            &MarkdownOptions::default()
        ),
        ""
    );
}

#[test]
fn headings_and_paragraphs() {
    let out = round_trip(vec![
        Block::with_text(BlockKind::heading(1), "One"),
        Block::with_text(BlockKind::heading(2), "Two"),
        Block::with_text(BlockKind::heading(6), "Six"),
        Block::paragraph("First paragraph."),
        para(),
        Block::paragraph("Second paragraph."),
    ]);
    assert_eq!(
        out,
        "# One\n\n## Two\n\n###### Six\n\nFirst paragraph.\n\nSecond paragraph.\n"
    );
}

#[test]
fn heading_edge_cases() {
    assert_eq!(
        round_trip(vec![Block::with_text(
            BlockKind::heading(2),
            "Line one\nline two"
        )]),
        "## Line one line two\n"
    );
    assert_eq!(
        round_trip(vec![Block::with_text(BlockKind::heading(1), "C# and F#")]),
        "# C# and F#\n"
    );
    assert_eq!(
        round_trip(vec![Block::with_text(BlockKind::heading(1), "Trailing #")]),
        "# Trailing \\#\n"
    );
    assert_eq!(
        round_trip(vec![Block::with_text(BlockKind::heading(3), "# hash")]),
        "### \\# hash\n"
    );
    assert_eq!(
        round_trip(vec![
            Block::new(BlockKind::heading(2))
                .with_span("Bold", bold())
                .with_span(" title", plain())
        ]),
        "## **Bold** title\n"
    );
}

#[test]
fn soft_breaks_are_hard_breaks() {
    assert_eq!(
        round_trip(vec![Block::paragraph("one\ntwo\nthree")]),
        "one\\\ntwo\\\nthree\n"
    );
    // Trailing spaces, blank edge lines and inner blank lines.
    assert_eq!(
        round_trip(vec![Block::paragraph("\n one  \n\n two \n\n")]),
        "one\\\n\\\ntwo\n"
    );
    assert_eq!(
        round_trip(vec![
            Block::with_text(BlockKind::BulletItem, "a\nb"),
            Block::with_text(BlockKind::Quote, "c\nd")
        ]),
        "- a\\\n  b\n\n> c\\\n> d\n"
    );
}

#[test]
fn lists() {
    let out = round_trip(vec![
        Block::with_text(BlockKind::BulletItem, "one"),
        Block::with_text(BlockKind::BulletItem, "two"),
        Block::with_text(BlockKind::BulletItem, "nested").with_indent(1),
        Block::with_text(BlockKind::BulletItem, "deeper").with_indent(2),
        Block::with_text(BlockKind::BulletItem, "three"),
    ]);
    assert_eq!(out, "- one\n- two\n  - nested\n    - deeper\n- three\n");

    let out = round_trip(vec![
        Block::with_text(BlockKind::NumberedItem, "one"),
        Block::with_text(BlockKind::NumberedItem, "two"),
        Block::with_text(BlockKind::NumberedItem, "two a").with_indent(1),
        Block::with_text(BlockKind::NumberedItem, "two b").with_indent(1),
        Block::with_text(BlockKind::NumberedItem, "three"),
    ]);
    assert_eq!(out, "1. one\n2. two\n   1. two a\n   2. two b\n3. three\n");

    let out = round_trip(vec![
        Block::with_text(BlockKind::TodoItem { checked: false }, "open"),
        Block::with_text(BlockKind::TodoItem { checked: true }, "done"),
        Block::with_text(BlockKind::BulletItem, "plain"),
    ]);
    assert_eq!(out, "- [ ] open\n- [x] done\n- plain\n");
}

/// Text is escaped by what is written around it, not by where formatting
/// that is not written (a colour, by default) happens to begin: the same
/// text gives the same Markdown however its runs are cut.
#[test]
fn unwritten_formatting_does_not_change_the_escapes() {
    let mut ink = plain();
    ink.color = Some(Rgb::new(10, 20, 30));
    let cut = Document::from_blocks(vec![p(&[("k", ink), ("_a_# x", plain())])]);
    let whole = Document::from_blocks(vec![p(&[("k_a_# x", plain())])]);
    let default = MarkdownOptions::default();
    assert_eq!(round_trip_with(&cut, &default), "k_a_# x\n");
    assert_eq!(round_trip_with(&whole, &default), "k_a_# x\n");
    let colors = MarkdownOptions {
        colors: true,
        ..Default::default()
    };
    assert_eq!(
        round_trip_with(&cut, &colors),
        "<span style=\"color:#0a141e\">k</span>\\_a\\_# x\n"
    );
}

/// `- |` under the text of the item above would be the delimiter row of a
/// table, with that text as its header.
#[test]
fn a_nested_item_of_pipes_is_not_a_table() {
    let out = round_trip(vec![
        Block::with_text(BlockKind::BulletItem, "header |"),
        Block::with_text(BlockKind::BulletItem, "|").with_indent(1),
        Block::with_text(BlockKind::BulletItem, "| - | :-: |").with_indent(1),
        Block::with_text(BlockKind::TodoItem { checked: false }, "a | b"),
        Block::with_text(BlockKind::BulletItem, "-|-").with_indent(1),
        Block::with_text(BlockKind::BulletItem, "| not only pipes").with_indent(1),
    ]);
    assert_eq!(
        out,
        "- header |\n  - \\|\n  - \\| - | :-: |\n- [ ] a | b\n  - \\-|-\n  - | not only pipes\n"
    );
}

#[test]
fn long_numbered_list_keeps_real_ordinals_and_alignment() {
    let mut blocks: Vec<Block> = (1..=11)
        .map(|i| Block::with_text(BlockKind::NumberedItem, &format!("item {i}")))
        .collect();
    blocks.insert(
        10,
        Block::with_text(BlockKind::BulletItem, "child").with_indent(1),
    );
    blocks.push(
        Block::with_text(BlockKind::BulletItem, "child of eleven\nsecond line").with_indent(1),
    );
    let out = round_trip(blocks);
    assert!(out.contains("\n9. item 9\n10. item 10\n    - child\n11. item 11\n    - child of eleven\\\n      second line\n"), "{out}");
}

#[test]
fn mixed_and_adjacent_lists() {
    // A different kind of list starts a new list; a blank line keeps them apart.
    let out = round_trip(vec![
        Block::with_text(BlockKind::BulletItem, "bullet"),
        Block::with_text(BlockKind::NumberedItem, "number"),
        Block::with_text(BlockKind::BulletItem, "bullet again"),
    ]);
    assert_eq!(out, "- bullet\n\n1. number\n\n- bullet again\n");

    // Two numbered lists separated only by an empty paragraph restart at 1:
    // the second takes the other delimiter so parsers do not merge them.
    let out = round_trip(vec![
        Block::with_text(BlockKind::NumberedItem, "a"),
        Block::with_text(BlockKind::NumberedItem, "b"),
        para(),
        Block::with_text(BlockKind::NumberedItem, "c"),
        Block::with_text(BlockKind::NumberedItem, "d"),
    ]);
    assert_eq!(out, "1. a\n2. b\n\n1) c\n2) d\n");

    // Nested lists of both kinds.
    let out = round_trip(vec![
        Block::with_text(BlockKind::NumberedItem, "one"),
        Block::with_text(BlockKind::BulletItem, "dot").with_indent(1),
        Block::with_text(BlockKind::TodoItem { checked: false }, "task").with_indent(2),
        Block::with_text(BlockKind::NumberedItem, "two"),
    ]);
    assert_eq!(out, "1. one\n   - dot\n     - [ ] task\n2. two\n");
}

#[test]
fn list_indent_jumps_are_clamped() {
    // An item cannot be nested two levels below its parent in Markdown.
    let out = round_trip(vec![
        Block::with_text(BlockKind::BulletItem, "deep start").with_indent(3),
        Block::with_text(BlockKind::BulletItem, "top"),
        Block::with_text(BlockKind::BulletItem, "jump").with_indent(4),
        Block::with_text(BlockKind::BulletItem, "back").with_indent(2),
        Block::with_text(BlockKind::BulletItem, "deepest").with_indent(8),
    ]);
    assert_eq!(
        out,
        "- deep start\n- top\n  - jump\n  - back\n    - deepest\n"
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
    let out = round_trip(blocks);
    assert!(
        out.starts_with("- level 0\n  1. level 1\n     - [x] level 2\n       - level 3\n"),
        "{out}"
    );
    assert_eq!(out.lines().count(), 9);
}

#[test]
fn blocks_inside_list_items() {
    let out = round_trip(vec![
        Block::with_text(BlockKind::NumberedItem, "step"),
        Block::paragraph("explanation").with_indent(1),
        Block::with_text(BlockKind::code(), "run()").with_indent(1),
        Block::with_text(BlockKind::Quote, "note").with_indent(1),
        Block::with_text(BlockKind::NumberedItem, "next step"),
        Block::paragraph("after the list"),
    ]);
    assert_eq!(
        out,
        "1. step\n\n   explanation\n\n   ```\n   run()\n   ```\n\n   > note\n\n2. next step\n\nafter the list\n"
    );
}

#[test]
fn quotes() {
    assert_eq!(
        round_trip(vec![Block::with_text(BlockKind::Quote, "one")]),
        "> one\n"
    );
    // Quote blocks in a row are one quote with several paragraphs.
    assert_eq!(
        round_trip(vec![
            Block::with_text(BlockKind::Quote, "one"),
            Block::with_text(BlockKind::Quote, "two\nlines"),
            Block::paragraph("out"),
            Block::with_text(BlockKind::Quote, "> nested marker"),
        ]),
        "> one\n>\n> two\\\n> lines\n\nout\n\n> \\> nested marker\n"
    );
}

#[test]
fn code_blocks() {
    assert_eq!(
        round_trip(vec![Block::with_text(
            BlockKind::CodeBlock {
                language: Some("rust".into())
            },
            "fn main() {\n    let x = 1;\n}"
        )]),
        "```rust\nfn main() {\n    let x = 1;\n}\n```\n"
    );
    // The fence is longer than any run of backticks inside.
    assert_eq!(
        round_trip(vec![Block::with_text(BlockKind::code(), "```\ninner\n```")]),
        "````\n```\ninner\n```\n````\n"
    );
    assert_eq!(
        round_trip(vec![Block::with_text(BlockKind::code(), "a `````` b")]),
        "```````\na `````` b\n```````\n"
    );
    // Markdown inside code is left alone; blank lines survive.
    assert_eq!(
        round_trip(vec![Block::with_text(
            BlockKind::code(),
            "# not a heading\n\n*not* <b>bold</b> &amp;\n"
        )]),
        "```\n# not a heading\n\n*not* <b>bold</b> &amp;\n\n```\n"
    );
    assert_eq!(
        round_trip(vec![Block::new(BlockKind::code())]),
        "```\n```\n"
    );
    // A hostile language tag cannot break the fence.
    let out = md(vec![Block::with_text(
        BlockKind::CodeBlock {
            language: Some("c++ `x`\n&amp;".into()),
        },
        "x",
    )]);
    assert_eq!(out, "```c++\nx\n```\n");
}

#[test]
fn dividers_and_page_breaks() {
    let blocks = || {
        vec![
            Block::new(BlockKind::Divider),
            Block::paragraph("middle"),
            Block::new(BlockKind::PageBreak),
            Block::paragraph("end"),
            Block::new(BlockKind::Divider),
        ]
    };
    assert_eq!(
        round_trip(blocks()),
        "---\n\nmiddle\n\n<!-- pagebreak -->\n\nend\n\n---\n"
    );
    let mut o = MarkdownOptions {
        page_break: MarkdownPageBreak::Rule,
        ..Default::default()
    };
    let doc = Document::from_blocks(blocks());
    assert_eq!(
        round_trip_with(&doc, &o),
        "---\n\nmiddle\n\n---\n\nend\n\n---\n"
    );
    o.page_break = MarkdownPageBreak::Omit;
    assert_eq!(round_trip_with(&doc, &o), "---\n\nmiddle\n\nend\n\n---\n");
    o.page_break = MarkdownPageBreak::Html;
    assert!(
        round_trip_with(&doc, &o)
            .contains("\n\n<div style=\"page-break-after: always\"></div>\n\n")
    );
    // Void blocks only.
    assert_eq!(
        round_trip(vec![Block::new(BlockKind::PageBreak)]),
        "<!-- pagebreak -->\n"
    );
    assert_eq!(
        round_trip(vec![
            Block::new(BlockKind::Divider),
            Block::new(BlockKind::Divider)
        ]),
        "---\n\n---\n"
    );
}

// ----- exact output: inline ---------------------------------------------------------------

#[test]
fn basic_marks() {
    assert_eq!(
        one(&[("a ", plain()), ("bold", bold()), (" b", plain())]),
        "a **bold** b"
    );
    assert_eq!(
        one(&[("a ", plain()), ("italic", italic()), (" b", plain())]),
        "a *italic* b"
    );
    assert_eq!(
        one(&[("a ", plain()), ("gone", strike()), (" b", plain())]),
        "a ~~gone~~ b"
    );
    assert_eq!(
        one(&[("a ", plain()), ("code", code()), (" b", plain())]),
        "a `code` b"
    );
    assert_eq!(
        one(&[("a ", plain()), ("under", underline()), (" b", plain())]),
        "a <u>under</u> b"
    );
    let hl = InlineAttrs {
        highlight: Some(Highlight::Yellow),
        ..plain()
    };
    assert_eq!(
        one(&[("a ", plain()), ("marked", hl), (" b", plain())]),
        "a <mark>marked</mark> b"
    );
    assert_eq!(
        one(&[
            ("see ", plain()),
            ("the site", link("https://example.com")),
            (".", plain())
        ]),
        "see [the site](https://example.com)."
    );
}

#[test]
fn whitespace_moves_outside_delimiters() {
    assert_eq!(
        one(&[("a", plain()), (" bold ", bold()), ("b", plain())]),
        "a **bold** b"
    );
    assert_eq!(
        one(&[("a", plain()), ("  italic\t", italic()), ("b", plain())]),
        "a  *italic*\tb"
    );
    assert_eq!(one(&[("a", plain()), (" ", bold()), ("b", plain())]), "a b");
    assert_eq!(
        one(&[("x", plain()), (" link ", link("u:x")), ("y", plain())]),
        "x [link](u:x) y"
    );
    // Around a line break too.
    assert_eq!(
        one(&[("one\n", bold()), ("two", plain())]),
        "**one**\\\ntwo"
    );
    assert_eq!(
        one(&[("one", plain()), ("\ntwo", bold())]),
        "one\\\n**two**"
    );
    assert_eq!(one(&[("one\ntwo", bold())]), "**one\\\ntwo**");
}

#[test]
fn nesting() {
    let bi = bold().with(InlineStyle::Italic);
    assert_eq!(one(&[("both", bi.clone())]), "***both***");
    assert_eq!(
        one(&[("bold ", bold()), ("both", bi.clone()), (" bold", bold())]),
        "**bold *both* bold**"
    );
    assert_eq!(
        one(&[("it ", italic()), ("both", bi.clone()), (" it", italic())]),
        "*it **both** it*"
    );
    assert_eq!(
        one(&[("both", bi.clone()), (" bold", bold())]),
        "***both* bold**"
    );
    assert_eq!(
        one(&[("bold ", bold()), ("both", bi.clone())]),
        "**bold *both***"
    );
    let all = InlineAttrs {
        bold: true,
        italic: true,
        strikethrough: true,
        underline: true,
        highlight: Some(Highlight::Green),
        link: Some("https://x.test/".into()),
        ..plain()
    };
    assert_eq!(
        one(&[("all", all)]),
        "[<mark><u>~~***all***~~</u></mark>](https://x.test/)"
    );
    assert_eq!(
        one(&[("code", code().with(InlineStyle::Bold))]),
        "**`code`**"
    );
}

#[test]
fn adjacent_runs_do_not_break_markers() {
    assert_eq!(
        one(&[("bold", bold()), ("italic", italic())]),
        "**bold***italic*"
    );
    assert_eq!(
        one(&[("italic", italic()), ("bold", bold())]),
        "*italic***bold**"
    );
    assert_eq!(
        one(&[("a", bold()), ("b", strike()), ("c", italic())]),
        "**a**~~b~~*c*"
    );
    // Bold with a differently coloured middle is still one bold span.
    let red = InlineAttrs {
        color: Some(Rgb::new(255, 0, 0)),
        ..bold()
    };
    assert_eq!(
        one(&[("one ", bold()), ("two", red), (" three", bold())]),
        "**one two three**"
    );
    // Partial overlap: the italic part is split, the bold stays whole.
    assert_eq!(
        one(&[
            ("aaa ", italic()),
            ("bbb", italic().with(InlineStyle::Bold)),
            (" ccc", bold())
        ]),
        "*aaa* ***bbb* ccc**"
    );
}

#[test]
fn intraword_and_punctuation() {
    assert_eq!(
        one(&[("un", plain()), ("believ", bold()), ("able", plain())]),
        "un**believ**able"
    );
    assert_eq!(
        one(&[("“", plain()), ("quoted", italic()), ("”", plain())]),
        "“*quoted*”"
    );
    assert_eq!(
        one(&[("(", plain()), ("x", bold()), (")", plain())]),
        "(**x**)"
    );
    // Delimiters that CommonMark would not pair fall back to HTML.
    assert_eq!(
        one(&[("a", plain()), ("(b)", bold()), ("c", plain())]),
        "a<strong>(b)</strong>c"
    );
    assert_eq!(
        one(&[("wow!", italic()), ("really", plain())]),
        "<em>wow!</em>really"
    );
    assert_eq!(
        one(&[("x", plain()), ("-y-", strike()), ("z", plain())]),
        "x<del>-y-</del>z"
    );
    // An emoji next to a letter: parsers disagree, so HTML it is.
    assert_eq!(
        one(&[("a", plain()), ("😀b", bold())]),
        "a<strong>😀b</strong>"
    );
    assert_eq!(one(&[("a ", plain()), ("😀b", bold())]), "a **😀b**");
}

#[test]
fn inline_code_edge_cases() {
    assert_eq!(one(&[("a`b", code())]), "``a`b``");
    assert_eq!(one(&[("`", code())]), "`` ` ``");
    assert_eq!(one(&[("``x``", code())]), "``` ``x`` ```");
    assert_eq!(
        one(&[("say ", plain()), (" x ", code()), (" now", plain())]),
        "say `  x  ` now"
    );
    assert_eq!(
        one(&[("*not* <b> &amp; [x](y) \\", code())]),
        "`*not* <b> &amp; [x](y) \\`"
    );
    // A line break inside code splits the span.
    assert_eq!(one(&[("a\nb", code())]), "`a`\\\n`b`");
    // Half bold code.
    assert_eq!(
        one(&[("ab", code().with(InlineStyle::Bold)), ("cd", code())]),
        "**`ab`**`cd`"
    );
    // Backticks in ordinary text are escaped.
    assert_eq!(one(&[("use `x` here", plain())]), "use \\`x\\` here");
}

#[test]
fn links() {
    assert_eq!(
        one(&[("https://example.com/a_b", link("https://example.com/a_b"))]),
        "<https://example.com/a_b>"
    );
    assert_eq!(
        one(&[(
            "wiki",
            link("https://en.wikipedia.org/wiki/Rust_(programming_language)")
        )]),
        "[wiki](https://en.wikipedia.org/wiki/Rust_\\(programming_language\\))"
    );
    assert_eq!(one(&[("a b", link("my file.txt"))]), "[a b](my%20file.txt)");
    assert_eq!(one(&[("[brackets]", link("u:1"))]), "[\\[brackets\\]](u:1)");
    assert_eq!(
        one(&[
            (
                "bold",
                InlineAttrs {
                    bold: true,
                    ..link("u:1")
                }
            ),
            (" plain", link("u:1"))
        ]),
        "[**bold** plain](u:1)"
    );
    assert_eq!(
        one(&[("one", link("u:1")), ("two", link("u:2"))]),
        "[one](u:1)[two](u:2)"
    );
    // A link is never split: the bold that crosses its edge is.
    assert_eq!(
        one(&[
            ("bold ", bold()),
            (
                "link",
                InlineAttrs {
                    bold: true,
                    ..link("u:1")
                }
            ),
            (" rest", link("u:1"))
        ]),
        "**bold** [**link** rest](u:1)"
    );
    // A blank target is no link at all.
    assert_eq!(one(&[("text", link("  "))]), "text");
}

#[test]
fn escaping_in_context() {
    assert_eq!(one(&[("1. not a list", plain())]), "1\\. not a list");
    assert_eq!(one(&[("# not a heading", plain())]), "\\# not a heading");
    assert_eq!(one(&[("- not a bullet", plain())]), "\\- not a bullet");
    assert_eq!(one(&[("> not a quote", plain())]), "\\> not a quote");
    assert_eq!(one(&[("---", plain())]), "\\---");
    assert_eq!(one(&[("line\n---", plain())]), "line\\\n\\---");
    assert_eq!(one(&[("line\n===", plain())]), "line\\\n\\===");
    assert_eq!(one(&[("a | b\n--|--", plain())]), "a | b\\\n\\--|--");
    assert_eq!(one(&[("    indented", plain())]), "indented");
    assert_eq!(
        one(&[("*stars* _under_ `ticks`", plain())]),
        "\\*stars\\* \\_under\\_ \\`ticks\\`"
    );
    assert_eq!(
        one(&[("<script>alert(1)</script>", plain())]),
        "\\<script>alert(1)\\</script>"
    );
    assert_eq!(
        one(&[("[not](a link) ![img](x)", plain())]),
        "[not\\](a link) ![img\\](x)"
    );
    // Characters that are only markup with a partner stay as they are in
    // ordinary prose.
    for prose in [
        "As shown in [1] and [2], the result [sic] holds [...] everywhere.",
        "Prices start at $5* a month.",
        "Use _config.yml, snake_case_names and 2 * 3 = 6.",
        "If a<b and c < d then e <- f; I <3 this.",
        "Tom & Jerry, R&D, 100% #1 @home ~5 min, C:\\Users\\me, a|b.",
        "“Quotes”, ‘single’, em—dash, ellipsis… (parentheses) {braces} + - = / ^ $ !",
        "A footnote marker* and one more thing.",
        "Email me@example.com or see example.com/path_a.",
    ] {
        assert_eq!(one(&[(prose, plain())]), prose);
    }
    // The partner arrives: now they are escaped.
    assert_eq!(one(&[("a* b*", plain())]), "a\\* b\\*");
    assert_eq!(one(&[("_a b_", plain())]), "\\_a b\\_");
    assert_eq!(one(&[("a<b>c", plain())]), "a\\<b>c");
    assert_eq!(one(&[("x* ", plain()), ("y", bold())]), "x\\* **y**");
    assert_eq!(
        one(&[("a<b ", plain()), ("y", underline())]),
        "a\\<b <u>y</u>"
    );
    assert_eq!(one(&[("[1]: /url", plain())]), "[1\\]: /url");
    assert_eq!(one(&[("[", plain()), ("a]: b", code())]), "\\[`a]: b`");
    assert_eq!(
        one(&[(
            "x]: y",
            InlineAttrs {
                code: true,
                ..link("u:1")
            }
        )]),
        "[<code>x\\]\\: y</code>](u:1)"
    );
    assert_eq!(
        one(&[("&copy; &#169; AT&T", plain())]),
        "\\&copy; \\&#169; AT&T"
    );
    // After bold at the start of a line, a "#" is not a heading.
    assert_eq!(one(&[("x", bold()), ("# y", plain())]), "**x**# y");
    // In list items and quotes the text is at a line start as well.
    assert_eq!(
        md(vec![
            Block::with_text(BlockKind::BulletItem, "- x"),
            Block::with_text(BlockKind::BulletItem, "[ ] y")
        ]),
        "- \\- x\n- \\[ ] y\n"
    );
}

#[test]
fn options_for_things_markdown_cannot_say() {
    let styled = InlineAttrs {
        underline: true,
        highlight: Some(Highlight::Pink),
        color: Some(Rgb::new(0x12, 0x34, 0x56)),
        font: Some("Iowan Old Style".into()),
        ..plain()
    };
    let doc = Document::from_blocks(vec![
        para()
            .with_span("text", styled)
            .with_alignment(Alignment::Center),
        Block::with_text(BlockKind::heading(1), "Right").with_alignment(Alignment::Right),
        Block::paragraph("left"),
    ]);
    let source = ExportSource::from(&doc);
    assert_eq!(
        to_markdown(&source, &MarkdownOptions::default()),
        "<mark><u>text</u></mark>\n\n# Right\n\nleft\n"
    );
    assert_eq!(
        to_markdown(&source, &MarkdownOptions::strict()),
        "text\n\n# Right\n\nleft\n"
    );
    let o = MarkdownOptions {
        highlight: MarkdownHighlight::Equals,
        underline_html: false,
        ..Default::default()
    };
    assert_eq!(to_markdown(&source, &o), "==text==\n\n# Right\n\nleft\n");
    let o = MarkdownOptions {
        colors: true,
        fonts: true,
        alignment: true,
        ..Default::default()
    };
    let out = to_markdown(&source, &o);
    // The wrappers are HTML blocks; what is between them is still Markdown.
    let lines: Vec<Line> = parse(&out)
        .into_iter()
        .filter(|l| !matches!(l.kind, Kind::Html(_)))
        .collect();
    assert_eq!(lines, expected(&doc, &o));
    assert_eq!(
        out,
        "<div align=\"center\">\n\n<span style=\"color:#123456;font-family:'Iowan Old Style'\"><mark><u>text</u></mark></span>\n\n</div>\n\n\
         <div align=\"right\">\n\n# Right\n\n</div>\n\nleft\n"
    );
}

// ----- images -----------------------------------------------------------------------------

const HARBOUR: &str = "3f2a9c1b5d6e7f80a1b2c3d4e5f60718";

#[test]
fn an_image_is_a_line_that_names_the_stored_image() {
    let out = round_trip(vec![
        Block::paragraph("Before."),
        image_block(HARBOUR, "The harbour at dusk"),
        Block::paragraph("After."),
    ]);
    assert_eq!(
        out,
        "Before.\n\n![The harbour at dusk](image:3f2a9c1b5d6e7f80a1b2c3d4e5f60718)\n\nAfter.\n"
    );
    // Without a description, and as the only block.
    assert_eq!(
        round_trip(vec![image_block(HARBOUR, "")]),
        "![](image:3f2a9c1b5d6e7f80a1b2c3d4e5f60718)\n"
    );
    // The width is not written, whatever it is; a source that has the
    // bytes changes nothing while references are asked for.
    let narrow = Block::image(Image {
        width: ImageWidth::from_percent(25),
        ..image(HARBOUR, "Small")
    });
    let doc = Document::from_blocks(vec![narrow]);
    let pictures = Pictures::default().with(HARBOUR, ImageFileFormat::Png, b"png bytes");
    let source = ExportSource::from(&doc).with_images(&pictures);
    assert_eq!(
        to_markdown(&source, &MarkdownOptions::default()),
        "![Small](image:3f2a9c1b5d6e7f80a1b2c3d4e5f60718)\n"
    );
    assert_eq!(
        to_markdown(&source, &MarkdownOptions::strict()),
        "![Small](image:3f2a9c1b5d6e7f80a1b2c3d4e5f60718)\n"
    );
    let package = export_package(&source, ExportFormat::Markdown, &ExportOptions::default());
    assert_eq!(
        package.bytes,
        b"![Small](image:3f2a9c1b5d6e7f80a1b2c3d4e5f60718)\n"
    );
    assert_eq!(package.assets, []);
}

#[test]
fn the_description_of_an_image_is_read_back_as_it_is() {
    for description in [
        "a ] b [ c",
        "[link](https://example.com) and ![nested](x.png)",
        "*stars* and **more**, _under_ and __scores__, ~~struck~~",
        "`code` and ``more``",
        "a <b>tag</b> and <https://example.com>",
        "AT&T &amp; &#42; 5 * 3 = 15",
        "ends with a backslash \\",
        "C:\\Users\\me and \\[escaped\\]",
        "[^1] and [ref][label] and ]( and )",
        "# not a heading",
        "- not a list",
        "1. not a list",
        "> not a quote",
        "| not | a | table |",
        "    four spaces inside are kept",
        "tab\tinside",
        "\"quoted\" and 'single'",
        "日本語の説明, שלום, émile, 👩‍👩‍👧",
        "^1",
        "^",
        "^note] and more",
        "!",
        "!!",
        "\\",
        "]",
        "[",
        "<",
        ">",
        "*",
        "_",
    ] {
        let doc = Document::from_blocks(vec![image_block(HARBOUR, description)]);
        // What the document holds is the description as it was cleaned.
        let kept = doc.blocks()[0].as_image().unwrap().description.clone();
        let out = round_trip_with(&doc, &MarkdownOptions::default());
        assert!(
            out.ends_with("](image:3f2a9c1b5d6e7f80a1b2c3d4e5f60718)\n") && out.starts_with("!["),
            "{description:?} gave {out:?}"
        );
        assert_eq!(out.lines().count(), 1, "{description:?} gave {out:?}");
        let lines = parse(&out);
        let alt: String = lines[0].text.iter().map(|(c, _)| *c).collect();
        assert_eq!(alt, kept, "{out:?}");
    }
}

#[test]
fn an_image_keeps_the_lists_around_it_apart() {
    // An image has no indent: it ends the list above it, and the list
    // below is a new one.
    let out = round_trip(vec![
        Block::with_text(BlockKind::BulletItem, "one"),
        Block::with_text(BlockKind::BulletItem, "two").with_indent(1),
        image_block(HARBOUR, "Between"),
        Block::with_text(BlockKind::BulletItem, "three"),
    ]);
    assert_eq!(
        out,
        "- one\n  - two\n\n![Between](image:3f2a9c1b5d6e7f80a1b2c3d4e5f60718)\n\n- three\n"
    );
    let out = round_trip(vec![
        Block::with_text(BlockKind::NumberedItem, "one"),
        image_block(HARBOUR, ""),
        Block::with_text(BlockKind::NumberedItem, "again one"),
    ]);
    assert_eq!(
        out,
        "1. one\n\n![](image:3f2a9c1b5d6e7f80a1b2c3d4e5f60718)\n\n1. again one\n"
    );
    // Under an item without text, where a paragraph would be read as the
    // item's own text.
    let out = round_trip(vec![
        Block::new(BlockKind::BulletItem),
        image_block(HARBOUR, "Alone"),
        Block::new(BlockKind::TodoItem { checked: false }),
        image_block(HARBOUR, "Alone"),
    ]);
    assert_eq!(
        out,
        "-\n\n![Alone](image:3f2a9c1b5d6e7f80a1b2c3d4e5f60718)\n\n- [ ]\n\n![Alone](image:3f2a9c1b5d6e7f80a1b2c3d4e5f60718)\n"
    );
    // Beside every other kind of block.
    round_trip(vec![
        Block::with_text(BlockKind::heading(2), "Title"),
        image_block(HARBOUR, "a"),
        Block::with_text(BlockKind::Quote, "quote"),
        image_block(HARBOUR, "b"),
        Block::with_text(BlockKind::code(), "code"),
        image_block(HARBOUR, "c"),
        Block::new(BlockKind::Divider),
        image_block(HARBOUR, "d"),
        Block::new(BlockKind::PageBreak),
        image_block(HARBOUR, "e"),
        image_block("00ff", "f"),
    ]);
}

#[test]
fn the_alignment_of_an_image_is_kept_when_alignment_is() {
    let doc = Document::from_blocks(vec![
        image_block(HARBOUR, "Left"),
        image_block(HARBOUR, "Centre").with_alignment(Alignment::Center),
        image_block(HARBOUR, "Right").with_alignment(Alignment::Right),
    ]);
    let source = ExportSource::from(&doc);
    let reference = |alt: &str| format!("![{alt}](image:{HARBOUR})");
    // Dropped by default, like the alignment of a paragraph.
    assert_eq!(
        to_markdown(&source, &MarkdownOptions::default()),
        format!(
            "{}\n\n{}\n\n{}\n",
            reference("Left"),
            reference("Centre"),
            reference("Right")
        )
    );
    let o = MarkdownOptions {
        alignment: true,
        ..Default::default()
    };
    let out = to_markdown(&source, &o);
    assert_eq!(
        out,
        format!(
            "{}\n\n<div align=\"center\">\n\n{}\n\n</div>\n\n<div align=\"right\">\n\n{}\n\n</div>\n",
            reference("Left"),
            reference("Centre"),
            reference("Right")
        )
    );
    // The wrappers are HTML blocks; what is between them is still an image.
    let lines: Vec<Line> = parse(&out)
        .into_iter()
        .filter(|l| !matches!(l.kind, Kind::Html(_)))
        .collect();
    assert_eq!(lines, expected(&doc, &o));
}

fn files(dir: &str) -> ExportOptions {
    ExportOptions {
        markdown: MarkdownOptions {
            images: MarkdownImages::Files {
                dir: dir.to_string(),
            },
            ..Default::default()
        },
        ..Default::default()
    }
}

#[test]
fn a_file_export_links_picture_files_and_returns_them() {
    let doc = Document::from_blocks(vec![
        Block::paragraph("Before."),
        image_block(HARBOUR, "The harbour"),
        image_block("00ff", "A drawing"),
        // The same picture a second time: one file.
        image_block(HARBOUR, "The harbour again"),
        image_block("abcd", "Moving"),
    ]);
    let pictures = Pictures::default()
        .with(HARBOUR, ImageFileFormat::Jpeg, b"\xff\xd8 the harbour")
        .with("00ff", ImageFileFormat::Png, b"\x89PNG a drawing")
        .with("abcd", ImageFileFormat::Gif, b"GIF89a moving");
    let source = ExportSource::single("Trip", &doc).with_images(&pictures);
    let package = export_package(&source, ExportFormat::Markdown, &files("pictures"));
    assert_eq!(
        String::from_utf8(package.bytes.clone()).unwrap(),
        "Before.\n\n![The harbour](pictures/image-3f2a9c1b.jpg)\n\n![A drawing](pictures/image-00ff0000.png)\n\n\
         ![The harbour again](pictures/image-3f2a9c1b.jpg)\n\n![Moving](pictures/image-abcd0000.gif)\n"
    );
    assert_eq!(
        package.assets,
        [
            ExportAsset {
                path: "pictures/image-3f2a9c1b.jpg".into(),
                bytes: b"\xff\xd8 the harbour".to_vec()
            },
            ExportAsset {
                path: "pictures/image-00ff0000.png".into(),
                bytes: b"\x89PNG a drawing".to_vec()
            },
            ExportAsset {
                path: "pictures/image-abcd0000.gif".into(),
                bytes: b"GIF89a moving".to_vec()
            },
        ]
    );
    // `export` and `to_markdown` write the same text and return no files.
    assert_eq!(
        export(&source, ExportFormat::Markdown, &files("pictures")),
        package.bytes
    );
    assert_eq!(
        to_markdown(&source, &files("pictures").markdown).into_bytes(),
        package.bytes
    );
    // The same export twice gives the same files.
    assert_eq!(
        export_package(&source, ExportFormat::Markdown, &files("pictures")),
        package
    );
    // Only Markdown has files beside it.
    for format in [ExportFormat::PlainText, ExportFormat::Docx] {
        let other = export_package(&source, format, &files("pictures"));
        assert_eq!(other.assets, []);
        assert_eq!(other.bytes, export(&source, format, &files("pictures")));
    }
    // A folder written with a slash at its end, or the other slash, is the
    // same folder; none at all is the folder of the file.
    let slashed = export_package(&source, ExportFormat::Markdown, &files(" a\\b/ "));
    assert_eq!(slashed.assets[0].path, "a/b/image-3f2a9c1b.jpg");
    assert!(
        String::from_utf8(slashed.bytes)
            .unwrap()
            .contains("](a/b/image-3f2a9c1b.jpg)")
    );
    let beside = export_package(&source, ExportFormat::Markdown, &files(""));
    assert_eq!(beside.assets[0].path, "image-3f2a9c1b.jpg");
    assert!(
        String::from_utf8(beside.bytes)
            .unwrap()
            .starts_with("Before.\n\n![The harbour](image-3f2a9c1b.jpg)\n")
    );
}

#[test]
fn a_folder_with_a_space_in_its_name_is_read_back_as_it_is_on_disk() {
    assert_eq!(images_dir_name("Chapter 2.md"), "Chapter 2 images");
    assert_eq!(images_dir_name("Chapter 2"), "Chapter 2 images");
    assert_eq!(images_dir_name("v1.2 notes.md"), "v1.2 notes images");
    assert_eq!(images_dir_name("out/Trip.markdown"), "Trip images");
    assert_eq!(images_dir_name(".md"), ".md images");
    assert_eq!(images_dir_name(""), "Untitled images");

    let pictures = Pictures::default().with(HARBOUR, ImageFileFormat::Jpeg, b"jpeg");
    for (dir, written) in [
        ("Chapter 2 images", "<Chapter 2 images/image-3f2a9c1b.jpg>"),
        (
            "Émile (draft) images",
            "<Émile (draft) images/image-3f2a9c1b.jpg>",
        ),
        (
            "a <b> & c images",
            "<a \\<b\\> & c images/image-3f2a9c1b.jpg>",
        ),
        ("R&amp;D images", "<R\\&amp;D images/image-3f2a9c1b.jpg>"),
        (
            "back\\slash images",
            "<back/slash images/image-3f2a9c1b.jpg>",
        ),
        // Without a space the destination needs no brackets, and what
        // would end it early is escaped.
        ("Trip-images", "Trip-images/image-3f2a9c1b.jpg"),
        ("Trip(1)", "Trip\\(1\\)/image-3f2a9c1b.jpg"),
        ("東京", "東京/image-3f2a9c1b.jpg"),
    ] {
        let doc = Document::from_blocks(vec![image_block(HARBOUR, "a ] b")]);
        let source = ExportSource::single("Trip", &doc).with_images(&pictures);
        let package = export_package(&source, ExportFormat::Markdown, &files(dir));
        let out = String::from_utf8(package.bytes).unwrap();
        assert_eq!(out, format!("![a \\] b]({written})\n"), "{dir:?}");
        // A parser finds the path of the file that was returned.
        let lines = parse(&out);
        assert_eq!(lines.len(), 1, "{out:?}");
        assert_eq!(
            lines[0].kind,
            Kind::Image(package.assets[0].path.clone()),
            "{out:?}"
        );
        let alt: String = lines[0].text.iter().map(|(c, _)| *c).collect();
        assert_eq!(alt, "a ] b");
    }
}

#[test]
fn a_folder_with_a_hash_or_a_percent_in_its_name_is_a_link_a_viewer_follows() {
    // A link is a URL to whoever follows it: `#` ends the path, `?` too,
    // and `%` is an escape. The folder named after "Draft #2 100%25.md" is
    // still the folder the link leads to.
    let pictures = Pictures::default().with(HARBOUR, ImageFileFormat::Jpeg, b"jpeg");
    let base = url::Url::parse("file:///Users/someone/out/Draft.md").unwrap();
    for file_name in [
        "Draft #2 100%25.md",
        "C# notes.md",
        "Issue #14?.md",
        "50% done.md",
        "Trip.md",
        "#.md",
    ] {
        let dir = images_dir_name(file_name);
        let doc = Document::from_blocks(vec![image_block(HARBOUR, "The harbour")]);
        let source = ExportSource::single("Trip", &doc).with_images(&pictures);
        let package = export_package(&source, ExportFormat::Markdown, &files(&dir));
        let out = String::from_utf8(package.bytes).unwrap();
        // The file is written where its folder is called as on disk.
        assert_eq!(
            package.assets[0].path,
            format!("{dir}/image-3f2a9c1b.jpg"),
            "{file_name:?}"
        );
        let lines = parse(&out);
        assert_eq!(lines.len(), 1, "{out:?}");
        let Kind::Image(destination) = &lines[0].kind else {
            panic!("no image in {out:?}");
        };
        let link = base.join(destination).unwrap();
        assert_eq!(link.fragment(), None, "{out:?}");
        assert_eq!(link.query(), None, "{out:?}");
        let followed = link.to_file_path().unwrap();
        assert_eq!(
            followed,
            std::path::Path::new("/Users/someone/out").join(&package.assets[0].path),
            "{out:?}"
        );
    }
}

#[test]
fn two_images_whose_ids_begin_alike_get_two_files() {
    // Three different pictures whose ids share their first eight digits,
    // the last two their first twelve.
    let first = "3f2a9c1b00000000000000000000000a";
    let second = "3f2a9c1b11110000000000000000000b";
    let third = "3f2a9c1b11112222000000000000000c";
    let doc = Document::from_blocks(vec![
        image_block(first, "one"),
        image_block(second, "two"),
        image_block(first, "one again"),
        image_block(third, "three"),
        image_block(second, "two again"),
    ]);
    let pictures = Pictures::default()
        .with(first, ImageFileFormat::Png, b"first")
        .with(second, ImageFileFormat::Png, b"second")
        .with(third, ImageFileFormat::Jpeg, b"third");
    let source = ExportSource::single("T", &doc).with_images(&pictures);
    let package = export_package(&source, ExportFormat::Markdown, &files("p"));
    assert_eq!(
        String::from_utf8(package.bytes).unwrap(),
        "![one](p/image-3f2a9c1b.png)\n\n![two](p/image-3f2a9c1b1111.png)\n\n![one again](p/image-3f2a9c1b.png)\n\n\
         ![three](p/image-3f2a9c1b11112222.jpg)\n\n![two again](p/image-3f2a9c1b1111.png)\n"
    );
    let assets: Vec<(&str, &[u8])> = package
        .assets
        .iter()
        .map(|a| (a.path.as_str(), a.bytes.as_slice()))
        .collect();
    assert_eq!(
        assets,
        [
            ("p/image-3f2a9c1b.png", &b"first"[..]),
            ("p/image-3f2a9c1b1111.png", &b"second"[..]),
            ("p/image-3f2a9c1b11112222.jpg", &b"third"[..]),
        ]
    );
    // Eight pictures whose ids differ only in their last digits: each takes
    // four digits more than the one before, up to the whole id.
    let ids: Vec<String> = (0..8)
        .map(|n| format!("3f2a9c1b5d6e7f80a1b2c3d4e5f6{n:04x}"))
        .collect();
    let doc = Document::from_blocks(ids.iter().map(|id| image_block(id, "")).collect());
    let mut pictures = Pictures::default();
    for id in &ids {
        pictures = pictures.with(id, ImageFileFormat::Png, id.as_bytes());
    }
    let source = ExportSource::single("T", &doc).with_images(&pictures);
    let package = export_package(&source, ExportFormat::Markdown, &files("p"));
    let paths: Vec<&str> = package.assets.iter().map(|a| a.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "p/image-3f2a9c1b.png",
            "p/image-3f2a9c1b5d6e.png",
            "p/image-3f2a9c1b5d6e7f80.png",
            "p/image-3f2a9c1b5d6e7f80a1b2.png",
            "p/image-3f2a9c1b5d6e7f80a1b2c3d4.png",
            "p/image-3f2a9c1b5d6e7f80a1b2c3d4e5f6.png",
            "p/image-3f2a9c1b5d6e7f80a1b2c3d4e5f60006.png",
            "p/image-3f2a9c1b5d6e7f80a1b2c3d4e5f60007.png",
        ]
    );
    // Every file holds its own picture, and every link names its own file.
    let text = String::from_utf8(package.bytes).unwrap();
    for ((id, asset), line) in ids.iter().zip(&package.assets).zip(text.split("\n\n")) {
        assert_eq!(asset.bytes, id.as_bytes());
        assert_eq!(line.trim_end(), format!("![]({})", asset.path));
    }
}

#[test]
fn an_image_whose_bytes_are_missing_is_a_reference_in_a_file_export_too() {
    let doc = Document::from_blocks(vec![
        image_block(HARBOUR, "Here"),
        image_block("00ff", "Gone"),
        image_block("abcd", "Empty file"),
    ]);
    let pictures = Pictures::default()
        .with(HARBOUR, ImageFileFormat::Png, b"png")
        .with("abcd", ImageFileFormat::Png, b"");
    let source = ExportSource::single("T", &doc).with_images(&pictures);
    let package = export_package(&source, ExportFormat::Markdown, &files("p"));
    assert_eq!(
        String::from_utf8(package.bytes).unwrap(),
        "![Here](p/image-3f2a9c1b.png)\n\n![Gone](image:00ff0000000000000000000000000000)\n\n\
         ![Empty file](image:abcd0000000000000000000000000000)\n"
    );
    assert_eq!(package.assets.len(), 1);
    // A source that was given no images at all has none.
    let bare = ExportSource::single("T", &doc);
    assert!(!bare.has_images() && source.has_images());
    let package = export_package(&bare, ExportFormat::Markdown, &files("p"));
    assert_eq!(package.assets, []);
    assert_eq!(
        String::from_utf8(package.bytes).unwrap(),
        to_markdown(&bare, &MarkdownOptions::default())
    );
}

#[test]
fn the_documents_of_a_collection_share_their_picture_files() {
    let one = Document::from_blocks(vec![image_block(HARBOUR, "In one")]);
    let two = Document::from_blocks(vec![
        image_block(HARBOUR, "In two"),
        image_block("00ff", "Only in two"),
    ]);
    let pictures = Pictures::default()
        .with(HARBOUR, ImageFileFormat::Png, b"harbour")
        .with("00ff", ImageFileFormat::Png, b"other");
    let source =
        ExportSource::collection("Book", [("One", &one), ("Two", &two)]).with_images(&pictures);
    let options = ExportOptions {
        markdown: MarkdownOptions {
            part_titles: PartTitles::Never,
            ..files("Book images").markdown
        },
        ..Default::default()
    };
    let package = export_package(&source, ExportFormat::Markdown, &options);
    assert_eq!(
        String::from_utf8(package.bytes).unwrap(),
        "![In one](<Book images/image-3f2a9c1b.png>)\n\n---\n\n![In two](<Book images/image-3f2a9c1b.png>)\n\n\
         ![Only in two](<Book images/image-00ff0000.png>)\n"
    );
    let paths: Vec<&str> = package.assets.iter().map(|a| a.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "Book images/image-3f2a9c1b.png",
            "Book images/image-00ff0000.png"
        ]
    );
}

#[test]
fn image_file_formats() {
    assert_eq!(ImageFileFormat::Png.extension(), "png");
    assert_eq!(ImageFileFormat::Jpeg.extension(), "jpg");
    assert_eq!(ImageFileFormat::Gif.extension(), "gif");
    assert_eq!(ImageFileFormat::Png.mime_type(), "image/png");
    assert_eq!(ImageFileFormat::Jpeg.mime_type(), "image/jpeg");
    assert_eq!(ImageFileFormat::Gif.mime_type(), "image/gif");
    for format in ImageFileFormat::ALL {
        assert_eq!(
            ImageFileFormat::from_extension(format.extension()),
            Some(format)
        );
    }
    assert_eq!(
        ImageFileFormat::from_extension(".JPEG"),
        Some(ImageFileFormat::Jpeg)
    );
    assert_eq!(
        ImageFileFormat::from_extension(" Png "),
        Some(ImageFileFormat::Png)
    );
    for other in ["", "webp", "heic", "svg", "pdf", "tiff", "pn g"] {
        assert_eq!(ImageFileFormat::from_extension(other), None, "{other:?}");
    }
    // Megabytes of picture do not go into a log.
    let data = ImageData {
        bytes: Cow::Owned(vec![7; 5000]),
        format: ImageFileFormat::Gif,
    };
    assert_eq!(
        format!("{data:?}"),
        "ImageData { bytes: 5000 bytes, format: Gif }"
    );
}

#[test]
fn plain_text_has_the_description_of_an_image_on_a_line_of_its_own() {
    let doc = Document::from_blocks(vec![
        Block::paragraph("Before."),
        image_block(HARBOUR, "The harbour at dusk"),
        Block::paragraph("Between."),
        // Nobody described this one: nothing is written for it, and the
        // paragraphs around it are one blank line apart.
        image_block("00ff", ""),
        Block::paragraph("After."),
        image_block("abcd", "*Not* [markup] here"),
    ]);
    let source = ExportSource::from(&doc);
    assert_eq!(
        to_plain_text(&source, &Default::default()),
        "Before.\n\nThe harbour at dusk\n\nBetween.\n\nAfter.\n\n*Not* [markup] here\n"
    );
    // With the bytes at hand it is the same: plain text holds no picture.
    let pictures = Pictures::default().with(HARBOUR, ImageFileFormat::Png, b"png");
    assert_eq!(
        to_plain_text(&source.clone().with_images(&pictures), &Default::default()),
        to_plain_text(&source, &Default::default())
    );
    // The letters of a description are not restyled for a post.
    let social = to_plain_text(&source, &innerword_export::PlainTextOptions::social());
    assert!(social.contains("\n\nThe harbour at dusk\n\n"), "{social:?}");
    // Only images, none described: nothing at all.
    let doc = Document::from_blocks(vec![image_block(HARBOUR, ""), image_block("00ff", "")]);
    assert_eq!(
        to_plain_text(&ExportSource::from(&doc), &Default::default()),
        ""
    );
}

/// Random documents with images among their blocks, their descriptions
/// made of everything Markdown could read as markup.
#[test]
fn random_documents_with_images_round_trip() {
    for seed in 1..=seeds(3000) {
        let mut rng = Rng(seed.wrapping_mul(0xA24B_AED4_963E_E407) | 1);
        let n = 1 + rng.below(8);
        let blocks: Vec<Block> = (0..n)
            .map(|_| {
                if !rng.chance(40) {
                    return adv_block(&mut rng);
                }
                let pieces = rng.below(5);
                let description = if rng.chance(50) {
                    adv_text(&mut rng, pieces)
                } else {
                    random_text(&mut rng, pieces)
                };
                let id = ["3f2a", "00ff", "abcd"][rng.below(3)];
                let alignment =
                    [Alignment::Left, Alignment::Center, Alignment::Right][rng.below(3)];
                image_block(id, &description).with_alignment(alignment)
            })
            .collect();
        let doc = Document::from_blocks(blocks);
        let options = adv_options(&mut rng);
        let result = std::panic::catch_unwind(|| round_trip_with(&doc, &options));
        if let Err(e) = result {
            eprintln!("seed {seed} failed");
            std::panic::resume_unwind(e);
        }
    }
}

// ----- comments ---------------------------------------------------------------------------

fn commented() -> Document {
    let mut ed = EditorState::new(Document::from_blocks(vec![
        Block::paragraph("The quick brown fox."),
        Block::with_text(BlockKind::code(), "let x = 1;"),
        Block::paragraph("Last words"),
    ]));
    ed.set_selection(Selection::new(Position::new(0, 4), Position::new(0, 9)));
    let first = ed
        .add_comment("Is it -- really --> quick?\nSecond line.", "Ada", 1)
        .unwrap();
    ed.add_reply(first, "Yes. *Very*.", "Ben", 2).unwrap();
    ed.resolve_comment(first);
    ed.set_selection(Selection::new(Position::new(1, 4), Position::new(1, 5)));
    ed.add_comment("Rename this", "", 3).unwrap();
    ed.set_selection(Selection::new(Position::new(2, 0), Position::new(2, 4)));
    ed.add_comment("# Tighten", "Ada", 4).unwrap();
    ed.into_document()
}

#[test]
fn comments_are_omitted_by_default() {
    let doc = commented();
    assert_eq!(
        round_trip_with(&doc, &MarkdownOptions::default()),
        "The quick brown fox.\n\n```\nlet x = 1;\n```\n\nLast words\n"
    );
}

#[test]
fn comments_as_html_comments() {
    let doc = commented();
    let o = MarkdownOptions {
        comments: MarkdownComments::HtmlComments,
        ..Default::default()
    };
    let out = to_markdown(&ExportSource::from(&doc), &o);
    // Apart from the comment after the code block, the text is unchanged.
    let mut lines = parse(&out);
    assert_eq!(
        lines.remove(2).kind,
        Kind::Html("<!-- Rename this -->".to_string())
    );
    assert_eq!(lines, expected(&doc, &o));
    assert_eq!(
        out,
        "The quick<!-- Ada: Is it - - really - -> quick? Second line. | Ben: Yes. *Very*. (resolved) --> brown fox.\n\n\
         ```\nlet x = 1;\n```\n\n<!-- Rename this -->\n\nLast<!-- Ada: # Tighten --> words\n"
    );
}

#[test]
fn comments_as_footnotes() {
    let doc = commented();
    let o = MarkdownOptions {
        comments: MarkdownComments::Footnotes,
        ..Default::default()
    };
    let out = to_markdown(&ExportSource::from(&doc), &o);
    assert_eq!(
        out,
        "The quick[^1] brown fox.\n\n```\nlet x = 1;\n```\n\n[^2]\n\nLast[^3] words\n\n\
         [^1]: **Ada (resolved):** Is it -- really --> quick?\\\n    Second line.\n\n    **Ben:** Yes. \\*Very\\*.\n\n\
         [^2]: Rename this\n\n[^3]: **Ada:** \\# Tighten\n"
    );
    // The parser sees three references and three definitions.
    let events: Vec<Event> = Parser::new_ext(&out, parser_options()).collect();
    let refs = events
        .iter()
        .filter(|e| matches!(e, Event::FootnoteReference(_)))
        .count();
    let defs = events
        .iter()
        .filter(|e| matches!(e, Event::Start(Tag::FootnoteDefinition(_))))
        .count();
    assert_eq!((refs, defs), (3, 3));
}

// ----- collections ------------------------------------------------------------------------

#[test]
fn collections() {
    let one = Document::from_blocks(vec![
        Block::with_text(BlockKind::heading(1), "Chapter One"),
        Block::paragraph("First."),
    ]);
    let two = Document::from_blocks(vec![Block::paragraph("Second.")]);
    let empty = Document::new();
    let source = ExportSource::collection(
        "Book",
        [("Chapter One", &one), ("# Two", &two), ("Empty", &empty)],
    );
    assert_eq!(
        to_markdown(&source, &MarkdownOptions::default()),
        "# Chapter One\n\nFirst.\n\n---\n\n# \\# Two\n\nSecond.\n\n---\n\n# Empty\n"
    );
    let o = MarkdownOptions {
        part_titles: PartTitles::Never,
        part_rule: false,
        ..Default::default()
    };
    assert_eq!(
        to_markdown(&source, &o),
        "# Chapter One\n\nFirst.\n\nSecond.\n"
    );
    let o = MarkdownOptions {
        part_titles: PartTitles::Always,
        ..Default::default()
    };
    assert!(to_markdown(&source, &o).starts_with("# Chapter One\n\n# Chapter One\n\nFirst."));
    // A single document never gets its title written.
    assert_eq!(
        to_markdown(&ExportSource::single("Title", &two), &o),
        "Second.\n"
    );
}

// ----- round trips ------------------------------------------------------------------------

#[test]
fn hostile_paragraphs_round_trip() {
    let texts = [
        "*a* **b** ***c*** _d_ __e__ ~f~ ~~g~~ `h` ``i``",
        "[a](b) ![c](d) [e] [f]: g [^h] [^i]: j <k> <l@m.n> <https://o.p>",
        "# a\n## b\n> c\n- d\n+ e\n* f\n1. g\n2) h\n--- \n===\n***\n___\n```\n~~~",
        "a\\b \\* \\\\ \\` \\[ \\n trailing\\",
        "&amp; &lt; &#35; &#x23; &nosuch; & &; &a",
        "<div>block</div> <!-- comment --> <?php ?> <![CDATA[x]]> </close> <br/>",
        "| a | b |\n|---|---|\n| c | d |",
        "a | b\n- | -\n:-: | --:",
        "Setext\n===\nUnder\n---\nDone",
        "👨‍👩‍👧‍👦 👍🏽 🇦🇱 *😀* **€5** ~~™~~",
        "שלום *עולם* مرحبا **بالعالم** [קישור](x)",
        "日本語の*強調*と**太字**。「引用」",
        "tab\there and nbsp\u{a0}there and zero\u{200b}width",
        "    four spaces\n\tone tab\n  - nested?",
        "1986. A year\n1.\n1)\n-\n+\n#\n>",
        "http://example.com www.example.com me@example.com",
        "a  \nb   \nc\\\nd",
        "$math$ $$x$$ ^sup^ ~sub~ ==mark== :smile: @user #123",
        "</u></mark></strong> <u> <mark>",
        "[x] [ ] [X] - [ ] task",
        "*\n*\n**\n_\n`\n~",
    ];
    for t in texts {
        for kind in [
            BlockKind::Paragraph,
            BlockKind::heading(2),
            BlockKind::BulletItem,
            BlockKind::NumberedItem,
            BlockKind::TodoItem { checked: true },
            BlockKind::Quote,
            BlockKind::code(),
        ] {
            round_trip(vec![
                Block::paragraph("before"),
                Block::with_text(kind.clone(), t),
                Block::paragraph("after"),
            ]);
            // The same text with every attribute, and split across marks.
            if !kind.is_code() {
                let all = InlineAttrs {
                    bold: true,
                    italic: true,
                    underline: true,
                    strikethrough: true,
                    highlight: Some(Highlight::Blue),
                    color: Some(Rgb::new(1, 2, 3)),
                    font: Some("Menlo".into()),
                    link: Some("https://example.com/?a=1&b=(2)".into()),
                    ..plain()
                };
                round_trip(vec![Block::new(kind.clone()).with_span(t, all.clone())]);
                round_trip(vec![Block::new(kind.clone()).with_span(t, code())]);
                let mid = t
                    .char_indices()
                    .nth(t.chars().count() / 2)
                    .map_or(0, |(i, _)| i);
                round_trip(vec![
                    Block::new(kind.clone())
                        .with_span(&t[..mid], bold())
                        .with_span(&t[mid..], italic().with(InlineStyle::Strikethrough)),
                ]);
            }
        }
    }
}

#[test]
fn every_attribute_at_once_round_trips() {
    let all = InlineAttrs {
        bold: true,
        italic: true,
        underline: true,
        strikethrough: true,
        code: true,
        highlight: Some(Highlight::Purple),
        color: Some(Rgb::new(200, 10, 10)),
        font: Some("Charter".into()),
        link: Some("https://example.com".into()),
        ..plain()
    };
    let out = round_trip(vec![para().with_span("every`thing", all)]);
    assert_eq!(
        out,
        "[<mark><u>~~***``every`thing``***~~</u></mark>](https://example.com)\n"
    );
}

/// A small deterministic generator (xorshift), so failures reproduce.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }
    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len())]
    }
}

const WORDS: &[&str] = &[
    "word",
    "two words",
    "a",
    "I",
    "x",
    "é",
    "naïve",
    "日本語",
    "שלום",
    "مرحبا",
    "😀",
    "👨‍👩‍👧",
    "€",
    "™",
    "5",
    "42",
];
const HOSTILE: &[&str] = &[
    "*",
    "**",
    "***",
    "_",
    "__",
    "`",
    "``",
    "~",
    "~~",
    "[",
    "]",
    "(",
    ")",
    "<",
    ">",
    "&",
    "&amp;",
    "&#35;",
    "#",
    "##",
    "-",
    "--",
    "---",
    "+",
    "=",
    "===",
    "|",
    ":",
    "!",
    "![",
    "](",
    "\\",
    "\\*",
    "1.",
    "2)",
    "10.",
    ".",
    ",",
    ";",
    "?",
    "\"",
    "'",
    "“",
    "”",
    "—",
    "…",
    "<b>",
    "</b>",
    "<br>",
    "<!--",
    "-->",
    "http://x.y",
    "www.x.y",
    "a@b.c",
    "[^1]",
    "[x]",
    "[ ]",
    "$",
    "^",
    "{",
    "}",
    "%",
    "@",
    "/",
];
const SPACES: &[&str] = &[" ", " ", " ", " ", "  ", "\n", "\t", "\u{a0}"];

fn random_text(rng: &mut Rng, pieces: usize) -> String {
    let mut s = String::new();
    for _ in 0..pieces {
        match rng.below(10) {
            0..=3 => s.push_str(rng.pick(WORDS)),
            4..=6 => s.push_str(rng.pick(HOSTILE)),
            _ => s.push_str(rng.pick(SPACES)),
        }
    }
    s
}

fn random_attrs(rng: &mut Rng) -> InlineAttrs {
    let mut a = plain();
    if rng.chance(50) {
        return a;
    }
    a.bold = rng.chance(35);
    a.italic = rng.chance(35);
    a.strikethrough = rng.chance(20);
    a.underline = rng.chance(15);
    a.code = rng.chance(15);
    if rng.chance(15) {
        a.highlight = Some(Highlight::ALL[rng.below(6)]);
    }
    if rng.chance(10) {
        a.color = Some(Rgb::new(10, 20, 30));
    }
    if rng.chance(20) {
        a.link = Some(
            rng.pick(&[
                "https://example.com",
                "u:1",
                "/path/a_b*c",
                "https://x.y/(z)?a=1&amp;b=<2>",
                "#frag",
            ])
            .to_string(),
        );
    }
    a
}

fn random_block(rng: &mut Rng) -> Block {
    let kind = match rng.below(16) {
        0..=5 => BlockKind::Paragraph,
        6 => BlockKind::heading(1 + rng.below(6) as u8),
        7 | 8 => BlockKind::BulletItem,
        9 | 10 => BlockKind::NumberedItem,
        11 => BlockKind::TodoItem {
            checked: rng.chance(50),
        },
        12 => BlockKind::Quote,
        13 => BlockKind::CodeBlock {
            language: rng
                .chance(50)
                .then(|| rng.pick(&["rust", "c++", "plain text", "`"]).to_string()),
        },
        14 => BlockKind::Divider,
        _ => BlockKind::PageBreak,
    };
    let mut block = Block::new(kind.clone());
    if kind.is_code() {
        let n = rng.below(8);
        let text = random_text(rng, n).replace('\t', " ");
        block = block.with_span(&text, plain());
    } else if !kind.is_void() {
        for _ in 0..rng.below(5) {
            let n = 1 + rng.below(5);
            let text = random_text(rng, n);
            let attrs = random_attrs(rng);
            block = block.with_span(&text, attrs);
        }
    }
    if rng.chance(40) {
        block = block.with_indent(rng.below(4) as u8);
    }
    block
}

#[test]
fn random_documents_round_trip() {
    let seeds: u64 = std::env::var("INNERWORD_EXPORT_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3000);
    for seed in 1..=seeds {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let n = 1 + rng.below(8);
        let blocks: Vec<Block> = (0..n).map(|_| random_block(&mut rng)).collect();
        let doc = Document::from_blocks(blocks);
        let options = match seed % 4 {
            0 => MarkdownOptions::strict(),
            1 => MarkdownOptions {
                colors: true,
                fonts: true,
                ..Default::default()
            },
            _ => MarkdownOptions::default(),
        };
        let result = std::panic::catch_unwind(|| round_trip_with(&doc, &options));
        if let Err(e) = result {
            eprintln!("seed {seed} failed");
            std::panic::resume_unwind(e);
        }
    }
}

/// Many short runs with overlapping marks in one block: the hard case for
/// delimiter placement.
#[test]
fn random_inline_marks_round_trip() {
    let seeds: u64 = std::env::var("INNERWORD_EXPORT_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3000);
    for seed in 1..=seeds {
        let mut rng = Rng(seed.wrapping_mul(0xD1B5_4A32_D192_ED03) | 1);
        let kind = match rng.below(5) {
            0 => BlockKind::heading(2),
            1 => BlockKind::BulletItem,
            2 => BlockKind::Quote,
            _ => BlockKind::Paragraph,
        };
        let mut block = Block::new(kind);
        // Marks switch on and off independently, so they overlap partially.
        let mut attrs = plain();
        for _ in 0..(3 + rng.below(12)) {
            match rng.below(9) {
                0 | 1 => attrs.bold = !attrs.bold,
                2 | 3 => attrs.italic = !attrs.italic,
                4 => attrs.strikethrough = !attrs.strikethrough,
                5 => attrs.underline = !attrs.underline,
                6 => attrs.code = !attrs.code,
                7 => {
                    attrs.link = if attrs.link.is_some() {
                        None
                    } else {
                        Some("https://example.com/a".to_string())
                    }
                }
                _ => {
                    attrs.highlight = if attrs.highlight.is_some() {
                        None
                    } else {
                        Some(Highlight::Yellow)
                    }
                }
            }
            let text = match rng.below(6) {
                0 => rng.pick(HOSTILE).to_string(),
                1 => format!(" {}", rng.pick(WORDS)),
                2 => format!("{} ", rng.pick(WORDS)),
                3 => rng.pick(SPACES).to_string(),
                _ => rng.pick(WORDS).to_string(),
            };
            block = block.with_span(&text, attrs.clone());
        }
        let doc = Document::from_blocks(vec![block]);
        let options = if seed % 5 == 0 {
            MarkdownOptions::strict()
        } else {
            MarkdownOptions::default()
        };
        let result = std::panic::catch_unwind(|| round_trip_with(&doc, &options));
        match result {
            Ok(out) if seed <= 12 && std::env::var("INNERWORD_EXPORT_SHOW").is_ok() => {
                eprintln!("{out}")
            }
            Ok(out) => dump_case(10_000_000 + seed, &doc, &options, &out),
            Err(e) => {
                eprintln!("seed {seed} failed");
                std::panic::resume_unwind(e);
            }
        }
    }
}

/// Comments at random places, in both comment modes: the text and the
/// structure must not change, whatever the comment bodies contain.
#[test]
fn random_comments_do_not_disturb_the_text() {
    let seeds: u64 = std::env::var("INNERWORD_EXPORT_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3000);
    for seed in 1..=seeds.min(20_000) {
        let mut rng = Rng(seed.wrapping_mul(0xA24B_AED4_963E_E407) | 1);
        let n = 1 + rng.below(5);
        let blocks: Vec<Block> = (0..n).map(|_| random_block(&mut rng)).collect();
        let mut ed = EditorState::new(Document::from_blocks(blocks));
        for _ in 0..(1 + rng.below(3)) {
            let count = ed.document().block_count();
            let (a, b) = (rng.below(count), rng.below(count));
            let pos = |rng: &mut Rng, block: usize| {
                let len = ed.document().blocks()[block].len();
                ed.document()
                    .clamp_position(Position::new(block, rng.below(len + 1)))
            };
            let (start, end) = (pos(&mut rng, a), pos(&mut rng, b));
            ed.set_selection(Selection::new(start, end));
            let body = random_text(&mut rng, 4);
            if let Some(id) = ed.add_comment(&body, rng.pick(&["Ada", "", "*B*", "--> x"]), 1)
                && rng.chance(40)
            {
                let reply = random_text(&mut rng, 3);
                ed.add_reply(id, &reply, "Ben -- B", 2);
            }
        }
        let doc = ed.into_document();
        for comments in [MarkdownComments::HtmlComments, MarkdownComments::Footnotes] {
            let options = MarkdownOptions {
                comments,
                ..Default::default()
            };
            let out = to_markdown(&ExportSource::from(&doc), &options);
            // Apart from the comment-only blocks, the parser must see the
            // same document as without comments.
            let result = std::panic::catch_unwind(|| {
                let got: Vec<Line> = parse(&out)
                    .into_iter()
                    .filter(|l| !matches!(&l.kind, Kind::Html(h) if h.starts_with("<!--") && h != "<!-- pagebreak -->"))
                    .filter(|l| !(l.kind == Kind::Para && l.text.is_empty()))
                    .collect();
                let want = expected(&doc, &options);
                assert_eq!(got, want, "seed {seed} {comments:?}\n{out}");
            });
            if let Err(e) = result {
                eprintln!("seed {seed} failed with {comments:?}:\n{out}");
                for b in doc.blocks() {
                    eprintln!(
                        "{:?} indent {} {:?} {:?}",
                        b.kind(),
                        b.indent(),
                        b.text(),
                        b.runs()
                            .iter()
                            .map(|r| (r.len, r.attrs.comments.clone()))
                            .collect::<Vec<_>>()
                    );
                }
                std::panic::resume_unwind(e);
            }
        }
    }
}

// ----- second, independent generator (review pass) ----------------------------------------

/// Text that tries to start a block when it lands at the start of a line.
const LINE_STARTS: &[&str] = &[
    "# ",
    "#",
    "## x",
    "####### x",
    "#\t",
    "> ",
    ">",
    ">x",
    "- ",
    "-",
    "-\t",
    "+ ",
    "+",
    "* ",
    "*",
    "1. ",
    "1.",
    "1) ",
    "0. ",
    "123456789. ",
    "1234567890. ",
    "007. ",
    "```",
    "````",
    "~~~",
    "~~~~",
    "```rust",
    "===",
    "=",
    "== ",
    "---",
    "- - -",
    "***",
    "* * *",
    "___",
    "_ _ _",
    "-- -",
    "|",
    "| a | b |",
    "|---|---|",
    "---|---",
    ":--",
    ":-:",
    "--:|",
    "a | b",
    "[x]: /url",
    "[x]: <u> \"t\"",
    "[^1]: note",
    "[^n]:",
    "<div>",
    "</div>",
    "<!-- x -->",
    "<!--",
    "<script>",
    "<pre>",
    "<style>",
    "<?php",
    "<![CDATA[",
    "<a href=\"x\">",
    "<b>",
    "</b>",
    "<br/>",
    "<img src=x>",
    "<x-y>",
    "<1>",
    "< a",
    "<",
    "[ ] ",
    "[x] ",
    "[X] x",
    "- [ ] x",
    "\\",
    "\\\\",
    "\\#",
    "&nbsp;",
    "&#x41;",
    "&#65;",
    "&copy;",
    "&",
    "&x",
    "&;",
    ":",
    ": x",
    "::",
    "%%",
    "@",
    "$",
    "$$",
    "{",
    "}",
    "^",
    "!",
    "![",
    "!\\[",
    "=",
    "==x==",
    "+++",
    "---x",
    "    x",
    "\tx",
    "1.x",
    "1.5",
    "1986. A year",
    "-5",
    "+1",
    "*x",
    "_x",
    "~x",
    "~~~x",
];

/// Inline constructs written as literal text: every one must come back as text.
const INLINE_HOSTILE: &[&str] = &[
    "*a*",
    "**a**",
    "***a***",
    "_a_",
    "__a__",
    "*a",
    "a*",
    "a*b*c",
    "a_b_c",
    "2*3*4",
    "snake_case_x",
    "__init__",
    "~a~",
    "~~a~~",
    "~~~a~~~",
    "`a`",
    "``a``",
    "`",
    "a`b",
    "[a](b)",
    "[a](<b c>)",
    "[a][b]",
    "[a]",
    "[a]:",
    "[]",
    "[](x)",
    "![a](b)",
    "!",
    "![",
    "](",
    "](x)",
    "[[a]]",
    "]]",
    "<http://x.y>",
    "<a@b.c>",
    "<mailto:a@b.c>",
    "http://x.y/a_b_c",
    "https://x.y/*a*",
    "www.x.y",
    "a@b.c",
    "<b>x</b>",
    "<i>",
    "<!---->",
    "<!-- -->",
    "-->",
    "&amp;",
    "&lt;",
    "&#35;",
    "&#x23;",
    "&unknown;",
    "&amp",
    "\\*",
    "\\\\",
    "\\",
    "\\n",
    "\\`",
    "\\[",
    "\\<",
    "a\\",
    "*",
    "**",
    "_",
    "__",
    "~",
    "~~",
    "^a^",
    "$a$",
    "==a==",
    "++a++",
    ":a:",
    "@a",
    "#a",
    "#",
    "a#",
    " #",
    "##",
    "|",
    "a|b",
    "(",
    ")",
    "((",
    "))",
    "()",
    "(a)",
    "[",
    "]",
    "{",
    "}",
    "<",
    ">",
    "<<",
    ">>",
    "< >",
    "<>",
    "'",
    "\"",
    "“",
    "”",
    "‘",
    "’",
    "—",
    "–",
    "…",
    ".",
    ",",
    ";",
    ":",
    "?",
    "!?",
    "/",
    "%",
    "=",
    "+",
    "-",
    "€",
    "£",
    "™",
    "©",
    "°",
    "×",
    "😀",
    "👍🏽",
    "🇳🇱",
    "\u{200b}",
    "\u{200d}",
    "\u{feff}",
    "\u{ad}",
    "\u{2060}",
    "\u{202e}",
    "\u{3000}",
    "\u{2003}",
    "\u{2028}",
    "\u{85}",
    "\u{b}",
    "\u{c}",
];

const URLS: &[&str] = &[
    "https://example.com",
    "https://example.com/a b",
    "https://en.wikipedia.org/wiki/A_(b)",
    "https://x.y/(",
    "https://x.y/)",
    "https://x.y/((a)",
    "https://x.y/a)b(c",
    "https://x.y/[a]",
    "https://x.y/]",
    "https://x.y/<a>",
    "https://x.y/a\\b",
    "https://x.y/\"q\"",
    "https://x.y/'q'",
    "https://x.y/?a=1&b=2",
    "https://x.y/?a=1&amp;b",
    "https://x.y/&#35;",
    "https://x.y/*a*_b_`c`",
    "https://x.y/a#b",
    "https://x.y/%20%zz",
    "https://日本.jp/パス",
    "https://x.y/😀",
    "mailto:a@b.c",
    "tel:+1-555",
    "/relative/path",
    "relative path",
    "#frag",
    "?q",
    "x",
    "<",
    ">",
    "<>",
    "()",
    "(",
    ")",
    "\\",
    "a\\",
    "\\)",
    " https://padded.example ",
    "javascript:alert(1)",
    "u:1",
    "http://x.y/\ttab",
    "https://x.y/a\u{a0}b",
    "https://x.y/a\u{2028}b",
    "*",
    "_",
    "`",
    "[",
    "]",
    "!",
    "#",
    "&",
    "&amp;",
    "https://x.y/~a~~b~~",
    "title \"x\"",
    "a 'b'",
    "a (b)",
];

fn adv_text(rng: &mut Rng, pieces: usize) -> String {
    let mut s = String::new();
    for _ in 0..pieces {
        match rng.below(12) {
            0..=3 => s.push_str(rng.pick(WORDS)),
            4 | 5 => s.push_str(rng.pick(INLINE_HOSTILE)),
            6 => s.push_str(rng.pick(HOSTILE)),
            7 => {
                // A line start, sometimes really at the start of a line.
                if rng.chance(70) {
                    s.push('\n');
                }
                s.push_str(rng.pick(LINE_STARTS));
            }
            _ => s.push_str(rng.pick(SPACES)),
        }
    }
    s
}

fn adv_attrs(rng: &mut Rng) -> InlineAttrs {
    let mut a = plain();
    if rng.chance(45) {
        return a;
    }
    a.bold = rng.chance(30);
    a.italic = rng.chance(30);
    a.strikethrough = rng.chance(20);
    a.underline = rng.chance(15);
    a.code = rng.chance(20);
    if rng.chance(12) {
        a.highlight = Some(Highlight::ALL[rng.below(6)]);
    }
    if rng.chance(10) {
        a.color = Some(Rgb::new(10, 20, 30));
    }
    if rng.chance(8) {
        a.font = Some(
            rng.pick(&["Menlo", "A 'B'", "x;y:z", "<b>", "Gill Sans"])
                .to_string(),
        );
    }
    if rng.chance(30) {
        a.link = Some(rng.pick(URLS).to_string());
    }
    a
}

const CODE_LINES: &[&str] = &[
    "```",
    "````",
    "`````",
    "~~~",
    "~~~~",
    "``` rust",
    "```\n```",
    "    indented",
    "\tx",
    "",
    "",
    " ",
    "fn main() {",
    "}",
    "# not a heading",
    "- not a list",
    "> not a quote",
    "1. x",
    "<div>",
    "</div>",
    "<!-- x -->",
    "*a* **b** `c`",
    "\\",
    "a  ",
    "  a",
    "---",
    "===",
    "[x]: y",
    "| a |",
    "😀 שלום",
    "`",
    "``",
    "a ``` b",
    "~",
    "end",
];

fn adv_block(rng: &mut Rng) -> Block {
    let kind = match rng.below(20) {
        0..=5 => BlockKind::Paragraph,
        6 => BlockKind::heading(1 + rng.below(6) as u8),
        7..=9 => BlockKind::BulletItem,
        10..=12 => BlockKind::NumberedItem,
        13 | 14 => BlockKind::TodoItem {
            checked: rng.chance(50),
        },
        15 | 16 => BlockKind::Quote,
        17 => BlockKind::CodeBlock {
            language: rng.chance(60).then(|| {
                rng.pick(&[
                    "rust",
                    "c++",
                    "c#",
                    "plain text",
                    "`",
                    "~~~",
                    "a`b",
                    " ",
                    "",
                    "日本",
                    "{x}",
                ])
                .to_string()
            }),
        },
        18 => BlockKind::Divider,
        _ => BlockKind::PageBreak,
    };
    let mut block = Block::new(kind.clone());
    if kind.is_code() {
        let lines: Vec<&str> = (0..rng.below(6)).map(|_| rng.pick(CODE_LINES)).collect();
        block = block.with_span(&lines.join("\n"), plain());
    } else if !kind.is_void() {
        for _ in 0..rng.below(6) {
            let n = 1 + rng.below(4);
            let text = adv_text(rng, n);
            let attrs = adv_attrs(rng);
            block = block.with_span(&text, attrs);
        }
    }
    if rng.chance(55) {
        block = block.with_indent(rng.below(9) as u8);
    }
    block
}

fn adv_options(rng: &mut Rng) -> MarkdownOptions {
    MarkdownOptions {
        underline_html: rng.chance(70),
        highlight: if rng.chance(70) {
            MarkdownHighlight::Html
        } else {
            MarkdownHighlight::Drop
        },
        colors: rng.chance(30),
        fonts: rng.chance(30),
        ..Default::default()
    }
}

fn seeds(default: u64) -> u64 {
    std::env::var("INNERWORD_EXPORT_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

#[test]
fn adversarial_documents_round_trip() {
    let first: u64 = std::env::var("INNERWORD_EXPORT_FIRST")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    for seed in first..first + seeds(3000) {
        let mut rng = Rng(seed.wrapping_mul(0xC2B2_AE3D_27D4_EB4F) | 1);
        let n = 1 + rng.below(10);
        let blocks: Vec<Block> = (0..n).map(|_| adv_block(&mut rng)).collect();
        let doc = Document::from_blocks(blocks);
        let options = adv_options(&mut rng);
        let result = std::panic::catch_unwind(|| round_trip_with(&doc, &options));
        match result {
            Ok(out) => dump_case(seed, &doc, &options, &out),
            Err(e) => {
                eprintln!("seed {seed} failed");
                std::panic::resume_unwind(e);
            }
        }
    }
}

/// With `INNERWORD_EXPORT_DUMP_DIR` set, writes the Markdown of a case as
/// `<seed>.md` and what a parser should find as `<seed>.expect`, so that
/// parsers other than the one linked into the tests can be run over the
/// same corpus. One line per block: kind, depth, the text as hex, and two
/// hex digits of style flags per character (1 bold, 2 italic, 4 strike,
/// 8 code, 16 underline, 32 highlight, 64 link).
fn dump_case(seed: u64, doc: &Document, options: &MarkdownOptions, markdown: &str) {
    let Ok(dir) = std::env::var("INNERWORD_EXPORT_DUMP_DIR") else {
        return;
    };
    let mut expect = String::new();
    for line in expected(doc, options) {
        let kind = match &line.kind {
            Kind::Para => "para".to_string(),
            Kind::Heading(n) => format!("h{n}"),
            Kind::Bullet => "bullet".to_string(),
            Kind::Numbered(n) => format!("num{n}"),
            Kind::Todo(done) => format!("todo{}", u8::from(*done)),
            Kind::Quote => "quote".to_string(),
            Kind::Code(lang) => format!("code:{lang}"),
            Kind::Rule => "rule".to_string(),
            Kind::Html(_) => "html".to_string(),
            Kind::Image(destination) => format!("image:{destination}"),
        };
        let text: String = line.text.iter().map(|(c, _)| *c).collect();
        let hex: String = text.bytes().map(|b| format!("{b:02x}")).collect();
        let flags: String = line
            .text
            .iter()
            .map(|(_, s)| {
                let f = u8::from(s.bold)
                    | u8::from(s.italic) << 1
                    | u8::from(s.strike) << 2
                    | u8::from(s.code) << 3
                    | u8::from(s.underline) << 4
                    | u8::from(s.highlight) << 5
                    | u8::from(s.link.is_some()) << 6;
                format!("{f:02x}")
            })
            .collect();
        expect.push_str(&format!("{kind}\t{}\t{hex}\t{flags}\n", line.depth));
    }
    std::fs::write(format!("{dir}/{seed}.md"), markdown).unwrap();
    std::fs::write(format!("{dir}/{seed}.expect"), expect).unwrap();
}

/// One block with a great many short runs, most of them in places where
/// the delimiters cannot pair: the exporter replaces them in batches, and
/// the result must still read back exactly.
#[test]
fn dense_paragraphs_round_trip() {
    for seed in 1..=seeds(3000) / 10 {
        let mut rng = Rng(seed.wrapping_mul(0x8CB9_2BA7_2F3D_8DD7) | 1);
        let mut block = Block::new(BlockKind::Paragraph);
        let pieces = [
            "(b)",
            "t",
            "e.",
            "n, ",
            "文字。",
            "次の ",
            " s ",
            "x",
            "*",
            "_",
            "~",
            "€",
            "😀",
            "a",
            "!",
            "\n",
        ];
        let mut attrs = plain();
        for _ in 0..(30 + rng.below(120)) {
            match rng.below(6) {
                0 | 1 => attrs.bold = !attrs.bold,
                2 | 3 => attrs.italic = !attrs.italic,
                4 => attrs.strikethrough = !attrs.strikethrough,
                _ => {
                    attrs.link = match attrs.link {
                        Some(_) => None,
                        None => Some("u:1".to_string()),
                    }
                }
            }
            block = block.with_span(rng.pick(&pieces), attrs.clone());
        }
        let doc = Document::from_blocks(vec![block]);
        let options = MarkdownOptions::default();
        let result = std::panic::catch_unwind(|| round_trip_with(&doc, &options));
        match result {
            Ok(out) => dump_case(20_000_000 + seed, &doc, &options, &out),
            Err(e) => {
                eprintln!("seed {seed} failed");
                std::panic::resume_unwind(e);
            }
        }
    }
}

/// Comment bodies and author names with hostile text come back from the
/// footnotes as written: nothing in them is read as markup.
#[test]
fn footnote_comments_keep_their_text() {
    for seed in 1..=seeds(3000).min(200_000) {
        let mut rng = Rng(seed.wrapping_mul(0xE703_7ED1_A0B4_28DB) | 1);
        let mut ed = EditorState::new(Document::from_blocks(vec![
            Block::paragraph("one two three"),
            Block::with_text(BlockKind::BulletItem, "item text"),
        ]));
        let mut want: Vec<String> = Vec::new();
        for (block, (from, to)) in [(0, (0, 3)), (0, (4, 7)), (1, (0, 4))] {
            ed.set_selection(Selection::new(
                Position::new(block, from),
                Position::new(block, to),
            ));
            let mut entries: Vec<(String, String)> = Vec::new();
            for _ in 0..1 + rng.below(3) {
                let author = match rng.below(4) {
                    0 => String::new(),
                    1 => "Ada".to_string(),
                    _ => adv_text(&mut rng, 2),
                };
                let n = 1 + rng.below(5);
                entries.push((author, adv_text(&mut rng, n)));
            }
            let resolved = rng.chance(30);
            let Some(id) = ed.add_comment(&entries[0].1, &entries[0].0, 1) else {
                continue;
            };
            for (author, body) in &entries[1..] {
                ed.add_reply(id, body, author, 2);
            }
            if resolved {
                ed.resolve_comment(id);
            }
            // What the note should read as: one paragraph per entry.
            let thread = ed.document().comment(id).unwrap().clone();
            let mut stored = vec![(thread.author.clone(), thread.body.clone())];
            stored.extend(
                thread
                    .replies
                    .iter()
                    .map(|r| (r.author.clone(), r.body.clone())),
            );
            let mut paragraphs = Vec::new();
            for (i, (author, body)) in stored.iter().enumerate() {
                let mut lines: Vec<&str> = body.split('\n').map(str::trim).collect();
                while lines.last().is_some_and(|l| l.is_empty()) {
                    lines.pop();
                }
                let lead = lines.iter().take_while(|l| l.is_empty()).count();
                lines.drain(..lead);
                let text = lines.join("\n");
                let author = author.split_whitespace().collect::<Vec<_>>().join(" ");
                let mark = if i == 0 && resolved {
                    " (resolved)"
                } else {
                    ""
                };
                let p = if !author.is_empty() {
                    if text.is_empty() {
                        format!("{author}{mark}:")
                    } else {
                        format!("{author}{mark}: {text}")
                    }
                } else {
                    format!("{text}{mark}").trim().to_string()
                };
                if !p.is_empty() {
                    paragraphs.push(p);
                }
            }
            if paragraphs.is_empty() {
                paragraphs.push("(empty comment)".to_string());
            }
            want.push(paragraphs.join("\n\n"));
        }
        let doc = ed.into_document();
        let out = to_markdown(
            &ExportSource::from(&doc),
            &MarkdownOptions {
                comments: MarkdownComments::Footnotes,
                ..Default::default()
            },
        );
        let mut got: Vec<String> = Vec::new();
        let mut inside = false;
        let mut refs = 0;
        for event in Parser::new_ext(&out, parser_options()) {
            match event {
                Event::Start(Tag::FootnoteDefinition(_)) => {
                    inside = true;
                    got.push(String::new());
                }
                Event::End(TagEnd::FootnoteDefinition) => inside = false,
                Event::FootnoteReference(_) => refs += 1,
                _ if !inside => {}
                Event::Text(t) => got.last_mut().unwrap().push_str(&t),
                Event::HardBreak => got.last_mut().unwrap().push('\n'),
                Event::Start(Tag::Paragraph) => {
                    let note = got.last_mut().unwrap();
                    if !note.is_empty() {
                        note.push_str("\n\n");
                    }
                }
                Event::End(TagEnd::Paragraph)
                | Event::Start(Tag::Strong)
                | Event::End(TagEnd::Strong) => {}
                other => panic!("seed {seed}: {other:?} in a footnote:\n{out}"),
            }
        }
        // Whitespace inside a line is not compared (Markdown collapses none
        // of it, but no-break and other spaces at line edges are trimmed).
        let squash = |s: &str| {
            s.split('\n')
                .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let got: Vec<String> = got.iter().map(|s| squash(s)).collect();
        let want: Vec<String> = want.iter().map(|s| squash(s)).collect();
        assert_eq!(got, want, "seed {seed}\n{out}");
        assert_eq!(refs, want.len(), "seed {seed}\n{out}");
    }
}

// ----- import: the corpus and the round trips with the importer ---------------------------

mod import;
