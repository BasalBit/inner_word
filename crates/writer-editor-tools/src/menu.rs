//! The popup list shared by the format menu, the insert menu and the font
//! picker: an optional query field over a virtualized list of rows.
//!
//! It has the look of `writer_ui::Picker` in its popup presentation and
//! uses the picker's key context and actions (so `up` / `down` / `ctrl-n` /
//! `ctrl-p` / `pageup` / `pagedown` / `home` / `end` / `cmd-enter` come from
//! the same bindings). It differs in what a row can show: a state mark
//! (check or "mixed" dash) in a fixed column, a colour swatch, and a sample
//! in a font. The owner supplies ready-made entries and reacts to
//! [`MenuEvent`]s; the menu owns selection, scrolling and keys.

use std::ops::Range;

use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, Hsla, MouseButton, Pixels,
    ScrollStrategy, SharedString, Subscription, UniformListScrollHandle, Window, div, prelude::*,
    px, uniform_list,
};
use writer_ui::{
    Divider, FieldAppearance, HighlightedLabel, Icon, IconName, KeyHint, TextField, TextFieldEvent,
    picker,
    theme::{ActiveTheme, RADIUS_LG, RADIUS_MD, UI_TEXT_BASE, UI_TEXT_SM, UI_TEXT_XS},
};

/// The state mark in front of a row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mark {
    /// Off, or not a stateful row.
    #[default]
    None,
    /// On for the whole selection.
    Check,
    /// On for a part of the selection only.
    Mixed,
}

/// A selectable row.
#[derive(Clone, Debug, Default)]
pub(crate) struct MenuItem {
    /// The owner's handle for the row (an index into its own item table).
    pub(crate) id: usize,
    pub(crate) label: SharedString,
    /// Byte offsets of matched characters in `label`.
    pub(crate) positions: Vec<usize>,
    pub(crate) detail: Option<SharedString>,
    pub(crate) mark: Mark,
    pub(crate) swatch: Option<Hsla>,
    /// A font family to show a short sample in.
    pub(crate) sample_font: Option<SharedString>,
    pub(crate) key_hint: Option<KeyHint>,
}

#[derive(Clone, Debug)]
pub(crate) enum MenuEntry {
    Header(SharedString),
    Item(MenuItem),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MenuEvent {
    /// The text of the query field changed.
    QueryChanged,
    /// `enter` / click (`secondary = false`) or `cmd-enter` on the selected
    /// row.
    Confirmed { secondary: bool },
    /// `escape`.
    Dismissed,
    /// The query field lost focus.
    Blurred,
}

pub(crate) struct MenuOptions {
    /// `Some`: the menu has a query field with this placeholder and takes
    /// focus. `None`: the owner drives the query and the keys.
    pub(crate) placeholder: Option<SharedString>,
    pub(crate) width: Pixels,
    pub(crate) max_rows: usize,
    /// Reserve the mark column.
    pub(crate) marks: bool,
    pub(crate) empty_text: SharedString,
    /// Key hints under the list ("⌘↩ Set as document font").
    pub(crate) footer: Vec<(KeyHint, SharedString)>,
}

const ROW_HEIGHT: Pixels = px(28.);

pub(crate) struct Menu {
    entries: Vec<MenuEntry>,
    selected: Option<usize>,
    query_field: Option<Entity<TextField>>,
    scroll: UniformListScrollHandle,
    focus_handle: FocusHandle,
    width: Pixels,
    max_rows: usize,
    marks: bool,
    empty_text: SharedString,
    footer: Vec<(KeyHint, SharedString)>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<MenuEvent> for Menu {}

impl Focusable for Menu {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        match &self.query_field {
            Some(field) => field.focus_handle(cx),
            None => self.focus_handle.clone(),
        }
    }
}

impl Menu {
    pub(crate) fn new(options: MenuOptions, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut subscriptions = Vec::new();
        let query_field = options.placeholder.map(|placeholder| {
            let field = cx.new(|cx| {
                let mut field = TextField::new(window, cx)
                    .with_placeholder(placeholder)
                    .with_appearance(FieldAppearance::Borderless)
                    .with_text_size(UI_TEXT_BASE)
                    .consume_escape();
                field.set_list_navigation(true);
                field
            });
            subscriptions.push(cx.subscribe(
                &field,
                |this: &mut Self, _, event: &TextFieldEvent, cx| match event {
                    TextFieldEvent::Changed => cx.emit(MenuEvent::QueryChanged),
                    TextFieldEvent::Submitted => this.confirm(false, cx),
                    TextFieldEvent::Cancelled => cx.emit(MenuEvent::Dismissed),
                    TextFieldEvent::Blurred => cx.emit(MenuEvent::Blurred),
                    TextFieldEvent::Focused => {}
                },
            ));
            field
        });
        Self {
            entries: Vec::new(),
            selected: None,
            query_field,
            scroll: UniformListScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            width: options.width,
            max_rows: options.max_rows.max(1),
            marks: options.marks,
            empty_text: options.empty_text,
            footer: options.footer,
            _subscriptions: subscriptions,
        }
    }

    /// The text of the query field (empty without a field).
    pub(crate) fn query(&self, cx: &App) -> String {
        self.query_field
            .as_ref()
            .map(|field| field.read(cx).text().to_string())
            .unwrap_or_default()
    }

    pub(crate) fn entries(&self) -> &[MenuEntry] {
        &self.entries
    }

    /// Replaces the rows. Selects the row with id `select` when it is
    /// there, else the first row.
    pub(crate) fn set_entries(
        &mut self,
        entries: Vec<MenuEntry>,
        select: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        self.entries = entries;
        let preferred = select.and_then(|id| {
            self.entries
                .iter()
                .position(|entry| matches!(entry, MenuEntry::Item(item) if item.id == id))
        });
        self.selected = preferred.or_else(|| self.next_selectable(0));
        match preferred {
            // Bring the preselected row into view with some context above.
            Some(ix) => self
                .scroll
                .scroll_to_item(ix.saturating_sub(2), ScrollStrategy::Top),
            // Show the header above the first row.
            None => self.scroll.scroll_to_item_strict(0, ScrollStrategy::Top),
        }
        cx.notify();
    }

    pub(crate) fn selected_item(&self) -> Option<&MenuItem> {
        match self.entries.get(self.selected?)? {
            MenuEntry::Item(item) => Some(item),
            MenuEntry::Header(_) => None,
        }
    }

    pub(crate) fn selected_id(&self) -> Option<usize> {
        self.selected_item().map(|item| item.id)
    }

    /// The index of the selected row among the entries.
    pub(crate) fn selected_index(&self) -> Option<usize> {
        self.selected
    }

    fn is_selectable(&self, ix: usize) -> bool {
        matches!(self.entries.get(ix), Some(MenuEntry::Item(_)))
    }

    fn next_selectable(&self, from: usize) -> Option<usize> {
        (from..self.entries.len()).find(|ix| self.is_selectable(*ix))
    }

    fn prev_selectable(&self, from: usize) -> Option<usize> {
        if self.entries.is_empty() {
            return None;
        }
        (0..=from.min(self.entries.len() - 1))
            .rev()
            .find(|ix| self.is_selectable(*ix))
    }

    fn select(&mut self, ix: Option<usize>, strategy: ScrollStrategy, cx: &mut Context<Self>) {
        if let Some(ix) = ix {
            // Reveal a header right above the selection when moving up.
            let target = if strategy == ScrollStrategy::Top
                && ix > 0
                && matches!(self.entries[ix - 1], MenuEntry::Header(_))
            {
                ix - 1
            } else {
                ix
            };
            self.scroll.scroll_to_item(target, strategy);
        }
        self.selected = ix;
        cx.notify();
    }

    pub(crate) fn select_next(&mut self, cx: &mut Context<Self>) {
        let from = self.selected.map_or(0, |ix| ix + 1);
        if let Some(ix) = self.next_selectable(from) {
            self.select(Some(ix), ScrollStrategy::Bottom, cx);
        }
    }

    pub(crate) fn select_prev(&mut self, cx: &mut Context<Self>) {
        let Some(current) = self.selected else {
            return self.select_first(cx);
        };
        if current > 0
            && let Some(ix) = self.prev_selectable(current - 1)
        {
            self.select(Some(ix), ScrollStrategy::Top, cx);
        }
    }

    pub(crate) fn select_first(&mut self, cx: &mut Context<Self>) {
        let first = self.next_selectable(0);
        self.scroll.scroll_to_item_strict(0, ScrollStrategy::Top);
        self.select(first, ScrollStrategy::Top, cx);
    }

    pub(crate) fn select_last(&mut self, cx: &mut Context<Self>) {
        let last = self.prev_selectable(self.entries.len().saturating_sub(1));
        self.select(last, ScrollStrategy::Bottom, cx);
    }

    fn select_page(&mut self, down: bool, cx: &mut Context<Self>) {
        if self.entries.is_empty() {
            return;
        }
        let step = self.max_rows.saturating_sub(1).max(1);
        let current = self.selected.unwrap_or(0);
        if down {
            let target = (current + step).min(self.entries.len() - 1);
            let ix = self
                .next_selectable(target)
                .or_else(|| self.prev_selectable(target));
            self.select(ix, ScrollStrategy::Bottom, cx);
        } else {
            let target = current.saturating_sub(step);
            let ix = self
                .prev_selectable(target)
                .or_else(|| self.next_selectable(target));
            self.select(ix, ScrollStrategy::Top, cx);
        }
    }

    /// Emits [`MenuEvent::Confirmed`] when a row is selected.
    pub(crate) fn confirm(&mut self, secondary: bool, cx: &mut Context<Self>) {
        if self.selected_item().is_some() {
            cx.emit(MenuEvent::Confirmed { secondary });
        }
    }

    fn render_rows(
        &mut self,
        range: Range<usize>,
        cx: &mut Context<Self>,
    ) -> Vec<gpui::AnyElement> {
        let colors = cx.theme().colors;
        range
            .map(|ix| {
                let outer = div()
                    .id(("menu-row", ix))
                    .debug_selector(|| format!("menu-row-{ix}"))
                    .h(ROW_HEIGHT)
                    .w_full()
                    .px(px(4.))
                    .flex();
                match self.entries[ix].clone() {
                    MenuEntry::Header(title) => outer
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
                    MenuEntry::Item(item) => {
                        let selected = self.selected == Some(ix);
                        let mark = self.marks.then(|| {
                            let slot = div()
                                .flex_none()
                                .size(px(16.))
                                .flex()
                                .items_center()
                                .justify_center();
                            match item.mark {
                                Mark::None => slot,
                                Mark::Check => slot.child(
                                    Icon::new(IconName::Check)
                                        .size(px(14.))
                                        .color(colors.accent),
                                ),
                                Mark::Mixed => slot.child(
                                    div().w(px(8.)).h(px(2.)).rounded(px(1.)).bg(colors.muted),
                                ),
                            }
                        });
                        let swatch = item.swatch.map(|color| {
                            div()
                                .flex_none()
                                .size(px(12.))
                                .rounded(px(3.))
                                .bg(color)
                                .border_1()
                                .border_color(colors.border_strong)
                        });
                        let sample = item.sample_font.map(|family| {
                            div()
                                .flex_none()
                                .font_family(family)
                                .text_size(UI_TEXT_BASE)
                                .text_color(colors.muted)
                                .child("Aa Bb Cc")
                        });
                        outer
                            .cursor_pointer()
                            .on_mouse_move(cx.listener(move |this, _, _, cx| {
                                if this.selected != Some(ix) {
                                    this.selected = Some(ix);
                                    cx.notify();
                                }
                            }))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.selected = Some(ix);
                                this.confirm(false, cx);
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
                                    .children(mark)
                                    .children(swatch)
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .flex()
                                            .items_center()
                                            .gap(px(8.))
                                            .overflow_hidden()
                                            .child(
                                                div().flex_none().max_w(gpui::relative(1.)).child(
                                                    HighlightedLabel::new(
                                                        item.label,
                                                        item.positions,
                                                    ),
                                                ),
                                            )
                                            .when_some(item.detail, |el, detail| {
                                                el.child(
                                                    div().flex_1().min_w_0().child(
                                                        HighlightedLabel::new(detail, Vec::new())
                                                            .color(colors.muted)
                                                            .size(UI_TEXT_SM),
                                                    ),
                                                )
                                            }),
                                    )
                                    .children(sample)
                                    .children(item.key_hint),
                            )
                            .into_any_element()
                    }
                }
            })
            .collect()
    }
}

impl Render for Menu {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let colors = theme.colors;
        let count = self.entries.len();
        let list_height = ROW_HEIGHT * count.min(self.max_rows) as f32;

        let query_row = self.query_field.clone().map(|field| {
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(8.))
                .h(px(34.))
                .px(px(10.))
                .child(Icon::new(IconName::Search).color(colors.muted))
                .child(div().flex_1().min_w_0().child(field))
        });
        let footer = (!self.footer.is_empty()).then(|| {
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(12.))
                .h(px(28.))
                .px(px(12.))
                .text_size(UI_TEXT_XS)
                .text_color(colors.muted)
                .children(self.footer.iter().map(|(hint, label)| {
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .child(hint.clone().size(UI_TEXT_XS))
                        .child(label.clone())
                }))
        });

        div()
            .key_context(picker::KEY_CONTEXT)
            .debug_selector(|| "tools-menu".into())
            .track_focus(&self.focus_handle)
            // The panel is over the document: clicks must not reach the
            // text under it, and must not take focus from the query field
            // (or, without a field, from the editor).
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
            .on_action(cx.listener(|this, _: &picker::SelectPrev, _, cx| this.select_prev(cx)))
            .on_action(cx.listener(|this, _: &picker::SelectNext, _, cx| this.select_next(cx)))
            .on_action(cx.listener(|this, _: &picker::SelectFirst, _, cx| this.select_first(cx)))
            .on_action(cx.listener(|this, _: &picker::SelectLast, _, cx| this.select_last(cx)))
            .on_action(
                cx.listener(|this, _: &picker::SelectPageUp, _, cx| this.select_page(false, cx)),
            )
            .on_action(
                cx.listener(|this, _: &picker::SelectPageDown, _, cx| this.select_page(true, cx)),
            )
            .on_action(cx.listener(|this, _: &picker::Confirm, _, cx| this.confirm(false, cx)))
            .on_action(
                cx.listener(|this, _: &picker::SecondaryConfirm, _, cx| this.confirm(true, cx)),
            )
            .on_action(cx.listener(|_, _: &picker::Dismiss, _, cx| cx.emit(MenuEvent::Dismissed)))
            .w(self.width)
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(colors.elevated)
            .border_1()
            .border_color(colors.hairline)
            .rounded(RADIUS_LG)
            .shadow(theme.shadow_popover())
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
                            .py(px(10.))
                            .px(px(12.))
                            .flex()
                            .justify_center()
                            .text_color(colors.muted)
                            .child(self.empty_text.clone()),
                    )
                } else {
                    el.child(
                        div().py(px(4.)).child(
                            uniform_list(
                                "menu-list",
                                count,
                                cx.processor(|this, range, _, cx| this.render_rows(range, cx)),
                            )
                            .track_scroll(self.scroll.clone())
                            .h(list_height)
                            .w_full(),
                        ),
                    )
                }
            })
            .when_some(footer, |el, footer| {
                el.child(Divider::horizontal()).child(footer)
            })
    }
}

/// Ranks `(id, label, keywords)` candidates against `query`. The label is
/// matched fuzzily; the keywords (synonyms, separated by spaces) only by
/// prefix, so they do not flood the list. A match in the label beats a
/// match in the keywords; within the labels the better fuzzy score wins,
/// then the original order. Returns `(id, matched label
/// positions)`. An empty query returns everything in order.
pub(crate) fn rank<'a>(
    query: &str,
    candidates: impl IntoIterator<Item = (usize, &'a str, &'a str)>,
) -> Vec<(usize, Vec<usize>)> {
    let query = query.trim();
    if query.is_empty() {
        return candidates
            .into_iter()
            .map(|(id, _, _)| (id, Vec::new()))
            .collect();
    }
    let mut matches: Vec<(bool, i32, usize, usize, Vec<usize>)> = candidates
        .into_iter()
        .enumerate()
        .filter_map(|(order, (id, label, keywords))| {
            if let Some(found) = writer_ui::fuzzy::fuzzy_match(query, label) {
                return Some((false, found.score, order, id, found.positions));
            }
            keyword_match(query, keywords).then(|| (true, 0, order, id, Vec::new()))
        })
        .collect();
    matches.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)).then(a.2.cmp(&b.2)));
    matches
        .into_iter()
        .map(|(_, _, _, id, positions)| (id, positions))
        .collect()
}

/// Whether `query` is the start of one of the space-separated `keywords`
/// (or of the rest of the keyword string, for queries with spaces),
/// ignoring case.
fn keyword_match(query: &str, keywords: &str) -> bool {
    let query = query.to_lowercase();
    let keywords = keywords.to_lowercase();
    keywords
        .char_indices()
        .filter(|(index, _)| *index == 0 || keywords[..*index].ends_with(char::is_whitespace))
        .any(|(index, _)| keywords[index..].starts_with(&query))
}
