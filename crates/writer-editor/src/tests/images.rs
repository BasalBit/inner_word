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
use writer_core::{
    Alignment, Block, BlockKind, Document, DocumentSettings, EditorState, Image, ImageWidth,
    Margins, PageSetup, PaperSize, Position, Selection,
};
use writer_print::Item;
use writer_ui::theme::ActiveTheme;

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
