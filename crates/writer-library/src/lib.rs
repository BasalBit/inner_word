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
//! All calls are **synchronous on the main thread**. SQLite is fast enough for this app; the
//! app debounces autosave. A content save does not reload the tree (its entry is updated in
//! place), so it costs what the database write costs: about 0.2 ms for a document of a few
//! pages in a library of 5,000 documents, and about 22 ms for a single document of 1 MB,
//! nearly all of it the search index (release build, measured with
//! `LIBRARY_DEMO_BENCH=5000 cargo run --release -p writer-library --example library_demo`).
//! Writes that change the shape of the tree reload it, which takes under a millisecond for
//! 5,000 documents.
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
//! [`LibraryPanel`] is the sidebar view over an `Entity<Library>`: the tree of folders and
//! documents, a name filter and the trash, all driven from the keyboard (the keys are listed
//! in its docs). It emits [`LibraryPanelEvent`]s (open this document, dismissed) and exposes
//! the selection so the app knows where `cmd-n` should create
//! ([`LibraryPanel::selected_folder`]).
//!
//! ```ignore
//! writer_library::init(cx);                                  // the panel's key bindings
//! let panel = cx.new(|cx| LibraryPanel::new(library.clone(), cx));
//! panel.update(cx, |panel, cx| {
//!     panel.set_modal_layer(modal_layer.clone(), cx);        // the window's own modal layer
//!     panel.set_new_document_content(empty_document_json);   // what `n` puts in a new document
//! });
//! // Toasts ("Moved “Chapter 3” to the trash. Press ⌘Z to undo.") go through
//! // writer_ui::toast::show_toast, so install a global ToastLayer.
//! // In the window root's render, so the panel's commands also work from the editor:
//! writer_library::forward_actions(div().size_full(), &panel)
//! // The palette lists writer_library::commands().
//! ```

#![warn(missing_docs)]

mod library;
mod panel;

pub use library::{DocumentEntry, Library, LibraryEvent};
pub use panel::{
    Collapse, CollapseAll, DeleteForever, Dismiss, Duplicate, EXPANDED_FOLDERS_SETTING, EmptyTrash,
    Expand, ExpandAll, FocusFilter, FocusTree, HEADER_HEIGHT, KEY_CONTEXT, LibraryItem,
    LibraryMenuTarget, LibraryPanel, LibraryPanelEvent, MoveDown, MoveToFolder, MoveToTrash,
    MoveUp, NewDocument, NewFolder, OpenSelected, ROOT_ROW_HEIGHT, ROW_DETAIL_SETTING, ROW_HEIGHT,
    Rename, RestoreSelected, RowDetail, SELECTED_ITEM_SETTING, SelectFirst, SelectLast, SelectNext,
    SelectPageDown, SelectPageUp, SelectPrevious, ShowTrash, TRASH_ROW_HEIGHT, ToggleRowDetail,
    UndoTrash, commands, forward_actions, key_bindings,
};

/// Text helpers of the panel, public only so that the crate's one integration-test binary
/// can test them directly. Not part of the API.
#[doc(hidden)]
pub mod internals {
    pub use crate::panel::rows::{label_around_match, short, short_path};
}

/// The whole storage crate, for anything not re-exported above.
pub use writer_store as store;
/// The storage types that appear in this crate's API, re-exported so that consumers do not
/// need a direct dependency on `writer-store`.
pub use writer_store::{
    Clock, DATA_DIR_ENV, Document, DocumentContent, DocumentId, DocumentMeta, EmptiedTrash, Folder,
    FolderId, FolderNode, ItemKind, LibraryTree, MATCH_END, MATCH_START, Result, SearchHit,
    Snapshot, SnapshotId, SnapshotMeta, SnapshotPolicy, SnippetPart, StoreError, TrashEntry,
    data_dir, database_path,
};

/// Installs the key bindings of the library panel. Call once at startup, after the app's
/// own context-free bindings (a later binding wins at equal context depth).
pub fn init(cx: &mut gpui::App) {
    cx.bind_keys(panel::key_bindings());
}
