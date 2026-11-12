//! From a [`Target`] to the document or folder it names.

use crate::error::Result;
use crate::library::{Library, unfinished};
use crate::types::{DocumentSummary, FolderSummary, Item, Target};

impl Library {
    /// The document `target` names: by id (one in the trash is found too) or by its path
    /// among the documents that are not in the trash. Fails with
    /// [`Error::NotFound`](crate::Error::NotFound) or
    /// [`Error::Ambiguous`](crate::Error::Ambiguous).
    pub fn resolve_document(&self, target: &Target) -> Result<DocumentSummary> {
        let _ = target;
        unfinished("resolve_document")
    }

    /// The folder `target` names: by id (one in the trash is found too) or by its path
    /// among the folders that are not in the trash. The top of the library
    /// ([`Target::root`]) is not a folder: it fails with
    /// [`Error::Invalid`](crate::Error::Invalid).
    pub fn resolve_folder(&self, target: &Target) -> Result<FolderSummary> {
        let _ = target;
        unfinished("resolve_folder")
    }

    /// The document or the folder `target` names, for a front end whose argument may be
    /// either. A path that fits a document and a folder, or a bare id that both a
    /// document and a folder have, fails with
    /// [`Error::Ambiguous`](crate::Error::Ambiguous) (`doc:12` and `folder:12` are
    /// exact). With `in_trash` the name is looked up among the entries of the trash
    /// instead: what a restore or a permanent deletion names.
    pub fn resolve(&self, target: &Target, in_trash: bool) -> Result<Item> {
        let _ = (target, in_trash);
        unfinished("resolve")
    }
}
