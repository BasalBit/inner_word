//! Loads a plain text file as a document, for looking at realistic text
//! in the demo (`INNER_WORD_DEMO_FILE=<path>`). One block per line, with a
//! small Markdown-like notation:
//!
//! | Line | Block |
//! |---|---|
//! | `# ` … `###### ` | heading 1 to 6 |
//! | `- ` | bullet item |
//! | `1. ` (any number) | numbered item |
//! | `[ ] ` / `[x] ` | to-do, open / done |
//! | `> ` | quote |
//! | `---` / `===` | divider / page break |
//! | lines between ```` ``` ```` fences | one code block |
//! | anything else | paragraph (empty lines are skipped) |
//!
//! Two leading spaces per indent level. A leading `|c|`, `|r|` or `|j|`
//! centres, right-aligns or justifies the block. Inline: `**bold**`,
//! `*italic*`, `` `code` ``, `==highlight==`, `__underline__`, and `\n`
//! for a soft line break.

use innerword_core::{Alignment, Block, BlockKind, Document, Highlight, InlineAttrs, InlineStyle};

pub fn load(path: &str) -> std::io::Result<Document> {
    Ok(parse(&std::fs::read_to_string(path)?))
}

pub fn parse(source: &str) -> Document {
    let mut blocks = Vec::new();
    let mut code: Option<(String, Vec<&str>)> = None;
    for line in source.lines() {
        if let Some((language, lines)) = &mut code {
            if line.trim_start().starts_with("```") {
                let kind = BlockKind::CodeBlock {
                    language: (!language.is_empty()).then(|| language.clone()),
                };
                blocks.push(Block::with_text(kind, &lines.join("\n")));
                code = None;
            } else {
                lines.push(line);
            }
            continue;
        }
        if let Some(language) = line.trim_start().strip_prefix("```") {
            code = Some((language.trim().to_string(), Vec::new()));
            continue;
        }
        if line.trim().is_empty() {
            continue;
        }
        let indent = line.len() - line.trim_start_matches("  ").len();
        let mut rest = &line[indent..];
        let indent = (indent / 2) as u8;
        let mut alignment = Alignment::Left;
        for (mark, value) in [
            ("|c|", Alignment::Center),
            ("|r|", Alignment::Right),
            ("|j|", Alignment::Justify),
        ] {
            if let Some(stripped) = rest.strip_prefix(mark) {
                alignment = value;
                rest = stripped.trim_start();
            }
        }
        let (kind, text) = classify(rest);
        let block = if kind.is_void() {
            Block::new(kind)
        } else {
            inline(Block::new(kind), text)
        };
        blocks.push(block.with_indent(indent).with_alignment(alignment));
    }
    if blocks.is_empty() {
        return Document::new();
    }
    Document::from_blocks(blocks)
}

fn classify(line: &str) -> (BlockKind, &str) {
    if line == "---" {
        return (BlockKind::Divider, "");
    }
    if line == "===" {
        return (BlockKind::PageBreak, "");
    }
    let hashes = line.bytes().take_while(|byte| *byte == b'#').count();
    if (1..=6).contains(&hashes)
        && let Some(text) = line[hashes..].strip_prefix(' ')
    {
        return (BlockKind::heading(hashes as u8), text);
    }
    if let Some(text) = line.strip_prefix("- ") {
        return (BlockKind::BulletItem, text);
    }
    if let Some(text) = line.strip_prefix("[ ] ") {
        return (BlockKind::todo(), text);
    }
    if let Some(text) = line.strip_prefix("[x] ") {
        return (BlockKind::TodoItem { checked: true }, text);
    }
    if let Some(text) = line.strip_prefix("> ") {
        return (BlockKind::Quote, text);
    }
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    if digits > 0
        && let Some(text) = line[digits..].strip_prefix(". ")
    {
        return (BlockKind::NumberedItem, text);
    }
    (BlockKind::Paragraph, line)
}

/// Appends `text` to `block`, turning the inline marks into attributes.
fn inline(mut block: Block, text: &str) -> Block {
    let text = text.replace("\\n", "\n");
    let mut attrs = InlineAttrs::default();
    let mut pending = String::new();
    let mut rest = text.as_str();
    type Toggle = fn(&mut InlineAttrs);
    let marks: [(&str, Toggle); 5] = [
        ("**", |attrs| attrs.bold = !attrs.bold),
        ("==", |attrs| {
            attrs.highlight = match attrs.highlight {
                Some(_) => None,
                None => Some(Highlight::Yellow),
            }
        }),
        ("__", |attrs| attrs.underline = !attrs.underline),
        ("*", |attrs| attrs.italic = !attrs.italic),
        ("`", |attrs| {
            *attrs = if attrs.code {
                InlineAttrs::default()
            } else {
                InlineAttrs::default().with(InlineStyle::Code)
            }
        }),
    ];
    'outer: while !rest.is_empty() {
        for (mark, toggle) in &marks {
            if let Some(after) = rest.strip_prefix(mark) {
                if !pending.is_empty() {
                    block = block.with_span(&pending, attrs.clone());
                    pending.clear();
                }
                toggle(&mut attrs);
                rest = after;
                continue 'outer;
            }
        }
        let next = rest.chars().next().map_or(0, char::len_utf8);
        pending.push_str(&rest[..next]);
        rest = &rest[next..];
    }
    if !pending.is_empty() {
        block = block.with_span(&pending, attrs);
    }
    block
}
