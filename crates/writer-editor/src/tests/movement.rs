//! Caret movement and selection from the keyboard, across wrapped rows,
//! and hit testing round trips.

use gpui::{TestAppContext, px};
use writer_core::{Alignment, Block, BlockKind, Document, Position, Selection, text};

use super::slice::{open, setup};
use crate::{Editor, EditorConfig};

/// A paragraph that wraps into several rows of the 700px column, a short
/// one, and another long one.
fn wrapping() -> Document {
    let long = "word ".repeat(60);
    Document::from_blocks(vec![
        Block::paragraph(long.trim_end()),
        Block::paragraph("short"),
        Block::paragraph(long.trim_end()),
    ])
}

fn head(editor: &Editor) -> Position {
    editor.selection().head
}

#[gpui::test]
fn vertical_movement_keeps_a_goal_column(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(wrapping(), EditorConfig::full(), cx);

    // Rows of the first paragraph, from the oracle.
    let rows = editor.read_with(cx, |editor, _| {
        let mut rows = Vec::new();
        let mut at = Position::ZERO;
        loop {
            let row = editor.visual_row(at).unwrap();
            rows.push(row);
            if row.end.offset >= editor.document().blocks()[0].len() {
                break;
            }
            at = row.end;
        }
        rows
    });
    assert!(rows.len() >= 4, "the paragraph wraps: {} rows", rows.len());

    // Column 32 of the first row; down keeps it on every row.
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.set_caret(Position::new(0, 32)), cx);
    });
    for row in &rows[1..] {
        cx.simulate_keystrokes("down");
        editor.read_with(cx, |editor, _| {
            assert_eq!(head(editor), Position::new(0, row.start.offset + 32));
        });
    }
    // Into the short paragraph: clamped to its end. Further down the
    // column comes back, because the goal x is kept.
    cx.simulate_keystrokes("down");
    editor.read_with(cx, |editor, _| {
        assert_eq!(head(editor), Position::new(1, 5))
    });
    cx.simulate_keystrokes("down");
    editor.read_with(cx, |editor, _| {
        assert_eq!(head(editor), Position::new(2, 32))
    });
    cx.simulate_keystrokes("up up");
    editor.read_with(cx, |editor, _| {
        assert_eq!(
            head(editor),
            Position::new(0, rows.last().unwrap().start.offset + 32)
        );
    });

    // A horizontal move sets a new goal.
    cx.simulate_keystrokes("left down");
    editor.read_with(cx, |editor, _| {
        assert_eq!(head(editor), Position::new(1, 5))
    });
    cx.simulate_keystrokes("down");
    editor.read_with(cx, |editor, _| {
        assert_eq!(head(editor), Position::new(2, 31))
    });

    // shift extends by rows; a plain vertical move collapses to the
    // selection's edge first.
    cx.simulate_keystrokes("shift-up shift-up");
    editor.read_with(cx, |editor, _| {
        let selection = editor.selection();
        assert_eq!(selection.anchor, Position::new(2, 31));
        assert_eq!(selection.head.block, 0);
        assert!(selection.is_reversed());
    });
    cx.simulate_keystrokes("down");
    editor.read_with(cx, |editor, _| {
        let selection = editor.selection();
        assert!(selection.is_collapsed());
        assert_eq!(
            selection.head.block, 2,
            "down leaves from the selection's end"
        );
    });

    // At the first row up goes to the document start, at the last row down
    // goes to its end.
    cx.simulate_keystrokes("cmd-up right right up");
    editor.read_with(cx, |editor, _| assert_eq!(head(editor), Position::ZERO));
    cx.simulate_keystrokes("cmd-down left left down");
    editor.read_with(cx, |editor, _| {
        assert_eq!(head(editor), editor.document().end());
    });
}

#[gpui::test]
fn line_keys_work_on_visual_rows(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(wrapping(), EditorConfig::full(), cx);
    let at = |cx: &mut gpui::VisualTestContext| editor.read_with(cx, |editor, _| head(editor));
    // Rows of the first paragraph are 65 bytes ("word " thirteen times).
    let second = editor.read_with(cx, |editor, _| {
        editor.visual_row(Position::new(0, 70)).unwrap()
    });
    assert_eq!((second.start.offset, second.end.offset), (65, 130));

    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.set_caret(Position::new(0, 70)), cx);
    });
    // The start of the row, not of the paragraph.
    cx.simulate_keystrokes("cmd-left");
    assert_eq!(at(cx), Position::new(0, 65));
    cx.simulate_keystrokes("cmd-left");
    assert_eq!(at(cx), Position::new(0, 65));
    // The end of the row: the position where it wraps, which is also the
    // start of the next row. The caret stays on the row it came from.
    let row_top = |cx: &mut gpui::VisualTestContext, offset: usize| {
        editor.read_with(cx, |editor, _| {
            editor
                .position_bounds(Position::new(0, offset))
                .unwrap()
                .top()
        })
    };
    let caret_top = |cx: &mut gpui::VisualTestContext| {
        editor.read_with(cx, |editor, _| editor.caret_bounds().unwrap().top())
    };
    cx.simulate_keystrokes("cmd-right");
    assert_eq!(at(cx), Position::new(0, 130));
    assert_eq!(caret_top(cx), row_top(cx, 70));
    assert!(row_top(cx, 130) > row_top(cx, 70));
    cx.simulate_keystrokes("end");
    assert_eq!(at(cx), Position::new(0, 130));
    assert_eq!(caret_top(cx), row_top(cx, 70));
    cx.simulate_keystrokes("home");
    assert_eq!(at(cx), Position::new(0, 65));
    // Extending selects the row, with the space it wraps at.
    cx.simulate_keystrokes("cmd-shift-right");
    editor.read_with(cx, |editor, _| {
        assert_eq!(
            editor.selection(),
            Selection::new(Position::new(0, 65), Position::new(0, 130))
        );
    });
    // Collapsing to the end leaves the caret at the end of that row, and
    // from there the row is still this one.
    cx.simulate_keystrokes("right");
    assert_eq!(at(cx), Position::new(0, 130));
    assert_eq!(caret_top(cx), row_top(cx, 70));
    cx.simulate_keystrokes("shift-home");
    editor.read_with(cx, |editor, _| {
        assert_eq!(
            editor.selection(),
            Selection::new(Position::new(0, 130), Position::new(0, 65))
        );
    });
    // The last row of the paragraph ends at the end of the block.
    cx.simulate_keystrokes("cmd-down cmd-up down down down down cmd-right");
    editor.read_with(cx, |editor, _| {
        assert_eq!(
            head(editor),
            Position::new(0, editor.document().blocks()[0].len())
        );
    });

    // Deleting to the start and to the end of the row.
    let length = |cx: &mut gpui::VisualTestContext| {
        editor.read_with(cx, |editor, _| editor.document().blocks()[0].len())
    };
    let before = length(cx);
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.set_caret(Position::new(0, 70)), cx);
    });
    cx.simulate_keystrokes("cmd-backspace");
    assert_eq!(at(cx), Position::new(0, 65));
    assert_eq!(length(cx), before - 5);
    cx.simulate_keystrokes("right right cmd-delete");
    assert_eq!(at(cx), Position::new(0, 67));
    // The row was 65 bytes again after the first deletion; 63 of them go.
    assert_eq!(length(cx), before - 5 - 63);
    cx.simulate_keystrokes("cmd-z cmd-z");
    assert_eq!(length(cx), before);
}

#[gpui::test]
fn horizontal_word_line_and_document_movement(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(
        Document::from_blocks(vec![
            Block::paragraph("one two"),
            Block::paragraph("three\nfour five"),
        ]),
        EditorConfig::full(),
        cx,
    );
    let at = |cx: &mut gpui::VisualTestContext| editor.read_with(cx, |editor, _| head(editor));

    cx.simulate_keystrokes("right right");
    assert_eq!(at(cx), Position::new(0, 2));
    cx.simulate_keystrokes("alt-right");
    assert_eq!(at(cx), Position::new(0, 3));
    cx.simulate_keystrokes("alt-right alt-left");
    assert_eq!(at(cx), Position::new(0, 4));
    cx.simulate_keystrokes("cmd-right");
    assert_eq!(at(cx), Position::new(0, 7));
    // Right at the end of a block enters the next one; left returns.
    cx.simulate_keystrokes("right");
    assert_eq!(at(cx), Position::new(1, 0));
    cx.simulate_keystrokes("left");
    assert_eq!(at(cx), Position::new(0, 7));
    // Line start and end work per hard line of a block.
    cx.simulate_keystrokes("cmd-down");
    assert_eq!(at(cx), Position::new(1, 15));
    cx.simulate_keystrokes("cmd-left");
    assert_eq!(at(cx), Position::new(1, 6));
    cx.simulate_keystrokes("up end");
    assert_eq!(at(cx), Position::new(1, 5));
    cx.simulate_keystrokes("home ctrl-e ctrl-a ctrl-f ctrl-b ctrl-n ctrl-p");
    assert_eq!(at(cx), Position::new(1, 0));
    cx.simulate_keystrokes("cmd-up");
    assert_eq!(at(cx), Position::ZERO);

    // The same with shift selects.
    cx.simulate_keystrokes("shift-right alt-shift-right");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.state().selected_text(), "one")
    });
    cx.simulate_keystrokes("cmd-shift-right");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.state().selected_text(), "one two")
    });
    cx.simulate_keystrokes("cmd-shift-down");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.state().selected_text(), "one two\nthree\nfour five");
    });
    cx.simulate_keystrokes("shift-left alt-shift-left cmd-shift-left");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.state().selected_text(), "one two\nthree\n");
    });
    cx.simulate_keystrokes("cmd-shift-up");
    editor.read_with(cx, |editor, _| assert!(editor.selection().is_collapsed()));
    cx.simulate_keystrokes("cmd-a");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.state().selected_text(), editor.plain_text());
    });
}

#[gpui::test]
fn page_keys_keep_the_caret_row_on_screen(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(
        Document::from_blocks(
            (0..400)
                .map(|ix| Block::paragraph(&format!("Paragraph number {ix}")))
                .collect(),
        ),
        EditorConfig::full(),
        cx,
    );
    // Start somewhere in the middle of the viewport.
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.set_caret(Position::new(6, 3)), cx);
    });
    cx.run_until_parked();
    let offset_in_viewport = |editor: &Editor| {
        editor.caret_bounds().unwrap().top() - editor.viewport_bounds().unwrap().top()
    };
    let (before, page) = editor.read_with(cx, |editor, _| {
        (offset_in_viewport(editor), editor.rows_per_page().unwrap())
    });

    cx.simulate_keystrokes("pagedown");
    editor.read_with(cx, |editor, _| {
        assert_eq!(head(editor), Position::new(6 + page - 1, 3));
        assert!((offset_in_viewport(editor) - before).abs() < px(1.));
        assert!(editor.scroll_offset() > px(0.));
    });
    cx.simulate_keystrokes("pagedown shift-pagedown");
    editor.read_with(cx, |editor, _| {
        let selection = editor.selection();
        assert_eq!(selection.anchor, Position::new(6 + 2 * (page - 1), 3));
        assert_eq!(selection.head, Position::new(6 + 3 * (page - 1), 3));
        assert!((offset_in_viewport(editor) - before).abs() < px(1.));
    });
    // A plain page up leaves from the selection's start.
    cx.simulate_keystrokes("pageup pageup");
    editor.read_with(cx, |editor, _| {
        assert!(editor.selection().is_collapsed());
        assert_eq!(head(editor), Position::new(6, 3));
        assert!(editor.scroll_offset() < px(0.5));
    });
    // Fewer rows than a page are left: the caret goes as far as it can,
    // then to the very start.
    cx.simulate_keystrokes("pageup");
    editor.read_with(cx, |editor, _| {
        assert_eq!(head(editor), Position::new(0, 3))
    });
    cx.simulate_keystrokes("pageup");
    editor.read_with(cx, |editor, _| assert_eq!(head(editor), Position::ZERO));
}

/// Every block kind, alignment and indent, with rows that wrap.
fn varied() -> Document {
    let long = "The quick brown fox jumps over the lazy dog and keeps running. ".repeat(4);
    let long = long.trim_end();
    Document::from_blocks(vec![
        Block::with_text(
            BlockKind::heading(1),
            "A heading that is long enough to wrap in the column for sure",
        ),
        Block::paragraph(long),
        Block::paragraph(long).with_alignment(Alignment::Center),
        Block::paragraph(long).with_alignment(Alignment::Right),
        Block::paragraph(long).with_alignment(Alignment::Justify),
        Block::with_text(BlockKind::BulletItem, long).with_indent(2),
        Block::with_text(BlockKind::NumberedItem, "short"),
        Block::with_text(BlockKind::todo(), long),
        Block::with_text(BlockKind::Quote, long),
        Block::with_text(
            BlockKind::code(),
            "fn main() {\n    let wide = \"\u{65e5}\u{672c}\u{8a9e} and caf\u{e9}\";\n}",
        ),
        Block::new(BlockKind::Divider),
        Block::paragraph("soft\nbreaks\n\nin a block").with_text_size(Some(24.)),
        Block::paragraph(""),
        Block::paragraph("emoji \u{1f469}\u{200d}\u{1f469}\u{200d}\u{1f467} and e\u{301} combine"),
    ])
}

/// Position to point to position, for every caret position of the
/// document.
fn assert_hit_testing_round_trips(editor: &Editor) {
    let document = editor.document();
    let mut checked = 0;
    for (index, block) in document.blocks().iter().enumerate() {
        let content = block.text();
        let mut offset = 0;
        loop {
            let position = Position::new(index, offset);
            let bounds = editor
                .position_bounds(position)
                .unwrap_or_else(|| panic!("no bounds for {position:?}"));
            assert_eq!(bounds.size.width, px(0.));
            assert!(bounds.size.height > px(0.));
            assert_eq!(
                editor.position_at_point(bounds.center()),
                Some(position),
                "block {index} ({:?}) offset {offset}",
                block.kind()
            );
            // The row oracle agrees with the geometry.
            let row = editor.visual_row(position).unwrap();
            assert!(row.start <= position && position <= row.end);
            checked += 1;
            if offset >= content.len() {
                break;
            }
            offset = text::next_grapheme_boundary(content, offset);
        }
    }
    assert!(checked > 100);
}

#[gpui::test]
fn hit_testing_round_trips_in_flow_view(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(varied(), EditorConfig::full(), cx);
    editor.read_with(cx, |editor, _| assert_hit_testing_round_trips(editor));

    // Also when scrolled, and for positions that are off screen.
    editor.update(cx, |editor, cx| editor.scroll_by_rows(7, cx));
    cx.run_until_parked();
    editor.read_with(cx, |editor, _| {
        assert!(editor.scroll_offset() > px(0.));
        assert_hit_testing_round_trips(editor);
        // Rectangles of a range across rows and blocks: one per row, each
        // inside the column, top to bottom.
        let range = writer_core::TextRange::new(Position::new(1, 10), Position::new(3, 20));
        let rects = editor.range_bounds(range).unwrap();
        assert!(rects.len() >= 5);
        for pair in rects.windows(2) {
            assert!(pair[1].top() >= pair[0].top());
        }
        let all = editor
            .range_bounds(range)
            .unwrap()
            .into_iter()
            .reduce(|a, b| a.union(&b))
            .unwrap();
        let first = editor.position_bounds(range.start).unwrap();
        let last = editor.position_bounds(range.end).unwrap();
        assert_eq!(all.top(), first.top());
        assert_eq!(all.bottom(), last.bottom());
    });
}

#[gpui::test]
fn hit_testing_round_trips_in_page_view(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(varied(), EditorConfig::full(), cx);
    editor.update(cx, |editor, cx| editor.set_page_view(true, cx));
    cx.run_until_parked();
    editor.read_with(cx, |editor, _| {
        assert!(editor.page_count().unwrap() >= 2);
        assert_hit_testing_round_trips(editor);
    });
}

#[gpui::test]
fn justified_rows_fill_the_column(cx: &mut TestAppContext) {
    setup(cx);
    let long = "The quick brown fox jumps over the lazy dog and keeps running. ".repeat(4);
    let (editor, cx) = open(
        Document::from_blocks(vec![
            Block::paragraph(long.trim_end()),
            Block::paragraph(long.trim_end()).with_alignment(Alignment::Justify),
            Block::paragraph(long.trim_end()).with_alignment(Alignment::Right),
            Block::paragraph(long.trim_end()).with_alignment(Alignment::Center),
        ]),
        EditorConfig::full(),
        cx,
    );
    editor.read_with(cx, |editor, _| {
        // The visible end of a row: before the space it wraps at.
        let row_end = |block: usize| {
            let row = editor.visual_row(Position::new(block, 0)).unwrap();
            let text = &editor.document().blocks()[block].text()[..row.end.offset];
            Position::new(block, text.trim_end().len())
        };
        let left = |position: Position| editor.position_bounds(position).unwrap().left();
        let column_left = left(Position::new(0, 0));
        let ragged = left(row_end(0)) - column_left;
        let justified = left(row_end(1)) - column_left;
        // 700px column; the unjustified first row is shorter.
        assert!(ragged < px(699.));
        assert!((justified - px(700.)).abs() < px(0.5), "{justified:?}");
        assert_eq!(left(Position::new(1, 0)), column_left);
        // The last row of the paragraph is not stretched.
        let last = editor.document().blocks()[1].len();
        let last_row = editor.visual_row(Position::new(1, last)).unwrap();
        assert!(left(last_row.end) - column_left < px(699.));

        // Right-aligned and centred rows end at, or are centred in, the column.
        assert!((left(row_end(2)) - column_left - px(700.)).abs() < px(0.5));
        let centre_start = left(Position::new(3, 0)) - column_left;
        let centre_end = px(700.) - (left(row_end(3)) - column_left);
        assert!((centre_start - centre_end).abs() < px(0.5));
        assert!(centre_start > px(1.));
    });
}

#[gpui::test]
fn the_caret_stays_on_the_row_it_was_put_on(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(wrapping(), EditorConfig::full(), cx);
    let at = |cx: &mut gpui::VisualTestContext| editor.read_with(cx, |editor, _| head(editor));
    let caret = |cx: &mut gpui::VisualTestContext| {
        editor.read_with(cx, |editor, _| editor.caret_bounds().unwrap())
    };
    let line_box = |cx: &mut gpui::VisualTestContext, offset: usize| {
        editor.read_with(cx, |editor, _| {
            editor.position_bounds(Position::new(0, offset)).unwrap()
        })
    };
    let put = |cx: &mut gpui::VisualTestContext, offset: usize| {
        editor.update(cx, |editor, cx| {
            editor.edit(|state| state.set_caret(Position::new(0, offset)), cx);
        });
    };
    // The first paragraph: rows of 65 bytes ("word " thirteen times), the
    // last one shorter. Row n starts at 65 n.
    let rows: Vec<_> = (0..5).map(|row| line_box(cx, row * 65)).collect();
    for pair in rows.windows(2) {
        assert!(pair[1].top() > pair[0].top());
    }
    let column_left = rows[0].left();

    // Line end on a wrapped row: the position is where the next row
    // starts, the caret is drawn at the end of this one, right of the
    // row's last letter and inside the column.
    put(cx, 70);
    cx.simulate_keystrokes("cmd-right");
    assert_eq!(at(cx), Position::new(0, 130));
    assert_eq!(caret(cx).top(), rows[1].top());
    assert!(caret(cx).left() > line_box(cx, 129).left());
    assert!(caret(cx).right() <= column_left + px(701.));
    editor.read_with(cx, |editor, _| {
        // The position itself has no affinity: it starts the third row.
        assert_eq!(
            editor.visual_row(head(editor)).unwrap().start,
            Position::new(0, 130)
        );
        assert_eq!(
            editor.position_bounds(head(editor)).unwrap().top(),
            rows[2].top()
        );
    });
    // Pressing it again changes nothing; line start is this row's start.
    cx.simulate_keystrokes("cmd-right");
    assert_eq!(at(cx), Position::new(0, 130));
    assert_eq!(caret(cx).top(), rows[1].top());
    cx.simulate_keystrokes("cmd-left");
    assert_eq!(at(cx), Position::new(0, 65));
    assert_eq!(caret(cx).top(), rows[1].top());

    // Up and down from the end of a row go to the ends of the rows above
    // and below, and stay on them.
    cx.simulate_keystrokes("cmd-right down");
    assert_eq!(at(cx), Position::new(0, 195));
    assert_eq!(caret(cx).top(), rows[2].top());
    cx.simulate_keystrokes("up up");
    assert_eq!(at(cx), Position::new(0, 65));
    assert_eq!(caret(cx).top(), rows[0].top());
    // The last row of the paragraph ends at a real line end.
    cx.simulate_keystrokes("down down down down");
    assert_eq!(at(cx), Position::new(0, 299));
    assert_eq!(caret(cx).top(), rows[4].top());
    cx.simulate_keystrokes("up");
    assert_eq!(at(cx), Position::new(0, 260));
    assert_eq!(caret(cx).top(), rows[3].top());

    // Moving on by a character leaves the row end: the same position
    // reached from the right is the start of the next row.
    cx.simulate_keystrokes("right");
    assert_eq!(at(cx), Position::new(0, 261));
    cx.simulate_keystrokes("left");
    assert_eq!(at(cx), Position::new(0, 260));
    assert_eq!(caret(cx).top(), rows[4].top());
    assert_eq!(caret(cx).left() + px(1.), column_left);
    cx.simulate_keystrokes("cmd-left");
    assert_eq!(at(cx), Position::new(0, 260));

    // Text typed at the end of a row goes in at that position (before the
    // word that starts the next row), and the caret follows it.
    put(cx, 70);
    cx.simulate_keystrokes("cmd-right");
    cx.simulate_input("x");
    editor.read_with(cx, |editor, _| {
        assert_eq!(
            &editor.document().blocks()[0].text()[125..136],
            "word xword "
        );
        assert_eq!(head(editor), Position::new(0, 131));
    });
    assert_eq!(caret(cx).top(), rows[2].top());
    cx.simulate_keystrokes("backspace");
    assert_eq!(at(cx), Position::new(0, 130));
    assert_eq!(caret(cx).top(), rows[2].top());

    // Extending to the line end and on by rows.
    put(cx, 70);
    cx.simulate_keystrokes("cmd-shift-right shift-down");
    editor.read_with(cx, |editor, _| {
        assert_eq!(
            editor.selection(),
            Selection::new(Position::new(0, 70), Position::new(0, 195))
        );
    });
    cx.simulate_keystrokes("shift-up shift-up");
    editor.read_with(cx, |editor, _| {
        assert_eq!(
            editor.selection(),
            Selection::new(Position::new(0, 70), Position::new(0, 65))
        );
    });

    // Deleting to the line start from the end of a row takes that row;
    // deleting to the line end there takes nothing.
    put(cx, 70);
    cx.simulate_keystrokes("cmd-right cmd-delete");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().blocks()[0].len(), 299);
        assert_eq!(head(editor), Position::new(0, 130));
    });
    cx.simulate_keystrokes("cmd-backspace");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().blocks()[0].len(), 299 - 65);
        assert_eq!(head(editor), Position::new(0, 65));
    });
    assert_eq!(caret(cx).top(), rows[1].top());
    cx.simulate_keystrokes("cmd-z");

    // A block caret sits on a character and has no affinity: line end is
    // the row's last character, as before.
    editor.update(cx, |editor, cx| {
        editor.set_caret_style(crate::CaretStyle::Block, cx)
    });
    put(cx, 70);
    cx.simulate_keystrokes("cmd-right");
    assert_eq!(at(cx), Position::new(0, 129));
    assert_eq!(caret(cx).top(), rows[1].top());
    cx.simulate_keystrokes("down");
    assert_eq!(at(cx), Position::new(0, 194));
    // So does a caret that is drawn somewhere else.
    editor.update(cx, |editor, cx| {
        editor.set_caret_style(crate::CaretStyle::Bar, cx);
        editor.set_caret_override(Some(Position::new(0, 3)), cx);
    });
    put(cx, 70);
    cx.simulate_keystrokes("cmd-right");
    assert_eq!(at(cx), Position::new(0, 129));
}

#[gpui::test]
fn the_caret_affinity_holds_in_page_view_and_for_scrolling(cx: &mut TestAppContext) {
    setup(cx);
    let long = "word ".repeat(400);
    let (editor, cx) = open(
        Document::from_blocks(vec![Block::paragraph(long.trim_end())]),
        EditorConfig::full(),
        cx,
    );
    editor.update(cx, |editor, cx| editor.set_page_view(true, cx));
    cx.run_until_parked();
    // Go down row by row at the far right: the caret is at the end of
    // every row it passes, on that row, and stays in view.
    cx.simulate_keystrokes("cmd-right");
    let mut last = None;
    for _ in 0..40 {
        let (position, caret, row_top, viewport) = editor.read_with(cx, |editor, _| {
            let position = head(editor);
            let before = Position::new(0, position.offset - 1);
            (
                position,
                editor.caret_bounds().unwrap(),
                editor.position_bounds(before).unwrap().top(),
                editor.viewport_bounds().unwrap(),
            )
        });
        assert_eq!(position.offset % 5, 0, "after a space: {position:?}");
        assert_eq!(caret.top(), row_top, "on the row that ends at {position:?}");
        assert!(caret.top() >= viewport.top() && caret.bottom() <= viewport.bottom());
        assert!(caret.right() <= viewport.right());
        assert!(
            last.is_none_or(|last| position > last),
            "a row further down"
        );
        last = Some(position);
        cx.simulate_keystrokes("down");
    }
    editor.read_with(cx, |editor, _| {
        assert!(editor.scroll_offset() > px(0.));
        // The page of the caret is the page of the row it is on.
        let page = editor.caret_page().unwrap();
        assert!(page >= 1 && page <= editor.page_count().unwrap());
    });
}

#[gpui::test]
fn the_emacs_keys_work_on_the_paragraph(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(wrapping(), EditorConfig::full(), cx);
    let at = |cx: &mut gpui::VisualTestContext| editor.read_with(cx, |editor, _| head(editor));
    let put = |cx: &mut gpui::VisualTestContext, block: usize, offset: usize| {
        editor.update(cx, |editor, cx| {
            editor.edit(|state| state.set_caret(Position::new(block, offset)), cx);
        });
    };

    // In a paragraph that wraps into five rows, ctrl-a and ctrl-e go to
    // the ends of the paragraph; cmd-left and cmd-right stay on the row.
    put(cx, 0, 140);
    cx.simulate_keystrokes("ctrl-a");
    assert_eq!(at(cx), Position::ZERO);
    put(cx, 0, 140);
    cx.simulate_keystrokes("cmd-left");
    assert_eq!(at(cx), Position::new(0, 130));
    cx.simulate_keystrokes("ctrl-e");
    assert_eq!(at(cx), Position::new(0, 299));
    put(cx, 0, 140);
    cx.simulate_keystrokes("cmd-right");
    assert_eq!(at(cx), Position::new(0, 195));
    // At the ends they stay.
    cx.simulate_keystrokes("ctrl-e ctrl-e");
    assert_eq!(at(cx), Position::new(0, 299));
    cx.simulate_keystrokes("ctrl-a ctrl-a");
    assert_eq!(at(cx), Position::ZERO);

    // With shift they select.
    put(cx, 0, 140);
    cx.simulate_keystrokes("ctrl-shift-a");
    editor.read_with(cx, |editor, _| {
        assert_eq!(
            editor.selection(),
            Selection::new(Position::new(0, 140), Position::ZERO)
        );
    });
    put(cx, 0, 140);
    cx.simulate_keystrokes("ctrl-shift-e");
    editor.read_with(cx, |editor, _| {
        assert_eq!(
            editor.selection(),
            Selection::new(Position::new(0, 140), Position::new(0, 299))
        );
    });

    // ctrl-k deletes to the end of the paragraph, over every row of it;
    // at the end it takes the paragraph break. cmd-delete stops at the
    // end of the row.
    put(cx, 0, 140);
    cx.simulate_keystrokes("cmd-delete");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().blocks()[0].len(), 299 - 55);
    });
    cx.simulate_keystrokes("cmd-z");
    put(cx, 0, 140);
    cx.simulate_keystrokes("ctrl-k");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().block_count(), 3);
        assert_eq!(editor.document().blocks()[0].len(), 140);
        assert_eq!(head(editor), Position::new(0, 140));
    });
    cx.simulate_keystrokes("ctrl-k");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().block_count(), 2);
        assert!(editor.document().blocks()[0].text().ends_with("word short"));
        assert_eq!(head(editor), Position::new(0, 140));
    });
    cx.simulate_keystrokes("cmd-z cmd-z");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().block_count(), 3);
        assert_eq!(editor.document().blocks()[0].len(), 299);
    });
    // Read-only: nothing is deleted.
    editor.update(cx, |editor, cx| editor.set_read_only(true, cx));
    put(cx, 0, 140);
    cx.simulate_keystrokes("ctrl-k");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().blocks()[0].len(), 299);
    });
}

#[gpui::test]
fn the_paragraph_keys_stop_at_line_breaks_inside_a_block(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(
        Document::from_blocks(vec![
            Block::paragraph("one\ntwo three\nfour"),
            Block::with_text(BlockKind::code(), "fn main() {\n    body();\n}"),
        ]),
        EditorConfig::full(),
        cx,
    );
    let at = |cx: &mut gpui::VisualTestContext| editor.read_with(cx, |editor, _| head(editor));
    let put = |cx: &mut gpui::VisualTestContext, block: usize, offset: usize| {
        editor.update(cx, |editor, cx| {
            editor.edit(|state| state.set_caret(Position::new(block, offset)), cx);
        });
    };
    // A soft line break ends the line the keys work on.
    put(cx, 0, 7);
    cx.simulate_keystrokes("ctrl-a");
    assert_eq!(at(cx), Position::new(0, 4));
    cx.simulate_keystrokes("ctrl-e");
    assert_eq!(at(cx), Position::new(0, 13));
    put(cx, 0, 7);
    cx.simulate_keystrokes("ctrl-k");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().blocks()[0].text(), "one\ntwo\nfour");
    });
    // At the end of the line: the break itself.
    cx.simulate_keystrokes("ctrl-k");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().blocks()[0].text(), "one\ntwofour");
        assert_eq!(head(editor), Position::new(0, 7));
    });
    // Lines of a code block.
    put(cx, 1, 18);
    cx.simulate_keystrokes("ctrl-a");
    assert_eq!(at(cx), Position::new(1, 12));
    cx.simulate_keystrokes("ctrl-e");
    assert_eq!(at(cx), Position::new(1, 23));
}
