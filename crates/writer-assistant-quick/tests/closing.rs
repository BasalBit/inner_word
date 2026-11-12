//! Every way the popover closes, and the ways it does not.

use gpui::{TestAppContext, point, px};
use writer_assistant_quick::QuickChatEvent;
use writer_core::{Block, Document, Position, TextRange};

use crate::support::*;

fn opened(cx: &mut TestAppContext) -> (Fixture, &mut gpui::VisualTestContext) {
    let (fx, cx) = open_quick(
        paragraphs(&["The quick brown fox", "jumps over", "the lazy dog"]),
        cx,
    );
    select(&fx.editor, TextRange::in_block(0, 4, 9), cx);
    fx.toggle(cx);
    assert!(fx.is_open(cx));
    assert!(fx.composer_focused(cx));
    (fx, cx)
}

#[gpui::test]
fn the_key_that_opened_it_closes_it(cx: &mut TestAppContext) {
    let (fx, cx) = opened(cx);
    fx.toggle(cx);
    assert!(!fx.is_open(cx));
    assert!(fx.editor_focused(cx));
    // The selection is what it was.
    assert_eq!(selected_text(&fx.editor, cx), "quick");
    assert_eq!(
        fx.events(cx),
        vec![QuickChatEvent::Opened, QuickChatEvent::Closed]
    );
    assert_eq!(fx.quick.read_with(cx, |quick, _| quick.bounds()), None);
}

#[gpui::test]
fn escape_closes_it(cx: &mut TestAppContext) {
    let (fx, cx) = opened(cx);
    cx.simulate_input("half a quest");
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(!fx.is_open(cx));
    assert!(fx.editor_focused(cx));
    assert_eq!(selected_text(&fx.editor, cx), "quick");
    assert_eq!(
        text(&fx.editor, cx),
        "The quick brown fox\njumps over\nthe lazy dog"
    );
    assert!(fx.fake.sent().is_empty());
}

#[gpui::test]
fn the_close_button_closes_it(cx: &mut TestAppContext) {
    let (fx, cx) = opened(cx);
    fx.fake.hold();
    fx.send("Another word?", cx);
    assert!(fx.quick.read_with(cx, |quick, _| quick.is_answering()));

    // Also in the middle of an answer: the button closes outright.
    click("quick-chat-close", cx);
    assert!(!fx.is_open(cx));
    assert!(fx.editor_focused(cx));
    assert_eq!(fx.fake.alive(), 0, "the agent's conversation is closed");
}

#[gpui::test]
fn a_press_in_the_text_closes_it_and_goes_on_to_the_text(cx: &mut TestAppContext) {
    let (fx, cx) = opened(cx);
    let popover = fx.popover(cx);
    // In the third paragraph, clear of the popover.
    let target = fx
        .editor
        .read_with(cx, |editor, _| editor.position_bounds(Position::new(2, 4)))
        .unwrap();
    let at = point(target.left(), target.center().y);
    let at = if popover.contains(&at) {
        point(popover.right() + px(30.), popover.top() - px(40.))
    } else {
        at
    };

    press_at(at, cx);

    assert!(!fx.is_open(cx));
    assert!(fx.editor_focused(cx));
    // The press did what a press in the text does.
    assert!(selection(&fx.editor, cx).is_empty());
    assert_ne!(
        selection(&fx.editor, cx).start,
        Position::new(0, 9),
        "the caret went where the press was"
    );
}

#[gpui::test]
fn a_press_on_something_that_takes_no_keyboard_gives_it_to_the_text(cx: &mut TestAppContext) {
    let (fx, cx) = opened(cx);
    let band = bounds("top-band", cx);
    press_at(band.center(), cx);
    assert!(!fx.is_open(cx));
    assert!(fx.editor_focused(cx));
    assert_eq!(selected_text(&fx.editor, cx), "quick");
}

#[gpui::test]
fn a_press_on_another_panel_leaves_the_keyboard_there(cx: &mut TestAppContext) {
    let (fx, cx) = opened(cx);
    let strip = bounds("side-strip", cx);
    press_at(strip.center(), cx);
    assert!(!fx.is_open(cx));
    assert!(!fx.editor_focused(cx));
    let elsewhere = fx.host.read_with(cx, |host, _| host.elsewhere.clone());
    assert!(cx.update(|window, _| elsewhere.is_focused(window)));
}

#[gpui::test]
fn the_keyboard_going_elsewhere_closes_it(cx: &mut TestAppContext) {
    let (fx, cx) = opened(cx);
    let elsewhere = fx.host.read_with(cx, |host, _| host.elsewhere.clone());
    cx.update(|window, _| window.focus(&elsewhere));
    cx.run_until_parked();
    assert!(!fx.is_open(cx));
    // The keyboard stays where it went.
    assert!(cx.update(|window, _| elsewhere.is_focused(window)));
    assert!(!fx.editor_focused(cx));
}

#[gpui::test]
fn a_command_that_gives_the_text_the_keyboard_closes_it(cx: &mut TestAppContext) {
    let (fx, cx) = opened(cx);
    fx.editor
        .update_in(cx, |editor, window, _| editor.focus(window));
    cx.run_until_parked();
    assert!(!fx.is_open(cx));
    assert!(fx.editor_focused(cx));
}

#[gpui::test]
fn another_app_coming_to_the_front_does_not_close_it(cx: &mut TestAppContext) {
    let (fx, cx) = opened(cx);
    cx.simulate_input("half a quest");
    cx.deactivate_window();
    cx.run_until_parked();
    assert!(fx.is_open(cx));
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    assert!(fx.is_open(cx));
    assert!(fx.composer_focused(cx));
    assert_eq!(fx.draft(cx), "half a quest");
}

#[gpui::test]
fn another_document_closes_it(cx: &mut TestAppContext) {
    let (fx, cx) = opened(cx);
    fx.editor.update(cx, |editor, cx| {
        editor.set_document(
            Document::from_blocks(vec![Block::paragraph("Another document.")]),
            cx,
        )
    });
    cx.run_until_parked();
    assert!(!fx.is_open(cx));
    assert!(fx.editor_focused(cx), "the keyboard was in the popover");
}

#[gpui::test]
fn a_press_inside_never_moves_the_keyboard_to_the_text(cx: &mut TestAppContext) {
    let (fx, cx) = opened(cx);
    fx.fake.reply("swift");
    fx.send("Another word?", cx);
    let popover = fx.popover(cx);

    // On the header beside its controls.
    let header = bounds("quick-chat-header", cx);
    press_at(point(header.left() + px(60.), header.center().y), cx);
    assert!(fx.is_open(cx));
    assert!(!fx.editor_focused(cx));
    assert!(fx.has_keyboard(cx));
    // The keyboard is where typing makes sense.
    assert!(fx.composer_focused(cx));

    // On the question (a message of the user is marked by a press).
    let conversation = bounds("quick-chat-conversation", cx);
    press_at(
        point(conversation.right() - px(20.), conversation.top() + px(12.)),
        cx,
    );
    assert!(fx.is_open(cx));
    assert!(!fx.editor_focused(cx));
    assert!(fx.has_keyboard(cx));

    // On the very edge of the popover.
    press_at(
        point(popover.left() + px(2.), popover.bottom() - px(2.)),
        cx,
    );
    assert!(fx.is_open(cx));
    assert!(!fx.editor_focused(cx));
    assert!(fx.has_keyboard(cx));

    // The text under the popover did not get any of the presses.
    assert_eq!(selected_text(&fx.editor, cx), "quick");
    assert_eq!(
        text(&fx.editor, cx),
        "The quick brown fox\njumps over\nthe lazy dog"
    );

    // Typing still goes to the composer.
    cx.simulate_input("more");
    cx.run_until_parked();
    assert_eq!(fx.draft(cx), "more");
}

#[gpui::test]
fn a_menu_of_the_popover_takes_escape_and_presses_first(cx: &mut TestAppContext) {
    let (fx, cx) = opened(cx);
    click("assistant-effort", cx);
    assert!(menu_is_open(cx));

    // escape closes the menu, not the popover.
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(!menu_is_open(cx));
    assert!(fx.is_open(cx));

    // A press beside an open menu closes the menu only.
    click("assistant-model", cx);
    assert!(menu_is_open(cx));
    let band = bounds("top-band", cx);
    press_at(band.center(), cx);
    assert!(!menu_is_open(cx));
    assert!(fx.is_open(cx));
    assert!(fx.composer_focused(cx));
}

#[gpui::test]
fn opening_twice_keeps_one_popover(cx: &mut TestAppContext) {
    let (fx, cx) = opened(cx);
    cx.simulate_input("half");
    let opened_again = fx
        .quick
        .update_in(cx, |quick, window, cx| quick.open(window, cx));
    cx.run_until_parked();
    assert!(opened_again);
    assert_eq!(fx.draft(cx), "half", "the same popover");
    assert_eq!(
        fx.events(cx)
            .iter()
            .filter(|event| **event == QuickChatEvent::Opened)
            .count(),
        1
    );
}

#[gpui::test]
fn closing_a_closed_popover_does_nothing(cx: &mut TestAppContext) {
    let (fx, cx) = open_quick(paragraphs(&["The quick brown fox"]), cx);
    fx.quick
        .update_in(cx, |quick, window, cx| quick.close(window, cx));
    fx.quick
        .update_in(cx, |quick, window, cx| quick.dismiss(window, cx));
    cx.run_until_parked();
    assert!(fx.events(cx).is_empty());
    assert!(fx.editor_focused(cx));
}

#[gpui::test]
fn a_popover_that_lost_the_keyboard_before_its_first_frame_closes(cx: &mut TestAppContext) {
    // A modal that closes as the popover opens gives the keyboard back to
    // the text after the popover took it: no frame ever had the keyboard
    // in the popover, so no focus event says that it left.
    let (fx, cx) = open_quick(paragraphs(&["The quick brown fox"]), cx);
    let quick = fx.quick.clone();
    let editor = fx.editor.clone();
    cx.update(|window, cx| {
        quick.update(cx, |quick, cx| quick.open(window, cx));
        editor.read(cx).focus(window);
    });
    cx.run_until_parked();
    assert!(
        !fx.is_open(cx),
        "a popover without the keyboard does not stay"
    );
    assert!(fx.editor_focused(cx));
    // Typing is the text's.
    cx.simulate_input("x");
    cx.run_until_parked();
    assert!(text(&fx.editor, cx).contains('x'));
}

#[gpui::test]
fn in_a_window_in_the_background_the_keyboard_leaving_still_closes_it(cx: &mut TestAppContext) {
    // A window that is not active hears no focus events (a scripted run
    // of the app is like that).
    let (fx, cx) = opened(cx);
    cx.deactivate_window();
    cx.run_until_parked();
    assert!(fx.is_open(cx));
    let elsewhere = fx.host.read_with(cx, |host, _| host.elsewhere.clone());
    cx.update(|window, _| window.focus(&elsewhere));
    cx.run_until_parked();
    // Something has to draw a frame for the popover to notice.
    fx.quick.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    assert!(!fx.is_open(cx));
    assert!(cx.update(|window, _| elsewhere.is_focused(window)));
}
