//! SQLite-backed library storage for The Writer.
//!
//! A [`Store`] owns one SQLite connection to the user's library (`library.db`, see
//! [`database_path`]) and persists:
//!
//! - **Folders** ([`Folder`]), nested to any depth, manually ordered among siblings.
//! - **Documents** ([`DocumentMeta`], [`Document`]). Content is an opaque JSON string supplied
//!   by the caller together with derived fields (title, plain text, word and character
//!   counts) in a [`DocumentContent`]. This crate never parses content. A separate opaque
//!   view-state JSON (caret, scroll, toggles) is stored without touching `updated_at`.
//! - **The library tree** ([`LibraryTree`]) for the sidebar, in one call.
//! - **Trash** ([`TrashEntry`]): deletes are soft; a folder takes its subtree with it.
//! - **Version snapshots** ([`SnapshotMeta`], [`Snapshot`]) with a pruning policy
//!   ([`SnapshotPolicy`]).
//! - **Settings**: a typed key/value table through serde.
//! - **Full-text search** ([`SearchHit`]) over title and plain text with FTS5, kept in sync
//!   by triggers, tolerant of any user input, with prefix matching on the last word.
//! - **Daily backups** written on open into a `backups` directory next to the database.
//!
//! Ordering: folders are ordered among sibling folders and documents among sibling
//! documents; the tree lists a folder's subfolders first, then its documents.
//!
//! Timestamps are unix milliseconds read from the store's [`Clock`]; tests use
//! [`Clock::manual`] to control time.
//!
//! The store is `Send` (not `Sync`): move it to a background thread or wrap it in a
//! `Mutex`. Reads take `&self`, writes take `&mut self` and run in a transaction.
//!
//! ```
//! use writer_store::{DocumentContent, Store};
//! let mut store = Store::open_in_memory().unwrap();
//! let drafts = store.create_folder(None, "Drafts").unwrap();
//! let doc = store
//!     .create_document(Some(drafts.id), &DocumentContent {
//!         content: r#"{"blocks":[]}"#,
//!         title: "Chapter one",
//!         plain_text: "It was a dark and stormy night.",
//!         word_count: 7,
//!         char_count: 31,
//!     })
//!     .unwrap();
//! let hits = store.search("storm", 10).unwrap();
//! assert_eq!(hits[0].document_id, doc.id);
//! ```

#![warn(missing_docs)]

mod backup;
mod clock;
mod documents;
mod error;
mod folders;
mod ids;
mod paths;
mod schema;
mod search;
mod settings;
mod snapshots;
mod trash;
mod tree;

use std::path::{Path, PathBuf};

use rusqlite::Connection;

pub use crate::backup::{DEFAULT_KEEP_BACKUPS, list_backups};
pub use crate::clock::Clock;
pub use crate::documents::{Document, DocumentContent, DocumentMeta};
pub use crate::error::{ItemKind, Result, StoreError};
pub use crate::folders::Folder;
pub use crate::ids::{DocumentId, FolderId, SnapshotId};
pub use crate::paths::{
    BACKUPS_DIR_NAME, DATA_DIR_ENV, DATABASE_FILE_NAME, backups_dir, data_dir, database_path,
};
pub use crate::schema::SCHEMA_VERSION;
pub use crate::search::{MATCH_END, MATCH_START, SearchHit, SnippetPart};
pub use crate::snapshots::{Snapshot, SnapshotMeta, SnapshotPolicy};
pub use crate::trash::{EmptiedTrash, TrashEntry};
pub use crate::tree::{FolderNode, LibraryTree};

/// Options for [`Store::open_with`].
#[derive(Clone, Debug)]
pub struct OpenOptions {
    /// Time source for every timestamp. Default: the system clock.
    pub clock: Clock,
    /// Write a backup on open when the newest one is at least a day old. Default: true.
    pub daily_backup: bool,
    /// How many backups to keep; older ones are deleted after a new one is written.
    /// Default: [`DEFAULT_KEEP_BACKUPS`].
    pub keep_backups: usize,
}

impl Default for OpenOptions {
    fn default() -> Self {
        Self {
            clock: Clock::system(),
            daily_backup: true,
            keep_backups: DEFAULT_KEEP_BACKUPS,
        }
    }
}

/// A connection to the library database. See the [crate docs](crate).
pub struct Store {
    conn: Connection,
    clock: Clock,
    path: Option<PathBuf>,
    snapshot_policy: SnapshotPolicy,
    backup_outcome: Option<std::result::Result<PathBuf, String>>,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store")
            .field("path", &self.path)
            .field("clock", &self.clock)
            .finish_non_exhaustive()
    }
}

impl Store {
    /// Opens (creating when missing) the library at `path` with default [`OpenOptions`].
    ///
    /// Creates parent directories, applies the pragmas (WAL, foreign keys, busy timeout,
    /// `synchronous=FULL`), writes the daily backup when due, and migrates the schema.
    /// Fails with [`StoreError::SchemaTooNew`] for a database from a newer app version and
    /// [`StoreError::NotALibrary`] for an unrelated SQLite file; neither is modified.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with(path, OpenOptions::default())
    }

    /// Opens the library at [`database_path`] (honouring `THE_WRITER_DATA_DIR`).
    pub fn open_default() -> Result<Self> {
        Self::open(database_path()?)
    }

    /// Like [`Store::open`] with explicit options.
    pub fn open_with(path: impl AsRef<Path>, options: OpenOptions) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let mut conn = Connection::open(&path)?;
        schema::configure(&conn, true)?;
        let header = schema::inspect(&conn)?;
        let mut backup_outcome = None;
        if options.daily_backup && header.user_version > 0 {
            backup_outcome = backup::daily_backup(
                &conn,
                &backups_dir(&path),
                options.clock.now_ms(),
                options.keep_backups,
            )
            .map_err(|e| e.to_string())
            .transpose();
        }
        schema::migrate(&mut conn)?;
        Ok(Self {
            conn,
            clock: options.clock,
            path: Some(path),
            snapshot_policy: SnapshotPolicy::default(),
            backup_outcome,
        })
    }

    /// Opens a fresh in-memory library (for tests) with the system clock.
    pub fn open_in_memory() -> Result<Self> {
        Self::open_in_memory_with_clock(Clock::system())
    }

    /// Opens a fresh in-memory library with the given clock.
    pub fn open_in_memory_with_clock(clock: Clock) -> Result<Self> {
        let mut conn = Connection::open_in_memory()?;
        schema::configure(&conn, false)?;
        schema::migrate(&mut conn)?;
        Ok(Self {
            conn,
            clock,
            path: None,
            snapshot_policy: SnapshotPolicy::default(),
            backup_outcome: None,
        })
    }

    /// The database file path, or `None` for an in-memory store.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// The clock the store stamps time with.
    pub fn clock(&self) -> &Clock {
        &self.clock
    }

    /// Replaces the clock.
    pub fn set_clock(&mut self, clock: Clock) {
        self.clock = clock;
    }

    /// The schema version stored in the open database.
    pub fn schema_version(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))?)
    }

    /// What the backup on open did: `None` when no backup was attempted or due (new
    /// database, backups disabled, or the newest backup is less than a day old),
    /// `Some(Ok(path))` when one was written, `Some(Err(message))` when writing it failed.
    /// A failed backup never prevents opening.
    pub fn backup_on_open(&self) -> Option<&std::result::Result<PathBuf, String>> {
        self.backup_outcome.as_ref()
    }

    /// Writes a consistent copy of the whole database to `dest` (replacing it). The copy is
    /// written to a temporary file next to `dest` and renamed into place.
    pub fn backup_to(&self, dest: impl AsRef<Path>) -> Result<()> {
        backup::write_backup(&self.conn, dest.as_ref())
    }

    /// The snapshot pruning policy used by [`Store::snapshot_if_stale`].
    pub fn snapshot_policy(&self) -> &SnapshotPolicy {
        &self.snapshot_policy
    }

    /// Replaces the snapshot pruning policy used by [`Store::snapshot_if_stale`].
    pub fn set_snapshot_policy(&mut self, policy: SnapshotPolicy) {
        self.snapshot_policy = policy;
    }

    /// Checkpoints the WAL into the main file and closes the connection, reporting errors
    /// that a plain drop would swallow. Dropping without calling this is safe too.
    pub fn close(self) -> Result<()> {
        if self.path.is_some() {
            self.conn
                .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
        }
        self.conn.close().map_err(|(_, e)| StoreError::Sqlite(e))
    }

    pub(crate) fn now(&self) -> i64 {
        self.clock.now_ms()
    }
}

#[cfg(test)]
mod send_check {
    fn assert_send<T: Send>() {}

    #[test]
    fn store_is_send() {
        assert_send::<super::Store>();
        assert_send::<super::StoreError>();
    }
}
