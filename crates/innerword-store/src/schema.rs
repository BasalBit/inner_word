//! Connection setup and schema migrations.
//!
//! The schema version lives in SQLite's `user_version` header field. [`MIGRATIONS`] is an
//! append-only list: step `i` upgrades a database from version `i` to `i + 1`, and runs in
//! its own transaction together with the `user_version` bump, so a crash mid-migration
//! leaves the previous version intact. To change the schema, append a step; never edit a
//! released one.

use std::time::Duration;

use rusqlite::{Connection, TransactionBehavior};

use crate::error::{Result, StoreError};

/// `application_id` written into the database header ("WRIT"), so the store can refuse to
/// open unrelated SQLite files.
pub(crate) const APPLICATION_ID: i32 = 0x5752_4954;

/// The newest schema version this build understands. A library is taken to it lazily:
/// opening one brings it to [`OPEN_SCHEMA_VERSION`] only, and the step to 5 is taken by
/// the first whiteboard made in it ([`crate::Store::create_board`]), so that a library in
/// which nobody made a whiteboard stays readable by a version that knows schema 4.
pub const SCHEMA_VERSION: i64 = MIGRATIONS.len() as i64;

/// The schema version an open migrates a library to (and creates a new one at). The steps
/// above it are taken by the operation that first needs them, inside its own transaction.
pub const OPEN_SCHEMA_VERSION: i64 = 4;

const MIGRATIONS: &[&str] = &[V1, V2, V3, V4, V5];

const V1: &str = r#"
CREATE TABLE folders (
    id           INTEGER PRIMARY KEY,
    parent_id    INTEGER REFERENCES folders(id) ON DELETE SET NULL,
    name         TEXT    NOT NULL,
    position     INTEGER NOT NULL DEFAULT 0,
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL,
    trashed_at   INTEGER,
    -- The top-level trashed folder this row was trashed together with (NULL when the row
    -- itself is a top-level trash entry or not trashed).
    trashed_with INTEGER REFERENCES folders(id) ON DELETE CASCADE
);
CREATE INDEX folders_by_parent ON folders(parent_id, position);
CREATE INDEX folders_by_trashed_with ON folders(trashed_with);

CREATE TABLE documents (
    id           INTEGER PRIMARY KEY,
    folder_id    INTEGER REFERENCES folders(id) ON DELETE SET NULL,
    title        TEXT    NOT NULL,
    content      TEXT    NOT NULL,
    plain_text   TEXT    NOT NULL,
    word_count   INTEGER NOT NULL DEFAULT 0,
    char_count   INTEGER NOT NULL DEFAULT 0,
    position     INTEGER NOT NULL DEFAULT 0,
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL,
    view_state   TEXT,
    trashed_at   INTEGER,
    trashed_with INTEGER REFERENCES folders(id) ON DELETE CASCADE
);
CREATE INDEX documents_by_folder ON documents(folder_id, position);
CREATE INDEX documents_by_updated ON documents(updated_at);
CREATE INDEX documents_by_trashed_with ON documents(trashed_with);

CREATE TABLE snapshots (
    id          INTEGER PRIMARY KEY,
    document_id INTEGER NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    created_at  INTEGER NOT NULL,
    title       TEXT    NOT NULL,
    word_count  INTEGER NOT NULL,
    char_count  INTEGER NOT NULL,
    content     TEXT    NOT NULL
);
CREATE INDEX snapshots_by_document ON snapshots(document_id, created_at);

CREATE TABLE settings (
    key        TEXT PRIMARY KEY,
    value      TEXT NOT NULL,
    updated_at INTEGER NOT NULL
) WITHOUT ROWID;

CREATE VIRTUAL TABLE documents_fts USING fts5(
    title,
    plain_text,
    content = 'documents',
    content_rowid = 'id',
    tokenize = 'unicode61 remove_diacritics 2',
    prefix = '2 3'
);

CREATE TRIGGER documents_fts_insert AFTER INSERT ON documents BEGIN
    INSERT INTO documents_fts(rowid, title, plain_text)
    VALUES (new.id, new.title, new.plain_text);
END;

CREATE TRIGGER documents_fts_delete AFTER DELETE ON documents BEGIN
    INSERT INTO documents_fts(documents_fts, rowid, title, plain_text)
    VALUES ('delete', old.id, old.title, old.plain_text);
END;

CREATE TRIGGER documents_fts_update AFTER UPDATE OF title, plain_text ON documents BEGIN
    INSERT INTO documents_fts(documents_fts, rowid, title, plain_text)
    VALUES ('delete', old.id, old.title, old.plain_text);
    INSERT INTO documents_fts(rowid, title, plain_text)
    VALUES (new.id, new.title, new.plain_text);
END;
"#;

/// Version 2: an index that holds everything a document listing shows.
///
/// In the `documents` table the counts, the position and the timestamps are stored after
/// the content and the plain text, so reading them makes SQLite walk through the text of
/// every document: listing the library cost time in proportion to everything ever written
/// (22 ms for 5,000 documents of 2,000 words, on every reload of the tree). With this
/// index the listings of live documents never touch the text.
const V2: &str = r#"
CREATE INDEX documents_meta ON documents(
    trashed_at, position, id, folder_id, title, word_count, char_count, created_at, updated_at
);
"#;

/// Version 3: documents may hold images (block type "image"; the bytes are in images.db).
///
/// The step changes nothing in this file. It exists so that a version of the app or of
/// the `innerword` tool that does not know images refuses the library
/// ([`StoreError::SchemaTooNew`]) instead of reading an image as an empty paragraph and
/// saving that.
const V3: &str = r#"
SELECT 1;
"#;

/// Version 4: library items may be pages (kind 'page'; their files are in images.db).
///
/// Two columns: `kind` (`'text'` for every row there is, `'page'` for a page) and
/// `source_url` (the address a page came from). The covering index of the listings is
/// made again with both, so that listings still never read the text, and an index of the
/// addresses answers the lookup of a page that is added again. No `CHECK` on `kind`: an
/// unknown value is an error of that row when it is read. A version that does not know
/// pages refuses the library ([`StoreError::SchemaTooNew`]) instead of showing a page's
/// manifest as text and saving that over it.
const V4: &str = r#"
ALTER TABLE documents ADD COLUMN kind TEXT NOT NULL DEFAULT 'text';
ALTER TABLE documents ADD COLUMN source_url TEXT;
DROP INDEX documents_meta;
CREATE INDEX documents_meta ON documents(
    trashed_at, position, id, folder_id, title, word_count, char_count, created_at,
    updated_at, kind, source_url
);
CREATE INDEX documents_by_source_url ON documents(source_url)
    WHERE source_url IS NOT NULL;
"#;

/// Version 5: library items may be whiteboards (kind 'board'; their pictures are in
/// images.db `files`, named by "file" in the scene as a page's files are in its manifest).
///
/// The step changes nothing in this file: `kind` has no CHECK (version 4). It exists so
/// that a version of the app or of the `innerword` tool that does not know whiteboards
/// refuses the library ([`StoreError::SchemaTooNew`]) instead of failing on a row of an
/// unknown kind, or treating a whiteboard as a text and saving over it.
///
/// It is taken lazily, by [`take_steps_above_open`] in the transaction of the first
/// whiteboard ([`crate::Store::create_board`]), never by an open.
const V5: &str = r#"
SELECT 1;
"#;

/// Applies the per-connection pragmas that do not modify the file: busy timeout, foreign
/// keys, in-memory temp storage.
pub(crate) fn configure(conn: &Connection) -> Result<()> {
    conn.busy_timeout(Duration::from_secs(5))?;
    conn.pragma_update(None, "foreign_keys", true)?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    Ok(())
}

/// Switches an on-disk database to WAL with `synchronous=FULL`. Called only after
/// [`inspect`] accepted the file, because changing the journal mode writes the header.
///
/// WAL lets readers run during writes and makes commits atomic on disk; `synchronous=FULL`
/// fsyncs the WAL on every commit, so a committed save survives an app crash and power loss.
pub(crate) fn enable_wal(conn: &Connection) -> Result<()> {
    let mode: String = conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(StoreError::JournalMode(mode));
    }
    conn.pragma_update(None, "synchronous", "FULL")?;
    Ok(())
}

/// What [`inspect`] found in the database header.
pub(crate) struct Header {
    pub(crate) user_version: i64,
}

/// Validates that the file is ours (or empty) and not from a newer app version.
pub(crate) fn inspect(conn: &Connection) -> Result<Header> {
    inspect_as(conn, SCHEMA_VERSION)
}

/// [`inspect`] as a build does it whose newest schema version is `supported`.
fn inspect_as(conn: &Connection, supported: i64) -> Result<Header> {
    let application_id: i32 = conn.query_row("PRAGMA application_id", [], |r| r.get(0))?;
    let user_version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if application_id != 0 && application_id != APPLICATION_ID {
        return Err(StoreError::NotALibrary);
    }
    if user_version == 0 {
        let tables: i64 = conn.query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
            [],
            |r| r.get(0),
        )?;
        if tables > 0 {
            return Err(StoreError::NotALibrary);
        }
    }
    if user_version > supported {
        return Err(StoreError::SchemaTooNew {
            found: user_version,
            supported,
        });
    }
    if user_version < 0 {
        return Err(StoreError::NotALibrary);
    }
    Ok(Header { user_version })
}

/// Runs every migration step up to [`OPEN_SCHEMA_VERSION`] the database has not seen yet.
/// A library already above it (one that holds a whiteboard) is left as it is.
pub(crate) fn migrate(conn: &mut Connection) -> Result<()> {
    migrate_to(conn, OPEN_SCHEMA_VERSION)
}

/// Inside the caller's transaction: takes the database from the version it is at (at
/// least [`OPEN_SCHEMA_VERSION`], what an open left) to [`SCHEMA_VERSION`], running the
/// steps it has not seen yet. Nothing is committed here: when the caller's transaction
/// is rolled back, the steps and the version go with it. Called by the operation that
/// first writes what a later step announces (a whiteboard: [`crate::Store::create_board`]).
pub(crate) fn take_steps_above_open(tx: &rusqlite::Transaction<'_>) -> Result<()> {
    let current: i64 = tx.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if current >= SCHEMA_VERSION {
        return Ok(());
    }
    let from = usize::try_from(current).unwrap_or(0);
    for (index, step) in MIGRATIONS.iter().enumerate().skip(from) {
        tx.execute_batch(step)?;
        tx.pragma_update(None, "user_version", index as i64 + 1)?;
    }
    tx.pragma_update(None, "application_id", APPLICATION_ID)?;
    Ok(())
}

/// Runs the migration steps the database has not seen yet, up to version `target`.
fn migrate_to(conn: &mut Connection, target: i64) -> Result<()> {
    let header = inspect(conn)?;
    for (index, step) in MIGRATIONS
        .iter()
        .enumerate()
        .take(usize::try_from(target).unwrap_or(0))
        .skip(header.user_version as usize)
    {
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        // Re-check inside the write lock: another process may have migrated meanwhile.
        let current: i64 = tx.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if current > index as i64 {
            tx.commit()?;
            continue;
        }
        tx.execute_batch(step)?;
        tx.pragma_update(None, "user_version", index as i64 + 1)?;
        tx.pragma_update(None, "application_id", APPLICATION_ID)?;
        tx.commit()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A database as version 1 of the app left it, with one document.
    fn version_1() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        configure(&conn).unwrap();
        conn.execute_batch(V1).unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();
        conn.pragma_update(None, "application_id", APPLICATION_ID)
            .unwrap();
        conn.execute(
            "INSERT INTO documents (title, content, plain_text, word_count, char_count,
                 position, created_at, updated_at)
             VALUES ('Kept', '{}', 'kept words', 2, 10, 0, 5, 6)",
            [],
        )
        .unwrap();
        conn
    }

    fn plan(conn: &Connection, sql: &str) -> String {
        let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
        let rows: Vec<String> = stmt
            .query_map([], |row| row.get(3))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        rows.join("; ")
    }

    #[test]
    fn version_1_upgrades_in_place() {
        let mut conn = version_1();
        migrate(&mut conn).unwrap();
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, OPEN_SCHEMA_VERSION);
        assert_eq!(OPEN_SCHEMA_VERSION, 4);
        assert_eq!(SCHEMA_VERSION, 5);
        let (title, words): (String, i64) = conn
            .query_row("SELECT title, word_count FROM documents", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!((title.as_str(), words), ("Kept", 2));
        // The search index is untouched by the upgrade.
        let hits: i64 = conn
            .query_row(
                "SELECT count(*) FROM documents_fts WHERE documents_fts MATCH 'kept'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hits, 1);
        // Running it again changes nothing.
        migrate(&mut conn).unwrap();
    }

    /// A database as version 2 of the app (0.6.0) left it, with one document.
    fn version_2() -> Connection {
        let conn = version_1();
        conn.execute_batch(V2).unwrap();
        conn.pragma_update(None, "user_version", 2).unwrap();
        conn
    }

    /// Every row of `sql`, each cell as SQLite has it.
    fn rows_of(conn: &Connection, sql: &str) -> Vec<String> {
        let mut rows = Vec::new();
        let mut stmt = conn.prepare(sql).unwrap();
        let columns = stmt.column_count();
        let mut found = stmt.query([]).unwrap();
        while let Some(row) = found.next().unwrap() {
            let cells: Vec<String> = (0..columns)
                .map(|i| format!("{:?}", row.get::<_, rusqlite::types::Value>(i).unwrap()))
                .collect();
            rows.push(cells.join("|"));
        }
        rows
    }

    /// The documents as a version up to 3 has them, column for column.
    const DOCUMENTS_UP_TO_3: &str = "SELECT id, folder_id, title, content, plain_text,
            word_count, char_count, position, created_at, updated_at, view_state, trashed_at
         FROM documents ORDER BY id";

    /// Step 3 changes nothing a version-2 library holds: it only moves the version on.
    #[test]
    fn version_2_upgrades_to_3_and_its_documents_are_unchanged() {
        fn everything(conn: &Connection) -> Vec<String> {
            let mut rows = rows_of(conn, DOCUMENTS_UP_TO_3);
            rows.extend(rows_of(
                conn,
                "SELECT type, name, tbl_name, sql FROM sqlite_schema ORDER BY name",
            ));
            rows
        }
        let mut conn = version_2();
        let before = everything(&conn);
        assert!(before.iter().any(|row| row.contains("Kept")));
        migrate_to(&mut conn, 3).unwrap();
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 3);
        assert_eq!(everything(&conn), before);
    }

    /// What step 3 is for: a build whose newest schema is 2 (0.6.0, the app and the
    /// `innerword` tool) refuses a library that may hold images, and one that knows 3 opens
    /// it.
    #[test]
    fn a_build_that_knows_schema_2_refuses_a_library_of_schema_3() {
        let mut conn = version_2();
        assert!(inspect_as(&conn, 2).is_ok());
        migrate_to(&mut conn, 3).unwrap();
        match inspect_as(&conn, 2) {
            Err(StoreError::SchemaTooNew { found, supported }) => {
                assert_eq!((found, supported), (3, 2));
            }
            other => panic!(
                "expected SchemaTooNew, got {:?}",
                other.map(|h| h.user_version)
            ),
        }
        assert_eq!(inspect_as(&conn, 3).unwrap().user_version, 3);
    }

    /// A database as version 3 of the app (0.7.0) left it, with one document.
    fn version_3() -> Connection {
        let mut conn = version_2();
        migrate_to(&mut conn, 3).unwrap();
        conn
    }

    /// Step 4 adds the two columns of pages; every document there is stays as it was, a
    /// text without an address, and the search index finds it as before.
    #[test]
    fn version_3_upgrades_to_4_and_its_documents_are_unchanged() {
        let mut conn = version_3();
        let before = rows_of(&conn, DOCUMENTS_UP_TO_3);
        assert!(before.iter().any(|row| row.contains("Kept")));
        migrate(&mut conn).unwrap();
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 4);
        assert_eq!(rows_of(&conn, DOCUMENTS_UP_TO_3), before);
        assert_eq!(
            rows_of(&conn, "SELECT kind, source_url FROM documents ORDER BY id"),
            vec![r#"Text("text")|Null"#.to_owned()]
        );
        let meta = conn
            .query_row(
                &format!("SELECT {} FROM documents", crate::documents::DOC_META_COLS),
                [],
                crate::documents::meta_from_row,
            )
            .unwrap();
        assert_eq!(meta.kind, crate::DocumentKind::Text);
        assert_eq!(meta.source_url, None);
        assert_eq!((meta.title.as_str(), meta.word_count), ("Kept", 2));
        let hits: i64 = conn
            .query_row(
                "SELECT count(*) FROM documents_fts WHERE documents_fts MATCH 'kept'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hits, 1);
        // The index of the addresses is there, and only for rows that have one.
        let indexes = rows_of(
            &conn,
            "SELECT name FROM sqlite_schema WHERE type = 'index' AND tbl_name = 'documents'
             ORDER BY name",
        );
        assert!(
            indexes
                .iter()
                .any(|name| name.contains("documents_by_source_url"))
        );
        assert!(indexes.iter().any(|name| name.contains("documents_meta")));
        migrate(&mut conn).unwrap();
    }

    /// What step 4 is for: a build whose newest schema is 3 (0.7.0, the app and the
    /// `innerword` tool) refuses a library that may hold pages, and one that knows 4 opens it.
    #[test]
    fn a_build_that_knows_schema_3_refuses_a_library_of_schema_4() {
        let mut conn = version_3();
        assert!(inspect_as(&conn, 3).is_ok());
        migrate(&mut conn).unwrap();
        match inspect_as(&conn, 3) {
            Err(StoreError::SchemaTooNew { found, supported }) => {
                assert_eq!((found, supported), (4, 3));
            }
            other => panic!(
                "expected SchemaTooNew, got {:?}",
                other.map(|h| h.user_version)
            ),
        }
        assert_eq!(inspect(&conn).unwrap().user_version, 4);
    }

    /// A database as version 4 of the app (0.10.1) leaves it, with one document: what an
    /// open of this build leaves too.
    fn version_4() -> Connection {
        let mut conn = version_3();
        migrate(&mut conn).unwrap();
        conn
    }

    /// An open never takes the step to 5: a library at 4 stays at 4, one at 5 stays at 5.
    #[test]
    fn an_open_migrates_to_4_and_leaves_5_alone() {
        let mut conn = version_4();
        migrate(&mut conn).unwrap();
        assert_eq!(inspect(&conn).unwrap().user_version, OPEN_SCHEMA_VERSION);
        let tx = conn.transaction().unwrap();
        take_steps_above_open(&tx).unwrap();
        tx.commit().unwrap();
        assert_eq!(inspect(&conn).unwrap().user_version, 5);
        migrate(&mut conn).unwrap();
        assert_eq!(inspect(&conn).unwrap().user_version, 5);
    }

    /// Step 5 changes nothing a version-4 library holds, and is undone with the
    /// transaction it was taken in.
    #[test]
    fn step_5_changes_nothing_and_goes_with_its_transaction() {
        fn everything(conn: &Connection) -> Vec<String> {
            let mut rows = rows_of(
                conn,
                "SELECT id, folder_id, title, content, plain_text, word_count, char_count,
                    position, created_at, updated_at, view_state, trashed_at, kind, source_url
                 FROM documents ORDER BY id",
            );
            rows.extend(rows_of(
                conn,
                "SELECT type, name, tbl_name, sql FROM sqlite_schema ORDER BY name",
            ));
            rows
        }
        let mut conn = version_4();
        let before = everything(&conn);
        let tx = conn.transaction().unwrap();
        take_steps_above_open(&tx).unwrap();
        drop(tx);
        assert_eq!(inspect(&conn).unwrap().user_version, 4);
        let tx = conn.transaction().unwrap();
        take_steps_above_open(&tx).unwrap();
        tx.commit().unwrap();
        assert_eq!(inspect(&conn).unwrap().user_version, 5);
        assert_eq!(everything(&conn), before);
    }

    /// What step 5 is for: a build whose newest schema is 4 (0.10.1, the app and the
    /// `innerword` tool) refuses a library that holds a whiteboard, and this one opens it.
    #[test]
    fn a_build_that_knows_schema_4_refuses_a_library_of_schema_5() {
        let mut conn = version_4();
        assert!(inspect_as(&conn, 4).is_ok());
        let tx = conn.transaction().unwrap();
        take_steps_above_open(&tx).unwrap();
        tx.commit().unwrap();
        match inspect_as(&conn, 4) {
            Err(StoreError::SchemaTooNew { found, supported }) => {
                assert_eq!((found, supported), (5, 4));
            }
            other => panic!(
                "expected SchemaTooNew, got {:?}",
                other.map(|h| h.user_version)
            ),
        }
        assert_eq!(inspect(&conn).unwrap().user_version, 5);
    }

    /// The listings must be answered from the index alone: a plan that visits the table
    /// reads the text of every document.
    #[test]
    fn listings_of_live_documents_do_not_read_the_text() {
        let mut conn = version_1();
        migrate(&mut conn).unwrap();
        let cols = crate::documents::DOC_META_COLS;
        for order in ["position, id", "updated_at DESC, id DESC"] {
            let plan = plan(
                &conn,
                &format!(
                    "SELECT {cols} FROM documents INDEXED BY documents_meta
                     WHERE trashed_at IS NULL ORDER BY {order}"
                ),
            );
            assert!(
                plan.contains("COVERING INDEX documents_meta"),
                "ORDER BY {order}: {plan}"
            );
        }
        // The list of pages, live and trashed, is answered from the index too.
        let plan = plan(
            &conn,
            &format!(
                "SELECT {cols} FROM documents INDEXED BY documents_meta
                 WHERE kind = 'page' ORDER BY updated_at DESC, id DESC"
            ),
        );
        assert!(plan.contains("COVERING INDEX documents_meta"), "{plan}");
    }

    /// The list of whiteboards is answered from the covering index too, at schema 5 (where
    /// a library has whiteboards) as at 4: a plan that visits the table reads every scene.
    #[test]
    fn the_list_of_whiteboards_does_not_read_the_scenes() {
        let mut conn = version_1();
        migrate(&mut conn).unwrap();
        let sql = crate::documents::boards_query();
        let at_4 = plan(&conn, &sql);
        assert!(at_4.contains("COVERING INDEX documents_meta"), "{at_4}");
        let tx = conn.transaction().unwrap();
        take_steps_above_open(&tx).unwrap();
        tx.commit().unwrap();
        assert_eq!(inspect(&conn).unwrap().user_version, 5);
        let at_5 = plan(&conn, &sql);
        assert!(at_5.contains("COVERING INDEX documents_meta"), "{at_5}");
    }

    /// A page that is added again is looked up by its address through the index of the
    /// addresses, not by reading every row.
    #[test]
    fn a_page_is_found_by_its_address_through_the_index() {
        let mut conn = version_1();
        migrate(&mut conn).unwrap();
        let plan = plan(
            &conn,
            &format!(
                "SELECT {} FROM documents
                 WHERE source_url = 'https://claude.ai/artifact/x' AND kind = 'page'
                   AND trashed_at IS NULL
                 ORDER BY updated_at DESC, id DESC LIMIT 1",
                crate::documents::DOC_META_COLS
            ),
        );
        assert!(plan.contains("documents_by_source_url"), "{plan}");
    }
}
