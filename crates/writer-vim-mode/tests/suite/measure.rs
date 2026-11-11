//! What a layout key costs on a book: 10,000 blocks with a full undo
//! history (990 single edits and two whole-document substitutions).
//!
//! The numbers are printed, not asserted beyond "no seconds per key":
//! `cargo test -p writer-vim-mode book -- --nocapture`.

use std::time::{Duration, Instant};

use gpui::{TestAppContext, VisualTestContext, px, size};
use writer_core::{Block, BlockKind, Document, Position};

use super::{Rig, open, setup};

const BLOCKS: usize = 10_000;

fn book() -> Document {
    Document::from_blocks(
        (0..BLOCKS)
            .map(|index| {
                if index % 50 == 0 {
                    Block::with_text(
                        BlockKind::heading(2),
                        &format!("Chapter {}", index / 50 + 1),
                    )
                } else {
                    Block::paragraph(&format!(
                        "Paragraph {index} of a long manuscript, with enough words in it to wrap \
                         over two or three rows of the column, as prose does."
                    ))
                }
            })
            .collect(),
    )
}

/// Fills the undo history up to its limit of a thousand steps: separate
/// small edits all over the document, then two substitutions that rewrite
/// every paragraph (each one undo step holding the whole text).
fn age(rig: &Rig, cx: &mut VisualTestContext) {
    rig.editor.update(cx, |editor, cx| {
        for step in 0..990 {
            editor.edit_with(
                writer_editor::Autoscroll::None,
                |state| {
                    state.set_caret(Position::new((step * 7919) % BLOCKS, 0));
                    state.insert_text("x");
                    state.break_undo_group();
                },
                cx,
            );
        }
        editor.edit_with(
            writer_editor::Autoscroll::None,
            |state| state.set_caret(Position::ZERO),
            cx,
        );
    });
    rig.vim.update(cx, |vim, cx| {
        vim.execute("%s/manuscript/typescript/", cx);
        vim.execute("%s/typescript/manuscript/", cx);
    });
    cx.simulate_keystrokes("g g");
    cx.run_until_parked();
}

fn time(keys: &str, times: usize, cx: &mut VisualTestContext) -> Duration {
    let started = Instant::now();
    for _ in 0..times {
        cx.simulate_keystrokes(keys);
    }
    started.elapsed() / times as u32
}

#[gpui::test]
fn layout_keys_on_a_book_with_a_long_undo_history(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(book(), cx);
    cx.simulate_resize(size(px(900.), px(700.)));
    cx.run_until_parked();
    age(&rig, cx);
    let depth = rig
        .editor
        .read_with(cx, |editor, _| editor.state().undo_depth());
    assert!(depth >= 900, "{depth} undo steps");

    // Warm up: the first keys shape the rows around the caret.
    cx.simulate_keystrokes("j k");
    let j = time("j", 200, cx);
    let k = time("k", 200, cx);
    let w = time("w", 200, cx);
    cx.simulate_keystrokes("g g");
    let half_down = time("ctrl-d", 60, cx);
    let half_up = time("ctrl-u", 60, cx);
    let page_down = time("ctrl-f", 30, cx);
    let page_up = time("ctrl-b", 30, cx);
    eprintln!(
        "{BLOCKS} blocks, {depth} undo steps, per key: j {j:?}, k {k:?}, w {w:?} (no layout \
         question), ctrl-d {half_down:?}, ctrl-u {half_up:?}, ctrl-f {page_down:?}, ctrl-b \
         {page_up:?}"
    );
    for cost in [j, k, half_down, half_up, page_down, page_up] {
        assert!(cost < Duration::from_secs(1), "{cost:?} per key");
    }
    assert_eq!(rig.fallbacks(cx), 0);
}
