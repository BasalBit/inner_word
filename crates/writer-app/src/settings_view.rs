//! The settings modal (`cmd-,`).
//!
//! One list of rows, each a focus stop. `up` / `down` (or `tab` /
//! `shift-tab`) move between rows, `left` / `right` change a choice,
//! `space` or `enter` flips a switch or steps a choice forward, and
//! `escape` closes. A row with a text field is typed into. The mouse works
//! on every control too.
//!
//! The app's settings are written to the library at once; the workspace
//! applies them live (see [`crate::settings`]).
//!
//! # This document
//!
//! With a document open the list ends with a section for that document
//! alone: page view, paper (A4, US Letter or a custom size), orientation,
//! the four margins, the body font, text size, line height and paragraph
//! spacing. These are part of the document ([`writer_core::DocumentSettings`]):
//! they are changed through the editor, saved with the text and exported
//! with it. Lengths are typed in millimetres, or in inches on US Letter;
//! another unit can be named (`20mm`, `0.75in`, `2cm`, `36pt`). The palette
//! command "Document settings" opens the modal at this section.

use std::collections::HashMap;

use gpui::{
    App, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, KeyBinding,
    MouseButton, ScrollHandle, SharedString, Subscription, Window, actions, div, prelude::*, px,
};
use serde::Serialize;
use writer_core::{DocumentSettings, Orientation, PaperSize};
use writer_editor::{Autoscroll, Editor, EditorEvent};
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
    self, LINE_HEIGHTS, LengthUnit, Paper, Settings, TEXT_SIZES, character_limit_name,
    format_length, keys, parse_length,
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

/// The smallest and largest body text size of a document, in points, that
/// the settings step through.
pub const DOCUMENT_TEXT_SIZES: (f32, f32) = (9., 36.);
/// The smallest and largest line height of a document.
pub const DOCUMENT_LINE_HEIGHTS: (f32, f32) = (1.0, 3.0);
/// The largest paragraph spacing of a document, in points.
pub const DOCUMENT_PARAGRAPH_SPACING_MAX: f32 = 48.;

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
    /// This document: page view or the flowing column.
    DocumentPageView,
    /// This document: A4, US Letter or a custom size.
    DocumentPaper,
    /// This document: the width of a custom sheet.
    DocumentPaperWidth,
    /// This document: the height of a custom sheet.
    DocumentPaperHeight,
    /// This document: portrait or landscape.
    DocumentOrientation,
    /// This document: the top margin.
    DocumentMarginTop,
    /// This document: the bottom margin.
    DocumentMarginBottom,
    /// This document: the left margin.
    DocumentMarginLeft,
    /// This document: the right margin.
    DocumentMarginRight,
    /// This document: the body font.
    DocumentFont,
    /// This document: the body text size.
    DocumentTextSize,
    /// This document: the line height.
    DocumentLineHeight,
    /// This document: the space after a paragraph.
    DocumentParagraphSpacing,
}

impl SettingRow {
    /// The rows about the app, in order. They are always listed.
    pub const APP: [SettingRow; 11] = [
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

    /// The rows about the open document, in order. Listed while a document
    /// is open; the two sizes of a custom sheet only while the paper is
    /// custom.
    pub const DOCUMENT: [SettingRow; 13] = [
        SettingRow::DocumentPageView,
        SettingRow::DocumentPaper,
        SettingRow::DocumentPaperWidth,
        SettingRow::DocumentPaperHeight,
        SettingRow::DocumentOrientation,
        SettingRow::DocumentMarginTop,
        SettingRow::DocumentMarginBottom,
        SettingRow::DocumentMarginLeft,
        SettingRow::DocumentMarginRight,
        SettingRow::DocumentFont,
        SettingRow::DocumentTextSize,
        SettingRow::DocumentLineHeight,
        SettingRow::DocumentParagraphSpacing,
    ];

    /// The rows that are typed into as a length.
    const LENGTHS: [SettingRow; 6] = [
        SettingRow::DocumentPaperWidth,
        SettingRow::DocumentPaperHeight,
        SettingRow::DocumentMarginTop,
        SettingRow::DocumentMarginBottom,
        SettingRow::DocumentMarginLeft,
        SettingRow::DocumentMarginRight,
    ];

    /// Whether the row belongs to the open document rather than the app.
    pub fn is_document(self) -> bool {
        Self::DOCUMENT.contains(&self)
    }

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
            _ => "This document",
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
            SettingRow::DocumentPageView => "Page view",
            SettingRow::DocumentPaper => "Paper size",
            SettingRow::DocumentPaperWidth => "Paper width",
            SettingRow::DocumentPaperHeight => "Paper height",
            SettingRow::DocumentOrientation => "Orientation",
            SettingRow::DocumentMarginTop => "Top margin",
            SettingRow::DocumentMarginBottom => "Bottom margin",
            SettingRow::DocumentMarginLeft => "Left margin",
            SettingRow::DocumentMarginRight => "Right margin",
            SettingRow::DocumentFont => "Font",
            SettingRow::DocumentTextSize => "Text size",
            SettingRow::DocumentLineHeight => "Line height",
            SettingRow::DocumentParagraphSpacing => "Paragraph spacing",
        }
    }

    fn description(self) -> &'static str {
        match self {
            SettingRow::Theme => "Paper is light, Night is dark.",
            SettingRow::VimMode => "Modal editing with Vim keys.",
            SettingRow::MarkdownShortcuts => "Typing # , - , > or **bold** formats as you go.",
            SettingRow::SmartPunctuation => "Curly quotes, dashes and ellipses.",
            SettingRow::Author => "Written on the comments you add.",
            SettingRow::Font => "Body text of a new document.",
            SettingRow::TextSize => "In points.",
            SettingRow::LineHeight => "As a multiple of the text size.",
            SettingRow::PageView => "Show sheets of paper instead of one column.",
            SettingRow::Paper => "Used by page view and Word export.",
            SettingRow::CharacterLimit => "Count the text against a social network's post limit.",
            SettingRow::DocumentPageView => "Show sheets of paper instead of one column.",
            SettingRow::DocumentPaper => "Used by page view and Word export.",
            SettingRow::DocumentPaperWidth | SettingRow::DocumentPaperHeight => {
                "Of the upright sheet."
            }
            SettingRow::DocumentOrientation => "Upright or turned on its side.",
            SettingRow::DocumentMarginTop
            | SettingRow::DocumentMarginBottom
            | SettingRow::DocumentMarginLeft
            | SettingRow::DocumentMarginRight => "",
            SettingRow::DocumentFont => "Body text of this document.",
            SettingRow::DocumentTextSize => "Body text, in points. Headings scale with it.",
            SettingRow::DocumentLineHeight => "As a multiple of the text size.",
            SettingRow::DocumentParagraphSpacing => "Space after each paragraph, in points.",
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
    /// A text field: the author's name, or a length with its unit.
    Field {
        unit: Option<&'static str>,
    },
}

/// The settings modal. See the [module docs](self).
pub struct SettingsView {
    library: Entity<Library>,
    settings: Settings,
    /// The editor whose document the "This document" rows change. `None`
    /// while no document is open.
    document: Option<Entity<Editor>>,
    fonts: Vec<String>,
    document_fonts: Vec<String>,
    handles: HashMap<SettingRow, FocusHandle>,
    author_field: Entity<TextField>,
    length_fields: HashMap<SettingRow, Entity<TextField>>,
    /// The length row whose field is being typed into: its text is left
    /// alone until the keyboard moves on.
    editing: Option<SettingRow>,
    /// The row the keyboard starts on.
    start: SettingRow,
    scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DismissEvent> for SettingsView {}

impl Focusable for SettingsView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.handles[&self.start].clone()
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

/// The position of `value` on a scale from `low` in steps of `step`.
fn step_index(value: f32, low: f32, step: f32) -> usize {
    ((value - low) / step).round().max(0.) as usize
}

impl SettingsView {
    /// Builds the modal over `library`, with the app's settings only.
    pub fn new(library: Entity<Library>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::build(library, None, window, cx)
    }

    /// Builds the modal with the "This document" section for the document
    /// open in `editor`.
    pub fn for_document(
        library: Entity<Library>,
        editor: Entity<Editor>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::build(library, Some(editor), window, cx)
    }

    /// Starts with the keyboard on `row` instead of the first one (ignored
    /// when the row is not listed).
    pub fn starting_at(mut self, row: SettingRow, cx: &mut Context<Self>) -> Self {
        let rows = self.rows(cx);
        if let Some(ix) = rows.iter().position(|listed| *listed == row) {
            self.start = row;
            self.scroll.scroll_to_item(self.child_index(&rows, ix));
        }
        self
    }

    fn build(
        library: Entity<Library>,
        document: Option<Entity<Editor>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings = Settings::load(library.read(cx));
        let fonts = settings::available_fonts(&settings.new_document_font, cx);
        let document_settings = document
            .as_ref()
            .map(|editor| editor.read(cx).document().settings().clone());
        let document_fonts = document_settings
            .as_ref()
            .map(|settings| settings::available_fonts(&settings.typography.body_font, cx))
            .unwrap_or_default();

        let author_field = cx.new(|cx| {
            let mut field = TextField::new(window, cx)
                .with_placeholder("Your name")
                .with_text(settings.comment_author.clone())
                .with_appearance(FieldAppearance::Bordered);
            // up and down leave the field for the neighbouring rows.
            field.set_list_navigation(true);
            field
        });
        let mut length_fields = HashMap::new();
        if let Some(document_settings) = &document_settings {
            for row in SettingRow::LENGTHS {
                let text = length_text(row, document_settings);
                let field = cx.new(|cx| {
                    let mut field = TextField::new(window, cx)
                        .with_text(text)
                        .with_appearance(FieldAppearance::Bordered);
                    field.set_list_navigation(true);
                    field
                });
                length_fields.insert(row, field);
            }
        }

        let mut handles = HashMap::new();
        for row in SettingRow::APP.into_iter().chain(SettingRow::DOCUMENT) {
            let handle = match row {
                SettingRow::Author => author_field.focus_handle(cx),
                _ => match length_fields.get(&row) {
                    Some(field) => field.focus_handle(cx),
                    None => cx.focus_handle(),
                },
            };
            handles.insert(row, handle);
        }

        let mut subscriptions = vec![
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
        for (row, field) in &length_fields {
            let row = *row;
            subscriptions.push(cx.subscribe_in(
                field,
                window,
                move |this: &mut Self, field, event: &TextFieldEvent, window, cx| match event {
                    TextFieldEvent::Focused => this.editing = Some(row),
                    TextFieldEvent::Changed => {
                        this.editing = Some(row);
                        let text = field.read(cx).text().to_string();
                        this.length_typed(row, &text, cx);
                    }
                    TextFieldEvent::Blurred => {
                        if this.editing == Some(row) {
                            this.editing = None;
                        }
                        this.refresh_length_fields(cx);
                    }
                    TextFieldEvent::Submitted => this.move_focus(1, window, cx),
                    _ => {}
                },
            ));
        }
        if let Some(editor) = &document {
            // The document can change behind the modal (a shortcut such as
            // cmd-alt-p reaches the workspace): show what it is now.
            subscriptions.push(cx.subscribe(
                editor,
                |this: &mut Self, _, event: &EditorEvent, cx| {
                    if matches!(
                        event,
                        EditorEvent::SettingsChanged | EditorEvent::DocumentReplaced
                    ) {
                        this.refresh_length_fields(cx);
                        cx.notify();
                    }
                },
            ));
        }

        Self {
            library,
            settings,
            document,
            fonts,
            document_fonts,
            handles,
            author_field,
            length_fields,
            editing: None,
            start: SettingRow::Theme,
            scroll: ScrollHandle::new(),
            _subscriptions: subscriptions,
        }
    }

    /// The settings as this view last read them.
    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// The settings of the open document, when the modal has its section.
    pub fn document_settings(&self, cx: &App) -> Option<DocumentSettings> {
        self.document
            .as_ref()
            .map(|editor| editor.read(cx).document().settings().clone())
    }

    /// The rows that are listed right now, in order.
    pub fn rows(&self, cx: &App) -> Vec<SettingRow> {
        let mut rows = SettingRow::APP.to_vec();
        if let Some(document) = self.document_settings(cx) {
            let custom = matches!(document.page.paper, PaperSize::Custom { .. });
            rows.extend(SettingRow::DOCUMENT.into_iter().filter(|row| {
                custom
                    || !matches!(
                        row,
                        SettingRow::DocumentPaperWidth | SettingRow::DocumentPaperHeight
                    )
            }));
        }
        rows
    }

    /// The row that has focus.
    pub fn focused_row(&self, window: &Window, cx: &App) -> Option<SettingRow> {
        self.rows(cx)
            .into_iter()
            .find(|row| self.handles[row].contains_focused(window, cx))
    }

    /// The text in the field of a length row (a paper size or a margin).
    pub fn length_field_text(&self, row: SettingRow, cx: &App) -> Option<String> {
        self.length_fields
            .get(&row)
            .map(|field| field.read(cx).text().to_string())
    }

    /// What the row says under its name. A row that is typed into explains
    /// its units while the keyboard is in it (`focused`).
    pub fn description(&self, row: SettingRow, focused: bool, cx: &App) -> SharedString {
        match row {
            SettingRow::VimMode => {
                // The accent menu on held keys follows this setting, but the
                // system only looks at launch (see `press_and_hold`).
                let state = crate::press_and_hold::state(cx);
                if !state.restart_note_due() {
                    "Modal editing with Vim keys. Held keys repeat instead of offering accents."
                        .into()
                } else if state.repeats_next() == Some(true) {
                    "Modal editing with Vim keys. Restart The Writer to make held keys repeat \
                     while you type, too."
                        .into()
                } else {
                    "Modal editing with Vim keys. Restart The Writer to get the accent menu on \
                     held keys back."
                        .into()
                }
            }
            row if SettingRow::LENGTHS.contains(&row) && focused => {
                let unit = self
                    .document_settings(cx)
                    .map(|settings| LengthUnit::for_paper(settings.page.paper))
                    .unwrap_or(LengthUnit::Millimetres);
                format!(
                    "In {}. Another unit can be named: 20mm, 2cm, 0.75in, 36pt.",
                    unit.name()
                )
                .into()
            }
            _ => row.description().into(),
        }
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

    /// Changes the settings of the open document through the editor, which
    /// reports the change so that it is saved with the text.
    fn change_document(
        &mut self,
        change: impl FnOnce(&mut DocumentSettings),
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self.document.clone() else {
            return;
        };
        if editor.read(cx).is_read_only() {
            return;
        }
        let before = editor.read(cx).document().settings().clone();
        let mut settings = before.clone();
        change(&mut settings);
        let settings = settings.sanitized();
        if settings == before {
            return;
        }
        editor.update(cx, |editor, cx| {
            editor.edit_with(Autoscroll::None, |state| state.set_settings(settings), cx);
        });
        self.refresh_length_fields(cx);
        cx.notify();
    }

    /// A length was typed into the field of `row`: apply it when it reads
    /// as one.
    fn length_typed(&mut self, row: SettingRow, text: &str, cx: &mut Context<Self>) {
        let Some(document) = self.document_settings(cx) else {
            return;
        };
        let unit = LengthUnit::for_paper(document.page.paper);
        let Some(points) = parse_length(text, unit) else {
            return;
        };
        self.change_document(
            |settings| match row {
                SettingRow::DocumentPaperWidth | SettingRow::DocumentPaperHeight => {
                    if let PaperSize::Custom { width, height } = &mut settings.page.paper {
                        if row == SettingRow::DocumentPaperWidth {
                            *width = points;
                        } else {
                            *height = points;
                        }
                    }
                }
                SettingRow::DocumentMarginTop => settings.page.margins.top = points,
                SettingRow::DocumentMarginBottom => settings.page.margins.bottom = points,
                SettingRow::DocumentMarginLeft => settings.page.margins.left = points,
                SettingRow::DocumentMarginRight => settings.page.margins.right = points,
                _ => {}
            },
            cx,
        );
    }

    /// Shows the document's values in the length fields again, except in
    /// the one that is being typed into.
    fn refresh_length_fields(&mut self, cx: &mut Context<Self>) {
        let Some(document) = self.document_settings(cx) else {
            return;
        };
        for (row, field) in &self.length_fields {
            if self.editing == Some(*row) {
                continue;
            }
            let text = length_text(*row, &document);
            if field.read(cx).text() != text {
                field.update(cx, |field, cx| field.set_text(text, cx));
            }
        }
    }

    fn limit_options() -> Vec<Option<SocialNetwork>> {
        std::iter::once(None)
            .chain(SocialNetwork::ALL.into_iter().map(Some))
            .collect()
    }

    fn control(&self, row: SettingRow, cx: &App) -> Control {
        let settings = &self.settings;
        let document = self.document_settings(cx).unwrap_or_default();
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
            SettingRow::Author => Control::Field { unit: None },
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
                options: Paper::ALL
                    .iter()
                    .map(|paper| paper.label().into())
                    .collect(),
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
                        Some(network) => format!("{} ({})", network.name(), network.limit()).into(),
                    },
                    index,
                    count: options.len(),
                }
            }
            SettingRow::DocumentPageView => Control::Switch(document.page_view),
            SettingRow::DocumentPaper => Control::Segments {
                options: vec!["A4".into(), "US Letter".into(), "Custom".into()],
                selected: match document.page.paper {
                    PaperSize::A4 => 0,
                    PaperSize::UsLetter => 1,
                    PaperSize::Custom { .. } => 2,
                },
            },
            SettingRow::DocumentOrientation => Control::Segments {
                options: vec!["Portrait".into(), "Landscape".into()],
                selected: match document.page.orientation {
                    Orientation::Portrait => 0,
                    Orientation::Landscape => 1,
                },
            },
            SettingRow::DocumentPaperWidth
            | SettingRow::DocumentPaperHeight
            | SettingRow::DocumentMarginTop
            | SettingRow::DocumentMarginBottom
            | SettingRow::DocumentMarginLeft
            | SettingRow::DocumentMarginRight => Control::Field {
                unit: Some(LengthUnit::for_paper(document.page.paper).abbreviation()),
            },
            SettingRow::DocumentFont => {
                let font = &document.typography.body_font;
                Control::Stepper {
                    value: font.clone().into(),
                    index: self
                        .document_fonts
                        .iter()
                        .position(|candidate| candidate == font)
                        .unwrap_or(0),
                    count: self.document_fonts.len().max(1),
                }
            }
            SettingRow::DocumentTextSize => {
                let (low, high) = DOCUMENT_TEXT_SIZES;
                let size = document.typography.base_size;
                Control::Stepper {
                    value: format!("{} pt", trim_number(size)).into(),
                    index: step_index(size.clamp(low, high), low, 1.),
                    count: step_index(high, low, 1.) + 1,
                }
            }
            SettingRow::DocumentLineHeight => {
                let (low, high) = DOCUMENT_LINE_HEIGHTS;
                let height = document.typography.line_height;
                Control::Stepper {
                    value: format!("{height:.1}").into(),
                    index: step_index(height.clamp(low, high), low, 0.1),
                    count: step_index(high, low, 0.1) + 1,
                }
            }
            SettingRow::DocumentParagraphSpacing => {
                let spacing = document.typography.paragraph_spacing;
                Control::Stepper {
                    value: format!("{} pt", trim_number(spacing)).into(),
                    index: step_index(spacing.clamp(0., DOCUMENT_PARAGRAPH_SPACING_MAX), 0., 1.),
                    count: step_index(DOCUMENT_PARAGRAPH_SPACING_MAX, 0., 1.) + 1,
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
            SettingRow::SmartPunctuation => self.write(keys::SMART_PUNCTUATION, &(index != 0), cx),
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
            SettingRow::DocumentPageView => {
                self.change_document(|settings| settings.page_view = index != 0, cx)
            }
            SettingRow::DocumentPaper => self.change_document(
                |settings| {
                    settings.page.paper = match index {
                        0 => PaperSize::A4,
                        1 => PaperSize::UsLetter,
                        2 => match settings.page.paper {
                            custom @ PaperSize::Custom { .. } => custom,
                            // A custom size starts from the sheet it replaces.
                            standard => {
                                let (width, height) = standard.portrait_size();
                                PaperSize::Custom { width, height }
                            }
                        },
                        _ => settings.page.paper,
                    }
                },
                cx,
            ),
            SettingRow::DocumentOrientation => self.change_document(
                |settings| {
                    settings.page.orientation = match index {
                        0 => Orientation::Portrait,
                        1 => Orientation::Landscape,
                        _ => settings.page.orientation,
                    }
                },
                cx,
            ),
            SettingRow::DocumentFont => {
                if let Some(font) = self.document_fonts.get(index).cloned() {
                    self.change_document(|settings| settings.typography.body_font = font, cx);
                }
            }
            SettingRow::DocumentTextSize => {
                let (low, high) = DOCUMENT_TEXT_SIZES;
                let size = low + index as f32;
                if size <= high {
                    self.change_document(|settings| settings.typography.base_size = size, cx);
                }
            }
            SettingRow::DocumentLineHeight => {
                let (low, high) = DOCUMENT_LINE_HEIGHTS;
                let height = ((low + index as f32 * 0.1) * 10.).round() / 10.;
                if height <= high {
                    self.change_document(|settings| settings.typography.line_height = height, cx);
                }
            }
            SettingRow::DocumentParagraphSpacing => {
                let spacing = index as f32;
                if spacing <= DOCUMENT_PARAGRAPH_SPACING_MAX {
                    self.change_document(
                        |settings| settings.typography.paragraph_spacing = spacing,
                        cx,
                    );
                }
            }
            // Typed, not chosen.
            SettingRow::DocumentPaperWidth
            | SettingRow::DocumentPaperHeight
            | SettingRow::DocumentMarginTop
            | SettingRow::DocumentMarginBottom
            | SettingRow::DocumentMarginLeft
            | SettingRow::DocumentMarginRight => {}
        }
    }

    /// Moves the value of `row` by `delta` options. Switches turn on with a
    /// positive delta and off with a negative one; choices stop at the ends.
    pub fn step(&mut self, row: SettingRow, delta: isize, cx: &mut Context<Self>) {
        match self.control(row, cx) {
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
            Control::Field { .. } => {}
        }
    }

    /// `space` / `enter`: flips a switch, steps a choice forward and wraps
    /// around at the end.
    pub fn activate(&mut self, row: SettingRow, cx: &mut Context<Self>) {
        match self.control(row, cx) {
            Control::Switch(on) => self.choose(row, usize::from(!on), cx),
            Control::Segments { options, selected } => {
                self.choose(row, (selected + 1) % options.len().max(1), cx)
            }
            Control::Stepper { index, count, .. } => {
                self.choose(row, (index + 1) % count.max(1), cx)
            }
            Control::Field { .. } => {}
        }
    }

    fn move_focus(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let rows = self.rows(cx);
        let count = rows.len() as isize;
        let current = rows
            .iter()
            .position(|row| self.handles[row].contains_focused(window, cx))
            .map(|ix| ix as isize)
            .unwrap_or(-1);
        let target = (current + delta).clamp(0, count - 1) as usize;
        window.focus(&self.handles[&rows[target]]);
        self.scroll.scroll_to_item(self.child_index(&rows, target));
        cx.notify();
    }

    /// The index of row `row_ix` of `rows` among the scrolled children
    /// (section headings are children too).
    fn child_index(&self, rows: &[SettingRow], row_ix: usize) -> usize {
        let mut child = 0;
        let mut section = "";
        for (ix, row) in rows.iter().enumerate() {
            if row.section() != section {
                section = row.section();
                child += 1;
            }
            if ix == row_ix {
                // Reveal the section heading together with its first row.
                let first_of_section = ix == 0 || rows[ix - 1].section() != section;
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
                .bg(if on {
                    colors.accent
                } else {
                    colors.border_strong
                })
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
                                el.bg(if dark {
                                    colors.border_strong
                                } else {
                                    colors.page
                                })
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
            Control::Field { unit: None } => div()
                .w(px(220.))
                .flex_none()
                .child(self.author_field.clone())
                .into_any_element(),
            Control::Field { unit: Some(unit) } => div()
                .flex()
                .flex_none()
                .items_center()
                .gap(SPACE_2)
                .child(
                    div()
                        .w(px(96.))
                        .children(self.length_fields.get(&row).cloned()),
                )
                .child(
                    div()
                        .w(px(24.))
                        .text_size(UI_TEXT_SM)
                        .text_color(colors.muted)
                        .child(unit),
                )
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
        let control = self.control(row, cx);
        let is_field = matches!(control, Control::Field { .. });
        let handle = self.handles[&row].clone();
        let focused = handle.contains_focused(window, cx);
        let ring = focus_ring(&colors);
        let description = self.description(row, focused, cx);
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
            .when(focused && !is_field, |el| el.bg(colors.hover).shadow(ring))
            .when(focused && is_field, |el| el.bg(colors.hover))
            .map(|el| {
                if is_field {
                    // The text field is the focus stop of this row.
                    el
                } else {
                    el.key_context(ROW_KEY_CONTEXT)
                        .track_focus(&handle)
                        .cursor_pointer()
                        .on_action(
                            cx.listener(move |this, _: &Increase, _, cx| this.step(row, 1, cx)),
                        )
                        .on_action(
                            cx.listener(move |this, _: &Decrease, _, cx| this.step(row, -1, cx)),
                        )
                        .on_action(
                            cx.listener(move |this, _: &Activate, _, cx| this.activate(row, cx)),
                        )
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
                    .when(!description.is_empty(), |el| {
                        el.child(
                            div()
                                .text_size(UI_TEXT_SM)
                                .line_height(px(16.))
                                .text_color(colors.muted)
                                .child(description),
                        )
                    }),
            )
            .child(control_element)
            .into_any_element()
    }
}

/// What the field of the length row `row` shows for `document`.
fn length_text(row: SettingRow, document: &DocumentSettings) -> String {
    let unit = LengthUnit::for_paper(document.page.paper);
    let (width, height) = document.page.paper.portrait_size();
    let margins = document.page.margins;
    let points = match row {
        SettingRow::DocumentPaperWidth => width,
        SettingRow::DocumentPaperHeight => height,
        SettingRow::DocumentMarginTop => margins.top,
        SettingRow::DocumentMarginBottom => margins.bottom,
        SettingRow::DocumentMarginLeft => margins.left,
        SettingRow::DocumentMarginRight => margins.right,
        _ => return String::new(),
    };
    format_length(points, unit)
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
        for (ix, row) in self.rows(cx).into_iter().enumerate() {
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
