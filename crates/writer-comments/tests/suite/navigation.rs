//! Moving between comments: from the editor with next / previous, in the
//! panel with the arrow keys, and the caret marking the thread it is in.

use gpui::TestAppContext;
use writer_comments::CommentsEvent;
use writer_core::{CommentId, Position, Selection};

use super::support::{Fixture, Focus, open, sample};

/// Three threads, one per paragraph: "quick", "second", "third".
fn three(f: &Fixture, cx: &mut gpui::VisualTestContext) -> [CommentId; 3] {
    let quick = f.comment(0, 4, 9, "On quick", cx);
    let second = f.comment(1, 2, 8, "On second", cx);
    let third = f.comment(2, 4, 9, "On third", cx);
    [quick, second, third]
}

fn caret_at(block: usize, offset: usize) -> Selection {
    Selection::caret(Position::new(block, offset))
}

#[gpui::test]
fn next_and_previous_comment_from_the_editor(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    let [quick, second, third] = three(&f, cx);
    f.caret(0, 0, cx);
    assert_eq!(f.active(cx), None);

    // Forward: the caret lands on each anchor's start, the thread is
    // active in the panel and in the editor, focus stays in the text.
    cx.simulate_keystrokes("cmd-alt-]");
    assert_eq!(f.selection(cx), caret_at(0, 4));
    assert_eq!(f.active(cx), Some(quick));
    assert_eq!(f.focus(cx), Focus::Editor);
    cx.simulate_keystrokes("cmd-alt-]");
    assert_eq!(f.selection(cx), caret_at(1, 2));
    assert_eq!(f.active(cx), Some(second));
    cx.simulate_keystrokes("cmd-alt-]");
    assert_eq!(f.selection(cx), caret_at(2, 4));
    assert_eq!(f.active(cx), Some(third));
    // Wraps to the first.
    cx.simulate_keystrokes("cmd-alt-]");
    assert_eq!(f.selection(cx), caret_at(0, 4));
    assert_eq!(f.active(cx), Some(quick));

    // Backward, wrapping to the last.
    cx.simulate_keystrokes("cmd-alt-[");
    assert_eq!(f.selection(cx), caret_at(2, 4));
    assert_eq!(f.active(cx), Some(third));
    cx.simulate_keystrokes("cmd-alt-[");
    assert_eq!(f.selection(cx), caret_at(1, 2));
    assert_eq!(f.active(cx), Some(second));
    assert_eq!(f.focus(cx), Focus::Editor);

    // From a caret between comments, not standing in any.
    f.caret(1, 30, cx);
    assert_eq!(f.active(cx), None);
    cx.simulate_keystrokes("cmd-alt-[");
    assert_eq!(f.selection(cx), caret_at(1, 2));
    assert_eq!(f.active(cx), Some(second));
    f.caret(1, 30, cx);
    cx.simulate_keystrokes("cmd-alt-]");
    assert_eq!(f.selection(cx), caret_at(2, 4));
    assert_eq!(f.active(cx), Some(third));

    // The panel reported every change of the active thread.
    let events = f.events(cx);
    assert!(events.contains(&CommentsEvent::ActiveThreadChanged(Some(third))));
    assert!(events.contains(&CommentsEvent::ActiveThreadChanged(None)));
}

#[gpui::test]
fn navigation_skips_threads_the_panel_does_not_list(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    let [quick, second, third] = three(&f, cx);
    // Resolve "second" from the list.
    f.caret(1, 4, cx);
    cx.simulate_keystrokes("cmd-2 x escape");
    assert_eq!(f.listed_ids(cx), vec![quick, third]);

    f.caret(0, 0, cx);
    cx.simulate_keystrokes("cmd-alt-] cmd-alt-]");
    assert_eq!(f.active(cx), Some(third));
    assert_eq!(f.selection(cx), caret_at(2, 4));
    cx.simulate_keystrokes("cmd-alt-[");
    assert_eq!(f.active(cx), Some(quick));

    // With resolved threads shown, it is a stop again.
    f.panel.update_in(cx, |panel, window, cx| {
        panel.set_show_resolved(true, window, cx);
    });
    cx.simulate_keystrokes("cmd-alt-]");
    assert_eq!(f.active(cx), Some(second));
    assert_eq!(f.selection(cx), caret_at(1, 2));

    // A caret inside a hidden resolved thread marks nothing.
    f.panel.update_in(cx, |panel, window, cx| {
        panel.set_show_resolved(false, window, cx);
    });
    f.caret(1, 5, cx);
    assert_eq!(f.active(cx), None);
}

#[gpui::test]
fn overlapping_threads_are_all_reachable(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    // Two threads that start at the same place, one inside the other.
    let word = f.comment(0, 4, 9, "The word", cx);
    let phrase = f.comment(0, 4, 19, "The phrase", cx);
    let third = f.comment(2, 4, 9, "On third", cx);
    let listed = f.listed_ids(cx);
    assert_eq!(listed.len(), 3);
    assert_eq!(listed[2], third);
    assert!(listed[..2].contains(&word) && listed[..2].contains(&phrase));

    f.caret(0, 0, cx);
    cx.simulate_keystrokes("cmd-alt-]");
    assert_eq!(f.active(cx), Some(listed[0]));
    cx.simulate_keystrokes("cmd-alt-]");
    assert_eq!(f.active(cx), Some(listed[1]));
    assert_eq!(f.selection(cx), caret_at(0, 4));
    cx.simulate_keystrokes("cmd-alt-]");
    assert_eq!(f.active(cx), Some(third));
    cx.simulate_keystrokes("cmd-alt-[");
    assert_eq!(f.active(cx), Some(listed[1]));
    cx.simulate_keystrokes("cmd-alt-[");
    assert_eq!(f.active(cx), Some(listed[0]));

    // A caret that moves inside the overlap keeps the active thread.
    cx.simulate_keystrokes("right right");
    assert_eq!(f.active(cx), Some(listed[0]));
    // Past the end of the word, only the phrase is under the caret.
    f.caret(0, 14, cx);
    assert_eq!(f.active(cx), Some(phrase));
}

#[gpui::test]
fn the_caret_marks_the_thread_it_enters(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    let [quick, second, _] = three(&f, cx);
    f.caret(0, 0, cx);
    assert_eq!(f.active(cx), None);

    // Arrow keys into "quick" (0..4 is "The ").
    cx.simulate_keystrokes("right right right");
    assert_eq!(f.active(cx), None);
    cx.simulate_keystrokes("right");
    assert_eq!(f.active(cx), Some(quick));
    cx.simulate_keystrokes("right right right right right");
    assert_eq!(f.selection(cx), caret_at(0, 9));
    assert_eq!(
        f.active(cx),
        Some(quick),
        "the end of the anchor still counts"
    );
    cx.simulate_keystrokes("right");
    assert_eq!(f.active(cx), None);

    f.caret(1, 5, cx);
    assert_eq!(f.active(cx), Some(second));
    f.caret(1, 20, cx);
    assert_eq!(f.active(cx), None);

    // Typing inside commented text keeps the thread active and its quote
    // follows the text.
    f.caret(0, 9, cx);
    cx.simulate_keystrokes("left");
    cx.simulate_input("l");
    assert_eq!(f.active(cx), Some(quick));
    f.panel.read_with(cx, |panel, _| {
        assert_eq!(panel.thread(quick).unwrap().quote, "quiclk");
    });
}

#[gpui::test]
fn arrow_keys_move_between_threads_in_the_panel(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    let [quick, second, third] = three(&f, cx);
    // The caret is between comments: focusing the panel picks the next
    // thread after it.
    f.caret(0, 20, cx);
    assert_eq!(f.active(cx), None);
    cx.simulate_keystrokes("cmd-2");
    assert_eq!(f.focus(cx), Focus::List);
    assert_eq!(f.active(cx), Some(second));

    cx.simulate_keystrokes("down");
    assert_eq!(f.active(cx), Some(third));
    cx.simulate_keystrokes("down");
    assert_eq!(f.active(cx), Some(third), "no wrap in the list");
    cx.simulate_keystrokes("up up");
    assert_eq!(f.active(cx), Some(quick));
    cx.simulate_keystrokes("k");
    assert_eq!(f.active(cx), Some(quick));
    cx.simulate_keystrokes("j");
    assert_eq!(f.active(cx), Some(second));
    cx.simulate_keystrokes("end");
    assert_eq!(f.active(cx), Some(third));
    cx.simulate_keystrokes("home");
    assert_eq!(f.active(cx), Some(quick));
    cx.simulate_keystrokes("ctrl-n ctrl-p cmd-down");
    assert_eq!(f.active(cx), Some(third));

    // The panel leads: the caret in the text did not move.
    assert_eq!(f.selection(cx), caret_at(0, 20));
    assert_eq!(f.focus(cx), Focus::List);

    // Next / previous comment work from the list too and do move the caret.
    cx.simulate_keystrokes("cmd-alt-]");
    assert_eq!(f.active(cx), Some(second));
    assert_eq!(f.selection(cx), caret_at(1, 2));
    assert_eq!(f.focus(cx), Focus::List);

    // escape returns to the text. The thread stays marked until the caret
    // leaves it.
    cx.simulate_keystrokes("escape");
    assert_eq!(f.focus(cx), Focus::Editor);
    assert_eq!(f.active(cx), Some(second));
    cx.simulate_keystrokes("cmd-down");
    assert_eq!(f.active(cx), None);
}

#[gpui::test]
fn focusing_the_panel_prefers_the_thread_at_the_caret(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    let [quick, _, third] = three(&f, cx);

    f.caret(2, 6, cx);
    cx.simulate_keystrokes("cmd-2");
    assert_eq!(f.active(cx), Some(third));
    cx.simulate_keystrokes("escape");

    // After the last comment: the first one.
    f.caret(2, 20, cx);
    assert_eq!(f.active(cx), None);
    cx.simulate_keystrokes("cmd-2");
    assert_eq!(f.active(cx), Some(quick));
    assert_eq!(f.focus(cx), Focus::List);
}

#[gpui::test]
fn a_document_without_comments_has_nothing_to_go_to(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    f.caret(1, 3, cx);
    cx.simulate_keystrokes("cmd-alt-] cmd-alt-[ cmd-2");
    assert_eq!(f.selection(cx), caret_at(1, 3));
    assert_eq!(f.focus(cx), Focus::Editor);
    assert_eq!(f.active(cx), None);
    f.panel.read_with(cx, |panel, _| assert!(!panel.is_open()));
}
