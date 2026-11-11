//! The vertical slice: typed input and keys reach the document through the
//! real dispatch path (bindings, then the platform input handler).

use gpui::{Entity, TestAppContext, VisualTestContext};
use writer_core::{Document, Position, Selection};
use writer_ui::theme::{self, ThemeMode};

use crate::{Editor, EditorConfig};

pub(super) fn setup(cx: &mut TestAppContext) {
    cx.update(|cx| {
        theme::init(ThemeMode::Light, cx);
        crate::init(cx);
    });
}

/// Opens a focused editor in an active test window.
pub(super) fn open(
    document: Document,
    config: EditorConfig,
    cx: &mut TestAppContext,
) -> (Entity<Editor>, &mut VisualTestContext) {
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let editor = Editor::new(document, config, window, cx);
        editor.focus(window);
        editor
    });
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    (editor, cx)
}

#[gpui::test]
fn typed_input_and_keys_edit_the_document(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(Document::new(), EditorConfig::full(), cx);

    cx.simulate_input("Hello");
    cx.simulate_keystrokes("enter");
    cx.simulate_keystrokes("cmd-b");
    cx.simulate_input("bold");
    cx.simulate_keystrokes("cmd-b");
    cx.simulate_input(" plain");
    cx.simulate_keystrokes("backspace backspace");
    editor.read_with(cx, |editor, _| {
        let document = editor.document();
        assert_eq!(document.plain_text(), "Hello\nbold pla");
        assert!(document.blocks()[1].attrs_at(0).unwrap().bold);
        assert!(!document.blocks()[1].attrs_at(6).unwrap().bold);
        assert_eq!(editor.selection(), Selection::caret(Position::new(1, 8)));
        assert_eq!(editor.stats().words, 3);
    });

    // Arrows: up keeps the column, shift-arrows extend, typing replaces.
    cx.simulate_keystrokes("left left left up");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.selection(), Selection::caret(Position::new(0, 5)));
    });
    cx.simulate_keystrokes("shift-left shift-left");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.state().selected_text(), "lo");
    });
    cx.simulate_input("p");
    cx.simulate_keystrokes("down");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().plain_text(), "Help\nbold pla");
        assert_eq!(editor.selection().head.block, 1);
    });

    // Undo goes through the same seam.
    cx.simulate_keystrokes("cmd-z");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().plain_text(), "Hello\nbold pla");
    });

    // Markdown input rules run for typed characters.
    cx.simulate_keystrokes("cmd-down enter");
    cx.simulate_input("# Title");
    editor.read_with(cx, |editor, _| {
        let document = editor.document();
        assert_eq!(document.blocks()[2].kind().heading_level(), Some(1));
        assert_eq!(document.blocks()[2].text(), "Title");
    });
}
