//! The document session: which document is open, opening and closing,
//! autosave, the title, snapshots and the view state.
//!
//! Exactly one document is open at a time. Its text lives in the editor;
//! the library holds the last saved state. The rules:
//!
//! - Every edit marks the session dirty and (re)starts a short timer
//!   ([`AUTOSAVE_DELAY`]); when it fires the document is saved. Typing
//!   without such a pause still saves every [`MAX_AUTOSAVE_WAIT`].
//! - The autosave works on a background thread (see [`super::saver`]): the
//!   main thread only copies the document, so typing in a book does not
//!   stutter when it is saved. One background save runs at a time; edits
//!   made while it runs keep the session dirty and are saved next.
//! - [`Workspace::flush`] saves synchronously and completely. It runs
//!   before another document is opened, when the window closes, when the
//!   app quits and when the window loses focus. It first waits for a
//!   background save that is writing and cancels one that has not started,
//!   so an older copy of the text can never be written after a newer one.
//! - A failed save keeps the session dirty, shows a toast once and retries
//!   every [`RETRY_DELAY`] until it succeeds. Nothing is dropped.
//! - The title is derived from the text until the user renames the document
//!   in the library; from then on the chosen name is kept
//!   ([`Workspace::use_title_from_text`] goes back to deriving). A derived
//!   title follows the text with every edit, undo included; the library's
//!   row follows with the save.
//! - After a successful save the library takes a version snapshot when the
//!   newest one is older than [`SNAPSHOT_INTERVAL`] and the text differs.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use gpui::{AppContext as _, Context, Task, Window};
use writer_core::{Block, BlockKind, Document, Position, SearchOptions, TextRange};
use writer_editor::{Autoscroll, ViewState};
use writer_library::{DocumentId, LibraryItem};
use writer_ui::Toast;

use super::{
    Workspace, WorkspaceEvent,
    saver::{SaveJob, SaveOutcome},
};
use crate::{
    document_io::{StoredForm, UNTITLED, derived_title, load_document},
    settings::keys,
    welcome,
};

/// How long after the last edit the document is saved.
pub const AUTOSAVE_DELAY: Duration = Duration::from_millis(800);
/// The longest edits stay unsaved while the writer keeps typing without a
/// pause long enough for [`AUTOSAVE_DELAY`].
pub const MAX_AUTOSAVE_WAIT: Duration = Duration::from_secs(5);
/// How long after a failed save the next attempt is made.
pub const RETRY_DELAY: Duration = Duration::from_secs(5);
/// The minimum age of the newest version snapshot before another is taken.
pub const SNAPSHOT_INTERVAL: Duration = Duration::from_secs(10 * 60);

/// What the status bar says about saving.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveState {
    /// Everything is in the library.
    Saved,
    /// There are edits the timer has not saved yet.
    Edited,
    /// The last save failed (the message); it is being retried.
    Failed(String),
}

/// What one save cost. Kept for the last save, for measuring (the `state`
/// step of the automation script prints it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SaveCost {
    /// Time spent on the main thread, where it delays typing and frames.
    pub main_thread: Duration,
    /// Time the background thread worked; `None` for a synchronous save.
    pub background: Option<Duration>,
    /// Size of the stored content in bytes.
    pub bytes: usize,
}

/// A background save that has been started and not yet answered.
pub(crate) struct InFlight {
    /// Its position among the saves of this window.
    pub seq: u64,
    /// The document revision that was copied for it.
    pub revision: u64,
    /// Main-thread time spent starting it.
    pub started_in: Duration,
    /// Waits for the background thread and reports back. Dropping it
    /// abandons the answer (the save itself is settled by
    /// [`super::saver::Saver::settle`]).
    pub _task: Task<()>,
}

/// The open document.
pub(crate) struct Session {
    pub id: DocumentId,
    /// The title shown above the text: the one the text gives as it is
    /// now (it follows every edit and is what the next save stores), or
    /// the one the user chose.
    pub title: String,
    /// The user named the document; do not derive the title from the text.
    /// Shared with a background save, which reads it right before writing.
    pub custom_title: Arc<AtomicBool>,
    /// The text changed since the last successful save.
    pub dirty: bool,
    /// When the oldest unsaved edit was made.
    pub dirty_since: Option<std::time::Instant>,
    /// The view state (caret, scroll) as last written to the library.
    pub stored_view_state: Option<String>,
    /// The message of the last failed save, while it has not succeeded since.
    pub failure: Option<String>,
    /// The pending autosave or retry timer.
    pub save_task: Option<Task<()>>,
    /// The background save that is running, if any.
    pub in_flight: Option<InFlight>,
}

impl Session {
    pub fn save_state(&self) -> SaveState {
        match (&self.failure, self.dirty) {
            (Some(message), _) => SaveState::Failed(message.clone()),
            (None, true) => SaveState::Edited,
            (None, false) => SaveState::Saved,
        }
    }
}

impl Workspace {
    // ----- startup ---------------------------------------------------------------

    /// First run: create the welcome document. Then open the document that
    /// was open last, else the most recently edited one.
    pub(crate) fn restore_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let welcome_created: bool = self
            .library
            .read(cx)
            .setting_or(keys::WELCOME_CREATED, false);
        let mut first = None;
        if !welcome_created {
            let empty = {
                let library = self.library.read(cx);
                let tree = library.tree();
                tree.folders.is_empty()
                    && tree.documents.is_empty()
                    && library
                        .trash()
                        .map(|trash| trash.is_empty())
                        .unwrap_or(false)
            };
            let result = self.library.update(cx, |library, cx| {
                let created = if empty {
                    Some(welcome::create_welcome_document(library, cx)?)
                } else {
                    None
                };
                library.set_setting(keys::WELCOME_CREATED, &true, cx)?;
                Ok::<_, writer_library::StoreError>(created)
            });
            match result {
                Ok(created) => first = created.map(|meta| meta.id),
                Err(error) => {
                    self.toast_error(format!("Could not prepare the library: {error}"), cx)
                }
            }
        }

        let target = first.or_else(|| {
            let library = self.library.read(cx);
            let last: Option<DocumentId> = library.setting(keys::LAST_DOCUMENT).ok().flatten();
            last.filter(|id| library.document_exists(*id).unwrap_or(false))
                .or_else(|| {
                    library
                        .recent_documents(1)
                        .ok()
                        .and_then(|recent| recent.first().map(|meta| meta.id))
                })
        });
        if let Some(id) = target {
            self.open_document(id, false, window, cx);
        }
    }

    // ----- opening and closing ---------------------------------------------------

    /// Opens document `id` in the editor, saving the open one first. Returns
    /// whether the document is open afterwards. With `focus_editor` focus
    /// moves to the editor (and any modal closes).
    pub fn open_document(
        &mut self,
        id: DocumentId,
        focus_editor: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.open_document_id() != Some(id) && !self.load_into_editor(id, window, cx) {
            return false;
        }
        if focus_editor {
            self.focus_editor(window, cx);
        }
        true
    }

    fn load_into_editor(
        &mut self,
        id: DocumentId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        // Never leave the open document behind unsaved.
        self.flush(cx);

        let stored = match self.library.read(cx).document(id) {
            Ok(stored) => stored,
            Err(error) => {
                self.toast_error(format!("Could not open the document: {error}"), cx);
                return false;
            }
        };
        let loaded = load_document(&stored.content);
        if let Some(error) = &loaded.recovered_from {
            // Keep the unreadable original as a version before anything is
            // saved over it.
            self.library
                .update(cx, |library, cx| library.create_snapshot(id, cx))
                .ok();
            // Toasts are one line: say what happened to the text, and keep
            // the reason for the log.
            eprintln!(
                "The Writer: “{}” could not be read as a document: {error}",
                display_title(&stored.meta.title)
            );
            self.toast_error(
                "Unreadable document: shown as plain text, the original is kept.",
                cx,
            );
        }
        let view_state: Option<ViewState> = stored
            .view_state
            .as_deref()
            .and_then(|json| serde_json::from_str(json).ok())
            .map(|mut state: ViewState| {
                // With Vim mode on, a selection would open the document in
                // Visual mode. Come back to the caret in Normal mode.
                if self.settings.vim_mode {
                    state.anchor_block = state.head_block;
                    state.anchor_offset = state.head_offset;
                }
                state
            });

        self.editor.update(cx, |editor, cx| {
            editor.set_document(loaded.document, cx);
            editor.set_read_only(false, cx);
            if let Some(view_state) = &view_state {
                editor.restore_view_state(view_state, cx);
            }
        });

        let custom_title = self
            .library
            .read(cx)
            .setting_or(&keys::custom_title(id), false);
        self.session = Some(Session {
            id,
            title: stored.meta.title.clone(),
            custom_title: Arc::new(AtomicBool::new(custom_title)),
            dirty: false,
            dirty_since: None,
            stored_view_state: stored.view_state.clone(),
            failure: None,
            save_task: None,
            in_flight: None,
        });
        self.character_count = None;
        self.limit_counter = None;

        self.panel.update(cx, |panel, cx| {
            panel.set_active_document(Some(id), cx);
            // Follow the open document in the tree unless the user is
            // looking at something else there on purpose.
            if !matches!(panel.selected(), Some(LibraryItem::Document(selected)) if selected == id)
            {
                panel.reveal_document(id, cx);
            }
        });
        self.library
            .update(cx, |library, cx| {
                library.set_setting(keys::LAST_DOCUMENT, &id, cx)
            })
            .ok();
        self.update_window_title(window);
        cx.emit(WorkspaceEvent::DocumentOpened(id));
        cx.notify();
        true
    }

    /// Closes the open document (saving it first) and shows the empty state.
    pub fn close_document(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.session.is_none() {
            return;
        }
        self.flush(cx);
        self.session = None;
        self.character_count = None;
        self.limit_counter = None;
        self.editor.update(cx, |editor, cx| {
            editor.set_document(Document::new(), cx);
            editor.set_read_only(true, cx);
        });
        self.panel
            .update(cx, |panel, cx| panel.set_active_document(None, cx));
        if self.editor.read(cx).is_focused(window) {
            window.focus(&self.focus_handle);
        }
        self.update_window_title(window);
        cx.emit(WorkspaceEvent::DocumentClosed);
        cx.notify();
    }

    /// The open document went to the trash: save what was typed (the store
    /// accepts saves on trashed documents), then show the most recently
    /// edited document that is left, or the empty state.
    pub(crate) fn open_document_left(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let had_editor_focus = self.editor.read(cx).is_focused(window);
        self.close_document(window, cx);
        let next = self
            .library
            .read(cx)
            .recent_documents(1)
            .ok()
            .and_then(|recent| recent.first().map(|meta| meta.id));
        if let Some(next) = next {
            self.open_document(next, had_editor_focus, window, cx);
        }
    }

    // ----- creating --------------------------------------------------------------

    /// Creates an empty document in the folder selected in the library,
    /// opens it and focuses the editor.
    pub fn new_document(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<DocumentId> {
        self.create_document(None, window, cx)
    }

    /// Like [`Workspace::new_document`], with `name` as the first heading
    /// (so it becomes the title) and the caret on the line below.
    pub fn new_document_named(
        &mut self,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<DocumentId> {
        self.create_document(
            Some(name.trim()).filter(|name| !name.is_empty()),
            window,
            cx,
        )
    }

    fn create_document(
        &mut self,
        name: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<DocumentId> {
        self.dismiss_modal(window, cx);
        let document = match name {
            Some(name) => Document::from_blocks(vec![
                Block::with_text(BlockKind::heading(1), name),
                Block::paragraph(""),
            ]),
            None => Document::new(),
        }
        .with_settings(self.settings.new_document_settings());
        let form = StoredForm::of(&document, None);

        let folder = self.panel.read(cx).selected_folder();
        let created = self.library.update(cx, |library, cx| {
            library
                .create_document(folder, &form.content(), cx)
                // The selected folder may be gone (trashed elsewhere).
                .or_else(|_| library.create_document(None, &form.content(), cx))
        });
        let meta = match created {
            Ok(meta) => meta,
            Err(error) => {
                self.toast_error(format!("Could not create the document: {error}"), cx);
                return None;
            }
        };
        if !self.open_document(meta.id, true, window, cx) {
            return None;
        }
        if name.is_some() {
            self.editor.update(cx, |editor, cx| {
                editor.edit(|state| state.set_caret(Position::new(1, 0)), cx);
            });
        }
        Some(meta.id)
    }

    /// Creates a folder in the folder selected in the library and starts
    /// renaming it there.
    pub fn new_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_modal(window, cx);
        let parent = self.panel.read(cx).selected_folder();
        let created = self.library.update(cx, |library, cx| {
            library
                .create_folder(parent, "New folder", cx)
                .or_else(|_| library.create_folder(None, "New folder", cx))
        });
        let folder = match created {
            Ok(folder) => folder,
            Err(error) => {
                self.toast_error(format!("Could not create the folder: {error}"), cx);
                return;
            }
        };
        if !self.settings.sidebar_visible {
            self.set_sidebar_visible(true, window, cx);
        }
        self.last_side_panel = super::SidePanel::Library;
        let panel = self.panel.clone();
        window.focus(&gpui::Focusable::focus_handle(panel.read(cx), cx));
        panel.update(cx, |panel, cx| {
            let item = LibraryItem::Folder(folder.id);
            panel.reveal(item, cx);
            panel.begin_rename(item, window, cx);
        });
    }

    // ----- saving ----------------------------------------------------------------

    /// An edit happened: remember it and (re)start the autosave timer.
    pub(crate) fn mark_dirty(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.session.as_mut() else {
            return;
        };
        session.dirty = true;
        let now = cx.background_executor().now();
        session.dirty_since.get_or_insert(now);
        // The title above the text follows the text at once (an edit, an
        // undo, a heading made or unmade), not when the save comes round.
        if !session.custom_title.load(Ordering::SeqCst) {
            let title = derived_title(self.editor.read(cx).document());
            if session.title != title {
                session.title = title;
            }
        }
        let delay = self.autosave_delay(cx);
        self.schedule_save(delay, cx);
        cx.notify();
    }

    /// How long from now the open document should be saved.
    fn autosave_delay(&self, cx: &Context<Self>) -> Duration {
        let Some(session) = self.session.as_ref() else {
            return AUTOSAVE_DELAY;
        };
        if session.failure.is_some() {
            // While a save is failing, keep the retry rhythm instead of
            // hammering the store on every keystroke.
            return RETRY_DELAY;
        }
        // Debounce, but never let steady typing postpone the save beyond
        // MAX_AUTOSAVE_WAIT.
        let now = cx.background_executor().now();
        let waited = session
            .dirty_since
            .map(|since| now.saturating_duration_since(since))
            .unwrap_or_default();
        AUTOSAVE_DELAY.min(MAX_AUTOSAVE_WAIT.saturating_sub(waited))
    }

    fn schedule_save(&mut self, delay: Duration, cx: &mut Context<Self>) {
        let Some(session) = self.session.as_mut() else {
            return;
        };
        let id = session.id;
        session.save_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            this.update(cx, |this, cx| {
                if this.open_document_id() == Some(id) {
                    this.autosave(cx);
                }
            })
            .ok();
        }));
    }

    /// The autosave timer fired: save the open document without holding up
    /// the main thread, when that is possible.
    pub fn autosave(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.session.as_mut() else {
            return;
        };
        // The timer that called this has done its work.
        session.save_task = None;
        if session.in_flight.is_some() {
            // One background save at a time. When it reports back and the
            // text has changed meanwhile, the next one is scheduled.
            return;
        }
        if !session.dirty || !self.saver.is_background() {
            self.save_now(cx);
            return;
        }
        self.save_in_background(cx);
    }

    /// Starts a background save of the open document as it is now.
    fn save_in_background(&mut self, cx: &mut Context<Self>) {
        let started = Instant::now();
        let Some(session) = self.session.as_mut() else {
            return;
        };
        let id = session.id;
        let (document, stats, revision, view_state) = {
            let editor = self.editor.read(cx);
            (
                // The only heavy thing the main thread does for an
                // autosave: well under a millisecond for a book.
                editor.document().clone(),
                // Counts that are still being brought up to date (a book
                // that was just opened) are counted on the background
                // thread instead of storing stale ones.
                (!editor.stats_pending()).then(|| editor.stats()),
                editor.state().revision(),
                serde_json::to_string(&editor.view_state()).ok(),
            )
        };
        let seq = self.saver.next_seq();
        let job = SaveJob {
            seq,
            id,
            document,
            stats,
            custom_title: session.custom_title.clone(),
            view_state: view_state
                .filter(|json| session.stored_view_state.as_deref() != Some(json.as_str())),
            snapshot_interval: SNAPSHOT_INTERVAL,
        };
        let Some(work) = self.saver.work(job) else {
            self.save_now(cx);
            return;
        };
        // Edits from here on are newer than what is being saved.
        session.dirty_since = None;
        let background = cx.background_spawn(async move { work() });
        let task = cx.spawn(async move |this, cx| {
            let outcome = background.await;
            this.update(cx, |this, cx| {
                this.background_save_finished(id, seq, outcome, cx)
            })
            .ok();
        });
        session.in_flight = Some(InFlight {
            seq,
            revision,
            started_in: started.elapsed(),
            _task: task,
        });
    }

    /// A background save reported back.
    fn background_save_finished(
        &mut self,
        id: DocumentId,
        seq: u64,
        outcome: SaveOutcome,
        cx: &mut Context<Self>,
    ) {
        let started = Instant::now();
        let Some(session) = self.session.as_mut().filter(|session| session.id == id) else {
            return;
        };
        if session.in_flight.as_ref().map(|save| save.seq) != Some(seq) {
            return;
        }
        let Some(in_flight) = session.in_flight.take() else {
            return;
        };
        match outcome {
            // A synchronous save took over; it has dealt with the session.
            SaveOutcome::Superseded => {}
            SaveOutcome::Failed(message) => self.save_failed(message, cx),
            SaveOutcome::Saved(saved) => {
                // Clean only if nothing was edited while the save ran.
                let unchanged = self.editor.read(cx).state().revision() == in_flight.revision;
                // The title of text that has changed since is already the
                // newer one (`mark_dirty`); a title the user chose is
                // whatever the library has.
                if unchanged || session.custom_title.load(Ordering::SeqCst) {
                    session.title = saved.meta.title.clone();
                }
                if let Some(view_state) = saved.view_state {
                    session.stored_view_state = Some(view_state);
                }
                if unchanged {
                    session.dirty = false;
                    session.dirty_since = None;
                    session.save_task = None;
                }
                let recovered = session.failure.take().is_some();
                // The write went past the library's own connection: its
                // cached tree (title, counts, edit time) and its
                // subscribers have to hear of it.
                if let Err(error) = self.library.update(cx, |library, cx| {
                    library.document_saved_elsewhere(&saved.meta, cx)
                }) {
                    eprintln!("The Writer: could not re-read the library: {error}");
                }
                if recovered {
                    self.show_toast(Toast::new("Saved").success(), cx);
                }
                if !unchanged {
                    self.ensure_save_scheduled(cx);
                }
                self.last_save = Some(SaveCost {
                    main_thread: in_flight.started_in + started.elapsed(),
                    background: Some(saved.duration),
                    bytes: saved.bytes,
                });
                cx.notify();
            }
        }
    }

    /// Makes sure a dirty session has a save coming.
    fn ensure_save_scheduled(&mut self, cx: &mut Context<Self>) {
        let waiting = self.session.as_ref().is_some_and(|session| {
            session.dirty && session.save_task.is_none() && session.in_flight.is_none()
        });
        if waiting {
            let delay = self.autosave_delay(cx);
            self.schedule_save(delay, cx);
        }
    }

    /// Saves the open document and its view state now, if there is anything
    /// to save, and returns when it is in the library. Returns `false` when
    /// the save failed (it is then retried).
    pub fn flush(&mut self, cx: &mut Context<Self>) -> bool {
        self.save_now(cx)
    }

    /// What the last save of this window cost.
    pub fn last_save_cost(&self) -> Option<SaveCost> {
        self.last_save
    }

    /// Whether a background save of the open document is running.
    pub fn is_saving_in_background(&self) -> bool {
        self.session
            .as_ref()
            .is_some_and(|session| session.in_flight.is_some())
    }

    /// The save itself. Synchronous.
    pub(crate) fn save_now(&mut self, cx: &mut Context<Self>) -> bool {
        let started = Instant::now();
        // Nothing older may be written after this: wait for the background
        // save that is writing, cancel one that has not started. Its answer
        // is not needed any more; the session is still dirty, so the text
        // as it is now is written below.
        if let Some(session) = self.session.as_mut() {
            session.in_flight = None;
        }
        self.saver.settle();

        let Some(session) = self.session.as_ref() else {
            return true;
        };
        let id = session.id;
        if !session.dirty {
            self.save_view_state(cx);
            return true;
        }

        // A title the user chose is read back from the library, so a rename
        // made a moment ago is never overwritten by an older copy.
        let kept_title = if session.custom_title.load(Ordering::SeqCst) {
            self.library
                .read(cx)
                .document_meta(id)
                .ok()
                .map(|meta| meta.title)
        } else {
            None
        };
        let form = {
            let editor = self.editor.read(cx);
            // The editor keeps the statistics current; no need to recount,
            // unless it is still counting a document it has just opened.
            let stats = if editor.stats_pending() {
                editor.document().stats()
            } else {
                editor.stats()
            };
            StoredForm::with_stats(editor.document(), kept_title.as_deref(), stats)
        };
        let result = self.library.update(cx, |library, cx| {
            library.save_document(id, &form.content(), cx)
        });

        match result {
            Ok(meta) => {
                let session = self.session.as_mut().expect("session checked above");
                session.dirty = false;
                session.dirty_since = None;
                session.title = meta.title;
                session.save_task = None;
                let recovered = session.failure.take().is_some();
                if recovered {
                    self.show_toast(Toast::new("Saved").success(), cx);
                }
                // Version history. A failure here loses no text and is not
                // worth interrupting the writer for.
                if let Err(error) = self.library.update(cx, |library, cx| {
                    library.snapshot_if_stale(id, SNAPSHOT_INTERVAL, cx)
                }) {
                    eprintln!("The Writer: could not take a version snapshot: {error}");
                }
                self.save_view_state(cx);
                self.last_save = Some(SaveCost {
                    main_thread: started.elapsed(),
                    background: None,
                    bytes: form.json.len(),
                });
                cx.notify();
                true
            }
            Err(error) => {
                self.save_failed(error.to_string(), cx);
                false
            }
        }
    }

    /// A save failed: keep the session dirty, say so once, and try again
    /// after [`RETRY_DELAY`].
    fn save_failed(&mut self, message: String, cx: &mut Context<Self>) {
        let Some(session) = self.session.as_mut() else {
            return;
        };
        let first_failure = session.failure.is_none();
        session.failure = Some(message.clone());
        if first_failure {
            // One line: what matters to the writer first, the reason after.
            self.toast_error(
                format!("Not saved yet. Your text is kept and the save is retried. ({message})"),
                cx,
            );
        }
        self.schedule_save(RETRY_DELAY, cx);
        cx.notify();
    }

    /// Writes the caret and scroll position when they differ from what the
    /// library has. Cheap enough to call at every flush point.
    fn save_view_state(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.session.as_ref() else {
            return;
        };
        let id = session.id;
        let view_state = self.editor.read(cx).view_state();
        let Ok(json) = serde_json::to_string(&view_state) else {
            return;
        };
        if session.stored_view_state.as_deref() == Some(json.as_str()) {
            return;
        }
        let saved = self
            .library
            .update(cx, |library, _| library.save_view_state(id, &json));
        if saved.is_ok()
            && let Some(session) = self.session.as_mut()
        {
            session.stored_view_state = Some(json);
        }
    }

    // ----- versions --------------------------------------------------------------

    /// Puts the text of version `snapshot` back as the text of the open
    /// document. The text as it is now is saved and kept as a version
    /// first, so the step can be taken back from the version history.
    /// Returns whether the version was restored.
    pub fn restore_version(
        &mut self,
        snapshot: writer_library::SnapshotId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(id) = self.open_document_id() else {
            return false;
        };
        let stored = match self.library.read(cx).snapshot(snapshot) {
            Ok(stored) if stored.meta.document_id == id => stored,
            Ok(_) => return false,
            Err(error) => {
                self.toast_error(format!("Could not read that version: {error}"), cx);
                return false;
            }
        };
        // What is on screen goes to the library, and becomes a version
        // itself (unless the newest version already is this text).
        let kept = self.flush(cx)
            && self
                .library
                .update(cx, |library, cx| {
                    library.snapshot_if_stale(id, Duration::ZERO, cx)
                })
                .is_ok();
        if !kept {
            self.toast_error(
                "Nothing was restored: the text as it is now could not be saved.",
                cx,
            );
            return false;
        }

        let loaded = load_document(&stored.content);
        self.editor.update(cx, |editor, cx| {
            editor.set_document(loaded.document, cx);
        });
        self.character_count = None;
        self.limit_counter = None;
        if let Some(session) = self.session.as_mut() {
            session.dirty = true;
        }
        let saved = self.save_now(cx);
        self.focus_editor(window, cx);
        self.update_window_title(window);
        if saved {
            let when = crate::version_history::version_time_label(
                crate::version_history::LocalTime::from_unix_ms(stored.meta.created_at),
                crate::version_history::LocalTime::from_unix_ms(
                    self.library.read(cx).clock().now_ms(),
                ),
            );
            self.show_toast(
                Toast::new(format!(
                    "Restored {when}. The text from before is kept as a version."
                ))
                .success()
                .duration(Duration::from_secs(6)),
                cx,
            );
        }
        cx.notify();
        saved
    }

    // ----- title -----------------------------------------------------------------

    /// The library reported a rename. The app never renames documents
    /// itself, so this is the user naming the document in the sidebar: from
    /// now on the title is theirs.
    pub(crate) fn document_renamed(&mut self, id: DocumentId, cx: &mut Context<Self>) {
        let key = keys::custom_title(id);
        self.library
            .update(cx, |library, cx| library.set_setting(&key, &true, cx))
            .ok();
        let title = self
            .library
            .read(cx)
            .document_meta(id)
            .ok()
            .map(|meta| meta.title);
        if let Some(session) = self.session.as_mut().filter(|session| session.id == id) {
            session.custom_title.store(true, Ordering::SeqCst);
            if let Some(title) = title {
                session.title = title;
            }
            cx.notify();
        }
    }

    /// Re-reads the open document's title from the library.
    pub(crate) fn refresh_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.open_document_id() else {
            return;
        };
        let Ok(meta) = self.library.read(cx).document_meta(id) else {
            return;
        };
        if let Some(session) = self.session.as_mut()
            && session.title != meta.title
            // Text that is newer than what the library holds has a newer
            // title too, unless the user chose the title.
            && (!session.dirty || session.custom_title.load(Ordering::SeqCst))
        {
            session.title = meta.title;
            cx.notify();
        }
        self.update_window_title(window);
    }

    /// Forgets the name the user gave the open document: its title follows
    /// the text again.
    pub fn use_title_from_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.session.as_mut() else {
            return;
        };
        let id = session.id;
        session.custom_title.store(false, Ordering::SeqCst);
        session.dirty = true;
        let key = keys::custom_title(id);
        self.library
            .update(cx, |library, cx| library.remove_setting(&key, cx))
            .ok();
        self.save_now(cx);
        self.update_window_title(window);
    }

    /// The title for the titlebar band: the open document's title (see
    /// [`Session::title`]), or the app name.
    pub(crate) fn display_title(&self) -> String {
        match &self.session {
            Some(session) => display_title(&session.title),
            None => "The Writer".to_string(),
        }
    }

    /// Keeps the native window title (Mission Control, the Window menu) in
    /// step with the document.
    pub(crate) fn update_window_title(&mut self, window: &mut Window) {
        let title = self.display_title();
        if self.window_title != title {
            window.set_window_title(&title);
            self.window_title = title;
        }
    }

    // ----- search ----------------------------------------------------------------

    /// Opens document `id` and selects the first place where `query` occurs
    /// in its text (the whole query, else the earliest of its words).
    pub fn open_search_hit(
        &mut self,
        id: DocumentId,
        query: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.open_document(id, true, window, cx) {
            return false;
        }
        let found = first_occurrence(self.editor.read(cx).document(), query);
        if let Some(range) = found {
            self.editor.update(cx, |editor, cx| {
                editor.edit_with(Autoscroll::Center, |state| state.select_range(range), cx);
            });
        }
        true
    }
}

/// The stored title, or [`UNTITLED`] when it is blank.
pub(crate) fn display_title(title: &str) -> String {
    if title.trim().is_empty() {
        UNTITLED.to_string()
    } else {
        title.to_string()
    }
}

/// The first occurrence of `query` in `document`: the whole query (without
/// surrounding quotes) when it occurs, else the earliest occurrence of any
/// of its words. Case-insensitive.
pub fn first_occurrence(document: &Document, query: &str) -> Option<TextRange> {
    let options = SearchOptions::default();
    let whole = query
        .trim()
        .trim_matches(|c| matches!(c, '"' | '“' | '”'))
        .trim();
    if whole.is_empty() {
        return None;
    }
    if let Some(range) = document.find_all(whole, options).into_iter().next() {
        return Some(range);
    }
    whole
        .split(|c: char| c.is_whitespace() || matches!(c, '"' | '“' | '”'))
        .filter(|word| !word.is_empty())
        .filter_map(|word| document.find_all(word, options).into_iter().next())
        .min_by_key(|range| range.start)
}
