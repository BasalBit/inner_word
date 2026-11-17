//! Working on threads from the thread list: reply, edit, resolve and
//! reopen, delete, detached threads; undo of each.

use gpui::{Modifiers, TestAppContext, VisualTestContext};
use innerword_comments::{DeleteTarget, DraftKind};
use innerword_core::{Block, Document, Position, ReplyId, TextRange};

use super::support::{AUTHOR, Fixture, Focus, NOW, open, sample};

#[gpui::test]
fn reply_from_the_list_and_undo_there(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    let id = f.comment(0, 4, 9, "Tighten this", cx);

    cx.simulate_keystrokes("cmd-2");
    assert_eq!(f.focus(cx), Focus::List);
    assert_eq!(f.active(cx), Some(id));

    f.clock.set(NOW + 120_000);
    cx.simulate_keystrokes("r");
    f.panel.read_with(cx, |panel, _| {
        assert_eq!(panel.draft(), Some(&DraftKind::Reply { thread: id }));
    });
    assert_eq!(f.focus(cx), Focus::Composer);
    // j, k, r, e, x, s, h, l are list commands; in the composer they are text.
    cx.simulate_input("jerks relax hl");
    assert_eq!(f.composer_text(cx), "jerks relax hl");
    cx.simulate_keystrokes("enter");

    // Back in the list, where the reply was started.
    assert_eq!(f.focus(cx), Focus::List);
    assert_eq!(f.active(cx), Some(id));
    f.panel.read_with(cx, |panel, _| {
        let thread = &panel.thread(id).unwrap().thread;
        assert_eq!(thread.replies.len(), 1);
        assert_eq!(thread.replies[0].body, "jerks relax hl");
        assert_eq!(thread.replies[0].author, AUTHOR);
        assert_eq!(thread.replies[0].created_at, NOW + 120_000);
        assert_eq!(thread.created_at, NOW);
    });

    // `enter` replies too; escape cancels back to the list.
    cx.simulate_keystrokes("enter");
    assert_eq!(f.focus(cx), Focus::Composer);
    cx.simulate_input("no");
    cx.simulate_keystrokes("escape");
    assert_eq!(f.focus(cx), Focus::List);
    f.panel.read_with(cx, |panel, _| {
        assert!(!panel.is_composing());
        assert_eq!(panel.thread(id).unwrap().thread.replies.len(), 1);
    });

    // Undo and redo without leaving the list.
    cx.simulate_keystrokes("cmd-z");
    f.panel.read_with(cx, |panel, _| {
        assert!(panel.thread(id).unwrap().thread.replies.is_empty());
    });
    assert_eq!(f.focus(cx), Focus::List);
    cx.simulate_keystrokes("cmd-shift-z");
    f.panel.read_with(cx, |panel, _| {
        assert_eq!(panel.thread(id).unwrap().thread.replies.len(), 1);
    });

    // escape leaves the list for the editor; the thread stays marked.
    cx.simulate_keystrokes("escape");
    assert_eq!(f.focus(cx), Focus::Editor);
    assert_eq!(f.active(cx), Some(id));
}

#[gpui::test]
fn edit_the_body_and_undo(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    let id = f.comment(0, 4, 9, "Tighten this", cx);
    cx.simulate_keystrokes("cmd-2 e");
    f.panel.read_with(cx, |panel, _| {
        assert_eq!(panel.draft(), Some(&DraftKind::EditComment { thread: id }));
    });
    assert_eq!(f.focus(cx), Focus::Composer);
    // The composer starts with the current body, caret at its end.
    assert_eq!(f.composer_text(cx), "Tighten this");
    cx.simulate_input(" sentence");
    cx.simulate_keystrokes("cmd-enter");

    assert_eq!(f.focus(cx), Focus::List);
    let body = |f: &super::support::Fixture, cx: &mut gpui::VisualTestContext| {
        f.panel
            .read_with(cx, |panel, _| panel.thread(id).unwrap().thread.body.clone())
    };
    assert_eq!(body(&f, cx), "Tighten this sentence");
    f.editor.read_with(cx, |editor, _| {
        assert_eq!(
            editor.document().comment(id).unwrap().body,
            "Tighten this sentence"
        );
    });

    cx.simulate_keystrokes("cmd-z");
    assert_eq!(body(&f, cx), "Tighten this");
    cx.simulate_keystrokes("cmd-shift-z");
    assert_eq!(body(&f, cx), "Tighten this sentence");

    // Replacing everything, and cancelling an edit.
    cx.simulate_keystrokes("e cmd-a");
    cx.simulate_input("Cut it");
    cx.simulate_keystrokes("enter");
    assert_eq!(body(&f, cx), "Cut it");
    cx.simulate_keystrokes("e");
    cx.simulate_input(" maybe");
    cx.simulate_keystrokes("escape");
    assert_eq!(body(&f, cx), "Cut it");
    assert_eq!(f.focus(cx), Focus::List);
}

#[gpui::test]
fn resolve_hides_reopen_brings_back(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    let first = f.comment(0, 4, 9, "First", cx);
    let second = f.comment(1, 2, 8, "Second", cx);
    assert_eq!(f.listed_ids(cx), vec![first, second]);

    f.caret(0, 5, cx);
    cx.simulate_keystrokes("cmd-2");
    assert_eq!(f.active(cx), Some(first));
    cx.simulate_keystrokes("x");

    // Resolved: out of the list, its neighbour is active, focus stays.
    assert_eq!(f.listed_ids(cx), vec![second]);
    assert_eq!(f.active(cx), Some(second));
    assert_eq!(f.focus(cx), Focus::List);
    f.panel.read_with(cx, |panel, _| {
        assert!(panel.thread(first).unwrap().is_resolved());
        assert_eq!(panel.resolved_count(), 1);
        assert!(!panel.show_resolved());
    });
    f.editor.read_with(cx, |editor, _| {
        let thread = editor.document().comment(first).unwrap();
        assert!(thread.resolved);
        // The anchor stays in the text.
        assert_eq!(
            editor.document().comment_ranges(first),
            vec![TextRange::in_block(0, 4, 9)]
        );
    });

    // A thread that is not listed is not tinted in the text either.
    let hidden = |f: &Fixture, cx: &mut VisualTestContext| {
        f.editor
            .read_with(cx, |editor, _| editor.hidden_comments().collect::<Vec<_>>())
    };
    assert_eq!(hidden(&f, cx), vec![first]);

    // Show resolved, go to it, reopen.
    cx.simulate_keystrokes("s");
    f.panel
        .read_with(cx, |panel, _| assert!(panel.show_resolved()));
    assert_eq!(f.listed_ids(cx), vec![first, second]);
    // Listed again, but done with: its text stays plain until the thread
    // is the active one.
    assert_eq!(hidden(&f, cx), vec![first]);
    cx.simulate_keystrokes("s");
    assert_eq!(hidden(&f, cx), vec![first]);
    cx.simulate_keystrokes("s");
    cx.simulate_keystrokes("up");
    assert_eq!(f.active(cx), Some(first));
    assert_eq!(hidden(&f, cx), vec![]);
    cx.simulate_keystrokes("down");
    assert_eq!(hidden(&f, cx), vec![first]);
    cx.simulate_keystrokes("up");
    cx.simulate_keystrokes("x");
    f.panel.read_with(cx, |panel, _| {
        assert!(!panel.thread(first).unwrap().is_resolved());
        assert_eq!(panel.resolved_count(), 0);
    });
    assert_eq!(f.active(cx), Some(first));

    // Undo the reopen, then the resolve.
    cx.simulate_keystrokes("cmd-z");
    f.panel.read_with(cx, |panel, _| {
        assert!(panel.thread(first).unwrap().is_resolved())
    });
    cx.simulate_keystrokes("cmd-z");
    f.panel.read_with(cx, |panel, _| {
        assert!(!panel.thread(first).unwrap().is_resolved());
        assert_eq!(panel.resolved_count(), 0);
    });

    // Resolve everything with resolved threads hidden: the panel stays (its
    // footer leads back to them), and with nothing left to act on the
    // keyboard is back in the text.
    cx.simulate_keystrokes("s");
    f.panel
        .read_with(cx, |panel, _| assert!(!panel.show_resolved()));
    cx.simulate_keystrokes("x");
    assert_eq!(f.focus(cx), Focus::List);
    cx.simulate_keystrokes("x");
    assert!(f.listed_ids(cx).is_empty());
    assert_eq!(f.active(cx), None);
    assert_eq!(f.focus(cx), Focus::Editor);
    f.panel.read_with(cx, |panel, _| {
        assert!(panel.is_open());
        assert_eq!(panel.resolved_count(), 2);
    });
    // What is typed now is text, not a list command.
    let before = f.editor.read_with(cx, |editor, _| editor.plain_text());
    cx.simulate_input("zz");
    let after = f.editor.read_with(cx, |editor, _| editor.plain_text());
    assert_eq!(after.len(), before.len() + 2);
    cx.simulate_keystrokes("cmd-z");

    // The empty list can still be entered: it says that it has the
    // keyboard and which keys lead on.
    cx.simulate_keystrokes("cmd-2");
    assert_eq!(f.focus(cx), Focus::List);
    assert!(cx.debug_bounds("comments-empty").is_some());
    assert!(cx.debug_bounds("comments-empty-hints").is_some());
    cx.simulate_keystrokes("s down");
    assert_eq!(f.active(cx), Some(first));
}

#[gpui::test]
fn delete_asks_first_and_can_be_undone(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    let first = f.comment(0, 4, 9, "First", cx);
    let second = f.comment(1, 2, 8, "Second", cx);
    f.caret(0, 5, cx);
    cx.simulate_keystrokes("cmd-2 cmd-backspace");
    f.panel.read_with(cx, |panel, _| {
        assert_eq!(panel.pending_delete(), Some(DeleteTarget::Thread(first)));
    });
    assert_eq!(
        f.comment_count(cx),
        2,
        "nothing is deleted before the answer"
    );

    // While the question is up, the list commands are off.
    cx.simulate_keystrokes("j x e");
    assert_eq!(f.active(cx), Some(first));
    f.panel.read_with(cx, |panel, _| {
        assert_eq!(panel.pending_delete(), Some(DeleteTarget::Thread(first)));
        assert!(!panel.is_composing());
        assert!(!panel.thread(first).unwrap().is_resolved());
    });

    // escape keeps the comment and stays in the list.
    cx.simulate_keystrokes("escape");
    f.panel
        .read_with(cx, |panel, _| assert_eq!(panel.pending_delete(), None));
    assert_eq!(f.focus(cx), Focus::List);
    assert_eq!(f.comment_count(cx), 2);

    // enter deletes: thread and anchor are gone, the neighbour is active.
    cx.simulate_keystrokes("cmd-backspace enter");
    assert_eq!(f.all_ids(cx), vec![second]);
    assert_eq!(f.active(cx), Some(second));
    assert_eq!(f.focus(cx), Focus::List);
    f.editor.read_with(cx, |editor, _| {
        assert!(editor.document().comment(first).is_none());
        assert!(editor.document().comment_ranges(first).is_empty());
        assert!(editor.document().blocks()[0].comment_ids().is_empty());
    });

    // Undo in the list restores the thread with its anchor.
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(f.all_ids(cx), vec![first, second]);
    f.editor.read_with(cx, |editor, _| {
        assert_eq!(
            editor.document().comment_ranges(first),
            vec![TextRange::in_block(0, 4, 9)]
        );
        assert_eq!(editor.document().comment(first).unwrap().body, "First");
    });

    // Deleting the last thread closes the panel and hands focus back.
    cx.simulate_keystrokes("home cmd-backspace enter");
    cx.simulate_keystrokes("cmd-backspace enter");
    assert_eq!(f.comment_count(cx), 0);
    f.panel.read_with(cx, |panel, _| assert!(!panel.is_open()));
    assert_eq!(f.focus(cx), Focus::Editor);
    assert_eq!(f.active(cx), None);
    // Undo from the editor works as well.
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(f.comment_count(cx), 1);
}

#[gpui::test]
fn replies_can_be_selected_edited_and_deleted(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    let id = f.comment(0, 4, 9, "Root", cx);
    cx.simulate_keystrokes("cmd-2 r");
    cx.simulate_input("One");
    cx.simulate_keystrokes("enter r");
    cx.simulate_input("Two");
    cx.simulate_keystrokes("enter");
    let replies = |f: &super::support::Fixture, cx: &mut gpui::VisualTestContext| {
        f.panel.read_with(cx, |panel, _| {
            panel
                .thread(id)
                .unwrap()
                .thread
                .replies
                .iter()
                .map(|reply| (reply.id, reply.body.clone()))
                .collect::<Vec<_>>()
        })
    };
    let (one, two) = (ReplyId(1), ReplyId(2));
    assert_eq!(
        replies(&f, cx),
        vec![(one, "One".to_string()), (two, "Two".to_string())]
    );
    let selected = |f: &super::support::Fixture, cx: &mut gpui::VisualTestContext| {
        f.panel.read_with(cx, |panel, _| panel.selected_reply())
    };

    // right / l step into the replies, left / h back out.
    assert_eq!(selected(&f, cx), None);
    cx.simulate_keystrokes("right");
    assert_eq!(selected(&f, cx), Some(one));
    cx.simulate_keystrokes("l");
    assert_eq!(selected(&f, cx), Some(two));
    cx.simulate_keystrokes("l");
    assert_eq!(selected(&f, cx), Some(two));
    cx.simulate_keystrokes("h");
    assert_eq!(selected(&f, cx), Some(one));

    // e edits the selected reply.
    cx.simulate_keystrokes("e");
    f.panel.read_with(cx, |panel, _| {
        assert_eq!(
            panel.draft(),
            Some(&DraftKind::EditReply {
                thread: id,
                reply: one
            })
        );
    });
    assert_eq!(f.composer_text(cx), "One");
    cx.simulate_input(" more");
    cx.simulate_keystrokes("enter");
    assert_eq!(replies(&f, cx)[0].1, "One more");
    assert_eq!(f.focus(cx), Focus::List);
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(replies(&f, cx)[0].1, "One");

    // cmd-backspace on a selected reply deletes only that reply.
    assert_eq!(selected(&f, cx), Some(one));
    cx.simulate_keystrokes("cmd-backspace");
    f.panel.read_with(cx, |panel, _| {
        assert_eq!(panel.pending_delete(), Some(DeleteTarget::Reply(id, one)));
    });
    cx.simulate_keystrokes("enter");
    assert_eq!(replies(&f, cx), vec![(two, "Two".to_string())]);
    assert_eq!(selected(&f, cx), None);
    assert_eq!(f.comment_count(cx), 1);
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(replies(&f, cx).len(), 2);

    // left from the first reply is the comment itself again.
    cx.simulate_keystrokes("right left");
    assert_eq!(selected(&f, cx), None);
}

#[gpui::test]
fn deleted_text_detaches_the_thread(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    let kept = f.comment(1, 2, 8, "Stays anchored", cx);
    let lost = f.comment(0, 4, 9, "Loses its text", cx);
    assert_eq!(f.listed_ids(cx), vec![lost, kept]);

    // Delete "quick " in the text.
    f.editor.update(cx, |editor, cx| {
        editor.edit(
            |state| {
                state.select_range(TextRange::in_block(0, 4, 10));
                state.delete_selection()
            },
            cx,
        );
    });
    cx.run_until_parked();
    assert_eq!(
        f.listed_ids(cx),
        vec![kept, lost],
        "detached threads go last"
    );
    f.panel.read_with(cx, |panel, _| {
        let entry = panel.thread(lost).unwrap();
        assert!(entry.is_detached());
        assert_eq!(entry.quote, "");
        assert_eq!(entry.thread.body, "Loses its text");
        assert!(!panel.thread(kept).unwrap().is_detached());
    });
    assert!(cx.debug_bounds("comments-detached").is_some());

    // A detached thread can still be reached, answered and resolved.
    cx.simulate_keystrokes("cmd-2 end");
    assert_eq!(f.active(cx), Some(lost));
    cx.simulate_keystrokes("r");
    cx.simulate_input("Where did it go");
    cx.simulate_keystrokes("enter x");
    f.panel.read_with(cx, |panel, _| {
        let entry = panel.thread(lost).unwrap();
        assert_eq!(entry.thread.replies.len(), 1);
        assert!(entry.is_resolved());
    });
    assert_eq!(f.listed_ids(cx), vec![kept]);

    // Undo: resolve, reply, then the deletion of the text re-anchors it.
    cx.simulate_keystrokes("cmd-z cmd-z cmd-z");
    f.panel.read_with(cx, |panel, _| {
        let entry = panel.thread(lost).unwrap();
        assert!(!entry.is_detached());
        assert!(!entry.is_resolved());
        assert!(entry.thread.replies.is_empty());
        assert_eq!(entry.quote, "quick");
    });
    assert_eq!(f.listed_ids(cx), vec![lost, kept]);
}

#[gpui::test]
fn the_palette_can_run_thread_commands_from_the_editor(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    let id = f.comment(0, 4, 9, "Root", cx);
    // Focus is in the editor, the caret in the commented word: the thread
    // is active, and the commands act on it.
    assert_eq!(f.focus(cx), Focus::Editor);
    assert_eq!(f.active(cx), Some(id));

    cx.dispatch_action(innerword_comments::ReplyToThread);
    assert_eq!(f.focus(cx), Focus::Composer);
    cx.simulate_input("From the palette");
    cx.simulate_keystrokes("enter");
    // The draft was started from the editor, so focus returns there.
    assert_eq!(f.focus(cx), Focus::Editor);
    f.panel.read_with(cx, |panel, _| {
        assert_eq!(panel.thread(id).unwrap().thread.replies.len(), 1);
    });

    // Delete: the question needs the keyboard, so the list takes focus;
    // either answer returns to the editor.
    cx.dispatch_action(innerword_comments::DeleteComment);
    assert_eq!(f.focus(cx), Focus::List);
    cx.simulate_keystrokes("escape");
    assert_eq!(f.focus(cx), Focus::Editor);
    assert_eq!(f.comment_count(cx), 1);
    cx.dispatch_action(innerword_comments::DeleteComment);
    cx.simulate_keystrokes("enter");
    assert_eq!(f.focus(cx), Focus::Editor);
    assert_eq!(f.comment_count(cx), 0);

    cx.simulate_keystrokes("cmd-z");
    assert_eq!(f.comment_count(cx), 1);
    cx.dispatch_action(innerword_comments::ToggleResolved);
    f.panel.read_with(cx, |panel, _| {
        assert!(panel.thread(id).unwrap().is_resolved());
    });
    assert_eq!(f.focus(cx), Focus::Editor);
}

#[gpui::test]
fn a_read_only_editor_keeps_its_comments_as_they_are(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    let id = f.comment(0, 4, 9, "Root", cx);
    f.editor
        .update(cx, |editor, cx| editor.set_read_only(true, cx));
    cx.simulate_keystrokes("cmd-2");
    assert_eq!(f.focus(cx), Focus::List);
    cx.simulate_keystrokes("x r e cmd-backspace cmd-z");
    assert_eq!(f.focus(cx), Focus::List);
    f.panel.read_with(cx, |panel, _| {
        let entry = panel.thread(id).unwrap();
        assert!(!entry.is_resolved());
        assert!(entry.thread.replies.is_empty());
        assert!(!panel.is_composing());
        assert_eq!(panel.pending_delete(), None);
    });
    assert_eq!(f.comment_count(cx), 1);
}

#[gpui::test]
fn clicking_a_thread_makes_it_active(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    let first = f.comment(0, 4, 9, "First", cx);
    let second = f.comment(1, 2, 8, "Second", cx);
    assert_eq!(f.active(cx), Some(second));

    let panel_bounds = cx
        .debug_bounds("comments-panel")
        .expect("panel is rendered");
    let card = cx
        .debug_bounds(Box::leak(
            format!("comment-thread-{}", first.0).into_boxed_str(),
        ))
        .expect("thread card is rendered");
    assert!(panel_bounds.contains(&card.center()));

    cx.simulate_click(card.center(), Modifiers::none());
    assert_eq!(f.active(cx), Some(first));
    assert_eq!(f.focus(cx), Focus::List);
    // The caret did not move.
    assert_eq!(f.selection(cx).head.block, 1);
}

#[gpui::test]
fn the_list_stays_usable_beside_a_draft_that_was_left_open(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    let first = f.comment(0, 4, 9, "First", cx);
    let second = f.comment(1, 2, 8, "Second", cx);
    f.caret(0, 5, cx);
    cx.simulate_keystrokes("cmd-2 r");
    cx.simulate_input("half");
    assert_eq!(f.focus(cx), Focus::Composer);

    // A click on the other thread: it becomes active and the list has
    // focus. The draft stays where it was, with its text.
    let card = cx
        .debug_bounds(Box::leak(
            format!("comment-thread-{}", second.0).into_boxed_str(),
        ))
        .unwrap();
    cx.simulate_click(
        gpui::point(card.center().x, card.top() + gpui::px(6.)),
        Modifiers::none(),
    );
    assert_eq!(f.focus(cx), Focus::List);
    assert_eq!(f.active(cx), Some(second));
    assert_eq!(f.composer_text(cx), "half");
    f.panel.read_with(cx, |panel, _| {
        assert_eq!(panel.draft(), Some(&DraftKind::Reply { thread: first }));
    });

    // The list keys work again; `r` on the draft's thread goes back to it.
    cx.simulate_keystrokes("k");
    assert_eq!(f.active(cx), Some(first));
    cx.simulate_keystrokes("r");
    assert_eq!(f.focus(cx), Focus::Composer);
    assert_eq!(f.composer_text(cx), "half");
    cx.simulate_input(" done");
    cx.simulate_keystrokes("enter");
    f.panel.read_with(cx, |panel, _| {
        assert_eq!(
            panel.thread(first).unwrap().thread.replies[0].body,
            "half done"
        );
    });

    // A click into the composer keeps focus in the composer.
    cx.simulate_keystrokes("r");
    let composer = cx.debug_bounds("comment-composer").unwrap();
    cx.simulate_keystrokes("escape cmd-2 r");
    cx.simulate_click(composer.center(), Modifiers::none());
    assert_eq!(f.focus(cx), Focus::Composer);
}

/// A document long enough to scroll: `count` paragraphs.
fn long_document(count: usize) -> Document {
    Document::from_blocks(
        (0..count)
            .map(|ix| Block::paragraph(&format!("Paragraph number {ix} of a long text.")))
            .collect(),
    )
}

#[gpui::test]
fn undo_from_the_list_shows_what_it_undid(cx: &mut TestAppContext) {
    let (f, cx) = open(long_document(300), cx);
    let scroll =
        |cx: &mut VisualTestContext| f.editor.read_with(cx, |editor, _| editor.scroll_offset());
    let first = f.comment(0, 0, 9, "At the top", cx);

    // A text edit far down, then back to the top and into the list.
    f.editor.update(cx, |editor, cx| {
        editor.edit(
            |state| {
                state.set_caret(Position::new(280, 0));
                state.insert_text("Typed far away. ")
            },
            cx,
        );
    });
    f.caret(0, 2, cx);
    cx.simulate_keystrokes("cmd-2");
    assert_eq!(f.focus(cx), Focus::List);
    assert!(scroll(cx) < gpui::px(100.), "at the top: {:?}", scroll(cx));

    // Undo takes the text edit back, and the text scrolls to it.
    cx.simulate_keystrokes("cmd-z");
    f.editor.read_with(cx, |editor, _| {
        assert_eq!(
            editor.document().block(280).unwrap().text(),
            "Paragraph number 280 of a long text."
        );
    });
    assert!(
        scroll(cx) > gpui::px(1000.),
        "the undone edit is brought into view: {:?}",
        scroll(cx)
    );
    assert_eq!(f.focus(cx), Focus::List);
    assert_eq!(f.comment_count(cx), 1);

    // Redo shows it again from wherever the text is.
    f.editor.update(cx, |editor, cx| {
        editor.scroll_to_position(Position::new(0, 0), innerword_editor::Autoscroll::Top, cx)
    });
    cx.run_until_parked();
    assert!(scroll(cx) < gpui::px(100.));
    cx.simulate_keystrokes("cmd-shift-z");
    assert!(scroll(cx) > gpui::px(1000.));
    cx.simulate_keystrokes("cmd-z");

    // The next undo step is the comment: its place in the text is shown.
    cx.simulate_keystrokes("escape");
    let far = f.comment(250, 0, 9, "Far down", cx);
    f.editor.update(cx, |editor, cx| {
        editor.scroll_to_position(Position::new(0, 0), innerword_editor::Autoscroll::Top, cx)
    });
    f.caret(0, 2, cx);
    cx.simulate_keystrokes("cmd-2");
    assert_eq!(f.active(cx), Some(first));
    assert!(scroll(cx) < gpui::px(100.));
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(f.all_ids(cx), vec![first]);
    assert!(scroll(cx) > gpui::px(1000.), "where the comment was");
    // Redo: the thread is back, active, and the text is at its anchor.
    f.editor.update(cx, |editor, cx| {
        editor.scroll_to_position(Position::new(0, 0), innerword_editor::Autoscroll::Top, cx)
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("cmd-shift-z");
    assert_eq!(f.all_ids(cx), vec![first, far]);
    assert_eq!(f.active(cx), Some(far));
    assert!(scroll(cx) > gpui::px(1000.));

    // A change of a thread only (a reply) makes that thread the active one.
    cx.simulate_keystrokes("r");
    cx.simulate_input("An answer");
    cx.simulate_keystrokes("enter home");
    assert_eq!(f.active(cx), Some(first));
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(f.active(cx), Some(far), "the thread that lost its reply");
    f.panel.read_with(cx, |panel, _| {
        assert!(panel.thread(far).unwrap().thread.replies.is_empty());
    });
    // Nothing left to redo twice: no harm.
    cx.simulate_keystrokes("cmd-shift-z cmd-shift-z cmd-shift-z");
    assert_eq!(f.focus(cx), Focus::List);
}

#[gpui::test]
fn commented_text_partly_deleted_fully_deleted_and_restored(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    // "quick brown fox"
    let id = f.comment(0, 4, 19, "All three words", cx);
    let delete = |start: usize, end: usize, cx: &mut VisualTestContext| {
        f.editor.update(cx, |editor, cx| {
            editor.edit(
                |state| {
                    state.select_range(TextRange::in_block(0, start, end));
                    state.delete_selection()
                },
                cx,
            );
        });
        cx.run_until_parked();
    };
    let entry = |cx: &mut VisualTestContext| {
        f.panel
            .read_with(cx, |panel, _| panel.thread(id).unwrap().clone())
    };

    // The middle word goes: the thread stays on what is left.
    delete(10, 16, cx);
    assert_eq!(entry(cx).quote, "quick fox");
    assert_eq!(entry(cx).ranges, vec![TextRange::in_block(0, 4, 13)]);
    // The front of it and some text before it.
    delete(0, 10, cx);
    assert_eq!(entry(cx).quote, "fox");
    // The rest: detached, listed under its own heading, still active.
    delete(0, 3, cx);
    assert!(entry(cx).is_detached());
    assert_eq!(entry(cx).quote, "");
    assert!(cx.debug_bounds("comments-detached").is_some());
    assert_eq!(f.listed_ids(cx), vec![id]);
    // Next and previous comment have nowhere to go, and say so quietly.
    cx.simulate_keystrokes("cmd-alt-] cmd-alt-[");
    assert_eq!(f.focus(cx), Focus::Editor);

    // Undo from the text, step by step: anchored again, then whole again.
    cx.simulate_keystrokes("cmd-z");
    assert!(!entry(cx).is_detached());
    assert_eq!(entry(cx).quote, "fox");
    cx.simulate_keystrokes("cmd-z cmd-z");
    assert_eq!(entry(cx).quote, "quick brown fox");
    assert_eq!(entry(cx).ranges, vec![TextRange::in_block(0, 4, 19)]);
    // And redo detaches it again.
    cx.simulate_keystrokes("cmd-shift-z cmd-shift-z cmd-shift-z");
    assert!(entry(cx).is_detached());
    // Undo from the list makes the re-anchored thread the active one.
    cx.simulate_keystrokes("cmd-2");
    assert_eq!(f.focus(cx), Focus::List);
    cx.simulate_keystrokes("cmd-z");
    assert!(!entry(cx).is_detached());
    assert_eq!(f.active(cx), Some(id));
    assert_eq!(f.focus(cx), Focus::List);
}
