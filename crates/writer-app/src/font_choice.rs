//! The searchable list of fonts behind the two font rows of the settings
//! ("Font" under "New documents" and under "This document").
//!
//! It lists what the font picker of the text lists
//! ([`writer_editor_tools::FontList`], the one source of "the fonts that
//! can be chosen"): the installed writing fonts, the recently used
//! families and every family by name, each row with a sample in its own
//! face; typing narrows the list with the same fuzzy match. The list is
//! virtualized ([`writer_ui::Picker`]), so only the dozen rows in view are
//! built and only their faces are loaded, however many families the Mac
//! has.
//!
//! It works like the list of spelling languages
//! ([`crate::spelling::LanguagePicker`]): it takes the place of the
//! settings, opens on the font in use, and goes back to its row in the
//! settings after a choice (`enter`, a click) and after `escape`, which
//! changes nothing. A click outside the list changes nothing either and
//! closes all of it: the keyboard is back in the text.
//!
//! A font that is set but not installed on this Mac (a library that came
//! from another one) is the first row, marked [`MISSING_LABEL`]: it stays
//! set until another is chosen.

use gpui::{App, Context, SharedString, WeakEntity, Window};
use writer_editor_tools::{FontList, FontRow};
use writer_ui::picker::{Picker, PickerDelegate, PickerEntry, PickerItem};

use crate::{settings_view::SettingRow, spelling::IN_USE_LABEL, workspace::Workspace};

/// What the list says beside a font that is set but not installed.
pub const MISSING_LABEL: &str = "Not on this Mac";

/// Whose font a [`FontChoicePicker`] chooses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontTarget {
    /// The body font a new document starts with ("Font" under "New
    /// documents").
    NewDocuments,
    /// The body font of the open document ("Font" under "This document").
    Document,
}

impl FontTarget {
    /// The row of the settings that opens the list for this target.
    pub fn setting_row(self) -> SettingRow {
        match self {
            FontTarget::NewDocuments => SettingRow::Font,
            FontTarget::Document => SettingRow::DocumentFont,
        }
    }
}

/// The list of fonts of the settings as a modal view.
pub type FontChoicePicker = Picker<FontChoiceDelegate>;

/// The picker delegate behind the font rows of the settings. Type to
/// filter by name; `enter` chooses.
pub struct FontChoiceDelegate {
    workspace: WeakEntity<Workspace>,
    target: FontTarget,
    /// The font in force: the row that says so, and the one the list
    /// opens on.
    current: String,
    fonts: FontList,
    /// `current` when it is not an installed family: the leading row.
    missing: Option<String>,
    rows: Vec<FontRow>,
    query: String,
    confirmed: bool,
}

impl FontChoiceDelegate {
    /// A list for `target` over `fonts`, with `current` marked.
    pub fn new(
        workspace: WeakEntity<Workspace>,
        target: FontTarget,
        current: String,
        fonts: FontList,
    ) -> Self {
        let missing =
            (!current.is_empty() && fonts.position(&current).is_none()).then(|| current.clone());
        Self {
            workspace,
            target,
            current,
            fonts,
            missing,
            rows: Vec::new(),
            query: String::new(),
            confirmed: false,
        }
    }

    /// Whose font is chosen.
    pub fn target(&self) -> FontTarget {
        self.target
    }

    /// The font in force.
    pub fn current(&self) -> &str {
        &self.current
    }

    /// The font that is set but not installed on this Mac, if that is so.
    pub fn missing(&self) -> Option<&str> {
        self.missing.as_deref()
    }

    /// What is typed into the list.
    pub fn query(&self) -> &str {
        &self.query
    }

    /// The listed rows as the shared list of fonts gives them.
    pub fn rows(&self) -> &[FontRow] {
        &self.rows
    }

    /// The font of row `ix`; `None` for a heading.
    pub fn font_at(&self, ix: usize) -> Option<String> {
        match self.rows.get(ix)? {
            FontRow::Header(_) => None,
            FontRow::Leading { .. } => self.missing.clone(),
            FontRow::Family { index, .. } => {
                self.fonts.families().get(*index).map(|f| f.to_string())
            }
        }
    }

    /// The labels of the listed rows, headings included, in order.
    pub fn labels(&self) -> Vec<String> {
        (0..self.rows.len())
            .map(|ix| match &self.rows[ix] {
                FontRow::Header(title) => title.to_string(),
                _ => self.font_at(ix).unwrap_or_default(),
            })
            .collect()
    }

    fn leading(&self) -> Vec<&str> {
        self.missing.as_deref().into_iter().collect()
    }

    fn back_to_settings(&self, window: &mut Window, cx: &mut Context<Picker<Self>>) {
        let workspace = self.workspace.clone();
        let row = self.target.setting_row();
        window.defer(cx, move |window, cx| {
            workspace
                .update(cx, |workspace, cx| {
                    workspace.show_settings_at(row, window, cx)
                })
                .ok();
        });
    }
}

impl PickerDelegate for FontChoiceDelegate {
    fn update_matches(&mut self, query: &str, _: &mut Window, _: &mut Context<Picker<Self>>) {
        self.query = query.to_string();
        self.rows = self.fonts.rows(&self.leading(), query);
    }

    fn entry_count(&self) -> usize {
        self.rows.len()
    }

    fn entry(&self, ix: usize, _: &App) -> PickerEntry {
        // The font in force says so beside its name (as the language in
        // force does in the list of languages).
        let in_use = |font: &str| font == self.current;
        match &self.rows[ix] {
            FontRow::Header(title) => PickerEntry::Header(SharedString::from(*title)),
            FontRow::Leading { positions, .. } => {
                let font = self.missing.clone().unwrap_or_default();
                let detail = if in_use(&font) {
                    format!("{MISSING_LABEL} \u{b7} {IN_USE_LABEL}")
                } else {
                    MISSING_LABEL.to_string()
                };
                // No sample: there is no such face to set it in.
                PickerItem::new(font)
                    .positions(positions.clone())
                    .detail(detail)
                    .into()
            }
            FontRow::Family { index, positions } => {
                let family = self.fonts.families()[*index].clone();
                let item = PickerItem::new(family.clone())
                    .positions(positions.clone())
                    .sample_font(family.clone());
                if in_use(&family) {
                    item.detail(IN_USE_LABEL).into()
                } else {
                    item.into()
                }
            }
        }
    }

    fn initial_selection(&self, query: &str) -> Option<usize> {
        if !query.trim().is_empty() {
            return None;
        }
        (0..self.rows.len()).find(|ix| self.font_at(*ix).as_deref() == Some(&self.current))
    }

    fn confirm(
        &mut self,
        ix: usize,
        _secondary: bool,
        window: &mut Window,
        cx: &mut Context<Picker<Self>>,
    ) -> bool {
        let Some(font) = self.font_at(ix) else {
            return false;
        };
        let installed = !matches!(self.rows.get(ix), Some(FontRow::Leading { .. }));
        let target = self.target;
        let workspace = self.workspace.clone();
        self.confirmed = true;
        window.defer(cx, move |window, cx| {
            workspace
                .update(cx, |workspace, cx| {
                    workspace.dismiss_modal(window, cx);
                    workspace.choose_font(target, &font, installed, cx);
                    workspace.show_settings_at(target.setting_row(), window, cx);
                })
                .ok();
        });
        false
    }

    fn dismissed(&mut self, window: &mut Window, cx: &mut Context<Picker<Self>>) {
        // Escape goes back to the row the list was opened from.
        if !self.confirmed {
            self.back_to_settings(window, cx);
        }
    }

    fn placeholder(&self) -> SharedString {
        match self.target {
            FontTarget::NewDocuments => "Font of new documents".into(),
            FontTarget::Document => "Font of this document".into(),
        }
    }

    fn empty_text(&self, _query: &str) -> SharedString {
        "No matching font".into()
    }
}
