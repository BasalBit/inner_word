//! Writing text: creating a document, replacing its text, editing it in place.

use crate::error::Result;
use crate::library::{Library, unfinished};
use crate::types::{
    ContentChanged, CreateDocumentRequest, DocumentCreated, DocumentText, EditRequest,
    FindRequest, FindResults, InsertRequest, ReadRequest, ReplaceDocumentRequest,
    ReplaceSectionRequest,
};

impl Library {
    /// The text of a document, or of one section of it: as Markdown, as plain text or as
    /// the native JSON, optionally with its comment threads.
    pub fn read(&self, request: &ReadRequest) -> Result<DocumentText> {
        let _ = request;
        unfinished("read")
    }

    /// Where a piece of text occurs in a document: what an edit or a comment with this
    /// text would find, without changing anything.
    pub fn find(&self, request: &FindRequest) -> Result<FindResults> {
        let _ = request;
        unfinished("find")
    }

    /// Creates a document from Markdown. See [`CreateDocumentRequest`].
    pub fn create_document(&self, request: &CreateDocumentRequest) -> Result<DocumentCreated> {
        let _ = request;
        unfinished("create_document")
    }

    /// Replaces the whole text of a document with Markdown, changing as little of the
    /// stored document as the new text allows. See [`ReplaceDocumentRequest`].
    pub fn replace_document(&self, request: &ReplaceDocumentRequest) -> Result<ContentChanged> {
        let _ = request;
        unfinished("replace_document")
    }

    /// Replaces an exact piece of text: once, or everywhere. See [`EditRequest`].
    pub fn edit(&self, request: &EditRequest) -> Result<ContentChanged> {
        let _ = request;
        unfinished("edit")
    }

    /// Inserts Markdown as new blocks: at the end (append), at the start, or at a heading.
    /// See [`InsertRequest`].
    pub fn insert(&self, request: &InsertRequest) -> Result<ContentChanged> {
        let _ = request;
        unfinished("insert")
    }

    /// Replaces the section under a heading with Markdown. See [`ReplaceSectionRequest`].
    pub fn replace_section(&self, request: &ReplaceSectionRequest) -> Result<ContentChanged> {
        let _ = request;
        unfinished("replace_section")
    }
}
