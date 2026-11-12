//! The checker: a GPUI entity that attaches to an [`Editor`] and keeps its
//! spelling marks current.
//!
//! Status: skeleton (the public API is final; the work behind it follows).

use std::rc::Rc;

use gpui::{
    Action, App, Context, Entity, EventEmitter, InteractiveElement, SharedString, Subscription,
    Window,
};
use writer_core::{Position, TextRange};
use writer_editor::{ContextMenuRequest, Editor};
use writer_ui::ContextMenuItem;

use crate::actions::{
    IgnoreSpelling, LearnSpelling, NextMisspelling, PreviousMisspelling, ShowSuggestions,
    ToggleSpellChecking, UnlearnSpelling,
};
use crate::service::{SpellDocument, SpellLanguage, SpellService};

/// The most suggestions a menu offers.
pub const MAX_SUGGESTIONS: usize = 5;

/// The row a menu shows when the service has no suggestion.
pub const NO_GUESSES: &str = "No Guesses Found";
/// The title of the menu item that learns the word.
pub const LEARN_SPELLING: &str = "Learn Spelling";
/// The title of the menu item that ignores the word in this document.
pub const IGNORE_SPELLING: &str = "Ignore Spelling";
/// The title of the menu item that takes a learned word back.
pub const UNLEARN_SPELLING: &str = "Unlearn Spelling";

/// What a [`SpellChecker`] tells its subscribers. `#[non_exhaustive]`:
/// match with a wildcard arm.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum SpellEvent {
    /// Checking was turned on or off ([`SpellChecker::set_enabled`], the
    /// [`ToggleSpellChecking`] action). The app stores the setting on
    /// this.
    EnabledChanged(bool),
    /// The language changed ([`SpellChecker::set_language`]).
    LanguageChanged(SpellLanguage),
    /// The marks of at least one block changed.
    MarksChanged,
    /// Every block of the document has been checked (the background pass
    /// is done); emitted again after each later change has been checked.
    Settled,
    /// A word was added to the user's own dictionary.
    Learned(String),
    /// A word was taken out of the user's own dictionary.
    Unlearned(String),
    /// A word is ignored in this document from now on.
    Ignored(String),
    /// A misspelled word was replaced by a suggestion.
    Replaced {
        /// The word that was in the text.
        from: String,
        /// What replaced it.
        to: String,
    },
    /// Something the user should read ("No misspellings found"). The
    /// checker has already shown it as a toast when the window has a
    /// toast layer.
    Notice(SharedString),
}

/// Keeps the spelling marks of one [`Editor`] current. See the crate
/// docs.
pub struct SpellChecker {
    editor: Entity<Editor>,
    service: Rc<dyn SpellService>,
    enabled: bool,
    language: SpellLanguage,
    document: SpellDocument,
    last_notice: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SpellEvent> for SpellChecker {}

impl SpellChecker {
    /// Attaches a checker to `editor`: `cx.new(|cx| SpellChecker::new(editor.clone(), service, cx))`.
    /// It starts enabled, with [`SpellLanguage::Automatic`]; nothing is
    /// checked before the current effect cycle ends, so a
    /// [`SpellChecker::set_language`] or [`SpellChecker::set_enabled`]
    /// right after construction costs nothing.
    ///
    /// A compact or read-only editor is left alone: the checker stays
    /// attached and does nothing while the editor is in that state.
    pub fn new(
        editor: Entity<Editor>,
        service: Rc<dyn SpellService>,
        cx: &mut Context<Self>,
    ) -> Self {
        let _ = cx;
        let document = service.open_document();
        SpellChecker {
            editor,
            service,
            enabled: true,
            language: SpellLanguage::Automatic,
            document,
            last_notice: None,
            _subscriptions: Vec::new(),
        }
    }

    /// The editor this checker is attached to.
    pub fn editor(&self) -> &Entity<Editor> {
        &self.editor
    }

    /// The service that answers.
    pub fn service(&self) -> Rc<dyn SpellService> {
        self.service.clone()
    }

    /// The tag of the open document (its ignored words belong to it).
    pub fn document(&self) -> SpellDocument {
        self.document
    }

    /// Whether checking is on.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Turns checking on or off. Off removes every mark and stops all
    /// work; on starts over with what is on screen. Emits
    /// [`SpellEvent::EnabledChanged`] when the value changed.
    pub fn set_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.enabled != enabled {
            self.enabled = enabled;
            cx.emit(SpellEvent::EnabledChanged(enabled));
            cx.notify();
        }
    }

    /// Turns checking on when it is off and off when it is on.
    pub fn toggle_enabled(&mut self, cx: &mut Context<Self>) {
        self.set_enabled(!self.enabled, cx);
    }

    /// Whether the checker is at work on this editor: enabled, and the
    /// editor is neither compact nor read-only.
    pub fn is_active(&self, cx: &App) -> bool {
        let editor = self.editor.read(cx);
        self.enabled && !editor.config().mode.is_compact() && !editor.is_read_only()
    }

    /// The effective language.
    pub fn language(&self) -> &SpellLanguage {
        &self.language
    }

    /// Sets the effective language (the document's own, or the app's
    /// setting). A change removes every mark and checks again. Emits
    /// [`SpellEvent::LanguageChanged`] when the value changed.
    pub fn set_language(&mut self, language: SpellLanguage, cx: &mut Context<Self>) {
        if self.language != language {
            self.language = language.clone();
            cx.emit(SpellEvent::LanguageChanged(language));
            cx.notify();
        }
    }

    /// The language the service detected for the block at `block` (only
    /// with [`SpellLanguage::Automatic`], once the block was checked).
    pub fn detected_language(&self, block: usize, cx: &App) -> Option<String> {
        let _ = (block, cx);
        None
    }

    // --- State, for a status line, a script or a test ---

    /// Whether every block of the document has been checked for its
    /// current text.
    pub fn is_settled(&self, cx: &App) -> bool {
        let _ = cx;
        true
    }

    /// How many blocks still wait to be checked.
    pub fn pending_blocks(&self, cx: &App) -> usize {
        let _ = cx;
        0
    }

    /// How many words are marked in the whole document.
    pub fn misspelling_count(&self, cx: &App) -> usize {
        let _ = cx;
        0
    }

    /// Every marked range, in document order.
    pub fn misspellings(&self, cx: &App) -> Vec<TextRange> {
        let _ = cx;
        Vec::new()
    }

    /// The last [`SpellEvent::Notice`].
    pub fn last_notice(&self) -> Option<SharedString> {
        self.last_notice.clone()
    }

    /// Checks everything that is waiting, now, without regard to time
    /// (the word being typed stays unmarked). For tests and scripts; the
    /// app never needs it.
    pub fn check_all_now(&mut self, cx: &mut Context<Self>) {
        let _ = cx;
    }

    // --- Queries (the Vim keys use them) ---

    /// The marked word at `position`: the mark that contains it or ends
    /// or starts there.
    pub fn misspelling_at(&self, position: Position, cx: &App) -> Option<TextRange> {
        let _ = (position, cx);
        None
    }

    /// The first marked word that starts after `from`, wrapping around
    /// the end of the document. Blocks that were not checked yet are
    /// checked on the way, within a time limit; `None` when nothing is
    /// marked (or nothing was found in time).
    pub fn next_misspelling(
        &mut self,
        from: Position,
        cx: &mut Context<Self>,
    ) -> Option<TextRange> {
        let _ = (from, cx);
        None
    }

    /// The last marked word that starts before `from`, wrapping around
    /// the start of the document. See [`SpellChecker::next_misspelling`].
    pub fn previous_misspelling(
        &mut self,
        from: Position,
        cx: &mut Context<Self>,
    ) -> Option<TextRange> {
        let _ = (from, cx);
        None
    }

    /// The suggestions for the marked word at `position`, best first, at
    /// most [`MAX_SUGGESTIONS`]. Empty when the word is not marked or the
    /// service has none.
    pub fn suggestions_at(&mut self, position: Position, cx: &mut Context<Self>) -> Vec<String> {
        let _ = (position, cx);
        Vec::new()
    }

    /// The word at `position` (marked or not): its range and text.
    pub fn word_at(&self, position: Position, cx: &App) -> Option<(TextRange, String)> {
        let _ = (position, cx);
        None
    }

    /// Replaces the text at `range` (a marked word) by `replacement` as
    /// one undo step, keeping the formatting of the replaced text, and
    /// puts the caret after it. Returns whether the document changed.
    pub fn replace(&mut self, range: TextRange, replacement: &str, cx: &mut Context<Self>) -> bool {
        let _ = (range, replacement, cx);
        false
    }

    /// Learns the marked word at `position` (permanent). Every occurrence
    /// in the document loses its mark at once. Returns the word.
    pub fn learn_at(&mut self, position: Position, cx: &mut Context<Self>) -> Option<String> {
        let _ = (position, cx);
        None
    }

    /// Takes the word at `position` out of the user's own dictionary, if
    /// it was learned; the document is checked again. Returns the word.
    pub fn unlearn_at(&mut self, position: Position, cx: &mut Context<Self>) -> Option<String> {
        let _ = (position, cx);
        None
    }

    /// Ignores the marked word at `position` in this document until it is
    /// closed. Every occurrence loses its mark at once. Returns the word.
    pub fn ignore_at(&mut self, position: Position, cx: &mut Context<Self>) -> Option<String> {
        let _ = (position, cx);
        None
    }

    /// Learns `word` (permanent) and removes its marks.
    pub fn learn_word(&mut self, word: &str, cx: &mut Context<Self>) {
        let _ = (word, cx);
    }

    /// Unlearns `word` and checks the document again.
    pub fn unlearn_word(&mut self, word: &str, cx: &mut Context<Self>) {
        let _ = (word, cx);
    }

    /// Ignores `word` in this document and removes its marks.
    pub fn ignore_word(&mut self, word: &str, cx: &mut Context<Self>) {
        let _ = (word, cx);
    }

    // --- Commands ---

    /// Selects the next misspelled word after the caret and scrolls to
    /// it, wrapping. Says so ([`SpellEvent::Notice`]) when there is none.
    /// Returns whether a word was selected.
    pub fn go_to_next_misspelling(&mut self, cx: &mut Context<Self>) -> bool {
        let _ = cx;
        false
    }

    /// Selects the previous misspelled word before the caret and scrolls
    /// to it, wrapping. See [`SpellChecker::go_to_next_misspelling`].
    pub fn go_to_previous_misspelling(&mut self, cx: &mut Context<Self>) -> bool {
        let _ = cx;
        false
    }

    /// Learns the marked word at the caret (or the marked word that is
    /// selected). Returns whether there was one.
    pub fn learn_at_caret(&mut self, cx: &mut Context<Self>) -> bool {
        let _ = cx;
        false
    }

    /// Unlearns the word at the caret, if it was learned.
    pub fn unlearn_at_caret(&mut self, cx: &mut Context<Self>) -> bool {
        let _ = cx;
        false
    }

    /// Ignores the marked word at the caret (or the marked word that is
    /// selected) in this document. Returns whether there was one.
    pub fn ignore_at_caret(&mut self, cx: &mut Context<Self>) -> bool {
        let _ = cx;
        false
    }

    /// Opens the suggestions for the marked word at the caret as a menu
    /// under it, with the first row marked (the keyboard path; `z=` in Vim
    /// mode). Returns whether a menu was opened.
    pub fn show_suggestions(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let _ = (window, cx);
        false
    }

    // --- The context menu ---

    /// The rows that lead the text context menu for `request`: when the
    /// request is on a marked word, up to [`MAX_SUGGESTIONS`] suggestions
    /// (or a disabled [`NO_GUESSES`] row), [`LEARN_SPELLING`],
    /// [`IGNORE_SPELLING`]; otherwise nothing. The caller adds the
    /// separator and the rest of the menu.
    pub fn context_menu_items(
        &mut self,
        request: &ContextMenuRequest,
        cx: &mut Context<Self>,
    ) -> Vec<ContextMenuItem> {
        self.menu_items_at(request.document_position, cx)
    }

    /// The same rows for the marked word at `position`.
    pub fn menu_items_at(
        &mut self,
        position: Position,
        cx: &mut Context<Self>,
    ) -> Vec<ContextMenuItem> {
        let _ = (position, cx);
        Vec::new()
    }

    // --- Actions ---

    /// Runs one of the [`crate::actions`] and returns `true`; `false`
    /// when `action` is not one of them. For a host that passes commands
    /// on by hand; [`crate::handle_actions`] is the usual way.
    pub fn run_action(
        &mut self,
        action: &dyn Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let any = action.as_any();
        if any.is::<NextMisspelling>() {
            self.go_to_next_misspelling(cx);
        } else if any.is::<PreviousMisspelling>() {
            self.go_to_previous_misspelling(cx);
        } else if any.is::<ShowSuggestions>() {
            self.show_suggestions(window, cx);
        } else if any.is::<LearnSpelling>() {
            self.learn_at_caret(cx);
        } else if any.is::<UnlearnSpelling>() {
            self.unlearn_at_caret(cx);
        } else if any.is::<IgnoreSpelling>() {
            self.ignore_at_caret(cx);
        } else if any.is::<ToggleSpellChecking>() {
            self.toggle_enabled(cx);
        } else {
            return false;
        }
        true
    }
}

/// Puts the handlers of the spelling actions on `element`, which must be
/// on the way from the editor outwards (an ancestor of the editor in the
/// element tree: the element that holds the editor, or the root of the
/// workspace so that the palette and the menu bar reach them from
/// anywhere).
///
/// ```ignore
/// writer_spell::handle_actions(div().size_full(), &self.spell).child(self.tools.clone())
/// ```
pub fn handle_actions<E: InteractiveElement>(element: E, checker: &Entity<SpellChecker>) -> E {
    fn on<A: Action, E: InteractiveElement>(element: E, checker: &Entity<SpellChecker>) -> E {
        let checker = checker.clone();
        element.on_action(move |action: &A, window: &mut Window, cx: &mut App| {
            checker.update(cx, |checker, cx| {
                checker.run_action(action, window, cx);
            });
        })
    }
    let element = on::<NextMisspelling, E>(element, checker);
    let element = on::<PreviousMisspelling, E>(element, checker);
    let element = on::<ShowSuggestions, E>(element, checker);
    let element = on::<LearnSpelling, E>(element, checker);
    let element = on::<UnlearnSpelling, E>(element, checker);
    let element = on::<IgnoreSpelling, E>(element, checker);
    on::<ToggleSpellChecking, E>(element, checker)
}
