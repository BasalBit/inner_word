//! Autosave off the main thread.
//!
//! A save of a book costs tens of milliseconds: serialising the document,
//! rebuilding its full-text index row and a commit that waits for the disk
//! (measured in a release build on a 10,000-block document: about 65 ms for
//! the library write alone, see `savecost` in [`crate::automation`]). Done
//! on the main thread that is several dropped frames every time the writer
//! pauses, so the autosave hands the work to a background thread:
//!
//! 1. On the main thread the document is copied (well under a millisecond
//!    for a book) together with the counts and the view state: a
//!    [`SaveJob`].
//! 2. A background task serialises the copy and writes it through a second
//!    connection to the library file ([`Saver`] owns it; SQLite in WAL mode
//!    lets the main connection keep reading meanwhile).
//! 3. Back on the main thread the session takes note of the result and the
//!    library re-reads its tree.
//!
//! # The rules that keep text safe
//!
//! - **One at a time, in order.** The session starts a background save
//!   only when none is in flight, so two of them can never race.
//! - **A synchronous save settles the background first.**
//!   [`Saver::settle`] waits for the save that is running and cancels one
//!   that has not started; only then does the caller write the current text
//!   through the library. So the flush on switching documents, closing and
//!   quitting is still complete when it returns, and an older copy can
//!   never land after a newer one.
//! - **A title the user chose is read at the last moment**, on the
//!   background thread right before the write, so a rename made while the
//!   save was queued is kept.
//! - **Nothing is lost on failure.** The text stays in the editor, the
//!   session stays dirty and the save is retried, exactly as for a
//!   synchronous save.
//!
//! A library that is not a file (the in-memory libraries of the tests) has
//! no second connection: [`Saver::is_background`] is false and every save
//! is synchronous.

use std::{
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use innerword_core::{Document, Stats};
use innerword_library::{
    DocumentContent, DocumentId, DocumentMeta, Library,
    store::{OpenOptions, Store},
};

use crate::document_io::derived_title;

/// Everything a background save needs, taken on the main thread.
pub(crate) struct SaveJob {
    /// The position of this save in the order they were started.
    pub seq: u64,
    /// The document to save.
    pub id: DocumentId,
    /// A copy of the document as it was when the save started.
    pub document: Document,
    /// The counts, when the editor had them ready; counted on the
    /// background thread otherwise.
    pub stats: Option<Stats>,
    /// Whether the user named the document. Shared with the session and
    /// read right before the write, so a rename that happens while the save
    /// is queued is respected.
    pub custom_title: Arc<AtomicBool>,
    /// The view state (caret, scroll) to store with it, when it changed.
    pub view_state: Option<String>,
    /// How old the newest version snapshot must be before another is taken.
    pub snapshot_interval: Duration,
}

/// What a background save did.
pub(crate) enum SaveOutcome {
    /// The document is in the library.
    Saved(SavedDocument),
    /// A synchronous save took over before this one started; nothing was
    /// written.
    Superseded,
    /// The write failed (the message). Nothing changed in the library.
    Failed(String),
}

/// The result of a successful background save.
pub(crate) struct SavedDocument {
    /// The stored metadata after the save.
    pub meta: DocumentMeta,
    /// The view state that was stored with it, if one was given and the
    /// write succeeded.
    pub view_state: Option<String>,
    /// How long the background thread worked.
    pub duration: Duration,
    /// The size of the stored content.
    pub bytes: usize,
}

struct Background {
    store: Store,
    /// Saves with a lower sequence number must not write any more.
    cancel_before: u64,
}

/// The background half of the autosave. See the [module docs](self).
pub(crate) struct Saver {
    background: Option<Arc<Mutex<Background>>>,
    next_seq: u64,
}

impl Saver {
    /// A saver for `library`. Opens the second connection when the library
    /// is a file; a failure to open it only means that saves stay
    /// synchronous.
    pub fn new(library: &Library) -> Self {
        let background = library.path().and_then(|path| {
            let options = OpenOptions {
                clock: library.clock().clone(),
                // The library's own connection made today's backup.
                daily_backup: false,
                ..OpenOptions::default()
            };
            match Store::open_with(path, options) {
                Ok(store) => Some(Arc::new(Mutex::new(Background {
                    store,
                    cancel_before: 0,
                }))),
                Err(error) => {
                    eprintln!(
                        "Inner Word: saving in the background is off, the library could not \
                         be opened a second time: {error}"
                    );
                    None
                }
            }
        });
        Self {
            background,
            next_seq: 0,
        }
    }

    /// Whether saves can run on a background thread.
    pub fn is_background(&self) -> bool {
        self.background.is_some()
    }

    /// The sequence number for the next save.
    pub fn next_seq(&mut self) -> u64 {
        let seq = self.next_seq;
        self.next_seq += 1;
        seq
    }

    /// The work of `job` as a closure for a background thread. `None` when
    /// this saver is synchronous.
    pub fn work(&self, job: SaveJob) -> Option<impl FnOnce() -> SaveOutcome + Send + 'static> {
        let background = self.background.clone()?;
        Some(move || {
            // Serialising needs no lock: do it before waiting for one, so a
            // settle on the main thread waits only for the write itself.
            let started = Instant::now();
            let json = job.document.to_json();
            let plain_text = job.document.plain_text();
            let stats = job.stats.unwrap_or_else(|| job.document.stats());
            let derived = derived_title(&job.document);

            let mut background = lock(&background);
            if job.seq < background.cancel_before {
                return SaveOutcome::Superseded;
            }
            match write(
                &mut background.store,
                &job,
                &json,
                &plain_text,
                stats,
                &derived,
            ) {
                Ok((meta, view_state)) => SaveOutcome::Saved(SavedDocument {
                    meta,
                    view_state,
                    duration: started.elapsed(),
                    bytes: json.len(),
                }),
                Err(error) => SaveOutcome::Failed(error.to_string()),
            }
        })
    }

    /// Waits for the background save that is writing right now, if any, and
    /// cancels every save that was started but has not begun to write.
    /// After this returns the caller may write through the library: nothing
    /// older can follow.
    pub fn settle(&self) {
        if let Some(background) = &self.background {
            lock(background).cancel_before = self.next_seq;
        }
    }
}

/// A poisoned lock only means a save panicked; the store behind it is still
/// a valid connection (the transaction was rolled back when it unwound).
fn lock(background: &Arc<Mutex<Background>>) -> MutexGuard<'_, Background> {
    background
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn write(
    store: &mut Store,
    job: &SaveJob,
    json: &str,
    plain_text: &str,
    stats: Stats,
    derived: &str,
) -> innerword_library::Result<(DocumentMeta, Option<String>)> {
    // The name the user gave is read now, not when the save was queued.
    let kept;
    let title = if job.custom_title.load(Ordering::SeqCst) {
        kept = store.document_meta(job.id)?.title;
        kept.as_str()
    } else {
        derived
    };
    let meta = store.save_document(
        job.id,
        &DocumentContent {
            content: json,
            title,
            plain_text,
            word_count: stats.words as u64,
            char_count: stats.characters as u64,
        },
    )?;
    // Version history. A failure here loses no text.
    if let Err(error) = store.snapshot_if_stale(job.id, job.snapshot_interval) {
        eprintln!("Inner Word: could not take a version snapshot: {error}");
    }
    let view_state = job
        .view_state
        .as_ref()
        .filter(|json| store.save_view_state(job.id, json).is_ok())
        .cloned();
    Ok((meta, view_state))
}

// ----- whiteboards ---------------------------------------------------------------------

/// What a picture of a whiteboard is, for a save: its stored bytes and their
/// type, by Excalidraw's file id.
pub(crate) type StoredPictures =
    std::collections::HashMap<String, (innerword_board::FileHex, String)>;

/// A whiteboard's scene made into what the library stores.
pub(crate) struct PreparedBoard {
    /// The stored scene (`Scene::to_stored`).
    pub scene: String,
    /// The `FileId`s it names.
    pub files: Vec<innerword_library::store::FileId>,
    pub plain_text: String,
    pub words: u64,
    pub characters: u64,
}

impl PreparedBoard {
    /// The content of a save of it.
    pub fn content(&self) -> innerword_library::store::BoardContent<'_> {
        innerword_library::store::BoardContent {
            scene: &self.scene,
            files: &self.files,
            plain_text: &self.plain_text,
            word_count: self.words,
            char_count: self.characters,
        }
    }
}

/// Why a posted scene was not made into a stored one.
pub(crate) enum NotPrepared {
    /// It names pictures the app does not know (their Excalidraw ids): the
    /// page is asked for them first.
    NeedsFiles(Vec<String>),
    /// It cannot be saved at all (not JSON, no scene, over the hard cap):
    /// the reason, for the recovery file.
    Unsaveable(String),
}

/// Makes the scene the page posted (`json`) into what the library stores:
/// `Scene::from_view` with the pictures the app has stored (`stored`), and
/// `known` the ids the app knows at all (a picture it knows as missing or
/// refused is left out of `files`, its element stays). The work of a
/// whiteboard's save, on whichever thread saves.
pub(crate) fn prepare_board(
    json: &str,
    stored: &StoredPictures,
    known: &std::collections::HashSet<String>,
) -> Result<PreparedBoard, NotPrepared> {
    let lookup = |id: &str| stored.get(id).cloned();
    let parsed = innerword_board::Scene::from_view(json, &lookup)
        .map_err(|error| NotPrepared::Unsaveable(error.to_string()))?;
    for warning in &parsed.warnings {
        eprintln!("Inner Word: a whiteboard's scene: {warning}");
    }
    let unknown: Vec<String> = parsed
        .scene
        .named_file_ids()
        .into_iter()
        .filter(|id| !known.contains(id) && !stored.contains_key(id))
        .collect();
    if !unknown.is_empty() {
        return Err(NotPrepared::NeedsFiles(unknown));
    }
    Ok(prepared_of(&parsed.scene))
}

/// What the library stores of `scene`.
pub(crate) fn prepared_of(scene: &innerword_board::Scene) -> PreparedBoard {
    let plain_text = scene.text();
    let stats = innerword_core::Fragment::from_plain_text(&plain_text)
        .to_document()
        .stats();
    PreparedBoard {
        scene: scene.to_stored(),
        files: scene
            .stored_files()
            .iter()
            .filter_map(|hex| innerword_library::store::FileId::parse(hex.as_str()))
            .collect(),
        plain_text,
        words: stats.words as u64,
        characters: stats.characters as u64,
    }
}

/// Everything a background save of a whiteboard needs, taken on the main
/// thread: the raw scene the page posted (it is parsed only here).
pub(crate) struct BoardSaveJob {
    pub seq: u64,
    pub id: DocumentId,
    /// The rev of the posted scene.
    pub rev: u64,
    pub json: Arc<str>,
    pub stored: StoredPictures,
    pub known: std::collections::HashSet<String>,
    /// The stamp of the stored scene the view's scene descends from.
    pub expect: innerword_library::store::ContentStamp,
    /// Where the writer looked, to store with it.
    pub view_state: Option<String>,
    pub snapshot_interval: Duration,
}

/// What a background save of a whiteboard did.
pub(crate) enum BoardSaveOutcome {
    Saved {
        meta: DocumentMeta,
        stamp: innerword_library::store::ContentStamp,
        /// The stored scene that was written.
        scene: Arc<str>,
        files: Vec<String>,
        view_state: Option<String>,
        duration: Duration,
    },
    /// See [`NotPrepared::NeedsFiles`]; nothing was written.
    NeedsFiles(Vec<String>),
    /// The row changed under the view (`StoreError::Stale`); nothing was
    /// written.
    Stale,
    /// The scene cannot be saved (the reason); nothing was written.
    Unsaveable(String),
    Superseded,
    Failed(String),
}

impl Saver {
    /// The work of `job` for a background thread; `None` when this saver is
    /// synchronous.
    pub fn work_board(
        &self,
        job: BoardSaveJob,
    ) -> Option<impl FnOnce() -> BoardSaveOutcome + Send + 'static> {
        let background = self.background.clone()?;
        Some(move || {
            let started = Instant::now();
            let prepared = match prepare_board(&job.json, &job.stored, &job.known) {
                Ok(prepared) => prepared,
                Err(NotPrepared::NeedsFiles(ids)) => return BoardSaveOutcome::NeedsFiles(ids),
                Err(NotPrepared::Unsaveable(why)) => return BoardSaveOutcome::Unsaveable(why),
            };
            let mut background = lock(&background);
            if job.seq < background.cancel_before {
                return BoardSaveOutcome::Superseded;
            }
            let store = &mut background.store;
            match store.save_board(job.id, &prepared.content(), Some(job.expect)) {
                Ok(saved) => {
                    if let Err(error) = store.snapshot_if_stale(job.id, job.snapshot_interval) {
                        eprintln!("Inner Word: could not take a version snapshot: {error}");
                    }
                    let view_state = job
                        .view_state
                        .as_ref()
                        .filter(|json| store.save_view_state(job.id, json).is_ok())
                        .cloned();
                    BoardSaveOutcome::Saved {
                        meta: saved.meta,
                        stamp: saved.stamp,
                        files: prepared.files.iter().map(|id| id.to_hex()).collect(),
                        scene: Arc::from(prepared.scene),
                        view_state,
                        duration: started.elapsed(),
                    }
                }
                Err(innerword_library::StoreError::Stale { .. }) => BoardSaveOutcome::Stale,
                Err(error) => BoardSaveOutcome::Failed(error.to_string()),
            }
        })
    }

    /// The work of storing the shape library (`items`: what the glue
    /// posted) for a background thread; `None` when this saver is
    /// synchronous. The answer is the stored form that was written, or why
    /// nothing was.
    pub fn work_shape_library(
        &self,
        seq: u64,
        items: Arc<str>,
    ) -> Option<impl FnOnce() -> Result<Option<String>, String> + Send + 'static> {
        let background = self.background.clone()?;
        Some(move || {
            let stored = shape_library_to_store(&items)?;
            let mut background = lock(&background);
            if seq < background.cancel_before {
                return Ok(None);
            }
            let raw = serde_json::value::RawValue::from_string(stored.clone())
                .map_err(|error| error.to_string())?;
            background
                .store
                .set_setting(crate::boards::SHAPE_LIBRARY_SETTING, &raw)
                .map_err(|error| error.to_string())?;
            Ok(Some(stored))
        })
    }
}

/// The stored form of the shape library the glue posted (`items`), or why
/// it is not stored: not readable, or over the limits (8 MB stored, 1,000
/// items).
pub(crate) fn shape_library_to_store(items: &str) -> Result<String, String> {
    use innerword_board::limits::{MAX_LIBRARY_BYTES, MAX_LIBRARY_ITEMS};
    let parsed = innerword_board::ShapeLibrary::from_view(items).map_err(|error| match error {
        innerword_board::SceneError::TooLarge { .. } => TOO_LARGE_LIBRARY.to_string(),
        other => other.to_string(),
    })?;
    for warning in &parsed.warnings {
        eprintln!("Inner Word: the shape library: {warning}");
    }
    if parsed.library.items().len() > MAX_LIBRARY_ITEMS {
        return Err(TOO_LARGE_LIBRARY.to_string());
    }
    let stored = parsed.library.to_stored();
    if stored.len() > MAX_LIBRARY_BYTES {
        return Err(TOO_LARGE_LIBRARY.to_string());
    }
    Ok(stored)
}

/// Why a shape library is not stored when it is over the limits.
pub(crate) const TOO_LARGE_LIBRARY: &str = "too large";
