//! The seams the Vim glue builds on beyond the general ones: layout answers
//! while the state is borrowed ([`crate::Editor::edit_with_rows`]), the
//! report of typed text ([`EditorEvent::TextInput`]) and the caret override
//! ([`crate::Editor::set_caret_override`]).

use std::{cell::RefCell, rc::Rc, time::Duration};

use gpui::{Entity, EntityInputHandler, TestAppContext, VisualTestContext, px};
use writer_core::{Block, Document, Position, Selection, TextRange};

use super::slice::{open, setup};
use crate::{Autoscroll, CaretStyle, Editor, EditorConfig, EditorEvent, VisualRow};

fn paragraphs(count: usize) -> Document {
    Document::from_blocks(
        (0..count)
            .map(|ix| Block::paragraph(&format!("Paragraph number {ix}")))
            .collect(),
    )
}

/// One paragraph long enough to wrap in the 700px column, then a short one.
fn wrapping_document() -> Document {
    let long = "word ".repeat(60);
    Document::from_blocks(vec![
        Block::paragraph(long.trim_end()),
        Block::paragraph("short"),
    ])
}

/// Collects the editor's events; the returned closure hands out what
/// arrived since it was last called.
fn record(
    editor: &Entity<Editor>,
    cx: &mut VisualTestContext,
) -> impl Fn(&mut VisualTestContext) -> Vec<EditorEvent> + use<> {
    let events = Rc::new(RefCell::new(Vec::new()));
    cx.update(|_, cx| {
        let events = events.clone();
        cx.subscribe(editor, move |_, event: &EditorEvent, _| {
            events.borrow_mut().push(event.clone());
        })
        .detach();
    });
    move |cx: &mut VisualTestContext| {
        cx.run_until_parked();
        std::mem::take(&mut *events.borrow_mut())
    }
}

fn typed(text: &str) -> EditorEvent {
    EditorEvent::TextInput {
        text: text.to_string(),
        replaced: None,
    }
}

// ---------------------------------------------------------------------------
// edit_with_rows
// ---------------------------------------------------------------------------

#[gpui::test]
fn edit_with_rows_answers_while_the_state_is_borrowed(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(wrapping_document(), EditorConfig::full(), cx);
    let take = record(&editor, cx);
    let from = Position::new(0, 7);

    // What the editor itself answers.
    let (row, x, step, visible, per_page, words) = editor.read_with(cx, |editor, _| {
        (
            editor.visual_row(Position::ZERO).unwrap(),
            editor.column_x(from).unwrap(),
            editor.move_by_visual_rows(from, 2, None).unwrap(),
            editor.visible_rows().unwrap(),
            editor.rows_per_page().unwrap(),
            editor.stats().words,
        )
    });
    assert!(row.end.offset < 299, "the long paragraph wraps");
    assert_eq!(step.rows_moved, 2);
    assert!(take(cx).is_empty());

    let result = editor.update(cx, |editor, cx| {
        editor.edit_with_rows(
            Autoscroll::Fit,
            |state, rows| {
                // The same answers, with the state mutably borrowed.
                assert_eq!(rows.visual_row(state, Position::ZERO), Some(row));
                assert_eq!(rows.column_x(state, from), Some(x));
                assert_eq!(rows.move_by_visual_rows(state, from, 2, None), Some(step));
                assert_eq!(rows.visible_rows(state), Some(visible.clone()));
                assert_eq!(rows.rows_per_page(), Some(per_page));

                // Only the lent state is answered for.
                let copy = state.clone();
                assert_eq!(rows.visual_row(&copy, Position::ZERO), None);
                assert_eq!(rows.column_x(&copy, from), None);
                assert_eq!(rows.move_by_visual_rows(&copy, from, 2, None), None);
                assert_eq!(rows.visible_rows(&copy), None);

                // Vim `2j`, then a question about where the caret now is.
                state.set_caret(step.position);
                let landed = rows.visual_row(state, step.position).unwrap();
                assert!(landed.start.offset > row.start.offset);
                let landed_x = rows.column_x(state, step.position).unwrap();
                assert!((landed_x - step.goal_x).abs() < px(0.01));

                // An edit that adds a block before the long paragraph, and
                // questions about the document as it is now.
                state.set_caret(Position::ZERO);
                state.insert_text("New");
                state.insert_paragraph_break();
                assert_eq!(state.document().block_count(), 3);
                assert_eq!(
                    rows.visual_row(state, Position::ZERO),
                    Some(VisualRow {
                        start: Position::ZERO,
                        end: Position::new(0, 3),
                    })
                );
                assert_eq!(
                    rows.visual_row(state, Position::new(1, 0)),
                    Some(VisualRow {
                        start: Position::new(1, 0),
                        end: Position::new(1, row.end.offset),
                    })
                );
                let down = rows
                    .move_by_visual_rows(state, Position::new(0, 2), 2, None)
                    .unwrap();
                assert_eq!(down.rows_moved, 2);
                assert_eq!(down.position.block, 1);
                assert_eq!(down.position.offset, row.end.offset + 2);
                state.set_caret(down.position);
                // And one more change after the question.
                state.insert_text("x");
                42
            },
            cx,
        )
    });
    assert_eq!(result, 42);

    // One batch of events for the whole call.
    let seen = take(cx);
    assert!(
        matches!(seen[0], EditorEvent::Edited { change } if change.blocks.is_some()),
        "{seen:?}"
    );
    assert_eq!(seen[1], EditorEvent::SelectionChanged);
    assert_eq!(seen.len(), 2);

    editor.read_with(cx, |editor, _| {
        // Layout and statistics describe the result.
        assert_eq!(editor.document().block_count(), 3);
        assert_eq!(editor.stats().words, words + 1);
        assert_eq!(
            editor.selection(),
            Selection::caret(Position::new(1, row.end.offset + 3))
        );
        assert_eq!(
            editor.visual_row(Position::ZERO).unwrap().end,
            Position::new(0, 3)
        );
        assert_eq!(
            editor.visual_row(Position::new(2, 0)).unwrap().end.offset,
            5
        );
        assert!(editor.caret_bounds().is_some());
    });
    // Undo takes the edits back as with `edit`.
    cx.simulate_keystrokes("cmd-z cmd-z cmd-z");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().block_count(), 2);
        assert_eq!(editor.visual_row(Position::ZERO), Some(row));
    });
}

#[gpui::test]
fn edit_with_rows_scrolls_and_keeps_the_view_like_edit_with(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(paragraphs(300), EditorConfig::full(), cx);
    let top_text = |editor: &Editor| {
        let top = editor.visible_range().unwrap().start;
        editor.document().blocks()[top.block].text().to_string()
    };

    // Autoscroll follows the caret the closure leaves behind.
    editor.update(cx, |editor, cx| {
        editor.edit_with_rows(
            Autoscroll::Fit,
            |state, rows| {
                let last = rows.visible_rows(state).unwrap().last().unwrap().start;
                let step = rows.move_by_visual_rows(state, last, 150, None).unwrap();
                assert_eq!(step.rows_moved, 150);
                state.set_caret(step.position);
            },
            cx,
        )
    });
    cx.run_until_parked();
    let (offset, top) = editor.read_with(cx, |editor, _| {
        let head = editor.selection().head;
        let visible = editor.visible_range().unwrap();
        assert!(visible.start <= head && head <= visible.end);
        assert!(editor.scroll_offset() > px(0.));
        (editor.scroll_offset(), top_text(editor))
    });

    // Blocks added above the viewport, with a layout question between the
    // edits: what is on screen stays on screen.
    editor.update(cx, |editor, cx| {
        editor.edit_with_rows(
            Autoscroll::None,
            |state, rows| {
                let head = state.selection().head;
                state.set_caret(Position::ZERO);
                state.insert_paragraph_break();
                assert_eq!(
                    rows.visual_row(state, Position::new(1, 0)).unwrap().end,
                    Position::new(1, 18)
                );
                state.insert_paragraph_break();
                assert!(rows.visible_rows(state).is_some());
                state.insert_paragraph_break();
                state.set_caret(Position::new(head.block + 3, head.offset));
            },
            cx,
        )
    });
    cx.run_until_parked();
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().block_count(), 303);
        assert_eq!(top_text(editor), top);
        assert!(editor.scroll_offset() > offset);
    });
}

#[gpui::test]
fn edit_with_rows_before_the_first_layout(cx: &mut TestAppContext) {
    setup(cx);
    // Asked inside the constructor's window update: nothing was laid out.
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let mut editor = Editor::new(paragraphs(3), EditorConfig::full(), window, cx);
        editor.edit_with_rows(
            Autoscroll::Fit,
            |state, rows| {
                assert_eq!(rows.visual_row(state, Position::ZERO), None);
                assert_eq!(rows.column_x(state, Position::ZERO), None);
                assert_eq!(
                    rows.move_by_visual_rows(state, Position::ZERO, 1, None),
                    None
                );
                assert_eq!(rows.visible_rows(state), None);
                assert_eq!(rows.rows_per_page(), None);
                state.set_caret(Position::new(2, 0));
            },
            cx,
        );
        editor
    });
    cx.run_until_parked();
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.selection(), Selection::caret(Position::new(2, 0)));
        assert!(editor.visual_row(Position::ZERO).is_some());
    });
}

// ---------------------------------------------------------------------------
// EditorEvent::TextInput
// ---------------------------------------------------------------------------

#[gpui::test]
fn text_input_reports_what_was_typed(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(Document::new(), EditorConfig::full(), cx);
    let take = record(&editor, cx);

    // A typed character: announced ahead of its Edited.
    cx.simulate_input("a");
    let seen = take(cx);
    assert_eq!(seen[0], typed("a"), "{seen:?}");
    assert!(matches!(seen[1], EditorEvent::Edited { change } if change.blocks.is_some()));
    assert_eq!(seen[2], EditorEvent::SelectionChanged);
    assert_eq!(seen.len(), 3);

    // The text is reported as delivered, before smart punctuation.
    assert!(editor.read_with(cx, |editor, _| editor.config().smart_punctuation));
    cx.simulate_input(" ");
    take(cx);
    cx.simulate_input("\"");
    let seen = take(cx);
    assert_eq!(seen[0], typed("\""));
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.plain_text(), "a \u{201c}");
    });

    // Typing over a selection is plain typing.
    cx.simulate_keystrokes("shift-left");
    take(cx);
    cx.simulate_input("b");
    let seen = take(cx);
    assert_eq!(seen[0], typed("b"));
    editor.read_with(cx, |editor, _| assert_eq!(editor.plain_text(), "a b"));

    // Text that arrives any other way is not text input.
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.insert_text("c"), cx)
    });
    cx.simulate_keystrokes("enter");
    cx.simulate_keystrokes("backspace");
    let seen = take(cx);
    assert!(!seen.is_empty());
    assert!(
        !seen
            .iter()
            .any(|event| matches!(event, EditorEvent::TextInput { .. })),
        "{seen:?}"
    );

    // Refused input reports nothing and changes nothing.
    editor.update_in(cx, |editor, window, cx| {
        editor.set_text_input_enabled(false, cx);
        editor.replace_text_in_range(None, "x", window, cx);
        editor.set_text_input_enabled(true, cx);
        editor.set_read_only(true, cx);
        editor.replace_text_in_range(None, "y", window, cx);
        editor.set_read_only(false, cx);
        assert_eq!(editor.plain_text(), "a bc");
    });
    assert!(take(cx).is_empty());

    // Several characters in one call are one report.
    editor.update_in(cx, |editor, window, cx| {
        editor.replace_text_in_range(None, "def", window, cx);
    });
    let seen = take(cx);
    assert_eq!(seen[0], typed("def"));
    assert_eq!(seen.len(), 3);
}

#[gpui::test]
fn text_input_reports_a_committed_composition_once(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(
        Document::from_blocks(vec![Block::paragraph("caf")]),
        EditorConfig::full(),
        cx,
    );
    cx.simulate_keystrokes("cmd-down");
    let take = record(&editor, cx);
    let is_input = |event: &EditorEvent| matches!(event, EditorEvent::TextInput { .. });

    // Marked text and its updates are not reported.
    editor.update_in(cx, |editor, window, cx| {
        editor.replace_and_mark_text_in_range(None, "\u{b4}", Some(1..1), window, cx);
    });
    let seen = take(cx);
    assert!(!seen.is_empty() && !seen.iter().any(is_input), "{seen:?}");
    editor.update_in(cx, |editor, window, cx| {
        editor.replace_and_mark_text_in_range(None, "\u{b4}\u{b4}", Some(2..2), window, cx);
    });
    assert!(!take(cx).iter().any(is_input));

    // The commit is, once, with the committed text.
    editor.update_in(cx, |editor, window, cx| {
        editor.replace_text_in_range(None, "\u{e9}", window, cx);
        assert_eq!(editor.plain_text(), "caf\u{e9}");
    });
    let seen = take(cx);
    assert_eq!(seen[0], typed("\u{e9}"), "{seen:?}");
    assert!(matches!(seen[1], EditorEvent::Edited { .. }));
    assert_eq!(seen.iter().filter(|event| is_input(event)).count(), 1);

    // A composition that is cancelled typed nothing.
    editor.update_in(cx, |editor, window, cx| {
        editor.replace_and_mark_text_in_range(None, "\u{b4}", Some(1..1), window, cx);
        editor.replace_text_in_range(None, "", window, cx);
        assert_eq!(editor.plain_text(), "caf\u{e9}");
        assert_eq!(editor.marked_range(), None);
    });
    assert!(!take(cx).iter().any(is_input));

    // A commit with the marked range named explicitly (as AppKit does).
    editor.update_in(cx, |editor, window, cx| {
        editor.replace_and_mark_text_in_range(None, "n", Some(1..1), window, cx);
        let marked = editor.marked_text_range(window, cx).unwrap();
        assert_eq!(marked, 4..5);
        editor.replace_text_in_range(Some(marked), "\u{306b}", window, cx);
        assert_eq!(editor.plain_text(), "caf\u{e9}\u{306b}");
    });
    let seen = take(cx);
    assert_eq!(
        seen.iter()
            .filter(|event| is_input(event))
            .collect::<Vec<_>>(),
        [&typed("\u{306b}")]
    );

    // The platform replacing committed text (the accent menu on a held
    // key): the replaced range comes with the report.
    cx.simulate_input("e");
    take(cx);
    editor.update_in(cx, |editor, window, cx| {
        assert_eq!(editor.plain_text(), "caf\u{e9}\u{306b}e");
        // UTF-16: c a f é に e
        editor.replace_text_in_range(Some(5..6), "\u{e8}", window, cx);
        assert_eq!(editor.plain_text(), "caf\u{e9}\u{306b}\u{e8}");
    });
    let seen = take(cx);
    assert_eq!(
        seen[0],
        EditorEvent::TextInput {
            text: "\u{e8}".to_string(),
            // Bytes: c a f (3) é (2) に (3) e (1).
            replaced: Some(TextRange::in_block(0, 8, 9)),
        },
        "{seen:?}"
    );
    assert!(matches!(seen[1], EditorEvent::Edited { .. }));
}

// ---------------------------------------------------------------------------
// The caret override
// ---------------------------------------------------------------------------

#[gpui::test]
fn caret_override_draws_the_caret_away_from_the_head(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(paragraphs(3), EditorConfig::full(), cx);
    let painted = |cx: &mut VisualTestContext| {
        cx.run_until_parked();
        editor.read_with(cx, |editor, _| editor.caret_painted)
    };
    editor.update(cx, |editor, cx| {
        editor.set_draw_as_active(false, cx);
        // A Visual selection over "Paragraph": the head is past the "h".
        editor.edit(
            |state| state.set_selection(Selection::new(Position::ZERO, Position::new(0, 9))),
            cx,
        );
        assert_eq!(editor.caret_override(), None);
        assert_eq!(editor.caret_position(), Position::new(0, 9));
    });
    // A bar caret is not drawn with a selection.
    assert!(!painted(cx));

    let cursor = Position::new(0, 8);
    editor.update(cx, |editor, cx| editor.set_caret_override(Some(cursor), cx));
    // With an override it is, in any style.
    assert!(painted(cx));
    editor.update(cx, |editor, cx| {
        editor.set_caret_style(CaretStyle::Block, cx)
    });
    assert!(painted(cx));
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.caret_override(), Some(cursor));
        assert_eq!(editor.caret_position(), cursor);
        // The selection is untouched.
        assert_eq!(
            editor.selection(),
            Selection::new(Position::ZERO, Position::new(0, 9))
        );
        // The block caret covers the last selected character.
        let caret = editor.caret_bounds().unwrap();
        let cell = editor
            .range_bounds(TextRange::in_block(0, 8, 9))
            .unwrap()
            .remove(0);
        assert_eq!(caret, cell);
        let head = editor.position_bounds(Position::new(0, 9)).unwrap();
        assert_eq!(caret.right(), head.left());
    });

    // It blinks, and moving it shows it at once.
    cx.executor().advance_clock(Duration::from_millis(600));
    assert!(!painted(cx));
    editor.update(cx, |editor, cx| {
        editor.set_caret_override(Some(Position::new(2, 3)), cx)
    });
    assert!(painted(cx));
    editor.read_with(cx, |editor, _| {
        // In another block than the head.
        let caret = editor.caret_bounds().unwrap();
        let there = editor.position_bounds(Position::new(2, 3)).unwrap();
        assert_eq!(caret.origin, there.origin);
    });

    // It stays through edits and selection changes, clamped to the
    // document.
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.set_caret(Position::new(1, 0)), cx);
        assert_eq!(editor.caret_override(), Some(Position::new(2, 3)));
        editor.set_caret_override(Some(Position::new(40, 400)), cx);
        let clamped = editor.document().clamp_position(Position::new(40, 400));
        assert_eq!(editor.caret_position(), clamped);
        assert_eq!(clamped.block, 2);
    });
    assert!(painted(cx));

    // Cleared by hand: the caret is the head again.
    editor.update(cx, |editor, cx| {
        editor.set_caret_override(None, cx);
        assert_eq!(editor.caret_position(), Position::new(1, 0));
        let caret = editor.caret_bounds().unwrap();
        let head = editor.position_bounds(Position::new(1, 0)).unwrap();
        assert_eq!(caret.origin, head.origin);
    });
    assert!(painted(cx));

    // Cleared by a new document.
    editor.update(cx, |editor, cx| {
        editor.set_caret_override(Some(Position::new(1, 2)), cx);
        editor.set_document(paragraphs(2), cx);
        assert_eq!(editor.caret_override(), None);
        assert_eq!(editor.caret_position(), Position::ZERO);
    });
}

#[gpui::test]
fn scrolling_to_the_caret_follows_the_override(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(paragraphs(300), EditorConfig::full(), cx);
    let cursor = Position::new(200, 4);
    editor.update(cx, |editor, cx| {
        editor.set_caret_override(Some(cursor), cx);
        editor.scroll_to_caret(Autoscroll::Fit, cx);
    });
    cx.run_until_parked();
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.selection(), Selection::caret(Position::ZERO));
        let visible = editor.visible_range().unwrap();
        assert!(visible.start <= cursor && cursor <= visible.end);
        let viewport = editor.viewport_bounds().unwrap();
        let caret = editor.caret_bounds().unwrap();
        assert!(caret.top() >= viewport.top() && caret.bottom() <= viewport.bottom());
    });

    // The autoscroll of an edit keeps the drawn caret in view, not the
    // head (which is at the top of the document here).
    let offset = editor.read_with(cx, |editor, _| editor.scroll_offset());
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.set_caret(Position::new(0, 3)), cx);
    });
    cx.run_until_parked();
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.scroll_offset(), offset);
    });

    // Without the override the head is the caret again.
    editor.update(cx, |editor, cx| {
        editor.set_caret_override(None, cx);
        editor.scroll_to_caret(Autoscroll::Fit, cx);
    });
    cx.run_until_parked();
    editor.read_with(cx, |editor, _| assert_eq!(editor.scroll_offset(), px(0.)));
}
