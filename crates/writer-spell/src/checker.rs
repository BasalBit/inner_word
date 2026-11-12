//! The checker: a GPUI entity that attaches to an [`Editor`] and keeps its
//! spelling marks current.
//!
//! This file holds the entity, its settings and what it tells the world;
//! `work` holds the checking itself (what, when, in which language),
//! `words` what is done with a marked word (suggestions, replace, learn,
//! ignore, the menu rows) and `navigate` the walk from one marked word to
//! the next.

use std::collections::{HashMap, HashSet, VecDeque};
use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    Action, App, Context, Entity, EventEmitter, InteractiveElement, Pixels, SharedString, Size,
    Subscription, Task, Window,
};
use writer_core::{BlockId, Position, Selection, TextRange};
use writer_editor::{Editor, EditorEvent};
use writer_ui::toast::{Toast, show_toast};

use crate::actions::{
    IgnoreSpelling, LearnSpelling, NextMisspelling, PreviousMisspelling, ShowSuggestions,
    ToggleSpellChecking, UnlearnSpelling,
};
use crate::marks;
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

/// How long one slice of checking may take before it gives the main
/// thread back. A slice ends after the first piece of work that passes
/// this, so it can run over by one call into the service (about 2 ms at
/// most).
pub const SLICE_BUDGET: Duration = Duration::from_millis(4);
/// The pause between two slices.
pub const SLICE_PAUSE: Duration = Duration::from_millis(4);
/// How long after the first edit of a burst the edited blocks are checked
/// again. Not pushed back by the edits that follow: while somebody types
/// without a break, their paragraph is still checked this often.
pub const EDIT_PAUSE: Duration = Duration::from_millis(150);
/// How long after the last edit or scroll the rest of the document (what
/// is neither on screen nor just edited) is checked again: the background
/// pass waits while somebody is at work.
pub const IDLE_PAUSE: Duration = Duration::from_millis(300);
/// How long after the last edit the word the caret is in gets its mark,
/// when the caret has not left it before.
pub const TYPING_PAUSE: Duration = Duration::from_millis(1000);
/// How long after a checker was made its service is warmed up
/// ([`SpellService::warm_up`]): after the first marks are on screen.
pub const WARM_UP_DELAY: Duration = Duration::from_millis(400);
/// How long "next misspelling" may check ahead in a document that is not
/// fully checked before it says that it has found nothing yet.
pub const SEEK_LIMIT: Duration = Duration::from_millis(200);

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

/// What the checker knows about the language of a block that was checked
/// with [`SpellLanguage::Automatic`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Tag {
    /// The service identified this language.
    Detected(Rc<str>),
    /// The service could not name a language (too little text, or a
    /// language it has no dictionary for).
    Unplaced {
        /// It marked words all the same: with a text it cannot place the
        /// system checker accepts a word that any of the user's languages
        /// knows and marks the others.
        flagged: bool,
        /// The language the block was checked in instead: the language
        /// of the text around it. `None`: the service's own answer
        /// stands.
        checked_in: Option<Rc<str>>,
    },
}

/// A block whose check was interrupted by the end of a slice.
pub(crate) struct Partial {
    pub(crate) block: BlockId,
    pub(crate) revision: u64,
    /// How many bytes of the text are done.
    pub(crate) at: usize,
    pub(crate) found: Vec<Range<usize>>,
    pub(crate) tag: Option<Tag>,
}

/// The result of the block the caret is typing in: one word is not shown
/// yet.
pub(crate) struct Held {
    pub(crate) block: BlockId,
    pub(crate) revision: u64,
    /// The word that is held back.
    pub(crate) word: Range<usize>,
    /// The whole result, the held word included.
    pub(crate) all: Vec<Range<usize>>,
}

/// Keeps the spelling marks of one [`Editor`] current: what is on screen
/// first, the rest of the document in small slices while nobody types, an
/// edited paragraph again after a short pause. See the crate docs.
pub struct SpellChecker {
    pub(crate) editor: Entity<Editor>,
    pub(crate) service: Rc<dyn SpellService>,
    pub(crate) enabled: bool,
    pub(crate) language: SpellLanguage,
    pub(crate) document: SpellDocument,
    /// The user's preferred languages and every language of the service,
    /// read when first needed.
    pub(crate) known_languages: Option<(Vec<String>, Vec<String>)>,

    // --- The work ---
    /// Whether the checker was at work on the editor when it last looked
    /// (enabled, a full editor that can be written in).
    pub(crate) was_active: bool,
    /// Whether the worker task is running.
    pub(crate) working: bool,
    pub(crate) worker: Option<Task<()>>,
    /// Warms the service up a moment after the checker was made.
    pub(crate) warm_up: Option<Task<()>>,
    /// Every block before this index is checked, or waits in `waiting` or
    /// `recheck`: where the background pass goes on.
    pub(crate) scan: usize,
    /// Blocks that were just edited: checked when the pause is over.
    pub(crate) waiting: HashSet<BlockId>,
    /// Blocks to check before anything else.
    pub(crate) recheck: VecDeque<BlockId>,
    pub(crate) partial: Option<Partial>,
    /// Whether every block has been checked and nothing waits.
    pub(crate) settled: bool,
    /// Somebody is typing or scrolling: the background pass waits.
    pub(crate) busy: bool,
    pub(crate) busy_timer: Option<Task<()>>,
    pub(crate) edit_timer: Option<Task<()>>,
    pub(crate) typing_timer: Option<Task<()>>,
    /// The block the caret is typing in.
    pub(crate) typing: Option<BlockId>,
    pub(crate) held: Option<Held>,
    /// The selection right after the last edit: a `SelectionChanged` that
    /// finds it unchanged belongs to that edit.
    pub(crate) selection_at_edit: Option<Selection>,
    /// Scroll offset and viewport size when the checker last looked.
    pub(crate) last_view: Option<(Pixels, Option<Size<Pixels>>)>,

    // --- Languages, with automatic detection ---
    pub(crate) tags: HashMap<BlockId, Tag>,
    /// How many blocks were identified as each language.
    pub(crate) detected: HashMap<Rc<str>, usize>,
    /// How many blocks the service marked words in without naming a
    /// language.
    pub(crate) unidentified: usize,
    /// The language a text is checked in when the service cannot tell.
    pub(crate) fallback: Option<Rc<str>>,

    // --- For a status line, a script or a test ---
    pub(crate) last_notice: Option<SharedString>,
    pub(crate) slices: u64,
    pub(crate) longest_slice: Duration,
    /// Slices that took more than twice their budget.
    pub(crate) slow_slices: u64,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SpellEvent> for SpellChecker {}

impl Drop for SpellChecker {
    fn drop(&mut self) {
        // The words ignored in the document go with it.
        self.service.close_document(self.document);
    }
}

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
        let document = service.open_document();
        let subscriptions = vec![
            cx.subscribe(&editor, |this, _, event: &EditorEvent, cx| {
                this.on_editor_event(event, cx)
            }),
            cx.observe(&editor, |this, _, cx| this.editor_notified(cx)),
        ];
        let mut this = SpellChecker {
            editor,
            service,
            enabled: true,
            language: SpellLanguage::Automatic,
            document,
            known_languages: None,
            was_active: false,
            working: false,
            worker: None,
            warm_up: None,
            scan: 0,
            waiting: HashSet::new(),
            recheck: VecDeque::new(),
            partial: None,
            settled: false,
            busy: false,
            busy_timer: None,
            edit_timer: None,
            typing_timer: None,
            typing: None,
            held: None,
            selection_at_edit: None,
            last_view: None,
            tags: HashMap::new(),
            detected: HashMap::new(),
            unidentified: 0,
            fallback: None,
            last_notice: None,
            slices: 0,
            longest_slice: Duration::ZERO,
            slow_slices: 0,
            _subscriptions: subscriptions,
        };
        this.was_active = this.is_active(cx);
        this.start_worker(cx);
        // What is slow the first time (starting the system checker for
        // suggestions, loading the dictionaries it consults for a text it
        // cannot place) happens once, a moment after the first marks are
        // on screen, and not under a keystroke or a right-click later.
        this.warm_up = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(WARM_UP_DELAY).await;
            this.update(cx, |this, _| {
                this.warm_up = None;
                if this.was_active {
                    this.service.warm_up();
                }
            })
            .ok();
        }));
        this
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
        if self.enabled == enabled {
            return;
        }
        self.enabled = enabled;
        self.follow_activity(cx);
        cx.emit(SpellEvent::EnabledChanged(enabled));
        cx.notify();
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
    /// setting). A change removes every mark and checks again, what is on
    /// screen first. Emits [`SpellEvent::LanguageChanged`] when the value
    /// changed.
    pub fn set_language(&mut self, language: SpellLanguage, cx: &mut Context<Self>) {
        if self.language == language {
            return;
        }
        self.language = language.clone();
        if self.was_active {
            let had_marks = marks::count(self.editor.read(cx)) > 0;
            marks::clear(&self.editor, cx);
            self.forget_results();
            if had_marks {
                cx.emit(SpellEvent::MarksChanged);
            }
            self.start_worker(cx);
        }
        cx.emit(SpellEvent::LanguageChanged(language));
        cx.notify();
    }

    /// The language the block at `block` was checked in, when the
    /// language is [`SpellLanguage::Automatic`] and the block has been
    /// checked: the one the service identified, or the one the checker
    /// fell back to when the service could not tell. `None` when it is
    /// not known (and always with a fixed language).
    pub fn detected_language(&self, block: usize, cx: &App) -> Option<String> {
        let id = self.editor.read(cx).document().block(block)?.id();
        match self.tags.get(&id)? {
            Tag::Detected(language)
            | Tag::Unplaced {
                checked_in: Some(language),
                ..
            } => Some(language.to_string()),
            Tag::Unplaced {
                checked_in: None, ..
            } => None,
        }
    }

    // --- State, for a status line, a script or a test ---

    /// Whether every block of the document has been checked for its
    /// current text (always false while checking is off or the editor is
    /// left alone).
    pub fn is_settled(&self, cx: &App) -> bool {
        self.is_active(cx) && self.settled
    }

    /// How many blocks still wait to be checked. Counts the whole
    /// document: for a status line, not for every frame.
    pub fn pending_blocks(&self, cx: &App) -> usize {
        if !self.is_active(cx) || self.settled {
            return 0;
        }
        let editor = self.editor.read(cx);
        (0..editor.document().block_count())
            .filter(|index| !marks::is_checked(editor, *index))
            .count()
    }

    /// How many words are marked in the whole document.
    pub fn misspelling_count(&self, cx: &App) -> usize {
        marks::count(self.editor.read(cx))
    }

    /// Every marked range, in document order. Walks the whole document:
    /// for tests and scripts.
    pub fn misspellings(&self, cx: &App) -> Vec<TextRange> {
        let editor = self.editor.read(cx);
        if marks::count(editor) == 0 {
            return Vec::new();
        }
        (0..editor.document().block_count())
            .flat_map(|index| {
                marks::of_block(editor, index)
                    .iter()
                    .map(move |range| TextRange::in_block(index, range.start, range.end))
            })
            .collect()
    }

    /// The marked words as text, in document order. For tests and
    /// scripts.
    pub fn misspelled_words(&self, cx: &App) -> Vec<String> {
        let document = self.editor.read(cx).document();
        self.misspellings(cx)
            .into_iter()
            .map(|range| document.text_in(range))
            .collect()
    }

    /// The last [`SpellEvent::Notice`].
    pub fn last_notice(&self) -> Option<SharedString> {
        self.last_notice.clone()
    }

    /// How many slices of checking have run, and the longest of them:
    /// what the checker cost the main thread at its worst.
    pub fn slice_stats(&self) -> (u64, Duration) {
        (self.slices, self.longest_slice)
    }

    /// How many slices took more than twice [`SLICE_BUDGET`] (a call into
    /// the service that was slow, or a main thread that was kept waiting).
    pub fn slow_slice_count(&self) -> u64 {
        self.slow_slices
    }

    /// Forgets [`SpellChecker::slice_stats`] and
    /// [`SpellChecker::slow_slice_count`].
    pub fn reset_slice_stats(&mut self) {
        self.slices = 0;
        self.longest_slice = Duration::ZERO;
        self.slow_slices = 0;
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

    // --- Inside ---

    /// Tells the user something: an event, and a toast where the window
    /// has a toast layer.
    pub(crate) fn notice(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        let message = message.into();
        self.last_notice = Some(message.clone());
        show_toast(Toast::new(message.clone()), cx);
        cx.emit(SpellEvent::Notice(message));
    }

    /// Says why a command did nothing, when the checker is not at work.
    /// Returns whether it is.
    pub(crate) fn ready_for_command(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.enabled {
            self.notice("Spell checking is off", cx);
            return false;
        }
        self.is_active(cx)
    }

    /// The position a command at the caret is about: the caret, or the
    /// start of the selection.
    pub(crate) fn caret(&self, cx: &App) -> Position {
        let selection = self.editor.read(cx).selection();
        if selection.is_collapsed() {
            selection.head
        } else {
            selection.start()
        }
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
