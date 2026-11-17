//! What a row offers: another key, one more key, no key, its default,
//! running its command; from the keys and from the row's menu.

use gpui::TestAppContext;
use innerword_keymap_ui::{ModeKind, Request};
use innerword_ui::ContextMenu;

use super::*;

fn menu_labels(cx: &mut VisualTestContext) -> Vec<String> {
    cx.update(|window, cx| {
        ContextMenu::current(window, cx)
            .map(|menu| menu.read(cx).labels())
            .unwrap_or_default()
    })
}

fn choose(cx: &mut VisualTestContext, label: &str) {
    let chosen = cx.update(|window, cx| ContextMenu::choose(label, window, cx));
    assert!(chosen, "{label} is in the menu: {:?}", menu_labels(cx));
    cx.run_until_parked();
}

fn context_menu() -> Target {
    Target::new("demo::OpenContextMenu", Some("Editor"))
}

#[gpui::test]
fn a_second_key_is_added_and_a_key_is_removed(cx: &mut TestAppContext) {
    let (host, view, cx) = open(cx);
    select(&view, &bold(), cx);
    press(cx, "cmd-enter");
    assert_eq!(
        view.read_with(cx, |view, _| view.mode()),
        ModeKind::Recording
    );
    assert!(describe(&view, cx).contains("replacing=nothing"));
    press(cx, "cmd-alt-b enter");
    assert_eq!(keys_of(&host, &bold(), cx), vec!["cmd-b", "cmd-alt-b"]);
    assert_eq!(
        requests(&host, cx),
        vec![Request::AddKey {
            target: bold(),
            new: key("cmd-alt-b"),
            take: Vec::new(),
        }]
    );
    // Both run the command.
    press(cx, "escape cmd-b cmd-alt-b");
    assert_eq!(host.read_with(cx, |host, _| host.bolds), 2);

    // With two keys `tab` chooses which one the keys act on.
    press(cx, "cmd-/");
    let view = host.read_with(cx, |host, _| host.view.clone().unwrap());
    select(&view, &bold(), cx);
    press(cx, "tab cmd-backspace");
    assert_eq!(
        keys_of(&host, &bold(), cx),
        vec!["cmd-b"],
        "the second key went"
    );
    press(cx, "cmd-backspace");
    assert!(
        keys_of(&host, &bold(), cx).is_empty(),
        "a command may be left without a key"
    );
    view.read_with(cx, |view, cx| {
        assert!(view.selected_keys().is_empty());
        assert!(
            view.describe(cx)
                .contains("keys=[] default=[cmd-b] (changed)")
        );
    });
    // Nothing left to remove.
    press(cx, "cmd-backspace");
    assert_eq!(requests(&host, cx).len(), 3);

    // Its default comes back with one key.
    press(cx, "cmd-d");
    assert_eq!(keys_of(&host, &bold(), cx), vec!["cmd-b"]);
    assert_eq!(view.read_with(cx, |view, _| view.changed_count()), 0);
    // And a row that has its defaults has nothing to put back.
    press(cx, "cmd-d");
    assert_eq!(requests(&host, cx).len(), 4);
}

#[gpui::test]
fn a_command_without_a_key_is_given_one(cx: &mut TestAppContext) {
    let (host, view, cx) = open(cx);
    select(&view, &version_history(), cx);
    assert!(view.read_with(cx, |view, _| view.selected_keys().is_empty()));
    // `enter` on a row without a key adds its first.
    press(cx, "enter cmd-alt-h enter");
    assert_eq!(keys_of(&host, &version_history(), cx), vec!["cmd-alt-h"]);
    assert!(matches!(
        requests(&host, cx).as_slice(),
        [Request::AddKey { .. }]
    ));
    view.read_with(cx, |view, cx| {
        assert!(
            view.describe(cx)
                .contains("keys=[cmd-alt-h] default=[] (changed)")
        );
    });
}

#[gpui::test]
fn a_key_bound_in_two_spellings_is_one_key_and_changes_as_one(cx: &mut TestAppContext) {
    let (host, view, cx) = open(cx);
    let numbered = Target::new("demo::NumberedList", Some("Editor"));
    select(&view, &numbered, cx);
    assert_eq!(
        view.read_with(cx, |view, _| view.selected_keys()),
        vec![key("cmd-shift-7")]
    );
    press(cx, "enter cmd-alt-7 enter");
    assert_eq!(keys_of(&host, &numbered, cx), vec!["cmd-alt-7"]);
    // Neither spelling of the old key is bound any more.
    let left: Vec<String> = cx.update(|_, cx| {
        cx.key_bindings()
            .borrow()
            .bindings()
            .filter(|binding| binding.action().name() == "demo::NumberedList")
            .map(|binding| binding.keystrokes()[0].unparse())
            .collect()
    });
    assert_eq!(left, vec!["alt-cmd-7"]);

    // A key recorded as the keyboard sends it (the shifted character) is
    // shown in the readable form, and bound in both.
    press(cx, "enter cmd-* enter");
    assert_eq!(keys_of(&host, &numbered, cx), vec!["cmd-shift-8"]);
    let bound: Vec<String> = cx.update(|_, cx| {
        cx.key_bindings()
            .borrow()
            .bindings()
            .filter(|binding| binding.action().name() == "demo::NumberedList")
            .map(|binding| binding.keystrokes()[0].unparse())
            .collect()
    });
    assert_eq!(bound, vec!["cmd-*", "cmd-shift-8"]);
}

#[gpui::test]
fn a_row_with_two_keys_changes_the_one_that_is_chosen(cx: &mut TestAppContext) {
    let (host, view, cx) = open(cx);
    select(&view, &context_menu(), cx);
    assert_eq!(
        view.read_with(cx, |view, _| view.selected_keys()),
        vec![key("shift-f10"), key("ctrl-enter")]
    );
    // `enter` acts on the key the rest of the app shows for the command,
    // the one bound last.
    press(cx, "enter");
    assert!(
        describe(&view, cx).contains("replacing=ctrl-enter"),
        "{}",
        describe(&view, cx)
    );
    press(cx, "f9 enter");
    assert_eq!(keys_of(&host, &context_menu(), cx), vec!["shift-f10", "f9"]);
    // `tab` moves on to the other key, `shift-tab` back, round.
    press(cx, "tab enter");
    assert!(describe(&view, cx).contains("replacing=shift-f10"));
    press(cx, "escape escape");
    press(cx, "shift-tab enter");
    assert!(describe(&view, cx).contains("replacing=f9"));
}

#[gpui::test]
fn the_rows_menu_opens_from_the_keys_and_does_what_the_keys_do(cx: &mut TestAppContext) {
    let (host, view, cx) = open(cx);
    select(&view, &bold(), cx);
    press(cx, "ctrl-enter");
    assert_eq!(
        menu_labels(cx),
        vec![
            "Change Key…",
            "Add Another Key…",
            "Remove Key",
            "-",
            "Use Default (⌘B)",
            "-",
            "Run Command"
        ]
    );
    // Opened from the keys: the first item is marked, and the keyboard
    // stays where it was.
    cx.update(|window, cx| {
        let menu = ContextMenu::current(window, cx).unwrap();
        assert_eq!(menu.read(cx).selected_index(), Some(0));
        // "Use Default" is greyed while the row has its default.
        assert!(menu.read(cx).items()[4].is_disabled());
    });
    // `enter` chooses the marked item: change the key.
    press(cx, "enter");
    assert!(!cx.update(|window, cx| ContextMenu::is_open(window, cx)));
    assert_eq!(
        view.read_with(cx, |view, _| view.mode()),
        ModeKind::Recording
    );
    press(cx, "cmd-alt-b enter");
    assert_eq!(keys_of(&host, &bold(), cx), vec!["cmd-alt-b"]);

    // The second key of the menu, and the other items.
    press(cx, "shift-f10");
    assert!(menu_labels(cx).contains(&"Use Default (⌘B)".to_string()));
    choose(cx, "Add Another Key…");
    assert!(describe(&view, cx).contains("mode=recording"));
    assert!(describe(&view, cx).contains("replacing=nothing"));
    press(cx, "f6 enter");
    assert_eq!(keys_of(&host, &bold(), cx), vec!["cmd-alt-b", "f6"]);

    // With two keys the menu names each.
    press(cx, "ctrl-enter");
    assert_eq!(
        menu_labels(cx),
        vec![
            "Change ⌥⌘B…",
            "Change F6…",
            "Add Another Key…",
            "Remove ⌥⌘B",
            "Remove F6",
            "-",
            "Use Default (⌘B)",
            "-",
            "Run Command"
        ]
    );
    choose(cx, "Remove ⌥⌘B");
    assert_eq!(keys_of(&host, &bold(), cx), vec!["f6"]);
    press(cx, "ctrl-enter");
    choose(cx, "Use Default (⌘B)");
    assert_eq!(keys_of(&host, &bold(), cx), vec!["cmd-b"]);
    press(cx, "ctrl-enter");
    choose(cx, "Run Command");
    assert_eq!(host.read_with(cx, |host, _| host.ran.clone()), vec![bold()]);

    // The key pressed again closes the menu; so does escape, and the view
    // stays.
    press(cx, "ctrl-enter");
    assert!(cx.update(|window, cx| ContextMenu::is_open(window, cx)));
    press(cx, "escape");
    assert!(!cx.update(|window, cx| ContextMenu::is_open(window, cx)));
    assert!(host.read_with(cx, |host, cx| host.is_open(cx)));
}

#[gpui::test]
fn the_command_of_a_row_is_run_from_the_key(cx: &mut TestAppContext) {
    let (host, view, cx) = open(cx);
    select(&view, &bold(), cx);
    press(cx, "cmd-r");
    assert_eq!(requests(&host, cx), vec![Request::Run { target: bold() }]);
    // A key that only means something where it is pressed is not offered
    // to be run.
    let caret = view.read_with(cx, |view, _| {
        view.listed()
            .into_iter()
            .find(|(target, _)| target.command.action == "text_field::WordLeft")
            .map(|(target, _)| target)
            .unwrap()
    });
    select(&view, &caret, cx);
    press(cx, "cmd-r");
    assert_eq!(requests(&host, cx).len(), 1);
    press(cx, "ctrl-enter");
    assert!(!menu_labels(cx).contains(&"Run Command".to_string()));
}

#[gpui::test]
fn a_row_that_cannot_be_changed_says_why(cx: &mut TestAppContext) {
    let (host, view, cx) = open(cx);
    let insert = Target::new("demo::OpenInsertMenu", Some("Editor"));
    select(&view, &insert, cx);
    view.read_with(cx, |view, cx| {
        // It shows the character that opens the menu.
        assert_eq!(view.selected_keys(), vec![key("/")]);
        let line = view.describe(cx);
        assert!(
            line.contains(&format!("locked={:?}", host::TYPED_REASON)),
            "{line}"
        );
    });
    // None of the keys changes it.
    press(cx, "enter");
    assert_eq!(view.read_with(cx, |view, _| view.mode()), ModeKind::List);
    press(cx, "cmd-enter cmd-backspace cmd-d");
    assert_eq!(view.read_with(cx, |view, _| view.mode()), ModeKind::List);
    assert!(requests(&host, cx).is_empty());
    // Its menu says so too, and offers nothing that changes it.
    press(cx, "ctrl-enter");
    let labels = menu_labels(cx);
    assert_eq!(labels[0], "Cannot be changed here");
    assert_eq!(labels[1], host::TYPED_REASON);
    assert!(!labels.iter().any(|label| label.starts_with("Change")));
    cx.update(|window, cx| {
        let menu = ContextMenu::current(window, cx).unwrap();
        assert!(menu.read(cx).items()[1].is_disabled());
    });
}

/// The menu of a row opens under the row's "more" control at the right,
/// where it covers the least of the rows below: from the keys and from a
/// click on the control. (In the released list it opened at the left edge
/// of the row and lay over the names of the rows under it.) A right-click
/// opens it at the pointer, as everywhere.
#[gpui::test]
fn the_rows_menu_opens_under_the_more_control(cx: &mut TestAppContext) {
    use innerword_ui::context_menu::MenuAnchor;
    let (_, view, cx) = open(cx);
    select(&view, &bold(), cx);
    let row = view.read_with(cx, |view, _| view.row_of(&bold()).unwrap());
    let more = bounds(cx, selector(format!("keymap-more-{row}")));
    let row_bounds = bounds(cx, selector(format!("keymap-row-{row}")));
    // The control is at the right end of the row.
    assert!(more.left() > row_bounds.left() + (row_bounds.size.width / 2.));

    let menu_at = |cx: &mut VisualTestContext| {
        cx.update(|window, cx| {
            let menu = ContextMenu::current(window, cx).expect("the menu is open");
            (menu.read(cx).anchor(), menu.read(cx).bounds())
        })
    };
    // From the keys.
    press(cx, "ctrl-enter");
    let (anchor, drawn) = menu_at(cx);
    assert_eq!(anchor, MenuAnchor::Below(more));
    let drawn = drawn.expect("the menu is drawn");
    assert_eq!(drawn.left(), more.left(), "its left edge at the control's");
    assert!(drawn.top() >= more.bottom(), "under the control");
    // The names of the rows below are not under it.
    assert!(drawn.left() > row_bounds.left() + (row_bounds.size.width / 2.));
    press(cx, "escape");

    // From a click on the control: the same place.
    click(cx, selector(format!("keymap-more-{row}")));
    let (anchor, _) = menu_at(cx);
    assert_eq!(anchor, MenuAnchor::Below(more));
    press(cx, "escape");

    // A right-click on the row: at the pointer.
    let at = row_bounds.center();
    right_click(cx, selector(format!("keymap-row-{row}")));
    let (anchor, _) = menu_at(cx);
    assert_eq!(anchor, MenuAnchor::Point(at));
    press(cx, "escape");

    // Another row, selected with the keys: its control, not the last one's.
    press(cx, "down");
    let next = view.read_with(cx, |view, _| {
        let target = view.selected_target().cloned().unwrap();
        view.row_of(&target).unwrap()
    });
    assert_ne!(next, row);
    let more = bounds(cx, selector(format!("keymap-more-{next}")));
    press(cx, "ctrl-enter");
    let (anchor, _) = menu_at(cx);
    assert_eq!(anchor, MenuAnchor::Below(more));
}

/// In the menu of a row with several keys, the items of the key that the
/// keys of the list act on show those keys; the other key's items show
/// none. (All four were without a key in the released list.)
#[gpui::test]
fn the_menu_of_a_row_with_two_keys_says_which_key_the_keys_act_on(cx: &mut TestAppContext) {
    let (_, view, cx) = open(cx);
    select(&view, &context_menu(), cx);
    let hints = |cx: &mut VisualTestContext| -> Vec<(String, Option<String>)> {
        cx.update(|window, cx| {
            let menu = ContextMenu::current(window, cx).expect("the menu is open");
            menu.read(cx)
                .items()
                .iter()
                .filter(|item| !item.is_separator())
                .map(|item| {
                    (
                        item.title().to_string(),
                        item.explicit_key_hint().map(|hint| hint.text()),
                    )
                })
                .collect()
        })
    };
    let hint_of = |label: &str, cx: &mut VisualTestContext| {
        hints(cx)
            .into_iter()
            .find(|(title, _)| title == label)
            .unwrap_or_else(|| panic!("{label} is in the menu"))
            .1
    };
    // The keys act on the key the app shows for the command: ⌃↩.
    press(cx, "ctrl-enter");
    assert_eq!(hint_of("Change ⌃↩…", cx).as_deref(), Some("↩"));
    assert_eq!(hint_of("Remove ⌃↩", cx).as_deref(), Some("⌘⌫"));
    assert_eq!(hint_of("Change ⇧F10…", cx), None);
    assert_eq!(hint_of("Remove ⇧F10", cx), None);
    press(cx, "escape");
    // After tab they act on the other one.
    press(cx, "tab ctrl-enter");
    assert_eq!(hint_of("Change ⇧F10…", cx).as_deref(), Some("↩"));
    assert_eq!(hint_of("Remove ⇧F10", cx).as_deref(), Some("⌘⌫"));
    assert_eq!(hint_of("Change ⌃↩…", cx), None);
    assert_eq!(hint_of("Remove ⌃↩", cx), None);
}
