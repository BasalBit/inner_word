//! While The Writer is open: a change is carried out by the app. A thread of the test
//! stands in for the app (`harness::LiveApp`); the tool is the built binary, run as a
//! process, and the MCP server is that binary too.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use super::harness::Sandbox;

const CHAPTER: &str = "# Chapter 1\n\nThe keeper could not swim. The *Maren* left at dawn.\n\n## The harbour\n\nGulls, rope, tar. The keeper watched.\n";

/// A library with something for every command to work on, made while no app is open:
/// `doc:1` "The Salt Road/Chapter 1", `doc:2` "Ideas", `doc:3` "Gone" in the trash, and
/// the folder "Attic" (with `doc:4`) in the trash.
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
    assert_eq!(sandbox.create("Gone", None, "In the trash."), "doc:3");
    sandbox.ok(&["trash", "doc:3"]);
    assert_eq!(sandbox.create("Old box", Some("Attic"), "Dust."), "doc:4");
    sandbox.ok(&["trash", "Attic"]);
    sandbox
}

/// The commands of the tool by the name their own help gives them, each with whether its
/// help says that it changes the library. Read from the overview, so a command that is
/// added later is in the list.
fn commands(sandbox: &Sandbox) -> BTreeMap<String, bool> {
    let overview = sandbox.ok(&["--help"]);
    let mut commands = BTreeMap::new();
    let lines = overview
        .lines()
        .skip_while(|line| !line.starts_with("Look around"))
        .take_while(|line| !line.starts_with("Options for every command"));
    for line in lines.filter(|line| line.starts_with("  ")) {
        let mut words = line.split_whitespace();
        let first = words.next().expect("a command");
        let second = words
            .next()
            .filter(|word| word.chars().all(|c| c.is_ascii_lowercase() || c == '-'));
        // A second word that is a word may be the second half of the name ("comment
        // add"); the help of the command says what the command is called.
        let mut args = vec![first];
        args.extend(second);
        args.push("--help");
        let help = sandbox.ok(&args);
        let name = help
            .lines()
            .next()
            .and_then(|line| line.strip_prefix("writer "))
            .and_then(|line| line.split_once(": "))
            .map(|(name, _)| name.to_string())
            .unwrap_or_else(|| panic!("the help of {args:?} does not begin with its name: {help}"));
        commands.insert(name, help.contains("This command changes the library."));
    }
    assert!(commands.len() >= 35, "{commands:?}");
    commands
}

#[test]
fn every_command_that_changes_the_library_is_carried_out_by_the_app_while_it_is_open() {
    let sandbox = library();
    let app = sandbox.open_the_app();

    // The app is open and takes changes: the status says so.
    let status = sandbox.json(&["status"]);
    assert_eq!(status["app_is_open"], true);
    assert_eq!(status["app_takes_changes"], true);
    assert_eq!(status["app_pid"], std::process::id());
    let said = sandbox.ok(&["status"]);
    assert!(
        said.contains(&format!(
            "The Writer is open (process {}): reading works, and a change is carried out by \
             the app, where the user sees it at once and can undo it.\n",
            std::process::id()
        )),
        "{said}"
    );
    app.served();

    // One command line (or more) for every command that changes the library, with the
    // operation the app is asked for. `{version}` is the oldest version of doc:1.
    let lines: Vec<(&str, Vec<&str>, &str)> = vec![
        (
            "comment add",
            vec![
                "comment",
                "add",
                "doc:1",
                "--quote",
                "could not swim",
                "--body",
                "Hm.",
            ],
            "add_comment",
        ),
        (
            "comment reply",
            vec!["comment", "reply", "doc:1", "1", "--body", "Yes."],
            "reply",
        ),
        (
            "comment resolve",
            vec!["comment", "resolve", "doc:1", "1"],
            "resolve_comment",
        ),
        (
            "comment reopen",
            vec!["comment", "reopen", "doc:1", "1"],
            "reopen_comment",
        ),
        (
            "comment delete",
            vec!["comment", "delete", "doc:1", "1"],
            "delete_comment",
        ),
        (
            "edit",
            vec!["edit", "doc:1", "--old", "at dawn", "--new", "at dusk"],
            "edit",
        ),
        (
            "append",
            vec!["append", "doc:1", "--content", "One more line."],
            "insert",
        ),
        (
            "insert",
            vec!["insert", "doc:1", "--at-start", "--content", "A motto."],
            "insert",
        ),
        (
            "replace-section",
            vec![
                "replace-section",
                "doc:1",
                "The harbour",
                "--content",
                "Fog.",
            ],
            "replace_section",
        ),
        (
            "set",
            vec!["set", "doc:1", "--content", "# Chapter 1\n\nAll new.\n"],
            "replace_document",
        ),
        (
            "history restore",
            vec!["history", "restore", "{version}"],
            "restore_version",
        ),
        (
            "create",
            vec![
                "create",
                "New",
                "--content",
                "x",
                "--in",
                "Drafts/Deep",
                "--create-folders",
            ],
            "create_document",
        ),
        (
            "duplicate",
            vec!["duplicate", "doc:2"],
            "duplicate_document",
        ),
        ("mkdir", vec!["mkdir", "Shelf/Top"], "create_folder"),
        (
            "rename",
            vec!["rename", "doc:2", "Renamed"],
            "rename_document",
        ),
        (
            "rename",
            vec!["rename", "The Salt Road", "The Road"],
            "rename_folder",
        ),
        ("move", vec!["move", "doc:2", "Shelf"], "move_document"),
        ("move", vec!["move", "Shelf/Top", "/"], "move_folder"),
        ("trash", vec!["trash", "doc:2"], "trash_document"),
        ("trash", vec!["trash", "Top"], "trash_folder"),
        ("restore", vec!["restore", "doc:3"], "restore_document"),
        ("restore", vec!["restore", "Attic"], "restore_folder"),
        (
            "trash",
            vec!["trash", "doc:3", "--permanently"],
            "delete_document",
        ),
        (
            "trash",
            vec!["trash", "Drafts", "--permanently"],
            "delete_folder",
        ),
        (
            "empty-trash",
            vec!["empty-trash", "--permanently"],
            "empty_trash",
        ),
    ];
    let mut ran = BTreeSet::new();
    let mut asked = BTreeSet::new();
    for (command, args, operation) in &lines {
        let version;
        let mut args = args.clone();
        if let Some(place) = args.iter().position(|arg| *arg == "{version}") {
            let versions = sandbox.json(&["history", "doc:1"]);
            let oldest = versions["versions"].as_array().unwrap().last().unwrap()["id"].clone();
            version = oldest.to_string();
            args[place] = &version;
            app.served();
        }
        args.extend(["--author", "Claude"]);
        let result = sandbox.json(&args);
        assert_eq!(result["changed"], true, "{args:?}: {result}");
        assert_eq!(result["dry_run"], false, "{args:?}: {result}");
        // (A command that takes a document or a folder looks first which of the two it
        // is: a read, for which the app is asked to save.)
        let served: Vec<_> = app
            .served()
            .into_iter()
            .filter(|served| served.name != "flush")
            .collect();
        assert_eq!(
            served.len(),
            1,
            "{args:?}: one change for the app: {served:?}"
        );
        assert_eq!(served[0].name, *operation, "{args:?}");
        assert_eq!(served[0].author.as_deref(), Some("Claude"), "{args:?}");
        assert!(
            served[0].done && served[0].changes > 0,
            "{args:?}: {served:?}"
        );
        ran.insert(command.to_string());
        asked.insert(*operation);
    }
    // The app is still the one that has the library.
    assert_eq!(sandbox.json(&["status"])["app_is_open"], true);

    // Every command whose help says that it changes the library was among them, and
    // none of them is a command that says it does not.
    let commands = commands(&sandbox);
    let writing: BTreeSet<String> = commands
        .iter()
        .filter(|(_, writes)| **writes)
        .map(|(name, _)| name.clone())
        .collect();
    assert_eq!(
        ran, writing,
        "the commands this test runs and the commands whose help says that they change \
         the library differ: a command that writes has to be run here, with the app open"
    );
    // And every operation that writes was asked of the app at least once.
    let operations: BTreeSet<&str> = writer_agent::live::Operation::CHANGES
        .iter()
        .copied()
        .collect();
    assert_eq!(asked, operations);

    // What the app wrote is what a read finds; a read asks the app to save first.
    app.served();
    assert_eq!(
        sandbox.ok(&["get", "doc:1"]),
        "# Chapter 1\n\nThe keeper could not swim. The *Maren* left at dawn.\n\n## The harbour\n\nGulls, rope, tar. The keeper watched.\n",
        "the oldest version was restored last"
    );
    let served = app.served();
    assert_eq!(served.len(), 1);
    assert_eq!(served[0].name, "flush");
    assert_eq!(
        sandbox.json(&["info", "doc:1"])["document"]["path"],
        "The Road/Chapter 1"
    );
    let tree = sandbox.ok(&["list"]);
    assert!(tree.contains("Chapter 1  doc:1"), "{tree}");
    assert!(tree.contains("Old box  doc:4"), "{tree}");
    assert!(!tree.contains("Drafts") && !tree.contains("Gone"), "{tree}");
    let status = sandbox.json(&["status"]);
    assert_eq!(status["trashed"], 0, "the trash was emptied");

    // A change that fails in the app fails as it does without the app.
    let missing = sandbox.fail(&["edit", "doc:1", "--old", "the mate", "--new", "x"], 6);
    assert!(
        missing.contains("writer find"),
        "the hint of the command line: {missing}"
    );
    let twice = sandbox.run(&[
        "edit", "doc:1", "--old", "keeper", "--new", "warden", "--json",
    ]);
    assert_eq!(twice.code, 7);
    assert_eq!(twice.error()["code"], "not_unique");
    assert_eq!(twice.error()["occurrences"].as_array().unwrap().len(), 2);
    assert_eq!(app.changes(), ["edit", "edit"]);

    // A dry run is worked out by the tool, and says who would make the change.
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
            "The Writer is open: the real run will be carried out by the app, where the user \
             sees the change at once.\n"
        ),
        "{dry}"
    );
    let dry = sandbox.json(&["trash", "doc:1", "--dry-run"]);
    assert_eq!(dry["app_is_open"], true);
    assert_eq!(dry["app_takes_changes"], true);
    assert_eq!(app.changes(), [] as [&str; 0]);

    // The app quits: the tool writes by itself again.
    drop(app);
    assert_eq!(sandbox.json(&["status"])["app_is_open"], false);
    sandbox.ok(&["append", "doc:1", "--content", "After the app."]);
    assert!(sandbox.ok(&["get", "doc:1"]).ends_with("After the app.\n"));
}

#[test]
fn every_tool_of_the_server_that_changes_the_library_is_carried_out_by_the_app_while_it_is_open() {
    let sandbox = library();
    let mut client = sandbox.mcp();
    client.initialize("2025-11-25", "Test Agent");
    // The server was started before the app: it finds the app when the app is there.
    let app = sandbox.open_the_app();

    let (said, status) = client.call_ok("library_status", json!({}));
    assert_eq!(status["app_is_open"], true);
    assert_eq!(status["app_takes_changes"], true);
    assert!(
        said.contains("a change is carried out by the app"),
        "{said}"
    );
    app.served();

    let calls: Vec<(&str, Value, &str)> = vec![
        (
            "add_comment",
            json!({ "document": 1, "quote": "could not swim", "body": "Hm." }),
            "add_comment",
        ),
        (
            "reply_to_comment",
            json!({ "document": 1, "comment": 1, "body": "Yes." }),
            "reply",
        ),
        (
            "resolve_comment",
            json!({ "document": 1, "comment": 1 }),
            "resolve_comment",
        ),
        (
            "reopen_comment",
            json!({ "document": 1, "comment": 1 }),
            "reopen_comment",
        ),
        (
            "delete_comment",
            json!({ "document": 1, "comment": 1 }),
            "delete_comment",
        ),
        (
            "edit_document",
            json!({ "document": "Chapter 1", "old_text": "at dawn", "new_text": "at dusk" }),
            "edit",
        ),
        (
            "append_to_document",
            json!({ "document": 1, "markdown": "The end." }),
            "insert",
        ),
        (
            "insert_in_document",
            json!({ "document": 1, "markdown": "Before.", "position": "before_heading", "heading": "The harbour" }),
            "insert",
        ),
        (
            "replace_section",
            json!({ "document": 1, "heading": "The harbour", "markdown": "Only fog." }),
            "replace_section",
        ),
        (
            "replace_document",
            json!({ "document": 1, "markdown": "# Chapter 1\n\nAll new.\n" }),
            "replace_document",
        ),
        (
            "restore_version",
            json!({ "version": 1 }),
            "restore_version",
        ),
        (
            "create_document",
            json!({ "title": "New", "folder": "Drafts/Deep", "create_folders": true, "markdown": "x" }),
            "create_document",
        ),
        (
            "duplicate_document",
            json!({ "document": 2 }),
            "duplicate_document",
        ),
        (
            "create_folder",
            json!({ "path": "Shelf/Top" }),
            "create_folder",
        ),
        (
            "rename_document",
            json!({ "document": 2, "title": "Renamed" }),
            "rename_document",
        ),
        (
            "rename_folder",
            json!({ "folder": "The Salt Road", "name": "The Road" }),
            "rename_folder",
        ),
        (
            "move_document",
            json!({ "document": 2, "folder": "Shelf" }),
            "move_document",
        ),
        (
            "move_folder",
            json!({ "folder": "Shelf/Top", "parent": "/" }),
            "move_folder",
        ),
        ("trash_document", json!({ "document": 2 }), "trash_document"),
        ("trash_folder", json!({ "folder": "Top" }), "trash_folder"),
        (
            "restore_document",
            json!({ "document": 3 }),
            "restore_document",
        ),
        (
            "restore_folder",
            json!({ "folder": "Attic" }),
            "restore_folder",
        ),
    ];
    let mut called = BTreeSet::new();
    for (tool, arguments, operation) in &calls {
        let (text, data) = client.call_ok(tool, arguments.clone());
        assert_eq!(data["changed"], true, "{tool}: {text}");
        let served: Vec<_> = app
            .served()
            .into_iter()
            .filter(|served| served.name != "flush")
            .collect();
        assert_eq!(
            served.len(),
            1,
            "{tool}: one change for the app: {served:?}"
        );
        assert_eq!(served[0].name, *operation, "{tool}");
        // Without a name in the arguments, the client's name is who asks.
        assert_eq!(served[0].author.as_deref(), Some("Test Agent"), "{tool}");
        assert!(
            served[0].done && served[0].changes > 0,
            "{tool}: {served:?}"
        );
        called.insert(tool.to_string());
    }

    // Every tool the server lists as one that changes something was among them.
    let listed = client.request("tools/list", json!({}));
    let tools = listed["result"]["tools"].as_array().unwrap().clone();
    assert!(tools.len() >= 30, "{}", tools.len());
    // (`export` writes a file, which is why it is not listed as read-only; of the
    // library it only reads, and is called with the readers below.)
    let reads_only =
        |tool: &Value| tool["annotations"]["readOnlyHint"] == true || tool["name"] == "export";
    let changing: BTreeSet<String> = tools
        .iter()
        .filter(|tool| !reads_only(tool))
        .map(|tool| tool["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        called, changing,
        "the tools this test calls and the tools the server lists as changing something \
         differ: a tool that writes has to be called here, with the app open"
    );
    // The tools that only read ask the app to save, and for nothing else.
    assert_eq!(tools.iter().filter(|tool| reads_only(tool)).count(), 13);
    for tool in tools.iter().filter(|tool| reads_only(tool)) {
        let name = tool["name"].as_str().unwrap();
        let arguments = match name {
            "library_status" | "list_library" | "recent_documents" | "list_trash" => json!({}),
            "search_library" => json!({ "query": "keeper" }),
            "folder_info" => json!({ "folder": "The Road" }),
            "read_version" => json!({ "version": 1 }),
            "find_in_document" => json!({ "document": 1, "text": "keeper" }),
            "export" => json!({
                "document": 1,
                "path": sandbox.root().join("out/chapter.md").to_str().unwrap(),
            }),
            _ => json!({ "document": 1 }),
        };
        client.call_ok(name, arguments);
        let served = app.served();
        assert!(
            !served.is_empty() && served.iter().all(|served| served.name == "flush"),
            "{name}: {served:?}"
        );
    }

    // A failure in the app is the tool's failure, with what it carries.
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
    // A dry run says who would make the change.
    let (dry, dry_data) =
        client.call_ok("trash_document", json!({ "document": 1, "dry_run": true }));
    assert!(
        dry.contains("The Writer is open: the real run will be carried out by the app"),
        "{dry}"
    );
    assert_eq!(dry_data["app_takes_changes"], true);

    // The app quits; the same server, still running, writes by itself.
    app.served();
    drop(app);
    let (appended, _) = client.call_ok(
        "append_to_document",
        json!({ "document": 1, "markdown": "More." }),
    );
    assert!(appended.starts_with("Appended to doc:1"), "{appended}");
    client.close();
}

#[test]
fn with_the_switch_off_every_change_is_refused_and_says_where_the_switch_is() {
    let sandbox = library();
    let app = sandbox.open_the_app();
    app.set_switch(false);
    let before = sandbox.ok(&["get", "doc:1"]);
    let versions_before = sandbox.ok(&["history", "doc:1"]);

    let status = sandbox.json(&["status"]);
    assert_eq!(status["app_is_open"], true);
    assert_eq!(status["app_takes_changes"], false);
    let said = sandbox.ok(&["status"]);
    assert!(
        said.contains(
            "and takes no changes from agents (they are switched off in its Agents panel, or \
             it is a version before 0.6.0): reading works, every change is refused until \
             that is changed or the user quits the app.\n"
        ),
        "{said}"
    );

    let writes: Vec<Vec<&str>> = vec![
        vec!["create", "New", "--content", "x"],
        vec!["edit", "doc:1", "--old", "at dawn", "--new", "at dusk"],
        vec!["append", "doc:1", "--content", "x"],
        vec!["rename", "doc:1", "Renamed"],
        vec!["trash", "doc:1"],
        vec!["mkdir", "New folder"],
        vec![
            "comment",
            "add",
            "doc:1",
            "--quote",
            "keeper watched",
            "--body",
            "x",
        ],
        vec!["empty-trash", "--permanently"],
    ];
    app.served();
    for args in &writes {
        let message = sandbox.fail(args, 5);
        assert!(
            message.starts_with(&format!(
                "writer: The Writer is open (process {}) and changes by agents while it is \
                 open are switched off there, so nothing was changed. Ask the user to switch \
                 on \u{201c}Agents can make changes while The Writer is open\u{201d} in the \
                 Agents panel of the app (The Writer > Agents\u{2026}), or to quit The \
                 Writer; then try again.\n",
                std::process::id()
            )),
            "{args:?}: {message}"
        );
        // The command line adds what goes on working.
        assert!(
            message.ends_with(
                "Reading works while The Writer is open (list, search, get, find, comments, \
                 history, export), and so does every command with --dry-run. \"writer \
                 status\" says whether the app takes changes.\n"
            ),
            "{args:?}: {message}"
        );
    }
    // The switch has one name: the one the message gives is the one the skill tells
    // agents about (and the one the app's Agents panel shows).
    let message = sandbox.fail(&writes[0], 5);
    let name = message
        .split('\u{201c}')
        .nth(1)
        .and_then(|rest| rest.split('\u{201d}').next())
        .expect("the message names the switch");
    assert_eq!(name, "Agents can make changes while The Writer is open");
    let skill = sandbox.ok(&["agents", "skill"]);
    assert!(skill.contains(&format!("\"{name}\"")), "{skill}");

    // The app was asked each time, and refused each time.
    let served: Vec<_> = app
        .served()
        .into_iter()
        .filter(|served| served.name != "flush")
        .collect();
    assert_eq!(served.len(), writes.len() + 1);
    assert!(
        served
            .iter()
            .all(|served| !served.done && served.changes == 0)
    );

    let run = sandbox.run(&["append", "doc:1", "--content", "x", "--json"]);
    assert_eq!(run.code, 5);
    let error = run.error();
    assert_eq!(error["code"], "app_is_open");
    assert_eq!(error["refusal"]["why"], "switched_off");
    assert_eq!(error["nothing_changed"], true);
    assert!(error["hint"].as_str().unwrap().contains("--dry-run"));

    // The server says the same in its own words.
    let mut client = sandbox.mcp();
    client.initialize("2025-11-25", "Test Agent");
    let (refused, error) = client.call_error(
        "append_to_document",
        json!({ "document": 1, "markdown": "More." }),
    );
    assert_eq!(error["code"], "app_is_open");
    assert_eq!(error["refusal"]["why"], "switched_off");
    assert!(
        refused.contains("changes by agents while it is open are switched off there"),
        "{refused}"
    );
    assert!(
        refused.ends_with(
            "Everything that only reads (list_library, search_library, read_document, \
             list_comments, export) works while the app is open, and so does any tool with \
             dry_run."
        ),
        "{refused}"
    );
    let (said, status) = client.call_ok("library_status", json!({}));
    assert_eq!(status["app_takes_changes"], false);
    assert!(said.contains("takes no changes from agents"), "{said}");
    // A dry run works, and says what the real run would meet.
    let (dry, dry_data) =
        client.call_ok("trash_document", json!({ "document": 1, "dry_run": true }));
    assert!(
        dry.contains("The Writer is open and takes no changes from agents"),
        "{dry}"
    );
    assert_eq!(dry_data["app_is_open"], true);
    assert_eq!(dry_data["app_takes_changes"], false);
    client.close();
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
        dry.ends_with(
            "the real run will be refused until that is changed or the user quits the app.\n"
        ),
        "{dry}"
    );

    // Nothing of all that changed the library, and reading went on.
    assert_eq!(sandbox.ok(&["get", "doc:1"]), before);
    assert_eq!(sandbox.ok(&["history", "doc:1"]), versions_before);
    assert!(sandbox.ok(&["search", "keeper"]).contains("doc:1"));

    // The owner switches it on again: the same command line is carried out.
    app.set_switch(true);
    assert_eq!(sandbox.json(&["status"])["app_takes_changes"], true);
    app.served();
    sandbox.ok(&["edit", "doc:1", "--old", "at dawn", "--new", "at dusk"]);
    assert_eq!(app.changes(), ["edit"]);
    assert!(sandbox.ok(&["get", "doc:1"]).contains("left at dusk"));
}

#[test]
fn several_tools_at_once_are_served_one_after_the_other_while_the_app_is_open() {
    let sandbox = library();
    let app = sandbox.open_the_app();
    // Ten processes at the same moment, and a server besides.
    let mut client = sandbox.mcp();
    client.initialize("2025-11-25", "Test Agent");
    let children: Vec<_> = (0..10)
        .map(|number| {
            sandbox
                .command()
                .args([
                    "append",
                    "doc:1",
                    "--content",
                    &format!("Line {number}."),
                    "--author",
                    &format!("Agent {number}"),
                ])
                .spawn()
                .expect("the binary starts")
        })
        .collect();
    client.call_ok(
        "append_to_document",
        json!({ "document": 1, "markdown": "Line of the server." }),
    );
    for child in children {
        let output = child.wait_with_output().expect("the binary ran");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).starts_with("Appended to doc:1"));
    }
    client.close();
    // None was lost, and each was a request of its own.
    let text = sandbox.ok(&["get", "doc:1"]);
    for number in 0..10 {
        assert_eq!(
            text.matches(&format!("Line {number}.")).count(),
            1,
            "{text}"
        );
    }
    assert_eq!(text.matches("Line of the server.").count(), 1, "{text}");
    let served = app.served();
    let appends: Vec<_> = served
        .iter()
        .filter(|served| served.name == "insert")
        .collect();
    assert_eq!(appends.len(), 11);
    assert!(
        appends
            .iter()
            .all(|served| served.done && served.changes > 0)
    );
    let authors: BTreeSet<&str> = appends
        .iter()
        .filter_map(|served| served.author.as_deref())
        .collect();
    assert_eq!(authors.len(), 11, "{authors:?}");
    let versions = sandbox.json(&["history", "doc:1"]);
    assert_eq!(versions["versions"].as_array().unwrap().len(), 11);
}
