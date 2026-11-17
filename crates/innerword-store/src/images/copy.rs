//! The one copy of the pictures and of the files of pages, in the backups directory.
//!
//! The daily backups of the library database hold the documents and no pictures (that is
//! what the second file is for), and no files of pages. So that a damaged or deleted
//! `images.db` does not lose them, there is one copy: `images.db` in the backups directory
//! ([`crate::images_backup_path`]), a database of the same form, with both tables.
//!
//! - **Bringing it up to date** ([`back_up_images`]) copies the rows the copy does not
//!   have yet, found by their ids, a slice at a time (images first, then files, in one
//!   budget of bytes). A row is written once and never changes, so nothing is ever copied
//!   twice. It is meant for a background thread, after a launch: never before the window
//!   opens, never on the main thread.
//! - **Reading from it**: when `images.db` does not have an image or a file that is asked
//!   for (or is damaged, or has bytes that are not the ones the id names), the copy is
//!   asked ([`read_from_copy`], [`read_file_from_copy`]), and what it has is written back.
//! - **A damaged copy** is moved aside and made again by the next [`back_up_images`];
//!   a copy that cannot be brought up to date is said by the collector
//!   ([`crate::ImagesCollected::copy_failed`]).
//! - **Taking a row out of it** is the collector's and nobody else's: a row leaves the
//!   copy only when the collector deleted it from `images.db` and no kept backup of the
//!   library database names it. The collector marks the rows it is about to delete (in
//!   the copy, `unreferenced_since` is that mark: the time the collector took the row out
//!   of `images.db`), and only marked rows are ever removed. A copy whose `images.db` was
//!   lost holds no marks, so nothing of it goes.
//!
//! So a backup of the library database that is brought back finds its pictures and the
//! files of its pages in `images.db` or in the copy.

use std::path::Path;

use rusqlite::types::{ToSqlOutput, ValueRef};
use rusqlite::{Connection, OpenFlags, TransactionBehavior, params, params_from_iter};

use super::collect::give_back_free_pages;
use super::file::{self, Mode, Table};
use super::scan::{self, Named};
use super::{FileId, FileInfo, ImageId, ImageInfo, hash16, parse16};
use crate::backup::list_backups;
use crate::error::Result;
use crate::paths::{backups_dir, images_backup_path, images_path};

/// How many bytes of pictures and files of pages [`back_up_all_images`] copies in one
/// transaction: 32 MB, two pictures of the largest size the library stores, or one file of
/// a page of the largest size. The two tables count together.
pub const IMAGE_BACKUP_SLICE_BYTES: u64 = 32 * 1024 * 1024;

/// What a call of [`back_up_images`] found and did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImagesBackedUp {
    /// How many images `images.db` holds.
    pub images: u64,
    /// How many were copied by this call.
    pub copied: u64,
    /// Their bytes.
    pub bytes_copied: u64,
    /// How many the copy still does not have: call again.
    pub waiting: u64,
    /// How many files of pages `images.db` holds.
    pub files: u64,
    /// How many of them were copied by this call.
    pub files_copied: u64,
    /// Their bytes.
    pub file_bytes_copied: u64,
    /// How many files the copy still does not have: call again.
    pub files_waiting: u64,
}

impl ImagesBackedUp {
    /// The counts of one table: held, copied, bytes copied, waiting.
    fn of(&mut self, table: Table) -> (&mut u64, &mut u64, &mut u64, &mut u64) {
        match table {
            Table::Images => (
                &mut self.images,
                &mut self.copied,
                &mut self.bytes_copied,
                &mut self.waiting,
            ),
            Table::Files => (
                &mut self.files,
                &mut self.files_copied,
                &mut self.file_bytes_copied,
                &mut self.files_waiting,
            ),
        }
    }
}

/// Brings the copy of the pictures and the files of pages in the backups directory of the
/// library at `database` one slice further up to date: copies rows the copy does not have
/// yet (images first, then files) until `max_bytes` are copied (always at least one row),
/// in one transaction, and says how many are still waiting.
///
/// Opens connections of its own (`images.db` read-only, the copy for writing), so it runs
/// on any thread while a [`crate::Store`] of the same process is open: run it in the
/// background. Does nothing, and creates nothing, for a library without images or pages.
///
/// **A damaged file** is no reason for the copy to stop. When the copy turns out damaged
/// (its header or a page is not what SQLite wrote), it is moved aside
/// (`backups/images.db.damaged-<unix ms>`) and a new one is made, with what can still be
/// read of the old one and `images.db` does not have, and then everything of `images.db`.
/// A damaged `images.db` is moved aside the same way, its rows coming back from the copy
/// ([`crate::ImageFile`]): so call it only where the library is written (the app, the
/// holder of the data directory's lock). Bytes in `images.db` that are not the ones their
/// id names are not copied.
pub fn back_up_images(database: &Path, max_bytes: u64) -> Result<ImagesBackedUp> {
    match back_up_slice(database, max_bytes) {
        Err(error) if file::is_damage(&error) => {
            let images = images_path(database);
            let copy = images_backup_path(database);
            let mut replaced = false;
            for (path, other) in [(&images, &copy), (&copy, &images)] {
                if file::is_damaged(path) {
                    file::replace_damaged(path, other, file::file_identity(path))?;
                    replaced = true;
                }
            }
            if !replaced {
                return Err(error);
            }
            back_up_slice(database, max_bytes)
        }
        other => other,
    }
}

fn back_up_slice(database: &Path, max_bytes: u64) -> Result<ImagesBackedUp> {
    let mut done = ImagesBackedUp::default();
    let Some(images) = file::open(&images_path(database), Mode::Read)? else {
        return Ok(done);
    };
    // The ids `images.db` holds, per table; a file of version 1 has no files of pages.
    let mut stored: Vec<(Table, Vec<String>)> = Vec::new();
    for table in Table::BOTH {
        let ids = if table.is_in(&images)? {
            let mut stmt =
                images.prepare(&format!("SELECT id FROM {} ORDER BY rowid", table.name()))?;
            stmt.query_map([], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<String>>>()?
        } else {
            Vec::new()
        };
        *done.of(table).0 = ids.len() as u64;
        stored.push((table, ids));
    }
    let copy_path = images_backup_path(database);
    if stored.iter().all(|(_, ids)| ids.is_empty()) && !copy_path.exists() {
        return Ok(done);
    }
    // Made, or brought to this version, with both tables.
    let Some(copy) = file::open(&copy_path, Mode::Create)? else {
        unreachable!("a file that is created is opened");
    };
    // Per table: the rows the collector once took out and that are stored again (they
    // are rows like any other), and the rows the copy does not have.
    let mut plan: Vec<(Table, Vec<String>, Vec<String>)> = Vec::new();
    for (table, ids) in stored {
        let in_copy: std::collections::HashMap<String, Option<i64>> = {
            let mut stmt = copy.prepare(&format!(
                "SELECT id, unreferenced_since FROM {}",
                table.name()
            ))?;
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?
        };
        let back: Vec<String> = ids
            .iter()
            .filter(|id| matches!(in_copy.get(*id), Some(Some(_))))
            .cloned()
            .collect();
        let waiting: Vec<String> = ids
            .into_iter()
            .filter(|id| !in_copy.contains_key(id))
            .collect();
        plan.push((table, back, waiting));
    }
    if plan
        .iter()
        .all(|(_, back, waiting)| back.is_empty() && waiting.is_empty())
    {
        return Ok(done);
    }

    let tx = rusqlite::Transaction::new_unchecked(&copy, TransactionBehavior::Immediate)?;
    let mut copied_any = false;
    let mut bytes_copied = 0u64;
    for (table, back, waiting) in &plan {
        for id in back {
            tx.execute(
                &format!(
                    "UPDATE {} SET unreferenced_since = NULL WHERE id = ?1",
                    table.name()
                ),
                [id],
            )?;
        }
        let cols = table.info_cols();
        let count = cols.split(',').count();
        let mut read = images.prepare(&format!(
            "SELECT {cols}, bytes FROM {} WHERE id = ?1",
            table.name()
        ))?;
        let places: Vec<String> = (1..=count).map(|n| format!("?{n}")).collect();
        let mut write = tx.prepare(&format!(
            "INSERT OR IGNORE INTO {} ({cols}, unreferenced_since, bytes)
             VALUES ({}, NULL, ?{})",
            table.name(),
            places.join(", "),
            count + 1
        ))?;
        let mut looked_at = 0;
        for id in waiting {
            if copied_any && bytes_copied >= max_bytes {
                break;
            }
            looked_at += 1;
            // A row of `images.db` that cannot be read, or whose bytes are not the ones
            // its id names, is not copied: the copy keeps nothing that is not whole, and
            // `images.db` repairs it from there when it is read.
            let mut rows = read.query([id])?;
            // Deleted since the list was read: nothing to copy.
            let row = match rows.next() {
                Ok(Some(row)) => row,
                Ok(None) => continue,
                Err(error) if file::is_sqlite_damage(&error) => continue,
                Err(error) => return Err(error.into()),
            };
            let bytes = match row.get_ref(count).and_then(|value| Ok(value.as_blob()?)) {
                Ok(bytes) => bytes,
                Err(error) if file::is_sqlite_damage(&error) => continue,
                Err(error) => return Err(error.into()),
            };
            if parse16(id.as_bytes()).is_none_or(|id| hash16(bytes) != id) {
                continue;
            }
            // The row as it is, column for column.
            let mut values = Vec::with_capacity(count + 1);
            for index in 0..count {
                values.push(ToSqlOutput::Borrowed(row.get_ref(index)?));
            }
            values.push(ToSqlOutput::Borrowed(ValueRef::Blob(bytes)));
            write.execute(params_from_iter(values.iter()))?;
            copied_any = true;
            bytes_copied += bytes.len() as u64;
            let (_, copied, table_bytes, _) = done.of(*table);
            *copied += 1;
            *table_bytes += bytes.len() as u64;
        }
        *done.of(*table).3 = (waiting.len() - looked_at) as u64;
    }
    tx.commit()?;
    Ok(done)
}

/// Brings the copy of the pictures and the files of pages all the way up to date, a slice
/// of [`IMAGE_BACKUP_SLICE_BYTES`] at a time ([`back_up_images`]; each slice is committed,
/// so a run that is cut short loses the slice it was in and nothing else). The answer
/// counts what all slices copied.
pub fn back_up_all_images(database: &Path) -> Result<ImagesBackedUp> {
    let mut all = ImagesBackedUp::default();
    loop {
        let slice = back_up_images(database, IMAGE_BACKUP_SLICE_BYTES)?;
        all.images = slice.images;
        all.copied += slice.copied;
        all.bytes_copied += slice.bytes_copied;
        all.waiting = slice.waiting;
        all.files = slice.files;
        all.files_copied += slice.files_copied;
        all.file_bytes_copied += slice.file_bytes_copied;
        all.files_waiting = slice.files_waiting;
        let waiting = slice.waiting + slice.files_waiting;
        if waiting == 0 || slice.copied + slice.files_copied == 0 {
            return Ok(all);
        }
    }
}

/// An image as the copy at `copy` has it: what is known about it and its bytes. `None`
/// when there is no copy, when it does not have the image, when it cannot be read, and
/// when the bytes it has are not the ones the id names (a copy is only as good as its
/// disk).
pub(crate) fn read_from_copy(copy: &Path, id: &ImageId) -> Option<(ImageInfo, Vec<u8>)> {
    let conn = file::open(copy, Mode::Read).ok()??;
    let (info, _) = file::read_info(&conn, id).ok()??;
    let bytes = file::read_bytes(&conn, id).ok()??;
    (ImageId::of(&bytes) == *id).then_some((info, bytes))
}

/// [`read_from_copy`] for a file of a page. A copy of version 1 has none.
pub(crate) fn read_file_from_copy(copy: &Path, id: &FileId) -> Option<(FileInfo, Vec<u8>)> {
    let conn = file::open(copy, Mode::Read).ok()??;
    if !file::has_files(&conn).ok()? {
        return None;
    }
    let (info, _) = file::read_file_info(&conn, id).ok()??;
    let bytes = file::read_file_bytes(&conn, id).ok()??;
    (FileId::of(&bytes) == *id).then_some((info, bytes))
}

/// Whether the copy holds a row the collector marked: an image or a file it deleted from
/// `images.db` that could not leave the copy yet.
pub(crate) fn holds_collected(database: &Path) -> bool {
    let marked = || -> Result<bool> {
        let Some(copy) = file::open(&images_backup_path(database), Mode::Read)? else {
            return Ok(false);
        };
        for table in Table::BOTH {
            if table.is_in(&copy)?
                && copy.query_row(
                    &format!(
                        "SELECT EXISTS (SELECT 1 FROM {} WHERE unreferenced_since IS NOT NULL)",
                        table.name()
                    ),
                    [],
                    |row| row.get(0),
                )?
            {
                return Ok(true);
            }
        }
        Ok(false)
    };
    marked().unwrap_or(false)
}

/// Marks, in the copy, the rows the collector is about to delete from `images.db`: each
/// with its table and its id as the column holds it.
pub(crate) fn mark_collected(database: &Path, rows: &[(Table, String)], now_ms: i64) -> Result<()> {
    let Some(copy) = file::open(&images_backup_path(database), Mode::Write)? else {
        return Ok(());
    };
    let tx = rusqlite::Transaction::new_unchecked(&copy, TransactionBehavior::Immediate)?;
    for (table, hex) in rows {
        tx.execute(
            &format!(
                "UPDATE {} SET unreferenced_since = ?2
                 WHERE id = ?1 AND unreferenced_since IS NULL",
                table.name()
            ),
            params![hex, now_ms],
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// Takes out of the copy what the collector deleted from `images.db`: the marked rows
/// that `images` (the connection to `images.db`) no longer has, that the library does
/// not name (`named`) and that no kept backup of the library database names. A marked
/// row that `images.db` has after all (the collector's delete lost against a put or a
/// touch, or the row was brought back from the copy) loses its mark. Answers how many
/// rows went, of both tables.
///
/// When a kept backup cannot be read nothing goes: what it names is not known.
pub(crate) fn remove_collected(database: &Path, images: &Connection, named: &Named) -> Result<u64> {
    let Some(copy) = file::open(&images_backup_path(database), Mode::Write)? else {
        return Ok(0);
    };
    let mut back = Vec::new();
    let mut gone = Vec::new();
    for table in Table::BOTH {
        let marked = {
            let mut stmt = copy.prepare(&format!(
                "SELECT id FROM {} WHERE unreferenced_since IS NOT NULL ORDER BY rowid",
                table.name()
            ))?;
            stmt.query_map([], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<String>>>()?
        };
        if marked.is_empty() {
            continue;
        }
        // `images.db` of version 1 has no files of pages at all.
        let mut stored = if table.is_in(images)? {
            Some(images.prepare(&format!(
                "SELECT EXISTS (SELECT 1 FROM {} WHERE id = ?1)",
                table.name()
            ))?)
        } else {
            None
        };
        for hex in marked {
            let still_there = match stored.as_mut() {
                Some(stmt) => stmt.query_row([&hex], |row| row.get::<_, bool>(0))?,
                None => false,
            };
            if still_there {
                back.push((table, hex));
            } else if parse16(hex.as_bytes()).is_some() && !named.names_hex(table, &hex) {
                gone.push((table, hex));
            }
        }
    }
    if !gone.is_empty() {
        let kept = named_by_kept_backups(database)?;
        gone.retain(|(table, hex)| !kept.names_hex(*table, hex));
    }
    if back.is_empty() && gone.is_empty() {
        return Ok(0);
    }
    let tx = rusqlite::Transaction::new_unchecked(&copy, TransactionBehavior::Immediate)?;
    for (table, hex) in &back {
        tx.execute(
            &format!(
                "UPDATE {} SET unreferenced_since = NULL WHERE id = ?1",
                table.name()
            ),
            [hex],
        )?;
    }
    let mut removed = 0;
    for (table, hex) in &gone {
        removed += tx.execute(
            &format!(
                "DELETE FROM {} WHERE id = ?1 AND unreferenced_since IS NOT NULL",
                table.name()
            ),
            [hex],
        )? as u64;
    }
    tx.commit()?;
    if removed > 0 {
        give_back_free_pages(&copy);
    }
    Ok(removed)
}

/// Every image id and file id that a kept daily backup of the library database at
/// `database` names.
fn named_by_kept_backups(database: &Path) -> Result<Named> {
    let mut named = Named::default();
    for (_, backup) in list_backups(&backups_dir(database))? {
        // A backup is a file that was written once and renamed into place: it is read
        // as it is, without locks and without the files of a journal beside it.
        let conn = Connection::open_with_flags(
            immutable_uri(&backup),
            OpenFlags::SQLITE_OPEN_READ_ONLY
                | OpenFlags::SQLITE_OPEN_URI
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        named.extend(scan::referenced_all(&conn)?);
    }
    Ok(named)
}

/// `path` as the address of a database file that nothing changes.
fn immutable_uri(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;

    let mut uri = String::from("file:");
    for byte in path.as_os_str().as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                uri.push(*byte as char);
            }
            other => uri.push_str(&format!("%{other:02X}")),
        }
    }
    uri.push_str("?immutable=1");
    uri
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn a_path_becomes_an_address_sqlite_reads_as_that_path() {
        assert_eq!(
            immutable_uri(Path::new("/a/Inner Word/backups/library-17.db")),
            "file:/a/Inner%20Word/backups/library-17.db?immutable=1"
        );
        assert_eq!(
            immutable_uri(Path::new("/a/50%?#é/x.db")),
            "file:/a/50%25%3F%23%C3%A9/x.db?immutable=1"
        );
    }

    /// The address is really opened as the file it names, with characters in the path
    /// that mean something in an address.
    #[test]
    fn a_backup_in_a_directory_with_a_strange_name_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let odd = dir.path().join("Inner Word 50%?#é");
        let database = odd.join("library.db");
        let id = "3f2a9c1b5d6e7f80a1b2c3d4e5f60718";
        {
            let mut store = crate::Store::open(&database).unwrap();
            store
                .create_document(
                    None,
                    &crate::DocumentContent {
                        content: &format!(r#"{{"blocks":[{{"type":"image","image":"{id}"}}]}}"#),
                        title: "With a picture",
                        plain_text: "",
                        word_count: 0,
                        char_count: 0,
                    },
                )
                .unwrap();
            std::fs::create_dir_all(backups_dir(&database)).unwrap();
            store
                .backup_to(backups_dir(&database).join("library-1700000000000.db"))
                .unwrap();
        }
        let named = named_by_kept_backups(&database).unwrap();
        assert_eq!(named.images, HashSet::from([ImageId::parse(id).unwrap()]));
        assert!(named.files.is_empty());
        // Reading a backup leaves nothing beside it.
        let mut names: Vec<String> = std::fs::read_dir(backups_dir(&database))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, vec!["library-1700000000000.db"]);
    }
}
