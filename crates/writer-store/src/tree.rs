//! The nested library tree used by the sidebar.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::Store;
use crate::documents::{DOC_META_COLS, DocumentMeta, meta_from_row};
use crate::error::Result;
use crate::folders::{FOLDER_COLS, Folder, folder_from_row};
use crate::ids::FolderId;

/// A folder with its live subfolders and documents, each in display order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderNode {
    /// The folder itself.
    pub folder: Folder,
    /// Subfolders in display order.
    pub folders: Vec<FolderNode>,
    /// Documents directly in this folder, in display order.
    pub documents: Vec<DocumentMeta>,
}

/// The whole live library: root folders and root documents, nested. Trashed items are
/// excluded. Display order is subfolders first, then documents.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryTree {
    /// Root folders in display order.
    pub folders: Vec<FolderNode>,
    /// Root documents in display order.
    pub documents: Vec<DocumentMeta>,
}

impl LibraryTree {
    /// Finds a folder node anywhere in the tree.
    pub fn find_folder(&self, id: FolderId) -> Option<&FolderNode> {
        fn walk(nodes: &[FolderNode], id: FolderId) -> Option<&FolderNode> {
            nodes.iter().find_map(|n| {
                if n.folder.id == id {
                    Some(n)
                } else {
                    walk(&n.folders, id)
                }
            })
        }
        walk(&self.folders, id)
    }
}

impl Store {
    /// Loads the whole live library as a nested tree in one call (two queries, one read
    /// transaction). Content is not loaded.
    pub fn library_tree(&self) -> Result<LibraryTree> {
        let tx = self.conn.unchecked_transaction()?;
        let folders: Vec<Folder> = {
            let mut stmt = tx.prepare_cached(&format!(
                "SELECT {FOLDER_COLS} FROM folders WHERE trashed_at IS NULL
                 ORDER BY position, id"
            ))?;
            stmt.query_map([], folder_from_row)?
                .collect::<rusqlite::Result<_>>()?
        };
        let documents: Vec<DocumentMeta> = {
            let mut stmt = tx.prepare_cached(&format!(
                "SELECT {DOC_META_COLS} FROM documents WHERE trashed_at IS NULL
                 ORDER BY position, id"
            ))?;
            stmt.query_map([], meta_from_row)?
                .collect::<rusqlite::Result<_>>()?
        };
        tx.commit()?;

        let mut child_folders: HashMap<Option<FolderId>, Vec<Folder>> = HashMap::new();
        for folder in folders {
            child_folders.entry(folder.parent_id).or_default().push(folder);
        }
        let mut child_docs: HashMap<Option<FolderId>, Vec<DocumentMeta>> = HashMap::new();
        for doc in documents {
            child_docs.entry(doc.folder_id).or_default().push(doc);
        }

        // Recursive build: depth equals folder nesting, which people keep shallow. Folders
        // unreachable from the root (impossible under the store's invariants) are dropped.
        fn build(
            parent: Option<FolderId>,
            child_folders: &mut HashMap<Option<FolderId>, Vec<Folder>>,
            child_docs: &mut HashMap<Option<FolderId>, Vec<DocumentMeta>>,
        ) -> Vec<FolderNode> {
            let Some(folders) = child_folders.remove(&parent) else {
                return Vec::new();
            };
            folders
                .into_iter()
                .map(|folder| {
                    let id = Some(folder.id);
                    FolderNode {
                        folders: build(id, child_folders, child_docs),
                        documents: child_docs.remove(&id).unwrap_or_default(),
                        folder,
                    }
                })
                .collect()
        }
        let roots = build(None, &mut child_folders, &mut child_docs);
        Ok(LibraryTree {
            folders: roots,
            documents: child_docs.remove(&None).unwrap_or_default(),
        })
    }
}
