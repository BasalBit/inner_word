//! Working on threads from the thread list: reply, edit, resolve and
//! reopen, delete, detached threads; undo of each.

use gpui::{Modifiers, TestAppContext};
use writer_comments::{DeleteTarget, DraftKind};
use writer_core::{ReplyId, TextRange};

use super::support::{AUTHOR, Focus, NOW, open, sample};

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

    // Show resolved, go to it, reopen.
    cx.simulate_keystrokes("s");
    f.panel
        .read_with(cx, |panel, _| assert!(panel.show_resolved()));
    assert_eq!(f.listed_ids(cx), vec![first, second]);
    cx.simulate_keystrokes("up");
    assert_eq!(f.active(cx), Some(first));
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
    // footer leads back to them) and the list keeps focus.
    cx.simulate_keystrokes("s");
    f.panel
        .read_with(cx, |panel, _| assert!(!panel.show_resolved()));
    cx.simulate_keystrokes("x x");
    assert!(f.listed_ids(cx).is_empty());
    assert_eq!(f.active(cx), None);
    assert_eq!(f.focus(cx), Focus::List);
    f.panel.read_with(cx, |panel, _| {
        assert!(panel.is_open());
        assert_eq!(panel.resolved_count(), 2);
    });
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

    cx.dispatch_action(writer_comments::ReplyToThread);
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
    cx.dispatch_action(writer_comments::DeleteComment);
    assert_eq!(f.focus(cx), Focus::List);
    cx.simulate_keystrokes("escape");
    assert_eq!(f.focus(cx), Focus::Editor);
    assert_eq!(f.comment_count(cx), 1);
    cx.dispatch_action(writer_comments::DeleteComment);
    cx.simulate_keystrokes("enter");
    assert_eq!(f.focus(cx), Focus::Editor);
    assert_eq!(f.comment_count(cx), 0);

    cx.simulate_keystrokes("cmd-z");
    assert_eq!(f.comment_count(cx), 1);
    cx.dispatch_action(writer_comments::ToggleResolved);
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
