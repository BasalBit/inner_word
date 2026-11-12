//! The capture: what was selected, and where it is after edits.

use gpui::TestAppContext;
use writer_assistant_quick::capture::{Capture, QUOTE_CHARS, quote};
use writer_core::{Block, Fragment, Position, TextRange};

use crate::support::*;

fn capture_of(document: &writer_core::Document, range: TextRange) -> Capture {
    Capture::new(document, range)
}

#[test]
fn a_selection_is_captured_exactly() {
    let document = paragraphs(&["The quick brown fox", "jumps over", "the lazy dog"]);
    let within = capture_of(&document, TextRange::in_block(0, 4, 9));
    assert!(within.has_selection());
    assert_eq!(within.text(), "quick");
    assert_eq!(within.quote().as_deref(), Some("quick"));

    // Over several blocks: joined by a line break, nothing else.
    let across = capture_of(
        &document,
        TextRange::new(Position::new(0, 10), Position::new(2, 8)),
    );
    assert_eq!(across.text(), "brown fox\njumps over\nthe lazy");
    assert_eq!(
        across.quote().as_deref(),
        Some("brown fox jumps over the lazy")
    );

    // A reversed range is the same selection.
    let reversed = capture_of(
        &document,
        TextRange {
            start: Position::new(0, 9),
            end: Position::new(0, 4),
        },
    );
    assert_eq!(reversed.text(), "quick");
}

#[test]
fn a_caret_captures_nothing() {
    let document = paragraphs(&["The quick brown fox"]);
    let caret = capture_of(&document, TextRange::empty(Position::new(0, 4)));
    assert!(!caret.has_selection());
    assert_eq!(caret.text(), "");
    assert_eq!(caret.quote(), None);
    assert!(
        !caret.is_intact(&document),
        "a caret has no text to replace"
    );
    assert_eq!(caret.end_block_index(&document), Some(0));
}

#[test]
fn a_long_selection_is_quoted_short() {
    let long = "word ".repeat(100);
    let quoted = quote(&long, QUOTE_CHARS);
    assert!(quoted.ends_with('…'));
    assert!(quoted.chars().count() <= QUOTE_CHARS + 1);
    assert!(!quoted.contains("  "));
    assert!(!quoted.contains(" …"), "no space before the ellipsis");

    assert_eq!(quote("  a\n\n b\tc  ", 20), "a b c");
    assert_eq!(quote("abcdef", 3), "abc…");
    assert_eq!(quote("abc", 3), "abc");
    assert_eq!(quote("ab cd", 2), "ab…");
    assert_eq!(quote("ab cd", 3), "ab…");
    assert_eq!(quote("äöü ß", 3), "äöü…");
    assert_eq!(quote("", 10), "");
}

#[gpui::test]
fn a_capture_follows_its_blocks(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open_editor(paragraphs(&["First.", "The quick brown fox", "Last."]), cx);
    let capture = editor.read_with(cx, |editor, _| {
        Capture::new(editor.document(), TextRange::in_block(1, 4, 9))
    });

    // A paragraph inserted before it moves the block, not the text.
    editor.update(cx, |editor, cx| {
        editor.edit(
            |state| state.insert_blocks_at(0, &Fragment::new(vec![Block::paragraph("New first.")])),
            cx,
        )
    });
    editor.read_with(cx, |editor, _| {
        let document = editor.document();
        assert_eq!(
            capture.resolve(document),
            Some(TextRange::in_block(2, 4, 9))
        );
        assert!(capture.is_intact(document));
        assert_eq!(capture.end_block_index(document), Some(2));
    });

    // An edit after it in the same block leaves it alone.
    editor.update(cx, |editor, cx| {
        editor.edit(
            |state| {
                state.set_caret(Position::new(2, 19));
                state.insert_text(" jumps")
            },
            cx,
        )
    });
    editor.read_with(cx, |editor, _| {
        assert!(capture.is_intact(editor.document()))
    });

    // An edit inside it does not.
    editor.update(cx, |editor, cx| {
        editor.edit(
            |state| {
                state.set_caret(Position::new(2, 6));
                state.insert_text("x")
            },
            cx,
        )
    });
    editor.read_with(cx, |editor, _| {
        let document = editor.document();
        assert!(!capture.is_intact(document));
        assert_eq!(capture.intact_range(document), None);
        // Where it was is still known.
        assert_eq!(capture.end_block_index(document), Some(2));
    });
}

#[gpui::test]
fn a_capture_whose_block_is_gone_resolves_to_nothing(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open_editor(paragraphs(&["First.", "The quick brown fox", "Last."]), cx);
    let capture = editor.read_with(cx, |editor, _| {
        Capture::new(editor.document(), TextRange::in_block(1, 4, 9))
    });
    editor.update(cx, |editor, cx| {
        editor.edit(
            |state| {
                state.select_blocks(1..2);
                state.delete_blocks()
            },
            cx,
        )
    });
    editor.read_with(cx, |editor, _| {
        let document = editor.document();
        assert_eq!(document.plain_text(), "First.\nLast.");
        assert_eq!(capture.resolve(document), None);
        assert!(!capture.is_intact(document));
        assert_eq!(capture.end_block_index(document), None);
    });
}

#[gpui::test]
fn text_before_the_capture_in_its_block_spoils_it(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open_editor(paragraphs(&["The quick brown fox"]), cx);
    let capture = editor.read_with(cx, |editor, _| {
        Capture::new(editor.document(), TextRange::in_block(0, 4, 9))
    });
    editor.update(cx, |editor, cx| {
        editor.edit(
            |state| {
                state.set_caret(Position::new(0, 0));
                state.insert_text("So: ")
            },
            cx,
        )
    });
    // The offsets no longer hold "quick": the capture says so instead of
    // pointing at other words.
    editor.read_with(cx, |editor, _| {
        assert!(!capture.is_intact(editor.document()))
    });
}
