//! Where the library lives on disk.
//!
//! The real library is `~/Library/Application Support/The Writer/`. Tests,
//! automation runs and manual QA set `THE_WRITER_DATA_DIR` to a throwaway
//! directory so they never touch it.

use std::{
    ffi::OsString,
    io,
    path::{Path, PathBuf},
};

/// Environment variable that overrides the data directory.
pub const DATA_DIR_ENV: &str = "THE_WRITER_DATA_DIR";

/// Folder name under `~/Library/Application Support`.
pub const APP_FOLDER_NAME: &str = "The Writer";

/// The data directory: `$THE_WRITER_DATA_DIR` when set and non-empty, else
/// `~/Library/Application Support/The Writer`. Does not create it.
pub fn resolve_data_dir() -> PathBuf {
    resolve_from(std::env::var_os(DATA_DIR_ENV), home_dir())
}

/// [`resolve_data_dir`] plus `create_dir_all`.
pub fn ensure_data_dir() -> io::Result<PathBuf> {
    let dir = resolve_data_dir();
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// True when the data directory comes from `THE_WRITER_DATA_DIR`.
pub fn is_overridden() -> bool {
    std::env::var_os(DATA_DIR_ENV).is_some_and(|value| !value.is_empty())
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .or_else(std::env::home_dir)
}

fn resolve_from(override_dir: Option<OsString>, home: Option<PathBuf>) -> PathBuf {
    if let Some(dir) = override_dir.filter(|dir| !dir.is_empty()) {
        return PathBuf::from(dir);
    }
    let home = home.unwrap_or_else(|| PathBuf::from("/tmp"));
    default_dir_in(&home)
}

fn default_dir_in(home: &Path) -> PathBuf {
    home.join("Library")
        .join("Application Support")
        .join(APP_FOLDER_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn override_wins() {
        assert_eq!(
            resolve_from(Some("/tmp/x".into()), Some("/Users/a".into())),
            PathBuf::from("/tmp/x")
        );
    }

    #[test]
    fn empty_override_is_ignored() {
        assert_eq!(
            resolve_from(Some("".into()), Some("/Users/a".into())),
            PathBuf::from("/Users/a/Library/Application Support/The Writer")
        );
    }

    #[test]
    fn default_location() {
        assert_eq!(
            resolve_from(None, Some("/Users/a".into())),
            PathBuf::from("/Users/a/Library/Application Support/The Writer")
        );
    }
}
