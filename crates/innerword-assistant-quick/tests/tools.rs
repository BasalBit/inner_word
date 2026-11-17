//! The quick chat beside the editor's own tools: opening it closes the
//! format bar and the popup tools.

use gpui::TestAppContext;
use innerword_core::TextRange;
use innerword_editor_tools::ToolKind;

use crate::support::*;

#[gpui::test]
fn opening_closes_the_format_bar(cx: &mut TestAppContext) {
    let (fx, cx) = open_quick(paragraphs(&["The quick brown fox", "Next."]), cx);
    select(&fx.editor, TextRange::in_block(0, 4, 9), cx);
    let shown = fx
        .tools
        .update_in(cx, |tools, window, cx| tools.show_format_bar(window, cx));
    cx.run_until_parked();
    assert!(shown);
    assert!(
        fx.tools
            .read_with(cx, |tools, _| tools.format_bar_visible())
    );

    fx.toggle(cx);
    assert!(fx.is_open(cx));
    assert!(
        !fx.tools
            .read_with(cx, |tools, _| tools.format_bar_visible()),
        "the format bar gives way"
    );
    assert!(fx.composer_focused(cx));
    // The selection the bar was for is what the chat is about.
    assert_eq!(
        fx.quick.read_with(cx, |quick, _| quick.quote()).as_deref(),
        Some("quick")
    );
}

#[gpui::test]
fn opening_closes_the_insert_menu(cx: &mut TestAppContext) {
    let (fx, cx) = open_quick(paragraphs(&["The quick brown fox", ""]), cx);
    caret(&fx.editor, 1, 0, cx);
    cx.simulate_input("/");
    cx.run_until_parked();
    assert!(
        fx.tools
            .read_with(cx, |tools, _| tools.is_open(ToolKind::Insert))
    );

    fx.toggle(cx);
    assert!(fx.is_open(cx));
    assert!(
        !fx.tools
            .read_with(cx, |tools, _| tools.is_open(ToolKind::Insert))
    );
    assert!(fx.composer_focused(cx));
    // Typing is the composer's, not the menu's filter.
    cx.simulate_input("heading");
    cx.run_until_parked();
    assert_eq!(fx.draft(cx), "heading");
    assert_eq!(text(&fx.editor, cx), "The quick brown fox\n/");
}

#[gpui::test]
fn opening_closes_a_popup_tool_with_a_field(cx: &mut TestAppContext) {
    let (fx, cx) = open_quick(paragraphs(&["The quick brown fox", "Next."]), cx);
    select(&fx.editor, TextRange::in_block(0, 4, 9), cx);
    cx.simulate_keystrokes("cmd-k");
    cx.run_until_parked();
    assert!(
        fx.tools
            .read_with(cx, |tools, _| tools.is_open(ToolKind::Link))
    );

    fx.toggle(cx);
    assert!(fx.is_open(cx));
    assert!(
        !fx.tools
            .read_with(cx, |tools, _| tools.is_open(ToolKind::Link))
    );
    assert!(fx.composer_focused(cx));
    assert_eq!(fx.tools.read_with(cx, |tools, _| tools.active_tool()), None);
}

#[gpui::test]
fn editor_keys_in_the_composer_do_not_reach_the_text(cx: &mut TestAppContext) {
    let (fx, cx) = open_quick(paragraphs(&["The quick brown fox", "Next."]), cx);
    select(&fx.editor, TextRange::in_block(0, 4, 9), cx);
    fx.toggle(cx);

    // An editor key pressed in the composer is not a command on the
    // document under the popover.
    cx.simulate_input("bold?");
    cx.simulate_keystrokes("cmd-a backspace");
    cx.run_until_parked();
    assert_eq!(
        fx.draft(cx),
        "",
        "select all and delete act on the composer"
    );
    assert_eq!(text(&fx.editor, cx), "The quick brown fox\nNext.");
    assert!(fx.is_open(cx));
}

#[gpui::test]
fn a_context_menu_item_can_open_it(cx: &mut TestAppContext) {
    let (fx, cx) = open_quick(paragraphs(&["The quick brown fox", "Next."]), cx);
    // The app adds its item to the text menu; choosing it opens the chat
    // on the selection the right-click made.
    let quick = fx.quick.downgrade();
    fx.tools.update(cx, |tools, _| {
        tools.set_text_menu_extension(move |_, _, _| {
            let quick = quick.clone();
            vec![innerword_ui::ContextMenuItem::new(
                "Ask the Assistant",
                move |window, cx| {
                    quick.update(cx, |quick, cx| quick.toggle(window, cx)).ok();
                },
            )]
        })
    });
    select(&fx.editor, TextRange::in_block(0, 4, 9), cx);
    cx.simulate_keystrokes("ctrl-enter");
    cx.run_until_parked();
    assert!(menu_is_open(cx));
    choose("Ask the Assistant", cx);

    assert!(!menu_is_open(cx));
    assert!(fx.is_open(cx));
    assert!(fx.composer_focused(cx));
    assert_eq!(
        fx.quick.read_with(cx, |quick, _| quick.quote()).as_deref(),
        Some("quick")
    );
    assert_eq!(fx.tools.read_with(cx, |tools, _| tools.active_tool()), None);
}
