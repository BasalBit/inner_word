//! A rebuilt keymap: the user's own shortcuts reach GPUI by clearing its
//! keymap and binding it again with other keystrokes
//! (`writer_ui::keymap::replace_key` does that for one key).
//!
//! Vim mode has no bindings of its own: its grammar is a language the
//! engine reads key by key, before the editor's bindings, and cannot be
//! changed that way. What it does with the keys of others must follow the
//! keymap all the same: the keys it lets through to the editor, and the
//! one key it knows by its meaning, the key of the context menu.

use std::{cell::RefCell, rc::Rc};

use gpui::{Action, TestAppContext, VisualTestContext};
use writer_editor::{
    ContextMenuRequest, EditorEvent,
    actions::{MoveBlockDown, OpenContextMenu, ToggleBold},
};
use writer_ui::{KeyHint, keymap};
use writer_vim_mode::{Mode, ToggleVimMode};

use super::*;

fn hint(action: &dyn Action, cx: &mut VisualTestContext) -> Option<String> {
    cx.update(|_, cx| KeyHint::for_action(action, cx).map(|hint| hint.text()))
}

fn rebind(action: &dyn Action, from: &str, to: &str, cx: &mut VisualTestContext) -> usize {
    let changed = cx.update(|_, cx| keymap::replace_key(action, from, Some(to), cx));
    cx.run_until_parked();
    changed
}

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

/// The key of the context menu is passed to the editor in every mode but
/// the command line. Which key that is, is the keymap's answer: here it
/// becomes `ctrl-m`, which is a command of Vim otherwise (`enter`).
#[gpui::test]
fn the_key_of_the_context_menu_is_let_through_whatever_it_is(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["alpha beta gamma", "second line"]), cx);
    let requests = menu_requests(&rig, cx);

    // Before: `ctrl-m` is Vim's (down a line), `ctrl-enter` the menu's.
    cx.simulate_keystrokes("ctrl-m");
    assert_eq!(requests.borrow().len(), 0);
    assert_eq!(rig.head(cx).block, 1);
    cx.simulate_keystrokes("g g ctrl-enter");
    assert_eq!(requests.borrow().len(), 1);
    assert_eq!(rig.head(cx), at(0, 0));

    assert_eq!(rebind(&OpenContextMenu, "ctrl-enter", "ctrl-m", cx), 1);
    assert_eq!(hint(&OpenContextMenu, cx).as_deref(), Some("⌃M"));

    // Normal mode: the new key asks for the menu and moves nothing.
    cx.simulate_keystrokes("w ctrl-m");
    assert_eq!(requests.borrow().len(), 2);
    assert!(requests.borrow()[1].from_keyboard);
    assert_eq!(rig.mode(cx), Mode::Normal);
    assert_eq!(rig.selection(cx), Selection::caret(at(0, 6)));
    // The old key asks for nothing: it is the engine's again.
    cx.simulate_keystrokes("ctrl-enter");
    assert_eq!(requests.borrow().len(), 2, "the old key opens no menu");
    cx.simulate_keystrokes("g g w");

    // Visual mode: for the selection, which stays.
    cx.simulate_keystrokes("v e ctrl-m");
    assert_eq!(requests.borrow().len(), 3);
    assert!(requests.borrow()[2].has_selection);
    assert_eq!(rig.mode(cx), Mode::Visual);
    assert_eq!(rig.selected_text(cx), "beta");
    cx.simulate_keystrokes("escape");

    // A half-typed command is dropped, not carried out on the next key.
    cx.simulate_keystrokes("d ctrl-m");
    assert_eq!(requests.borrow().len(), 4);
    assert_eq!(rig.status(cx).pending_keys, "");

    // Insert mode.
    cx.simulate_keystrokes("i ctrl-m");
    assert_eq!(requests.borrow().len(), 5);
    assert_eq!(rig.mode(cx), Mode::Insert);
    assert_eq!(rig.text(cx), "alpha beta gamma\nsecond line");
    cx.simulate_keystrokes("escape");

    // The second key of the menu was not changed.
    cx.simulate_keystrokes("shift-f10");
    assert_eq!(requests.borrow().len(), 6);

    // An open command line keeps the key: there it ends the line.
    cx.simulate_keystrokes(":");
    assert_eq!(rig.mode(cx), Mode::CommandLine);
    cx.simulate_keystrokes("ctrl-m");
    assert_eq!(requests.borrow().len(), 6);
    assert_eq!(rig.mode(cx), Mode::Normal);
}

/// The keys the engine leaves to the editor reach whatever the keymap binds
/// them to now: a `cmd` chord in every mode, an option chord in Normal
/// mode.
#[gpui::test]
fn the_keys_vim_lets_through_do_what_the_keymap_says_now(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["first word", "second"]), cx);
    let bold = |cx: &mut VisualTestContext| {
        rig.editor.read_with(cx, |editor, _| {
            editor.document().blocks()[0].attrs_at(0).unwrap().bold
        })
    };
    assert_eq!(rebind(&ToggleBold, "cmd-b", "cmd-alt-b", cx), 1);
    cx.simulate_keystrokes("v e cmd-b");
    assert!(!bold(cx), "the old key does nothing");
    assert_eq!(rig.mode(cx), Mode::Visual);
    cx.simulate_keystrokes("cmd-alt-b");
    assert!(bold(cx));
    assert_eq!(rig.mode(cx), Mode::Visual, "bold keeps the selection");
    cx.simulate_keystrokes("escape");
    assert_eq!(hint(&ToggleBold, cx).as_deref(), Some("⌥⌘B"));

    assert_eq!(rebind(&MoveBlockDown, "alt-down", "alt-j", cx), 1);
    cx.simulate_keystrokes("alt-down");
    assert_eq!(rig.text(cx), "first word\nsecond", "the old key");
    cx.simulate_keystrokes("alt-j");
    assert_eq!(rig.text(cx), "second\nfirst word");
    assert_eq!(rig.mode(cx), Mode::Normal);
}

/// The menu of the mode chip shows the key of "turn off Vim mode" as the
/// keymap has it when the menu is drawn.
#[gpui::test]
fn the_menu_of_the_mode_names_the_key_of_the_switch_as_it_is(cx: &mut TestAppContext) {
    setup(cx);
    cx.update(|cx| cx.bind_keys([gpui::KeyBinding::new("cmd-alt-v", ToggleVimMode, None)]));
    let (rig, cx) = open(doc(&["text"]), cx);
    let shown = |cx: &mut VisualTestContext| {
        let items = rig
            .status_item
            .update_in(cx, |status, window, cx| status.menu_items(window, cx));
        let item = items
            .iter()
            .find(|item| item.title().as_ref() == "Turn off Vim mode")
            .expect("the switch is in the menu");
        assert!(item.explicit_key_hint().is_none(), "its key is not kept");
        let action = item.action_ref().expect("it is an action").boxed_clone();
        hint(action.as_ref(), cx)
    };
    assert_eq!(shown(cx).as_deref(), Some("⌥⌘V"));
    assert_eq!(rebind(&ToggleVimMode, "cmd-alt-v", "cmd-alt-i", cx), 1);
    assert_eq!(shown(cx).as_deref(), Some("⌥⌘I"));
    // And the key works from Normal mode, where every other key is Vim's.
    cx.simulate_keystrokes("cmd-alt-v");
    assert!(rig.status(cx).attached, "the old key");
    cx.simulate_keystrokes("cmd-alt-i");
    assert!(!rig.status(cx).attached);
}
