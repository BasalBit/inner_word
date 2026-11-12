//! The states of the brief: not signed in, the limit reached, a failure,
//! each with what to do; the note about where the text goes; starting the
//! agent ahead of the first message.

use gpui::TestAppContext;
use writer_assistant::{
    AssistantError, Availability, Backend, FakeBackend, FakeReply, MessageState, SignIn,
    quick_prompt,
};
use writer_assistant_quick::{QuickChatEvent, QuickNotice};
use writer_core::TextRange;

use crate::support::*;

fn notices(fx: &Fixture, cx: &mut gpui::VisualTestContext) -> Vec<(QuickNotice, String)> {
    fx.quick.read_with(cx, |quick, _| {
        quick
            .notices()
            .into_iter()
            .map(|(kind, notice)| (kind, notice.line()))
            .collect()
    })
}

fn signed_in() -> SignIn {
    SignIn::SignedIn {
        account: Some("Fake plan".into()),
    }
}

#[gpui::test]
fn not_signed_in_says_what_to_do_and_looks_again(cx: &mut TestAppContext) {
    setup_all(cx);
    let fake = FakeBackend::signed_out();
    // The app has looked at launch.
    assert!(fake.availability().is_signed_out());
    let (fx, cx) = open_quick_on(fake, paragraphs(&["The quick brown fox"]), cx);
    select(&fx.editor, TextRange::in_block(0, 4, 9), cx);
    fx.toggle(cx);

    assert_eq!(
        notices(&fx, cx),
        vec![(
            QuickNotice::Availability,
            "Sign in with Fake Agent Open Terminal, run fake-agent and sign in.".to_string()
        )]
    );
    let popover = fx.popover(cx);
    let row = bounds("quick-chat-availability", cx);
    assert!(popover.contains(&row.center()));

    // A question typed anyway is not sent and not lost: the popover looks
    // again, and with nobody signed in the question is back in the field.
    let checks = fx.fake.availability_checks();
    cx.simulate_input("Another word?");
    assert_eq!(fx.fake.prepares(), 0, "nothing is started while signed out");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(fx.fake.availability_checks(), checks + 1);
    assert!(fx.fake.sent().is_empty());
    assert!(fx.messages(cx).is_empty());
    assert_eq!(fx.draft(cx), "Another word?");
    assert!(fx.is_open(cx));

    // The user signs in (in the terminal) and presses "Check Again".
    fx.fake.set_sign_in(signed_in());
    click("quick-chat-availability-action", cx);
    assert!(notices(&fx, cx).is_empty());
    assert!(matches!(
        fx.events(cx).last(),
        Some(QuickChatEvent::AvailabilityChanged(found)) if found.is_ready()
    ));

    fx.fake.reply("swift");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(
        fx.fake.sent_texts(),
        vec![quick_prompt("Another word?", Some("quick"))]
    );
    assert_eq!(fx.answers(cx), vec!["swift"]);
}

#[gpui::test]
fn a_question_sent_after_signing_in_goes_out_without_the_button(cx: &mut TestAppContext) {
    setup_all(cx);
    let fake = FakeBackend::signed_out();
    fake.availability();
    let (fx, cx) = open_quick_on(fake, paragraphs(&["The quick brown fox"]), cx);
    fx.toggle(cx);
    assert_eq!(notices(&fx, cx).len(), 1);

    // Signed in meanwhile: sending looks again and sends.
    fx.fake.set_sign_in(signed_in());
    fx.fake.reply("Yes.");
    fx.send("Is anyone there?", cx);
    assert_eq!(fx.fake.sent_texts(), vec!["Is anyone there?"]);
    assert_eq!(fx.answers(cx), vec!["Yes."]);
    assert!(notices(&fx, cx).is_empty());
    assert_eq!(fx.draft(cx), "");
}

#[gpui::test]
fn what_the_app_knows_about_the_agent_is_shown(cx: &mut TestAppContext) {
    let (fx, cx) = open_quick(paragraphs(&["The quick brown fox"]), cx);
    fx.toggle(cx);
    assert!(notices(&fx, cx).is_empty());

    fx.quick.update(cx, |quick, cx| {
        quick.set_availability(Availability::NotInstalled, cx)
    });
    cx.run_until_parked();
    assert_eq!(
        notices(&fx, cx),
        vec![(
            QuickNotice::Availability,
            "Fake Agent is not installed on this Mac Install Fake Agent, then open the settings again."
                .to_string()
        )]
    );
}

#[gpui::test]
fn the_usage_limit_is_said_in_plain_words(cx: &mut TestAppContext) {
    let (fx, cx) = open_quick(paragraphs(&["The quick brown fox"]), cx);
    select(&fx.editor, TextRange::in_block(0, 4, 9), cx);
    fx.toggle(cx);
    let error = AssistantError::UsageLimit {
        resets_at: None,
        detail: "You have hit your limit; it resets at 5pm.".into(),
    };
    fx.fake.fail(error.clone());
    fx.send("Another word?", cx);

    let messages = fx.messages(cx);
    assert_eq!(messages[1].state, MessageState::Failed(error.clone()));
    // The engine's words for it, in the popover, with what to do.
    let words = error.text(&fx.fake.info());
    assert_eq!(
        words.headline,
        "You have reached the usage limit of your Fake Agent plan."
    );
    assert!(words.advice.contains("resets at 5pm"));
    let row = bounds(format!("assistant-failed-{}", messages[1].id.0), cx);
    assert!(fx.popover(cx).contains(&row.center()));
    // The popover is still usable: the question can be asked again.
    assert!(fx.is_open(cx));
    assert!(fx.composer_focused(cx));
}

#[gpui::test]
fn a_failed_answer_can_be_tried_again_with_its_selection(cx: &mut TestAppContext) {
    let (fx, cx) = open_quick(paragraphs(&["The quick brown fox"]), cx);
    select(&fx.editor, TextRange::in_block(0, 4, 9), cx);
    fx.toggle(cx);
    fx.fake.fail(AssistantError::Network {
        detail: "could not resolve host".into(),
    });
    fx.send("Another word?", cx);
    let failed = fx.last_answer_id(cx);
    assert!(matches!(
        fx.messages(cx)[1].state,
        MessageState::Failed(AssistantError::Network { .. })
    ));

    // "Try Again" under the failed answer.
    fx.fake.reply("swift");
    click(format!("assistant-failed-{}-action", failed.0), cx);
    // The same question, with the selection again: the first one may
    // never have arrived.
    let prompt = quick_prompt("Another word?", Some("quick"));
    assert_eq!(fx.fake.sent_texts(), vec![prompt.clone(), prompt]);
    assert_eq!(fx.answers(cx), vec!["", "swift"]);

    // Once an answer came, later questions go alone.
    fx.send("And another?", cx);
    assert_eq!(fx.fake.last_sent().unwrap(), "And another?");
}

#[gpui::test]
fn an_expired_sign_in_shows_as_a_failed_answer(cx: &mut TestAppContext) {
    let (fx, cx) = open_quick(paragraphs(&["The quick brown fox"]), cx);
    fx.toggle(cx);
    fx.fake.fail(AssistantError::NotSignedIn);
    fx.send("Anyone?", cx);
    let messages = fx.messages(cx);
    assert_eq!(
        messages[1].state,
        MessageState::Failed(AssistantError::NotSignedIn)
    );
    let words = AssistantError::NotSignedIn.text(&fx.fake.info());
    assert_eq!(words.headline, "You are not signed in to Fake Agent.");
    assert!(words.advice.contains("run fake-agent and sign in"));
    assert!(try_bounds(format!("assistant-failed-{}", messages[1].id.0), cx).is_some());
}

#[gpui::test]
fn the_agent_dying_fails_the_answer_and_the_next_question_continues(cx: &mut TestAppContext) {
    let (fx, cx) = open_quick(paragraphs(&["The quick brown fox"]), cx);
    select(&fx.editor, TextRange::in_block(0, 4, 9), cx);
    fx.toggle(cx);
    fx.fake.reply("swift");
    fx.send("Another word?", cx);

    fx.fake.hold();
    fx.fake.push(FakeReply::pieces(["fle", "et"]));
    fx.send("One more.", cx);
    fx.first_piece(cx);
    fx.fake.crash();
    cx.run_until_parked();
    let messages = fx.messages(cx);
    assert!(matches!(
        messages[3].state,
        MessageState::Failed(AssistantError::Exited { .. })
    ));
    assert_eq!(messages[3].text, "fle", "what had arrived stays");
    assert!(fx.is_open(cx));
    assert!(!fx.quick.read_with(cx, |quick, _| quick.is_answering()));

    // The next question starts the agent again and tells it what was said.
    fx.fake.release();
    fx.fake.reply("nimble");
    fx.send("Again, please.", cx);
    let started = fx.fake.started();
    assert_eq!(started.len(), 2);
    assert_eq!(
        started[1].history[0].text,
        quick_prompt("Another word?", Some("quick"))
    );
    assert_eq!(started[1].history[1].text, "swift");
    assert_eq!(fx.answers(cx).last().unwrap(), "nimble");
}

#[gpui::test]
fn the_note_about_where_the_text_goes_is_shown_until_it_is_read(cx: &mut TestAppContext) {
    let (fx, cx) = open_quick(paragraphs(&["The quick brown fox"]), cx);
    fx.quick
        .update(cx, |quick, cx| quick.set_privacy_notice(true, cx));
    select(&fx.editor, TextRange::in_block(0, 4, 9), cx);
    fx.toggle(cx);

    let shown = notices(&fx, cx);
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].0, QuickNotice::Privacy);
    assert_eq!(
        shown[0].1,
        "Where your text goes Your question and the text you have selected are sent to Fake \
         Vendor through your Fake Agent sign-in, and each message counts against your Fake Agent \
         plan. Nothing is sent before you send a message."
    );
    let row = bounds("quick-chat-privacy", cx);
    assert!(fx.popover(cx).contains(&row.center()));

    // Nothing is started before it was read, not even ahead of a message.
    cx.simulate_input("Another");
    cx.run_until_parked();
    assert_eq!(fx.fake.prepares(), 0);
    assert!(fx.fake.started().is_empty());

    click("quick-chat-privacy-action", cx);
    assert!(notices(&fx, cx).is_empty());
    assert_eq!(
        fx.events(cx).last(),
        Some(&QuickChatEvent::PrivacyNoticeAcknowledged)
    );
    assert!(fx.is_open(cx));
    assert!(fx.composer_focused(cx));
    assert_eq!(fx.draft(cx), "Another", "the question is still there");
    assert!(fx.fake.sent().is_empty(), "OK sends nothing");

    // It does not come back with the next popover.
    fx.toggle(cx);
    fx.toggle(cx);
    assert!(notices(&fx, cx).is_empty());
}

#[gpui::test]
fn sending_the_first_message_acknowledges_the_note(cx: &mut TestAppContext) {
    let (fx, cx) = open_quick(paragraphs(&["The quick brown fox"]), cx);
    fx.quick
        .update(cx, |quick, cx| quick.set_privacy_notice(true, cx));
    fx.toggle(cx);
    assert_eq!(notices(&fx, cx).len(), 1);

    fx.fake.reply("Yes.");
    fx.send("Anyone?", cx);
    assert!(notices(&fx, cx).is_empty());
    assert!(
        fx.events(cx)
            .contains(&QuickChatEvent::PrivacyNoticeAcknowledged)
    );
    assert_eq!(fx.fake.sent_texts(), vec!["Anyone?"]);
}

#[gpui::test]
fn the_agent_is_started_with_the_first_character_and_sent_nothing(cx: &mut TestAppContext) {
    let (fx, cx) = open_quick(paragraphs(&["The quick brown fox"]), cx);
    select(&fx.editor, TextRange::in_block(0, 4, 9), cx);
    fx.toggle(cx);
    assert_eq!(fx.fake.prepares(), 0, "opening starts nothing");
    assert!(fx.fake.started().is_empty());

    cx.simulate_input("A");
    cx.run_until_parked();
    assert_eq!(fx.fake.prepares(), 1);
    assert_eq!(fx.fake.started().len(), 1);
    assert!(fx.fake.sent().is_empty(), "nothing of the text has left");

    cx.simulate_input("nother word?");
    cx.run_until_parked();
    assert_eq!(fx.fake.prepares(), 1, "once");

    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    // The message goes to the conversation that was started ahead.
    assert_eq!(fx.fake.started().len(), 1);
    assert_eq!(fx.fake.sent().len(), 1);
    assert_eq!(fx.fake.sent()[0].conversation, 0);
}

#[gpui::test]
fn a_notice_of_the_agent_is_shown_and_can_be_dismissed(cx: &mut TestAppContext) {
    let (fx, cx) = open_quick(paragraphs(&["The quick brown fox"]), cx);
    fx.toggle(cx);
    fx.fake.reply("Yes.");
    fx.send("Anyone?", cx);
    fx.fake.notice("You have used 90% of your weekly limit.");
    cx.run_until_parked();
    assert_eq!(
        notices(&fx, cx),
        vec![(
            QuickNotice::Agent,
            "You have used 90% of your weekly limit.".to_string()
        )]
    );
    click("quick-chat-agent-notice-action", cx);
    assert!(notices(&fx, cx).is_empty());
    assert!(fx.is_open(cx));
}

#[gpui::test]
fn a_compact_editor_has_no_quick_chat(cx: &mut TestAppContext) {
    let (fx, cx) = open_quick(paragraphs(&["The quick brown fox"]), cx);
    fx.editor.update(cx, |editor, cx| {
        editor.set_config(
            writer_editor::EditorConfig::compact(writer_editor::CompactOptions::default()),
            cx,
        )
    });
    let opened = fx
        .quick
        .update_in(cx, |quick, window, cx| quick.open(window, cx));
    assert!(!opened);
    assert!(!fx.is_open(cx));
}
