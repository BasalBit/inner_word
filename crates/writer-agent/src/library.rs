//! The [`Library`] handle: where the library is, and how it is opened for one operation.

use std::path::{Path, PathBuf};
use std::time::Duration;

use writer_store::instance_lock::{self, Acquired, HolderKind, InstanceLock};
use writer_store::{Clock, OpenOptions, ReadOnlyLibrary, Store};

use crate::error::{Error, Result};
use crate::types::{LibraryStatus, WriteOptions, Written};

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
        let database = self.database_path();
        let holder = instance_lock::holder(&self.data_dir).unwrap_or(None);
        let app = holder.as_ref().filter(|copy| copy.kind == HolderKind::App);
        let mut status = LibraryStatus {
            data_dir: self.data_dir.clone(),
            database: database.clone(),
            exists: database.is_file(),
            app_is_open: app.is_some(),
            app_pid: app.and_then(|copy| copy.pid),
            documents: 0,
            folders: 0,
            trashed: 0,
            version: env!("CARGO_PKG_VERSION").to_string(),
        };
        if status.exists {
            let session = self.reader()?;
            let tree = session.store.library_tree()?;
            fn count(nodes: &[writer_store::FolderNode]) -> (u64, u64) {
                nodes.iter().fold((0, 0), |(folders, documents), node| {
                    let (f, d) = count(&node.folders);
                    (folders + 1 + f, documents + node.documents.len() as u64 + d)
                })
            }
            let (folders, documents) = count(&tree.folders);
            status.folders = folders;
            status.documents = documents + tree.documents.len() as u64;
            status.trashed = session.store.trash()?.len() as u64;
        }
        Ok(status)
    }

    /// Opens the library for reading. Fails with [`Error::LibraryNotFound`] when there is
    /// none: reading never creates one.
    pub(crate) fn reader(&self) -> Result<Session> {
        let path = self.database_path();
        if !path.is_file() {
            return Err(Error::LibraryNotFound { path });
        }
        let store = match Store::open_read_only(&path)? {
            ReadOnlyLibrary::Open(store) => *store,
            ReadOnlyLibrary::NeedsUpgrade { found, supported } => {
                // A library of an older version: upgrading writes to it, which only the
                // holder of the lock may do. The app does the same when it starts.
                match self.lock()? {
                    Ok(lock) => {
                        self.open_for_writing()?.close()?;
                        drop(lock);
                    }
                    Err(_) => {
                        return Err(Error::OutdatedLibrary {
                            path,
                            found,
                            supported,
                        });
                    }
                }
                match Store::open_read_only(&path)? {
                    ReadOnlyLibrary::Open(store) => *store,
                    ReadOnlyLibrary::NeedsUpgrade { found, supported } => {
                        return Err(Error::OutdatedLibrary {
                            path,
                            found,
                            supported,
                        });
                    }
                }
            }
        };
        Ok(Session {
            store,
            lock: None,
            dry_run: true,
            app_is_open: false,
        })
    }

    /// Opens the library for an operation that writes. A real run takes the lock of the
    /// data directory (and fails with [`Error::AppIsOpen`] when the app has it, with
    /// [`Error::Busy`] when another tool does not let go in time) and creates the library
    /// when there is none. A dry run opens it read-only, or an empty library in memory
    /// when there is none, and notes whether the app is open.
    pub(crate) fn writer(&self, options: &WriteOptions) -> Result<Session> {
        if options.dry_run {
            let mut session = if self.database_path().is_file() {
                self.reader()?
            } else {
                Session {
                    store: Store::open_in_memory_with_clock(self.clock.clone())?,
                    lock: None,
                    dry_run: true,
                    app_is_open: false,
                }
            };
            session.app_is_open = self.app_is_open();
            return Ok(session);
        }
        let lock = self.lock()??;
        let store = self.open_for_writing()?;
        Ok(Session {
            store,
            lock: Some(lock),
            dry_run: false,
            app_is_open: false,
        })
    }

    /// Takes the lock of the data directory as a tool. The outer error is a failure to
    /// try at all, the inner one says who has it.
    fn lock(&self) -> Result<Result<InstanceLock>> {
        let acquired =
            instance_lock::acquire_waiting(&self.data_dir, HolderKind::Tool, self.patience)
                .map_err(|source| Error::Io {
                    path: self.data_dir.clone(),
                    source,
                })?;
        Ok(match acquired {
            Acquired::Lock(lock) => Ok(lock),
            Acquired::Taken(copy) => Err(match copy.kind {
                HolderKind::App => Error::AppIsOpen {
                    pid: copy.pid,
                    data_dir: self.data_dir.clone(),
                },
                HolderKind::Tool => Error::Busy { pid: copy.pid },
            }),
        })
    }

    fn open_for_writing(&self) -> Result<Store> {
        Ok(Store::open_with(
            self.database_path(),
            OpenOptions {
                clock: self.clock.clone(),
                ..OpenOptions::default()
            },
        )?)
    }
}

/// The library, open for one operation.
pub(crate) struct Session {
    /// The store. Read-only (or in memory) when `dry_run` is set.
    pub(crate) store: Store,
    /// The lock of the data directory, held until the session ends. Declared after the
    /// store so that the store is closed first.
    lock: Option<InstanceLock>,
    /// Nothing may be written: this is a read, or the dry run of a write.
    pub(crate) dry_run: bool,
    /// For the dry run of a write: the app has the library open.
    pub(crate) app_is_open: bool,
}

impl Session {
    /// What a result says about the write: `changed` is whether the operation had
    /// anything to do.
    pub(crate) fn written(&self, changed: bool) -> Written {
        Written {
            dry_run: self.dry_run,
            changed: changed && !self.dry_run,
            app_is_open: self.dry_run && self.app_is_open,
        }
    }

    /// Ends a session that wrote: folds the write-ahead log into the database file and
    /// gives the lock back.
    pub(crate) fn finish(self) -> Result<()> {
        let Session { store, lock, .. } = self;
        if lock.is_some() {
            store.close()?;
        }
        drop(lock);
        Ok(())
    }
}

/// The name of whoever makes a change, for a comment's signature.
pub(crate) fn author_name(options: &WriteOptions) -> String {
    options
        .author
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(DEFAULT_AUTHOR)
        .to_string()
}

/// The same for the label of a version: "Claude", or "an agent".
pub(crate) fn author_phrase(options: &WriteOptions) -> String {
    options
        .author
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("an agent")
        .to_string()
}
