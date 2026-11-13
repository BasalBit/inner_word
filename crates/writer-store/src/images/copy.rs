//! The one copy of the pictures, in the backups directory.
//!
//! The daily backups of the library database hold the documents and no pictures (that is
//! what the second file is for). So that a damaged or deleted `images.db` does not lose
//! the pictures, there is one copy of them: `images.db` in the backups directory
//! ([`crate::images_backup_path`]), a database of the same form.
//!
//! - **Bringing it up to date** ([`back_up_images`]) copies the rows the copy does not
//!   have yet, found by their ids, a slice at a time. A picture is written once and never
//!   changes, so nothing is ever copied twice. It is meant for a background thread,
//!   after a launch: never before the window opens, never on the main thread.
//! - **Reading from it**: when `images.db` does not have an image that is asked for (or
//!   is damaged, or has bytes that are not the ones the id names), the copy is asked
//!   ([`read_from_copy`]), and what it has is written back.
//! - **A damaged copy** is moved aside and made again by the next [`back_up_images`];
//!   a copy that cannot be brought up to date is said by the collector
//!   ([`crate::ImagesCollected::copy_failed`]).
//! - **Taking a picture out of it** is the collector's and nobody else's: a row leaves
//!   the copy only when the collector deleted it from `images.db` and no kept backup of
//!   the library database names it. The collector marks the rows it is about to delete
//!   (in the copy, `unreferenced_since` is that mark: the time the collector took the
//!   image out of `images.db`), and only marked rows are ever removed. A copy whose
//!   `images.db` was lost holds no marks, so nothing of it goes.
//!
//! So a backup of the library database that is brought back finds its pictures in
//! `images.db` or in the copy.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use rusqlite::types::ToSqlOutput;
use rusqlite::{Connection, OpenFlags, TransactionBehavior, params};

use super::collect::give_back_free_pages;
use super::file::{self, INFO_COLS, Mode};
use super::{ImageId, ImageInfo, scan};
use crate::backup::list_backups;
use crate::error::Result;
use crate::paths::{backups_dir, images_backup_path, images_path};

/// How many bytes of pictures [`back_up_all_images`] copies in one transaction: 32 MB,
/// two pictures of the largest size the library stores.
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
}

/// Brings the copy of the pictures in the backups directory of the library at `database`
/// one slice further up to date: copies images the copy does not have yet until
/// `max_bytes` are copied (always at least one image), in one transaction, and says how
/// many are still waiting.
///
/// Opens connections of its own (`images.db` read-only, the copy for writing), so it runs
/// on any thread while a [`crate::Store`] of the same process is open: run it in the
/// background. Does nothing, and creates nothing, for a library without images.
///
/// **A damaged file** is no reason for the copy to stop. When the copy turns out damaged
/// (its header or a page is not what SQLite wrote), it is moved aside
/// (`backups/images.db.damaged-<unix ms>`) and a new one is made, with what can still be
/// read of the old one and `images.db` does not have, and then everything of `images.db`.
/// A damaged `images.db` is moved aside the same way, its pictures coming back from the
/// copy ([`crate::ImageFile`]): so call it only where the library is written (the app, the
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
                    file::replace_damaged(path, other, file::file_id(path))?;
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
    let stored = {
        let mut stmt = images.prepare("SELECT id FROM images ORDER BY rowid")?;
        stmt.query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?
    };
    done.images = stored.len() as u64;
    let copy_path = images_backup_path(database);
    if stored.is_empty() && !copy_path.exists() {
        return Ok(done);
    }
    let Some(copy) = file::open(&copy_path, Mode::Create)? else {
        unreachable!("a file that is created is opened");
    };
    let in_copy: HashMap<String, Option<i64>> = {
        let mut stmt = copy.prepare("SELECT id, unreferenced_since FROM images")?;
        stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?
    };
    // An image the collector once took out and that is stored again is a row like any
    // other.
    let back: Vec<&String> = stored
        .iter()
        .filter(|id| matches!(in_copy.get(*id), Some(Some(_))))
        .collect();
    let waiting: Vec<&String> = stored
        .iter()
        .filter(|id| !in_copy.contains_key(*id))
        .collect();
    if back.is_empty() && waiting.is_empty() {
        return Ok(done);
    }

    let tx = rusqlite::Transaction::new_unchecked(&copy, TransactionBehavior::Immediate)?;
    for id in back {
        tx.execute(
            "UPDATE images SET unreferenced_since = NULL WHERE id = ?1",
            [id],
        )?;
    }
    let mut looked_at = 0;
    {
        let mut read = images.prepare(&format!(
            "SELECT {INFO_COLS}, bytes FROM images WHERE id = ?1"
        ))?;
        let mut write = tx.prepare(
            "INSERT OR IGNORE INTO images
                 (id, format, width, height, density, byte_len, created_at, unreferenced_since, bytes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8)",
        )?;
        for id in &waiting {
            if done.copied > 0 && done.bytes_copied >= max_bytes {
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
            // The row as it is, column for column.
            let column = |index: usize| row.get_ref(index).map(ToSqlOutput::Borrowed);
            let bytes = match row.get_ref(7).and_then(|value| Ok(value.as_blob()?)) {
                Ok(bytes) => bytes,
                Err(error) if file::is_sqlite_damage(&error) => continue,
                Err(error) => return Err(error.into()),
            };
            if ImageId::parse(id).is_none_or(|id| ImageId::of(bytes) != id) {
                continue;
            }
            write.execute(params![
                column(0)?,
                column(1)?,
                column(2)?,
                column(3)?,
                column(4)?,
                column(5)?,
                column(6)?,
                bytes,
            ])?;
            done.copied += 1;
            done.bytes_copied += bytes.len() as u64;
        }
    }
    tx.commit()?;
    done.waiting = (waiting.len() - looked_at) as u64;
    Ok(done)
}

/// Brings the copy of the pictures all the way up to date, a slice of
/// [`IMAGE_BACKUP_SLICE_BYTES`] at a time ([`back_up_images`]; each slice is committed,
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
        if slice.waiting == 0 || slice.copied == 0 {
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

/// Whether the copy holds a row the collector marked: an image it deleted from
/// `images.db` that could not leave the copy yet.
pub(crate) fn holds_collected(database: &Path) -> bool {
    let marked = || -> Result<bool> {
        let Some(copy) = file::open(&images_backup_path(database), Mode::Read)? else {
            return Ok(false);
        };
        Ok(copy.query_row(
            "SELECT EXISTS (SELECT 1 FROM images WHERE unreferenced_since IS NOT NULL)",
            [],
            |row| row.get(0),
        )?)
    };
    marked().unwrap_or(false)
}

/// Marks, in the copy, the images the collector is about to delete from `images.db`.
pub(crate) fn mark_collected(database: &Path, ids: &[ImageId], now_ms: i64) -> Result<()> {
    let Some(copy) = file::open(&images_backup_path(database), Mode::Write)? else {
        return Ok(());
    };
    let tx = rusqlite::Transaction::new_unchecked(&copy, TransactionBehavior::Immediate)?;
    for id in ids {
        tx.execute(
            "UPDATE images SET unreferenced_since = ?2
             WHERE id = ?1 AND unreferenced_since IS NULL",
            params![id.to_hex(), now_ms],
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// Takes out of the copy what the collector deleted from `images.db`: the marked rows
/// that `images` (the connection to `images.db`) no longer has, that the library does
/// not name (`named`) and that no kept backup of the library database names. A marked
/// row that `images.db` has after all (the collector's delete lost against a put or a
/// touch, or the image was brought back from the copy) loses its mark. Answers how many
/// rows went.
///
/// When a kept backup cannot be read nothing goes: what it names is not known.
pub(crate) fn remove_collected(
    database: &Path,
    images: &Connection,
    named: &HashSet<ImageId>,
) -> Result<u64> {
    let Some(copy) = file::open(&images_backup_path(database), Mode::Write)? else {
        return Ok(0);
    };
    let marked = {
        let mut stmt = copy
            .prepare("SELECT id FROM images WHERE unreferenced_since IS NOT NULL ORDER BY rowid")?;
        stmt.query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?
    };
    if marked.is_empty() {
        return Ok(0);
    }
    let mut back = Vec::new();
    let mut gone = Vec::new();
    {
        let mut stored = images.prepare("SELECT EXISTS (SELECT 1 FROM images WHERE id = ?1)")?;
        for hex in marked {
            if stored.query_row([&hex], |row| row.get::<_, bool>(0))? {
                back.push(hex);
            } else if ImageId::parse(&hex).is_some_and(|id| !named.contains(&id)) {
                gone.push(hex);
            }
        }
    }
    if !gone.is_empty() {
        let kept = named_by_kept_backups(database)?;
        gone.retain(|hex| ImageId::parse(hex).is_some_and(|id| !kept.contains(&id)));
    }
    if back.is_empty() && gone.is_empty() {
        return Ok(0);
    }
    let tx = rusqlite::Transaction::new_unchecked(&copy, TransactionBehavior::Immediate)?;
    for hex in &back {
        tx.execute(
            "UPDATE images SET unreferenced_since = NULL WHERE id = ?1",
            [hex],
        )?;
    }
    let mut removed = 0;
    for hex in &gone {
        removed += tx.execute(
            "DELETE FROM images WHERE id = ?1 AND unreferenced_since IS NOT NULL",
            [hex],
        )? as u64;
    }
    tx.commit()?;
    if removed > 0 {
        give_back_free_pages(&copy);
    }
    Ok(removed)
}

/// Every image id that a kept daily backup of the library database at `database` names.
fn named_by_kept_backups(database: &Path) -> Result<HashSet<ImageId>> {
    let mut named = HashSet::new();
    for (_, backup) in list_backups(&backups_dir(database))? {
        // A backup is a file that was written once and renamed into place: it is read
        // as it is, without locks and without the files of a journal beside it.
        let conn = Connection::open_with_flags(
            immutable_uri(&backup),
            OpenFlags::SQLITE_OPEN_READ_ONLY
                | OpenFlags::SQLITE_OPEN_URI
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        named.extend(scan::referenced(&conn)?);
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
    use super::*;

    #[test]
    fn a_path_becomes_an_address_sqlite_reads_as_that_path() {
        assert_eq!(
            immutable_uri(Path::new("/a/The Writer/backups/library-17.db")),
            "file:/a/The%20Writer/backups/library-17.db?immutable=1"
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
        let odd = dir.path().join("The Writer 50%?#é");
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
        assert_eq!(named, HashSet::from([ImageId::parse(id).unwrap()]));
        // Reading a backup leaves nothing beside it.
        let mut names: Vec<String> = std::fs::read_dir(backups_dir(&database))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, vec!["library-1700000000000.db"]);
    }
}
