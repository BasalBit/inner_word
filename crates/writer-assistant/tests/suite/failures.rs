//! What goes wrong, and that each of it ends the turn with an error the interface can
//! act on, without hanging and without losing the conversation.

use std::time::{Duration, Instant};

use serde_json::json;
use writer_assistant::claude::MINIMUM_VERSION;
use writer_assistant::{
    AssistantError, Backend, ClaudeCode, ConversationOptions, Selection, TurnEnd,
};

use super::support::{Answer, PATIENCE, Stage, Talk, fixture};

fn options() -> ConversationOptions {
    ConversationOptions::new("You help with writing.")
        .with_selection(Selection::named("haiku", "low"))
}

/// Replays one recording as the only turn and returns how the turn ended.
fn replayed(name: &str) -> (Stage, Talk, Answer) {
    let stage = Stage::new(json!({"turns": [
        {"replay": fixture(name)},
        {"text": "Fine again.", "pieces": 1},
    ]}));
    let talk = Talk::start(&stage.backend(), options());
    let answer = talk.ask("A word for very tired?");
    (stage, talk, answer)
}

#[test]
fn a_signed_out_program_says_so() {
    // Recorded from the real program with an empty configuration directory.
    let (_stage, talk, answer) = replayed("signed-out-turn.jsonl");
    assert_eq!(answer.error(), &AssistantError::NotSignedIn);
    // The program's sentence ("Not logged in · Please run /login") is not shown as an
    // answer.
    assert_eq!(answer.text, "");
    assert_eq!(answer.final_text, None);
    // The process is still there: after signing in the next message is answered.
    assert!(talk.conversation.is_alive());
    assert_eq!(talk.ask("And now?").text, "Fine again.");
}

#[test]
fn an_expired_sign_in_is_not_signed_in() {
    let (_stage, _talk, answer) = replayed("error-sign-in-expired.jsonl");
    assert_eq!(answer.error(), &AssistantError::NotSignedIn);
    // The program tried again first; the line under the answer said so.
    assert!(
        answer
            .statuses()
            .iter()
            .any(|status| status.starts_with("Signing in, trying again"))
    );
}

#[test]
fn the_usage_limit_comes_with_the_time_it_resets() {
    // Recorded from the real program against a stand-in for the service that answers
    // as the service does when a subscriber's five-hour allowance is used up.
    let (_stage, talk, answer) = replayed("error-usage-limit.jsonl");
    let AssistantError::UsageLimit { resets_at, detail } = answer.error() else {
        panic!("{:?}", answer.end);
    };
    assert_eq!(
        *resets_at,
        Some(1794501407),
        "the reset time of the rate limit event"
    );
    assert!(
        detail.starts_with("You've hit your 5-hour limit"),
        "{detail}"
    );
    assert_eq!(answer.text, "");
    // In words: what happened and when it is over.
    let text = answer.error().text(&ClaudeCode::new().info());
    assert_eq!(
        text.headline,
        "You have reached the usage limit of your Claude Code plan."
    );
    assert!(text.advice.starts_with("It resets "), "{}", text.advice);
    assert!(!answer.error().may_retry());
    assert!(talk.conversation.is_alive());
}

#[test]
fn an_overloaded_service_is_unavailable() {
    let (_stage, _talk, answer) = replayed("error-overloaded.jsonl");
    assert!(
        matches!(answer.error(), AssistantError::Unavailable { detail } if detail.contains("529"))
    );
    assert!(answer.error().may_retry());
    let (_stage, _talk, answer) = replayed("error-server.jsonl");
    assert!(
        matches!(answer.error(), AssistantError::Unavailable { detail } if detail.contains("500"))
    );
}

#[test]
fn retries_show_under_the_answer_while_they_last() {
    let (_stage, _talk, answer) = replayed("error-overloaded-retries.jsonl");
    assert_eq!(
        answer.statuses(),
        [
            "The service is busy, trying again (1 of 4)",
            "The service is busy, trying again (2 of 4)",
            "The service is busy, trying again (3 of 4)",
            "The service is busy, trying again (4 of 4)",
        ]
    );
    assert!(matches!(answer.error(), AssistantError::Unavailable { .. }));
}

#[test]
fn no_connection_is_a_network_error() {
    let (_stage, talk, answer) = replayed("error-no-connection.jsonl");
    assert!(
        matches!(answer.error(), AssistantError::Network { detail } if detail.contains("Connection refused")),
        "{:?}",
        answer.end
    );
    let text = answer.error().text(&ClaudeCode::new().info());
    assert_eq!(text.headline, "Claude Code could not reach Anthropic.");
    assert_eq!(
        text.advice,
        "Check the internet connection, then send the message again."
    );
    assert!(answer.error().may_retry());
    // The conversation is kept: the same process answers when the connection is back.
    assert_eq!(talk.ask("Again.").text, "Fine again.");

    let (_stage, _talk, answer) = replayed("error-no-connection-retries.jsonl");
    assert!(matches!(answer.error(), AssistantError::Network { .. }));
    assert_eq!(answer.statuses()[0], "No connection, trying again (1 of 4)");
    assert_eq!(answer.statuses().len(), 4);
}

#[test]
fn a_model_the_service_does_not_know_is_rejected() {
    let (_stage, _talk, answer) = replayed("error-model.jsonl");
    assert!(
        matches!(answer.error(), AssistantError::Rejected { detail } if detail.contains("selected model"))
    );
    assert!(!answer.error().may_retry());
}

#[test]
fn a_stream_that_is_cut_off_fails_with_what_the_program_says() {
    let (_stage, _talk, answer) = replayed("error-cut-stream.jsonl");
    assert!(matches!(answer.end, TurnEnd::Failed(_)));
    // What had arrived before the cut stays with the turn.
    assert_eq!(answer.text, "This answer is cut");
}

#[test]
fn a_program_that_dies_in_the_middle_fails_the_turn_and_ends_the_conversation() {
    let stage = Stage::new(json!({"turns": [{
        "replay": fixture("three-turns.jsonl"),
        "die_after_lines": 4, "exit_code": 9,
        "stderr": "fatal: something broke inside",
    }]}));
    let talk = Talk::start(&stage.backend(), options());
    let open = talk.conversation.send("One.");
    let waiting = talk.conversation.send("Two.");
    let events = talk.until_finished(waiting);
    let answer = Answer::of(open, &events);
    let AssistantError::Exited { code, detail } = answer.error() else {
        panic!("{:?}", answer.end);
    };
    assert_eq!(*code, Some(9));
    assert!(detail.contains("something broke inside"), "{detail}");
    // The message that was waiting fails with it: it is not silently lost.
    assert!(matches!(
        Answer::of(waiting, &events).error(),
        AssistantError::Exited { .. }
    ));
    // The conversation is over; a new one is started with the history.
    assert!(!talk.conversation.is_alive());
    let late = talk.ask("Anyone?");
    assert!(matches!(late.error(), AssistantError::Exited { .. }));
    let text = answer.error().text(&ClaudeCode::new().info());
    assert_eq!(text.headline, "Claude Code stopped unexpectedly.");
    assert!(text.advice.contains("the conversation is kept"));
}

#[test]
fn a_program_that_dies_saying_it_is_not_logged_in_is_not_signed_in() {
    let stage = Stage::new(json!({"turns": [{
        "text": "never printed", "die_after_lines": 0, "exit_code": 1,
        "stderr": "Not logged in · Please run /login",
    }]}));
    let talk = Talk::start(&stage.backend(), options());
    assert_eq!(talk.ask("One.").error(), &AssistantError::NotSignedIn);
}

#[test]
fn a_program_that_ignores_stop_is_ended() {
    let stage = Stage::new(json!({"turns": [{
        "replay": fixture("interrupted-turn.jsonl"), "hold_after_deltas": 3, "ignore_interrupt": true,
    }]}));
    let talk = Talk::start(&stage.backend(), options());
    let turn = talk.conversation.send("Count.");
    let (mut seen, _) = talk.events.wait_until(PATIENCE, |event| {
        matches!(event, writer_assistant::AssistantEvent::Text { .. })
    });
    let pressed = Instant::now();
    talk.conversation.stop();
    seen.extend(talk.until_finished(turn));
    // For the user the answer stopped at once...
    assert_eq!(Answer::of(turn, &seen).end, TurnEnd::Stopped);
    assert!(pressed.elapsed() < Duration::from_secs(2));
    // ...and the program that did not confirm is ended after its patience, so that the
    // next message does not wait behind an answer nobody wants.
    let deadline = Instant::now() + PATIENCE;
    while talk.conversation.is_alive() {
        assert!(Instant::now() < deadline, "the program was never ended");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(pressed.elapsed() >= Duration::from_secs(4));
    assert!(matches!(
        talk.ask("Hello?").error(),
        AssistantError::Exited { .. }
    ));
}

#[test]
fn a_silent_program_can_still_be_stopped() {
    let stage = Stage::new(json!({"turns": [{"silent": true}]}));
    let talk = Talk::start(&stage.backend(), options());
    let turn = talk.conversation.send("Hello?");
    stage.wait_for_log("the message", |stage| stage.messages().len() == 1);
    talk.conversation.stop();
    assert_eq!(
        Answer::of(turn, &talk.until_finished(turn)).end,
        TurnEnd::Stopped
    );
}

#[test]
fn a_missing_program_is_not_installed() {
    let stage = Stage::new(json!({}));
    let claude = ClaudeCode::with_program(stage.path().join("no such claude"));
    let talk = Talk::start(&claude, options());
    let answer = talk.ask("Hello?");
    assert_eq!(answer.error(), &AssistantError::NotInstalled);
    let text = answer.error().text(&claude.info());
    assert_eq!(text.headline, "Claude Code is not installed on this Mac.");
    // The conversation is not dead: once the program is there, it answers.
    assert!(talk.conversation.is_alive());
}

#[test]
fn a_program_that_appears_later_is_found_with_the_next_message() {
    let stage = Stage::new(json!({"turns": [{"text": "Here now.", "pieces": 1}]}));
    let hidden = stage.path().join("bin").join("claude");
    let elsewhere = stage.path().join("bin").join("claude.away");
    std::fs::rename(&hidden, &elsewhere).unwrap();
    let claude = ClaudeCode::with_program(&hidden);
    let talk = Talk::start(&claude, options());
    assert_eq!(talk.ask("Hello?").error(), &AssistantError::NotInstalled);
    // The user installs it and sends again: no restart of the app is needed.
    std::fs::rename(&elsewhere, &hidden).unwrap();
    assert_eq!(talk.ask("Hello?").text, "Here now.");
    assert!(claude.cached_availability().unwrap().is_ready());
}

#[test]
fn a_signed_out_user_is_told_before_any_process_is_started() {
    let stage = Stage::new(json!({"auth": "signed-out", "turns": [{"text": "x", "pieces": 1}]}));
    let claude = stage.backend();
    let talk = Talk::start(&claude, options());
    let answer = talk.ask("Hello?");
    assert_eq!(answer.error(), &AssistantError::NotSignedIn);
    assert!(
        stage.conversations().is_empty(),
        "no conversation process for a signed-out user"
    );
    let text = answer.error().text(&claude.info());
    assert_eq!(text.headline, "You are not signed in to Claude Code.");
    assert_eq!(
        text.advice,
        "Open Terminal, run claude and sign in. Then send the message again."
    );
}

#[test]
fn a_version_that_would_expand_file_mentions_is_refused() {
    let stage = Stage::new(
        json!({"version": "2.1.100 (Claude Code)", "turns": [{"text": "x", "pieces": 1}]}),
    );
    let claude = stage.backend();
    let talk = Talk::start(&claude, options());
    let answer = talk.ask("Is @notes.md right?");
    assert_eq!(
        answer.error(),
        &AssistantError::Outdated {
            found: "2.1.100".to_string(),
            needed: MINIMUM_VERSION.to_string()
        }
    );
    assert!(
        stage.conversations().is_empty(),
        "the message never reaches the old program"
    );
    let text = answer.error().text(&claude.info());
    assert_eq!(
        text.headline,
        "Claude Code 2.1.100 is too old for the assistant."
    );
    assert!(text.advice.contains(MINIMUM_VERSION));
}
