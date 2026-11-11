//! The library of The Writer: the [`Library`] model and the [`LibraryPanel`] sidebar.
//!
//! # Library
//!
//! [`Library`] is a GPUI entity and the **single owner** of the app's
//! [`writer_store::Store`]. Every read and write of folders, documents, trash, snapshots,
//! settings and search goes through it. Each mutating method calls the store, refreshes the
//! cached [`LibraryTree`] and emits [`LibraryEvent`]s, so every view that subscribes stays in
//! sync no matter who made the change.
//!
//! All calls are **synchronous on the main thread**. SQLite is fast enough for this app
//! (single-digit milliseconds for a save of a normal document); the app debounces autosave.
//!
//! Errors are returned as [`StoreError`] (`Result<T>`), never panics. A failed call changes
//! nothing and emits nothing.
//!
//! ```ignore
//! use writer_library::{Library, LibraryEvent, DocumentContent};
//!
//! // Startup: THE_WRITER_DATA_DIR is honoured by the store's own helpers.
//! let library = match Library::open_default() {
//!     Ok(library) => cx.new(|_| library),
//!     Err(err) => { /* show err.to_string(), quit */ return; }
//! };
//! writer_library::init(cx);                       // panel key bindings
//!
//! // Stay in sync (inside a view: store the Subscription).
//! cx.subscribe(&library, |this: &mut Workspace, _library, event: &LibraryEvent, cx| match event {
//!     LibraryEvent::DocumentTrashed(id) => this.close_if_open(*id, cx),
//!     LibraryEvent::FolderTrashed { documents, .. } => { /* close if the open one is listed */ }
//!     LibraryEvent::SettingChanged { key } if key.as_ref() == "theme" => { /* re-read */ }
//!     _ => {}
//! });
//!
//! // Load and save.
//! let document = library.read(cx).document(id)?;          // content JSON + view state
//! library.update(cx, |library, cx| {
//!     library.save_document(id, &DocumentContent {
//!         content: &json, title: &title, plain_text: &plain, word_count, char_count,
//!     }, cx)
//! })?;
//!
//! // Settings.
//! let vim: bool = library.read(cx).setting_or("editor.vim_mode", false);
//! library.update(cx, |library, cx| library.set_setting("editor.vim_mode", &true, cx))?;
//! ```
//!
//! # LibraryPanel
//!
//! [`LibraryPanel`] is the sidebar view over a `Entity<Library>`. It emits
//! [`LibraryPanelEvent`]s (open this document, dismissed) and exposes the selection so the
//! app knows where `cmd-n` should create ([`LibraryPanel::selected_folder`]).

#![warn(missing_docs)]

mod library;
mod panel;

pub use library::{DocumentEntry, Library, LibraryEvent};
pub use panel::{
    Collapse, Dismiss, Expand, LibraryItem, LibraryPanel, LibraryPanelEvent, OpenSelected,
    SelectFirst, SelectLast, SelectNext, SelectPrevious, EXPANDED_FOLDERS_SETTING, KEY_CONTEXT,
    key_bindings,
};

/// The storage types that appear in this crate's API, re-exported so that consumers do not
/// need a direct dependency on `writer-store`.
pub use writer_store::{
    Clock, DATA_DIR_ENV, Document, DocumentContent, DocumentId, DocumentMeta, EmptiedTrash,
    Folder, FolderId, FolderNode, ItemKind, LibraryTree, MATCH_END, MATCH_START, Result,
    SearchHit, Snapshot, SnapshotId, SnapshotMeta, SnapshotPolicy, SnippetPart, StoreError,
    TrashEntry, data_dir, database_path,
};
/// The whole storage crate, for anything not re-exported above.
pub use writer_store as store;

/// Installs the key bindings of the library panel. Call once at startup, after the app's
/// own context-free bindings (a later binding wins at equal context depth).
pub fn init(cx: &mut gpui::App) {
    cx.bind_keys(panel::key_bindings());
}
