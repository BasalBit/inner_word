//! The conversation: what is sent, how the answer arrives, back and forth,
//! stopping.

use gpui::TestAppContext;
use writer_assistant::{
    EffortChoice, FakeReply, MessageState, ModelChoice, QUICK_SYSTEM_PROMPT, Role, Selection,
    quick_prompt,
};
use writer_assistant_quick::QuickChatEvent;
use writer_core::{Block, BlockKind, Document, Position, TextRange};

use crate::support::*;

/// A document whose every part is recognisable in what is sent.
fn marked_document() -> Document {
    Document::from_blocks(vec![
        Block::with_text(BlockKind::heading(1), "TITLE-OF-THE-BOOK"),
        Block::paragraph("BEFORE-THE-SELECTION The quick brown fox AFTER-THE-SELECTION"),
        Block::paragraph("ANOTHER-PARAGRAPH of the document."),
    ])
}

#[gpui::test]
fn the_selection_and_the_question_are_all_that_is_sent(cx: &mut TestAppContext) {
    let (fx, cx) = open_quick(marked_document(), cx);
    select(&fx.editor, TextRange::in_block(1, 21, 40), cx);
    assert_eq!(selected_text(&fx.editor, cx), "The quick brown fox");

    fx.toggle(cx);
    // Opening sends nothing and starts nothing.
    assert!(fx.fake.started().is_empty());
    assert!(fx.fake.sent().is_empty());

    fx.fake.reply("A swift brown fox");
    fx.send("Make it livelier.", cx);

    // Exactly the engine's quick prompt of the selection and the question.
    let sent = fx.fake.sent_texts();
    assert_eq!(
        sent,
        vec![quick_prompt(
            "Make it livelier.",
            Some("The quick brown fox")
        )]
    );
    assert_eq!(
        sent[0],
        "This is the text I have selected in my document. My request below is about this \
         text.\n\n<selected_text>\nThe quick brown fox\n</selected_text>\n\nMake it livelier."
    );
    // Nothing of the rest of the document.
    for leak in [
        "TITLE-OF-THE-BOOK",
        "BEFORE-THE-SELECTION",
        "AFTER-THE-SELECTION",
        "ANOTHER-PARAGRAPH",
    ] {
        assert!(!sent[0].contains(leak), "{leak} was sent");
    }

    // One conversation: the quick chat's system prompt, no tools, no
    // history, the engine's suggestion for a quick answer.
    let started = fx.fake.started();
    assert_eq!(started.len(), 1);
    assert_eq!(started[0].system_prompt, QUICK_SYSTEM_PROMPT);
    assert_eq!(started[0].tools, None);
    assert!(started[0].history.is_empty());
    assert_eq!(started[0].working_dir, None);
    assert_eq!(started[0].selection, Selection::named("swift", "low"));
    for leak in ["TITLE-OF-THE-BOOK", "ANOTHER-PARAGRAPH", "quick brown"] {
        assert!(!started[0].system_prompt.contains(leak));
    }

    assert_eq!(fx.answers(cx), vec!["A swift brown fox"]);
}

#[gpui::test]
fn without_a_selection_the_question_goes_alone(cx: &mut TestAppContext) {
    let (fx, cx) = open_quick(marked_document(), cx);
    caret(&fx.editor, 1, 25, cx);

    fx.toggle(cx);
    fx.fake.reply("weary, drained, spent");
    fx.send("A word for very tired?", cx);

    assert_eq!(fx.fake.sent_texts(), vec!["A word for very tired?"]);
    let started = fx.fake.started();
    assert_eq!(started.len(), 1);
    assert!(started[0].history.is_empty());
    assert_eq!(started[0].tools, None);
    // The question is shown without a quote.
    let messages = fx.messages(cx);
    assert_eq!(messages[0].role, Role::User);
    assert_eq!(messages[0].text, "A word for very tired?");
    assert_eq!(messages[0].quote, None);
    assert_eq!(fx.quick.read_with(cx, |quick, _| quick.quote()), None);
}

#[gpui::test]
fn a_selection_over_paragraphs_is_sent_as_it_is(cx: &mut TestAppContext) {
    let (fx, cx) = open_quick(paragraphs(&["One two", "three four", "five six"]), cx);
    select(
        &fx.editor,
        TextRange::new(Position::new(0, 4), Position::new(2, 4)),
        cx,
    );
    fx.toggle(cx);
    fx.send("Shorter.", cx);
    assert_eq!(
        fx.fake.sent_texts(),
        vec![quick_prompt("Shorter.", Some("two\nthree four\nfive"))]
    );
}

#[gpui::test]
fn the_popover_quotes_the_selection_until_the_question_does(cx: &mut TestAppContext) {
    let long = "word ".repeat(80);
    let (fx, cx) = open_quick(paragraphs(&[&long]), cx);
    select(&fx.editor, TextRange::in_block(0, 0, long.len()), cx);

    fx.toggle(cx);
    let quote = fx
        .quick
        .read_with(cx, |quick, _| quick.quote())
        .expect("a quote");
    assert!(quote.starts_with("word word"));
    assert!(quote.ends_with('…'), "cut short: {quote:?}");
    assert!(quote.chars().count() <= 121);
    assert!(try_bounds("quick-chat-quote", cx).is_some());

    fx.send("Shorter.", cx);
    // The first question carries the selected text now.
    // (GPUI keeps the debug bounds of elements that are gone, so the row
    // itself cannot be asked for.)
    assert_eq!(fx.quick.read_with(cx, |quick, _| quick.quote()), None);
    assert_eq!(fx.messages(cx)[0].quote.as_deref(), Some(long.as_str()));
}

#[gpui::test]
fn the_keyboard_is_in_the_composer_and_enter_sends(cx: &mut TestAppContext) {
    let (fx, cx) = open_quick(paragraphs(&["The quick brown fox"]), cx);
    select(&fx.editor, TextRange::in_block(0, 4, 9), cx);

    fx.toggle(cx);
    assert!(fx.is_open(cx));
    assert!(fx.composer_focused(cx));
    assert!(!fx.editor_focused(cx));
    assert_eq!(fx.events(cx), vec![QuickChatEvent::Opened]);

    // Typing goes to the composer, not to the document.
    cx.simulate_input("Another word?");
    cx.run_until_parked();
    assert_eq!(fx.draft(cx), "Another word?");
    assert_eq!(text(&fx.editor, cx), "The quick brown fox");
    assert!(fx.fake.sent().is_empty(), "nothing is sent before enter");

    // shift-enter is a new line, enter sends.
    cx.simulate_keystrokes("shift-enter");
    cx.simulate_input("One only.");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(fx.draft(cx), "");
    assert_eq!(
        fx.fake.last_sent().unwrap(),
        quick_prompt("Another word?\nOne only.", Some("quick"))
    );
    // An empty composer sends nothing.
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(fx.fake.sent().len(), 1);
}

#[gpui::test]
fn the_answer_streams_in_above_the_composer(cx: &mut TestAppContext) {
    let (fx, cx) = open_quick(paragraphs(&["The quick brown fox"]), cx);
    select(&fx.editor, TextRange::in_block(0, 4, 9), cx);
    fx.toggle(cx);

    fx.fake.hold();
    fx.fake
        .push(FakeReply::pieces(["**swift**, ", "fleet, ", "*nimble*"]));
    fx.send("Another word?", cx);

    // Asked, nothing has arrived: the answer is awaited.
    let messages = fx.messages(cx);
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[1].state, MessageState::Streaming);
    assert_eq!(messages[1].text, "");
    assert!(fx.quick.read_with(cx, |quick, _| quick.is_answering()));
    assert!(
        try_bounds("assistant-stop", cx).is_some(),
        "Stop is offered"
    );

    fx.first_piece(cx);
    let answer = fx.last_answer_id(cx);
    assert_eq!(fx.answers(cx), vec!["**swift**, "]);
    assert_eq!(fx.shown_answer(answer, cx), "swift,");

    fx.fake.advance(1);
    cx.run_until_parked();
    assert_eq!(fx.shown_answer(answer, cx), "swift, fleet,");
    assert!(fx.quick.read_with(cx, |quick, _| quick.is_answering()));

    fx.fake.finish();
    cx.run_until_parked();
    assert_eq!(fx.answers(cx), vec!["**swift**, fleet, *nimble*"]);
    // Formatted text, not Markdown source.
    assert_eq!(fx.shown_answer(answer, cx), "swift, fleet, nimble");
    assert_eq!(fx.messages(cx)[1].state, MessageState::Done);
    assert!(!fx.quick.read_with(cx, |quick, _| quick.is_answering()));
    assert!(try_bounds("assistant-send", cx).is_some());

    // The answer is above the composer, inside the popover.
    let popover = fx.popover(cx);
    let conversation = bounds("quick-chat-conversation", cx);
    let composer = bounds("assistant-composer", cx);
    assert!(conversation.bottom() <= composer.top());
    assert!(popover.contains(&conversation.origin));
    assert!(popover.contains(&composer.center()));
    // The keyboard never left the composer.
    assert!(fx.composer_focused(cx));
}

#[gpui::test]
fn the_conversation_goes_back_and_forth(cx: &mut TestAppContext) {
    let (fx, cx) = open_quick(marked_document(), cx);
    select(&fx.editor, TextRange::in_block(1, 21, 40), cx);
    fx.toggle(cx);

    fx.fake.reply("A swift brown fox");
    fx.send("Make it livelier.", cx);
    fx.fake.reply("A swift, sly fox");
    fx.send("Sly, too.", cx);
    fx.fake.reply("Un renard vif et rusé");
    fx.send("In French.", cx);

    // One conversation; the selection went with the first question only,
    // the later ones are the questions as typed.
    assert_eq!(fx.fake.started().len(), 1);
    assert_eq!(
        fx.fake.sent_texts(),
        vec![
            quick_prompt("Make it livelier.", Some("The quick brown fox")),
            "Sly, too.".to_string(),
            "In French.".to_string(),
        ]
    );
    let roles: Vec<Role> = fx.messages(cx).iter().map(|message| message.role).collect();
    assert_eq!(
        roles,
        vec![
            Role::User,
            Role::Assistant,
            Role::User,
            Role::Assistant,
            Role::User,
            Role::Assistant
        ]
    );
    assert_eq!(
        fx.answers(cx),
        vec![
            "A swift brown fox",
            "A swift, sly fox",
            "Un renard vif et rusé"
        ]
    );
    assert!(fx.composer_focused(cx));
    assert!(fx.is_open(cx));
}

#[gpui::test]
fn escape_stops_the_answer_first_and_closes_then(cx: &mut TestAppContext) {
    let (fx, cx) = open_quick(paragraphs(&["The quick brown fox"]), cx);
    select(&fx.editor, TextRange::in_block(0, 4, 9), cx);
    fx.toggle(cx);

    fx.fake.hold();
    fx.fake
        .push(FakeReply::pieces(["swift, ", "fleet, ", "nimble"]));
    fx.send("Another word?", cx);
    fx.first_piece(cx);
    assert_eq!(fx.answers(cx), vec!["swift, "]);

    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    // Stopped, with what had arrived; the popover is still there.
    assert_eq!(fx.fake.stops(), 1);
    assert!(fx.is_open(cx));
    let messages = fx.messages(cx);
    assert_eq!(messages[1].state, MessageState::Stopped);
    assert_eq!(messages[1].text, "swift, ");
    assert!(!fx.quick.read_with(cx, |quick, _| quick.is_answering()));
    assert!(fx.composer_focused(cx));

    // The conversation goes on after a stop.
    fx.fake.release();
    fx.fake.reply("fleet");
    fx.send("Just one.", cx);
    assert_eq!(fx.answers(cx), vec!["swift, ", "fleet"]);
    assert_eq!(fx.fake.started().len(), 1);

    // With nothing being written, escape closes.
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(!fx.is_open(cx));
    assert!(fx.editor_focused(cx));
}

#[gpui::test]
fn the_stop_button_and_its_key_stop_the_answer(cx: &mut TestAppContext) {
    let (fx, cx) = open_quick(paragraphs(&["The quick brown fox"]), cx);
    fx.toggle(cx);

    fx.fake.hold();
    fx.fake.push(FakeReply::pieces(["one ", "two ", "three"]));
    fx.send("Count.", cx);
    fx.first_piece(cx);
    click("assistant-stop", cx);
    assert_eq!(fx.fake.stops(), 1);
    assert_eq!(fx.messages(cx)[1].state, MessageState::Stopped);
    assert!(fx.is_open(cx));
    assert!(fx.composer_focused(cx));

    fx.fake.push(FakeReply::pieces(["four ", "five"]));
    fx.send("Go on.", cx);
    fx.first_piece(cx);
    cx.simulate_keystrokes("cmd-.");
    cx.run_until_parked();
    assert_eq!(fx.fake.stops(), 2);
    assert_eq!(fx.messages(cx)[3].state, MessageState::Stopped);
    assert_eq!(fx.messages(cx)[3].text, "four ");
}

#[gpui::test]
fn closing_ends_the_conversation_and_opening_starts_a_fresh_one(cx: &mut TestAppContext) {
    let (fx, cx) = open_quick(paragraphs(&["The quick brown fox", "jumps over"]), cx);
    select(&fx.editor, TextRange::in_block(0, 4, 9), cx);
    fx.toggle(cx);
    fx.fake.reply("swift");
    fx.send("Another word?", cx);
    cx.simulate_input("a half-typed quest");
    cx.run_until_parked();
    assert_eq!(fx.fake.closes(), 0);

    fx.toggle(cx);
    assert!(!fx.is_open(cx));
    // The agent's conversation was closed with the popover.
    assert_eq!(fx.fake.closes(), 1);
    assert_eq!(fx.fake.alive(), 0);

    select(&fx.editor, TextRange::in_block(1, 0, 5), cx);
    fx.toggle(cx);
    assert!(fx.messages(cx).is_empty(), "no messages of the last chat");
    assert_eq!(fx.draft(cx), "", "no draft of the last chat");
    fx.fake.reply("leaps");
    fx.send("Another word?", cx);
    // A new conversation, about the new selection, without history.
    let started = fx.fake.started();
    assert_eq!(started.len(), 2);
    assert!(started[1].history.is_empty());
    assert_eq!(
        fx.fake.last_sent().unwrap(),
        quick_prompt("Another word?", Some("jumps"))
    );
    assert_eq!(fx.answers(cx), vec!["leaps"]);
}

#[gpui::test]
fn a_model_chosen_in_the_popover_is_used_and_reported(cx: &mut TestAppContext) {
    let (fx, cx) = open_quick(paragraphs(&["The quick brown fox"]), cx);
    select(&fx.editor, TextRange::in_block(0, 4, 9), cx);
    fx.toggle(cx);
    fx.fake.reply("swift");
    fx.send("Another word?", cx);

    // The model picker, with the mouse.
    click("assistant-model", cx);
    assert!(menu_is_open(cx));
    assert_eq!(
        menu_labels(cx),
        vec!["Fake Agent's default", "-", "Swift", "Deep"]
    );
    choose("Deep", cx);
    assert!(
        fx.is_open(cx),
        "choosing from a menu is not a press outside"
    );
    assert!(fx.composer_focused(cx));
    let chosen = Selection {
        model: ModelChoice::named("deep"),
        effort: EffortChoice::named("low"),
    };
    assert_eq!(
        fx.events(cx).last(),
        Some(&QuickChatEvent::SelectionChanged(chosen.clone()))
    );
    assert_eq!(
        fx.quick.read_with(cx, |quick, _| quick.selection().clone()),
        chosen
    );

    // The effort picker, from the keyboard.
    cx.simulate_keystrokes("cmd-alt-e");
    cx.run_until_parked();
    assert!(menu_is_open(cx));
    assert_eq!(
        menu_labels(cx),
        vec!["Fake Agent's default", "-", "Low", "Medium", "High"]
    );
    choose("High", cx);
    let chosen = Selection::named("deep", "high");
    assert_eq!(
        fx.events(cx).last(),
        Some(&QuickChatEvent::SelectionChanged(chosen.clone()))
    );

    // The next question goes to the chosen model; the conversation is
    // carried over with what was said.
    fx.fake.reply("fleet");
    fx.send("One more.", cx);
    let started = fx.fake.started();
    assert_eq!(started.len(), 2);
    assert_eq!(started[1].selection, chosen);
    assert_eq!(started[1].history.len(), 2);
    assert_eq!(
        started[1].history[0].text,
        quick_prompt("Another word?", Some("quick"))
    );
    assert_eq!(started[1].history[1].text, "swift");
    assert_eq!(fx.fake.last_sent().unwrap(), "One more.");

    // The choice outlives the popover.
    fx.toggle(cx);
    fx.toggle(cx);
    fx.send("Again.", cx);
    assert_eq!(fx.fake.last_started().unwrap().selection, chosen);
}

#[gpui::test]
fn the_app_supplies_the_model_and_effort(cx: &mut TestAppContext) {
    let (fx, cx) = open_quick(paragraphs(&["The quick brown fox"]), cx);
    fx.quick.update(cx, |quick, cx| {
        quick.set_selection(Selection::named("deep", "medium"), cx)
    });
    fx.toggle(cx);
    fx.send("Hello?", cx);
    assert_eq!(
        fx.fake.last_started().unwrap().selection,
        Selection::named("deep", "medium")
    );
    // What the app set is not reported back to it.
    assert!(
        !fx.events(cx)
            .iter()
            .any(|event| matches!(event, QuickChatEvent::SelectionChanged(_)))
    );
    // The pickers show it.
    let quick = fx.quick.clone();
    let line = cx.update(|_, cx| quick.read(cx).describe(cx)).unwrap();
    assert!(line.contains("model=\"Deep\""), "{line}");
    assert!(line.contains("effort=\"Medium\""), "{line}");
}
