//! A book-sized document goes through every exporter in reasonable time
//! and the cost grows with the size of the document, not faster.

use std::time::Instant;

use writer_core::{Block, BlockKind, Document, InlineAttrs, InlineStyle};
use writer_export::{
    ExportFormat, ExportOptions, ExportSource, SocialNetwork, export, export_text,
};

fn book(blocks: usize) -> Document {
    let plain = InlineAttrs::default;
    let mut out = Vec::with_capacity(blocks);
    for i in 0..blocks {
        let block = match i % 40 {
            0 => Block::with_text(BlockKind::heading(2), &format!("Chapter {}", i / 40 + 1)),
            7..=9 => Block::with_text(
                BlockKind::NumberedItem,
                "A numbered point with a few words in it",
            ),
            15 | 16 => Block::with_text(BlockKind::BulletItem, "A bullet, nested")
                .with_indent((i % 3) as u8),
            22 => Block::with_text(
                BlockKind::Quote,
                "Somebody said something worth quoting here.",
            ),
            30 => Block::with_text(BlockKind::code(), "fn main() {\n    println!(\"hi\");\n}"),
            _ => Block::paragraph(
                "It was a bright cold day in April, and the clocks were striking thirteen; ",
            )
            .with_span("some of it bold", plain().with(InlineStyle::Bold))
            .with_span(", some of it ", plain())
            .with_span(
                "italic and linked",
                InlineAttrs {
                    link: Some("https://example.com/a".into()),
                    ..plain().with(InlineStyle::Italic)
                },
            )
            .with_span(
                " — and the rest plain, with *stars* and _underscores_ that need escaping.",
                plain(),
            ),
        };
        out.push(block);
    }
    Document::from_blocks(out)
}

#[test]
fn a_book_exports_in_linear_time() {
    let options = ExportOptions::default();
    let mut per_block = Vec::new();
    for size in [2_000usize, 20_000] {
        let doc = book(size);
        let source = ExportSource::single("Book", &doc);
        let mut line = format!("{size} blocks:");
        let mut total = 0.0;
        for format in ExportFormat::ALL {
            let start = Instant::now();
            let bytes = export(&source, format, &options);
            let secs = start.elapsed().as_secs_f64();
            total += secs;
            line.push_str(&format!(
                " {} {:.0} ms ({} KB)",
                format.extension(),
                secs * 1000.0,
                bytes.len() / 1024
            ));
            assert!(!bytes.is_empty());
        }
        println!("{line}");
        per_block.push(total / size as f64);
    }
    // Ten times the blocks must not cost much more than ten times the time
    // (the bound is loose: timings on a busy machine are noisy).
    assert!(
        per_block[1] < per_block[0] * 4.0 + 1e-4,
        "export time grows faster than the document: {per_block:?}"
    );
}

/// A deterministic generator for the stress documents below.
struct Rng(u64);

impl Rng {
    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n as u64) as usize
    }
}

/// A 10,000-block book with every block kind, nested lists, thousands of
/// separate numbered lists, styled runs, links and 500 comment threads.
fn varied_book(blocks: usize) -> Document {
    use writer_core::{EditorState, Position, Selection};
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let words = [
        "the",
        "quick",
        "brown",
        "fox",
        "jumps",
        "over",
        "a",
        "lazy",
        "dog",
        "—",
        "*",
        "_x_",
        "[1]",
        "日本語",
        "שלום",
        "😀",
        "a<b",
        "&amp;",
        "1.",
        "`code`",
        "https://example.com/a_b",
    ];
    let mut out = Vec::with_capacity(blocks);
    for i in 0..blocks {
        let kind = match rng.below(20) {
            0 => BlockKind::heading(1 + rng.below(6) as u8),
            1..=3 => BlockKind::NumberedItem,
            4..=6 => BlockKind::BulletItem,
            7 => BlockKind::TodoItem {
                checked: i % 2 == 0,
            },
            8 => BlockKind::Quote,
            9 => BlockKind::code(),
            10 if i % 7 == 0 => BlockKind::Divider,
            11 if i % 50 == 0 => BlockKind::PageBreak,
            _ => BlockKind::Paragraph,
        };
        let mut block = Block::new(kind.clone());
        if !kind.is_void() {
            for _ in 0..1 + rng.below(6) {
                let mut text = String::new();
                for _ in 0..1 + rng.below(12) {
                    text.push_str(words[rng.below(words.len())]);
                    text.push(' ');
                }
                let mut attrs = InlineAttrs::default();
                match rng.below(8) {
                    0 => attrs.bold = true,
                    1 => attrs.italic = true,
                    2 => attrs.strikethrough = true,
                    3 => attrs.link = Some("https://example.com/x?y=1&z=(2)".into()),
                    4 => attrs.code = true,
                    _ => {}
                }
                block = block.with_span(&text, attrs);
            }
        }
        if kind.is_list_item() {
            block = block.with_indent(rng.below(4) as u8);
        }
        out.push(block);
    }
    let mut ed = EditorState::new(Document::from_blocks(out));
    for n in 0..500usize {
        let block = (n * 19) % blocks;
        let len = ed.document().blocks()[block].len();
        if len < 4 {
            continue;
        }
        let start = ed.document().clamp_position(Position::new(block, 1));
        let end = ed.document().clamp_position(Position::new(block, len - 1));
        ed.set_selection(Selection::new(start, end));
        if let Some(id) = ed.add_comment("Check this.", "Ada", 1_794_391_505_000) {
            ed.add_reply(id, "Done.", "Ben", 1_794_391_506_000);
        }
    }
    ed.into_document()
}

/// One paragraph with thousands of styled runs, many of them in places
/// where Markdown emphasis needs the HTML fallback.
fn dense_paragraph(runs: usize) -> Document {
    let mut block = Block::new(BlockKind::Paragraph);
    for i in 0..runs {
        let attrs = match i % 4 {
            0 => InlineAttrs::default().with(InlineStyle::Bold),
            2 => InlineAttrs::default().with(InlineStyle::Italic),
            _ => InlineAttrs::default(),
        };
        let text = match i % 8 {
            0 => "(bold)",
            1 => "tight",
            2 => "em.",
            3 => "next, ",
            4 => "文字。",
            5 => "次の ",
            6 => " spaced ",
            _ => " plain ",
        };
        block = block.with_span(text, attrs);
    }
    Document::from_blocks(vec![block])
}

/// Prints the time of every exporter for a varied 10,000-block book and
/// for one very dense paragraph, and fails only on gross slowness. With
/// `WRITER_EXPORT_DUMP_DIR` set, the files are written there.
#[test]
fn a_varied_book_and_a_dense_paragraph_export_quickly() {
    let dump = std::env::var("WRITER_EXPORT_DUMP_DIR").ok();
    let options = ExportOptions::default();
    for (name, doc) in [
        ("book-10000", varied_book(10_000)),
        ("dense-paragraph-4000", dense_paragraph(4_000)),
    ] {
        let source = ExportSource::single(name, &doc);
        let mut line = format!("{name}:");
        for format in ExportFormat::ALL {
            let start = Instant::now();
            let bytes = export(&source, format, &options);
            let secs = start.elapsed().as_secs_f64();
            line.push_str(&format!(
                " {} {:.0} ms ({} KB)",
                format.extension(),
                secs * 1000.0,
                bytes.len() / 1024
            ));
            // Generous even for an unoptimised build on a busy machine.
            assert!(secs < 20.0, "{name} took {secs:.1} s as {format:?}");
            if let Some(dir) = &dump {
                std::fs::write(format!("{dir}/{name}.{}", format.extension()), &bytes).unwrap();
            }
        }
        println!("{line}");
        // The status bar counts the text against a network's limit while
        // the writer types: that must stay quick for a whole book too.
        let text = export_text(&source, ExportFormat::PlainText, &options).unwrap();
        let start = Instant::now();
        let counts: Vec<usize> = SocialNetwork::ALL
            .into_iter()
            .map(|n| n.length(&text))
            .collect();
        let secs = start.elapsed().as_secs_f64();
        println!(
            "{name}: social counts of {} KB in {:.0} ms {counts:?}",
            text.len() / 1024,
            secs * 1000.0
        );
        assert!(secs < 20.0, "{name}: counting took {secs:.1} s");
        assert!(counts.iter().all(|c| *c > 0));
    }
}
