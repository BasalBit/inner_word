//! The tools of the side chat: the read-only tools of the library's own server, and the
//! words for what a call is doing.

use innerword_assistant::{
    LIBRARY_SERVER_NAME, NameBook, describe_library_tool, describe_unknown_tool, library_observer,
    library_tools, library_tools_named, read_only_tools, withheld_tools,
};
use serde_json::json;

#[test]
fn the_lists_are_the_servers_own_and_every_tool_is_on_exactly_one() {
    let allowed = read_only_tools();
    let withheld = withheld_tools();
    // Every tool the server lists is either read-only or withheld.
    let all: Vec<String> = {
        use innerword_mcp::protocol::ToolHost;
        innerword_mcp::Toolbox::new(innerword_mcp::Library::at("unused"))
            .tools()
            .iter()
            .map(|tool| tool.name.to_string())
            .collect()
    };
    assert_eq!(allowed.len() + withheld.len(), all.len());
    for tool in &all {
        assert!(allowed.contains(tool) != withheld.contains(tool), "{tool}");
    }
    // The read-only ones are those the server marks so, and the names say it too.
    assert_eq!(allowed.len(), 12);
    for tool in &allowed {
        let reads = ["read_", "list_", "find_", "search_", "recent_"]
            .iter()
            .any(|prefix| tool.starts_with(prefix))
            || tool.ends_with("_info")
            || tool.ends_with("_status");
        assert!(reads, "{tool} does not look like a tool that only reads");
    }
    for tool in &withheld {
        let writes = [
            "create_",
            "append_",
            "insert_",
            "duplicate_",
            "restore_",
            "add_",
            "reply_",
            "replace_",
            "edit_",
            "rename_",
            "move_",
            "trash_",
            "resolve_",
            "reopen_",
            "delete_",
            "export",
        ]
        .iter()
        .any(|prefix| tool.starts_with(prefix));
        assert!(
            writes,
            "{tool} is withheld but does not look like a tool that changes something"
        );
    }
}

#[test]
fn the_server_is_the_bundled_tool_on_the_apps_library() {
    let server = library_tools(
        "/Applications/Inner Word.app/Contents/MacOS/innerword",
        "/Users/someone/Library/Application Support/Inner Word",
    );
    assert_eq!(server.name, LIBRARY_SERVER_NAME);
    assert_eq!(server.name, "inner-word");
    assert_eq!(
        server.command.to_str(),
        Some("/Applications/Inner Word.app/Contents/MacOS/innerword")
    );
    assert_eq!(
        server.args,
        [
            "--library",
            "/Users/someone/Library/Application Support/Inner Word",
            "mcp"
        ]
    );
    assert!(server.env.is_empty());
    assert_eq!(server.allowed, read_only_tools());
    assert_eq!(server.denied, withheld_tools());
    assert!(server.describe.is_some());
    assert!(server.observe.is_some());
}

#[test]
fn a_call_reads_as_what_the_assistant_is_doing() {
    let names = NameBook::new();
    names.learn("doc:12", "Chapter 2");
    names.learn("folder:3", "Part two");
    let says =
        |tool: &str, input: serde_json::Value| describe_library_tool(tool, &input, &names).unwrap();
    // By id: the title the conversation knows.
    assert_eq!(
        says("read_document", json!({"document": "doc:12"})),
        "Reading Chapter 2"
    );
    assert_eq!(
        says("read_document", json!({"document": 12})),
        "Reading Chapter 2"
    );
    assert_eq!(
        says(
            "read_document",
            json!({"document": "12", "heading": "The harbour"})
        ),
        "Reading Chapter 2"
    );
    // By path: its last part.
    assert_eq!(
        says(
            "read_document",
            json!({"document": "The Salt Road/Part one/Chapter 7"})
        ),
        "Reading Chapter 7"
    );
    assert_eq!(
        says("read_document", json!({"document": "/Preface"})),
        "Reading Preface"
    );
    assert_eq!(
        says("read_document", json!({"document": "AC\\/DC notes"})),
        "Reading AC/DC notes"
    );
    // An id nobody has named yet: no number is shown to a writer.
    assert_eq!(
        says("read_document", json!({"document": "doc:99"})),
        "Reading a document"
    );
    assert_eq!(says("read_document", json!({})), "Reading a document");
    assert_eq!(
        says(
            "find_in_document",
            json!({"document": "doc:12", "text": "the heron"})
        ),
        "Looking for “the heron” in Chapter 2"
    );
    assert_eq!(
        says("search_library", json!({"query": "lighthouse"})),
        "Searching the library for “lighthouse”"
    );
    assert_eq!(
        says(
            "search_library",
            json!({"query": "lighthouse", "folder": "folder:3"})
        ),
        "Searching Part two for “lighthouse”"
    );
    assert_eq!(
        says("list_library", json!({})),
        "Looking through the library"
    );
    assert_eq!(
        says("list_library", json!({"folder": "folder:3"})),
        "Looking through Part two"
    );
    assert_eq!(
        says("list_library", json!({"folder": "/"})),
        "Looking through the library"
    );
    assert_eq!(
        says("document_info", json!({"document": "doc:12"})),
        "Looking at Chapter 2"
    );
    assert_eq!(
        says("folder_info", json!({"folder": 3})),
        "Looking at Part two"
    );
    assert_eq!(
        says("list_comments", json!({"document": "doc:12"})),
        "Reading the comments on Chapter 2"
    );
    assert_eq!(
        says("list_versions", json!({"document": "doc:12"})),
        "Looking at the versions of Chapter 2"
    );
    assert_eq!(
        says("read_version", json!({"version": 87})),
        "Reading an earlier version"
    );
    assert_eq!(
        says("recent_documents", json!({})),
        "Looking at the recent documents"
    );
    // A long query is cut, on one line.
    let long = says(
        "search_library",
        json!({"query": "a very long question about\nthe lighthouse keeper and her dog and the sea"}),
    );
    assert!(
        long.chars().count() < 90 && !long.contains('\n') && long.ends_with("…”"),
        "{long}"
    );
    // Every read-only tool has words of its own.
    for tool in read_only_tools() {
        assert!(
            describe_library_tool(&tool, &json!({}), &names).is_some(),
            "{tool} has no description"
        );
    }
    // A tool this table does not know has its name, with spaces.
    assert_eq!(
        describe_library_tool("count_words", &json!({}), &names),
        None
    );
    assert_eq!(describe_unknown_tool("count_words"), "Using count words");
    assert_eq!(describe_unknown_tool(""), "Using a tool");
}

#[test]
fn what_a_tool_returns_teaches_the_names_behind_the_ids() {
    let names = NameBook::new();
    let server = library_tools_named("/innerword", "/library", &names);
    let observe = server.observe.clone().unwrap();
    let describe = server.describe.clone().unwrap();
    assert_eq!(
        describe("read_document", &json!({"document": "doc:1"})).as_deref(),
        Some("Reading a document")
    );
    // The result of list_library, as the server gives it.
    observe(
        "list_library",
        &json!({}),
        &json!({"folders": [{"id": 1, "name": "The Salt Road", "path": "The Salt Road", "document_count": 2,
            "documents": [
                {"id": 1, "title": "Chapter 2", "path": "The Salt Road/Chapter 2", "words": 31},
                {"id": 2, "title": "Chapter 3", "path": "The Salt Road/Chapter 3", "words": 40}],
            "folders": []}], "documents": []}),
    );
    assert_eq!(
        describe("read_document", &json!({"document": "doc:1"})).as_deref(),
        Some("Reading Chapter 2")
    );
    assert_eq!(
        describe("read_document", &json!({"document": 2})).as_deref(),
        Some("Reading Chapter 3")
    );
    assert_eq!(
        describe("list_library", &json!({"folder": "folder:1"})).as_deref(),
        Some("Looking through The Salt Road")
    );
    // Search hits name their documents too; a result without data teaches nothing.
    observe(
        "search_library",
        &json!({"query": "x"}),
        &json!({"hits": [{"document": {"id": 7, "title": "Notes"}, "snippet": "x"}]}),
    );
    assert_eq!(names.title("doc:7").as_deref(), Some("Notes"));
    library_observer(&names)("read_document", &json!({}), &serde_json::Value::Null);
    assert_eq!(names.title("doc:99"), None);
    assert_eq!(names.title("folder:1").as_deref(), Some("The Salt Road"));
}
