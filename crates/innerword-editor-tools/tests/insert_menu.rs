use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gpui::TestAppContext;
use innerword_core::{Alignment, Block, BlockKind, Document, InlineAttrs, Position, Selection};
use innerword_editor::{InterceptPriority, KeyDisposition};
use innerword_editor_tools::{ToolKind, actions::OpenInsertMenu, today_string};

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
            "Heading 1",
            "Heading 2",
            "Heading 3",
            "Bullet list",
            "Numbered list",
            "To-do list",
            "Quote",
            "Code block",
            "Divider",
            "Page break",
            "Link",
            "Today's date",
            "Paragraph",
            "Heading 4",
            "Heading 5",
            "Heading 6",
        ],
        "the blocks asked for most come first; ten rows are in view"
    );
    assert_eq!(
        row(&tools, "Page break", cx).key_hint.as_deref(),
        Some("⌘↩")
    );

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
    editor.update(cx, |editor, cx| {
        editor.set_document(paragraphs(&["First", ""]), cx)
    });
    caret(&editor, 1, 0, cx);
    cx.simulate_input("/z");
    cx.simulate_keystrokes("enter");
    assert_eq!(active(&tools, cx), None);
    assert_eq!(text(&editor, cx), "First\n/z\n");

    // Moving the caret out of the query closes the menu.
    editor.update(cx, |editor, cx| {
        editor.set_document(paragraphs(&["First", ""]), cx)
    });
    caret(&editor, 1, 0, cx);
    cx.simulate_input("/he");
    cx.simulate_keystrokes("left");
    assert_eq!(
        active(&tools, cx),
        Some(ToolKind::Insert),
        "still inside the query"
    );
    // The query is now "h".
    assert_eq!(
        labels(&tools, cx)[..6].join(","),
        "Heading 1,Heading 2,Heading 3,Heading 4,Heading 5,Heading 6"
    );
    cx.simulate_keystrokes("cmd-up");
    assert_eq!(active(&tools, cx), None);
    assert_eq!(text(&editor, cx), "First\n/he");

    // A space right after the slash is ordinary prose.
    editor.update(cx, |editor, cx| {
        editor.set_document(paragraphs(&["this "]), cx)
    });
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

#[gpui::test]
fn slash_in_every_kind_of_text_block(cx: &mut TestAppContext) {
    setup(cx);
    let kinds = [
        BlockKind::Paragraph,
        BlockKind::heading(2),
        BlockKind::BulletItem,
        BlockKind::NumberedItem,
        BlockKind::todo(),
        BlockKind::Quote,
    ];
    for kind in kinds {
        let document = Document::from_blocks(vec![
            Block::new(kind.clone()).with_span("some text", InlineAttrs::default()),
        ]);
        let (tools, editor, cx) = open(document, cx);
        let kind_of = |block: usize, cx: &mut gpui::VisualTestContext| {
            editor.read_with(cx, |editor, _| {
                editor.document().blocks()[block].kind().clone()
            })
        };

        // At the very start of a block that already has text.
        caret(&editor, 0, 0, cx);
        cx.simulate_input("/");
        assert_eq!(
            active(&tools, cx),
            Some(ToolKind::Insert),
            "{kind:?}: start"
        );
        cx.simulate_keystrokes("escape");
        assert_eq!(text(&editor, cx), "/some text");
        cx.simulate_keystrokes("backspace");

        // Inside a word and right after one: ordinary text.
        for offset in [2, 9] {
            caret(&editor, 0, offset, cx);
            cx.simulate_input("/");
            assert_eq!(active(&tools, cx), None, "{kind:?}: offset {offset}");
            cx.simulate_keystrokes("backspace");
        }

        // After a space: the menu, and an item that adds a block leaves
        // this one as it is.
        caret(&editor, 0, 9, cx);
        cx.simulate_input(" /");
        assert_eq!(
            active(&tools, cx),
            Some(ToolKind::Insert),
            "{kind:?}: space"
        );
        cx.simulate_input("div");
        assert_eq!(selected(&tools, cx).as_deref(), Some("Divider"));
        cx.simulate_keystrokes("enter");
        assert_eq!(active(&tools, cx), None);
        assert!(kind_of(0, cx).same_type(&kind), "{kind:?} is kept");
        assert_eq!(kind_of(1, cx), BlockKind::Divider);
        assert_eq!(text(&editor, cx).lines().next(), Some("some text "));
        cx.simulate_keystrokes("cmd-z");
        assert_eq!(text(&editor, cx), "some text /div", "one undo step");
        // (What was typed takes one more undo per word.)
        for _ in 0..3 {
            if text(&editor, cx) != "some text" {
                cx.simulate_keystrokes("cmd-z");
            }
        }
        assert_eq!(text(&editor, cx), "some text");

        // An item that turns the block into something else does that to
        // this block only, whatever it was.
        caret(&editor, 0, 0, cx);
        cx.simulate_input("/code");
        assert_eq!(selected(&tools, cx).as_deref(), Some("Code block"));
        cx.simulate_keystrokes("tab");
        assert!(kind_of(0, cx).is_code(), "{kind:?} becomes a code block");
        assert_eq!(text(&editor, cx), "some text");
        // And in the code block the slash is a slash.
        cx.simulate_input(" /");
        assert_eq!(active(&tools, cx), None);
    }
}

#[gpui::test]
fn no_menu_inside_inline_code(cx: &mut TestAppContext) {
    setup(cx);
    let code = InlineAttrs {
        code: true,
        ..InlineAttrs::default()
    };
    let document = Document::from_blocks(vec![
        Block::new(BlockKind::Paragraph)
            .with_span("run ", InlineAttrs::default())
            .with_span("ls -la ", code)
            .with_span(" then", InlineAttrs::default()),
    ]);
    let (tools, editor, cx) = open(document, cx);
    // After the space inside the code: a path is being typed.
    caret(&editor, 0, 11, cx);
    cx.simulate_input("/");
    assert_eq!(text(&editor, cx), "run ls -la / then");
    assert_eq!(active(&tools, cx), None);
    // After the space outside it: the menu.
    caret(&editor, 0, 13, cx);
    cx.simulate_input("/");
    assert_eq!(text(&editor, cx), "run ls -la / /then");
    assert_eq!(active(&tools, cx), Some(ToolKind::Insert));
}

/// What Vim mode does to the editor, without Vim: a key interceptor at the
/// modal priority that takes every key in "normal mode" (with text input
/// switched off) and only `escape` in "insert mode".
#[gpui::test]
fn under_a_modal_layer_the_menu_opens_only_when_the_slash_is_typed(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(paragraphs(&["text", ""]), cx);
    caret(&editor, 1, 0, cx);
    let normal = Rc::new(Cell::new(true));
    let seen = Rc::new(RefCell::new(Vec::<String>::new()));
    let _layer = editor.read_with(cx, |editor, _| {
        let normal = normal.clone();
        let seen = seen.clone();
        editor.intercept_keys(InterceptPriority::MODAL, move |key, _, _| {
            if key.modifiers.platform || key.key.is_empty() {
                return KeyDisposition::Pass;
            }
            seen.borrow_mut().push(key.key.clone());
            if normal.get() {
                KeyDisposition::Consumed
            } else if key.key == "escape" {
                normal.set(true);
                KeyDisposition::Consumed
            } else {
                KeyDisposition::Pass
            }
        })
    });
    let set_normal = |on: bool, cx: &mut gpui::VisualTestContext| {
        normal.set(on);
        editor.update(cx, |editor, cx| editor.set_text_input_enabled(!on, cx));
        cx.run_until_parked();
    };

    // Normal mode: the slash is a command of the layer, nothing is typed,
    // no menu.
    set_normal(true, cx);
    cx.simulate_input("/");
    assert_eq!(*seen.borrow(), ["/"]);
    assert_eq!(text(&editor, cx), "text\n");
    assert_eq!(active(&tools, cx), None);

    // Insert mode: the slash is typed and opens the menu.
    set_normal(false, cx);
    cx.simulate_input("/");
    assert_eq!(text(&editor, cx), "text\n/");
    assert_eq!(active(&tools, cx), Some(ToolKind::Insert));
    // Letters reach the layer (which lets them through) and filter.
    cx.simulate_input("he");
    assert_eq!(selected(&tools, cx).as_deref(), Some("Heading 1"));

    // The menu's keys go to the menu first: the layer never sees them,
    // and escape closes the menu without leaving insert mode.
    seen.borrow_mut().clear();
    cx.simulate_keystrokes("down");
    assert_eq!(selected(&tools, cx).as_deref(), Some("Heading 2"));
    cx.simulate_keystrokes("escape");
    assert_eq!(active(&tools, cx), None);
    assert!(seen.borrow().is_empty(), "{:?}", seen.borrow());
    assert!(!normal.get());
    assert_eq!(text(&editor, cx), "text\n/he");
    // The next escape is the layer's.
    cx.simulate_keystrokes("escape");
    assert!(normal.get());

    // Applying an item in insert mode stays in insert mode.
    set_normal(false, cx);
    cx.simulate_keystrokes("backspace backspace backspace");
    cx.simulate_input("/quo");
    seen.borrow_mut().clear();
    cx.simulate_keystrokes("enter");
    assert!(seen.borrow().is_empty());
    assert!(!normal.get());
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().blocks()[1].kind(), &BlockKind::Quote);
    });
    cx.simulate_input("said");
    assert_eq!(text(&editor, cx), "text\nsaid");

    // The find bar's escape from the text also waits its turn: the layer
    // takes the key in both modes.
    cx.simulate_keystrokes("cmd-f");
    cx.update(|window, cx| editor.read(cx).focus(window));
    cx.simulate_keystrokes("escape");
    assert!(normal.get());
    assert_eq!(active(&tools, cx), Some(ToolKind::Find));
}

#[gpui::test]
fn only_a_typed_slash_opens_the_menu(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(paragraphs(&["a", "/", ""]), cx);
    let edit = |cx: &mut gpui::VisualTestContext, f: &dyn Fn(&mut innerword_core::EditorState)| {
        editor.update(cx, |editor, cx| editor.edit(|state| f(state), cx));
        cx.run_until_parked();
    };

    // A slash key that types nothing, and then an edit that has nothing to
    // do with it while the caret happens to sit after an old slash: the
    // key press is forgotten.
    caret(&editor, 1, 1, cx);
    editor.update(cx, |editor, cx| editor.set_read_only(true, cx));
    cx.simulate_input("/");
    assert_eq!(text(&editor, cx), "a\n/\n");
    editor.update(cx, |editor, cx| editor.set_read_only(false, cx));
    edit(cx, &|state| {
        state.set_block_kind(BlockKind::heading(1));
    });
    assert_eq!(active(&tools, cx), None);

    editor.update(cx, |editor, cx| editor.set_text_input_enabled(false, cx));
    cx.simulate_input("/");
    editor.update(cx, |editor, cx| editor.set_text_input_enabled(true, cx));
    edit(cx, &|state| {
        state.set_alignment(Alignment::Center);
    });
    assert_eq!(text(&editor, cx), "a\n/\n");
    assert_eq!(active(&tools, cx), None);

    // A slash that arrives without being typed: inserted by a command,
    // brought back by undo and redo.
    edit(cx, &|state| {
        state.set_caret(Position::new(2, 0));
        state.insert_text("/");
    });
    assert_eq!(text(&editor, cx), "a\n/\n/");
    assert_eq!(active(&tools, cx), None);
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(active(&tools, cx), None);
    cx.simulate_keystrokes("cmd-shift-z");
    assert_eq!(text(&editor, cx), "a\n/\n/");
    assert_eq!(active(&tools, cx), None);
    cx.simulate_keystrokes("cmd-z");

    // Typed, in the same place: the menu.
    cx.simulate_input("/");
    assert_eq!(text(&editor, cx), "a\n/\n/");
    assert_eq!(active(&tools, cx), Some(ToolKind::Insert));
    // A typed character that is not a slash, landing after an old one,
    // does not reopen it once it was dismissed.
    cx.simulate_keystrokes("escape");
    cx.simulate_input("x");
    cx.simulate_keystrokes("backspace");
    assert_eq!(active(&tools, cx), None);
}
