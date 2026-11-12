//! Looking around: the tree, search, recent documents, what is known about one item, the
//! trash.

use crate::error::Result;
use crate::library::{Library, unfinished};
use crate::types::{
    DocumentInfo, DocumentList, FolderInfo, FolderInfoRequest, InfoRequest, RecentRequest,
    SearchRequest, SearchResults, TrashList, Tree, TreeRequest,
};

impl Library {
    /// The tree of folders and documents: all of it, or one folder, to a depth.
    pub fn tree(&self, request: &TreeRequest) -> Result<Tree> {
        let _ = request;
        unfinished("tree")
    }

    /// Full-text search over the titles and the text of every document, best hit first,
    /// each with a snippet around the words that matched.
    pub fn search(&self, request: &SearchRequest) -> Result<SearchResults> {
        let _ = request;
        unfinished("search")
    }

    /// The documents that were edited last, newest first.
    pub fn recent(&self, request: &RecentRequest) -> Result<DocumentList> {
        let _ = request;
        unfinished("recent")
    }

    /// What is known about one document without its text: where it is, its counts and
    /// dates, its headings, how many comments and versions it has, its settings.
    pub fn info(&self, request: &InfoRequest) -> Result<DocumentInfo> {
        let _ = request;
        unfinished("info")
    }

    /// What is known about one folder: where it is and what is in it.
    pub fn folder_info(&self, request: &FolderInfoRequest) -> Result<FolderInfo> {
        let _ = request;
        unfinished("folder_info")
    }

    /// What is in the trash, the entry that went there last first.
    pub fn trash(&self) -> Result<TrashList> {
        unfinished("trash")
    }
}
