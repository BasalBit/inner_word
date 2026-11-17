//! Conversations with the stand-in program, which replays recordings of the real one.

use std::time::{Duration, Instant};

use innerword_assistant::{
    AssistantEvent, Backend, ConversationOptions, HistoryMessage, NameBook, Role, Selection,
    ToolState, TurnEnd, library_tools_named,
};
use serde_json::json;

use super::support::{Answer, PATIENCE, Stage, Talk, fixture, value_after};

fn options() -> ConversationOptions {
    ConversationOptions::new("You help with writing.")
        .with_selection(Selection::named("haiku", "low"))
}

#[test]
fn an_answer_arrives_as_it_is_written_and_ends_once() {
    // The recording of a real answer (Claude Code 2.1.322, the real service).
    let stage = Stage::new(json!({"turns": [{"replay": fixture("quick-turn.jsonl")}]}));
    let talk = Talk::start(&stage.backend(), options());
    assert!(
        stage.conversations().is_empty(),
        "nothing is started before a message"
    );

    let turn = talk
        .conversation
        .send("What is the code word? Answer with the word only.");
    let events = talk.until_finished(turn);
    let answer = Answer::of(turn, &events);
    assert_eq!(answer.text, "HERON");
    assert_eq!(answer.end, TurnEnd::Complete);
    // The pieces are the answer; the last event does not repeat them.
    assert_eq!(answer.final_text, None);
    let usage = answer.usage.expect("the program reports usage");
    assert_eq!(usage.input_tokens, Some(639));
    assert_eq!(usage.output_tokens, Some(5));
    assert!(usage.cost_usd.unwrap() > 0.0);
    assert!(usage.duration_ms.unwrap() > 0);
    // In order: the turn starts, the program says which model answers, text, the end.
    assert!(matches!(events[0], AssistantEvent::Started { turn: started } if started == turn));
    assert!(events.iter().any(
        |event| matches!(event, AssistantEvent::Ready { model: Some(model) } if model == "claude-haiku-5-5")
    ));
    let finished = events
        .iter()
        .filter(|event| matches!(event, AssistantEvent::Finished { .. }))
        .count();
    assert_eq!(finished, 1);
    assert!(talk.conversation.is_alive());
}

#[test]
fn a_conversation_is_one_process_for_all_its_messages() {
    let stage = Stage::new(json!({"turns": [
        {"text": "First answer.", "pieces": 3},
        {"text": "Second answer.", "pieces": 2},
    ]}));
    let talk = Talk::start(&stage.backend(), options());
    assert_eq!(talk.ask("One?").text, "First answer.");
    assert_eq!(talk.ask("Two?").text, "Second answer.");
    assert_eq!(talk.ask("Three?").text, "Second answer.");
    assert_eq!(stage.conversations().len(), 1);
    assert_eq!(stage.messages(), ["One?", "Two?", "Three?"]);
}

#[test]
fn a_message_sent_while_another_is_answered_waits_its_turn() {
    let stage = Stage::new(json!({"turns": [
        {"text": "The first answer, in many pieces, takes a while.", "pieces": 12, "delay_ms": 15},
        {"text": "The second answer.", "pieces": 2},
    ]}));
    let talk = Talk::start(&stage.backend(), options());
    let first = talk.conversation.send("First question");
    let second = talk.conversation.send("Second question");
    let third = talk.conversation.send("Third question");
    let events = talk.until_finished(third);
    assert_eq!(
        Answer::of(first, &events).text,
        "The first answer, in many pieces, takes a while."
    );
    assert_eq!(Answer::of(second, &events).text, "The second answer.");
    assert_eq!(Answer::of(third, &events).end, TurnEnd::Complete);
    // No piece of one answer arrives inside another: every event of the first turn
    // comes before the first event of the second.
    let last_of_first = events
        .iter()
        .rposition(|event| event.turn() == Some(first))
        .unwrap();
    let first_of_second = events
        .iter()
        .position(|event| event.turn() == Some(second))
        .unwrap();
    assert!(last_of_first < first_of_second);
    // And the program was handed each message only when the one before was answered.
    let order: Vec<String> = stage
        .log()
        .iter()
        .filter_map(|entry| {
            if entry["stdin"]["type"] == "user" {
                Some(format!(
                    "message {}",
                    entry["stdin"]["message"]["content"][0]["text"]
                ))
            } else {
                entry["event"]
                    .as_str()
                    .filter(|event| *event == "answered")
                    .map(str::to_string)
            }
        })
        .collect();
    assert_eq!(
        order,
        [
            "message \"First question\"",
            "answered",
            "message \"Second question\"",
            "answered",
            "message \"Third question\"",
            "answered"
        ]
    );
}

#[test]
fn a_tool_call_shows_what_the_assistant_is_reading() {
    // The recording of a real answer that reads a document through the library's server.
    let stage = Stage::new(json!({"turns": [
        {"replay": fixture("side-tool-turn.jsonl")},
        {"replay": fixture("side-tool-turn.jsonl")},
    ]}));
    let names = NameBook::new();
    let tools = library_tools_named(
        "/Applications/Inner Word.app/Contents/MacOS/innerword",
        "/tmp/library",
        &names,
    );
    let talk = Talk::start(&stage.backend(), options().with_tools(Some(tools)));

    let answer = talk.ask("Read doc:1 and give me its first sentence.");
    assert_eq!(
        answer.text,
        "The heron stood in the shallows at dawn, and Mara counted the boats that did not come back."
    );
    assert_eq!(answer.end, TurnEnd::Complete);
    // Nothing is known of doc:1 yet: the line says so much and no more. It starts when
    // the call starts and is done when the tool has answered.
    assert_eq!(
        answer.tools(),
        [
            ("Reading a document".to_string(), ToolState::Running),
            ("Reading a document".to_string(), ToolState::Done),
        ]
    );
    let AssistantEvent::Tool { activity, .. } = answer
        .events
        .iter()
        .find(|event| matches!(event, AssistantEvent::Tool { .. }))
        .unwrap()
    else {
        unreachable!()
    };
    assert_eq!(
        activity.tool, "read_document",
        "the server's own name, without the program's prefix"
    );
    // The model thought before it answered: the line under the answer said so.
    assert!(answer.statuses().contains(&"Thinking".to_string()));
    // The tool call comes before the text.
    let tool_at = answer
        .events
        .iter()
        .position(|event| matches!(event, AssistantEvent::Tool { .. }))
        .unwrap();
    let text_at = answer
        .events
        .iter()
        .position(|event| matches!(event, AssistantEvent::Text { .. }))
        .unwrap();
    assert!(tool_at < text_at);

    // The result named the document, so the next call that names it by id reads well.
    assert_eq!(names.title("doc:1").as_deref(), Some("Chapter 2"));
    let again = talk.ask("Once more.");
    assert_eq!(
        again.tools(),
        [
            ("Reading Chapter 2".to_string(), ToolState::Running),
            ("Reading Chapter 2".to_string(), ToolState::Done),
        ]
    );

    // The program was given the one server, its read-only tools by name, and every
    // other tool of it as withheld.
    let argv = &stage.conversations()[0];
    let config: serde_json::Value =
        serde_json::from_str(value_after(argv, "--mcp-config").unwrap()).unwrap();
    assert_eq!(
        config["mcpServers"]["inner-word"]["args"],
        json!(["--library", "/tmp/library", "mcp"])
    );
    assert!(
        value_after(argv, "--allowedTools")
            .unwrap()
            .contains("mcp__inner-word__read_document")
    );
    assert!(
        value_after(argv, "--disallowedTools")
            .unwrap()
            .contains("mcp__inner-word__edit_document")
    );
    assert_eq!(value_after(argv, "--tools"), Some(""));
}

#[test]
fn a_tool_the_assistant_may_not_use_fails_and_the_answer_goes_on() {
    // Recorded against a stand-in for the service that makes the model call a tool
    // that writes: the program answers "No such tool available".
    let stage = Stage::new(json!({"turns": [{"replay": fixture("withheld-tool-turn.jsonl")}]}));
    let names = NameBook::new();
    let tools = library_tools_named("/innerword", "/tmp/library", &names);
    let talk = Talk::start(&stage.backend(), options().with_tools(Some(tools)));
    let answer = talk.ask("Add a line to the document.");
    assert_eq!(answer.end, TurnEnd::Complete);
    let tools = answer.tools();
    assert_eq!(tools.last().unwrap().1, ToolState::Failed);
    assert_eq!(answer.text, "The answer from the **mock** service.");
}

#[test]
fn stop_ends_the_answer_at_once_and_the_conversation_goes_on() {
    // The recording of a real answer that was interrupted, replayed up to its sixth
    // piece; the rest follows the interrupt.
    let stage = Stage::new(json!({"turns": [
        {"replay": fixture("interrupted-turn.jsonl"), "hold_after_deltas": 6},
        {"text": "A short one.", "pieces": 2},
    ]}));
    let talk = Talk::start(&stage.backend(), options());
    let turn = talk.conversation.send("Count from 1 to 400.");
    // Text arrives...
    let (seen, got_text) = talk.events.wait_until(PATIENCE, |event| {
        matches!(event, AssistantEvent::Text { .. })
    });
    assert!(got_text, "{seen:#?}");
    // ...and the user presses Stop.
    let pressed = Instant::now();
    talk.conversation.stop();
    let rest = talk.until_finished(turn);
    assert!(
        pressed.elapsed() < Duration::from_secs(2),
        "stopping does not wait for the program"
    );
    let mut all = seen;
    all.extend(rest);
    let answer = Answer::of(turn, &all);
    assert_eq!(answer.end, TurnEnd::Stopped);
    assert!(
        answer.text.starts_with("1\n2"),
        "what had arrived stays: {:?}",
        answer.text
    );
    // The program was interrupted through its own request, not killed.
    stage.wait_for_log("the interrupt", |stage| {
        stage.requests().contains(&"interrupt".to_string())
    });
    assert!(talk.conversation.is_alive());
    // The next message is answered by the same process, and nothing of the stopped
    // answer leaks into it.
    let next = talk.ask("Something short.");
    assert_eq!(next.text, "A short one.");
    assert_eq!(next.end, TurnEnd::Complete);
    assert_eq!(stage.conversations().len(), 1);
}

#[test]
fn stop_also_drops_the_messages_that_were_waiting() {
    let stage = Stage::new(json!({"turns": [
        {"replay": fixture("interrupted-turn.jsonl"), "hold_after_deltas": 6},
        {"text": "After the stop.", "pieces": 1},
    ]}));
    let talk = Talk::start(&stage.backend(), options());
    let first = talk.conversation.send("Count.");
    let waiting = talk.conversation.send("And then this.");
    let (mut seen, _) = talk.events.wait_until(PATIENCE, |event| {
        matches!(event, AssistantEvent::Text { .. })
    });
    talk.conversation.stop();
    seen.extend(talk.until_finished(first));
    assert_eq!(Answer::of(first, &seen).end, TurnEnd::Stopped);
    assert_eq!(Answer::of(waiting, &seen).end, TurnEnd::Stopped);
    // The waiting message was never given to the program.
    let after = talk.ask("A new question.");
    assert_eq!(after.text, "After the stop.");
    assert_eq!(stage.messages(), ["Count.", "A new question."]);
}

#[test]
fn stop_with_nothing_to_stop_does_nothing() {
    let stage = Stage::new(json!({"turns": [{"text": "Fine.", "pieces": 1}]}));
    let talk = Talk::start(&stage.backend(), options());
    talk.conversation.stop();
    assert_eq!(talk.ask("Hello?").text, "Fine.");
    talk.conversation.stop();
    assert_eq!(talk.ask("Again?").text, "Fine.");
    assert!(!stage.requests().contains(&"interrupt".to_string()));
}

#[test]
fn lines_that_are_not_json_and_lines_of_unknown_kinds_are_skipped() {
    let stage = Stage::new(json!({"turns": [
        {"replay": fixture("quick-turn.jsonl"), "noise": true},
        {"text": "Still here.", "pieces": 2, "noise": true},
    ]}));
    let talk = Talk::start(&stage.backend(), options());
    let answer = talk.ask("The code word?");
    assert_eq!(answer.text, "HERON");
    assert_eq!(answer.end, TurnEnd::Complete);
    assert_eq!(talk.ask("And now?").text, "Still here.");
    assert!(talk.conversation.is_alive());
}

#[test]
fn a_very_long_answer_arrives_whole() {
    let paragraph = "The tide went out and the tide came in, and nothing was said of it. ";
    let long = paragraph.repeat(30_000); // about two megabytes
    let stage = Stage::new(json!({"turns": [{"text": long, "pieces": 400}]}));
    let talk = Talk::start(&stage.backend(), options());
    let answer = talk.ask("Write a great deal.");
    assert_eq!(answer.end, TurnEnd::Complete);
    assert_eq!(answer.text.len(), long.len());
    assert!(answer.text == long);
    // The final line of the program repeats the two megabytes; they are not sent on.
    assert_eq!(answer.final_text, None);
    assert_eq!(talk.ask("And a short one?").end, TurnEnd::Complete);
}

#[test]
fn an_answer_without_pieces_arrives_as_one() {
    // A program that does not stream gives the whole message at once.
    let stage =
        Stage::new(json!({"turns": [{"text": "All at once.", "pieces": 1, "no_partials": true}]}));
    let talk = Talk::start(&stage.backend(), options());
    let answer = talk.ask("Go.");
    assert_eq!(answer.text, "All at once.");
    assert_eq!(answer.end, TurnEnd::Complete);
}

#[test]
fn prepare_starts_the_program_and_sends_it_nothing() {
    let stage = Stage::new(json!({"turns": [{"text": "Ready.", "pieces": 1}]}));
    let talk = Talk::start(&stage.backend(), options());
    talk.conversation.prepare();
    stage.wait_for_log("the conversation process", |stage| {
        stage.conversations().len() == 1
    });
    assert!(
        stage.received().is_empty(),
        "nothing leaves before the first message"
    );
    talk.conversation.prepare();
    assert_eq!(talk.ask("Now.").text, "Ready.");
    assert_eq!(
        stage.conversations().len(),
        1,
        "the prepared process answers"
    );
}

#[test]
fn closing_ends_the_process_and_finishes_what_was_open() {
    let stage = Stage::new(json!({"turns": [
        {"text": "Done.", "pieces": 1},
        {"silent": true},
    ]}));
    let talk = Talk::start(&stage.backend(), options());
    assert_eq!(talk.ask("One.").text, "Done.");
    let open = talk.conversation.send("This one is never answered.");
    let waiting = talk.conversation.send("Nor this.");
    talk.conversation.close();
    let events = talk.until_finished(waiting);
    assert_eq!(Answer::of(open, &events).end, TurnEnd::Stopped);
    assert_eq!(Answer::of(waiting, &events).end, TurnEnd::Stopped);
    assert!(!talk.conversation.is_alive());
    // A message to a closed conversation is still answered, with an error.
    let late = talk.conversation.send("Anyone?");
    let events = talk.until_finished(late);
    assert!(matches!(
        Answer::of(late, &events).end,
        TurnEnd::Failed(innerword_assistant::AssistantError::Exited { .. })
    ));
    // The stream ends when the conversation is gone.
    drop(talk.conversation);
    let deadline = Instant::now() + PATIENCE;
    while !talk.events.is_finished() {
        assert!(Instant::now() < deadline, "the stream never ended");
        let _ = talk.events.wait(Duration::from_millis(20));
    }
    // The process that was in the middle of a turn was not waited for: it was ended.
    stage.wait_until_processes_are_gone();
}

#[test]
fn dropping_a_conversation_closes_the_programs_input() {
    let stage = Stage::new(json!({"turns": [{"text": "Done.", "pieces": 1}]}));
    let talk = Talk::start(&stage.backend(), options());
    assert_eq!(talk.ask("One.").text, "Done.");
    drop(talk.conversation);
    stage.wait_for_log("the end of the input", |stage| {
        stage.notes().contains(&"input closed".to_string())
    });
    stage.wait_until_processes_are_gone();
}

#[test]
fn a_conversation_that_continues_an_earlier_one_begins_with_what_was_said() {
    let stage = Stage::new(json!({"turns": [{"echo": true, "pieces": 1}]}));
    let history = vec![
        HistoryMessage {
            role: Role::User,
            text: "A name for a lighthouse keeper?".into(),
        },
        HistoryMessage {
            role: Role::Assistant,
            text: "Mara Venn.".into(),
        },
    ];
    let talk = Talk::start(&stage.backend(), options().with_history(history));
    talk.ask("And her dog?");
    talk.ask("Shorter.");
    let messages = stage.messages();
    assert!(messages[0].contains("<conversation_so_far>"));
    assert!(messages[0].contains("<me>\nA name for a lighthouse keeper?\n</me>"));
    assert!(messages[0].contains("<you>\nMara Venn.\n</you>"));
    assert!(messages[0].ends_with("And her dog?"));
    // Only the first message of the process carries it.
    assert_eq!(messages[1], "Shorter.");
}

#[test]
fn several_conversations_run_side_by_side() {
    let stage = Stage::new(json!({"turns": [{"echo": true, "pieces": 3, "delay_ms": 5}]}));
    let claude = stage.backend();
    let one = Talk::start(&claude, options());
    let two = Talk::start(&claude, options());
    let first = one.conversation.send("for the first");
    let second = two.conversation.send("for the second");
    assert_eq!(
        Answer::of(first, &one.until_finished(first)).text,
        "for the first"
    );
    assert_eq!(
        Answer::of(second, &two.until_finished(second)).text,
        "for the second"
    );
    assert_eq!(stage.conversations().len(), 2);
    // The backend can be asked from any thread.
    let shared: std::sync::Arc<dyn Backend> = std::sync::Arc::new(claude);
    let handle = std::thread::spawn(move || shared.info().name);
    assert_eq!(handle.join().unwrap(), "Claude Code");
}

#[test]
fn tools_that_did_not_start_are_said_once() {
    // Recorded with a server command that does not exist: the program goes on without
    // the tools and says so in its first line.
    let stage = Stage::new(json!({"turns": [
        {"replay": fixture("tools-not-started-turn.jsonl")},
        {"replay": fixture("tools-not-started-turn.jsonl")},
    ]}));
    let tools = innerword_assistant::library_tools("/no/such/innerword", "/tmp/library");
    let talk = Talk::start(&stage.backend(), options().with_tools(Some(tools)));
    let first = talk.conversation.send("Who is Tam?");
    let mut events = talk.until_finished(first);
    let second = talk.conversation.send("And Mara?");
    events.extend(talk.until_finished(second));
    let notices: Vec<&str> = events
        .iter()
        .filter_map(|event| match event {
            AssistantEvent::Notice { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(notices.len(), 1, "{notices:?}");
    assert!(notices[0].starts_with("The assistant's tools did not start"));
    assert_eq!(
        Answer::of(second, &events).end,
        TurnEnd::Complete,
        "the conversation goes on"
    );
}
