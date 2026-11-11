use std::{cell::Cell, time::Duration};

use gpui::{Entity, TestAppContext, VisualTestContext};
use writer_core::{
    Block, BlockKind, Document, InlineAttrs, Position, SearchOptions, Selection, TextRange,
};
use writer_editor::Editor;
use writer_editor_tools::{
    EditorTools, FIND_BUTTONS, FIND_CURRENT_LAYER, FIND_FLASH_DURATION, FIND_LAYER, ToolKind,
};

use crate::support::*;

fn document() -> Document {
    paragraphs(&[
        "The cat sat on the mat.",
        "Then the other cat left; theatre people stayed.",
        "Nothing here.",
        "THE END of the story",
    ])
}

fn status(tools: &Entity<EditorTools>, cx: &mut VisualTestContext) -> String {
    tools.read_with(cx, |tools, _| tools.find_status())
}

fn selection(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> TextRange {
    editor.read_with(cx, |editor, _| editor.selection().range())
}

fn layer(editor: &Entity<Editor>, name: &str, cx: &mut VisualTestContext) -> Vec<TextRange> {
    editor.read_with(cx, |editor, _| editor.highlights(name).to_vec())
}

#[gpui::test]
fn finds_counts_and_steps_through_matches(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(document(), cx);

    cx.simulate_keystrokes("cmd-f");
    assert_eq!(active(&tools, cx), Some(ToolKind::Find));
    assert!(!editor_focused(&editor, cx), "focus is in the query field");
    assert_eq!(status(&tools, cx), "");

    // Live: every match is highlighted, the first one from the caret is
    // current and selected in the editor.
    cx.simulate_input("the");
    assert_eq!(
        text(&editor, cx).lines().next(),
        Some("The cat sat on the mat.")
    );
    tools.read_with(cx, |tools, _| {
        assert_eq!(tools.find_query(), "the");
        assert_eq!(tools.find_match_count(), 8);
        assert_eq!(tools.find_current_match(), Some(1));
    });
    assert_eq!(status(&tools, cx), "1 of 8");
    assert_eq!(layer(&editor, FIND_LAYER, cx).len(), 8);
    assert_eq!(
        layer(&editor, FIND_CURRENT_LAYER, cx),
        [TextRange::in_block(0, 0, 3)]
    );
    assert_eq!(selection(&editor, cx), TextRange::in_block(0, 0, 3));

    // enter / shift-enter and cmd-g / cmd-shift-g.
    cx.simulate_keystrokes("enter");
    assert_eq!(status(&tools, cx), "2 of 8");
    assert_eq!(selection(&editor, cx), TextRange::in_block(0, 15, 18));
    cx.simulate_keystrokes("cmd-g");
    assert_eq!(status(&tools, cx), "3 of 8");
    assert_eq!(selection(&editor, cx), TextRange::in_block(1, 0, 3));
    assert_eq!(
        layer(&editor, FIND_CURRENT_LAYER, cx),
        [TextRange::in_block(1, 0, 3)]
    );
    cx.simulate_keystrokes("shift-enter");
    assert_eq!(status(&tools, cx), "2 of 8");
    cx.simulate_keystrokes("cmd-shift-g cmd-shift-g");
    // Wraps backwards from the first to the last.
    assert_eq!(status(&tools, cx), "8 of 8");
    assert_eq!(selection(&editor, cx), TextRange::in_block(3, 11, 14));
    cx.simulate_keystrokes("enter");
    assert_eq!(status(&tools, cx), "1 of 8");

    // A longer query keeps the place it is at.
    cx.simulate_keystrokes("enter enter");
    assert_eq!(status(&tools, cx), "3 of 8");
    cx.simulate_input("n");
    assert_eq!(status(&tools, cx), "1 of 1");
    assert_eq!(selection(&editor, cx), TextRange::in_block(1, 0, 4));

    cx.simulate_input("x");
    assert_eq!(status(&tools, cx), "No matches");
    assert!(layer(&editor, FIND_LAYER, cx).is_empty());
    assert!(layer(&editor, FIND_CURRENT_LAYER, cx).is_empty());
    assert_eq!(
        selection(&editor, cx),
        TextRange::in_block(1, 0, 4),
        "selection stays"
    );
}

#[gpui::test]
fn options_have_their_own_shortcuts(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(document(), cx);

    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("the");
    assert_eq!(status(&tools, cx), "1 of 8");

    cx.simulate_keystrokes("cmd-alt-c");
    tools.read_with(cx, |tools, _| assert!(tools.find_options().case_sensitive));
    // "the" in lower case: the, the, other, theatre, the.
    assert_eq!(tools.read_with(cx, |tools, _| tools.find_match_count()), 5);
    assert_eq!(selection(&editor, cx), TextRange::in_block(0, 15, 18));

    cx.simulate_keystrokes("cmd-alt-w");
    tools.read_with(cx, |tools, _| assert!(tools.find_options().whole_word));
    assert_eq!(tools.read_with(cx, |tools, _| tools.find_match_count()), 3);

    cx.simulate_keystrokes("cmd-alt-c");
    // Whole word, any case: The, the, the, THE, the.
    assert_eq!(tools.read_with(cx, |tools, _| tools.find_match_count()), 5);
    assert_eq!(layer(&editor, FIND_LAYER, cx).len(), 5);
    cx.simulate_keystrokes("cmd-alt-w");
    assert_eq!(tools.read_with(cx, |tools, _| tools.find_match_count()), 8);
}

#[gpui::test]
fn escape_closes_and_leaves_the_match_selected(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(document(), cx);

    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("cat");
    cx.simulate_keystrokes("enter");
    assert_eq!(status(&tools, cx), "2 of 2");
    cx.simulate_keystrokes("escape");
    assert_eq!(active(&tools, cx), None);
    assert!(editor_focused(&editor, cx));
    assert_eq!(selection(&editor, cx), TextRange::in_block(1, 15, 18));
    assert!(layer(&editor, FIND_LAYER, cx).is_empty());
    assert!(layer(&editor, FIND_CURRENT_LAYER, cx).is_empty());

    // The query is remembered: cmd-g goes on without opening the bar.
    cx.simulate_keystrokes("cmd-g");
    assert_eq!(active(&tools, cx), None);
    assert_eq!(selection(&editor, cx), TextRange::in_block(0, 4, 7));
    cx.simulate_keystrokes("cmd-shift-g");
    assert_eq!(selection(&editor, cx), TextRange::in_block(1, 15, 18));

    // Typing replaces the selected match, as with any selection.
    cx.simulate_input("dog");
    assert_eq!(
        text(&editor, cx).lines().nth(1),
        Some("Then the other dog left; theatre people stayed.")
    );
}

#[gpui::test]
fn find_next_with_the_bar_closed_shows_where_it_lands(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(document(), cx);
    // (gpui keeps the debug bounds of elements that are gone, so only
    // their presence says something.)
    let flash = |cx: &mut VisualTestContext| {
        let visible = tools.read_with(cx, |tools, _| tools.find_flash_visible());
        if visible {
            assert!(
                cx.debug_bounds("find-flash").is_some(),
                "the label is drawn"
            );
        }
        visible
    };
    let wait = |milliseconds: u64, cx: &mut VisualTestContext| {
        cx.executor()
            .advance_clock(Duration::from_millis(milliseconds));
        cx.run_until_parked();
    };

    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("cat");
    cx.simulate_keystrokes("escape");
    assert_eq!(selection(&editor, cx), TextRange::in_block(0, 4, 7));
    assert!(!flash(cx));
    assert!(cx.debug_bounds("find-flash").is_none());
    assert_eq!(status(&tools, cx), "");

    // cmd-g: the bar stays closed and the keyboard stays in the text, but
    // the matches are highlighted and a label says which one this is.
    cx.simulate_keystrokes("cmd-g");
    assert_eq!(active(&tools, cx), None);
    assert!(editor_focused(&editor, cx));
    assert!(flash(cx));
    assert_eq!(selection(&editor, cx), TextRange::in_block(1, 15, 18));
    assert_eq!(status(&tools, cx), "2 of 2");
    assert_eq!(layer(&editor, FIND_LAYER, cx).len(), 2);
    assert_eq!(
        layer(&editor, FIND_CURRENT_LAYER, cx),
        [TextRange::in_block(1, 15, 18)]
    );
    tools.read_with(cx, |tools, _| {
        assert_eq!(tools.find_match_count(), 2);
        assert_eq!(tools.find_current_match(), Some(2));
        assert!(tools.find_results_visible());
    });

    // Stepping on does not search again while the results are showing.
    let searches = tools.read_with(cx, |tools, _| tools.find_full_search_count());
    cx.simulate_keystrokes("cmd-shift-g");
    assert_eq!(status(&tools, cx), "1 of 2");
    cx.simulate_keystrokes("cmd-g cmd-g");
    assert_eq!(status(&tools, cx), "1 of 2");
    assert_eq!(
        tools.read_with(cx, |tools, _| tools.find_full_search_count()),
        searches
    );
    // Editing meanwhile keeps the results in step with the text.
    caret(&editor, 2, 13, cx);
    cx.simulate_input(" A cat.");
    assert_eq!(status(&tools, cx), "3 matches");
    assert_eq!(layer(&editor, FIND_LAYER, cx).len(), 3);
    assert!(layer(&editor, FIND_CURRENT_LAYER, cx).is_empty());

    // A moment after the last step everything is as before: no label, no
    // highlights. The caret stays where it is.
    wait(FIND_FLASH_DURATION.as_millis() as u64 + 50, cx);
    assert!(!flash(cx));
    assert_eq!(status(&tools, cx), "");
    assert!(layer(&editor, FIND_LAYER, cx).is_empty());
    assert!(layer(&editor, FIND_CURRENT_LAYER, cx).is_empty());
    tools.read_with(cx, |tools, _| {
        assert_eq!(tools.find_match_count(), 0);
        assert!(!tools.find_results_visible());
        assert_eq!(tools.find_query(), "cat");
    });
    assert_eq!(
        editor.read_with(cx, |editor, _| editor.selection()),
        Selection::caret(Position::new(2, 20))
    );

    // Every step starts the moment again.
    let most = FIND_FLASH_DURATION.as_millis() as u64 - 300;
    cx.simulate_keystrokes("cmd-g");
    assert_eq!(status(&tools, cx), "1 of 3");
    wait(most, cx);
    cx.simulate_keystrokes("cmd-g");
    wait(most, cx);
    assert!(flash(cx));
    assert_eq!(status(&tools, cx), "2 of 3");
    wait(400, cx);
    assert!(!flash(cx));

    // escape in the text ends it at once (and is used up by that).
    cx.simulate_keystrokes("cmd-g");
    assert!(flash(cx));
    cx.simulate_keystrokes("escape");
    assert!(!flash(cx));
    assert!(layer(&editor, FIND_LAYER, cx).is_empty());
    assert_eq!(selection(&editor, cx), TextRange::in_block(2, 16, 19));

    // Opening the bar takes over from the label, and the label's timer
    // does not close the bar later.
    cx.simulate_keystrokes("cmd-g");
    assert!(flash(cx));
    cx.simulate_keystrokes("cmd-f");
    assert!(!flash(cx));
    assert_eq!(active(&tools, cx), Some(ToolKind::Find));
    assert_eq!(status(&tools, cx), "1 of 3");
    wait(FIND_FLASH_DURATION.as_millis() as u64 + 50, cx);
    assert_eq!(active(&tools, cx), Some(ToolKind::Find));
    assert_eq!(layer(&editor, FIND_LAYER, cx).len(), 3);

    // Nothing to find any more: the label says so instead of doing
    // nothing.
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("zebra");
    cx.simulate_keystrokes("escape");
    let before = editor.read_with(cx, |editor, _| editor.selection());
    cx.simulate_keystrokes("cmd-g");
    assert!(flash(cx));
    assert_eq!(status(&tools, cx), "No matches");
    assert_eq!(editor.read_with(cx, |editor, _| editor.selection()), before);

    // A new document ends it.
    editor.update(cx, |editor, cx| {
        editor.set_document(paragraphs(&["zebra crossing"]), cx)
    });
    cx.run_until_parked();
    assert!(!flash(cx));
    cx.simulate_keystrokes("cmd-g");
    assert_eq!(status(&tools, cx), "1 of 1");
    assert_eq!(selection(&editor, cx), TextRange::in_block(0, 0, 5));
}

#[gpui::test]
fn the_replace_row_shows_and_hides_from_the_keyboard(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(document(), cx);
    let row = |cx: &mut VisualTestContext| {
        let visible = tools.read_with(cx, |tools, _| tools.find_replace_visible());
        if visible {
            assert!(cx.debug_bounds("find-replace-all").is_some());
        }
        visible
    };

    cx.simulate_keystrokes("cmd-f");
    assert!(!row(cx));
    assert!(cx.debug_bounds("find-replace-all").is_none());
    // cmd-alt-f shows the row; the keyboard stays in the query field.
    cx.simulate_keystrokes("cmd-alt-f");
    assert!(row(cx));
    cx.simulate_input("cat");
    tools.read_with(cx, |tools, _| assert_eq!(tools.find_query(), "cat"));
    cx.simulate_keystrokes("tab");
    cx.simulate_input("dog");
    tools.read_with(cx, |tools, cx| {
        assert_eq!(tools.find_replacement(cx), "dog")
    });

    // Once more, from the replace field: the row goes and the keyboard is
    // in the query field, not nowhere.
    cx.simulate_keystrokes("cmd-alt-f");
    assert!(!row(cx));
    assert_eq!(active(&tools, cx), Some(ToolKind::Find));
    cx.simulate_input("s");
    tools.read_with(cx, |tools, _| assert_eq!(tools.find_query(), "cats"));
    cx.simulate_keystrokes("backspace");
    // With the row hidden, tab has nowhere to go and cmd-enter still
    // replaces nothing by accident.
    cx.simulate_keystrokes("tab");
    cx.simulate_input("s");
    tools.read_with(cx, |tools, _| assert_eq!(tools.find_query(), "cats"));
    cx.simulate_keystrokes("backspace");

    // And back: what was typed in the replace field is still there.
    cx.simulate_keystrokes("cmd-alt-f");
    assert!(row(cx));
    tools.read_with(cx, |tools, cx| {
        assert_eq!(tools.find_replacement(cx), "dog")
    });

    // From the document the key brings the keyboard to the bar and never
    // hides the row.
    cx.update(|window, cx| editor.read(cx).focus(window));
    cx.simulate_keystrokes("cmd-alt-f");
    assert!(row(cx));
    assert!(!editor_focused(&editor, cx));
    cx.simulate_keystrokes("cmd-alt-f");
    assert!(!row(cx));

    // With the bar closed it opens with the row.
    cx.simulate_keystrokes("escape");
    assert_eq!(active(&tools, cx), None);
    cx.simulate_keystrokes("cmd-alt-f");
    assert!(row(cx));
}

#[gpui::test]
fn matches_follow_edits_without_searching_the_whole_document(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(document(), cx);
    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("the");
    cx.update(|window, cx| editor.read(cx).focus(window));
    cx.run_until_parked();
    let searches = Cell::new(tools.read_with(cx, |tools, _| tools.find_full_search_count()));

    // After every kind of edit the match list is exactly what a search of
    // the whole document finds, and the whole document was not searched.
    let check = |what: &str, cx: &mut VisualTestContext| {
        cx.run_until_parked();
        let expected = editor.read_with(cx, |editor, _| {
            editor.document().find_all("the", SearchOptions::default())
        });
        assert_eq!(layer(&editor, FIND_LAYER, cx), expected, "{what}");
        tools.read_with(cx, |tools, _| {
            assert_eq!(tools.find_match_count(), expected.len(), "{what}");
            assert_eq!(tools.find_full_search_count(), searches.get(), "{what}");
        });
        expected.len()
    };
    assert_eq!(check("start", cx), 8);

    caret(&editor, 2, 0, cx);
    cx.simulate_input("the ");
    assert_eq!(check("typing a match", cx), 9);
    cx.simulate_keystrokes("backspace backspace");
    assert_eq!(check("typing it away", cx), 8);

    // A block more, a block less: the matches after it move with it.
    caret(&editor, 1, 8, cx);
    cx.simulate_keystrokes("enter");
    assert_eq!(
        editor.read_with(cx, |editor, _| editor.document().blocks().len()),
        5
    );
    assert_eq!(check("splitting a block", cx), 8);
    cx.simulate_keystrokes("backspace");
    assert_eq!(check("joining blocks", cx), 8);
    cx.simulate_keystrokes("cmd-enter");
    check("a page break", cx);
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(check("undo", cx), 8);

    // A selection over several blocks, deleted.
    select(
        &editor,
        TextRange::new(Position::new(0, 10), Position::new(2, 3)),
        cx,
    );
    cx.simulate_keystrokes("backspace");
    assert_eq!(
        editor.read_with(cx, |editor, _| editor.document().blocks().len()),
        2
    );
    check("deleting across blocks", cx);
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(check("undoing it", cx), 8);
    cx.simulate_keystrokes("cmd-shift-z");
    check("redoing it", cx);
    cx.simulate_keystrokes("cmd-z");

    // Blocks that trade places.
    caret(&editor, 3, 0, cx);
    cx.simulate_keystrokes("alt-up alt-up");
    assert_eq!(check("moving a block", cx), 8);

    // Several edits before their events are delivered: the block range of
    // the first no longer describes the document, so this once the whole
    // document is searched (once, not once per edit).
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.insert_text("the theory "), cx);
        editor.edit(|state| state.insert_paragraph_break(), cx);
        editor.edit(|state| state.insert_text("the end"), cx);
    });
    searches.set(searches.get() + 1);
    assert_eq!(check("three edits at once", cx), 11);
    searches.set(searches.get() + 1);
    editor.update(cx, |editor, cx| {
        editor.edit(
            |state| {
                state.undo();
                state.undo();
            },
            cx,
        );
        editor.edit(|state| state.insert_text("then "), cx);
    });
    check("undo and an edit at once", cx);

    // Formatting changes the block but not its matches.
    cx.simulate_keystrokes("cmd-a cmd-b");
    check("bold everywhere", cx);
    cx.simulate_keystrokes("cmd-alt-2");
    check("headings everywhere", cx);

    // An option does need a new search.
    cx.simulate_keystrokes("cmd-f");
    let searches = tools.read_with(cx, |tools, _| tools.find_full_search_count());
    cx.simulate_keystrokes("cmd-alt-w");
    assert_eq!(
        tools.read_with(cx, |tools, _| tools.find_full_search_count()),
        searches + 1
    );
}

#[gpui::test]
fn matches_inside_formatted_text(cx: &mut TestAppContext) {
    setup(cx);
    let bold = InlineAttrs {
        bold: true,
        ..InlineAttrs::default()
    };
    let link = InlineAttrs {
        link: Some("https://example.com".into()),
        italic: true,
        ..InlineAttrs::default()
    };
    // "harbour" three times: across a bold boundary, inside a link, and in
    // a heading.
    let document = Document::from_blocks(vec![
        Block::new(BlockKind::heading(1)).with_span("The harbour", InlineAttrs::default()),
        Block::new(BlockKind::Paragraph)
            .with_span("A quiet har", InlineAttrs::default())
            .with_span("bour", bold.clone())
            .with_span(" and the ", InlineAttrs::default())
            .with_span("old harbour wall", link.clone()),
        Block::new(BlockKind::BulletItem).with_span("no match here", InlineAttrs::default()),
    ]);
    let original = document.clone();
    let (tools, editor, cx) = open(document, cx);

    cx.simulate_keystrokes("cmd-alt-f");
    cx.simulate_input("harbour");
    assert_eq!(status(&tools, cx), "1 of 3");
    assert_eq!(
        layer(&editor, FIND_LAYER, cx),
        [
            TextRange::in_block(0, 4, 11),
            TextRange::in_block(1, 8, 15),
            TextRange::in_block(1, 28, 35),
        ]
    );
    // The match that straddles the bold boundary is one match and is
    // selected whole.
    cx.simulate_keystrokes("enter");
    assert_eq!(selection(&editor, cx), TextRange::in_block(1, 8, 15));

    // Replace all: one step, the text around keeps its formatting, the
    // replacement inside the link stays a link.
    cx.simulate_keystrokes("tab");
    cx.simulate_input("port");
    cx.simulate_keystrokes("cmd-enter");
    assert_eq!(status(&tools, cx), "Replaced 3");
    assert_eq!(
        text(&editor, cx),
        "The port\nA quiet port and the old port wall\nno match here"
    );
    editor.read_with(cx, |editor, _| {
        let blocks = editor.document().blocks();
        assert_eq!(blocks[0].kind(), &BlockKind::heading(1));
        let attrs = |offset: usize| blocks[1].attrs_at(offset).unwrap().clone();
        assert!(!attrs(7).bold, "the text before the match");
        assert!(!attrs(12).bold, "the text after the match");
        assert_eq!(attrs(21).link.as_deref(), Some("https://example.com"));
        assert_eq!(
            attrs(25).link.as_deref(),
            Some("https://example.com"),
            "the replacement inside the link"
        );
        assert!(attrs(25).italic);
        assert_eq!(attrs(30).link.as_deref(), Some("https://example.com"));
        assert_eq!(editor.state().undo_depth(), 1, "one undo step");
    });

    // One undo brings back the text and every attribute.
    cx.update(|window, cx| editor.read(cx).focus(window));
    cx.simulate_keystrokes("cmd-z");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().plain_text(), original.plain_text());
        for (now, before) in editor.document().blocks().iter().zip(original.blocks()) {
            let spans = |block: &Block| -> Vec<_> {
                block
                    .spans()
                    .map(|(range, attrs)| (range, attrs.clone()))
                    .collect()
            };
            assert_eq!(spans(now), spans(before));
        }
        assert!(!editor.state().can_undo());
    });
    // Undo also put the selection back on the match it was on.
    assert_eq!(status(&tools, cx), "2 of 3");
    assert_eq!(layer(&editor, FIND_LAYER, cx).len(), 3);
}

#[gpui::test]
fn find_bar_buttons_have_tooltips_with_their_keys(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, _editor, cx) = open(document(), cx);
    cx.simulate_keystrokes("cmd-f");
    let tooltip = |button: &str, cx: &mut VisualTestContext| {
        tools.read_with(cx, |tools, cx| tools.find_button_tooltip(button, cx))
    };
    let expected = [
        ("find-disclosure", "Show replace ⌥⌘F"),
        ("find-case", "Match case ⌥⌘C"),
        ("find-word", "Whole word ⌥⌘W"),
        ("find-previous", "Previous match ⇧⌘G"),
        ("find-next", "Next match ⌘G"),
        ("find-close", "Close ⎋"),
        ("find-replace-one", "Replace this match ↩"),
        ("find-replace-all", "Replace all matches ⌘↩"),
    ];
    assert_eq!(expected.map(|(button, _)| button), FIND_BUTTONS);
    for (button, text) in expected {
        assert_eq!(tooltip(button, cx).as_deref(), Some(text));
    }
    cx.simulate_keystrokes("cmd-alt-f");
    assert_eq!(
        tooltip("find-disclosure", cx).as_deref(),
        Some("Hide replace ⌥⌘F")
    );
    assert_eq!(tooltip("find-nothing", cx), None);

    // Resting the pointer on a button shows it, at the pointer; every
    // button of the bar is on screen to rest on. (gpui keeps the debug
    // bounds of a tooltip that is gone, so the test looks at where the
    // tooltip is: a stale one is over the button before.)
    assert!(cx.debug_bounds("tool-tip").is_none());
    for button in FIND_BUTTONS {
        let pointer = cx
            .debug_bounds(button)
            .unwrap_or_else(|| panic!("{button} is not drawn"))
            .center();
        cx.simulate_mouse_move(pointer, None, gpui::Modifiers::default());
        assert!(
            cx.debug_bounds("tool-tip")
                .is_none_or(|tip| (tip.origin.x - pointer.x).abs() > gpui::px(12.)),
            "the tooltip waits for the pointer to rest on {button}"
        );
        cx.executor().advance_clock(Duration::from_millis(600));
        cx.run_until_parked();
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        let tip = cx
            .debug_bounds("tool-tip")
            .unwrap_or_else(|| panic!("no tooltip over {button}"));
        assert!(
            (tip.origin.x - pointer.x).abs() <= gpui::px(12.) && tip.origin.y > pointer.y,
            "the tooltip of {button} is at {tip:?}, the pointer at {pointer:?}"
        );
        // Away from the bar it goes again.
        cx.simulate_mouse_move(
            gpui::point(gpui::px(400.), gpui::px(600.)),
            None,
            gpui::Modifiers::default(),
        );
        cx.run_until_parked();
    }
}

#[gpui::test]
fn the_selection_seeds_the_query(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(document(), cx);
    select(&editor, TextRange::in_block(1, 15, 18), cx);

    cx.simulate_keystrokes("cmd-f");
    tools.read_with(cx, |tools, _| assert_eq!(tools.find_query(), "cat"));
    assert_eq!(status(&tools, cx), "2 of 2");
    assert_eq!(selection(&editor, cx), TextRange::in_block(1, 15, 18));
    assert_eq!(layer(&editor, FIND_LAYER, cx).len(), 2);

    // The seeded text is selected in the field: typing replaces it.
    cx.simulate_input("mat");
    tools.read_with(cx, |tools, _| assert_eq!(tools.find_query(), "mat"));
    assert_eq!(status(&tools, cx), "1 of 1");

    // Reopening without a selection keeps the last query and does not move
    // the caret.
    cx.simulate_keystrokes("escape");
    caret(&editor, 2, 3, cx);
    cx.simulate_keystrokes("cmd-f");
    tools.read_with(cx, |tools, _| assert_eq!(tools.find_query(), "mat"));
    assert_eq!(status(&tools, cx), "1 match");
    assert_eq!(
        editor.read_with(cx, |editor, _| editor.selection()),
        Selection::caret(Position::new(2, 3))
    );
    cx.simulate_keystrokes("enter");
    assert_eq!(status(&tools, cx), "1 of 1");
}

#[gpui::test]
fn replaces_one_and_all(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(document(), cx);

    cx.simulate_keystrokes("cmd-alt-f");
    assert_eq!(active(&tools, cx), Some(ToolKind::Find));
    tools.read_with(cx, |tools, _| assert!(tools.find_replace_visible()));
    cx.simulate_input("cat");
    assert_eq!(status(&tools, cx), "1 of 2");

    // Tab moves to the replace field; typing there does not search.
    cx.simulate_keystrokes("tab");
    cx.simulate_input("lion");
    tools.read_with(cx, |tools, cx| {
        assert_eq!(tools.find_query(), "cat");
        assert_eq!(tools.find_replacement(cx), "lion");
    });

    // Enter in the replace field replaces the current match and moves on.
    cx.simulate_keystrokes("enter");
    assert_eq!(
        text(&editor, cx).lines().next(),
        Some("The lion sat on the mat.")
    );
    assert_eq!(status(&tools, cx), "1 of 1");
    assert_eq!(selection(&editor, cx), TextRange::in_block(1, 15, 18));
    cx.simulate_keystrokes("enter");
    assert_eq!(
        text(&editor, cx).lines().nth(1),
        Some("Then the other lion left; theatre people stayed.")
    );
    assert_eq!(status(&tools, cx), "No matches");
    // Each replacement is its own undo step.
    editor.update(cx, |editor, cx| editor.edit(|state| state.undo(), cx));
    cx.run_until_parked();
    // Results follow the document; undo put the selection back on the match.
    assert_eq!(status(&tools, cx), "1 of 1");
    editor.update(cx, |editor, cx| editor.edit(|state| state.undo(), cx));
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), document().plain_text());
    assert_eq!(tools.read_with(cx, |tools, _| tools.find_match_count()), 2);

    // Shift-tab goes back to the query; cmd-enter replaces all.
    cx.simulate_keystrokes("shift-tab");
    cx.simulate_input("the");
    assert_eq!(tools.read_with(cx, |tools, _| tools.find_match_count()), 8);
    cx.simulate_keystrokes("cmd-enter");
    assert_eq!(status(&tools, cx), "Replaced 8");
    assert_eq!(
        text(&editor, cx),
        "lion cat sat on lion mat.\n\
         lionn lion olionr cat left; lionatre people stayed.\n\
         Nothing here.\n\
         lion END of lion story"
    );
    assert_eq!(tools.read_with(cx, |tools, _| tools.find_match_count()), 0);
    assert!(layer(&editor, FIND_LAYER, cx).is_empty());

    // Replace all is one undo step.
    editor.update(cx, |editor, cx| editor.edit(|state| state.undo(), cx));
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), document().plain_text());
    assert_eq!(tools.read_with(cx, |tools, _| tools.find_match_count()), 8);

    // A replacement that contains the query is not found again.
    caret(&editor, 0, 0, cx);
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("cat");
    cx.simulate_keystrokes("tab cmd-a");
    cx.simulate_input("cat cat");
    cx.simulate_keystrokes("enter");
    assert_eq!(
        text(&editor, cx).lines().next(),
        Some("The cat cat sat on the mat.")
    );
    assert_eq!(selection(&editor, cx), TextRange::in_block(1, 15, 18));
}

#[gpui::test]
fn results_follow_document_changes(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(document(), cx);

    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("cat");
    assert_eq!(status(&tools, cx), "1 of 2");

    // Back in the document with the bar open: typing updates the results
    // and the highlights without moving the caret.
    cx.update(|window, cx| editor.read(cx).focus(window));
    caret(&editor, 2, 13, cx);
    assert_eq!(
        status(&tools, cx),
        "2 matches",
        "the selection is no longer on a match"
    );
    assert!(layer(&editor, FIND_CURRENT_LAYER, cx).is_empty());
    cx.simulate_input(" A cat.");
    assert_eq!(tools.read_with(cx, |tools, _| tools.find_match_count()), 3);
    assert_eq!(layer(&editor, FIND_LAYER, cx).len(), 3);
    assert_eq!(
        layer(&editor, FIND_LAYER, cx)[2],
        TextRange::in_block(2, 16, 19)
    );
    assert_eq!(
        editor.read_with(cx, |editor, _| editor.selection()),
        Selection::caret(Position::new(2, 20))
    );
    assert_eq!(active(&tools, cx), Some(ToolKind::Find));

    // cmd-g from the document goes to the next match from the caret.
    cx.simulate_keystrokes("cmd-g");
    assert_eq!(status(&tools, cx), "1 of 3");
    // cmd-f from the document brings focus back to the bar.
    cx.simulate_keystrokes("cmd-f");
    assert!(!editor_focused(&editor, cx));
    cx.simulate_keystrokes("enter");
    assert_eq!(status(&tools, cx), "2 of 3");

    // A new document: searched again.
    editor.update(cx, |editor, cx| {
        editor.set_document(paragraphs(&["one cat"]), cx)
    });
    cx.run_until_parked();
    assert_eq!(tools.read_with(cx, |tools, _| tools.find_match_count()), 1);
    assert_eq!(
        layer(&editor, FIND_LAYER, cx),
        [TextRange::in_block(0, 4, 7)]
    );
}

#[gpui::test]
fn read_only_documents_can_be_searched_but_not_replaced(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(document(), cx);
    editor.update(cx, |editor, cx| editor.set_read_only(true, cx));

    cx.simulate_keystrokes("cmd-alt-f");
    cx.simulate_input("cat");
    assert_eq!(status(&tools, cx), "1 of 2");
    cx.simulate_keystrokes("tab");
    cx.simulate_input("dog");
    cx.simulate_keystrokes("enter cmd-enter");
    assert_eq!(text(&editor, cx), document().plain_text());
    cx.simulate_keystrokes("escape");
    assert!(editor_focused(&editor, cx));
}

#[gpui::test]
fn escape_in_the_text_closes_the_bar(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(document(), cx);

    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("cat");
    cx.simulate_keystrokes("enter");
    assert_eq!(status(&tools, cx), "2 of 2");
    // The keyboard goes back to the text (as cmd-1 does in the app); the
    // bar and its highlights stay.
    cx.update(|window, cx| editor.read(cx).focus(window));
    cx.run_until_parked();
    assert_eq!(active(&tools, cx), Some(ToolKind::Find));
    assert_eq!(layer(&editor, FIND_LAYER, cx).len(), 2);

    // Escape from there closes the bar and takes the highlights away.
    cx.simulate_keystrokes("escape");
    assert_eq!(active(&tools, cx), None);
    assert!(editor_focused(&editor, cx));
    assert!(layer(&editor, FIND_LAYER, cx).is_empty());
    assert!(layer(&editor, FIND_CURRENT_LAYER, cx).is_empty());
    assert_eq!(selection(&editor, cx), TextRange::in_block(1, 15, 18));
}

#[gpui::test]
fn the_replace_key_on_an_open_bar_moves_to_the_replace_field(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(document(), cx);
    // Whether the accent frame is the one of the field `frame`.
    let ring_on = |frame: &'static str, cx: &mut VisualTestContext| {
        let ring = cx.debug_bounds("find-focus-ring").unwrap();
        let frame = cx.debug_bounds(frame).unwrap();
        frame.contains(&ring.origin) && frame.contains(&ring.bottom_right())
    };

    // A search is under way; asking for replace must not let the next
    // typing overwrite the query.
    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("cat");
    assert_eq!(status(&tools, cx), "1 of 2");
    assert!(ring_on("find-query-frame", cx));
    cx.simulate_keystrokes("cmd-alt-f");
    assert!(tools.read_with(cx, |tools, _| tools.find_replace_visible()));
    cx.simulate_input("dog");
    tools.read_with(cx, |tools, cx| {
        assert_eq!(tools.find_query(), "cat");
        assert_eq!(tools.find_replacement(cx), "dog");
    });
    assert_eq!(status(&tools, cx), "1 of 2");
    // The frame of the field that has the keyboard carries the accent.
    assert!(ring_on("find-replace-frame", cx));
    cx.simulate_keystrokes("enter");
    assert_eq!(
        text(&editor, cx).lines().next(),
        Some("The dog sat on the mat.")
    );
    cx.simulate_keystrokes("shift-tab");
    assert!(ring_on("find-query-frame", cx));

    // The same from the text, with the bar open and the row hidden.
    cx.simulate_keystrokes("cmd-alt-f");
    assert!(!tools.read_with(cx, |tools, _| tools.find_replace_visible()));
    cx.update(|window, cx| editor.read(cx).focus(window));
    cx.run_until_parked();
    cx.simulate_keystrokes("cmd-alt-f");
    cx.simulate_input("fox");
    tools.read_with(cx, |tools, cx| {
        assert_eq!(tools.find_query(), "cat");
        assert_eq!(tools.find_replacement(cx), "fox");
    });

    // With nothing to find yet, the query comes first.
    cx.simulate_keystrokes("escape");
    cx.simulate_keystrokes("cmd-f cmd-a backspace cmd-alt-f");
    cx.simulate_input("mat");
    tools.read_with(cx, |tools, cx| {
        assert_eq!(tools.find_query(), "mat");
        assert_eq!(tools.find_replacement(cx), "");
    });
}
