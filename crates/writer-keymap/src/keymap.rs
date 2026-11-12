//! [`Keymap`]: the defaults, the places and the user's changes held
//! together, for the app to keep in one place.

use crate::changes::{Changes, StoredError};
use crate::context::Places;
use crate::defaults::{Command, Defaults, Target};
use crate::effective::{Effective, apply};
use crate::file::{Catalogue, ImportError, ImportMode, ImportPlan, export, plan_import_among};
use crate::keystroke::KeySequence;
use crate::verdict::{Absent, Collision, Holder, Verdict, check_among, collisions_among};

/// The keymap of the app as data: what every crate bound, what the caller
/// says about the places of the window, and what the user changed. Every
/// method is one of the crate's functions with these three filled in, and
/// with the commands that are not there right now ([`Keymap::set_absent`]).
///
/// ```
/// use writer_keymap::{DefaultBinding, Defaults, KeySequence, Keymap, Places, Target};
///
/// let mut defaults = Defaults::new();
/// defaults.push(DefaultBinding::new("editor::ToggleBold", None, Some("Editor"), "cmd-b").unwrap());
/// let mut keymap = Keymap::new(defaults, Places::new().typing("Editor"));
///
/// let bold = Target::new("editor::ToggleBold", Some("Editor"));
/// let old = KeySequence::parse("cmd-b").unwrap();
/// let new = KeySequence::parse("cmd-alt-b").unwrap();
/// assert!(keymap.check(&bold, &new, Some(&old)).is_allowed());
/// assert!(keymap.replace_key(&bold, &old, new));
///
/// let effective = keymap.effective();
/// assert_eq!(effective.bindings()[0].spelling, "cmd-alt-b");
/// assert!(effective.target(&bold).unwrap().changed);
/// assert!(keymap.stored().is_some());
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct Keymap {
    defaults: Defaults,
    places: Places,
    changes: Changes,
    absent: Absent,
}

impl Keymap {
    /// The default keymap, without changes.
    pub fn new(defaults: Defaults, places: Places) -> Self {
        Self {
            defaults,
            places,
            changes: Changes::new(),
            absent: Absent::new(),
        }
    }

    /// The keymap with the changes stored in the library's settings
    /// (`None`: nothing is stored). A stored value that cannot be read
    /// gives the default keymap and the error; the caller says so and
    /// leaves the stored text alone until the user changes a key.
    pub fn with_stored(
        defaults: Defaults,
        places: Places,
        stored: Option<&str>,
    ) -> (Self, Option<StoredError>) {
        let mut keymap = Self::new(defaults, places);
        let error = match stored.map(Changes::from_stored) {
            Some(Ok(changes)) => {
                keymap.changes = changes;
                None
            }
            Some(Err(error)) => Some(error),
            None => None,
        };
        (keymap, error)
    }

    /// The defaults.
    pub fn defaults(&self) -> &Defaults {
        &self.defaults
    }

    /// The places.
    pub fn places(&self) -> &Places {
        &self.places
    }

    /// The user's changes.
    pub fn changes(&self) -> &Changes {
        &self.changes
    }

    /// Says which commands are not there right now (a part of the app
    /// that is not shown on this Mac, such as the assistant without its
    /// engine), by their targets. Their keys then count for nothing in
    /// [`Keymap::check`], [`Keymap::collisions`] and the conflicts of
    /// [`Keymap::plan_import`]: see [`crate::verdict`]. Nothing else
    /// changes: they are in [`Keymap::effective`] and in the exported
    /// file, and the user's changes to them are kept. Returns whether the
    /// set changed.
    pub fn set_absent(&mut self, absent: impl IntoIterator<Item = Target>) -> bool {
        let absent: Absent = absent.into_iter().collect();
        let changed = absent != self.absent;
        self.absent = absent;
        changed
    }

    /// The commands that are not there right now ([`Keymap::set_absent`]).
    pub fn absent(&self) -> &Absent {
        &self.absent
    }

    /// Whether `target` is a command that is not there right now.
    pub fn is_absent(&self, target: &Target) -> bool {
        self.absent.contains(target)
    }

    /// Puts other changes in the place of the user's and returns the old
    /// ones (carrying out an import, undoing one).
    pub fn set_changes(&mut self, changes: Changes) -> Changes {
        std::mem::replace(&mut self.changes, changes)
    }

    /// The value to store in the library's settings; `None` when there is
    /// nothing to store and the setting can be removed.
    pub fn stored(&self) -> Option<String> {
        (!self.changes.is_empty()).then(|| self.changes.to_stored())
    }

    /// The keymap to install and the state of every target: see
    /// [`apply`]. Computed on each call.
    pub fn effective(&self) -> Effective {
        apply(&self.defaults, &self.changes)
    }

    /// What would happen if `target` got `key`, in the place of its key
    /// `replacing` or as one more: see [`check`](crate::check).
    pub fn check(
        &self,
        target: &Target,
        key: &KeySequence,
        replacing: Option<&KeySequence>,
    ) -> Verdict {
        check_among(
            &self.defaults,
            &self.changes,
            &self.places,
            &self.absent,
            target,
            key,
            replacing,
        )
    }

    /// The conflicts in the keymap as it is: see
    /// [`collisions`](crate::collisions).
    pub fn collisions(&self) -> Vec<Collision> {
        collisions_among(&self.defaults, &self.changes, &self.places, &self.absent)
    }

    /// "Take it from that command": removes the key from a holder a
    /// verdict named. See [`Holder::take`].
    pub fn take(&mut self, holder: &Holder) -> bool {
        holder.take(&self.defaults, &mut self.changes)
    }

    /// See [`Changes::replace_key`].
    pub fn replace_key(&mut self, target: &Target, old: &KeySequence, new: KeySequence) -> bool {
        self.changes.replace_key(&self.defaults, target, old, new)
    }

    /// See [`Changes::add_key`].
    pub fn add_key(&mut self, target: &Target, new: KeySequence) -> bool {
        self.changes.add_key(&self.defaults, target, new)
    }

    /// See [`Changes::remove_key`].
    pub fn remove_key(&mut self, target: &Target, key: &KeySequence) -> bool {
        self.changes.remove_key(&self.defaults, target, key)
    }

    /// See [`Changes::set_keys`].
    pub fn set_keys(&mut self, target: &Target, keys: &[KeySequence]) -> bool {
        self.changes.set_keys(&self.defaults, target, keys)
    }

    /// Puts one command in one place back to its defaults: see
    /// [`Changes::reset`].
    pub fn reset(&mut self, target: &Target) -> bool {
        self.changes.reset(target)
    }

    /// Puts one command back in every place: see
    /// [`Changes::reset_command`].
    pub fn reset_command(&mut self, command: &Command) -> bool {
        self.changes.reset_command(command)
    }

    /// Puts everything back and returns the changes as they were, for
    /// whoever offers Undo.
    pub fn reset_all(&mut self) -> Changes {
        let before = self.changes.clone();
        self.changes.reset_all();
        before
    }

    /// The shortcuts file for the keymap as it is: see [`export`].
    pub fn export(&self, catalogue: &Catalogue, app_version: &str) -> String {
        export(&self.defaults, &self.changes, catalogue, app_version)
    }

    /// Reads a shortcuts file and plans its import: see
    /// [`plan_import`](crate::plan_import). Nothing is changed.
    pub fn plan_import(&self, text: &str) -> Result<ImportPlan, ImportError> {
        plan_import_among(
            text,
            &self.defaults,
            &self.changes,
            &self.places,
            &self.absent,
        )
    }

    /// Carries a plan out and returns the changes as they were, for Undo
    /// ([`Keymap::set_changes`] puts them back). The plan has to be one
    /// made for this keymap as it is now.
    pub fn import(&mut self, plan: &ImportPlan, mode: ImportMode) -> Changes {
        let carried = plan.carry_out(mode);
        self.set_changes(carried.changes)
    }
}
