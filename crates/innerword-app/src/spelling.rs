//! Spell checking, as the app sees it.
//!
//! The checking itself is `innerword-spell`: a [`SpellService`] (the Mac's own
//! checker, or a fake built from a word list) and a checker entity that
//! keeps the editor's spelling marks current. This module holds what the
//! app adds:
//!
//! - **which service a workspace uses** ([`set_service`]): `main` installs
//!   the system service ([`use_system_service`]); a test installs a fake
//!   before it opens a workspace. Without one a workspace has no checker
//!   at all, so no test ever reaches the system dictionary by accident;
//! - **the languages** for the settings and the picker ([`languages`],
//!   [`language_label`]): "Automatic" and the system's languages by their
//!   display names, the user's preferred ones first;
//! - **which language is in force** ([`effective_language`]): the open
//!   document's own (`innerword_core::DocumentSettings::spelling_language`)
//!   when it has one, else the app's setting;
//! - **the picker** ([`LanguagePickerDelegate`]): a searchable list for the
//!   language of the open document (the palette command "Spelling
//!   language", Edit > Spelling in the menu bar) or of the app (`enter` on
//!   the row in the settings).

use std::rc::Rc;

use gpui::{App, Context, Global, SharedString, WeakEntity, Window};
use innerword_core::SpellingLanguage;
use innerword_spell::{LanguageInfo, SpellLanguage, SpellService};
use innerword_ui::{Picker, PickerDelegate, PickerEntry, PickerItem, fuzzy::match_strings};

use crate::{settings_view::SettingRow, workspace::Workspace};

/// What the settings and the picker call automatic detection.
pub const AUTOMATIC_LABEL: &str = "Automatic";
/// What the "This document" settings and the picker call "no language of
/// its own".
pub const FOLLOW_APP_LABEL: &str = "Follow the app setting";

/// What the picker writes beside the choice that applies now.
pub const IN_USE_LABEL: &str = "in use";

/// The spell service the workspaces of this app use.
struct ServiceGlobal(Rc<dyn SpellService>);

impl Global for ServiceGlobal {}

/// Installs `service` for every workspace opened from now on. Tests call
/// this with a fake before they open a workspace.
pub fn set_service(service: Rc<dyn SpellService>, cx: &mut App) {
    cx.set_global(ServiceGlobal(service));
}

/// Installs the Mac's own spell checker. Called by `main`, and by nothing
/// else: its answers differ between systems, and its learned words are
/// shared with every other app.
pub fn use_system_service(cx: &mut App) {
    let service: Rc<dyn SpellService> = Rc::new(innerword_spell::SystemSpellService::new());
    // The checker needs a moment the first time it is asked; better now
    // than under the first keystroke.
    service.warm_up();
    set_service(service, cx);
}

/// The installed spell service, if there is one.
pub fn service(cx: &App) -> Option<Rc<dyn SpellService>> {
    cx.try_global::<ServiceGlobal>()
        .map(|global| global.0.clone())
}

/// The languages a person can choose from: the user's preferred ones
/// first, then the others by name. Empty without a service.
pub fn languages(cx: &App) -> Vec<LanguageInfo> {
    match service(cx) {
        Some(service) => innerword_spell::language_choices(service.as_ref()),
        None => Vec::new(),
    }
}

/// How a language choice is named in the interface: "Automatic", or the
/// display name of the language (its identifier when the system does not
/// know it, as with a document written on another Mac).
pub fn language_label(language: &SpellingLanguage, cx: &App) -> String {
    match language {
        SpellingLanguage::Automatic => AUTOMATIC_LABEL.to_string(),
        SpellingLanguage::Language(id) => match service(cx) {
            Some(service) => innerword_spell::language_name(service.as_ref(), id),
            None => id.clone(),
        },
    }
}

/// The language spelling is checked in: the document's own when it names
/// one, else the app's setting.
pub fn effective_language(
    app: &SpellingLanguage,
    document: Option<&SpellingLanguage>,
) -> SpellingLanguage {
    document.unwrap_or(app).clone()
}

/// A stored language choice as the spell service wants it.
pub fn service_language(language: &SpellingLanguage) -> SpellLanguage {
    match language {
        SpellingLanguage::Automatic => SpellLanguage::Automatic,
        SpellingLanguage::Language(id) => SpellLanguage::fixed(id.clone()),
    }
}

/// The choices of a language row, in order: "Automatic", then the
/// languages. A fixed language that is not among the system's (`current`)
/// is listed right after "Automatic", so the row can show it and step
/// away from it.
pub fn app_language_options(current: &SpellingLanguage, cx: &App) -> Vec<SpellingLanguage> {
    let languages = languages(cx);
    let mut options = vec![SpellingLanguage::Automatic];
    if let SpellingLanguage::Language(id) = current
        && !languages.iter().any(|language| &language.id == id)
    {
        options.push(current.clone());
    }
    options.extend(
        languages
            .into_iter()
            .filter_map(|language| SpellingLanguage::language(&language.id))
            .filter(|language| !language.is_automatic()),
    );
    options
}

/// The choices of the document's language row: "Follow the app setting"
/// (`None`), then those of [`app_language_options`].
pub fn document_language_options(
    current: Option<&SpellingLanguage>,
    cx: &App,
) -> Vec<Option<SpellingLanguage>> {
    let listed = app_language_options(current.unwrap_or(&SpellingLanguage::Automatic), cx);
    std::iter::once(None)
        .chain(listed.into_iter().map(Some))
        .collect()
}

// ----- the picker ---------------------------------------------------------------

/// Whose language a [`LanguagePicker`] chooses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LanguageTarget {
    /// The app's setting ("Spelling language" under Writing).
    App,
    /// The open document's own language.
    Document,
}

/// The spelling-language picker as a modal view.
pub type LanguagePicker = Picker<LanguagePickerDelegate>;

/// One row of the picker: `None` is "Follow the app setting".
type Choice = Option<SpellingLanguage>;

/// The picker delegate behind "Spelling language". Type to filter by name
/// (or by identifier, `en_GB`); `enter` chooses.
pub struct LanguagePickerDelegate {
    workspace: WeakEntity<Workspace>,
    target: LanguageTarget,
    /// Reopen the settings at the language row after a choice or a
    /// dismissal: the picker was opened from there.
    back_to_settings: bool,
    /// What is in force now: the row that says so, and the one the
    /// picker opens on.
    current: Choice,
    /// Every choice with its label and detail.
    choices: Vec<(Choice, String, String)>,
    /// The listed rows: an index into `choices` and the match positions.
    rows: Vec<(usize, Vec<usize>)>,
    confirmed: bool,
}

impl LanguagePickerDelegate {
    /// A picker for `target`, with `current` marked.
    pub fn new(
        workspace: WeakEntity<Workspace>,
        target: LanguageTarget,
        current: Choice,
        app_language: &SpellingLanguage,
        cx: &App,
    ) -> Self {
        let languages = languages(cx);
        let mut choices: Vec<(Choice, String, String)> = Vec::new();
        if target == LanguageTarget::Document {
            choices.push((
                None,
                FOLLOW_APP_LABEL.to_string(),
                language_label(app_language, cx),
            ));
        }
        choices.push((
            Some(SpellingLanguage::Automatic),
            AUTOMATIC_LABEL.to_string(),
            "Detects the language of each paragraph".to_string(),
        ));
        if let Some(SpellingLanguage::Language(id)) = &current
            && !languages.iter().any(|language| &language.id == id)
        {
            choices.push((current.clone(), id.clone(), "Not on this Mac".to_string()));
        }
        for language in languages {
            if let Some(choice) = SpellingLanguage::language(&language.id)
                && !choice.is_automatic()
            {
                choices.push((Some(choice), language.name, language.id));
            }
        }
        Self {
            workspace,
            target,
            back_to_settings: false,
            current,
            choices,
            rows: Vec::new(),
            confirmed: false,
        }
    }

    /// Reopens the settings at the language row when the picker closes.
    pub fn returning_to_settings(mut self) -> Self {
        self.back_to_settings = true;
        self
    }

    /// Whose language is chosen.
    pub fn target(&self) -> LanguageTarget {
        self.target
    }

    /// The label of the choice that applies now, if it is listed.
    pub fn current_label(&self) -> Option<String> {
        self.rows
            .iter()
            .map(|(ix, _)| &self.choices[*ix])
            .find(|(choice, _, _)| *choice == self.current)
            .map(|(_, label, _)| label.clone())
    }

    /// The labels of the listed rows, in order.
    pub fn labels(&self) -> Vec<String> {
        self.rows
            .iter()
            .map(|(ix, _)| self.choices[*ix].1.clone())
            .collect()
    }

    fn setting_row(&self) -> SettingRow {
        match self.target {
            LanguageTarget::App => SettingRow::SpellingLanguage,
            LanguageTarget::Document => SettingRow::DocumentSpellingLanguage,
        }
    }

    fn reopen_settings(&self, window: &mut Window, cx: &mut Context<Picker<Self>>) {
        let workspace = self.workspace.clone();
        let row = self.setting_row();
        window.defer(cx, move |window, cx| {
            workspace
                .update(cx, |workspace, cx| {
                    workspace.show_settings_at(row, window, cx)
                })
                .ok();
        });
    }
}

impl PickerDelegate for LanguagePickerDelegate {
    fn update_matches(&mut self, query: &str, _: &mut Window, _: &mut Context<Picker<Self>>) {
        let query = query.trim();
        if query.is_empty() {
            self.rows = (0..self.choices.len()).map(|ix| (ix, Vec::new())).collect();
            return;
        }
        let by_name = match_strings(
            query,
            self.choices.iter().map(|(_, label, _)| label.as_str()),
            self.choices.len(),
        );
        self.rows = by_name
            .into_iter()
            .map(|found| (found.candidate_index, found.positions))
            .collect();
        // An identifier typed as it is written (`en_GB`, `de`) finds its
        // language too.
        let wanted = query.to_lowercase().replace('-', "_");
        for (ix, (choice, _, _)) in self.choices.iter().enumerate() {
            let matches = choice
                .as_ref()
                .and_then(SpellingLanguage::identifier)
                .is_some_and(|id| id.to_lowercase().replace('-', "_").starts_with(&wanted));
            if matches && !self.rows.iter().any(|(listed, _)| *listed == ix) {
                self.rows.push((ix, Vec::new()));
            }
        }
    }

    fn entry_count(&self) -> usize {
        self.rows.len()
    }

    fn entry(&self, ix: usize, _: &App) -> PickerEntry {
        let (choice_ix, positions) = &self.rows[ix];
        let (choice, label, detail) = &self.choices[*choice_ix];
        // The choice in force says so beside its name (an icon on one row
        // only would push its label out of line with the others); it is
        // also the row the picker opens on.
        let detail = if *choice == self.current {
            format!("{detail} \u{b7} {IN_USE_LABEL}")
        } else {
            detail.clone()
        };
        PickerItem::new(label.clone())
            .positions(positions.clone())
            .detail(detail)
            .into()
    }

    fn initial_selection(&self, query: &str) -> Option<usize> {
        if !query.trim().is_empty() {
            return None;
        }
        self.rows
            .iter()
            .position(|(ix, _)| self.choices[*ix].0 == self.current)
    }

    fn confirm(
        &mut self,
        ix: usize,
        _secondary: bool,
        window: &mut Window,
        cx: &mut Context<Picker<Self>>,
    ) -> bool {
        let Some((choice_ix, _)) = self.rows.get(ix) else {
            return false;
        };
        let choice = self.choices[*choice_ix].0.clone();
        let target = self.target;
        let workspace = self.workspace.clone();
        let back = self.back_to_settings.then(|| self.setting_row());
        self.confirmed = true;
        window.defer(cx, move |window, cx| {
            workspace
                .update(cx, |workspace, cx| {
                    workspace.dismiss_modal(window, cx);
                    match (target, choice) {
                        (LanguageTarget::Document, choice) => {
                            workspace.set_document_spelling_language(choice, cx)
                        }
                        (LanguageTarget::App, Some(language)) => {
                            workspace.set_app_spelling_language(&language, cx)
                        }
                        (LanguageTarget::App, None) => {}
                    }
                    if let Some(row) = back {
                        workspace.show_settings_at(row, window, cx);
                    }
                })
                .ok();
        });
        false
    }

    fn dismissed(&mut self, window: &mut Window, cx: &mut Context<Picker<Self>>) {
        // Escape goes back to where the picker was opened from.
        if self.back_to_settings && !self.confirmed {
            self.reopen_settings(window, cx);
        }
    }

    fn placeholder(&self) -> SharedString {
        match self.target {
            LanguageTarget::App => "Spelling language".into(),
            LanguageTarget::Document => "Spelling language of this document".into(),
        }
    }

    fn empty_text(&self, _query: &str) -> SharedString {
        "No language of that name".into()
    }
}
