//! Spell checking for Inner Word.
//!
//! Two layers:
//!
//! - **The service** ([`SpellService`]): what a spell checker can be asked.
//!   [`SystemSpellService`] answers with the Mac's own checker
//!   (`NSSpellChecker`: its languages, its learned words, shared with
//!   every other app; nothing is bundled); [`FakeSpellService`] answers
//!   from word lists, for tests in every crate. Tests never ask the
//!   system checker: its answers differ between macOS versions and
//!   between users.
//! - **The checker** ([`SpellChecker`]): a GPUI entity that attaches to a
//!   `innerword_editor::Editor` and keeps its spelling marks current, offers
//!   the rows for the text context menu, the actions and the queries the
//!   Vim keys need.
//!
//! # Setup
//!
//! ```ignore
//! // once at startup, after innerword_editor::init
//! innerword_spell::init(cx);
//!
//! // one service for the app, made and used on the main thread
//! let service: Rc<dyn SpellService> = Rc::new(SystemSpellService::new());
//!
//! // per editor (the app keeps one editor and replaces its document)
//! let spell = cx.new(|cx| SpellChecker::new(editor.clone(), service.clone(), cx));
//! spell.update(cx, |spell, cx| {
//!     // the document's own language, else the app's setting
//!     spell.set_language(SpellLanguage::effective(document_language.as_ref(), &app_language), cx);
//!     spell.set_enabled(check_while_typing, cx);
//! });
//! cx.subscribe(&spell, |this, _, event: &SpellEvent, cx| match event {
//!     SpellEvent::EnabledChanged(on) => this.store_setting(*on, cx), // the toggle action
//!     _ => {}
//! }).detach();
//!
//! // the actions: on an element around the editor, or on the root of the
//! // workspace so that the palette and the menu bar reach them
//! innerword_spell::handle_actions(div().size_full(), &spell).child(tools.clone())
//!
//! // the top of the text context menu (innerword-editor-tools)
//! tools.update(cx, |tools, _| {
//!     let spell = spell.clone();
//!     tools.set_text_menu_leading_extension(move |request, _window, cx| {
//!         spell.update(cx, |spell, cx| spell.context_menu_items(request, cx))
//!     });
//! });
//!
//! // the palette, the shortcuts overlay, the menu bar
//! let commands = innerword_spell::commands();
//!
//! // a settings row: the user's languages first
//! let languages = innerword_spell::language_choices(&*service);
//! ```
//!
//! # What the checker does
//!
//! - **What is on screen first.** A block is checked once and then left
//!   alone until it changes; the editor's own store of marks
//!   (`innerword_editor::spelling`) is the cache. The rest of a long document
//!   is checked in slices of [`SLICE_BUDGET`] on the main thread, with
//!   [`SLICE_PAUSE`] between them, and not at all while somebody types or
//!   scrolls ([`IDLE_PAUSE`]). Measured with the system checker on a book
//!   of 10,000 paragraphs (700,000 words, the `spell_demo` example): what
//!   is on screen is checked 25 ms after the document is set, the whole
//!   book after 17 s with automatic detection and 12 s with a fixed
//!   language, no slice of the 1,556 took more than 6 ms, and a keystroke
//!   costs the same with the checker as without it.
//! - **An edit** makes its paragraph unchecked (the editor keeps the old
//!   marks in step with the text meanwhile); it is checked again
//!   [`EDIT_PAUSE`] after the first edit of a burst, and nothing else is.
//! - **The word being typed** is not marked: the misspelled word the
//!   caret is in or right behind is held back until the caret leaves it
//!   or [`TYPING_PAUSE`] has passed since the last edit.
//! - **Not checked**: code blocks, dividers and page breaks; inline code
//!   is blanked out before the text goes to the service. A compact or
//!   read-only editor is left alone altogether.
//! - **Language.** [`SpellChecker::set_language`] takes the effective
//!   language: a fixed one, or [`SpellLanguage::Automatic`], where the
//!   service identifies the language of each paragraph. A line it cannot
//!   place (a heading of two words) is checked in the language most of
//!   the document was identified as, in the user's own variant of it.
//!   A change of language removes the marks and checks again.
//! - **A marked word**: [`SpellChecker::context_menu_items`] gives the
//!   rows that lead the text menu (up to [`MAX_SUGGESTIONS`] suggestions
//!   or a disabled [`NO_GUESSES`], [`LEARN_SPELLING`],
//!   [`IGNORE_SPELLING`]); [`SpellChecker::show_suggestions`] opens them
//!   as a menu of their own from the keyboard. A suggestion replaces only
//!   the letters that differ, as one undo step, so the word keeps its
//!   formatting. Learning is permanent and shared with other apps (the
//!   toast that says so has an Undo); ignoring lasts until the document
//!   is closed or replaced. Either takes the mark off every occurrence at
//!   once.
//! - **From one to the next**: [`actions::NextMisspelling`] (`cmd-;`) and
//!   [`actions::PreviousMisspelling`] (`cmd-shift-;`) select the word and
//!   scroll to it, around the ends of the document, checking ahead for at
//!   most [`SEEK_LIMIT`] in a document that is not fully checked and
//!   saying so when that was not enough.
//! - **Another document** in the same editor starts clean: marks, what
//!   was learned about its languages, and the words ignored in it.
//!
//! # For tests in other crates
//!
//! ```ignore
//! let service = Rc::new(
//!     FakeSpellService::with_words(["the", "ship", "came", "in"]).suggest("teh", ["the"]),
//! );
//! let spell = cx.new(|cx| {
//!     let mut spell = SpellChecker::new(editor.clone(), service.clone(), cx);
//!     spell.set_language(SpellLanguage::fixed(innerword_spell::FAKE_LANGUAGE), cx);
//!     spell
//! });
//! cx.run_until_parked();                        // a short document is checked now
//! cx.executor().advance_clock(EDIT_PAUSE);      // after typing: the paragraph again
//! cx.executor().advance_clock(TYPING_PAUSE);    // and the word under the caret
//! assert_eq!(spell.read_with(cx, |spell, cx| spell.misspelled_words(cx)), ["teh"]);
//! assert_eq!(service.check_count(), 2);
//! ```

pub mod actions;
mod checker;
pub mod fake;
mod marks;
mod navigate;
mod service;
pub mod system;
mod text;
mod words;
mod work;

pub use checker::{
    EDIT_PAUSE, IDLE_PAUSE, IGNORE_SPELLING, LEARN_SPELLING, MAX_SUGGESTIONS, NO_GUESSES,
    SEEK_LIMIT, SLICE_BUDGET, SLICE_PAUSE, SpellChecker, SpellEvent, TYPING_PAUSE,
    UNLEARN_SPELLING, WARM_UP_DELAY, handle_actions,
};
pub use fake::{FAKE_LANGUAGE, FakeSpellService};
pub use service::{
    LanguageInfo, OWN_CHOICE_LIMIT, SpellCheck, SpellCheckPart, SpellDocument, SpellLanguage,
    SpellService, language_choices, language_name,
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

use innerword_ui::CommandDescriptor;

/// The category of the spelling commands in the palette.
pub const CATEGORY_SPELLING: &str = "Spelling";

/// Installs the key bindings of the spelling actions (in the editor's key
/// context). Call once at startup, after `innerword_editor::init`.
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
