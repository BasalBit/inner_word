//! Names, places and the trash: documents and folders as items of the library.

use innerword_store::{
    DocumentId, DocumentMeta, Folder, FolderId, FolderNode, Store, StoreError, TrashEntry,
};

use crate::content::{self, CUSTOM_TITLE_PREFIX};
use crate::error::{Error, ItemKind, Result, quoted_excerpt};
use crate::library::{Library, Session};
use crate::live::Change;
use crate::resolve::{Found, Index, join_path, parse_path, title_of};
use crate::types::{
    CreateFolderRequest, DeleteDocumentRequest, DeleteFolderRequest, DocumentChanged,
    DocumentSummary, DuplicateDocumentRequest, EmptyTrashRequest, FolderChanged, FolderCreated,
    FolderSummary, MoveDocumentRequest, MoveFolderRequest, Placement, RenameDocumentRequest,
    RenameFolderRequest, RestoreDocumentRequest, RestoreFolderRequest, Target, Time,
    TrashDocumentRequest, TrashEmptied, TrashFolderRequest, TrashItem, TrashList,
};

/// Walks `path` down from `base` (or from the top of the library) through the folders
/// that are there. Returns the deepest folder that exists and the names that are missing
/// below it. A name is taken as it is first, then without regard to case.
pub(crate) fn plan_folders(
    index: &Index,
    base: Option<&Folder>,
    path: &str,
) -> Result<(Option<Folder>, Vec<String>)> {
    let (_, names) = parse_path(path);
    if names.is_empty() {
        return Err(Error::invalid(
            "no folder was named: give its path, the names separated by \"/\"",
        ));
    }
    let mut current: Option<Folder> = base.cloned();
    for (position, name) in names.iter().enumerate() {
        let children = index.child_folders(current.as_ref().map(|folder| folder.id));
        let found = children
            .iter()
            .find(|node| node.folder.name.trim() == name)
            .or_else(|| {
                children
                    .iter()
                    .find(|node| node.folder.name.trim().to_lowercase() == name.to_lowercase())
            });
        match found {
            Some(node) => current = Some(node.folder.clone()),
            None => return Ok((current, names[position..].to_vec())),
        }
    }
    Ok((current, Vec::new()))
}

/// The place a [`Placement`] means among `siblings` (the ids of the items that are there,
/// in order, without the item that is placed): an index from 0 to their number.
pub(crate) fn place_among(
    siblings: &[i64],
    placement: &Placement,
    resolve: impl Fn(&Target) -> Result<i64>,
) -> Result<usize> {
    let beside = |target: &Target| -> Result<usize> {
        let id = resolve(target)?;
        siblings
            .iter()
            .position(|&other| other == id)
            .ok_or_else(|| {
                Error::invalid(format!(
                    "{target} is not in the folder the item goes to, so nothing can be placed \
                 beside it"
                ))
            })
    };
    Ok(match placement {
        Placement::First => 0,
        Placement::Last => siblings.len(),
        Placement::Index(index) => (*index).min(siblings.len()),
        Placement::Before(target) => beside(target)?,
        Placement::After(target) => beside(target)? + 1,
    })
}

/// How many folders and documents a live folder holds, at every depth.
pub(crate) fn contents(node: &FolderNode) -> (u64, u64, u64) {
    let mut total = (0, node.documents.len() as u64, 0);
    total.2 = node
        .documents
        .iter()
        .map(|document| document.word_count)
        .sum();
    for child in &node.folders {
        let (folders, documents, words) = contents(child);
        total.0 += 1 + folders;
        total.1 += documents;
        total.2 += words;
    }
    total
}

/// A folder that would be created by a dry run, as results show it.
fn planned_folder(path: &[String], parent: Option<FolderId>, now: Time) -> FolderSummary {
    FolderSummary {
        id: FolderId(0),
        name: path.last().cloned().unwrap_or_default(),
        path: join_path(path),
        parent_id: parent,
        created_at: now,
        updated_at: now,
        trashed_at: None,
    }
}

fn confirmation(what: String) -> Error {
    Error::ConfirmationRequired { what }
}

fn folder_error(error: StoreError) -> Error {
    match error {
        StoreError::FolderCycle => Error::invalid(
            "a folder cannot be moved into itself or into one of the folders inside it",
        ),
        StoreError::EmptyName => Error::invalid("the name of a folder cannot be empty"),
        other => other.into(),
    }
}

/// Removes the "somebody chose this title" flags of documents that are gone.
fn forget_deleted_documents(store: &mut Store) -> Result<()> {
    for key in store.setting_keys()? {
        let Some(id) = key
            .strip_prefix(CUSTOM_TITLE_PREFIX)
            .and_then(|id| id.parse::<i64>().ok())
        else {
            continue;
        };
        if matches!(
            store.document_meta(DocumentId(id)),
            Err(StoreError::NotFound { .. })
        ) {
            store.remove_setting(&key)?;
        }
    }
    store.remove_stale_snapshot_labels()?;
    Ok(())
}

impl Library {
    /// What is in the trash, the entry that went there last first.
    pub fn trash(&self) -> Result<TrashList> {
        let session = self.reader()?;
        let index = Index::load(&session.store)?;
        let entries = session
            .store
            .trash()?
            .iter()
            .map(|entry| match entry {
                TrashEntry::Document(meta) => TrashItem::Document {
                    document: index.document_summary(&session.store, meta),
                },
                TrashEntry::Folder {
                    folder,
                    folder_count,
                    document_count,
                } => TrashItem::Folder {
                    folder: index.folder_summary(&session.store, folder),
                    folders: *folder_count,
                    documents: *document_count,
                },
            })
            .collect();
        Ok(TrashList { entries })
    }
}

/// Renames a document, or gives its title back to its text. See
/// [`RenameDocumentRequest`].
pub(crate) fn rename_document(
    session: &mut Session<'_>,
    request: &RenameDocumentRequest,
) -> Result<DocumentChanged> {
    let index = Index::load(session.store)?;
    let meta = index.live_document(session.store, &request.document)?;
    match meta.kind {
        innerword_store::DocumentKind::Text => {}
        innerword_store::DocumentKind::Page => {
            return crate::pages::rename(session, &index, &meta, request);
        }
        innerword_store::DocumentKind::Board => {
            return crate::boards::rename(session, &index, &meta, request);
        }
    }
    let loaded = content::load(session.store, &meta)?;
    let title = if request.follow_text {
        content::derived_title(&loaded.document)
    } else {
        let title = request
            .title
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if title.is_empty() {
            return Err(Error::invalid("the new title is empty"));
        }
        title
    };
    let chosen = !request.follow_text;
    let changed = title != meta.title || chosen != loaded.custom_title;
    let before = index.document_summary(session.store, &meta);
    let mut after = DocumentMeta {
        title: title.clone(),
        ..meta.clone()
    };
    if changed && !session.dry_run {
        let store = &mut session.store;
        if title != meta.title {
            store.rename_document(meta.id, &title)?;
        }
        after = store.document_meta(meta.id)?;
        content::set_title_flag(store, &after, chosen)?;
        session.note(Change::DocumentRenamed { id: meta.id });
    }
    let result = DocumentChanged {
        document: index.document_summary(session.store, &after),
        before: Some(before),
        created_folders: Vec::new(),
        written: session.written(changed),
    };
    Ok(result)
}

/// Moves a document to another folder, to another place in its folder, or both. See
/// [`MoveDocumentRequest`].
pub(crate) fn move_document(
    session: &mut Session<'_>,
    request: &MoveDocumentRequest,
) -> Result<DocumentChanged> {
    if request.folder.is_none() && request.placement.is_none() {
        return Err(Error::invalid(
            "nothing to do: name the folder to move it to, a place in its folder, or both",
        ));
    }
    let index = Index::load(session.store)?;
    let store = &session.store;
    let meta = index.live_document(store, &request.document)?;
    let (parent, missing): (Option<Folder>, Vec<String>) = match &request.folder {
        None => (
            match meta.folder_id {
                Some(id) => Some(store.folder(id)?),
                None => None,
            },
            Vec::new(),
        ),
        Some(target) => match index.live_folder_or_root(store, target) {
            Ok(folder) => (folder, Vec::new()),
            Err(Error::NotFound { .. }) if request.create_folders => match target {
                Target::Path(path) => plan_folders(&index, None, path)?,
                _ => return Err(index.live_folder_or_root(store, target).unwrap_err()),
            },
            Err(error) => return Err(error),
        },
    };
    let before = index.document_summary(store, &meta);
    let destination = parent.as_ref().map(|folder| folder.id);
    let same_folder = missing.is_empty() && destination == meta.folder_id;
    // The documents it will stand among, without itself.
    let siblings: Vec<i64> = if missing.is_empty() {
        index
            .child_documents(destination)
            .iter()
            .map(|other| other.id.0)
            .filter(|&id| id != meta.id.0)
            .collect()
    } else {
        Vec::new()
    };
    let was = index
        .child_documents(meta.folder_id)
        .iter()
        .position(|other| other.id == meta.id);
    let place = match &request.placement {
        Some(placement) => Some(place_among(&siblings, placement, |target| {
            Ok(index.live_document(store, target)?.id.0)
        })?),
        None => None,
    };
    let changed = !same_folder || place.is_some_and(|place| Some(place) != was);

    if session.dry_run || !changed {
        let mut folder_path = index.folder_segments(store, destination);
        let now = Time(session.clock.now_ms());
        let mut created_folders = Vec::new();
        let mut above = destination;
        for name in &missing {
            folder_path.push(name.clone());
            created_folders.push(planned_folder(&folder_path, above, now));
            above = Some(FolderId(0));
        }
        let mut path = folder_path.clone();
        path.push(title_of(&meta).to_string());
        let document = DocumentSummary {
            path: join_path(&path),
            folder_id: above,
            folder: join_path(&folder_path),
            ..before.clone()
        };
        return Ok(DocumentChanged {
            document,
            before: Some(before),
            created_folders,
            written: session.written(changed),
        });
    }

    let mut created = Vec::new();
    let mut folder_id = destination;
    for name in &missing {
        let folder = session.store.create_folder(folder_id, name)?;
        session.note(Change::FolderCreated { id: folder.id });
        folder_id = Some(folder.id);
        created.push(folder);
    }
    session.store.move_document(meta.id, folder_id)?;
    session.note(Change::DocumentMoved { id: meta.id });
    if let Some(place) = place {
        session.store.reorder_document(meta.id, place)?;
    }
    let store = &session.store;
    let after = store.document_meta(meta.id)?;
    let result = DocumentChanged {
        document: index.document_summary(store, &after),
        before: Some(before),
        created_folders: created
            .iter()
            .map(|folder| index.folder_summary(store, folder))
            .collect(),
        written: session.written(true),
    };
    Ok(result)
}

/// Copies a document: text, comments and settings. See [`DuplicateDocumentRequest`].
/// In the result `before` is the original.
pub(crate) fn duplicate_document(
    session: &mut Session<'_>,
    request: &DuplicateDocumentRequest,
) -> Result<DocumentChanged> {
    let index = Index::load(session.store)?;
    let store = &session.store;
    let meta = index.live_document(store, &request.document)?;
    let folder = match &request.folder {
        Some(target) => Some(index.live_folder_or_root(store, target)?),
        None => None,
    };
    let title = match request.title.as_deref().map(str::trim) {
        Some("") => return Err(Error::invalid("the title of the copy is empty")),
        Some(title) => Some(title.split_whitespace().collect::<Vec<_>>().join(" ")),
        None => None,
    };
    let original = index.document_summary(store, &meta);
    // As the library names a copy.
    let default_title = if meta.title.trim().is_empty() {
        "Untitled copy".to_string()
    } else {
        format!("{} copy", meta.title)
    };
    let destination = match &folder {
        Some(folder) => folder.as_ref().map(|folder| folder.id),
        None => meta.folder_id,
    };

    if session.dry_run {
        let now = Time(session.clock.now_ms());
        let planned = DocumentMeta {
            id: DocumentId(0),
            title: title.clone().unwrap_or(default_title),
            folder_id: destination,
            created_at: now.0,
            updated_at: now.0,
            ..meta.clone()
        };
        return Ok(DocumentChanged {
            document: index.document_summary(store, &planned),
            before: Some(original),
            created_folders: Vec::new(),
            written: session.written(true),
        });
    }

    let store = &mut session.store;
    let copy = store.duplicate_document(meta.id)?;
    session
        .changes
        .push(Change::DocumentCreated { id: copy.id });
    if let Some(title) = &title {
        store.rename_document(copy.id, title)?;
    }
    if destination != meta.folder_id {
        store.move_document(copy.id, destination)?;
    }
    let copy = store.document_meta(copy.id)?;
    // A title that was asked for is kept like a rename; the library's own "copy"
    // name follows the text again with the first change, as in the app.
    content::set_title_flag(store, &copy, title.is_some())?;
    let result = DocumentChanged {
        document: index.document_summary(store, &copy),
        before: Some(original),
        created_folders: Vec::new(),
        written: session.written(true),
    };
    Ok(result)
}

/// Moves a document to the trash.
pub(crate) fn trash_document(
    session: &mut Session<'_>,
    request: &TrashDocumentRequest,
) -> Result<DocumentChanged> {
    let index = Index::load(session.store)?;
    let meta = index.live_document(session.store, &request.document)?;
    let before = index.document_summary(session.store, &meta);
    let after = if session.dry_run {
        DocumentMeta {
            trashed_at: Some(session.clock.now_ms()),
            ..meta.clone()
        }
    } else {
        session.store.trash_document(meta.id)?;
        session.note(Change::DocumentTrashed { id: meta.id });
        session.store.document_meta(meta.id)?
    };
    let result = DocumentChanged {
        document: index.document_summary(session.store, &after),
        before: Some(before),
        created_folders: Vec::new(),
        written: session.written(true),
    };
    Ok(result)
}

/// Brings a document back from the trash.
pub(crate) fn restore_document(
    session: &mut Session<'_>,
    request: &RestoreDocumentRequest,
) -> Result<DocumentChanged> {
    let index = Index::load(session.store)?;
    let found = index.trash_entry(session.store, &request.document, Some(ItemKind::Document))?;
    let Found::Document(meta) = found else {
        return Err(Error::invalid("that entry of the trash is a folder"));
    };
    let before = index.document_summary(session.store, &meta);
    let after = if session.dry_run {
        // Back to its folder when that is still there, else to the top.
        let folder_id = meta.folder_id.filter(|&id| index.node(id).is_some());
        DocumentMeta {
            trashed_at: None,
            folder_id,
            ..meta.clone()
        }
    } else {
        let restored = session.store.restore_document(meta.id)?;
        session.note(Change::DocumentRestored { id: meta.id });
        restored
    };
    let result = DocumentChanged {
        document: index.document_summary(session.store, &after),
        before: Some(before),
        created_folders: Vec::new(),
        written: session.written(true),
    };
    Ok(result)
}

/// Deletes a document for good, with its versions. Refused with
/// [`Error::ConfirmationRequired`](crate::Error::ConfirmationRequired) unless the
/// request says `permanently`.
pub(crate) fn delete_document(
    session: &mut Session<'_>,
    request: &DeleteDocumentRequest,
) -> Result<DocumentChanged> {
    let index = Index::load(session.store)?;
    let store = &session.store;
    // A live document by id or path; else an entry of the trash by id or name.
    let meta = match index.document(store, &request.document) {
        Ok(meta) => meta,
        Err(Error::NotFound { .. }) if matches!(request.document, Target::Path(_)) => {
            match index.trash_entry(store, &request.document, Some(ItemKind::Document))? {
                Found::Document(meta) => meta,
                Found::Folder(_) => {
                    return Err(Error::invalid("that entry of the trash is a folder"));
                }
            }
        }
        Err(error) => return Err(error),
    };
    if meta.trashed_at.is_some() {
        // One that went to the trash with its folder goes with that folder.
        index.trash_entry(
            store,
            &Target::DocumentId(meta.id.0),
            Some(ItemKind::Document),
        )?;
    }
    let summary = index.document_summary(store, &meta);
    if !request.permanently {
        return Err(confirmation(format!(
            "The {} {} ({})",
            meta.kind.noun(),
            quoted_excerpt(title_of(&meta)),
            summary.reference()
        )));
    }
    if !session.dry_run {
        let store = &mut session.store;
        if meta.trashed_at.is_none() {
            store.trash_document(meta.id)?;
            // Should the next step fail, the document is in the trash.
            session
                .changes
                .push(Change::DocumentTrashed { id: meta.id });
        }
        store.purge_document(meta.id)?;
        session
            .changes
            .retain(|change| *change != Change::DocumentTrashed { id: meta.id });
        session
            .changes
            .push(Change::DocumentDeleted { id: meta.id });
        forget_deleted_documents(store)?;
    }
    let result = DocumentChanged {
        document: summary,
        before: None,
        created_folders: Vec::new(),
        written: session.written(true),
    };
    Ok(result)
}

/// Creates a folder, with the folders above it that are missing. See
/// [`CreateFolderRequest`].
pub(crate) fn create_folder(
    session: &mut Session<'_>,
    request: &CreateFolderRequest,
) -> Result<FolderCreated> {
    let index = Index::load(session.store)?;
    let store = &session.store;
    let base = match &request.parent {
        Some(target) => index.live_folder_or_root(store, target)?,
        None => None,
    };
    let (existing, missing) = plan_folders(&index, base.as_ref(), &request.path)?;
    if missing.is_empty() {
        let folder = existing.expect("a path with names ends at a folder");
        return Ok(FolderCreated {
            folder: index.folder_summary(store, &folder),
            created: Vec::new(),
            written: session.written(false),
        });
    }
    if session.dry_run {
        let mut path = index.folder_segments(store, existing.as_ref().map(|f| f.id));
        let now = Time(session.clock.now_ms());
        let mut above = existing.as_ref().map(|folder| folder.id);
        let mut created = Vec::new();
        for name in &missing {
            path.push(name.clone());
            created.push(planned_folder(&path, above, now));
            above = Some(FolderId(0));
        }
        return Ok(FolderCreated {
            folder: created.last().cloned().expect("something is missing"),
            created,
            written: session.written(true),
        });
    }
    let store = &mut session.store;
    let mut parent = existing.as_ref().map(|folder| folder.id);
    let mut created = Vec::new();
    for name in &missing {
        let folder = store.create_folder(parent, name).map_err(folder_error)?;
        session
            .changes
            .push(Change::FolderCreated { id: folder.id });
        parent = Some(folder.id);
        created.push(folder);
    }
    let created: Vec<FolderSummary> = created
        .iter()
        .map(|folder| index.folder_summary(store, folder))
        .collect();
    let result = FolderCreated {
        folder: created.last().cloned().expect("something was created"),
        created,
        written: session.written(true),
    };
    Ok(result)
}

/// Renames a folder.
pub(crate) fn rename_folder(
    session: &mut Session<'_>,
    request: &RenameFolderRequest,
) -> Result<FolderChanged> {
    let index = Index::load(session.store)?;
    let folder = index.live_folder(session.store, &request.folder)?;
    let name = request
        .name
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if name.is_empty() {
        return Err(Error::invalid("the new name is empty"));
    }
    let changed = name != folder.name;
    let before = index.folder_summary(session.store, &folder);
    let mut after = Folder {
        name: name.clone(),
        ..folder.clone()
    };
    if changed && !session.dry_run {
        session
            .store
            .rename_folder(folder.id, &name)
            .map_err(folder_error)?;
        session.note(Change::FolderRenamed { id: folder.id });
        after = session.store.folder(folder.id)?;
    }
    // The paths of what is inside follow the name; the summary is built from the
    // row, not from the tree that was read before.
    let mut document = index.folder_summary(session.store, &after);
    let mut path = index.folder_segments(session.store, after.parent_id);
    path.push(after.name.clone());
    document.path = join_path(&path);
    let result = FolderChanged {
        folder: document,
        before: Some(before),
        folders: 0,
        documents: 0,
        written: session.written(changed),
    };
    Ok(result)
}

/// Moves a folder, with everything in it. See [`MoveFolderRequest`].
pub(crate) fn move_folder(
    session: &mut Session<'_>,
    request: &MoveFolderRequest,
) -> Result<FolderChanged> {
    if request.parent.is_none() && request.placement.is_none() {
        return Err(Error::invalid(
            "nothing to do: name the folder to move it into, a place among its \
                 neighbours, or both",
        ));
    }
    let index = Index::load(session.store)?;
    let store = &session.store;
    let folder = index.live_folder(store, &request.folder)?;
    let destination = match &request.parent {
        Some(target) => index
            .live_folder_or_root(store, target)?
            .map(|parent| parent.id),
        None => folder.parent_id,
    };
    // Not into itself, nor into anything inside it.
    let mut above = destination;
    for _ in 0..1000 {
        let Some(id) = above else { break };
        if id == folder.id {
            return Err(folder_error(StoreError::FolderCycle));
        }
        above = store.folder(id)?.parent_id;
    }
    let siblings: Vec<i64> = index
        .child_folders(destination)
        .iter()
        .map(|node| node.folder.id.0)
        .filter(|&id| id != folder.id.0)
        .collect();
    let was = index
        .child_folders(folder.parent_id)
        .iter()
        .position(|node| node.folder.id == folder.id);
    let place = match &request.placement {
        Some(placement) => Some(place_among(&siblings, placement, |target| {
            Ok(index.live_folder(store, target)?.id.0)
        })?),
        None => None,
    };
    let same_parent = destination == folder.parent_id;
    let changed = !same_parent || place.is_some_and(|place| Some(place) != was);
    let before = index.folder_summary(store, &folder);
    let (folders, documents, _) = index.node(folder.id).map_or((0, 0, 0), contents);
    let mut after = Folder {
        parent_id: destination,
        ..folder.clone()
    };
    if changed && !session.dry_run {
        let store = &mut session.store;
        store
            .move_folder(folder.id, destination)
            .map_err(folder_error)?;
        session.changes.push(Change::FolderMoved { id: folder.id });
        if let Some(place) = place {
            store.reorder_folder(folder.id, place)?;
        }
        after = store.folder(folder.id)?;
    }
    let mut path = index.folder_segments(session.store, destination);
    path.push(after.name.clone());
    let result = FolderChanged {
        folder: FolderSummary {
            path: join_path(&path),
            parent_id: destination,
            updated_at: Time(after.updated_at),
            ..before.clone()
        },
        before: Some(before),
        folders,
        documents,
        written: session.written(changed),
    };
    Ok(result)
}

/// Moves a folder to the trash, with everything in it.
pub(crate) fn trash_folder(
    session: &mut Session<'_>,
    request: &TrashFolderRequest,
) -> Result<FolderChanged> {
    let index = Index::load(session.store)?;
    let folder = index.live_folder(session.store, &request.folder)?;
    let before = index.folder_summary(session.store, &folder);
    let (folders, documents, _) = index.node(folder.id).map_or((0, 0, 0), contents);
    let mut after = before.clone();
    if session.dry_run {
        after.trashed_at = Some(Time(session.clock.now_ms()));
    } else {
        session.store.trash_folder(folder.id)?;
        session.note(Change::FolderTrashed { id: folder.id });
        after.trashed_at = session.store.folder(folder.id)?.trashed_at.map(Time);
    }
    let result = FolderChanged {
        folder: after,
        before: Some(before),
        folders,
        documents,
        written: session.written(true),
    };
    Ok(result)
}

/// Brings a folder back from the trash, with everything that went there with it.
pub(crate) fn restore_folder(
    session: &mut Session<'_>,
    request: &RestoreFolderRequest,
) -> Result<FolderChanged> {
    let index = Index::load(session.store)?;
    let found = index.trash_entry(session.store, &request.folder, Some(ItemKind::Folder))?;
    let Found::Folder(folder) = found else {
        return Err(Error::invalid("that entry of the trash is a document"));
    };
    let (folders, documents) = trashed_with(session.store, folder.id)?;
    let before = index.folder_summary(session.store, &folder);
    let after = if session.dry_run {
        // Back into its folder when that is still there, else to the top.
        let parent_id = folder.parent_id.filter(|&id| index.node(id).is_some());
        Folder {
            trashed_at: None,
            parent_id,
            ..folder.clone()
        }
    } else {
        let restored = session.store.restore_folder(folder.id)?;
        session.note(Change::FolderRestored { id: folder.id });
        restored
    };
    let result = FolderChanged {
        folder: index.folder_summary(session.store, &after),
        before: Some(before),
        folders,
        documents,
        written: session.written(true),
    };
    Ok(result)
}

/// Deletes a folder for good, with every document in it. Refused with
/// [`Error::ConfirmationRequired`](crate::Error::ConfirmationRequired) unless the
/// request says `permanently`.
pub(crate) fn delete_folder(
    session: &mut Session<'_>,
    request: &DeleteFolderRequest,
) -> Result<FolderChanged> {
    let index = Index::load(session.store)?;
    let store = &session.store;
    let folder = match index.folder(store, &request.folder) {
        Ok(folder) => folder,
        Err(Error::NotFound { .. }) if matches!(request.folder, Target::Path(_)) => {
            match index.trash_entry(store, &request.folder, Some(ItemKind::Folder))? {
                Found::Folder(folder) => folder,
                Found::Document(_) => {
                    return Err(Error::invalid("that entry of the trash is a document"));
                }
            }
        }
        Err(error) => return Err(error),
    };
    let (folders, documents) = if folder.trashed_at.is_some() {
        index.trash_entry(
            store,
            &Target::FolderId(folder.id.0),
            Some(ItemKind::Folder),
        )?;
        trashed_with(store, folder.id)?
    } else {
        let (folders, documents, _) = index.node(folder.id).map_or((0, 0, 0), contents);
        (folders, documents)
    };
    let summary = index.folder_summary(store, &folder);
    if !request.permanently {
        return Err(confirmation(format!(
            "The folder {} (folder:{}) with {documents} document{} in it",
            quoted_excerpt(&folder.name),
            folder.id,
            if documents == 1 { "" } else { "s" }
        )));
    }
    if !session.dry_run {
        let store = &mut session.store;
        if folder.trashed_at.is_none() {
            store.trash_folder(folder.id)?;
            // Should the next step fail, the folder is in the trash.
            session
                .changes
                .push(Change::FolderTrashed { id: folder.id });
        }
        store.purge_folder(folder.id)?;
        session
            .changes
            .retain(|change| *change != Change::FolderTrashed { id: folder.id });
        session
            .changes
            .push(Change::FolderDeleted { id: folder.id });
        forget_deleted_documents(store)?;
    }
    let result = FolderChanged {
        folder: summary,
        before: None,
        folders,
        documents,
        written: session.written(true),
    };
    Ok(result)
}

/// Deletes everything in the trash for good. Refused with
/// [`Error::ConfirmationRequired`](crate::Error::ConfirmationRequired) unless the
/// request says `permanently`.
pub(crate) fn empty_trash(
    session: &mut Session<'_>,
    request: &EmptyTrashRequest,
) -> Result<TrashEmptied> {
    let entries = session.store.trash()?;
    let (mut folders, mut documents) = (0, 0);
    for entry in &entries {
        match entry {
            TrashEntry::Document(_) => documents += 1,
            TrashEntry::Folder {
                folder_count,
                document_count,
                ..
            } => {
                folders += 1 + folder_count;
                documents += document_count;
            }
        }
    }
    if !request.permanently {
        return Err(confirmation(format!(
            "Everything in the trash ({documents} document{}, {folders} folder{})",
            if documents == 1 { "" } else { "s" },
            if folders == 1 { "" } else { "s" },
        )));
    }
    let changed = !entries.is_empty();
    if changed && !session.dry_run {
        let emptied = session.store.empty_trash()?;
        session.note(Change::TrashEmptied);
        folders = emptied.folders;
        documents = emptied.documents;
        forget_deleted_documents(session.store)?;
    }
    let result = TrashEmptied {
        folders,
        documents,
        written: session.written(changed),
    };
    Ok(result)
}

/// How many folders and documents went to the trash with the folder `id`.
fn trashed_with(store: &Store, id: FolderId) -> Result<(u64, u64)> {
    Ok(store
        .trash()?
        .iter()
        .find_map(|entry| match entry {
            TrashEntry::Folder {
                folder,
                folder_count,
                document_count,
            } if folder.id == id => Some((*folder_count, *document_count)),
            _ => None,
        })
        .unwrap_or((0, 0)))
}
