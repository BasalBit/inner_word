use std::{cell::RefCell, collections::HashSet, rc::Rc};

use gpui::{Focusable, TestAppContext};
use writer_core::TextRange;
use writer_editor_tools::{EditorTools, ToolKind, ToolsEvent, actions, commands};
use writer_ui::KeyHint;

use crate::support::*;

#[gpui::test]
fn opening_one_tool_closes_any_other(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(paragraphs(&["Some words to work on"]), cx);
    select(&editor, TextRange::in_block(0, 5, 10), cx);
    let events = Rc::new(RefCell::new(Vec::new()));
    cx.update(|_, cx| {
        let events = events.clone();
        cx.subscribe(&tools, move |_, event: &ToolsEvent, _| {
            events.borrow_mut().push(*event);
        })
        .detach();
    });

    cx.simulate_keystrokes("cmd-.");
    assert_eq!(active(&tools, cx), Some(ToolKind::Format));
    // From inside the format menu's filter field.
    cx.simulate_keystrokes("cmd-k");
    assert_eq!(active(&tools, cx), Some(ToolKind::Link));
    tools.read_with(cx, |tools, _| {
        assert!(!tools.is_open(ToolKind::Format));
        assert!(tools.is_open(ToolKind::Link));
    });
    cx.simulate_keystrokes("cmd-t");
    assert_eq!(active(&tools, cx), Some(ToolKind::Font));
    cx.simulate_keystrokes("cmd-f");
    assert_eq!(active(&tools, cx), Some(ToolKind::Find));
    editor.read_with(cx, |editor, _| assert!(!editor.has_popup()));
    // A popup tool closes the find bar.
    cx.simulate_keystrokes("cmd-.");
    assert_eq!(active(&tools, cx), Some(ToolKind::Format));
    tools.read_with(cx, |tools, _| assert!(!tools.is_open(ToolKind::Find)));
    cx.simulate_keystrokes("escape");
    assert_eq!(active(&tools, cx), None);
    assert!(editor_focused(&editor, cx));
    // The selection survived all of it.
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.selection().range(), TextRange::in_block(0, 5, 10));
        assert!(!editor.has_popup());
    });

    use ToolKind::*;
    use ToolsEvent::*;
    assert_eq!(
        *events.borrow(),
        [
            Opened(Format),
            Closed(Format),
            Opened(Link),
            Closed(Link),
            Opened(Font),
            Closed(Font),
            Opened(Find),
            Closed(Find),
            Opened(Format),
            Closed(Format),
        ]
    );
}

#[gpui::test]
fn typing_a_slash_keeps_the_find_bar(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(paragraphs(&["word", ""]), cx);

    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("word");
    cx.update(|window, cx| editor.read(cx).focus(window));
    caret(&editor, 1, 0, cx);
    cx.simulate_input("/");
    tools.read_with(cx, |tools, _| {
        assert!(tools.is_open(ToolKind::Insert));
        assert!(tools.is_open(ToolKind::Find));
        assert_eq!(tools.active_tool(), Some(ToolKind::Insert));
    });
    // Escape closes the menu first; the bar stays until it is closed itself.
    cx.simulate_keystrokes("escape");
    assert_eq!(active(&tools, cx), Some(ToolKind::Find));
    tools.update_in(cx, |tools, window, cx| tools.close(window, cx));
    assert_eq!(active(&tools, cx), None);
    assert!(editor_focused(&editor, cx));
}

#[gpui::test]
fn a_menu_closes_when_focus_goes_elsewhere(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(paragraphs(&["Text"]), cx);

    cx.simulate_keystrokes("cmd-.");
    assert_eq!(active(&tools, cx), Some(ToolKind::Format));
    // For example a click in the document.
    cx.update(|window, cx| editor.read(cx).focus(window));
    cx.run_until_parked();
    assert_eq!(active(&tools, cx), None);
    editor.read_with(cx, |editor, _| assert!(!editor.has_popup()));
    assert!(editor_focused(&editor, cx));

    cx.simulate_keystrokes("cmd-k");
    assert_eq!(active(&tools, cx), Some(ToolKind::Link));
    cx.update(|window, cx| editor.read(cx).focus(window));
    cx.run_until_parked();
    assert_eq!(active(&tools, cx), None);

    // Replacing the document drops whatever is open.
    cx.simulate_keystrokes("cmd-t");
    assert_eq!(active(&tools, cx), Some(ToolKind::Font));
    editor.update(cx, |editor, cx| editor.set_document(paragraphs(&["New"]), cx));
    cx.run_until_parked();
    assert_eq!(active(&tools, cx), None);
}

#[gpui::test]
fn the_tools_view_focuses_the_editor(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(paragraphs(&["Text"]), cx);
    cx.update(|window, cx| {
        assert_eq!(
            tools.read(cx).focus_handle(cx),
            editor.read(cx).focus_handle(cx)
        );
        window.blur();
        window.focus(&tools.focus_handle(cx));
    });
    cx.run_until_parked();
    assert!(editor_focused(&editor, cx));
    // Editor keys still work through the wrapper.
    cx.simulate_input("More ");
    cx.simulate_keystrokes("cmd-b");
    cx.simulate_input("bold");
    assert_eq!(text(&editor, cx), "More boldText");
    let _: &gpui::Entity<EditorTools> = &tools;
}

#[gpui::test]
fn commands_cover_the_tools_and_show_real_bindings(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(paragraphs(&["cat and cat"]), cx);

    let all = commands();
    let names: HashSet<&str> = all.iter().map(|command| command.action_name()).collect();
    assert_eq!(names.len(), all.len(), "one command per action");
    assert!(all.iter().all(|command| command.action_name().starts_with("editor_tools::")));

    let hint = |title: &str, cx: &mut gpui::VisualTestContext| {
        let command = all
            .iter()
            .find(|command| command.title.as_ref() == title)
            .unwrap_or_else(|| panic!("no command {title:?}"));
        cx.update(|_, cx| command.key_hint(cx).map(|hint| hint.text()))
    };
    assert_eq!(hint("Format Menu", cx).as_deref(), Some("⌘."));
    assert_eq!(hint("Add or Edit Link", cx).as_deref(), Some("⌘K"));
    assert_eq!(hint("Remove Link", cx).as_deref(), Some("⇧⌘K"));
    assert_eq!(hint("Choose Font", cx).as_deref(), Some("⌘T"));
    assert_eq!(hint("Find", cx).as_deref(), Some("⌘F"));
    assert_eq!(hint("Find and Replace", cx).as_deref(), Some("⌥⌘F"));
    assert_eq!(hint("Find Next", cx).as_deref(), Some("⌘G"));
    assert_eq!(hint("Find Previous", cx).as_deref(), Some("⇧⌘G"));
    assert_eq!(hint("Toggle Case Sensitive", cx).as_deref(), Some("⌥⌘C"));
    assert_eq!(hint("Toggle Whole Word", cx).as_deref(), Some("⌥⌘W"));
    assert_eq!(hint("Replace All", cx).as_deref(), Some("⌘↩"));
    assert_eq!(hint("Insert Menu", cx), None);
    cx.update(|_, cx| {
        assert_eq!(
            KeyHint::for_action(&actions::OpenLinkInBrowser, cx).map(|hint| hint.text()).as_deref(),
            Some("⌘↩")
        );
    });

    // Every command runs from the editor, the way the palette dispatches
    // it (to the focused element), and leaves a consistent state.
    for command in &all {
        cx.update(|window, cx| editor.read(cx).focus(window));
        cx.run_until_parked();
        cx.update(|window, cx| window.dispatch_action(command.action(), cx));
        cx.run_until_parked();
        let expected = match command.title.as_ref() {
            "Format Menu" => Some(ToolKind::Format),
            "Add or Edit Link" => Some(ToolKind::Link),
            "Choose Font" => Some(ToolKind::Font),
            "Insert Menu" => Some(ToolKind::Insert),
            "Find" | "Find and Replace" => Some(ToolKind::Find),
            _ => None,
        };
        if let Some(expected) = expected {
            assert_eq!(active(&tools, cx), Some(expected), "{}", command.title);
        }
        tools.update_in(cx, |tools, window, cx| tools.close(window, cx));
        cx.run_until_parked();
        assert_eq!(active(&tools, cx), None, "{}", command.title);
        assert!(editor_focused(&editor, cx), "{}", command.title);
    }

    // Options toggled from the palette (focus in the editor) apply to the
    // next search.
    cx.dispatch_action(actions::ToggleCaseSensitive);
    assert!(tools.read_with(cx, |tools, _| !tools.find_options().case_sensitive));
    cx.dispatch_action(actions::ToggleWholeWord);
    assert!(tools.read_with(cx, |tools, _| !tools.find_options().whole_word));
}
