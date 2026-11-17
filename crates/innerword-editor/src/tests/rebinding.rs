//! A rebuilt keymap: the user's own shortcuts reach GPUI by clearing its
//! keymap and binding it again with other keystrokes
//! (`innerword_ui::keymap::replace_key` does that for one key). The editor
//! obeys and shows the new keys at once, and nothing in it still knows the
//! old ones.

use std::{cell::RefCell, rc::Rc};

use gpui::{Action, TestAppContext, VisualTestContext};
use innerword_core::{Block, BlockKind, Document, Position, Selection};
use innerword_ui::{KeyHint, keymap};

use super::slice::{open, setup};
use crate::{
    CompactOptions, EditorConfig, EditorEvent,
    actions::{
        MoveDown, Newline, OpenContextMenu, ToggleBold, ToggleItalic, ToggleNumberedList, Undo,
    },
};

fn hint(action: &dyn Action, cx: &mut VisualTestContext) -> Option<String> {
    cx.update(|_, cx| KeyHint::for_action(action, cx).map(|hint| hint.text()))
}

fn rebind(action: &dyn Action, from: &str, to: &str, cx: &mut VisualTestContext) -> usize {
    let changed = cx.update(|_, cx| keymap::replace_key(action, from, Some(to), cx));
    cx.run_until_parked();
    changed
}

fn document(text: &str) -> Document {
    Document::from_blocks(vec![Block::paragraph(text)])
}

#[gpui::test]
fn a_formatting_key_that_was_changed(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(document("alpha beta"), EditorConfig::full(), cx);
    let bold = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |editor, _| {
            editor.document().blocks()[0].attrs_at(0).unwrap().bold
        })
    };
    cx.simulate_keystrokes("alt-shift-right");
    assert_eq!(hint(&ToggleBold, cx).as_deref(), Some("⌘B"));

    assert_eq!(rebind(&ToggleBold, "cmd-b", "cmd-alt-b", cx), 1);
    cx.simulate_keystrokes("cmd-b");
    assert!(!bold(cx), "the old key does nothing");
    cx.simulate_keystrokes("cmd-alt-b");
    assert!(bold(cx), "the new key does it");
    assert_eq!(hint(&ToggleBold, cx).as_deref(), Some("⌥⌘B"));

    // The key that became free can be given to another command.
    assert_eq!(rebind(&ToggleItalic, "cmd-i", "cmd-b", cx), 1);
    cx.simulate_keystrokes("cmd-b");
    editor.read_with(cx, |editor, _| {
        let attrs = editor.document().blocks()[0].attrs_at(0).unwrap();
        assert!(attrs.bold && attrs.italic);
    });
    assert_eq!(hint(&ToggleItalic, cx).as_deref(), Some("⌘B"));

    // The keys nobody changed are where they were.
    cx.simulate_keystrokes("cmd-z cmd-z");
    editor.read_with(cx, |editor, _| {
        assert!(
            editor.document().blocks()[0]
                .attrs_at(0)
                .unwrap()
                .is_plain()
        );
    });
    assert_eq!(hint(&Undo, cx).as_deref(), Some("⌘Z"));
}

/// Shift with a digit is bound in two spellings (`cmd-shift-7` for scripts
/// and hints, `cmd-&` for what a keyboard reports). Each is a binding of
/// its own and is replaced as one.
#[gpui::test]
fn a_key_bound_in_two_spellings(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(document("item"), EditorConfig::full(), cx);
    let kind = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |editor, _| editor.document().blocks()[0].kind().clone())
    };
    assert_eq!(hint(&ToggleNumberedList, cx).as_deref(), Some("⇧⌘7"));
    assert_eq!(
        rebind(&ToggleNumberedList, "cmd-shift-7", "cmd-alt-7", cx),
        1
    );
    assert_eq!(rebind(&ToggleNumberedList, "cmd-&", "cmd-alt-7", cx), 1);

    cx.simulate_keystrokes("cmd-shift-7");
    assert_eq!(kind(cx), BlockKind::Paragraph);
    cx.simulate_keystrokes("cmd-&");
    assert_eq!(kind(cx), BlockKind::Paragraph);
    cx.simulate_keystrokes("cmd-alt-7");
    assert_eq!(kind(cx), BlockKind::NumberedItem);
    assert_eq!(hint(&ToggleNumberedList, cx).as_deref(), Some("⌥⌘7"));
}

#[gpui::test]
fn a_caret_key_that_was_changed(cx: &mut TestAppContext) {
    setup(cx);
    let document = Document::from_blocks(vec![
        Block::paragraph("one"),
        Block::paragraph("two"),
        Block::paragraph("three"),
    ]);
    let (editor, cx) = open(document, EditorConfig::full(), cx);
    let block = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |editor, _| editor.selection().head.block)
    };
    cx.simulate_keystrokes("cmd-up");
    assert_eq!(rebind(&MoveDown, "down", "ctrl-j", cx), 1);
    cx.simulate_keystrokes("down");
    assert_eq!(block(cx), 0, "the old key moves nothing");
    cx.simulate_keystrokes("ctrl-j");
    assert_eq!(block(cx), 1);
    // The action's other key (`ctrl-n`) is a binding of its own.
    cx.simulate_keystrokes("ctrl-n");
    assert_eq!(block(cx), 2);
    assert_eq!(hint(&MoveDown, cx).as_deref(), Some("⌃J"));
}

/// A compact editor submits on the key of "new paragraph": the key is the
/// binding's, not `enter` written in the code.
#[gpui::test]
fn a_compact_editor_submits_on_whatever_new_paragraph_is_bound_to(cx: &mut TestAppContext) {
    setup(cx);
    let config = EditorConfig::compact(CompactOptions::default());
    let (editor, cx) = open(Document::new(), config, cx);
    let submitted = Rc::new(RefCell::new(0));
    cx.update(|_, cx| {
        let submitted = submitted.clone();
        cx.subscribe(&editor, move |_, event: &EditorEvent, _| {
            if matches!(event, EditorEvent::Submitted) {
                *submitted.borrow_mut() += 1;
            }
        })
        .detach();
    });
    cx.simulate_input("hi");
    cx.simulate_keystrokes("enter");
    assert_eq!(*submitted.borrow(), 1);

    assert_eq!(rebind(&Newline, "enter", "ctrl-j", cx), 1);
    cx.simulate_keystrokes("enter");
    assert_eq!(*submitted.borrow(), 1, "the old key submits no more");
    cx.simulate_keystrokes("ctrl-j");
    assert_eq!(*submitted.borrow(), 2);
    assert_eq!(hint(&Newline, cx).as_deref(), Some("⌃J"));
}

#[gpui::test]
fn the_key_of_the_context_menu_that_was_changed(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(document("alpha beta"), EditorConfig::full(), cx);
    let requests = Rc::new(RefCell::new(0));
    cx.update(|_, cx| {
        let requests = requests.clone();
        cx.subscribe(&editor, move |_, event: &EditorEvent, _| {
            if matches!(event, EditorEvent::ContextMenuRequested(_)) {
                *requests.borrow_mut() += 1;
            }
        })
        .detach();
    });
    cx.simulate_keystrokes("ctrl-enter shift-f10");
    assert_eq!(*requests.borrow(), 2);
    assert_eq!(hint(&OpenContextMenu, cx).as_deref(), Some("⌃↩"));

    assert_eq!(rebind(&OpenContextMenu, "ctrl-enter", "ctrl-m", cx), 1);
    cx.simulate_keystrokes("ctrl-enter");
    assert_eq!(*requests.borrow(), 2, "the old key asks for no menu");
    cx.simulate_keystrokes("ctrl-m");
    assert_eq!(*requests.borrow(), 3);
    // The second key was not changed.
    cx.simulate_keystrokes("shift-f10");
    assert_eq!(*requests.borrow(), 4);
    assert_eq!(hint(&OpenContextMenu, cx).as_deref(), Some("⌃M"));
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.plain_text(), "alpha beta");
        assert_eq!(editor.selection(), Selection::caret(Position::new(0, 0)));
    });
}
