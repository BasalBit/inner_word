//! The operations of the command layer (`writer-agent`) by name, with JSON arguments.
//!
//! This is the one place where an operation gets its name, its description, the schema of
//! its arguments and its text: the MCP server lists these operations as its tools, and the
//! `writer` command line tool runs the same operations for its subcommands, so the two
//! front ends cannot drift apart.
//!
//! An operation takes a JSON object and answers with a [`Reply`] (text for a reader and
//! the result as data) or a [`Failure`] (the kind of failure, a message that says what to
//! do, and the same as data). The arguments are checked against the schema first
//! ([`crate::args::check`]) and then read into the request type of the command layer,
//! whose field names the schemas use.
//!
//! Three operations are not listed as tools and exist only for the command line, which
//! asks for them with an explicit flag: `delete_document`, `delete_folder` and
//! `empty_trash` erase for good. `list_images` and `list_pages` are the command line's too
//! (`writer images`, `writer pages`): they only read, and a model has the same in the
//! result of `read_document` and `list_library`, so the tools that read stay the ones
//! they were.

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};
use writer_agent::{
    AddCommentRequest, AddImageRequest, AddPageRequest, CommentRequest, CreateDocumentRequest, CreateFolderRequest,
    DeleteCommentRequest, DeleteDocumentRequest, DeleteFolderRequest, DuplicateDocumentRequest,
    EditRequest, EmptyTrashRequest, Error, ErrorKind, ErrorReport, ExportRequest, FindRequest,
    FolderInfoRequest, InfoRequest, InsertRequest, Item, ItemKind, Library, ListCommentsRequest,
    ListVersionsRequest, MoveDocumentRequest, MoveFolderRequest, PagesRequest, ReadRequest,
    ReadVersionRequest,
    RecentRequest, RenameDocumentRequest, RenameFolderRequest, ReplaceDocumentRequest,
    ReplaceSectionRequest, ReplyRequest, RestoreDocumentRequest, RestoreFolderRequest,
    RestoreVersionRequest, SearchRequest, Target, TrashDocumentRequest, TrashFolderRequest,
    TreeRequest,
};

use crate::args::{self, boolean, integer, integer_between, one_of, string, string_map};
use crate::protocol::{Effect, ToolDef, ToolHost, ToolOutput};
use crate::render;

/// What an operation came to.
#[derive(Clone, Debug, PartialEq)]
pub struct Reply {
    /// The result as text. Ends with a line break unless it is the text of a document,
    /// which is given as it is.
    pub text: String,
    /// The result as data: the result type of the command layer as JSON.
    pub data: Value,
}

/// Why an operation did not do what was asked.
#[derive(Clone, Debug, PartialEq)]
pub struct Failure {
    /// The kind of failure; [`ErrorKind::exit_code`] is the exit code of the command line.
    pub kind: ErrorKind,
    /// What happened and what to do about it.
    pub message: String,
    /// The same as data: `{"code": ..., "message": ..., "candidates": [...], "occurrences": [...]}`.
    pub data: Value,
}

impl Failure {
    /// A failure of the given kind with this message.
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Failure {
        let message = message.into();
        let data = json!({ "code": kind.code(), "message": message });
        Failure {
            kind,
            message,
            data,
        }
    }

    /// A request that is wrong in itself.
    pub fn invalid(message: impl Into<String>) -> Failure {
        Failure::new(ErrorKind::Invalid, message)
    }
}

impl From<Error> for Failure {
    fn from(error: Error) -> Failure {
        let mut report = error.report();
        if let Some(message) = ambiguous_items(&report) {
            report.message = message;
        }
        let data = serde_json::to_value(&report)
            .unwrap_or_else(|_| json!({ "code": report.code, "message": report.message }));
        Failure {
            kind: error.kind(),
            message: report.message,
            data,
        }
    }
}

/// The message for a name that fits several documents or folders, with each candidate
/// written the way it can be passed back (`doc:12`, `folder:3`): a bare number would not
/// say which of the two kinds it is.
fn ambiguous_items(report: &ErrorReport) -> Option<String> {
    let all_items = report
        .candidates
        .iter()
        .all(|candidate| matches!(candidate.kind, ItemKind::Document | ItemKind::Folder));
    if report.candidates.is_empty() || !all_items {
        return None;
    }
    let (first_line, _) = report
        .message
        .split_once('\n')
        .unwrap_or((&report.message, ""));
    let subject = first_line
        .split_once(" fits ")
        .map_or("The name", |(subject, _)| subject);
    let mut message = format!(
        "{subject} fits {} items; name one by its id or by a longer path:",
        report.candidates.len()
    );
    for candidate in &report.candidates {
        let kind = if candidate.kind == ItemKind::Folder {
            "folder"
        } else {
            "doc"
        };
        message.push_str(&format!("\n  {kind}:{}  {}", candidate.id, candidate.path));
        if candidate.trashed {
            message.push_str("  (in the trash)");
        }
    }
    Some(message)
}

type Run = fn(&Library, Map<String, Value>) -> Result<Reply, Error>;

/// One operation.
struct Operation {
    name: &'static str,
    title: &'static str,
    description: &'static str,
    effect: Effect,
    idempotent: bool,
    /// Whether the MCP server offers it as a tool.
    listed: bool,
    schema: Value,
    run: Run,
}

// ----- the words the schemas share -------------------------------------------------------

const DOCUMENT: &str = "The document: its id (12 or \"doc:12\") or its path in the library (\"The Salt Road/Part one/Chapter 1\").";
const FOLDER: &str = "The folder: its id (3 or \"folder:3\") or its path in the library (\"The Salt Road/Part one\").";
const FOLDER_OR_TOP: &str =
    "The folder: its id (3 or \"folder:3\") or its path in the library; \"/\" is the top level.";
const MARKDOWN: &str = "The text, as Markdown (headings, lists, quotes, code, *emphasis*, [links](url)). An image is a \
line of its own: ![description](image:<id>) for one the library has, ![description](/absolute/path.png) to \
bring one in from a file.";
const HEADING: &str = "The heading, by its text (\"The harbour\"); # signs in front (\"## The harbour\") also say its level.";
const HEADING_OCCURRENCE: &str =
    "Which one, when several headings have that text: its number among them, from 1.";
const DRY_RUN: &str = "Report what would change, and change nothing.";
const AUTHOR: &str =
    "Your name (\"Claude\"): it labels the version kept before the change and signs a comment.";
const FORMAT: &str = "\"markdown\" (default), \"text\" (plain text without markup) or \"json\" (the document as the app stores it, with every attribute).";
const COMMENT: &str = "The id of the comment thread, as list_comments gives it.";
const FIRST: &str = "Put it before its siblings instead of after them.";
const BEFORE: &str = "Put it directly before this sibling in the destination folder (id or name).";
const AFTER: &str = "Put it directly after this sibling in the destination folder (id or name).";

/// The schema of a reading operation.
fn reads(properties: Vec<(&str, Value)>, required: &[&str]) -> Value {
    args::object(properties, required)
}

/// The schema of a writing operation: its own properties, then `dry_run` and `author`.
fn writes(mut properties: Vec<(&'static str, Value)>, required: &[&str]) -> Value {
    properties.push(("dry_run", boolean(DRY_RUN)));
    properties.push(("author", string(AUTHOR)));
    args::object(properties, required)
}

// ----- from arguments to requests and from results to replies ----------------------------

fn request<T: DeserializeOwned>(arguments: Map<String, Value>) -> Result<T, Error> {
    serde_json::from_value(Value::Object(arguments))
        .map_err(|error| Error::invalid(format!("the arguments cannot be read: {error}")))
}

fn reply<T: Serialize>(result: &T, text: String) -> Result<Reply, Error> {
    let data = serde_json::to_value(result).map_err(|error| {
        Error::invalid(format!("the result cannot be written as JSON: {error}"))
    })?;
    Ok(Reply { text, data })
}

/// Turns `first`, `before` and `after` into the `placement` of a request.
fn placement(arguments: &mut Map<String, Value>) -> Result<(), Error> {
    let first = arguments
        .remove("first")
        .and_then(|first| first.as_bool())
        .unwrap_or(false);
    let before = arguments.remove("before");
    let after = arguments.remove("after");
    let given = usize::from(first) + usize::from(before.is_some()) + usize::from(after.is_some());
    if given > 1 {
        return Err(Error::invalid(
            "give one of \"first\", \"before\" and \"after\", not several",
        ));
    }
    if first {
        arguments.insert("placement".into(), json!("first"));
    } else if let Some(before) = before {
        arguments.insert("placement".into(), json!({ "before": before }));
    } else if let Some(after) = after {
        arguments.insert("placement".into(), json!({ "after": after }));
    }
    Ok(())
}

// ----- the operations ----------------------------------------------------------------------

fn library_status(library: &Library, _: Map<String, Value>) -> Result<Reply, Error> {
    let status = library.status()?;
    reply(&status, render::status(&status))
}

fn list_library(library: &Library, mut arguments: Map<String, Value>) -> Result<Reply, Error> {
    // The top level is the whole library.
    if arguments
        .get("folder")
        .and_then(Value::as_str)
        .is_some_and(|folder| folder.trim() == "/")
    {
        arguments.remove("folder");
    }
    let tree = library.tree(&request::<TreeRequest>(arguments)?)?;
    reply(&tree, render::tree(&tree))
}

fn search_library(library: &Library, arguments: Map<String, Value>) -> Result<Reply, Error> {
    let results = library.search(&request::<SearchRequest>(arguments)?)?;
    reply(&results, render::search(&results))
}

fn recent_documents(library: &Library, arguments: Map<String, Value>) -> Result<Reply, Error> {
    let list = library.recent(&request::<RecentRequest>(arguments)?)?;
    reply(&list, render::documents(&list))
}

fn document_info(library: &Library, arguments: Map<String, Value>) -> Result<Reply, Error> {
    let info = library.info(&request::<InfoRequest>(arguments)?)?;
    reply(&info, render::info(&info))
}

fn folder_info(library: &Library, arguments: Map<String, Value>) -> Result<Reply, Error> {
    let info = library.folder_info(&request::<FolderInfoRequest>(arguments)?)?;
    reply(&info, render::folder_info(&info))
}

fn list_trash(library: &Library, _: Map<String, Value>) -> Result<Reply, Error> {
    let trash = library.trash()?;
    reply(&trash, render::trash(&trash))
}

fn read_document(library: &Library, arguments: Map<String, Value>) -> Result<Reply, Error> {
    let text = library.read(&request::<ReadRequest>(arguments)?)?;
    reply(&text, render::text(&text))
}

fn find_in_document(library: &Library, arguments: Map<String, Value>) -> Result<Reply, Error> {
    let found = library.find(&request::<FindRequest>(arguments)?)?;
    reply(&found, render::found(&found))
}

fn create_document(library: &Library, mut arguments: Map<String, Value>) -> Result<Reply, Error> {
    placement(&mut arguments)?;
    let created = library.create_document(&request::<CreateDocumentRequest>(arguments)?)?;
    reply(&created, render::created(&created))
}

fn replace_document(library: &Library, arguments: Map<String, Value>) -> Result<Reply, Error> {
    let changed = library.replace_document(&request::<ReplaceDocumentRequest>(arguments)?)?;
    reply(
        &changed,
        render::content_changed("Replaced the text of", &changed),
    )
}

fn edit_document(library: &Library, arguments: Map<String, Value>) -> Result<Reply, Error> {
    let changed = library.edit(&request::<EditRequest>(arguments)?)?;
    reply(&changed, render::content_changed("Edited", &changed))
}

fn append_to_document(
    library: &Library,
    mut arguments: Map<String, Value>,
) -> Result<Reply, Error> {
    arguments.insert("position".into(), json!("end"));
    let changed = library.insert(&request::<InsertRequest>(arguments)?)?;
    reply(&changed, render::content_changed("Appended to", &changed))
}

fn insert_in_document(library: &Library, arguments: Map<String, Value>) -> Result<Reply, Error> {
    let changed = library.insert(&request::<InsertRequest>(arguments)?)?;
    reply(&changed, render::content_changed("Inserted into", &changed))
}

fn replace_section(library: &Library, arguments: Map<String, Value>) -> Result<Reply, Error> {
    let changed = library.replace_section(&request::<ReplaceSectionRequest>(arguments)?)?;
    reply(
        &changed,
        render::content_changed("Replaced a section of", &changed),
    )
}

fn add_image(library: &Library, arguments: Map<String, Value>) -> Result<Reply, Error> {
    let changed = library.add_image(&request::<AddImageRequest>(arguments)?)?;
    reply(
        &changed,
        render::content_changed("Added an image to", &changed),
    )
}

/// The images of a document: what `read_document` lists beside the text, by itself.
fn list_images(library: &Library, arguments: Map<String, Value>) -> Result<Reply, Error> {
    let text = library.read(&request::<ReadRequest>(arguments)?)?;
    let data = json!({ "document": text.document, "images": text.images });
    reply(&data, render::images(&text))
}

fn rename_document(library: &Library, arguments: Map<String, Value>) -> Result<Reply, Error> {
    let changed = library.rename_document(&request::<RenameDocumentRequest>(arguments)?)?;
    reply(&changed, render::document_changed("Renamed", &changed))
}

fn move_document(library: &Library, mut arguments: Map<String, Value>) -> Result<Reply, Error> {
    placement(&mut arguments)?;
    let changed = library.move_document(&request::<MoveDocumentRequest>(arguments)?)?;
    reply(&changed, render::document_changed("Moved", &changed))
}

fn duplicate_document(library: &Library, arguments: Map<String, Value>) -> Result<Reply, Error> {
    let changed = library.duplicate_document(&request::<DuplicateDocumentRequest>(arguments)?)?;
    reply(&changed, render::document_copied(&changed))
}

fn trash_document(library: &Library, arguments: Map<String, Value>) -> Result<Reply, Error> {
    let changed = library.trash_document(&request::<TrashDocumentRequest>(arguments)?)?;
    reply(
        &changed,
        render::document_changed("Moved to the trash:", &changed),
    )
}

fn restore_document(library: &Library, arguments: Map<String, Value>) -> Result<Reply, Error> {
    let changed = library.restore_document(&request::<RestoreDocumentRequest>(arguments)?)?;
    reply(
        &changed,
        render::document_changed("Restored from the trash:", &changed),
    )
}

fn delete_document(library: &Library, arguments: Map<String, Value>) -> Result<Reply, Error> {
    let changed = library.delete_document(&request::<DeleteDocumentRequest>(arguments)?)?;
    reply(
        &changed,
        render::document_changed("Erased for good:", &changed),
    )
}

fn create_folder(library: &Library, arguments: Map<String, Value>) -> Result<Reply, Error> {
    let created = library.create_folder(&request::<CreateFolderRequest>(arguments)?)?;
    reply(&created, render::folder_created(&created))
}

fn rename_folder(library: &Library, arguments: Map<String, Value>) -> Result<Reply, Error> {
    let changed = library.rename_folder(&request::<RenameFolderRequest>(arguments)?)?;
    reply(&changed, render::folder_changed("Renamed", &changed))
}

fn move_folder(library: &Library, mut arguments: Map<String, Value>) -> Result<Reply, Error> {
    placement(&mut arguments)?;
    let changed = library.move_folder(&request::<MoveFolderRequest>(arguments)?)?;
    reply(&changed, render::folder_changed("Moved", &changed))
}

fn trash_folder(library: &Library, arguments: Map<String, Value>) -> Result<Reply, Error> {
    let changed = library.trash_folder(&request::<TrashFolderRequest>(arguments)?)?;
    reply(
        &changed,
        render::folder_changed("Moved to the trash:", &changed),
    )
}

fn restore_folder(library: &Library, arguments: Map<String, Value>) -> Result<Reply, Error> {
    let changed = library.restore_folder(&request::<RestoreFolderRequest>(arguments)?)?;
    reply(
        &changed,
        render::folder_changed("Restored from the trash:", &changed),
    )
}

fn delete_folder(library: &Library, arguments: Map<String, Value>) -> Result<Reply, Error> {
    let changed = library.delete_folder(&request::<DeleteFolderRequest>(arguments)?)?;
    reply(
        &changed,
        render::folder_changed("Erased for good:", &changed),
    )
}

fn empty_trash(library: &Library, arguments: Map<String, Value>) -> Result<Reply, Error> {
    let emptied = library.empty_trash(&request::<EmptyTrashRequest>(arguments)?)?;
    reply(&emptied, render::trash_emptied(&emptied))
}

fn list_comments(library: &Library, arguments: Map<String, Value>) -> Result<Reply, Error> {
    let list = library.comments(&request::<ListCommentsRequest>(arguments)?)?;
    reply(&list, render::comments(&list))
}

fn add_comment(library: &Library, arguments: Map<String, Value>) -> Result<Reply, Error> {
    let changed = library.add_comment(&request::<AddCommentRequest>(arguments)?)?;
    reply(&changed, render::comment_changed("Added", &changed))
}

fn reply_to_comment(library: &Library, arguments: Map<String, Value>) -> Result<Reply, Error> {
    let changed = library.reply(&request::<ReplyRequest>(arguments)?)?;
    reply(&changed, render::comment_changed("Replied in", &changed))
}

fn resolve_comment(library: &Library, arguments: Map<String, Value>) -> Result<Reply, Error> {
    let changed = library.resolve_comment(&request::<CommentRequest>(arguments)?)?;
    reply(&changed, render::comment_changed("Resolved", &changed))
}

fn reopen_comment(library: &Library, arguments: Map<String, Value>) -> Result<Reply, Error> {
    let changed = library.reopen_comment(&request::<CommentRequest>(arguments)?)?;
    reply(&changed, render::comment_changed("Reopened", &changed))
}

fn delete_comment(library: &Library, arguments: Map<String, Value>) -> Result<Reply, Error> {
    let only_a_reply = arguments.contains_key("reply");
    let changed = library.delete_comment(&request::<DeleteCommentRequest>(arguments)?)?;
    let did = if only_a_reply {
        "Deleted a reply of"
    } else {
        "Deleted"
    };
    reply(&changed, render::comment_changed(did, &changed))
}

fn list_versions(library: &Library, arguments: Map<String, Value>) -> Result<Reply, Error> {
    let list = library.versions(&request::<ListVersionsRequest>(arguments)?)?;
    reply(&list, render::versions(&list))
}

fn read_version(library: &Library, arguments: Map<String, Value>) -> Result<Reply, Error> {
    let text = library.read_version(&request::<ReadVersionRequest>(arguments)?)?;
    reply(&text, render::version_text(&text))
}

fn restore_version(library: &Library, arguments: Map<String, Value>) -> Result<Reply, Error> {
    let restored = library.restore_version(&request::<RestoreVersionRequest>(arguments)?)?;
    reply(&restored, render::version_restored(&restored))
}

fn add_page(library: &Library, arguments: Map<String, Value>) -> Result<Reply, Error> {
    // What travels to the app is worked out here, always: a caller that hands in a
    // prepared page of its own is refused (the schema has no such argument, and this
    // says why to whoever builds the arguments by hand).
    if arguments.contains_key("prepared") {
        return Err(Error::invalid(PREPARED_REFUSED));
    }
    let added = library.add_page(&request::<AddPageRequest>(arguments)?)?;
    reply(&added, render::page_added(&added))
}

/// Why `prepared` is not taken from a caller.
pub const PREPARED_REFUSED: &str = "\"prepared\" cannot be given: a page is always read, fetched \
and checked by the tool that is asked, from its path";

/// The pages of the library: `writer pages`.
fn list_pages(library: &Library, arguments: Map<String, Value>) -> Result<Reply, Error> {
    let list = library.pages(&request::<PagesRequest>(arguments)?)?;
    reply(&list, render::pages(&list))
}

fn export(library: &Library, arguments: Map<String, Value>) -> Result<Reply, Error> {
    let exported = library.export(&request::<ExportRequest>(arguments)?)?;
    reply(&exported, render::exported(&exported))
}

fn operations() -> Vec<Operation> {
    let read = |name, title, description, schema, run| Operation {
        name,
        title,
        description,
        effect: Effect::Read,
        idempotent: true,
        listed: true,
        schema,
        run,
    };
    let add = |name, title, description, idempotent, schema, run| Operation {
        name,
        title,
        description,
        effect: Effect::Add,
        idempotent,
        listed: true,
        schema,
        run,
    };
    let change = |name, title, description, idempotent, schema, run| Operation {
        name,
        title,
        description,
        effect: Effect::Change,
        idempotent,
        listed: true,
        schema,
        run,
    };
    let hidden = |name, title, description, schema, run| Operation {
        name,
        title,
        description,
        effect: Effect::Change,
        idempotent: true,
        listed: false,
        schema,
        run,
    };
    let hidden_read = |name, title, description, schema, run| Operation {
        name,
        title,
        description,
        effect: Effect::Read,
        idempotent: true,
        listed: false,
        schema,
        run,
    };
    let place = |mut properties: Vec<(&'static str, Value)>| {
        properties.push(("first", boolean(FIRST)));
        properties.push(("before", string(BEFORE)));
        properties.push(("after", string(AFTER)));
        properties
    };

    vec![
        // ----- look around -----------------------------------------------------------
        read(
            "library_status",
            "Library status",
            "Where the library of The Writer is, how much is in it, and whether the app is open right now. \
While the app is open a change is carried out by the app, where the user sees it at once and can undo it; \
the status says when the app takes no changes, and which it is: the user switched that off, or it is an \
older version (app_refuses). Check here when in doubt.",
            reads(vec![], &[]),
            library_status as Run,
        ),
        read(
            "list_library",
            "List the library",
            "The folders and documents of the library as a tree, each with its id, documents with their word \
count and the time of their last change. Start here to see what exists. Give a folder to list only that \
folder; depth limits how many levels of folders are gone into.",
            reads(
                vec![
                    ("folder", string(FOLDER)),
                    (
                        "depth",
                        integer(
                            "How many levels of folders to go into (0: only what is directly there). Default: all.",
                            0,
                        ),
                    ),
                    ("folders_only", boolean("Leave the documents out.")),
                ],
                &[],
            ),
            list_library,
        ),
        read(
            "search_library",
            "Search the library",
            "Full-text search over the titles and the text of every document. Every word must occur; words in \
double quotes must occur as that phrase; the last word also finds words that begin with it; case and accents \
do not matter. Returns the matching documents, best first, each with a snippet around the match. To find \
where a text stands inside one document, use find_in_document.",
            reads(
                vec![
                    ("query", string("The words to look for.")),
                    ("limit", integer("The most hits to return. Default: 20.", 1)),
                    (
                        "folder",
                        string("Only documents in this folder, at any depth (id or path)."),
                    ),
                ],
                &["query"],
            ),
            search_library,
        ),
        read(
            "recent_documents",
            "Recent documents",
            "The documents that were changed last, newest first: what the user is working on.",
            reads(
                vec![(
                    "limit",
                    integer("The most documents to return. Default: 10.", 1),
                )],
                &[],
            ),
            recent_documents,
        ),
        read(
            "document_info",
            "Document info",
            "Everything about one document except its text: path, word and character counts, dates, the outline \
of its headings, how many comments and versions it has, its page and type settings. The outline is where to \
pick a heading for read_document, insert_in_document or replace_section.",
            reads(vec![("document", string(DOCUMENT))], &["document"]),
            document_info,
        ),
        read(
            "folder_info",
            "Folder info",
            "One folder: its path and dates, the folders and documents directly in it, and how much it holds in \
all.",
            reads(vec![("folder", string(FOLDER_OR_TOP))], &["folder"]),
            folder_info,
        ),
        read(
            "list_trash",
            "List the trash",
            "What is in the trash: the documents and folders that were trashed, with their ids. \
restore_document and restore_folder bring them back.",
            reads(vec![], &[]),
            list_trash,
        ),
        // ----- read ------------------------------------------------------------------
        read(
            "read_document",
            "Read a document",
            "The text of a document: as Markdown (default), as plain text without markup, or as the JSON the app \
stores. Give a heading to read only that section. The plain text is what edit_document and add_comment match \
against; an image has no text there (not even its description), so the paragraphs around it read as two \
paragraphs do. An image is a line ![description](image:<id>) in the Markdown: keep that \
line in a text you write back and the image stays, with its size; the result lists the images with what the \
line does not say (width, alignment, the description). You cannot see a picture here: export writes the pictures as files.",
            reads(
                vec![
                    ("document", string(DOCUMENT)),
                    ("format", one_of(FORMAT, &["markdown", "text", "json"])),
                    (
                        "heading",
                        string(
                            "Read only the section under this heading: the heading and everything up to the next heading of the same or a higher level.",
                        ),
                    ),
                    ("heading_occurrence", integer(HEADING_OCCURRENCE, 1)),
                    (
                        "comments",
                        boolean(
                            "Also return the comment threads (of the section, when a heading is given).",
                        ),
                    ),
                ],
                &["document"],
            ),
            read_document,
        ),
        read(
            "find_in_document",
            "Find text in a document",
            "Where a piece of text stands in a document, matched the way edit_document and add_comment match: \
how many times it occurs, and each occurrence with its number, the heading it is under and the line it is on. \
Changes nothing. Use it to make sure a text occurs once before editing or commenting, or to choose an \
occurrence.",
            reads(
                vec![
                    ("document", string(DOCUMENT)),
                    (
                        "text",
                        string("The text to find, exactly as it stands in the document."),
                    ),
                    (
                        "limit",
                        integer("The most occurrences to return. Default: 20.", 1),
                    ),
                ],
                &["document", "text"],
            ),
            find_in_document,
        ),
        // ----- write text ------------------------------------------------------------
        add(
            "create_document",
            "Create a document",
            "Creates a new document from Markdown and returns its id and path. Without a title the title \
follows the text (the first heading, else the first line). The folder must exist unless create_folders is \
true. What the app has no form for (a table, a footnote) is kept as plain text and named in the warnings. \
An image file named by its absolute path, ![description](/absolute/path.png), is read and becomes an image \
of the document; an address on the web stays a link.",
            false,
            writes(
                place(vec![
                    (
                        "markdown",
                        string(
                            "The text of the new document, as Markdown. Leave it out for an empty document.",
                        ),
                    ),
                    (
                        "title",
                        string("The title. Leave it out to let the title follow the text."),
                    ),
                    (
                        "folder",
                        string("The folder to create it in (id or path). Default: the top level."),
                    ),
                    (
                        "create_folders",
                        boolean("Create the folder path when it does not exist yet."),
                    ),
                ]),
                &[],
            ),
            create_document,
        ),
        change(
            "replace_document",
            "Replace the whole text",
            "Replaces the whole text of a document with new Markdown. Paragraphs that read the same are left as \
they are, with their formatting and comments, but for a change to part of a text edit_document, \
replace_section and append_to_document are the better tools. An image stays where its line \
![description](image:<id>) stays, and goes when the line is left out. The text as it was is kept as a \
version first.",
            true,
            writes(
                vec![
                    ("document", string(DOCUMENT)),
                    ("markdown", string(MARKDOWN)),
                ],
                &["document", "markdown"],
            ),
            replace_document,
        ),
        change(
            "edit_document",
            "Edit a document",
            "Replaces an exact piece of text in a document, the way a string replacement edits a file. old_text \
must be in the document exactly as given (words, punctuation, quotation marks, capitals) and exactly once, \
unless all is true or occurrence picks one: include enough of the words around it. It is matched against the \
text without markup; a piece copied from the Markdown form is found too. new_text is Markdown: plain words \
take the formatting of what they replace, an empty text deletes. Formatting and comments outside the replaced \
text stay. An old_text that runs across an image is refused: replace the text before and after it \
separately, or give old_text as Markdown with the line of the image in it. The text as it was is kept as a \
version first.",
            false,
            writes(
                vec![
                    ("document", string(DOCUMENT)),
                    (
                        "old_text",
                        string("The text to replace, exactly as it stands in the document."),
                    ),
                    (
                        "new_text",
                        string(
                            "What to put in its place, as Markdown. Empty deletes the old text.",
                        ),
                    ),
                    ("all", boolean("Replace every occurrence.")),
                    (
                        "occurrence",
                        integer(
                            "Replace only this occurrence: its number among the matches, from 1 (find_in_document lists them).",
                            1,
                        ),
                    ),
                    (
                        "literal",
                        boolean(
                            "Take old_text and new_text as the characters they are: no Markdown is read in either.",
                        ),
                    ),
                ],
                &["document", "old_text", "new_text"],
            ),
            edit_document,
        ),
        add(
            "append_to_document",
            "Append to a document",
            "Adds Markdown as new paragraphs at the end of a document. Nothing that is there is changed.",
            false,
            writes(
                vec![
                    ("document", string(DOCUMENT)),
                    ("markdown", string(MARKDOWN)),
                ],
                &["document", "markdown"],
            ),
            append_to_document,
        ),
        add(
            "insert_in_document",
            "Insert into a document",
            "Inserts Markdown as new paragraphs at one place in a document: at the start, directly before a \
heading, directly after a heading line, or at the end of the section under a heading. Nothing that is there \
is changed. For the very end, use append_to_document.",
            false,
            writes(
                vec![
                    ("document", string(DOCUMENT)),
                    ("markdown", string(MARKDOWN)),
                    (
                        "position",
                        one_of(
                            "Where: \"start\" of the document, \"before_heading\", \"after_heading\" (right under the heading line) or \"end_of_section\" (after the section's text and subsections).",
                            &["start", "before_heading", "after_heading", "end_of_section"],
                        ),
                    ),
                    (
                        "heading",
                        string(
                            "The heading the position refers to; required for every position but \"start\". By its text; # signs in front also say its level.",
                        ),
                    ),
                    ("heading_occurrence", integer(HEADING_OCCURRENCE, 1)),
                ],
                &["document", "markdown", "position"],
            ),
            insert_in_document,
        ),
        change(
            "replace_section",
            "Replace a section",
            "Replaces the section under a heading with new Markdown: everything below the heading down to the \
next heading of the same or a higher level, subsections included. The heading line stays unless \
include_heading is true. The text as it was is kept as a version first.",
            true,
            writes(
                vec![
                    ("document", string(DOCUMENT)),
                    ("heading", string(HEADING)),
                    (
                        "markdown",
                        string("The new text of the section, as Markdown."),
                    ),
                    ("heading_occurrence", integer(HEADING_OCCURRENCE, 1)),
                    (
                        "include_heading",
                        boolean(
                            "Replace the heading line too; the Markdown should then begin with the new heading.",
                        ),
                    ),
                ],
                &["document", "heading", "markdown"],
            ),
            replace_section,
        ),
        add(
            "add_image",
            "Add an image",
            "Adds an image from a file on this Mac to a document, as a block of its own: at the end, at the \
start, or at a heading. The file is read here and stored in the library; PNG, JPEG, GIF, HEIC, TIFF, WebP \
and BMP work, anything else fails with the reason. Without width_percent the image has its own size on the \
page, and it is never wider than the text or taller than a page; without align it is centered. In the \
Markdown of the document it then reads as a line ![description](image:<id>). The text as it was is kept as \
a version first.",
            false,
            writes(
                vec![
                    ("document", string(DOCUMENT)),
                    (
                        "path",
                        string(
                            "The image file: its absolute path on this Mac (\"/Users/ada/Pictures/harbour.jpg\").",
                        ),
                    ),
                    (
                        "description",
                        string(
                            "What the image shows, in a line: for whoever cannot see it, and for the exports.",
                        ),
                    ),
                    (
                        "width_percent",
                        integer_between(
                            "How wide it is drawn: a share of the text column in percent, 5 to 100. Default: its own size.",
                            5,
                            100,
                        ),
                    ),
                    (
                        "align",
                        one_of(
                            "Where it stands on its line. Default: \"center\".",
                            &["left", "center", "right"],
                        ),
                    ),
                    (
                        "position",
                        one_of(
                            "Where: \"end\" of the document (default), \"start\", \"before_heading\", \"after_heading\" (right under the heading line) or \"end_of_section\".",
                            &[
                                "end",
                                "start",
                                "before_heading",
                                "after_heading",
                                "end_of_section",
                            ],
                        ),
                    ),
                    (
                        "heading",
                        string(
                            "The heading the position refers to; required for the three positions at a heading. By its text; # signs in front also say its level.",
                        ),
                    ),
                    ("heading_occurrence", integer(HEADING_OCCURRENCE, 1)),
                ],
                &["document", "path"],
            ),
            add_image,
        ),
        add(
            "add_page",
            "Add pages",
            "Adds a web page to the library from an HTML file or a folder on this Mac, kept whole: its styles, \
scripts, fonts, pictures and sounds are stored, and everything it loads from the web is downloaded now, so that \
it shows the same offline and for good. A folder is taken whole and opens with its index.html (entry names \
another file). A page that needs something that cannot be kept (data from a service, an embedded site) is not \
added: the error page_incomplete says why, and nothing is saved. With source_url, adding the same address again \
changes nothing when the page is the same and updates it (keeping the old one as a version) when it changed; a \
page with that address in the trash is not added again unless include_trashed is true. One page per call: each \
is fetched and checked in WebKit, which takes seconds and can take minutes, longer than some clients wait for \
one call. For many pages call once per page, or run the command line tool's writer page add --list, which \
takes any number. Paths are absolute. The warnings of the result are for the user: pass them on.",
            false,
            writes(
                vec![
                    (
                        "path",
                        string(
                            "The HTML file or the folder of the page: its absolute path on this Mac.",
                        ),
                    ),
                    (
                        "entry",
                        string(
                            "For a folder: the HTML file the page opens with, by its path in the folder. Default: index.html.",
                        ),
                    ),
                    (
                        "title",
                        string(
                            "The title. Default: the page's own title, else its first heading.",
                        ),
                    ),
                    (
                        "source_url",
                        string(
                            "The address the page came from (https://claude.ai/artifact/...): adding it again later updates this page.",
                        ),
                    ),
                    (
                        "externals",
                        string_map(
                            "Pieces you already have as files: each address the page names, with the absolute path of the file that is its content. They are not fetched.",
                        ),
                    ),
                    (
                        "folder",
                        string("The folder to add it in (id or path). Default: the top level."),
                    ),
                    (
                        "create_folders",
                        boolean("Create the folder path when it does not exist yet."),
                    ),
                    (
                        "include_trashed",
                        boolean(
                            "Add the page as new even when a page with this address is in the trash.",
                        ),
                    ),
                ],
                &["path"],
            ),
            add_page,
        ),
        // ----- documents -------------------------------------------------------------
        change(
            "rename_document",
            "Rename a document",
            "Gives a document a new title. It stays where it is (move_document puts it somewhere else). With \
follow_text the title is handed back to the text instead: it becomes the first heading or the first line and \
follows them from then on.",
            true,
            writes(
                vec![
                    ("document", string(DOCUMENT)),
                    ("title", string("The new title.")),
                    (
                        "follow_text",
                        boolean("Let the title follow the text again; title is then not needed."),
                    ),
                ],
                &["document"],
            ),
            rename_document,
        ),
        change(
            "move_document",
            "Move a document",
            "Moves a document into another folder, to another place among the documents of its folder, or both.",
            true,
            writes(
                place(vec![
                    ("document", string(DOCUMENT)),
                    (
                        "folder",
                        string(
                            "The folder to move it into (id or path; \"/\" is the top level). Leave it out to only change its place in its folder.",
                        ),
                    ),
                    (
                        "create_folders",
                        boolean("Create the folder path when it does not exist yet."),
                    ),
                ]),
                &["document"],
            ),
            move_document,
        ),
        add(
            "duplicate_document",
            "Duplicate a document",
            "Copies a document (its text, comments and settings; not its versions) and returns the id of the \
copy. Without a title the copy is named after the original with \" copy\"; without a folder it stands right \
after the original.",
            false,
            writes(
                vec![
                    ("document", string(DOCUMENT)),
                    ("title", string("The title of the copy.")),
                    (
                        "folder",
                        string(
                            "The folder for the copy (id or path). Default: the folder of the original.",
                        ),
                    ),
                ],
                &["document"],
            ),
            duplicate_document,
        ),
        change(
            "trash_document",
            "Move a document to the trash",
            "Moves a document to the trash. Nothing is lost: it stays there with its versions until \
restore_document brings it back. This is how a document is deleted; no tool erases one for good.",
            true,
            writes(vec![("document", string(DOCUMENT))], &["document"]),
            trash_document,
        ),
        add(
            "restore_document",
            "Restore a document from the trash",
            "Brings a document back from the trash to where it was (to the top level when its folder is gone).",
            true,
            writes(
                vec![(
                    "document",
                    string(
                        "The trashed document: its id (12 or \"doc:12\") or its title among the entries of the trash (list_trash shows them).",
                    ),
                )],
                &["document"],
            ),
            restore_document,
        ),
        // ----- folders ---------------------------------------------------------------
        add(
            "create_folder",
            "Create a folder",
            "Creates a folder at a path in the library, with every folder above it that is missing, and returns \
its id. A folder that is there already is returned as it is.",
            true,
            writes(
                vec![
                    (
                        "path",
                        string(
                            "The path of the folder, parts separated by \"/\": \"The Salt Road/Part two\".",
                        ),
                    ),
                    (
                        "parent",
                        string(
                            "The folder the path starts in (id or path). Default: the top level.",
                        ),
                    ),
                ],
                &["path"],
            ),
            create_folder,
        ),
        change(
            "rename_folder",
            "Rename a folder",
            "Gives a folder a new name. It stays where it is, with everything in it.",
            true,
            writes(
                vec![
                    ("folder", string(FOLDER)),
                    ("name", string("The new name.")),
                ],
                &["folder", "name"],
            ),
            rename_folder,
        ),
        change(
            "move_folder",
            "Move a folder",
            "Moves a folder, with everything in it, into another folder, to another place among the folders \
beside it, or both.",
            true,
            writes(
                place(vec![
                    ("folder", string(FOLDER)),
                    (
                        "parent",
                        string(
                            "The folder to move it into (id or path; \"/\" is the top level). Leave it out to only change its place.",
                        ),
                    ),
                ]),
                &["folder"],
            ),
            move_folder,
        ),
        change(
            "trash_folder",
            "Move a folder to the trash",
            "Moves a folder to the trash with every folder and document in it. Nothing is lost: restore_folder \
brings all of it back.",
            true,
            writes(vec![("folder", string(FOLDER))], &["folder"]),
            trash_folder,
        ),
        add(
            "restore_folder",
            "Restore a folder from the trash",
            "Brings a folder back from the trash, with everything that went there with it.",
            true,
            writes(
                vec![(
                    "folder",
                    string(
                        "The trashed folder: its id (3 or \"folder:3\") or its name among the entries of the trash (list_trash shows them).",
                    ),
                )],
                &["folder"],
            ),
            restore_folder,
        ),
        // ----- comments --------------------------------------------------------------
        read(
            "list_comments",
            "List comments",
            "The comment threads of a document: for each its id, the text it is anchored to, the heading it is \
under, whether it is resolved, and the comment and its replies with their authors. This is where the user's \
notes on a text are.",
            reads(
                vec![
                    ("document", string(DOCUMENT)),
                    ("open_only", boolean("Leave the resolved threads out.")),
                ],
                &["document"],
            ),
            list_comments,
        ),
        add(
            "add_comment",
            "Comment on a text",
            "Comments on an exact quote of a document, to review a text instead of rewriting it. The thread \
shows up in the comments panel of the app, anchored to the quoted text and signed with author. quote is \
matched the way edit_document matches old_text and must occur exactly once, unless occurrence picks one.",
            false,
            writes(
                vec![
                    ("document", string(DOCUMENT)),
                    (
                        "quote",
                        string(
                            "The text the comment is about, exactly as it stands in the document.",
                        ),
                    ),
                    ("body", string("What the comment says.")),
                    (
                        "occurrence",
                        integer(
                            "Which occurrence of the quote, when it occurs several times: its number, from 1.",
                            1,
                        ),
                    ),
                ],
                &["document", "quote", "body"],
            ),
            add_comment,
        ),
        add(
            "reply_to_comment",
            "Reply to a comment",
            "Adds a reply to a comment thread, signed with author.",
            false,
            writes(
                vec![
                    ("document", string(DOCUMENT)),
                    ("comment", integer(COMMENT, 0)),
                    ("body", string("What the reply says.")),
                ],
                &["document", "comment", "body"],
            ),
            reply_to_comment,
        ),
        change(
            "resolve_comment",
            "Resolve a comment",
            "Marks a comment thread as resolved. It stays in the document and can be reopened.",
            true,
            writes(
                vec![
                    ("document", string(DOCUMENT)),
                    ("comment", integer(COMMENT, 0)),
                ],
                &["document", "comment"],
            ),
            resolve_comment,
        ),
        change(
            "reopen_comment",
            "Reopen a comment",
            "Opens a resolved comment thread again.",
            true,
            writes(
                vec![
                    ("document", string(DOCUMENT)),
                    ("comment", integer(COMMENT, 0)),
                ],
                &["document", "comment"],
            ),
            reopen_comment,
        ),
        change(
            "delete_comment",
            "Delete a comment",
            "Deletes a comment thread with its replies, or only one reply of it. The text it was about is not \
touched. The document as it was is kept as a version first, so the thread can be brought back.",
            false,
            writes(
                vec![
                    ("document", string(DOCUMENT)),
                    ("comment", integer(COMMENT, 0)),
                    (
                        "reply",
                        integer("Delete only this reply of the thread, by its id.", 0),
                    ),
                ],
                &["document", "comment"],
            ),
            delete_comment,
        ),
        // ----- history ---------------------------------------------------------------
        read(
            "list_versions",
            "List versions",
            "The versions the library keeps of a document, newest first: id, time, word count, and for a \
version kept before a change by an agent the label that says so (\"Before an edit by Claude\"). One is kept \
before every change these tools make to a text.",
            reads(vec![("document", string(DOCUMENT))], &["document"]),
            list_versions,
        ),
        read(
            "read_version",
            "Read a version",
            "The text of one version of a document, as Markdown, plain text or JSON.",
            reads(
                vec![
                    (
                        "version",
                        integer("The id of the version, as list_versions gives it.", 0),
                    ),
                    ("format", one_of(FORMAT, &["markdown", "text", "json"])),
                ],
                &["version"],
            ),
            read_version,
        ),
        change(
            "restore_version",
            "Restore a version",
            "Makes a version the text of its document again. The text as it is now is kept as a version first, \
so a restore can itself be taken back.",
            true,
            writes(
                vec![
                    (
                        "version",
                        integer("The id of the version, as list_versions gives it.", 0),
                    ),
                    (
                        "document",
                        string(
                            "The document the version is expected to belong to (id or path): a check against restoring the wrong one.",
                        ),
                    ),
                ],
                &["version"],
            ),
            restore_version,
        ),
        // ----- export ----------------------------------------------------------------
        change(
            "export",
            "Export to a file",
            "Writes a document, or a folder as one file with its documents in the order of the library, to a \
file on disk: Markdown, DOCX or plain text. Give exactly one of document and folder. Only reads the library, \
so it works while the app is open. A file that exists is not replaced unless overwrite is true. The images \
of a Markdown export are written as files into a folder beside it (\"<name> images\") and linked from the \
text: that is how to look at a picture of a document. A DOCX holds its pictures. A page is written as a \
folder of web files that opens offline (format html; original: the files it was added from), as a PDF the \
way WebKit prints it (format pdf, or a path ending in .pdf), or as its text (.txt); a folder exported as one \
file leaves its pages out.",
            true,
            args::object(
                vec![
                    ("document", string("The document to export (id or path).")),
                    (
                        "folder",
                        string(
                            "The folder to export as one file (id or path; \"/\" is the whole library).",
                        ),
                    ),
                    (
                        "path",
                        string(
                            "The file to write (for a page as a folder: the folder). Use an absolute path; missing directories are created.",
                        ),
                    ),
                    (
                        "format",
                        one_of(
                            "The format. Default: by the ending of path (.md, .docx, .txt, .pdf), else Markdown (for a page: a folder).",
                            &["markdown", "docx", "text", "html", "pdf"],
                        ),
                    ),
                    (
                        "original",
                        boolean(
                            "For a page written as a folder: the files it was added from, as they were, instead of the files it is shown from.",
                        ),
                    ),
                    ("overwrite", boolean("Replace the file when it exists.")),
                    (
                        "dry_run",
                        boolean("Report what would be written, and write nothing."),
                    ),
                ],
                &["path"],
            ),
            export,
        ),
        // ----- for the command line only ---------------------------------------------
        hidden_read(
            "list_images",
            "List the images",
            "The images of a document, in the order of the text: for each the line it is in Markdown, the \
size of the picture, how wide it is drawn and where it stands.",
            reads(vec![("document", string(DOCUMENT))], &["document"]),
            list_images,
        ),
        hidden_read(
            "list_pages",
            "List the pages",
            "The pages of the library, live and in the trash: for each its id, path, the address it came from, \
when its content was taken, its files and size.",
            reads(
                vec![(
                    "folder",
                    string("Only the pages in this folder, at any depth (id or path)."),
                )],
                &[],
            ),
            list_pages,
        ),
        hidden(
            "delete_document",
            "Erase a document",
            "Erases a document for good, with every version of it.",
            writes(
                vec![
                    ("document", string(DOCUMENT)),
                    ("permanently", boolean("The confirmation. Must be true.")),
                ],
                &["document"],
            ),
            delete_document,
        ),
        hidden(
            "delete_folder",
            "Erase a folder",
            "Erases a folder for good, with every document in it.",
            writes(
                vec![
                    ("folder", string(FOLDER)),
                    ("permanently", boolean("The confirmation. Must be true.")),
                ],
                &["folder"],
            ),
            delete_folder,
        ),
        hidden(
            "empty_trash",
            "Empty the trash",
            "Erases everything in the trash for good.",
            writes(
                vec![("permanently", boolean("The confirmation. Must be true."))],
                &[],
            ),
            empty_trash,
        ),
    ]
}

/// The operations of one library.
pub struct Toolbox {
    library: Library,
    operations: Vec<Operation>,
    tools: Vec<ToolDef>,
    /// Whether every call of `add_page` fetches with a cache of its own
    /// ([`Toolbox::fetching_afresh`]).
    afresh: bool,
}

impl Toolbox {
    /// The operations on `library`.
    pub fn new(library: Library) -> Toolbox {
        let operations = operations();
        let tools = operations
            .iter()
            .filter(|operation| operation.listed)
            .map(|operation| ToolDef {
                name: operation.name,
                title: operation.title,
                description: operation.description.to_string(),
                input_schema: operation.schema.clone(),
                effect: operation.effect,
                idempotent: operation.idempotent,
            })
            .collect();
        Toolbox {
            library,
            operations,
            tools,
            afresh: false,
        }
    }

    /// The same operations, where each call of `add_page` gets a fetcher of its own: what
    /// it fetched is let go of when the call ends. For the MCP server, which lives as long
    /// as a session of its client: a library's fetcher keeps every answer it had (up to
    /// 256 MB), which is right for one command that adds many pages and wrong for a
    /// process that adds one page per call for hours.
    pub fn fetching_afresh(mut self) -> Toolbox {
        self.afresh = true;
        self
    }

    /// The library the operations work on.
    pub fn library(&self) -> &Library {
        &self.library
    }

    /// The names of all operations, the ones that are not tools included.
    pub fn operation_names(&self) -> Vec<&'static str> {
        self.operations
            .iter()
            .map(|operation| operation.name)
            .collect()
    }

    /// Runs the operation `name`. `None` when there is no such operation. `client` is the
    /// name of whoever asks, when it is known: it stands in for `author` when the
    /// arguments name none.
    pub fn run(
        &self,
        name: &str,
        arguments: &Map<String, Value>,
        client: Option<&str>,
    ) -> Option<Result<Reply, Failure>> {
        let operation = self
            .operations
            .iter()
            .find(|operation| operation.name == name)?;
        Some(self.run_operation(operation, arguments, client))
    }

    fn run_operation(
        &self,
        operation: &Operation,
        arguments: &Map<String, Value>,
        client: Option<&str>,
    ) -> Result<Reply, Failure> {
        let checked = args::check(&operation.schema, arguments)
            .map_err(|problems| Failure::invalid(format!("{}: {problems}", operation.name)))?;
        let mut arguments = checked.into_map();
        let takes_author = operation.schema["properties"].get("author").is_some();
        if takes_author
            && !arguments.contains_key("author")
            && let Some(client) = client.map(str::trim).filter(|client| !client.is_empty())
        {
            arguments.insert("author".into(), json!(client));
        }
        if self.afresh && operation.name == "add_page" {
            let library = self.library.clone().with_fetcher(fresh_fetcher());
            return (operation.run)(&library, arguments).map_err(Failure::from);
        }
        (operation.run)(&self.library, arguments).map_err(Failure::from)
    }

    /// The document or folder a name stands for, where either may be meant. With
    /// `in_trash` the name is looked up among the entries of the trash.
    pub fn resolve(&self, item: &str, in_trash: bool) -> Result<Item, Failure> {
        self.library
            .resolve(&Target::parse(item), in_trash)
            .map_err(Failure::from)
    }

    /// Runs an operation on an item that may be a document or a folder: `document_operation`
    /// with the item as its `document`, or `folder_operation` with the item as its
    /// `folder`. `adapt` gets the arguments for the kind that was found and may rename
    /// what the two operations call differently.
    pub fn run_on_item(
        &self,
        item: &str,
        in_trash: bool,
        document_operation: &str,
        folder_operation: &str,
        arguments: &Map<String, Value>,
        adapt: impl FnOnce(&Item, &mut Map<String, Value>) -> Result<(), Failure>,
    ) -> Result<Reply, Failure> {
        let found = self.resolve(item, in_trash)?;
        let mut arguments = arguments.clone();
        adapt(&found, &mut arguments)?;
        let (name, key) = match &found {
            Item::Document { .. } => (document_operation, "document"),
            Item::Folder { .. } => (folder_operation, "folder"),
        };
        arguments.insert(key.into(), json!(found.target().to_string()));
        self.run(name, &arguments, None).unwrap_or_else(|| {
            Err(Failure::new(
                ErrorKind::Failed,
                format!("there is no operation {name}"),
            ))
        })
    }
}

/// A fetcher as a library has it by default, with nothing fetched yet: `/usr/bin/curl`,
/// public addresses only, each address once. A debug build lets it ask this Mac with
/// `THE_WRITER_FETCH_LOCAL=1`, as the library's own does (for the tests, whose server is
/// on 127.0.0.1).
fn fresh_fetcher() -> std::sync::Arc<dyn writer_page::Fetcher> {
    let mut curl = writer_page::CurlFetcher::new();
    if cfg!(debug_assertions)
        && std::env::var(writer_agent::FETCH_LOCAL_ENV).is_ok_and(|value| value == "1")
    {
        curl = curl.allowing_local();
    }
    std::sync::Arc::new(writer_page::CachingFetcher::new(std::sync::Arc::new(curl)))
}

impl ToolHost for Toolbox {
    fn tools(&self) -> &[ToolDef] {
        &self.tools
    }

    fn call(
        &mut self,
        name: &str,
        arguments: &Map<String, Value>,
        client: Option<&str>,
    ) -> Option<ToolOutput> {
        let operation = self
            .operations
            .iter()
            .find(|operation| operation.listed && operation.name == name)?;
        Some(match self.run_operation(operation, arguments, client) {
            Ok(reply) => ToolOutput::ok(tool_text(&reply.text), reply.data),
            Err(failure) => {
                ToolOutput::error(tool_error_text(&failure), json!({ "error": failure.data }))
            }
        })
    }
}

/// The text of a result as a model should read it in a tool result. The texts are the
/// ones the command line prints, and where they say how a change is taken back they name
/// a command of the command line; a model that works through this server has the tool of
/// that name instead.
fn tool_text(text: &str) -> String {
    const HISTORY: &str = "\"writer history restore ";
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(HISTORY) {
        let after = &rest[at + HISTORY.len()..];
        let digits = after.bytes().take_while(u8::is_ascii_digit).count();
        if digits > 0 && after[digits..].starts_with('"') {
            out.push_str(&rest[..at]);
            out.push_str(&format!(
                "restore_version with version {}",
                &after[..digits]
            ));
            rest = &after[digits + 1..];
        } else {
            out.push_str(&rest[..at + HISTORY.len()]);
            rest = after;
        }
    }
    out.push_str(rest);
    out.replace(
        "\"writer restore <id>\" brings one back",
        "restore_document or restore_folder brings one back",
    )
    .replace(
        "or add it with --include-trashed",
        "or call add_page again with include_trashed",
    )
    .replace("opens with --entry", "opens with entry")
}

/// What a model is told besides the message when the app is open and did not make a
/// change: the command layer's message says which case it is and what to do, in words for
/// both front ends; this names the tools that go on working.
const WHILE_OPEN: &str = "Everything that only reads (list_library, search_library, read_document, \
list_comments, export) works while the app is open, and so does any tool with dry_run.";

/// The message of a failure as a model should read it in a tool result.
fn tool_error_text(failure: &Failure) -> String {
    match failure.kind {
        ErrorKind::AppIsOpen => format!("{} {WHILE_OPEN}", failure.message),
        ErrorKind::WrongKind if failure.message.contains(" is a page: ") => {
            format!("{} {NEWER_PAGE}", failure.message)
        }
        _ => tool_text(&failure.message),
    }
}

/// What a model is told besides the message when it asked to change the text of a page:
/// the tool that brings a newer copy.
const NEWER_PAGE: &str = "That is add_page with the page's file or folder and its source_url.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tool_result_names_tools_where_the_command_line_names_commands() {
        assert_eq!(
            tool_text(
                "Edited doc:1.\nThe text as it was is kept as version 12 (Before an edit by Claude); \
                 \"writer history restore 12\" brings it back.\n"
            ),
            "Edited doc:1.\nThe text as it was is kept as version 12 (Before an edit by Claude); \
             restore_version with version 12 brings it back.\n"
        );
        assert_eq!(
            tool_text(
                "1 item in the trash:\ndoc:2  Notes\n\"writer restore <id>\" brings one back.\n"
            ),
            "1 item in the trash:\ndoc:2  Notes\nrestore_document or restore_folder brings one back.\n"
        );
        // The text of a document is the text of a document.
        let text = "She typed \"writer history restore\" and \"writer history restore x\".";
        assert_eq!(tool_text(text), text);
        // No result of a tool names a command of the command line.
        for line in include_str!("render.rs").lines() {
            let said = line.trim_start();
            if said.starts_with("//") || !said.contains("\\\"writer ") {
                continue;
            }
            let shown = said.replace("\\\"", "\"").replace("{}", "7");
            assert!(
                !tool_text(&shown).contains("\"writer "),
                "a result still names a command of the command line: {said}"
            );
        }
    }

    fn toolbox() -> (tempfile::TempDir, Toolbox) {
        let dir = tempfile::tempdir().unwrap();
        let toolbox = Toolbox::new(Library::at(dir.path()));
        (dir, toolbox)
    }

    #[test]
    fn every_tool_has_a_sound_definition() {
        let (_dir, toolbox) = toolbox();
        let mut names = std::collections::BTreeSet::new();
        for operation in &toolbox.operations {
            assert!(names.insert(operation.name), "{} twice", operation.name);
            assert!(
                operation
                    .name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '_'),
                "{}: a tool name is lower case with underscores",
                operation.name
            );
            assert!(operation.description.ends_with('.'), "{}", operation.name);
            assert!(operation.description.is_ascii(), "{}", operation.name);
            assert!(
                !operation.description.contains("  "),
                "{}: double space",
                operation.name
            );
            assert_eq!(operation.schema["type"], "object", "{}", operation.name);
            assert_eq!(
                operation.schema["additionalProperties"], false,
                "{}",
                operation.name
            );
            let properties = operation.schema["properties"].as_object().unwrap();
            for (name, property) in properties {
                assert!(
                    property["description"]
                        .as_str()
                        .is_some_and(|d| !d.is_empty()),
                    "{}.{name}",
                    operation.name
                );
                assert!(property["type"].is_string(), "{}.{name}", operation.name);
            }
            // What changes the library can be tried first and is signed.
            if operation.effect != Effect::Read {
                assert!(
                    properties.contains_key("dry_run"),
                    "{} has no dry_run",
                    operation.name
                );
                assert_eq!(
                    properties.contains_key("author"),
                    operation.name != "export",
                    "{}",
                    operation.name
                );
            } else {
                assert!(!properties.contains_key("dry_run"), "{}", operation.name);
            }
        }
        assert_eq!(toolbox.tools().len(), 37);
        // Nothing that erases for good is offered as a tool.
        for tool in toolbox.tools() {
            assert!(
                !tool.name.starts_with("delete_d") && !tool.name.starts_with("delete_f"),
                "{}",
                tool.name
            );
            assert_ne!(tool.name, "empty_trash");
            assert!(
                !tool.input_schema["properties"]
                    .as_object()
                    .unwrap()
                    .contains_key("permanently")
            );
        }
        for hidden in [
            "delete_document",
            "delete_folder",
            "empty_trash",
            "list_images",
            "list_pages",
        ] {
            assert!(toolbox.operation_names().contains(&hidden));
            assert!(toolbox.tools().iter().all(|tool| tool.name != hidden));
        }
        // The tools that only read are the twelve they were: what the app's assistant
        // is given was checked for those, and an image is read through them (a line of
        // Markdown, and the list in the result of read_document).
        let reading: Vec<&str> = toolbox
            .tools()
            .iter()
            .filter(|tool| tool.effect == Effect::Read)
            .map(|tool| tool.name)
            .collect();
        assert_eq!(reading.len(), 12, "{reading:?}");
        let add_image = toolbox
            .tools()
            .iter()
            .find(|tool| tool.name == "add_image")
            .expect("add_image is a tool");
        assert_eq!(add_image.effect, Effect::Add);
        // Both ends of the width are in its schema, as its description says them.
        let width = &add_image.input_schema["properties"]["width_percent"];
        assert_eq!(
            (&width["minimum"], &width["maximum"]),
            (&json!(5), &json!(100))
        );
        // A page is added one per call, and the call changes the library: never a tool
        // of the assistant, never one a client may repeat as if nothing happened.
        let add_page = toolbox
            .tools()
            .iter()
            .find(|tool| tool.name == "add_page")
            .expect("add_page is a tool");
        assert_eq!(add_page.effect, Effect::Add);
        assert!(!add_page.idempotent);
        assert!(add_page.description.contains("One page per call"));
        assert!(add_page.description.contains("writer page add --list"));
        let properties = add_page.input_schema["properties"].as_object().unwrap();
        assert!(!properties.contains_key("prepared"));
        assert_eq!(properties["externals"]["type"], "object");
        let export = toolbox
            .tools()
            .iter()
            .find(|tool| tool.name == "export")
            .unwrap();
        assert_eq!(
            export.input_schema["properties"]["format"]["enum"],
            json!(["markdown", "docx", "text", "html", "pdf"])
        );
        assert_eq!(
            export.input_schema["properties"]["original"]["type"],
            "boolean"
        );
    }

    /// A page that comes prepared is refused, by the schema for a tool call and by the
    /// operation for whoever builds its arguments by hand: what travels to the app is
    /// always worked out by the tool that is asked.
    #[test]
    fn a_page_that_comes_prepared_is_refused() {
        let (dir, mut toolbox) = toolbox();
        std::fs::write(dir.path().join("index.html"), "<h1>Hi</h1>").unwrap();
        let arguments = json!({
            "path": dir.path().join("index.html"),
            "prepared": { "state": "unchanged", "page": 1, "fingerprint": "x" },
        });
        let output = toolbox
            .call("add_page", arguments.as_object().unwrap(), None)
            .unwrap();
        assert!(output.is_error);
        assert_eq!(output.structured["error"]["code"], "invalid");
        let text = &output.text;
        assert!(text.contains("There is no argument \"prepared\""), "{text}");
        let mut by_hand = Map::new();
        by_hand.insert("prepared".into(), json!({}));
        let error = add_page(toolbox.library(), by_hand).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Invalid);
        assert_eq!(error.to_string(), PREPARED_REFUSED);
    }

    #[test]
    fn a_model_is_told_the_tool_where_the_command_line_names_its_options() {
        assert_eq!(
            tool_text(
                "trashed page:9 \u{201c}A\u{201d} is in the trash, so it was not added again: \
                 restore it, or add it with --include-trashed\n"
            ),
            "trashed page:9 \u{201c}A\u{201d} is in the trash, so it was not added again: \
             restore it, or call add_page again with include_trashed\n"
        );
        let failure = Failure::invalid(
            "the folder has 3 pages at its top and no index.html: say which one opens with --entry",
        );
        assert!(tool_error_text(&failure).ends_with("opens with entry"));
        let wrong = Failure::from(Error::WrongKind {
            id: 3,
            name: "\u{201c}Orbitarium\u{201d}".into(),
            is_page: true,
        });
        let text = tool_error_text(&wrong);
        assert!(text.ends_with(NEWER_PAGE), "{text}");
        assert!(text.contains("add_page") && text.contains("source_url"));
        let text_not_page = Failure::from(Error::WrongKind {
            id: 3,
            name: "\u{201c}Chapter 2\u{201d}".into(),
            is_page: false,
        });
        assert!(!tool_error_text(&text_not_page).contains("add_page"));
    }

    #[test]
    fn bad_arguments_are_a_failure_that_names_the_tool_and_the_problem() {
        let (_dir, mut toolbox) = toolbox();
        let arguments = json!({ "document": "x", "old": "a" });
        let failure = toolbox
            .run("edit_document", arguments.as_object().unwrap(), None)
            .unwrap()
            .unwrap_err();
        assert_eq!(failure.kind, ErrorKind::Invalid);
        assert!(
            failure.message.starts_with("edit_document: "),
            "{}",
            failure.message
        );
        assert!(
            failure.message.contains("There is no argument \"old\"."),
            "{}",
            failure.message
        );
        assert!(
            failure.message.contains("\"old_text\" is required."),
            "{}",
            failure.message
        );
        assert_eq!(failure.data["code"], "invalid");

        let output = toolbox
            .call("edit_document", arguments.as_object().unwrap(), None)
            .unwrap();
        assert!(output.is_error);
        assert_eq!(output.structured["error"]["code"], "invalid");
        assert!(
            toolbox
                .run("no_such_operation", &Map::new(), None)
                .is_none()
        );
        assert!(
            toolbox.call("delete_document", &Map::new(), None).is_none(),
            "not a tool"
        );
        assert!(
            toolbox.call("empty_trash", &Map::new(), None).is_none(),
            "not a tool"
        );
    }

    #[test]
    fn placement_words_become_a_placement() {
        let mut arguments = json!({ "first": true, "title": "x" })
            .as_object()
            .unwrap()
            .clone();
        placement(&mut arguments).unwrap();
        assert_eq!(arguments["placement"], "first");
        assert!(!arguments.contains_key("first"));
        let mut arguments = json!({ "after": "Chapter 3", "first": false })
            .as_object()
            .unwrap()
            .clone();
        placement(&mut arguments).unwrap();
        assert_eq!(arguments["placement"], json!({ "after": "Chapter 3" }));
        let mut arguments = json!({ "after": "a", "before": "b" })
            .as_object()
            .unwrap()
            .clone();
        assert!(placement(&mut arguments).is_err());
        let mut arguments = Map::new();
        placement(&mut arguments).unwrap();
        assert!(arguments.is_empty());
    }

    #[test]
    fn the_refusal_while_the_app_is_open_tells_the_model_which_case_it_is_and_what_to_do() {
        use writer_agent::Refusal;
        let failure = |why: Refusal| {
            Failure::from(Error::AppIsOpen {
                pid: Some(7),
                data_dir: "/tmp/x".into(),
                why,
            })
        };
        // The owner switched changes off: the model is told where the switch is.
        let off = failure(Refusal::SwitchedOff);
        assert_eq!(off.kind, ErrorKind::AppIsOpen);
        assert_eq!(off.data["code"], "app_is_open");
        assert_eq!(off.data["refusal"]["why"], "switched_off");
        assert_eq!(off.data["nothing_changed"], true);
        let text = tool_error_text(&off);
        assert!(text.starts_with("The Writer is open (process 7)"), "{text}");
        assert!(text.contains("switched off"), "{text}");
        assert!(text.contains("Ask the user to switch on"), "{text}");
        assert!(text.contains("try again"), "{text}");
        assert!(text.ends_with(WHILE_OPEN), "{text}");
        // An app from before 0.6.0: the user has to quit it, or update it.
        let old = tool_error_text(&failure(Refusal::OldVersion));
        assert!(old.contains("Ask the user to quit The Writer"), "{old}");
        assert!(old.contains("nothing was changed"), "{old}");
        // An answer that did not arrive: nobody is told that nothing was changed.
        let lost = failure(Refusal::AnswerLost);
        assert_eq!(lost.data["nothing_changed"], false);
        assert_eq!(lost.data["refusal"]["why"], "answer_lost");
        let text = tool_error_text(&lost);
        assert!(!text.contains("nothing was changed"), "{text}");
        assert!(text.contains("read the document again"), "{text}");
        // The tools the text names are tools of this server.
        let names = Toolbox::new(Library::at("/tmp/x")).operation_names();
        for tool in [
            "list_library",
            "search_library",
            "read_document",
            "list_comments",
            "export",
        ] {
            assert!(WHILE_OPEN.contains(tool) && names.contains(&tool), "{tool}");
        }
        let other = Failure::invalid("no");
        assert_eq!(tool_error_text(&other), "no");
    }
}
