//! The spelling actions and their keys.
//!
//! Unit structs in the `spell` namespace (`spell::NextMisspelling`), bound
//! in the editor's key context, so the keys work while the text has the
//! keyboard. The [`crate::SpellChecker`] of the editor handles them once
//! its handlers are on the way from the text outwards: see
//! [`crate::handle_actions`] and [`crate::SpellChecker::run_action`].

use gpui::{KeyBinding, actions};

actions!(
    spell,
    [
        /// Select the next misspelled word after the caret (wrapping
        /// around the document) and scroll to it.
        NextMisspelling,
        /// Select the previous misspelled word before the caret (wrapping
        /// around the document) and scroll to it.
        PreviousMisspelling,
        /// Open the suggestions for the misspelled word at the caret, as
        /// a menu under it.
        ShowSuggestions,
        /// Add the misspelled word at the caret to the user's own
        /// dictionary (permanent; the Mac's learned words).
        LearnSpelling,
        /// Take the word at the caret out of the user's own dictionary
        /// again.
        UnlearnSpelling,
        /// Stop marking the misspelled word at the caret in this document
        /// until it is closed.
        IgnoreSpelling,
        /// Turn checking while typing on or off.
        ToggleSpellChecking,
    ]
);

/// The key context the bindings live in: the editor's.
pub const KEY_CONTEXT: &str = writer_editor::actions::KEY_CONTEXT;

/// The default key bindings: `cmd-;` next misspelling, `cmd-shift-;`
/// previous misspelling. Shift plus a punctuation key arrives from a real
/// keyboard as the shifted character without the shift flag (`cmd-:` on a
/// US layout), so that spelling is bound too, before the readable one that
/// key hints show.
pub fn key_bindings() -> Vec<KeyBinding> {
    let c = Some(KEY_CONTEXT);
    vec![
        KeyBinding::new("cmd-;", NextMisspelling, c),
        KeyBinding::new("cmd-:", PreviousMisspelling, c),
        KeyBinding::new("cmd-shift-;", PreviousMisspelling, c),
    ]
}
