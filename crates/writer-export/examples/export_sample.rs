//! Writes a sample document that uses every feature in every format, for
//! checking the output in real applications:
//!
//! ```text
//! CARGO_TARGET_DIR=target-pure cargo run -p writer-export --example export_sample -- /some/dir
//! ```
//!
//! Files written: `sample.md`, `sample-footnotes.md`, `sample.txt`,
//! `sample-social.txt`, `sample.docx`, `lists.docx` (one page of lists that
//! restart in every way, small enough for a Quick Look thumbnail),
//! `book.docx`, `book.md`, and `long.docx`: a book of 1,200 sections whose
//! numbered items say their own number ("item 2"), which passes every
//! limit the DOCX numbering has (a definition per list, shared
//! definitions, numbers as text).

use writer_core::{
    Alignment, Block, BlockKind, Document, DocumentSettings, EditorState, Highlight, InlineAttrs,
    InlineStyle, Margins, Orientation, PageSetup, PaperSize, Position, Rgb, Selection,
};
use writer_export::{
    DocxOptions, ExportFormat, ExportSource, MarkdownComments, MarkdownOptions, PlainTextOptions,
    export, suggested_file_name, to_docx, to_markdown, to_plain_text,
};

fn plain() -> InlineAttrs {
    InlineAttrs::default()
}

fn style(s: InlineStyle) -> InlineAttrs {
    plain().with(s)
}

fn sample() -> Document {
    let link = |url: &str| InlineAttrs {
        link: Some(url.to_string()),
        ..plain()
    };
    let blocks = vec![
        Block::with_text(BlockKind::heading(1), "The Writer: export sample"),
        Block::paragraph("A paragraph with ")
            .with_span("bold", style(InlineStyle::Bold))
            .with_span(", ", plain())
            .with_span("italic", style(InlineStyle::Italic))
            .with_span(", ", plain())
            .with_span("underline", style(InlineStyle::Underline))
            .with_span(", ", plain())
            .with_span("strikethrough", style(InlineStyle::Strikethrough))
            .with_span(", ", plain())
            .with_span("inline code", style(InlineStyle::Code))
            .with_span(", ", plain())
            .with_span("a highlight", InlineAttrs { highlight: Some(Highlight::Yellow), ..plain() })
            .with_span(", ", plain())
            .with_span("red text", InlineAttrs { color: Some(Rgb::new(0xc0, 0x39, 0x2b)), ..plain() })
            .with_span(", ", plain())
            .with_span("another font", InlineAttrs { font: Some("Helvetica Neue".into()), ..plain() })
            .with_span(" and ", plain())
            .with_span("a link", link("https://example.com/path?a=1&b=2"))
            .with_span(".", plain()),
        Block::paragraph("Everything at once: ").with_span(
            "bold italic underlined struck highlighted link",
            InlineAttrs {
                bold: true,
                italic: true,
                underline: true,
                strikethrough: true,
                highlight: Some(Highlight::Green),
                color: Some(Rgb::new(0x1a, 0x5f, 0xb4)),
                link: Some("https://example.com/all".into()),
                ..plain()
            },
        ),
        Block::paragraph("A soft line break follows here,\nand this is the second line of the same paragraph."),
        Block::with_text(BlockKind::heading(2), "Lists"),
        Block::with_text(BlockKind::BulletItem, "First bullet"),
        Block::with_text(BlockKind::BulletItem, "Second bullet"),
        Block::with_text(BlockKind::BulletItem, "Nested bullet").with_indent(1),
        Block::with_text(BlockKind::BulletItem, "Deeper bullet").with_indent(2),
        Block::with_text(BlockKind::BulletItem, "Third bullet"),
        Block::paragraph("A numbered list:"),
        Block::with_text(BlockKind::NumberedItem, "One"),
        Block::with_text(BlockKind::NumberedItem, "Two"),
        Block::with_text(BlockKind::NumberedItem, "Two, first part").with_indent(1),
        Block::with_text(BlockKind::NumberedItem, "Two, second part").with_indent(1),
        Block::with_text(BlockKind::NumberedItem, "Three"),
        Block::paragraph("A second numbered list, which starts again at one:"),
        Block::with_text(BlockKind::NumberedItem, "Uno"),
        Block::with_text(BlockKind::NumberedItem, "Due"),
        Block::paragraph("Lists that restart in different ways. Expected: 1, (1, 2), 2, (1), bullet, (1), indented text, 3:"),
        Block::with_text(BlockKind::NumberedItem, "Alpha (1)"),
        Block::with_text(BlockKind::NumberedItem, "Alpha a (1)").with_indent(1),
        Block::with_text(BlockKind::NumberedItem, "Alpha b (2)").with_indent(1),
        Block::with_text(BlockKind::NumberedItem, "Beta (2)"),
        Block::with_text(BlockKind::NumberedItem, "Beta a (1)").with_indent(1),
        Block::with_text(BlockKind::BulletItem, "A bullet between two nested lists").with_indent(1),
        Block::with_text(BlockKind::NumberedItem, "Beta, new list (1)").with_indent(1),
        Block::paragraph("A paragraph inside the item.").with_indent(1),
        Block::with_text(BlockKind::NumberedItem, "Gamma (3)"),
        Block::paragraph("Numbered lists under bullets. Expected: bullet, (1, 2), bullet, (1):"),
        Block::with_text(BlockKind::BulletItem, "Fruit"),
        Block::with_text(BlockKind::NumberedItem, "Apple (1)").with_indent(1),
        Block::with_text(BlockKind::NumberedItem, "Pear (2)").with_indent(1),
        Block::with_text(BlockKind::BulletItem, "Vegetables"),
        Block::with_text(BlockKind::NumberedItem, "Leek (1)").with_indent(1),
        Block::paragraph("To-dos:"),
        Block::with_text(BlockKind::TodoItem { checked: true }, "Write the exporter"),
        Block::with_text(BlockKind::TodoItem { checked: false }, "Open the file in Word"),
        Block::with_text(BlockKind::TodoItem { checked: false }, "A nested to-do").with_indent(1),
        Block::with_text(BlockKind::heading(2), "Quotes and code"),
        Block::with_text(BlockKind::Quote, "The quote block holds a first paragraph."),
        Block::new(BlockKind::Quote)
            .with_span("And a second one, with ", plain())
            .with_span("emphasis", style(InlineStyle::Italic))
            .with_span(".", plain()),
        Block::with_text(
            BlockKind::CodeBlock { language: Some("rust".into()) },
            "fn main() {\n    let greeting = \"hello\";   // spaces   kept\n\tprintln!(\"{greeting}\");\n}",
        ),
        Block::new(BlockKind::Divider),
        Block::with_text(BlockKind::heading(2), "Alignment and sizes"),
        Block::paragraph("Centred paragraph.").with_alignment(Alignment::Center),
        Block::paragraph("Right-aligned paragraph.").with_alignment(Alignment::Right),
        Block::paragraph(
            "A justified paragraph needs enough words to wrap over more than one line, so here are some more words \
             that say nothing in particular but fill the line and show both edges straight.",
        )
        .with_alignment(Alignment::Justify),
        Block::paragraph("A paragraph with a larger text size.").with_text_size(Some(24.0)),
        Block::paragraph("An indented paragraph.").with_indent(1),
        Block::with_text(BlockKind::heading(3), "Heading 3"),
        Block::with_text(BlockKind::heading(4), "Heading 4"),
        Block::with_text(BlockKind::heading(5), "Heading 5"),
        Block::with_text(BlockKind::heading(6), "Heading 6"),
        Block::with_text(BlockKind::heading(2), "Hostile text"),
        Block::paragraph("Markdown in text: *stars* _underscores_ `ticks` [brackets](parens) <angle> & ampersand # hash."),
        Block::paragraph("1. A paragraph that looks like a list item"),
        Block::paragraph("Emoji 😀 👨‍👩‍👧‍👦 🇦🇱, CJK 日本語, and “smart quotes” — dashes…"),
        Block::paragraph("שלום עולם, זו פסקה מימין לשמאל."),
        Block::paragraph("مرحبا بالعالم"),
        Block::new(BlockKind::PageBreak),
        Block::with_text(BlockKind::heading(2), "After the page break"),
        Block::paragraph("This paragraph starts the second page. It carries a comment thread with a reply."),
    ];
    let settings = DocumentSettings {
        page: PageSetup {
            paper: PaperSize::A4,
            orientation: Orientation::Portrait,
            margins: Margins {
                top: 72.0,
                right: 90.0,
                bottom: 72.0,
                left: 90.0,
            },
        },
        ..Default::default()
    };
    let mut ed = EditorState::new(Document::from_blocks(blocks).with_settings(settings));
    let last = ed.document().block_count() - 1;
    ed.set_selection(Selection::new(
        Position::new(last, 5),
        Position::new(last, 14),
    ));
    let thread = ed.add_comment(
        "Is \"paragraph\" the right word here?\nMaybe \"section\".",
        "Ada Lovelace",
        1_794_391_505_000,
    );
    if let Some(thread) = thread {
        ed.add_reply(thread, "It is. Leave it.", "Ben Example", 1_794_395_105_000);
    }
    ed.set_selection(Selection::new(Position::new(1, 17), Position::new(1, 21)));
    if let Some(done) = ed.add_comment("Too bold?", "Ada Lovelace", 1_794_391_000_000) {
        ed.resolve_comment(done);
    }
    ed.into_document()
}

/// One page of numbered lists; every item says the number it must show.
fn lists() -> Document {
    let n = |text: &str, indent: u8| {
        Block::with_text(BlockKind::NumberedItem, text).with_indent(indent)
    };
    let b =
        |text: &str, indent: u8| Block::with_text(BlockKind::BulletItem, text).with_indent(indent);
    Document::from_blocks(vec![
        n("one", 0),
        n("two", 0),
        Block::paragraph("A paragraph ends the list."),
        n("one", 0),
        n("one", 1),
        n("two", 1),
        n("two", 0),
        n("one", 1),
        b("bullet", 1),
        n("one", 1),
        Block::paragraph("Text inside the item.").with_indent(1),
        n("three", 0),
        Block::with_text(BlockKind::heading(3), "Under bullets"),
        b("bullet", 0),
        n("one", 1),
        n("two", 1),
        b("bullet", 0),
        n("one", 1),
        n("one", 2),
        n("two", 1),
        n("one", 2),
        n("two", 2),
    ])
}

/// A long book: every section has prose, a numbered list with a nested
/// list, and now and then bullets, a quote and code.
fn long_book(sections: usize) -> Document {
    let n = |number: usize, indent: u8| {
        Block::with_text(BlockKind::NumberedItem, &format!("item {number}")).with_indent(indent)
    };
    let mut blocks = Vec::new();
    for i in 1..=sections {
        if i % 20 == 1 {
            blocks.push(Block::with_text(
                BlockKind::heading(1),
                &format!("Chapter {}", i / 20 + 1),
            ));
        }
        blocks.push(Block::with_text(BlockKind::heading(2), &format!("Section {i}")));
        blocks.push(Block::paragraph(
            "The quick brown fox jumps over the lazy dog, and then the dog, being lazy, does nothing at all \
             about it, which is exactly what the fox was counting on when it planned the jump.",
        ));
        blocks.extend([n(1, 0), n(1, 1), n(2, 1), n(2, 0), n(1, 1), n(3, 0)]);
        if i % 3 == 0 {
            blocks.push(Block::with_text(BlockKind::BulletItem, "A bullet"));
            blocks.extend([n(1, 1), n(2, 1)]);
        }
        if i % 7 == 0 {
            blocks.push(Block::with_text(BlockKind::Quote, "A quote between the lists."));
        }
        if i % 11 == 0 {
            blocks.push(Block::with_text(BlockKind::code(), "let x = 1;\nlet y = 2;"));
        }
    }
    Document::from_blocks(blocks)
}

fn main() -> std::io::Result<()> {
    let dir = std::env::args().nth(1).unwrap_or_else(|| ".".to_string());
    let dir = std::path::Path::new(&dir);
    std::fs::create_dir_all(dir)?;
    let doc = sample();
    let source = ExportSource::single("The Writer: export sample", &doc);
    let docx = DocxOptions {
        author: "Ada Lovelace".into(),
        created: 1_794_391_505_000,
        modified: 1_794_395_105_000,
        ..DocxOptions::default()
    };

    std::fs::write(
        dir.join("sample.md"),
        to_markdown(&source, &MarkdownOptions::default()),
    )?;
    let footnotes = MarkdownOptions {
        comments: MarkdownComments::Footnotes,
        ..MarkdownOptions::default()
    };
    std::fs::write(
        dir.join("sample-footnotes.md"),
        to_markdown(&source, &footnotes),
    )?;
    std::fs::write(
        dir.join("sample.txt"),
        to_plain_text(&source, &PlainTextOptions::default()),
    )?;
    std::fs::write(
        dir.join("sample-social.txt"),
        to_plain_text(&source, &PlainTextOptions::social()),
    )?;
    std::fs::write(dir.join("sample.docx"), to_docx(&source, &docx))?;
    let lists = lists();
    std::fs::write(
        dir.join("lists.docx"),
        to_docx(&ExportSource::single("Lists", &lists), &docx),
    )?;

    let long = long_book(1200);
    std::fs::write(
        dir.join("long.docx"),
        to_docx(&ExportSource::single("A Long Book", &long), &docx),
    )?;

    // A folder exported as one file: three chapters, the second in landscape.
    let chapter = |n: usize, with_heading: bool| {
        let mut blocks = Vec::new();
        if with_heading {
            blocks.push(Block::with_text(
                BlockKind::heading(1),
                &format!("Chapter {n}"),
            ));
        }
        blocks.push(Block::paragraph(&format!(
            "The text of chapter {n} starts here."
        )));
        blocks.push(Block::with_text(
            BlockKind::NumberedItem,
            "A numbered point",
        ));
        blocks.push(Block::with_text(
            BlockKind::NumberedItem,
            "Another numbered point",
        ));
        Document::from_blocks(blocks)
    };
    let one = chapter(1, true);
    let two = chapter(2, false).with_settings(DocumentSettings {
        page: PageSetup {
            orientation: Orientation::Landscape,
            ..Default::default()
        },
        ..Default::default()
    });
    let three = chapter(3, true);
    let book = ExportSource::collection(
        "A Small Book",
        [
            ("Chapter 1", &one),
            ("Chapter 2", &two),
            ("Chapter 3", &three),
        ],
    );
    std::fs::write(dir.join("book.docx"), to_docx(&book, &docx))?;
    std::fs::write(
        dir.join("book.md"),
        export(&book, ExportFormat::Markdown, &Default::default()),
    )?;

    println!("wrote samples to {}", dir.display());
    println!(
        "suggested name: {}",
        suggested_file_name(source.title(), ExportFormat::Docx)
    );
    Ok(())
}
