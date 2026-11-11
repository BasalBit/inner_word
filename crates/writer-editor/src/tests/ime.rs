//! Platform text input: marked text (IME composition, dead keys), UTF-16
//! ranges, candidate window bounds. The platform's calls are made directly
//! on the `EntityInputHandler` implementation, as AppKit would.

use gpui::{EntityInputHandler, TestAppContext, px};
use writer_core::{Block, Document, Position, Selection, TextRange};

use super::slice::{open, setup};
use crate::EditorConfig;

#[gpui::test]
fn dead_key_composition_is_marked_then_replaced(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(
        Document::from_blocks(vec![Block::paragraph("caf")]),
        EditorConfig::full(),
        cx,
    );
    cx.simulate_keystrokes("cmd-down");

    // option-e: AppKit marks an acute accent.
    editor.update_in(cx, |editor, window, cx| {
        assert_eq!(editor.marked_text_range(window, cx), None);
        editor.replace_and_mark_text_in_range(None, "\u{b4}", Some(1..1), window, cx);
        assert_eq!(editor.plain_text(), "caf\u{b4}");
        assert_eq!(editor.marked_range(), Some(TextRange::in_block(0, 3, 5)));
        // UTF-16: one unit after "caf".
        assert_eq!(editor.marked_text_range(window, cx), Some(3..4));
        let selected = editor.selected_text_range(false, window, cx).unwrap();
        assert_eq!(selected.range, 4..4);
    });
    cx.run_until_parked();

    // Then "e": the marked text is replaced by the composed character.
    editor.update_in(cx, |editor, window, cx| {
        editor.replace_text_in_range(None, "\u{e9}", window, cx);
        assert_eq!(editor.plain_text(), "caf\u{e9}");
        assert_eq!(editor.marked_range(), None);
        assert_eq!(editor.selection(), Selection::caret(Position::new(0, 5)));
    });
    // The whole composition is one undo step.
    cx.simulate_keystrokes("cmd-z");
    editor.read_with(cx, |editor, _| assert_eq!(editor.plain_text(), "caf"));
    cx.simulate_keystrokes("cmd-shift-z");
    editor.read_with(cx, |editor, _| assert_eq!(editor.plain_text(), "caf\u{e9}"));
}

#[gpui::test]
fn multi_step_composition_with_a_selected_clause(cx: &mut TestAppContext) {
    setup(cx);
    // An emoji before the caret: two UTF-16 units, four bytes.
    let (editor, cx) = open(
        Document::from_blocks(vec![
            Block::paragraph("\u{1f600}a"),
            Block::paragraph("next"),
        ]),
        EditorConfig::full(),
        cx,
    );
    cx.simulate_keystrokes("cmd-right");

    editor.update_in(cx, |editor, window, cx| {
        // Each step replaces the marked text ("k", then kana, then more).
        editor.replace_and_mark_text_in_range(None, "k", Some(1..1), window, cx);
        editor.replace_and_mark_text_in_range(None, "\u{304b}", Some(1..1), window, cx);
        editor.replace_and_mark_text_in_range(
            None,
            "\u{304b}\u{3093}\u{3058}",
            Some(1..3),
            window,
            cx,
        );
        assert_eq!(
            editor.plain_text(),
            "\u{1f600}a\u{304b}\u{3093}\u{3058}\nnext"
        );
        // Marked: three kana after 3 UTF-16 units (emoji = 2, "a" = 1).
        assert_eq!(editor.marked_text_range(window, cx), Some(3..6));
        assert_eq!(editor.marked_range(), Some(TextRange::in_block(0, 5, 14)));
        // The clause the input method selected is the editor's selection.
        let selected = editor.selected_text_range(false, window, cx).unwrap();
        assert_eq!(selected.range, 4..6);
        assert_eq!(
            editor.selection(),
            Selection::new(Position::new(0, 8), Position::new(0, 14))
        );

        // The platform reads text back in UTF-16 ranges.
        let mut adjusted = None;
        assert_eq!(
            editor
                .text_for_range(0..2, &mut adjusted, window, cx)
                .as_deref(),
            Some("\u{1f600}")
        );
        assert_eq!(adjusted, Some(0..2));
        assert_eq!(
            editor
                .text_for_range(3..5, &mut adjusted, window, cx)
                .as_deref(),
            Some("\u{304b}\u{3093}")
        );

        // The candidate window goes under the marked text.
        let caret = editor.position_bounds(Position::new(0, 5)).unwrap();
        let bounds = editor
            .bounds_for_range(3..6, caret, window, cx)
            .expect("bounds for the marked range");
        assert_eq!(bounds.left(), caret.left());
        assert_eq!(bounds.top(), caret.top());
        assert!(bounds.size.width > px(20.));
        let empty = editor.bounds_for_range(6..6, caret, window, cx).unwrap();
        assert!((empty.left() - bounds.right()).abs() < px(0.01));

        // Converting: the marked range is replaced and stays marked.
        editor.replace_and_mark_text_in_range(None, "\u{6f22}\u{5b57}", None, window, cx);
        assert_eq!(editor.marked_text_range(window, cx), Some(3..5));
        // Committing with an explicit range (what insertText passes).
        editor.replace_text_in_range(Some(3..5), "\u{6f22}\u{5b57}", window, cx);
        assert_eq!(editor.plain_text(), "\u{1f600}a\u{6f22}\u{5b57}\nnext");
        assert_eq!(editor.marked_range(), None);
        assert_eq!(editor.selection(), Selection::caret(Position::new(0, 11)));
    });
    cx.simulate_keystrokes("cmd-z");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.plain_text(), "\u{1f600}a\nnext")
    });

    // A cancelled composition leaves nothing behind.
    editor.update_in(cx, |editor, window, cx| {
        editor.replace_and_mark_text_in_range(None, "x", None, window, cx);
        assert!(editor.marked_range().is_some());
        editor.replace_and_mark_text_in_range(None, "", None, window, cx);
        assert_eq!(editor.marked_range(), None);
        assert_eq!(editor.plain_text(), "\u{1f600}a\nnext");
    });

    // Composing, then unmark: the text stays, the marking goes.
    editor.update_in(cx, |editor, window, cx| {
        editor.replace_and_mark_text_in_range(None, "y", None, window, cx);
        editor.unmark_text(window, cx);
        assert_eq!(editor.marked_range(), None);
        assert_eq!(editor.plain_text(), "\u{1f600}ay\nnext");
    });
    // A frame with marked text paints (the underline) without trouble.
    editor.update_in(cx, |editor, window, cx| {
        editor.replace_and_mark_text_in_range(None, "zz", Some(0..1), window, cx);
    });
    cx.run_until_parked();
    // Moving the caret with the mouse or an outside edit ends it.
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.insert_text("!"), cx);
        assert_eq!(editor.marked_range(), None);
    });
}

#[gpui::test]
fn character_index_for_point_and_refusal(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(
        Document::from_blocks(vec![Block::paragraph("\u{1f600}abc")]),
        EditorConfig::full(),
        cx,
    );
    editor.update_in(cx, |editor, window, cx| {
        // The point of the caret before "b": UTF-16 index 3.
        let bounds = editor.position_bounds(Position::new(0, 5)).unwrap();
        assert_eq!(
            editor.character_index_for_point(bounds.center(), window, cx),
            Some(3)
        );

        // Read-only and disabled text input refuse every platform edit.
        editor.set_read_only(true, cx);
        editor.replace_text_in_range(None, "x", window, cx);
        editor.replace_and_mark_text_in_range(None, "y", None, window, cx);
        assert_eq!(editor.plain_text(), "\u{1f600}abc");
        editor.set_read_only(false, cx);
        editor.set_text_input_enabled(false, cx);
        editor.replace_text_in_range(None, "x", window, cx);
        editor.replace_and_mark_text_in_range(None, "y", None, window, cx);
        assert_eq!(editor.plain_text(), "\u{1f600}abc");
        assert_eq!(editor.marked_range(), None);

        // A range from the platform replaces exactly that text (the accent
        // menu after a held key).
        editor.set_text_input_enabled(true, cx);
        editor.replace_text_in_range(Some(2..3), "\u{e0}", window, cx);
        assert_eq!(editor.plain_text(), "\u{1f600}\u{e0}bc");
    });
}
