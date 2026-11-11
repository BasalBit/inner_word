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

/// The newest schema version this build creates and understands.
pub const SCHEMA_VERSION: i64 = MIGRATIONS.len() as i64;

const MIGRATIONS: &[&str] = &[V1];

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

/// Applies the per-connection pragmas.
///
/// WAL lets readers run during writes and makes commits atomic on disk; `synchronous=FULL`
/// fsyncs the WAL on every commit, so a committed save survives an app crash and power loss.
pub(crate) fn configure(conn: &Connection, on_disk: bool) -> Result<()> {
    conn.busy_timeout(Duration::from_secs(5))?;
    conn.pragma_update(None, "foreign_keys", true)?;
    if on_disk {
        let mode: String = conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))?;
        if !mode.eq_ignore_ascii_case("wal") {
            return Err(StoreError::JournalMode(mode));
        }
        conn.pragma_update(None, "synchronous", "FULL")?;
    }
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    Ok(())
}

/// What [`inspect`] found in the database header.
pub(crate) struct Header {
    pub(crate) user_version: i64,
}

/// Validates that the file is ours (or empty) and not from a newer app version.
pub(crate) fn inspect(conn: &Connection) -> Result<Header> {
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
    if user_version > SCHEMA_VERSION {
        return Err(StoreError::SchemaTooNew {
            found: user_version,
            supported: SCHEMA_VERSION,
        });
    }
    if user_version < 0 {
        return Err(StoreError::NotALibrary);
    }
    Ok(Header { user_version })
}

/// Runs every migration step the database has not seen yet.
pub(crate) fn migrate(conn: &mut Connection) -> Result<()> {
    let header = inspect(conn)?;
    for (index, step) in MIGRATIONS
        .iter()
        .enumerate()
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
