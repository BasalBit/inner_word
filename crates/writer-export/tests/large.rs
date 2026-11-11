//! A book-sized document goes through every exporter in reasonable time
//! and the cost grows with the size of the document, not faster.

use std::time::Instant;

use writer_core::{Block, BlockKind, Document, InlineAttrs, InlineStyle};
use writer_export::{ExportFormat, ExportOptions, ExportSource, export};

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
