//! Changing the model and the effort in the middle of a conversation.

use std::sync::Arc;

use serde_json::json;
use writer_assistant::{
    AssistantError, AssistantEvent, ChatSession, ConversationOptions, MessageState, Selection,
};

use super::session::settle;
use super::support::{Stage, Talk, value_after};

#[test]
fn the_running_program_is_told_and_keeps_the_conversation() {
    let stage = Stage::new(json!({"select": "ok", "turns": [{"echo": true, "pieces": 1}]}));
    let talk = Talk::start(
        &stage.backend(),
        ConversationOptions::new("x").with_selection(Selection::named("haiku", "low")),
    );
    talk.ask("One.");
    assert!(talk.conversation.select(Selection::named("sonnet", "high")));
    let turn = talk.conversation.send("Two.");
    let events = talk.until_finished(turn);
    // The same process, told through its own requests; no restart, no replay.
    assert_eq!(stage.conversations().len(), 1);
    assert_eq!(stage.messages(), ["One.", "Two."]);
    let requests: Vec<serde_json::Value> = stage
        .received()
        .into_iter()
        .filter(|line| line["type"] == "control_request")
        .map(|line| line["request"].clone())
        .collect();
    assert_eq!(
        requests,
        [
            json!({"subtype": "set_model", "model": "sonnet"}),
            json!({"subtype": "apply_flag_settings", "settings": {"effortLevel": "high"}}),
        ]
    );
    // The requests went out before the message they are for.
    let kinds: Vec<String> = stage
        .received()
        .iter()
        .map(|line| line["type"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        kinds,
        ["user", "control_request", "control_request", "user"]
    );
    // The program says which model answers now.
    assert!(events.iter().any(
        |event| matches!(event, AssistantEvent::Ready { model: Some(model) } if model == "claude-sonnet")
    ));
}

#[test]
fn only_what_changed_is_asked_for_and_the_default_can_be_chosen_again() {
    let stage = Stage::new(json!({"turns": [{"text": "ok", "pieces": 1}]}));
    let talk = Talk::start(
        &stage.backend(),
        ConversationOptions::new("x").with_selection(Selection::named("haiku", "low")),
    );
    talk.ask("One.");
    talk.conversation
        .select(Selection::named("haiku", "medium"));
    talk.ask("Two.");
    talk.conversation.select(Selection::default());
    talk.ask("Three.");
    let requests: Vec<serde_json::Value> = stage
        .received()
        .into_iter()
        .filter(|line| line["type"] == "control_request")
        .map(|line| line["request"].clone())
        .collect();
    assert_eq!(
        requests,
        [
            json!({"subtype": "apply_flag_settings", "settings": {"effortLevel": "medium"}}),
            json!({"subtype": "set_model", "model": "default"}),
            json!({"subtype": "apply_flag_settings", "settings": {"effortLevel": null}}),
        ]
    );
}

#[test]
fn a_selection_made_before_the_first_message_is_how_the_program_starts() {
    let stage = Stage::new(json!({"turns": [{"text": "ok", "pieces": 1}]}));
    let talk = Talk::start(&stage.backend(), ConversationOptions::new("x"));
    talk.conversation.select(Selection::named("opus", "max"));
    talk.ask("One.");
    let argv = &stage.conversations()[0];
    assert_eq!(value_after(argv, "--model"), Some("opus"));
    assert_eq!(value_after(argv, "--effort"), Some("max"));
    assert!(stage.requests().is_empty());
}

#[test]
fn a_model_the_program_refuses_fails_the_next_message_until_another_is_chosen() {
    let stage = Stage::new(json!({"select": "reject", "turns": [{"text": "ok", "pieces": 1}]}));
    let talk = Talk::start(
        &stage.backend(),
        ConversationOptions::new("x").with_selection(Selection::named("haiku", "low")),
    );
    talk.ask("One.");
    talk.conversation
        .select(Selection::named("no-such-model", "low"));
    let refused = talk.ask("Two.");
    assert!(
        matches!(refused.error(), AssistantError::Rejected { detail } if detail.contains("Unable to validate model")),
        "{:?}",
        refused.end
    );
    assert!(talk.conversation.is_alive(), "the conversation is kept");
    assert_eq!(
        stage.messages(),
        ["One."],
        "the message did not go to the wrong model"
    );
    // Back to the model that works: no request is needed, and the message goes out.
    talk.conversation.select(Selection::named("haiku", "low"));
    assert_eq!(talk.ask("Two, again.").text, "ok");
}

#[test]
fn a_program_that_cannot_change_while_it_runs_is_started_anew_with_the_history() {
    let stage =
        Stage::new(json!({"select": "unsupported", "turns": [{"echo": true, "pieces": 1}]}));
    let mut chat = ChatSession::quick(Arc::new(stage.backend()))
        .with_selection(Selection::named("haiku", "low"));
    let events = chat.events();
    chat.ask_quick("A name for a lighthouse keeper?", None);
    settle(&mut chat, &events);
    chat.set_selection(Selection::named("sonnet", "medium"));
    // The program refuses the request and the conversation ends quietly...
    stage.wait_for_log("the request", |stage| {
        stage.requests().contains(&"set_model".to_string())
    });
    stage.wait_for_log("the end of the first process", |stage| {
        stage
            .notes()
            .iter()
            .any(|note| note.starts_with("input closed"))
    });
    // ...so the next message starts the program again, with the selection as flags and
    // with what was said so far.
    chat.ask_quick("And her dog?", None);
    settle(&mut chat, &events);
    let launches = stage.conversations();
    assert_eq!(launches.len(), 2);
    assert_eq!(value_after(&launches[1], "--model"), Some("sonnet"));
    assert_eq!(value_after(&launches[1], "--effort"), Some("medium"));
    let messages = stage.messages();
    assert!(
        messages[1].contains("<conversation_so_far>"),
        "{}",
        messages[1]
    );
    assert!(messages[1].contains("A name for a lighthouse keeper?"));
    assert!(messages[1].ends_with("And her dog?"));
    // The user saw two questions and two answers, and no error.
    let states: Vec<&MessageState> = chat
        .transcript()
        .messages
        .iter()
        .map(|message| &message.state)
        .collect();
    assert_eq!(states, [&MessageState::Done; 4]);
}
