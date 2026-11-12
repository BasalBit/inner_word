//! Spell checking for The Writer.
//!
//! Two layers:
//!
//! - **The service** ([`SpellService`]): what a spell checker can be asked.
//!   [`SystemSpellService`] answers with the Mac's own checker
//!   (`NSSpellChecker`: its languages, its learned words, shared with
//!   every other app); [`FakeSpellService`] answers from word lists, for
//!   tests in every crate.
//! - **The checker** ([`SpellChecker`]): a GPUI entity that attaches to a
//!   `writer_editor::Editor` and keeps its spelling marks current, offers
//!   the rows for the text context menu, the actions and the queries the
//!   Vim keys need.
//!
//! # Setup
//!
//! ```ignore
//! // once at startup, after writer_editor::init
//! writer_spell::init(cx);
//!
//! // one service for the app (main thread)
//! let service: Rc<dyn SpellService> = Rc::new(SystemSpellService::new());
//!
//! // per editor (the app keeps one editor and replaces its document)
//! let spell = cx.new(|cx| SpellChecker::new(editor.clone(), service.clone(), cx));
//! spell.update(cx, |spell, cx| {
//!     spell.set_language(SpellLanguage::from_setting(&stored), cx);
//!     spell.set_enabled(check_while_typing, cx);
//! });
//!
//! // the actions: on an element around the editor (or the workspace root)
//! writer_spell::handle_actions(div().size_full(), &spell).child(tools.clone())
//!
//! // the top of the text context menu
//! tools.update(cx, |tools, cx| tools.set_text_menu_leading_extension(move |request, _window, cx| {
//!     spell.update(cx, |spell, cx| spell.context_menu_items(request, cx))
//! }, cx));
//!
//! // the palette
//! let commands = writer_spell::commands();
//! ```

pub mod actions;
mod checker;
mod fake;
mod marks;
mod navigate;
mod service;
mod system;
mod text;
mod words;
mod work;

pub use checker::{
    EDIT_PAUSE, IDLE_PAUSE, IGNORE_SPELLING, LEARN_SPELLING, MAX_SUGGESTIONS, NO_GUESSES,
    SEEK_LIMIT, SLICE_BUDGET, SLICE_PAUSE, SpellChecker, SpellEvent, TYPING_PAUSE,
    UNLEARN_SPELLING, handle_actions,
};
pub use fake::{FAKE_LANGUAGE, FakeSpellService};
pub use service::{
    LanguageInfo, SpellCheck, SpellCheckPart, SpellDocument, SpellLanguage, SpellService,
    language_choices, language_name,
};
pub use system::SystemSpellService;

/// The byte ranges of `text` for UTF-16 ranges `(location, length)` given
/// in rising order, as the system service converts what `NSSpellChecker`
/// returns. For this crate's tests.
#[doc(hidden)]
pub fn byte_ranges_of_utf16(text: &str, ranges: &[(usize, usize)]) -> Vec<std::ops::Range<usize>> {
    let mut map = system::Utf16Map::new(text);
    ranges
        .iter()
        .map(|&(location, length)| map.byte_range(location, length))
        .collect()
}

/// The UTF-16 range of a byte range of `text`, as the system service
/// passes it to `NSSpellChecker`. For this crate's tests.
#[doc(hidden)]
pub fn utf16_range_of_bytes(text: &str, range: std::ops::Range<usize>) -> std::ops::Range<usize> {
    system::utf16_range(text, range)
}

use writer_ui::CommandDescriptor;

/// The category of the spelling commands in the palette.
pub const CATEGORY_SPELLING: &str = "Spelling";

/// Installs the key bindings of the spelling actions (in the editor's key
/// context). Call once at startup, after `writer_editor::init`.
pub fn init(cx: &mut gpui::App) {
    cx.bind_keys(actions::key_bindings());
}

/// The spelling commands for the command palette, the shortcuts overlay
/// and the menu bar. They act on the editor that has the keyboard, through
/// the handlers [`handle_actions`] installs.
pub fn commands() -> Vec<CommandDescriptor> {
    use actions::*;
    vec![
        CommandDescriptor::new("Next Misspelling", CATEGORY_SPELLING, NextMisspelling)
            .keywords(["spell", "typo", "mistake", "error"]),
        CommandDescriptor::new(
            "Previous Misspelling",
            CATEGORY_SPELLING,
            PreviousMisspelling,
        )
        .keywords(["spell", "typo", "mistake", "error"]),
        CommandDescriptor::new(
            "Show Spelling Suggestions",
            CATEGORY_SPELLING,
            ShowSuggestions,
        )
        .keywords(["spell", "correct", "guess", "replace"]),
        CommandDescriptor::new("Learn Spelling", CATEGORY_SPELLING, LearnSpelling).keywords([
            "dictionary",
            "add word",
            "remember",
        ]),
        CommandDescriptor::new("Unlearn Spelling", CATEGORY_SPELLING, UnlearnSpelling).keywords([
            "dictionary",
            "remove word",
            "forget",
        ]),
        CommandDescriptor::new("Ignore Spelling", CATEGORY_SPELLING, IgnoreSpelling)
            .keywords(["skip", "this document"]),
        CommandDescriptor::new(
            "Check Spelling While Typing",
            CATEGORY_SPELLING,
            ToggleSpellChecking,
        )
        .keywords(["spell", "checker", "on", "off", "toggle", "underline"]),
    ]
}
