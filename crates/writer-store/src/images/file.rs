//! `images.db` by itself: the file, its schema and pragmas, and one connection to it.

use std::cell::{Cell, Ref, RefCell};
use std::collections::HashSet;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, OptionalExtension, Row, TransactionBehavior, params};

use super::{ImageFormat, ImageId, ImageInfo, NewImage, copy};
use crate::clock::Clock;
use crate::error::{Result, StoreError};
use crate::paths::{images_backup_path, images_path};

/// `application_id` of an images database ("WIMG"): the library database has its own
/// ("WRIT"), so neither file is taken for the other.
pub(crate) const IMAGES_APPLICATION_ID: i32 = 0x5749_4D47;

/// The schema version of an images database, in its `user_version`.
pub(crate) const IMAGES_SCHEMA_VERSION: i64 = 1;

/// How long a connection waits for another one that is writing. The `Store` of the app
/// and a background [`ImageFile`] of the same process wait for each other.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// The one table. A rowid table, so that large rows go to overflow pages; the bytes come
/// last, so the columns before them are read without walking the blob.
const SCHEMA: &str = r#"
CREATE TABLE images (
    id                 TEXT    NOT NULL UNIQUE,  -- 32 lowercase hex digits
    format             TEXT    NOT NULL,         -- 'png' | 'jpeg' | 'gif'
    width              INTEGER NOT NULL,         -- pixels
    height             INTEGER NOT NULL,
    density            INTEGER NOT NULL,         -- pixels per inch, 72 when the file says nothing
    byte_len           INTEGER NOT NULL,
    created_at         INTEGER NOT NULL,         -- unix ms
    unreferenced_since INTEGER,                  -- set and cleared by the collector; cleared by put and touch
    bytes              BLOB    NOT NULL          -- last, so the columns before it are read without walking the blob
);
"#;

/// The columns of an [`ImageInfo`], in the order [`info_from_row`] reads them.
pub(crate) const INFO_COLS: &str = "id, format, width, height, density, byte_len, created_at";

/// Making the file (the pragmas that write its header, the table) is done by one
/// connection of this process at a time: two that find an empty file at the same moment
/// would both try to switch its journal mode, which SQLite does not let one wait for.
/// Other processes do not get here: only the holder of the data directory's lock writes.
static MAKING: Mutex<()> = Mutex::new(());

/// How a connection to an images database is opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    /// For reading. Nothing is created or changed.
    Read,
    /// For reading and writing a file that is there. Nothing is created.
    Write,
    /// For writing: the directory, the file and the table are made when they are missing.
    Create,
}

/// Opens the images database at `path`. `None` when there is nothing to read there and
/// nothing is to be made: no file, or a file without the table (one that was left behind
/// when the making of it was interrupted).
pub(crate) fn open(path: &Path, mode: Mode) -> Result<Option<Connection>> {
    if mode != Mode::Create && !path.exists() {
        return Ok(None);
    }
    if mode == Mode::Read {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.busy_timeout(BUSY_TIMEOUT)?;
        return Ok(is_made(&conn)?.then_some(conn));
    }
    let _making = making();
    open_making(path, mode)
}

/// The lock of [`MAKING`].
fn making() -> std::sync::MutexGuard<'static, ()> {
    MAKING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// [`open`] for writing, by whoever holds [`MAKING`].
fn open_making(path: &Path, mode: Mode) -> Result<Option<Connection>> {
    if mode == Mode::Write && !path.exists() {
        return Ok(None);
    }
    let mut flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    if mode == Mode::Create {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        flags |= OpenFlags::SQLITE_OPEN_CREATE;
    }
    let conn = Connection::open_with_flags(path, flags)?;
    conn.busy_timeout(BUSY_TIMEOUT)?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    if !is_made(&conn)? {
        if mode != Mode::Create {
            return Ok(None);
        }
        // Before the first table, or the file never shrinks after a collection.
        conn.pragma_update(None, "auto_vacuum", "INCREMENTAL")?;
    }
    let journal: String = conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))?;
    if !journal.eq_ignore_ascii_case("wal") {
        return Err(StoreError::JournalMode(journal));
    }
    // Every commit is on disk when it returns: a document may name the bytes right after.
    conn.pragma_update(None, "synchronous", "FULL")?;
    if mode == Mode::Create {
        make(&conn)?;
    }
    Ok(Some(conn))
}

/// A database in memory with the table, for a library in memory.
fn open_in_memory() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    make(&conn)?;
    Ok(conn)
}

/// Whether the file holds the table. An empty file does not; a file that is not an images
/// database, or one of a newer version, is an error.
fn is_made(conn: &Connection) -> Result<bool> {
    let application_id: i32 = conn.query_row("PRAGMA application_id", [], |r| r.get(0))?;
    let user_version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if user_version == 0 && application_id == 0 {
        let tables: i64 = conn.query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
            [],
            |r| r.get(0),
        )?;
        return if tables == 0 {
            Ok(false)
        } else {
            Err(StoreError::NotALibrary)
        };
    }
    if application_id != IMAGES_APPLICATION_ID || user_version < 1 {
        return Err(StoreError::NotALibrary);
    }
    if user_version > IMAGES_SCHEMA_VERSION {
        return Err(StoreError::SchemaTooNew {
            found: user_version,
            supported: IMAGES_SCHEMA_VERSION,
        });
    }
    Ok(true)
}

/// Creates the table in a file that does not have it yet, with the two header fields that
/// say what the file is, in one transaction.
fn make(conn: &Connection) -> Result<()> {
    let tx = rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let version: i64 = tx.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version == 0 {
        tx.execute_batch(SCHEMA)?;
        tx.pragma_update(None, "user_version", IMAGES_SCHEMA_VERSION)?;
        tx.pragma_update(None, "application_id", IMAGES_APPLICATION_ID)?;
    }
    tx.commit()?;
    Ok(())
}

/// The error SQLite gives a write on a database that is read-only, for the writes that
/// are refused before they reach it.
fn read_only_error() -> StoreError {
    StoreError::Sqlite(rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_READONLY),
        Some("attempt to write a readonly database".to_owned()),
    ))
}

/// One row as an [`ImageInfo`] (the columns of [`INFO_COLS`], starting at `first`).
pub(crate) fn info_from_row(row: &Row<'_>, first: usize) -> rusqlite::Result<ImageInfo> {
    fn unreadable(column: usize, what: &str) -> rusqlite::Error {
        rusqlite::Error::FromSqlConversionFailure(
            column,
            rusqlite::types::Type::Text,
            format!("not {what}").into(),
        )
    }
    fn count<T: TryFrom<i64>>(row: &Row<'_>, column: usize) -> rusqlite::Result<T> {
        let value: i64 = row.get(column)?;
        T::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(column, value))
    }
    let id = row.get_ref(first)?.as_str()?;
    let id = ImageId::parse(id).ok_or_else(|| unreadable(first, "the id of an image"))?;
    let format = row.get_ref(first + 1)?.as_str()?;
    let format = ImageFormat::from_column(format)
        .ok_or_else(|| unreadable(first + 1, "a format of stored images"))?;
    Ok(ImageInfo {
        id,
        format,
        width: count(row, first + 2)?,
        height: count(row, first + 3)?,
        density: count(row, first + 4)?,
        byte_len: count(row, first + 5)?,
        created_at: row.get(first + 6)?,
    })
}

/// What a connection knows about an image, and the collector's stamp on it.
pub(crate) fn read_info(
    conn: &Connection,
    id: &ImageId,
) -> Result<Option<(ImageInfo, Option<i64>)>> {
    Ok(conn
        .query_row(
            &format!("SELECT {INFO_COLS}, unreferenced_since FROM images WHERE id = ?1"),
            [id.to_hex()],
            |row| Ok((info_from_row(row, 0)?, row.get(7)?)),
        )
        .optional()?)
}

/// The bytes of an image as a connection has them.
pub(crate) fn read_bytes(conn: &Connection, id: &ImageId) -> Result<Option<Vec<u8>>> {
    Ok(conn
        .query_row(
            "SELECT bytes FROM images WHERE id = ?1",
            [id.to_hex()],
            |row| row.get(0),
        )
        .optional()?)
}

/// Writes a row unless one with this id is there. A row that is there loses the
/// collector's stamp, and gets these bytes when the ones it has are others (bytes that
/// went bad on the disk: the id names these); everything else of it stays. One
/// statement, so it is whole or not at all.
pub(crate) fn write_row(conn: &Connection, info: &ImageInfo, bytes: &[u8]) -> Result<()> {
    conn.execute(
        "INSERT INTO images
             (id, format, width, height, density, byte_len, created_at, unreferenced_since, bytes)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8)
         ON CONFLICT(id) DO UPDATE SET
             unreferenced_since = NULL, bytes = excluded.bytes, byte_len = excluded.byte_len
             WHERE unreferenced_since IS NOT NULL OR bytes IS NOT excluded.bytes",
        params![
            info.id.to_hex(),
            info.format.as_str(),
            info.width,
            info.height,
            info.density,
            i64::try_from(info.byte_len).unwrap_or(i64::MAX),
            info.created_at,
            bytes,
        ],
    )?;
    Ok(())
}

// ----- a damaged file ---------------------------------------------------------------------

/// Which file a path names at a moment: a file that is moved aside, deleted or replaced
/// under a connection is no longer the one at the path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FileIdentity {
    device: u64,
    inode: u64,
}

/// The file at `path` now; `None` when there is none.
pub(crate) fn file_identity(path: &Path) -> Option<FileIdentity> {
    std::fs::metadata(path).ok().map(|m| FileIdentity {
        device: m.dev(),
        inode: m.ino(),
    })
}

/// Whether an error says that the bytes of a database file are not what SQLite wrote:
/// "file is not a database" (the header is gone) or "database disk image is malformed"
/// (a page is). A file of another kind under the name, or of a newer version, is not
/// damaged: that one is refused and left alone.
pub(crate) fn is_damage(error: &StoreError) -> bool {
    matches!(error, StoreError::Sqlite(error) if is_sqlite_damage(error))
}

/// [`is_damage`] for an error of SQLite itself.
pub(crate) fn is_sqlite_damage(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(failure, _)
            if matches!(
                failure.code,
                rusqlite::ErrorCode::NotADatabase | rusqlite::ErrorCode::DatabaseCorrupt
            )
    )
}

/// Whether the images database at `path` is damaged: what the collector and the copy
/// read of it cannot be read, or SQLite's own quick check finds a page that is not
/// whole. For whoever got a damage error from a connection that may have been to
/// another file (the library database): asked before anything is moved aside. Reads the
/// whole file; for the rare moment after such an error.
pub(crate) fn is_damaged(path: &Path) -> bool {
    let probe = || -> Result<bool> {
        let Some(conn) = open(path, Mode::Read)? else {
            return Ok(false);
        };
        {
            let mut stmt = conn.prepare("SELECT id, unreferenced_since FROM images")?;
            let mut rows = stmt.query([])?;
            while rows.next()?.is_some() {}
        }
        let check: String = conn.query_row("PRAGMA quick_check(1)", [], |r| r.get(0))?;
        Ok(check != "ok")
    };
    match probe() {
        Ok(damaged) => damaged,
        Err(error) => is_damage(&error),
    }
}

/// The name a damaged file is given beside its own: `images.db.damaged-<unix ms>`.
pub(crate) const DAMAGED_MARK: &str = ".damaged-";

/// Moves the damaged images database at `path` aside (with the files of its journal, so
/// that what can still be read of it can be read) and makes a new one in its place, then
/// copies into the new one every image of the old one that can still be read, whose
/// bytes are the ones its id names, and that `other` (the other images database of the
/// library: the copy for `images.db`, `images.db` for the copy) does not have. The rest
/// is in `other` and comes back from there.
///
/// Only when the file at `path` is still `suspect` (the file that was found damaged):
/// another connection may have done all this first, and then nothing is done. Answers
/// where the damaged file is now.
///
/// Only the writer of the library calls it (the app, or the tool while it holds the
/// lock of the data directory), never a connection for reading or a dry run.
pub(crate) fn replace_damaged(
    path: &Path,
    other: &Path,
    suspect: Option<FileIdentity>,
) -> Result<Option<PathBuf>> {
    let (aside, conn) = {
        let _making = making();
        if suspect.is_none() || file_identity(path) != suspect {
            return Ok(None);
        }
        let aside = move_aside(path)?;
        let Some(conn) = open_making(path, Mode::Create)? else {
            unreachable!("a file that is created is opened");
        };
        (aside, conn)
    };
    salvage(&aside, &conn, other);
    Ok(Some(aside))
}

/// `path` with `-wal` or `-shm` after it: the files of SQLite's journal.
fn beside_it(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// Renames `path` to `<name>.damaged-<unix ms>` with its journal. A journal that cannot
/// be moved is removed: left behind, SQLite would play it into the new file.
fn move_aside(path: &Path) -> Result<PathBuf> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "images.db".to_owned());
    let now = Clock::system().now_ms();
    let mut aside = path.with_file_name(format!("{name}{DAMAGED_MARK}{now}"));
    let mut n = 1;
    while aside.exists() || beside_it(&aside, "-wal").exists() {
        aside = path.with_file_name(format!("{name}{DAMAGED_MARK}{now}-{n}"));
        n += 1;
    }
    std::fs::rename(path, &aside)?;
    for suffix in ["-wal", "-shm"] {
        let from = beside_it(path, suffix);
        if from.exists() && std::fs::rename(&from, beside_it(&aside, suffix)).is_err() {
            std::fs::remove_file(&from)?;
        }
    }
    Ok(aside)
}

/// What can be read of the damaged file at `damaged` and `other` does not have, into
/// `into`. Answers how many images were saved. Nothing that goes wrong here is an error:
/// what cannot be read is lost to this file and stays in the one moved aside.
fn salvage(damaged: &Path, into: &Connection, other: &Path) -> u64 {
    let Ok(old) = Connection::open_with_flags(
        damaged,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) else {
        return 0;
    };
    let _ = old.busy_timeout(BUSY_TIMEOUT);
    // A file that is shorter than its header says (cut off) is not read at all without
    // this; with it, the pages that are there are read. Nothing is written to it.
    let _ = old.pragma_update(None, "writable_schema", true);
    // Every row as far as it can be read, by the index of the ids and by the table: a
    // page of the one can be lost where the other is whole.
    let ids = |conn: &Connection| -> Result<Vec<(i64, String)>> {
        let mut found = std::collections::BTreeMap::new();
        for sql in [
            "SELECT rowid, id FROM images INDEXED BY sqlite_autoindex_images_1",
            "SELECT rowid, id FROM images NOT INDEXED",
        ] {
            let Ok(mut stmt) = conn.prepare(sql) else {
                continue;
            };
            let Ok(mut rows) = stmt.query([]) else {
                continue;
            };
            while let Ok(Some(row)) = rows.next() {
                if let (Ok(rowid), Ok(id)) = (row.get(0), row.get(1)) {
                    found.insert(rowid, id);
                }
            }
        }
        Ok(found.into_iter().collect())
    };
    let known: HashSet<String> = open(other, Mode::Read)
        .ok()
        .flatten()
        .and_then(|conn| ids(&conn).ok())
        .map(|rows| rows.into_iter().map(|(_, id)| id).collect())
        .unwrap_or_default();
    let Ok(rows) = ids(&old) else {
        return 0;
    };
    let mut saved = 0;
    for (rowid, hex) in rows {
        if known.contains(&hex) {
            continue;
        }
        let row = old.query_row(
            &format!("SELECT {INFO_COLS}, bytes FROM images WHERE rowid = ?1"),
            [rowid],
            |row| Ok((info_from_row(row, 0)?, row.get::<_, Vec<u8>>(7)?)),
        );
        let Ok((info, bytes)) = row else {
            continue;
        };
        if ImageId::of(&bytes) == info.id && write_row(into, &info, &bytes).is_ok() {
            saved += 1;
        }
    }
    saved
}

// ----- the file by itself -------------------------------------------------------------------

/// `images.db` by itself: one connection, for a thread of its own.
///
/// The image methods of a [`crate::Store`] go through the one it holds. Whoever reads or
/// writes pictures in the background (16 MB should never be hashed, written or read on
/// the main thread of an app) has another, on the same file: the two wait for each other.
/// A library in memory has no file a second connection could see; there everything goes
/// through the `Store`.
///
/// **A damaged file** (its header or a page is not what SQLite wrote) is no reason to
/// lose a picture or to refuse a new one. A read answers from the copy of the pictures
/// in the backups directory; bytes whose hash is not their id are not believed and are
/// taken from the copy too (and written over the bad ones). A connection that may write
/// moves the damaged file aside (`images.db.damaged-<unix ms>`, [`crate::images_moved_aside`]
/// lists them) and starts a new one with what can still be read of the old one and the
/// copy does not have; the rest comes back from the copy as it is asked for. Another
/// connection of the same file notices that the file at the path is no longer the one it
/// has open, and opens the new one.
///
/// `Send`, not `Sync`.
pub struct ImageFile {
    place: Place,
    read_only: bool,
    /// Whether writes are refused for the time being ([`crate::Store::set_query_only`]).
    query_only: Cell<bool>,
    /// Opened at the first use that finds something to open; let go when the file at the
    /// path is no longer the one it has open, or turned out damaged.
    conn: RefCell<Option<Kept>>,
}

/// A connection, and the file it was opened on.
struct Kept {
    conn: Connection,
    /// `None` in memory.
    file: Option<FileIdentity>,
}

enum Place {
    /// The file beside a library database, and where the copy of the pictures is kept.
    Disk { path: PathBuf, copy: PathBuf },
    /// A database in memory, for a library in memory.
    Memory,
}

impl std::fmt::Debug for ImageFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImageFile")
            .field("path", &self.path())
            .field("read_only", &self.read_only)
            .field("open", &self.conn.borrow().is_some())
            .finish()
    }
}

impl ImageFile {
    /// The `images.db` beside `database` (the path of a library database). Opens nothing
    /// yet. A read while there is no file answers `None` and does not remember that; the
    /// first [`put`](Self::put) makes the file.
    pub fn beside(database: &Path) -> ImageFile {
        ImageFile {
            place: Place::Disk {
                path: images_path(database),
                copy: images_backup_path(database),
            },
            read_only: false,
            query_only: Cell::new(false),
            conn: RefCell::new(None),
        }
    }

    /// Like [`beside`](Self::beside), for reading only: nothing is created or changed,
    /// and [`put`](Self::put) and [`touch`](Self::touch) fail.
    pub fn read_only(database: &Path) -> ImageFile {
        ImageFile {
            read_only: true,
            ..ImageFile::beside(database)
        }
    }

    /// The images of a library in memory.
    pub(crate) fn in_memory() -> ImageFile {
        ImageFile {
            place: Place::Memory,
            read_only: false,
            query_only: Cell::new(false),
            conn: RefCell::new(None),
        }
    }

    /// The file, or `None` for a library in memory.
    pub fn path(&self) -> Option<&Path> {
        match &self.place {
            Place::Disk { path, .. } => Some(path),
            Place::Memory => None,
        }
    }

    /// Stores the bytes once, as [`crate::Store::put_image`] does, stamped with the
    /// system's time.
    pub fn put(&mut self, image: &NewImage<'_>) -> Result<ImageInfo> {
        self.put_at(image, Clock::system().now_ms())
    }

    /// Clears the collector's stamp on an image, as [`crate::Store::touch_image`] does.
    pub fn touch(&mut self, id: &ImageId) -> Result<Option<ImageInfo>> {
        self.touch_image(id)
    }

    /// What is known about an image; `None` when the library does not have it.
    pub fn info(&self, id: &ImageId) -> Result<Option<ImageInfo>> {
        if let Some((info, _)) = self.read(|conn| read_info(conn, id))? {
            return Ok(Some(info));
        }
        Ok(self.out_of_the_copy(id).map(|(info, _)| info))
    }

    /// The bytes of an image; `None` when the library does not have it. As
    /// [`crate::Store::image_bytes`]: an image that only the copy in the backups
    /// directory has is read from there and written back, and so are bytes that are not
    /// the ones the id names (they went bad on the disk).
    pub fn bytes(&self, id: &ImageId) -> Result<Option<Vec<u8>>> {
        if let Some(bytes) = self.read(|conn| read_bytes(conn, id))?
            && ImageId::of(&bytes) == *id
        {
            return Ok(Some(bytes));
        }
        Ok(self.out_of_the_copy(id).map(|(_, bytes)| bytes))
    }

    /// How many images the file holds; 0 when there is no file.
    pub fn count(&self) -> Result<u64> {
        let count = self.read(|conn| {
            let count: i64 = conn.query_row("SELECT count(*) FROM images", [], |r| r.get(0))?;
            Ok(Some(count))
        })?;
        Ok(count
            .and_then(|count| u64::try_from(count).ok())
            .unwrap_or(0))
    }

    pub(crate) fn put_at(&self, image: &NewImage<'_>, now_ms: i64) -> Result<ImageInfo> {
        if self.read_only {
            return Err(read_only_error());
        }
        let info = ImageInfo {
            id: ImageId::of(image.bytes),
            format: image.format,
            width: image.width,
            height: image.height,
            density: image.density,
            byte_len: image.bytes.len() as u64,
            created_at: now_ms,
        };
        // The usual second put (the same picture inserted again): nothing is written,
        // unless the bytes that are there are not these any more.
        let there = self.read(|conn| {
            Ok(conn
                .query_row(
                    &format!(
                        "SELECT {INFO_COLS}, unreferenced_since, bytes = ?2 FROM images WHERE id = ?1"
                    ),
                    params![info.id.to_hex(), image.bytes],
                    |row| {
                        Ok((
                            info_from_row(row, 0)?,
                            row.get::<_, Option<i64>>(7)?,
                            row.get::<_, bool>(8)?,
                        ))
                    },
                )
                .optional()?)
        })?;
        if let Some((stored, None, true)) = there {
            return Ok(stored);
        }
        let write = || -> Result<ImageInfo> {
            let conn = self.writer()?;
            write_row(&conn, &info, image.bytes)?;
            match read_info(&conn, &info.id)? {
                Some((stored, _)) => Ok(stored),
                None => Err(StoreError::Sqlite(rusqlite::Error::QueryReturnedNoRows)),
            }
        };
        match write() {
            Err(error) if is_damage(&error) => {
                if self.repair()? {
                    write()
                } else {
                    Err(error)
                }
            }
            other => other,
        }
    }

    pub(crate) fn touch_image(&self, id: &ImageId) -> Result<Option<ImageInfo>> {
        self.touch_image_between(id, &mut || {})
    }

    /// [`touch_image`](Self::touch_image), with `between` run after the row was read and
    /// before its stamp is cleared: a test puts the collector's delete there.
    pub(crate) fn touch_image_between(
        &self,
        id: &ImageId,
        between: &mut dyn FnMut(),
    ) -> Result<Option<ImageInfo>> {
        if self.read_only {
            return Err(read_only_error());
        }
        let found = self.read(|conn| {
            let Some((info, stamp)) = read_info(conn, id)? else {
                return Ok(None);
            };
            if stamp.is_some() {
                between();
                // The collector deletes a row only while it has the stamp the collector
                // read, on a connection of its own: when that delete came between the
                // read and this, the row is gone and nothing is changed here.
                let changed = conn.execute(
                    "UPDATE images SET unreferenced_since = NULL WHERE id = ?1",
                    [id.to_hex()],
                )?;
                if changed == 0 {
                    return Ok(None);
                }
            }
            Ok(Some(info))
        })?;
        if found.is_some() {
            return Ok(found);
        }
        // Not in the file: the copy may have it, and brings it back without a stamp.
        Ok(self.out_of_the_copy(id).map(|(info, _)| info))
    }

    /// Refuses writes (`true`) or takes them again, as the `Store` this file belongs to
    /// does. While writes are refused the file is not created either.
    pub(crate) fn set_query_only(&self, on: bool) -> Result<()> {
        self.query_only.set(on);
        if let Some(kept) = &*self.conn.borrow()
            && !self.read_only
        {
            kept.conn.pragma_update(None, "query_only", on)?;
        }
        Ok(())
    }

    /// Closes the connection when one was opened, after a checkpoint of the WAL when
    /// `checkpoint` asks for it and the file can be written.
    pub(crate) fn close(self, checkpoint: bool) -> Result<()> {
        let on_disk = matches!(self.place, Place::Disk { .. });
        let Some(kept) = self.conn.into_inner() else {
            return Ok(());
        };
        if checkpoint && on_disk && !self.read_only {
            kept.conn
                .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
        }
        kept.conn.close().map_err(|(_, e)| StoreError::Sqlite(e))
    }

    /// Runs `read` on the file. `None` without running it while there is no file. When
    /// the file turns out damaged, the answer is `None` (so that the caller asks the
    /// copy), after the file was replaced where this connection may do that and `read`
    /// was run once more on the new one.
    fn read<T>(&self, mut read: impl FnMut(&Connection) -> Result<Option<T>>) -> Result<Option<T>> {
        for first in [true, false] {
            let outcome = match self.reader() {
                Ok(Some(conn)) => read(&conn),
                Ok(None) => return Ok(None),
                Err(error) => Err(error),
            };
            match outcome {
                Err(error) if is_damage(&error) => {
                    // A repair that fails leaves the reads to the copy.
                    if !(first && self.repair().unwrap_or(false)) {
                        return Ok(None);
                    }
                }
                other => return other,
            }
        }
        Ok(None)
    }

    /// After a damage error: lets the connection go and, where this connection may
    /// write, replaces the file ([`replace_damaged`]). Answers whether a new file may be
    /// at the path now.
    fn repair(&self) -> Result<bool> {
        let suspect = self.conn.borrow().as_ref().and_then(|kept| kept.file);
        self.conn.borrow_mut().take();
        let Place::Disk { path, copy } = &self.place else {
            return Ok(false);
        };
        if self.read_only || self.query_only.get() {
            return Ok(false);
        }
        // The error came from opening the file: it is the one at the path now.
        let suspect = suspect.or_else(|| file_identity(path));
        replace_damaged(path, copy, suspect)?;
        Ok(true)
    }

    /// The connection for a read: `None` while there is no file. The absence is not
    /// remembered, so a file that appears later (the first put of another connection,
    /// the tool) is found by the next read; nor is a file that went, so a new file at the
    /// path (one that replaced a damaged one) is the one that is read.
    fn reader(&self) -> Result<Option<Ref<'_, Connection>>> {
        if let Place::Disk { path, .. } = &self.place {
            let gone =
                matches!(&*self.conn.borrow(), Some(kept) if file_identity(path) != kept.file);
            if gone {
                self.conn.borrow_mut().take();
            }
        }
        if self.conn.borrow().is_none() {
            let opened = match &self.place {
                Place::Memory => Some(open_in_memory()?),
                Place::Disk { path, .. } if self.read_only => open(path, Mode::Read)?,
                Place::Disk { path, .. } => open(path, Mode::Write)?,
            };
            let Some(conn) = opened else {
                return Ok(None);
            };
            self.keep(conn)?;
        }
        Ok(self.kept())
    }

    /// The connection for a write, making the file when it is not there.
    fn writer(&self) -> Result<Ref<'_, Connection>> {
        if self.read_only {
            return Err(read_only_error());
        }
        if let Some(conn) = self.reader()? {
            return Ok(conn);
        }
        let Place::Disk { path, .. } = &self.place else {
            unreachable!("a library in memory always has its connection");
        };
        if self.query_only.get() {
            // A dry run creates nothing.
            return Err(read_only_error());
        }
        let Some(conn) = open(path, Mode::Create)? else {
            unreachable!("a file that is created is opened");
        };
        self.keep(conn)?;
        Ok(self.kept().expect("the connection was just kept"))
    }

    fn keep(&self, conn: Connection) -> Result<()> {
        if self.query_only.get() && !self.read_only {
            conn.pragma_update(None, "query_only", true)?;
        }
        let file = match &self.place {
            Place::Disk { path, .. } => file_identity(path),
            Place::Memory => None,
        };
        *self.conn.borrow_mut() = Some(Kept { conn, file });
        Ok(())
    }

    fn kept(&self) -> Option<Ref<'_, Connection>> {
        Ref::filter_map(self.conn.borrow(), |kept| {
            kept.as_ref().map(|kept| &kept.conn)
        })
        .ok()
    }

    /// An image that this file does not have, from the copy of the pictures in the
    /// backups directory: what is known about it and its bytes, when the copy has it and
    /// the bytes are the ones the id names. Written back into this file (over bytes that
    /// went bad), unless this connection is for reading only or in a dry run. Nothing
    /// that goes wrong on this way is an error: the answer is then that the library does
    /// not have the image.
    fn out_of_the_copy(&self, id: &ImageId) -> Option<(ImageInfo, Vec<u8>)> {
        let Place::Disk { copy, .. } = &self.place else {
            return None;
        };
        let (info, bytes) = copy::read_from_copy(copy, id)?;
        if !self.read_only && !self.query_only.get() {
            // Whether or not it can be written back, the bytes are found.
            let _ = self
                .writer()
                .and_then(|conn| write_row(&conn, &info, &bytes));
        }
        Some((info, bytes))
    }
}

#[cfg(test)]
mod send_check {
    fn assert_send<T: Send>() {}

    #[test]
    fn an_image_file_is_send() {
        assert_send::<super::ImageFile>();
    }
}
