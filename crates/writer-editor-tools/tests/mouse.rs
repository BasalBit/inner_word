//! The mouse is never required, but it works: rows and buttons can be
//! clicked, and clicks on an overlay do not fall through to the document.

use gpui::{Bounds, Modifiers, Pixels, TestAppContext, VisualTestContext};
use writer_core::{Block, BlockKind, Document, InlineAttrs, TextRange};
use writer_editor_tools::ToolKind;

use crate::support::*;

fn bounds(selector: &'static str, cx: &mut VisualTestContext) -> Bounds<Pixels> {
    cx.debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} was not drawn"))
}

fn click(selector: &'static str, cx: &mut VisualTestContext) {
    let target = bounds(selector, cx).center();
    cx.simulate_click(target, Modifiers::none());
    cx.run_until_parked();
}

#[gpui::test]
fn menu_rows_can_be_hovered_and_clicked(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(paragraphs(&["Hello brave world", "Second", "Third"]), cx);
    select(&editor, TextRange::in_block(0, 6, 11), cx);

    cx.simulate_keystrokes("cmd-.");
    // Entries: header "Text", Bold, Italic, Underline, ...
    let underline = bounds("menu-row-3", cx).center();
    cx.simulate_mouse_move(underline, None, Modifiers::none());
    assert_eq!(selected(&tools, cx).as_deref(), Some("Underline"));

    click("menu-row-2", cx);
    assert_eq!(active(&tools, cx), None);
    assert!(editor_focused(&editor, cx));
    editor.read_with(cx, |editor, _| {
        let block = &editor.document().blocks()[0];
        assert!(block.attrs_at(6).unwrap().italic);
        assert!(!block.attrs_at(6).unwrap().underline);
        // The click did not reach the text under the menu.
        assert_eq!(editor.selection().range(), TextRange::in_block(0, 6, 11));
    });

    // A click on a header does nothing and keeps the menu.
    cx.simulate_keystrokes("cmd-.");
    click("menu-row-0", cx);
    assert_eq!(active(&tools, cx), Some(ToolKind::Format));
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.selection().range(), TextRange::in_block(0, 6, 11));
    });
    // Typing still goes to the filter after the click.
    cx.simulate_input("bold");
    assert_eq!(selected(&tools, cx).as_deref(), Some("Bold"));
    assert_eq!(text(&editor, cx), "Hello brave world\nSecond\nThird");
}

#[gpui::test]
fn insert_menu_rows_can_be_clicked(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(Document::new(), cx);

    cx.simulate_input("/");
    // Entries: Heading 1, Heading 2, ...
    click("menu-row-0", cx);
    assert_eq!(active(&tools, cx), None);
    assert!(editor_focused(&editor, cx));
    editor.read_with(cx, |editor, _| {
        assert_eq!(
            editor.document().blocks()[0].kind().heading_level(),
            Some(1)
        );
        assert_eq!(editor.document().blocks()[0].text(), "");
    });
}

#[gpui::test]
fn find_bar_buttons_work_and_keep_focus_in_the_field(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(paragraphs(&["cat Cat cat", "concat"]), cx);
    let status = |cx: &mut VisualTestContext| tools.read_with(cx, |tools, _| tools.find_status());

    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("cat");
    assert_eq!(status(cx), "1 of 4");
    click("find-next", cx);
    assert_eq!(status(cx), "2 of 4");
    click("find-previous", cx);
    assert_eq!(status(cx), "1 of 4");
    click("find-case", cx);
    assert_eq!(status(cx), "1 of 3");
    click("find-word", cx);
    assert_eq!(status(cx), "1 of 2");
    click("find-case", cx);
    click("find-word", cx);
    assert_eq!(status(cx), "1 of 4");
    // Focus never left the query field.
    cx.simulate_input("x");
    assert_eq!(
        tools.read_with(cx, |tools, _| tools.find_query().to_string()),
        "catx"
    );
    cx.simulate_keystrokes("backspace");

    // The disclosure shows the replace row; its buttons replace.
    click("find-disclosure", cx);
    assert!(tools.read_with(cx, |tools, _| tools.find_replace_visible()));
    cx.simulate_keystrokes("tab");
    cx.simulate_input("dog");
    click("find-replace-one", cx);
    assert_eq!(text(&editor, cx), "dog Cat cat\nconcat");
    click("find-replace-all", cx);
    assert_eq!(text(&editor, cx), "dog dog dog\ncondog");
    assert_eq!(status(cx), "Replaced 3");

    click("find-close", cx);
    assert_eq!(active(&tools, cx), None);
    assert!(editor_focused(&editor, cx));
}

#[gpui::test]
fn link_popover_actions_can_be_clicked(cx: &mut TestAppContext) {
    setup(cx);
    let link = InlineAttrs {
        link: Some("https://example.com".into()),
        ..InlineAttrs::default()
    };
    let document = Document::from_blocks(vec![
        Block::new(BlockKind::Paragraph).with_span("linked", link),
    ]);
    let (tools, editor, cx) = open(document, cx);
    caret(&editor, 0, 3, cx);
    let link_at = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |editor, _| {
            editor.document().blocks()[0]
                .attrs_at(0)
                .and_then(|attrs| attrs.link.clone())
        })
    };

    cx.simulate_keystrokes("cmd-k");
    click("link-open", cx);
    assert_eq!(cx.opened_url().as_deref(), Some("https://example.com"));
    assert_eq!(active(&tools, cx), Some(ToolKind::Link));
    click("link-cancel", cx);
    assert_eq!(active(&tools, cx), None);
    assert!(editor_focused(&editor, cx));
    assert!(link_at(cx).is_some());

    cx.simulate_keystrokes("cmd-k cmd-a");
    cx.simulate_input("https://other.example");
    click("link-apply", cx);
    assert_eq!(link_at(cx).as_deref(), Some("https://other.example"));
    assert!(editor_focused(&editor, cx));

    cx.simulate_keystrokes("cmd-k");
    click("link-remove", cx);
    assert_eq!(link_at(cx), None);
    assert_eq!(active(&tools, cx), None);
    assert!(editor_focused(&editor, cx));
}
