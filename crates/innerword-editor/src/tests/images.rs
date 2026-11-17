//! Images: the box, the picture and its placeholder, selecting, resizing
//! and moving with the pointer and the keys, the ways in (chosen files,
//! paste, drop), the cache of decoded pictures, the picture on paper.
//!
//! Every test uses [`FakeImages`]: pictures in memory, no library.

use std::{cell::RefCell, path::PathBuf, rc::Rc, sync::Arc};

use gpui::{
    Bounds, ClipboardItem, Context, Entity, IntoElement, Modifiers, MouseButton, MouseDownEvent,
    MouseUpEvent, Pixels, Point, Render, TestAppContext, VisualTestContext, Window, div, point,
    prelude::*, px, size,
};
use innerword_core::{
    Alignment, Block, BlockKind, Document, DocumentSettings, EditorState, Image, ImageWidth,
    Margins, PageSetup, PaperSize, Position, Selection,
};
use innerword_print::Item;
use innerword_ui::theme::ActiveTheme;

use super::slice::{open, setup};
use crate::{
    CompactOptions, Editor, EditorConfig, EditorEvent, FakeImages, ImageStatus, NewImageSource,
    Zoom,
    actions::{ImageOriginalSize, ImageWidth50, InsertImage},
    element::PaintedImage,
    layout::block::ImageBox,
};

/// The text area of A4 with one-inch margins, in points.
const A4_TEXT: (f32, f32) = (451.276, 697.89);

fn paragraph(index: usize) -> Block {
    Block::paragraph(&format!(
        "Paragraph number {index}, with a few words in it."
    ))
}

/// The block of `image` at `thousandths` of the column.
fn picture(image: &Image, thousandths: u16, alignment: Alignment) -> Block {
    let mut image = image.clone();
    image.width = ImageWidth::new(thousandths);
    Block::image(image).with_alignment(alignment)
}

fn with_view(blocks: Vec<Block>, page_view: bool) -> Document {
    Document::from_blocks(blocks).with_settings(DocumentSettings {
        page_view,
        ..DocumentSettings::default()
    })
}

fn paged(blocks: Vec<Block>) -> Document {
    with_view(blocks, true)
}

fn flow(blocks: Vec<Block>) -> Document {
    with_view(blocks, false)
}

/// Opens `document` in a window of 1200 by 900 with `images` as the
/// provider, and lets the pictures on screen arrive.
fn open_with<'a>(
    document: Document,
    images: &Rc<FakeImages>,
    cx: &'a mut TestAppContext,
) -> (Entity<Editor>, &'a mut VisualTestContext) {
    let (editor, cx) = open(document, EditorConfig::full(), cx);
    cx.simulate_resize(size(px(1200.), px(900.)));
    editor.update(cx, |editor, cx| {
        editor.set_image_provider(Some(images.clone()), cx);
    });
    cx.run_until_parked();
    (editor, cx)
}

/// The box of the picture of block `index`, in layout units, and the
/// width of the column it stands in.
fn boxed(editor: &Editor, index: usize) -> (ImageBox, f32) {
    editor
        .with_layout(|lay| {
            let column = f32::from(lay.env().column_width);
            let placed = lay.placed(index).expect("a block");
            (placed.layout.image.expect("an image block"), column)
        })
        .expect("laid out")
}

/// `(sheet, y from the top of that sheet's text area)` of the top of block
/// `index`, in layout units. Page view.
fn on_sheet(editor: &Editor, index: usize) -> (usize, f32) {
    editor
        .with_layout(|lay| {
            let (geometry, pad_top) = {
                let env = lay.env();
                (env.page.expect("page view"), env.pad_top)
            };
            let placed = lay.placed(index).expect("a block");
            let page = lay.page_at(placed.top);
            let text_top = geometry.sheet_top(page, pad_top) + geometry.margin_top;
            (page, f32::from(placed.top - text_top))
        })
        .expect("laid out")
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.02
}

fn near(a: Pixels, b: Pixels) -> bool {
    (a - b).abs() <= px(0.5)
}

fn same_place(a: Bounds<Pixels>, b: Bounds<Pixels>) -> bool {
    near(a.left(), b.left())
        && near(a.top(), b.top())
        && near(a.right(), b.right())
        && near(a.bottom(), b.bottom())
}

fn painted(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> Vec<PaintedImage> {
    editor.read_with(cx, |editor, _| editor.painted_images.clone())
}

/// What the last frame painted for the image block `index`.
fn painted_at(editor: &Entity<Editor>, index: usize, cx: &mut VisualTestContext) -> PaintedImage {
    painted(editor, cx)
        .into_iter()
        .find(|image| image.block == index)
        .unwrap_or_else(|| panic!("block {index} painted no image"))
}

fn selection(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> Selection {
    editor.read_with(cx, |editor, _| editor.selection())
}

fn caret(block: usize, offset: usize) -> Selection {
    Selection::caret(Position::new(block, offset))
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

/// The window bounds of the picture of block `index`.
fn picture_bounds(
    editor: &Entity<Editor>,
    index: usize,
    cx: &mut VisualTestContext,
) -> Bounds<Pixels> {
    editor.read_with(cx, |editor, _| {
        editor
            .image_bounds(index)
            .expect("an image on a laid out page")
    })
}

/// The kinds of the blocks, in a word each.
fn kinds(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> Vec<&'static str> {
    editor.read_with(cx, |editor, _| {
        editor
            .document()
            .blocks()
            .iter()
            .map(|block| block.kind().name())
            .collect()
    })
}

fn width_of(editor: &Entity<Editor>, index: usize, cx: &mut VisualTestContext) -> u16 {
    editor.read_with(cx, |editor, _| {
        editor.document().blocks()[index]
            .as_image()
            .expect("an image block")
            .width
            .thousandths()
    })
}

// ---------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------

#[gpui::test]
fn the_box_is_the_images_share_of_the_column_at_its_aspect(cx: &mut TestAppContext) {
    setup(cx);
    let images = FakeImages::new();
    let photo = images.add_png(800, 600);
    let tall = images.add_png(300, 900);
    let (editor, cx) = open_with(
        flow(vec![
            paragraph(0),
            picture(&photo, 500, Alignment::Left),
            picture(&photo, 500, Alignment::Center),
            picture(&photo, 250, Alignment::Right),
            picture(&photo, 1000, Alignment::Justify),
            picture(&tall, 100, Alignment::Center),
            paragraph(1),
        ]),
        &images,
        cx,
    );
    editor.read_with(cx, |editor, _| {
        let (left, column) = boxed(editor, 1);
        assert!(close(column, 700.), "{column}");
        // Half the column wide, three quarters of that high; at the left.
        assert!(close(f32::from(left.size.width), 350.), "{left:?}");
        assert!(close(f32::from(left.size.height), 262.5), "{left:?}");
        assert!(close(f32::from(left.x), 0.));
        assert_eq!(left.image, photo.id);
        // Centred: the same room on both sides.
        let (centred, _) = boxed(editor, 2);
        assert!(close(f32::from(centred.x), 175.), "{centred:?}");
        assert_eq!(centred.size, left.size);
        // At the right: its right edge is the column's.
        let (right, _) = boxed(editor, 3);
        assert!(close(f32::from(right.size.width), 175.));
        assert!(
            close(f32::from(right.x + right.size.width), 700.),
            "{right:?}"
        );
        // The whole column, never more; a justified image stands left.
        let (full, _) = boxed(editor, 4);
        assert!(close(f32::from(full.size.width), 700.), "{full:?}");
        assert!(close(f32::from(full.size.height), 525.));
        assert!(close(f32::from(full.x), 0.));
        // A tenth of the column wide and three times as high.
        let (narrow, _) = boxed(editor, 5);
        assert!(close(f32::from(narrow.size.width), 70.), "{narrow:?}");
        assert!(close(f32::from(narrow.size.height), 210.));

        // The row of a picture is as high as the picture, in whole points,
        // and the text goes on under it after the paragraph spacing.
        editor
            .with_layout(|lay| {
                let image = lay.placed(1).unwrap();
                assert_eq!(image.layout.rows.len(), 1);
                assert!(image.layout.void);
                assert_eq!(f32::from(image.layout.line_height), 263.);
                let next = lay.placed(2).unwrap();
                let spacing = editor
                    .document()
                    .settings()
                    .typography
                    .paragraph_spacing
                    .round();
                assert!(close(f32::from(next.top - image.top), 263. + spacing));
            })
            .unwrap();
    });
    // The window bounds are the box in the window, and what is painted.
    for index in 1..=5 {
        let bounds = picture_bounds(&editor, index, cx);
        let (image, _) = editor.read_with(cx, |editor, _| boxed(editor, index));
        assert!(near(bounds.size.width, image.size.width), "{index}");
        assert!(near(bounds.size.height, image.size.height), "{index}");
    }
    let bounds = picture_bounds(&editor, 1, cx);
    assert!(same_place(painted_at(&editor, 1, cx).bounds, bounds));
    // Not an image, no bounds.
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.image_bounds(0), None);
        assert_eq!(editor.image_bounds(99), None);
    });
}

/// A document on `paper` with one-inch margins, in page view.
fn on_paper(blocks: Vec<Block>, paper: PaperSize, margins: Margins) -> Document {
    Document::from_blocks(blocks).with_settings(DocumentSettings {
        page_view: true,
        page: PageSetup {
            paper,
            margins,
            ..PageSetup::default()
        },
        ..DocumentSettings::default()
    })
}

#[gpui::test]
fn an_image_that_does_not_fit_goes_to_the_next_sheet(cx: &mut TestAppContext) {
    setup(cx);
    let images = FakeImages::new();
    let photo = images.add_png(800, 600);
    // Twelve short paragraphs, then a picture 338 points high: the rest of
    // the first sheet does not hold it.
    let mut blocks: Vec<Block> = (0..12).map(paragraph).collect();
    blocks.push(picture(&photo, 1000, Alignment::Center));
    blocks.push(paragraph(12));
    let (editor, cx) = open_with(paged(blocks), &images, cx);
    editor.update(cx, |editor, cx| editor.finish_layout(cx));
    editor.read_with(cx, |editor, _| {
        let (image, column) = boxed(editor, 12);
        assert!(close(column, A4_TEXT.0), "{column}");
        assert!(close(f32::from(image.size.width), A4_TEXT.0));
        assert!(close(f32::from(image.size.height), A4_TEXT.0 * 0.75));
        assert_eq!(on_sheet(editor, 11).0, 0);
        // At the top of the text area of the second sheet, whole.
        let (sheet, y) = on_sheet(editor, 12);
        assert_eq!(sheet, 1);
        assert!(close(y, 0.), "{y}");
        assert_eq!(on_sheet(editor, 13).0, 1);
        assert_eq!(editor.page_count(), Some(2));
    });

    // With fewer paragraphs before it, it stays on the first sheet.
    let mut blocks: Vec<Block> = (0..4).map(paragraph).collect();
    blocks.push(picture(&photo, 1000, Alignment::Center));
    editor.update(cx, |editor, cx| {
        editor.set_document(paged(blocks), cx);
        editor.finish_layout(cx);
        assert_eq!(on_sheet(editor, 4).0, 0);
        assert_eq!(editor.page_count(), Some(1));
    });
}

#[gpui::test]
fn an_image_is_never_higher_than_the_text_area_of_a_page(cx: &mut TestAppContext) {
    setup(cx);
    let images = FakeImages::new();
    // A phone screenshot: more than twice as high as it is wide.
    let phone = images.add_png(1170, 2532);
    let aspect = 2532. / 1170.;
    let inch = Margins {
        top: 72.,
        right: 72.,
        bottom: 72.,
        left: 72.,
    };
    // (paper, margins, the text area)
    let cases = [
        (PaperSize::A4, inch, (451.276, 697.89)),
        (PaperSize::UsLetter, inch, (468., 648.)),
        // Margins that grew after the width was stored.
        (
            PaperSize::A4,
            Margins {
                top: 150.,
                bottom: 150.,
                ..inch
            },
            (451.276, 541.89),
        ),
    ];
    let (editor, cx) = open_with(paged(vec![paragraph(0)]), &images, cx);
    for (paper, margins, (text_width, text_height)) in cases {
        // The whole column: a width that was stored for another page, or
        // by a version that did not know the limit.
        let document = on_paper(
            vec![
                paragraph(0),
                picture(&phone, 1000, Alignment::Center),
                paragraph(1),
            ],
            paper,
            margins,
        );
        editor.update(cx, |editor, cx| {
            editor.set_document(document, cx);
            editor.finish_layout(cx);
        });
        cx.run_until_parked();
        editor.read_with(cx, |editor, _| {
            let what = format!("{paper:?} with a text area of {text_width} x {text_height}");
            let (image, column) = boxed(editor, 1);
            assert!(close(column, text_width), "{what}: {column}");
            let (width, height) = (f32::from(image.size.width), f32::from(image.size.height));
            // Not higher than the text area, and as high as its whole
            // points: nothing was lost to a rounding through a division.
            assert!(height <= text_height, "{what}: {height}");
            assert_eq!(height, text_height.floor(), "{what}");
            // Narrower by its aspect, and centred.
            assert!(close(width, height / aspect), "{what}: {width}");
            assert!(
                close(f32::from(image.x), (text_width - width) / 2.),
                "{what}"
            );
            editor
                .with_layout(|lay| {
                    let placed = lay.placed(1).unwrap();
                    // One row, which is not higher than the text area
                    // either: it fits on an empty sheet.
                    assert_eq!(placed.layout.rows.len(), 1);
                    assert!(
                        f32::from(placed.layout.line_height) <= text_height,
                        "{what}"
                    );
                    let geometry = lay.env().page.unwrap();
                    assert!(
                        placed.layout.line_height <= geometry.content_height,
                        "{what}"
                    );
                })
                .unwrap();
            // On one sheet: the second, at the top of its text area, and
            // its bottom inside that text area.
            let (sheet, y) = on_sheet(editor, 1);
            assert_eq!(sheet, 1, "{what}");
            assert!(close(y, 0.), "{what}: {y}");
            assert_eq!(on_sheet(editor, 2).0, 2, "{what}");
            assert_eq!(editor.page_count(), Some(3), "{what}");
        });
    }
}

#[gpui::test]
fn the_same_sheet_and_box_at_every_zoom(cx: &mut TestAppContext) {
    setup(cx);
    let images = FakeImages::new();
    let photo = images.add_png(800, 600);
    let phone = images.add_png(1170, 2532);
    let mut blocks = Vec::new();
    for round in 0..6 {
        blocks.extend((0..5).map(|ix| paragraph(round * 10 + ix)));
        blocks.push(picture(&photo, 300 + round as u16 * 120, Alignment::Center));
        blocks.push(picture(&phone, 1000, Alignment::Right));
    }
    let (editor, cx) = open_with(paged(blocks.clone()), &images, cx);
    cx.simulate_resize(size(px(1600.), px(1000.)));
    editor.update(cx, |editor, cx| editor.finish_layout(cx));
    cx.run_until_parked();
    let image_blocks: Vec<usize> = (0..blocks.len())
        .filter(|ix| blocks[*ix].as_image().is_some())
        .collect();
    let places = |editor: &Editor| -> Vec<(usize, usize, f32, ImageBox)> {
        image_blocks
            .iter()
            .map(|index| {
                let (sheet, y) = on_sheet(editor, *index);
                (*index, sheet, y, boxed(editor, *index).0)
            })
            .collect()
    };
    let (at_100, pages) = editor.read_with(cx, |editor, _| (places(editor), editor.page_count()));
    assert!(pages.unwrap() >= 8, "{pages:?}");
    for percent in [75, 150, 100] {
        editor.update(cx, |editor, cx| editor.set_zoom(Zoom::Percent(percent), cx));
        cx.run_until_parked();
        editor.update(cx, |editor, cx| {
            editor.finish_layout(cx);
            assert_eq!(editor.page_count(), pages, "{percent} %");
            assert_eq!(places(editor), at_100, "{percent} %");
        });
        // And in the window the picture is the box at the zoom.
        let zoom = percent as f32 / 100.;
        let (index, _, _, image) = at_100[0];
        let bounds = picture_bounds(&editor, index, cx);
        assert!(
            near(bounds.size.width, image.size.width * zoom),
            "{percent} %"
        );
        assert!(
            near(bounds.size.height, image.size.height * zoom),
            "{percent} %"
        );
    }
}

#[gpui::test]
fn the_placeholder_has_the_pictures_bounds_and_nothing_moves(cx: &mut TestAppContext) {
    setup(cx);
    let images = FakeImages::new();
    let photo = images.add_png(640, 480);
    let mut described = photo.clone();
    described.description = "The harbour at dusk".to_string();
    // The pictures are held back: the frame has placeholders.
    images.hold();
    let (editor, cx) = open_with(
        flow(vec![
            paragraph(0),
            picture(&photo, 600, Alignment::Center),
            paragraph(1),
            picture(&described, 400, Alignment::Left),
            paragraph(2),
        ]),
        &images,
        cx,
    );
    let places = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |editor, _| {
            (0..5)
                .map(|index| {
                    editor
                        .position_bounds(Position::new(index, 0))
                        .expect("laid out")
                })
                .collect::<Vec<_>>()
        })
    };
    let before = places(cx);
    let waiting = painted(&editor, cx);
    assert_eq!(waiting.len(), 2);
    assert_eq!(waiting[0].level, None);
    assert_eq!(waiting[0].label.as_deref(), Some("Image"));
    assert_eq!(waiting[1].label.as_deref(), Some("The harbour at dusk"));
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.image_status(photo.id), ImageStatus::Loading);
    });
    // One picture for the two blocks was asked for once, and is not asked
    // for again while it is on its way.
    assert_eq!(images.asked(), vec![photo.id]);
    cx.simulate_keystrokes("down");
    cx.run_until_parked();
    assert_eq!(images.asked(), vec![photo.id]);

    images.release();
    cx.run_until_parked();
    let arrived = painted(&editor, cx);
    assert_eq!(arrived.len(), 2);
    for (was, is) in waiting.iter().zip(&arrived) {
        assert!(is.level.is_some(), "{is:?}");
        assert_eq!(is.label, None);
        assert_eq!(is.bounds, was.bounds);
    }
    assert_eq!(
        places(cx),
        before,
        "something moved when the picture arrived"
    );
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.image_status(photo.id), ImageStatus::Ready);
    });
    assert_eq!(images.asked(), vec![photo.id]);
}

// ---------------------------------------------------------------------------
// The same proportion in both views
// ---------------------------------------------------------------------------

#[gpui::test]
fn a_picture_has_the_same_share_of_its_column_in_both_views(cx: &mut TestAppContext) {
    setup(cx);
    let images = FakeImages::new();
    let photo = images.add_png(1600, 1200);
    let small = images.add_png(120, 90);
    let phone = images.add_png(1170, 2532);
    let page = PageSetup::default();
    let widest = phone.max_width(&page);
    assert!(widest < ImageWidth::FULL, "{widest:?}");
    let blocks = vec![
        paragraph(0),
        picture(&photo, 500, Alignment::Center),
        picture(
            &small,
            small.original_width(&page).thousandths(),
            Alignment::Left,
        ),
        picture(&phone, widest.thousandths(), Alignment::Center),
        paragraph(1),
    ];
    let (editor, cx) = open_with(paged(blocks), &images, cx);
    let shares = |editor: &Editor| -> Vec<(f32, f32, f32)> {
        (1..=3)
            .map(|index| {
                let (image, column) = boxed(editor, index);
                (
                    f32::from(image.size.width) / column,
                    f32::from(image.size.height),
                    column,
                )
            })
            .collect()
    };
    let on_page = editor.read_with(cx, |editor, _| shares(editor));
    assert!(close(on_page[0].2, A4_TEXT.0));
    assert!((on_page[0].0 - 0.5).abs() < 0.001);
    // The small one at its own size: 120 points.
    assert!((on_page[1].0 * A4_TEXT.0 - 120.).abs() < 0.3, "{on_page:?}");
    // The phone screenshot as high as a page allows.
    assert!(
        on_page[2].1 <= A4_TEXT.1.floor() && on_page[2].1 > A4_TEXT.1 - 2.,
        "{on_page:?}"
    );

    // Flow view, in a column of 700 points and in a window that leaves it
    // 480.
    editor.update(cx, |editor, cx| editor.set_page_view(false, cx));
    for (window_width, column) in [(1200., 700.), (544., 480.)] {
        cx.simulate_resize(size(px(window_width), px(900.)));
        cx.run_until_parked();
        editor.read_with(cx, |editor, _| {
            let in_flow = shares(editor);
            assert!(close(in_flow[0].2, column), "{in_flow:?}");
            for (page, flow) in on_page.iter().zip(&in_flow) {
                assert!(
                    (page.0 - flow.0).abs() < 0.001,
                    "column {column}: {page:?} on the page, {flow:?} in flow view"
                );
                // Never higher than a page is, in proportion to the
                // column.
                assert!(flow.1 <= column * A4_TEXT.1 / A4_TEXT.0, "{flow:?}");
            }
        });
    }

    // A picture that is stored wider than the page allows is capped in
    // flow view too: by the proportion of the page.
    cx.simulate_resize(size(px(1200.), px(900.)));
    editor.update(cx, |editor, cx| {
        editor.set_document(
            flow(vec![paragraph(0), picture(&phone, 1000, Alignment::Center)]),
            cx,
        );
    });
    cx.run_until_parked();
    let cap = (700. * A4_TEXT.1 / A4_TEXT.0).floor();
    editor.read_with(cx, |editor, _| {
        let (image, column) = boxed(editor, 1);
        assert!(close(column, 700.));
        assert_eq!(f32::from(image.size.height), cap);
        assert!(close(f32::from(image.size.width), cap * 1170. / 2532.));
    });
    // Another paper while in flow view: the cap follows the page.
    editor.update(cx, |editor, cx| {
        editor.edit(
            |state| {
                let mut settings = state.document().settings().clone();
                settings.page.paper = PaperSize::UsLetter;
                state.set_settings(settings);
            },
            cx,
        );
    });
    cx.run_until_parked();
    editor.read_with(cx, |editor, _| {
        let (image, _) = boxed(editor, 1);
        assert_eq!(f32::from(image.size.height), (700f32 * 648. / 468.).floor());
    });
}

// ---------------------------------------------------------------------------
// Selection
// ---------------------------------------------------------------------------

#[gpui::test]
fn the_caret_on_an_image_selects_it(cx: &mut TestAppContext) {
    setup(cx);
    let images = FakeImages::new();
    let photo = images.add_png(800, 600);
    let (editor, cx) = open_with(
        flow(vec![
            Block::paragraph("above"),
            picture(&photo, 500, Alignment::Left),
            Block::paragraph("below"),
        ]),
        &images,
        cx,
    );
    let colors = cx.update(|_, cx| cx.theme().colors);

    // The arrows stop on it once, in both directions and both ways.
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.set_caret(Position::new(0, 5)), cx);
    });
    for (key, expected) in [
        ("right", caret(1, 0)),
        ("right", caret(2, 0)),
        ("left", caret(1, 0)),
        ("left", caret(0, 5)),
        ("down", caret(1, 0)),
        // (Back to the column the caret came from.)
        ("up", caret(0, 5)),
    ] {
        cx.simulate_keystrokes(key);
        assert_eq!(selection(&editor, cx), expected, "{key}");
    }
    cx.simulate_keystrokes("down");
    assert_eq!(selection(&editor, cx), caret(1, 0));
    cx.simulate_keystrokes("down");
    assert_eq!(selection(&editor, cx).head.block, 2);
    cx.simulate_keystrokes("up");
    assert_eq!(selection(&editor, cx), caret(1, 0));

    // Selected: an outline in the accent, four handles, and no caret.
    let bounds = picture_bounds(&editor, 1, cx);
    let image = painted_at(&editor, 1, cx);
    assert_eq!(image.outline, Some(colors.accent));
    assert_eq!(image.handles.len(), 4);
    editor.read_with(cx, |editor, _| {
        assert!(!editor.caret_painted, "an image has no caret");
        assert_eq!(editor.selected_image().map(|(index, _)| index), Some(1));
        // The caret's bounds are the picture: what hangs at the caret
        // hangs under the picture.
        assert_eq!(editor.caret_bounds(), Some(bounds));
        assert_eq!(editor.selection_bounds(), Some(bounds));
        // The handles are at the corners, where the frame drew them.
        let handles = editor.image_handle_bounds().expect("handles");
        assert_eq!(handles.to_vec(), editor.painted_images[0].handles);
        let corners = [
            point(bounds.left(), bounds.top()),
            point(bounds.right(), bounds.top()),
            point(bounds.left(), bounds.bottom()),
            point(bounds.right(), bounds.bottom()),
        ];
        for (handle, corner) in handles.iter().zip(corners) {
            assert_eq!(handle.size, size(px(8.), px(8.)));
            assert!(near(handle.center().x, corner.x) && near(handle.center().y, corner.y));
        }
    });
    // Whatever the caret style: a block caret (Vim) is the outline too.
    editor.update(cx, |editor, cx| {
        editor.set_caret_style(crate::CaretStyle::Block, cx);
    });
    cx.run_until_parked();
    assert_eq!(painted_at(&editor, 1, cx).outline, Some(colors.accent));
    editor.read_with(cx, |editor, _| {
        assert!(!editor.caret_painted);
        assert_eq!(editor.caret_bounds(), Some(bounds));
    });
    editor.update(cx, |editor, cx| {
        editor.set_caret_style(crate::CaretStyle::Bar, cx);
    });

    // In the text the caret is a caret again and the image has no outline.
    cx.simulate_keystrokes("down");
    let image = painted_at(&editor, 1, cx);
    assert_eq!((image.outline, image.handles.len()), (None, 0));
    editor.read_with(cx, |editor, _| {
        assert!(editor.caret_painted);
        assert_eq!(editor.selected_image(), None);
        assert_eq!(editor.image_handle_bounds(), None);
    });

    // A click on the picture selects it; a second and a third click leave
    // it selected.
    for clicks in [1, 2, 3] {
        cx.simulate_keystrokes("cmd-down");
        click(cx, bounds.center(), clicks, Modifiers::none());
        assert_eq!(selection(&editor, cx), caret(1, 0), "{clicks} clicks");
        assert_eq!(painted_at(&editor, 1, cx).handles.len(), 4);
    }

    // A right-click on it selects it too and asks for the menu of its
    // block; `ctrl-enter` asks for the same menu under the picture.
    cx.simulate_keystrokes("cmd-down");
    let requests = Rc::new(RefCell::new(Vec::new()));
    let _watching = cx.update(|_, cx| {
        let requests = requests.clone();
        cx.subscribe(&editor, move |_, event: &EditorEvent, _| {
            if let EditorEvent::ContextMenuRequested(request) = event {
                requests.borrow_mut().push(request.clone());
            }
        })
    });
    cx.simulate_mouse_down(bounds.center(), MouseButton::Right, Modifiers::none());
    cx.simulate_mouse_up(bounds.center(), MouseButton::Right, Modifiers::none());
    assert_eq!(selection(&editor, cx), caret(1, 0));
    cx.simulate_keystrokes("ctrl-enter");
    {
        let requests = requests.borrow();
        assert_eq!(requests.len(), 2);
        assert_eq!((requests[0].block, requests[0].has_selection), (1, false));
        assert_eq!(requests[0].position, bounds.center());
        assert!(!requests[0].from_keyboard && requests[1].from_keyboard);
        assert_eq!(requests[1].block, 1);
        assert!(near(requests[1].position.x, bounds.left()));
        assert!(near(requests[1].position.y, bounds.bottom()));
    }
    // A shift-click on it extends the selection to it, as to any place.
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.set_caret(Position::new(0, 2)), cx);
    });
    click(cx, bounds.center(), 1, Modifiers::shift());
    assert_eq!(
        selection(&editor, cx),
        Selection::new(Position::new(0, 2), Position::new(1, 0))
    );

    // A range over it tints its box: no outline, the picture is part of
    // what is selected.
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.set_caret(Position::new(0, 5)), cx);
    });
    cx.simulate_keystrokes("shift-right");
    assert_eq!(
        selection(&editor, cx),
        Selection::new(Position::new(0, 5), Position::new(1, 0))
    );
    cx.simulate_keystrokes("shift-right");
    assert_eq!(
        selection(&editor, cx),
        Selection::new(Position::new(0, 5), Position::new(2, 0))
    );
    editor.read_with(cx, |editor, _| {
        assert!(
            editor
                .painted_selection
                .iter()
                .any(|tint| same_place(*tint, bounds)),
            "{:?} has no tint at {bounds:?}",
            editor.painted_selection
        );
        assert_eq!(editor.selected_image(), None);
        assert_eq!(editor.painted_images[0].outline, None);
    });

    // Without the keyboard: the outline in grey, no handles. And in an
    // editor that cannot be changed: the outline, no handles.
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.set_caret(Position::new(1, 0)), cx);
    });
    cx.update(|window, _| window.blur());
    cx.run_until_parked();
    let image = painted_at(&editor, 1, cx);
    assert_eq!(image.outline, Some(colors.border_strong));
    assert!(image.handles.is_empty());
    editor.update_in(cx, |editor, window, cx| {
        editor.focus(window);
        editor.set_read_only(true, cx);
    });
    cx.run_until_parked();
    let image = painted_at(&editor, 1, cx);
    assert_eq!(image.outline, Some(colors.accent));
    assert!(image.handles.is_empty());
    assert_eq!(
        editor.read_with(cx, |editor, _| editor.image_handle_bounds()),
        None
    );
}

// ---------------------------------------------------------------------------
// Resize with the pointer
// ---------------------------------------------------------------------------

/// Counts the `Edited` events of an editor.
fn count_edits(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> Rc<RefCell<usize>> {
    let edits = Rc::new(RefCell::new(0));
    cx.update(|_, cx| {
        let edits = edits.clone();
        cx.subscribe(editor, move |_, event: &EditorEvent, _| {
            if matches!(event, EditorEvent::Edited { .. }) {
                *edits.borrow_mut() += 1;
            }
        })
        .detach();
    });
    edits
}

/// The centre of handle `corner` of the selected image.
fn handle(editor: &Entity<Editor>, corner: usize, cx: &mut VisualTestContext) -> Point<Pixels> {
    editor.read_with(cx, |editor, _| {
        editor.image_handle_bounds().expect("a selected image")[corner].center()
    })
}

fn press(cx: &mut VisualTestContext, at: Point<Pixels>) {
    cx.simulate_mouse_down(at, MouseButton::Left, Modifiers::none());
}

fn drag_to(cx: &mut VisualTestContext, at: Point<Pixels>) {
    cx.simulate_mouse_move(at, MouseButton::Left, Modifiers::none());
}

fn release(cx: &mut VisualTestContext, at: Point<Pixels>) {
    cx.simulate_mouse_up(at, MouseButton::Left, Modifiers::none());
}

/// `(revision, undo depth)` of the editor's state.
fn history(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> (u64, usize) {
    editor.read_with(cx, |editor, _| {
        (editor.state().revision(), editor.state().undo_depth())
    })
}

#[gpui::test]
fn a_handle_resizes_the_image_and_only_the_release_is_an_edit(cx: &mut TestAppContext) {
    setup(cx);
    let images = FakeImages::new();
    let photo = images.add_png(800, 600);
    let (editor, cx) = open_with(
        flow(vec![
            paragraph(0),
            picture(&photo, 400, Alignment::Left),
            paragraph(1),
        ]),
        &images,
        cx,
    );
    let bounds = picture_bounds(&editor, 1, cx);
    click(cx, bounds.center(), 1, Modifiers::none());
    let edits = count_edits(&editor, cx);
    let before = history(&editor, cx);
    let below = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |editor, _| {
            editor.position_bounds(Position::new(2, 0)).unwrap().top()
        })
    };
    let text_was_at = below(cx);

    // Press the bottom right handle and pull it 40 pixels outwards.
    let grip = handle(&editor, 3, cx);
    press(cx, grip);
    let pulled = grip + point(px(40.), px(13.));
    drag_to(cx, pulled);
    // The box follows: 280 points became 320, and the text under the
    // picture moved down with it.
    editor.read_with(cx, |editor, _| {
        let (image, column) = boxed(editor, 1);
        assert!(close(column, 700.));
        assert!(
            (f32::from(image.size.width) - 320.).abs() < 0.4,
            "{image:?}"
        );
        assert!(
            (f32::from(image.size.height) - 240.).abs() < 0.4,
            "{image:?}"
        );
        assert_eq!(editor.painted_chip.as_deref(), Some("46 %"));
        assert_eq!(
            editor
                .layout
                .borrow()
                .image_preview()
                .map(|preview| preview.1.thousandths()),
            Some(457)
        );
    });
    assert!(below(cx) > text_was_at + px(25.));
    // The handle is under the pointer.
    assert!(near(handle(&editor, 3, cx).x, pulled.x));
    // Nothing of the document changed: no edit, no undo step.
    assert_eq!(width_of(&editor, 1, cx), 400);
    assert_eq!(history(&editor, cx), before);
    assert_eq!(*edits.borrow(), 0);

    // The release stores the width: one edit, one undo step.
    release(cx, pulled);
    assert_eq!(width_of(&editor, 1, cx), 457);
    assert_eq!(*edits.borrow(), 1);
    assert_eq!(history(&editor, cx).1, before.1 + 1);
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.painted_chip, None);
        assert_eq!(editor.layout.borrow().image_preview(), None);
        let (image, _) = boxed(editor, 1);
        assert!(
            (f32::from(image.size.width) - 320.).abs() < 0.4,
            "{image:?}"
        );
        // Still selected.
        assert_eq!(editor.selection(), caret(1, 0));
    });
    // Undo brings the width back, in one step.
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(width_of(&editor, 1, cx), 400);
    assert_eq!(history(&editor, cx).1, before.1);
    assert!(near(below(cx), text_was_at));

    // A press and a release on a handle without a move changes nothing.
    let grip = handle(&editor, 3, cx);
    press(cx, grip);
    release(cx, grip);
    assert_eq!(width_of(&editor, 1, cx), 400);
    assert_eq!(history(&editor, cx).1, before.1);
}

#[gpui::test]
fn a_dragged_width_snaps_and_stays_within_what_a_width_can_be(cx: &mut TestAppContext) {
    setup(cx);
    let images = FakeImages::new();
    let photo = images.add_png(800, 600);
    let phone = images.add_png(1170, 2532);
    // 200 points wide at its own size: 443 thousandths of an A4 column.
    let small = images.add_png(200, 150);
    let (editor, cx) = open_with(
        flow(vec![
            paragraph(0),
            picture(&photo, 400, Alignment::Left),
            picture(&phone, 200, Alignment::Left),
            picture(&small, 300, Alignment::Left),
            paragraph(1),
        ]),
        &images,
        cx,
    );
    let select = |cx: &mut VisualTestContext, index: usize| {
        let bounds = picture_bounds(&editor, index, cx);
        click(cx, bounds.center(), 1, Modifiers::none());
        assert_eq!(selection(&editor, cx), caret(index, 0));
    };
    let preview = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |editor, _| {
            editor
                .layout
                .borrow()
                .image_preview()
                .map(|preview| preview.1.thousandths())
        })
    };

    select(cx, 1);
    let grip = handle(&editor, 1, cx);
    press(cx, grip);
    // 280 points and 67 more: within six pixels of half the column.
    drag_to(cx, grip + point(px(67.), px(0.)));
    assert_eq!(preview(cx), Some(500));
    // And eight pixels away from it: not snapped.
    drag_to(cx, grip + point(px(62.), px(0.)));
    assert_eq!(preview(cx), Some(489));
    // Near three quarters, and near the whole column.
    drag_to(cx, grip + point(px(249.), px(0.)));
    assert_eq!(preview(cx), Some(750));
    drag_to(cx, grip + point(px(416.), px(0.)));
    assert_eq!(preview(cx), Some(1000));
    // However far the pointer goes: the column.
    drag_to(cx, grip + point(px(900.), px(0.)));
    assert_eq!(preview(cx), Some(1000));
    // And the other way: a twentieth of the column, however far.
    drag_to(cx, grip + point(px(-270.), px(0.)));
    assert_eq!(preview(cx), Some(50));
    drag_to(cx, grip + point(px(-2000.), px(0.)));
    assert_eq!(preview(cx), Some(50));
    release(cx, grip + point(px(-2000.), px(0.)));
    assert_eq!(width_of(&editor, 1, cx), 50);
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(width_of(&editor, 1, cx), 400);

    // The phone screenshot stops where it would be taller than a page.
    let widest = editor.read_with(cx, |editor, _| {
        let image = editor.document().blocks()[2].as_image().unwrap().clone();
        editor.state().max_image_width(&image).thousandths()
    });
    assert!(widest < 1000, "{widest}");
    select(cx, 2);
    let grip = handle(&editor, 3, cx);
    press(cx, grip);
    drag_to(cx, grip + point(px(3000.), px(0.)));
    assert_eq!(preview(cx), Some(widest));
    // Three quarters and the whole column are beyond it: nothing snaps
    // there, the pointer just passes.
    release(cx, grip + point(px(3000.), px(0.)));
    assert_eq!(width_of(&editor, 2, cx), widest);
    cx.simulate_keystrokes("cmd-z");

    // The original size is a place to snap to as well.
    let original = editor.read_with(cx, |editor, _| {
        let image = editor.document().blocks()[3].as_image().unwrap().clone();
        editor.state().original_image_width(&image).thousandths()
    });
    assert_eq!(original, 443);
    select(cx, 3);
    let grip = handle(&editor, 3, cx);
    press(cx, grip);
    // 210 points and 96 more are 306: four away from 443 thousandths of
    // 700 points.
    drag_to(cx, grip + point(px(96.), px(0.)));
    assert_eq!(preview(cx), Some(443));
    release(cx, grip + point(px(96.), px(0.)));
    assert_eq!(width_of(&editor, 3, cx), 443);
}

#[gpui::test]
fn a_centred_image_grows_on_both_sides_and_every_handle_pulls_outwards(cx: &mut TestAppContext) {
    setup(cx);
    let images = FakeImages::new();
    let photo = images.add_png(800, 600);
    let (editor, cx) = open_with(
        flow(vec![
            paragraph(0),
            picture(&photo, 400, Alignment::Center),
            picture(&photo, 400, Alignment::Right),
            picture(&photo, 400, Alignment::Left),
            paragraph(1),
        ]),
        &images,
        cx,
    );
    let select = |cx: &mut VisualTestContext, index: usize| {
        let bounds = picture_bounds(&editor, index, cx);
        click(cx, bounds.center(), 1, Modifiers::none());
    };

    // Centred: 20 pixels at a handle are 40 points of width, 20 on each
    // side, and the handle stays under the pointer.
    select(cx, 1);
    let was = picture_bounds(&editor, 1, cx);
    for (corner, pull) in [(1, 20.), (3, 20.), (0, -20.), (2, -20.)] {
        let grip = handle(&editor, corner, cx);
        press(cx, grip);
        let pulled = grip + point(px(pull), px(0.));
        drag_to(cx, pulled);
        let now = picture_bounds(&editor, 1, cx);
        assert!(
            near(now.left(), was.left() - px(20.)),
            "corner {corner}: {now:?}"
        );
        assert!(
            near(now.right(), was.right() + px(20.)),
            "corner {corner}: {now:?}"
        );
        assert!(
            near(handle(&editor, corner, cx).x, pulled.x),
            "corner {corner}"
        );
        // `escape` gives it up.
        cx.simulate_keystrokes("escape");
        assert!(same_place(picture_bounds(&editor, 1, cx), was));
        release(cx, pulled);
    }
    assert_eq!(width_of(&editor, 1, cx), 400);

    // At the right: the left edge moves, the right one is the column's.
    select(cx, 2);
    let was = picture_bounds(&editor, 2, cx);
    let grip = handle(&editor, 2, cx);
    press(cx, grip);
    drag_to(cx, grip + point(px(-30.), px(0.)));
    let now = picture_bounds(&editor, 2, cx);
    assert!(near(now.right(), was.right()));
    assert!(near(now.left(), was.left() - px(30.)), "{now:?}");
    // The handle on the side that cannot move pulls outwards too.
    drag_to(cx, grip);
    release(cx, grip);
    let grip = handle(&editor, 1, cx);
    press(cx, grip);
    drag_to(cx, grip + point(px(30.), px(0.)));
    let now = picture_bounds(&editor, 2, cx);
    assert!(near(now.right(), was.right()));
    assert!(near(now.left(), was.left() - px(30.)), "{now:?}");
    cx.simulate_keystrokes("escape");
    release(cx, grip);

    // At the left: the right edge moves.
    select(cx, 3);
    let was = picture_bounds(&editor, 3, cx);
    let grip = handle(&editor, 3, cx);
    press(cx, grip);
    drag_to(cx, grip + point(px(-30.), px(0.)));
    let now = picture_bounds(&editor, 3, cx);
    assert!(near(now.left(), was.left()));
    assert!(near(now.right(), was.right() - px(30.)), "{now:?}");
    release(cx, grip + point(px(-30.), px(0.)));
    assert_eq!(width_of(&editor, 3, cx), 357);
}

#[gpui::test]
fn escape_cancels_a_resize_and_a_zoomed_view_holds_the_handle(cx: &mut TestAppContext) {
    setup(cx);
    let images = FakeImages::new();
    let photo = images.add_png(800, 600);
    let (editor, cx) = open_with(
        flow(vec![
            paragraph(0),
            picture(&photo, 400, Alignment::Left),
            paragraph(1),
        ]),
        &images,
        cx,
    );
    let edits = count_edits(&editor, cx);
    let bounds = picture_bounds(&editor, 1, cx);
    click(cx, bounds.center(), 1, Modifiers::none());
    let before = history(&editor, cx);

    let grip = handle(&editor, 3, cx);
    press(cx, grip);
    drag_to(cx, grip + point(px(120.), px(0.)));
    assert!(picture_bounds(&editor, 1, cx).size.width > bounds.size.width + px(100.));
    cx.simulate_keystrokes("escape");
    // As wide as it was, and the release that follows stores nothing.
    assert!(same_place(picture_bounds(&editor, 1, cx), bounds));
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.drag, None);
        assert_eq!(editor.painted_chip, None);
    });
    release(cx, grip + point(px(120.), px(0.)));
    drag_to(cx, grip + point(px(200.), px(0.)));
    assert_eq!(width_of(&editor, 1, cx), 400);
    assert_eq!(history(&editor, cx), before);
    assert_eq!(*edits.borrow(), 0);
    // `escape` is the editor's again: it does not swallow the key now.
    editor.read_with(cx, |editor, _| assert!(editor.drag_escape.is_none()));

    // At 150 %: sixty pixels of the pointer are forty points of width,
    // and the handle is where the pointer is.
    editor.update(cx, |editor, cx| editor.set_zoom(Zoom::Percent(150), cx));
    cx.run_until_parked();
    let bounds = picture_bounds(&editor, 1, cx);
    assert!(near(bounds.size.width, px(420.)), "{bounds:?}");
    click(cx, bounds.center(), 1, Modifiers::none());
    let grip = handle(&editor, 3, cx);
    press(cx, grip);
    let pulled = grip + point(px(60.), px(0.));
    drag_to(cx, pulled);
    assert!(near(handle(&editor, 3, cx).x, pulled.x));
    editor.read_with(cx, |editor, _| {
        let (image, _) = boxed(editor, 1);
        assert!(
            (f32::from(image.size.width) - 320.).abs() < 0.4,
            "{image:?}"
        );
    });
    release(cx, pulled);
    assert_eq!(width_of(&editor, 1, cx), 457);
}

// ---------------------------------------------------------------------------
// Move
// ---------------------------------------------------------------------------

/// The top and the bottom of block `index` in the window.
fn span_of(editor: &Entity<Editor>, index: usize, cx: &mut VisualTestContext) -> (Pixels, Pixels) {
    editor.read_with(cx, |editor, _| {
        editor
            .with_layout(|lay| {
                let placed = lay.placed(index).expect("a block");
                let view = lay.view();
                (view.y(placed.top), view.y(placed.bottom()))
            })
            .expect("laid out")
    })
}

#[gpui::test]
fn dragging_a_picture_moves_its_block(cx: &mut TestAppContext) {
    setup(cx);
    let images = FakeImages::new();
    let photo = images.add_png(800, 600);
    let (editor, cx) = open_with(
        flow(vec![
            paragraph(0),
            paragraph(1),
            picture(&photo, 300, Alignment::Left),
            paragraph(2),
            paragraph(3),
        ]),
        &images,
        cx,
    );
    let colors = cx.update(|_, cx| cx.theme().colors);
    let edits = count_edits(&editor, cx);
    let id = editor.read_with(cx, |editor, _| editor.document().blocks()[2].id());
    let before = history(&editor, cx);
    let column = editor.read_with(cx, |editor, _| {
        let left = editor.position_bounds(Position::new(0, 0)).unwrap().left();
        (left, left + px(700.))
    });

    // A press on the picture selects it; three pixels are not a move yet.
    let grab = picture_bounds(&editor, 2, cx).center();
    press(cx, grab);
    assert_eq!(selection(&editor, cx), caret(2, 0));
    drag_to(cx, grab + point(px(2.), px(2.)));
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.painted_drop_line, None);
        assert_eq!(editor.painted_images[0].veil, None);
    });

    // Into the upper half of the first paragraph: the line is above it,
    // as wide as the column, and the picture is dimmed where it is.
    let (top, bottom) = span_of(&editor, 0, cx);
    let over_first = point(grab.x, top + (bottom - top) * 0.25);
    drag_to(cx, over_first);
    editor.read_with(cx, |editor, _| {
        let line = editor.painted_drop_line.expect("a drop line");
        assert!(line.bottom() <= top + px(0.5), "{line:?} above {top:?}");
        assert!(line.top() > top - px(12.));
        assert!(
            near(line.left(), column.0) && near(line.right(), column.1),
            "{line:?}"
        );
        assert_eq!(line.size.height, px(2.));
        // It keeps its outline and has no handles meanwhile.
        assert!(editor.painted_images[0].outline.is_some());
        assert!(editor.painted_images[0].handles.is_empty());
        // The surface's colour at half strength over the picture: that
        // is what dims it.
        assert_eq!(
            editor.painted_images[0].veil,
            Some(colors.canvas.alpha(0.5))
        );
    });
    // The lower half of the same paragraph: between it and the next.
    drag_to(cx, point(grab.x, top + (bottom - top) * 0.75));
    let (next_top, _) = span_of(&editor, 1, cx);
    editor.read_with(cx, |editor, _| {
        let line = editor.painted_drop_line.expect("a drop line");
        assert!(
            line.top() >= bottom - px(0.5) && line.bottom() <= next_top + px(0.5),
            "{line:?}"
        );
    });
    // Nothing changed yet.
    assert_eq!(history(&editor, cx), before);
    assert_eq!(*edits.borrow(), 0);

    // Released above the first paragraph: the block is the first now,
    // the same block, still selected, in one undo step.
    drag_to(cx, over_first);
    release(cx, over_first);
    assert_eq!(
        kinds(&editor, cx),
        ["image", "paragraph", "paragraph", "paragraph", "paragraph"]
    );
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().blocks()[0].id(), id);
        assert_eq!(editor.selection(), caret(0, 0));
        assert_eq!(editor.painted_drop_line, None);
        assert_eq!(editor.painted_images[0].veil, None);
        assert_eq!(editor.drag, None);
    });
    assert_eq!(*edits.borrow(), 1);
    assert_eq!(history(&editor, cx).1, before.1 + 1);
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(
        kinds(&editor, cx),
        ["paragraph", "paragraph", "image", "paragraph", "paragraph"]
    );
    assert_eq!(history(&editor, cx).1, before.1);

    // Down to under the last paragraph: the end of the document.
    let grab = picture_bounds(&editor, 2, cx).center();
    press(cx, grab);
    let (_, bottom) = span_of(&editor, 4, cx);
    let under = point(grab.x, bottom + px(30.));
    drag_to(cx, under);
    editor.read_with(cx, |editor, _| {
        let line = editor.painted_drop_line.expect("a drop line");
        assert!(line.top() >= bottom - px(0.5), "{line:?}");
    });
    release(cx, under);
    assert_eq!(
        kinds(&editor, cx),
        ["paragraph", "paragraph", "paragraph", "paragraph", "image"]
    );
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().blocks()[4].id(), id);
        assert_eq!(editor.selection(), caret(4, 0));
    });
    cx.simulate_keystrokes("cmd-z");
}

#[gpui::test]
fn a_picture_dragged_to_the_edge_scrolls_the_view_under_it(cx: &mut TestAppContext) {
    setup(cx);
    let images = FakeImages::new();
    let photo = images.add_png(800, 600);
    let mut blocks = vec![paragraph(0), picture(&photo, 200, Alignment::Left)];
    blocks.extend((1..120).map(paragraph));
    let (editor, cx) = open_with(flow(blocks), &images, cx);
    let id = editor.read_with(cx, |editor, _| editor.document().blocks()[1].id());
    let viewport = editor.read_with(cx, |editor, _| editor.viewport_bounds().unwrap());
    let grab = picture_bounds(&editor, 1, cx).center();
    press(cx, grab);
    // Under the bottom edge of the window, and resting there: the view
    // scrolls, and the place the block would go to moves on with it.
    let below = point(grab.x, viewport.bottom() + px(60.));
    drag_to(cx, below);
    let first = editor.read_with(cx, |editor, _| match &editor.drag {
        Some(crate::mouse::Drag::MoveImage(moving)) => moving.drop,
        other => panic!("not a move: {other:?}"),
    });
    for _ in 0..12 {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(35));
    }
    let (scrolled, later) = editor.read_with(cx, |editor, _| {
        let drop = match &editor.drag {
            Some(crate::mouse::Drag::MoveImage(moving)) => moving.drop,
            other => panic!("not a move: {other:?}"),
        };
        (editor.scroll_offset(), drop)
    });
    assert!(scrolled > px(100.), "{scrolled:?}");
    assert!(later > first, "the place went from {first:?} to {later:?}");
    // Back inside the scrolling stops; a release puts the block where the
    // line is now, far below where it was.
    let inside = point(grab.x, viewport.center().y);
    drag_to(cx, inside);
    let resting = editor.read_with(cx, |editor, _| editor.scroll_offset());
    for _ in 0..5 {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(35));
    }
    assert_eq!(
        editor.read_with(cx, |editor, _| editor.scroll_offset()),
        resting
    );
    release(cx, inside);
    editor.read_with(cx, |editor, _| {
        let now = editor.selected_image().expect("still selected").0;
        assert!(now > 8, "the image is block {now}");
        assert_eq!(editor.document().blocks()[now].id(), id);
        // And it is where the pointer let it go: on screen.
        let picture = editor.image_bounds(now).unwrap();
        assert!(viewport.intersects(&picture), "{picture:?}");
    });
}

#[gpui::test]
fn a_picture_dropped_where_it_is_or_given_up_stays(cx: &mut TestAppContext) {
    setup(cx);
    let images = FakeImages::new();
    let photo = images.add_png(800, 600);
    let (editor, cx) = open_with(
        flow(vec![
            paragraph(0),
            paragraph(1),
            picture(&photo, 300, Alignment::Left),
            paragraph(2),
            paragraph(3),
        ]),
        &images,
        cx,
    );
    let edits = count_edits(&editor, cx);
    let order = kinds(&editor, cx);
    let grab = picture_bounds(&editor, 2, cx).center();
    click(cx, grab, 1, Modifiers::none());
    let before = history(&editor, cx);

    // The two boundaries around the picture itself: the lower half of the
    // paragraph before it, its own box, the upper half of the paragraph
    // after it. No line, and a release changes nothing.
    let (top, bottom) = span_of(&editor, 1, cx);
    let before_it = point(grab.x, top + (bottom - top) * 0.75);
    let (top, bottom) = span_of(&editor, 3, cx);
    let after_it = point(grab.x, top + (bottom - top) * 0.25);
    let (top, _) = span_of(&editor, 2, cx);
    let on_itself = point(grab.x, top + px(8.));
    for target in [before_it, after_it, on_itself] {
        press(cx, grab);
        drag_to(cx, target);
        editor.read_with(cx, |editor, _| {
            assert_eq!(editor.painted_drop_line, None, "{target:?}");
            // It is being dragged all the same.
            assert!(editor.painted_images[0].veil.is_some());
        });
        release(cx, target);
        assert_eq!(kinds(&editor, cx), order);
        assert_eq!(history(&editor, cx), before);
    }

    // `escape` in the middle of a move: it stays where it was, and the
    // release after it does nothing.
    press(cx, grab);
    let (top, _) = span_of(&editor, 0, cx);
    let far = point(grab.x, top + px(2.));
    drag_to(cx, far);
    assert!(editor.read_with(cx, |editor, _| editor.painted_drop_line.is_some()));
    cx.simulate_keystrokes("escape");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.painted_drop_line, None);
        assert_eq!(editor.painted_images[0].veil, None);
        assert_eq!(editor.drag, None);
    });
    release(cx, far);
    assert_eq!(kinds(&editor, cx), order);
    assert_eq!(history(&editor, cx), before);
    assert_eq!(*edits.borrow(), 0);
    assert_eq!(selection(&editor, cx), caret(2, 0));

    // A picture that is only pressed is not armed for a move in an editor
    // that cannot be changed: the pointer selects text from it instead.
    editor.update(cx, |editor, cx| editor.set_read_only(true, cx));
    cx.run_until_parked();
    press(cx, grab);
    drag_to(cx, far);
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.painted_drop_line, None);
        assert_eq!(editor.painted_images[0].veil, None);
        assert!(!editor.selection().is_collapsed());
    });
    release(cx, far);
    assert_eq!(kinds(&editor, cx), order);
}

#[gpui::test]
fn the_keys_move_delete_and_write_around_an_image(cx: &mut TestAppContext) {
    setup(cx);
    let images = FakeImages::new();
    let photo = images.add_png(800, 600);
    let document = || {
        flow(vec![
            Block::paragraph("above"),
            picture(&photo, 300, Alignment::Left),
            Block::paragraph("below"),
        ])
    };
    let (editor, cx) = open_with(document(), &images, cx);
    let on_image = |cx: &mut VisualTestContext, index: usize| {
        editor.update(cx, |editor, cx| {
            editor.edit(|state| state.set_caret(Position::new(index, 0)), cx);
        });
    };
    let id = editor.read_with(cx, |editor, _| editor.document().blocks()[1].id());

    // Move Block Up and Down take the image past its neighbours.
    on_image(cx, 1);
    cx.simulate_keystrokes("alt-up");
    assert_eq!(kinds(&editor, cx), ["image", "paragraph", "paragraph"]);
    assert_eq!(selection(&editor, cx), caret(0, 0));
    cx.simulate_keystrokes("alt-down alt-down");
    assert_eq!(kinds(&editor, cx), ["paragraph", "paragraph", "image"]);
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().blocks()[2].id(), id);
        assert_eq!(editor.selection(), caret(2, 0));
    });
    cx.simulate_keystrokes("alt-up");

    // Backspace and delete on it remove it.
    for key in ["backspace", "delete"] {
        on_image(cx, 1);
        cx.simulate_keystrokes(key);
        assert_eq!(kinds(&editor, cx), ["paragraph", "paragraph"], "{key}");
        cx.simulate_keystrokes("cmd-z");
        assert_eq!(
            kinds(&editor, cx),
            ["paragraph", "image", "paragraph"],
            "{key}"
        );
    }

    // Backspace at the start of the paragraph under it selects it first;
    // the second press deletes it. Delete at the end of the paragraph
    // above it likewise.
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.set_caret(Position::new(2, 0)), cx);
    });
    let before = history(&editor, cx);
    cx.simulate_keystrokes("backspace");
    assert_eq!(kinds(&editor, cx), ["paragraph", "image", "paragraph"]);
    assert_eq!(selection(&editor, cx), caret(1, 0));
    assert_eq!(history(&editor, cx), before);
    assert_eq!(painted_at(&editor, 1, cx).handles.len(), 4);
    cx.simulate_keystrokes("backspace");
    assert_eq!(kinds(&editor, cx), ["paragraph", "paragraph"]);
    cx.simulate_keystrokes("cmd-z");
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.set_caret(Position::new(0, 5)), cx);
    });
    cx.simulate_keystrokes("delete");
    assert_eq!(kinds(&editor, cx), ["paragraph", "image", "paragraph"]);
    assert_eq!(selection(&editor, cx), caret(1, 0));
    cx.simulate_keystrokes("delete");
    assert_eq!(kinds(&editor, cx), ["paragraph", "paragraph"]);
    cx.simulate_keystrokes("cmd-z");

    // `enter` starts a paragraph under it, `shift-enter` one above it.
    on_image(cx, 1);
    cx.simulate_keystrokes("enter");
    assert_eq!(
        kinds(&editor, cx),
        ["paragraph", "image", "paragraph", "paragraph"]
    );
    assert_eq!(selection(&editor, cx), caret(2, 0));
    cx.simulate_input("under");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().blocks()[2].text(), "under");
        assert_eq!(editor.document().blocks()[1].id(), id);
    });
    on_image(cx, 1);
    cx.simulate_keystrokes("shift-enter");
    assert_eq!(
        kinds(&editor, cx),
        ["paragraph", "paragraph", "image", "paragraph", "paragraph"]
    );
    assert_eq!(selection(&editor, cx), caret(1, 0));
    cx.simulate_input("over");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().blocks()[1].text(), "over");
        assert_eq!(editor.document().blocks()[2].id(), id);
    });
    // Typing on a selected image writes under it.
    on_image(cx, 2);
    cx.simulate_input("typed");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().blocks()[3].text(), "typed");
        assert_eq!(editor.selection(), caret(3, 5));
    });

    // An image that is the first thing in a document: `shift-enter` is
    // the way to a line above it, and the view shows that line.
    let mut blocks = vec![picture(&photo, 1000, Alignment::Center)];
    blocks.extend((0..40).map(paragraph));
    editor.update(cx, |editor, cx| editor.set_document(flow(blocks), cx));
    cx.run_until_parked();
    // From further down, so that the top is out of view.
    cx.simulate_keystrokes("cmd-down");
    cx.run_until_parked();
    assert!(editor.read_with(cx, |editor, _| editor.scroll_offset()) > px(100.));
    on_image(cx, 0);
    cx.simulate_keystrokes("shift-enter");
    cx.run_until_parked();
    assert_eq!(
        &kinds(&editor, cx)[..3],
        ["paragraph", "image", "paragraph"]
    );
    assert_eq!(selection(&editor, cx), caret(0, 0));
    editor.read_with(cx, |editor, _| {
        let viewport = editor.viewport_bounds().unwrap();
        let line = editor.caret_bounds().unwrap();
        assert!(
            line.top() >= viewport.top() && line.bottom() <= viewport.bottom(),
            "{line:?} in {viewport:?}"
        );
        assert_eq!(editor.scroll_offset(), px(0.));
    });
    cx.simulate_input("A title");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().blocks()[0].text(), "A title");
    });
}

#[gpui::test]
fn an_image_taller_than_the_window_does_not_send_the_view_back_and_forth(cx: &mut TestAppContext) {
    setup(cx);
    let images = FakeImages::new();
    // 700 points wide and 1400 high in a window 500 high.
    let poster = images.add_png(600, 1200);
    let mut blocks: Vec<Block> = (0..6).map(paragraph).collect();
    blocks.push(picture(&poster, 1000, Alignment::Center));
    blocks.extend((6..30).map(paragraph));
    let (editor, cx) = open_with(
        Document::from_blocks(blocks).with_settings(DocumentSettings {
            // A page long enough that the picture is as high as it says.
            page: PageSetup {
                paper: PaperSize::Custom {
                    width: 600.,
                    height: 2400.,
                },
                ..PageSetup::default()
            },
            ..DocumentSettings::default()
        }),
        &images,
        cx,
    );
    cx.simulate_resize(size(px(1200.), px(500.)));
    cx.run_until_parked();
    editor.read_with(cx, |editor, _| {
        let (image, _) = boxed(editor, 6);
        assert!(f32::from(image.size.height) > 1300., "{image:?}");
    });
    let offset = |cx: &mut VisualTestContext| {
        cx.run_until_parked();
        editor.read_with(cx, |editor, _| editor.scroll_offset())
    };

    // The caret comes onto it from above: its top comes into view, with
    // the room a row gets, and stays there frame after frame.
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.set_caret(Position::new(5, 0)), cx);
    });
    cx.simulate_keystrokes("down");
    assert_eq!(selection(&editor, cx), caret(6, 0));
    let settled = offset(cx);
    let (viewport, picture) = editor.read_with(cx, |editor, _| {
        (
            editor.viewport_bounds().unwrap(),
            editor.image_bounds(6).unwrap(),
        )
    });
    assert!(
        picture.top() > viewport.top() && picture.top() < viewport.top() + px(60.),
        "{picture:?}"
    );
    assert!(picture.bottom() > viewport.bottom());
    for _ in 0..5 {
        editor.update(cx, |_, cx| cx.notify());
        assert_eq!(offset(cx), settled);
    }
    // A command on it (a narrower width) keeps it by its top as well.
    cx.simulate_keystrokes("cmd-shift-,");
    assert_eq!(width_of(&editor, 6, cx), 900);
    let after = offset(cx);
    editor.update(cx, |_, cx| cx.notify());
    assert_eq!(offset(cx), after);

    // From below: its top again (the one rule), not its bottom.
    cx.simulate_keystrokes("cmd-z");
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.set_caret(Position::new(12, 0)), cx);
    });
    cx.run_until_parked();
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.set_caret(Position::new(6, 0)), cx);
    });
    assert_eq!(offset(cx), settled);

    // Scrolled into its middle, where it fills the window: selecting it
    // there (a click on it) moves nothing.
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.set_caret(Position::new(12, 0)), cx);
        editor.scroll_by_pixels(px(-700.), cx);
    });
    let filled = offset(cx);
    let (viewport, picture) = editor.read_with(cx, |editor, _| {
        (
            editor.viewport_bounds().unwrap(),
            editor.image_bounds(6).unwrap(),
        )
    });
    assert!(
        picture.top() < viewport.top() && picture.bottom() > viewport.bottom(),
        "{picture:?}"
    );
    click(cx, viewport.center(), 1, Modifiers::none());
    assert_eq!(selection(&editor, cx), caret(6, 0));
    assert_eq!(offset(cx), filled);
    cx.simulate_keystrokes("cmd-shift-e");
    assert_eq!(offset(cx), filled);
}

#[gpui::test]
fn a_page_key_never_moves_the_view_further_than_a_window_over_pictures(cx: &mut TestAppContext) {
    setup(cx);
    let images = FakeImages::new();
    let photo = images.add_png(600, 400);
    let mut blocks = Vec::new();
    for ix in 0..90 {
        blocks.push(paragraph(ix));
        if ix % 3 == 2 {
            blocks.push(picture(&photo, 1000, Alignment::Center));
        }
    }
    for page_view in [false, true] {
        let (editor, cx) = open_with(with_view(blocks.clone(), page_view), &images, cx);
        editor.update(cx, |editor, cx| {
            editor.edit(|state| state.set_caret(Position::ZERO), cx);
        });
        cx.run_until_parked();
        let height = editor.read_with(cx, |editor, _| {
            editor.viewport_bounds().unwrap().size.height
        });
        let state = |cx: &mut VisualTestContext| {
            cx.run_until_parked();
            editor.read_with(cx, |editor, _| {
                let viewport = editor.viewport_bounds().unwrap();
                let caret = editor.caret_bounds().unwrap();
                assert!(
                    caret.top() >= viewport.top() && caret.top() < viewport.bottom(),
                    "the caret is on screen: {caret:?} in {viewport:?}"
                );
                (editor.scroll_offset(), editor.selection().head)
            })
        };
        // Down: every press shows what follows the window it left, and
        // gets on by at least half a window.
        let mut pages = vec![state(cx)];
        for _ in 0..6 {
            cx.simulate_keystrokes("pagedown");
            let (before, _) = *pages.last().unwrap();
            let (after, head) = state(cx);
            let moved = after - before;
            assert!(
                moved > height * 0.5 && moved <= height,
                "page view {page_view}: {before:?} to {after:?} in a window {height:?} high"
            );
            pages.push((after, head));
        }
        // Up again by the same steps, to the start.
        for _ in 0..6 {
            let (before, _) = state(cx);
            cx.simulate_keystrokes("pageup");
            let (after, _) = state(cx);
            let moved = before - after;
            assert!(
                moved > px(0.) && moved <= height,
                "page view {page_view}: {before:?} to {after:?} in a window {height:?} high"
            );
        }
        assert!(state(cx).0 < px(0.5));
        // A selection made with shift covers one window, not several.
        cx.simulate_keystrokes("shift-pagedown");
        let (_, head) = state(cx);
        assert!(head.block <= pages[1].1.block + 1, "{head:?}");
    }
}

#[gpui::test]
fn a_press_beside_a_half_visible_picture_selects_it_and_scrolls_nothing(cx: &mut TestAppContext) {
    setup(cx);
    let images = FakeImages::new();
    // Narrow and tall: a third of the column, with room beside it.
    let tall = images.add_png(400, 1000);
    let mut blocks: Vec<Block> = (0..12).map(paragraph).collect();
    blocks.push(picture(&tall, 333, Alignment::Center));
    blocks.extend((12..40).map(paragraph));
    let (editor, cx) = open_with(flow(blocks), &images, cx);
    // Only the lower part of the picture is on screen.
    editor.update(cx, |editor, cx| {
        let picture = editor.image_bounds(12).unwrap();
        let viewport = editor.viewport_bounds().unwrap();
        editor.scroll_by_pixels(
            picture.top() - viewport.top() + picture.size.height * 0.7,
            cx,
        );
    });
    cx.run_until_parked();
    let (viewport, picture, before) = editor.read_with(cx, |editor, _| {
        (
            editor.viewport_bounds().unwrap(),
            editor.image_bounds(12).unwrap(),
            editor.scroll_offset(),
        )
    });
    assert!(picture.top() < viewport.top() && picture.bottom() > viewport.top() + px(50.));
    // Left of it, on its row.
    let beside = point(
        picture.left() - px(40.),
        viewport.top() + (picture.bottom() - viewport.top()) / 2.,
    );
    assert!(!picture.contains(&beside));
    click(cx, beside, 1, Modifiers::none());
    assert_eq!(selection(&editor, cx), caret(12, 0));
    cx.run_until_parked();
    assert_eq!(
        editor.read_with(cx, |editor, _| editor.scroll_offset()),
        before
    );
}

// ---------------------------------------------------------------------------
// The ways in: chosen files, paste, drop
// ---------------------------------------------------------------------------

/// A folder for the picture files of one test, removed when it ends.
struct Files(PathBuf);

impl Files {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "innerword-editor-images-{name}-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("a temporary folder");
        Files(dir)
    }

    /// Writes a PNG of that size and returns its path.
    fn png(&self, name: &str, width: u32, height: u32) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, FakeImages::png(width, height)).expect("a picture file");
        path
    }

    /// Writes a file that is no picture.
    fn text(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, "not a picture").expect("a file");
        path
    }
}

impl Drop for Files {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

/// The image of block `index` with its alignment.
fn image_of(
    editor: &Entity<Editor>,
    index: usize,
    cx: &mut VisualTestContext,
) -> (Image, Alignment) {
    editor.read_with(cx, |editor, _| {
        let block = &editor.document().blocks()[index];
        (
            block.as_image().expect("an image block").clone(),
            block.alignment(),
        )
    })
}

#[gpui::test]
fn chosen_files_become_images_where_an_insertion_goes(cx: &mut TestAppContext) {
    setup(cx);
    let files = Files::new("chosen");
    let images = FakeImages::new();
    let (editor, cx) = open_with(
        flow(vec![
            Block::paragraph("first"),
            Block::paragraph(""),
            Block::paragraph("last"),
        ]),
        &images,
        cx,
    );
    let empty = editor.read_with(cx, |editor, _| editor.document().blocks()[1].id());

    // Into an empty paragraph: the paragraph becomes the image, which is
    // selected, centred, and as large as it is on the page: 200 points of
    // a column of 451.
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.set_caret(Position::new(1, 0)), cx);
    });
    let before = history(&editor, cx);
    images.choose_beforehand(vec![files.png("a.png", 200, 100)]);
    cx.dispatch_action(InsertImage);
    cx.run_until_parked();
    assert_eq!(kinds(&editor, cx), ["paragraph", "image", "paragraph"]);
    assert_eq!(selection(&editor, cx), caret(1, 0));
    let (image, alignment) = image_of(&editor, 1, cx);
    assert_eq!(image.pixels, (200, 100));
    assert_eq!(image.width.thousandths(), 443);
    assert_eq!(alignment, Alignment::Center);
    assert_eq!(image.description, "");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().blocks()[1].id(), empty);
    });
    assert_eq!(history(&editor, cx).1, before.1 + 1);
    // Its picture is there a moment later.
    assert!(painted_at(&editor, 1, cx).level.is_some());
    assert_eq!(painted_at(&editor, 1, cx).handles.len(), 4);

    // Into text: after the block the caret is in, and nothing of the text
    // changes.
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.set_caret(Position::new(0, 2)), cx);
    });
    images.choose_beforehand(vec![files.png("b.png", 90, 90)]);
    cx.dispatch_action(InsertImage);
    cx.run_until_parked();
    assert_eq!(
        kinds(&editor, cx),
        ["paragraph", "image", "image", "paragraph"]
    );
    assert_eq!(selection(&editor, cx), caret(1, 0));
    assert_eq!(image_of(&editor, 1, cx).0.pixels, (90, 90));
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().blocks()[0].text(), "first");
    });

    // Three files at once, one of them refused and one that is no
    // picture: the others in their order, the last one selected, all of
    // it one undo step.
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.set_caret(Position::new(3, 4)), cx);
    });
    let before = history(&editor, cx);
    let refused = files.png("refused.png", 30, 30);
    images.refuse(&refused);
    images.choose_beforehand(vec![
        files.png("c.png", 300, 200),
        refused,
        files.png("d.png", 1600, 1200),
        files.text("notes.txt"),
        files.png("e.png", 100, 50),
    ]);
    cx.dispatch_action(InsertImage);
    cx.run_until_parked();
    assert_eq!(
        kinds(&editor, cx),
        [
            "paragraph",
            "image",
            "image",
            "paragraph",
            "image",
            "image",
            "image"
        ]
    );
    assert_eq!(selection(&editor, cx), caret(6, 0));
    let sizes: Vec<(u32, u32)> = (4..7)
        .map(|ix| image_of(&editor, ix, cx).0.pixels)
        .collect();
    assert_eq!(sizes, [(300, 200), (1600, 1200), (100, 50)]);
    for index in 4..7 {
        let (image, alignment) = image_of(&editor, index, cx);
        assert_eq!(alignment, Alignment::Center, "block {index}");
        let original = image.original_width(&PageSetup::default());
        assert_eq!(image.width, original, "block {index}");
    }
    // The large one is as wide as the column, not wider.
    assert_eq!(image_of(&editor, 5, cx).0.width, ImageWidth::FULL);
    assert_eq!(history(&editor, cx).1, before.1 + 1);
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(
        kinds(&editor, cx),
        ["paragraph", "image", "image", "paragraph"]
    );

    // A panel that was cancelled, and one that had only a refused file:
    // nothing.
    let before = history(&editor, cx);
    cx.dispatch_action(InsertImage);
    cx.run_until_parked();
    images.choose_beforehand(vec![files.text("more.txt")]);
    cx.dispatch_action(InsertImage);
    cx.run_until_parked();
    assert_eq!(history(&editor, cx), before);

    // The same without the action: `insert_image_files`.
    let path = files.png("f.png", 64, 64);
    editor.update_in(cx, |editor, window, cx| {
        editor.edit(|state| state.set_caret(Position::new(0, 0)), cx);
        editor.insert_image_files(vec![path], window, cx);
    });
    cx.run_until_parked();
    assert_eq!(image_of(&editor, 1, cx).0.pixels, (64, 64));
    assert_eq!(selection(&editor, cx), caret(1, 0));
}

#[gpui::test]
fn a_picture_that_was_on_its_way_lands_where_it_was_asked_for(cx: &mut TestAppContext) {
    setup(cx);
    let files = Files::new("late");
    let images = FakeImages::new();
    let (editor, cx) = open_with(
        flow(vec![
            Block::paragraph("first"),
            Block::paragraph("second"),
            Block::paragraph("third"),
        ]),
        &images,
        cx,
    );
    // Asked for with the caret in the second paragraph; before the
    // picture is stored, a paragraph is added above and the caret moves
    // to the end of the document.
    let path = files.png("a.png", 80, 80);
    editor.update_in(cx, |editor, window, cx| {
        editor.edit(|state| state.set_caret(Position::new(1, 3)), cx);
        editor.insert_image_files(vec![path.clone()], window, cx);
        editor.edit(
            |state| {
                state.set_caret(Position::new(0, 0));
                state.insert_paragraph_break();
                state.move_caret(innerword_core::Movement::DocEnd, false);
            },
            cx,
        );
    });
    cx.run_until_parked();
    assert_eq!(
        kinds(&editor, cx),
        ["paragraph", "paragraph", "paragraph", "image", "paragraph"]
    );
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().blocks()[2].text(), "second");
        assert_eq!(editor.selection(), caret(3, 0));
    });

    // The block it was asked for is gone by then: where the caret is.
    editor.update_in(cx, |editor, window, cx| {
        editor.edit(|state| state.set_caret(Position::new(2, 0)), cx);
        editor.insert_image_files(vec![path.clone()], window, cx);
        editor.edit(
            |state| {
                state.select_block(2);
                state.delete_selection();
                state.set_caret(Position::new(0, 0));
            },
            cx,
        );
    });
    cx.run_until_parked();
    let now = kinds(&editor, cx);
    assert_eq!(
        now.iter().filter(|kind| **kind == "image").count(),
        2,
        "{now:?}"
    );
    assert_eq!(selection(&editor, cx).head.offset, 0);
    assert!(editor.read_with(cx, |editor, _| editor.selected_image().is_some()));

    // Another document is opened before the picture is there: it does not
    // arrive in that one.
    editor.update_in(cx, |editor, window, cx| {
        editor.insert_image_files(vec![path], window, cx);
        editor.set_document(flow(vec![Block::paragraph("another document")]), cx);
    });
    cx.run_until_parked();
    assert_eq!(kinds(&editor, cx), ["paragraph"]);
}

#[gpui::test]
fn pictures_are_pasted_copied_and_cut(cx: &mut TestAppContext) {
    setup(cx);
    let files = Files::new("paste");
    let images = FakeImages::new();
    let photo = images.add_png(400, 300);
    let mut described = photo.clone();
    described.description = "The harbour at dusk".to_string();
    let (editor, cx) = open_with(
        flow(vec![
            Block::paragraph("first"),
            picture(&described, 300, Alignment::Right),
            Block::paragraph("last"),
        ]),
        &images,
        cx,
    );
    let can_paste =
        |cx: &mut VisualTestContext| editor.update(cx, |editor, cx| editor.can_paste(cx));

    // Nothing on the clipboard, nothing on the pasteboard: nothing to
    // paste. A picture only the provider sees is something to paste,
    // although GPUI's clipboard is empty.
    assert!(!can_paste(cx));
    assert_eq!(cx.read_from_clipboard(), None);
    images.set_pasteboard(Some(NewImageSource::Bytes(FakeImages::png(8, 8))));
    assert!(can_paste(cx));
    images.set_pasteboard(None);
    assert!(!can_paste(cx));
    cx.simulate_keystrokes("cmd-v");
    assert_eq!(kinds(&editor, cx), ["paragraph", "image", "paragraph"]);

    // A picture on the clipboard (a screenshot): it goes in after the
    // caret's block, selected, at its own size, centred.
    let screenshot = FakeImages::png(120, 80);
    cx.write_to_clipboard(ClipboardItem::new_image(&gpui::Image::from_bytes(
        gpui::ImageFormat::Png,
        screenshot.clone(),
    )));
    assert!(can_paste(cx));
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.set_caret(Position::new(0, 3)), cx);
    });
    let before = history(&editor, cx);
    cx.simulate_keystrokes("cmd-v");
    cx.run_until_parked();
    assert_eq!(
        kinds(&editor, cx),
        ["paragraph", "image", "image", "paragraph"]
    );
    assert_eq!(selection(&editor, cx), caret(1, 0));
    let (pasted, alignment) = image_of(&editor, 1, cx);
    assert_eq!(pasted.pixels, (120, 80));
    assert_eq!(alignment, Alignment::Center);
    assert_eq!(pasted.width, pasted.original_width(&PageSetup::default()));
    assert_eq!(history(&editor, cx).1, before.1 + 1);
    assert!(painted_at(&editor, 1, cx).level.is_some());
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(kinds(&editor, cx), ["paragraph", "image", "paragraph"]);

    // "Paste as plain text" never takes a picture.
    cx.simulate_keystrokes("cmd-shift-v");
    cx.run_until_parked();
    assert_eq!(kinds(&editor, cx), ["paragraph", "image", "paragraph"]);

    // A picture GPUI's clipboard does not show (a file copied in Finder
    // arrives as its name): the provider sees it, and it is the picture
    // that is pasted, not the name.
    cx.write_to_clipboard(ClipboardItem::new_string("holiday.png".to_string()));
    images.set_pasteboard(Some(NewImageSource::File(files.png("holiday.png", 64, 48))));
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.set_caret(Position::new(2, 4)), cx);
    });
    cx.simulate_keystrokes("cmd-v");
    cx.run_until_parked();
    assert_eq!(
        kinds(&editor, cx),
        ["paragraph", "image", "paragraph", "image"]
    );
    assert_eq!(image_of(&editor, 3, cx).0.pixels, (64, 48));
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().blocks()[2].text(), "last");
        assert_eq!(editor.selection(), caret(3, 0));
    });
    cx.simulate_keystrokes("cmd-z");
    // With an empty clipboard too: that is what `can_paste` is for.
    cx.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string(String::new())));
    images.set_pasteboard(None);
    cx.simulate_keystrokes("cmd-v");
    assert_eq!(kinds(&editor, cx), ["paragraph", "image", "paragraph"]);

    // Copy with the caret on an image puts the picture on the clipboard,
    // for other apps.
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.set_caret(Position::new(1, 0)), cx);
    });
    cx.simulate_keystrokes("cmd-c");
    cx.run_until_parked();
    let item = cx.read_from_clipboard().expect("something was copied");
    match item.entries() {
        [gpui::ClipboardEntry::Image(copied)] => {
            assert_eq!(copied.format, gpui::ImageFormat::Png);
            assert_eq!(copied.bytes, FakeImages::png(400, 300));
        }
        other => panic!("not one picture: {other:?}"),
    }
    assert_eq!(kinds(&editor, cx), ["paragraph", "image", "paragraph"]);
    // Pasted back here it is the block as it was: its width, its place
    // on the line, what it shows. And the provider hears that the image
    // came in.
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.set_caret(Position::new(2, 2)), cx);
    });
    assert_eq!(images.touched(), []);
    cx.simulate_keystrokes("cmd-v");
    cx.run_until_parked();
    assert_eq!(
        kinds(&editor, cx),
        ["paragraph", "image", "paragraph", "image"]
    );
    assert_eq!(selection(&editor, cx), caret(3, 0));
    assert_eq!(image_of(&editor, 3, cx), image_of(&editor, 1, cx));
    let (back, alignment) = image_of(&editor, 3, cx);
    assert_eq!(back.width.thousandths(), 300);
    assert_eq!(alignment, Alignment::Right);
    assert_eq!(back.description, "The harbour at dusk");
    assert_eq!(images.touched(), vec![photo.id]);

    // Cut removes the block and puts the picture there likewise.
    cx.simulate_keystrokes("cmd-x");
    cx.run_until_parked();
    assert_eq!(kinds(&editor, cx), ["paragraph", "image", "paragraph"]);
    assert!(matches!(
        cx.read_from_clipboard().unwrap().entries(),
        [gpui::ClipboardEntry::Image(_)]
    ));
    cx.simulate_keystrokes("cmd-up cmd-v");
    cx.run_until_parked();
    assert_eq!(
        kinds(&editor, cx),
        ["paragraph", "image", "image", "paragraph"]
    );
    assert_eq!(
        image_of(&editor, 1, cx).0.description,
        "The harbour at dusk"
    );
    cx.simulate_keystrokes("cmd-z");

    // A selection of text and a picture is copied as it always was: its
    // text, with the blocks beside it. Pasted, the image is among them,
    // and the provider hears of it.
    editor.update(cx, |editor, cx| {
        editor.edit(
            |state| {
                state.set_selection(Selection::new(Position::new(0, 2), Position::new(2, 2)));
            },
            cx,
        );
    });
    cx.simulate_keystrokes("cmd-c");
    cx.run_until_parked();
    let item = cx.read_from_clipboard().unwrap();
    assert_eq!(item.text().as_deref(), Some("rst\n\nla"));
    assert!(item.metadata().is_some());
    let touched = images.touched().len();
    cx.simulate_keystrokes("cmd-down cmd-v");
    cx.run_until_parked();
    assert_eq!(
        kinds(&editor, cx),
        ["paragraph", "image", "paragraph", "image", "paragraph"]
    );
    assert_eq!(image_of(&editor, 3, cx), image_of(&editor, 1, cx));
    assert_eq!(images.touched().len(), touched + 1);
    assert_eq!(images.touched().last(), Some(&photo.id));
}

#[gpui::test]
fn an_image_is_copied_without_its_bytes_as_its_block(cx: &mut TestAppContext) {
    setup(cx);
    let images = FakeImages::new();
    let photo = images.add_png(400, 300);
    let mut described = photo.clone();
    described.description = "A map of the island".to_string();
    // An editor without a provider: there are no bytes to copy.
    let (editor, cx) = open(
        flow(vec![
            Block::paragraph("first"),
            picture(&described, 250, Alignment::Left),
            picture(&photo, 500, Alignment::Center),
        ]),
        EditorConfig::full(),
        cx,
    );
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.image_status(photo.id), ImageStatus::NoProvider);
        assert!(!editor.has_image_provider());
    });
    assert_eq!(
        painted_at(&editor, 1, cx).label.as_deref(),
        Some("A map of the island")
    );
    assert_eq!(painted_at(&editor, 2, cx).label.as_deref(), Some("Image"));

    // What it shows in words, with the block beside them: pasted here it
    // is the image again.
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.set_caret(Position::new(1, 0)), cx);
    });
    cx.simulate_keystrokes("cmd-c");
    cx.run_until_parked();
    let item = cx.read_from_clipboard().unwrap();
    assert_eq!(item.text().as_deref(), Some("A map of the island"));
    cx.simulate_keystrokes("cmd-up cmd-v");
    cx.run_until_parked();
    assert_eq!(kinds(&editor, cx), ["paragraph", "image", "image", "image"]);
    assert_eq!(selection(&editor, cx), caret(1, 0));
    assert_eq!(image_of(&editor, 1, cx), image_of(&editor, 2, cx));
    // One without a description is never an empty text.
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.set_caret(Position::new(3, 0)), cx);
    });
    cx.simulate_keystrokes("cmd-c");
    cx.run_until_parked();
    assert_eq!(
        cx.read_from_clipboard().unwrap().text().as_deref(),
        Some("Image")
    );

    // Without a provider nothing new comes in: not from the panel, not
    // from the clipboard, not from a drop.
    let before = kinds(&editor, cx);
    cx.dispatch_action(InsertImage);
    cx.write_to_clipboard(ClipboardItem::new_image(&gpui::Image::from_bytes(
        gpui::ImageFormat::Png,
        FakeImages::png(10, 10),
    )));
    cx.simulate_keystrokes("cmd-v");
    editor.update_in(cx, |editor, window, cx| {
        editor.drop_files(
            &[PathBuf::from("/nowhere/a.png")],
            point(px(300.), px(100.)),
            window,
            cx,
        );
    });
    cx.run_until_parked();
    assert_eq!(kinds(&editor, cx), before);
}

#[gpui::test]
fn dropped_files_go_between_the_blocks_nearest_to_the_pointer(cx: &mut TestAppContext) {
    setup(cx);
    let files = Files::new("drop");
    let images = FakeImages::new();
    let (editor, cx) = open_with(flow((0..4).map(paragraph).collect()), &images, cx);
    let column = editor.read_with(cx, |editor, _| {
        editor.position_bounds(Position::new(0, 0)).unwrap().left() + px(200.)
    });
    // The keyboard is elsewhere when the files arrive.
    cx.update(|window, _| window.blur());

    // Over the lower half of the second paragraph: the line is under it.
    let (top, bottom) = span_of(&editor, 1, cx);
    let at = point(column, top + (bottom - top) * 0.75);
    editor.update(cx, |editor, cx| editor.file_drag_moved(at, cx));
    cx.run_until_parked();
    editor.read_with(cx, |editor, _| {
        let line = editor.painted_drop_line.expect("a drop line");
        assert!(line.top() >= bottom - px(0.5), "{line:?}");
        assert_eq!(editor.file_drop, Some(2));
    });
    // Outside the editor there is none.
    editor.update(cx, |editor, cx| {
        editor.file_drag_moved(point(px(-5.), px(-5.)), cx)
    });
    cx.run_until_parked();
    assert_eq!(
        editor.read_with(cx, |editor, _| editor.painted_drop_line),
        None
    );
    editor.update(cx, |editor, cx| editor.file_drag_moved(at, cx));

    // Dropped there: two pictures between the second and the third
    // paragraph, in their order, the last one selected, one undo step.
    // The text file among them is left out. The editor has the keyboard.
    let before = history(&editor, cx);
    let dropped = [
        files.png("a.png", 300, 200),
        files.text("notes.txt"),
        files.png("b.png", 50, 50),
    ];
    editor.update_in(cx, |editor, window, cx| {
        editor.drop_files(&dropped, at, window, cx)
    });
    cx.run_until_parked();
    assert_eq!(
        kinds(&editor, cx),
        [
            "paragraph",
            "paragraph",
            "image",
            "image",
            "paragraph",
            "paragraph"
        ]
    );
    assert_eq!(image_of(&editor, 2, cx).0.pixels, (300, 200));
    assert_eq!(image_of(&editor, 3, cx).0.pixels, (50, 50));
    assert_eq!(image_of(&editor, 3, cx).1, Alignment::Center);
    assert_eq!(selection(&editor, cx), caret(3, 0));
    assert_eq!(history(&editor, cx).1, before.1 + 1);
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.painted_drop_line, None);
        assert_eq!(editor.file_drop, None);
    });
    assert!(cx.update(|window, cx| editor.read(cx).is_focused(window)));
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(kinds(&editor, cx), ["paragraph"; 4]);

    // Above the first block and under the last one.
    let (top, _) = span_of(&editor, 0, cx);
    let first = [files.png("first.png", 20, 20)];
    editor.update_in(cx, |editor, window, cx| {
        editor.drop_files(&first, point(column, top + px(1.)), window, cx);
    });
    cx.run_until_parked();
    assert_eq!(kinds(&editor, cx)[0], "image");
    let (_, bottom) = span_of(&editor, 4, cx);
    let last = [files.png("last.png", 22, 22)];
    editor.update_in(cx, |editor, window, cx| {
        editor.drop_files(&last, point(column, bottom + px(40.)), window, cx);
    });
    cx.run_until_parked();
    assert_eq!(
        kinds(&editor, cx),
        [
            "image",
            "paragraph",
            "paragraph",
            "paragraph",
            "paragraph",
            "image"
        ]
    );
    assert_eq!(selection(&editor, cx), caret(5, 0));
}

#[gpui::test]
fn an_editor_that_cannot_be_changed_or_holds_plain_text_takes_no_pictures(cx: &mut TestAppContext) {
    setup(cx);
    let files = Files::new("refuse");
    let images = FakeImages::new();
    let photo = images.add_png(400, 300);
    let path = files.png("a.png", 60, 60);
    let picture_on_clipboard = |cx: &mut VisualTestContext| {
        cx.write_to_clipboard(ClipboardItem::new_image(&gpui::Image::from_bytes(
            gpui::ImageFormat::Png,
            FakeImages::png(10, 10),
        )));
    };

    // Read-only: nothing comes in, nothing is resized, moved or removed.
    let document = flow(vec![
        Block::paragraph("first"),
        picture(&photo, 400, Alignment::Left),
        Block::paragraph("last"),
    ]);
    let (editor, cx) = open_with(document.clone(), &images, cx);
    editor.update(cx, |editor, cx| {
        editor.set_read_only(true, cx);
        editor.edit(|state| state.set_caret(Position::new(1, 0)), cx);
    });
    cx.run_until_parked();
    images.choose_beforehand(vec![path.clone()]);
    cx.dispatch_action(InsertImage);
    picture_on_clipboard(cx);
    assert!(editor.update(cx, |editor, cx| editor.can_paste(cx)));
    cx.simulate_keystrokes("cmd-v cmd-x backspace cmd-shift-. alt-up enter shift-enter");
    cx.dispatch_action(ImageWidth50);
    cx.dispatch_action(ImageOriginalSize);
    images.set_pasteboard(Some(NewImageSource::File(path.clone())));
    cx.simulate_keystrokes("cmd-v");
    editor.update_in(cx, |editor, window, cx| {
        editor.file_drag_moved(point(px(400.), px(60.)), cx);
        assert_eq!(editor.file_drop, None);
        editor.drop_files(
            std::slice::from_ref(&path),
            point(px(400.), px(60.)),
            window,
            cx,
        );
        editor.insert_image_files(vec![path.clone()], window, cx);
    });
    cx.run_until_parked();
    editor.read_with(cx, |editor, _| assert_eq!(editor.document(), &document));
    // A pasteboard picture is nothing it could take.
    cx.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string(String::new())));
    // Copy still works: reading is allowed.
    cx.simulate_keystrokes("cmd-c");
    cx.run_until_parked();
    assert!(matches!(
        cx.read_from_clipboard().unwrap().entries(),
        [gpui::ClipboardEntry::Image(_)]
    ));

    // A compact editor (a comment being written) holds plain text.
    let (composer, cx) = open(
        Document::new(),
        EditorConfig::compact(CompactOptions::default().submit_on_enter(false)),
        cx,
    );
    composer.update(cx, |editor, cx| {
        editor.set_image_provider(Some(images.clone()), cx);
    });
    cx.simulate_input("a comment");
    images.choose_beforehand(vec![path.clone()]);
    cx.dispatch_action(InsertImage);
    picture_on_clipboard(cx);
    cx.simulate_keystrokes("cmd-v");
    images.set_pasteboard(Some(NewImageSource::File(path.clone())));
    cx.simulate_keystrokes("cmd-v");
    composer.update_in(cx, |editor, window, cx| {
        editor.drop_files(
            std::slice::from_ref(&path),
            point(px(40.), px(10.)),
            window,
            cx,
        );
        editor.insert_image_files(vec![path.clone()], window, cx);
        // The pasteboard's picture does not count for it.
        cx.write_to_clipboard(ClipboardItem::new_string(String::new()));
    });
    cx.run_until_parked();
    composer.read_with(cx, |editor, _| {
        assert_eq!(editor.plain_text(), "a comment");
        assert_eq!(editor.document().block_count(), 1);
    });
}

// ---------------------------------------------------------------------------
// Keys and commands
// ---------------------------------------------------------------------------

/// A parent of the editor that counts the image commands that reach it.
struct Host {
    editor: Entity<Editor>,
    reached: Rc<RefCell<Vec<&'static str>>>,
}

impl Host {
    /// A listener that notes that an action reached the host.
    fn seen<A: gpui::Action>(
        name: &'static str,
        cx: &mut Context<Self>,
    ) -> impl Fn(&A, &mut Window, &mut gpui::App) + 'static {
        cx.listener(move |host: &mut Host, _: &A, _, _| host.reached.borrow_mut().push(name))
    }
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .on_action::<ImageWidth50>(Self::seen("50", cx))
            .on_action::<ImageOriginalSize>(Self::seen("original", cx))
            .on_action::<InsertImage>(Self::seen("insert", cx))
            .on_action::<crate::actions::IncreaseTextSize>(Self::seen("larger", cx))
            .child(self.editor.clone())
    }
}

#[gpui::test]
fn the_size_commands_are_for_the_selected_image(cx: &mut TestAppContext) {
    setup(cx);
    let images = FakeImages::new();
    let photo = images.add_png(200, 150);
    let reached = Rc::new(RefCell::new(Vec::new()));
    let (host, cx) = cx.add_window_view(|window, cx| {
        let editor = cx.new(|cx| {
            Editor::new(
                flow(vec![
                    Block::paragraph("text"),
                    picture(&photo, 400, Alignment::Left),
                    Block::with_text(BlockKind::heading(2), "a heading"),
                ]),
                EditorConfig::full(),
                window,
                cx,
            )
        });
        editor.read(cx).focus(window);
        Host {
            editor,
            reached: reached.clone(),
        }
    });
    cx.update(|window, _| window.activate_window());
    let editor = host.read_with(cx, |host, _| host.editor.clone());
    cx.run_until_parked();
    let sizes = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |editor, _| {
            let blocks = editor.document().blocks();
            (blocks[0].text_size(), blocks[2].text_size())
        })
    };
    let text_sizes = sizes(cx);

    // On the image: larger and smaller step its width, and no text size
    // changes.
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.set_caret(Position::new(1, 0)), cx);
    });
    let before = history(&editor, cx);
    cx.simulate_keystrokes("cmd-shift-.");
    assert_eq!(width_of(&editor, 1, cx), 500);
    cx.simulate_keystrokes("cmd-shift-. cmd-shift-.");
    assert_eq!(width_of(&editor, 1, cx), 667);
    cx.simulate_keystrokes("cmd-shift-,");
    assert_eq!(width_of(&editor, 1, cx), 600);
    assert_eq!(history(&editor, cx).1, before.1 + 4);
    assert_eq!(sizes(cx), text_sizes);
    // The presets and the original size.
    cx.dispatch_action(ImageWidth50);
    assert_eq!(width_of(&editor, 1, cx), 500);
    cx.dispatch_action(crate::actions::ImageWidth25);
    assert_eq!(width_of(&editor, 1, cx), 250);
    cx.dispatch_action(crate::actions::ImageWidth75);
    assert_eq!(width_of(&editor, 1, cx), 750);
    cx.dispatch_action(crate::actions::ImageWidth100);
    assert_eq!(width_of(&editor, 1, cx), 1000);
    cx.dispatch_action(ImageOriginalSize);
    assert_eq!(width_of(&editor, 1, cx), 443);
    // None of them went on to the parent.
    assert_eq!(*reached.borrow(), Vec::<&str>::new());
    // The alignment keys work on it; justify leaves it as it is.
    cx.simulate_keystrokes("cmd-shift-r");
    assert_eq!(image_of(&editor, 1, cx).1, Alignment::Right);
    cx.simulate_keystrokes("cmd-shift-j");
    assert_eq!(image_of(&editor, 1, cx).1, Alignment::Right);
    cx.simulate_keystrokes("cmd-shift-e");
    assert_eq!(image_of(&editor, 1, cx).1, Alignment::Center);

    // In the text: larger is the text's, as it was, and the image keeps
    // its width.
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.set_caret(Position::new(0, 2)), cx);
    });
    cx.simulate_keystrokes("cmd-shift-.");
    let now = sizes(cx);
    assert_ne!(now.0, text_sizes.0);
    assert_eq!(now.1, text_sizes.1);
    assert_eq!(width_of(&editor, 1, cx), 443);
    // A command that is only about an image changes nothing there, and
    // goes on to whoever is further out.
    let before = history(&editor, cx);
    cx.dispatch_action(ImageWidth50);
    cx.dispatch_action(ImageOriginalSize);
    assert_eq!(history(&editor, cx), before);
    assert_eq!(width_of(&editor, 1, cx), 443);
    assert_eq!(*reached.borrow(), ["50", "original"]);
    // A range that covers the image is not "the image is selected".
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.select_all(), cx);
    });
    cx.dispatch_action(ImageWidth50);
    assert_eq!(width_of(&editor, 1, cx), 443);
    assert_eq!(reached.borrow().len(), 3);

    // "Image" without a provider goes on as well; with one it is the
    // editor's.
    reached.borrow_mut().clear();
    cx.dispatch_action(InsertImage);
    assert_eq!(*reached.borrow(), ["insert"]);
    editor.update(cx, |editor, cx| {
        editor.set_image_provider(Some(images.clone()), cx);
    });
    cx.dispatch_action(InsertImage);
    cx.run_until_parked();
    assert_eq!(*reached.borrow(), ["insert"]);

    // The commands are there to be found, the two that resize by the word
    // "image".
    let commands = crate::commands();
    let titles: Vec<&str> = commands
        .iter()
        .map(|command| command.title.as_ref())
        .collect();
    for title in [
        "Image",
        "Image Width 25 %",
        "Image Width 50 %",
        "Image Width 75 %",
        "Image Width 100 %",
        "Image Original Size",
        "Larger Text or Image",
        "Smaller Text or Image",
    ] {
        assert!(
            titles.contains(&title),
            "no command {title:?} among {titles:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// The cache of decoded pictures
// ---------------------------------------------------------------------------

/// What somebody else makes of the document as the editor has it.
fn elsewhere(
    editor: &Entity<Editor>,
    cx: &mut VisualTestContext,
    change: impl FnOnce(&mut EditorState),
) -> Document {
    let stored = editor.read_with(cx, |editor, _| editor.document().to_json());
    let mut other = EditorState::new(Document::from_json(&stored).unwrap());
    change(&mut other);
    Document::from_json(&other.document().to_json()).unwrap()
}

#[gpui::test]
fn a_picture_is_asked_for_once_whatever_happens_to_its_block(cx: &mut TestAppContext) {
    setup(cx);
    let images = FakeImages::new();
    let photo = images.add_png(800, 600);
    let other = images.add_png(640, 640);
    let document = flow(vec![
        paragraph(0),
        picture(&photo, 200, Alignment::Left),
        paragraph(1),
        picture(&photo, 200, Alignment::Right),
        picture(&other, 200, Alignment::Center),
    ]);
    let (editor, cx) = open_with(document.clone(), &images, cx);
    // Two blocks, one picture: asked for once, painted twice.
    assert_eq!(images.asked(), vec![photo.id, other.id]);
    let shown = painted(&editor, cx);
    assert_eq!(shown.len(), 3);
    assert!(shown.iter().all(|image| image.level.is_some()));
    assert_eq!(shown[0].level, shown[1].level);

    // Its block is moved, copied beside itself and typed around.
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.set_caret(Position::new(1, 0)), cx);
    });
    cx.simulate_keystrokes("alt-down alt-up enter");
    cx.simulate_input("words");
    editor.update(cx, |editor, cx| {
        editor.edit(
            |state| {
                state.set_caret(Position::new(1, 0));
                state.duplicate_blocks();
            },
            cx,
        );
    });
    cx.run_until_parked();
    assert_eq!(
        painted(&editor, cx)
            .iter()
            .filter(|image| image.level.is_some())
            .count(),
        4
    );
    assert_eq!(images.asked().len(), 2);

    // A change from elsewhere is taken over: the picture is there in the
    // very next frame, without a placeholder in between.
    let target = elsewhere(&editor, cx, |state| {
        state.set_caret(Position::new(0, 0));
        state.insert_text("Changed elsewhere. ");
        state.set_image_width(1, ImageWidth::from_percent(25));
    });
    editor.update(cx, |editor, cx| {
        editor.adopt_document(target, cx);
    });
    cx.run_until_parked();
    assert!(
        painted(&editor, cx)
            .iter()
            .all(|image| image.level.is_some())
    );
    assert_eq!(width_of(&editor, 1, cx), 250);
    assert_eq!(images.asked().len(), 2);

    // Another document and back: the pictures are still there.
    images.hold();
    editor.update(cx, |editor, cx| {
        editor.set_document(flow(vec![paragraph(0)]), cx);
    });
    cx.run_until_parked();
    assert_eq!(painted(&editor, cx), []);
    editor.update(cx, |editor, cx| editor.set_document(document, cx));
    cx.run_until_parked();
    let shown = painted(&editor, cx);
    assert_eq!(shown.len(), 3);
    assert!(shown.iter().all(|image| image.level.is_some()), "{shown:?}");
    assert_eq!(images.asked().len(), 2);
    images.release();
}

#[gpui::test]
fn a_change_from_elsewhere_in_the_middle_of_a_drag_ends_it_or_lets_it_go_on(
    cx: &mut TestAppContext,
) {
    setup(cx);
    let images = FakeImages::new();
    let photo = images.add_png(800, 600);
    let (editor, cx) = open_with(
        flow(vec![
            paragraph(0),
            picture(&photo, 400, Alignment::Left),
            paragraph(1),
            paragraph(2),
        ]),
        &images,
        cx,
    );
    let id = editor.read_with(cx, |editor, _| editor.document().blocks()[1].id());
    let middle = picture_bounds(&editor, 1, cx).center();
    click(cx, middle, 1, Modifiers::none());

    // A handle is held while somebody else adds a paragraph above: the
    // image is another block now, the same image, and the resize goes on.
    let grip = handle(&editor, 3, cx);
    press(cx, grip);
    drag_to(cx, grip + point(px(40.), px(0.)));
    let target = elsewhere(&editor, cx, |state| {
        state.set_caret(Position::new(0, 0));
        state.insert_paragraph_break();
    });
    editor.update(cx, |editor, cx| {
        editor.adopt_document(target, cx);
    });
    cx.run_until_parked();
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().blocks()[2].id(), id);
        let (image, _) = boxed(editor, 2);
        assert!(
            (f32::from(image.size.width) - 320.).abs() < 0.4,
            "{image:?}"
        );
    });
    drag_to(cx, grip + point(px(70.), px(0.)));
    release(cx, grip + point(px(70.), px(0.)));
    assert_eq!(width_of(&editor, 2, cx), 500);

    // A handle is held while somebody else deletes the image: the next
    // move of the pointer ends the drag, and the release does nothing.
    let grip = handle(&editor, 3, cx);
    press(cx, grip);
    drag_to(cx, grip + point(px(20.), px(0.)));
    let target = elsewhere(&editor, cx, |state| {
        state.set_caret(Position::new(2, 0));
        state.backspace();
    });
    editor.update(cx, |editor, cx| {
        editor.adopt_document(target.clone(), cx);
    });
    cx.run_until_parked();
    drag_to(cx, grip + point(px(60.), px(0.)));
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.drag, None);
        assert!(editor.drag_escape.is_none());
        assert_eq!(editor.layout.borrow().image_preview(), None);
    });
    release(cx, grip + point(px(60.), px(0.)));
    editor.read_with(cx, |editor, _| assert_eq!(editor.document(), &target));
    assert_eq!(kinds(&editor, cx), ["paragraph"; 4]);

    // The same for a picture that is being moved.
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(kinds(&editor, cx)[2], "image");
    let grab = picture_bounds(&editor, 2, cx).center();
    press(cx, grab);
    let (top, _) = span_of(&editor, 0, cx);
    drag_to(cx, point(grab.x, top + px(2.)));
    assert!(editor.read_with(cx, |editor, _| editor.painted_drop_line.is_some()));
    editor.update(cx, |editor, cx| {
        editor.adopt_document(target.clone(), cx);
    });
    cx.run_until_parked();
    release(cx, point(grab.x, top + px(2.)));
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.drag, None);
        assert_eq!(editor.document(), &target);
        assert_eq!(editor.painted_drop_line, None);
    });
}

#[gpui::test]
fn a_larger_zoom_asks_for_a_finer_picture_and_paints_the_one_it_has_meanwhile(
    cx: &mut TestAppContext,
) {
    setup(cx);
    let images = FakeImages::new();
    let photo = images.add_png(1600, 1200);
    let (editor, cx) = open_with(
        paged(vec![
            paragraph(0),
            picture(&photo, 500, Alignment::Center),
            paragraph(1),
        ]),
        &images,
        cx,
    );
    cx.simulate_resize(size(px(1600.), px(1000.)));
    editor.update(cx, |editor, cx| editor.set_zoom(Zoom::Percent(100), cx));
    cx.run_until_parked();
    // Half a column of 451 points on a display of two pixels to the
    // point: 451 device pixels. A quarter of the picture's 1600 does not
    // do, half of them do.
    assert_eq!(painted_at(&editor, 1, cx).level, Some(1));
    assert_eq!(images.asked(), vec![photo.id]);
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.images.levels(photo.id), [1]);
        assert_eq!(editor.images.decoded_bytes(), 800 * 600 * 4);
    });

    // Smaller: the picture it has is finer than needed, and a coarser one
    // is made (the GPU does not shrink a picture to less than half).
    images.hold();
    editor.update(cx, |editor, cx| editor.set_zoom(Zoom::Percent(50), cx));
    cx.run_until_parked();
    assert_eq!(painted_at(&editor, 1, cx).level, Some(1), "meanwhile");
    assert_eq!(images.asked().len(), 2);
    images.release();
    cx.run_until_parked();
    assert_eq!(painted_at(&editor, 1, cx).level, Some(2));

    // 300 %: 1354 device pixels need the picture as it is. Asked for
    // once, however many frames pass, and the coarse one is painted
    // until the fine one is there.
    images.hold();
    editor.update(cx, |editor, cx| editor.set_zoom(Zoom::Percent(300), cx));
    cx.run_until_parked();
    assert_eq!(
        painted_at(&editor, 1, cx).level,
        Some(1),
        "the nearest it has"
    );
    for _ in 0..3 {
        editor.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
    }
    assert_eq!(images.asked().len(), 3);
    images.release();
    cx.run_until_parked();
    assert_eq!(painted_at(&editor, 1, cx).level, Some(0));
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.images.levels(photo.id), [0, 1, 2]);
    });
    // Back at 100 %: what is there is used, nothing is asked for.
    editor.update(cx, |editor, cx| editor.set_zoom(Zoom::Percent(100), cx));
    cx.run_until_parked();
    assert_eq!(painted_at(&editor, 1, cx).level, Some(1));
    assert_eq!(images.asked().len(), 3);
    // The box never changed with any of it.
    editor.read_with(cx, |editor, _| {
        let (image, column) = boxed(editor, 1);
        assert!(close(f32::from(image.size.width), column / 2.));
    });
}

#[gpui::test]
fn an_image_without_a_picture_says_why_and_is_asked_for_again_when_told(cx: &mut TestAppContext) {
    setup(cx);
    let images = FakeImages::new();
    let lost = images.add_png(300, 200);
    images.remove(lost.id);
    // A PNG whose header is whole and whose pixels are not.
    let mut spoilt = FakeImages::png(320, 240);
    let middle = spoilt.len() / 2;
    for byte in &mut spoilt[middle..middle + 16] {
        *byte = !*byte;
    }
    let broken = images
        .add_bytes(spoilt)
        .expect("the header says what it is");
    assert_eq!(broken.pixels, (320, 240));
    let mut described = lost.clone();
    described.description = "A chart of the tides".to_string();
    let (editor, cx) = open_with(
        flow(vec![
            paragraph(0),
            picture(&described, 400, Alignment::Left),
            picture(&broken, 400, Alignment::Left),
            paragraph(1),
        ]),
        &images,
        cx,
    );
    // The box is there at its size, with a sentence in it: also for an
    // image with a description, because that is what is the matter.
    let missing = painted_at(&editor, 1, cx);
    assert_eq!(missing.level, None);
    assert_eq!(missing.label.as_deref(), Some("Missing image"));
    assert!(near(missing.bounds.size.width, px(280.)), "{missing:?}");
    assert_eq!(
        painted_at(&editor, 2, cx).label.as_deref(),
        Some("This image cannot be shown")
    );
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.image_status(lost.id), ImageStatus::Missing);
        assert_eq!(editor.image_status(broken.id), ImageStatus::Broken);
    });
    // It is selected, resized and deleted like any image.
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.set_caret(Position::new(1, 0)), cx);
    });
    cx.simulate_keystrokes("cmd-shift-.");
    assert_eq!(width_of(&editor, 1, cx), 500);
    assert_eq!(painted_at(&editor, 1, cx).handles.len(), 4);

    // Neither is asked for again frame after frame.
    let asked = images.asked().len();
    assert_eq!(asked, 2);
    for _ in 0..4 {
        editor.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
    }
    assert_eq!(images.asked().len(), asked);

    // The library has the picture now (an agent brought it, a version was
    // restored). The editor asks again when it is told to.
    assert_eq!(images.add_png(300, 200).id, lost.id);
    editor.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    assert_eq!(
        painted_at(&editor, 1, cx).label.as_deref(),
        Some("Missing image")
    );
    editor.update(cx, |editor, cx| editor.images_changed(cx));
    cx.run_until_parked();
    let found = painted_at(&editor, 1, cx);
    assert!(found.level.is_some());
    assert_eq!(found.label, None);
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.image_status(lost.id), ImageStatus::Ready);
        // The broken one was asked for again as well, and is what it was.
        assert_eq!(editor.image_status(broken.id), ImageStatus::Broken);
    });
    assert_eq!(images.asked().len(), asked + 2);

    // A new provider is a new chance too; none at all is a placeholder
    // with the description.
    editor.update(cx, |editor, cx| editor.set_image_provider(None, cx));
    cx.run_until_parked();
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.image_status(broken.id), ImageStatus::NoProvider);
        // What is decoded stays.
        assert_eq!(editor.image_status(lost.id), ImageStatus::Ready);
    });
    assert_eq!(painted_at(&editor, 2, cx).label.as_deref(), Some("Image"));

    // A placeholder too small for its words says less, or nothing.
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.set_image_width(2, ImageWidth::MIN), cx);
    });
    cx.run_until_parked();
    let small = painted_at(&editor, 2, cx);
    assert!(small.bounds.size.width < px(40.));
    assert_eq!(small.label, None);
}

#[gpui::test]
fn decoded_pictures_stay_within_their_budget_and_their_textures_go_with_them(
    cx: &mut TestAppContext,
) {
    setup(cx);
    let images = FakeImages::new();
    // Twelve pictures, each 400 by 300: 480,000 bytes decoded at the size
    // a column of 700 points needs.
    let pictures: Vec<Image> = (0..12).map(|ix| images.add_png(400 + ix, 300)).collect();
    let mut blocks = Vec::new();
    for image in &pictures {
        blocks.push(picture(image, 1000, Alignment::Center));
        blocks.extend((0..3).map(paragraph));
    }
    let (editor, cx) = open_with(flow(blocks), &images, cx);
    // Room for three of them, not for four.
    let budget = 1_500_000;
    editor.update(cx, |editor, _| editor.images.set_budget(budget));
    // Down through the document, a third of a screen at a time.
    for _ in 0..40 {
        editor.update(cx, |editor, cx| editor.scroll_by_pixels(px(300.), cx));
        cx.run_until_parked();
        editor.read_with(cx, |editor, _| {
            // Whatever a frame painted has its picture: nothing that is
            // on screen is ever given up for room.
            assert!(!editor.painted_images.is_empty());
            for image in &editor.painted_images {
                assert!(image.level.is_some(), "{image:?}");
            }
            assert!(editor.images.decoded_bytes() <= budget);
        });
    }
    let asked = images.asked().len();
    assert_eq!(asked, 12, "every picture was on screen once");
    editor.read_with(cx, |editor, _| {
        let decoded = editor.images.decoded_bytes();
        assert!(decoded <= budget, "{decoded} bytes are decoded");
        // What left the cache was handed to the window, texture by
        // texture.
        let kept = pictures
            .iter()
            .filter(|image| !editor.images.levels(image.id).is_empty())
            .count();
        assert!(kept <= 3, "{kept} pictures are kept");
        assert_eq!(editor.images.textures_dropped, 12 - kept);
    });
    // Back at the top the first picture is decoded again.
    cx.simulate_keystrokes("cmd-up");
    cx.run_until_parked();
    assert!(painted_at(&editor, 0, cx).level.is_some());
    let again = images.asked().split_off(asked);
    assert!(again.contains(&pictures[0].id), "{again:?}");
}

/// A view that shows an editor until it lets go of it.
struct Holder {
    editor: Option<Entity<Editor>>,
}

impl Render for Holder {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().children(self.editor.clone())
    }
}

#[gpui::test]
fn an_editor_that_goes_away_takes_its_textures_out_of_the_window(cx: &mut TestAppContext) {
    setup(cx);
    let images = FakeImages::new();
    let pictures = [images.add_png(300, 200), images.add_png(200, 300)];
    let (holder, cx) = cx.add_window_view(|window, cx| {
        let editor = cx.new(|cx| {
            let mut editor = Editor::new(
                flow(vec![
                    picture(&pictures[0], 300, Alignment::Left),
                    picture(&pictures[1], 200, Alignment::Left),
                    paragraph(0),
                ]),
                EditorConfig::full(),
                window,
                cx,
            );
            editor.set_image_provider(Some(images.clone()), cx);
            editor
        });
        Holder {
            editor: Some(editor),
        }
    });
    cx.run_until_parked();
    let shown = holder.read_with(cx, |holder, cx| {
        holder
            .editor
            .as_ref()
            .unwrap()
            .read(cx)
            .painted_images
            .len()
    });
    assert_eq!(shown, 2);
    let before = crate::images::RELEASED_TEXTURES.with(|count| count.get());
    // The window stays, the editor goes. (The window holds on to what the
    // last frames registered, the editor's listeners among it, for two
    // frames more.)
    holder.update(cx, |holder, cx| {
        holder.editor = None;
        cx.notify();
    });
    cx.run_until_parked();
    for _ in 0..2 {
        holder.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
    }
    assert_eq!(
        crate::images::RELEASED_TEXTURES.with(|count| count.get()),
        before + 2,
        "the pictures of an editor that went away are still in the window's atlas"
    );
}

// ---------------------------------------------------------------------------
// On paper
// ---------------------------------------------------------------------------

#[gpui::test]
fn an_image_is_on_the_printed_page_where_page_view_has_it(cx: &mut TestAppContext) {
    setup(cx);
    let images = FakeImages::new();
    let photo = images.add_png(800, 600);
    let mut chart = images.add_png(500, 500);
    chart.description = "Rainfall by month".to_string();
    let mut blocks: Vec<Block> = (0..12).map(paragraph).collect();
    // Does not fit under twelve paragraphs: the second sheet.
    blocks.push(picture(&photo, 1000, Alignment::Center));
    blocks.push(paragraph(12));
    blocks.push(picture(&chart, 400, Alignment::Right));
    // The same picture a second time.
    blocks.push(picture(&photo, 250, Alignment::Left));
    let (editor, cx) = open_with(paged(blocks), &images, cx);
    editor.update(cx, |editor, cx| editor.finish_layout(cx));
    let page_count = editor.read_with(cx, |editor, _| editor.page_count().unwrap());
    assert_eq!(page_count, 2);
    // Where page view has each picture: its sheet, and its box from the
    // sheet's top left corner (one-inch margins).
    let on_screen: Vec<(usize, f32, f32, f32, f32)> = editor.read_with(cx, |editor, _| {
        [12, 14, 15]
            .into_iter()
            .map(|index| {
                let (sheet, y) = on_sheet(editor, index);
                let (image, _) = boxed(editor, index);
                (
                    sheet,
                    72. + f32::from(image.x),
                    72. + y,
                    f32::from(image.size.width),
                    f32::from(image.size.height),
                )
            })
            .collect()
    });
    assert_eq!(on_screen[0].0, 1);

    let bytes: Rc<dyn Fn(innerword_core::ImageId) -> Option<Arc<[u8]>>> = {
        let photo = photo.id;
        Rc::new(move |image| (image == photo).then(|| FakeImages::png(800, 600).into()))
    };
    let pages = cx.update(|window, cx| {
        let mut pass = editor.read(cx).print_pass(window, cx);
        // What the pass will ask for: each image once.
        assert_eq!(pass.images(), vec![photo.id, chart.id]);
        pass.set_images(bytes.clone());
        pass.run_to_end()
    });
    assert_eq!(pages.len(), page_count);
    assert!(
        !pages[0]
            .items
            .iter()
            .any(|item| matches!(item, Item::Image(_))),
        "the first page has no picture"
    );
    let printed: Vec<&innerword_print::ImageItem> = pages[1]
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Image(image) => Some(image),
            _ => None,
        })
        .collect();
    // The photograph twice, at its two boxes, with one key and the bytes
    // as they are stored.
    assert_eq!(printed.len(), 2);
    for (item, place) in printed.iter().zip([on_screen[0], on_screen[2]]) {
        assert_eq!(item.key, *photo.id.as_bytes());
        assert_eq!(&item.bytes[..], &FakeImages::png(800, 600)[..]);
        assert!(close(item.x, place.1), "{item:?} against {place:?}");
        assert!(close(item.y, place.2), "{item:?} against {place:?}");
        assert!(
            close(item.width, place.3) && close(item.height, place.4),
            "{item:?}"
        );
    }
    assert!(close(printed[0].width, A4_TEXT.0));
    // The one nobody has bytes for: its box as a hairline, and what it
    // shows in its middle.
    let place = on_screen[1];
    let frame = pages[1]
        .items
        .iter()
        .find_map(|item| match item {
            Item::Rect(rect) if rect.fill.is_none() && close(rect.width, place.3) => Some(rect),
            _ => None,
        })
        .expect("the box of the picture without bytes");
    assert!(
        close(frame.x, place.1) && close(frame.y, place.2),
        "{frame:?}"
    );
    assert!(close(frame.height, place.4));
    assert!(frame.border.is_some());
    let words = pages[1]
        .items
        .iter()
        .find_map(|item| match item {
            Item::Label(label) if label.text == "Rainfall by month" => Some(label),
            _ => None,
        })
        .expect("the description of the picture without bytes");
    assert!(close(words.x, place.1 + place.3 / 2.), "{words:?}");
    assert!(
        words.baseline > place.2 && words.baseline < place.2 + place.4,
        "{words:?}"
    );
    assert_eq!(words.anchor, innerword_print::Anchor::Middle);

    // Without anybody to ask: every picture is its box, and the pages are
    // the same pages.
    let bare = cx.update(|window, cx| editor.read(cx).print_pass(window, cx).run_to_end());
    assert_eq!(bare.len(), page_count);
    assert!(
        bare.iter()
            .flat_map(|page| &page.items)
            .all(|item| !matches!(item, Item::Image(_)))
    );
    // Whatever the window shows (flow view, a zoom), paper has them where
    // the page has them.
    editor.update(cx, |editor, cx| {
        editor.set_page_view(false, cx);
        editor.set_zoom(Zoom::Percent(150), cx);
    });
    cx.run_until_parked();
    let again = cx.update(|window, cx| {
        let mut pass = editor.read(cx).print_pass(window, cx);
        pass.set_images(bytes.clone());
        pass.run_to_end()
    });
    assert_eq!(again, pages);
}
