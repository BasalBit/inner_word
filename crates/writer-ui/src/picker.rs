//! [`Picker`]: a query field over a filtered, virtualized list, driven by a
//! [`PickerDelegate`].
//!
//! One component serves the command palette, quick open, the font picker,
//! the `/` insert menu and the `cmd-.` format menu. The delegate owns the
//! items and the filtering; the picker owns the query field, the selection,
//! scrolling, keys and the look.
//!
//! # A delegate
//!
//! ```ignore
//! use writer_ui::{fuzzy, picker::*, KeyHint, IconName};
//!
//! struct Commands { all: Vec<Command>, matches: Vec<fuzzy::StringMatch> }
//!
//! impl PickerDelegate for Commands {
//!     fn update_matches(&mut self, query: &str, _: &mut Window, _: &mut Context<Picker<Self>>) {
//!         self.matches = fuzzy::match_strings(query, self.all.iter().map(|c| c.name.as_str()), usize::MAX);
//!     }
//!     fn entry_count(&self) -> usize { self.matches.len() }
//!     fn entry(&self, ix: usize, cx: &App) -> PickerEntry {
//!         let m = &self.matches[ix];
//!         let command = &self.all[m.candidate_index];
//!         PickerItem::new(command.name.clone())
//!             .positions(m.positions.clone())
//!             .key_hint(KeyHint::for_action(command.action.as_ref(), cx))
//!             .into()
//!     }
//!     fn confirm(&mut self, ix: usize, _secondary: bool, window: &mut Window, cx: &mut Context<Picker<Self>>) -> bool {
//!         let action = self.all[self.matches[ix].candidate_index].action.boxed_clone();
//!         window.dispatch_action(action, cx);
//!         true // dismiss
//!     }
//!     fn placeholder(&self) -> SharedString { "Run a command".into() }
//! }
//! ```
//!
//! Section headers are entries too ([`PickerEntry::Header`]); navigation
//! skips them. Lists of thousands of entries are fine: only visible rows are
//! built (`uniform_list`), so `entry` is called for about a screenful of
//! rows per frame. Keep `update_matches` synchronous; [`crate::fuzzy::match_strings`]
//! filters several thousand candidates in a few milliseconds.
//!
//! # Presentations
//!
//! ```ignore
//! // centred in the modal layer (command palette, quick open)
//! let picker = cx.new(|cx| Picker::modal(Commands::new(), window, cx));
//! modal_layer.update(cx, |layer, cx| layer.show(picker, window, cx));
//!
//! // compact popup positioned by the parent, e.g. at the caret
//! let menu = cx.new(|cx| Picker::popup(FormatMenu::new(), window, cx));
//! deferred(anchored().position(caret).child(menu.clone()))
//!
//! // popup whose query is the text typed in the document (insert menu):
//! // no query field, the editor keeps focus and forwards keys
//! let menu = cx.new(|cx| Picker::popup(InsertMenu::new(), window, cx).external_query());
//! menu.update(cx, |menu, cx| menu.set_query("hea", window, cx));
//! menu.update(cx, |menu, cx| menu.select_next(window, cx));   // editor's down / ctrl-n
//! menu.update(cx, |menu, cx| menu.confirm(false, window, cx)); // editor's enter
//! ```
//!
//! The picker emits [`DismissEvent`] when it wants to close (escape, or a
//! confirm whose delegate returned `true`): a modal layer closes it
//! automatically; a popup's parent subscribes and drops it.
//!
//! # Keys
//!
//! In the `"Picker"` key context: `up` / `ctrl-p`, `down` / `ctrl-n`,
//! `pageup`, `pagedown`, `home` / `cmd-up`, `end` / `cmd-down`, `enter`,
//! `cmd-enter` (secondary confirm), `escape`. The query field passes the
//! navigation keys through (see [`TextField::set_list_navigation`]).

use std::ops::Range;

use gpui::{
    App, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, KeyBinding,
    MouseButton, Pixels, ScrollStrategy, SharedString, Subscription, UniformListScrollHandle,
    Window, actions, div, prelude::*, px, uniform_list,
};

use crate::{
    icon::{Icon, IconName},
    key_hint::KeyHint,
    label::{Divider, HighlightedLabel},
    text_field::{FieldAppearance, TextField, TextFieldEvent},
    theme::{ActiveTheme, RADIUS_LG, RADIUS_MD, UI_TEXT_BASE, UI_TEXT_LG, UI_TEXT_SM, UI_TEXT_XS},
};

pub const KEY_CONTEXT: &str = "Picker";

actions!(
    picker,
    [
        /// Select the previous item.
        SelectPrev,
        /// Select the next item.
        SelectNext,
        /// Move the selection up by a page.
        SelectPageUp,
        /// Move the selection down by a page.
        SelectPageDown,
        /// Select the first item.
        SelectFirst,
        /// Select the last item.
        SelectLast,
        /// Confirm the selected item.
        Confirm,
        /// Confirm the selected item with the secondary action.
        SecondaryConfirm,
        /// Close the picker.
        Dismiss,
    ]
);

pub fn key_bindings() -> Vec<KeyBinding> {
    let c = Some(KEY_CONTEXT);
    vec![
        KeyBinding::new("ctrl-p", SelectPrev, c),
        KeyBinding::new("up", SelectPrev, c),
        KeyBinding::new("ctrl-n", SelectNext, c),
        KeyBinding::new("down", SelectNext, c),
        KeyBinding::new("pageup", SelectPageUp, c),
        KeyBinding::new("pagedown", SelectPageDown, c),
        KeyBinding::new("cmd-up", SelectFirst, c),
        KeyBinding::new("home", SelectFirst, c),
        KeyBinding::new("cmd-down", SelectLast, c),
        KeyBinding::new("end", SelectLast, c),
        KeyBinding::new("enter", Confirm, c),
        KeyBinding::new("cmd-enter", SecondaryConfirm, c),
        KeyBinding::new("escape", Dismiss, c),
    ]
}

/// A selectable row.
#[derive(Clone, Debug, Default)]
pub struct PickerItem {
    pub label: SharedString,
    /// Byte offsets of matched characters in `label` (highlighted).
    pub label_positions: Vec<usize>,
    /// Secondary text after the label (folder path, font sample, ...).
    pub detail: Option<SharedString>,
    pub detail_positions: Vec<usize>,
    pub icon: Option<IconName>,
    /// Right-aligned key binding.
    pub key_hint: Option<KeyHint>,
}

impl PickerItem {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            ..Default::default()
        }
    }

    pub fn positions(mut self, positions: Vec<usize>) -> Self {
        self.label_positions = positions;
        self
    }

    pub fn detail(mut self, detail: impl Into<SharedString>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn detail_positions(mut self, positions: Vec<usize>) -> Self {
        self.detail_positions = positions;
        self
    }

    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Accepts `Option` so `KeyHint::for_action(..)` can be passed directly.
    pub fn key_hint(mut self, hint: impl Into<Option<KeyHint>>) -> Self {
        self.key_hint = hint.into();
        self
    }
}

/// One row of the list.
#[derive(Clone, Debug)]
pub enum PickerEntry {
    /// A non-selectable section header.
    Header(SharedString),
    Item(PickerItem),
}

impl From<PickerItem> for PickerEntry {
    fn from(item: PickerItem) -> Self {
        PickerEntry::Item(item)
    }
}

/// Supplies and filters a [`Picker`]'s entries. See the [module docs](self).
pub trait PickerDelegate: Sized + 'static {
    /// Recomputes the entries for `query`. Called once at creation with the
    /// initial (empty) query and after every query change.
    fn update_matches(&mut self, query: &str, window: &mut Window, cx: &mut Context<Picker<Self>>);

    /// Number of entries, headers included.
    fn entry_count(&self) -> usize;

    /// The entry at `ix` (< `entry_count()`). Called for visible rows only.
    fn entry(&self, ix: usize, cx: &App) -> PickerEntry;

    /// Whether `ix` can be selected. Default: items yes, headers no.
    fn is_selectable(&self, ix: usize, cx: &App) -> bool {
        matches!(self.entry(ix, cx), PickerEntry::Item(_))
    }

    /// `enter` (`secondary = false`) or `cmd-enter` (`secondary = true`) on
    /// entry `ix`. Return `true` to dismiss the picker.
    fn confirm(
        &mut self,
        ix: usize,
        secondary: bool,
        window: &mut Window,
        cx: &mut Context<Picker<Self>>,
    ) -> bool;

    /// The picker is closing (escape, backdrop click, or after a confirm).
    fn dismissed(&mut self, _window: &mut Window, _cx: &mut Context<Picker<Self>>) {}

    /// The selection moved (live previews, e.g. the font picker).
    fn selection_changed(
        &mut self,
        _ix: Option<usize>,
        _window: &mut Window,
        _cx: &mut Context<Picker<Self>>,
    ) {
    }

    /// Which entry to select after `update_matches`. Default: the first
    /// selectable one.
    fn initial_selection(&self, _query: &str) -> Option<usize> {
        None
    }

    /// Placeholder of the query field.
    fn placeholder(&self) -> SharedString {
        "Search".into()
    }

    /// Text shown when there are no entries.
    fn empty_text(&self, _query: &str) -> SharedString {
        "No matches".into()
    }
}

/// Modal (palette) or popup (menu at the caret) look.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerPresentation {
    Modal,
    Popup,
}

/// See the [module docs](self).
pub struct Picker<D: PickerDelegate> {
    /// The delegate; mutate it and call [`Picker::refresh`] if its data changed.
    pub delegate: D,
    presentation: PickerPresentation,
    query_field: Option<Entity<TextField>>,
    external_query: String,
    selected: Option<usize>,
    scroll: UniformListScrollHandle,
    focus_handle: FocusHandle,
    width: Pixels,
    max_rows: usize,
    dismissed: bool,
    _subscriptions: Vec<Subscription>,
}

impl<D: PickerDelegate> EventEmitter<DismissEvent> for Picker<D> {}

impl<D: PickerDelegate> Focusable for Picker<D> {
    /// The query field's handle, or the picker's own one without a field.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        match &self.query_field {
            Some(field) => field.focus_handle(cx),
            None => self.focus_handle.clone(),
        }
    }
}

impl<D: PickerDelegate> Picker<D> {
    /// The command palette look: 560px wide, large query row, 32px rows.
    pub fn modal(delegate: D, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::new(delegate, PickerPresentation::Modal, window, cx)
    }

    /// The compact menu look: 280px wide, small query row, 28px rows.
    pub fn popup(delegate: D, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::new(delegate, PickerPresentation::Popup, window, cx)
    }

    fn new(
        delegate: D,
        presentation: PickerPresentation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let placeholder = delegate.placeholder();
        let field = cx.new(|cx| {
            let mut field = TextField::new(window, cx)
                .with_placeholder(placeholder)
                .with_appearance(FieldAppearance::Borderless)
                .with_text_size(match presentation {
                    PickerPresentation::Modal => UI_TEXT_LG,
                    PickerPresentation::Popup => UI_TEXT_BASE,
                });
            field.set_list_navigation(true);
            field
        });
        let subscription =
            cx.subscribe_in(
                &field,
                window,
                |this, field, event, window, cx| match event {
                    TextFieldEvent::Changed => {
                        let query = field.read(cx).text().to_string();
                        this.run_query(&query, window, cx);
                    }
                    TextFieldEvent::Submitted => this.confirm(false, window, cx),
                    _ => {}
                },
            );
        let mut picker = Self {
            delegate,
            presentation,
            query_field: Some(field),
            external_query: String::new(),
            selected: None,
            scroll: UniformListScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            width: match presentation {
                PickerPresentation::Modal => px(560.),
                PickerPresentation::Popup => px(280.),
            },
            max_rows: match presentation {
                PickerPresentation::Modal => 12,
                PickerPresentation::Popup => 8,
            },
            dismissed: false,
            _subscriptions: vec![subscription],
        };
        picker.run_query("", window, cx);
        picker
    }

    // ---------------------------------------------------------------------
    // Builders
    // ---------------------------------------------------------------------

    /// Removes the query field: the parent drives the query with
    /// [`Self::set_query`] and forwards navigation keys.
    pub fn external_query(mut self) -> Self {
        self.query_field = None;
        self._subscriptions.clear();
        self
    }

    pub fn width(mut self, width: Pixels) -> Self {
        self.width = width;
        self
    }

    /// Rows visible before the list scrolls.
    pub fn max_visible_rows(mut self, rows: usize) -> Self {
        self.max_rows = rows.max(1);
        self
    }

    // ---------------------------------------------------------------------
    // State
    // ---------------------------------------------------------------------

    /// The current query.
    pub fn query(&self, cx: &App) -> String {
        match &self.query_field {
            Some(field) => field.read(cx).text().to_string(),
            None => self.external_query.clone(),
        }
    }

    /// Replaces the query and refilters (works with and without a field).
    pub fn set_query(
        &mut self,
        query: impl Into<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let query = query.into();
        match &self.query_field {
            Some(field) => field.update(cx, |field, cx| field.set_text(query.clone(), cx)),
            None => self.external_query = query.clone(),
        }
        self.run_query(&query, window, cx);
    }

    /// The query field, if the picker has one.
    pub fn query_field(&self) -> Option<&Entity<TextField>> {
        self.query_field.as_ref()
    }

    pub fn presentation(&self) -> PickerPresentation {
        self.presentation
    }

    pub fn selected_index(&self) -> Option<usize> {
        self.selected
    }

    /// Selects `ix` if it is selectable and scrolls it into view.
    pub fn set_selected_index(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix < self.delegate.entry_count() && self.delegate.is_selectable(ix, cx) {
            let strategy = match self.selected {
                Some(current) if ix > current => ScrollStrategy::Bottom,
                _ => ScrollStrategy::Top,
            };
            self.select(Some(ix), strategy, window, cx);
        }
    }

    /// Re-runs `update_matches` with the current query (call after the
    /// delegate's data changed).
    pub fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.query(cx);
        self.run_query(&query, window, cx);
    }

    fn run_query(&mut self, query: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.delegate.update_matches(query, window, cx);
        let count = self.delegate.entry_count();
        let initial = self
            .delegate
            .initial_selection(query)
            .filter(|ix| *ix < count && self.delegate.is_selectable(*ix, cx))
            .or_else(|| self.next_selectable(0, cx));
        // Show headers above the first item.
        self.scroll.scroll_to_item_strict(0, ScrollStrategy::Top);
        self.selected = None;
        self.select(initial, ScrollStrategy::Top, window, cx);
        cx.notify();
    }

    fn next_selectable(&self, from: usize, cx: &App) -> Option<usize> {
        (from..self.delegate.entry_count()).find(|ix| self.delegate.is_selectable(*ix, cx))
    }

    fn prev_selectable(&self, from: usize, cx: &App) -> Option<usize> {
        let count = self.delegate.entry_count();
        if count == 0 {
            return None;
        }
        (0..=from.min(count - 1))
            .rev()
            .find(|ix| self.delegate.is_selectable(*ix, cx))
    }

    fn select(
        &mut self,
        ix: Option<usize>,
        strategy: ScrollStrategy,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(ix) = ix {
            // Reveal a header right above the selection when scrolling up.
            let target = if strategy == ScrollStrategy::Top
                && ix > 0
                && matches!(self.delegate.entry(ix - 1, cx), PickerEntry::Header(_))
            {
                ix - 1
            } else {
                ix
            };
            self.scroll.scroll_to_item(target, strategy);
        }
        if self.selected != ix {
            self.selected = ix;
            self.delegate.selection_changed(ix, window, cx);
        }
        cx.notify();
    }

    pub fn select_next(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let from = self.selected.map_or(0, |ix| ix + 1);
        if let Some(ix) = self.next_selectable(from, cx) {
            self.select(Some(ix), ScrollStrategy::Bottom, window, cx);
        }
    }

    pub fn select_prev(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(current) = self.selected else {
            return self.select_first(window, cx);
        };
        if current > 0
            && let Some(ix) = self.prev_selectable(current - 1, cx)
        {
            self.select(Some(ix), ScrollStrategy::Top, window, cx);
        }
    }

    pub fn select_first(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let first = self.next_selectable(0, cx);
        self.scroll.scroll_to_item_strict(0, ScrollStrategy::Top);
        self.select(first, ScrollStrategy::Top, window, cx);
    }

    pub fn select_last(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let last = self.prev_selectable(self.delegate.entry_count().saturating_sub(1), cx);
        self.select(last, ScrollStrategy::Bottom, window, cx);
    }

    pub fn select_page_down(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let count = self.delegate.entry_count();
        if count == 0 {
            return;
        }
        let target =
            (self.selected.unwrap_or(0) + self.max_rows.saturating_sub(1).max(1)).min(count - 1);
        let ix = self
            .next_selectable(target, cx)
            .or_else(|| self.prev_selectable(target, cx));
        self.select(ix, ScrollStrategy::Bottom, window, cx);
    }

    pub fn select_page_up(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target = self
            .selected
            .unwrap_or(0)
            .saturating_sub(self.max_rows.saturating_sub(1).max(1));
        let ix = self
            .prev_selectable(target, cx)
            .or_else(|| self.next_selectable(target, cx));
        self.select(ix, ScrollStrategy::Top, window, cx);
    }

    /// Confirms the selected entry (no-op without a selection).
    pub fn confirm(&mut self, secondary: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.selected else {
            return;
        };
        if self.delegate.confirm(ix, secondary, window, cx) {
            self.dismiss(window, cx);
        }
    }

    /// Tells the delegate and emits [`DismissEvent`] (once).
    pub fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dismissed {
            return;
        }
        self.dismissed = true;
        self.delegate.dismissed(window, cx);
        cx.emit(DismissEvent);
    }

    // ---------------------------------------------------------------------
    // Rendering
    // ---------------------------------------------------------------------

    fn row_height(&self) -> Pixels {
        match self.presentation {
            PickerPresentation::Modal => px(32.),
            PickerPresentation::Popup => px(28.),
        }
    }

    fn render_rows(
        &mut self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<gpui::AnyElement> {
        let colors = cx.theme().colors;
        let row_height = self.row_height();
        range
            .map(|ix| {
                let entry = self.delegate.entry(ix, cx);
                let outer = div()
                    .id(("picker-row", ix))
                    .h(row_height)
                    .w_full()
                    .px(px(4.))
                    .flex();
                match entry {
                    PickerEntry::Header(title) => outer
                        .items_end()
                        .pb(px(4.))
                        .child(
                            div()
                                .px(px(8.))
                                .text_size(UI_TEXT_XS)
                                .line_height(px(14.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(colors.muted)
                                .child(title),
                        )
                        .into_any_element(),
                    PickerEntry::Item(item) => {
                        let selected = self.selected == Some(ix);
                        outer
                            .cursor_pointer()
                            .on_mouse_move(cx.listener(move |this, _, window, cx| {
                                if this.selected != Some(ix) {
                                    this.select(Some(ix), ScrollStrategy::Top, window, cx);
                                }
                            }))
                            .on_mouse_down(MouseButton::Left, |_, window, _| {
                                // Keep focus in the query field.
                                window.prevent_default();
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.selected = Some(ix);
                                this.confirm(false, window, cx);
                            }))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .h_full()
                                    .px(px(8.))
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .rounded(RADIUS_MD)
                                    .when(selected, |el| el.bg(colors.accent_subtle))
                                    .when_some(item.icon, |el, icon| {
                                        el.child(Icon::new(icon).color(if selected {
                                            colors.ink
                                        } else {
                                            colors.muted
                                        }))
                                    })
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .flex()
                                            .items_center()
                                            .gap(px(8.))
                                            .overflow_hidden()
                                            .child(div().flex_none().max_w(relative_full()).child(
                                                HighlightedLabel::new(
                                                    item.label,
                                                    item.label_positions,
                                                ),
                                            ))
                                            .when_some(item.detail, |el, detail| {
                                                el.child(
                                                    div().flex_1().min_w_0().child(
                                                        HighlightedLabel::new(
                                                            detail,
                                                            item.detail_positions,
                                                        )
                                                        .color(colors.muted)
                                                        .size(UI_TEXT_SM),
                                                    ),
                                                )
                                            }),
                                    )
                                    .when_some(item.key_hint, |el, hint| el.child(hint)),
                            )
                            .into_any_element()
                    }
                }
            })
            .collect()
    }

    fn on_select_prev(&mut self, _: &SelectPrev, window: &mut Window, cx: &mut Context<Self>) {
        self.select_prev(window, cx);
    }
    fn on_select_next(&mut self, _: &SelectNext, window: &mut Window, cx: &mut Context<Self>) {
        self.select_next(window, cx);
    }
    fn on_page_up(&mut self, _: &SelectPageUp, window: &mut Window, cx: &mut Context<Self>) {
        self.select_page_up(window, cx);
    }
    fn on_page_down(&mut self, _: &SelectPageDown, window: &mut Window, cx: &mut Context<Self>) {
        self.select_page_down(window, cx);
    }
    fn on_first(&mut self, _: &SelectFirst, window: &mut Window, cx: &mut Context<Self>) {
        self.select_first(window, cx);
    }
    fn on_last(&mut self, _: &SelectLast, window: &mut Window, cx: &mut Context<Self>) {
        self.select_last(window, cx);
    }
    fn on_confirm(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm(false, window, cx);
    }
    fn on_secondary(&mut self, _: &SecondaryConfirm, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm(true, window, cx);
    }
    fn on_dismiss(&mut self, _: &Dismiss, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss(window, cx);
    }
}

fn relative_full() -> gpui::DefiniteLength {
    gpui::relative(1.)
}

impl<D: PickerDelegate> Render for Picker<D> {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let colors = theme.colors;
        let modal = self.presentation == PickerPresentation::Modal;
        let count = self.delegate.entry_count();
        let row_height = self.row_height();
        let visible_rows = count.min(self.max_rows);
        let list_height = row_height * visible_rows as f32;
        let query = self.query(cx);

        let query_row = self.query_field.clone().map(|field| {
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(if modal { 10. } else { 8. }))
                .h(px(if modal { 48. } else { 34. }))
                .px(px(if modal { 14. } else { 10. }))
                .child(Icon::new(IconName::Search).color(colors.muted))
                .child(div().flex_1().min_w_0().child(field))
        });

        div()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            // Clicks inside the panel never move focus: the query field (or,
            // for an external query, the parent editor) keeps it. Runs before
            // the focus-on-click listener of `track_focus` (same element,
            // registered later, bubble order is reversed).
            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
            .on_action(cx.listener(Self::on_select_prev))
            .on_action(cx.listener(Self::on_select_next))
            .on_action(cx.listener(Self::on_page_up))
            .on_action(cx.listener(Self::on_page_down))
            .on_action(cx.listener(Self::on_first))
            .on_action(cx.listener(Self::on_last))
            .on_action(cx.listener(Self::on_confirm))
            .on_action(cx.listener(Self::on_secondary))
            .on_action(cx.listener(Self::on_dismiss))
            .w(self.width)
            .max_w_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(colors.elevated)
            .border_1()
            .border_color(colors.hairline)
            .rounded(RADIUS_LG)
            .shadow(if modal {
                theme.shadow_overlay()
            } else {
                theme.shadow_popover()
            })
            .font_family(theme.fonts.ui.clone())
            .text_size(UI_TEXT_BASE)
            .line_height(px(18.))
            .text_color(colors.ink)
            .when_some(query_row, |el, row| {
                el.child(row).child(Divider::horizontal())
            })
            .map(|el| {
                if count == 0 {
                    el.child(
                        div()
                            .py(px(if modal { 20. } else { 10. }))
                            .px(px(12.))
                            .flex()
                            .justify_center()
                            .text_color(colors.muted)
                            .child(self.delegate.empty_text(&query)),
                    )
                } else {
                    el.child(
                        div().py(px(4.)).child(
                            uniform_list(
                                "picker-list",
                                count,
                                cx.processor(|this, range, window, cx| {
                                    this.render_rows(range, window, cx)
                                }),
                            )
                            .track_scroll(self.scroll.clone())
                            .h(list_height)
                            .w_full(),
                        ),
                    )
                }
            })
    }
}
