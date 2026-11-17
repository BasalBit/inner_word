//! The print pass: the pages page view has, as data for paper.
//!
//! What page view shows is asked of the editor through its public seams
//! (`page_count`, `visual_row`, `caret_page`) and through what its last
//! frame painted; what the pass hands out is read from the pages. The two
//! are worked out by different code on different layouts (the editor's own
//! and the pass's), which is the point of comparing them.
//!
//! Glyphs are 0.6 em wide here (GPUI's test text system), so a PDF made of
//! these pages reads back as its text only where the font is that wide:
//! the documents that go through the PDF writer are set in Courier New.

use std::time::Duration;

use gpui::{Bounds, Entity, Pixels, TestAppContext, VisualTestContext, px, size};
use innerword_core::{
    Alignment, Block, BlockKind, Document, DocumentSettings, Highlight, InlineAttrs, InlineStyle,
    Margins, Orientation, PageSetup, PaperSize, Position, Rgb, TextRange, Typography,
};
use innerword_print::{Anchor, Color, Item, JobInfo, Label, Page, PdfFile, PdfWriter, TextRow};
use innerword_ui::theme::{self, ThemeMode};

use super::slice::{open, setup};
use crate::{
    Autoscroll, Editor, EditorConfig, PrintPass, PrintProgress, Zoom,
    layout::{
        style::{MARKER_GAP, QUOTE_GUTTER},
        wrap::INLINE_CODE_SCALE,
    },
    print::{on_paper, painted_pieces},
};

/// A4 portrait: 595.28 x 841.89 points, one-inch margins, a text area of
/// 451.28 x 697.89.
const A4: (f32, f32) = (595.276, 841.89);
const MARGIN: f32 = 72.;

fn paged(blocks: Vec<Block>) -> Document {
    Document::from_blocks(blocks).with_settings(DocumentSettings {
        page_view: true,
        ..DocumentSettings::default()
    })
}

/// The same in Courier New, whose glyphs are as wide as the test text
/// system says every glyph is.
fn paged_in_courier(blocks: Vec<Block>) -> Document {
    Document::from_blocks(blocks).with_settings(DocumentSettings {
        page_view: true,
        typography: Typography {
            body_font: "Courier New".to_string(),
            ..Typography::default()
        },
        ..DocumentSettings::default()
    })
}

const LONG: &str = "A sentence that is long enough to wrap in the text area of a page. ";

/// One of everything, in a document long enough for several pages when it is
/// repeated: headings, paragraphs of several rows, formatted runs, lists of
/// every kind, a quote, a code block, a rule, aligned text.
fn chapter(number: usize) -> Vec<Block> {
    let plain = InlineAttrs::default;
    let style = |style: InlineStyle| InlineAttrs::default().with(style);
    vec![
        Block::with_text(BlockKind::heading(2), &format!("Chapter {number}")),
        Block::paragraph(LONG.repeat(3 + number % 4).trim_end()),
        Block::new(BlockKind::Paragraph)
            .with_span("Runs: ", plain())
            .with_span("bold", style(InlineStyle::Bold))
            .with_span(", ", plain())
            .with_span("italic", style(InlineStyle::Italic))
            .with_span(", ", plain())
            .with_span("code", style(InlineStyle::Code))
            .with_span(", ", plain())
            .with_span(
                "highlighted",
                InlineAttrs {
                    highlight: Some(Highlight::Yellow),
                    ..plain()
                },
            )
            .with_span(" and ", plain())
            .with_span(
                "a link",
                InlineAttrs {
                    link: Some("https://example.com".into()),
                    ..plain()
                },
            )
            .with_span(".", plain()),
        Block::with_text(BlockKind::BulletItem, "A bullet item"),
        Block::with_text(BlockKind::BulletItem, LONG.repeat(2).trim_end()).with_indent(1),
        Block::with_text(BlockKind::NumberedItem, "First"),
        Block::with_text(BlockKind::NumberedItem, "Second"),
        Block::with_text(BlockKind::todo(), "An open to-do"),
        Block::with_text(BlockKind::TodoItem { checked: true }, "A finished to-do"),
        Block::with_text(BlockKind::Quote, LONG.repeat(2).trim_end()),
        Block::with_text(
            BlockKind::CodeBlock {
                language: Some("rust".to_string()),
            },
            &"let line = of_code();\n".repeat(5 + number % 7),
        ),
        Block::new(BlockKind::Divider),
        Block::paragraph("Centred.").with_alignment(Alignment::Center),
        Block::paragraph(LONG.repeat(2).trim_end()).with_alignment(Alignment::Justify),
        Block::paragraph("naïve café, 日本語"),
    ]
}

/// 300 blocks of every kind, with a page break now and then.
fn mixed() -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut number = 0;
    while blocks.len() < 300 {
        blocks.extend(chapter(number));
        if number % 5 == 4 {
            blocks.push(Block::new(BlockKind::PageBreak));
        }
        number += 1;
    }
    blocks.truncate(300);
    blocks
}

/// Lets the editor's background layout run until every block is measured:
/// until then its page count rests on estimates.
fn settle(editor: &Entity<Editor>, cx: &mut VisualTestContext) {
    for _ in 0..2000 {
        if !editor.read_with(cx, |editor, _| editor.layout_pending()) {
            return;
        }
        cx.executor().advance_clock(Duration::from_millis(2));
        cx.run_until_parked();
    }
    panic!("the background layout did not settle");
}

fn pages_of(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> Vec<Page> {
    cx.update(|window, cx| editor.read(cx).print_pass(window, cx).run_to_end())
}

fn rows_of(page: &Page) -> Vec<&TextRow> {
    page.items
        .iter()
        .filter_map(|item| match item {
            Item::Row(row) => Some(row),
            _ => None,
        })
        .collect()
}

fn labels_of(page: &Page) -> Vec<&Label> {
    page.items
        .iter()
        .filter_map(|item| match item {
            Item::Label(label) => Some(label),
            _ => None,
        })
        .collect()
}

fn row_texts(pages: &[Page]) -> Vec<Vec<String>> {
    pages
        .iter()
        .map(|page| rows_of(page).iter().map(|row| row.text.clone()).collect())
        .collect()
}

/// The rows page view has on each page, read through the editor's public
/// seams: every visual row of every block, and the page a caret at its
/// start is on. Rows without text (an empty paragraph, a rule) are left
/// out, as nothing is printed for them.
fn rows_page_view_has(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> Vec<Vec<String>> {
    editor.update(cx, |editor, cx| {
        assert!(editor.page_view() && !editor.layout_pending());
        let before = editor.selection();
        let pages = editor.page_count().expect("page view");
        let mut rows = vec![Vec::new(); pages];
        for block in 0..editor.document().block_count() {
            let length = editor.document().blocks()[block].len();
            let mut at = Position::new(block, 0);
            loop {
                let row = editor.visual_row(at).expect("laid out");
                editor.edit_with(Autoscroll::None, |state| state.set_caret(row.start), cx);
                let page = editor.caret_page().expect("page view") - 1;
                let text = editor.document().blocks()[block].text()
                    [row.start.offset..row.end.offset]
                    .to_string();
                if !text.is_empty() {
                    rows[page].push(text);
                }
                // The next row starts where this one ends, or after the
                // line break that ends it.
                let next = if row.end.offset < length
                    && editor.document().blocks()[block].text().as_bytes()[row.end.offset] == b'\n'
                {
                    row.end.offset + 1
                } else {
                    row.end.offset
                };
                if next >= length || next <= row.start.offset {
                    break;
                }
                at = Position::new(block, next);
            }
        }
        editor.edit_with(Autoscroll::None, |state| state.set_selection(before), cx);
        rows
    })
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.02
}

/// Every item of every page lies on its sheet.
fn assert_on_their_sheets(pages: &[Page]) {
    for (number, page) in pages.iter().enumerate() {
        let on_sheet = |x: f32, y: f32| {
            (-0.01..=page.width + 0.01).contains(&x) && (-0.01..=page.height + 0.01).contains(&y)
        };
        for item in &page.items {
            match item {
                Item::Rect(rect) => {
                    assert!(
                        on_sheet(rect.x, rect.y)
                            && on_sheet(rect.x + rect.width, rect.y + rect.height),
                        "page {number}: {rect:?}"
                    );
                    assert!(
                        rect.width > 0. && rect.height > 0.,
                        "page {number}: {rect:?}"
                    );
                }
                Item::Stroke(stroke) => {
                    assert!(stroke.points.iter().all(|(x, y)| on_sheet(*x, *y)));
                }
                Item::Row(row) => {
                    // The text area, top to bottom; sideways only white
                    // space at the end of a wrapped row hangs past it.
                    assert!(
                        row.baseline > MARGIN && row.baseline < page.height - MARGIN,
                        "page {number}: the baseline of {:?} is at {}",
                        row.text,
                        row.baseline
                    );
                    assert!(row.x >= MARGIN - 0.01, "page {number}: {:?}", row.text);
                    for glyph in &row.glyphs {
                        assert!(on_sheet(row.x + glyph.x, row.baseline));
                        assert!(row.text.is_char_boundary(glyph.byte as usize));
                    }
                    assert_eq!(row.runs.last().map(|run| run.end), Some(row.text.len()));
                    assert!(row.runs.windows(2).all(|pair| pair[0].end < pair[1].end));
                }
                Item::Label(label) => assert!(on_sheet(label.x, label.baseline)),
                Item::Image(image) => {
                    assert!(
                        on_sheet(image.x, image.y)
                            && on_sheet(image.x + image.width, image.y + image.height),
                        "page {number}: a picture at {} {} of {} x {}",
                        image.x,
                        image.y,
                        image.width,
                        image.height
                    );
                    assert!(image.width > 0. && image.height > 0.);
                }
            }
        }
    }
}

#[gpui::test]
fn the_pass_hands_out_the_pages_page_view_has(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(paged(mixed()), EditorConfig::full(), cx);
    // Until the background layout has measured every block the editor's
    // count rests on estimates.
    assert!(editor.read_with(cx, |editor, _| editor.layout_pending()));
    settle(&editor, cx);
    let page_count = editor.read_with(cx, |editor, _| editor.page_count().unwrap());
    assert!(page_count > 12, "{page_count} pages");

    let pages = pages_of(&editor, cx);
    assert_eq!(pages.len(), page_count);
    assert_on_their_sheets(&pages);
    for (number, page) in pages.iter().enumerate() {
        assert!(close(page.width, A4.0) && close(page.height, A4.1));
        // Its number, last.
        let Some(Item::Label(label)) = page.items.last() else {
            panic!("page {number} does not end with its number");
        };
        assert_eq!(label.text, (number + 1).to_string());
    }
    // The rows on each page are the rows page view has on that page.
    assert_eq!(row_texts(&pages), rows_page_view_has(&editor, cx));
    // The pass did not move anything in the editor.
    assert_eq!(
        editor.read_with(cx, |editor, _| editor.page_count()),
        Some(page_count)
    );

    // In slices of one block, the same pages; and the progress says how far.
    let sliced = cx.update(|window, cx| {
        let mut pass = editor.read(cx).print_pass(window, cx);
        assert_eq!(pass.paper(), (595.276, 841.89));
        assert_eq!(
            pass.progress(),
            PrintProgress {
                blocks_done: 0,
                blocks: 300,
                pages_done: 0
            }
        );
        let mut out = Vec::new();
        let mut calls = 0;
        loop {
            let before = pass.progress();
            let done = pass.advance(Duration::ZERO, &mut out);
            let after = pass.progress();
            calls += 1;
            assert_eq!(after.pages_done, out.len());
            assert!(after.pages_done >= before.pages_done);
            if done {
                break;
            }
            // Always at least one block, and with no time to spend exactly
            // one.
            assert_eq!(after.blocks_done, before.blocks_done + 1);
        }
        assert!(calls >= 300, "{calls} calls");
        assert_eq!(pass.progress().blocks_done, 300);
        // Done stays done and adds nothing.
        assert!(pass.advance(Duration::from_secs(1), &mut out));
        assert_eq!(pass.progress().pages_done, out.len());
        out
    });
    assert_eq!(sliced, pages);

    // Whatever the window shows, the pages are the same: flow view, another
    // zoom, Night, a selection and a spelling mark.
    editor.update(cx, |editor, cx| editor.set_page_view(false, cx));
    cx.run_until_parked();
    assert_eq!(editor.read_with(cx, |editor, _| editor.page_count()), None);
    assert_eq!(pages_of(&editor, cx), pages, "flow view");

    editor.update(cx, |editor, cx| {
        editor.set_page_view(true, cx);
        editor.set_zoom(Zoom::Percent(200), cx);
    });
    cx.run_until_parked();
    assert_eq!(
        editor.read_with(cx, |editor, _| editor.zoom_percent()),
        200.
    );
    assert_eq!(pages_of(&editor, cx), pages, "200 %");

    cx.update(|_, cx| theme::set_theme_mode(ThemeMode::Dark, cx));
    cx.run_until_parked();
    assert_eq!(pages_of(&editor, cx), pages, "Night");

    editor.update(cx, |editor, cx| {
        editor.edit(
            |state| {
                state.select_range(TextRange::new(Position::new(1, 3), Position::new(4, 2)));
            },
            cx,
        );
        let block = &editor.document().blocks()[1];
        let (id, revision) = (block.id(), block.revision());
        editor.set_spelling_marks(id, revision, [2..6, 8..12], cx);
    });
    cx.run_until_parked();
    assert_eq!(
        pages_of(&editor, cx),
        pages,
        "a selection and a spelling mark"
    );
    cx.update(|_, cx| theme::set_theme_mode(ThemeMode::Light, cx));
}

#[gpui::test]
fn the_pass_holds_the_document_as_it_was(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(
        paged(vec![Block::paragraph("As it was.")]),
        EditorConfig::full(),
        cx,
    );
    let mut pass = cx.update(|window, cx| editor.read(cx).print_pass(window, cx));
    // An edit made after the pass began is not in its pages.
    cx.simulate_keystrokes("cmd-down");
    cx.simulate_input(" And more, typed while the pages are made.");
    cx.run_until_parked();
    let pages = pass.run_to_end();
    assert_eq!(row_texts(&pages), vec![vec!["As it was.".to_string()]]);
    // The next pass has it.
    let pages = pages_of(&editor, cx);
    assert_eq!(
        row_texts(&pages)[0].concat(),
        "As it was. And more, typed while the pages are made."
    );
}

/// A pass over a document that no editor shows (`PrintPass::new`, what a
/// PDF export of the library makes) hands out the very pages the editor's
/// own pass hands out for the same document, whatever the editor shows;
/// numbered on from another document, only the numbers differ.
#[gpui::test]
fn a_pass_over_any_document_is_the_editors_pass(cx: &mut TestAppContext) {
    setup(cx);
    let document = paged(mixed());
    let (editor, cx) = open(document.clone(), EditorConfig::full(), cx);
    // The editor shows something else than paper: flow view, a zoom.
    editor.update(cx, |editor, cx| {
        editor.set_page_view(false, cx);
        editor.set_zoom(Zoom::Percent(150), cx);
    });
    cx.run_until_parked();
    let editors = pages_of(&editor, cx);
    assert!(editors.len() > 5, "{} pages", editors.len());
    let alone = cx.update(|window, cx| PrintPass::new(document.clone(), window, cx).run_to_end());
    assert_eq!(alone, editors);

    // The same, numbered from 7: every page as it was, but its number.
    let numbered = cx.update(|window, cx| {
        let mut pass = PrintPass::new(document.clone(), window, cx);
        pass.number_pages_from(7);
        pass.run_to_end()
    });
    assert_eq!(numbered.len(), editors.len());
    for (index, (numbered, plain)) in numbered.iter().zip(&editors).enumerate() {
        let number = |page: &Page| {
            labels_of(page)
                .last()
                .map(|label| label.text.clone())
                .expect("a page number")
        };
        assert_eq!(number(plain), (index + 1).to_string());
        assert_eq!(number(numbered), (index + 7).to_string());
        let without_number = |page: &Page| {
            let mut items = page.items.clone();
            items.pop();
            items
        };
        assert_eq!(without_number(numbered), without_number(plain));
    }
}

#[gpui::test]
fn empty_sheets_are_handed_out_with_their_numbers(cx: &mut TestAppContext) {
    setup(cx);
    let only_its_number = |page: &Page, number: &str| {
        assert_eq!(page.items.len(), 1, "{:?}", page.items);
        let Item::Label(label) = &page.items[0] else {
            panic!("not a label: {:?}", page.items[0]);
        };
        assert_eq!(label.text, number);
        assert_eq!(label.anchor, Anchor::Middle);
        assert!(close(label.x, A4.0 / 2.));
    };

    // A document that ends in a page break: the editor counts the empty last
    // sheet, and the pass hands it out.
    let (editor, cx) = open(
        paged(vec![
            Block::paragraph("Before the break."),
            Block::new(BlockKind::PageBreak),
        ]),
        EditorConfig::full(),
        cx,
    );
    settle(&editor, cx);
    assert_eq!(
        editor.read_with(cx, |editor, _| editor.page_count()),
        Some(2)
    );
    let pages = pages_of(&editor, cx);
    assert_eq!(pages.len(), 2);
    assert_eq!(
        row_texts(&pages),
        vec![vec!["Before the break.".to_string()], vec![]]
    );
    only_its_number(&pages[1], "2");

    // Two page breaks in a row: the sheet that holds only the second one.
    editor.update(cx, |editor, cx| {
        editor.set_document(
            paged(vec![
                Block::paragraph("One."),
                Block::new(BlockKind::PageBreak),
                Block::new(BlockKind::PageBreak),
                Block::paragraph("Three."),
            ]),
            cx,
        );
    });
    cx.run_until_parked();
    settle(&editor, cx);
    assert_eq!(
        editor.read_with(cx, |editor, _| editor.page_count()),
        Some(3)
    );
    let pages = pages_of(&editor, cx);
    assert_eq!(
        row_texts(&pages),
        vec![vec!["One.".to_string()], vec![], vec!["Three.".to_string()]]
    );
    only_its_number(&pages[1], "2");
    // The page break marker is for the screen: neither its words nor its
    // hairlines are on a page.
    for page in &pages {
        assert!(
            labels_of(page)
                .iter()
                .all(|label| label.text != "Page break")
        );
        assert!(!page.items.iter().any(|item| matches!(item, Item::Rect(_))));
    }

    // Three in a row at the end of a document.
    editor.update(cx, |editor, cx| {
        editor.set_document(
            paged(vec![
                Block::new(BlockKind::PageBreak),
                Block::new(BlockKind::PageBreak),
                Block::new(BlockKind::PageBreak),
            ]),
            cx,
        );
    });
    cx.run_until_parked();
    settle(&editor, cx);
    assert_eq!(
        editor.read_with(cx, |editor, _| editor.page_count()),
        Some(4)
    );
    let pages = pages_of(&editor, cx);
    assert_eq!(pages.len(), 4);
    for (number, page) in pages.iter().enumerate() {
        only_its_number(page, &(number + 1).to_string());
    }

    // An empty document is one page with its number, and nothing of the
    // placeholder the screen shows on it.
    editor.update(cx, |editor, cx| {
        editor.set_document(paged(vec![Block::paragraph("")]), cx)
    });
    cx.run_until_parked();
    assert_eq!(
        editor.read_with(cx, |editor, _| editor.page_count()),
        Some(1)
    );
    let pages = pages_of(&editor, cx);
    assert_eq!(pages.len(), 1);
    only_its_number(&pages[0], "1");
}

#[gpui::test]
fn blocks_go_on_over_pages_as_in_page_view(cx: &mut TestAppContext) {
    setup(cx);
    // A paragraph of three pages, a page break, a code block of two pages.
    let (editor, cx) = open(
        paged(vec![
            Block::paragraph("Short."),
            Block::paragraph(LONG.repeat(42).trim_end()),
            Block::new(BlockKind::PageBreak),
            Block::with_text(
                BlockKind::code(),
                "a line of a listing\n".repeat(40).trim_end(),
            ),
            Block::paragraph("After the listing."),
        ]),
        EditorConfig::full(),
        cx,
    );
    settle(&editor, cx);
    let pages = pages_of(&editor, cx);
    assert_eq!(
        Some(pages.len()),
        editor.read_with(cx, |editor, _| editor.page_count())
    );
    assert_eq!(pages.len(), 5);
    assert_on_their_sheets(&pages);
    let texts = row_texts(&pages);
    assert_eq!(texts, rows_page_view_has(&editor, cx));

    // The long paragraph has rows on three pages, and on no other.
    let of_paragraph: Vec<usize> = texts
        .iter()
        .map(|rows| rows.iter().filter(|row| row.contains("sentence")).count())
        .collect();
    assert!(
        of_paragraph[0] > 2 && of_paragraph[1] > 2 && of_paragraph[2] >= 2,
        "{of_paragraph:?}"
    );
    assert_eq!(of_paragraph[3..], [0, 0]);
    assert_eq!(texts[0][0], "Short.");
    // The page break starts a page: the listing begins the page after the
    // paragraph's last, at its top.
    assert_eq!(texts[3][0], "a line of a listing");
    assert!(
        texts[..3]
            .iter()
            .flatten()
            .all(|row| !row.contains("listing"))
    );

    // The code block goes on over two pages: a panel on each, behind its
    // rows, from the left edge of the text area to the right one.
    let panels: Vec<Vec<&innerword_print::RectItem>> = pages
        .iter()
        .map(|page| {
            page.items
                .iter()
                .filter_map(|item| match item {
                    Item::Rect(rect) if rect.height > 40. => Some(rect),
                    _ => None,
                })
                .collect()
        })
        .collect();
    assert_eq!(
        panels.iter().map(Vec::len).collect::<Vec<_>>(),
        vec![0, 0, 0, 1, 1]
    );
    for page in [3, 4] {
        let panel = panels[page][0];
        assert!(
            close(panel.x, MARGIN) && close(panel.width, A4.0 - 2. * MARGIN),
            "{panel:?}"
        );
        assert_eq!(panel.radius, f32::from(theme::RADIUS_MD));
        assert!(panel.fill.is_some() && panel.border.is_none());
        let position = |wanted: &Item| {
            pages[page]
                .items
                .iter()
                .position(|item| item == wanted)
                .unwrap()
        };
        // Behind its rows, and around every one of them on its page.
        let rows: Vec<&TextRow> = rows_of(&pages[page])
            .into_iter()
            .filter(|row| row.text == "a line of a listing")
            .collect();
        assert!(rows.len() > 5);
        for row in rows {
            assert!(
                position(&Item::Rect(panel.clone())) < position(&Item::Row(row.clone())),
                "the panel lies behind the rows"
            );
            assert!(row.baseline > panel.y && row.baseline < panel.y + panel.height);
        }
    }
    // The first part of the panel ends inside the text area, the second
    // begins at its top.
    assert!(panels[3][0].y + panels[3][0].height <= A4.1 - MARGIN + 0.02);
    assert!(close(panels[4][0].y, MARGIN), "{:?}", panels[4][0]);
}

#[gpui::test]
fn the_paper_is_the_documents(cx: &mut TestAppContext) {
    setup(cx);
    let on = |page: PageSetup| {
        Document::from_blocks(vec![
            Block::paragraph(LONG.repeat(40).trim_end()),
            Block::paragraph("The end."),
        ])
        .with_settings(DocumentSettings {
            page,
            page_view: true,
            ..DocumentSettings::default()
        })
    };
    let (editor, cx) = open(on(PageSetup::default()), EditorConfig::full(), cx);
    for (setup, paper) in [
        (PageSetup::default(), (595.276, 841.89)),
        (
            PageSetup {
                orientation: Orientation::Landscape,
                ..PageSetup::default()
            },
            (841.89, 595.276),
        ),
        (
            PageSetup {
                paper: PaperSize::UsLetter,
                margins: Margins {
                    top: 36.,
                    right: 36.,
                    bottom: 36.,
                    left: 36.,
                },
                ..PageSetup::default()
            },
            (612., 792.),
        ),
        (
            PageSetup {
                paper: PaperSize::Custom {
                    width: 360.,
                    height: 500.,
                },
                margins: Margins {
                    top: 40.,
                    right: 30.,
                    bottom: 60.,
                    left: 50.,
                },
                ..PageSetup::default()
            },
            (360., 500.),
        ),
    ] {
        let margins = setup.margins;
        editor.update(cx, |editor, cx| editor.set_document(on(setup), cx));
        cx.run_until_parked();
        settle(&editor, cx);
        let (paper_of_pass, pages) = cx.update(|window, cx| {
            let mut pass = editor.read(cx).print_pass(window, cx);
            (pass.paper(), pass.run_to_end())
        });
        assert_eq!(paper_of_pass, paper);
        assert_eq!(
            Some(pages.len()),
            editor.read_with(cx, |editor, _| editor.page_count())
        );
        assert!(pages.len() > 1);
        assert_eq!(row_texts(&pages), rows_page_view_has(&editor, cx));
        for page in &pages {
            assert_eq!((page.width, page.height), paper);
            // The rows begin at the left margin and stay in the text area.
            for row in rows_of(page) {
                assert!(close(row.x, margins.left), "{} on {paper:?}", row.x);
                assert!(row.baseline > margins.top && row.baseline < paper.1 - margins.bottom);
            }
            // The number is in the bottom margin, in its lower part.
            let number = labels_of(page).pop().expect("the page number");
            assert!(close(number.x, paper.0 / 2.));
            assert!(
                number.baseline > paper.1 - margins.bottom * 0.6 && number.baseline < paper.1,
                "{} on {paper:?}",
                number.baseline
            );
        }
    }
}

#[gpui::test]
fn a_bottom_margin_without_room_has_no_page_number(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(
        Document::from_blocks(vec![Block::paragraph("No room for a number.")]).with_settings(
            DocumentSettings {
                page: PageSetup {
                    margins: Margins {
                        top: 72.,
                        right: 72.,
                        bottom: 8.,
                        left: 72.,
                    },
                    ..PageSetup::default()
                },
                page_view: true,
                ..DocumentSettings::default()
            },
        ),
        EditorConfig::full(),
        cx,
    );
    let pages = pages_of(&editor, cx);
    assert_eq!(pages.len(), 1);
    assert!(labels_of(&pages[0]).is_empty());
    assert_eq!(rows_of(&pages[0]).len(), 1);
    // And page view draws none either.
    editor.read_with(cx, |editor, _| assert!(editor.painted_markers.is_empty()));
}

#[test]
fn a_translucent_colour_is_its_colour_on_white_paper() {
    // Paper has no transparency: what shows through a colour is the paper.
    let half_black = gpui::hsla(0., 0., 0., 0.5);
    let grey = on_paper(half_black);
    assert!(
        close(grey.r, 0.5) && close(grey.g, 0.5) && close(grey.b, 0.5),
        "{grey:?}"
    );
    let clear = on_paper(gpui::transparent_black());
    assert_eq!((clear.r, clear.g, clear.b), (1., 1., 1.));
    let red: gpui::Hsla = gpui::rgb(0xff0000).into();
    let solid = on_paper(red);
    assert!(close(solid.r, 1.) && close(solid.g, 0.) && close(solid.b, 0.));
    let faint_red = on_paper(red.alpha(0.25));
    assert!(close(faint_red.r, 1.) && close(faint_red.g, 0.75) && close(faint_red.b, 0.75));
}

/// The colours of Paper, as the pass has them.
fn paper_color(color: gpui::Hsla) -> Color {
    let rgba: gpui::Rgba = color.into();
    assert_eq!(rgba.a, 1., "an opaque colour of the theme");
    Color {
        r: rgba.r,
        g: rgba.g,
        b: rgba.b,
    }
}

#[gpui::test]
fn every_block_kind_puts_on_paper_what_it_shows(cx: &mut TestAppContext) {
    setup(cx);
    let colors =
        innerword_ui::theme::Theme::paper(innerword_ui::theme::ResolvedFonts::unresolved()).colors;
    let plain = InlineAttrs::default;
    let blocks = vec![
        Block::with_text(BlockKind::BulletItem, "A bullet"),
        Block::with_text(BlockKind::BulletItem, "A nested bullet").with_indent(1),
        Block::with_text(BlockKind::BulletItem, "Nested twice").with_indent(2),
        Block::paragraph("Between the lists."),
        Block::with_text(BlockKind::NumberedItem, "First"),
        Block::with_text(BlockKind::NumberedItem, "Second"),
        Block::with_text(BlockKind::todo(), "Open"),
        Block::with_text(BlockKind::TodoItem { checked: true }, "Done"),
        Block::with_text(BlockKind::Quote, "A quote."),
        Block::with_text(
            BlockKind::CodeBlock {
                language: Some("rust".to_string()),
            },
            "code();",
        ),
        Block::new(BlockKind::Divider),
        Block::new(BlockKind::Paragraph)
            .with_span("Plain ", plain())
            .with_span(
                "marked",
                InlineAttrs {
                    highlight: Some(Highlight::Yellow),
                    underline: true,
                    color: Some(Rgb::new(0x2f, 0x6f, 0xb0)),
                    link: Some("https://example.com".into()),
                    ..plain()
                },
            )
            .with_span(" plain", plain()),
        Block::new(BlockKind::PageBreak),
        Block::paragraph("After the break."),
    ];
    let (editor, cx) = open(paged(blocks), EditorConfig::full(), cx);
    settle(&editor, cx);
    let pages = pages_of(&editor, cx);
    assert_eq!(pages.len(), 2);
    assert_on_their_sheets(&pages);
    let page = &pages[0];
    let row = |text: &str| -> &TextRow {
        rows_of(page)
            .into_iter()
            .find(|row| row.text == text)
            .unwrap_or_else(|| panic!("no row {text:?}"))
    };
    let index_of = |wanted: &Item| page.items.iter().position(|item| item == wanted).unwrap();
    // The shapes that stand beside a row: left of its text, on its line.
    let beside = |row: &TextRow| -> Vec<&Item> {
        page.items
            .iter()
            .filter(|item| match item {
                Item::Rect(rect) => {
                    rect.x + rect.width <= row.x
                        && rect.y < row.baseline
                        && rect.y + rect.height > row.baseline - row.size
                        && rect.height < 40.
                }
                Item::Stroke(stroke) => stroke.points.iter().all(|(x, y)| {
                    *x < row.x && *y < row.baseline + 4. && *y > row.baseline - row.size
                }),
                // (On the row's line. With real fonts on its baseline: the
                // test text system gives a shaped line a descent of the
                // other sign than a font's, and GPUI's line painter, which
                // the label's baseline is taken from, centres with it.)
                Item::Label(label) => {
                    label.x <= row.x && (label.baseline - row.baseline).abs() < row.size * 0.5
                }
                _ => false,
            })
            .collect()
    };
    let muted = paper_color(colors.muted);

    // A bullet: a disc at the first level, a ring at the second, a square at
    // the third.
    let [Item::Rect(disc)] = beside(row("A bullet"))[..] else {
        panic!("{:?}", beside(row("A bullet")));
    };
    assert_eq!(disc.fill, Some(muted));
    assert_eq!(disc.border, None);
    assert!(close(disc.width, disc.height) && close(disc.radius, disc.width / 2.));
    let [Item::Rect(ring)] = beside(row("A nested bullet"))[..] else {
        panic!("{:?}", beside(row("A nested bullet")));
    };
    assert_eq!(ring.fill, None);
    assert_eq!(ring.border, Some((1., muted)));
    assert!(close(ring.radius, ring.width / 2.));
    assert!(ring.x > disc.x + 20., "an indent further in");
    let [Item::Rect(square)] = beside(row("Nested twice"))[..] else {
        panic!("{:?}", beside(row("Nested twice")));
    };
    assert_eq!((square.fill, square.radius), (Some(muted), 0.));

    // A numbered item: its number as a label that ends before the text, on
    // the row's baseline, in the body font at the row's size.
    for (text, number) in [("First", "1."), ("Second", "2.")] {
        let item = row(text);
        let [Item::Label(label)] = beside(item)[..] else {
            panic!("{:?}", beside(item));
        };
        assert_eq!(label.text, number);
        assert_eq!(label.anchor, Anchor::End);
        assert!(
            close(label.x, item.x - MARKER_GAP),
            "{} and {}",
            label.x,
            item.x
        );
        assert_eq!(label.size, item.size);
        assert_eq!(label.color, muted);
        assert_eq!(label.font, item.runs[0].font);
        // After the rows of its block.
        assert!(index_of(&Item::Label(label.clone())) > index_of(&Item::Row(item.clone())));
    }

    // A to-do: an open box is its border (its inside is the paper), a ticked
    // one is filled and has its tick over it.
    let [Item::Rect(open_box)] = beside(row("Open"))[..] else {
        panic!("{:?}", beside(row("Open")));
    };
    assert_eq!(open_box.fill, None);
    assert_eq!(
        open_box.border,
        Some((1.5, paper_color(colors.border_strong)))
    );
    let [Item::Rect(ticked), Item::Stroke(tick)] = beside(row("Done"))[..] else {
        panic!("{:?}", beside(row("Done")));
    };
    assert_eq!(ticked.fill, Some(paper_color(colors.accent)));
    assert_eq!(ticked.border, None);
    assert_eq!(tick.points.len(), 3);
    assert_eq!(tick.color, paper_color(colors.on_accent));
    assert!(tick.points.iter().all(|(x, y)| {
        *x > ticked.x
            && *x < ticked.x + ticked.width
            && *y > ticked.y
            && *y < ticked.y + ticked.height
    }));
    assert!(index_of(&Item::Stroke(tick.clone())) > index_of(&Item::Rect(ticked.clone())));
    // A finished to-do is struck through, in its faint colour.
    let done = row("Done");
    assert_eq!(done.runs[0].color, paper_color(colors.faint));
    let Item::Rect(struck) = &page.items[index_of(&Item::Row(done.clone())) + 1] else {
        panic!("no line through the finished to-do");
    };
    assert!(close(struck.x, done.x) && close(struck.width, done.width));
    assert!(struck.y < done.baseline && struck.y > done.baseline - done.size);

    // A quote: a bar left of the text, as high as its rows, and muted text.
    let quote = row("A quote.");
    let [Item::Rect(bar)] = beside(quote)[..] else {
        panic!("{:?}", beside(quote));
    };
    assert!(close(bar.x, quote.x - QUOTE_GUTTER + 1.) && close(bar.width, 3.));
    assert_eq!(bar.fill, Some(paper_color(colors.border_strong)));
    assert_eq!(quote.runs[0].color, muted);

    // A code block: its panel behind it, its language at the panel's right
    // edge, its text in the monospace font.
    let code = row("code();");
    let Item::Rect(panel) = &page.items[index_of(&Item::Row(code.clone())) - 1] else {
        panic!("no panel behind the code");
    };
    assert_eq!(panel.fill, Some(paper_color(colors.hover)));
    assert!(close(panel.x, MARGIN) && close(panel.width, A4.0 - 2. * MARGIN));
    let language = labels_of(page)
        .into_iter()
        .find(|label| label.text == "rust")
        .expect("the language of the code block");
    assert_eq!(language.anchor, Anchor::End);
    assert!(close(language.x, panel.x + panel.width - 8.));
    assert!(language.baseline > panel.y && language.baseline < panel.y + 20.);
    assert_eq!(language.font.family, ".SystemUIFont");
    assert_eq!(language.size, f32::from(theme::UI_TEXT_XS));
    assert_ne!(
        code.runs[0].font.family,
        row("A quote.").runs[0].font.family
    );

    // A rule across the text area.
    let rules: Vec<&innerword_print::RectItem> = page
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Rect(rect) if close(rect.width, A4.0 - 2. * MARGIN) && rect.height <= 1. => {
                Some(rect)
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        rules.len(),
        1,
        "the rule, and no hairline of the page break marker"
    );
    assert_eq!(rules[0].fill, Some(paper_color(colors.border_strong)));
    assert_eq!(rules[0].height, 1.);

    // A run that is highlighted, underlined, coloured and a link: its
    // background before the row, the colours in the row's runs, its
    // underline after the row.
    let marked = row("Plain marked plain");
    let at = index_of(&Item::Row(marked.clone()));
    let (Item::Rect(background), Item::Rect(underline)) =
        (&page.items[at - 1], &page.items[at + 1])
    else {
        panic!("no background before and no underline after the row");
    };
    let glyph_x = |byte: u32| {
        marked
            .glyphs
            .iter()
            .find(|glyph| glyph.byte == byte)
            .unwrap()
            .x
    };
    assert!(close(background.x, marked.x + glyph_x(6)));
    assert!(close(
        background.x + background.width,
        marked.x + glyph_x(12)
    ));
    assert!(background.y < marked.baseline && background.y + background.height > marked.baseline);
    assert_eq!(background.radius, 2.);
    assert_eq!(
        background.fill,
        Some(on_paper(
            colors.highlight.get(theme::HighlightColor::Yellow)
        ))
    );
    // A link has the accent as its colour, whatever colour the text was
    // given.
    let accent = paper_color(colors.accent);
    assert_eq!(
        marked
            .runs
            .iter()
            .map(|run| (run.end, run.color))
            .collect::<Vec<_>>(),
        vec![
            (6, paper_color(colors.ink)),
            (12, accent),
            (18, paper_color(colors.ink))
        ]
    );
    assert!(close(underline.x, background.x) && close(underline.width, background.width));
    assert!(underline.y > marked.baseline && underline.height >= 1.);
    assert_eq!(underline.fill, Some(accent));

    // Nothing of the page break marker, on either page.
    for page in &pages {
        assert!(
            labels_of(page)
                .iter()
                .all(|label| label.text != "Page break")
        );
    }
    assert_eq!(row_texts(&pages)[1], vec!["After the break.".to_string()]);
}

#[gpui::test]
fn a_row_says_where_every_glyph_starts(cx: &mut TestAppContext) {
    setup(cx);
    let plain = InlineAttrs::default;
    let style = |style: InlineStyle| InlineAttrs::default().with(style);
    let (editor, cx) = open(
        paged(vec![
            Block::paragraph("naïve café"),
            Block::paragraph("Centred.").with_alignment(Alignment::Center),
            Block::paragraph("At the right.").with_alignment(Alignment::Right),
            Block::paragraph(LONG.repeat(2).trim_end()).with_alignment(Alignment::Justify),
            Block::new(BlockKind::Paragraph)
                .with_span("some ", plain())
                .with_span("code", style(InlineStyle::Code))
                .with_span(" and ", plain())
                .with_span("bold", style(InlineStyle::Bold))
                .with_span(" and ", plain())
                .with_span("italic", style(InlineStyle::Italic)),
            Block::with_text(BlockKind::heading(1), "A heading"),
            Block::paragraph("Two lines\nin one block."),
        ]),
        EditorConfig::full(),
        cx,
    );
    let pages = pages_of(&editor, cx);
    let rows = rows_of(&pages[0]);
    let row = |text: &str| -> &TextRow {
        rows.iter()
            .find(|row| row.text.starts_with(text))
            .copied()
            .unwrap_or_else(|| panic!("no row {text:?}"))
    };
    let text_width = A4.0 - 2. * MARGIN;
    let em = 0.6 * 17.;

    // One start for every character, by its byte in the row's text, 0.6 em
    // apart from the left edge of the text area; the row is as wide as its
    // characters.
    let first = row("naïve");
    assert!(close(first.x, MARGIN));
    assert_eq!(first.size, 17.);
    assert_eq!(
        first
            .glyphs
            .iter()
            .map(|glyph| glyph.byte)
            .collect::<Vec<_>>(),
        vec![0, 1, 2, 4, 5, 6, 7, 8, 9, 10]
    );
    for (index, glyph) in first.glyphs.iter().enumerate() {
        assert!(
            close(glyph.x, index as f32 * em),
            "glyph {index} at {}",
            glyph.x
        );
        assert_eq!(glyph.scale, 1.);
    }
    assert!(close(first.width, 10. * em));
    assert_eq!(first.runs.len(), 1);
    assert_eq!(
        (first.runs[0].font.weight, first.runs[0].font.italic),
        (400., false)
    );

    // The alignment is in the starts: the row's own x is the text area's.
    let centred = row("Centred.");
    assert!(close(centred.x, MARGIN));
    assert!(close(centred.glyphs[0].x, (text_width - 8. * em) / 2.));
    let right = row("At the right.");
    assert!(close(right.glyphs[0].x, text_width - 13. * em));

    // A justified row: its last word ends at the right edge of the text
    // area, and its shaped width is still the natural one.
    let justified = row("A sentence");
    let last_letter = justified
        .glyphs
        .iter()
        .rev()
        .find(|glyph| !justified.text[glyph.byte as usize..].starts_with(' '))
        .unwrap();
    assert!(
        close(last_letter.x + em, text_width),
        "{}",
        last_letter.x + em
    );
    assert!(close(
        justified.width,
        justified.text.chars().count() as f32 * em
    ));
    assert!(justified.width < text_width);

    // Inline code is smaller, in the monospace font; bold and italic are
    // weights and styles of the body font.
    let runs = row("some code");
    assert_eq!(runs.text, "some code and bold and italic");
    assert_eq!(
        runs.runs.iter().map(|run| run.end).collect::<Vec<_>>(),
        vec![5, 9, 14, 18, 23, 29]
    );
    let body = &runs.runs[0].font;
    assert_eq!((body.weight, body.italic), (400., false));
    assert_ne!(
        runs.runs[1].font.family, body.family,
        "code is set in the monospace font"
    );
    assert_eq!(
        (runs.runs[3].font.weight, runs.runs[3].font.italic),
        (700., false)
    );
    assert_eq!(runs.runs[3].font.family, body.family);
    assert_eq!(
        (runs.runs[5].font.weight, runs.runs[5].font.italic),
        (400., true)
    );
    for glyph in &runs.glyphs {
        let code = (5..9).contains(&glyph.byte);
        assert_eq!(glyph.scale, if code { INLINE_CODE_SCALE } else { 1. });
    }
    // The fonts are shared between the rows that name them.
    assert!(std::sync::Arc::ptr_eq(body, &first.runs[0].font));
    assert!(
        !body.fallbacks.is_empty(),
        "the body font has its fallback families"
    );

    // A heading is bold and larger.
    let heading = row("A heading");
    assert_eq!(heading.runs[0].font.weight, 700.);
    assert_eq!(heading.size, 34.);
    assert!(close(heading.glyphs[1].x, 0.6 * 34.));

    // A line break inside a block ends a row; it is not in the row's text.
    assert_eq!(row("Two lines").text, "Two lines");
    assert_eq!(row("in one block.").text, "in one block.");
    assert!(row("in one block.").baseline > row("Two lines").baseline + 20.);
}

/// The rows of the first sheet of the last frame, from the sheet's corner.
fn painted_on_first_sheet(editor: &Editor) -> Vec<(f32, f32)> {
    let sheet = editor.painted_sheets[0];
    editor
        .painted_rows
        .iter()
        .map(|(origin, _)| {
            (
                f32::from(origin.x - sheet.left()),
                f32::from(origin.y - sheet.top()),
            )
        })
        .filter(|(_, y)| *y < f32::from(sheet.size.height))
        .collect()
}

#[gpui::test]
fn rows_and_labels_are_where_page_view_paints_them(cx: &mut TestAppContext) {
    setup(cx);
    let mut blocks = chapter(0);
    blocks.extend(chapter(1));
    let (editor, cx) = open(paged(blocks), EditorConfig::full(), cx);
    // A window that shows a whole sheet at 100 %.
    cx.simulate_resize(size(px(900.), px(1000.)));
    editor.update(cx, |editor, cx| editor.set_zoom(Zoom::Percent(100), cx));
    cx.run_until_parked();
    settle(&editor, cx);
    cx.run_until_parked();
    let pages = pages_of(&editor, cx);

    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.zoom_percent(), 100.);
        let sheet = editor.painted_sheets[0];
        assert!(close(f32::from(sheet.size.width), A4.0));
        // Every row with text that the frame painted on the first sheet is a
        // row of the first page, at the same place from the sheet's corner.
        // (The frame also plans the empty row of a rule.)
        let painted = painted_on_first_sheet(editor);
        let rows = rows_of(&pages[0]);
        assert!(rows.len() > 20);
        for row in &rows {
            // The top of a row is its baseline less the baseline's distance
            // from the top, which is the same for the rows of one size:
            // compare the differences between neighbours of one block
            // instead, and the x of all.
            assert!(
                painted.iter().any(|(x, _)| close(*x, row.x)),
                "no painted row at x {}",
                row.x
            );
        }
        // Row for row: the frame's rows with text are in the order of the
        // page's rows; their tops differ from the baselines by one number
        // per text size.
        let mut offsets: Vec<(f32, f32)> = Vec::new();
        let mut painted_with_text = painted.iter();
        for row in &rows {
            let found = painted_with_text
                .find(|(x, y)| close(*x, row.x) && *y < row.baseline && *y > row.baseline - 40.)
                .unwrap_or_else(|| panic!("the row {:?} was not painted", row.text));
            let offset = row.baseline - found.1;
            match offsets.iter().find(|(size, _)| *size == row.size) {
                Some((_, known)) => assert!(
                    close(*known, offset),
                    "{:?}: {offset} from the top, others of its size {known}",
                    row.text
                ),
                None => offsets.push((row.size, offset)),
            }
        }
        assert!(offsets.len() >= 2, "{offsets:?}");

        // The labels: the frame's markers on the first sheet are the page's
        // labels. A number ends where its label is anchored; the page
        // number is centred on the sheet.
        let markers: Vec<Bounds<Pixels>> = editor
            .painted_markers
            .iter()
            .copied()
            .filter(|marker| marker.top() >= sheet.top() && marker.bottom() <= sheet.bottom())
            .collect();
        let labels = labels_of(&pages[0]);
        assert_eq!(markers.len(), labels.len(), "{labels:?}");
        for label in labels {
            let from_sheet = |marker: &Bounds<Pixels>| {
                (
                    f32::from(marker.left() - sheet.left()),
                    f32::from(marker.right() - sheet.left()),
                    f32::from(marker.top() - sheet.top()),
                    f32::from(marker.bottom() - sheet.top()),
                )
            };
            let found = markers
                .iter()
                .map(from_sheet)
                .any(|(left, right, top, bottom)| {
                    let x = match label.anchor {
                        Anchor::End => close(right, label.x),
                        // The screen puts a centred label on a whole pixel.
                        Anchor::Middle => ((left + right) / 2. - label.x).abs() <= 0.5,
                        Anchor::Start => close(left, label.x),
                    };
                    x && label.baseline > top && label.baseline < bottom
                });
            assert!(found, "no marker where {label:?} is");
        }
    });
}

#[gpui::test]
fn a_block_puts_its_decorations_on_the_page_its_rows_are_on(cx: &mut TestAppContext) {
    setup(cx);
    // A quote of three pages: a bar on each, as high as the rows there.
    let (editor, cx) = open(
        paged(vec![
            Block::paragraph("Before."),
            Block::with_text(BlockKind::Quote, LONG.repeat(40).trim_end()),
            Block::with_text(BlockKind::BulletItem, "After, a bullet."),
        ]),
        EditorConfig::full(),
        cx,
    );
    settle(&editor, cx);
    let pages = pages_of(&editor, cx);
    assert_eq!(
        Some(pages.len()),
        editor.read_with(cx, |editor, _| editor.page_count())
    );
    assert_eq!(pages.len(), 3);
    assert_on_their_sheets(&pages);
    for (number, page) in pages.iter().enumerate() {
        let bars: Vec<&innerword_print::RectItem> = page
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Rect(rect) if rect.width == 3. => Some(rect),
                _ => None,
            })
            .collect();
        assert_eq!(bars.len(), 1, "page {number}");
        let quoted: Vec<&TextRow> = rows_of(page)
            .into_iter()
            .filter(|row| row.text.contains("sentence") || row.text.contains("page."))
            .collect();
        let (first, last) = (quoted[0], quoted[quoted.len() - 1]);
        // From above the first row's ink to below the last row's.
        assert!(
            bars[0].y < first.baseline - first.size * 0.5,
            "page {number}"
        );
        assert!(bars[0].y + bars[0].height > last.baseline, "page {number}");
        assert!(bars[0].y >= MARGIN - 0.02 && bars[0].y + bars[0].height <= A4.1 - MARGIN + 0.02);
    }
    // The bullet after it is on the last page only, once.
    let discs = |page: &Page| {
        page.items
            .iter()
            .filter(|item| {
                matches!(item, Item::Rect(rect) if rect.radius > 0. && close(rect.width, rect.height))
            })
            .count()
    };
    assert_eq!(pages.iter().map(discs).collect::<Vec<_>>(), vec![0, 0, 1]);
}

#[gpui::test]
fn a_page_far_down_a_book_is_as_exact_as_the_first(cx: &mut TestAppContext) {
    setup(cx);
    // The same page 400 times: a chapter and a page break. Far down the
    // content the y of a row is a number in the hundreds of thousands,
    // where single precision has an eighth of a point to offer. A page of
    // the pass is worked out on its own sheet instead.
    let plain = InlineAttrs::default;
    let mut blocks = Vec::new();
    for _ in 0..400 {
        blocks.extend([
            Block::with_text(BlockKind::heading(2), "A chapter"),
            Block::new(BlockKind::Paragraph)
                .with_span(LONG.repeat(3).trim_end(), plain())
                .with_span(
                    " underlined",
                    InlineAttrs {
                        underline: true,
                        highlight: Some(Highlight::Green),
                        ..plain()
                    },
                ),
            Block::with_text(BlockKind::NumberedItem, "A number"),
            Block::with_text(BlockKind::TodoItem { checked: true }, "A box"),
            Block::with_text(BlockKind::Quote, LONG.repeat(2).trim_end()),
            Block::with_text(BlockKind::code(), "code();\nmore();"),
            Block::new(BlockKind::Divider),
            Block::new(BlockKind::PageBreak),
        ]);
    }
    // And a paragraph that goes on over several sheets at the very end.
    blocks.push(Block::paragraph(LONG.repeat(50).trim_end()));
    let (editor, cx) = open(paged(blocks), EditorConfig::full(), cx);
    settle(&editor, cx);
    let pages = pages_of(&editor, cx);
    assert_eq!(
        Some(pages.len()),
        editor.read_with(cx, |editor, _| editor.page_count())
    );
    assert!(pages.len() > 402);
    assert_on_their_sheets(&pages);

    // Every one of the 400 pages is the first page, to the last bit, but
    // for its number.
    let (first, number) = pages[0].items.split_at(pages[0].items.len() - 1);
    assert!(first.len() > 15);
    assert!(matches!(&number[0], Item::Label(label) if label.text == "1"));
    for (index, page) in pages[..400].iter().enumerate() {
        let (items, number) = page.items.split_at(page.items.len() - 1);
        assert!(items == first, "page {} differs from page 1", index + 1);
        let (Item::Label(label), Item::Label(first_number)) =
            (&number[0], &pages[0].items[first.len()])
        else {
            panic!("no page number");
        };
        assert_eq!(label.text, (index + 1).to_string());
        assert_eq!(
            (label.x, label.baseline, label.size),
            (first_number.x, first_number.baseline, first_number.size)
        );
    }

    // The long paragraph: on each of its sheets after the first its rows
    // begin at the top of the text area, one line height apart.
    let tail = &pages[400..];
    assert!(tail.len() >= 3);
    let line_height = 27.;
    let first_baseline = rows_of(&tail[1])[0].baseline;
    for page in &tail[1..] {
        let rows = rows_of(page);
        for (index, row) in rows.iter().enumerate() {
            let wanted = first_baseline + index as f32 * line_height;
            assert!(
                (row.baseline - wanted).abs() < 0.001,
                "row {index}: {} and not {wanted}",
                row.baseline
            );
        }
    }
    // The first row of a sheet has its top at the top margin.
    let top_of_first = rows_of(&pages[0])[0].baseline;
    assert!(first_baseline > MARGIN && first_baseline < MARGIN + line_height);
    assert!(top_of_first > MARGIN);
}

/// Where a test leaves a PDF to be looked at: the folder
/// `INNER_WORD_PRINT_QA_DIR` names, if it is set.
fn keep_for_looking(name: &str, pdf: &innerword_print::Pdf) {
    let Some(folder) = std::env::var_os("INNER_WORD_PRINT_QA_DIR") else {
        return;
    };
    let folder = std::path::PathBuf::from(folder);
    std::fs::create_dir_all(&folder).expect("the folder for the PDF");
    std::fs::write(folder.join(format!("{name}.pdf")), pdf.bytes()).expect("the PDF written");
    let file = PdfFile::open(pdf.bytes()).expect("a PDF");
    for page in 0..file.page_count() {
        let png = file.page_png(page, 2.).expect("the page as a picture");
        std::fs::write(folder.join(format!("{name}-{}.png", page + 1)), png).expect("the picture");
    }
}

#[gpui::test]
fn the_pages_of_the_pass_make_a_pdf_with_the_documents_text(cx: &mut TestAppContext) {
    setup(cx);
    // Every block kind, set in Courier New: its glyphs are as wide as the
    // test text system says, so the PDF reads back as the text.
    let mut blocks = vec![Block::with_text(BlockKind::heading(1), "The print pass")];
    for number in 0..4 {
        blocks.extend(chapter(number));
    }
    blocks.push(Block::new(BlockKind::PageBreak));
    blocks.push(Block::paragraph("The last page."));
    let (editor, cx) = open(paged_in_courier(blocks), EditorConfig::full(), cx);
    settle(&editor, cx);
    let (paper, pages) = cx.update(|window, cx| {
        let mut pass = editor.read(cx).print_pass(window, cx);
        (pass.paper(), pass.run_to_end())
    });
    assert_eq!(
        Some(pages.len()),
        editor.read_with(cx, |editor, _| editor.page_count())
    );

    let mut writer = PdfWriter::new(&JobInfo {
        title: "The print pass".to_string(),
        creator: "Inner Word (a test of the editor)".to_string(),
    });
    for page in &pages {
        writer.add_page(page);
    }
    let pdf = writer.finish();
    keep_for_looking("print-pass", &pdf);
    let file = PdfFile::open(pdf.bytes()).expect("the pages made a PDF");
    assert_eq!(file.page_count(), pages.len());
    assert_eq!(pdf.report.pages, pages.len());
    assert_eq!(
        pdf.report.rows,
        pages.iter().map(|page| rows_of(page).len()).sum::<usize>()
    );
    // The faces the layout asked for: Courier New in three of them and the
    // interface font of the labels. (The test platform lists no installed
    // fonts, so the theme names the first family of its monospace stack
    // whether this Mac has it or not; a family it does not have is named
    // as missing and set in GPUI's last resort.)
    assert!(
        pdf.report
            .missing_families
            .iter()
            .all(|family| family == "SF Mono"),
        "{:?}",
        pdf.report
    );
    let fonts = &pdf.report.fonts;
    for name in [
        "CourierNewPSMT",
        "CourierNewPS-BoldMT",
        "CourierNewPS-ItalicMT",
    ] {
        assert!(
            fonts.iter().any(|font| font == name),
            "{name} is not among {fonts:?}"
        );
    }
    assert!(
        fonts.iter().any(|font| font.starts_with(".SF")),
        "{fonts:?}"
    );

    let squeezed = |text: &str| {
        text.chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
    };
    for (number, page) in pages.iter().enumerate() {
        let (width, height) = file.page_size(number);
        assert!(close(width, paper.0) && close(height, paper.1));
        // The text of the page is the text of its rows and labels (a list
        // number stands before its row on the same line, the language of a
        // code block over its first line, the page number last).
        // (PDFKit reads a glyph back as one of the characters the font maps
        // to it: 日 comes back as the Kangxi radical of the same shape.)
        let found = squeezed(&file.page_text(number)).replace('\u{2f47}', "日");
        for row in rows_of(page) {
            assert!(
                found.contains(&squeezed(&row.text)),
                "page {}: {:?} is not in {:?}",
                number + 1,
                row.text,
                file.page_text(number)
            );
        }
        for label in labels_of(page) {
            assert!(
                found.contains(&label.text),
                "page {}: {:?}",
                number + 1,
                label.text
            );
        }
        assert!(found.ends_with(&(number + 1).to_string()));
        let wanted: usize = rows_of(page)
            .iter()
            .map(|row| squeezed(&row.text).chars().count())
            .chain(
                labels_of(page)
                    .iter()
                    .map(|label| label.text.chars().count()),
            )
            .sum();
        assert_eq!(
            found.chars().count(),
            wanted,
            "page {}: nothing else is on it",
            number + 1
        );
    }
    // A word is where its row says: the first word of the second paragraph.
    let first = rows_of(&pages[0])
        .into_iter()
        .find(|row| row.text.starts_with("A sentence"))
        .unwrap();
    let (x, top, _, box_height) = file.find(0, "sentence").expect("the word on the page");
    assert!(close(x, first.x + 2. * 0.6 * first.size), "{x}");
    assert!(top < first.baseline && top + box_height > first.baseline);
    // Rows in Courier New are as wide in the PDF as in the layout: the
    // writer found the faces the layout meant. (The rows that are not are
    // those with characters Courier New does not have and those with code,
    // which real fonts do not set 0.6 em wide.)
    let courier_alone = |row: &TextRow| {
        row.text.is_ascii() && row.runs.iter().all(|run| run.font.family == "Courier New")
    };
    let others = pages
        .iter()
        .flat_map(rows_of)
        .filter(|row| !courier_alone(row))
        .count();
    assert!(others > 0 && others < pdf.report.rows / 2);
    assert!(pdf.report.rows_off <= others, "{:?}", pdf.report);
    let mut writer = PdfWriter::new(&JobInfo {
        title: String::new(),
        creator: String::new(),
    });
    for page in &pages {
        let mut page = page.clone();
        page.items
            .retain(|item| matches!(item, Item::Row(row) if courier_alone(row)));
        writer.add_page(&page);
    }
    let report = writer.finish().report;
    assert_eq!(report.rows, pdf.report.rows - others);
    assert_eq!(report.rows_off, 0, "{report:?}");
}

/// GPUI's test text system shapes everything left to right, so the rows of
/// a right-to-left paragraph are given here as CoreText makes them: a
/// Hebrew line of 40 letters of two bytes, 0..80, cut into rows of 40
/// bytes by logical order, the first row painting the visual left of the
/// line, which is its logical end.
#[test]
fn a_row_on_paper_is_the_text_its_glyphs_were_shaped_from() {
    let clusters: Vec<usize> = (0..40).map(|letter| letter * 2).collect();
    // Text that runs one way: the row's own range, whatever glyph stands
    // at its end.
    let ltr: Vec<usize> = (0..20).map(|letter| letter * 2).collect();
    assert_eq!(painted_pieces(0..40, &ltr, &clusters, 80), vec![0..40]);
    assert_eq!(painted_pieces(0..40, &[0, 40], &clusters, 80), vec![0..40]);
    // The first row by bytes paints the letters 20..40 (bytes 40..80),
    // right to left: its text on paper is those bytes, every glyph at a
    // cluster of its own, not 0..40 with every glyph folded onto an end.
    let visual_left: Vec<usize> = (20..40).rev().map(|letter| letter * 2).collect();
    assert_eq!(
        painted_pieces(0..40, &visual_left, &clusters, 80),
        vec![40..80]
    );
    let visual_right: Vec<usize> = (0..20).rev().map(|letter| letter * 2).collect();
    assert_eq!(
        painted_pieces(40..80, &visual_right, &clusters, 80),
        vec![0..40]
    );
    // A cluster of two glyphs (a letter with its point) is one piece.
    let pointed = [0, 2, 6];
    assert_eq!(
        painted_pieces(60..80, &pointed, &[0, 2, 6, 8], 80),
        vec![0..8]
    );
    // Glyphs of two stretches that do not touch: two pieces, in byte order.
    assert_eq!(
        painted_pieces(0..10, &[70, 72, 20], &clusters, 80),
        vec![20..22, 70..74]
    );
}
