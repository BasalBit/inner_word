use gpui::TestAppContext;
use writer_core::{Block, BlockKind, Document, InlineAttrs, Position, Selection, TextRange};
use writer_editor_tools::ToolKind;

use crate::support::*;

fn linked_document() -> Document {
    let link = InlineAttrs {
        link: Some("https://old.example".into()),
        ..InlineAttrs::default()
    };
    Document::from_blocks(vec![
        Block::new(BlockKind::Paragraph)
            .with_span("see ", InlineAttrs::default())
            .with_span("the docs", link)
            .with_span(" now", InlineAttrs::default()),
    ])
}

fn link_at(
    editor: &gpui::Entity<writer_editor::Editor>,
    offset: usize,
    cx: &mut gpui::VisualTestContext,
) -> Option<String> {
    editor.read_with(cx, |editor, _| {
        editor.document().blocks()[0]
            .attrs_at(offset)
            .and_then(|attrs| attrs.link.clone())
    })
}

fn field(
    tools: &gpui::Entity<writer_editor_tools::EditorTools>,
    cx: &mut gpui::VisualTestContext,
) -> Option<String> {
    tools.read_with(cx, |tools, cx| tools.link_field_text(cx))
}

#[gpui::test]
fn links_the_selection(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(paragraphs(&["Read the manual first"]), cx);
    select(&editor, TextRange::in_block(0, 9, 15), cx);

    cx.simulate_keystrokes("cmd-k");
    assert_eq!(active(&tools, cx), Some(ToolKind::Link));
    assert!(!editor_focused(&editor, cx));
    assert_eq!(field(&tools, cx).as_deref(), Some(""));

    cx.simulate_input("example.com/manual");
    assert_eq!(
        text(&editor, cx),
        "Read the manual first",
        "typing goes to the field"
    );
    cx.simulate_keystrokes("enter");
    assert_eq!(active(&tools, cx), None);
    assert!(editor_focused(&editor, cx));
    assert_eq!(
        link_at(&editor, 9, cx).as_deref(),
        Some("https://example.com/manual")
    );
    assert_eq!(
        link_at(&editor, 14, cx).as_deref(),
        Some("https://example.com/manual")
    );
    assert_eq!(link_at(&editor, 8, cx), None);
    assert_eq!(link_at(&editor, 15, cx), None);
    assert_eq!(text(&editor, cx), "Read the manual first");

    cx.simulate_keystrokes("cmd-z");
    assert_eq!(link_at(&editor, 9, cx), None);
}

#[gpui::test]
fn edits_and_removes_the_link_at_the_caret(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(linked_document(), cx);
    caret(&editor, 0, 7, cx);

    // Prefilled with the link under the caret, all selected.
    cx.simulate_keystrokes("cmd-k");
    assert_eq!(field(&tools, cx).as_deref(), Some("https://old.example"));
    cx.simulate_input("https://new.example/a");
    assert_eq!(field(&tools, cx).as_deref(), Some("https://new.example/a"));
    cx.simulate_keystrokes("enter");
    // The whole link changed, the text and the caret did not.
    for offset in 4..12 {
        assert_eq!(
            link_at(&editor, offset, cx).as_deref(),
            Some("https://new.example/a")
        );
    }
    assert_eq!(link_at(&editor, 3, cx), None);
    assert_eq!(link_at(&editor, 12, cx), None);
    assert_eq!(text(&editor, cx), "see the docs now");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.selection(), Selection::caret(Position::new(0, 7)));
    });

    // Remove from inside the popover.
    cx.simulate_keystrokes("cmd-k");
    assert_eq!(active(&tools, cx), Some(ToolKind::Link));
    cx.simulate_keystrokes("cmd-shift-k");
    assert_eq!(active(&tools, cx), None);
    assert!(editor_focused(&editor, cx));
    assert_eq!(link_at(&editor, 7, cx), None);
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(
        link_at(&editor, 7, cx).as_deref(),
        Some("https://new.example/a")
    );

    // Remove straight from the editor.
    cx.simulate_keystrokes("cmd-shift-k");
    assert_eq!(link_at(&editor, 7, cx), None);
    cx.simulate_keystrokes("cmd-z");

    // An emptied field removes the link too.
    cx.simulate_keystrokes("cmd-k cmd-a backspace enter");
    assert_eq!(link_at(&editor, 7, cx), None);
    assert_eq!(text(&editor, cx), "see the docs now");
}

#[gpui::test]
fn inserts_the_url_as_linked_text_at_a_collapsed_caret(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(paragraphs(&["Source: "]), cx);
    caret(&editor, 0, 8, cx);

    cx.simulate_keystrokes("cmd-k");
    assert_eq!(field(&tools, cx).as_deref(), Some(""));
    cx.simulate_input("https://example.org/paper");
    cx.simulate_keystrokes("enter");
    assert_eq!(text(&editor, cx), "Source: https://example.org/paper");
    assert_eq!(
        link_at(&editor, 8, cx).as_deref(),
        Some("https://example.org/paper")
    );
    assert_eq!(link_at(&editor, 0, cx), None);
    assert!(editor_focused(&editor, cx));
    // One undo step for the insertion.
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(text(&editor, cx), "Source: ");
    cx.simulate_keystrokes("cmd-shift-z");
    assert_eq!(text(&editor, cx), "Source: https://example.org/paper");

    // Text typed next is not part of the link.
    cx.simulate_input(" x");
    editor.read_with(cx, |editor, _| {
        let block = &editor.document().blocks()[0];
        assert_eq!(block.attrs_at(block.len() - 1).unwrap().link, None);
    });
}

#[gpui::test]
fn escape_cancels_and_cmd_enter_opens_the_browser(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(linked_document(), cx);
    caret(&editor, 0, 7, cx);

    cx.simulate_keystrokes("cmd-k");
    cx.simulate_input("changed.example");
    cx.simulate_keystrokes("cmd-enter");
    assert_eq!(cx.opened_url().as_deref(), Some("https://changed.example"));
    assert_eq!(
        active(&tools, cx),
        Some(ToolKind::Link),
        "opening does not apply or close"
    );

    cx.simulate_keystrokes("escape");
    assert_eq!(active(&tools, cx), None);
    assert!(editor_focused(&editor, cx));
    assert_eq!(
        link_at(&editor, 7, cx).as_deref(),
        Some("https://old.example")
    );
    editor.read_with(cx, |editor, _| {
        assert!(!editor.has_popup());
        assert!(!editor.state().can_undo(), "nothing was changed");
    });

    // The same shortcut toggles the popover.
    cx.simulate_keystrokes("cmd-k");
    assert_eq!(active(&tools, cx), Some(ToolKind::Link));
    cx.simulate_keystrokes("cmd-k");
    assert_eq!(active(&tools, cx), None);
    assert!(editor_focused(&editor, cx));
}

#[gpui::test]
fn a_selected_url_is_prefilled(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(paragraphs(&["go to www.example.com today"]), cx);
    select(&editor, TextRange::in_block(0, 6, 21), cx);

    cx.simulate_keystrokes("cmd-k");
    assert_eq!(field(&tools, cx).as_deref(), Some("www.example.com"));
    cx.simulate_keystrokes("enter");
    assert_eq!(
        link_at(&editor, 6, cx).as_deref(),
        Some("https://www.example.com")
    );
    assert_eq!(text(&editor, cx), "go to www.example.com today");
}
