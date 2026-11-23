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
//! # A place name is a promise
//!
//! Two rows under one place name are in the same place to whoever reads
//! the list: a key of the one given to the other is "taken", never "used
//! in another place" (the keymap is told the rows and their place names,
//! [`listed_rows`], and judges by them). So key contexts that never have
//! the keyboard together have names of their own, also where they are
//! parts of one panel: the comment list and the question it asks before
//! a comment is deleted, the list of shortcuts and a question it asks,
//! the conversation of a chat, the header and the context row of the side
//! chat, a button and a segmented control. A test holds every pair of rows of
//! the real keymap against this.
//!
//! Two things the keymap cannot tell are added: the insert menu, which is
//! opened by typing `/` and not by a binding ([`TYPED_TRIGGERS`]), and a
//! first line that says the same commands are in the menus
//! ([`MENUS_NOTE`]), for whoever would rather click.
//!
//! # A part of the app that is not shown
//!
//! The commands of a hidden category ([`commands::set_category_hidden`];
//! today the assistant's, while it is not shown) and the keys of its
//! places are not rows of the list ([`collect_shortcuts`]). They are rows
//! all the same ([`collect_every_shortcut`], in the same order, so the
//! list and the shortcuts file have one order whatever is shown): the
//! file has every command, and the keymap is told which commands are not
//! there ([`hidden_targets`], [`crate::keymap::refresh_listed`]), so that
//! their keys count for nothing in what the list says about a key.
//!
//! # Every key is a row
//!
//! Every command in every key context the keymap has is in a row
//! ([`Shortcut::targets`]): a test holds that against the keymap. A row
//! can stand for several key contexts, so there are fewer rows than the
//! shortcuts file has entries (one per command and key context); the one
//! row that is no binding (the `/` of the insert menu) is not in the file.

use std::collections::HashSet;

use gpui::{Action, App, SharedString};
use innerword_keymap::{KeySequence, Target, TargetKey, TargetKeys};
use innerword_ui::KeyHint;

use crate::commands;

/// Action namespaces whose bindings are listed even without a command
/// descriptor, with the category they are listed under (empty: the place
/// says it all).
const KEY_GROUPS: [(&str, &str); 21] = [
    // The keys of the menu bar that are no commands of the palette: hide
    // the app, hide the others, minimize the window. They are keys like
    // every other (and were in the shortcuts file without being in the
    // list, in 0.5.0). Not commands to run from a row: an undescribed row
    // has no "Run Command".
    ("app", crate::actions::CATEGORY_APP),
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
    // An open page: its context menu.
    ("page", ""),
    // An open whiteboard: its menu.
    ("board", ""),
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
    /// On an open page (a web page of the library): the page has the
    /// keyboard, and text is typed there (its fields).
    Page,
    /// On an open whiteboard: Excalidraw has the keyboard (its tools, its
    /// texts).
    Board,
    /// In the list of comment threads.
    CommentList,
    /// In the comment list while it asks whether a comment is to be
    /// deleted: the list's own keys do not apply then.
    CommentConfirm,
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
    /// In a question the list of keyboard shortcuts asks (about a key,
    /// about an import, about "reset all"): the choices have the keyboard,
    /// not the list.
    ShortcutsQuestion,
    /// In one of the assistant's chats: the quick chat at the selection,
    /// the side chat beside the text. Wherever the keyboard is in them,
    /// and where a message is written.
    AssistantChat,
    /// In the conversation of a chat: on its messages.
    AssistantConversation,
    /// In the header of the side chat: on its controls.
    AssistantHeader,
    /// In the context row of the side chat: on the documents and folders
    /// that were added.
    AssistantContextRow,
    /// In a dialog.
    Dialog,
    /// On a button or a switch.
    Button,
    /// On a segmented control.
    Segmented,
    /// Between the controls of a form (`tab` and `shift-tab`).
    Form,
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
            Place::Page => "On a page",
            Place::Board => "On a whiteboard",
            Place::CommentList => "In the comment list",
            Place::CommentConfirm => "Confirming in the comment list",
            Place::CommentField => "Writing a comment",
            Place::Library => "In the library",
            Place::Trash => "In the trash",
            Place::List => "In a list or menu",
            Place::Settings => "In settings",
            Place::VersionHistory => "In the version history",
            Place::Agents => "In the agents panel",
            Place::Shortcuts => "In keyboard shortcuts",
            Place::ShortcutsQuestion => "In a question of keyboard shortcuts",
            Place::AssistantChat => "In the assistant's chats",
            Place::AssistantConversation => "In a chat's conversation",
            Place::AssistantHeader => "In the header of the side chat",
            Place::AssistantContextRow => "In the context row of the side chat",
            Place::Dialog => "In a dialog",
            Place::Button => "On a button or a switch",
            Place::Segmented => "On a segmented control",
            Place::Form => "In a form",
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
        if has("KeymapChoices") {
            // A question the list of keyboard shortcuts asks.
            Place::ShortcutsQuestion
        } else if has("Keymap") {
            // The list of keyboard shortcuts: its rows, its query field.
            Place::Shortcuts
        } else if has("FindBar") {
            Place::FindBar
        } else if has("LinkPopover") {
            Place::LinkField
        } else if has(crate::pages::PAGE_VIEW) {
            Place::Page
        } else if has(crate::boards::BOARD_VIEW) {
            Place::Board
        } else if has("CommentComposer") {
            Place::CommentField
        } else if has("CommentThreads") && has("confirming") && !has("!confirming") {
            Place::CommentConfirm
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
        } else if has("AssistantMessages") {
            Place::AssistantConversation
        } else if has("AssistantHeader") {
            Place::AssistantHeader
        } else if has("AssistantContext") {
            Place::AssistantContextRow
        } else if has("QuickChat") || has("Assistant") || has("assistant") {
            // The side chat (`AssistantPanel`), the quick chat's popover,
            // and the editors of both (the composer and the answers:
            // `Editor && assistant...`).
            Place::AssistantChat
        } else if has("Picker") {
            Place::List
        } else if has("ModalLayer") || has("ConfirmDialog") {
            Place::Dialog
        } else if has("Button") || has("Toggle") {
            Place::Button
        } else if has("SegmentedControl") {
            Place::Segmented
        } else if has("TabNavigation") {
            Place::Form
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
            place if place.is_of_the_assistant() => Some(crate::actions::CATEGORY_ASSISTANT),
            _ => None,
        }
    }

    /// Whether the place is a part of the assistant's chats.
    pub fn is_of_the_assistant(self) -> bool {
        matches!(
            self,
            Place::AssistantChat
                | Place::AssistantConversation
                | Place::AssistantHeader
                | Place::AssistantContextRow
        )
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
/// typed there.
const DISTINCTIONS: [(&str, &str); 1] = [("!TextField", " (outside a text field)")];

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
                    .relation(&target.context, &innerword_keymap::Context::anywhere())
                    .rank
                    == innerword_keymap::Rank::LaterWins
        })
    };
    // A command of the assistant that works wherever the keyboard is in a
    // chat is bound for the chat and once more for the conversation (which
    // has keys of its own on the same level): one command "in the
    // assistant's chats", not one there and one in the conversation.
    let chat_wide = targets
        .iter()
        .any(|keys| place_of(&keys.target) == Place::AssistantChat);
    let place_in_the_list = |target: &Target| match place_of(target) {
        place if chat_wide && place.is_of_the_assistant() => Place::AssistantChat,
        place => place,
    };
    let mut places: Vec<Place> = targets
        .iter()
        .map(|keys| place_in_the_list(&keys.target))
        .collect();
    places.sort();
    places.dedup();
    let mut rows = Vec::new();
    for place in places {
        let mut groups: Vec<Vec<&TargetKeys>> = Vec::new();
        for target in targets
            .iter()
            .filter(|keys| place_in_the_list(&keys.target) == place)
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
                // The key context that stands for the row first.
                targets: std::iter::once(state.target.clone())
                    .chain(
                        group
                            .iter()
                            .map(|keys| keys.target.clone())
                            .filter(|target| *target != state.target),
                    )
                    .collect(),
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

/// Keys without a descriptor whose action's name alone does not say what
/// they do: the action, and the title of its row.
const KEY_TITLES: [(&str, &str); 5] = [
    ("app::Hide", "Hide Inner Word"),
    ("app::HideOthers", "Hide the other apps"),
    ("app::Minimize", "Minimize the window"),
    ("page::OpenContextMenu", "Show the page's context menu"),
    (
        "board::OpenContextMenu",
        "Show the whiteboard's context menu",
    ),
];

/// What the row of one of the whiteboard's own keys says about why it stays.
pub const BOARD_KEY_NOTE: &str =
    "This is the whiteboard's own key: Excalidraw acts on it there. It cannot be changed.";

/// What the row of the tool keys says.
pub const TOOL_KEYS_NOTE: &str = "On a whiteboard the letters and digits choose Excalidraw's \
     tools (V selection, R rectangle, D diamond, O ellipse, A arrow, L line, P pencil, T text, \
     E eraser, H hand, 1 to 0). They cannot be changed.";

/// The category the whiteboard's own keys are listed under.
pub const BOARD_KEYS_CATEGORY: &str = "The whiteboard's own keys";

/// The rows of the whiteboard's own keys (`WHITEBOARD_KEYS`) and of its tool
/// keys: fixed rows "On a whiteboard", which are no bindings of commands
/// (the keymap's `NoAction` bindings that keep GPUI off them are fixed) and
/// no entries of the shortcuts file.
fn whiteboard_key_rows() -> Vec<Shortcut> {
    let place = Place::Board;
    let category: SharedString = BOARD_KEYS_CATEGORY.into();
    let heading: SharedString = place.heading(&category).into();
    // One row for each thing a key does (two keys clear the canvas).
    let mut grouped: Vec<(&'static str, Vec<&'static str>)> = Vec::new();
    for key in innerword_board_web::keys::WHITEBOARD_KEYS {
        match grouped.iter_mut().find(|(what, _)| *what == key.what) {
            Some((_, keys)) => keys.push(key.keys),
            None => grouped.push((key.what, vec![key.keys])),
        }
    }
    let mut rows: Vec<Shortcut> = grouped
        .into_iter()
        .map(|(what, keys)| {
            let typed = KeySequence::parse(keys[0]).ok();
            let hint = typed.as_ref().and_then(hint_of);
            let shown: Vec<String> = keys
                .iter()
                .filter_map(|keys| KeySequence::parse(keys).ok())
                .map(|keys| key_text(&keys))
                .collect();
            Shortcut {
                title: what.into(),
                category: category.clone(),
                place,
                heading: heading.clone(),
                keys: shown.join(" "),
                all_keys: Vec::new(),
                default_keys: Vec::new(),
                changed: false,
                target: None,
                targets: Vec::new(),
                fixed: Some(BOARD_KEY_NOTE.into()),
                typed,
                described: false,
                action: Box::new(gpui::NoAction),
                hint,
            }
        })
        .collect();
    rows.push(Shortcut {
        title: innerword_board_web::keys::TOOL_KEYS_ROW.into(),
        category: category.clone(),
        place,
        heading,
        keys: String::new(),
        all_keys: Vec::new(),
        default_keys: Vec::new(),
        changed: false,
        target: None,
        targets: Vec::new(),
        fixed: Some(TOOL_KEYS_NOTE.into()),
        typed: None,
        described: false,
        action: Box::new(gpui::NoAction),
        hint: None,
    });
    rows
}

/// The title of a command that has no descriptor, from its action's name.
pub fn title_of_action(name: &str) -> String {
    match KEY_TITLES.iter().find(|(action, _)| *action == name) {
        Some((_, title)) => title.to_string(),
        None => plain_words(&humanize_action_name(name)),
    }
}

/// Every row of the list, in the order the list has them: by place,
/// within a place by heading (the commands' categories in registration
/// order, then the undescribed keys of the [`KEY_GROUPS`] namespaces).
/// Without the rows of a part of the app that is not shown.
pub fn collect_shortcuts(cx: &App) -> Vec<Shortcut> {
    collect(cx)
        .into_iter()
        .filter(|(_, hidden)| !hidden)
        .map(|(shortcut, _)| shortcut)
        .collect()
}

/// [`collect_shortcuts`] with the rows that are not shown right now where
/// they would stand: every command the app has, in one order whatever is
/// shown. What the shortcuts file is written from.
pub fn collect_every_shortcut(cx: &App) -> Vec<Shortcut> {
    collect(cx)
        .into_iter()
        .map(|(shortcut, _)| shortcut)
        .collect()
}

/// The commands, each in every key context of its row, that are not shown
/// right now (the commands of a hidden category and the keys of its
/// places). The keymap is told ([`crate::keymap::refresh_listed`]): their
/// keys count for nothing in a verdict.
pub fn hidden_targets(cx: &App) -> Vec<Target> {
    collect(cx)
        .into_iter()
        .filter(|(_, hidden)| *hidden)
        .flat_map(|(shortcut, _)| shortcut.targets)
        .collect()
}

/// The rows as the keymap is told them (`innerword_keymap::Row`): the key
/// contexts of each row, the one that stands for it first, under the
/// name of its place. The keymap judges a key by them: see "A place name
/// is a promise" in the [module docs](self).
pub fn listed_rows(cx: &App) -> Vec<innerword_keymap::Row> {
    listed(cx).1
}

/// [`hidden_targets`] and [`listed_rows`] from one pass over the rows.
pub fn listed(cx: &App) -> (Vec<Target>, Vec<innerword_keymap::Row>) {
    let mut hidden_targets = Vec::new();
    let mut rows = Vec::new();
    for (shortcut, hidden) in collect(cx) {
        if shortcut.targets.is_empty() {
            continue;
        }
        if hidden {
            hidden_targets.extend(shortcut.targets.iter().cloned());
        }
        rows.push(innerword_keymap::Row::new(
            shortcut.targets,
            shortcut.place.label(),
        ));
    }
    (hidden_targets, rows)
}

/// How the list names the place of a command that is not shown right now,
/// where such a command has to be named at all (a file that is imported
/// changes its keys): "In the assistant's chats, not shown now".
pub fn hidden_place_label(place: Place) -> String {
    format!("{}, not shown now", place.label())
}

/// Every row with whether it is hidden, in the order of the list.
fn collect(cx: &App) -> Vec<(Shortcut, bool)> {
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
    let mut shortcuts: Vec<(Shortcut, bool)> = Vec::new();
    let mut described: HashSet<&'static str> = HashSet::new();
    // Every registered command, those of a hidden category too: they are
    // rows that are not shown.
    let registry = cx.try_global::<commands::CommandRegistry>();
    let all: Vec<innerword_ui::CommandDescriptor> = registry
        .map(|registry| registry.commands().to_vec())
        .unwrap_or_default();
    for category in commands::categories(&all) {
        let hidden = registry.is_some_and(|registry| registry.is_hidden(&category));
        for command in all.iter().filter(|command| command.category == category) {
            if !described.insert(command.action_name()) {
                continue;
            }
            let targets = targets_of(command.action_name());
            shortcuts.extend(
                rows_of(
                    &command.title,
                    &category,
                    command.action_ref(),
                    true,
                    &targets,
                    cx,
                )
                .into_iter()
                .map(|row| (row, hidden)),
            );
            if let Some((_, typed, whereabouts)) = TYPED_TRIGGERS
                .iter()
                .find(|(action, _, _)| *action == command.action_name())
                && let Some(hint) = KeyHint::parse(typed)
            {
                let place = Place::Text;
                let row = Shortcut {
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
                };
                shortcuts.push((row, hidden));
            }
        }
    }

    // Keys without a descriptor, straight from the keymap, one row per
    // action and place, in the order the actions were first bound.
    let assistant_hidden = commands::category_hidden(crate::actions::CATEGORY_ASSISTANT, cx);
    for (namespace, category) in KEY_GROUPS {
        let hidden = assistant_hidden && ASSISTANT_NAMESPACES.contains(&namespace);
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
            shortcuts.extend(
                rows_of(
                    &title_of_action(name).into(),
                    &category.into(),
                    action.as_ref(),
                    false,
                    &targets_of(name),
                    cx,
                )
                .into_iter()
                .map(|row| (row, hidden)),
            );
        }
    }

    // The whiteboard's own keys: fixed rows "On a whiteboard".
    shortcuts.extend(whiteboard_key_rows().into_iter().map(|row| (row, false)));

    // Without the assistant its chats are not a place: the context menu
    // that opens in an answer is not shown either.
    if assistant_hidden {
        for (shortcut, hidden) in &mut shortcuts {
            *hidden |= shortcut.place.is_of_the_assistant();
        }
    }

    // By place, then by heading in the order the headings first appeared;
    // rows keep their order within a heading. The rows that are not shown
    // are sorted with the others, so the order is one whatever is shown.
    let mut headings: Vec<SharedString> = Vec::new();
    for (shortcut, _) in &shortcuts {
        if !headings.contains(&shortcut.heading) {
            headings.push(shortcut.heading.clone());
        }
    }
    let rank = |(shortcut, _): &(Shortcut, bool)| {
        let heading = headings
            .iter()
            .position(|heading| *heading == shortcut.heading)
            .unwrap_or(usize::MAX);
        (shortcut.place, heading)
    };
    shortcuts.sort_by_key(rank);
    shortcuts
}
