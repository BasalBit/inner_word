//! Soft delete: trash, restore, purge.
//!
//! Trashing sets `trashed_at`. Trashing a folder also trashes every live folder and document
//! in its subtree and marks them `trashed_with = <that folder>`; only the folder itself is a
//! top-level trash entry. Restoring or purging that folder restores or purges exactly the
//! items trashed with it. Items trashed individually earlier stay separate entries.

use rusqlite::{Connection, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

use crate::Store;
use crate::documents::{
    DOC_META_COLS, DocumentMeta, load_meta, meta_from_row, require_live_document,
};
use crate::error::{ItemKind, Result, StoreError};
use crate::folders::{
    FOLDER_COLS, Folder, Siblings, folder_from_row, is_live_folder, load_folder, next_position,
    place_at, require_live_folder, sibling_ids, write_positions,
};
use crate::ids::{DocumentId, FolderId};

/// The position of a trashed item whose folder was deleted for good: it has no place to
/// go back to, and a restore puts it at the end of the root.
const NO_PLACE: i64 = -1;

/// One top-level entry in the trash.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrashEntry {
    /// A trashed folder and how much was trashed with it.
    Folder {
        /// The folder (`trashed_at` is set).
        folder: Folder,
        /// Subfolders trashed together with it (all depths).
        folder_count: u64,
        /// Documents trashed together with it (all depths).
        document_count: u64,
    },
    /// A document trashed on its own.
    Document(DocumentMeta),
}

impl TrashEntry {
    /// When the entry was trashed, unix ms.
    pub fn trashed_at(&self) -> i64 {
        match self {
            TrashEntry::Folder { folder, .. } => folder.trashed_at.unwrap_or(0),
            TrashEntry::Document(doc) => doc.trashed_at.unwrap_or(0),
        }
    }
}

/// What [`Store::empty_trash`] deleted.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct EmptiedTrash {
    /// Folders deleted.
    pub folders: u64,
    /// Documents deleted.
    pub documents: u64,
}

/// Fails unless the folder is a top-level trash entry.
fn require_trash_folder(conn: &Connection, id: FolderId) -> Result<Folder> {
    let folder = load_folder(conn, id)?;
    let top_level: bool = conn.query_row(
        "SELECT trashed_at IS NOT NULL AND trashed_with IS NULL FROM folders WHERE id = ?1",
        [id],
        |r| r.get(0),
    )?;
    if !top_level {
        return Err(StoreError::NotInTrash {
            kind: ItemKind::Folder,
            id: id.0,
        });
    }
    Ok(folder)
}

fn require_trash_document(conn: &Connection, id: DocumentId) -> Result<DocumentMeta> {
    let meta = load_meta(conn, id)?;
    let top_level: bool = conn.query_row(
        "SELECT trashed_at IS NOT NULL AND trashed_with IS NULL FROM documents WHERE id = ?1",
        [id],
        |r| r.get(0),
    )?;
    if !top_level {
        return Err(StoreError::NotInTrash {
            kind: ItemKind::Document,
            id: id.0,
        });
    }
    Ok(meta)
}

impl Store {
    /// Moves a live document to the trash. Its former siblings are renumbered.
    pub fn trash_document(&mut self, id: DocumentId) -> Result<()> {
        let now = self.now();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let meta = require_live_document(&tx, id)?;
        tx.execute(
            "UPDATE documents SET trashed_at = ?2, trashed_with = NULL WHERE id = ?1",
            params![id, now],
        )?;
        let siblings = sibling_ids(&tx, Siblings::Documents, meta.folder_id)?;
        write_positions(&tx, Siblings::Documents, &siblings)?;
        tx.commit()?;
        Ok(())
    }

    /// Moves a live folder and its whole live subtree (folders and documents) to the trash.
    pub fn trash_folder(&mut self, id: FolderId) -> Result<()> {
        let now = self.now();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_live_folder(&tx, id)?;
        let folder = load_folder(&tx, id)?;
        const SUBTREE: &str = "WITH RECURSIVE sub(id) AS (
                SELECT ?1
                UNION
                SELECT f.id FROM folders f JOIN sub ON f.parent_id = sub.id
                WHERE f.trashed_at IS NULL
            )";
        tx.execute(
            &format!(
                "{SUBTREE} UPDATE documents SET trashed_at = ?2, trashed_with = ?1
                 WHERE trashed_at IS NULL AND folder_id IN (SELECT id FROM sub)"
            ),
            params![id, now],
        )?;
        tx.execute(
            &format!(
                "{SUBTREE} UPDATE folders SET trashed_at = ?2, trashed_with = ?1
                 WHERE id != ?1 AND id IN (SELECT id FROM sub)"
            ),
            params![id, now],
        )?;
        tx.execute(
            "UPDATE folders SET trashed_at = ?2, trashed_with = NULL WHERE id = ?1",
            params![id, now],
        )?;
        let siblings = sibling_ids(&tx, Siblings::Folders, folder.parent_id)?;
        write_positions(&tx, Siblings::Folders, &siblings)?;
        tx.commit()?;
        Ok(())
    }

    /// Top-level trash entries, most recently trashed first.
    pub fn trash(&self) -> Result<Vec<TrashEntry>> {
        let mut entries = Vec::new();
        {
            let mut stmt = self.conn.prepare_cached(&format!(
                "SELECT {FOLDER_COLS},
                    (SELECT count(*) FROM folders c WHERE c.trashed_with = folders.id),
                    (SELECT count(*) FROM documents d WHERE d.trashed_with = folders.id)
                 FROM folders WHERE trashed_at IS NOT NULL AND trashed_with IS NULL"
            ))?;
            let rows = stmt.query_map([], |row| {
                Ok(TrashEntry::Folder {
                    folder: folder_from_row(row)?,
                    folder_count: row.get::<_, i64>(7)?.max(0) as u64,
                    document_count: row.get::<_, i64>(8)?.max(0) as u64,
                })
            })?;
            for row in rows {
                entries.push(row?);
            }
        }
        {
            let mut stmt = self.conn.prepare_cached(&format!(
                "SELECT {DOC_META_COLS} FROM documents
                 WHERE trashed_at IS NOT NULL AND trashed_with IS NULL"
            ))?;
            for row in stmt.query_map([], meta_from_row)? {
                entries.push(TrashEntry::Document(row?));
            }
        }
        entries.sort_by_key(|e| std::cmp::Reverse(e.trashed_at()));
        Ok(entries)
    }

    /// Restores a trashed document into its original folder when that folder is live, at
    /// the place it had among the documents there (exactly so when it is the last thing
    /// trashed from that folder and nothing was reordered since, which is what an undo
    /// does). When the folder is gone it goes to the root, after the documents there.
    /// Returns the restored metadata. [`StoreError::NotInTrash`] unless it is a top-level
    /// trash entry.
    pub fn restore_document(&mut self, id: DocumentId) -> Result<DocumentMeta> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let meta = require_trash_document(&tx, id)?;
        let target = match meta.folder_id {
            Some(f) if is_live_folder(&tx, f)? => Some(f),
            _ => None,
        };
        let position = next_position(&tx, Siblings::Documents, target)?;
        tx.execute(
            "UPDATE documents SET trashed_at = NULL, trashed_with = NULL, folder_id = ?2,
                 position = ?3 WHERE id = ?1",
            params![id, target, position],
        )?;
        if target == meta.folder_id && meta.position != NO_PLACE {
            // Its old position is its old index: trashing renumbered the others without
            // a gap.
            let index = usize::try_from(meta.position).unwrap_or(0);
            place_at(&tx, Siblings::Documents, target, id.0, index)?;
        }
        let meta = load_meta(&tx, id)?;
        tx.commit()?;
        Ok(meta)
    }

    /// Restores a trashed folder together with everything trashed with it, into its
    /// original parent when that is live, at the place it had among the folders there (see
    /// [`Store::restore_document`]); otherwise to the root, after the folders there. Items
    /// inside it that were trashed separately stay in the trash.
    pub fn restore_folder(&mut self, id: FolderId) -> Result<Folder> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let folder = require_trash_folder(&tx, id)?;
        let target = match folder.parent_id {
            Some(p) if is_live_folder(&tx, p)? => Some(p),
            _ => None,
        };
        let position = next_position(&tx, Siblings::Folders, target)?;
        tx.execute(
            "UPDATE folders SET trashed_at = NULL, trashed_with = NULL WHERE trashed_with = ?1",
            [id],
        )?;
        tx.execute(
            "UPDATE documents SET trashed_at = NULL, trashed_with = NULL WHERE trashed_with = ?1",
            [id],
        )?;
        tx.execute(
            "UPDATE folders SET trashed_at = NULL, parent_id = ?2, position = ?3 WHERE id = ?1",
            params![id, target, position],
        )?;
        if target == folder.parent_id && folder.position != NO_PLACE {
            let index = usize::try_from(folder.position).unwrap_or(0);
            place_at(&tx, Siblings::Folders, target, id.0, index)?;
        }
        let folder = load_folder(&tx, id)?;
        tx.commit()?;
        Ok(folder)
    }

    /// Permanently deletes a top-level trashed document and its snapshots.
    pub fn purge_document(&mut self, id: DocumentId) -> Result<()> {
        let tx = self.conn.transaction()?;
        require_trash_document(&tx, id)?;
        tx.execute("DELETE FROM documents WHERE id = ?1", [id])?;
        tx.commit()?;
        Ok(())
    }

    /// Permanently deletes a top-level trashed folder and everything trashed with it.
    /// Separately trashed items that lived inside it stay in the trash, now restoring to
    /// the root.
    pub fn purge_folder(&mut self, id: FolderId) -> Result<()> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_trash_folder(&tx, id)?;
        // What was trashed on its own inside this folder survives and loses its folder
        // (the foreign key sets it to the root). It never had a place at the root, so
        // mark it: a restore then puts it at the end instead of at its old index.
        const DOOMED: &str = "SELECT id FROM folders WHERE id = ?1 OR trashed_with = ?1";
        tx.execute(
            &format!(
                "UPDATE documents SET position = {NO_PLACE}
                 WHERE trashed_with IS NOT ?1 AND folder_id IN ({DOOMED})"
            ),
            [id],
        )?;
        tx.execute(
            &format!(
                "UPDATE folders SET position = {NO_PLACE}
                 WHERE id != ?1 AND trashed_with IS NOT ?1 AND parent_id IN ({DOOMED})"
            ),
            [id],
        )?;
        tx.execute("DELETE FROM documents WHERE trashed_with = ?1", [id])?;
        tx.execute("DELETE FROM folders WHERE trashed_with = ?1", [id])?;
        tx.execute("DELETE FROM folders WHERE id = ?1", [id])?;
        tx.commit()?;
        Ok(())
    }

    /// Permanently deletes everything in the trash.
    pub fn empty_trash(&mut self) -> Result<EmptiedTrash> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        // Count first: rows removed by foreign-key cascades are not reported as changes.
        let documents: i64 = tx.query_row(
            "SELECT count(*) FROM documents WHERE trashed_at IS NOT NULL",
            [],
            |r| r.get(0),
        )?;
        let folders: i64 = tx.query_row(
            "SELECT count(*) FROM folders WHERE trashed_at IS NOT NULL",
            [],
            |r| r.get(0),
        )?;
        tx.execute("DELETE FROM documents WHERE trashed_at IS NOT NULL", [])?;
        tx.execute("DELETE FROM folders WHERE trashed_at IS NOT NULL", [])?;
        tx.commit()?;
        Ok(EmptiedTrash {
            folders: folders.max(0) as u64,
            documents: documents.max(0) as u64,
        })
    }
}
