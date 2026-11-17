//! The user's changes: what is stored, and how it is edited.
//!
//! # What a change is
//!
//! For a command in a context ([`Target`]) the user has **the keys they
//! want for it**, and beside them **the default keys as they were when
//! that was chosen**. One entry per changed target; a target that is not
//! changed has no entry, so the stored value holds only the differences
//! and the defaults of a later version arrive by themselves.
//!
//! What kind of change it is follows from comparing the two lists
//! ([`crate::apply`] does it, key by key): a default key that is gone and a
//! new key in its position is a key **replaced**, a new key beside the
//! defaults a key **added**, a default key that is gone a key **removed**,
//! and a key for a target without defaults a key **given**. Keeping the
//! defaults of the moment in the entry is what lets a later version tell
//! "the user replaced `cmd-b`" from "the default is no longer `cmd-b`".
//!
//! # The stored value
//!
//! One JSON text ([`Changes::to_stored`], read by
//! [`Changes::from_stored`]):
//!
//! ```json
//! {"version":1,"changes":[
//!   {"command":"editor::ToggleBold","context":"Editor",
//!    "default":["cmd-b"],"keys":["cmd-alt-b"]},
//!   {"command":"app::VersionHistory","context":null,
//!    "default":[],"keys":["cmd-alt-h"]}
//! ]}
//! ```
//!
//! `input` is there for an action with input. Nothing is lost by reading
//! and writing a value another version wrote: fields this version does not
//! know are kept where they were, an entry it cannot read (a key name it
//! does not know, another shape) is kept as it is, and an entry for a
//! command this version does not have is kept too. None of them does
//! anything; [`crate::Effective::inactive`] lists them.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::context::Context;
use crate::defaults::{Command, Defaults, Target};
use crate::keystroke::KeySequence;

/// The version of the stored value this crate writes.
pub const STORED_VERSION: u32 = 1;

fn stored_version() -> u32 {
    STORED_VERSION
}

/// The change to one target: the keys the user wants, and the defaults at
/// the time. See the [module docs](self).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChangeEntry {
    /// The action's name.
    pub command: String,
    /// The action's input, for an action with input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    /// The context the keys apply in (`null`: anywhere).
    #[serde(default)]
    pub context: Context,
    /// The default keys of the target when the change was made.
    #[serde(default)]
    pub default: Vec<KeySequence>,
    /// The keys the user wants, in the order the row shows them.
    pub keys: Vec<KeySequence>,
    /// Fields another version wrote, kept and written back.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl ChangeEntry {
    /// The target the entry is about.
    pub fn target(&self) -> Target {
        Target::of(
            Command {
                action: self.command.clone(),
                input: self.input.clone(),
            },
            self.context.clone(),
        )
    }
}

/// One element of the stored list: an entry, or something this version
/// cannot read as one, kept as it is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum Slot {
    Entry(ChangeEntry),
    Unread(Value),
}

/// A stored value that could not be read at all.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredError {
    /// What is wrong, in words.
    pub problem: String,
    /// The line of the text the problem is on (from 1), when it is not
    /// JSON.
    pub line: Option<usize>,
    /// The column on that line (from 1).
    pub column: Option<usize>,
}

impl fmt::Display for StoredError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "The stored keyboard shortcuts cannot be read: {}",
            self.problem
        )?;
        if let (Some(line), Some(column)) = (self.line, self.column) {
            write!(f, " (line {line}, column {column})")?;
        }
        Ok(())
    }
}

impl std::error::Error for StoredError {}

/// The user's changes to the default keymap. See the [module docs](self).
///
/// The editing methods take the [`Defaults`] because a change is kept as a
/// difference from them: giving a command its default key back by hand
/// leaves no change behind.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Changes {
    #[serde(default = "stored_version")]
    version: u32,
    #[serde(default, rename = "changes")]
    slots: Vec<Slot>,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

impl Default for Changes {
    fn default() -> Self {
        Self {
            version: STORED_VERSION,
            slots: Vec::new(),
            extra: BTreeMap::new(),
        }
    }
}

impl Changes {
    /// No changes: the default keymap.
    pub fn new() -> Self {
        Self::default()
    }

    /// Reads the stored value. Fails only for a text that is not a JSON
    /// object; everything inside one is read or kept (see the [module
    /// docs](self)). The caller then starts from no changes and should
    /// leave the stored text alone until the user changes a key.
    pub fn from_stored(text: &str) -> Result<Self, StoredError> {
        let value: Value = serde_json::from_str(text).map_err(|error| StoredError {
            problem: json_problem(&error),
            line: Some(error.line()),
            column: Some(error.column()),
        })?;
        if !value.is_object() {
            return Err(StoredError {
                problem: "it is not a JSON object".to_string(),
                line: None,
                column: None,
            });
        }
        serde_json::from_value(value).map_err(|error| StoredError {
            problem: error.to_string(),
            line: None,
            column: None,
        })
    }

    /// The value to store: one line of JSON. Reading it back gives the
    /// same changes.
    pub fn to_stored(&self) -> String {
        serde_json::to_string(self).expect("changes are plain JSON")
    }

    /// Whether there is nothing to store: no entry, read or not, and no
    /// field of another version. The app can then remove the setting.
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty() && self.extra.is_empty()
    }

    /// The entries this version can read, in stored order. Includes those
    /// for commands this version does not have.
    pub fn entries(&self) -> impl Iterator<Item = &ChangeEntry> {
        self.slots.iter().filter_map(|slot| match slot {
            Slot::Entry(entry) => Some(entry),
            Slot::Unread(_) => None,
        })
    }

    /// The elements of the stored list this version could not read as an
    /// entry, as they were stored.
    pub fn unread(&self) -> impl Iterator<Item = &Value> {
        self.slots.iter().filter_map(|slot| match slot {
            Slot::Entry(_) => None,
            Slot::Unread(value) => Some(value),
        })
    }

    /// The entry for `target`: the first one, when a stored value has
    /// several (the others do nothing).
    pub fn entry(&self, target: &Target) -> Option<&ChangeEntry> {
        self.entries().find(|entry| entry.target() == *target)
    }

    /// The keys `target` has with these changes, in the order its row
    /// shows them. The default keys when it has no change.
    pub fn keys_of(&self, defaults: &Defaults, target: &Target) -> Vec<KeySequence> {
        let default = defaults.default_keys(target);
        resolve(&default, self.entry(target)).row
    }

    /// Whether `target` differs from its defaults.
    pub fn is_changed(&self, defaults: &Defaults, target: &Target) -> bool {
        defaults.knows(target)
            && resolve(&defaults.default_keys(target), self.entry(target)).is_changed()
    }

    /// Replaces the key `old` of `target` with `new`, in its position: a
    /// default key replaced stands where the default stood. Replacing a
    /// changed key with the default it came from leaves no change. When
    /// the target already has `new`, `old` is only removed.
    ///
    /// Returns whether anything changed. Nothing changes for a target this
    /// version does not have, or one that has no key `old`.
    pub fn replace_key(
        &mut self,
        defaults: &Defaults,
        target: &Target,
        old: &KeySequence,
        new: KeySequence,
    ) -> bool {
        self.edit(defaults, target, |keys| {
            let Some(position) = keys.iter().position(|key| key == old) else {
                return;
            };
            if keys.contains(&new) {
                keys.remove(position);
            } else {
                keys[position] = new;
            }
        })
    }

    /// Gives `target` one more key: a second key for a command that has
    /// one, a first key for a command without. Returns whether anything
    /// changed (nothing does when it has the key already).
    pub fn add_key(&mut self, defaults: &Defaults, target: &Target, new: KeySequence) -> bool {
        self.edit(defaults, target, |keys| {
            if !keys.contains(&new) {
                keys.push(new);
            }
        })
    }

    /// Takes the key `key` from `target`. The command may be left without
    /// a key. Returns whether anything changed.
    pub fn remove_key(&mut self, defaults: &Defaults, target: &Target, key: &KeySequence) -> bool {
        self.edit(defaults, target, |keys| keys.retain(|have| have != key))
    }

    /// Makes `keys` the keys of `target`, all at once (a key listed twice
    /// counts once). The default keys in any order leave no change.
    /// Returns whether anything changed.
    pub fn set_keys(&mut self, defaults: &Defaults, target: &Target, keys: &[KeySequence]) -> bool {
        self.edit(defaults, target, |have| *have = keys.to_vec())
    }

    /// Puts `target` back to its default keys. Returns whether there was a
    /// change to drop. Works for a target this version does not have too:
    /// its stored entry is dropped.
    pub fn reset(&mut self, target: &Target) -> bool {
        let before = self.slots.len();
        self.slots
            .retain(|slot| !matches!(slot, Slot::Entry(entry) if entry.target() == *target));
        self.slots.len() != before
    }

    /// Puts `command` back to its default keys in every context it has
    /// any. Returns whether there was a change to drop.
    pub fn reset_command(&mut self, command: &Command) -> bool {
        let before = self.slots.len();
        self.slots.retain(
            |slot| !matches!(slot, Slot::Entry(entry) if entry.target().command == *command),
        );
        self.slots.len() != before
    }

    /// Puts everything back: every entry goes, also those this version
    /// could not read or has no command for. (Fields of another version
    /// beside the list stay; they are not changes to a key.)
    pub fn reset_all(&mut self) {
        self.slots.clear();
    }

    /// The targets that have an entry, in stored order, each once.
    pub fn targets(&self) -> Vec<Target> {
        let mut targets: Vec<Target> = Vec::new();
        for entry in self.entries() {
            let target = entry.target();
            if !targets.contains(&target) {
                targets.push(target);
            }
        }
        targets
    }

    /// Changes the keys of `target` through `change`, which is handed the
    /// keys the row shows now, and stores the result as a difference from
    /// the defaults of now.
    fn edit(
        &mut self,
        defaults: &Defaults,
        target: &Target,
        change: impl FnOnce(&mut Vec<KeySequence>),
    ) -> bool {
        if !defaults.knows(target) {
            return false;
        }
        let default = defaults.default_keys(target);
        let before = resolve(&default, self.entry(target)).row;
        let mut keys = before.clone();
        change(&mut keys);
        let mut seen: Vec<KeySequence> = Vec::new();
        keys.retain(|key| {
            let new = !seen.contains(key);
            if new {
                seen.push(key.clone());
            }
            new
        });
        // The same keys in another order are the same keys.
        if diff(&before, &keys).is_empty() {
            return false;
        }
        self.store(target, default, keys);
        true
    }

    /// Stores `keys` as what the user wants for `target`, or drops the
    /// entry when that is what the defaults give anyway.
    pub(crate) fn store(
        &mut self,
        target: &Target,
        default: Vec<KeySequence>,
        keys: Vec<KeySequence>,
    ) {
        let unchanged = diff(&default, &keys).is_empty();
        let is_target =
            |slot: &Slot| matches!(slot, Slot::Entry(entry) if entry.target() == *target);
        let first = self.slots.iter().position(is_target);
        // Further entries for the same target did nothing and would come
        // to life when the first one goes: they go too.
        let mut ix = 0;
        self.slots.retain(|slot| {
            let keep = !is_target(slot) || Some(ix) == first;
            ix += 1;
            keep
        });
        match (first, unchanged) {
            (Some(position), true) => {
                self.slots.remove(position);
            }
            (Some(position), false) => {
                if let Slot::Entry(entry) = &mut self.slots[position] {
                    entry.default = default;
                    entry.keys = keys;
                }
            }
            (None, true) => {}
            (None, false) => self.slots.push(Slot::Entry(ChangeEntry {
                command: target.command.action.clone(),
                input: target.command.input.clone(),
                context: target.context.clone(),
                default,
                keys,
                extra: BTreeMap::new(),
            })),
        }
    }

    pub(crate) fn slots(&self) -> &[Slot] {
        &self.slots
    }
}

/// What serde_json says is wrong, without its own "at line x column y".
pub(crate) fn json_problem(error: &serde_json::Error) -> String {
    let text = error.to_string();
    match text.rfind(" at line ") {
        Some(position) => text[..position].to_string(),
        None => text,
    }
}

/// The difference between a list of default keys and the keys wanted.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Ops {
    /// A default key and the key that stands in its position.
    pub replaced: Vec<(KeySequence, KeySequence)>,
    /// Default keys that are gone.
    pub removed: Vec<KeySequence>,
    /// Keys beside the defaults.
    pub added: Vec<KeySequence>,
}

impl Ops {
    pub fn is_empty(&self) -> bool {
        self.replaced.is_empty() && self.removed.is_empty() && self.added.is_empty()
    }
}

/// Compares the keys wanted with the defaults they were chosen against.
///
/// Keys in both lists are kept. They cut both lists into stretches: the
/// defaults between two kept keys, and the new keys after the same kept
/// key. Within a stretch the first default that is gone and the first new
/// key are a replacement, and so on in order; a default left over was
/// removed, a new key left over was added. So `[a, b, c]` wanted as
/// `[b, x]` is "a removed, c replaced by x", and `[a]` wanted as `[a, x]`
/// is "x added".
pub(crate) fn diff(default: &[KeySequence], keys: &[KeySequence]) -> Ops {
    let mut ops = Ops::default();
    let mut wanted: Vec<&KeySequence> = Vec::new();
    for key in keys {
        if !wanted.contains(&key) {
            wanted.push(key);
        }
    }
    // The stretch of a key: the kept key before it (None: the start).
    let mut gone: Vec<(Option<&KeySequence>, &KeySequence)> = Vec::new();
    let mut stretch = None;
    for key in default {
        if wanted.contains(&key) {
            stretch = Some(key);
        } else {
            gone.push((stretch, key));
        }
    }
    let mut new: Vec<(Option<&KeySequence>, &KeySequence)> = Vec::new();
    let mut stretch = None;
    for key in wanted {
        if default.contains(key) {
            stretch = Some(key);
        } else {
            new.push((stretch, key));
        }
    }
    let mut taken = vec![false; new.len()];
    for (stretch, old) in gone {
        let partner = new
            .iter()
            .enumerate()
            .find(|(ix, (theirs, _))| !taken[*ix] && *theirs == stretch);
        match partner {
            Some((ix, (_, key))) => {
                taken[ix] = true;
                ops.replaced.push((old.clone(), (*key).clone()));
            }
            None => ops.removed.push(old.clone()),
        }
    }
    for (ix, (_, key)) in new.iter().enumerate() {
        if !taken[ix] {
            ops.added.push((*key).clone());
        }
    }
    ops
}

/// What became of one default key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Fate {
    Kept,
    Removed,
    Replaced(KeySequence),
}

/// A part of an entry that does nothing because the default it was made
/// against is no longer a default.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Stale {
    /// The key replaced is not a default now: the new key is kept as an
    /// added one.
    Replaced { from: KeySequence, to: KeySequence },
    /// The key removed is not a default now.
    Removed(KeySequence),
}

/// An entry applied to the defaults of now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Resolved {
    /// What became of each default key, in the order of the defaults.
    pub fates: Vec<Fate>,
    /// Keys beside the defaults.
    pub added: Vec<KeySequence>,
    /// Parts of the entry that do nothing now.
    pub stale: Vec<Stale>,
    /// The keys the target has, in the order its row shows them: the
    /// user's order, then defaults that arrived since.
    pub row: Vec<KeySequence>,
}

impl Resolved {
    pub fn is_changed(&self) -> bool {
        !self.added.is_empty() || self.fates.iter().any(|fate| *fate != Fate::Kept)
    }
}

/// Applies `entry` to the default keys of now. The entry was made against
/// the defaults written in it; where those are not the defaults of now,
/// what the user chose is kept as far as it still means something:
///
/// - a key replaced that is no longer a default: the user's key stays, as
///   an added key, beside the new defaults;
/// - a key removed that is no longer a default: nothing to remove;
/// - a default that is new: it is there (it arrives by itself);
/// - a key the user added that is a default now: it is that default.
pub(crate) fn resolve(default: &[KeySequence], entry: Option<&ChangeEntry>) -> Resolved {
    let mut resolved = Resolved {
        fates: vec![Fate::Kept; default.len()],
        added: Vec::new(),
        stale: Vec::new(),
        row: Vec::new(),
    };
    let Some(entry) = entry else {
        resolved.row = default.to_vec();
        return resolved;
    };
    let position = |key: &KeySequence| default.iter().position(|have| have == key);
    let ops = diff(&entry.default, &entry.keys);

    for (from, to) in ops.replaced {
        match position(&from) {
            // The new key is a default of its own now: the old one just goes.
            Some(ix) if position(&to).is_some() => resolved.fates[ix] = Fate::Removed,
            Some(ix) => resolved.fates[ix] = Fate::Replaced(to),
            None => {
                if position(&to).is_none() && !resolved.added.contains(&to) {
                    resolved.added.push(to.clone());
                }
                resolved.stale.push(Stale::Replaced { from, to });
            }
        }
    }
    for key in ops.removed {
        match position(&key) {
            Some(ix) => resolved.fates[ix] = Fate::Removed,
            None => resolved.stale.push(Stale::Removed(key)),
        }
    }
    for key in ops.added {
        if position(&key).is_none() && !resolved.added.contains(&key) {
            resolved.added.push(key);
        }
    }
    // A key cannot be there twice: an added key that also replaces a
    // default is that replacement.
    let replacements: Vec<KeySequence> = resolved
        .fates
        .iter()
        .filter_map(|fate| match fate {
            Fate::Replaced(to) => Some(to.clone()),
            _ => None,
        })
        .collect();
    resolved.added.retain(|key| !replacements.contains(key));

    // The row: what the target has, in the user's order.
    let mut have: Vec<KeySequence> = Vec::new();
    for (key, fate) in default.iter().zip(&resolved.fates) {
        match fate {
            Fate::Kept => have.push(key.clone()),
            Fate::Replaced(to) => have.push(to.clone()),
            Fate::Removed => {}
        }
    }
    have.extend(resolved.added.iter().cloned());
    for key in &entry.keys {
        if have.contains(key) && !resolved.row.contains(key) {
            resolved.row.push(key.clone());
        }
    }
    for key in have {
        if !resolved.row.contains(&key) {
            resolved.row.push(key);
        }
    }
    resolved
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::defaults::DefaultBinding;

    fn key(text: &str) -> KeySequence {
        KeySequence::parse(text).unwrap()
    }

    fn keys(texts: &[&str]) -> Vec<KeySequence> {
        texts.iter().map(|text| key(text)).collect()
    }

    fn defaults() -> Defaults {
        let binding = |action: &str, context: Option<&str>, keys: &str| {
            DefaultBinding::new(action, None, context, keys).unwrap()
        };
        let mut defaults = Defaults::from_bindings([
            binding("app::Quit", None, "cmd-q"),
            binding("editor::ToggleBold", Some("Editor"), "cmd-b"),
            binding("editor::MoveLeft", Some("Editor"), "ctrl-b"),
            binding("editor::MoveLeft", Some("Editor"), "left"),
            binding("editor::ToggleNumberedList", Some("Editor"), "cmd-&"),
            binding("editor::ToggleNumberedList", Some("Editor"), "cmd-shift-7"),
        ]);
        defaults.add_keyless(Target::new("app::VersionHistory", None));
        defaults
    }

    fn bold() -> Target {
        Target::new("editor::ToggleBold", Some("Editor"))
    }

    fn left() -> Target {
        Target::new("editor::MoveLeft", Some("Editor"))
    }

    #[test]
    fn the_difference_between_two_lists() {
        let ops = |default: &[&str], wanted: &[&str]| diff(&keys(default), &keys(wanted));
        assert!(ops(&["a", "b"], &["a", "b"]).is_empty());
        assert!(ops(&["a", "b"], &["b", "a"]).is_empty());
        assert_eq!(ops(&["a"], &["x"]).replaced, vec![(key("a"), key("x"))]);
        assert_eq!(ops(&["a"], &["a", "x"]).added, keys(&["x"]));
        assert_eq!(ops(&["a"], &["x", "a"]).added, keys(&["x"]));
        assert_eq!(ops(&["a", "b"], &["a"]).removed, keys(&["b"]));
        assert_eq!(ops(&[], &["x"]).added, keys(&["x"]));
        assert_eq!(ops(&["a"], &[]).removed, keys(&["a"]));

        // a removed, c replaced by x.
        let mixed = ops(&["a", "b", "c"], &["b", "x"]);
        assert_eq!(mixed.removed, keys(&["a"]));
        assert_eq!(mixed.replaced, vec![(key("c"), key("x"))]);
        assert!(mixed.added.is_empty());
        // a replaced by x, c removed.
        let mixed = ops(&["a", "b", "c"], &["x", "b"]);
        assert_eq!(mixed.replaced, vec![(key("a"), key("x"))]);
        assert_eq!(mixed.removed, keys(&["c"]));
        // More new keys than defaults gone: the rest is added.
        let more = ops(&["a"], &["x", "y"]);
        assert_eq!(more.replaced, vec![(key("a"), key("x"))]);
        assert_eq!(more.added, keys(&["y"]));
        // A key listed twice counts once.
        assert_eq!(ops(&["a"], &["a", "x", "x"]).added, keys(&["x"]));
    }

    #[test]
    fn replacing_adding_and_removing() {
        let defaults = defaults();
        let mut changes = Changes::new();
        assert!(changes.is_empty());
        assert_eq!(changes.keys_of(&defaults, &bold()), keys(&["cmd-b"]));

        assert!(changes.replace_key(&defaults, &bold(), &key("cmd-b"), key("cmd-alt-b")));
        assert_eq!(changes.keys_of(&defaults, &bold()), keys(&["cmd-alt-b"]));
        assert!(changes.is_changed(&defaults, &bold()));
        assert!(!changes.is_changed(&defaults, &left()));
        // The same once more changes nothing.
        assert!(!changes.replace_key(&defaults, &bold(), &key("cmd-b"), key("cmd-alt-b")));

        assert!(changes.add_key(&defaults, &bold(), key("f5")));
        assert!(!changes.add_key(&defaults, &bold(), key("f5")));
        assert_eq!(
            changes.keys_of(&defaults, &bold()),
            keys(&["cmd-alt-b", "f5"])
        );

        assert!(changes.remove_key(&defaults, &bold(), &key("cmd-alt-b")));
        assert_eq!(changes.keys_of(&defaults, &bold()), keys(&["f5"]));
        assert!(changes.remove_key(&defaults, &bold(), &key("f5")));
        assert!(changes.keys_of(&defaults, &bold()).is_empty());
        assert!(changes.is_changed(&defaults, &bold()));
        assert!(!changes.remove_key(&defaults, &bold(), &key("f5")));

        // One entry all along.
        assert_eq!(changes.entries().count(), 1);
        assert_eq!(changes.targets(), vec![bold()]);
    }

    #[test]
    fn the_default_key_given_back_by_hand_leaves_no_change() {
        let defaults = defaults();
        let mut changes = Changes::new();
        changes.replace_key(&defaults, &bold(), &key("cmd-b"), key("cmd-alt-b"));
        assert!(changes.replace_key(&defaults, &bold(), &key("cmd-alt-b"), key("cmd-b")));
        assert!(changes.is_empty());

        changes.remove_key(&defaults, &left(), &key("left"));
        assert!(!changes.is_empty());
        assert!(changes.add_key(&defaults, &left(), key("left")));
        assert!(changes.is_empty(), "{changes:?}");

        // The defaults in another order are the defaults.
        assert!(!changes.set_keys(&defaults, &left(), &keys(&["left", "ctrl-b"])));
        assert!(changes.is_empty());
    }

    #[test]
    fn a_key_in_two_spellings_is_changed_as_one() {
        let defaults = defaults();
        let list = Target::new("editor::ToggleNumberedList", Some("Editor"));
        let mut changes = Changes::new();
        // Named by either spelling.
        assert!(changes.replace_key(&defaults, &list, &key("cmd-&"), key("cmd-alt-7")));
        assert_eq!(changes.keys_of(&defaults, &list), keys(&["cmd-alt-7"]));
        assert!(changes.replace_key(&defaults, &list, &key("cmd-alt-7"), key("cmd-shift-7")));
        assert!(changes.is_empty());
    }

    #[test]
    fn a_key_for_a_command_without_one() {
        let defaults = defaults();
        let history = Target::new("app::VersionHistory", None);
        let mut changes = Changes::new();
        assert!(changes.add_key(&defaults, &history, key("cmd-alt-h")));
        assert_eq!(changes.keys_of(&defaults, &history), keys(&["cmd-alt-h"]));
        assert!(changes.is_changed(&defaults, &history));
        let entry = changes.entry(&history).unwrap();
        assert!(entry.default.is_empty());
        assert!(entry.context.is_anywhere());
    }

    #[test]
    fn a_target_this_version_does_not_have_is_not_edited() {
        let defaults = defaults();
        let gone = Target::new("app::Gone", None);
        let mut changes = Changes::new();
        assert!(!changes.add_key(&defaults, &gone, key("f5")));
        // Bold exists in the text, not in the library.
        let elsewhere = Target::new("editor::ToggleBold", Some("LibraryPanel"));
        assert!(!changes.add_key(&defaults, &elsewhere, key("f5")));
        assert!(changes.is_empty());
    }

    #[test]
    fn replacing_with_a_key_the_target_has_removes_the_old_one() {
        let defaults = defaults();
        let mut changes = Changes::new();
        assert!(changes.replace_key(&defaults, &left(), &key("ctrl-b"), key("left")));
        assert_eq!(changes.keys_of(&defaults, &left()), keys(&["left"]));
        // A key the target does not have cannot be replaced.
        assert!(!changes.replace_key(&defaults, &left(), &key("f9"), key("f8")));
    }

    #[test]
    fn putting_back() {
        let defaults = defaults();
        let mut changes = Changes::new();
        changes.replace_key(&defaults, &bold(), &key("cmd-b"), key("cmd-alt-b"));
        changes.remove_key(&defaults, &left(), &key("ctrl-b"));
        changes.add_key(
            &defaults,
            &Target::new("app::VersionHistory", None),
            key("f6"),
        );

        assert!(changes.reset(&bold()));
        assert!(!changes.reset(&bold()));
        assert_eq!(changes.keys_of(&defaults, &bold()), keys(&["cmd-b"]));
        assert!(changes.reset_command(&Command::new("editor::MoveLeft")));
        assert_eq!(changes.targets().len(), 1);
        changes.reset_all();
        assert!(changes.is_empty());
    }

    #[test]
    fn the_stored_value_round_trips() {
        let defaults = defaults();
        let mut changes = Changes::new();
        changes.replace_key(&defaults, &bold(), &key("cmd-b"), key("cmd-alt-b"));
        changes.add_key(
            &defaults,
            &Target::new("app::VersionHistory", None),
            key("cmd-alt-h"),
        );
        let stored = changes.to_stored();
        assert_eq!(
            stored,
            "{\"version\":1,\"changes\":[\
             {\"command\":\"editor::ToggleBold\",\"context\":\"Editor\",\
             \"default\":[\"cmd-b\"],\"keys\":[\"cmd-alt-b\"]},\
             {\"command\":\"app::VersionHistory\",\"context\":null,\
             \"default\":[],\"keys\":[\"cmd-alt-h\"]}]}"
        );
        assert_eq!(Changes::from_stored(&stored).unwrap(), changes);
        assert_eq!(Changes::from_stored("{}").unwrap(), Changes::new());
    }

    #[test]
    fn what_another_version_wrote_is_kept() {
        let stored = r#"{"version":3,"layout":"de","changes":[
            {"command":"editor::ToggleBold","context":"Editor","default":["cmd-b"],"keys":["cmd-alt-b"],"since":"0.7.0"},
            {"command":"app::Gone","context":null,"default":["cmd-g"],"keys":["f7"]},
            {"command":"editor::MoveLeft","context":"Editor","default":["left"],"keys":["hyper-"]},
            {"chord":["a","b"]},
            "a note"
        ]}"#;
        let mut changes = Changes::from_stored(stored).unwrap();
        assert_eq!(changes.entries().count(), 2);
        assert_eq!(changes.unread().count(), 3);

        let written: Value = serde_json::from_str(&changes.to_stored()).unwrap();
        let original: Value = serde_json::from_str(stored).unwrap();
        assert_eq!(written, original);

        // An edit keeps all of it, and the fields of the entry it changes.
        let defaults = defaults();
        assert!(changes.replace_key(&defaults, &bold(), &key("cmd-alt-b"), key("f5")));
        let written: Value = serde_json::from_str(&changes.to_stored()).unwrap();
        assert_eq!(written["version"], 3);
        assert_eq!(written["layout"], "de");
        assert_eq!(written["changes"][0]["since"], "0.7.0");
        assert_eq!(written["changes"][0]["keys"][0], "f5");
        assert_eq!(written["changes"][1], original["changes"][1]);
        assert_eq!(written["changes"][2], original["changes"][2]);
        assert_eq!(written["changes"][3], original["changes"][3]);
        assert_eq!(written["changes"][4], "a note");

        // Putting everything back drops the entries, read or not.
        changes.reset_all();
        let written: Value = serde_json::from_str(&changes.to_stored()).unwrap();
        assert_eq!(written["changes"], serde_json::json!([]));
        assert_eq!(written["layout"], "de");
    }

    #[test]
    fn a_stored_value_that_is_not_one() {
        let error = Changes::from_stored("{\"version\": 1,\n  oops").unwrap_err();
        assert_eq!(error.line, Some(2));
        assert!(error.column.is_some());
        assert!(error.to_string().contains("line 2"), "{error}");
        let error = Changes::from_stored("[1, 2]").unwrap_err();
        assert_eq!(error.line, None);
        assert!(error.to_string().contains("not a JSON object"), "{error}");
        // A list that is not a list is the only other thing that fails.
        assert!(Changes::from_stored("{\"changes\": 4}").is_err());
    }

    #[test]
    fn defaults_that_changed_since_the_entry_was_made() {
        let entry = |default: &[&str], wanted: &[&str]| ChangeEntry {
            command: "x".to_string(),
            input: None,
            context: Context::anywhere(),
            default: keys(default),
            keys: keys(wanted),
            extra: BTreeMap::new(),
        };
        // The user replaced cmd-b; the default is cmd-shift-b now: the
        // user's key stays beside the new default.
        let resolved = resolve(
            &keys(&["cmd-shift-b"]),
            Some(&entry(&["cmd-b"], &["cmd-alt-b"])),
        );
        assert_eq!(resolved.fates, vec![Fate::Kept]);
        assert_eq!(resolved.added, keys(&["cmd-alt-b"]));
        assert_eq!(resolved.row, keys(&["cmd-alt-b", "cmd-shift-b"]));
        assert_eq!(resolved.stale.len(), 1);

        // The user removed a key that is gone anyway: nothing left.
        let resolved = resolve(
            &keys(&["left"]),
            Some(&entry(&["ctrl-b", "left"], &["left"])),
        );
        assert!(!resolved.is_changed());
        assert_eq!(resolved.stale, vec![Stale::Removed(key("ctrl-b"))]);

        // A second default arrived: it is there, after the user's keys.
        let resolved = resolve(
            &keys(&["cmd-b", "f9"]),
            Some(&entry(&["cmd-b"], &["cmd-alt-b"])),
        );
        assert_eq!(
            resolved.fates,
            vec![Fate::Replaced(key("cmd-alt-b")), Fate::Kept]
        );
        assert_eq!(resolved.row, keys(&["cmd-alt-b", "f9"]));
        assert!(resolved.stale.is_empty());

        // The key the user added became the default: no change left.
        let resolved = resolve(
            &keys(&["cmd-b", "f5"]),
            Some(&entry(&["cmd-b"], &["cmd-b", "f5"])),
        );
        assert!(!resolved.is_changed());

        // The key the user replaced the default with became a default of
        // its own: the old default goes, the key is there once.
        let resolved = resolve(&keys(&["cmd-b", "f5"]), Some(&entry(&["cmd-b"], &["f5"])));
        assert_eq!(resolved.fates, vec![Fate::Removed, Fate::Kept]);
        assert_eq!(resolved.row, keys(&["f5"]));
    }
}
