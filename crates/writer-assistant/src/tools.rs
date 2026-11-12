//! The tools the side chat gives the assistant: the app's own MCP server (`writer mcp`
//! of the `writer` tool that ships beside the app) on the app's library, restricted to
//! the tools that only read.
//!
//! The names come from the operations table of `writer-mcp` itself
//! ([`read_only_tools`], [`withheld_tools`]), so a tool added to the server is read-only
//! here exactly when the server says it is, and everything else is withheld.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::Value;
use writer_mcp::protocol::{Effect, ToolHost};

use crate::backend::{ToolDescriber, ToolServer, one_line};

/// The name the server is registered under with the agent: `the-writer`.
pub const LIBRARY_SERVER_NAME: &str = writer_mcp::SERVER_NAME;

/// Every tool of the server with what it does to the library, from the server's own
/// table. The table does not touch the library it is built for.
fn tool_table() -> Vec<(&'static str, Effect)> {
    let toolbox = writer_mcp::Toolbox::new(writer_mcp::Library::at(PathBuf::new()));
    toolbox
        .tools()
        .iter()
        .map(|tool| (tool.name, tool.effect))
        .collect()
}

/// The tools of the library's MCP server that only read: the ones the assistant gets.
pub fn read_only_tools() -> Vec<String> {
    tool_table()
        .into_iter()
        .filter(|(_, effect)| *effect == Effect::Read)
        .map(|(name, _)| name.to_string())
        .collect()
}

/// The tools of the library's MCP server that add or change something: withheld from
/// the assistant.
pub fn withheld_tools() -> Vec<String> {
    tool_table()
        .into_iter()
        .filter(|(_, effect)| *effect != Effect::Read)
        .map(|(name, _)| name.to_string())
        .collect()
}

/// The library's MCP server for the side chat: `<writer> --library <library> mcp`, with
/// its read-only tools allowed and every other tool withheld.
///
/// `writer` is the `writer` tool beside the app's executable
/// (`The Writer.app/Contents/MacOS/writer`; `target/debug/writer` beside a debug build),
/// `library` the data directory of the library the app has open.
pub fn library_tools(writer: impl Into<PathBuf>, library: impl Into<PathBuf>) -> ToolServer {
    library_tools_named(writer, library, &NameBook::new())
}

/// [`library_tools`] whose activity lines name documents and folders by the titles in
/// `names` ("Reading Chapter 2" for a call that names `doc:12`).
pub fn library_tools_named(
    writer: impl Into<PathBuf>,
    library: impl Into<PathBuf>,
    names: &NameBook,
) -> ToolServer {
    let library: PathBuf = library.into();
    let names = names.clone();
    let describe: ToolDescriber =
        Arc::new(move |tool, input| describe_library_tool(tool, input, &names));
    ToolServer {
        name: LIBRARY_SERVER_NAME.to_string(),
        command: writer.into(),
        args: vec![
            "--library".to_string(),
            library.to_string_lossy().into_owned(),
            "mcp".to_string(),
        ],
        env: Vec::new(),
        allowed: read_only_tools(),
        denied: withheld_tools(),
        describe: Some(describe),
    }
}

/// The titles of the documents and folders a conversation knows by id: the open
/// document and the attached items. Shared between the session that fills it and the
/// describer that reads it.
#[derive(Clone, Debug, Default)]
pub struct NameBook {
    names: Arc<Mutex<HashMap<String, String>>>,
}

impl NameBook {
    /// An empty book.
    pub fn new() -> Self {
        NameBook::default()
    }

    /// Notes the title of the item with this reference (`doc:12`, `folder:3`).
    pub fn learn(&self, reference: impl Into<String>, title: impl Into<String>) {
        let title = title.into();
        if title.trim().is_empty() {
            return;
        }
        if let Ok(mut names) = self.names.lock() {
            // A conversation names a handful of items; a runaway caller cannot grow it.
            if names.len() < 4096 {
                names.insert(reference.into(), title);
            }
        }
    }

    /// The title noted for this reference.
    pub fn title(&self, reference: &str) -> Option<String> {
        self.names.lock().ok()?.get(reference).cloned()
    }
}

/// How a tool argument names an item.
enum Named {
    Document,
    Folder,
}

/// The name to show for the item an argument names: its title when the book knows the
/// id, the last part of a path, `None` for an id the book does not know.
fn item_name(value: Option<&Value>, kind: Named, names: &NameBook) -> Option<String> {
    let prefix = match kind {
        Named::Document => "doc",
        Named::Folder => "folder",
    };
    let raw = match value? {
        Value::String(text) => text.trim().to_string(),
        Value::Number(number) => number.to_string(),
        _ => return None,
    };
    if raw.is_empty() {
        return None;
    }
    let digits = raw
        .strip_prefix('#')
        .or_else(|| raw.strip_prefix("id:"))
        .or_else(|| raw.strip_prefix("doc:"))
        .or_else(|| raw.strip_prefix("document:"))
        .or_else(|| raw.strip_prefix("folder:"))
        .unwrap_or(&raw);
    if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
        let reference = if raw.starts_with("folder:") {
            format!("folder:{digits}")
        } else if raw.starts_with("doc") {
            format!("doc:{digits}")
        } else {
            format!("{prefix}:{digits}")
        };
        return names.title(&reference);
    }
    if raw == "/" {
        return None;
    }
    // A path: its last part is the name (`\/` is a slash inside a name).
    let mut last = String::new();
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' if matches!(chars.peek(), Some('/') | Some('\\')) => {
                last.push(chars.next().unwrap_or('/'));
            }
            '/' => {
                if chars.peek().is_some() {
                    last.clear();
                }
            }
            ch => last.push(ch),
        }
    }
    let last = last.trim();
    (!last.is_empty()).then(|| one_line(last, 60))
}

fn quoted(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?.trim();
    (!text.is_empty()).then(|| format!("\u{201c}{}\u{201d}", one_line(text, 48)))
}

/// What a call of a tool of the library is doing, in words for a person: "Reading
/// Chapter 2", "Searching the library for “lighthouse”". `tool` is the server's own name
/// of the tool, `input` its arguments. `None` for a tool this table does not know.
pub fn describe_library_tool(tool: &str, input: &Value, names: &NameBook) -> Option<String> {
    let document = || item_name(input.get("document"), Named::Document, names);
    let folder = || item_name(input.get("folder"), Named::Folder, names);
    let described = match tool {
        "read_document" => match document() {
            Some(name) => format!("Reading {name}"),
            None => "Reading a document".to_string(),
        },
        "find_in_document" => match (document(), quoted(input.get("text"))) {
            (Some(name), Some(text)) => format!("Looking for {text} in {name}"),
            (Some(name), None) => format!("Looking through {name}"),
            (None, Some(text)) => format!("Looking for {text}"),
            (None, None) => "Looking through a document".to_string(),
        },
        "search_library" => match (quoted(input.get("query")), folder()) {
            (Some(query), Some(folder)) => format!("Searching {folder} for {query}"),
            (Some(query), None) => format!("Searching the library for {query}"),
            (None, _) => "Searching the library".to_string(),
        },
        "list_library" => match folder() {
            Some(folder) => format!("Looking through {folder}"),
            None => "Looking through the library".to_string(),
        },
        "recent_documents" => "Looking at the recent documents".to_string(),
        "document_info" => match document() {
            Some(name) => format!("Looking at {name}"),
            None => "Looking at a document".to_string(),
        },
        "folder_info" => match folder() {
            Some(name) => format!("Looking at {name}"),
            None => "Looking at a folder".to_string(),
        },
        "list_comments" => match document() {
            Some(name) => format!("Reading the comments on {name}"),
            None => "Reading the comments".to_string(),
        },
        "list_versions" => match document() {
            Some(name) => format!("Looking at the versions of {name}"),
            None => "Looking at the versions".to_string(),
        },
        "read_version" => "Reading an earlier version".to_string(),
        "list_trash" => "Looking in the trash".to_string(),
        "library_status" => "Checking the library".to_string(),
        _ => return None,
    };
    Some(described)
}

/// Words for a tool nothing describes: its name with spaces, "Using read document".
pub fn describe_unknown_tool(tool: &str) -> String {
    let name = tool.replace(['_', '-'], " ");
    let name = name.trim();
    if name.is_empty() {
        "Using a tool".to_string()
    } else {
        format!("Using {name}")
    }
}
