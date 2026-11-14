//! The table of the operations that write: the one place where an operation gets its
//! method on [`Library`], its name on the channel to the app, and the function that
//! carries it out.
//!
//! Nothing writes to a library except through this table: [`Library::write`] is the only
//! way to a session that can write, and the table is the only caller of it. An operation
//! that is not in the table has no method, and one that is in it is routed through the
//! app whenever the app has the library open ([`crate::live`]).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::attach::Prepare;
use crate::error::{Error, Result};
use crate::library::{Library, Session};
use crate::types::*;
use crate::{comments, items, pages, versions, write_text};

fn as_json<T: Serialize>(result: T) -> Result<Value> {
    serde_json::to_value(result)
        .map_err(|error| Error::invalid(format!("the result cannot be written as JSON: {error}")))
}

macro_rules! operations {
    ($(
        $(#[$doc:meta])*
        $method:ident / $variant:ident ($request:ty) -> $done:ty = $run:path;
    )*) => {
        /// What a tool asks the app that has the library open to do: one line of the
        /// channel ([`crate::live`]).
        ///
        /// Besides the two that change nothing, there is one variant per operation of
        /// [`Library`] that writes, with the request of that operation as it is.
        #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
        #[serde(tag = "op", content = "request", rename_all = "snake_case")]
        // One per request, never in a collection: the request of a page is not boxed.
        #[allow(clippy::large_enum_variant)]
        pub enum Operation {
            /// Says whether the app takes changes from agents. Changes nothing, saves
            /// nothing.
            Hello,
            /// Save what is not saved yet, so that a read from the library sees the text
            /// as it is on screen.
            Flush,
            $(
                #[doc = concat!(
                    "[`Library::", stringify!($method), "`](crate::Library::",
                    stringify!($method), ")."
                )]
                $variant($request),
            )*
        }

        impl Operation {
            /// The names of the operations that write, in the order of the table: the
            /// names of their methods on [`Library`], which are their names on the
            /// channel too.
            pub const CHANGES: &'static [&'static str] = &[$(stringify!($method),)*];

            /// The name of the operation on the channel: `"hello"`, `"flush"`, or the
            /// name of the method on [`Library`] (`"edit"`, `"create_document"`).
            pub fn name(&self) -> &'static str {
                match self {
                    Operation::Hello => "hello",
                    Operation::Flush => "flush",
                    $(Operation::$variant(_) => stringify!($method),)*
                }
            }

            /// Dry run and author of an operation that writes; `None` for the two that
            /// do not.
            pub fn options(&self) -> Option<&WriteOptions> {
                match self {
                    Operation::Hello | Operation::Flush => None,
                    $(Operation::$variant(request) => Some(&request.write),)*
                }
            }

            /// Carries the operation out in `session`. The result is the result of the
            /// method on [`Library`], as JSON.
            pub(crate) fn run(&self, session: &mut Session<'_>) -> Result<Value> {
                match self {
                    Operation::Hello | Operation::Flush => Ok(Value::Null),
                    $(Operation::$variant(request) => as_json($run(session, request)?),)*
                }
            }
        }

        impl Library {
            $(
                $(#[$doc])*
                pub fn $method(&self, request: &$request) -> Result<$done> {
                    // What the request names of this Mac (an image file) is read here,
                    // in the process that was asked: the app never opens such a path.
                    let request = Prepare::prepared(request, self)?;
                    let request: &$request = &request;
                    self.write(
                        &request.write,
                        || Operation::$variant(request.clone()),
                        |session| $run(session, request),
                    )
                }
            )*
        }
    };
}

operations! {
    /// Creates a document from Markdown. See [`CreateDocumentRequest`].
    create_document / CreateDocument(CreateDocumentRequest) -> DocumentCreated
        = write_text::create_document;
    /// Replaces the whole text of a document with Markdown, changing as little of the
    /// stored document as the new text allows. See [`ReplaceDocumentRequest`].
    replace_document / ReplaceDocument(ReplaceDocumentRequest) -> ContentChanged
        = write_text::replace_document;
    /// Replaces an exact piece of text: once, or everywhere. See [`EditRequest`].
    edit / Edit(EditRequest) -> ContentChanged = write_text::edit;
    /// Inserts Markdown as new blocks: at the end (append), at the start, or at a heading.
    /// See [`InsertRequest`].
    insert / Insert(InsertRequest) -> ContentChanged = write_text::insert;
    /// Replaces the section under a heading with Markdown. See [`ReplaceSectionRequest`].
    replace_section / ReplaceSection(ReplaceSectionRequest) -> ContentChanged
        = write_text::replace_section;
    /// Adds an image from a file on this Mac to a document, as a block of its own. The
    /// file is read here, by whoever asks, and travels with the request. See
    /// [`AddImageRequest`].
    add_image / AddImage(AddImageRequest) -> ContentChanged = write_text::add_image;
    /// Adds a page: a web page from a file or a folder on this Mac, kept whole and
    /// offline with everything it loads. The file or folder is read, what the page names
    /// on the web is fetched and the page is checked here, by whoever asks, and the page
    /// travels with the request. With a source address the page there is updated, or
    /// left as it is when it is the same. See [`AddPageRequest`].
    add_page / AddPage(AddPageRequest) -> PageAdded = pages::add_page;

    /// Renames a document, or gives its title back to its text. See
    /// [`RenameDocumentRequest`].
    rename_document / RenameDocument(RenameDocumentRequest) -> DocumentChanged
        = items::rename_document;
    /// Moves a document to another folder, to another place in its folder, or both. See
    /// [`MoveDocumentRequest`].
    move_document / MoveDocument(MoveDocumentRequest) -> DocumentChanged
        = items::move_document;
    /// Copies a document: text, comments and settings. See [`DuplicateDocumentRequest`].
    /// In the result `before` is the original.
    duplicate_document / DuplicateDocument(DuplicateDocumentRequest) -> DocumentChanged
        = items::duplicate_document;
    /// Moves a document to the trash.
    trash_document / TrashDocument(TrashDocumentRequest) -> DocumentChanged
        = items::trash_document;
    /// Brings a document back from the trash.
    restore_document / RestoreDocument(RestoreDocumentRequest) -> DocumentChanged
        = items::restore_document;
    /// Deletes a document for good, with its versions. Refused with
    /// [`Error::ConfirmationRequired`](crate::Error::ConfirmationRequired) unless the
    /// request says `permanently`.
    delete_document / DeleteDocument(DeleteDocumentRequest) -> DocumentChanged
        = items::delete_document;

    /// Creates a folder, with the folders above it that are missing. See
    /// [`CreateFolderRequest`].
    create_folder / CreateFolder(CreateFolderRequest) -> FolderCreated = items::create_folder;
    /// Renames a folder.
    rename_folder / RenameFolder(RenameFolderRequest) -> FolderChanged = items::rename_folder;
    /// Moves a folder, with everything in it. See [`MoveFolderRequest`].
    move_folder / MoveFolder(MoveFolderRequest) -> FolderChanged = items::move_folder;
    /// Moves a folder to the trash, with everything in it.
    trash_folder / TrashFolder(TrashFolderRequest) -> FolderChanged = items::trash_folder;
    /// Brings a folder back from the trash, with everything that went there with it.
    restore_folder / RestoreFolder(RestoreFolderRequest) -> FolderChanged
        = items::restore_folder;
    /// Deletes a folder for good, with every document in it. Refused with
    /// [`Error::ConfirmationRequired`](crate::Error::ConfirmationRequired) unless the
    /// request says `permanently`.
    delete_folder / DeleteFolder(DeleteFolderRequest) -> FolderChanged = items::delete_folder;
    /// Deletes everything in the trash for good. Refused with
    /// [`Error::ConfirmationRequired`](crate::Error::ConfirmationRequired) unless the
    /// request says `permanently`.
    empty_trash / EmptyTrash(EmptyTrashRequest) -> TrashEmptied = items::empty_trash;

    /// Starts a comment thread on an exact quote of the text. See [`AddCommentRequest`].
    add_comment / AddComment(AddCommentRequest) -> CommentChanged = comments::add_comment;
    /// Adds a reply to a comment thread.
    reply / Reply(ReplyRequest) -> CommentChanged = comments::reply;
    /// Marks a comment thread as resolved.
    resolve_comment / ResolveComment(CommentRequest) -> CommentChanged
        = comments::resolve_comment;
    /// Opens a resolved comment thread again.
    reopen_comment / ReopenComment(CommentRequest) -> CommentChanged
        = comments::reopen_comment;
    /// Deletes a comment thread, or one reply of it. See [`DeleteCommentRequest`].
    delete_comment / DeleteComment(DeleteCommentRequest) -> CommentChanged
        = comments::delete_comment;

    /// Makes a version the text of its document again, after keeping the text as it is
    /// now as a version. See [`RestoreVersionRequest`].
    restore_version / RestoreVersion(RestoreVersionRequest) -> VersionRestored
        = versions::restore_version;
}
