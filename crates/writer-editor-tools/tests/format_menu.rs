use gpui::TestAppContext;
use writer_core::{Alignment, Block, BlockKind, Document, Highlight, InlineAttrs, TextRange};
use writer_editor_tools::{Mark, TEXT_COLORS, ToolKind};

use crate::support::*;

#[gpui::test]
fn opens_with_inline_formatting_first_on_a_selection(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(paragraphs(&["Hello brave world"]), cx);
    select(&editor, TextRange::in_block(0, 6, 11), cx);

    cx.simulate_keystrokes("cmd-.");
    assert_eq!(active(&tools, cx), Some(ToolKind::Format));
    assert!(!editor_focused(&editor, cx), "focus moves into the menu's filter");

    let rows = rows(&tools, cx);
    assert!(rows[0].header && rows[0].label == "Text");
    assert_eq!(rows[1].label, "Bold");
    assert!(rows[1].selected);
    // The real binding from the keymap.
    assert_eq!(rows[1].key_hint.as_deref(), Some("⌘B"));
    let labels = labels(&tools, cx);
    for expected in [
        "Bold",
        "Italic",
        "Underline",
        "Strikethrough",
        "Inline code",
        "Link",
        "Font",
        "Clear formatting",
        "Yellow highlight",
        "Purple highlight",
        "Default text colour",
        "Red text",
        "Paragraph",
        "Heading 1",
        "Heading 6",
        "Bullet list",
        "Numbered list",
        "To-do list",
        "Quote",
        "Code block",
        "Align left",
        "Align centre",
        "Align right",
        "Justify",
        "Larger text",
        "Smaller text",
        "Reset text size",
    ] {
        assert!(labels.iter().any(|label| label == expected), "missing {expected}");
    }
    let position = |label: &str| labels.iter().position(|other| other == label).unwrap();
    assert!(position("Bold") < position("Heading 1"));
    assert_eq!(row(&tools, "Heading 1", cx).key_hint.as_deref(), Some("⌥⌘1"));
    assert_eq!(row(&tools, "Link", cx).key_hint.as_deref(), Some("⌘K"));
    assert_eq!(row(&tools, "Font", cx).key_hint.as_deref(), Some("⌘T"));

    // Enter applies the selected row and closes; focus is back in the editor.
    cx.simulate_keystrokes("enter");
    assert_eq!(active(&tools, cx), None);
    assert!(editor_focused(&editor, cx));
    editor.read_with(cx, |editor, _| {
        let block = &editor.document().blocks()[0];
        assert!(block.attrs_at(6).unwrap().bold);
        assert!(!block.attrs_at(0).unwrap().bold);
        assert!(!block.attrs_at(11).unwrap().bold);
    });

    // One undo step.
    cx.simulate_keystrokes("cmd-z");
    editor.read_with(cx, |editor, _| {
        assert!(!editor.document().blocks()[0].attrs_at(6).unwrap().bold);
    });
}

#[gpui::test]
fn leads_with_block_options_on_a_collapsed_caret(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(paragraphs(&["Title", "Body"]), cx);

    cx.simulate_keystrokes("cmd-.");
    let rows = rows(&tools, cx);
    assert!(rows[0].header && rows[0].label == "Turn into");
    assert_eq!(rows[1].label, "Paragraph");
    assert_eq!(rows[1].mark, Mark::Check, "the caret's block is a paragraph");
    let labels = labels(&tools, cx);
    let position = |label: &str| labels.iter().position(|other| other == label).unwrap();
    assert!(position("Heading 1") < position("Align left"));
    assert!(position("Reset text size") < position("Bold"));
    assert_eq!(row(&tools, "Align left", cx).mark, Mark::Check);

    // Arrows move, enter applies: down twice from Paragraph is Heading 2.
    cx.simulate_keystrokes("down down");
    assert_eq!(selected(&tools, cx).as_deref(), Some("Heading 2"));
    cx.simulate_keystrokes("up");
    assert_eq!(selected(&tools, cx).as_deref(), Some("Heading 1"));
    cx.simulate_keystrokes("enter");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().blocks()[0].kind().heading_level(), Some(1));
        assert_eq!(editor.document().blocks()[1].kind(), &BlockKind::Paragraph);
    });
    assert!(editor_focused(&editor, cx));

    // The state is read again on the next open.
    cx.simulate_keystrokes("cmd-.");
    assert_eq!(row(&tools, "Heading 1", cx).mark, Mark::Check);
    assert_eq!(row(&tools, "Paragraph", cx).mark, Mark::None);
    cx.simulate_keystrokes("escape");
    assert_eq!(active(&tools, cx), None);
    assert!(editor_focused(&editor, cx));
}

#[gpui::test]
fn typing_filters_and_escape_closes(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(paragraphs(&["One", "Two"]), cx);
    caret(&editor, 1, 0, cx);

    cx.simulate_keystrokes("cmd-.");
    cx.simulate_input("quo");
    let filtered = labels(&tools, cx);
    assert_eq!(filtered.first().map(String::as_str), Some("Quote"));
    assert!(rows(&tools, cx).iter().all(|row| !row.header), "no headers while filtering");
    // The text went to the filter, not to the document.
    assert_eq!(text(&editor, cx), "One\nTwo");

    // Keywords match too: "h3" finds Heading 3.
    cx.simulate_keystrokes("cmd-a backspace");
    cx.simulate_input("h3");
    assert_eq!(selected(&tools, cx).as_deref(), Some("Heading 3"));

    cx.simulate_keystrokes("cmd-a backspace");
    cx.simulate_input("zzzz");
    assert!(labels(&tools, cx).is_empty());
    // Enter with nothing selected does nothing.
    cx.simulate_keystrokes("enter");
    assert_eq!(active(&tools, cx), Some(ToolKind::Format));

    cx.simulate_keystrokes("escape");
    assert_eq!(active(&tools, cx), None);
    assert!(editor_focused(&editor, cx));
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().blocks()[1].kind(), &BlockKind::Paragraph);
        assert!(!editor.has_popup());
    });

    // Filter then apply.
    cx.simulate_keystrokes("cmd-.");
    cx.simulate_input("quote");
    cx.simulate_keystrokes("enter");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().blocks()[1].kind(), &BlockKind::Quote);
        assert_eq!(editor.document().blocks()[0].kind(), &BlockKind::Paragraph);
    });
}

#[gpui::test]
fn shows_checks_and_mixed_state(cx: &mut TestAppContext) {
    setup(cx);
    let bold = InlineAttrs {
        bold: true,
        ..InlineAttrs::default()
    };
    let yellow = InlineAttrs {
        highlight: Some(Highlight::Yellow),
        ..InlineAttrs::default()
    };
    let document = Document::from_blocks(vec![
        Block::new(BlockKind::heading(2))
            .with_span("bold", bold.clone())
            .with_span(" plain", InlineAttrs::default()),
        Block::new(BlockKind::Paragraph)
            .with_span("marked", yellow)
            .with_alignment(Alignment::Center),
    ]);
    let (tools, editor, cx) = open(document, cx);

    // All bold.
    select(&editor, TextRange::in_block(0, 0, 4), cx);
    cx.simulate_keystrokes("cmd-.");
    assert_eq!(row(&tools, "Bold", cx).mark, Mark::Check);
    assert_eq!(row(&tools, "Italic", cx).mark, Mark::None);
    assert_eq!(row(&tools, "Heading 2", cx).mark, Mark::Check);
    cx.simulate_keystrokes("escape");

    // Bold in a part of the selection, two block kinds, two alignments,
    // highlight in a part.
    select(
        &editor,
        TextRange::new(writer_core::Position::new(0, 0), writer_core::Position::new(1, 6)),
        cx,
    );
    cx.simulate_keystrokes("cmd-.");
    assert_eq!(row(&tools, "Bold", cx).mark, Mark::Mixed);
    assert_eq!(row(&tools, "Heading 2", cx).mark, Mark::Mixed);
    assert_eq!(row(&tools, "Paragraph", cx).mark, Mark::Mixed);
    assert_eq!(row(&tools, "Heading 1", cx).mark, Mark::None);
    assert_eq!(row(&tools, "Align left", cx).mark, Mark::Mixed);
    assert_eq!(row(&tools, "Align centre", cx).mark, Mark::Mixed);
    assert_eq!(row(&tools, "Align right", cx).mark, Mark::None);
    assert_eq!(row(&tools, "Yellow highlight", cx).mark, Mark::Mixed);
    assert_eq!(row(&tools, "Green highlight", cx).mark, Mark::None);
    cx.simulate_keystrokes("escape");

    select(&editor, TextRange::in_block(1, 0, 6), cx);
    cx.simulate_keystrokes("cmd-.");
    assert_eq!(row(&tools, "Yellow highlight", cx).mark, Mark::Check);
    assert_eq!(row(&tools, "Default text colour", cx).mark, Mark::Check);
    assert_eq!(row(&tools, "Align centre", cx).mark, Mark::Check);
}

#[gpui::test]
fn applies_highlight_and_text_colours(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(paragraphs(&["Colour me in"]), cx);
    select(&editor, TextRange::in_block(0, 0, 6), cx);

    cx.simulate_keystrokes("cmd-.");
    cx.simulate_input("green hi");
    assert_eq!(selected(&tools, cx).as_deref(), Some("Green highlight"));
    cx.simulate_keystrokes("enter");
    editor.read_with(cx, |editor, _| {
        let block = &editor.document().blocks()[0];
        assert_eq!(block.attrs_at(0).unwrap().highlight, Some(Highlight::Green));
        assert_eq!(block.attrs_at(7).unwrap().highlight, None);
    });

    cx.simulate_keystrokes("cmd-.");
    assert_eq!(row(&tools, "Green highlight", cx).mark, Mark::Check);
    cx.simulate_input("blue text");
    assert_eq!(selected(&tools, cx).as_deref(), Some("Blue text"));
    cx.simulate_keystrokes("enter");
    let blue = TEXT_COLORS.iter().find(|(name, _)| *name == "Blue").unwrap().1;
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().blocks()[0].attrs_at(0).unwrap().color, Some(blue));
    });

    // Choosing the checked highlight again removes it (a toggle).
    cx.simulate_keystrokes("cmd-.");
    assert_eq!(row(&tools, "Blue text", cx).mark, Mark::Check);
    cx.simulate_input("green hi");
    cx.simulate_keystrokes("enter");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().blocks()[0].attrs_at(0).unwrap().highlight, None);
    });

    // Back to the default colour.
    cx.simulate_keystrokes("cmd-.");
    cx.simulate_input("default");
    cx.simulate_keystrokes("enter");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().blocks()[0].attrs_at(0).unwrap().color, None);
    });
    assert!(editor_focused(&editor, cx));
}

#[gpui::test]
fn shortcut_toggles_and_read_only_blocks(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(paragraphs(&["Text"]), cx);

    cx.simulate_keystrokes("cmd-.");
    assert_eq!(active(&tools, cx), Some(ToolKind::Format));
    editor.read_with(cx, |editor, _| assert!(editor.has_popup()));
    // The same shortcut closes it again.
    cx.simulate_keystrokes("cmd-.");
    assert_eq!(active(&tools, cx), None);
    assert!(editor_focused(&editor, cx));

    editor.update(cx, |editor, cx| editor.set_read_only(true, cx));
    cx.simulate_keystrokes("cmd-.");
    assert_eq!(active(&tools, cx), None);
}
