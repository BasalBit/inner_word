//! The [`Library`] handle: where the library is, and how it is opened for one operation.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use innerword_store::instance_lock::{self, Acquired, HolderKind, InstanceLock};
use innerword_store::{Clock, OpenOptions, ReadOnlyLibrary, Store};

use crate::checker::PageChecker;
use crate::error::{Error, Refusal, Result};
use crate::live::{self, Change, Operation};
use crate::types::{LibraryStatus, WriteOptions, Written};

/// The name a comment is signed with, and a version is labelled with, when the request
/// names no author ([`WriteOptions::author`](crate::WriteOptions::author)).
pub const DEFAULT_AUTHOR: &str = "Agent";

/// A library of Inner Word, named by its data directory.
///
/// Creating one does nothing on disk. Every operation opens the library, does its work and
/// closes it again. Reading opens the database read-only and takes no lock; while the app
/// has the library open it is asked first to save what is not saved yet, so the text read
/// is the text on screen. Writing takes the lock of the data directory for the duration
/// of the operation; while the app has it, the operation is handed to the app, which
/// carries it out and answers ([`crate::live`]). See the [crate documentation](crate).
#[derive(Clone)]
pub struct Library {
    pub(crate) data_dir: PathBuf,
    pub(crate) clock: Clock,
    pub(crate) patience: Duration,
    pub(crate) app_patience: Duration,
    pub(crate) save_patience: Duration,
    pub(crate) base_dir: Option<PathBuf>,
    /// Who checks a page that is added, and prints one that is exported as a PDF. None
    /// unless one was installed ([`Library::with_page_checker`]).
    pub(crate) page_checker: Option<Arc<dyn PageChecker>>,
    /// Who fetches what a page names on the web, once for every page this library adds.
    pub(crate) fetcher: Arc<dyn innerword_page::Fetcher>,
}

impl std::fmt::Debug for Library {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Library")
            .field("data_dir", &self.data_dir)
            .field("clock", &self.clock)
            .field("patience", &self.patience)
            .field("app_patience", &self.app_patience)
            .field("save_patience", &self.save_patience)
            .field("base_dir", &self.base_dir)
            .field("page_checker", &self.page_checker.is_some())
            .finish_non_exhaustive()
    }
}

/// The fetcher of a library that was given none: `/usr/bin/curl`, public addresses only,
/// each address once ([`innerword_page::CachingFetcher`]). In a debug build
/// `INNER_WORD_FETCH_LOCAL=1` lets it ask 127.0.0.1 and `localhost` too, for the tests of
/// the tool, whose server is there; a released tool does not read the variable.
fn default_fetcher() -> Arc<dyn innerword_page::Fetcher> {
    let mut curl = innerword_page::CurlFetcher::new();
    if cfg!(debug_assertions) && std::env::var(FETCH_LOCAL_ENV).is_ok_and(|value| value == "1") {
        curl = curl.allowing_local();
    }
    Arc::new(innerword_page::CachingFetcher::new(Arc::new(curl)))
}

/// The variable a debug build reads to let the default fetcher ask this Mac
/// (`INNER_WORD_FETCH_LOCAL=1`): for tests that run a server on 127.0.0.1.
pub const FETCH_LOCAL_ENV: &str = "INNER_WORD_FETCH_LOCAL";

impl Library {
    /// The library in the data directory `data_dir` (the directory that holds
    /// `library.db`).
    pub fn at(data_dir: impl Into<PathBuf>) -> Library {
        Library {
            data_dir: data_dir.into(),
            clock: Clock::system(),
            patience: Duration::from_secs(10),
            app_patience: live::ANSWER_PATIENCE,
            save_patience: live::SAVE_PATIENCE,
            base_dir: None,
            page_checker: None,
            fetcher: default_fetcher(),
        }
    }

    /// The user's library: the data directory the app uses, from the store's own helper
    /// (`innerword_store::data_dir`). That is `$INNER_WORD_DATA_DIR` when it is set, else
    /// `~/Library/Application Support/Inner Word`.
    ///
    /// Tests never call this without the variable set: it is the user's real library.
    pub fn default_location() -> Result<Library> {
        Ok(Library::at(innerword_store::data_dir()?))
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
    /// [`Clock::manual`]). A change the app carries out is stamped by the app's clock.
    pub fn with_clock(mut self, clock: Clock) -> Library {
        self.clock = clock;
        self
    }

    /// The same library with another limit for how long a write waits for another tool
    /// that is writing to it right now (10 seconds unless changed).
    pub fn with_patience(mut self, patience: Duration) -> Library {
        self.patience = patience;
        self
    }

    /// The same library with another limit for how long a change waits for the app that
    /// has the library open to carry it out ([`live::ANSWER_PATIENCE`], 10 seconds, unless
    /// changed). Tests of "the app does not answer" make it short.
    pub fn with_app_patience(mut self, patience: Duration) -> Library {
        self.app_patience = patience;
        self
    }

    /// The same library with another limit for how long a read waits for the app to save
    /// what is not saved yet ([`live::SAVE_PATIENCE`], one second, unless changed). When
    /// the app does not answer in time the read goes ahead on what is saved.
    pub fn with_save_patience(mut self, patience: Duration) -> Library {
        self.save_patience = patience;
        self
    }

    /// The same library with a directory that the relative path of an image file is
    /// taken from, for every request that names none of its own (`base_dir` of the
    /// request): a path in the Markdown an operation is given, the file of
    /// [`Library::add_image`]. A command line tool gives the directory it works in, or
    /// the directory of the file its text came from. Without one, a relative path is
    /// not read: an image in Markdown stays a link (a warning says so), and
    /// `add_image` asks for an absolute path.
    pub fn with_base_dir(mut self, base_dir: impl Into<PathBuf>) -> Library {
        self.base_dir = Some(base_dir.into());
        self
    }

    /// The same library with a checker for the pages it adds, and the printer of the
    /// pages it exports as a PDF. None by default: add_page and a PDF export fail with
    /// `failed`, "pages cannot be checked here". The `innerword` binary installs
    /// [`ProcessChecker::current_exe`](crate::ProcessChecker::current_exe); a test
    /// [`StaticChecker`](crate::StaticChecker).
    pub fn with_page_checker(mut self, checker: Arc<dyn PageChecker>) -> Library {
        self.page_checker = Some(checker);
        self
    }

    /// The same library with another fetcher for what the pages it adds name on the web.
    /// Default: `CachingFetcher` over `CurlFetcher::new()`. Tests give a
    /// `innerword_page::MapFetcher`, or a `CurlFetcher` allowed to ask their server on
    /// 127.0.0.1.
    pub fn with_fetcher(mut self, fetcher: Arc<dyn innerword_page::Fetcher>) -> Library {
        self.fetcher = fetcher;
        self
    }

    /// The data directory.
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// The database file in it.
    pub fn database_path(&self) -> PathBuf {
        self.data_dir.join(innerword_store::DATABASE_FILE_NAME)
    }

    /// Whether the app has this library open right now. A change is then carried out by
    /// the app ([`LibraryStatus::app_takes_changes`] says whether it takes one).
    pub fn app_is_open(&self) -> bool {
        innerword_store::instance_lock::app_is_running(&self.data_dir).unwrap_or(false)
    }

    /// Where the library is, whether it exists, whether the app has it open (and takes
    /// changes), and how much is in it. Never fails for a library that does not exist:
    /// that is one of the answers.
    pub fn status(&self) -> Result<LibraryStatus> {
        let database = self.database_path();
        let holder = instance_lock::holder(&self.data_dir).unwrap_or(None);
        let app = holder.as_ref().filter(|copy| copy.kind == HolderKind::App);
        let asked = self.ask_app_to_save();
        let mut status = LibraryStatus {
            data_dir: self.data_dir.clone(),
            database: database.clone(),
            exists: database.is_file(),
            app_is_open: app.is_some(),
            app_pid: app.and_then(|copy| copy.pid),
            app_takes_changes: app.is_some() && asked.takes_changes,
            app_refuses: asked
                .why_not
                .filter(|_| app.is_some() && !asked.takes_changes),
            documents: 0,
            folders: 0,
            trashed: 0,
            version: env!("CARGO_PKG_VERSION").to_string(),
        };
        if status.exists {
            let session = self.open_reader()?;
            let tree = session.store.library_tree()?;
            fn count(nodes: &[innerword_store::FolderNode]) -> (u64, u64) {
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

    /// Opens the library for reading, after asking the app that has it open (if one has)
    /// to save what is not saved yet. Fails with [`Error::LibraryNotFound`] when there is
    /// none: reading never creates one.
    pub(crate) fn reader(&self) -> Result<Reader> {
        self.ask_app_to_save();
        self.open_reader()
    }

    /// Asks the app that has the library open to save what is not saved yet, and waits
    /// for it up to [`Library::with_save_patience`]. Says what is known about the app
    /// afterwards; without an app, or without an answer, nothing was asked or learned.
    pub(crate) fn ask_app_to_save(&self) -> AppAsked {
        let Ok(Some(channel)) = instance_lock::app_channel(&self.data_dir) else {
            // No app, or one that announced no channel: it takes no changes either. Which
            // app that is, is known once it has held the lock for longer than an app
            // takes to open its channel: one that never will. A younger one may be about
            // to (a write waits for it, [`Library::wait_for_channel`]; a read does not).
            let is_open = self.app_is_open();
            let old = is_open
                && instance_lock::held_since(&self.data_dir)
                    .ok()
                    .flatten()
                    .is_some_and(|since| {
                        since.elapsed().unwrap_or(live::STARTING_GRACE) >= live::STARTING_GRACE
                    });
            return AppAsked {
                is_open,
                takes_changes: false,
                why_not: old.then_some(Refusal::OldVersion),
            };
        };
        let state = live::client::save(&channel.path, self.save_patience);
        // An app that listens and did not answer in time is taken at its word.
        let takes_changes = state.is_none_or(|state| state.takes_changes);
        AppAsked {
            is_open: true,
            takes_changes,
            // An app that listens and says that it takes no changes: its owner's switch.
            why_not: (!takes_changes).then_some(Refusal::SwitchedOff),
        }
    }

    /// Opens the library for reading as it is on disk.
    fn open_reader(&self) -> Result<Reader> {
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
        Ok(Reader { store })
    }

    /// Carries out an operation that writes: the one way to a library that can be
    /// written to.
    ///
    /// - A **dry run** only reads: the app is asked to save, the library is opened
    ///   read-only (or, when there is none, an empty one in memory) and `run` works out
    ///   what it would do.
    /// - Otherwise the lock of the data directory is taken and `run` writes, creating the
    ///   library when there is none. When another tool does not let go in time this fails
    ///   with [`Error::Busy`].
    /// - While **the app** has the lock, `operation` is sent to it and its answer is the
    ///   result ([`crate::live`]); when the app takes no change this fails with
    ///   [`Error::AppIsOpen`], which says why.
    pub(crate) fn write<T: DeserializeOwned>(
        &self,
        options: &WriteOptions,
        operation: impl Fn() -> Operation,
        run: impl FnOnce(&mut Session<'_>) -> Result<T>,
    ) -> Result<T> {
        if options.dry_run {
            let asked = self.ask_app_to_save();
            let mut store = if self.database_path().is_file() {
                self.open_reader()?.store
            } else {
                Store::open_in_memory_with_clock(self.clock.clone())?
            };
            let mut session = Session {
                store: &mut store,
                dry_run: true,
                app_is_open: asked.is_open,
                app_takes_changes: asked.is_open && asked.takes_changes,
                app_refuses: asked.why_not,
                clock: self.clock.clone(),
                changes: Vec::new(),
            };
            return run(&mut session);
        }
        let mut run = Some(run);
        let refuse = |pid: Option<u32>, why: Refusal| Error::AppIsOpen {
            pid,
            data_dir: self.data_dir.clone(),
            why,
        };
        let (mut said_quitting, mut found_gone) = (false, false);
        // Every way round the loop but the last waits for something that then is so (the
        // app is gone, or says where it listens), so a few rounds are all there can be.
        for _ in 0..8 {
            // An app that listens is handed the operation at once; its lock is not tried.
            let channel = match instance_lock::app_channel(&self.data_dir).unwrap_or(None) {
                Some(channel) => channel,
                None => match self.lock()? {
                    Ok(lock) => {
                        let mut store = self.open_for_writing()?;
                        let mut session = Session::writing(&mut store, self.clock.clone());
                        let run = run.take().expect("an operation runs once");
                        let result = run(&mut session)?;
                        // Folds the write-ahead log into the database file, then gives
                        // the lock back.
                        store.close()?;
                        drop(lock);
                        return Ok(result);
                    }
                    Err(Taken::Tool(pid)) => return Err(Error::Busy { pid }),
                    Err(Taken::App(pid)) => {
                        // The app has the lock and names no channel. One that has only
                        // just started will in a moment; an older one never does.
                        if self.wait_for_channel() {
                            continue;
                        }
                        return Err(refuse(pid, Refusal::OldVersion));
                    }
                },
            };
            match live::client::change(&channel.path, &operation(), self.app_patience) {
                live::client::Sent::Done(mut value) => {
                    // The app made the change, so the result says that the app is open
                    // and takes changes. An app of 0.6.0 left both at false in the
                    // result of a real run; the tool knows better than that answer.
                    if let Some(result) = value.as_object_mut() {
                        result.insert("app_is_open".into(), true.into());
                        result.insert("app_takes_changes".into(), true.into());
                    }
                    return serde_json::from_value(value).map_err(|error| Error::InApp {
                        kind: crate::error::ErrorKind::Failed,
                        message: format!(
                            "Inner Word made the change, but its answer cannot be read \
                             ({error}). Read the document again to see what it is now."
                        ),
                    });
                }
                live::client::Sent::Failed(error) => return Err(error),
                live::client::Sent::Refused(Refusal::Quitting) if !said_quitting => {
                    // The app goes away: its lock is free in a moment, and the change is
                    // written directly then.
                    said_quitting = true;
                    live::client::wait_for_quit(
                        &self.data_dir,
                        live::QUIT_PATIENCE.min(self.app_patience),
                    );
                }
                live::client::Sent::Refused(why) => return Err(refuse(channel.pid, why)),
                live::client::Sent::Gone => {
                    // Nobody listens where the lock file says: the app is on its way out
                    // (the lock is free in a moment), or another one is on its way in
                    // (the file names its channel in a moment).
                    if found_gone {
                        return Err(refuse(channel.pid, Refusal::NotListening));
                    }
                    found_gone = true;
                    live::client::wait_for_another(
                        &self.data_dir,
                        &channel.path,
                        live::GONE_PATIENCE,
                    );
                }
            }
        }
        let pid = instance_lock::holder(&self.data_dir)
            .unwrap_or(None)
            .and_then(|copy| copy.pid);
        Err(refuse(pid, Refusal::NotListening))
    }

    /// Waits for the app that holds the lock without naming a channel to name one, for as
    /// long as it may still be starting ([`live::STARTING_GRACE`] from the moment it took
    /// the lock, and never longer than [`Library::with_app_patience`]). True when there
    /// is something new to look at: it names a channel now, or it is gone.
    fn wait_for_channel(&self) -> bool {
        let started = Instant::now();
        loop {
            match instance_lock::app_channel(&self.data_dir) {
                Ok(Some(_)) => return true,
                Ok(None) => {}
                Err(_) => return false,
            }
            let since = match instance_lock::held_since(&self.data_dir) {
                // Nobody holds the lock any more.
                Ok(None) => return true,
                Ok(Some(since)) => since,
                Err(_) => return false,
            };
            if !instance_lock::app_is_running(&self.data_dir).unwrap_or(true) {
                // A tool has it now: wait for that one as for any tool.
                return true;
            }
            // A lock file from the future is not one of an app that has just started.
            let held_for = since.elapsed().unwrap_or(live::STARTING_GRACE);
            if held_for >= live::STARTING_GRACE || started.elapsed() >= self.app_patience {
                return false;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Takes the lock of the data directory as a tool. The outer error is a failure to
    /// try at all, the inner one says who has it.
    fn lock(&self) -> Result<std::result::Result<InstanceLock, Taken>> {
        let acquired =
            instance_lock::acquire_waiting(&self.data_dir, HolderKind::Tool, self.patience)
                .map_err(|source| Error::Io {
                    path: self.data_dir.clone(),
                    source,
                })?;
        Ok(match acquired {
            Acquired::Lock(lock) => Ok(lock),
            Acquired::Taken(copy) => Err(match copy.kind {
                HolderKind::App => Taken::App(copy.pid),
                HolderKind::Tool => Taken::Tool(copy.pid),
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

/// Who has the lock of the data directory, with the process id the lock file names.
enum Taken {
    App(Option<u32>),
    Tool(Option<u32>),
}

/// What is known about the app after it was asked to save.
#[derive(Clone, Debug)]
pub(crate) struct AppAsked {
    /// The app has the library open.
    pub(crate) is_open: bool,
    /// It takes changes from agents (as far as known).
    pub(crate) takes_changes: bool,
    /// Why it takes none, where that can be known: its owner switched them off (an app
    /// that listens says so), or it is a version that never takes one (it has held the
    /// lock for a while and names no channel). `None` while it takes changes, and for an
    /// app that has only just started and names no channel yet.
    pub(crate) why_not: Option<Refusal>,
}

/// The library, open for reading.
pub(crate) struct Reader {
    /// The store, read-only.
    pub(crate) store: Store,
}

/// The library, open for one operation that writes (or for its dry run).
pub(crate) struct Session<'a> {
    /// The store: the tool's own connection, the app's when the app carries the
    /// operation out, read-only (or in memory) in a dry run by the tool.
    pub(crate) store: &'a mut Store,
    /// Nothing may be written: this is the dry run of a write.
    pub(crate) dry_run: bool,
    /// The app has the library open: it is the app that carries this operation out, or,
    /// for a dry run by the tool, that would carry the real run out.
    pub(crate) app_is_open: bool,
    /// While the app is open: the app takes changes from agents.
    pub(crate) app_takes_changes: bool,
    /// While the app is open and takes none: why, where that is known.
    pub(crate) app_refuses: Option<Refusal>,
    /// The clock for what the operation stamps itself (a comment, a planned item).
    pub(crate) clock: Clock,
    /// What the operation changed so far, for the app ([`crate::live::Change`]).
    pub(crate) changes: Vec<Change>,
}

impl<'a> Session<'a> {
    /// A session in which the tool itself writes to `store`: no app has the library
    /// open.
    pub(crate) fn writing(store: &'a mut Store, clock: Clock) -> Session<'a> {
        Session {
            store,
            dry_run: false,
            app_is_open: false,
            app_takes_changes: false,
            app_refuses: None,
            clock,
            changes: Vec::new(),
        }
    }

    /// What a result says about the write: `changed` is whether the operation had
    /// anything to do. Who made the change (or would make it) is said for a real run as
    /// for a dry run: a result that says "the app is not open" after the app carried the
    /// change out would be read as a fact.
    pub(crate) fn written(&self, changed: bool) -> Written {
        Written {
            dry_run: self.dry_run,
            changed: changed && !self.dry_run,
            app_is_open: self.app_is_open,
            app_takes_changes: self.app_is_open && self.app_takes_changes,
            app_refuses: self
                .app_refuses
                .clone()
                .filter(|_| self.app_is_open && !self.app_takes_changes),
            would_change: changed,
        }
    }

    /// Notes something the operation changed, once.
    pub(crate) fn note(&mut self, change: Change) {
        if !self.changes.contains(&change) {
            self.changes.push(change);
        }
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
