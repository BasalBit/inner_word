//! The shortcuts file: every key of the app as JSON a person can read and
//! edit, written by [`export`] and read by [`plan_import`].
//!
//! # The format
//!
//! One JSON object ("the-writer-shortcuts", version 1):
//!
//! ```json
//! {
//!   "format": "the-writer-shortcuts",
//!   "version": 1,
//!   "app_version": "0.5.0",
//!   "about": "The keyboard shortcuts of The Writer. ...",
//!   "shortcuts": [
//!     {
//!       "command": "editor::ToggleBold",
//!       "title": "Bold",
//!       "where": "In the text",
//!       "context": "Editor",
//!       "default": ["cmd-b"],
//!       "keys": ["cmd-alt-b"]
//!     },
//!     {
//!       "command": "workspace::VersionHistory",
//!       "title": "Version history",
//!       "where": "Anywhere",
//!       "context": null,
//!       "default": [],
//!       "keys": []
//!     }
//!   ]
//! }
//! ```
//!
//! - `format` and `version` say what the file is. A file with another
//!   format name, or a version later than this one, is not read.
//! - `app_version` is the version of the app that wrote it; `about` is a
//!   sentence for whoever opens the file. Neither is used when reading.
//! - `shortcuts` has one entry for every command in every place it has (or
//!   can be given) a key, in the order of the app's list:
//!   - `command`: the action's name. With `context` (the key context the
//!     keys apply in, `null` for anywhere in the window) it says which
//!     command where; `input` is there too for an action with input.
//!     These are what an import goes by.
//!   - `title` and `where`: the command's name and its place in words, as
//!     the list shows them. For reading only.
//!   - `default`: the keys the command has by default **in the version
//!     that wrote the file**.
//!   - `keys`: the keys it has. To change a key, edit this list.
//! - A key is written as GPUI writes it: modifiers and a key joined by
//!   `-` (`cmd`, `ctrl`, `alt` for the option key, `shift`, `fn`), as in
//!   `cmd-shift-p`, `alt-up`, `ctrl-enter`, `f2`; keys pressed one after
//!   the other are separated by a space (`g g`). Shift with a digit or a
//!   punctuation key is written with the key, not the character:
//!   `cmd-shift-7`, not `cmd-&` (both are read). `opt`, `option`,
//!   `command`, `control`, `esc` and `return` are read too.
//!
//! # What an import takes from a file
//!
//! **Only what differs from the default written in the same entry counts
//! as a change.** An entry whose `keys` are its `default` says "this
//! command is not changed", whatever the defaults of the reading version
//! are, so a file from an older version does not undo newer defaults. An
//! entry without `default` (written by hand) is compared with the
//! defaults of the reading version. Entries may be left out; the order
//! does not matter; fields that are not named above are ignored.
//!
//! Nothing is applied by reading. [`plan_import`] gives an
//! [`ImportPlan`]: the changes the file makes, the entries it skips with
//! the reason for each (a command this version does not have, a key that
//! cannot be read, a key a guard rail refuses), and for both ways of
//! carrying it out ([`ImportMode`]) the keys that change and the
//! conflicts that would be left. Carrying it out gives the new changes
//! and the old ones, for Undo.

use std::fmt;

use serde_json::Value;

use crate::changes::{ChangeEntry, Changes, diff, json_problem, resolve};
use crate::context::{Context, Places};
use crate::defaults::{Command, Defaults, Target};
use crate::effective::apply;
use crate::keystroke::{KeySequence, KeystrokeError};
use crate::verdict::{Absent, Collision, Listed, Refusal, collisions_in, guard};

/// The `format` of a shortcuts file.
pub const FILE_FORMAT: &str = "the-writer-shortcuts";

/// The `version` of the format this crate writes, and the latest it
/// reads.
pub const FILE_VERSION: u32 = 1;

/// The name the save panel suggests.
pub const FILE_NAME: &str = "The Writer Shortcuts.json";

/// The sentence at the top of an exported file.
const ABOUT: &str = "The keyboard shortcuts of The Writer. To change a key, edit \"keys\" and \
    import the file in Keyboard shortcuts. A key is written like cmd-shift-p (alt is the option \
    key); keys pressed one after the other are separated by a space. \"default\" is the key of \
    the version that wrote this file: leave it as it is.";

/// What the caller knows about a command in a place, for a person to
/// read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandInfo {
    /// The command and its context.
    pub target: Target,
    /// The command's name as the list shows it ("Bold").
    pub title: String,
    /// The place in words ("In the text").
    pub place: String,
}

/// The names and places of the commands, in the order the app lists
/// them. Decides the order of an exported file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Catalogue {
    entries: Vec<CommandInfo>,
}

impl Catalogue {
    /// An empty catalogue.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a command in a place with its name and the place in words.
    /// The first entry for a target counts.
    pub fn add(&mut self, target: Target, title: &str, place: &str) {
        if self.info(&target).is_none() {
            self.entries.push(CommandInfo {
                target,
                title: title.to_string(),
                place: place.to_string(),
            });
        }
    }

    /// What is known about `target`.
    pub fn info(&self, target: &Target) -> Option<&CommandInfo> {
        self.entries.iter().find(|info| info.target == *target)
    }

    /// The entries, in the order they were added.
    pub fn entries(&self) -> &[CommandInfo] {
        &self.entries
    }
}

fn quoted(text: &str) -> String {
    serde_json::to_string(text).expect("a string is JSON")
}

fn key_list(keys: &[KeySequence]) -> String {
    let keys: Vec<String> = keys.iter().map(|key| quoted(&key.to_string())).collect();
    format!("[{}]", keys.join(", "))
}

/// The shortcuts file for the keymap as it is: every target of
/// `defaults` (those with a key and those without), with its title and
/// place from `catalogue`, its default keys and the keys it has with
/// `changes`. `app_version` is written into the file.
///
/// The order is the catalogue's, then the targets it does not name in the
/// order of the defaults (their title is the action's name, their place
/// the context or "Anywhere"). The same keymap always gives the same
/// text. Stored changes that do nothing in this version (see
/// [`crate::Effective::inactive`]) are not written.
pub fn export(
    defaults: &Defaults,
    changes: &Changes,
    catalogue: &Catalogue,
    app_version: &str,
) -> String {
    let effective = apply(defaults, changes);
    let mut order: Vec<&Target> = Vec::new();
    for info in catalogue.entries() {
        if defaults.knows(&info.target) && !order.contains(&&info.target) {
            order.push(&info.target);
        }
    }
    let named = order.len();
    for target in defaults.targets() {
        if !order[..named].contains(&target) {
            order.push(target);
        }
    }

    let mut out = String::new();
    out.push_str("{\n");
    out.push_str(&format!("  \"format\": {},\n", quoted(FILE_FORMAT)));
    out.push_str(&format!("  \"version\": {FILE_VERSION},\n"));
    out.push_str(&format!("  \"app_version\": {},\n", quoted(app_version)));
    out.push_str(&format!("  \"about\": {},\n", quoted(ABOUT)));
    out.push_str("  \"shortcuts\": [");
    for (ix, target) in order.iter().enumerate() {
        let (title, place) = match catalogue.info(target) {
            Some(info) => (info.title.clone(), info.place.clone()),
            None => (
                target.command.action.clone(),
                target.context.text().unwrap_or("Anywhere").to_string(),
            ),
        };
        let state = effective.target(target);
        let default = state
            .map(|state| state.default_keys.clone())
            .unwrap_or_default();
        let keys: Vec<KeySequence> = state
            .map(|state| state.keys.iter().map(|key| key.keys.clone()).collect())
            .unwrap_or_default();
        out.push_str(if ix == 0 { "\n" } else { ",\n" });
        out.push_str("    {\n");
        out.push_str(&format!(
            "      \"command\": {},\n",
            quoted(&target.command.action)
        ));
        if let Some(input) = &target.command.input {
            out.push_str(&format!("      \"input\": {},\n", quoted(input)));
        }
        out.push_str(&format!("      \"title\": {},\n", quoted(&title)));
        out.push_str(&format!("      \"where\": {},\n", quoted(&place)));
        match target.context.text() {
            Some(context) => out.push_str(&format!("      \"context\": {},\n", quoted(context))),
            None => out.push_str("      \"context\": null,\n"),
        }
        out.push_str(&format!("      \"default\": {},\n", key_list(&default)));
        out.push_str(&format!("      \"keys\": {}\n", key_list(&keys)));
        out.push_str("    }");
    }
    out.push_str(if order.is_empty() { "]\n" } else { "\n  ]\n" });
    out.push_str("}\n");
    out
}

/// A text that is not a shortcuts file: what is wrong and where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImportError {
    /// The text is not JSON.
    NotJson {
        /// The line the problem is on, from 1.
        line: usize,
        /// The column on that line, from 1.
        column: usize,
        /// What the JSON reader says is wrong.
        problem: String,
    },
    /// The text is JSON, but not an object (a list, a number, ...).
    NotAnObject,
    /// The object has no `"format"`: it is some other JSON file.
    NoFormat,
    /// `"format"` names something else.
    OtherFormat {
        /// The format the file names.
        found: String,
    },
    /// `"version"` is missing or not a whole number from 1.
    BadVersion,
    /// The file was written by a later version of the format.
    NewerVersion {
        /// The version the file names.
        found: u64,
        /// The latest version this crate reads.
        supported: u32,
    },
    /// `"shortcuts"` is missing or not a list.
    NoShortcuts,
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImportError::NotJson {
                line,
                column,
                problem,
            } => write!(
                f,
                "This is not a shortcuts file: it is not JSON. Line {line}, column {column}: {problem}."
            ),
            ImportError::NotAnObject => write!(
                f,
                "This is not a shortcuts file: it is JSON, but not an object with \"format\" and \"shortcuts\"."
            ),
            ImportError::NoFormat => write!(
                f,
                "This is not a shortcuts file: it has no \"format\". A shortcuts file starts with \"format\": \"{FILE_FORMAT}\"."
            ),
            ImportError::OtherFormat { found } => write!(
                f,
                "This is not a shortcuts file: its \"format\" is \"{found}\", not \"{FILE_FORMAT}\"."
            ),
            ImportError::BadVersion => write!(
                f,
                "This shortcuts file cannot be read: its \"version\" is missing or not a number."
            ),
            ImportError::NewerVersion { found, supported } => write!(
                f,
                "This shortcuts file was written by a later version of The Writer (format version {found}; this version reads up to {supported})."
            ),
            ImportError::NoShortcuts => write!(
                f,
                "This shortcuts file cannot be read: it has no list \"shortcuts\"."
            ),
        }
    }
}

impl std::error::Error for ImportError {}

/// Why an entry of the file is skipped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SkipReason {
    /// The element of `shortcuts` is not an object.
    NotAnEntry,
    /// The entry has no `command` (or it is not a text).
    NoCommand,
    /// This version does not have the command.
    UnknownCommand,
    /// This version has the command, but not in the entry's context.
    UnknownPlace,
    /// A field is not what it has to be (`keys` is missing or not a list
    /// of texts, `context` is not a text).
    BadField {
        /// The field's name.
        field: &'static str,
    },
    /// A key cannot be read.
    UnreadableKey {
        /// The field it is in (`keys` or `default`).
        field: &'static str,
        /// The key and what is wrong with it.
        error: KeystrokeError,
    },
    /// A guard rail refuses a key the entry gives the command.
    Refused {
        /// The key.
        key: KeySequence,
        /// Why.
        refusal: Refusal,
    },
    /// An earlier entry of the file is about the same command in the
    /// same place.
    Duplicate,
}

impl fmt::Display for SkipReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SkipReason::NotAnEntry => write!(f, "it is not an entry"),
            SkipReason::NoCommand => write!(f, "it names no command"),
            SkipReason::UnknownCommand => write!(f, "this version does not have the command"),
            SkipReason::UnknownPlace => {
                write!(f, "this version does not have the command in that place")
            }
            SkipReason::BadField { field } => match *field {
                "keys" | "default" => write!(
                    f,
                    "its \"{field}\" is not a list of keys in quotes (like [\"cmd-b\", \"f5\"])"
                ),
                _ => write!(f, "its \"{field}\" is not a text in quotes"),
            },
            SkipReason::UnreadableKey { error, .. } => write!(f, "a key cannot be read: {error}"),
            // A key that was read is named as the list names keys.
            SkipReason::Refused { key, refusal } => {
                let key = key.glyphs();
                match refusal {
                    Refusal::TypesText => write!(
                        f,
                        "{key} is a character, and text is typed where the command applies"
                    ),
                    Refusal::SystemKeeps(system) => {
                        write!(f, "macOS keeps {key} for itself: it {}", system.what)
                    }
                    Refusal::AlreadyHasKey | Refusal::UnknownCommand => {
                        write!(f, "{key} cannot be given to the command")
                    }
                }
            }
            SkipReason::Duplicate => write!(f, "an earlier entry is about the same command"),
        }
    }
}

/// An entry of the file that is not applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skipped {
    /// Which entry of `shortcuts`, from 1.
    pub entry: usize,
    /// The command it names, when it names one.
    pub command: Option<String>,
    /// The title written in the entry, when there is one.
    pub title: Option<String>,
    /// The place written in the entry (`where`), when there is one.
    pub place: Option<String>,
    /// Why it is skipped.
    pub reason: SkipReason,
}

impl fmt::Display for Skipped {
    /// "Entry 12 (Bold, editor::ToggleBold): a key cannot be read: ..."
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Entry {}", self.entry)?;
        match (&self.title, &self.command) {
            (Some(title), Some(command)) => write!(f, " ({title}, {command})")?,
            (None, Some(name)) | (Some(name), None) => write!(f, " ({name})")?,
            (None, None) => {}
        }
        write!(f, ": {}", self.reason)
    }
}

/// A change the file makes: an entry whose keys differ from the default
/// written beside them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileChange {
    /// The command and its place.
    pub target: Target,
    /// The default keys the file was written against.
    pub default: Vec<KeySequence>,
    /// The keys the file gives the command.
    pub keys: Vec<KeySequence>,
}

/// The two ways to carry out an import.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ImportMode {
    /// The file's changes replace the user's own: afterwards only what
    /// the file changes differs from the defaults.
    Replace,
    /// The file's changes are added to the user's own. Where both change
    /// the same command, the file wins.
    Merge,
}

/// A command whose keys an import changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyDifference {
    /// The command and its place.
    pub target: Target,
    /// Its keys now.
    pub before: Vec<KeySequence>,
    /// Its keys after the import.
    pub after: Vec<KeySequence>,
}

/// What one way of carrying out an import would give.
#[derive(Clone, Debug, PartialEq)]
pub struct ImportOutcome {
    /// The user's changes afterwards.
    pub changes: Changes,
    /// Every command whose keys would differ from now, in the order of
    /// the defaults. Its length is "how many keys change"; empty when the
    /// import would change nothing.
    pub differences: Vec<KeyDifference>,
    /// The conflicts the import would bring: two commands on one key in
    /// the same place, involving a command the import changes. (Conflicts
    /// that are there already are not listed.)
    pub conflicts: Vec<Collision>,
}

/// What reading a shortcuts file found. Nothing is applied yet.
#[derive(Clone, Debug, PartialEq)]
pub struct ImportPlan {
    /// The version of the app that wrote the file, when it says.
    pub app_version: Option<String>,
    /// How many entries the file has.
    pub entries: usize,
    /// How many of them say that their command is not changed.
    pub unchanged: usize,
    /// The changes the file makes, in the order of the file.
    pub changes: Vec<FileChange>,
    /// The entries that are skipped, with the reason for each.
    pub skipped: Vec<Skipped>,
    replace: ImportOutcome,
    merge: ImportOutcome,
    before: Changes,
}

/// The result of carrying out a plan.
#[derive(Clone, Debug, PartialEq)]
pub struct Carried {
    /// The user's changes now: store these and install their keymap.
    pub changes: Changes,
    /// The user's changes as they were: Undo puts these back.
    pub undo: Changes,
}

impl ImportPlan {
    /// What carrying the plan out in `mode` would give.
    pub fn outcome(&self, mode: ImportMode) -> &ImportOutcome {
        match mode {
            ImportMode::Replace => &self.replace,
            ImportMode::Merge => &self.merge,
        }
    }

    /// Carries the plan out: the new changes, and the old ones for Undo.
    pub fn carry_out(&self, mode: ImportMode) -> Carried {
        Carried {
            changes: self.outcome(mode).changes.clone(),
            undo: self.before.clone(),
        }
    }

    /// Whether the user has changes of their own that the two modes treat
    /// differently. When not, there is nothing to choose between.
    pub fn modes_differ(&self) -> bool {
        self.replace.differences != self.merge.differences
    }
}

/// Reads `text` as a shortcuts file and plans its import over the keymap
/// as it is (`defaults` with the user's `changes`). See the [module
/// docs](self) for what counts as a change and what is skipped.
pub fn plan_import(
    text: &str,
    defaults: &Defaults,
    changes: &Changes,
    places: &Places,
) -> Result<ImportPlan, ImportError> {
    plan_import_among(text, defaults, changes, places, &Absent::new())
}

/// [`plan_import`] while the commands of `absent` are not there (see
/// [`crate::verdict`]). The file's changes to them are taken like the
/// others, and are among the differences: a file means the same on every
/// Mac. Only the conflicts are counted without them, by the one rule
/// [`crate::check_among`] judges a single key with: what the plan lists
/// as a conflict is what the question about that key would call taken.
pub fn plan_import_among(
    text: &str,
    defaults: &Defaults,
    changes: &Changes,
    places: &Places,
    absent: &Absent,
) -> Result<ImportPlan, ImportError> {
    let listed = Listed {
        absent: absent.clone(),
        rows: Vec::new(),
    };
    plan_import_listed(text, defaults, changes, places, &listed)
}

/// [`plan_import`] for the keymap as the caller lists it
/// ([`crate::Listed`]): the conflicts are counted without the commands
/// that are not there and between rows, as [`crate::check_listed`] judges
/// a single key.
pub fn plan_import_listed(
    text: &str,
    defaults: &Defaults,
    changes: &Changes,
    places: &Places,
    listed: &Listed,
) -> Result<ImportPlan, ImportError> {
    let value: Value = serde_json::from_str(text).map_err(|error| ImportError::NotJson {
        line: error.line(),
        column: error.column(),
        problem: json_problem(&error),
    })?;
    let object = value.as_object().ok_or(ImportError::NotAnObject)?;
    match object.get("format") {
        None => return Err(ImportError::NoFormat),
        Some(Value::String(format)) if format == FILE_FORMAT => {}
        Some(Value::String(format)) => {
            return Err(ImportError::OtherFormat {
                found: format.clone(),
            });
        }
        Some(other) => {
            return Err(ImportError::OtherFormat {
                found: other.to_string(),
            });
        }
    }
    let version = object
        .get("version")
        .and_then(Value::as_u64)
        .filter(|version| *version >= 1)
        .ok_or(ImportError::BadVersion)?;
    if version > u64::from(FILE_VERSION) {
        return Err(ImportError::NewerVersion {
            found: version,
            supported: FILE_VERSION,
        });
    }
    let shortcuts = object
        .get("shortcuts")
        .and_then(Value::as_array)
        .ok_or(ImportError::NoShortcuts)?;

    let mut plan_changes: Vec<FileChange> = Vec::new();
    let mut skipped = Vec::new();
    let mut unchanged = 0;
    let mut seen: Vec<Target> = Vec::new();
    // Targets the file says are at their defaults.
    for (ix, element) in shortcuts.iter().enumerate() {
        let text_of = |field: &str| {
            element
                .get(field)
                .and_then(Value::as_str)
                .map(str::to_string)
        };
        let mut skip = |reason: SkipReason| {
            skipped.push(Skipped {
                entry: ix + 1,
                command: text_of("command"),
                title: text_of("title"),
                place: text_of("where"),
                reason,
            })
        };
        match read_entry(element, defaults, places) {
            Err(reason) => skip(reason),
            Ok(change) if seen.contains(&change.target) => skip(SkipReason::Duplicate),
            Ok(change) => {
                seen.push(change.target.clone());
                if diff(&change.default, &change.keys).is_empty() {
                    unchanged += 1;
                } else {
                    plan_changes.push(change);
                }
            }
        }
    }

    let outcome = |mode: ImportMode| {
        let mut next = changes.clone();
        if mode == ImportMode::Replace {
            next.reset_all();
        }
        for change in &plan_changes {
            // The entry is made against the file's defaults; what it
            // means for the defaults of now is stored.
            let default_now = defaults.default_keys(&change.target);
            let entry = ChangeEntry {
                command: change.target.command.action.clone(),
                input: change.target.command.input.clone(),
                context: change.target.context.clone(),
                default: change.default.clone(),
                keys: change.keys.clone(),
                extra: Default::default(),
            };
            let row = resolve(&default_now, Some(&entry)).row;
            next.store(&change.target, default_now, row);
        }
        let before = apply(defaults, changes);
        let after = apply(defaults, &next);
        let mut differences = Vec::new();
        for (was, is) in before.targets().iter().zip(after.targets()) {
            let keys = |state: &crate::effective::TargetKeys| -> Vec<KeySequence> {
                state.keys.iter().map(|key| key.keys.clone()).collect()
            };
            let (was_keys, is_keys) = (keys(was), keys(is));
            let mut sorted = (was_keys.clone(), is_keys.clone());
            sorted.0.sort();
            sorted.1.sort();
            if sorted.0 != sorted.1 {
                differences.push(KeyDifference {
                    target: was.target.clone(),
                    before: was_keys,
                    after: is_keys,
                });
            }
        }
        let already = collisions_in(&before, places, listed);
        let conflicts = collisions_in(&after, places, listed)
            .into_iter()
            .filter(|collision| {
                // A conflict is reported with the key context that stands
                // for its row: the import touches it when it changes the
                // row in any of its key contexts.
                let touched = |target: &Target| {
                    differences.iter().any(|difference| {
                        difference.target == *target
                            || listed.rows.iter().any(|row| {
                                row.targets.contains(target)
                                    && row.targets.contains(&difference.target)
                            })
                    })
                };
                (touched(&collision.target) || touched(&collision.holder.target))
                    && !already.iter().any(|old| {
                        (old.target == collision.target
                            && old.key == collision.key
                            && old.holder.target == collision.holder.target)
                            || (old.target == collision.holder.target
                                && old.key == collision.holder.keys
                                && old.holder.target == collision.target)
                    })
            })
            .collect();
        ImportOutcome {
            changes: next,
            differences,
            conflicts,
        }
    };
    let replace = outcome(ImportMode::Replace);
    let merge = outcome(ImportMode::Merge);

    Ok(ImportPlan {
        app_version: object
            .get("app_version")
            .and_then(Value::as_str)
            .map(str::to_string),
        entries: shortcuts.len(),
        unchanged,
        changes: plan_changes,
        skipped,
        replace,
        merge,
        before: changes.clone(),
    })
}

/// Reads one element of `shortcuts`: the command, its place, the default
/// written in the entry (the defaults of now when there is none) and its
/// keys.
fn read_entry(
    element: &Value,
    defaults: &Defaults,
    places: &Places,
) -> Result<FileChange, SkipReason> {
    let object = element.as_object().ok_or(SkipReason::NotAnEntry)?;
    let action = object
        .get("command")
        .and_then(Value::as_str)
        .filter(|command| !command.trim().is_empty())
        .ok_or(SkipReason::NoCommand)?;
    let input = match object.get("input") {
        None | Some(Value::Null) => None,
        Some(Value::String(input)) => Some(input.as_str()),
        Some(_) => return Err(SkipReason::BadField { field: "input" }),
    };
    let context = match object.get("context") {
        None | Some(Value::Null) => Context::anywhere(),
        Some(Value::String(context)) => Context::parse(context),
        Some(_) => return Err(SkipReason::BadField { field: "context" }),
    };
    let command = Command::with_input(action.trim(), input);
    let target = Target::of(command, context);
    if !defaults.knows(&target) {
        return Err(if defaults.knows_command(&target.command) {
            SkipReason::UnknownPlace
        } else {
            SkipReason::UnknownCommand
        });
    }

    let read_keys = |field: &'static str| -> Result<Option<Vec<KeySequence>>, SkipReason> {
        let list = match object.get(field) {
            None => return Ok(None),
            Some(Value::Array(list)) => list,
            Some(_) => return Err(SkipReason::BadField { field }),
        };
        let mut keys = Vec::with_capacity(list.len());
        for item in list {
            let text = item.as_str().ok_or(SkipReason::BadField { field })?;
            // As a person wrote it: a name no key has is not a key.
            let key = KeySequence::parse_written(text)
                .map_err(|error| SkipReason::UnreadableKey { field, error })?;
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
        Ok(Some(keys))
    };
    let keys = read_keys("keys")?.ok_or(SkipReason::BadField { field: "keys" })?;
    let default = match read_keys("default")? {
        Some(default) => default,
        None => defaults.default_keys(&target),
    };

    // The guard rails, for the keys the entry gives the command (not
    // for a key it has by default, then or now).
    let default_now = defaults.default_keys(&target);
    for key in &keys {
        if default.contains(key) || default_now.contains(key) {
            continue;
        }
        if let (Some(refusal), _) = guard(places, &target.context, key) {
            return Err(SkipReason::Refused {
                key: key.clone(),
                refusal,
            });
        }
    }
    Ok(FileChange {
        target,
        default,
        keys,
    })
}
