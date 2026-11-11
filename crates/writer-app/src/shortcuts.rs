//! The keyboard shortcuts overlay (`cmd-/`).
//!
//! Generated, not written: the rows are the registered command descriptors
//! that have a binding in GPUI's keymap, grouped by category, followed by
//! the editing keys that have no descriptor (caret movement, selection,
//! deletion), named after their actions. The key shown is whatever the
//! keymap says right now. The list is searchable, and enter runs the
//! selected command.

use std::collections::HashSet;

use gpui::{Action, App, Context, SharedString, WeakEntity, Window};
use writer_ui::{
    KeyHint, Picker, PickerDelegate, PickerEntry, PickerItem, fuzzy::fuzzy_match,
};

use crate::{commands, workspace::Workspace};

/// The shortcuts overlay as a modal view.
pub type ShortcutsOverlay = Picker<ShortcutsDelegate>;

/// Action namespaces whose bindings are listed even without a command
/// descriptor, with the heading they are listed under.
const KEY_GROUPS: [(&str, &str); 1] = [("editor", "Editing keys")];

/// One shortcut.
pub struct Shortcut {
    /// What it does.
    pub title: SharedString,
    /// The group it is listed under.
    pub category: SharedString,
    /// The keys, as shown.
    pub keys: String,
    action: Box<dyn Action>,
    hint: KeyHint,
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

/// Every shortcut: bound commands by category, then the undescribed keys of
/// the [`KEY_GROUPS`] namespaces.
pub fn collect_shortcuts(cx: &App) -> Vec<Shortcut> {
    let mut shortcuts = Vec::new();
    let mut described: HashSet<&'static str> = HashSet::new();
    let all = commands::all(cx);
    for category in commands::categories(&all) {
        for command in all.iter().filter(|command| command.category == category) {
            described.insert(command.action_name());
            if let Some(hint) = command.key_hint(cx) {
                shortcuts.push(Shortcut {
                    title: command.title.clone(),
                    category: category.clone(),
                    keys: hint.text(),
                    action: command.action(),
                    hint,
                });
            }
        }
    }

    // Keys without a descriptor, straight from the keymap. Later bindings
    // take precedence, so walk backwards and keep the first of each action.
    let keymap = cx.key_bindings();
    let keymap = keymap.borrow();
    for (namespace, heading) in KEY_GROUPS {
        let prefix = format!("{namespace}::");
        let mut seen: HashSet<&'static str> = HashSet::new();
        let mut group = Vec::new();
        for binding in keymap.bindings().rev() {
            let name = binding.action().name();
            if !name.starts_with(&prefix) || described.contains(name) || !seen.insert(name) {
                continue;
            }
            let hint = KeyHint::from_binding(binding);
            group.push(Shortcut {
                title: humanize_action_name(name).into(),
                category: heading.into(),
                keys: hint.text(),
                action: binding.action().boxed_clone(),
                hint,
            });
        }
        // Back to keymap (declaration) order.
        group.reverse();
        shortcuts.extend(group);
    }
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
            let mut category: Option<&SharedString> = None;
            for (index, shortcut) in self.shortcuts.iter().enumerate() {
                if category != Some(&shortcut.category) {
                    category = Some(&shortcut.category);
                    self.rows.push(Row::Header(shortcut.category.clone()));
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
                let haystack = format!("{} {}", shortcut.category, shortcut.title);
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
                // Headers already name the group while browsing.
                if matches!(self.rows.first(), Some(Row::Header(_))) {
                    item.into()
                } else {
                    item.detail(shortcut.category.clone()).into()
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
        "No shortcut matches".into()
    }
}
