//! The document session: which document is open, opening and closing,
//! autosave, the title, snapshots and the view state.
//!
//! Exactly one document is open at a time. Its text lives in the editor;
//! the library holds the last saved state. The rules:
//!
//! - Every edit marks the session dirty and (re)starts a short timer
//!   ([`AUTOSAVE_DELAY`]); when it fires the document is saved.
//! - [`Workspace::flush`] saves synchronously. It runs before another
//!   document is opened, when the window closes, when the app quits and when
//!   the window loses focus.
//! - A failed save keeps the session dirty, shows a toast once and retries
//!   every [`RETRY_DELAY`] until it succeeds. Nothing is dropped.
//! - The title is derived from the text until the user renames the document
//!   in the library; from then on the chosen name is kept
//!   ([`Workspace::use_title_from_text`] goes back to deriving).
//! - After a successful save the library takes a version snapshot when the
//!   newest one is older than [`SNAPSHOT_INTERVAL`] and the text differs.

use std::time::Duration;

use gpui::{Context, Task, Window};
use writer_core::{Block, BlockKind, Document, Position, SearchOptions, TextRange};
use writer_editor::{Autoscroll, ViewState};
use writer_library::{DocumentId, LibraryItem};
use writer_ui::Toast;

use super::{Workspace, WorkspaceEvent};
use crate::{
    document_io::{StoredForm, UNTITLED, load_document},
    settings::keys,
    welcome,
};

/// How long after the last edit the document is saved.
pub const AUTOSAVE_DELAY: Duration = Duration::from_millis(800);
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

/// The open document.
pub(crate) struct Session {
    pub id: DocumentId,
    /// The title as stored in the library.
    pub title: String,
    /// The user named the document; do not derive the title from the text.
    pub custom_title: bool,
    /// The text changed since the last successful save.
    pub dirty: bool,
    /// The caret or scroll position changed since it was last stored.
    pub view_dirty: bool,
    /// The message of the last failed save, while it has not succeeded since.
    pub failure: Option<String>,
    /// The pending autosave or retry.
    pub save_task: Option<Task<()>>,
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
                    && library.trash().map(|trash| trash.is_empty()).unwrap_or(false)
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
            self.toast_error(
                format!(
                    "“{}” could not be read as a document ({error}). It is shown as plain \
                     text; the original is kept in its version history.",
                    display_title(&stored.meta.title)
                ),
                cx,
            );
        }
        let view_state: Option<ViewState> = stored
            .view_state
            .as_deref()
            .and_then(|json| serde_json::from_str(json).ok());

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
            custom_title,
            dirty: false,
            view_dirty: false,
            failure: None,
            save_task: None,
        });
        self.character_count = None;

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
        self.editor.update(cx, |editor, cx| {
            editor.set_document(Document::new(), cx);
            editor.set_read_only(true, cx);
        });
        self.panel
            .update(cx, |panel, cx| panel.set_active_document(None, cx));
        if self
            .editor
            .read(cx)
            .is_focused(window)
        {
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
        self.create_document(Some(name.trim()).filter(|name| !name.is_empty()), window, cx)
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
        session.view_dirty = true;
        // While a save is failing, keep the retry rhythm instead of
        // hammering the store on every keystroke.
        let delay = if session.failure.is_some() {
            RETRY_DELAY
        } else {
            AUTOSAVE_DELAY
        };
        self.schedule_save(delay, cx);
        cx.notify();
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
                    this.save_now(cx);
                }
            })
            .ok();
        }));
    }

    /// Saves the open document and its view state now, if there is anything
    /// to save. Returns `false` when the save failed (it is then retried).
    pub fn flush(&mut self, cx: &mut Context<Self>) -> bool {
        self.save_now(cx)
    }

    /// The save itself. Synchronous.
    pub(crate) fn save_now(&mut self, cx: &mut Context<Self>) -> bool {
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
        let kept_title = if session.custom_title {
            self.library
                .read(cx)
                .document_meta(id)
                .ok()
                .map(|meta| meta.title)
        } else {
            None
        };
        let form = StoredForm::of(self.editor.read(cx).document(), kept_title.as_deref());
        let result = self.library.update(cx, |library, cx| {
            library.save_document(id, &form.content(), cx)
        });

        match result {
            Ok(meta) => {
                let session = self.session.as_mut().expect("session checked above");
                session.dirty = false;
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
                cx.notify();
                true
            }
            Err(error) => {
                let message = error.to_string();
                let session = self.session.as_mut().expect("session checked above");
                let first_failure = session.failure.is_none();
                session.failure = Some(message.clone());
                if first_failure {
                    self.toast_error(
                        format!(
                            "Could not save: {message}. Your text is kept here and the save \
                             is retried."
                        ),
                        cx,
                    );
                }
                self.schedule_save(RETRY_DELAY, cx);
                cx.notify();
                false
            }
        }
    }

    fn save_view_state(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.session.as_mut() else {
            return;
        };
        if !session.view_dirty {
            return;
        }
        let id = session.id;
        let view_state = self.editor.read(cx).view_state();
        let Ok(json) = serde_json::to_string(&view_state) else {
            return;
        };
        let saved = self
            .library
            .update(cx, |library, _| library.save_view_state(id, &json));
        if saved.is_ok()
            && let Some(session) = self.session.as_mut()
        {
            session.view_dirty = false;
        }
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
            session.custom_title = true;
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
        session.custom_title = false;
        session.dirty = true;
        let key = keys::custom_title(id);
        self.library
            .update(cx, |library, cx| library.remove_setting(&key, cx))
            .ok();
        self.save_now(cx);
        self.update_window_title(window);
    }

    /// The title for the titlebar band: the stored title, or the app name.
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
