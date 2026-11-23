//! Whiteboards: library items of kind `board`, whose content is a stored scene that names
//! its pictures in `images.db` `files`. The lazy step to schema 5, created and saved with a
//! compare-and-swap, kept apart from texts and pages both ways, their pictures checked and
//! kept, duplicated, trashed and restored, searched, versioned, listed.

mod common;

use std::path::{Path, PathBuf};

use common::*;
use innerword_store::{
    BoardContent, BoardSaved, Clock, ContentStamp, DocumentId, DocumentKind, FileId, FolderId,
    IMAGE_RETENTION_MS, OPEN_SCHEMA_VERSION, OpenOptions, PageContent, SCHEMA_VERSION, Store,
    StoreError, collect_unused_images,
};

// ----- helpers -------------------------------------------------------------------------

/// A stored scene as `innerword_board::Scene::to_stored` writes it: one text element per
/// word of `texts` and one picture per id of `pictures`.
fn scene(texts: &[&str], pictures: &[FileId]) -> String {
    let mut elements: Vec<String> = texts
        .iter()
        .enumerate()
        .map(|(index, text)| {
            format!(
                r#"{{"id":"t{index}","type":"text","x":0,"y":{},"text":{},"version":1}}"#,
                index * 40,
                serde_json::to_string(text).unwrap()
            )
        })
        .collect();
    let mut files = Vec::new();
    for (index, id) in pictures.iter().enumerate() {
        elements.push(format!(
            r#"{{"id":"p{index}","type":"image","fileId":"{id}","status":"saved","version":1}}"#
        ));
        files.push(format!(
            r#""{id}":{{"mimeType":"image/png","file":"{id}","created":1}}"#
        ));
    }
    format!(
        r##"{{"type":"excalidraw","version":2,"source":"inner-word","elements":[{}],"appState":{{"viewBackgroundColor":"#ffffff","gridSize":20,"gridStep":5,"gridModeEnabled":false}},"files":{{{}}}}}"##,
        elements.join(","),
        files.join(",")
    )
}

/// The text of a scene as the caller derives it: its texts, one per line.
fn text_of(texts: &[&str]) -> String {
    texts.join("\n")
}

fn board_of<'a>(scene: &'a str, files: &'a [FileId], text: &'a str) -> BoardContent<'a> {
    BoardContent {
        scene,
        files,
        plain_text: text,
        word_count: text.split_whitespace().count() as u64,
        char_count: text.chars().count() as u64,
    }
}

/// Makes a whiteboard with these texts and pictures (stored first).
fn add_board(
    store: &mut Store,
    folder: Option<FolderId>,
    title: &str,
    texts: &[&str],
    pictures: &[FileId],
) -> BoardSaved {
    let json = scene(texts, pictures);
    let text = text_of(texts);
    store
        .create_board(folder, title, &board_of(&json, pictures, &text))
        .unwrap()
}

/// Saves new texts and pictures over a whiteboard.
fn save(
    store: &mut Store,
    id: DocumentId,
    texts: &[&str],
    pictures: &[FileId],
    expect: Option<ContentStamp>,
) -> innerword_store::Result<BoardSaved> {
    let json = scene(texts, pictures);
    let text = text_of(texts);
    store.save_board(id, &board_of(&json, pictures, &text), expect)
}

fn picture(n: u8) -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n a picture of a whiteboard ".to_vec();
    bytes.push(n);
    bytes
}

fn board_ids(store: &Store) -> Vec<i64> {
    store
        .boards()
        .unwrap()
        .into_iter()
        .map(|b| b.id.0)
        .collect()
}

fn page_of<'a>(manifest: &'a str, title: &'a str, text: &'a str) -> PageContent<'a> {
    PageContent {
        manifest,
        title,
        plain_text: text,
        word_count: text.split_whitespace().count() as u64,
        char_count: text.chars().count() as u64,
        source_url: None,
    }
}

const EMPTY_MANIFEST: &str = r#"{"page":1,"entry":"index.html","files":[],"original":[],"externals":[],"source_url":null,"fingerprint":"","added_at":0,"checked":{"engine":"WebKit","at":0,"requests":0,"warnings":[]},"text":"x"}"#;

/// A library on disk with a manual clock and no daily backup.
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

    fn user_version(&self) -> i64 {
        let conn = rusqlite::Connection::open(self.database()).unwrap();
        conn.query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap()
    }
}

/// The collector's stamp on a file as `images.db` has it; `None` when it does not have it.
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

// ----- the kind ------------------------------------------------------------------------

#[test]
fn a_kind_has_its_column_value_its_reference_and_its_noun() {
    assert_eq!(DocumentKind::Board.as_str(), "board");
    assert_eq!(DocumentKind::parse("board"), Some(DocumentKind::Board));
    assert_eq!(
        serde_json::to_string(&DocumentKind::Board).unwrap(),
        "\"board\""
    );
    let prefixes: Vec<&str> = [DocumentKind::Text, DocumentKind::Page, DocumentKind::Board]
        .iter()
        .map(|k| k.reference_prefix())
        .collect();
    assert_eq!(prefixes, ["doc", "page", "board"]);
    let nouns: Vec<&str> = [DocumentKind::Text, DocumentKind::Page, DocumentKind::Board]
        .iter()
        .map(|k| k.noun())
        .collect();
    assert_eq!(nouns, ["document", "page", "whiteboard"]);
}

#[test]
fn a_content_stamp_is_the_first_half_of_the_sha_256_of_the_content() {
    // The published digest of "abc".
    let stamp = ContentStamp::of("abc");
    assert_eq!(stamp.to_hex(), "ba7816bf8f01cfea414140de5dae2223");
    assert_eq!(stamp.to_string(), stamp.to_hex());
    assert_eq!(ContentStamp::parse(&stamp.to_hex()), Some(stamp));
    assert_eq!(
        ContentStamp::parse("BA7816BF8F01CFEA414140DE5DAE2223"),
        Some(stamp)
    );
    assert_eq!(ContentStamp::parse("ba7816bf"), None);
    assert_eq!(
        ContentStamp::parse("zz7816bf8f01cfea414140de5dae2223"),
        None
    );
    assert_ne!(ContentStamp::of("abd"), stamp);
}

// ----- schema 5, lazily ----------------------------------------------------------------

#[test]
fn a_library_stays_at_4_until_its_first_whiteboard() {
    assert_eq!((OPEN_SCHEMA_VERSION, SCHEMA_VERSION), (4, 5));
    let lib = OnDisk::new();
    let mut store = lib.open();
    assert_eq!(store.schema_version().unwrap(), 4);
    // Texts, pages, folders, versions and settings take it nowhere.
    let folder = store.create_folder(None, "Notes").unwrap();
    let text = doc(&mut store, Some(folder.id), "A text", "words");
    store
        .create_page(None, &page_of(EMPTY_MANIFEST, "A page", "x"))
        .unwrap();
    store.create_snapshot(text.id).unwrap();
    store.set_setting("some.key", &true).unwrap();
    assert!(store.boards().unwrap().is_empty());
    store.close().unwrap();
    assert_eq!(lib.user_version(), 4);
    // Nor does opening it again, read-only or not.
    let reader = Store::open_read_only(lib.database())
        .unwrap()
        .store()
        .unwrap();
    assert_eq!(reader.schema_version().unwrap(), 4);
    drop(reader);
    lib.open().close().unwrap();
    assert_eq!(lib.user_version(), 4);
}

#[test]
fn the_first_whiteboard_takes_the_library_to_5_and_its_documents_are_unchanged() {
    let lib = OnDisk::new();
    let mut store = lib.open();
    let folder = store.create_folder(None, "Plans").unwrap();
    let text = doc(&mut store, Some(folder.id), "A text", "kept words");
    let before = store.document(text.id).unwrap();
    let board = add_board(&mut store, Some(folder.id), "Plan", &["API"], &[]);
    assert_eq!(store.schema_version().unwrap(), 5);
    assert_eq!(store.document(text.id).unwrap(), before);
    // The second whiteboard has nothing more to do.
    add_board(&mut store, None, "Second", &[], &[]);
    assert_eq!(store.schema_version().unwrap(), 5);
    store.close().unwrap();
    assert_eq!(lib.user_version(), 5);

    // A library at 5 opens as it is, for writing and for reading.
    let store = lib.open();
    assert_eq!(store.schema_version().unwrap(), 5);
    assert_eq!(
        store.document(board.meta.id).unwrap().meta.kind,
        DocumentKind::Board
    );
    store.close().unwrap();
    let reader = Store::open_read_only(lib.database())
        .unwrap()
        .store()
        .unwrap();
    assert_eq!(reader.schema_version().unwrap(), 5);
    assert_eq!(reader.boards().unwrap().len(), 2);
    assert_eq!(lib.user_version(), 5);
}

#[test]
fn a_failed_first_whiteboard_leaves_the_library_at_4() {
    let lib = OnDisk::new();
    let mut store = lib.open();
    // A folder in the trash: refused inside the transaction that took the step.
    let gone = store.create_folder(None, "Gone").unwrap();
    store.trash_folder(gone.id).unwrap();
    let json = scene(&["x"], &[]);
    assert!(matches!(
        store.create_board(Some(gone.id), "Lost", &board_of(&json, &[], "x")),
        Err(StoreError::NotFound { .. })
    ));
    assert_eq!(store.schema_version().unwrap(), 4);
    // A picture that is not there: refused before it.
    let lost = FileId::of(b"never stored");
    let json = scene(&[], &[lost]);
    assert!(matches!(
        store.create_board(None, "Lost", &board_of(&json, &[lost], "")),
        Err(StoreError::FilesMissing { .. })
    ));
    assert_eq!(store.schema_version().unwrap(), 4);
    assert!(store.boards().unwrap().is_empty());
    store.close().unwrap();
    assert_eq!(lib.user_version(), 4);
}

#[test]
fn an_in_memory_library_steps_up_too() {
    let (mut store, _) = store();
    assert_eq!(store.schema_version().unwrap(), 4);
    add_board(&mut store, None, "Board", &[], &[]);
    assert_eq!(store.schema_version().unwrap(), 5);
}

// ----- creating and saving -------------------------------------------------------------

#[test]
fn a_whiteboard_is_an_item_among_the_documents_of_its_folder() {
    let (mut store, clock) = store();
    let folder = store.create_folder(None, "Plans").unwrap();
    let first = doc(&mut store, Some(folder.id), "First", "x");
    clock.advance_ms(MINUTE);
    let json = scene(&["System overview", "API"], &[]);
    let text = text_of(&["System overview", "API"]);
    let saved = store
        .create_board(
            Some(folder.id),
            "Architecture",
            &board_of(&json, &[], &text),
        )
        .unwrap();
    let meta = &saved.meta;
    assert_eq!(meta.kind, DocumentKind::Board);
    assert_eq!(meta.title, "Architecture");
    assert_eq!(meta.source_url, None);
    assert_eq!(meta.folder_id, Some(folder.id));
    assert_eq!((meta.word_count, meta.char_count), (3, 19));
    assert_eq!(
        (meta.created_at, meta.updated_at),
        (START + MINUTE, START + MINUTE)
    );
    assert_eq!(saved.stamp, ContentStamp::of(&json));
    let stored = store.document(meta.id).unwrap();
    assert_eq!(stored.content, json);
    assert_eq!(stored.view_state, None);
    assert_eq!(
        doc_titles(&store, Some(folder.id)),
        ["First", "Architecture"]
    );
    assert!(meta.position > first.position);
    // At the root, too.
    let root = add_board(&mut store, None, "", &[], &[]);
    assert_eq!(root.meta.folder_id, None);
    assert_eq!(root.meta.title, "");
}

#[test]
fn a_whiteboard_is_saved_with_new_content_and_keeps_its_title_and_place() {
    let (mut store, clock) = store();
    let folder = store.create_folder(None, "Plans").unwrap();
    let board = add_board(&mut store, Some(folder.id), "Plan", &["one"], &[]);
    let other = doc(&mut store, Some(folder.id), "After", "x");
    clock.advance_ms(MINUTE);
    // A rename between the read and the save: the save never writes a title.
    store.rename_document(board.meta.id, "Renamed").unwrap();
    clock.advance_ms(MINUTE);
    let saved = save(
        &mut store,
        board.meta.id,
        &["one", "two words"],
        &[],
        Some(board.stamp),
    )
    .unwrap();
    assert_eq!(saved.meta.title, "Renamed");
    assert_eq!(saved.meta.updated_at, START + 2 * MINUTE);
    assert_eq!((saved.meta.word_count, saved.meta.char_count), (3, 13));
    assert_eq!(saved.meta.folder_id, Some(folder.id));
    assert_eq!(saved.meta.position, board.meta.position);
    assert_ne!(saved.stamp, board.stamp);
    let stored = store.document(board.meta.id).unwrap();
    assert_eq!(saved.stamp, ContentStamp::of(&stored.content));
    assert_eq!(doc_titles(&store, Some(folder.id)), ["Renamed", "After"]);
    assert_eq!(store.document_meta(other.id).unwrap().title, "After");
    // The search index follows the text.
    assert_eq!(store.search("two", 10).unwrap().len(), 1);
    // The same text again: saved all the same.
    clock.advance_ms(MINUTE);
    let again = save(&mut store, board.meta.id, &["one", "two words"], &[], None).unwrap();
    assert_eq!(again.meta.updated_at, START + 3 * MINUTE);
    assert_eq!(again.stamp, saved.stamp);
    assert_eq!(store.search("two", 10).unwrap().len(), 1);
}

#[test]
fn a_trashed_whiteboard_is_saved_too() {
    let (mut store, _) = store();
    let board = add_board(&mut store, None, "Plan", &["one"], &[]);
    store.trash_document(board.meta.id).unwrap();
    let saved = save(&mut store, board.meta.id, &["two"], &[], Some(board.stamp)).unwrap();
    assert!(saved.meta.trashed_at.is_some());
    assert!(
        store
            .document(board.meta.id)
            .unwrap()
            .content
            .contains("two")
    );
}

// ----- compare-and-swap ----------------------------------------------------------------

#[test]
fn a_save_against_content_that_changed_is_stale_and_writes_nothing() {
    let (mut store, clock) = store();
    let board = add_board(&mut store, None, "Plan", &["base"], &[]);
    // An agent's change lands: the view's scene descends from the old content.
    clock.advance_ms(MINUTE);
    let theirs = save(&mut store, board.meta.id, &["base", "theirs"], &[], None).unwrap();
    let before = store.document(board.meta.id).unwrap();
    clock.advance_ms(MINUTE);
    match save(
        &mut store,
        board.meta.id,
        &["base", "mine"],
        &[],
        Some(board.stamp),
    ) {
        Err(StoreError::Stale { id, stored }) => {
            assert_eq!(id, board.meta.id.0);
            assert_eq!(stored, theirs.stamp);
        }
        other => panic!("expected Stale, got {other:?}"),
    }
    assert_eq!(store.document(board.meta.id).unwrap(), before);
    assert!(store.search("mine", 10).unwrap().is_empty());
    let message = StoreError::Stale {
        id: board.meta.id.0,
        stored: theirs.stamp,
    }
    .to_string();
    assert!(message.contains("changed"), "{message}");
    // Against the stamp it has now, the save goes through.
    let merged = save(
        &mut store,
        board.meta.id,
        &["base", "theirs", "mine"],
        &[],
        Some(theirs.stamp),
    )
    .unwrap();
    assert_eq!(merged.meta.updated_at, START + 2 * MINUTE);
    // Two saves of the same millisecond have different stamps.
    let next = save(
        &mut store,
        board.meta.id,
        &["base"],
        &[],
        Some(merged.stamp),
    )
    .unwrap();
    assert_eq!(next.meta.updated_at, merged.meta.updated_at);
    assert!(matches!(
        save(
            &mut store,
            board.meta.id,
            &["late"],
            &[],
            Some(merged.stamp)
        ),
        Err(StoreError::Stale { .. })
    ));
}

/// A stale save is told so before anything about its pictures: the app reads the stored
/// scene and merges, whatever the late post named.
#[test]
fn stale_comes_before_missing_files() {
    let (mut store, _) = store();
    let board = add_board(&mut store, None, "Plan", &["base"], &[]);
    let changed = save(&mut store, board.meta.id, &["changed"], &[], None).unwrap();
    let lost = FileId::of(b"never stored");
    assert!(matches!(
        save(&mut store, board.meta.id, &[], &[lost], Some(board.stamp)),
        Err(StoreError::Stale { stored, .. }) if stored == changed.stamp
    ));
}

// ----- the three kinds, both ways ------------------------------------------------------

#[test]
fn each_kind_is_saved_only_over_its_own() {
    let (mut store, _) = store();
    let board = add_board(&mut store, None, "A whiteboard", &["drawn"], &[]);
    let page = store
        .create_page(None, &page_of(EMPTY_MANIFEST, "A page", "x"))
        .unwrap();
    let text = doc(&mut store, None, "A text", "Its words");
    let befores: Vec<_> = [board.meta.id, page.id, text.id]
        .iter()
        .map(|id| store.document(*id).unwrap())
        .collect();

    let wrong = |result: StoreError| match result {
        StoreError::WrongKind { id, kind, saving } => (id, kind, saving),
        other => panic!("expected WrongKind, got {other:?}"),
    };
    let text_json = r#"{"blocks":[]}"#;
    assert_eq!(
        wrong(
            store
                .save_document(board.meta.id, &content("x", "x", text_json))
                .unwrap_err()
        ),
        (board.meta.id.0, DocumentKind::Board, DocumentKind::Text)
    );
    assert_eq!(
        wrong(
            store
                .save_page(board.meta.id, &page_of(EMPTY_MANIFEST, "x", "x"))
                .unwrap_err()
        ),
        (board.meta.id.0, DocumentKind::Board, DocumentKind::Page)
    );
    assert_eq!(
        wrong(save(&mut store, text.id, &["x"], &[], None).unwrap_err()),
        (text.id.0, DocumentKind::Text, DocumentKind::Board)
    );
    assert_eq!(
        wrong(save(&mut store, page.id, &["x"], &[], None).unwrap_err()),
        (page.id.0, DocumentKind::Page, DocumentKind::Board)
    );
    // With an expected stamp the kind is told first, too.
    assert_eq!(
        wrong(save(&mut store, text.id, &["x"], &[], Some(board.stamp)).unwrap_err()),
        (text.id.0, DocumentKind::Text, DocumentKind::Board)
    );
    let afters: Vec<_> = [board.meta.id, page.id, text.id]
        .iter()
        .map(|id| store.document(*id).unwrap())
        .collect();
    assert_eq!(afters, befores);

    // The messages name the item by its id and say what it is and what was refused.
    let message = |kind, saving| {
        StoreError::WrongKind {
            id: 12,
            kind,
            saving,
        }
        .to_string()
    };
    let over_board = message(DocumentKind::Board, DocumentKind::Text);
    assert!(over_board.contains("item 12"), "{over_board}");
    assert!(over_board.contains("is a whiteboard"), "{over_board}");
    assert!(over_board.contains("text"), "{over_board}");
    let over_text = message(DocumentKind::Text, DocumentKind::Board);
    assert!(over_text.contains("is a document"), "{over_text}");
    assert!(
        over_text.contains("a whiteboard cannot be saved"),
        "{over_text}"
    );
    let over_page = message(DocumentKind::Page, DocumentKind::Board);
    assert!(over_page.contains("is a page"), "{over_page}");

    // A missing item is missing.
    assert!(matches!(
        save(&mut store, DocumentId(999), &["x"], &[], None),
        Err(StoreError::NotFound { .. })
    ));
    assert!(matches!(
        save(&mut store, DocumentId(999), &["x"], &[], Some(board.stamp)),
        Err(StoreError::NotFound { .. })
    ));
}

// ----- pictures ------------------------------------------------------------------------

#[test]
fn a_whiteboard_that_names_a_new_picture_the_library_does_not_have_is_not_written() {
    let (mut store, clock) = store();
    let there = store.put_file(&picture(1)).unwrap().id;
    let lost = FileId::of(b"never stored");
    let also_lost = FileId::of(b"never stored either");
    let files = [there, lost, also_lost, lost];
    let json = scene(&["x"], &files);
    match store.create_board(None, "Half", &board_of(&json, &files, "x")) {
        Err(StoreError::FilesMissing { ids }) => assert_eq!(ids, vec![lost, also_lost]),
        other => panic!("expected FilesMissing, got {other:?}"),
    }
    assert!(store.documents().unwrap().is_empty());
    let message = StoreError::FilesMissing {
        ids: vec![lost, also_lost],
    }
    .to_string();
    assert!(message.contains("the item names 2 files"), "{message}");
    assert!(message.contains(&lost.to_hex()), "{message}");

    // A save that would name a new missing picture changes nothing either.
    let board = add_board(&mut store, None, "Whole", &["whole"], &[there]);
    let before = store.document(board.meta.id).unwrap();
    clock.advance_ms(MINUTE);
    assert!(matches!(
        store.save_board(board.meta.id, &board_of(&json, &files, "x"), Some(board.stamp)),
        Err(StoreError::FilesMissing { ids }) if ids == vec![lost, also_lost]
    ));
    assert_eq!(store.document(board.meta.id).unwrap(), before);

    // Stored, the same scene is taken.
    store.put_file(b"never stored").unwrap();
    store.put_file(b"never stored either").unwrap();
    store
        .save_board(
            board.meta.id,
            &board_of(&json, &files, "x"),
            Some(board.stamp),
        )
        .unwrap();
}

/// What the store checks is what the caller says the scene names, not a scan of it: a
/// `"file"` in an element's `customData` that names nothing stored never refuses a save.
#[test]
fn the_store_checks_the_files_it_is_given_and_does_not_scan_the_scene() {
    let (mut store, _) = store();
    let stray = FileId::of(b"only in customData");
    let json = format!(
        r#"{{"type":"excalidraw","version":2,"source":"inner-word","elements":[{{"id":"a","type":"rectangle","customData":{{"file":"{stray}"}}}}],"appState":{{}},"files":{{}}}}"#
    );
    let board = store
        .create_board(None, "Pasted", &board_of(&json, &[], ""))
        .unwrap();
    store
        .save_board(board.meta.id, &board_of(&json, &[], ""), Some(board.stamp))
        .unwrap();
}

/// A `library.db` brought back without the copy of its pictures: the whiteboard names
/// pictures the library lost, shows them as missing, and goes on saving.
#[test]
fn a_missing_picture_the_whiteboard_or_a_version_named_already_does_not_refuse() {
    let lib = OnDisk::new();
    let mut store = lib.open();
    let old = store.put_file(&picture(1)).unwrap().id;
    let current = store.put_file(&picture(2)).unwrap().id;
    let board = add_board(&mut store, None, "Plan", &["one"], &[old]);
    store.create_snapshot(board.meta.id).unwrap();
    let saved = save(
        &mut store,
        board.meta.id,
        &["two"],
        &[current],
        Some(board.stamp),
    )
    .unwrap();
    store.close().unwrap();
    // images.db is lost (and there is no copy among the backups).
    std::fs::remove_file(innerword_store::images_path(&lib.database())).unwrap();
    let mut store = lib.open();
    assert_eq!(store.file_info(&current).unwrap(), None);

    // The row names `current`, a version names `old`: both are kept.
    let saved = save(
        &mut store,
        board.meta.id,
        &["three"],
        &[current, old],
        Some(saved.stamp),
    )
    .unwrap();
    // A new id that is missing is still refused, and only it is named.
    let new = FileId::of(b"new and never stored");
    match save(
        &mut store,
        board.meta.id,
        &["four"],
        &[current, new, old],
        Some(saved.stamp),
    ) {
        Err(StoreError::FilesMissing { ids }) => assert_eq!(ids, vec![new]),
        other => panic!("expected FilesMissing, got {other:?}"),
    }
    // Another whiteboard cannot borrow the first one's names.
    let other = add_board(&mut store, None, "Other", &[], &[]);
    match save(&mut store, other.meta.id, &[], &[old], Some(other.stamp)) {
        Err(StoreError::FilesMissing { ids }) => assert_eq!(ids, vec![old]),
        other => panic!("expected FilesMissing, got {other:?}"),
    }
}

/// A save clears the collector's stamp on every picture it names that the library has.
#[test]
fn a_save_counts_its_pictures_as_used() {
    let lib = OnDisk::new();
    let mut store = lib.open();
    let id = store.put_file(&picture(1)).unwrap().id;
    // Nothing names it yet: the collector's look stamps it.
    let run = innerword_store::mark_unused_images(&lib.database(), lib.clock.now_ms()).unwrap();
    assert_eq!(run.files_newly_unreferenced, 1);
    let images = innerword_store::images_path(&lib.database());
    assert_eq!(file_stamp(&images, &id), Some(Some(START)));
    lib.clock.advance_ms(DAY);
    add_board(&mut store, None, "Plan", &[], &[id]);
    assert_eq!(file_stamp(&images, &id), Some(None));
}

/// A picture that only a version of a whiteboard names stays for as long as the version
/// does.
#[test]
fn a_file_named_only_by_a_version_of_a_whiteboard_survives_the_collector() {
    let lib = OnDisk::new();
    let mut store = lib.open();
    let old = store.put_file(&picture(1)).unwrap().id;
    let kept = store.put_file(&picture(2)).unwrap().id;
    let board = add_board(&mut store, None, "Plan", &["one"], &[old, kept]);
    let version = store.create_snapshot(board.meta.id).unwrap();
    save(
        &mut store,
        board.meta.id,
        &["two"],
        &[kept],
        Some(board.stamp),
    )
    .unwrap();
    let images = innerword_store::images_path(&lib.database());
    let collect =
        || collect_unused_images(&lib.database(), lib.clock.now_ms(), IMAGE_RETENTION_MS).unwrap();
    for _ in 0..3 {
        lib.clock.advance_ms(40 * DAY);
        let run = collect();
        assert_eq!(
            (run.files, run.files_referenced, run.files_deleted),
            (2, 2, 0)
        );
        assert_eq!(file_stamp(&images, &old), Some(None));
    }
    // The version goes; thirty days later its picture does, and the whiteboard's stays.
    store.delete_snapshot(version.id).unwrap();
    assert_eq!(collect().files_newly_unreferenced, 1);
    lib.clock.advance_ms(31 * DAY);
    assert_eq!(collect().files_deleted, 1);
    assert_eq!(store.file_bytes(&old).unwrap(), None);
    assert_eq!(store.file_bytes(&kept).unwrap(), Some(picture(2)));
    // In the trash, the whiteboard still keeps its picture.
    store.trash_document(board.meta.id).unwrap();
    lib.clock.advance_ms(40 * DAY);
    assert_eq!(collect().files_deleted, 0);
    assert_eq!(file_stamp(&images, &kept), Some(None));
}

// ----- the rest of an item's life ------------------------------------------------------

#[test]
fn a_duplicate_of_a_whiteboard_is_a_whiteboard_that_names_the_same_pictures() {
    let (mut store, _) = store();
    let id = store.put_file(&picture(1)).unwrap().id;
    let board = add_board(&mut store, None, "Plan", &["API"], &[id]);
    store
        .save_view_state(board.meta.id, r#"{"zoom":1.25}"#)
        .unwrap();
    let copy = store.duplicate_document(board.meta.id).unwrap();
    assert_eq!(copy.kind, DocumentKind::Board);
    assert_eq!(copy.title, "Plan copy");
    let original = store.document(board.meta.id).unwrap();
    let copied = store.document(copy.id).unwrap();
    assert_eq!(copied.content, original.content);
    assert_eq!(copied.view_state, original.view_state);
    assert_eq!(copy.word_count, 1);
    assert_eq!(store.file_count().unwrap(), 1);
    assert_eq!(
        store.referenced_files().unwrap(),
        [id].into_iter().collect()
    );
    // The copy saves as a whiteboard, against its own stamp.
    let stamp = ContentStamp::of(&copied.content);
    save(&mut store, copy.id, &["Copied"], &[id], Some(stamp)).unwrap();
    assert_eq!(store.boards().unwrap().len(), 2);
}

#[test]
fn a_whiteboard_goes_to_the_trash_and_comes_back_as_it_was() {
    let (mut store, _) = store();
    let folder = store.create_folder(None, "Plans").unwrap();
    let board = add_board(&mut store, Some(folder.id), "Plan", &["API"], &[]);
    let before = store.document(board.meta.id).unwrap();
    store.trash_document(board.meta.id).unwrap();
    assert!(doc_titles(&store, Some(folder.id)).is_empty());
    let trash = store.trash().unwrap();
    assert_eq!(trash.len(), 1);
    let restored = store.restore_document(board.meta.id).unwrap();
    assert_eq!(restored.kind, DocumentKind::Board);
    assert_eq!(restored.folder_id, Some(folder.id));
    let after = store.document(board.meta.id).unwrap();
    assert_eq!(after.content, before.content);
    assert_eq!(after.meta.kind, DocumentKind::Board);
    // Moved, it stays what it is and keeps its content.
    store.move_document(board.meta.id, None).unwrap();
    assert_eq!(
        store.document(board.meta.id).unwrap().content,
        before.content
    );
    // Deleted for good, it is gone.
    store.trash_document(board.meta.id).unwrap();
    store.purge_document(board.meta.id).unwrap();
    assert!(store.boards().unwrap().is_empty());
}

#[test]
fn search_finds_the_words_of_a_label_and_says_it_is_a_whiteboard() {
    let (mut store, _) = store();
    let board = add_board(
        &mut store,
        None,
        "Architecture",
        &["System overview", "Postgres", "reads"],
        &[],
    );
    doc(&mut store, None, "Notes", "Nothing about databases");
    let hits = store.search("postgres", 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].document_id, board.meta.id);
    assert_eq!(hits[0].kind, DocumentKind::Board);
    assert!(hits[0].snippet.contains("Postgres"));
    // By its title too, and the new words after a save.
    assert_eq!(store.search("architecture", 10).unwrap().len(), 1);
    save(
        &mut store,
        board.meta.id,
        &["MySQL"],
        &[],
        Some(board.stamp),
    )
    .unwrap();
    assert!(store.search("postgres", 10).unwrap().is_empty());
    assert_eq!(store.search("mysql", 10).unwrap().len(), 1);
}

#[test]
fn a_version_of_a_whiteboard_holds_its_scene_and_is_brought_back_by_a_save() {
    let (mut store, clock) = store();
    let board = add_board(&mut store, None, "Plan", &["first"], &[]);
    let first = store.document(board.meta.id).unwrap().content;
    clock.advance_ms(HOUR);
    let version = store
        .snapshot_if_stale(board.meta.id, std::time::Duration::from_secs(60))
        .unwrap()
        .unwrap();
    let second = save(
        &mut store,
        board.meta.id,
        &["second"],
        &[],
        Some(board.stamp),
    )
    .unwrap();
    let snapshot = store.snapshot(version.id).unwrap();
    assert_eq!(snapshot.content, first);
    // Restoring is a save of the version's scene over the current one.
    let restored = store
        .save_board(
            board.meta.id,
            &board_of(&snapshot.content, &[], "first"),
            Some(second.stamp),
        )
        .unwrap();
    assert_eq!(restored.stamp, board.stamp);
    assert_eq!(store.document(board.meta.id).unwrap().content, first);
}

#[test]
fn the_list_of_whiteboards_is_every_whiteboard_most_recently_updated_first() {
    let (mut store, clock) = store();
    let a = add_board(&mut store, None, "A", &[], &[]);
    clock.advance_ms(MINUTE);
    let b = add_board(&mut store, None, "B", &[], &[]);
    clock.advance_ms(MINUTE);
    doc(&mut store, None, "A text", "x");
    store
        .create_page(None, &page_of(EMPTY_MANIFEST, "A page", "x"))
        .unwrap();
    let c = add_board(&mut store, None, "C", &[], &[]);
    clock.advance_ms(MINUTE);
    store.trash_document(b.meta.id).unwrap();
    save(&mut store, a.meta.id, &["new"], &[], Some(a.stamp)).unwrap();
    assert_eq!(board_ids(&store), [a.meta.id.0, c.meta.id.0, b.meta.id.0]);
    let boards = store.boards().unwrap();
    assert!(boards.iter().all(|m| m.kind == DocumentKind::Board));
    assert!(boards[2].trashed_at.is_some());
    // Pages are not whiteboards, nor the reverse.
    assert!(
        store
            .pages()
            .unwrap()
            .iter()
            .all(|m| m.kind == DocumentKind::Page)
    );
    assert_eq!(store.pages().unwrap().len(), 1);
}

#[test]
fn a_library_on_disk_keeps_its_whiteboards_across_a_reopening() {
    let lib = OnDisk::new();
    let mut store = lib.open();
    let id = store.put_file(&picture(1)).unwrap().id;
    let board = add_board(&mut store, None, "Plan", &["kept"], &[id]);
    store.close().unwrap();
    let mut store = lib.open();
    let stored = store.document(board.meta.id).unwrap();
    assert_eq!(stored.meta.kind, DocumentKind::Board);
    assert_eq!(ContentStamp::of(&stored.content), board.stamp);
    assert_eq!(store.file_bytes(&id).unwrap(), Some(picture(1)));
    save(
        &mut store,
        board.meta.id,
        &["again"],
        &[id],
        Some(board.stamp),
    )
    .unwrap();
}
