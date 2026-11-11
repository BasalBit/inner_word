//! What `cmd-z` in the library takes back: moves to the trash, renames, moves to another
//! folder and reorders. A stack for the session, newest last.

use gpui::Context;
use writer_store::{FolderId, StoreError};
use writer_ui::Toast;
use writer_ui::toast::show_toast;

use super::rows::{UNTITLED, place_of};
use super::{LibraryItem, LibraryPanel};

/// How many changes the panel remembers.
const UNDO_LIMIT: usize = 64;

/// One change that can be taken back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum UndoEntry {
    /// The item went to the trash: restore it. Recorded from the library's events, so
    /// items trashed by the app are covered too.
    Trash(LibraryItem),
    /// The item was renamed in the panel; `name` is what it was called before (empty for
    /// a document that had no title).
    Rename { item: LibraryItem, name: String },
    /// The item was moved or reordered in the panel; it was at `index` among its siblings
    /// in `parent` before.
    Place {
        item: LibraryItem,
        parent: Option<FolderId>,
        index: usize,
        /// It changed folder (otherwise it only changed place among its siblings).
        moved: bool,
    },
}

impl LibraryPanel {
    fn remember(&mut self, entry: UndoEntry) {
        self.undo_stack.push(entry);
        if self.undo_stack.len() > UNDO_LIMIT {
            self.undo_stack.remove(0);
        }
    }

    pub(super) fn remember_trashed(&mut self, item: LibraryItem) {
        self.undo_stack
            .retain(|entry| *entry != UndoEntry::Trash(item));
        self.remember(UndoEntry::Trash(item));
    }

    pub(super) fn remember_rename(&mut self, item: LibraryItem, name: String) {
        self.remember(UndoEntry::Rename { item, name });
    }

    /// Remembers where `item` was before a move to another folder.
    pub(super) fn remember_move(&mut self, item: LibraryItem, from: (Option<FolderId>, usize)) {
        self.remember(UndoEntry::Place {
            item,
            parent: from.0,
            index: from.1,
            moved: true,
        });
    }

    /// Remembers where `item` was before a reorder. A run of reorders of the same item
    /// (holding `alt-up`) is one change: the place it started from.
    pub(super) fn remember_reorder(&mut self, item: LibraryItem, from: (Option<FolderId>, usize)) {
        if matches!(
            self.undo_stack.last(),
            Some(UndoEntry::Place { item: last, moved: false, .. }) if *last == item
        ) {
            return;
        }
        self.remember(UndoEntry::Place {
            item,
            parent: from.0,
            index: from.1,
            moved: false,
        });
    }

    /// Forgets the trash entries of items that are no longer in the trash.
    pub(super) fn forget_trashed_except(&mut self, in_trash: impl Fn(LibraryItem) -> bool) {
        self.undo_stack.retain(|entry| match entry {
            UndoEntry::Trash(item) => in_trash(*item),
            _ => true,
        });
    }

    pub(super) fn forget_trashed(&mut self, item: LibraryItem) {
        self.undo_stack
            .retain(|entry| *entry != UndoEntry::Trash(item));
    }

    /// Takes back the last change made in the library in this session: a move to the
    /// trash (by the panel or by anything else that went through the library), or a
    /// rename, a move to another folder or a reorder made in the panel. A toast says what
    /// was undone. Repeating it walks back through earlier changes; a change that can no
    /// longer be taken back (the item was deleted for good, its old folder is gone) is
    /// skipped. Returns whether something was undone.
    pub fn undo(&mut self, cx: &mut Context<Self>) -> bool {
        while let Some(entry) = self.undo_stack.pop() {
            if self.undo_entry(entry, cx) {
                return true;
            }
        }
        show_toast(Toast::new("Nothing to undo."), cx);
        false
    }

    /// The same as [`undo`](Self::undo); the name is from when only moves to the trash
    /// could be undone.
    pub fn undo_trash(&mut self, cx: &mut Context<Self>) -> bool {
        self.undo(cx)
    }

    /// Whether `cmd-z` has something to take back.
    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    /// Applies the reverse of `entry`. False when there was nothing to do any more.
    fn undo_entry(&mut self, entry: UndoEntry, cx: &mut Context<Self>) -> bool {
        match entry {
            UndoEntry::Trash(item) => self.restore_item(item, cx).is_some(),
            UndoEntry::Rename { item, name } => {
                let tree = self.library.read(cx).tree();
                if place_of(&tree, item).is_none() {
                    return false;
                }
                let current = self.name_of(item, cx);
                let renamed = self.library.update(cx, |library, cx| match item {
                    LibraryItem::Folder(id) => library.rename_folder(id, &name, cx),
                    LibraryItem::Document(id) => library.rename_document(id, &name, cx),
                });
                match renamed {
                    Ok(()) => {
                        self.reveal(item, cx);
                        let name = if name.trim().is_empty() {
                            UNTITLED
                        } else {
                            &name
                        };
                        show_toast(
                            Toast::new(format!("Renamed “{current}” back to “{name}”.")),
                            cx,
                        );
                        true
                    }
                    Err(err) => self.undo_failed(err, cx),
                }
            }
            UndoEntry::Place {
                item,
                parent,
                index,
                moved,
            } => {
                let tree = self.library.read(cx).tree();
                let Some(current) = place_of(&tree, item) else {
                    return false;
                };
                // The folder it was in is gone, or it is back there already.
                if parent.is_some_and(|parent| tree.find_folder(parent).is_none())
                    || current == (parent, index)
                {
                    return false;
                }
                drop(tree);
                let placed = self.library.update(cx, |library, cx| match item {
                    LibraryItem::Folder(id) => library.move_folder(id, parent, Some(index), cx),
                    LibraryItem::Document(id) => library.move_document(id, parent, Some(index), cx),
                });
                match placed {
                    Ok(()) => {
                        self.reveal(item, cx);
                        let name = self.name_of(item, cx);
                        let message = if moved {
                            let place = self.place_label(parent, cx);
                            format!("Moved “{name}” back to {place}.")
                        } else {
                            format!("Moved “{name}” back to its place.")
                        };
                        show_toast(Toast::new(message), cx);
                        true
                    }
                    Err(err) => self.undo_failed(err, cx),
                }
            }
        }
    }

    /// An undo that the store refused. A missing item is skipped silently (the next entry
    /// is tried); anything else is said, and counts as the answer to this `cmd-z`.
    fn undo_failed(&mut self, err: StoreError, cx: &mut Context<Self>) -> bool {
        match err {
            StoreError::NotFound { .. } | StoreError::NotInTrash { .. } => false,
            err => {
                self.report_error(&err.to_string(), cx);
                true
            }
        }
    }
}
