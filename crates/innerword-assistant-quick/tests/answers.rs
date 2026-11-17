//! The actions of an answer in the popover: Replace Selection, Insert
//! Below, Copy, with the mouse and from the keyboard.

use gpui::TestAppContext;
use innerword_assistant_quick::{QuickAction, QuickChatEvent, ReplaceState};
use innerword_core::{BlockKind, Position, TextRange};

use crate::support::*;

/// Opens the chat on "quick" of "The quick brown fox" and gets `answer`.
fn asked<'a>(
    answer: &str,
    cx: &'a mut TestAppContext,
) -> (Fixture, &'a mut gpui::VisualTestContext) {
    let (fx, cx) = open_quick(paragraphs(&["The quick brown fox", "Next."]), cx);
    select(&fx.editor, TextRange::in_block(0, 4, 9), cx);
    fx.toggle(cx);
    fx.fake.reply(answer);
    fx.send("Another word?", cx);
    (fx, cx)
}

fn action_button(action: QuickAction, fx: &Fixture, cx: &mut gpui::VisualTestContext) -> String {
    format!(
        "assistant-action-{}-{}",
        action.id(),
        fx.last_answer_id(cx).0
    )
}

fn replace_state(fx: &Fixture, cx: &mut gpui::VisualTestContext) -> Option<ReplaceState> {
    let quick = fx.quick.clone();
    cx.update(|_, cx| quick.read(cx).replace_state(cx))
}

fn action_enabled(action: QuickAction, fx: &Fixture, cx: &mut gpui::VisualTestContext) -> bool {
    fx.quick.read_with(cx, |quick, cx| {
        quick
            .messages()
            .expect("open")
            .read(cx)
            .actions()
            .iter()
            .find(|candidate| candidate.id.as_ref() == action.id())
            .expect("the action")
            .enabled
    })
}

#[gpui::test]
fn an_answer_has_its_three_buttons(cx: &mut TestAppContext) {
    let (fx, cx) = asked("swift", cx);
    let labels: Vec<String> = fx.quick.read_with(cx, |quick, cx| {
        quick
            .messages()
            .unwrap()
            .read(cx)
            .actions()
            .iter()
            .map(|action| action.label.to_string())
            .collect()
    });
    assert_eq!(labels, vec!["Replace Selection", "Insert Below", "Copy"]);
    let popover = fx.popover(cx);
    for action in QuickAction::ALL {
        let button = bounds(action_button(action, &fx, cx), cx);
        assert!(
            popover.contains(&button.center()),
            "{action:?} is in the popover"
        );
        assert!(action_enabled(action, &fx, cx));
    }
}

#[gpui::test]
fn replace_selection_with_the_mouse(cx: &mut TestAppContext) {
    let (fx, cx) = asked("**swift**", cx);
    let depth = undo_depth(&fx.editor, cx);

    click(action_button(QuickAction::Replace, &fx, cx), cx);

    assert_eq!(text(&fx.editor, cx), "The swift brown fox\nNext.");
    assert!(attrs(&fx.editor, 0, 5, cx).bold, "formatted, not Markdown");
    // The popover is gone, the changed text is selected, the text has the
    // keyboard.
    assert!(!fx.is_open(cx));
    assert_eq!(selected_text(&fx.editor, cx), "swift");
    assert!(fx.editor_focused(cx));
    assert_eq!(fx.fake.alive(), 0, "the conversation ended");
    let events = fx.events(cx);
    assert!(events.contains(&QuickChatEvent::Closed));
    assert_eq!(
        events.last(),
        Some(&QuickChatEvent::Applied {
            action: QuickAction::Replace,
            range: TextRange::in_block(0, 4, 9),
        })
    );

    // One undo step, taken back with the editor's own key.
    assert_eq!(undo_depth(&fx.editor, cx), depth + 1);
    cx.simulate_keystrokes("cmd-z");
    cx.run_until_parked();
    assert_eq!(text(&fx.editor, cx), "The quick brown fox\nNext.");
}

#[gpui::test]
fn replace_selection_from_the_keyboard(cx: &mut TestAppContext) {
    let (fx, cx) = asked("swift", cx);
    assert!(fx.composer_focused(cx));

    cx.simulate_keystrokes("cmd-shift-enter");
    cx.run_until_parked();

    assert_eq!(text(&fx.editor, cx), "The swift brown fox\nNext.");
    assert!(!fx.is_open(cx));
    assert_eq!(selected_text(&fx.editor, cx), "swift");
    assert!(fx.editor_focused(cx));
    // Typing replaces the new text, as a selection is replaced.
    cx.simulate_input("fleet");
    cx.run_until_parked();
    assert_eq!(text(&fx.editor, cx), "The fleet brown fox\nNext.");
}

#[gpui::test]
fn replace_uses_the_captured_range_after_the_caret_moved(cx: &mut TestAppContext) {
    let (fx, cx) = asked("swift", cx);
    // The selection is somewhere else by now (a command from outside).
    select(&fx.editor, TextRange::in_block(1, 0, 4), cx);
    assert_eq!(replace_state(&fx, cx), Some(ReplaceState::Ready));

    click(action_button(QuickAction::Replace, &fx, cx), cx);

    assert_eq!(text(&fx.editor, cx), "The swift brown fox\nNext.");
    assert_eq!(selected_text(&fx.editor, cx), "swift");
}

#[gpui::test]
fn insert_below_with_the_mouse(cx: &mut TestAppContext) {
    let (fx, cx) = asked("A *swift* fox.\n\n- one\n- two", cx);
    let depth = undo_depth(&fx.editor, cx);

    click(action_button(QuickAction::InsertBelow, &fx, cx), cx);

    assert_eq!(
        text(&fx.editor, cx),
        "The quick brown fox\nA swift fox.\none\ntwo\nNext."
    );
    assert_eq!(
        kinds(&fx.editor, cx),
        vec![
            BlockKind::Paragraph,
            BlockKind::Paragraph,
            BlockKind::BulletItem,
            BlockKind::BulletItem,
            BlockKind::Paragraph,
        ]
    );
    assert!(!fx.is_open(cx));
    assert_eq!(selected_text(&fx.editor, cx), "A swift fox.\none\ntwo");
    assert!(fx.editor_focused(cx));
    assert_eq!(
        fx.events(cx).last(),
        Some(&QuickChatEvent::Applied {
            action: QuickAction::InsertBelow,
            range: TextRange::new(Position::new(1, 0), Position::new(3, 3)),
        })
    );

    assert_eq!(undo_depth(&fx.editor, cx), depth + 1);
    undo(&fx.editor, cx);
    assert_eq!(text(&fx.editor, cx), "The quick brown fox\nNext.");
}

#[gpui::test]
fn insert_below_from_the_keyboard_without_a_selection(cx: &mut TestAppContext) {
    let (fx, cx) = open_quick(paragraphs(&["The quick brown fox", "Next."]), cx);
    caret(&fx.editor, 0, 4, cx);
    fx.toggle(cx);
    fx.fake.reply("A new line.");
    fx.send("Write a line.", cx);

    // Nothing was selected: Replace is not offered, and its key does
    // nothing.
    assert_eq!(replace_state(&fx, cx), Some(ReplaceState::NothingSelected));
    assert!(!action_enabled(QuickAction::Replace, &fx, cx));
    cx.simulate_keystrokes("cmd-shift-enter");
    cx.run_until_parked();
    assert!(fx.is_open(cx));
    assert_eq!(text(&fx.editor, cx), "The quick brown fox\nNext.");

    cx.simulate_keystrokes("cmd-enter");
    cx.run_until_parked();
    assert_eq!(
        text(&fx.editor, cx),
        "The quick brown fox\nA new line.\nNext."
    );
    assert!(!fx.is_open(cx));
    assert_eq!(selected_text(&fx.editor, cx), "A new line.");
    assert!(fx.editor_focused(cx));
}

#[gpui::test]
fn copy_leaves_the_popover_open(cx: &mut TestAppContext) {
    let (fx, cx) = asked("A **swift** fox", cx);

    click(action_button(QuickAction::Copy, &fx, cx), cx);

    let item = cx.read_from_clipboard().expect("something was copied");
    assert_eq!(item.text().as_deref(), Some("A **swift** fox"));
    assert!(fx.is_open(cx));
    assert!(fx.composer_focused(cx));
    assert_eq!(text(&fx.editor, cx), "The quick brown fox\nNext.");
    assert_eq!(fx.events(cx).last(), Some(&QuickChatEvent::Copied));

    // Pasted into the document it is formatted text.
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    caret(&fx.editor, 1, 5, cx);
    cx.simulate_keystrokes("cmd-v");
    cx.run_until_parked();
    assert_eq!(
        text(&fx.editor, cx),
        "The quick brown fox\nNext.A swift fox"
    );
    assert!(attrs(&fx.editor, 1, 8, cx).bold);

    // The key copies too.
    select(&fx.editor, TextRange::in_block(0, 4, 9), cx);
    fx.toggle(cx);
    fx.fake.reply("fleet");
    fx.send("Another?", cx);
    cx.simulate_keystrokes("cmd-shift-c");
    cx.run_until_parked();
    assert_eq!(
        cx.read_from_clipboard()
            .and_then(|item| item.text())
            .as_deref(),
        Some("fleet")
    );
    assert!(fx.is_open(cx));
}

#[gpui::test]
fn replace_is_not_offered_once_the_text_changed(cx: &mut TestAppContext) {
    let (fx, cx) = asked("swift", cx);
    assert_eq!(replace_state(&fx, cx), Some(ReplaceState::Ready));

    // The selected word changes under the open popover.
    fx.editor.update(cx, |editor, cx| {
        editor.edit(
            |state| {
                state.set_caret(Position::new(0, 6));
                state.insert_text("x")
            },
            cx,
        )
    });
    cx.run_until_parked();
    assert!(fx.is_open(cx));
    assert_eq!(replace_state(&fx, cx), Some(ReplaceState::TextChanged));
    assert!(!action_enabled(QuickAction::Replace, &fx, cx));
    assert!(action_enabled(QuickAction::InsertBelow, &fx, cx));

    // Neither the button nor the key replaces anything.
    click(action_button(QuickAction::Replace, &fx, cx), cx);
    cx.simulate_keystrokes("cmd-shift-enter");
    cx.run_until_parked();
    assert!(fx.is_open(cx));
    assert_eq!(text(&fx.editor, cx), "The quxick brown fox\nNext.");

    // The text as it was again: Replace is back.
    undo(&fx.editor, cx);
    assert_eq!(replace_state(&fx, cx), Some(ReplaceState::Ready));
    assert!(action_enabled(QuickAction::Replace, &fx, cx));
    click(action_button(QuickAction::Replace, &fx, cx), cx);
    assert_eq!(text(&fx.editor, cx), "The swift brown fox\nNext.");
}

#[gpui::test]
fn an_answer_that_is_being_written_cannot_be_applied(cx: &mut TestAppContext) {
    let (fx, cx) = open_quick(paragraphs(&["The quick brown fox", "Next."]), cx);
    select(&fx.editor, TextRange::in_block(0, 4, 9), cx);
    fx.toggle(cx);
    fx.fake.hold();
    fx.fake
        .push(innerword_assistant::FakeReply::pieces(["swi", "ft"]));
    fx.send("Another word?", cx);
    fx.first_piece(cx);

    cx.simulate_keystrokes("cmd-shift-enter cmd-enter");
    cx.run_until_parked();
    assert_eq!(text(&fx.editor, cx), "The quick brown fox\nNext.");
    assert!(fx.is_open(cx));

    fx.fake.release();
    cx.run_until_parked();
    cx.simulate_keystrokes("cmd-shift-enter");
    cx.run_until_parked();
    assert_eq!(text(&fx.editor, cx), "The swift brown fox\nNext.");
}

#[gpui::test]
fn the_keys_act_on_the_marked_answer_in_the_conversation(cx: &mut TestAppContext) {
    let (fx, cx) = asked("swift", cx);
    fx.fake.reply("fleet");
    fx.send("Another one?", cx);
    assert_eq!(fx.answers(cx), vec!["swift", "fleet"]);

    // tab moves to the conversation (the last message is marked), up
    // marks the messages before it.
    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    assert!(!fx.composer_focused(cx));
    assert!(fx.has_keyboard(cx));
    cx.simulate_keystrokes("up up");
    cx.run_until_parked();
    let marked = fx.quick.read_with(cx, |quick, cx| {
        quick.messages().unwrap().read(cx).selected()
    });
    assert_eq!(marked, Some(fx.messages(cx)[1].id), "the first answer");

    cx.simulate_keystrokes("cmd-shift-enter");
    cx.run_until_parked();
    assert_eq!(text(&fx.editor, cx), "The swift brown fox\nNext.");
    assert!(!fx.is_open(cx));
}

#[gpui::test]
fn tab_moves_between_the_composer_and_the_conversation(cx: &mut TestAppContext) {
    let (fx, cx) = open_quick(paragraphs(&["The quick brown fox"]), cx);
    fx.toggle(cx);
    // Nothing said yet: tab stays in the composer.
    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    assert!(fx.composer_focused(cx));

    fx.fake.reply("An answer.");
    fx.send("A question?", cx);
    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    assert!(!fx.composer_focused(cx));
    assert!(fx.has_keyboard(cx));
    assert!(
        fx.is_open(cx),
        "moving inside the popover does not close it"
    );
    cx.simulate_keystrokes("shift-tab");
    cx.run_until_parked();
    assert!(fx.composer_focused(cx));

    // escape in the conversation closes, as in the composer.
    cx.simulate_keystrokes("tab escape");
    cx.run_until_parked();
    assert!(!fx.is_open(cx));
    assert!(fx.editor_focused(cx));
}

#[gpui::test]
fn a_read_only_document_can_be_asked_about_but_not_changed(cx: &mut TestAppContext) {
    let (fx, cx) = open_quick(paragraphs(&["The quick brown fox", "Next."]), cx);
    select(&fx.editor, TextRange::in_block(0, 4, 9), cx);
    fx.editor
        .update(cx, |editor, cx| editor.set_read_only(true, cx));
    fx.toggle(cx);
    fx.fake.reply("swift");
    fx.send("Another word?", cx);

    assert_eq!(replace_state(&fx, cx), Some(ReplaceState::ReadOnly));
    assert!(!action_enabled(QuickAction::Replace, &fx, cx));
    assert!(!action_enabled(QuickAction::InsertBelow, &fx, cx));
    assert!(action_enabled(QuickAction::Copy, &fx, cx));
    cx.simulate_keystrokes("cmd-shift-enter cmd-enter");
    cx.run_until_parked();
    assert_eq!(text(&fx.editor, cx), "The quick brown fox\nNext.");
    assert!(fx.is_open(cx));
}

#[gpui::test]
fn a_press_right_after_an_answer_arrived_lands_on_its_button(cx: &mut TestAppContext) {
    // The frame that shows a new answer measures the conversation before
    // it draws it. What that leaves in the frame must not be in the way
    // of the pointer: no move first, the press comes straight down.
    let (fx, cx) = asked("swift", cx);
    let button = bounds(action_button(QuickAction::Replace, &fx, cx), cx);
    cx.simulate_click(button.center(), gpui::Modifiers::none());
    cx.run_until_parked();
    assert_eq!(text(&fx.editor, cx), "The swift brown fox\nNext.");
    assert!(!fx.is_open(cx));
}

#[gpui::test]
fn the_keys_work_in_the_frame_that_measured(cx: &mut TestAppContext) {
    let (fx, cx) = open_quick(paragraphs(&["The quick brown fox", "Next."]), cx);
    select(&fx.editor, TextRange::in_block(0, 4, 9), cx);
    fx.toggle(cx);
    fx.fake.hold();
    fx.fake
        .push(innerword_assistant::FakeReply::pieces(["swi", "ft"]));
    fx.send("Another word?", cx);
    fx.first_piece(cx);
    // Typing goes on in the composer while the answer grows, frame after
    // frame.
    cx.simulate_input("and ");
    fx.fake.advance(1);
    cx.run_until_parked();
    cx.simulate_input("then?");
    cx.run_until_parked();
    assert_eq!(fx.draft(cx), "and then?");
    assert!(fx.composer_focused(cx));
    fx.fake.release();
    cx.run_until_parked();
    cx.simulate_keystrokes("cmd-a backspace cmd-shift-enter");
    cx.run_until_parked();
    assert_eq!(text(&fx.editor, cx), "The swift brown fox\nNext.");
}
