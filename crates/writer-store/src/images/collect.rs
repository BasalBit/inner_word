//! The collector: the only thing that ever deletes the bytes of an image.
//!
//! Nothing else does: not a purge, not emptying the trash, not the pruning of version
//! snapshots, not the `writer` tool. Deleting a document frees the space of its pictures
//! only later, and only through here.
//!
//! A run looks at what stored content names (`scan.rs`), stamps every image that nothing
//! names with the time of the run and clears the stamp of every image that is named. An
//! image goes when its stamp is older than the retention **and** older than the oldest
//! daily backup that is kept, so no kept backup of the library can name it.
//!
//! Who can hold an id, and why the collector never takes the image away from them:
//!
//! | Holder | Why it is safe |
//! |---|---|
//! | a live or trashed document, a version | the scan finds it |
//! | the undo history of an open editor, a Vim register, a cut waiting to be pasted | only the run at launch deletes ([`collect_unused_images`]); the look that is repeated while the app stays open ([`mark_unused_images`]) deletes nothing, and at a launch histories and registers are empty |
//! | the system clipboard, an old answer of the assistant | every way in from there goes through [`crate::Store::touch_image`], which clears the stamp; an image that is gone shows as missing |
//! | a daily backup of the library database | an image goes only when it was already unnamed when the oldest kept backup was written |
//!
//! **A stamp must not grow old on an image that is named.** Only a run that sees the
//! image named again (or a put, or a touch) clears a stamp, and an image can be named
//! again without either: an undo brings back an image that was cut, and nothing asks the
//! store. So whoever keeps a library open for long runs [`mark_unused_images`] once a
//! day: then no stamp is more than a day older than the moment its image was last named.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, TransactionBehavior, params};

use super::file::{self, Mode};
use super::{ImageId, copy, scan};
use crate::backup::list_backups;
use crate::error::Result;
use crate::paths::{backups_dir, images_path};
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
}

/// The collector: looks at what is named, stamps, and deletes what has been unnamed for
/// `retention_ms` ([`IMAGE_RETENTION_MS`]) and since before the oldest kept backup.
///
/// Opens connections of its own (the library database read-only, `images.db` for
/// writing), so it runs on any thread while a [`crate::Store`] of the same process is
/// open. Does nothing when there is no `images.db`, or no image in it.
///
/// **Run it once per launch and early**, before anything of the launch could hold an id
/// outside stored content for long (the undo history of an editor, a register): those
/// are not seen by the look. For a session that lasts there is [`mark_unused_images`].
/// The `writer` tool never collects.
///
/// An image that is deleted here is deleted from the copy of the pictures in the backups
/// directory too, unless a kept backup of the library database names it
/// ([`crate::back_up_images`]); and the run ends by bringing that copy up to date.
pub fn collect_unused_images(
    database: &Path,
    now_ms: i64,
    retention_ms: i64,
) -> Result<ImagesCollected> {
    let Some(mut run) = Run::look(database, now_ms)? else {
        return Ok(ImagesCollected::default());
    };
    let unused = run.unused(now_ms, retention_ms)?;
    run.delete(&unused, now_ms)?;
    let collected = run.collected;
    // The copy is a second line of safety, and the collection is done: what goes wrong
    // with the copy is found by whoever asks for it directly (`back_up_images`).
    let _ = copy::back_up_all_images(database);
    Ok(collected)
}

/// The looking half of the collector alone: clears the stamp of every image that is
/// named and stamps what is not. Deletes nothing (`deleted` is 0), whatever the age of a
/// stamp. For a session that lasts: once a day for as long as the app stays open.
///
/// Like [`collect_unused_images`] it ends by bringing the copy of the pictures in the
/// backups directory up to date.
pub fn mark_unused_images(database: &Path, now_ms: i64) -> Result<ImagesCollected> {
    let Some(run) = Run::look(database, now_ms)? else {
        return Ok(ImagesCollected::default());
    };
    let collected = run.collected;
    let _ = copy::back_up_all_images(database);
    Ok(collected)
}

/// An image the collector is about to delete: its id, the stamp it was read with, and how
/// many bytes it has.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Unused {
    id: ImageId,
    since: i64,
    byte_len: u64,
}

/// One run of the collector, after it looked.
struct Run {
    database: PathBuf,
    /// `images.db`, for writing.
    images: Connection,
    /// Every image id stored content named when the run looked.
    named: HashSet<ImageId>,
    collected: ImagesCollected,
}

impl Run {
    /// Steps 1 to 3: `None` when there is no `images.db` or no image in it; otherwise
    /// what stored content names is read, and in one transaction the stamp of every
    /// image that is named is cleared and every unnamed image without a stamp gets one.
    fn look(database: &Path, now_ms: i64) -> Result<Option<Run>> {
        let Some(images) = file::open(&images_path(database), Mode::Write)? else {
            return Ok(None);
        };
        let any: bool =
            images.query_row("SELECT EXISTS (SELECT 1 FROM images)", [], |r| r.get(0))?;
        // Nothing to look at, unless the copy of the pictures still holds an image an
        // earlier run deleted here and a backup named: that one is looked at again.
        if !any && !copy::holds_collected(database) {
            return Ok(None);
        }
        let named = named_in(database)?;

        let mut collected = ImagesCollected::default();
        let tx = rusqlite::Transaction::new_unchecked(&images, TransactionBehavior::Immediate)?;
        let mut clear = Vec::new();
        let mut stamp = Vec::new();
        {
            let mut stmt = tx.prepare("SELECT id, unreferenced_since FROM images")?;
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                let hex: String = row.get(0)?;
                let since: Option<i64> = row.get(1)?;
                collected.images += 1;
                // A row whose id cannot be read is named by nothing that can be found.
                let is_named = ImageId::parse(&hex).is_some_and(|id| named.contains(&id));
                if is_named {
                    collected.referenced += 1;
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
                "UPDATE images SET unreferenced_since = NULL WHERE id = ?1",
                [hex],
            )?;
        }
        for hex in &stamp {
            tx.execute(
                "UPDATE images SET unreferenced_since = ?2
                 WHERE id = ?1 AND unreferenced_since IS NULL",
                params![hex, now_ms],
            )?;
        }
        tx.commit()?;
        collected.newly_unreferenced = stamp.len() as u64;

        Ok(Some(Run {
            database: database.to_path_buf(),
            images,
            named,
            collected,
        }))
    }

    /// The first part of step 4: the images that may go. Unnamed, with a stamp older than
    /// the retention and older than the oldest daily backup that is kept.
    fn unused(&self, now_ms: i64, retention_ms: i64) -> Result<Vec<Unused>> {
        let mut cutoff = now_ms.saturating_sub(retention_ms.max(0));
        if let Some((oldest, _)) = list_backups(&backups_dir(&self.database))?.last() {
            cutoff = cutoff.min(*oldest);
        }
        let mut stmt = self.images.prepare(
            "SELECT id, unreferenced_since, byte_len FROM images
             WHERE unreferenced_since IS NOT NULL AND unreferenced_since < ?1
             ORDER BY rowid",
        )?;
        let mut rows = stmt.query([cutoff])?;
        let mut unused = Vec::new();
        while let Some(row) = rows.next()? {
            let hex: String = row.get(0)?;
            let Some(id) = ImageId::parse(&hex) else {
                continue;
            };
            if self.named.contains(&id) {
                continue;
            }
            let byte_len: i64 = row.get(2)?;
            unused.push(Unused {
                id,
                since: row.get(1)?,
                byte_len: u64::try_from(byte_len).unwrap_or(0),
            });
        }
        Ok(unused)
    }

    /// The rest of step 4, and step 5: deletes each image that still carries the stamp
    /// it was read with, so a put or a touch that came in meanwhile wins; gives the freed
    /// pages back to the file system; and takes the same images out of the copy of the
    /// pictures, where no kept backup names them.
    fn delete(&mut self, unused: &[Unused], now_ms: i64) -> Result<()> {
        if !unused.is_empty() {
            // The copy first: a row that is marked there and still here loses the mark at
            // the next look at the copy, while a row deleted here without a mark there
            // would stay in the copy for good.
            let ids: Vec<ImageId> = unused.iter().map(|image| image.id).collect();
            let _ = copy::mark_collected(&self.database, &ids, now_ms);

            let tx =
                rusqlite::Transaction::new_unchecked(&self.images, TransactionBehavior::Immediate)?;
            for image in unused {
                let deleted = tx.execute(
                    "DELETE FROM images WHERE id = ?1 AND unreferenced_since = ?2",
                    params![image.id.to_hex(), image.since],
                )?;
                if deleted > 0 {
                    self.collected.deleted += 1;
                    self.collected.bytes_freed += image.byte_len;
                }
            }
            tx.commit()?;
        }
        if self.collected.deleted > 0 {
            give_back_free_pages(&self.images);
        }
        // Also without a deletion of this run: an image an earlier run left in the copy
        // because a backup named it goes once that backup is no longer kept.
        let _ = copy::remove_collected(&self.database, &self.images, &self.named);
        Ok(())
    }
}

/// Every image id that the content stored in the library database at `database` names,
/// read through a connection of its own, in one read transaction.
fn named_in(database: &Path) -> Result<HashSet<ImageId>> {
    let library = Connection::open_with_flags(
        database,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    schema::configure(&library)?;
    // A library of a newer version may name images in ways this scan does not know.
    schema::inspect(&library)?;
    let tx = library.unchecked_transaction()?;
    let named = scan::referenced(&tx)?;
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
            unused.iter().map(|image| image.id).collect::<Vec<_>>(),
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
