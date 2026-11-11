//! The mouse with Vim mode on, as in a graphical Vim: real mouse events on
//! a real editor.

use std::{cell::RefCell, rc::Rc};

use gpui::{
    Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, Pixels, Point, TestAppContext,
    VisualTestContext, point, px,
};
use writer_core::{Position, Selection};
use writer_editor::{
    ContextMenuRequest, EditorEvent,
    actions::{Backspace, ToggleBold},
};
use writer_ui::{ContextMenu, ContextMenuItem};
use writer_vim_mode::{Mode, ToggleVimMode};

use super::{Rig, at, doc, open, setup};

/// A point on the character that starts at `offset` of `block`.
fn on(rig: &Rig, block: usize, offset: usize, cx: &mut VisualTestContext) -> Point<Pixels> {
    rig.editor.read_with(cx, |editor, _| {
        editor
            .position_bounds(Position::new(block, offset))
            .unwrap()
            .center()
    }) + point(px(2.), px(0.))
}

fn click_with(cx: &mut VisualTestContext, at: Point<Pixels>, button: MouseButton, count: usize) {
    cx.simulate_event(MouseDownEvent {
        position: at,
        modifiers: Modifiers::none(),
        button,
        click_count: count,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        position: at,
        modifiers: Modifiers::none(),
        button,
        click_count: count,
    });
    cx.run_until_parked();
}

fn click(cx: &mut VisualTestContext, at: Point<Pixels>) {
    click_with(cx, at, MouseButton::Left, 1);
}

fn drag(cx: &mut VisualTestContext, from: Point<Pixels>, to: Point<Pixels>) {
    cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(to, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(to, MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
}

/// Collects the context menu requests of the editor.
fn menu_requests(rig: &Rig, cx: &mut VisualTestContext) -> Rc<RefCell<Vec<ContextMenuRequest>>> {
    let requests = Rc::new(RefCell::new(Vec::new()));
    cx.update(|_, cx| {
        cx.subscribe(&rig.editor, {
            let requests = requests.clone();
            move |_, event: &EditorEvent, _| {
                if let EditorEvent::ContextMenuRequested(request) = event {
                    requests.borrow_mut().push(request.clone());
                }
            }
        })
        .detach();
    });
    requests
}

fn editor_focused(rig: &Rig, cx: &mut VisualTestContext) -> bool {
    let editor = rig.editor.clone();
    cx.update(|window, cx| editor.read(cx).is_focused(window))
}

#[gpui::test]
fn a_click_in_normal_mode_puts_the_caret_on_a_character(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["alpha beta gamma", "second line", ""]), cx);

    let target = on(&rig, 1, 7, cx);
    click(cx, target);
    assert_eq!(rig.mode(cx), Mode::Normal);
    assert_eq!(rig.selection(cx), Selection::caret(at(1, 7)));
    assert_eq!(rig.drawn_caret(cx), at(1, 7));

    // Past the end of a line: on its last character, not after it.
    let target = on(&rig, 0, 16, cx) + point(px(40.), px(0.));
    click(cx, target);
    assert_eq!(rig.mode(cx), Mode::Normal);
    assert_eq!(rig.selection(cx), Selection::caret(at(0, 15)));
    // The keys go on from there.
    cx.simulate_keystrokes("x");
    assert_eq!(rig.text(cx), "alpha beta gamm\nsecond line\n");

    // An empty line has one place.
    let target = on(&rig, 2, 0, cx);
    click(cx, target);
    assert_eq!(rig.selection(cx), Selection::caret(at(2, 0)));
    assert_eq!(rig.mode(cx), Mode::Normal);

    // A click drops a half-typed command.
    cx.simulate_keystrokes("2 d");
    assert_eq!(rig.status(cx).pending_keys, "2d");
    let target = on(&rig, 1, 3, cx);
    click(cx, target);
    assert_eq!(rig.status(cx).pending_keys, "");
    assert_eq!(rig.mode(cx), Mode::Normal);
    cx.simulate_keystrokes("w");
    assert_eq!(rig.text(cx), "alpha beta gamm\nsecond line\n");
    assert_eq!(rig.head(cx), at(1, 7));
}

#[gpui::test]
fn a_drag_starts_visual_mode_and_a_click_ends_it(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["alpha beta gamma", "second line"]), cx);

    // Press on the "b" of beta, move to its last "a", still held.
    let (from, to) = (on(&rig, 0, 6, cx), on(&rig, 0, 9, cx));
    cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
    assert_eq!(rig.mode(cx), Mode::Normal);
    cx.simulate_mouse_move(to, MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
    assert_eq!(rig.mode(cx), Mode::Visual);
    assert_eq!(rig.context_mode(cx).as_deref(), Some("visual"));
    assert_eq!(rig.selected_text(cx), "bet");
    // The Vim cursor is on the last selected character and drawn there.
    assert_eq!(rig.cursor(cx), at(0, 8));
    assert_eq!(rig.drawn_caret(cx), at(0, 8));
    // Back over the start and beyond: the selection turns round.
    let back = on(&rig, 0, 2, cx);
    cx.simulate_mouse_move(back, MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
    assert_eq!(rig.mode(cx), Mode::Visual);
    assert_eq!(rig.selected_text(cx), "pha ");
    assert_eq!(rig.cursor(cx), at(0, 2));
    cx.simulate_mouse_move(to, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(to, MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
    assert_eq!(rig.mode(cx), Mode::Visual);
    assert_eq!(rig.selected_text(cx), "bet");
    assert!(!rig.text_input(cx));

    // Vim keys work on it: extend, then delete.
    cx.simulate_keystrokes("l");
    assert_eq!(rig.selected_text(cx), "beta");
    cx.simulate_keystrokes("d");
    assert_eq!(rig.mode(cx), Mode::Normal);
    assert_eq!(rig.text(cx), "alpha  gamma\nsecond line");

    // Over two lines.
    let (from, to) = (on(&rig, 0, 6, cx), on(&rig, 1, 3, cx));
    drag(cx, from, to);
    assert_eq!(rig.mode(cx), Mode::Visual);
    assert_eq!(
        rig.selection(cx),
        Selection::new(at(0, 6), at(1, 3)),
        "the editor's selection is the one the drag made"
    );
    // A plain click collapses it: Normal mode, caret on the character.
    let target = on(&rig, 1, 5, cx);
    click(cx, target);
    assert_eq!(rig.mode(cx), Mode::Normal);
    assert_eq!(rig.selection(cx), Selection::caret(at(1, 5)));
    assert_eq!(rig.caret_override(cx), None);

    // Shift-click extends from the caret.
    let target = on(&rig, 1, 9, cx);
    cx.simulate_click(target, Modifiers::shift());
    cx.run_until_parked();
    assert_eq!(rig.mode(cx), Mode::Visual);
    assert_eq!(rig.selected_text(cx), "d li");
    cx.simulate_keystrokes("escape");
    assert_eq!(rig.mode(cx), Mode::Normal);
    assert!(rig.selection(cx).is_collapsed());
}

#[gpui::test]
fn double_and_triple_clicks_select_in_visual_mode(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["alpha beta gamma", "second line"]), cx);

    let target = on(&rig, 0, 7, cx);
    click_with(cx, target, MouseButton::Left, 2);
    assert_eq!(rig.mode(cx), Mode::Visual);
    assert_eq!(rig.selected_text(cx), "beta");
    assert_eq!(rig.cursor(cx), at(0, 9));
    cx.simulate_keystrokes("c");
    assert_eq!(rig.mode(cx), Mode::Insert);
    assert_eq!(rig.text(cx), "alpha  gamma\nsecond line");
    cx.simulate_keystrokes("escape");

    let target = on(&rig, 1, 3, cx);
    click_with(cx, target, MouseButton::Left, 3);
    assert_eq!(rig.mode(cx), Mode::Visual);
    assert_eq!(rig.selected_text(cx), "second line");
    cx.simulate_keystrokes("y");
    assert_eq!(rig.mode(cx), Mode::Normal);
    assert!(rig.selection(cx).is_collapsed());
}

#[gpui::test]
fn clicks_in_insert_mode_only_move_the_caret(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["alpha beta", "second"]), cx);
    cx.simulate_keystrokes("i");
    assert_eq!(rig.mode(cx), Mode::Insert);

    // After the last character is a place in Insert mode.
    let target = on(&rig, 0, 10, cx) + point(px(40.), px(0.));
    click(cx, target);
    assert_eq!(rig.mode(cx), Mode::Insert);
    assert_eq!(rig.selection(cx), Selection::caret(at(0, 10)));
    assert!(rig.text_input(cx));
    cx.simulate_input("!");
    assert_eq!(rig.text(cx), "alpha beta!\nsecond");

    // A drag selects and typing replaces it, still in Insert mode.
    let (from, to) = (
        on(&rig, 1, 0, cx) - point(px(2.), px(0.)),
        on(&rig, 1, 3, cx),
    );
    drag(cx, from, to);
    assert_eq!(rig.mode(cx), Mode::Insert);
    assert_eq!(rig.selected_text(cx), "sec");
    cx.simulate_input("x");
    assert_eq!(rig.text(cx), "alpha beta!\nxond");
    cx.simulate_keystrokes("escape");
    assert_eq!(rig.mode(cx), Mode::Normal);
    assert_eq!(rig.head(cx), at(1, 0));
}

#[gpui::test]
fn a_right_click_selects_the_word_in_visual_mode(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["alpha beta gamma", "second line"]), cx);
    let requests = menu_requests(&rig, cx);

    // On a word: the editor selects it, Vim follows into Visual mode.
    let target = on(&rig, 0, 7, cx);
    click_with(cx, target, MouseButton::Right, 1);
    assert_eq!(requests.borrow().len(), 1);
    assert!(requests.borrow()[0].has_selection);
    assert_eq!(rig.mode(cx), Mode::Visual);
    assert_eq!(rig.selected_text(cx), "beta");
    assert_eq!(rig.cursor(cx), at(0, 9));
    assert!(editor_focused(&rig, cx));

    // The menu is dismissed without a choice: the selection is a Visual
    // selection like any other, and `escape` leaves it.
    cx.simulate_keystrokes("escape");
    assert_eq!(rig.mode(cx), Mode::Normal);
    assert_eq!(rig.selection(cx), Selection::caret(at(0, 9)));

    // A menu item that works on the selection without removing it (bold).
    click_with(cx, target, MouseButton::Right, 1);
    assert_eq!(rig.mode(cx), Mode::Visual);
    rig.editor.update(cx, |editor, cx| {
        editor.edit(|state| state.toggle_bold(), cx);
    });
    cx.run_until_parked();
    assert_eq!(rig.mode(cx), Mode::Visual);
    assert_eq!(rig.selected_text(cx), "beta");
    // A right-click inside the selection keeps it.
    let inside = on(&rig, 0, 8, cx);
    click_with(cx, inside, MouseButton::Right, 1);
    assert_eq!(rig.mode(cx), Mode::Visual);
    assert_eq!(rig.selected_text(cx), "beta");
    // An item that removes it (cut, without the clipboard): Normal mode
    // with the caret on a character.
    rig.editor.update(cx, |editor, cx| {
        editor.edit(|state| state.delete_selection(), cx);
    });
    cx.run_until_parked();
    assert_eq!(rig.mode(cx), Mode::Normal);
    assert_eq!(rig.text(cx), "alpha  gamma\nsecond line");
    assert_eq!(rig.selection(cx), Selection::caret(at(0, 6)));
    assert!(!rig.text_input(cx));

    // Not on a word (the space after the last word of the line): only the
    // caret moves, clamped onto a character.
    let target = on(&rig, 1, 11, cx) + point(px(60.), px(0.));
    click_with(cx, target, MouseButton::Right, 1);
    assert_eq!(rig.mode(cx), Mode::Normal);
    assert_eq!(rig.selection(cx), Selection::caret(at(1, 10)));

    // A half-typed command does not wait behind the menu.
    cx.simulate_keystrokes("d");
    assert_eq!(rig.status(cx).pending_keys, "d");
    click_with(cx, target, MouseButton::Right, 1);
    assert_eq!(rig.status(cx).pending_keys, "");
    assert_eq!(rig.mode(cx), Mode::Normal);
    assert_eq!(requests.borrow().len(), 5);
}

#[gpui::test]
fn a_right_click_in_insert_mode_stays_in_insert_mode(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["alpha beta gamma"]), cx);
    let requests = menu_requests(&rig, cx);
    cx.simulate_keystrokes("i");
    let target = on(&rig, 0, 7, cx);
    click_with(cx, target, MouseButton::Right, 1);
    assert_eq!(requests.borrow().len(), 1);
    assert_eq!(rig.mode(cx), Mode::Insert);
    assert_eq!(rig.selected_text(cx), "beta");
    assert!(rig.text_input(cx));
    cx.simulate_input("B");
    assert_eq!(rig.text(cx), "alpha B gamma");
}

#[gpui::test]
fn ctrl_enter_asks_for_the_context_menu_in_every_mode(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["alpha beta gamma", "second line"]), cx);
    let requests = menu_requests(&rig, cx);

    // Normal mode: at the caret, nothing selected, nothing moved.
    cx.simulate_keystrokes("w");
    cx.simulate_keystrokes("ctrl-enter");
    assert_eq!(requests.borrow().len(), 1);
    let request = requests.borrow()[0].clone();
    assert!(request.from_keyboard);
    assert!(!request.has_selection);
    assert_eq!(request.document_position, at(0, 6));
    assert_eq!(rig.mode(cx), Mode::Normal);
    assert_eq!(rig.selection(cx), Selection::caret(at(0, 6)));

    // Visual mode: for the selection, which stays.
    cx.simulate_keystrokes("v e ctrl-enter");
    assert_eq!(requests.borrow().len(), 2);
    assert!(requests.borrow()[1].has_selection);
    assert_eq!(rig.mode(cx), Mode::Visual);
    assert_eq!(rig.selected_text(cx), "beta");
    cx.simulate_keystrokes("escape");

    // A half-typed command is dropped, not carried out on the next key.
    cx.simulate_keystrokes("d ctrl-enter");
    assert_eq!(requests.borrow().len(), 3);
    assert_eq!(rig.status(cx).pending_keys, "");
    assert_eq!(rig.mode(cx), Mode::Normal);

    // Insert mode.
    cx.simulate_keystrokes("i ctrl-enter");
    assert_eq!(requests.borrow().len(), 4);
    assert_eq!(rig.mode(cx), Mode::Insert);
    assert_eq!(rig.text(cx), "alpha beta gamma\nsecond line");
}

#[gpui::test]
fn the_editors_undo_of_a_step_on_a_selection_stays_in_normal_mode(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["Alpha beta"]), cx);
    // A step made on a selection (bold on a word that was dragged over).
    let (from, to) = (
        on(&rig, 0, 6, cx),
        on(&rig, 0, 10, cx) + point(px(30.), px(0.)),
    );
    drag(cx, from, to);
    assert_eq!(rig.mode(cx), Mode::Visual);
    cx.simulate_keystrokes("cmd-b escape");
    assert_eq!(rig.mode(cx), Mode::Normal);
    assert!(rig.selection(cx).is_collapsed());

    // cmd-z (or Undo in the Edit menu) brings the selection back with the
    // text; Vim stays in Normal mode with the caret at its start.
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(rig.mode(cx), Mode::Normal);
    assert_eq!(rig.selection(cx), Selection::caret(at(0, 6)));
    assert_eq!(rig.caret_override(cx), None);
    // The next operator key works on a motion, not on a stale selection.
    cx.simulate_keystrokes("d");
    assert_eq!(rig.text(cx), "Alpha beta");
    assert_eq!(rig.status(cx).pending_keys, "d");
    cx.simulate_keystrokes("escape");
    // Redo the same way.
    cx.simulate_keystrokes("cmd-shift-z");
    assert_eq!(rig.mode(cx), Mode::Normal);
    assert!(rig.selection(cx).is_collapsed());
}

#[gpui::test]
fn the_command_line_takes_clicks(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["alpha beta gamma"]), cx);
    cx.simulate_keystrokes(": s / a");
    assert_eq!(rig.mode(cx), Mode::CommandLine);
    let strip = cx.debug_bounds("vim-command-line").expect("the strip");

    // The keyboard went somewhere else; a click on the line brings it
    // back, and typing goes on where it was.
    cx.update(|window, _| window.blur());
    cx.run_until_parked();
    assert!(!editor_focused(&rig, cx));
    assert_eq!(rig.mode(cx), Mode::CommandLine, "blur keeps the line");
    click(cx, point(strip.left() + px(60.), strip.center().y));
    assert!(editor_focused(&rig, cx));
    cx.simulate_keystrokes("l");
    assert_eq!(rig.status(cx).command_line.unwrap().text, "s/al");

    // The close control does what escape does.
    let close = cx.debug_bounds("vim-command-line-close").expect("close");
    assert!(strip.contains(&close.center()));
    click(cx, close.center());
    assert_eq!(rig.mode(cx), Mode::Normal);
    assert_eq!(rig.status(cx).command_line, None);
    assert!(editor_focused(&rig, cx));
    assert_eq!(rig.text(cx), "alpha beta gamma");
    assert!(!rig.command_line.read_with(cx, |line, cx| line.is_open(cx)));

    // With the keyboard elsewhere the close control still closes it and
    // returns the keyboard to the text.
    cx.simulate_keystrokes("/ b e");
    cx.update(|window, _| window.blur());
    cx.run_until_parked();
    let close = cx.debug_bounds("vim-command-line-close").expect("close");
    click(cx, close.center());
    assert_eq!(rig.mode(cx), Mode::Normal);
    assert!(editor_focused(&rig, cx));

    // A click in the text closes an open command line.
    cx.simulate_keystrokes(": d");
    assert_eq!(rig.mode(cx), Mode::CommandLine);
    let target = on(&rig, 0, 7, cx);
    click(cx, target);
    assert_eq!(rig.mode(cx), Mode::Normal);
    assert_eq!(rig.selection(cx), Selection::caret(at(0, 7)));
    assert_eq!(rig.text(cx), "alpha beta gamma");
}

/// Opens a menu for every context menu request of the editor, as the
/// tools crate does in the app.
fn show_text_menus(rig: &Rig, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        let handle = window.window_handle();
        cx.subscribe(&rig.editor, move |_, event: &EditorEvent, cx| {
            let EditorEvent::ContextMenuRequested(request) = event else {
                return;
            };
            let position = request.position;
            cx.defer(move |cx| {
                handle
                    .update(cx, |_, window, cx| {
                        ContextMenu::open(
                            position,
                            vec![
                                ContextMenuItem::action("Bold", ToggleBold),
                                ContextMenuItem::action("Delete", Backspace),
                            ],
                            window,
                            cx,
                        );
                    })
                    .ok();
            });
        })
        .detach();
    });
}

fn menu_open(cx: &mut VisualTestContext) -> bool {
    cx.update(|window, cx| ContextMenu::is_open(window, cx))
}

fn choose(title: &str, cx: &mut VisualTestContext) {
    let chosen = cx.update(|window, cx| ContextMenu::choose(title, window, cx));
    assert!(chosen, "no item {title:?} to choose");
    cx.run_until_parked();
}

fn bold_at(rig: &Rig, offset: usize, cx: &mut VisualTestContext) -> bool {
    rig.editor.read_with(cx, |editor, _| {
        editor
            .document()
            .block(0)
            .and_then(|block| block.attrs_at(offset))
            .is_some_and(|attrs| attrs.bold)
    })
}

#[gpui::test]
fn a_context_menu_over_the_text_leaves_vim_in_a_consistent_mode(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["alpha beta gamma"]), cx);
    show_text_menus(&rig, cx);

    // Right-click on a word: Visual mode with the word, the menu is open
    // and has the keys. They are not Vim commands while it is.
    let target = on(&rig, 0, 7, cx);
    click_with(cx, target, MouseButton::Right, 1);
    assert!(menu_open(cx));
    assert_eq!(rig.mode(cx), Mode::Visual);
    assert_eq!(rig.selected_text(cx), "beta");
    cx.simulate_keystrokes("x");
    assert_eq!(rig.text(cx), "alpha beta gamma");
    assert_eq!(rig.mode(cx), Mode::Visual);
    // The first escape closes the menu, the second leaves Visual mode.
    cx.simulate_keystrokes("escape");
    assert!(!menu_open(cx));
    assert_eq!(rig.mode(cx), Mode::Visual);
    assert_eq!(rig.selected_text(cx), "beta");
    assert!(editor_focused(&rig, cx));
    cx.simulate_keystrokes("escape");
    assert_eq!(rig.mode(cx), Mode::Normal);
    assert!(rig.selection(cx).is_collapsed());

    // An item that keeps the selection: still Visual, and Vim keys work
    // on the same selection afterwards.
    click_with(cx, target, MouseButton::Right, 1);
    choose("Bold", cx);
    assert!(!menu_open(cx));
    assert!(bold_at(&rig, 7, cx));
    assert_eq!(rig.mode(cx), Mode::Visual);
    assert_eq!(rig.selected_text(cx), "beta");
    assert!(editor_focused(&rig, cx));
    cx.simulate_keystrokes("o");
    assert_eq!(rig.cursor(cx), at(0, 6));
    cx.simulate_keystrokes("escape");

    // An item that removes it, chosen with a real click on its row.
    click_with(cx, target, MouseButton::Right, 1);
    let row = cx.update(|window, cx| {
        let menu = ContextMenu::current(window, cx).expect("an open menu");
        let menu = menu.read(cx);
        let index = menu
            .labels()
            .iter()
            .position(|label| label == "Delete")
            .expect("the item");
        menu.item_bounds(index).expect("drawn")
    });
    click(cx, row.center());
    assert!(!menu_open(cx));
    assert_eq!(rig.text(cx), "alpha  gamma");
    assert_eq!(rig.mode(cx), Mode::Normal);
    assert_eq!(rig.selection(cx), Selection::caret(at(0, 6)));
    assert!(!rig.text_input(cx));
    assert_eq!(rig.context_mode(cx).as_deref(), Some("normal"));

    // A click outside the menu closes it and does nothing else.
    let word = on(&rig, 0, 2, cx);
    click_with(cx, word, MouseButton::Right, 1);
    assert_eq!(rig.selected_text(cx), "alpha");
    let elsewhere = on(&rig, 0, 9, cx);
    click(cx, elsewhere);
    assert!(!menu_open(cx));
    assert_eq!(rig.mode(cx), Mode::Visual);
    assert_eq!(rig.selected_text(cx), "alpha");
    // The next click is an ordinary one.
    click(cx, elsewhere);
    assert_eq!(rig.mode(cx), Mode::Normal);
    assert_eq!(rig.selection(cx), Selection::caret(at(0, 9)));

    // From the keyboard, in Normal mode.
    cx.simulate_keystrokes("ctrl-enter");
    assert!(menu_open(cx));
    assert_eq!(rig.mode(cx), Mode::Normal);
    cx.simulate_keystrokes("escape");
    assert!(!menu_open(cx));
    cx.simulate_keystrokes("x");
    assert_eq!(rig.text(cx), "alpha  gama");
}

#[gpui::test]
fn the_mode_chip_opens_a_menu(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["alpha beta gamma"]), cx);
    let chip = cx.debug_bounds("vim-mode-chip").expect("the chip");
    let item = |title: &str, cx: &mut VisualTestContext| {
        cx.update(|window, cx| {
            let menu = ContextMenu::current(window, cx).expect("an open menu");
            menu.read(cx)
                .items()
                .iter()
                .find(|item| item.title().as_ref() == title)
                .cloned()
                .expect("the item")
        })
    };

    click(cx, chip.center());
    assert!(menu_open(cx));
    let labels =
        cx.update(|window, cx| ContextMenu::current(window, cx).unwrap().read(cx).labels());
    assert_eq!(
        labels,
        [
            "Normal mode",
            "Insert mode",
            "-",
            "Command line",
            "Search",
            "Clear search highlight",
            "-",
            "Turn off Vim mode"
        ]
    );
    assert_eq!(item("Normal mode", cx).is_checked(), Some(true));
    assert_eq!(item("Insert mode", cx).is_checked(), Some(false));
    assert!(item("Clear search highlight", cx).is_disabled());
    // The menu drops from the chip (above it here: the bar is at the
    // bottom of the window) and does not cover it.
    let menu = cx.update(|window, cx| ContextMenu::current(window, cx).unwrap().read(cx).bounds());
    let menu = menu.expect("the menu is drawn");
    assert!(!menu.intersects(&chip), "{menu:?} covers {chip:?}");

    choose("Insert mode", cx);
    assert_eq!(rig.mode(cx), Mode::Insert);
    assert!(editor_focused(&rig, cx));
    cx.simulate_input("X");
    assert_eq!(rig.text(cx), "Xalpha beta gamma");

    // A right-click opens the same menu.
    let chip = cx.debug_bounds("vim-mode-chip").expect("the chip");
    click_with(cx, chip.center(), MouseButton::Right, 1);
    assert_eq!(item("Insert mode", cx).is_checked(), Some(true));
    assert_eq!(item("Normal mode", cx).is_checked(), Some(false));
    choose("Normal mode", cx);
    assert_eq!(rig.mode(cx), Mode::Normal);
    assert_eq!(rig.head(cx), at(0, 0));

    // The command line and the search, also from a mode in which their
    // key would be typed.
    cx.simulate_keystrokes("i");
    click(cx, chip.center());
    choose("Command line", cx);
    assert_eq!(rig.mode(cx), Mode::CommandLine);
    assert_eq!(rig.status(cx).command_line.unwrap().prefix, ':');
    assert_eq!(rig.text(cx), "Xalpha beta gamma");
    let chip = cx.debug_bounds("vim-mode-chip").expect("the chip");
    click(cx, chip.center());
    choose("Search", cx);
    assert_eq!(rig.status(cx).command_line.unwrap().prefix, '/');
    cx.simulate_keystrokes("b e t a enter");
    assert_eq!(rig.head(cx), at(0, 7));
    assert!(rig.status(cx).search_highlighted);
    // A Visual selection is the range of the command line.
    cx.simulate_keystrokes("shift-v");
    let chip = cx.debug_bounds("vim-mode-chip").expect("the chip");
    click(cx, chip.center());
    choose("Command line", cx);
    assert_eq!(rig.status(cx).command_line.unwrap().text, "'<,'>");
    cx.simulate_keystrokes("escape");

    let chip = cx.debug_bounds("vim-mode-chip").expect("the chip");
    click(cx, chip.center());
    assert!(!item("Clear search highlight", cx).is_disabled());
    choose("Clear search highlight", cx);
    assert!(!rig.status(cx).search_highlighted);
    assert!(rig.layer(writer_vim_mode::SEARCH_LAYER, cx).is_empty());

    // The host's items and its own toggle action.
    rig.status_item.update(cx, |status, _| {
        status.extend_menu(|items, _, _| {
            items.push(ContextMenuItem::separator());
            items.push(ContextMenuItem::new("Settings", |_, _| {}));
        });
        status.set_toggle_action(ToggleVimMode);
    });
    click(cx, chip.center());
    let labels =
        cx.update(|window, cx| ContextMenu::current(window, cx).unwrap().read(cx).labels());
    assert_eq!(labels.last().map(String::as_str), Some("Settings"));
    choose("Turn off Vim mode", cx);
    assert!(!rig.vim.read_with(cx, |vim, _| vim.is_attached()));
    assert!(rig.text_input(cx));
    // No chip, no menu, while Vim mode is off.
    let items = rig
        .status_item
        .clone()
        .update_in(cx, |status, window, cx| status.menu_items(window, cx).len());
    assert_eq!(items, 0);
}
