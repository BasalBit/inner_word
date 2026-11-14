//! A chat as a view holds it, over the Claude Code backend and the stand-in program.

use std::sync::Arc;

use serde_json::json;
use writer_assistant::{
    Attachment, ChatSession, EventStream, MessageState, OpenDocument, OpenKind, Role, Selection,
    Transcript, library_tools,
};

use super::support::{PATIENCE, Stage, fixture, value_after};

/// Applies the events of the chat until no answer is being written.
pub fn settle(chat: &mut ChatSession, events: &EventStream) {
    while chat.is_answering() {
        let event = events
            .wait(PATIENCE)
            .unwrap_or_else(|| panic!("the answer never finished: {:#?}", chat.transcript()));
        chat.apply(event);
    }
    for event in events.drain() {
        chat.apply(event);
    }
}

#[test]
fn the_quick_chat_sends_the_selection_and_the_question_and_nothing_else() {
    let stage = Stage::new(
        json!({"turns": [{"replay": fixture("quick-turn.jsonl")}, {"echo": true, "pieces": 1}]}),
    );
    let mut chat = ChatSession::quick(Arc::new(stage.backend()));
    let events = chat.events();
    assert!(
        stage.launches().is_empty(),
        "opening the chat starts nothing and sends nothing"
    );

    chat.ask_quick("What is the code word?", Some("The code word is HERON."));
    assert!(chat.is_answering());
    settle(&mut chat, &events);

    let messages = &chat.transcript().messages;
    assert_eq!(messages.len(), 2);
    assert_eq!(
        (messages[0].role, messages[0].text.as_str()),
        (Role::User, "What is the code word?")
    );
    assert_eq!(
        messages[0].quote.as_deref(),
        Some("The code word is HERON.")
    );
    assert_eq!(
        (messages[1].role, messages[1].text.as_str()),
        (Role::Assistant, "HERON")
    );
    assert_eq!(messages[1].state, MessageState::Done);
    assert_eq!(
        messages[1].selection,
        Some(Selection::named("haiku", "low")),
        "the quick chat's default"
    );
    assert!(messages[1].usage.is_some());
    assert_eq!(chat.model_in_use(), Some("claude-haiku-5-5"));

    // With nothing selected, the question goes as it was typed.
    chat.ask_quick("A word for very tired?", None);
    settle(&mut chat, &events);
    let sent = stage.messages();
    assert!(sent[0].contains("<selected_text>\nThe code word is HERON.\n</selected_text>"));
    assert!(sent[0].ends_with("What is the code word?"));
    assert_eq!(sent[1], "A word for very tired?");
    // One process, started with the quick chat's prompt, a fast model and no tools.
    let launches = stage.conversations();
    assert_eq!(launches.len(), 1);
    assert_eq!(value_after(&launches[0], "--model"), Some("haiku"));
    assert_eq!(value_after(&launches[0], "--effort"), Some("low"));
    assert_eq!(
        value_after(&launches[0], "--system-prompt"),
        Some(writer_assistant::QUICK_SYSTEM_PROMPT)
    );
    assert!(!launches[0].iter().any(|arg| arg == "--allowedTools"));
}

#[test]
fn the_side_chat_sends_the_document_when_it_changed_and_names_what_is_attached() {
    let stage = Stage::new(json!({"turns": [{"echo": true, "pieces": 1}]}));
    let tools = library_tools(stage.path().join("writer"), stage.path().join("library"));
    let mut chat = ChatSession::side(Arc::new(stage.backend()), Some(tools));
    let events = chat.events();
    let mut document = OpenDocument {
        id: Some(12),
        title: "Chapter 2".into(),
        path: "The Salt Road/Part one/Chapter 2".into(),
        markdown: "# Chapter 2\n\nThe heron stood in the shallows at dawn.".into(),
        kind: OpenKind::Text,
    };
    chat.attach(Attachment::folder(3, "Part two", "The Salt Road/Part two"));

    chat.ask_side("Is the opening too slow?", Some(&document));
    settle(&mut chat, &events);
    chat.ask_side("And the first sentence?", Some(&document));
    settle(&mut chat, &events);
    document.markdown.push_str("\n\nMara counted the boats.");
    chat.ask_side("Better now?", Some(&document));
    settle(&mut chat, &events);

    let sent = stage.messages();
    // The first message carries the document whole, with its title, path and id, and
    // the attached folder by id and path.
    assert!(sent[0].contains(r#"<open_document title="Chapter 2" path="The Salt Road/Part one/Chapter 2" id="doc:12">"#), "{}", sent[0]);
    assert!(sent[0].contains("The heron stood in the shallows at dawn."));
    assert!(sent[0].contains(r#"- folder folder:3 "The Salt Road/Part two""#));
    assert!(sent[0].ends_with("Is the opening too slow?"));
    // Unchanged: one line says so, and the text is not sent again.
    assert!(!sent[1].contains("<open_document"));
    assert!(sent[1].contains("has not changed since my last message"));
    assert!(!sent[1].contains("<attached>"));
    assert!(sent[1].ends_with("And the first sentence?"));
    // Changed: sent again, as it is now.
    assert!(sent[2].contains("<open_document"));
    assert!(sent[2].contains("Mara counted the boats."));
    // What the user sees is only what they typed.
    assert_eq!(
        chat.transcript().messages[0].text,
        "Is the opening too slow?"
    );
    assert_eq!(chat.transcript().messages[0].attachments.len(), 1);
    // The process has the library's server, the side chat's prompt and model.
    let launch = &stage.conversations()[0];
    assert_eq!(value_after(launch, "--model"), Some("sonnet"));
    assert_eq!(value_after(launch, "--effort"), Some("medium"));
    assert!(
        value_after(launch, "--system-prompt")
            .unwrap()
            .contains("read_document")
    );
    assert!(
        value_after(launch, "--mcp-config")
            .unwrap()
            .contains("the-writer")
    );
}

#[test]
fn after_the_program_died_the_next_message_starts_it_again_with_the_conversation() {
    let stage = Stage::new(json!({"turns": [
        {"text": "It is slow, yes.", "pieces": 2},
        {"text": "never arrives", "die_after_lines": 3, "exit_code": 7, "stderr": "boom"},
        {"echo": true, "pieces": 1},
    ]}));
    let mut chat = ChatSession::side(Arc::new(stage.backend()), None);
    let events = chat.events();
    let document = OpenDocument {
        id: Some(12),
        title: "Chapter 2".into(),
        path: "Chapter 2".into(),
        markdown: "The heron stood in the shallows.".into(),
        kind: OpenKind::Text,
    };
    chat.ask_side("Is the opening too slow?", Some(&document));
    settle(&mut chat, &events);
    chat.ask_side("How would you cut it?", Some(&document));
    settle(&mut chat, &events);
    // The answer failed, with words that say the conversation is kept.
    let failed = chat.transcript().messages.last().unwrap();
    assert!(matches!(
        &failed.state,
        MessageState::Failed(writer_assistant::AssistantError::Exited { code: Some(7), .. })
    ));

    // The user sends again: a new process, told what was said, and given the document
    // again although it has not changed (the new process never saw it).
    chat.ask_side("How would you cut it?", Some(&document));
    settle(&mut chat, &events);
    assert_eq!(stage.conversations().len(), 2);
    // The first process took two messages (it died on the second); the third went to
    // the new one.
    let third = &stage.messages()[2];
    assert!(third.contains("<conversation_so_far>"), "{third}");
    assert!(third.contains("<me>\nIs the opening too slow?\n</me>"));
    assert!(third.contains("<you>\nIt is slow, yes.\n</you>"));
    assert!(
        third.contains("<open_document"),
        "the document is sent again"
    );
    assert!(third.ends_with("How would you cut it?"));
    assert_eq!(
        chat.transcript().messages.last().unwrap().state,
        MessageState::Done
    );
}

#[test]
fn a_stored_conversation_continues_after_the_app_was_restarted() {
    // What the app stored: a question, an answer, and an answer that was being written
    // when the app quit.
    let first =
        Stage::new(json!({"turns": [{"text": "Mara Venn.", "pieces": 2}, {"silent": true}]}));
    let mut chat = ChatSession::quick(Arc::new(first.backend()));
    let events = chat.events();
    chat.ask_quick("A name for a lighthouse keeper?", None);
    settle(&mut chat, &events);
    chat.ask_quick("And her dog?", None);
    let stored = chat.transcript().to_json();
    drop(chat);

    // The app starts again.
    let second = Stage::new(json!({"turns": [{"echo": true, "pieces": 1}]}));
    let restored = Transcript::from_json(&stored).unwrap();
    assert_eq!(restored.messages.len(), 4);
    assert_eq!(
        restored.messages[3].state,
        MessageState::Stopped,
        "the open answer is closed"
    );
    let mut chat = ChatSession::quick(Arc::new(second.backend())).with_transcript(restored);
    assert!(!chat.is_answering());
    let events = chat.events();
    chat.ask_quick("Something shorter?", None);
    settle(&mut chat, &events);
    let sent = &second.messages()[0];
    assert!(
        sent.contains("<me>\nA name for a lighthouse keeper?\n</me>"),
        "{sent}"
    );
    assert!(sent.contains("<you>\nMara Venn.\n</you>"));
    assert!(sent.contains("<me>\nAnd her dog?\n</me>"));
    assert!(sent.ends_with("Something shorter?"));
    assert_eq!(chat.transcript().messages.len(), 6);
}

#[test]
fn clearing_ends_the_process_and_the_next_message_starts_clean() {
    let stage = Stage::new(json!({"turns": [{"echo": true, "pieces": 1}]}));
    let mut chat = ChatSession::quick(Arc::new(stage.backend()));
    let events = chat.events();
    chat.ask_quick("First conversation.", None);
    settle(&mut chat, &events);
    chat.clear();
    assert!(chat.transcript().is_empty());
    stage.wait_for_log("the end of the first process", |stage| {
        stage.notes().contains(&"input closed".to_string())
    });
    chat.ask_quick("Second conversation.", None);
    settle(&mut chat, &events);
    assert_eq!(stage.conversations().len(), 2);
    assert_eq!(
        stage.messages()[1],
        "Second conversation.",
        "nothing of the cleared conversation is carried over"
    );
    assert_eq!(chat.transcript().messages.len(), 2);
}

#[test]
fn stopping_a_chat_keeps_what_had_arrived() {
    let stage = Stage::new(json!({"turns": [
        {"replay": fixture("interrupted-turn.jsonl"), "hold_after_deltas": 6},
        {"text": "Short.", "pieces": 1},
    ]}));
    let mut chat = ChatSession::quick(Arc::new(stage.backend()));
    let events = chat.events();
    chat.ask_quick("Count to 400.", None);
    while chat.transcript().messages[1].text.is_empty() {
        let event = events.wait(PATIENCE).expect("text arrives");
        chat.apply(event);
    }
    chat.stop();
    settle(&mut chat, &events);
    let answer = &chat.transcript().messages[1];
    assert_eq!(answer.state, MessageState::Stopped);
    assert!(answer.text.starts_with("1\n2"));
    assert!(
        answer.is_applicable(),
        "a stopped answer can still be inserted"
    );
    chat.ask_quick("Something short.", None);
    settle(&mut chat, &events);
    assert_eq!(chat.transcript().messages[3].text, "Short.");
    assert_eq!(stage.conversations().len(), 1);
}
