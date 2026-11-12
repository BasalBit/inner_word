//! Request and result types. Everything here serialises to JSON: a command line front
//! end prints the results, an MCP server returns them, and both can read the requests
//! from JSON (every request field but the item it names is optional there).

use std::path::PathBuf;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
pub use writer_store::{DocumentId, FolderId, SnapshotId};

use crate::error::Occurrence;

// ----- naming things ---------------------------------------------------------------------

/// How a request names a document or a folder: by its id, or by its path in the library.
///
/// **Ids** are the numbers every result carries (`id`). They are exact. Documents and
/// folders are numbered separately, so an id is written with its kind wherever either
/// could be meant: `doc:12`, `folder:3`.
///
/// **Paths** are written the way the library shows them, folder names and then the
/// document's title, separated by `/`: `The Salt Road/Part one/Chapter 1`. A path does not
/// have to start at the top: `Chapter 1`, or `Part one/Chapter 1`, names every item whose
/// path *ends* that way. When that is more than one item the request fails with
/// [`Error::Ambiguous`](crate::Error::Ambiguous), which lists the candidates with their
/// ids. A path that starts with `/` is taken from the top of the library only (`/Notes` is
/// the item "Notes" at the top, not one in a folder). A `/` inside a name is written
/// `\/`, a backslash `\\`. Names are compared exactly first; when nothing fits, again
/// without regard to case. A document without a title is named `Untitled`. The path `/`
/// alone is the top of the library, where a folder is asked for.
///
/// In JSON an id is a number or one of the strings below, and a path is any other string.
/// [`Target::parse`] reads what somebody typed:
///
/// | Written | Means |
/// |---|---|
/// | `12`, `#12`, `id:12` | the id 12 (of a document or a folder, whichever the request asks for) |
/// | `doc:12`, `document:12` | the document 12 |
/// | `folder:3` | the folder 3 |
/// | anything else | a path |
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    /// The item with this id: a document where a document is asked for, a folder where a
    /// folder is.
    Id(i64),
    /// The document with this id. Where a folder is asked for, the request fails with
    /// [`Error::Invalid`](crate::Error::Invalid).
    DocumentId(i64),
    /// The folder with this id. Where a document is asked for, the request fails with
    /// [`Error::Invalid`](crate::Error::Invalid).
    FolderId(i64),
    /// The item at this path.
    Path(String),
}

impl Target {
    /// The item with this id.
    pub fn id(id: i64) -> Target {
        Target::Id(id)
    }

    /// The item at this path.
    pub fn path(path: impl Into<String>) -> Target {
        Target::Path(path.into())
    }

    /// The top of the library, where a folder is asked for (the path `/`).
    pub fn root() -> Target {
        Target::Path("/".to_string())
    }

    /// Reads what somebody typed (see the table at [`Target`]). An item whose name is a
    /// number, or looks like one of the id forms, is named by a path that starts at the
    /// top (`/1984`).
    pub fn parse(text: &str) -> Target {
        let trimmed = text.trim();
        let number = |digits: &str| -> Option<i64> {
            let is_number = !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit());
            digits.parse::<i64>().ok().filter(|_| is_number)
        };
        let prefixed = |prefix: &str| -> Option<i64> {
            let head = trimmed.get(..prefix.len())?;
            if !head.eq_ignore_ascii_case(prefix) {
                return None;
            }
            number(&trimmed[prefix.len()..])
        };
        if let Some(id) = prefixed("doc:").or_else(|| prefixed("document:")) {
            return Target::DocumentId(id);
        }
        if let Some(id) = prefixed("folder:") {
            return Target::FolderId(id);
        }
        if let Some(id) = number(trimmed)
            .or_else(|| prefixed("#"))
            .or_else(|| prefixed("id:"))
        {
            return Target::Id(id);
        }
        Target::Path(text.to_string())
    }

    /// Whether this is the top of the library.
    pub fn is_root(&self) -> bool {
        matches!(self, Target::Path(path) if path.trim() == "/")
    }

    /// Whether this names nothing at all: the empty path (what a request holds when the
    /// item was left out).
    pub fn is_empty(&self) -> bool {
        matches!(self, Target::Path(path) if path.trim().is_empty())
    }
}

/// No item: the empty path. A request that leaves the item out fails with
/// [`Error::Invalid`](crate::Error::Invalid).
impl Default for Target {
    fn default() -> Target {
        Target::Path(String::new())
    }
}

/// An id as `12`, `doc:12` or `folder:3`; a path as it is.
impl std::fmt::Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Target::Id(id) => write!(f, "{id}"),
            Target::DocumentId(id) => write!(f, "doc:{id}"),
            Target::FolderId(id) => write!(f, "folder:{id}"),
            Target::Path(path) => f.write_str(path),
        }
    }
}

impl From<i64> for Target {
    fn from(id: i64) -> Target {
        Target::Id(id)
    }
}

impl From<DocumentId> for Target {
    fn from(id: DocumentId) -> Target {
        Target::DocumentId(id.0)
    }
}

impl From<FolderId> for Target {
    fn from(id: FolderId) -> Target {
        Target::FolderId(id.0)
    }
}

impl From<&str> for Target {
    fn from(path: &str) -> Target {
        Target::Path(path.to_string())
    }
}

impl From<String> for Target {
    fn from(path: String) -> Target {
        Target::Path(path)
    }
}

impl Serialize for Target {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Target::Id(id) => serializer.serialize_i64(*id),
            Target::DocumentId(_) | Target::FolderId(_) => {
                serializer.serialize_str(&self.to_string())
            }
            Target::Path(path) => serializer.serialize_str(path),
        }
    }
}

impl<'de> Deserialize<'de> for Target {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Target, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Id(i64),
            Text(String),
        }
        Ok(match Raw::deserialize(deserializer)? {
            Raw::Id(id) => Target::Id(id),
            Raw::Text(text) => Target::parse(&text),
        })
    }
}

/// Where an item goes among its siblings. Documents are ordered among the documents of
/// their folder and folders among the folders of theirs (the library lists a folder's
/// subfolders first, then its documents).
///
/// In JSON: `"first"`, `"last"`, `{"index": 2}`, `{"before": 12}`, `{"after": "Chapter 4"}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Placement {
    /// Before every sibling.
    First,
    /// After every sibling.
    Last,
    /// At this position, counted from 0 (past the end is the end).
    Index(usize),
    /// Directly before this sibling, which has to be in the destination folder.
    Before(Target),
    /// Directly after this sibling, which has to be in the destination folder.
    After(Target),
}

/// A point in time. In JSON it is written as UTC in RFC 3339 form
/// (`"2026-11-12T09:30:00.000Z"`) and read from that form or from a number of
/// milliseconds since the Unix epoch.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Time(pub i64);

impl Time {
    /// Milliseconds since the Unix epoch.
    pub fn unix_ms(self) -> i64 {
        self.0
    }

    /// UTC in RFC 3339 form with milliseconds: `2026-11-12T09:30:00.000Z`.
    pub fn to_rfc3339(self) -> String {
        let ms = self.0.rem_euclid(1000);
        let seconds = self.0.div_euclid(1000);
        let day = seconds.div_euclid(86_400);
        let second_of_day = seconds.rem_euclid(86_400);
        let (year, month, date) = civil_from_days(day);
        format!(
            "{year:04}-{month:02}-{date:02}T{:02}:{:02}:{:02}.{ms:03}Z",
            second_of_day / 3600,
            second_of_day % 3600 / 60,
            second_of_day % 60
        )
    }

    /// Reads the form [`Time::to_rfc3339`] writes (the fraction of a second is optional;
    /// only UTC, `Z`).
    pub fn parse_rfc3339(text: &str) -> Option<Time> {
        let text = text.trim();
        let text = text.strip_suffix('Z').or_else(|| text.strip_suffix('z'))?;
        let (date, time) = text.split_once(['T', 't', ' '])?;
        let mut date = date.splitn(3, '-');
        // A year before 1 is not written by `to_rfc3339`.
        let year: i64 = date.next()?.parse().ok()?;
        let month: i64 = date.next()?.parse().ok()?;
        let day: i64 = date.next()?.parse().ok()?;
        let (clock, fraction) = match time.split_once('.') {
            Some((clock, fraction)) => (clock, fraction),
            None => (time, ""),
        };
        let mut clock = clock.splitn(3, ':');
        let hour: i64 = clock.next()?.parse().ok()?;
        let minute: i64 = clock.next()?.parse().ok()?;
        let second: i64 = clock.next()?.parse().ok()?;
        if !(1..=12).contains(&month)
            || !(1..=31).contains(&day)
            || !(0..24).contains(&hour)
            || !(0..60).contains(&minute)
            || !(0..61).contains(&second)
            || !fraction.bytes().all(|b| b.is_ascii_digit())
        {
            return None;
        }
        let mut ms = 0;
        for (place, digit) in fraction.bytes().take(3).enumerate() {
            ms += i64::from(digit - b'0') * [100, 10, 1][place];
        }
        let days = days_from_civil(year, month, day);
        Some(Time(
            ((days * 24 + hour) * 60 + minute) * 60_000 + second * 1000 + ms,
        ))
    }
}

impl From<i64> for Time {
    fn from(ms: i64) -> Time {
        Time(ms)
    }
}

impl std::fmt::Display for Time {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_rfc3339())
    }
}

impl Serialize for Time {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_rfc3339())
    }
}

impl<'de> Deserialize<'de> for Time {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Time, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Ms(i64),
            Text(String),
        }
        match Raw::deserialize(deserializer)? {
            Raw::Ms(ms) => Ok(Time(ms)),
            Raw::Text(text) => Time::parse_rfc3339(&text)
                .ok_or_else(|| serde::de::Error::custom(format!("not a time: {text}"))),
        }
    }
}

/// The civil date of a day number (days since 1970-01-01), proleptic Gregorian.
fn civil_from_days(day: i64) -> (i64, i64, i64) {
    let z = day + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let date = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, date)
}

/// The day number (days since 1970-01-01) of a civil date.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year.rem_euclid(400);
    let shifted_month = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The form a document's text is read in.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextFormat {
    /// Markdown (the default): what the app's Markdown export writes, and what every
    /// operation that takes text reads.
    #[default]
    Markdown,
    /// Plain text without any markup, as the app's plain text export writes it: a blank
    /// line between blocks, list items with their bullets and numbers.
    #[serde(alias = "text", alias = "txt", alias = "plain")]
    PlainText,
    /// The document as the library stores it: the native JSON of the document model, with
    /// every attribute, the comment threads and the settings.
    Json,
}

impl TextFormat {
    /// Reads a name: `markdown` or `md`, `text`, `txt`, `plain` or `plain_text`, `json`
    /// (case does not matter).
    pub fn parse(name: &str) -> Option<TextFormat> {
        match name.trim().to_ascii_lowercase().as_str() {
            "markdown" | "md" => Some(TextFormat::Markdown),
            "text" | "txt" | "plain" | "plain_text" | "plaintext" => Some(TextFormat::PlainText),
            "json" => Some(TextFormat::Json),
            _ => None,
        }
    }

    /// The name [`TextFormat::parse`] reads and JSON carries: `"markdown"`,
    /// `"plain_text"`, `"json"`.
    pub fn name(self) -> &'static str {
        match self {
            TextFormat::Markdown => "markdown",
            TextFormat::PlainText => "plain_text",
            TextFormat::Json => "json",
        }
    }
}

/// The file format of an export.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    /// Markdown, `.md`.
    #[default]
    #[serde(alias = "md")]
    Markdown,
    /// A Word document, `.docx`.
    #[serde(alias = "word")]
    Docx,
    /// Plain text, `.txt`.
    #[serde(alias = "text", alias = "txt", alias = "plain")]
    PlainText,
}

impl ExportFormat {
    /// Reads a name or a file extension: `markdown` or `md`, `docx` or `word`, `text`,
    /// `txt`, `plain` or `plain_text` (case does not matter, a leading dot is ignored).
    pub fn parse(name: &str) -> Option<ExportFormat> {
        match name
            .trim()
            .trim_start_matches('.')
            .to_ascii_lowercase()
            .as_str()
        {
            "markdown" | "md" => Some(ExportFormat::Markdown),
            "docx" | "word" => Some(ExportFormat::Docx),
            "text" | "txt" | "plain" | "plain_text" | "plaintext" => Some(ExportFormat::PlainText),
            _ => None,
        }
    }

    /// The name JSON carries: `"markdown"`, `"docx"`, `"plain_text"`.
    pub fn name(self) -> &'static str {
        match self {
            ExportFormat::Markdown => "markdown",
            ExportFormat::Docx => "docx",
            ExportFormat::PlainText => "plain_text",
        }
    }

    /// The file extension without the dot: `"md"`, `"docx"`, `"txt"`.
    pub fn extension(self) -> &'static str {
        match self {
            ExportFormat::Markdown => "md",
            ExportFormat::Docx => "docx",
            ExportFormat::PlainText => "txt",
        }
    }
}

/// What every operation that writes to the library takes besides its own fields.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WriteOptions {
    /// Work out and report what the operation would do, and change nothing. A dry run
    /// needs no access to write: it also runs while the app is open, and then says so
    /// ([`Written::app_is_open`]).
    pub dry_run: bool,
    /// The name of whoever makes the change (the agent's name, "Claude" for example). It
    /// labels the version that is kept of a document before its text is changed ("Before
    /// an edit by Claude"), and it is the author of a comment or a reply. Without one the
    /// label says "an agent" and a comment is signed "Agent".
    pub author: Option<String>,
}

impl WriteOptions {
    /// The options of a dry run.
    pub fn dry_run() -> WriteOptions {
        WriteOptions {
            dry_run: true,
            author: None,
        }
    }

    /// The options of a real run by `author`.
    pub fn by(author: impl Into<String>) -> WriteOptions {
        WriteOptions {
            dry_run: false,
            author: Some(author.into()),
        }
    }
}

/// What every result of an operation that writes says about the write itself.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Written {
    /// Whether this was a dry run: the result says what would be done, nothing was.
    pub dry_run: bool,
    /// Whether the library was changed. False for a dry run, and for a request that asked
    /// for what is already so (the same text, the same name, the same place).
    pub changed: bool,
    /// Only in a dry run: the app has the library open, so the real run would be refused
    /// with [`Error::AppIsOpen`](crate::Error::AppIsOpen) until the app is quit.
    #[serde(default)]
    pub app_is_open: bool,
    /// Whether the request has anything to do: what `changed` is after a real run, and
    /// what it would be after the real run of a dry run.
    #[serde(default)]
    pub would_change: bool,
}

// ----- what results are made of ----------------------------------------------------------

/// A document as listings and results show it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentSummary {
    /// Its id.
    pub id: DocumentId,
    /// Its title.
    pub title: String,
    /// Its path in the library: the folders it is in and its title, separated by `/`
    /// (see [`Target`]). For a document in the trash, where it was.
    pub path: String,
    /// The folder it is in, `None` at the top of the library.
    pub folder_id: Option<FolderId>,
    /// The path of that folder; empty at the top of the library.
    pub folder: String,
    /// How many words it has.
    pub words: u64,
    /// How many characters it has (what a reader would count as one: grapheme clusters).
    pub characters: u64,
    /// When it was created.
    pub created_at: Time,
    /// When its text or its title last changed.
    pub updated_at: Time,
    /// When it went to the trash; `None` for a document that is not in the trash.
    pub trashed_at: Option<Time>,
}

/// A folder as listings and results show it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderSummary {
    /// Its id.
    pub id: FolderId,
    /// Its name.
    pub name: String,
    /// Its path in the library: the folders it is in and its name, separated by `/`.
    pub path: String,
    /// The folder it is in, `None` at the top of the library.
    pub parent_id: Option<FolderId>,
    /// When it was created.
    pub created_at: Time,
    /// When it was last renamed or moved.
    pub updated_at: Time,
    /// When it went to the trash; `None` for a folder that is not in the trash.
    pub trashed_at: Option<Time>,
}

/// A heading of a document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeadingInfo {
    /// Its level, 1 to 6.
    pub level: u8,
    /// Its text.
    pub text: String,
    /// The index of its block in the document, from 0.
    pub block: usize,
    /// Its number among the headings of the document with exactly this text, from 1: the
    /// `heading_occurrence` that names it when the text alone does not.
    pub occurrence: usize,
    /// How many words its section has: the heading itself and everything up to the next
    /// heading of the same or a higher level.
    pub words: u64,
}

/// A comment thread: a comment on a piece of the text, with its replies.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommentThreadInfo {
    /// The thread's id, unique in its document.
    pub id: u64,
    /// Who wrote the comment.
    pub author: String,
    /// The comment.
    pub body: String,
    /// When it was written.
    pub created_at: Time,
    /// Whether the thread is resolved.
    pub resolved: bool,
    /// The text the thread is anchored to. `None` when that text is gone from the
    /// document (the thread is then kept without a place; the app lists it last).
    pub anchored_text: Option<String>,
    /// The nearest heading above the anchored text, when there is one.
    pub heading: Option<String>,
    /// The index of the block the anchored text starts in, from 0.
    pub block: Option<usize>,
    /// The replies, oldest first.
    pub replies: Vec<ReplyInfo>,
}

/// A reply in a comment thread.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplyInfo {
    /// The reply's id, unique in its thread.
    pub id: u64,
    /// Who wrote it.
    pub author: String,
    /// The reply.
    pub body: String,
    /// When it was written.
    pub created_at: Time,
}

/// A version of a document: its text as it was at some moment, kept by the library.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionInfo {
    /// The version's id (unique in the library).
    pub id: SnapshotId,
    /// The document it is a version of.
    pub document_id: DocumentId,
    /// When it was kept.
    pub created_at: Time,
    /// The document's title at that time.
    pub title: String,
    /// How many words it had.
    pub words: u64,
    /// How many characters it had.
    pub characters: u64,
    /// Why it was kept, when it was kept for a reason: "Before an edit by Claude". `None`
    /// for the versions the app keeps as the writer works.
    pub label: Option<String>,
}

/// The page and type settings of a document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DocumentSettingsInfo {
    /// Whether the document is shown as pages.
    pub page_view: bool,
    /// The paper: `"A4"`, `"US Letter"` or `"Custom"`.
    pub paper: String,
    /// The paper's width in points (1/72 inch), as it is turned.
    pub page_width: f32,
    /// The paper's height in points, as it is turned.
    pub page_height: f32,
    /// `"portrait"` or `"landscape"`.
    pub orientation: String,
    /// The margins in points: top, right, bottom, left.
    pub margins: [f32; 4],
    /// The font of the text.
    pub font: String,
    /// The size of the text in points.
    pub text_size: f32,
    /// The line height, as a multiple of the text size.
    pub line_height: f32,
    /// The space between paragraphs in points.
    pub paragraph_spacing: f32,
    /// The language the spelling is checked in: `None` follows the app's setting,
    /// `"auto"` detects it per paragraph, anything else is a language identifier
    /// (`"en_GB"`).
    pub spelling_language: Option<String>,
}

// ----- the library -----------------------------------------------------------------------

/// What [`Library::status`](crate::Library::status) reports.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryStatus {
    /// The data directory.
    pub data_dir: PathBuf,
    /// The database file in it.
    pub database: PathBuf,
    /// Whether the library exists. Reading a library that does not exist fails with
    /// [`Error::LibraryNotFound`](crate::Error::LibraryNotFound); the first write creates
    /// it.
    pub exists: bool,
    /// Whether the app has the library open right now. While it does, everything that
    /// writes is refused with [`Error::AppIsOpen`](crate::Error::AppIsOpen).
    pub app_is_open: bool,
    /// The process id of the app that has it open, when the lock file names it.
    pub app_pid: Option<u32>,
    /// How many documents the library has (not counting the trash).
    pub documents: u64,
    /// How many folders (not counting the trash).
    pub folders: u64,
    /// How many entries the trash has.
    pub trashed: u64,
    /// The version of this command layer (the version of the app it ships with).
    pub version: String,
}

/// A request for the tree of the library.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TreeRequest {
    /// Only this folder and what is in it. Without it: the whole library.
    pub folder: Option<Target>,
    /// How many levels of folders below the starting point to go into. Without it: all of
    /// them. `0` lists only what is directly in the starting point. A folder that is not
    /// gone into is listed with its counts and without its content.
    pub depth: Option<usize>,
    /// Leave the documents out and list only the folders.
    pub folders_only: bool,
}

/// The tree of a library, or of one folder of it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tree {
    /// The folder the tree starts at; `None` when it is the whole library.
    pub folder: Option<FolderSummary>,
    /// The folders directly in it, in the order the library shows them.
    pub folders: Vec<TreeFolder>,
    /// The documents directly in it, in the order the library shows them (after the
    /// folders).
    pub documents: Vec<DocumentSummary>,
    /// How many folders there are below the starting point, at every depth.
    pub total_folders: u64,
    /// How many documents there are below the starting point, at every depth.
    pub total_documents: u64,
    /// How many words those documents have together.
    pub total_words: u64,
}

/// A folder in a [`Tree`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeFolder {
    /// The folder.
    #[serde(flatten)]
    pub folder: FolderSummary,
    /// How many folders it holds, at every depth.
    pub folder_count: u64,
    /// How many documents it holds, at every depth.
    pub document_count: u64,
    /// The folders directly in it. Empty also when the tree was asked not to go this
    /// deep (see `folder_count`).
    pub folders: Vec<TreeFolder>,
    /// The documents directly in it. Empty also when the tree was asked not to go this
    /// deep, or without documents (see `document_count`).
    pub documents: Vec<DocumentSummary>,
}

/// A request to search the text and the titles of every document.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchRequest {
    /// The words to look for. Every word has to occur in a document (in its title or its
    /// text); words in double quotes have to occur as that phrase; the last word also
    /// finds words that begin with it. Case and accents do not matter.
    pub query: String,
    /// The most hits to return. Without it: 20.
    pub limit: Option<usize>,
    /// Only documents in this folder (at any depth): those among the 2,000 best hits of
    /// the whole library.
    pub folder: Option<Target>,
}

/// The hits of a search, best first.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SearchResults {
    /// The query as it was given.
    pub query: String,
    /// The hits, best first.
    pub hits: Vec<SearchHit>,
}

/// One document a search found.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SearchHit {
    /// The document.
    pub document: DocumentSummary,
    /// A piece of its text around the first word that matched (the start of the text when
    /// only the title matched).
    pub snippet: String,
    /// The same snippet in pieces, saying which of them are the words that matched.
    pub snippet_parts: Vec<SnippetPart>,
    /// How well it matched; higher is better. Only good for comparing the hits of one
    /// search.
    pub rank: f64,
}

/// A piece of a [`SearchHit`]'s snippet.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnippetPart {
    /// The text.
    pub text: String,
    /// Whether it is one of the words that matched.
    pub is_match: bool,
}

/// A request for the documents that were edited last.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RecentRequest {
    /// The most documents to return. Without it: 10.
    pub limit: Option<usize>,
}

/// A list of documents.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentList {
    /// The documents.
    pub documents: Vec<DocumentSummary>,
}

/// A request for what is known about one document.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct InfoRequest {
    /// The document. By id, one in the trash is found too.
    pub document: Target,
}

/// What is known about one document without its text.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DocumentInfo {
    /// The document: id, title, path, folder, counts and dates.
    pub document: DocumentSummary,
    /// Whether the title was chosen by someone (a rename). Otherwise it follows the text:
    /// the first heading, else the first line.
    pub custom_title: bool,
    /// How many paragraphs (blocks with text) it has.
    pub paragraphs: u64,
    /// How many characters it has without counting white space.
    pub characters_no_spaces: u64,
    /// How many minutes it takes to read.
    pub reading_minutes: u64,
    /// Its headings, in order: the outline of the document.
    pub headings: Vec<HeadingInfo>,
    /// How many comment threads are open.
    pub open_comments: usize,
    /// How many comment threads are resolved.
    pub resolved_comments: usize,
    /// How many versions of it the library keeps.
    pub versions: usize,
    /// Its page and type settings.
    pub settings: DocumentSettingsInfo,
}

/// A request for what is known about one folder.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FolderInfoRequest {
    /// The folder ([`Target::root`], the path `/`, is the whole library). By id, one in
    /// the trash is found too.
    pub folder: Target,
}

/// What is known about one folder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderInfo {
    /// The folder: id, name, path and dates. `None` for the top of the library.
    pub folder: Option<FolderSummary>,
    /// The folders directly in it, in the order the library shows them.
    pub folders: Vec<FolderSummary>,
    /// The documents directly in it, in the order the library shows them.
    pub documents: Vec<DocumentSummary>,
    /// How many folders it holds, at every depth.
    pub total_folders: u64,
    /// How many documents it holds, at every depth.
    pub total_documents: u64,
    /// How many words those documents have together.
    pub total_words: u64,
}

/// A document or a folder: what [`Library::resolve`](crate::Library::resolve) finds for a
/// name that may be either.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Item {
    /// A document.
    Document {
        /// The document.
        document: DocumentSummary,
    },
    /// A folder.
    Folder {
        /// The folder.
        folder: FolderSummary,
    },
}

impl Item {
    /// The exact name of the item for the next request: `doc:12` or `folder:3`.
    pub fn target(&self) -> Target {
        match self {
            Item::Document { document } => Target::DocumentId(document.id.0),
            Item::Folder { folder } => Target::FolderId(folder.id.0),
        }
    }

    /// Its path in the library.
    pub fn path(&self) -> &str {
        match self {
            Item::Document { document } => &document.path,
            Item::Folder { folder } => &folder.path,
        }
    }

    /// Whether it is in the trash.
    pub fn is_trashed(&self) -> bool {
        match self {
            Item::Document { document } => document.trashed_at.is_some(),
            Item::Folder { folder } => folder.trashed_at.is_some(),
        }
    }
}

/// A request for the text of a document.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReadRequest {
    /// The document. By id, one in the trash is found too.
    pub document: Target,
    /// The form to read it in. Without it: Markdown.
    pub format: TextFormat,
    /// Only the section under this heading: the heading and everything up to the next
    /// heading of the same or a higher level. The heading is named by its text; `#` signs
    /// in front of it (`## Notes`) also say its level.
    pub heading: Option<String>,
    /// Which one, when several headings have that text: its number among them, from 1.
    pub heading_occurrence: Option<usize>,
    /// Also return the comment threads (of the section, when a heading is given).
    pub comments: bool,
}

/// The text of a document, or of one section of it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentText {
    /// The document.
    pub document: DocumentSummary,
    /// The form `content` is in.
    pub format: TextFormat,
    /// The text.
    pub content: String,
    /// The section that was read, when a heading was asked for.
    pub section: Option<HeadingInfo>,
    /// The comment threads, when they were asked for, in the order of the text
    /// (threads without a place last).
    pub comments: Option<Vec<CommentThreadInfo>>,
}

/// A request to find a piece of text in a document: where an edit or a comment with this
/// text would land.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FindRequest {
    /// The document.
    pub document: Target,
    /// The text to find, matched the way an edit matches its `old_text` (see
    /// [`EditRequest`]).
    pub text: String,
    /// The most occurrences to return. Without it: 20.
    pub limit: Option<usize>,
}

/// Where a piece of text occurs in a document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FindResults {
    /// The document.
    pub document: DocumentSummary,
    /// How many times the text occurs.
    pub count: usize,
    /// The occurrences (the first ones, up to the limit), each with its number.
    pub occurrences: Vec<Occurrence>,
    /// How the text was found: as it stands, or read as Markdown.
    pub matched_as: Option<MatchedAs>,
}

// ----- writing documents -----------------------------------------------------------------

/// A request to create a document from Markdown.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CreateDocumentRequest {
    /// The text, as Markdown. May be empty.
    pub markdown: String,
    /// The title. Without it the title follows the text, as it does in the app: the
    /// first heading, else the first line, else "Untitled". A title that is given and is
    /// not the one the text gives is kept the way a rename in the app is kept.
    pub title: Option<String>,
    /// The folder to create it in. Without it: the top of the library.
    pub folder: Option<Target>,
    /// When `folder` is a path that does not exist yet, create it (every missing level)
    /// instead of failing.
    pub create_folders: bool,
    /// Where it goes among the documents of the folder. Without it: last.
    pub placement: Option<Placement>,
    /// Dry run and author.
    #[serde(flatten)]
    pub write: WriteOptions,
}

/// A document that was created (or, in a dry run, would be).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentCreated {
    /// The new document. In a dry run its id is 0.
    pub document: DocumentSummary,
    /// The folders that were created for it, outermost first.
    pub created_folders: Vec<FolderSummary>,
    /// What in the Markdown could not be kept as written (a table, an image), one
    /// sentence each.
    pub warnings: Vec<String>,
    /// About the write itself.
    #[serde(flatten)]
    pub written: Written,
}

/// A request to replace the whole text of a document with Markdown.
///
/// The document is brought to the given text with as few changes as possible, so that an
/// agent can read the Markdown, change it and write it back without disturbing what it
/// did not change:
///
/// - A block (paragraph, heading, list item) that reads the same in the new text is not
///   touched. It keeps its comments and everything Markdown does not say (colours, fonts,
///   alignment, text size).
/// - A block that was edited (the old and the new one are of the same kind and share at
///   least half of their words, or one is nearly all inside the other) is changed word by
///   word. The words that are the same keep their comments and everything else.
/// - Everything else is removed and inserted.
///
/// The settings of the document (page, font, language) stay.
///
/// **Comments.** A thread stays anchored to its text wherever that text is unchanged.
/// Where words are replaced by other words, a thread that began at the first of the
/// replaced words stays on the replacement; a thread on words that are removed without a
/// replacement, or on a block that is removed, loses its place. It is not deleted: it is
/// kept without a place (the app lists such threads last), and the result names it in
/// [`ContentChanged::comments_detached`]. The version kept before the change has every
/// thread where it was.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReplaceDocumentRequest {
    /// The document.
    pub document: Target,
    /// The new text, as Markdown.
    pub markdown: String,
    /// Dry run and author.
    #[serde(flatten)]
    pub write: WriteOptions,
}

/// A request to replace a piece of text in a document, the way an agent's own edit tool
/// replaces a string in a file.
///
/// **What `old_text` is matched against.** The text of the document without markup: the
/// text of its blocks (paragraphs, headings, list items, without list markers or `#`
/// signs), with one line break between two blocks. The match is exact: every character,
/// capitals and punctuation included; line breaks at the ends of `old_text` do not count.
/// When the text does not occur like that, it is read as Markdown and looked for again,
/// so a piece copied from the Markdown form of the document (with `**bold**`, a `## ` in
/// front of a heading, list markers, a blank line between paragraphs) finds its place
/// too; [`ContentChanged::matched_as`] says which it was. [`Library::find`](crate::Library::find)
/// shows what a text would match without changing anything.
///
/// **How many matches.** Exactly one, or the request fails: with
/// [`Error::NoMatch`](crate::Error::NoMatch) for none, with
/// [`Error::NotUnique`](crate::Error::NotUnique) for several (it lists them). `all`
/// replaces every match; `occurrence` chooses one by its number.
///
/// **What `new_text` is.** Markdown. Markup becomes formatting (`*so*` is italic,
/// `[text](url)` a link); several paragraphs become several blocks; a line break at the
/// start or the end of `new_text` ends the block there. When `old_text` was found as it
/// stands, words without markup take the formatting of the text they replace (a replaced
/// word in a bold passage stays bold), and what `old_text` and `new_text` begin and end
/// with in common is not replaced at all: words quoted around the change so that the
/// place is found once keep the formatting each of them has (one of them in bold, a
/// link). When `old_text` was found by reading it as Markdown, the
/// markup of `new_text` is all the formatting the replacement has (`**word**` replaced by
/// `word` is no longer bold). A leading `#`, `-`, `>` or `1.` is only read as a heading,
/// a list item or a quote when the match covers whole blocks: in the middle of a
/// sentence it is the character it is. With `literal` nothing is read as Markdown at
/// all, in `old_text` or in `new_text`; a line break in `new_text` is then a new
/// paragraph.
///
/// **What stays.** Everything outside the replaced text keeps its formatting and its
/// comments, and inside one block only the words that differ between the match and
/// `new_text` are touched. A comment on a replaced word stays on its replacement; a
/// comment on text that is deleted loses its place and is named in
/// [`ContentChanged::comments_detached`]. An empty `new_text` deletes the match, and a
/// block whose whole text is deleted goes with it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EditRequest {
    /// The document.
    pub document: Target,
    /// The text to replace. Must not be empty.
    pub old_text: String,
    /// What to put in its place.
    pub new_text: String,
    /// Replace every occurrence.
    pub all: bool,
    /// Replace only this occurrence: its number among the matches, from 1.
    pub occurrence: Option<usize>,
    /// Take `old_text` and `new_text` as the characters they are: no Markdown is read in
    /// either.
    pub literal: bool,
    /// Dry run and author.
    #[serde(flatten)]
    pub write: WriteOptions,
}

/// Where an insertion goes. See [`InsertRequest`].
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InsertPosition {
    /// At the very start of the document.
    Start,
    /// At the very end of the document (the default): appending.
    #[default]
    End,
    /// Directly before a heading (after the section that precedes it).
    BeforeHeading,
    /// Directly after a heading line, before the text of its section.
    AfterHeading,
    /// At the end of the section under a heading: after its text and its subsections,
    /// before the next heading of the same or a higher level.
    EndOfSection,
}

impl InsertPosition {
    /// Reads a name: `start`, `end`, `before_heading`, `after_heading`, `end_of_section`
    /// (dashes work as underscores do).
    pub fn parse(name: &str) -> Option<InsertPosition> {
        match name.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "start" => Some(InsertPosition::Start),
            "end" => Some(InsertPosition::End),
            "before_heading" => Some(InsertPosition::BeforeHeading),
            "after_heading" => Some(InsertPosition::AfterHeading),
            "end_of_section" => Some(InsertPosition::EndOfSection),
            _ => None,
        }
    }

    /// Whether the position needs a heading to be named.
    pub fn needs_heading(self) -> bool {
        !matches!(self, InsertPosition::Start | InsertPosition::End)
    }
}

/// A request to insert Markdown into a document as new blocks: at the start, at the end
/// (append), or at a heading. Nothing that is there is changed.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct InsertRequest {
    /// The document.
    pub document: Target,
    /// The text to insert, as Markdown.
    pub markdown: String,
    /// Where. Without it: at the end.
    pub position: InsertPosition,
    /// The heading, for the positions that are relative to one: its text, optionally with
    /// the `#` signs of its level in front.
    pub heading: Option<String>,
    /// Which one, when several headings have that text: its number among them, from 1.
    pub heading_occurrence: Option<usize>,
    /// Dry run and author.
    #[serde(flatten)]
    pub write: WriteOptions,
}

/// A request to replace the section under a heading with Markdown.
///
/// The section is everything from the heading to the next heading of the same or a higher
/// level, subsections included. The heading line itself stays unless `include_heading` is
/// set; then the Markdown replaces it too (and should begin with the new heading). The
/// section is brought to the new text the way [`ReplaceDocumentRequest`] brings a
/// document to it: what reads the same is not touched, and comments stay or lose their
/// place by the same rules.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReplaceSectionRequest {
    /// The document.
    pub document: Target,
    /// The heading of the section: its text, optionally with the `#` signs of its level
    /// in front.
    pub heading: String,
    /// Which one, when several headings have that text: its number among them, from 1.
    pub heading_occurrence: Option<usize>,
    /// The new text of the section, as Markdown.
    pub markdown: String,
    /// Replace the heading line too.
    pub include_heading: bool,
    /// Dry run and author.
    #[serde(flatten)]
    pub write: WriteOptions,
}

/// How an edit's `old_text` (or a comment's `quote`) was found.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchedAs {
    /// Character for character, as it was given.
    Text,
    /// After reading it as Markdown: its markup was not part of what was looked for.
    Markdown,
}

/// What a change to the text of a document did (or, in a dry run, would do).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentChanged {
    /// The document as it is after the change.
    pub document: DocumentSummary,
    /// About the write itself. `changed` is false when the request left the text as it
    /// was (the new text is the old one).
    #[serde(flatten)]
    pub written: Written,
    /// For an edit: how many places were replaced. Otherwise 0.
    pub replacements: usize,
    /// For an edit: how `old_text` was found.
    pub matched_as: Option<MatchedAs>,
    /// How many blocks (paragraphs, headings, list items) were added.
    pub blocks_added: usize,
    /// How many blocks were removed.
    pub blocks_removed: usize,
    /// How many blocks were changed in place.
    pub blocks_changed: usize,
    /// The word count before the change.
    pub words_before: u64,
    /// The word count after it.
    pub words_after: u64,
    /// The character count before the change.
    pub characters_before: u64,
    /// The character count after it.
    pub characters_after: u64,
    /// The headings whose sections the change touched, in the order of the document (a
    /// change above the first heading is not under any).
    pub headings: Vec<String>,
    /// What in the given Markdown could not be kept as written, one sentence each.
    pub warnings: Vec<String>,
    /// The comment threads that lost their place through this change: the text they
    /// were anchored to was replaced or removed. They are kept, without a place.
    pub comments_detached: Vec<CommentThreadInfo>,
    /// The version that was kept of the text as it was before the change. `None` in a dry
    /// run and when nothing changed.
    pub snapshot: Option<VersionInfo>,
}

// ----- names, places, the trash ----------------------------------------------------------

/// A request to rename a document. The name is kept the way a rename in the app is kept:
/// the title no longer follows the text.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RenameDocumentRequest {
    /// The document.
    pub document: Target,
    /// The new title. Must not be empty unless `follow_text` is set.
    pub title: String,
    /// Give the title back to the text instead: it becomes the first heading, else the
    /// first line, and follows them from now on. `title` is ignored.
    pub follow_text: bool,
    /// Dry run and author.
    #[serde(flatten)]
    pub write: WriteOptions,
}

/// A request to move a document to another folder, to another place among the documents
/// of its folder, or both.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MoveDocumentRequest {
    /// The document.
    pub document: Target,
    /// The folder to move it to ([`Target::root`], the path `/`, is the top of the
    /// library). Without it the document stays in its folder.
    pub folder: Option<Target>,
    /// When `folder` is a path that does not exist yet, create it.
    pub create_folders: bool,
    /// Where it goes among the documents of that folder. Without it: last when the folder
    /// changes, and where it is when it does not.
    pub placement: Option<Placement>,
    /// Dry run and author.
    #[serde(flatten)]
    pub write: WriteOptions,
}

/// A request to copy a document: the text, the comments and the settings, not the
/// versions.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DuplicateDocumentRequest {
    /// The document to copy.
    pub document: Target,
    /// The title of the copy. Without it: the original's title and " copy", as in the
    /// app.
    pub title: Option<String>,
    /// The folder for the copy. Without it: the original's folder, directly after the
    /// original.
    pub folder: Option<Target>,
    /// Dry run and author.
    #[serde(flatten)]
    pub write: WriteOptions,
}

/// A request to move a document to the trash. It stays there, with its versions, until it
/// is restored or deleted for good.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TrashDocumentRequest {
    /// The document.
    pub document: Target,
    /// Dry run and author.
    #[serde(flatten)]
    pub write: WriteOptions,
}

/// A request to bring a document back from the trash, to where it was (to the top of the
/// library when its folder is gone).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RestoreDocumentRequest {
    /// The document: its id, or its title among the entries of the trash.
    pub document: Target,
    /// Dry run and author.
    #[serde(flatten)]
    pub write: WriteOptions,
}

/// A request to delete a document for good, with every version of it. There is no way
/// back from this, so it is only done when `permanently` is set; without it the request
/// fails with [`Error::ConfirmationRequired`](crate::Error::ConfirmationRequired) and
/// nothing is deleted. To delete in the ordinary sense, move the document to the trash.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DeleteDocumentRequest {
    /// The document, in the trash or not: its id, a library path, or its title among the
    /// entries of the trash.
    pub document: Target,
    /// The confirmation. Must be true.
    pub permanently: bool,
    /// Dry run and author.
    #[serde(flatten)]
    pub write: WriteOptions,
}

/// What a change to a document's name, place or state did.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentChanged {
    /// The document as it is after the change (for a permanent deletion: as it was).
    pub document: DocumentSummary,
    /// The document as it was before, when the change is one of name, place or state.
    /// For a copy: the original. `None` for a permanent deletion.
    pub before: Option<DocumentSummary>,
    /// The folders that were created for a move, outermost first.
    #[serde(default)]
    pub created_folders: Vec<FolderSummary>,
    /// About the write itself.
    #[serde(flatten)]
    pub written: Written,
}

/// A request to create a folder.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CreateFolderRequest {
    /// The path of the new folder, from the top of the library or from `parent`:
    /// `Book/Part one/Drafts`. Levels that are missing are created; a folder that exists
    /// is used as it is (so creating a path twice makes it once).
    pub path: String,
    /// The folder the path starts in. Without it: the top of the library.
    pub parent: Option<Target>,
    /// Dry run and author.
    #[serde(flatten)]
    pub write: WriteOptions,
}

/// A folder that was created (or was there already).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderCreated {
    /// The folder at the end of the path. In a dry run a folder that would be created has
    /// the id 0.
    pub folder: FolderSummary,
    /// Every folder that was created, outermost first. Empty when the whole path existed.
    pub created: Vec<FolderSummary>,
    /// About the write itself.
    #[serde(flatten)]
    pub written: Written,
}

/// A request to rename a folder.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RenameFolderRequest {
    /// The folder.
    pub folder: Target,
    /// The new name. Must not be empty.
    pub name: String,
    /// Dry run and author.
    #[serde(flatten)]
    pub write: WriteOptions,
}

/// A request to move a folder, with everything in it, into another folder, to another
/// place among the folders of its parent, or both.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MoveFolderRequest {
    /// The folder.
    pub folder: Target,
    /// The folder to move it into ([`Target::root`], the path `/`, is the top of the
    /// library). Without it the folder stays where it is.
    pub parent: Option<Target>,
    /// Where it goes among the folders there. Without it: last when the parent changes,
    /// and where it is when it does not.
    pub placement: Option<Placement>,
    /// Dry run and author.
    #[serde(flatten)]
    pub write: WriteOptions,
}

/// A request to move a folder to the trash, with everything in it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TrashFolderRequest {
    /// The folder.
    pub folder: Target,
    /// Dry run and author.
    #[serde(flatten)]
    pub write: WriteOptions,
}

/// A request to bring a folder back from the trash, with everything that went there with
/// it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RestoreFolderRequest {
    /// The folder: its id, or its name among the entries of the trash.
    pub folder: Target,
    /// Dry run and author.
    #[serde(flatten)]
    pub write: WriteOptions,
}

/// A request to delete a folder for good, with every document in it and every version of
/// them. Only done when `permanently` is set, as for
/// [a document](DeleteDocumentRequest).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DeleteFolderRequest {
    /// The folder, in the trash or not.
    pub folder: Target,
    /// The confirmation. Must be true.
    pub permanently: bool,
    /// Dry run and author.
    #[serde(flatten)]
    pub write: WriteOptions,
}

/// What a change to a folder's name, place or state did.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderChanged {
    /// The folder as it is after the change (for a permanent deletion: as it was).
    pub folder: FolderSummary,
    /// The folder as it was before, when the change is one of name, place or state.
    /// `None` for a permanent deletion.
    pub before: Option<FolderSummary>,
    /// How many folders inside it went with it (move, trash, restore, permanent
    /// deletion). 0 for a rename.
    pub folders: u64,
    /// How many documents inside it went with it.
    pub documents: u64,
    /// About the write itself.
    #[serde(flatten)]
    pub written: Written,
}

/// What is in the trash.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrashList {
    /// The entries, the one that went there last first.
    pub entries: Vec<TrashItem>,
}

/// One entry of the trash: a document, or a folder with everything that went to the trash
/// with it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TrashItem {
    /// A document.
    Document {
        /// The document. Its `path` is where it was.
        document: DocumentSummary,
    },
    /// A folder.
    Folder {
        /// The folder. Its `path` is where it was.
        folder: FolderSummary,
        /// How many folders are in it.
        folders: u64,
        /// How many documents are in it.
        documents: u64,
    },
}

/// A request to delete everything in the trash for good. Only done when `permanently` is
/// set, as for [a document](DeleteDocumentRequest).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EmptyTrashRequest {
    /// The confirmation. Must be true.
    pub permanently: bool,
    /// Dry run and author.
    #[serde(flatten)]
    pub write: WriteOptions,
}

/// What emptying the trash deleted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrashEmptied {
    /// How many folders were deleted.
    pub folders: u64,
    /// How many documents were deleted.
    pub documents: u64,
    /// About the write itself.
    #[serde(flatten)]
    pub written: Written,
}

// ----- comments --------------------------------------------------------------------------

/// A request for the comment threads of a document.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ListCommentsRequest {
    /// The document.
    pub document: Target,
    /// Leave the resolved threads out.
    pub open_only: bool,
}

/// The comment threads of a document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommentList {
    /// The document.
    pub document: DocumentSummary,
    /// The threads in the order of the text they are anchored to; threads without a place
    /// last.
    pub threads: Vec<CommentThreadInfo>,
}

/// A request to comment on a piece of the text. The comment shows up in the app's
/// comments panel, anchored to the quoted text, signed with the author's name.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AddCommentRequest {
    /// The document.
    pub document: Target,
    /// The exact text the comment is about, matched the way an edit matches its
    /// `old_text` (see [`EditRequest`]). It has to occur once, unless `occurrence` says
    /// which one.
    pub quote: String,
    /// Which occurrence of the quote, when it occurs several times: its number, from 1.
    pub occurrence: Option<usize>,
    /// The comment. Must not be empty.
    pub body: String,
    /// Dry run and author ([`WriteOptions::author`] signs the comment).
    #[serde(flatten)]
    pub write: WriteOptions,
}

/// A request to reply in a comment thread.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReplyRequest {
    /// The document.
    pub document: Target,
    /// The thread's id.
    pub comment: u64,
    /// The reply. Must not be empty.
    pub body: String,
    /// Dry run and author ([`WriteOptions::author`] signs the reply).
    #[serde(flatten)]
    pub write: WriteOptions,
}

/// A request that names one comment thread: to resolve it, or to reopen it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CommentRequest {
    /// The document.
    pub document: Target,
    /// The thread's id.
    pub comment: u64,
    /// Dry run and author.
    #[serde(flatten)]
    pub write: WriteOptions,
}

/// A request to delete a comment thread, or one reply of it. The text it was about is not
/// touched. A version of the document is kept first, so the thread can be brought back
/// (adding, answering, resolving and reopening keep none: they lose nothing, and each
/// can be taken back with a request of this kind).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DeleteCommentRequest {
    /// The document.
    pub document: Target,
    /// The thread's id.
    pub comment: u64,
    /// Delete only this reply of the thread, by its id.
    pub reply: Option<u64>,
    /// Dry run and author.
    #[serde(flatten)]
    pub write: WriteOptions,
}

/// What a change to a comment thread did.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommentChanged {
    /// The document.
    pub document: DocumentSummary,
    /// The thread as it is after the change (for a deleted thread: as it was). In a dry
    /// run a thread or reply that would be created has the id 0.
    pub thread: CommentThreadInfo,
    /// The id of the reply that was added or deleted.
    pub reply: Option<u64>,
    /// The version that was kept before a thread or a reply was deleted.
    pub snapshot: Option<VersionInfo>,
    /// About the write itself.
    #[serde(flatten)]
    pub written: Written,
}

// ----- versions --------------------------------------------------------------------------

/// A request for the versions of a document.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ListVersionsRequest {
    /// The document. By id, one in the trash is found too.
    pub document: Target,
}

/// The versions of a document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionList {
    /// The document.
    pub document: DocumentSummary,
    /// The versions, newest first.
    pub versions: Vec<VersionInfo>,
}

/// A request for the text of one version.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReadVersionRequest {
    /// The version's id.
    pub version: i64,
    /// The form to read it in. Without it: Markdown.
    pub format: TextFormat,
}

/// The text of one version of a document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionText {
    /// The document it is a version of, as it is now.
    pub document: DocumentSummary,
    /// The version.
    pub version: VersionInfo,
    /// The form `content` is in.
    pub format: TextFormat,
    /// The text as it was.
    pub content: String,
}

/// A request to make a version the text of its document again. The text as it is now is
/// kept as a version first, so restoring can itself be taken back.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RestoreVersionRequest {
    /// The version's id.
    pub version: i64,
    /// The document the version is expected to belong to. When given and it is a version
    /// of another document, the request fails and nothing is restored.
    pub document: Option<Target>,
    /// Dry run and author.
    #[serde(flatten)]
    pub write: WriteOptions,
}

/// What restoring a version did.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionRestored {
    /// The document as it is after the restore.
    pub document: DocumentSummary,
    /// The version that was restored.
    pub restored: VersionInfo,
    /// The version that was kept of the text as it was before the restore.
    pub snapshot: Option<VersionInfo>,
    /// The word count before the restore.
    pub words_before: u64,
    /// The word count after it.
    pub words_after: u64,
    /// About the write itself. `changed` is false when the document already had the text
    /// of that version.
    #[serde(flatten)]
    pub written: Written,
}

// ----- export ----------------------------------------------------------------------------

/// A request to export a document, or a folder as one file, to a file on disk.
///
/// Exactly one of `document` and `folder` is given. A folder is written as one file: its
/// documents in the order of the library, the documents of its subfolders after its own,
/// each starting with its title as a heading when its text does not already.
///
/// Exporting reads the library and writes one file outside it, so it also works while the
/// app is open.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportRequest {
    /// The document to export.
    pub document: Option<Target>,
    /// The folder to export ([`Target::root`], the path `/`, is the whole library).
    pub folder: Option<Target>,
    /// The file to write. A relative path is taken from the current directory; missing
    /// directories are created.
    pub path: PathBuf,
    /// The format. Without it: by the extension of `path` (`.md`, `.docx`, `.txt`), and
    /// Markdown when that says nothing.
    pub format: Option<ExportFormat>,
    /// Replace the file when it exists. Without it an existing file fails the request
    /// with [`Error::FileExists`](crate::Error::FileExists).
    pub overwrite: bool,
    /// Report what would be written, and write nothing.
    pub dry_run: bool,
}

/// What an export wrote.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Exported {
    /// The file, as an absolute path.
    pub path: PathBuf,
    /// The format it is in.
    pub format: ExportFormat,
    /// Its size in bytes.
    pub bytes: u64,
    /// The title of the export: the document's title, or the folder's name.
    pub title: String,
    /// The documents that are in it, in order.
    pub documents: Vec<DocumentSummary>,
    /// Whether this was a dry run: nothing was written.
    pub dry_run: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_read_what_was_typed() {
        assert_eq!(Target::parse("12"), Target::Id(12));
        assert_eq!(Target::parse(" 12 "), Target::Id(12));
        assert_eq!(Target::parse("#12"), Target::Id(12));
        assert_eq!(Target::parse("id:12"), Target::Id(12));
        assert_eq!(Target::parse("doc:12"), Target::DocumentId(12));
        assert_eq!(Target::parse("Document:12"), Target::DocumentId(12));
        assert_eq!(Target::parse("folder:3"), Target::FolderId(3));
        assert_eq!(Target::parse("folder:x"), Target::path("folder:x"));
        assert_eq!(Target::parse("doc:"), Target::path("doc:"));
        assert_eq!(Target::DocumentId(12).to_string(), "doc:12");
        assert_eq!(
            Target::parse(&Target::FolderId(3).to_string()),
            Target::FolderId(3)
        );
        assert!(Target::default().is_empty());
        assert_eq!(Target::parse("/1984"), Target::path("/1984"));
        assert_eq!(Target::parse("Chapter 12"), Target::path("Chapter 12"));
        assert_eq!(Target::parse("-12"), Target::path("-12"));
        assert_eq!(Target::parse("#tag"), Target::path("#tag"));
        assert_eq!(Target::parse(""), Target::path(""));
        // Too long for an id: a name.
        assert_eq!(
            Target::parse("99999999999999999999999"),
            Target::path("99999999999999999999999")
        );
        assert!(Target::root().is_root());
        assert!(!Target::path("/a").is_root());
    }

    #[test]
    fn targets_in_json() {
        let read = |json: &str| serde_json::from_str::<Target>(json).unwrap();
        assert_eq!(read("12"), Target::Id(12));
        assert_eq!(read("\"12\""), Target::Id(12));
        assert_eq!(read("\"Book/Chapter 1\""), Target::path("Book/Chapter 1"));
        assert_eq!(serde_json::to_string(&Target::Id(7)).unwrap(), "7");
        assert_eq!(read("\"doc:7\""), Target::DocumentId(7));
        assert_eq!(
            serde_json::to_string(&Target::FolderId(7)).unwrap(),
            "\"folder:7\""
        );
        assert_eq!(
            serde_json::to_string(&Target::path("a/b")).unwrap(),
            "\"a/b\""
        );

        // A request reads with only the fields that matter.
        let request: EditRequest = serde_json::from_str(
            r#"{"document": 3, "old_text": "a", "new_text": "b", "dry_run": true}"#,
        )
        .unwrap();
        assert_eq!(request.document, Target::Id(3));
        assert!(request.write.dry_run);
        assert!(!request.all);
        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["dry_run"], true);
        assert_eq!(json["document"], 3);

        let placement: Placement = serde_json::from_str(r#"{"after": "Chapter 4"}"#).unwrap();
        assert_eq!(placement, Placement::After(Target::path("Chapter 4")));
        assert_eq!(
            serde_json::from_str::<Placement>("\"first\"").unwrap(),
            Placement::First
        );
        assert_eq!(
            serde_json::to_string(&Placement::Index(2)).unwrap(),
            r#"{"index":2}"#
        );
    }

    #[test]
    fn times_are_written_as_utc() {
        assert_eq!(Time(0).to_rfc3339(), "1970-01-01T00:00:00.000Z");
        assert_eq!(
            Time(1_000_000_000_123).to_rfc3339(),
            "2001-09-09T01:46:40.123Z"
        );
        assert_eq!(Time(-1).to_rfc3339(), "1969-12-31T23:59:59.999Z");
        // 2024-02-29 12:00:00 UTC.
        assert_eq!(
            Time(1_709_208_000_000).to_rfc3339(),
            "2024-02-29T12:00:00.000Z"
        );
        for ms in [
            0,
            1,
            -1,
            1_709_208_000_000,
            1_799_999_999_999,
            951_782_400_000,
            -62_135_596_800_000,
            4_102_444_800_000,
        ] {
            let text = Time(ms).to_rfc3339();
            assert_eq!(Time::parse_rfc3339(&text), Some(Time(ms)), "{text}");
            let json = serde_json::to_string(&Time(ms)).unwrap();
            assert_eq!(serde_json::from_str::<Time>(&json).unwrap(), Time(ms));
        }
        assert_eq!(
            Time::parse_rfc3339("2024-02-29T12:00:00Z"),
            Some(Time(1_709_208_000_000))
        );
        assert_eq!(serde_json::from_str::<Time>("1500").unwrap(), Time(1500));
        assert_eq!(Time::parse_rfc3339("yesterday"), None);
        assert_eq!(Time::parse_rfc3339("2024-13-01T00:00:00Z"), None);
    }

    #[test]
    fn formats_have_names() {
        for format in [
            TextFormat::Markdown,
            TextFormat::PlainText,
            TextFormat::Json,
        ] {
            assert_eq!(TextFormat::parse(format.name()), Some(format));
            assert_eq!(
                serde_json::to_string(&format).unwrap(),
                format!("\"{}\"", format.name())
            );
        }
        assert_eq!(TextFormat::parse("MD"), Some(TextFormat::Markdown));
        assert_eq!(TextFormat::parse("text"), Some(TextFormat::PlainText));
        assert_eq!(
            serde_json::from_str::<TextFormat>("\"text\"").unwrap(),
            TextFormat::PlainText
        );
        for format in [
            ExportFormat::Markdown,
            ExportFormat::Docx,
            ExportFormat::PlainText,
        ] {
            assert_eq!(ExportFormat::parse(format.name()), Some(format));
            assert_eq!(ExportFormat::parse(format.extension()), Some(format));
        }
        assert_eq!(ExportFormat::parse(".DOCX"), Some(ExportFormat::Docx));
        assert_eq!(ExportFormat::parse("pdf"), None);
        assert_eq!(
            InsertPosition::parse("end-of-section"),
            Some(InsertPosition::EndOfSection)
        );
        assert!(InsertPosition::AfterHeading.needs_heading());
        assert!(!InsertPosition::End.needs_heading());
    }
}
