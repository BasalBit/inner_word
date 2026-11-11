//! The seams other crates plug into: mutation and events, key interception
//! and key context, geometry and visual rows, scrolling, popups, highlight
//! layers, view state, compact mode.

use std::{cell::RefCell, rc::Rc};

use gpui::{Context, KeyBinding, Render, TestAppContext, Window, div, prelude::*, px};
use writer_core::{Block, Document, Movement, Position, Selection, TextRange};

use super::slice::{open, setup};
use crate::{
    Autoscroll, CaretStyle, CompactOptions, EditorConfig, EditorEvent, HighlightStyle,
    InterceptPriority, KeyDisposition, PopupAnchor, PopupPlacement, ViewState, actions,
};

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

#[gpui::test]
fn edit_is_the_mutation_seam_and_emits_events(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(Document::new(), EditorConfig::full(), cx);
    let events = Rc::new(RefCell::new(Vec::new()));
    cx.update(|_, cx| {
        let events = events.clone();
        cx.subscribe(&editor, move |_, event: &EditorEvent, _| {
            events.borrow_mut().push(event.clone());
        })
        .detach();
    });
    let take = |cx: &mut gpui::VisualTestContext| {
        cx.run_until_parked();
        std::mem::take(&mut *events.borrow_mut())
    };

    let changed = editor.update(cx, |editor, cx| {
        editor.edit(|state| state.insert_text("Hi"), cx)
    });
    assert!(changed);
    let seen = take(cx);
    assert!(matches!(seen[0], EditorEvent::Edited { change } if change.blocks.is_some()));
    assert_eq!(seen[1], EditorEvent::SelectionChanged);
    assert_eq!(seen.len(), 2);
    editor.read_with(cx, |editor, _| assert_eq!(editor.stats().words, 1));

    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.move_caret(Movement::Left, false), cx)
    });
    assert_eq!(take(cx), vec![EditorEvent::SelectionChanged]);

    // Nothing changed: nothing emitted.
    editor.update(cx, |editor, cx| editor.edit(|_| (), cx));
    assert!(take(cx).is_empty());

    editor.update(cx, |editor, cx| editor.toggle_page_view(cx));
    assert_eq!(take(cx), vec![EditorEvent::SettingsChanged]);
    editor.read_with(cx, |editor, _| {
        assert!(editor.page_view());
        assert_eq!(editor.page_count(), Some(1));
        assert_eq!(editor.caret_page(), Some(1));
    });

    editor.update(cx, |editor, cx| editor.set_document(paragraphs(3), cx));
    assert_eq!(take(cx), vec![EditorEvent::DocumentReplaced]);

    // Read-only blocks the user, not the seam.
    editor.update(cx, |editor, cx| editor.set_read_only(true, cx));
    cx.simulate_input("x");
    cx.simulate_keystrokes("backspace enter");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().block_count(), 3);
        assert_eq!(editor.document().blocks()[0].text(), "Paragraph number 0");
    });
    cx.simulate_keystrokes("right");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.selection().head.offset, 1)
    });
}

#[gpui::test]
fn interceptors_see_keys_before_bindings_and_text(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(Document::new(), EditorConfig::full(), cx);
    let log = Rc::new(RefCell::new(Vec::<String>::new()));

    // A modal layer swallows "x" and "left"; an overlay above it takes "enter".
    let modal = editor.read_with(cx, |editor, _| {
        let log = log.clone();
        editor.intercept_keys(InterceptPriority::MODAL, move |key, _, _| {
            log.borrow_mut().push(format!("modal {}", key.key));
            match key.key.as_str() {
                "x" | "left" => KeyDisposition::Consumed,
                _ => KeyDisposition::Pass,
            }
        })
    });
    let overlay = editor.read_with(cx, |editor, _| {
        let log = log.clone();
        editor.intercept_keys(InterceptPriority::OVERLAY, move |key, _, _| {
            log.borrow_mut().push(format!("overlay {}", key.key));
            if key.key == "enter" {
                KeyDisposition::Consumed
            } else {
                KeyDisposition::Pass
            }
        })
    });

    cx.simulate_input("ax");
    cx.simulate_keystrokes("enter left");
    editor.read_with(cx, |editor, _| {
        // "a" passed both and was typed; "x", "enter" and "left" were consumed.
        assert_eq!(editor.plain_text(), "a");
        assert_eq!(editor.selection(), Selection::caret(Position::new(0, 1)));
    });
    assert_eq!(
        *log.borrow(),
        [
            "overlay a",
            "modal a",
            "overlay x",
            "modal x",
            "overlay enter",
            "overlay left",
            "modal left"
        ]
    );

    // Dropping the subscription removes the interceptor.
    drop(overlay);
    cx.simulate_keystrokes("enter");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().block_count(), 2)
    });

    // Interceptors may update the editor: they run outside any update.
    drop(modal);
    let weak = editor.downgrade();
    let _typist = editor.read_with(cx, |editor, _| {
        editor.intercept_keys(InterceptPriority::DEFAULT, move |key, _, cx| {
            if key.key != "q" {
                return KeyDisposition::Pass;
            }
            weak.update(cx, |editor, cx| {
                editor.edit(|state| state.insert_text("Q!"), cx);
            })
            .ok();
            KeyDisposition::Consumed
        })
    });
    cx.simulate_input("q");
    editor.read_with(cx, |editor, _| assert_eq!(editor.plain_text(), "a\nQ!"));

    // Not offered while the editor is not focused.
    log.borrow_mut().clear();
    let _spy = editor.read_with(cx, |editor, _| {
        let log = log.clone();
        editor.intercept_keys(InterceptPriority::OVERLAY, move |key, _, _| {
            log.borrow_mut().push(key.key.clone());
            KeyDisposition::Pass
        })
    });
    cx.update(|window, _| window.blur());
    cx.simulate_keystrokes("down");
    assert!(log.borrow().is_empty());
}

#[gpui::test]
fn text_input_switch_and_key_context(cx: &mut TestAppContext) {
    setup(cx);
    cx.update(|cx| {
        cx.bind_keys([KeyBinding::new(
            "j",
            actions::MoveDown,
            Some("Editor && vim_mode == normal"),
        )]);
    });
    let (editor, cx) = open(paragraphs(3), EditorConfig::full(), cx);

    // Without the context entry "j" is text.
    cx.simulate_input("j");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().blocks()[0].text(), "jParagraph number 0");
    });

    // With it, the consumer's binding takes part in normal dispatch.
    editor.update(cx, |editor, cx| {
        editor.set_key_context_value("vim_mode", "normal", cx);
        editor.set_text_input_enabled(false, cx);
        editor.set_caret_style(CaretStyle::Block, cx);
    });
    cx.simulate_keystrokes("j");
    cx.simulate_input("z");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.selection().head.block, 1);
        // Typed text is refused while text input is off.
        assert!(!editor.plain_text().contains('z'));
        assert!(editor.key_context().contains("vim_mode"));
        // The block caret is as wide as a character.
        assert!(editor.caret_bounds().unwrap().size.width > px(5.));
    });

    editor.update(cx, |editor, cx| {
        editor.remove_key_context_entry("vim_mode", cx);
        editor.set_text_input_enabled(true, cx);
    });
    cx.simulate_input("j");
    editor.read_with(cx, |editor, _| {
        assert!(editor.document().blocks()[1].text().contains('j'))
    });
}

#[gpui::test]
fn geometry_and_visual_rows(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(wrapping_document(), EditorConfig::full(), cx);

    editor.update(cx, |editor, cx| {
        let viewport = editor
            .viewport_bounds()
            .expect("laid out after the first frame");
        let caret = editor.caret_bounds().unwrap();
        assert!(viewport.contains(&caret.origin));
        assert_eq!(
            editor.position_at_point(caret.center()),
            Some(Position::ZERO)
        );

        // The long paragraph wraps: its first row ends before the block does.
        let block_len = editor.document().blocks()[0].len();
        let first = editor.visual_row(Position::ZERO).unwrap();
        assert_eq!(first.start, Position::ZERO);
        assert!(first.end.offset < block_len);
        let second = editor.visual_row(first.end).unwrap();
        assert_eq!(second.start, first.end);

        // One row down keeps the column.
        let from = Position::new(0, 7);
        let step = editor.move_by_visual_rows(from, 1, None).unwrap();
        assert_eq!(step.rows_moved, 1);
        assert_eq!(step.position, Position::new(0, second.start.offset + 7));
        assert_eq!(Some(step.goal_x), editor.column_x(from));
        // Back up with the same goal x returns to the start.
        let back = editor
            .move_by_visual_rows(step.position, -1, Some(step.goal_x))
            .unwrap();
        assert_eq!(back.position, from);
        // Moving past the ends stops at the first / last row.
        assert_eq!(
            editor
                .move_by_visual_rows(from, -5, None)
                .unwrap()
                .rows_moved,
            0
        );
        let down = editor.move_by_visual_rows(from, 1000, None).unwrap();
        assert_eq!(down.position.block, 1);

        // Range rectangles: one per row.
        let rects = editor
            .range_bounds(TextRange::new(Position::ZERO, second.end))
            .unwrap();
        assert_eq!(rects.len(), 2);
        assert!(rects[1].top() >= rects[0].bottom() - px(0.5));

        assert!(editor.rows_per_page().unwrap() >= 1);
        let rows = editor.visible_rows().unwrap();
        assert_eq!(rows[0], first);
        assert_eq!(rows[1], second);
        assert_eq!(rows.last().copied(), editor.last_visible_row());
        assert_eq!(editor.first_visible_row().unwrap().start, Position::ZERO);
        let visible = editor.visible_range().unwrap();
        assert_eq!(visible.start, Position::ZERO);
        assert_eq!(visible.end.block, 1);

        // Geometry is valid right after an edit, before any new frame.
        let before = editor.caret_bounds().unwrap();
        editor.edit(|state| state.insert_text("X"), cx);
        let after = editor.caret_bounds().unwrap();
        assert!(after.left() > before.left());
        assert_eq!(after.top(), before.top());
    });

    // The editor's own up / down use the oracle.
    cx.simulate_keystrokes("down");
    editor.read_with(cx, |editor, _| {
        let head = editor.selection().head;
        assert_eq!(head.block, 0);
        assert!(
            head.offset > 1,
            "down moved one visual row inside the block"
        );
    });
}

#[gpui::test]
fn scrolling_follows_the_caret_and_requests(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(paragraphs(400), EditorConfig::full(), cx);
    let visible = |editor: &crate::Editor| {
        let viewport = editor.viewport_bounds().unwrap();
        let caret = editor.caret_bounds().unwrap();
        caret.top() >= viewport.top() && caret.bottom() <= viewport.bottom()
    };

    cx.simulate_keystrokes("cmd-down");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.selection().head.block, 399);
        assert!(editor.scroll_offset() > px(0.));
        assert!(visible(editor));
    });
    cx.simulate_keystrokes("cmd-up");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.scroll_offset(), px(0.));
        assert!(visible(editor));
    });

    editor.update(cx, |editor, cx| {
        editor.scroll_by_pages(1., cx);
        let one_page = editor.scroll_offset();
        assert!(one_page > px(0.));
        editor.scroll_by_rows(-1, cx);
        assert!(editor.scroll_offset() < one_page);
        // The caret did not move, so it is off screen now.
        assert_eq!(editor.selection().head, Position::ZERO);

        editor.edit(|state| state.set_caret(Position::new(200, 0)), cx);
        assert!(visible(editor));
        editor.scroll_to_caret(Autoscroll::Top, cx);
        let viewport = editor.viewport_bounds().unwrap();
        assert_eq!(editor.caret_bounds().unwrap().top(), viewport.top());
        assert_eq!(
            editor.first_visible_row().unwrap().start,
            Position::new(200, 0)
        );
        editor.scroll_to_caret(Autoscroll::Bottom, cx);
        assert_eq!(editor.caret_bounds().unwrap().bottom(), viewport.bottom());
        assert_eq!(
            editor.last_visible_row().unwrap().start,
            Position::new(200, 0)
        );
    });

    cx.simulate_keystrokes("pagedown");
    editor.read_with(cx, |editor, _| {
        assert!(editor.selection().head.block > 200);
        assert!(visible(editor));
    });
}

struct Card;

impl Render for Card {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .debug_selector(|| "card".into())
            .w(px(120.))
            .h(px(40.))
    }
}

#[gpui::test]
fn popup_is_placed_at_the_anchor_inside_the_viewport(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(paragraphs(400), EditorConfig::full(), cx);
    let card = cx.update(|_, cx| cx.new(|_| Card));

    editor.update(cx, |editor, cx| {
        editor.show_popup(
            card.clone().into(),
            PopupAnchor::Caret,
            PopupPlacement::Below,
            cx,
        );
        assert!(editor.has_popup());
        assert_eq!(editor.popup_anchor(), Some(PopupAnchor::Caret));
    });
    cx.run_until_parked();
    let caret = editor.read_with(cx, |editor, _| editor.caret_bounds().unwrap());
    let bounds = cx.debug_bounds("card").expect("popup is drawn");
    assert_eq!(bounds.size.width, px(120.));
    assert!(bounds.top() > caret.bottom(), "below the caret");
    assert!(bounds.top() - caret.bottom() < px(12.));

    // At the bottom of the viewport it flips above the anchor, and it
    // follows the caret.
    cx.simulate_keystrokes("cmd-down");
    cx.run_until_parked();
    let (caret, viewport) = editor.update(cx, |editor, cx| {
        editor.scroll_to_caret(Autoscroll::Bottom, cx);
        (
            editor.caret_bounds().unwrap(),
            editor.viewport_bounds().unwrap(),
        )
    });
    cx.run_until_parked();
    let bounds = cx.debug_bounds("card").unwrap();
    assert!(bounds.bottom() < caret.top(), "flipped above the caret");
    assert!(bounds.top() >= viewport.top() && bounds.bottom() <= viewport.bottom());

    // Focus stayed in the editor the whole time.
    cx.update(|window, cx| assert!(editor.read(cx).is_focused(window)));

    // (GPUI's debug-bounds map keeps stale entries, so hiding is checked
    // through the API; the demo example shows it on screen.)
    editor.update(cx, |editor, cx| {
        editor.hide_popup(cx);
        assert!(!editor.has_popup());
        assert_eq!(editor.popup_anchor(), None);
    });
    cx.run_until_parked();
}

#[gpui::test]
fn highlight_layers_and_active_comment(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(paragraphs(5), EditorConfig::full(), cx);
    editor.update(cx, |editor, cx| {
        let color = gpui::red();
        let matches = editor
            .document()
            .find_all("number", writer_core::SearchOptions::default());
        assert_eq!(matches.len(), 5);
        // Given out of order, returned sorted.
        let mut shuffled = matches.clone();
        shuffled.reverse();
        editor.set_highlights("find", shuffled, HighlightStyle::background(color), cx);
        assert_eq!(editor.highlights("find"), matches.as_slice());
        editor.set_highlights(
            "find-current",
            vec![matches[2]],
            HighlightStyle::background(color).underline(color),
            cx,
        );
        assert_eq!(editor.highlights("find-current"), [matches[2]]);
        assert!(editor.highlights("missing").is_empty());
        editor.clear_highlights("find", cx);
        assert!(editor.highlights("find").is_empty());

        // Comments: mark a thread active.
        editor.edit(|state| state.select_range(TextRange::in_block(1, 0, 9)), cx);
        let id = editor
            .edit(|state| state.add_comment("Check", "Me", 0), cx)
            .expect("comment added on the selection");
        editor.set_active_comment(Some(id), cx);
        assert_eq!(editor.active_comment(), Some(id));
    });
    // A frame with tints, layers and a selection paints without trouble.
    cx.run_until_parked();
}

#[gpui::test]
fn view_state_round_trips(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(paragraphs(400), EditorConfig::full(), cx);
    let saved = editor.update(cx, |editor, cx| {
        editor.edit(
            |state| {
                state.set_selection(Selection::new(Position::new(150, 2), Position::new(151, 4)))
            },
            cx,
        );
        editor.scroll_by_rows(3, cx);
        editor.view_state()
    });
    assert_eq!(saved.selection().head, Position::new(151, 4));
    let json = serde_json::to_string(&saved).unwrap();
    let loaded: ViewState = serde_json::from_str(&json).unwrap();
    assert_eq!(loaded, saved);
    // Partial snapshots load with defaults.
    let partial: ViewState = serde_json::from_str(r#"{"head_block": 3}"#).unwrap();
    assert_eq!(partial.head_block, 3);

    let scroll = editor.read_with(cx, |editor, _| editor.scroll_offset());
    editor.update(cx, |editor, cx| {
        editor.set_document(paragraphs(400), cx);
        assert_eq!(editor.scroll_offset(), px(0.));
        editor.restore_view_state(&loaded, cx);
    });
    cx.run_until_parked();
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.selection(), saved.selection());
        assert_eq!(editor.scroll_offset(), scroll);
        assert_eq!(editor.view_state(), saved);
    });
}

#[gpui::test]
fn compact_mode_grows_and_submits(cx: &mut TestAppContext) {
    setup(cx);
    let config = EditorConfig::compact(CompactOptions::default().max_height(px(60.)))
        .placeholder("Add a comment");
    let (editor, cx) = open(Document::new(), config, cx);
    let submitted = Rc::new(RefCell::new(Vec::new()));
    cx.update(|_, cx| {
        let submitted = submitted.clone();
        cx.subscribe(&editor, move |editor, event: &EditorEvent, cx| {
            if matches!(event, EditorEvent::Submitted) {
                submitted.borrow_mut().push(editor.read(cx).plain_text());
            }
        })
        .detach();
    });
    let height = |cx: &mut gpui::VisualTestContext| {
        cx.run_until_parked();
        editor.read_with(cx, |editor, _| {
            editor.viewport_bounds().unwrap().size.height
        })
    };

    let one_row = height(cx);
    assert!(one_row > px(10.) && one_row < px(30.));
    editor.read_with(cx, |editor, _| assert!(editor.is_empty()));

    cx.simulate_input("hi");
    cx.simulate_keystrokes("shift-enter");
    cx.simulate_input("there");
    let two_rows = height(cx);
    assert!(two_rows > one_row, "the editor grows with its content");

    // Markdown rules are off in compact editors by default.
    cx.simulate_keystrokes("shift-enter");
    cx.simulate_input("# x");
    cx.simulate_keystrokes("shift-enter shift-enter shift-enter shift-enter");
    assert_eq!(height(cx), px(60.), "capped at the maximum height");

    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(*submitted.borrow(), ["hi\nthere\n# x\n\n\n\n"]);

    editor.update(cx, |editor, cx| {
        editor.clear(cx);
        assert!(editor.is_empty());
        editor.set_plain_text("one\ntwo", cx);
        assert_eq!(editor.plain_text(), "one\ntwo");
        assert_eq!(editor.document().block_count(), 2);
        assert_eq!(editor.selection(), Selection::caret(Position::new(1, 3)));
    });
}

#[gpui::test]
fn caret_blinks_and_follows_focus_and_the_window(cx: &mut TestAppContext) {
    use std::time::Duration;

    setup(cx);
    let (editor, cx) = open(paragraphs(3), EditorConfig::full(), cx);
    let painted = |cx: &mut gpui::VisualTestContext| {
        cx.run_until_parked();
        editor.read_with(cx, |editor, _| editor.caret_painted)
    };
    // Outside a scripted run the caret follows the window's state.
    editor.update(cx, |editor, cx| editor.set_draw_as_active(false, cx));
    assert!(painted(cx), "focused, active window: the caret shows");

    // Half a second on, half a second off.
    cx.executor().advance_clock(Duration::from_millis(600));
    assert!(!painted(cx));
    cx.executor().advance_clock(Duration::from_millis(530));
    assert!(painted(cx));
    cx.executor().advance_clock(Duration::from_millis(530));
    assert!(!painted(cx));
    // Typing and moving show it at once and start the blink over.
    cx.simulate_input("a");
    assert!(painted(cx));
    cx.executor().advance_clock(Duration::from_millis(400));
    cx.simulate_keystrokes("right");
    cx.executor().advance_clock(Duration::from_millis(400));
    assert!(painted(cx), "still in the first half after the move");

    // A range has no bar caret; the block and underline carets stay (Vim
    // shows its cursor in visual mode).
    cx.simulate_keystrokes("shift-right");
    assert!(!painted(cx));
    editor.update(cx, |editor, cx| {
        editor.set_caret_style(CaretStyle::Block, cx)
    });
    assert!(painted(cx));
    editor.update(cx, |editor, cx| editor.set_caret_style(CaretStyle::Bar, cx));
    cx.simulate_keystrokes("left");
    assert!(painted(cx));

    // No caret in a background window, unless asked for (scripted runs).
    cx.deactivate_window();
    assert!(!painted(cx));
    editor.update(cx, |editor, cx| editor.set_draw_as_active(true, cx));
    assert!(painted(cx));
    editor.update(cx, |editor, cx| editor.set_draw_as_active(false, cx));
    cx.update(|window, _| window.activate_window());
    assert!(painted(cx));

    // No caret without focus.
    cx.update(|window, _| window.blur());
    assert!(!painted(cx));
    cx.executor().advance_clock(Duration::from_secs(2));
    assert!(!painted(cx));
    cx.update(|window, cx| editor.read(cx).focus(window));
    assert!(painted(cx));
}

#[gpui::test]
fn hidden_comments_and_read_only_checks(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(paragraphs(3), EditorConfig::full(), cx);
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.select_range(TextRange::in_block(0, 0, 9)), cx);
        let first = editor
            .edit(|state| state.add_comment("One", "Me", 0), cx)
            .unwrap();
        editor.edit(|state| state.select_range(TextRange::in_block(1, 0, 9)), cx);
        let second = editor
            .edit(|state| state.add_comment("Two", "Me", 0), cx)
            .unwrap();
        // Hidden threads are remembered until replaced.
        editor.set_hidden_comments([first], cx);
        assert_eq!(editor.hidden_comments().collect::<Vec<_>>(), [first]);
        editor.set_hidden_comments([second, first], cx);
        assert_eq!(editor.hidden_comments().count(), 2);
        editor.set_hidden_comments([], cx);
        assert_eq!(editor.hidden_comments().count(), 0);
        editor.set_hidden_comments([first], cx);
    });
    // A frame with a hidden and a visible thread paints.
    cx.run_until_parked();
    editor.update(cx, |editor, cx| {
        // Replacing the document forgets them.
        editor.set_document(paragraphs(2), cx);
        assert_eq!(editor.hidden_comments().count(), 0);
        assert!(!editor.layout_pending() || editor.viewport_bounds().is_some());
    });
}
