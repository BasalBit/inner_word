//! Spelling marks: what a checker sets, how the marks follow edits, what
//! the queries answer, where the waves are drawn, what the context menu
//! request carries, and what all of it costs in a book.

// A list of one range is what these tests mean by `[2..5]`.
#![allow(clippy::single_range_in_vec_init)]

use std::{ops::Range, time::Instant};

use gpui::{TestAppContext, VisualTestContext, px};
use innerword_core::{
    Block, BlockId, BlockKind, Document, DocumentSettings, InlineAttrs, Position, Selection,
    TextRange,
};

use super::{
    mouse::{point_of, record_events, request_in, right_click},
    slice::{open, setup},
};
use crate::{Editor, EditorConfig, SpellingMarksOutcome, paint::snap};

/// The words the stand-in checker of these tests does not know.
const MISSPELLED: [&str; 6] = ["teh", "wrld", "naïf", "recieve", "adress", "zzz"];

/// Byte ranges of the misspelled words of `text`. A word is a run of
/// letters, digits and apostrophes, as for the editor's own rule of what
/// continues a word.
fn misspelled(text: &str) -> Vec<Range<usize>> {
    let mut found = Vec::new();
    let mut start = None;
    let word_char = |c: char| c.is_alphanumeric() || c == '\'' || c == '\u{2019}';
    for (offset, c) in text.char_indices().chain([(text.len(), ' ')]) {
        match (start, word_char(c)) {
            (None, true) => start = Some(offset),
            (Some(from), false) => {
                if MISSPELLED.contains(&&text[from..offset]) {
                    found.push(from..offset);
                }
                start = None;
            }
            _ => {}
        }
    }
    found
}

/// Checks block `index` as a checker would: its text at its revision.
fn check_block(
    editor: &gpui::Entity<Editor>,
    index: usize,
    cx: &mut VisualTestContext,
) -> SpellingMarksOutcome {
    editor.update(cx, |editor, cx| {
        let block = &editor.document().blocks()[index];
        let (id, revision, ranges) = (block.id(), block.revision(), misspelled(block.text()));
        editor.set_spelling_marks(id, revision, ranges, cx)
    })
}

fn check_all(editor: &gpui::Entity<Editor>, cx: &mut VisualTestContext) {
    let count = editor.read_with(cx, |editor, _| editor.document().block_count());
    for index in 0..count {
        assert!(check_block(editor, index, cx).is_applied());
    }
    cx.run_until_parked();
}

/// The marked words of block `index`, as text.
fn marked(editor: &Editor, index: usize) -> Vec<String> {
    let text = editor.document().blocks()[index].text();
    editor
        .spelling_marks(index)
        .iter()
        .map(|range| text[range.clone()].to_string())
        .collect()
}

/// The store is in order, and every mark is a word the checker would mark
/// again: a mark never survives on text that changed under it.
fn assert_sound(editor: &Editor) {
    editor.spelling.check_invariants(editor.document()).unwrap();
    for (index, block) in editor.document().blocks().iter().enumerate() {
        let fresh = misspelled(block.text());
        for range in editor.spelling_marks(index) {
            assert!(
                fresh.contains(range),
                "block {index} {:?}: {range:?} is marked, a check gives {fresh:?}",
                block.text()
            );
        }
    }
}

fn caret(editor: &gpui::Entity<Editor>, block: usize, offset: usize, cx: &mut VisualTestContext) {
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.set_caret(Position::new(block, offset)), cx)
    });
}

fn mark(block: usize, start: usize, end: usize) -> Option<TextRange> {
    Some(TextRange::in_block(block, start, end))
}

#[gpui::test]
fn a_result_is_stored_for_the_revision_it_was_made_for(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(
        Document::from_blocks(vec![
            Block::paragraph("I saw teh wrld"),
            Block::paragraph("nothing wrong here"),
            Block::paragraph("an adress, zzz"),
        ]),
        EditorConfig::full(),
        cx,
    );
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.spelling_mark_count(), 0);
        assert!(!editor.spelling_checked(0));
        assert_eq!(editor.first_unchecked_spelling_block(0..3), Some(0));
        assert_eq!(editor.next_spelling_mark(Position::new(0, 0)), None);
        assert_eq!(editor.previous_spelling_mark(Position::new(0, 0)), None);
        assert!(editor.spelling_marks_visible());
    });

    assert_eq!(check_block(&editor, 0, cx), SpellingMarksOutcome::Applied);
    assert_eq!(check_block(&editor, 2, cx), SpellingMarksOutcome::Applied);
    editor.read_with(cx, |editor, _| {
        assert_eq!(marked(editor, 0), ["teh", "wrld"]);
        assert_eq!(editor.spelling_marks(0), [6..9, 10..14]);
        assert_eq!(marked(editor, 2), ["adress", "zzz"]);
        assert_eq!(editor.spelling_mark_count(), 4);
        assert!(editor.spelling_checked(0) && editor.spelling_checked(2));
        assert!(!editor.spelling_checked(1));
        assert!(!editor.spelling_checked(99));
        assert!(editor.spelling_marks(99).is_empty());
        assert_eq!(editor.first_unchecked_spelling_block(0..3), Some(1));
        assert_eq!(editor.first_unchecked_spelling_block(2..3), None);
        assert_eq!(editor.first_unchecked_spelling_block(2..99), None);
    });
    // An empty result is a result.
    assert!(check_block(&editor, 1, cx).is_applied());
    editor.read_with(cx, |editor, _| {
        assert!(editor.spelling_checked(1));
        assert_eq!(editor.first_unchecked_spelling_block(0..3), None);
        assert_eq!(editor.spelling_mark_count(), 4);
        assert_sound(editor);
    });

    // A result for a revision the block has left, or for a block that is
    // not there, is not stored, and the call says so.
    let (id, old_revision) = editor.read_with(cx, |editor, _| {
        let block = &editor.document().blocks()[0];
        (block.id(), block.revision())
    });
    caret(&editor, 0, 0, cx);
    cx.simulate_input("x");
    editor.update(cx, |editor, cx| {
        assert_eq!(
            editor.set_spelling_marks(id, old_revision, [0..1], cx),
            SpellingMarksOutcome::Stale
        );
        assert_eq!(
            editor.set_spelling_marks(BlockId(987_654), 1, [0..1], cx),
            SpellingMarksOutcome::Missing
        );
        // The marks have followed the edit meanwhile, unchecked.
        assert_eq!(editor.spelling_marks(0), [7..10, 11..15]);
        assert!(!editor.spelling_checked(0));
        assert_eq!(editor.spelling_mark_count(), 4);
        assert_sound(editor);
    });
    // Ranges that do not fit are left out, overlapping ones become one.
    editor.update(cx, |editor, cx| {
        let block = &editor.document().blocks()[2];
        let (id, revision) = (block.id(), block.revision());
        let outcome =
            editor.set_spelling_marks(id, revision, [11..14, 3..5, 4..9, 14..14, 12..99], cx);
        assert!(outcome.is_applied());
        assert_eq!(editor.spelling_marks(2), [3..9, 11..14]);
        assert_eq!(editor.spelling_mark_count(), 4);
    });

    // Invalidating keeps the marks and makes every block unchecked;
    // clearing drops both.
    editor.update(cx, |editor, cx| {
        editor.invalidate_spelling_marks();
        assert_eq!(editor.spelling_mark_count(), 4);
        assert_eq!(editor.first_unchecked_spelling_block(0..3), Some(0));
        assert!(!editor.spelling_checked(1) && !editor.spelling_checked(2));
        editor.clear_spelling_marks(cx);
        assert_eq!(editor.spelling_mark_count(), 0);
        assert!(editor.spelling_marks(0).is_empty());
        assert!(!editor.spelling_checked(1));
        assert_eq!(editor.next_spelling_mark(Position::new(0, 0)), None);
        editor.spelling.check_invariants(editor.document()).unwrap();
    });

    // A new document starts without marks.
    check_all(&editor, cx);
    editor.update(cx, |editor, cx| {
        assert!(editor.spelling_mark_count() > 0);
        editor.set_document(Document::from_blocks(vec![Block::paragraph("teh")]), cx);
        assert_eq!(editor.spelling_mark_count(), 0);
        assert!(!editor.spelling_checked(0));
        editor.spelling.check_invariants(editor.document()).unwrap();
    });
    assert!(check_block(&editor, 0, cx).is_applied());
    editor.read_with(cx, |editor, _| assert_eq!(marked(editor, 0), ["teh"]));
}

#[gpui::test]
fn the_mark_at_a_position_and_the_next_and_previous_one(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(
        Document::from_blocks(vec![
            Block::paragraph("fine"),
            Block::paragraph("I saw teh wrld"),
            Block::new(BlockKind::Divider),
            Block::paragraph("fine again"),
            Block::paragraph("zzz at the start, adress"),
            Block::paragraph("the end"),
        ]),
        EditorConfig::full(),
        cx,
    );
    check_all(&editor, cx);
    editor.read_with(cx, |editor, _| {
        // In the word, and at both of its ends.
        for offset in 6..=9 {
            assert_eq!(
                editor.spelling_mark_at(Position::new(1, offset)),
                mark(1, 6, 9)
            );
        }
        assert_eq!(editor.spelling_mark_at(Position::new(1, 5)), None);
        for offset in 10..=14 {
            assert_eq!(
                editor.spelling_mark_at(Position::new(1, offset)),
                mark(1, 10, 14)
            );
        }
        assert_eq!(editor.spelling_mark_at(Position::new(4, 0)), mark(4, 0, 3));
        assert_eq!(editor.spelling_mark_at(Position::new(4, 4)), None);
        assert_eq!(editor.spelling_mark_at(Position::new(0, 2)), None);
        assert_eq!(editor.spelling_mark_at(Position::new(2, 0)), None);
        assert_eq!(editor.spelling_mark_at(Position::new(77, 0)), None);

        // Next: the first that starts at or after the position.
        let next = |block, offset| editor.next_spelling_mark(Position::new(block, offset));
        assert_eq!(next(0, 0), mark(1, 6, 9));
        assert_eq!(next(1, 6), mark(1, 6, 9));
        assert_eq!(next(1, 7), mark(1, 10, 14));
        assert_eq!(next(1, 9), mark(1, 10, 14));
        assert_eq!(next(1, 14), mark(4, 0, 3));
        assert_eq!(next(2, 0), mark(4, 0, 3));
        assert_eq!(next(4, 3), mark(4, 18, 24));
        // Round the end of the document.
        assert_eq!(next(4, 24), mark(1, 6, 9));
        assert_eq!(next(5, 7), mark(1, 6, 9));
        assert_eq!(next(4, 19), mark(1, 6, 9));
        assert_eq!(next(99, 0), mark(1, 6, 9));

        // Previous: the last that starts before the position.
        let previous = |block, offset| editor.previous_spelling_mark(Position::new(block, offset));
        assert_eq!(previous(5, 7), mark(4, 18, 24));
        assert_eq!(previous(4, 18), mark(4, 0, 3));
        assert_eq!(previous(4, 20), mark(4, 18, 24));
        assert_eq!(previous(4, 0), mark(1, 10, 14));
        assert_eq!(previous(1, 10), mark(1, 6, 9));
        assert_eq!(previous(1, 9), mark(1, 6, 9));
        // Round the start of the document.
        assert_eq!(previous(1, 6), mark(4, 18, 24));
        assert_eq!(previous(0, 0), mark(4, 18, 24));
    });

    // Walking forwards from the selection's end and backwards from its
    // start visits every mark once and comes round.
    let all = [
        mark(1, 6, 9),
        mark(1, 10, 14),
        mark(4, 0, 3),
        mark(4, 18, 24),
    ];
    caret(&editor, 0, 0, cx);
    for step in 0..9 {
        let found = editor.update(cx, |editor, cx| {
            let found = editor.next_spelling_mark(editor.selection().range().end);
            editor.edit(
                |state| state.set_selection(Selection::from_range(found.unwrap())),
                cx,
            );
            found
        });
        assert_eq!(found, all[step % 4]);
    }
    for step in 0..9 {
        let found = editor.update(cx, |editor, cx| {
            let found = editor.previous_spelling_mark(editor.selection().range().start);
            editor.edit(
                |state| state.set_selection(Selection::from_range(found.unwrap())),
                cx,
            );
            found
        });
        assert_eq!(found, all[(8 - step + 3) % 4]);
    }

    // A single mark is its own next and previous.
    editor.update(cx, |editor, cx| {
        editor.clear_spelling_marks(cx);
        let block = &editor.document().blocks()[1];
        let (id, revision) = (block.id(), block.revision());
        editor.set_spelling_marks(id, revision, [6..9], cx);
        for position in [
            Position::new(0, 0),
            Position::new(1, 6),
            Position::new(1, 9),
            Position::new(5, 3),
        ] {
            assert_eq!(editor.next_spelling_mark(position), mark(1, 6, 9));
            assert_eq!(editor.previous_spelling_mark(position), mark(1, 6, 9));
        }
    });
}

#[gpui::test]
fn marks_follow_typing_character_by_character(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(
        Document::from_blocks(vec![Block::paragraph("I saw teh wrld today")]),
        EditorConfig::full()
            .input_rules(false)
            .smart_punctuation(false),
        cx,
    );
    check_all(&editor, cx);
    let marks = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |editor, _| {
            assert_sound(editor);
            editor.spelling_marks(0).to_vec()
        })
    };
    assert_eq!(marks(cx), [6..9, 10..14]);

    // Typing before both: they shift with every character.
    caret(&editor, 0, 0, cx);
    for (typed, ch) in "Oh, ".chars().enumerate() {
        cx.simulate_input(&ch.to_string());
        let by = typed + 1;
        assert_eq!(marks(cx), [6 + by..9 + by, 10 + by..14 + by]);
        editor.read_with(cx, |editor, _| assert!(!editor.spelling_checked(0)));
    }
    // "Oh, I saw teh wrld today": a space after the first word leaves it
    // marked and moves the second.
    caret(&editor, 0, 13, cx);
    cx.simulate_input(" ");
    assert_eq!(marks(cx), [10..13, 15..19]);
    // A letter between the two spaces adjoins neither word.
    cx.simulate_input("b");
    assert_eq!(marks(cx), [10..13, 16..20]);
    // A letter right before the second word: that word is being edited.
    cx.simulate_keystrokes("right");
    cx.simulate_input("i");
    assert_eq!(marks(cx), [10..13]);
    // Deleting back to the first word keeps its mark...
    cx.simulate_keystrokes("backspace backspace backspace");
    editor.read_with(cx, |editor, _| {
        assert_eq!(
            editor.document().blocks()[0].text(),
            "Oh, I saw teh wrld today"
        );
    });
    assert_eq!(marks(cx), [10..13]);
    // ...until the space that parts it from the next word goes.
    cx.simulate_keystrokes("backspace");
    assert!(marks(cx).is_empty());
    // And a letter deleted from inside a word takes its mark.
    assert!(check_block(&editor, 0, cx).is_applied());
    editor.update(cx, |editor, cx| {
        editor.edit(
            |state| {
                state.select_all();
                state.insert_text("a wrld b teh")
            },
            cx,
        )
    });
    assert!(check_block(&editor, 0, cx).is_applied());
    assert_eq!(marks(cx), [2..6, 9..12]);
    caret(&editor, 0, 4, cx);
    cx.simulate_keystrokes("backspace");
    assert_eq!(marks(cx), [8..11]);
    cx.simulate_keystrokes("cmd-right backspace");
    assert!(marks(cx).is_empty());

    // A fresh check, then punctuation and spaces at the edges of a mark.
    editor.update(cx, |editor, cx| {
        editor.edit(
            |state| {
                state.select_all();
                state.insert_text("a teh b")
            },
            cx,
        )
    });
    assert!(check_block(&editor, 0, cx).is_applied());
    assert_eq!(marks(cx), [2..5]);
    caret(&editor, 0, 5, cx);
    cx.simulate_input(".");
    assert_eq!(marks(cx), [2..5]);
    cx.simulate_input(" ");
    assert_eq!(marks(cx), [2..5]);
    caret(&editor, 0, 2, cx);
    cx.simulate_input(" ");
    assert_eq!(marks(cx), [3..6]);
    cx.simulate_input("(");
    assert_eq!(marks(cx), [4..7]);
    // A letter at its start: gone. Undo brings the text back, not the
    // mark: the block is unchecked, the checker sets it again.
    cx.simulate_input("s");
    assert!(marks(cx).is_empty());
    cx.simulate_keystrokes("cmd-z");
    cx.run_until_parked();
    editor.read_with(cx, |editor, _| assert!(!editor.spelling_checked(0)));
    assert_sound_after_recheck(&editor, cx);
}

/// After a new check every block is checked and the marks are exactly the
/// checker's.
fn assert_sound_after_recheck(editor: &gpui::Entity<Editor>, cx: &mut VisualTestContext) {
    check_all(editor, cx);
    editor.read_with(cx, |editor, _| {
        assert_sound(editor);
        let mut total = 0;
        for (index, block) in editor.document().blocks().iter().enumerate() {
            assert!(editor.spelling_checked(index));
            assert_eq!(editor.spelling_marks(index), misspelled(block.text()));
            total += editor.spelling_marks(index).len();
        }
        assert_eq!(editor.spelling_mark_count(), total);
    });
}

#[gpui::test]
fn marks_follow_edits_in_multi_byte_text(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(
        Document::from_blocks(vec![Block::paragraph("é naïf ö, teh 日本 zzz")]),
        EditorConfig::full()
            .input_rules(false)
            .smart_punctuation(false),
        cx,
    );
    check_all(&editor, cx);
    let words = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |editor, _| {
            assert_sound(editor);
            marked(editor, 0)
        })
    };
    assert_eq!(words(cx), ["naïf", "teh", "zzz"]);
    // Multi-byte characters typed and deleted before, between and after.
    caret(&editor, 0, 0, cx);
    for ch in ["ü", "日", "🙂", " "] {
        cx.simulate_input(ch);
        assert_eq!(words(cx), ["naïf", "teh", "zzz"]);
    }
    cx.simulate_keystrokes("backspace backspace backspace backspace");
    assert_eq!(words(cx), ["naïf", "teh", "zzz"]);
    // Replacing é by è (one byte differs) in front of the first mark.
    cx.simulate_keystrokes("shift-right");
    cx.simulate_input("è");
    assert_eq!(words(cx), ["naïf", "teh", "zzz"]);
    // The ï inside the word replaced.
    editor.update(cx, |editor, cx| {
        let text = editor.document().blocks()[0].text().to_string();
        let at = text.find('ï').unwrap();
        editor.edit(
            |state| {
                state.set_selection(Selection::from_range(TextRange::in_block(0, at, at + 2)));
                state.insert_text("i")
            },
            cx,
        );
    });
    assert_eq!(words(cx), ["teh", "zzz"]);
    // A CJK character (a letter) right after "teh" continues the word; an
    // emoji right before "zzz" does not.
    editor.update(cx, |editor, cx| {
        let text = editor.document().blocks()[0].text().to_string();
        let at = text.find("teh").unwrap() + 3;
        editor.edit(
            |state| {
                state.set_caret(Position::new(0, at));
                state.insert_text("本")
            },
            cx,
        );
    });
    assert_eq!(words(cx), ["zzz"]);
    editor.update(cx, |editor, cx| {
        let text = editor.document().blocks()[0].text().to_string();
        let at = text.find("zzz").unwrap();
        editor.edit(
            |state| {
                state.set_caret(Position::new(0, at));
                state.insert_text("🙂")
            },
            cx,
        );
    });
    assert_eq!(words(cx), ["zzz"]);
    assert_sound_after_recheck(&editor, cx);
}

#[gpui::test]
fn splitting_and_joining_blocks_keeps_the_marks(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(
        Document::from_blocks(vec![
            Block::paragraph("first teh line"),
            Block::paragraph("I saw teh wrld today, zzz"),
            Block::paragraph("adress last"),
        ]),
        EditorConfig::full()
            .input_rules(false)
            .smart_punctuation(false),
        cx,
    );
    check_all(&editor, cx);
    let words = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |editor, _| {
            assert_sound(editor);
            (0..editor.document().block_count())
                .map(|index| marked(editor, index))
                .collect::<Vec<_>>()
        })
    };
    let before = words(cx);
    assert_eq!(
        before,
        [vec!["teh"], vec!["teh", "wrld", "zzz"], vec!["adress"]]
    );

    // Enter between "teh" and "wrld": the marks behind go to the new block.
    caret(&editor, 1, 10, cx);
    cx.simulate_keystrokes("enter");
    assert_eq!(
        words(cx),
        [
            vec!["teh"],
            vec!["teh"],
            vec!["wrld", "zzz"],
            vec!["adress"]
        ]
    );
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.spelling_marks(2), [0..4, 12..15]);
        assert_eq!(editor.spelling_mark_count(), 5);
        assert!(editor.spelling_checked(0) && editor.spelling_checked(3));
        assert!(!editor.spelling_checked(1) && !editor.spelling_checked(2));
    });
    // Backspace joins them again: the marks come back to the first.
    cx.simulate_keystrokes("backspace");
    assert_eq!(words(cx), before);
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.spelling_marks(1), [6..9, 10..14, 22..25]);
        assert_eq!(editor.spelling_mark_count(), 5);
    });
    // Undo and redo of the split and of the join.
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(words(cx).len(), 4);
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(words(cx).len(), 3);
    cx.simulate_keystrokes("cmd-shift-z cmd-shift-z");
    assert_eq!(words(cx).len(), 3);
    assert_sound_after_recheck(&editor, cx);

    // Enter inside a marked word: the word is two words now.
    caret(&editor, 1, 12, cx);
    cx.simulate_keystrokes("enter");
    assert_eq!(
        words(cx),
        [vec!["teh"], vec!["teh"], vec!["zzz"], vec!["adress"]]
    );
    cx.simulate_keystrokes("cmd-z");
    assert_sound_after_recheck(&editor, cx);

    // Joining so that two words become one drops the marks at the seam:
    // "first teh line" + "I saw ..." stays apart (a space is typed first),
    // "…zzz" + "adress last" becomes "zzzadress".
    caret(&editor, 2, 0, cx);
    cx.simulate_keystrokes("backspace");
    assert_eq!(words(cx), [vec!["teh"], vec!["teh", "wrld"]]);
    cx.simulate_keystrokes("cmd-z");
    assert_sound_after_recheck(&editor, cx);
    caret(&editor, 1, 25, cx);
    cx.simulate_input(" ");
    cx.simulate_keystrokes("delete");
    assert_eq!(
        words(cx),
        [vec!["teh"], vec!["teh", "wrld", "zzz", "adress"]]
    );
    assert_sound_after_recheck(&editor, cx);

    // Deleting a selection that spans blocks: what is left of the last
    // block keeps its marks, the blocks that went lose theirs.
    editor.update(cx, |editor, cx| {
        editor.set_document(
            Document::from_blocks(vec![
                Block::paragraph("one teh two"),
                Block::paragraph("gone wrld gone"),
                Block::paragraph("three zzz four adress"),
            ]),
            cx,
        )
    });
    check_all(&editor, cx);
    editor.update(cx, |editor, cx| {
        editor.edit(
            |state| {
                state.set_selection(Selection::new(Position::new(0, 8), Position::new(2, 6)));
                state.backspace()
            },
            cx,
        );
    });
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().block_count(), 1);
        assert_eq!(
            editor.document().blocks()[0].text(),
            "one teh zzz four adress"
        );
        assert_eq!(editor.spelling_mark_count(), 3);
    });
    assert_eq!(words(cx), [vec!["teh", "zzz", "adress"]]);
    // Undo: the three blocks are back; what the checker has not seen
    // again is unchecked, and nothing is marked that should not be.
    cx.simulate_keystrokes("cmd-z");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().block_count(), 3);
        assert_sound(editor);
        assert!(!editor.spelling_checked(1));
    });
    assert_sound_after_recheck(&editor, cx);

    // Moving a block keeps its marks and its result.
    caret(&editor, 2, 0, cx);
    cx.simulate_keystrokes("alt-up");
    editor.read_with(cx, |editor, _| {
        assert_eq!(marked(editor, 1), ["zzz", "adress"]);
        assert_eq!(marked(editor, 2), ["wrld"]);
        assert!(editor.spelling_checked(1) && editor.spelling_checked(2));
        assert_sound(editor);
    });
}

#[gpui::test]
fn undo_and_redo_never_leave_a_wrong_mark(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(
        Document::from_blocks(vec![
            Block::paragraph("I saw teh wrld today"),
            Block::paragraph("an adress and zzz"),
        ]),
        EditorConfig::full()
            .input_rules(false)
            .smart_punctuation(false),
        cx,
    );
    check_all(&editor, cx);
    // A run of edits of every kind, checked now and then as a checker
    // with a pause would.
    let steps: [&dyn Fn(&mut VisualTestContext); 10] = [
        &|cx| cx.simulate_input("x"),
        &|cx| cx.simulate_keystrokes("alt-right alt-right backspace"),
        &|cx| cx.simulate_keystrokes("enter"),
        &|cx| cx.simulate_keystrokes("cmd-b"),
        &|cx| cx.simulate_keystrokes("shift-alt-right cmd-i"),
        &|cx| cx.simulate_keystrokes("down cmd-left delete delete"),
        &|cx| cx.simulate_keystrokes("cmd-a cmd-shift-8"),
        &|cx| cx.simulate_keystrokes("up cmd-right enter"),
        &|cx| cx.simulate_input("teh zzz"),
        &|cx| cx.simulate_keystrokes("alt-up"),
    ];
    caret(&editor, 0, 3, cx);
    for (ix, step) in steps.iter().enumerate() {
        step(cx);
        cx.run_until_parked();
        editor.read_with(cx, |editor, _| assert_sound(editor));
        if ix % 3 == 2 {
            check_all(&editor, cx);
        }
    }
    // All the way back and forth again; a block whose old revision comes
    // back is not taken for checked.
    for keys in ["cmd-z", "cmd-shift-z", "cmd-z"] {
        for _ in 0..steps.len() + 4 {
            cx.simulate_keystrokes(keys);
            cx.run_until_parked();
            editor.read_with(cx, |editor, _| {
                assert_sound(editor);
                for index in 0..editor.document().block_count() {
                    if editor.spelling_checked(index) {
                        let block = &editor.document().blocks()[index];
                        assert_eq!(editor.spelling_marks(index), misspelled(block.text()));
                    }
                }
            });
        }
    }
    editor.read_with(cx, |editor, _| {
        assert_eq!(
            editor.document().plain_text(),
            "I saw teh wrld today\nan adress and zzz"
        );
    });
    assert_sound_after_recheck(&editor, cx);
}

/// A few thousand random edits; after each one the store is in order and
/// no mark sits on text a checker would not mark.
#[gpui::test]
fn random_edits_keep_the_marks_sound(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(
        Document::from_blocks(vec![
            Block::paragraph("I saw teh wrld today, naïf as ever"),
            Block::with_text(BlockKind::heading(2), "An adress"),
            Block::paragraph("zzz recieve teh teh. é ö 日本"),
        ]),
        EditorConfig::full()
            .input_rules(false)
            .smart_punctuation(false),
        cx,
    );
    check_all(&editor, cx);
    let mut seed = 0x2545_f491_4f6c_dd1d_u64;
    let mut random = |below: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % below as u64) as usize
    };
    let pieces = [
        "a", " ", "teh", "é", ".", "zzz ", "'", "日", "wrld", "\u{301}", "x y", "🙂",
    ];
    for round in 0..1500 {
        let action = random(17);
        editor.update(cx, |editor, cx| {
            let document = editor.document();
            let block = random(document.block_count());
            let text = document.blocks()[block].text();
            let mut boundaries: Vec<usize> = text.char_indices().map(|(at, _)| at).collect();
            boundaries.push(text.len());
            let a = boundaries[random(boundaries.len())];
            let b = boundaries[random(boundaries.len())];
            let piece = pieces[random(pieces.len())];
            editor.edit(
                |state| match action {
                    0..=5 => {
                        state.set_caret(Position::new(block, a));
                        state.insert_text(piece);
                    }
                    6 | 7 => {
                        state.set_caret(Position::new(block, a));
                        state.backspace();
                    }
                    8 => {
                        state.set_caret(Position::new(block, a));
                        state.delete_forward();
                    }
                    9 => {
                        state.set_selection(Selection::new(
                            Position::new(block, a.min(b)),
                            Position::new(block, a.max(b)),
                        ));
                        state.insert_text(piece);
                    }
                    10 => {
                        state.set_caret(Position::new(block, a));
                        state.insert_paragraph_break();
                    }
                    11 => {
                        let other = random(state.document().block_count());
                        let (from, to) = if other < block {
                            (Position::new(other, 0), Position::new(block, a))
                        } else {
                            (
                                Position::new(block, a),
                                state.document().block_range(other).end,
                            )
                        };
                        state.set_selection(Selection::new(from, to));
                        state.backspace();
                    }
                    12 => {
                        state.undo();
                    }
                    13 => {
                        state.redo();
                    }
                    14 => {
                        state.set_selection(Selection::new(
                            Position::new(block, a.min(b)),
                            Position::new(block, a.max(b)),
                        ));
                        state.toggle_bold();
                    }
                    15 => {
                        state.set_caret(Position::new(block, a));
                        state.duplicate_blocks();
                    }
                    _ => {
                        state.set_caret(Position::new(block, a));
                        state.move_blocks_up();
                    }
                },
                cx,
            );
        });
        editor.read_with(cx, |editor, _| assert_sound(editor));
        // The checker catches up with a block now and then.
        if round % 7 == 0 {
            let count = editor.read_with(cx, |editor, _| editor.document().block_count());
            let index = random(count);
            assert!(check_block(&editor, index, cx).is_applied());
        }
        if round % 400 == 399 {
            assert_sound_after_recheck(&editor, cx);
        }
    }
}

/// The band of each wave of the last frame, top to bottom, left to right.
fn waves(editor: &Editor) -> Vec<gpui::Bounds<gpui::Pixels>> {
    let mut bands = editor.painted_spelling.clone();
    bands.sort_by(|a, b| {
        (a.top(), a.left())
            .partial_cmp(&(b.top(), b.left()))
            .unwrap()
    });
    bands
}

#[gpui::test]
fn a_wave_is_drawn_under_each_marked_word(cx: &mut TestAppContext) {
    setup(cx);
    let bold = InlineAttrs {
        bold: true,
        ..InlineAttrs::default()
    };
    let long = "word ".repeat(30);
    let document = Document::from_blocks(vec![
        Block::with_text(BlockKind::heading(1), "A teh heading"),
        Block::new(BlockKind::Paragraph)
            .with_span("plain teh and ", InlineAttrs::default())
            .with_span("bold wrld", bold)
            .with_span(" text", InlineAttrs::default()),
        Block::paragraph(long.trim_end()),
    ]);
    for page_view in [false, true] {
        let document = document.clone().with_settings(DocumentSettings {
            page_view,
            ..DocumentSettings::default()
        });
        let (editor, cx) = open(document, EditorConfig::full(), cx);
        editor.read_with(cx, |editor, _| assert!(waves(editor).is_empty()));
        check_all(&editor, cx);
        // A mark that runs over the end of a wrapped row (two "words").
        let wrapped = editor.update(cx, |editor, cx| {
            let block = &editor.document().blocks()[2];
            let (id, revision) = (block.id(), block.revision());
            let rows = editor
                .range_bounds(TextRange::in_block(2, 0, block.len()))
                .unwrap();
            assert!(rows.len() > 1, "the long paragraph wraps");
            // The first row break: where the caret's line box drops.
            let first_top = editor.position_bounds(Position::new(2, 0)).unwrap().top();
            let break_at = (0..block.len())
                .find(|offset| {
                    editor
                        .position_bounds(Position::new(2, *offset))
                        .unwrap()
                        .top()
                        > first_top
                })
                .unwrap();
            let range = break_at - 4..break_at + 4;
            assert!(
                editor
                    .set_spelling_marks(id, revision, [range.clone()], cx)
                    .is_applied()
            );
            range
        });
        cx.run_until_parked();

        editor.read_with(cx, |editor, _| {
            let bands = waves(editor);
            // Heading, two in the paragraph, two halves of the wrapped one.
            assert_eq!(bands.len(), 5, "{bands:?}");
            let expected = [
                (0, 2..5),
                (1, 6..9),
                (1, 19..23),
                (2, wrapped.start..wrapped.start + 3),
                (2, wrapped.start + 4..wrapped.end),
            ];
            for (band, (block, range)) in bands.iter().zip(expected) {
                let rows = editor
                    .range_bounds(TextRange::in_block(block, range.start, range.end))
                    .unwrap();
                assert_eq!(rows.len(), 1);
                let row = rows[0];
                // Exactly as wide as the word, and where the word is. (The
                // space a row was broken at gets no wave: the first half
                // of the wrapped mark is "ord", not "ord ".)
                assert_eq!(band.left(), row.left(), "block {block} {range:?}");
                assert!(
                    (band.size.width - row.size.width).abs() < px(0.01),
                    "block {block} {range:?}: {band:?} under {row:?}"
                );
                // Just under the baseline, inside the row's line box.
                let (layout, row_top) = {
                    let mut layout = editor.layout.borrow_mut();
                    let block_layout = layout.layout(block, editor.document()).unwrap();
                    (block_layout, row.top())
                };
                // (The layout is in points; page view draws a point
                // larger than a pixel at 100 %.)
                let zoom = editor.view_zoom();
                let baseline = row_top + layout.baseline * zoom;
                // One point under body text, two under a first heading
                // (34 pt), to the half pixel.
                let offset = band.top() - baseline;
                let expected = if block == 0 { px(2.) } else { px(1.) } * zoom;
                assert!(
                    (offset - expected).abs() <= px(0.25),
                    "just under the baseline: {band:?} {baseline:?}"
                );
                assert_eq!(band.top(), snap(band.top()), "on a device pixel");
                assert!(
                    band.bottom() <= row.bottom(),
                    "inside the row: {band:?} in {row:?}"
                );
                // The same line under every text size: a point thick,
                // on device pixels.
                assert_eq!(band.size.height, snap(px(1.) * zoom) * 3.);
            }
            // The two halves of the wrapped mark are on two rows.
            assert!(bands[4].top() > bands[3].bottom());
        });

        // The drawing switch: nothing is drawn, the marks are kept.
        editor.update(cx, |editor, cx| {
            editor.set_spelling_marks_visible(false, cx)
        });
        cx.run_until_parked();
        editor.read_with(cx, |editor, _| {
            assert!(waves(editor).is_empty());
            assert!(!editor.spelling_marks_visible());
            assert_eq!(editor.spelling_mark_count(), 4);
            assert_eq!(editor.spelling_mark_at(Position::new(0, 3)), mark(0, 2, 5));
        });
        editor.update(cx, |editor, cx| editor.set_spelling_marks_visible(true, cx));
        cx.run_until_parked();
        editor.read_with(cx, |editor, _| assert_eq!(waves(editor).len(), 5));

        // The marks do not move anything: the layout with and without them
        // is the same.
        let with_marks = editor.read_with(cx, |editor, _| {
            (0..3)
                .map(|block| editor.position_bounds(Position::new(block, 3)).unwrap())
                .collect::<Vec<_>>()
        });
        editor.update(cx, |editor, cx| editor.clear_spelling_marks(cx));
        cx.run_until_parked();
        editor.read_with(cx, |editor, _| {
            assert!(waves(editor).is_empty());
            let without: Vec<_> = (0..3)
                .map(|block| editor.position_bounds(Position::new(block, 3)).unwrap())
                .collect();
            assert_eq!(with_marks, without);
        });

        // Typing moves the wave with its word, in the same frame.
        check_all(&editor, cx);
        caret(&editor, 1, 0, cx);
        let before = editor.read_with(cx, |editor, _| waves(editor));
        cx.simulate_input("ab");
        cx.run_until_parked();
        editor.read_with(cx, |editor, _| {
            let after = waves(editor);
            assert_eq!(after.len(), before.len());
            // Two glyphs of the test platform's font to the right.
            let shift = after[1].left() - before[1].left();
            let two = px(2. * 0.6 * 17.) * editor.view_zoom();
            assert!((shift - two).abs() < px(0.01), "{shift:?}");
            assert_eq!(after[0], before[0]);
        });
    }
}

#[gpui::test]
fn the_context_menu_request_carries_the_mark(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open(
        Document::from_blocks(vec![
            Block::paragraph("I saw teh wrld today"),
            Block::with_text(BlockKind::todo(), "zzz to do"),
        ]),
        EditorConfig::full(),
        cx,
    );
    check_all(&editor, cx);
    let take = record_events(&editor, cx);
    let on = |cx: &mut VisualTestContext, block, offset| {
        editor.read_with(cx, |editor, _| {
            point_of(editor, block, offset) + gpui::point(px(3.), px(0.))
        })
    };

    // A right-click on a marked word: the word is selected and named.
    let at = on(cx, 0, 7);
    right_click(cx, at);
    let request = request_in(&take(cx));
    assert_eq!(request.spelling_mark, mark(0, 6, 9));
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.state().selected_text(), "teh")
    });
    // On its first and on its last character.
    for offset in [6, 8] {
        let at = on(cx, 0, offset);
        right_click(cx, at);
        assert_eq!(request_in(&take(cx)).spelling_mark, mark(0, 6, 9));
    }
    // On the space after it, and on a word that is not marked: none.
    for offset in [9, 5, 2, 16] {
        let at = on(cx, 0, offset);
        right_click(cx, at);
        assert_eq!(request_in(&take(cx)).spelling_mark, None, "offset {offset}");
    }
    // On the checkbox of a to-do whose first word is marked: none.
    let checkbox = editor.read_with(cx, |editor, _| editor.checkbox_hits[0].bounds.center());
    right_click(cx, checkbox);
    let request = request_in(&take(cx));
    assert!(request.on_checkbox);
    assert_eq!(request.spelling_mark, None);

    // From the keyboard: the caret in the word, or at one of its ends.
    for (offset, expected) in [
        (6, mark(0, 6, 9)),
        (7, mark(0, 6, 9)),
        (9, mark(0, 6, 9)),
        (10, mark(0, 10, 14)),
        (14, mark(0, 10, 14)),
        (15, None),
        (3, None),
    ] {
        caret(&editor, 0, offset, cx);
        take(cx);
        cx.simulate_keystrokes("ctrl-enter");
        let request = request_in(&take(cx));
        assert!(request.from_keyboard);
        assert_eq!(request.spelling_mark, expected, "offset {offset}");
    }
    cx.simulate_keystrokes("shift-f10");
    assert_eq!(request_in(&take(cx)).spelling_mark, None);

    // Marks that are not drawn are not offered.
    editor.update(cx, |editor, cx| {
        editor.set_spelling_marks_visible(false, cx)
    });
    caret(&editor, 0, 7, cx);
    take(cx);
    cx.simulate_keystrokes("ctrl-enter");
    assert_eq!(request_in(&take(cx)).spelling_mark, None);
    let at = on(cx, 0, 7);
    right_click(cx, at);
    assert_eq!(request_in(&take(cx)).spelling_mark, None);
}

fn median(mut values: Vec<std::time::Duration>) -> std::time::Duration {
    values.sort();
    values[values.len() / 2]
}

/// A 10,000-block book with a misspelling in most paragraphs: what the
/// marks cost. The bounds are loose (this runs unoptimised, next to other
/// builds); the numbers are for the report:
/// `cargo test -p innerword-editor --lib spelling_marks_in_a_book -- --nocapture`.
#[gpui::test]
fn spelling_marks_in_a_book(cx: &mut TestAppContext) {
    setup(cx);
    let sentence = "The quick brown fox jumps over teh lazy dog and keeps on running wrld. ";
    let blocks: Vec<Block> = (0..10_000)
        .map(|index| match index % 40 {
            0 => Block::with_text(
                BlockKind::heading(2),
                &format!("Chapter {}", index / 40 + 1),
            ),
            _ => Block::paragraph(sentence.repeat(1 + index % 3).trim_end()),
        })
        .collect();
    let (editor, cx) = open(Document::from_blocks(blocks), EditorConfig::full(), cx);

    // Typing without any marks, as the baseline.
    let type_at = |block: usize, cx: &mut VisualTestContext| {
        caret(&editor, block, 0, cx);
        cx.run_until_parked();
        let mut keys = Vec::new();
        for ch in "The five boxing wizards jump quickly. ".chars() {
            let started = Instant::now();
            cx.simulate_input(&ch.to_string());
            keys.push(started.elapsed());
        }
        median(keys)
    };
    let key_before = type_at(5_000, cx);
    let frame_before = editor.read_with(cx, |editor, _| editor.frame_perf());

    // The checker's results for every block, in document order.
    let texts: Vec<(BlockId, u64, Vec<Range<usize>>)> = editor.read_with(cx, |editor, _| {
        editor
            .document()
            .blocks()
            .iter()
            .map(|block| (block.id(), block.revision(), misspelled(block.text())))
            .collect()
    });
    let started = Instant::now();
    editor.update(cx, |editor, cx| {
        for (id, revision, ranges) in texts {
            assert!(
                editor
                    .set_spelling_marks(id, revision, ranges, cx)
                    .is_applied()
            );
        }
    });
    let set_all = started.elapsed();
    cx.run_until_parked();
    let marks = editor.read_with(cx, |editor, _| editor.spelling_mark_count());
    assert!(marks > 30_000, "{marks}");

    let key_after = type_at(5_000, cx);
    let frame_after = editor.read_with(cx, |editor, _| {
        assert!(!waves(editor).is_empty());
        editor.frame_perf()
    });
    // Enter and backspace: a split and a join with marks to carry.
    caret(&editor, 5_001, 80, cx);
    let mut splits = Vec::new();
    for _ in 0..20 {
        let started = Instant::now();
        cx.simulate_keystrokes("enter");
        cx.simulate_keystrokes("backspace");
        splits.push(started.elapsed() / 2);
    }
    // The queries, at their worst: from the end of the document round to
    // its start, and an unchecked block that is not there.
    let started = Instant::now();
    let (next, previous, unchecked) = editor.read_with(cx, |editor, _| {
        let end = editor.document().end();
        let mut next = None;
        let mut previous = None;
        let mut unchecked = None;
        for _ in 0..10 {
            next = editor.next_spelling_mark(end);
            previous = editor.previous_spelling_mark(Position::new(0, 0));
            unchecked = editor.first_unchecked_spelling_block(0..usize::MAX);
        }
        (next, previous, unchecked)
    });
    let queries = started.elapsed() / 10;
    assert_eq!(next.map(|range| range.start.block), Some(1));
    assert_eq!(previous.map(|range| range.start.block), Some(9_999));
    assert_eq!(unchecked, Some(5_000));
    // A document without a single mark: the search passes every block.
    let started = Instant::now();
    editor.update(cx, |editor, cx| {
        let first = &editor.document().blocks()[0];
        let (id, revision) = (first.id(), first.revision());
        editor.clear_spelling_marks(cx);
        // One mark at the very start, searched for from just behind it.
        editor.set_spelling_marks(id, revision, [0..3], cx);
        for _ in 0..10 {
            assert!(editor.next_spelling_mark(Position::new(0, 1)).is_some());
        }
    });
    let lap = started.elapsed() / 10;

    eprintln!(
        "10,000 blocks, {marks} spelling marks: setting all {set_all:?} ({:?} a block), \
         key + frame median {key_before:?} without marks, {key_after:?} with marks \
         (prepaint {:?} / {:?}, paint {:?} / {:?}), enter or backspace {:?}, \
         next + previous + first unchecked across the document {queries:?}, \
         next round a document with one mark {lap:?}",
        set_all / 10_000,
        frame_before.prepaint,
        frame_after.prepaint,
        frame_before.paint,
        frame_after.paint,
        median(splits),
    );
    assert!(set_all.as_millis() < 2_000);
    assert!(queries.as_millis() < 100);
    editor.read_with(cx, |editor, _| {
        editor.spelling.check_invariants(editor.document()).unwrap()
    });
}
