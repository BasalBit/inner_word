//! Guard rails and conflicts: what would happen if a command got a key.
//!
//! [`check`] answers before anything is changed, with a [`Verdict`] the
//! interface words itself:
//!
//! - **refused**, with the reason: the key would stop text from being
//!   typed where text is typed (a bare character, or shift with one);
//!   macOS keeps the key and no setting frees it (`cmd-tab`); the command
//!   has that key already;
//! - **taken**: another command has the key in the same place, so one of
//!   the two would never run. The user takes it from that command
//!   ([`Holder::take`]) or cancels;
//! - **also used elsewhere**: the same key in another place, which is
//!   allowed (the find bar and the window both have `cmd-alt-c`). Each
//!   such holder says which of the two keeps the key where both apply;
//! - **fine**.
//!
//! Warnings come with any of them: option with a character types a
//! special character, the key is one a text is edited with, macOS uses
//! the key unless that was switched off (`cmd-space` for Spotlight, the
//! screenshot keys, the keys of Mission Control).
//!
//! Which places are the same, and who gets a key where two bindings both
//! apply, is decided from the text of the key contexts and the caller's
//! [`Places`]; what that assumes is written in [`crate::context`]. The
//! order of the keymap that would result is taken into account: the
//! verdict is about the keymap [`crate::apply`] would build.
//!
//! # Commands that are not there
//!
//! A part of the app can be absent on a Mac (the assistant without its
//! engine): its commands are listed nowhere and its keys do nothing. The
//! caller names their targets ([`Absent`]), and they are then nobody's
//! holders: a key of theirs is free for every other command, no row is
//! warned about them, an import counts no collision with them
//! ([`check_among`], [`collisions_among`], [`crate::plan_import_among`]).
//! Nothing about them is changed or forgotten: their keys stay bound and
//! the user's changes to them stay stored, so the day the part is there
//! again the same functions, asked without them in [`Absent`], say what
//! collides.
//!
//! # Rows: what is one key, and one place, to the user
//!
//! The caller's list has rows, and tells which ([`Row`], in [`Listed`]):
//! the key contexts a command is bound in that are one command with one
//! key to the user (the find bar binds "case sensitive" for the bar and
//! for its field), and the name of the place the row is listed under ("In
//! the library"). The verdict is then about rows
//! ([`check_listed`], [`collisions_listed`]):
//!
//! - **A command the key collides with is named once**, however many
//!   key contexts its row has: by the most serious of what was found
//!   among them. (The key itself is judged where the caller asks: in the
//!   key context that stands for its row, the one the keyboard itself is
//!   in. A second key context of the row that only ever has the keyboard
//!   in something inside it, such as the find bar around its field,
//!   would say that a key of the window beats it, which is true of the
//!   bar and not of the row.)
//! - **Two rows under one place name that can apply together are in the
//!   same place.** Their key contexts may only overlap (the library binds
//!   some keys wherever the keyboard is in it and others only while
//!   nothing is typed there), so that by the contexts one of the two
//!   would keep the key "in the rest of its place". The user is told one
//!   place for both, where one of them has lost the key: that is
//!   **taken**, never "another place". (Two rows under one name whose
//!   contexts never apply together would both work; the caller gives
//!   places that are apart names of their own.)
//!
//! Without rows every key context is a row of its own and its place has
//! no name: the verdict is then about key contexts alone.

use std::collections::BTreeSet;

use crate::changes::Changes;
use crate::context::{Context, Extent, Places, Rank};
use crate::defaults::{Defaults, Target};
use crate::effective::{Effective, Origin, apply};
use crate::keystroke::{KeySequence, SystemUse};

/// The commands that are not there right now, by their targets: see the
/// [module docs](self). Empty when everything is there.
pub type Absent = BTreeSet<Target>;

/// One row of the caller's list: a command in every key context that is
/// one command with one key to the user, and the name of the place the
/// row is listed under. See the [module docs](self).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// The command in each key context of the row. The first stands for
    /// the row: a conflict of the row is reported with it.
    pub targets: Vec<Target>,
    /// The name of the place, as the list writes it ("In the text").
    pub place: String,
}

impl Row {
    /// A row of `targets` listed under `place`.
    pub fn new(targets: impl IntoIterator<Item = Target>, place: &str) -> Self {
        Self {
            targets: targets.into_iter().collect(),
            place: place.to_string(),
        }
    }
}

/// How the keymap is listed right now: which commands are not there, and
/// which key contexts are one row under which place name. Empty when the
/// caller says nothing: every key context is then a row of its own.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Listed {
    /// The commands that are not there: see [`Absent`].
    pub absent: Absent,
    /// The rows: see [`Row`].
    pub rows: Vec<Row>,
}

impl Listed {
    /// The row `target` is in, with its index.
    fn row_of(&self, target: &Target) -> Option<(usize, &Row)> {
        self.rows
            .iter()
            .enumerate()
            .find(|(_, row)| row.targets.contains(target))
    }

    /// The target that stands for the row of `target`: the row's first,
    /// or `target` itself when it is in no row.
    fn standing_for(&self, target: &Target) -> Target {
        self.row_of(target)
            .and_then(|(_, row)| row.targets.first())
            .unwrap_or(target)
            .clone()
    }

    /// Whether the two are rows under one place name.
    fn in_one_named_place(&self, a: &Target, b: &Target) -> bool {
        match (self.row_of(a), self.row_of(b)) {
            (Some((_, a)), Some((_, b))) => a.place == b.place,
            _ => false,
        }
    }
}

/// Why a key cannot be given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The key is a character, alone or with shift, and text is typed
    /// where the command applies: the character could no longer be typed.
    TypesText,
    /// macOS keeps the key for itself: it never reaches an app.
    SystemKeeps(SystemUse),
    /// The command has this key in this place already.
    AlreadyHasKey,
    /// This version has no such command in that context.
    UnknownCommand,
}

/// Something to say about a key that can be given all the same.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Warning {
    /// Option with a character, where text is typed: on a Mac keyboard
    /// that types a special character or starts an accent, which could no
    /// longer be typed.
    SpecialCharacter,
    /// A key a text is edited with (`enter`, `backspace`, `tab`, an
    /// arrow, ...), alone or with shift, where text is typed.
    EditingKey,
    /// macOS uses the key unless the user switched that off in System
    /// Settings: the shortcut may not run.
    SystemUses(SystemUse),
}

/// How a holder of the key and the proposed key get along.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Overlap {
    /// The same place. Taken.
    SamePlace,
    /// Wherever the holder applies, the proposed key would win: the
    /// holder's command would lose its key. Taken.
    HolderNeverRuns,
    /// Wherever the proposed key applies, the holder wins: the proposed
    /// key would do nothing. Taken.
    ProposedNeverRuns,
    /// The places overlap; where both apply the proposed key wins, and
    /// the holder keeps the key in the rest of its place. Allowed.
    ProposedWinsThere,
    /// The places overlap; where both apply the holder wins, and the
    /// proposed key works in the rest of its place. Allowed.
    HolderWinsThere,
    /// Different places: both work. Allowed.
    Apart,
}

impl Overlap {
    /// Whether this is a conflict the user has to settle.
    pub fn is_taken(self) -> bool {
        matches!(
            self,
            Overlap::SamePlace | Overlap::HolderNeverRuns | Overlap::ProposedNeverRuns
        )
    }

    /// How much there is to say about it, for choosing what to say about
    /// a row that was found in several of its key contexts: a conflict
    /// before an overlap, an overlap before a place of its own.
    fn weight(self) -> u8 {
        match self {
            Overlap::SamePlace => 5,
            Overlap::HolderNeverRuns => 4,
            Overlap::ProposedNeverRuns => 3,
            Overlap::ProposedWinsThere => 2,
            Overlap::HolderWinsThere => 1,
            Overlap::Apart => 0,
        }
    }
}

/// A command that has the proposed key (or a key that begins it, or one
/// it begins: `g` and `g g` get in each other's way).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Holder {
    /// The command and where it has the key.
    pub target: Target,
    /// Its key. The proposed key, unless one is the beginning of the
    /// other.
    pub keys: KeySequence,
    /// Whether that key is a default or the user's own.
    pub origin: Origin,
    /// How the two get along.
    pub overlap: Overlap,
}

impl Holder {
    /// "Take it from that command": removes the key from the holder in
    /// `changes`. Returns whether anything changed.
    pub fn take(&self, defaults: &Defaults, changes: &mut Changes) -> bool {
        changes.remove_key(defaults, &self.target, &self.keys)
    }
}

/// The four outcomes, most serious first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum VerdictKind {
    /// Nothing in the way.
    Fine,
    /// The key is used in other places too; allowed.
    UsedElsewhere,
    /// Another command has the key in the same place.
    Taken,
    /// The key cannot be given.
    Refused,
}

/// What would happen if the command got the key. See the [module
/// docs](self).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verdict {
    /// The key that was proposed.
    pub key: KeySequence,
    /// Why it cannot be given, if it cannot.
    pub refusal: Option<Refusal>,
    /// What to say about it besides.
    pub warnings: Vec<Warning>,
    /// The commands that have the key in the same place.
    pub taken: Vec<Holder>,
    /// The commands that have the key in another place.
    pub elsewhere: Vec<Holder>,
}

impl Verdict {
    /// The outcome in one word.
    pub fn kind(&self) -> VerdictKind {
        if self.refusal.is_some() {
            VerdictKind::Refused
        } else if !self.taken.is_empty() {
            VerdictKind::Taken
        } else if !self.elsewhere.is_empty() {
            VerdictKind::UsedElsewhere
        } else {
            VerdictKind::Fine
        }
    }

    /// Whether the key can be given as it is, without asking: not
    /// refused and not taken.
    pub fn is_allowed(&self) -> bool {
        self.kind() <= VerdictKind::UsedElsewhere
    }
}

/// The guard rails alone, for a key in a context: the refusal, if any,
/// and the warnings. (Not whether the key is taken; [`check`] does both.)
pub fn guard(
    places: &Places,
    context: &Context,
    key: &KeySequence,
) -> (Option<Refusal>, Vec<Warning>) {
    let typing = places.is_typing(context);
    let mut refusal = None;
    let mut warnings = Vec::new();
    // What is pressed first decides whether a character can still be
    // typed: a second keystroke only counts once the first was pressed.
    let first = key.first();
    if typing && first.types_text() {
        refusal = Some(Refusal::TypesText);
    }
    for stroke in key.strokes() {
        match stroke.system_use() {
            Some(system) if system.kept => {
                refusal.get_or_insert(Refusal::SystemKeeps(system));
            }
            Some(system) => {
                let warning = Warning::SystemUses(system);
                if !warnings.contains(&warning) {
                    warnings.push(warning);
                }
            }
            None => {}
        }
    }
    if typing && first.types_special_character() {
        warnings.push(Warning::SpecialCharacter);
    }
    if typing && first.is_editing_key() {
        warnings.push(Warning::EditingKey);
    }
    (refusal, warnings)
}

/// What would happen if `target` got the key `key`: in the place of its
/// key `replacing`, or as one more key (`None`). Nothing is changed.
///
/// The guard rails do not apply to a key the command has by default: a
/// default that was taken away can always be given back.
///
/// The holders are those in the keymap that would result, so a key the
/// user already took from a command is not held by it, and the key being
/// replaced is not in the way.
pub fn check(
    defaults: &Defaults,
    changes: &Changes,
    places: &Places,
    target: &Target,
    key: &KeySequence,
    replacing: Option<&KeySequence>,
) -> Verdict {
    check_among(
        defaults,
        changes,
        places,
        &Absent::new(),
        target,
        key,
        replacing,
    )
}

/// [`check`] while the commands of `absent` are not there: they hold no
/// key, so a key of theirs is free, taken by nobody and used nowhere
/// else. See the [module docs](self).
pub fn check_among(
    defaults: &Defaults,
    changes: &Changes,
    places: &Places,
    absent: &Absent,
    target: &Target,
    key: &KeySequence,
    replacing: Option<&KeySequence>,
) -> Verdict {
    let listed = Listed {
        absent: absent.clone(),
        rows: Vec::new(),
    };
    check_listed(defaults, changes, places, &listed, target, key, replacing)
}

/// [`check`] for the keymap as the caller lists it ([`Listed`]): without
/// the commands that are not there, and about rows. What the key collides
/// with is said once for each row, and a row under the place name of
/// `target`'s own that can apply together with it is in the same place.
/// See the [module docs](self).
///
/// `target` is the key context that stands for its row: the caller asks
/// with the one where a key is judged hardest.
pub fn check_listed(
    defaults: &Defaults,
    changes: &Changes,
    places: &Places,
    listed: &Listed,
    target: &Target,
    key: &KeySequence,
    replacing: Option<&KeySequence>,
) -> Verdict {
    let mut verdict = Verdict {
        key: key.clone(),
        refusal: None,
        warnings: Vec::new(),
        taken: Vec::new(),
        elsewhere: Vec::new(),
    };
    if !defaults.knows(target) {
        verdict.refusal = Some(Refusal::UnknownCommand);
        return verdict;
    }
    // A key the command has by default is never refused: giving it back
    // after it was taken away is always possible.
    if !defaults.default_keys(target).contains(key) {
        let (refusal, warnings) = guard(places, &target.context, key);
        verdict.refusal = refusal;
        verdict.warnings = warnings;
    }

    let has = changes.keys_of(defaults, target);
    if has.contains(key) {
        // Also when it is the key being replaced: nothing would change.
        verdict.refusal.get_or_insert(Refusal::AlreadyHasKey);
        return verdict;
    }

    let mut proposed = changes.clone();
    match replacing {
        Some(old) if has.contains(old) => {
            proposed.replace_key(defaults, target, old, key.clone());
        }
        _ => {
            proposed.add_key(defaults, target, key.clone());
        }
    }
    let effective = apply(defaults, &proposed);
    let position = effective
        .bindings()
        .iter()
        .rposition(|binding| binding.target == *target && binding.keys == *key);
    if let Some(position) = position {
        let found = holders(&effective, places, listed, position);
        for holder in once_for_each_row(found, listed) {
            if holder.overlap.is_taken() {
                verdict.taken.push(holder);
            } else {
                verdict.elsewhere.push(holder);
            }
        }
    }
    verdict
}

/// `holders` with each row named once: of the holders that are one row
/// (and have the same key), the one there is most to say about. A holder
/// in no row is a row of its own.
fn once_for_each_row(holders: Vec<Holder>, listed: &Listed) -> Vec<Holder> {
    let mut kept: Vec<(Target, Holder)> = Vec::new();
    for holder in holders {
        let stands = listed.standing_for(&holder.target);
        match kept
            .iter_mut()
            .find(|(have, kept)| *have == stands && kept.keys == holder.keys)
        {
            Some((_, kept)) => {
                if holder.overlap.weight() > kept.overlap.weight() {
                    *kept = holder;
                }
            }
            None => kept.push((stands, holder)),
        }
    }
    kept.into_iter().map(|(_, holder)| holder).collect()
}

/// Every other command that has the key of the binding at `position` (or
/// a key that collides with it), with how the two get along. The binding
/// at `position` is "the proposed key". Bindings of the same command in
/// other contexts are not holders: the same thing runs either way. Nor are
/// the bindings of a command that is not there ([`Listed::absent`]).
///
/// Two rows under one place name whose contexts overlap are in the same
/// place to the user: the one that loses where both apply has lost its
/// key (see the [module docs](self)).
pub(crate) fn holders(
    effective: &Effective,
    places: &Places,
    listed: &Listed,
    position: usize,
) -> Vec<Holder> {
    let bindings = effective.bindings();
    let proposed = &bindings[position];
    // The last binding of each (target, key): a key in two spellings is
    // two bindings side by side.
    let mut found: Vec<(usize, Holder)> = Vec::new();
    for (index, binding) in bindings.iter().enumerate() {
        if binding.target.command == proposed.target.command
            || !binding.keys.collides_with(&proposed.keys)
            || listed.absent.contains(&binding.target)
        {
            continue;
        }
        let relation = places.relation(&proposed.target.context, &binding.target.context);
        let proposed_wins = match relation.rank {
            Rank::FirstWins => true,
            Rank::SecondWins => false,
            Rank::LaterWins => position > index,
        };
        let overlap = match relation.extent {
            Extent::Same => Overlap::SamePlace,
            Extent::Apart => Overlap::Apart,
            Extent::FirstInsideSecond if proposed_wins => Overlap::ProposedWinsThere,
            Extent::FirstInsideSecond => Overlap::ProposedNeverRuns,
            Extent::SecondInsideFirst if proposed_wins => Overlap::HolderNeverRuns,
            Extent::SecondInsideFirst => Overlap::HolderWinsThere,
            Extent::Overlapping if proposed_wins => Overlap::ProposedWinsThere,
            Extent::Overlapping => Overlap::HolderWinsThere,
        };
        // One place by its name: who loses where both apply has lost the
        // key there.
        let overlap = match overlap {
            Overlap::ProposedWinsThere | Overlap::HolderWinsThere
                if listed.in_one_named_place(&proposed.target, &binding.target) =>
            {
                if proposed_wins {
                    Overlap::HolderNeverRuns
                } else {
                    Overlap::ProposedNeverRuns
                }
            }
            other => other,
        };
        let holder = Holder {
            target: binding.target.clone(),
            keys: binding.keys.clone(),
            origin: binding.origin.clone(),
            overlap,
        };
        match found
            .iter_mut()
            .find(|(_, have)| have.target == holder.target && have.keys == holder.keys)
        {
            Some(slot) => *slot = (index, holder),
            None => found.push((index, holder)),
        }
    }
    found.into_iter().map(|(_, holder)| holder).collect()
}

/// Two commands on one key in the same place, of which at least one key
/// is the user's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Collision {
    /// The command whose key is the user's.
    pub target: Target,
    /// Its key.
    pub key: KeySequence,
    /// The other command, with how the two get along (always a
    /// [taken](Overlap::is_taken) kind).
    pub holder: Holder,
}

/// Every conflict in the keymap as it is with `changes`: pairs of
/// commands on one key in the same place, where at least one of the keys
/// is the user's (the defaults have keys that several commands share by
/// design; those are not reported). Each pair once. For marking rows, and
/// for what an import leaves behind.
pub fn collisions(defaults: &Defaults, changes: &Changes, places: &Places) -> Vec<Collision> {
    collisions_among(defaults, changes, places, &Absent::new())
}

/// [`collisions`] while the commands of `absent` are not there: a pair
/// with one of them is not a conflict now (the absent command cannot be
/// run, and its key does nothing). See the [module docs](self).
pub fn collisions_among(
    defaults: &Defaults,
    changes: &Changes,
    places: &Places,
    absent: &Absent,
) -> Vec<Collision> {
    let listed = Listed {
        absent: absent.clone(),
        rows: Vec::new(),
    };
    collisions_listed(defaults, changes, places, &listed)
}

/// [`collisions`] for the keymap as the caller lists it ([`Listed`]):
/// without the commands that are not there, and about rows. A conflict
/// between two rows is reported once, with the key context that stands
/// for each ([`Row::targets`], the first), by the rule [`check_listed`]
/// judges a key with.
pub fn collisions_listed(
    defaults: &Defaults,
    changes: &Changes,
    places: &Places,
    listed: &Listed,
) -> Vec<Collision> {
    collisions_in(&apply(defaults, changes), places, listed)
}

pub(crate) fn collisions_in(
    effective: &Effective,
    places: &Places,
    listed: &Listed,
) -> Vec<Collision> {
    let bindings = effective.bindings();
    let mut found: Vec<Collision> = Vec::new();
    for (position, binding) in bindings.iter().enumerate() {
        if !binding.origin.is_users() || listed.absent.contains(&binding.target) {
            continue;
        }
        // The last binding of a key in two spellings speaks for both.
        let is_last = !bindings[position + 1..]
            .iter()
            .any(|later| later.target == binding.target && later.keys == binding.keys);
        if !is_last {
            continue;
        }
        // A row's key is judged in the key context that stands for the
        // row, as the question about it was: a second key context of the
        // row with the same key is not asked by itself.
        let target = listed.standing_for(&binding.target);
        if target != binding.target
            && bindings
                .iter()
                .any(|other| other.target == target && other.keys == binding.keys)
        {
            continue;
        }
        for mut holder in holders(effective, places, listed, position) {
            if !holder.overlap.is_taken() {
                continue;
            }
            holder.target = listed.standing_for(&holder.target);
            // Each pair of rows once, whichever of the two it was found
            // from and in however many of their key contexts.
            let seen = found.iter().any(|collision| {
                (collision.target == holder.target
                    && collision.key == holder.keys
                    && collision.holder.target == target
                    && collision.holder.keys == binding.keys)
                    || (collision.target == target
                        && collision.key == binding.keys
                        && collision.holder.target == holder.target
                        && collision.holder.keys == holder.keys)
            });
            if !seen {
                found.push(Collision {
                    target: target.clone(),
                    key: binding.keys.clone(),
                    holder,
                });
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::defaults::DefaultBinding;

    fn key(text: &str) -> KeySequence {
        KeySequence::parse(text).unwrap()
    }

    fn places() -> Places {
        Places::new()
            .typing("Editor")
            .typing("TextField")
            .inside("TextField", "FindBar")
            .inside("TextField", "LibraryPanel")
            .container("FindBar")
    }

    /// The app's order in miniature: the window's keys first, the widgets,
    /// the text, the library, the comments' window-wide key, the tools.
    fn defaults() -> Defaults {
        let binding = |action: &str, context: Option<&str>, keys: &str| {
            DefaultBinding::new(action, None, context, keys).unwrap()
        };
        let mut defaults = Defaults::from_bindings([
            binding("app::Quit", None, "cmd-q"),
            binding("app::QuickOpen", None, "cmd-p"),
            binding("field::Confirm", Some("TextField"), "enter"),
            binding("editor::ToggleBold", Some("Editor"), "cmd-b"),
            binding("editor::Newline", Some("Editor"), "enter"),
            binding("editor::PageBreak", Some("Editor"), "cmd-enter"),
            binding(
                "library::SelectFirst",
                Some("LibraryPanel && !TextField"),
                "g g",
            ),
            binding("library::Rename", Some("LibraryPanel && !TextField"), "f2"),
            binding(
                "library::NewFolder",
                Some("LibraryPanel && mode == tree && !TextField"),
                "shift-n",
            ),
            binding("comments::Toggle", None, "cmd-alt-c"),
            binding("tools::Find", Some("EditorTools"), "cmd-f"),
            binding("tools::FindNext", Some("FindBar"), "cmd-g"),
            binding(
                "tools::CaseSensitive",
                Some("FindBar > TextField"),
                "cmd-alt-c",
            ),
            binding(
                "tools::ReplaceAll",
                Some("FindBar > TextField"),
                "cmd-enter",
            ),
        ]);
        defaults.add_keyless(Target::new("app::VersionHistory", None));
        defaults
    }

    fn ask(action: &str, context: Option<&str>, text: &str) -> Verdict {
        check(
            &defaults(),
            &Changes::new(),
            &places(),
            &Target::new(action, context),
            &key(text),
            None,
        )
    }

    #[test]
    fn a_free_key_is_fine() {
        let verdict = ask("editor::ToggleBold", Some("Editor"), "cmd-alt-b");
        assert_eq!(verdict.kind(), VerdictKind::Fine);
        assert!(verdict.is_allowed());
        assert!(verdict.warnings.is_empty());
        assert_eq!(verdict.key, key("cmd-alt-b"));
    }

    #[test]
    fn a_character_where_text_is_typed_is_refused() {
        for text in ["b", "shift-b", "B", "7", "?", "space", "b cmd-b"] {
            let verdict = ask("editor::ToggleBold", Some("Editor"), text);
            assert_eq!(verdict.refusal, Some(Refusal::TypesText), "{text}");
            assert_eq!(verdict.kind(), VerdictKind::Refused);
            assert!(!verdict.is_allowed());
        }
        // Anywhere includes the text.
        assert_eq!(
            ask("app::Quit", None, "q").refusal,
            Some(Refusal::TypesText)
        );
        // Not where no text is typed: the library has such keys today.
        assert_eq!(
            ask("library::Rename", Some("LibraryPanel && !TextField"), "r").refusal,
            None
        );
        // A character after a command key is not typed.
        assert_eq!(
            ask("editor::ToggleBold", Some("Editor"), "cmd-k b").refusal,
            None
        );
    }

    #[test]
    fn a_default_key_can_always_be_given_back() {
        // The library's field confirms with enter, a key a text is edited
        // with; a place could have a bare character by default too.
        let binding = |action: &str, context: Option<&str>, keys: &str| {
            DefaultBinding::new(action, None, context, keys).unwrap()
        };
        let defaults = Defaults::from_bindings([
            binding("field::Confirm", Some("TextField"), "enter"),
            binding("field::Space", Some("TextField"), "space"),
        ]);
        let confirm = Target::new("field::Confirm", Some("TextField"));
        let space = Target::new("field::Space", Some("TextField"));
        let mut changes = Changes::new();
        changes.remove_key(&defaults, &confirm, &key("enter"));
        changes.remove_key(&defaults, &space, &key("space"));
        for (target, text) in [(&confirm, "enter"), (&space, "space")] {
            let verdict = check(&defaults, &changes, &places(), target, &key(text), None);
            assert_eq!(verdict.kind(), VerdictKind::Fine, "{text}");
            assert!(verdict.warnings.is_empty(), "{text}");
        }
        // Another command does not get that key there.
        let verdict = check(
            &defaults,
            &changes,
            &places(),
            &confirm,
            &key("space"),
            None,
        );
        assert_eq!(verdict.refusal, Some(Refusal::TypesText));
    }

    #[test]
    fn the_keys_of_macos_are_refused_or_warned_about() {
        let kept = ask("editor::ToggleBold", Some("Editor"), "cmd-tab");
        assert!(matches!(kept.refusal, Some(Refusal::SystemKeeps(system)) if system.kept));
        assert!(matches!(
            ask("app::Quit", None, "cmd-alt-escape").refusal,
            Some(Refusal::SystemKeeps(_))
        ));
        // Spotlight's key can be freed in System Settings: a warning.
        let spotlight = ask("app::Quit", None, "cmd-space");
        assert_eq!(spotlight.refusal, None);
        assert!(matches!(spotlight.warnings[..], [Warning::SystemUses(_)]));
        let usual = ask("editor::ToggleBold", Some("Editor"), "ctrl-up");
        assert_eq!(usual.refusal, None);
        assert!(matches!(usual.warnings[..], [Warning::SystemUses(system)] if !system.kept));
        assert!(usual.is_allowed());
    }

    #[test]
    fn warnings_for_keys_a_text_needs() {
        let special = ask("editor::ToggleBold", Some("Editor"), "alt-e");
        assert_eq!(special.warnings, vec![Warning::SpecialCharacter]);
        assert!(special.is_allowed());
        let editing = ask("editor::ToggleBold", Some("Editor"), "shift-backspace");
        assert_eq!(editing.warnings, vec![Warning::EditingKey]);
        // Not where no text is typed.
        assert!(
            ask(
                "library::Rename",
                Some("LibraryPanel && !TextField"),
                "alt-e"
            )
            .warnings
            .is_empty()
        );
    }

    #[test]
    fn the_key_a_command_has_already() {
        let verdict = ask("editor::ToggleBold", Some("Editor"), "cmd-b");
        assert_eq!(verdict.refusal, Some(Refusal::AlreadyHasKey));
        let unknown = ask("editor::ToggleBold", Some("LibraryPanel"), "cmd-b");
        assert_eq!(unknown.refusal, Some(Refusal::UnknownCommand));
    }

    #[test]
    fn a_key_another_command_has_in_the_same_place_is_taken() {
        let verdict = ask("editor::ToggleBold", Some("Editor"), "cmd-enter");
        assert_eq!(verdict.kind(), VerdictKind::Taken);
        assert_eq!(verdict.taken.len(), 1);
        let holder = &verdict.taken[0];
        assert_eq!(
            holder.target,
            Target::new("editor::PageBreak", Some("Editor"))
        );
        assert_eq!(holder.overlap, Overlap::SamePlace);
        assert_eq!(holder.origin, Origin::Default);
        // The find bar's field has the key too: another place.
        assert_eq!(verdict.elsewhere.len(), 1);
        assert_eq!(verdict.elsewhere[0].overlap, Overlap::Apart);

        // Taking it from that command settles it.
        let defaults = defaults();
        let mut changes = Changes::new();
        assert!(holder.take(&defaults, &mut changes));
        let again = check(
            &defaults,
            &changes,
            &places(),
            &Target::new("editor::ToggleBold", Some("Editor")),
            &key("cmd-enter"),
            None,
        );
        assert_eq!(again.kind(), VerdictKind::UsedElsewhere);
        assert!(again.is_allowed());
    }

    #[test]
    fn the_key_being_replaced_is_not_in_the_way() {
        let defaults = defaults();
        let mut changes = Changes::new();
        let bold = Target::new("editor::ToggleBold", Some("Editor"));
        let page = Target::new("editor::PageBreak", Some("Editor"));
        // Swap by hand: the page break gets another key first.
        changes.replace_key(&defaults, &page, &key("cmd-enter"), key("cmd-alt-enter"));
        let verdict = check(
            &defaults,
            &changes,
            &places(),
            &bold,
            &key("cmd-enter"),
            Some(&key("cmd-b")),
        );
        assert!(verdict.taken.is_empty(), "{verdict:?}");
        // And the key bold gives up is free for the page break.
        changes.replace_key(&defaults, &bold, &key("cmd-b"), key("cmd-enter"));
        let verdict = check(
            &defaults,
            &changes,
            &places(),
            &page,
            &key("cmd-b"),
            Some(&key("cmd-alt-enter")),
        );
        assert_eq!(verdict.kind(), VerdictKind::Fine);
    }

    #[test]
    fn window_wide_keys_and_keys_of_a_place() {
        // The find bar's field and the window both have cmd-alt-c today.
        // A third command of the text on it: the window-wide one comes
        // later in the keymap and ranks like the deepest, so it would win
        // in the text and the new key would do nothing.
        let shadowed = ask("editor::ToggleBold", Some("Editor"), "cmd-alt-c");
        assert_eq!(shadowed.kind(), VerdictKind::Taken);
        assert_eq!(
            shadowed.taken[0].target,
            Target::new("comments::Toggle", None)
        );
        assert_eq!(shadowed.taken[0].overlap, Overlap::ProposedNeverRuns);
        assert_eq!(shadowed.elsewhere[0].overlap, Overlap::Apart);

        // A window-wide key that comes before the text's bindings: the
        // text's key wins in the text, the window keeps it elsewhere.
        let over = ask("editor::ToggleBold", Some("Editor"), "cmd-p");
        assert_eq!(over.kind(), VerdictKind::UsedElsewhere);
        assert_eq!(
            over.elsewhere[0].target,
            Target::new("app::QuickOpen", None)
        );
        assert_eq!(over.elsewhere[0].overlap, Overlap::ProposedWinsThere);

        // Two window-wide commands on one key: the same place.
        let same = ask("app::Quit", None, "cmd-p");
        assert_eq!(same.taken[0].overlap, Overlap::SamePlace);

        // A window-wide key that a place has: the place's binding comes
        // later, so the place keeps it and the rest of the window gets
        // the new one.
        let window = ask("app::Quit", None, "cmd-b");
        assert_eq!(window.kind(), VerdictKind::UsedElsewhere);
        assert_eq!(window.elsewhere[0].overlap, Overlap::HolderWinsThere);

        // The find bar never has the keyboard itself: against a
        // window-wide key that comes later than its own binding stood,
        // its command would never run.
        let bar = ask("comments::Toggle", None, "cmd-g");
        assert_eq!(bar.kind(), VerdictKind::Taken);
        assert_eq!(bar.taken[0].overlap, Overlap::HolderNeverRuns);
        // And a key for the bar that the window has does nothing.
        let bar = ask("tools::FindNext", Some("FindBar"), "cmd-p");
        assert_eq!(bar.taken[0].overlap, Overlap::ProposedNeverRuns);
        // Its field does keep a key against the window (bound later).
        let field = ask("tools::ReplaceAll", Some("FindBar > TextField"), "cmd-p");
        assert_eq!(field.kind(), VerdictKind::UsedElsewhere);
        assert_eq!(field.elsewhere[0].overlap, Overlap::ProposedWinsThere);
    }

    #[test]
    fn a_key_for_a_command_without_one_loses_ties() {
        // At the front of the keymap: the text keeps cmd-b, and the new
        // key works wherever the text does not have the keyboard.
        let history = ask("app::VersionHistory", None, "cmd-b");
        assert_eq!(history.kind(), VerdictKind::UsedElsewhere);
        assert_eq!(history.elsewhere[0].overlap, Overlap::HolderWinsThere);
        // Against another window-wide key it is the same place.
        let taken = ask("app::VersionHistory", None, "cmd-q");
        assert_eq!(taken.taken[0].overlap, Overlap::SamePlace);
        // The find bar's own binding still loses to a window-wide one.
        let bar = ask("app::VersionHistory", None, "cmd-g");
        assert_eq!(bar.taken[0].overlap, Overlap::HolderNeverRuns);
    }

    #[test]
    fn the_same_context_with_more_conditions() {
        // New Folder is bound for the tree only, later than Rename for
        // the whole list: on the same key it would win in the tree, and
        // Rename would keep the key in the trash.
        let tree = ask(
            "library::NewFolder",
            Some("LibraryPanel && mode == tree && !TextField"),
            "f2",
        );
        assert_eq!(tree.kind(), VerdictKind::UsedElsewhere);
        assert_eq!(tree.elsewhere[0].overlap, Overlap::ProposedWinsThere);
        // The other way round: Rename on New Folder's key would do
        // nothing in the tree and work in the trash.
        let list = ask(
            "library::Rename",
            Some("LibraryPanel && !TextField"),
            "shift-n",
        );
        assert_eq!(list.kind(), VerdictKind::UsedElsewhere);
        assert_eq!(list.elsewhere[0].overlap, Overlap::HolderWinsThere);
    }

    #[test]
    fn one_key_that_begins_another() {
        let verdict = ask("library::Rename", Some("LibraryPanel && !TextField"), "g");
        assert_eq!(verdict.kind(), VerdictKind::Taken);
        assert_eq!(verdict.taken[0].keys, key("g g"));
        let longer = ask(
            "library::Rename",
            Some("LibraryPanel && !TextField"),
            "f2 f2",
        );
        // Its own key is not a holder.
        assert_eq!(longer.kind(), VerdictKind::Fine);
    }

    #[test]
    fn the_same_command_in_another_place_is_no_holder() {
        let binding = |action: &str, context: Option<&str>, keys: &str| {
            DefaultBinding::new(action, None, context, keys).unwrap()
        };
        let defaults = Defaults::from_bindings([
            binding("menu::Open", Some("LibraryPanel"), "ctrl-enter"),
            binding("menu::Open", Some("TextField"), "ctrl-enter"),
        ]);
        let verdict = check(
            &defaults,
            &Changes::new(),
            &places(),
            &Target::new("menu::Open", Some("LibraryPanel")),
            &key("shift-f10"),
            None,
        );
        assert_eq!(verdict.kind(), VerdictKind::Fine);
        let mut changes = Changes::new();
        changes.add_key(
            &defaults,
            &Target::new("menu::Open", Some("TextField")),
            key("shift-f10"),
        );
        let verdict = check(
            &defaults,
            &changes,
            &places(),
            &Target::new("menu::Open", Some("LibraryPanel")),
            &key("shift-f10"),
            None,
        );
        assert_eq!(verdict.kind(), VerdictKind::Fine);
    }

    #[test]
    fn conflicts_left_in_a_keymap() {
        let defaults = defaults();
        let places = places();
        assert!(collisions(&defaults, &Changes::new(), &places).is_empty());

        let mut changes = Changes::new();
        let bold = Target::new("editor::ToggleBold", Some("Editor"));
        changes.replace_key(&defaults, &bold, &key("cmd-b"), key("cmd-enter"));
        let found = collisions(&defaults, &changes, &places);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].target, bold);
        assert_eq!(found[0].key, key("cmd-enter"));
        assert_eq!(
            found[0].holder.target,
            Target::new("editor::PageBreak", Some("Editor"))
        );

        // Two of the user's keys against each other: the pair once.
        let page = Target::new("editor::PageBreak", Some("Editor"));
        changes.replace_key(&defaults, &page, &key("cmd-enter"), key("f6"));
        changes.replace_key(&defaults, &bold, &key("cmd-enter"), key("f6"));
        let found = collisions(&defaults, &changes, &places);
        assert_eq!(found.len(), 1, "{found:?}");
    }
}
