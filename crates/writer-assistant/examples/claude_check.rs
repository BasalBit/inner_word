//! A check of the Claude Code backend against the real program on this Mac.
//!
//! Without arguments it only looks: where the program is, its version, whether the
//! user is signed in, and which models it offers. None of that sends a message or
//! costs anything.
//!
//! ```text
//! CARGO_TARGET_DIR=target-pure cargo run -p writer-assistant --example claude_check
//! ```
//!
//! With `--ask` it also sends ONE short question through the quick chat, on the
//! cheapest model at low effort. **That spends a little of the user's own Claude Code
//! allowance.** No test runs this; it is here so that a person can confirm, after an
//! update of Claude Code, that the assistant still talks to it.
//!
//! ```text
//! CARGO_TARGET_DIR=target-pure cargo run -p writer-assistant --example claude_check -- --ask
//! ```

use std::sync::Arc;
use std::time::{Duration, Instant};

use writer_assistant::{
    AssistantEvent, Availability, Backend, ChatSession, ClaudeCode, MessageState, Mode, Selection,
};

fn main() {
    let ask = std::env::args().any(|arg| arg == "--ask");
    let claude = ClaudeCode::new();
    let info = claude.info();

    let started = Instant::now();
    let availability = claude.availability();
    println!("looked in {:?}", started.elapsed());
    let Availability::Installed(installation) = &availability else {
        println!("{} is not installed. {}", info.name, info.install_help);
        return;
    };
    println!("program:  {}", installation.program.display());
    println!(
        "version:  {}",
        installation.version.as_deref().unwrap_or("unknown")
    );
    println!("sign-in:  {:?}", installation.sign_in);
    if !availability.is_ready() {
        println!("{}", info.sign_in_help);
        return;
    }

    let started = Instant::now();
    let catalog = claude.refresh_catalog();
    println!(
        "catalog ({:?}, asked in {:?}):",
        catalog.source,
        started.elapsed()
    );
    for model in &catalog.models {
        println!(
            "  {:8} {:14} {}  efforts: {}",
            model.id,
            model.name,
            model.description,
            model
                .efforts
                .as_ref()
                .map(|levels| levels.join(","))
                .unwrap_or_else(|| "all".into())
        );
    }
    println!("  default label: {}", catalog.agent_default_label);
    println!("  quick: {:?}", catalog.default_for(Mode::Quick));
    println!("  side:  {:?}", catalog.default_for(Mode::Side));

    if !ask {
        println!("(no message was sent; pass --ask to send one short question)");
        return;
    }

    let mut chat =
        ChatSession::quick(Arc::new(claude)).with_selection(Selection::named("haiku", "low"));
    let events = chat.events();
    let started = Instant::now();
    chat.ask_quick("Another word for this, one word only.", Some("very tired"));
    let mut first_text = None;
    while chat.is_answering() {
        let Some(event) = events.wait(Duration::from_secs(120)) else {
            println!("no answer within two minutes");
            break;
        };
        match &event {
            AssistantEvent::Text { .. } if first_text.is_none() => {
                first_text = Some(started.elapsed())
            }
            AssistantEvent::Ready { model } => println!("model:    {model:?}"),
            AssistantEvent::Status { text, .. } => println!("status:   {text}"),
            AssistantEvent::Notice { text } => println!("notice:   {text}"),
            _ => {}
        }
        chat.apply(event);
    }
    let answer = chat
        .transcript()
        .last_answer()
        .expect("an answer was opened");
    println!("answer:   {:?}", answer.text);
    match &answer.state {
        MessageState::Failed(error) => println!("failed:   {}", error.text(&info).line()),
        state => println!("state:    {state:?}"),
    }
    println!("usage:    {:?}", answer.usage);
    println!(
        "first text after {first_text:?}, done after {:?}",
        started.elapsed()
    );
}
