//! The keyboard shortcuts of The Writer as data: no GPUI, no files, no
//! clock. The app hands over what it knows and gets back what to bind,
//! what to store and what to say.
//!
//! # The pieces
//!
//! | Piece | What it is |
//! |---|---|
//! | [`Keystroke`], [`KeySequence`] | A key as text in GPUI's spelling, in one normal form ([`mod@keystroke`]). |
//! | [`Context`], [`Places`] | Where a binding applies, as the text of its key context, and what the caller says about the places of the window ([`mod@context`]). |
//! | [`Command`], [`Target`] | An action by name, and an action in a context: the unit a key belongs to. |
//! | [`Defaults`] | Every binding of the assembled keymap in order, and the commands without a key ([`mod@defaults`]). |
//! | [`Changes`] | What the user changed; the one text the app stores ([`mod@changes`]). |
//! | [`apply`], [`Effective`] | The keymap to install, in an order that keeps precedence, and the state of every target ([`mod@effective`]). |
//! | [`check`], [`Verdict`] | Guard rails and conflicts for a key the user proposes ([`mod@verdict`]). |
//! | [`export`], [`plan_import`] | The shortcuts file ([`mod@file`]). |
//! | [`Keymap`] | Defaults, places and changes held together ([`mod@keymap`]). |
//!
//! # How the app uses it
//!
//! 1. After every crate has bound its keys, walk GPUI's keymap in order
//!    and push a [`DefaultBinding`] for each binding (action name, action
//!    input, the context predicate's text, the keystrokes as text). Add
//!    the commands of the palette that have no key
//!    ([`Defaults::add_keyless`]) with the context a key for them would
//!    apply in.
//! 2. Describe the window once ([`Places`]): which contexts are places
//!    where text is typed, which can lie inside which, which never have
//!    the keyboard themselves.
//! 3. Read the stored changes ([`Keymap::with_stored`]) and install
//!    [`Keymap::effective`]: clear GPUI's keymap, then for each
//!    [`EffectiveBinding`] in order bind its `spelling` with the action
//!    and the context predicate of the default binding at its `source`
//!    (or, without a source, the action built from its name and the
//!    context parsed from its text).
//! 4. For a key the user proposes (a recorded GPUI keystroke is read with
//!    `Keystroke::parse(&keystroke.unparse())`, or from its parts with
//!    [`Keystroke::from_parts`]), show [`Keymap::check`], then change
//!    ([`Keymap::replace_key`] and the others), store
//!    [`Keymap::stored`], install again.
//! 5. To show a key, hand its text to GPUI: `Keystroke::to_string` is a
//!    text `gpui::Keystroke::parse` reads.
//!
//! # One key, two spellings
//!
//! Shift with a digit or a punctuation key reaches GPUI as the shifted
//! character without the shift flag, so the app binds such a key twice
//! (`cmd-&` and `cmd-shift-7`). Here the two are one key: both read as
//! the same [`Keystroke`], which prints as the readable form
//! (`cmd-shift-7`), and bindings of one command in one context with the
//! same key are one default key of that [`Target`]. Whatever the user
//! does to it is done to both; [`apply`] writes both spellings again for
//! a key that needs them ([`Keystroke::spellings`], the readable one
//! last). Details and the limits of that on other keyboard layouts are in
//! [`mod@keystroke`].
//!
//! # What cannot be changed here
//!
//! Only GPUI bindings are in the keymap. The grammar of Vim mode, the `/`
//! that opens the insert menu, and whatever else the app handles in code
//! are not bindings and are not known to this crate.
//!
//! # The stored value and the file
//!
//! The stored value (one JSON text in the library's settings) is
//! described in [`mod@changes`], the shortcuts file a person can read and
//! edit in [`mod@file`].

pub mod changes;
pub mod context;
pub mod defaults;
pub mod effective;
pub mod file;
pub mod keymap;
pub mod keystroke;
pub mod verdict;

pub use changes::{ChangeEntry, Changes, STORED_VERSION, StoredError};
pub use context::{Context, Extent, Places, Rank, Relation};
pub use defaults::{Command, DefaultBinding, Defaults, Target};
pub use effective::{
    Effective, EffectiveBinding, InactiveChange, InactiveReason, Origin, TargetKey, TargetKeys,
    apply,
};
pub use file::{
    Carried, Catalogue, CommandInfo, FILE_FORMAT, FILE_NAME, FILE_VERSION, FileChange, ImportError,
    ImportMode, ImportOutcome, ImportPlan, KeyDifference, SkipReason, Skipped, export, plan_import,
};
pub use keymap::Keymap;
pub use keystroke::{
    KeySequence, Keystroke, KeystrokeError, KeystrokeProblem, Modifiers, SystemUse,
};
pub use verdict::{
    Collision, Holder, Overlap, Refusal, Verdict, VerdictKind, Warning, check, collisions, guard,
};
