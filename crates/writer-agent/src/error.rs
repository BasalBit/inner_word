//! The one error type of the command layer.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// What kind of thing a request referred to.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    /// A document.
    Document,
    /// A folder.
    Folder,
    /// A version (snapshot) of a document.
    Version,
    /// A comment thread.
    Comment,
    /// A reply in a comment thread.
    Reply,
    /// A heading inside a document.
    Heading,
}

impl ItemKind {
    /// The word for it in a message: `"document"`, `"folder"`, `"version"`, `"comment"`,
    /// `"reply"`, `"heading"`.
    pub fn name(self) -> &'static str {
        match self {
            ItemKind::Document => "document",
            ItemKind::Folder => "folder",
            ItemKind::Version => "version",
            ItemKind::Comment => "comment",
            ItemKind::Reply => "reply",
            ItemKind::Heading => "heading",
        }
    }
}

impl std::fmt::Display for ItemKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// One of several things a name could mean, in an [`Error::Ambiguous`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    /// What it is.
    pub kind: ItemKind,
    /// Its id (for a heading: its number among the headings that matched, from 1).
    pub id: i64,
    /// Where it is: the library path of a document or folder, the headings above a
    /// heading.
    pub path: String,
    /// Whether it is in the trash.
    #[serde(default)]
    pub trashed: bool,
}

/// One place where a piece of text was found, in an [`Error::NotUnique`] and in
/// [`FindResults`](crate::FindResults).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Occurrence {
    /// Its number among the occurrences in the document, from 1. Pass it as `occurrence`
    /// to choose this one.
    pub occurrence: usize,
    /// The index of the block it starts in, from 0.
    pub block: usize,
    /// The nearest heading above it, when there is one.
    pub heading: Option<String>,
    /// The line it is on, shortened around the match.
    pub context: String,
}

/// The kinds of failure, without their details: what an exit code or a `code` field tells
/// apart. See [`Error::kind`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// Something else went wrong: the database, the file system.
    Failed,
    /// The request itself is wrong: an empty name, a field that is missing, two that
    /// exclude each other.
    Invalid,
    /// Nothing has that id or name.
    NotFound,
    /// A name fits several items.
    Ambiguous,
    /// The app has the library open: nothing was written.
    AppIsOpen,
    /// The text to replace or to comment on does not occur in the document.
    NoMatch,
    /// The text to replace or to comment on occurs several times.
    NotUnique,
    /// A permanent deletion was asked for without the field that confirms it.
    ConfirmationRequired,
    /// The item is in the trash, or is not, and the operation needs the opposite.
    WrongState,
    /// There is no library at that place.
    LibraryNotFound,
    /// Another tool is writing to the library and did not finish in time.
    Busy,
    /// The file an export would write exists already.
    FileExists,
    /// The stored content of the document cannot be read as a document.
    Unreadable,
}

impl ErrorKind {
    /// Every kind, in the order of their exit codes.
    pub const ALL: [ErrorKind; 13] = [
        ErrorKind::Failed,
        ErrorKind::Invalid,
        ErrorKind::NotFound,
        ErrorKind::Ambiguous,
        ErrorKind::AppIsOpen,
        ErrorKind::NoMatch,
        ErrorKind::NotUnique,
        ErrorKind::ConfirmationRequired,
        ErrorKind::WrongState,
        ErrorKind::LibraryNotFound,
        ErrorKind::Busy,
        ErrorKind::FileExists,
        ErrorKind::Unreadable,
    ];

    /// A short stable name for machine-readable output: `"failed"`, `"invalid"`,
    /// `"not_found"`, `"ambiguous"`, `"app_is_open"`, `"no_match"`, `"not_unique"`,
    /// `"confirmation_required"`, `"wrong_state"`, `"library_not_found"`, `"busy"`,
    /// `"file_exists"`, `"unreadable"`.
    pub fn code(self) -> &'static str {
        match self {
            ErrorKind::Failed => "failed",
            ErrorKind::Invalid => "invalid",
            ErrorKind::NotFound => "not_found",
            ErrorKind::Ambiguous => "ambiguous",
            ErrorKind::AppIsOpen => "app_is_open",
            ErrorKind::NoMatch => "no_match",
            ErrorKind::NotUnique => "not_unique",
            ErrorKind::ConfirmationRequired => "confirmation_required",
            ErrorKind::WrongState => "wrong_state",
            ErrorKind::LibraryNotFound => "library_not_found",
            ErrorKind::Busy => "busy",
            ErrorKind::FileExists => "file_exists",
            ErrorKind::Unreadable => "unreadable",
        }
    }

    /// The exit code a command line front end gives for this kind of failure: 1
    /// [`Failed`](ErrorKind::Failed), 2 [`Invalid`](ErrorKind::Invalid), 3
    /// [`NotFound`](ErrorKind::NotFound), 4 [`Ambiguous`](ErrorKind::Ambiguous), 5
    /// [`AppIsOpen`](ErrorKind::AppIsOpen), 6 [`NoMatch`](ErrorKind::NoMatch), 7
    /// [`NotUnique`](ErrorKind::NotUnique), 8
    /// [`ConfirmationRequired`](ErrorKind::ConfirmationRequired), 9
    /// [`WrongState`](ErrorKind::WrongState), 10
    /// [`LibraryNotFound`](ErrorKind::LibraryNotFound), 11 [`Busy`](ErrorKind::Busy), 12
    /// [`FileExists`](ErrorKind::FileExists), 13 [`Unreadable`](ErrorKind::Unreadable).
    /// Never 0.
    pub fn exit_code(self) -> u8 {
        match self {
            ErrorKind::Failed => 1,
            ErrorKind::Invalid => 2,
            ErrorKind::NotFound => 3,
            ErrorKind::Ambiguous => 4,
            ErrorKind::AppIsOpen => 5,
            ErrorKind::NoMatch => 6,
            ErrorKind::NotUnique => 7,
            ErrorKind::ConfirmationRequired => 8,
            ErrorKind::WrongState => 9,
            ErrorKind::LibraryNotFound => 10,
            ErrorKind::Busy => 11,
            ErrorKind::FileExists => 12,
            ErrorKind::Unreadable => 13,
        }
    }
}

/// Everything that can go wrong in the command layer. The message of each variant
/// ([`std::fmt::Display`]) is written for whoever asked: it says what happened and, where
/// there is one, what to do next.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The request itself is wrong.
    #[error("{message}")]
    Invalid {
        /// What is wrong with it.
        message: String,
    },
    /// Nothing has that id or name.
    #[error("{}", not_found_message(*.kind, .reference, .hint.as_deref()))]
    NotFound {
        /// What was looked for.
        kind: ItemKind,
        /// The id or name that was given.
        reference: String,
        /// Something that helps to find the right one (the headings a document does have,
        /// for example).
        hint: Option<String>,
    },
    /// A name fits several items. The candidates carry the ids to choose with.
    #[error("{}", ambiguous_message(*.kind, .reference, .candidates))]
    Ambiguous {
        /// What was looked for.
        kind: ItemKind,
        /// The name that was given.
        reference: String,
        /// Everything it fits.
        candidates: Vec<Candidate>,
    },
    /// The app has the library open, so nothing was written. Reading works while the app
    /// is open; writing needs the app to be quit first.
    #[error(
        "The Writer is open{}, so nothing was changed. Quit The Writer and run this again \
         (reading works while it is open: list, search, get, export).",
        .pid.map(|pid| format!(" (process {pid})")).unwrap_or_default()
    )]
    AppIsOpen {
        /// The app's process id, when the lock file names it.
        pid: Option<u32>,
        /// The data directory it has open.
        data_dir: PathBuf,
    },
    /// Another tool is writing to the library and did not finish in time.
    #[error(
        "another tool is writing to this library{} and did not finish in time; try again",
        .pid.map(|pid| format!(" (process {pid})")).unwrap_or_default()
    )]
    Busy {
        /// Its process id, when the lock file names it.
        pid: Option<u32>,
    },
    /// The text to replace or to comment on does not occur in the document.
    #[error("{}", no_match_message(.text, .hint.as_deref()))]
    NoMatch {
        /// The text that was looked for.
        text: String,
        /// What might have gone wrong.
        hint: Option<String>,
    },
    /// The text to replace or to comment on occurs several times, and the request did not
    /// say which one (`occurrence`) or all of them (`all`).
    #[error("{}", not_unique_message(.text, *.count, .occurrences))]
    NotUnique {
        /// The text that was looked for.
        text: String,
        /// How many times it occurs.
        count: usize,
        /// The first occurrences, each with its number and the line it is on.
        occurrences: Vec<Occurrence>,
    },
    /// A permanent deletion was asked for without the field that confirms it. Nothing was
    /// deleted.
    #[error(
        "{what} would be deleted for good, with every version of it. Nothing was deleted: \
         confirm with the explicit \"permanently\" option, or move it to the trash instead."
    )]
    ConfirmationRequired {
        /// What would have been deleted.
        what: String,
    },
    /// The item is in the trash and the operation needs a live one.
    #[error("the {kind} {name} (id {id}) is in the trash; restore it first")]
    InTrash {
        /// What it is.
        kind: ItemKind,
        /// Its id.
        id: i64,
        /// Its title or name, quoted.
        name: String,
    },
    /// The item is not an entry of the trash: it is not trashed, or it went to the trash
    /// together with its folder and comes back with that folder.
    #[error("{}", not_in_trash_message(*.kind, *.id, .name, .inside.as_deref()))]
    NotInTrash {
        /// What it is.
        kind: ItemKind,
        /// Its id.
        id: i64,
        /// Its title or name, quoted.
        name: String,
        /// The trashed folder it is part of, when that is why.
        inside: Option<String>,
    },
    /// There is no library at that place (reading never creates one).
    #[error(
        "there is no library at {}. The Writer creates it the first time it runs; a library \
         in another place is named with the data directory option or THE_WRITER_DATA_DIR.",
        .path.display()
    )]
    LibraryNotFound {
        /// The database file that was looked for.
        path: PathBuf,
    },
    /// The stored content of a document cannot be read as a document (it is damaged, or
    /// written by a newer version). It was left alone.
    #[error("the document {name} (id {id}) cannot be read: {reason}. It was left as it is.")]
    Unreadable {
        /// The document's id.
        id: i64,
        /// Its title, quoted.
        name: String,
        /// Why.
        reason: String,
    },
    /// The file an export would write exists already.
    #[error("{} exists already; choose another path or ask for it to be overwritten", .path.display())]
    FileExists {
        /// The file.
        path: PathBuf,
    },
    /// A file could not be read or written.
    #[error("{}: {source}", .path.display())]
    Io {
        /// The file or directory.
        path: PathBuf,
        /// What the system said.
        #[source]
        source: std::io::Error,
    },
    /// The library reported an error.
    #[error(transparent)]
    Store(writer_store::StoreError),
}

impl Error {
    /// Which kind of failure this is.
    pub fn kind(&self) -> ErrorKind {
        match self {
            Error::Invalid { .. } => ErrorKind::Invalid,
            Error::NotFound { .. } => ErrorKind::NotFound,
            Error::Ambiguous { .. } => ErrorKind::Ambiguous,
            Error::AppIsOpen { .. } => ErrorKind::AppIsOpen,
            Error::Busy { .. } => ErrorKind::Busy,
            Error::NoMatch { .. } => ErrorKind::NoMatch,
            Error::NotUnique { .. } => ErrorKind::NotUnique,
            Error::ConfirmationRequired { .. } => ErrorKind::ConfirmationRequired,
            Error::InTrash { .. } | Error::NotInTrash { .. } => ErrorKind::WrongState,
            Error::LibraryNotFound { .. } => ErrorKind::LibraryNotFound,
            Error::Unreadable { .. } => ErrorKind::Unreadable,
            Error::FileExists { .. } => ErrorKind::FileExists,
            Error::Io { .. } | Error::Store(_) => ErrorKind::Failed,
        }
    }

    /// The short stable name of the kind of failure ([`ErrorKind::code`]).
    pub fn code(&self) -> &'static str {
        self.kind().code()
    }

    /// The exit code for a command line front end ([`ErrorKind::exit_code`]).
    pub fn exit_code(&self) -> u8 {
        self.kind().exit_code()
    }

    /// The failure as data, for a front end that answers in JSON.
    pub fn report(&self) -> ErrorReport {
        let (candidates, occurrences) = match self {
            Error::Ambiguous { candidates, .. } => (candidates.clone(), Vec::new()),
            Error::NotUnique { occurrences, .. } => (Vec::new(), occurrences.clone()),
            _ => (Vec::new(), Vec::new()),
        };
        ErrorReport {
            code: self.code().to_string(),
            message: self.to_string(),
            candidates,
            occurrences,
        }
    }

    /// An [`Error::Invalid`] with this message.
    pub fn invalid(message: impl Into<String>) -> Error {
        Error::Invalid {
            message: message.into(),
        }
    }
}

impl From<writer_store::StoreError> for Error {
    fn from(error: writer_store::StoreError) -> Error {
        Error::Store(error)
    }
}

/// An [`Error`] as data: what a front end that answers in JSON writes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorReport {
    /// The kind of failure ([`ErrorKind::code`]).
    pub code: String,
    /// The message, as [`Error`] displays it.
    pub message: String,
    /// For `"ambiguous"`: everything the name fits.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<Candidate>,
    /// For `"not_unique"`: the first places where the text occurs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub occurrences: Vec<Occurrence>,
}

/// Shorthand for `Result<T, Error>`.
pub type Result<T, E = Error> = std::result::Result<T, E>;

fn not_found_message(kind: ItemKind, reference: &str, hint: Option<&str>) -> String {
    let mut message = format!("there is no {kind} {reference}");
    if let Some(hint) = hint {
        message.push_str(". ");
        message.push_str(hint);
    }
    message
}

fn ambiguous_message(kind: ItemKind, reference: &str, candidates: &[Candidate]) -> String {
    let mut message = format!(
        "{reference} fits {} {kind}s; name one by its id or by a longer path:",
        candidates.len()
    );
    for candidate in candidates {
        message.push_str(&format!("\n  {}  {}", candidate.id, candidate.path));
        if candidate.trashed {
            message.push_str("  (in the trash)");
        }
    }
    message
}

fn no_match_message(text: &str, hint: Option<&str>) -> String {
    let mut message = format!(
        "the text was not found in the document: {}",
        quoted_excerpt(text)
    );
    message.push_str(". ");
    message.push_str(hint.unwrap_or(
        "It has to be in the document exactly as given: the same words, punctuation, \
         quotation marks and capitals",
    ));
    message
}

fn not_unique_message(text: &str, count: usize, occurrences: &[Occurrence]) -> String {
    let mut message = format!(
        "the text occurs {count} times in the document: {}. Give more of the text around \
         it so that it is found once, choose one by its number (occurrence), or ask for \
         all of them:",
        quoted_excerpt(text)
    );
    for occurrence in occurrences {
        message.push_str(&format!("\n  {}  ", occurrence.occurrence));
        if let Some(heading) = &occurrence.heading {
            message.push_str(&format!("[{heading}] "));
        }
        message.push_str(&occurrence.context);
    }
    if count > occurrences.len() {
        message.push_str(&format!("\n  and {} more", count - occurrences.len()));
    }
    message
}

fn not_in_trash_message(kind: ItemKind, id: i64, name: &str, inside: Option<&str>) -> String {
    match inside {
        Some(folder) => format!(
            "the {kind} {name} (id {id}) went to the trash together with the folder \
             {folder}; restore or delete that folder"
        ),
        None => format!("the {kind} {name} (id {id}) is not in the trash"),
    }
}

/// `text` in quotation marks, on one line, shortened in the middle when it is long.
pub(crate) fn quoted_excerpt(text: &str) -> String {
    const KEEP: usize = 60;
    let line: String = text
        .chars()
        .map(|c| if c == '\n' { '\u{23ce}' } else { c })
        .collect();
    let count = line.chars().count();
    if count <= 2 * KEEP + 5 {
        return format!("\u{201c}{line}\u{201d}");
    }
    let head: String = line.chars().take(KEEP).collect();
    let tail: String = line.chars().skip(count - KEEP).collect();
    format!("\u{201c}{head} \u{2026} {tail}\u{201d}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_have_distinct_codes() {
        let mut codes = std::collections::BTreeSet::new();
        let mut names = std::collections::BTreeSet::new();
        for kind in ErrorKind::ALL {
            assert_ne!(kind.exit_code(), 0);
            assert!(codes.insert(kind.exit_code()), "{kind:?}");
            assert!(names.insert(kind.code()), "{kind:?}");
            // The serialised name is the code.
            assert_eq!(
                serde_json::to_string(&kind).unwrap(),
                format!("\"{}\"", kind.code())
            );
        }
        assert_eq!(codes.len(), ErrorKind::ALL.len());
    }

    #[test]
    fn messages_say_what_to_do() {
        let open = Error::AppIsOpen {
            pid: Some(42),
            data_dir: PathBuf::from("/tmp/x"),
        };
        assert_eq!(open.exit_code(), 5);
        assert_eq!(open.code(), "app_is_open");
        let message = open.to_string();
        assert!(message.contains("Quit The Writer"), "{message}");
        assert!(message.contains("process 42"), "{message}");

        let ambiguous = Error::Ambiguous {
            kind: ItemKind::Document,
            reference: "\u{201c}Chapter 1\u{201d}".into(),
            candidates: vec![
                Candidate {
                    kind: ItemKind::Document,
                    id: 3,
                    path: "Book/Chapter 1".into(),
                    trashed: false,
                },
                Candidate {
                    kind: ItemKind::Document,
                    id: 9,
                    path: "Drafts/Chapter 1".into(),
                    trashed: false,
                },
            ],
        };
        let message = ambiguous.to_string();
        assert!(message.contains("3  Book/Chapter 1"), "{message}");
        assert!(message.contains("9  Drafts/Chapter 1"), "{message}");
        let report = ambiguous.report();
        assert_eq!(report.code, "ambiguous");
        assert_eq!(report.candidates.len(), 2);
        assert_eq!(report.message, message);
    }

    #[test]
    fn long_text_is_shortened_in_messages() {
        let text = "word ".repeat(200);
        let quoted = quoted_excerpt(&text);
        assert!(quoted.chars().count() < 140);
        assert!(quoted.contains('\u{2026}'));
        assert_eq!(quoted_excerpt("a\nb"), "\u{201c}a\u{23ce}b\u{201d}");
    }
}
