mod support;

// The engine without a process: the scripted backend, the conversation model, the
// context of a message.
mod chat;
mod context;
mod drawing;
mod events;
mod fake;
mod tools;
mod transcript;

// The Claude Code backend against the stand-in program and the recordings.
mod availability;
mod conversation;
mod failures;
mod isolation;
mod selection;
mod session;
mod stream;
