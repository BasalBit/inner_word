//! Comments, history, export, and the one rule: while the app is open, read but do not
//! write.

use serde_json::json;

use super::harness::Sandbox;

const CHAPTER: &str = "# Chapter 1\n\nThe keeper could not swim. The *Maren* left at dawn.\n\n## The harbour\n\nGulls, rope, tar. The keeper watched.\n";

fn library() -> Sandbox {
    let sandbox = Sandbox::new();
    assert_eq!(
        sandbox.create("Chapter 1", Some("The Salt Road"), CHAPTER),
        "doc:1"
    );
    assert_eq!(
        sandbox.create("Ideas", None, "A lighthouse keeper who cannot swim."),
        "doc:2"
    );
    sandbox
}

#[test]
fn comments_are_added_answered_resolved_and_deleted() {
    let sandbox = library();
    assert_eq!(
        sandbox.ok(&["comments", "doc:1"]),
        "No comments in doc:1 \"The Salt Road/Chapter 1\".\n"
    );

    let added = sandbox.ok(&[
        "comment",
        "add",
        "doc:1",
        "--quote",
        "could not swim",
        "--body",
        "Set this up earlier?",
        "--author",
        "Claude",
    ]);
    assert_eq!(
        added,
        "Added thread 1 in doc:1 \"The Salt Road/Chapter 1\".\n  on: \"could not swim\"\n  Claude: Set this up earlier?\n"
    );
    // The text is untouched by a comment.
    assert_eq!(sandbox.ok(&["get", "doc:1"]), CHAPTER);
    // A quote that is not there, and one that is there twice.
    assert!(
        sandbox
            .fail(
                &[
                    "comment", "add", "doc:1", "--quote", "the mate", "--body", "?"
                ],
                6
            )
            .contains("was not found")
    );
    assert!(
        sandbox
            .fail(
                &[
                    "comment", "add", "doc:1", "--quote", "keeper", "--body", "?"
                ],
                7
            )
            .contains("occurs 2 times")
    );
    let second = sandbox.json(&[
        "comment",
        "add",
        "doc:1",
        "--quote",
        "keeper",
        "--occurrence",
        "2",
        "--body",
        "Who?",
    ]);
    assert_eq!(second["thread"]["id"], 2);
    assert_eq!(
        second["thread"]["author"], "Agent",
        "without --author a comment is signed Agent"
    );
    assert_eq!(second["thread"]["anchored_text"], "keeper");
    assert_eq!(second["thread"]["heading"], "The harbour");

    let replied = sandbox.ok(&[
        "comment", "reply", "doc:1", "1", "--body", "Agreed.", "--author", "Eraldo",
    ]);
    assert!(
        replied
            .starts_with("Replied in thread 1 in doc:1 \"The Salt Road/Chapter 1\" (reply 1).\n"),
        "{replied}"
    );
    assert!(
        replied.contains("    reply 1  Eraldo: Agreed.\n"),
        "{replied}"
    );

    let listed = sandbox.ok(&["comments", "doc:1"]);
    assert_eq!(sandbox.ok(&["comment", "list", "doc:1"]), listed);
    assert_eq!(sandbox.ok(&["comment", "doc:1"]), listed);
    let lines: Vec<&str> = listed.lines().collect();
    assert_eq!(
        lines[0],
        "2 comment threads in doc:1 \"The Salt Road/Chapter 1\":"
    );
    assert_eq!(lines[1], "thread 1  open  under \"Chapter 1\"");
    assert_eq!(lines[2], "  on: \"could not swim\"");
    assert!(
        lines[3].starts_with("  Claude (20") && lines[3].ends_with("): Set this up earlier?"),
        "{listed}"
    );
    assert!(
        lines[4].starts_with("    reply 1  Eraldo (20") && lines[4].ends_with("): Agreed."),
        "{listed}"
    );
    assert_eq!(lines[5], "thread 2  open  under \"The harbour\"");
    let json = sandbox.json(&["comments", "doc:1"]);
    assert_eq!(json["threads"].as_array().unwrap().len(), 2);
    assert_eq!(json["threads"][0]["replies"][0]["author"], "Eraldo");
    assert_eq!(json["document"]["id"], 1);

    assert!(
        sandbox
            .ok(&["comment", "resolve", "doc:1", "1"])
            .starts_with("Resolved thread 1 in doc:1")
    );
    assert_eq!(
        sandbox.json(&["comments", "doc:1"])["threads"][0]["resolved"],
        true
    );
    let open = sandbox.json(&["comments", "doc:1", "--open"]);
    assert_eq!(open["threads"].as_array().unwrap().len(), 1);
    assert_eq!(open["threads"][0]["id"], 2);
    assert!(
        sandbox
            .ok(&["comments", "doc:1"])
            .contains("thread 1  resolved")
    );
    assert!(
        sandbox
            .ok(&["comment", "reopen", "doc:1", "1"])
            .starts_with("Reopened thread 1 in doc:1")
    );
    assert_eq!(sandbox.json(&["info", "doc:1"])["open_comments"], 2);

    // `get --comments` prints the threads after the text.
    let with_comments = sandbox.ok(&["get", "doc:1", "--comments"]);
    assert!(with_comments.starts_with(CHAPTER), "{with_comments}");
    assert!(
        with_comments.contains("\n---\n2 comment threads:\nthread 1  open"),
        "{with_comments}"
    );

    // An edit beside a comment leaves it where it is.
    sandbox.ok(&["edit", "doc:1", "--old", "at dawn", "--new", "at dusk"]);
    assert_eq!(
        sandbox.json(&["comments", "doc:1"])["threads"][0]["anchored_text"],
        "could not swim"
    );

    let dry = sandbox.ok(&["comment", "delete", "doc:1", "1", "--dry-run"]);
    assert!(
        dry.starts_with(
            "Dry run: nothing was changed. The real run would report:\nDeleted thread 1 in doc:1"
        ),
        "{dry}"
    );
    assert!(
        sandbox
            .ok(&["comment", "delete", "doc:1", "1", "--reply", "1"])
            .starts_with("Deleted a reply of thread 1")
    );
    assert_eq!(
        sandbox.json(&["comments", "doc:1"])["threads"][0]["replies"],
        json!([])
    );
    let deleted = sandbox.ok(&["comment", "delete", "doc:1", "1"]);
    assert!(
        deleted.starts_with("Deleted thread 1 in doc:1"),
        "{deleted}"
    );
    assert!(
        deleted.contains("\"writer history restore "),
        "a version was kept: {deleted}"
    );
    assert_eq!(
        sandbox.json(&["comments", "doc:1"])["threads"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(
        sandbox
            .fail(&["comment", "resolve", "doc:1", "1"], 3)
            .contains("there is no comment")
    );
}

#[test]
fn every_change_keeps_a_version_that_can_be_read_and_restored() {
    let sandbox = library();
    assert_eq!(
        sandbox.ok(&["history", "doc:1"]),
        "No versions are kept of doc:1 \"The Salt Road/Chapter 1\".\n"
    );

    sandbox.ok(&[
        "edit", "doc:1", "--old", "at dawn", "--new", "at dusk", "--author", "Claude",
    ]);
    sandbox.ok(&["append", "doc:1", "--content", "The end."]);
    let history = sandbox.ok(&["history", "doc:1"]);
    assert_eq!(sandbox.ok(&["history", "list", "doc:1"]), history);
    assert_eq!(sandbox.ok(&["versions", "doc:1"]), history);
    let lines: Vec<&str> = history.lines().collect();
    assert_eq!(
        lines[0],
        "2 versions of doc:1 \"The Salt Road/Chapter 1\", newest first:"
    );
    assert!(
        lines[1].starts_with("version 2  20")
            && lines[1].ends_with("  20 words  Before an insertion by an agent"),
        "{history}"
    );
    assert!(
        lines[2].starts_with("version 1  20")
            && lines[2].ends_with("  20 words  Before an edit by Claude"),
        "{history}"
    );
    let json = sandbox.json(&["history", "doc:1"]);
    assert_eq!(json["versions"][1]["id"], 1);
    assert_eq!(json["versions"][1]["label"], "Before an edit by Claude");
    assert_eq!(json["versions"][1]["document_id"], 1);

    // A version reads the way a document does.
    assert_eq!(sandbox.ok(&["history", "show", "1"]), CHAPTER);
    assert!(
        sandbox
            .ok(&["history", "show", "2", "--format", "text"])
            .contains("left at dusk.")
    );
    assert_eq!(sandbox.json(&["history", "show", "1"])["content"], CHAPTER);
    assert!(
        sandbox
            .fail(&["history", "show", "99"], 3)
            .contains("there is no version")
    );

    // Restoring keeps what is replaced, so it can be taken back.
    let dry = sandbox.ok(&["history", "restore", "1", "--dry-run"]);
    assert!(
        dry.starts_with(
            "Dry run: nothing was changed. The real run would report:\nRestored version 1 ("
        ),
        "{dry}"
    );
    assert!(sandbox.ok(&["get", "doc:1"]).contains("at dusk"));
    let restored = sandbox.ok(&["history", "restore", "1"]);
    assert!(restored.starts_with("Restored version 1 ("), "{restored}");
    assert!(
        restored.contains("of doc:1 \"The Salt Road/Chapter 1\": 22 -> 20 words.\n"),
        "{restored}"
    );
    assert!(restored.contains("The text as it was is kept as version 3; \"writer history restore 3\" brings it back.\n"), "{restored}");
    assert_eq!(sandbox.ok(&["get", "doc:1"]), CHAPTER);
    sandbox.ok(&["history", "restore", "3"]);
    assert!(
        sandbox
            .ok(&["get", "doc:1"])
            .ends_with("The keeper watched.\n\nThe end.\n")
    );
    // The check against restoring into the wrong document.
    // With the document named too (as the skill writes it), the version must be its own.
    assert_eq!(sandbox.ok(&["history", "show", "doc:1", "1"]), CHAPTER);
    let wrong = sandbox.fail(&["history", "show", "doc:2", "1"], 2);
    assert!(
        wrong.contains(
            "version 1 is a version of doc:1 \"The Salt Road/Chapter 1\", not of doc:2 \"Ideas\""
        ),
        "{wrong}"
    );
    let wrong = sandbox.run(&["history", "restore", "doc:2", "1"]);
    assert_ne!(wrong.code, 0);
    assert_eq!(wrong.stdout, "");
    assert!(wrong.stderr.contains("version 1"), "{}", wrong.stderr);
    assert!(
        sandbox
            .ok(&["history", "restore", "Chapter 1", "1", "--dry-run"])
            .contains("Restored version 1 (")
    );
}

#[test]
fn export_writes_a_document_or_a_folder_to_a_file() {
    let sandbox = library();
    sandbox.create("Chapter 2", Some("The Salt Road"), "# Chapter 2\n\nFog.\n");
    let target = sandbox
        .work()
        .canonicalize()
        .unwrap()
        .join("out/chapter1.md");

    // A relative path is taken from the current directory; missing folders are made.
    let exported = sandbox.ok(&["export", "doc:1", "--to", "out/chapter1.md"]);
    assert_eq!(
        exported,
        format!(
            "Exported \"Chapter 1\" (1 document) to {} (Markdown, {} bytes).\n",
            target.display(),
            CHAPTER.len()
        )
    );
    assert_eq!(std::fs::read_to_string(&target).unwrap(), CHAPTER);
    // A file that is there is not replaced without the flag: exit code 12.
    let exists = sandbox.fail(&["export", "doc:1", "--to", "out/chapter1.md"], 12);
    assert!(exists.contains("exists already"), "{exists}");
    sandbox.ok(&["edit", "doc:1", "--old", "at dawn", "--new", "at dusk"]);
    sandbox.ok(&["export", "doc:1", "--to", "out/chapter1.md", "--overwrite"]);
    assert!(
        std::fs::read_to_string(&target)
            .unwrap()
            .contains("at dusk")
    );

    // A folder as one file, in the order of the library; the format from the ending.
    let book = sandbox.json(&["export", "The Salt Road", "--to", "out/book.docx"]);
    assert_eq!(book["format"], "docx");
    assert_eq!(book["title"], "The Salt Road");
    assert_eq!(book["documents"].as_array().unwrap().len(), 2);
    assert_eq!(book["documents"][0]["title"], "Chapter 1");
    let docx = std::fs::read(sandbox.work().join("out/book.docx")).unwrap();
    assert_eq!(&docx[..2], b"PK", "a DOCX file is a zip archive");
    assert_eq!(book["bytes"], docx.len());

    let text = sandbox.ok(&["export", "folder:1", "--to", "out/book.txt"]);
    assert!(
        text.contains("(2 documents)") && text.contains("(plain text, "),
        "{text}"
    );
    let plain = std::fs::read_to_string(sandbox.work().join("out/book.txt")).unwrap();
    assert!(
        plain.contains("The keeper could not swim.") && plain.contains("Fog."),
        "{plain}"
    );
    assert!(plain.find("Chapter 1").unwrap() < plain.find("Chapter 2").unwrap());
    // The format named, whatever the ending says; the whole library with "/".
    sandbox.ok(&[
        "export",
        "/",
        "--to",
        "out/everything.out",
        "--format",
        "markdown",
    ]);
    let everything = std::fs::read_to_string(sandbox.work().join("out/everything.out")).unwrap();
    assert!(
        everything.contains("# Chapter 1") && everything.contains("A lighthouse keeper"),
        "{everything}"
    );

    let dry = sandbox.ok(&["export", "doc:2", "--to", "out/ideas.md", "--dry-run"]);
    assert!(dry.starts_with("Dry run: nothing was written. The real run would report:\nExported \"Ideas\" (1 document) to "), "{dry}");
    assert!(!sandbox.work().join("out/ideas.md").exists());
    assert!(
        sandbox
            .fail(&["export", "Nothing here", "--to", "out/x.md"], 3)
            .contains("there is no document or folder")
    );
    assert!(
        sandbox
            .fail(
                &["export", "doc:1", "--to", "out/x.pdf", "--format", "pdf"],
                2
            )
            .contains("\"format\" must be one of")
    );
}

#[test]
fn while_the_app_is_open_reading_works_and_every_change_is_refused() {
    let sandbox = library();
    sandbox.ok(&[
        "comment",
        "add",
        "doc:1",
        "--quote",
        "could not swim",
        "--body",
        "Hm.",
    ]);
    sandbox.ok(&["trash", "doc:2"]);
    sandbox.ok(&["append", "doc:1", "--content", "One more line."]);
    let before = sandbox.ok(&["get", "doc:1"]);
    let versions_before = sandbox.ok(&["history", "doc:1"]);

    let app = sandbox.open_the_app();

    // Everything that reads.
    let status = sandbox.json(&["status"]);
    assert_eq!(status["app_is_open"], true);
    assert_eq!(status["app_pid"], std::process::id());
    assert!(
        sandbox
            .ok(&["status"])
            .contains("The Writer is open (process ")
    );
    assert!(sandbox.ok(&["list"]).contains("Chapter 1  doc:1"));
    assert!(sandbox.ok(&["search", "keeper"]).contains("doc:1"));
    assert!(sandbox.ok(&["recent"]).contains("doc:1"));
    assert!(sandbox.ok(&["info", "doc:1"]).contains("comments:  1 open"));
    assert_eq!(sandbox.ok(&["get", "doc:1"]), before);
    assert!(
        sandbox
            .ok(&["find", "doc:1", "keeper"])
            .contains("occurs 2 times")
    );
    assert!(
        sandbox
            .ok(&["comments", "doc:1"])
            .contains("thread 1  open")
    );
    assert_eq!(sandbox.ok(&["history", "doc:1"]), versions_before);
    assert!(sandbox.ok(&["history", "show", "1"]).contains("The keeper"));
    assert!(sandbox.ok(&["list", "--trash"]).contains("doc:2  Ideas"));
    assert!(
        sandbox
            .ok(&["export", "doc:1", "--to", "open.md"])
            .starts_with("Exported \"Chapter 1\"")
    );
    assert_eq!(
        std::fs::read_to_string(sandbox.work().join("open.md")).unwrap(),
        before
    );

    // Everything that writes: exit code 5, a message that says what to do, nothing changed.
    let writes: Vec<Vec<&str>> = vec![
        vec!["create", "New", "--content", "x"],
        vec!["set", "doc:1", "--content", "x"],
        vec!["edit", "doc:1", "--old", "at dawn", "--new", "at dusk"],
        vec!["append", "doc:1", "--content", "x"],
        vec!["insert", "doc:1", "--at-start", "--content", "x"],
        vec!["replace-section", "doc:1", "The harbour", "--content", "x"],
        vec!["duplicate", "doc:1"],
        vec!["mkdir", "New folder"],
        vec!["rename", "doc:1", "Renamed"],
        vec!["rename", "The Salt Road", "Renamed"],
        vec!["move", "doc:1", "/"],
        vec!["trash", "doc:1"],
        vec!["trash", "The Salt Road"],
        vec!["restore", "doc:2"],
        vec!["trash", "doc:2", "--permanently"],
        vec!["empty-trash", "--permanently"],
        vec![
            "comment",
            "add",
            "doc:1",
            "--quote",
            "keeper watched",
            "--body",
            "x",
        ],
        vec!["comment", "reply", "doc:1", "1", "--body", "x"],
        vec!["comment", "resolve", "doc:1", "1"],
        vec!["comment", "delete", "doc:1", "1"],
        vec!["history", "restore", "1"],
    ];
    for args in &writes {
        let message = sandbox.fail(args, 5);
        assert!(
            message.starts_with("writer: The Writer is open (process "),
            "{args:?}: {message}"
        );
        assert!(
            message.contains("so nothing was changed. Ask the user to quit The Writer"),
            "{args:?}: {message}"
        );
    }
    let run = sandbox.run(&["append", "doc:1", "--content", "x", "--json"]);
    assert_eq!(run.code, 5);
    assert_eq!(run.error()["code"], "app_is_open");

    // A dry run only reads, so it works, and says that the real run would be refused.
    let dry = sandbox.ok(&[
        "edit",
        "doc:1",
        "--old",
        "at dawn",
        "--new",
        "at dusk",
        "--dry-run",
    ]);
    assert!(
        dry.starts_with("Dry run: nothing was changed. The real run would report:\nEdited doc:1"),
        "{dry}"
    );
    assert!(
        dry.ends_with(
            "The Writer is open: the real run will be refused until the user quits the app.\n"
        ),
        "{dry}"
    );
    assert_eq!(
        sandbox.json(&["trash", "doc:1", "--dry-run"])["app_is_open"],
        true
    );

    // Nothing of all that changed the library.
    assert_eq!(sandbox.ok(&["get", "doc:1"]), before);
    assert_eq!(sandbox.ok(&["history", "doc:1"]), versions_before);
    let status = sandbox.json(&["status"]);
    assert_eq!(
        (
            status["documents"].as_i64(),
            status["folders"].as_i64(),
            status["trashed"].as_i64()
        ),
        (Some(1), Some(1), Some(1))
    );

    // The app quits: the same command now works.
    drop(app);
    assert_eq!(sandbox.json(&["status"])["app_is_open"], false);
    assert!(
        sandbox
            .ok(&["edit", "doc:1", "--old", "at dawn", "--new", "at dusk"])
            .starts_with("Edited doc:1")
    );
}

#[test]
fn the_tools_of_the_server_do_what_the_commands_do() {
    let sandbox = Sandbox::new();
    let mut client = sandbox.mcp();
    client.initialize("2025-11-25", "Test Agent");

    // An empty world: reading says there is no library, the first write makes it.
    let (message, error) = client.call_error("list_library", json!({}));
    assert_eq!(error["code"], "library_not_found");
    assert!(message.contains("there is no library"), "{message}");
    let (text, created) = client.call_ok(
        "create_document",
        json!({ "title": "Chapter 1", "folder": "The Salt Road", "create_folders": true, "markdown": CHAPTER }),
    );
    assert!(
        text.starts_with("Created doc:1 \"The Salt Road/Chapter 1\" (20 words).\n"),
        "{text}"
    );
    assert_eq!(created["document"]["id"], 1);
    assert_eq!(created["document"]["path"], "The Salt Road/Chapter 1");

    // The text of a tool is the text of the command, its data the command's JSON.
    let (tree, tree_data) = client.call_ok("list_library", json!({}));
    assert_eq!(tree, sandbox.ok(&["list"]));
    assert_eq!(tree_data, sandbox.json(&["list"]));
    let (read, read_data) = client.call_ok("read_document", json!({ "document": "doc:1" }));
    assert_eq!(read, CHAPTER);
    assert_eq!(read_data["content"], CHAPTER);
    assert_eq!(read_data, sandbox.json(&["get", "doc:1"]));
    // A document named by its number, as a number.
    let (section, _) = client.call_ok(
        "read_document",
        json!({ "document": 1, "heading": "The harbour", "format": "text" }),
    );
    assert_eq!(
        section,
        "The harbour\n\nGulls, rope, tar. The keeper watched.\n"
    );

    // An edit: the name the client gave in the handshake labels the version.
    let (edited, edit_data) = client.call_ok(
        "edit_document",
        json!({ "document": "Chapter 1", "old_text": "at dawn", "new_text": "at dusk" }),
    );
    assert!(
        edited.contains("kept as version 1 (Before an edit by Test Agent)"),
        "{edited}"
    );
    assert_eq!(edit_data["replacements"], 1);
    assert_eq!(
        edit_data["snapshot"]["label"],
        "Before an edit by Test Agent"
    );
    let (_, not_unique) = client.call_error(
        "edit_document",
        json!({ "document": 1, "old_text": "keeper", "new_text": "warden" }),
    );
    assert_eq!(not_unique["code"], "not_unique");
    assert_eq!(not_unique["occurrences"].as_array().unwrap().len(), 2);
    let (_, no_match) = client.call_error(
        "edit_document",
        json!({ "document": 1, "old_text": "the mate", "new_text": "x" }),
    );
    assert_eq!(no_match["code"], "no_match");
    let (dry, dry_data) = client.call_ok(
        "edit_document",
        json!({ "document": 1, "old_text": "keeper", "new_text": "warden", "all": true, "dry_run": true }),
    );
    assert!(dry.starts_with("Dry run: nothing was changed."), "{dry}");
    assert_eq!(dry_data["dry_run"], true);
    assert_eq!(dry_data["replacements"], 2);

    // A comment signed by whoever the arguments name.
    let (commented, comment_data) = client.call_ok(
        "add_comment",
        json!({ "document": 1, "quote": "could not swim", "body": "Set this up earlier?", "author": "Claude" }),
    );
    assert!(
        commented.contains("Claude: Set this up earlier?"),
        "{commented}"
    );
    assert_eq!(comment_data["thread"]["id"], 1);
    let (_, comments) = client.call_ok("list_comments", json!({ "document": 1 }));
    assert_eq!(comments["threads"][0]["author"], "Claude");
    assert_eq!(comments["threads"][0]["anchored_text"], "could not swim");
    client.call_ok(
        "reply_to_comment",
        json!({ "document": 1, "comment": 1, "body": "Yes." }),
    );
    let (_, comments) = client.call_ok("list_comments", json!({ "document": 1 }));
    assert_eq!(comments["threads"][0]["replies"][0]["author"], "Test Agent");

    client.call_ok(
        "append_to_document",
        json!({ "document": 1, "markdown": "The end." }),
    );
    client.call_ok(
        "insert_in_document",
        json!({ "document": 1, "markdown": "Before the harbour.", "position": "before_heading", "heading": "The harbour" }),
    );
    client.call_ok(
        "replace_section",
        json!({ "document": 1, "heading": "The harbour", "markdown": "Only fog." }),
    );
    let (read, _) = client.call_ok("read_document", json!({ "document": 1 }));
    assert_eq!(
        read,
        "# Chapter 1\n\nThe keeper could not swim. The *Maren* left at dusk.\n\nBefore the harbour.\n\n## The harbour\n\nOnly fog.\n"
    );

    // Folders, names, places, the trash.
    let (_, folder) = client.call_ok("create_folder", json!({ "path": "The Salt Road/Part two" }));
    assert_eq!(folder["folder"]["id"], 2);
    let (moved, _) = client.call_ok(
        "move_document",
        json!({ "document": 1, "folder": "folder:2" }),
    );
    assert_eq!(
        moved,
        "Moved doc:1 \"The Salt Road/Chapter 1\" -> \"The Salt Road/Part two/Chapter 1\".\n"
    );
    client.call_ok(
        "rename_document",
        json!({ "document": 1, "title": "Chapter One" }),
    );
    client.call_ok(
        "rename_folder",
        json!({ "folder": 2, "name": "Second part" }),
    );
    let (_, copy) = client.call_ok("duplicate_document", json!({ "document": 1 }));
    assert_eq!(
        copy["document"]["path"],
        "The Salt Road/Second part/Chapter One copy"
    );
    client.call_ok(
        "move_folder",
        json!({ "folder": "Second part", "parent": "/" }),
    );
    let (_, info) = client.call_ok("document_info", json!({ "document": 1 }));
    assert_eq!(info["document"]["path"], "Second part/Chapter One");
    let (_, folder_info) = client.call_ok("folder_info", json!({ "folder": "/" }));
    assert_eq!(folder_info["total_documents"], 2);
    let (trashed, _) = client.call_ok("trash_document", json!({ "document": 2 }));
    assert_eq!(
        trashed,
        "Moved to the trash: doc:2 \"Second part/Chapter One copy\".\n"
    );
    let (_, trash) = client.call_ok("list_trash", json!({}));
    assert_eq!(trash["entries"][0]["document"]["id"], 2);
    client.call_ok("restore_document", json!({ "document": 2 }));
    client.call_ok("trash_folder", json!({ "folder": "The Salt Road" }));
    client.call_ok("restore_folder", json!({ "folder": "The Salt Road" }));

    // Looking around.
    let (_, hits) = client.call_ok("search_library", json!({ "query": "fog" }));
    assert_eq!(hits["hits"].as_array().unwrap().len(), 2);
    let (_, recent) = client.call_ok("recent_documents", json!({ "limit": 1 }));
    assert_eq!(recent["documents"].as_array().unwrap().len(), 1);
    let (_, found) = client.call_ok(
        "find_in_document",
        json!({ "document": 1, "text": "keeper" }),
    );
    assert_eq!(found["count"], 1);

    // History and export.
    let (_, versions) = client.call_ok("list_versions", json!({ "document": 1 }));
    let versions = versions["versions"].as_array().unwrap().clone();
    assert!(versions.len() >= 4, "{versions:?}");
    let first = versions.last().unwrap()["id"].as_i64().unwrap();
    let (old, _) = client.call_ok("read_version", json!({ "version": first }));
    assert_eq!(old, CHAPTER);
    let (restored, _) = client.call_ok(
        "restore_version",
        json!({ "version": first, "document": 1 }),
    );
    assert!(restored.starts_with("Restored version 1 ("), "{restored}");
    let target = sandbox.root().join("exported/chapter.md");
    let (_, exported) = client.call_ok(
        "export",
        json!({ "document": 1, "path": target.to_str().unwrap() }),
    );
    assert_eq!(exported["format"], "markdown");
    assert_eq!(std::fs::read_to_string(&target).unwrap(), CHAPTER);
    let (_, exists) = client.call_error(
        "export",
        json!({ "document": 1, "path": target.to_str().unwrap() }),
    );
    assert_eq!(exists["code"], "file_exists");

    // The app opens: tools that read go on working, a tool that writes answers with an
    // ordinary tool error that tells the model what to tell the user.
    let app = sandbox.open_the_app();
    let (_, status) = client.call_ok("library_status", json!({}));
    assert_eq!(status["app_is_open"], true);
    let (read, _) = client.call_ok("read_document", json!({ "document": 1 }));
    assert_eq!(read, CHAPTER);
    let (refused, error) = client.call_error(
        "append_to_document",
        json!({ "document": 1, "markdown": "More." }),
    );
    assert_eq!(error["code"], "app_is_open");
    assert!(
        refused.starts_with("The Writer is open, so nothing was changed."),
        "{refused}"
    );
    assert!(
        refused.contains("Tell the user that The Writer has to be quit"),
        "{refused}"
    );
    assert!(refused.contains("call this tool again"), "{refused}");
    let (dry, dry_data) =
        client.call_ok("trash_document", json!({ "document": 1, "dry_run": true }));
    assert!(
        dry.contains("The Writer is open: the real run will be refused"),
        "{dry}"
    );
    assert_eq!(dry_data["app_is_open"], true);
    let (read, _) = client.call_ok("read_document", json!({ "document": 1 }));
    assert_eq!(read, CHAPTER, "nothing was changed");
    // The app quits; the same server, still running, can write again.
    drop(app);
    let (appended, _) = client.call_ok(
        "append_to_document",
        json!({ "document": 1, "markdown": "More." }),
    );
    assert!(appended.starts_with("Appended to doc:1"), "{appended}");
    client.close();
}
