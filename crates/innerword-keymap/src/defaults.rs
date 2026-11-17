//! The defaults as the app hands them over: every binding of the assembled
//! keymap, in order, and the commands that have no key.

use std::collections::HashMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::context::Context;
use crate::keystroke::{KeySequence, KeystrokeError};

/// A command: a GPUI action by its name (`"editor::ToggleBold"`), with its
/// input where the action has one (the JSON text GPUI's
/// `KeyBinding::action_input` gives; the actions of Inner Word have none).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Command {
    /// The action's name, as `Action::name` gives it.
    pub action: String,
    /// The action's input as JSON text, for an action that takes one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
}

impl Command {
    /// The command of an action without input.
    pub fn new(action: &str) -> Self {
        Self {
            action: action.to_string(),
            input: None,
        }
    }

    /// The command of an action with the input `input` (JSON text).
    pub fn with_input(action: &str, input: Option<&str>) -> Self {
        Self {
            action: action.to_string(),
            input: input.map(str::to_string),
        }
    }
}

impl fmt::Display for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.action)?;
        if let Some(input) = &self.input {
            write!(f, " {input}")?;
        }
        Ok(())
    }
}

/// A command in a context: the unit a key belongs to, and what the user
/// changes. "Open context menu" in the text and "Open context menu" in the
/// library are two targets, each with keys of its own.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Target {
    /// The command.
    pub command: Command,
    /// Where its keys apply.
    pub context: Context,
}

impl Target {
    /// The target of an action without input in the context `context`
    /// (`None`: anywhere).
    pub fn new(action: &str, context: Option<&str>) -> Self {
        Self {
            command: Command::new(action),
            context: Context::from_option(context),
        }
    }

    /// The target of `command` in `context`.
    pub fn of(command: Command, context: Context) -> Self {
        Self { command, context }
    }
}

impl fmt::Display for Target {
    /// `editor::ToggleBold in Editor`, `app::Quit anywhere`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.context.text() {
            Some(context) => write!(f, "{} in {context}", self.command),
            None => write!(f, "{} anywhere", self.command),
        }
    }
}

/// One binding of the default keymap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefaultBinding {
    /// The command it runs.
    pub command: Command,
    /// Where it applies.
    pub context: Context,
    /// Its key in normal form: `cmd-&` and `cmd-shift-7` are the same key
    /// here (see [`crate::keystroke`]).
    pub keys: KeySequence,
    /// Its keystrokes as they were handed over, which is what is bound
    /// again while the user has not changed the key.
    pub spelling: String,
}

impl DefaultBinding {
    /// A default binding from what a GPUI `KeyBinding` gives: the action's
    /// name, its input (`action_input()`), the text of its context
    /// predicate (`predicate().map(|p| p.to_string())`), and its
    /// keystrokes as text (each `unparse()`d, joined by a space; any
    /// spelling GPUI reads will do).
    ///
    /// Fails only for keystrokes this crate does not read, which no
    /// binding of the app has (a test holds the corpus of the tree's keys
    /// against the parser): such a binding is bound as it is and not
    /// handed over.
    pub fn new(
        action: &str,
        input: Option<&str>,
        context: Option<&str>,
        keystrokes: &str,
    ) -> Result<Self, KeystrokeError> {
        Ok(Self {
            command: Command::with_input(action, input),
            context: Context::from_option(context),
            keys: KeySequence::parse(keystrokes)?,
            spelling: keystrokes.split_whitespace().collect::<Vec<_>>().join(" "),
        })
    }

    /// The target the binding belongs to.
    pub fn target(&self) -> Target {
        Target::of(self.command.clone(), self.context.clone())
    }
}

/// One default key of a target: a key to the user, and the bindings that
/// make it (two for a key bound in two spellings).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DefaultKey {
    pub keys: KeySequence,
    /// Indices into [`Defaults::bindings`], ascending.
    pub indices: Vec<usize>,
}

/// The default keymap: the bindings in the order they were bound (the
/// order is the precedence: among bindings that apply at the same depth,
/// the later one wins), and the commands without a key.
///
/// Bindings of one command in one context with the same key in normal form
/// are one key of that target, wherever they stand: that is how the two
/// spellings of shift with a digit or punctuation become one key.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Defaults {
    bindings: Vec<DefaultBinding>,
    keyless: Vec<Target>,
    /// Every target, in the order of its first appearance: the bindings'
    /// first, then the keyless ones.
    targets: Vec<Target>,
    keys: HashMap<Target, Vec<DefaultKey>>,
}

impl Defaults {
    /// No bindings yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// The defaults with these bindings, in this order.
    pub fn from_bindings(bindings: impl IntoIterator<Item = DefaultBinding>) -> Self {
        let mut defaults = Self::new();
        for binding in bindings {
            defaults.push(binding);
        }
        defaults
    }

    /// Adds the next binding of the keymap. Its index is the number of
    /// bindings pushed before it, which is what
    /// [`crate::EffectiveBinding::source`] refers to.
    pub fn push(&mut self, binding: DefaultBinding) {
        let index = self.bindings.len();
        let target = binding.target();
        if !self.keys.contains_key(&target) {
            // A keyless target that gets a binding after all moves to its
            // binding's place in the order.
            self.keyless.retain(|keyless| *keyless != target);
            self.targets.retain(|known| *known != target);
            self.targets.push(target.clone());
        }
        let keys = self.keys.entry(target).or_default();
        match keys.iter_mut().find(|key| key.keys == binding.keys) {
            Some(key) => key.indices.push(index),
            None => keys.push(DefaultKey {
                keys: binding.keys.clone(),
                indices: vec![index],
            }),
        }
        self.bindings.push(binding);
    }

    /// Says that `target` exists and has no default key (a command of the
    /// palette such as "Version history", with the context a key for it
    /// would apply in). The user can give it one; without this the change
    /// would count as one for a command this version does not have.
    /// Nothing happens for a target that has a binding.
    pub fn add_keyless(&mut self, target: Target) {
        if !self.keys.contains_key(&target) && !self.keyless.contains(&target) {
            self.keyless.push(target.clone());
            self.targets.push(target);
        }
    }

    /// The bindings, in the order they were pushed.
    pub fn bindings(&self) -> &[DefaultBinding] {
        &self.bindings
    }

    /// Every target: those with a binding in the order of their first
    /// binding, then those without a key in the order they were added.
    pub fn targets(&self) -> &[Target] {
        &self.targets
    }

    /// The targets without a default key.
    pub fn keyless(&self) -> &[Target] {
        &self.keyless
    }

    /// Whether `target` exists in this version: it has a default binding,
    /// or it was added as a command without a key.
    pub fn knows(&self, target: &Target) -> bool {
        self.keys.contains_key(target) || self.keyless.contains(target)
    }

    /// Whether `command` exists in this version, in any context.
    pub fn knows_command(&self, command: &Command) -> bool {
        self.targets.iter().any(|target| target.command == *command)
    }

    /// The default keys of `target`, in the order they were bound, each
    /// key once (a key bound in two spellings is one key). Empty for a
    /// target without a default key and for one this version does not
    /// have.
    pub fn default_keys(&self, target: &Target) -> Vec<KeySequence> {
        self.keys_of(target)
            .iter()
            .map(|key| key.keys.clone())
            .collect()
    }

    /// The default keys of `target` with the bindings that make them.
    pub(crate) fn keys_of(&self, target: &Target) -> &[DefaultKey] {
        self.keys.get(target).map(Vec::as_slice).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(action: &str, context: Option<&str>, keys: &str) -> DefaultBinding {
        DefaultBinding::new(action, None, context, keys).unwrap()
    }

    #[test]
    fn two_spellings_are_one_key_of_the_target() {
        let defaults = Defaults::from_bindings([
            binding("editor::ToggleBold", Some("Editor"), "cmd-b"),
            binding("editor::ToggleNumberedList", Some("Editor"), "cmd-&"),
            binding("editor::ToggleNumberedList", Some("Editor"), "cmd-shift-7"),
            binding("editor::MoveLeft", Some("Editor"), "ctrl-b"),
            binding("editor::MoveLeft", Some("Editor"), "left"),
        ]);
        let list = Target::new("editor::ToggleNumberedList", Some("Editor"));
        assert_eq!(
            defaults.default_keys(&list),
            vec![KeySequence::parse("cmd-shift-7").unwrap()]
        );
        assert_eq!(defaults.keys_of(&list)[0].indices, vec![1, 2]);
        // The spelling handed over is kept.
        assert_eq!(defaults.bindings()[1].spelling, "cmd-&");
        let left = Target::new("editor::MoveLeft", Some("Editor"));
        assert_eq!(defaults.default_keys(&left).len(), 2);
        assert_eq!(defaults.targets().len(), 3);
    }

    #[test]
    fn the_same_command_in_two_contexts_is_two_targets() {
        let mut defaults = Defaults::from_bindings([
            binding("context_menu::Open", Some("LibraryPanel"), "ctrl-enter"),
            binding("context_menu::Open", Some("(TextField)"), "ctrl-enter"),
            binding("app::Quit", None, "cmd-q"),
        ]);
        assert_eq!(defaults.targets().len(), 3);
        assert!(defaults.knows(&Target::new("context_menu::Open", Some("TextField"))));
        assert!(!defaults.knows(&Target::new("context_menu::Open", None)));
        assert!(defaults.knows_command(&Command::new("context_menu::Open")));
        assert!(!defaults.knows_command(&Command::new("app::Gone")));

        let history = Target::new("app::VersionHistory", None);
        defaults.add_keyless(history.clone());
        defaults.add_keyless(history.clone());
        defaults.add_keyless(Target::new("app::Quit", None));
        assert_eq!(defaults.keyless(), std::slice::from_ref(&history));
        assert!(defaults.knows(&history));
        assert!(defaults.default_keys(&history).is_empty());
        assert_eq!(defaults.targets().len(), 4);

        // A binding for it after all: no longer keyless.
        defaults.push(binding("app::VersionHistory", None, "cmd-alt-h"));
        assert!(defaults.keyless().is_empty());
        assert_eq!(defaults.targets().len(), 4);
        assert_eq!(defaults.default_keys(&history).len(), 1);
    }

    #[test]
    fn a_target_prints_as_a_person_reads_it() {
        assert_eq!(
            Target::new("editor::ToggleBold", Some("Editor")).to_string(),
            "editor::ToggleBold in Editor"
        );
        assert_eq!(
            Target::new("app::Quit", None).to_string(),
            "app::Quit anywhere"
        );
    }
}
