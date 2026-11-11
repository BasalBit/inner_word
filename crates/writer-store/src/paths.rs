//! Where the library lives on disk.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::error::{Result, StoreError};

/// Environment variable that overrides the data directory (used by tests and manual QA).
pub const DATA_DIR_ENV: &str = "THE_WRITER_DATA_DIR";

/// File name of the library database inside the data directory.
pub const DATABASE_FILE_NAME: &str = "library.db";

/// Name of the directory, next to the database, that holds the daily backups.
pub const BACKUPS_DIR_NAME: &str = "backups";

/// The data directory: `$THE_WRITER_DATA_DIR` when set and non-empty, otherwise
/// `~/Library/Application Support/The Writer`. The directory is not created.
pub fn data_dir() -> Result<PathBuf> {
    resolve_data_dir(std::env::var_os(DATA_DIR_ENV), std::env::var_os("HOME"))
}

/// The library database path: `data_dir()/library.db`.
pub fn database_path() -> Result<PathBuf> {
    Ok(data_dir()?.join(DATABASE_FILE_NAME))
}

/// The backups directory used for a database at `database_path`: a `backups` directory next
/// to it.
pub fn backups_dir(database_path: &Path) -> PathBuf {
    database_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default()
        .join(BACKUPS_DIR_NAME)
}

pub(crate) fn resolve_data_dir(env: Option<OsString>, home: Option<OsString>) -> Result<PathBuf> {
    if let Some(dir) = env.filter(|d| !d.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    match home.filter(|h| !h.is_empty()) {
        Some(home) => Ok(PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("The Writer")),
        None => Err(StoreError::NoDataDirectory),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_override_wins() {
        let dir = resolve_data_dir(Some("/tmp/x".into()), Some("/Users/me".into())).unwrap();
        assert_eq!(dir, PathBuf::from("/tmp/x"));
    }

    #[test]
    fn falls_back_to_application_support() {
        let dir = resolve_data_dir(Some("".into()), Some("/Users/me".into())).unwrap();
        assert_eq!(
            dir,
            PathBuf::from("/Users/me/Library/Application Support/The Writer")
        );
    }

    #[test]
    fn no_home_is_an_error() {
        assert!(matches!(
            resolve_data_dir(None, None),
            Err(StoreError::NoDataDirectory)
        ));
    }

    #[test]
    fn backups_dir_is_next_to_database() {
        assert_eq!(
            backups_dir(Path::new("/a/b/library.db")),
            PathBuf::from("/a/b/backups")
        );
    }
}
