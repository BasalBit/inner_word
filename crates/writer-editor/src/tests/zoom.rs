//! The zoom of the view (see `crate::zoom`): the same lines and pages at
//! every zoom, everything that is drawn at or found from a position in
//! the text, keeping the place, scrolling sideways.
//!
//! Each geometry test runs in flow view and in page view, at 100 %, at a
//! zoom below and at a zoom above.

use std::{cell::RefCell, rc::Rc};

use gpui::{
    Bounds, Context, Entity, EntityInputHandler, IntoElement, Modifiers, MouseButton,
    MouseDownEvent, MouseUpEvent, Pixels, Point, Render, ScrollDelta, ScrollWheelEvent,
    TestAppContext, TouchPhase, VisualTestContext, Window, div, point, prelude::*, px, size,
};
use writer_core::{
    Block, BlockKind, Document, DocumentSettings, InlineAttrs, Position, Selection, TextRange,
};

use super::slice::{open, setup};
use crate::{
    Autoscroll, CaretStyle, CompactOptions, Editor, EditorConfig, EditorEvent, HighlightStyle,
    PopupAnchor, PopupPlacement, Zoom, geometry::CARET_WIDTH, layout::block::SHAPED_BYTES,
    paint::snap,
};

/// The zooms every geometry test runs at: 100 %, one below, one above.
const ZOOMS: [u16; 3] = [100, 75, 150];

/// Where every row of the document starts and which sheet it is on:
/// `(block, byte offset of the row's start, sheet)`. Two layouts that agree
/// on this have the same words on the same lines and the same lines on
/// the same pages.
pub(super) fn row_starts(editor: &Editor) -> Vec<(usize, usize, usize)> {
    editor
        .with_layout(|lay| {
            let mut rows = Vec::new();
            for index in 0..lay.count() {
                let placed = lay.placed(index).expect("a block");
                for (row_ix, row) in placed.layout.rows.iter().enumerate() {
                    let page = lay.page_at(placed.row_top(row_ix));
                    rows.push((index, row.range.start, page));
                }
            }
            rows
        })
        .expect("laid out")
}

fn with_view(document: Document, page_view: bool) -> Document {
    document.with_settings(DocumentSettings {
        page_view,
        ..DocumentSettings::default()
    })
}

/// A document with every kind of block and enough text for a dozen
/// sheets.
fn manuscript(page_view: bool) -> Document {
    let sentence = "The quick brown fox jumps over the lazy dog, and keeps on running. ";
    let code = InlineAttrs {
        code: true,
        ..InlineAttrs::default()
    };
    let bold = InlineAttrs {
        bold: true,
        ..InlineAttrs::default()
    };
    let mut blocks = Vec::new();
    for chapter in 0..10usize {
        blocks.push(Block::with_text(
            BlockKind::heading(1 + (chapter % 3) as u8),
            &format!("Chapter {chapter}: a heading that may need a second line at a large size"),
        ));
        for paragraph in 0..6 {
            let text = sentence.repeat(1 + (chapter + paragraph * 3) % 7);
            blocks.push(Block::paragraph(text.trim_end()));
        }
        blocks.push(
            Block::paragraph("")
                .with_span("Words with ", InlineAttrs::default())
                .with_span("inline_code(in, them)", code.clone())
                .with_span(" and ", InlineAttrs::default())
                .with_span("bold ones that go on for a while", bold.clone())
                .with_span(
                    " until the row is full and breaks more than once.",
                    InlineAttrs::default(),
                ),
        );
        for item in 0..3 {
            blocks.push(Block::with_text(
                BlockKind::NumberedItem,
                sentence.repeat(1 + item).trim_end(),
            ));
        }
        blocks.push(Block::with_text(BlockKind::BulletItem, sentence.trim_end()).with_indent(1));
        blocks.push(Block::with_text(
            BlockKind::Quote,
            sentence.repeat(3).trim_end(),
        ));
        blocks.push(Block::with_text(
            BlockKind::code(),
            "fn main() {\n    println!(\"a line of code that is long enough to wrap in a column\");\n}",
        ));
        if chapter % 4 == 3 {
            blocks.push(Block::with_text(BlockKind::PageBreak, ""));
        }
    }
    with_view(Document::from_blocks(blocks), page_view)
}

/// A short document for looking at positions: wrapped paragraphs, a
/// heading, a list item.
fn sample(page_view: bool) -> Document {
    let long = "A sentence that is long enough to wrap in the column of a page, twice over. ";
    let mut blocks = vec![
        Block::with_text(BlockKind::heading(1), "A heading"),
        Block::paragraph("alpha beta gamma delta"),
        Block::paragraph(long.repeat(4).trim_end()),
        Block::with_text(BlockKind::NumberedItem, "an item of a list"),
        Block::paragraph("omega"),
    ];
    for index in 0..120 {
        blocks.push(Block::paragraph(&format!(
            "Paragraph {index}. {}",
            long.trim_end()
        )));
    }
    with_view(Document::from_blocks(blocks), page_view)
}

fn zoom_to(editor: &Entity<Editor>, zoom: Zoom, cx: &mut VisualTestContext) {
    editor.update(cx, |editor, cx| editor.set_zoom(zoom, cx));
    cx.run_until_parked();
}

/// Opens `document` in a window with room for a column at 150 %.
fn open_wide(
    document: Document,
    cx: &mut TestAppContext,
) -> (Entity<Editor>, &mut VisualTestContext) {
    let (editor, cx) = open(document, EditorConfig::full(), cx);
    cx.simulate_resize(size(px(1600.), px(1000.)));
    cx.run_until_parked();
    (editor, cx)
}

fn click(cx: &mut VisualTestContext, at: Point<Pixels>, count: usize, modifiers: Modifiers) {
    cx.simulate_event(MouseDownEvent {
        position: at,
        modifiers,
        button: MouseButton::Left,
        click_count: count,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        position: at,
        modifiers,
        button: MouseButton::Left,
        click_count: count,
    });
}

fn bounds_of(editor: &Editor, block: usize, offset: usize) -> Bounds<Pixels> {
    editor
        .position_bounds(Position::new(block, offset))
        .expect("laid out")
}

fn close(a: Pixels, b: Pixels) -> bool {
    (a - b).abs() < px(0.02)
}

#[gpui::test]
fn the_same_words_on_the_same_lines_and_pages_at_every_zoom(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(manuscript(true), EditorConfig::full(), cx);
    editor.update(cx, |editor, cx| editor.finish_layout(cx));
    let (rows, pages) = editor.read_with(cx, |editor, _| (row_starts(editor), editor.page_count()));
    assert!(pages.unwrap() >= 12, "{pages:?}");
    assert!(rows.len() > 400);
    // Everything is shaped once, in points. From here on nothing is
    // shaped again: a zoom is not a layout.
    let shaped = SHAPED_BYTES.with(|bytes| bytes.get());

    let same = |cx: &mut VisualTestContext, what: &str| {
        editor.update(cx, |editor, cx| {
            editor.finish_layout(cx);
            assert_eq!(editor.page_count(), pages, "{what}");
            assert_eq!(row_starts(editor), rows, "{what}");
        });
    };
    let zooms = crate::ZOOM_STEPS
        .iter()
        .map(|step| Zoom::Percent(*step))
        .chain([Zoom::Percent(133), Zoom::FitWidth, Zoom::FitPage]);
    for zoom in zooms {
        zoom_to(&editor, zoom, cx);
        same(cx, &format!("{zoom:?}"));
        // Text moved around under the zoom: still the same.
        cx.simulate_keystrokes("pagedown pagedown");
        same(cx, &format!("{zoom:?}, paged down"));
    }
    // Windows of every shape, at a fixed zoom and at one that follows the
    // window.
    for zoom in [Zoom::Percent(125), Zoom::FitWidth] {
        zoom_to(&editor, zoom, cx);
        for (width, height) in [(420., 700.), (900., 480.), (2400., 1300.), (640., 480.)] {
            cx.simulate_resize(size(px(width), px(height)));
            cx.run_until_parked();
            same(cx, &format!("{zoom:?} in a window of {width} x {height}"));
        }
    }
    assert_eq!(
        SHAPED_BYTES.with(|bytes| bytes.get()),
        shaped,
        "a zoom shaped text again"
    );
}

#[gpui::test]
fn flow_view_grows_the_column_with_the_zoom_and_then_reflows(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open_wide(manuscript(false), cx);
    editor.update(cx, |editor, cx| editor.finish_layout(cx));
    let rows = editor.read_with(cx, |editor, _| row_starts(editor));
    // The column in window pixels: from the start of a paragraph to the
    // right edge of the widest row.
    let column = |editor: &Editor| {
        editor
            .with_layout(|lay| {
                let env = lay.env();
                (env.column_left, env.column_width * env.zoom)
            })
            .unwrap()
    };
    let viewport = editor.read_with(cx, |editor, _| editor.viewport_bounds().unwrap());
    assert_eq!(viewport.size.width, px(1600.));

    // While there is room the column grows and nothing reflows.
    for (percent, width) in [(100, 700.), (75, 525.), (150, 1050.), (200, 1400.)] {
        zoom_to(&editor, Zoom::Percent(percent), cx);
        editor.update(cx, |editor, cx| {
            editor.finish_layout(cx);
            let (left, drawn) = column(editor);
            assert!(close(drawn, px(width)), "{percent} %: {drawn:?}");
            // Centred.
            assert!((left - (viewport.left() + (px(1600.) - drawn) / 2.)).abs() <= px(0.5));
            assert_eq!(row_starts(editor), rows, "{percent} %");
            assert_eq!(editor.max_horizontal_scroll(), px(0.));
        });
    }
    // Beyond the width there is, the column keeps the window's width and
    // the text reflows: more rows, nothing to scroll sideways.
    for percent in [250, 300, 400] {
        zoom_to(&editor, Zoom::Percent(percent), cx);
        editor.update(cx, |editor, cx| {
            editor.finish_layout(cx);
            let (left, drawn) = column(editor);
            assert!(close(drawn, px(1600. - 64.)), "{percent} %: {drawn:?}");
            assert!(left >= viewport.left());
            assert!(left + drawn <= viewport.right());
            assert!(row_starts(editor).len() > rows.len(), "{percent} %");
            assert_eq!(editor.max_horizontal_scroll(), px(0.));
            // The end of a full row is inside the window.
            let end = editor.document().blocks()[2].len();
            for offset in (0..end).step_by(7) {
                let at = bounds_of(editor, 2, offset);
                assert!(at.left() >= left - px(0.5) && at.left() <= left + drawn + px(0.5));
            }
        });
    }
    // Fit width: the column as wide as its room.
    zoom_to(&editor, Zoom::FitWidth, cx);
    editor.update(cx, |editor, cx| {
        editor.finish_layout(cx);
        let (_, drawn) = column(editor);
        assert!(close(drawn, px(1600. - 64.)), "{drawn:?}");
        assert!((editor.zoom_percent() - (1600. - 64.) / 700. * 100.).abs() < 0.1);
        assert_eq!(row_starts(editor), rows, "the full column, larger");
    });
    // Back at 100 % the text is as it was.
    zoom_to(&editor, Zoom::Percent(100), cx);
    editor.update(cx, |editor, cx| {
        editor.finish_layout(cx);
        assert_eq!(row_starts(editor), rows);
    });
}

#[gpui::test]
fn positions_are_where_the_text_is_drawn_at_every_zoom(cx: &mut TestAppContext) {
    setup(cx);
    for page_view in [false, true] {
        let (editor, cx) = open_wide(sample(page_view), cx);
        let positions: Vec<Position> = [
            (0, 0),
            (0, 4),
            (1, 0),
            (1, 11),
            (2, 0),
            (2, 150),
            (2, 299),
            (3, 5),
            (4, 5),
            (7, 30),
        ]
        .into_iter()
        .map(|(block, offset)| Position::new(block, offset))
        .collect();
        // At 100 %: every position relative to the first one, in points.
        let reference: Vec<Bounds<Pixels>> = editor.read_with(cx, |editor, _| {
            let origin = bounds_of(editor, 0, 0).origin;
            positions
                .iter()
                .map(|position| {
                    let bounds = editor.position_bounds(*position).unwrap();
                    Bounds::new(bounds.origin - origin, bounds.size)
                })
                .collect()
        });
        let rows = editor.read_with(cx, |editor, _| row_starts(editor));
        for percent in ZOOMS {
            let zoom = f32::from(percent) / 100.;
            zoom_to(&editor, Zoom::Percent(percent), cx);
            let what = format!("page view {page_view}, {percent} %");
            editor.update(cx, |editor, cx| {
                assert_eq!(editor.zoom(), Zoom::Percent(percent));
                assert_eq!(editor.zoom_percent(), f32::from(percent));
                assert_eq!(row_starts(editor), rows, "{what}");
                let origin = bounds_of(editor, 0, 0).origin;
                for (position, reference) in positions.iter().zip(&reference) {
                    let bounds = editor.position_bounds(*position).unwrap();
                    // The same place, `zoom` times as far from the start.
                    assert!(
                        close(bounds.left() - origin.x, reference.left() * zoom)
                            && close(bounds.top() - origin.y, reference.top() * zoom)
                            && close(bounds.size.height, reference.size.height * zoom),
                        "{what}: {position:?} at {bounds:?}, at 100 % {reference:?}"
                    );
                    // Hit testing is the way back, anywhere on the row.
                    for dy in [0.1, 0.5, 0.9] {
                        let at = point(
                            bounds.left() + px(0.4),
                            bounds.top() + bounds.size.height * dy,
                        );
                        assert_eq!(editor.position_at_point(at), Some(*position), "{what}");
                    }
                }
                // The text is drawn where its positions are, at the size
                // the zoom gives it: 17 pt body text, a 34 pt heading.
                let body = bounds_of(editor, 1, 0);
                let drawn = editor
                    .painted_rows
                    .iter()
                    .find(|(origin, _)| close(origin.y, body.top()))
                    .expect("the row is painted");
                assert!(close(drawn.0.x, body.left()), "{what}: {drawn:?} {body:?}");
                assert!(close(drawn.1, px(17.) * zoom), "{what}: {drawn:?}");
                assert!(close(body.size.height, px(27.) * zoom), "{what}");
                let heading = editor.painted_rows[0];
                assert!(close(heading.1, px(34.) * zoom), "{what}: {heading:?}");
                // One glyph of the test platform's font is 0.6 of the
                // size wide: the advance zooms with the type.
                let advance = bounds_of(editor, 1, 1).left() - body.left();
                assert!(close(advance, px(0.6 * 17.) * zoom), "{what}: {advance:?}");

                // The bar caret: where the position is, as tall as its
                // row, and as thin as a caret is at every zoom.
                editor.edit(|state| state.set_caret(Position::new(1, 11)), cx);
                let caret = editor.caret_bounds().unwrap();
                let line = bounds_of(editor, 1, 11);
                assert!(close(caret.size.width, CARET_WIDTH), "{what}: {caret:?}");
                assert!(close(caret.center().x, line.left()), "{what}");
                assert!(close(caret.top(), line.top()) && close(caret.bottom(), line.bottom()));
                // Vim's block caret covers its character; the underline
                // keeps its thickness.
                editor.set_caret_style(CaretStyle::Block, cx);
                let block = editor.caret_bounds().unwrap();
                assert!(close(block.left(), line.left()), "{what}");
                assert!(close(block.size.width, px(0.6 * 17.) * zoom), "{what}");
                assert!(close(block.size.height, line.size.height), "{what}");
                editor.set_caret_style(CaretStyle::Underline, cx);
                let underline = editor.caret_bounds().unwrap();
                assert!(close(underline.size.height, CARET_WIDTH), "{what}");
                assert!(close(underline.size.width, block.size.width), "{what}");
                assert!(close(underline.bottom(), line.bottom()), "{what}");
                editor.set_caret_style(CaretStyle::Bar, cx);

                // A range: one rectangle per row, each on its row.
                let range = TextRange::in_block(2, 10, 290);
                let rects = editor.range_bounds(range).unwrap();
                assert!(rects.len() >= 3, "{what}: the paragraph wraps");
                assert!(close(rects[0].left(), bounds_of(editor, 2, 10).left()));
                assert!(close(rects[0].top(), bounds_of(editor, 2, 10).top()));
                let last = rects.last().unwrap();
                assert!(close(last.right(), bounds_of(editor, 2, 290).left()));
                for pair in rects.windows(2) {
                    assert!(close(pair[0].bottom(), pair[1].top()), "{what}");
                    assert!(close(pair[0].size.height, px(27.) * zoom), "{what}");
                }
                // The selection is painted on those rectangles.
                editor.edit(
                    |state| state.set_selection(Selection::from_range(range)),
                    cx,
                );
                let around = editor.selection_bounds().unwrap();
                assert!(close(around.top(), rects[0].top()));
                assert!(close(around.bottom(), last.bottom()));
            });
            cx.run_until_parked();
            editor.update(cx, |editor, cx| {
                let rects = editor
                    .range_bounds(TextRange::in_block(2, 10, 290))
                    .unwrap();
                let mut painted = editor.painted_selection.clone();
                painted.sort_by(|a, b| a.top().partial_cmp(&b.top()).unwrap());
                assert_eq!(painted.len(), rects.len(), "{what}");
                for (painted, rect) in painted.iter().zip(&rects) {
                    assert!(close(painted.left(), rect.left()), "{what}");
                    assert!(close(painted.size.width, rect.size.width), "{what}");
                    // Vertically on whole device pixels.
                    assert_eq!(painted.top(), snap(rect.top()), "{what}");
                    assert_eq!(painted.bottom(), snap(rect.bottom()), "{what}");
                }
                editor.edit(|state| state.set_caret(Position::ZERO), cx);
            });
            cx.run_until_parked();
        }
    }
}

#[gpui::test]
fn clicks_and_drags_find_the_text_at_every_zoom(cx: &mut TestAppContext) {
    setup(cx);
    for page_view in [false, true] {
        let (editor, cx) = open_wide(sample(page_view), cx);
        for percent in ZOOMS {
            zoom_to(&editor, Zoom::Percent(percent), cx);
            let what = format!("page view {page_view}, {percent} %");
            let at = |cx: &mut VisualTestContext, block, offset| {
                editor.read_with(cx, |editor, _| bounds_of(editor, block, offset).center())
            };
            let selection =
                |cx: &mut VisualTestContext| editor.read_with(cx, |editor, _| editor.selection());

            // A click puts the caret at the boundary nearest to it.
            for (block, offset) in [(1, 7), (2, 160), (0, 3), (3, 4)] {
                let target = at(cx, block, offset) + point(px(1.), px(0.));
                click(cx, target, 1, Modifiers::none());
                assert_eq!(
                    selection(cx),
                    Selection::caret(Position::new(block, offset)),
                    "{what}"
                );
            }
            // Shift-click extends from the anchor.
            let target = at(cx, 4, 2);
            click(cx, target, 1, Modifiers::shift());
            assert_eq!(
                selection(cx),
                Selection::new(Position::new(3, 4), Position::new(4, 2)),
                "{what}"
            );
            // A double click takes the word, a triple click the block.
            let word = at(cx, 1, 8);
            click(cx, word, 2, Modifiers::none());
            editor.read_with(cx, |editor, _| {
                assert_eq!(editor.state().selected_text(), "beta", "{what}");
            });
            click(cx, word, 3, Modifiers::none());
            editor.read_with(cx, |editor, _| {
                assert_eq!(
                    editor.state().selected_text(),
                    "alpha beta gamma delta",
                    "{what}"
                );
            });
            // A drag selects from where it started to where it is, over
            // rows and blocks.
            let (from, over, to) = (at(cx, 1, 6), at(cx, 2, 90), at(cx, 4, 3));
            cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
            cx.simulate_mouse_move(over, MouseButton::Left, Modifiers::none());
            assert_eq!(
                selection(cx),
                Selection::new(Position::new(1, 6), Position::new(2, 90)),
                "{what}"
            );
            cx.simulate_mouse_move(to, MouseButton::Left, Modifiers::none());
            cx.simulate_mouse_up(to, MouseButton::Left, Modifiers::none());
            assert_eq!(
                selection(cx),
                Selection::new(Position::new(1, 6), Position::new(4, 3)),
                "{what}"
            );
            // A right-click on a word selects it and asks for the menu
            // where the pointer is.
            let requests = Rc::new(RefCell::new(Vec::new()));
            let subscription = cx.update(|_, cx| {
                let requests = requests.clone();
                cx.subscribe(&editor, move |_, event: &EditorEvent, _| {
                    if let EditorEvent::ContextMenuRequested(request) = event {
                        requests.borrow_mut().push(request.clone());
                    }
                })
            });
            // (Not inside the selection the drag left: a right-click
            // there keeps the selection.)
            let start = at(cx, 0, 0);
            click(cx, start, 1, Modifiers::none());
            let target = at(cx, 1, 13);
            cx.simulate_mouse_down(target, MouseButton::Right, Modifiers::none());
            cx.simulate_mouse_up(target, MouseButton::Right, Modifiers::none());
            editor.read_with(cx, |editor, _| {
                assert_eq!(editor.state().selected_text(), "gamma", "{what}");
            });
            assert_eq!(requests.borrow().len(), 1, "{what}");
            assert_eq!(requests.borrow()[0].position, target, "{what}");
            drop(subscription);
            let start = at(cx, 0, 0);
            click(cx, start, 1, Modifiers::none());
        }
    }
}

#[gpui::test]
fn marks_behind_and_under_the_text_follow_the_zoom(cx: &mut TestAppContext) {
    setup(cx);
    for page_view in [false, true] {
        let (editor, cx) = open_wide(sample(page_view), cx);
        // A comment on "beta", a find highlight on "gamma", a spelling
        // mark on "delta".
        editor.update(cx, |editor, cx| {
            editor.edit(
                |state| {
                    state.set_selection(Selection::from_range(TextRange::in_block(1, 6, 10)));
                    state.add_comment("note", "me", 0).unwrap();
                    state.set_caret(Position::ZERO);
                },
                cx,
            );
            let colors = writer_ui::theme::ActiveTheme::theme(&**cx).colors;
            editor.set_highlights(
                "find",
                vec![TextRange::in_block(1, 11, 16)],
                HighlightStyle::background(colors.find_match),
                cx,
            );
            let block = &editor.document().blocks()[1];
            let (id, revision) = (block.id(), block.revision());
            assert!(
                editor
                    .set_spelling_marks(id, revision, std::iter::once(17..22), cx)
                    .is_applied()
            );
        });
        for percent in ZOOMS {
            let zoom = f32::from(percent) / 100.;
            zoom_to(&editor, Zoom::Percent(percent), cx);
            let what = format!("page view {page_view}, {percent} %");
            editor.read_with(cx, |editor, _| {
                let word = |start, end| {
                    editor
                        .range_bounds(TextRange::in_block(1, start, end))
                        .unwrap()[0]
                };
                // The tints cover the ink of their words: as wide as the
                // word, inside its row.
                let mut tints = editor.painted_tints.clone();
                tints.sort_by(|a, b| a.left().partial_cmp(&b.left()).unwrap());
                assert_eq!(tints.len(), 2, "{what}: {tints:?}");
                for (tint, word) in tints.iter().zip([word(6, 10), word(11, 16)]) {
                    assert!(close(tint.left(), word.left()), "{what}: {tint:?} {word:?}");
                    assert!(close(tint.size.width, word.size.width), "{what}");
                    assert!(tint.top() >= word.top() && tint.bottom() <= word.bottom());
                    assert!(tint.size.height > word.size.height * 0.5, "{what}");
                }
                // The wave is under its word, as wide as the word, and as
                // heavy as the type is large.
                let delta = word(17, 22);
                assert_eq!(editor.painted_spelling.len(), 1, "{what}");
                let wave = editor.painted_spelling[0];
                assert!(close(wave.left(), delta.left()), "{what}: {wave:?}");
                assert!(close(wave.size.width, delta.size.width), "{what}");
                assert!(wave.top() > delta.top() + delta.size.height * 0.5, "{what}");
                assert!(
                    wave.bottom() <= delta.bottom(),
                    "{what}: {wave:?} {delta:?}"
                );
                assert_eq!(wave.size.height, snap(px(1.) * zoom).max(px(1.)) * 3.);
                // What a click on the marked word would be told.
                assert_eq!(
                    editor.spelling_mark_at(Position::new(1, 19)),
                    Some(TextRange::in_block(1, 17, 22))
                );
                // The number of the list item ends left of its text, the
                // same share of the gutter away at every zoom.
                let item = bounds_of(editor, 3, 0);
                let number = editor
                    .painted_markers
                    .iter()
                    .find(|marker| close(marker.top(), item.top()))
                    .expect("the number of the item");
                assert!(close(item.left() - number.right(), px(7.) * zoom), "{what}");
                assert!(close(number.size.height, px(27.) * zoom), "{what}");
            });
        }
    }
}

struct Card;

impl Render for Card {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .debug_selector(|| "zoom-card".into())
            .w(px(120.))
            .h(px(40.))
    }
}

#[gpui::test]
fn popups_and_input_windows_are_at_the_caret_at_every_zoom(cx: &mut TestAppContext) {
    setup(cx);
    for page_view in [false, true] {
        let (editor, cx) = open_wide(sample(page_view), cx);
        let card = cx.update(|_, cx| cx.new(|_| Card));
        for percent in ZOOMS {
            zoom_to(&editor, Zoom::Percent(percent), cx);
            let what = format!("page view {page_view}, {percent} %");
            for (anchor, placement) in [
                (PopupAnchor::Caret, PopupPlacement::Below),
                (PopupAnchor::Selection, PopupPlacement::Above),
                (
                    PopupAnchor::Position(Position::new(2, 200)),
                    PopupPlacement::Below,
                ),
            ] {
                let expected = editor.update(cx, |editor, cx| {
                    editor.edit(
                        |state| {
                            state.set_selection(Selection::new(
                                Position::new(2, 100),
                                Position::new(2, 20),
                            ))
                        },
                        cx,
                    );
                    editor.show_popup(card.clone().into(), anchor, placement, cx);
                    match anchor {
                        PopupAnchor::Caret => editor.caret_bounds().unwrap(),
                        PopupAnchor::Selection => editor.selection_bounds().unwrap(),
                        PopupAnchor::Position(position) => {
                            editor.position_bounds(position).unwrap()
                        }
                    }
                });
                cx.run_until_parked();
                let popup = cx.debug_bounds("zoom-card").expect("the popup is drawn");
                assert!(close(popup.left(), expected.left()), "{what} {anchor:?}");
                match placement {
                    PopupPlacement::Below => {
                        assert!(close(popup.top(), expected.bottom() + px(6.)), "{what}")
                    }
                    PopupPlacement::Above => {
                        assert!(close(popup.bottom(), expected.top() - px(6.)), "{what}")
                    }
                }
                editor.update(cx, |editor, cx| editor.hide_popup(cx));
            }
            // The rectangle macOS is given for its input windows (the
            // accent menu, the character viewer, an input method), and
            // the way back from a point to a character.
            editor.update_in(cx, |editor, window, cx| {
                editor.edit(|state| state.set_caret(Position::new(1, 6)), cx);
                let caret = bounds_of(editor, 1, 6);
                let any = Bounds::default();
                let empty = editor.bounds_for_range(6..6, any, window, cx).unwrap();
                assert_eq!(empty, caret, "{what}");
                let word = editor.bounds_for_range(6..10, any, window, cx).unwrap();
                assert!(close(word.left(), caret.left()) && close(word.top(), caret.top()));
                assert!(
                    close(word.right(), bounds_of(editor, 1, 10).left()),
                    "{what}"
                );
                let index = editor
                    .character_index_for_point(word.center() + point(px(-1.), px(0.)), window, cx)
                    .unwrap();
                assert_eq!(index, 8, "{what}");
                // The key for the context menu asks for it under the
                // caret.
                editor.edit(|state| state.set_caret(Position::ZERO), cx);
            });
            cx.run_until_parked();
        }
    }
}

#[gpui::test]
fn scrolling_and_page_keys_count_in_what_is_on_screen(cx: &mut TestAppContext) {
    setup(cx);
    for page_view in [false, true] {
        let (editor, cx) = open_wide(sample(page_view), cx);
        for percent in ZOOMS {
            let zoom = f32::from(percent) / 100.;
            zoom_to(&editor, Zoom::Percent(percent), cx);
            let what = format!("page view {page_view}, {percent} %");
            // (The zoom kept the line of the caret where it was, so the
            // view may have left the top: back to it.)
            editor.update(cx, |editor, cx| {
                editor.finish_layout(cx);
                editor.scroll_by_pixels(px(-100_000.), cx);
            });
            cx.run_until_parked();
            let viewport = editor.read_with(cx, |editor, _| editor.viewport_bounds().unwrap());
            let scroll = |cx: &mut VisualTestContext| {
                editor.read_with(cx, |editor, _| editor.scroll_offset())
            };
            assert_eq!(scroll(cx), px(0.), "{what}");

            // A page is what fits in the window at this zoom.
            let rows = editor.read_with(cx, |editor, _| editor.rows_per_page().unwrap());
            let fit = (f32::from(viewport.size.height) / (27. * zoom)).floor() as usize;
            assert_eq!(rows, fit, "{what}");
            editor.read_with(cx, |editor, _| {
                let visible = editor.visible_rows().unwrap();
                // (Fewer than a page of rows: headings are taller, there
                // is space between paragraphs and desk between sheets.)
                assert!(visible.len() <= rows && visible.len() * 2 > rows, "{what}");
                for row in &visible {
                    let bounds = editor.position_bounds(row.start).unwrap();
                    assert!(bounds.top() >= viewport.top() - px(0.5), "{what}");
                    assert!(bounds.bottom() <= viewport.bottom() + px(0.5), "{what}");
                }
            });

            // The wheel moves the text by the pixels it says, at any zoom.
            cx.simulate_event(ScrollWheelEvent {
                position: viewport.center(),
                delta: ScrollDelta::Pixels(point(px(0.), px(-240.))),
                modifiers: Modifiers::none(),
                touch_phase: TouchPhase::Moved,
            });
            assert!(close(scroll(cx), px(240.)), "{what}: {:?}", scroll(cx));
            // A line on the wheel is a line of text as it is drawn.
            cx.simulate_event(ScrollWheelEvent {
                position: viewport.center(),
                delta: ScrollDelta::Lines(point(0., -2.)),
                modifiers: Modifiers::none(),
                touch_phase: TouchPhase::Moved,
            });
            assert!(close(scroll(cx), px(240.) + px(54.) * zoom), "{what}");
            editor.update(cx, |editor, cx| {
                editor.scroll_by_rows(-2, cx);
                assert!(close(editor.scroll_offset(), px(240.)), "{what}");
                editor.scroll_by_pixels(px(-240.), cx);
                assert_eq!(editor.scroll_offset(), px(0.), "{what}");
            });
            cx.run_until_parked();

            // The scroll bar: the thumb is the share of the text that is
            // on screen, and dragging it to the end is the end.
            let geometry = editor.read_with(cx, |editor, _| editor.scrollbar.geometry.unwrap());
            let content = editor.read_with(cx, |editor, _| {
                editor
                    .with_layout(|lay| lay.layout.content_height() * lay.env().zoom)
                    .unwrap()
            });
            let share = viewport.size.height / content;
            assert!(
                (geometry.thumb.size.height / geometry.track.size.height - share).abs() < 0.01
                    || geometry.thumb.size.height == px(28.),
                "{what}"
            );
            let grab = geometry.thumb.center();
            let travel = geometry.track.size.height - geometry.thumb.size.height;
            cx.simulate_mouse_down(grab, MouseButton::Left, Modifiers::none());
            cx.simulate_mouse_move(
                grab + point(px(0.), travel + px(40.)),
                MouseButton::Left,
                Modifiers::none(),
            );
            cx.simulate_mouse_up(
                grab + point(px(0.), travel + px(40.)),
                MouseButton::Left,
                Modifiers::none(),
            );
            assert!(
                close(scroll(cx), content - viewport.size.height),
                "{what}: {:?} of {content:?}",
                scroll(cx)
            );
            editor.read_with(cx, |editor, _| {
                let last = editor.document().block_count() - 1;
                let end = bounds_of(editor, last, 0);
                assert!(end.bottom() <= viewport.bottom() + px(0.5), "{what}");
                // (The last sheet ends with paper that has no text on it,
                // and may be taller than the window.)
                if !page_view {
                    assert!(end.top() >= viewport.top(), "{what}");
                }
            });

            // Page down and page up move the caret by a page of rows (less
            // the one row that stays for the eye) and keep it where it was
            // on screen.
            editor.update(cx, |editor, cx| {
                editor.edit(|state| state.set_caret(Position::new(6, 0)), cx);
                editor.scroll_to_caret(Autoscroll::Center, cx);
            });
            cx.run_until_parked();
            let before = editor.read_with(cx, |editor, _| {
                (
                    editor.caret_bounds().unwrap().top(),
                    editor.visual_row(editor.selection().head).unwrap(),
                )
            });
            cx.simulate_keystrokes("pagedown");
            cx.run_until_parked();
            editor.read_with(cx, |editor, _| {
                let head = editor.selection().head;
                let page = rows as isize - 1;
                let expected = editor
                    .move_by_visual_rows(Position::new(6, 0), page, None)
                    .unwrap();
                assert_eq!(head, expected.position, "{what}");
                assert_eq!(expected.rows_moved, page, "{what}");
                let after = editor.caret_bounds().unwrap().top();
                assert!((after - before.0).abs() < px(1.), "{what}: {after:?}");
            });
            cx.simulate_keystrokes("pageup");
            cx.run_until_parked();
            editor.read_with(cx, |editor, _| {
                assert_eq!(
                    editor.visual_row(editor.selection().head).unwrap(),
                    before.1,
                    "{what}"
                );
            });
            // Rows up and down keep their column in the text, not on the
            // screen.
            cx.simulate_keystrokes("cmd-up down down right right right down down");
            let column = editor.read_with(cx, |editor, _| editor.selection().head);
            assert_eq!(column.block, 2, "{what}");
            editor.update(cx, |editor, cx| {
                let goal = editor.column_x(column).unwrap();
                let step = editor.move_by_visual_rows(column, 2, Some(goal)).unwrap();
                // The goal is a place in the text column, in points.
                assert_eq!(step.goal_x, goal);
                assert!(close(
                    editor.position_bounds(step.position).unwrap().left(),
                    editor.position_bounds(column).unwrap().left()
                ));
                editor.edit(|state| state.set_caret(Position::ZERO), cx);
                editor.scroll_to_caret(Autoscroll::Fit, cx);
            });
            cx.run_until_parked();
            // The first line is on screen again.
            editor.read_with(cx, |editor, _| {
                let caret = editor.caret_bounds().unwrap();
                assert!(caret.top() >= viewport.top() && caret.bottom() <= viewport.bottom());
            });
        }
    }
}

#[gpui::test]
fn a_zoom_keeps_the_place(cx: &mut TestAppContext) {
    setup(cx);
    for page_view in [false, true] {
        let (editor, cx) = open_wide(sample(page_view), cx);
        let caret_y = |cx: &mut VisualTestContext| {
            editor.read_with(cx, |editor, _| {
                editor.caret_bounds().unwrap().top() - editor.viewport_bounds().unwrap().top()
            })
        };
        // The caret somewhere in the text and in the window.
        editor.update(cx, |editor, cx| {
            editor.edit(|state| state.set_caret(Position::new(60, 20)), cx);
            editor.scroll_to_caret(Autoscroll::Center, cx);
        });
        cx.run_until_parked();
        let before = caret_y(cx);
        assert!(before > px(200.));
        // From the keyboard: the line the caret is on stays where it is.
        for zoom in [
            Zoom::Percent(150),
            Zoom::Percent(67),
            Zoom::Percent(300),
            Zoom::FitWidth,
            Zoom::Percent(100),
        ] {
            zoom_to(&editor, zoom, cx);
            let after = caret_y(cx);
            assert!(
                (after - before).abs() < px(1.),
                "page view {page_view}, {zoom:?}: the caret's line moved from {before:?} to {after:?}"
            );
        }
        editor.update(cx, |editor, cx| {
            editor.zoom_in(cx);
            assert_eq!(editor.zoom(), Zoom::Percent(110));
        });
        cx.run_until_parked();
        assert!((caret_y(cx) - before).abs() < px(1.));
        editor.update(cx, |editor, cx| {
            editor.zoom_out(cx);
            editor.zoom_out(cx);
            assert_eq!(editor.zoom(), Zoom::Percent(90));
            editor.set_zoom(Zoom::Percent(100), cx);
        });
        cx.run_until_parked();
        assert!((caret_y(cx) - before).abs() < px(1.));

        // With the caret out of sight, the text at the top stays.
        editor.update(cx, |editor, cx| editor.scroll_by_pages(3., cx));
        cx.run_until_parked();
        let top = editor.read_with(cx, |editor, _| editor.visible_range().unwrap().start);
        zoom_to(&editor, Zoom::Percent(150), cx);
        if page_view {
            // (In flow view a larger zoom reflows the paragraph the top
            // is in; it is still that paragraph.)
            let after = editor.read_with(cx, |editor, _| editor.visible_range().unwrap().start);
            assert_eq!(after, top, "page view {page_view}");
        } else {
            let after = editor.read_with(cx, |editor, _| editor.visible_range().unwrap().start);
            assert_eq!(after.block, top.block);
        }
        zoom_to(&editor, Zoom::Percent(100), cx);

        // With `cmd` and the wheel: what is under the pointer stays under
        // it, whatever it is.
        let viewport = editor.read_with(cx, |editor, _| editor.viewport_bounds().unwrap());
        for at in [
            viewport.center(),
            viewport.center() + point(px(-150.), px(-200.)),
            viewport.center() + point(px(100.), px(250.)),
        ] {
            let under = |cx: &mut VisualTestContext| {
                editor.read_with(cx, |editor, _| {
                    let position = editor.position_at_point(at).unwrap();
                    let row = editor.visual_row(position).unwrap();
                    let top = editor.position_bounds(row.start).unwrap().top();
                    (row.start, at.y - top)
                })
            };
            let before = under(cx);
            for wheel in [120., 120., -90., 300., -450.] {
                let zoom_before = editor.read_with(cx, |editor, _| editor.zoom_percent());
                cx.simulate_event(ScrollWheelEvent {
                    position: at,
                    delta: ScrollDelta::Pixels(point(px(0.), px(wheel))),
                    modifiers: Modifiers::command(),
                    touch_phase: TouchPhase::Moved,
                });
                cx.run_until_parked();
                let zoom_after = editor.read_with(cx, |editor, _| editor.zoom_percent());
                // Towards the top of the wheel is in.
                assert_eq!(zoom_after > zoom_before, wheel > 0., "{wheel}");
                let after = under(cx);
                if page_view {
                    assert_eq!(after.0, before.0, "page view, wheel {wheel} at {at:?}");
                    // The same share of the row's height from its top.
                    let scale = zoom_after / 100.;
                    assert!(
                        (after.1 / scale - before.1).abs() < px(1.),
                        "wheel {wheel} at {at:?}: {after:?} against {before:?}"
                    );
                } else {
                    // Flow view may reflow under the pointer: the block
                    // stays.
                    assert_eq!(after.0.block, before.0.block, "flow, wheel {wheel}");
                }
            }
            zoom_to(&editor, Zoom::Percent(100), cx);
        }
        // The wheel alone still scrolls; a pinch that ends at the limits
        // stays inside them.
        editor.update(cx, |editor, cx| {
            editor.zoom_by(100., viewport.center(), cx);
            assert_eq!(editor.zoom(), Zoom::Percent(400));
            editor.zoom_by(0.0001, viewport.center(), cx);
            assert_eq!(editor.zoom(), Zoom::Percent(50));
            // Many steps of less than a percent add up.
            editor.set_zoom(Zoom::Percent(100), cx);
            for _ in 0..40 {
                editor.zoom_by(1.004, viewport.center(), cx);
            }
            let reached = editor.zoom_percent();
            assert!((reached - 100. * 1.004f32.powi(40)).abs() < 1., "{reached}");
        });
    }
}

#[gpui::test]
fn a_sheet_wider_than_its_room_scrolls_sideways(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(sample(true), EditorConfig::full(), cx);
    cx.simulate_resize(size(px(700.), px(800.)));
    cx.run_until_parked();
    let viewport = editor.read_with(cx, |editor, _| editor.viewport_bounds().unwrap());
    let sheet =
        |cx: &mut VisualTestContext| editor.read_with(cx, |editor, _| editor.painted_sheets[0]);
    // At 100 % an A4 sheet (595 wide) fits: centred, nothing to scroll.
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.max_horizontal_scroll(), px(0.));
        let sheet = editor.painted_sheets[0];
        assert!(close(sheet.size.width, px(595.276)));
        assert!((sheet.center().x - viewport.center().x).abs() <= px(0.5));
    });
    // The wheel sideways does nothing then.
    cx.simulate_event(ScrollWheelEvent {
        position: viewport.center(),
        delta: ScrollDelta::Pixels(point(px(-80.), px(0.))),
        modifiers: Modifiers::none(),
        touch_phase: TouchPhase::Moved,
    });
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.horizontal_scroll_offset(), px(0.));
    });

    // At 200 % the sheet is 1190 wide in a room of 700.
    zoom_to(&editor, Zoom::Percent(200), cx);
    let max = editor.read_with(cx, |editor, _| editor.max_horizontal_scroll());
    assert!(close(max, px(595.276 * 2. + 32. - 700.)), "{max:?}");
    // It rests with the start of the lines in view: the text column is
    // 902 wide, wider than the room, so its left edge is at the edge of
    // the window, less the desk that is kept.
    let text = editor.read_with(cx, |editor, _| bounds_of(editor, 2, 0));
    assert!(close(text.left(), viewport.left() + px(16.)), "{text:?}");
    let resting = editor.read_with(cx, |editor, _| editor.horizontal_scroll_offset());
    assert!(close(resting, px(144.)), "{resting:?}");

    // Two fingers sideways (or shift with the wheel) move the sheet, to
    // its edges and not further.
    let down = editor.read_with(cx, |editor, _| editor.scroll_offset());
    cx.simulate_event(ScrollWheelEvent {
        position: viewport.center(),
        delta: ScrollDelta::Pixels(point(px(-100.), px(0.))),
        modifiers: Modifiers::none(),
        touch_phase: TouchPhase::Moved,
    });
    editor.read_with(cx, |editor, _| {
        assert!(close(editor.horizontal_scroll_offset(), resting + px(100.)));
        assert!(close(
            bounds_of(editor, 2, 0).left(),
            text.left() - px(100.)
        ));
        // Up and down nothing moved.
        assert_eq!(editor.scroll_offset(), down);
    });
    cx.simulate_event(ScrollWheelEvent {
        position: viewport.center(),
        delta: ScrollDelta::Pixels(point(px(-5000.), px(0.))),
        modifiers: Modifiers::none(),
        touch_phase: TouchPhase::Moved,
    });
    editor.read_with(cx, |editor, _| {
        assert!(close(editor.horizontal_scroll_offset(), max));
        // The right edge of the sheet, and the desk beside it.
        assert!(close(
            editor.painted_sheets[0].right(),
            viewport.right() - px(16.)
        ));
    });
    cx.simulate_event(ScrollWheelEvent {
        position: viewport.center(),
        delta: ScrollDelta::Pixels(point(px(5000.), px(0.))),
        modifiers: Modifiers::none(),
        touch_phase: TouchPhase::Moved,
    });
    assert!(close(sheet(cx).left(), viewport.left() + px(16.)));

    // A wheel that only goes up and down goes sideways with shift.
    let from = editor.read_with(cx, |editor, _| editor.horizontal_scroll_offset());
    cx.simulate_event(ScrollWheelEvent {
        position: viewport.center(),
        delta: ScrollDelta::Pixels(point(px(0.), px(-150.))),
        modifiers: Modifiers::shift(),
        touch_phase: TouchPhase::Moved,
    });
    editor.read_with(cx, |editor, _| {
        assert!(close(editor.horizontal_scroll_offset(), from + px(150.)));
        assert_eq!(editor.scroll_offset(), down);
    });
    // Two fingers that go down and a little sideways scroll the text and
    // leave the sheet where it is; a move across both does both.
    cx.simulate_event(ScrollWheelEvent {
        position: viewport.center(),
        delta: ScrollDelta::Pixels(point(px(-9.), px(-60.))),
        modifiers: Modifiers::none(),
        touch_phase: TouchPhase::Moved,
    });
    editor.read_with(cx, |editor, _| {
        assert!(close(editor.horizontal_scroll_offset(), from + px(150.)));
        assert!(close(editor.scroll_offset(), down + px(60.)));
    });
    cx.simulate_event(ScrollWheelEvent {
        position: viewport.center(),
        delta: ScrollDelta::Pixels(point(px(40.), px(60.))),
        modifiers: Modifiers::none(),
        touch_phase: TouchPhase::Moved,
    });
    editor.read_with(cx, |editor, _| {
        assert!(close(editor.horizontal_scroll_offset(), from + px(110.)));
        assert!(close(editor.scroll_offset(), down));
    });
    cx.simulate_event(ScrollWheelEvent {
        position: viewport.center(),
        delta: ScrollDelta::Pixels(point(px(5000.), px(0.))),
        modifiers: Modifiers::none(),
        touch_phase: TouchPhase::Moved,
    });

    // Hit testing knows where the sheet is: a click finds the character
    // under it wherever the sheet was scrolled to.
    for offset in [px(0.), px(200.), max] {
        editor.update(cx, |editor, cx| {
            editor.scroll_sideways_by_pixels(px(-5000.), cx);
            editor.scroll_sideways_by_pixels(offset, cx);
        });
        cx.run_until_parked();
        for (block, at) in [(2, 12), (1, 3)] {
            let target = editor.read_with(cx, |editor, _| bounds_of(editor, block, at).center());
            if target.x <= viewport.left() || target.x >= viewport.right() - px(14.) {
                continue;
            }
            click(cx, target + point(px(1.), px(0.)), 1, Modifiers::none());
            editor.read_with(cx, |editor, _| {
                assert_eq!(
                    editor.selection(),
                    Selection::caret(Position::new(block, at)),
                    "scrolled {offset:?}"
                );
            });
        }
    }

    // Whoever types never types out of sight: the view follows the caret
    // to the end of a line and back to the start of the next.
    let inside = |cx: &mut VisualTestContext, what: &str| {
        editor.read_with(cx, |editor, _| {
            let caret = editor.caret_bounds().unwrap();
            assert!(
                caret.left() >= viewport.left() && caret.right() <= viewport.right(),
                "{what}: the caret is at {caret:?} in {viewport:?}"
            );
        });
    };
    editor.update(cx, |editor, cx| {
        editor.scroll_sideways_by_pixels(px(-5000.), cx);
        editor.edit(|state| state.set_caret(Position::new(1, 22)), cx);
    });
    cx.run_until_parked();
    let mut far = px(0.);
    for _ in 0..12 {
        cx.simulate_input("more words ");
        cx.run_until_parked();
        inside(cx, "typing along a line");
        far = far.max(editor.read_with(cx, |editor, _| editor.horizontal_scroll_offset()));
    }
    // A full line ends 898 pixels from where it starts, in a window of
    // 700.
    assert!(far > px(350.), "the view followed the caret: {far:?}");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    inside(cx, "a new paragraph");
    assert!(editor.read_with(cx, |editor, _| editor.horizontal_scroll_offset()) < px(200.));
    cx.simulate_keystrokes("up cmd-right");
    cx.run_until_parked();
    inside(cx, "the end of the line");
    cx.simulate_keystrokes("cmd-left");
    cx.run_until_parked();
    inside(cx, "the start of the line");

    // Fit width ends it: the sheet between the edges.
    zoom_to(&editor, Zoom::FitWidth, cx);
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.max_horizontal_scroll(), px(0.));
        let sheet = editor.painted_sheets[0];
        assert!(close(sheet.left(), viewport.left() + px(16.)), "{sheet:?}");
        assert!(
            close(sheet.right(), viewport.right() - px(16.)),
            "{sheet:?}"
        );
        assert!((editor.zoom_percent() - 668. / 595.276 * 100.).abs() < 0.1);
    });
    // Fit width up to a limit: the same in a window this narrow, and no
    // larger than the limit in a wide one.
    zoom_to(&editor, Zoom::FitWidthUpTo(177), cx);
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.max_horizontal_scroll(), px(0.));
        assert!((editor.zoom_percent() - 668. / 595.276 * 100.).abs() < 0.1);
    });
    cx.simulate_resize(size(px(2000.), px(800.)));
    cx.run_until_parked();
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.zoom(), Zoom::FitWidthUpTo(177));
        assert_eq!(editor.zoom_percent(), 177.);
        let sheet = editor.painted_sheets[0];
        assert!(close(sheet.size.width, px(595.276 * 1.77)), "{sheet:?}");
        assert!((sheet.center().x - px(1000.)).abs() <= px(0.5));
        assert_eq!(editor.max_horizontal_scroll(), px(0.));
    });
    cx.simulate_resize(size(px(700.), px(800.)));
    cx.run_until_parked();
    // Fit page: a whole sheet, with the desk above and below it.
    zoom_to(&editor, Zoom::FitPage, cx);
    editor.read_with(cx, |editor, _| {
        let sheet = editor.painted_sheets[0];
        assert!(sheet.top() > viewport.top() && sheet.bottom() < viewport.bottom());
        assert!(sheet.size.height > viewport.size.height * 0.9, "{sheet:?}");
        assert!((sheet.center().x - viewport.center().x).abs() <= px(0.5));
    });

    // Flow view never scrolls sideways, at any zoom.
    editor.update(cx, |editor, cx| editor.set_page_view(false, cx));
    cx.run_until_parked();
    for percent in [50, 100, 250, 400] {
        zoom_to(&editor, Zoom::Percent(percent), cx);
        cx.simulate_event(ScrollWheelEvent {
            position: viewport.center(),
            delta: ScrollDelta::Pixels(point(px(-300.), px(0.))),
            modifiers: Modifiers::none(),
            touch_phase: TouchPhase::Moved,
        });
        editor.read_with(cx, |editor, _| {
            assert_eq!(editor.max_horizontal_scroll(), px(0.), "{percent} %");
            assert_eq!(editor.horizontal_scroll_offset(), px(0.), "{percent} %");
            let end = editor.document().blocks()[2].len();
            for offset in (0..end).step_by(11) {
                let at = bounds_of(editor, 2, offset);
                assert!(at.left() >= viewport.left() && at.left() <= viewport.right());
            }
        });
    }
}

#[gpui::test]
fn each_view_has_its_zoom_and_a_zoom_is_not_an_edit(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open_wide(sample(false), cx);
    let events = Rc::new(RefCell::new(Vec::new()));
    cx.update(|_, cx| {
        let events = events.clone();
        cx.subscribe(&editor, move |_, event: &EditorEvent, _| {
            events.borrow_mut().push(event.clone());
        })
        .detach();
    });
    cx.simulate_input("typed");
    let stored = editor.read_with(cx, |editor, _| editor.document().to_json());
    let depth = editor.read_with(cx, |editor, _| editor.state().undo_depth());
    events.borrow_mut().clear();

    zoom_to(&editor, Zoom::Percent(150), cx);
    zoom_to(&editor, Zoom::Percent(150), cx);
    editor.update(cx, |editor, cx| {
        editor.zoom_in(cx);
        editor.zoom_out(cx);
    });
    cx.run_until_parked();
    // Three changes of zoom, three events, and nothing else: no edit, no
    // change of the settings, nothing for undo.
    assert_eq!(
        *events.borrow(),
        vec![
            EditorEvent::ZoomChanged,
            EditorEvent::ZoomChanged,
            EditorEvent::ZoomChanged
        ]
    );
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().to_json(), stored);
        assert_eq!(editor.state().undo_depth(), depth);
        assert_eq!(editor.document().settings().typography.base_size, 17.);
        assert_eq!(editor.zoom(), Zoom::Percent(150));
    });
    // Undo takes the typing out, not the zoom.
    cx.simulate_keystrokes("cmd-z");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().blocks()[0].text(), "A heading");
        assert_eq!(editor.zoom(), Zoom::Percent(150));
    });

    // Page view has a zoom of its own; each comes back with its view.
    events.borrow_mut().clear();
    editor.update(cx, |editor, cx| {
        assert_eq!(editor.zoom_of(true), Zoom::Percent(100));
        editor.set_page_view(true, cx);
        assert_eq!(editor.zoom(), Zoom::Percent(100));
        editor.set_zoom(Zoom::FitWidth, cx);
    });
    cx.run_until_parked();
    editor.update(cx, |editor, cx| {
        assert_eq!(editor.zoom(), Zoom::FitWidth);
        assert_eq!(editor.zoom_of(false), Zoom::Percent(150));
        // What a fit comes to is known after a frame.
        assert!((editor.zoom_percent() - (1600. - 32.) / 595.276 * 100.).abs() < 0.1);
        editor.set_page_view(false, cx);
        assert_eq!(editor.zoom(), Zoom::Percent(150));
        assert_eq!(editor.zoom_percent(), 150.);
    });
    cx.run_until_parked();
    // A zoom that follows the window says so when the window changes.
    editor.update(cx, |editor, cx| editor.set_page_view(true, cx));
    cx.run_until_parked();
    events.borrow_mut().clear();
    cx.simulate_resize(size(px(1000.), px(900.)));
    cx.run_until_parked();
    assert_eq!(*events.borrow(), vec![EditorEvent::ZoomChanged]);
    editor.read_with(cx, |editor, _| {
        assert!((editor.zoom_percent() - (1000. - 32.) / 595.276 * 100.).abs() < 0.1);
        // Steps go on from what is shown: 162.6 % in, 175; out, 150.
        assert_eq!(crate::step_above(editor.zoom_percent()), Some(175));
        assert_eq!(crate::step_below(editor.zoom_percent()), Some(150));
    });
    // A fixed zoom does not follow the window.
    zoom_to(&editor, Zoom::Percent(125), cx);
    events.borrow_mut().clear();
    cx.simulate_resize(size(px(1300.), px(900.)));
    cx.run_until_parked();
    assert!(events.borrow().is_empty(), "{:?}", events.borrow());

    // Restoring what was stored moves nothing and tells nobody.
    editor.update(cx, |editor, cx| {
        editor.set_view_zooms(Zoom::Percent(80), Zoom::Percent(9999), cx);
        assert_eq!(editor.zoom_of(false), Zoom::Percent(80));
        assert_eq!(editor.zoom_of(true), Zoom::Percent(crate::ZOOM_MAX));
    });
    cx.run_until_parked();
    assert!(events.borrow().is_empty());
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().to_json(), {
            // (The settings changed with the view, nothing else.)
            editor.document().to_json()
        })
    });
}

#[gpui::test]
fn a_compact_editor_does_not_zoom(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(
        Document::from_blocks(vec![Block::paragraph("a comment being written")]),
        EditorConfig::compact(CompactOptions::default()),
        cx,
    );
    let before = editor.read_with(cx, |editor, _| bounds_of(editor, 0, 10));
    editor.update(cx, |editor, cx| {
        editor.set_zoom(Zoom::Percent(200), cx);
        editor.zoom_in(cx);
        editor.zoom_by(3., before.center(), cx);
        assert_eq!(editor.zoom(), Zoom::Percent(100));
    });
    cx.simulate_event(ScrollWheelEvent {
        position: before.center(),
        delta: ScrollDelta::Pixels(point(px(0.), px(200.))),
        modifiers: Modifiers::command(),
        touch_phase: TouchPhase::Moved,
    });
    cx.run_until_parked();
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.zoom(), Zoom::Percent(100));
        assert_eq!(bounds_of(editor, 0, 10), before);
    });
}

#[gpui::test]
fn zooming_a_book_lays_nothing_out_again(cx: &mut TestAppContext) {
    setup(cx);
    let sentence = "The quick brown fox jumps over the lazy dog and keeps on running. ";
    let blocks: Vec<Block> = (0..10_000)
        .map(|index| Block::paragraph(sentence.repeat(1 + index % 4).trim_end()))
        .collect();
    for page_view in [true, false] {
        let (editor, cx) = open_wide(
            with_view(Document::from_blocks(blocks.clone()), page_view),
            cx,
        );
        editor.update(cx, |editor, cx| {
            editor.edit(|state| state.set_caret(Position::new(5_000, 3)), cx);
            editor.scroll_to_caret(Autoscroll::Center, cx);
        });
        cx.run_until_parked();
        editor.update(cx, |editor, cx| editor.finish_layout(cx));
        cx.run_until_parked();
        let pages = editor.read_with(cx, |editor, _| editor.page_count());
        let measured = |cx: &mut VisualTestContext| {
            editor.read_with(cx, |editor, _| editor.layout.borrow().unmeasured())
        };
        assert_eq!(measured(cx), 0);
        let shaped = SHAPED_BYTES.with(|bytes| bytes.get());
        // A zoom at which the column still has its full width: no block
        // loses its measure, and only what comes into view is shaped (a
        // smaller zoom shows more of the book).
        for percent in [150, 125, 200, 100] {
            zoom_to(&editor, Zoom::Percent(percent), cx);
            assert_eq!(measured(cx), 0, "page view {page_view}, {percent} %");
            editor.read_with(cx, |editor, _| {
                assert_eq!(editor.page_count(), pages);
                assert!(!editor.layout_pending());
                // The caret did not leave the screen.
                let viewport = editor.viewport_bounds().unwrap();
                let caret = editor.caret_bounds().unwrap();
                assert!(caret.top() >= viewport.top() && caret.bottom() <= viewport.bottom());
            });
        }
        let again = SHAPED_BYTES.with(|bytes| bytes.get()) - shaped;
        assert!(
            again < 40_000,
            "page view {page_view}: {again} bytes of a book of 1.6 MB were shaped again"
        );
    }
}
