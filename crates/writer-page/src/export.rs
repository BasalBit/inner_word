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
///
/// Every file is read before anything is written, so an export is whole or nothing: a
/// file that cannot be read (the library's file of a newer version, busy past its
/// timeout) writes nothing and makes no folder, nor does a page whose entry the library
/// does not have (nothing written would open). A file the library does not have is
/// left out and named in `missing`. A write that fails takes back the files and folders
/// it made (a file it replaced stays replaced).
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

    // Everything is read first.
    let mut read: Vec<(&ManifestFile, PathBuf, Vec<u8>)> = Vec::with_capacity(targets.len());
    let mut missing = Vec::new();
    for (file, path) in targets {
        match bytes_of(&file.file) {
            Ok(Some(bytes)) => read.push((file, path, bytes)),
            Ok(None) => missing.push(file.path.clone()),
            Err(error) => {
                return Err(PageError::Unreadable {
                    reason: format!(
                        "its file {} could not be read from the library ({error}), so nothing was written",
                        file.path
                    ),
                });
            }
        }
    }
    let entry_listed = files.iter().any(|file| file.path == manifest.entry);
    if entry_listed && missing.contains(&manifest.entry) {
        return Err(PageError::Unreadable {
            reason: format!(
                "the library does not have its entry {}, so nothing was written (it would not open)",
                manifest.entry
            ),
        });
    }
    if !files.is_empty() && read.is_empty() {
        return Err(PageError::Unreadable {
            reason: "the library has none of its files, so nothing was written".to_string(),
        });
    }

    let mut made = Made::default();
    let result = write_all(dir, &read, &mut made);
    if result.is_err() {
        made.take_back();
    }
    result?;
    let mut written = Written { dir: dir.to_path_buf(), files: 0, bytes: 0, missing };
    for (_, _, bytes) in &read {
        written.files += 1;
        written.bytes += bytes.len() as u64;
    }
    Ok(written)
}

/// What a write made, so that a failed one can take it back.
#[derive(Default)]
struct Made {
    dirs: Vec<PathBuf>,
    files: Vec<PathBuf>,
}

impl Made {
    fn dir_all(&mut self, dir: &Path) -> io::Result<()> {
        let mut missing = Vec::new();
        let mut at = Some(dir);
        while let Some(path) = at {
            if fs::symlink_metadata(path).is_ok() {
                break;
            }
            missing.push(path.to_path_buf());
            at = path.parent();
        }
        for path in missing.into_iter().rev() {
            match fs::create_dir(&path) {
                Ok(()) => self.dirs.push(path),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    fn take_back(&self) {
        for file in self.files.iter().rev() {
            fs::remove_file(file).ok();
        }
        for dir in self.dirs.iter().rev() {
            fs::remove_dir(dir).ok();
        }
    }
}

fn write_all(dir: &Path, read: &[(&ManifestFile, PathBuf, Vec<u8>)], made: &mut Made) -> Result<(), PageError> {
    made.dir_all(dir).map_err(|error| PageError::io(dir, error))?;
    for (_, path, bytes) in read {
        if let Some(parent) = path.parent() {
            made.dir_all(parent).map_err(|error| PageError::io(parent, error))?;
        }
        let existing = fs::symlink_metadata(path);
        if existing.as_ref().map(|m| m.file_type().is_symlink() || m.is_dir()).unwrap_or(false) {
            // Never written through a link, nor over a folder.
            return Err(PageError::io(path, io::Error::from(io::ErrorKind::AlreadyExists)));
        }
        fs::write(path, bytes).map_err(|error| PageError::io(path, error))?;
        if existing.is_err() {
            made.files.push(path.clone());
        }
    }
    Ok(())
}
