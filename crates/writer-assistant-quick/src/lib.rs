//! The quick chat of The Writer's assistant: a small popover at the
//! selection or the caret, for a word, a sentence or a paragraph.
//!
//! This first part of the crate is what the popover does to the text and
//! where it stands; the popover itself builds on `writer-assistant` (the
//! engine) and `writer-assistant-ui` (the shared chat components).
//!
//! | Module | Contents |
//! |---|---|
//! | [`capture`] | the selection as it was when the chat opened, and where it is now |
//! | [`apply`] | Replace Selection and Insert Below: an answer (Markdown) into the text, one undo step |
//! | [`place`] | where the popover goes: beside its text, inside the editor's viewport |

pub mod apply;
pub mod capture;
pub mod place;

pub use capture::Capture;
