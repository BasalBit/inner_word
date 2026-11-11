//! Find and replace (`cmd-f`, `cmd-alt-f`): a slim bar above or below the
//! editor.
//!
//! The matches come from `writer_core`'s search and are painted through the
//! editor's highlight layers (`"find"` for all of them, `"find-current"`
//! for the current one). The current match is the editor's selection: going
//! to a match selects it, so closing the bar leaves it selected and editing
//! shortcuts act on it.

use gpui::{
    AnyElement, App, Context, Entity, MouseButton, SharedString, Subscription, Transformation,
    Window, div, percentage, prelude::*, px, svg,
};
use writer_core::{Position, SearchOptions, TextRange};
use writer_editor::{Autoscroll, HighlightStyle};
use writer_ui::{
    FieldAppearance, Icon, IconName, KeyHint, TextField, TextFieldEvent,
    theme::{ActiveTheme, RADIUS_MD, RADIUS_SM, UI_TEXT_BASE, UI_TEXT_SM, UI_TEXT_XS},
};

use crate::{
    actions::{
        FIND_BAR_CONTEXT, FindNext, FindPrevious, ReplaceAll, ToggleCaseSensitive, ToggleWholeWord,
    },
    tools::{EditorTools, ToolKind, ToolsEvent},
};

/// The highlight layer with every match.
pub const FIND_LAYER: &str = "find";
/// The highlight layer with the current match.
pub const FIND_CURRENT_LAYER: &str = "find-current";

/// Longest selection taken over as the query when the bar opens.
const SEED_LIMIT: usize = 200;

/// What is searched for. Survives closing the bar, so "find next" keeps
/// working.
#[derive(Default)]
pub(crate) struct FindState {
    pub(crate) query: String,
    pub(crate) options: SearchOptions,
    /// The matches in document order (kept up to date only while the bar
    /// is open).
    pub(crate) matches: Vec<TextRange>,
    /// The match the editor's selection is on.
    pub(crate) current: Option<usize>,
    /// A message shown in place of the count ("Replaced 12").
    notice: Option<SharedString>,
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

    /// Number of matches (0 while the bar is closed).
    pub fn find_match_count(&self) -> usize {
        self.find.matches.len()
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

    /// What the bar shows next to the query: "3 of 17", "17 matches",
    /// "No matches", "Replaced 12", or nothing for an empty query (and
    /// while the bar is closed).
    pub fn find_status(&self) -> String {
        if self.find_bar.is_none() {
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
        // With the bar open, a selection that is the current match is the
        // result of the search, not a new thing to search for.
        let seed = if opened || self.find.current.is_none() {
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
                    _ => {}
                },
            ),
            cx.subscribe_in(
                &replace,
                window,
                |this, _, event: &TextFieldEvent, window, cx| match event {
                    TextFieldEvent::Submitted => this.replace_current(window, cx),
                    TextFieldEvent::Cancelled => this.close_find(window, cx),
                    _ => {}
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

    /// Closes the bar without touching focus.
    pub(crate) fn close_find_bar(&mut self, cx: &mut Context<Self>) {
        if self.find_bar.take().is_none() {
            return;
        }
        self.find.matches.clear();
        self.find.current = None;
        self.find.notice = None;
        self.editor.update(cx, |editor, cx| {
            editor.clear_highlights(FIND_LAYER, cx);
            editor.clear_highlights(FIND_CURRENT_LAYER, cx);
        });
        cx.emit(ToolsEvent::Closed(ToolKind::Find));
        cx.notify();
    }

    // -----------------------------------------------------------------
    // Matches
    // -----------------------------------------------------------------

    fn search(&self, cx: &App) -> Vec<TextRange> {
        if self.find.query.is_empty() {
            return Vec::new();
        }
        self.editor
            .read(cx)
            .document()
            .find_all(&self.find.query, self.find.options)
    }

    /// Searches again. With `reveal` (the query or an option changed) the
    /// first match at or after the selection becomes the current one and is
    /// selected; otherwise (the document changed) the selection is left
    /// alone and the current match is whichever one it is on.
    pub(crate) fn refresh_find(&mut self, reveal: bool, cx: &mut Context<Self>) {
        self.find.matches = self.search(cx);
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
        if self.find_bar.is_none() {
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
        if self.find_bar.is_none() {
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
    /// for the last query without opening the bar; with no query at all it
    /// opens the bar.
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
        let selection = self.editor.read(cx).selection().range();
        if self.find_bar.is_some() {
            let target = if forward {
                next_match(&self.find.matches, selection)
            } else {
                previous_match(&self.find.matches, selection)
            };
            if let Some(index) = target {
                self.go_to_match(index, cx);
            }
            return;
        }
        // Bar closed: one search, no highlights.
        let matches = self.search(cx);
        let target = if forward {
            next_match(&matches, selection)
        } else {
            previous_match(&matches, selection)
        };
        if let Some(index) = target {
            self.select_match(matches[index], cx);
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
        self.find.matches = self.search(cx);
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
        let Some(bar) = &mut self.find_bar else {
            return;
        };
        bar.show_replace = !bar.show_replace;
        if !bar.show_replace && bar.replace.read(cx).is_focused(window) {
            bar.query.read(cx).focus(window);
        }
        cx.notify();
    }

    // -----------------------------------------------------------------
    // Rendering
    // -----------------------------------------------------------------

    pub(crate) fn render_find_bar(
        &mut self,
        _: &mut Window,
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

        // A field in its frame.
        let framed = |field: Entity<TextField>, trailing: Option<AnyElement>| {
            div()
                .flex_1()
                .min_w_0()
                .h(px(26.))
                .px(px(8.))
                .flex()
                .items_center()
                .gap(px(8.))
                .rounded(RADIUS_MD)
                .border_1()
                .border_color(colors.hairline)
                .bg(colors.canvas)
                .child(div().flex_1().min_w_0().child(field))
                .children(trailing)
        };

        let disclosure = bar_button("find-disclosure", cx)
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

        let case = toggle_button("find-case", "Aa", options.case_sensitive, cx)
            .children(
                KeyHint::for_action(&ToggleCaseSensitive, cx).map(|hint| hint.size(UI_TEXT_XS)),
            )
            .on_click(cx.listener(|this, _, _, cx| this.toggle_case_sensitive(cx)));
        let word = toggle_button("find-word", "Word", options.whole_word, cx)
            .children(KeyHint::for_action(&ToggleWholeWord, cx).map(|hint| hint.size(UI_TEXT_XS)))
            .on_click(cx.listener(|this, _, _, cx| this.toggle_whole_word(cx)));

        let previous = bar_button("find-previous", cx)
            .child(
                svg()
                    .path(IconName::ChevronDown.path())
                    .flex_none()
                    .size(px(16.))
                    .text_color(colors.muted)
                    .with_transformation(Transformation::rotate(percentage(0.5))),
            )
            .children(KeyHint::for_action(&FindPrevious, cx).map(|hint| hint.size(UI_TEXT_XS)))
            .on_click(cx.listener(|this, _, window, cx| this.find_previous(window, cx)));
        let next = bar_button("find-next", cx)
            .child(Icon::new(IconName::ChevronDown))
            .children(KeyHint::for_action(&FindNext, cx).map(|hint| hint.size(UI_TEXT_XS)))
            .on_click(cx.listener(|this, _, window, cx| this.find_next(window, cx)));
        let close = bar_button("find-close", cx)
            .child(Icon::new(IconName::Close))
            .on_click(cx.listener(|this, _, window, cx| this.close_find(window, cx)));

        let find_row = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .h(px(36.))
            .child(disclosure)
            .child(framed(bar.query.clone(), Some(count.into_any_element())))
            .child(case)
            .child(word)
            .child(previous)
            .child(next)
            .child(close);

        let replace_row = show_replace.then(|| {
            let replace = text_button("find-replace-one", "Replace", read_only, cx)
                .children(KeyHint::parse("enter").map(|hint| hint.size(UI_TEXT_XS)))
                .on_click(cx.listener(|this, _, window, cx| this.replace_current(window, cx)));
            let replace_all = text_button("find-replace-all", "Replace all", read_only, cx)
                .children(KeyHint::for_action(&ReplaceAll, cx).map(|hint| hint.size(UI_TEXT_XS)))
                .on_click(cx.listener(|this, _, _, cx| this.replace_all(cx)));
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .h(px(36.))
                // Line the field up with the query field.
                .child(div().flex_none().w(px(24.)))
                .child(framed(bar.replace.clone(), None))
                .child(replace)
                .child(replace_all)
        });

        Some(
            div()
                .key_context(FIND_BAR_CONTEXT)
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
                .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                .child(
                    div()
                        .w_full()
                        .max_w(px(760.))
                        .px(px(12.))
                        .flex()
                        .flex_col()
                        .child(find_row)
                        .children(replace_row),
                )
                .into_any_element(),
        )
    }
}

/// A small square-ish clickable in the bar. Never takes focus.
fn bar_button(id: &'static str, cx: &App) -> gpui::Stateful<gpui::Div> {
    let colors = cx.theme().colors;
    div()
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
        .hover(|style| style.bg(colors.hover))
}

/// An option of the search that is on or off.
fn toggle_button(
    id: &'static str,
    label: &'static str,
    on: bool,
    cx: &App,
) -> gpui::Stateful<gpui::Div> {
    let colors = cx.theme().colors;
    bar_button(id, cx)
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
}

/// A labelled action in the bar.
fn text_button(
    id: &'static str,
    label: &'static str,
    disabled: bool,
    cx: &App,
) -> gpui::Stateful<gpui::Div> {
    let colors = cx.theme().colors;
    bar_button(id, cx)
        .px(px(8.))
        .border_1()
        .border_color(colors.hairline)
        .when(disabled, |el| el.opacity(0.5))
        .child(div().text_color(colors.ink).child(label))
}
