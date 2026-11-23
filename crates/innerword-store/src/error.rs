//! The error type shared by every fallible store operation.

use std::fmt;

use crate::documents::{ContentStamp, DocumentKind};
use crate::images::FileId;

/// The kind of library item an error refers to.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum ItemKind {
    /// A folder.
    Folder,
    /// A document.
    Document,
    /// A version snapshot of a document.
    Snapshot,
}

impl fmt::Display for ItemKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ItemKind::Folder => "folder",
            ItemKind::Document => "document",
            ItemKind::Snapshot => "snapshot",
        })
    }
}

/// Everything that can go wrong in the store.
///
/// No store operation panics on IO or SQL failures; they surface as one of these variants.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// SQLite reported an error.
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// A file system operation (creating directories, backups) failed.
    #[error("file system error: {0}")]
    Io(#[from] std::io::Error),
    /// A setting value could not be encoded to or decoded from JSON.
    #[error("could not encode or decode a setting: {0}")]
    Json(#[from] serde_json::Error),
    /// The item does not exist, or (for operations that need a live item) is in the trash.
    #[error("{kind} {id} does not exist")]
    NotFound {
        /// What kind of item was looked up.
        kind: ItemKind,
        /// The raw id that was looked up.
        id: i64,
    },
    /// A trash operation (restore, purge) was called on an item that is not a top-level
    /// trash entry: either it is not trashed at all, or it was trashed together with a
    /// parent folder and must be restored or purged through that folder.
    #[error("{kind} {id} is not in the trash")]
    NotInTrash {
        /// What kind of item was given.
        kind: ItemKind,
        /// The raw id that was given.
        id: i64,
    },
    /// A folder move would put the folder inside itself or one of its descendants.
    #[error("cannot move a folder into itself or one of its subfolders")]
    FolderCycle,
    /// A folder name was empty or only whitespace.
    #[error("a folder name cannot be empty")]
    EmptyName,
    /// The database was written by a newer version of the app. It is left untouched.
    #[error(
        "this library was created by a newer version of Inner Word \
         (schema version {found}, this version supports up to {supported})"
    )]
    SchemaTooNew {
        /// The `user_version` found in the file.
        found: i64,
        /// The newest schema version this build knows.
        supported: i64,
    },
    /// The file is an SQLite database that does not belong to Inner Word.
    #[error("this file is not a library of Inner Word")]
    NotALibrary,
    /// SQLite refused to switch the database to write-ahead logging (the value is the
    /// journal mode it reported instead).
    #[error("could not enable write-ahead logging (journal mode is {0})")]
    JournalMode(String),
    /// Neither `INNER_WORD_DATA_DIR` nor `HOME` is set, so no data directory can be chosen.
    #[error("cannot find the data directory: neither INNER_WORD_DATA_DIR nor HOME is set")]
    NoDataDirectory,
    /// Content of one kind was to be saved over an item of another: a text over a page or
    /// a whiteboard ([`crate::Store::save_document`]), a page over a text or a whiteboard
    /// ([`crate::Store::save_page`]), a whiteboard over a text or a page
    /// ([`crate::Store::save_board`]). Nothing was written.
    #[error("{}", wrong_kind_message(*.id, *.kind, *.saving))]
    WrongKind {
        /// The raw id of the item.
        id: i64,
        /// What the item is.
        kind: DocumentKind,
        /// What the caller tried to save over it.
        saving: DocumentKind,
    },
    /// The content of a page or a whiteboard names files whose bytes the library does not
    /// have (`images.db`, nor the copy among the backups). Nothing was written.
    #[error("{}", files_missing_message(.ids))]
    FilesMissing {
        /// The files that are not there, in order.
        ids: Vec<FileId>,
    },
    /// A compare-and-swap save ([`crate::Store::save_board`] with an expected stamp) found
    /// the row's content changed: it is not the content the caller's scene descends from.
    /// Nothing was written.
    #[error("item {id} was changed since its content was read, so it was not saved over")]
    Stale {
        /// The raw id of the item.
        id: i64,
        /// The stamp of the content the row has now.
        stored: ContentStamp,
    },
}

fn wrong_kind_message(id: i64, kind: DocumentKind, saving: DocumentKind) -> String {
    let what = match saving {
        DocumentKind::Text => "the text of a document",
        DocumentKind::Page => "a page",
        DocumentKind::Board => "a whiteboard",
    };
    format!(
        "item {id} of the library is a {}: {what} cannot be saved over it",
        kind.noun()
    )
}

fn files_missing_message(ids: &[FileId]) -> String {
    let shown: Vec<String> = ids.iter().take(3).map(FileId::to_hex).collect();
    let more = ids.len().saturating_sub(shown.len());
    let mut message = match ids.len() {
        1 => "the item names a file that the library does not have: ".to_owned(),
        n => format!("the item names {n} files that the library does not have: "),
    };
    message.push_str(&shown.join(", "));
    if more > 0 {
        message.push_str(&format!(" and {more} more"));
    }
    message
}

/// Shorthand for `Result<T, StoreError>`.
pub type Result<T, E = StoreError> = std::result::Result<T, E>;

/// Turns "no rows" into [`StoreError::NotFound`] for the given item.
pub(crate) fn not_found(kind: ItemKind, id: i64) -> impl FnOnce(rusqlite::Error) -> StoreError {
    move |err| match err {
        rusqlite::Error::QueryReturnedNoRows => StoreError::NotFound { kind, id },
        other => StoreError::Sqlite(other),
    }
}
