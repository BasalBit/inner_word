//! `images.db` by itself: the file, its schema and pragmas, and one connection to it.

use std::cell::{Cell, OnceCell};
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
    let _making = MAKING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
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

/// Writes a row unless one with this id is there; a row that is there loses the
/// collector's stamp and keeps everything else. One statement, so it is whole or not at
/// all.
pub(crate) fn write_row(conn: &Connection, info: &ImageInfo, bytes: &[u8]) -> Result<()> {
    conn.execute(
        "INSERT INTO images
             (id, format, width, height, density, byte_len, created_at, unreferenced_since, bytes)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8)
         ON CONFLICT(id) DO UPDATE SET unreferenced_since = NULL
             WHERE unreferenced_since IS NOT NULL",
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

/// `images.db` by itself: one connection, for a thread of its own.
///
/// The image methods of a [`crate::Store`] go through the one it holds. Whoever reads or
/// writes pictures in the background (16 MB should never be hashed, written or read on
/// the main thread of an app) has another, on the same file: the two wait for each other.
/// A library in memory has no file a second connection could see; there everything goes
/// through the `Store`.
///
/// `Send`, not `Sync`.
pub struct ImageFile {
    place: Place,
    read_only: bool,
    /// Whether writes are refused for the time being ([`crate::Store::set_query_only`]).
    query_only: Cell<bool>,
    /// Opened at the first use that finds something to open.
    conn: OnceCell<Connection>,
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
            .field("open", &self.conn.get().is_some())
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
            conn: OnceCell::new(),
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
            conn: OnceCell::new(),
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
        if let Some(conn) = self.reader()?
            && let Some((info, _)) = read_info(conn, id)?
        {
            return Ok(Some(info));
        }
        Ok(self.out_of_the_copy(id).map(|(info, _)| info))
    }

    /// The bytes of an image; `None` when the library does not have it. As
    /// [`crate::Store::image_bytes`]: an image that only the copy in the backups
    /// directory has is read from there and written back.
    pub fn bytes(&self, id: &ImageId) -> Result<Option<Vec<u8>>> {
        if let Some(conn) = self.reader()?
            && let Some(bytes) = read_bytes(conn, id)?
        {
            return Ok(Some(bytes));
        }
        Ok(self.out_of_the_copy(id).map(|(_, bytes)| bytes))
    }

    /// How many images the file holds; 0 when there is no file.
    pub fn count(&self) -> Result<u64> {
        let Some(conn) = self.reader()? else {
            return Ok(0);
        };
        let count: i64 = conn.query_row("SELECT count(*) FROM images", [], |r| r.get(0))?;
        Ok(u64::try_from(count).unwrap_or(0))
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
        // The usual second put (the same picture inserted again): nothing is written.
        if let Some(conn) = self.reader()?
            && let Some((stored, None)) = read_info(conn, &info.id)?
        {
            return Ok(stored);
        }
        let conn = self.writer()?;
        write_row(conn, &info, image.bytes)?;
        match read_info(conn, &info.id)? {
            Some((stored, _)) => Ok(stored),
            None => Err(StoreError::Sqlite(rusqlite::Error::QueryReturnedNoRows)),
        }
    }

    pub(crate) fn touch_image(&self, id: &ImageId) -> Result<Option<ImageInfo>> {
        if self.read_only {
            return Err(read_only_error());
        }
        if let Some(conn) = self.reader()?
            && let Some((info, stamp)) = read_info(conn, id)?
        {
            if stamp.is_some() {
                conn.execute(
                    "UPDATE images SET unreferenced_since = NULL WHERE id = ?1",
                    [id.to_hex()],
                )?;
            }
            return Ok(Some(info));
        }
        // Not in the file: the copy may have it, and brings it back without a stamp.
        Ok(self.out_of_the_copy(id).map(|(info, _)| info))
    }

    /// Refuses writes (`true`) or takes them again, as the `Store` this file belongs to
    /// does. While writes are refused the file is not created either.
    pub(crate) fn set_query_only(&self, on: bool) -> Result<()> {
        self.query_only.set(on);
        if let Some(conn) = self.conn.get()
            && !self.read_only
        {
            conn.pragma_update(None, "query_only", on)?;
        }
        Ok(())
    }

    /// Closes the connection when one was opened, after a checkpoint of the WAL when
    /// `checkpoint` asks for it and the file can be written.
    pub(crate) fn close(self, checkpoint: bool) -> Result<()> {
        let on_disk = matches!(self.place, Place::Disk { .. });
        let Some(conn) = self.conn.into_inner() else {
            return Ok(());
        };
        if checkpoint && on_disk && !self.read_only {
            conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
        }
        conn.close().map_err(|(_, e)| StoreError::Sqlite(e))
    }

    /// The connection for a read: `None` while there is no file. The absence is not
    /// remembered, so a file that appears later (the first put of another connection,
    /// the tool) is found by the next read.
    fn reader(&self) -> Result<Option<&Connection>> {
        if let Some(conn) = self.conn.get() {
            return Ok(Some(conn));
        }
        let opened = match &self.place {
            Place::Memory => Some(open_in_memory()?),
            Place::Disk { path, .. } if self.read_only => open(path, Mode::Read)?,
            Place::Disk { path, .. } => open(path, Mode::Write)?,
        };
        opened.map(|conn| self.keep(conn)).transpose()
    }

    /// The connection for a write, making the file when it is not there.
    fn writer(&self) -> Result<&Connection> {
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
        self.keep(conn)
    }

    fn keep(&self, conn: Connection) -> Result<&Connection> {
        if self.query_only.get() && !self.read_only {
            conn.pragma_update(None, "query_only", true)?;
        }
        Ok(self.conn.get_or_init(|| conn))
    }

    /// An image that this file does not have, from the copy of the pictures in the
    /// backups directory: what is known about it and its bytes, when the copy has it and
    /// the bytes are the ones the id names. Written back into this file, unless this
    /// connection is for reading only or in a dry run. Nothing that goes wrong on this
    /// way is an error: the answer is then that the library does not have the image.
    fn out_of_the_copy(&self, id: &ImageId) -> Option<(ImageInfo, Vec<u8>)> {
        let Place::Disk { copy, .. } = &self.place else {
            return None;
        };
        let (info, bytes) = copy::read_from_copy(copy, id)?;
        if !self.read_only && !self.query_only.get() {
            // Whether or not it can be written back, the bytes are found.
            let _ = self
                .writer()
                .and_then(|conn| write_row(conn, &info, &bytes));
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
