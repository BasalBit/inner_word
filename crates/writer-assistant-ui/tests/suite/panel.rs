//! The side chat in a workspace (`support::Workspace`), on the fake
//! backend: sending, the context, a conversation per document, what
//! reaches the host, the states, the keyboard and the mouse.

use gpui::{TestAppContext, VisualTestContext, point, px};
use writer_assistant::{
    AssistantError, Attachment, AttachmentKind, Availability, FakeReply, MessageState, Selection,
    SignIn,
};
use writer_assistant_ui::{
    AddContext, ApplyKind, AssistantPanelEvent, AssistantPanelOptions, ClearChat, HeaderControl,
    InsertAtCaret, NewChat, PanelArea, PanelNotice,
};

use super::support::{
    Focus, PANEL_WIDTH, bounds, choose, click, click_at, menu_is_open, menu_labels, move_to,
    open_panel, open_panel_on, right_click_at, setup,
};

// ----- sending ------------------------------------------------------------------

#[gpui::test]
fn a_message_goes_out_with_the_open_document_as_it_is_now(cx: &mut TestAppContext) {
    let (ws, cx) = open_panel(cx);
    assert_eq!(ws.focus(cx), Focus::Text);
    // An empty conversation says what the chat is for.
    bounds("assistant-empty", cx);
    assert!(
        ws.fake.started().is_empty(),
        "nothing runs before a message"
    );

    ws.fake.reply("It begins **well**.");
    ws.send("How does it begin?", cx);
    let sent = ws.fake.last_sent().unwrap();
    assert!(sent.contains("Chapter 1 begins here."), "{sent}");
    assert!(sent.ends_with("How does it begin?"), "{sent}");
    assert_eq!(ws.texts(cx), ["How does it begin?", "It begins **well**."]);
    assert_eq!(ws.focus(cx), Focus::Panel(PanelArea::Composer));
    ws.panel.read_with(cx, |panel, cx| {
        assert!(panel.composer().read(cx).is_blank(cx));
        assert!(!panel.is_answering());
    });
    // The host was handed the conversation to store: when the question
    // went out, and when the answer ended.
    let stored: Vec<usize> = ws
        .events(cx)
        .iter()
        .filter_map(|event| match event {
            AssistantPanelEvent::TranscriptChanged {
                document,
                transcript,
            } => {
                assert_eq!(*document, Some(1));
                Some(transcript.len())
            }
            _ => None,
        })
        .collect();
    assert_eq!(stored, [2, 2]);
    assert_eq!(ws.stored(1, cx).messages[1].state, MessageState::Done);

    // The text changed: the next message carries it as it is now.
    ws.editor.update(cx, |editor, cx| {
        editor.set_document(
            writer_core::Document::from_blocks(vec![writer_core::Block::paragraph(
                "Chapter 1 now begins differently.",
            )]),
            cx,
        )
    });
    ws.fake.reply("Better.");
    ws.send("And now?", cx);
    let sent = ws.fake.last_sent().unwrap();
    assert!(sent.contains("Chapter 1 now begins differently."), "{sent}");
    // One conversation with the agent for both messages.
    assert_eq!(ws.fake.started().len(), 1);
    // The side chat's own default model and effort.
    assert_eq!(
        ws.fake.last_started().unwrap().selection,
        Selection::named("deep", "medium")
    );
}

#[gpui::test]
fn an_answer_streams_in_and_is_stopped(cx: &mut TestAppContext) {
    let (ws, cx) = open_panel(cx);
    ws.fake.hold();
    ws.fake.push(
        FakeReply::pieces(["First ", "second ", "third."])
            .with_tool("search", "Searching the library"),
    );
    ws.send("Go", cx);
    ws.panel
        .read_with(cx, |panel, _| assert!(panel.is_answering()));
    bounds("assistant-stop", cx);

    // The tool starts: the line under the answer says so.
    ws.advance_until(cx, |transcript| {
        transcript.messages.last().unwrap().current_activity() == Some("Searching the library")
    });
    let answer = *ws.message_ids(cx).last().unwrap();
    bounds(format!("assistant-activity-{}", answer.0), cx);
    ws.advance_to_text("First ", cx);
    ws.panel.read_with(cx, |panel, _| {
        let answer = panel.transcript().messages.last().unwrap();
        assert_eq!(answer.current_activity(), None);
    });

    // The stop key works from the message list too.
    ws.panel.update_in(cx, |panel, window, cx| {
        panel.focus_area(PanelArea::Messages, window, cx)
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("cmd-.");
    cx.run_until_parked();
    assert_eq!(ws.fake.stops(), 1);
    ws.panel.read_with(cx, |panel, _| {
        assert!(!panel.is_answering());
        assert_eq!(
            panel.transcript().messages.last().unwrap().state,
            MessageState::Stopped
        );
    });
    assert_eq!(ws.stored(1, cx).messages[1].state, MessageState::Stopped);
    bounds("assistant-send", cx);

    // And the button.
    ws.fake.push(FakeReply::pieces(["More ", "words."]));
    ws.send("Again", cx);
    ws.advance_to_text("More ", cx);
    click("assistant-stop", cx);
    assert_eq!(ws.fake.stops(), 2);
    ws.panel
        .read_with(cx, |panel, _| assert!(!panel.is_answering()));
}

// ----- errors and the other states ---------------------------------------------------

#[gpui::test]
fn a_failed_answer_is_asked_again(cx: &mut TestAppContext) {
    let (ws, cx) = open_panel(cx);
    ws.fake.fail(AssistantError::UsageLimit {
        resets_at: None,
        detail: "Your limit resets at 6pm.".into(),
    });
    ws.send("Anything?", cx);
    let answer = *ws.message_ids(cx).last().unwrap();
    ws.panel.read_with(cx, |panel, _| {
        assert!(matches!(
            panel.transcript().message(answer).unwrap().state,
            MessageState::Failed(AssistantError::UsageLimit { .. })
        ));
    });
    let notice = bounds(format!("assistant-failed-{}", answer.0), cx);
    assert!(notice.size.height < px(140.), "{notice:?}");
    assert!(notice.size.width <= PANEL_WIDTH);

    // "Try Again" asks the same question again.
    ws.fake.reply("Now it works.");
    click(format!("assistant-failed-{}-action", answer.0), cx);
    let texts = ws.texts(cx);
    assert_eq!(texts.len(), 4);
    assert_eq!(texts[2], "Anything?");
    assert_eq!(texts[3], "Now it works.");
    assert_eq!(ws.fake.sent().len(), 2);

    // The agent's program dies in the middle of an answer: what had
    // arrived stays, and the next message starts the agent again with the
    // conversation so far.
    ws.fake.hold();
    ws.fake.push(FakeReply::pieces(["Half ", "an answer."]));
    ws.send("Long one", cx);
    ws.advance_to_text("Half ", cx);
    ws.fake.crash();
    cx.run_until_parked();
    ws.panel.read_with(cx, |panel, _| {
        let last = panel.transcript().messages.last().unwrap();
        assert!(matches!(
            last.state,
            MessageState::Failed(AssistantError::Exited { .. })
        ));
        assert_eq!(last.text, "Half ");
        assert!(!panel.is_answering());
    });
    ws.fake.release();
    ws.fake.reply("Back again.");
    ws.send("Still there?", cx);
    assert_eq!(ws.fake.started().len(), 2);
    let history = ws.fake.last_started().unwrap().history;
    assert!(
        history
            .iter()
            .any(|message| message.text == "Now it works.")
    );
    assert_eq!(ws.texts(cx).last().unwrap(), "Back again.");
}

#[gpui::test]
fn signed_out_the_panel_says_what_to_run_and_looks_again(cx: &mut TestAppContext) {
    setup(cx);
    let fake = writer_assistant::FakeBackend::signed_out();
    let (ws, cx) = open_panel_on(fake, AssistantPanelOptions::default(), cx);
    let notices = ws.panel.read_with(cx, |panel, _| panel.notices());
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].0, PanelNotice::Availability);
    assert_eq!(notices[0].1.headline, "Sign in with Fake Agent");
    assert_eq!(
        notices[0].1.advice,
        "Open Terminal, run fake-agent and sign in."
    );
    let row = bounds("assistant-notice-availability", cx);
    assert!(row.size.height < px(120.), "{row:?}");
    let checks = ws.fake.availability_checks();

    // enter with a message looks again first. Still signed out: nothing
    // is sent and what was typed is not lost.
    ws.send("Is anyone there?", cx);
    assert_eq!(ws.fake.availability_checks(), checks + 1);
    assert!(ws.fake.sent().is_empty());
    assert!(ws.fake.started().is_empty());
    ws.panel.read_with(cx, |panel, cx| {
        assert_eq!(panel.composer().read(cx).text(cx), "Is anyone there?");
        assert!(panel.transcript().is_empty());
    });

    // The user signs in; "Check Again" finds out.
    ws.fake.set_sign_in(SignIn::SignedIn { account: None });
    click("assistant-notice-availability-action", cx);
    assert_eq!(ws.fake.availability_checks(), checks + 2);
    ws.panel.read_with(cx, |panel, _| {
        assert!(panel.notices().is_empty());
        assert!(panel.availability().unwrap().is_ready());
    });
    assert!(ws.take_events(cx).iter().any(|event| matches!(
        event,
        AssistantPanelEvent::AvailabilityChanged(found) if found.is_ready()
    )));
    // The message is still in the composer: enter sends it.
    ws.fake.reply("Yes.");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(ws.texts(cx), ["Is anyone there?", "Yes."]);

    // Signed out again later, and signed in by the time enter is pressed:
    // the message goes out without another key.
    ws.panel.update(cx, |panel, cx| {
        let mut signed_out = ws.availability();
        if let Availability::Installed(installation) = &mut signed_out {
            installation.sign_in = SignIn::SignedOut;
        }
        panel.set_availability(signed_out, cx)
    });
    cx.run_until_parked();
    bounds("assistant-notice-availability", cx);
    ws.fake.reply("Sent after the check.");
    ws.send("Once more", cx);
    assert_eq!(ws.texts(cx).last().unwrap(), "Sent after the check.");

    // An agent that is gone says how to get it.
    ws.panel.update(cx, |panel, cx| {
        panel.set_availability(Availability::NotInstalled, cx)
    });
    let notices = ws.panel.read_with(cx, |panel, _| panel.notices());
    assert_eq!(
        notices[0].1.headline,
        "Fake Agent is not installed on this Mac"
    );
}

#[gpui::test]
fn the_note_about_where_the_text_goes_is_shown_once(cx: &mut TestAppContext) {
    setup(cx);
    let options = AssistantPanelOptions {
        privacy_notice: true,
        ..AssistantPanelOptions::default()
    };
    let (ws, cx) = open_panel_on(writer_assistant::FakeBackend::new(), options, cx);
    let notices = ws.panel.read_with(cx, |panel, _| panel.notices());
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].0, PanelNotice::Privacy);
    assert!(
        notices[0].1.advice.contains("Fake Vendor"),
        "{:?}",
        notices[0].1
    );
    assert!(
        notices[0]
            .1
            .advice
            .contains("Nothing is sent before you send a message")
    );
    bounds("assistant-notice-privacy", cx);
    // Typing starts nothing, and neither does a host that asks for the
    // agent ahead of time while the note is unread.
    ws.panel
        .update_in(cx, |panel, window, cx| panel.focus(window, cx));
    cx.simulate_input("Hel");
    ws.panel.update(cx, |panel, cx| panel.prepare(cx));
    assert_eq!(ws.fake.prepares(), 0);
    assert!(ws.fake.started().is_empty());

    click("assistant-notice-privacy-action", cx);
    assert_eq!(
        ws.take_events(cx),
        [AssistantPanelEvent::PrivacyNoticeAcknowledged]
    );
    ws.panel
        .read_with(cx, |panel, _| assert!(!panel.shows_privacy_notice()));
    assert_eq!(ws.focus(cx), Focus::Panel(PanelArea::Composer));
    // The panel still starts nothing by itself; a host can have the
    // agent's program started ahead of the first message, once, and
    // nothing is sent to it.
    cx.simulate_input("lo");
    cx.run_until_parked();
    assert_eq!(ws.fake.prepares(), 0);
    assert!(ws.fake.started().is_empty());
    ws.panel.update(cx, |panel, cx| {
        panel.prepare(cx);
        panel.prepare(cx);
    });
    assert_eq!(ws.fake.prepares(), 1);
    assert!(ws.fake.sent().is_empty());
    cx.simulate_input("?");

    // Sending the first message is reading the note too.
    ws.panel
        .update(cx, |panel, cx| panel.set_privacy_notice(true, cx));
    ws.fake.reply("Hi.");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(
        ws.take_events(cx),
        [AssistantPanelEvent::PrivacyNoticeAcknowledged]
    );
    assert_eq!(ws.texts(cx), ["Hello?", "Hi."]);
}

#[gpui::test]
fn what_the_agent_wants_known_is_shown_until_dismissed(cx: &mut TestAppContext) {
    let (ws, cx) = open_panel(cx);
    ws.fake.reply("Yes.");
    ws.send("Hello", cx);
    ws.fake.notice("You have used 90% of your weekly limit.");
    cx.run_until_parked();
    let notices = ws.panel.read_with(cx, |panel, _| panel.notices());
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].0, PanelNotice::Agent);
    assert_eq!(
        notices[0].1.headline,
        "You have used 90% of your weekly limit."
    );
    click("assistant-notice-agent-action", cx);
    ws.panel
        .read_with(cx, |panel, _| assert!(panel.notices().is_empty()));
}

// ----- model and effort ---------------------------------------------------------------

#[gpui::test]
fn the_model_and_the_effort_are_chosen_and_remembered(cx: &mut TestAppContext) {
    setup(cx);
    let options = AssistantPanelOptions {
        selection: Some(Selection::named("swift", "medium")),
        ..AssistantPanelOptions::default()
    };
    let (ws, cx) = open_panel_on(writer_assistant::FakeBackend::new(), options, cx);
    let selection =
        |cx: &mut VisualTestContext| ws.panel.read_with(cx, |panel, _| panel.selection().clone());
    assert_eq!(selection(cx), Selection::named("swift", "medium"));

    // With the mouse: the menu drops from the control; the keyboard stays
    // in the text.
    click("assistant-model", cx);
    assert_eq!(
        menu_labels(cx),
        ["Fake Agent's default", "-", "Swift", "Deep"]
    );
    choose("Deep", cx);
    assert_eq!(selection(cx), Selection::named("deep", "medium"));
    assert_eq!(
        ws.take_events(cx),
        [AssistantPanelEvent::SelectionChanged(Selection::named(
            "deep", "medium"
        ))]
    );
    assert_eq!(ws.focus(cx), Focus::Text);

    // With the keys of the panel.
    ws.panel
        .update_in(cx, |panel, window, cx| panel.focus(window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("cmd-alt-e");
    assert_eq!(
        menu_labels(cx),
        ["Fake Agent's default", "-", "Low", "Medium", "High"]
    );
    cx.simulate_keystrokes("end enter");
    cx.run_until_parked();
    assert_eq!(selection(cx), Selection::named("deep", "high"));
    assert_eq!(ws.focus(cx), Focus::Panel(PanelArea::Composer));

    // A model that does not take the chosen effort puts it back on the
    // agent's default.
    cx.simulate_keystrokes("cmd-shift-m");
    assert!(menu_is_open(cx));
    choose("Swift", cx);
    let now = selection(cx);
    assert_eq!(now.model.id(), Some("swift"));
    assert_eq!(now.effort.id(), None);
    assert_eq!(ws.take_events(cx).len(), 2);

    // The next message is asked with the choice. A later change reaches
    // the agent too: one that cannot change while it runs is started anew
    // and told what was said, one that can is told the choice.
    ws.fake.set_live_selection(false);
    ws.fake.reply("One.");
    ws.send("First", cx);
    assert_eq!(ws.fake.last_started().unwrap().selection, now);
    ws.panel.update(cx, |panel, cx| {
        panel.set_selection(Selection::named("deep", "low"), cx)
    });
    ws.fake.reply("Two.");
    ws.send("Second", cx);
    assert_eq!(ws.fake.started().len(), 2);
    let started = ws.fake.last_started().unwrap();
    assert_eq!(started.selection, Selection::named("deep", "low"));
    assert_eq!(started.history.len(), 2);
    ws.fake.set_live_selection(true);
    ws.panel
        .update_in(cx, |panel, window, cx| panel.new_chat(window, cx));
    ws.fake.reply("Three.");
    ws.send("Third", cx);
    ws.panel.update(cx, |panel, cx| {
        panel.set_selection(Selection::named("deep", "high"), cx)
    });
    ws.fake.reply("Four.");
    ws.send("Fourth", cx);
    assert_eq!(ws.fake.started().len(), 3, "the same agent goes on");
    assert_eq!(
        ws.fake
            .selections()
            .last()
            .map(|(_, selection)| selection.clone()),
        Some(Selection::named("deep", "high"))
    );
    assert_eq!(ws.texts(cx).last().unwrap(), "Four.");

    // A remembered effort the model does not take is put right when the
    // panel is made.
    ws.panel.update(cx, |panel, cx| {
        panel.set_selection(Selection::named("swift", "high"), cx);
        assert_eq!(panel.selection().effort.id(), None);
    });
}

// ----- the context --------------------------------------------------------------------

#[gpui::test]
fn library_items_are_added_to_the_context_and_removed(cx: &mut TestAppContext) {
    let (ws, cx) = open_panel(cx);
    // The open document is always there.
    bounds("assistant-context-document", cx);
    ws.panel.read_with(cx, |panel, cx| {
        assert!(panel.attachments().is_empty());
        let offered: Vec<String> = panel
            .context_candidates(cx)
            .iter()
            .map(|candidate| candidate.title.clone())
            .collect();
        // The open document itself is not offered.
        assert_eq!(
            offered,
            [
                "The Salt Road",
                "Notes",
                "Chapter 2",
                "Characters",
                "Timeline"
            ]
        );
    });

    // With the mouse: "Add" opens the picker under it, with the keyboard
    // in its field.
    click("assistant-add-context", cx);
    let picker = ws
        .panel
        .read_with(cx, |panel, _| panel.context_picker().cloned())
        .expect("the picker is open");
    assert_eq!(ws.focus(cx), Focus::Picker);
    let popup = bounds("assistant-context-picker", cx);
    let add = bounds("assistant-add-context", cx);
    assert!(popup.top() >= add.bottom());
    cx.simulate_input("char");
    picker.read_with(cx, |picker, _| {
        let shown: Vec<&str> = picker
            .delegate
            .shown()
            .iter()
            .map(|candidate| candidate.title.as_str())
            .collect();
        // The best match first ("Chapter 2" has the letters too).
        assert_eq!(shown, ["Characters", "Chapter 2"]);
    });
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    ws.panel.read_with(cx, |panel, _| {
        assert!(panel.context_picker().is_none());
        assert_eq!(
            panel.attachments(),
            [Attachment::document(
                3,
                "Characters",
                "The Salt Road/Notes/Characters"
            )]
        );
    });
    // The keyboard is back in the text, where it was.
    assert_eq!(ws.focus(cx), Focus::Text);
    bounds("assistant-context-doc:3", cx);
    assert_eq!(ws.stored(1, cx).attachments.len(), 1);

    // With the key, from the composer: a folder. escape closes the picker
    // without adding.
    ws.panel
        .update_in(cx, |panel, window, cx| panel.focus(window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("cmd-shift-a");
    assert_eq!(ws.focus(cx), Focus::Picker);
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert_eq!(ws.focus(cx), Focus::Panel(PanelArea::Composer));
    ws.panel
        .read_with(cx, |panel, _| assert_eq!(panel.attachments().len(), 1));
    cx.simulate_keystrokes("cmd-shift-a");
    cx.simulate_input("notes");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(ws.focus(cx), Focus::Panel(PanelArea::Composer));
    bounds("assistant-context-folder:11", cx);
    // What is attached is not offered again.
    ws.panel.read_with(cx, |panel, cx| {
        let offered: Vec<i64> = panel
            .context_candidates(cx)
            .iter()
            .map(|candidate| candidate.id)
            .collect();
        assert_eq!(offered, [10, 2, 4]);
    });

    // The message says what is attached; the assistant reads it with the
    // tools.
    ws.fake.reply("Read.");
    ws.send("Who is Mara?", cx);
    let sent = ws.fake.last_sent().unwrap();
    assert!(sent.contains("doc:3"), "{sent}");
    assert!(sent.contains("folder:11"), "{sent}");
    assert!(sent.contains("Characters"), "{sent}");

    // Removing: the chip's own button.
    click("assistant-context-doc:3-remove", cx);
    ws.panel.read_with(cx, |panel, _| {
        assert_eq!(panel.attachments().len(), 1);
        assert_eq!(panel.attachments()[0].kind, AttachmentKind::Folder);
    });
    assert_eq!(ws.stored(1, cx).attachments.len(), 1);
    // The chip's menu.
    right_click_at(bounds("assistant-context-folder:11", cx).center(), cx);
    assert_eq!(menu_labels(cx), ["Remove from the Context"]);
    choose("Remove from the Context", cx);
    ws.panel
        .read_with(cx, |panel, _| assert!(panel.attachments().is_empty()));

    // Moving on with tab closes an open picker.
    ws.panel
        .update_in(cx, |panel, window, cx| panel.focus(window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("cmd-shift-a");
    assert_eq!(ws.focus(cx), Focus::Picker);
    cx.simulate_keystrokes("tab");
    assert!(
        ws.panel
            .read_with(cx, |panel, _| panel.context_picker().is_none())
    );
    assert_eq!(ws.focus(cx), Focus::Panel(PanelArea::Header));
    ws.editor
        .update_in(cx, |editor, window, _| editor.focus(window));
    cx.run_until_parked();

    // A click outside the open picker closes it.
    click("assistant-add-context", cx);
    assert!(
        ws.panel
            .read_with(cx, |panel, _| panel.context_picker().is_some())
    );
    click_at(bounds("assistant-title", cx).center(), cx);
    assert!(
        ws.panel
            .read_with(cx, |panel, _| panel.context_picker().is_none())
    );

    // The open document cannot be attached to its own conversation.
    ws.panel.update(cx, |panel, cx| {
        assert!(!panel.attach(
            Attachment::document(1, "Chapter 1", "The Salt Road/Chapter 1"),
            cx
        ));
        assert!(panel.attach(
            Attachment::document(4, "Timeline", "The Salt Road/Notes/Timeline"),
            cx
        ));
        assert!(!panel.attach(
            Attachment::document(4, "Timeline", "The Salt Road/Notes/Timeline"),
            cx
        ));
    });
}

#[gpui::test]
fn the_context_row_is_worked_with_the_keyboard(cx: &mut TestAppContext) {
    let (ws, cx) = open_panel(cx);
    ws.panel.update(cx, |panel, cx| {
        panel.attach(Attachment::folder(11, "Notes", "The Salt Road/Notes"), cx);
        panel.attach(
            Attachment::document(3, "Characters", "The Salt Road/Notes/Characters"),
            cx,
        );
    });
    ws.panel.update_in(cx, |panel, window, cx| {
        panel.focus_area(PanelArea::Context, window, cx)
    });
    cx.run_until_parked();
    assert_eq!(ws.focus(cx), Focus::Panel(PanelArea::Context));
    let index =
        |cx: &mut VisualTestContext| ws.panel.read_with(cx, |panel, _| panel.context_index());
    assert_eq!(index(cx), 0);
    cx.simulate_keystrokes("right right right");
    assert_eq!(index(cx), 2, "the last stop is Add");
    cx.simulate_keystrokes("left");
    assert_eq!(index(cx), 1);

    // delete removes the marked chip.
    cx.simulate_keystrokes("delete");
    ws.panel.read_with(cx, |panel, _| {
        assert_eq!(panel.attachments().len(), 1);
        assert_eq!(panel.attachments()[0].id, 11);
    });
    assert_eq!(index(cx), 1, "the mark is on Add now");

    // enter on Add opens the picker; when it closes the keyboard is back
    // in the row.
    cx.simulate_keystrokes("enter");
    assert_eq!(ws.focus(cx), Focus::Picker);
    cx.simulate_input("time");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(ws.focus(cx), Focus::Panel(PanelArea::Context));
    ws.panel
        .read_with(cx, |panel, _| assert_eq!(panel.attachments().len(), 2));

    // enter on a chip presses its one control: remove.
    cx.simulate_keystrokes("left left enter");
    ws.panel.read_with(cx, |panel, _| {
        assert_eq!(panel.attachments().len(), 1);
        assert_eq!(panel.attachments()[0].id, 4);
    });
    cx.simulate_keystrokes("backspace");
    ws.panel
        .read_with(cx, |panel, _| assert!(panel.attachments().is_empty()));
    cx.simulate_keystrokes("escape");
    assert_eq!(
        ws.take_events(cx),
        [AssistantPanelEvent::FocusTextRequested]
    );
}

// ----- a conversation per document --------------------------------------------------

#[gpui::test]
fn every_document_has_its_own_conversation(cx: &mut TestAppContext) {
    let (ws, cx) = open_panel(cx);
    ws.fake.reply("About chapter one.");
    ws.send("What is this?", cx);
    ws.panel.update(cx, |panel, cx| {
        panel.attach(
            Attachment::document(4, "Timeline", "The Salt Road/Notes/Timeline"),
            cx,
        );
    });
    cx.run_until_parked();

    // Another document: an empty conversation, its own context.
    ws.open_chapter(2, cx);
    ws.panel.read_with(cx, |panel, _| {
        assert_eq!(panel.document().unwrap().title, "Chapter 2");
        assert!(panel.transcript().is_empty());
        assert!(panel.attachments().is_empty());
    });
    bounds("assistant-empty", cx);
    ws.fake.reply("About chapter two.");
    ws.send("And this?", cx);
    let sent = ws.fake.last_sent().unwrap();
    assert!(sent.contains("Chapter 2 begins here."), "{sent}");
    assert!(!sent.contains("What is this?"), "{sent}");
    // Its agent knows nothing of the other conversation.
    assert_eq!(ws.fake.started().len(), 2);
    assert!(ws.fake.last_started().unwrap().history.is_empty());
    assert_eq!(ws.fake.alive(), 1, "the first document's agent was let go");

    // Back: the first conversation, as the host stored it.
    ws.open_chapter(1, cx);
    assert_eq!(ws.texts(cx), ["What is this?", "About chapter one."]);
    ws.panel.read_with(cx, |panel, _| {
        assert_eq!(panel.attachments().len(), 1);
    });
    // The next message continues it: the agent is told what was said.
    ws.fake.reply("More about one.");
    ws.send("Go on", cx);
    let started = ws.fake.last_started().unwrap();
    assert_eq!(started.history.len(), 2);
    assert_eq!(started.history[1].text, "About chapter one.");
    assert_eq!(ws.stored(1, cx).len(), 4);
    assert_eq!(ws.stored(2, cx).len(), 2);

    // The title of the open document follows the text.
    ws.panel
        .update(cx, |panel, cx| panel.set_document_title("Chapter One", cx));
    ws.panel.read_with(cx, |panel, _| {
        assert_eq!(panel.document().unwrap().title, "Chapter One");
        assert_eq!(panel.transcript().len(), 4, "the same conversation");
    });
    // Telling the panel the same document again changes nothing.
    ws.panel.update_in(cx, |panel, window, cx| {
        panel.set_document(
            Some(writer_assistant_ui::DocumentRef::new(1, "Chapter 1")),
            Some(writer_assistant::Transcript::new()),
            window,
            cx,
        )
    });
    ws.panel
        .read_with(cx, |panel, _| assert_eq!(panel.transcript().len(), 4));

    // No document at all: the chat still answers, without one.
    ws.panel.update_in(cx, |panel, window, cx| {
        panel.set_document(None, None, window, cx)
    });
    cx.run_until_parked();
    ws.panel.read_with(cx, |panel, _| {
        assert!(panel.document().is_none());
        assert!(panel.transcript().is_empty());
    });
}

#[gpui::test]
fn an_answer_goes_on_while_another_document_is_open(cx: &mut TestAppContext) {
    let (ws, cx) = open_panel(cx);
    ws.fake.hold();
    ws.fake
        .push(FakeReply::pieces(["A long ", "answer ", "about one."]));
    ws.send("Tell me everything", cx);
    ws.advance_to_text("A long ", cx);

    // The user opens another document while it is being written.
    ws.open_chapter(2, cx);
    ws.panel.read_with(cx, |panel, _| {
        assert!(panel.transcript().is_empty());
        assert!(!panel.is_answering());
        assert_eq!(panel.answering_elsewhere(), [Some(1)]);
    });
    assert_eq!(ws.fake.stops(), 0);
    bounds("assistant-send", cx);

    // The answer ends: the host gets the whole conversation of the
    // document that is not open, and its agent is let go.
    ws.fake.finish();
    cx.run_until_parked();
    let stored = ws.stored(1, cx);
    assert_eq!(stored.messages[1].text, "A long answer about one.");
    assert_eq!(stored.messages[1].state, MessageState::Done);
    ws.panel.read_with(cx, |panel, _| {
        assert!(panel.answering_elsewhere().is_empty())
    });
    assert_eq!(ws.fake.alive(), 0);
    ws.panel
        .read_with(cx, |panel, _| assert!(panel.transcript().is_empty()));

    // Coming back while it is still being written shows it as it is.
    ws.open_chapter(1, cx);
    assert_eq!(ws.texts(cx)[1], "A long answer about one.");
    ws.fake.push(FakeReply::pieces(["Second ", "answer."]));
    ws.send("And more", cx);
    ws.advance_to_text("Second ", cx);
    ws.open_chapter(2, cx);
    ws.open_chapter(1, cx);
    ws.panel.read_with(cx, |panel, _| {
        assert!(panel.is_answering());
        assert!(panel.answering_elsewhere().is_empty());
        assert_eq!(panel.transcript().messages.last().unwrap().text, "Second ");
    });
    bounds("assistant-stop", cx);
    ws.fake.finish();
    cx.run_until_parked();
    assert_eq!(ws.texts(cx).last().unwrap(), "Second answer.");
    assert_eq!(ws.stored(1, cx).len(), 4);
}

// ----- new chat and clear --------------------------------------------------------------

#[gpui::test]
fn a_chat_is_cleared_started_anew_and_brought_back(cx: &mut TestAppContext) {
    let (ws, cx) = open_panel(cx);
    ws.fake.reply("One.");
    ws.send("First", cx);
    ws.panel.update(cx, |panel, cx| {
        panel.attach(Attachment::folder(11, "Notes", "The Salt Road/Notes"), cx);
    });

    // Clear: the messages go, the context stays; the agent starts afresh.
    click("assistant-clear", cx);
    ws.panel.read_with(cx, |panel, _| {
        assert!(panel.transcript().is_empty());
        assert_eq!(panel.attachments().len(), 1);
        assert!(panel.can_restore_cleared());
    });
    assert!(ws.stored(1, cx).is_empty());
    assert_eq!(ws.fake.alive(), 0);
    bounds("assistant-empty", cx);
    // A toast offers to take it back.
    let toast = ws.workspace.read_with(cx, |workspace, cx| {
        let toasts = workspace.toasts.read(cx);
        let id = *toasts.ids().last().expect("a toast");
        (id, toasts.toasts().last().unwrap().action_label().cloned())
    });
    assert_eq!(toast.1.as_ref().map(|label| label.as_ref()), Some("Undo"));
    ws.workspace.update_in(cx, |workspace, window, cx| {
        workspace
            .toasts
            .update(cx, |toasts, cx| toasts.run_action(toast.0, window, cx));
    });
    cx.run_until_parked();
    assert_eq!(ws.texts(cx), ["First", "One."]);
    assert_eq!(ws.stored(1, cx).len(), 2);
    ws.panel
        .read_with(cx, |panel, _| assert!(!panel.can_restore_cleared()));

    // New chat: the context goes too.
    ws.panel
        .update_in(cx, |panel, window, cx| panel.focus(window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("cmd-alt-n");
    cx.run_until_parked();
    ws.panel.read_with(cx, |panel, _| {
        assert!(panel.transcript().is_empty());
        assert!(panel.attachments().is_empty());
    });
    // Once something was said in the new one, the old one is gone.
    ws.fake.reply("Fresh.");
    ws.send("Start", cx);
    ws.panel.update_in(cx, |panel, window, cx| {
        assert!(!panel.can_restore_cleared());
        assert!(!panel.restore_cleared(window, cx));
    });
    assert!(ws.fake.last_started().unwrap().history.is_empty());

    // The key for clear; with nothing to clear nothing happens.
    cx.simulate_keystrokes("cmd-alt-k");
    cx.run_until_parked();
    assert!(ws.texts(cx).is_empty());
    let toasts = ws
        .workspace
        .read_with(cx, |workspace, cx| workspace.toasts.read(cx).ids().len());
    cx.simulate_keystrokes("cmd-alt-k cmd-alt-n");
    cx.run_until_parked();
    assert_eq!(
        ws.workspace
            .read_with(cx, |workspace, cx| workspace.toasts.read(cx).ids().len()),
        toasts
    );

    // Clearing while an answer is being written stops it.
    ws.fake.hold();
    ws.fake.push(FakeReply::pieces(["Half ", "done."]));
    ws.send("Slow", cx);
    ws.advance_to_text("Half ", cx);
    ws.panel
        .update_in(cx, |panel, window, cx| panel.clear(window, cx));
    cx.run_until_parked();
    ws.panel.read_with(cx, |panel, _| {
        assert!(panel.transcript().is_empty());
        assert!(!panel.is_answering());
    });
    ws.fake.finish();
    cx.run_until_parked();
    assert!(ws.texts(cx).is_empty(), "late events change nothing");
}

// ----- what reaches the host -----------------------------------------------------------

#[gpui::test]
fn the_actions_of_an_answer_reach_the_host(cx: &mut TestAppContext) {
    let (ws, cx) = open_panel(cx);
    ws.fake.reply("First *answer*.");
    ws.send("One", cx);
    ws.fake.reply("Second answer.");
    ws.send("Two", cx);
    let ids = ws.message_ids(cx);
    let (first, second) = (ids[1], ids[3]);
    ws.take_events(cx);
    let apply = |kind, message, markdown: &str| AssistantPanelEvent::Apply {
        kind,
        message,
        markdown: markdown.to_string(),
    };

    // The buttons under the latest answer. Replace needs a selection.
    click(format!("assistant-action-insert-{}", second.0), cx);
    click(format!("assistant-action-replace-{}", second.0), cx);
    click(format!("assistant-action-copy-{}", second.0), cx);
    assert_eq!(
        ws.take_events(cx),
        [
            apply(ApplyKind::InsertAtCaret, second, "Second answer."),
            apply(ApplyKind::Copy, second, "Second answer."),
        ]
    );
    assert_eq!(ws.focus(cx), Focus::Panel(PanelArea::Composer));

    ws.panel
        .update(cx, |panel, cx| panel.set_apply_state(true, true, cx));
    cx.run_until_parked();
    click(format!("assistant-action-replace-{}", second.0), cx);
    assert_eq!(
        ws.take_events(cx),
        [apply(ApplyKind::ReplaceSelection, second, "Second answer.")]
    );

    // The keys, from the composer: the latest answer.
    cx.simulate_keystrokes("cmd-enter cmd-shift-enter cmd-shift-c");
    assert_eq!(
        ws.take_events(cx),
        [
            apply(ApplyKind::InsertAtCaret, second, "Second answer."),
            apply(ApplyKind::ReplaceSelection, second, "Second answer."),
            apply(ApplyKind::Copy, second, "Second answer."),
        ]
    );
    // In the composer cmd-enter is not the editor's page break.
    ws.panel.read_with(cx, |panel, cx| {
        assert!(panel.composer().read(cx).is_blank(cx));
    });

    // From the message list: the marked answer. The answer keeps its
    // Markdown.
    ws.panel.update_in(cx, |panel, window, cx| {
        panel.focus_area(PanelArea::Messages, window, cx)
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("up up cmd-enter");
    assert_eq!(
        ws.take_events(cx),
        [apply(ApplyKind::InsertAtCaret, first, "First *answer*.")]
    );
    // A marked message of the user's: its text is copied, nothing is
    // inserted.
    cx.simulate_keystrokes("up cmd-enter cmd-c");
    assert_eq!(
        ws.take_events(cx),
        [AssistantPanelEvent::CopyText("One".into())]
    );
    cx.simulate_keystrokes("down cmd-c");
    assert_eq!(
        ws.take_events(cx),
        [apply(ApplyKind::Copy, first, "First *answer*.")]
    );

    // The menu of an answer.
    let row = bounds(format!("assistant-message-{}", first.0), cx);
    right_click_at(point(row.right() - px(8.), row.bottom() - px(8.)), cx);
    assert_eq!(
        menu_labels(cx),
        ["Insert at Caret", "Replace Selection", "Copy Answer"]
    );
    choose("Replace Selection", cx);
    assert_eq!(
        ws.take_events(cx),
        [apply(ApplyKind::ReplaceSelection, first, "First *answer*.")]
    );

    // With no caret to insert at (no document that can be changed).
    ws.panel
        .update(cx, |panel, cx| panel.set_apply_state(false, false, cx));
    cx.run_until_parked();
    ws.panel
        .update_in(cx, |panel, window, cx| panel.focus(window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("cmd-enter cmd-shift-enter");
    click(format!("assistant-action-insert-{}", second.0), cx);
    assert!(ws.take_events(cx).is_empty());

    // An answer that is still being written cannot be applied.
    ws.fake.hold();
    ws.fake.push(FakeReply::pieces(["Not ", "yet."]));
    ws.send("Three", cx);
    ws.advance_to_text("Not ", cx);
    cx.simulate_keystrokes("cmd-shift-c");
    assert!(ws.take_events(cx).is_empty());
    ws.fake.release();
    cx.run_until_parked();
    cx.simulate_keystrokes("cmd-shift-c");
    assert_eq!(ws.take_events(cx).len(), 1);
}

#[gpui::test]
fn the_commands_reach_the_panel_from_wherever_the_keyboard_is(cx: &mut TestAppContext) {
    let (ws, cx) = open_panel(cx);
    ws.fake.reply("An answer.");
    ws.send("A question", cx);
    let answer = *ws.message_ids(cx).last().unwrap();
    ws.editor
        .update_in(cx, |editor, window, _| editor.focus(window));
    cx.run_until_parked();
    assert_eq!(ws.focus(cx), Focus::Text);
    ws.take_events(cx);

    // As the command palette runs them: dispatched where the keyboard is.
    cx.dispatch_action(InsertAtCaret);
    cx.run_until_parked();
    assert_eq!(
        ws.take_events(cx),
        [AssistantPanelEvent::Apply {
            kind: ApplyKind::InsertAtCaret,
            message: answer,
            markdown: "An answer.".into()
        }]
    );
    cx.dispatch_action(AddContext);
    cx.run_until_parked();
    assert_eq!(ws.focus(cx), Focus::Picker);
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert_eq!(ws.focus(cx), Focus::Text);

    // A panel that is not on screen is left alone.
    ws.workspace.update(cx, |workspace, cx| {
        workspace.panel_open = false;
        workspace
            .panel
            .update(cx, |panel, cx| panel.set_shown(false, cx));
        cx.notify();
    });
    cx.run_until_parked();
    cx.dispatch_action(ClearChat);
    cx.dispatch_action(NewChat);
    cx.run_until_parked();
    assert_eq!(ws.texts(cx).len(), 2);
    ws.workspace.update(cx, |workspace, cx| {
        workspace.panel_open = true;
        workspace
            .panel
            .update(cx, |panel, cx| panel.set_shown(true, cx));
        cx.notify();
    });
    cx.run_until_parked();
    cx.dispatch_action(ClearChat);
    cx.run_until_parked();
    assert!(ws.texts(cx).is_empty());
    assert_eq!(writer_assistant_ui::commands().len(), 9);
}

// ----- the keyboard ---------------------------------------------------------------------

#[gpui::test]
fn the_keyboard_walks_the_panel(cx: &mut TestAppContext) {
    let (ws, cx) = open_panel(cx);
    ws.fake.reply("An answer.");
    ws.send("A question", cx);
    assert_eq!(ws.focus(cx), Focus::Panel(PanelArea::Composer));

    // tab: header, context, messages, composer, and round again.
    let mut seen = Vec::new();
    for _ in 0..5 {
        cx.simulate_keystrokes("tab");
        seen.push(ws.focus(cx));
    }
    assert_eq!(
        seen,
        [
            Focus::Panel(PanelArea::Header),
            Focus::Panel(PanelArea::Context),
            Focus::Panel(PanelArea::Messages),
            Focus::Panel(PanelArea::Composer),
            Focus::Panel(PanelArea::Header),
        ]
    );
    cx.simulate_keystrokes("shift-tab");
    assert_eq!(ws.focus(cx), Focus::Panel(PanelArea::Composer));
    cx.simulate_keystrokes("shift-tab");
    assert_eq!(ws.focus(cx), Focus::Panel(PanelArea::Messages));
    // tab did not reach the text as a tab or an indent.
    ws.panel.read_with(cx, |panel, cx| {
        assert!(panel.composer().read(cx).is_blank(cx));
    });

    // The header: the arrows move over its controls, enter presses.
    cx.simulate_keystrokes("shift-tab shift-tab");
    assert_eq!(ws.focus(cx), Focus::Panel(PanelArea::Header));
    let control =
        |cx: &mut VisualTestContext| ws.panel.read_with(cx, |panel, _| panel.header_control());
    assert_eq!(control(cx), HeaderControl::Model);
    cx.simulate_keystrokes("left");
    assert_eq!(control(cx), HeaderControl::Model);
    cx.simulate_keystrokes("enter");
    assert_eq!(menu_labels(cx).len(), 4);
    cx.simulate_keystrokes("escape");
    assert!(!menu_is_open(cx));
    assert_eq!(ws.focus(cx), Focus::Panel(PanelArea::Header));
    cx.simulate_keystrokes("right");
    assert_eq!(control(cx), HeaderControl::Effort);
    cx.simulate_keystrokes("space");
    assert_eq!(menu_labels(cx).len(), 5);
    cx.simulate_keystrokes("escape");
    cx.simulate_keystrokes("right right");
    assert_eq!(control(cx), HeaderControl::Clear);
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(ws.texts(cx).is_empty());
    // Clear has nothing to do any more: the mark is not left on it.
    assert_eq!(control(cx), HeaderControl::Model);
    // With an empty chat the arrows step over what cannot be pressed.
    cx.simulate_keystrokes("right right");
    assert_eq!(control(cx), HeaderControl::Close);
    cx.simulate_keystrokes("right");
    assert_eq!(control(cx), HeaderControl::Close);
    ws.take_events(cx);
    cx.simulate_keystrokes("enter");
    assert_eq!(ws.take_events(cx), [AssistantPanelEvent::CloseRequested]);

    // escape hands the keyboard back from every part.
    ws.workspace.update(cx, |workspace, cx| {
        workspace.panel_open = true;
        cx.notify();
    });
    cx.run_until_parked();
    for area in [
        PanelArea::Header,
        PanelArea::Context,
        PanelArea::Messages,
        PanelArea::Composer,
    ] {
        ws.panel
            .update_in(cx, |panel, window, cx| panel.focus_area(area, window, cx));
        cx.run_until_parked();
        assert_eq!(ws.focus(cx), Focus::Panel(area));
        cx.simulate_keystrokes("escape");
        assert_eq!(
            ws.take_events(cx),
            [AssistantPanelEvent::FocusTextRequested],
            "{area:?}"
        );
    }

    // In an empty conversation the list offers the example prompts: enter
    // puts the marked one into the composer, ready to be sent or changed.
    ws.panel.update_in(cx, |panel, window, cx| {
        panel.focus_area(PanelArea::Messages, window, cx)
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("down enter");
    cx.run_until_parked();
    assert_eq!(ws.focus(cx), Focus::Panel(PanelArea::Composer));
    ws.panel.read_with(cx, |panel, cx| {
        assert_eq!(
            panel.composer().read(cx).text(cx),
            "Which sentences are harder to read than they need to be?"
        );
    });
    assert!(
        ws.fake.sent().len() == 1,
        "an example is not sent by itself"
    );
}

// ----- the mouse and the layout -----------------------------------------------------------

#[gpui::test]
fn the_header_and_the_panel_menu_work_with_the_mouse(cx: &mut TestAppContext) {
    let (ws, cx) = open_panel(cx);
    // Nothing to clear yet: the two buttons do nothing.
    click("assistant-clear", cx);
    click("assistant-new-chat", cx);
    assert!(ws.take_events(cx).is_empty());
    assert_eq!(
        ws.focus(cx),
        Focus::Text,
        "a press on a control does not move the keyboard"
    );

    ws.fake.reply("An answer.");
    ws.send("A question", cx);
    ws.editor
        .update_in(cx, |editor, window, _| editor.focus(window));
    cx.run_until_parked();

    // The menu of the panel: a right-click on the header.
    right_click_at(bounds("assistant-title", cx).center(), cx);
    assert_eq!(
        menu_labels(cx),
        ["New Chat", "Clear Chat", "-", "Add Context…", "-", "Close"]
    );
    choose("Clear Chat", cx);
    assert!(ws.texts(cx).is_empty());
    right_click_at(bounds("assistant-title", cx).center(), cx);
    cx.update(|window, cx| {
        let menu = writer_ui::ContextMenu::current(window, cx).unwrap();
        let disabled: Vec<bool> = menu
            .read(cx)
            .items()
            .iter()
            .map(|item| item.is_disabled())
            .collect();
        assert_eq!(disabled, [true, true, false, false, false, false]);
    });
    choose("Add Context…", cx);
    assert_eq!(ws.focus(cx), Focus::Picker);
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert_eq!(ws.focus(cx), Focus::Text);

    // Tooltips say what a control is and its key.
    move_to(bounds("assistant-close", cx).center(), cx);
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(900));
    cx.run_until_parked();
    bounds("tooltip", cx);

    ws.take_events(cx);
    click("assistant-close", cx);
    assert_eq!(ws.take_events(cx), [AssistantPanelEvent::CloseRequested]);
}

#[gpui::test]
fn everything_stays_inside_the_panel(cx: &mut TestAppContext) {
    let (ws, cx) = open_panel(cx);
    ws.panel.update(cx, |panel, cx| {
        for (id, title) in [(3, "Characters and their histories"), (4, "Timeline")] {
            panel.attach(Attachment::document(id, title, "The Salt Road/Notes"), cx);
        }
        panel.attach(Attachment::folder(11, "Notes", "The Salt Road/Notes"), cx);
        panel.set_privacy_notice(true, cx);
    });
    ws.fake.reply(
        "# A heading\n\nA paragraph of an answer that is long enough to wrap in a column of \
         this width more than once, so that its height is that of several lines.\n\n- one\n- two",
    );
    ws.panel.update_in(cx, |panel, window, cx| {
        panel.set_privacy_notice(false, cx);
        panel.send(
            "A question that is itself long enough to wrap in the box it is shown in, twice",
            window,
            cx,
        );
    });
    cx.run_until_parked();
    ws.panel
        .update(cx, |panel, cx| panel.set_privacy_notice(true, cx));
    cx.run_until_parked();

    for width in [PANEL_WIDTH, px(300.)] {
        ws.workspace.update(cx, |workspace, cx| {
            workspace.panel_width = width;
            cx.notify();
        });
        cx.run_until_parked();
        let panel = bounds("assistant-panel", cx);
        assert_eq!(panel.size.width, width);
        let ids = ws.message_ids(cx);
        let mut inside = vec![
            "assistant-header".to_string(),
            "assistant-title".to_string(),
            "assistant-model".to_string(),
            "assistant-effort".to_string(),
            "assistant-new-chat".to_string(),
            "assistant-clear".to_string(),
            "assistant-close".to_string(),
            "assistant-context".to_string(),
            "assistant-context-document".to_string(),
            "assistant-context-doc:3".to_string(),
            "assistant-context-doc:4".to_string(),
            "assistant-context-folder:11".to_string(),
            "assistant-add-context".to_string(),
            "assistant-messages".to_string(),
            "assistant-notice-privacy".to_string(),
            "assistant-composer".to_string(),
            "assistant-send".to_string(),
        ];
        inside.extend(ids.iter().map(|id| format!("assistant-message-{}", id.0)));
        for name in &inside {
            let part = bounds(name.clone(), cx);
            assert!(
                part.left() >= panel.left() && part.right() <= panel.right(),
                "{name} at {width:?}: {part:?} in {panel:?}"
            );
            assert!(
                part.top() >= panel.top() && part.bottom() <= panel.bottom(),
                "{name} at {width:?}: {part:?} in {panel:?}"
            );
        }
        // Nothing is measured at no width (text in a growing flex column
        // comes out thousands of pixels tall).
        let question = bounds(format!("assistant-message-{}", ids[0].0), cx);
        assert!(question.size.height < px(120.), "{question:?}");
        let answer = bounds(format!("assistant-message-{}", ids[1].0), cx);
        assert!(
            answer.size.height > px(100.) && answer.size.height < px(420.),
            "{answer:?}"
        );
        let notice = bounds("assistant-notice-privacy", cx);
        assert!(notice.size.height < px(320.), "{notice:?}");
        // The parts are stacked: header, context, messages, notice, composer.
        let order = [
            "assistant-header",
            "assistant-context",
            "assistant-messages",
            "assistant-notice-privacy",
            "assistant-composer",
        ];
        for pair in order.windows(2) {
            let (above, below) = (bounds(pair[0], cx), bounds(pair[1], cx));
            assert!(above.bottom() <= below.top(), "{pair:?} at {width:?}");
        }
        // The title is not squeezed away by the controls.
        if width == PANEL_WIDTH {
            assert!(bounds("assistant-title", cx).size.width > px(60.));
        }
    }
}
