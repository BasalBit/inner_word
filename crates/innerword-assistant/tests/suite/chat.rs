//! A chat as a view holds it, over the scripted backend: what the other crates build on.

use std::sync::Arc;

use innerword_assistant::{
    AssistantError, Attachment, AttachmentKind, Backend, ChatSession, FakeBackend, FakeReply,
    MessageState, Mode, OpenDocument, OpenKind, Role, Selection, ToolState, Transcript,
    library_tools, privacy_notice,
};

/// Applies everything that is waiting in the chat's stream.
fn pump(chat: &mut ChatSession) -> bool {
    let events = chat.events();
    let mut changed = false;
    for event in events.drain() {
        changed |= chat.apply(event);
    }
    changed
}

fn quick(fake: &FakeBackend) -> ChatSession {
    ChatSession::quick(Arc::new(fake.clone()))
}

fn document() -> OpenDocument {
    OpenDocument {
        id: Some(12),
        title: "Chapter 2".into(),
        path: "The Salt Road/Chapter 2".into(),
        markdown: "The heron stood in the shallows.".into(),
        kind: OpenKind::Text,
    }
}

#[test]
fn nothing_is_started_and_nothing_is_sent_before_the_first_message() {
    let fake = FakeBackend::new();
    let chat = quick(&fake);
    assert_eq!(chat.mode(), Mode::Quick);
    assert!(chat.transcript().is_empty());
    assert!(!chat.is_answering());
    assert_eq!(
        chat.selection(),
        &Selection::named("swift", "low"),
        "the default of the mode"
    );
    assert!(fake.started().is_empty());
    assert!(fake.sent().is_empty());
    assert_eq!(
        fake.availability_checks(),
        0,
        "making a chat asks nothing of the agent"
    );
}

#[test]
fn a_back_and_forth_in_the_quick_chat() {
    let fake = FakeBackend::new();
    fake.reply("exhausted, drained, *spent*");
    fake.reply("**spent**");
    let mut chat = quick(&fake);
    let question = chat.ask_quick("  Another word for this?  ", Some("very tired"));
    assert!(chat.is_answering());
    assert!(pump(&mut chat));
    assert!(!chat.is_answering());
    chat.ask_quick("The shortest of them?", None);
    pump(&mut chat);

    let messages = &chat.transcript().messages;
    assert_eq!(messages.len(), 4);
    assert_eq!(messages[0].id, question);
    assert_eq!(
        messages[0].text, "Another word for this?",
        "shown as typed, trimmed"
    );
    assert_eq!(messages[0].quote.as_deref(), Some("very tired"));
    assert_eq!(messages[1].text, "exhausted, drained, *spent*");
    assert_eq!(messages[1].state, MessageState::Done);
    assert_eq!(
        messages[1].selection,
        Some(Selection::named("swift", "low"))
    );
    assert_eq!(messages[2].quote, None);
    assert_eq!(messages[3].text, "**spent**");
    // One conversation; the second question went alone, as it was typed.
    assert_eq!(fake.started().len(), 1);
    assert_eq!(
        fake.started()[0].system_prompt,
        innerword_assistant::QUICK_SYSTEM_PROMPT
    );
    assert!(
        fake.started()[0].tools.is_none(),
        "the quick chat has no tools"
    );
    assert!(fake.started()[0].history.is_empty());
    let sent = fake.sent_texts();
    assert!(sent[0].contains("<selected_text>\nvery tired\n</selected_text>"));
    assert_eq!(sent[1], "The shortest of them?");
    assert_eq!(chat.model_in_use(), Some("fake-model"));
}

#[test]
fn an_answer_is_seen_while_it_is_written_and_can_be_stopped() {
    let fake = FakeBackend::new();
    fake.hold();
    fake.push(
        FakeReply::pieces(["One, ", "two, ", "three."])
            .with_tool("read_document", "Reading Chapter 2"),
    );
    let mut chat = quick(&fake);
    chat.ask_quick("Count.", None);
    assert!(!pump(&mut chat), "nothing has arrived yet");
    assert_eq!(chat.transcript().messages[1].state, MessageState::Streaming);
    assert_eq!(chat.transcript().messages[1].text, "");

    fake.advance(3); // ready, started, the tool starts
    pump(&mut chat);
    assert_eq!(
        chat.transcript().messages[1].current_activity(),
        Some("Reading Chapter 2")
    );
    fake.advance(2); // the tool is done, the first piece
    pump(&mut chat);
    assert_eq!(chat.transcript().messages[1].text, "One, ");
    assert_eq!(chat.transcript().messages[1].current_activity(), None);
    assert!(chat.is_answering());

    chat.stop();
    pump(&mut chat);
    let answer = &chat.transcript().messages[1];
    assert_eq!(answer.state, MessageState::Stopped);
    assert_eq!(answer.text, "One, ", "what had arrived stays");
    assert_eq!(answer.activity[0].state, ToolState::Done);
    assert!(!chat.is_answering());
    assert_eq!(fake.stops(), 1);
    // The conversation goes on in the same conversation with the agent.
    fake.release();
    chat.ask_quick("And on?", None);
    pump(&mut chat);
    assert_eq!(chat.transcript().messages[3].state, MessageState::Done);
    assert_eq!(fake.started().len(), 1);
}

#[test]
fn a_failed_answer_shows_its_error_and_the_next_message_is_answered() {
    let fake = FakeBackend::new();
    fake.fail(AssistantError::UsageLimit {
        resets_at: None,
        detail: "You've hit your limit".into(),
    });
    fake.reply("Here again.");
    let mut chat = quick(&fake);
    chat.ask_quick("Hello?", None);
    pump(&mut chat);
    let failed = &chat.transcript().messages[1];
    assert!(matches!(
        failed.state,
        MessageState::Failed(AssistantError::UsageLimit { .. })
    ));
    let text = failed.error().unwrap().text(&fake.info());
    assert_eq!(
        text.headline,
        "You have reached the usage limit of your Fake Agent plan."
    );
    assert_eq!(text.advice, "Fake Agent says: You've hit your limit");
    chat.ask_quick("Hello again?", None);
    pump(&mut chat);
    assert_eq!(chat.transcript().messages[3].text, "Here again.");
    assert_eq!(
        fake.started().len(),
        1,
        "an error of the service does not end the conversation"
    );
}

#[test]
fn another_model_starts_the_agent_anew_and_tells_it_what_was_said() {
    let fake = FakeBackend::new();
    fake.reply("Mara Venn.");
    fake.reply("Pilot.");
    let mut chat = quick(&fake);
    chat.ask_quick("A name for a lighthouse keeper?", Some("the keeper"));
    pump(&mut chat);
    chat.set_selection(Selection::named("deep", "high"));
    assert_eq!(chat.selection(), &Selection::named("deep", "high"));
    assert_eq!(
        fake.started().len(),
        1,
        "nothing happens until the next message"
    );
    chat.ask_quick("And her dog?", None);
    pump(&mut chat);

    let started = fake.started();
    assert_eq!(started.len(), 2);
    assert_eq!(started[1].selection, Selection::named("deep", "high"));
    // The new conversation is told the old one: the question as the agent saw it (with
    // its selection) and the answer.
    assert_eq!(started[1].history.len(), 2);
    assert_eq!(started[1].history[0].role, Role::User);
    assert!(
        started[1].history[0]
            .text
            .contains("<selected_text>\nthe keeper\n</selected_text>")
    );
    assert_eq!(started[1].history[1].text, "Mara Venn.");
    assert_eq!(fake.closes(), 1, "the old conversation is closed");
    // The answers say which model and effort they were asked with.
    let messages = &chat.transcript().messages;
    assert_eq!(
        messages[1].selection,
        Some(Selection::named("swift", "low"))
    );
    assert_eq!(
        messages[3].selection,
        Some(Selection::named("deep", "high"))
    );
    // Choosing what is already chosen does nothing.
    chat.set_selection(Selection::named("deep", "high"));
    chat.ask_quick("Shorter?", None);
    pump(&mut chat);
    assert_eq!(fake.started().len(), 2);
}

#[test]
fn an_agent_that_can_change_while_it_runs_is_told_and_not_restarted() {
    let fake = FakeBackend::new();
    fake.set_live_selection(true);
    let mut chat = quick(&fake);
    chat.ask_quick("One.", None);
    pump(&mut chat);
    chat.set_selection(Selection::named("deep", "medium"));
    chat.ask_quick("Two.", None);
    pump(&mut chat);
    assert_eq!(fake.started().len(), 1);
    assert_eq!(fake.selections(), [(0, Selection::named("deep", "medium"))]);
    assert_eq!(
        chat.transcript().messages[3].selection,
        Some(Selection::named("deep", "medium"))
    );
    // Before the first message there is nothing to tell: the conversation starts with it.
    let fake = FakeBackend::new();
    fake.set_live_selection(true);
    let mut chat = quick(&fake);
    chat.set_selection(Selection::named("deep", "high"));
    chat.ask_quick("One.", None);
    assert!(fake.selections().is_empty());
    assert_eq!(
        fake.started()[0].selection,
        Selection::named("deep", "high")
    );
}

#[test]
fn a_model_changed_while_an_answer_is_written_leaves_that_answer_alone() {
    let fake = FakeBackend::new();
    fake.hold();
    fake.reply("The first answer.");
    fake.reply("The second answer.");
    let mut chat = quick(&fake);
    chat.ask_quick("One.", None);
    fake.advance(3);
    pump(&mut chat);
    chat.set_selection(Selection::named("deep", "medium"));
    chat.ask_quick("Two.", None);
    // The first conversation is kept until its answer has finished.
    assert_eq!(fake.started().len(), 2);
    assert_eq!(fake.closes(), 0);
    fake.finish();
    pump(&mut chat);
    let messages = &chat.transcript().messages;
    assert_eq!(messages[1].text, "The first answer.");
    assert_eq!(messages[1].state, MessageState::Done);
    assert_eq!(
        messages[1].selection,
        Some(Selection::named("swift", "low"))
    );
    assert_eq!(messages[3].text, "The second answer.");
    assert_eq!(
        messages[3].selection,
        Some(Selection::named("deep", "medium"))
    );
    assert_eq!(fake.closes(), 1, "then it is closed");
}

#[test]
fn after_the_agent_died_the_next_message_starts_it_again() {
    let fake = FakeBackend::new();
    fake.reply("First.");
    let mut chat = quick(&fake);
    chat.ask_quick("One.", None);
    pump(&mut chat);
    fake.crash();
    pump(&mut chat);
    fake.reply("Second.");
    chat.ask_quick("Two.", None);
    pump(&mut chat);
    assert_eq!(fake.started().len(), 2);
    assert_eq!(fake.started()[1].history.len(), 2);
    assert_eq!(chat.transcript().messages[3].text, "Second.");
    // A crash in the middle of an answer fails it, and what had arrived stays.
    fake.hold();
    fake.reply("Half of an answer that never ends");
    chat.ask_quick("Three.", None);
    fake.advance(2);
    pump(&mut chat);
    fake.crash();
    pump(&mut chat);
    let failed = chat.transcript().messages.last().unwrap();
    assert!(matches!(
        failed.state,
        MessageState::Failed(AssistantError::Exited { .. })
    ));
    assert_eq!(failed.text, "Half ");
    assert!(failed.error().unwrap().may_retry());
}

#[test]
fn the_side_chat_has_the_document_the_attachments_and_the_tools() {
    let fake = FakeBackend::new();
    let tools = library_tools("/app/innerword", "/data/library");
    let mut chat = ChatSession::side(Arc::new(fake.clone()), Some(tools));
    assert_eq!(chat.mode(), Mode::Side);
    assert!(chat.has_tools());
    assert_eq!(chat.selection(), &Selection::named("deep", "medium"));
    assert!(chat.attach(Attachment::folder(3, "Part two", "The Salt Road/Part two")));
    assert!(!chat.attach(Attachment::folder(3, "Part two", "The Salt Road/Part two")));
    assert_eq!(chat.attachments().len(), 1);

    let doc = document();
    chat.ask_side("Is the opening slow?", Some(&doc));
    pump(&mut chat);
    chat.ask_side("And the ending?", Some(&doc));
    pump(&mut chat);
    assert!(chat.detach(AttachmentKind::Folder, 3));
    chat.ask_side("Without the folder?", Some(&doc));
    pump(&mut chat);

    let started = &fake.started()[0];
    assert_eq!(started.system_prompt, innerword_assistant::SIDE_SYSTEM_PROMPT);
    let server = started
        .tools
        .as_ref()
        .expect("the side chat has the library's server");
    assert_eq!(server.name, "inner-word");
    assert_eq!(server.args, ["--library", "/data/library", "mcp"]);
    assert!(server.allowed.contains(&"read_document".to_string()));
    assert!(server.denied.contains(&"edit_document".to_string()));
    // The activity line names what the conversation knows by id.
    let describe = server.describe.as_ref().unwrap();
    assert_eq!(
        describe("read_document", &serde_json::json!({"document": "doc:12"})).as_deref(),
        Some("Reading Chapter 2")
    );
    assert_eq!(
        describe("list_library", &serde_json::json!({"folder": "folder:3"})).as_deref(),
        Some("Looking through Part two")
    );

    let sent = fake.sent_texts();
    assert!(sent[0].contains("<open_document title=\"Chapter 2\""));
    assert!(sent[0].contains("- folder folder:3"));
    assert!(sent[1].contains("has not changed since my last message"));
    assert!(!sent[1].contains("<attached>"));
    assert!(sent[2].contains("Nothing from the library is attached any more"));
    // The user's messages show what they typed and what was attached then.
    let messages = &chat.transcript().messages;
    assert_eq!(messages[0].text, "Is the opening slow?");
    assert_eq!(messages[0].attachments.len(), 1);
    assert_eq!(messages[4].attachments.len(), 0);
}

#[test]
fn without_the_writer_tool_the_side_chat_works_on_the_open_document_alone() {
    let fake = FakeBackend::new();
    let mut chat = ChatSession::side(Arc::new(fake.clone()), None);
    assert!(!chat.has_tools());
    chat.attach(Attachment::document(14, "Characters", "Notes/Characters"));
    chat.ask_side("Who is Tam?", Some(&document()));
    let started = &fake.started()[0];
    assert!(started.tools.is_none());
    assert!(started.system_prompt.contains("you have no tools"));
    assert!(
        fake.last_sent()
            .unwrap()
            .contains("you cannot read them now")
    );
    // Tools that appear later (the tool was found) are used from the next message on.
    pump(&mut chat);
    chat.set_tools(Some(library_tools("/app/innerword", "/data/library")));
    chat.ask_side("And now?", Some(&document()));
    assert_eq!(fake.started().len(), 2);
    assert!(fake.started()[1].tools.is_some());
    assert!(
        fake.last_sent().unwrap().contains("<open_document"),
        "the new conversation is given the document"
    );
}

#[test]
fn clear_keeps_what_is_attached_and_a_new_conversation_does_not() {
    let fake = FakeBackend::new();
    let mut chat = ChatSession::side(Arc::new(fake.clone()), None);
    chat.attach(Attachment::document(14, "Characters", "Notes/Characters"));
    chat.ask_side("One.", Some(&document()));
    pump(&mut chat);
    chat.clear();
    assert!(chat.transcript().is_empty());
    assert_eq!(chat.attachments().len(), 1);
    assert_eq!(fake.closes(), 1);
    chat.ask_side("Two.", Some(&document()));
    pump(&mut chat);
    assert_eq!(fake.started().len(), 2);
    assert!(
        fake.started()[1].history.is_empty(),
        "a cleared conversation is not carried over"
    );
    assert!(
        fake.last_sent().unwrap().contains("<open_document"),
        "and the document goes again"
    );
    chat.start_new();
    assert!(chat.transcript().is_empty());
    assert!(chat.attachments().is_empty());
}

#[test]
fn clearing_in_the_middle_of_an_answer_drops_what_arrives_afterwards() {
    let fake = FakeBackend::new();
    fake.hold();
    fake.reply("An answer that is cleared away.");
    let mut chat = quick(&fake);
    chat.ask_quick("One.", None);
    fake.advance(3);
    chat.clear();
    // The pieces that were already under way find no answer to go into.
    pump(&mut chat);
    assert!(chat.transcript().is_empty());
    assert!(!chat.is_answering());
    fake.release();
    fake.reply("Fresh.");
    chat.ask_quick("Two.", None);
    pump(&mut chat);
    assert_eq!(chat.transcript().messages.len(), 2);
    assert_eq!(chat.transcript().messages[1].text, "Fresh.");
}

#[test]
fn each_document_keeps_its_own_conversation() {
    let fake = FakeBackend::new();
    let mut chat = ChatSession::side(Arc::new(fake.clone()), None);
    chat.ask_side("About chapter two.", Some(&document()));
    pump(&mut chat);
    // Another document is opened: its stored conversation takes the place of this one.
    let mut other = Transcript::new();
    other.push_user("About the preface.", None);
    other.attach(Attachment::document(20, "Preface notes", "Notes/Preface"));
    let chapter_two = chat.replace_transcript(other);
    assert_eq!(chapter_two.messages.len(), 2);
    assert_eq!(chat.transcript().messages[0].text, "About the preface.");
    assert_eq!(chat.attachments().len(), 1);
    assert_eq!(
        fake.closes(),
        1,
        "the conversation of the first document ends"
    );
    let preface = OpenDocument {
        id: Some(1),
        title: "Preface".into(),
        path: "Preface".into(),
        markdown: "It began with salt.".into(),
        kind: OpenKind::Text,
    };
    chat.ask_side("More about it.", Some(&preface));
    pump(&mut chat);
    let started = fake.last_started().unwrap();
    assert_eq!(started.history.len(), 1);
    assert_eq!(started.history[0].text, "About the preface.");
    assert!(fake.last_sent().unwrap().contains("It began with salt."));
    // And back.
    let preface_talk = chat.replace_transcript(chapter_two);
    assert_eq!(preface_talk.messages.len(), 3);
    assert_eq!(chat.transcript().messages[0].text, "About chapter two.");
    assert!(chat.attachments().is_empty());
}

#[test]
fn a_notice_of_the_agent_is_kept_until_it_is_dismissed() {
    let fake = FakeBackend::new();
    let mut chat = quick(&fake);
    chat.ask_quick("One.", None);
    pump(&mut chat);
    assert_eq!(chat.notice(), None);
    fake.notice("You are close to the usage limit.");
    assert!(pump(&mut chat));
    assert_eq!(chat.notice(), Some("You are close to the usage limit."));
    chat.dismiss_notice();
    assert_eq!(chat.notice(), None);
}

#[test]
fn preparing_starts_the_agent_without_sending_anything() {
    let fake = FakeBackend::new();
    let mut chat = quick(&fake);
    chat.prepare();
    assert_eq!(fake.started().len(), 1);
    assert_eq!(fake.prepares(), 1);
    assert!(fake.sent().is_empty());
    chat.ask_quick("One.", None);
    assert_eq!(fake.started().len(), 1, "the prepared conversation answers");
}

#[test]
fn closing_and_dropping_a_chat_end_its_conversation() {
    let fake = FakeBackend::new();
    fake.hold();
    let mut chat = quick(&fake);
    chat.ask_quick("One.", None);
    chat.close();
    assert_eq!(fake.closes(), 1);
    assert!(!chat.is_answering(), "the open answer is stopped");
    assert_eq!(chat.transcript().messages[1].state, MessageState::Stopped);
    fake.release();
    chat.ask_quick("Two.", None);
    assert_eq!(
        fake.started().len(),
        2,
        "the next message starts the agent again"
    );
    assert_eq!(fake.started()[1].history.len(), 1);
    drop(chat);
    assert_eq!(fake.closes(), 2);
    assert_eq!(fake.alive(), 0);
}

#[test]
fn the_first_notice_says_where_the_text_goes() {
    let fake = FakeBackend::new();
    let quick = privacy_notice(&fake.info(), Mode::Quick);
    assert_eq!(
        quick,
        "Your question and the text you have selected are sent to Fake Vendor through your Fake Agent \
         sign-in, and each message counts against your Fake Agent plan. Nothing is sent before you send a message."
    );
    let side = privacy_notice(&fake.info(), Mode::Side);
    assert!(side.contains("the open document"));
    assert!(side.contains("what the assistant reads from your library"));
}
