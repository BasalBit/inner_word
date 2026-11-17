//! Where the popover is drawn in a window: beside its text, inside the
//! editor's viewport, following the text.

use gpui::{Bounds, Pixels, TestAppContext, VisualTestContext, px};
use innerword_assistant_quick::place::{FULL_HEIGHT, GAP, MARGIN, Side, WIDTH};
use innerword_core::{Document, Position, TextRange};

use crate::support::*;

/// A document of many short paragraphs and one long one.
fn long_document() -> Document {
    let mut lines: Vec<String> = (0..120)
        .map(|index| format!("Paragraph number {index} of the text."))
        .collect();
    lines[3] = "word ".repeat(60).trim_end().to_string();
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    paragraphs(&lines)
}

fn rows(fx: &Fixture, range: TextRange, cx: &mut VisualTestContext) -> Vec<Bounds<Pixels>> {
    fx.editor
        .read_with(cx, |editor, _| editor.range_bounds(range))
        .expect("the editor has a layout")
}

fn inside(inner: Bounds<Pixels>, outer: Bounds<Pixels>) -> bool {
    inner.left() >= outer.left() + MARGIN - px(0.5)
        && inner.top() >= outer.top() + MARGIN - px(0.5)
        && inner.right() <= outer.right() - MARGIN + px(0.5)
        && inner.bottom() <= outer.bottom() - MARGIN + px(0.5)
}

/// The first block whose line box starts in the last `from_bottom` pixels
/// of the viewport.
fn block_near_bottom(fx: &Fixture, from_bottom: f32, cx: &mut VisualTestContext) -> usize {
    let viewport = fx.viewport(cx);
    fx.editor.read_with(cx, |editor, _| {
        (0..editor.document().block_count())
            .find(|index| {
                editor
                    .position_bounds(Position::new(*index, 0))
                    .is_some_and(|bounds| {
                        bounds.top() > viewport.bottom() - px(from_bottom)
                            && bounds.bottom() < viewport.bottom()
                    })
            })
            .expect("a block near the bottom of the viewport")
    })
}

#[gpui::test]
fn it_hangs_under_the_selection(cx: &mut TestAppContext) {
    let (fx, cx) = open_quick(long_document(), cx);
    let range = TextRange::in_block(1, 10, 16);
    select(&fx.editor, range, cx);
    fx.toggle(cx);

    let popover = fx.popover(cx);
    let viewport = fx.viewport(cx);
    let rows = rows(&fx, range, cx);
    assert_eq!(rows.len(), 1);
    // Under the row, its left edge on the selection's left edge.
    assert_eq!(popover.top(), rows[0].bottom() + GAP);
    assert_eq!(popover.left(), rows[0].left());
    assert_eq!(popover.size.width, WIDTH);
    assert!(inside(popover, viewport));
    assert!(!popover.intersects(&rows[0]));
    let drawn = fx.quick.read_with(cx, |quick, _| quick.drawn()).unwrap();
    assert_eq!(drawn.side, Side::Below);
    assert!(drawn.clear);
    assert_eq!(drawn.max_height, FULL_HEIGHT);
}

#[gpui::test]
fn it_opens_at_the_caret_when_nothing_is_selected(cx: &mut TestAppContext) {
    let (fx, cx) = open_quick(long_document(), cx);
    caret(&fx.editor, 2, 9, cx);
    fx.toggle(cx);

    let popover = fx.popover(cx);
    let at = fx
        .editor
        .read_with(cx, |editor, _| editor.position_bounds(Position::new(2, 9)))
        .unwrap();
    assert_eq!(popover.top(), at.bottom() + GAP);
    assert_eq!(popover.left(), at.left());
    assert!(inside(popover, fx.viewport(cx)));
}

#[gpui::test]
fn a_selection_of_several_rows_is_not_covered(cx: &mut TestAppContext) {
    let (fx, cx) = open_quick(long_document(), cx);
    // From the long paragraph into the one after the next.
    let range = TextRange::new(Position::new(3, 20), Position::new(5, 9));
    select(&fx.editor, range, cx);
    fx.toggle(cx);
    fx.fake.reply("An answer of a few words.");
    fx.send("Shorter.", cx);

    let popover = fx.popover(cx);
    let rows = rows(&fx, range, cx);
    assert!(rows.len() >= 3);
    for row in &rows {
        assert!(
            !popover.intersects(row),
            "{row:?} is covered by {popover:?}"
        );
    }
    let last = rows.last().unwrap();
    assert_eq!(popover.top(), last.bottom() + GAP);
    assert_eq!(popover.left(), last.left());
}

#[gpui::test]
fn near_the_bottom_it_stands_over_the_text(cx: &mut TestAppContext) {
    let (fx, cx) = open_quick(long_document(), cx);
    let block = block_near_bottom(&fx, 80., cx);
    let range = TextRange::in_block(block, 0, 9);
    select(&fx.editor, range, cx);
    fx.toggle(cx);

    let viewport = fx.viewport(cx);
    let row = rows(&fx, range, cx)[0];
    let popover = fx.popover(cx);
    assert_eq!(popover.bottom(), row.top() - GAP);
    assert!(inside(popover, viewport));
    assert!(!popover.intersects(&row));
    let before = popover;

    // It stays over the text while it grows with the conversation.
    fx.fake
        .reply("One.\n\nTwo.\n\nThree.\n\nFour.\n\nFive.\n\nSix.");
    fx.send("Count.", cx);
    let popover = fx.popover(cx);
    assert!(popover.size.height > before.size.height);
    assert_eq!(popover.bottom(), row.top() - GAP);
    assert!(inside(popover, viewport));
    assert_eq!(
        fx.quick
            .read_with(cx, |quick, _| quick.drawn())
            .unwrap()
            .side,
        Side::Above
    );
}

#[gpui::test]
fn at_the_right_edge_it_stays_inside_the_viewport(cx: &mut TestAppContext) {
    let (fx, cx) = open_quick(long_document(), cx);
    // The end of the first row of the long paragraph.
    let first_row_end = fx.editor.read_with(cx, |editor, _| {
        let first = editor.position_bounds(Position::new(3, 0)).unwrap();
        (0..300)
            .rev()
            .find(|offset| {
                editor
                    .position_bounds(Position::new(3, *offset))
                    .is_some_and(|bounds| bounds.top() == first.top())
            })
            .unwrap()
    });
    let range = TextRange::in_block(3, first_row_end - 4, first_row_end);
    select(&fx.editor, range, cx);
    fx.toggle(cx);

    let viewport = fx.viewport(cx);
    let popover = fx.popover(cx);
    let row = rows(&fx, range, cx)[0];
    assert!(inside(popover, viewport));
    assert!(popover.left() <= row.left());
    assert!(!popover.intersects(&row));
}

#[gpui::test]
fn it_follows_the_text_when_the_editor_scrolls(cx: &mut TestAppContext) {
    let (fx, cx) = open_quick(long_document(), cx);
    let block = block_near_bottom(&fx, 400., cx);
    let range = TextRange::in_block(block, 0, 9);
    select(&fx.editor, range, cx);
    fx.toggle(cx);
    let before = fx.popover(cx);
    let row_before = rows(&fx, range, cx)[0];

    fx.editor
        .update(cx, |editor, cx| editor.scroll_by_pixels(px(54.), cx));
    cx.run_until_parked();

    let after = fx.popover(cx);
    let row_after = rows(&fx, range, cx)[0];
    assert_eq!(row_after.top(), row_before.top() - px(54.));
    // In the same frame as the text.
    assert_eq!(after.top(), before.top() - px(54.));
    assert_eq!(after.left(), before.left());
    assert!(fx.is_open(cx), "scrolling does not close it");
    assert!(fx.composer_focused(cx));
}

#[gpui::test]
fn it_stays_in_the_viewport_while_its_text_is_out_of_sight(cx: &mut TestAppContext) {
    let (fx, cx) = open_quick(long_document(), cx);
    let range = TextRange::in_block(1, 0, 9);
    select(&fx.editor, range, cx);
    fx.toggle(cx);

    fx.editor
        .update(cx, |editor, cx| editor.scroll_by_pixels(px(1500.), cx));
    cx.run_until_parked();
    let viewport = fx.viewport(cx);
    let row = rows(&fx, range, cx)[0];
    assert!(
        row.bottom() < viewport.top(),
        "the text is above the viewport"
    );
    let popover = fx.popover(cx);
    assert!(inside(popover, viewport));
    assert_eq!(popover.top(), viewport.top() + MARGIN);
    assert!(fx.is_open(cx));

    // And back with the text.
    fx.editor
        .update(cx, |editor, cx| editor.scroll_by_pixels(px(-1500.), cx));
    cx.run_until_parked();
    let row = rows(&fx, range, cx)[0];
    assert_eq!(fx.popover(cx).top(), row.bottom() + GAP);
}

#[gpui::test]
fn it_grows_with_the_conversation_to_a_limit_and_then_scrolls(cx: &mut TestAppContext) {
    let (fx, cx) = open_quick(long_document(), cx);
    let range = TextRange::in_block(1, 0, 9);
    select(&fx.editor, range, cx);
    fx.toggle(cx);
    let empty = fx.popover(cx);
    assert!(empty.size.height < px(160.), "small while nothing is said");

    fx.fake.reply("A short answer.");
    fx.send("One?", cx);
    let one = fx.popover(cx);
    assert!(one.size.height > empty.size.height);
    assert!(one.size.height < FULL_HEIGHT);
    // Everything said so far is in view.
    let (_, hidden) = fx.quick.read_with(cx, |quick, cx| {
        quick.messages().unwrap().read(cx).scroll_position()
    });
    assert_eq!(hidden, px(0.));
    assert_eq!(one.top(), empty.top(), "it grows downwards");

    fx.fake.reply("A second short answer.");
    fx.send("Two?", cx);
    let two = fx.popover(cx);
    assert!(two.size.height > one.size.height);

    // A long answer: the popover stops at its limit and the conversation
    // scrolls inside it.
    let long: Vec<String> = (0..40)
        .map(|index| format!("Paragraph {index} of a long answer."))
        .collect();
    fx.fake.reply(long.join("\n\n"));
    fx.send("At length?", cx);
    let full = fx.popover(cx);
    assert_eq!(full.size.height, FULL_HEIGHT);
    assert!(inside(full, fx.viewport(cx)));
    let (offset, hidden) = fx.quick.read_with(cx, |quick, cx| {
        quick.messages().unwrap().read(cx).scroll_position()
    });
    assert!(hidden > px(200.), "the conversation scrolls: {hidden:?}");
    assert!(offset > px(0.), "and follows the answer to its end");
    // The composer is still inside, under the conversation.
    let composer = bounds("assistant-composer", cx);
    assert!(full.contains(&composer.center()));
    assert!(composer.bottom() <= full.bottom());
    // And the text it is about is still not covered.
    assert!(!full.intersects(&rows(&fx, range, cx)[0]));
}

#[gpui::test]
fn a_selection_that_fills_the_viewport_is_covered_at_its_edge(cx: &mut TestAppContext) {
    let (fx, cx) = open_quick(long_document(), cx);
    cx.simulate_keystrokes("cmd-a");
    cx.run_until_parked();
    fx.toggle(cx);

    let viewport = fx.viewport(cx);
    let popover = fx.popover(cx);
    assert!(inside(popover, viewport));
    let drawn = fx.quick.read_with(cx, |quick, _| quick.drawn()).unwrap();
    assert!(
        !drawn.clear,
        "there is no room beside a selection like this"
    );
    assert!(fx.composer_focused(cx));
}

#[gpui::test]
fn a_narrow_window_gets_a_narrow_popover(cx: &mut TestAppContext) {
    let (fx, cx) = open_quick(long_document(), cx);
    cx.simulate_resize(gpui::size(px(520.), px(600.)));
    cx.run_until_parked();
    select(&fx.editor, TextRange::in_block(1, 0, 9), cx);
    fx.toggle(cx);

    let viewport = fx.viewport(cx);
    assert!(viewport.size.width < WIDTH + MARGIN * 2.);
    let popover = fx.popover(cx);
    assert_eq!(popover.size.width, viewport.size.width - MARGIN * 2.);
    assert!(inside(popover, viewport));
    // Its controls are inside it.
    for control in ["assistant-model", "assistant-effort", "quick-chat-close"] {
        let control = bounds(control, cx);
        assert!(control.left() >= popover.left() && control.right() <= popover.right());
    }
}

#[gpui::test]
fn an_answer_that_grows_makes_the_popover_taller_in_the_same_frame(cx: &mut TestAppContext) {
    let (fx, cx) = open_quick(long_document(), cx);
    select(&fx.editor, TextRange::in_block(1, 0, 9), cx);
    fx.toggle(cx);

    // An answer that arrives a paragraph at a time.
    let pieces: Vec<String> = (0..12)
        .map(|index| format!("Paragraph {index} of the answer, a few words long.\n\n"))
        .collect();
    fx.fake.hold();
    fx.fake.push(innerword_assistant::FakeReply::pieces(pieces));
    fx.send("At length?", cx);

    let mut heights = vec![fx.popover(cx).size.height];
    while fx.fake.advance(1) == 1 {
        cx.run_until_parked();
        let popover = fx.popover(cx);
        if popover.size.height < FULL_HEIGHT {
            // Below its limit nothing of the conversation is hidden.
            let (offset, hidden) = fx.quick.read_with(cx, |quick, cx| {
                quick.messages().unwrap().read(cx).scroll_position()
            });
            assert_eq!((offset, hidden), (px(0.), px(0.)));
        }
        heights.push(popover.size.height);
    }
    assert!(
        heights.windows(2).all(|pair| pair[1] >= pair[0]),
        "{heights:?}"
    );
    assert!(heights.len() > 10);
    assert_eq!(*heights.last().unwrap(), FULL_HEIGHT);
    // No frame was drawn with the conversation scrolled for want of room
    // it was given a frame later.
    assert_eq!(fx.quick.read_with(cx, |quick, _| quick.late_growths()), 0);
}
