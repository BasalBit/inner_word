//! Page view: pagination invariants, page counts, the caret's page,
//! scaling to a narrow window, switching views.

use gpui::{TestAppContext, px, size};
use innerword_core::{
    Block, BlockKind, Document, DocumentSettings, Margins, Orientation, PageSetup, PaperSize,
    Position,
};

use super::slice::{open, setup};
use crate::{Editor, EditorConfig, layout::pages::PAGE_GAP};

/// A4 portrait with one-inch margins: 595.28 x 841.89 points, a text area
/// of 451.28 x 697.89.
fn paged(blocks: Vec<Block>) -> Document {
    Document::from_blocks(blocks).with_settings(DocumentSettings {
        page_view: true,
        ..DocumentSettings::default()
    })
}

fn one_row_paragraphs(count: usize) -> Vec<Block> {
    (0..count)
        .map(|ix| Block::paragraph(&format!("Paragraph {ix}")))
        .collect()
}

/// Checks, for every row of the document, that it lies completely inside
/// the text area of exactly one sheet and that rows go down the sheets in
/// document order. Returns the number of sheets that carry rows.
fn assert_rows_on_sheets(editor: &Editor) -> usize {
    editor
        .with_layout(|lay| {
            let (geometry, pad_top) = {
                let env = lay.env();
                (env.page.expect("page view"), env.pad_top)
            };
            let mut previous = (0, px(-1.));
            let mut last_page = 0;
            for index in 0..lay.count() {
                let placed = lay.placed(index).unwrap();
                for row in 0..placed.layout.rows.len() {
                    let top = placed.row_top(row);
                    let bottom = top + placed.layout.line_height;
                    let page = lay.page_at(top);
                    let text_top = geometry.sheet_top(page, pad_top) + geometry.margin_top;
                    let text_bottom = text_top + geometry.content_height;
                    assert!(
                        top >= text_top - px(0.02),
                        "block {index} row {row} starts above the text area of sheet {page}"
                    );
                    assert!(
                        bottom <= text_bottom + px(0.02),
                        "block {index} row {row} crosses the bottom of sheet {page}: \
                         {bottom:?} > {text_bottom:?}"
                    );
                    assert!(
                        (page, top) > previous,
                        "block {index} row {row} is not below the row before it"
                    );
                    previous = (page, top);
                    last_page = page;
                }
            }
            last_page + 1
        })
        .expect("laid out")
}

#[gpui::test]
fn known_page_counts(cx: &mut TestAppContext) {
    setup(cx);
    // One-row paragraphs are 27px rows 39px apart: 18 fit in the 697.89px
    // text area (17 * 39 + 27 = 690).
    let (editor, cx) = open(paged(one_row_paragraphs(100)), EditorConfig::full(), cx);
    editor.update(cx, |editor, cx| {
        editor.finish_layout(cx);
        assert_eq!(editor.page_count(), Some(6));
        assert_eq!(assert_rows_on_sheets(editor), 6);
        assert_eq!(editor.caret_page(), Some(1));
        // Block 17 is the last on sheet 1, block 18 the first on sheet 2.
        editor.edit(|state| state.set_caret(Position::new(17, 2)), cx);
        assert_eq!(editor.caret_page(), Some(1));
        editor.edit(|state| state.set_caret(Position::new(18, 0)), cx);
        assert_eq!(editor.caret_page(), Some(2));
        editor.edit(|state| state.set_caret(Position::new(99, 0)), cx);
        assert_eq!(editor.caret_page(), Some(6));
    });

    // A page break after the first paragraph: the other 99 start on sheet
    // 2 and need 6 sheets (5 * 18 + 9).
    cx.simulate_keystrokes("cmd-up cmd-right cmd-enter");
    editor.update(cx, |editor, cx| {
        editor.finish_layout(cx);
        assert!(matches!(
            editor.document().blocks()[1].kind(),
            BlockKind::PageBreak
        ));
        assert_eq!(editor.page_count(), Some(7));
        assert_eq!(assert_rows_on_sheets(editor), 7);
        editor.edit(|state| state.set_caret(Position::new(2, 0)), cx);
        assert_eq!(editor.caret_page(), Some(2));
    });
    // Undo removes the break and the sheet.
    cx.simulate_keystrokes("cmd-z");
    editor.update(cx, |editor, cx| {
        editor.finish_layout(cx);
        assert_eq!(editor.page_count(), Some(6));
    });

    // A trailing page break opens an empty sheet.
    cx.simulate_keystrokes("cmd-down cmd-enter");
    editor.update(cx, |editor, cx| {
        editor.finish_layout(cx);
        assert_eq!(editor.page_count(), Some(7));
    });

    // Flow view has no pages.
    editor.update(cx, |editor, cx| {
        editor.set_page_view(false, cx);
        assert_eq!(editor.page_count(), None);
        assert_eq!(editor.caret_page(), None);
    });
}

#[gpui::test]
fn rows_never_cross_a_sheet_and_headings_stay_with_their_text(cx: &mut TestAppContext) {
    setup(cx);
    let long = "A sentence that is long enough to wrap in the text area of a page. ".repeat(9);
    let mut blocks = Vec::new();
    for chapter in 0..12 {
        blocks.push(Block::with_text(
            BlockKind::heading(2),
            &format!("Chapter {chapter}"),
        ));
        for paragraph in 0..(3 + chapter % 4) {
            blocks.push(Block::paragraph(&long[..long.len() - paragraph * 37]));
        }
        blocks.push(Block::with_text(BlockKind::BulletItem, "a list item"));
        blocks.push(Block::with_text(BlockKind::BulletItem, long.trim_end()).with_indent(1));
        blocks.push(Block::with_text(
            BlockKind::code(),
            &"let line = of_code();\n".repeat(11 + chapter),
        ));
        blocks.push(Block::with_text(BlockKind::Quote, long.trim_end()));
        if chapter % 5 == 4 {
            blocks.push(Block::new(BlockKind::PageBreak));
        }
        blocks.push(Block::new(BlockKind::Divider));
    }
    // One block far taller than a sheet.
    blocks.push(Block::with_text(
        BlockKind::code(),
        &"a very long listing\n".repeat(90),
    ));
    let (editor, cx) = open(paged(blocks), EditorConfig::full(), cx);
    editor.update(cx, |editor, cx| {
        editor.finish_layout(cx);
        let sheets = assert_rows_on_sheets(editor);
        assert_eq!(editor.page_count(), Some(sheets));
        assert!(sheets > 12);

        // No heading is the last thing on its sheet: the first row of the
        // block after it is on the same sheet.
        let page_of = |position: Position| {
            editor
                .with_layout(|lay| {
                    let placed = lay.placed(position.block).unwrap();
                    let row = placed.layout.row_for_offset(position.offset);
                    lay.page_at(placed.row_top(row))
                })
                .unwrap()
        };
        let document = editor.document();
        let mut headings = 0;
        for (index, block) in document.blocks().iter().enumerate() {
            if block.kind().heading_level().is_some() {
                headings += 1;
                assert_eq!(
                    page_of(Position::new(index, 0)),
                    page_of(Position::new(index + 1, 0)),
                    "heading {index} was left behind"
                );
            }
            // The block after a page break starts a sheet.
            if matches!(block.kind(), BlockKind::PageBreak) {
                assert_eq!(
                    page_of(Position::new(index + 1, 0)),
                    page_of(Position::new(index, 0)) + 1
                );
            }
        }
        assert_eq!(headings, 12);

        // The tall block spans several sheets, at row boundaries.
        let tall = document.block_count() - 1;
        let end = document.blocks()[tall].len();
        assert!(page_of(Position::new(tall, end)) >= page_of(Position::new(tall, 0)) + 2);

        // Typing keeps the invariants (the layout is incremental).
        editor.edit(|state| state.set_caret(Position::new(1, 0)), cx);
        for _ in 0..40 {
            editor.edit(|state| state.insert_text("more words to push rows "), cx);
        }
        editor.finish_layout(cx);
        assert_eq!(editor.page_count(), Some(assert_rows_on_sheets(editor)));
    });
}

#[gpui::test]
fn page_setup_changes_the_sheets(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(paged(one_row_paragraphs(100)), EditorConfig::full(), cx);
    let sheet = |editor: &Editor| {
        editor
            .with_layout(|lay| {
                let geometry = lay.env().page.unwrap();
                (
                    geometry.sheet_width,
                    geometry.sheet_height,
                    lay.env().column_width,
                )
            })
            .unwrap()
    };
    editor.update(cx, |editor, cx| {
        editor.finish_layout(cx);
        let (width, height, column) = sheet(editor);
        assert!((width - px(595.28)).abs() < px(0.1));
        assert!((height - px(841.89)).abs() < px(0.1));
        assert!((column - px(451.28)).abs() < px(0.1));

        // US Letter, landscape, half-inch margins.
        let mut settings = editor.document().settings().clone();
        settings.page = PageSetup {
            paper: PaperSize::UsLetter,
            orientation: Orientation::Landscape,
            margins: Margins::uniform(36.),
        };
        editor.edit(|state| state.set_settings(settings), cx);
    });
    cx.run_until_parked();
    editor.update(cx, |editor, cx| {
        editor.finish_layout(cx);
        let (width, height, column) = sheet(editor);
        assert_eq!((width, height, column), (px(792.), px(612.), px(720.)));
        // 540px of text: 14 one-row paragraphs per sheet (13 * 39 + 27 =
        // 534), 100 paragraphs on 8 sheets.
        assert_eq!(editor.page_count(), Some(8));
        assert_eq!(assert_rows_on_sheets(editor), 8);
    });
}

#[gpui::test]
fn a_narrow_window_scrolls_the_page_sideways_and_fit_width_fits_it(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(paged(one_row_paragraphs(100)), EditorConfig::full(), cx);
    editor.update(cx, |editor, cx| editor.finish_layout(cx));
    let rows_before = editor.read_with(cx, |editor, _| super::zoom::row_starts(editor));
    cx.simulate_resize(size(px(420.), px(700.)));
    cx.run_until_parked();
    let sheet = |editor: &Editor| {
        // Window x of the sheet's left and right edge.
        let text = editor.position_bounds(Position::new(3, 0)).unwrap().left();
        editor
            .with_layout(|lay| {
                let geometry = lay.env().page.unwrap();
                let zoom = lay.env().zoom;
                let left = text - geometry.margin_left * zoom;
                (left, left + geometry.sheet_width * zoom, zoom)
            })
            .unwrap()
    };
    editor.update(cx, |editor, cx| {
        editor.finish_layout(cx);
        let viewport = editor.viewport_bounds().unwrap();
        assert_eq!(viewport.size.width, px(420.));
        // The sheet keeps its size: it is wider than the window now, and
        // the view scrolls sideways instead.
        let (left, right, zoom) = sheet(editor);
        assert_eq!(zoom, crate::PAGE_VIEW_SCALE);
        assert!(right - left > viewport.size.width);
        assert!(editor.max_horizontal_scroll() > px(200.));
        // The same lines on the same pages as in the wide window.
        assert_eq!(editor.page_count(), Some(6));
        assert_eq!(assert_rows_on_sheets(editor), 6);
        assert_eq!(super::zoom::row_starts(editor), rows_before);

        // Fit width: the sheet between the edges, nothing to scroll.
        editor.set_zoom(crate::Zoom::FitWidth, cx);
    });
    cx.run_until_parked();
    editor.update(cx, |editor, cx| {
        editor.finish_layout(cx);
        let viewport = editor.viewport_bounds().unwrap();
        let (left, right, zoom) = sheet(editor);
        assert!(zoom < 0.7 && zoom > 0.6, "{zoom}");
        assert!(left >= viewport.left() + px(15.));
        assert!(right <= viewport.right() - px(15.));
        assert_eq!(editor.max_horizontal_scroll(), px(0.));
        assert_eq!(editor.page_count(), Some(6));
        assert_eq!(assert_rows_on_sheets(editor), 6);
        assert_eq!(super::zoom::row_starts(editor), rows_before);
        let row = editor.position_bounds(Position::new(3, 0)).unwrap();
        assert!((row.size.height - px(27. * zoom)).abs() < px(0.01));
        // Everything is inside the sheet horizontally.
        let end = editor.document().blocks()[3].len();
        let end_x = editor
            .position_bounds(Position::new(3, end))
            .unwrap()
            .left();
        assert!(row.left() > left && end_x < right);
    });
}

#[gpui::test]
fn switching_views_keeps_the_caret_on_screen(cx: &mut TestAppContext) {
    setup(cx);
    let long = "A sentence that is long enough to wrap in the text area of a page. ".repeat(5);
    let blocks: Vec<Block> = (0..300)
        .map(|_| Block::paragraph(long.trim_end()))
        .collect();
    let (editor, cx) = open(Document::from_blocks(blocks), EditorConfig::full(), cx);
    let caret_y = |editor: &Editor| {
        editor.caret_bounds().unwrap().top() - editor.viewport_bounds().unwrap().top()
    };

    // Somewhere in the middle of the document and of the viewport.
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.set_caret(Position::new(150, 40)), cx);
        editor.scroll_to_caret(crate::Autoscroll::Center, cx);
    });
    cx.run_until_parked();
    let before = editor.read_with(cx, |editor, _| caret_y(editor));
    assert!(before > px(100.));

    for on in [true, false, true] {
        editor.update(cx, |editor, cx| editor.set_page_view(on, cx));
        cx.run_until_parked();
        editor.read_with(cx, |editor, _| {
            assert_eq!(editor.page_view(), on);
            assert_eq!(editor.selection().head, Position::new(150, 40));
            let after = caret_y(editor);
            assert!(
                (after - before).abs() < px(1.),
                "page view {on}: the caret moved from {before:?} to {after:?}"
            );
        });
    }

    // At the very top the top stays: the first sheet shows its margin.
    editor.update(cx, |editor, cx| {
        editor.set_page_view(false, cx);
        editor.edit(|state| state.set_caret(Position::ZERO), cx);
    });
    cx.run_until_parked();
    editor.update(cx, |editor, cx| {
        assert_eq!(editor.scroll_offset(), px(0.));
        editor.set_page_view(true, cx);
    });
    cx.run_until_parked();
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.scroll_offset(), px(0.));
        // Desk padding, then the sheet's top margin, then the text, at
        // the size page view draws a point at 100 %.
        assert_eq!(caret_y(editor), px(24. + 72.) * crate::PAGE_VIEW_SCALE);
    });

    // The gap between two sheets is desk: PAGE_GAP pixels without rows.
    editor.read_with(cx, |editor, _| {
        let geometry = editor.with_layout(|lay| lay.env().page.unwrap()).unwrap();
        assert_eq!(geometry.gap, px(PAGE_GAP));
    });
}

/// A document with every kind of gap between blocks: heading space, list
/// items, quotes, a code block with inner padding, a divider, an empty
/// paragraph, a paragraph that wraps.
fn mixed_blocks() -> Vec<Block> {
    let long = "The quick brown fox jumps over the lazy dog and keeps running. ".repeat(3);
    vec![
        Block::with_text(BlockKind::heading(1), "A heading"),
        Block::paragraph(long.trim_end()),
        Block::with_text(BlockKind::heading(2), "Another heading"),
        Block::with_text(BlockKind::BulletItem, "one"),
        Block::with_text(BlockKind::BulletItem, "two"),
        Block::paragraph(""),
        Block::with_text(BlockKind::Quote, "a quote"),
        Block::with_text(BlockKind::Quote, "more of it"),
        Block::with_text(BlockKind::code(), "fn main() {\n    body();\n}"),
        Block::new(BlockKind::Divider),
        Block::paragraph("the end"),
    ]
}

/// The selection rectangles of the last frame, top to bottom.
fn painted_selection(editor: &Editor) -> Vec<gpui::Bounds<gpui::Pixels>> {
    let mut rects = editor.painted_selection.clone();
    rects.sort_by(|a, b| a.top().partial_cmp(&b.top()).unwrap());
    rects
}

#[gpui::test]
fn the_selection_is_one_shape_across_blocks(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(
        Document::from_blocks(mixed_blocks()),
        EditorConfig::full(),
        cx,
    );
    cx.simulate_keystrokes("cmd-a");
    cx.run_until_parked();
    editor.read_with(cx, |editor, _| {
        let rects = painted_selection(editor);
        // 3 rows of the long paragraph, 3 of the code block, 9 single rows.
        assert_eq!(rects.len(), 15);
        // Every rectangle starts exactly where the one above it ends: no
        // stripes between rows, between blocks or around the code panel.
        for pair in rects.windows(2) {
            assert_eq!(
                pair[0].bottom(),
                pair[1].top(),
                "a gap or an overlap between {:?} and {:?}",
                pair[0],
                pair[1]
            );
            assert!(pair[0].size.height > px(0.));
        }
        // From the first row's top to the last row's bottom.
        let first = editor.position_bounds(Position::ZERO).unwrap();
        let last = editor.position_bounds(editor.document().end()).unwrap();
        assert_eq!(rects[0].top(), first.top());
        assert_eq!(rects.last().unwrap().bottom(), last.bottom());
        // The last row is not stretched, and rows keep the width of their
        // text: the empty paragraph is a stub.
        assert_eq!(rects.last().unwrap().size.height, last.size.height);
        let stub = &rects[7];
        assert!(stub.size.width > px(0.) && stub.size.width < px(12.));
        assert!(
            stub.size.height > last.size.height,
            "the stub of an empty paragraph also covers the space below it"
        );
        // The geometry API still reports plain rows.
        let rows = editor.range_bounds(editor.selection().range()).unwrap();
        assert_eq!(rows.len(), 15);
        let height = |rects: &[gpui::Bounds<gpui::Pixels>]| {
            rects
                .iter()
                .fold(px(0.), |sum, rect| sum + rect.size.height)
        };
        assert!(height(&rows) < height(&rects), "rows, without the gaps");
    });

    // A selection that ends at the very start of a block paints nothing in
    // that block and does not reach down to it.
    editor.update(cx, |editor, cx| {
        editor.edit(
            |state| {
                state.set_selection(innerword_core::Selection::new(
                    Position::new(3, 1),
                    Position::new(4, 0),
                ))
            },
            cx,
        );
    });
    cx.run_until_parked();
    editor.read_with(cx, |editor, _| {
        let rects = painted_selection(editor);
        let row = editor.position_bounds(Position::new(3, 1)).unwrap();
        assert_eq!(rects.len(), 1);
        assert_eq!(rects[0].size.height, row.size.height);
    });
    // One that takes the first character of the next block does.
    cx.simulate_keystrokes("shift-right");
    cx.run_until_parked();
    editor.read_with(cx, |editor, _| {
        let rects = painted_selection(editor);
        assert_eq!(rects.len(), 2);
        assert_eq!(rects[0].bottom(), rects[1].top());
    });
}

#[gpui::test]
fn the_selection_does_not_cross_the_desk_between_sheets(cx: &mut TestAppContext) {
    setup(cx);
    // 18 one-row paragraphs fill a sheet; the 19th starts the next one.
    let (editor, cx) = open(paged(one_row_paragraphs(30)), EditorConfig::full(), cx);
    editor.update(cx, |editor, cx| editor.finish_layout(cx));
    cx.simulate_keystrokes("cmd-a");
    cx.run_until_parked();
    // Bring the break between the sheets into view.
    editor.update(cx, |editor, cx| {
        editor.scroll_to_position(Position::new(18, 0), crate::Autoscroll::Center, cx)
    });
    cx.run_until_parked();
    editor.read_with(cx, |editor, _| {
        let rects = painted_selection(editor);
        let end_of_sheet = editor.position_bounds(Position::new(17, 0)).unwrap();
        let start_of_next = editor.position_bounds(Position::new(18, 0)).unwrap();
        assert!(start_of_next.top() - end_of_sheet.bottom() > px(PAGE_GAP));
        let viewport = editor.viewport_bounds().unwrap();
        assert!(end_of_sheet.top() > viewport.top() && start_of_next.bottom() < viewport.bottom());
        let mut on_first = 0;
        let mut on_second = 0;
        // Painted rectangles sit on whole device pixels.
        let close = |a: gpui::Pixels, b: gpui::Pixels| (a - b).abs() <= px(0.5);
        for pair in rects.windows(2) {
            if close(pair[0].top(), end_of_sheet.top()) {
                // The last row of the sheet keeps its height; the next
                // rectangle is the first row of the next sheet.
                assert!(close(pair[0].size.height, end_of_sheet.size.height));
                assert!(close(pair[1].top(), start_of_next.top()));
            } else {
                assert_eq!(pair[0].bottom(), pair[1].top());
                if pair[0].top() < end_of_sheet.top() {
                    on_first += 1;
                } else {
                    on_second += 1;
                }
            }
        }
        assert!(on_first > 0 && on_second > 0, "both sheets are on screen");
    });
}

#[gpui::test]
fn the_page_number_keeps_clear_of_the_text_and_fits_small_margins(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(paged(one_row_paragraphs(60)), EditorConfig::full(), cx);
    let set_margins = |cx: &mut gpui::VisualTestContext, margin: f32| {
        editor.update(cx, |editor, cx| {
            let mut settings = editor.document().settings().clone();
            settings.page.margins = Margins::uniform(margin);
            editor.edit(|state| state.set_settings(settings), cx);
            editor.finish_layout(cx);
        });
        cx.run_until_parked();
    };
    // The first sheet's number and the bottom of its text area.
    let number = |cx: &mut gpui::VisualTestContext| {
        editor.read_with(cx, |editor, _| {
            let (sheet_bottom, text_bottom) = editor
                .with_layout(|lay| {
                    let view = lay.view();
                    let env = lay.env();
                    let geometry = env.page.unwrap();
                    let top = geometry.sheet_top(0, env.pad_top);
                    (
                        view.y(top + geometry.sheet_height),
                        view.y(top + geometry.margin_top + geometry.content_height),
                    )
                })
                .unwrap();
            let label = editor
                .painted_markers
                .iter()
                .find(|marker| marker.top() >= text_bottom && marker.bottom() <= sheet_bottom)
                .copied();
            (label, text_bottom, sheet_bottom)
        })
    };

    // One-inch margins: the number sits in the lower half of the margin,
    // 36 pt and more under the text.
    cx.simulate_keystrokes("cmd-down cmd-up");
    let (label, text_bottom, sheet_bottom) = number(cx);
    let label = label.expect("a page number in a one-inch margin");
    let points = crate::PAGE_VIEW_SCALE;
    assert!(label.top() - text_bottom >= px(35.) * points, "{label:?}");
    assert!(
        sheet_bottom - label.bottom() >= px(18.) * points,
        "{label:?}"
    );
    // A margin of 14 pt still has one, smaller, inside the margin.
    set_margins(cx, 14.);
    let (small, text_bottom, sheet_bottom) = number(cx);
    let small = small.expect("a page number in a 14 pt margin");
    assert!(small.size.height < label.size.height);
    assert!(small.top() >= text_bottom && small.bottom() <= sheet_bottom);
    // A margin with no room for a number has none.
    set_margins(cx, 6.);
    assert_eq!(number(cx).0, None);
}

#[gpui::test]
fn a_code_block_ends_inside_the_text_area(cx: &mut TestAppContext) {
    setup(cx);
    // Paragraphs fill the first sheet to different heights; the code
    // block after them has rows that would fit and padding that may not.
    for lead in 14..24 {
        let mut blocks = one_row_paragraphs(lead);
        let code: Vec<String> = (0..8).map(|line| format!("line_{line}();")).collect();
        blocks.push(Block::with_text(BlockKind::code(), &code.join("\n")));
        blocks.push(Block::paragraph("after"));
        let (editor, cx) = open(paged(blocks), EditorConfig::full(), cx);
        editor.update(cx, |editor, cx| {
            editor.finish_layout(cx);
            assert_rows_on_sheets(editor);
            editor
                .with_layout(|lay| {
                    let (geometry, pad_top) = {
                        let env = lay.env();
                        (env.page.unwrap(), env.pad_top)
                    };
                    let placed = lay.placed(lead).unwrap();
                    for segment in placed.segments() {
                        let page = lay.page_at(segment.top);
                        let text_top = geometry.sheet_top(page, pad_top) + geometry.margin_top;
                        let text_bottom = text_top + geometry.content_height;
                        assert!(
                            segment.bottom <= text_bottom + px(0.02),
                            "after {lead} paragraphs the panel ends {:?} below the text area",
                            segment.bottom - text_bottom
                        );
                    }
                })
                .unwrap();
        });
        cx.update(|window, _| window.remove_window());
    }
}
