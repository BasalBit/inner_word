//! Starting a comment from the editor: the composer, submit, cancel, undo.

use gpui::TestAppContext;
use writer_comments::{CommentsEvent, DRAFT_HIGHLIGHT_LAYER, DraftKind};
use writer_core::{Position, Selection, TextRange};

use super::support::{AUTHOR, Focus, NOW, open, sample};

#[gpui::test]
fn add_comment_on_a_selection_and_undo_it(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    let quick = TextRange::in_block(0, 4, 9);
    f.panel.read_with(cx, |panel, _| {
        assert!(panel.is_visible());
        assert!(!panel.is_open(), "no comments, no panel");
    });

    f.select(0, 4, 9, cx);
    cx.simulate_keystrokes("cmd-alt-m");
    f.panel.read_with(cx, |panel, _| {
        assert_eq!(panel.draft(), Some(&DraftKind::New { range: quick }));
        assert!(panel.is_open());
    });
    assert_eq!(f.focus(cx), Focus::Composer);
    // The target is marked in the text while the composer is open.
    f.editor.read_with(cx, |editor, _| {
        assert_eq!(editor.highlights(DRAFT_HIGHLIGHT_LAYER), &[quick]);
    });

    // Letters that are commands in the thread list are text here.
    cx.simulate_input("Tighten this, je rex");
    assert_eq!(f.composer_text(cx), "Tighten this, je rex");
    cx.simulate_keystrokes("enter");

    let id = f.active(cx).expect("the new thread is active");
    f.panel.read_with(cx, |panel, _| {
        assert!(!panel.is_composing());
        assert_eq!(panel.threads().len(), 1);
        let entry = panel.thread(id).unwrap();
        assert_eq!(entry.quote, "quick");
        assert_eq!(entry.ranges, vec![quick]);
        assert_eq!(entry.thread.body, "Tighten this, je rex");
        assert_eq!(entry.thread.author, AUTHOR);
        assert_eq!(entry.thread.created_at, NOW);
        assert!(!entry.thread.resolved);
    });
    assert_eq!(f.composer_text(cx), "");
    f.editor.read_with(cx, |editor, _| {
        assert!(editor.highlights(DRAFT_HIGHLIGHT_LAYER).is_empty());
        assert!(editor.document().comment(id).is_some());
        assert_eq!(editor.document().comment_ranges(id), vec![quick]);
        // The user's selection is as they left it.
        assert_eq!(editor.selection(), Selection::from_range(quick));
    });
    assert_eq!(f.focus(cx), Focus::Editor);

    // One undo step in the editor removes the thread and its anchor.
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(f.comment_count(cx), 0);
    f.editor.read_with(cx, |editor, _| {
        assert!(editor.document().all_comment_ranges().is_empty());
    });
    f.panel.read_with(cx, |panel, _| {
        assert!(panel.threads().is_empty());
        assert!(!panel.is_open());
    });
    assert_eq!(f.active(cx), None);

    cx.simulate_keystrokes("cmd-shift-z");
    f.panel.read_with(cx, |panel, _| {
        assert_eq!(panel.threads().len(), 1);
        assert_eq!(panel.threads()[0].ranges, vec![quick]);
        assert_eq!(panel.threads()[0].thread.body, "Tighten this, je rex");
    });
}

#[gpui::test]
fn a_collapsed_caret_comments_on_its_word(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    // Inside "brown".
    f.caret(0, 12, cx);
    cx.simulate_keystrokes("cmd-alt-m");
    let brown = TextRange::in_block(0, 10, 15);
    f.panel.read_with(cx, |panel, _| {
        assert_eq!(panel.draft(), Some(&DraftKind::New { range: brown }));
    });
    // The caret itself did not move.
    assert_eq!(f.selection(cx), Selection::caret(Position::new(0, 12)));

    // shift-enter is a line break, cmd-enter submits.
    cx.simulate_input("Which brown");
    cx.simulate_keystrokes("shift-enter");
    cx.simulate_input("exactly");
    cx.simulate_keystrokes("cmd-enter");

    let id = f.active(cx).expect("the new thread is active");
    f.panel.read_with(cx, |panel, _| {
        let entry = panel.thread(id).unwrap();
        assert_eq!(entry.thread.body, "Which brown\nexactly");
        assert_eq!(entry.ranges, vec![brown]);
        assert_eq!(entry.quote, "brown");
    });
    assert_eq!(f.selection(cx), Selection::caret(Position::new(0, 12)));
    assert_eq!(f.focus(cx), Focus::Editor);
}

#[gpui::test]
fn escape_cancels_and_returns_to_the_editor(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    f.select(1, 2, 8, cx);
    cx.simulate_keystrokes("cmd-alt-m");
    cx.simulate_input("Never mind");
    cx.simulate_keystrokes("escape");

    assert_eq!(f.focus(cx), Focus::Editor);
    assert_eq!(f.comment_count(cx), 0);
    f.panel.read_with(cx, |panel, _| {
        assert!(!panel.is_composing());
        assert!(!panel.is_open());
        assert!(panel.is_visible());
    });
    f.editor.read_with(cx, |editor, _| {
        assert!(editor.highlights(DRAFT_HIGHLIGHT_LAYER).is_empty());
        assert!(!editor.state().can_undo(), "cancelling leaves no undo step");
        assert_eq!(
            editor.selection(),
            Selection::from_range(TextRange::in_block(1, 2, 8))
        );
    });
    // The discarded text does not come back with the next draft.
    cx.simulate_keystrokes("cmd-alt-m");
    assert_eq!(f.composer_text(cx), "");
}

#[gpui::test]
fn an_empty_comment_is_not_saved(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    f.select(0, 4, 9, cx);
    cx.simulate_keystrokes("cmd-alt-m enter");
    assert_eq!(f.focus(cx), Focus::Composer);
    cx.simulate_input("   ");
    cx.simulate_keystrokes("cmd-enter");
    assert_eq!(f.focus(cx), Focus::Composer);
    assert_eq!(f.comment_count(cx), 0);
    f.panel
        .read_with(cx, |panel, _| assert!(panel.is_composing()));
}

#[gpui::test]
fn nothing_to_anchor_to_opens_nothing(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    // Between the full stop and the end of block 0: no word touches the caret.
    f.caret(0, 44, cx);
    cx.simulate_keystrokes("cmd-alt-m");
    f.panel
        .read_with(cx, |panel, _| assert!(!panel.is_composing()));
    assert_eq!(f.focus(cx), Focus::Editor);

    // A read-only editor takes no comments.
    f.select(0, 4, 9, cx);
    f.editor
        .update(cx, |editor, cx| editor.set_read_only(true, cx));
    cx.simulate_keystrokes("cmd-alt-m");
    f.panel
        .read_with(cx, |panel, _| assert!(!panel.is_composing()));
    assert_eq!(f.focus(cx), Focus::Editor);
}

#[gpui::test]
fn add_comment_again_points_the_open_composer_at_the_new_selection(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    f.select(0, 4, 9, cx);
    cx.simulate_keystrokes("cmd-alt-m");
    cx.simulate_input("Keep me");

    // Back in the text, another selection, the same key.
    cx.update(|window, cx| f.editor.read(cx).focus(window));
    f.select(2, 4, 9, cx);
    cx.simulate_keystrokes("cmd-alt-m");
    let third = TextRange::in_block(2, 4, 9);
    f.panel.read_with(cx, |panel, _| {
        assert_eq!(panel.draft(), Some(&DraftKind::New { range: third }));
    });
    assert_eq!(f.focus(cx), Focus::Composer);
    assert_eq!(f.composer_text(cx), "Keep me");
    f.editor.read_with(cx, |editor, _| {
        assert_eq!(editor.highlights(DRAFT_HIGHLIGHT_LAYER), &[third]);
    });
    cx.simulate_keystrokes("enter");
    let id = f.active(cx).unwrap();
    f.panel.read_with(cx, |panel, _| {
        assert_eq!(panel.thread(id).unwrap().ranges, vec![third]);
    });
}

#[gpui::test]
fn adding_while_comments_are_hidden_shows_them(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    cx.simulate_keystrokes("cmd-alt-c");
    f.panel
        .read_with(cx, |panel, _| assert!(!panel.is_visible()));

    f.select(0, 4, 9, cx);
    cx.simulate_keystrokes("cmd-alt-m");
    f.panel.read_with(cx, |panel, _| {
        assert!(panel.is_visible());
        assert!(panel.is_open());
    });
    f.editor
        .read_with(cx, |editor, _| assert!(editor.config().comments_visible));

    // Cancelling puts the flag back; submitting keeps comments visible.
    cx.simulate_keystrokes("escape");
    f.panel
        .read_with(cx, |panel, _| assert!(!panel.is_visible()));
    f.editor
        .read_with(cx, |editor, _| assert!(!editor.config().comments_visible));

    cx.simulate_keystrokes("cmd-alt-m");
    cx.simulate_input("Stay");
    cx.simulate_keystrokes("enter");
    f.panel.read_with(cx, |panel, _| {
        assert!(panel.is_visible());
        assert_eq!(panel.threads().len(), 1);
    });
    assert_eq!(
        f.events(cx)
            .into_iter()
            .filter(|event| matches!(event, CommentsEvent::VisibilityChanged(_)))
            .collect::<Vec<_>>(),
        vec![
            CommentsEvent::VisibilityChanged(false),
            CommentsEvent::VisibilityChanged(true),
            CommentsEvent::VisibilityChanged(false),
            CommentsEvent::VisibilityChanged(true),
        ]
    );
}
