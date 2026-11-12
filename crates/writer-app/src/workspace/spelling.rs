//! Spell checking in the workspace: the checker of the one editor, the
//! language in force, the commands and the picker.
//!
//! The checker ([`writer_spell::SpellChecker`]) is attached to the
//! workspace's editor once, when the app has a spell service
//! ([`crate::spelling::set_service`]), and follows the editor from one
//! document to the next by itself. The workspace tells it two things:
//!
//! - **whether to check** (the `spelling.check_while_typing` setting; the
//!   checker's own toggle reports back and the setting follows);
//! - **in which language**: the open document's own
//!   (`DocumentSettings::spelling_language`) when it has one, else the
//!   app's setting. Looked at again whenever either changes and whenever
//!   another document is opened.
//!
//! The spelling commands (`spell::*`) are handled on the workspace root, so
//! they work from the palette and the menu bar wherever the keyboard is:
//! the text takes the keyboard first (see [`Workspace::on_spell_action`]).

use std::time::Duration;

use std::rc::Rc;

use gpui::{App, AppContext as _, Context, Entity, Focusable as _, Window, px};
use writer_core::{Position, SpellingLanguage, TextRange};
use writer_spell::{SpellChecker, SpellEvent, actions as spell};
use writer_ui::{Picker, Toast};
use writer_vim_mode::{VimMode, VimSpelling, VimSuggestions};

use super::{MODAL_WIDTH, Workspace, picker_rows};
use crate::{
    actions::ChooseSpellingLanguage,
    menus,
    settings::keys,
    settings_view::{SettingRow, SettingsEvent, SettingsView},
    spelling::{self, LanguagePicker, LanguagePickerDelegate, LanguageTarget},
};

/// The title of the menu bar item that carries the check mark of the
/// "check spelling while typing" setting.
pub const CHECK_SPELLING_MENU_ITEM: &str = "Check Spelling While Typing";

/// The spell checker behind the spelling keys of Vim mode (`]s`, `[s`,
/// `z=`, `zg`, `zug`). The Vim crates know nothing of `writer-spell`; the
/// app connects the two ([`VimMode::set_spelling`]).
struct VimSpellingHooks(Entity<SpellChecker>);

impl VimSpelling for VimSpellingHooks {
    fn is_enabled(&self, cx: &App) -> bool {
        self.0.read(cx).is_active(cx)
    }

    fn misspelling_from(&self, from: Position, forward: bool, cx: &mut App) -> Option<TextRange> {
        self.0.update(cx, |checker, cx| {
            if forward {
                checker.next_misspelling(from, cx)
            } else {
                checker.previous_misspelling(from, cx)
            }
        })
    }

    fn suggestions_at(&self, at: Position, cx: &mut App) -> Option<VimSuggestions> {
        self.0.update(cx, |checker, cx| {
            let range = checker.misspelling_at(at, cx)?;
            let suggestions = checker.suggestions_at(at, cx);
            Some(VimSuggestions { range, suggestions })
        })
    }

    fn replace(&self, range: TextRange, replacement: &str, cx: &mut App) -> bool {
        self.0
            .update(cx, |checker, cx| checker.replace(range, replacement, cx))
    }

    fn learn(&self, at: Position, _word: &str, cx: &mut App) -> Option<String> {
        self.0.update(cx, |checker, cx| checker.learn_at(at, cx))
    }

    fn unlearn(&self, at: Position, _word: &str, cx: &mut App) -> Option<String> {
        self.0.update(cx, |checker, cx| checker.unlearn_at(at, cx))
    }

    /// `z=`: the menu the `Show Suggestions` command opens, under the
    /// caret, with the first suggestion marked.
    fn open_suggestions(&self, _at: Position, window: &mut Window, cx: &mut App) -> bool {
        self.0
            .update(cx, |checker, cx| checker.show_suggestions(window, cx))
    }
}

impl Workspace {
    /// The spell checker of the editor. `None` when the app was given no
    /// spell service (see [`crate::spelling`]).
    pub fn spell(&self) -> Option<&Entity<SpellChecker>> {
        self.spell.as_ref()
    }

    /// Connects the Vim keys for spelling to the checker. Called once, from
    /// [`Workspace::new`]; without a checker the keys keep reporting that
    /// spell checking is off.
    pub(super) fn connect_vim_spelling(&mut self, cx: &mut Context<Self>) {
        let hooks = self
            .spell
            .clone()
            .map(|spell| Rc::new(VimSpellingHooks(spell)) as Rc<dyn VimSpelling>);
        self.vim
            .update(cx, |vim: &mut VimMode, _| vim.set_spelling(hooks));
    }

    /// The language spelling is checked in right now: the open document's
    /// own when it has one, else the app's setting.
    pub fn spelling_language(&self, cx: &gpui::App) -> SpellingLanguage {
        let document = self
            .session
            .is_some()
            .then(|| {
                self.editor
                    .read(cx)
                    .document()
                    .settings()
                    .spelling_language
                    .clone()
            })
            .flatten();
        spelling::effective_language(&self.settings.spelling_language, document.as_ref())
    }

    /// The open document's own spelling language (`None`: it follows the
    /// app's setting, or no document is open).
    pub fn document_spelling_language(&self, cx: &gpui::App) -> Option<SpellingLanguage> {
        self.session.as_ref()?;
        self.editor
            .read(cx)
            .document()
            .settings()
            .spelling_language
            .clone()
    }

    /// Tells the checker whether to check and in which language, and the
    /// menu bar whether "Check Spelling While Typing" is on. Cheap when
    /// nothing changed.
    pub(crate) fn apply_spelling(&mut self, cx: &mut Context<Self>) {
        let on = self.settings.check_spelling;
        menus::set_check_mark(CHECK_SPELLING_MENU_ITEM, on, cx);
        let Some(spell) = self.spell.clone() else {
            return;
        };
        let language = spelling::service_language(&self.spelling_language(cx));
        spell.update(cx, |spell, cx| {
            spell.set_language(language, cx);
            spell.set_enabled(on, cx);
        });
    }

    pub(crate) fn on_spell_event(
        &mut self,
        _spell: &Entity<SpellChecker>,
        event: &SpellEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            // The checker was switched by its own toggle (the palette, the
            // menu bar): the setting follows.
            SpellEvent::EnabledChanged(on) => {
                if self.settings.check_spelling != *on {
                    self.write_setting(keys::CHECK_SPELLING, on, cx);
                }
            }
            // The marks are drawn by the editor; the status line of a
            // scripted run reads them.
            _ => cx.notify(),
        }
    }

    /// Turns checking while typing on or off (a setting) and says so.
    pub fn toggle_spell_checking(&mut self, cx: &mut Context<Self>) {
        let on = !self.settings.check_spelling;
        self.write_setting(keys::CHECK_SPELLING, &on, cx);
        let message = if on {
            "Spelling is checked while you type"
        } else {
            "Spelling is not checked"
        };
        self.show_toast(Toast::new(message).duration(Duration::from_secs(2)), cx);
    }

    /// A spelling command arrived at the workspace root. The ones that act
    /// on a word need the text: it takes the keyboard first (an open modal
    /// closes), so the word that is found is selected where the user can
    /// go on typing, and the suggestions open under a caret that is on
    /// screen.
    pub(crate) fn on_spell_action(
        &mut self,
        action: &dyn gpui::Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if action.as_any().is::<spell::ToggleSpellChecking>() {
            self.toggle_spell_checking(cx);
            return;
        }
        let Some(spell) = self.spell.clone() else {
            cx.propagate();
            return;
        };
        if self.session.is_none() {
            cx.propagate();
            return;
        }
        if !self.settings.check_spelling {
            self.show_toast(
                Toast::new("Spelling is not checked. Turn it on in Edit > Spelling.")
                    .duration(Duration::from_secs(4)),
                cx,
            );
            return;
        }
        if !self.editor.read(cx).is_focused(window) {
            self.dismiss_modal(window, cx);
            window.focus(&self.editor.focus_handle(cx));
        }
        spell.update(cx, |spell, cx| {
            spell.run_action(action, window, cx);
        });
    }

    // ----- the language -------------------------------------------------------------

    /// Gives the open document a spelling language of its own (`None`: it
    /// follows the app's setting again) and says what applies now. Part of
    /// the document: saved with it, and a fixed language is the proofing
    /// language of a Word export.
    pub fn set_document_spelling_language(
        &mut self,
        language: Option<SpellingLanguage>,
        cx: &mut Context<Self>,
    ) {
        if self.session.is_none() || self.editor.read(cx).is_read_only() {
            return;
        }
        self.editor.update(cx, |editor, cx| {
            editor.edit_with(
                writer_editor::Autoscroll::None,
                |state| state.set_spelling_language(language.clone()),
                cx,
            );
        });
        self.mark_dirty(cx);
        self.apply_spelling(cx);
        let message = match &language {
            Some(language) => format!(
                "Spelling of this document: {}",
                spelling::language_label(language, cx)
            ),
            None => format!(
                "Spelling of this document follows the app setting: {}",
                spelling::language_label(&self.settings.spelling_language, cx)
            ),
        };
        self.show_toast(Toast::new(message).duration(Duration::from_secs(3)), cx);
    }

    /// Stores the app's spelling language (what applies to every document
    /// without a language of its own).
    pub fn set_app_spelling_language(
        &mut self,
        language: &SpellingLanguage,
        cx: &mut Context<Self>,
    ) {
        self.write_setting(keys::SPELLING_LANGUAGE, language.as_setting_str(), cx);
    }

    /// Opens the picker for the spelling language of the open document
    /// (the palette command "Spelling language"), or closes it when it is
    /// open.
    pub fn toggle_spelling_language_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.modal_layer.read(cx).is_open::<LanguagePicker>() {
            self.dismiss_modal(window, cx);
            return;
        }
        if self.session.is_none() {
            self.show_toast(
                Toast::new("Open a document to choose its spelling language"),
                cx,
            );
            return;
        }
        self.show_language_picker(LanguageTarget::Document, false, window, cx);
    }

    /// Opens the searchable list of spelling languages for `target`.
    /// `from_settings`: it was asked for in the settings modal and goes
    /// back to its row there, chosen or not.
    pub(crate) fn show_language_picker(
        &mut self,
        target: LanguageTarget,
        from_settings: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current = match target {
            LanguageTarget::App => Some(self.settings.spelling_language.clone()),
            LanguageTarget::Document => {
                if self.session.is_none() || self.editor.read(cx).is_read_only() {
                    return;
                }
                self.document_spelling_language(cx)
            }
        };
        let workspace = cx.weak_entity();
        let app_language = self.settings.spelling_language.clone();
        self.modal_layer.update(cx, |layer, cx| {
            let view = cx.new(|cx| {
                let mut delegate =
                    LanguagePickerDelegate::new(workspace, target, current, &app_language, cx);
                if from_settings {
                    delegate = delegate.returning_to_settings();
                }
                let rows = picker_rows(window, 14);
                Picker::modal(delegate, window, cx)
                    .width(px(MODAL_WIDTH))
                    .max_visible_rows(rows)
            });
            layer.show(view, window, cx);
        });
    }

    /// Opens the settings with the keyboard on `row` (they stay open when
    /// they already are).
    pub fn show_settings_at(
        &mut self,
        row: SettingRow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.modal_layer.read(cx).is_open::<SettingsView>() {
            return;
        }
        self.show_settings(Some(row), window, cx);
    }

    /// What the settings modal asks of the workspace.
    pub(crate) fn on_settings_event(
        &mut self,
        _view: &Entity<SettingsView>,
        event: &SettingsEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            SettingsEvent::ChooseLanguage(target) => {
                self.show_language_picker(*target, true, window, cx);
            }
            SettingsEvent::ChooseFont(target) => self.show_font_list(*target, window, cx),
            // The list takes the place of the settings.
            SettingsEvent::OpenKeyboardShortcuts => {
                self.show_shortcuts(window, cx);
            }
        }
    }

    pub(crate) fn on_choose_spelling_language(
        &mut self,
        _: &ChooseSpellingLanguage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_spelling_language_picker(window, cx);
    }

    /// One line about spell checking, for the `state` step of a scripted
    /// run: on or off, the language in force (and whose it is), how many
    /// words are marked, how many blocks still wait.
    pub fn describe_spelling(&self, cx: &gpui::App) -> String {
        let Some(spell) = &self.spell else {
            return "none".to_string();
        };
        let spell = spell.read(cx);
        if !spell.is_enabled() {
            return "off".to_string();
        }
        let whose = if self.document_spelling_language(cx).is_some() {
            "document"
        } else {
            "app"
        };
        let caret_block = self.editor.read(cx).selection().head.block;
        let detected = spell
            .detected_language(caret_block, cx)
            .map(|language| format!(" detected={language}"))
            .unwrap_or_default();
        format!(
            "on language={} ({whose}){detected} marks={} pending={} active={}",
            spell.language().as_setting(),
            spell.misspelling_count(cx),
            spell.pending_blocks(cx),
            spell.is_active(cx),
        )
    }
}
