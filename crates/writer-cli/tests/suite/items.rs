//! Documents and folders as items of the library: names, places, the trash, and what a
//! name means when several items have it.

use super::harness::Sandbox;

fn library() -> Sandbox {
    let sandbox = Sandbox::new();
    assert_eq!(
        sandbox.create(
            "Chapter 1",
            Some("The Salt Road/Part one"),
            "# Chapter 1\n\nThe keeper could not swim.\n"
        ),
        "doc:1"
    );
    assert_eq!(
        sandbox.create("Ideas", None, "A lighthouse keeper who cannot swim."),
        "doc:2"
    );
    assert_eq!(
        sandbox.create("Reading list", Some("Notes"), "- The Rings of Saturn\n"),
        "doc:3"
    );
    sandbox
}

fn titles(sandbox: &Sandbox, folder: &str) -> Vec<String> {
    let listing = sandbox.json(&["list", folder]);
    listing["documents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["title"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn mkdir_makes_a_path_once() {
    let sandbox = library();
    assert_eq!(
        sandbox.ok(&["mkdir", "The Salt Road/Part two"]),
        "Created folder:4 \"The Salt Road/Part two\".\n"
    );
    assert_eq!(
        sandbox.ok(&["mkdir", "The Salt Road/Part two"]),
        "The folder is there already: folder:4 \"The Salt Road/Part two\".\n"
    );
    let dry = sandbox.ok(&["mkdir", "Drafts/Old", "--dry-run"]);
    assert_eq!(
        dry,
        "Dry run: nothing was changed. The real run would report:\nCreated a new folder \"Drafts/Old\".\n\
Folder created on the way: a new folder \"Drafts\"\n"
    );
    assert!(sandbox.fail(&["list", "Drafts"], 3).contains("Drafts"));
    let nested = sandbox.json(&["mkdir", "Scenes/Storm", "--in", "The Salt Road/Part two"]);
    assert_eq!(
        nested["folder"]["path"],
        "The Salt Road/Part two/Scenes/Storm"
    );
    assert_eq!(nested["created"].as_array().unwrap().len(), 2);
    assert_eq!(nested["changed"], true);
}

#[test]
fn rename_move_and_duplicate() {
    let sandbox = library();
    // rename: a document, a folder, and the title given back to the text.
    assert_eq!(
        sandbox.ok(&["rename", "doc:2", "Loose ideas"]),
        "Renamed doc:2 \"Ideas\" -> \"Loose ideas\".\n"
    );
    assert_eq!(
        sandbox.ok(&["rename", "Notes", "Notebook"]),
        "Renamed folder:3 \"Notes\" -> \"Notebook\".\n"
    );
    assert_eq!(
        sandbox.ok(&["get", "Notebook/Reading list"]),
        "- The Rings of Saturn\n"
    );
    assert_eq!(
        sandbox.ok(&["rename", "doc:2", "Loose ideas"]),
        "Nothing changed: doc:2 \"Loose ideas\" is already so.\n"
    );
    let renamed = sandbox.json(&["rename", "doc:1", "Chapter One"]);
    assert_eq!(renamed["document"]["title"], "Chapter One");
    assert_eq!(renamed["before"]["title"], "Chapter 1");
    assert_eq!(
        sandbox.ok(&["rename", "doc:1", "--follow-text"]),
        "Renamed doc:1 \"The Salt Road/Part one/Chapter One\" -> \"The Salt Road/Part one/Chapter 1\".\n"
    );
    assert!(
        sandbox
            .fail(&["rename", "Notebook", "--follow-text"], 2)
            .contains("a folder has no text to follow")
    );
    let dry = sandbox.ok(&["rename", "doc:2", "Ideas", "--dry-run"]);
    assert_eq!(
        dry,
        "Dry run: nothing was changed. The real run would report:\nRenamed doc:2 \"Loose ideas\" -> \"Ideas\".\n"
    );
    assert_eq!(
        sandbox.json(&["info", "doc:2"])["document"]["title"],
        "Loose ideas"
    );

    // move: into a folder, to the top, a folder into a folder, and among its siblings.
    assert_eq!(
        sandbox.ok(&["move", "doc:2", "The Salt Road/Part one"]),
        "Moved doc:2 \"Loose ideas\" -> \"The Salt Road/Part one/Loose ideas\".\n"
    );
    assert_eq!(
        titles(&sandbox, "folder:2"),
        vec!["Chapter 1", "Loose ideas"]
    );
    let first = sandbox.ok(&["mv", "doc:2", "--first"]);
    assert!(
        first.starts_with("Moved doc:2 \"The Salt Road/Part one/Loose ideas\" to another place"),
        "{first}"
    );
    assert_eq!(
        titles(&sandbox, "folder:2"),
        vec!["Loose ideas", "Chapter 1"]
    );
    sandbox.ok(&["move", "doc:2", "--after", "doc:1"]);
    assert_eq!(
        titles(&sandbox, "folder:2"),
        vec!["Chapter 1", "Loose ideas"]
    );
    assert_eq!(
        sandbox.ok(&["move", "doc:2", "/"]),
        "Moved doc:2 \"The Salt Road/Part one/Loose ideas\" -> \"Loose ideas\".\n"
    );
    assert_eq!(
        sandbox.ok(&["move", "Notebook", "The Salt Road"]),
        "Moved folder:3 \"Notebook\" -> \"The Salt Road/Notebook\", with 0 folders and 1 document in it.\n"
    );
    assert_eq!(
        sandbox.json(&["info", "doc:3"])["document"]["path"],
        "The Salt Road/Notebook/Reading list"
    );
    let made = sandbox.ok(&["move", "doc:2", "Drafts/2026", "--create-folders"]);
    assert!(
        made.starts_with(
            "Moved doc:2 \"Loose ideas\" -> \"Drafts/2026/Loose ideas\".\nFolder created: folder:"
        ),
        "{made}"
    );
    assert!(
        sandbox
            .fail(&["move", "doc:2", "No such folder"], 3)
            .contains("No such folder")
    );
    assert!(
        sandbox
            .fail(&["move", "folder:1", "The Salt Road/Part one"], 2)
            .len()
            > 10,
        "a folder cannot go into itself"
    );

    // duplicate: beside the original, or somewhere else under another title.
    assert_eq!(
        sandbox.ok(&["duplicate", "doc:1"]),
        "Copied to doc:4 \"The Salt Road/Part one/Chapter 1 copy\" from doc:1 \"The Salt Road/Part one/Chapter 1\" (7 words).\n"
    );
    assert_eq!(
        titles(&sandbox, "folder:2"),
        vec!["Chapter 1", "Chapter 1 copy"]
    );
    let copy = sandbox.json(&[
        "cp",
        "doc:1",
        "--title",
        "Chapter 1 (second draft)",
        "--in",
        "Drafts",
    ]);
    assert_eq!(copy["document"]["path"], "Drafts/Chapter 1 (second draft)");
    assert_eq!(copy["before"]["id"], 1);
    assert_eq!(
        sandbox.ok(&["get", &format!("doc:{}", copy["document"]["id"])]),
        sandbox.ok(&["get", "doc:1"])
    );
}

#[test]
fn the_trash_keeps_what_is_deleted_until_it_is_restored_or_erased() {
    let sandbox = library();
    let dry = sandbox.ok(&["trash", "doc:2", "--dry-run"]);
    assert_eq!(
        dry,
        "Dry run: nothing was changed. The real run would report:\nMoved to the trash: doc:2 \"Ideas\".\n"
    );
    assert_eq!(sandbox.ok(&["list", "--trash"]), "The trash is empty.\n");

    assert_eq!(
        sandbox.ok(&["trash", "doc:2"]),
        "Moved to the trash: doc:2 \"Ideas\".\n"
    );
    assert_eq!(
        sandbox.ok(&["delete", "The Salt Road"]),
        "Moved to the trash: folder:1 \"The Salt Road\", with 1 folder and 1 document in it.\n"
    );
    assert!(
        sandbox
            .fail(&["get", "Ideas"], 3)
            .contains("there is no document")
    );
    let listing = sandbox.ok(&["list"]);
    assert!(
        !listing.contains("Ideas") && !listing.contains("The Salt Road"),
        "{listing}"
    );
    // By id a trashed document can still be read.
    assert_eq!(
        sandbox.ok(&["get", "doc:2"]),
        "A lighthouse keeper who cannot swim.\n"
    );
    // And it cannot be changed while it is there.
    assert!(
        sandbox
            .fail(&["append", "doc:2", "--content", "x"], 9)
            .contains("is in the trash; restore it first")
    );

    let trash = sandbox.ok(&["list", "--trash"]);
    let lines: Vec<&str> = trash.lines().collect();
    assert_eq!(lines[0], "2 items in the trash:");
    assert!(
        lines[1].starts_with("folder:1  The Salt Road/  (1 folder, 1 document, trashed 20"),
        "{trash}"
    );
    assert!(
        lines[2].starts_with("doc:2  Ideas  (6 words, trashed 20"),
        "{trash}"
    );
    let json = sandbox.json(&["list", "--trash"]);
    assert_eq!(json["entries"][0]["kind"], "folder");
    assert_eq!(json["entries"][1]["kind"], "document");
    assert_eq!(json["entries"][1]["document"]["id"], 2);
    assert_eq!(sandbox.json(&["status"])["trashed"], 2);

    // A document that went with its folder comes back with its folder.
    let inside = sandbox.fail(&["restore", "doc:1"], 9);
    assert!(
        inside.contains("went to the trash together with the folder"),
        "{inside}"
    );
    assert_eq!(
        sandbox.ok(&["restore", "The Salt Road"]),
        "Restored from the trash: folder:1 \"The Salt Road\", with 1 folder and 1 document in it.\n"
    );
    assert_eq!(
        sandbox.ok(&["untrash", "doc:2"]),
        "Restored from the trash: doc:2 \"Ideas\".\n"
    );
    assert_eq!(
        sandbox.ok(&["get", "The Salt Road/Part one/Chapter 1"]),
        "# Chapter 1\n\nThe keeper could not swim.\n"
    );
    assert!(
        sandbox
            .fail(&["restore", "doc:2"], 9)
            .contains("is not in the trash")
    );
    assert!(sandbox.fail(&["restore", "No such thing"], 3).len() > 10);
}

#[test]
fn erasing_for_good_needs_the_flag() {
    let sandbox = library();
    sandbox.ok(&["trash", "doc:2"]);
    sandbox.ok(&["trash", "Notes"]);

    // Without the flag nothing is erased: exit code 8.
    let refused = sandbox.fail(&["empty-trash"], 8);
    assert!(refused.contains("Nothing was deleted"), "{refused}");
    assert_eq!(sandbox.json(&["status"])["trashed"], 2);

    let dry = sandbox.ok(&["trash", "doc:2", "--permanently", "--dry-run"]);
    assert_eq!(
        dry,
        "Dry run: nothing was changed. The real run would report:\nErased for good: doc:2 \"Ideas\".\n"
    );
    assert_eq!(sandbox.json(&["status"])["trashed"], 2);
    assert_eq!(
        sandbox.ok(&["trash", "doc:2", "--permanently"]),
        "Erased for good: doc:2 \"Ideas\".\n"
    );
    assert!(
        sandbox
            .fail(&["get", "doc:2"], 3)
            .contains("there is no document")
    );
    assert_eq!(sandbox.json(&["status"])["trashed"], 1);

    let dry = sandbox.ok(&["empty-trash", "--permanently", "--dry-run"]);
    assert!(
        dry.contains("Emptied the trash: erased 1 folder and 1 document for good."),
        "{dry}"
    );
    assert_eq!(sandbox.json(&["status"])["trashed"], 1);
    let emptied = sandbox.json(&["empty-trash", "--permanently"]);
    assert_eq!(emptied["folders"], 1);
    assert_eq!(emptied["documents"], 1);
    assert_eq!(
        sandbox.ok(&["empty-trash", "--permanently"]),
        "The trash was empty.\n"
    );
    assert_eq!(sandbox.ok(&["list", "--trash"]), "The trash is empty.\n");

    // A document that is not in the trash can be erased too, with the flag and only with it.
    assert_eq!(
        sandbox.ok(&["rm", "doc:1", "--permanently"]),
        "Erased for good: doc:1 \"The Salt Road/Part one/Chapter 1\".\n"
    );
    assert_eq!(sandbox.json(&["status"])["documents"], 0);
}

#[test]
fn a_name_that_fits_several_items_is_an_error_that_lists_them_with_their_ids() {
    let sandbox = library();
    sandbox.create("Chapter 1", None, "Another one.");
    sandbox.ok(&["mkdir", "Ideas"]);

    let two = sandbox.fail(&["get", "Chapter 1"], 4);
    assert!(
        two.contains("fits 2 items; name one by its id or by a longer path:"),
        "{two}"
    );
    assert!(
        two.contains("\n  doc:1  The Salt Road/Part one/Chapter 1\n"),
        "{two}"
    );
    assert!(two.contains("\n  doc:4  Chapter 1\n"), "{two}");
    // A longer path, a path from the top and an id are each exact.
    assert!(
        sandbox
            .ok(&["get", "Part one/Chapter 1"])
            .starts_with("# Chapter 1")
    );
    assert_eq!(sandbox.ok(&["get", "/Chapter 1"]), "Another one.\n");
    assert_eq!(sandbox.ok(&["get", "doc:4"]), "Another one.\n");
    // Nothing was changed by a command that could not tell which one was meant.
    let run = sandbox.run(&["trash", "Chapter 1", "--json"]);
    assert_eq!(run.code, 4);
    let error = run.error();
    assert_eq!(error["code"], "ambiguous");
    assert_eq!(error["candidates"].as_array().unwrap().len(), 2);
    assert_eq!(error["candidates"][0]["kind"], "document");
    assert_eq!(error["candidates"][0]["id"], 1);
    assert_eq!(sandbox.json(&["status"])["trashed"], 0);

    // A document and a folder of one name, and a bare number that both kinds have.
    let either = sandbox.fail(&["rename", "Ideas", "Thoughts"], 4);
    assert!(either.contains("\n  doc:2  Ideas\n"), "{either}");
    assert!(either.contains("  folder:4  Ideas\n"), "{either}");
    let number = sandbox.fail(&["trash", "1"], 4);
    assert!(
        number.contains("  doc:1  The Salt Road/Part one/Chapter 1\n"),
        "{number}"
    );
    assert!(number.contains("  folder:1  The Salt Road\n"), "{number}");
    assert_eq!(
        sandbox.ok(&["rename", "folder:4", "Thoughts"]),
        "Renamed folder:4 \"Ideas\" -> \"Thoughts\".\n"
    );
    // Where only a document can be meant, a bare number is the document.
    assert!(sandbox.ok(&["get", "1"]).starts_with("# Chapter 1"));

    let nothing = sandbox.fail(&["info", "Nothing here"], 3);
    assert!(
        nothing.contains("there is no document or folder"),
        "{nothing}"
    );
}
