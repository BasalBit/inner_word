//! Showing and hiding comments, and following the editor to another
//! document.

use gpui::{AppContext, TestAppContext};
use innerword_comments::CommentsEvent;
use innerword_core::{Block, Document, EditorState, TextRange};
use innerword_editor::{Editor, EditorConfig};

use super::support::{Focus, NOW, open, open_window, sample, setup};

/// A document that already has one comment thread, on "other".
fn other_document() -> Document {
    let mut state = EditorState::new(Document::from_blocks(vec![Block::paragraph(
        "Some other text entirely.",
    )]));
    state.select_range(TextRange::in_block(0, 5, 10));
    state
        .add_comment("From before", "Grace", NOW - 3_600_000)
        .unwrap();
    state.into_document()
}

#[gpui::test]
fn toggle_hides_the_panel_and_the_tints(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    let id = f.comment(0, 4, 9, "Tighten this", cx);
    f.editor
        .read_with(cx, |editor, _| assert!(editor.config().comments_visible));
    f.panel.read_with(cx, |panel, _| {
        assert!(panel.is_visible());
        assert!(panel.is_open());
    });
    assert!(cx.debug_bounds("comments-panel").is_some());
    assert_eq!(f.open_events(cx), vec![true]);

    cx.simulate_keystrokes("cmd-alt-c");
    f.panel.read_with(cx, |panel, _| {
        assert!(!panel.is_visible());
        assert!(!panel.is_open());
        // The threads are still known, only not shown.
        assert_eq!(panel.threads().len(), 1);
    });
    f.editor
        .read_with(cx, |editor, _| assert!(!editor.config().comments_visible));
    assert_eq!(f.active(cx), None);
    assert_eq!(f.visibility_events(cx), vec![false]);
    assert_eq!(f.open_events(cx), vec![true, false]);
    // Hidden comments mark nothing when the caret moves through them.
    f.caret(0, 6, cx);
    assert_eq!(f.active(cx), None);

    cx.simulate_keystrokes("cmd-alt-c");
    f.panel.read_with(cx, |panel, _| {
        assert!(panel.is_visible());
        assert!(panel.is_open());
    });
    f.editor
        .read_with(cx, |editor, _| assert!(editor.config().comments_visible));
    assert_eq!(f.visibility_events(cx), vec![false, true]);
    assert_eq!(f.open_events(cx), vec![true, false, true]);
    assert_eq!(f.focus(cx), Focus::Editor);
    f.caret(0, 5, cx);
    assert_eq!(f.active(cx), Some(id));

    // Setting the same value again reports nothing.
    let events = f.events(cx).len();
    f.panel
        .update_in(cx, |panel, window, cx| panel.set_visible(true, window, cx));
    assert_eq!(f.events(cx).len(), events);
}

#[gpui::test]
fn hiding_from_inside_the_panel_returns_focus_to_the_editor(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    f.comment(0, 4, 9, "Tighten this", cx);
    cx.simulate_keystrokes("cmd-2");
    assert_eq!(f.focus(cx), Focus::List);
    cx.simulate_keystrokes("cmd-alt-c");
    assert_eq!(f.focus(cx), Focus::Editor);

    // From an open reply composer: the composer closes with the panel and
    // nothing is saved, but what was written is kept.
    cx.simulate_keystrokes("cmd-2 r");
    assert_eq!(f.focus(cx), Focus::Composer);
    cx.simulate_input("half a thought");
    cx.simulate_keystrokes("cmd-alt-c");
    assert_eq!(f.focus(cx), Focus::Editor);
    f.panel.read_with(cx, |panel, _| {
        assert!(!panel.is_visible());
        assert!(!panel.is_composing());
        assert!(panel.threads()[0].thread.replies.is_empty());
        assert_eq!(panel.kept_drafts(), 1);
    });
    assert_eq!(f.composer_text(cx), "");

    // Focusing the panel, or going to a comment, shows comments again,
    // and the reply is back in its composer.
    cx.simulate_keystrokes("cmd-2");
    assert_eq!(f.focus(cx), Focus::Composer);
    assert_eq!(f.composer_text(cx), "half a thought");
    f.panel.read_with(cx, |panel, _| {
        assert!(panel.is_visible());
        assert_eq!(panel.kept_drafts(), 0);
    });
    // An empty composer is simply closed.
    cx.simulate_keystrokes("escape r cmd-alt-c");
    f.panel
        .read_with(cx, |panel, _| assert_eq!(panel.kept_drafts(), 0));
    cx.simulate_keystrokes("cmd-2");
    assert_eq!(f.focus(cx), Focus::List);
    f.panel
        .read_with(cx, |panel, _| assert!(panel.is_visible()));
    cx.simulate_keystrokes("cmd-alt-c");
    f.caret(0, 0, cx);
    cx.simulate_keystrokes("cmd-alt-]");
    f.panel
        .read_with(cx, |panel, _| assert!(panel.is_visible()));
    f.editor
        .read_with(cx, |editor, _| assert!(editor.config().comments_visible));
}

#[gpui::test]
fn show_resolved_is_reported(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    f.comment(0, 4, 9, "Tighten this", cx);
    cx.simulate_keystrokes("cmd-2 x");
    // The last open thread is resolved: the keyboard is back in the text.
    cx.simulate_keystrokes("cmd-2 s");
    assert!(
        f.events(cx)
            .contains(&CommentsEvent::ShowResolvedChanged(true))
    );
    f.panel
        .read_with(cx, |panel, _| assert!(panel.show_resolved()));
    // The footer row does the same with the mouse.
    let footer = cx
        .debug_bounds("comments-resolved-toggle")
        .expect("footer is rendered");
    cx.simulate_click(footer.center(), gpui::Modifiers::none());
    f.panel
        .read_with(cx, |panel, _| assert!(!panel.show_resolved()));
    assert_eq!(
        f.events(cx).last(),
        Some(&CommentsEvent::ShowResolvedChanged(false))
    );
}

#[gpui::test]
fn the_panel_follows_a_replaced_document(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    let old = f.comment(0, 4, 9, "Tighten this", cx);
    cx.simulate_keystrokes("cmd-2 r");
    cx.simulate_input("unfinished");
    assert_eq!(f.focus(cx), Focus::Composer);

    // The app opens another document in the same editor.
    f.editor
        .update(cx, |editor, cx| editor.set_document(other_document(), cx));
    cx.run_until_parked();
    f.panel.read_with(cx, |panel, _| {
        assert!(
            !panel.is_composing(),
            "the draft belonged to the old document"
        );
        assert_eq!(panel.threads().len(), 1);
        let entry = &panel.threads()[0];
        assert_eq!(entry.quote, "other");
        assert_eq!(entry.thread.body, "From before");
        assert_eq!(entry.thread.author, "Grace");
        assert!(panel.is_open());
    });
    assert_eq!(f.active(cx), None);
    assert_eq!(f.composer_text(cx), "");
    assert_eq!(f.focus(cx), Focus::Editor);
    let _ = old;

    // A document without comments closes the panel.
    cx.simulate_keystrokes("cmd-2");
    assert_eq!(f.focus(cx), Focus::List);
    f.editor
        .update(cx, |editor, cx| editor.set_document(sample(), cx));
    cx.run_until_parked();
    f.panel.read_with(cx, |panel, _| {
        assert!(panel.threads().is_empty());
        assert!(!panel.is_open());
    });
    assert_eq!(f.focus(cx), Focus::Editor);
}

#[gpui::test]
fn the_panel_can_switch_to_another_editor(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    let old = f.comment(0, 4, 9, "Tighten this", cx);
    assert_eq!(f.active(cx), Some(old));
    cx.simulate_keystrokes("cmd-alt-c");

    let second = cx.update(|window, cx| {
        cx.new(|cx| Editor::new(other_document(), EditorConfig::full(), window, cx))
    });
    f.workspace.update_in(cx, |workspace, window, cx| {
        workspace.editor = second.clone();
        workspace
            .panel
            .update(cx, |panel, cx| panel.set_editor(second.clone(), window, cx));
        second.read(cx).focus(window);
        cx.notify();
    });
    cx.run_until_parked();

    // The new editor took over the visibility flag (hidden), the old one
    // has no active thread left.
    second.read_with(cx, |editor, _| assert!(!editor.config().comments_visible));
    f.editor
        .read_with(cx, |editor, _| assert_eq!(editor.active_comment(), None));
    f.panel.read_with(cx, |panel, _| {
        assert_eq!(panel.editor(), &second);
        assert_eq!(panel.threads().len(), 1);
        assert_eq!(panel.threads()[0].thread.body, "From before");
    });

    // The panel now works on the new editor's document...
    cx.simulate_keystrokes("cmd-alt-c cmd-2 r");
    cx.simulate_input("Still true");
    cx.simulate_keystrokes("enter");
    second.read_with(cx, |editor, _| {
        let thread = editor.document().comments().next().unwrap();
        assert_eq!(thread.replies.len(), 1);
        assert_eq!(thread.replies[0].body, "Still true");
        assert!(editor.config().comments_visible);
    });
    // ...and no longer listens to the old one.
    f.editor.update(cx, |editor, cx| {
        editor.edit(|state| state.delete_comment(old), cx);
    });
    cx.run_until_parked();
    f.panel
        .read_with(cx, |panel, _| assert_eq!(panel.threads().len(), 1));
    f.editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().comment_count(), 0);
    });
}

#[gpui::test]
fn relative_times_follow_the_clock(cx: &mut TestAppContext) {
    let (f, cx) = open(other_document(), cx);
    // "From before" was written an hour before the test clock's now. The
    // label is part of the rendered tree; here the clock and the panel's
    // refresh timer are exercised: advancing both must not disturb state.
    f.panel.read_with(cx, |panel, _| {
        let entry = &panel.threads()[0];
        assert_eq!(
            innerword_comments::time::relative_time(entry.thread.created_at, NOW),
            "1 hour ago"
        );
    });
    f.clock.set(NOW + 86_400_000);
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(61));
    cx.run_until_parked();
    f.panel.read_with(cx, |panel, _| {
        assert!(panel.is_open());
        assert_eq!(panel.threads().len(), 1);
    });
}

#[gpui::test]
fn the_editor_flag_and_the_panel_stay_in_step(cx: &mut TestAppContext) {
    // An editor that starts with comments hidden (the app restored its
    // setting before the panel existed).
    setup(cx);
    let (f, cx) = open_window(
        other_document(),
        EditorConfig::full().comments_visible(false),
        cx,
    );
    f.panel.read_with(cx, |panel, _| {
        assert!(!panel.is_visible());
        assert!(!panel.is_open());
        assert_eq!(panel.threads().len(), 1);
    });
    f.editor
        .read_with(cx, |editor, _| assert!(!editor.config().comments_visible));
    assert!(f.events(cx).is_empty());

    // The app's own settings code flips the editor flag: the panel follows
    // and reports it.
    f.editor
        .update(cx, |editor, cx| editor.set_comments_visible(true, cx));
    cx.run_until_parked();
    f.panel.read_with(cx, |panel, _| {
        assert!(panel.is_visible());
        assert!(panel.is_open());
    });
    assert_eq!(f.visibility_events(cx), vec![true]);
    assert_eq!(f.open_events(cx), vec![true]);
    assert!(cx.debug_bounds("comments-panel").is_some());

    cx.simulate_keystrokes("cmd-2");
    assert_eq!(f.focus(cx), Focus::List);
    f.editor
        .update(cx, |editor, cx| editor.set_comments_visible(false, cx));
    cx.run_until_parked();
    f.panel
        .read_with(cx, |panel, _| assert!(!panel.is_visible()));
    assert_eq!(f.focus(cx), Focus::Editor);
    assert_eq!(f.visibility_events(cx), vec![true, false]);
    assert_eq!(f.open_events(cx), vec![true, false]);
}

#[gpui::test]
fn open_follows_the_threads(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    assert!(f.open_events(cx).is_empty());

    // A composer opens the panel, cancelling closes it again.
    f.select(0, 4, 9, cx);
    cx.simulate_keystrokes("cmd-alt-m");
    assert_eq!(f.open_events(cx), vec![true]);
    cx.simulate_keystrokes("escape");
    assert_eq!(f.open_events(cx), vec![true, false]);

    // A saved comment keeps it open; undo closes it, redo opens it.
    cx.simulate_keystrokes("cmd-alt-m");
    cx.simulate_input("Stay");
    cx.simulate_keystrokes("enter");
    assert_eq!(f.open_events(cx), vec![true, false, true]);
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(f.open_events(cx), vec![true, false, true, false]);
    cx.simulate_keystrokes("cmd-shift-z");
    assert_eq!(f.open_events(cx), vec![true, false, true, false, true]);
    assert_eq!(f.visibility_events(cx), Vec::<bool>::new());
}

#[gpui::test]
fn each_window_has_its_own_panel(cx: &mut TestAppContext) {
    setup(cx);
    let (first, first_cx) = open_window(sample(), EditorConfig::full(), cx);
    let first_window = first_cx.update(|window, _| window.window_handle());
    let (second, cx) = open_window(other_document(), EditorConfig::full(), cx);

    // The second window is the active one: its panel gets the action.
    second.select(0, 0, 4, cx);
    cx.simulate_keystrokes("cmd-alt-m");
    second
        .panel
        .read_with(cx, |panel, _| assert!(panel.is_composing()));
    first
        .panel
        .read_with(cx, |panel, _| assert!(!panel.is_composing()));
    cx.simulate_input("In the second window");
    cx.simulate_keystrokes("enter");
    second.editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().comment_count(), 2);
    });
    first.editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().comment_count(), 0);
    });

    // Back in the first window.
    cx.update_window(first_window, |_, window, _| window.activate_window())
        .unwrap();
    cx.run_until_parked();
    first.editor.update(cx, |editor, cx| {
        editor.edit(|state| state.select_range(TextRange::in_block(0, 4, 9)), cx);
    });
    TestAppContext::dispatch_action(cx, first_window, innerword_comments::AddComment);
    cx.run_until_parked();
    first
        .panel
        .read_with(cx, |panel, _| assert!(panel.is_composing()));
    second
        .panel
        .read_with(cx, |panel, _| assert!(!panel.is_composing()));
}

#[gpui::test]
fn actions_reach_the_only_panel_when_no_window_is_active(cx: &mut TestAppContext) {
    // The app in the background, driven by a script: no window is active,
    // and the one panel there is still gets the window-wide actions.
    let (f, cx) = open(sample(), cx);
    f.select(0, 4, 9, cx);
    cx.deactivate_window();
    cx.run_until_parked();
    assert!(cx.update(|_, cx| cx.active_window().is_none()));

    cx.simulate_keystrokes("cmd-alt-m");
    f.panel
        .read_with(cx, |panel, _| assert!(panel.is_composing()));
    cx.simulate_input("From the background");
    cx.simulate_keystrokes("enter");
    assert_eq!(f.comment_count(cx), 1);
    cx.simulate_keystrokes("cmd-alt-c");
    f.panel
        .read_with(cx, |panel, _| assert!(!panel.is_visible()));
}

#[gpui::test]
fn a_container_that_is_always_shown_says_why_it_is_empty(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    f.workspace.update(cx, |workspace, cx| {
        workspace.keep_slot = true;
        cx.notify();
    });
    cx.run_until_parked();
    let slot = cx.debug_bounds("panel-slot").expect("the slot is shown");
    let note = cx
        .debug_bounds("comments-none")
        .expect("the empty state is painted");
    assert_eq!(note, slot);
    f.panel.read_with(cx, |panel, _| {
        assert!(panel.is_visible());
        assert!(!panel.is_open());
    });

    // With a comment the list takes its place.
    let id = f.comment(0, 4, 9, "First", cx);
    assert!(cx.debug_bounds("comment-threads").is_some());
    assert_eq!(f.active(cx), Some(id));
}
