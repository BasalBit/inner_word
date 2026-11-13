//! Headless tests (`#[gpui::test]`). They run on GPUI's test platform: no
//! real fonts, every glyph is 0.6 × font size wide, so geometry is
//! deterministic but not real. The test clock only moves when a test
//! advances it, so timers (caret blink, typing pause, background layout,
//! drag scrolling) fire exactly when a test says so.
//!
//! - `slice`: typing and keys through the real dispatch path.
//! - `seams`: the public seams other crates build on.
//! - `editing`: every editing action from the keyboard, clipboard, undo.
//! - `movement`: caret movement over wrapped rows, hit testing round trips.
//! - `ime`: platform text input with marked text, UTF-16 ranges.
//! - `mouse`: clicks, drags, checkboxes, links, wheel, scroll bar.
//! - `pages`: pagination invariants and page view.
//! - `print`: the print pass: the pages page view has, as data for paper,
//!   and a PDF made of them.
//! - `rebinding`: the keymap cleared and bound again with other keys.
//! - `layout`: the lazy layout on a book-length document, with timings.
//! - `vim_seams`: layout answers inside a mutation, the typed-text report,
//!   the caret override.
//! - `spelling`: the spelling marks a checker sets: following edits,
//!   queries, drawing, the context menu request, the cost in a book.
//! - `zoom`: the zoom of the two views: the same lines and pages at every
//!   zoom, everything that is drawn at or found from a position, keeping
//!   the place, sideways scrolling.

mod editing;
mod ime;
mod layout;
mod mouse;
mod movement;
mod pages;
mod print;
mod rebinding;
mod seams;
mod slice;
mod spelling;
mod vim_seams;
mod zoom;
