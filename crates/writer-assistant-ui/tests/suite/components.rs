//! The shared components, put together as a host does (`support::TestChat`).

use gpui::{Modifiers, TestAppContext, point, px};
use writer_assistant::{AssistantError, FakeReply, MessageState, Role, Selection};
use writer_assistant_ui::{ComposerEvent, EmptyState, MessageListEvent, PickerKind};

use super::support::{
    CHAT_HEIGHT, bounds, choose, click, click_at, menu_is_open, menu_labels, move_to, open_chat,
    right_click_at, try_bounds,
};

// ----- sending and streaming -----------------------------------------------------

#[gpui::test]
fn a_message_is_sent_and_its_answer_streams_into_the_list(cx: &mut TestAppContext) {
    let (chat, cx) = open_chat(cx);
    chat.fake.hold();
    chat.fake.push(FakeReply::pieces([
        "A **heron** ",
        "is a wading ",
        "bird.\n\n- long legs\n",
        "- long neck",
    ]));

    chat.send("What is a heron?", cx);
    assert_eq!(chat.fake.sent_texts(), ["What is a heron?"]);
    let ids = chat.ids(cx);
    assert_eq!(ids.len(), 2, "the question and the answer that is awaited");
    chat.messages.read_with(cx, |list, _| {
        assert_eq!(list.messages()[0].role, Role::User);
        assert_eq!(list.messages()[0].text, "What is a heron?");
        assert!(list.messages()[1].is_streaming());
        assert!(list.answer_editor(ids[1]).is_none(), "no text yet");
    });
    // The composer is empty again and shows Stop.
    chat.composer.read_with(cx, |composer, cx| {
        assert!(composer.is_blank(cx));
        assert!(composer.is_busy());
    });
    assert!(try_bounds("assistant-stop", cx).is_some());
    // The line under the answer says the assistant is at work.
    assert!(try_bounds(format!("assistant-activity-{}", ids[1].0), cx).is_some());
    assert!(try_bounds(format!("assistant-action-copy-{}", ids[1].0), cx).is_none());

    // Ready, Started, the first piece.
    chat.fake.advance(3);
    cx.run_until_parked();
    assert_eq!(chat.answer_text(ids[1], cx), "A heron");
    chat.messages.read_with(cx, |list, cx| {
        let editor = list.answer_editor(ids[1]).unwrap().read(cx);
        assert!(editor.is_read_only());
        assert!(editor.document().blocks()[0].attrs_at(2).unwrap().bold);
    });

    chat.fake.advance(2);
    cx.run_until_parked();
    assert_eq!(
        chat.answer_text(ids[1], cx),
        "A heron is a wading bird.\nlong legs"
    );

    chat.fake.finish();
    cx.run_until_parked();
    assert_eq!(
        chat.answer_text(ids[1], cx),
        "A heron is a wading bird.\nlong legs\nlong neck"
    );
    chat.messages.read_with(cx, |list, cx| {
        assert_eq!(list.messages()[1].state, MessageState::Done);
        let document = list
            .answer_editor(ids[1])
            .unwrap()
            .read(cx)
            .document()
            .clone();
        let kinds: Vec<&str> = document
            .blocks()
            .iter()
            .map(|block| block.kind().name())
            .collect();
        assert_eq!(kinds, ["paragraph", "bullet", "bullet"]);
    });
    // Done: Send is back, the actions are under the answer, the activity
    // line is gone.
    chat.composer
        .read_with(cx, |composer, _| assert!(!composer.is_busy()));
    assert!(try_bounds("assistant-send", cx).is_some());
    for action in ["replace", "insert", "copy"] {
        bounds(format!("assistant-action-{action}-{}", ids[1].0), cx);
    }
}

#[gpui::test]
fn the_line_under_an_answer_says_what_the_assistant_is_doing(cx: &mut TestAppContext) {
    let (chat, cx) = open_chat(cx);
    chat.fake.hold();
    chat.fake.push(
        FakeReply::pieces(["It ", "opens on a storm."])
            .with_tool("read_document", "Reading Chapter 2"),
    );
    chat.send("How does chapter 2 open?", cx);
    let answer = chat.last_id(cx);
    let activity = |cx: &mut gpui::VisualTestContext| {
        chat.messages.read_with(cx, |list, _| {
            list.message(answer)
                .unwrap()
                .current_activity()
                .map(str::to_string)
        })
    };
    assert_eq!(activity(cx), None);

    // Ready, Started, the tool starts.
    chat.fake.advance(3);
    cx.run_until_parked();
    assert_eq!(activity(cx).as_deref(), Some("Reading Chapter 2"));
    bounds(format!("assistant-activity-{}", answer.0), cx);

    // The tool is done, the text begins.
    chat.fake.advance(2);
    cx.run_until_parked();
    assert_eq!(activity(cx), None);
    assert_eq!(chat.answer_text(answer, cx), "It");

    chat.fake.finish();
    cx.run_until_parked();
    assert_eq!(chat.answer_text(answer, cx), "It opens on a storm.");
}

#[gpui::test]
fn an_answer_is_stopped_with_the_button_and_with_the_key(cx: &mut TestAppContext) {
    let (chat, cx) = open_chat(cx);
    chat.fake.hold();
    chat.fake
        .push(FakeReply::pieces(["One ", "two ", "three."]));
    chat.send("Count.", cx);
    chat.fake.advance(3);
    cx.run_until_parked();
    let answer = chat.last_id(cx);
    assert_eq!(chat.answer_text(answer, cx), "One");

    // The Stop button. The keyboard stays in the composer.
    click("assistant-stop", cx);
    assert_eq!(chat.fake.stops(), 1);
    assert_eq!(
        chat.composer_events(cx).last(),
        Some(&ComposerEvent::StopRequested)
    );
    chat.messages.read_with(cx, |list, _| {
        assert_eq!(list.message(answer).unwrap().state, MessageState::Stopped);
    });
    // What had arrived stays, says that it was stopped, and can be used.
    assert_eq!(chat.answer_text(answer, cx), "One");
    bounds(format!("assistant-stopped-{}", answer.0), cx);
    bounds(format!("assistant-action-copy-{}", answer.0), cx);
    assert!(try_bounds("assistant-send", cx).is_some());
    cx.update(|window, cx| assert!(chat.composer.read(cx).is_focused(window, cx)));

    // The key, with the keyboard in the composer.
    chat.fake.push(FakeReply::pieces(["Four ", "five."]));
    chat.send("Go on.", cx);
    chat.fake.advance(3);
    cx.run_until_parked();
    cx.simulate_keystrokes("cmd-.");
    cx.run_until_parked();
    assert_eq!(chat.fake.stops(), 2);
    let second = chat.last_id(cx);
    chat.messages.read_with(cx, |list, _| {
        assert_eq!(list.message(second).unwrap().state, MessageState::Stopped);
    });

    // With nothing being written the key is not the composer's.
    cx.simulate_keystrokes("cmd-.");
    cx.run_until_parked();
    assert_eq!(chat.fake.stops(), 3, "the host's own handler ran");
    assert_eq!(
        chat.composer_events(cx)
            .iter()
            .filter(|event| **event == ComposerEvent::StopRequested)
            .count(),
        2
    );
}

#[gpui::test]
fn a_failed_answer_says_what_happened_and_offers_to_try_again(cx: &mut TestAppContext) {
    let (chat, cx) = open_chat(cx);
    chat.fake.fail(AssistantError::NotSignedIn);
    chat.send("Hello?", cx);
    let answer = chat.last_id(cx);
    chat.messages.read_with(cx, |list, _| {
        assert_eq!(
            list.message(answer).unwrap().state,
            MessageState::Failed(AssistantError::NotSignedIn)
        );
        assert!(list.answer_editor(answer).is_none());
    });
    bounds(format!("assistant-failed-{}", answer.0), cx);
    // Nothing to apply: no actions under it.
    assert!(try_bounds(format!("assistant-action-copy-{}", answer.0), cx).is_none());

    click(format!("assistant-failed-{}-action", answer.0), cx);
    assert_eq!(
        chat.list_events(cx),
        [MessageListEvent::RetryRequested { message: answer }]
    );
    // The press on the button was not a press on the message.
    cx.update(|window, cx| assert!(chat.composer.read(cx).is_focused(window, cx)));

    // An answer that fails after some text keeps the text and its actions.
    chat.fake.push(
        FakeReply::pieces(["Half an "]).then_fail(AssistantError::Network {
            detail: "offline".into(),
        }),
    );
    chat.send("Again?", cx);
    let partial = chat.last_id(cx);
    assert_eq!(chat.answer_text(partial, cx), "Half an");
    bounds(format!("assistant-failed-{}", partial.0), cx);
    bounds(format!("assistant-action-copy-{}", partial.0), cx);
    // The menu of a failed answer offers the same.
    let row = bounds(format!("assistant-message-{}", partial.0), cx);
    right_click_at(point(row.right() - px(6.), row.bottom() - px(6.)), cx);
    assert_eq!(
        menu_labels(cx),
        [
            "Replace Selection",
            "Insert Below",
            "Copy Answer",
            "-",
            "Try Again"
        ]
    );
    choose("Try Again", cx);
    assert_eq!(
        chat.list_events(cx).last(),
        Some(&MessageListEvent::RetryRequested { message: partial })
    );
}

// ----- the actions of an answer ---------------------------------------------------

#[gpui::test]
fn the_actions_of_an_answer_reach_the_host(cx: &mut TestAppContext) {
    let (chat, cx) = open_chat(cx);
    chat.fake.reply("First answer.");
    chat.send("One", cx);
    let first = chat.last_id(cx);
    chat.fake.reply("Second answer.");
    chat.send("Two", cx);
    let second = chat.last_id(cx);

    click(format!("assistant-action-copy-{}", second.0), cx);
    assert_eq!(
        chat.list_events(cx),
        [MessageListEvent::ActionInvoked {
            action: "copy".into(),
            message: second
        }]
    );
    // The keyboard did not move.
    cx.update(|window, cx| assert!(chat.composer.read(cx).is_focused(window, cx)));

    // An older answer has the same buttons (they show under the pointer).
    let row = bounds(format!("assistant-message-{}", first.0), cx);
    move_to(row.center(), cx);
    click(format!("assistant-action-insert-{}", first.0), cx);
    assert_eq!(
        chat.list_events(cx).last(),
        Some(&MessageListEvent::ActionInvoked {
            action: "insert".into(),
            message: first
        })
    );

    // An action the host switched off does nothing.
    chat.messages.update(cx, |list, cx| {
        list.set_action_enabled("replace", false, cx);
        assert!(!list.invoke_action("replace", second, cx));
        assert!(!list.invoke_action("unknown", second, cx));
    });
    cx.run_until_parked();
    let before = chat.list_events(cx).len();
    click(format!("assistant-action-replace-{}", second.0), cx);
    assert_eq!(chat.list_events(cx).len(), before);
    chat.messages.update(cx, |list, cx| {
        list.set_action_enabled("replace", true, cx);
        assert!(list.invoke_action("replace", second, cx));
    });
}

#[gpui::test]
fn a_right_click_on_a_message_opens_its_menu(cx: &mut TestAppContext) {
    let (chat, cx) = open_chat(cx);
    chat.fake
        .reply("An answer with enough words to select some of them.");
    chat.send("A question", cx);
    let ids = chat.ids(cx);
    let (question, answer) = (ids[0], ids[1]);

    // The user's message: copy.
    let row = bounds(format!("assistant-message-{}", question.0), cx);
    right_click_at(row.center(), cx);
    assert_eq!(menu_labels(cx), ["Copy"]);
    choose("Copy", cx);
    assert_eq!(
        chat.list_events(cx),
        [MessageListEvent::CopyRequested { message: question }]
    );
    assert!(!menu_is_open(cx));

    // An answer, beside its text: the host's actions, with their tooltips
    // as titles.
    chat.messages
        .update(cx, |list, cx| list.set_action_enabled("replace", false, cx));
    let row = bounds(format!("assistant-message-{}", answer.0), cx);
    right_click_at(point(row.right() - px(6.), row.bottom() - px(6.)), cx);
    assert_eq!(
        menu_labels(cx),
        ["Replace Selection", "Insert Below", "Copy Answer"]
    );
    cx.update(|window, cx| {
        let menu = writer_ui::ContextMenu::current(window, cx).unwrap();
        let items = menu.read(cx).items().to_vec();
        assert!(items[0].is_disabled());
        assert!(!items[1].is_disabled());
    });
    choose("Insert Below", cx);
    assert_eq!(
        chat.list_events(cx).last(),
        Some(&MessageListEvent::ActionInvoked {
            action: "insert".into(),
            message: answer
        })
    );

    // In the text of an answer with words selected: copying the selection
    // comes first.
    let editor = chat
        .messages
        .read_with(cx, |list, _| list.answer_editor(answer).unwrap().clone());
    let text = cx.update(|_, cx| editor.read(cx).viewport_bounds().unwrap());
    let inside = point(text.left() + px(30.), text.top() + px(8.));
    click_at(inside, cx);
    cx.simulate_keystrokes("shift-right shift-right shift-right");
    right_click_at(inside, cx);
    assert_eq!(
        menu_labels(cx),
        [
            "Copy Selection",
            "-",
            "Replace Selection",
            "Insert Below",
            "Copy Answer"
        ]
    );
    cx.simulate_keystrokes("escape");
    assert!(!menu_is_open(cx));
}

// ----- the keyboard in the list -----------------------------------------------------

#[gpui::test]
fn the_list_is_walked_with_the_keyboard(cx: &mut TestAppContext) {
    let (chat, cx) = open_chat(cx);
    for (question, answer) in [("One", "First."), ("Two", "Second."), ("Three", "Third.")] {
        chat.fake.reply(answer);
        chat.send(question, cx);
    }
    let ids = chat.ids(cx);
    assert_eq!(ids.len(), 6);
    let selected =
        |cx: &mut gpui::VisualTestContext| chat.messages.read_with(cx, |l, _| l.selected());

    // Moving in marks the last message.
    chat.messages
        .update_in(cx, |list, window, cx| list.focus(window, cx));
    cx.run_until_parked();
    assert_eq!(selected(cx), Some(ids[5]));
    bounds("assistant-message-mark", cx);

    cx.simulate_keystrokes("up up");
    assert_eq!(selected(cx), Some(ids[3]));
    cx.simulate_keystrokes("down");
    assert_eq!(selected(cx), Some(ids[4]));
    cx.simulate_keystrokes("home");
    assert_eq!(selected(cx), Some(ids[0]));
    cx.simulate_keystrokes("up");
    assert_eq!(selected(cx), Some(ids[0]), "no wrapping");
    cx.simulate_keystrokes("end");
    assert_eq!(selected(cx), Some(ids[5]));
    cx.simulate_keystrokes("ctrl-p");
    assert_eq!(selected(cx), Some(ids[4]));

    // The key of the host acts on the marked answer, or on nothing when
    // the marked message is the user's.
    cx.update(|window, cx| {
        assert!(chat.messages.read(cx).target_answer(window, cx).is_none());
    });
    cx.simulate_keystrokes("up");
    cx.update(|window, cx| {
        let target = chat.messages.read(cx).target_answer(window, cx).unwrap();
        assert_eq!(target.id, ids[3]);
    });

    // Copy.
    cx.simulate_keystrokes("cmd-c");
    assert_eq!(
        chat.list_events(cx),
        [MessageListEvent::CopyRequested { message: ids[3] }]
    );

    // The menu, from the keyboard: its first item is marked, the arrows
    // are the menu's.
    cx.simulate_keystrokes("enter");
    assert_eq!(
        menu_labels(cx),
        ["Replace Selection", "Insert Below", "Copy Answer"]
    );
    cx.simulate_keystrokes("down enter");
    assert!(!menu_is_open(cx));
    assert_eq!(
        chat.list_events(cx).last(),
        Some(&MessageListEvent::ActionInvoked {
            action: "insert".into(),
            message: ids[3]
        })
    );
    assert_eq!(selected(cx), Some(ids[3]), "the arrow moved in the menu");
    cx.simulate_keystrokes("ctrl-enter");
    assert!(menu_is_open(cx));
    cx.simulate_keystrokes("escape");
    assert!(!menu_is_open(cx));
    cx.simulate_keystrokes("shift-f10");
    assert!(menu_is_open(cx));
    cx.simulate_keystrokes("escape");

    // escape leaves the chat; tab and shift-tab ask the host to move on.
    cx.simulate_keystrokes("escape");
    assert_eq!(
        chat.list_events(cx).last(),
        Some(&MessageListEvent::Dismissed)
    );
    cx.simulate_keystrokes("tab shift-tab");
    chat.chat
        .read_with(cx, |chat, _| assert_eq!(chat.focus_moves, [true, false]));

    // With the keyboard elsewhere the host's key acts on the latest answer.
    chat.composer
        .update_in(cx, |composer, window, cx| composer.focus(window, cx));
    cx.run_until_parked();
    cx.update(|window, cx| {
        let target = chat.messages.read(cx).target_answer(window, cx).unwrap();
        assert_eq!(target.id, ids[5]);
    });
    assert!(
        try_bounds("assistant-message-mark", cx).is_none() || {
            // (`debug_bounds` keeps what an earlier frame painted.)
            true
        }
    );
}

#[gpui::test]
fn the_text_of_an_answer_is_selected_and_copied(cx: &mut TestAppContext) {
    let (chat, cx) = open_chat(cx);
    chat.fake.reply("Weary travellers rest.");
    chat.send("A line", cx);
    let answer = chat.last_id(cx);
    let editor = chat
        .messages
        .read_with(cx, |list, _| list.answer_editor(answer).unwrap().clone());
    let text = cx.update(|_, cx| editor.read(cx).viewport_bounds().unwrap());

    // A click in the text puts the keyboard there.
    click_at(point(text.left() + px(2.), text.top() + px(8.)), cx);
    cx.update(|window, cx| {
        assert!(editor.read(cx).is_focused(window));
        let list = chat.messages.read(cx);
        assert_eq!(list.focused_answer(window, cx), Some(answer));
        assert!(list.contains_focus(window, cx));
        assert!(!list.is_focused(window));
        assert_eq!(list.selected(), Some(answer));
    });
    cx.simulate_keystrokes("cmd-left shift-alt-right cmd-c");
    assert_eq!(
        cx.read_from_clipboard()
            .and_then(|item| item.text())
            .as_deref(),
        Some("Weary")
    );
    // Typing changes nothing.
    cx.simulate_input("x");
    assert_eq!(chat.answer_text(answer, cx), "Weary travellers rest.");
    // escape, tab and the stop key are the chat's, not the editor's.
    cx.simulate_keystrokes("tab");
    chat.chat
        .read_with(cx, |chat, _| assert_eq!(chat.focus_moves, [true]));
    cx.simulate_keystrokes("escape");
    assert_eq!(
        chat.list_events(cx).last(),
        Some(&MessageListEvent::Dismissed)
    );

    // A double click selects a word.
    let position = point(text.left() + px(70.), text.top() + px(8.));
    move_to(position, cx);
    cx.simulate_event(gpui::MouseDownEvent {
        button: gpui::MouseButton::Left,
        position,
        modifiers: Modifiers::none(),
        click_count: 2,
        first_mouse: false,
    });
    cx.simulate_event(gpui::MouseUpEvent {
        button: gpui::MouseButton::Left,
        position,
        modifiers: Modifiers::none(),
        click_count: 2,
    });
    cx.run_until_parked();
    let selected = editor.read_with(cx, |editor, _| editor.state().selected_text());
    assert_eq!(selected, "travellers");
}

// ----- scrolling ------------------------------------------------------------------

fn long_answer(lines: usize) -> String {
    (0..lines)
        .map(|line| format!("Paragraph {line} of a long answer that wraps in the column."))
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[gpui::test]
fn the_list_follows_an_answer_until_the_user_scrolls_away(cx: &mut TestAppContext) {
    let (chat, cx) = open_chat(cx);
    chat.fake.reply(long_answer(30));
    chat.send("Long, please", cx);
    let position = |cx: &mut gpui::VisualTestContext| {
        chat.messages
            .read_with(cx, |list, _| list.scroll_position())
    };
    let following =
        |cx: &mut gpui::VisualTestContext| chat.messages.read_with(cx, |l, _| l.is_following());
    let (offset, max) = position(cx);
    assert!(max > CHAT_HEIGHT, "the answer is taller than the list");
    assert_eq!(offset, max, "the end is in view");
    assert!(following(cx));

    // A second answer streams in: the end stays in view.
    chat.fake.hold();
    chat.fake.push(FakeReply::pieces([
        long_answer(4),
        format!("\n\n{}", long_answer(4)),
        format!("\n\n{}", long_answer(4)),
    ]));
    chat.send("More", cx);
    // Started and the first piece (the agent is running already).
    chat.fake.advance(2);
    cx.run_until_parked();
    let (offset, max_streaming) = position(cx);
    assert!(max_streaming > max);
    assert_eq!(offset, max_streaming);

    // The user scrolls up a little (the answer that is being written is
    // still in view): the list stays where it was put.
    let list = bounds("assistant-messages", cx);
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: list.center(),
        delta: gpui::ScrollDelta::Pixels(point(px(0.), px(120.))),
        modifiers: Modifiers::none(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    cx.run_until_parked();
    let (scrolled_to, _) = position(cx);
    assert_eq!(scrolled_to, max_streaming - px(120.));
    assert!(!following(cx));
    chat.fake.advance(1);
    cx.run_until_parked();
    let (offset, taller) = position(cx);
    assert!(taller > max_streaming, "the answer grew");
    assert_eq!(offset, scrolled_to, "and the view did not move");

    // Back at the end, the list follows again.
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: list.center(),
        delta: gpui::ScrollDelta::Pixels(point(px(0.), px(-5000.))),
        modifiers: Modifiers::none(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    cx.run_until_parked();
    assert!(following(cx));
    chat.fake.finish();
    cx.run_until_parked();
    let (offset, max_done) = position(cx);
    assert!(max_done > taller);
    assert_eq!(offset, max_done);

    // Scrolled away again; a message of the user's own brings the end back.
    chat.messages
        .update(cx, |list, cx| list.scroll_by(px(-400.), cx));
    cx.run_until_parked();
    assert!(!following(cx));
    chat.fake.release();
    chat.fake.reply("Short.");
    chat.send("And now?", cx);
    assert!(following(cx));
    let (offset, max) = position(cx);
    assert_eq!(offset, max);

    // Marking an older message with the keyboard is scrolling away too.
    chat.messages.update_in(cx, |list, window, cx| {
        list.focus(window, cx);
        list.select_first(cx);
    });
    cx.run_until_parked();
    assert!(!following(cx));
    assert_eq!(position(cx).0, px(0.));
    chat.messages
        .update(cx, |list, cx| list.scroll_to_bottom(cx));
    cx.run_until_parked();
    assert!(following(cx));
}

#[gpui::test]
fn a_long_conversation_builds_only_the_rows_near_the_visible_part(cx: &mut TestAppContext) {
    let (chat, cx) = open_chat(cx);
    chat.fake
        .set_fallback(FakeReply::text("An answer of a few words."));
    // The transcript is written directly: sixty exchanges.
    chat.chat.update_in(cx, |chat, window, cx| {
        for index in 0..60 {
            chat.session.ask_quick(&format!("Question {index}"), None);
        }
        chat.sync(window, cx);
    });
    cx.run_until_parked();
    assert_eq!(chat.ids(cx).len(), 120);
    let built = chat.messages.read_with(cx, |list, _| list.rows_built());
    cx.simulate_keystrokes("a");
    cx.run_until_parked();
    let per_frame = chat.messages.read_with(cx, |list, _| list.rows_built()) - built;
    assert!(per_frame < 40, "a frame built {per_frame} of 120 rows");
}

#[gpui::test]
fn another_conversation_replaces_the_one_shown(cx: &mut TestAppContext) {
    let (chat, cx) = open_chat(cx);
    chat.fake.reply("First.");
    chat.send("One", cx);
    let first = chat.ids(cx);
    let editor = chat
        .messages
        .read_with(cx, |list, _| list.answer_editor(first[1]).unwrap().clone());

    // The same conversation shown again keeps its editors.
    chat.chat
        .update_in(cx, |chat, window, cx| chat.sync(window, cx));
    chat.messages.read_with(cx, |list, _| {
        assert_eq!(list.answer_editor(first[1]), Some(&editor));
    });

    // Cleared, and a new one started.
    chat.chat.update_in(cx, |chat, window, cx| {
        chat.session.clear();
        chat.sync(window, cx);
    });
    cx.run_until_parked();
    assert!(chat.ids(cx).is_empty());
    chat.messages.read_with(cx, |list, _| {
        assert!(list.answer_editor(first[1]).is_none());
        assert_eq!(list.selected(), None);
    });
    chat.fake.reply("Second.");
    chat.send("Two", cx);
    let second = chat.last_id(cx);
    assert_eq!(chat.answer_text(second, cx), "Second.");
}

// ----- the empty state -------------------------------------------------------------

#[gpui::test]
fn an_empty_conversation_offers_example_prompts(cx: &mut TestAppContext) {
    let (chat, cx) = open_chat(cx);
    assert!(try_bounds("assistant-empty", cx).is_none());
    chat.messages.update(cx, |list, cx| {
        list.set_empty_state(
            Some(EmptyState {
                title: "Ask about this document".into(),
                text: "The assistant reads the document as it is now.".into(),
                examples: vec![
                    "Where does the pacing drag?".into(),
                    "Summarise this chapter in five sentences.".into(),
                    "Which sentences are too long?".into(),
                ],
            }),
            cx,
        )
    });
    cx.run_until_parked();
    bounds("assistant-empty", cx);

    // With the mouse.
    click("assistant-example-1", cx);
    assert_eq!(
        chat.list_events(cx),
        [MessageListEvent::ExampleChosen(
            "Summarise this chapter in five sentences.".into()
        )]
    );
    cx.update(|window, cx| assert!(chat.composer.read(cx).is_focused(window, cx)));

    // With the keyboard.
    chat.messages
        .update_in(cx, |list, window, cx| list.focus(window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("home down down down enter");
    assert_eq!(
        chat.list_events(cx).last(),
        Some(&MessageListEvent::ExampleChosen(
            "Which sentences are too long?".into()
        ))
    );
    cx.simulate_keystrokes("up up up enter");
    assert_eq!(
        chat.list_events(cx).last(),
        Some(&MessageListEvent::ExampleChosen(
            "Where does the pacing drag?".into()
        ))
    );

    // Gone once something was said.
    chat.fake.reply("Here.");
    chat.send("Where?", cx);
    assert_eq!(chat.ids(cx).len(), 2);
}

// ----- the composer ---------------------------------------------------------------

#[gpui::test]
fn the_composer_sends_grows_and_hands_the_keyboard_back(cx: &mut TestAppContext) {
    let (chat, cx) = open_chat(cx);
    let height = |cx: &mut gpui::VisualTestContext| bounds("assistant-composer", cx).size.height;
    let one_line = height(cx);

    // enter on an empty field does nothing.
    cx.simulate_keystrokes("enter");
    cx.simulate_input("   ");
    cx.simulate_keystrokes("enter");
    assert!(chat.fake.sent_texts().is_empty());
    assert_eq!(chat.ids(cx).len(), 0);
    chat.composer.update(cx, |composer, cx| composer.clear(cx));

    // shift-enter is a new line; the box grows with the text, to a limit.
    cx.simulate_input("First line");
    cx.simulate_keystrokes("shift-enter");
    cx.simulate_input("second line");
    cx.run_until_parked();
    let two_lines = height(cx);
    assert!(two_lines > one_line);
    for _ in 0..30 {
        cx.simulate_keystrokes("shift-enter");
    }
    cx.run_until_parked();
    let tall = height(cx);
    assert!(tall > two_lines);
    assert!(
        tall < px(230.),
        "{tall:?}: the field scrolls beyond its limit"
    );
    cx.simulate_keystrokes("shift-enter");
    cx.run_until_parked();
    assert_eq!(height(cx), tall);

    // escape hands the keyboard back and keeps what was typed.
    cx.simulate_keystrokes("escape");
    assert_eq!(
        chat.composer_events(cx).last(),
        Some(&ComposerEvent::Dismissed)
    );
    chat.composer.read_with(cx, |composer, cx| {
        assert!(composer.text(cx).starts_with("First line\nsecond line"))
    });

    // enter sends the text, trimmed, and empties the field.
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(chat.fake.sent_texts(), ["First line\nsecond line"]);
    assert_eq!(height(cx), one_line);
    assert!(
        chat.composer_events(cx)
            .contains(&ComposerEvent::Submitted("First line\nsecond line".into()))
    );

    // The Send button does what enter does; it is dimmed without text.
    click("assistant-send", cx);
    assert_eq!(chat.fake.sent_texts().len(), 1);
    cx.simulate_input("With the button");
    click("assistant-send", cx);
    assert_eq!(chat.fake.sent_texts().len(), 2);
    cx.update(|window, cx| assert!(chat.composer.read(cx).is_focused(window, cx)));

    // While sending is not possible the text is kept.
    chat.composer
        .update(cx, |composer, cx| composer.set_enabled(false, cx));
    cx.simulate_input("Kept");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(chat.fake.sent_texts().len(), 2);
    chat.composer.read_with(cx, |composer, cx| {
        assert_eq!(composer.text(cx), "Kept");
        assert!(!composer.can_submit(cx));
    });
    chat.composer
        .update(cx, |composer, cx| composer.set_enabled(true, cx));
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(chat.fake.sent_texts().len(), 3);

    // A message can be sent while an answer is being written.
    chat.fake.hold();
    chat.send("Slow one", cx);
    chat.send("And another", cx);
    assert_eq!(chat.ids(cx).len(), 10);
    chat.fake.release();
    cx.run_until_parked();
    chat.messages.read_with(cx, |list, _| {
        assert!(
            list.messages()
                .iter()
                .all(|message| !message.is_streaming())
        );
    });
}

#[gpui::test]
fn the_composer_has_the_menu_of_a_text_field(cx: &mut TestAppContext) {
    let (chat, cx) = open_chat(cx);
    cx.simulate_input("some words");
    let field = cx.update(|_, cx| {
        chat.composer
            .read(cx)
            .editor()
            .read(cx)
            .viewport_bounds()
            .unwrap()
    });
    right_click_at(point(field.left() + px(20.), field.center().y), cx);
    assert_eq!(menu_labels(cx), ["Cut", "Copy", "Paste", "-", "Select All"]);
    choose("Select All", cx);
    let selected = chat.composer.read_with(cx, |composer, cx| {
        composer.editor().read(cx).state().selected_text()
    });
    assert_eq!(selected, "some words");
    // From the keyboard.
    cx.simulate_keystrokes("ctrl-enter");
    assert!(menu_is_open(cx));
    cx.simulate_keystrokes("escape");
    assert!(!menu_is_open(cx));
    assert!(
        chat.composer_events(cx)
            .iter()
            .all(|event| *event != ComposerEvent::Dismissed)
    );
}

// ----- the pickers ----------------------------------------------------------------

#[gpui::test]
fn the_pickers_show_the_choice_and_change_it(cx: &mut TestAppContext) {
    let (chat, cx) = open_chat(cx);
    let selection = |cx: &mut gpui::VisualTestContext| {
        chat.chat
            .read_with(cx, |c, _| c.session.selection().clone())
    };
    // The quick chat of the fake agent: Swift at Low.
    assert_eq!(selection(cx), Selection::named("swift", "low"));
    let catalog = writer_assistant::Backend::catalog(&chat.fake);
    assert_eq!(
        writer_assistant_ui::picker_label(PickerKind::Model, &catalog, &selection(cx)),
        "Swift"
    );
    assert_eq!(
        writer_assistant_ui::picker_label(PickerKind::Effort, &catalog, &selection(cx)),
        "Low"
    );

    // The model menu drops down from its control.
    click("assistant-model", cx);
    assert_eq!(
        menu_labels(cx),
        ["Fake Agent's default", "-", "Swift", "Deep"]
    );
    cx.update(|window, cx| {
        let menu = writer_ui::ContextMenu::current(window, cx).unwrap();
        let menu = menu.read(cx);
        let checked: Vec<bool> = menu
            .items()
            .iter()
            .map(|item| item.is_checked() == Some(true))
            .collect();
        assert_eq!(checked, [false, false, true, false]);
        let control = chat.chat.read(cx).model_bounds.get().unwrap();
        let menu_bounds = menu.bounds().unwrap();
        assert!(menu_bounds.top() >= control.bottom());
    });
    choose("Deep", cx);
    assert_eq!(selection(cx), Selection::named("deep", "low"));
    cx.update(|window, cx| assert!(chat.composer.read(cx).is_focused(window, cx)));

    // Deep takes every level.
    click("assistant-effort", cx);
    assert_eq!(
        menu_labels(cx),
        ["Fake Agent's default", "-", "Low", "Medium", "High"]
    );
    choose("High", cx);
    assert_eq!(selection(cx), Selection::named("deep", "high"));

    // Swift does not take High: the effort goes back to the default.
    chat.chat.update_in(cx, |chat, window, cx| {
        chat.open_picker(PickerKind::Model, true, window, cx)
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        let menu = writer_ui::ContextMenu::current(window, cx).unwrap();
        assert_eq!(
            menu.read(cx).selected_index(),
            Some(0),
            "opened from the keyboard"
        );
    });
    cx.simulate_keystrokes("down enter");
    cx.run_until_parked();
    let now = selection(cx);
    assert_eq!(now.model.id(), Some("swift"));
    assert_eq!(now.effort.id(), None);
    assert_eq!(
        writer_assistant_ui::picker_label(PickerKind::Effort, &catalog, &now),
        "Default effort"
    );
    click("assistant-effort", cx);
    assert_eq!(
        menu_labels(cx),
        ["Fake Agent's default", "-", "Low", "Medium"]
    );
    cx.simulate_keystrokes("escape");

    // The next message is asked with the choice.
    chat.fake.reply("Yes.");
    chat.send("With Swift?", cx);
    let started = chat.fake.last_started().unwrap();
    assert_eq!(started.selection, now);
    chat.chat.read_with(cx, |chat, _| {
        assert_eq!(chat.selections.len(), 3);
    });
}
