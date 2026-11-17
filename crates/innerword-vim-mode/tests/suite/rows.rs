//! Movement over wrapped rows, the viewport commands and scrolling: the
//! engine's layout questions are answered by the editor's layout.

use gpui::{TestAppContext, VisualTestContext, px, size};
use innerword_core::{Block, Document, Position};
use innerword_vim_mode::Mode;

use super::{Rig, at, open, setup};

/// A paragraph that wraps to several rows in a narrow window, followed by
/// short ones.
fn wrapping_document() -> Document {
    let long = "lorem ipsum dolor sit amet ".repeat(12);
    Document::from_blocks(vec![
        Block::paragraph(long.trim_end()),
        Block::paragraph("short"),
        Block::paragraph("another short paragraph"),
    ])
}

fn narrow(cx: &mut VisualTestContext) {
    cx.simulate_resize(size(px(420.), px(600.)));
    cx.run_until_parked();
}

fn row_of(rig: &Rig, position: Position, cx: &mut VisualTestContext) -> (usize, usize) {
    rig.editor.read_with(cx, |editor, _| {
        let row = editor.visual_row(position).expect("laid out");
        (row.start.offset, row.end.offset)
    })
}

fn column_of(rig: &Rig, position: Position, cx: &mut VisualTestContext) -> f32 {
    rig.editor.read_with(cx, |editor, _| {
        f32::from(editor.column_x(position).unwrap())
    })
}

#[gpui::test]
fn j_and_k_move_by_visual_rows(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(wrapping_document(), cx);
    narrow(cx);
    let first_row = row_of(&rig, at(0, 0), cx);
    assert!(first_row.1 < 200, "the paragraph wraps: {first_row:?}");

    cx.simulate_keystrokes("4 l j");
    let below = rig.head(cx);
    assert_eq!(below.block, 0, "still inside the wrapped paragraph");
    let second_row = row_of(&rig, below, cx);
    assert_eq!(second_row.0, first_row.1, "the row right below");
    // The column is kept.
    let x = column_of(&rig, at(0, 4), cx);
    assert!((column_of(&rig, below, cx) - x).abs() < 6.);

    cx.simulate_keystrokes("k");
    assert_eq!(rig.head(cx), at(0, 4));

    // A count, and over the block boundary.
    cx.simulate_keystrokes("2 j");
    let third = rig.head(cx);
    assert_eq!(row_of(&rig, third, cx).0, second_row.1);
    cx.simulate_keystrokes("5 0 j");
    assert_eq!(rig.head(cx).block, 2, "clamped at the last row");
    cx.simulate_keystrokes("k");
    assert_eq!(rig.head(cx), at(1, 4));
    cx.simulate_keystrokes("k");
    let back = rig.head(cx);
    assert_eq!(back.block, 0);
    // The last row of the long paragraph.
    let last_row = row_of(&rig, back, cx);
    assert_eq!(last_row.1, wrapping_document().blocks()[0].len());

    // Arrow keys do the same in Normal mode.
    cx.simulate_keystrokes("g g down");
    assert_eq!(row_of(&rig, rig.head(cx), cx), second_row);
    cx.simulate_keystrokes("up");
    assert_eq!(rig.head(cx), at(0, 0));

    assert_eq!(rig.fallbacks(cx), 0, "every question had a layout answer");
}

#[gpui::test]
fn row_motions_and_the_sticky_row_end(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(wrapping_document(), cx);
    narrow(cx);
    let first_row = row_of(&rig, at(0, 0), cx);

    // g$ is the last character of the visual row, g0 its first.
    cx.simulate_keystrokes("g $");
    let end = rig.head(cx);
    assert_eq!(row_of(&rig, end, cx), first_row);
    assert!(end.offset + 2 >= first_row.1, "{end:?} in {first_row:?}");
    cx.simulate_keystrokes("j");
    let below = rig.head(cx);
    let second_row = row_of(&rig, below, cx);
    assert_eq!(second_row.0, first_row.1);
    assert!(below.offset + 2 >= second_row.1, "sticks to the row end");
    cx.simulate_keystrokes("g 0");
    assert_eq!(rig.head(cx), at(0, second_row.0));
    cx.simulate_keystrokes("g k");
    assert_eq!(rig.head(cx), at(0, 0));

    // `$` is the end of the hard line (the whole paragraph), and `j` after
    // it lands on the last character of the next line.
    cx.simulate_keystrokes("$");
    let len = wrapping_document().blocks()[0].len();
    assert_eq!(rig.head(cx), at(0, len - 1));
    cx.simulate_keystrokes("j");
    assert_eq!(rig.head(cx), at(1, 4));

    // An operator with a row motion: `d g $` deletes to the end of the row.
    cx.simulate_keystrokes("g g d g $");
    rig.editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().blocks()[0].len(), len - first_row.1);
    });
    // Visual mode moves by rows too.
    cx.simulate_keystrokes("v j");
    assert_eq!(rig.mode(cx), Mode::Visual);
    assert_eq!(rig.cursor(cx).block, 0);
    assert!(rig.cursor(cx).offset > 0);
    assert_eq!(rig.fallbacks(cx), 0);
}

fn many_paragraphs(count: usize) -> Document {
    Document::from_blocks(
        (0..count)
            .map(|index| Block::paragraph(&format!("  paragraph number {index}")))
            .collect(),
    )
}

fn visible(rig: &Rig, cx: &mut VisualTestContext) -> Vec<Position> {
    rig.editor.read_with(cx, |editor, _| {
        editor
            .visible_rows()
            .unwrap()
            .iter()
            .map(|row| row.start)
            .collect()
    })
}

fn scroll(rig: &Rig, cx: &mut VisualTestContext) -> f32 {
    rig.editor
        .read_with(cx, |editor, _| f32::from(editor.scroll_offset()))
}

#[gpui::test]
fn viewport_motions(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(many_paragraphs(200), cx);
    cx.simulate_resize(size(px(700.), px(500.)));
    cx.run_until_parked();
    let rows = visible(&rig, cx);
    assert!(rows.len() >= 6, "{} rows", rows.len());

    // L: the last row but one (a row of context stays below), on its
    // first non-blank character.
    cx.simulate_keystrokes("shift-l");
    let bottom = rows[rows.len() - 2];
    assert_eq!(rig.head(cx), at(bottom.block, 2));
    assert_eq!(scroll(&rig, cx), 0., "the view did not move");
    // H at the top of the document is the first row.
    cx.simulate_keystrokes("shift-h");
    assert_eq!(rig.head(cx), at(0, 2));
    cx.simulate_keystrokes("shift-m");
    let middle = rig.head(cx);
    assert!(middle.block > 0 && middle.block < bottom.block);
    assert_eq!(scroll(&rig, cx), 0.);
    assert_eq!(rig.fallbacks(cx), 0);
}

#[gpui::test]
fn scroll_commands_scroll_the_editor(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(many_paragraphs(300), cx);
    cx.simulate_resize(size(px(700.), px(500.)));
    cx.run_until_parked();
    let page = visible(&rig, cx).len();

    // ctrl-d: half a page down, the caret travels with the text.
    cx.simulate_keystrokes("ctrl-d");
    let after = scroll(&rig, cx);
    assert!(after > 0.);
    let head = rig.head(cx);
    assert!(head.block >= 1 && head.block <= page, "{head:?}");
    assert!(visible(&rig, cx).contains(&at(head.block, 0)));
    cx.simulate_keystrokes("ctrl-d");
    assert!(scroll(&rig, cx) > after);
    assert!(rig.head(cx).block > head.block);
    assert!(visible(&rig, cx).contains(&at(rig.head(cx).block, 0)));
    cx.simulate_keystrokes("ctrl-u ctrl-u");
    assert_eq!(scroll(&rig, cx), 0.);
    assert_eq!(rig.head(cx).block, 0);

    // ctrl-f / ctrl-b: a page.
    cx.simulate_keystrokes("ctrl-f");
    let paged = scroll(&rig, cx);
    assert!(paged > after);
    assert!(visible(&rig, cx).contains(&at(rig.head(cx).block, 0)));
    cx.simulate_keystrokes("ctrl-b");
    assert_eq!(scroll(&rig, cx), 0.);

    // ctrl-e scrolls a row; the caret stays until it would leave.
    cx.simulate_keystrokes("g g 5 j");
    let stay = rig.head(cx);
    cx.simulate_keystrokes("ctrl-e");
    let one_row = scroll(&rig, cx);
    assert!(one_row > 0.);
    assert_eq!(rig.head(cx), stay);
    cx.simulate_keystrokes("ctrl-y");
    assert_eq!(scroll(&rig, cx), 0.);
    // Scrolled far enough, the caret is dragged along and stays visible.
    cx.simulate_keystrokes("2 0 ctrl-e");
    assert!(rig.head(cx).block > stay.block);
    assert!(visible(&rig, cx).contains(&at(rig.head(cx).block, 0)));

    // zt, zz, zb put the caret's row at the top, the middle, the bottom.
    cx.simulate_keystrokes("1 0 0 shift-g z t");
    let rows = visible(&rig, cx);
    assert_eq!(rig.head(cx).block, 99);
    assert!(rows[..2].contains(&at(99, 0)), "{:?}", &rows[..2]);
    cx.simulate_keystrokes("z b");
    let rows = visible(&rig, cx);
    assert!(rows[rows.len() - 2..].contains(&at(99, 0)));
    cx.simulate_keystrokes("z z");
    let rows = visible(&rig, cx);
    let index = rows.iter().position(|row| *row == at(99, 0)).unwrap();
    assert!(
        index.abs_diff(rows.len() / 2) <= 1,
        "{index} of {}",
        rows.len()
    );
    // A movement right after does not shift the view.
    let before = scroll(&rig, cx);
    cx.simulate_keystrokes("l");
    assert_eq!(scroll(&rig, cx), before);

    assert_eq!(rig.fallbacks(cx), 0);
}

#[gpui::test]
fn random_keys_never_ask_for_rows_without_a_layout(cx: &mut TestAppContext) {
    setup(cx);
    let mut blocks = vec![Block::paragraph(&"lorem ipsum dolor sit amet ".repeat(10))];
    blocks.extend((0..40).map(|index| Block::paragraph(&format!("line {index} with words"))));
    let (rig, cx) = open(Document::from_blocks(blocks), cx);
    narrow(cx);

    const KEYS: &[&str] = &[
        "h",
        "j",
        "k",
        "l",
        "w",
        "b",
        "e",
        "0",
        "$",
        "^",
        "g",
        "shift-g",
        "shift-h",
        "shift-m",
        "shift-l",
        "d",
        "c",
        "y",
        "x",
        "p",
        "shift-p",
        "u",
        "ctrl-r",
        ".",
        "v",
        "shift-v",
        "o",
        "i",
        "a",
        "shift-a",
        "escape",
        "escape",
        "escape",
        "enter",
        "space",
        "backspace",
        "/",
        "?",
        "n",
        "shift-n",
        ":",
        "ctrl-d",
        "ctrl-u",
        "ctrl-e",
        "ctrl-y",
        "ctrl-f",
        "ctrl-b",
        "z",
        "t",
        "f",
        "r",
        "2",
        "3",
        "{",
        "}",
        "(",
        ")",
        "up",
        "down",
        "left",
        "right",
        "pagedown",
        "pageup",
        "home",
        "end",
        "tab",
        "*",
        "#",
        "%",
        "~",
        "shift-j",
        ">",
        "<",
        "shift-d",
        "shift-c",
        "s",
        "shift-y",
        "'",
        "m",
        ";",
        ",",
        "-",
        "+",
        "_",
        "|",
        "\"",
        "shift-r",
        "ctrl-o",
        "ctrl-w",
        "ctrl-n",
        "ctrl-p",
        "ctrl-j",
        "shift-z",
    ];
    // More steps with `VIM_MODE_WALK_STEPS=50000 cargo test -p innerword-vim-mode random_keys`.
    let steps = std::env::var("VIM_MODE_WALK_STEPS")
        .ok()
        .and_then(|steps| steps.parse().ok())
        .unwrap_or(1500);
    let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
    for step in 0..steps {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let key = KEYS[(seed % KEYS.len() as u64) as usize];
        cx.simulate_keystrokes(key);
        let fallbacks = rig.fallbacks(cx);
        assert_eq!(fallbacks, 0, "step {step}: {key:?} asked without a layout");
        // The editor follows the engine after every key.
        let mode = rig.mode(cx);
        assert_eq!(rig.text_input(cx), mode.is_insert(), "step {step}: {key:?}");
        assert_eq!(
            rig.status(cx).command_line.is_some(),
            mode == Mode::CommandLine
        );
    }
    rig.editor.read_with(cx, |editor, _| {
        editor.state().check_invariants().unwrap();
        let selection = editor.selection();
        assert!(editor.document().is_valid_position(selection.head));
    });
    // Whatever state the walk ended in, escape leads back to Normal mode.
    cx.simulate_keystrokes("escape escape escape");
    assert_eq!(rig.mode(cx), Mode::Normal);
}

#[gpui::test]
fn large_documents_stay_responsive(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(
        Document::from_blocks(
            (0..5000)
                .map(|index| {
                    Block::paragraph(&format!(
                        "Paragraph {index} of a long manuscript, with enough words to wrap."
                    ))
                })
                .collect(),
        ),
        cx,
    );
    cx.simulate_keystrokes("j");
    let started = std::time::Instant::now();
    for _ in 0..50 {
        cx.simulate_keystrokes("j");
    }
    let rows = started.elapsed();
    assert_eq!(rig.head(cx).block, 51);
    let started = std::time::Instant::now();
    for _ in 0..50 {
        cx.simulate_keystrokes("w");
    }
    let words = started.elapsed();
    eprintln!("5000 blocks: 50 x j in {rows:?}, 50 x w in {words:?}");
    // A debug build on a busy machine; the point is "no seconds per key".
    assert!(rows.as_secs_f32() < 20., "{rows:?}");
    assert!(words.as_secs_f32() < 20., "{words:?}");
    assert_eq!(rig.fallbacks(cx), 0);
}
