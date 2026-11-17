//! From a [`Target`] to the document or folder it names, and from the library's rows to the
//! summaries results carry.

use std::cell::RefCell;
use std::collections::HashMap;

use innerword_store::{
    DocumentId, DocumentMeta, Folder, FolderId, FolderNode, LibraryTree, Store, StoreError,
    TrashEntry,
};

use crate::error::{Candidate, Error, ItemKind, Result, quoted_excerpt};
use crate::library::Library;
use crate::types::{DocumentSummary, FolderSummary, Item, Target, Time};

/// The name of a document without a title, in paths and listings.
pub(crate) const UNTITLED: &str = "Untitled";

/// How many near misses a "not found" names.
const SUGGESTIONS: usize = 5;

/// The library as one operation sees it: the tree of what is not in the trash, with the
/// path of everything in it.
pub(crate) struct Index {
    /// The tree, as the store gives it.
    pub(crate) tree: LibraryTree,
    /// Every live folder with the names on the way to it, itself included.
    folders: HashMap<FolderId, (Folder, Vec<String>)>,
    /// Every live document with the names on the way to it, its title included, in the
    /// order of the tree.
    documents: Vec<(DocumentMeta, Vec<String>)>,
    /// The paths of folders that are not in the tree (trashed ones), as they are asked
    /// for.
    other_folders: RefCell<HashMap<FolderId, Vec<String>>>,
}

impl Index {
    /// Reads the tree of `store`.
    pub(crate) fn load(store: &Store) -> Result<Index> {
        let tree = store.library_tree()?;
        let mut index = Index {
            folders: HashMap::new(),
            documents: Vec::new(),
            other_folders: RefCell::new(HashMap::new()),
            tree: LibraryTree {
                folders: Vec::new(),
                documents: Vec::new(),
            },
        };
        fn walk(index: &mut Index, nodes: &[FolderNode], above: &[String]) {
            for node in nodes {
                let mut path = above.to_vec();
                path.push(node.folder.name.clone());
                index
                    .folders
                    .insert(node.folder.id, (node.folder.clone(), path.clone()));
                walk(index, &node.folders, &path);
                for document in &node.documents {
                    let mut segments = path.clone();
                    segments.push(title_of(document).to_string());
                    index.documents.push((document.clone(), segments));
                }
            }
        }
        walk(&mut index, &tree.folders, &[]);
        for document in &tree.documents {
            index
                .documents
                .push((document.clone(), vec![title_of(document).to_string()]));
        }
        index.tree = tree;
        Ok(index)
    }

    /// The names on the way to a folder, itself included; empty for the top of the
    /// library. Folders in the trash are followed up through the store.
    pub(crate) fn folder_segments(&self, store: &Store, folder: Option<FolderId>) -> Vec<String> {
        let Some(id) = folder else {
            return Vec::new();
        };
        if let Some((_, path)) = self.folders.get(&id) {
            return path.clone();
        }
        if let Some(path) = self.other_folders.borrow().get(&id) {
            return path.clone();
        }
        let mut names = Vec::new();
        let mut next = Some(id);
        // A parent chain in a damaged library could loop; no library is this deep.
        for _ in 0..1000 {
            let Some(current) = next else { break };
            if let Some((_, path)) = self.folders.get(&current) {
                let mut whole = path.clone();
                whole.extend(names.into_iter().rev());
                self.other_folders.borrow_mut().insert(id, whole.clone());
                return whole;
            }
            match store.folder(current) {
                Ok(folder) => {
                    names.push(folder.name);
                    next = folder.parent_id;
                }
                Err(_) => break,
            }
        }
        names.reverse();
        self.other_folders.borrow_mut().insert(id, names.clone());
        names
    }

    /// A document as results show it.
    pub(crate) fn document_summary(&self, store: &Store, meta: &DocumentMeta) -> DocumentSummary {
        let folder = self.folder_segments(store, meta.folder_id);
        let mut path = folder.clone();
        path.push(title_of(meta).to_string());
        DocumentSummary {
            id: meta.id,
            title: meta.title.clone(),
            path: join_path(&path),
            folder_id: meta.folder_id,
            folder: join_path(&folder),
            words: meta.word_count,
            characters: meta.char_count,
            created_at: Time(meta.created_at),
            updated_at: Time(meta.updated_at),
            trashed_at: meta.trashed_at.map(Time),
            kind: meta.kind,
            source_url: meta.source_url.clone(),
        }
    }

    /// A folder as results show it.
    pub(crate) fn folder_summary(&self, store: &Store, folder: &Folder) -> FolderSummary {
        let mut path = self.folder_segments(store, folder.parent_id);
        path.push(folder.name.clone());
        FolderSummary {
            id: folder.id,
            name: folder.name.clone(),
            path: join_path(&path),
            parent_id: folder.parent_id,
            created_at: Time(folder.created_at),
            updated_at: Time(folder.updated_at),
            trashed_at: folder.trashed_at.map(Time),
        }
    }

    /// The node of a live folder in the tree.
    pub(crate) fn node(&self, id: FolderId) -> Option<&FolderNode> {
        self.tree.find_folder(id)
    }

    /// The live folders directly in `parent`, in order.
    pub(crate) fn child_folders(&self, parent: Option<FolderId>) -> &[FolderNode] {
        match parent {
            None => &self.tree.folders,
            Some(id) => self.node(id).map_or(&[], |node| &node.folders),
        }
    }

    /// The live documents directly in `folder`, in order.
    pub(crate) fn child_documents(&self, folder: Option<FolderId>) -> &[DocumentMeta] {
        match folder {
            None => &self.tree.documents,
            Some(id) => self.node(id).map_or(&[], |node| &node.documents),
        }
    }

    // ----- documents ---------------------------------------------------------------------

    /// The document `target` names: by id whatever its state, by path among the live
    /// ones.
    pub(crate) fn document(&self, store: &Store, target: &Target) -> Result<DocumentMeta> {
        match target {
            Target::Id(id) | Target::DocumentId(id) => document_by_id(store, *id),
            Target::FolderId(id) => Err(Error::invalid(format!(
                "folder:{id} names a folder; a document is needed here"
            ))),
            Target::Path(path) => {
                let (anchored, segments) = parse_path(path);
                if segments.is_empty() {
                    return Err(Error::invalid(if target.is_empty() {
                        "no document was named: give its id or its path in the library"
                    } else {
                        "the top of the library is not a document"
                    }));
                }
                let found = self.matching_documents(anchored, &segments);
                match found.len() {
                    1 => Ok(found[0].0.clone()),
                    0 => Err(Error::NotFound {
                        kind: ItemKind::Document,
                        reference: format!("at {}", quoted_excerpt(path.trim())),
                        hint: self.document_hint(store, &segments),
                    }),
                    _ => Err(Error::Ambiguous {
                        kind: ItemKind::Document,
                        reference: quoted_excerpt(path.trim()),
                        candidates: found
                            .iter()
                            .map(|(meta, segments)| Candidate {
                                kind: ItemKind::Document,
                                id: meta.id.0,
                                path: join_path(segments),
                                trashed: false,
                            })
                            .collect(),
                    }),
                }
            }
        }
    }

    /// The document `target` names, which has to be live.
    pub(crate) fn live_document(&self, store: &Store, target: &Target) -> Result<DocumentMeta> {
        let meta = self.document(store, target)?;
        if meta.trashed_at.is_some() {
            return Err(Error::InTrash {
                kind: ItemKind::Document,
                id: meta.id.0,
                name: quoted_excerpt(title_of(&meta)),
            });
        }
        Ok(meta)
    }

    fn matching_documents(
        &self,
        anchored: bool,
        segments: &[String],
    ) -> Vec<&(DocumentMeta, Vec<String>)> {
        for fold in [false, true] {
            let found: Vec<_> = self
                .documents
                .iter()
                .filter(|(_, path)| path_fits(path, segments, anchored, fold))
                .collect();
            if !found.is_empty() {
                return found;
            }
        }
        Vec::new()
    }

    /// What to say when no document has that path: the documents whose title is close.
    fn document_hint(&self, store: &Store, segments: &[String]) -> Option<String> {
        let wanted = segments.last()?.to_lowercase();
        let mut close: Vec<String> = self
            .documents
            .iter()
            .filter(|(meta, _)| {
                let title = title_of(meta).to_lowercase();
                title.contains(&wanted) || (!title.is_empty() && wanted.contains(&title))
            })
            .take(SUGGESTIONS)
            .map(|(meta, path)| format!("doc:{} {}", meta.id, join_path(path)))
            .collect();
        if close.is_empty() {
            // Perhaps it is a folder, or in the trash.
            if !self.matching_folders(false, segments).is_empty() {
                return Some("A folder has that path".to_string());
            }
            if let Ok(entries) = store.trash() {
                close = entries
                    .iter()
                    .filter_map(|entry| match entry {
                        TrashEntry::Document(meta) if title_of(meta).to_lowercase() == wanted => {
                            Some(format!("doc:{} (in the trash)", meta.id))
                        }
                        _ => None,
                    })
                    .take(SUGGESTIONS)
                    .collect();
            }
            if close.is_empty() {
                return Some(
                    "Paths are folder names and a title separated by \"/\"; the tree of the \
                     library lists them with their ids"
                        .to_string(),
                );
            }
        }
        Some(format!("Close: {}", close.join("; ")))
    }

    // ----- folders -----------------------------------------------------------------------

    /// The folder `target` names, or `None` for the top of the library: by id whatever its
    /// state, by path among the live ones.
    pub(crate) fn folder_or_root(&self, store: &Store, target: &Target) -> Result<Option<Folder>> {
        match target {
            Target::Id(id) | Target::FolderId(id) => folder_by_id(store, *id).map(Some),
            Target::DocumentId(id) => Err(Error::invalid(format!(
                "doc:{id} names a document; a folder is needed here"
            ))),
            Target::Path(path) => {
                if target.is_root() {
                    return Ok(None);
                }
                let (anchored, segments) = parse_path(path);
                if segments.is_empty() {
                    return Err(Error::invalid(
                        "no folder was named: give its id or its path in the library (\"/\" is \
                         the top of the library)",
                    ));
                }
                let found = self.matching_folders(anchored, &segments);
                match found.len() {
                    1 => Ok(Some(found[0].0.clone())),
                    0 => Err(Error::NotFound {
                        kind: ItemKind::Folder,
                        reference: format!("at {}", quoted_excerpt(path.trim())),
                        hint: self.folder_hint(&segments),
                    }),
                    _ => Err(Error::Ambiguous {
                        kind: ItemKind::Folder,
                        reference: quoted_excerpt(path.trim()),
                        candidates: found
                            .iter()
                            .map(|(folder, segments)| Candidate {
                                kind: ItemKind::Folder,
                                id: folder.id.0,
                                path: join_path(segments),
                                trashed: false,
                            })
                            .collect(),
                    }),
                }
            }
        }
    }

    /// The folder `target` names; the top of the library is not one.
    pub(crate) fn folder(&self, store: &Store, target: &Target) -> Result<Folder> {
        self.folder_or_root(store, target)?.ok_or_else(|| {
            Error::invalid("the top of the library is not a folder that can be named here")
        })
    }

    /// The folder `target` names, which has to be live.
    pub(crate) fn live_folder(&self, store: &Store, target: &Target) -> Result<Folder> {
        let folder = self.folder(store, target)?;
        require_live_folder(&folder)?;
        Ok(folder)
    }

    /// A destination: a live folder, or `None` for the top of the library.
    pub(crate) fn live_folder_or_root(
        &self,
        store: &Store,
        target: &Target,
    ) -> Result<Option<Folder>> {
        let folder = self.folder_or_root(store, target)?;
        if let Some(folder) = &folder {
            require_live_folder(folder)?;
        }
        Ok(folder)
    }

    fn matching_folders(&self, anchored: bool, segments: &[String]) -> Vec<&(Folder, Vec<String>)> {
        for fold in [false, true] {
            let mut found: Vec<_> = self
                .folders
                .values()
                .filter(|(_, path)| path_fits(path, segments, anchored, fold))
                .collect();
            if !found.is_empty() {
                found.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.id.cmp(&b.0.id)));
                return found;
            }
        }
        Vec::new()
    }

    fn folder_hint(&self, segments: &[String]) -> Option<String> {
        let wanted = segments.last()?.to_lowercase();
        let mut close: Vec<(&Vec<String>, FolderId)> = self
            .folders
            .values()
            .filter(|(folder, _)| {
                let name = folder.name.to_lowercase();
                name.contains(&wanted) || wanted.contains(&name)
            })
            .map(|(folder, path)| (path, folder.id))
            .collect();
        close.sort();
        if close.is_empty() {
            if !self.matching_documents(false, segments).is_empty() {
                return Some("A document has that path".to_string());
            }
            return Some(
                "Paths are folder names separated by \"/\"; the tree of the library lists them \
                 with their ids"
                    .to_string(),
            );
        }
        Some(format!(
            "Close: {}",
            close
                .iter()
                .take(SUGGESTIONS)
                .map(|(path, id)| format!("folder:{id} {}", join_path(path)))
                .collect::<Vec<_>>()
                .join("; ")
        ))
    }

    // ----- either ------------------------------------------------------------------------

    /// The live document or folder `target` names (by id: whatever its state).
    pub(crate) fn item(&self, store: &Store, target: &Target) -> Result<Found> {
        match target {
            Target::DocumentId(_) => self.document(store, target).map(Found::Document),
            Target::FolderId(_) => self.folder(store, target).map(Found::Folder),
            Target::Id(id) => {
                let document = document_by_id(store, *id);
                let folder = folder_by_id(store, *id);
                match (document, folder) {
                    (Ok(document), Ok(folder)) => Err(Error::Ambiguous {
                        kind: ItemKind::Document,
                        reference: format!("the id {id}"),
                        candidates: vec![
                            self.document_candidate(store, &document),
                            self.folder_candidate(store, &folder),
                        ],
                    }),
                    (Ok(document), Err(_)) => Ok(Found::Document(document)),
                    (Err(_), Ok(folder)) => Ok(Found::Folder(folder)),
                    (Err(error), Err(_)) => Err(match error {
                        Error::NotFound { .. } => Error::NotFound {
                            kind: ItemKind::Document,
                            reference: format!("or folder with the id {id}"),
                            hint: None,
                        },
                        other => other,
                    }),
                }
            }
            Target::Path(path) => {
                if target.is_root() {
                    return Err(Error::invalid(
                        "the top of the library is neither a document nor a folder",
                    ));
                }
                let (anchored, segments) = parse_path(path);
                if segments.is_empty() {
                    return Err(Error::invalid(
                        "nothing was named: give an id (doc:12, folder:3) or a path in the library",
                    ));
                }
                // Exact names first, for documents and folders alike.
                for fold in [false, true] {
                    let documents: Vec<_> = self
                        .documents
                        .iter()
                        .filter(|(_, p)| path_fits(p, &segments, anchored, fold))
                        .collect();
                    let mut folders: Vec<_> = self
                        .folders
                        .values()
                        .filter(|(_, p)| path_fits(p, &segments, anchored, fold))
                        .collect();
                    folders.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.id.cmp(&b.0.id)));
                    match (documents.len(), folders.len()) {
                        (0, 0) => continue,
                        (1, 0) => return Ok(Found::Document(documents[0].0.clone())),
                        (0, 1) => return Ok(Found::Folder(folders[0].0.clone())),
                        _ => {
                            let mut candidates: Vec<Candidate> = folders
                                .iter()
                                .map(|(folder, p)| Candidate {
                                    kind: ItemKind::Folder,
                                    id: folder.id.0,
                                    path: join_path(p),
                                    trashed: false,
                                })
                                .collect();
                            candidates.extend(documents.iter().map(|(meta, p)| Candidate {
                                kind: ItemKind::Document,
                                id: meta.id.0,
                                path: join_path(p),
                                trashed: false,
                            }));
                            return Err(Error::Ambiguous {
                                kind: if documents.is_empty() {
                                    ItemKind::Folder
                                } else {
                                    ItemKind::Document
                                },
                                reference: quoted_excerpt(path.trim()),
                                candidates,
                            });
                        }
                    }
                }
                Err(Error::NotFound {
                    kind: ItemKind::Document,
                    reference: format!("or folder at {}", quoted_excerpt(path.trim())),
                    hint: self
                        .document_hint(store, &segments)
                        .filter(|hint| hint.starts_with("Close"))
                        .or_else(|| self.folder_hint(&segments)),
                })
            }
        }
    }

    fn document_candidate(&self, store: &Store, meta: &DocumentMeta) -> Candidate {
        Candidate {
            kind: ItemKind::Document,
            id: meta.id.0,
            path: self.document_summary(store, meta).path,
            trashed: meta.trashed_at.is_some(),
        }
    }

    fn folder_candidate(&self, store: &Store, folder: &Folder) -> Candidate {
        Candidate {
            kind: ItemKind::Folder,
            id: folder.id.0,
            path: self.folder_summary(store, folder).path,
            trashed: folder.trashed_at.is_some(),
        }
    }

    // ----- the trash ---------------------------------------------------------------------

    /// The entry of the trash `target` names: by id, or by its name (or the path it had)
    /// among the entries. `want` narrows a name to documents or to folders.
    pub(crate) fn trash_entry(
        &self,
        store: &Store,
        target: &Target,
        want: Option<ItemKind>,
    ) -> Result<Found> {
        let entries = store.trash()?;
        let kind = want.unwrap_or(ItemKind::Document);
        let wants = |kind: ItemKind| want.is_none_or(|wanted| wanted == kind);
        match (target, want) {
            (Target::DocumentId(id), Some(ItemKind::Folder)) => Err(Error::invalid(format!(
                "doc:{id} names a document; a folder is needed here"
            ))),
            (Target::FolderId(id), Some(ItemKind::Document)) => Err(Error::invalid(format!(
                "folder:{id} names a folder; a document is needed here"
            ))),
            (Target::DocumentId(id), _) | (Target::Id(id), Some(ItemKind::Document)) => {
                let meta = document_by_id(store, *id)?;
                self.require_document_entry(store, &entries, &meta)?;
                Ok(Found::Document(meta))
            }
            (Target::FolderId(id), _) | (Target::Id(id), Some(_)) => {
                let folder = folder_by_id(store, *id)?;
                self.require_folder_entry(store, &entries, &folder)?;
                Ok(Found::Folder(folder))
            }
            (Target::Id(id), None) => {
                // Either kind: whichever is an entry of the trash.
                let document = entries.iter().find_map(|entry| match entry {
                    TrashEntry::Document(meta) if meta.id.0 == *id => Some(meta.clone()),
                    _ => None,
                });
                let folder = entries.iter().find_map(|entry| match entry {
                    TrashEntry::Folder { folder, .. } if folder.id.0 == *id => Some(folder.clone()),
                    _ => None,
                });
                match (document, folder) {
                    (Some(document), Some(folder)) => Err(Error::Ambiguous {
                        kind: ItemKind::Document,
                        reference: format!("the id {id}"),
                        candidates: vec![
                            self.document_candidate(store, &document),
                            self.folder_candidate(store, &folder),
                        ],
                    }),
                    (Some(document), None) => Ok(Found::Document(document)),
                    (None, Some(folder)) => Ok(Found::Folder(folder)),
                    (None, None) => {
                        // Say why: it may exist and not be an entry.
                        if let Ok(meta) = document_by_id(store, *id) {
                            self.require_document_entry(store, &entries, &meta)?;
                        }
                        if let Ok(folder) = folder_by_id(store, *id) {
                            self.require_folder_entry(store, &entries, &folder)?;
                        }
                        Err(Error::NotFound {
                            kind,
                            reference: format!("with the id {id} in the trash"),
                            hint: None,
                        })
                    }
                }
            }
            (Target::Path(path), _) => {
                let (_, segments) = parse_path(path);
                if segments.is_empty() {
                    return Err(Error::invalid(
                        "nothing was named: give the id or the name of an entry of the trash",
                    ));
                }
                for fold in [false, true] {
                    let mut found: Vec<(Found, Vec<String>)> = Vec::new();
                    for entry in &entries {
                        match entry {
                            TrashEntry::Document(meta) if wants(ItemKind::Document) => {
                                let mut was = self.folder_segments(store, meta.folder_id);
                                was.push(title_of(meta).to_string());
                                if path_fits(&was, &segments, false, fold) {
                                    found.push((Found::Document(meta.clone()), was));
                                }
                            }
                            TrashEntry::Folder { folder, .. } if wants(ItemKind::Folder) => {
                                let mut was = self.folder_segments(store, folder.parent_id);
                                was.push(folder.name.clone());
                                if path_fits(&was, &segments, false, fold) {
                                    found.push((Found::Folder(folder.clone()), was));
                                }
                            }
                            _ => {}
                        }
                    }
                    match found.len() {
                        0 => continue,
                        1 => return Ok(found.remove(0).0),
                        _ => {
                            return Err(Error::Ambiguous {
                                kind,
                                reference: quoted_excerpt(path.trim()),
                                candidates: found
                                    .iter()
                                    .map(|(item, was)| Candidate {
                                        kind: item.kind(),
                                        id: item.id(),
                                        path: join_path(was),
                                        trashed: true,
                                    })
                                    .collect(),
                            });
                        }
                    }
                }
                Err(Error::NotFound {
                    kind,
                    reference: format!("named {} in the trash", quoted_excerpt(path.trim())),
                    hint: Some(if entries.is_empty() {
                        "The trash is empty".to_string()
                    } else {
                        "The listing of the trash names its entries with their ids".to_string()
                    }),
                })
            }
        }
    }

    fn require_document_entry(
        &self,
        store: &Store,
        entries: &[TrashEntry],
        meta: &DocumentMeta,
    ) -> Result<()> {
        let is_entry = entries
            .iter()
            .any(|entry| matches!(entry, TrashEntry::Document(other) if other.id == meta.id));
        if is_entry {
            return Ok(());
        }
        Err(Error::NotInTrash {
            kind: ItemKind::Document,
            id: meta.id.0,
            name: quoted_excerpt(title_of(meta)),
            inside: meta
                .trashed_at
                .and_then(|_| self.trashed_ancestor(store, entries, meta.folder_id)),
        })
    }

    fn require_folder_entry(
        &self,
        store: &Store,
        entries: &[TrashEntry],
        folder: &Folder,
    ) -> Result<()> {
        let is_entry = entries.iter().any(
            |entry| matches!(entry, TrashEntry::Folder { folder: other, .. } if other.id == folder.id),
        );
        if is_entry {
            return Ok(());
        }
        Err(Error::NotInTrash {
            kind: ItemKind::Folder,
            id: folder.id.0,
            name: quoted_excerpt(&folder.name),
            inside: folder
                .trashed_at
                .and_then(|_| self.trashed_ancestor(store, entries, folder.parent_id)),
        })
    }

    /// The entry of the trash that a trashed item below `folder` went there with.
    fn trashed_ancestor(
        &self,
        store: &Store,
        entries: &[TrashEntry],
        folder: Option<FolderId>,
    ) -> Option<String> {
        let mut next = folder;
        for _ in 0..1000 {
            let id = next?;
            let found = entries.iter().find_map(|entry| match entry {
                TrashEntry::Folder { folder, .. } if folder.id == id => Some(folder),
                _ => None,
            });
            if let Some(folder) = found {
                return Some(format!(
                    "{} (folder:{})",
                    quoted_excerpt(&folder.name),
                    folder.id
                ));
            }
            next = store.folder(id).ok()?.parent_id;
        }
        None
    }
}

/// A document or a folder, as the store has it.
#[derive(Clone, Debug)]
pub(crate) enum Found {
    /// A document.
    Document(DocumentMeta),
    /// A folder.
    Folder(Folder),
}

impl Found {
    fn kind(&self) -> ItemKind {
        match self {
            Found::Document(_) => ItemKind::Document,
            Found::Folder(_) => ItemKind::Folder,
        }
    }

    fn id(&self) -> i64 {
        match self {
            Found::Document(meta) => meta.id.0,
            Found::Folder(folder) => folder.id.0,
        }
    }
}

/// The title of a document as paths and messages show it.
pub(crate) fn title_of(meta: &DocumentMeta) -> &str {
    let title = meta.title.trim();
    if title.is_empty() { UNTITLED } else { title }
}

fn document_by_id(store: &Store, id: i64) -> Result<DocumentMeta> {
    match store.document_meta(DocumentId(id)) {
        Ok(meta) => Ok(meta),
        Err(StoreError::NotFound { .. }) => Err(Error::NotFound {
            kind: ItemKind::Document,
            reference: format!("with the id {id}"),
            hint: None,
        }),
        Err(other) => Err(other.into()),
    }
}

fn folder_by_id(store: &Store, id: i64) -> Result<Folder> {
    match store.folder(FolderId(id)) {
        Ok(folder) => Ok(folder),
        Err(StoreError::NotFound { .. }) => Err(Error::NotFound {
            kind: ItemKind::Folder,
            reference: format!("with the id {id}"),
            hint: None,
        }),
        Err(other) => Err(other.into()),
    }
}

fn require_live_folder(folder: &Folder) -> Result<()> {
    if folder.trashed_at.is_some() {
        return Err(Error::InTrash {
            kind: ItemKind::Folder,
            id: folder.id.0,
            name: quoted_excerpt(&folder.name),
        });
    }
    Ok(())
}

/// Splits a path into names: `/` separates, `\/` is a slash in a name, `\\` a backslash.
/// The first value says whether the path starts at the top of the library.
pub(crate) fn parse_path(path: &str) -> (bool, Vec<String>) {
    let path = path.trim();
    let anchored = path.starts_with('/');
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut chars = path.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if matches!(chars.peek(), Some('/') | Some('\\')) => {
                current.push(chars.next().expect("peeked"));
            }
            '/' => {
                segments.push(std::mem::take(&mut current));
            }
            c => current.push(c),
        }
    }
    segments.push(current);
    let segments = segments
        .into_iter()
        .map(|segment| segment.trim().to_string())
        .filter(|segment| !segment.is_empty())
        .collect();
    (anchored, segments)
}

/// Joins names into a path, escaping what [`parse_path`] reads as syntax.
pub(crate) fn join_path(segments: &[String]) -> String {
    segments
        .iter()
        .map(|segment| segment.replace('\\', "\\\\").replace('/', "\\/"))
        .collect::<Vec<_>>()
        .join("/")
}

/// Whether the names `wanted` are the path `path` (anchored) or its end.
fn path_fits(path: &[String], wanted: &[String], anchored: bool, fold: bool) -> bool {
    if wanted.len() > path.len() || (anchored && wanted.len() != path.len()) {
        return false;
    }
    let tail = &path[path.len() - wanted.len()..];
    tail.iter().zip(wanted).all(|(have, want)| {
        let have = have.trim();
        if fold {
            have.to_lowercase() == want.to_lowercase()
        } else {
            have == want
        }
    })
}

impl Library {
    /// The document `target` names: by id (one in the trash is found too) or by its path
    /// among the documents that are not in the trash. Fails with
    /// [`Error::NotFound`](crate::Error::NotFound) or
    /// [`Error::Ambiguous`](crate::Error::Ambiguous).
    pub fn resolve_document(&self, target: &Target) -> Result<DocumentSummary> {
        let session = self.reader()?;
        let index = Index::load(&session.store)?;
        let meta = index.document(&session.store, target)?;
        Ok(index.document_summary(&session.store, &meta))
    }

    /// The folder `target` names: by id (one in the trash is found too) or by its path
    /// among the folders that are not in the trash. The top of the library
    /// ([`Target::root`]) is not a folder: it fails with
    /// [`Error::Invalid`](crate::Error::Invalid).
    pub fn resolve_folder(&self, target: &Target) -> Result<FolderSummary> {
        let session = self.reader()?;
        let index = Index::load(&session.store)?;
        let folder = index.folder(&session.store, target)?;
        Ok(index.folder_summary(&session.store, &folder))
    }

    /// The document or the folder `target` names, for a front end whose argument may be
    /// either. A path that fits a document and a folder, or a bare id that both a
    /// document and a folder have, fails with
    /// [`Error::Ambiguous`](crate::Error::Ambiguous) (`doc:12` and `folder:12` are
    /// exact). With `in_trash` the name is looked up among the entries of the trash
    /// instead: what a restore or a permanent deletion names.
    pub fn resolve(&self, target: &Target, in_trash: bool) -> Result<Item> {
        let session = self.reader()?;
        let index = Index::load(&session.store)?;
        let found = if in_trash {
            index.trash_entry(&session.store, target, None)?
        } else {
            index.item(&session.store, target)?
        };
        Ok(match found {
            Found::Document(meta) => Item::Document {
                document: index.document_summary(&session.store, &meta),
            },
            Found::Folder(folder) => Item::Folder {
                folder: index.folder_summary(&session.store, &folder),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_split_and_join() {
        let names = |path: &str| parse_path(path).1;
        assert_eq!(
            names("Book/Part one/Chapter 1"),
            ["Book", "Part one", "Chapter 1"]
        );
        assert_eq!(parse_path("/Notes"), (true, vec!["Notes".to_string()]));
        assert_eq!(parse_path(" Notes / "), (false, vec!["Notes".to_string()]));
        assert_eq!(names(r"AC\/DC/Back in Black"), ["AC/DC", "Back in Black"]);
        assert_eq!(names(r"a\\b"), [r"a\b"]);
        assert_eq!(names(r"a\b"), [r"a\b"]);
        assert_eq!(parse_path("/"), (true, vec![]));
        assert_eq!(parse_path(""), (false, vec![]));
        let path = vec!["AC/DC".to_string(), r"a\b".to_string(), "c".to_string()];
        assert_eq!(join_path(&path), r"AC\/DC/a\\b/c");
        assert_eq!(names(&join_path(&path)), path);
    }

    #[test]
    fn paths_fit_at_the_end() {
        let path: Vec<String> = ["Book", "Part one", "Chapter 1"].map(String::from).to_vec();
        let wanted = |names: &[&str]| names.iter().map(|n| n.to_string()).collect::<Vec<_>>();
        assert!(path_fits(&path, &wanted(&["Chapter 1"]), false, false));
        assert!(path_fits(
            &path,
            &wanted(&["Part one", "Chapter 1"]),
            false,
            false
        ));
        assert!(!path_fits(
            &path,
            &wanted(&["Book", "Chapter 1"]),
            false,
            false
        ));
        assert!(!path_fits(&path, &wanted(&["Chapter 1"]), true, false));
        assert!(path_fits(
            &path,
            &wanted(&["Book", "Part one", "Chapter 1"]),
            true,
            false
        ));
        assert!(!path_fits(&path, &wanted(&["chapter 1"]), false, false));
        assert!(path_fits(&path, &wanted(&["chapter 1"]), false, true));
        assert!(!path_fits(
            &path,
            &wanted(&["x", "Book", "Part one", "Chapter 1"]),
            false,
            true
        ));
    }
}
