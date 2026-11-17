//! The collector: the only thing that ever deletes the bytes of an image or of a file of a
//! page.
//!
//! Nothing else does: not a purge, not emptying the trash, not the pruning of version
//! snapshots, not the `innerword` tool. Deleting a document frees the space of its pictures,
//! and deleting a page the space of its files, only later, and only through here.
//!
//! A run looks at what stored content names (`scan.rs`), stamps every image and every file
//! that nothing names with the time of the run and clears the stamp of every one that is
//! named. A row goes when its stamp is older than the retention **and** older than the
//! oldest daily backup that is kept, so no kept backup of the library can name it. The
//! two tables of `images.db` (`images`, `files`) are looked at in the same run by the same
//! rules.
//!
//! Who can hold an id, and why the collector never takes the bytes away from them:
//!
//! | Holder | Why it is safe |
//! |---|---|
//! | a live or trashed document, a version | the scan finds it |
//! | a page's manifest, live, trashed or in a version | the scan finds it (`"file"`); nothing else holds the id of a file: no undo history, no clipboard, no register names one |
//! | the undo history of an open editor, a Vim register, a cut waiting to be pasted | only the run at launch deletes ([`collect_unused_images`]); the look that is repeated while the app stays open ([`mark_unused_images`]) deletes nothing, and at a launch histories and registers are empty |
//! | the system clipboard, an old answer of the assistant | every way in from there goes through [`crate::Store::touch_image`], which clears the stamp; an image that is gone shows as missing |
//! | a daily backup of the library database | an image goes only when it was already unnamed when the oldest kept backup was written |
//!
//! **A stamp must not grow old on an image that is named.** Only a run that sees the
//! image named again (or a put, or a touch) clears a stamp, and an image can be named
//! again without either: an undo brings back an image that was cut, and nothing asks the
//! store. So whoever keeps a library open for long runs [`mark_unused_images`] once a
//! day: then no stamp is more than a day older than the moment its image was last named.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, TransactionBehavior, params};

use super::file::{self, Mode, Table};
use super::scan::{self, Named};
use super::{FileId, ImageId, copy, parse16};
use crate::backup::list_backups;
use crate::error::Result;
use crate::paths::{backups_dir, images_backup_path, images_path};
use crate::schema;

/// How long an image that nothing names is kept before the collector may delete it: 30
/// days.
pub const IMAGE_RETENTION_MS: i64 = 30 * 24 * 60 * 60 * 1000;

/// What a run of the collector found and did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImagesCollected {
    /// How many images the file held when the run looked.
    pub images: u64,
    /// How many of them stored content names.
    pub referenced: u64,
    /// How many were stamped by this run: unnamed, and not stamped before.
    pub newly_unreferenced: u64,
    /// How many were deleted. Always 0 for [`mark_unused_images`].
    pub deleted: u64,
    /// The bytes of the deleted images.
    pub bytes_freed: u64,
    /// How many files of pages the file held when the run looked.
    pub files: u64,
    /// How many of them stored content names (the manifest of a page, live, trashed or
    /// in a version).
    pub files_referenced: u64,
    /// How many files were stamped by this run: unnamed, and not stamped before.
    pub files_newly_unreferenced: u64,
    /// How many files were deleted. Always 0 for [`mark_unused_images`].
    pub files_deleted: u64,
    /// The bytes of the deleted files.
    pub file_bytes_freed: u64,
    /// Why the copy of the pictures in the backups directory could not be brought up to
    /// date at the end of the run, when it could not ([`crate::back_up_all_images`]):
    /// the collection itself is done, and the copy is behind until a later run brings it
    /// up to date. A damaged copy is no such reason: it is made again.
    pub copy_failed: Option<String>,
}

/// The counts of one table in an [`ImagesCollected`].
struct Counts<'a> {
    held: &'a mut u64,
    referenced: &'a mut u64,
    newly_unreferenced: &'a mut u64,
    deleted: &'a mut u64,
    bytes_freed: &'a mut u64,
}

impl ImagesCollected {
    fn of(&mut self, table: Table) -> Counts<'_> {
        match table {
            Table::Images => Counts {
                held: &mut self.images,
                referenced: &mut self.referenced,
                newly_unreferenced: &mut self.newly_unreferenced,
                deleted: &mut self.deleted,
                bytes_freed: &mut self.bytes_freed,
            },
            Table::Files => Counts {
                held: &mut self.files,
                referenced: &mut self.files_referenced,
                newly_unreferenced: &mut self.files_newly_unreferenced,
                deleted: &mut self.files_deleted,
                bytes_freed: &mut self.file_bytes_freed,
            },
        }
    }
}

/// The collector: looks at what is named, stamps, and deletes what has been unnamed for
/// `retention_ms` ([`IMAGE_RETENTION_MS`]) and since before the oldest kept backup: images
/// and the files of pages alike.
///
/// Opens connections of its own (the library database read-only, `images.db` for
/// writing), so it runs on any thread while a [`crate::Store`] of the same process is
/// open. Does nothing when there is no `images.db`, or no image and no file in it.
///
/// **Run it once per launch and early**, before anything of the launch could hold an id
/// outside stored content for long (the undo history of an editor, a register): those
/// are not seen by the look. For a session that lasts there is [`mark_unused_images`].
/// The `innerword` tool never collects.
///
/// An image or a file that is deleted here is deleted from the copy in the backups
/// directory too, unless a kept backup of the library database names it
/// ([`crate::back_up_images`]); and the run ends by bringing that copy up to date.
pub fn collect_unused_images(
    database: &Path,
    now_ms: i64,
    retention_ms: i64,
) -> Result<ImagesCollected> {
    let Some(mut run) = Run::look_mending(database, now_ms)? else {
        return Ok(with_the_copy(database, ImagesCollected::default()));
    };
    let unused = run.unused(now_ms, retention_ms)?;
    run.delete(&unused, now_ms)?;
    Ok(with_the_copy(database, run.collected))
}

/// Brings the copy of the pictures up to date after a run, and says in `collected` when
/// that failed: the copy is a second line of safety, and the collection is done.
fn with_the_copy(database: &Path, mut collected: ImagesCollected) -> ImagesCollected {
    if let Err(error) = copy::back_up_all_images(database) {
        collected.copy_failed = Some(error.to_string());
    }
    collected
}

/// The looking half of the collector alone: clears the stamp of every image that is
/// named and stamps what is not. Deletes nothing (`deleted` is 0), whatever the age of a
/// stamp. For a session that lasts: once a day for as long as the app stays open.
///
/// Like [`collect_unused_images`] it ends by bringing the copy of the pictures in the
/// backups directory up to date.
pub fn mark_unused_images(database: &Path, now_ms: i64) -> Result<ImagesCollected> {
    let Some(run) = Run::look_mending(database, now_ms)? else {
        return Ok(with_the_copy(database, ImagesCollected::default()));
    };
    Ok(with_the_copy(database, run.collected))
}

/// An image or a file the collector is about to delete: its table, its id, the stamp it
/// was read with, and how many bytes it has.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Unused {
    table: Table,
    id: [u8; 16],
    since: i64,
    byte_len: u64,
}

impl Unused {
    fn hex(&self) -> String {
        match self.table {
            Table::Images => ImageId(self.id).to_hex(),
            Table::Files => FileId(self.id).to_hex(),
        }
    }
}

/// One run of the collector, after it looked.
struct Run {
    database: PathBuf,
    /// `images.db`, for writing.
    images: Connection,
    /// Every image id and file id stored content named when the run looked.
    named: Named,
    collected: ImagesCollected,
}

impl Run {
    /// [`look`](Self::look); when `images.db` turns out damaged, it is moved aside and a
    /// new one is made ([`file::replace_damaged`]: its pictures come back from the copy),
    /// and the new one is looked at. A damaged library database is an error, as it was.
    fn look_mending(database: &Path, now_ms: i64) -> Result<Option<Run>> {
        match Run::look(database, now_ms) {
            Err(error) if file::is_damage(&error) => {
                let images = images_path(database);
                if !file::is_damaged(&images) {
                    return Err(error);
                }
                file::replace_damaged(
                    &images,
                    &images_backup_path(database),
                    file::file_identity(&images),
                )?;
                Run::look(database, now_ms)
            }
            other => other,
        }
    }

    /// Steps 1 to 3: `None` when there is no `images.db` or no image and no file in it;
    /// otherwise what stored content names is read, and in one transaction the stamp of
    /// every image and file that is named is cleared and every unnamed one without a stamp
    /// gets one.
    fn look(database: &Path, now_ms: i64) -> Result<Option<Run>> {
        // Opened for writing: a file of version 1 is brought to version 2 here.
        let Some(images) = file::open(&images_path(database), Mode::Write)? else {
            return Ok(None);
        };
        let mut any = false;
        for table in Table::BOTH {
            any = any
                || (table.is_in(&images)?
                    && images.query_row(
                        &format!("SELECT EXISTS (SELECT 1 FROM {})", table.name()),
                        [],
                        |r| r.get(0),
                    )?);
        }
        // Nothing to look at, unless the copy still holds a row an earlier run deleted
        // here and a backup named: that one is looked at again.
        if !any && !copy::holds_collected(database) {
            return Ok(None);
        }
        let named = named_in(database)?;

        let mut collected = ImagesCollected::default();
        let tx = rusqlite::Transaction::new_unchecked(&images, TransactionBehavior::Immediate)?;
        for table in Table::BOTH {
            if !table.is_in(&tx)? {
                continue;
            }
            let counts = collected.of(table);
            let mut clear = Vec::new();
            let mut stamp = Vec::new();
            {
                let mut stmt = tx.prepare(&format!(
                    "SELECT id, unreferenced_since FROM {}",
                    table.name()
                ))?;
                let mut rows = stmt.query([])?;
                while let Some(row) = rows.next()? {
                    let hex: String = row.get(0)?;
                    let since: Option<i64> = row.get(1)?;
                    *counts.held += 1;
                    // A row whose id cannot be read is named by nothing that can be found.
                    if named.names_hex(table, &hex) {
                        *counts.referenced += 1;
                        if since.is_some() {
                            clear.push(hex);
                        }
                    } else if since.is_none() {
                        stamp.push(hex);
                    }
                }
            }
            for hex in &clear {
                tx.execute(
                    &format!(
                        "UPDATE {} SET unreferenced_since = NULL WHERE id = ?1",
                        table.name()
                    ),
                    [hex],
                )?;
            }
            for hex in &stamp {
                tx.execute(
                    &format!(
                        "UPDATE {} SET unreferenced_since = ?2
                         WHERE id = ?1 AND unreferenced_since IS NULL",
                        table.name()
                    ),
                    params![hex, now_ms],
                )?;
            }
            *counts.newly_unreferenced = stamp.len() as u64;
        }
        tx.commit()?;

        Ok(Some(Run {
            database: database.to_path_buf(),
            images,
            named,
            collected,
        }))
    }

    /// The first part of step 4: the images and files that may go. Unnamed, with a stamp
    /// older than the retention and older than the oldest daily backup that is kept.
    fn unused(&self, now_ms: i64, retention_ms: i64) -> Result<Vec<Unused>> {
        let mut cutoff = now_ms.saturating_sub(retention_ms.max(0));
        if let Some((oldest, _)) = list_backups(&backups_dir(&self.database))?.last() {
            cutoff = cutoff.min(*oldest);
        }
        let mut unused = Vec::new();
        for table in Table::BOTH {
            if !table.is_in(&self.images)? {
                continue;
            }
            let mut stmt = self.images.prepare(&format!(
                "SELECT id, unreferenced_since, byte_len FROM {}
                 WHERE unreferenced_since IS NOT NULL AND unreferenced_since < ?1
                 ORDER BY rowid",
                table.name()
            ))?;
            let mut rows = stmt.query([cutoff])?;
            while let Some(row) = rows.next()? {
                let hex: String = row.get(0)?;
                let Some(id) = parse16(hex.as_bytes()) else {
                    continue;
                };
                if self.named.names(table, id) {
                    continue;
                }
                let byte_len: i64 = row.get(2)?;
                unused.push(Unused {
                    table,
                    id,
                    since: row.get(1)?,
                    byte_len: u64::try_from(byte_len).unwrap_or(0),
                });
            }
        }
        Ok(unused)
    }

    /// The rest of step 4, and step 5: deletes each image and file that still carries the
    /// stamp it was read with, so a put or a touch that came in meanwhile wins; gives the
    /// freed pages back to the file system; and takes the same rows out of the copy in the
    /// backups directory, where no kept backup names them.
    fn delete(&mut self, unused: &[Unused], now_ms: i64) -> Result<()> {
        if !unused.is_empty() {
            // The copy first: a row that is marked there and still here loses the mark at
            // the next look at the copy, while a row deleted here without a mark there
            // would stay in the copy for good.
            let rows: Vec<(Table, String)> =
                unused.iter().map(|row| (row.table, row.hex())).collect();
            let _ = copy::mark_collected(&self.database, &rows, now_ms);

            let tx =
                rusqlite::Transaction::new_unchecked(&self.images, TransactionBehavior::Immediate)?;
            for row in unused {
                let deleted = tx.execute(
                    &format!(
                        "DELETE FROM {} WHERE id = ?1 AND unreferenced_since = ?2",
                        row.table.name()
                    ),
                    params![row.hex(), row.since],
                )?;
                if deleted > 0 {
                    let counts = self.collected.of(row.table);
                    *counts.deleted += 1;
                    *counts.bytes_freed += row.byte_len;
                }
            }
            tx.commit()?;
        }
        if self.collected.deleted + self.collected.files_deleted > 0 {
            give_back_free_pages(&self.images);
        }
        // Also without a deletion of this run: a row an earlier run left in the copy
        // because a backup named it goes once that backup is no longer kept.
        let _ = copy::remove_collected(&self.database, &self.images, &self.named);
        Ok(())
    }
}

/// Every image id and file id that the content stored in the library database at
/// `database` names, read through a connection of its own, in one read transaction.
fn named_in(database: &Path) -> Result<Named> {
    let library = Connection::open_with_flags(
        database,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    schema::configure(&library)?;
    // A library of a newer version may name images or files in ways this scan does not
    // know.
    schema::inspect(&library)?;
    let tx = library.unchecked_transaction()?;
    let named = scan::referenced_all(&tx)?;
    tx.commit()?;
    Ok(named)
}

/// Shrinks the file by the pages a deletion freed. The file itself gets smaller when the
/// WAL is written back into it, which another connection that is reading can put off:
/// then it happens at a later checkpoint, and nothing is wrong.
pub(crate) fn give_back_free_pages(conn: &Connection) {
    // The pragma answers with a row for every page it frees and frees no more than it
    // is asked rows for.
    let freed = || -> rusqlite::Result<()> {
        let mut stmt = conn.prepare("PRAGMA incremental_vacuum")?;
        let mut rows = stmt.query([])?;
        while rows.next()?.is_some() {}
        Ok(())
    };
    let _ = freed();
    let _ = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NewImage, Store};

    const DAY: i64 = 24 * 60 * 60 * 1000;
    const START: i64 = 1_700_000_000_000;

    fn picture(seed: u8) -> Vec<u8> {
        let mut bytes = vec![0x89, b'P', b'N', b'G'];
        bytes.extend(std::iter::repeat_n(seed, 64));
        bytes
    }

    fn new_image(bytes: &[u8]) -> NewImage<'_> {
        NewImage {
            bytes,
            format: crate::ImageFormat::Png,
            width: 4,
            height: 3,
            density: 72,
        }
    }

    /// Between the moment the collector reads which images may go and the moment it
    /// deletes them, a touch or a put clears the stamp: the delete names the stamp it
    /// read, finds another value and leaves the image.
    #[test]
    fn a_touch_or_a_put_between_reading_and_deleting_wins() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.db");
        let mut store = Store::open(&path).unwrap();
        let touched = store.put_image(&new_image(&picture(1))).unwrap().id;
        let put_again = store.put_image(&new_image(&picture(2))).unwrap().id;
        let left_alone = store.put_image(&new_image(&picture(3))).unwrap().id;

        // Stamped by a first run, and 31 days later old enough to go.
        let first = mark_unused_images(&path, START).unwrap();
        assert_eq!(first.newly_unreferenced, 3);
        let later = START + 31 * DAY;
        let mut run = Run::look(&path, later).unwrap().unwrap();
        let unused = run.unused(later, IMAGE_RETENTION_MS).unwrap();
        assert_eq!(
            unused
                .iter()
                .map(|image| ImageId(image.id))
                .collect::<Vec<_>>(),
            vec![touched, put_again, left_alone]
        );

        assert!(store.touch_image(&touched).unwrap().is_some());
        store.put_image(&new_image(&picture(2))).unwrap();

        run.delete(&unused, later).unwrap();
        assert_eq!(run.collected.deleted, 1);
        assert_eq!(run.collected.bytes_freed, picture(3).len() as u64);
        assert!(store.image_info(&touched).unwrap().is_some());
        assert!(store.image_info(&put_again).unwrap().is_some());
        assert!(store.image_info(&left_alone).unwrap().is_none());
        assert_eq!(store.image_count().unwrap(), 2);
    }

    /// A touch reads the row and then clears its stamp, in two statements; when the
    /// collector's delete comes in between (it has the stamp the touch read), the touch
    /// does not say that the image is there.
    #[test]
    fn a_delete_between_the_read_and_the_update_of_a_touch_is_seen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.db");
        let mut store = Store::open(&path).unwrap();
        let id = store.put_image(&new_image(&picture(1))).unwrap().id;
        mark_unused_images(&path, START).unwrap();
        let later = START + 31 * DAY;
        let mut run = Run::look(&path, later).unwrap().unwrap();
        let unused = run.unused(later, IMAGE_RETENTION_MS).unwrap();
        assert_eq!(unused.len(), 1);

        let touched = store
            .images
            .touch_image_between(&id, &mut || run.delete(&unused, later).unwrap())
            .unwrap();
        assert_eq!(run.collected.deleted, 1);
        assert_eq!(touched, None);
        assert!(store.image_info(&id).unwrap().is_none());
    }

    /// The same when the stamp was cleared and set again in between (another run of the
    /// look, a day later): the stamp is a different one, the image stays.
    #[test]
    fn a_stamp_that_was_set_anew_is_not_the_one_that_was_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.db");
        let mut store = Store::open(&path).unwrap();
        let id = store.put_image(&new_image(&picture(1))).unwrap().id;
        mark_unused_images(&path, START).unwrap();
        let later = START + 31 * DAY;
        let mut run = Run::look(&path, later).unwrap().unwrap();
        let unused = run.unused(later, IMAGE_RETENTION_MS).unwrap();
        assert_eq!(unused.len(), 1);

        store.touch_image(&id).unwrap();
        mark_unused_images(&path, later + 1).unwrap();

        run.delete(&unused, later).unwrap();
        assert_eq!(run.collected.deleted, 0);
        assert!(store.image_info(&id).unwrap().is_some());
    }
}
