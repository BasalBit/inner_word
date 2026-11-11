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

    // 80 cards do not fit; the far ones are not drawn at all. (The test
    // platform keeps the bounds of the last frame that drew an element,
    // so only cards that must be on screen are asked for.)
    let last = *ids.last().unwrap();

    cx.simulate_keystrokes("end");
    assert_eq!(f.active(cx), Some(last));
    assert!(
        contains(list, thread_bounds(last, cx)),
        "the active thread is scrolled into view"
    );

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

    // Step by step, down and up again, the active card is always whole:
    // also the one that grows by its row of key hints when it gets active.
    cx.simulate_keystrokes("cmd-2");
    for (step, id) in ids.iter().enumerate().take(50).skip(31) {
        cx.simulate_keystrokes("j");
        assert_eq!(f.active(cx), Some(*id));
        assert!(contains(list, thread_bounds(*id, cx)), "down to {step}");
    }
    for (step, id) in ids.iter().enumerate().take(49).skip(20).rev() {
        cx.simulate_keystrokes("k");
        assert!(contains(list, thread_bounds(*id, cx)), "up to {step}");
    }
    // A reply composer that opens in the last card stays in view.
    cx.simulate_keystrokes("end r");
    assert_eq!(f.focus(cx), Focus::Composer);
    cx.run_until_parked();
    assert!(contains(list, bounds("comment-composer".into(), cx)));
}

/// The list is virtualised: with a thousand threads only the cards near
/// the visible part are built. (Before, every card was laid out for every
/// frame: 40 steps through this list took 13 s in a debug build, now 0.2 s.)
#[gpui::test]
fn a_thousand_threads_cost_no_more_than_a_screenful(cx: &mut TestAppContext) {
    let (f, cx) = open(many_comments(1000), cx);
    let ids = ids(&f, cx);
    assert_eq!(ids.len(), 1000);
    cx.simulate_keystrokes("cmd-2");
    let list = bounds("comment-threads".into(), cx);
    let built = |cx: &mut VisualTestContext| f.panel.read_with(cx, |panel, _| panel.rows_built());
    assert!(contains(list, thread_bounds(ids[0], cx)));
    let so_far = built(cx);
    assert!(
        so_far < 400,
        "{so_far} cards were built to show the first screenful of 1000"
    );

    let started = std::time::Instant::now();
    for id in ids.iter().take(41).skip(1) {
        cx.simulate_keystrokes("j");
        assert!(contains(list, thread_bounds(*id, cx)));
    }
    let stepping = started.elapsed();
    let per_step = (built(cx) - so_far) / 40;
    assert!(
        per_step < 150,
        "{per_step} cards built per step through a list of 1000"
    );
    // The far end and back: no walk through the cards in between.
    let so_far = built(cx);
    cx.simulate_keystrokes("end");
    assert_eq!(f.active(cx), Some(ids[999]));
    assert!(contains(list, thread_bounds(ids[999], cx)));
    cx.simulate_keystrokes("home");
    assert!(contains(list, thread_bounds(ids[0], cx)));
    assert!(
        built(cx) - so_far < 600,
        "{} cards built",
        built(cx) - so_far
    );

    // Resolving in the middle of the list keeps the place.
    cx.simulate_keystrokes("escape");
    f.caret(500, 3, cx);
    cx.simulate_keystrokes("cmd-2 x");
    assert_eq!(f.listed_ids(cx).len(), 999);
    assert_eq!(f.active(cx), Some(ids[501]));
    assert!(contains(list, thread_bounds(ids[501], cx)));
    eprintln!("1000 threads: 40 steps through the list took {stepping:?}");
    // Generous: a debug build on a busy machine. It was 13 s.
    assert!(stepping < std::time::Duration::from_secs(5));
}
