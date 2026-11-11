//! [`Library`]: the single owner of the store, with change events.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use gpui::{Context, EventEmitter, SharedString};
use serde::Serialize;
use serde::de::DeserializeOwned;
use writer_store::{
    Clock, Document, DocumentContent, DocumentId, DocumentMeta, EmptiedTrash, Folder, FolderId,
    FolderNode, LibraryTree, Result, SearchHit, Snapshot, SnapshotId, SnapshotMeta, Store,
    TrashEntry,
};

/// A change made through a [`Library`].
///
/// Rules every subscriber can rely on:
///
/// - Events are emitted only after the store call succeeded, and after the cached tree was
///   refreshed, so a handler that reads [`Library::tree`] sees the new state.
/// - One call emits its specific event first (for example [`LibraryEvent::DocumentTrashed`]),
///   then [`LibraryEvent::TrashChanged`] if the trash listing changed, then
///   [`LibraryEvent::TreeChanged`] if the structure, order or names in the tree changed.
/// - A view that only draws the tree needs `TreeChanged` alone. A view that only draws the
///   trash needs `TrashChanged` alone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LibraryEvent {
    /// The live tree changed in structure, order or names: something was created, renamed,
    /// moved, reordered, trashed or restored, or a save changed a document's title.
    /// Not emitted for a content save that keeps the title (see `DocumentSaved`).
    TreeChanged,
    /// A document was created (also by duplicating; the id is the new document).
    DocumentCreated(DocumentId),
    /// A document's content was saved. Word and character counts and `updated_at` in the
    /// cached tree are fresh.
    DocumentSaved {
        /// The saved document.
        id: DocumentId,
        /// True when the save changed the title (a `TreeChanged` follows).
        title_changed: bool,
    },
    /// A document was renamed through [`Library::rename_document`].
    DocumentRenamed(DocumentId),
    /// A document was moved to another folder or reordered in its folder.
    DocumentMoved(DocumentId),
    /// A document was moved to the trash on its own. An editor showing it should close it
    /// (saving first is safe: saves work on trashed documents).
    DocumentTrashed(DocumentId),
    /// A document was restored from the trash.
    DocumentRestored(DocumentId),
    /// A trashed document was deleted for good.
    DocumentPurged(DocumentId),
    /// A folder was created.
    FolderCreated(FolderId),
    /// A folder was renamed.
    FolderRenamed(FolderId),
    /// A folder was moved to another parent or reordered among its siblings.
    FolderMoved(FolderId),
    /// A folder was moved to the trash together with its whole subtree.
    FolderTrashed {
        /// The trashed folder.
        id: FolderId,
        /// Every live document that went to the trash with it, at any depth. An editor
        /// showing one of these should close it.
        documents: Vec<DocumentId>,
    },
    /// A folder and what was trashed with it were restored.
    FolderRestored(FolderId),
    /// A trashed folder and what was trashed with it were deleted for good.
    FolderPurged(FolderId),
    /// The trash listing changed: something was trashed, restored or purged, or the trash
    /// was emptied.
    TrashChanged,
    /// A version snapshot of this document was created or deleted.
    SnapshotsChanged(DocumentId),
    /// A setting was written or removed.
    SettingChanged {
        /// The setting's key.
        key: SharedString,
    },
}

/// A live document with the names of the folders that lead to it, for quick open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentEntry {
    /// The document's metadata (no content).
    pub meta: DocumentMeta,
    /// Folder names from the root down to the document's folder. Empty for a root document.
    pub folder_path: Vec<String>,
}

impl DocumentEntry {
    /// The folder path as one string, for example `"Novel / Part one"`. Empty at the root.
    pub fn folder_label(&self) -> String {
        self.folder_path.join(" / ")
    }
}

/// The app's library: the single owner of the [`Store`], a cached [`LibraryTree`] and the
/// source of [`LibraryEvent`]s.
///
/// Create the value with one of the constructors (which can fail), then hand it to GPUI:
/// `let library = cx.new(|_| Library::open_default()?)` does not work with `?`, so write
/// `let library = Library::open_default()?; let library = cx.new(|_| library);`.
///
/// Every method is synchronous and runs on the main thread. Reads take `&self`. Writes take
/// `&mut self` and a `Context<Library>`, emit events and call `cx.notify()`. Errors are
/// returned; a failed write changes nothing and emits nothing.
pub struct Library {
    store: Store,
    tree: Arc<LibraryTree>,
}

impl EventEmitter<LibraryEvent> for Library {}

impl Library {
    // ----- opening --------------------------------------------------------------------

    /// Opens the library in the data directory: `$THE_WRITER_DATA_DIR/library.db` if that
    /// variable is set, else `~/Library/Application Support/The Writer/library.db`
    /// (see [`writer_store::database_path`]). Creates it on first run.
    pub fn open_default() -> Result<Self> {
        Self::from_store(Store::open_default()?)
    }

    /// Opens (or creates) the library database at `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::from_store(Store::open(path)?)
    }

    /// A library that lives in memory, for tests.
    pub fn in_memory() -> Result<Self> {
        Self::from_store(Store::open_in_memory()?)
    }

    /// An in-memory library whose timestamps come from `clock` (use [`Clock::manual`]).
    pub fn in_memory_with_clock(clock: Clock) -> Result<Self> {
        Self::from_store(Store::open_in_memory_with_clock(clock)?)
    }

    /// Wraps an already opened store and loads the tree.
    pub fn from_store(store: Store) -> Result<Self> {
        let tree = Arc::new(store.library_tree()?);
        Ok(Self { store, tree })
    }

    /// Read-only access to the store, for reads this type does not wrap. There is
    /// deliberately no mutable access: writes must go through `Library` so events fire.
    pub fn store(&self) -> &Store {
        &self.store
    }

    /// The database file, or `None` for an in-memory library.
    pub fn path(&self) -> Option<&Path> {
        self.store.path()
    }

    /// The clock that stamps this library's rows.
    pub fn clock(&self) -> &Clock {
        self.store.clock()
    }

    // ----- tree -----------------------------------------------------------------------

    /// The cached tree of live folders and documents (subfolders first, then documents, each
    /// in display order). Cheap to call; the `Arc` is replaced on every change.
    pub fn tree(&self) -> Arc<LibraryTree> {
        self.tree.clone()
    }

    /// Reloads the cached tree from the store and emits [`LibraryEvent::TreeChanged`].
    /// Normal use never needs this; every write refreshes the cache itself.
    pub fn refresh(&mut self, cx: &mut Context<Self>) -> Result<()> {
        self.reload_tree()?;
        cx.emit(LibraryEvent::TreeChanged);
        cx.notify();
        Ok(())
    }

    /// Folder names from the root down to `folder`, from the cached tree. Empty for the
    /// root (`None`) and for a folder that is not live.
    pub fn folder_path(&self, folder: Option<FolderId>) -> Vec<String> {
        fn walk(nodes: &[FolderNode], id: FolderId, path: &mut Vec<String>) -> bool {
            for node in nodes {
                path.push(node.folder.name.clone());
                if node.folder.id == id || walk(&node.folders, id, path) {
                    return true;
                }
                path.pop();
            }
            false
        }
        let mut path = Vec::new();
        if let Some(id) = folder {
            walk(&self.tree.folders, id, &mut path);
        }
        path
    }

    // ----- folders --------------------------------------------------------------------

    /// One folder, live or trashed.
    pub fn folder(&self, id: FolderId) -> Result<Folder> {
        self.store.folder(id)
    }

    /// All live folders, flat.
    pub fn folders(&self) -> Result<Vec<Folder>> {
        self.store.folders()
    }

    /// Creates a folder at the end of `parent`'s subfolders (`None` is the root).
    /// Emits `FolderCreated`, `TreeChanged`.
    pub fn create_folder(
        &mut self,
        parent: Option<FolderId>,
        name: &str,
        cx: &mut Context<Self>,
    ) -> Result<Folder> {
        let folder = self.store.create_folder(parent, name)?;
        self.changed(cx, [LibraryEvent::FolderCreated(folder.id)], false, true)?;
        Ok(folder)
    }

    /// Renames a live folder. Emits `FolderRenamed`, `TreeChanged`.
    pub fn rename_folder(
        &mut self,
        id: FolderId,
        name: &str,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        self.store.rename_folder(id, name)?;
        self.changed(cx, [LibraryEvent::FolderRenamed(id)], false, true)
    }

    /// Moves a folder under `parent` (`None` is the root). With `index: None` it goes to
    /// the end; with `Some(i)` it lands at 0-based position `i` among the live subfolders
    /// (clamped). Moving a folder into itself or a descendant is an error
    /// ([`writer_store::StoreError::FolderCycle`]). Emits `FolderMoved`, `TreeChanged`.
    pub fn move_folder(
        &mut self,
        id: FolderId,
        parent: Option<FolderId>,
        index: Option<usize>,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        self.store.move_folder(id, parent)?;
        let reordered = match index {
            Some(index) => self.store.reorder_folder(id, index),
            None => Ok(()),
        };
        // The move itself happened even if the reorder failed: report it either way.
        let refreshed = self.changed(cx, [LibraryEvent::FolderMoved(id)], false, true);
        reordered.and(refreshed)
    }

    /// Puts a folder at 0-based `index` among its live sibling folders (clamped).
    /// Emits `FolderMoved`, `TreeChanged`.
    pub fn reorder_folder(
        &mut self,
        id: FolderId,
        index: usize,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        self.store.reorder_folder(id, index)?;
        self.changed(cx, [LibraryEvent::FolderMoved(id)], false, true)
    }

    /// Moves a folder and its whole subtree to the trash.
    /// Emits `FolderTrashed` (with the documents that went with it), `TrashChanged`,
    /// `TreeChanged`.
    pub fn trash_folder(&mut self, id: FolderId, cx: &mut Context<Self>) -> Result<()> {
        fn collect(node: &FolderNode, out: &mut Vec<DocumentId>) {
            out.extend(node.documents.iter().map(|d| d.id));
            for child in &node.folders {
                collect(child, out);
            }
        }
        // Fresh tree, so the list is right even if the cache were ever stale.
        let before = self.store.library_tree()?;
        let mut documents = Vec::new();
        if let Some(node) = before.find_folder(id) {
            collect(node, &mut documents);
        }
        self.store.trash_folder(id)?;
        self.changed(
            cx,
            [LibraryEvent::FolderTrashed { id, documents }],
            true,
            true,
        )
    }

    // ----- documents ------------------------------------------------------------------

    /// Creates a document at the end of `folder` (`None` is the root). The caller supplies
    /// the content JSON and the fields derived from it. Emits `DocumentCreated`,
    /// `TreeChanged`.
    pub fn create_document(
        &mut self,
        folder: Option<FolderId>,
        content: &DocumentContent<'_>,
        cx: &mut Context<Self>,
    ) -> Result<DocumentMeta> {
        let meta = self.store.create_document(folder, content)?;
        self.changed(cx, [LibraryEvent::DocumentCreated(meta.id)], false, true)?;
        Ok(meta)
    }

    /// Loads a document with its content and view state. Works on trashed documents too.
    pub fn document(&self, id: DocumentId) -> Result<Document> {
        self.store.document(id)
    }

    /// A document's metadata without its content. Works on trashed documents too.
    pub fn document_meta(&self, id: DocumentId) -> Result<DocumentMeta> {
        self.store.document_meta(id)
    }

    /// True only for a live (not trashed, not purged) document.
    pub fn document_exists(&self, id: DocumentId) -> Result<bool> {
        self.store.document_exists(id)
    }

    /// Saves a document's content together with its title, plain text and counts. Also
    /// works on a trashed document, so an autosave racing a delete never loses text.
    /// Emits `DocumentSaved`, and `TreeChanged` when the title changed on a live document.
    pub fn save_document(
        &mut self,
        id: DocumentId,
        content: &DocumentContent<'_>,
        cx: &mut Context<Self>,
    ) -> Result<DocumentMeta> {
        let old_title = self.store.document_meta(id)?.title;
        let meta = self.store.save_document(id, content)?;
        let title_changed = old_title != meta.title;
        let tree_changed = title_changed && meta.trashed_at.is_none();
        self.changed(
            cx,
            [LibraryEvent::DocumentSaved { id, title_changed }],
            false,
            tree_changed,
        )?;
        Ok(meta)
    }

    /// Renames a live document. If the app derives titles from content, its next save
    /// overwrites this title. Emits `DocumentRenamed`, `TreeChanged`.
    pub fn rename_document(
        &mut self,
        id: DocumentId,
        title: &str,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        self.store.rename_document(id, title)?;
        self.changed(cx, [LibraryEvent::DocumentRenamed(id)], false, true)
    }

    /// Moves a document into `folder` (`None` is the root). With `index: None` it goes to
    /// the end; with `Some(i)` it lands at 0-based position `i` among the folder's live
    /// documents (clamped). Emits `DocumentMoved`, `TreeChanged`.
    pub fn move_document(
        &mut self,
        id: DocumentId,
        folder: Option<FolderId>,
        index: Option<usize>,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        self.store.move_document(id, folder)?;
        let reordered = match index {
            Some(index) => self.store.reorder_document(id, index),
            None => Ok(()),
        };
        let refreshed = self.changed(cx, [LibraryEvent::DocumentMoved(id)], false, true);
        reordered.and(refreshed)
    }

    /// Puts a document at 0-based `index` among the live documents of its folder (clamped).
    /// Emits `DocumentMoved`, `TreeChanged`.
    pub fn reorder_document(
        &mut self,
        id: DocumentId,
        index: usize,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        self.store.reorder_document(id, index)?;
        self.changed(cx, [LibraryEvent::DocumentMoved(id)], false, true)
    }

    /// Copies a document into the same folder, directly after the original, titled
    /// "`<title>` copy". Emits `DocumentCreated` (the copy), `TreeChanged`.
    pub fn duplicate_document(
        &mut self,
        id: DocumentId,
        cx: &mut Context<Self>,
    ) -> Result<DocumentMeta> {
        let meta = self.store.duplicate_document(id)?;
        self.changed(cx, [LibraryEvent::DocumentCreated(meta.id)], false, true)?;
        Ok(meta)
    }

    /// Moves a document to the trash. Emits `DocumentTrashed`, `TrashChanged`,
    /// `TreeChanged`.
    pub fn trash_document(&mut self, id: DocumentId, cx: &mut Context<Self>) -> Result<()> {
        self.store.trash_document(id)?;
        self.changed(cx, [LibraryEvent::DocumentTrashed(id)], true, true)
    }

    /// Live documents directly in `folder`, in display order.
    pub fn documents_in_folder(&self, folder: Option<FolderId>) -> Result<Vec<DocumentMeta>> {
        self.store.documents_in_folder(folder)
    }

    /// The `limit` most recently edited live documents, newest first.
    pub fn recent_documents(&self, limit: usize) -> Result<Vec<DocumentMeta>> {
        self.store.recent_documents(limit)
    }

    /// Every live document with its folder path, most recently edited first. No content is
    /// loaded; this is the listing for quick open.
    pub fn all_documents(&self) -> Result<Vec<DocumentEntry>> {
        fn index(
            nodes: &[FolderNode],
            path: &mut Vec<String>,
            out: &mut HashMap<FolderId, Vec<String>>,
        ) {
            for node in nodes {
                path.push(node.folder.name.clone());
                out.insert(node.folder.id, path.clone());
                index(&node.folders, path, out);
                path.pop();
            }
        }
        let mut paths = HashMap::new();
        index(&self.tree.folders, &mut Vec::new(), &mut paths);
        Ok(self
            .store
            .documents()?
            .into_iter()
            .map(|meta| DocumentEntry {
                folder_path: meta
                    .folder_id
                    .and_then(|id| paths.get(&id).cloned())
                    .unwrap_or_default(),
                meta,
            })
            .collect())
    }

    // ----- view state -----------------------------------------------------------------

    /// Stores a document's opaque view-state JSON (caret, scroll, toggles). Does not touch
    /// `updated_at` and emits nothing.
    pub fn save_view_state(&mut self, id: DocumentId, view_state: &str) -> Result<()> {
        self.store.save_view_state(id, view_state)
    }

    /// A document's stored view-state JSON, if any.
    pub fn view_state(&self, id: DocumentId) -> Result<Option<String>> {
        self.store.view_state(id)
    }

    // ----- trash ----------------------------------------------------------------------

    /// The top-level entries of the trash, most recently trashed first.
    pub fn trash(&self) -> Result<Vec<TrashEntry>> {
        self.store.trash()
    }

    /// Restores a trashed document to its original folder (or the root if that folder is
    /// gone). Emits `DocumentRestored`, `TrashChanged`, `TreeChanged`.
    pub fn restore_document(
        &mut self,
        id: DocumentId,
        cx: &mut Context<Self>,
    ) -> Result<DocumentMeta> {
        let meta = self.store.restore_document(id)?;
        self.changed(cx, [LibraryEvent::DocumentRestored(id)], true, true)?;
        Ok(meta)
    }

    /// Restores a trashed folder with everything that was trashed with it.
    /// Emits `FolderRestored`, `TrashChanged`, `TreeChanged`.
    pub fn restore_folder(&mut self, id: FolderId, cx: &mut Context<Self>) -> Result<Folder> {
        let folder = self.store.restore_folder(id)?;
        self.changed(cx, [LibraryEvent::FolderRestored(id)], true, true)?;
        Ok(folder)
    }

    /// Deletes a trashed document for good, with its snapshots.
    /// Emits `DocumentPurged`, `TrashChanged`.
    pub fn purge_document(&mut self, id: DocumentId, cx: &mut Context<Self>) -> Result<()> {
        self.store.purge_document(id)?;
        self.changed(cx, [LibraryEvent::DocumentPurged(id)], true, false)
    }

    /// Deletes a trashed folder for good, with everything that was trashed with it.
    /// Emits `FolderPurged`, `TrashChanged`.
    pub fn purge_folder(&mut self, id: FolderId, cx: &mut Context<Self>) -> Result<()> {
        self.store.purge_folder(id)?;
        self.changed(cx, [LibraryEvent::FolderPurged(id)], true, false)
    }

    /// Deletes everything in the trash for good. Emits `TrashChanged`.
    pub fn empty_trash(&mut self, cx: &mut Context<Self>) -> Result<EmptiedTrash> {
        let emptied = self.store.empty_trash()?;
        self.changed(cx, [], true, false)?;
        Ok(emptied)
    }

    // ----- snapshots ------------------------------------------------------------------

    /// A document's version snapshots, newest first, without content.
    pub fn snapshots(&self, document: DocumentId) -> Result<Vec<SnapshotMeta>> {
        self.store.snapshots(document)
    }

    /// One snapshot with its content. To restore it, derive title, plain text and counts
    /// from the content and call [`Library::save_document`].
    pub fn snapshot(&self, id: SnapshotId) -> Result<Snapshot> {
        self.store.snapshot(id)
    }

    /// Snapshots the stored content of a document (save first). Emits `SnapshotsChanged`.
    pub fn create_snapshot(
        &mut self,
        document: DocumentId,
        cx: &mut Context<Self>,
    ) -> Result<SnapshotMeta> {
        let meta = self.store.create_snapshot(document)?;
        self.announce(cx, LibraryEvent::SnapshotsChanged(document))?;
        Ok(meta)
    }

    /// Takes a snapshot if the document has none, or if the newest one is at least
    /// `min_age` old and differs from the stored content; then prunes old snapshots with
    /// the store's policy. Returns the new snapshot, if one was taken, and emits
    /// `SnapshotsChanged` in that case only. Call it after an autosave.
    pub fn snapshot_if_stale(
        &mut self,
        document: DocumentId,
        min_age: Duration,
        cx: &mut Context<Self>,
    ) -> Result<Option<SnapshotMeta>> {
        let taken = self.store.snapshot_if_stale(document, min_age)?;
        if taken.is_some() {
            self.announce(cx, LibraryEvent::SnapshotsChanged(document))?;
        }
        Ok(taken)
    }

    /// Deletes one snapshot. Emits `SnapshotsChanged` for its document.
    pub fn delete_snapshot(&mut self, id: SnapshotId, cx: &mut Context<Self>) -> Result<()> {
        let document = self.store.snapshot(id)?.meta.document_id;
        self.store.delete_snapshot(id)?;
        self.announce(cx, LibraryEvent::SnapshotsChanged(document))
    }

    // ----- settings -------------------------------------------------------------------

    /// Reads a setting. `Ok(None)` when the key is missing; an error when the stored value
    /// does not decode as `T`.
    pub fn setting<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        self.store.setting(key)
    }

    /// Reads a setting, falling back to `default` when the key is missing, the stored value
    /// does not decode as `T`, or the read fails. Never errors, so it is safe in `render`.
    pub fn setting_or<T: DeserializeOwned + Clone>(&self, key: &str, default: T) -> T {
        self.store
            .setting_or(key, default.clone())
            .unwrap_or(default)
    }

    /// Writes a setting (any serde value) and emits `SettingChanged` with the key.
    pub fn set_setting<T: Serialize + ?Sized>(
        &mut self,
        key: &str,
        value: &T,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        self.store.set_setting(key, value)?;
        let key = SharedString::from(key.to_string());
        self.announce(cx, LibraryEvent::SettingChanged { key })
    }

    /// Removes a setting. Returns whether it existed; emits `SettingChanged` if it did.
    pub fn remove_setting(&mut self, key: &str, cx: &mut Context<Self>) -> Result<bool> {
        let existed = self.store.remove_setting(key)?;
        if existed {
            let key = SharedString::from(key.to_string());
            self.announce(cx, LibraryEvent::SettingChanged { key })?;
        }
        Ok(existed)
    }

    /// Every stored setting key.
    pub fn setting_keys(&self) -> Result<Vec<String>> {
        self.store.setting_keys()
    }

    // ----- search ---------------------------------------------------------------------

    /// Full-text search over titles and text of live documents, best match first. Any
    /// input is safe; the last word is a prefix match. An empty query gives no hits.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>> {
        self.store.search(query, limit)
    }

    // ----- internals ------------------------------------------------------------------

    fn reload_tree(&mut self) -> Result<()> {
        self.tree = Arc::new(self.store.library_tree()?);
        Ok(())
    }

    /// Emits one event for a change that does not touch the tree or the trash.
    fn announce(&mut self, cx: &mut Context<Self>, event: LibraryEvent) -> Result<()> {
        cx.emit(event);
        cx.notify();
        Ok(())
    }

    /// Called after a successful store write: refreshes the cache, emits `events`, then
    /// `TrashChanged` / `TreeChanged` as asked, and notifies observers. The events are
    /// emitted even if the cache refresh fails (the write did happen); that failure is
    /// returned.
    fn changed<const N: usize>(
        &mut self,
        cx: &mut Context<Self>,
        events: [LibraryEvent; N],
        trash_changed: bool,
        tree_changed: bool,
    ) -> Result<()> {
        // Always refresh: even a plain save changes counts and timestamps in the tree.
        let refreshed = self.reload_tree();
        for event in events {
            cx.emit(event);
        }
        if trash_changed {
            cx.emit(LibraryEvent::TrashChanged);
        }
        if tree_changed {
            cx.emit(LibraryEvent::TreeChanged);
        }
        cx.notify();
        refreshed
    }
}
