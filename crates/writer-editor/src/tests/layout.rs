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
type Fingerprint = (Vec<(f32, usize, f32, f32, u64)>, f32, usize);

/// A hash of the rows of a block: which bytes each one holds and how it
/// ends.
fn rows_hash(layout: &crate::layout::BlockLayout) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for row in &layout.rows {
        (row.range.clone(), row.soft_wrapped).hash(&mut hasher);
    }
    hasher.finish()
}

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
                    rows_hash(&placed.layout),
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
        "(why?)",
        "don\u{2019}t",
        "well-known",
        "\u{201c}so!\u{201d}",
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
                72..=73 => {
                    editor.edit(|state| state.toggle_bold(), cx);
                }
                74 => {
                    editor.edit(|state| state.toggle_code(), cx);
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
        // measured so far. (Positions in the white space that hangs past
        // the end of a wrapped row are all drawn at the edge of the text
        // box: there the click finds one that is drawn where the caret
        // is.)
        editor.read_with(cx, |editor, _| {
            let head = editor.selection().head;
            let bounds = editor.position_bounds(head).unwrap();
            let found = editor.position_at_point(bounds.center()).unwrap();
            assert_eq!(
                editor.position_bounds(found),
                Some(bounds),
                "seed {seed}, step {step}: {found:?} for {head:?}"
            );
            let hangs = editor
                .document()
                .block(head.block)
                .is_some_and(|block| block.text()[..head.offset].ends_with(' '));
            assert!(found == head || hangs, "seed {seed}, step {step}");
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

/// A long paragraph and a long code block, built from a fixed sequence.
fn long_blocks() -> Document {
    let mut dice = Dice(7);
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
        "na\u{ef}ve",
        "\u{65e5}\u{672c}\u{8a9e}",
        "\u{1f469}\u{200d}\u{1f469}\u{200d}\u{1f467}",
    ];
    let mut prose = String::new();
    while prose.len() < 6_000 {
        prose.push_str(words[dice.roll(words.len())]);
        prose.push(' ');
    }
    let mut code = String::new();
    for line in 0..160 {
        code.push_str(&" ".repeat(4 * dice.roll(3)));
        code.push_str(&format!(
            "let value_{line} = compute({});\n",
            dice.roll(1_000)
        ));
    }
    Document::from_blocks(vec![
        Block::paragraph("before"),
        Block::paragraph(prose.trim_end()),
        Block::with_text(BlockKind::code(), code.trim_end()),
        Block::paragraph("after"),
    ])
}

/// Sets how long a block must be to be shaped in pieces and how long a
/// piece is, for this test thread.
fn set_piece_limits(limits: (usize, usize)) -> (usize, usize) {
    crate::layout::block::PIECE_LIMITS.with(|cell| cell.replace(limits))
}

#[gpui::test]
fn a_long_block_shaped_in_pieces_wraps_like_one_shaped_whole(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(long_blocks(), EditorConfig::full(), cx);
    for page_view in [false, true] {
        editor.update(cx, |editor, cx| editor.set_page_view(page_view, cx));
        cx.run_until_parked();
        for alignment in [
            writer_core::Alignment::Left,
            writer_core::Alignment::Justify,
            writer_core::Alignment::Right,
        ] {
            editor.update(cx, |editor, cx| {
                editor.edit(
                    |state| {
                        state.select_all();
                        state.set_alignment(alignment);
                    },
                    cx,
                );
            });
            cx.run_until_parked();
            editor.read_with(cx, |editor, _| {
                let document = editor.document();
                for index in [1, 2] {
                    let mut layout = editor.layout.borrow_mut();
                    // In pieces of several sizes, and as a whole.
                    let mut prints = Vec::new();
                    // (The last of the piecewise ones has pieces as long as
                    // the text: the code block is then cut by its number
                    // of lines.)
                    let all_limits = [
                        (200, 150),
                        (200, 333),
                        (500, 1_000),
                        (500, 100_000),
                        (usize::MAX, usize::MAX),
                    ];
                    for limits in all_limits {
                        let before = set_piece_limits(limits);
                        let shaped = layout.fresh_layout(index, document);
                        set_piece_limits(before);
                        match limits {
                            (usize::MAX, _) => assert_eq!(shaped.piece_count(), 0),
                            (_, 100_000) if index == 1 => assert_eq!(shaped.piece_count(), 1),
                            (_, 100_000) => assert_eq!(shaped.piece_count(), 4),
                            _ => assert!(shaped.piece_count() >= 5, "{}", shaped.piece_count()),
                        }
                        assert!(shaped.rows.len() > 50);
                        prints.push(shaped.row_fingerprint());
                    }
                    for print in &prints[..4] {
                        assert_eq!(print.len(), prints[4].len(), "block {index}");
                        for (row, (pieces, whole)) in print.iter().zip(&prints[4]).enumerate() {
                            // The same bytes on every row; glyphs at the
                            // same places, as far as positions hold up
                            // that are measured along one line of tens
                            // of thousands of pixels (the whole block
                            // shaped at once: the test platform adds up
                            // advances in an `f32`).
                            let same = pieces.range == whole.range
                                && pieces.soft_wrapped == whole.soft_wrapped
                                && pieces.y == whole.y
                                && pieces.glyphs.len() == whole.glyphs.len()
                                && pieces
                                    .glyphs
                                    .iter()
                                    .zip(&whole.glyphs)
                                    .all(|(a, b)| a.0 == b.0 && (a.1 - b.1).abs() < 0.5);
                            assert!(
                                same,
                                "block {index}, row {row}, {alignment:?}, page view \
                                 {page_view}: {:?} / {:?}",
                                pieces.range, whole.range,
                            );
                        }
                    }
                }
            });
        }
    }
}

/// After any sequence of edits to a long block, the layout that took over
/// pieces of its predecessors is the layout shaped from scratch.
#[gpui::test]
fn edits_to_a_long_block_keep_its_layout_exact(cx: &mut TestAppContext) {
    setup(cx);
    let seeds = std::env::var("WRITER_EDITOR_SEEDS")
        .ok()
        .and_then(|seeds| seeds.parse::<u64>().ok())
        .unwrap_or(3);
    let first = std::env::var("WRITER_EDITOR_FIRST_SEED")
        .ok()
        .and_then(|seed| seed.parse::<u64>().ok())
        .unwrap_or(0);
    for seed in first..first + seeds {
        let (editor, cx) = open(long_blocks(), EditorConfig::full(), cx);
        let mut dice = Dice(977 + seed * 131);
        let words = [
            "ink",
            "a",
            "margin",
            "typography",
            "\u{65e5}\u{672c}",
            "to",
            "row.",
        ];
        for step in 0..300 {
            let roll = dice.roll(100);
            editor.update(cx, |editor, cx| {
                let document = editor.document();
                // (Blocks can merge: a backspace at the start of one.)
                let block = (1 + dice.roll(2)).min(document.block_count() - 1);
                let length = document.blocks()[block].len();
                let at = |dice: &mut Dice| {
                    document.clamp_position(Position::new(block, dice.roll(length + 1)))
                };
                let start = at(&mut dice);
                // A range of up to a few rows from there.
                let end = document.clamp_position(Position::new(
                    block,
                    start.offset + dice.roll(if roll.is_multiple_of(7) { 900 } else { 40 }),
                ));
                let range = writer_core::Selection::new(start, end);
                match roll {
                    0..=34 => {
                        let mut text = String::new();
                        for _ in 0..1 + dice.roll(if roll.is_multiple_of(5) { 60 } else { 3 }) {
                            text.push_str(words[dice.roll(words.len())]);
                            text.push(' ');
                        }
                        editor.edit(
                            |state| {
                                state.set_caret(start);
                                state.insert_text(&text);
                            },
                            cx,
                        );
                    }
                    35..=44 => {
                        editor.edit(
                            |state| {
                                state.set_caret(start);
                                state.insert_text("x");
                            },
                            cx,
                        );
                    }
                    45..=59 => {
                        editor.edit(
                            |state| {
                                state.set_selection(range);
                                state.delete_selection();
                            },
                            cx,
                        );
                    }
                    60..=67 => {
                        editor.edit(
                            |state| {
                                state.set_caret(start);
                                state.backspace();
                            },
                            cx,
                        );
                    }
                    68..=75 => {
                        editor.edit(
                            |state| {
                                state.set_selection(range);
                                state.toggle_bold();
                            },
                            cx,
                        );
                    }
                    76..=81 => {
                        editor.edit(
                            |state| {
                                state.set_caret(start);
                                state.insert_soft_break();
                            },
                            cx,
                        );
                    }
                    82..=85 => {
                        editor.edit(
                            |state| {
                                state.set_selection(range);
                                state.toggle_italic();
                            },
                            cx,
                        );
                    }
                    86..=89 => {
                        editor.edit(|state| state.undo(), cx);
                    }
                    90..=91 => {
                        editor.edit(|state| state.redo(), cx);
                    }
                    92..=93 => {
                        let alignment = [
                            writer_core::Alignment::Left,
                            writer_core::Alignment::Justify,
                            writer_core::Alignment::Center,
                        ][dice.roll(3)];
                        editor.edit(
                            |state| {
                                state.set_caret(start);
                                state.set_alignment(alignment);
                            },
                            cx,
                        );
                    }
                    94..=95 => {
                        // The start and the very end of the block.
                        let end = document.block_range(block).end;
                        editor.edit(
                            |state| {
                                state.set_caret(end);
                                state.insert_text(" tail");
                                state.set_caret(Position::new(block, 0));
                                state.insert_text("Head ");
                            },
                            cx,
                        );
                    }
                    96 => editor.toggle_page_view(cx),
                    97 => {
                        editor.edit(
                            |state| {
                                state.set_caret(start);
                                state.step_text_size(1);
                            },
                            cx,
                        );
                    }
                    _ => {
                        editor.edit(
                            |state| {
                                state.set_caret(start);
                                state.paste_plain_text("one line\nanother line");
                            },
                            cx,
                        );
                    }
                }
            });
            // Sometimes a frame between two edits, sometimes not.
            if dice.roll(3) > 0 {
                cx.run_until_parked();
            }
            // Whatever an edit left behind (a reflow may be cut short at a
            // seam) is a consistent layout: the rows cover the text, and
            // a position is where a click on it would put the caret.
            editor.read_with(cx, |editor, _| {
                let document = editor.document();
                for index in 0..document.block_count() {
                    let kept = editor
                        .with_layout(|lay| lay.placed(index).map(|placed| placed.layout))
                        .flatten()
                        .expect("laid out");
                    let length = document.blocks()[index].len();
                    assert!(
                        kept.rows_are_contiguous(length),
                        "seed {seed}, step {step} (roll {roll}), block {index} of {length} \
                         bytes:\n{}",
                        kept.describe()
                    );
                    for _ in 0..4 {
                        let position =
                            document.clamp_position(Position::new(index, dice.roll(length + 1)));
                        let bounds = editor.position_bounds(position).unwrap();
                        assert_eq!(
                            editor.position_at_point(bounds.center()),
                            Some(position),
                            "seed {seed}, step {step} (roll {roll})"
                        );
                    }
                }
            });
            // Once the background has finished the reflow, it is the
            // layout shaped from scratch.
            if step % 5 == 4 {
                editor.update(cx, |editor, cx| editor.finish_layout(cx));
                editor.read_with(cx, |editor, _| {
                    let document = editor.document();
                    for index in 0..document.block_count() {
                        let kept = editor
                            .with_layout(|lay| lay.placed(index).map(|placed| placed.layout))
                            .flatten()
                            .expect("laid out");
                        assert!(!kept.has_seam());
                        let fresh = editor.layout.borrow_mut().fresh_layout(index, document);
                        assert_eq!(
                            kept.rows.len(),
                            fresh.rows.len(),
                            "seed {seed}, step {step} (roll {roll}), block {index}"
                        );
                        let (kept, fresh) = (kept.row_fingerprint(), fresh.row_fingerprint());
                        for (row, (kept, fresh)) in kept.iter().zip(&fresh).enumerate() {
                            assert_eq!(
                                kept, fresh,
                                "seed {seed}, step {step} (roll {roll}), block {index}, row {row}"
                            );
                        }
                    }
                });
            }
        }
        cx.update(|window, _| window.remove_window());
    }
}

#[gpui::test]
fn typing_in_a_very_long_paragraph_shapes_a_piece_of_it(cx: &mut TestAppContext) {
    setup(cx);
    // The limits of the real app, and a paragraph of half a megabyte.
    set_piece_limits(crate::layout::block::REAL_PIECE_LIMITS);
    let piece = crate::layout::block::REAL_PIECE_LIMITS.1;
    let sentence = "The quick brown fox jumps over the lazy dog and keeps on running. ";
    let text = sentence.repeat(512 * 1024 / sentence.len());
    let length = text.trim_end().len();
    let (editor, cx) = open(
        Document::from_blocks(vec![
            Block::with_text(BlockKind::heading(2), "One very long block"),
            Block::paragraph(text.trim_end()),
            Block::paragraph("The end."),
        ]),
        EditorConfig::full(),
        cx,
    );
    let shaped = || crate::layout::block::SHAPED_BYTES.with(|bytes| bytes.get());
    editor.update(cx, |editor, cx| editor.finish_layout(cx));
    cx.run_until_parked();
    let rows = editor.read_with(cx, |editor, _| {
        let layout = editor
            .with_layout(|lay| lay.placed(1).map(|placed| placed.layout))
            .flatten()
            .unwrap();
        assert!(layout.piece_count() > 20, "{}", layout.piece_count());
        layout.rows.len()
    });
    assert!(rows > 5_000);

    // Typing in the middle, at the start and at the end: each key shapes
    // a piece or two, not the paragraph.
    for offset in [length / 2, 0, length] {
        editor.update(cx, |editor, cx| {
            editor.edit(|state| state.set_caret(Position::new(1, offset)), cx);
        });
        cx.run_until_parked();
        let before = shaped();
        let started = Instant::now();
        cx.simulate_input("Hello there");
        let per_key = (shaped() - before) / 11;
        eprintln!(
            "typing at {offset} of {length}: {per_key} bytes shaped per key, {:?} per key",
            started.elapsed() / 11
        );
        assert!(
            per_key <= 4 * piece,
            "{per_key} bytes shaped per key at {offset}"
        );
        // The rows of this text are nearly full, so a character more
        // can move every row after it: the edit leaves that to the
        // background. What is on screen is laid out for good.
        editor.read_with(cx, |editor, _| {
            let kept = editor
                .with_layout(|lay| lay.placed(1).map(|placed| placed.layout))
                .flatten()
                .unwrap();
            assert!(kept.rows_are_contiguous(editor.document().blocks()[1].len()));
            assert!(!kept.has_seam() || editor.layout_pending());
            let fresh = editor
                .layout
                .borrow_mut()
                .fresh_layout(1, editor.document());
            let (kept, fresh) = (kept.row_fingerprint(), fresh.row_fingerprint());
            let caret_row = kept
                .iter()
                .position(|row| row.range.end >= editor.selection().head.offset)
                .unwrap();
            let from = caret_row.saturating_sub(40);
            let to = (caret_row + 150).min(kept.len()).min(fresh.len());
            assert_eq!(kept[from..to], fresh[from..to], "the rows around the caret");
        });
        // The background finishes the reflow in slices.
        let mut slices = 0;
        while editor.read_with(cx, |editor, _| editor.layout_pending()) {
            cx.executor().advance_clock(Duration::from_millis(2));
            slices += 1;
            assert!(slices < 400, "the reflow does not finish");
        }
        editor.read_with(cx, |editor, _| {
            let kept = editor
                .with_layout(|lay| lay.placed(1).map(|placed| placed.layout))
                .flatten()
                .unwrap();
            assert!(!kept.has_seam());
            let fresh = editor
                .layout
                .borrow_mut()
                .fresh_layout(1, editor.document());
            assert_eq!(kept.row_fingerprint(), fresh.row_fingerprint());
        });
    }
    // Deleting a word, splitting the paragraph and joining it again.
    let before = shaped();
    cx.simulate_keystrokes("alt-backspace alt-backspace");
    assert!(shaped() - before <= 6 * piece);
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.set_caret(Position::new(1, length / 3)), cx);
    });
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    cx.simulate_keystrokes("backspace");
    cx.run_until_parked();
    editor.update(cx, |editor, cx| editor.finish_layout(cx));
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().block_count(), 3);
        let kept = editor
            .with_layout(|lay| lay.placed(1).map(|placed| placed.layout))
            .flatten()
            .unwrap();
        let fresh = editor
            .layout
            .borrow_mut()
            .fresh_layout(1, editor.document());
        assert_eq!(kept.row_fingerprint(), fresh.row_fingerprint());
        // The counts are right once the background has caught up.
        assert_eq!(editor.stats(), editor.document().stats());
    });
}

// --- Line breaking and inline code --------------------------------------------

/// The text of every visual row of a block.
fn rows_of(editor: &Editor, block: usize) -> Vec<String> {
    let text = editor.document().blocks()[block].text().to_string();
    let mut rows = Vec::new();
    let mut at = 0;
    while at <= text.len() {
        let row = editor.visual_row(Position::new(block, at)).unwrap();
        rows.push(text[row.start.offset..row.end.offset].to_string());
        if row.end.offset >= text.len() {
            break;
        }
        // A soft-wrapped row ends where the next one starts; a hard line
        // ends before its line break.
        at = if text.as_bytes()[row.end.offset] == b'\n' {
            row.end.offset + 1
        } else {
            row.end.offset.max(at + 1)
        };
    }
    rows
}

#[gpui::test]
fn a_row_never_starts_with_closing_punctuation(cx: &mut TestAppContext) {
    setup(cx);
    // Dialogue, shifted by a prefix of one to thirty characters so that
    // every mark comes to stand at the end of a full row once.
    let dialogue = "Could they? \u{201c}Why not!\u{201d} she said; and then (quietly): \
                    \u{201c}don\u{2019}t\u{2026}\u{201d} It was well-known, wasn\u{2019}t it? Yes! ";
    let blocks: Vec<Block> = (1..=30)
        .map(|prefix| Block::paragraph(&format!("{}{}", "x".repeat(prefix), dialogue.repeat(6))))
        .collect();
    let count = blocks.len();
    let (editor, cx) = open(Document::from_blocks(blocks), EditorConfig::full(), cx);
    for page_view in [false, true] {
        editor.update(cx, |editor, cx| {
            editor.set_page_view(page_view, cx);
            editor.finish_layout(cx);
        });
        cx.run_until_parked();
        editor.read_with(cx, |editor, _| {
            let mut wrapped = 0;
            for block in 0..count {
                let rows = rows_of(editor, block);
                wrapped += rows.len() - 1;
                for (index, row) in rows.iter().enumerate() {
                    let first = row.chars().next().unwrap();
                    assert!(
                        index == 0 || first.is_alphanumeric() || "\u{201c}(".contains(first),
                        "block {block}: a row starts with {first:?}: {row:?}"
                    );
                    // Rows end after their white space, or after a hyphen.
                    let last = row.chars().next_back().unwrap();
                    assert!(
                        index == rows.len() - 1 || last == ' ' || last == '-',
                        "block {block}: a row ends with {last:?}: {row:?}"
                    );
                    assert!(!row.ends_with("\u{201c}") && !row.ends_with('('));
                }
                assert_eq!(rows.concat(), editor.document().blocks()[block].text());
            }
            assert!(wrapped > 150, "{wrapped} wraps looked at");
        });
    }
}

#[gpui::test]
fn a_word_keeps_its_row_when_only_the_space_after_it_does_not_fit(cx: &mut TestAppContext) {
    setup(cx);
    // The test font is 10.2 wide per character at body size and the
    // column 700: 68 characters fit, the 69th does not.
    let full = format!("{} {} tail", "a".repeat(33), "b".repeat(34));
    let (editor, cx) = open(
        Document::from_blocks(vec![Block::paragraph(&full)]),
        EditorConfig::full(),
        cx,
    );
    editor.read_with(cx, |editor, _| {
        let rows = rows_of(editor, 0);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].len(), 69, "the space hangs: {rows:?}");
        assert_eq!(rows[1], "tail");
        // The caret after the hanging space is drawn at the edge of the
        // text box, not outside it, and a click there finds it.
        let column = editor.position_bounds(Position::ZERO).unwrap().left();
        let end_of_word = editor.position_bounds(Position::new(0, 68)).unwrap();
        assert!((end_of_word.left() - column - px(693.6)).abs() < px(0.01));
        assert_eq!(
            editor.position_at_point(gpui::point(column + px(699.), end_of_word.center().y)),
            Some(Position::new(0, 68))
        );
    });
    // End of line goes past the space, and stays on the row.
    cx.simulate_keystrokes("cmd-right");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.selection().head, Position::new(0, 69));
        let caret = editor.caret_bounds().unwrap();
        let column = editor.position_bounds(Position::ZERO).unwrap();
        assert_eq!(caret.top(), column.top());
        assert!(caret.right() <= column.left() + px(701.), "{caret:?}");
    });
}

#[gpui::test]
fn inline_code_is_drawn_smaller_in_a_chip_inside_the_column(cx: &mut TestAppContext) {
    setup(cx);
    let code = writer_core::InlineAttrs {
        code: true,
        ..Default::default()
    };
    let plain = writer_core::InlineAttrs::default;
    let (editor, cx) = open(
        Document::from_blocks(vec![
            Block::new(BlockKind::Paragraph)
                .with_span("ab ", plain())
                .with_span("code", code.clone())
                .with_span(" cd", plain()),
            // Code at the very start of a row, and a justified row.
            Block::new(BlockKind::Paragraph)
                .with_span("start", code.clone())
                .with_span(" of the row", plain()),
            Block::new(BlockKind::Paragraph)
                .with_span(&"word ".repeat(11), plain())
                .with_span("some code", code.clone())
                .with_span(&" word".repeat(20), plain())
                .with_alignment(writer_core::Alignment::Justify),
            Block::paragraph("ab code cd"),
        ]),
        EditorConfig::full(),
        cx,
    );
    editor.read_with(cx, |editor, _| {
        let x = |block: usize, offset: usize| {
            editor
                .position_bounds(Position::new(block, offset))
                .unwrap()
                .left()
        };
        let column = x(0, 0);
        let close = |a: gpui::Pixels, b: f32| (f32::from(a - column) - b).abs() < 0.01;
        // 17 px text: 10.2 per shaped character, 8.976 per character of
        // code, 2.72 of padding on each side of the chip.
        let (glyph, small, pad) = (10.2, 10.2 * 0.88, 17. * 0.16);
        assert!(close(x(0, 2), glyph * 2.));
        // The chip starts where the code starts; its first letter comes
        // after the padding.
        assert!(close(x(0, 3), glyph * 3.), "{:?}", x(0, 3) - column);
        assert!(close(x(0, 4), glyph * 3. + pad + small));
        assert!(close(x(0, 6), glyph * 3. + pad + small * 3.));
        // It ends after the padding; what follows is not pushed further
        // than the chip is wide.
        let chip_end = glyph * 3. + pad * 2. + small * 4.;
        assert!(close(x(0, 7), chip_end));
        assert!(close(x(0, 8), chip_end + glyph));
        // The same text without code is laid out as shaped.
        assert!(close(x(3, 7), glyph * 7.));

        // The chip of code at the start of a row starts at the column
        // edge, not left of it.
        let chip = editor
            .range_bounds(writer_core::TextRange::in_block(1, 0, 5))
            .unwrap();
        assert_eq!(chip.len(), 1);
        assert_eq!(chip[0].left(), column);
        assert!(close(chip[0].right(), pad * 2. + small * 5.));

        // Every position on a row with code is found again by a click on
        // it, and a click on a letter of code finds that letter.
        for block in [0, 1, 2] {
            let length = editor.document().blocks()[block].len();
            for offset in 0..=length {
                let position = Position::new(block, offset);
                let bounds = editor.position_bounds(position).unwrap();
                let found = editor.position_at_point(bounds.center()).unwrap();
                assert_eq!(
                    editor.position_bounds(found),
                    Some(bounds),
                    "{position:?} found as {found:?}"
                );
            }
        }
        let on_d = gpui::point(
            column + px(glyph * 3. + pad + small * 2.5),
            editor
                .position_bounds(Position::new(0, 5))
                .unwrap()
                .center()
                .y,
        );
        assert_eq!(editor.position_under_point(on_d), Some(Position::new(0, 5)));

        // A justified row with code still ends at the edge of the column.
        let rows = rows_of(editor, 2);
        assert!(rows.len() >= 2, "{rows:?}");
        assert!(rows[0].contains("some code"));
        let visible_end = rows[0].trim_end().len();
        assert!(
            close(x(2, visible_end), 700.),
            "{:?}",
            x(2, visible_end) - column
        );
        // And holds more than it could if the code took its shaped width:
        // 68 shaped characters fill the column.
        assert!(rows[0].trim_end().len() >= 68, "{}", rows[0].len());
    });
}

// --- Writing at the end, and the settled layout ------------------------------

#[gpui::test]
fn writing_at_the_end_of_a_document_has_room_under_the_last_line(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(book(200), EditorConfig::full(), cx);
    let room = |cx: &mut gpui::VisualTestContext| {
        cx.run_until_parked();
        editor.read_with(cx, |editor, _| {
            let viewport = editor.viewport_bounds().unwrap();
            let caret = editor.caret_bounds().unwrap();
            (viewport.bottom() - caret.bottom()) / viewport.size.height
        })
    };
    // Going to the end shows the end, with a line under it.
    cx.simulate_keystrokes("cmd-down");
    let scroll_at_end = editor.read_with(cx, |editor, _| editor.scroll_offset());
    assert!(room(cx) < 0.2);
    // Typing there lifts the line: about a third of the window stays
    // free under it, and it stays there line after line.
    cx.simulate_input(" More");
    let after_typing = room(cx);
    assert!((0.29..0.36).contains(&after_typing), "{after_typing}");
    assert!(editor.read_with(cx, |editor, _| editor.scroll_offset()) > scroll_at_end);
    for _ in 0..5 {
        cx.simulate_keystrokes("enter");
        cx.simulate_input("Another line.");
        let now = room(cx);
        assert!((0.29..0.36).contains(&now), "{now}");
    }
    // Moving the caret or clicking does not move the text.
    let scroll = editor.read_with(cx, |editor, _| editor.scroll_offset());
    cx.simulate_keystrokes("left left up down");
    assert_eq!(
        editor.read_with(cx, |editor, _| editor.scroll_offset()),
        scroll
    );
    // Writing further up is not lifted.
    cx.simulate_keystrokes("up up up up up up up up up up up up up up up up up up");
    let scroll = editor.read_with(cx, |editor, _| editor.scroll_offset());
    cx.simulate_input("x");
    assert_eq!(
        editor.read_with(cx, |editor, _| editor.scroll_offset()),
        scroll
    );

    // Page view has the sheet's own margin under the last line.
    cx.simulate_keystrokes("cmd-down");
    editor.update(cx, |editor, cx| editor.set_page_view(true, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("cmd-up cmd-down");
    let before = editor.read_with(cx, |editor, _| editor.scroll_offset());
    cx.simulate_input("y");
    cx.run_until_parked();
    assert_eq!(
        editor.read_with(cx, |editor, _| editor.scroll_offset()),
        before
    );
}

#[gpui::test]
fn the_layout_says_when_page_numbers_are_final(cx: &mut TestAppContext) {
    setup(cx);
    let document = book(3000).with_settings(DocumentSettings {
        page_view: true,
        ..DocumentSettings::default()
    });
    let (editor, cx) = open(document, EditorConfig::full(), cx);
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    cx.update(|_, cx| {
        let events = events.clone();
        cx.subscribe(&editor, move |_, event: &EditorEvent, _| {
            events.borrow_mut().push(event.clone());
        })
        .detach();
    });
    // Right after opening, the total rests on estimates.
    let (pending, estimate) = editor.read_with(cx, |editor, _| {
        (editor.layout_pending(), editor.page_count().unwrap())
    });
    assert!(pending);
    assert!(!events.borrow().contains(&EditorEvent::LayoutSettled));
    // The background layout runs in slices; when it is through, it says
    // so once, and the total is the one a full layout gives.
    for _ in 0..400 {
        cx.executor().advance_clock(Duration::from_millis(2));
        cx.run_until_parked();
        if !editor.read_with(cx, |editor, _| editor.layout_pending()) {
            break;
        }
    }
    let settled = events
        .borrow()
        .iter()
        .filter(|event| **event == EditorEvent::LayoutSettled)
        .count();
    assert_eq!(settled, 1);
    let total = editor.read_with(cx, |editor, _| editor.page_count().unwrap());
    let exact = editor.update(cx, |editor, cx| {
        editor.finish_layout(cx);
        editor.page_count().unwrap()
    });
    assert_eq!(total, exact);
    // The estimate was close, which is all it has to be.
    let off = (estimate as f32 - exact as f32).abs() / exact as f32;
    assert!(off < 0.1, "estimated {estimate}, exact {exact}");
    // An edit that leaves nothing to measure in the background does not
    // announce anything.
    events.borrow_mut().clear();
    cx.simulate_input("x");
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(50));
    cx.run_until_parked();
    assert!(!editor.read_with(cx, |editor, _| editor.layout_pending()));
}
