//! Folders: nested, manually ordered among sibling folders.

use rusqlite::{Connection, OptionalExtension, Row, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

use crate::Store;
use crate::error::{ItemKind, Result, StoreError, not_found};
use crate::ids::FolderId;

/// A folder row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Folder {
    /// The folder's id.
    pub id: FolderId,
    /// The parent folder, or `None` at the library root. For a trashed folder this is where
    /// it was (the parent may since have been purged, giving `None`).
    pub parent_id: Option<FolderId>,
    /// Display name (non-empty, stored as given).
    pub name: String,
    /// Sort key among sibling folders (ascending). Not necessarily contiguous.
    pub position: i64,
    /// Creation time, unix ms.
    pub created_at: i64,
    /// Last rename or move, unix ms.
    pub updated_at: i64,
    /// When the folder went to the trash, or `None` when live.
    pub trashed_at: Option<i64>,
}

pub(crate) const FOLDER_COLS: &str =
    "id, parent_id, name, position, created_at, updated_at, trashed_at";

pub(crate) fn folder_from_row(row: &Row<'_>) -> rusqlite::Result<Folder> {
    Ok(Folder {
        id: row.get(0)?,
        parent_id: row.get(1)?,
        name: row.get(2)?,
        position: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
        trashed_at: row.get(6)?,
    })
}

pub(crate) fn load_folder(conn: &Connection, id: FolderId) -> Result<Folder> {
    conn.query_row(
        &format!("SELECT {FOLDER_COLS} FROM folders WHERE id = ?1"),
        [id],
        folder_from_row,
    )
    .map_err(not_found(ItemKind::Folder, id.0))
}

/// Fails with `NotFound` unless the folder exists and is not trashed.
pub(crate) fn require_live_folder(conn: &Connection, id: FolderId) -> Result<()> {
    let live: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM folders WHERE id = ?1 AND trashed_at IS NULL",
            [id],
            |r| r.get(0),
        )
        .optional()?;
    match live {
        Some(_) => Ok(()),
        None => Err(StoreError::NotFound {
            kind: ItemKind::Folder,
            id: id.0,
        }),
    }
}

/// Whether the folder exists and is live.
pub(crate) fn is_live_folder(conn: &Connection, id: FolderId) -> Result<bool> {
    match require_live_folder(conn, id) {
        Ok(()) => Ok(true),
        Err(StoreError::NotFound { .. }) => Ok(false),
        Err(e) => Err(e),
    }
}

/// Which table a sibling list lives in.
#[derive(Copy, Clone)]
pub(crate) enum Siblings {
    Folders,
    Documents,
}

impl Siblings {
    fn table(self) -> &'static str {
        match self {
            Siblings::Folders => "folders",
            Siblings::Documents => "documents",
        }
    }
    fn parent_col(self) -> &'static str {
        match self {
            Siblings::Folders => "parent_id",
            Siblings::Documents => "folder_id",
        }
    }
}

/// Live sibling ids under `parent`, in display order.
pub(crate) fn sibling_ids(
    conn: &Connection,
    kind: Siblings,
    parent: Option<FolderId>,
) -> Result<Vec<i64>> {
    let sql = format!(
        "SELECT id FROM {t} WHERE {p} IS ?1 AND trashed_at IS NULL ORDER BY position, id",
        t = kind.table(),
        p = kind.parent_col()
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    let ids = stmt
        .query_map([parent], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<i64>>>()?;
    Ok(ids)
}

/// The position after the last live sibling under `parent`.
pub(crate) fn next_position(
    conn: &Connection,
    kind: Siblings,
    parent: Option<FolderId>,
) -> Result<i64> {
    let sql = format!(
        "SELECT coalesce(max(position) + 1, 0) FROM {t} WHERE {p} IS ?1 AND trashed_at IS NULL",
        t = kind.table(),
        p = kind.parent_col()
    );
    Ok(conn.query_row(&sql, [parent], |r| r.get(0))?)
}

/// Rewrites positions 0..n for `ids` in the given order.
pub(crate) fn write_positions(conn: &Connection, kind: Siblings, ids: &[i64]) -> Result<()> {
    let sql = format!(
        "UPDATE {t} SET position = ?2 WHERE id = ?1 AND position IS NOT ?2",
        t = kind.table()
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    for (pos, id) in ids.iter().enumerate() {
        stmt.execute(params![id, pos as i64])?;
    }
    Ok(())
}

/// Moves `id` to `index` (clamped) among its live siblings under `parent`, renumbering them.
pub(crate) fn place_at(
    conn: &Connection,
    kind: Siblings,
    parent: Option<FolderId>,
    id: i64,
    index: usize,
) -> Result<()> {
    let mut ids = sibling_ids(conn, kind, parent)?;
    ids.retain(|&other| other != id);
    let index = index.min(ids.len());
    ids.insert(index, id);
    write_positions(conn, kind, &ids)
}

fn clean_name(name: &str) -> Result<&str> {
    if name.trim().is_empty() {
        Err(StoreError::EmptyName)
    } else {
        Ok(name)
    }
}

impl Store {
    /// Creates a folder under `parent` (or at the root), after its existing sibling folders.
    ///
    /// Fails with [`StoreError::EmptyName`] for a blank name and `NotFound` when the parent
    /// does not exist or is trashed.
    pub fn create_folder(&mut self, parent: Option<FolderId>, name: &str) -> Result<Folder> {
        let name = clean_name(name)?;
        let now = self.now();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(parent) = parent {
            require_live_folder(&tx, parent)?;
        }
        let position = next_position(&tx, Siblings::Folders, parent)?;
        tx.execute(
            "INSERT INTO folders (parent_id, name, position, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?4)",
            params![parent, name, position, now],
        )?;
        let id = FolderId(tx.last_insert_rowid());
        let folder = load_folder(&tx, id)?;
        tx.commit()?;
        Ok(folder)
    }

    /// Loads a folder, live or trashed.
    pub fn folder(&self, id: FolderId) -> Result<Folder> {
        load_folder(&self.conn, id)
    }

    /// All live folders, ordered by parent then position. For a nested view use
    /// [`Store::library_tree`].
    pub fn folders(&self) -> Result<Vec<Folder>> {
        let mut stmt = self.conn.prepare_cached(&format!(
            "SELECT {FOLDER_COLS} FROM folders WHERE trashed_at IS NULL
             ORDER BY parent_id IS NOT NULL, parent_id, position, id"
        ))?;
        let rows = stmt
            .query_map([], folder_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Live subfolders of `parent` (or root folders), in display order.
    pub fn child_folders(&self, parent: Option<FolderId>) -> Result<Vec<Folder>> {
        let mut stmt = self.conn.prepare_cached(&format!(
            "SELECT {FOLDER_COLS} FROM folders WHERE parent_id IS ?1 AND trashed_at IS NULL
             ORDER BY position, id"
        ))?;
        let rows = stmt
            .query_map([parent], folder_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Renames a live folder and bumps its `updated_at`.
    pub fn rename_folder(&mut self, id: FolderId, name: &str) -> Result<()> {
        let name = clean_name(name)?;
        let now = self.now();
        let tx = self.conn.transaction()?;
        require_live_folder(&tx, id)?;
        tx.execute(
            "UPDATE folders SET name = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, name, now],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Moves a live folder (with its subtree) under `new_parent`, or to the root with
    /// `None`, placing it after the existing siblings there. Moving to the current parent
    /// does nothing.
    ///
    /// Fails with [`StoreError::FolderCycle`] when `new_parent` is the folder itself or one
    /// of its descendants, and `NotFound` when either folder is missing or trashed.
    pub fn move_folder(&mut self, id: FolderId, new_parent: Option<FolderId>) -> Result<()> {
        let now = self.now();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_live_folder(&tx, id)?;
        let folder = load_folder(&tx, id)?;
        if folder.parent_id == new_parent {
            return Ok(());
        }
        if let Some(target) = new_parent {
            require_live_folder(&tx, target)?;
            // Walk up from the target; meeting `id` means the target is inside it.
            let cycle: Option<i64> = tx
                .query_row(
                    "WITH RECURSIVE up(id, depth) AS (
                         SELECT ?1, 0
                         UNION ALL
                         SELECT f.parent_id, up.depth + 1 FROM folders f JOIN up ON f.id = up.id
                         WHERE f.parent_id IS NOT NULL AND up.depth < 100000
                     )
                     SELECT 1 FROM up WHERE id = ?2 LIMIT 1",
                    params![target, id],
                    |r| r.get(0),
                )
                .optional()?;
            if cycle.is_some() {
                return Err(StoreError::FolderCycle);
            }
        }
        let position = next_position(&tx, Siblings::Folders, new_parent)?;
        tx.execute(
            "UPDATE folders SET parent_id = ?2, position = ?3, updated_at = ?4 WHERE id = ?1",
            params![id, new_parent, position, now],
        )?;
        let old_siblings = sibling_ids(&tx, Siblings::Folders, folder.parent_id)?;
        write_positions(&tx, Siblings::Folders, &old_siblings)?;
        tx.commit()?;
        Ok(())
    }

    /// Moves a live folder to `index` among its live sibling folders (0 = first; an index
    /// past the end places it last). Does not bump `updated_at`.
    pub fn reorder_folder(&mut self, id: FolderId, index: usize) -> Result<()> {
        let tx = self.conn.transaction()?;
        require_live_folder(&tx, id)?;
        let folder = load_folder(&tx, id)?;
        place_at(&tx, Siblings::Folders, folder.parent_id, id.0, index)?;
        tx.commit()?;
        Ok(())
    }
}
