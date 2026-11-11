use gpui::{Entity, TestAppContext, VisualTestContext};
use writer_core::{Document, Position, Selection, TextRange};
use writer_editor::Editor;
use writer_editor_tools::{EditorTools, FIND_CURRENT_LAYER, FIND_LAYER, ToolKind};

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
    assert_eq!(text(&editor, cx).lines().next(), Some("The cat sat on the mat."));
    tools.read_with(cx, |tools, _| {
        assert_eq!(tools.find_query(), "the");
        assert_eq!(tools.find_match_count(), 8);
        assert_eq!(tools.find_current_match(), Some(1));
    });
    assert_eq!(status(&tools, cx), "1 of 8");
    assert_eq!(layer(&editor, FIND_LAYER, cx).len(), 8);
    assert_eq!(layer(&editor, FIND_CURRENT_LAYER, cx), [TextRange::in_block(0, 0, 3)]);
    assert_eq!(selection(&editor, cx), TextRange::in_block(0, 0, 3));

    // enter / shift-enter and cmd-g / cmd-shift-g.
    cx.simulate_keystrokes("enter");
    assert_eq!(status(&tools, cx), "2 of 8");
    assert_eq!(selection(&editor, cx), TextRange::in_block(0, 15, 18));
    cx.simulate_keystrokes("cmd-g");
    assert_eq!(status(&tools, cx), "3 of 8");
    assert_eq!(selection(&editor, cx), TextRange::in_block(1, 0, 3));
    assert_eq!(layer(&editor, FIND_CURRENT_LAYER, cx), [TextRange::in_block(1, 0, 3)]);
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
    assert_eq!(selection(&editor, cx), TextRange::in_block(1, 0, 4), "selection stays");
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
    assert_eq!(text(&editor, cx).lines().next(), Some("The lion sat on the mat."));
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
    assert_eq!(text(&editor, cx).lines().next(), Some("The cat cat sat on the mat."));
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
    assert_eq!(status(&tools, cx), "2 matches", "the selection is no longer on a match");
    assert!(layer(&editor, FIND_CURRENT_LAYER, cx).is_empty());
    cx.simulate_input(" A cat.");
    assert_eq!(tools.read_with(cx, |tools, _| tools.find_match_count()), 3);
    assert_eq!(layer(&editor, FIND_LAYER, cx).len(), 3);
    assert_eq!(layer(&editor, FIND_LAYER, cx)[2], TextRange::in_block(2, 16, 19));
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
    editor.update(cx, |editor, cx| editor.set_document(paragraphs(&["one cat"]), cx));
    cx.run_until_parked();
    assert_eq!(tools.read_with(cx, |tools, _| tools.find_match_count()), 1);
    assert_eq!(layer(&editor, FIND_LAYER, cx), [TextRange::in_block(0, 4, 7)]);
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
