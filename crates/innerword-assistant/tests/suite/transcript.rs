//! The conversation as it is shown and stored.

use innerword_assistant::{
    AssistantError, AssistantEvent, Attachment, AttachmentKind, CUT_OFF_NOTE, MessageState, Role,
    Selection, ToolActivity, ToolState, Transcript, TurnEnd, TurnId, Usage,
};

fn text(turn: TurnId, piece: &str) -> AssistantEvent {
    AssistantEvent::Text {
        turn,
        text: piece.to_string(),
    }
}

fn finished(turn: TurnId, end: TurnEnd) -> AssistantEvent {
    AssistantEvent::Finished {
        turn,
        text: None,
        end,
        usage: None,
    }
}

fn tool(turn: TurnId, id: &str, description: &str, state: ToolState) -> AssistantEvent {
    AssistantEvent::Tool {
        turn,
        activity: ToolActivity {
            id: id.to_string(),
            tool: "read_document".to_string(),
            description: description.to_string(),
            state,
        },
    }
}

#[test]
fn a_question_and_its_answer_grow_from_the_events() {
    let mut transcript = Transcript::new();
    assert!(transcript.is_empty());
    let question = transcript.push_user("A word for very tired?", Some("very tired".into()));
    let turn = TurnId::next();
    let answer = transcript.begin_answer(turn, Selection::named("haiku", "low"));
    assert_eq!(transcript.len(), 2);
    assert!(transcript.is_answering());
    assert_eq!(
        transcript.message(question).unwrap().state,
        MessageState::Done
    );
    assert_eq!(
        transcript.message(question).unwrap().quote.as_deref(),
        Some("very tired")
    );
    assert_eq!(
        transcript.message(answer).unwrap().state,
        MessageState::Streaming
    );
    assert!(
        !transcript.message(answer).unwrap().is_applicable(),
        "nothing to insert yet"
    );

    assert!(transcript.apply(&text(turn, "exhausted, ")));
    assert!(transcript.apply(&text(turn, "*spent*")));
    assert!(
        !transcript.apply(&text(turn, "")),
        "an empty piece changes nothing"
    );
    assert_eq!(
        transcript.message(answer).unwrap().text,
        "exhausted, *spent*"
    );
    let usage = Usage {
        input_tokens: Some(40),
        output_tokens: Some(6),
        cost_usd: Some(0.0001),
        duration_ms: Some(900),
    };
    assert!(transcript.apply(&AssistantEvent::Finished {
        turn,
        text: None,
        end: TurnEnd::Complete,
        usage: Some(usage.clone()),
    }));
    let message = transcript.message(answer).unwrap();
    assert_eq!(message.state, MessageState::Done);
    assert_eq!(message.usage.as_ref(), Some(&usage));
    assert_eq!(message.selection, Some(Selection::named("haiku", "low")));
    assert!(message.is_applicable());
    assert!(!transcript.is_answering());
    assert_eq!(transcript.message_for_turn(turn).unwrap().id, answer);
    assert_eq!(transcript.last_answer().unwrap().id, answer);
    assert_eq!(transcript.question_of(answer).unwrap().id, question);
    // Once an answer is finished, late events of its turn change nothing.
    assert!(!transcript.apply(&text(turn, " and more")));
    assert!(!transcript.apply(&finished(turn, TurnEnd::Stopped)));
    assert_eq!(
        transcript.message(answer).unwrap().text,
        "exhausted, *spent*"
    );
}

#[test]
fn an_answer_can_end_stopped_or_failed_and_keeps_what_arrived() {
    let mut transcript = Transcript::new();
    transcript.push_user("Count.", None);
    let stopped = TurnId::next();
    let first = transcript.begin_answer(stopped, Selection::default());
    transcript.apply(&text(stopped, "1, 2, 3"));
    transcript.apply(&finished(stopped, TurnEnd::Stopped));
    let message = transcript.message(first).unwrap();
    assert_eq!(message.state, MessageState::Stopped);
    assert_eq!(message.text, "1, 2, 3");
    assert!(
        message.is_applicable(),
        "a stopped answer with text can be inserted"
    );

    transcript.push_user("Again.", None);
    let failed = TurnId::next();
    let second = transcript.begin_answer(failed, Selection::default());
    let error = AssistantError::UsageLimit {
        resets_at: Some(1_800_000_000),
        detail: "limit".into(),
    };
    transcript.apply(&finished(failed, TurnEnd::Failed(error.clone())));
    let message = transcript.message(second).unwrap();
    assert_eq!(message.state, MessageState::Failed(error.clone()));
    assert_eq!(message.error(), Some(&error));
    assert!(
        !message.is_applicable(),
        "nothing to insert from an empty failed answer"
    );
}

#[test]
fn the_final_text_fills_an_answer_that_did_not_arrive_in_pieces() {
    let mut transcript = Transcript::new();
    transcript.push_user("?", None);
    let turn = TurnId::next();
    let answer = transcript.begin_answer(turn, Selection::default());
    transcript.apply(&AssistantEvent::Finished {
        turn,
        text: Some("All at once.".into()),
        end: TurnEnd::Complete,
        usage: None,
    });
    assert_eq!(transcript.message(answer).unwrap().text, "All at once.");
    // Pieces that arrived are not replaced by a shorter or equal final text.
    transcript.push_user("?", None);
    let turn = TurnId::next();
    let answer = transcript.begin_answer(turn, Selection::default());
    transcript.apply(&text(turn, "Before the tool.\n\nAfter the tool."));
    transcript.apply(&AssistantEvent::Finished {
        turn,
        text: Some("After the tool.".into()),
        end: TurnEnd::Complete,
        usage: None,
    });
    assert_eq!(
        transcript.message(answer).unwrap().text,
        "Before the tool.\n\nAfter the tool."
    );
}

#[test]
fn tool_calls_and_status_lines_say_what_the_assistant_is_doing() {
    let mut transcript = Transcript::new();
    transcript.push_user("Compare the chapters.", None);
    let turn = TurnId::next();
    let answer = transcript.begin_answer(turn, Selection::default());
    assert_eq!(transcript.message(answer).unwrap().current_activity(), None);

    assert!(transcript.apply(&AssistantEvent::Status {
        turn,
        text: "Thinking".into()
    }));
    assert!(
        !transcript.apply(&AssistantEvent::Status {
            turn,
            text: "Thinking".into()
        }),
        "the same line again"
    );
    assert_eq!(
        transcript.message(answer).unwrap().current_activity(),
        Some("Thinking")
    );

    assert!(transcript.apply(&tool(turn, "t1", "Reading Chapter 1", ToolState::Running)));
    assert_eq!(
        transcript.message(answer).unwrap().current_activity(),
        Some("Reading Chapter 1")
    );
    assert!(!transcript.apply(&tool(turn, "t1", "Reading Chapter 1", ToolState::Running)));
    assert!(transcript.apply(&tool(turn, "t2", "Reading Chapter 2", ToolState::Running)));
    assert_eq!(
        transcript.message(answer).unwrap().current_activity(),
        Some("Reading Chapter 2")
    );
    assert!(transcript.apply(&tool(turn, "t2", "Reading Chapter 2", ToolState::Done)));
    assert_eq!(
        transcript.message(answer).unwrap().current_activity(),
        Some("Reading Chapter 1")
    );
    assert!(transcript.apply(&tool(turn, "t1", "", ToolState::Failed)));
    let message = transcript.message(answer).unwrap();
    assert_eq!(message.current_activity(), None);
    // The calls stay with the answer, in the order they began, each once.
    let calls: Vec<(&str, ToolState)> = message
        .activity
        .iter()
        .map(|a| (a.description.as_str(), a.state))
        .collect();
    assert_eq!(
        calls,
        [
            ("Reading Chapter 1", ToolState::Failed),
            ("Reading Chapter 2", ToolState::Done)
        ]
    );

    // A status holds until the next event of the turn; an empty one clears it.
    transcript.apply(&AssistantEvent::Status {
        turn,
        text: "No connection, trying again (1 of 4)".into(),
    });
    assert_eq!(
        transcript.message(answer).unwrap().current_activity(),
        Some("No connection, trying again (1 of 4)")
    );
    assert!(transcript.apply(&AssistantEvent::Status {
        turn,
        text: String::new()
    }));
    assert_eq!(transcript.message(answer).unwrap().current_activity(), None);
    transcript.apply(&AssistantEvent::Status {
        turn,
        text: "Thinking".into(),
    });
    transcript.apply(&text(turn, "They differ in"));
    assert_eq!(transcript.message(answer).unwrap().current_activity(), None);

    // A call still running when the answer ends is closed with it.
    transcript.apply(&tool(turn, "t3", "Reading Chapter 3", ToolState::Running));
    transcript.apply(&finished(turn, TurnEnd::Stopped));
    let message = transcript.message(answer).unwrap();
    assert_eq!(message.activity[2].state, ToolState::Failed);
    assert_eq!(message.current_activity(), None);
}

#[test]
fn events_of_turns_the_transcript_does_not_know_change_nothing() {
    let mut transcript = Transcript::new();
    transcript.push_user("?", None);
    let known = TurnId::next();
    transcript.begin_answer(known, Selection::default());
    let before = transcript.clone();
    let unknown = TurnId::next();
    assert!(!transcript.apply(&text(unknown, "from a cleared conversation")));
    assert!(!transcript.apply(&finished(unknown, TurnEnd::Complete)));
    assert!(!transcript.apply(&AssistantEvent::Ready {
        model: Some("m".into())
    }));
    assert!(!transcript.apply(&AssistantEvent::Notice {
        text: "close to the limit".into()
    }));
    assert_eq!(transcript, before);
}

#[test]
fn it_is_stored_as_json_and_comes_back_with_open_answers_closed() {
    let mut transcript = Transcript::new();
    transcript.attach(Attachment::folder(3, "Part two", "The Salt Road/Part two"));
    transcript.push_user("Who is Mara's brother?", None);
    let turn = TurnId::next();
    let done = transcript.begin_answer(turn, Selection::named("sonnet", "medium"));
    transcript.apply(&tool(turn, "t1", "Reading Characters", ToolState::Running));
    transcript.apply(&tool(turn, "t1", "Reading Characters", ToolState::Done));
    transcript.apply(&text(turn, "**Tam**, the netmaker."));
    transcript.apply(&AssistantEvent::Finished {
        turn,
        text: None,
        end: TurnEnd::Complete,
        usage: Some(Usage {
            input_tokens: Some(900),
            output_tokens: Some(12),
            cost_usd: None,
            duration_ms: Some(2100),
        }),
    });
    transcript.push_user("And his age?", None);
    let failed = TurnId::next();
    transcript.begin_answer(failed, Selection::named("sonnet", "medium"));
    transcript.apply(&finished(
        failed,
        TurnEnd::Failed(AssistantError::Network {
            detail: "ECONNREFUSED".into(),
        }),
    ));
    transcript.push_user("Again: his age?", None);
    let open = TurnId::next();
    let streaming = transcript.begin_answer(open, Selection::default());
    transcript.apply(&text(open, "He is"));
    transcript.apply(&tool(open, "t9", "Reading Chapter 4", ToolState::Running));

    let json = transcript.to_json();
    let back = Transcript::from_json(&json).unwrap();
    assert_eq!(back.messages.len(), 6);
    assert_eq!(back.attachments, transcript.attachments);
    // What was finished is as it was...
    let stored = back.message(done).unwrap();
    assert_eq!(stored.text, "**Tam**, the netmaker.");
    assert_eq!(stored.state, MessageState::Done);
    assert_eq!(stored.selection, Some(Selection::named("sonnet", "medium")));
    assert_eq!(stored.activity.len(), 1);
    assert_eq!(stored.usage.as_ref().unwrap().input_tokens, Some(900));
    assert_eq!(
        back.messages[0].attachments.len(),
        1,
        "what was attached when it was asked"
    );
    assert!(matches!(
        back.messages[3].state,
        MessageState::Failed(AssistantError::Network { .. })
    ));
    // ...and the answer that was being written when the app quit is a stopped one.
    let reopened = back.message(streaming).unwrap();
    assert_eq!(reopened.state, MessageState::Stopped);
    assert_eq!(reopened.text, "He is");
    assert_eq!(reopened.activity[0].state, ToolState::Failed);
    assert!(!back.is_answering());
    // New messages get ids that no stored message has.
    let mut back = back;
    let next = back.push_user("One more.", None);
    assert!(
        back.messages
            .iter()
            .filter(|message| message.id == next)
            .count()
            == 1
    );
    assert!(next.0 > streaming.0);
    // What is not a transcript is an error, not a panic; an empty object is an empty one.
    assert!(Transcript::from_json("not json").is_err());
    assert!(Transcript::from_json("{}").unwrap().is_empty());
}

#[test]
fn the_history_for_a_new_process_is_what_was_said_without_open_or_empty_answers() {
    let mut transcript = Transcript::new();
    transcript.push_user(
        "Shorter?",
        Some("The rain, which had been falling, fell.".into()),
    );
    let one = TurnId::next();
    transcript.begin_answer(one, Selection::default());
    transcript.apply(&text(one, "The rain fell."));
    transcript.apply(&finished(one, TurnEnd::Complete));
    transcript.push_user("And without the article?", None);
    let two = TurnId::next();
    transcript.begin_answer(two, Selection::default());
    transcript.apply(&finished(two, TurnEnd::Failed(AssistantError::NotSignedIn)));
    transcript.push_user("Again?", None);
    let three = TurnId::next();
    transcript.begin_answer(three, Selection::default());
    transcript.apply(&text(three, "Rain f"));

    let history = transcript.history(10_000);
    let read: Vec<(Role, &str)> = history.iter().map(|m| (m.role, m.text.as_str())).collect();
    assert_eq!(read.len(), 4);
    // The quick chat's question carries its selection, as the agent first saw it.
    assert!(
        read[0]
            .1
            .contains("<selected_text>\nThe rain, which had been falling, fell.\n</selected_text>")
    );
    assert!(read[0].1.ends_with("Shorter?"));
    assert_eq!(read[1], (Role::Assistant, "The rain fell."));
    assert_eq!(read[2], (Role::User, "And without the article?"));
    // The failed answer had no text and the open one is not history yet.
    assert_eq!(read[3], (Role::User, "Again?"));
    // Bounded: the most recent that fit, beginning with a question.
    let short = transcript.history(40);
    assert_eq!(short.first().unwrap().role, Role::User);
    assert!(short.len() < 4);
    assert_eq!(short.last().unwrap().text, "Again?");
}

#[test]
fn the_oldest_messages_go_when_there_are_too_many() {
    let mut transcript = Transcript::new();
    for round in 0..(Transcript::MAX_MESSAGES + 30) {
        transcript.push_user(format!("question {round}"), None);
        let turn = TurnId::next();
        transcript.begin_answer(turn, Selection::default());
        transcript.apply(&text(turn, &format!("answer {round}")));
        transcript.apply(&finished(turn, TurnEnd::Complete));
    }
    assert!(transcript.len() <= Transcript::MAX_MESSAGES);
    assert!(transcript.dropped > 0);
    assert_eq!(
        transcript.dropped + transcript.len(),
        2 * (Transcript::MAX_MESSAGES + 30)
    );
    // It still begins with a question and ends with the last answer.
    assert_eq!(transcript.messages[0].role, Role::User);
    assert_eq!(
        transcript.messages.last().unwrap().text,
        format!("answer {}", Transcript::MAX_MESSAGES + 29)
    );
    // The bound holds for a stored transcript too.
    let back = Transcript::from_json(&transcript.to_json()).unwrap();
    assert!(back.len() <= Transcript::MAX_MESSAGES);
    assert_eq!(back.dropped, transcript.dropped);
}

#[test]
fn the_oldest_messages_go_when_the_text_is_too_much_but_never_the_last_exchange() {
    let mut transcript = Transcript::new();
    let big = "x".repeat(Transcript::MAX_CHARS / 3);
    for _ in 0..5 {
        transcript.push_user("question", None);
        let turn = TurnId::next();
        transcript.begin_answer(turn, Selection::default());
        transcript.apply(&text(turn, &big));
        transcript.apply(&finished(turn, TurnEnd::Complete));
    }
    assert!(
        transcript.char_count() <= Transcript::MAX_CHARS,
        "{}",
        transcript.char_count()
    );
    assert!(transcript.len() < 10);
    assert_eq!(transcript.messages[0].role, Role::User);
    // One exchange that alone is over the bound stays: there is nothing older to drop.
    let mut transcript = Transcript::new();
    transcript.push_user("x".repeat(Transcript::MAX_CHARS + 10), None);
    let turn = TurnId::next();
    transcript.begin_answer(turn, Selection::default());
    assert_eq!(transcript.len(), 2);
}

#[test]
fn one_runaway_answer_is_cut_off_and_says_so() {
    let mut transcript = Transcript::new();
    transcript.push_user("Write forever.", None);
    let turn = TurnId::next();
    let answer = transcript.begin_answer(turn, Selection::default());
    let piece = "é".repeat(10_000);
    for _ in 0..(Transcript::MAX_ANSWER_CHARS / 10_000 + 5) {
        transcript.apply(&text(turn, &piece));
    }
    let message = transcript.message(answer).unwrap();
    assert_eq!(
        message.text.chars().count(),
        Transcript::MAX_ANSWER_CHARS + CUT_OFF_NOTE.chars().count()
    );
    assert!(message.text.ends_with(CUT_OFF_NOTE));
    // More pieces change nothing, and the answer still ends like any other.
    assert!(!transcript.apply(&text(turn, "more")));
    assert!(transcript.apply(&finished(turn, TurnEnd::Complete)));
    assert_eq!(
        transcript.message(answer).unwrap().state,
        MessageState::Done
    );
}

#[test]
fn library_items_are_attached_once_and_detached() {
    let mut transcript = Transcript::new();
    assert!(transcript.attach(Attachment::document(14, "Characters", "Notes/Characters")));
    assert!(
        transcript.attach(Attachment::folder(14, "Fourteen", "Fourteen")),
        "a folder with the same number is another item"
    );
    // The same document again: not added twice, but its new title is taken.
    assert!(!transcript.attach(Attachment::document(
        14,
        "The Characters",
        "Notes/The Characters"
    )));
    assert_eq!(transcript.attachments.len(), 2);
    assert_eq!(transcript.attachments[0].title, "The Characters");
    assert_eq!(transcript.attachments[0].reference(), "doc:14");
    assert_eq!(transcript.attachments[1].reference(), "folder:14");
    assert!(transcript.detach(AttachmentKind::Document, 14));
    assert!(!transcript.detach(AttachmentKind::Document, 14));
    assert_eq!(transcript.attachments.len(), 1);
    // There is a most.
    for id in 100..200 {
        transcript.attach(Attachment::document(id, "d", "d"));
    }
    assert_eq!(transcript.attachments.len(), Transcript::MAX_ATTACHMENTS);
    // Clearing the messages keeps what is attached; a new conversation does not.
    transcript.push_user("?", None);
    transcript.clear_messages();
    assert!(transcript.is_empty());
    assert_eq!(transcript.attachments.len(), Transcript::MAX_ATTACHMENTS);
    transcript.reset();
    assert!(transcript.attachments.is_empty());
}
