//! The settings modal (`cmd-,`).
//!
//! One list of rows, each a focus stop. `up` / `down` (or `tab` /
//! `shift-tab`) move between rows, `left` / `right` change a choice,
//! `space` or `enter` flips a switch or steps a choice forward, and
//! `escape` closes. A row with a text field is typed into. The mouse works
//! on every control too.
//!
//! The two spelling-language rows have a long list behind them (every
//! language the Mac can check): `left` / `right` step through it, the
//! user's preferred languages first, and `enter`, `space` or a click on the
//! row opens the searchable list ([`crate::spelling::LanguagePicker`]),
//! which comes back here.
//!
//! The app's settings are written to the library at once; the workspace
//! applies them live (see [`crate::settings`]).
//!
//! # Assistant
//!
//! When the app has an engine for the assistant and the engine is
//! installed on this Mac, the list has a section "Assistant": a switch
//! that says which engine was found (its name, its version, the account)
//! and turns the assistant off, and the model and the effort each of the
//! two chats asks with. Under them stands the note about where the text
//! goes. When the engine is not installed there is no such section, only
//! one quiet line at the end of the app's settings that says what the
//! assistant needs. The engine is looked for again every time the settings
//! are opened (`Workspace::show_settings`), in the background; the section
//! follows when the answer is in.
//!
//! # This document
//!
//! With a document open the list ends with a section for that document
//! alone: page view, paper (A4, US Letter or a custom size), orientation,
//! the four margins, the body font, text size, line height, paragraph
//! spacing and the language its spelling is checked in (its own, or the
//! app's). These are part of the document ([`writer_core::DocumentSettings`]):
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

use crate::{
    settings::{
        self, LINE_HEIGHTS, LengthUnit, Paper, Settings, TEXT_SIZES, character_limit_name,
        format_length, keys, parse_length,
    },
    spelling::{self, LanguageTarget},
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
    /// The format bar over a selection made with the mouse.
    FormatBar,
    /// Misspelled words are marked as they are written.
    CheckSpelling,
    /// The language spelling is checked in.
    SpellingLanguage,
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
    /// The word count in the status bar.
    StatusWords,
    /// The character count in the status bar.
    StatusCharacters,
    /// The reading time in the status bar.
    StatusReadingTime,
    /// The keyboard shortcuts: opens the list they are looked up and
    /// changed in.
    KeyboardShortcuts,
    /// The assistant: on or off. Says which engine was found.
    Assistant,
    /// The assistant: the model of the quick chat.
    AssistantQuickModel,
    /// The assistant: the effort of the quick chat.
    AssistantQuickEffort,
    /// The assistant: the model of the side chat.
    AssistantSideModel,
    /// The assistant: the effort of the side chat.
    AssistantSideEffort,
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
    /// This document: the language its spelling is checked in.
    DocumentSpellingLanguage,
}

impl SettingRow {
    /// The rows about the app, in order. They are always listed.
    pub const APP: [SettingRow; 18] = [
        SettingRow::Theme,
        SettingRow::VimMode,
        SettingRow::MarkdownShortcuts,
        SettingRow::SmartPunctuation,
        SettingRow::FormatBar,
        SettingRow::CheckSpelling,
        SettingRow::SpellingLanguage,
        SettingRow::Author,
        SettingRow::Font,
        SettingRow::TextSize,
        SettingRow::LineHeight,
        SettingRow::PageView,
        SettingRow::Paper,
        SettingRow::CharacterLimit,
        SettingRow::StatusWords,
        SettingRow::StatusCharacters,
        SettingRow::StatusReadingTime,
        SettingRow::KeyboardShortcuts,
    ];

    /// The rows about the assistant, in order. Listed while its engine is
    /// installed; the four choices only while the assistant is on.
    pub const ASSISTANT: [SettingRow; 5] = [
        SettingRow::Assistant,
        SettingRow::AssistantQuickModel,
        SettingRow::AssistantQuickEffort,
        SettingRow::AssistantSideModel,
        SettingRow::AssistantSideEffort,
    ];

    /// The rows about the open document, in order. Listed while a document
    /// is open; the two sizes of a custom sheet only while the paper is
    /// custom.
    pub const DOCUMENT: [SettingRow; 14] = [
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
        SettingRow::DocumentSpellingLanguage,
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
            | SettingRow::FormatBar
            | SettingRow::CheckSpelling
            | SettingRow::SpellingLanguage
            | SettingRow::Author => "Writing",
            SettingRow::Font
            | SettingRow::TextSize
            | SettingRow::LineHeight
            | SettingRow::PageView
            | SettingRow::Paper => "New documents",
            SettingRow::CharacterLimit
            | SettingRow::StatusWords
            | SettingRow::StatusCharacters
            | SettingRow::StatusReadingTime => "Status bar",
            SettingRow::KeyboardShortcuts => "Keyboard",
            SettingRow::Assistant
            | SettingRow::AssistantQuickModel
            | SettingRow::AssistantQuickEffort
            | SettingRow::AssistantSideModel
            | SettingRow::AssistantSideEffort => ASSISTANT_SECTION,
            _ => "This document",
        }
    }

    fn label(self) -> &'static str {
        match self {
            SettingRow::Theme => "Theme",
            SettingRow::VimMode => "Vim mode",
            SettingRow::MarkdownShortcuts => "Markdown shortcuts",
            SettingRow::SmartPunctuation => "Smart punctuation",
            SettingRow::FormatBar => "Format bar",
            SettingRow::CheckSpelling => "Check spelling while typing",
            SettingRow::SpellingLanguage => "Spelling language",
            SettingRow::Author => "Name on comments",
            SettingRow::Font => "Font",
            SettingRow::TextSize => "Text size",
            SettingRow::LineHeight => "Line spacing",
            SettingRow::PageView => "Start in page view",
            SettingRow::Paper => "Paper size",
            SettingRow::CharacterLimit => "Character limit",
            SettingRow::StatusWords => "Word count",
            SettingRow::StatusCharacters => "Character count",
            SettingRow::StatusReadingTime => "Reading time",
            SettingRow::KeyboardShortcuts => "Keyboard shortcuts",
            SettingRow::Assistant => "Assistant",
            SettingRow::AssistantQuickModel => "Quick chat model",
            SettingRow::AssistantQuickEffort => "Quick chat effort",
            SettingRow::AssistantSideModel => "Side chat model",
            SettingRow::AssistantSideEffort => "Side chat effort",
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
            SettingRow::DocumentLineHeight => "Line spacing",
            SettingRow::DocumentParagraphSpacing => "Paragraph spacing",
            SettingRow::DocumentSpellingLanguage => "Spelling language",
        }
    }

    fn description(self) -> &'static str {
        match self {
            SettingRow::Theme => "Paper is light, Night is dark.",
            SettingRow::VimMode => "Modal editing with Vim keys.",
            SettingRow::MarkdownShortcuts => "Typing # , - , > or **bold** formats as you go.",
            SettingRow::SmartPunctuation => "Curly quotes, dashes and ellipses.",
            SettingRow::FormatBar => "Comes up over text selected with the mouse.",
            SettingRow::CheckSpelling => {
                "Marks misspelled words. Right-click a marked word to correct it."
            }
            SettingRow::SpellingLanguage => {
                // Which key opens the whole list is added from the keymap:
                // see `SettingsView::description`.
                "Automatic detects the language of each paragraph."
            }
            SettingRow::Author => "Written on the comments you add.",
            SettingRow::Font => "Body text of a new document.",
            SettingRow::TextSize => "In points.",
            SettingRow::LineHeight => "As a multiple of the text size.",
            SettingRow::PageView => "Show sheets of paper instead of one column.",
            SettingRow::Paper => "Used by page view and Word export.",
            SettingRow::CharacterLimit => "Count the text against a social network's post limit.",
            SettingRow::StatusWords => "Words in the document, or in the selection.",
            SettingRow::StatusCharacters => "Characters, spaces included.",
            SettingRow::StatusReadingTime => "Minutes at an unhurried pace.",
            // How many are changed is added: see `SettingsView::description`.
            SettingRow::KeyboardShortcuts => {
                "Every key of the app in one list: look one up, give a command another key, \
                 export and import them."
            }
            // Says which engine was found: see `SettingsView::description`.
            SettingRow::Assistant => "",
            SettingRow::AssistantQuickModel => {
                "Answers about a word, a sentence or a paragraph. A fast model keeps it quick."
            }
            SettingRow::AssistantQuickEffort => "How long the model thinks before it answers.",
            SettingRow::AssistantSideModel => "Answers about the whole document and the library.",
            SettingRow::AssistantSideEffort => "How long the model thinks before it answers.",
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
            SettingRow::DocumentSpellingLanguage => "This document's own language, or the app's.",
        }
    }
}

/// The heading of the assistant's section.
pub const ASSISTANT_SECTION: &str = "Assistant";

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
    /// Nothing to set in the row itself: it opens something.
    Open {
        label: SharedString,
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
    /// A row that is still to be scrolled to the top of the list.
    reveal_at_top: Option<SettingRow>,
    scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DismissEvent> for SettingsView {}

/// What the settings modal asks of the workspace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsEvent {
    /// Open the searchable list of spelling languages for the app's
    /// setting or for the open document, and come back to its row.
    ChooseLanguage(LanguageTarget),
    /// Open the list of keyboard shortcuts in the place of the settings.
    OpenKeyboardShortcuts,
}

impl EventEmitter<SettingsEvent> for SettingsView {}

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
        if rows.contains(&row) {
            self.start = row;
            // The section the modal was opened for starts at the top of
            // the list: scrolled there once the list has been laid out
            // (see `render`).
            self.reveal_at_top = Some(row);
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
        for row in SettingRow::APP
            .into_iter()
            .chain(SettingRow::ASSISTANT)
            .chain(SettingRow::DOCUMENT)
        {
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
        if let Some(host) = crate::assistant::host(cx) {
            // The engine is looked for in the background while the modal
            // is open: the section appears, or its line changes, when the
            // answer is in.
            subscriptions.push(cx.observe(&host, |_, _, cx| cx.notify()));
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
            reveal_at_top: None,
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
        if self.assistant_installed(cx) {
            rows.push(SettingRow::Assistant);
            if self.settings.assistant.enabled {
                rows.extend(
                    SettingRow::ASSISTANT
                        .into_iter()
                        .filter(|row| *row != SettingRow::Assistant),
                );
            }
        }
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

    /// Whether the app has an engine for the assistant and it is installed
    /// on this Mac, as far as is known.
    fn assistant_installed(&self, cx: &App) -> bool {
        let Some(host) = crate::assistant::host(cx) else {
            return false;
        };
        match host.read(cx).availability() {
            Some(availability) => availability.is_installed(),
            // Not looked yet in this launch: as it was found last, so
            // that the section does not come and go while the look runs.
            None => self
                .library
                .read(cx)
                .setting_or(keys::ASSISTANT_ENGINE_SEEN, false),
        }
    }

    /// The line that stands in for the assistant's section while its
    /// engine is not installed: what the assistant needs. `None` while the
    /// engine is installed, while it is still being looked for, and in an
    /// app without an engine.
    pub fn assistant_missing_note(&self, cx: &App) -> Option<String> {
        let host = crate::assistant::host(cx)?;
        let host = host.read(cx);
        match host.availability() {
            Some(availability) if !availability.is_installed() => {
                let info = host.info();
                Some(format!(
                    "The assistant needs {}, which is not installed on this Mac. {}",
                    info.name, info.install_help
                ))
            }
            _ => None,
        }
    }

    /// The note under the assistant's section: where the text goes and
    /// what it costs. `None` while the section is not listed or the
    /// assistant is off.
    pub fn assistant_privacy_note(&self, cx: &App) -> Option<String> {
        if !self.assistant_installed(cx) || !self.settings.assistant.enabled {
            return None;
        }
        let host = crate::assistant::host(cx)?;
        let info = host.read(cx).info().clone();
        Some(format!(
            "What you ask is sent to {vendor} through your own {name} sign-in, and each message \
             counts against your {name} plan. The quick chat sends your question and the selected \
             text; the side chat sends your messages, the open document and what the assistant \
             reads from your library. Nothing is sent before you send a message, and the \
             assistant never changes a document by itself.",
            vendor = info.vendor,
            name = info.name,
        ))
    }

    /// The mode and whether the row is the model (not the effort), for the
    /// four choices of the assistant.
    fn assistant_choice(row: SettingRow) -> Option<(writer_assistant::Mode, bool)> {
        use writer_assistant::Mode;
        match row {
            SettingRow::AssistantQuickModel => Some((Mode::Quick, true)),
            SettingRow::AssistantQuickEffort => Some((Mode::Quick, false)),
            SettingRow::AssistantSideModel => Some((Mode::Side, true)),
            SettingRow::AssistantSideEffort => Some((Mode::Side, false)),
            _ => None,
        }
    }

    /// The engine's catalog and the remembered choice of `mode`.
    fn assistant_selection(
        &self,
        mode: writer_assistant::Mode,
        cx: &App,
    ) -> Option<(writer_assistant::Catalog, writer_assistant::Selection)> {
        let host = crate::assistant::host(cx)?;
        let catalog = host.read(cx).catalog().clone();
        let selection =
            crate::assistant::choices::selection_for(mode, &self.settings.assistant, &catalog);
        Some((catalog, selection))
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
                    "Modal editing with Vim keys. Restart The Writer for held keys to repeat \
                     instead of offering accents."
                        .into()
                } else {
                    "Modal editing with Vim keys. Restart The Writer to get the accent menu on \
                     held keys back."
                        .into()
                }
            }
            SettingRow::Assistant => {
                // Which engine was found, and what to do when the user is
                // not signed in to it.
                let Some(host) = crate::assistant::host(cx) else {
                    return "".into();
                };
                let host = host.read(cx);
                if host.availability().is_none() {
                    // Still looking.
                    host.status_line().into()
                } else if host.is_signed_out() {
                    format!("{}. {}", host.status_line(), host.info().sign_in_help).into()
                } else {
                    format!("Runs on {}.", host.status_line()).into()
                }
            }
            SettingRow::DocumentSpellingLanguage => {
                let follows = self
                    .document_settings(cx)
                    .is_some_and(|settings| settings.spelling_language.is_none());
                let says = if follows {
                    format!(
                        "The app's setting applies: {}.",
                        spelling::language_label(&self.settings.spelling_language, cx)
                    )
                } else {
                    row.description().to_string()
                };
                format!("{says}{}", whole_list_sentence(cx)).into()
            }
            SettingRow::SpellingLanguage => {
                format!("{}{}", row.description(), whole_list_sentence(cx)).into()
            }
            SettingRow::KeyboardShortcuts => {
                let changed = crate::workspace::changed_command_count(cx);
                match crate::workspace::own_keys_sentence(changed) {
                    None => row.description().into(),
                    Some(sentence) => format!("{} {sentence}", row.description()).into(),
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
            SettingRow::KeyboardShortcuts => Control::Open {
                label: "Open".into(),
            },
            SettingRow::VimMode => Control::Switch(settings.vim_mode),
            SettingRow::MarkdownShortcuts => Control::Switch(settings.input_rules),
            SettingRow::SmartPunctuation => Control::Switch(settings.smart_punctuation),
            SettingRow::FormatBar => Control::Switch(settings.format_bar),
            SettingRow::CheckSpelling => Control::Switch(settings.check_spelling),
            SettingRow::SpellingLanguage => {
                let options = spelling::app_language_options(&settings.spelling_language, cx);
                Control::Stepper {
                    value: spelling::language_label(&settings.spelling_language, cx).into(),
                    index: options
                        .iter()
                        .position(|option| *option == settings.spelling_language)
                        .unwrap_or(0),
                    count: options.len(),
                }
            }
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
            SettingRow::StatusWords => Control::Switch(settings.status_words),
            SettingRow::StatusCharacters => Control::Switch(settings.status_characters),
            SettingRow::StatusReadingTime => Control::Switch(settings.status_reading_time),
            SettingRow::Assistant => Control::Switch(settings.assistant.enabled),
            SettingRow::AssistantQuickModel
            | SettingRow::AssistantQuickEffort
            | SettingRow::AssistantSideModel
            | SettingRow::AssistantSideEffort => {
                use crate::assistant::choices;
                let unknown = Control::Stepper {
                    value: "".into(),
                    index: 0,
                    count: 1,
                };
                let Some((mode, is_model)) = Self::assistant_choice(row) else {
                    return unknown;
                };
                let Some((catalog, selection)) = self.assistant_selection(mode, cx) else {
                    return unknown;
                };
                if is_model {
                    let options = choices::model_options(&catalog, &selection.model);
                    Control::Stepper {
                        value: catalog.model_label(&selection.model).into(),
                        index: options
                            .iter()
                            .position(|option| *option == selection.model)
                            .unwrap_or(0),
                        count: options.len(),
                    }
                } else {
                    let options = choices::effort_options(&catalog, &selection.model);
                    Control::Stepper {
                        value: catalog.effort_label(&selection.effort).into(),
                        index: options
                            .iter()
                            .position(|option| *option == selection.effort)
                            .unwrap_or(0),
                        count: options.len(),
                    }
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
            SettingRow::DocumentSpellingLanguage => {
                let current = document.spelling_language.as_ref();
                let options = spelling::document_language_options(current, cx);
                Control::Stepper {
                    value: match current {
                        None => spelling::FOLLOW_APP_LABEL.into(),
                        Some(language) => spelling::language_label(language, cx).into(),
                    },
                    index: options
                        .iter()
                        .position(|option| option.as_ref() == current)
                        .unwrap_or(0),
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
            SettingRow::SmartPunctuation => self.write(keys::SMART_PUNCTUATION, &(index != 0), cx),
            SettingRow::FormatBar => self.write(keys::FORMAT_BAR, &(index != 0), cx),
            SettingRow::CheckSpelling => self.write(keys::CHECK_SPELLING, &(index != 0), cx),
            SettingRow::SpellingLanguage => {
                let options = spelling::app_language_options(&self.settings.spelling_language, cx);
                if let Some(language) = options.get(index) {
                    self.write(keys::SPELLING_LANGUAGE, language.as_setting_str(), cx);
                }
            }
            SettingRow::Author | SettingRow::KeyboardShortcuts => {}
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
            SettingRow::StatusWords => self.write(keys::STATUS_WORDS, &(index != 0), cx),
            SettingRow::StatusCharacters => self.write(keys::STATUS_CHARACTERS, &(index != 0), cx),
            SettingRow::StatusReadingTime => {
                self.write(keys::STATUS_READING_TIME, &(index != 0), cx)
            }
            SettingRow::Assistant => self.write(keys::ASSISTANT_ENABLED, &(index != 0), cx),
            SettingRow::AssistantQuickModel
            | SettingRow::AssistantQuickEffort
            | SettingRow::AssistantSideModel
            | SettingRow::AssistantSideEffort => {
                use crate::assistant::choices;
                let Some((mode, is_model)) = Self::assistant_choice(row) else {
                    return;
                };
                let Some((catalog, selection)) = self.assistant_selection(mode, cx) else {
                    return;
                };
                let (model_key, effort_key) = choices::keys_for(mode);
                if is_model {
                    let options = choices::model_options(&catalog, &selection.model);
                    let Some(model) = options.get(index).cloned() else {
                        return;
                    };
                    // An effort the new model does not take gives way to
                    // the engine's default, and is stored so.
                    let normalized = catalog.normalized(&writer_assistant::Selection {
                        model: model.clone(),
                        effort: selection.effort.clone(),
                    });
                    self.write(model_key, &choices::stored_model(&model), cx);
                    if normalized.effort != selection.effort {
                        self.write(effort_key, &choices::stored_effort(&normalized.effort), cx);
                    }
                } else {
                    let options = choices::effort_options(&catalog, &selection.model);
                    if let Some(effort) = options.get(index) {
                        self.write(effort_key, &choices::stored_effort(effort), cx);
                    }
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
            SettingRow::DocumentSpellingLanguage => {
                let current = self
                    .document_settings(cx)
                    .and_then(|settings| settings.spelling_language);
                let options = spelling::document_language_options(current.as_ref(), cx);
                if let Some(language) = options.get(index).cloned() {
                    self.change_document(|settings| settings.spelling_language = language, cx);
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
            Control::Field { .. } | Control::Open { .. } => {}
        }
    }

    /// `space` / `enter`: flips a switch, steps a choice forward and wraps
    /// around at the end.
    pub fn activate(&mut self, row: SettingRow, cx: &mut Context<Self>) {
        if let Some(target) = Self::language_target(row) {
            // A long list: the searchable picker, which comes back here.
            cx.emit(SettingsEvent::ChooseLanguage(target));
            return;
        }
        if row == SettingRow::KeyboardShortcuts {
            cx.emit(SettingsEvent::OpenKeyboardShortcuts);
            return;
        }
        match self.control(row, cx) {
            Control::Switch(on) => self.choose(row, usize::from(!on), cx),
            Control::Segments { options, selected } => {
                self.choose(row, (selected + 1) % options.len().max(1), cx)
            }
            Control::Stepper { index, count, .. } => {
                self.choose(row, (index + 1) % count.max(1), cx)
            }
            Control::Field { .. } | Control::Open { .. } => {}
        }
    }

    /// Whose language the row chooses, for the two spelling-language rows.
    fn language_target(row: SettingRow) -> Option<LanguageTarget> {
        match row {
            SettingRow::SpellingLanguage => Some(LanguageTarget::App),
            SettingRow::DocumentSpellingLanguage => Some(LanguageTarget::Document),
            _ => None,
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
        // One scrolled child per row (see `render`): the row comes into
        // view whole, with its section heading when it is the first of
        // its section.
        self.scroll.scroll_to_item(target);
        cx.notify();
    }

    /// The scroll position of the list (tests read it).
    pub fn scroll_handle(&self) -> &ScrollHandle {
        &self.scroll
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
            // What a click on the row does, as a small button of its own.
            Control::Open { label } => div()
                .flex()
                .flex_none()
                .items_center()
                .gap(SPACE_1)
                .h(px(26.))
                .px(SPACE_3)
                .rounded(RADIUS_MD)
                .border_1()
                .border_color(colors.hairline)
                .bg(colors.hover)
                .text_size(UI_TEXT_SM)
                .whitespace_nowrap()
                .child(label)
                .child(div().text_color(colors.muted).child("›"))
                .into_any_element(),
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
            .min_h(px(42.))
            .py(px(4.))
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

/// " ↩ or Space opens the whole list.", with the keys the keymap has for
/// a row's own action; empty when it has none.
fn whole_list_sentence(cx: &App) -> String {
    crate::keymap::keys_sentence_in(&Activate, ROW_KEY_CONTEXT, cx)
        .map(|keys| format!(" {keys} opens the whole list."))
        .unwrap_or_default()
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
        // What space does on the row the keyboard is on: the two language
        // rows open their list, every other row switches or steps.
        let on_row = |wanted: &dyn Fn(SettingRow) -> bool| {
            self.handles
                .iter()
                .any(|(row, handle)| wanted(*row) && handle.is_focused(window))
        };
        // The row that opens the keyboard shortcuts has nothing to step.
        let opens = on_row(&|row| row == SettingRow::KeyboardShortcuts);
        let space_hint = if on_row(&|row| Self::language_target(row).is_some()) {
            "Whole list"
        } else if opens {
            "Open"
        } else {
            "Switch"
        };

        // A scroll position asked for before the list was ever drawn is
        // worked out against a list at the top left of the window (GPUI
        // learns where the list is while drawing it) and lands too far
        // down. So the row to start at is scrolled to after the first
        // frame.
        if let Some(row) = self.reveal_at_top {
            if self.scroll.bounds().size.height > px(0.) {
                self.reveal_at_top = None;
                if let Some(ix) = self.rows(cx).iter().position(|listed| *listed == row) {
                    self.scroll.scroll_to_top_of_item(ix);
                }
            } else {
                cx.defer_in(window, |_, _, cx| cx.notify());
            }
        }

        // One child of the scrolled list per row. The first row of a
        // section carries the section heading in the same child, so that
        // bringing the row into view brings the heading with it (and never
        // the heading alone). The child is a little taller than the row:
        // the focus ring is drawn outside the row and would otherwise be
        // cut at the edge of the list.
        let mut children: Vec<gpui::AnyElement> = Vec::new();
        let mut section = "";
        let listed = self.rows(cx);
        for (ix, row) in listed.iter().copied().enumerate() {
            let heading = (row.section() != section).then(|| {
                section = row.section();
                div()
                    .px(SPACE_3)
                    .pt(if ix == 0 { px(1.) } else { px(11.) })
                    .pb(px(4.))
                    .text_size(UI_TEXT_XS)
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(colors.muted)
                    .child(section)
            });
            // What stands under a row without being a setting (and so
            // without being a stop of the keyboard): it is part of that
            // row's child, so that the list still has one child per row.
            let note_after = if listed.get(ix + 1).map(|next| next.section()) != Some(section) {
                match section {
                    // The assistant's section ends with where the text
                    // goes.
                    ASSISTANT_SECTION => self
                        .assistant_privacy_note(cx)
                        .map(|note| (None, note, "assistant-privacy-note")),
                    // Without its engine the assistant has no section:
                    // one quiet line after the app's settings says what
                    // it needs.
                    "Status bar" => self
                        .assistant_missing_note(cx)
                        .map(|note| (Some(ASSISTANT_SECTION), note, "assistant-missing-note")),
                    _ => None,
                }
            } else {
                None
            };
            let note_after = note_after.map(|(heading, note, selector)| {
                div()
                    .debug_selector(move || selector.into())
                    .flex()
                    .flex_col()
                    .children(heading.map(|heading| {
                        div()
                            .px(SPACE_3)
                            .pt(px(14.))
                            .pb(px(4.))
                            .text_size(UI_TEXT_XS)
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(colors.muted)
                            .child(heading)
                    }))
                    .child(
                        div()
                            .px(SPACE_3)
                            .pt(px(4.))
                            .pb(px(2.))
                            .text_size(UI_TEXT_SM)
                            .line_height(px(16.))
                            .text_color(colors.muted)
                            .child(note),
                    )
            });
            children.push(
                div()
                    .debug_selector(move || format!("setting-{ix}"))
                    .flex_none()
                    .py(px(3.))
                    .flex()
                    .flex_col()
                    .children(heading)
                    .child(self.render_row(ix, row, window, cx))
                    .children(note_after)
                    .into_any_element(),
            );
        }
        let close = crate::controls::close_control(
            cx.listener(|_, _: &gpui::ClickEvent, _, cx| cx.emit(DismissEvent)),
            cx,
        );

        // The footer names the keys as the keymap has them.
        let hint = |actions: &[&dyn gpui::Action], place: &str, what: &'static str| {
            crate::controls::footer_hint(actions, place, what, cx)
        };

        div()
            .id("settings")
            .key_context(KEY_CONTEXT)
            .on_action(cx.listener(Self::on_focus_next))
            .on_action(cx.listener(Self::on_focus_previous))
            // Clicks on the panel's own chrome must not close the modal or
            // steal focus from the focused row.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .w(px(crate::workspace::MODAL_WIDTH))
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
                    .child(div().flex_1().text_size(UI_TEXT_LG).child("Settings"))
                    .child(close),
            )
            .child(
                div()
                    .id("settings-list")
                    .debug_selector(|| "settings-list".into())
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
                    .children(hint(&[&FocusPrevious, &FocusNext], KEY_CONTEXT, "Move"))
                    .when(!opens, |el| {
                        el.children(hint(&[&Decrease, &Increase], ROW_KEY_CONTEXT, "Change"))
                    })
                    .children(hint(&[&Activate], ROW_KEY_CONTEXT, space_hint))
                    .children(crate::controls::close_hint(cx)),
            )
    }
}
