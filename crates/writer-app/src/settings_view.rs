//! The settings modal (`cmd-,`).
//!
//! One list of rows, each a focus stop. `up` / `down` (or `tab` /
//! `shift-tab`) move between rows, `left` / `right` change a choice,
//! `space` or `enter` flips a switch or steps a choice forward, and
//! `escape` closes. The mouse works on every control too.
//!
//! Every change is written to the library's settings at once; the workspace
//! applies it live (see [`crate::settings`]).

use gpui::{
    App, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, KeyBinding,
    MouseButton, ScrollHandle, SharedString, Subscription, Window, actions, div, prelude::*, px,
};
use serde::Serialize;
use writer_export::SocialNetwork;
use writer_library::{Library, LibraryEvent};
use writer_ui::{
    FieldAppearance, Icon, IconName, TextField, TextFieldEvent, ThemeMode, Toast,
    theme::{
        ActiveTheme, RADIUS_LG, RADIUS_MD, RADIUS_SM, SPACE_1, SPACE_2, SPACE_3, SPACE_4,
        UI_TEXT_BASE, UI_TEXT_LG, UI_TEXT_SM, UI_TEXT_XS, focus_ring,
    },
};

use crate::settings::{
    self, LINE_HEIGHTS, Paper, Settings, TEXT_SIZES, character_limit_name, keys,
};

actions!(
    settings,
    [
        /// Move to the next setting.
        FocusNext,
        /// Move to the previous setting.
        FocusPrevious,
        /// Choose the next value of the focused setting.
        Increase,
        /// Choose the previous value of the focused setting.
        Decrease,
        /// Flip the focused switch, or step the focused choice forward.
        Activate,
    ]
);

/// The key context of the settings modal.
pub const KEY_CONTEXT: &str = "SettingsView";
/// The key context of one focused setting row (not the text row).
pub const ROW_KEY_CONTEXT: &str = "SettingsRow";

/// The key bindings of the settings modal.
pub fn key_bindings() -> Vec<KeyBinding> {
    let view = Some(KEY_CONTEXT);
    let row = Some(ROW_KEY_CONTEXT);
    vec![
        KeyBinding::new("ctrl-n", FocusNext, view),
        KeyBinding::new("tab", FocusNext, view),
        KeyBinding::new("down", FocusNext, view),
        KeyBinding::new("ctrl-p", FocusPrevious, view),
        KeyBinding::new("shift-tab", FocusPrevious, view),
        KeyBinding::new("up", FocusPrevious, view),
        KeyBinding::new("right", Increase, row),
        KeyBinding::new("left", Decrease, row),
        KeyBinding::new("enter", Activate, row),
        KeyBinding::new("space", Activate, row),
    ]
}

/// The settings, in the order they are listed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SettingRow {
    /// Paper, Night or follow the system.
    Theme,
    /// Vim mode.
    VimMode,
    /// Markdown shortcuts while typing.
    MarkdownShortcuts,
    /// Smart quotes and dashes.
    SmartPunctuation,
    /// Author name for comments.
    Author,
    /// Font of new documents.
    Font,
    /// Text size of new documents.
    TextSize,
    /// Line height of new documents.
    LineHeight,
    /// Whether new documents start in page view.
    PageView,
    /// Paper size of new documents.
    Paper,
    /// The character-limit indicator.
    CharacterLimit,
}

impl SettingRow {
    /// Every row, in order.
    pub const ALL: [SettingRow; 11] = [
        SettingRow::Theme,
        SettingRow::VimMode,
        SettingRow::MarkdownShortcuts,
        SettingRow::SmartPunctuation,
        SettingRow::Author,
        SettingRow::Font,
        SettingRow::TextSize,
        SettingRow::LineHeight,
        SettingRow::PageView,
        SettingRow::Paper,
        SettingRow::CharacterLimit,
    ];

    fn section(self) -> &'static str {
        match self {
            SettingRow::Theme => "Appearance",
            SettingRow::VimMode
            | SettingRow::MarkdownShortcuts
            | SettingRow::SmartPunctuation
            | SettingRow::Author => "Writing",
            SettingRow::Font
            | SettingRow::TextSize
            | SettingRow::LineHeight
            | SettingRow::PageView
            | SettingRow::Paper => "New documents",
            SettingRow::CharacterLimit => "Status bar",
        }
    }

    fn label(self) -> &'static str {
        match self {
            SettingRow::Theme => "Theme",
            SettingRow::VimMode => "Vim mode",
            SettingRow::MarkdownShortcuts => "Markdown shortcuts",
            SettingRow::SmartPunctuation => "Smart punctuation",
            SettingRow::Author => "Name on comments",
            SettingRow::Font => "Font",
            SettingRow::TextSize => "Text size",
            SettingRow::LineHeight => "Line height",
            SettingRow::PageView => "Start in page view",
            SettingRow::Paper => "Paper size",
            SettingRow::CharacterLimit => "Character limit",
        }
    }

    fn description(self) -> &'static str {
        match self {
            SettingRow::Theme => "Paper is light, Night is dark.",
            SettingRow::VimMode => "Modal editing with Vim keys.",
            SettingRow::MarkdownShortcuts => {
                "Typing # , - , > or **bold** formats as you go."
            }
            SettingRow::SmartPunctuation => "Curly quotes, dashes and ellipses.",
            SettingRow::Author => "Written on the comments you add.",
            SettingRow::Font => "Body text of a new document.",
            SettingRow::TextSize => "In points.",
            SettingRow::LineHeight => "As a multiple of the text size.",
            SettingRow::PageView => "Show sheets of paper instead of one column.",
            SettingRow::Paper => "Used by page view and Word export.",
            SettingRow::CharacterLimit => "Count the text against a social network's post limit.",
        }
    }
}

/// How a row's value is shown and changed.
#[derive(Clone, Debug, PartialEq)]
enum Control {
    Switch(bool),
    /// A few options, all visible.
    Segments {
        options: Vec<SharedString>,
        selected: usize,
    },
    /// Many options, one visible, stepped with left and right.
    Stepper {
        value: SharedString,
        index: usize,
        count: usize,
    },
    Text,
}

/// The settings modal. See the [module docs](self).
pub struct SettingsView {
    library: Entity<Library>,
    settings: Settings,
    fonts: Vec<String>,
    handles: Vec<FocusHandle>,
    author_field: Entity<TextField>,
    scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DismissEvent> for SettingsView {}

impl Focusable for SettingsView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.handles[0].clone()
    }
}

fn nearest_index(values: &[f32], value: f32) -> usize {
    values
        .iter()
        .enumerate()
        .min_by(|a, b| (a.1 - value).abs().total_cmp(&(b.1 - value).abs()))
        .map(|(ix, _)| ix)
        .unwrap_or(0)
}

fn trim_number(value: f32) -> String {
    let text = format!("{value:.1}");
    text.strip_suffix(".0").map(str::to_string).unwrap_or(text)
}

impl SettingsView {
    /// Builds the modal over `library`.
    pub fn new(library: Entity<Library>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = Settings::load(library.read(cx));
        let fonts = settings::available_fonts(&settings.new_document_font, cx);
        let author_field = cx.new(|cx| {
            let mut field = TextField::new(window, cx)
                .with_placeholder("Your name")
                .with_text(settings.comment_author.clone())
                .with_appearance(FieldAppearance::Bordered);
            // up and down leave the field for the neighbouring rows.
            field.set_list_navigation(true);
            field
        });
        let author_handle = author_field.focus_handle(cx);
        let handles = SettingRow::ALL
            .iter()
            .map(|row| match row {
                SettingRow::Author => author_handle.clone(),
                _ => cx.focus_handle(),
            })
            .collect();
        let subscriptions = vec![
            cx.subscribe(
                &library,
                |this: &mut Self, library, event: &LibraryEvent, cx| {
                    if matches!(event, LibraryEvent::SettingChanged { .. }) {
                        this.settings = Settings::load(library.read(cx));
                        cx.notify();
                    }
                },
            ),
            cx.subscribe_in(
                &author_field,
                window,
                |this: &mut Self, field, event: &TextFieldEvent, window, cx| match event {
                    TextFieldEvent::Changed => {
                        let name = field.read(cx).text().trim().to_string();
                        this.write(keys::COMMENT_AUTHOR, &name, cx);
                    }
                    TextFieldEvent::Submitted => this.move_focus(1, window, cx),
                    _ => {}
                },
            ),
        ];
        Self {
            library,
            settings,
            fonts,
            handles,
            author_field,
            scroll: ScrollHandle::new(),
            _subscriptions: subscriptions,
        }
    }

    /// The settings as this view last read them.
    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// The row that has focus.
    pub fn focused_row(&self, window: &Window, cx: &App) -> Option<SettingRow> {
        self.handles
            .iter()
            .position(|handle| handle.contains_focused(window, cx))
            .map(|ix| SettingRow::ALL[ix])
    }

    fn write<T: Serialize + ?Sized>(&mut self, key: &str, value: &T, cx: &mut Context<Self>) {
        let result = self
            .library
            .update(cx, |library, cx| library.set_setting(key, value, cx));
        match result {
            Ok(()) => {
                self.settings = Settings::load(self.library.read(cx));
                cx.notify();
            }
            Err(error) => {
                writer_ui::toast::show_toast(
                    Toast::new(format!("Could not save the setting: {error}")).error(),
                    cx,
                );
            }
        }
    }

    fn limit_options() -> Vec<Option<SocialNetwork>> {
        std::iter::once(None)
            .chain(SocialNetwork::ALL.into_iter().map(Some))
            .collect()
    }

    fn control(&self, row: SettingRow) -> Control {
        let settings = &self.settings;
        match row {
            SettingRow::Theme => Control::Segments {
                options: vec!["System".into(), "Paper".into(), "Night".into()],
                selected: match settings.theme {
                    ThemeMode::System => 0,
                    ThemeMode::Light => 1,
                    ThemeMode::Dark => 2,
                },
            },
            SettingRow::VimMode => Control::Switch(settings.vim_mode),
            SettingRow::MarkdownShortcuts => Control::Switch(settings.input_rules),
            SettingRow::SmartPunctuation => Control::Switch(settings.smart_punctuation),
            SettingRow::Author => Control::Text,
            SettingRow::Font => {
                let index = self
                    .fonts
                    .iter()
                    .position(|font| *font == settings.new_document_font)
                    .unwrap_or(0);
                Control::Stepper {
                    value: settings.new_document_font.clone().into(),
                    index,
                    count: self.fonts.len(),
                }
            }
            SettingRow::TextSize => Control::Stepper {
                value: format!("{} pt", trim_number(settings.new_document_size)).into(),
                index: nearest_index(&TEXT_SIZES, settings.new_document_size),
                count: TEXT_SIZES.len(),
            },
            SettingRow::LineHeight => Control::Stepper {
                value: trim_number(settings.new_document_line_height).into(),
                index: nearest_index(&LINE_HEIGHTS, settings.new_document_line_height),
                count: LINE_HEIGHTS.len(),
            },
            SettingRow::PageView => Control::Switch(settings.new_document_page_view),
            SettingRow::Paper => Control::Segments {
                options: Paper::ALL.iter().map(|paper| paper.label().into()).collect(),
                selected: Paper::ALL
                    .iter()
                    .position(|paper| *paper == settings.new_document_paper)
                    .unwrap_or(0),
            },
            SettingRow::CharacterLimit => {
                let options = Self::limit_options();
                let index = options
                    .iter()
                    .position(|option| *option == settings.character_limit)
                    .unwrap_or(0);
                Control::Stepper {
                    value: match settings.character_limit {
                        None => "Off".into(),
                        Some(network) => {
                            format!("{} ({})", network.name(), network.limit()).into()
                        }
                    },
                    index,
                    count: options.len(),
                }
            }
        }
    }

    /// Sets the value of `row` to its option number `index` (switches: 0 is
    /// off, 1 is on). Out-of-range indices are ignored.
    pub fn choose(&mut self, row: SettingRow, index: usize, cx: &mut Context<Self>) {
        match row {
            SettingRow::Theme => {
                let Some(mode) = [ThemeMode::System, ThemeMode::Light, ThemeMode::Dark].get(index)
                else {
                    return;
                };
                self.write(keys::THEME, mode.as_str(), cx);
            }
            SettingRow::VimMode => self.write(keys::VIM_MODE, &(index != 0), cx),
            SettingRow::MarkdownShortcuts => self.write(keys::INPUT_RULES, &(index != 0), cx),
            SettingRow::SmartPunctuation => {
                self.write(keys::SMART_PUNCTUATION, &(index != 0), cx)
            }
            SettingRow::Author => {}
            SettingRow::Font => {
                if let Some(font) = self.fonts.get(index).cloned() {
                    self.write(keys::NEW_DOCUMENT_FONT, &font, cx);
                }
            }
            SettingRow::TextSize => {
                if let Some(size) = TEXT_SIZES.get(index) {
                    self.write(keys::NEW_DOCUMENT_SIZE, size, cx);
                }
            }
            SettingRow::LineHeight => {
                if let Some(height) = LINE_HEIGHTS.get(index) {
                    self.write(keys::NEW_DOCUMENT_LINE_HEIGHT, height, cx);
                }
            }
            SettingRow::PageView => self.write(keys::NEW_DOCUMENT_PAGE_VIEW, &(index != 0), cx),
            SettingRow::Paper => {
                if let Some(paper) = Paper::ALL.get(index) {
                    self.write(keys::NEW_DOCUMENT_PAPER, paper.as_str(), cx);
                }
            }
            SettingRow::CharacterLimit => {
                if let Some(option) = Self::limit_options().get(index) {
                    self.write(keys::CHARACTER_LIMIT, character_limit_name(*option), cx);
                }
            }
        }
    }

    /// Moves the value of `row` by `delta` options. Switches turn on with a
    /// positive delta and off with a negative one; choices stop at the ends.
    pub fn step(&mut self, row: SettingRow, delta: isize, cx: &mut Context<Self>) {
        match self.control(row) {
            Control::Switch(_) => self.choose(row, usize::from(delta > 0), cx),
            Control::Segments { options, selected } => {
                let target = selected as isize + delta;
                if (0..options.len() as isize).contains(&target) {
                    self.choose(row, target as usize, cx);
                }
            }
            Control::Stepper { index, count, .. } => {
                let target = index as isize + delta;
                if (0..count as isize).contains(&target) {
                    self.choose(row, target as usize, cx);
                }
            }
            Control::Text => {}
        }
    }

    /// `space` / `enter`: flips a switch, steps a choice forward and wraps
    /// around at the end.
    pub fn activate(&mut self, row: SettingRow, cx: &mut Context<Self>) {
        match self.control(row) {
            Control::Switch(on) => self.choose(row, usize::from(!on), cx),
            Control::Segments { options, selected } => {
                self.choose(row, (selected + 1) % options.len().max(1), cx)
            }
            Control::Stepper { index, count, .. } => {
                self.choose(row, (index + 1) % count.max(1), cx)
            }
            Control::Text => {}
        }
    }

    fn move_focus(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let count = self.handles.len() as isize;
        let current = self
            .handles
            .iter()
            .position(|handle| handle.contains_focused(window, cx))
            .map(|ix| ix as isize)
            .unwrap_or(-1);
        let target = (current + delta).clamp(0, count - 1) as usize;
        window.focus(&self.handles[target]);
        self.scroll.scroll_to_item(self.child_index(target));
        cx.notify();
    }

    /// The index of row `row_ix` among the scrolled children (section
    /// headings are children too).
    fn child_index(&self, row_ix: usize) -> usize {
        let mut child = 0;
        let mut section = "";
        for (ix, row) in SettingRow::ALL.iter().enumerate() {
            if row.section() != section {
                section = row.section();
                child += 1;
            }
            if ix == row_ix {
                // Reveal the section heading together with its first row.
                let first_of_section = ix == 0 || SettingRow::ALL[ix - 1].section() != section;
                return if first_of_section { child - 1 } else { child };
            }
            child += 1;
        }
        child
    }

    fn on_focus_next(&mut self, _: &FocusNext, window: &mut Window, cx: &mut Context<Self>) {
        self.move_focus(1, window, cx);
    }

    fn on_focus_previous(
        &mut self,
        _: &FocusPrevious,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_focus(-1, window, cx);
    }

    fn render_control(
        &self,
        row: SettingRow,
        control: Control,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let theme = cx.theme();
        let colors = theme.colors;
        let dark = theme.is_dark();
        let popover_shadow = theme.shadow_popover();
        match control {
            Control::Switch(on) => div()
                .flex_none()
                .w(px(30.))
                .h(px(18.))
                .p(px(2.))
                .rounded_full()
                .bg(if on { colors.accent } else { colors.border_strong })
                .flex()
                .when(on, |el| el.justify_end())
                .child(
                    div()
                        .size(px(14.))
                        .rounded_full()
                        .bg(if dark { colors.ink } else { colors.page })
                        .shadow(popover_shadow),
                )
                .into_any_element(),
            Control::Segments { options, selected } => div()
                .flex()
                .flex_none()
                .p(px(2.))
                .gap(px(2.))
                .rounded(RADIUS_MD)
                .bg(colors.hover)
                .border_1()
                .border_color(colors.hairline)
                .children(options.into_iter().enumerate().map(|(ix, label)| {
                    let is_selected = ix == selected;
                    div()
                        .id(("segment", ix))
                        .px(px(10.))
                        .h(px(22.))
                        .flex()
                        .items_center()
                        .rounded(RADIUS_SM)
                        .text_size(UI_TEXT_SM)
                        .whitespace_nowrap()
                        .cursor_pointer()
                        .map(|el| {
                            if is_selected {
                                el.bg(if dark { colors.border_strong } else { colors.page })
                                    .text_color(colors.ink)
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .shadow(popover_shadow.clone())
                            } else {
                                el.text_color(colors.muted)
                                    .hover(|style| style.text_color(colors.ink))
                            }
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.choose(row, ix, cx);
                        }))
                        .child(label)
                }))
                .into_any_element(),
            Control::Stepper {
                value,
                index,
                count,
            } => {
                let arrow = |id: &'static str, glyph: &'static str, enabled: bool, delta: isize| {
                    div()
                        .id(id)
                        .size(px(22.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(RADIUS_SM)
                        .text_size(px(16.))
                        .line_height(px(18.))
                        .text_color(if enabled { colors.muted } else { colors.faint })
                        .when(enabled, |el| {
                            el.cursor_pointer()
                                .hover(|style| style.bg(colors.hover).text_color(colors.ink))
                        })
                        .child(glyph)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.step(row, delta, cx);
                        }))
                };
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(SPACE_1)
                    .child(arrow("previous", "‹", index > 0, -1))
                    .child(
                        div()
                            .min_w(px(132.))
                            .h(px(26.))
                            .px(SPACE_2)
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(RADIUS_MD)
                            .border_1()
                            .border_color(colors.hairline)
                            .bg(colors.hover)
                            .text_size(UI_TEXT_SM)
                            .whitespace_nowrap()
                            .child(value),
                    )
                    .child(arrow("next", "›", index + 1 < count, 1))
                    .into_any_element()
            }
            Control::Text => div()
                .w(px(220.))
                .flex_none()
                .child(self.author_field.clone())
                .into_any_element(),
        }
    }

    fn render_row(
        &self,
        ix: usize,
        row: SettingRow,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let colors = cx.theme().colors;
        let control = self.control(row);
        let is_text = control == Control::Text;
        let handle = self.handles[ix].clone();
        let focused = handle.contains_focused(window, cx);
        let ring = focus_ring(&colors);
        let control_element = self.render_control(row, control, cx);

        div()
            .id(("setting", ix))
            .flex()
            .items_center()
            .gap(SPACE_4)
            .min_h(px(46.))
            .py(px(6.))
            .px(SPACE_3)
            .rounded(RADIUS_MD)
            .when(focused && !is_text, |el| el.bg(colors.hover).shadow(ring))
            .when(focused && is_text, |el| el.bg(colors.hover))
            .map(|el| {
                if is_text {
                    // The text field is the focus stop of this row.
                    el
                } else {
                    el.key_context(ROW_KEY_CONTEXT)
                        .track_focus(&handle)
                        .cursor_pointer()
                        .on_action(cx.listener(move |this, _: &Increase, _, cx| {
                            this.step(row, 1, cx)
                        }))
                        .on_action(cx.listener(move |this, _: &Decrease, _, cx| {
                            this.step(row, -1, cx)
                        }))
                        .on_action(cx.listener(move |this, _: &Activate, _, cx| {
                            this.activate(row, cx)
                        }))
                        .on_click(cx.listener(move |this, _, _, cx| this.activate(row, cx)))
                }
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .text_size(UI_TEXT_BASE)
                            .line_height(px(18.))
                            .text_color(colors.ink)
                            .child(row.label()),
                    )
                    .child(
                        div()
                            .text_size(UI_TEXT_SM)
                            .line_height(px(16.))
                            .text_color(colors.muted)
                            .child(row.description()),
                    ),
            )
            .child(control_element)
            .into_any_element()
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let colors = theme.colors;
        let ui_font = theme.fonts.ui.clone();
        let overlay_shadow = theme.shadow_overlay();
        // Leave room for the modal's top offset and a margin below.
        let list_height = (f32::from(window.viewport_size().height) * 0.80 - 150.).max(160.);

        let mut children: Vec<gpui::AnyElement> = Vec::new();
        let mut section = "";
        for (ix, row) in SettingRow::ALL.into_iter().enumerate() {
            if row.section() != section {
                section = row.section();
                children.push(
                    div()
                        .px(SPACE_3)
                        .pt(if ix == 0 { px(4.) } else { px(14.) })
                        .pb(px(4.))
                        .text_size(UI_TEXT_XS)
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.muted)
                        .child(section)
                        .into_any_element(),
                );
            }
            children.push(self.render_row(ix, row, window, cx));
        }

        let hint = |keys: &'static str, what: &'static str| {
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(div().text_color(colors.ink).child(keys))
                .child(what)
        };

        div()
            .id("settings")
            .key_context(KEY_CONTEXT)
            .on_action(cx.listener(Self::on_focus_next))
            .on_action(cx.listener(Self::on_focus_previous))
            // Clicks on the panel's own chrome must not close the modal or
            // steal focus from the focused row.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .w(px(580.))
            .max_w_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(colors.elevated)
            .border_1()
            .border_color(colors.hairline)
            .rounded(RADIUS_LG)
            .shadow(overlay_shadow)
            .font_family(ui_font)
            .text_size(UI_TEXT_BASE)
            .text_color(colors.ink)
            .child(
                div()
                    .flex_none()
                    .h(px(48.))
                    .px(SPACE_4)
                    .flex()
                    .items_center()
                    .gap(SPACE_2)
                    .border_b_1()
                    .border_color(colors.hairline)
                    .child(Icon::new(IconName::Settings).color(colors.muted))
                    .child(div().text_size(UI_TEXT_LG).child("Settings")),
            )
            .child(
                div()
                    .id("settings-list")
                    .max_h(px(list_height))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .p(SPACE_2)
                    .flex()
                    .flex_col()
                    .children(children),
            )
            .child(
                div()
                    .flex_none()
                    .h(px(34.))
                    .px(SPACE_4)
                    .flex()
                    .items_center()
                    .gap(SPACE_4)
                    .border_t_1()
                    .border_color(colors.hairline)
                    .text_size(UI_TEXT_XS)
                    .text_color(colors.muted)
                    .child(hint("↑ ↓", "Move"))
                    .child(hint("← →", "Change"))
                    .child(hint("Space", "Switch"))
                    .child(hint("Esc", "Close")),
            )
    }
}
