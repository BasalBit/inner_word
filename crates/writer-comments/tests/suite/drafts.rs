//! What happens to a comment that is still being written when the world
//! around it changes: the document is edited, the commented text is
//! deleted, comments are hidden, another composer opens, the document is
//! switched. Nothing that was typed is thrown away, and nothing ends up in
//! another document.

use gpui::TestAppContext;
use writer_comments::{DRAFT_HIGHLIGHT_LAYER, DraftKind};
use writer_core::{Block, Document, EditorState, Position, TextRange};

use super::support::{Focus, open, sample};

fn edit<R>(
    f: &super::support::Fixture,
    cx: &mut gpui::VisualTestContext,
    change: impl FnOnce(&mut EditorState) -> R,
) -> R {
    let result = f.editor.update(cx, |editor, cx| editor.edit(change, cx));
    cx.run_until_parked();
    result
}

fn draft_range(f: &super::support::Fixture, cx: &mut gpui::VisualTestContext) -> TextRange {
    f.panel.read_with(cx, |panel, _| match panel.draft() {
        Some(DraftKind::New { range }) => *range,
        other => panic!("no new-comment draft: {other:?}"),
    })
}

fn text_in(
    f: &super::support::Fixture,
    range: TextRange,
    cx: &mut gpui::VisualTestContext,
) -> String {
    f.editor
        .read_with(cx, |editor, _| editor.document().text_in(range))
}

#[gpui::test]
fn the_target_of_a_draft_follows_edits_of_the_document(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    // "brown" in "The quick brown fox jumps over the lazy dog."
    f.select(0, 10, 15, cx);
    cx.simulate_keystrokes("cmd-alt-m");
    cx.simulate_input("Colour?");

    // Text typed before it, in the same paragraph.
    edit(&f, cx, |state| {
        state.set_caret(Position::new(0, 4));
        state.insert_text("very ")
    });
    assert_eq!(draft_range(&f, cx), TextRange::in_block(0, 15, 20));
    f.editor.read_with(cx, |editor, _| {
        assert_eq!(
            editor.highlights(DRAFT_HIGHLIGHT_LAYER),
            &[TextRange::in_block(0, 15, 20)],
            "the mark in the text moves along"
        );
    });

    // A paragraph inserted above, then the paragraph split before the
    // target and joined again.
    edit(&f, cx, |state| {
        state.set_caret(Position::new(0, 0));
        state.insert_text("A new first paragraph.");
        state.insert_paragraph_break()
    });
    assert_eq!(draft_range(&f, cx), TextRange::in_block(1, 15, 20));
    edit(&f, cx, |state| {
        state.set_caret(Position::new(1, 4));
        state.insert_paragraph_break()
    });
    assert_eq!(draft_range(&f, cx), TextRange::in_block(2, 11, 16));
    edit(&f, cx, |state| {
        state.set_caret(Position::new(2, 0));
        state.backspace()
    });
    assert_eq!(draft_range(&f, cx), TextRange::in_block(1, 15, 20));

    // Text typed right behind it is not part of it; text typed into it is.
    edit(&f, cx, |state| {
        state.set_caret(Position::new(1, 20));
        state.insert_text("ish")
    });
    assert_eq!(text_in(&f, draft_range(&f, cx), cx), "brown");
    edit(&f, cx, |state| {
        state.set_caret(Position::new(1, 17));
        state.insert_text("--")
    });
    assert_eq!(text_in(&f, draft_range(&f, cx), cx), "br--own");

    // Undo of all of it: back where it started.
    while edit(&f, cx, |state| state.undo()) {}
    assert_eq!(draft_range(&f, cx), TextRange::in_block(0, 10, 15));

    // The comment lands on the text it was started on.
    edit(&f, cx, |state| {
        state.set_caret(Position::new(0, 0));
        state.insert_text("Look: ")
    });
    cx.update(|window, cx| f.panel.read(cx).composer().read(cx).focus(window));
    cx.simulate_keystrokes("enter");
    let id = f.active(cx).expect("the comment was saved");
    f.panel.read_with(cx, |panel, _| {
        let entry = panel.thread(id).unwrap();
        assert_eq!(entry.quote, "brown");
        assert_eq!(entry.ranges, vec![TextRange::in_block(0, 16, 21)]);
        assert_eq!(entry.thread.body, "Colour?");
    });
}

#[gpui::test]
fn a_draft_whose_text_was_deleted_is_kept_open(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    f.select(0, 10, 15, cx);
    cx.simulate_keystrokes("cmd-alt-m");
    cx.simulate_input("Colour?");

    // The sentence around the target is deleted.
    edit(&f, cx, |state| {
        state.select_range(TextRange::in_block(0, 4, 20));
        state.delete_selection()
    });
    f.panel.read_with(cx, |panel, _| {
        assert!(panel.draft_target_lost());
        assert!(panel.is_composing());
    });
    assert!(draft_range(&f, cx).is_empty());
    assert!(
        cx.debug_bounds("comment-target-lost").is_some(),
        "the composer says so in place of the quote"
    );
    f.editor.read_with(cx, |editor, _| {
        assert!(editor.highlights(DRAFT_HIGHLIGHT_LAYER).is_empty());
    });

    // It cannot be saved, and what was written stays in the composer.
    cx.update(|window, cx| f.panel.read(cx).composer().read(cx).focus(window));
    cx.simulate_keystrokes("enter");
    assert_eq!(f.comment_count(cx), 0);
    assert_eq!(f.composer_text(cx), "Colour?");
    assert_eq!(f.focus(cx), Focus::Composer);
    cx.simulate_keystrokes("cmd-enter");
    assert_eq!(f.comment_count(cx), 0);
    assert_eq!(f.composer_text(cx), "Colour?");

    // Other text typed at that place does not become the target.
    edit(&f, cx, |state| {
        state.set_caret(Position::new(0, 4));
        state.insert_text("slow ")
    });
    f.panel
        .read_with(cx, |panel, _| assert!(panel.draft_target_lost()));
    edit(&f, cx, |state| state.undo());

    // Undo brings the text back, and the target with it.
    edit(&f, cx, |state| state.undo());
    f.panel
        .read_with(cx, |panel, _| assert!(!panel.draft_target_lost()));
    assert_eq!(draft_range(&f, cx), TextRange::in_block(0, 10, 15));
    cx.simulate_keystrokes("enter");
    let id = f.active(cx).unwrap();
    f.panel.read_with(cx, |panel, _| {
        assert_eq!(panel.thread(id).unwrap().quote, "brown");
    });

    // Or the composer is pointed at other text with the same key.
    f.select(1, 2, 8, cx);
    cx.simulate_keystrokes("cmd-alt-m");
    cx.simulate_input("Second");
    edit(&f, cx, |state| {
        state.select_range(TextRange::in_block(1, 0, 10));
        state.delete_selection()
    });
    f.panel
        .read_with(cx, |panel, _| assert!(panel.draft_target_lost()));
    f.select(2, 4, 9, cx);
    cx.simulate_keystrokes("cmd-alt-m");
    f.panel
        .read_with(cx, |panel, _| assert!(!panel.draft_target_lost()));
    assert_eq!(f.composer_text(cx), "Second");
    cx.simulate_keystrokes("enter");
    assert_eq!(f.comment_count(cx), 2);
}

#[gpui::test]
fn a_comment_on_a_selection_that_spans_blocks(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    // From "lazy" in the first paragraph to "second" in the next.
    edit(&f, cx, |state| {
        state.select_range(TextRange::new(Position::new(0, 35), Position::new(1, 8)))
    });
    cx.simulate_keystrokes("cmd-alt-m");
    assert_eq!(
        draft_range(&f, cx),
        TextRange::new(Position::new(0, 35), Position::new(1, 8))
    );
    cx.simulate_input("One thought across two paragraphs");
    // An edit in the first of the two blocks while it is written.
    edit(&f, cx, |state| {
        state.set_caret(Position::new(0, 0));
        state.insert_text(">> ")
    });
    cx.update(|window, cx| f.panel.read(cx).composer().read(cx).focus(window));
    cx.simulate_keystrokes("cmd-enter");

    let id = f.active(cx).expect("saved");
    f.panel.read_with(cx, |panel, _| {
        let entry = panel.thread(id).unwrap();
        assert_eq!(
            entry.ranges,
            vec![TextRange::new(Position::new(0, 38), Position::new(1, 8))],
            "one anchor across the paragraph break"
        );
        assert_eq!(entry.quote, "lazy dog. A second");
    });
    // One thread, one undo step.
    edit(&f, cx, |state| state.undo());
    assert_eq!(f.comment_count(cx), 0);
    edit(&f, cx, |state| state.redo());
    assert_eq!(f.comment_count(cx), 1);

    // Next comment stops once at the thread, at its start, and wraps.
    f.caret(2, 0, cx);
    cx.simulate_keystrokes("cmd-alt-]");
    assert_eq!(f.selection(cx).head, Position::new(0, 38));
    cx.simulate_keystrokes("cmd-alt-]");
    assert_eq!(f.selection(cx).head, Position::new(0, 38));
    assert_eq!(f.active(cx), Some(id));
    // A caret in the second block's part marks the same thread.
    f.caret(2, 0, cx);
    f.caret(1, 4, cx);
    assert_eq!(f.active(cx), Some(id));
}

#[gpui::test]
fn hiding_comments_keeps_what_was_written(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    f.select(0, 10, 15, cx);
    cx.simulate_keystrokes("cmd-alt-m");
    cx.simulate_input("Not finished");
    cx.simulate_keystrokes("cmd-alt-c");
    assert_eq!(f.focus(cx), Focus::Editor);
    f.panel.read_with(cx, |panel, _| {
        assert!(!panel.is_visible());
        assert!(!panel.is_open());
        assert!(!panel.is_composing());
        assert_eq!(panel.kept_drafts(), 1);
    });
    assert_eq!(f.comment_count(cx), 0);
    f.editor.read_with(cx, |editor, _| {
        assert!(editor.highlights(DRAFT_HIGHLIGHT_LAYER).is_empty());
    });

    // The document is edited while comments are hidden.
    edit(&f, cx, |state| {
        state.set_caret(Position::new(0, 0));
        state.insert_text("Well. ")
    });

    // Showing comments brings the composer back, on the same text. The
    // keyboard stays in the text.
    cx.simulate_keystrokes("cmd-alt-c");
    f.panel.read_with(cx, |panel, _| {
        assert!(panel.is_open());
        assert_eq!(panel.kept_drafts(), 0);
    });
    assert_eq!(f.composer_text(cx), "Not finished");
    assert_eq!(draft_range(&f, cx), TextRange::in_block(0, 16, 21));
    assert_eq!(f.focus(cx), Focus::Editor);
    f.editor.read_with(cx, |editor, _| {
        assert_eq!(
            editor.highlights(DRAFT_HIGHLIGHT_LAYER),
            &[TextRange::in_block(0, 16, 21)]
        );
    });
    // cmd-2 goes to the open composer; escape discards it for good.
    cx.simulate_keystrokes("cmd-2");
    assert_eq!(f.focus(cx), Focus::Composer);
    cx.simulate_keystrokes("escape");
    f.panel.read_with(cx, |panel, _| {
        assert!(!panel.is_composing());
        assert_eq!(panel.kept_drafts(), 0);
        assert!(panel.is_visible(), "comments were visible before the draft");
    });
}

#[gpui::test]
fn another_composer_does_not_overwrite_a_draft(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    let thread = f.comment(1, 2, 8, "Existing", cx);
    f.select(0, 10, 15, cx);
    cx.simulate_keystrokes("cmd-alt-m");
    cx.simulate_input("New one, unfinished");

    // A reply is started while the new comment is still open.
    cx.simulate_keystrokes("cmd-2");
    assert_eq!(f.focus(cx), Focus::Composer);
    f.panel.update_in(cx, |panel, window, cx| {
        panel.select_thread(Some(thread), cx);
        panel.reply(window, cx);
    });
    cx.run_until_parked();
    f.panel.read_with(cx, |panel, _| {
        assert_eq!(panel.draft(), Some(&DraftKind::Reply { thread }));
        assert_eq!(panel.kept_drafts(), 1);
    });
    assert_eq!(f.composer_text(cx), "");
    cx.simulate_input("A reply");
    cx.simulate_keystrokes("enter");
    f.panel.read_with(cx, |panel, _| {
        assert_eq!(
            panel.thread(thread).unwrap().thread.replies[0].body,
            "A reply"
        );
        // The new comment has its composer back.
        assert_eq!(
            panel.draft(),
            Some(&DraftKind::New {
                range: TextRange::in_block(0, 10, 15)
            })
        );
        assert_eq!(panel.kept_drafts(), 0);
    });
    assert_eq!(f.composer_text(cx), "New one, unfinished");

    // An edit that changed nothing is not worth keeping.
    cx.simulate_keystrokes("cmd-2 escape");
    f.panel
        .read_with(cx, |panel, _| assert!(!panel.is_composing()));
    cx.simulate_keystrokes("cmd-2 e");
    assert_eq!(f.composer_text(cx), "Existing");
    f.select(2, 4, 9, cx);
    cx.simulate_keystrokes("cmd-alt-m");
    f.panel.read_with(cx, |panel, _| {
        assert!(matches!(panel.draft(), Some(DraftKind::New { .. })));
        assert_eq!(panel.kept_drafts(), 0);
    });
}

fn other_document() -> Document {
    Document::from_blocks(vec![
        Block::paragraph("Another document altogether."),
        Block::paragraph("With a second paragraph of its own."),
    ])
}

#[gpui::test]
fn switching_documents_keeps_a_draft_for_its_own_document(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    f.select(0, 10, 15, cx);
    cx.simulate_keystrokes("cmd-alt-m");
    cx.simulate_input("About the fox");
    let original = f
        .editor
        .read_with(cx, |editor, _| editor.document().clone());

    // Another document is opened in the same editor.
    f.editor
        .update(cx, |editor, cx| editor.set_document(other_document(), cx));
    cx.run_until_parked();
    f.panel.read_with(cx, |panel, _| {
        assert!(!panel.is_composing(), "the draft is not shown here");
        assert!(!panel.is_open());
        assert_eq!(panel.kept_drafts(), 1);
    });
    assert_eq!(f.composer_text(cx), "");
    assert_eq!(f.focus(cx), Focus::Editor);
    f.editor.read_with(cx, |editor, _| {
        assert!(editor.highlights(DRAFT_HIGHLIGHT_LAYER).is_empty());
        assert_eq!(editor.document().comment_count(), 0);
    });

    // A comment written in the other document is its own.
    let there = f.comment(0, 8, 16, "Here", cx);
    f.panel.read_with(cx, |panel, _| {
        assert_eq!(panel.thread(there).unwrap().quote, "document");
        assert_eq!(panel.threads().len(), 1);
        assert_eq!(panel.kept_drafts(), 1, "the kept draft does not fit here");
        assert!(!panel.is_composing());
    });

    // Back in the first document the composer is open again, with the
    // text, on the same words. The keyboard stays in the text.
    f.editor
        .update(cx, |editor, cx| editor.set_document(original, cx));
    cx.run_until_parked();
    f.panel.read_with(cx, |panel, _| {
        assert_eq!(panel.kept_drafts(), 0);
        assert!(panel.is_open());
    });
    assert_eq!(draft_range(&f, cx), TextRange::in_block(0, 10, 15));
    assert_eq!(f.composer_text(cx), "About the fox");
    assert_eq!(f.focus(cx), Focus::Editor);
    cx.simulate_keystrokes("cmd-2");
    assert_eq!(f.focus(cx), Focus::Composer);
    cx.simulate_keystrokes("enter");
    let id = f.active(cx).unwrap();
    f.panel.read_with(cx, |panel, _| {
        let entry = panel.thread(id).unwrap();
        assert_eq!(entry.quote, "brown");
        assert_eq!(entry.thread.body, "About the fox");
        assert_eq!(panel.threads().len(), 1);
    });
}

#[gpui::test]
fn a_kept_reply_waits_for_its_thread(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    let thread = f.comment(0, 4, 9, "Tighten this", cx);
    cx.simulate_keystrokes("cmd-2 r");
    cx.simulate_input("Half a reply");
    let original = f
        .editor
        .read_with(cx, |editor, _| editor.document().clone());

    // A document that happens to have a thread with the same id.
    let mut state = EditorState::new(other_document());
    state.select_range(TextRange::in_block(0, 0, 7));
    let same_id = state
        .add_comment("Someone else's comment", "Bea", 5)
        .unwrap();
    assert_eq!(same_id, thread);
    f.editor.update(cx, |editor, cx| {
        editor.set_document(state.into_document(), cx)
    });
    cx.run_until_parked();
    f.panel.read_with(cx, |panel, _| {
        assert!(!panel.is_composing(), "not a reply to this thread");
        assert_eq!(panel.kept_drafts(), 1);
        assert!(panel.threads()[0].thread.replies.is_empty());
    });
    // The list had the keyboard before; it is somewhere on screen now.
    assert_ne!(f.focus(cx), Focus::Composer);
    assert_ne!(f.focus(cx), Focus::Elsewhere);

    f.editor
        .update(cx, |editor, cx| editor.set_document(original, cx));
    cx.run_until_parked();
    f.panel.read_with(cx, |panel, _| {
        assert_eq!(panel.draft(), Some(&DraftKind::Reply { thread }));
        assert_eq!(panel.kept_drafts(), 0);
    });
    assert_eq!(f.composer_text(cx), "Half a reply");
}
