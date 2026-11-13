//! The bytes of the images and of the files of pages: storing and reading them, the
//! second file, which images and files stored content names, the collector, a connection
//! of its own to the file, and the copy in the backups directory.

mod common;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use common::*;
use writer_store::{
    Clock, DocumentMeta, FileId, FileInfo, IMAGE_RETENTION_MS, ImageFile, ImageFormat, ImageId,
    ImageInfo, ImagesBackedUp, ImagesCollected, NewImage, OpenOptions, PageContent, Store,
    StoreError, back_up_all_images, back_up_images, backups_dir, collect_unused_images,
    images_backup_path, images_moved_aside, images_path, mark_unused_images,
};

// ----- helpers -------------------------------------------------------------------------

/// Bytes that stand for a picture file: the store hashes them and does not decode them.
fn picture(seed: u8, len: usize) -> Vec<u8> {
    let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    bytes.extend((0..len).map(|i| seed.wrapping_add((i % 251) as u8)));
    bytes
}

fn png(bytes: &[u8]) -> NewImage<'_> {
    NewImage {
        bytes,
        format: ImageFormat::Png,
        width: 1600,
        height: 1067,
        density: 144,
    }
}

/// The content of a document that holds these images, as `writer-core` writes it.
fn content_with(ids: &[ImageId]) -> String {
    let mut blocks = vec![r#"{"id":1,"type":"paragraph","text":"Before"}"#.to_owned()];
    for (index, id) in ids.iter().enumerate() {
        blocks.push(format!(
            r#"{{"id":{},"type":"image","image":"{id}","px":[1600,1067],"ppi":144,"width":500}}"#,
            index + 2
        ));
    }
    format!(r#"{{"version":2,"blocks":[{}]}}"#, blocks.join(","))
}

fn document_with(store: &mut Store, title: &str, ids: &[ImageId]) -> DocumentMeta {
    let json = content_with(ids);
    store
        .create_document(None, &content(title, "Before", &json))
        .unwrap()
}

/// A library on disk in a temporary directory, with a manual clock and no daily backup
/// (a test that wants one writes it).
struct OnDisk {
    dir: tempfile::TempDir,
    clock: Clock,
}

impl OnDisk {
    fn new() -> OnDisk {
        OnDisk {
            dir: tempfile::tempdir().unwrap(),
            clock: Clock::manual(START),
        }
    }

    fn database(&self) -> PathBuf {
        self.dir.path().join("library.db")
    }

    fn images(&self) -> PathBuf {
        images_path(&self.database())
    }

    fn copy(&self) -> PathBuf {
        images_backup_path(&self.database())
    }

    fn open(&self) -> Store {
        Store::open_with(
            self.database(),
            OpenOptions {
                clock: self.clock.clone(),
                daily_backup: false,
                ..OpenOptions::default()
            },
        )
        .unwrap()
    }

    /// Runs the collector as the app does at a launch, at the clock's time.
    fn collect(&self) -> ImagesCollected {
        collect_unused_images(&self.database(), self.clock.now_ms(), IMAGE_RETENTION_MS).unwrap()
    }

    /// Runs the looking half, at the clock's time.
    fn look(&self) -> ImagesCollected {
        mark_unused_images(&self.database(), self.clock.now_ms()).unwrap()
    }

    /// A daily backup of the library as it is now, stamped with the clock's time.
    fn write_backup(&self, store: &Store) -> PathBuf {
        let dir = backups_dir(&self.database());
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("library-{}.db", self.clock.now_ms()));
        store.backup_to(&path).unwrap();
        path
    }
}

/// The collector's stamp on an image as the file at `images` has it; `None` when the
/// file does not have the image.
fn stamp(images: &Path, id: &ImageId) -> Option<Option<i64>> {
    use rusqlite::OptionalExtension;
    let conn = rusqlite::Connection::open(images).unwrap();
    conn.query_row(
        "SELECT unreferenced_since FROM images WHERE id = ?1",
        [id.to_hex()],
        |row| row.get(0),
    )
    .optional()
    .unwrap()
}

/// The ids an images database holds, sorted.
fn ids_in(images: &Path) -> Vec<String> {
    let conn = rusqlite::Connection::open(images).unwrap();
    let mut stmt = conn.prepare("SELECT id FROM images ORDER BY id").unwrap();
    stmt.query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

fn sorted(ids: &[ImageId]) -> Vec<String> {
    let mut hex: Vec<String> = ids.iter().map(ImageId::to_hex).collect();
    hex.sort();
    hex
}

/// What changes whenever another connection commits a change to the database.
fn data_version(conn: &rusqlite::Connection) -> i64 {
    conn.query_row("PRAGMA data_version", [], |r| r.get(0))
        .unwrap()
}

/// Bytes that stand for a file of a page (a font, a sound): the store hashes them. No two
/// seeds give bytes with a stretch in common, so a page of the file is one file's.
fn page_file(seed: u8, len: usize) -> Vec<u8> {
    let mut bytes = b"wOF2".to_vec();
    let mut state = u64::from(seed).wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    bytes.extend((0..len).map(|_| {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (state >> 33) as u8
    }));
    bytes
}

/// The manifest of a page that serves these files, as `writer-page` writes it.
fn manifest_with(ids: &[FileId]) -> String {
    let files: Vec<String> = ids
        .iter()
        .enumerate()
        .map(|(index, id)| {
            format!(r#"{{"path":"f{index}.woff2","file":"{id}","type":"font/woff2","size":1}}"#)
        })
        .collect();
    format!(
        r#"{{"page":1,"entry":"index.html","files":[{}],"original":[],"externals":[],"source_url":null,"fingerprint":"","added_at":0,"checked":{{"engine":"WebKit","at":0,"requests":0,"warnings":[]}},"text":"A page"}}"#,
        files.join(",")
    )
}

fn page_content(manifest: &str) -> PageContent<'_> {
    PageContent {
        manifest,
        title: "A page",
        plain_text: "A page",
        word_count: 2,
        char_count: 6,
        source_url: None,
    }
}

fn page_with(store: &mut Store, ids: &[FileId]) -> DocumentMeta {
    let manifest = manifest_with(ids);
    store.create_page(None, &page_content(&manifest)).unwrap()
}

/// The collector's stamp on a file of a page as the file at `images` has it.
fn file_stamp(images: &Path, id: &FileId) -> Option<Option<i64>> {
    use rusqlite::OptionalExtension;
    let conn = rusqlite::Connection::open(images).unwrap();
    conn.query_row(
        "SELECT unreferenced_since FROM files WHERE id = ?1",
        [id.to_hex()],
        |row| row.get(0),
    )
    .optional()
    .unwrap()
}

/// The ids of files of pages an images database holds, sorted.
fn file_ids_in(images: &Path) -> Vec<String> {
    let conn = rusqlite::Connection::open(images).unwrap();
    let mut stmt = conn.prepare("SELECT id FROM files ORDER BY id").unwrap();
    stmt.query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

fn sorted_files(ids: &[FileId]) -> Vec<String> {
    let mut hex: Vec<String> = ids.iter().map(FileId::to_hex).collect();
    hex.sort();
    hex
}

fn user_version(path: &Path) -> i64 {
    let conn = rusqlite::Connection::open(path).unwrap();
    conn.query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap()
}

fn has_table(path: &Path, name: &str) -> bool {
    let conn = rusqlite::Connection::open(path).unwrap();
    conn.query_row(
        "SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = ?1",
        [name],
        |r| r.get::<_, i64>(0),
    )
    .unwrap()
        == 1
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

// ----- storing and reading -------------------------------------------------------------

#[test]
fn an_image_is_stored_once_under_the_hash_of_its_bytes() {
    let (mut store, clock) = store();
    assert_eq!(store.image_count().unwrap(), 0);

    // The id is the first 16 bytes of the SHA-256: the published digest of "abc".
    let first = store
        .put_image(&NewImage {
            bytes: b"abc",
            format: ImageFormat::Gif,
            width: 3,
            height: 2,
            density: 72,
        })
        .unwrap();
    assert_eq!(first.id.to_hex(), "ba7816bf8f01cfea414140de5dae2223");
    assert_eq!(first.id, ImageId::of(b"abc"));
    assert_eq!(
        first,
        ImageInfo {
            id: ImageId::of(b"abc"),
            format: ImageFormat::Gif,
            width: 3,
            height: 2,
            density: 72,
            byte_len: 3,
            created_at: START,
        }
    );

    // The same bytes again, later and described differently: the row that is there
    // stays as it is.
    clock.advance_ms(HOUR);
    let again = store
        .put_image(&NewImage {
            bytes: b"abc",
            format: ImageFormat::Png,
            width: 30,
            height: 20,
            density: 144,
        })
        .unwrap();
    assert_eq!(again, first);
    assert_eq!(store.image_count().unwrap(), 1);

    assert_eq!(
        store.image_bytes(&first.id).unwrap().as_deref(),
        Some(&b"abc"[..])
    );
    assert_eq!(store.image_info(&first.id).unwrap(), Some(first.clone()));
    assert_eq!(store.touch_image(&first.id).unwrap(), Some(first.clone()));

    // Other bytes are another image.
    let bytes = picture(7, 100_000);
    let second = store.put_image(&png(&bytes)).unwrap();
    assert_ne!(second.id, first.id);
    assert_eq!(second.byte_len, bytes.len() as u64);
    assert_eq!(second.created_at, START + HOUR);
    assert_eq!(
        (second.width, second.height, second.density),
        (1600, 1067, 144)
    );
    assert_eq!(store.image_bytes(&second.id).unwrap(), Some(bytes));
    assert_eq!(store.image_count().unwrap(), 2);

    // An image the library does not have.
    let unknown = ImageId::of(b"nothing has these bytes");
    assert_eq!(store.image_info(&unknown).unwrap(), None);
    assert_eq!(store.image_bytes(&unknown).unwrap(), None);
    assert_eq!(store.touch_image(&unknown).unwrap(), None);
}

#[test]
fn a_library_without_images_never_gets_the_second_file() {
    let lib = OnDisk::new();
    let unknown = ImageId::of(b"x");
    let mut store = lib.open();
    let d = doc(&mut store, None, "Words only", "no pictures here");

    // Reads, a touch of an image that is not there, the scan, the collector, the copy.
    assert_eq!(store.image_count().unwrap(), 0);
    assert_eq!(store.image_info(&unknown).unwrap(), None);
    assert_eq!(store.image_bytes(&unknown).unwrap(), None);
    assert_eq!(store.touch_image(&unknown).unwrap(), None);
    assert!(store.referenced_images().unwrap().is_empty());
    assert_eq!(lib.collect(), ImagesCollected::default());
    assert_eq!(lib.look(), ImagesCollected::default());
    assert_eq!(
        back_up_all_images(&lib.database()).unwrap(),
        ImagesBackedUp::default()
    );
    assert!(!lib.images().exists());

    // A reader beside it.
    let reader = Store::open_read_only(lib.database())
        .unwrap()
        .store()
        .unwrap();
    assert_eq!(reader.image_count().unwrap(), 0);
    assert_eq!(reader.image_bytes(&unknown).unwrap(), None);
    drop(reader);
    assert!(!lib.images().exists());

    // A save through a second store, as the app's background saver makes it.
    let mut saver = lib.open();
    saver
        .save_document(d.id, &content("Words only", "more words", "{}"))
        .unwrap();
    saver.snapshot_if_stale(d.id, Duration::ZERO).unwrap();
    saver.close().unwrap();
    store.close().unwrap();

    assert!(!lib.images().exists());
    assert!(!backups_dir(&lib.database()).exists());
    assert!(
        names_in(lib.dir.path())
            .iter()
            .all(|name| name.starts_with("library.db")),
        "{:?}",
        names_in(lib.dir.path())
    );
}

#[test]
fn the_second_file_is_made_by_the_first_put_with_its_own_header() {
    let lib = OnDisk::new();
    let mut store = lib.open();
    let bytes = picture(1, 5_000);
    let info = store.put_image(&png(&bytes)).unwrap();
    assert!(lib.images().exists());
    assert_eq!(lib.images(), lib.dir.path().join("images.db"));

    // What a second connection finds: the file says what it is, writes go through a WAL,
    // and freed pages can be given back.
    let conn = rusqlite::Connection::open(lib.images()).unwrap();
    let pragma = |name: &str| -> String {
        conn.query_row(&format!("PRAGMA {name}"), [], |r| {
            r.get::<_, rusqlite::types::Value>(0)
        })
        .map(|value| match value {
            rusqlite::types::Value::Integer(i) => i.to_string(),
            rusqlite::types::Value::Text(t) => t,
            other => format!("{other:?}"),
        })
        .unwrap()
    };
    assert_eq!(pragma("application_id"), 0x5749_4D47.to_string());
    assert_eq!(pragma("user_version"), "2");
    assert_eq!(pragma("journal_mode").to_lowercase(), "wal");
    assert_eq!(pragma("auto_vacuum"), "2");
    let (format, width, height, density, byte_len, created_at, since, stored): (
        String,
        i64,
        i64,
        i64,
        i64,
        i64,
        Option<i64>,
        Vec<u8>,
    ) = conn
        .query_row(
            "SELECT format, width, height, density, byte_len, created_at, unreferenced_since, bytes
             FROM images WHERE id = ?1",
            [info.id.to_hex()],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(format, "png");
    assert_eq!((width, height, density), (1600, 1067, 144));
    assert_eq!(byte_len, bytes.len() as i64);
    assert_eq!(created_at, START);
    assert_eq!(since, None);
    assert_eq!(stored, bytes);
    drop(conn);

    // The library database knows nothing of it: it is the documents, as before.
    let library = rusqlite::Connection::open(lib.database()).unwrap();
    let tables: i64 = library
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name = 'images'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(tables, 0);
    drop(library);

    // Closing the store writes the WAL of the second file back too.
    store.close().unwrap();
    let wal = lib.dir.path().join("images.db-wal");
    assert!(!wal.exists() || std::fs::metadata(&wal).unwrap().len() == 0);

    // And the image is there for the next one who opens the library.
    let store = lib.open();
    assert_eq!(store.image_bytes(&info.id).unwrap(), Some(bytes));
    assert_eq!(store.image_info(&info.id).unwrap(), Some(info));
}

#[test]
fn a_committed_image_survives_a_store_that_is_never_closed() {
    let lib = OnDisk::new();
    let mut store = lib.open();
    let bytes = picture(2, 40_000);
    let id = store.put_image(&png(&bytes)).unwrap().id;
    // A crash: nothing is closed, nothing is checkpointed.
    std::mem::forget(store);
    let store = lib.open();
    assert_eq!(store.image_bytes(&id).unwrap(), Some(bytes));
}

#[test]
fn a_dry_run_stores_nothing_and_creates_nothing() {
    let lib = OnDisk::new();
    let mut store = lib.open();
    let first = picture(1, 100);
    let second = picture(2, 100);

    // Before the library holds any image: the put fails and no file appears.
    store.set_query_only(true).unwrap();
    assert!(matches!(
        store.put_image(&png(&first)),
        Err(StoreError::Sqlite(_))
    ));
    assert_eq!(store.image_count().unwrap(), 0);
    assert_eq!(store.touch_image(&ImageId::of(&first)).unwrap(), None);
    assert!(!lib.images().exists());
    store.set_query_only(false).unwrap();
    let stored = store.put_image(&png(&first)).unwrap();

    // With the file there and open: the put of other bytes fails, the put of bytes that
    // are there answers (it writes nothing), reads go on.
    store.set_query_only(true).unwrap();
    assert!(matches!(
        store.put_image(&png(&second)),
        Err(StoreError::Sqlite(_))
    ));
    assert_eq!(store.put_image(&png(&first)).unwrap(), stored);
    assert_eq!(store.touch_image(&stored.id).unwrap(), Some(stored.clone()));
    assert_eq!(store.image_bytes(&stored.id).unwrap(), Some(first.clone()));
    assert_eq!(store.image_count().unwrap(), 1);
    assert_eq!(store.image_info(&ImageId::of(&second)).unwrap(), None);

    // A stamp is not cleared in a dry run either: the touch fails instead of writing.
    lib.look();
    assert_eq!(stamp(&lib.images(), &stored.id), Some(Some(START)));
    assert!(store.touch_image(&stored.id).is_err());
    assert!(store.put_image(&png(&first)).is_err());
    assert_eq!(stamp(&lib.images(), &stored.id), Some(Some(START)));

    store.set_query_only(false).unwrap();
    assert_eq!(store.touch_image(&stored.id).unwrap(), Some(stored.clone()));
    assert_eq!(stamp(&lib.images(), &stored.id), Some(None));
    store.put_image(&png(&second)).unwrap();
    assert_eq!(store.image_count().unwrap(), 2);

    // A store that opens the file only during the dry run refuses the same way.
    let mut other = lib.open();
    other.set_query_only(true).unwrap();
    assert_eq!(other.image_count().unwrap(), 2);
    assert!(other.put_image(&png(&picture(3, 100))).is_err());
    other.set_query_only(false).unwrap();
    other.put_image(&png(&picture(3, 100))).unwrap();
    assert_eq!(store.image_count().unwrap(), 3);
}

#[test]
fn a_read_only_store_reads_images_and_stores_none() {
    let lib = OnDisk::new();
    let mut app = lib.open();
    let bytes = picture(4, 2_000);
    let info = app.put_image(&png(&bytes)).unwrap();

    let mut reader = Store::open_read_only(lib.database())
        .unwrap()
        .store()
        .unwrap();
    assert_eq!(reader.image_count().unwrap(), 1);
    assert_eq!(reader.image_info(&info.id).unwrap(), Some(info.clone()));
    assert_eq!(reader.image_bytes(&info.id).unwrap(), Some(bytes));
    assert!(matches!(
        reader.put_image(&png(&picture(5, 10))),
        Err(StoreError::Sqlite(_))
    ));
    assert!(matches!(
        reader.touch_image(&info.id),
        Err(StoreError::Sqlite(_))
    ));
    assert_eq!(reader.referenced_images().unwrap(), HashSet::new());

    // What the app stores while the reader is open is seen by its next read.
    let later = app.put_image(&png(&picture(6, 10))).unwrap();
    assert_eq!(reader.image_info(&later.id).unwrap(), Some(later));
    assert_eq!(reader.image_count().unwrap(), 2);
    reader.close().unwrap();
    app.close().unwrap();
}

#[test]
fn a_file_that_is_not_an_images_database_is_refused_and_left_alone() {
    let lib = OnDisk::new();
    let mut store = lib.open();
    let id = ImageId::of(b"x");

    // An empty file (the making of it was cut short): there is no image, and the first
    // put makes it whole.
    std::fs::write(lib.images(), b"").unwrap();
    assert_eq!(store.image_count().unwrap(), 0);
    assert_eq!(store.image_bytes(&id).unwrap(), None);
    assert_eq!(std::fs::metadata(lib.images()).unwrap().len(), 0);
    let stored = store.put_image(&png(&picture(1, 10))).unwrap();
    assert_eq!(store.image_count().unwrap(), 1);
    store.close().unwrap();

    // Some other database under that name.
    std::fs::remove_file(lib.images()).unwrap();
    let conn = rusqlite::Connection::open(lib.images()).unwrap();
    conn.execute_batch("CREATE TABLE stuff (x); INSERT INTO stuff VALUES (1);")
        .unwrap();
    drop(conn);
    let before = std::fs::read(lib.images()).unwrap();
    let mut store = lib.open();
    assert!(matches!(store.image_count(), Err(StoreError::NotALibrary)));
    assert!(matches!(
        store.put_image(&png(&picture(1, 10))),
        Err(StoreError::NotALibrary)
    ));
    assert!(matches!(
        store.image_bytes(&stored.id),
        Err(StoreError::NotALibrary)
    ));
    drop(store);
    assert_eq!(std::fs::read(lib.images()).unwrap(), before);

    // The library database itself is not an images database.
    std::fs::copy(lib.database(), lib.images()).unwrap();
    let store = lib.open();
    assert!(matches!(store.image_count(), Err(StoreError::NotALibrary)));
    drop(store);

    // An images database of a later version.
    std::fs::remove_file(lib.images()).unwrap();
    let mut store = lib.open();
    store.put_image(&png(&picture(1, 10))).unwrap();
    store.close().unwrap();
    let conn = rusqlite::Connection::open(lib.images()).unwrap();
    conn.pragma_update(None, "user_version", 3).unwrap();
    drop(conn);
    let store = lib.open();
    match store.image_count() {
        Err(StoreError::SchemaTooNew { found, supported }) => {
            assert_eq!((found, supported), (3, 2))
        }
        other => panic!("expected SchemaTooNew, got {other:?}"),
    }
}

// ----- which images stored content names -----------------------------------------------

#[test]
fn the_images_that_stored_content_names_are_found() {
    let (mut store, clock) = store();
    let live = ImageId::of(b"in a live document");
    let twice = ImageId::of(b"in the same document, twice");
    let trashed = ImageId::of(b"in a trashed document");
    let version = ImageId::of(b"only in a version");
    let unnamed = ImageId::of(b"nowhere");

    document_with(&mut store, "Live", &[live, twice, twice]);
    let goes = document_with(&mut store, "Trashed", &[trashed]);
    store.trash_document(goes.id).unwrap();
    assert_eq!(
        store.referenced_images().unwrap(),
        HashSet::from([live, twice, trashed])
    );

    // A version taken by the autosave's snapshot names what the document named then.
    let edited = document_with(&mut store, "Edited", &[version]);
    assert!(
        store
            .snapshot_if_stale(edited.id, Duration::ZERO)
            .unwrap()
            .is_some()
    );
    clock.advance_ms(HOUR);
    store
        .save_document(edited.id, &content("Edited", "Before", &content_with(&[])))
        .unwrap();
    let named = store.referenced_images().unwrap();
    assert!(named.contains(&version));
    assert!(!named.contains(&unnamed));
    assert_eq!(named.len(), 4);

    // A duplicate is content copied inside the database: it names the same images, also
    // after the original is deleted for good.
    let original = document_with(&mut store, "Original", &[unnamed]);
    store.duplicate_document(original.id).unwrap();
    store.trash_document(original.id).unwrap();
    store.purge_document(original.id).unwrap();
    assert!(store.referenced_images().unwrap().contains(&unnamed));
}

#[test]
fn only_the_token_of_an_image_block_counts() {
    let (mut store, _) = store();
    let spaced = ImageId::of(b"white space around the colon");
    let good = spaced.to_hex();
    for (title, json) in [
        (
            "a number",
            r#"{"blocks":[{"type":"image","image": 3}]}"#.to_owned(),
        ),
        (
            "31 digits",
            format!(
                r#"{{"blocks":[{{"type":"image","image":"{}"}}]}}"#,
                &good[..31]
            ),
        ),
        (
            "33 digits",
            format!(r#"{{"blocks":[{{"type":"image","image":"{good}0"}}]}}"#),
        ),
        (
            "another key",
            format!(r#"{{"blocks":[{{"type":"image","alt":"{good}"}}]}}"#),
        ),
        ("no images", r#"{"blocks":[]}"#.to_owned()),
    ] {
        store
            .create_document(None, &content(title, "", &json))
            .unwrap();
    }
    assert!(store.referenced_images().unwrap().is_empty());

    store
        .create_document(
            None,
            &content(
                "spaced",
                "",
                &format!("{{\"blocks\":[{{\"type\":\"image\",\"image\" :\n  \"{good}\"}}]}}"),
            ),
        )
        .unwrap();
    assert_eq!(store.referenced_images().unwrap(), HashSet::from([spaced]));
}

// ----- the collector ---------------------------------------------------------------------

#[test]
fn an_image_nothing_names_goes_after_thirty_days_and_not_before() {
    let lib = OnDisk::new();
    let mut store = lib.open();
    let named_bytes = picture(1, 1_000);
    let unnamed_bytes = picture(2, 3_000);
    let named = store.put_image(&png(&named_bytes)).unwrap().id;
    let unnamed = store.put_image(&png(&unnamed_bytes)).unwrap().id;
    document_with(&mut store, "Holds one", &[named]);

    // The first run stamps what nothing names.
    assert_eq!(
        lib.collect(),
        ImagesCollected {
            images: 2,
            referenced: 1,
            newly_unreferenced: 1,
            deleted: 0,
            bytes_freed: 0,
            copy_failed: None,
            ..ImagesCollected::default()
        }
    );
    assert_eq!(stamp(&lib.images(), &named), Some(None));
    assert_eq!(stamp(&lib.images(), &unnamed), Some(Some(START)));

    // 29 days later it is still there, and its stamp is the first one.
    lib.clock.advance_ms(29 * DAY);
    assert_eq!(
        lib.collect(),
        ImagesCollected {
            images: 2,
            referenced: 1,
            newly_unreferenced: 0,
            deleted: 0,
            bytes_freed: 0,
            copy_failed: None,
            ..ImagesCollected::default()
        }
    );
    assert_eq!(stamp(&lib.images(), &unnamed), Some(Some(START)));
    assert_eq!(
        store.image_bytes(&unnamed).unwrap(),
        Some(unnamed_bytes.clone())
    );

    // On day 30 to the millisecond it is not older than the retention yet.
    lib.clock.set_ms(START + IMAGE_RETENTION_MS);
    assert_eq!(lib.collect().deleted, 0);

    // 31 days after the stamp it goes; the image that is named stays.
    lib.clock.set_ms(START + 31 * DAY);
    assert_eq!(
        lib.collect(),
        ImagesCollected {
            images: 2,
            referenced: 1,
            newly_unreferenced: 0,
            deleted: 1,
            bytes_freed: unnamed_bytes.len() as u64,
            copy_failed: None,
            ..ImagesCollected::default()
        }
    );
    assert_eq!(store.image_info(&unnamed).unwrap(), None);
    assert_eq!(store.image_bytes(&unnamed).unwrap(), None);
    assert_eq!(store.image_bytes(&named).unwrap(), Some(named_bytes));
    assert_eq!(store.image_count().unwrap(), 1);
}

#[test]
fn an_image_that_is_named_or_asked_for_again_starts_over() {
    let lib = OnDisk::new();
    let mut store = lib.open();
    let by_content = store.put_image(&png(&picture(1, 100))).unwrap().id;
    let by_touch = store.put_image(&png(&picture(2, 100))).unwrap().id;
    let by_put = store.put_image(&png(&picture(3, 100))).unwrap().id;
    assert_eq!(lib.collect().newly_unreferenced, 3);

    // Twenty days later each comes back its own way.
    lib.clock.advance_ms(20 * DAY);
    document_with(&mut store, "Names it", &[by_content]);
    assert!(store.touch_image(&by_touch).unwrap().is_some());
    assert_eq!(stamp(&lib.images(), &by_touch), Some(None));
    store.put_image(&png(&picture(3, 100))).unwrap();
    assert_eq!(stamp(&lib.images(), &by_put), Some(None));
    // The one that content names loses its stamp at the next run.
    assert_eq!(stamp(&lib.images(), &by_content), Some(Some(START)));

    // 31 days after the first stamp nothing goes: one is named, two have no stamp or a
    // new one.
    lib.clock.set_ms(START + 31 * DAY);
    let run = lib.collect();
    assert_eq!(
        (run.deleted, run.referenced, run.newly_unreferenced),
        (0, 1, 2)
    );
    assert_eq!(stamp(&lib.images(), &by_content), Some(None));
    assert_eq!(
        stamp(&lib.images(), &by_touch),
        Some(Some(START + 31 * DAY))
    );
    assert_eq!(stamp(&lib.images(), &by_put), Some(Some(START + 31 * DAY)));
    assert_eq!(store.image_count().unwrap(), 3);

    // The two that nothing names go thirty days after their new stamps.
    lib.clock.set_ms(START + 62 * DAY);
    assert_eq!(lib.collect().deleted, 2);
    assert_eq!(ids_in(&lib.images()), sorted(&[by_content]));
}

#[test]
fn a_purged_document_keeps_its_image_for_thirty_days() {
    let lib = OnDisk::new();
    let mut store = lib.open();
    let bytes = picture(1, 500);
    let id = store.put_image(&png(&bytes)).unwrap().id;
    let document = document_with(&mut store, "Goes", &[id]);

    // In the trash the document still names it, for as long as it lies there.
    store.trash_document(document.id).unwrap();
    lib.clock.advance_ms(100 * DAY);
    assert_eq!(lib.collect().referenced, 1);
    assert_eq!(stamp(&lib.images(), &id), Some(None));

    // Deleted for good (or with the trash emptied): the bytes stay, a stamp is set.
    store.empty_trash().unwrap();
    assert_eq!(store.image_bytes(&id).unwrap(), Some(bytes.clone()));
    let purged_at = lib.clock.now_ms();
    assert_eq!(lib.collect().newly_unreferenced, 1);
    lib.clock.advance_ms(29 * DAY);
    assert_eq!(lib.collect().deleted, 0);
    assert_eq!(store.image_bytes(&id).unwrap(), Some(bytes));
    lib.clock.set_ms(purged_at + 30 * DAY + 1);
    assert_eq!(lib.collect().deleted, 1);
    assert_eq!(store.image_bytes(&id).unwrap(), None);
}

#[test]
fn a_version_keeps_its_image_for_as_long_as_the_version_exists() {
    let lib = OnDisk::new();
    let mut store = lib.open();
    let id = store.put_image(&png(&picture(1, 500))).unwrap().id;
    let document = document_with(&mut store, "Had a picture", &[id]);
    let version = store.create_snapshot(document.id).unwrap();
    // The picture is taken out of the document; only the version names it now.
    store
        .save_document(
            document.id,
            &content("Had a picture", "Before", &content_with(&[])),
        )
        .unwrap();

    for _ in 0..4 {
        lib.clock.advance_ms(40 * DAY);
        let run = lib.collect();
        assert_eq!((run.referenced, run.deleted), (1, 0));
        assert_eq!(stamp(&lib.images(), &id), Some(None));
    }

    // The version goes; thirty days later the image does.
    store.delete_snapshot(version.id).unwrap();
    assert_eq!(lib.collect().newly_unreferenced, 1);
    lib.clock.advance_ms(31 * DAY);
    assert_eq!(lib.collect().deleted, 1);
    assert_eq!(store.image_count().unwrap(), 0);
}

#[test]
fn an_image_stays_while_a_kept_backup_is_older_than_its_stamp() {
    let lib = OnDisk::new();
    let mut store = lib.open();
    let id = store.put_image(&png(&picture(1, 500))).unwrap().id;
    let document = document_with(&mut store, "Backed up", &[id]);

    // A backup is written while the document names the image.
    let old_backup = lib.write_backup(&store);
    lib.clock.advance_ms(DAY);
    store.trash_document(document.id).unwrap();
    store.purge_document(document.id).unwrap();
    assert_eq!(lib.collect().newly_unreferenced, 1);
    let stamped_at = lib.clock.now_ms();

    // Forty days on the retention is over, but the backup from before the stamp is
    // still kept: bringing it back would name the image.
    lib.clock.advance_ms(40 * DAY);
    assert_eq!(lib.collect().deleted, 0);
    assert_eq!(stamp(&lib.images(), &id), Some(Some(stamped_at)));

    // A newer backup beside it changes nothing: the oldest one decides.
    let new_backup = lib.write_backup(&store);
    assert_eq!(lib.collect().deleted, 0);

    // Once the old backup is no longer kept, every kept backup was written after the
    // image stopped being named, and it goes.
    std::fs::remove_file(old_backup).unwrap();
    assert_eq!(lib.collect().deleted, 1);
    assert_eq!(store.image_count().unwrap(), 0);
    assert!(new_backup.exists());
}

#[test]
fn the_file_shrinks_after_a_deletion() {
    let lib = OnDisk::new();
    let mut store = lib.open();
    let kept = store.put_image(&png(&picture(1, 20_000))).unwrap().id;
    document_with(&mut store, "Keeps one", &[kept]);
    let mut dropped = Vec::new();
    for seed in 2..8 {
        dropped.push(store.put_image(&png(&picture(seed, 400_000))).unwrap().id);
    }
    lib.collect();
    lib.clock.advance_ms(31 * DAY);
    // The size with everything written back into the file.
    drop(store);
    let store = lib.open();
    assert_eq!(store.image_count().unwrap(), 7);
    drop(store);
    let before = std::fs::metadata(lib.images()).unwrap().len();
    assert!(before > 2_400_000, "{before}");

    let run = lib.collect();
    assert_eq!(run.deleted, 6);
    assert_eq!(run.bytes_freed, 6 * (400_000 + 8));
    let after = std::fs::metadata(lib.images()).unwrap().len();
    assert!(after < 200_000, "{before} bytes before, {after} after");
    let store = lib.open();
    assert_eq!(ids_in(&lib.images()), sorted(&[kept]));
    assert_eq!(store.image_bytes(&kept).unwrap(), Some(picture(1, 20_000)));
}

/// The hole the daily look closes. An image is stamped, then named again by a document
/// without anything telling the store (what an undo does), and long after it is removed
/// for good. With no look in between the old stamp is still on it that day, and its
/// thirty days are over before they began.
#[test]
fn a_stamp_does_not_grow_old_on_an_image_that_is_named() {
    fn sequence(with_the_daily_look: bool) -> (ImagesCollected, bool) {
        let lib = OnDisk::new();
        let mut store = lib.open();
        let id = store.put_image(&png(&picture(1, 500))).unwrap().id;
        // Inserted and cut again: nothing names it when the run at launch looks.
        assert_eq!(lib.collect().newly_unreferenced, 1);
        assert_eq!(stamp(&lib.images(), &id), Some(Some(START)));

        // The cut is undone: the document names the image, and nobody touched it.
        let document = document_with(&mut store, "Undone", &[id]);
        assert_eq!(stamp(&lib.images(), &id), Some(Some(START)));

        // The app stays open. A day later it looks.
        lib.clock.advance_ms(DAY);
        if with_the_daily_look {
            let looked = lib.look();
            assert_eq!((looked.referenced, looked.deleted), (1, 0));
            assert_eq!(stamp(&lib.images(), &id), Some(None));
        }

        // Forty days after that the image is taken out for good, and the app is started
        // again.
        lib.clock.advance_ms(40 * DAY);
        store.trash_document(document.id).unwrap();
        store.purge_document(document.id).unwrap();
        let run = lib.collect();
        (run, store.image_info(&id).unwrap().is_some())
    }

    let (run, still_there) = sequence(true);
    assert_eq!((run.newly_unreferenced, run.deleted), (1, 0));
    assert!(
        still_there,
        "the image has its thirty days from the day it was removed"
    );

    // The control: without the look the same steps delete it at once.
    let (run, still_there) = sequence(false);
    assert_eq!((run.newly_unreferenced, run.deleted), (0, 1));
    assert!(!still_there);
}

#[test]
fn the_look_alone_deletes_nothing_however_old_a_stamp_is() {
    let lib = OnDisk::new();
    let mut store = lib.open();
    let named = store.put_image(&png(&picture(1, 500))).unwrap().id;
    let unnamed = store.put_image(&png(&picture(2, 500))).unwrap().id;
    let back = store.put_image(&png(&picture(3, 500))).unwrap().id;
    document_with(&mut store, "Names one", &[named]);

    assert_eq!(
        lib.look(),
        ImagesCollected {
            images: 3,
            referenced: 1,
            newly_unreferenced: 2,
            deleted: 0,
            bytes_freed: 0,
            copy_failed: None,
            ..ImagesCollected::default()
        }
    );
    lib.clock.advance_ms(365 * DAY);
    document_with(&mut store, "Names another", &[back]);
    assert_eq!(
        lib.look(),
        ImagesCollected {
            images: 3,
            referenced: 2,
            newly_unreferenced: 0,
            deleted: 0,
            bytes_freed: 0,
            copy_failed: None,
            ..ImagesCollected::default()
        }
    );
    assert_eq!(stamp(&lib.images(), &named), Some(None));
    assert_eq!(stamp(&lib.images(), &back), Some(None));
    assert_eq!(stamp(&lib.images(), &unnamed), Some(Some(START)));
    assert_eq!(store.image_count().unwrap(), 3);
}

#[test]
fn the_collector_runs_beside_an_open_store_and_refuses_a_library_it_cannot_read() {
    let lib = OnDisk::new();
    let mut store = lib.open();
    let id = store.put_image(&png(&picture(1, 500))).unwrap().id;
    // On a thread of its own while the store is open, as the app runs it.
    let database = lib.database();
    let run = std::thread::spawn(move || {
        collect_unused_images(&database, START, IMAGE_RETENTION_MS).unwrap()
    })
    .join()
    .unwrap();
    assert_eq!(run.newly_unreferenced, 1);
    // The store goes on writing afterwards.
    store.put_image(&png(&picture(2, 500))).unwrap();
    assert!(store.touch_image(&id).unwrap().is_some());
    store.close().unwrap();

    // A library of a later version may name images in ways this one cannot find: the
    // collector stamps and deletes nothing there.
    let conn = rusqlite::Connection::open(lib.database()).unwrap();
    conn.pragma_update(None, "user_version", writer_store::SCHEMA_VERSION + 1)
        .unwrap();
    drop(conn);
    assert!(matches!(
        collect_unused_images(&lib.database(), START + 100 * DAY, IMAGE_RETENTION_MS),
        Err(StoreError::SchemaTooNew { .. })
    ));
    assert!(matches!(
        mark_unused_images(&lib.database(), START + 100 * DAY),
        Err(StoreError::SchemaTooNew { .. })
    ));
    assert_eq!(stamp(&lib.images(), &id), Some(None));
    assert_eq!(ids_in(&lib.images()).len(), 2);
}

// ----- a connection of its own -----------------------------------------------------------

#[test]
fn an_image_file_and_a_store_of_the_same_library_see_each_other() {
    let lib = OnDisk::new();
    let mut store = lib.open();
    let mut file = ImageFile::beside(&lib.database());
    assert_eq!(file.path(), Some(lib.images().as_path()));

    // Nothing is there yet: reads answer that, and make nothing.
    let first = picture(1, 30_000);
    let second = picture(2, 30_000);
    assert_eq!(file.count().unwrap(), 0);
    assert_eq!(file.bytes(&ImageId::of(&first)).unwrap(), None);
    assert_eq!(file.info(&ImageId::of(&first)).unwrap(), None);
    assert_eq!(file.touch(&ImageId::of(&first)).unwrap(), None);
    assert_eq!(store.image_count().unwrap(), 0);
    assert_eq!(store.image_bytes(&ImageId::of(&first)).unwrap(), None);
    assert!(!lib.images().exists());

    // Put through the file, read through the store (which found no file a moment ago,
    // and did not remember that).
    let info = file.put(&png(&first)).unwrap();
    assert_eq!(info.id, ImageId::of(&first));
    assert_eq!(store.image_bytes(&info.id).unwrap(), Some(first.clone()));
    assert_eq!(store.image_info(&info.id).unwrap(), Some(info.clone()));
    assert_eq!(store.image_count().unwrap(), 1);

    // Put through the store, read through the file.
    let other = store.put_image(&png(&second)).unwrap();
    assert_eq!(file.bytes(&other.id).unwrap(), Some(second));
    assert_eq!(file.info(&other.id).unwrap(), Some(other.clone()));
    assert_eq!(file.count().unwrap(), 2);

    // The same bytes through the other one: stored once.
    assert_eq!(store.put_image(&png(&first)).unwrap(), info);
    assert_eq!(file.count().unwrap(), 2);

    // A file that is opened for another thread works there.
    let moved = std::thread::spawn(move || {
        let mut file = file;
        let third = picture(3, 10);
        let stored = file.put(&png(&third)).unwrap();
        (stored.id, file.count().unwrap())
    })
    .join()
    .unwrap();
    assert_eq!(moved.1, 3);
    assert_eq!(store.image_bytes(&moved.0).unwrap(), Some(picture(3, 10)));
}

#[test]
fn the_same_bytes_put_from_two_threads_are_stored_once() {
    for _ in 0..8 {
        let lib = OnDisk::new();
        // The library is there; its second file is not, so both threads may be the one
        // that makes it.
        lib.open().close().unwrap();
        let bytes = picture(9, 200_000);
        let threads: Vec<_> = (0..2)
            .map(|_| {
                let database = lib.database();
                let bytes = bytes.clone();
                std::thread::spawn(move || {
                    let mut file = ImageFile::beside(&database);
                    file.put(&png(&bytes)).unwrap()
                })
            })
            .collect();
        let stored: Vec<ImageInfo> = threads.into_iter().map(|t| t.join().unwrap()).collect();
        assert_eq!(stored[0], stored[1]);
        assert_eq!(stored[0].id, ImageId::of(&bytes));
        let store = lib.open();
        assert_eq!(store.image_count().unwrap(), 1);
        assert_eq!(store.image_bytes(&stored[0].id).unwrap(), Some(bytes));
    }
}

#[test]
fn a_touch_of_an_image_without_a_stamp_writes_nothing() {
    let lib = OnDisk::new();
    let mut store = lib.open();
    let bytes = picture(1, 1_000);
    let id = store.put_image(&png(&bytes)).unwrap().id;
    let mut file = ImageFile::beside(&lib.database());
    assert!(file.info(&id).unwrap().is_some());

    let watcher = rusqlite::Connection::open(lib.images()).unwrap();
    let version = data_version(&watcher);
    let modified = |name: &str| {
        let path = lib.dir.path().join(name);
        std::fs::metadata(path)
            .ok()
            .map(|meta| (meta.len(), meta.modified().unwrap()))
    };
    let before = (modified("images.db"), modified("images.db-wal"));
    // File systems keep times in steps: a write right now could carry the same time.
    std::thread::sleep(Duration::from_millis(30));

    // The usual case: there is no stamp to clear. Through the file, through the store,
    // and a put of bytes that are there.
    assert!(file.touch(&id).unwrap().is_some());
    assert!(store.touch_image(&id).unwrap().is_some());
    assert_eq!(store.put_image(&png(&bytes)).unwrap().id, id);
    assert_eq!(file.put(&png(&bytes)).unwrap().id, id);
    assert_eq!(data_version(&watcher), version);
    assert_eq!((modified("images.db"), modified("images.db-wal")), before);

    // With a stamp on it the touch is a write, and the watcher sees one.
    drop(file);
    lib.look();
    assert_eq!(stamp(&lib.images(), &id), Some(Some(START)));
    let version = data_version(&watcher);
    let mut file = ImageFile::beside(&lib.database());
    assert!(file.touch(&id).unwrap().is_some());
    assert_ne!(data_version(&watcher), version);
    assert_eq!(stamp(&lib.images(), &id), Some(None));
}

#[test]
fn a_read_only_image_file_refuses_to_store() {
    let lib = OnDisk::new();
    lib.open().close().unwrap();
    let bytes = picture(1, 1_000);

    // No file yet: it reads nothing, stores nothing, creates nothing.
    let mut reader = ImageFile::read_only(&lib.database());
    assert_eq!(reader.count().unwrap(), 0);
    assert!(matches!(
        reader.put(&png(&bytes)),
        Err(StoreError::Sqlite(_))
    ));
    assert!(matches!(
        reader.touch(&ImageId::of(&bytes)),
        Err(StoreError::Sqlite(_))
    ));
    assert!(!lib.images().exists());

    let mut writer = ImageFile::beside(&lib.database());
    let info = writer.put(&png(&bytes)).unwrap();
    assert_eq!(reader.bytes(&info.id).unwrap(), Some(bytes));
    assert_eq!(reader.info(&info.id).unwrap(), Some(info.clone()));
    assert_eq!(reader.count().unwrap(), 1);
    assert!(reader.put(&png(&picture(2, 10))).is_err());
    assert!(reader.touch(&info.id).is_err());
    assert_eq!(writer.count().unwrap(), 1);
}

// ----- the copy of the pictures in the backups directory ---------------------------------

#[test]
fn the_copy_has_every_picture_after_it_is_brought_up_to_date() {
    let lib = OnDisk::new();
    let mut store = lib.open();
    let first = picture(1, 50_000);
    let second = picture(2, 70_000);
    let a = store.put_image(&png(&first)).unwrap();
    let b = store
        .put_image(&NewImage {
            bytes: &second,
            format: ImageFormat::Jpeg,
            width: 640,
            height: 480,
            density: 72,
        })
        .unwrap();
    // Storing a picture does not write the copy: that is for the background.
    assert!(!lib.copy().exists());

    // With the store open, on another thread.
    let database = lib.database();
    let done = std::thread::spawn(move || back_up_all_images(&database).unwrap())
        .join()
        .unwrap();
    assert_eq!(
        done,
        ImagesBackedUp {
            images: 2,
            copied: 2,
            bytes_copied: (first.len() + second.len()) as u64,
            waiting: 0,
            ..ImagesBackedUp::default()
        }
    );
    assert_eq!(lib.copy(), lib.dir.path().join("backups").join("images.db"));

    // The copy is an images database with the same rows: read it as one.
    let beside_the_copy = lib.copy().with_file_name("library.db");
    let copy = ImageFile::read_only(&beside_the_copy);
    assert_eq!(copy.count().unwrap(), 2);
    assert_eq!(copy.info(&a.id).unwrap(), Some(a.clone()));
    assert_eq!(copy.info(&b.id).unwrap(), Some(b.clone()));
    assert_eq!(copy.bytes(&a.id).unwrap(), Some(first));
    assert_eq!(copy.bytes(&b.id).unwrap(), Some(second));
    assert_eq!(stamp(&lib.copy(), &a.id), Some(None));

    // It is not taken for a daily backup of the library.
    assert!(
        writer_store::list_backups(&backups_dir(&lib.database()))
            .unwrap()
            .is_empty()
    );
    // And the store goes on as before.
    store.put_image(&png(&picture(3, 10))).unwrap();
}

#[test]
fn nothing_is_copied_twice() {
    let lib = OnDisk::new();
    let mut store = lib.open();
    for seed in 0..3 {
        store.put_image(&png(&picture(seed, 10_000))).unwrap();
    }
    assert_eq!(back_up_all_images(&lib.database()).unwrap().copied, 3);

    let watcher = rusqlite::Connection::open(lib.copy()).unwrap();
    let version = data_version(&watcher);
    let again = back_up_all_images(&lib.database()).unwrap();
    assert_eq!(
        again,
        ImagesBackedUp {
            images: 3,
            copied: 0,
            bytes_copied: 0,
            waiting: 0,
            ..ImagesBackedUp::default()
        }
    );
    assert_eq!(
        back_up_images(&lib.database(), 1).unwrap(),
        ImagesBackedUp {
            images: 3,
            ..ImagesBackedUp::default()
        }
    );
    assert_eq!(
        data_version(&watcher),
        version,
        "the copy was written again"
    );

    // One more picture: that one is copied, and only that one.
    let new = picture(9, 20_000);
    let id = store.put_image(&png(&new)).unwrap().id;
    assert_eq!(
        back_up_all_images(&lib.database()).unwrap(),
        ImagesBackedUp {
            images: 4,
            copied: 1,
            bytes_copied: new.len() as u64,
            waiting: 0,
            ..ImagesBackedUp::default()
        }
    );
    assert_ne!(data_version(&watcher), version);
    assert_eq!(ids_in(&lib.copy()).len(), 4);
    assert!(ids_in(&lib.copy()).contains(&id.to_hex()));
}

#[test]
fn the_copy_is_brought_up_to_date_a_slice_at_a_time() {
    let lib = OnDisk::new();
    let mut store = lib.open();
    let mut ids = Vec::new();
    for seed in 0..5 {
        ids.push(store.put_image(&png(&picture(seed, 9_992))).unwrap().id);
    }
    // A slice of 20,000 bytes takes pictures of 10,000 until it has that many: two.
    let first = back_up_images(&lib.database(), 20_000).unwrap();
    assert_eq!(
        first,
        ImagesBackedUp {
            images: 5,
            copied: 2,
            bytes_copied: 20_000,
            waiting: 3,
            ..ImagesBackedUp::default()
        }
    );
    // Oldest first.
    assert_eq!(ids_in(&lib.copy()), sorted(&ids[..2]));
    // A slice smaller than a picture still takes one.
    let second = back_up_images(&lib.database(), 1).unwrap();
    assert_eq!((second.copied, second.waiting), (1, 2));
    let third = back_up_images(&lib.database(), 0).unwrap();
    assert_eq!((third.copied, third.waiting), (1, 1));
    let last = back_up_images(&lib.database(), u64::MAX).unwrap();
    assert_eq!((last.copied, last.waiting), (1, 0));
    assert_eq!(ids_in(&lib.copy()), sorted(&ids));
    assert_eq!(back_up_images(&lib.database(), u64::MAX).unwrap().copied, 0);
}

#[test]
fn a_library_whose_images_file_was_deleted_gets_its_pictures_back_from_the_copy() {
    let lib = OnDisk::new();
    let mut store = lib.open();
    let first = picture(1, 60_000);
    let second = picture(2, 60_000);
    let third = picture(3, 60_000);
    let a = store.put_image(&png(&first)).unwrap();
    let b = store.put_image(&png(&second)).unwrap();
    let c = store.put_image(&png(&third)).unwrap();
    let document = document_with(&mut store, "Three pictures", &[a.id, b.id, c.id]);
    back_up_all_images(&lib.database()).unwrap();
    store.close().unwrap();

    // The file is lost, with what was beside it.
    for name in ["images.db", "images.db-wal", "images.db-shm"] {
        let _ = std::fs::remove_file(lib.dir.path().join(name));
    }
    assert!(!lib.images().exists());

    // A reader finds the pictures in the copy and writes nothing.
    let reader = Store::open_read_only(lib.database())
        .unwrap()
        .store()
        .unwrap();
    assert_eq!(reader.image_bytes(&a.id).unwrap(), Some(first.clone()));
    assert_eq!(reader.image_info(&a.id).unwrap(), Some(a.clone()));
    assert_eq!(reader.image_count().unwrap(), 0);
    drop(reader);
    assert!(!lib.images().exists());

    // So does a dry run.
    let mut store = lib.open();
    store.set_query_only(true).unwrap();
    assert_eq!(store.image_bytes(&a.id).unwrap(), Some(first.clone()));
    assert_eq!(store.touch_image(&a.id).unwrap(), Some(a.clone()));
    assert!(!lib.images().exists());
    store.set_query_only(false).unwrap();

    // The library itself reads them from the copy and has them again: by its bytes, by
    // what is known about it, by a touch. Each as it was, with the time it was first
    // stored.
    lib.clock.advance_ms(DAY);
    assert_eq!(store.image_bytes(&a.id).unwrap(), Some(first.clone()));
    assert!(lib.images().exists());
    assert_eq!(store.image_count().unwrap(), 1);
    assert_eq!(store.image_info(&b.id).unwrap(), Some(b.clone()));
    assert_eq!(store.image_count().unwrap(), 2);
    assert_eq!(store.touch_image(&c.id).unwrap(), Some(c.clone()));
    assert_eq!(store.image_count().unwrap(), 3);
    assert_eq!(ids_in(&lib.images()), sorted(&[a.id, b.id, c.id]));
    assert_eq!(store.image_info(&a.id).unwrap(), Some(a.clone()));
    for id in [a.id, b.id, c.id] {
        assert_eq!(stamp(&lib.images(), &id), Some(None));
    }

    // What the copy does not have is still missing, and makes no difference.
    assert_eq!(
        store.image_bytes(&ImageId::of(b"never stored")).unwrap(),
        None
    );
    assert_eq!(
        store.touch_image(&ImageId::of(b"never stored")).unwrap(),
        None
    );

    // The library is whole again: a second file of its own serves the pictures, the
    // collector finds them named, and the copy is as it was.
    store.close().unwrap();
    std::fs::rename(lib.copy(), lib.copy().with_file_name("moved-away.db")).unwrap();
    let store = lib.open();
    assert_eq!(store.image_bytes(&b.id).unwrap(), Some(second));
    assert_eq!(store.image_bytes(&c.id).unwrap(), Some(third));
    assert_eq!(
        store.document(document.id).unwrap().content,
        content_with(&[a.id, b.id, c.id])
    );
    let run = lib.collect();
    assert_eq!((run.images, run.referenced, run.deleted), (3, 3, 0));
}

#[test]
fn an_image_file_of_its_own_reads_from_the_copy_too() {
    let lib = OnDisk::new();
    let mut store = lib.open();
    let bytes = picture(1, 10_000);
    let info = store.put_image(&png(&bytes)).unwrap();
    back_up_all_images(&lib.database()).unwrap();
    store.close().unwrap();
    for name in ["images.db", "images.db-wal", "images.db-shm"] {
        let _ = std::fs::remove_file(lib.dir.path().join(name));
    }

    // Reading only: found, nothing written.
    let reader = ImageFile::read_only(&lib.database());
    assert_eq!(reader.bytes(&info.id).unwrap(), Some(bytes.clone()));
    assert_eq!(reader.info(&info.id).unwrap(), Some(info.clone()));
    assert!(!lib.images().exists());

    // The app's background connection: found, and written back.
    let file = ImageFile::beside(&lib.database());
    assert_eq!(file.bytes(&info.id).unwrap(), Some(bytes.clone()));
    assert_eq!(file.count().unwrap(), 1);
    assert_eq!(reader.count().unwrap(), 1);
    let store = lib.open();
    assert_eq!(store.image_info(&info.id).unwrap(), Some(info));
}

#[test]
fn a_copy_that_does_not_hold_the_bytes_of_the_id_is_not_believed() {
    let lib = OnDisk::new();
    let mut store = lib.open();
    let bytes = picture(1, 10_000);
    let id = store.put_image(&png(&bytes)).unwrap().id;
    back_up_all_images(&lib.database()).unwrap();
    store.close().unwrap();
    std::fs::remove_file(lib.images()).unwrap();

    // The disk of the copy went bad: the row is there, the bytes are others.
    let conn = rusqlite::Connection::open(lib.copy()).unwrap();
    conn.execute("UPDATE images SET bytes = ?1", [picture(2, 10_000)])
        .unwrap();
    drop(conn);
    let mut store = lib.open();
    assert_eq!(store.image_bytes(&id).unwrap(), None);
    assert_eq!(store.image_info(&id).unwrap(), None);
    assert_eq!(store.touch_image(&id).unwrap(), None);
    assert_eq!(store.image_count().unwrap(), 0);

    // A copy that is not a database at all is no reason to fail a read either.
    std::fs::write(
        lib.copy(),
        b"not a database, only some text that is long enough",
    )
    .unwrap();
    assert_eq!(store.image_bytes(&id).unwrap(), None);
    // The library goes on storing pictures.
    assert_eq!(store.put_image(&png(&bytes)).unwrap().id, id);
    assert_eq!(store.image_bytes(&id).unwrap(), Some(bytes));
}

#[test]
fn the_collector_and_the_copy_agree() {
    let lib = OnDisk::new();
    let mut store = lib.open();
    let kept_bytes = picture(1, 30_000);
    let kept = store.put_image(&png(&kept_bytes)).unwrap().id;
    let goes = store.put_image(&png(&picture(2, 300_000))).unwrap().id;
    let comes_back = store.put_image(&png(&picture(3, 30_000))).unwrap().id;
    document_with(&mut store, "Names one", &[kept]);

    // A run of the collector brings the copy up to date by itself, the looking half too.
    assert!(!lib.copy().exists());
    assert_eq!(lib.collect().newly_unreferenced, 2);
    assert_eq!(ids_in(&lib.copy()), sorted(&[kept, goes, comes_back]));
    let late = store.put_image(&png(&picture(4, 30_000))).unwrap().id;
    document_with(&mut store, "Names the late one", &[late]);
    lib.clock.advance_ms(DAY);
    lib.look();
    assert_eq!(ids_in(&lib.copy()), sorted(&[kept, goes, comes_back, late]));
    store.close().unwrap();
    let copy_before = std::fs::metadata(lib.copy()).unwrap().len();

    // While the collector keeps an image the copy keeps it; nothing in the copy carries
    // a mark.
    lib.clock.advance_ms(20 * DAY);
    assert_eq!(lib.collect().deleted, 0);
    assert_eq!(ids_in(&lib.copy()).len(), 4);
    for id in [kept, goes, comes_back, late] {
        assert_eq!(stamp(&lib.copy(), &id), Some(None));
    }

    // One of the two unnamed images is asked for again; the other one goes after its
    // thirty days, from the file and from the copy in the same run.
    let mut store = lib.open();
    store.touch_image(&comes_back).unwrap();
    lib.clock.set_ms(START + 31 * DAY);
    let run = lib.collect();
    assert_eq!((run.deleted, run.bytes_freed), (1, 300_008));
    assert_eq!(ids_in(&lib.images()), sorted(&[kept, comes_back, late]));
    assert_eq!(ids_in(&lib.copy()), sorted(&[kept, comes_back, late]));
    for id in [kept, comes_back, late] {
        assert_eq!(stamp(&lib.copy(), &id), Some(None));
    }
    assert_eq!(store.image_bytes(&goes).unwrap(), None);
    assert_eq!(store.image_bytes(&kept).unwrap(), Some(kept_bytes));
    // The copy gave the space back as well.
    let copy_after = std::fs::metadata(lib.copy()).unwrap().len();
    assert!(
        copy_after + 250_000 < copy_before,
        "{copy_before} then {copy_after}"
    );
}

/// A row leaves the copy only when no kept backup of the library names it: read from the
/// backups themselves, whatever the stamps say.
#[test]
fn the_copy_keeps_what_a_kept_backup_names() {
    let lib = OnDisk::new();
    let mut store = lib.open();
    let bytes = picture(1, 40_000);
    let id = store.put_image(&png(&bytes)).unwrap().id;
    back_up_all_images(&lib.database()).unwrap();
    // Stamped while nothing names it ...
    assert_eq!(lib.collect().newly_unreferenced, 1);
    // ... then named again without a touch, and a daily backup is written of that.
    lib.clock.advance_ms(10 * DAY);
    let document = document_with(&mut store, "Undone", &[id]);
    let backup = lib.write_backup(&store);
    // Taken out again before anything looked. Thirty-one days after the stamp the
    // collector deletes the image: by its stamp it was unnamed before the backup was
    // written. (This is the stale stamp the daily look is there to prevent.)
    store.trash_document(document.id).unwrap();
    store.purge_document(document.id).unwrap();
    lib.clock.set_ms(START + 31 * DAY);
    assert_eq!(lib.collect().deleted, 1);
    assert_eq!(ids_in(&lib.images()), Vec::<String>::new());

    // The backup names it, so the copy still has it, marked as collected.
    assert_eq!(ids_in(&lib.copy()), sorted(&[id]));
    assert_eq!(stamp(&lib.copy(), &id), Some(Some(START + 31 * DAY)));
    // Further runs leave it there for as long as the backup is kept.
    lib.clock.advance_ms(DAY);
    assert_eq!(lib.collect(), ImagesCollected::default());
    lib.look();
    assert_eq!(ids_in(&lib.copy()), sorted(&[id]));

    // The backup is brought back: the library names the image again, and gets it from
    // the copy.
    store.close().unwrap();
    std::fs::copy(&backup, lib.database()).unwrap();
    let _ = std::fs::remove_file(lib.dir.path().join("library.db-wal"));
    let _ = std::fs::remove_file(lib.dir.path().join("library.db-shm"));
    let mut store = lib.open();
    assert_eq!(store.referenced_images().unwrap(), HashSet::from([id]));
    assert_eq!(store.image_bytes(&id).unwrap(), Some(bytes.clone()));
    assert_eq!(ids_in(&lib.images()), sorted(&[id]));
    // It is a row like any other again: the next run takes the mark off the copy.
    lib.collect();
    assert_eq!(stamp(&lib.copy(), &id), Some(None));
    assert_eq!(stamp(&lib.images(), &id), Some(None));

    // Removed for good once more, and this time the backup that names it is no longer
    // kept when the collector deletes it: it leaves the copy too.
    for document in store.documents().unwrap() {
        store.trash_document(document.id).unwrap();
    }
    store.empty_trash().unwrap();
    std::fs::remove_file(&backup).unwrap();
    assert_eq!(lib.collect().newly_unreferenced, 1);
    lib.clock.advance_ms(31 * DAY);
    assert_eq!(lib.collect().deleted, 1);
    assert_eq!(ids_in(&lib.copy()), Vec::<String>::new());
    assert_eq!(store.image_bytes(&id).unwrap(), None);
}

/// The same, where the backup is still kept when the collector deletes and goes later:
/// the row waits in the copy, marked, and leaves with a later run.
#[test]
fn a_collected_image_leaves_the_copy_when_the_backup_that_names_it_is_gone() {
    let lib = OnDisk::new();
    let mut store = lib.open();
    let id = store.put_image(&png(&picture(1, 40_000))).unwrap().id;
    let other = store.put_image(&png(&picture(2, 40_000))).unwrap().id;
    document_with(&mut store, "Keeps the other", &[other]);
    lib.collect();
    lib.clock.advance_ms(10 * DAY);
    let document = document_with(&mut store, "Undone", &[id]);
    let backup = lib.write_backup(&store);
    store.trash_document(document.id).unwrap();
    store.purge_document(document.id).unwrap();
    lib.clock.set_ms(START + 31 * DAY);
    assert_eq!(lib.collect().deleted, 1);
    assert_eq!(ids_in(&lib.copy()), sorted(&[id, other]));

    // A backup that cannot be read: what it names is not known, nothing leaves the copy.
    let unreadable = backups_dir(&lib.database()).join("library-1.db");
    std::fs::write(
        &unreadable,
        b"not a database, only some text that is long enough",
    )
    .unwrap();
    std::fs::remove_file(&backup).unwrap();
    lib.clock.advance_ms(DAY);
    lib.collect();
    assert_eq!(ids_in(&lib.copy()), sorted(&[id, other]));

    // With only readable backups left that do not name it, it goes; the other stays.
    std::fs::remove_file(&unreadable).unwrap();
    lib.write_backup(&store);
    lib.clock.advance_ms(DAY);
    let run = lib.collect();
    assert_eq!((run.images, run.deleted), (1, 0));
    assert_eq!(ids_in(&lib.copy()), sorted(&[other]));
    assert_eq!(stamp(&lib.copy(), &other), Some(None));
}

/// A copy whose images file was lost is not thinned out by the collector: nothing in it
/// was collected, however long nothing has named it.
#[test]
fn a_lost_images_file_takes_nothing_out_of_the_copy() {
    let lib = OnDisk::new();
    let mut store = lib.open();
    let named = store.put_image(&png(&picture(1, 10_000))).unwrap().id;
    let unnamed = store.put_image(&png(&picture(2, 10_000))).unwrap().id;
    document_with(&mut store, "Names one", &[named]);
    lib.collect();
    store.close().unwrap();
    for name in ["images.db", "images.db-wal", "images.db-shm"] {
        let _ = std::fs::remove_file(lib.dir.path().join(name));
    }

    // Launches over a year, with no images file: the collector has nothing to look at.
    for _ in 0..4 {
        lib.clock.advance_ms(100 * DAY);
        assert_eq!(lib.collect(), ImagesCollected::default());
        assert_eq!(lib.look(), ImagesCollected::default());
    }
    assert_eq!(ids_in(&lib.copy()), sorted(&[named, unnamed]));

    // One picture is read and comes back; the collector then runs on a file that holds
    // one image, and still takes nothing out of the copy that it did not delete itself.
    let store = lib.open();
    assert!(store.image_bytes(&named).unwrap().is_some());
    lib.clock.advance_ms(100 * DAY);
    let run = lib.collect();
    assert_eq!((run.images, run.referenced, run.deleted), (1, 1, 0));
    assert_eq!(ids_in(&lib.copy()), sorted(&[named, unnamed]));
    // The one nothing names is still to be had, for whoever asks for it.
    assert!(store.image_bytes(&unnamed).unwrap().is_some());
}

// ----- a damaged file -------------------------------------------------------------------

/// Overwrites `len` bytes of the file at `path` from `offset` with a pattern.
fn scribble(path: &Path, offset: u64, len: usize) {
    use std::io::{Seek, SeekFrom, Write};
    let mut file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    file.seek(SeekFrom::Start(offset)).unwrap();
    file.write_all(&[0xDE, 0xAD, 0xBE, 0xEF].repeat(len / 4))
        .unwrap();
}

/// The two files of the page [`two_pictures_and_their_copy`] stores beside its pictures.
fn the_two_files() -> [Vec<u8>; 2] {
    [page_file(1, 50_000), page_file(2, 50_000)]
}

/// A library with two pictures and a page of two files in `images.db` and in its copy,
/// closed.
fn two_pictures_and_their_copy(lib: &OnDisk) -> [(ImageInfo, Vec<u8>); 2] {
    let mut store = lib.open();
    let first = picture(1, 60_000);
    let second = picture(2, 60_000);
    let a = store.put_image(&png(&first)).unwrap();
    let b = store.put_image(&png(&second)).unwrap();
    document_with(&mut store, "Two pictures", &[a.id, b.id]);
    let files: Vec<FileId> = the_two_files()
        .iter()
        .map(|bytes| store.put_file(bytes).unwrap().id)
        .collect();
    page_with(&mut store, &files);
    back_up_all_images(&lib.database()).unwrap();
    store.close().unwrap();
    [(a, first), (b, second)]
}

/// What a library whose `images.db` is damaged does, whatever the damage: the pictures
/// come from the copy, a new picture is stored, the damaged file is set aside and named,
/// a reader and a dry run change nothing.
fn a_damaged_file_is_set_aside_and_its_pictures_come_back(
    lib: &OnDisk,
    pictures: &[(ImageInfo, Vec<u8>)],
) {
    let damaged = std::fs::read(lib.images()).unwrap();

    // A reader (the tool reading) and a dry run find the pictures in the copy and leave
    // the damaged file where it is.
    let reader = Store::open_read_only(lib.database())
        .unwrap()
        .store()
        .unwrap();
    for (info, bytes) in pictures {
        assert_eq!(reader.image_bytes(&info.id).unwrap().as_ref(), Some(bytes));
        assert_eq!(reader.image_info(&info.id).unwrap().as_ref(), Some(info));
    }
    for bytes in the_two_files() {
        assert_eq!(reader.file_bytes(&FileId::of(&bytes)).unwrap(), Some(bytes));
    }
    drop(reader);
    let mut store = lib.open();
    store.set_query_only(true).unwrap();
    let (first, first_bytes) = &pictures[0];
    assert_eq!(
        store.image_bytes(&first.id).unwrap().as_ref(),
        Some(first_bytes)
    );
    assert_eq!(store.touch_image(&first.id).unwrap().as_ref(), Some(first));
    assert_eq!(std::fs::read(lib.images()).unwrap(), damaged);
    assert!(images_moved_aside(&lib.database()).is_empty());
    store.set_query_only(false).unwrap();

    // The library itself: every picture and every file of the page as it was, a new one
    // stored.
    for (info, bytes) in pictures {
        assert_eq!(store.image_bytes(&info.id).unwrap().as_ref(), Some(bytes));
        assert_eq!(store.image_info(&info.id).unwrap().as_ref(), Some(info));
    }
    let files: Vec<FileId> = the_two_files().iter().map(|b| FileId::of(b)).collect();
    for (id, bytes) in files.iter().zip(the_two_files()) {
        assert_eq!(store.file_bytes(id).unwrap(), Some(bytes));
    }
    let new = picture(9, 5_000);
    let stored = store.put_image(&png(&new)).unwrap();
    assert_eq!(store.image_bytes(&stored.id).unwrap(), Some(new));

    // The damaged file is beside the new one, as it was, and is named.
    let aside = images_moved_aside(&lib.database());
    assert_eq!(aside.len(), 1, "{aside:?}");
    assert!(
        aside[0]
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("images.db.damaged-")
    );
    assert_eq!(std::fs::read(&aside[0]).unwrap(), damaged);
    let mut ids: Vec<ImageId> = pictures.iter().map(|(info, _)| info.id).collect();
    ids.push(stored.id);
    assert_eq!(ids_in(&lib.images()), sorted(&ids));

    assert_eq!(file_ids_in(&lib.images()), sorted_files(&files));

    // The collector and the copy go on.
    let run = lib.collect();
    assert_eq!(run.copy_failed, None);
    assert_eq!(run.images, 3);
    assert_eq!((run.files, run.files_referenced), (2, 2));
    assert_eq!(ids_in(&lib.copy()), sorted(&ids));
    assert_eq!(file_ids_in(&lib.copy()), sorted_files(&files));
    store.close().unwrap();
}

#[test]
fn an_images_file_whose_header_is_gone_gives_its_pictures_back_from_the_copy() {
    let lib = OnDisk::new();
    let pictures = two_pictures_and_their_copy(&lib);
    scribble(&lib.images(), 0, 100);
    a_damaged_file_is_set_aside_and_its_pictures_come_back(&lib, &pictures);
}

#[test]
fn an_images_file_cut_in_half_gives_its_pictures_back_from_the_copy() {
    let lib = OnDisk::new();
    let pictures = two_pictures_and_their_copy(&lib);
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(lib.images())
        .unwrap();
    let len = file.metadata().unwrap().len();
    file.set_len(len / 2).unwrap();
    drop(file);
    a_damaged_file_is_set_aside_and_its_pictures_come_back(&lib, &pictures);
}

/// The collector finds the damage before anything else does (a launch that shows no
/// picture): it sets the file aside, and the pictures come back from the copy.
#[test]
fn the_collector_sets_a_damaged_images_file_aside() {
    let lib = OnDisk::new();
    let pictures = two_pictures_and_their_copy(&lib);
    scribble(&lib.images(), 0, 100);
    let run = lib.collect();
    assert_eq!((run.images, run.deleted, run.copy_failed), (0, 0, None));
    assert_eq!(images_moved_aside(&lib.database()).len(), 1);
    // Nothing went out of the copy.
    assert_eq!(
        ids_in(&lib.copy()),
        sorted(&[pictures[0].0.id, pictures[1].0.id])
    );
    let store = lib.open();
    for (info, bytes) in &pictures {
        assert_eq!(store.image_bytes(&info.id).unwrap().as_ref(), Some(bytes));
    }
    assert_eq!(store.image_count().unwrap(), 2);
}

/// What of a damaged file can still be read and the copy does not have is not lost:
/// it is in the new file.
#[test]
fn what_the_copy_does_not_have_is_saved_from_a_damaged_file() {
    let lib = OnDisk::new();
    let [(a, a_bytes), (b, b_bytes)] = two_pictures_and_their_copy(&lib);
    // A third picture, stored after the copy was brought up to date.
    let mut store = lib.open();
    let c_bytes = picture(3, 100);
    let c = store.put_image(&png(&c_bytes)).unwrap();
    store.close().unwrap();
    // A page of the second picture's bytes points to a page the file does not have:
    // its bytes cannot be read, the rows can.
    let mut file = std::fs::read(lib.images()).unwrap();
    let page = (1..file.len() / 4096)
        .find(|page| {
            let data = &file[page * 4096 + 4..page * 4096 + 68];
            b_bytes.windows(data.len()).any(|window| window == data)
        })
        .expect("a page of the second picture's bytes");
    file[page * 4096..page * 4096 + 4].copy_from_slice(&0xFFFF_FFF0u32.to_be_bytes());
    std::fs::write(lib.images(), &file).unwrap();
    let store = lib.open();
    assert_eq!(store.image_bytes(&a.id).unwrap(), Some(a_bytes));
    assert_eq!(store.image_bytes(&b.id).unwrap(), Some(b_bytes));
    assert_eq!(images_moved_aside(&lib.database()).len(), 1);
    assert_eq!(store.image_info(&c.id).unwrap(), Some(c.clone()));
    assert_eq!(store.image_bytes(&c.id).unwrap(), Some(c_bytes));
}

/// Bytes that went bad inside a row (SQLite's own checks find nothing) are not
/// believed: the copy's are given and written over them, and without a copy the same
/// picture stored again repairs them.
#[test]
fn bytes_that_went_bad_are_not_believed_and_are_repaired() {
    let lib = OnDisk::new();
    let [(a, a_bytes), (b, b_bytes)] = two_pictures_and_their_copy(&lib);
    let rot = |id: &ImageId| {
        let conn = rusqlite::Connection::open(lib.images()).unwrap();
        conn.execute(
            "UPDATE images SET bytes = ?2 WHERE id = ?1",
            rusqlite::params![id.to_hex(), picture(7, 60_000)],
        )
        .unwrap();
    };
    let stored = |id: &ImageId| -> Vec<u8> {
        let conn = rusqlite::Connection::open(lib.images()).unwrap();
        conn.query_row(
            "SELECT bytes FROM images WHERE id = ?1",
            [id.to_hex()],
            |row| row.get(0),
        )
        .unwrap()
    };
    rot(&a.id);
    rot(&b.id);

    // A reader is given the copy's bytes and writes nothing.
    let reader = ImageFile::read_only(&lib.database());
    assert_eq!(reader.bytes(&a.id).unwrap(), Some(a_bytes.clone()));
    drop(reader);
    assert_eq!(stored(&a.id), picture(7, 60_000));

    // The library repairs the row from the copy.
    let mut store = lib.open();
    assert_eq!(store.image_bytes(&a.id).unwrap(), Some(a_bytes.clone()));
    assert_eq!(stored(&a.id), a_bytes);
    assert!(images_moved_aside(&lib.database()).is_empty());

    // Without a copy the bad bytes are a picture the library does not have, until the
    // same picture is stored again.
    std::fs::remove_file(lib.copy()).unwrap();
    assert_eq!(store.image_bytes(&b.id).unwrap(), None);
    assert_eq!(store.put_image(&png(&b_bytes)).unwrap(), b);
    assert_eq!(stored(&b.id), b_bytes);
    assert_eq!(store.image_bytes(&b.id).unwrap(), Some(b_bytes.clone()));

    // A row whose bytes went bad is not copied into a new copy either.
    rot(&b.id);
    back_up_all_images(&lib.database()).unwrap();
    assert_eq!(ids_in(&lib.copy()), sorted(&[a.id]));
}

/// A damaged copy does not stop the copy for good: the next run sets it aside and makes
/// a new one that holds every picture. One that cannot be written at all is said.
#[test]
fn a_damaged_copy_is_made_again_and_a_copy_that_cannot_be_written_is_said() {
    let lib = OnDisk::new();
    let [(a, _), (b, _)] = two_pictures_and_their_copy(&lib);
    scribble(&lib.copy(), 0, 100);
    let mut store = lib.open();
    let c = store.put_image(&png(&picture(3, 1_000))).unwrap();
    store.close().unwrap();

    let run = lib.collect();
    assert_eq!(run.copy_failed, None);
    assert_eq!(ids_in(&lib.copy()), sorted(&[a.id, b.id, c.id]));
    let aside = images_moved_aside(&lib.database());
    assert_eq!(aside.len(), 1);
    assert_eq!(aside[0].parent().unwrap(), backups_dir(&lib.database()));

    // The same through the look of a day, and through the copy by itself.
    scribble(&lib.copy(), 0, 100);
    assert_eq!(lib.look().copy_failed, None);
    assert_eq!(ids_in(&lib.copy()), sorted(&[a.id, b.id, c.id]));
    scribble(&lib.copy(), 0, 100);
    let done = back_up_all_images(&lib.database()).unwrap();
    assert_eq!(done.copied, 3);
    assert_eq!(images_moved_aside(&lib.database()).len(), 3);

    // Something that is no file at all where the copy goes.
    for name in ["images.db", "images.db-wal", "images.db-shm"] {
        let _ = std::fs::remove_file(backups_dir(&lib.database()).join(name));
    }
    std::fs::create_dir(lib.copy()).unwrap();
    let run = lib.collect();
    assert!(run.copy_failed.is_some(), "{run:?}");
    assert_eq!(run.images, 3);
    assert!(lib.look().copy_failed.is_some());
}

/// A connection that has the file open notices when the file at the path is another
/// one (set aside by another connection, or deleted), and opens that: nothing it stores
/// goes into a file that is no longer the library's.
#[test]
fn a_connection_follows_the_file_at_the_path() {
    let lib = OnDisk::new();
    let mut store = lib.open();
    let a = store.put_image(&png(&picture(1, 1_000))).unwrap();
    back_up_all_images(&lib.database()).unwrap();
    let mut file = ImageFile::beside(&lib.database());
    assert_eq!(file.count().unwrap(), 1);

    std::fs::rename(lib.images(), lib.dir.path().join("elsewhere.db")).unwrap();
    for suffix in ["-wal", "-shm"] {
        let _ = std::fs::rename(
            lib.dir.path().join(format!("images.db{suffix}")),
            lib.dir.path().join(format!("elsewhere.db{suffix}")),
        );
    }
    assert_eq!(file.count().unwrap(), 0);
    let b = file.put(&png(&picture(2, 1_000))).unwrap();
    assert_eq!(ids_in(&lib.images()), sorted(&[b.id]));
    // The first picture comes back from the copy, through either connection.
    assert!(file.info(&a.id).unwrap().is_some());
    assert!(store.image_bytes(&b.id).unwrap().is_some());
    assert_eq!(ids_in(&lib.images()), sorted(&[a.id, b.id]));
    assert_eq!(store.image_count().unwrap(), 2);
}

// ----- the files of pages ---------------------------------------------------------------

#[test]
fn a_file_of_a_page_is_stored_once_under_the_hash_of_its_bytes() {
    let lib = OnDisk::new();
    let mut store = lib.open();
    let bytes = page_file(1, 30_000);
    let info = store.put_file(&bytes).unwrap();
    assert_eq!(
        info,
        FileInfo {
            id: FileId::of(&bytes),
            byte_len: bytes.len() as u64,
            created_at: START,
        }
    );
    // The same bytes later: the row that is there, with its first time.
    lib.clock.advance_ms(DAY);
    assert_eq!(store.put_file(&bytes).unwrap(), info);
    assert_eq!(store.file_count().unwrap(), 1);
    assert_eq!(store.file_info(&info.id).unwrap(), Some(info.clone()));
    assert_eq!(store.file_bytes(&info.id).unwrap(), Some(bytes.clone()));
    assert_eq!(store.touch_file(&info.id).unwrap(), Some(info.clone()));
    let never = FileId::of(b"never stored");
    assert_eq!(store.file_info(&never).unwrap(), None);
    assert_eq!(store.file_bytes(&never).unwrap(), None);
    assert_eq!(store.touch_file(&never).unwrap(), None);

    // The files of pages and the pictures are two tables: the same 16 bytes name nothing
    // in the other.
    assert_eq!(store.image_count().unwrap(), 0);
    assert_eq!(store.image_info(&ImageId(info.id.0)).unwrap(), None);
    let picture_bytes = picture(1, 100);
    let image = store.put_image(&png(&picture_bytes)).unwrap();
    assert_eq!(store.file_info(&FileId(image.id.0)).unwrap(), None);
    assert_eq!(
        (store.image_count().unwrap(), store.file_count().unwrap()),
        (1, 1)
    );

    // What the row is, as a second connection reads it.
    let conn = rusqlite::Connection::open(lib.images()).unwrap();
    let (byte_len, created_at, since, stored): (i64, i64, Option<i64>, Vec<u8>) = conn
        .query_row(
            "SELECT byte_len, created_at, unreferenced_since, bytes FROM files WHERE id = ?1",
            [info.id.to_hex()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        (byte_len, created_at, since),
        (bytes.len() as i64, START, None)
    );
    assert_eq!(stored, bytes);
    drop(conn);

    // A connection of its own (the app's listener) stores and reads them too, and the
    // store sees what it stored; one for reading only refuses to store.
    let mut file = ImageFile::beside(&lib.database());
    let other = page_file(2, 10_000);
    let other_info = file.put_file(&other).unwrap();
    assert_eq!(
        store.file_bytes(&other_info.id).unwrap(),
        Some(other.clone())
    );
    assert_eq!(file.file_info(&info.id).unwrap(), Some(info.clone()));
    assert_eq!(file.file_bytes(&info.id).unwrap(), Some(bytes));
    let mut reader = ImageFile::read_only(&lib.database());
    assert_eq!(reader.file_info(&other_info.id).unwrap(), Some(other_info));
    assert!(matches!(
        reader.put_file(&page_file(3, 10)),
        Err(StoreError::Sqlite(_))
    ));
    assert_eq!(store.file_count().unwrap(), 2);

    // The file of a library in memory holds them too.
    let (mut memory, _) = common::store();
    let stored = memory.put_file(&page_file(4, 100)).unwrap();
    assert_eq!(
        memory.file_bytes(&stored.id).unwrap(),
        Some(page_file(4, 100))
    );
    assert_eq!(memory.file_count().unwrap(), 1);
}

/// `images.db` as 0.7.0 made it: version 1, the table of images and no other.
fn images_file_of_version_1(lib: &OnDisk, image: &[u8]) -> ImageId {
    let conn = rusqlite::Connection::open(lib.images()).unwrap();
    conn.pragma_update(None, "auto_vacuum", "INCREMENTAL")
        .unwrap();
    let _: String = conn
        .query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))
        .unwrap();
    conn.execute_batch(
        "CREATE TABLE images (
             id TEXT NOT NULL UNIQUE, format TEXT NOT NULL, width INTEGER NOT NULL,
             height INTEGER NOT NULL, density INTEGER NOT NULL, byte_len INTEGER NOT NULL,
             created_at INTEGER NOT NULL, unreferenced_since INTEGER, bytes BLOB NOT NULL
         );
         PRAGMA user_version = 1;
         PRAGMA application_id = 1464421703;",
    )
    .unwrap();
    let id = ImageId::of(image);
    conn.execute(
        "INSERT INTO images VALUES (?1, 'png', 4, 3, 72, ?2, ?3, NULL, ?4)",
        rusqlite::params![id.to_hex(), image.len() as i64, START, image],
    )
    .unwrap();
    id
}

/// A version-1 `images.db` is brought to version 2 by the first connection that may
/// write it, and read as it is, without files and without an error, by one that reads.
#[test]
fn an_images_file_of_version_1_is_brought_to_version_2_by_a_writer_and_read_as_it_is() {
    let lib = OnDisk::new();
    let store = lib.open();
    drop(store);
    let image = picture(1, 1_000);
    let id = images_file_of_version_1(&lib, &image);
    let some_file = FileId::of(b"a font");

    // A reader: the pictures are there, the files of pages are not, nothing is changed.
    let reader = ImageFile::read_only(&lib.database());
    assert_eq!(reader.bytes(&id).unwrap(), Some(image.clone()));
    assert_eq!(reader.file_info(&some_file).unwrap(), None);
    assert_eq!(reader.file_bytes(&some_file).unwrap(), None);
    let tool = Store::open_read_only(lib.database())
        .unwrap()
        .store()
        .unwrap();
    assert_eq!(tool.file_count().unwrap(), 0);
    assert_eq!(tool.file_info(&some_file).unwrap(), None);
    assert_eq!(tool.image_count().unwrap(), 1);
    assert_eq!(user_version(&lib.images()), 1);
    assert!(!has_table(&lib.images(), "files"));

    // A dry run reads it as it is too, and stores nothing.
    let mut store = lib.open();
    store.set_query_only(true).unwrap();
    assert_eq!(store.image_count().unwrap(), 1);
    assert_eq!(store.file_count().unwrap(), 0);
    assert!(store.put_file(b"a font").is_err());
    assert_eq!(user_version(&lib.images()), 1);
    store.set_query_only(false).unwrap();

    // The first write brings it to version 2, with its pictures as they were.
    let info = store.put_file(b"a font").unwrap();
    assert_eq!(user_version(&lib.images()), 2);
    assert!(has_table(&lib.images(), "files"));
    assert_eq!(store.image_bytes(&id).unwrap(), Some(image.clone()));
    assert_eq!(store.file_count().unwrap(), 1);
    // The readers that opened it at version 1 see the new table and its row.
    assert_eq!(reader.file_info(&info.id).unwrap(), Some(info.clone()));
    assert_eq!(tool.file_count().unwrap(), 1);
    drop((reader, tool, store));

    // Opening it for writing is enough: the collector does, and so does a store that
    // only reads a picture.
    for brings_it in ["collector", "store"] {
        std::fs::remove_file(lib.images()).unwrap();
        for suffix in ["-wal", "-shm"] {
            let _ = std::fs::remove_file(lib.dir.path().join(format!("images.db{suffix}")));
        }
        images_file_of_version_1(&lib, &image);
        match brings_it {
            "collector" => {
                let run = lib.look();
                assert_eq!((run.images, run.files), (1, 0));
            }
            _ => {
                let store = lib.open();
                assert_eq!(store.image_bytes(&id).unwrap(), Some(image.clone()));
            }
        }
        assert_eq!(user_version(&lib.images()), 2, "{brings_it}");
        assert!(has_table(&lib.images(), "files"), "{brings_it}");
    }
}

#[test]
fn the_files_that_stored_content_names_are_found() {
    let (mut store, clock) = store();
    let live = store.put_file(b"in a live page").unwrap().id;
    let trashed = store.put_file(b"in a trashed page").unwrap().id;
    let versioned = store.put_file(b"in a version").unwrap().id;
    let page = page_with(&mut store, &[live]);
    let gone = page_with(&mut store, &[trashed]);
    store.trash_document(gone.id).unwrap();
    let old = page_with(&mut store, &[versioned, live]);
    store.create_snapshot(old.id).unwrap();
    clock.advance_ms(MINUTE);
    let manifest = manifest_with(&[live]);
    store.save_page(old.id, &page_content(&manifest)).unwrap();
    assert_eq!(
        store.referenced_files().unwrap(),
        HashSet::from([live, trashed, versioned])
    );
    // The images' scan does not take a file for an image.
    assert_eq!(store.referenced_images().unwrap(), HashSet::new());
    // Purged, a page names nothing.
    store.purge_document(gone.id).unwrap();
    assert_eq!(
        store.referenced_files().unwrap(),
        HashSet::from([live, versioned])
    );
    assert!(store.document_exists(page.id).unwrap());
}

/// A file that only a version of a page names stays for as long as the version does.
#[test]
fn a_file_named_only_by_a_version_survives() {
    let lib = OnDisk::new();
    let mut store = lib.open();
    let old = store.put_file(&page_file(1, 1_000)).unwrap().id;
    let kept = store.put_file(&page_file(2, 1_000)).unwrap().id;
    let page = page_with(&mut store, &[old, kept]);
    let version = store.create_snapshot(page.id).unwrap();
    let manifest = manifest_with(&[kept]);
    store.save_page(page.id, &page_content(&manifest)).unwrap();

    for _ in 0..4 {
        lib.clock.advance_ms(40 * DAY);
        let run = lib.collect();
        assert_eq!(
            (run.files, run.files_referenced, run.files_deleted),
            (2, 2, 0)
        );
        assert_eq!(file_stamp(&lib.images(), &old), Some(None));
    }

    // The version goes; thirty days later its file does, and the page's stays.
    store.delete_snapshot(version.id).unwrap();
    assert_eq!(lib.collect().files_newly_unreferenced, 1);
    lib.clock.advance_ms(31 * DAY);
    let run = lib.collect();
    assert_eq!((run.files_deleted, run.file_bytes_freed), (1, 1_004));
    assert_eq!(file_ids_in(&lib.images()), sorted_files(&[kept]));
    assert_eq!(store.file_bytes(&old).unwrap(), None);
    assert_eq!(store.file_bytes(&kept).unwrap(), Some(page_file(2, 1_000)));
}

/// Deleting a page frees its files only through the collector: thirty days after, and
/// only once no kept backup is older than that.
#[test]
fn a_file_of_a_deleted_page_goes_after_thirty_days_and_the_oldest_backup() {
    let lib = OnDisk::new();
    let mut store = lib.open();
    let bytes = page_file(1, 5_000);
    let shared_bytes = page_file(2, 5_000);
    let id = store.put_file(&bytes).unwrap().id;
    let shared = store.put_file(&shared_bytes).unwrap().id;
    let page = page_with(&mut store, &[id, shared]);
    page_with(&mut store, &[shared]);
    let old_backup = lib.write_backup(&store);

    // In the trash the page still names its files.
    lib.clock.advance_ms(DAY);
    store.trash_document(page.id).unwrap();
    assert_eq!(lib.collect().files_referenced, 2);
    store.purge_document(page.id).unwrap();
    assert_eq!(store.file_bytes(&id).unwrap(), Some(bytes.clone()));
    assert_eq!(
        lib.collect(),
        ImagesCollected {
            files: 2,
            files_referenced: 1,
            files_newly_unreferenced: 1,
            ..ImagesCollected::default()
        }
    );
    let stamped_at = lib.clock.now_ms();

    // Thirty days are over, but the backup from before the stamp is kept.
    lib.clock.advance_ms(40 * DAY);
    assert_eq!(lib.collect().files_deleted, 0);
    assert_eq!(file_stamp(&lib.images(), &id), Some(Some(stamped_at)));
    assert_eq!(file_ids_in(&lib.copy()), sorted_files(&[id, shared]));

    // Without it the file goes, from `images.db` and from the copy; the file the other
    // page names stays.
    std::fs::remove_file(old_backup).unwrap();
    let run = lib.collect();
    assert_eq!(
        (run.files_deleted, run.file_bytes_freed),
        (1, bytes.len() as u64)
    );
    assert_eq!(store.file_bytes(&id).unwrap(), None);
    assert_eq!(file_ids_in(&lib.images()), sorted_files(&[shared]));
    assert_eq!(file_ids_in(&lib.copy()), sorted_files(&[shared]));
    assert_eq!(store.file_bytes(&shared).unwrap(), Some(shared_bytes));
}

/// A file a kept backup of the library names stays in the copy after the collector took
/// it out of `images.db`, and leaves the copy with the backup.
#[test]
fn the_copy_keeps_a_file_a_kept_backup_names() {
    let lib = OnDisk::new();
    let mut store = lib.open();
    let bytes = page_file(1, 4_000);
    let id = store.put_file(&bytes).unwrap().id;
    back_up_all_images(&lib.database()).unwrap();
    // Stamped while nothing names it ...
    assert_eq!(lib.collect().files_newly_unreferenced, 1);
    // ... then named by stored content without a touch, and backed up so.
    lib.clock.advance_ms(10 * DAY);
    let json = format!(r#"{{"file":"{id}"}}"#);
    let document = store
        .create_document(None, &content("Names a file", "x", &json))
        .unwrap();
    let backup = lib.write_backup(&store);
    store.trash_document(document.id).unwrap();
    store.purge_document(document.id).unwrap();
    lib.clock.set_ms(START + 31 * DAY);
    assert_eq!(lib.collect().files_deleted, 1);
    assert_eq!(file_ids_in(&lib.images()), Vec::<String>::new());
    // The backup names it: the copy keeps it, marked, and gives it back.
    assert_eq!(file_ids_in(&lib.copy()), sorted_files(&[id]));
    assert_eq!(file_stamp(&lib.copy(), &id), Some(Some(START + 31 * DAY)));
    lib.clock.advance_ms(DAY);
    lib.collect();
    assert_eq!(file_ids_in(&lib.copy()), sorted_files(&[id]));

    // The backup goes: so does the row.
    std::fs::remove_file(backup).unwrap();
    lib.clock.advance_ms(DAY);
    lib.collect();
    assert_eq!(file_ids_in(&lib.copy()), Vec::<String>::new());
    assert_eq!(store.file_bytes(&id).unwrap(), None);
}

#[test]
fn the_copy_has_every_file_of_a_page_and_counts_them_with_the_pictures() {
    let lib = OnDisk::new();
    let mut store = lib.open();
    let image = picture(1, 9_992);
    let first = page_file(1, 9_996);
    let second = page_file(2, 9_996);
    store.put_image(&png(&image)).unwrap();
    let a = store.put_file(&first).unwrap();
    let b = store.put_file(&second).unwrap();
    assert!(!lib.copy().exists());

    // A slice of one row at a time: the picture first, then the files, oldest first.
    let slice = back_up_images(&lib.database(), 1).unwrap();
    assert_eq!(
        slice,
        ImagesBackedUp {
            images: 1,
            copied: 1,
            bytes_copied: 10_000,
            waiting: 0,
            files: 2,
            files_copied: 0,
            file_bytes_copied: 0,
            files_waiting: 2,
        }
    );
    let slice = back_up_images(&lib.database(), 1).unwrap();
    assert_eq!(
        (slice.copied, slice.files_copied, slice.files_waiting),
        (0, 1, 1)
    );
    assert_eq!(file_ids_in(&lib.copy()), sorted_files(&[a.id]));
    // The rest, all of it.
    let done = back_up_all_images(&lib.database()).unwrap();
    assert_eq!(
        done,
        ImagesBackedUp {
            images: 1,
            files: 2,
            files_copied: 1,
            file_bytes_copied: 10_000,
            ..ImagesBackedUp::default()
        }
    );
    // Nothing is copied twice.
    assert_eq!(
        back_up_all_images(&lib.database()).unwrap(),
        ImagesBackedUp {
            images: 1,
            files: 2,
            ..ImagesBackedUp::default()
        }
    );

    // The copy is an images database with the same rows.
    let copy = ImageFile::read_only(&lib.copy().with_file_name("library.db"));
    assert_eq!(copy.file_info(&a.id).unwrap(), Some(a.clone()));
    assert_eq!(copy.file_bytes(&b.id).unwrap(), Some(second));
    assert_eq!(file_stamp(&lib.copy(), &a.id), Some(None));
}

#[test]
fn a_library_whose_images_file_was_lost_gets_the_files_of_its_pages_back_from_the_copy() {
    let lib = OnDisk::new();
    let mut store = lib.open();
    let first = page_file(1, 20_000);
    let second = page_file(2, 20_000);
    let a = store.put_file(&first).unwrap();
    let b = store.put_file(&second).unwrap();
    back_up_all_images(&lib.database()).unwrap();
    store.close().unwrap();
    for name in ["images.db", "images.db-wal", "images.db-shm"] {
        let _ = std::fs::remove_file(lib.dir.path().join(name));
    }

    // A reader finds them in the copy and writes nothing.
    let reader = ImageFile::read_only(&lib.database());
    assert_eq!(reader.file_bytes(&a.id).unwrap(), Some(first.clone()));
    assert_eq!(reader.file_info(&b.id).unwrap(), Some(b.clone()));
    assert!(!lib.images().exists());

    // The library has them again, each as it was: by its bytes, and by the page that is
    // created naming the other one.
    let mut store = lib.open();
    assert_eq!(store.file_bytes(&a.id).unwrap(), Some(first));
    assert_eq!(store.file_count().unwrap(), 1);
    page_with(&mut store, &[b.id]);
    assert_eq!(store.file_count().unwrap(), 2);
    assert_eq!(store.file_info(&b.id).unwrap(), Some(b));
    assert_eq!(
        file_ids_in(&lib.images()),
        sorted_files(&[a.id, FileId::of(&second)])
    );
}

/// What of a damaged file can still be read and the copy does not have is saved into the
/// new file, of the files of pages too.
#[test]
fn a_file_of_a_page_the_copy_does_not_have_is_saved_from_a_damaged_file() {
    let lib = OnDisk::new();
    two_pictures_and_their_copy(&lib);
    let [first, second] = the_two_files();
    // A third file, stored after the copy was brought up to date.
    let mut store = lib.open();
    let third = page_file(3, 100);
    let late = store.put_file(&third).unwrap();
    store.close().unwrap();
    // A page of the second file's bytes points to a page the file does not have.
    let mut file = std::fs::read(lib.images()).unwrap();
    let page = (1..file.len() / 4096)
        .find(|page| {
            let data = &file[page * 4096 + 4..page * 4096 + 68];
            second.windows(data.len()).any(|window| window == data)
        })
        .expect("a page of the second file's bytes");
    file[page * 4096..page * 4096 + 4].copy_from_slice(&0xFFFF_FFF0u32.to_be_bytes());
    std::fs::write(lib.images(), &file).unwrap();

    let store = lib.open();
    assert_eq!(
        store.file_bytes(&FileId::of(&second)).unwrap(),
        Some(second)
    );
    assert_eq!(images_moved_aside(&lib.database()).len(), 1);
    assert_eq!(store.file_info(&late.id).unwrap(), Some(late.clone()));
    assert_eq!(store.file_bytes(&late.id).unwrap(), Some(third));
    assert_eq!(store.file_bytes(&FileId::of(&first)).unwrap(), Some(first));
    assert_eq!(images_moved_aside(&lib.database()).len(), 1);
}

/// Bytes of a file that went bad inside its row are not believed: the copy's are given
/// and written over them, and without a copy the same file stored again repairs them.
#[test]
fn bytes_of_a_file_that_went_bad_are_taken_from_the_copy() {
    let lib = OnDisk::new();
    two_pictures_and_their_copy(&lib);
    let [first, second] = the_two_files();
    let (a, b) = (FileId::of(&first), FileId::of(&second));
    let rot = |id: &FileId| {
        let conn = rusqlite::Connection::open(lib.images()).unwrap();
        conn.execute(
            "UPDATE files SET bytes = ?2 WHERE id = ?1",
            rusqlite::params![id.to_hex(), page_file(9, 50_000)],
        )
        .unwrap();
    };
    let stored = |id: &FileId| -> Vec<u8> {
        let conn = rusqlite::Connection::open(lib.images()).unwrap();
        conn.query_row(
            "SELECT bytes FROM files WHERE id = ?1",
            [id.to_hex()],
            |row| row.get(0),
        )
        .unwrap()
    };
    rot(&a);
    rot(&b);

    let reader = ImageFile::read_only(&lib.database());
    assert_eq!(reader.file_bytes(&a).unwrap(), Some(first.clone()));
    drop(reader);
    assert_eq!(stored(&a), page_file(9, 50_000));

    let mut store = lib.open();
    assert_eq!(store.file_bytes(&a).unwrap(), Some(first.clone()));
    assert_eq!(stored(&a), first);

    std::fs::remove_file(lib.copy()).unwrap();
    assert_eq!(store.file_bytes(&b).unwrap(), None);
    assert_eq!(store.put_file(&second).unwrap().id, b);
    assert_eq!(stored(&b), second);
    assert_eq!(store.file_bytes(&b).unwrap(), Some(second));

    // A row whose bytes went bad is not copied into a new copy either.
    rot(&b);
    back_up_all_images(&lib.database()).unwrap();
    assert_eq!(file_ids_in(&lib.copy()), sorted_files(&[a]));
}
