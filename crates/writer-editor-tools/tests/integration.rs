use std::{cell::RefCell, collections::HashSet, rc::Rc};

use gpui::{Focusable, TestAppContext};
use writer_core::TextRange;
use writer_editor_tools::{EditorTools, FIND_LAYER, ToolKind, ToolsEvent, actions, commands};
use writer_ui::KeyHint;

use crate::support::*;

#[gpui::test]
fn one_popup_tool_at_a_time_and_the_find_bar_stays(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(paragraphs(&["Some words to work on, words again"]), cx);
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
    // The find bar takes the keyboard, so the popup goes.
    cx.simulate_keystrokes("cmd-f");
    assert_eq!(active(&tools, cx), Some(ToolKind::Find));
    editor.read_with(cx, |editor, _| assert!(!editor.has_popup()));
    tools.read_with(cx, |tools, _| {
        assert_eq!(tools.find_query(), "words", "seeded from the selection");
        assert_eq!(tools.find_status(), "1 of 2");
    });

    // A popup tool opened from the find bar leaves the bar and its search
    // alone: same query, same matches, same highlights.
    for (keys, kind) in [
        ("cmd-.", ToolKind::Format),
        ("cmd-k", ToolKind::Link),
        ("cmd-t", ToolKind::Font),
    ] {
        cx.simulate_keystrokes(keys);
        assert_eq!(active(&tools, cx), Some(kind));
        tools.read_with(cx, |tools, _| {
            assert!(
                tools.is_open(ToolKind::Find),
                "{kind:?} closed the find bar"
            );
            assert_eq!(tools.find_query(), "words");
            assert_eq!(tools.find_status(), "1 of 2");
        });
        editor.read_with(cx, |editor, _| {
            assert!(editor.has_popup());
            assert_eq!(editor.highlights(FIND_LAYER).len(), 2);
        });
    }
    // escape closes the popup and returns to the text; the bar is still
    // there, and the next escape, in the text, closes it.
    cx.simulate_keystrokes("escape");
    assert_eq!(active(&tools, cx), Some(ToolKind::Find));
    assert!(editor_focused(&editor, cx));
    cx.simulate_keystrokes("escape");
    assert_eq!(active(&tools, cx), None);
    assert!(editor_focused(&editor, cx));
    // The selection survived all of it.
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.selection().range(), TextRange::in_block(0, 5, 10));
        assert!(!editor.has_popup());
        assert!(editor.highlights(FIND_LAYER).is_empty());
    });
    // With nothing open escape is not ours: it changes nothing.
    cx.simulate_keystrokes("escape");
    assert_eq!(text(&editor, cx), "Some words to work on, words again");

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
            Opened(Format),
            Closed(Format),
            Opened(Link),
            Closed(Link),
            Opened(Font),
            Closed(Font),
            Closed(Find),
        ]
    );
}

#[gpui::test]
fn applying_a_format_from_the_find_bar_keeps_the_search(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(paragraphs(&["one cat", "two cat"]), cx);

    // Find, step to the second match, make it bold through the format
    // menu, then go on searching: the bar never went away.
    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("cat");
    cx.simulate_keystrokes("enter");
    assert_eq!(
        tools.read_with(cx, |tools, _| tools.find_status()),
        "2 of 2"
    );
    cx.simulate_keystrokes("cmd-.");
    cx.simulate_input("bold");
    cx.simulate_keystrokes("enter");
    assert!(editor_focused(&editor, cx));
    editor.read_with(cx, |editor, _| {
        assert!(editor.document().blocks()[1].attrs_at(4).unwrap().bold);
        assert!(!editor.document().blocks()[0].attrs_at(4).unwrap().bold);
        assert_eq!(editor.highlights(FIND_LAYER).len(), 2);
    });
    tools.read_with(cx, |tools, _| {
        assert_eq!(tools.active_tool(), Some(ToolKind::Find));
        assert_eq!(tools.find_status(), "2 of 2");
    });
    cx.simulate_keystrokes("cmd-g");
    assert_eq!(
        tools.read_with(cx, |tools, _| tools.find_status()),
        "1 of 2"
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
    editor.update(cx, |editor, cx| {
        editor.set_document(paragraphs(&["New"]), cx)
    });
    cx.run_until_parked();
    assert_eq!(active(&tools, cx), None);
    // The picker had the keyboard; it goes back to the editor.
    assert!(editor_focused(&editor, cx));
    cx.simulate_input("A ");
    assert_eq!(text(&editor, cx), "A New");
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
    assert!(
        all.iter()
            .all(|command| command.action_name().starts_with("editor_tools::"))
    );

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
    // (The kit decides how escape is written.)
    assert_eq!(
        hint("Close Find Bar", cx),
        KeyHint::parse("escape").map(|hint| hint.text())
    );
    assert_eq!(hint("Replace", cx), None);
    assert_eq!(hint("Insert Menu", cx), None);
    cx.update(|_, cx| {
        assert_eq!(
            KeyHint::for_action(&actions::OpenLinkInBrowser, cx)
                .map(|hint| hint.text())
                .as_deref(),
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
            // The find commands that need the bar open it rather than
            // doing nothing.
            "Find"
            | "Find and Replace"
            | "Replace"
            | "Replace All"
            | "Toggle Case Sensitive"
            | "Toggle Whole Word" => Some(ToolKind::Find),
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

    // Options toggled from the palette (focus in the editor) open the bar,
    // which shows them.
    cx.dispatch_action(actions::ToggleCaseSensitive);
    assert!(tools.read_with(cx, |tools, _| !tools.find_options().case_sensitive));
    cx.dispatch_action(actions::ToggleWholeWord);
    assert!(tools.read_with(cx, |tools, _| !tools.find_options().whole_word));
    assert_eq!(active(&tools, cx), Some(ToolKind::Find));

    // "Replace All" from the palette with the bar closed opens the bar
    // with the replace row and replaces nothing yet; so does "Replace".
    editor.update(cx, |editor, cx| {
        editor.set_document(paragraphs(&["cat and cat"]), cx)
    });
    for action in [
        Box::new(actions::ReplaceAll) as Box<dyn gpui::Action>,
        Box::new(actions::ReplaceCurrent),
    ] {
        tools.update_in(cx, |tools, window, cx| tools.close(window, cx));
        cx.simulate_keystrokes("cmd-f");
        cx.simulate_input("cat");
        cx.simulate_keystrokes("escape");
        cx.update(|window, cx| window.dispatch_action(action, cx));
        cx.run_until_parked();
        tools.read_with(cx, |tools, _| {
            assert!(tools.is_open(ToolKind::Find));
            assert!(tools.find_replace_visible());
            assert_eq!(tools.find_match_count(), 2);
        });
        assert_eq!(text(&editor, cx), "cat and cat");
    }
    // "Close Find Bar" closes it from the document.
    cx.update(|window, cx| editor.read(cx).focus(window));
    cx.dispatch_action(actions::CloseTool);
    assert_eq!(active(&tools, cx), None);
    assert!(editor_focused(&editor, cx));
}

#[gpui::test]
fn every_action_has_a_key_or_a_palette_entry(cx: &mut TestAppContext) {
    setup(cx);
    let bound: HashSet<&str> = actions::key_bindings()
        .iter()
        .map(|binding| binding.action().name())
        .collect();
    let listed: Vec<_> = commands();
    let listed: HashSet<&str> = listed.iter().map(|command| command.action_name()).collect();
    let all: Vec<&'static str> = cx.update(|cx| {
        cx.all_action_names()
            .iter()
            .copied()
            .filter(|name| name.starts_with("editor_tools::"))
            .collect()
    });
    assert_eq!(all.len(), 18, "{all:?}");
    for name in all {
        assert!(
            bound.contains(name) || listed.contains(name),
            "{name} has neither a key binding nor a palette entry"
        );
    }
}

#[gpui::test]
fn switching_to_another_app_keeps_the_open_tool(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(paragraphs(&["Some text", ""]), cx);
    select(&editor, TextRange::in_block(0, 0, 4), cx);

    // The link popover survives the trip to the browser.
    cx.simulate_keystrokes("cmd-k");
    cx.simulate_input("example.com");
    cx.simulate_keystrokes("cmd-enter");
    cx.deactivate_window();
    cx.run_until_parked();
    assert_eq!(active(&tools, cx), Some(ToolKind::Link));
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    assert_eq!(active(&tools, cx), Some(ToolKind::Link));
    cx.simulate_input("/a");
    assert_eq!(
        tools
            .read_with(cx, |tools, cx| tools.link_field_text(cx))
            .as_deref(),
        Some("example.com/a"),
        "the field still has the keyboard"
    );
    cx.simulate_keystrokes("enter");
    editor.read_with(cx, |editor, _| {
        assert_eq!(
            editor.document().blocks()[0]
                .attrs_at(0)
                .unwrap()
                .link
                .as_deref(),
            Some("https://example.com/a")
        );
    });

    // So does a menu with a filter field.
    cx.simulate_keystrokes("cmd-.");
    cx.deactivate_window();
    cx.run_until_parked();
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    assert_eq!(active(&tools, cx), Some(ToolKind::Format));
    cx.simulate_input("bold");
    cx.simulate_keystrokes("enter");
    editor.read_with(cx, |editor, _| {
        assert!(editor.document().blocks()[0].attrs_at(0).unwrap().bold);
    });
    assert!(editor_focused(&editor, cx));

    // And the insert menu, which lives on the editor's focus.
    caret(&editor, 1, 0, cx);
    cx.simulate_input("/quo");
    cx.deactivate_window();
    cx.run_until_parked();
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    assert_eq!(active(&tools, cx), Some(ToolKind::Insert));
    cx.simulate_keystrokes("enter");
    editor.read_with(cx, |editor, _| {
        assert_eq!(
            editor.document().blocks()[1].kind(),
            &writer_core::BlockKind::Quote
        );
    });
}
