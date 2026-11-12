//! The mouse: clicks, drags, checkboxes, links, the wheel and the scroll
//! bar. Never required, but it has to work.

use std::{cell::RefCell, rc::Rc, time::Duration};

use gpui::{
    Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, Pixels, Point, ScrollDelta,
    ScrollWheelEvent, TestAppContext, TouchPhase, VisualTestContext, point, px,
};
use writer_core::{Block, BlockKind, Document, InlineAttrs, Position, Selection, TextRange};

use super::slice::{open, setup};
use crate::{ContextMenuRequest, Editor, EditorConfig, EditorEvent, link_tip::LINK_TIP_DELAY};

fn paragraphs(count: usize) -> Document {
    Document::from_blocks(
        (0..count)
            .map(|ix| Block::paragraph(&format!("Paragraph number {ix}")))
            .collect(),
    )
}

/// The middle of the caret line box of a position.
pub(super) fn point_of(editor: &Editor, block: usize, offset: usize) -> Point<Pixels> {
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

#[gpui::test]
fn the_mouse_works_on_sheets(cx: &mut TestAppContext) {
    setup(cx);
    let long = "A sentence that is long enough to wrap in the text area of a page. ".repeat(6);
    let mut blocks: Vec<Block> = (0..40).map(|_| Block::paragraph(long.trim_end())).collect();
    blocks.insert(
        1,
        Block::with_text(BlockKind::todo(), "a to-do on the first sheet"),
    );
    let (editor, cx) = open(Document::from_blocks(blocks), EditorConfig::full(), cx);
    editor.update(cx, |editor, cx| editor.set_page_view(true, cx));
    cx.run_until_parked();

    // A click on text, on the first and on a later sheet.
    let target = editor.read_with(cx, |editor, _| point_of(editor, 2, 30));
    click(cx, target, 1, Modifiers::none());
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.selection(), Selection::caret(Position::new(2, 30)));
        assert_eq!(editor.caret_page(), Some(1));
    });
    editor.update(cx, |editor, cx| {
        editor.scroll_to_position(Position::new(20, 0), crate::Autoscroll::Center, cx);
    });
    cx.run_until_parked();
    let target = editor.read_with(cx, |editor, _| point_of(editor, 20, 12));
    click(cx, target, 1, Modifiers::none());
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.selection(), Selection::caret(Position::new(20, 12)));
        assert!(editor.caret_page().unwrap() > 1);
    });

    // A click on the desk beside a sheet lands on the row at that height.
    let viewport = editor.read_with(cx, |editor, _| editor.viewport_bounds().unwrap());
    let beside = point(viewport.left() + px(4.), target.y);
    click(cx, beside, 1, Modifiers::none());
    editor.read_with(cx, |editor, _| {
        let head = editor.selection().head;
        assert_eq!(head.block, 20);
        assert_eq!(
            editor.visual_row(head),
            editor.visual_row(Position::new(20, 12))
        );
    });

    // Checkboxes work on a sheet too.
    cx.simulate_keystrokes("cmd-up");
    cx.run_until_parked();
    let boxes = editor.read_with(cx, |editor, _| editor.checkbox_hits.clone());
    assert_eq!(boxes.len(), 1);
    click(cx, boxes[0].bounds.center(), 1, Modifiers::none());
    editor.read_with(cx, |editor, _| {
        assert_eq!(
            editor.document().blocks()[1].kind(),
            &BlockKind::TodoItem { checked: true }
        );
    });
}

#[gpui::test]
fn a_click_past_the_end_of_a_wrapped_row_stays_on_it(cx: &mut TestAppContext) {
    setup(cx);
    let long = "word ".repeat(60);
    let (editor, cx) = open(
        Document::from_blocks(vec![Block::paragraph(long.trim_end())]),
        EditorConfig::full(),
        cx,
    );
    // Rows of 65 bytes; the second one is 65..130.
    let (row, next, column_left) = editor.read_with(cx, |editor, _| {
        (
            editor.position_bounds(Position::new(0, 70)).unwrap(),
            editor.position_bounds(Position::new(0, 130)).unwrap(),
            editor.position_bounds(Position::new(0, 65)).unwrap().left(),
        )
    });
    assert!(next.top() > row.top());
    let caret = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |editor, _| {
            (editor.selection().head, editor.caret_bounds().unwrap())
        })
    };

    // Past the end of the row's text: the caret is at the end of the row.
    let past = point(column_left + px(695.), row.center().y);
    click(cx, past, 1, Modifiers::none());
    let (head, bounds) = caret(cx);
    assert_eq!(head, Position::new(0, 130));
    assert_eq!(bounds.top(), row.top());
    assert!(bounds.left() > column_left + px(600.));
    // The same position clicked at the start of the next row is there.
    click(cx, next.center(), 1, Modifiers::none());
    let (head, bounds) = caret(cx);
    assert_eq!(head, Position::new(0, 130));
    assert_eq!(bounds.top(), next.top());
    // And back, although the position is the same.
    click(cx, past, 1, Modifiers::none());
    assert_eq!(caret(cx).1.top(), row.top());
    // The public hit test knows no affinity: it stays before the wrap.
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.position_at_point(past), Some(Position::new(0, 129)));
    });

    // Dragging to the right of a row selects through its end and leaves
    // the head there; shift-click does the same.
    let from = editor.read_with(cx, |editor, _| point_of(editor, 0, 70));
    cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(past, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(past, MouseButton::Left, Modifiers::none());
    editor.read_with(cx, |editor, _| {
        assert_eq!(
            editor.selection(),
            Selection::new(Position::new(0, 70), Position::new(0, 130))
        );
    });
    cx.simulate_keystrokes("shift-up");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.selection().head, Position::new(0, 65));
    });
    click(cx, from, 1, Modifiers::none());
    click(cx, past, 1, Modifiers::shift());
    editor.read_with(cx, |editor, _| {
        assert_eq!(
            editor.selection(),
            Selection::new(Position::new(0, 70), Position::new(0, 130))
        );
    });

    // A double click past the end selects the last word, not the caret's
    // affinity.
    click(cx, past, 2, Modifiers::none());
    editor.read_with(cx, |editor, _| {
        assert!(!editor.selection().is_collapsed());
        assert!(!editor.caret_upstream);
    });

    // A block caret has no affinity.
    editor.update(cx, |editor, cx| {
        editor.set_caret_style(crate::CaretStyle::Block, cx)
    });
    click(cx, past, 1, Modifiers::none());
    assert_eq!(caret(cx).0, Position::new(0, 129));
}

// --- The context menu request -----------------------------------------------

pub(super) fn right_click(cx: &mut VisualTestContext, at: Point<Pixels>) {
    cx.simulate_mouse_down(at, MouseButton::Right, Modifiers::none());
    cx.simulate_mouse_up(at, MouseButton::Right, Modifiers::none());
}

/// Collects the editor's events; the returned closure takes what arrived.
pub(super) fn record_events(
    editor: &gpui::Entity<Editor>,
    cx: &mut VisualTestContext,
) -> impl Fn(&mut VisualTestContext) -> Vec<EditorEvent> + use<> {
    let events = Rc::new(RefCell::new(Vec::new()));
    cx.update(|_, cx| {
        let events = events.clone();
        cx.subscribe(editor, move |_, event: &EditorEvent, _| {
            events.borrow_mut().push(event.clone());
        })
        .detach();
    });
    move |cx| {
        cx.run_until_parked();
        std::mem::take(&mut *events.borrow_mut())
    }
}

/// The one context menu request among `events`.
pub(super) fn request_in(events: &[EditorEvent]) -> ContextMenuRequest {
    let mut requests = events.iter().filter_map(|event| match event {
        EditorEvent::ContextMenuRequested(request) => Some(request.clone()),
        _ => None,
    });
    let request = requests.next().expect("a context menu request");
    assert!(requests.next().is_none(), "one request per click");
    request
}

fn context_document() -> Document {
    let link = InlineAttrs {
        link: Some("https://example.com/page".into()),
        ..InlineAttrs::default()
    };
    Document::from_blocks(vec![
        Block::paragraph("alpha beta gamma, delta"),
        Block::new(BlockKind::Paragraph)
            .with_span("see ", InlineAttrs::default())
            .with_span("the page", link)
            .with_span(" for more", InlineAttrs::default()),
        Block::new(BlockKind::Divider),
        Block::with_text(BlockKind::todo(), "buy milk"),
        Block::paragraph(""),
        Block::paragraph("last words here"),
    ])
}

#[gpui::test]
fn a_right_click_selects_the_word_and_asks_for_the_menu(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(context_document(), EditorConfig::full(), cx);
    let take = record_events(&editor, cx);
    let on = |cx: &mut VisualTestContext, block, offset| {
        // A little right of the boundary: on the character that starts there.
        editor.read_with(cx, |editor, _| {
            point_of(editor, block, offset) + point(px(3.), px(0.))
        })
    };

    // On a word: the word is selected, as in a macOS text view.
    let at = on(cx, 0, 7);
    right_click(cx, at);
    let events = take(cx);
    let request = request_in(&events);
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.state().selected_text(), "beta");
    });
    assert_eq!(request.position, at);
    assert_eq!(request.document_position, Position::new(0, 7));
    assert_eq!(request.block, 0);
    assert!(request.has_selection);
    assert_eq!(request.link, None);
    assert_eq!(request.link_range, None);
    assert!(request.comments.is_empty());
    assert!(!request.on_checkbox);
    assert!(!request.from_keyboard);
    // The selection change is announced before the request.
    assert_eq!(events[0], EditorEvent::SelectionChanged);
    assert!(matches!(
        events.last(),
        Some(EditorEvent::ContextMenuRequested(_))
    ));
    // Nothing was edited and no drag started.
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, EditorEvent::Edited { .. }))
    );
    editor.read_with(cx, |editor, _| assert!(!editor.mouse_busy()));
    let elsewhere = on(cx, 5, 3);
    cx.simulate_mouse_move(elsewhere, None, Modifiers::none());
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.state().selected_text(), "beta");
    });

    // Inside the selection: it is kept, and nothing but the request is
    // emitted.
    let at = on(cx, 0, 8);
    right_click(cx, at);
    let events = take(cx);
    let request = request_in(&events);
    assert_eq!(events.len(), 1);
    assert!(request.has_selection);
    editor.read_with(cx, |editor, _| {
        assert_eq!(
            editor.selection(),
            Selection::new(Position::new(0, 6), Position::new(0, 10))
        );
    });

    // On another word: the selection moves there.
    let at = on(cx, 0, 13);
    right_click(cx, at);
    assert!(request_in(&take(cx)).has_selection);
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.state().selected_text(), "gamma");
    });

    // On a space or a punctuation mark: only the caret.
    for (offset, caret) in [(10, 10), (16, 16)] {
        let at = on(cx, 0, offset);
        right_click(cx, at);
        let request = request_in(&take(cx));
        assert!(!request.has_selection, "offset {offset}");
        assert_eq!(request.document_position, Position::new(0, caret));
        editor.read_with(cx, |editor, _| {
            assert_eq!(
                editor.selection(),
                Selection::caret(Position::new(0, caret))
            );
        });
    }

    // A selection over several blocks is kept by a right-click anywhere
    // in it: on a character, past the end of a row, in the left margin.
    let whole = Selection::new(Position::new(0, 6), Position::new(5, 4));
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.set_selection(whole), cx)
    });
    take(cx);
    let viewport = editor.read_with(cx, |editor, _| editor.viewport_bounds().unwrap());
    let row_1 = on(cx, 1, 2);
    for at in [
        on(cx, 1, 6),
        point(viewport.right() - px(40.), row_1.y),
        point(viewport.left() + px(4.), row_1.y),
        on(cx, 5, 2),
    ] {
        right_click(cx, at);
        let events = take(cx);
        assert_eq!(events.len(), 1, "{at:?}");
        assert!(request_in(&events).has_selection);
        editor.read_with(cx, |editor, _| assert_eq!(editor.selection(), whole));
    }
    // Outside it (after its end, before its start) it is replaced.
    let at = on(cx, 5, 6);
    right_click(cx, at);
    take(cx);
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.state().selected_text(), "words");
    });
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.set_selection(whole), cx)
    });
    let at = on(cx, 0, 2);
    right_click(cx, at);
    take(cx);
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.state().selected_text(), "alpha");
    });
}

#[gpui::test]
fn a_right_click_reports_what_is_under_the_pointer(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(context_document(), EditorConfig::full(), cx);
    // Two comment threads on "gamma", one of them hidden; one on "beta".
    let (beta, gamma, hidden) = editor.update(cx, |editor, cx| {
        editor.edit(
            |state| {
                let mut comment = |from: usize, to: usize| {
                    state.set_selection(Selection::new(
                        Position::new(0, from),
                        Position::new(0, to),
                    ));
                    state.add_comment("note", "me", 0).unwrap()
                };
                let ids = (comment(6, 10), comment(11, 16), comment(11, 16));
                state.set_caret(Position::ZERO);
                ids
            },
            cx,
        )
    });
    editor.update(cx, |editor, cx| editor.set_hidden_comments([hidden], cx));
    let take = record_events(&editor, cx);
    let on = |cx: &mut VisualTestContext, block, offset| {
        editor.read_with(cx, |editor, _| {
            point_of(editor, block, offset) + point(px(3.), px(0.))
        })
    };

    // Commented text: the threads on that character, hidden ones left out.
    let at = on(cx, 0, 12);
    right_click(cx, at);
    let request = request_in(&take(cx));
    assert_eq!(request.comments, vec![gamma]);
    assert!(request.has_selection);
    let at = on(cx, 0, 7);
    right_click(cx, at);
    assert_eq!(request_in(&take(cx)).comments, vec![beta]);
    let at = on(cx, 0, 2);
    right_click(cx, at);
    assert!(request_in(&take(cx)).comments.is_empty());
    // The threads are reported whether or not the tints are drawn.
    editor.update(cx, |editor, cx| editor.set_comments_visible(false, cx));
    let at = on(cx, 0, 7);
    right_click(cx, at);
    assert_eq!(request_in(&take(cx)).comments, vec![beta]);

    // A link: its address and the whole linked text, on its first and on
    // its last character, and not on the characters around it.
    for offset in [4, 8, 11] {
        let at = on(cx, 1, offset);
        right_click(cx, at);
        let request = request_in(&take(cx));
        assert_eq!(
            request.link.as_deref(),
            Some("https://example.com/page"),
            "offset {offset}"
        );
        assert_eq!(
            request.link_range,
            Some(TextRange::in_block(1, 4, 12)),
            "offset {offset}"
        );
        assert_eq!(request.block, 1);
    }
    for offset in [2, 13] {
        let at = on(cx, 1, offset);
        right_click(cx, at);
        let request = request_in(&take(cx));
        assert_eq!(request.link, None, "offset {offset}");
        assert_eq!(request.link_range, None);
    }
    // A right-click never opens the link.
    assert_eq!(cx.opened_url(), None);

    // A divider: the caret goes there, nothing is selected.
    let divider = editor.read_with(cx, |editor, _| {
        editor.position_bounds(Position::new(2, 0)).unwrap()
    });
    let viewport = editor.read_with(cx, |editor, _| editor.viewport_bounds().unwrap());
    right_click(cx, point(viewport.center().x, divider.center().y));
    let request = request_in(&take(cx));
    assert_eq!(request.block, 2);
    assert!(!request.has_selection);
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.selection(), Selection::caret(Position::new(2, 0)));
    });

    // A to-do checkbox: reported, not toggled; the caret is in the item.
    let boxes = editor.read_with(cx, |editor, _| editor.checkbox_hits.clone());
    assert_eq!(boxes.len(), 1);
    right_click(cx, boxes[0].bounds.center());
    let events = take(cx);
    let request = request_in(&events);
    assert!(request.on_checkbox);
    assert_eq!(request.block, 3);
    assert_eq!(request.document_position, Position::new(3, 0));
    assert!(!request.has_selection);
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, EditorEvent::Edited { .. }))
    );
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().blocks()[3].kind(), &BlockKind::todo());
        assert_eq!(editor.selection(), Selection::caret(Position::new(3, 0)));
    });
    // On the item's text it is an ordinary right-click.
    let at = on(cx, 3, 5);
    right_click(cx, at);
    let request = request_in(&take(cx));
    assert!(!request.on_checkbox);
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.state().selected_text(), "milk");
    });

    // An empty paragraph and the space below the text: just the caret.
    let empty = editor.read_with(cx, |editor, _| point_of(editor, 4, 0));
    right_click(cx, empty + point(px(30.), px(0.)));
    let request = request_in(&take(cx));
    assert_eq!(request.document_position, Position::new(4, 0));
    assert!(!request.has_selection);
    right_click(cx, point(viewport.center().x, viewport.bottom() - px(20.)));
    let request = request_in(&take(cx));
    assert_eq!(request.block, 5);
    assert!(!request.has_selection);
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.selection(), Selection::caret(Position::new(5, 15)));
    });

    // Past the end of a row: the caret at its end, nothing selected.
    let row = on(cx, 0, 2);
    right_click(cx, point(viewport.right() - px(40.), row.y));
    let request = request_in(&take(cx));
    assert_eq!(request.document_position, Position::new(0, 23));
    assert!(!request.has_selection);
}

#[gpui::test]
fn a_right_click_focuses_and_works_read_only_and_on_sheets(cx: &mut TestAppContext) {
    setup(cx);
    let long = "A sentence that is long enough to wrap in the text area of a page. ".repeat(6);
    let blocks: Vec<Block> = (0..40).map(|_| Block::paragraph(long.trim_end())).collect();
    let (editor, cx) = open(Document::from_blocks(blocks), EditorConfig::full(), cx);
    let take = record_events(&editor, cx);

    // Page view, on a later sheet.
    editor.update(cx, |editor, cx| {
        editor.set_page_view(true, cx);
        editor.scroll_to_position(Position::new(20, 0), crate::Autoscroll::Center, cx);
    });
    cx.run_until_parked();
    take(cx);
    let scroll = editor.read_with(cx, |editor, _| editor.scroll_offset());
    let at = editor.read_with(cx, |editor, _| {
        point_of(editor, 20, 12) + point(px(3.), px(0.))
    });
    right_click(cx, at);
    let request = request_in(&take(cx));
    assert_eq!(request.document_position, Position::new(20, 12));
    assert_eq!(request.position, at);
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.state().selected_text(), "that");
        // The view does not move under the menu.
        assert_eq!(editor.scroll_offset(), scroll);
    });
    // On the desk beside the sheet: the caret, on the row at that height.
    let viewport = editor.read_with(cx, |editor, _| editor.viewport_bounds().unwrap());
    right_click(cx, point(viewport.left() + px(4.), at.y));
    let request = request_in(&take(cx));
    assert_eq!(request.block, 20);
    assert!(!request.has_selection);

    // Read-only: the word is still selected (it can be copied).
    editor.update(cx, |editor, cx| editor.set_read_only(true, cx));
    cx.run_until_parked();
    take(cx);
    right_click(cx, at);
    assert!(request_in(&take(cx)).has_selection);
    editor.update(cx, |editor, cx| editor.set_read_only(false, cx));

    // The scroll bar strip is not text.
    right_click(cx, point(viewport.right() - px(4.), viewport.center().y));
    assert!(take(cx).is_empty());

    // A right-click brings the keyboard to the editor.
    cx.update(|window, _| window.blur());
    cx.run_until_parked();
    take(cx);
    editor.update_in(cx, |editor, window, _| assert!(!editor.is_focused(window)));
    right_click(cx, at);
    let events = take(cx);
    request_in(&events);
    assert!(events.contains(&EditorEvent::Focused));
    editor.update_in(cx, |editor, window, _| assert!(editor.is_focused(window)));

    // While the left button is dragging, the right one is ignored.
    cx.simulate_mouse_down(at, MouseButton::Left, Modifiers::none());
    take(cx);
    cx.simulate_mouse_down(at, MouseButton::Right, Modifiers::none());
    assert!(
        !take(cx)
            .iter()
            .any(|event| matches!(event, EditorEvent::ContextMenuRequested(_)))
    );
    cx.simulate_mouse_up(at, MouseButton::Left, Modifiers::none());
}

#[gpui::test]
fn ctrl_click_is_a_right_click(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(context_document(), EditorConfig::full(), cx);
    let take = record_events(&editor, cx);
    // GPUI's macOS platform reports a ctrl-click as the right button; a
    // left press that still carries ctrl is taken the same way and does
    // not start a drag selection.
    let at = editor.read_with(cx, |editor, _| {
        point_of(editor, 0, 7) + point(px(3.), px(0.))
    });
    click(cx, at, 1, Modifiers::control());
    let request = request_in(&take(cx));
    assert!(request.has_selection);
    assert!(!request.from_keyboard);
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.state().selected_text(), "beta");
        assert!(!editor.mouse_busy());
    });
}

#[gpui::test]
fn ctrl_enter_asks_for_the_menu_at_the_caret(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(context_document(), EditorConfig::full(), cx);
    let comment = editor.update(cx, |editor, cx| {
        editor.edit(
            |state| {
                state.set_selection(Selection::new(Position::new(0, 6), Position::new(0, 10)));
                let id = state.add_comment("note", "me", 0).unwrap();
                state.set_caret(Position::new(0, 8));
                id
            },
            cx,
        )
    });
    let take = record_events(&editor, cx);

    // At the caret: under its row, at its x.
    cx.simulate_keystrokes("ctrl-enter");
    let events = take(cx);
    let request = request_in(&events);
    assert_eq!(events.len(), 1, "the selection is left alone");
    let caret = editor.read_with(cx, |editor, _| editor.caret_bounds().unwrap());
    assert_eq!(request.position, point(caret.left(), caret.bottom()));
    assert_eq!(request.document_position, Position::new(0, 8));
    assert_eq!(request.block, 0);
    assert!(request.from_keyboard);
    assert!(!request.has_selection);
    assert!(!request.on_checkbox);
    assert_eq!(request.comments, vec![comment]);
    assert_eq!(request.link, None);

    // With a selection: it is kept and reported.
    cx.simulate_keystrokes("shift-right shift-right");
    take(cx);
    cx.simulate_keystrokes("ctrl-enter");
    let request = request_in(&take(cx));
    assert!(request.has_selection);
    editor.read_with(cx, |editor, _| {
        assert_eq!(
            editor.selection(),
            Selection::new(Position::new(0, 8), Position::new(0, 10))
        );
    });

    // In a link, and at either end of it.
    for offset in [4, 8, 12] {
        editor.update(cx, |editor, cx| {
            editor.edit(|state| state.set_caret(Position::new(1, offset)), cx)
        });
        take(cx);
        cx.simulate_keystrokes("ctrl-enter");
        let request = request_in(&take(cx));
        assert_eq!(request.link.as_deref(), Some("https://example.com/page"));
        assert_eq!(request.link_range, Some(TextRange::in_block(1, 4, 12)));
    }

    // The action is listed as a command and its key is in the keymap.
    assert!(
        crate::commands()
            .iter()
            .any(|command| command.title == "Context Menu")
    );

    // Read-only editors answer too.
    editor.update(cx, |editor, cx| editor.set_read_only(true, cx));
    cx.run_until_parked();
    take(cx);
    cx.simulate_keystrokes("ctrl-enter");
    request_in(&take(cx));
}

#[gpui::test]
fn ctrl_enter_far_from_the_viewport_scrolls_to_the_caret(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(paragraphs(300), EditorConfig::full(), cx);
    let take = record_events(&editor, cx);
    let viewport = editor.read_with(cx, |editor, _| editor.viewport_bounds().unwrap());
    // The caret is at the start; scroll it far out of view.
    cx.simulate_event(ScrollWheelEvent {
        position: viewport.center(),
        delta: ScrollDelta::Pixels(point(px(0.), px(-3000.))),
        modifiers: Modifiers::none(),
        touch_phase: TouchPhase::Moved,
    });
    take(cx);
    cx.simulate_keystrokes("ctrl-enter");
    let request = request_in(&take(cx));
    assert!(viewport.contains(&request.position), "{request:?}");
    let caret = editor.read_with(cx, |editor, _| editor.caret_bounds().unwrap());
    assert_eq!(request.position, point(caret.left(), caret.bottom()));
    assert!(editor.read_with(cx, |editor, _| editor.scroll_offset()) < px(100.));
}

// --- The link tooltip -------------------------------------------------------

#[gpui::test]
fn resting_on_a_link_shows_its_address(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(context_document(), EditorConfig::full(), cx);
    let on = |cx: &mut VisualTestContext, block, offset| {
        editor.read_with(cx, |editor, _| {
            point_of(editor, block, offset) + point(px(3.), px(0.))
        })
    };
    let tip = |cx: &mut VisualTestContext| {
        cx.run_until_parked();
        editor.read_with(cx, |editor, cx| editor.link_tooltip(cx))
    };
    let rest = |cx: &mut VisualTestContext| {
        cx.executor()
            .advance_clock(LINK_TIP_DELAY + Duration::from_millis(20));
        cx.run_until_parked();
    };
    let (on_link, further, off_link) = (on(cx, 1, 6), on(cx, 1, 10), on(cx, 1, 15));

    // Not at once, and not at all when the pointer only passes over.
    cx.simulate_mouse_move(on_link, None, Modifiers::none());
    assert_eq!(tip(cx), None);
    cx.simulate_mouse_move(off_link, None, Modifiers::none());
    rest(cx);
    assert_eq!(tip(cx), None);

    // After a moment on the link: the address and how to open it, under
    // the row, inside the window.
    cx.simulate_mouse_move(on_link, None, Modifiers::none());
    rest(cx);
    assert_eq!(
        tip(cx).as_deref(),
        Some("https://example.com/page \u{2318}-click to open")
    );
    let row = editor.read_with(cx, |editor, _| {
        editor.position_bounds(Position::new(1, 6)).unwrap()
    });
    let panel = cx.debug_bounds("link-tip").expect("the tooltip is drawn");
    assert!(panel.top() >= row.bottom(), "{panel:?} under {row:?}");
    assert!(panel.top() - row.bottom() < px(8.));
    assert!(panel.left() >= on_link.x && panel.left() - on_link.x < px(4.));
    assert!(panel.size.width > px(200.));
    // It stays while the pointer moves along the same link, and does not
    // follow it.
    cx.simulate_mouse_move(further, None, Modifiers::none());
    assert!(tip(cx).is_some());
    assert_eq!(cx.debug_bounds("link-tip").unwrap(), panel);
    // The cursor only turns into a hand with cmd held; the tooltip does
    // not need it.
    editor.read_with(cx, |editor, _| assert!(!editor.link_hover));

    // Leaving the link takes it away.
    cx.simulate_mouse_move(off_link, None, Modifiers::none());
    assert_eq!(tip(cx), None);

    // So do a click, the wheel, typing and moving the caret.
    let show = |cx: &mut VisualTestContext| {
        cx.simulate_mouse_move(off_link, None, Modifiers::none());
        cx.simulate_mouse_move(on_link, None, Modifiers::none());
        rest(cx);
        assert!(tip(cx).is_some());
    };
    show(cx);
    click(cx, on_link, 1, Modifiers::none());
    assert_eq!(tip(cx), None);
    // (The pointer has to move again before it comes back.)
    rest(cx);
    assert_eq!(tip(cx), None);
    show(cx);
    right_click(cx, on_link);
    assert_eq!(tip(cx), None);
    show(cx);
    cx.simulate_event(ScrollWheelEvent {
        position: on_link,
        delta: ScrollDelta::Pixels(point(px(0.), px(-10.))),
        modifiers: Modifiers::none(),
        touch_phase: TouchPhase::Moved,
    });
    assert_eq!(tip(cx), None);
    show(cx);
    cx.simulate_keystrokes("right");
    assert_eq!(tip(cx), None);
    show(cx);
    cx.simulate_input("x");
    assert_eq!(tip(cx), None);

    // And another document.
    show(cx);
    editor.update(cx, |editor, cx| {
        editor.set_document(paragraphs(3), cx);
    });
    assert_eq!(tip(cx), None);
}

#[gpui::test]
fn the_link_tooltip_shows_on_a_sheet_and_not_under_a_popup(cx: &mut TestAppContext) {
    setup(cx);
    let mut blocks = context_document().blocks().to_vec();
    blocks.extend(paragraphs(200).blocks().iter().cloned());
    let (editor, cx) = open(Document::from_blocks(blocks), EditorConfig::full(), cx);
    editor.update(cx, |editor, cx| editor.set_page_view(true, cx));
    cx.run_until_parked();
    let on_link = editor.read_with(cx, |editor, _| {
        point_of(editor, 1, 6) + point(px(3.), px(0.))
    });
    let off_link = editor.read_with(cx, |editor, _| {
        point_of(editor, 1, 15) + point(px(3.), px(0.))
    });
    let rest = |cx: &mut VisualTestContext| {
        cx.executor()
            .advance_clock(LINK_TIP_DELAY + Duration::from_millis(20));
        cx.run_until_parked();
    };
    cx.simulate_mouse_move(on_link, None, Modifiers::none());
    rest(cx);
    editor.read_with(cx, |editor, cx| assert!(editor.link_tooltip(cx).is_some()));
    assert!(cx.debug_bounds("link-tip").is_some());

    // The text scrolls away under the resting pointer (a key, a command):
    // the frame that shows it has no tooltip.
    editor.update(cx, |editor, cx| editor.scroll_by_pixels(px(80.), cx));
    cx.run_until_parked();
    editor.read_with(cx, |editor, cx| assert_eq!(editor.link_tooltip(cx), None));
    editor.update(cx, |editor, cx| editor.scroll_by_pixels(px(-80.), cx));
    cx.run_until_parked();
    editor.read_with(cx, |editor, cx| assert_eq!(editor.link_tooltip(cx), None));

    // While a popup is open over the text, no tooltip comes up.
    cx.simulate_mouse_move(off_link, None, Modifiers::none());
    let popup = cx.update(|_, cx| gpui::AppContext::new(cx, |_| gpui::Empty));
    editor.update(cx, |editor, cx| {
        editor.show_popup(
            popup.into(),
            crate::PopupAnchor::Caret,
            crate::PopupPlacement::Below,
            cx,
        );
    });
    cx.run_until_parked();
    cx.simulate_mouse_move(on_link, None, Modifiers::none());
    rest(cx);
    editor.read_with(cx, |editor, cx| assert_eq!(editor.link_tooltip(cx), None));
}

// --- The mouse while a modal key layer (Vim) is attached --------------------

/// Everything a non-keyboard user does in the text, with the editor set up
/// the way the Vim glue sets it up in Normal mode: a block caret, no text
/// input, every key without `cmd` consumed by an interceptor. In flow view
/// and on sheets.
#[gpui::test]
fn the_mouse_works_under_a_modal_key_layer(cx: &mut TestAppContext) {
    for (page_view, vim) in [(false, false), (false, true), (true, false), (true, true)] {
        let case = format!("page view {page_view}, modal keys {vim}");
        setup(cx);
        let link = InlineAttrs {
            link: Some("https://example.com/page".into()),
            ..InlineAttrs::default()
        };
        let mut blocks = vec![
            Block::paragraph("alpha beta gamma"),
            Block::with_text(BlockKind::todo(), "buy milk"),
            Block::new(BlockKind::Paragraph)
                .with_span("see ", InlineAttrs::default())
                .with_span("the page", link)
                .with_span(" for more", InlineAttrs::default()),
        ];
        blocks.extend(paragraphs(300).blocks().iter().cloned());
        let (editor, cx) = open(Document::from_blocks(blocks), EditorConfig::full(), cx);
        let consumed = Rc::new(RefCell::new(Vec::new()));
        editor.update(cx, |editor, cx| {
            editor.set_page_view(page_view, cx);
            if vim {
                editor.set_caret_style(crate::CaretStyle::Block, cx);
                editor.set_text_input_enabled(false, cx);
                let consumed = consumed.clone();
                editor
                    .intercept_keys(crate::InterceptPriority::MODAL, move |keystroke, _, _| {
                        if keystroke.modifiers.platform {
                            return crate::KeyDisposition::Pass;
                        }
                        consumed.borrow_mut().push(keystroke.unparse());
                        crate::KeyDisposition::Consumed
                    })
                    .detach();
            }
        });
        cx.run_until_parked();
        let take = record_events(&editor, cx);
        let on = |cx: &mut VisualTestContext, block, offset| {
            editor.read_with(cx, |editor, _| {
                point_of(editor, block, offset) + point(px(3.), px(0.))
            })
        };
        let selection =
            |cx: &mut VisualTestContext| editor.read_with(cx, |editor, _| editor.selection());
        let selected = |cx: &mut VisualTestContext| {
            editor.read_with(cx, |editor, _| editor.state().selected_text())
        };

        // Click and shift-click.
        let (a, b) = (on(cx, 0, 2), on(cx, 0, 8));
        click(cx, a, 1, Modifiers::none());
        assert_eq!(
            selection(cx),
            Selection::caret(Position::new(0, 2)),
            "{case}"
        );
        click(cx, b, 1, Modifiers::shift());
        assert_eq!(
            selection(cx),
            Selection::new(Position::new(0, 2), Position::new(0, 8)),
            "{case}"
        );
        // Drag over two blocks.
        let (from, to) = (on(cx, 0, 6), on(cx, 2, 3));
        cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(to, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_up(to, MouseButton::Left, Modifiers::none());
        assert_eq!(
            selection(cx),
            Selection::new(Position::new(0, 6), Position::new(2, 3)),
            "{case}"
        );
        // Double and triple click.
        let word = on(cx, 0, 7);
        click(cx, word, 2, Modifiers::none());
        assert_eq!(selected(cx), "beta", "{case}");
        click(cx, word, 3, Modifiers::none());
        assert_eq!(selected(cx), "alpha beta gamma", "{case}");
        // The to-do checkbox.
        let boxes = editor.read_with(cx, |editor, _| editor.checkbox_hits.clone());
        assert_eq!(boxes.len(), 1, "{case}");
        click(cx, boxes[0].bounds.center(), 1, Modifiers::none());
        editor.read_with(cx, |editor, _| {
            assert_eq!(
                editor.document().blocks()[1].kind(),
                &BlockKind::TodoItem { checked: true },
                "{case}"
            );
        });
        // cmd-click on a link; resting on it.
        let on_link = on(cx, 2, 6);
        click(cx, on_link, 1, Modifiers::command());
        assert_eq!(
            cx.opened_url().as_deref(),
            Some("https://example.com/page"),
            "{case}"
        );
        cx.simulate_mouse_move(on_link, None, Modifiers::none());
        cx.executor()
            .advance_clock(LINK_TIP_DELAY + Duration::from_millis(20));
        cx.run_until_parked();
        editor.read_with(cx, |editor, cx| {
            assert!(editor.link_tooltip(cx).is_some(), "{case}")
        });
        // The right button: inside the selection (the triple-clicked
        // block) it keeps it; after a click elsewhere it takes the word.
        take(cx);
        right_click(cx, word);
        let request = request_in(&take(cx));
        assert!(request.has_selection, "{case}");
        assert_eq!(selected(cx), "alpha beta gamma", "{case}");
        click(cx, a, 1, Modifiers::none());
        take(cx);
        right_click(cx, word);
        let request = request_in(&take(cx));
        assert!(request.has_selection, "{case}");
        assert_eq!(selected(cx), "beta", "{case}");
        // The wheel.
        let viewport = editor.read_with(cx, |editor, _| editor.viewport_bounds().unwrap());
        cx.simulate_event(ScrollWheelEvent {
            position: viewport.center(),
            delta: ScrollDelta::Pixels(point(px(0.), px(-300.))),
            modifiers: Modifiers::none(),
            touch_phase: TouchPhase::Moved,
        });
        editor.read_with(cx, |editor, _| {
            assert_eq!(editor.scroll_offset(), px(300.), "{case}")
        });
        // A drag that leaves the viewport scrolls and keeps selecting.
        let start = point(viewport.center().x - px(100.), viewport.center().y);
        cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
        let anchor = selection(cx).anchor;
        cx.simulate_mouse_move(
            point(start.x, viewport.bottom() + px(60.)),
            MouseButton::Left,
            Modifiers::none(),
        );
        let first = selection(cx).head;
        for _ in 0..10 {
            cx.executor().advance_clock(Duration::from_millis(35));
        }
        editor.read_with(cx, |editor, _| {
            assert!(editor.scroll_offset() > px(400.), "{case}");
            assert!(editor.selection().head > first, "{case}");
            assert_eq!(editor.selection().anchor, anchor, "{case}");
        });
        cx.simulate_mouse_up(start, MouseButton::Left, Modifiers::none());

        // The keyboard path to the menu is the modal layer's to pass on.
        take(cx);
        cx.simulate_keystrokes("ctrl-enter");
        let asked = take(cx)
            .iter()
            .any(|event| matches!(event, EditorEvent::ContextMenuRequested(_)));
        assert_eq!(asked, !vim, "{case}");
        assert_eq!(consumed.borrow().len(), usize::from(vim), "{case}");
    }
}
