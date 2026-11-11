//! Documents: opaque JSON content plus derived metadata.

use rusqlite::{Connection, OptionalExtension, Row, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

use crate::Store;
use crate::error::{ItemKind, Result, StoreError, not_found};
use crate::folders::{Siblings, next_position, place_at, require_live_folder, sibling_ids};
use crate::ids::{DocumentId, FolderId, count_from_sql, count_to_sql};

/// Document metadata: everything except content and view state. Cheap to list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentMeta {
    /// The document's id.
    pub id: DocumentId,
    /// Containing folder, or `None` at the root. For a trashed document this is where it
    /// was (`None` if that folder has since been purged).
    pub folder_id: Option<FolderId>,
    /// Title as last saved or renamed.
    pub title: String,
    /// Word count supplied with the last save.
    pub word_count: u64,
    /// Character count supplied with the last save.
    pub char_count: u64,
    /// Sort key among sibling documents (ascending). Not necessarily contiguous.
    pub position: i64,
    /// Creation time, unix ms.
    pub created_at: i64,
    /// Last content save or rename, unix ms. Moves, reorders and view-state saves do not
    /// change it.
    pub updated_at: i64,
    /// When the document went to the trash, or `None` when live.
    pub trashed_at: Option<i64>,
}

/// A fully loaded document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Document {
    /// Metadata.
    pub meta: DocumentMeta,
    /// The opaque content JSON as last saved.
    pub content: String,
    /// The opaque view-state JSON, or `None` if never saved.
    pub view_state: Option<String>,
}

/// The content of a save: the opaque content JSON and the fields derived from it by the
/// caller. The store trusts these values; it does not parse `content`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct DocumentContent<'a> {
    /// Opaque content JSON.
    pub content: &'a str,
    /// Display title (may be empty).
    pub title: &'a str,
    /// Plain text of the document, indexed for search.
    pub plain_text: &'a str,
    /// Word count.
    pub word_count: u64,
    /// Character count.
    pub char_count: u64,
}

pub(crate) const DOC_META_COLS: &str =
    "id, folder_id, title, word_count, char_count, position, created_at, updated_at, trashed_at";

pub(crate) fn meta_from_row(row: &Row<'_>) -> rusqlite::Result<DocumentMeta> {
    Ok(DocumentMeta {
        id: row.get(0)?,
        folder_id: row.get(1)?,
        title: row.get(2)?,
        word_count: count_from_sql(row.get(3)?),
        char_count: count_from_sql(row.get(4)?),
        position: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
        trashed_at: row.get(8)?,
    })
}

pub(crate) fn load_meta(conn: &Connection, id: DocumentId) -> Result<DocumentMeta> {
    conn.query_row(
        &format!("SELECT {DOC_META_COLS} FROM documents WHERE id = ?1"),
        [id],
        meta_from_row,
    )
    .map_err(not_found(ItemKind::Document, id.0))
}

pub(crate) fn require_live_document(conn: &Connection, id: DocumentId) -> Result<DocumentMeta> {
    let meta = load_meta(conn, id)?;
    if meta.trashed_at.is_some() {
        return Err(StoreError::NotFound {
            kind: ItemKind::Document,
            id: id.0,
        });
    }
    Ok(meta)
}

impl Store {
    /// Creates a document in `folder` (or at the root), after its existing sibling
    /// documents. `NotFound` when the folder is missing or trashed.
    pub fn create_document(
        &mut self,
        folder: Option<FolderId>,
        content: &DocumentContent<'_>,
    ) -> Result<DocumentMeta> {
        let now = self.now();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(folder) = folder {
            require_live_folder(&tx, folder)?;
        }
        let position = next_position(&tx, Siblings::Documents, folder)?;
        tx.execute(
            "INSERT INTO documents
                 (folder_id, title, content, plain_text, word_count, char_count, position,
                  created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
            params![
                folder,
                content.title,
                content.content,
                content.plain_text,
                count_to_sql(content.word_count),
                count_to_sql(content.char_count),
                position,
                now
            ],
        )?;
        let meta = load_meta(&tx, DocumentId(tx.last_insert_rowid()))?;
        tx.commit()?;
        Ok(meta)
    }

    /// Loads a document with content and view state. Works for trashed documents too
    /// (check `meta.trashed_at`).
    pub fn document(&self, id: DocumentId) -> Result<Document> {
        self.conn
            .query_row(
                &format!(
                    "SELECT {DOC_META_COLS}, content, view_state FROM documents WHERE id = ?1"
                ),
                [id],
                |row| {
                    Ok(Document {
                        meta: meta_from_row(row)?,
                        content: row.get(9)?,
                        view_state: row.get(10)?,
                    })
                },
            )
            .map_err(not_found(ItemKind::Document, id.0))
    }

    /// Loads a document's metadata only (live or trashed).
    pub fn document_meta(&self, id: DocumentId) -> Result<DocumentMeta> {
        load_meta(&self.conn, id)
    }

    /// Saves content, title, plain text and counts in one transaction and bumps
    /// `updated_at`; the search index follows automatically. Returns the new metadata.
    ///
    /// Deliberately also works on a trashed document, so an autosave racing a delete never
    /// loses text.
    pub fn save_document(
        &mut self,
        id: DocumentId,
        content: &DocumentContent<'_>,
    ) -> Result<DocumentMeta> {
        let now = self.now();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = tx.execute(
            "UPDATE documents SET content = ?2, title = ?3, plain_text = ?4, word_count = ?5,
                 char_count = ?6, updated_at = ?7
             WHERE id = ?1",
            params![
                id,
                content.content,
                content.title,
                content.plain_text,
                count_to_sql(content.word_count),
                count_to_sql(content.char_count),
                now
            ],
        )?;
        if changed == 0 {
            return Err(StoreError::NotFound {
                kind: ItemKind::Document,
                id: id.0,
            });
        }
        let meta = load_meta(&tx, id)?;
        tx.commit()?;
        Ok(meta)
    }

    /// Changes a live document's title and bumps `updated_at`. The content is untouched,
    /// so a caller that derives the title from content should save content instead.
    pub fn rename_document(&mut self, id: DocumentId, title: &str) -> Result<()> {
        let now = self.now();
        let tx = self.conn.transaction()?;
        require_live_document(&tx, id)?;
        tx.execute(
            "UPDATE documents SET title = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, title, now],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Saves the opaque view-state JSON without touching `updated_at`. Works on trashed
    /// documents too.
    pub fn save_view_state(&mut self, id: DocumentId, view_state: &str) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE documents SET view_state = ?2 WHERE id = ?1",
            params![id, view_state],
        )?;
        if changed == 0 {
            return Err(StoreError::NotFound {
                kind: ItemKind::Document,
                id: id.0,
            });
        }
        Ok(())
    }

    /// The saved view-state JSON, `None` when never saved.
    pub fn view_state(&self, id: DocumentId) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT view_state FROM documents WHERE id = ?1",
                [id],
                |r| r.get(0),
            )
            .map_err(not_found(ItemKind::Document, id.0))
    }

    /// Moves a live document into `folder` (or the root), after the documents there.
    /// Moving to the current folder does nothing. Does not bump `updated_at`.
    pub fn move_document(&mut self, id: DocumentId, folder: Option<FolderId>) -> Result<()> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let meta = require_live_document(&tx, id)?;
        if meta.folder_id == folder {
            return Ok(());
        }
        if let Some(folder) = folder {
            require_live_folder(&tx, folder)?;
        }
        let position = next_position(&tx, Siblings::Documents, folder)?;
        tx.execute(
            "UPDATE documents SET folder_id = ?2, position = ?3 WHERE id = ?1",
            params![id, folder, position],
        )?;
        let old = sibling_ids(&tx, Siblings::Documents, meta.folder_id)?;
        crate::folders::write_positions(&tx, Siblings::Documents, &old)?;
        tx.commit()?;
        Ok(())
    }

    /// Moves a live document to `index` among its live sibling documents (clamped). Does not
    /// bump `updated_at`.
    pub fn reorder_document(&mut self, id: DocumentId, index: usize) -> Result<()> {
        let tx = self.conn.transaction()?;
        let meta = require_live_document(&tx, id)?;
        place_at(&tx, Siblings::Documents, meta.folder_id, id.0, index)?;
        tx.commit()?;
        Ok(())
    }

    /// Copies a live document (content, plain text, counts, view state) into the same
    /// folder, directly after the original, titled `"<title> copy"` (or `"Untitled copy"`).
    /// Snapshots are not copied. Returns the copy's metadata.
    pub fn duplicate_document(&mut self, id: DocumentId) -> Result<DocumentMeta> {
        let now = self.now();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let meta = require_live_document(&tx, id)?;
        let title = if meta.title.trim().is_empty() {
            "Untitled copy".to_owned()
        } else {
            format!("{} copy", meta.title)
        };
        tx.execute(
            "INSERT INTO documents
                 (folder_id, title, content, plain_text, word_count, char_count, position,
                  created_at, updated_at, view_state)
             SELECT folder_id, ?2, content, plain_text, word_count, char_count, position,
                    ?3, ?3, view_state
             FROM documents WHERE id = ?1",
            params![id, title, now],
        )?;
        let copy = DocumentId(tx.last_insert_rowid());
        let mut ids = sibling_ids(&tx, Siblings::Documents, meta.folder_id)?;
        ids.retain(|&other| other != copy.0);
        let at = ids.iter().position(|&other| other == id.0).map_or(ids.len(), |i| i + 1);
        ids.insert(at, copy.0);
        crate::folders::write_positions(&tx, Siblings::Documents, &ids)?;
        let copy = load_meta(&tx, copy)?;
        tx.commit()?;
        Ok(copy)
    }

    /// All live documents, most recently edited first.
    pub fn documents(&self) -> Result<Vec<DocumentMeta>> {
        self.query_metas(
            &format!(
                "SELECT {DOC_META_COLS} FROM documents WHERE trashed_at IS NULL
                 ORDER BY updated_at DESC, id DESC"
            ),
            [],
        )
    }

    /// Live documents directly in `folder` (or at the root), in display order.
    pub fn documents_in_folder(&self, folder: Option<FolderId>) -> Result<Vec<DocumentMeta>> {
        self.query_metas(
            &format!(
                "SELECT {DOC_META_COLS} FROM documents WHERE folder_id IS ?1
                 AND trashed_at IS NULL ORDER BY position, id"
            ),
            [folder],
        )
    }

    /// The `limit` most recently edited live documents.
    pub fn recent_documents(&self, limit: usize) -> Result<Vec<DocumentMeta>> {
        self.query_metas(
            &format!(
                "SELECT {DOC_META_COLS} FROM documents WHERE trashed_at IS NULL
                 ORDER BY updated_at DESC, id DESC LIMIT ?1"
            ),
            [i64::try_from(limit).unwrap_or(i64::MAX)],
        )
    }

    /// Whether a document exists and is not trashed.
    pub fn document_exists(&self, id: DocumentId) -> Result<bool> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM documents WHERE id = ?1 AND trashed_at IS NULL",
                [id],
                |r| r.get::<_, i64>(0),
            )
            .optional()?
            .is_some())
    }

    pub(crate) fn query_metas<P: rusqlite::Params>(
        &self,
        sql: &str,
        params: P,
    ) -> Result<Vec<DocumentMeta>> {
        let mut stmt = self.conn.prepare_cached(sql)?;
        let rows = stmt
            .query_map(params, meta_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }
}
