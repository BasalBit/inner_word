//! Closing the view, where the keyboard goes then, and that the view's own
//! keys can be changed without locking anybody in.

use gpui::{Focusable, Modifiers, TestAppContext};
use writer_keymap::Target;
use writer_keymap_ui::{LIST_KEY_CONTEXT, ModeKind};

use super::*;

fn document_has_keyboard(host: &Entity<Host>, cx: &mut VisualTestContext) -> bool {
    let field = host.read_with(cx, |host, cx| host.field.focus_handle(cx));
    cx.update(|window, _| field.is_focused(window))
}

fn reopen(host: &Entity<Host>, cx: &mut VisualTestContext) -> Entity<KeymapView> {
    press(cx, "cmd-/");
    host.read_with(cx, |host, _| host.view.clone().expect("the view is open"))
}

#[gpui::test]
fn the_keyboard_goes_back_where_it_came_from(cx: &mut TestAppContext) {
    let (host, cx) = window(cx);
    assert!(document_has_keyboard(&host, cx));

    // Opened: the keyboard is in the query field.
    let view = reopen(&host, cx);
    assert!(!document_has_keyboard(&host, cx));
    let query = view.read_with(cx, |view, cx| view.focus_handle(cx));
    assert!(cx.update(|window, _| query.is_focused(window)));

    // Closed with escape.
    press(cx, "escape");
    assert!(!host.read_with(cx, |host, cx| host.is_open(cx)));
    assert!(document_has_keyboard(&host, cx));
    cx.simulate_input("typed");
    assert_eq!(
        host.read_with(cx, |host, cx| host.field.read(cx).text().to_string()),
        "typed"
    );

    // Closed with its control.
    reopen(&host, cx);
    click(cx, "keymap-close");
    assert!(!host.read_with(cx, |host, cx| host.is_open(cx)));
    assert!(document_has_keyboard(&host, cx));

    // Closed with a click outside.
    reopen(&host, cx);
    cx.simulate_click(gpui::point(gpui::px(6.), gpui::px(6.)), Modifiers::none());
    cx.run_until_parked();
    assert!(!host.read_with(cx, |host, cx| host.is_open(cx)));
    assert!(document_has_keyboard(&host, cx));

    // Closed with the key that opened it.
    reopen(&host, cx);
    press(cx, "cmd-/");
    assert!(!host.read_with(cx, |host, cx| host.is_open(cx)));
    assert!(document_has_keyboard(&host, cx));

    // Closed while it waits for a key (with the pointer) and while it
    // asks: still back to the document.
    let view = reopen(&host, cx);
    select(&view, &bold(), cx);
    press(cx, "enter");
    assert_eq!(
        view.read_with(cx, |view, _| view.mode()),
        ModeKind::Recording
    );
    click(cx, "keymap-close");
    assert!(document_has_keyboard(&host, cx));
    // Nothing waits for a key any more: the next key is the document's.
    press(cx, "cmd-n");
    assert_eq!(host.read_with(cx, |host, _| host.new_documents), 1);
    cx.simulate_input("!");
    assert_eq!(
        host.read_with(cx, |host, cx| host.field.read(cx).text().to_string()),
        "typed!"
    );
}

#[gpui::test]
fn inside_the_view_the_keyboard_returns_to_the_query(cx: &mut TestAppContext) {
    let (host, view, cx) = open(cx);
    let query = view.read_with(cx, |view, cx| view.focus_handle(cx));
    select(&view, &bold(), cx);
    // After a change, after a cancelled question, after a closed menu.
    press(cx, "enter cmd-alt-b enter");
    assert!(cx.update(|window, _| query.is_focused(window)));
    press(cx, "enter f5 escape");
    assert!(cx.update(|window, _| query.is_focused(window)));
    press(cx, "ctrl-enter escape");
    assert!(cx.update(|window, _| query.is_focused(window)));
    press(cx, "cmd-shift-d escape");
    assert!(cx.update(|window, _| query.is_focused(window)));
    cx.simulate_input("it");
    assert_eq!(view.read_with(cx, |view, cx| view.query(cx)), "it");
    drop(host);
}

#[gpui::test]
fn the_views_own_keys_can_be_changed_and_the_hints_follow(cx: &mut TestAppContext) {
    let (host, view, cx) = open(cx);
    let change_key = Target::new("keymap_view::ChangeKey", Some(LIST_KEY_CONTEXT));
    select(&view, &change_key, cx);
    assert_eq!(
        view.read_with(cx, |view, _| view.selected_keys()),
        vec![key("enter")]
    );
    press(cx, "enter cmd-k enter");
    assert_eq!(keys_of(&host, &change_key, cx), vec!["cmd-k"]);

    // At once: `enter` no longer starts the waiting, the new key does.
    select(&view, &bold(), cx);
    press(cx, "enter");
    assert_eq!(view.read_with(cx, |view, _| view.mode()), ModeKind::List);
    press(cx, "cmd-k");
    assert_eq!(
        view.read_with(cx, |view, _| view.mode()),
        ModeKind::Recording
    );
    press(cx, "f5 enter");
    assert_eq!(keys_of(&host, &bold(), cx), vec!["f5"]);

    // The menu of a row shows the new key beside "Change Key…".
    press(cx, "ctrl-enter");
    cx.update(|window, cx| {
        let menu = writer_ui::ContextMenu::current(window, cx).unwrap();
        let item = &menu.read(cx).items()[0];
        assert_eq!(item.title().as_ref(), "Change Key…");
        assert_eq!(
            item.explicit_key_hint().map(|hint| hint.text()).as_deref(),
            Some("⌘K")
        );
    });
    press(cx, "escape");

    // Put back: `enter` again.
    select(&view, &change_key, cx);
    press(cx, "cmd-d");
    select(&view, &bold(), cx);
    press(cx, "enter");
    assert_eq!(
        view.read_with(cx, |view, _| view.mode()),
        ModeKind::Recording
    );
}

#[gpui::test]
fn escape_still_closes_after_the_key_of_close_was_taken_away(cx: &mut TestAppContext) {
    let (host, view, cx) = open(cx);
    let close = Target::new("keymap_view::Close", Some(LIST_KEY_CONTEXT));
    select(&view, &close, cx);
    assert_eq!(
        view.read_with(cx, |view, _| view.selected_keys()),
        vec![key("escape")]
    );
    press(cx, "cmd-backspace");
    assert!(keys_of(&host, &close, cx).is_empty());
    // The view has no binding of its own on escape now; the query field's
    // escape closes it.
    press(cx, "escape");
    assert!(!host.read_with(cx, |host, cx| host.is_open(cx)));

    // And with that one gone too (the field's own escape removed), the key
    // itself still closes: no binding takes it.
    let view = reopen(&host, cx);
    let field_escape = Target::new("text_field::Cancel", Some("TextField"));
    select(&view, &field_escape, cx);
    press(cx, "cmd-backspace");
    let modal_escape = Target::new("modal::Dismiss", Some("ModalLayer"));
    select(&view, &modal_escape, cx);
    press(cx, "cmd-backspace");
    assert!(keys_of(&host, &field_escape, cx).is_empty());
    assert!(keys_of(&host, &modal_escape, cx).is_empty());
    press(cx, "escape");
    assert!(
        !host.read_with(cx, |host, cx| host.is_open(cx)),
        "escape closes with no binding at all"
    );

    // A question is cancelled the same way when its key is gone.
    let view = reopen(&host, cx);
    let cancel = Target::new(
        "keymap_view::Cancel",
        Some(writer_keymap_ui::CHOICES_KEY_CONTEXT),
    );
    select(&view, &cancel, cx);
    press(cx, "cmd-backspace");
    select(&view, &bold(), cx);
    press(cx, "enter f5");
    assert_eq!(
        view.read_with(cx, |view, _| view.mode()),
        ModeKind::Question
    );
    press(cx, "escape");
    assert_eq!(view.read_with(cx, |view, _| view.mode()), ModeKind::List);
    assert!(host.read_with(cx, |host, cx| host.is_open(cx)));

    // The pointer closes whatever the keys are.
    click(cx, "keymap-close");
    assert!(!host.read_with(cx, |host, cx| host.is_open(cx)));
}

#[gpui::test]
fn everything_is_put_back_from_the_keys_after_a_confirmation(cx: &mut TestAppContext) {
    let (host, view, cx) = open(cx);
    // Nothing to put back: the key does nothing.
    press(cx, "cmd-shift-d");
    assert_eq!(view.read_with(cx, |view, _| view.mode()), ModeKind::List);

    select(&view, &bold(), cx);
    press(cx, "enter cmd-alt-b enter");
    press(cx, "cmd-shift-d");
    assert_eq!(
        view.read_with(cx, |view, _| view.mode()),
        ModeKind::Question
    );
    assert!(
        view.read_with(cx, |view, _| view.question_lines()[0].text.clone())
            .starts_with("1 command goes back to the default keys.")
    );
    // Escape: nothing happens.
    press(cx, "escape");
    assert_eq!(keys_of(&host, &bold(), cx), vec!["cmd-alt-b"]);
    // Confirmed.
    press(cx, "cmd-shift-d enter");
    assert_eq!(keys_of(&host, &bold(), cx), vec!["cmd-b"]);
    assert_eq!(view.read_with(cx, |view, _| view.changed_count()), 0);
    assert!(!view.read_with(cx, |view, _| view.can_reset_all()));
}

/// While the view waits for a key it says that every key counts and how
/// to leave without a change: with the pointer, and from the keyboard
/// alone. What it says about the keyboard is read from the keymap, and it
/// is true.
#[gpui::test]
fn waiting_for_a_key_says_how_to_leave_and_it_is_so(cx: &mut TestAppContext) {
    let (host, view, cx) = open(cx);
    let waiting = |cx: &mut VisualTestContext| view.read_with(cx, |view, cx| view.waiting_says(cx));
    let mode = |cx: &mut VisualTestContext| view.read_with(cx, |view, _| view.mode());
    assert_eq!(waiting(cx), None, "not waiting yet");

    select(&view, &bold(), cx);
    press(cx, "enter");
    assert_eq!(
        waiting(cx).as_deref(),
        Some(
            "Every key counts, Esc and Return included. To leave without a change: click \
             Cancel, or press Esc twice (the first is taken as the key, the second cancels it)."
        )
    );
    assert!(cx.debug_bounds("keymap-record-help").is_some());
    // Every key counts: Return and Esc are asked about, not obeyed.
    press(cx, "enter");
    assert_eq!(mode(cx), ModeKind::Question);
    assert!(
        view.read_with(cx, |view, _| view.question_title().unwrap())
            .starts_with("↩ ")
    );
    press(cx, "backspace");
    assert_eq!(mode(cx), ModeKind::Recording);
    // With the pointer: Cancel.
    click(cx, "keymap-record-cancel");
    assert_eq!(mode(cx), ModeKind::List);
    // From the keyboard alone: Esc twice.
    press(cx, "enter");
    assert_eq!(mode(cx), ModeKind::Recording);
    press(cx, "escape");
    assert_eq!(mode(cx), ModeKind::Question, "the first Esc is asked about");
    press(cx, "escape");
    assert_eq!(mode(cx), ModeKind::List, "the second cancels");
    assert!(requests(&host, cx).is_empty(), "nothing was changed");
    assert!(host.read_with(cx, |host, cx| host.is_open(cx)));

    // The key that cancels a question is the user's to change: the
    // sentence names the key it is now.
    let cancel = Target::new(
        "keymap_view::Cancel",
        Some(writer_keymap_ui::CHOICES_KEY_CONTEXT),
    );
    select(&view, &cancel, cx);
    press(cx, "enter f1 enter");
    assert_eq!(keys_of(&host, &cancel, cx), vec!["f1"]);
    select(&view, &bold(), cx);
    press(cx, "enter");
    let says = waiting(cx).unwrap();
    assert!(
        says.ends_with("click Cancel, or press any key and then F1."),
        "{says}"
    );
    press(cx, "f9");
    assert_eq!(mode(cx), ModeKind::Question);
    press(cx, "f1");
    assert_eq!(mode(cx), ModeKind::List);
    assert_eq!(keys_of(&host, &bold(), cx), vec!["cmd-b"]);

    // With no key for it at all, the way leads through the question's
    // choices, which the keys still reach.
    select(&view, &cancel, cx);
    press(cx, "cmd-backspace");
    assert!(keys_of(&host, &cancel, cx).is_empty());
    select(&view, &bold(), cx);
    press(cx, "enter");
    let says = waiting(cx).unwrap();
    assert!(
        says.ends_with(
            "click Cancel, or press any key and choose Cancel in the question that follows."
        ),
        "{says}"
    );
    // Any key (a free one: "Use" is marked), then to "Cancel" and take it.
    press(cx, "f9");
    assert_eq!(
        view.read_with(cx, |view, _| view.marked_choice()),
        Some(writer_keymap_ui::Choice::UseKey)
    );
    press(cx, "right right");
    assert_eq!(
        view.read_with(cx, |view, _| view.marked_choice()),
        Some(writer_keymap_ui::Choice::Cancel)
    );
    press(cx, "enter");
    assert_eq!(mode(cx), ModeKind::List);
    assert_eq!(keys_of(&host, &bold(), cx), vec!["cmd-b"]);
}
