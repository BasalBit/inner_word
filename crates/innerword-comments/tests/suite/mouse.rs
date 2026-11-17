//! The panel with the mouse alone: clicks on cards, replies and buttons,
//! the buttons that appear under the pointer, tooltips. (The context menus
//! are in `menus`.)

use std::time::Duration;

use gpui::{Bounds, Modifiers, Pixels, Point, TestAppContext, VisualTestContext, point, px};
use innerword_comments::{DeleteTarget, DraftKind};
use innerword_core::CommentId;

use super::support::{Focus, open, sample};

pub fn try_bounds(
    selector: impl Into<String>,
    cx: &mut VisualTestContext,
) -> Option<Bounds<Pixels>> {
    // `debug_bounds` wants a `&'static str`; a test may leak a few names.
    let selector: &'static str = Box::leak(selector.into().into_boxed_str());
    cx.debug_bounds(selector)
}

pub fn bounds(selector: impl Into<String>, cx: &mut VisualTestContext) -> Bounds<Pixels> {
    let selector = selector.into();
    try_bounds(selector.clone(), cx).unwrap_or_else(|| panic!("nothing was painted for {selector}"))
}

pub fn card(id: CommentId, cx: &mut VisualTestContext) -> Bounds<Pixels> {
    bounds(format!("comment-thread-{}", id.0), cx)
}

/// Moves the pointer to `position` (no button held).
pub fn move_to(position: Point<Pixels>, cx: &mut VisualTestContext) {
    cx.simulate_mouse_move(position, None, Modifiers::none());
    cx.run_until_parked();
}

/// Moves the pointer onto the element and clicks it, as a hand does.
pub fn click(selector: impl Into<String>, cx: &mut VisualTestContext) {
    let center = bounds(selector, cx).center();
    move_to(center, cx);
    cx.simulate_click(center, Modifiers::none());
    cx.run_until_parked();
}

/// A point of a card that is not on its buttons or replies: the top left
/// corner of its text.
pub fn card_point(id: CommentId, cx: &mut VisualTestContext) -> Point<Pixels> {
    let card = card(id, cx);
    point(card.left() + px(24.), card.top() + px(12.))
}

/// Moves the pointer onto a card, where its buttons appear, and clicks one
/// of them (`comment-reply-button`, `comment-resolve-button`, ...).
pub fn click_card_button(button: &str, id: CommentId, cx: &mut VisualTestContext) {
    move_to(card_point(id, cx), cx);
    click(format!("{button}-{}", id.0), cx);
}

fn editor_point(cx: &mut VisualTestContext) -> Point<Pixels> {
    let panel = bounds("comments-panel", cx);
    point(panel.left() - px(80.), panel.top() + px(300.))
}

#[gpui::test]
fn a_card_shows_its_buttons_under_the_pointer_and_they_act_on_that_card(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    let first = f.comment(0, 4, 9, "First", cx);
    let second = f.comment(1, 2, 8, "Second", cx);
    assert_eq!(f.active(cx), Some(second));
    assert_eq!(f.focus(cx), Focus::Editor);
    f.panel
        .read_with(cx, |panel, _| assert_eq!(panel.hovered_thread(), None));

    // Under the pointer the first card has its buttons, without being the
    // active one.
    move_to(card_point(first, cx), cx);
    f.panel.read_with(cx, |panel, _| {
        assert_eq!(panel.hovered_thread(), Some(first))
    });
    assert_eq!(f.active(cx), Some(second));
    let reply = bounds(format!("comment-reply-button-{}", first.0), cx);
    let resolve = bounds(format!("comment-resolve-button-{}", first.0), cx);
    let card_bounds = card(first, cx);
    for button in [reply, resolve] {
        assert!(card_bounds.contains(&button.center()));
        assert!(button.size.width >= px(20.) && button.size.height >= px(20.));
    }

    // Reply: the card's own thread, not the active one.
    click(format!("comment-reply-button-{}", first.0), cx);
    assert_eq!(f.active(cx), Some(first));
    f.panel.read_with(cx, |panel, _| {
        assert_eq!(panel.draft(), Some(&DraftKind::Reply { thread: first }));
    });
    assert_eq!(f.focus(cx), Focus::Composer);
    cx.simulate_input("Agreed");

    // The composer's own button saves it.
    click("comment-submit", cx);
    f.panel.read_with(cx, |panel, _| {
        assert!(!panel.is_composing());
        let thread = &panel.thread(first).unwrap().thread;
        assert_eq!(thread.replies.len(), 1);
        assert_eq!(thread.replies[0].body, "Agreed");
    });
    assert_eq!(f.focus(cx), Focus::List);

    // Away from the panel no card is under the pointer.
    move_to(editor_point(cx), cx);
    f.panel
        .read_with(cx, |panel, _| assert_eq!(panel.hovered_thread(), None));
}

#[gpui::test]
fn the_buttons_are_not_there_until_the_pointer_or_the_keyboard_is(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    let first = f.comment(0, 4, 9, "First", cx);
    let second = f.comment(1, 2, 8, "Second", cx);
    assert_eq!(f.active(cx), Some(second));

    // Where the resolve button of the active card is, the other card has
    // nothing yet: a press there (without the pointer having moved onto
    // the card) is a click on the card.
    let on_second = bounds(format!("comment-resolve-button-{}", second.0), cx);
    let offset = on_second.center() - card(second, cx).origin;
    let same_place_on_first = card(first, cx).origin + offset;
    cx.simulate_click(same_place_on_first, Modifiers::none());
    assert_eq!(f.active(cx), Some(first));
    f.panel.read_with(cx, |panel, _| {
        assert!(!panel.thread(first).unwrap().is_resolved());
        assert_eq!(panel.hovered_thread(), None);
    });

    // Now it is the active card, so the button is there.
    cx.simulate_click(same_place_on_first, Modifiers::none());
    f.panel.read_with(cx, |panel, _| {
        assert!(panel.thread(first).unwrap().is_resolved());
    });
}

#[gpui::test]
fn resolve_and_reopen_with_the_card_button(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    let first = f.comment(0, 4, 9, "First", cx);
    let second = f.comment(1, 2, 8, "Second", cx);

    click_card_button("comment-resolve-button", first, cx);
    f.panel.read_with(cx, |panel, _| {
        assert!(panel.thread(first).unwrap().is_resolved());
    });
    // It left the list; its neighbour is the active one and the list has
    // the keyboard, as after `x`.
    assert_eq!(f.listed_ids(cx), vec![second]);
    assert_eq!(f.active(cx), Some(second));
    assert_eq!(f.focus(cx), Focus::List);

    // The footer is a button: it lists the resolved thread again.
    click("comments-resolved-toggle", cx);
    assert_eq!(f.listed_ids(cx), vec![first, second]);

    // The same button on a resolved card reopens it.
    click_card_button("comment-resolve-button", first, cx);
    f.panel.read_with(cx, |panel, _| {
        assert!(!panel.thread(first).unwrap().is_resolved());
    });
    assert_eq!(f.active(cx), Some(first));

    // Resolved threads are still listed, so resolving keeps the card.
    click_card_button("comment-resolve-button", first, cx);
    assert_eq!(f.listed_ids(cx), vec![first, second]);
    assert_eq!(f.active(cx), Some(first));
    click("comments-resolved-toggle", cx);
    f.panel
        .read_with(cx, |panel, _| assert!(!panel.show_resolved()));
    assert_eq!(f.listed_ids(cx), vec![second]);

    // Resolving the last open thread with the mouse: nothing is left to
    // act on, so the keyboard is back in the text.
    click_card_button("comment-resolve-button", second, cx);
    assert!(f.listed_ids(cx).is_empty());
    assert_eq!(f.focus(cx), Focus::Editor);
    f.panel.read_with(cx, |panel, _| {
        assert!(panel.is_open());
        assert_eq!(panel.resolved_count(), 2);
    });
}

#[gpui::test]
fn the_key_hints_of_the_active_card_are_buttons(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    let id = f.comment(0, 4, 9, "Tighten this", cx);
    let n = id.0;

    // A click on the card: it is active and the list has the keyboard,
    // so the card names its commands.
    let at = card_point(id, cx);
    move_to(at, cx);
    cx.simulate_click(at, Modifiers::none());
    assert_eq!(f.focus(cx), Focus::List);
    assert!(try_bounds("comment-thread-hints", cx).is_some());

    // Resolve, and back.
    click(format!("comment-hint-resolve-{n}"), cx);
    f.panel.read_with(cx, |panel, _| {
        assert!(panel.thread(id).unwrap().is_resolved());
    });
    cx.simulate_keystrokes("cmd-z");
    f.panel.read_with(cx, |panel, _| {
        assert!(!panel.thread(id).unwrap().is_resolved());
    });
    cx.simulate_keystrokes("cmd-2");
    assert_eq!(f.active(cx), Some(id));

    // Reply opens the composer; its Cancel button closes it again and the
    // keyboard is back on the list.
    click(format!("comment-hint-reply-{n}"), cx);
    f.panel.read_with(cx, |panel, _| {
        assert_eq!(panel.draft(), Some(&DraftKind::Reply { thread: id }));
    });
    assert_eq!(f.focus(cx), Focus::Composer);
    cx.simulate_input("never mind");
    click("comment-cancel", cx);
    f.panel
        .read_with(cx, |panel, _| assert!(!panel.is_composing()));
    assert_eq!(f.focus(cx), Focus::List);
    assert_eq!(f.comment_count(cx), 1);
    f.panel.read_with(cx, |panel, _| {
        assert!(panel.thread(id).unwrap().thread.replies.is_empty());
        assert_eq!(panel.kept_drafts(), 0);
    });

    // Edit opens the composer with the comment in it; Save stores it.
    click(format!("comment-hint-edit-{n}"), cx);
    f.panel.read_with(cx, |panel, _| {
        assert_eq!(panel.draft(), Some(&DraftKind::EditComment { thread: id }));
    });
    assert_eq!(f.composer_text(cx), "Tighten this");
    cx.simulate_input(" up");
    click("comment-submit", cx);
    f.panel.read_with(cx, |panel, _| {
        assert_eq!(panel.thread(id).unwrap().thread.body, "Tighten this up");
    });
    assert_eq!(f.focus(cx), Focus::List);

    // Delete asks first. Keep keeps.
    click(format!("comment-hint-delete-{n}"), cx);
    f.panel.read_with(cx, |panel, _| {
        assert_eq!(panel.pending_delete(), Some(DeleteTarget::Thread(id)));
    });
    click("cancel-delete-comment", cx);
    f.panel
        .read_with(cx, |panel, _| assert_eq!(panel.pending_delete(), None));
    assert_eq!(f.comment_count(cx), 1);
    assert_eq!(f.focus(cx), Focus::List);

    // Delete deletes; with no thread left the panel closes and the
    // keyboard is in the text.
    click(format!("comment-hint-delete-{n}"), cx);
    click("confirm-delete-comment", cx);
    assert_eq!(f.comment_count(cx), 0);
    assert_eq!(f.focus(cx), Focus::Editor);
    f.panel.read_with(cx, |panel, _| assert!(!panel.is_open()));
}

#[gpui::test]
fn deleting_with_the_mouse_moves_on_to_the_neighbour(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    let first = f.comment(0, 4, 9, "First", cx);
    let second = f.comment(1, 2, 8, "Second", cx);

    let at = card_point(first, cx);
    move_to(at, cx);
    cx.simulate_click(at, Modifiers::none());
    click(format!("comment-hint-delete-{}", first.0), cx);
    click("confirm-delete-comment", cx);
    assert_eq!(f.all_ids(cx), vec![second]);
    // The keyboard is still on the list, on a thread.
    assert_eq!(f.focus(cx), Focus::List);
    assert_eq!(f.active(cx), Some(second));
    cx.simulate_keystrokes("x");
    f.panel.read_with(cx, |panel, _| {
        assert!(panel.thread(second).unwrap().is_resolved());
    });
}

#[gpui::test]
fn the_header_buttons_add_a_comment_and_hide_the_panel(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    let first = f.comment(0, 4, 9, "First", cx);

    // Select text with the keyboard in the text, then press the button:
    // the keyboard was not taken from the editor by the press.
    f.select(1, 2, 8, cx);
    click("comments-add", cx);
    f.panel.read_with(cx, |panel, cx| {
        assert!(matches!(panel.draft(), Some(DraftKind::New { range })
            if range.start.block == 1 && range.start.offset == 2 && range.end.offset == 8));
        // Nothing to save yet.
        assert!(!panel.can_submit(cx));
    });
    assert_eq!(f.focus(cx), Focus::Composer);

    // The save button is dimmed while the composer is empty: a click on
    // it changes nothing.
    click("comment-submit", cx);
    assert_eq!(f.comment_count(cx), 1);
    f.panel
        .read_with(cx, |panel, _| assert!(panel.is_composing()));
    assert_eq!(f.focus(cx), Focus::Composer);

    cx.simulate_input("Second");
    f.panel
        .read_with(cx, |panel, cx| assert!(panel.can_submit(cx)));
    click("comment-submit", cx);
    assert_eq!(f.comment_count(cx), 2);
    let second = f.all_ids(cx)[1];
    assert_ne!(second, first);
    assert_eq!(f.active(cx), Some(second));
    // A new comment returns the keyboard to the text.
    assert_eq!(f.focus(cx), Focus::Editor);

    // Cancel discards a new comment.
    f.select(2, 4, 9, cx);
    click("comments-add", cx);
    cx.simulate_input("no");
    click("comment-cancel", cx);
    f.panel.read_with(cx, |panel, _| {
        assert!(!panel.is_composing());
        assert_eq!(panel.kept_drafts(), 0);
    });
    assert_eq!(f.comment_count(cx), 2);
    assert_eq!(f.focus(cx), Focus::Editor);

    // Without text to anchor to, the button changes nothing and the
    // keyboard stays in the text.
    f.editor.update(cx, |editor, cx| {
        editor.edit(|state| state.insert_text("   "), cx);
    });
    cx.run_until_parked();
    click("comments-add", cx);
    f.panel
        .read_with(cx, |panel, _| assert!(!panel.is_composing()));
    assert_eq!(f.focus(cx), Focus::Editor);

    // Hide: the panel is gone, the keyboard is still in the text.
    click("comments-hide", cx);
    f.panel.read_with(cx, |panel, _| {
        assert!(!panel.is_visible());
        assert!(!panel.is_open());
    });
    assert_eq!(f.visibility_events(cx).last(), Some(&false));
    assert_eq!(f.focus(cx), Focus::Editor);
}

#[gpui::test]
fn a_click_on_a_reply_selects_it(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    let id = f.comment(0, 4, 9, "Tighten this", cx);
    cx.simulate_keystrokes("cmd-2 r");
    cx.simulate_input("One");
    cx.simulate_keystrokes("enter r");
    cx.simulate_input("Two");
    cx.simulate_keystrokes("enter escape");
    assert_eq!(f.focus(cx), Focus::Editor);
    let replies: Vec<_> = f.panel.read_with(cx, |panel, _| {
        panel
            .thread(id)
            .unwrap()
            .thread
            .replies
            .iter()
            .map(|reply| reply.id)
            .collect()
    });
    assert_eq!(replies.len(), 2);

    click(format!("comment-reply-{}-{}", id.0, replies[1].0), cx);
    f.panel.read_with(cx, |panel, _| {
        assert_eq!(panel.selected_reply(), Some(replies[1]));
        assert_eq!(panel.hovered_reply(), Some((id, replies[1])));
    });
    assert_eq!(f.focus(cx), Focus::List);

    // The card's commands are now about the reply.
    click(format!("comment-hint-edit-{}", id.0), cx);
    f.panel.read_with(cx, |panel, _| {
        assert_eq!(
            panel.draft(),
            Some(&DraftKind::EditReply {
                thread: id,
                reply: replies[1]
            })
        );
    });
    assert_eq!(f.composer_text(cx), "Two");
    click("comment-cancel", cx);

    click(format!("comment-reply-{}-{}", id.0, replies[0].0), cx);
    click(format!("comment-hint-delete-{}", id.0), cx);
    f.panel.read_with(cx, |panel, _| {
        assert_eq!(
            panel.pending_delete(),
            Some(DeleteTarget::Reply(id, replies[0]))
        );
    });
    click("confirm-delete-comment", cx);
    f.panel.read_with(cx, |panel, _| {
        let thread = &panel.thread(id).unwrap().thread;
        assert_eq!(thread.replies.len(), 1);
        assert_eq!(thread.replies[0].body, "Two");
    });

    // A click on the card outside a reply is about the thread again.
    click(format!("comment-reply-{}-{}", id.0, replies[1].0), cx);
    f.panel.read_with(cx, |panel, _| {
        assert_eq!(panel.selected_reply(), Some(replies[1]))
    });
    let at = card_point(id, cx);
    move_to(at, cx);
    cx.simulate_click(at, Modifiers::none());
    f.panel
        .read_with(cx, |panel, _| assert_eq!(panel.selected_reply(), None));
}

#[gpui::test]
fn the_empty_list_offers_its_two_ways_out_as_buttons(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    let id = f.comment(0, 4, 9, "Only", cx);
    cx.simulate_keystrokes("cmd-2 x");
    assert_eq!(f.focus(cx), Focus::Editor);
    // Not marked while the keyboard is elsewhere.
    assert!(try_bounds("comments-empty", cx).is_some());

    cx.simulate_keystrokes("cmd-2");
    assert_eq!(f.focus(cx), Focus::List);
    click("comments-empty-back", cx);
    assert_eq!(f.focus(cx), Focus::Editor);

    cx.simulate_keystrokes("cmd-2");
    click("comments-empty-show", cx);
    f.panel
        .read_with(cx, |panel, _| assert!(panel.show_resolved()));
    assert_eq!(f.listed_ids(cx), vec![id]);
}

#[gpui::test]
fn a_button_says_what_it_does_and_which_key_does_the_same(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    let id = f.comment(0, 4, 9, "First", cx);
    assert!(try_bounds("tooltip", cx).is_none());

    let button = bounds(format!("comment-resolve-button-{}", id.0), cx);
    move_to(button.center(), cx);
    // GPUI shows a tooltip after the pointer has rested for half a second.
    cx.executor().advance_clock(Duration::from_millis(800));
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let tip = bounds("tooltip", cx);
    assert!(tip.size.width > px(40.));
    let _ = f;
}
