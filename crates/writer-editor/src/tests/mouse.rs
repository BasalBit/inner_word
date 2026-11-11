//! The mouse: clicks, drags, checkboxes, links, the wheel and the scroll
//! bar. Never required, but it has to work.

use std::time::Duration;

use gpui::{
    Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, Pixels, Point, ScrollDelta,
    ScrollWheelEvent, TestAppContext, TouchPhase, VisualTestContext, point, px,
};
use writer_core::{Block, BlockKind, Document, InlineAttrs, Position, Selection};

use super::slice::{open, setup};
use crate::{Editor, EditorConfig};

fn paragraphs(count: usize) -> Document {
    Document::from_blocks(
        (0..count)
            .map(|ix| Block::paragraph(&format!("Paragraph number {ix}")))
            .collect(),
    )
}

/// The middle of the caret line box of a position.
fn point_of(editor: &Editor, block: usize, offset: usize) -> Point<Pixels> {
    editor
        .position_bounds(Position::new(block, offset))
        .unwrap()
        .center()
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

#[gpui::test]
fn clicks_and_drags_select(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(
        Document::from_blocks(vec![
            Block::paragraph("alpha beta gamma"),
            Block::paragraph("second paragraph here"),
            Block::paragraph("third"),
        ]),
        EditorConfig::full(),
        cx,
    );
    let at = |cx: &mut VisualTestContext, block, offset| {
        editor.read_with(cx, |editor, _| point_of(editor, block, offset))
    };
    let selection =
        |cx: &mut VisualTestContext| editor.read_with(cx, |editor, _| editor.selection());

    // Click: the caret goes to the nearest boundary.
    let target = at(cx, 1, 7);
    click(cx, target, 1, Modifiers::none());
    assert_eq!(selection(cx), Selection::caret(Position::new(1, 7)));
    // A click a little right of a boundary still picks it.
    let target = at(cx, 0, 3) + point(px(3.), px(0.));
    click(cx, target, 1, Modifiers::none());
    assert_eq!(selection(cx), Selection::caret(Position::new(0, 3)));

    // Shift-click extends from the anchor.
    let target = at(cx, 1, 6);
    click(cx, target, 1, Modifiers::shift());
    assert_eq!(
        selection(cx),
        Selection::new(Position::new(0, 3), Position::new(1, 6))
    );

    // Drag: press, move with the button held, release.
    let (from, over, to) = (at(cx, 0, 6), at(cx, 1, 3), at(cx, 2, 2));
    cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(over, MouseButton::Left, Modifiers::none());
    assert_eq!(
        selection(cx),
        Selection::new(Position::new(0, 6), Position::new(1, 3))
    );
    cx.simulate_mouse_move(to, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(to, MouseButton::Left, Modifiers::none());
    assert_eq!(
        selection(cx),
        Selection::new(Position::new(0, 6), Position::new(2, 2))
    );
    // After the release, moving the mouse changes nothing.
    let elsewhere = at(cx, 0, 0);
    cx.simulate_mouse_move(elsewhere, None, Modifiers::none());
    assert_eq!(selection(cx).head, Position::new(2, 2));

    // Double click: the word; dragging on extends by words.
    let (word, later, earlier) = (at(cx, 0, 8), at(cx, 1, 9), at(cx, 0, 2));
    cx.simulate_event(MouseDownEvent {
        position: word,
        modifiers: Modifiers::none(),
        button: MouseButton::Left,
        click_count: 2,
        first_mouse: false,
    });
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.state().selected_text(), "beta")
    });
    cx.simulate_mouse_move(later, MouseButton::Left, Modifiers::none());
    editor.read_with(cx, |editor, _| {
        assert_eq!(
            editor.state().selected_text(),
            "beta gamma\nsecond paragraph"
        );
    });
    cx.simulate_mouse_move(earlier, MouseButton::Left, Modifiers::none());
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.state().selected_text(), "alpha beta")
    });
    cx.simulate_mouse_up(word, MouseButton::Left, Modifiers::none());

    // Triple click: the block; dragging on extends by blocks.
    let (line, below) = (at(cx, 1, 4), at(cx, 2, 1));
    cx.simulate_event(MouseDownEvent {
        position: line,
        modifiers: Modifiers::none(),
        button: MouseButton::Left,
        click_count: 3,
        first_mouse: false,
    });
    editor.read_with(cx, |editor, _| {
        assert_eq!(
            editor.state().selected_text().trim_end(),
            "second paragraph here"
        );
    });
    cx.simulate_mouse_move(below, MouseButton::Left, Modifiers::none());
    editor.read_with(cx, |editor, _| {
        assert_eq!(
            editor.state().selected_text(),
            "second paragraph here\nthird"
        );
    });
    cx.simulate_mouse_up(line, MouseButton::Left, Modifiers::none());

    // A click below the last block lands at the end of the document.
    let viewport = editor.read_with(cx, |editor, _| editor.viewport_bounds().unwrap());
    click(
        cx,
        point(viewport.center().x, viewport.bottom() - px(20.)),
        1,
        Modifiers::none(),
    );
    assert_eq!(selection(cx).head.block, 2);
}

#[gpui::test]
fn checkbox_click_toggles_without_moving_the_caret(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(
        Document::from_blocks(vec![
            Block::paragraph("intro"),
            Block::with_text(BlockKind::todo(), "buy milk"),
            Block::with_text(BlockKind::TodoItem { checked: true }, "done already"),
        ]),
        EditorConfig::full(),
        cx,
    );
    cx.simulate_keystrokes("right right");
    // The checkbox sits in the gutter left of the item's text.
    let boxes = editor.read_with(cx, |editor, _| editor.checkbox_hits.clone());
    assert_eq!(boxes.len(), 2);
    let text_left = editor.read_with(cx, |editor, _| point_of(editor, 1, 0).x);
    assert!(boxes[0].bounds.right() < text_left);
    assert!(text_left - boxes[0].bounds.left() < px(40.));

    click(cx, boxes[0].bounds.center(), 1, Modifiers::none());
    editor.read_with(cx, |editor, _| {
        assert_eq!(
            editor.document().blocks()[1].kind(),
            &BlockKind::TodoItem { checked: true }
        );
        assert_eq!(editor.selection(), Selection::caret(Position::new(0, 2)));
    });
    click(cx, boxes[1].bounds.center(), 1, Modifiers::none());
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().blocks()[2].kind(), &BlockKind::todo());
    });
    // One undo step each.
    cx.simulate_keystrokes("cmd-z");
    editor.read_with(cx, |editor, _| {
        assert_eq!(
            editor.document().blocks()[2].kind(),
            &BlockKind::TodoItem { checked: true }
        );
    });

    // Read-only: the click does nothing.
    editor.update(cx, |editor, cx| editor.set_read_only(true, cx));
    cx.run_until_parked();
    let boxes = editor.read_with(cx, |editor, _| editor.checkbox_hits.clone());
    click(cx, boxes[0].bounds.center(), 1, Modifiers::none());
    editor.read_with(cx, |editor, _| {
        assert_eq!(
            editor.document().blocks()[1].kind(),
            &BlockKind::TodoItem { checked: true }
        );
    });
}

#[gpui::test]
fn cmd_click_opens_a_link(cx: &mut TestAppContext) {
    setup(cx);
    let link = InlineAttrs {
        link: Some("https://example.com/page".into()),
        ..InlineAttrs::default()
    };
    let (editor, cx) = open(
        Document::from_blocks(vec![
            Block::new(BlockKind::Paragraph)
                .with_span("see ", InlineAttrs::default())
                .with_span("the page", link)
                .with_span(" for more", InlineAttrs::default()),
        ]),
        EditorConfig::full(),
        cx,
    );
    let on_link = editor.read_with(cx, |editor, _| {
        point_of(editor, 0, 7) + point(px(2.), px(0.))
    });
    let off_link = editor.read_with(cx, |editor, _| {
        point_of(editor, 0, 15) + point(px(2.), px(0.))
    });

    // A plain click only places the caret.
    click(cx, on_link, 1, Modifiers::none());
    assert_eq!(cx.opened_url(), None);
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.selection().head, Position::new(0, 7))
    });

    // cmd-click outside the link places the caret too.
    click(cx, off_link, 1, Modifiers::command());
    assert_eq!(cx.opened_url(), None);
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.selection().head, Position::new(0, 15))
    });

    // cmd-click on the link opens it and leaves the caret alone.
    click(cx, on_link, 1, Modifiers::command());
    assert_eq!(cx.opened_url().as_deref(), Some("https://example.com/page"));
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.selection().head, Position::new(0, 15))
    });

    // With cmd held over a link the cursor is a pointing hand.
    cx.simulate_mouse_move(on_link, None, Modifiers::command());
    editor.read_with(cx, |editor, _| assert!(editor.link_hover));
    cx.simulate_mouse_move(off_link, None, Modifiers::command());
    editor.read_with(cx, |editor, _| assert!(!editor.link_hover));
    cx.simulate_mouse_move(on_link, None, Modifiers::none());
    editor.read_with(cx, |editor, _| assert!(!editor.link_hover));
    cx.simulate_modifiers_change(Modifiers::command());
    editor.read_with(cx, |editor, _| assert!(editor.link_hover));
}

#[gpui::test]
fn wheel_scroll_bar_and_drag_autoscroll(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(paragraphs(300), EditorConfig::full(), cx);
    let viewport = editor.read_with(cx, |editor, _| editor.viewport_bounds().unwrap());
    let scroll =
        |cx: &mut VisualTestContext| editor.read_with(cx, |editor, _| editor.scroll_offset());

    // The wheel scrolls by pixels, the caret stays.
    cx.simulate_event(ScrollWheelEvent {
        position: viewport.center(),
        delta: ScrollDelta::Pixels(point(px(0.), px(-240.))),
        modifiers: Modifiers::none(),
        touch_phase: TouchPhase::Moved,
    });
    assert_eq!(scroll(cx), px(240.));
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.selection().head, Position::ZERO);
        // The bar shows while scrolling and hides again.
        assert!(editor.scrollbar.is_shown());
    });
    cx.executor().advance_clock(Duration::from_secs(2));
    editor.read_with(cx, |editor, _| assert!(!editor.scrollbar.is_shown()));
    // Lines scroll by rows of body text; scrolling stops at the top.
    cx.simulate_event(ScrollWheelEvent {
        position: viewport.center(),
        delta: ScrollDelta::Lines(point(0., 2.)),
        modifiers: Modifiers::none(),
        touch_phase: TouchPhase::Moved,
    });
    assert_eq!(scroll(cx), px(240. - 54.));
    cx.simulate_event(ScrollWheelEvent {
        position: viewport.center(),
        delta: ScrollDelta::Pixels(point(px(0.), px(5000.))),
        modifiers: Modifiers::none(),
        touch_phase: TouchPhase::Moved,
    });
    assert_eq!(scroll(cx), px(0.));

    // Dragging the scroll bar thumb scrolls proportionally.
    let geometry = editor.read_with(cx, |editor, _| editor.scrollbar.geometry.unwrap());
    assert_eq!(geometry.thumb.top(), geometry.track.top());
    let grab = geometry.thumb.center();
    let travel = geometry.track.size.height - geometry.thumb.size.height;
    cx.simulate_mouse_move(grab, None, Modifiers::none());
    editor.read_with(cx, |editor, _| assert!(editor.scrollbar.hovered));
    cx.simulate_mouse_down(grab, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(
        grab + point(px(0.), travel / 2.),
        MouseButton::Left,
        Modifiers::none(),
    );
    let half = scroll(cx);
    assert!((half - geometry.max_scroll / 2.).abs() < px(2.), "{half:?}");
    cx.simulate_mouse_move(
        grab + point(px(0.), travel * 2.),
        MouseButton::Left,
        Modifiers::none(),
    );
    cx.simulate_mouse_up(grab, MouseButton::Left, Modifiers::none());
    editor.read_with(cx, |editor, _| {
        // At the end; the caret did not move and nothing was selected.
        assert!(editor.scroll_offset() > half * 1.8);
        assert_eq!(editor.selection(), Selection::caret(Position::ZERO));
        assert_eq!(editor.last_visible_row().unwrap().start.block, 299);
    });
    cx.simulate_keystrokes("cmd-up");
    assert_eq!(scroll(cx), px(0.));

    // A drag selection that leaves the viewport scrolls and keeps
    // extending while the pointer rests outside.
    let start = editor.read_with(cx, |editor, _| point_of(editor, 2, 4));
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
    let below = point(start.x, viewport.bottom() + px(60.));
    cx.simulate_mouse_move(below, MouseButton::Left, Modifiers::none());
    let first = editor.read_with(cx, |editor, _| editor.selection().head.block);
    for _ in 0..10 {
        cx.executor().advance_clock(Duration::from_millis(35));
    }
    let (scrolled, head) = editor.read_with(cx, |editor, _| {
        (editor.scroll_offset(), editor.selection().head.block)
    });
    assert!(scrolled > px(100.), "{scrolled:?}");
    assert!(head > first, "the selection grew from {first} to {head}");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.selection().anchor, Position::new(2, 4));
    });
    // Back inside, the scrolling stops.
    cx.simulate_mouse_move(viewport.center(), MouseButton::Left, Modifiers::none());
    let resting = scroll(cx);
    for _ in 0..5 {
        cx.executor().advance_clock(Duration::from_millis(35));
    }
    assert_eq!(scroll(cx), resting);
    cx.simulate_mouse_up(viewport.center(), MouseButton::Left, Modifiers::none());
}
