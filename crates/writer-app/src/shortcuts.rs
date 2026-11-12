//! The keyboard shortcuts overlay (`cmd-/`).
//!
//! Generated, not written: the rows are the registered command descriptors
//! that have a binding in GPUI's keymap, followed by the keys that have no
//! descriptor (caret movement, selection and deletion in the text; moving
//! around the find bar, the comment list, the library, the pickers and the
//! settings), named after their actions. The key shown is whatever the
//! keymap says right now.
//!
//! A key only works where its binding applies, so the list is grouped by
//! where the keyboard has to be ([`Place`], read from the key context of
//! the binding): "Anywhere" first, then "In the text", then the panels and
//! overlays. Within a place the commands keep their categories; a heading
//! reads "In the text · Format". The list is searchable, and enter runs the
//! selected command.
//!
//! Two things the keymap cannot tell are added: the insert menu, which is
//! opened by typing `/` and not by a binding ([`TYPED_TRIGGERS`]), and a
//! first line that says the same commands are in the menus
//! ([`MENUS_NOTE`]), for whoever would rather click.

use std::collections::HashSet;

use gpui::{Action, App, Context, SharedString, WeakEntity, Window};
use writer_ui::{KeyHint, Picker, PickerDelegate, PickerEntry, PickerItem, fuzzy::fuzzy_match};

use crate::{commands, workspace::Workspace};

/// The shortcuts overlay as a modal view.
pub type ShortcutsOverlay = Picker<ShortcutsDelegate>;

/// Action namespaces whose bindings are listed even without a command
/// descriptor, with the category they are listed under (empty: the place
/// says it all).
const KEY_GROUPS: [(&str, &str); 9] = [
    ("editor", "Moving and selecting"),
    ("editor_tools", ""),
    ("comments", ""),
    ("library_panel", ""),
    ("picker", ""),
    ("settings", ""),
    ("version_history", ""),
    ("agents", ""),
    // `ctrl-enter` (and `shift-f10`) in the library, the comment list and a
    // text field.
    ("context_menu", ""),
];

/// Commands that are started by typing a character, not by a key binding,
/// so the keymap does not know them: the action, the character, and where
/// it has to be typed. They are listed like the others.
const TYPED_TRIGGERS: [(&str, &str, &str); 1] = [(
    "editor_tools::OpenInsertMenu",
    "/",
    "at the start of a line or after a space",
)];

/// Actions whose row names every key they are bound to in their place, not
/// only the preferred one ("Context menu (also ⇧F10)"): the context menu
/// opens on `ctrl-enter` and, should the system keep that key, on
/// `shift-f10`. (Other actions have second bindings that are the same key
/// spelled another way, `cmd-shift-7` and `cmd-&`; those are not named.)
const EVERY_KEY: [&str; 2] = ["editor::OpenContextMenu", "context_menu::OpenContextMenu"];

/// What the overlay says above the list: the keys are one way, the menus
/// are the other.
pub const MENUS_NOTE: &str = "Every command is also in a menu: right-click, or the menu bar";

/// Where the keyboard has to be for a shortcut to work. In the order the
/// overlay lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Place {
    /// Wherever the keyboard is in the window.
    Anywhere,
    /// In the text of the document.
    Text,
    /// In the find bar.
    FindBar,
    /// In the link field.
    LinkField,
    /// In the list of comment threads.
    CommentList,
    /// In the field a comment or a reply is written in.
    CommentField,
    /// In the library sidebar.
    Library,
    /// In the trash view of the library.
    Trash,
    /// In a list with a search field: the palette, a picker, a menu.
    List,
    /// In the settings.
    Settings,
    /// In the version history.
    VersionHistory,
    /// In the Agents panel.
    Agents,
    /// In a dialog.
    Dialog,
    /// In a text field.
    TextField,
    /// Somewhere this list does not know by name.
    Elsewhere,
}

impl Place {
    /// How the overlay names the place.
    pub fn label(self) -> &'static str {
        match self {
            Place::Anywhere => "Anywhere",
            Place::Text => "In the text",
            Place::FindBar => "In the find bar",
            Place::LinkField => "In the link field",
            Place::CommentList => "In the comment list",
            Place::CommentField => "Writing a comment",
            Place::Library => "In the library",
            Place::Trash => "In the trash",
            Place::List => "In a list or menu",
            Place::Settings => "In settings",
            Place::VersionHistory => "In the version history",
            Place::Agents => "In the agents panel",
            Place::Dialog => "In a dialog",
            Place::TextField => "In a text field",
            Place::Elsewhere => "Elsewhere",
        }
    }

    /// The place a binding with the key context predicate `predicate`
    /// works in (`None`: the binding has no context).
    pub fn of_predicate(predicate: Option<&str>) -> Place {
        let Some(predicate) = predicate else {
            return Place::Anywhere;
        };
        let has = |name: &str| predicate.contains(name);
        if has("FindBar") {
            Place::FindBar
        } else if has("LinkPopover") {
            Place::LinkField
        } else if has("CommentComposer") {
            Place::CommentField
        } else if has("CommentThreads") || has("CommentsPanel") {
            Place::CommentList
        } else if has("LibraryPanel") {
            if has("mode == trash") {
                Place::Trash
            } else {
                Place::Library
            }
        } else if has("SettingsView") || has("SettingsRow") {
            Place::Settings
        } else if has("VersionHistory") {
            Place::VersionHistory
        } else if has("AgentsView") {
            Place::Agents
        } else if has("Picker") {
            Place::List
        } else if [
            "ConfirmDialog",
            "Button",
            "Toggle",
            "SegmentedControl",
            "TabNavigation",
        ]
        .into_iter()
        .any(has)
        {
            Place::Dialog
        } else if has("EditorTools") || has("Editor") || has("vim_mode") {
            Place::Text
        } else if has("TextField") {
            Place::TextField
        } else if has("Workspace") {
            Place::Anywhere
        } else {
            Place::Elsewhere
        }
    }

    /// The command category that needs no mention beside this place.
    fn own_category(self) -> Option<&'static str> {
        match self {
            Place::Library => Some("Library"),
            Place::CommentList => Some("Comments"),
            _ => None,
        }
    }

    /// The heading of the commands of `category` in this place.
    pub fn heading(self, category: &str) -> String {
        if category.is_empty() || self.own_category() == Some(category) {
            self.label().to_string()
        } else {
            format!("{} \u{b7} {category}", self.label())
        }
    }
}

/// One shortcut.
pub struct Shortcut {
    /// What it does.
    pub title: SharedString,
    /// The category of its command.
    pub category: SharedString,
    /// Where the keyboard has to be.
    pub place: Place,
    /// The heading it is listed under: place and category.
    pub heading: SharedString,
    /// The keys, as shown.
    pub keys: String,
    action: Box<dyn Action>,
    hint: KeyHint,
}

/// The binding to list for `action`, as a key hint and the place it works
/// in. An action can be bound in several places (undo in the text and in
/// the comment list); it is listed once, in the widest of them (the order
/// of [`Place`]), and among the bindings there with the one the keymap
/// prefers, which is the one added last.
fn preferred_binding(action: &dyn Action, cx: &App) -> Option<(KeyHint, Place)> {
    let keymap = cx.key_bindings();
    let keymap = keymap.borrow();
    let mut best: Option<(&gpui::KeyBinding, Place)> = None;
    for binding in keymap.bindings_for_action(action) {
        let place = binding_place(binding);
        if best.is_none_or(|(_, best_place)| place <= best_place) {
            best = Some((binding, place));
        }
    }
    best.map(|(binding, place)| (KeyHint::from_binding(binding), place))
}

/// `title` with the other keys `action` is bound to in `place`, for the
/// actions of [`EVERY_KEY`]: every binding there whose key is not
/// `preferred`, once each, in the order the keymap has them, as the keymap
/// spells them.
fn with_second_keys(
    title: SharedString,
    action: &dyn Action,
    place: Place,
    preferred: &KeyHint,
    cx: &App,
) -> SharedString {
    if !EVERY_KEY.contains(&action.name()) {
        return title;
    }
    let keymap = cx.key_bindings();
    let keymap = keymap.borrow();
    let mut others: Vec<String> = Vec::new();
    for binding in keymap.bindings_for_action(action) {
        if binding_place(binding) != place {
            continue;
        }
        let text = KeyHint::from_binding(binding).text();
        if text != preferred.text() && !others.contains(&text) {
            others.push(text);
        }
    }
    if others.is_empty() {
        title
    } else {
        format!("{title} (also {})", others.join(", ")).into()
    }
}

fn binding_place(binding: &gpui::KeyBinding) -> Place {
    let predicate = binding.predicate().map(|predicate| predicate.to_string());
    Place::of_predicate(predicate.as_deref())
}

#[derive(Clone, Debug, PartialEq)]
enum Row {
    Header(SharedString),
    Shortcut { index: usize, positions: Vec<usize> },
}

/// The picker delegate behind the shortcuts overlay.
pub struct ShortcutsDelegate {
    workspace: WeakEntity<Workspace>,
    shortcuts: Vec<Shortcut>,
    rows: Vec<Row>,
}

/// `MoveWordLeft` as `Move word left`.
pub fn humanize_action_name(name: &str) -> String {
    let short = name.rsplit("::").next().unwrap_or(name);
    let mut out = String::with_capacity(short.len() + 4);
    let mut previous: Option<char> = None;
    for ch in short.chars() {
        let boundary = previous.is_some_and(|previous| {
            (ch.is_uppercase() && !previous.is_uppercase())
                || (ch.is_ascii_digit() && !previous.is_ascii_digit())
        });
        if boundary {
            out.push(' ');
        }
        if previous.is_none() {
            out.extend(ch.to_uppercase());
        } else {
            out.extend(ch.to_lowercase());
        }
        previous = Some(ch);
    }
    out
}

/// A title made from an action name, in the words the rest of the app
/// uses: the code says "thread" where the interface says "comment".
fn plain_words(title: &str) -> String {
    title.replace(" thread", " comment")
}

/// Every shortcut, in the order the overlay lists them: by place, within a
/// place by heading (the commands' categories in registration order, then
/// the undescribed keys of the [`KEY_GROUPS`] namespaces).
pub fn collect_shortcuts(cx: &App) -> Vec<Shortcut> {
    let mut shortcuts = Vec::new();
    let mut described: HashSet<&'static str> = HashSet::new();
    let all = commands::all(cx);
    for category in commands::categories(&all) {
        for command in all.iter().filter(|command| command.category == category) {
            described.insert(command.action_name());
            if let Some((hint, place)) = preferred_binding(command.action_ref(), cx) {
                let title = with_second_keys(
                    command.title.clone(),
                    command.action_ref(),
                    place,
                    &hint,
                    cx,
                );
                shortcuts.push(Shortcut {
                    title,
                    category: category.clone(),
                    place,
                    heading: place.heading(&category).into(),
                    keys: hint.text(),
                    action: command.action(),
                    hint,
                });
            } else if let Some((_, typed, whereabouts)) = TYPED_TRIGGERS
                .iter()
                .find(|(action, _, _)| *action == command.action_name())
                && let Some(hint) = KeyHint::parse(typed)
            {
                let place = Place::Text;
                shortcuts.push(Shortcut {
                    title: format!("{} ({whereabouts})", command.title).into(),
                    category: category.clone(),
                    place,
                    heading: place.heading(&category).into(),
                    keys: hint.text(),
                    action: command.action(),
                    hint,
                });
            }
        }
    }

    // Keys without a descriptor, straight from the keymap, one row per
    // action, in the order the actions were first bound.
    let keymap = cx.key_bindings();
    let keymap = keymap.borrow();
    let mut undescribed: Vec<Box<dyn Action>> = Vec::new();
    for (namespace, category) in KEY_GROUPS {
        let prefix = format!("{namespace}::");
        let mut seen: HashSet<&'static str> = HashSet::new();
        for binding in keymap.bindings() {
            let name = binding.action().name();
            if name.starts_with(&prefix) && !described.contains(name) && seen.insert(name) {
                undescribed.push(binding.action().boxed_clone());
            }
        }
        for action in undescribed.drain(..) {
            let Some((hint, place)) = preferred_binding(action.as_ref(), cx) else {
                continue;
            };
            let title = with_second_keys(
                plain_words(&humanize_action_name(action.name())).into(),
                action.as_ref(),
                place,
                &hint,
                cx,
            );
            shortcuts.push(Shortcut {
                title,
                category: category.into(),
                place,
                heading: place.heading(category).into(),
                keys: hint.text(),
                action,
                hint,
            });
        }
    }
    drop(keymap);

    // By place, then by heading in the order the headings first appeared;
    // rows keep their order within a heading.
    let mut headings: Vec<SharedString> = Vec::new();
    for shortcut in &shortcuts {
        if !headings.contains(&shortcut.heading) {
            headings.push(shortcut.heading.clone());
        }
    }
    let rank = |shortcut: &Shortcut| {
        let heading = headings
            .iter()
            .position(|heading| *heading == shortcut.heading)
            .unwrap_or(usize::MAX);
        (shortcut.place, heading)
    };
    shortcuts.sort_by_key(rank);
    shortcuts
}

impl ShortcutsDelegate {
    /// Collects the shortcuts from the registry and the keymap.
    pub fn new(workspace: WeakEntity<Workspace>, cx: &App) -> Self {
        Self {
            workspace,
            shortcuts: collect_shortcuts(cx),
            rows: Vec::new(),
        }
    }

    /// The listed shortcuts, in order (headers excluded).
    pub fn listed(&self) -> Vec<&Shortcut> {
        self.rows
            .iter()
            .filter_map(|row| match row {
                Row::Shortcut { index, .. } => Some(&self.shortcuts[*index]),
                Row::Header(_) => None,
            })
            .collect()
    }
}

impl PickerDelegate for ShortcutsDelegate {
    fn update_matches(&mut self, query: &str, _: &mut Window, _: &mut Context<Picker<Self>>) {
        self.rows.clear();
        if query.trim().is_empty() {
            self.rows.push(Row::Header(MENUS_NOTE.into()));
            let mut heading: Option<&SharedString> = None;
            for (index, shortcut) in self.shortcuts.iter().enumerate() {
                if heading != Some(&shortcut.heading) {
                    heading = Some(&shortcut.heading);
                    self.rows.push(Row::Header(shortcut.heading.clone()));
                }
                self.rows.push(Row::Shortcut {
                    index,
                    positions: Vec::new(),
                });
            }
            return;
        }
        let mut matches: Vec<(i32, usize, Vec<usize>)> = self
            .shortcuts
            .iter()
            .enumerate()
            .filter_map(|(index, shortcut)| {
                if let Some(found) = fuzzy_match(query, &shortcut.title) {
                    return Some((found.score + 10_000, index, found.positions));
                }
                let haystack = format!(
                    "{} {} {}",
                    shortcut.heading, shortcut.category, shortcut.title
                );
                crate::palette::words_match(query, &haystack).then(|| (0, index, Vec::new()))
            })
            .collect();
        matches.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        self.rows = matches
            .into_iter()
            .map(|(_, index, positions)| Row::Shortcut { index, positions })
            .collect();
    }

    fn entry_count(&self) -> usize {
        self.rows.len()
    }

    fn entry(&self, ix: usize, _: &App) -> PickerEntry {
        match &self.rows[ix] {
            Row::Header(title) => PickerEntry::Header(title.clone()),
            Row::Shortcut { index, positions } => {
                let shortcut = &self.shortcuts[*index];
                let item = PickerItem::new(shortcut.title.clone())
                    .positions(positions.clone())
                    .key_hint(shortcut.hint.clone());
                // Headers already say where and what while browsing.
                if matches!(self.rows.first(), Some(Row::Header(_))) {
                    item.into()
                } else {
                    item.detail(shortcut.heading.clone()).into()
                }
            }
        }
    }

    fn is_selectable(&self, ix: usize, _: &App) -> bool {
        matches!(self.rows.get(ix), Some(Row::Shortcut { .. }))
    }

    fn confirm(
        &mut self,
        ix: usize,
        _secondary: bool,
        window: &mut Window,
        cx: &mut Context<Picker<Self>>,
    ) -> bool {
        let Some(Row::Shortcut { index, .. }) = self.rows.get(ix) else {
            return false;
        };
        let action = self.shortcuts[*index].action.boxed_clone();
        let workspace = self.workspace.clone();
        window.defer(cx, move |window, cx| {
            workspace
                .update(cx, |workspace, cx| workspace.run_action(action, window, cx))
                .ok();
        });
        false
    }

    fn placeholder(&self) -> SharedString {
        "Search keyboard shortcuts".into()
    }

    fn empty_text(&self, _query: &str) -> SharedString {
        "No shortcuts match".into()
    }
}
