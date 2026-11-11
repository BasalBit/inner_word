//! Where things end up on screen (test platform metrics: every glyph is
//! 0.6 × font size wide, so the numbers are deterministic, not real).

use gpui::{Bounds, Pixels, TestAppContext, VisualTestContext};
use writer_core::{Block, CommentId, Document, TextRange};

use super::support::{AUTHOR, Fixture, Focus, NOW, SLOT_WIDTH, open, sample};

fn bounds(selector: String, cx: &mut VisualTestContext) -> Bounds<Pixels> {
    // `debug_bounds` wants a `&'static str`; a test may leak a few names.
    let selector: &'static str = Box::leak(selector.into_boxed_str());
    cx.debug_bounds(selector)
        .unwrap_or_else(|| panic!("nothing was painted for {selector}"))
}

fn thread_bounds(id: CommentId, cx: &mut VisualTestContext) -> Bounds<Pixels> {
    bounds(format!("comment-thread-{}", id.0), cx)
}

fn contains(outer: Bounds<Pixels>, inner: Bounds<Pixels>) -> bool {
    outer.left() <= inner.left()
        && inner.right() <= outer.right()
        && outer.top() <= inner.top()
        && inner.bottom() <= outer.bottom()
}

#[gpui::test]
fn the_panel_sits_right_of_the_editor(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    let viewport = cx.update(|window, _| window.viewport_size());

    // The new-comment composer is under the header, above the list.
    f.select(0, 4, 9, cx);
    cx.simulate_keystrokes("cmd-alt-m");
    let panel = bounds("comments-panel".into(), cx);
    // The panel fills the container the workspace gives it.
    assert_eq!(panel, bounds("panel-slot".into(), cx));
    assert_eq!(panel.size.width, SLOT_WIDTH);
    assert_eq!(panel.right(), viewport.width);
    assert_eq!(panel.top(), gpui::px(0.));
    assert_eq!(panel.size.height, viewport.height);
    let composer = bounds("comment-composer".into(), cx);
    assert!(contains(panel, composer));
    assert!(composer.size.width > gpui::px(240.));
    assert!(composer.size.height >= gpui::px(18.));
    let one_line = composer.size.height;

    // The composer grows with its text, up to a limit.
    cx.simulate_input("one");
    cx.simulate_keystrokes("shift-enter");
    cx.simulate_input("two");
    cx.simulate_keystrokes("shift-enter");
    cx.simulate_input("three");
    let three_lines = bounds("comment-composer".into(), cx).size.height;
    assert!(three_lines > one_line * 2.);
    for _ in 0..30 {
        cx.simulate_keystrokes("shift-enter");
    }
    let capped = bounds("comment-composer".into(), cx).size.height;
    assert!(capped > three_lines);
    assert!(
        capped <= gpui::px(180.),
        "the composer stops growing: {capped:?}"
    );
    cx.simulate_keystrokes("enter");

    // The thread card is inside the list, below the header.
    let id = f.active(cx).unwrap();
    let list = bounds("comment-threads".into(), cx);
    let card = thread_bounds(id, cx);
    assert!(contains(panel, list));
    assert!(contains(list, card));
    assert!(list.top() >= gpui::px(36.));
    assert!(card.size.height > gpui::px(40.));

    // A reply is inside the card; the reply composer too.
    cx.simulate_keystrokes("cmd-2 r");
    cx.simulate_input("A reply");
    let card_with_composer = thread_bounds(id, cx);
    let composer = bounds("comment-composer".into(), cx);
    assert!(contains(card_with_composer, composer));
    assert!(card_with_composer.size.height > card.size.height);
    cx.simulate_keystrokes("enter");
    let reply = bounds(format!("comment-reply-{}-1", id.0), cx);
    let card = thread_bounds(id, cx);
    assert!(contains(card, reply));
    assert!(reply.left() > card.left(), "replies are indented");

    // The delete confirmation appears inside the card.
    cx.simulate_keystrokes("cmd-backspace");
    let confirmation = bounds("comment-delete-confirmation".into(), cx);
    assert!(contains(thread_bounds(id, cx), confirmation));
    cx.simulate_keystrokes("escape");
}

/// A document of `count` short paragraphs, each with one comment thread.
fn many_comments(count: usize) -> Document {
    let mut state = writer_core::EditorState::new(Document::from_blocks(
        (0..count)
            .map(|ix| Block::paragraph(&format!("Paragraph number {ix} has a comment.")))
            .collect(),
    ));
    for ix in 0..count {
        state.select_range(TextRange::in_block(ix, 0, 9));
        state
            .add_comment(&format!("Comment {ix}"), AUTHOR, NOW)
            .unwrap();
    }
    state.into_document()
}

fn ids(f: &Fixture, cx: &mut VisualTestContext) -> Vec<CommentId> {
    f.listed_ids(cx)
}

#[gpui::test]
fn the_list_scrolls_to_the_active_thread(cx: &mut TestAppContext) {
    let (f, cx) = open(many_comments(80), cx);
    let ids = ids(&f, cx);
    assert_eq!(ids.len(), 80);
    cx.simulate_keystrokes("cmd-2");
    assert_eq!(f.focus(cx), Focus::List);
    let list = bounds("comment-threads".into(), cx);

    // 80 cards do not fit: the last one starts below the list.
    let last = *ids.last().unwrap();
    assert!(thread_bounds(last, cx).top() > list.bottom());

    cx.simulate_keystrokes("end");
    assert_eq!(f.active(cx), Some(last));
    assert!(
        contains(list, thread_bounds(last, cx)),
        "the active thread is scrolled into view"
    );
    assert!(thread_bounds(ids[0], cx).bottom() < list.top());

    cx.simulate_keystrokes("home");
    assert!(contains(list, thread_bounds(ids[0], cx)));

    // Going to a comment from the text scrolls the list as well.
    cx.simulate_keystrokes("escape");
    f.caret(60, 20, cx);
    cx.simulate_keystrokes("cmd-alt-]");
    assert_eq!(f.active(cx), Some(ids[61]));
    assert!(contains(list, thread_bounds(ids[61], cx)));
    // And so does a caret that walks into commented text.
    f.caret(30, 3, cx);
    assert_eq!(f.active(cx), Some(ids[30]));
    cx.run_until_parked();
    assert!(contains(list, thread_bounds(ids[30], cx)));
}
