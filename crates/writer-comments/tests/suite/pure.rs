//! The parts without GPUI: relative times, quotes, the word at the caret,
//! thread order; and the published command list.

use gpui::TestAppContext;
use writer_comments::{
    threads::{QUOTE_MAX_CHARS, collect_threads, quote, truncate, word_range_at},
    time::relative_time,
    track::RangeTracker,
};
use writer_core::{Block, Document, EditorState, Position, TextRange};

use super::support::{NOW, sample, setup};

const MINUTE: i64 = 60_000;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;

#[test]
fn relative_times_read_plainly() {
    let ago = |elapsed: i64| relative_time(NOW - elapsed, NOW);
    assert_eq!(ago(0), "just now");
    assert_eq!(ago(59_000), "just now");
    assert_eq!(ago(MINUTE), "1 min ago");
    assert_eq!(ago(59 * MINUTE), "59 min ago");
    assert_eq!(ago(HOUR), "1 hour ago");
    assert_eq!(ago(23 * HOUR), "23 hours ago");
    assert_eq!(ago(DAY), "yesterday");
    assert_eq!(ago(2 * DAY), "2 days ago");
    assert_eq!(ago(6 * DAY), "6 days ago");
    assert_eq!(ago(7 * DAY), "1 week ago");
    assert_eq!(ago(20 * DAY), "2 weeks ago");
    assert_eq!(ago(31 * DAY), "1 month ago");
    assert_eq!(ago(200 * DAY), "6 months ago");
    assert_eq!(ago(365 * DAY), "1 year ago");
    assert_eq!(ago(800 * DAY), "2 years ago");
    // A clock that was set back does not produce negative times.
    assert_eq!(relative_time(NOW + HOUR, NOW), "just now");
    assert_eq!(
        relative_time(i64::MIN, i64::MAX),
        relative_time(0, i64::MAX)
    );
}

#[test]
fn quotes_are_one_line_and_short() {
    assert_eq!(truncate("short", 10), "short");
    assert_eq!(truncate("exactly ten", 11), "exactly ten");
    assert_eq!(truncate("one two three", 8), "one two…");
    assert_eq!(truncate("naïve café señor", 6), "naïve…");

    let document = Document::from_blocks(vec![
        Block::paragraph("First  line\nwith a soft break"),
        Block::paragraph(&"word ".repeat(40)),
    ]);
    // White space, soft breaks included, collapses to single spaces.
    assert_eq!(
        quote(&document, &[TextRange::in_block(0, 0, 29)]),
        "First line with a soft break"
    );
    // Pieces of a split anchor are joined with an ellipsis.
    assert_eq!(
        quote(
            &document,
            &[TextRange::in_block(0, 0, 5), TextRange::in_block(0, 12, 16)]
        ),
        "First … with"
    );
    let long = quote(&document, &[document.block_range(1)]);
    assert_eq!(long.chars().count(), QUOTE_MAX_CHARS);
    assert!(long.ends_with('…'));
}

#[test]
fn the_word_at_a_collapsed_caret() {
    let document = Document::from_blocks(vec![
        Block::paragraph("The quick, brown fox"),
        Block::paragraph(""),
        Block::paragraph("   "),
    ]);
    let word = |block, offset| word_range_at(&document, Position::new(block, offset));
    // Inside a word, at its start and right after it.
    assert_eq!(word(0, 6), Some(TextRange::in_block(0, 4, 9)));
    assert_eq!(word(0, 4), Some(TextRange::in_block(0, 4, 9)));
    assert_eq!(word(0, 9), Some(TextRange::in_block(0, 4, 9)));
    assert_eq!(word(0, 0), Some(TextRange::in_block(0, 0, 3)));
    assert_eq!(word(0, 20), Some(TextRange::in_block(0, 17, 20)));
    // Between a comma and a space: no word touches the caret.
    assert_eq!(word(0, 10), None);
    assert_eq!(word(1, 0), None);
    assert_eq!(word(2, 1), None);
    // Out-of-range positions clamp.
    assert_eq!(word(9, 99), None);
}

#[test]
fn threads_come_in_document_order_with_detached_last() {
    let mut state = EditorState::new(sample());
    let add = |state: &mut EditorState, block, start, end, body: &str| {
        state.select_range(TextRange::in_block(block, start, end));
        state.add_comment(body, "Ada", NOW).unwrap()
    };
    // Created out of document order on purpose.
    let third = add(&mut state, 2, 4, 9, "on third");
    let quick = add(&mut state, 0, 4, 9, "on quick");
    let second = add(&mut state, 1, 2, 8, "on second");
    let lazy = add(&mut state, 0, 35, 39, "on lazy");

    let entries = collect_threads(state.document());
    let ids: Vec<_> = entries.iter().map(|entry| entry.id).collect();
    assert_eq!(ids, vec![quick, lazy, second, third]);
    assert_eq!(entries[0].quote, "quick");
    assert_eq!(entries[0].anchor(), Some(TextRange::in_block(0, 4, 9)));
    assert!(entries[0].contains(Position::new(0, 4)));
    assert!(entries[0].contains(Position::new(0, 9)));
    assert!(!entries[0].contains(Position::new(0, 10)));

    // Delete the text under two threads: they become detached and move to
    // the end, in creation order.
    state.select_range(TextRange::in_block(2, 4, 9));
    state.delete_selection();
    state.select_range(TextRange::in_block(0, 4, 9));
    state.delete_selection();
    let entries = collect_threads(state.document());
    let ids: Vec<_> = entries.iter().map(|entry| entry.id).collect();
    assert_eq!(ids, vec![lazy, second, third, quick]);
    assert!(!entries[0].is_detached());
    assert!(entries[2].is_detached() && entries[3].is_detached());
    assert_eq!(entries[3].quote, "");
    assert_eq!(entries[3].anchor(), None);

    assert!(collect_threads(&sample()).is_empty());
}

#[gpui::test]
fn commands_are_published_with_their_keys(cx: &mut TestAppContext) {
    setup(cx);
    let commands = writer_comments::commands();
    assert_eq!(commands.len(), 10);
    assert!(
        commands
            .iter()
            .all(|command| command.category.as_ref() == writer_comments::CATEGORY)
    );
    let hint = |title: &str| {
        let command = commands
            .iter()
            .find(|command| command.title.as_ref() == title)
            .unwrap_or_else(|| panic!("no command titled {title}"));
        cx.update(|cx| command.key_hint(cx).map(|hint| hint.text()))
    };
    assert_eq!(hint("Add comment").as_deref(), Some("⌥⌘M"));
    assert_eq!(hint("Next comment").as_deref(), Some("⌥⌘]"));
    assert_eq!(hint("Previous comment").as_deref(), Some("⌥⌘["));
    assert_eq!(hint("Show or hide comments").as_deref(), Some("⌥⌘C"));
    assert_eq!(hint("Focus comments").as_deref(), Some("⌘2"));
    // The canonical key of an action with alternatives is the one shown.
    assert_eq!(hint("Reply to comment").as_deref(), Some("↩"));
    assert_eq!(hint("Delete comment").as_deref(), Some("⌘⌫"));

    // Every action can be built by name, as the palette does.
    cx.update(|cx| {
        for command in &commands {
            assert!(
                cx.build_action(command.action_name(), None).is_ok(),
                "{} cannot be built by name",
                command.action_name()
            );
        }
    });
}

// ----- following a range through edits ------------------------------------------

/// Runs `change` on the state and tells the tracker what the editor would
/// report for it.
fn follow<R>(
    state: &mut EditorState,
    tracker: &mut RangeTracker,
    change: impl FnOnce(&mut EditorState) -> R,
) -> R {
    let result = change(state);
    let splice = state.take_changes().and_then(|change| change.blocks);
    tracker.update(state.document(), splice);
    result
}

fn tracked(state: &EditorState, tracker: &RangeTracker) -> String {
    assert_eq!(
        tracker.text(),
        state.document().text_in(tracker.range()),
        "the remembered text is the text of the range"
    );
    state.document().text_in(tracker.range())
}

/// "brown" in the first paragraph of the sample.
fn brown() -> (EditorState, RangeTracker) {
    let mut state = EditorState::new(sample());
    state.take_changes();
    let tracker = RangeTracker::new(state.document(), TextRange::in_block(0, 10, 15));
    (state, tracker)
}

#[test]
fn a_tracked_range_moves_with_the_text_around_it() {
    let (mut state, mut tracker) = brown();
    assert_eq!(tracked(&state, &tracker), "brown");

    // Typing before it, in the same paragraph, shifts it.
    follow(&mut state, &mut tracker, |state| {
        state.set_caret(Position::new(0, 0));
        state.insert_text("Look: ")
    });
    assert_eq!(tracker.range(), TextRange::in_block(0, 16, 21));
    // Typing behind it does not.
    follow(&mut state, &mut tracker, |state| {
        state.set_caret(Position::new(0, 30));
        state.insert_text("!!")
    });
    assert_eq!(tracker.range(), TextRange::in_block(0, 16, 21));
    // An edit in another paragraph does not.
    follow(&mut state, &mut tracker, |state| {
        state.set_caret(Position::new(2, 3));
        state.insert_text("xyz")
    });
    assert_eq!(tracker.range(), TextRange::in_block(0, 16, 21));
    // Deleting before it pulls it back.
    follow(&mut state, &mut tracker, |state| {
        state.select_range(TextRange::in_block(0, 0, 6));
        state.delete_selection()
    });
    assert_eq!(tracker.range(), TextRange::in_block(0, 10, 15));
    assert_eq!(tracked(&state, &tracker), "brown");

    // A change of formatting, or a comment on it, changes no text.
    follow(&mut state, &mut tracker, |state| {
        state.select_range(TextRange::in_block(0, 4, 19));
        state.toggle_bold();
        state.add_comment("On it", "Ada", NOW)
    });
    assert_eq!(tracker.range(), TextRange::in_block(0, 10, 15));
    // Nothing changed at all.
    tracker.update(state.document(), None);
    assert_eq!(tracker.range(), TextRange::in_block(0, 10, 15));
}

#[test]
fn text_typed_at_the_edges_of_a_tracked_range_stays_outside() {
    let (mut state, mut tracker) = brown();
    follow(&mut state, &mut tracker, |state| {
        state.set_caret(Position::new(0, 10));
        state.insert_text("dark ")
    });
    assert_eq!(tracked(&state, &tracker), "brown");
    assert_eq!(tracker.range(), TextRange::in_block(0, 15, 20));
    follow(&mut state, &mut tracker, |state| {
        state.set_caret(Position::new(0, 20));
        state.insert_text("ish")
    });
    assert_eq!(tracked(&state, &tracker), "brown");
    // Typed into it: part of it.
    follow(&mut state, &mut tracker, |state| {
        state.set_caret(Position::new(0, 17));
        state.insert_text("OO")
    });
    assert_eq!(tracked(&state, &tracker), "brOOown");
    // Part of it deleted: what is left.
    follow(&mut state, &mut tracker, |state| {
        state.select_range(TextRange::in_block(0, 13, 19));
        state.delete_selection()
    });
    assert_eq!(tracked(&state, &tracker), "own");
    // A deletion that takes its end and more.
    follow(&mut state, &mut tracker, |state| {
        state.select_range(TextRange::in_block(0, 15, 22));
        state.delete_selection()
    });
    assert_eq!(tracked(&state, &tracker), "ow");
    assert!(!tracker.is_lost());
}

#[test]
fn a_tracked_range_follows_paragraphs_that_split_join_and_move() {
    let (mut state, mut tracker) = brown();
    // A paragraph break before it: it is in the new second paragraph.
    follow(&mut state, &mut tracker, |state| {
        state.set_caret(Position::new(0, 4));
        state.insert_paragraph_break()
    });
    assert_eq!(tracker.range(), TextRange::in_block(1, 6, 11));
    assert_eq!(tracked(&state, &tracker), "brown");
    // A paragraph break behind it: nothing.
    follow(&mut state, &mut tracker, |state| {
        state.set_caret(Position::new(1, 12));
        state.insert_paragraph_break()
    });
    assert_eq!(tracker.range(), TextRange::in_block(1, 6, 11));
    // A paragraph break in the middle of it: it spans both.
    follow(&mut state, &mut tracker, |state| {
        state.set_caret(Position::new(1, 8));
        state.insert_paragraph_break()
    });
    assert_eq!(
        tracker.range(),
        TextRange::new(Position::new(1, 6), Position::new(2, 3))
    );
    assert_eq!(tracked(&state, &tracker), "br\nown");
    // Joined again, one by one.
    follow(&mut state, &mut tracker, |state| {
        state.set_caret(Position::new(2, 0));
        state.backspace()
    });
    assert_eq!(tracker.range(), TextRange::in_block(1, 6, 11));
    follow(&mut state, &mut tracker, |state| {
        state.set_caret(Position::new(1, 0));
        state.backspace()
    });
    assert_eq!(tracker.range(), TextRange::in_block(0, 10, 15));
    assert_eq!(tracked(&state, &tracker), "brown");

    // Its paragraph is moved down, and a paragraph above it is deleted.
    follow(&mut state, &mut tracker, |state| {
        state.set_caret(Position::new(0, 2));
        state.move_blocks_down()
    });
    assert_eq!(tracker.range().start.block, 1);
    assert_eq!(tracked(&state, &tracker), "brown");
    follow(&mut state, &mut tracker, |state| {
        state.set_caret(Position::new(0, 2));
        state.delete_blocks()
    });
    assert_eq!(tracker.range().start.block, 0);
    assert_eq!(tracked(&state, &tracker), "brown");
    // Undo of both, one step each.
    follow(&mut state, &mut tracker, |state| state.undo());
    follow(&mut state, &mut tracker, |state| state.undo());
    assert_eq!(tracked(&state, &tracker), "brown");
    assert_eq!(tracker.range().start.block, 0);
}

#[test]
fn a_tracked_range_whose_text_is_deleted_comes_back_with_it() {
    let (mut state, mut tracker) = brown();
    // Exactly its text.
    follow(&mut state, &mut tracker, |state| {
        state.select_range(TextRange::in_block(0, 10, 15));
        state.delete_selection()
    });
    assert!(tracker.is_lost());
    assert_eq!(tracker.range(), TextRange::in_block(0, 10, 10));
    // Typing there does not make the new text the range.
    follow(&mut state, &mut tracker, |state| state.insert_text("red"));
    assert!(tracker.is_lost());
    follow(&mut state, &mut tracker, |state| state.undo());
    follow(&mut state, &mut tracker, |state| state.undo());
    assert!(!tracker.is_lost());
    assert_eq!(tracker.range(), TextRange::in_block(0, 10, 15));
    assert_eq!(tracked(&state, &tracker), "brown");

    // More than its text, across paragraphs.
    follow(&mut state, &mut tracker, |state| {
        state.select_range(TextRange::new(Position::new(0, 4), Position::new(1, 8)));
        state.delete_selection()
    });
    assert!(tracker.is_lost());
    assert_eq!(tracker.range().start, Position::new(0, 4));
    follow(&mut state, &mut tracker, |state| state.undo());
    assert_eq!(tracker.range(), TextRange::in_block(0, 10, 15));
    assert_eq!(tracked(&state, &tracker), "brown");

    // The whole document replaced: lost, and found again by undo.
    follow(&mut state, &mut tracker, |state| {
        state.select_all();
        state.insert_text("Nothing of it is left.")
    });
    assert!(tracker.is_lost());
    follow(&mut state, &mut tracker, |state| state.undo());
    assert_eq!(tracked(&state, &tracker), "brown");
    assert_eq!(tracker.range(), TextRange::in_block(0, 10, 15));
}

#[test]
fn a_tracked_range_is_safe_with_text_that_is_not_ascii() {
    let mut state = EditorState::new(Document::from_blocks(vec![
        Block::paragraph("Größe und Maß, naïve café."),
        Block::paragraph("Zweite Zeile."),
    ]));
    state.take_changes();
    let range = {
        let text = state.document().block(0).unwrap().text();
        let start = text.find("Maß").unwrap();
        TextRange::in_block(0, start, start + "Maß".len())
    };
    let mut tracker = RangeTracker::new(state.document(), range);
    assert_eq!(tracked(&state, &tracker), "Maß");
    // "ö" replaced by "ü": the bytes around the change are shared.
    follow(&mut state, &mut tracker, |state| {
        state.select_range(TextRange::in_block(0, 2, 4));
        state.insert_text("ü")
    });
    assert_eq!(tracked(&state, &tracker), "Maß");
    follow(&mut state, &mut tracker, |state| {
        state.set_caret(Position::new(0, 0));
        state.insert_text("é")
    });
    assert_eq!(tracked(&state, &tracker), "Maß");
    // The ß replaced by ss, inside it.
    let end = tracker.range().end;
    follow(&mut state, &mut tracker, |state| {
        state.select_range(TextRange::in_block(0, end.offset - "ß".len(), end.offset));
        state.insert_text("ss")
    });
    assert_eq!(tracked(&state, &tracker), "Mass");
    // Several changes reported as one.
    let mut tracker = RangeTracker::new(state.document(), TextRange::in_block(1, 0, 6));
    state.set_caret(Position::new(0, 0));
    state.insert_text("Ä ");
    state.set_caret(Position::new(1, 0));
    state.insert_text("Die ");
    let splice = state.take_changes().and_then(|change| change.blocks);
    tracker.update(state.document(), splice);
    assert_eq!(tracked(&state, &tracker), "Zweite");
}
