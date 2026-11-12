//! What the view asks its host for.
//!
//! The view changes nothing itself. Whatever the user decides in it comes
//! out as a [`Request`] (an event of [`crate::KeymapView`]); the host
//! carries it out on its keymap, installs and stores the result, and hands
//! the view the new state ([`crate::KeymapView::set_listing`]).

use writer_keymap::{Changes, Holder, ImportMode, ImportPlan, KeySequence, Keymap, Target};

/// One thing the user asked for. See the [module docs](self).
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    /// Give `target` the key `new` in the place of its key `old`.
    ReplaceKey {
        /// The command and where.
        target: Target,
        /// The key it has.
        old: KeySequence,
        /// The key it gets.
        new: KeySequence,
        /// The commands the user chose to take the key from, each to be
        /// left without it ([`Holder::take`]). Empty when nothing is in
        /// the way, or when both keep the key.
        take: Vec<Holder>,
    },
    /// Give `target` one more key: a second one, or a first one for a
    /// command that has none.
    AddKey {
        /// The command and where.
        target: Target,
        /// The key it gets.
        new: KeySequence,
        /// As in [`Request::ReplaceKey`].
        take: Vec<Holder>,
    },
    /// Take the key `key` from `target`; the command may be left without
    /// one.
    RemoveKey {
        /// The command and where.
        target: Target,
        /// The key to take away.
        key: KeySequence,
    },
    /// Put `target` back to its default keys.
    UseDefault {
        /// The command and where.
        target: Target,
    },
    /// Put every key back to its default. The user has confirmed it.
    ResetAll,
    /// Write the shortcuts file (the host asks where).
    Export,
    /// Let the user choose a shortcuts file to import. The host reads it
    /// and answers with [`crate::KeymapView::show_import_preview`] or
    /// [`crate::KeymapView::show_import_error`].
    ChooseImportFile,
    /// Carry out the import the user looked at in the preview.
    Import {
        /// The plan the host handed to the preview.
        plan: Box<ImportPlan>,
        /// Replace the user's own changes, or add to them.
        mode: ImportMode,
    },
    /// Run the command of `target`.
    Run {
        /// The command and where.
        target: Target,
    },
}

impl Request {
    /// Carries out a request that changes keys on `keymap` and returns the
    /// user's changes as they were before (for Undo:
    /// [`Keymap::set_changes`] puts them back). `None` when nothing
    /// changed, and for the requests that are not about keys
    /// ([`Request::Export`], [`Request::ChooseImportFile`],
    /// [`Request::Run`]).
    ///
    /// The host still has to install and store the keymap afterwards.
    pub fn apply_to(&self, keymap: &mut Keymap) -> Option<Changes> {
        let before = keymap.changes().clone();
        let take = |keymap: &mut Keymap, holders: &[Holder]| {
            let mut changes = keymap.changes().clone();
            for holder in holders {
                holder.take(keymap.defaults(), &mut changes);
            }
            keymap.set_changes(changes);
        };
        match self {
            Request::ReplaceKey {
                target,
                old,
                new,
                take: holders,
            } => {
                take(keymap, holders);
                keymap.replace_key(target, old, new.clone());
            }
            Request::AddKey {
                target,
                new,
                take: holders,
            } => {
                take(keymap, holders);
                keymap.add_key(target, new.clone());
            }
            Request::RemoveKey { target, key } => {
                keymap.remove_key(target, key);
            }
            Request::UseDefault { target } => {
                keymap.reset(target);
            }
            Request::ResetAll => {
                keymap.reset_all();
            }
            Request::Import { plan, mode } => {
                keymap.import(plan, *mode);
            }
            Request::Export | Request::ChooseImportFile | Request::Run { .. } => return None,
        }
        (*keymap.changes() != before).then_some(before)
    }
}
