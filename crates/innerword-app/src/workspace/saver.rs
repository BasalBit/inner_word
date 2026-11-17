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
