//! What the editor's Vim seams carry: typed text as the editor reports it,
//! the cursor drawn through the caret override, scrolling by real rows,
//! and what happens on blur and on a failed command.

use std::time::Duration;

use gpui::{EntityInputHandler, TestAppContext, VisualTestContext, px, size};
use writer_core::{Block, BlockKind, Document, Position, Selection};
use writer_editor::CaretStyle;
use writer_vim_mode::{CaretShape, Mode, VimEvent};

use super::{Rig, at, doc, open, setup};

// ----- typed text ----------------------------------------------------------------------------

#[gpui::test]
fn text_that_arrives_without_a_key_is_typed_text(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["ab"]), cx);
    cx.simulate_keystrokes("i");
    cx.simulate_input("x");
    // The emoji palette, dictation: the platform inserts text and no key
    // was pressed for it.
    rig.editor.update_in(cx, |editor, window, cx| {
        editor.replace_text_in_range(None, "\u{1f642}", window, cx);
    });
    cx.run_until_parked();
    cx.simulate_input("y");
    cx.simulate_keystrokes("escape");
    assert_eq!(rig.text(cx), "x\u{1f642}yab");
    rig.vim.read_with(cx, |vim, _| {
        assert_eq!(vim.engine().last_inserted_text(), "x\u{1f642}y");
    });
    // It repeats with the rest of the session, and the session is one undo
    // step.
    cx.simulate_keystrokes("$ .");
    assert_eq!(rig.text(cx), "x\u{1f642}yax\u{1f642}yb");
    cx.simulate_keystrokes("u u");
    assert_eq!(rig.text(cx), "ab");
}

#[gpui::test]
fn an_edit_after_a_key_that_typed_nothing_is_not_typed_text(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["abc"]), cx);
    cx.simulate_keystrokes("i");
    cx.simulate_input("x");
    // A printable key that Vim lets through and that then inserts nothing
    // (something below takes it) …
    let swallow = rig.editor.read_with(cx, |editor, _| {
        editor.intercept_keys(writer_editor::InterceptPriority::DEFAULT, |_, _, _| {
            writer_editor::KeyDisposition::Consumed
        })
    });
    cx.simulate_keystrokes("y");
    drop(swallow);
    // … followed by a change from somewhere else before the next key.
    rig.editor.update(cx, |editor, cx| {
        editor.edit(|state| state.insert_text("Q"), cx);
    });
    cx.run_until_parked();
    cx.simulate_input("z");
    cx.simulate_keystrokes("escape");
    assert_eq!(rig.text(cx), "xQzabc");
    // The engine never took "Q" for the text of the "y" key: what repeats
    // is what was typed.
    rig.vim.read_with(cx, |vim, _| {
        assert_eq!(vim.engine().last_inserted_text(), "xz");
    });
    cx.simulate_keystrokes("$ .");
    assert_eq!(rig.text(cx), "xQzabxzc");
}

#[gpui::test]
fn smart_punctuation_is_reported_as_typed(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["", ""]), cx);
    cx.simulate_keystrokes("i");
    cx.simulate_input("\"hi\" -- ok");
    cx.simulate_keystrokes("escape");
    let typed = rig.text(cx);
    assert!(typed.starts_with('\u{201c}'), "{typed:?}");
    // The record holds the keys, so the repeat runs the same rules again.
    rig.vim.read_with(cx, |vim, _| {
        assert_eq!(vim.engine().last_inserted_text(), "\"hi\" -- ok");
    });
    cx.simulate_keystrokes("j .");
    let blocks: Vec<String> = rig.editor.read_with(cx, |editor, _| {
        editor
            .document()
            .blocks()
            .iter()
            .map(|block| block.text().to_string())
            .collect()
    });
    assert_eq!(blocks[1], blocks[0], "{blocks:?}");
    assert!(blocks[0].ends_with("ok"));
}

#[gpui::test]
fn a_replacement_by_the_platform_corrects_what_was_typed(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["", "caf\u{e9} old"]), cx);
    cx.simulate_keystrokes("i");
    cx.simulate_input("cafe");
    // The accent menu of a held key replaces the letter that was typed.
    rig.editor.update_in(cx, |editor, window, cx| {
        editor.replace_text_in_range(Some(3..4), "\u{e9}", window, cx);
    });
    cx.run_until_parked();
    cx.simulate_input("!");
    cx.simulate_keystrokes("escape");
    assert_eq!(rig.mode(cx), Mode::Normal);
    rig.vim.read_with(cx, |vim, _| {
        assert_eq!(vim.engine().last_inserted_text(), "caf\u{e9}!");
    });
    // The repeat types the accented word.
    cx.simulate_keystrokes("j 0 .");
    assert_eq!(rig.text(cx), "caf\u{e9}!\ncaf\u{e9}!caf\u{e9} old");
    // One undo step per session.
    cx.simulate_keystrokes("u u");
    assert_eq!(rig.text(cx), "\ncaf\u{e9} old");
}

#[gpui::test]
fn a_replacement_of_older_text_is_a_change_from_outside(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["old"]), cx);
    cx.simulate_keystrokes("shift-a");
    cx.simulate_input(" new");
    // Text that was not typed in this session is replaced: Insert mode
    // goes on, and the repeat starts afresh.
    rig.editor.update_in(cx, |editor, window, cx| {
        editor.replace_text_in_range(Some(0..3), "OLD", window, cx);
    });
    cx.run_until_parked();
    assert_eq!(rig.mode(cx), Mode::Insert);
    assert_eq!(rig.text(cx), "OLD new");
    // (The editor leaves the caret after the replacement.)
    cx.simulate_input("!");
    cx.simulate_keystrokes("escape");
    assert_eq!(rig.text(cx), "OLD! new");
    rig.vim.read_with(cx, |vim, _| {
        assert_eq!(vim.engine().last_inserted_text(), "!");
    });
}

// ----- the cursor ----------------------------------------------------------------------------

/// A paragraph that wraps to many rows, between short ones.
fn tall_paragraph() -> Document {
    let long = "lorem ipsum dolor sit amet ".repeat(120);
    let mut blocks = vec![Block::paragraph("first")];
    blocks.push(Block::paragraph(long.trim_end()));
    blocks.extend((0..40).map(|index| Block::paragraph(&format!("line {index}"))));
    Document::from_blocks(blocks)
}

fn scroll(rig: &Rig, cx: &mut VisualTestContext) -> f32 {
    rig.editor
        .read_with(cx, |editor, _| f32::from(editor.scroll_offset()))
}

#[gpui::test]
fn the_visual_cursor_is_the_editors_caret(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["hello world", "", "next"]), cx);

    // Forward selection: the caret is drawn on the last selected
    // character, as a block, although the selection head is after it.
    cx.simulate_keystrokes("v e");
    assert_eq!(rig.caret_style(cx), CaretStyle::Block);
    assert_eq!(rig.status(cx).caret, CaretShape::Block);
    assert_eq!(rig.head(cx), at(0, 5));
    assert_eq!(rig.drawn_caret(cx), at(0, 4));
    let (caret, cell) = rig.editor.read_with(cx, |editor, _| {
        (
            editor.caret_bounds().expect("laid out"),
            editor.position_bounds(at(0, 4)).expect("laid out"),
        )
    });
    assert_eq!(caret.origin.x, cell.origin.x, "the caret is on the o");

    // Every movement moves it.
    cx.simulate_keystrokes("h");
    assert_eq!(rig.drawn_caret(cx), at(0, 3));
    cx.simulate_keystrokes("o");
    assert_eq!(rig.drawn_caret(cx), at(0, 0));
    assert_eq!(rig.caret_override(cx), None, "the head is the cursor");
    cx.simulate_keystrokes("o");
    assert_eq!(rig.drawn_caret(cx), at(0, 3));

    // An outside change of the selection moves it too.
    rig.editor.update(cx, |editor, cx| {
        editor.edit(
            |state| state.set_selection(Selection::new(at(0, 6), at(0, 11))),
            cx,
        );
    });
    cx.run_until_parked();
    assert_eq!(rig.mode(cx), Mode::Visual);
    assert_eq!(rig.drawn_caret(cx), at(0, 10));

    // Leaving Visual mode hands the caret back to the selection head.
    cx.simulate_keystrokes("escape");
    assert_eq!(rig.caret_override(cx), None);
    assert_eq!(rig.drawn_caret(cx), rig.head(cx));

    // Visual Line: the cursor keeps its column while the selection runs to
    // the ends of the lines.
    cx.simulate_keystrokes("0 l l shift-v");
    assert_eq!(rig.selection(cx), Selection::new(at(0, 0), at(0, 11)));
    assert_eq!(rig.drawn_caret(cx), at(0, 2));
    assert_eq!(rig.caret_style(cx), CaretStyle::Block);

    // An operator-pending underline and the Insert bar draw at the head.
    cx.simulate_keystrokes("escape d");
    assert_eq!(rig.caret_style(cx), CaretStyle::Underline);
    assert_eq!(rig.caret_override(cx), None);
    cx.simulate_keystrokes("escape i");
    assert_eq!(rig.caret_style(cx), CaretStyle::Bar);
    assert_eq!(rig.caret_override(cx), None);
    cx.simulate_keystrokes("escape");

    // Detaching in Visual mode leaves no override behind.
    cx.simulate_keystrokes("v e");
    assert!(rig.caret_override(cx).is_some());
    rig.vim.update(cx, |vim, cx| vim.detach(cx));
    cx.run_until_parked();
    assert_eq!(rig.caret_override(cx), None);
}

#[gpui::test]
fn an_empty_line_in_a_visual_line_selection_is_selected_text(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["one", "", "three"]), cx);

    // A single empty line: its line break is selected, so the editor has
    // something to tint, and the block caret sits on the line.
    cx.simulate_keystrokes("j shift-v");
    assert_eq!(rig.mode(cx), Mode::VisualLine);
    assert_eq!(rig.selection(cx), Selection::new(at(1, 0), at(2, 0)));
    assert_eq!(rig.drawn_caret(cx), at(1, 0));
    let tinted = rig.editor.read_with(cx, |editor, _| {
        editor
            .range_bounds(editor.selection().range())
            .expect("laid out")
    });
    assert!(
        tinted.iter().any(|rect| rect.size.width > px(0.)),
        "{tinted:?}"
    );

    // Extended upwards the empty line is the far end and stays tinted.
    cx.simulate_keystrokes("k");
    assert_eq!(rig.selection(cx), Selection::new(at(2, 0), at(0, 0)));
    assert_eq!(rig.drawn_caret(cx), at(0, 0));
    // The commands still see two lines.
    cx.simulate_keystrokes("d");
    assert_eq!(rig.text(cx), "three");

    // An empty last line reached from above.
    cx.simulate_keystrokes("u shift-v j");
    assert_eq!(rig.selection(cx), Selection::new(at(0, 0), at(2, 0)));
    assert_eq!(rig.drawn_caret(cx), at(1, 0));
    // The editor's own commands take the lines and nothing of "three".
    cx.simulate_keystrokes("cmd-b");
    rig.editor.read_with(cx, |editor, _| {
        let blocks = editor.document().blocks();
        assert!(blocks[0].attrs_at(1).unwrap().bold);
        assert!(!blocks[2].attrs_at(1).unwrap().bold);
    });
    assert_eq!(rig.mode(cx), Mode::VisualLine);
    cx.simulate_keystrokes("escape");
    assert_eq!(rig.head(cx), at(1, 0));
}

#[gpui::test]
fn the_view_follows_the_cursor_not_the_selection_head(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(tall_paragraph(), cx);
    cx.simulate_resize(size(px(520.), px(400.)));
    cx.run_until_parked();

    // Visual Line on the first row of a paragraph that runs far below the
    // window: the head of the selection is at its end, the cursor at its
    // start, and the view stays with the cursor.
    cx.simulate_keystrokes("j shift-v");
    assert_eq!(rig.mode(cx), Mode::VisualLine);
    let end = rig.head(cx);
    assert_eq!(end.block, 1);
    let end_is_visible = rig.editor.read_with(cx, |editor, _| {
        editor
            .visible_range()
            .is_some_and(|range| range.start <= end && end <= range.end)
    });
    assert!(!end_is_visible, "the paragraph is taller than the window");
    assert_eq!(scroll(&rig, cx), 0.);
    // Moving within the paragraph scrolls only as far as the cursor needs.
    cx.simulate_keystrokes("j j");
    assert_eq!(scroll(&rig, cx), 0.);
    let cursor = rig.cursor(cx);
    let visible = rig.editor.read_with(cx, |editor, _| {
        editor
            .visible_range()
            .is_some_and(|range| range.start <= cursor && cursor <= range.end)
    });
    assert!(visible);
    assert_eq!(rig.fallbacks(cx), 0);
}

// ----- scrolling -----------------------------------------------------------------------------

/// Headings, paragraphs that wrap and short ones: rows of several heights.
fn uneven_document() -> Document {
    let mut blocks = Vec::new();
    for chapter in 0..40 {
        blocks.push(Block::with_text(
            BlockKind::heading(if chapter % 3 == 0 { 1 } else { 2 }),
            &format!("Chapter {chapter}"),
        ));
        blocks.push(Block::paragraph(&format!(
            "Paragraph {chapter} runs on long enough to wrap over more than one row of the \
             column, so that rows and blocks are not the same thing here."
        )));
        blocks.push(Block::paragraph("A short one."));
        blocks.push(Block::paragraph("And another."));
    }
    Document::from_blocks(blocks)
}

fn visible_rows(rig: &Rig, cx: &mut VisualTestContext) -> Vec<Position> {
    rig.editor.read_with(cx, |editor, _| {
        editor
            .visible_rows()
            .unwrap()
            .iter()
            .map(|row| row.start)
            .collect()
    })
}

/// The start of the visual row the Vim cursor is on.
fn cursor_row(rig: &Rig, cx: &mut VisualTestContext) -> Position {
    let cursor = rig.cursor(cx);
    rig.editor
        .read_with(cx, |editor, _| editor.visual_row(cursor).unwrap().start)
}

/// How many rows lie between two row starts (`to` after `from`).
fn rows_between(rig: &Rig, from: Position, to: Position, cx: &mut VisualTestContext) -> usize {
    rig.editor.read_with(cx, |editor, _| {
        (0..2000)
            .find(|rows| {
                let step = editor
                    .move_by_visual_rows(from, *rows as isize, None)
                    .unwrap();
                editor.visual_row(step.position).unwrap().start == to
            })
            .expect("within 2000 rows")
    })
}

#[gpui::test]
fn scrolling_counts_the_rows_that_are_there(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(uneven_document(), cx);
    cx.simulate_resize(size(px(640.), px(520.)));
    cx.run_until_parked();
    cx.simulate_keystrokes("4 j");
    let start_rows = visible_rows(&rig, cx);
    let start_cursor = cursor_row(&rig, cx);
    assert_eq!(scroll(&rig, cx), 0.);

    // ctrl-e: the row below the top row becomes the top row, whatever its
    // height, and the caret stays where it is.
    let mut top = start_rows[0];
    for _ in 0..12 {
        let expected = rows_between(&rig, Position::ZERO, top, cx) + 1;
        cx.simulate_keystrokes("ctrl-e");
        let rows = visible_rows(&rig, cx);
        assert_eq!(
            rows_between(&rig, Position::ZERO, rows[0], cx),
            expected,
            "one row per ctrl-e"
        );
        top = rows[0];
        // The caret is dragged along one row inside the view.
        assert!(rows[1..].contains(&cursor_row(&rig, cx)), "{rows:?}");
    }
    // ctrl-y takes exactly the same rows back.
    for _ in 0..12 {
        cx.simulate_keystrokes("ctrl-y");
    }
    assert_eq!(scroll(&rig, cx), 0.);
    assert_eq!(visible_rows(&rig, cx), start_rows);

    // ctrl-d: the caret and the view move by the same rows, so the caret
    // keeps its place on screen through headings and wrapped paragraphs.
    cx.simulate_keystrokes("g g 4 j");
    assert_eq!(cursor_row(&rig, cx), start_cursor);
    let index = |rig: &Rig, cx: &mut VisualTestContext| {
        let row = cursor_row(rig, cx);
        visible_rows(rig, cx)
            .iter()
            .position(|start| *start == row)
            .expect("the caret row is completely visible")
    };
    let at_start = index(&rig, cx);
    let mut offsets = vec![scroll(&rig, cx)];
    let mut carets = vec![rig.cursor(cx)];
    for _ in 0..8 {
        let before_top = visible_rows(&rig, cx)[0];
        let before_caret = cursor_row(&rig, cx);
        let half = visible_rows(&rig, cx).len() / 2;
        cx.simulate_keystrokes("ctrl-d");
        let rows = visible_rows(&rig, cx);
        assert_eq!(rows_between(&rig, before_top, rows[0], cx), half);
        assert_eq!(
            rows_between(&rig, before_caret, cursor_row(&rig, cx), cx),
            half
        );
        assert_eq!(
            index(&rig, cx),
            at_start,
            "the caret kept its row on screen"
        );
        offsets.push(scroll(&rig, cx));
        carets.push(rig.cursor(cx));
    }
    assert!(offsets.windows(2).all(|pair| pair[1] > pair[0]));
    assert_eq!(rig.fallbacks(cx), 0);
}

#[gpui::test]
fn pages_are_the_rows_on_screen(cx: &mut TestAppContext) {
    setup(cx);
    let blocks: Vec<Block> = (0..400)
        .map(|index| Block::paragraph(&format!("line {index}")))
        .collect();
    let (rig, cx) = open(Document::from_blocks(blocks), cx);
    cx.simulate_resize(size(px(640.), px(520.)));
    cx.run_until_parked();
    let page = visible_rows(&rig, cx).len();
    assert!(page >= 8, "{page} rows");

    // Half a page is half the rows on screen …
    cx.simulate_keystrokes("ctrl-d");
    assert_eq!(rig.head(cx).block, page / 2);
    assert_eq!(visible_rows(&rig, cx)[0].block, page / 2);
    cx.simulate_keystrokes("ctrl-u");
    assert_eq!(rig.head(cx).block, 0);
    assert_eq!(scroll(&rig, cx), 0.);
    // … and a page is all of them but two, which stay for context.
    cx.simulate_keystrokes("ctrl-f");
    assert_eq!(rig.head(cx).block, page - 2);
    assert_eq!(visible_rows(&rig, cx)[0].block, page - 2);
    // (Below the top of the document the padding above the first row is
    // gone and a row more may fit: the page is what is on screen now.)
    let second_page = visible_rows(&rig, cx).len();
    cx.simulate_keystrokes("ctrl-f");
    assert_eq!(visible_rows(&rig, cx)[0].block, page - 2 + second_page - 2);
    assert_eq!(rig.head(cx).block, page - 2 + second_page - 2);
    cx.simulate_keystrokes("ctrl-b ctrl-b");
    assert_eq!(scroll(&rig, cx), 0.);
    assert_eq!(rig.head(cx).block, 0);
    // A count is rows for ctrl-d and pages for ctrl-f.
    cx.simulate_keystrokes("3 ctrl-d");
    assert_eq!(rig.head(cx).block, 3);
    assert_eq!(visible_rows(&rig, cx)[0].block, 3);

    // H and L keep the row of context the editor keeps; M is the middle.
    cx.simulate_keystrokes("5 0 shift-g z z");
    let rows = visible_rows(&rig, cx);
    cx.simulate_keystrokes("shift-h");
    assert_eq!(rig.head(cx).block, rows[1].block);
    cx.simulate_keystrokes("shift-l");
    assert_eq!(rig.head(cx).block, rows[rows.len() - 2].block);
    cx.simulate_keystrokes("shift-m");
    assert_eq!(rig.head(cx).block, rows[(rows.len() - 1) / 2].block);
    assert_eq!(visible_rows(&rig, cx), rows, "none of them scrolled");
    assert_eq!(rig.fallbacks(cx), 0);
}

// ----- blur and bell -------------------------------------------------------------------------

#[gpui::test]
fn losing_the_focus_drops_a_half_typed_command(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["one two three", "four"]), cx);
    cx.simulate_keystrokes("2 d");
    let status = rig.status(cx);
    assert_eq!(status.mode, Mode::OperatorPending);
    assert_eq!(status.pending_keys, "2d");
    assert_eq!(rig.caret_style(cx), CaretStyle::Underline);

    // The keyboard goes somewhere else (another panel, a modal).
    cx.update(|window, _| window.blur());
    cx.run_until_parked();
    let status = rig.status(cx);
    assert_eq!(status.mode, Mode::Normal);
    assert_eq!(status.pending_keys, "");
    assert_eq!(rig.caret_style(cx), CaretStyle::Block);
    assert_eq!(rig.context_mode(cx).as_deref(), Some("normal"));
    assert!(!status.bell, "nothing failed");

    // Back in the text, the next key is a command of its own.
    rig.editor
        .update_in(cx, |editor, window, _| editor.focus(window));
    cx.run_until_parked();
    cx.simulate_keystrokes("w");
    assert_eq!(rig.text(cx), "one two three\nfour");
    assert_eq!(rig.head(cx), at(0, 4));

    // A key that waits for its argument is dropped the same way.
    cx.simulate_keystrokes("f");
    assert_eq!(rig.status(cx).pending_keys, "f");
    cx.update(|window, _| window.blur());
    cx.run_until_parked();
    assert_eq!(rig.status(cx).pending_keys, "");

    // Modes are not commands: Insert mode and Visual mode stay, and an
    // open command line keeps what was typed into it.
    rig.editor
        .update_in(cx, |editor, window, _| editor.focus(window));
    cx.run_until_parked();
    cx.simulate_keystrokes("v e");
    cx.update(|window, _| window.blur());
    cx.run_until_parked();
    assert_eq!(rig.mode(cx), Mode::Visual);
    assert_eq!(rig.selected_text(cx), "two");
    rig.editor
        .update_in(cx, |editor, window, _| editor.focus(window));
    cx.run_until_parked();
    cx.simulate_keystrokes("escape : s / t w o");
    cx.update(|window, _| window.blur());
    cx.run_until_parked();
    assert_eq!(rig.status(cx).command_line.unwrap().text, "s/two");
    rig.editor
        .update_in(cx, |editor, window, _| editor.focus(window));
    cx.run_until_parked();
    cx.simulate_keystrokes("/ 2 enter");
    assert_eq!(rig.text(cx), "one 2 three\nfour");
}

#[gpui::test]
fn a_failed_command_rings_the_bell_for_a_moment(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["one two"]), cx);
    assert!(!rig.status(cx).bell);
    rig.take_events();

    // There is no "z" on the line.
    cx.simulate_keystrokes("f z");
    assert!(rig.status(cx).bell);
    let events = rig.take_events();
    assert!(events.contains(&VimEvent::Bell), "{events:?}");
    assert!(events.contains(&VimEvent::StatusChanged));
    assert_eq!(rig.status(cx).message, None, "a bell is not a message");

    // It goes out by itself …
    cx.executor().advance_clock(Duration::from_millis(500));
    cx.run_until_parked();
    assert!(!rig.status(cx).bell);
    assert!(rig.take_events().contains(&VimEvent::StatusChanged));

    // … or with the next command, whichever comes first.
    cx.simulate_keystrokes("f z");
    assert!(rig.status(cx).bell);
    cx.simulate_keystrokes("l");
    assert!(!rig.status(cx).bell);
    // A second failure while the first still shows starts over.
    cx.simulate_keystrokes("f z");
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.simulate_keystrokes("f z");
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    assert!(rig.status(cx).bell, "the second bell is 200 ms old");
    cx.executor().advance_clock(Duration::from_millis(300));
    cx.run_until_parked();
    assert!(!rig.status(cx).bell);

    // An error with a text rings too, and the text stays after the flash.
    cx.simulate_keystrokes("/ n o p e enter");
    let status = rig.status(cx);
    assert!(status.bell || status.message.is_some());
    cx.executor().advance_clock(Duration::from_millis(500));
    cx.run_until_parked();
    let status = rig.status(cx);
    assert!(!status.bell);
    assert!(status.message.is_some_and(|message| message.is_error));

    // Switching Vim mode off takes the flash with it.
    cx.simulate_keystrokes("f z");
    rig.vim.update(cx, |vim, cx| vim.detach(cx));
    assert!(!rig.status(cx).bell);
}

// ----- a popup that is filtered by typing ---------------------------------------------------

/// Stands in for the insert menu: a popup of the editor that leaves the
/// keyboard in the document.
struct Blank;

impl gpui::Render for Blank {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        _: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        gpui::div()
    }
}

#[gpui::test]
fn typing_for_a_popup_in_normal_mode_starts_insert_mode(cx: &mut TestAppContext) {
    use gpui::AppContext as _;
    use writer_editor::{PopupAnchor, PopupPlacement};

    setup(cx);
    let (rig, cx) = open(doc(&["quote me", "undo"]), cx);
    cx.simulate_keystrokes("x u");
    assert_eq!(rig.text(cx), "quote me\nundo");

    // The insert menu was opened from Normal mode (`:insertmenu`, the
    // palette): the keyboard stays in the document.
    let popup = cx.new(|_| Blank);
    rig.editor.update(cx, |editor, cx| {
        editor.show_popup(
            popup.clone().into(),
            PopupAnchor::Caret,
            PopupPlacement::Below,
            cx,
        );
    });
    cx.run_until_parked();
    assert_eq!(rig.mode(cx), Mode::Normal, "opening it changes nothing");

    // Its filter is typed: these are not the commands q, u, o, t, e.
    cx.simulate_input("quote");
    assert_eq!(rig.mode(cx), Mode::Insert);
    assert!(rig.text_input(cx));
    assert_eq!(rig.text(cx), "quotequote me\nundo");
    assert_eq!(rig.head(cx), at(0, 5));

    // The popup goes (applied or dismissed); typing goes on in Insert
    // mode until escape.
    rig.editor.update(cx, |editor, cx| editor.hide_popup(cx));
    cx.simulate_input("!");
    cx.simulate_keystrokes("escape");
    assert_eq!(rig.mode(cx), Mode::Normal);
    assert_eq!(rig.text(cx), "quote!quote me\nundo");
    // One undo step for what was typed.
    cx.simulate_keystrokes("u");
    assert_eq!(rig.text(cx), "quote me\nundo");

    // Keys that are not text never start Insert mode for a popup, and a
    // command that is already under way is finished, not typed.
    cx.simulate_keystrokes("d");
    rig.editor.update(cx, |editor, cx| {
        editor.show_popup(
            popup.clone().into(),
            PopupAnchor::Caret,
            PopupPlacement::Below,
            cx,
        );
    });
    cx.simulate_keystrokes("w");
    assert_eq!(rig.mode(cx), Mode::Normal);
    assert_eq!(rig.text(cx), "me\nundo");
    cx.simulate_keystrokes("right ctrl-e");
    assert_eq!(rig.mode(cx), Mode::Normal);
    assert_eq!(rig.text(cx), "me\nundo");
    // Without a popup the same letters are commands again.
    rig.editor.update(cx, |editor, cx| editor.hide_popup(cx));
    cx.simulate_keystrokes("g g x");
    assert_eq!(rig.text(cx), "e\nundo");
    assert_eq!(rig.mode(cx), Mode::Normal);
}
