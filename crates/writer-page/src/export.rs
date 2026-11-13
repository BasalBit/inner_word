//! A stored page written as a folder: its served files (it opens offline) or its
//! original files (the input as it was added). No store in here: the caller hands a
//! function that gives the bytes of a file id.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::error::PageError;
use crate::manifest::{Manifest, ManifestFile};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Which {
    Served,
    Original,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Written {
    pub dir: PathBuf,
    /// Files written.
    pub files: usize,
    /// Bytes written.
    pub bytes: u64,
    /// Paths whose bytes the library does not have (not written).
    pub missing: Vec<String>,
}

fn target(dir: &Path, path: &str) -> Result<PathBuf, PageError> {
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\0')
        || path.split('/').any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err(PageError::Unreadable { reason: format!("{path:?} is not a path of a page") });
    }
    Ok(path.split('/').fold(dir.to_path_buf(), |acc, segment| acc.join(segment)))
}

/// Writes the files of the page into `dir` (made when it is not there). A file that is
/// there already is replaced only with `overwrite`; without it nothing is written and
/// the error is `Io` with `AlreadyExists` for the first such file.
pub fn write_folder(
    manifest: &Manifest,
    which: Which,
    dir: &Path,
    overwrite: bool,
    bytes_of: &mut dyn FnMut(&str) -> io::Result<Option<Vec<u8>>>,
) -> Result<Written, PageError> {
    let files: &[ManifestFile] = match which {
        Which::Served => &manifest.files,
        Which::Original => &manifest.original,
    };
    let mut targets = Vec::with_capacity(files.len());
    for file in files {
        let path = target(dir, &file.path)?;
        if !overwrite && fs::symlink_metadata(&path).is_ok() {
            return Err(PageError::io(path, io::Error::from(io::ErrorKind::AlreadyExists)));
        }
        targets.push((file, path));
    }
    fs::create_dir_all(dir).map_err(|error| PageError::io(dir, error))?;
    let mut written = Written { dir: dir.to_path_buf(), files: 0, bytes: 0, missing: Vec::new() };
    for (file, path) in targets {
        let Some(bytes) = bytes_of(&file.file).map_err(|error| PageError::io(&path, error))? else {
            written.missing.push(file.path.clone());
            continue;
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| PageError::io(parent, error))?;
        }
        if fs::symlink_metadata(&path).map(|m| m.file_type().is_symlink() || m.is_dir()).unwrap_or(false) {
            // Never written through a link, nor over a folder.
            return Err(PageError::io(path, io::Error::from(io::ErrorKind::AlreadyExists)));
        }
        fs::write(&path, &bytes).map_err(|error| PageError::io(&path, error))?;
        written.files += 1;
        written.bytes += bytes.len() as u64;
    }
    Ok(written)
}
