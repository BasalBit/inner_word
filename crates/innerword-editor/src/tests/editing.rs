//! Editing from the keyboard: every editor action through the real
//! dispatch path (key bindings in the "Editor" context, then the platform
//! input handler).

use std::time::Duration;

use gpui::{ClipboardItem, TestAppContext};
use innerword_core::{
    Alignment, Block, BlockKind, Document, Highlight, InlineAttrs, InlineStyle, Position,
    Selection, TextRange,
};

use super::slice::{open, setup};
use crate::{CompactOptions, EditorConfig};

fn document(blocks: Vec<Block>) -> Document {
    Document::from_blocks(blocks)
}

#[gpui::test]
fn inline_formatting_keys(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(
        document(vec![Block::paragraph("alpha beta gamma")]),
        EditorConfig::full(),
        cx,
    );
    // Select "alpha" and switch everything on.
    cx.simulate_keystrokes("alt-shift-right cmd-b cmd-i cmd-u cmd-shift-x cmd-e cmd-shift-h");
    editor.read_with(cx, |editor, _| {
        let block = &editor.document().blocks()[0];
        let attrs = block.attrs_at(0).unwrap();
        assert!(attrs.bold && attrs.italic && attrs.underline && attrs.strikethrough && attrs.code);
        assert_eq!(attrs.highlight, Some(Highlight::Yellow));
        assert!(block.attrs_at(6).unwrap().is_plain());
    });
    // Toggling again removes; clear formatting removes the rest.
    cx.simulate_keystrokes("cmd-b");
    editor.read_with(cx, |editor, _| {
        assert!(!editor.document().blocks()[0].attrs_at(0).unwrap().bold);
    });
    cx.simulate_keystrokes("cmd-\\");
    editor.read_with(cx, |editor, _| {
        assert!(
            editor.document().blocks()[0]
                .attrs_at(0)
                .unwrap()
                .is_plain()
        );
    });

    // A toggle on a caret applies to what is typed next.
    cx.simulate_keystrokes("cmd-down cmd-i");
    cx.simulate_input("!");
    editor.read_with(cx, |editor, _| {
        let block = &editor.document().blocks()[0];
        assert_eq!(block.text(), "alpha beta gamma!");
        assert!(block.attrs_at(block.len() - 1).unwrap().italic);
    });
}

#[gpui::test]
fn block_kind_alignment_size_and_order_keys(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(
        document(vec![
            Block::paragraph("one"),
            Block::paragraph("two"),
            Block::paragraph("three"),
        ]),
        EditorConfig::full(),
        cx,
    );
    let kind = |cx: &mut gpui::VisualTestContext, index: usize| {
        editor.read_with(cx, |editor, _| {
            editor.document().blocks()[index].kind().clone()
        })
    };

    for (keys, expected) in [
        ("cmd-alt-1", BlockKind::heading(1)),
        ("cmd-alt-6", BlockKind::heading(6)),
        ("cmd-alt-0", BlockKind::Paragraph),
        ("cmd-shift-7", BlockKind::NumberedItem),
        ("cmd-shift-8", BlockKind::BulletItem),
        ("cmd-shift-8", BlockKind::Paragraph),
        ("cmd-shift-9", BlockKind::todo()),
        ("cmd-shift-enter", BlockKind::TodoItem { checked: true }),
        ("cmd-'", BlockKind::Quote),
        ("cmd-shift-'", BlockKind::code()),
        ("cmd-shift-'", BlockKind::Paragraph),
        // What a US keyboard really sends for shift plus a digit or a
        // punctuation key: the shifted character without the shift flag.
        ("cmd-&", BlockKind::NumberedItem),
        ("cmd-*", BlockKind::BulletItem),
        ("cmd-(", BlockKind::todo()),
        ("cmd-\"", BlockKind::code()),
        ("cmd-alt-0", BlockKind::Paragraph),
    ] {
        cx.simulate_keystrokes(keys);
        assert_eq!(kind(cx, 0), expected, "after {keys}");
    }

    // Alignment and text size act on the caret's block.
    for (keys, expected) in [
        ("cmd-shift-e", Alignment::Center),
        ("cmd-shift-r", Alignment::Right),
        ("cmd-shift-j", Alignment::Justify),
        ("cmd-shift-l", Alignment::Left),
    ] {
        cx.simulate_keystrokes(keys);
        editor.read_with(cx, |editor, _| {
            assert_eq!(editor.document().blocks()[0].alignment(), expected);
        });
    }
    cx.simulate_keystrokes("cmd-shift-.");
    let larger = editor.read_with(cx, |editor, _| editor.document().blocks()[0].text_size());
    assert!(larger.is_some_and(|size| size > 17.));
    cx.simulate_keystrokes("cmd-< cmd-shift-,");
    let smaller = editor.read_with(cx, |editor, _| editor.document().blocks()[0].text_size());
    assert!(smaller.is_some_and(|size| size < 17.), "{smaller:?}");
    cx.simulate_keystrokes("cmd->");

    // Indent with tab and cmd-], outdent with shift-tab and cmd-[.
    cx.simulate_keystrokes("cmd-shift-8 tab cmd-]");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().blocks()[0].indent(), 2)
    });
    cx.simulate_keystrokes("shift-tab cmd-[");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().blocks()[0].indent(), 0)
    });

    // Move the block down and back up; the caret travels with it.
    cx.simulate_keystrokes("alt-down");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.plain_text(), "two\none\nthree");
        assert_eq!(editor.selection().head.block, 1);
    });
    cx.simulate_keystrokes("alt-down alt-up alt-up");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.plain_text(), "one\ntwo\nthree");
        assert_eq!(editor.selection().head.block, 0);
    });
}

#[gpui::test]
fn deletion_and_insertion_keys(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(
        document(vec![
            Block::paragraph("one two three"),
            Block::paragraph("four"),
        ]),
        EditorConfig::full(),
        cx,
    );
    let text =
        |cx: &mut gpui::VisualTestContext| editor.read_with(cx, |editor, _| editor.plain_text());

    cx.simulate_keystrokes("cmd-right alt-backspace");
    assert_eq!(text(cx), "one two \nfour");
    cx.simulate_keystrokes("backspace ctrl-h");
    assert_eq!(text(cx), "one tw\nfour");
    cx.simulate_keystrokes("cmd-left delete ctrl-d");
    assert_eq!(text(cx), "e tw\nfour");
    cx.simulate_keystrokes("alt-delete");
    assert_eq!(text(cx), " tw\nfour");
    cx.simulate_keystrokes("right right ctrl-k");
    assert_eq!(text(cx), " t\nfour");
    cx.simulate_keystrokes("cmd-backspace");
    assert_eq!(text(cx), "\nfour");
    // Forward delete in the emptied block removes it and keeps "four".
    cx.simulate_keystrokes("delete");
    assert_eq!(text(cx), "four");
    cx.simulate_keystrokes("right right right cmd-delete");
    assert_eq!(text(cx), "fou");

    // enter splits, shift-enter breaks the line inside the block,
    // cmd-enter inserts a page break.
    cx.simulate_keystrokes("left enter");
    assert_eq!(text(cx), "fo\nu");
    cx.simulate_keystrokes("shift-enter");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().block_count(), 2);
        assert_eq!(editor.document().blocks()[1].text(), "\nu");
    });
    cx.simulate_keystrokes("cmd-enter");
    editor.read_with(cx, |editor, _| {
        assert!(
            editor
                .document()
                .blocks()
                .iter()
                .any(|block| matches!(block.kind(), BlockKind::PageBreak))
        );
    });

    // Select all, then typing replaces everything.
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("x");
    assert_eq!(text(cx), "x");
}

#[gpui::test]
fn undo_groups_break_after_a_typing_pause(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(Document::new(), EditorConfig::full(), cx);
    let text =
        |cx: &mut gpui::VisualTestContext| editor.read_with(cx, |editor, _| editor.plain_text());

    // Typed without a pause: one step.
    cx.simulate_input("abcd");
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(text(cx), "");
    cx.simulate_keystrokes("cmd-shift-z");
    assert_eq!(text(cx), "abcd");

    // A pause in the middle of a word ends the step.
    cx.simulate_input("ef");
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.simulate_input("gh");
    assert_eq!(text(cx), "abcdefgh");
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(text(cx), "abcdef");
    // (Typing after a redo started its own step.)
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(text(cx), "abcd");
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(text(cx), "");
    cx.simulate_keystrokes("cmd-shift-z cmd-shift-z cmd-shift-z");
    assert_eq!(text(cx), "abcdefgh");

    // The pause does not split a composition or anything that is not
    // typing: a block command after the pause is its own step anyway.
    cx.simulate_input("ij");
    cx.simulate_keystrokes("cmd-b");
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(text(cx), "abcdefgh");
}

#[gpui::test]
fn input_rules_and_smart_punctuation_follow_the_config(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(Document::new(), EditorConfig::full(), cx);
    cx.simulate_input("- item");
    cx.simulate_keystrokes("enter enter");
    cx.simulate_input("\"quoted\" -- and **bold** text");
    editor.read_with(cx, |editor, _| {
        let document = editor.document();
        assert_eq!(document.blocks()[0].kind(), &BlockKind::BulletItem);
        assert_eq!(document.blocks()[0].text(), "item");
        let last = document.blocks().last().unwrap();
        assert_eq!(last.text(), "\u{201c}quoted\u{201d} \u{2014} and bold text");
        let bold_at = last.text().find("bold").unwrap();
        assert!(last.attrs_at(bold_at).unwrap().bold);
    });

    // Both switched off: the characters stay as typed.
    editor.update(cx, |editor, cx| {
        editor.update_config(
            |config| {
                config.input_rules = false;
                config.smart_punctuation = false;
            },
            cx,
        );
        editor.set_document(Document::new(), cx);
    });
    cx.simulate_input("- \"x\" **y**");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.plain_text(), "- \"x\" **y**");
        assert_eq!(editor.document().blocks()[0].kind(), &BlockKind::Paragraph);
    });
}

#[gpui::test]
fn clipboard_round_trips_formatting(cx: &mut TestAppContext) {
    setup(cx);
    let bold = InlineAttrs::default().with(InlineStyle::Bold);
    let (editor, cx) = open(
        document(vec![
            Block::with_text(BlockKind::heading(2), "Title"),
            Block::new(BlockKind::Paragraph)
                .with_span("plain ", InlineAttrs::default())
                .with_span("bold", bold),
            Block::paragraph("end"),
        ]),
        EditorConfig::full(),
        cx,
    );

    // Copy the heading and the paragraph: plain text for other apps, the
    // fragment as metadata.
    cx.simulate_keystrokes("shift-down shift-down cmd-c");
    let item = cx.read_from_clipboard().expect("copied");
    assert_eq!(item.text().as_deref(), Some("Title\nplain bold\n"));
    assert!(item.metadata().is_some(), "the rich fragment travels along");

    // Paste at the end: kinds and attributes survive.
    cx.simulate_keystrokes("cmd-down enter cmd-v");
    editor.read_with(cx, |editor, _| {
        let blocks = editor.document().blocks();
        assert_eq!(
            editor.plain_text(),
            "Title\nplain bold\nend\nTitle\nplain bold\n"
        );
        assert_eq!(blocks[3].kind(), &BlockKind::heading(2));
        assert!(blocks[4].attrs_at(6).unwrap().bold);
        assert!(!blocks[4].attrs_at(0).unwrap().bold);
    });

    // Paste as plain text: same text, no formatting.
    cx.simulate_keystrokes("cmd-shift-v");
    editor.read_with(cx, |editor, _| {
        let blocks = editor.document().blocks();
        let pasted = blocks
            .iter()
            .rposition(|block| block.text() == "plain bold");
        let block = &blocks[pasted.unwrap()];
        assert!(!block.attrs_at(6).unwrap().bold);
        assert!(
            blocks
                .iter()
                .filter(|block| block.text() == "Title")
                .count()
                == 3
        );
        assert_eq!(
            blocks
                .iter()
                .filter(|block| block.kind().heading_level().is_some())
                .count(),
            2
        );
    });

    // Cut removes the selection and fills the clipboard; undo restores.
    cx.simulate_keystrokes("cmd-up alt-shift-right cmd-x");
    assert_eq!(
        cx.read_from_clipboard().unwrap().text().as_deref(),
        Some("Title")
    );
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().blocks()[0].text(), "")
    });
    cx.simulate_keystrokes("cmd-v");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().blocks()[0].text(), "Title");
        assert_eq!(editor.document().blocks()[0].kind(), &BlockKind::heading(2));
    });

    // Text from another app has no fragment: lines become paragraphs.
    cx.write_to_clipboard(ClipboardItem::new_string("from\nelsewhere".into()));
    cx.simulate_keystrokes("cmd-a cmd-v");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.plain_text(), "from\nelsewhere");
        assert_eq!(editor.document().block_count(), 2);
    });

    // Copy with a collapsed selection leaves the clipboard alone.
    cx.simulate_keystrokes("cmd-c");
    assert_eq!(
        cx.read_from_clipboard().unwrap().text().as_deref(),
        Some("from\nelsewhere")
    );
}

#[gpui::test]
fn tab_in_a_code_block_indents_text(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(
        document(vec![Block::with_text(
            BlockKind::code(),
            "fn a() {\nb();\n}",
        )]),
        EditorConfig::full(),
        cx,
    );
    let code = |cx: &mut gpui::VisualTestContext| {
        editor.read_with(cx, |editor, _| {
            editor.document().blocks()[0].text().to_string()
        })
    };

    // A caret moves to the next stop.
    cx.simulate_keystrokes("down tab");
    assert_eq!(code(cx), "fn a() {\n    b();\n}");
    cx.simulate_input("x");
    cx.simulate_keystrokes("tab");
    assert_eq!(code(cx), "fn a() {\n    x   b();\n}");
    editor.read_with(cx, |editor, _| {
        assert_eq!(
            editor.document().blocks()[0].indent(),
            0,
            "the block is not indented"
        );
    });
    cx.simulate_keystrokes("cmd-z cmd-z cmd-z");
    assert_eq!(code(cx), "fn a() {\nb();\n}");

    // A selection indents and outdents the lines it touches.
    editor.update(cx, |editor, cx| {
        editor.edit(
            |state| state.select_range(TextRange::in_block(0, 2, 11)),
            cx,
        );
    });
    cx.simulate_keystrokes("tab");
    assert_eq!(code(cx), "    fn a() {\n    b();\n}");
    editor.read_with(cx, |editor, _| {
        assert_eq!(
            editor.selection(),
            Selection::new(Position::new(0, 6), Position::new(0, 19))
        );
    });
    cx.simulate_keystrokes("shift-tab");
    assert_eq!(code(cx), "fn a() {\nb();\n}");
    editor.read_with(cx, |editor, _| {
        assert_eq!(
            editor.selection(),
            Selection::new(Position::new(0, 2), Position::new(0, 11))
        );
    });
    // One undo step per indent.
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(code(cx), "    fn a() {\n    b();\n}");
}

#[gpui::test]
fn compact_editor_is_plain_text(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(
        Document::new(),
        EditorConfig::compact(CompactOptions::default().submit_on_enter(false)),
        cx,
    );
    cx.simulate_input("note");
    // Formatting, block and layout commands do nothing here.
    cx.simulate_keystrokes("cmd-a cmd-b cmd-alt-1 cmd-shift-8 cmd-shift-e cmd-enter tab");
    editor.read_with(cx, |editor, _| {
        let block = &editor.document().blocks()[0];
        assert_eq!(editor.plain_text(), "note");
        assert_eq!(block.kind(), &BlockKind::Paragraph);
        assert_eq!(block.alignment(), Alignment::Left);
        assert_eq!(block.indent(), 0);
        assert!(block.attrs_at(0).unwrap().is_plain());
    });
    // Without submit-on-enter, enter starts a new paragraph.
    cx.simulate_keystrokes("cmd-down enter");
    cx.simulate_input("more");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.plain_text(), "note\nmore")
    });

    // Rich fragments are pasted as their text.
    let fragment =
        Document::from_blocks(vec![Block::with_text(BlockKind::heading(1), "Big")]).to_fragment();
    cx.write_to_clipboard(ClipboardItem::new_string_with_metadata(
        fragment.to_plain_text(),
        fragment.to_json(),
    ));
    cx.simulate_keystrokes("cmd-a cmd-v");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.plain_text(), "Big");
        assert_eq!(editor.document().blocks()[0].kind(), &BlockKind::Paragraph);
    });
}
