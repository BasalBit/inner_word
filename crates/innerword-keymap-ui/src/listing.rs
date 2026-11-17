//! What the host hands the view: the keymap as it is, and the commands to
//! list with their names.
//!
//! The view does not hold the keymap. The host (the app) owns it, installs
//! it and stores it; each time it changes the host gives the view a new
//! [`Listing`] with a copy ([`crate::KeymapView::set_listing`]). From the
//! copy the view reads the keys of every row, their defaults and whether
//! the user changed them, and asks it what a proposed key would collide
//! with ([`innerword_keymap::Keymap::check`]).

use gpui::SharedString;
use innerword_keymap::{Catalogue, KeySequence, Keymap, Target, TargetKey};

/// One command in one place: a row of the list.
#[derive(Clone, Debug, PartialEq)]
pub struct CommandRow {
    /// The command and the key context its keys apply in. What a change is
    /// made to.
    pub target: Target,
    /// What the command does ("Bold").
    pub title: SharedString,
    /// The category of the command ("Format"); empty for a key without one.
    pub category: SharedString,
    /// Where the keyboard has to be, in words ("In the text").
    pub place: SharedString,
    /// The heading the row is listed under: the place, and the category
    /// where it adds something ("In the text · Format").
    pub heading: SharedString,
    /// Set for a row whose key cannot be changed here, with the reason.
    pub locked: Option<Locked>,
    /// Whether "Run Command" is offered for the row.
    pub runnable: bool,
}

/// Why a row cannot be changed, and the key it shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Locked {
    /// The reason, as a sentence ("It is typed in the text, not a
    /// shortcut.").
    pub reason: SharedString,
    /// The keys the row shows when the keymap does not hold its target
    /// (the `/` that opens the insert menu is not a binding). A row whose
    /// target the keymap knows shows the keymap's keys.
    pub keys: Vec<KeySequence>,
}

impl CommandRow {
    /// A row for `target`, called `title`, that works in `place`. Its
    /// heading is the place; it can be changed and run.
    pub fn new(
        target: Target,
        title: impl Into<SharedString>,
        place: impl Into<SharedString>,
    ) -> Self {
        let place = place.into();
        Self {
            target,
            title: title.into(),
            category: SharedString::default(),
            heading: place.clone(),
            place,
            locked: None,
            runnable: true,
        }
    }

    /// Sets the category, and the heading to "place · category" (the
    /// place alone for an empty category).
    pub fn category(mut self, category: impl Into<SharedString>) -> Self {
        self.category = category.into();
        self.heading = if self.category.is_empty() {
            self.place.clone()
        } else {
            format!("{} \u{b7} {}", self.place, self.category).into()
        };
        self
    }

    /// Sets the heading the row is listed under, where it is not "place ·
    /// category" (a category that says nothing beside its place).
    pub fn heading(mut self, heading: impl Into<SharedString>) -> Self {
        self.heading = heading.into();
        self
    }

    /// Says that the row's keys cannot be changed here, and why.
    pub fn locked(mut self, reason: impl Into<SharedString>) -> Self {
        let keys = self
            .locked
            .take()
            .map(|locked| locked.keys)
            .unwrap_or_default();
        self.locked = Some(Locked {
            reason: reason.into(),
            keys,
        });
        self
    }

    /// Like [`CommandRow::locked`], for a row the keymap does not hold:
    /// `keys` is what it shows.
    pub fn locked_with_keys(
        mut self,
        reason: impl Into<SharedString>,
        keys: Vec<KeySequence>,
    ) -> Self {
        self.locked = Some(Locked {
            reason: reason.into(),
            keys,
        });
        self
    }

    /// Leaves "Run Command" out for the row (a key that only means
    /// something where it is pressed, such as moving the caret).
    pub fn not_runnable(mut self) -> Self {
        self.runnable = false;
        self
    }
}

/// Everything the view shows: the keymap as it is, and the commands to list
/// with their names.
///
/// The view does not hold the keymap. The host owns it, installs it and
/// stores it; each time it changes the host hands the view a new listing
/// with a copy ([`crate::KeymapView::set_listing`]). From the copy the view
/// reads the keys of every row, their defaults and whether the user changed
/// them, and asks it what a proposed key would collide with
/// ([`innerword_keymap::Keymap::check`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Listing {
    /// The keymap as it is now: defaults, places and the user's changes.
    pub keymap: Keymap,
    /// The rows, in the order they are listed: rows with the same heading
    /// side by side.
    pub rows: Vec<CommandRow>,
    /// Names for commands that are not rows, for sentences that name one
    /// (the command a key is taken by, an entry of an imported file). The
    /// rows are looked up first.
    pub catalogue: Catalogue,
    /// Lines shown above the list while nothing is searched for (that
    /// every command is also in a menu; what cannot be changed here).
    pub notes: Vec<SharedString>,
    /// What sees keys before the keymap does right now (Vim mode in the
    /// text), so that the question about a key can say so.
    pub takers: Vec<TakesKeysFirst>,
}

/// Something of the host's that sees the keys of a place before the keymap
/// does, and takes those it has a use for: Vim mode in the text. The
/// keymap as data knows nothing of it, so a key that is free there can
/// still be one that never runs. The view cannot tell which keys those
/// are; it says, for a key that could be one, that this can happen.
#[derive(Clone, Debug)]
pub struct TakesKeysFirst {
    /// What it is called where the heading of the question names it ("Vim
    /// mode": "⌃Y: Vim mode sees it first"). Without a name the heading
    /// says that the key may not run.
    pub name: Option<SharedString>,
    /// The key contexts it sits in, as the keymap writes them (`Editor`).
    /// A command that works anywhere is concerned too.
    pub contexts: Vec<SharedString>,
    /// Whether it certainly leaves `key` to the keymap (Vim leaves every
    /// key with ⌘ in it): nothing is said about such a key.
    pub leaves: fn(&KeySequence) -> bool,
    /// What the question about any other key says under its heading, as a
    /// sentence: the consequence.
    pub warning: SharedString,
}

impl PartialEq for TakesKeysFirst {
    /// By what is said and where: two functions cannot be compared.
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && self.contexts == other.contexts && self.warning == other.warning
    }
}

impl Listing {
    /// A listing of `rows` over `keymap`, without notes.
    pub fn new(keymap: Keymap, rows: Vec<CommandRow>) -> Self {
        Self {
            keymap,
            rows,
            catalogue: Catalogue::new(),
            notes: Vec::new(),
            takers: Vec::new(),
        }
    }

    /// Says that something sees the keys of `contexts` before the keymap
    /// does (see [`TakesKeysFirst`]), except those `leaves` says it
    /// leaves alone; `warning` is what the question about any other key
    /// then says, for a command in one of those contexts or one that
    /// works anywhere.
    ///
    /// The heading of that question cannot say who it is ("⌃Y may not
    /// run"): prefer [`Listing::sees_keys_first`], which names it.
    pub fn takes_keys_first<C: Into<SharedString>>(
        mut self,
        contexts: impl IntoIterator<Item = C>,
        leaves: fn(&KeySequence) -> bool,
        warning: impl Into<SharedString>,
    ) -> Self {
        self.takers.push(TakesKeysFirst {
            name: None,
            contexts: contexts.into_iter().map(Into::into).collect(),
            leaves,
            warning: warning.into(),
        });
        self
    }

    /// [`Listing::takes_keys_first`] with what it is called, as the
    /// heading of the question names it: `name` "Vim mode" gives "⌃Y: Vim
    /// mode sees it first", with `warning` as the sentence under it.
    pub fn sees_keys_first<C: Into<SharedString>>(
        mut self,
        name: impl Into<SharedString>,
        contexts: impl IntoIterator<Item = C>,
        leaves: fn(&KeySequence) -> bool,
        warning: impl Into<SharedString>,
    ) -> Self {
        self.takers.push(TakesKeysFirst {
            name: Some(name.into()),
            contexts: contexts.into_iter().map(Into::into).collect(),
            leaves,
            warning: warning.into(),
        });
        self
    }

    /// What may take `key` before the keymap sees it, for a command at
    /// `target` ([`Listing::takes_keys_first`]): in one of its key
    /// contexts, or anywhere.
    pub fn seeing_first(&self, target: &Target, key: &KeySequence) -> Vec<&TakesKeysFirst> {
        let context = target.context.text();
        self.takers
            .iter()
            .filter(|taker| !(taker.leaves)(key))
            .filter(|taker| match context {
                None => true,
                Some(context) => taker.contexts.iter().any(|name| name.as_ref() == context),
            })
            .collect()
    }

    /// What to say about `key` for `target` because something may take
    /// the key before the keymap sees it ([`Listing::takes_keys_first`]).
    pub fn taken_first_warnings(&self, target: &Target, key: &KeySequence) -> Vec<SharedString> {
        self.seeing_first(target, key)
            .into_iter()
            .map(|taker| taker.warning.clone())
            .collect()
    }

    /// Names for the commands that are not rows.
    pub fn catalogue(mut self, catalogue: Catalogue) -> Self {
        self.catalogue = catalogue;
        self
    }

    /// Adds a line above the list.
    pub fn note(mut self, note: impl Into<SharedString>) -> Self {
        self.notes.push(note.into());
        self
    }

    /// How many commands have other keys than their defaults, as a person
    /// counts them ([`Listing::commands_among`]).
    pub fn changed_commands(&self) -> usize {
        let effective = self.keymap.effective();
        self.commands_among(effective.changed().map(|changed| &changed.target))
    }

    /// How many commands `targets` are, as a person counts them: a command
    /// the keymap holds in several key contexts of one place (the chats of
    /// the assistant bind theirs in four) is one command with one key, as
    /// it is one row of the list. Commands are told apart by their name
    /// and place, of their row or in the catalogue, so a command that is
    /// not listed right now is counted too.
    pub fn commands_among<'a>(&self, targets: impl IntoIterator<Item = &'a Target>) -> usize {
        let mut seen: Vec<(&str, &str)> = Vec::new();
        let mut unnamed = 0;
        for target in targets {
            let named = self
                .rows
                .iter()
                .find(|row| row.target == *target)
                .map(|row| (row.title.as_ref(), row.place.as_ref()))
                .or_else(|| {
                    self.catalogue
                        .info(target)
                        .map(|info| (info.title.as_str(), info.place.as_str()))
                });
            match named {
                Some(name) if !seen.contains(&name) => seen.push(name),
                Some(_) => {}
                None => unnamed += 1,
            }
        }
        seen.len() + unnamed
    }
}

/// A row with what the keymap says about it.
#[derive(Clone, Debug)]
pub(crate) struct Entry {
    pub row: CommandRow,
    /// The keys the row shows, in order.
    pub keys: Vec<TargetKey>,
    pub default_keys: Vec<KeySequence>,
    pub changed: bool,
    /// Whether the keymap holds the row's target.
    pub known: bool,
    /// The key that key hints and the menu bar show for the command.
    pub shown: Option<KeySequence>,
    /// Set when another command has one of the row's keys in the same
    /// place, so that one of the two cannot run with it: the sentence
    /// that says so.
    pub clash: Option<SharedString>,
}

impl Entry {
    /// Whether the row's keys can be changed.
    pub fn changeable(&self) -> bool {
        self.row.locked.is_none() && self.known
    }

    /// The key the keys of the view act on while nothing else was chosen:
    /// the one the rest of the app shows for the command (the last one
    /// bound), which is the one a person knows.
    pub fn main_key(&self) -> usize {
        self.shown
            .as_ref()
            .and_then(|shown| self.keys.iter().position(|key| key.keys == *shown))
            .unwrap_or(0)
    }

    /// Why the row cannot be changed, if it cannot.
    pub fn locked_reason(&self) -> Option<SharedString> {
        match &self.row.locked {
            Some(locked) => Some(locked.reason.clone()),
            None if !self.known => Some("This version cannot give this command a key.".into()),
            None => None,
        }
    }
}

/// The rows of `listing` with their keys, and with what the keymap says
/// is wrong with them: two commands on one key in the same place (which
/// the user can bring about by keeping both, and an import can leave).
pub(crate) fn entries(listing: &Listing) -> Vec<Entry> {
    let mut entries = entries_without_clashes(listing);
    let collisions = listing.keymap.collisions();
    if collisions.is_empty() {
        return entries;
    }
    let mut clashes: Vec<(usize, SharedString)> = Vec::new();
    {
        let names = crate::words::Names {
            entries: &entries,
            catalogue: &listing.catalogue,
        };
        for collision in &collisions {
            let sides = [
                (&collision.target, &collision.key, &collision.holder.target),
                (
                    &collision.holder.target,
                    &collision.holder.keys,
                    &collision.target,
                ),
            ];
            for (target, key, other) in sides {
                if let Some(index) = entries.iter().position(|entry| entry.row.target == *target)
                    && !clashes.iter().any(|(have, _)| *have == index)
                {
                    clashes.push((index, crate::words::clash_text(key, other, &names).into()));
                }
            }
        }
    }
    for (index, text) in clashes {
        entries[index].clash = Some(text);
    }
    entries
}

/// What to say above the list about stored changes that do nothing in
/// this version (they are kept, and listed nowhere else).
pub(crate) fn inactive_note(listing: &Listing) -> Option<SharedString> {
    let inactive = listing.keymap.effective().inactive().len();
    (inactive > 0).then(|| {
        format!(
            "{} made in another version of Inner Word {} kept; {} nothing in this one",
            crate::words::count(inactive, "change", "changes"),
            if inactive == 1 { "is" } else { "are" },
            if inactive == 1 { "it does" } else { "they do" }
        )
        .into()
    })
}

fn entries_without_clashes(listing: &Listing) -> Vec<Entry> {
    let effective = listing.keymap.effective();
    listing
        .rows
        .iter()
        .map(|row| match effective.target(&row.target) {
            Some(state) => Entry {
                row: row.clone(),
                keys: state.keys.clone(),
                default_keys: state.default_keys.clone(),
                changed: state.changed,
                known: true,
                shown: state.shown.clone(),
                clash: None,
            },
            None => {
                let keys: Vec<KeySequence> = row
                    .locked
                    .as_ref()
                    .map(|locked| locked.keys.clone())
                    .unwrap_or_default();
                Entry {
                    row: row.clone(),
                    keys: keys
                        .iter()
                        .map(|keys| TargetKey {
                            keys: keys.clone(),
                            origin: innerword_keymap::Origin::Default,
                        })
                        .collect(),
                    default_keys: keys,
                    changed: false,
                    known: false,
                    shown: None,
                    clash: None,
                }
            }
        })
        .collect()
}
