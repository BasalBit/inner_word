//! The [`Library`] handle: where the library is, and how it is opened for one operation.

use std::path::{Path, PathBuf};
use std::time::Duration;

use writer_store::Clock;

use crate::error::{Error, Result};
use crate::types::LibraryStatus;

/// The name a comment is signed with, and a version is labelled with, when the request
/// names no author ([`WriteOptions::author`](crate::WriteOptions::author)).
pub const DEFAULT_AUTHOR: &str = "Agent";

/// A library of The Writer, named by its data directory.
///
/// Creating one does nothing on disk. Every operation opens the library, does its work and
/// closes it again: reading opens the database read-only and takes no lock; writing takes
/// the lock of the data directory for the duration of the operation and is refused with
/// [`Error::AppIsOpen`] while the app has it. See the [crate documentation](crate).
#[derive(Clone, Debug)]
pub struct Library {
    pub(crate) data_dir: PathBuf,
    pub(crate) clock: Clock,
    pub(crate) patience: Duration,
}

impl Library {
    /// The library in the data directory `data_dir` (the directory that holds
    /// `library.db`).
    pub fn at(data_dir: impl Into<PathBuf>) -> Library {
        Library {
            data_dir: data_dir.into(),
            clock: Clock::system(),
            patience: Duration::from_secs(10),
        }
    }

    /// The user's library: the data directory the app uses, from the store's own helper
    /// (`writer_store::data_dir`). That is `$THE_WRITER_DATA_DIR` when it is set, else
    /// `~/Library/Application Support/The Writer`.
    ///
    /// Tests never call this without the variable set: it is the user's real library.
    pub fn default_location() -> Result<Library> {
        Ok(Library::at(writer_store::data_dir()?))
    }

    /// The library in `data_dir` when one is given, else [`Library::default_location`]:
    /// what a front end with a "data directory" option calls.
    pub fn locate(data_dir: Option<&Path>) -> Result<Library> {
        match data_dir {
            Some(dir) => Ok(Library::at(dir)),
            None => Library::default_location(),
        }
    }

    /// The same library with another clock for every time stamp it writes (tests use
    /// [`Clock::manual`]).
    pub fn with_clock(mut self, clock: Clock) -> Library {
        self.clock = clock;
        self
    }

    /// The same library with another limit for how long a write waits for another tool
    /// that is writing to it right now (10 seconds unless changed). The app is never
    /// waited for: it holds the library for as long as it runs.
    pub fn with_patience(mut self, patience: Duration) -> Library {
        self.patience = patience;
        self
    }

    /// The data directory.
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// The database file in it.
    pub fn database_path(&self) -> PathBuf {
        self.data_dir.join(writer_store::DATABASE_FILE_NAME)
    }

    /// Whether the app has this library open right now, so that writes would be refused.
    pub fn app_is_open(&self) -> bool {
        writer_store::instance_lock::app_is_running(&self.data_dir).unwrap_or(false)
    }

    /// Where the library is, whether it exists, whether the app has it open, and how much
    /// is in it. Never fails for a library that does not exist: that is one of the
    /// answers.
    pub fn status(&self) -> Result<LibraryStatus> {
        unfinished("status")
    }
}

/// The answer of an operation that this build does not have yet.
pub(crate) fn unfinished<T>(operation: &str) -> Result<T> {
    Err(Error::invalid(format!(
        "\"{operation}\" is not available in this build yet"
    )))
}
