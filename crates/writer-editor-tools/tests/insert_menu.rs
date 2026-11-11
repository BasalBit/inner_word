use gpui::TestAppContext;
use writer_core::{Block, BlockKind, Document, Position, Selection};
use writer_editor_tools::{ToolKind, actions::OpenInsertMenu, today_string};

use crate::support::*;

#[gpui::test]
fn slash_in_an_empty_block_opens_and_filters(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(Document::new(), cx);

    cx.simulate_input("/");
    assert_eq!(active(&tools, cx), Some(ToolKind::Insert));
    assert!(editor_focused(&editor, cx), "focus stays in the document");
    assert_eq!(text(&editor, cx), "/");
    editor.read_with(cx, |editor, _| assert!(editor.has_popup()));
    assert_eq!(
        labels(&tools, cx),
        [
            "Paragraph",
            "Heading 1",
            "Heading 2",
            "Heading 3",
            "Heading 4",
            "Heading 5",
            "Heading 6",
            "Bullet list",
            "Numbered list",
            "To-do list",
            "Quote",
            "Code block",
            "Divider",
            "Page break",
            "Link",
            "Today's date",
        ]
    );
    assert_eq!(row(&tools, "Page break", cx).key_hint.as_deref(), Some("⌘↩"));

    // What is typed after the slash goes to the document and filters.
    cx.simulate_input("hea");
    assert_eq!(text(&editor, cx), "/hea");
    let filtered = labels(&tools, cx);
    assert_eq!(filtered.len(), 6);
    assert_eq!(selected(&tools, cx).as_deref(), Some("Heading 1"));

    // Up and down go to the menu, not to the caret.
    cx.simulate_keystrokes("down down up");
    assert_eq!(selected(&tools, cx).as_deref(), Some("Heading 2"));
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.selection(), Selection::caret(Position::new(0, 4)));
    });

    // Enter applies: slash and query are gone, the block is a heading.
    cx.simulate_keystrokes("enter");
    assert_eq!(active(&tools, cx), None);
    editor.read_with(cx, |editor, _| {
        let document = editor.document();
        assert_eq!(document.block_count(), 1);
        assert_eq!(document.blocks()[0].text(), "");
        assert_eq!(document.blocks()[0].kind().heading_level(), Some(2));
        assert!(!editor.has_popup());
    });
    assert!(editor_focused(&editor, cx));
    cx.simulate_input("Title");
    assert_eq!(text(&editor, cx), "Title");

    // Removing the text and changing the kind were one undo step.
    cx.simulate_keystrokes("cmd-z cmd-z");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().blocks()[0].text(), "/hea");
        assert_eq!(editor.document().blocks()[0].kind(), &BlockKind::Paragraph);
    });
    assert_eq!(active(&tools, cx), None, "undo does not reopen the menu");
}

#[gpui::test]
fn tab_applies_and_every_kind_of_item_works(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(Document::new(), cx);

    cx.simulate_input("/todo");
    assert_eq!(selected(&tools, cx).as_deref(), Some("To-do list"));
    cx.simulate_keystrokes("tab");
    editor.read_with(cx, |editor, _| {
        let block = &editor.document().blocks()[0];
        assert_eq!(block.kind(), &BlockKind::TodoItem { checked: false });
        assert_eq!(block.text(), "");
        assert_eq!(block.indent(), 0, "tab went to the menu, not to indent");
    });

    // A divider replaces the empty paragraph and the caret lands after it.
    editor.update(cx, |editor, cx| editor.set_document(Document::new(), cx));
    cx.simulate_input("/div");
    assert_eq!(selected(&tools, cx).as_deref(), Some("Divider"));
    cx.simulate_keystrokes("enter");
    editor.read_with(cx, |editor, _| {
        let document = editor.document();
        assert_eq!(document.blocks()[0].kind(), &BlockKind::Divider);
        assert_eq!(document.block_count(), 2);
        assert_eq!(editor.selection(), Selection::caret(Position::new(1, 0)));
    });
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(text(&editor, cx), "/div");

    // Page break after text: the slash text goes, the break comes after.
    editor.update(cx, |editor, cx| {
        editor.set_document(paragraphs(&["Chapter one "]), cx);
        editor.edit(|state| state.set_caret(Position::new(0, 12)), cx);
    });
    cx.simulate_input("/page");
    assert_eq!(selected(&tools, cx).as_deref(), Some("Page break"));
    cx.simulate_keystrokes("enter");
    editor.read_with(cx, |editor, _| {
        let document = editor.document();
        assert_eq!(document.blocks()[0].text(), "Chapter one ");
        assert_eq!(document.blocks()[1].kind(), &BlockKind::PageBreak);
    });

    // Today's date is inserted as text in place of the slash.
    editor.update(cx, |editor, cx| {
        editor.set_document(paragraphs(&["On "]), cx);
        editor.edit(|state| state.set_caret(Position::new(0, 3)), cx);
    });
    cx.simulate_input("/date");
    assert_eq!(selected(&tools, cx).as_deref(), Some("Today's date"));
    cx.simulate_keystrokes("enter");
    assert_eq!(text(&editor, cx), format!("On {}", today_string()));
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(text(&editor, cx), "On /date");

    // Choosing the kind the block already has keeps it (no toggle).
    editor.update(cx, |editor, cx| {
        editor.set_document(
            Document::from_blocks(vec![Block::new(BlockKind::Quote)]),
            cx,
        );
    });
    cx.simulate_input("/quote");
    cx.simulate_keystrokes("enter");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().blocks()[0].kind(), &BlockKind::Quote);
        assert_eq!(editor.document().blocks()[0].text(), "");
    });
}

#[gpui::test]
fn link_item_opens_the_link_popover(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(Document::new(), cx);

    cx.simulate_input("/link");
    assert_eq!(selected(&tools, cx).as_deref(), Some("Link"));
    cx.simulate_keystrokes("enter");
    assert_eq!(active(&tools, cx), Some(ToolKind::Link));
    assert_eq!(text(&editor, cx), "");
    cx.simulate_input("example.com");
    cx.simulate_keystrokes("enter");
    editor.read_with(cx, |editor, _| {
        let block = &editor.document().blocks()[0];
        assert_eq!(block.text(), "example.com");
        assert_eq!(
            block.attrs_at(0).unwrap().link.as_deref(),
            Some("https://example.com")
        );
    });
    assert!(editor_focused(&editor, cx));
}

#[gpui::test]
fn escape_keeps_the_text_and_deleting_the_slash_closes(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(Document::new(), cx);

    cx.simulate_input("/he");
    cx.simulate_keystrokes("escape");
    assert_eq!(active(&tools, cx), None);
    assert_eq!(text(&editor, cx), "/he");
    assert!(editor_focused(&editor, cx));
    // Typing on does not bring it back; enter is a plain enter again.
    cx.simulate_input("y");
    assert_eq!(active(&tools, cx), None);
    cx.simulate_keystrokes("enter");
    assert_eq!(text(&editor, cx), "/hey\n");

    // Backspace over the query keeps it open, over the slash closes it.
    cx.simulate_input("/q");
    assert_eq!(active(&tools, cx), Some(ToolKind::Insert));
    cx.simulate_keystrokes("backspace");
    assert_eq!(active(&tools, cx), Some(ToolKind::Insert));
    assert_eq!(labels(&tools, cx).len(), 16);
    cx.simulate_keystrokes("backspace");
    assert_eq!(active(&tools, cx), None);
    assert_eq!(text(&editor, cx), "/hey\n");
    editor.read_with(cx, |editor, _| assert!(!editor.has_popup()));
}

#[gpui::test]
fn closes_when_nothing_matches_or_the_caret_leaves(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(paragraphs(&["First", ""]), cx);
    caret(&editor, 1, 0, cx);

    // One character without a match shows the empty state, two close.
    cx.simulate_input("/z");
    assert_eq!(active(&tools, cx), Some(ToolKind::Insert));
    assert!(labels(&tools, cx).is_empty());
    cx.simulate_input("z");
    assert_eq!(active(&tools, cx), None);
    assert_eq!(text(&editor, cx), "First\n/zz");

    // With no row to apply, enter is the document's.
    editor.update(cx, |editor, cx| editor.set_document(paragraphs(&["First", ""]), cx));
    caret(&editor, 1, 0, cx);
    cx.simulate_input("/z");
    cx.simulate_keystrokes("enter");
    assert_eq!(active(&tools, cx), None);
    assert_eq!(text(&editor, cx), "First\n/z\n");

    // Moving the caret out of the query closes the menu.
    editor.update(cx, |editor, cx| editor.set_document(paragraphs(&["First", ""]), cx));
    caret(&editor, 1, 0, cx);
    cx.simulate_input("/he");
    cx.simulate_keystrokes("left");
    assert_eq!(active(&tools, cx), Some(ToolKind::Insert), "still inside the query");
    // The query is now "h".
    assert_eq!(labels(&tools, cx)[..6].join(","), "Heading 1,Heading 2,Heading 3,Heading 4,Heading 5,Heading 6");
    cx.simulate_keystrokes("cmd-up");
    assert_eq!(active(&tools, cx), None);
    assert_eq!(text(&editor, cx), "First\n/he");

    // A space right after the slash is ordinary prose.
    editor.update(cx, |editor, cx| editor.set_document(paragraphs(&["this "]), cx));
    caret(&editor, 0, 5, cx);
    cx.simulate_input("/");
    assert_eq!(active(&tools, cx), Some(ToolKind::Insert));
    cx.simulate_input(" that");
    assert_eq!(active(&tools, cx), None);
    assert_eq!(text(&editor, cx), "this / that");
}

#[gpui::test]
fn opens_only_at_a_block_start_or_after_whitespace(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(paragraphs(&["and"]), cx);

    // Inside a word: no menu.
    caret(&editor, 0, 3, cx);
    cx.simulate_input("/");
    assert_eq!(active(&tools, cx), None);
    cx.simulate_input("or ");
    assert_eq!(text(&editor, cx), "and/or ");
    // After whitespace: menu.
    cx.simulate_input("/");
    assert_eq!(active(&tools, cx), Some(ToolKind::Insert));
    cx.simulate_input("bul");
    cx.simulate_keystrokes("enter");
    editor.read_with(cx, |editor, _| {
        let block = &editor.document().blocks()[0];
        assert_eq!(block.kind(), &BlockKind::BulletItem);
        assert_eq!(block.text(), "and/or ");
    });

    // Never in a code block.
    editor.update(cx, |editor, cx| {
        editor.set_document(
            Document::from_blocks(vec![Block::new(BlockKind::code())]),
            cx,
        );
    });
    cx.simulate_input("/");
    assert_eq!(active(&tools, cx), None);
    assert_eq!(text(&editor, cx), "/");

    // Not in a read-only editor (nothing is typed either).
    editor.update(cx, |editor, cx| {
        editor.set_document(Document::new(), cx);
        editor.set_read_only(true, cx);
    });
    cx.simulate_input("/");
    assert_eq!(active(&tools, cx), None);
}

#[gpui::test]
fn the_command_opens_it_without_a_slash(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(paragraphs(&["Note"]), cx);
    caret(&editor, 0, 4, cx);

    cx.dispatch_action(OpenInsertMenu);
    assert_eq!(active(&tools, cx), Some(ToolKind::Insert));
    assert!(editor_focused(&editor, cx));
    assert_eq!(labels(&tools, cx).len(), 16);
    // The typed filter is removed again when an item is applied.
    cx.simulate_input("quo");
    assert_eq!(text(&editor, cx), "Notequo");
    assert_eq!(selected(&tools, cx).as_deref(), Some("Quote"));
    cx.simulate_keystrokes("enter");
    editor.read_with(cx, |editor, _| {
        let block = &editor.document().blocks()[0];
        assert_eq!(block.text(), "Note");
        assert_eq!(block.kind(), &BlockKind::Quote);
    });

    // Backspacing past where it was opened closes it.
    cx.dispatch_action(OpenInsertMenu);
    assert_eq!(active(&tools, cx), Some(ToolKind::Insert));
    cx.simulate_keystrokes("backspace");
    assert_eq!(active(&tools, cx), None);
    assert_eq!(text(&editor, cx), "Not");
}
