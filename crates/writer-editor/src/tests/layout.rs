//! The lazy layout on a book-length document: what is shaped when, that
//! the view does not move while estimates turn into measures, and what
//! opening and typing cost.

use std::time::{Duration, Instant};

use gpui::{TestAppContext, px};
use writer_core::{Block, BlockKind, Document, DocumentSettings, Position, Stats};

use super::slice::{open, setup};
use crate::{Autoscroll, Editor, EditorConfig, EditorEvent};

/// A book: 10,000 blocks, chapters with headings, paragraphs of one to a
/// dozen sentences, a few lists. About 1.3 MB of text.
fn book(blocks: usize) -> Document {
    let sentence = "The quick brown fox jumps over the lazy dog and keeps on running. ";
    let mut out = Vec::with_capacity(blocks);
    for index in 0..blocks {
        out.push(match index % 40 {
            0 => Block::with_text(
                BlockKind::heading(2),
                &format!("Chapter {}", index / 40 + 1),
            ),
            7 | 8 | 9 => Block::with_text(BlockKind::BulletItem, sentence.trim_end()),
            _ => Block::paragraph(sentence.repeat(1 + index % 3 + (index % 7) / 3).trim_end()),
        });
    }
    Document::from_blocks(out)
}

fn unmeasured(editor: &Editor) -> usize {
    editor.layout.borrow().unmeasured()
}

fn shaped(editor: &Editor) -> usize {
    editor.layout.borrow().cached_layouts()
}

#[gpui::test]
fn only_what_is_on_screen_is_shaped(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(book(10_000), EditorConfig::full(), cx);
    let on_screen = editor.read_with(cx, |editor, _| {
        // The first frame shaped the viewport's blocks and nothing else.
        let shaped_now = shaped(editor);
        assert!(
            shaped_now > 5 && shaped_now < 60,
            "{shaped_now} blocks shaped"
        );
        assert_eq!(unmeasured(editor), 10_000 - shaped_now);
        assert!(editor.layout_pending());
        shaped_now
    });

    // Typing reshapes the block that changes, not its neighbours.
    cx.simulate_input("Hello there");
    editor.read_with(cx, |editor, _| {
        assert_eq!(shaped(editor), on_screen);
        assert_eq!(unmeasured(editor), 10_000 - on_screen);
    });

    // Jumping to the end shapes what is there; what was shaped at the top
    // stays cached.
    cx.simulate_keystrokes("cmd-down");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.selection().head.block, 9_999);
        let caret = editor.caret_bounds().unwrap();
        let viewport = editor.viewport_bounds().unwrap();
        assert!(caret.top() >= viewport.top() && caret.bottom() <= viewport.bottom());
        assert!(shaped(editor) < on_screen + 80);
        assert!(unmeasured(editor) > 9_800);
    });

    // A position far away can be asked about without a frame.
    editor.read_with(cx, |editor, _| {
        let far = editor.position_bounds(Position::new(5_000, 3)).unwrap();
        assert!(far.top() < editor.viewport_bounds().unwrap().top());
        let step = editor
            .move_by_visual_rows(Position::new(5_000, 3), 200, None)
            .unwrap();
        assert_eq!(step.rows_moved, 200);
        assert!(step.position.block > 5_000);
    });

    // The cache of shaped blocks is bounded however far the caret travels.
    for target in (0..10_000).step_by(37) {
        editor.update(cx, |editor, cx| {
            editor.edit(|state| state.set_caret(Position::new(target, 0)), cx);
        });
        cx.run_until_parked();
    }
    editor.read_with(cx, |editor, _| {
        assert!(shaped(editor) <= 900, "{}", shaped(editor))
    });
}

#[gpui::test]
fn the_view_does_not_move_while_the_layout_settles(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(book(4_000), EditorConfig::full(), cx);

    // Go to the middle: everything above is still an estimate.
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.set_caret(Position::new(2_000, 5)), cx);
        editor.scroll_to_caret(Autoscroll::Center, cx);
    });
    cx.run_until_parked();
    let snapshot = |editor: &Editor| {
        (
            editor.first_visible_row().unwrap(),
            editor.caret_bounds().unwrap(),
            editor.view_state(),
        )
    };
    let before = editor.read_with(cx, |editor, _| {
        assert!(unmeasured(editor) > 3_800);
        snapshot(editor)
    });
    let estimated_offset = editor.read_with(cx, |editor, _| editor.scroll_offset());

    // One slice of the background task per tick of the clock.
    let mut slices = 0;
    while editor.read_with(cx, |editor, _| editor.layout_pending()) {
        cx.executor().advance_clock(Duration::from_millis(2));
        slices += 1;
        assert!(slices < 200, "the background layout does not finish");
        if slices == 3 {
            // Part of the way: the view has not moved.
            editor.read_with(cx, |editor, _| assert_eq!(snapshot(editor), before));
        }
    }
    assert!(slices > 5, "the work is spread over slices: {slices}");
    cx.run_until_parked();
    editor.read_with(cx, |editor, _| {
        assert_eq!(unmeasured(editor), 0);
        assert_eq!(snapshot(editor), before, "same rows at the same place");
        // The pixel offset is another number now (the estimates above were
        // replaced), the view is the same.
        let exact = editor.scroll_offset();
        assert!(exact > px(10_000.) && estimated_offset > px(10_000.));
    });

    // Edits above the viewport do not move it either.
    editor.update(cx, |editor, cx| {
        let before = snapshot(editor).1;
        editor.edit_with(
            Autoscroll::None,
            |state| {
                state.grouped(|state| {
                    let selection = state.selection();
                    state.set_caret(Position::new(3, 0));
                    state.insert_text(
                        "One paragraph.\nAnother.\nA third one, all above the viewport.\n",
                    );
                    state.set_selection(writer_core::Selection::caret(Position::new(
                        selection.head.block + 3,
                        selection.head.offset,
                    )));
                })
            },
            cx,
        );
        let after = editor.caret_bounds().unwrap();
        assert_eq!(editor.selection().head, Position::new(2_003, 5));
        assert_eq!(after, before, "the text on screen stayed where it was");
    });
}

#[gpui::test]
fn word_counts_are_incremental_and_counted_in_the_background(cx: &mut TestAppContext) {
    setup(cx);
    let document = book(3_000);
    let expected = document.stats();
    let (editor, cx) = open(document, EditorConfig::full(), cx);
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    cx.update(|_, cx| {
        let events = events.clone();
        cx.subscribe(&editor, move |_, event: &EditorEvent, _| {
            events.borrow_mut().push(event.clone());
        })
        .detach();
    });

    // A long document is counted by the background task.
    editor.read_with(cx, |editor, _| {
        assert!(editor.stats_pending());
        assert_eq!(editor.stats(), Stats::default());
    });
    let mut ticks = 0;
    while editor.read_with(cx, |editor, _| editor.stats_pending()) {
        cx.executor().advance_clock(Duration::from_millis(2));
        ticks += 1;
        assert!(ticks < 100);
    }
    cx.run_until_parked();
    editor.read_with(cx, |editor, _| assert_eq!(editor.stats(), expected));
    assert_eq!(
        events
            .borrow()
            .iter()
            .filter(|event| matches!(event, EditorEvent::StatsChanged))
            .count(),
        1
    );

    // From here on every edit updates the counts at once.
    cx.simulate_input("Three more words ");
    editor.read_with(cx, |editor, _| {
        assert!(!editor.stats_pending());
        assert_eq!(editor.stats().words, expected.words + 3);
        assert_eq!(editor.stats(), editor.document().stats());
    });
    cx.simulate_keystrokes("cmd-a backspace");
    editor.read_with(cx, |editor, _| assert_eq!(editor.stats(), Stats::default()));
    cx.simulate_keystrokes("cmd-z");
    // Bringing the whole book back is counted in the background again;
    // until it is done the previous result is shown.
    editor.read_with(cx, |editor, _| {
        assert!(editor.stats_pending());
        assert_eq!(editor.stats(), Stats::default());
    });
    editor.update(cx, |editor, cx| editor.finish_layout(cx));
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.stats(), editor.document().stats());
    });
    // Everything selected: the selection's counts are the document's.
    cx.simulate_keystrokes("cmd-a");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.selection_stats(), Some(editor.stats()));
    });
}

/// Median of a set of durations.
fn median(mut samples: Vec<Duration>) -> Duration {
    samples.sort();
    samples[samples.len() / 2]
}

/// Opens a 10,000-block document and types into it, in flow view and in
/// page view, and prints what it costs. The bounds are loose (this runs
/// unoptimised, next to other builds); the numbers are for the report:
/// `cargo test -p writer-editor --lib ten_thousand -- --nocapture`.
#[gpui::test]
fn ten_thousand_blocks_open_and_type(cx: &mut TestAppContext) {
    setup(cx);
    for page_view in [false, true] {
        let document = book(10_000).with_settings(DocumentSettings {
            page_view,
            ..DocumentSettings::default()
        });
        let bytes: usize = document.blocks().iter().map(|block| block.len()).sum();

        let started = Instant::now();
        let (editor, cx) = open(document, EditorConfig::full(), cx);
        let open_time = started.elapsed();
        let first_frame = editor.read_with(cx, |editor, _| editor.frame_perf());

        // Typing at the start, in the middle and at the end: one key, one
        // frame.
        let mut keys = Vec::new();
        let mut frames = Vec::new();
        for block in [0, 5_000, 9_999] {
            editor.update(cx, |editor, cx| {
                editor.edit(|state| state.set_caret(Position::new(block, 0)), cx);
            });
            cx.run_until_parked();
            for ch in "The five boxing wizards jump quickly. ".chars() {
                let started = Instant::now();
                cx.simulate_input(&ch.to_string());
                keys.push(started.elapsed());
                frames.push(editor.read_with(cx, |editor, _| editor.frame_perf().prepaint));
            }
        }
        // Enter and backspace change the number of blocks.
        let mut splits = Vec::new();
        for _ in 0..20 {
            let started = Instant::now();
            cx.simulate_keystrokes("enter");
            cx.simulate_keystrokes("backspace");
            splits.push(started.elapsed() / 2);
        }
        let pending = editor.read_with(cx, |editor, _| unmeasured(editor));

        // The whole background layout, done in one go.
        let started = Instant::now();
        editor.update(cx, |editor, cx| editor.finish_layout(cx));
        let settle = started.elapsed();
        let pages = editor.read_with(cx, |editor, _| editor.page_count());

        // Typing after everything is measured (placement is incremental).
        let mut settled_keys = Vec::new();
        editor.update(cx, |editor, cx| {
            editor.edit(|state| state.set_caret(Position::new(5_000, 0)), cx);
        });
        cx.run_until_parked();
        for ch in "Pack my box with five dozen liquor jugs. ".chars() {
            let started = Instant::now();
            cx.simulate_input(&ch.to_string());
            settled_keys.push(started.elapsed());
        }

        eprintln!(
            "10,000 blocks ({:.2} MB), {}: open + first frame {:?} (prepaint {:?}), \
             key + frame median {:?} / max {:?} (prepaint median {:?}), \
             enter or backspace {:?}, {} blocks still estimated, \
             full background layout {:?}, key + frame after it {:?}, pages {:?}",
            bytes as f64 / 1_048_576.,
            if page_view { "page view" } else { "flow view" },
            open_time,
            first_frame.prepaint,
            median(keys.clone()),
            keys.iter().max().unwrap(),
            median(frames),
            median(splits),
            pending,
            settle,
            median(settled_keys.clone()),
            pages,
        );
        assert!(
            open_time < Duration::from_secs(3),
            "opening took {open_time:?}"
        );
        assert!(median(keys) < Duration::from_millis(40));
        assert!(median(settled_keys) < Duration::from_millis(40));
        assert!(pending > 9_000, "typing does not lay out the document");
        if page_view {
            assert!(pages.unwrap() > 500);
        }
        cx.update(|window, _| window.remove_window());
    }
}
