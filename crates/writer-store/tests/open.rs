mod common;

use std::collections::BTreeMap;

use common::*;
use serde::{Deserialize, Serialize};
use writer_store::{
    Clock, OpenOptions, SCHEMA_VERSION, Store, StoreError, backups_dir, list_backups,
};

fn opts(clock: &Clock) -> OpenOptions {
    OpenOptions {
        clock: clock.clone(),
        ..OpenOptions::default()
    }
}

#[test]
fn open_creates_parent_dirs_and_migrates() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested/deeper/library.db");
    let store = Store::open(&path).unwrap();
    assert!(path.exists());
    assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
    assert_eq!(store.path(), Some(path.as_path()));
    // No backup for a brand-new library.
    assert!(store.backup_on_open().is_none());
    store.close().unwrap();
}

#[test]
fn pragmas_are_applied() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.db");
    let store = Store::open(&path).unwrap();
    // A second connection sees the persistent settings.
    let conn = rusqlite::Connection::open(&path).unwrap();
    let mode: String = conn
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .unwrap();
    assert_eq!(mode.to_lowercase(), "wal");
    let app_id: i64 = conn
        .query_row("PRAGMA application_id", [], |r| r.get(0))
        .unwrap();
    assert_eq!(app_id, 0x5752_4954);
    let fts: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name = 'documents_fts'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(fts, 1);
    drop(store);
}

#[test]
fn migrates_an_empty_database_file() {
    let dir = tempfile::tempdir().unwrap();
    // A zero-byte file and an initialised SQLite file without tables both count as empty.
    let zero = dir.path().join("zero.db");
    std::fs::write(&zero, b"").unwrap();
    let mut store = Store::open(&zero).unwrap();
    assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
    doc(&mut store, None, "works", "yes");

    let blank = dir.path().join("blank.db");
    let conn = rusqlite::Connection::open(&blank).unwrap();
    conn.execute_batch("PRAGMA user_version = 0; VACUUM;")
        .unwrap();
    drop(conn);
    let store = Store::open(&blank).unwrap();
    assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
}

#[test]
fn reopen_current_database_keeps_data() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.db");
    let clock = Clock::manual(START);
    let id = {
        let mut store = Store::open_with(&path, opts(&clock)).unwrap();
        let f = store.create_folder(None, "Kept").unwrap();
        let d = doc(&mut store, Some(f.id), "Doc", "persisted words");
        store.close().unwrap();
        d.id
    };
    let store = Store::open_with(&path, opts(&clock)).unwrap();
    assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
    assert_eq!(store.document(id).unwrap().meta.title, "Doc");
    assert_eq!(store.search("persisted", 5).unwrap().len(), 1);
}

#[test]
fn reopen_after_unclean_drop() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.db");
    let clock = Clock::manual(START);
    let mut store = Store::open_with(&path, opts(&clock)).unwrap();
    let d = doc(&mut store, None, "Crash", "survives the crash");
    store
        .save_document(d.id, &content("Crash", "saved before crash", "{\"v\":2}"))
        .unwrap();
    // Simulate a crash: the connection is never closed and the WAL never checkpointed.
    std::mem::forget(store);
    assert!(dir.path().join("library.db-wal").exists());

    let store = Store::open_with(&path, opts(&clock)).unwrap();
    let loaded = store.document(d.id).unwrap();
    assert_eq!(loaded.content, "{\"v\":2}");
    assert_eq!(store.search("before crash", 5).unwrap().len(), 1);
}

#[test]
fn newer_schema_is_refused_without_modification() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.db");
    Store::open(&path).unwrap().close().unwrap();
    // Bump user_version (header bytes 60..64) as a newer app would.
    let mut bytes = std::fs::read(&path).unwrap();
    bytes[60..64].copy_from_slice(&((SCHEMA_VERSION + 1) as u32).to_be_bytes());
    std::fs::write(&path, &bytes).unwrap();
    let before = std::fs::read(&path).unwrap();
    match Store::open(&path) {
        Err(StoreError::SchemaTooNew { found, supported }) => {
            assert_eq!(found, SCHEMA_VERSION + 1);
            assert_eq!(supported, SCHEMA_VERSION);
        }
        other => panic!("expected SchemaTooNew, got {other:?}"),
    }
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let message = Store::open(&path).unwrap_err().to_string();
    assert!(message.contains("newer version"), "{message}");
}

#[test]
fn unrelated_database_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("other.db");
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch("CREATE TABLE stuff (x); INSERT INTO stuff VALUES (1);")
        .unwrap();
    drop(conn);
    assert!(matches!(Store::open(&path), Err(StoreError::NotALibrary)));
    let foreign_id = dir.path().join("foreign.db");
    let conn = rusqlite::Connection::open(&foreign_id).unwrap();
    conn.execute_batch("PRAGMA application_id = 12345;")
        .unwrap();
    drop(conn);
    assert!(matches!(
        Store::open(&foreign_id),
        Err(StoreError::NotALibrary)
    ));
    // The foreign data is untouched, still in its original journal mode.
    let conn = rusqlite::Connection::open(&path).unwrap();
    let mode: String = conn
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .unwrap();
    assert_eq!(mode.to_lowercase(), "delete");
    let x: i64 = conn
        .query_row("SELECT x FROM stuff", [], |r| r.get(0))
        .unwrap();
    assert_eq!(x, 1);

    let garbage = dir.path().join("garbage.db");
    std::fs::write(&garbage, b"this is not a database at all, just some text").unwrap();
    assert!(Store::open(&garbage).is_err());
}

#[test]
fn daily_backups_on_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.db");
    let clock = Clock::manual(START);
    {
        let mut store = Store::open_with(&path, opts(&clock)).unwrap();
        doc(&mut store, None, "Backed up", "precious words");
        assert!(store.backup_on_open().is_none());
    }
    // Second open the same day: first backup of an existing library is written.
    let store = Store::open_with(&path, opts(&clock)).unwrap();
    let first = store.backup_on_open().unwrap().as_ref().unwrap().clone();
    assert!(first.exists());
    drop(store);
    // Again within a day: nothing.
    clock.advance_ms(HOUR);
    let store = Store::open_with(&path, opts(&clock)).unwrap();
    assert!(store.backup_on_open().is_none());
    drop(store);
    // The backup is a working library.
    let backup = Store::open_with(
        &first,
        OpenOptions {
            daily_backup: false,
            ..opts(&clock)
        },
    )
    .unwrap();
    assert_eq!(backup.search("precious", 5).unwrap().len(), 1);
    drop(backup);

    // Ten more days: only the newest few are kept.
    for _ in 0..10 {
        clock.advance_ms(DAY);
        let store = Store::open_with(
            &path,
            OpenOptions {
                keep_backups: 3,
                ..opts(&clock)
            },
        )
        .unwrap();
        assert!(matches!(store.backup_on_open(), Some(Ok(_))));
    }
    let backups = list_backups(&backups_dir(&path)).unwrap();
    assert_eq!(backups.len(), 3);
    assert!(backups[0].0 > backups[1].0);
    assert_eq!(backups[0].0, clock.now_ms());
}

#[test]
fn backup_to_explicit_path() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, _) = common::store();
    doc(&mut store, None, "Mem", "in memory text");
    let dest = dir.path().join("copy.db");
    store.backup_to(&dest).unwrap();
    store.backup_to(&dest).unwrap(); // replaces
    let copy = Store::open(&dest).unwrap();
    assert_eq!(copy.search("memory", 5).unwrap().len(), 1);
}

#[derive(Debug, PartialEq, Serialize, Deserialize, Default)]
struct Prefs {
    theme: String,
    font_size: f32,
    vim: bool,
}

#[test]
fn settings_round_trip() {
    let (mut store, _) = store();
    assert_eq!(store.setting::<bool>("vim").unwrap(), None);
    assert!(store.setting_or("vim", true).unwrap());
    store.set_setting("vim", &false).unwrap();
    assert!(!store.setting_or("vim", true).unwrap());
    let prefs = Prefs {
        theme: "Night ☾".into(),
        font_size: 17.5,
        vim: true,
    };
    store.set_setting("prefs", &prefs).unwrap();
    assert_eq!(store.setting::<Prefs>("prefs").unwrap(), Some(prefs));
    let mut map = BTreeMap::new();
    map.insert("ключ".to_owned(), vec![1u32, 2, 3]);
    store.set_setting("map", &map).unwrap();
    assert_eq!(
        store.setting::<BTreeMap<String, Vec<u32>>>("map").unwrap(),
        Some(map)
    );
    store.set_setting("name", "plain str").unwrap();
    assert_eq!(
        store.setting::<String>("name").unwrap().as_deref(),
        Some("plain str")
    );
    // Overwrite with a different type.
    store.set_setting("vim", &"yes").unwrap();
    assert!(matches!(
        store.setting::<bool>("vim"),
        Err(StoreError::Json(_))
    ));
    assert!(store.setting_or("vim", true).unwrap());
    assert_eq!(
        store.setting_keys().unwrap(),
        vec!["map", "name", "prefs", "vim"]
    );
    assert!(store.remove_setting("vim").unwrap());
    assert!(!store.remove_setting("vim").unwrap());
}

#[test]
fn settings_persist_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.db");
    {
        let mut store = Store::open(&path).unwrap();
        store.set_setting("sidebar_width", &280.0f64).unwrap();
    }
    let store = Store::open(&path).unwrap();
    assert_eq!(store.setting_or("sidebar_width", 0.0f64).unwrap(), 280.0);
}

#[test]
fn store_moves_to_another_thread() {
    let (mut store, _) = store();
    doc(&mut store, None, "t", "threaded");
    let handle = std::thread::spawn(move || store.search("threaded", 5).unwrap().len());
    assert_eq!(handle.join().unwrap(), 1);
}
