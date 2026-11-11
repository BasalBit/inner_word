//! Vim emulation engine for The Writer.

#![warn(missing_docs)]

mod buf;
mod effect;
mod engine;
mod ex;
mod host;
mod key;
mod motion;
mod register;
mod search;
mod textobj;

pub use effect::{CaretShape, CommandLine, Effect, HostRequest, Mode, Outcome, SearchPattern};
pub use engine::{Vim, VimOptions};
pub use host::{GoalColumn, Scroll, ScrollAlign, Viewport, VimHost};
pub use key::{Key, KeyCode};
pub use register::RegisterContent;
