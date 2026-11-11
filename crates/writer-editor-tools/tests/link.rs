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

#[gpui::test]
fn editing_an_existing_link_from_its_edges_and_from_a_selection(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(linked_document(), cx);
    let links = |cx: &mut gpui::VisualTestContext| -> Vec<Option<String>> {
        (0..16).map(|offset| link_at(&editor, offset, cx)).collect()
    };
    let old = links(cx);

    // Right after the last character of the link (where the caret is when
    // the link was just made) and right before its first: cmd-k edits that
    // link, it does not start a new one.
    for (offset, url) in [(12, "https://one.example"), (4, "https://two.example")] {
        caret(&editor, 0, offset, cx);
        cx.simulate_keystrokes("cmd-k");
        assert!(
            field(&tools, cx).is_some_and(|text| text.ends_with(".example")),
            "the field has the link at offset {offset}"
        );
        cx.simulate_input(url);
        cx.simulate_keystrokes("enter");
        assert_eq!(
            text(&editor, cx),
            "see the docs now",
            "nothing was inserted"
        );
        for inside in 4..12 {
            assert_eq!(link_at(&editor, inside, cx).as_deref(), Some(url));
        }
        assert_eq!(link_at(&editor, 3, cx), None);
        assert_eq!(link_at(&editor, 12, cx), None);
    }
    // Each edit is one undo step, back to the original link.
    cx.simulate_keystrokes("cmd-z cmd-z");
    assert_eq!(links(cx), old);

    // The link selected exactly: prefilled, and the change covers it.
    select(&editor, TextRange::in_block(0, 4, 12), cx);
    cx.simulate_keystrokes("cmd-k");
    assert_eq!(field(&tools, cx).as_deref(), Some("https://old.example"));
    cx.simulate_keystrokes("end");
    cx.simulate_input("/guide");
    cx.simulate_keystrokes("enter");
    for inside in 4..12 {
        assert_eq!(
            link_at(&editor, inside, cx).as_deref(),
            Some("https://old.example/guide")
        );
    }
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(links(cx), old);

    // A selection that covers the link and plain text around it: the
    // field starts empty (there is no one URL to show), offers to remove,
    // and the new URL goes on everything selected.
    select(&editor, TextRange::in_block(0, 0, 16), cx);
    cx.simulate_keystrokes("cmd-k");
    assert_eq!(field(&tools, cx).as_deref(), Some(""));
    assert!(cx.debug_bounds("link-remove").is_some());
    cx.simulate_input("all.example");
    cx.simulate_keystrokes("enter");
    for inside in 0..16 {
        assert_eq!(
            link_at(&editor, inside, cx).as_deref(),
            Some("https://all.example")
        );
    }
    // And cmd-shift-k takes the link off the whole selection again.
    cx.simulate_keystrokes("cmd-shift-k");
    assert!(links(cx).iter().all(Option::is_none));
    assert_eq!(text(&editor, cx), "see the docs now");
}
