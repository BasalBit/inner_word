use gpui::{TestAppContext, VisualTestContext, px};
use innerword_core::{Block, BlockKind, Document, InlineAttrs, Position, Selection, TextRange};
use innerword_editor_tools::ToolKind;

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
    editor: &gpui::Entity<innerword_editor::Editor>,
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
    tools: &gpui::Entity<innerword_editor_tools::EditorTools>,
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

#[gpui::test]
fn the_popover_of_a_link_over_several_rows_hangs_below_its_last_row(cx: &mut TestAppContext) {
    setup(cx);
    let words = "a link that goes on for longer than one row of the column ".repeat(5);
    let link = InlineAttrs {
        link: Some("https://example.com".into()),
        ..InlineAttrs::default()
    };
    let document = Document::from_blocks(vec![
        Block::new(BlockKind::Paragraph)
            .with_span("Before ", InlineAttrs::default())
            .with_span(words.trim_end(), link)
            .with_span(" after.", InlineAttrs::default()),
        Block::paragraph("Next paragraph"),
    ]);
    let (tools, editor, cx) = open(document, cx);
    let link_rows = editor.read_with(cx, |editor, _| {
        let range = editor
            .document()
            .link_at(innerword_core::Position::new(0, 10))
            .unwrap()
            .range;
        editor.range_bounds(range).unwrap()
    });
    assert!(link_rows.len() >= 3, "the link wraps: {link_rows:?}");
    let last = *link_rows.last().unwrap();

    // The caret is on the link's first row: under the caret the popover
    // would cover the rest of the link.
    caret(&editor, 0, 10, cx);
    cx.simulate_keystrokes("cmd-k");
    assert_eq!(field(&tools, cx).as_deref(), Some("https://example.com"));
    let popover = cx.debug_bounds("link-popover").unwrap();
    assert!(
        popover.top() >= last.bottom(),
        "below the last row of the link: {popover:?} {last:?}"
    );
    assert!(popover.top() - last.bottom() < px(12.), "and close to it");
    assert!(
        (popover.left() - last.left()).abs() < px(12.),
        "at the start of that row"
    );
    cx.simulate_keystrokes("escape");

    // On the last row the caret is the anchor, as for a link on one row.
    let end = editor.read_with(cx, |editor, _| {
        editor
            .document()
            .link_at(innerword_core::Position::new(0, 10))
            .unwrap()
            .range
            .end
    });
    caret(&editor, 0, end.offset - 2, cx);
    cx.simulate_keystrokes("cmd-k");
    let caret_bounds = editor.read_with(cx, |editor, _| editor.caret_bounds().unwrap());
    let popover = cx.debug_bounds("link-popover").unwrap();
    assert!(popover.top() >= caret_bounds.bottom());
    assert_eq!(popover.left(), caret_bounds.left());
}

#[gpui::test]
fn open_and_apply_wait_for_an_address(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(paragraphs(&["Some words here"]), cx);
    select(&editor, TextRange::in_block(0, 5, 10), cx);
    let enabled = |button: &str, cx: &mut VisualTestContext| {
        tools.read_with(cx, |tools, cx| tools.link_button_enabled(button, cx))
    };
    let click = |button: &'static str, cx: &mut VisualTestContext| {
        let target = cx.debug_bounds(button).unwrap().center();
        cx.simulate_click(target, gpui::Modifiers::none());
        cx.run_until_parked();
    };
    assert_eq!(enabled("link-open", cx), None, "closed");

    // An empty field on unlinked text: nothing to open, nothing to apply.
    cx.simulate_keystrokes("cmd-k");
    assert_eq!(enabled("link-open", cx), Some(false));
    assert_eq!(enabled("link-apply", cx), Some(false));
    assert_eq!(enabled("link-cancel", cx), Some(true));
    click("link-open", cx);
    assert_eq!(cx.opened_url(), None);
    click("link-apply", cx);
    assert_eq!(
        active(&tools, cx),
        Some(ToolKind::Link),
        "a dimmed button does nothing"
    );

    // With an address both are offered.
    cx.simulate_input("example.com");
    assert_eq!(enabled("link-open", cx), Some(true));
    assert_eq!(enabled("link-apply", cx), Some(true));
    click("link-apply", cx);
    assert_eq!(
        link_at(&editor, 6, cx).as_deref(),
        Some("https://example.com")
    );

    // On a link, an emptied field can still be applied: it takes the link
    // off. There is still nothing to open.
    cx.simulate_keystrokes("cmd-k cmd-a backspace");
    assert_eq!(enabled("link-open", cx), Some(false));
    assert_eq!(enabled("link-apply", cx), Some(true));
    click("link-apply", cx);
    assert_eq!(link_at(&editor, 6, cx), None);
    assert_eq!(active(&tools, cx), None);
}

#[gpui::test]
fn the_buttons_of_the_popover_say_what_they_do(cx: &mut TestAppContext) {
    setup(cx);
    let (_tools, editor, cx) = open(linked_document(), cx);
    caret(&editor, 0, 6, cx);
    cx.simulate_keystrokes("cmd-k");

    // Resting the pointer on a button shows its tooltip at the pointer.
    for button in ["link-apply", "link-open", "link-remove", "link-cancel"] {
        let pointer = cx
            .debug_bounds(button)
            .unwrap_or_else(|| panic!("{button} is not drawn"))
            .center();
        cx.simulate_mouse_move(pointer, None, gpui::Modifiers::default());
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(600));
        cx.run_until_parked();
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        let tip = cx
            .debug_bounds("tool-tip")
            .unwrap_or_else(|| panic!("no tooltip over {button}"));
        assert!(
            (tip.origin.x - pointer.x).abs() <= px(12.) && tip.origin.y > pointer.y,
            "the tooltip of {button} is at {tip:?}, the pointer at {pointer:?}"
        );
        cx.simulate_mouse_move(
            gpui::point(px(40.), px(700.)),
            None,
            gpui::Modifiers::default(),
        );
        cx.run_until_parked();
    }
}
