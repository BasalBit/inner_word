//! Find and replace (`cmd-f`, `cmd-alt-f`): a slim bar above or below the
//! editor.
//!
//! The matches come from `innerword_core`'s search and are painted through the
//! editor's highlight layers (`"find"` for all of them, `"find-current"`
//! for the current one). The current match is the editor's selection: going
//! to a match selects it, so closing the bar leaves it selected and editing
//! shortcuts act on it.
//!
//! The match list follows the document without searching it again: an edit
//! reports the range of blocks it changed, only those are searched, and the
//! matches after them shift (matches never span blocks). The whole document
//! is searched when the query or an option changes.
//!
//! With the bar closed, `cmd-g` / `cmd-shift-g` still show where they
//! land: the matches are highlighted and a small label with the query and
//! the count floats over the editor for a moment (the "flash").

use std::time::Duration;

use gpui::{
    AnyElement, App, Context, Entity, SharedString, Subscription, Task, Transformation, Window,
    div, percentage, prelude::*, px, svg,
};
use innerword_core::{Change, Position, SearchOptions, TextRange};
use innerword_editor::{Autoscroll, HighlightStyle};
use innerword_ui::{
    FieldAppearance, Icon, IconName, KeyHint, TextField, TextFieldEvent,
    theme::{ActiveTheme, RADIUS_MD, RADIUS_SM, UI_TEXT_BASE, UI_TEXT_SM, UI_TEXT_XS},
};

use crate::{
    actions::{
        FIND_BAR_CONTEXT, FindAndReplace, FindNext, FindPrevious, ReplaceAll, ReplaceCurrent,
        ToggleCaseSensitive, ToggleWholeWord,
    },
    tip::{apply_hint, close_hint, tip},
    tools::{EditorTools, ToolKind, ToolsEvent},
};

/// The highlight layer with every match.
pub const FIND_LAYER: &str = "find";
/// The highlight layer with the current match.
pub const FIND_CURRENT_LAYER: &str = "find-current";

/// Longest selection taken over as the query when the bar opens.
const SEED_LIMIT: usize = 200;

/// How long the results stay on screen after `cmd-g` / `cmd-shift-g` with
/// the bar closed.
pub const FIND_FLASH_DURATION: Duration = Duration::from_millis(2500);

/// Below this width the bar drops the key hints beside its buttons (they
/// are in the tooltips), so the query field keeps a usable width.
const COMPACT_BELOW: f32 = 620.;

/// The debug selectors of the find bar's buttons, in the order they are
/// laid out. Each has a tooltip ([`EditorTools::find_button_tooltip`]).
pub const FIND_BUTTONS: [&str; 8] = [
    "find-disclosure",
    "find-case",
    "find-word",
    "find-previous",
    "find-next",
    "find-close",
    "find-replace-one",
    "find-replace-all",
];

/// What is searched for. Survives closing the bar, so "find next" keeps
/// working.
#[derive(Default)]
pub(crate) struct FindState {
    pub(crate) query: String,
    pub(crate) options: SearchOptions,
    /// The matches in document order (kept up to date only while results
    /// are showing: the bar is open, or the flash is on).
    pub(crate) matches: Vec<TextRange>,
    /// The document revision `matches` were computed for.
    revision: u64,
    /// The match the editor's selection is on.
    pub(crate) current: Option<usize>,
    /// A message shown in place of the count ("Replaced 12").
    notice: Option<SharedString>,
    /// The document revision the notice was made for.
    notice_revision: u64,
    /// Results shown without the bar, after `cmd-g` with the bar closed.
    /// The task ends the flash when its time is up.
    flash: Option<Task<()>>,
    /// How often the whole document was searched (the incremental update
    /// after an edit does not count). For tests.
    pub(crate) full_searches: usize,
}

/// The open find bar.
pub(crate) struct FindBar {
    query: Entity<TextField>,
    replace: Entity<TextField>,
    show_replace: bool,
    _subscriptions: [Subscription; 2],
}

/// The first match that starts at or after `from`, wrapping to the first.
fn match_at_or_after(matches: &[TextRange], from: Position) -> Option<usize> {
    if matches.is_empty() {
        return None;
    }
    let index = matches.partition_point(|found| found.start < from);
    Some(if index < matches.len() { index } else { 0 })
}

/// The match "find next" goes to from a selection.
fn next_match(matches: &[TextRange], selection: TextRange) -> Option<usize> {
    if matches.is_empty() {
        return None;
    }
    let mut index = matches.partition_point(|found| found.start < selection.start);
    if matches.get(index) == Some(&selection) {
        index += 1;
    }
    Some(if index < matches.len() { index } else { 0 })
}

/// The match "find previous" goes to from a selection.
fn previous_match(matches: &[TextRange], selection: TextRange) -> Option<usize> {
    if matches.is_empty() {
        return None;
    }
    let index = matches.partition_point(|found| found.start < selection.start);
    Some(if index == 0 {
        matches.len() - 1
    } else {
        index - 1
    })
}

/// The key that replaces the current match: the one of [`ReplaceCurrent`]
/// when the keymap has one, else the key the replace field submits on
/// (which is what replaces while the keyboard is in that field).
fn replace_hint(cx: &App) -> Option<KeyHint> {
    KeyHint::for_action(&ReplaceCurrent, cx).or_else(|| apply_hint(cx))
}

impl EditorTools {
    // -----------------------------------------------------------------
    // State, for hosts and tests
    // -----------------------------------------------------------------

    /// The search text (kept after the bar is closed).
    pub fn find_query(&self) -> &str {
        &self.find.query
    }

    /// Case sensitivity and whole-word matching.
    pub fn find_options(&self) -> SearchOptions {
        self.find.options
    }

    /// Number of matches (0 while no results are showing).
    pub fn find_match_count(&self) -> usize {
        self.find.matches.len()
    }

    /// Whether find results are on screen: the bar is open, or `cmd-g` /
    /// `cmd-shift-g` with the bar closed is showing where it landed
    /// ([`EditorTools::find_flash_visible`]).
    pub fn find_results_visible(&self) -> bool {
        self.find_bar.is_some() || self.find.flash.is_some()
    }

    /// Whether the label that `cmd-g` / `cmd-shift-g` show with the bar
    /// closed is on screen (for [`FIND_FLASH_DURATION`] after the last
    /// step).
    pub fn find_flash_visible(&self) -> bool {
        self.find_bar.is_none() && self.find.flash.is_some()
    }

    /// How often the whole document has been searched so far. Edits while
    /// results are showing update the match list from the changed blocks
    /// and do not count. For tests and performance checks.
    pub fn find_full_search_count(&self) -> usize {
        self.find.full_searches
    }

    /// What the tooltip of a find bar button says: its name and key
    /// ("Next match ⌘G"). `button` is one of [`FIND_BUTTONS`].
    pub fn find_button_tooltip(&self, button: &str, cx: &App) -> Option<String> {
        let (label, hint) = self.find_button_tip(button, cx)?;
        Some(match hint {
            Some(hint) => format!("{label} {}", hint.text()),
            None => label.to_string(),
        })
    }

    fn find_button_tip(&self, button: &str, cx: &App) -> Option<(&'static str, Option<KeyHint>)> {
        Some(match button {
            "find-disclosure" => (
                if self.find_replace_visible() {
                    "Hide replace"
                } else {
                    "Show replace"
                },
                KeyHint::for_action(&FindAndReplace, cx),
            ),
            "find-case" => ("Match case", KeyHint::for_action(&ToggleCaseSensitive, cx)),
            "find-word" => ("Whole word", KeyHint::for_action(&ToggleWholeWord, cx)),
            "find-previous" => ("Previous match", KeyHint::for_action(&FindPrevious, cx)),
            "find-next" => ("Next match", KeyHint::for_action(&FindNext, cx)),
            "find-close" => ("Close", close_hint(cx)),
            "find-replace-one" => ("Replace this match", replace_hint(cx)),
            "find-replace-all" => ("Replace all matches", KeyHint::for_action(&ReplaceAll, cx)),
            _ => return None,
        })
    }

    /// The 1-based number of the current match, when the editor's
    /// selection is on a match.
    pub fn find_current_match(&self) -> Option<usize> {
        self.find.current.map(|index| index + 1)
    }

    /// Whether the replace row of the find bar is showing.
    pub fn find_replace_visible(&self) -> bool {
        self.find_bar.as_ref().is_some_and(|bar| bar.show_replace)
    }

    /// The text of the replace field.
    pub fn find_replacement(&self, cx: &App) -> String {
        self.find_bar
            .as_ref()
            .map(|bar| bar.replace.read(cx).text().to_string())
            .unwrap_or_default()
    }

    /// What the bar (or, with the bar closed, the flash) shows next to the
    /// query: "3 of 17", "17 matches", "No matches", "Replaced 12", or
    /// nothing for an empty query and while no results are showing.
    pub fn find_status(&self) -> String {
        if !self.find_results_visible() {
            return String::new();
        }
        if let Some(notice) = &self.find.notice {
            return notice.to_string();
        }
        let count = self.find.matches.len();
        match (self.find.query.is_empty(), self.find.current, count) {
            (true, _, _) => String::new(),
            (false, _, 0) => "No matches".into(),
            (false, Some(index), _) => format!("{} of {count}", index + 1),
            (false, None, 1) => "1 match".into(),
            (false, None, _) => format!("{count} matches"),
        }
    }

    // -----------------------------------------------------------------
    // Opening and closing
    // -----------------------------------------------------------------

    /// Opens the find bar (with the replace row when `replace`), or brings
    /// focus back to its query field when it is open. A selection inside
    /// one line becomes the query.
    pub fn open_find(&mut self, replace: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.close_popup(false, window, cx);
        let opened = self.find_bar.is_none();
        // The bar takes over from the flash (the matches stay).
        self.find.flash = None;
        // With the bar open, the selection only becomes the query when the
        // shortcut comes from the document and the selection is not the
        // current match (which is a result of the search, not a new thing
        // to search for).
        let from_bar = self.find_bar.as_ref().is_some_and(|bar| {
            bar.query.read(cx).is_focused(window) || bar.replace.read(cx).is_focused(window)
        });
        let seed = if opened || (!from_bar && self.find.current.is_none()) {
            self.find_seed(cx)
        } else {
            None
        };
        if opened {
            let bar = self.build_find_bar(window, cx);
            self.find_bar = Some(bar);
            cx.emit(ToolsEvent::Opened(ToolKind::Find));
        }
        let Some(bar) = &mut self.find_bar else {
            return;
        };
        bar.show_replace |= replace;
        let query = bar.query.clone();
        if let Some(seed) = seed {
            self.find.query = seed;
        }
        let text = self.find.query.clone();
        query.update(cx, |field, cx| {
            if field.text() != text {
                field.set_text(text, cx);
            }
            field.select_all(cx);
            field.focus(window);
        });
        self.find.notice = None;
        self.refresh_find(false, cx);
        cx.notify();
    }

    /// The editor's selection as a query, when it is short and inside one
    /// line.
    fn find_seed(&self, cx: &App) -> Option<String> {
        let editor = self.editor.read(cx);
        let range = editor.selection().range();
        if range.is_empty()
            || range.start.block != range.end.block
            || range.end.offset - range.start.offset > SEED_LIMIT
        {
            return None;
        }
        let text = editor.state().selected_text();
        (!text.contains('\n') && !text.trim().is_empty()).then_some(text)
    }

    fn build_find_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> FindBar {
        let field = |placeholder: &'static str, cx: &mut Context<Self>, window: &mut Window| {
            cx.new(|cx| {
                TextField::new(window, cx)
                    .with_placeholder(placeholder)
                    .with_appearance(FieldAppearance::Borderless)
                    .with_text_size(UI_TEXT_BASE)
                    .consume_escape()
            })
        };
        let query = field("Find", cx, window);
        let replace = field("Replace with", cx, window);
        let subscriptions = [
            cx.subscribe_in(
                &query,
                window,
                |this, field, event: &TextFieldEvent, window, cx| match event {
                    TextFieldEvent::Changed => {
                        this.find.query = field.read(cx).text().to_string();
                        this.find.notice = None;
                        this.refresh_find(true, cx);
                    }
                    TextFieldEvent::Submitted => this.find_next(window, cx),
                    TextFieldEvent::Cancelled => this.close_find(window, cx),
                    // The frame of the field that has the keyboard is
                    // drawn in the accent.
                    TextFieldEvent::Focused | TextFieldEvent::Blurred => cx.notify(),
                },
            ),
            cx.subscribe_in(
                &replace,
                window,
                |this, _, event: &TextFieldEvent, window, cx| match event {
                    TextFieldEvent::Submitted => this.replace_current(window, cx),
                    TextFieldEvent::Cancelled => this.close_find(window, cx),
                    TextFieldEvent::Focused | TextFieldEvent::Blurred => cx.notify(),
                    TextFieldEvent::Changed => {}
                },
            ),
        ];
        FindBar {
            query,
            replace,
            show_replace: false,
            _subscriptions: subscriptions,
        }
    }

    /// Closes the find bar and returns focus to the editor. The current
    /// match stays selected; the query is kept for "find next".
    pub fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_find_bar(cx);
        self.editor.read(cx).focus(window);
    }

    /// `cmd-alt-f`: shows the replace row (opening the bar when it is
    /// closed), or hides it again when the keyboard is already in the bar
    /// and the row is showing. From the document it always brings the
    /// keyboard to the bar and never hides anything.
    ///
    /// When the key makes the row appear on a bar that already has a
    /// query, the keyboard goes to the replace field: the user has said
    /// what to find and is asking to replace it, and what they type next
    /// must not overwrite the query. With an empty query (and when the bar
    /// was closed) the keyboard is in the query field, which comes first.
    pub fn toggle_find_replace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let in_bar_with_row = self.find_bar.as_ref().is_some_and(|bar| {
            bar.show_replace
                && (bar.query.read(cx).is_focused(window)
                    || bar.replace.read(cx).is_focused(window))
        });
        if in_bar_with_row {
            self.set_replace_row(false, window, cx);
            return;
        }
        let row_appears_on_open_bar = self.find_bar.as_ref().is_some_and(|bar| !bar.show_replace);
        self.open_find(true, window, cx);
        if row_appears_on_open_bar && !self.find.query.is_empty() {
            self.focus_replace_field(window, cx);
        }
    }

    /// Puts the keyboard in the replace field, with its text selected.
    fn focus_replace_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(bar) = &self.find_bar else {
            return;
        };
        bar.replace.update(cx, |field, cx| {
            field.select_all(cx);
            field.focus(window);
        });
    }

    /// Closes the bar without touching focus.
    pub(crate) fn close_find_bar(&mut self, cx: &mut Context<Self>) {
        if self.find_bar.take().is_none() {
            return;
        }
        self.clear_find_results(cx);
        cx.emit(ToolsEvent::Closed(ToolKind::Find));
        cx.notify();
    }

    /// Forgets the matches and removes their highlights.
    fn clear_find_results(&mut self, cx: &mut Context<Self>) {
        self.find.matches.clear();
        self.find.current = None;
        self.find.notice = None;
        self.find.flash = None;
        self.editor.update(cx, |editor, cx| {
            editor.clear_highlights(FIND_LAYER, cx);
            editor.clear_highlights(FIND_CURRENT_LAYER, cx);
        });
    }

    // -----------------------------------------------------------------
    // The flash: results without the bar
    // -----------------------------------------------------------------

    /// Shows the results for [`FIND_FLASH_DURATION`] from now.
    fn start_find_flash(&mut self, cx: &mut Context<Self>) {
        self.find.flash = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(FIND_FLASH_DURATION).await;
            this.update(cx, |this, cx| this.end_find_flash(cx)).ok();
        }));
        cx.notify();
    }

    /// Ends the flash, if it is on. Returns whether it was.
    pub(crate) fn end_find_flash(&mut self, cx: &mut Context<Self>) -> bool {
        if self.find.flash.take().is_none() {
            return false;
        }
        if self.find_bar.is_none() {
            self.clear_find_results(cx);
        }
        cx.notify();
        true
    }

    // -----------------------------------------------------------------
    // Matches
    // -----------------------------------------------------------------

    /// Searches the whole document.
    fn search(&mut self, cx: &App) {
        let editor = self.editor.read(cx);
        self.find.revision = editor.state().revision();
        self.find.matches = if self.find.query.is_empty() {
            Vec::new()
        } else {
            self.find.full_searches += 1;
            editor
                .document()
                .find_all(&self.find.query, self.find.options)
        };
    }

    /// The document changed while results are showing: brings the match
    /// list up to date. Only the changed blocks are searched; the matches
    /// after them move with their blocks.
    pub(crate) fn find_document_edited(&mut self, change: &Change, cx: &mut Context<Self>) {
        let Some(splice) = change.blocks else {
            return;
        };
        let now = self.editor.read(cx).state().revision();
        if change.revision <= self.find.revision {
            // A search made after this change already covers it.
            return;
        }
        if now != change.revision || self.find.query.is_empty() {
            // More changes are on their way (their events follow): the
            // block range of this one no longer describes the document.
            self.search(cx);
        } else {
            let editor = self.editor.read(cx);
            let document = editor.document();
            let matches = &mut self.find.matches;
            let low = matches.partition_point(|found| found.start.block < splice.start);
            let high =
                matches.partition_point(|found| found.start.block < splice.start + splice.old_len);
            let fresh: Vec<TextRange> = splice
                .new_range()
                .flat_map(|block| {
                    document
                        .find_in_block(block, &self.find.query, self.find.options)
                        .into_iter()
                        .map(move |found| TextRange::in_block(block, found.start, found.end))
                })
                .collect();
            let tail = low + fresh.len();
            let moved = splice.new_len != splice.old_len && high < matches.len();
            if low == high && fresh.is_empty() && !moved {
                // Nothing to find in what changed and nothing after it
                // moved (typing in a paragraph without a match, the usual
                // case): the highlights stand as they are.
                self.find.revision = now;
                self.expire_find_notice(cx);
                self.sync_find_current(cx);
                cx.notify();
                return;
            }
            matches.splice(low..high, fresh);
            if moved {
                for found in &mut matches[tail..] {
                    let block = found.start.block + splice.new_len - splice.old_len;
                    *found = TextRange::in_block(block, found.start.offset, found.end.offset);
                }
            }
            self.find.revision = now;
        }
        self.show_find_results(false, cx);
    }

    /// "Replaced 12" stands until the document changes again.
    fn expire_find_notice(&mut self, cx: &App) {
        if self.find.notice.is_some()
            && self.find.notice_revision != self.editor.read(cx).state().revision()
        {
            self.find.notice = None;
        }
    }

    /// Searches again. With `reveal` (the query or an option changed) the
    /// first match at or after the selection becomes the current one and is
    /// selected; otherwise (the document changed) the selection is left
    /// alone and the current match is whichever one it is on.
    pub(crate) fn refresh_find(&mut self, reveal: bool, cx: &mut Context<Self>) {
        self.search(cx);
        self.show_find_results(reveal, cx);
    }

    /// The match list is up to date: works out the current match, paints
    /// the highlights. With `reveal`, goes to the first match at or after
    /// the selection first.
    fn show_find_results(&mut self, reveal: bool, cx: &mut Context<Self>) {
        self.expire_find_notice(cx);
        if reveal {
            let from = self.editor.read(cx).selection().start();
            if let Some(index) = match_at_or_after(&self.find.matches, from) {
                self.select_match(self.find.matches[index], cx);
            }
        }
        self.find.current = self.match_under_selection(cx);
        self.apply_find_highlights(cx);
        cx.notify();
    }

    /// The match the editor's selection is exactly on.
    fn match_under_selection(&self, cx: &App) -> Option<usize> {
        let selection = self.editor.read(cx).selection().range();
        self.find
            .matches
            .binary_search_by(|found| found.start.cmp(&selection.start))
            .ok()
            .filter(|index| self.find.matches[*index] == selection)
    }

    /// The selection moved: recomputes which match it is on.
    pub(crate) fn sync_find_current(&mut self, cx: &mut Context<Self>) {
        let current = self.match_under_selection(cx);
        if self.find.current != current {
            self.find.current = current;
            self.paint_current_match(cx);
            cx.notify();
        }
    }

    /// Paints the matches through the editor's highlight layers.
    pub(crate) fn apply_find_highlights(&mut self, cx: &mut Context<Self>) {
        if !self.find_results_visible() {
            return;
        }
        let style = HighlightStyle::background(cx.theme().colors.find_match);
        let matches = self.find.matches.clone();
        self.editor.update(cx, |editor, cx| {
            editor.set_highlights(FIND_LAYER, matches, style, cx);
        });
        self.paint_current_match(cx);
    }

    fn paint_current_match(&mut self, cx: &mut Context<Self>) {
        if !self.find_results_visible() {
            return;
        }
        let style = HighlightStyle::background(cx.theme().colors.find_match_current);
        let current: Vec<TextRange> = self
            .find
            .current
            .and_then(|index| self.find.matches.get(index).copied())
            .into_iter()
            .collect();
        self.editor.update(cx, |editor, cx| {
            editor.set_highlights(FIND_CURRENT_LAYER, current, style, cx);
        });
    }

    /// Selects a match in the editor and brings it into view.
    fn select_match(&mut self, found: TextRange, cx: &mut Context<Self>) {
        self.editor.update(cx, |editor, cx| {
            editor.edit_with(Autoscroll::None, |state| state.select_range(found), cx);
            let visible = editor
                .visible_range()
                .is_some_and(|visible| visible.start <= found.start && found.end <= visible.end);
            if !visible {
                editor.scroll_to_position(found.start, Autoscroll::Center, cx);
            }
        });
    }

    fn go_to_match(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(found) = self.find.matches.get(index).copied() else {
            return;
        };
        self.find.notice = None;
        self.select_match(found, cx);
        self.sync_find_current(cx);
        cx.notify();
    }

    /// Goes to the next match (wrapping). With the bar closed it searches
    /// for the last query without opening the bar, and shows the matches
    /// and the count for a moment ([`FIND_FLASH_DURATION`]); with no query
    /// at all it opens the bar.
    pub fn find_next(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.find_step(true, window, cx);
    }

    /// Goes to the previous match (wrapping).
    pub fn find_previous(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.find_step(false, window, cx);
    }

    fn find_step(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.find.query.is_empty() {
            if self.find_bar.is_none() {
                self.open_find(false, window, cx);
            }
            return;
        }
        if self.find_bar.is_none() {
            // Bar closed: the results show for a moment. While they do,
            // the match list is kept current, so stepping on does not
            // search again.
            let fresh = self.find.flash.is_none();
            if fresh {
                self.search(cx);
            }
            self.start_find_flash(cx);
            if fresh {
                self.apply_find_highlights(cx);
            }
        }
        let selection = self.editor.read(cx).selection().range();
        let target = if forward {
            next_match(&self.find.matches, selection)
        } else {
            previous_match(&self.find.matches, selection)
        };
        if let Some(index) = target {
            self.go_to_match(index, cx);
        }
    }

    /// Switches case-sensitive search on or off.
    pub fn toggle_case_sensitive(&mut self, cx: &mut Context<Self>) {
        self.find.options.case_sensitive = !self.find.options.case_sensitive;
        self.options_changed(cx);
    }

    /// Switches whole-word search on or off.
    pub fn toggle_whole_word(&mut self, cx: &mut Context<Self>) {
        self.find.options.whole_word = !self.find.options.whole_word;
        self.options_changed(cx);
    }

    fn options_changed(&mut self, cx: &mut Context<Self>) {
        self.find.notice = None;
        if self.find_bar.is_some() {
            self.refresh_find(true, cx);
        } else {
            // The flash showed the results of the old options.
            self.end_find_flash(cx);
        }
        cx.notify();
    }

    // -----------------------------------------------------------------
    // Replace
    // -----------------------------------------------------------------

    /// Replaces the current match with the replace field's text and goes
    /// to the next match. When the selection is not on a match, it only
    /// goes to the next match.
    pub fn replace_current(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.find_bar.is_none() || self.read_only(cx) {
            return;
        }
        let Some(found) = self
            .find
            .current
            .and_then(|index| self.find.matches.get(index).copied())
        else {
            return self.find_next(window, cx);
        };
        let replacement = self.find_replacement(cx);
        self.editor.update(cx, |editor, cx| {
            editor.edit_with(
                Autoscroll::None,
                |state| state.replace_range(found, &replacement),
                cx,
            );
        });
        // Continue after the replacement, so a replacement that contains
        // the query is not found again.
        let after = self.editor.read(cx).selection().head;
        self.search(cx);
        self.find.notice = None;
        if let Some(index) = match_at_or_after(&self.find.matches, after) {
            self.select_match(self.find.matches[index], cx);
        }
        self.find.current = self.match_under_selection(cx);
        self.apply_find_highlights(cx);
        cx.notify();
    }

    /// Replaces every match as one undo step.
    pub fn replace_all(&mut self, cx: &mut Context<Self>) {
        if self.find_bar.is_none() || self.read_only(cx) || self.find.query.is_empty() {
            return;
        }
        let replacement = self.find_replacement(cx);
        let query = self.find.query.clone();
        let options = self.find.options;
        let count = self.editor.update(cx, |editor, cx| {
            editor.edit_with(
                Autoscroll::None,
                |state| state.replace_all(&query, options, &replacement),
                cx,
            )
        });
        self.refresh_find(false, cx);
        self.find.notice = Some(match count {
            0 => "No matches".into(),
            _ => format!("Replaced {count}").into(),
        });
        self.find.notice_revision = self.editor.read(cx).state().revision();
        cx.notify();
    }

    /// `tab` / `shift-tab` in the find bar: moves between the query and the
    /// replace field.
    pub(crate) fn focus_other_find_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(bar) = &self.find_bar else {
            return;
        };
        if !bar.show_replace {
            return bar.query.read(cx).focus(window);
        }
        let target = if bar.query.read(cx).is_focused(window) {
            &bar.replace
        } else {
            &bar.query
        };
        target.update(cx, |field, cx| {
            field.select_all(cx);
            field.focus(window);
        });
    }

    fn toggle_replace_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let show = !self.find_replace_visible();
        self.set_replace_row(show, window, cx);
    }

    /// Shows or hides the replace row. The keyboard moves to the query
    /// field when its field goes away.
    fn set_replace_row(&mut self, show: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(bar) = &mut self.find_bar else {
            return;
        };
        bar.show_replace = show;
        if !show && bar.replace.read(cx).is_focused(window) {
            bar.query.read(cx).focus(window);
        }
        cx.notify();
    }

    // -----------------------------------------------------------------
    // Rendering
    // -----------------------------------------------------------------

    /// A button of the bar with its tooltip. Never takes focus.
    fn bar_button(&self, id: &'static str, cx: &App) -> gpui::Stateful<gpui::Div> {
        let colors = cx.theme().colors;
        let button = div()
            .id(id)
            .debug_selector(|| id.to_string())
            .flex_none()
            .h(px(24.))
            .min_w(px(24.))
            .px(px(4.))
            .flex()
            .items_center()
            .justify_center()
            .gap(px(4.))
            .rounded(RADIUS_SM)
            .text_size(UI_TEXT_SM)
            .text_color(colors.muted)
            .cursor_pointer()
            .hover(|style| style.bg(colors.hover));
        match self.find_button_tip(id, cx) {
            Some((label, hint)) => button.tooltip(tip(label, hint)),
            None => button,
        }
    }

    pub(crate) fn render_find_bar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let bar = self.find_bar.as_ref()?;
        let colors = cx.theme().colors;
        let font = cx.theme().fonts.ui.clone();
        let options = self.find.options;
        let status = self.find_status();
        let no_matches = !self.find.query.is_empty() && self.find.matches.is_empty();
        let show_replace = bar.show_replace;
        let read_only = self.read_only(cx);
        // In a narrow editor the key hints beside the buttons go (they are
        // in the tooltips) and the field keeps its width. The width is the
        // editor's, from its last layout.
        let compact = self
            .editor
            .read(cx)
            .viewport_bounds()
            .is_some_and(|bounds| bounds.size.width < px(COMPACT_BELOW));
        let hint =
            |hint: Option<KeyHint>| hint.filter(|_| !compact).map(|hint| hint.size(UI_TEXT_XS));

        // A field in its frame. With two fields stacked, the frame of the
        // one that has the keyboard is drawn in the accent (as the comment
        // composer and the rename field do): the thin caret alone does not
        // say where typing goes.
        let framed =
            |name: &'static str, field: Entity<TextField>, trailing: Option<AnyElement>| {
                let focused = field.read(cx).is_focused(window);
                div()
                    .debug_selector(|| name.to_string())
                    .flex_1()
                    .min_w(px(96.))
                    .h(px(26.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .rounded(RADIUS_MD)
                    .border_1()
                    .border_color(if focused {
                        colors.accent
                    } else {
                        colors.hairline
                    })
                    .bg(colors.canvas)
                    .when(focused, |el| {
                        // Nothing to see: where the accent frame is, for
                        // tests.
                        el.relative().child(
                            div()
                                .absolute()
                                .inset_0()
                                .debug_selector(|| "find-focus-ring".into()),
                        )
                    })
                    .child(div().flex_1().min_w_0().child(field))
                    .children(trailing)
            };
        // The bar is laid out in three columns (the disclosure, the fields,
        // the buttons) of one or two cells each, so that the two fields
        // have the same edges whatever the buttons beside them measure.
        let cell = || div().h(px(36.)).flex().items_center().gap(px(6.));

        let disclosure = self
            .bar_button("find-disclosure", cx)
            .child(Icon::new(if show_replace {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            }))
            .on_click(cx.listener(|this, _, window, cx| this.toggle_replace_row(window, cx)));

        let count = div()
            .flex_none()
            .text_size(UI_TEXT_SM)
            .text_color(if no_matches {
                colors.danger
            } else {
                colors.muted
            })
            .child(status);

        let toggle = |button: gpui::Stateful<gpui::Div>, label: &'static str, on: bool| {
            button
                .px(px(6.))
                .border_1()
                .border_color(if on {
                    colors.accent
                } else {
                    gpui::transparent_black()
                })
                .when(on, |el| el.bg(colors.accent_subtle))
                .child(
                    div()
                        .text_color(if on { colors.ink } else { colors.muted })
                        .child(label),
                )
        };
        let case = toggle(
            self.bar_button("find-case", cx),
            "Aa",
            options.case_sensitive,
        )
        .children(hint(KeyHint::for_action(&ToggleCaseSensitive, cx)))
        .on_click(cx.listener(|this, _, _, cx| this.toggle_case_sensitive(cx)));
        let word = toggle(self.bar_button("find-word", cx), "Word", options.whole_word)
            .children(hint(KeyHint::for_action(&ToggleWholeWord, cx)))
            .on_click(cx.listener(|this, _, _, cx| this.toggle_whole_word(cx)));

        let previous = self
            .bar_button("find-previous", cx)
            .child(
                svg()
                    .path(IconName::ChevronDown.path())
                    .flex_none()
                    .size(px(16.))
                    .text_color(colors.muted)
                    .with_transformation(Transformation::rotate(percentage(0.5))),
            )
            .children(hint(KeyHint::for_action(&FindPrevious, cx)))
            .on_click(cx.listener(|this, _, window, cx| this.find_previous(window, cx)));
        let next = self
            .bar_button("find-next", cx)
            .child(Icon::new(IconName::ChevronDown))
            .children(hint(KeyHint::for_action(&FindNext, cx)))
            .on_click(cx.listener(|this, _, window, cx| this.find_next(window, cx)));
        let close = self
            .bar_button("find-close", cx)
            .child(Icon::new(IconName::Close))
            .on_click(cx.listener(|this, _, window, cx| this.close_find(window, cx)));

        let replace_buttons = show_replace.then(|| {
            let text_button = |button: gpui::Stateful<gpui::Div>, label: &'static str| {
                button
                    .px(px(8.))
                    .border_1()
                    .border_color(colors.hairline)
                    .when(read_only, |el| el.opacity(0.5))
                    .child(div().text_color(colors.ink).child(label))
            };
            let replace = text_button(self.bar_button("find-replace-one", cx), "Replace")
                .children(hint(replace_hint(cx)))
                .on_click(cx.listener(|this, _, window, cx| this.replace_current(window, cx)));
            let replace_all = text_button(
                self.bar_button("find-replace-all", cx),
                if compact { "All" } else { "Replace all" },
            )
            .children(hint(KeyHint::for_action(&ReplaceAll, cx)))
            .on_click(cx.listener(|this, _, _, cx| this.replace_all(cx)));
            cell().child(replace).child(replace_all)
        });

        let lead = div().flex_none().child(cell().child(disclosure));
        let fields = div()
            .flex_1()
            .min_w(px(96.))
            .flex()
            .flex_col()
            .child(cell().child(framed(
                "find-query-frame",
                bar.query.clone(),
                Some(count.into_any_element()),
            )))
            .when(show_replace, |el| {
                el.child(cell().child(framed("find-replace-frame", bar.replace.clone(), None)))
            });
        let buttons = div()
            .flex_none()
            .flex()
            .flex_col()
            .child(
                cell()
                    .child(case)
                    .child(word)
                    .child(previous)
                    .child(next)
                    // The close button keeps to the right edge when the
                    // replace buttons below are the wider row.
                    .child(div().flex_1())
                    .child(close),
            )
            .children(replace_buttons);

        Some(
            div()
                .key_context(FIND_BAR_CONTEXT)
                .debug_selector(|| "find-bar".into())
                .flex_none()
                .w_full()
                .flex()
                .justify_center()
                .bg(colors.elevated)
                .border_color(colors.hairline)
                .map(|el| match self.find_bar_placement() {
                    crate::FindBarPlacement::Top => el.border_b_1(),
                    crate::FindBarPlacement::Bottom => el.border_t_1(),
                })
                .font_family(font)
                .text_size(UI_TEXT_BASE)
                .line_height(px(18.))
                .text_color(colors.ink)
                // Clicks on the bar's chrome keep focus where it is.
                .on_any_mouse_down(|_, window, _| window.prevent_default())
                .child(
                    div()
                        .w_full()
                        .max_w(px(760.))
                        .px(px(12.))
                        .flex()
                        .items_start()
                        .gap(px(6.))
                        .child(lead)
                        .child(fields)
                        .child(buttons),
                )
                .into_any_element(),
        )
    }

    /// The label that floats over the editor while `cmd-g` /
    /// `cmd-shift-g` show their results with the bar closed: the query and
    /// the count. It is not interactive.
    pub(crate) fn render_find_flash(&self, cx: &App) -> Option<AnyElement> {
        if !self.find_flash_visible() {
            return None;
        }
        let theme = cx.theme();
        let colors = theme.colors;
        let no_matches = self.find.matches.is_empty();
        Some(
            div()
                .debug_selector(|| "find-flash".into())
                .absolute()
                .top(px(12.))
                .right(px(16.))
                .max_w(px(320.))
                .h(px(28.))
                .px(px(10.))
                .flex()
                .items_center()
                .gap(px(8.))
                .rounded(RADIUS_MD)
                .bg(colors.elevated)
                .border_1()
                .border_color(colors.hairline)
                .shadow(theme.shadow_popover())
                .font_family(theme.fonts.ui.clone())
                .text_size(UI_TEXT_SM)
                .line_height(px(16.))
                .child(
                    Icon::new(IconName::Search)
                        .size(px(14.))
                        .color(colors.muted),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(colors.ink)
                        .child(SharedString::from(self.find.query.clone())),
                )
                .child(
                    div()
                        .flex_none()
                        .text_color(if no_matches {
                            colors.danger
                        } else {
                            colors.muted
                        })
                        .child(self.find_status()),
                )
                .into_any_element(),
        )
    }
}
