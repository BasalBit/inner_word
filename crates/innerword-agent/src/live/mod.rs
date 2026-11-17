//! Changes while the app is open: the channel between a tool and the running app.
//!
//! The app keeps the open document in memory, with text that is not saved yet, and its own
//! picture of the library. A tool that wrote to the library underneath it would lose one
//! side's work. So **while the app is open, a change by an agent is carried out by the
//! app**: the tool hands it the operation, the app does it between two key presses, shows
//! it, and answers. While the app is closed the tool writes directly, as it always did.
//!
//! Every operation of [`Library`](crate::Library) that writes goes this way by itself
//! (`Library::write`, the table in `operations.rs`); the command line tool and the MCP
//! server know nothing of it.
//!
//! # The transport
//!
//! A Unix domain socket that the app opens once it holds the lock of its data directory
//! ([`Listener::bind`]) and removes when it quits. It lies in a directory only the user
//! can enter (`inner-word-<uid>` in the user's temporary directory), under a short name:
//! the path of a socket holds about a hundred bytes, and a data directory can be longer
//! than that. The tool finds it through the lock file of the data directory, whose second
//! line names it (`innerword_store::instance_lock::app_channel`). The lock says whether an app
//! is running; a socket left behind by an app that crashed is never looked at, because the
//! lock that app held is gone. macOS clears out of that directory what was not used for
//! three days, so the listener renews the times of its socket every hour.
//!
//! An app that holds the lock and names no socket is one that takes no changes (every
//! version before 0.6.0), unless it has only just taken the lock: a tool waits for the
//! channel of an app that started less than five seconds ago. A tool that finds an app
//! with a channel hands it the change at once and never tries the app's lock.
//!
//! # The protocol
//!
//! One request and one answer per connection, each one line of JSON.
//!
//! ```text
//! {"version":3,"wait_ms":10000,"op":"edit","request":{"document":"doc:3","old_text":"a","new_text":"b","author":"Claude"}}
//! {"version":3,"answer":"done","result":{ ...what Library::edit returns... }}
//! {"version":3,"answer":"failed","error":{"code":"no_match","message":"...", "detail":{...}}}
//! {"version":3,"answer":"refused","refusal":{"why":"switched_off"}}
//! ```
//!
//! A [`Request`] is one [`Operation`]: an operation of [`Library`](crate::Library) that
//! writes, with its request as it is (who is asking, dry run, the options), or one of the
//! two that change nothing: `flush` (save what is not saved yet, so that a read sees the
//! text on screen) and `hello`. An [`Answer`] is the result of the operation exactly as
//! the tool would have had it from a direct write, its error exactly as this crate words
//! it, or a [`Refusal`]: the app did not carry the operation out.
//!
//! `version`, `answer` and `refusal.why` keep their meaning in every later version of the
//! protocol, so two versions can always tell each other that they do not fit
//! ([`Refusal::Protocol`]).
//!
//! **Version 1** (0.6.0) is everything above. **Version 2** added images: the requests
//! that take Markdown (`create_document`, `replace_document`, `edit`, `insert`,
//! `replace_section`) and the new `add_image` carry `images`, the image files the text
//! names as the bytes the library stores, base64 in the line
//! ([`AttachedImage`](crate::AttachedImage)). The tool reads the files; the app never
//! opens a path an agent named, and stores what the request brings. A request reads past
//! a field it does not know, so an app of version 1 would have taken such a request,
//! kept every image as a link and answered "done": that is why the number changed, and
//! an app and a tool of different versions refuse each other whole
//! ([`Refusal::Protocol`], which the tool words for an app of another version). The
//! images of one request weigh [`MAX_ATTACHED_BYTES`] together at most; a line may be
//! 256 MB long.
//!
//! **Version 3** added pages: `add_page`, whose request carries the page the tool read,
//! fetched and checked (`prepared`, [`PreparedPage`](crate::PreparedPage)), its files as
//! base64, one page per request and [`MAX_PAGE_BYTES`] at most. The tool does all of the
//! preparing; the app never reads a path an agent named and never fetches for an agent.
//! The app's listener stores the files of such a request before it queues it
//! ([`store_page_files`]), so that the main thread only writes the row, and refuses it at
//! once while the owner's switch is off ([`carries_page_files`]). An app of version 2
//! does not know `add_page`, so the number changed again. The tool writes the whole line
//! before it connects, and the listener reads the version of a line before the rest of
//! it.
//!
//! `wait_ms` is how long the tool waits for the app to take the request up. An app that
//! is busy for longer answers [`Refusal::TooLate`] at that moment by itself, and does not
//! carry the request out when it gets to it ([`Incoming::begin`]): a change is never made
//! after the tool was told that it was not.
//!
//! # The app's half
//!
//! ```no_run
//! use innerword_agent::live::{self, Listener};
//! # fn store() -> innerword_store::Store { innerword_store::Store::open_in_memory().unwrap() }
//! # let data_dir = std::path::Path::new("a data directory");
//! // After taking the lock of the data directory:
//! let listener = Listener::bind(data_dir)?;
//! let closer = listener.closer(); // for the quit hook: closer.close()
//! std::thread::spawn(move || {
//!     while let Some(incoming) = listener.accept() {
//!         // Hand `incoming` to the main thread. There, in one go:
//!         # let mut store = store();
//!         if !incoming.begin() {
//!             continue; // the tool stopped waiting and was told so: do nothing
//!         }
//!         // 1. save what is not saved in the open document
//!         // 2. carry the request out on the library's own store
//!         let executed = live::execute(&mut store, incoming.request());
//!         // 3. bring the window up to date from `executed.changes`, 4. tell the writer
//!         // 5. answer
//!         incoming.answer(executed.answer).ok();
//!     }
//! });
//! # Ok::<(), std::io::Error>(())
//! ```

pub(crate) mod client;
mod socket;

pub(crate) use socket::private_dir;

use std::time::Duration;

use innerword_store::{DocumentId, FolderId, Store};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Candidate, Error, ErrorKind, ItemKind, Occurrence};
use crate::library::Session;
use crate::types::PreparedPage;

pub use crate::error::Refusal;
pub use crate::operations::Operation;
pub use socket::{Closer, Incoming, Listener};

/// The version of the protocol this build speaks: 3 since a request can bring a page
/// with its files (the module's notes say what each version added).
pub const PROTOCOL_VERSION: u32 = 3;

/// What the files of the page one request brings may weigh, in bytes as the library
/// stores them (before they are written as base64): 64 MB, the most a page can be. The
/// tool refuses more before it sends anything, and the app refuses it again.
pub const MAX_PAGE_BYTES: usize = 64 * 1024 * 1024;

/// What the images of one request may weigh together, in bytes as the library stores
/// them (before they are written as base64): 32 MB, two pictures of the largest size
/// the library holds. A request with more fails, in the process that was asked and
/// again where it is carried out, with a sentence that says to add them in several
/// requests.
pub const MAX_ATTACHED_BYTES: usize = 32 * 1024 * 1024;

/// How long a tool waits for the app to take a change up, unless the
/// [`Library`](crate::Library) was told otherwise
/// ([`with_app_patience`](crate::Library::with_app_patience)): 10 seconds. The app
/// answers a change in a fraction of a second; it waits a few seconds itself for a text
/// that is being composed with an input method.
pub const ANSWER_PATIENCE: Duration = Duration::from_secs(10);

/// How long a read waits for the app to save what is not saved yet
/// ([`with_save_patience`](crate::Library::with_save_patience)): one second. After it the
/// read goes ahead on what is saved.
pub const SAVE_PATIENCE: Duration = Duration::from_secs(1);

/// How long a tool goes on waiting for the answer once the app has taken a change up (or
/// would have been told that it was too late).
pub(crate) const ANSWER_GRACE: Duration = Duration::from_secs(3);

/// How long a tool waits for an app that said it is quitting to be gone, before it writes
/// directly.
pub(crate) const QUIT_PATIENCE: Duration = Duration::from_secs(5);

/// How long a tool waits for an app that does not listen where it said to be gone, or to
/// say where it listens now.
pub(crate) const GONE_PATIENCE: Duration = Duration::from_millis(500);

/// How long after it took the lock of its data directory an app is given to open its
/// channel. An app that holds the lock for longer than this and names no channel is one
/// that takes no changes ([`Refusal::OldVersion`]); a tool that meets a younger one waits
/// for the rest of this time.
pub(crate) const STARTING_GRACE: Duration = Duration::from_secs(5);

/// The longest time the app keeps a request waiting, whatever the request says
/// ([`Request::wait_ms`]): an hour.
pub(crate) const LONGEST_WAIT: Duration = Duration::from_secs(3600);

/// The setting of the library that is the owner's switch "Agents can make changes while
/// Inner Word is open": a bool, on when it is missing. [`execute`] reads it
/// ([`changes_allowed`]); the app writes it.
pub const CHANGES_SETTING: &str = "agents.changes_while_open";

/// What that switch is called, in the app's Agents panel and in everything the tool says
/// about it: one name everywhere.
pub const CHANGES_SWITCH: &str = "Agents can make changes while Inner Word is open";

/// Whether the owner lets agents make changes while the app is open: the setting
/// [`CHANGES_SETTING`] of `store`, on unless it was switched off.
pub fn changes_allowed(store: &Store) -> bool {
    store.setting_or(CHANGES_SETTING, true).unwrap_or(true)
}

// ----- the request -------------------------------------------------------------------------

/// What a tool sends to the app: one operation, and how long it waits.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Request {
    /// The version of the protocol the sender speaks.
    pub version: u32,
    /// How long the sender waits for the app to take the request up, in milliseconds.
    pub wait_ms: u64,
    /// What to do.
    #[serde(flatten)]
    pub operation: Operation,
}

impl Request {
    /// A request of this build for `operation`, by a sender that waits `patience`.
    pub fn new(operation: Operation, patience: Duration) -> Request {
        Request {
            version: PROTOCOL_VERSION,
            wait_ms: patience.as_millis().try_into().unwrap_or(u64::MAX),
            operation,
        }
    }

    /// What to do.
    pub fn operation(&self) -> &Operation {
        &self.operation
    }

    /// The name of the operation ([`Operation::name`]): `"edit"`, `"flush"`.
    pub fn name(&self) -> &'static str {
        self.operation.name()
    }

    /// Who is asking: the name the agent gave (`--author`), `None` when it gave none.
    pub fn author(&self) -> Option<&str> {
        self.operation
            .options()
            .and_then(|options| options.author.as_deref())
            .map(str::trim)
            .filter(|name| !name.is_empty())
    }

    /// Whether this is the dry run of an operation that writes: it changes nothing.
    pub fn is_dry_run(&self) -> bool {
        self.operation
            .options()
            .is_some_and(|options| options.dry_run)
    }

    /// Whether this is "save what is not saved yet".
    pub fn is_flush(&self) -> bool {
        self.operation == Operation::Flush
    }

    /// Whether carrying this out may change the library: an operation that writes, and
    /// not its dry run. False for `flush` and `hello`.
    pub fn changes_library(&self) -> bool {
        self.operation
            .options()
            .is_some_and(|options| !options.dry_run)
    }
}

/// A request as it is sent, without a copy of the operation (which may hold a book).
#[derive(Serialize)]
pub(crate) struct RequestLine<'a> {
    pub(crate) version: u32,
    pub(crate) wait_ms: u64,
    #[serde(flatten)]
    pub(crate) operation: &'a Operation,
}

/// Whether the request is an add_page that is not a dry run and carries blobs: the
/// listener refuses it at once (Refusal::SwitchedOff) while the owner's switch is off.
pub fn carries_page_files(request: &Request) -> bool {
    match &request.operation {
        Operation::AddPage(add) => {
            !add.write.dry_run
                && matches!(&add.prepared, Some(PreparedPage::Page(page)) if !page.blobs.is_empty())
        }
        _ => false,
    }
}

/// For the app's listener thread, before it queues a request (and only while the
/// switch is on): stores the blobs of an add_page that is not a dry run through
/// `files`, each after checking that `FileId::of(data)` is its `file`, empties their
/// data, marks them stored. Any other request is left as it is.
///
/// A blob that fails the check stops the storing with [`Error::Invalid`] (answer the
/// request with it: what is stored by then is named by nothing, and goes with the
/// collector). The main thread then writes the row, and the store refuses it when a
/// file is not there after all (`StoreError::FilesMissing`).
pub fn store_page_files(
    request: &mut Request,
    files: &mut innerword_store::ImageFile,
) -> std::result::Result<(), Error> {
    let Operation::AddPage(add) = &mut request.operation else {
        return Ok(());
    };
    if add.write.dry_run {
        return Ok(());
    }
    let Some(PreparedPage::Page(page)) = &mut add.prepared else {
        return Ok(());
    };
    let total: u64 = page.blobs.iter().map(|blob| blob.size).sum();
    if total > MAX_PAGE_BYTES as u64 {
        return Err(Error::invalid(crate::pages::too_heavy_sentence(total)));
    }
    for blob in &mut page.blobs {
        if blob.stored {
            continue;
        }
        crate::pages::check_blob(blob)?;
        let stored = files.put_file(&blob.data)?;
        if stored.id.to_hex() != blob.file {
            return Err(Error::invalid(crate::pages::DAMAGED_BLOB));
        }
        blob.data = Vec::new();
        blob.stored = true;
    }
    Ok(())
}

// ----- the answer --------------------------------------------------------------------------

/// What the app answers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "answer", rename_all = "snake_case")]
pub enum Answer {
    /// The operation was carried out. `result` is what the method of
    /// [`Library`](crate::Library) returns, as JSON; for `flush` and `hello` an
    /// [`AppState`].
    Done {
        /// The result.
        result: Value,
    },
    /// The operation failed the way it fails for a tool that writes directly (the text
    /// was not found, a name fits several items).
    Failed {
        /// The failure.
        error: WireError,
    },
    /// The app did not carry the operation out.
    Refused {
        /// Why not.
        refusal: Refusal,
    },
}

impl Answer {
    /// The answer of an app that does not carry the request out. Of the reasons, the app
    /// gives [`Refusal::SwitchedOff`] (though [`execute`] does that by itself),
    /// [`Refusal::Composing`], [`Refusal::CouldNotSave`], [`Refusal::Quitting`] and
    /// [`Refusal::NoLibrary`].
    pub fn refused(refusal: Refusal) -> Answer {
        Answer::Refused { refusal }
    }

    /// The answer to `flush` and to `hello`, by an app that does not call [`execute`] for
    /// them: what was unsaved is saved, and whether the app takes changes.
    pub fn state(takes_changes: bool) -> Answer {
        Answer::Done {
            result: serde_json::to_value(AppState::of(takes_changes)).unwrap_or(Value::Null),
        }
    }

    /// A failure of the operation.
    pub fn failed(error: &Error) -> Answer {
        Answer::Failed {
            error: WireError::of(error),
        }
    }

    /// Whether the operation was carried out.
    pub fn is_done(&self) -> bool {
        matches!(self, Answer::Done { .. })
    }
}

/// An answer as it is sent.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct AnswerLine {
    pub(crate) version: u32,
    #[serde(flatten)]
    pub(crate) answer: Answer,
}

/// What the app says about itself in answer to `flush` and `hello`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppState {
    /// Whether the app takes changes from agents (the owner's switch,
    /// [`CHANGES_SETTING`]).
    pub takes_changes: bool,
    /// The version of the app.
    #[serde(default)]
    pub app_version: String,
}

impl AppState {
    fn of(takes_changes: bool) -> AppState {
        AppState {
            takes_changes,
            app_version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

/// An [`Error`] of this crate on its way from the app to the tool, where it becomes the
/// same error again: the same kind, the same message, the same candidates and
/// occurrences. What cannot travel as it is (an error of the database or of the file
/// system) arrives as [`Error::InApp`] with the message it had.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WireError {
    /// The kind of failure ([`ErrorKind::code`]).
    pub code: String,
    /// The message, as [`Error`] displays it.
    pub message: String,
    /// The error itself, for a tool of the same version.
    #[serde(default)]
    detail: Value,
}

/// The variants of [`Error`] that travel as they are.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
enum Detail {
    Invalid {
        message: String,
    },
    NotFound {
        kind: ItemKind,
        reference: String,
        hint: Option<String>,
    },
    Ambiguous {
        kind: ItemKind,
        reference: String,
        candidates: Vec<Candidate>,
    },
    NoMatch {
        text: String,
        hint: Option<String>,
    },
    NotUnique {
        text: String,
        count: usize,
        occurrences: Vec<Occurrence>,
    },
    ConfirmationRequired {
        what: String,
    },
    InTrash {
        kind: ItemKind,
        id: i64,
        name: String,
    },
    NotInTrash {
        kind: ItemKind,
        id: i64,
        name: String,
        inside: Option<String>,
    },
    Unreadable {
        id: i64,
        name: String,
        reason: String,
    },
    FileExists {
        path: std::path::PathBuf,
    },
    PageIncomplete {
        name: String,
        problems: Vec<WireProblem>,
    },
    WrongKind {
        id: i64,
        name: String,
        is_page: bool,
        /// An answer without it is about a text operation.
        #[serde(default)]
        export: bool,
    },
    Failed {
        message: String,
    },
}

/// A [`innerword_page::Problem`] as it travels.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct WireProblem {
    what: String,
    reason: String,
}

impl WireError {
    /// `error`, ready to travel.
    pub fn of(error: &Error) -> WireError {
        let detail = match error {
            Error::Invalid { message } => Some(Detail::Invalid {
                message: message.clone(),
            }),
            Error::NotFound {
                kind,
                reference,
                hint,
            } => Some(Detail::NotFound {
                kind: *kind,
                reference: reference.clone(),
                hint: hint.clone(),
            }),
            Error::Ambiguous {
                kind,
                reference,
                candidates,
            } => Some(Detail::Ambiguous {
                kind: *kind,
                reference: reference.clone(),
                candidates: candidates.clone(),
            }),
            Error::NoMatch { text, hint } => Some(Detail::NoMatch {
                text: text.clone(),
                hint: hint.clone(),
            }),
            Error::NotUnique {
                text,
                count,
                occurrences,
            } => Some(Detail::NotUnique {
                text: text.clone(),
                count: *count,
                occurrences: occurrences.clone(),
            }),
            Error::ConfirmationRequired { what } => {
                Some(Detail::ConfirmationRequired { what: what.clone() })
            }
            Error::InTrash { kind, id, name } => Some(Detail::InTrash {
                kind: *kind,
                id: *id,
                name: name.clone(),
            }),
            Error::NotInTrash {
                kind,
                id,
                name,
                inside,
            } => Some(Detail::NotInTrash {
                kind: *kind,
                id: *id,
                name: name.clone(),
                inside: inside.clone(),
            }),
            Error::Unreadable { id, name, reason } => Some(Detail::Unreadable {
                id: *id,
                name: name.clone(),
                reason: reason.clone(),
            }),
            Error::FileExists { path } => Some(Detail::FileExists { path: path.clone() }),
            Error::PageIncomplete { name, problems } => Some(Detail::PageIncomplete {
                name: name.clone(),
                problems: problems
                    .iter()
                    .map(|problem| WireProblem {
                        what: problem.what.clone(),
                        reason: problem.reason.clone(),
                    })
                    .collect(),
            }),
            Error::WrongKind {
                id,
                name,
                is_page,
                export,
            } => Some(Detail::WrongKind {
                id: *id,
                name: name.clone(),
                is_page: *is_page,
                export: *export,
            }),
            Error::Failed { message } => Some(Detail::Failed {
                message: message.clone(),
            }),
            // What the app cannot run into while it carries an operation out, and what
            // cannot be taken apart: the kind and the message travel.
            Error::AppIsOpen { .. }
            | Error::Busy { .. }
            | Error::LibraryNotFound { .. }
            | Error::OutdatedLibrary { .. }
            | Error::InApp { .. }
            | Error::Io { .. }
            | Error::Store(_)
            | Error::Pictures { .. } => None,
        };
        WireError {
            code: error.code().to_string(),
            message: error.to_string(),
            detail: detail
                .and_then(|detail| serde_json::to_value(detail).ok())
                .unwrap_or(Value::Null),
        }
    }

    /// The error it was. One that this version cannot take apart (a kind from a newer
    /// app) keeps its kind, as far as this version knows it, and its message.
    pub fn into_error(self) -> Error {
        match serde_json::from_value::<Detail>(self.detail) {
            Ok(Detail::Invalid { message }) => Error::Invalid { message },
            Ok(Detail::NotFound {
                kind,
                reference,
                hint,
            }) => Error::NotFound {
                kind,
                reference,
                hint,
            },
            Ok(Detail::Ambiguous {
                kind,
                reference,
                candidates,
            }) => Error::Ambiguous {
                kind,
                reference,
                candidates,
            },
            Ok(Detail::NoMatch { text, hint }) => Error::NoMatch { text, hint },
            Ok(Detail::NotUnique {
                text,
                count,
                occurrences,
            }) => Error::NotUnique {
                text,
                count,
                occurrences,
            },
            Ok(Detail::ConfirmationRequired { what }) => Error::ConfirmationRequired { what },
            Ok(Detail::InTrash { kind, id, name }) => Error::InTrash { kind, id, name },
            Ok(Detail::NotInTrash {
                kind,
                id,
                name,
                inside,
            }) => Error::NotInTrash {
                kind,
                id,
                name,
                inside,
            },
            Ok(Detail::Unreadable { id, name, reason }) => Error::Unreadable { id, name, reason },
            Ok(Detail::FileExists { path }) => Error::FileExists { path },
            Ok(Detail::PageIncomplete { name, problems }) => Error::PageIncomplete {
                name,
                problems: problems
                    .into_iter()
                    .map(|problem| innerword_page::Problem::new(problem.what, problem.reason))
                    .collect(),
            },
            Ok(Detail::WrongKind {
                id,
                name,
                is_page,
                export,
            }) => Error::WrongKind {
                id,
                name,
                is_page,
                export,
            },
            Ok(Detail::Failed { message }) => Error::Failed { message },
            Err(_) => Error::InApp {
                kind: ErrorKind::ALL
                    .into_iter()
                    .find(|kind| kind.code() == self.code)
                    .unwrap_or(ErrorKind::Failed),
                message: self.message,
            },
        }
    }
}

// ----- what changed ------------------------------------------------------------------------

/// One thing an operation changed in the library: what the app has to bring up to date.
///
/// [`execute`] returns them in the order they happened. Three rules:
///
/// - A change of the stored content of a document is always in the list, as
///   [`Change::DocumentText`], [`Change::DocumentComments`] or
///   [`Change::DocumentSettings`]: any of them means "load the document again".
/// - A title that changed because the text changed has no entry of its own: read the
///   title again for every `DocumentText`. [`Change::DocumentRenamed`] is a name somebody
///   chose (a rename, a title given at creation) or gave back to the text.
/// - What goes with a folder (its documents and folders, to the trash, back from it, or
///   for good) has no entries of its own.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "change", rename_all = "snake_case")]
pub enum Change {
    /// A document was created (also by copying one: the id is the copy).
    DocumentCreated {
        /// The document.
        id: DocumentId,
    },
    /// The stored content of a document changed: its text, its formatting, a restored
    /// version.
    DocumentText {
        /// The document.
        id: DocumentId,
    },
    /// Comment threads of a document changed: one was added, answered, resolved, reopened
    /// or deleted, or lost its place through a change of the text (then beside a
    /// `DocumentText`).
    DocumentComments {
        /// The document.
        id: DocumentId,
    },
    /// The document's own settings changed (a restored version had other ones; then
    /// beside a `DocumentText`).
    DocumentSettings {
        /// The document.
        id: DocumentId,
    },
    /// A document was given a title, or its title was given back to its text.
    DocumentRenamed {
        /// The document.
        id: DocumentId,
    },
    /// A document went to another folder, or to another place in its folder.
    DocumentMoved {
        /// The document.
        id: DocumentId,
    },
    /// A document went to the trash.
    DocumentTrashed {
        /// The document.
        id: DocumentId,
    },
    /// A document came back from the trash.
    DocumentRestored {
        /// The document.
        id: DocumentId,
    },
    /// A document was deleted for good, from the trash or from the library (no
    /// `DocumentTrashed` before it).
    DocumentDeleted {
        /// The document.
        id: DocumentId,
    },
    /// A folder was created (one entry per folder, outermost first).
    FolderCreated {
        /// The folder.
        id: FolderId,
    },
    /// A folder was renamed.
    FolderRenamed {
        /// The folder.
        id: FolderId,
    },
    /// A folder went into another folder, or to another place among its neighbours.
    FolderMoved {
        /// The folder.
        id: FolderId,
    },
    /// A folder went to the trash, with everything in it.
    FolderTrashed {
        /// The folder.
        id: FolderId,
    },
    /// A folder came back from the trash, with everything that went there with it.
    FolderRestored {
        /// The folder.
        id: FolderId,
    },
    /// A folder was deleted for good, with everything in it.
    FolderDeleted {
        /// The folder.
        id: FolderId,
    },
    /// Everything in the trash was deleted for good.
    TrashEmptied,
    /// A version of a document was kept (the safety net before a change).
    VersionKept {
        /// The document.
        document: DocumentId,
    },
}

/// What [`execute`] did.
#[derive(Clone, Debug, PartialEq)]
pub struct Executed {
    /// What goes back to the tool.
    pub answer: Answer,
    /// What changed in the library, in order. Empty for a dry run, for `flush` and
    /// `hello`, for a refusal and for a request that asked for what is already so. An
    /// operation that failed has as a rule changed nothing; where it did (folders that
    /// were created before the document could not be placed) the list says what, so read
    /// it whatever the answer is.
    pub changes: Vec<Change>,
}

/// Carries a request out on `store`, as the app: on the library's own connection, with no
/// lock taken and no other connection opened, and with the safety net unchanged (a
/// version before the text of a document changes or a comment is deleted, the trash and
/// not deletion, a dry run that writes nothing).
///
/// The caller is the app, on the thread that owns the store, after it saved what was not
/// saved in the open document, and after [`Incoming::begin`] said that the tool is still
/// waiting. For `flush` and `hello` nothing is done here but to answer with the
/// [`AppState`].
///
/// When the owner switched changes by agents off ([`changes_allowed`]) an operation that
/// would change the library is refused with [`Refusal::SwitchedOff`]; its dry run and
/// `flush` are not.
///
/// No transaction is left open, whatever the answer.
pub fn execute(store: &mut Store, request: &Request) -> Executed {
    let Some(options) = request.operation.options() else {
        return Executed {
            answer: Answer::state(changes_allowed(store)),
            changes: Vec::new(),
        };
    };
    if !options.dry_run && !changes_allowed(store) {
        return Executed {
            answer: Answer::refused(Refusal::SwitchedOff),
            changes: Vec::new(),
        };
    }
    let clock = store.clock().clone();
    let allowed = changes_allowed(store);
    // A dry run does not write; the connection is told so, as the read-only connection
    // of a tool's own dry run is.
    let guarded = options.dry_run && store.set_query_only(true).is_ok();
    let mut session = Session::writing(store, clock);
    // What every result says about the app: it is open, here, and it is the one that
    // makes the change (or, with the switch off, would refuse the real run of a dry run).
    session.dry_run = options.dry_run;
    session.app_is_open = true;
    session.app_takes_changes = allowed;
    session.app_refuses = (!allowed).then_some(Refusal::SwitchedOff);
    // Whatever goes wrong in an operation must not take the app down with it: the
    // writer's window is not the agent's to lose. A transaction that was open when an
    // operation panicked is rolled back as its guard is dropped.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        request.operation.run(&mut session)
    }))
    .unwrap_or_else(|panic| {
        let said = panic
            .downcast_ref::<&str>()
            .map(|text| text.to_string())
            .or_else(|| panic.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "no message".to_string());
        Err(Error::InApp {
            kind: ErrorKind::Failed,
            message: format!(
                "Inner Word could not carry out {} (an internal error: {said}). Read the \
                 document again before you repeat it.",
                request.name()
            ),
        })
    });
    let changes = std::mem::take(&mut session.changes);
    if guarded {
        store.set_query_only(false).ok();
    }
    Executed {
        answer: match result {
            Ok(result) => Answer::Done { result },
            Err(error) => Answer::failed(&error),
        },
        changes,
    }
}
