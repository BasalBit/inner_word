//! Looking around, reading and writing text: every command, in text and in JSON.

use super::harness::Sandbox;

const CHAPTER: &str = "# Chapter 1\n\nThe keeper could not swim. The *Maren* left at dawn.\n\n## The harbour\n\nGulls, rope, tar. The keeper watched.\n";

/// A small library: a book with a chapter, a note, a loose document.
fn library() -> Sandbox {
    let sandbox = Sandbox::new();
    assert_eq!(
        sandbox.create("Chapter 1", Some("The Salt Road/Part one"), CHAPTER),
        "doc:1"
    );
    assert_eq!(
        sandbox.create("Ideas", None, "A lighthouse keeper who cannot swim."),
        "doc:2"
    );
    assert_eq!(
        sandbox.create(
            "Reading list",
            Some("Notes"),
            "- The Rings of Saturn\n- Austerlitz\n"
        ),
        "doc:3"
    );
    sandbox
}

#[test]
fn reading_a_library_that_is_not_there_says_so_and_makes_none() {
    let sandbox = Sandbox::new();
    for args in [
        &["list"][..],
        &["search", "x"],
        &["recent"],
        &["get", "doc:1"],
        &["list", "--trash"],
    ] {
        let message = sandbox.fail(args, 10);
        assert!(message.contains("there is no library at"), "{message}");
        assert!(message.contains("--library <dir>"), "{message}");
    }
    assert!(
        !sandbox.library().join("library.db").exists(),
        "reading never creates a library"
    );
    let status = sandbox.json(&["status"]);
    assert_eq!(status["exists"], false);
    assert_eq!(status["documents"], 0);
    let text = sandbox.ok(&["status"]);
    assert!(text.contains("There is no library there yet"), "{text}");
}

#[test]
fn status_list_recent_and_info() {
    let sandbox = library();
    let status = sandbox.json(&["status"]);
    assert_eq!(status["exists"], true);
    assert_eq!(status["documents"], 3);
    assert_eq!(status["folders"], 3);
    assert_eq!(status["trashed"], 0);
    assert_eq!(status["app_is_open"], false);
    let text = sandbox.ok(&["status"]);
    assert!(
        text.contains("3 documents, 3 folders, 0 items in the trash"),
        "{text}"
    );
    assert!(
        text.contains("The Writer is not running: reading and writing both work."),
        "{text}"
    );

    let tree = sandbox.ok(&["list"]);
    assert_eq!(sandbox.ok(&["ls"]), tree);
    assert_eq!(sandbox.ok(&["tree"]), tree);
    let lines: Vec<&str> = tree.lines().collect();
    assert_eq!(lines[0], "Library");
    assert_eq!(lines[1], "  The Salt Road/  folder:1");
    assert_eq!(lines[2], "    Part one/  folder:2");
    assert!(
        lines[3].starts_with("      Chapter 1  doc:1  (20 words, changed 20"),
        "{tree}"
    );
    assert_eq!(lines[4], "  Notes/  folder:3");
    assert!(lines[5].starts_with("    Reading list  doc:3  ("), "{tree}");
    assert!(lines[6].starts_with("  Ideas  doc:2  (6 words"), "{tree}");
    assert_eq!(lines[7], "3 folders, 3 documents, 31 words");

    let json = sandbox.json(&["list"]);
    assert_eq!(json["total_documents"], 3);
    assert_eq!(json["folders"][0]["name"], "The Salt Road");
    assert_eq!(json["folders"][0]["folders"][0]["documents"][0]["id"], 1);
    assert_eq!(
        json["folders"][0]["folders"][0]["documents"][0]["path"],
        "The Salt Road/Part one/Chapter 1"
    );
    assert_eq!(json["documents"][0]["title"], "Ideas");

    let part = sandbox.ok(&["list", "The Salt Road/Part one"]);
    assert!(
        part.starts_with("The Salt Road/Part one/  folder:2\n  Chapter 1  doc:1"),
        "{part}"
    );
    assert_eq!(sandbox.ok(&["list", "folder:2"]), part);
    let shallow = sandbox.ok(&["list", "--depth", "0"]);
    assert!(
        shallow.contains("The Salt Road/  folder:1  (1 folder, 1 document inside)"),
        "{shallow}"
    );
    assert!(!shallow.contains("Chapter 1"), "{shallow}");
    let folders = sandbox.ok(&["list", "--folders"]);
    assert!(
        folders.contains("Part one/") && !folders.contains("doc:"),
        "{folders}"
    );
    let missing = sandbox.fail(&["list", "No such folder"], 3);
    assert!(missing.contains("No such folder"), "{missing}");

    let recent = sandbox.ok(&["recent", "--limit", "2"]);
    let lines: Vec<&str> = recent.lines().collect();
    assert_eq!(lines.len(), 2, "{recent}");
    assert!(
        lines[0].starts_with("doc:3  Notes/Reading list  ("),
        "{recent}"
    );
    assert!(lines[1].starts_with("doc:2  Ideas  ("), "{recent}");
    assert_eq!(
        sandbox.json(&["recent"])["documents"]
            .as_array()
            .unwrap()
            .len(),
        3
    );

    let info = sandbox.ok(&["info", "Chapter 1"]);
    assert!(
        info.starts_with("doc:1  The Salt Road/Part one/Chapter 1\n"),
        "{info}"
    );
    assert!(
        info.contains("title:     Chapter 1 (follows the text)"),
        "{info}"
    );
    assert!(
        info.contains("folder:    The Salt Road/Part one  folder:2"),
        "{info}"
    );
    assert!(info.contains("size:      20 words, "), "{info}");
    assert!(info.contains("comments:  0 open, 0 resolved"), "{info}");
    assert!(info.contains("page:      A4 portrait"), "{info}");
    assert!(
        info.contains("headings:\n  # Chapter 1  (20 words)\n  ## The harbour  (8 words)\n"),
        "{info}"
    );
    let json = sandbox.json(&["info", "doc:1"]);
    assert_eq!(json["document"]["words"], 20);
    assert_eq!(json["headings"][1]["text"], "The harbour");
    assert_eq!(json["headings"][1]["level"], 2);
    assert_eq!(json["settings"]["paper"], "A4");

    let folder = sandbox.ok(&["info", "The Salt Road"]);
    assert!(folder.starts_with("folder:1  The Salt Road\n"), "{folder}");
    assert!(
        folder.contains("holds:     1 folder, 1 document, 20 words"),
        "{folder}"
    );
    assert!(
        folder.contains("directly in it:\n  Part one/  folder:2"),
        "{folder}"
    );
    assert_eq!(
        sandbox.json(&["info", "folder:1"])["folder"]["name"],
        "The Salt Road"
    );
    let top = sandbox.ok(&["info", "/"]);
    assert!(
        top.starts_with(
            "The top level of the library\nholds:     3 folders, 3 documents, 31 words"
        ),
        "{top}"
    );
}

#[test]
fn search_finds_documents_with_snippets() {
    let sandbox = library();
    let hits = sandbox.ok(&["search", "keeper"]);
    assert!(
        hits.starts_with("2 documents match \"keeper\":\n"),
        "{hits}"
    );
    assert!(hits.contains("doc:2  Ideas  ("), "{hits}");
    assert!(
        hits.contains("    A lighthouse **keeper** who cannot swim"),
        "{hits}"
    );
    assert!(
        hits.contains("doc:1  The Salt Road/Part one/Chapter 1  ("),
        "{hits}"
    );
    let json = sandbox.json(&["search", "keeper", "--limit", "1"]);
    assert_eq!(json["query"], "keeper");
    assert_eq!(json["hits"].as_array().unwrap().len(), 1);
    assert!(json["hits"][0]["document"]["id"].is_number());
    assert!(
        json["hits"][0]["snippet"]
            .as_str()
            .unwrap()
            .contains("keeper")
    );
    let in_folder = sandbox.json(&["search", "keeper", "--in", "The Salt Road"]);
    assert_eq!(in_folder["hits"].as_array().unwrap().len(), 1);
    assert_eq!(in_folder["hits"][0]["document"]["id"], 1);
    let phrase = sandbox.json(&["grep", "\"rings of saturn\""]);
    assert_eq!(phrase["hits"][0]["document"]["title"], "Reading list");
    assert_eq!(
        sandbox.ok(&["search", "zeppelin"]),
        "No document matches \"zeppelin\".\n"
    );
}

#[test]
fn get_prints_the_text_and_nothing_else() {
    let sandbox = library();
    assert_eq!(
        sandbox.ok(&["get", "doc:1"]),
        CHAPTER,
        "Markdown in, the same Markdown out"
    );
    for alias in ["cat", "read", "show"] {
        assert_eq!(
            sandbox.ok(&[alias, "The Salt Road/Part one/Chapter 1"]),
            CHAPTER
        );
    }
    assert_eq!(sandbox.ok(&["get", "1"]), CHAPTER);
    assert_eq!(
        sandbox.ok(&["get", "Part one/Chapter 1", "--format", "markdown"]),
        CHAPTER
    );
    assert_eq!(
        sandbox.ok(&["get", "doc:1", "--format", "text"]),
        "Chapter 1\n\nThe keeper could not swim. The Maren left at dawn.\n\nThe harbour\n\nGulls, rope, tar. The keeper watched.\n"
    );
    assert_eq!(
        sandbox.ok(&["get", "doc:1", "--section", "The harbour"]),
        "## The harbour\n\nGulls, rope, tar. The keeper watched.\n"
    );
    let native: serde_json::Value =
        serde_json::from_str(&sandbox.ok(&["get", "doc:1", "--format", "json"]))
            .expect("the native form is JSON");
    assert!(
        native["blocks"]
            .as_array()
            .is_some_and(|blocks| blocks.len() == 4),
        "{native}"
    );

    let json = sandbox.json(&["get", "doc:1"]);
    assert_eq!(json["content"], CHAPTER);
    assert_eq!(json["format"], "markdown");
    assert_eq!(json["document"]["id"], 1);
    assert_eq!(json["document"]["path"], "The Salt Road/Part one/Chapter 1");
    let section = sandbox.json(&[
        "get",
        "doc:1",
        "--section",
        "The harbour",
        "--format",
        "text",
    ]);
    assert_eq!(section["format"], "plain_text");
    assert_eq!(section["section"]["text"], "The harbour");

    let missing = sandbox.fail(&["get", "doc:99"], 3);
    assert!(missing.contains("there is no document"), "{missing}");
    let heading = sandbox.fail(&["get", "doc:1", "--section", "The cliffs"], 3);
    assert!(
        heading.contains("The document's headings: # Chapter 1; ## The harbour"),
        "{heading}"
    );
    let folder = sandbox.fail(&["get", "folder:1"], 2);
    assert!(
        folder.contains("names a folder; a document is needed here"),
        "{folder}"
    );
}

#[test]
fn create_takes_its_text_from_an_argument_a_file_or_standard_input() {
    let sandbox = Sandbox::new();
    // An argument.
    let created = sandbox.ok(&[
        "create",
        "Ideas",
        "--content",
        "A lighthouse keeper who cannot swim.",
    ]);
    assert_eq!(created, "Created doc:1 \"Ideas\" (6 words).\n");
    // A file, in a folder that is made on the way.
    std::fs::write(sandbox.work().join("chapter.md"), CHAPTER).unwrap();
    let created = sandbox.ok(&[
        "create",
        "Chapter 1",
        "--in",
        "The Salt Road/Part one",
        "--create-folders",
        "--file",
        "chapter.md",
    ]);
    assert_eq!(
        created,
        "Created doc:2 \"The Salt Road/Part one/Chapter 1\" (20 words).\n\
Folder created: folder:1 \"The Salt Road\"\nFolder created: folder:2 \"The Salt Road/Part one\"\n"
    );
    assert_eq!(sandbox.ok(&["get", "doc:2"]), CHAPTER);
    // Standard input, with the title taken from the text.
    let piped = sandbox.run_with_input(
        &["create", "--in", "The Salt Road", "--stdin", "--json"],
        "# Notes on tides\n\nSpring and neap.\n",
    );
    assert_eq!(piped.code, 0, "{}", piped.stderr);
    let created: serde_json::Value = serde_json::from_str(&piped.stdout).unwrap();
    assert_eq!(created["document"]["title"], "Notes on tides");
    assert_eq!(created["document"]["path"], "The Salt Road/Notes on tides");
    assert_eq!(created["document"]["id"], 3);
    assert_eq!(created["changed"], true);
    assert_eq!(created["dry_run"], false);
    let dash = sandbox.run_with_input(&["new", "From a dash", "--file", "-"], "Text.\n");
    assert_eq!(dash.stdout, "Created doc:4 \"From a dash\" (1 word).\n");
    // Without a source of text: an empty document, and standard input is not read.
    let empty = sandbox.run_with_input(&["create", "Empty"], "this is not read");
    assert_eq!(empty.stdout, "Created doc:5 \"Empty\" (0 words).\n");
    assert_eq!(sandbox.ok(&["get", "doc:5"]).trim(), "");

    // A place among the others.
    sandbox.ok(&[
        "create",
        "Prologue",
        "--in",
        "The Salt Road/Part one",
        "--first",
        "--content",
        "Before.",
    ]);
    sandbox.ok(&[
        "create",
        "Chapter 2",
        "--in",
        "folder:2",
        "--after",
        "Chapter 1",
        "--content",
        "After.",
    ]);
    let part = sandbox.json(&["list", "folder:2"]);
    let titles: Vec<&str> = part["documents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles, vec!["Prologue", "Chapter 1", "Chapter 2"]);

    // A dry run creates nothing.
    let dry = sandbox.ok(&["create", "Ghost", "--content", "Boo.", "--dry-run"]);
    assert_eq!(
        dry,
        "Dry run: nothing was changed. The real run would report:\nCreated a new document \"Ghost\" (1 word).\n"
    );
    assert!(
        sandbox
            .fail(&["get", "Ghost"], 3)
            .contains("there is no document")
    );
    // A folder that does not exist is not made by accident.
    let missing = sandbox.fail(
        &["create", "Lost", "--in", "No such folder", "--content", "x"],
        3,
    );
    assert!(missing.contains("No such folder"), "{missing}");
    let unreadable = sandbox.fail(&["create", "Lost", "--file", "no-such-file.md"], 1);
    assert!(
        unreadable.contains("no-such-file.md cannot be read"),
        "{unreadable}"
    );
}

#[test]
fn edit_replaces_an_exact_piece_of_text_and_keeps_the_rest() {
    let sandbox = library();
    let edited = sandbox.ok(&[
        "edit",
        "doc:1",
        "--old",
        "at dawn",
        "--new",
        "before **dawn**",
        "--author",
        "Claude",
    ]);
    assert!(
        edited.starts_with("Edited doc:1 \"The Salt Road/Part one/Chapter 1\": 1 replacement; blocks: 1 changed; 20 -> 20 words.\n"),
        "{edited}"
    );
    assert!(edited.contains("kept as version 1 (Before an edit by Claude); \"writer history restore 1\" brings it back."), "{edited}");
    assert_eq!(
        sandbox.ok(&["get", "doc:1"]),
        CHAPTER.replace("at dawn", "before **dawn**"),
        "the italics around Maren are still there, the new word is bold"
    );

    // Not there: exit code 6, nothing changes.
    let none = sandbox.fail(&["edit", "doc:1", "--old", "the mate", "--new", "x"], 6);
    assert!(
        none.contains("the text was not found in the document"),
        "{none}"
    );
    // Several times: exit code 7, with the places.
    let many = sandbox.fail(&["edit", "doc:1", "--old", "keeper", "--new", "warden"], 7);
    assert!(many.contains("the text occurs 2 times"), "{many}");
    assert!(
        many.contains("  1  [Chapter 1] The keeper could not swim."),
        "{many}"
    );
    assert!(
        many.contains("  2  [The harbour] Gulls, rope, tar. The keeper watched."),
        "{many}"
    );
    let run = sandbox.run(&[
        "edit", "doc:1", "--old", "keeper", "--new", "warden", "--json",
    ]);
    assert_eq!(run.code, 7);
    let error = run.error();
    assert_eq!(error["code"], "not_unique");
    assert_eq!(error["occurrences"].as_array().unwrap().len(), 2);
    assert_eq!(error["occurrences"][1]["heading"], "The harbour");

    // `find` shows the same places without changing anything.
    let found = sandbox.ok(&["find", "doc:1", "keeper"]);
    assert!(found.starts_with("The text occurs 2 times in doc:1 \"The Salt Road/Part one/Chapter 1\".\n  1  [Chapter 1]"), "{found}");
    assert_eq!(sandbox.json(&["find", "doc:1", "keeper"])["count"], 2);
    assert!(
        sandbox
            .ok(&["find", "doc:1", "zeppelin"])
            .starts_with("The text does not occur in doc:1")
    );

    // One of them, then all of them; a dry run first.
    let dry = sandbox.ok(&[
        "edit",
        "doc:1",
        "--old",
        "keeper",
        "--new",
        "warden",
        "--all",
        "--dry-run",
    ]);
    assert!(
        dry.starts_with("Dry run: nothing was changed. The real run would report:\nEdited doc:1"),
        "{dry}"
    );
    assert!(dry.contains("2 replacements"), "{dry}");
    assert!(
        sandbox
            .ok(&["get", "doc:1"])
            .contains("The keeper could not swim"),
        "a dry run changes nothing"
    );
    let second = sandbox.json(&[
        "edit",
        "doc:1",
        "--old",
        "keeper",
        "--new",
        "warden",
        "--occurrence",
        "2",
    ]);
    assert_eq!(second["replacements"], 1);
    assert_eq!(second["changed"], true);
    assert!(second["snapshot"]["id"].is_number());
    let text = sandbox.ok(&["get", "doc:1"]);
    assert!(
        text.contains("The keeper could not swim") && text.contains("The warden watched"),
        "{text}"
    );
    sandbox.ok(&["edit", "doc:1", "--old", "The", "--new", "A", "--all"]);
    assert!(
        sandbox
            .ok(&["get", "doc:1", "--format", "text"])
            .contains("A keeper could not swim. A Maren left")
    );

    // The old and the new text from files and from standard input; an empty new text deletes.
    std::fs::write(sandbox.work().join("old.txt"), "Gulls, rope, tar. ").unwrap();
    let from_files = sandbox.run_with_input(
        &["edit", "doc:1", "--old-file", "old.txt", "--new-file", "-"],
        "Fog. ",
    );
    assert_eq!(from_files.code, 0, "{}", from_files.stderr);
    assert!(
        sandbox
            .ok(&["get", "doc:1"])
            .contains("Fog. A warden watched.")
    );
    sandbox.ok(&["edit", "doc:1", "--old", "Fog. ", "--new", ""]);
    assert!(
        sandbox
            .ok(&["get", "doc:1"])
            .contains("\n\nA warden watched.\n")
    );
    // Literal: no Markdown is read in the new text.
    sandbox.ok(&[
        "edit",
        "doc:2",
        "--old",
        "lighthouse",
        "--new",
        "*light*house",
        "--literal",
    ]);
    assert!(
        sandbox
            .ok(&["get", "doc:2", "--format", "text"])
            .contains("A *light*house keeper")
    );
}

#[test]
fn set_append_insert_and_replace_section() {
    let sandbox = library();
    // append: from an argument and from a pipe.
    let appended = sandbox.ok(&["append", "doc:1", "--content", "The end."]);
    assert!(appended.starts_with("Appended to doc:1 \"The Salt Road/Part one/Chapter 1\": blocks: 1 added; 20 -> 22 words.\n"), "{appended}");
    assert!(
        appended.contains("kept as version 1 (Before an insertion by an agent)"),
        "{appended}"
    );
    let piped = sandbox.run_with_input(&["append", "doc:1"], "Piped in.\n");
    assert_eq!(piped.code, 0, "{}", piped.stderr);
    assert!(
        sandbox
            .ok(&["get", "doc:1"])
            .ends_with("The keeper watched.\n\nThe end.\n\nPiped in.\n")
    );
    // Nothing piped and nothing given: a usage error, not an empty paragraph.
    let nothing = sandbox.fail(&["append", "doc:1"], 2);
    assert!(nothing.contains("no text was given"), "{nothing}");

    // insert: every place.
    sandbox.ok(&["insert", "doc:1", "--at-start", "--content", "*For M.*"]);
    sandbox.ok(&[
        "insert",
        "doc:1",
        "--before",
        "The harbour",
        "--content",
        "Before the harbour.",
    ]);
    sandbox.ok(&[
        "insert",
        "doc:1",
        "--after",
        "The harbour",
        "--content",
        "Right under the heading.",
    ]);
    sandbox.ok(&[
        "insert",
        "doc:1",
        "--end-of",
        "## The harbour",
        "--content",
        "Last of the section.",
    ]);
    assert_eq!(
        sandbox.ok(&["get", "doc:1"]),
        "*For M.*\n\n# Chapter 1\n\nThe keeper could not swim. The *Maren* left at dawn.\n\nBefore the harbour.\n\n\
## The harbour\n\nRight under the heading.\n\nGulls, rope, tar. The keeper watched.\n\nThe end.\n\nPiped in.\n\nLast of the section.\n"
    );
    let no_heading = sandbox.fail(
        &["insert", "doc:1", "--after", "The cliffs", "--content", "x"],
        3,
    );
    assert!(no_heading.contains("there is no heading"), "{no_heading}");

    // replace-section: what is under the heading goes, the heading stays.
    let replaced = sandbox.ok(&[
        "replace-section",
        "doc:1",
        "The harbour",
        "--content",
        "Only fog.",
    ]);
    assert!(
        replaced.starts_with("Replaced a section of doc:1"),
        "{replaced}"
    );
    assert!(
        sandbox
            .ok(&["get", "doc:1"])
            .ends_with("Before the harbour.\n\n## The harbour\n\nOnly fog.\n")
    );
    sandbox.ok(&[
        "replace-section",
        "doc:1",
        "The harbour",
        "--with-heading",
        "--content",
        "## The quay\n\nStone.",
    ]);
    assert!(
        sandbox
            .ok(&["get", "doc:1"])
            .ends_with("Before the harbour.\n\n## The quay\n\nStone.\n")
    );

    // set: the whole text, from a pipe, as `get | ... | set` does it.
    let text = sandbox.ok(&["get", "doc:2"]);
    let set = sandbox.run_with_input(
        &["set", "doc:2", "--json"],
        &text.replace("lighthouse", "lightship"),
    );
    assert_eq!(set.code, 0, "{}", set.stderr);
    let changed: serde_json::Value = serde_json::from_str(&set.stdout).unwrap();
    assert_eq!(changed["changed"], true);
    assert_eq!(changed["document"]["id"], 2);
    assert_eq!(
        sandbox.ok(&["get", "doc:2"]),
        "A lightship keeper who cannot swim.\n"
    );
    // The same text again changes nothing and says so.
    let same = sandbox.ok(&[
        "write",
        "doc:2",
        "--content",
        "A lightship keeper who cannot swim.",
    ]);
    assert_eq!(
        same,
        "Nothing changed: doc:2 \"Ideas\" already reads that way.\n"
    );
    let dry = sandbox.ok(&["set", "doc:2", "--content", "Gone.", "--dry-run"]);
    assert!(dry.starts_with("Dry run: nothing was changed."), "{dry}");
    assert_eq!(
        sandbox.ok(&["get", "doc:2"]),
        "A lightship keeper who cannot swim.\n"
    );
}
