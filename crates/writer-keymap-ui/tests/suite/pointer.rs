//! Everything with the pointer alone: simulated clicks on what is drawn.

use gpui::{Modifiers, TestAppContext};
use writer_keymap_ui::{Choice, ModeKind, Request};
use writer_ui::ContextMenu;

use super::*;

fn mode(view: &Entity<KeymapView>, cx: &mut VisualTestContext) -> ModeKind {
    view.read_with(cx, |view, _| view.mode())
}

/// Scrolls `target` into view by selecting it, and gives the number of its
/// row.
fn row(view: &Entity<KeymapView>, target: &Target, cx: &mut VisualTestContext) -> usize {
    select(view, target, cx);
    view.read_with(cx, |view, _| view.row_of(target).unwrap())
}

fn click_choice(view: &Entity<KeymapView>, choice: Choice, cx: &mut VisualTestContext) {
    let index = view.read_with(cx, |view, _| {
        view.choices()
            .iter()
            .position(|(have, _)| *have == choice)
            .unwrap_or_else(|| panic!("{choice:?} is offered: {:?}", view.choices()))
    });
    click(cx, selector(format!("keymap-choice-{index}")));
}

#[gpui::test]
fn a_click_selects_a_row_and_a_double_click_changes_its_key(cx: &mut TestAppContext) {
    let (host, view, cx) = open(cx);
    let italic_row = row(&view, &italic(), cx);
    let bold_row = row(&view, &bold(), cx);
    click(cx, selector(format!("keymap-row-{italic_row}")));
    assert_eq!(
        view.read_with(cx, |view, _| view.selected_target().cloned()),
        Some(italic())
    );
    assert_eq!(mode(&view, cx), ModeKind::List);
    // The keyboard stayed in the query field.
    cx.simulate_input("x");
    assert_eq!(view.read_with(cx, |view, cx| view.query(cx)), "x");
    view.update(cx, |view, cx| view.set_query("", cx));

    let at = bounds(cx, selector(format!("keymap-row-{bold_row}"))).center();
    cx.simulate_click(at, Modifiers::none());
    cx.simulate_event(gpui::MouseDownEvent {
        button: gpui::MouseButton::Left,
        position: at,
        modifiers: Modifiers::none(),
        click_count: 2,
        first_mouse: false,
    });
    cx.simulate_event(gpui::MouseUpEvent {
        button: gpui::MouseButton::Left,
        position: at,
        modifiers: Modifiers::none(),
        click_count: 2,
    });
    cx.run_until_parked();
    assert_eq!(mode(&view, cx), ModeKind::Recording);
    assert_eq!(
        view.read_with(cx, |view, _| view.recording_target().cloned()),
        Some(bold())
    );
    click(cx, "keymap-record-cancel");
    assert_eq!(mode(&view, cx), ModeKind::List);
    assert!(requests(&host, cx).is_empty());
}

#[gpui::test]
fn a_click_on_a_key_changes_that_key(cx: &mut TestAppContext) {
    let (host, view, cx) = open(cx);
    let bold_row = row(&view, &bold(), cx);
    click(cx, selector(format!("keymap-key-{bold_row}-0")));
    assert_eq!(mode(&view, cx), ModeKind::Recording);
    assert!(describe(&view, cx).contains("for=\"Bold\" replacing=cmd-b"));
    // The key itself has to be pressed; the answer is given with the
    // pointer.
    press(cx, "cmd-alt-b");
    assert_eq!(mode(&view, cx), ModeKind::Question);
    click_choice(&view, Choice::UseKey, cx);
    assert_eq!(keys_of(&host, &bold(), cx), vec!["cmd-alt-b"]);
    assert_eq!(mode(&view, cx), ModeKind::List);

    // The second key of a row with two.
    let menu = Target::new("demo::OpenContextMenu", Some("Editor"));
    let menu_row = row(&view, &menu, cx);
    click(cx, selector(format!("keymap-key-{menu_row}-1")));
    assert!(
        describe(&view, cx).contains("replacing=ctrl-enter"),
        "{}",
        describe(&view, cx)
    );
    press(cx, "f9");
    // "Press Another Key" with the pointer, then cancel with it.
    click_choice(&view, Choice::PressAnotherKey, cx);
    assert_eq!(mode(&view, cx), ModeKind::Recording);
    press(cx, "f8");
    click_choice(&view, Choice::Cancel, cx);
    assert_eq!(mode(&view, cx), ModeKind::List);
    assert_eq!(keys_of(&host, &menu, cx), vec!["shift-f10", "ctrl-enter"]);
}

#[gpui::test]
fn the_small_controls_of_a_row_add_a_key_and_open_its_menu(cx: &mut TestAppContext) {
    let (host, view, cx) = open(cx);
    let bold_row = row(&view, &bold(), cx);
    click(cx, selector(format!("keymap-add-{bold_row}")));
    assert_eq!(mode(&view, cx), ModeKind::Recording);
    assert!(describe(&view, cx).contains("replacing=nothing"));
    press(cx, "cmd-alt-b");
    click_choice(&view, Choice::UseKey, cx);
    assert_eq!(keys_of(&host, &bold(), cx), vec!["cmd-b", "cmd-alt-b"]);

    // "More": the menu a right-click opens.
    click(cx, selector(format!("keymap-more-{bold_row}")));
    let labels = cx.update(|window, cx| {
        ContextMenu::current(window, cx)
            .map(|menu| menu.read(cx).labels())
            .unwrap_or_default()
    });
    assert!(labels.contains(&"Remove ⌘B".to_string()), "{labels:?}");
    // An item of it, clicked.
    let item = cx.update(|window, cx| {
        let menu = ContextMenu::current(window, cx).unwrap();
        let index = menu
            .read(cx)
            .labels()
            .iter()
            .position(|label| label == "Remove ⌘B")
            .unwrap();
        menu.read(cx).item_bounds(index).unwrap()
    });
    cx.simulate_click(item.center(), Modifiers::none());
    cx.run_until_parked();
    assert_eq!(keys_of(&host, &bold(), cx), vec!["cmd-alt-b"]);
    assert!(matches!(
        requests(&host, cx).last(),
        Some(Request::RemoveKey { .. })
    ));
}

#[gpui::test]
fn a_right_click_on_a_row_opens_its_menu(cx: &mut TestAppContext) {
    let (host, view, cx) = open(cx);
    let italic_row = row(&view, &italic(), cx);
    // Another row is selected; the right-click selects the one under it.
    select(&view, &bold(), cx);
    right_click(cx, selector(format!("keymap-row-{italic_row}")));
    assert_eq!(
        view.read_with(cx, |view, _| view.selected_target().cloned()),
        Some(italic())
    );
    let labels = cx.update(|window, cx| {
        ContextMenu::current(window, cx)
            .map(|menu| menu.read(cx).labels())
            .unwrap_or_default()
    });
    assert_eq!(labels[0], "Change Key…");
    assert!(labels.contains(&"Use Default (⌘I)".to_string()));
    let chosen = cx.update(|window, cx| ContextMenu::choose("Remove Key", window, cx));
    assert!(chosen);
    cx.run_until_parked();
    assert!(keys_of(&host, &italic(), cx).is_empty());
    // Put back from the menu, which now offers it.
    right_click(cx, selector(format!("keymap-row-{italic_row}")));
    let chosen = cx.update(|window, cx| ContextMenu::choose("Use Default (⌘I)", window, cx));
    assert!(chosen);
    cx.run_until_parked();
    assert_eq!(keys_of(&host, &italic(), cx), vec!["cmd-i"]);
}

#[gpui::test]
fn import_export_and_reset_all_have_controls(cx: &mut TestAppContext) {
    let (host, view, cx) = open(cx);
    click(cx, "keymap-export");
    assert_eq!(requests(&host, cx), vec![Request::Export]);
    let file = host.read_with(cx, |host, _| host.exported.clone().unwrap());
    assert!(file.contains("\"format\": \"the-writer-shortcuts\""));

    // Reset All does nothing while nothing is changed.
    click(cx, "keymap-reset-all");
    assert_eq!(mode(&view, cx), ModeKind::List);

    select(&view, &bold(), cx);
    press(cx, "enter cmd-alt-b enter");
    select(&view, &italic(), cx);
    press(cx, "cmd-backspace");
    click(cx, "keymap-reset-all");
    assert_eq!(mode(&view, cx), ModeKind::Question);
    view.read_with(cx, |view, _| {
        assert_eq!(
            view.question_title().as_deref(),
            Some("Put every key back to its default?")
        );
        // It says how many go back.
        assert!(
            view.question_lines()[0]
                .text
                .starts_with("2 commands go back")
        );
    });
    // Cancel with the pointer: nothing happens.
    click_choice(&view, Choice::Cancel, cx);
    assert_eq!(keys_of(&host, &bold(), cx), vec!["cmd-alt-b"]);
    click(cx, "keymap-reset-all");
    click_choice(&view, Choice::ResetAll, cx);
    assert_eq!(keys_of(&host, &bold(), cx), vec!["cmd-b"]);
    assert_eq!(keys_of(&host, &italic(), cx), vec!["cmd-i"]);
    assert_eq!(view.read_with(cx, |view, _| view.changed_count()), 0);
    assert!(matches!(
        requests(&host, cx).last(),
        Some(Request::ResetAll)
    ));

    // Import: the host is asked for a file.
    host.update(cx, |host, _| host.import_file = None);
    click(cx, "keymap-import");
    assert!(matches!(
        requests(&host, cx).last(),
        Some(Request::ChooseImportFile)
    ));
}

#[gpui::test]
fn a_click_elsewhere_leaves_a_question_without_changing_anything(cx: &mut TestAppContext) {
    let (host, view, cx) = open(cx);
    let italic_row = row(&view, &italic(), cx);
    select(&view, &bold(), cx);
    press(cx, "enter cmd-alt-b");
    assert_eq!(mode(&view, cx), ModeKind::Question);
    // On another row: the question is over, that row is selected.
    click(cx, selector(format!("keymap-row-{italic_row}")));
    assert_eq!(mode(&view, cx), ModeKind::List);
    assert_eq!(
        view.read_with(cx, |view, _| view.selected_target().cloned()),
        Some(italic())
    );
    assert!(requests(&host, cx).is_empty());
    // On the query field while the view waits for a key: back to the list,
    // and the next key is typed, not recorded.
    press(cx, "enter");
    assert_eq!(mode(&view, cx), ModeKind::Recording);
    let field = view.read_with(cx, |view, cx| view.focus_handle(cx));
    drop(field);
    let panel = bounds(cx, "keymap-panel");
    cx.simulate_click(
        gpui::point(panel.left() + gpui::px(200.), panel.top() + gpui::px(24.)),
        Modifiers::none(),
    );
    cx.run_until_parked();
    assert_eq!(mode(&view, cx), ModeKind::List);
    cx.simulate_input("b");
    assert_eq!(view.read_with(cx, |view, cx| view.query(cx)), "b");
}
