//! Pages: `writer page add` (one page, several, a list), `writer pages`, reads and exports
//! of a page, the exit codes 14 and 15, the MCP tool `add_page`. Most tests run with the
//! fake check (`THE_WRITER_PAGE_CHECK=fake`, which a debug build reads); three run the
//! real one, WebKit in the tool's child, against a server of the test on 127.0.0.1.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};

use super::harness::Sandbox;

/// A folder `name` in the directory the tool runs in, with an `index.html`.
fn site(sandbox: &Sandbox, name: &str, html: &str) -> PathBuf {
    let dir = sandbox.work().join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("index.html"), html).unwrap();
    dir
}

const ORBITARIUM: &str =
    "<!doctype html><title>Orbitarium</title><h1>The sky</h1><p>Stars at night.</p>";

#[test]
fn a_page_is_added_and_read_back_and_listed() {
    let sandbox = Sandbox::new().fake_page_check();
    site(&sandbox, "orbitarium", ORBITARIUM);
    let added = sandbox.ok(&[
        "page",
        "add",
        "orbitarium",
        "--source-url",
        "https://claude.ai/artifact/ExampleArtifactId00002",
        "--folder",
        "Claude Pages",
        "--create-folders",
        "--author",
        "Claude",
    ]);
    assert_eq!(
        added,
        "added page:1 \u{201c}Orbitarium\u{201d} in Claude Pages (1 file, 0.1 MB)\n"
    );
    // A page is page:<id> everywhere; doc:<id> names it too.
    assert_eq!(sandbox.ok(&["get", "page:1"]), "The sky\nStars at night.\n");
    assert_eq!(
        sandbox.ok(&["get", "doc:1", "--format", "text"]),
        sandbox.ok(&["get", "page:1"])
    );
    let text = sandbox.json(&["get", "page:1"]);
    assert_eq!(text["document"]["kind"], "page");
    assert_eq!(text["page"]["entry"], "index.html");
    assert_eq!(
        text["page"]["source_url"],
        "https://claude.ai/artifact/ExampleArtifactId00002"
    );
    let manifest = sandbox.ok(&["get", "page:1", "--format", "json"]);
    assert!(manifest.contains("\"index.html\""), "{manifest}");

    let info = sandbox.ok(&["info", "page:1"]);
    assert!(
        info.starts_with("page:1  Claude Pages/Orbitarium\n"),
        "{info}"
    );
    for line in [
        "title:     Orbitarium (the page's own)",
        "kind:      a page",
        "source:    https://claude.ai/artifact/ExampleArtifactId00002",
        "opens:     index.html",
        "files:     1 file, 0.1 MB",
        "checked:   by static (no WebKit)",
        "versions:  0",
        "warnings:  none",
    ] {
        assert!(info.contains(line), "{line}: {info}");
    }
    assert!(
        !info.contains("margins"),
        "a page has no page settings: {info}"
    );

    let pages = sandbox.ok(&["pages"]);
    assert!(
        pages.starts_with(
            "page:1  Claude Pages/Orbitarium  https://claude.ai/artifact/ExampleArtifactId00002  taken "
        ),
        "{pages}"
    );
    assert!(pages.ends_with("  1 file, 0.1 MB\n"), "{pages}");
    let listed = sandbox.json(&["pages"]);
    let page = &listed["pages"][0];
    assert_eq!(page["trashed"], false);
    assert!(page["summary"]["content_at"].is_string(), "{page}");
    assert!(page["page"]["updated_at"].is_string(), "{page}");
    assert_eq!(sandbox.ok(&["page", "list", "Claude Pages"]), pages);
    assert_eq!(sandbox.ok(&["pages", "/"]), pages);

    let tree = sandbox.ok(&["list"]);
    assert!(
        tree.contains("    Orbitarium  page:1  (page, changed "),
        "{tree}"
    );
    let found = sandbox.ok(&["search", "stars"]);
    assert!(
        found.contains("page:1  Claude Pages/Orbitarium  (page, changed"),
        "{found}"
    );
    let find = sandbox.ok(&["find", "page:1", "Stars"]);
    assert!(find.contains("The text occurs once in page:1"), "{find}");
    assert_eq!(
        sandbox.ok(&["comments", "page:1"]),
        "No comments in page:1 \"Claude Pages/Orbitarium\".\n"
    );
}

#[test]
fn adding_again_is_unchanged_or_updated_and_a_page_in_the_trash_is_left_there() {
    let sandbox = Sandbox::new().fake_page_check();
    let dir = site(&sandbox, "orbitarium", ORBITARIUM);
    let address = "https://claude.ai/artifact/ExampleArtifactId00002";
    let add = |extra: &[&str]| {
        let mut args = vec!["page", "add", "orbitarium", "--source-url", address];
        args.extend(extra);
        sandbox.ok(&args)
    };
    add(&["--author", "Claude"]);
    // The same address, another way of writing it: the same page.
    let same = sandbox.ok(&[
        "page",
        "add",
        "orbitarium",
        "--source-url",
        "https://Claude.AI/artifact/ExampleArtifactId00002/#top",
    ]);
    assert_eq!(
        same,
        format!("unchanged page:1 \u{201c}Orbitarium\u{201d} ({address})\n")
    );
    let unchanged = sandbox.json(&["page", "add", "orbitarium", "--source-url", address]);
    assert_eq!(unchanged["pages"][0]["outcome"], "unchanged");
    assert_eq!(unchanged["pages"][0]["changed"], false);

    std::fs::write(
        dir.join("index.html"),
        "<!doctype html><title>Orbitarium</title><h1>The sky</h1><p>Stars and planets.</p>",
    )
    .unwrap();
    // A dry run says what would happen and writes nothing.
    let dry = add(&["--dry-run"]);
    assert!(dry.starts_with("Dry run: nothing was changed."), "{dry}");
    assert!(dry.contains("updated page:1"), "{dry}");
    assert_eq!(sandbox.ok(&["get", "page:1"]), "The sky\nStars at night.\n");
    let updated = add(&["--author", "Claude"]);
    assert_eq!(
        updated,
        "updated page:1 \u{201c}Orbitarium\u{201d}: the page as it was is kept as version 1 (1 file, 0.1 MB)\n"
    );
    assert_eq!(
        sandbox.ok(&["get", "page:1"]),
        "The sky\nStars and planets.\n"
    );
    let history = sandbox.ok(&["history", "page:1"]);
    assert!(history.contains("Before an update by Claude"), "{history}");
    assert_eq!(
        sandbox.ok(&["history", "show", "1"]),
        "The sky\nStars at night.\n"
    );

    sandbox.ok(&["trash", "page:1"]);
    let trashed = add(&[]);
    assert_eq!(
        trashed,
        "trashed page:1 \u{201c}Orbitarium\u{201d} is in the trash, so it was not added again: \
         restore it, or add it with --include-trashed\n"
    );
    let listed = sandbox.json(&["pages"]);
    assert_eq!(listed["pages"][0]["trashed"], true);
    assert!(sandbox.ok(&["pages"]).ends_with("  (in the trash)\n"));
    let again = add(&["--include-trashed"]);
    assert!(
        again.starts_with("added page:2 \u{201c}Orbitarium\u{201d} at the top of the library"),
        "{again}"
    );
    sandbox.ok(&["restore", "page:1"]);
}

#[test]
fn a_list_adds_every_page_with_its_own_address_and_the_pieces_it_was_given() {
    let sandbox = Sandbox::new().fake_page_check();
    site(&sandbox, "one", "<title>One</title><p>First page.</p>");
    // The picture lives where the tool cannot fetch it (as claude.ai's asset store);
    // the list names the file that is its content, and nothing is fetched.
    site(
        &sandbox,
        "two",
        "<title>Two</title><p>Second page.</p><img src=\"https://assets.invalid/a/pic.png\">",
    );
    sandbox.picture("lists/pic.png", (4, 4), [200, 10, 10]);
    std::fs::create_dir_all(sandbox.work().join("lists")).unwrap();
    std::fs::write(
        sandbox.work().join("lists/pages.json"),
        json!([
            { "path": "../one", "source_url": "https://claude.ai/artifact/one" },
            { "path": "../two", "source_url": "https://claude.ai/artifact/two", "title": "Second",
              "externals": { "https://assets.invalid/a/pic.png": "pic.png" } },
            { "path": "../three" },
        ])
        .to_string(),
    )
    .unwrap();
    // Relative paths of a list are taken from its folder.
    let run = sandbox.run(&[
        "page",
        "add",
        "--list",
        "lists/pages.json",
        "--folder",
        "Claude Pages",
        "--create-folders",
        "--author",
        "Claude",
    ]);
    assert_eq!(run.code, 2, "the first page that failed: {}", run.stderr);
    let lines: Vec<&str> = run.stdout.lines().collect();
    assert_eq!(lines.len(), 3, "{}", run.stdout);
    assert_eq!(
        lines[0],
        "added page:1 \u{201c}One\u{201d} in Claude Pages (1 file, 0.1 MB)"
    );
    assert!(
        lines[1].starts_with("added page:2 \u{201c}Second\u{201d} in Claude Pages (2 files, "),
        "{}",
        lines[1]
    );
    assert!(lines[1].ends_with("1 piece from the web)"), "{}", lines[1]);
    assert!(
        lines[2].starts_with("not added \u{201c}../three\u{201d}: there is no file"),
        "{}",
        lines[2]
    );
    assert!(
        run.stderr
            .starts_with("writer: 1 of 3 pages were not added"),
        "{}",
        run.stderr
    );
    // As data: one result for each page, with the input it came from.
    let run = sandbox.run(&["page", "add", "--list", "lists/pages.json", "--json"]);
    assert_eq!(run.code, 2);
    let data: Value = serde_json::from_str(&run.stdout).unwrap();
    let pages = data["pages"].as_array().unwrap();
    assert_eq!(pages[0]["outcome"], "unchanged");
    assert_eq!(pages[0]["input"], "../one");
    assert_eq!(pages[1]["outcome"], "unchanged");
    assert_eq!(pages[2]["code"], "invalid");
    assert_eq!(pages[2]["input"], "../three");
    assert_eq!(run.error()["code"], "invalid");

    // A list on standard input; relative paths from where the tool runs.
    let list =
        json!([{ "path": "one", "source_url": "https://claude.ai/artifact/one" }]).to_string();
    let run = sandbox.run_with_input(&["page", "add", "--list", "-"], &list);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stdout.starts_with("unchanged page:1"), "{}", run.stdout);

    // A list cannot bring a page prepared elsewhere.
    let prepared = json!([{ "path": "one", "prepared": { "state": "trashed", "page": 1 } }]);
    let run = sandbox.run_with_input(&["page", "add", "--list", "-"], &prepared.to_string());
    assert_eq!(run.code, 2);
    assert!(
        run.stderr
            .contains("names \"prepared\", which a list cannot give"),
        "{}",
        run.stderr
    );
}

#[test]
fn what_is_for_one_page_is_refused_for_several_and_the_hint_says_how() {
    let sandbox = Sandbox::new().fake_page_check();
    site(&sandbox, "one", "<title>One</title><p>a</p>");
    site(&sandbox, "two", "<title>Two</title><p>b</p>");
    let several = sandbox.fail(
        &[
            "page",
            "add",
            "one",
            "two",
            "--source-url",
            "https://x.test/",
        ],
        2,
    );
    assert!(
        several.contains("--source-url is for one page, and 2 are named"),
        "{several}"
    );
    assert!(several.contains("--list <file.json>"), "{several}");
    let both = sandbox.fail(&["page", "add", "one", "--list", "x.json"], 2);
    assert!(both.contains("not both"), "{both}");
    let none = sandbox.fail(&["page", "add"], 2);
    assert!(none.contains("name the page"), "{none}");
    let address = sandbox.fail(&["page", "add", "https://claude.ai/artifact/x"], 2);
    assert!(
        address.contains("fetching a live site is not supported"),
        "{address}"
    );
    // A folder with several pages at its top and no index.html: --entry says which.
    let dir = sandbox.work().join("multi");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.html"), "<title>A</title><p>a</p>").unwrap();
    std::fs::write(dir.join("b.html"), "<title>B</title><p>b</p>").unwrap();
    let entry = sandbox.fail(&["page", "add", "multi"], 2);
    assert!(
        entry.contains("say which one opens with --entry"),
        "{entry}"
    );
    let added = sandbox.ok(&["page", "add", "multi", "--entry", "b.html"]);
    assert!(
        added.starts_with("added page:1 \u{201c}B\u{201d}"),
        "{added}"
    );
    // Several pages named on the command line, each its own line.
    let both = sandbox.ok(&["page", "add", "one", "two"]);
    assert_eq!(both.lines().count(), 2, "{both}");
}

#[test]
fn a_page_cannot_be_edited_and_the_codes_say_so() {
    let sandbox = Sandbox::new().fake_page_check();
    site(&sandbox, "orbitarium", ORBITARIUM);
    sandbox.ok(&[
        "page",
        "add",
        "orbitarium",
        "--source-url",
        "https://claude.ai/artifact/k",
    ]);
    let doc = sandbox.create("Chapter 2", None, "Text.");
    assert_eq!(doc, "doc:2");
    sandbox.picture("pic.png", (8, 8), [1, 2, 3]);
    for args in [
        &["edit", "page:1", "--old", "sky", "--new", "sea"][..],
        &["append", "page:1", "--content", "More."],
        &["insert", "page:1", "--at-start", "--content", "More."],
        &["set", "page:1", "--content", "New."],
        &["replace-section", "page:1", "The sky", "--content", "x"],
        &["image", "add", "page:1", "pic.png"],
        &[
            "comment", "add", "page:1", "--quote", "sky", "--body", "Hm.",
        ],
    ] {
        let message = sandbox.fail(args, 15);
        assert!(
            message.contains("\u{201c}Orbitarium\u{201d} is a page: it is kept as it was saved"),
            "{args:?}: {message}"
        );
        assert!(
            message.contains("writer page add <file or folder> --source-url <address>"),
            "the hint names the command that brings a newer copy: {message}"
        );
    }
    let run = sandbox.run(&["edit", "page:1", "--old", "sky", "--new", "sea", "--json"]);
    assert_eq!(run.code, 15);
    assert_eq!(run.error()["code"], "wrong_kind");
    // An export of a page in a format of a text: the hint says what a page is written as.
    let md = sandbox.fail(&["export", "page:1", "page.md"], 15);
    assert!(md.contains("a page is written as a folder"), "{md}");
    // A page as a document's format: the other way round.
    let pdf = sandbox.fail(&["export", "doc:2", "chapter.pdf"], 15);
    assert!(pdf.contains("is a document, not a page"), "{pdf}");
    // What works on any item works on a page.
    sandbox.ok(&["rename", "page:1", "Sky map"]);
    assert!(
        sandbox
            .ok(&["info", "page:1"])
            .contains("Sky map (chosen by hand)")
    );
    sandbox.ok(&["move", "page:1", "Maps", "--create-folders"]);
    let copy = sandbox.ok(&["duplicate", "page:1"]);
    assert!(copy.starts_with("Copied to page:3"), "{copy}");
    let trashed = sandbox.ok(&["trash", "page:3"]);
    assert_eq!(
        trashed,
        "Moved to the trash: page:3 \"Maps/Sky map copy\".\n"
    );
    assert!(
        sandbox
            .ok(&["list", "--trash"])
            .contains("page:3  Maps/Sky map copy  (page, trashed")
    );
    sandbox.ok(&["restore", "page:3"]);
    sandbox.ok(&["trash", "page:3", "--permanently"]);

    // The help of the tool explains both codes.
    let help = sandbox.ok(&["--help"]);
    assert!(
        help.contains("14 the page could not be kept whole"),
        "{help}"
    );
    assert!(
        help.contains("15 a page where a document is needed"),
        "{help}"
    );
}

#[test]
fn a_page_that_cannot_be_kept_whole_is_refused_with_14_and_nothing_is_saved() {
    let sandbox = Sandbox::new().fake_page_check();
    // A piece on this Mac's own network: never fetched (and nothing listens there).
    site(
        &sandbox,
        "local",
        "<title>Local</title><link rel=\"stylesheet\" href=\"http://127.0.0.1:9/a.css\"><p>x</p>",
    );
    let message = sandbox.fail(&["page", "add", "local"], 14);
    assert!(
        message.starts_with(
            "writer: \u{201c}Local\u{201d} was not added: it could not be kept whole."
        ),
        "{message}"
    );
    assert!(message.contains("http://127.0.0.1:9/a.css"), "{message}");
    assert!(
        message.trim_end().ends_with("Nothing was saved."),
        "{message}"
    );
    let run = sandbox.run(&["page", "add", "local", "--json"]);
    assert_eq!(run.code, 14);
    assert_eq!(run.error()["code"], "page_incomplete");
    // Nothing was saved: not even the library was made.
    sandbox.fail(&["pages"], 10);
}

#[test]
fn the_warnings_of_a_page_are_printed_under_its_line() {
    let sandbox = Sandbox::new().fake_page_check();
    site(
        &sandbox,
        "desk",
        "<title>Marking Desk</title><p>Desk.</p><script>window.claude.storage.get('x')</script>",
    );
    let added = sandbox.ok(&["page", "add", "desk"]);
    let lines: Vec<&str> = added.lines().collect();
    assert!(
        lines[0].starts_with("added page:1 \u{201c}Marking Desk\u{201d}"),
        "{added}"
    );
    assert!(lines[0].ends_with(", with 1 warning:"), "{added}");
    assert_eq!(
        lines[1],
        "  it uses claude.ai's storage (window.claude): what it saved there is not in this copy"
    );
    let info = sandbox.ok(&["info", "page:1"]);
    assert!(
        info.contains("warnings:\n  it uses claude.ai's storage"),
        "{info}"
    );
}

#[test]
fn a_page_is_exported_as_a_folder_as_the_files_it_came_from_and_as_a_pdf() {
    let sandbox = Sandbox::new().fake_page_check();
    let dir = site(
        &sandbox,
        "styled",
        "<title>Styled</title><link rel=\"stylesheet\" href=\"style.css\"><p>Styled text.</p>",
    );
    std::fs::write(dir.join("style.css"), "p { color: red }").unwrap();
    sandbox.ok(&["page", "add", "styled"]);
    let out = sandbox.root().join("out");
    let said = sandbox.ok(&["export", "page:1", out.join("styled").to_str().unwrap()]);
    assert!(said.starts_with("Exported page:1 \"Styled\" to "), "{said}");
    assert!(said.contains("(a web page folder, 2 files, "), "{said}");
    assert!(said.contains("python3 -m http.server"), "{said}");
    assert_eq!(
        std::fs::read_to_string(out.join("styled/style.css")).unwrap(),
        "p { color: red }"
    );
    // Again: the files are there, and are replaced only when asked.
    let exists = sandbox.fail(
        &["export", "page:1", out.join("styled").to_str().unwrap()],
        12,
    );
    assert!(exists.contains("--overwrite"), "{exists}");
    sandbox.ok(&[
        "export",
        "page:1",
        "--to",
        out.join("styled").to_str().unwrap(),
        "--overwrite",
    ]);
    let original = sandbox.ok(&[
        "export",
        "page:1",
        out.join("original").to_str().unwrap(),
        "--original",
    ]);
    assert!(original.contains("2 files"), "{original}");
    assert!(out.join("original/index.html").is_file());
    // The PDF of the fake check: one page. (The real one is below.)
    let pdf = sandbox.ok(&["export", "page:1", out.join("styled.pdf").to_str().unwrap()]);
    assert!(pdf.contains("(PDF, "), "{pdf}");
    assert!(
        std::fs::read(out.join("styled.pdf"))
            .unwrap()
            .starts_with(b"%PDF")
    );
    // Its text.
    sandbox.ok(&["export", "page:1", out.join("styled.txt").to_str().unwrap()]);
    assert_eq!(
        std::fs::read_to_string(out.join("styled.txt")).unwrap(),
        "Styled text.\n"
    );
    // The path once, as an argument or with --to.
    let twice = sandbox.fail(&["export", "page:1", "a", "--to", "b"], 2);
    assert!(twice.contains("give the path once"), "{twice}");
    let none = sandbox.fail(&["export", "page:1"], 2);
    assert!(none.contains("say where to write it"), "{none}");
    // A folder exported as one file leaves its pages out and says so.
    sandbox.create("Notes", None, "Some notes.");
    let all = sandbox.ok(&["export", "/", out.join("all.md").to_str().unwrap()]);
    assert!(all.contains("Note: 1 page was left out"), "{all}");
}

#[test]
fn a_page_is_added_through_the_mcp_server_one_per_call() {
    let sandbox = Sandbox::new().fake_page_check();
    let dir = site(&sandbox, "orbitarium", ORBITARIUM);
    let mut client = sandbox.mcp();
    client.initialize("2025-11-25", "Test Agent");
    let listed = client.request("tools/list", json!({}));
    let tools = listed["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 37);
    assert_eq!(
        tools
            .iter()
            .filter(|tool| tool["annotations"]["readOnlyHint"] == true)
            .count(),
        12
    );
    assert!(tools.iter().all(|tool| tool["name"] != "list_pages"));
    let add_page = tools
        .iter()
        .find(|tool| tool["name"] == "add_page")
        .unwrap();
    assert_eq!(add_page["annotations"]["readOnlyHint"], false);

    let (text, data) = client.call_ok(
        "add_page",
        json!({
            "path": dir.to_str().unwrap(),
            "source_url": "https://claude.ai/artifact/k",
            "folder": "Claude Pages",
            "create_folders": true,
        }),
    );
    assert_eq!(
        text,
        "added page:1 \u{201c}Orbitarium\u{201d} in Claude Pages (1 file, 0.1 MB)\n"
    );
    assert_eq!(data["outcome"], "added");
    assert_eq!(data["page"]["kind"], "page");
    // A second call: the server read nothing of its input meanwhile.
    let (_, again) = client.call_ok(
        "add_page",
        json!({ "path": dir.to_str().unwrap(), "source_url": "https://claude.ai/artifact/k" }),
    );
    assert_eq!(again["outcome"], "unchanged");
    // Arguments that bring a page prepared elsewhere are refused.
    let (message, error) = client.call_error(
        "add_page",
        json!({ "path": dir.to_str().unwrap(), "prepared": { "state": "trashed", "page": 1 } }),
    );
    assert_eq!(error["code"], "invalid");
    assert!(
        message.contains("There is no argument \"prepared\""),
        "{message}"
    );
    // Reads of a page.
    let (read, _) = client.call_ok("read_document", json!({ "document": "page:1" }));
    assert_eq!(read, "The sky\nStars at night.\n");
    let (_, info) = client.call_ok("document_info", json!({ "document": 1 }));
    assert_eq!(info["page"]["source_url"], "https://claude.ai/artifact/k");
    let (listing, _) = client.call_ok("list_library", json!({}));
    assert!(listing.contains("Orbitarium  page:1  (page,"), "{listing}");
    // Its text cannot be changed; the tool to bring a newer copy is named.
    let (wrong, error) = client.call_error(
        "edit_document",
        json!({ "document": 1, "old_text": "sky", "new_text": "sea" }),
    );
    assert_eq!(error["code"], "wrong_kind");
    assert!(
        wrong.contains("add_page") && wrong.contains("source_url"),
        "{wrong}"
    );
    // A page in the trash: what to do is said in the words of the tool.
    client.call_ok("trash_document", json!({ "document": 1 }));
    let (trashed, data) = client.call_ok(
        "add_page",
        json!({ "path": dir.to_str().unwrap(), "source_url": "https://claude.ai/artifact/k" }),
    );
    assert_eq!(data["outcome"], "trashed");
    assert!(
        trashed.contains("or call add_page again with include_trashed"),
        "{trashed}"
    );
    assert!(!trashed.contains("--"), "{trashed}");
    client.call_ok("restore_document", json!({ "document": 1 }));
    // Export as a folder, the original, and the PDF.
    let out = sandbox.root().join("out");
    let (_, folder) = client.call_ok(
        "export",
        json!({ "document": 1, "path": out.join("k").to_str().unwrap(), "format": "html" }),
    );
    assert_eq!(folder["format"], "html");
    assert_eq!(folder["files"], 1);
    client.call_ok(
        "export",
        json!({ "document": 1, "path": out.join("o").to_str().unwrap(), "original": true }),
    );
    assert!(out.join("o/index.html").is_file());
    let (_, pdf) = client.call_ok(
        "export",
        json!({ "document": 1, "path": out.join("k.pdf").to_str().unwrap(), "format": "pdf" }),
    );
    assert_eq!(pdf["format"], "pdf");
    client.close();
}

/// A server on 127.0.0.1 for the tests with the real check: it answers `/font.woff2`
/// with a few bytes and anything else with 404, and counts what it was asked.
struct Server {
    port: u16,
    hits: Arc<AtomicUsize>,
}

impl Server {
    fn start() -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let hits = Arc::new(AtomicUsize::new(0));
        let counted = hits.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut first = String::new();
                if reader.read_line(&mut first).is_err() {
                    continue;
                }
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                }
                counted.fetch_add(1, Ordering::SeqCst);
                let path = first.split_whitespace().nth(1).unwrap_or("/");
                let (status, kind, body): (&str, &str, &[u8]) = if path == "/font.woff2" {
                    ("200 OK", "font/woff2", b"wOF2 not really a font")
                } else {
                    ("404 Not Found", "text/plain", b"no")
                };
                let head = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                stream.write_all(head.as_bytes()).ok();
                stream.write_all(body).ok();
            }
        });
        Server { port, hits }
    }

    fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }
}

#[test]
fn the_real_check_keeps_a_font_from_the_web_and_reads_the_text_webkit_shows() {
    let sandbox = Sandbox::new().real_page_check();
    let server = Server::start();
    site(
        &sandbox,
        "font",
        &format!(
            "<!doctype html><title>Font page</title><style>@font-face {{ font-family: Kept; src: \
             url(http://127.0.0.1:{}/font.woff2) }} body {{ font-family: Kept }}</style>\
             <h1>Hello font</h1><p id=\"p\">Written by hand.</p>\
             <script>document.getElementById('p').textContent += ' And by a script.'</script>",
            server.port
        ),
    );
    let added = sandbox.ok(&[
        "page",
        "add",
        "font",
        "--source-url",
        "https://claude.ai/artifact/font",
    ]);
    assert!(
        added.starts_with(
            "added page:1 \u{201c}Font page\u{201d} at the top of the library (2 files, "
        ),
        "{added}"
    );
    assert!(added.contains("1 piece from the web"), "{added}");
    assert_eq!(
        server.hits(),
        1,
        "the font was fetched once, when the page was added"
    );
    let text = sandbox.ok(&["get", "page:1"]);
    assert!(text.contains("Hello font"), "{text}");
    assert!(
        text.contains("Written by hand. And by a script."),
        "the text after the page's scripts ran: {text}"
    );
    let info = sandbox.ok(&["info", "page:1"]);
    assert!(info.contains("checked:   by WebKit"), "{info}");

    // Its PDF, as WebKit prints it, in the tool's second child.
    let pdf = sandbox.root().join("out/font.pdf");
    let said = sandbox.ok(&["export", "page:1", pdf.to_str().unwrap()]);
    assert!(said.contains("(PDF, "), "{said}");
    let bytes = std::fs::read(&pdf).unwrap();
    assert!(bytes.starts_with(b"%PDF"), "a PDF");
    assert!(
        bytes.len() > 2_000,
        "more than the fixed page of the fake: {}",
        bytes.len()
    );
    assert_eq!(
        server.hits(),
        1,
        "showing and printing a page fetches nothing"
    );
}

#[test]
fn the_real_check_refuses_a_page_that_fetches_at_run_time_and_names_the_address() {
    let sandbox = Sandbox::new().real_page_check();
    let server = Server::start();
    // The address is put together by the script, so nothing finds it before the page runs.
    site(
        &sandbox,
        "live",
        &format!(
            "<!doctype html><title>Live data</title><p>Numbers.</p><script>\
             const host = '127.0.0.1:' + {};\
             fetch('ht' + 'tp://' + host + '/data.json').catch(() => {{}});</script>",
            server.port
        ),
    );
    let message = sandbox.fail(&["page", "add", "live"], 14);
    assert!(
        message.contains("\u{201c}Live data\u{201d} was not added: it could not be kept whole."),
        "{message}"
    );
    assert!(
        message.contains(&format!("http://127.0.0.1:{}/data.json", server.port)),
        "the address is named: {message}"
    );
    assert!(message.contains("Nothing was saved."), "{message}");
    assert_eq!(server.hits(), 0, "the check let nothing through");
    sandbox.fail(&["pages"], 10);
}
