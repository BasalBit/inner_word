//! The scripted backend the other crates test with.

use std::time::Duration;

use innerword_assistant::{
    AssistantError, AssistantEvent, Availability, Backend, ConversationOptions, FakeBackend,
    FakeReply, Mode, Selection, SignIn, ToolState, TurnEnd, Usage, event_channel, fake_catalog,
};

fn text_of(events: &[AssistantEvent]) -> String {
    events
        .iter()
        .filter_map(|event| match event {
            AssistantEvent::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_scripted_answer_streams_in_pieces_on_the_calling_thread() {
    let fake = FakeBackend::new();
    fake.reply("A **heron** is a wading bird.");
    let (sink, events) = event_channel();
    let conversation = fake.start(ConversationOptions::new("system"), sink);
    let turn = conversation.send("What is a heron?");
    // Everything is there when `send` returns: no thread, no waiting.
    let seen = events.drain();
    assert!(matches!(seen[0], AssistantEvent::Ready { .. }));
    assert_eq!(seen[1], AssistantEvent::Started { turn });
    let pieces = seen
        .iter()
        .filter(|event| matches!(event, AssistantEvent::Text { .. }))
        .count();
    assert_eq!(pieces, 6, "word by word");
    assert_eq!(text_of(&seen), "A **heron** is a wading bird.");
    assert_eq!(
        seen.last(),
        Some(&AssistantEvent::Finished {
            turn,
            text: Some("A **heron** is a wading bird.".to_string()),
            end: TurnEnd::Complete,
            usage: None,
        })
    );
    // What it was asked is on record.
    assert_eq!(fake.sent_texts(), ["What is a heron?"]);
    assert_eq!(fake.last_sent().as_deref(), Some("What is a heron?"));
    assert_eq!(fake.sent()[0].turn, turn);
    assert_eq!(fake.started().len(), 1);
    assert_eq!(fake.last_started().unwrap().system_prompt, "system");
}

#[test]
fn answers_are_used_in_order_and_then_the_fallback() {
    let fake = FakeBackend::new();
    fake.reply("first");
    fake.push(FakeReply::pieces(["se", "cond"]));
    let (sink, events) = event_channel();
    let conversation = fake.start(ConversationOptions::new("s"), sink);
    conversation.send("1");
    assert_eq!(text_of(&events.drain()), "first");
    conversation.send("2");
    assert_eq!(text_of(&events.drain()), "second");
    // Nothing scripted: the answer says what it answers.
    conversation.send("What now?");
    assert_eq!(text_of(&events.drain()), "Answer to: What now?");
    fake.set_fallback(FakeReply::text("Always this."));
    conversation.send("4");
    assert_eq!(text_of(&events.drain()), "Always this.");
}

#[test]
fn held_events_arrive_when_the_test_lets_them() {
    let fake = FakeBackend::new();
    fake.hold();
    fake.push(FakeReply::pieces(["One ", "two ", "three."]));
    let (sink, events) = event_channel();
    let conversation = fake.start(ConversationOptions::new("s"), sink);
    let turn = conversation.send("Count.");
    assert!(events.drain().is_empty());
    assert_eq!(fake.held(), 5, "started, three pieces, finished");
    assert_eq!(fake.advance(2), 2);
    assert_eq!(
        events.drain().last(),
        Some(&AssistantEvent::Started { turn })
    );
    assert_eq!(fake.advance(1), 1);
    assert_eq!(text_of(&events.drain()), "One ");
    assert_eq!(fake.finish(), 3);
    let rest = events.drain();
    assert_eq!(text_of(&rest), "two three.");
    assert!(matches!(
        rest.last(),
        Some(AssistantEvent::Finished {
            end: TurnEnd::Complete,
            ..
        })
    ));
    assert_eq!(fake.advance(5), 0, "nothing is left");
    // Still holding for the next message, until released.
    conversation.send("Again.");
    assert!(events.drain().is_empty());
    fake.release();
    assert!(!events.drain().is_empty());
    conversation.send("And again.");
    assert!(
        !events.drain().is_empty(),
        "after release answers arrive at once"
    );
}

#[test]
fn stopping_ends_the_open_answer_and_the_ones_waiting() {
    let fake = FakeBackend::new();
    fake.hold();
    fake.reply("a long answer that is never finished");
    let (sink, events) = event_channel();
    let conversation = fake.start(ConversationOptions::new("s"), sink);
    let open = conversation.send("One.");
    let waiting = conversation.send("Two.");
    fake.advance(3);
    events.drain();
    conversation.stop();
    assert_eq!(
        events.drain(),
        vec![
            AssistantEvent::Finished {
                turn: open,
                text: None,
                end: TurnEnd::Stopped,
                usage: None
            },
            AssistantEvent::Finished {
                turn: waiting,
                text: None,
                end: TurnEnd::Stopped,
                usage: None
            },
        ]
    );
    assert_eq!(fake.stops(), 1);
    assert_eq!(fake.held(), 0);
    assert!(conversation.is_alive(), "the conversation goes on");
    fake.release();
    conversation.send("Three.");
    assert!(matches!(
        events.drain().last(),
        Some(AssistantEvent::Finished {
            end: TurnEnd::Complete,
            ..
        })
    ));
}

#[test]
fn tool_activity_status_usage_and_failures_can_be_scripted() {
    let fake = FakeBackend::new();
    let usage = Usage {
        input_tokens: Some(10),
        output_tokens: Some(2),
        cost_usd: Some(0.5),
        duration_ms: Some(7),
    };
    fake.push(
        FakeReply::text("Tam is her brother.")
            .with_tool("read_document", "Reading Characters")
            .with_failed_tool("search_library", "Searching the library")
            .with_status("Thinking")
            .with_usage(usage.clone()),
    );
    fake.push(
        FakeReply::pieces(["Half an ans"]).then_fail(AssistantError::Network {
            detail: "gone".into(),
        }),
    );
    fake.fail(AssistantError::UsageLimit {
        resets_at: Some(1_800_000_000),
        detail: "limit".into(),
    });
    fake.push(FakeReply::text("only pieces").without_final_text());

    let (sink, events) = event_channel();
    let conversation = fake.start(ConversationOptions::new("s"), sink);
    conversation.send("Who?");
    let seen = events.drain();
    let shape: Vec<String> = seen
        .iter()
        .map(|event| match event {
            AssistantEvent::Ready { .. } => "ready".to_string(),
            AssistantEvent::Started { .. } => "started".to_string(),
            AssistantEvent::Status { text, .. } => format!("status {text}"),
            AssistantEvent::Tool { activity, .. } => {
                format!("{} {:?}", activity.description, activity.state)
            }
            AssistantEvent::Text { .. } => "text".to_string(),
            AssistantEvent::Finished { .. } => "finished".to_string(),
            other => format!("{other:?}"),
        })
        .collect();
    assert_eq!(
        shape,
        [
            "ready",
            "started",
            "status Thinking",
            "Reading Characters Running",
            "Reading Characters Done",
            "Searching the library Running",
            "Searching the library Failed",
            "text",
            "text",
            "text",
            "text",
            "finished",
        ]
    );
    let AssistantEvent::Tool {
        activity: first, ..
    } = &seen[3]
    else {
        unreachable!()
    };
    let AssistantEvent::Tool {
        activity: second, ..
    } = &seen[4]
    else {
        unreachable!()
    };
    assert_eq!(first.id, second.id, "the same call, in two states");
    assert_eq!(first.state, ToolState::Running);
    assert!(
        matches!(seen.last(), Some(AssistantEvent::Finished { usage: Some(reported), .. }) if *reported == usage)
    );

    conversation.send("Again?");
    let seen = events.drain();
    assert_eq!(text_of(&seen), "Half an ans");
    assert!(matches!(
        seen.last(),
        Some(AssistantEvent::Finished {
            text: None,
            end: TurnEnd::Failed(AssistantError::Network { .. }),
            ..
        })
    ));

    conversation.send("Once more?");
    let seen = events.drain();
    assert_eq!(text_of(&seen), "");
    assert!(matches!(
        seen.last(),
        Some(AssistantEvent::Finished {
            end: TurnEnd::Failed(AssistantError::UsageLimit {
                resets_at: Some(1_800_000_000),
                ..
            }),
            ..
        })
    ));

    conversation.send("Pieces?");
    assert!(matches!(
        events.drain().last(),
        Some(AssistantEvent::Finished {
            text: None,
            end: TurnEnd::Complete,
            ..
        })
    ));
}

#[test]
fn a_crash_fails_what_is_open_and_ends_every_conversation() {
    let fake = FakeBackend::new();
    fake.hold();
    let (sink, events) = event_channel();
    let conversation = fake.start(ConversationOptions::new("s"), sink);
    let turn = conversation.send("One.");
    fake.crash();
    assert!(matches!(
        events.drain().last(),
        Some(AssistantEvent::Finished { turn: failed, end: TurnEnd::Failed(AssistantError::Exited { code: Some(1), .. }), .. }) if *failed == turn
    ));
    assert!(!conversation.is_alive());
    assert_eq!(fake.alive(), 0);
    // A message to the dead conversation is answered with an error at once.
    conversation.send("Two.");
    assert!(matches!(
        events.drain().last(),
        Some(AssistantEvent::Finished {
            end: TurnEnd::Failed(AssistantError::Exited { .. }),
            ..
        })
    ));
    // A new conversation works.
    let (sink, events) = event_channel();
    let again = fake.start(ConversationOptions::new("s"), sink);
    fake.release();
    again.send("Three.");
    assert!(matches!(
        events.drain().last(),
        Some(AssistantEvent::Finished {
            end: TurnEnd::Complete,
            ..
        })
    ));
    assert_eq!(fake.alive(), 1);
}

#[test]
fn closing_and_dropping_are_counted_and_end_the_stream() {
    let fake = FakeBackend::new();
    let (sink, events) = event_channel();
    let conversation = fake.start(ConversationOptions::new("s"), sink);
    conversation.prepare();
    assert_eq!(fake.prepares(), 1);
    assert!(matches!(events.drain()[..], [AssistantEvent::Ready { .. }]));
    conversation.close();
    assert!(!conversation.is_alive());
    assert_eq!(fake.closes(), 1);
    drop(conversation);
    assert_eq!(fake.closes(), 1, "closed once");
    let (sink, events) = event_channel();
    drop(fake.start(ConversationOptions::new("s"), sink));
    assert_eq!(fake.closes(), 2, "dropping closes");
    assert_eq!(events.wait(Duration::from_millis(10)), None);
}

#[test]
fn availability_and_the_catalog_are_what_the_test_says() {
    let fake = FakeBackend::new();
    assert_eq!(fake.info().name, "Fake Agent");
    assert_eq!(fake.cached_availability(), None);
    let availability = fake.availability();
    assert!(availability.is_ready());
    assert_eq!(availability.account(), Some("Fake plan"));
    assert_eq!(availability.version(), Some("1.0.0"));
    assert_eq!(fake.availability_checks(), 1);
    fake.availability();
    assert_eq!(fake.availability_checks(), 1, "cached");
    fake.set_sign_in(SignIn::SignedOut);
    assert!(fake.recheck().is_signed_out());
    assert_eq!(fake.availability_checks(), 2);
    assert!(FakeBackend::not_installed().availability() == Availability::NotInstalled);
    assert!(FakeBackend::signed_out().availability().is_signed_out());

    // A message to a missing or signed-out agent fails as the real one does.
    for (fake, error) in [
        (FakeBackend::not_installed(), AssistantError::NotInstalled),
        (FakeBackend::signed_out(), AssistantError::NotSignedIn),
    ] {
        let (sink, events) = event_channel();
        let conversation = fake.start(ConversationOptions::new("s"), sink);
        conversation.send("Hello?");
        assert!(
            matches!(events.drain().last(), Some(AssistantEvent::Finished { end: TurnEnd::Failed(failed), .. }) if *failed == error)
        );
    }

    let catalog = fake.catalog();
    assert_eq!(catalog, fake_catalog());
    assert_eq!(
        catalog.default_for(Mode::Quick),
        Selection::named("swift", "low")
    );
    assert_eq!(
        catalog.default_for(Mode::Side),
        Selection::named("deep", "medium")
    );
    let mut bigger = fake_catalog();
    bigger.models.push(innerword_assistant::ModelOption {
        id: "new".into(),
        name: "New".into(),
        description: String::new(),
        efforts: None,
    });
    fake.set_refreshed_catalog(bigger.clone());
    assert_eq!(fake.catalog(), fake_catalog(), "not before it is asked for");
    assert_eq!(fake.refresh_catalog(), bigger);
    assert_eq!(fake.catalog(), bigger);
    assert_eq!(fake.catalog_refreshes(), 1);
}

#[test]
fn notices_reach_the_conversations_that_are_alive() {
    let fake = FakeBackend::new();
    let (sink, events) = event_channel();
    let _conversation = fake.start(ConversationOptions::new("s"), sink);
    fake.notice("You are close to the usage limit.");
    assert_eq!(
        events.drain(),
        vec![AssistantEvent::Notice {
            text: "You are close to the usage limit.".to_string()
        }]
    );
}
