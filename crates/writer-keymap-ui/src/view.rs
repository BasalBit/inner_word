//! [`KeymapView`]: its state and what can be done in it. How it is drawn
//! is in `render.rs`.

use std::{
    cell::Cell,
    rc::Rc,
    time::{Duration, Instant},
};

use gpui::{
    App, Bounds, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, Pixels,
    Point, ScrollStrategy, SharedString, Subscription, UniformListScrollHandle, Window, WindowId,
    prelude::*,
};
use writer_keymap::{ImportError, ImportMode, ImportPlan, KeySequence, Target};
use writer_ui::{
    ContextMenu, ContextMenuItem, FieldAppearance, TextField, TextFieldEvent,
    context_menu::MenuAnchor, theme::UI_TEXT_LG,
};

use crate::{
    AddKey, CHOICES_STACK, ChangeKey, LIST_STACK, RemoveKey, RunCommand, UseDefault,
    keys::{self, glyph_list, glyphs},
    listing::{Entry, Listing, entries, inactive_note},
    question::{Choice, Preview, Question, Subject},
    record,
    request::Request,
    search::{Found, search},
    words::{Line, Names, SeenFirst, Tone},
};

/// How long a held key is waited for when no release is reported: a little
/// longer than the longest delay before a held key starts to repeat (two
/// seconds, the slowest setting of macOS).
const HELD_KEY_PATIENCE: Duration = Duration::from_millis(2100);

/// A keystroke counts as the one that started something when it was seen
/// this short a time before.
const SAME_MOMENT: Duration = Duration::from_millis(100);

/// What the view is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModeKind {
    /// The list, with the keyboard in the query field.
    List,
    /// Waiting for the next keystroke.
    Recording,
    /// Asking something: what to do with the key that was pressed, whether
    /// to put everything back, what to do with a file.
    Question,
}

/// One line of the list as it is drawn.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum DisplayRow {
    /// A line of the host above the list.
    Note(SharedString),
    /// A heading: place and category.
    Header(SharedString),
    /// A command: the index of its entry, and the letters of its title
    /// that matched the query.
    Command { entry: usize, positions: Vec<usize> },
}

/// The view waits for a key.
pub(crate) struct Recording {
    pub target: Target,
    /// The key the new one takes the place of; `None` for one more key.
    pub replacing: Option<KeySequence>,
    /// The modifiers held right now.
    pub held: gpui::Modifiers,
    /// Why the last key pressed was not taken.
    pub problem: Option<SharedString>,
}

pub(crate) enum Mode {
    List,
    Recording(Recording),
    Asking(Question),
}

/// A key that is taken to be still held down: its repeats are not keys.
struct HeldKey {
    keystroke: gpui::Keystroke,
    last_seen: Instant,
}

/// The "Keyboard shortcuts" view. See the [crate docs](crate).
pub struct KeymapView {
    pub(crate) listing: Listing,
    pub(crate) entries: Vec<Entry>,
    pub(crate) rows: Vec<DisplayRow>,
    /// The selected row, an index into `rows`.
    pub(crate) selected: Option<usize>,
    /// Which key of the selected row the keys act on.
    pub(crate) selected_key: usize,
    pub(crate) changed_only: bool,
    pub(crate) mode: Mode,
    pub(crate) query_field: Entity<TextField>,
    pub(crate) capture_focus: FocusHandle,
    pub(crate) choices_focus: FocusHandle,
    pub(crate) scroll: UniformListScrollHandle,
    /// Where the selected row was drawn, for a menu opened from the keys
    /// when its "more" control is not drawn.
    pub(crate) selected_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// Where the "more" control of the selected row was drawn: the row's
    /// menu opens under it.
    pub(crate) more_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// How many rows the list shows before it scrolls.
    pub(crate) visible_rows: usize,
    window_id: WindowId,
    last_key: Option<(gpui::Keystroke, Instant)>,
    held: Option<HeldKey>,
    dismissed: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DismissEvent> for KeymapView {}
impl EventEmitter<Request> for KeymapView {}

impl Focusable for KeymapView {
    /// Where the keyboard is in the view right now: the query field while
    /// the list is shown.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        match self.mode {
            Mode::List => self.query_field.focus_handle(cx),
            Mode::Recording(_) => self.capture_focus.clone(),
            Mode::Asking(_) => self.choices_focus.clone(),
        }
    }
}

impl KeymapView {
    /// The view over `listing`, with the first row selected.
    pub fn new(listing: Listing, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query_field = cx.new(|cx| {
            let mut field = TextField::new(window, cx)
                .with_placeholder("Search by name, place or key")
                .with_appearance(FieldAppearance::Borderless)
                .with_text_size(UI_TEXT_LG);
            field.set_list_navigation(true);
            field
        });
        let subscription = cx.subscribe_in(
            &query_field,
            window,
            |this, _, event: &TextFieldEvent, window, cx| match event {
                TextFieldEvent::Changed => this.rebuild_rows(None, cx),
                // Only reached when the key of `Close` was taken away: the
                // field's own `escape` still closes.
                TextFieldEvent::Cancelled => this.close(window, cx),
                _ => {}
            },
        );
        let weak = cx.weak_entity();
        let window_id = window.window_handle().window_id();
        record::register(weak.clone(), window, cx);
        cx.on_release(move |this, cx| record::unregister(&weak, this.window_id, cx))
            .detach();
        let entries = entries(&listing);
        let mut view = Self {
            listing,
            entries,
            rows: Vec::new(),
            selected: None,
            selected_key: 0,
            changed_only: false,
            mode: Mode::List,
            query_field,
            capture_focus: cx.focus_handle(),
            choices_focus: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
            selected_bounds: Rc::new(Cell::new(None)),
            more_bounds: Rc::new(Cell::new(None)),
            visible_rows: 12,
            window_id,
            last_key: None,
            held: None,
            dismissed: false,
            _subscriptions: vec![subscription],
        };
        view.rebuild_rows(None, cx);
        view
    }

    // ---------------------------------------------------------------------
    // The host's side
    // ---------------------------------------------------------------------

    /// The keymap changed: shows `listing` in the place of what was shown.
    /// The query, the changed-only switch and the selection (the same
    /// command in the same place) stay; an open import preview stays too,
    /// a question about a key does not (it was about the keymap as it
    /// was).
    pub fn set_listing(&mut self, listing: Listing, window: &mut Window, cx: &mut Context<Self>) {
        let keep = self.selected_target().cloned();
        let key = self.selected_key;
        self.entries = entries(&listing);
        self.listing = listing;
        let stays = matches!(
            &self.mode,
            Mode::Asking(Question {
                subject: Subject::Import(_) | Subject::ImportError,
                ..
            })
        );
        if !stays && !matches!(self.mode, Mode::List) {
            self.back_to_list(window, cx);
        }
        self.rebuild_rows(keep.as_ref(), cx);
        self.selected_key = key.min(self.selected_keys().len().saturating_sub(1));
    }

    /// Shows what importing the file called `file` would do, with the
    /// ways to carry it out. The answer comes back as
    /// [`Request::Import`].
    pub fn show_import_preview(
        &mut self,
        file: impl Into<SharedString>,
        plan: ImportPlan,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let file = file.into();
        let question = Question::about_import(&file, plan, &self.names());
        self.ask(question, window, cx);
    }

    /// Says that the file called `file` is not a shortcuts file, in the
    /// words of `error` (what is wrong and where), and offers to choose
    /// another one.
    pub fn show_import_error(
        &mut self,
        file: impl Into<SharedString>,
        error: &ImportError,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let file = file.into();
        self.ask(Question::about_import_error(&file, error), window, cx);
    }

    // ---------------------------------------------------------------------
    // State, for the host's scripted runs and for tests
    // ---------------------------------------------------------------------

    /// What the view is doing.
    pub fn mode(&self) -> ModeKind {
        match self.mode {
            Mode::List => ModeKind::List,
            Mode::Recording(_) => ModeKind::Recording,
            Mode::Asking(_) => ModeKind::Question,
        }
    }

    /// The text in the query field.
    pub fn query(&self, cx: &App) -> String {
        self.query_field.read(cx).text().to_string()
    }

    /// Puts `query` into the query field and filters the list.
    pub fn set_query(&mut self, query: impl Into<String>, cx: &mut Context<Self>) {
        let query = query.into();
        self.query_field
            .update(cx, |field, cx| field.set_text(query, cx));
        self.rebuild_rows(None, cx);
    }

    /// Whether only the changed rows are shown.
    pub fn changed_only(&self) -> bool {
        self.changed_only
    }

    /// Shows only the rows the user changed, or all of them.
    pub fn set_changed_only(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.changed_only != on {
            let keep = self.selected_target().cloned();
            self.changed_only = on;
            self.rebuild_rows(keep.as_ref(), cx);
        }
    }

    /// How many of the rows differ from their defaults.
    pub fn changed_count(&self) -> usize {
        self.entries.iter().filter(|entry| entry.changed).count()
    }

    /// What the line under the query says about the user's changes: "No
    /// key changed", "1 command changed", "4 commands changed".
    ///
    /// The number is that of the rows the user changed. Commands that are
    /// changed and have no row right now are said beside it ("4 commands
    /// changed, 1 more not shown now"), so the line agrees with what
    /// "Reset All" says it puts back.
    pub fn changed_text(&self) -> String {
        let listed = self.changed_count();
        let hidden = self.listing.changed_commands().saturating_sub(listed);
        crate::words::changed_summary(listed, hidden)
    }

    /// What the view says while it waits for a key, under its heading:
    /// that every key counts and how to leave without a change. `None`
    /// while it is not waiting.
    pub fn waiting_says(&self, cx: &App) -> Option<String> {
        matches!(self.mode, Mode::Recording(_)).then(|| self.waiting_help(cx))
    }

    /// The commands the list shows now, in order, with their headings left
    /// out: the target and the title of each.
    pub fn listed(&self) -> Vec<(Target, String)> {
        self.rows
            .iter()
            .filter_map(|row| match row {
                DisplayRow::Command { entry, .. } => {
                    let row = &self.entries[*entry].row;
                    Some((row.target.clone(), row.title.to_string()))
                }
                _ => None,
            })
            .collect()
    }

    /// The letters of each listed command's title that the search marks,
    /// in the order of [`KeymapView::listed`]: the letters that matched
    /// the query, as texts ("Ital" of "Italic"; nothing for a row found by
    /// its key or its place).
    pub fn marked_letters(&self) -> Vec<String> {
        self.rows
            .iter()
            .filter_map(|row| match row {
                DisplayRow::Command { entry, positions } => {
                    let title = &self.entries[*entry].row.title;
                    Some(
                        positions
                            .iter()
                            .filter_map(|at| title[*at..].chars().next())
                            .collect(),
                    )
                }
                _ => None,
            })
            .collect()
    }

    /// The headings and notes the list shows now, in order.
    pub fn headings(&self) -> Vec<String> {
        self.rows
            .iter()
            .filter_map(|row| match row {
                DisplayRow::Header(text) | DisplayRow::Note(text) => Some(text.to_string()),
                DisplayRow::Command { .. } => None,
            })
            .collect()
    }

    /// The position of `target`'s row in the list as it is drawn (what the
    /// debug selector `keymap-row-<n>` counts), if it is listed.
    pub fn row_of(&self, target: &Target) -> Option<usize> {
        self.rows.iter().position(|row| {
            matches!(row, DisplayRow::Command { entry, .. } if self.entries[*entry].row.target == *target)
        })
    }

    /// The command of the selected row.
    pub fn selected_target(&self) -> Option<&Target> {
        self.selected_entry().map(|entry| &entry.row.target)
    }

    /// Selects the row of `target`. `false` when it is not listed (the
    /// query or the changed-only switch leaves it out).
    pub fn select_target(&mut self, target: &Target, cx: &mut Context<Self>) -> bool {
        match self.row_of(target) {
            Some(row) => {
                self.select_row(row, ScrollStrategy::Center, cx);
                true
            }
            None => false,
        }
    }

    /// The keys of the selected row, in the order it shows them.
    pub fn selected_keys(&self) -> Vec<KeySequence> {
        self.selected_entry()
            .map(|entry| entry.keys.iter().map(|key| key.keys.clone()).collect())
            .unwrap_or_default()
    }

    /// The command the view waits for a key for.
    pub fn recording_target(&self) -> Option<&Target> {
        match &self.mode {
            Mode::Recording(recording) => Some(&recording.target),
            _ => None,
        }
    }

    /// The open question in a few words ("⌘I is taken").
    pub fn question_title(&self) -> Option<String> {
        match &self.mode {
            Mode::Asking(question) => Some(question.title.to_string()),
            _ => None,
        }
    }

    /// How the title of the open question is to be read: the mark beside
    /// it (a tick, a warning sign, a stop sign, or none for
    /// [`Tone::Plain`]).
    pub fn question_mark(&self) -> Option<Tone> {
        match &self.mode {
            Mode::Asking(question) => Some(question.tone),
            _ => None,
        }
    }

    /// The sentences of the open question.
    pub fn question_lines(&self) -> Vec<Line> {
        match &self.mode {
            Mode::Asking(question) => question.lines.clone(),
            _ => Vec::new(),
        }
    }

    /// The choices of the open question with their labels, in order.
    pub fn choices(&self) -> Vec<(Choice, String)> {
        match &self.mode {
            Mode::Asking(question) => question
                .choices
                .iter()
                .map(|button| (button.choice, button.label.to_string()))
                .collect(),
            _ => Vec::new(),
        }
    }

    /// The choice that is marked: what the key that chooses takes.
    pub fn marked_choice(&self) -> Option<Choice> {
        match &self.mode {
            Mode::Asking(question) => Some(question.selected_choice()),
            _ => None,
        }
    }

    /// What stands under the rows of the import preview for the selected
    /// row: how the file names it (`In the file: entry 51,
    /// "editor::ToggleInlineCode"`), for whoever edits the file.
    pub fn preview_detail(&self) -> Option<String> {
        match &self.mode {
            Mode::Asking(Question {
                subject: Subject::Import(preview),
                ..
            }) => preview
                .selected
                .and_then(|selected| preview.rows.get(selected))
                .and_then(|row| row.detail()),
            _ => None,
        }
    }

    /// The rows of the import preview as sentences, headings included, as
    /// they read on screen: `Bold (in the text): ⌘B → ⌥⌘B`; for a skipped
    /// entry what the command is called, then the reason (`Italic: I is a
    /// character, ...`).
    pub fn preview_rows(&self) -> Vec<String> {
        match &self.mode {
            Mode::Asking(Question {
                subject: Subject::Import(preview),
                ..
            }) => preview.rows.iter().map(|row| row.text()).collect(),
            _ => Vec::new(),
        }
    }

    /// One line about the view, for the `state` step of a scripted run.
    pub fn describe(&self, cx: &App) -> String {
        let mut out = format!(
            "rows={} changed={} changed_only={} query={:?}",
            self.listed().len(),
            self.changed_count(),
            self.changed_only,
            self.query(cx),
        );
        if let Some(entry) = self.selected_entry() {
            out.push_str(&format!(
                " selected={:?} keys=[{}] default=[{}]{}",
                entry.row.title,
                key_texts(
                    &entry
                        .keys
                        .iter()
                        .map(|key| key.keys.clone())
                        .collect::<Vec<_>>()
                ),
                key_texts(&entry.default_keys),
                if entry.changed { " (changed)" } else { "" },
            ));
            if let Some(reason) = entry.locked_reason() {
                out.push_str(&format!(" locked={reason:?}"));
            }
            if let Some(clash) = &entry.clash {
                out.push_str(&format!(" clash={clash:?}"));
            }
        }
        match &self.mode {
            Mode::List => out.insert_str(0, "mode=list "),
            Mode::Recording(recording) => {
                out.insert_str(0, "mode=recording ");
                out.push_str(&format!(
                    " for={:?} replacing={}",
                    self.names().title(&recording.target),
                    recording
                        .replacing
                        .as_ref()
                        .map(|key| key.to_string())
                        .unwrap_or_else(|| "nothing".into()),
                ));
                let held = keys::modifier_glyphs(&recording.held);
                if !held.is_empty() {
                    out.push_str(&format!(" held={held}"));
                }
                if let Some(problem) = &recording.problem {
                    out.push_str(&format!(" problem={problem:?}"));
                }
            }
            Mode::Asking(question) => {
                out.insert_str(0, "mode=question ");
                let choices: Vec<String> = question
                    .choices
                    .iter()
                    .enumerate()
                    .map(|(ix, button)| {
                        if ix == question.selected {
                            format!("*{}", button.label)
                        } else {
                            button.label.to_string()
                        }
                    })
                    .collect();
                let lines: Vec<&str> = question
                    .lines
                    .iter()
                    .map(|line| line.text.as_str())
                    .collect();
                out.push_str(&format!(
                    " question={:?} mark={} says={:?} choices=[{}]",
                    question.title,
                    match question.tone {
                        Tone::Plain => "none",
                        Tone::Good => "good",
                        Tone::Warning => "warning",
                        Tone::Stop => "stop",
                    },
                    lines.join(" "),
                    choices.join(" | "),
                ));
                if let Subject::Import(preview) = &question.subject {
                    out.push_str(&format!(" preview_rows={}", preview.rows.len()));
                }
            }
        }
        out
    }

    /// Presses the control with this label, as a click on it would: a
    /// choice of the open question ("Use ⌥⌘B", "Cancel"), "Cancel" while
    /// the view waits for a key, or one of the view's own controls
    /// ("Import…", "Export…", "Reset All…", "Changed Only", "Close").
    /// `false` when nothing with that label can be pressed now.
    pub fn press(&mut self, label: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        match &self.mode {
            Mode::Asking(question) => {
                let found = question
                    .choices
                    .iter()
                    .find(|button| button.label.as_ref() == label)
                    .map(|button| button.choice);
                match found {
                    Some(choice) => self.choose(choice, window, cx),
                    None => false,
                }
            }
            Mode::Recording(_) if label == "Cancel" => {
                self.cancel(window, cx);
                true
            }
            Mode::Recording(_) => false,
            Mode::List => match label {
                "Import\u{2026}" => {
                    self.import(window, cx);
                    true
                }
                "Export\u{2026}" => {
                    self.export(window, cx);
                    true
                }
                "Reset All\u{2026}" => self.reset_all(window, cx),
                "Changed Only" => {
                    self.set_changed_only(!self.changed_only, cx);
                    true
                }
                "Close" => {
                    self.close(window, cx);
                    true
                }
                _ => false,
            },
        }
    }

    /// Says that the key the view took to be held down was released.
    /// GPUI reports releases of real keys to the view itself; keystrokes a
    /// script or a test dispatches come without one, and the same key
    /// pressed again right away would be taken for a repeat.
    pub fn key_released(&mut self) {
        self.held = None;
    }

    // ---------------------------------------------------------------------
    // The list
    // ---------------------------------------------------------------------

    pub(crate) fn names(&self) -> Names<'_> {
        Names {
            entries: &self.entries,
            catalogue: &self.listing.catalogue,
        }
    }

    pub(crate) fn selected_entry(&self) -> Option<&Entry> {
        match self.rows.get(self.selected?)? {
            DisplayRow::Command { entry, .. } => self.entries.get(*entry),
            _ => None,
        }
    }

    /// Whether the list is shown without a query: with its notes and
    /// headings.
    pub(crate) fn browsing(&self, cx: &App) -> bool {
        self.query_field.read(cx).text().trim().is_empty()
    }

    /// Filters the list for the query and the changed-only switch, and
    /// selects the row of `keep` (the first row without it, or when it is
    /// not listed any more).
    fn rebuild_rows(&mut self, keep: Option<&Target>, cx: &mut Context<Self>) {
        let query = self.query(cx);
        let Found { matches, close } = search(&self.entries, &query, self.changed_only);
        let mut rows = Vec::with_capacity(matches.len() + 16);
        if query.trim().is_empty() {
            if !self.changed_only {
                rows.extend(self.listing.notes.iter().cloned().map(DisplayRow::Note));
                rows.extend(inactive_note(&self.listing).map(DisplayRow::Note));
            }
            let mut heading: Option<&SharedString> = None;
            for matched in matches {
                let entry = &self.entries[matched.entry];
                if heading != Some(&entry.row.heading) {
                    heading = Some(&entry.row.heading);
                    rows.push(DisplayRow::Header(entry.row.heading.clone()));
                }
                rows.push(DisplayRow::Command {
                    entry: matched.entry,
                    positions: Vec::new(),
                });
            }
        } else {
            if close {
                rows.push(DisplayRow::Note(
                    crate::words::close_matches_note(query.trim()).into(),
                ));
            }
            rows.extend(matches.into_iter().map(|matched| DisplayRow::Command {
                entry: matched.entry,
                positions: matched.positions,
            }));
        }
        self.rows = rows;
        let kept = keep.and_then(|target| self.row_of(target));
        match kept {
            Some(row) => {
                self.selected = Some(row);
                self.scroll.scroll_to_item(row, ScrollStrategy::Center);
            }
            None => {
                self.selected = self.next_command(0);
                self.selected_key = self.selected_entry().map_or(0, Entry::main_key);
                // Show what stands above the first row.
                self.scroll.scroll_to_item_strict(0, ScrollStrategy::Top);
            }
        }
        cx.notify();
    }

    fn is_command(&self, row: usize) -> bool {
        matches!(self.rows.get(row), Some(DisplayRow::Command { .. }))
    }

    fn next_command(&self, from: usize) -> Option<usize> {
        (from..self.rows.len()).find(|row| self.is_command(*row))
    }

    fn previous_command(&self, from: usize) -> Option<usize> {
        if self.rows.is_empty() {
            return None;
        }
        (0..=from.min(self.rows.len() - 1))
            .rev()
            .find(|row| self.is_command(*row))
    }

    pub(crate) fn select_row(
        &mut self,
        row: usize,
        strategy: ScrollStrategy,
        cx: &mut Context<Self>,
    ) {
        if !self.is_command(row) {
            return;
        }
        // Scrolling up to a row shows the heading it stands under.
        let target = if strategy == ScrollStrategy::Top
            && row > 0
            && matches!(self.rows[row - 1], DisplayRow::Header(_))
        {
            row - 1
        } else {
            row
        };
        self.scroll.scroll_to_item(target, strategy);
        if self.selected != Some(row) {
            self.selected = Some(row);
            self.selected_key = self.selected_entry().map_or(0, Entry::main_key);
        }
        cx.notify();
    }

    pub(crate) fn select_next(&mut self, cx: &mut Context<Self>) {
        let from = self.selected.map_or(0, |row| row + 1);
        if let Some(row) = self.next_command(from) {
            self.select_row(row, ScrollStrategy::Bottom, cx);
        }
    }

    pub(crate) fn select_previous(&mut self, cx: &mut Context<Self>) {
        let Some(current) = self.selected else {
            return self.select_first(cx);
        };
        if current > 0
            && let Some(row) = self.previous_command(current - 1)
        {
            self.select_row(row, ScrollStrategy::Top, cx);
        }
    }

    pub(crate) fn select_first(&mut self, cx: &mut Context<Self>) {
        if let Some(row) = self.next_command(0) {
            self.select_row(row, ScrollStrategy::Top, cx);
            self.scroll.scroll_to_item_strict(0, ScrollStrategy::Top);
        }
    }

    pub(crate) fn select_last(&mut self, cx: &mut Context<Self>) {
        if let Some(row) = self.previous_command(self.rows.len().saturating_sub(1)) {
            self.select_row(row, ScrollStrategy::Bottom, cx);
        }
    }

    pub(crate) fn select_page(&mut self, down: bool, cx: &mut Context<Self>) {
        if self.rows.is_empty() {
            return;
        }
        let step = self.visible_rows.saturating_sub(1).max(1);
        let current = self.selected.unwrap_or(0);
        if down {
            let target = (current + step).min(self.rows.len() - 1);
            if let Some(row) = self
                .next_command(target)
                .or_else(|| self.previous_command(target))
            {
                self.select_row(row, ScrollStrategy::Bottom, cx);
            }
        } else {
            let target = current.saturating_sub(step);
            if let Some(row) = self
                .previous_command(target)
                .or_else(|| self.next_command(target))
            {
                self.select_row(row, ScrollStrategy::Top, cx);
            }
        }
    }

    /// Moves to the next (or previous) key of a row that has several.
    pub(crate) fn step_key(&mut self, forward: bool, cx: &mut Context<Self>) {
        let count = self.selected_keys().len();
        if count < 2 {
            return;
        }
        self.selected_key = if forward {
            (self.selected_key + 1) % count
        } else {
            (self.selected_key + count - 1) % count
        };
        cx.notify();
    }

    // ---------------------------------------------------------------------
    // What a row offers
    // ---------------------------------------------------------------------

    /// The key of the selected row the keys act on.
    fn acting_key(&self) -> Option<KeySequence> {
        let keys = self.selected_keys();
        keys.get(self.selected_key.min(keys.len().saturating_sub(1)))
            .cloned()
    }

    /// Waits for a key to take the place of the selected row's key (for
    /// its first key, on a row without one). `false` on a row that cannot
    /// be changed.
    pub fn change_key(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(entry) = self.selected_entry() else {
            return false;
        };
        if !entry.changeable() {
            return false;
        }
        let target = entry.row.target.clone();
        let replacing = self.acting_key();
        self.start_recording(target, replacing, window, cx);
        true
    }

    /// Waits for one more key for the selected row.
    pub fn add_key(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(entry) = self.selected_entry() else {
            return false;
        };
        if !entry.changeable() {
            return false;
        }
        let target = entry.row.target.clone();
        self.start_recording(target, None, window, cx);
        true
    }

    /// Takes the selected key from the selected row. `false` when there
    /// is none to take.
    pub fn remove_key(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(entry) = self.selected_entry() else {
            return false;
        };
        if !entry.changeable() {
            return false;
        }
        let target = entry.row.target.clone();
        let Some(key) = self.acting_key() else {
            return false;
        };
        cx.emit(Request::RemoveKey { target, key });
        true
    }

    /// Puts the selected row back to its default keys. `false` when it
    /// has them.
    pub fn use_default(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(entry) = self.selected_entry() else {
            return false;
        };
        if !entry.changeable() || !entry.changed {
            return false;
        }
        cx.emit(Request::UseDefault {
            target: entry.row.target.clone(),
        });
        true
    }

    /// Asks the host to run the command of the selected row.
    pub fn run_command(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(entry) = self.selected_entry() else {
            return false;
        };
        if !entry.row.runnable {
            return false;
        }
        cx.emit(Request::Run {
            target: entry.row.target.clone(),
        });
        true
    }

    /// The menu of the selected row: at `position` for a right-click; for
    /// the keys under the row's "more" control at the right (where it
    /// covers the least of the rows below), with its first item marked.
    pub fn open_row_menu(
        &mut self,
        position: Option<Point<Pixels>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match position {
            Some(position) => self.open_row_menu_at(MenuAnchor::Point(position), false, window, cx),
            None => {
                let anchor = match (self.more_bounds.get(), self.selected_bounds.get()) {
                    (Some(control), _) => MenuAnchor::Below(control),
                    (None, Some(row)) => MenuAnchor::Below(row),
                    (None, None) => MenuAnchor::Point(Point::default()),
                };
                self.open_row_menu_at(anchor, true, window, cx)
            }
        }
    }

    /// The menu of the selected row at `anchor`; `marked`: with its first
    /// item marked, as a menu opened from the keys has it.
    pub(crate) fn open_row_menu_at(
        &mut self,
        anchor: MenuAnchor,
        marked: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !matches!(self.mode, Mode::List) {
            return false;
        }
        let Some(row) = self.selected else {
            return false;
        };
        let DisplayRow::Command { entry, .. } = self.rows[row] else {
            return false;
        };
        let items = self.row_menu_items(entry, cx);
        let menu = ContextMenu::open(anchor, items, window, cx);
        if marked {
            menu.update(cx, |menu, cx| menu.select_first(cx));
        }
        true
    }

    fn row_menu_items(&self, entry: usize, cx: &mut Context<Self>) -> Vec<ContextMenuItem> {
        let entry = &self.entries[entry];
        let view = cx.weak_entity();
        let target = entry.row.target.clone();
        let mut items = Vec::new();
        let record = |label: String, replacing: Option<KeySequence>| {
            let view = view.clone();
            let target = target.clone();
            ContextMenuItem::new(label, move |window, cx| {
                view.update(cx, |view, cx| {
                    view.start_recording(target.clone(), replacing.clone(), window, cx)
                })
                .ok();
            })
        };
        let remove = |label: String, key: KeySequence| {
            let view = view.clone();
            let target = target.clone();
            ContextMenuItem::new(label, move |_, cx| {
                view.update(cx, |_, cx| {
                    cx.emit(Request::RemoveKey {
                        target: target.clone(),
                        key: key.clone(),
                    })
                })
                .ok();
            })
        };
        if let Some(reason) = entry.locked_reason() {
            items.push(ContextMenuItem::section("Cannot be changed here"));
            items.push(ContextMenuItem::new(reason, |_, _| {}).disabled(true));
        } else {
            let change = keys::hint_in(&ChangeKey, &LIST_STACK, cx);
            let add = keys::hint_in(&AddKey, &LIST_STACK, cx);
            let take = keys::hint_in(&RemoveKey, &LIST_STACK, cx);
            match entry.keys.as_slice() {
                [] => items.push(record("Add a Key\u{2026}".into(), None).key_hint(add)),
                [only] => {
                    items.push(
                        record("Change Key\u{2026}".into(), Some(only.keys.clone()))
                            .key_hint(change),
                    );
                    items.push(record("Add Another Key\u{2026}".into(), None).key_hint(add));
                    items.push(remove("Remove Key".into(), only.keys.clone()).key_hint(take));
                }
                several => {
                    // The keys of the list act on one of the keys (the
                    // one that is marked in the row): its two items show
                    // them.
                    let acting = self.selected_key.min(several.len() - 1);
                    for (k, key) in several.iter().enumerate() {
                        items.push(
                            record(
                                format!("Change {}\u{2026}", glyphs(&key.keys)),
                                Some(key.keys.clone()),
                            )
                            .key_hint(if k == acting {
                                change.clone()
                            } else {
                                None
                            }),
                        );
                    }
                    items.push(record("Add Another Key\u{2026}".into(), None).key_hint(add));
                    for (k, key) in several.iter().enumerate() {
                        items.push(
                            remove(format!("Remove {}", glyphs(&key.keys)), key.keys.clone())
                                .key_hint(if k == acting { take.clone() } else { None }),
                        );
                    }
                }
            }
            items.push(ContextMenuItem::separator());
            let view = view.clone();
            let default_target = target.clone();
            items.push(
                ContextMenuItem::new(
                    format!("Use Default ({})", default_label(&entry.default_keys)),
                    move |_, cx| {
                        view.update(cx, |_, cx| {
                            cx.emit(Request::UseDefault {
                                target: default_target.clone(),
                            })
                        })
                        .ok();
                    },
                )
                .key_hint(keys::hint_in(&UseDefault, &LIST_STACK, cx))
                .disabled(!entry.changed),
            );
        }
        if entry.row.runnable {
            items.push(ContextMenuItem::separator());
            let view = cx.weak_entity();
            items.push(
                ContextMenuItem::new("Run Command", move |_, cx| {
                    view.update(cx, |_, cx| {
                        cx.emit(Request::Run {
                            target: target.clone(),
                        })
                    })
                    .ok();
                })
                .key_hint(keys::hint_in(&RunCommand, &LIST_STACK, cx)),
            );
        }
        items
    }

    // ---------------------------------------------------------------------
    // The view's own controls
    // ---------------------------------------------------------------------

    /// Asks the host to let the user choose a file to import.
    pub fn import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.mode, Mode::List) {
            self.back_to_list(window, cx);
        }
        cx.emit(Request::ChooseImportFile);
    }

    /// Asks the host to export the shortcuts.
    pub fn export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.mode, Mode::List) {
            self.back_to_list(window, cx);
        }
        cx.emit(Request::Export);
    }

    /// Whether there is anything "Reset All" would put back.
    pub fn can_reset_all(&self) -> bool {
        !self.listing.keymap.changes().is_empty()
    }

    /// Asks whether to put every key back to its default, saying how many
    /// commands that concerns. `false` when nothing was changed.
    pub fn reset_all(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.can_reset_all() {
            return false;
        }
        let changed = self.listing.changed_commands();
        self.ask(Question::about_reset(changed), window, cx);
        true
    }

    /// Closes the view.
    pub fn close(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if !std::mem::replace(&mut self.dismissed, true) {
            cx.emit(DismissEvent);
        }
    }

    // ---------------------------------------------------------------------
    // Waiting for a key
    // ---------------------------------------------------------------------

    /// Waits for the next keystroke, to give it to `target`: in the place
    /// of its key `replacing`, or as one more key.
    pub fn start_recording(
        &mut self,
        target: Target,
        replacing: Option<KeySequence>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A menu would see the key first.
        ContextMenu::dismiss(window, cx);
        if let Some(row) = self.row_of(&target) {
            self.selected = Some(row);
        }
        // The key that started this is still down, and its repeats are not
        // the new key.
        self.held = self
            .last_key
            .take()
            .filter(|(_, at)| at.elapsed() < SAME_MOMENT)
            .map(|(keystroke, at)| HeldKey {
                keystroke,
                last_seen: at,
            });
        self.mode = Mode::Recording(Recording {
            target,
            replacing,
            held: window.modifiers(),
            problem: None,
        });
        window.focus(&self.capture_focus);
        cx.notify();
    }

    /// A key press of the view's window, before anything else sees it.
    /// Returns whether the view took it.
    pub(crate) fn intercept(
        &mut self,
        keystroke: &gpui::Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if keys::is_modifier_alone(keystroke) {
            return matches!(self.mode, Mode::Recording(_));
        }
        if let Some(held) = &mut self.held {
            if keys::same_press(&held.keystroke, keystroke)
                && held.last_seen.elapsed() < HELD_KEY_PATIENCE
            {
                held.last_seen = Instant::now();
                return true;
            }
            self.held = None;
        }
        match &mut self.mode {
            Mode::Recording(_) if !self.capture_focus.is_focused(window) => {
                // The keyboard went elsewhere (a command of the menu bar):
                // the waiting is over, and the key is not the view's.
                self.mode = Mode::List;
                cx.notify();
                false
            }
            Mode::Recording(recording) => {
                match keys::from_pressed(keystroke) {
                    Ok(key) => {
                        let target = recording.target.clone();
                        let replacing = recording.replacing.clone();
                        self.held = Some(HeldKey {
                            keystroke: keystroke.clone(),
                            last_seen: Instant::now(),
                        });
                        self.recorded(target, replacing, key, window, cx);
                    }
                    Err(error) => {
                        recording.problem =
                            Some(format!("That key cannot be used: {error}.").into());
                        cx.notify();
                    }
                }
                true
            }
            _ => {
                self.last_key = Some((keystroke.clone(), Instant::now()));
                false
            }
        }
    }

    /// The key `key` was pressed for `target`: asks what to do with it.
    fn recorded(
        &mut self,
        target: Target,
        replacing: Option<KeySequence>,
        key: KeySequence,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let verdict = self.listing.keymap.check(&target, &key, replacing.as_ref());
        // What the keymap cannot know: something of the host's may take
        // the key before any binding sees it.
        let seen_first: Vec<SeenFirst> = if verdict.refusal.is_none() {
            self.listing
                .seeing_first(&target, &key)
                .into_iter()
                .map(|taker| SeenFirst {
                    name: taker.name.as_ref().map(|name| name.to_string()),
                    warning: taker.warning.to_string(),
                })
                .collect()
        } else {
            Vec::new()
        };
        let question = Question::about_key(target, replacing, verdict, &seen_first, &self.names());
        self.ask(question, window, cx);
    }

    /// A key was released: it is not held any more.
    pub(crate) fn released(&mut self, keystroke: &gpui::Keystroke) {
        if self
            .held
            .as_ref()
            .is_some_and(|held| held.keystroke.key == keystroke.key)
        {
            self.held = None;
        }
    }

    /// The modifiers held changed.
    pub(crate) fn modifiers_changed(&mut self, modifiers: gpui::Modifiers, cx: &mut Context<Self>) {
        if self
            .held
            .as_ref()
            .is_some_and(|held| held.keystroke.modifiers != modifiers)
        {
            self.held = None;
        }
        if let Mode::Recording(recording) = &mut self.mode {
            recording.held = modifiers;
            cx.notify();
        }
    }

    // ---------------------------------------------------------------------
    // Questions
    // ---------------------------------------------------------------------

    fn ask(&mut self, question: Question, window: &mut Window, cx: &mut Context<Self>) {
        ContextMenu::dismiss(window, cx);
        self.mode = Mode::Asking(question);
        window.focus(&self.choices_focus);
        cx.notify();
    }

    pub(crate) fn back_to_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // The key that answered is still down: its repeats are not keys of
        // the list (a held `enter` would start changing the key again).
        self.held = self
            .last_key
            .take()
            .filter(|(_, at)| at.elapsed() < SAME_MOMENT)
            .map(|(keystroke, at)| HeldKey {
                keystroke,
                last_seen: at,
            });
        self.mode = Mode::List;
        let focus = self.query_field.focus_handle(cx);
        window.focus(&focus);
        cx.notify();
    }

    /// Leaves the waiting for a key, or the open question, without
    /// changing anything. With the list shown it closes the view.
    pub fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.mode {
            Mode::List => self.close(window, cx),
            _ => self.back_to_list(window, cx),
        }
    }

    /// Marks the next (or previous) choice of the open question.
    pub(crate) fn step_choice(&mut self, forward: bool, cx: &mut Context<Self>) {
        let Mode::Asking(question) = &self.mode else {
            return;
        };
        let count = question.choices.len();
        let next = if forward {
            (question.selected + 1) % count
        } else {
            (question.selected + count - 1) % count
        };
        self.mark_choice(next, cx);
    }

    pub(crate) fn mark_choice(&mut self, index: usize, cx: &mut Context<Self>) {
        let names = Names {
            entries: &self.entries,
            catalogue: &self.listing.catalogue,
        };
        if let Mode::Asking(question) = &mut self.mode
            && question.selected != index
        {
            question.select(index, &names);
            cx.notify();
        }
    }

    /// Takes the marked choice.
    pub(crate) fn choose_marked(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(choice) = self.marked_choice() {
            self.choose(choice, window, cx);
        }
    }

    /// Takes `choice` of the open question. `false` when the question
    /// does not offer it.
    pub fn choose(&mut self, choice: Choice, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Mode::Asking(question) = &self.mode else {
            return false;
        };
        if !question
            .choices
            .iter()
            .any(|button| button.choice == choice)
        {
            return false;
        }
        match (choice, &question.subject) {
            (
                Choice::UseKey | Choice::KeepBoth | Choice::TakeKey,
                Subject::Key {
                    target,
                    replacing,
                    verdict,
                },
            ) => {
                let take = match choice {
                    Choice::TakeKey if !verdict.taken.is_empty() => verdict.taken.clone(),
                    Choice::TakeKey => crate::words::overlapping(verdict).cloned().collect(),
                    _ => Vec::new(),
                };
                let request = match replacing {
                    Some(old) => Request::ReplaceKey {
                        target: target.clone(),
                        old: old.clone(),
                        new: verdict.key.clone(),
                        take,
                    },
                    None => Request::AddKey {
                        target: target.clone(),
                        new: verdict.key.clone(),
                        take,
                    },
                };
                self.back_to_list(window, cx);
                cx.emit(request);
            }
            (
                Choice::PressAnotherKey,
                Subject::Key {
                    target, replacing, ..
                },
            ) => {
                let (target, replacing) = (target.clone(), replacing.clone());
                self.start_recording(target, replacing, window, cx);
            }
            (Choice::ResetAll, Subject::ResetAll) => {
                self.back_to_list(window, cx);
                cx.emit(Request::ResetAll);
            }
            (Choice::ImportMerge | Choice::ImportReplace, Subject::Import(preview)) => {
                let request = Request::Import {
                    plan: Box::new(preview.plan.clone()),
                    mode: if choice == Choice::ImportReplace {
                        ImportMode::Replace
                    } else {
                        ImportMode::Merge
                    },
                };
                self.back_to_list(window, cx);
                cx.emit(request);
            }
            (Choice::ChooseFile, _) => {
                self.back_to_list(window, cx);
                cx.emit(Request::ChooseImportFile);
            }
            (Choice::Cancel, _) => self.back_to_list(window, cx),
            _ => return false,
        }
        true
    }

    /// The import preview of the open question.
    pub(crate) fn preview_mut(&mut self) -> Option<&mut Preview> {
        match &mut self.mode {
            Mode::Asking(Question {
                subject: Subject::Import(preview),
                ..
            }) => Some(preview),
            _ => None,
        }
    }

    /// Moves the selection of the import preview by `delta` rows, and
    /// scrolls as far as it takes to show the row.
    pub(crate) fn move_in_preview(&mut self, delta: isize, cx: &mut Context<Self>) {
        if let Some(preview) = self.preview_mut() {
            preview.move_selection(delta);
            if let Some(selected) = preview.selected {
                // The heading above the first row of a group comes along.
                let target = if selected > 0 && preview.rows[selected - 1].is_header() && delta < 0
                {
                    selected - 1
                } else {
                    selected
                };
                preview.scroll.scroll_to_item(target);
            }
            cx.notify();
        }
    }
}

/// Keys as the text the keymap's data writes, for [`KeymapView::describe`].
fn key_texts(keys: &[KeySequence]) -> String {
    keys.iter()
        .map(|key| key.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// "⌘B", "⌃↩, ⇧F10" or "no key": what "Use Default" would bring back.
pub(crate) fn default_label(keys: &[KeySequence]) -> String {
    if keys.is_empty() {
        "no key".to_string()
    } else {
        glyph_list(keys)
    }
}

/// The key contexts a hint of the view is looked up in.
pub(crate) fn stack_of(mode: &Mode) -> &'static [&'static str] {
    match mode {
        Mode::Asking(_) => &CHOICES_STACK,
        _ => &LIST_STACK,
    }
}
