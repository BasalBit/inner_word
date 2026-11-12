//! The rows of "Keyboard shortcuts" (`cmd-/`): every command in every
//! place it has a key, or can be given one.
//!
//! Generated, not written: the rows are the registered command descriptors
//! followed by the keys that have no descriptor (caret movement, selection
//! and deletion in the text; moving around the find bar, the comment list,
//! the library, the pickers and the settings), named after their actions.
//! The keys shown are whatever the keymap says right now: the rows are made
//! from what [`crate::keymap`] has installed, the defaults with the user's
//! changes.
//!
//! A key only works where its binding applies, so the list is grouped by
//! where the keyboard has to be ([`Place`], read from the key context of
//! the binding): "Anywhere" first, then "In the text", then the panels and
//! overlays. Within a place the commands keep their categories; a heading
//! reads "In the text · Format". A command with keys in several places
//! (undo, in the text and in the comment list) has a row in each, because
//! each can be given another key. A command without a key has a row too,
//! in the place a key for it would apply.
//!
//! A row is what the user changes ([`Shortcut::targets`]: the command in
//! every key context of the row's place; a change of the row is made to
//! all of them, see `workspace/shortcuts.rs`).
//!
//! Two things the keymap cannot tell are added: the insert menu, which is
//! opened by typing `/` and not by a binding ([`TYPED_TRIGGERS`]), and a
//! first line that says the same commands are in the menus
//! ([`MENUS_NOTE`]), for whoever would rather click.

use std::collections::HashSet;

use gpui::{Action, App, SharedString};
use writer_keymap::{KeySequence, Target, TargetKey, TargetKeys};
use writer_ui::KeyHint;

use crate::commands;

/// Action namespaces whose bindings are listed even without a command
/// descriptor, with the category they are listed under (empty: the place
/// says it all).
const KEY_GROUPS: [(&str, &str); 18] = [
    ("editor", "Moving and selecting"),
    ("editor_tools", ""),
    ("comments", ""),
    ("library_panel", ""),
    ("picker", ""),
    ("settings", ""),
    ("version_history", ""),
    ("agents", ""),
    // The chats of the assistant: listed only while the assistant is shown
    // (see `ASSISTANT_NAMESPACES`).
    ("assistant", ""),
    ("quick_chat", ""),
    // `ctrl-enter` (and `shift-f10`) in the library, the comment list and a
    // text field.
    ("context_menu", ""),
    // The keys of a text field, and of the small parts of a dialog.
    ("text_field", ""),
    ("modal", ""),
    ("confirm_dialog", ""),
    ("button", ""),
    ("segmented", ""),
    ("focus", ""),
    // The keys of this list itself.
    ("keymap_view", ""),
];

/// The action namespaces of the assistant's chats. Their keys are left out
/// of the list while the assistant is not shown (its engine is not
/// installed, or it is switched off), like its commands.
const ASSISTANT_NAMESPACES: [&str; 2] = ["assistant", "quick_chat"];

/// Commands that are started by typing a character, not by a key binding,
/// so the keymap does not know them: the action, the character, and where
/// it has to be typed. They are listed like the others.
const TYPED_TRIGGERS: [(&str, &str, &str); 1] = [(
    "editor_tools::OpenInsertMenu",
    "/",
    "at the start of a line or after a space",
)];

/// What the row of a typed trigger says about why its key stays.
pub const TYPED_TRIGGER_NOTE: &str =
    "This is typed into the text, not a keyboard shortcut. It cannot be changed.";

/// What the list says above its rows: the keys are one way, the menus
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
    /// In the list of keyboard shortcuts itself.
    Shortcuts,
    /// In one of the assistant's chats: the quick chat at the selection,
    /// the side chat beside the text.
    AssistantChat,
    /// In a dialog.
    Dialog,
    /// In a text field.
    TextField,
    /// Somewhere this list does not know by name.
    Elsewhere,
}

impl Place {
    /// How the list names the place.
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
            Place::Shortcuts => "In keyboard shortcuts",
            Place::AssistantChat => "In the assistant's chats",
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
        if has("Keymap") {
            // The list of keyboard shortcuts: its query field, the
            // question it asks about a key.
            Place::Shortcuts
        } else if has("FindBar") {
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
        } else if has("QuickChat") || has("Assistant") || has("assistant") {
            // The side chat (`AssistantPanel`, `AssistantMessages`, ...),
            // the quick chat's popover, and the editors of both (the
            // composer and the answers: `Editor && assistant...`).
            Place::AssistantChat
        } else if has("Picker") {
            Place::List
        } else if [
            "ModalLayer",
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
            Place::AssistantChat => Some(crate::actions::CATEGORY_ASSISTANT),
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

/// One row: a command in a place, with its keys.
pub struct Shortcut {
    /// What it does.
    pub title: SharedString,
    /// The category of its command.
    pub category: SharedString,
    /// Where the keyboard has to be.
    pub place: Place,
    /// The heading it is listed under: place and category.
    pub heading: SharedString,
    /// The key the app shows for it (in hints and in the menu bar), as
    /// shown; empty for a command without a key.
    pub keys: String,
    /// Every key it has, in the order the row shows them, with where each
    /// came from (a default, the user's in the place of a default, one the
    /// user added).
    pub all_keys: Vec<TargetKey>,
    /// Its keys by default.
    pub default_keys: Vec<KeySequence>,
    /// Whether its keys are not the defaults.
    pub changed: bool,
    /// The command in the key context the row stands for: what the list
    /// shows the keys of and checks a new key against. `None` for a row
    /// that is not a binding ([`Shortcut::fixed`]).
    pub target: Option<Target>,
    /// The command in every key context of this place that has the same
    /// keys ([`Shortcut::target`] among them): a change of the row is made
    /// to all of them. The find bar binds "case sensitive" for the bar and
    /// for the field in it; to the user that is one key.
    pub targets: Vec<Target>,
    /// For a row whose key is not a binding and cannot be changed: why.
    pub fixed: Option<SharedString>,
    /// What such a row shows as its key (the `/` of the insert menu).
    pub typed: Option<KeySequence>,
    /// Whether the row is a command of the palette (the others are keys
    /// named after their actions, which only mean something where they are
    /// pressed).
    pub described: bool,
    action: Box<dyn Action>,
    hint: Option<KeyHint>,
}

impl Shortcut {
    /// The command's action, for whoever runs it from its row.
    pub fn action(&self) -> Box<dyn Action> {
        self.action.boxed_clone()
    }

    /// The name of the command's action (`editor::ToggleBold`).
    pub fn action_name(&self) -> &'static str {
        self.action.name()
    }

    /// The key the app shows for it, as a hint.
    pub fn hint(&self) -> Option<&KeyHint> {
        self.hint.as_ref()
    }
}

/// A key of the model as a hint.
pub fn hint_of(keys: &KeySequence) -> Option<KeyHint> {
    KeyHint::parse(&keys.to_string())
}

/// A key of the model as the app shows keys ("⇧⌘7").
pub fn key_text(keys: &KeySequence) -> String {
    hint_of(keys).map(|hint| hint.text()).unwrap_or_default()
}

/// The place a command in a key context is listed under.
pub fn place_of(target: &Target) -> Place {
    Place::of_predicate(target.context.text())
}

/// What tells two rows of one command in one place apart, by something
/// in the key context of the second: the library moves with the arrows
/// wherever the keyboard is in it and with letters only while nothing is
/// typed there; the list of shortcuts moves through its rows and through
/// the lines of a question it asks.
const DISTINCTIONS: [(&str, &str); 2] = [
    ("!TextField", " (outside a text field)"),
    ("KeymapChoices", " (in a question)"),
];

fn distinction(target: &Target) -> Option<&'static str> {
    let context = target.context.text()?;
    DISTINCTIONS
        .iter()
        .find(|(mark, _)| context.contains(mark))
        .map(|(_, words)| *words)
}

fn same_keys(a: &TargetKeys, b: &TargetKeys) -> bool {
    let set = |keys: &[KeySequence]| {
        let mut keys: Vec<String> = keys.iter().map(|key| key.to_string()).collect();
        keys.sort();
        keys
    };
    let now = |target: &TargetKeys| {
        set(&target
            .keys
            .iter()
            .map(|key| key.keys.clone())
            .collect::<Vec<_>>())
    };
    now(a) == now(b) && set(&a.default_keys) == set(&b.default_keys)
}

/// The rows of one command: one for each place it has targets in, the
/// widest place first. Targets of one place that have the same keys are
/// one row (the same key bound in the bar and in the field of the bar is
/// one key "in the find bar"); targets with keys of their own are rows of
/// their own.
fn rows_of(
    title: &SharedString,
    category: &SharedString,
    action: &dyn Action,
    described: bool,
    targets: &[&TargetKeys],
    cx: &App,
) -> Vec<Shortcut> {
    let places = crate::keymap::model(cx).map(|model| model.places());
    // Where the keyboard itself can be, and text is typed: that is where
    // a new key is checked hardest, and where it has to win. (A context
    // that only ever has the keyboard in something inside it, such as the
    // find bar around its field, would say that a key of the window beats
    // it, which is true of the bar and not of the field.)
    let stands_for_the_row = |target: &Target| {
        places.is_some_and(|places| {
            places.is_typing(&target.context)
                && places
                    .relation(&target.context, &writer_keymap::Context::anywhere())
                    .rank
                    == writer_keymap::Rank::LaterWins
        })
    };
    let mut places: Vec<Place> = targets.iter().map(|keys| place_of(&keys.target)).collect();
    places.sort();
    places.dedup();
    let mut rows = Vec::new();
    for place in places {
        let mut groups: Vec<Vec<&TargetKeys>> = Vec::new();
        for target in targets
            .iter()
            .filter(|keys| place_of(&keys.target) == place)
        {
            match groups.iter_mut().find(|group| same_keys(group[0], target)) {
                Some(group) => group.push(target),
                None => groups.push(vec![target]),
            }
        }
        let several = groups.len() > 1;
        for group in groups {
            let state = group
                .iter()
                .find(|keys| stands_for_the_row(&keys.target))
                .unwrap_or(&group[0]);
            let title: SharedString = match distinction(&state.target).filter(|_| several) {
                Some(words) => format!("{title}{words}").into(),
                None => title.clone(),
            };
            let hint = state.shown.as_ref().and_then(hint_of);
            rows.push(Shortcut {
                title,
                category: category.clone(),
                place,
                heading: place.heading(category).into(),
                keys: hint.as_ref().map(KeyHint::text).unwrap_or_default(),
                all_keys: state.keys.clone(),
                default_keys: state.default_keys.clone(),
                changed: group.iter().any(|keys| keys.changed),
                target: Some(state.target.clone()),
                targets: group.iter().map(|keys| keys.target.clone()).collect(),
                fixed: None,
                typed: None,
                described,
                action: action.boxed_clone(),
                hint,
            });
        }
    }
    rows
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
/// uses: the code says "thread" where the interface says "comment", and
/// "prev" where it says "previous".
fn plain_words(title: &str) -> String {
    title
        .replace(" thread", " comment")
        .split(' ')
        .map(|word| if word == "prev" { "previous" } else { word })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The title of a command that has no descriptor, from its action's name.
pub fn title_of_action(name: &str) -> String {
    plain_words(&humanize_action_name(name))
}

/// Every row, in the order the list has them: by place, within a place by
/// heading (the commands' categories in registration order, then the
/// undescribed keys of the [`KEY_GROUPS`] namespaces).
pub fn collect_shortcuts(cx: &App) -> Vec<Shortcut> {
    let Some(effective) = crate::keymap::effective(cx) else {
        return Vec::new();
    };
    let targets_of = |name: &str| -> Vec<&TargetKeys> {
        effective
            .targets()
            .iter()
            .filter(|keys| keys.target.command.action == name)
            .collect()
    };
    let mut shortcuts = Vec::new();
    let mut described: HashSet<&'static str> = HashSet::new();
    let all = commands::all(cx);
    for category in commands::categories(&all) {
        for command in all.iter().filter(|command| command.category == category) {
            if !described.insert(command.action_name()) {
                continue;
            }
            let targets = targets_of(command.action_name());
            shortcuts.extend(rows_of(
                &command.title,
                &category,
                command.action_ref(),
                true,
                &targets,
                cx,
            ));
            if let Some((_, typed, whereabouts)) = TYPED_TRIGGERS
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
                    all_keys: Vec::new(),
                    default_keys: Vec::new(),
                    changed: false,
                    target: None,
                    targets: Vec::new(),
                    fixed: Some(TYPED_TRIGGER_NOTE.into()),
                    typed: KeySequence::parse(typed).ok(),
                    described: true,
                    action: command.action(),
                    hint: Some(hint),
                });
            }
        }
    }
    // The commands of a hidden category are not rows; nor are their keys
    // listed below under the names of their actions.
    if let Some(registry) = cx.try_global::<commands::CommandRegistry>() {
        for command in registry.commands() {
            described.insert(command.action_name());
        }
    }

    // Keys without a descriptor, straight from the keymap, one row per
    // action and place, in the order the actions were first bound.
    let assistant_hidden = commands::category_hidden(crate::actions::CATEGORY_ASSISTANT, cx);
    for (namespace, category) in KEY_GROUPS {
        if assistant_hidden && ASSISTANT_NAMESPACES.contains(&namespace) {
            continue;
        }
        let prefix = format!("{namespace}::");
        let mut seen: HashSet<&str> = HashSet::new();
        for keys in effective.targets() {
            let name = keys.target.command.action.as_str();
            if !name.starts_with(&prefix) || described.contains(name) || !seen.insert(name) {
                continue;
            }
            let Some(action) = crate::keymap::action_named(name, cx) else {
                continue;
            };
            shortcuts.extend(rows_of(
                &title_of_action(name).into(),
                &category.into(),
                action.as_ref(),
                false,
                &targets_of(name),
                cx,
            ));
        }
    }

    // Without the assistant its chats are not a place: the context menu
    // that opens in an answer is not listed either.
    if assistant_hidden {
        shortcuts.retain(|shortcut| shortcut.place != Place::AssistantChat);
    }

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
