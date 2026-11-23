//! Library items: opaque JSON content plus derived metadata.
//!
//! An item is a row of `documents` of one of three kinds ([`DocumentKind`]): a **text** (a
//! document of `innerword-core`, what the library held until schema 4), a **page** (an
//! HTML page kept whole: its content is the page's manifest, the JSON that names its files
//! in `images.db`, and its plain text is what the page showed) or a **whiteboard** (since
//! schema 5: its content is the stored scene of `innerword_board`, which names its
//! pictures in `images.db` `files` as a manifest names a page's files, and its plain text
//! is the text of its elements). The tree, folders, order, trash, versions and search
//! treat them alike. What keeps one from being read as another: whoever parses content
//! asks [`DocumentMeta::kind`] first, and each save refuses every kind but its own
//! ([`StoreError::WrongKind`]).

use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef};
use rusqlite::{Connection, OptionalExtension, Row, ToSql, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

use crate::images::{FileId, hash16, hex16, parse16};

use crate::Store;
use crate::error::{ItemKind, Result, StoreError, not_found};
use crate::folders::{Siblings, next_position, place_at, require_live_folder, sibling_ids};
use crate::ids::{DocumentId, FolderId, count_from_sql, count_to_sql};

/// What a library item is: the column `kind` of `documents` (schema 4).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentKind {
    /// A document of `innerword-core`: its content is the document's JSON. Every row written
    /// before schema 4 is one.
    #[default]
    Text,
    /// A page: an HTML page kept whole. Its content is the page's manifest (the JSON of
    /// `innerword_page::Manifest`), never a document of `innerword-core`.
    Page,
    /// A whiteboard (schema 5): its content is the stored scene
    /// (`innerword_board::Scene::to_stored`), never a document or a manifest.
    Board,
}

impl DocumentKind {
    /// The value of the column: `"text"`, `"page"` or `"board"`.
    pub fn as_str(self) -> &'static str {
        match self {
            DocumentKind::Text => "text",
            DocumentKind::Page => "page",
            DocumentKind::Board => "board",
        }
    }

    /// The kind a value of the column names; `None` for any other text.
    pub fn parse(text: &str) -> Option<DocumentKind> {
        match text {
            "text" => Some(DocumentKind::Text),
            "page" => Some(DocumentKind::Page),
            "board" => Some(DocumentKind::Board),
            _ => None,
        }
    }

    /// What an item of this kind is called in a reference to it: `"doc"` (`doc:12`),
    /// `"page"` or `"board"`. The three are read alike; this is the one that is written.
    pub fn reference_prefix(self) -> &'static str {
        match self {
            DocumentKind::Text => "doc",
            DocumentKind::Page => "page",
            DocumentKind::Board => "board",
        }
    }

    /// What an item of this kind is called in a sentence: `"document"`, `"page"` or
    /// `"whiteboard"`.
    pub fn noun(self) -> &'static str {
        match self {
            DocumentKind::Text => "document",
            DocumentKind::Page => "page",
            DocumentKind::Board => "whiteboard",
        }
    }
}

impl ToSql for DocumentKind {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.as_str()))
    }
}

impl FromSql for DocumentKind {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let text = value.as_str()?;
        DocumentKind::parse(text).ok_or_else(|| {
            FromSqlError::Other(format!("{text:?} is not a kind of library item").into())
        })
    }
}

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
    /// What the item is. `Text` for every row written before schema 4.
    #[serde(default)]
    pub kind: DocumentKind,
    /// The address a page came from; `None` for a text and for a page without one.
    #[serde(default)]
    pub source_url: Option<String>,
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

/// The content of a page: its manifest and the fields derived from it by the caller. The
/// store trusts these values and does not parse the manifest, except for the one token it
/// reads in it: the id of each file's bytes (`"file":"<32 hex digits>"`), which must be
/// stored before the page is ([`Store::put_file`]).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PageContent<'a> {
    /// The manifest as JSON (`innerword_page::Manifest`), stored in `content`.
    pub manifest: &'a str,
    /// Display title (may be empty).
    pub title: &'a str,
    /// The rendered text: what the page showed after its scripts ran. Indexed for search.
    pub plain_text: &'a str,
    /// Word count of the rendered text.
    pub word_count: u64,
    /// Character count of the rendered text.
    pub char_count: u64,
    /// The address the page came from, in the normal form the caller looks it up by;
    /// `None` when none was given.
    pub source_url: Option<&'a str>,
}

/// The content of a whiteboard: its stored scene (`innerword_board::Scene::to_stored`) and
/// what the caller derived from it. The store trusts these values and does not parse the
/// scene. **No title**: an autosave never writes the title, so it cannot put back a title
/// that a rename changed while the save was in flight; only [`Store::create_board`] and
/// [`Store::rename_document`] set it.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct BoardContent<'a> {
    /// The stored scene as JSON, stored in `content`.
    pub scene: &'a str,
    /// The files the scene names (`Scene::stored_files`): what the store checks. It does
    /// not scan the scene for them, so a `"file"` that only looks like the key (in an
    /// element's `customData`) can never refuse a save.
    pub files: &'a [FileId],
    /// The text of its elements. Indexed for search.
    pub plain_text: &'a str,
    /// Word count of that text.
    pub word_count: u64,
    /// Character count of that text.
    pub char_count: u64,
}

/// The first 16 bytes of the SHA-256 of a row's `content`, written as 32 lowercase hex
/// digits: what a compare-and-swap compares ([`Store::save_board`]). Two writes of the
/// same millisecond have different stamps where `updated_at` would be equal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ContentStamp([u8; 16]);

impl ContentStamp {
    /// The stamp of this content.
    pub fn of(content: &str) -> ContentStamp {
        ContentStamp(hash16(content.as_bytes()))
    }

    /// The stamp as 32 lowercase hex digits. [`std::fmt::Display`] writes the same.
    pub fn to_hex(&self) -> String {
        hex16(&self.0)
    }

    /// Reads exactly 32 hex digits, in either case. Anything else is `None`.
    pub fn parse(hex: &str) -> Option<ContentStamp> {
        parse16(hex.as_bytes()).map(ContentStamp)
    }
}

impl std::fmt::Display for ContentStamp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_hex())
    }
}

/// What a write of a whiteboard's content answers: the row as it is now and the stamp of
/// the content it holds.
#[derive(Clone, Debug, PartialEq)]
pub struct BoardSaved {
    /// The metadata after the write.
    pub meta: DocumentMeta,
    /// The stamp of the content written.
    pub stamp: ContentStamp,
}

/// The columns of a [`DocumentMeta`], in the order [`meta_from_row`] reads them. Every
/// one is in the covering index `documents_meta`, so listings never read the text.
pub(crate) const DOC_META_COLS: &str = "id, folder_id, title, word_count, char_count, position, \
     created_at, updated_at, trashed_at, kind, source_url";

/// How many columns [`DOC_META_COLS`] has: a query that selects more after them starts
/// there.
pub(crate) const DOC_META_COUNT: usize = 11;

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
        kind: row.get(9)?,
        source_url: row.get(10)?,
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

/// What the row `id` is; `NotFound` when there is none.
fn kind_of(conn: &Connection, id: DocumentId) -> Result<DocumentKind> {
    conn.query_row("SELECT kind FROM documents WHERE id = ?1", [id], |r| {
        r.get(0)
    })
    .optional()?
    .ok_or(StoreError::NotFound {
        kind: ItemKind::Document,
        id: id.0,
    })
}

/// `Ok` when the row `id` is of the kind `saving`; [`StoreError::WrongKind`] when it is of
/// another, `NotFound` when there is none.
fn require_kind(conn: &Connection, id: DocumentId, saving: DocumentKind) -> Result<()> {
    let kind = kind_of(conn, id)?;
    if kind != saving {
        return Err(StoreError::WrongKind {
            id: id.0,
            kind,
            saving,
        });
    }
    Ok(())
}

/// The stamp of the content the row `id` has now; `NotFound` when there is none.
fn stamp_of(conn: &Connection, id: DocumentId) -> Result<ContentStamp> {
    conn.query_row("SELECT content FROM documents WHERE id = ?1", [id], |r| {
        r.get_ref(0)?
            .as_str()
            .map(ContentStamp::of)
            .map_err(rusqlite::Error::from)
    })
    .map_err(not_found(ItemKind::Document, id.0))
}

/// What a new item of the library is made of, whatever its kind.
struct NewItem<'a> {
    kind: DocumentKind,
    title: &'a str,
    content: &'a str,
    plain_text: &'a str,
    word_count: u64,
    char_count: u64,
    source_url: Option<&'a str>,
}

/// Inserts an item at the end of the documents of `folder` (checked: live), inside the
/// caller's transaction.
fn insert_item(
    tx: &Connection,
    folder: Option<FolderId>,
    item: &NewItem<'_>,
    now: i64,
) -> Result<DocumentMeta> {
    if let Some(folder) = folder {
        require_live_folder(tx, folder)?;
    }
    let position = next_position(tx, Siblings::Documents, folder)?;
    tx.execute(
        "INSERT INTO documents
             (folder_id, title, content, plain_text, word_count, char_count, position,
              created_at, updated_at, kind, source_url)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, ?9, ?10)",
        params![
            folder,
            item.title,
            item.content,
            item.plain_text,
            count_to_sql(item.word_count),
            count_to_sql(item.char_count),
            position,
            now,
            item.kind,
            item.source_url,
        ],
    )?;
    load_meta(tx, DocumentId(tx.last_insert_rowid()))
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
                        content: row.get(DOC_META_COUNT)?,
                        view_state: row.get(DOC_META_COUNT + 1)?,
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
    ///
    /// Refuses a page and a whiteboard with [`StoreError::WrongKind`] and writes nothing:
    /// the content of a page is its manifest, that of a whiteboard its scene, and a text
    /// saved over either (the plain-text recovery of content that was read as a document)
    /// would lose it.
    pub fn save_document(
        &mut self,
        id: DocumentId,
        content: &DocumentContent<'_>,
    ) -> Result<DocumentMeta> {
        let now = self.now();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        // The search index follows `title` and `plain_text` through a trigger that fires
        // whenever a statement sets them, changed or not. A save that changes neither (new
        // formatting, a comment, a moved block of the same words) leaves them out, so the
        // text is not indexed again.
        let same_text: Option<(bool, DocumentKind)> = tx
            .query_row(
                "SELECT title = ?2 AND plain_text = ?3, kind FROM documents WHERE id = ?1",
                params![id, content.title, content.plain_text],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let same_text = match same_text {
            None => {
                return Err(StoreError::NotFound {
                    kind: ItemKind::Document,
                    id: id.0,
                });
            }
            Some((same, DocumentKind::Text)) => same,
            Some((_, kind)) => {
                return Err(StoreError::WrongKind {
                    id: id.0,
                    kind,
                    saving: DocumentKind::Text,
                });
            }
        };
        if same_text {
            tx.execute(
                "UPDATE documents SET content = ?2, word_count = ?3, char_count = ?4,
                     updated_at = ?5
                 WHERE id = ?1",
                params![
                    id,
                    content.content,
                    count_to_sql(content.word_count),
                    count_to_sql(content.char_count),
                    now
                ],
            )?;
        } else {
            tx.execute(
                "UPDATE documents SET content = ?2, title = ?3, plain_text = ?4,
                     word_count = ?5, char_count = ?6, updated_at = ?7
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
    ///
    /// A page is copied as a page (its kind, its manifest and its text), without its
    /// `source_url`: a copy is not "the page at that address", and a page that is added
    /// again from that address must find one page.
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
                  created_at, updated_at, view_state, kind)
             SELECT folder_id, ?2, content, plain_text, word_count, char_count, position,
                    ?3, ?3, view_state, kind
             FROM documents WHERE id = ?1",
            params![id, title, now],
        )?;
        let copy = DocumentId(tx.last_insert_rowid());
        let mut ids = sibling_ids(&tx, Siblings::Documents, meta.folder_id)?;
        ids.retain(|&other| other != copy.0);
        let at = ids
            .iter()
            .position(|&other| other == id.0)
            .map_or(ids.len(), |i| i + 1);
        ids.insert(at, copy.0);
        crate::folders::write_positions(&tx, Siblings::Documents, &ids)?;
        let copy = load_meta(&tx, copy)?;
        tx.commit()?;
        Ok(copy)
    }

    /// Creates a page in `folder` (or at the root), after its sibling documents, as
    /// [`Store::create_document`] creates a text. `NotFound` for a missing or trashed
    /// folder.
    ///
    /// **The store names no file it does not have**: every file id the manifest names
    /// (`"file":"<32 hex digits>"`) is looked up in `images.db` (a row read, no bytes), and
    /// when one is not there the page is refused with [`StoreError::FilesMissing`] and
    /// nothing is written. Each one that is there counts as used from now (the collector's
    /// stamp on it is cleared, as [`Store::touch_file`] does), so no run of the collector
    /// takes it between this look and the page.
    pub fn create_page(
        &mut self,
        folder: Option<FolderId>,
        page: &PageContent<'_>,
    ) -> Result<DocumentMeta> {
        let missing = self.files_missing(page.manifest)?;
        if !missing.is_empty() {
            return Err(StoreError::FilesMissing { ids: missing });
        }
        let now = self.now();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let meta = insert_item(
            &tx,
            folder,
            &NewItem {
                kind: DocumentKind::Page,
                title: page.title,
                content: page.manifest,
                plain_text: page.plain_text,
                word_count: page.word_count,
                char_count: page.char_count,
                source_url: page.source_url,
            },
            now,
        )?;
        tx.commit()?;
        Ok(meta)
    }

    /// New content of a page (manifest, text, counts, title, source address) in one
    /// transaction; bumps `updated_at`, the search index follows. Folder and position stay.
    /// Returns the new metadata.
    ///
    /// Refuses a text and a whiteboard with [`StoreError::WrongKind`] (`saving: Page`) and
    /// a manifest that names a file the library does not have with
    /// [`StoreError::FilesMissing`], as [`Store::create_page`]; neither writes anything.
    /// Works on a trashed page, as [`Store::save_document`] does.
    pub fn save_page(&mut self, id: DocumentId, page: &PageContent<'_>) -> Result<DocumentMeta> {
        require_kind(&self.conn, id, DocumentKind::Page)?;
        let missing = self.files_missing(page.manifest)?;
        if !missing.is_empty() {
            return Err(StoreError::FilesMissing { ids: missing });
        }
        let now = self.now();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        // As for a text: a save that changes neither the title nor the text leaves them
        // out, so the text is not indexed again.
        let same_text: Option<(bool, DocumentKind)> = tx
            .query_row(
                "SELECT title = ?2 AND plain_text = ?3, kind FROM documents WHERE id = ?1",
                params![id, page.title, page.plain_text],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let same_text = match same_text {
            // Deleted or made another kind since the look above (another connection).
            None => {
                return Err(StoreError::NotFound {
                    kind: ItemKind::Document,
                    id: id.0,
                });
            }
            Some((same, DocumentKind::Page)) => same,
            Some((_, kind)) => {
                return Err(StoreError::WrongKind {
                    id: id.0,
                    kind,
                    saving: DocumentKind::Page,
                });
            }
        };
        if same_text {
            tx.execute(
                "UPDATE documents SET content = ?2, word_count = ?3, char_count = ?4,
                     source_url = ?5, updated_at = ?6
                 WHERE id = ?1",
                params![
                    id,
                    page.manifest,
                    count_to_sql(page.word_count),
                    count_to_sql(page.char_count),
                    page.source_url,
                    now
                ],
            )?;
        } else {
            tx.execute(
                "UPDATE documents SET content = ?2, title = ?3, plain_text = ?4,
                     word_count = ?5, char_count = ?6, source_url = ?7, updated_at = ?8
                 WHERE id = ?1",
                params![
                    id,
                    page.manifest,
                    page.title,
                    page.plain_text,
                    count_to_sql(page.word_count),
                    count_to_sql(page.char_count),
                    page.source_url,
                    now
                ],
            )?;
        }
        let meta = load_meta(&tx, id)?;
        tx.commit()?;
        Ok(meta)
    }

    /// The live page with this address that was updated last; `None` when there is none.
    /// `url` is compared as it is given: the caller passes the normal form it stores
    /// addresses in. Answered from the index of the addresses.
    pub fn page_by_source_url(&self, url: &str) -> Result<Option<DocumentMeta>> {
        let mut found = self.query_metas(
            &format!(
                "SELECT {DOC_META_COLS} FROM documents
                 WHERE source_url = ?1 AND kind = 'page' AND trashed_at IS NULL
                 ORDER BY updated_at DESC, id DESC LIMIT 1"
            ),
            [url],
        )?;
        Ok(found.pop())
    }

    /// The trashed page with this address that was trashed last (on its own or with its
    /// folder); `None` when there is none. `url` is compared as given.
    pub fn trashed_page_by_source_url(&self, url: &str) -> Result<Option<DocumentMeta>> {
        let mut found = self.query_metas(
            &format!(
                "SELECT {DOC_META_COLS} FROM documents
                 WHERE source_url = ?1 AND kind = 'page' AND trashed_at IS NOT NULL
                 ORDER BY trashed_at DESC, id DESC LIMIT 1"
            ),
            [url],
        )?;
        Ok(found.pop())
    }

    /// Every page, live and trashed, most recently updated first (`trashed_at` says which
    /// is which). Answered from the covering index: no text is read.
    pub fn pages(&self) -> Result<Vec<DocumentMeta>> {
        self.query_metas(
            &format!(
                "SELECT {DOC_META_COLS} FROM documents INDEXED BY documents_meta
                 WHERE kind = 'page' ORDER BY updated_at DESC, id DESC"
            ),
            [],
        )
    }

    /// Creates a whiteboard in `folder` (or at the root), after its sibling documents, as
    /// [`Store::create_page`] creates a page, titled `title`. `NotFound` for a missing or
    /// trashed folder.
    ///
    /// **The library goes to schema 5 here**, lazily: when the file is at 4 (what every
    /// open leaves), the step is taken inside this transaction, before the row is
    /// inserted, so a failure leaves both undone. A library in which nobody made a
    /// whiteboard stays readable by a version that knows schema 4.
    ///
    /// **The store names no file it does not have**: every id of `board.files` is looked
    /// up in `images.db` (a row read, no bytes), and when one is not there the whiteboard
    /// is refused with [`StoreError::FilesMissing`] and nothing is written. Each one that
    /// is there counts as used from now ([`Store::touch_file`]).
    pub fn create_board(
        &mut self,
        folder: Option<FolderId>,
        title: &str,
        board: &BoardContent<'_>,
    ) -> Result<BoardSaved> {
        let missing = self.board_files_missing(None, board.files)?;
        if !missing.is_empty() {
            return Err(StoreError::FilesMissing { ids: missing });
        }
        let now = self.now();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        crate::schema::take_steps_above_open(&tx)?;
        let meta = insert_item(
            &tx,
            folder,
            &NewItem {
                kind: DocumentKind::Board,
                title,
                content: board.scene,
                plain_text: board.plain_text,
                word_count: board.word_count,
                char_count: board.char_count,
                source_url: None,
            },
            now,
        )?;
        tx.commit()?;
        Ok(BoardSaved {
            meta,
            stamp: ContentStamp::of(board.scene),
        })
    }

    /// New content of a whiteboard (scene, text, counts) in one transaction; bumps
    /// `updated_at`, the search index follows. **Never the title**, the folder or the
    /// position. Works on a trashed whiteboard, as [`Store::save_document`] does.
    ///
    /// Refused, with nothing written:
    /// - a text or a page: [`StoreError::WrongKind`] (`saving: Board`);
    /// - with `expect` given, a row whose content has another stamp (it changed since the
    ///   content the caller's scene descends from): [`StoreError::Stale`] with the stamp it
    ///   has, read in the same transaction as the write. `None` writes whatever is there
    ///   (agents, which read and write under one lock or one turn of the main thread);
    /// - an id of `board.files` that the library does not have and that neither the row's
    ///   content nor any version of it names: [`StoreError::FilesMissing`]. A missing id the
    ///   whiteboard named already is kept (its picture shows as missing and the whiteboard
    ///   goes on saving). Each id that is there counts as used from now.
    pub fn save_board(
        &mut self,
        id: DocumentId,
        board: &BoardContent<'_>,
        expect: Option<ContentStamp>,
    ) -> Result<BoardSaved> {
        // A look first, so that a save that would be refused anyway touches no file and a
        // stale save is told so before anything about its files.
        require_kind(&self.conn, id, DocumentKind::Board)?;
        if let Some(expect) = expect {
            let stored = stamp_of(&self.conn, id)?;
            if stored != expect {
                return Err(StoreError::Stale { id: id.0, stored });
            }
        }
        let missing = self.board_files_missing(Some(id), board.files)?;
        if !missing.is_empty() {
            return Err(StoreError::FilesMissing { ids: missing });
        }
        let now = self.now();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        // Again under the write lock: another connection may have written meanwhile.
        require_kind(&tx, id, DocumentKind::Board)?;
        if let Some(expect) = expect {
            let stored = stamp_of(&tx, id)?;
            if stored != expect {
                return Err(StoreError::Stale { id: id.0, stored });
            }
        }
        // As for a text: a save that does not change the text leaves it out, so it is not
        // indexed again.
        let same_text: bool = tx.query_row(
            "SELECT plain_text = ?2 FROM documents WHERE id = ?1",
            params![id, board.plain_text],
            |r| r.get(0),
        )?;
        if same_text {
            tx.execute(
                "UPDATE documents SET content = ?2, word_count = ?3, char_count = ?4,
                     updated_at = ?5
                 WHERE id = ?1",
                params![
                    id,
                    board.scene,
                    count_to_sql(board.word_count),
                    count_to_sql(board.char_count),
                    now
                ],
            )?;
        } else {
            tx.execute(
                "UPDATE documents SET content = ?2, plain_text = ?3, word_count = ?4,
                     char_count = ?5, updated_at = ?6
                 WHERE id = ?1",
                params![
                    id,
                    board.scene,
                    board.plain_text,
                    count_to_sql(board.word_count),
                    count_to_sql(board.char_count),
                    now
                ],
            )?;
        }
        let meta = load_meta(&tx, id)?;
        tx.commit()?;
        Ok(BoardSaved {
            meta,
            stamp: ContentStamp::of(board.scene),
        })
    }

    /// Every whiteboard, live and trashed, most recently updated first (`trashed_at` says
    /// which is which). Answered from the covering index: no scene is read.
    pub fn boards(&self) -> Result<Vec<DocumentMeta>> {
        self.query_metas(
            &format!(
                "SELECT {DOC_META_COLS} FROM documents INDEXED BY documents_meta
                 WHERE kind = 'board' ORDER BY updated_at DESC, id DESC"
            ),
            [],
        )
    }

    /// All live documents, most recently edited first.
    pub fn documents(&self) -> Result<Vec<DocumentMeta>> {
        self.query_metas(
            &format!(
                "SELECT {DOC_META_COLS} FROM documents INDEXED BY documents_meta
                 WHERE trashed_at IS NULL ORDER BY updated_at DESC, id DESC"
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
