//! The keymap that runs: the defaults with the user's changes applied.
//!
//! # Order
//!
//! The order of a keymap is its precedence (among bindings that apply at
//! the same depth the later one wins, and key hints show the last binding
//! of an action), so [`apply`] keeps it:
//!
//! - **A default that is not changed** stands where it stood, with the
//!   spelling it was handed over in. Without changes the result is the
//!   defaults, binding for binding.
//! - **A key replaced** stands where its default stood: in the position
//!   of the first binding of that default key (a key bound in two
//!   spellings has two bindings; the new key's spellings all go to the
//!   first one's position).
//! - **A key removed** leaves a gap.
//! - **A key added** to a target that has defaults goes immediately
//!   before the target's first default binding. It then relates to every
//!   other command's bindings as the target's own defaults do (a key the
//!   user adds to a command of the text ranks like the keys of the text),
//!   and the last binding of the target is still its default or what
//!   replaced it, so key hints and the menu bar keep showing that one and
//!   not the second key.
//! - **A key given to a target without defaults** goes to the very front
//!   of the keymap, in the order those targets were handed over
//!   ([`Defaults::keyless`]), so the order in which the user made the
//!   changes never shows in the keymap. A binding there
//!   loses every tie: where the key is also bound to something else at the
//!   same depth, the binding that was there before keeps it. So a key
//!   given to a command of the palette never takes a key away from a place
//!   that has it; [`crate::check`] says beforehand where that leaves the
//!   new key without effect.

use std::collections::HashMap;

use crate::changes::{Changes, Fate, Resolved, Slot, Stale, resolve};
use crate::defaults::{Defaults, Target};
use crate::keystroke::KeySequence;

/// Where an effective binding came from.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Origin {
    /// A default, as it was bound.
    Default,
    /// The user's key in the place of a default key.
    Changed {
        /// The default key it replaced.
        default: KeySequence,
    },
    /// A key the user added: beside the defaults, or to a command that
    /// had none.
    Added,
}

impl Origin {
    /// Whether the user made this key.
    pub fn is_users(&self) -> bool {
        !matches!(self, Origin::Default)
    }
}

/// One binding of the keymap to install.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectiveBinding {
    /// The target it belongs to.
    pub target: Target,
    /// Its key in normal form: the key to the user.
    pub keys: KeySequence,
    /// The keystrokes to bind, in a spelling GPUI reads. A key bound in
    /// two spellings is two bindings with the same `keys`.
    pub spelling: String,
    /// Where it came from.
    pub origin: Origin,
    /// The index (in [`Defaults::bindings`]) of a default binding with the
    /// same command and context, to copy the action and the context
    /// predicate from: the binding itself for a default, the one whose
    /// position it took for a changed key, the target's first for an
    /// added key. `None` only for a key given to a target without
    /// defaults: the app builds that action by its name and parses the
    /// context text.
    pub source: Option<usize>,
}

/// One key of a target, as a row shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetKey {
    /// The key.
    pub keys: KeySequence,
    /// Where it came from.
    pub origin: Origin,
}

/// A target with its keys: one row of the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetKeys {
    /// The command and where it applies.
    pub target: Target,
    /// Its keys, in the order the row shows them: the user's order for a
    /// changed target, the order of the defaults otherwise.
    pub keys: Vec<TargetKey>,
    /// Its default keys, in the order they are bound.
    pub default_keys: Vec<KeySequence>,
    /// Whether it differs from its defaults.
    pub changed: bool,
    /// The key that key hints and the menu bar show for the target: the
    /// last one bound. `None` when the target has no key.
    pub shown: Option<KeySequence>,
}

/// Why a stored change does nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InactiveReason {
    /// This version has no such command.
    UnknownCommand,
    /// This version has the command, but not in that context.
    UnknownPlace,
    /// An earlier entry is about the same target.
    Duplicate,
    /// The entry replaces a default key that is not a default any more;
    /// the user's key is kept as an added one.
    ReplacedKeyGone {
        /// The key that was replaced.
        from: KeySequence,
        /// The user's key.
        to: KeySequence,
    },
    /// The entry removes a default key that is not a default any more.
    RemovedKeyGone(KeySequence),
    /// This version cannot read the entry (another shape, a key it does
    /// not know).
    Unread,
}

/// A stored change, or a part of one, that does nothing in this version.
/// It stays stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InactiveChange {
    /// The target the entry is about; `None` for an entry that could not
    /// be read.
    pub target: Option<Target>,
    /// Why it does nothing.
    pub reason: InactiveReason,
}

/// The keymap that runs, and what it says about every target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Effective {
    bindings: Vec<EffectiveBinding>,
    targets: Vec<TargetKeys>,
    index: HashMap<Target, usize>,
    inactive: Vec<InactiveChange>,
}

impl Effective {
    /// The bindings to install, in order. See the [module docs](self).
    pub fn bindings(&self) -> &[EffectiveBinding] {
        &self.bindings
    }

    /// Every target of this version with its keys, in the order of
    /// [`Defaults::targets`].
    pub fn targets(&self) -> &[TargetKeys] {
        &self.targets
    }

    /// One target with its keys; `None` for a target this version does
    /// not have.
    pub fn target(&self, target: &Target) -> Option<&TargetKeys> {
        self.index.get(target).map(|ix| &self.targets[*ix])
    }

    /// The targets that differ from their defaults.
    pub fn changed(&self) -> impl Iterator<Item = &TargetKeys> {
        self.targets.iter().filter(|target| target.changed)
    }

    /// The stored changes that do nothing, and why.
    pub fn inactive(&self) -> &[InactiveChange] {
        &self.inactive
    }
}

/// Applies the user's changes to the defaults. The result is the keymap to
/// install (clear GPUI's, then bind [`Effective::bindings`] in order) and
/// the state of every target for the list.
pub fn apply(defaults: &Defaults, changes: &Changes) -> Effective {
    // The entry that counts for each target, and everything that does
    // nothing.
    let mut inactive = Vec::new();
    let mut resolved: HashMap<Target, Resolved> = HashMap::new();
    for slot in changes.slots() {
        let entry = match slot {
            Slot::Entry(entry) => entry,
            Slot::Unread(_) => {
                inactive.push(InactiveChange {
                    target: None,
                    reason: InactiveReason::Unread,
                });
                continue;
            }
        };
        let target = entry.target();
        let reason = if resolved.contains_key(&target) {
            Some(InactiveReason::Duplicate)
        } else if defaults.knows(&target) {
            None
        } else if defaults.knows_command(&target.command) {
            Some(InactiveReason::UnknownPlace)
        } else {
            Some(InactiveReason::UnknownCommand)
        };
        if let Some(reason) = reason {
            inactive.push(InactiveChange {
                target: Some(target),
                reason,
            });
            continue;
        }
        let state = resolve(&defaults.default_keys(&target), Some(entry));
        for stale in &state.stale {
            inactive.push(InactiveChange {
                target: Some(target.clone()),
                reason: match stale.clone() {
                    Stale::Replaced { from, to } => InactiveReason::ReplacedKeyGone { from, to },
                    Stale::Removed(key) => InactiveReason::RemovedKeyGone(key),
                },
            });
        }
        resolved.insert(target, state);
    }

    let mut bindings = Vec::new();
    let added = |target: &Target, state: &Resolved, source: Option<usize>| {
        let mut out = Vec::new();
        for keys in &state.added {
            for spelling in keys.spellings() {
                out.push(EffectiveBinding {
                    target: target.clone(),
                    keys: keys.clone(),
                    spelling,
                    origin: Origin::Added,
                    source,
                });
            }
        }
        out
    };

    // Keys given to targets without defaults: at the front.
    for target in defaults.keyless() {
        if let Some(state) = resolved.get(target) {
            bindings.extend(added(target, state, None));
        }
    }

    for (index, binding) in defaults.bindings().iter().enumerate() {
        let target = binding.target();
        let Some(state) = resolved.get(&target) else {
            bindings.push(EffectiveBinding {
                target,
                keys: binding.keys.clone(),
                spelling: binding.spelling.clone(),
                origin: Origin::Default,
                source: Some(index),
            });
            continue;
        };
        let keys = defaults.keys_of(&target);
        // Added keys: immediately before the target's first binding.
        if keys.first().and_then(|key| key.indices.first()) == Some(&index) {
            bindings.extend(added(&target, state, Some(index)));
        }
        let Some(position) = keys.iter().position(|key| key.keys == binding.keys) else {
            continue;
        };
        match &state.fates[position] {
            Fate::Kept => bindings.push(EffectiveBinding {
                target,
                keys: binding.keys.clone(),
                spelling: binding.spelling.clone(),
                origin: Origin::Default,
                source: Some(index),
            }),
            Fate::Removed => {}
            Fate::Replaced(to) => {
                if keys[position].indices.first() == Some(&index) {
                    for spelling in to.spellings() {
                        bindings.push(EffectiveBinding {
                            target: target.clone(),
                            keys: to.clone(),
                            spelling,
                            origin: Origin::Changed {
                                default: binding.keys.clone(),
                            },
                            source: Some(index),
                        });
                    }
                }
            }
        }
    }

    // The state of every target.
    let mut shown: HashMap<&Target, &KeySequence> = HashMap::new();
    for binding in &bindings {
        shown.insert(&binding.target, &binding.keys);
    }
    let mut targets = Vec::with_capacity(defaults.targets().len());
    let mut index = HashMap::with_capacity(defaults.targets().len());
    for target in defaults.targets() {
        let default_keys = defaults.default_keys(target);
        let (keys, changed) = match resolved.get(target) {
            None => (
                default_keys
                    .iter()
                    .map(|keys| TargetKey {
                        keys: keys.clone(),
                        origin: Origin::Default,
                    })
                    .collect(),
                false,
            ),
            Some(state) => {
                let origin = |keys: &KeySequence| {
                    for (default, fate) in default_keys.iter().zip(&state.fates) {
                        match fate {
                            Fate::Kept if default == keys => return Origin::Default,
                            Fate::Replaced(to) if to == keys => {
                                return Origin::Changed {
                                    default: default.clone(),
                                };
                            }
                            _ => {}
                        }
                    }
                    Origin::Added
                };
                (
                    state
                        .row
                        .iter()
                        .map(|keys| TargetKey {
                            keys: keys.clone(),
                            origin: origin(keys),
                        })
                        .collect(),
                    state.is_changed(),
                )
            }
        };
        index.insert(target.clone(), targets.len());
        targets.push(TargetKeys {
            target: target.clone(),
            keys,
            default_keys,
            changed,
            shown: shown.get(target).map(|keys| (*keys).clone()),
        });
    }

    Effective {
        bindings,
        targets,
        index,
        inactive,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::defaults::DefaultBinding;

    fn key(text: &str) -> KeySequence {
        KeySequence::parse(text).unwrap()
    }

    fn defaults() -> Defaults {
        let binding = |action: &str, context: Option<&str>, keys: &str| {
            DefaultBinding::new(action, None, context, keys).unwrap()
        };
        let mut defaults = Defaults::from_bindings([
            binding("app::Quit", None, "cmd-q"),
            binding("editor::MoveLeft", Some("Editor"), "ctrl-b"),
            binding("editor::MoveLeft", Some("Editor"), "left"),
            binding("editor::ToggleBold", Some("Editor"), "cmd-b"),
            binding("editor::ToggleNumberedList", Some("Editor"), "cmd-&"),
            binding("editor::ToggleNumberedList", Some("Editor"), "cmd-shift-7"),
            binding("comments::Toggle", None, "alt-cmd-c"),
        ]);
        defaults.add_keyless(Target::new("app::VersionHistory", None));
        defaults
    }

    fn spellings(effective: &Effective) -> Vec<&str> {
        effective
            .bindings()
            .iter()
            .map(|binding| binding.spelling.as_str())
            .collect()
    }

    #[test]
    fn without_changes_the_keymap_is_the_defaults_binding_for_binding() {
        let defaults = defaults();
        let effective = apply(&defaults, &Changes::new());
        assert_eq!(effective.bindings().len(), defaults.bindings().len());
        for (index, (binding, default)) in effective
            .bindings()
            .iter()
            .zip(defaults.bindings())
            .enumerate()
        {
            assert_eq!(binding.spelling, default.spelling);
            assert_eq!(binding.target, default.target());
            assert_eq!(binding.origin, Origin::Default);
            assert_eq!(binding.source, Some(index));
        }
        // As handed over, not in this crate's own spelling.
        assert_eq!(effective.bindings()[6].spelling, "alt-cmd-c");
        assert_eq!(effective.bindings()[6].keys, key("cmd-alt-c"));
        assert_eq!(effective.changed().count(), 0);
        assert!(effective.inactive().is_empty());
        assert_eq!(effective.targets().len(), 6);
    }

    #[test]
    fn a_changed_key_stands_where_its_default_stood() {
        let defaults = defaults();
        let mut changes = Changes::new();
        let bold = Target::new("editor::ToggleBold", Some("Editor"));
        changes.replace_key(&defaults, &bold, &key("cmd-b"), key("cmd-alt-b"));
        let effective = apply(&defaults, &changes);
        assert_eq!(
            spellings(&effective),
            [
                "cmd-q",
                "ctrl-b",
                "left",
                "cmd-alt-b",
                "cmd-&",
                "cmd-shift-7",
                "alt-cmd-c"
            ]
        );
        let binding = &effective.bindings()[3];
        assert_eq!(
            binding.origin,
            Origin::Changed {
                default: key("cmd-b")
            }
        );
        assert_eq!(binding.source, Some(3));
        let row = effective.target(&bold).unwrap();
        assert!(row.changed);
        assert_eq!(row.default_keys, vec![key("cmd-b")]);
        assert_eq!(row.shown, Some(key("cmd-alt-b")));
        assert_eq!(effective.changed().count(), 1);
    }

    #[test]
    fn a_key_in_two_spellings_is_replaced_and_given_as_one() {
        let defaults = defaults();
        let mut changes = Changes::new();
        let list = Target::new("editor::ToggleNumberedList", Some("Editor"));
        let bold = Target::new("editor::ToggleBold", Some("Editor"));
        // One spelling in the place of two.
        changes.replace_key(&defaults, &list, &key("cmd-shift-7"), key("cmd-alt-l"));
        // Two spellings in the place of one.
        changes.replace_key(&defaults, &bold, &key("cmd-b"), key("cmd-shift-8"));
        let effective = apply(&defaults, &changes);
        assert_eq!(
            spellings(&effective),
            [
                "cmd-q",
                "ctrl-b",
                "left",
                "cmd-*",
                "cmd-shift-8",
                "cmd-alt-l",
                "alt-cmd-c"
            ]
        );
        assert_eq!(effective.bindings()[3].source, Some(3));
        assert_eq!(effective.bindings()[4].source, Some(3));
        assert_eq!(effective.bindings()[5].source, Some(4));
        assert_eq!(effective.target(&list).unwrap().keys.len(), 1);
        assert_eq!(
            effective.target(&bold).unwrap().shown,
            Some(key("cmd-shift-8"))
        );
    }

    #[test]
    fn a_removed_key_leaves_a_gap_and_a_command_can_be_left_without_a_key() {
        let defaults = defaults();
        let mut changes = Changes::new();
        let left = Target::new("editor::MoveLeft", Some("Editor"));
        let bold = Target::new("editor::ToggleBold", Some("Editor"));
        changes.remove_key(&defaults, &left, &key("ctrl-b"));
        changes.remove_key(&defaults, &bold, &key("cmd-b"));
        let effective = apply(&defaults, &changes);
        assert_eq!(
            spellings(&effective),
            ["cmd-q", "left", "cmd-&", "cmd-shift-7", "alt-cmd-c"]
        );
        let row = effective.target(&bold).unwrap();
        assert!(row.keys.is_empty());
        assert!(row.changed);
        assert_eq!(row.shown, None);
        assert_eq!(effective.target(&left).unwrap().shown, Some(key("left")));
    }

    #[test]
    fn an_added_key_goes_before_the_target_s_defaults() {
        let defaults = defaults();
        let mut changes = Changes::new();
        let left = Target::new("editor::MoveLeft", Some("Editor"));
        changes.add_key(&defaults, &left, key("alt-h"));
        changes.add_key(&defaults, &left, key("cmd-shift-,"));
        let effective = apply(&defaults, &changes);
        assert_eq!(
            spellings(&effective),
            [
                "cmd-q",
                "alt-h",
                "cmd-<",
                "cmd-shift-,",
                "ctrl-b",
                "left",
                "cmd-b",
                "cmd-&",
                "cmd-shift-7",
                "alt-cmd-c"
            ]
        );
        assert_eq!(effective.bindings()[1].origin, Origin::Added);
        assert_eq!(effective.bindings()[1].source, Some(1));
        let row = effective.target(&left).unwrap();
        // The row: the defaults, then what was added. Hints still show the
        // default.
        let row_keys: Vec<String> = row.keys.iter().map(|key| key.keys.to_string()).collect();
        assert_eq!(row_keys, ["ctrl-b", "left", "alt-h", "cmd-shift-,"]);
        assert_eq!(row.keys[2].origin, Origin::Added);
        assert_eq!(row.shown, Some(key("left")));

        // With every default removed the added key is what is shown.
        changes.remove_key(&defaults, &left, &key("ctrl-b"));
        changes.remove_key(&defaults, &left, &key("left"));
        let effective = apply(&defaults, &changes);
        let row = effective.target(&left).unwrap();
        assert_eq!(row.keys.len(), 2);
        assert!(row.shown.is_some());
    }

    #[test]
    fn a_key_for_a_command_without_one_goes_to_the_front() {
        let defaults = defaults();
        let mut changes = Changes::new();
        let history = Target::new("app::VersionHistory", None);
        changes.add_key(&defaults, &history, key("cmd-alt-h"));
        let effective = apply(&defaults, &changes);
        assert_eq!(spellings(&effective)[0], "cmd-alt-h");
        let binding = &effective.bindings()[0];
        assert_eq!(binding.origin, Origin::Added);
        assert_eq!(binding.source, None);
        assert_eq!(binding.target, history);
        assert_eq!(effective.bindings().len(), defaults.bindings().len() + 1);
        let row = effective.target(&history).unwrap();
        assert!(row.changed);
        assert!(row.default_keys.is_empty());
        assert_eq!(row.shown, Some(key("cmd-alt-h")));
    }

    #[test]
    fn stored_changes_that_do_nothing_are_named() {
        let defaults = defaults();
        let changes = Changes::from_stored(
            r#"{"changes":[
                {"command":"app::Gone","context":null,"default":[],"keys":["f7"]},
                {"command":"editor::ToggleBold","context":"LibraryPanel","default":[],"keys":["f8"]},
                {"command":"editor::ToggleBold","context":"Editor","default":["cmd-b"],"keys":["f9"]},
                {"command":"editor::ToggleBold","context":"Editor","default":["cmd-b"],"keys":["f10"]},
                {"command":"editor::MoveLeft","context":"Editor","default":["ctrl-h","left"],"keys":["alt-h","left"]},
                {"what":"is this"}
            ]}"#,
        )
        .unwrap();
        let effective = apply(&defaults, &changes);
        let reasons: Vec<&InactiveReason> = effective
            .inactive()
            .iter()
            .map(|change| &change.reason)
            .collect();
        assert_eq!(
            reasons,
            [
                &InactiveReason::UnknownCommand,
                &InactiveReason::UnknownPlace,
                &InactiveReason::Duplicate,
                &InactiveReason::ReplacedKeyGone {
                    from: key("ctrl-h"),
                    to: key("alt-h")
                },
                &InactiveReason::Unread,
            ]
        );
        // None of them is bound, except the user's key that outlived its
        // default.
        let all = spellings(&effective);
        assert!(!all.contains(&"f7") && !all.contains(&"f8") && !all.contains(&"f10"));
        assert!(all.contains(&"f9"));
        assert_eq!(all[1], "alt-h");
        assert!(all.contains(&"ctrl-b"), "the new default is there");
        // And all of it is still stored.
        assert_eq!(changes.entries().count(), 5);
        assert_eq!(changes.unread().count(), 1);
    }
}
