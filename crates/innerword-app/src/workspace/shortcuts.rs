//! Keyboard shortcuts in the workspace: where the user's changes are
//! stored, the list they are made in, the shortcuts file.
//!
//! The keymap itself is the app's ([`crate::keymap`]); the workspace is
//! where it meets the library and the window:
//!
//! - **Storage.** The changes are one setting of the library
//!   ([`keys::KEYBOARD_SHORTCUTS`]). [`Workspace::new`] reads it first of
//!   all and installs what it gives, so no key hint is ever drawn with the
//!   defaults of a key the user changed. Every change the user makes goes
//!   through [`Workspace::set_key_changes`], which installs and stores.
//! - **The list** is `innerword_keymap_ui::KeymapView`, a modal. The
//!   workspace hands it the keymap and the rows ([`crate::shortcuts`]) and
//!   carries out what it asks for ([`Workspace::on_shortcuts_request`]).
//! - **The file.** Export writes every command with its keys through the
//!   save panel; import reads a file chosen in the open panel, shows what
//!   it would change in the list, and carries it out when the user says
//!   how. After an import and after "reset all" a toast offers Undo.
//!
//! # A row is one key to the user
//!
//! A command can be bound in several key contexts that are one place to
//! the user (the find bar binds "case sensitive" for the bar and for the
//! field in it; the assistant's chats bind their keys in four). The list
//! has one row for them ([`crate::shortcuts::Shortcut::targets`]), and
//! what the user does to the row is done to every one of them.
//!
//! # What a test or a script cannot drive
//!
//! The native open panel. A scripted run names the file in a step
//! (`shortcuts:import:<path>`, which is [`Workspace::import_shortcuts_file`]),
//! and a test that clicks "Import…" names it beforehand
//! ([`choose_import_file_beforehand`]). The save panel is driven by tests
//! (`simulate_new_path_selection`) and replaced by
//! `INNER_WORD_EXPORT_DIR` in a scripted run, like every other export.

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use gpui::{App, AppContext as _, Context, Entity, Global, PromptLevel, Subscription, Window};
use innerword_keymap::{Catalogue, Changes, ImportMode, ImportPlan, Target};
use innerword_keymap_ui::{CommandRow, KeymapView, Listing, Request};
use innerword_library::Library;
use innerword_ui::Toast;

use super::Workspace;
use crate::{
    actions::*,
    keymap,
    settings::keys,
    shortcuts::{self, Shortcut},
};

/// What the list says above its rows about the keys it does not have.
pub const NOT_CHANGEABLE_NOTE: &str = "Not here, because they are not shortcuts: the keys of Vim mode, and the / that opens the \
     insert menu";

/// The rest of what stays as it is, in a line of its own: the keys an open
/// right-click menu reads itself (it has no key context a binding could
/// apply in, and its way out must not depend on the keymap), the insert
/// menu's own completion key, and what the pointer does with a key held.
pub const ALSO_FIXED_NOTE: &str = "Also fixed: the keys inside a right-click menu, ⇥ in the insert menu, clicks with ⌘, ⇧ or ⌃, and a whiteboard's own keys (listed On a whiteboard)";

/// What the heading of that question calls what sees a key first on a
/// whiteboard ("⌘D: the whiteboard sees it first"; after a colon, so in
/// lower case).
pub const BOARD_NAME: &str = "the whiteboard";

/// What the question about a key says for a key that is one of the
/// whiteboard's own keys and would work on a whiteboard.
pub const BOARD_TAKES_KEYS_FIRST: &str =
    "On a whiteboard the whiteboard uses this key itself, so there the shortcut does not run.";

/// Whether the whiteboard leaves `key` to the keymap: every key that is not
/// one of its own (`WHITEBOARD_KEYS`; the tool letters cannot be bound
/// where text is typed anyway).
fn board_leaves(key: &innerword_keymap::KeySequence) -> bool {
    !crate::boards::is_whiteboard_keys(&key.to_string())
}

/// What the heading of that question calls what sees the key first:
/// "⌃Y: Vim mode sees it first".
pub const VIM_MODE_NAME: &str = "Vim mode";

/// What the question about a key says while Vim mode is on, for a key
/// that would work in the text and that Vim does not certainly leave alone
/// ([`vim_leaves`]): Vim sees the keys of the text before any shortcut
/// does and keeps the ones it has a command on.
pub const VIM_TAKES_KEYS_FIRST: &str = "Vim mode is on, and in the text Vim sees this key before any shortcut does. Where Vim \
     has a command on it (in Normal and Visual mode it has one on many keys), the shortcut \
     does not run.";

/// Whether Vim mode leaves `key` to the keymap in every mode: a key with
/// ⌘ or fn in it, a function key, and ⌥ with a key (the engine has no
/// command on an option chord). Read from `VimMode::on_key`.
fn vim_leaves(key: &innerword_keymap::KeySequence) -> bool {
    let first = key.first();
    let modifiers = first.modifiers();
    let name = first.key();
    let function_key = name.len() > 1
        && name.starts_with('f')
        && name[1..]
            .chars()
            .all(|character| character.is_ascii_digit());
    modifiers.command || modifiers.function || function_key || (modifiers.alt && !modifiers.control)
}

/// How long the toast that offers Undo stays.
const UNDO_TOAST: Duration = Duration::from_secs(12);

/// A shortcuts file larger than this is not read: it is something else.
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;

/// The workspace's part of the keyboard shortcuts.
#[derive(Default)]
pub(crate) struct ShortcutsState {
    /// The changes as they were before the last import or "reset all".
    undo: Option<Changes>,
    /// What the open list asks for.
    subscription: Option<Subscription>,
    /// The stored value could not be read: it is left alone until the
    /// user changes a key, and the user is told once.
    unread: Option<String>,
    /// The toast that says what the last export, import, reset or undo
    /// did, while it is shown.
    toast: Option<innerword_ui::toast::ToastId>,
    /// The rows of the list changed without a change of a key (a part of
    /// the app was shown or hidden): an open list is handed them again
    /// when the window is drawn next.
    stale: bool,
}

impl ShortcutsState {
    /// The state of a new workspace. `unread`: what reading the stored
    /// changes said when it failed.
    pub(crate) fn new(unread: Option<String>) -> Self {
        Self {
            unread,
            ..Default::default()
        }
    }
}

/// The file an "Import…" takes without asking in the open panel.
#[derive(Default)]
struct ChosenBeforehand(Option<PathBuf>);

impl Global for ChosenBeforehand {}

/// Names the file the next "Import…" takes instead of asking in the open
/// panel, which neither a test nor a script can drive. `None` asks again.
pub fn choose_import_file_beforehand(path: Option<PathBuf>, cx: &mut App) {
    cx.set_global(ChosenBeforehand(path));
}

/// Reads the changes the library has stored and installs them. Returns
/// what to say when the stored value cannot be read (the default keys are
/// installed then).
pub(crate) fn install_stored_changes(library: &Entity<Library>, cx: &mut App) -> Option<String> {
    let stored: Option<serde_json::Value> = library
        .read(cx)
        .setting(keys::KEYBOARD_SHORTCUTS)
        .ok()
        .flatten();
    let stored = stored.map(|value| value.to_string());
    keymap::set_stored_changes(stored.as_deref(), cx).map(|error| error.to_string())
}

/// The row of the list for one row of [`crate::shortcuts`].
fn command_row(shortcut: &Shortcut) -> CommandRow {
    let place = shortcut.place.label();
    match (&shortcut.target, &shortcut.fixed) {
        (Some(target), _) => {
            let row = CommandRow::new(target.clone(), shortcut.title.clone(), place)
                .category(shortcut.category.clone())
                .heading(shortcut.heading.clone());
            // A key that only means something where it is pressed (moving
            // the caret) is not a command to run from the list.
            if shortcut.described {
                row
            } else {
                row.not_runnable()
            }
        }
        // Not a binding: a made-up context keeps it apart from the
        // command's real row.
        (None, reason) => CommandRow::new(
            Target::new(shortcut.action_name(), Some("typed")),
            shortcut.title.clone(),
            place,
        )
        .category(shortcut.category.clone())
        .heading(shortcut.heading.clone())
        .locked_with_keys(
            reason.clone().unwrap_or_default(),
            shortcut.typed.iter().cloned().collect(),
        )
        .not_runnable(),
    }
}

/// The names of every command in every place as the shortcuts file has
/// them: every row in the order of the list (each key context of a row
/// under the row's name), the rows of a part of the app that is not shown
/// among them, where they stand when it is shown. So the file has the
/// same entries in the same order whatever is shown on the Mac that
/// writes it, and two files can be compared.
pub fn file_catalogue(cx: &App) -> Catalogue {
    catalogue(&shortcuts::collect_every_shortcut(cx), &[], cx)
}

/// The names of every command in every place for the sentences of the
/// list: the rows it shows, then the commands that are not shown right
/// now, each with its place marked as such
/// ([`shortcuts::hidden_place_label`]). The list names a command from
/// here when a file that is imported changes its keys; of a command that
/// is not shown it must not read as if the user could find it.
pub fn list_catalogue(cx: &App) -> Catalogue {
    let hidden = shortcuts::hidden_targets(cx);
    catalogue(&shortcuts::collect_every_shortcut(cx), &hidden, cx)
}

/// `shortcuts` as a catalogue, the targets of `hidden` with their place
/// marked as not shown; then whatever the keymap has besides (nothing: a
/// test holds that every key of the keymap is in a row).
fn catalogue(shortcuts: &[Shortcut], hidden: &[Target], cx: &App) -> Catalogue {
    let mut catalogue = Catalogue::new();
    for shortcut in shortcuts {
        for target in &shortcut.targets {
            if hidden.contains(target) {
                let place = shortcuts::hidden_place_label(shortcut.place);
                catalogue.add(target.clone(), &shortcut.title, &place);
            } else {
                catalogue.add(target.clone(), &shortcut.title, shortcut.place.label());
            }
        }
    }
    let Some(model) = keymap::model(cx) else {
        return catalogue;
    };
    let registry = cx.try_global::<crate::commands::CommandRegistry>();
    for target in model.defaults().targets() {
        if catalogue.info(target).is_some() {
            continue;
        }
        let name = target.command.action.as_str();
        let title = registry
            .and_then(|registry| {
                registry
                    .commands()
                    .iter()
                    .find(|command| command.action_name() == name)
            })
            .map(|command| command.title.to_string())
            .unwrap_or_else(|| shortcuts::title_of_action(name));
        catalogue.add(target.clone(), &title, shortcuts::place_of(target).label());
    }
    catalogue
}

/// Everything the list shows: the keymap as it is, the rows, the names.
pub fn listing(cx: &App) -> Option<Listing> {
    let model = keymap::model(cx)?.clone();
    let shortcuts = shortcuts::collect_shortcuts(cx);
    let rows = shortcuts.iter().map(command_row).collect();
    Some(
        Listing::new(model, rows)
            .catalogue(list_catalogue(cx))
            .note(shortcuts::MENUS_NOTE)
            .note(NOT_CHANGEABLE_NOTE)
            .note(ALSO_FIXED_NOTE),
    )
}

/// How many commands have other keys than their defaults, as a person
/// counts them and as the list shows them: a command that is bound in
/// several key contexts of one place is one row there, and one command
/// here ([`keymap::changed_count`] counts every key context). What the
/// settings, the question of "reset all" and the state line of a scripted
/// run say.
pub fn changed_command_count(cx: &App) -> usize {
    listing(cx).map_or(0, |listing| listing.changed_commands())
}

/// "1 command", "4 commands": what every count of changed keyboard
/// shortcuts counts. A command is a row of the list, never a single key:
/// a command with two keys of the user's is one, and so is one left
/// without a key.
pub fn commands_counted(count: usize) -> String {
    if count == 1 {
        "1 command".to_string()
    } else {
        format!("{count} commands")
    }
}

/// What the row of the settings says after its description while the user
/// has keys of their own; `None` while every key is the default.
pub fn own_keys_sentence(changed: usize) -> Option<String> {
    match changed {
        0 => None,
        1 => Some("1 command has keys of your own.".to_string()),
        count => Some(format!(
            "{} have keys of your own.",
            commands_counted(count)
        )),
    }
}

/// What the system dialog of "Reset All Keyboard Shortcuts…" says under
/// its question: the sentence of the list's own question, so that the two
/// ways to reset say one thing.
pub fn reset_all_detail(changed: usize) -> String {
    match changed {
        // Only what this version cannot use is stored (a change for a
        // command it does not have).
        0 => "No command has other keys than its defaults. Changes this version cannot use are \
              stored, and are removed."
            .to_string(),
        1 => {
            "1 command goes back to the default keys. Every change you made is removed.".to_string()
        }
        count => format!(
            "{} go back to the default keys. Every change you made is removed.",
            commands_counted(count)
        ),
    }
}

/// What the toast after an import says.
pub fn imported_toast(changed: usize) -> String {
    format!("Imported the keys of {}", commands_counted(changed))
}

/// Every key context the row of `target` stands for (see the [module
/// docs](self)); `target` alone when it is not a row.
fn row_targets(target: &Target, cx: &App) -> Vec<Target> {
    shortcuts::collect_shortcuts(cx)
        .into_iter()
        .find(|shortcut| shortcut.targets.contains(target))
        .map(|shortcut| shortcut.targets)
        .unwrap_or_else(|| vec![target.clone()])
}

/// `request` for the other key contexts of its row: the same change, with
/// nothing more to take (the first one took it).
fn for_the_rest_of_the_row(request: &Request, cx: &App) -> Vec<Request> {
    let others = |target: &Target| -> Vec<Target> {
        row_targets(target, cx)
            .into_iter()
            .filter(|other| other != target)
            .collect()
    };
    match request {
        Request::ReplaceKey {
            target, old, new, ..
        } => others(target)
            .into_iter()
            .map(|target| Request::ReplaceKey {
                target,
                old: old.clone(),
                new: new.clone(),
                take: Vec::new(),
            })
            .collect(),
        Request::AddKey { target, new, .. } => others(target)
            .into_iter()
            .map(|target| Request::AddKey {
                target,
                new: new.clone(),
                take: Vec::new(),
            })
            .collect(),
        Request::RemoveKey { target, key } => others(target)
            .into_iter()
            .map(|target| Request::RemoveKey {
                target,
                key: key.clone(),
            })
            .collect(),
        Request::UseDefault { target } => others(target)
            .into_iter()
            .map(|target| Request::UseDefault { target })
            .collect(),
        _ => Vec::new(),
    }
}

impl Workspace {
    // ----- the changes -----------------------------------------------------------

    /// Puts `changes` in the place of the user's changes to the keyboard
    /// shortcuts: installs the keymap they give, stores them in the
    /// library and shows them in the list when it is open. Returns the
    /// changes as they were.
    pub fn set_key_changes(
        &mut self,
        changes: Changes,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Changes {
        let before = keymap::set_changes(changes, cx);
        self.store_key_changes(cx);
        self.refresh_shortcuts_list(window, cx);
        cx.notify();
        before
    }

    /// Changes the keymap through `change` (handed a copy of the keymap as
    /// data: `keymap.replace_key(..)`, `keymap.reset(..)`), then installs
    /// and stores the result. Returns what `change` returned.
    pub fn change_keys<R>(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut innerword_keymap::Keymap) -> R,
    ) -> Option<R> {
        let mut model = keymap::model(cx)?.clone();
        let result = change(&mut model);
        self.set_key_changes(model.changes().clone(), window, cx);
        Some(result)
    }

    /// Writes the changes that are installed into the library: one
    /// setting, removed again when every key is the default.
    fn store_key_changes(&mut self, cx: &mut Context<Self>) {
        let stored = keymap::model(cx)
            .and_then(|model| model.stored())
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok());
        // From now on the library holds what the user has: a value that
        // could not be read is not kept beside it.
        self.shortcuts.unread = None;
        let result = self.library.update(cx, |library, cx| match &stored {
            Some(value) => library.set_setting(keys::KEYBOARD_SHORTCUTS, value, cx),
            None => library
                .remove_setting(keys::KEYBOARD_SHORTCUTS, cx)
                .map(|_| ()),
        });
        if let Err(error) = result {
            self.toast_error(
                format!("Could not save the keyboard shortcuts: {error}"),
                cx,
            );
        }
    }

    /// Says what was done to the keyboard shortcuts, in the place of what
    /// was said about them last. One toast at a time: the list stays open
    /// under these, a stack of them would lie over its lower part, and of
    /// two toasts with "Undo" only the later one would say what its button
    /// undoes.
    fn say_about_shortcuts(&mut self, toast: Toast, cx: &mut Context<Self>) {
        let last = self.shortcuts.toast.take();
        self.shortcuts.toast = Some(self.toast_layer.update(cx, |layer, cx| {
            if let Some(last) = last {
                layer.dismiss(last, cx);
            }
            layer.push(toast, cx)
        }));
    }

    /// Says once that the stored shortcuts could not be read (called when
    /// the workspace is built).
    pub(crate) fn report_unread_shortcuts(&mut self, cx: &mut Context<Self>) {
        if let Some(problem) = self.shortcuts.unread.clone() {
            self.toast_error(
                format!(
                    "{problem} The default keys are used; nothing is lost until a key is changed."
                ),
                cx,
            );
        }
    }

    /// How many commands have other keys than their defaults, as the list
    /// counts them ([`changed_command_count`]).
    pub fn changed_key_count(&self, cx: &App) -> usize {
        changed_command_count(cx)
    }

    // ----- the list --------------------------------------------------------------

    /// The open list of keyboard shortcuts.
    pub fn shortcuts_list(&self, cx: &App) -> Option<Entity<KeymapView>> {
        self.modal_layer.read(cx).active::<KeymapView>()
    }

    /// Opens "Keyboard shortcuts", or closes it when it is open.
    pub fn toggle_shortcuts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.shortcuts_list(cx).is_some() {
            self.dismiss_modal(window, cx);
            self.shortcuts.subscription = None;
            return;
        }
        self.show_shortcuts(window, cx);
    }

    /// Opens "Keyboard shortcuts" when it is not open, and returns it.
    pub fn show_shortcuts(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<KeymapView>> {
        if let Some(list) = self.shortcuts_list(cx) {
            return Some(list);
        }
        let listing = self.shortcuts_listing(cx)?;
        self.modal_layer.update(cx, |layer, cx| {
            layer.toggle(window, cx, |window, cx| {
                KeymapView::new(listing, window, cx)
            });
        });
        let list = self.shortcuts_list(cx)?;
        self.shortcuts.subscription =
            Some(cx.subscribe_in(&list, window, Self::on_shortcuts_request));
        Some(list)
    }

    /// The keymap changed: the open list shows it.
    fn refresh_shortcuts_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(list) = self.shortcuts_list(cx) else {
            return;
        };
        if let Some(listing) = self.shortcuts_listing(cx) {
            list.update(cx, |list, cx| list.set_listing(listing, window, cx));
        }
    }

    /// The rows of the list are other rows now, with no key changed (the
    /// assistant was shown or hidden): an open list follows when the
    /// window is drawn next ([`Workspace::refresh_a_stale_shortcuts_list`]).
    pub(crate) fn shortcuts_rows_changed(&mut self, cx: &mut Context<Self>) {
        self.shortcuts.stale = true;
        cx.notify();
    }

    /// Hands an open list its rows again when they changed behind it.
    /// Called as the window is drawn.
    pub(crate) fn refresh_a_stale_shortcuts_list(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !std::mem::take(&mut self.shortcuts.stale) || self.shortcuts_list(cx).is_none() {
            return;
        }
        cx.defer_in(window, |this, window, cx| {
            this.refresh_shortcuts_list(window, cx)
        });
    }

    /// [`listing`] as this window's list shows it: with what it says about
    /// a key while Vim mode is on ([`VIM_TAKES_KEYS_FIRST`]).
    fn shortcuts_listing(&self, cx: &App) -> Option<Listing> {
        // The whiteboard sees its own keys first, always.
        let listing = listing(cx)?.sees_keys_first(
            BOARD_NAME,
            [crate::boards::BOARD_VIEW],
            board_leaves,
            BOARD_TAKES_KEYS_FIRST,
        );
        Some(if self.vim.read(cx).is_attached() {
            listing.sees_keys_first(
                VIM_MODE_NAME,
                [
                    innerword_editor::actions::KEY_CONTEXT,
                    innerword_editor_tools::actions::KEY_CONTEXT,
                ],
                vim_leaves,
                VIM_TAKES_KEYS_FIRST,
            )
        } else {
            listing
        })
    }

    /// What the list asks for.
    pub(crate) fn on_shortcuts_request(
        &mut self,
        _list: &Entity<KeymapView>,
        request: &Request,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match request {
            Request::Export => self.export_shortcuts(window, cx),
            Request::ChooseImportFile => self.choose_shortcuts_file(window, cx),
            Request::Run { target } => {
                if let Some(action) = keymap::action_named(&target.command.action, cx) {
                    self.shortcuts.subscription = None;
                    self.run_action(action, window, cx);
                }
            }
            Request::Import { plan, mode } => self.carry_out_import(plan, *mode, window, cx),
            Request::ResetAll => self.reset_all_shortcuts_now(window, cx),
            change => {
                // The same change in every key context of the row. And a
                // key taken from a command is taken from its whole row:
                // to the user that command had one key there.
                let rest = for_the_rest_of_the_row(change, cx);
                let taken: Vec<(Target, innerword_keymap::KeySequence)> = match change {
                    Request::ReplaceKey { take, .. } | Request::AddKey { take, .. } => take
                        .iter()
                        .flat_map(|holder| {
                            row_targets(&holder.target, cx)
                                .into_iter()
                                .filter(|other| *other != holder.target)
                                .map(|other| (other, holder.keys.clone()))
                        })
                        .collect(),
                    _ => Vec::new(),
                };
                self.change_keys(window, cx, |model| {
                    change.apply_to(model);
                    for (target, key) in &taken {
                        model.remove_key(target, key);
                    }
                    for request in &rest {
                        request.apply_to(model);
                    }
                });
            }
        }
    }

    // ----- reset and undo --------------------------------------------------------

    /// "Reset all keyboard shortcuts" from the palette or the menu bar:
    /// asks in a system dialog, which works whatever the keys of the app
    /// do, and then puts every key back.
    pub fn reset_all_shortcuts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let changed = self.changed_key_count(cx);
        let stored = !keymap::changes(cx).is_empty();
        if changed == 0 && !stored {
            self.say_about_shortcuts(
                Toast::new("Every keyboard shortcut is the default already"),
                cx,
            );
            return;
        }
        let detail = reset_all_detail(changed);
        let answer = window.prompt(
            PromptLevel::Warning,
            "Put every keyboard shortcut back to its default?",
            Some(&detail),
            &["Reset All", "Cancel"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            if answer.await == Ok(0) {
                this.update_in(cx, |this, window, cx| {
                    this.reset_all_shortcuts_now(window, cx)
                })
                .ok();
            }
        })
        .detach();
    }

    /// Puts every key back to its default, without asking, and offers
    /// Undo.
    pub fn reset_all_shortcuts_now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(before) = self.change_keys(window, cx, |model| model.reset_all()) else {
            return;
        };
        if before == keymap::changes(cx) {
            return;
        }
        self.shortcuts.undo = Some(before);
        self.say_about_shortcuts(
            Toast::new("Every keyboard shortcut is back to its default")
                .success()
                .action("Undo", UndoShortcutsChange)
                .duration(UNDO_TOAST),
            cx,
        );
    }

    /// Puts the shortcuts back as they were before the last import or
    /// "reset all".
    pub fn undo_shortcuts_change(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.shortcuts.undo.take() {
            Some(before) => {
                self.set_key_changes(before, window, cx);
                self.say_about_shortcuts(
                    Toast::new("The keyboard shortcuts are as they were before").success(),
                    cx,
                );
            }
            None => self.say_about_shortcuts(
                Toast::new("No import or reset of the keyboard shortcuts to undo"),
                cx,
            ),
        }
    }

    /// Whether there is an import or a reset to undo.
    pub fn can_undo_shortcuts_change(&self) -> bool {
        self.shortcuts.undo.is_some()
    }

    // ----- export ----------------------------------------------------------------

    /// The shortcuts file for the keymap as it is: every command in every
    /// place with its name, its default keys and the keys it has.
    pub fn shortcuts_file(&self, cx: &App) -> Option<String> {
        let model = keymap::model(cx)?;
        Some(model.export(&file_catalogue(cx), env!("CARGO_PKG_VERSION")))
    }

    /// Writes the shortcuts file where the user says in the save panel (a
    /// scripted run: into `INNER_WORD_EXPORT_DIR`). The outcome is
    /// reported in a toast.
    pub fn export_shortcuts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = self.shortcuts_file(cx) else {
            return;
        };
        let name = innerword_keymap::FILE_NAME;
        let scripted = crate::automation::export_directory();
        let receiver = scripted.is_none().then(|| {
            let directory = self.export_directory(cx);
            cx.prompt_for_new_path(&directory, Some(name))
        });
        cx.spawn_in(window, async move |this, cx| {
            let chosen = match (scripted, receiver) {
                (Some(directory), _) => directory.join(name),
                (None, Some(receiver)) => match receiver.await {
                    Ok(Ok(Some(path))) => path,
                    // Cancelled.
                    Ok(Ok(None)) | Err(_) => return,
                    Ok(Err(error)) => {
                        this.update(cx, |this, cx| {
                            this.toast_error(format!("Could not open the save panel: {error}"), cx);
                        })
                        .ok();
                        return;
                    }
                },
                (None, None) => return,
            };
            // The save panel lets any name be typed: the file is JSON.
            let path = match chosen.extension() {
                Some(_) => chosen,
                None => chosen.with_extension("json"),
            };
            let written = cx
                .background_spawn({
                    let path = path.clone();
                    async move { std::fs::write(&path, text) }
                })
                .await;
            this.update(cx, |this, cx| match written {
                Ok(()) => {
                    if let Some(parent) = path.parent() {
                        let parent = parent.to_string_lossy().to_string();
                        this.library
                            .update(cx, |library, cx| {
                                library.set_setting(keys::EXPORT_DIRECTORY, &parent, cx)
                            })
                            .ok();
                    }
                    this.say_about_shortcuts(
                        // A path that fits: `~` for the home directory, a
                        // long one shortened in the middle, the file name
                        // whole.
                        Toast::new(format!(
                            "Exported the keyboard shortcuts to {}",
                            crate::export::short_path(&path)
                        ))
                        .success(),
                        cx,
                    );
                }
                Err(error) => this.toast_error(
                    format!(
                        "Could not write {}: {error}",
                        crate::export::short_path(&path)
                    ),
                    cx,
                ),
            })
            .ok();
        })
        .detach();
    }

    // ----- import ----------------------------------------------------------------

    /// Lets the user choose a shortcuts file in the open panel, then shows
    /// what importing it would do ([`Workspace::import_shortcuts_file`]).
    pub fn choose_shortcuts_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let beforehand = cx
            .try_global::<ChosenBeforehand>()
            .and_then(|chosen| chosen.0.clone());
        if let Some(path) = beforehand {
            cx.set_global(ChosenBeforehand(None));
            self.import_shortcuts_file(&path, window, cx);
            return;
        }
        let receiver = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Import".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let path = match receiver.await {
                Ok(Ok(Some(paths))) => paths.into_iter().next(),
                Ok(Ok(None)) | Err(_) => None,
                Ok(Err(error)) => {
                    this.update(cx, |this, cx| {
                        this.toast_error(format!("Could not open the file panel: {error}"), cx);
                    })
                    .ok();
                    None
                }
            };
            if let Some(path) = path {
                this.update_in(cx, |this, window, cx| {
                    this.import_shortcuts_file(&path, window, cx)
                })
                .ok();
            }
        })
        .detach();
    }

    /// Reads the shortcuts file at `path` and shows, in the list, what
    /// importing it would change: nothing is applied until the user says
    /// how (replace their own changes, add to them) there. A file that is
    /// not a shortcuts file is said to be so, with what is wrong and
    /// where.
    pub fn import_shortcuts_file(
        &mut self,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let shown = crate::export::display_path(path);
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| shown.clone());
        let read = std::fs::metadata(path).and_then(|metadata| {
            if metadata.len() > MAX_FILE_BYTES {
                return Err(std::io::Error::other(
                    "it is too large to be a shortcuts file",
                ));
            }
            std::fs::read(path)
        });
        let bytes = match read {
            Ok(bytes) => bytes,
            Err(error) => {
                // The name, not the path: a toast has two lines, and the
                // reason has to be on them.
                self.toast_error(
                    format!("Could not read \u{201c}{name}\u{201d}: {error}"),
                    cx,
                );
                return;
            }
        };
        // A file that is not text is not JSON either: the reader says so.
        let text = String::from_utf8_lossy(&bytes);
        let Some(plan) = keymap::model(cx).map(|model| model.plan_import(&text)) else {
            return;
        };
        let Some(list) = self.show_shortcuts(window, cx) else {
            return;
        };
        list.update(cx, |list, cx| match plan {
            Ok(plan) => list.show_import_preview(name, plan, window, cx),
            Err(error) => list.show_import_error(name, &error, window, cx),
        });
    }

    /// Carries out an import the user looked at, and offers Undo.
    fn carry_out_import(
        &mut self,
        plan: &ImportPlan,
        mode: ImportMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // As the preview counted them: a command the file changes in
        // several key contexts of one place is one command.
        let differences = &plan.outcome(mode).differences;
        let changed = listing(cx).map_or(differences.len(), |listing| {
            listing.commands_among(differences.iter().map(|difference| &difference.target))
        });
        let carried = plan.carry_out(mode);
        let before = self.set_key_changes(carried.changes, window, cx);
        if before == keymap::changes(cx) {
            self.say_about_shortcuts(Toast::new("The import changed no keyboard shortcut"), cx);
            return;
        }
        self.shortcuts.undo = Some(before);
        let what = imported_toast(changed);
        self.say_about_shortcuts(
            Toast::new(what)
                .success()
                .action("Undo", UndoShortcutsChange)
                .duration(UNDO_TOAST),
            cx,
        );
    }

    // ----- for scripted runs -----------------------------------------------------

    /// One part of the `state` line of a scripted run: how many commands
    /// have other keys than their defaults, and what the list shows while
    /// it is open.
    pub fn describe_shortcuts(&self, cx: &App) -> String {
        let mut text = format!("changed={}", self.changed_key_count(cx));
        if let Some(inactive) = keymap::effective(cx)
            .map(|effective| effective.inactive().len())
            .filter(|count| *count > 0)
        {
            text.push_str(&format!(" inactive={inactive}"));
        }
        if self.can_undo_shortcuts_change() {
            text.push_str(" undo=yes");
        }
        if let Some(list) = self.shortcuts_list(cx) {
            text.push_str(&format!(" list=[{}]", list.read(cx).describe(cx)));
        }
        text
    }

    /// Gives the command whose action is named `action` the keys `keys`
    /// and no others, without the list (`None`: its default keys). In the
    /// place the list names the command first, or in the place called
    /// `place` ("In the find bar"). For scripted runs and tests of other
    /// things; no guard rail is asked.
    pub fn set_keys_of_command(
        &mut self,
        action: &str,
        place: Option<&str>,
        keys: Option<&[innerword_keymap::KeySequence]>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let row = shortcuts::collect_shortcuts(cx)
            .into_iter()
            .filter(|shortcut| shortcut.fixed.is_none())
            .find(|shortcut| {
                shortcut.action_name() == action
                    && place.is_none_or(|place| shortcut.place.label().eq_ignore_ascii_case(place))
            })
            .ok_or_else(|| match place {
                Some(place) => format!("no command {action:?} is listed {place:?}"),
                None => format!("no command {action:?} is listed"),
            })?;
        self.change_keys(window, cx, |model| {
            for target in &row.targets {
                match keys {
                    Some(keys) => model.set_keys(target, keys),
                    None => model.reset(target),
                };
            }
        });
        Ok(())
    }

    // ----- action handlers ---------------------------------------------------------

    pub(crate) fn on_import_shortcuts(
        &mut self,
        _: &ImportShortcuts,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.choose_shortcuts_file(window, cx);
    }

    pub(crate) fn on_export_shortcuts(
        &mut self,
        _: &ExportShortcuts,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.export_shortcuts(window, cx);
    }

    pub(crate) fn on_reset_all_shortcuts(
        &mut self,
        _: &ResetAllShortcuts,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.reset_all_shortcuts(window, cx);
    }

    pub(crate) fn on_undo_shortcuts_change(
        &mut self,
        _: &UndoShortcutsChange,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.undo_shortcuts_change(window, cx);
    }
}
