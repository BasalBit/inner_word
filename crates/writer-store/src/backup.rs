//! Daily backups of the database file.
//!
//! Backups are named `library-<unix ms>.db` and written with `VACUUM INTO`, which produces a
//! consistent, compact copy (including anything still in the WAL) without blocking readers.

use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::error::Result;

/// How many backups [`crate::Store::open`] keeps by default.
pub const DEFAULT_KEEP_BACKUPS: usize = 7;

const DAY_MS: i64 = 24 * 60 * 60 * 1000;
const PREFIX: &str = "library-";
const SUFFIX: &str = ".db";

/// Lists the backups in `dir`, newest first, as `(created unix ms, path)`. A missing
/// directory gives an empty list. Files that do not follow the backup naming are ignored.
pub fn list_backups(dir: &Path) -> Result<Vec<(i64, PathBuf)>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let mut backups = Vec::new();
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(stamp) = name
            .strip_prefix(PREFIX)
            .and_then(|rest| rest.strip_suffix(SUFFIX))
            .and_then(|ms| ms.parse::<i64>().ok())
        else {
            continue;
        };
        backups.push((stamp, entry.path()));
    }
    backups.sort_by(|a, b| b.0.cmp(&a.0));
    Ok(backups)
}

pub(crate) fn write_backup(conn: &Connection, dest: &Path) -> Result<()> {
    let file_name = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "backup.db".to_owned());
    let tmp = dest.with_file_name(format!(".{file_name}.tmp"));
    let _ = std::fs::remove_file(&tmp);
    let tmp_str = tmp.to_string_lossy().into_owned();
    let result = conn
        .execute("VACUUM INTO ?1", [&tmp_str])
        .map_err(Into::into)
        .and_then(|_| std::fs::rename(&tmp, dest).map_err(Into::into));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Writes a backup when the newest one is at least a day old, then deletes all but the
/// newest `keep`. Returns the new backup's path, or `None` when none was due.
pub(crate) fn daily_backup(
    conn: &Connection,
    dir: &Path,
    now_ms: i64,
    keep: usize,
) -> Result<Option<PathBuf>> {
    let existing = list_backups(dir)?;
    if let Some((newest, _)) = existing.first() {
        // A backup stamped in the future (clock moved back) also counts as recent.
        if now_ms - newest < DAY_MS {
            return Ok(None);
        }
    }
    std::fs::create_dir_all(dir)?;
    let dest = dir.join(format!("{PREFIX}{now_ms}{SUFFIX}"));
    write_backup(conn, &dest)?;
    for (_, old) in list_backups(dir)?.into_iter().skip(keep.max(1)) {
        let _ = std::fs::remove_file(old);
    }
    Ok(Some(dest))
}
