//! A rebuilt keymap: the user's own shortcuts reach GPUI by clearing its keymap and binding
//! it again with other keystrokes (`innerword_ui::keymap::replace_key` does that for one key).
//! The panel obeys and shows the new keys at once, and the order of its bindings, which
//! decides who wins in the trash view, is the order they were bound in.

use gpui::Action;
use innerword_library::{
    NewDocument, OpenSelected, Rename, RestoreSelected, SelectFirst, SelectNext, ShowTrash,
};
use innerword_ui::{KeyHint, keymap};

use super::*;

fn hint(action: &dyn Action, cx: &mut VisualTestContext) -> Option<String> {
    cx.update(|_, cx| KeyHint::for_action(action, cx).map(|hint| hint.text()))
}

fn rebind(action: &dyn Action, from: &str, to: &str, cx: &mut VisualTestContext) -> usize {
    let changed = cx.update(|_, cx| keymap::replace_key(action, from, Some(to), cx));
    cx.run_until_parked();
    changed
}

#[gpui::test]
fn the_keys_of_the_tree_that_were_changed(cx: &mut TestAppContext) {
    let (f, cx) = fixture(cx);
    cx.simulate_keystrokes("down");
    assert_eq!(f.selected(cx), Some(Folder(f.folder)));

    // A key with a name.
    assert_eq!(hint(&Rename, cx).as_deref(), Some("R"));
    assert_eq!(rebind(&Rename, "f2", "f6", cx), 1);
    assert_eq!(rebind(&Rename, "r", "ctrl-r", cx), 1);
    cx.simulate_keystrokes("f2 r");
    assert_eq!(f.renaming(cx), None, "the old keys rename nothing");
    cx.simulate_keystrokes("f6");
    assert_eq!(f.renaming(cx), Some(Folder(f.folder)));
    cx.simulate_keystrokes("escape");
    assert_eq!(f.renaming(cx), None);
    cx.simulate_keystrokes("ctrl-r");
    assert_eq!(f.renaming(cx), Some(Folder(f.folder)));
    cx.simulate_keystrokes("escape");
    assert_eq!(hint(&Rename, cx).as_deref(), Some("⌃R"));

    // One of the three keys of an action; the others stay.
    assert_eq!(rebind(&SelectNext, "j", "ctrl-j", cx), 1);
    cx.simulate_keystrokes("j");
    assert_eq!(f.selected(cx), Some(Folder(f.folder)), "the old key");
    cx.simulate_keystrokes("ctrl-j");
    assert_eq!(f.selected(cx), Some(Document(f.at_root)));
    cx.simulate_keystrokes("up down");
    assert_eq!(f.selected(cx), Some(Document(f.at_root)));

    // A sequence of two keys.
    assert_eq!(rebind(&SelectFirst, "g g", "g t", cx), 1);
    cx.simulate_keystrokes("g g");
    cx.executor().advance_clock(Duration::from_millis(1100));
    cx.run_until_parked();
    assert_eq!(f.selected(cx), Some(Document(f.at_root)), "the old keys");
    cx.simulate_keystrokes("g t");
    assert_eq!(f.selected(cx), Some(Folder(f.folder)));

    // The key that opens.
    assert_eq!(rebind(&OpenSelected, "enter", "ctrl-o", cx), 1);
    cx.simulate_keystrokes("ctrl-j");
    f.events();
    cx.simulate_keystrokes("enter");
    assert_eq!(f.events(), vec![], "the old key opens nothing");
    cx.simulate_keystrokes("ctrl-o");
    assert_eq!(
        f.events(),
        vec![LibraryPanelEvent::OpenDocument {
            id: f.at_root,
            focus_editor: true
        }]
    );
    assert_eq!(hint(&OpenSelected, cx).as_deref(), Some("⌃O"));

    // The keys beside the controls of the panel are read when they are drawn.
    assert_eq!(hint(&NewDocument, cx).as_deref(), Some("N"));
    assert_eq!(rebind(&NewDocument, "n", "ctrl-n", cx), 1);
    assert_eq!(hint(&NewDocument, cx).as_deref(), Some("⌃N"));
    f.panel.update(cx, |p, _| p.set_new_document_content("{}"));
    let before = f.documents(None, cx).len();
    cx.simulate_keystrokes("n");
    assert_eq!(f.documents(None, cx).len(), before, "the old key");
    cx.simulate_keystrokes("ctrl-n");
    assert_eq!(f.documents(None, cx).len(), before + 1);
}

/// In the trash view `enter` restores, because the trash bindings were added after the
/// generic ones. A rebuilt keymap keeps that order, and a key that was changed stands where
/// its default stood.
#[gpui::test]
fn the_trash_view_wins_its_keys_after_a_rebuild(cx: &mut TestAppContext) {
    let (f, cx) = fixture(cx);
    let trash = |cx: &mut VisualTestContext| f.panel.read_with(cx, |p, _| p.trash_items());
    let showing = |cx: &mut VisualTestContext| f.panel.read_with(cx, |p, _| p.is_showing_trash());
    f.library
        .update(cx, |l, cx| l.trash_document(f.at_root, cx).unwrap());
    f.clock.advance_ms(1000);
    f.library
        .update(cx, |l, cx| l.trash_document(f.in_folder, cx).unwrap());
    cx.run_until_parked();

    // A rebuild that changes a key of the tree and one of the trash view.
    assert_eq!(rebind(&ShowTrash, "t", "ctrl-t", cx), 1);
    assert_eq!(rebind(&RestoreSelected, "r", "ctrl-r", cx), 1);
    cx.simulate_keystrokes("t");
    assert!(!showing(cx), "the old key");
    cx.simulate_keystrokes("ctrl-t");
    assert!(showing(cx));
    assert_eq!(trash(cx).len(), 2);

    // `enter` is bound twice where the trash is shown: to "open" for the whole panel and
    // to "restore" for the trash. The later binding still wins.
    cx.simulate_keystrokes("enter");
    assert_eq!(trash(cx).len(), 1, "enter restored");
    f.events();
    cx.simulate_keystrokes("r");
    assert_eq!(trash(cx).len(), 1, "the old key restores nothing");
    cx.simulate_keystrokes("ctrl-r");
    assert!(trash(cx).is_empty());
    assert_eq!(hint(&RestoreSelected, cx).as_deref(), Some("⌃R"));
}

/// The key that opens the menu of a row is the binding of an action the comment list, the
/// text fields and the chats bind too, each in its own place and with a key that can be
/// changed by itself. The "more" control of a row names the panel's key, not the binding
/// added last.
#[gpui::test]
fn the_more_control_names_the_key_of_the_panel(cx: &mut TestAppContext) {
    use innerword_ui::context_menu::OpenContextMenu;

    let (f, cx) = fixture(cx);
    // Another place binds the action after the library did, as the comments and the chats
    // do in the app.
    cx.update(|_, cx| {
        cx.bind_keys([gpui::KeyBinding::new(
            "ctrl-m",
            OpenContextMenu,
            Some("Elsewhere"),
        )])
    });
    cx.simulate_keystrokes("down");
    let shown = |cx: &mut VisualTestContext| {
        f.panel.update_in(cx, |panel, window, cx| {
            panel.context_menu_key(window, cx).map(|key| key.text())
        })
    };
    assert_eq!(hint(&OpenContextMenu, cx).as_deref(), Some("⌃M"));
    assert_eq!(shown(cx).as_deref(), Some("⌃↩"), "the panel's own key");

    // The panel's key is changed, and only the panel's.
    let changed = cx.update(|_, cx| {
        keymap::replace_key_in(
            &OpenContextMenu,
            Some("LibraryPanel && !TextField"),
            "ctrl-enter",
            Some("ctrl-o"),
            cx,
        )
    });
    assert_eq!(changed, 1);
    cx.run_until_parked();
    assert_eq!(shown(cx).as_deref(), Some("⌃O"));
    let menu_open = |cx: &mut VisualTestContext| {
        cx.update(|window, cx| innerword_ui::ContextMenu::is_open(window, cx))
    };
    cx.simulate_keystrokes("ctrl-enter");
    assert!(!menu_open(cx), "the old key opens no menu");
    cx.simulate_keystrokes("ctrl-o");
    assert!(menu_open(cx));
    cx.simulate_keystrokes("ctrl-o");
    assert!(!menu_open(cx), "the key that opened it closes it");
}
