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
            7..=9 => Block::with_text(BlockKind::BulletItem, sentence.trim_end()),
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

    // Selecting everything and asking where the selection is (a format
    // menu does) does not shape the book either.
    cx.simulate_keystrokes("cmd-a");
    editor.read_with(cx, |editor, _| {
        let before = shaped(editor);
        let viewport = editor.viewport_bounds().unwrap();
        let all = editor.selection_bounds().unwrap();
        assert!(all.top() < viewport.top() && all.bottom() >= viewport.bottom() - px(400.));
        let rows = editor.range_bounds(editor.selection().range()).unwrap();
        assert!(!rows.is_empty() && rows.len() < 2_000);
        assert!(
            shaped(editor) < before + 200,
            "{} blocks shaped",
            shaped(editor)
        );
    });
    cx.simulate_keystrokes("cmd-up");

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

/// A small deterministic generator (the tests have no `rand`).
struct Dice(u64);

impl Dice {
    fn roll(&mut self, sides: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) as usize) % sides.max(1)
    }
}

/// Everything about where blocks and rows are, in content space: per block
/// its top, its row count, the top of its last row and the left edge of
/// its text; then the content height and the page count.
type Fingerprint = (Vec<(f32, usize, f32, f32)>, f32, usize);

fn fingerprint(editor: &Editor) -> Fingerprint {
    editor
        .with_layout(|lay| {
            let mut blocks = Vec::new();
            for index in 0..lay.count() {
                let placed = lay.placed(index).unwrap();
                let rows = placed.layout.rows.len();
                blocks.push((
                    f32::from(placed.top),
                    rows,
                    f32::from(placed.row_top(rows - 1)),
                    f32::from(placed.layout.text_left),
                ));
            }
            (
                blocks,
                f32::from(lay.layout.content_height()),
                lay.layout.page_count(),
            )
        })
        .expect("laid out")
}

/// The fingerprint of a brand-new editor on `document`, in a window of
/// its own of the same size.
fn fresh_fingerprint(document: Document, cx: &mut TestAppContext) -> Fingerprint {
    let (fresh, cx) = open(document, EditorConfig::full(), cx);
    let result = fresh.update(cx, |editor, cx| {
        editor.finish_layout(cx);
        fingerprint(editor)
    });
    cx.update(|window, _| window.remove_window());
    result
}

/// After any sequence of edits, the layout that was kept up to date
/// change by change is the layout a fresh editor computes from scratch,
/// and the counts are the document's. `WRITER_EDITOR_SEEDS=50` runs more
/// sequences than the default three.
#[gpui::test]
fn incremental_layout_matches_a_fresh_one(cx: &mut TestAppContext) {
    setup(cx);
    let seeds = std::env::var("WRITER_EDITOR_SEEDS")
        .ok()
        .and_then(|seeds| seeds.parse::<u64>().ok())
        .unwrap_or(3);
    for seed in 0..seeds {
        random_edits_keep_the_layout_exact(seed, cx);
    }
}

fn random_edits_keep_the_layout_exact(seed: u64, cx: &mut TestAppContext) {
    let (editor, cx) = open(book(120), EditorConfig::full(), cx);
    let mut dice = Dice(20261111 + seed * 7919);
    let words = [
        "ink",
        "paper",
        "a",
        "margin",
        "the",
        "quiet",
        "sentence",
        "of",
        "typography",
        "row",
    ];
    let kinds = [
        BlockKind::Paragraph,
        BlockKind::heading(1),
        BlockKind::heading(3),
        BlockKind::BulletItem,
        BlockKind::NumberedItem,
        BlockKind::todo(),
        BlockKind::Quote,
        BlockKind::code(),
    ];
    for step in 0..400 {
        let roll = dice.roll(100);
        editor.update(cx, |editor, cx| {
            let document = editor.document();
            let count = document.block_count();
            let random_position = |dice: &mut Dice| {
                let block = dice.roll(count);
                let length = document.blocks()[block].len();
                document.clamp_position(Position::new(block, dice.roll(length + 1)))
            };
            match roll {
                0..=29 => {
                    let mut text = String::new();
                    for _ in 0..1 + dice.roll(12) {
                        text.push_str(words[dice.roll(words.len())]);
                        text.push(' ');
                    }
                    editor.edit(|state| state.insert_text(&text), cx);
                }
                30..=37 => {
                    editor.edit(|state| state.insert_paragraph_break(), cx);
                }
                38..=45 => {
                    for _ in 0..1 + dice.roll(30) {
                        editor.edit(|state| state.backspace(), cx);
                    }
                }
                46..=57 => {
                    let position = random_position(&mut dice);
                    editor.edit(|state| state.set_caret(position), cx);
                }
                58..=65 => {
                    let (a, b) = (random_position(&mut dice), random_position(&mut dice));
                    editor.edit(
                        |state| state.set_selection(writer_core::Selection::new(a, b)),
                        cx,
                    );
                }
                66..=71 => {
                    let kind = kinds[dice.roll(kinds.len())].clone();
                    editor.edit(|state| state.set_block_kind(kind), cx);
                }
                72..=74 => {
                    editor.edit(|state| state.toggle_bold(), cx);
                }
                75..=77 => {
                    editor.edit(|state| state.delete_selection(), cx);
                }
                78..=80 => {
                    editor.edit(|state| state.indent(), cx);
                }
                81..=82 => {
                    editor.edit(|state| state.move_blocks_down(), cx);
                }
                83..=84 => {
                    editor.edit(|state| state.move_blocks_up(), cx);
                }
                85..=86 => {
                    editor.edit(|state| state.insert_page_break(), cx);
                }
                87 => {
                    editor.edit(|state| state.insert_divider(), cx);
                }
                88..=89 => {
                    editor.edit(
                        |state| state.paste_plain_text("one line\nanother line\n\nand a last one"),
                        cx,
                    );
                }
                90..=92 => {
                    editor.edit(|state| state.undo(), cx);
                }
                93 => {
                    editor.edit(|state| state.redo(), cx);
                }
                94 => {
                    editor.edit(|state| state.step_text_size(1), cx);
                }
                95 => {
                    editor.edit(
                        |state| state.set_alignment(writer_core::Alignment::Justify),
                        cx,
                    );
                }
                96 => editor.toggle_page_view(cx),
                97 => editor.scroll_by_rows(dice.roll(60) as isize - 30, cx),
                _ => editor.scroll_by_pages(dice.roll(5) as f32 - 2., cx),
            }
        });
        cx.run_until_parked();

        // The caret is where a click on it would put it, whatever is
        // measured so far.
        editor.read_with(cx, |editor, _| {
            let head = editor.selection().head;
            let bounds = editor.position_bounds(head).unwrap();
            assert_eq!(
                editor.position_at_point(bounds.center()),
                Some(head),
                "seed {seed}, step {step}"
            );
            assert_eq!(
                editor.stats(),
                editor.document().stats(),
                "seed {seed}, step {step}"
            );
        });

        if step % 25 == 24 {
            let (kept, document) = editor.update(cx, |editor, cx| {
                editor.finish_layout(cx);
                (fingerprint(editor), editor.document().clone())
            });
            let fresh = fresh_fingerprint(document, cx);
            assert_eq!(kept.0.len(), fresh.0.len(), "seed {seed}, step {step}");
            for (index, (kept, fresh)) in kept.0.iter().zip(&fresh.0).enumerate() {
                assert_eq!(kept, fresh, "seed {seed}, step {step}, block {index}");
            }
            assert_eq!(
                (kept.1, kept.2),
                (fresh.1, fresh.2),
                "seed {seed}, step {step}"
            );
        }
    }
    cx.update(|window, _| window.remove_window());
}

#[gpui::test]
fn the_gutter_of_a_numbered_list_fits_its_largest_number(cx: &mut TestAppContext) {
    setup(cx);
    let items = |count: usize| {
        (0..count)
            .map(|ix| Block::with_text(BlockKind::NumberedItem, &format!("Item {ix}")))
            .collect::<Vec<_>>()
    };
    // A paragraph, a list of 9, a paragraph, a list of 120, a bullet.
    let mut blocks = vec![Block::paragraph("before")];
    blocks.extend(items(9));
    blocks.push(Block::paragraph("between"));
    blocks.extend(items(120));
    blocks.push(Block::with_text(BlockKind::BulletItem, "a bullet"));
    let (editor, cx) = open(Document::from_blocks(blocks), EditorConfig::full(), cx);
    editor.update(cx, |editor, cx| editor.finish_layout(cx));
    cx.run_until_parked();

    let column_left = editor.read_with(cx, |editor, _| {
        editor.position_bounds(Position::ZERO).unwrap().left()
    });
    let text_left = |editor: &Editor, block: usize| {
        editor
            .position_bounds(Position::new(block, 0))
            .unwrap()
            .left()
            - column_left
    };
    // The numbers painted in the last frame: inside the column, and
    // ending a little left of the text.
    let assert_numbers_fit = |editor: &Editor, gutters: &[gpui::Pixels]| {
        assert!(!editor.painted_markers.is_empty());
        for marker in &editor.painted_markers {
            assert!(
                marker.left() >= column_left - px(0.01),
                "a number starts left of the column: {marker:?}"
            );
            assert!(
                gutters.iter().any(|gutter| {
                    (marker.right() - (column_left + *gutter - px(7.))).abs() < px(0.01)
                }),
                "{marker:?} does not end at one of the gutters {gutters:?}"
            );
        }
    };

    let wide = editor.read_with(cx, |editor, _| {
        // Nine items: the standard gutter, the same as a bullet's.
        let standard = text_left(editor, 131);
        assert_eq!(standard, px(26.));
        for block in 1..=9 {
            assert_eq!(text_left(editor, block), standard);
        }
        // 120 items: one gutter for all of them, wide enough for "120.".
        let wide = text_left(editor, 11);
        assert!(wide > standard + px(15.), "{wide:?}");
        for block in 11..=130 {
            assert_eq!(text_left(editor, block), wide);
        }
        wide
    });
    // The numbers themselves, at the end of the long list.
    editor.update(cx, |editor, cx| {
        editor.scroll_to_position(Position::new(129, 0), Autoscroll::Center, cx)
    });
    cx.run_until_parked();
    editor.read_with(cx, |editor, _| {
        let first = editor.visible_range().unwrap().start.block;
        assert!(first > 100, "three-digit numbers are on screen");
        // The bullet is not a label; every label here is a number.
        assert_numbers_fit(editor, &[wide]);
    });

    // Typing in an item of the long list reshapes that item only.
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.set_caret(Position::new(125, 4)), cx);
    });
    cx.simulate_input("x");
    editor.read_with(cx, |editor, _| assert_eq!(unmeasured(editor), 0));

    // A tenth item widens the gutter of the short list, for all its
    // items, and of no other.
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.set_caret(Position::new(9, 6)), cx);
    });
    cx.simulate_keystrokes("enter");
    cx.simulate_input("ten");
    cx.run_until_parked();
    let two_digits = editor.read_with(cx, |editor, _| {
        let gutter = text_left(editor, 1);
        assert!(gutter > px(26.) && gutter < wide, "{gutter:?}");
        for block in 1..=10 {
            assert_eq!(text_left(editor, block), gutter);
        }
        assert_eq!(text_left(editor, 12), wide);
        // Both lists are on screen.
        assert_numbers_fit(editor, &[gutter, wide]);
        gutter
    });
    // Without it the list is back to the standard gutter.
    cx.simulate_keystrokes("cmd-z cmd-z");
    cx.run_until_parked();
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().block_count(), 132);
        for block in 1..=9 {
            assert_eq!(text_left(editor, block), px(26.));
        }
    });

    // A paragraph in the middle of the long list makes two lists, each
    // with its own gutter; a nested list counts by itself.
    editor.update(cx, |editor, cx| {
        editor.edit(
            |state| {
                state.set_caret(Position::new(31, 0));
                state.set_block_kind(BlockKind::Paragraph);
            },
            cx,
        );
    });
    cx.run_until_parked();
    editor.read_with(cx, |editor, _| {
        // 20 items before the paragraph, 99 after it.
        assert_eq!(text_left(editor, 11), two_digits);
        assert_eq!(text_left(editor, 30), two_digits);
        assert_eq!(text_left(editor, 31), px(0.));
        assert_eq!(text_left(editor, 32), two_digits);
        assert_eq!(text_left(editor, 130), two_digits);
    });
    editor.update(cx, |editor, cx| {
        editor.edit(
            |state| {
                state.set_selection(writer_core::Selection::new(
                    Position::new(40, 0),
                    Position::new(42, 1),
                ));
                state.indent();
            },
            cx,
        );
    });
    cx.run_until_parked();
    editor.read_with(cx, |editor, _| {
        // Three nested items: the standard gutter after one indent step.
        assert_eq!(text_left(editor, 41), px(24. + 26.));
        // The outer list lost three numbers and still has two digits.
        assert_eq!(text_left(editor, 43), two_digits);
    });
}
