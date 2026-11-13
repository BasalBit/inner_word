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
    FolderNode, ImageId, ImageInfo, LibraryTree, NewImage, Result, SearchHit, Snapshot, SnapshotId,
    SnapshotMeta, Store, TrashEntry,
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

/// One thing that was changed in the store by something other than a method of
/// [`Library`]: see [`Library::change_outside`].
///
/// The app carries out the requests of agents (the `writer` command line tool, while the
/// app is open) with code that works on a [`Store`] and knows nothing of this crate. That
/// code says what it changed in these terms, and the library turns each into the
/// [`LibraryEvent`] its own method would have emitted, so every view follows as if the
/// user had made the change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OutsideChange {
    /// A document was created ([`LibraryEvent::DocumentCreated`]).
    DocumentCreated(DocumentId),
    /// The stored content of a document was written: its text, its comments, its own
    /// settings, and with them its counts, its edit time and perhaps its title
    /// ([`LibraryEvent::DocumentSaved`], with `title_changed` worked out against the title
    /// the tree had).
    DocumentSaved(DocumentId),
    /// A document was given a name ([`LibraryEvent::DocumentRenamed`]).
    DocumentRenamed(DocumentId),
    /// A document was moved to another folder or to another place in its folder
    /// ([`LibraryEvent::DocumentMoved`]).
    DocumentMoved(DocumentId),
    /// A document was moved to the trash ([`LibraryEvent::DocumentTrashed`]).
    DocumentTrashed(DocumentId),
    /// A document was restored from the trash ([`LibraryEvent::DocumentRestored`]).
    DocumentRestored(DocumentId),
    /// A document was deleted for good ([`LibraryEvent::DocumentPurged`]). When it was a
    /// live document until then, [`LibraryEvent::DocumentTrashed`] is emitted first: a
    /// view that shows it closes it the way it does for the trash.
    DocumentPurged(DocumentId),
    /// A folder was created ([`LibraryEvent::FolderCreated`]).
    FolderCreated(FolderId),
    /// A folder was renamed ([`LibraryEvent::FolderRenamed`]).
    FolderRenamed(FolderId),
    /// A folder was moved ([`LibraryEvent::FolderMoved`]).
    FolderMoved(FolderId),
    /// A folder went to the trash with everything in it ([`LibraryEvent::FolderTrashed`];
    /// the documents that went with it are read from the tree as it was).
    FolderTrashed(FolderId),
    /// A folder was restored from the trash ([`LibraryEvent::FolderRestored`]).
    FolderRestored(FolderId),
    /// A folder was deleted for good ([`LibraryEvent::FolderPurged`]). When it was live
    /// until then, [`LibraryEvent::FolderTrashed`] is emitted first.
    FolderPurged(FolderId),
    /// The trash was emptied ([`LibraryEvent::TrashChanged`] alone, as
    /// [`Library::empty_trash`] emits it).
    TrashEmptied,
    /// A version snapshot of this document was created or deleted
    /// ([`LibraryEvent::SnapshotsChanged`]).
    SnapshotsChanged(DocumentId),
    /// A setting was written or removed ([`LibraryEvent::SettingChanged`]).
    SettingChanged(String),
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
    /// Normal use never needs this; every write refreshes the cache itself. After a
    /// document save that went through another connection, use
    /// [`document_saved_elsewhere`](Self::document_saved_elsewhere), which is far cheaper.
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

    /// Copies a folder with everything live inside it (subfolders and documents, at any
    /// depth) next to the original, named "`<name>` copy". One store transaction and one
    /// tree reload, however much is in the folder. Emits `FolderCreated` (the copy),
    /// `TreeChanged`; the copied documents are in the tree but get no events of their own.
    pub fn duplicate_folder(&mut self, id: FolderId, cx: &mut Context<Self>) -> Result<Folder> {
        let folder = self.store.duplicate_folder(id)?;
        self.changed(cx, [LibraryEvent::FolderCreated(folder.id)], false, true)?;
        Ok(folder)
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
        // The cache is the tree as of the last write, and every write goes through here.
        let mut documents = Vec::new();
        if let Some(node) = self.tree.find_folder(id) {
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
    ///
    /// This is the call behind autosave, so it is kept cheap: the tree is not reloaded. The
    /// document's entry in the cached tree (title, counts, `updated_at`) is replaced in
    /// place.
    pub fn save_document(
        &mut self,
        id: DocumentId,
        content: &DocumentContent<'_>,
        cx: &mut Context<Self>,
    ) -> Result<DocumentMeta> {
        // The title before the save: from the cache for a live document, from the store
        // for one that is in the trash (or missing, which is the error to return).
        let same_title = match self.tree.find_document(id) {
            Some(cached) => cached.title == content.title,
            None => self.store.document_meta(id)?.title == content.title,
        };
        let meta = self.store.save_document(id, content)?;
        let title_changed = !same_title;
        let live = meta.trashed_at.is_none();
        // A trashed document is not in the tree. For a live one, a reload is only needed
        // if the cache does not hold the document where the store says it is.
        let refreshed = if !live || Arc::make_mut(&mut self.tree).update_document(&meta) {
            Ok(())
        } else {
            self.reload_tree()
        };
        cx.emit(LibraryEvent::DocumentSaved { id, title_changed });
        if title_changed && live {
            cx.emit(LibraryEvent::TreeChanged);
        }
        cx.notify();
        refreshed?;
        Ok(meta)
    }

    /// Takes note of a document that was saved **through another connection** to the same
    /// library file (the app's background autosave writes that way, so the commit does not
    /// hold up the main thread). `meta` is what that save returned.
    ///
    /// Does for the cache and the subscribers what [`save_document`](Self::save_document)
    /// does, without touching the database when the document is where the cache has it:
    /// the cached entry is replaced in place, `DocumentSaved` is emitted, and `TreeChanged`
    /// when the title is not the one the cache had. Use it instead of
    /// [`refresh`](Self::refresh) after such a save: `refresh` reloads the whole tree and
    /// tells every view that the tree changed.
    ///
    /// For a trashed document (it is not in the tree) only `DocumentSaved` is emitted, with
    /// `title_changed: false`: the cache has no earlier title to compare with.
    pub fn document_saved_elsewhere(
        &mut self,
        meta: &DocumentMeta,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        let live = meta.trashed_at.is_none();
        let before = self.tree.find_document(meta.id).map(|cached| {
            // `meta` comes from another thread and may be older than what the cache has
            // (the document was renamed here while that save was on its way).
            let newer = meta.updated_at > cached.updated_at
                || (meta.updated_at == cached.updated_at && meta.title == cached.title);
            (cached.title.clone(), newer)
        });
        let in_place = matches!(before, Some((_, true)))
            && Arc::make_mut(&mut self.tree).update_document(meta);
        let refreshed = if !live || in_place {
            Ok(())
        } else {
            // Not where the cache has it (moved or restored meanwhile), or not newer than
            // it: read it all again.
            self.reload_tree()
        };
        // Against what the cache holds now, which is what the views will show.
        let title_changed = live
            && self
                .tree
                .find_document(meta.id)
                .is_some_and(|now| before.is_none_or(|(title, _)| title != now.title));
        cx.emit(LibraryEvent::DocumentSaved {
            id: meta.id,
            title_changed,
        });
        if title_changed {
            cx.emit(LibraryEvent::TreeChanged);
        }
        cx.notify();
        refreshed
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

    // ----- images ---------------------------------------------------------------------

    /// Stores the bytes of an image once, under the hash of the bytes, and answers what
    /// the library knows about them ([`Store::put_image`]). Committed before it returns.
    ///
    /// No event and no notification: nothing anybody shows changes when bytes are
    /// stored. **Store the bytes before a block that names them enters a document**, so
    /// that a save never names bytes that are not there.
    ///
    /// The reads (what is known about an image, its bytes) go through
    /// [`store`](Self::store). This runs on the main thread like every method here; for
    /// a library on disk, whoever has many bytes to hash and write does it in the
    /// background through a [`writer_store::ImageFile`] of its own.
    pub fn put_image(&mut self, image: &NewImage<'_>) -> Result<ImageInfo> {
        self.store.put_image(image)
    }

    /// For the id of an image that should be there (it came with a pasted fragment, or
    /// in Markdown): the image counts as used from now ([`Store::touch_image`]). `None`
    /// when the library does not have it. No event, as for
    /// [`put_image`](Self::put_image).
    pub fn touch_image(&mut self, id: &ImageId) -> Result<Option<ImageInfo>> {
        self.store.touch_image(id)
    }

    // ----- trash ----------------------------------------------------------------------

    /// The top-level entries of the trash, most recently trashed first.
    pub fn trash(&self) -> Result<Vec<TrashEntry>> {
        self.store.trash()
    }

    /// Restores a trashed document to its original folder, at the place it had among the
    /// documents there (or to the end of the root if that folder is gone).
    /// Emits `DocumentRestored`, `TrashChanged`, `TreeChanged`.
    pub fn restore_document(
        &mut self,
        id: DocumentId,
        cx: &mut Context<Self>,
    ) -> Result<DocumentMeta> {
        let meta = self.store.restore_document(id)?;
        self.changed(cx, [LibraryEvent::DocumentRestored(id)], true, true)?;
        Ok(meta)
    }

    /// Restores a trashed folder with everything that was trashed with it, at the place it
    /// had among its sibling folders (or to the end of the root if its parent is gone).
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

    // ----- changes from outside -------------------------------------------------------

    /// Lets `change` write to the store directly, and then does for the cache and the
    /// subscribers what the methods of this type do: `change` returns what it changed
    /// (besides its own result, which is handed back), the tree is read again once, and
    /// each [`OutsideChange`] becomes the event its own method would have emitted, in
    /// the order given, followed by one [`LibraryEvent::TrashChanged`] and one
    /// [`LibraryEvent::TreeChanged`] when the trash listing or the tree changed.
    ///
    /// This is the one way around "every write goes through a method of `Library`". It
    /// is for code that works on a [`Store`] and cannot know this crate: the app
    /// carrying out the request of an agent with `writer-agent`. Nothing is emitted for
    /// an empty list, and the store is not read again then.
    ///
    /// The second value is `Err` when the tree could not be read again (the events were
    /// emitted all the same: the write did happen).
    pub fn change_outside<R>(
        &mut self,
        change: impl FnOnce(&mut Store) -> (R, Vec<OutsideChange>),
        cx: &mut Context<Self>,
    ) -> (R, Result<()>) {
        let (result, changes) = change(&mut self.store);
        if changes.is_empty() {
            return (result, Ok(()));
        }
        // What only the tree as it was can tell.
        fn collect(node: &FolderNode, out: &mut Vec<DocumentId>) {
            out.extend(node.documents.iter().map(|d| d.id));
            for child in &node.folders {
                collect(child, out);
            }
        }
        let before = self.tree.clone();
        let refreshed = self.reload_tree();
        let mut trash_changed = false;
        let mut tree_changed = false;
        for change in changes {
            match change {
                OutsideChange::DocumentCreated(id) => {
                    cx.emit(LibraryEvent::DocumentCreated(id));
                    tree_changed = true;
                }
                OutsideChange::DocumentSaved(id) => {
                    let was = before.find_document(id).map(|cached| cached.title.as_str());
                    let now = self
                        .tree
                        .find_document(id)
                        .map(|cached| cached.title.as_str());
                    // A document in the trash is not in the tree: its row there may
                    // show another title now.
                    trash_changed |= now.is_none();
                    let title_changed = now.is_some() && was != now;
                    cx.emit(LibraryEvent::DocumentSaved { id, title_changed });
                    tree_changed |= title_changed;
                }
                OutsideChange::DocumentRenamed(id) => {
                    trash_changed |= self.tree.find_document(id).is_none();
                    cx.emit(LibraryEvent::DocumentRenamed(id));
                    tree_changed = true;
                }
                OutsideChange::DocumentMoved(id) => {
                    cx.emit(LibraryEvent::DocumentMoved(id));
                    tree_changed = true;
                }
                OutsideChange::DocumentTrashed(id) => {
                    cx.emit(LibraryEvent::DocumentTrashed(id));
                    trash_changed = true;
                    tree_changed = true;
                }
                OutsideChange::DocumentRestored(id) => {
                    cx.emit(LibraryEvent::DocumentRestored(id));
                    trash_changed = true;
                    tree_changed = true;
                }
                OutsideChange::DocumentPurged(id) => {
                    if before.find_document(id).is_some() {
                        cx.emit(LibraryEvent::DocumentTrashed(id));
                        tree_changed = true;
                    }
                    cx.emit(LibraryEvent::DocumentPurged(id));
                    trash_changed = true;
                }
                OutsideChange::FolderCreated(id) => {
                    cx.emit(LibraryEvent::FolderCreated(id));
                    tree_changed = true;
                }
                OutsideChange::FolderRenamed(id) => {
                    trash_changed |= self.tree.find_folder(id).is_none();
                    cx.emit(LibraryEvent::FolderRenamed(id));
                    tree_changed = true;
                }
                OutsideChange::FolderMoved(id) => {
                    cx.emit(LibraryEvent::FolderMoved(id));
                    tree_changed = true;
                }
                OutsideChange::FolderTrashed(id) => {
                    let mut documents = Vec::new();
                    if let Some(node) = before.find_folder(id) {
                        collect(node, &mut documents);
                    }
                    cx.emit(LibraryEvent::FolderTrashed { id, documents });
                    trash_changed = true;
                    tree_changed = true;
                }
                OutsideChange::FolderRestored(id) => {
                    cx.emit(LibraryEvent::FolderRestored(id));
                    trash_changed = true;
                    tree_changed = true;
                }
                OutsideChange::FolderPurged(id) => {
                    if let Some(node) = before.find_folder(id) {
                        let mut documents = Vec::new();
                        collect(node, &mut documents);
                        cx.emit(LibraryEvent::FolderTrashed { id, documents });
                        tree_changed = true;
                    }
                    cx.emit(LibraryEvent::FolderPurged(id));
                    trash_changed = true;
                }
                OutsideChange::TrashEmptied => trash_changed = true,
                OutsideChange::SnapshotsChanged(id) => {
                    cx.emit(LibraryEvent::SnapshotsChanged(id));
                }
                OutsideChange::SettingChanged(key) => {
                    cx.emit(LibraryEvent::SettingChanged { key: key.into() });
                }
            }
        }
        if trash_changed {
            cx.emit(LibraryEvent::TrashChanged);
        }
        if tree_changed {
            cx.emit(LibraryEvent::TreeChanged);
        }
        cx.notify();
        (result, refreshed)
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
        // Everything that comes through here can change the shape of the tree. (A content
        // save does not, and updates its entry in place: see `save_document`.)
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
