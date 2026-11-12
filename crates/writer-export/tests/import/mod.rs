//! Import tests: a corpus of hand-written Markdown with the blocks it must
//! give, and the round trips with the exporter (a document that is
//! exported and imported comes back; Markdown that is imported and
//! exported is stable from the second pass on).
//!
//! A submodule of the Markdown export tests, whose generators and
//! comparison it shares.

use super::*;
use writer_export::{
    ImportWarningKind, Imported, MarkdownFrontMatter, MarkdownImportOptions,
    MarkdownLeadingHeading, MarkdownSoftBreaks, from_markdown, summarize_warnings,
};

// ----- a readable form of a document ------------------------------------------------------

fn describe_block(b: &Block) -> String {
    let mut head = match b.kind() {
        BlockKind::Paragraph => "p".to_string(),
        BlockKind::Heading { level } => format!("h{level}"),
        BlockKind::BulletItem => "bullet".to_string(),
        BlockKind::NumberedItem => "numbered".to_string(),
        BlockKind::TodoItem { checked: true } => "todo[x]".to_string(),
        BlockKind::TodoItem { checked: false } => "todo[ ]".to_string(),
        BlockKind::Quote => "quote".to_string(),
        BlockKind::CodeBlock { language: None } => "code".to_string(),
        BlockKind::CodeBlock { language: Some(l) } => format!("code({l})"),
        BlockKind::Divider => "divider".to_string(),
        BlockKind::PageBreak => "pagebreak".to_string(),
    };
    if b.indent() > 0 {
        head.push_str(&format!("+{}", b.indent()));
    }
    if b.alignment() != Alignment::Left {
        head.push_str(&format!(" ({})", b.alignment().name()));
    }
    if b.text().is_empty() {
        return head;
    }
    let mut out = format!("{head}: ");
    for (_, text, a) in b.text_spans() {
        let text = text.replace('\n', "\\n").replace('\t', "\\t");
        let mut tags: Vec<String> = Vec::new();
        for (on, tag) in [
            (a.bold, "B"),
            (a.italic, "I"),
            (a.strikethrough, "S"),
            (a.underline, "U"),
            (a.code, "C"),
        ] {
            if on {
                tags.push(tag.to_string());
            }
        }
        match a.highlight {
            Some(Highlight::Yellow) => tags.push("H".to_string()),
            Some(h) => tags.push(format!("H={}", h.name())),
            None => {}
        }
        if let Some(c) = a.color {
            tags.push(format!("K={}", c.to_hex()));
        }
        if let Some(f) = &a.font {
            tags.push(format!("F={f}"));
        }
        if let Some(l) = &a.link {
            tags.push(format!("L={l}"));
        }
        if tags.is_empty() {
            out.push_str(&text);
        } else {
            out.push_str(&format!("[{}|{text}]", tags.join(" ")));
        }
    }
    out
}

fn describe(imported: &Imported) -> Vec<String> {
    if imported.is_empty() {
        return Vec::new();
    }
    imported
        .document
        .blocks()
        .iter()
        .map(describe_block)
        .collect()
}

fn import_with(markdown: &str, options: &MarkdownImportOptions) -> Imported {
    let imported = from_markdown(markdown, options);
    imported
        .document
        .check_invariants()
        .unwrap_or_else(|e| panic!("{e}\nfor:\n{markdown}"));
    assert_eq!(imported.document.settings(), &options.settings);
    assert_eq!(
        imported.is_empty(),
        imported.fragment().blocks().is_empty(),
        "{markdown:?}"
    );
    imported
}

fn import(markdown: &str) -> Imported {
    import_with(markdown, &MarkdownImportOptions::default())
}

/// The blocks of Markdown that must import without a warning.
#[track_caller]
fn blocks(markdown: &str) -> Vec<String> {
    let imported = import(markdown);
    assert_eq!(imported.warnings, [], "for:\n{markdown}");
    describe(&imported)
}

/// The blocks and the kinds of the warnings.
fn warned(markdown: &str) -> (Vec<String>, Vec<ImportWarningKind>) {
    let imported = import(markdown);
    let kinds = imported.warnings.iter().map(|w| w.kind).collect();
    (describe(&imported), kinds)
}

fn export(doc: &Document) -> String {
    to_markdown(&ExportSource::from(doc), &MarkdownOptions::default())
}

// ----- the corpus: what agents typically write --------------------------------------------

#[test]
fn nothing() {
    for empty in ["", "\n", "   \n\t\n", "\r\n\r\n", "\u{feff}", "\u{feff}\n"] {
        let imported = import(empty);
        assert!(imported.is_empty(), "{empty:?}");
        assert_eq!(imported.document, Document::new());
        assert_eq!(imported.title, None);
        assert_eq!(imported.front_matter, None);
        assert_eq!(imported.warnings, []);
        assert!(imported.fragment().blocks().is_empty());
        assert_eq!(export(&imported.document), "");
    }
}

#[test]
fn a_heading_and_paragraphs() {
    let md = "# The Salt Road\n\nIt rained for *three* days.\n\nOn the fourth, **nothing**.\n";
    assert_eq!(
        blocks(md),
        [
            "h1: The Salt Road",
            "p: It rained for [I|three] days.",
            "p: On the fourth, [B|nothing].",
        ]
    );
    let imported = import(md);
    assert_eq!(imported.title.as_deref(), Some("The Salt Road"));
    assert_eq!(imported.document.title().as_deref(), Some("The Salt Road"));
    assert_eq!(export(&imported.document), md);
    // Every level, both spellings, closing hashes.
    assert_eq!(
        blocks("# 1\n## 2\n### 3\n#### 4\n##### 5\n###### 6 ######\n####### 7\n\nA\n=\n\nB\n-\n"),
        [
            "h1: 1",
            "h2: 2",
            "h3: 3",
            "h4: 4",
            "h5: 5",
            "h6: 6",
            "p: ####### 7",
            "h1: A",
            "h2: B"
        ]
    );
    assert_eq!(blocks("#\n\n## \n"), ["h1", "h2"]);
    assert_eq!(
        blocks("#hashtag and # in text"),
        ["p: #hashtag and # in text"]
    );
}

#[test]
fn inline_formatting() {
    assert_eq!(
        blocks("**bold** *italic* _italic_ __bold__ ~~gone~~ `code` ***both***"),
        ["p: [B|bold] [I|italic] [I|italic] [B|bold] [S|gone] [C|code] [B I|both]"]
    );
    assert_eq!(
        blocks("**bold with *italic* and `code` inside**"),
        ["p: [B|bold with ][B I|italic][B| and ][B C|code][B| inside]"]
    );
    assert_eq!(
        blocks("a*b*c snake_case_name 2 * 3 * 4 \\*literal\\* \\# &amp; &copy; &#35;"),
        ["p: a[I|b]c snake_case_name 2 * 3 * 4 *literal* # & © #"]
    );
    assert_eq!(
        blocks("`` a`b `` and ` ` and `  x  `"),
        ["p: [C|a`b] and [C| ] and [C| x ]"]
    );
}

#[test]
fn links() {
    assert_eq!(
        blocks("See [the *full* report](https://example.com/r?a=1&b=2) now."),
        [
            "p: See [L=https://example.com/r?a=1&b=2|the ][I L=https://example.com/r?a=1&b=2|full][L=https://example.com/r?a=1&b=2| report] now."
        ]
    );
    assert_eq!(
        blocks("**[bold link](u:1)** and [`code`](u:2)"),
        ["p: [B L=u:1|bold link] and [C L=u:2|code]"]
    );
    assert_eq!(
        blocks("<https://example.com> and <me@example.com>"),
        [
            "p: [L=https://example.com|https://example.com] and [L=mailto:me@example.com|me@example.com]"
        ]
    );
    assert_eq!(
        blocks(
            "[ref][1] and [collapsed][] and [shortcut]\n\n[1]: /one\n[collapsed]: /two\n[shortcut]: </three four>\n"
        ),
        ["p: [L=/one|ref] and [L=/two|collapsed] and [L=/three four|shortcut]"]
    );
    assert_eq!(
        blocks("[not a link] and [empty]() and bare https://example.com"),
        ["p: [not a link] and empty and bare https://example.com"]
    );
    let (b, w) = warned("[titled](https://example.com \"The title\")");
    assert_eq!(b, ["p: [L=https://example.com|titled]"]);
    assert_eq!(w, [ImportWarningKind::LinkTitle]);
}

#[test]
fn line_breaks() {
    let md = "one\ntwo  \nthree\\\nfour\n";
    assert_eq!(blocks(md), ["p: one two\\nthree\\nfour"]);
    let lines = MarkdownImportOptions {
        soft_breaks: MarkdownSoftBreaks::LineBreak,
        ..Default::default()
    };
    assert_eq!(
        describe(&import_with(md, &lines)),
        ["p: one\\ntwo\\nthree\\nfour"]
    );
    assert_eq!(
        describe(&import_with(
            "> Roses are red\n> Violets are blue\n",
            &lines
        )),
        ["quote: Roses are red\\nViolets are blue"]
    );
    assert_eq!(blocks("a<br>b<br/>c <BR />\nd"), ["p: a\\nb\\nc\\nd"]);
    assert_eq!(blocks("One\\\ntwo\n==="), ["h1: One\\ntwo"]);
}

#[test]
fn bullet_lists_nest_by_indent() {
    let md = "- one\n- two\n  - two a\n  - two b\n    - deep\n- three\n";
    assert_eq!(
        blocks(md),
        [
            "bullet: one",
            "bullet: two",
            "bullet+1: two a",
            "bullet+1: two b",
            "bullet+2: deep",
            "bullet: three"
        ]
    );
    assert_eq!(export(&import(md).document), md);
    // Every marker, and a list that is loose.
    assert_eq!(
        blocks("* a\n\n* b\n\n+ c\n"),
        ["bullet: a", "bullet: b", "p", "bullet: c"]
    );
    // Four spaces and tabs nest too.
    assert_eq!(
        blocks("- a\n    - b\n\t- c\n"),
        ["bullet: a", "bullet+1: b", "bullet+1: c"]
    );
}

#[test]
fn numbered_lists() {
    let md = "1. first\n2. second\n   1. inner\n   2. inner two\n3. third\n";
    assert_eq!(
        blocks(md),
        [
            "numbered: first",
            "numbered: second",
            "numbered+1: inner",
            "numbered+1: inner two",
            "numbered: third"
        ]
    );
    assert_eq!(export(&import(md).document), md);
    // Lazy numbering is renumbered by the document without a warning.
    assert_eq!(
        blocks("1. a\n1. b\n1. c\n"),
        ["numbered: a", "numbered: b", "numbered: c"]
    );
    assert_eq!(
        export(&import("1. a\n1. b\n1) c\n").document),
        "1. a\n2. b\n\n1) c\n"
    );
    let (b, w) = warned("7. seven\n8. eight\n");
    assert_eq!(b, ["numbered: seven", "numbered: eight"]);
    assert_eq!(w, [ImportWarningKind::ListStart]);
    // A list that goes on after a paragraph starts again at 1.
    let (b, w) = warned("1. a\n\ntext\n\n2. b\n");
    assert_eq!(b, ["numbered: a", "p: text", "numbered: b"]);
    assert_eq!(w, [ImportWarningKind::ListStart]);
    // Mixed kinds.
    assert_eq!(
        blocks("1. a\n   - b\n   - c\n2. d\n\n- e\n"),
        [
            "numbered: a",
            "bullet+1: b",
            "bullet+1: c",
            "numbered: d",
            "bullet: e"
        ]
    );
}

#[test]
fn task_lists() {
    let md = "- [ ] open\n- [x] done\n- [X] also done\n  - [ ] nested\n- plain\n";
    assert_eq!(
        blocks(md),
        [
            "todo[ ]: open",
            "todo[x]: done",
            "todo[x]: also done",
            "todo[ ]+1: nested",
            "bullet: plain"
        ]
    );
    assert_eq!(blocks("1. [x] numbered task\n"), ["todo[x]: numbered task"]);
    assert_eq!(
        blocks("- [ ] loose\n\n- [x] list\n"),
        ["todo[ ]: loose", "todo[x]: list"]
    );
    assert_eq!(
        blocks("- [ ]\n- [y] not a task"),
        ["todo[ ]", "bullet: [y] not a task"]
    );
    // The exporter's filler for an empty to-do with children.
    assert_eq!(
        blocks("- [ ] <!-- -->\n  - child\n"),
        ["todo[ ]", "bullet+1: child"]
    );
}

#[test]
fn separate_lists_stay_separate() {
    // Two lists of one sort in a row: the document would run them
    // together, so an empty paragraph keeps them apart.
    assert_eq!(
        blocks("1. a\n2. b\n\n1) c\n2) d\n"),
        [
            "numbered: a",
            "numbered: b",
            "p",
            "numbered: c",
            "numbered: d"
        ]
    );
    assert_eq!(blocks("- a\n\n* b\n"), ["bullet: a", "p", "bullet: b"]);
    assert_eq!(
        blocks("- x\n  1. a\n  \n  1) b\n- y\n"),
        [
            "bullet: x",
            "numbered+1: a",
            "p+1",
            "numbered+1: b",
            "bullet: y"
        ]
    );
    // Different sorts need nothing.
    assert_eq!(
        blocks("- a\n1. b\n- c\n"),
        ["bullet: a", "numbered: b", "bullet: c"]
    );
    // And the export keeps them apart again.
    let md = "1. a\n\n1) b\n";
    assert_eq!(export(&import(md).document), md);
}

#[test]
fn blocks_inside_list_items() {
    let md = "1. Install\n\n   ```sh\n   brew install x\n   ```\n\n   Then restart.\n\n   > Careful.\n\n2. Run\n";
    assert_eq!(
        blocks(md),
        [
            "numbered: Install",
            "code(sh)+1: brew install x",
            "p+1: Then restart.",
            "quote+1: Careful.",
            "numbered: Run",
        ]
    );
    assert_eq!(export(&import(md).document), md);
    // A tight item whose text goes on after a nested block.
    assert_eq!(
        blocks("- a\n  ```\n  c\n  ```\n  more\n- b\n"),
        ["bullet: a", "code+1: c", "p+1: more", "bullet: b"]
    );
    // An item that opens with something that is not a paragraph.
    assert_eq!(
        blocks("- ```\n  c\n  ```\n- # h\n-\n  text\n"),
        ["bullet", "code+1: c", "bullet", "h1+1: h", "bullet: text"]
    );
    assert_eq!(
        blocks("- a\n\n  b\n\n  - c\n\n    d\n\n  e\n"),
        ["bullet: a", "p+1: b", "bullet+1: c", "p+2: d", "p+1: e"]
    );
}

#[test]
fn quotes() {
    assert_eq!(
        blocks("> One\n> still one\n>\n> Two with **bold**\n\n> Three\n"),
        [
            "quote: One still one",
            "quote: Two with [B|bold]",
            "p",
            "quote: Three"
        ]
    );
    // Two quotes stay two when exported again; one stays one.
    for md in ["> a\n\n> b\n", "> a\n>\n> b\n"] {
        assert_eq!(export(&import(md).document), md);
    }
    assert_eq!(
        blocks(">lazy\ncontinuation\n"),
        ["quote: lazy continuation"]
    );
    let (b, w) = warned("> # Heading\n> - item\n>\n> ```\n> code\n> ```\n> text\n> > nested\n");
    assert_eq!(
        b,
        [
            "h1: Heading",
            "bullet: item",
            "code: code",
            "quote: text",
            "quote: nested"
        ]
    );
    assert_eq!(
        w,
        [
            ImportWarningKind::QuotedBlock,
            ImportWarningKind::QuotedBlock,
            ImportWarningKind::QuotedBlock,
            ImportWarningKind::NestedQuote
        ]
    );
    // A paragraph of a list item inside a quote belongs to the item.
    let (b, _) = warned("> - item\n>\n>   more\n");
    assert_eq!(b, ["bullet: item", "p+1: more"]);
}

#[test]
fn code_blocks() {
    let md = "```rust\nfn main() {\n    println!(\"hi\");\n}\n```\n";
    assert_eq!(
        blocks(md),
        ["code(rust): fn main() {\\n    println!(\"hi\");\\n}"]
    );
    assert_eq!(export(&import(md).document), md);
    assert_eq!(blocks("```\n```\n"), ["code"]);
    assert_eq!(
        blocks("~~~ python title=x\n\ta = 1\n\n~~~\n"),
        ["code(python): \\ta = 1\\n"]
    );
    assert_eq!(
        blocks("    indented\n      more\n\ntext"),
        ["code: indented\\n  more", "p: text"]
    );
    assert_eq!(
        blocks("````md\n```\n# not a heading\n*not italic* <b>\n```\n````\n"),
        ["code(md): ```\\n# not a heading\\n*not italic* <b>\\n```"]
    );
    // Unclosed: the rest of the text.
    assert_eq!(blocks("```\na\n\nb"), ["code: a\\n\\nb"]);
}

#[test]
fn dividers_and_page_breaks() {
    assert_eq!(
        blocks("a\n\n---\n\n***\n\n_ _ _\n\nb\n\n<!-- pagebreak -->\n\nc\n"),
        [
            "p: a",
            "divider",
            "divider",
            "divider",
            "p: b",
            "pagebreak",
            "p: c"
        ]
    );
    assert_eq!(
        blocks(
            "<!--PageBreak-->\n\n<!-- page-break -->\n\n<div style=\"page-break-after: always\"></div>\n\n<div class=\"page-break\"></div>\n"
        ),
        ["pagebreak", "pagebreak", "pagebreak", "pagebreak"]
    );
    let md = "a\n\n---\n\n<!-- pagebreak -->\n\nb\n";
    assert_eq!(export(&import(md).document), md);
}

#[test]
fn the_exporters_html_is_read_back() {
    assert_eq!(
        blocks(
            "<u>under</u> <mark>marked</mark> <strong>b</strong><em>i</em><del>s</del> <code>a\\*b &lt;c&gt;</code>"
        ),
        ["p: [U|under] [H|marked] [B|b][I|i][S|s] [C|a*b <c>]"]
    );
    assert_eq!(
        blocks(
            "<b>b</b> <i>i</i> <s>s</s> <ins>u</ins> <kbd>Cmd</kbd> <a href=\"https://example.com/?a=1&amp;b=2\">link</a>"
        ),
        ["p: [B|b] [I|i] [S|s] [U|u] [C|Cmd] [L=https://example.com/?a=1&b=2|link]"]
    );
    assert_eq!(
        blocks(
            "<span style=\"color:#102030\">ink</span> <span style=\"font-family:'Gill Sans'\">font</span> <span style=\"color:#0a141e;font-family:'Menlo'\">both</span>"
        ),
        ["p: [K=#102030|ink] [F=Gill Sans|font] [K=#0a141e F=Menlo|both]"]
    );
    assert_eq!(
        blocks(
            "<span style=\"font-weight: bold; text-decoration: underline line-through\">x</span> <span style=\"background-color: green; color: red\">y</span> <mark class=\"blue\">z</mark>"
        ),
        ["p: [B S U|x] [H=green K=#ff0000|y] [H=blue|z]"]
    );
    // Tags nest with Markdown and with each other, in any order.
    assert_eq!(
        blocks("**<u>a *b*</u>** <u><mark>c</u>d</mark> e"),
        ["p: [B U|a ][B I U|b] [U H|c][H|d] e"]
    );
    // An unclosed tag ends with its block.
    assert_eq!(blocks("<u>a\n\nb"), ["p: [U|a]", "p: b"]);
    assert_eq!(
        blocks(
            "<div align=\"center\">\n\n# Title\n\nBy me\n\n</div>\n\n<div align=\"right\">\n\nSigned\n\n</div>\n\nBody\n"
        ),
        [
            "h1 (center): Title",
            "p (center): By me",
            "p (right): Signed",
            "p: Body"
        ]
    );
    let options = MarkdownOptions {
        colors: true,
        fonts: true,
        alignment: true,
        ..Default::default()
    };
    let md = "<div align=\"center\">\n\n<u>a</u> <mark>b</mark> <span style=\"color:#102030\">c</span>\n\n</div>\n";
    let doc = import(md).document;
    assert_eq!(to_markdown(&ExportSource::from(&doc), &options), md);
}

#[test]
fn highlight_with_equals_signs() {
    let md = "==marked== and a == b == c and ==two words==, x==y, ==== and `==code==`";
    assert_eq!(
        blocks(md),
        [format!("p: {}", md.replace("`==code==`", "[C|==code==]"))]
    );
    let options = MarkdownImportOptions {
        highlight_equals: true,
        ..Default::default()
    };
    assert_eq!(
        describe(&import_with(md, &options)),
        ["p: [H|marked] and a == b == c and [H|two words], x==y, ==== and [C|==code==]"]
    );
    assert_eq!(
        describe(&import_with(
            "==**bold** and plain== ==open\n\n== x ==",
            &options
        )),
        ["p: [B H|bold][H| and plain] ==open", "p: == x =="]
    );
    let doc = import_with("a ==b== c", &options).document;
    let equals = MarkdownOptions {
        highlight: MarkdownHighlight::Equals,
        ..Default::default()
    };
    assert_eq!(
        to_markdown(&ExportSource::from(&doc), &equals),
        "a ==b== c\n"
    );
}

#[test]
fn tables_become_text() {
    let (b, w) = warned(
        "| Name | Qty | Note |\n|:--|--:|:-:|\n| Apple | 3 | **fresh** |\n| Fig `x` | 12 | [link](u:1) \\| ok |\n| short |\n",
    );
    assert_eq!(
        b,
        ["code: | Name    | Qty |      Note       |\\n\
          | :------ | --: | :-------------: |\\n\
          | Apple   |   3 |      fresh      |\\n\
          | Fig `x` |  12 | link (u:1) | ok |\\n\
          | short   |     |                 |"]
    );
    assert_eq!(w, [ImportWarningKind::Table]);
    let (b, w) = warned("text\n\n| a |\n|---|\n\nmore\n");
    assert_eq!(b, ["p: text", "code: | a   |\\n| --- |", "p: more"]);
    assert_eq!(w, [ImportWarningKind::Table]);
    // Not a table without its rule.
    assert_eq!(blocks("| a | b |\n| c | d |"), ["p: | a | b | | c | d |"]);
}

#[test]
fn images_become_links() {
    let (b, w) = warned(
        "![A cat](cat.png) and ![](https://example.com/x.png) and [![badge](b.svg)](https://ci)",
    );
    assert_eq!(
        b,
        [
            "p: [L=cat.png|A cat] and [L=https://example.com/x.png|https://example.com/x.png] and [L=b.svg|badge]"
        ]
    );
    assert_eq!(w, [ImportWarningKind::Image]);
    let (b, w) = warned("text\n\n![*styled* alt](a.png \"title\")\n");
    assert_eq!(b, ["p: text", "p: [I L=a.png|styled][L=a.png| alt]"]);
    assert_eq!(w, [ImportWarningKind::Image]);
}

#[test]
fn footnotes_become_text() {
    let (b, w) = warned(
        "Claim[^1] and another[^note].\n\n[^1]: The source.\n\n    More of it.\n\n[^note]:\n    - a list\n\nAfter.\n",
    );
    assert_eq!(
        b,
        [
            "p: Claim[^1] and another[^note].",
            "p: [^1]: The source.",
            "p: More of it.",
            "p: [^note]:",
            "bullet: a list",
            "p: After.",
        ]
    );
    assert!(w.iter().all(|k| *k == ImportWarningKind::Footnote));
    assert_eq!(w.len(), 3);
    // Stable once exported.
    let once = export(&import("Claim[^1].\n\n[^1]: The source.\n").document);
    assert_eq!(export(&import(&once).document), once);
    assert_eq!(import(&once).warnings, []);
}

#[test]
fn raw_html() {
    let (b, w) = warned("<details>\n<summary>More</summary>\n\nHidden *text*.\n\n</details>\n");
    assert_eq!(
        b,
        [
            "code(html): <details>\\n<summary>More</summary>",
            "p: Hidden [I|text].",
            "code(html): </details>"
        ]
    );
    assert_eq!(
        w,
        [ImportWarningKind::HtmlBlock, ImportWarningKind::HtmlBlock]
    );
    let (b, w) =
        warned("H<sub>2</sub>O and x<sup>2</sup> <span class=\"x\">y</span> <unknown-tag> </u>");
    assert_eq!(
        b,
        ["p: H<sub>2</sub>O and x<sup>2</sup> <span class=\"x\">y</span> <unknown-tag> </u>"]
    );
    assert_eq!(w, [ImportWarningKind::InlineHtml]);
    let (b, w) =
        warned("<!-- a note to self -->\n\ntext <!-- inline --> more\n\n<!--\nmany\nlines\n-->\n");
    assert_eq!(b, ["p: text  more"]);
    assert_eq!(
        w,
        [
            ImportWarningKind::HtmlComment,
            ImportWarningKind::HtmlComment,
            ImportWarningKind::HtmlComment
        ]
    );
    // Stable once exported.
    for md in [
        "H<sub>2</sub>O",
        "<details>\nx\n</details>\n",
        "<p align=\"center\">x</p>",
    ] {
        let once = export(&import(md).document);
        let twice = import(&once);
        assert_eq!(twice.warnings, [], "{once}");
        assert_eq!(export(&twice.document), once);
    }
}

#[test]
fn front_matter() {
    let md = "---\ntitle: \"The Salt Road\"\ntags: [a, b]\n---\n\n# Chapter one\n\nText.\n";
    let imported = import(md);
    assert_eq!(describe(&imported), ["h1: Chapter one", "p: Text."]);
    assert_eq!(
        imported.front_matter.as_deref(),
        Some("title: \"The Salt Road\"\ntags: [a, b]")
    );
    assert_eq!(imported.title.as_deref(), Some("The Salt Road"));
    assert_eq!(imported.warnings.len(), 1);
    assert_eq!(imported.warnings[0].kind, ImportWarningKind::FrontMatter);
    assert_eq!(imported.warnings[0].line, 1);

    let keep = MarkdownImportOptions {
        front_matter: MarkdownFrontMatter::CodeBlock,
        leading_heading: MarkdownLeadingHeading::Title,
        ..Default::default()
    };
    let imported = import_with(md, &keep);
    assert_eq!(
        describe(&imported),
        [
            "code(yaml): title: \"The Salt Road\"\\ntags: [a, b]",
            "p: Text."
        ]
    );
    assert_eq!(imported.title.as_deref(), Some("Chapter one"));
    assert_eq!(imported.warnings, []);
    assert!(imported.front_matter.is_some());

    let raw = MarkdownImportOptions {
        front_matter: MarkdownFrontMatter::Markdown,
        ..Default::default()
    };
    let imported = import_with(md, &raw);
    assert_eq!(
        describe(&imported),
        [
            "divider",
            "h2: title: \"The Salt Road\" tags: [a, b]",
            "h1: Chapter one",
            "p: Text."
        ]
    );
    assert_eq!(imported.front_matter, None);

    // TOML, and titles in their spellings.
    let imported = import("+++\ntitle = 'Plain'\n+++\nBody\n");
    assert_eq!(describe(&imported), ["p: Body"]);
    assert_eq!(imported.title.as_deref(), Some("Plain"));
    for (line, title) in [
        ("title: Plain words # a comment", Some("Plain words")),
        ("Title:   spaced   out  ", Some("spaced out")),
        ("title: 'it''s'", Some("it")),
        ("title: |", None),
        ("title:", None),
        ("subtitle: x", None),
    ] {
        let imported = import(&format!("---\n{line}\n---\n"));
        assert_eq!(imported.title.as_deref(), title, "{line}");
        assert!(imported.is_empty());
    }

    // Not front matter: a rule and a heading, a rule alone, no closing
    // line, not at the start.
    for md in [
        "---\nJust a line\n---\n",
        "---\n\n---\n",
        "---\n---\n",
        "---\ntitle: x\n",
        "\n---\ntitle: x\n---\n",
        "text\n\n---\ntitle: x\n---\n",
    ] {
        let imported = import(md);
        assert_eq!(imported.front_matter, None, "{md:?}");
        assert_eq!(imported.warnings, [], "{md:?}");
    }
}

#[test]
fn leading_heading_as_title() {
    let take = MarkdownImportOptions {
        leading_heading: MarkdownLeadingHeading::Title,
        ..Default::default()
    };
    let imported = import_with("# A *styled*  title\n\nBody\n", &take);
    assert_eq!(imported.title.as_deref(), Some("A styled title"));
    assert_eq!(describe(&imported), ["p: Body"]);
    // Only a level-1 heading that comes first.
    for md in ["## Two\n\nBody", "Body\n\n# Late", "- # in a list"] {
        let imported = import_with(md, &take);
        assert_eq!(imported.title, None, "{md}");
        assert_eq!(describe(&imported), describe(&import(md)));
    }
    assert_eq!(import("Body\n\n# Late").title, None);
    // A heading alone leaves an empty document.
    let imported = import_with("# Only", &take);
    assert_eq!(imported.title.as_deref(), Some("Only"));
    assert!(imported.is_empty());
    assert_eq!(imported.document, Document::new());
    // The heading wins over the front matter when it is the one taken out.
    let md = "---\ntitle: Front\n---\n# Heading\n";
    assert_eq!(import_with(md, &take).title.as_deref(), Some("Heading"));
    assert_eq!(import(md).title.as_deref(), Some("Front"));
}

#[test]
fn settings_are_the_ones_given() {
    let mut settings = writer_core::DocumentSettings {
        page_view: true,
        ..Default::default()
    };
    settings.typography.line_height = 2.0;
    settings.spelling_language = writer_core::SpellingLanguage::language("de");
    let options = MarkdownImportOptions {
        settings: settings.clone(),
        ..Default::default()
    };
    assert_eq!(import_with("text", &options).document.settings(), &settings);
    assert_eq!(import_with("", &options).document.settings(), &settings);
}

#[test]
fn line_endings_tabs_and_scripts() {
    let unix = "# T\n\n- a\n- b\n\n```\ncode\n\tx\n```\n\nline one\\\nline two\n";
    let expected = blocks(unix);
    assert_eq!(
        expected,
        [
            "h1: T",
            "bullet: a",
            "bullet: b",
            "code: code\\n\\tx",
            "p: line one\\nline two"
        ]
    );
    assert_eq!(blocks(&unix.replace('\n', "\r\n")), expected);
    assert_eq!(blocks(&unix.replace('\n', "\r")), expected);
    assert_eq!(blocks(&format!("\u{feff}{unix}")), expected);
    assert_eq!(
        blocks("a\tb\n\n-\ttabbed item\n"),
        ["p: a\\tb", "bullet: tabbed item"]
    );
    assert_eq!(
        blocks("😀 **👨‍👩‍👧** café naïve 日本語 `🎉`\n\n# שלום עולם\n\n- مرحبا **بالعالم**\n"),
        [
            "p: 😀 [B|👨‍👩‍👧] café naïve 日本語 [C|🎉]",
            "h1: שלום עולם",
            "bullet: مرحبا [B|بالعالم]"
        ]
    );
    // Control characters cannot be in a document.
    assert_eq!(blocks("a\u{0}b\u{7}c\u{1b}[0m"), ["p: abc[0m"]);
}

#[test]
fn long_lines_and_large_texts() {
    let sentence = "The quick brown fox jumps over the lazy dog. ";
    let long = sentence.repeat(2_000);
    let got = blocks(&long);
    assert_eq!(got, [format!("p: {}", long.trim_end())]);
    // A book: a few thousand blocks of every sort, well under a second.
    let mut md = String::new();
    for i in 0..3_000 {
        md.push_str(&format!(
            "## Chapter {i}\n\nSome *text* with a [link](https://example.com/{i}).\n\n- one\n  - two\n\n1. first\n2. second\n\n> quote\n\n```rust\nfn f{i}() {{}}\n```\n\n"
        ));
    }
    let start = std::time::Instant::now();
    let imported = import(&md);
    let took = start.elapsed();
    println!(
        "imported {} bytes, {} blocks in {took:?}",
        md.len(),
        3_000 * 8
    );
    assert_eq!(imported.document.block_count(), 3_000 * 8);
    assert_eq!(imported.warnings, []);
    assert!(took.as_secs() < 20, "importing took {took:?}");
    assert_eq!(export(&imported.document), md.trim_end().to_string() + "\n");
}

#[test]
fn hostile_shapes_do_not_hang_or_panic() {
    let cases = [
        ">".repeat(20_000),
        "> ".repeat(5_000) + "x",
        "- ".repeat(5_000) + "x",
        "*".repeat(20_000),
        "[".repeat(20_000),
        "<u>".repeat(5_000) + "x",
        "</u>".repeat(5_000),
        "<!--".repeat(5_000),
        "|".repeat(5_000) + "\n" + &"|-".repeat(2_500) + "\n",
        "==".repeat(5_000),
        "a==".repeat(5_000),
        "---\n".repeat(5_000),
        "---\na:\n".to_string() + &"x\n".repeat(20_000),
        "`".repeat(9_999),
        "\\\n".repeat(5_000),
        "<br>".repeat(5_000),
        "![".repeat(3_000) + &"](x)".repeat(3_000),
        "1. ".repeat(200) + "x",
        "\u{0}\u{1}\u{2}".repeat(1_000),
    ];
    let equals = MarkdownImportOptions {
        highlight_equals: true,
        soft_breaks: MarkdownSoftBreaks::LineBreak,
        front_matter: MarkdownFrontMatter::CodeBlock,
        leading_heading: MarkdownLeadingHeading::Title,
        ..Default::default()
    };
    for case in &cases {
        let start = std::time::Instant::now();
        let a = import(case);
        let b = import_with(case, &equals);
        let took = start.elapsed();
        assert!(
            took.as_secs() < 20,
            "{:?}... took {took:?}",
            &case[..20.min(case.len())]
        );
        // Whatever came out can be exported and read again.
        for imported in [a, b] {
            let once = export(&imported.document);
            import(&once);
        }
    }
}

#[test]
fn deep_nesting_is_clamped() {
    let mut md = String::new();
    for depth in 0..12 {
        md.push_str(&format!("{}- level {depth}\n", "  ".repeat(depth)));
    }
    let (b, w) = warned(&md);
    assert_eq!(b.len(), 12);
    assert_eq!(b[8], "bullet+8: level 8");
    assert_eq!(b[11], "bullet+8: level 11");
    assert_eq!(w, [ImportWarningKind::NestingTooDeep; 3]);
    // Stable from there.
    let once = export(&import(&md).document);
    assert_eq!(export(&import(&once).document), once);
}

#[test]
fn warnings_carry_lines_and_summaries() {
    let md = "---\ntitle: x\n---\n\n![a](a.png)\n\n| t |\n|---|\n\n<div>\nraw\n</div>\n\nx<sup>2</sup>\n\n![b](b.png)\n";
    let imported = import(md);
    let got: Vec<(ImportWarningKind, usize)> =
        imported.warnings.iter().map(|w| (w.kind, w.line)).collect();
    assert_eq!(
        got,
        [
            (ImportWarningKind::FrontMatter, 1),
            (ImportWarningKind::Image, 5),
            (ImportWarningKind::Table, 7),
            (ImportWarningKind::HtmlBlock, 10),
            (ImportWarningKind::InlineHtml, 14),
            (ImportWarningKind::Image, 16),
        ]
    );
    assert_eq!(
        imported.warnings[2].to_string(),
        "line 7: a table was kept as text in a code block (documents have no tables)"
    );
    let summary = summarize_warnings(&imported.warnings);
    assert_eq!(summary.len(), 5);
    assert_eq!(
        summary[1],
        "2 images were kept as links to them (documents have no images): lines 5 and 16"
    );
    // The same lines with Windows line ends.
    let crlf = import(&md.replace('\n', "\r\n"));
    assert_eq!(crlf.warnings, imported.warnings);
}

#[test]
fn fragments() {
    let imported = import("Just a line with **bold**.");
    let fragment = imported.fragment();
    assert!(fragment.is_inline());
    assert_eq!(fragment.to_plain_text(), "Just a line with bold.");
    let imported = import("## Section\n\n- a\n- b\n");
    let fragment = imported.fragment();
    assert_eq!(fragment.blocks().len(), 3);
    assert!(!fragment.is_inline());
    // Inserted into a document that exists, as whole blocks.
    let mut editor = EditorState::new(Document::from_blocks(vec![
        Block::with_text(BlockKind::heading(1), "Title"),
        Block::paragraph("End."),
    ]));
    editor.insert_blocks_at(1, &fragment);
    let texts: Vec<&str> = editor.document().blocks().iter().map(Block::text).collect();
    assert_eq!(texts, ["Title", "Section", "a", "b", "End."]);
    assert_eq!(editor.document().blocks()[2].kind(), &BlockKind::BulletItem);
    editor.document().check_invariants().unwrap();
}

#[test]
fn things_that_become_text_keep_their_place() {
    // Inside a list item they are blocks of the item.
    let (b, w) = warned(
        "- item\n\n  | a | b |\n  |---|---|\n  | 1 | 2 |\n\n  <div>\n  raw\n  </div>\n\n  ## Heading\n\n  ---\n- next\n",
    );
    assert_eq!(
        b,
        [
            "bullet: item",
            "code+1: | a   | b   |\\n| --- | --- |\\n| 1   | 2   |",
            "code(html)+1: <div>\\nraw\\n</div>",
            "h2+1: Heading",
            "divider+1",
            "bullet: next",
        ]
    );
    assert_eq!(w, [ImportWarningKind::Table, ImportWarningKind::HtmlBlock]);
    let once = export(
        &import("- item\n\n  | a | b |\n  |---|---|\n\n  ## Heading\n\n  ---\n- next\n").document,
    );
    assert_eq!(
        once,
        "- item\n\n  ```\n  | a   | b   |\n  | --- | --- |\n  ```\n\n  ## Heading\n\n  ---\n\n- next\n"
    );
    assert_eq!(export(&import(&once).document), once);
    // A footnote that opens with a code block, and one with nothing.
    let (b, _) = warned("[^a]:\n    ```\n    code\n    ```\n\n[^b]:\n\nText[^a][^b]\n");
    assert_eq!(b, ["p: [^a]:", "code: code", "p: [^b]:", "p: Text[^a][^b]"]);
}

#[test]
fn white_space_that_is_meant_stays() {
    // A paragraph of a no-break space is someone's spacer.
    assert_eq!(blocks("a\n\n&nbsp;\n\nb"), ["p: a", "p: \u{a0}", "p: b"]);
    assert_eq!(
        blocks("a&nbsp;&nbsp;b\u{3000}c"),
        ["p: a\u{a0}\u{a0}b\u{3000}c"]
    );
    // Tabs and runs of spaces inside a line are kept; the edges are not.
    assert_eq!(blocks("a  b\tc \n  d"), ["p: a  b\\tc d"]);
    assert_eq!(blocks("a <br> b<br>"), ["p: a\\nb"]);
    assert_eq!(blocks("`  two  `"), ["p: [C| two ]"]);
    // The line separators of Unicode are line breaks, in code as a space.
    assert_eq!(blocks("a\u{2028}b `c\u{2028}d`"), ["p: a\\nb [C|c d]"]);
}

#[test]
fn a_byte_order_mark_never_comes_back() {
    for md in [
        "\u{feff}text",
        "&#xFEFF;text",
        " \u{a0}\u{feff} text",
        "\u{feff}\u{feff}text",
    ] {
        assert_eq!(blocks(md), ["p: text"], "{md:?}");
    }
    // Elsewhere the character is text like any other.
    assert_eq!(
        blocks("a\u{feff}b\n\n- \u{feff}c"),
        ["p: a\u{feff}b", "bullet: \u{feff}c"]
    );
    assert_eq!(blocks("**\u{feff}b**"), ["p: [B|\u{feff}b]"]);
}

// ----- round trips ------------------------------------------------------------------------

/// Text that Markdown keeps as it is: no white space at the edges, no
/// doubled or unusual spaces.
fn clean_words(rng: &mut Rng, pieces: usize) -> String {
    const PIECES: &[&str] = &[
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
        "5",
        "42",
        "*",
        "_",
        "`",
        "[x]",
        "<b>",
        "&amp;",
        "#",
        "1.",
        "-",
        "~~",
        "a_b",
        "2*3",
        "\\",
        "|",
        "$",
        "==",
        "!",
        "(y)",
        "https://x.y/z",
        "a@b.c",
        "[^1]",
        ">",
    ];
    let mut s = String::new();
    for i in 0..pieces.max(1) {
        if i > 0 {
            s.push(' ');
        }
        s.push_str(rng.pick(PIECES));
    }
    s
}

fn clean_attrs(rng: &mut Rng) -> InlineAttrs {
    let mut a = plain();
    if rng.chance(45) {
        return a;
    }
    a.bold = rng.chance(30);
    a.italic = rng.chance(30);
    a.strikethrough = rng.chance(15);
    a.underline = rng.chance(15);
    a.code = rng.chance(15);
    if rng.chance(12) {
        a.highlight = Some(Highlight::Yellow);
    }
    if rng.chance(10) {
        a.color = Some(Rgb::new(10, 20, 30));
    }
    if rng.chance(10) {
        a.font = Some(
            rng.pick(&["Menlo", "Gill Sans", "Iowan Old Style"])
                .to_string(),
        );
    }
    if rng.chance(20) {
        a.link = Some(
            rng.pick(&[
                "https://example.com",
                "u:1",
                "/path/a_b*c",
                "#frag",
                "https://x.y/(z)?a=1&b=2",
            ])
            .to_string(),
        );
    }
    a
}

/// A document that uses only what Markdown can say, in the form Markdown
/// keeps: no empty text blocks, clean white space, list levels that go one
/// step at a time, blocks inside an item one level below it.
fn clean_document(rng: &mut Rng) -> Document {
    let mut blocks: Vec<Block> = Vec::new();
    // The indent of the open list items, outermost first.
    let mut open: Vec<u8> = Vec::new();
    let n = 1 + rng.below(12);
    for _ in 0..n {
        let kind = match rng.below(20) {
            0..=4 => BlockKind::Paragraph,
            5 => BlockKind::heading(1 + rng.below(6) as u8),
            6..=8 => BlockKind::BulletItem,
            9..=11 => BlockKind::NumberedItem,
            12 | 13 => BlockKind::TodoItem {
                checked: rng.chance(50),
            },
            14 | 15 => BlockKind::Quote,
            16 | 17 => BlockKind::CodeBlock {
                language: rng
                    .chance(60)
                    .then(|| rng.pick(&["rust", "c++", "sh"]).to_string()),
            },
            18 => BlockKind::Divider,
            _ => BlockKind::PageBreak,
        };
        // Where the block goes: out of some of the open items, then
        // perhaps one level into the innermost one.
        while !open.is_empty() && rng.chance(35) {
            open.pop();
        }
        let indent = open.len() as u8;
        let mut block = Block::new(kind.clone()).with_indent(indent);
        if kind.is_code() {
            let lines: Vec<&str> = (0..rng.below(4))
                .map(|_| {
                    rng.pick(&[
                        "fn main() {",
                        "    indented",
                        "}",
                        "",
                        "# x",
                        "```",
                        "a  b",
                        "\tt",
                    ])
                })
                .collect();
            block = block.with_span(&lines.join("\n"), plain());
        } else if !kind.is_void() {
            let spans = 1 + rng.below(4);
            for i in 0..spans {
                if i > 0 {
                    let gap = if rng.chance(15) && kind.heading_level().is_none() {
                        "\n"
                    } else {
                        " "
                    };
                    block = block.with_span(gap, plain());
                }
                let pieces = 1 + rng.below(3);
                let text = clean_words(rng, pieces);
                block = block.with_span(&text, clean_attrs(rng));
            }
        }
        if kind.is_list_item() && indent < 8 {
            open.push(indent);
        }
        blocks.push(block);
    }
    // Alignment, where the exporter writes it: paragraphs and headings
    // outside lists.
    let mut in_list = false;
    for block in &mut blocks {
        if block.indent() == 0 {
            in_list = block.kind().is_list_item();
        }
        let top = block.indent() == 0 && !in_list;
        if top
            && matches!(
                block.kind(),
                BlockKind::Paragraph | BlockKind::Heading { .. }
            )
            && rng.chance(15)
        {
            let alignment = [Alignment::Center, Alignment::Right, Alignment::Justify][rng.below(3)];
            *block = block.clone().with_alignment(alignment);
        }
    }
    Document::from_blocks(blocks)
}

/// The characters of a block with their formatting, the formatting of
/// white space left out (it cannot be seen and Markdown does not keep it).
fn cells(block: &Block) -> Vec<(char, InlineAttrs)> {
    let mut out = Vec::new();
    for (_, text, attrs) in block.text_spans() {
        for c in text.chars() {
            let attrs = if c.is_whitespace() && !attrs.code {
                InlineAttrs::default()
            } else {
                attrs.clone()
            };
            out.push((c, attrs));
        }
    }
    out
}

fn full_options() -> MarkdownOptions {
    MarkdownOptions {
        colors: true,
        fonts: true,
        alignment: true,
        ..Default::default()
    }
}

/// Exporting a document that uses only what Markdown can express and
/// importing it gives the same blocks: kind, indent, alignment, text and
/// the formatting of every character.
#[test]
fn exported_documents_come_back() {
    let options = full_options();
    for seed in 1..=seeds(3000) {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let doc = clean_document(&mut rng);
        let markdown = to_markdown(&ExportSource::from(&doc), &options);
        let imported = import(&markdown);
        let back = &imported.document;
        let fail = |what: &str| -> ! {
            panic!(
                "seed {seed}: {what}\n--- markdown ---\n{markdown}\n--- document ---\n{}\n--- came back ---\n{}",
                doc.blocks()
                    .iter()
                    .map(describe_block)
                    .collect::<Vec<_>>()
                    .join("\n"),
                back.blocks()
                    .iter()
                    .map(describe_block)
                    .collect::<Vec<_>>()
                    .join("\n"),
            )
        };
        if !imported.warnings.is_empty() {
            fail(&format!("warnings {:?}", imported.warnings));
        }
        if back.block_count() != doc.block_count() {
            fail("another number of blocks");
        }
        for (a, b) in doc.blocks().iter().zip(back.blocks()) {
            if a.kind() != b.kind()
                || a.indent() != b.indent()
                || a.alignment() != b.alignment()
                || a.text() != b.text()
                || cells(a) != cells(b)
            {
                fail(&format!(
                    "{} came back as {}",
                    describe_block(a),
                    describe_block(b)
                ));
            }
        }
        // And the Markdown of what came back is the same Markdown.
        let again = to_markdown(&ExportSource::from(back), &options);
        if again != markdown {
            fail(&format!("exported again:\n{again}"));
        }
    }
}

/// The same for documents as hostile as the exporter's own tests make
/// them, in the form the exporter normalises them to: its tests compare
/// what a parser reads with [`expected`], and here the imported document
/// must have the same.
#[test]
fn hostile_documents_come_back_in_normal_form() {
    for seed in 1..=seeds(3000) {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let n = 1 + rng.below(8);
        let hostile = seed % 2 == 0;
        let blocks: Vec<Block> = (0..n)
            .map(|_| {
                if hostile {
                    adv_block(&mut rng)
                } else {
                    random_block(&mut rng)
                }
            })
            .collect();
        let doc = Document::from_blocks(blocks);
        let options = if hostile {
            adv_options(&mut rng)
        } else {
            MarkdownOptions::default()
        };
        let markdown = to_markdown(&ExportSource::from(&doc), &options);
        // A text that opens with U+FEFF opens with a byte order mark as
        // far as any reader of a file can tell, and the importer leaves
        // the character out (also when only a colour or a font is on it,
        // which most exports do not write).
        let mut opening = markdown.as_str();
        while opening.starts_with("<span ") {
            opening = &opening[opening.find('>').map_or(0, |i| i + 1)..];
        }
        if opening.starts_with('\u{feff}') {
            continue;
        }
        let imported = import(&markdown);
        let want = expected(&doc, &options);
        let got = expected(&imported.document, &options);
        if want != got || !imported.warnings.is_empty() {
            let mut msg = format!(
                "seed {seed}: warnings {:?}\n--- markdown ---\n{markdown}\n--- want / got ---\n",
                imported.warnings
            );
            for i in 0..want.len().max(got.len()) {
                let (w, g) = (want.get(i), got.get(i));
                let mark = if w == g { " " } else { "!" };
                msg.push_str(&format!("{mark} want {w:?}\n{mark} got  {g:?}\n"));
            }
            panic!("{msg}");
        }
        // What came back exports to Markdown that stays as it is. (It is
        // not always the first Markdown: an empty item and the paragraph
        // below it are written as an item with text.)
        let again = to_markdown(&ExportSource::from(&imported.document), &options);
        let third = to_markdown(&ExportSource::from(&import(&again).document), &options);
        assert_eq!(
            third, again,
            "seed {seed}: the Markdown changed on the third export"
        );
    }
}

/// Lines of Markdown as nobody would write them on purpose.
fn markdown_soup(rng: &mut Rng) -> String {
    const STARTS: &[&str] = &[
        "",
        "",
        "",
        "# ",
        "## ",
        "###### ",
        "- ",
        "* ",
        "+ ",
        "1. ",
        "2. ",
        "10) ",
        "- [ ] ",
        "- [x] ",
        "> ",
        "> > ",
        ">",
        "    ",
        "  ",
        "   ",
        "\t",
        "```",
        "```rust",
        "~~~",
        "---",
        "***",
        "===",
        "| a | b |",
        "|---|---|",
        "<div>",
        "</div>",
        "<div align=\"center\">",
        "<!-- x -->",
        "<!-- pagebreak -->",
        "[^1]: ",
        "[ref]: /url",
        "<u>",
        "</u>",
        "<br>",
        "---\ntitle: x\n---",
        "- > ",
        "> - ",
        "1. - ",
        "  - ",
        "    - ",
        "      1. ",
        "\\",
    ];
    let mut out = String::new();
    let lines = 1 + rng.below(10);
    for _ in 0..lines {
        out.push_str(rng.pick(STARTS));
        for _ in 0..rng.below(5) {
            match rng.below(12) {
                0..=3 => out.push_str(rng.pick(WORDS)),
                4 | 5 => out.push_str(rng.pick(INLINE_HOSTILE)),
                6 => out.push_str(rng.pick(HOSTILE)),
                7 => out.push_str(rng.pick(&[
                    "**b**",
                    "*i*",
                    "~~s~~",
                    "`c`",
                    "[l](u:1)",
                    "![i](p.png)",
                    "<u>u</u>",
                    "<mark>m</mark>",
                    "==h==",
                    "[^1]",
                    "<span style=\"color:#102030\">k</span>",
                    "<sup>2</sup>",
                    "<a href=\"x\">a</a>",
                    "<https://x.y>",
                    "&nbsp;",
                    "  \n",
                    "<kbd>k",
                    "</mark>",
                    "<!-- c -->",
                    "[r][ref]",
                    "<strong>",
                    "</em>",
                ])),
                8 => out.push_str(rng.pick(LINE_STARTS)),
                _ => out.push_str(rng.pick(SPACES)),
            }
        }
        out.push_str(if rng.chance(40) { "\n\n" } else { "\n" });
    }
    out
}

/// Importing Markdown and exporting it again is stable from the second
/// pass on: what the first pass wrote, every further pass writes again.
#[test]
fn markdown_is_stable_from_the_second_pass() {
    let all = [
        MarkdownImportOptions::default(),
        MarkdownImportOptions {
            soft_breaks: MarkdownSoftBreaks::LineBreak,
            front_matter: MarkdownFrontMatter::CodeBlock,
            ..Default::default()
        },
        MarkdownImportOptions {
            highlight_equals: true,
            leading_heading: MarkdownLeadingHeading::Title,
            front_matter: MarkdownFrontMatter::Markdown,
            ..Default::default()
        },
    ];
    for seed in 1..=seeds(3000) {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let source = markdown_soup(&mut rng);
        let options = &all[(seed % 3) as usize];
        let first = import_with(&source, options);
        let once = export(&first.document);
        let second = import(&once);
        let twice = export(&second.document);
        if once != twice {
            panic!(
                "seed {seed}: not stable\n--- source ---\n{source}\n--- first pass ---\n{once}\n--- second pass ---\n{twice}\n--- first document ---\n{}\n--- second document ---\n{}",
                describe(&first).join("\n"),
                describe(&second).join("\n"),
            );
        }
        // The second document is the first as far as Markdown can tell,
        // and it stays: a third pass reads the same blocks.
        let third = import(&twice);
        assert_eq!(describe(&second), describe(&third), "seed {seed}");
    }
}

/// Any string is some Markdown: text made of arbitrary characters imports
/// to a valid document, whatever the options, and what it exports to is
/// stable.
#[test]
fn arbitrary_text_imports() {
    const CHARS: &[char] = &[
        'a',
        'b',
        ' ',
        ' ',
        '\n',
        '\n',
        '\t',
        '\r',
        '*',
        '_',
        '`',
        '~',
        '[',
        ']',
        '(',
        ')',
        '<',
        '>',
        '!',
        '#',
        '-',
        '+',
        '=',
        '|',
        ':',
        '\\',
        '&',
        ';',
        '/',
        '"',
        '\'',
        '.',
        '1',
        '0',
        '^',
        '$',
        '{',
        '}',
        '@',
        'é',
        '日',
        'ש',
        '😀',
        '\u{a0}',
        '\u{200d}',
        '\u{feff}',
        '\u{0}',
        '\u{1b}',
        '\u{85}',
        '\u{2028}',
        '\u{202e}',
        '\u{fffd}',
        '\u{10ffff}',
    ];
    let all = [
        MarkdownImportOptions::default(),
        MarkdownImportOptions {
            soft_breaks: MarkdownSoftBreaks::LineBreak,
            front_matter: MarkdownFrontMatter::CodeBlock,
            highlight_equals: true,
            leading_heading: MarkdownLeadingHeading::Title,
            ..Default::default()
        },
    ];
    for seed in 1..=seeds(3000) {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let n = rng.below(60);
        let source: String = (0..n).map(|_| CHARS[rng.below(CHARS.len())]).collect();
        let first = import_with(&source, &all[(seed % 2) as usize]);
        let once = export(&first.document);
        let twice = export(&import(&once).document);
        assert_eq!(once, twice, "seed {seed}: not stable for {source:?}");
        // Plain text takes anything too.
        let plain = writer_export::from_plain_text(&source, &Default::default());
        plain.document.check_invariants().unwrap();
    }
}
