//! SQLite-backed library storage for The Writer.
//!
//! A [`Store`] owns one SQLite connection to the user's library (`library.db`, see
//! [`database_path`]) and persists:
//!
//! - **Folders** ([`Folder`]), nested to any depth, manually ordered among siblings.
//! - **Documents** ([`DocumentMeta`], [`Document`]). Content is an opaque JSON string supplied
//!   by the caller together with derived fields (title, plain text, word and character
//!   counts) in a [`DocumentContent`]. This crate never parses content, and reads one
//!   thing in it: the token an image block names its bytes with, `"image":"<32 hex
//!   digits>"` ([`Store::referenced_images`]). A separate opaque
//!   view-state JSON (caret, scroll, toggles) is stored without touching `updated_at`.
//! - **The library tree** ([`LibraryTree`]) for the sidebar, in one call.
//! - **Trash** ([`TrashEntry`]): deletes are soft; a folder takes its subtree with it.
//! - **Version snapshots** ([`SnapshotMeta`], [`Snapshot`]) with a pruning policy
//!   ([`SnapshotPolicy`]).
//! - **Settings**: a typed key/value table through serde.
//! - **Full-text search** ([`SearchHit`]) over title and plain text with FTS5, kept in sync
//!   by triggers, tolerant of any user input, with prefix matching on the last word.
//! - **Daily backups** written on open into a `backups` directory next to the database.
//! - **Images** ([`ImageId`], [`ImageInfo`], [`NewImage`]): the bytes of the pictures that
//!   documents hold, stored once each under the hash of their bytes, in a **second file**
//!   beside the database (`images.db`, see [`images_path`]; it is there only once the
//!   library holds an image, and [`ImageFile`] is that file by itself, for a thread of
//!   its own). The bytes are stored before a document names them
//!   ([`Store::put_image`]), and nothing deletes them but the collector
//!   ([`collect_unused_images`], [`mark_unused_images`]): an image goes only after no
//!   document, live or trashed, and no version has named it for 30 days, and no kept
//!   backup can name it.
//!
//! What a backup holds: a daily backup (and [`Store::backup_to`]) is a copy of the library
//! database, so it holds **the documents only**: folders, documents, versions, settings.
//! The pictures are not copied every day and kept seven times. They have **one copy**,
//! `images.db` in the backups directory ([`images_backup_path`]), which
//! [`back_up_images`] brings up to date in the background (only pictures the copy does not
//! have yet are copied) and from which a picture that `images.db` has lost is read and
//! written back ([`Store::image_bytes`]). A picture leaves the copy only when the
//! collector deleted it and no kept backup names it, so a backup that is brought back
//! finds its pictures in `images.db` or in the copy.
//!
//! A damaged file of the two (its header or a page is not what SQLite wrote) is moved
//! aside (`images.db.damaged-<unix ms>`, [`images_moved_aside`] lists them) and made
//! again, with what can still be read of it and the other file does not have; the rest
//! comes from the other file. Bytes whose hash is not their id are not believed in
//! either file, and are written over with the right ones where those are found.
//!
//! The schema version of the database ([`SCHEMA_VERSION`]) is 3 since documents may hold
//! images: the step changed nothing in the file, and makes a version of the app or of the
//! tool that does not know images refuse the library ([`StoreError::SchemaTooNew`])
//! instead of reading an image as an empty paragraph and saving that.
//!
//! A tool that only reads opens the library with [`Store::open_read_only`]: nothing is
//! created, migrated, backed up or locked, so a running app is not disturbed. Snapshots a
//! tool takes before it changes a document carry a label ([`Store::create_labeled_snapshot`],
//! [`Store::snapshot_labels`]).
//!
//! Next to the database lives the lock of the data directory ([`instance_lock`]): whoever
//! is going to write to a library (the app for as long as it runs, a tool for one write)
//! takes it first, so two writers never have one library.
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
mod images;
pub mod instance_lock;
mod paths;
mod schema;
mod search;
mod settings;
mod snapshots;
mod trash;
mod tree;

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};

pub use crate::backup::{DEFAULT_KEEP_BACKUPS, list_backups};
pub use crate::clock::Clock;
pub use crate::documents::{Document, DocumentContent, DocumentMeta};
pub use crate::error::{ItemKind, Result, StoreError};
pub use crate::folders::Folder;
pub use crate::ids::{DocumentId, FolderId, SnapshotId};
pub use crate::images::{
    IMAGE_BACKUP_SLICE_BYTES, IMAGE_RETENTION_MS, ImageFile, ImageFormat, ImageId, ImageInfo,
    ImagesBackedUp, ImagesCollected, NewImage, back_up_all_images, back_up_images,
    collect_unused_images, images_moved_aside, mark_unused_images,
};
pub use crate::paths::{
    BACKUPS_DIR_NAME, DATA_DIR_ENV, DATABASE_FILE_NAME, IMAGES_FILE_NAME, backups_dir, data_dir,
    database_path, images_backup_path, images_path,
};
pub use crate::schema::SCHEMA_VERSION;
pub use crate::search::{MATCH_END, MATCH_START, SearchHit, SnippetPart};
pub use crate::snapshots::{SNAPSHOT_LABEL_PREFIX, Snapshot, SnapshotMeta, SnapshotPolicy};
pub use crate::trash::{EmptiedTrash, TrashEntry};
pub use crate::tree::{FolderNode, LibraryTree};

/// Options for [`Store::open_with`].
#[derive(Clone, Debug)]
pub struct OpenOptions {
    /// Time source for every timestamp. Default: the system clock.
    pub clock: Clock,
    /// Write a backup on open when the newest one is at least a day old. Default: true.
    /// The backup is a copy of the library database: it holds the documents and not the
    /// bytes of the images (see the [crate docs](crate), "What a backup holds").
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
    read_only: bool,
    /// `images.db`, opened at the first use that finds something to open: a library
    /// without images never gets the file, and a store that only saves documents never
    /// opens it.
    images: ImageFile,
}

/// What [`Store::open_read_only`] found at the path.
#[derive(Debug)]
pub enum ReadOnlyLibrary {
    /// The library, open for reading. Everything that takes `&mut Store` fails on it
    /// with an SQLite "read-only database" error.
    Open(Box<Store>),
    /// The library was written by an older version of the app and has to be upgraded
    /// before this build can read it. Opening it with [`Store::open`] does that, and
    /// writes to the file: do it only while holding the [`instance_lock`].
    NeedsUpgrade {
        /// The schema version found in the file.
        found: i64,
        /// The schema version this build reads ([`SCHEMA_VERSION`]).
        supported: i64,
    },
}

impl ReadOnlyLibrary {
    /// The store, when the library could be opened as it is.
    pub fn store(self) -> Option<Store> {
        match self {
            ReadOnlyLibrary::Open(store) => Some(*store),
            ReadOnlyLibrary::NeedsUpgrade { .. } => None,
        }
    }
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
        schema::configure(&conn)?;
        let header = schema::inspect(&conn)?;
        schema::enable_wal(&conn)?;
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
            images: ImageFile::beside(&path),
            path: Some(path),
            snapshot_policy: SnapshotPolicy::default(),
            backup_outcome,
            read_only: false,
        })
    }

    /// Opens the library at `path` for reading only, as a tool does that must not disturb
    /// a running app: no directory or file is created, nothing is migrated, no backup is
    /// written and the journal mode is left as it is. The app (or another tool) may write
    /// to the library meanwhile; every read sees a consistent state.
    ///
    /// Fails when there is no file at `path` (an SQLite error), with
    /// [`StoreError::NotALibrary`] for an unrelated SQLite file and with
    /// [`StoreError::SchemaTooNew`] for a library of a newer version. A library of an
    /// older version is not an error but [`ReadOnlyLibrary::NeedsUpgrade`].
    ///
    /// The returned store stamps time with the system clock ([`Store::set_clock`]), though
    /// nothing a read-only store can do stamps anything.
    pub fn open_read_only(path: impl AsRef<Path>) -> Result<ReadOnlyLibrary> {
        let path = path.as_ref().to_path_buf();
        let conn = Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        schema::configure(&conn)?;
        let header = schema::inspect(&conn)?;
        if header.user_version < SCHEMA_VERSION {
            return Ok(ReadOnlyLibrary::NeedsUpgrade {
                found: header.user_version,
                supported: SCHEMA_VERSION,
            });
        }
        Ok(ReadOnlyLibrary::Open(Box::new(Self {
            conn,
            clock: Clock::system(),
            images: ImageFile::read_only(&path),
            path: Some(path),
            snapshot_policy: SnapshotPolicy::default(),
            backup_outcome: None,
            read_only: true,
        })))
    }

    /// Whether this store was opened with [`Store::open_read_only`].
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// Opens a fresh in-memory library (for tests) with the system clock.
    pub fn open_in_memory() -> Result<Self> {
        Self::open_in_memory_with_clock(Clock::system())
    }

    /// Opens a fresh in-memory library with the given clock.
    pub fn open_in_memory_with_clock(clock: Clock) -> Result<Self> {
        let mut conn = Connection::open_in_memory()?;
        schema::configure(&conn)?;
        schema::migrate(&mut conn)?;
        Ok(Self {
            conn,
            clock,
            images: ImageFile::in_memory(),
            path: None,
            snapshot_policy: SnapshotPolicy::default(),
            backup_outcome: None,
            read_only: false,
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
    ///
    /// The copy holds the documents only: folders, documents, versions, settings. The
    /// bytes of the images are in a second file ([`images_path`]) and are not in it; they
    /// have one copy of their own ([`back_up_images`]).
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

    /// Tells the connection to refuse every statement that would write (`true`), or to
    /// take them again (`false`). Whoever carries out the dry run of a change on a
    /// connection that can write switches this on for its duration, so that a mistake
    /// in the dry run fails instead of writing.
    ///
    /// It holds for the images too: while it is on no image is stored, the stamp of none
    /// is cleared, and `images.db` is not created.
    pub fn set_query_only(&self, on: bool) -> Result<()> {
        self.conn.pragma_update(None, "query_only", on)?;
        self.images.set_query_only(on)
    }

    /// Checkpoints the WAL into the main file and closes the connection, reporting errors
    /// that a plain drop would swallow. Dropping without calling this is safe too. A
    /// read-only store is closed without a checkpoint.
    ///
    /// The same is done for `images.db` when this store opened it.
    pub fn close(self) -> Result<()> {
        if self.path.is_some() && !self.read_only {
            self.conn
                .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
        }
        let images = self.images.close(true);
        self.conn.close().map_err(|(_, e)| StoreError::Sqlite(e))?;
        images
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
