//! A rebuilt keymap: the user's own shortcuts reach GPUI by clearing its
//! keymap and binding it again with other keystrokes
//! (`writer_ui::keymap::replace_key` does that for one key). The tools obey
//! and show the new keys at once: their bindings, the keys they take from
//! the editor before its bindings (the open insert menu), and every key
//! they name beside a control.

use gpui::{Action, TestAppContext, VisualTestContext};
use writer_core::{Document, TextRange};
use writer_editor::actions as editor_actions;
use writer_editor_tools::{
    ToolKind,
    actions::{CloseTool, Find, FindNext, OpenFormatMenu, OpenLink, ReplaceCurrent},
    set_font_families,
};
use writer_ui::{KeyHint, keymap, picker, text_field};

use crate::support::*;

fn hint(action: &dyn Action, cx: &mut VisualTestContext) -> Option<String> {
    cx.update(|_, cx| KeyHint::for_action(action, cx).map(|hint| hint.text()))
}

fn rebind(action: &dyn Action, from: &str, to: &str, cx: &mut VisualTestContext) -> usize {
    let changed = cx.update(|_, cx| keymap::replace_key(action, from, Some(to), cx));
    cx.run_until_parked();
    changed
}

fn tooltip(
    tools: &gpui::Entity<writer_editor_tools::EditorTools>,
    button: &str,
    cx: &mut VisualTestContext,
) -> Option<String> {
    tools.read_with(cx, |tools, cx| tools.find_button_tooltip(button, cx))
}

#[gpui::test]
fn the_keys_of_the_tools_that_were_changed(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(paragraphs(&["one two one"]), cx);

    // Opening a tool.
    assert_eq!(rebind(&Find, "cmd-f", "cmd-alt-g", cx), 1);
    cx.simulate_keystrokes("cmd-f");
    assert!(!tools.read_with(cx, |tools, _| tools.is_open(ToolKind::Find)));
    cx.simulate_keystrokes("cmd-alt-g");
    assert!(tools.read_with(cx, |tools, _| tools.is_open(ToolKind::Find)));
    assert_eq!(hint(&Find, cx).as_deref(), Some("⌥⌘G"));

    // A key of the bar, and the tooltip of its button.
    assert_eq!(
        tooltip(&tools, "find-next", cx).as_deref(),
        Some("Next match ⌘G")
    );
    assert_eq!(rebind(&FindNext, "cmd-g", "ctrl-s", cx), 1);
    assert_eq!(
        tooltip(&tools, "find-next", cx).as_deref(),
        Some("Next match ⌃S")
    );
    cx.simulate_input("one");
    let current =
        |cx: &mut VisualTestContext| tools.read_with(cx, |tools, _| tools.find_current_match());
    assert_eq!(current(cx), Some(1));
    cx.simulate_keystrokes("cmd-g");
    assert_eq!(current(cx), Some(1), "the old key finds nothing more");
    cx.simulate_keystrokes("ctrl-s");
    assert_eq!(current(cx), Some(2));

    // Closing: the key of "close the tool" works in the field of the bar
    // and is the key the close button names.
    assert_eq!(
        tooltip(&tools, "find-close", cx).as_deref(),
        Some("Close Esc")
    );
    assert_eq!(rebind(&CloseTool, "escape", "ctrl-g", cx), 1);
    assert_eq!(
        tooltip(&tools, "find-close", cx).as_deref(),
        Some("Close ⌃G")
    );
    cx.simulate_keystrokes("ctrl-g");
    assert!(!tools.read_with(cx, |tools, _| tools.is_open(ToolKind::Find)));
    assert!(editor_focused(&editor, cx));
}

#[gpui::test]
fn the_find_bar_names_the_keys_its_fields_take(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, _editor, cx) = open(paragraphs(&["one two one"]), cx);
    cx.simulate_keystrokes("cmd-alt-f");
    assert_eq!(
        tooltip(&tools, "find-replace-one", cx).as_deref(),
        Some("Replace this match ↩")
    );

    // `enter` in the replace field is the field's own submit.
    assert_eq!(rebind(&text_field::Submit, "enter", "ctrl-j", cx), 1);
    assert_eq!(
        tooltip(&tools, "find-replace-one", cx).as_deref(),
        Some("Replace this match ⌃J")
    );
    // With no key for "close the tool", the bar names the key its fields
    // cancel on.
    cx.update(|_, cx| keymap::replace_key(&CloseTool, "escape", None, cx));
    assert_eq!(rebind(&text_field::Cancel, "escape", "ctrl-g", cx), 1);
    assert_eq!(
        tooltip(&tools, "find-close", cx).as_deref(),
        Some("Close ⌃G")
    );
    // A key of its own for "replace" is the one to name.
    cx.update(|_, cx| {
        cx.bind_keys([gpui::KeyBinding::new(
            "cmd-alt-r",
            ReplaceCurrent,
            Some("EditorTools"),
        )])
    });
    cx.run_until_parked();
    assert_eq!(
        tooltip(&tools, "find-replace-one", cx).as_deref(),
        Some("Replace this match ⌥⌘R")
    );

    // And the keys work: the new submit replaces, the old one does not.
    cx.simulate_input("one");
    cx.simulate_keystrokes("tab");
    cx.simulate_input("1");
    let editor = tools.read_with(cx, |tools, _| tools.editor().clone());
    cx.simulate_keystrokes("enter");
    assert_eq!(text(&editor, cx), "one two one");
    // (Without a binding the test platform types `enter` into the field,
    // which takes a line break as a space.)
    tools.read_with(cx, |tools, cx| assert_eq!(tools.find_replacement(cx), "1 "));
    cx.simulate_keystrokes("backspace ctrl-j");
    assert_eq!(text(&editor, cx), "1 two one");
    cx.simulate_keystrokes("escape");
    assert!(tools.read_with(cx, |tools, _| tools.is_open(ToolKind::Find)));
    cx.simulate_keystrokes("ctrl-g");
    assert!(!tools.read_with(cx, |tools, _| tools.is_open(ToolKind::Find)));
}

#[gpui::test]
fn the_link_popover_names_the_keys_as_they_are(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(paragraphs(&["Read the manual first"]), cx);
    select(&editor, TextRange::in_block(0, 9, 15), cx);
    cx.simulate_keystrokes("cmd-k");
    assert_eq!(active(&tools, cx), Some(ToolKind::Link));
    let key = |button: &str, cx: &mut VisualTestContext| {
        tools.read_with(cx, |tools, cx| tools.link_button_key(button, cx))
    };
    assert_eq!(key("link-apply", cx).as_deref(), Some("↩"));
    assert_eq!(key("link-open", cx).as_deref(), Some("⌘↩"));
    assert_eq!(key("link-cancel", cx).as_deref(), Some("Esc"));

    assert_eq!(rebind(&text_field::Submit, "enter", "ctrl-j", cx), 1);
    assert_eq!(rebind(&CloseTool, "escape", "ctrl-g", cx), 1);
    assert_eq!(active(&tools, cx), Some(ToolKind::Link), "still open");
    assert_eq!(key("link-apply", cx).as_deref(), Some("⌃J"));
    assert_eq!(key("link-cancel", cx).as_deref(), Some("⌃G"));

    cx.simulate_input("example.com");
    cx.simulate_keystrokes("enter");
    assert_eq!(active(&tools, cx), Some(ToolKind::Link), "the old key");
    cx.simulate_keystrokes("ctrl-j");
    assert_eq!(active(&tools, cx), None);
    assert!(
        editor
            .read_with(cx, |editor, _| editor.document().blocks()[0]
                .attrs_at(9)
                .and_then(|attrs| attrs.link.clone()))
            .is_some()
    );

    // The key that opens it.
    assert_eq!(rebind(&OpenLink, "cmd-k", "cmd-alt-k", cx), 1);
    cx.simulate_keystrokes("cmd-k");
    assert_eq!(active(&tools, cx), None);
    cx.simulate_keystrokes("cmd-alt-k");
    assert_eq!(active(&tools, cx), Some(ToolKind::Link));
    cx.simulate_keystrokes("ctrl-g");
    assert_eq!(active(&tools, cx), None);
    assert_eq!(hint(&OpenLink, cx).as_deref(), Some("⌥⌘K"));
}

#[gpui::test]
fn the_font_picker_names_the_keys_as_they_are(cx: &mut TestAppContext) {
    setup(cx);
    cx.update(|cx| set_font_families(["Charter", "Georgia", "Menlo"], cx));
    let (tools, _editor, cx) = open(paragraphs(&["Set this in another face"]), cx);
    cx.simulate_keystrokes("cmd-t");
    assert_eq!(active(&tools, cx), Some(ToolKind::Font));
    let footer =
        |cx: &mut VisualTestContext| tools.read_with(cx, |tools, cx| tools.menu_footer(cx));
    let pair = |label: &str, key: &str| (label.to_string(), key.to_string());
    assert_eq!(
        footer(cx),
        [pair("Apply", "↩"), pair("Set as document font", "⌘↩")]
    );

    // The keymap changes under the open picker.
    assert_eq!(
        rebind(&picker::SecondaryConfirm, "cmd-enter", "cmd-alt-d", cx),
        1
    );
    assert_eq!(active(&tools, cx), Some(ToolKind::Font), "still open");
    assert_eq!(
        footer(cx),
        [pair("Apply", "↩"), pair("Set as document font", "⌥⌘D")]
    );
    // A key that is bound to nothing is not named.
    cx.update(|_, cx| keymap::replace_key(&picker::SecondaryConfirm, "cmd-alt-d", None, cx));
    cx.run_until_parked();
    assert_eq!(footer(cx), [pair("Apply", "↩")]);

    // "Apply" is the list's confirm, also with the keyboard in the filter
    // field: the field's submit (still `enter`) does not take a row.
    assert_eq!(rebind(&picker::Confirm, "enter", "ctrl-y", cx), 1);
    assert_eq!(footer(cx), [pair("Apply", "⌃Y")]);
    cx.simulate_input("Georg");
    cx.simulate_keystrokes("enter");
    assert_eq!(active(&tools, cx), Some(ToolKind::Font), "the old key");
    cx.simulate_keystrokes("backspace ctrl-y");
    assert_eq!(active(&tools, cx), None);
}

#[gpui::test]
fn an_open_menu_shows_the_keys_of_a_keymap_rebuilt_under_it(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(paragraphs(&["alpha beta"]), cx);
    select(&editor, TextRange::in_block(0, 0, 5), cx);

    // The format menu.
    cx.simulate_keystrokes("cmd-.");
    assert_eq!(active(&tools, cx), Some(ToolKind::Format));
    assert_eq!(row(&tools, "Bold", cx).key_hint.as_deref(), Some("⌘B"));
    assert_eq!(
        rebind(&editor_actions::ToggleBold, "cmd-b", "cmd-alt-b", cx),
        1
    );
    assert_eq!(active(&tools, cx), Some(ToolKind::Format), "still open");
    assert_eq!(row(&tools, "Bold", cx).key_hint.as_deref(), Some("⌥⌘B"));
    // The key that opened it closes it; the old one is gone.
    assert_eq!(rebind(&OpenFormatMenu, "cmd-.", "cmd-alt-.", cx), 1);
    cx.simulate_keystrokes("cmd-.");
    assert_eq!(active(&tools, cx), Some(ToolKind::Format));
    cx.simulate_keystrokes("cmd-alt-.");
    assert_eq!(active(&tools, cx), None);

    // The insert menu.
    caret(&editor, 0, 10, cx);
    cx.simulate_keystrokes("enter");
    cx.simulate_input("/");
    assert_eq!(active(&tools, cx), Some(ToolKind::Insert));
    assert_eq!(
        row(&tools, "Page break", cx).key_hint.as_deref(),
        Some("⌘↩")
    );
    assert_eq!(
        rebind(
            &editor_actions::InsertPageBreak,
            "cmd-enter",
            "cmd-alt-enter",
            cx
        ),
        1
    );
    assert_eq!(active(&tools, cx), Some(ToolKind::Insert), "still open");
    assert_eq!(
        row(&tools, "Page break", cx).key_hint.as_deref(),
        Some("⌥⌘↩")
    );
}

/// The insert menu takes the keys of a list from the editor while the
/// keyboard stays in the document. Which keys those are is the keymap's
/// answer, not `up` and `down` written in the code.
#[gpui::test]
fn the_insert_menu_obeys_the_keys_of_a_list_as_they_are(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(Document::new(), cx);
    cx.simulate_input("/hea");
    assert_eq!(active(&tools, cx), Some(ToolKind::Insert));
    assert_eq!(selected(&tools, cx).as_deref(), Some("Heading 1"));

    // The defaults, also the second key of each.
    cx.simulate_keystrokes("down ctrl-n");
    assert_eq!(selected(&tools, cx).as_deref(), Some("Heading 3"));
    cx.simulate_keystrokes("up ctrl-p");
    assert_eq!(selected(&tools, cx).as_deref(), Some("Heading 1"));

    assert_eq!(rebind(&picker::SelectNext, "down", "ctrl-j", cx), 1);
    assert_eq!(rebind(&picker::SelectPrev, "up", "ctrl-k", cx), 1);
    assert_eq!(rebind(&picker::Dismiss, "escape", "ctrl-g", cx), 1);
    assert_eq!(active(&tools, cx), Some(ToolKind::Insert), "still open");

    cx.simulate_keystrokes("ctrl-j ctrl-j");
    assert_eq!(selected(&tools, cx).as_deref(), Some("Heading 3"));
    cx.simulate_keystrokes("ctrl-k");
    assert_eq!(selected(&tools, cx).as_deref(), Some("Heading 2"));

    // `escape` is no key of the menu any more: it goes on to the document,
    // where it closes the open tool as it does without a menu of this kind
    // (the binding of the tools), so the menu is gone either way; what
    // matters is the new key.
    cx.simulate_keystrokes("ctrl-g");
    assert_eq!(active(&tools, cx), None);
    assert_eq!(text(&editor, cx), "/hea", "closing keeps what was typed");

    // The old arrow is the caret's again: it leaves the line, and the menu
    // closes because the caret left.
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.insert_text("\nsecond"), cx)
    });
    caret(&editor, 0, 4, cx);
    cx.simulate_keystrokes("backspace backspace backspace backspace");
    cx.simulate_input("/hea");
    assert_eq!(active(&tools, cx), Some(ToolKind::Insert));
    cx.simulate_keystrokes("down");
    assert_eq!(active(&tools, cx), None, "the old key moved the caret");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.selection().head.block, 1);
    });

    // Taking a row: the key of "confirm" in a list, and `tab`.
    cx.simulate_keystrokes("up");
    caret(&editor, 0, 4, cx);
    cx.simulate_keystrokes("backspace backspace backspace backspace");
    assert_eq!(rebind(&picker::Confirm, "enter", "ctrl-y", cx), 1);
    cx.simulate_input("/hea");
    assert_eq!(active(&tools, cx), Some(ToolKind::Insert));
    cx.simulate_keystrokes("ctrl-j ctrl-y");
    assert_eq!(active(&tools, cx), None);
    editor.read_with(cx, |editor, _| {
        assert_eq!(
            editor.document().blocks()[0].kind().heading_level(),
            Some(2)
        );
        assert_eq!(editor.document().blocks()[0].text(), "");
    });
    cx.simulate_input("/quo");
    cx.simulate_keystrokes("tab");
    assert_eq!(active(&tools, cx), None);
    editor.read_with(cx, |editor, _| {
        assert_eq!(
            editor.document().blocks()[0].kind(),
            &writer_core::BlockKind::Quote
        );
    });
}
