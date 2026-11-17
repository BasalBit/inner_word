//! What can go wrong in making a page, in the words of docs/pages-design.md (sections 3
//! and 9). `innerword-agent` wraps these sentences; it does not reword them.

use std::fmt;
use std::io;
use std::path::PathBuf;

/// Why a page was not made, or a stored page cannot be read.
#[derive(Debug)]
pub enum PageError {
    /// The input cannot be used as asked (no such file, a folder with several pages and
    /// no entry, a reference outside the root, an address given as input).
    Input {
        message: String,
    },
    /// Cannot be kept whole: each problem names a piece and why.
    Incomplete {
        name: String,
        problems: Vec<Problem>,
    },
    /// The checker itself failed.
    CheckFailed {
        reason: String,
    },
    /// Stored content that is no manifest.
    Unreadable {
        reason: String,
    },
    Io {
        path: PathBuf,
        error: io::Error,
    },
}

/// One reason a page cannot be kept whole.
///
/// `what` is the piece the problem is about: an address, a path of the page, or "the
/// page" for a limit of the whole. `reason` is the clause that says it, whole, as it
/// stands in the sentence (it names the piece itself: "https://fonts.gstatic.com/s/x.woff2
/// could not be fetched: HTTP 404", "the page asks for audio/18.mp3, which is not among
/// its files"), so `reason` alone is what a person reads; `Problem::sentence` gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub what: String,
    pub reason: String,
}

impl Problem {
    pub fn new(what: impl Into<String>, reason: impl Into<String>) -> Problem {
        Problem {
            what: what.into(),
            reason: reason.into(),
        }
    }

    /// The problem as a clause of the sentence ("… could not be fetched: HTTP 404").
    pub fn sentence(&self) -> String {
        self.reason.clone()
    }
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.reason)
    }
}

/// How many problems a sentence names before it counts the rest.
pub const PROBLEMS_NAMED: usize = 5;

/// The reasons of a refusal as one sentence without its full stop: the first five
/// problems, separated by semicolons, then "and 3 more"; the first letter upper case.
/// ("The page asks for https://claude.ai/api/… at run time (data it fetches), which
/// cannot be kept; and 2 more")
pub fn reasons_sentence(problems: &[Problem]) -> String {
    let mut parts: Vec<String> = problems
        .iter()
        .take(PROBLEMS_NAMED)
        .map(|problem| problem.reason.clone())
        .collect();
    if problems.len() > PROBLEMS_NAMED {
        parts.push(format!("and {} more", problems.len() - PROBLEMS_NAMED));
    }
    capitalized(&parts.join("; "))
}

impl PageError {
    /// The whole sentence, for the command line, the MCP server and the app alike.
    pub fn sentence(&self) -> String {
        match self {
            PageError::Input { message } => message.clone(),
            PageError::Incomplete { name, problems } => {
                if problems.is_empty() {
                    format!(
                        "“{name}” was not added: it could not be kept whole. Nothing was saved."
                    )
                } else {
                    format!(
                        "“{name}” was not added: it could not be kept whole. {}. Nothing was saved.",
                        reasons_sentence(problems)
                    )
                }
            }
            PageError::CheckFailed { reason } => format!("the page could not be checked: {reason}"),
            PageError::Unreadable { reason } => format!("the stored page cannot be read: {reason}"),
            PageError::Io { path, error } => format!("{}: {}", path.display(), io_words(error)),
        }
    }

    pub(crate) fn input(message: impl Into<String>) -> PageError {
        PageError::Input {
            message: message.into(),
        }
    }

    pub(crate) fn io(path: impl Into<PathBuf>, error: io::Error) -> PageError {
        PageError::Io {
            path: path.into(),
            error,
        }
    }
}

impl fmt::Display for PageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.sentence())
    }
}

impl std::error::Error for PageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            PageError::Io { error, .. } => Some(error),
            _ => None,
        }
    }
}

/// An I/O error in plain words (without "(os error 2)").
fn io_words(error: &io::Error) -> String {
    match error.kind() {
        io::ErrorKind::NotFound => "there is no such file or folder".to_string(),
        io::ErrorKind::PermissionDenied => "permission denied".to_string(),
        io::ErrorKind::AlreadyExists => "it exists already".to_string(),
        _ => {
            let text = error.to_string();
            match text.find(" (os error") {
                Some(cut) => text[..cut].to_string(),
                None => text,
            }
        }
    }
}

/// The text with its first letter upper case, unless it begins with an address or a
/// file name ("https://…", "index.html names …"), which are written as they are.
pub(crate) fn capitalized(text: &str) -> String {
    let first_word = text.split(' ').next().unwrap_or("");
    if first_word.contains([':', '/', '.']) {
        return text.to_string();
    }
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// "2,000"
pub(crate) fn count(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// "71 MB", "32.3 MB", "0.1 MB" (a megabyte being 1024 × 1024 bytes, as the limits
/// are; one decimal, none when it is 0; never 0.0 for bytes that are there).
pub fn megabytes(bytes: u64) -> String {
    let mb = bytes as f64 / (1024.0 * 1024.0);
    if bytes == 0 {
        return "0 MB".to_string();
    }
    if mb >= 1000.0 {
        return format!("{} MB", mb.round() as u64);
    }
    let tenths = ((mb * 10.0).round() as u64).max(1);
    if tenths % 10 == 0 {
        format!("{} MB", tenths / 10)
    } else {
        format!("{}.{} MB", tenths / 10, tenths % 10)
    }
}
