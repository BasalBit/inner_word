//! Names, places and the trash: documents and folders as items of the library.

use crate::error::Result;
use crate::library::{Library, unfinished};
use crate::types::{
    CreateFolderRequest, DeleteDocumentRequest, DeleteFolderRequest, DocumentChanged,
    DuplicateDocumentRequest, EmptyTrashRequest, FolderChanged, FolderCreated,
    MoveDocumentRequest, MoveFolderRequest, RenameDocumentRequest, RenameFolderRequest,
    RestoreDocumentRequest, RestoreFolderRequest, TrashDocumentRequest, TrashEmptied,
    TrashFolderRequest,
};

impl Library {
    /// Renames a document, or gives its title back to its text. See
    /// [`RenameDocumentRequest`].
    pub fn rename_document(&self, request: &RenameDocumentRequest) -> Result<DocumentChanged> {
        let _ = request;
        unfinished("rename_document")
    }

    /// Moves a document to another folder, to another place in its folder, or both. See
    /// [`MoveDocumentRequest`].
    pub fn move_document(&self, request: &MoveDocumentRequest) -> Result<DocumentChanged> {
        let _ = request;
        unfinished("move_document")
    }

    /// Copies a document: text, comments and settings. See [`DuplicateDocumentRequest`].
    pub fn duplicate_document(
        &self,
        request: &DuplicateDocumentRequest,
    ) -> Result<DocumentChanged> {
        let _ = request;
        unfinished("duplicate_document")
    }

    /// Moves a document to the trash.
    pub fn trash_document(&self, request: &TrashDocumentRequest) -> Result<DocumentChanged> {
        let _ = request;
        unfinished("trash_document")
    }

    /// Brings a document back from the trash.
    pub fn restore_document(&self, request: &RestoreDocumentRequest) -> Result<DocumentChanged> {
        let _ = request;
        unfinished("restore_document")
    }

    /// Deletes a document for good, with its versions. Refused with
    /// [`Error::ConfirmationRequired`](crate::Error::ConfirmationRequired) unless the
    /// request says `permanently`.
    pub fn delete_document(&self, request: &DeleteDocumentRequest) -> Result<DocumentChanged> {
        let _ = request;
        unfinished("delete_document")
    }

    /// Creates a folder, with the folders above it that are missing. See
    /// [`CreateFolderRequest`].
    pub fn create_folder(&self, request: &CreateFolderRequest) -> Result<FolderCreated> {
        let _ = request;
        unfinished("create_folder")
    }

    /// Renames a folder.
    pub fn rename_folder(&self, request: &RenameFolderRequest) -> Result<FolderChanged> {
        let _ = request;
        unfinished("rename_folder")
    }

    /// Moves a folder, with everything in it. See [`MoveFolderRequest`].
    pub fn move_folder(&self, request: &MoveFolderRequest) -> Result<FolderChanged> {
        let _ = request;
        unfinished("move_folder")
    }

    /// Moves a folder to the trash, with everything in it.
    pub fn trash_folder(&self, request: &TrashFolderRequest) -> Result<FolderChanged> {
        let _ = request;
        unfinished("trash_folder")
    }

    /// Brings a folder back from the trash, with everything that went there with it.
    pub fn restore_folder(&self, request: &RestoreFolderRequest) -> Result<FolderChanged> {
        let _ = request;
        unfinished("restore_folder")
    }

    /// Deletes a folder for good, with every document in it. Refused with
    /// [`Error::ConfirmationRequired`](crate::Error::ConfirmationRequired) unless the
    /// request says `permanently`.
    pub fn delete_folder(&self, request: &DeleteFolderRequest) -> Result<FolderChanged> {
        let _ = request;
        unfinished("delete_folder")
    }

    /// Deletes everything in the trash for good. Refused with
    /// [`Error::ConfirmationRequired`](crate::Error::ConfirmationRequired) unless the
    /// request says `permanently`.
    pub fn empty_trash(&self, request: &EmptyTrashRequest) -> Result<TrashEmptied> {
        let _ = request;
        unfinished("empty_trash")
    }
}
