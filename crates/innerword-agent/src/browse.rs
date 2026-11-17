//! Looking around: the tree, search, recent documents, what is known about one item.

use std::collections::HashSet;

use innerword_store::{DocumentMeta, FolderId, FolderNode, Store};

use crate::content;
use crate::error::Result;
use crate::items::contents;
use crate::library::Library;
use crate::resolve::Index;
use crate::sections::headings;
use crate::types::{
    DocumentInfo, DocumentList, FolderInfo, FolderInfoRequest, InfoRequest, RecentRequest,
    SearchHit, SearchRequest, SearchResults, SnippetPart, Tree, TreeFolder, TreeRequest,
};

/// How many hits a search returns unless the request says.
const DEFAULT_SEARCH_LIMIT: usize = 20;
/// How many of the best hits of the whole library a search in one folder looks through.
const FOLDER_SEARCH_POOL: usize = 2000;
/// How many documents "recent" returns unless the request says.
const DEFAULT_RECENT_LIMIT: usize = 10;

fn tree_folder(
    index: &Index,
    store: &Store,
    node: &FolderNode,
    levels: Option<usize>,
    folders_only: bool,
) -> TreeFolder {
    let (folder_count, document_count, _) = contents(node);
    let open = levels.is_none_or(|levels| levels > 0);
    let below = levels.map(|levels| levels.saturating_sub(1));
    TreeFolder {
        folder: index.folder_summary(store, &node.folder),
        folder_count,
        document_count,
        folders: if open {
            node.folders
                .iter()
                .map(|child| tree_folder(index, store, child, below, folders_only))
                .collect()
        } else {
            Vec::new()
        },
        documents: if open && !folders_only {
            node.documents
                .iter()
                .map(|document| index.document_summary(store, document))
                .collect()
        } else {
            Vec::new()
        },
    }
}

/// The ids of a folder and of every folder inside it.
fn subtree(node: &FolderNode, ids: &mut HashSet<FolderId>) {
    ids.insert(node.folder.id);
    for child in &node.folders {
        subtree(child, ids);
    }
}

impl Library {
    /// The tree of folders and documents: all of it, or one folder, to a depth.
    pub fn tree(&self, request: &TreeRequest) -> Result<Tree> {
        let session = self.reader()?;
        let store = &session.store;
        let index = Index::load(store)?;
        let start = match &request.folder {
            Some(target) => index.live_folder_or_root(store, target)?,
            None => None,
        };
        let start_id = start.as_ref().map(|folder| folder.id);
        let nodes = index.child_folders(start_id);
        let documents = index.child_documents(start_id);
        let mut totals = (0, documents.len() as u64, 0);
        totals.2 = documents.iter().map(|document| document.word_count).sum();
        for node in nodes {
            let (folders, documents, words) = contents(node);
            totals.0 += 1 + folders;
            totals.1 += documents;
            totals.2 += words;
        }
        Ok(Tree {
            folder: start
                .as_ref()
                .map(|folder| index.folder_summary(store, folder)),
            folders: nodes
                .iter()
                .map(|node| tree_folder(&index, store, node, request.depth, request.folders_only))
                .collect(),
            documents: if request.folders_only {
                Vec::new()
            } else {
                documents
                    .iter()
                    .map(|document| index.document_summary(store, document))
                    .collect()
            },
            total_folders: totals.0,
            total_documents: totals.1,
            total_words: totals.2,
        })
    }

    /// Full-text search over the titles and the text of every document, best hit first,
    /// each with a snippet around the words that matched.
    pub fn search(&self, request: &SearchRequest) -> Result<SearchResults> {
        let session = self.reader()?;
        let store = &session.store;
        let index = Index::load(store)?;
        let limit = request.limit.unwrap_or(DEFAULT_SEARCH_LIMIT);
        // Only documents of one folder: every hit is needed to find the best of them.
        let within: Option<HashSet<FolderId>> = match &request.folder {
            Some(target) => index.live_folder_or_root(store, target)?.map(|folder| {
                let mut ids = HashSet::new();
                if let Some(node) = index.node(folder.id) {
                    subtree(node, &mut ids);
                }
                ids
            }),
            None => None,
        };
        let found = store.search(
            &request.query,
            if within.is_some() {
                limit.max(FOLDER_SEARCH_POOL)
            } else {
                limit
            },
        )?;
        let mut hits = Vec::new();
        for hit in found {
            if let Some(within) = &within
                && !hit.folder_id.is_some_and(|id| within.contains(&id))
            {
                continue;
            }
            if hits.len() == limit {
                break;
            }
            let meta = store.document_meta(hit.document_id)?;
            let parts: Vec<SnippetPart> = hit
                .snippet_parts()
                .into_iter()
                .map(|part| SnippetPart {
                    text: part.text.to_string(),
                    is_match: part.is_match,
                })
                .collect();
            hits.push(SearchHit {
                document: index.document_summary(store, &meta),
                snippet: parts.iter().map(|part| part.text.as_str()).collect(),
                snippet_parts: parts,
                rank: hit.rank,
            });
        }
        Ok(SearchResults {
            query: request.query.clone(),
            hits,
        })
    }

    /// The documents that were edited last, newest first.
    pub fn recent(&self, request: &RecentRequest) -> Result<DocumentList> {
        let session = self.reader()?;
        let index = Index::load(&session.store)?;
        let documents: Vec<DocumentMeta> = session
            .store
            .recent_documents(request.limit.unwrap_or(DEFAULT_RECENT_LIMIT))?;
        Ok(DocumentList {
            documents: documents
                .iter()
                .map(|meta| index.document_summary(&session.store, meta))
                .collect(),
        })
    }

    /// What is known about one document without its text: where it is, its counts and
    /// dates, its headings, how many comments and versions it has, its settings.
    pub fn info(&self, request: &InfoRequest) -> Result<DocumentInfo> {
        let session = self.reader()?;
        let store = &session.store;
        let index = Index::load(store)?;
        let meta = index.document(store, &request.document)?;
        if meta.kind == innerword_store::DocumentKind::Page {
            return crate::pages::info(store, &index, &meta);
        }
        let loaded = content::load(store, &meta)?;
        let document = &loaded.document;
        let stats = document.stats();
        let resolved = document.comments().filter(|thread| thread.resolved).count();
        Ok(DocumentInfo {
            document: index.document_summary(store, &loaded.meta),
            custom_title: loaded.custom_title,
            paragraphs: stats.paragraphs as u64,
            characters_no_spaces: stats.characters_no_spaces as u64,
            reading_minutes: stats.reading_minutes as u64,
            headings: headings(document)
                .iter()
                .map(|heading| heading.info(document))
                .collect(),
            open_comments: document.comment_count() - resolved,
            resolved_comments: resolved,
            versions: store.snapshots(meta.id)?.len(),
            settings: content::settings_info(document.settings()),
            page: None,
        })
    }

    /// What is known about one folder: where it is and what is in it.
    pub fn folder_info(&self, request: &FolderInfoRequest) -> Result<FolderInfo> {
        let session = self.reader()?;
        let store = &session.store;
        let index = Index::load(store)?;
        let folder = index.folder_or_root(store, &request.folder)?;
        let id = folder.as_ref().map(|folder| folder.id);
        let nodes = index.child_folders(id);
        let documents = index.child_documents(id);
        let mut totals = (0, documents.len() as u64, 0);
        totals.2 = documents.iter().map(|document| document.word_count).sum();
        for node in nodes {
            let (folders, documents, words) = contents(node);
            totals.0 += 1 + folders;
            totals.1 += documents;
            totals.2 += words;
        }
        Ok(FolderInfo {
            folder: folder
                .as_ref()
                .map(|folder| index.folder_summary(store, folder)),
            folders: nodes
                .iter()
                .map(|node| index.folder_summary(store, &node.folder))
                .collect(),
            documents: documents
                .iter()
                .map(|document| index.document_summary(store, document))
                .collect(),
            total_folders: totals.0,
            total_documents: totals.1,
            total_words: totals.2,
        })
    }
}
