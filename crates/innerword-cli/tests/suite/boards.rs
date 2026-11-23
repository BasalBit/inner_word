//! Whiteboards: `innerword board new|add|update|remove`, `innerword boards`, the reads of a
//! whiteboard (get, info, find, history) and its other commands, its PDF, the exit codes
//! 2, 3, 9 and 15 with their hints, and the MCP tools through the server's stdio. Most
//! tests print with the fake printer (`INNER_WORD_PAGE_CHECK=fake`, which a debug build
//! reads); one runs the real child, `innerword __board-pdf`, with WebKit and the bundle.

use serde_json::{Value, json};

use super::harness::Sandbox;

/// The drawing of the design: two shapes with labels, an arrow between them with a label,
/// a title and a frame around the three.
const SHAPES: &str = r##"[
 {"type":"rectangle","id":"api","x":100,"y":80,"width":200,"height":90,"backgroundColor":"#a5d8ff","fillStyle":"solid","label":{"text":"API"}},
 {"type":"ellipse","id":"db","x":420,"y":80,"width":160,"height":90,"label":{"text":"Postgres"}},
 {"type":"arrow","id":"reads","x":300,"y":125,"start":{"id":"api"},"end":{"id":"db"},"label":{"text":"reads"}},
 {"type":"text","id":"title","x":100,"y":20,"text":"System overview","fontSize":28},
 {"type":"frame","id":"backend","children":["api","db","reads"],"name":"Backend"}
]"##;

/// The lines of a text that start with `start`.
fn lines_starting<'a>(text: &'a str, start: &str) -> Vec<&'a str> {
    text.lines()
        .filter(|line| line.starts_with(start))
        .collect()
}

#[test]
fn a_whiteboard_is_made_drawn_on_changed_and_read() {
    let sandbox = Sandbox::new().fake_page_check();
    let made = sandbox.ok(&[
        "board",
        "new",
        "Architecture",
        "--in",
        "Sketches",
        "--create-folders",
    ]);
    assert_eq!(
        made,
        "made board:1 \u{201c}Architecture\u{201d} in Sketches (empty)\n"
    );

    // The elements from a file; the picture among them is named by a path relative to
    // the folder of that file, not to where the command runs.
    sandbox.picture("drawings/icon.png", (60, 60), [10, 120, 200]);
    let mut elements: Vec<Value> = serde_json::from_str(SHAPES).unwrap();
    elements.push(json!({"type":"image","id":"icon","x":640,"y":80,"width":120,"path":"icon.png"}));
    std::fs::write(
        sandbox.work().join("drawings/shapes.json"),
        serde_json::to_string(&elements).unwrap(),
    )
    .unwrap();
    // A dry run names the ids the real run gives, and writes nothing.
    let dry = sandbox.json(&[
        "board",
        "add",
        "board:1",
        "--file",
        "drawings/shapes.json",
        "--dry-run",
    ]);
    assert_eq!(dry["dry_run"], true);
    assert_eq!(dry["changed"], false);
    assert!(
        sandbox
            .ok(&["history", "board:1"])
            .starts_with("No versions")
    );
    let added = sandbox.ok(&[
        "board",
        "add",
        "board:1",
        "--file",
        "drawings/shapes.json",
        "--author",
        "Claude",
    ]);
    assert_eq!(
        added,
        "added to board:1 \u{201c}Architecture\u{201d}: rectangle api (label), ellipse db \
         (label), arrow reads (label), text title, frame backend, image icon (icon.png, PNG as \
         it came, 60 x 60 pixels, drawn 120 x 120)\nkept the drawing as it was: version 1 \
         \u{201c}Before a drawing by Claude\u{201d}; \"innerword history restore 1\" brings it \
         back\n"
    );
    let versions = sandbox.json(&["history", "board:1"]);
    let real = versions["versions"][0]["id"].clone();
    assert_eq!(real, 1);
    let real_ids = sandbox.json(&["get", "board:1"])["board"]["elements"].clone();
    let dry_ids: Vec<&Value> = dry["added"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|added| {
            std::iter::once(&added["id"]).chain(added["with"].as_array().unwrap().iter())
        })
        .collect();
    for id in &dry_ids {
        assert!(
            real_ids
                .as_array()
                .unwrap()
                .iter()
                .any(|element| element["id"] == **id),
            "the dry run named {id}, which the real run did not give: {real_ids}"
        );
    }

    // The listing: one element a line, ids first, a label with its container.
    let listing = sandbox.ok(&["get", "board:1"]);
    assert!(
        listing.starts_with(
            "board:1 \u{201c}Architecture\u{201d} in Sketches: 6 elements, 1 picture; drawn area "
        ),
        "{listing}"
    );
    for (start, has) in [
        ("rectangle  api", "label \"API\""),
        ("ellipse    db", "label \"Postgres\""),
        ("arrow      reads", "from api to db"),
        ("text       title", "\"System overview\""),
        ("frame      backend", "holds api, db, reads"),
        ("image      icon", "picture stored"),
    ] {
        let line = lines_starting(&listing, start);
        assert_eq!(line.len(), 1, "{start}: {listing}");
        assert!(line[0].contains(has), "{start}: {}", line[0]);
    }
    // The same in plain text; as JSON, the stored scene, which names the picture's bytes.
    assert_eq!(sandbox.ok(&["get", "board:1", "--format", "text"]), listing);
    let raw = sandbox.ok(&["get", "board:1", "--format", "json"]);
    let scene: Value = serde_json::from_str(&raw).expect("the stored scene is JSON");
    assert_eq!(scene["type"], "excalidraw");
    let files = scene["files"].as_object().unwrap();
    assert_eq!(files.len(), 1);
    assert!(
        files
            .values()
            .all(|file| file["file"].as_str().is_some_and(|id| id.len() == 32)),
        "{raw}"
    );
    let read = sandbox.json(&["get", "board:1"]);
    assert_eq!(read["document"]["kind"], "board");
    assert_eq!(read["board"]["summary"]["pictures"], 1);
    let icon = read["board"]["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|element| element["id"] == "icon")
        .unwrap()
        .clone();
    assert_eq!(icon["picture"], "stored");
    assert_eq!(
        (icon["width"].as_f64(), icon["height"].as_f64()),
        (Some(120.0), Some(120.0))
    );

    // What else reads a whiteboard.
    let info = sandbox.ok(&["info", "board:1"]);
    assert!(
        info.starts_with("board:1  Sketches/Architecture\n"),
        "{info}"
    );
    assert!(
        info.contains("title:     Architecture (its name)"),
        "{info}"
    );
    assert!(info.contains("kind:      a whiteboard"), "{info}");
    assert!(info.contains("1 picture"), "{info}");
    assert_eq!(
        sandbox.json(&["info", "board:1"])["board"]["summary"]["pictures"],
        1
    );
    let found = sandbox.ok(&["find", "board:1", "reads"]);
    assert!(found.contains("  1  reads (arrow label): reads"), "{found}");
    let boards = sandbox.ok(&["boards"]);
    assert!(
        boards.starts_with("board:1  Sketches/Architecture  6 elements, 1 picture  changed "),
        "{boards}"
    );
    assert_eq!(sandbox.ok(&["board", "list", "Sketches"]), boards);
    assert_eq!(sandbox.ok(&["whiteboards", "/"]), boards);
    let listed = sandbox.json(&["boards"]);
    assert_eq!(listed["boards"][0]["elements"], 6);
    assert_eq!(listed["boards"][0]["board"]["kind"], "board");
    let tree = sandbox.ok(&["list"]);
    assert!(
        tree.contains("Architecture  board:1  (whiteboard, changed "),
        "{tree}"
    );

    // Changes by id: what followed is counted, the version is kept first.
    let dry = sandbox.ok(&[
        "board",
        "update",
        "board:1",
        "--content",
        r##"[{"id":"api","dx":40,"backgroundColor":"#ffc9c9"}]"##,
        "--dry-run",
    ]);
    assert!(
        dry.starts_with(
            "Dry run: nothing was changed. The real run would report:\nchanged on board:1 \
             \u{201c}Architecture\u{201d}: api; "
        ),
        "{dry}"
    );
    let changed = sandbox.ok(&[
        "board",
        "update",
        "board:1",
        "--content",
        r##"[{"id":"api","dx":40,"backgroundColor":"#ffc9c9"}]"##,
        "--author",
        "Claude",
    ]);
    assert!(
        changed.starts_with("changed on board:1 \u{201c}Architecture\u{201d}: api; "),
        "{changed}"
    );
    assert!(
        changed.contains("followed (labels, the ends of joined arrows"),
        "{changed}"
    );
    assert!(
        changed.contains("version 2 \u{201c}Before an edit by Claude\u{201d}"),
        "{changed}"
    );
    assert!(sandbox.ok(&["get", "board:1"]).contains("fill #ffc9c9"));
    // Removal by id: a shape with its label; the arrow joined to it stays, unbound.
    let removed = sandbox.ok(&["board", "remove", "board:1", "db", "--author", "Claude"]);
    assert!(
        removed.starts_with(
            "removed from board:1 \u{201c}Architecture\u{201d}: db and 1 label; unbound (they \
             stay): reads\n"
        ),
        "{removed}"
    );
    assert!(
        removed.contains("\u{201c}Before a removal by Claude\u{201d}"),
        "{removed}"
    );
    let history = sandbox.ok(&["history", "board:1"]);
    for label in [
        "Before a drawing by Claude",
        "Before an edit by Claude",
        "Before a removal by Claude",
    ] {
        assert!(history.contains(label), "{history}");
    }
    // A version is read as the listing it was, and is brought back.
    assert!(
        sandbox
            .ok(&["history", "show", "board:1", "1"])
            .starts_with("board:1 \u{201c}Architecture\u{201d}"),
    );
    // (Version 3 is the whiteboard before the removal.)
    sandbox.ok(&["history", "restore", "board:1", "3"]);
    assert!(
        sandbox
            .ok(&["get", "board:1"])
            .contains("label \"Postgres\"")
    );

    // What every item does.
    assert!(
        sandbox
            .ok(&["rename", "board:1", "Overview"])
            .starts_with("Renamed board:1 \"Sketches/Architecture\" -> \"Sketches/Overview\"")
    );
    sandbox.fail(&["rename", "board:1", "--follow-text"], 2);
    let copied = sandbox.ok(&["duplicate", "board:1"]);
    assert!(copied.starts_with("Copied to board:2 "), "{copied}");
    sandbox.ok(&["move", "board:2", "/"]);
    sandbox.ok(&["trash", "board:2"]);
    let trashed = sandbox.fail(
        &[
            "board",
            "add",
            "board:2",
            "--content",
            r#"[{"type":"rectangle","x":0,"y":0}]"#,
        ],
        9,
    );
    assert!(trashed.contains("in the trash"), "{trashed}");
    assert!(sandbox.ok(&["boards"]).contains("(in the trash)"));
    sandbox.ok(&["restore", "board:2"]);
    sandbox.ok(&["trash", "board:2", "--permanently"]);
    assert_eq!(
        sandbox.json(&["boards"])["boards"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn the_exit_codes_of_a_whiteboard_say_what_to_do() {
    let sandbox = Sandbox::new().fake_page_check();
    sandbox.ok(&["board", "new", "Plan", "--content", SHAPES]);
    sandbox.create("Notes", None, "Some notes.");

    // 15: a text command on a whiteboard, a whiteboard command on a document.
    let text = sandbox.fail(&["append", "Plan", "--content", "More."], 15);
    assert!(
        text.starts_with(
            "innerword: \u{201c}Plan\u{201d} is a whiteboard: it is changed with board add, board \
             update and board remove.\nHere: innerword board add, board update and board remove \
             change a whiteboard;"
        ),
        "{text}"
    );
    sandbox.picture("harbour.png", (64, 40), [30, 90, 160]);
    for args in [
        vec!["edit", "Plan", "--old", "API", "--new", "REST"],
        vec!["comment", "add", "Plan", "--quote", "API", "--body", "x"],
        vec!["image", "add", "Plan", "harbour.png"],
    ] {
        sandbox.fail(&args, 15);
    }
    let document = sandbox.fail(
        &[
            "board",
            "add",
            "Notes",
            "--content",
            r#"[{"type":"rectangle","x":0,"y":0}]"#,
        ],
        15,
    );
    assert!(
        document.ends_with(
            "\u{201c}Notes\u{201d} is a document, not a whiteboard.\nHere: \"innerword boards\" \
             lists the whiteboards.\n"
        ),
        "{document}"
    );
    let run = sandbox.run(&["board", "remove", "Notes", "x", "--json"]);
    assert_eq!(run.code, 15);
    assert_eq!(run.error()["code"], "wrong_kind");
    assert!(
        run.error()["hint"]
            .as_str()
            .unwrap()
            .contains("innerword boards")
    );

    // 3: an element the whiteboard does not have; it names the ones it has.
    let missing = sandbox.fail(
        &[
            "board",
            "update",
            "Plan",
            "--content",
            r#"[{"id":"cache","dx":1}]"#,
        ],
        3,
    );
    assert!(
        missing.starts_with("innerword: there is no element \u{201c}cache\u{201d}"),
        "{missing}"
    );
    assert!(missing.contains("api"), "{missing}");
    assert!(
        missing.ends_with("\nHere: \"innerword get <board>\" lists the elements with their ids.\n"),
        "{missing}"
    );
    sandbox.fail(&["board", "remove", "Plan", "cache"], 3);
    let unknown_end = sandbox.fail(
        &[
            "board",
            "add",
            "Plan",
            "--content",
            r#"[{"type":"arrow","x":0,"y":0,"start":{"id":"api"},"end":{"id":"cache"}}]"#,
        ],
        3,
    );
    assert!(unknown_end.contains("cache"), "{unknown_end}");

    // 2: a skeleton that cannot be read, a text with a size, elements that are no JSON,
    // no ids to remove, no elements at all.
    let field = sandbox.fail(
        &[
            "board",
            "add",
            "Plan",
            "--content",
            r#"[{"type":"rectangle","y":0}]"#,
        ],
        2,
    );
    assert!(field.contains("the element at index 0 (x)"), "{field}");
    let sized = sandbox.fail(
        &[
            "board",
            "add",
            "Plan",
            "--content",
            r#"[{"type":"text","x":0,"y":0,"text":"Hi","width":40}]"#,
        ],
        2,
    );
    assert!(sized.contains("(width)"), "{sized}");
    let unknown = sandbox.fail(
        &[
            "board",
            "add",
            "Plan",
            "--content",
            r#"[{"type":"iframe","x":0,"y":0}]"#,
        ],
        2,
    );
    assert!(unknown.contains("index 0"), "{unknown}");
    let not_json = sandbox.fail(&["board", "add", "Plan", "--content", "a box"], 2);
    assert!(
        not_json.contains("the elements cannot be read as JSON"),
        "{not_json}"
    );
    let not_objects = sandbox.fail(&["board", "add", "Plan", "--content", "[1, 2]"], 2);
    assert!(
        not_objects.contains("the one at index 0 is not an object"),
        "{not_objects}"
    );
    let no_ids = sandbox.fail(&["board", "remove", "Plan"], 2);
    assert!(no_ids.contains("name the elements to remove"), "{no_ids}");
    let none = sandbox.fail(&["board", "add", "Plan"], 2);
    assert!(none.contains("no elements were given"), "{none}");
    // One element alone is an array of one.
    sandbox.ok(&[
        "board",
        "add",
        "Plan",
        "--content",
        r#"{"type":"diamond","id":"gate","x":700,"y":80}"#,
    ]);
    // A picture that is not there: where a relative path was looked for.
    std::fs::create_dir_all(sandbox.work().join("drawings")).unwrap();
    std::fs::write(
        sandbox.work().join("drawings/photo.json"),
        r#"[{"type":"image","x":0,"y":0,"path":"harbour.png"}]"#,
    )
    .unwrap();
    let no_file = sandbox.fail(
        &["board", "add", "Plan", "--file", "drawings/photo.json"],
        2,
    );
    assert!(
        no_file.contains(
            "the element at index 0 (path): there is no file \u{201c}harbour.png\u{201d}"
        ),
        "{no_file}"
    );
    assert!(
        no_file.contains(
            "A path that does not begin with / or ~/ is taken from the folder of --file: "
        ) && no_file.contains("drawings"),
        "{no_file}"
    );
    // A file that is no picture is refused with the sentence of `image add`.
    std::fs::write(sandbox.work().join("notes.png"), "not a picture").unwrap();
    let no_picture = sandbox.fail(
        &[
            "board",
            "add",
            "Plan",
            "--content",
            r#"[{"type":"image","x":0,"y":0,"path":"notes.png"}]"#,
        ],
        2,
    );
    assert!(no_picture.contains("notes.png"), "{no_picture}");

    // 9 and 15: the PDF of an empty whiteboard, a whiteboard as a text.
    sandbox.ok(&["board", "new", "Empty"]);
    let empty = sandbox.fail(&["export", "Empty", "empty.pdf"], 9);
    assert!(
        empty.contains("\u{201c}Empty\u{201d} is empty: there is nothing to export."),
        "{empty}"
    );
    let markdown = sandbox.fail(&["export", "Plan", "plan.md"], 15);
    assert!(
        markdown.ends_with(
            "is a whiteboard, and a whiteboard is exported as a PDF only.\nHere: a path that \
             ends in .pdf.\n"
        ),
        "{markdown}"
    );
    // The PDF (the fake printer: one empty page of the size Rust estimates).
    let said = sandbox.ok(&["export", "Plan", "plan.pdf"]);
    assert!(said.starts_with("Exported board:1 \"Plan\" to "), "{said}");
    assert!(said.contains("(PDF, "), "{said}");
    assert!(
        std::fs::read(sandbox.work().join("plan.pdf"))
            .unwrap()
            .starts_with(b"%PDF")
    );
    sandbox.fail(&["export", "Plan", "plan.pdf"], 12);
    // A folder exported as one file leaves its whiteboards out and says so.
    let all = sandbox.ok(&["export", "/", "all.md"]);
    assert!(all.contains("whiteboard"), "{all}");
}

#[test]
fn a_whiteboard_is_drawn_through_the_mcp_server() {
    let sandbox = Sandbox::new().fake_page_check();
    let picture = sandbox.picture("harbour.png", (64, 40), [30, 90, 160]);
    let mut client = sandbox.mcp();
    client.initialize("2025-11-25", "Test Agent");
    let listed = client.request("tools/list", json!({}));
    let tools = listed["result"]["tools"].as_array().unwrap();
    for (name, read_only) in [
        ("create_board", false),
        ("add_to_board", false),
        ("update_board", false),
        ("remove_from_board", false),
    ] {
        let tool = tools
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap_or_else(|| panic!("{name} is listed"));
        assert_eq!(tool["annotations"]["readOnlyHint"], read_only, "{name}");
        assert!(tool["inputSchema"]["properties"].get("pictures").is_none());
    }
    assert!(tools.iter().all(|tool| tool["name"] != "list_boards"));

    let elements: Value = serde_json::from_str(SHAPES).unwrap();
    let (made, data) = client.call_ok(
        "create_board",
        json!({ "title": "Architecture", "folder": "Sketches", "create_folders": true, "elements": elements }),
    );
    assert!(
        made.starts_with(
            "made board:1 \u{201c}Architecture\u{201d} in Sketches: rectangle api (label), "
        ),
        "{made}"
    );
    assert_eq!(data["board"]["kind"], "board");
    assert_eq!(data["added"].as_array().unwrap().len(), 5);

    // A picture by its whole path: read by the server.
    let (added, data) = client.call_ok(
        "add_to_board",
        json!({ "board": "board:1", "elements": [
            { "type": "image", "id": "photo", "x": 640, "y": 80, "path": picture.to_str().unwrap() }
        ] }),
    );
    assert!(
        added.starts_with(
            "added to board:1 \u{201c}Architecture\u{201d}: image photo (harbour.png, PNG as it \
             came, 64 x 40 pixels, drawn 64 x 40)\n"
        ),
        "{added}"
    );
    assert!(
        added.contains("restore_version with version 1 brings it back"),
        "the tool's own name: {added}"
    );
    assert_eq!(data["pictures"][0]["id"], "photo");
    // A relative path: the server is given no folder, and says so.
    let (refused, error) = client.call_error(
        "add_to_board",
        json!({ "board": 1, "elements": [{ "type": "image", "x": 0, "y": 0, "path": "harbour.png" }] }),
    );
    assert_eq!(error["code"], "invalid");
    assert!(refused.contains("relative path"), "{refused}");
    // Pictures handed in with the arguments are refused.
    let (_, error) = client.call_error(
        "add_to_board",
        json!({ "board": 1, "elements": [], "pictures": [] }),
    );
    assert_eq!(error["code"], "invalid");

    let (listing, data) = client.call_ok("read_document", json!({ "document": "board:1" }));
    assert!(listing.contains("image      photo"), "{listing}");
    assert_eq!(data["board"]["summary"]["pictures"], 1);
    let (changed, _) = client.call_ok(
        "update_board",
        json!({ "board": 1, "changes": "[{\"id\": \"photo\", \"dx\": 20}]" }),
    );
    assert!(changed.starts_with("changed on board:1"), "{changed}");
    let (removed, data) = client.call_ok(
        "remove_from_board",
        json!({ "board": 1, "ids": ["db"], "author": "Claude" }),
    );
    assert!(removed.contains("unbound (they stay): reads"), "{removed}");
    assert_eq!(data["unbound"], json!(["reads"]));
    // A text tool on a whiteboard names the tools of whiteboards.
    let (wrong, error) = client.call_error(
        "append_to_document",
        json!({ "document": 1, "markdown": "More." }),
    );
    assert_eq!(error["code"], "wrong_kind");
    assert!(
        wrong.contains("That is add_to_board, update_board or remove_from_board"),
        "{wrong}"
    );
    // Its PDF, and nothing else.
    let out = sandbox.root().join("out");
    let (exported, _) = client.call_ok(
        "export",
        json!({ "document": 1, "path": out.join("plan.pdf").to_str().unwrap() }),
    );
    assert!(exported.contains("(PDF, "), "{exported}");
    let (not_pdf, error) = client.call_error(
        "export",
        json!({ "document": 1, "path": out.join("plan.md").to_str().unwrap() }),
    );
    assert_eq!(error["code"], "wrong_kind");
    assert!(
        not_pdf.contains("That is export with format \"pdf\""),
        "{not_pdf}"
    );
    client.close();
}

#[link(name = "PDFKit", kind = "framework")]
unsafe extern "C" {}

/// What PDFKit reads in a PDF: its pages, the size of the first, its text.
// `objc`'s macros name a cfg of their own crate (`cargo-clippy`).
#[allow(unexpected_cfgs)]
fn read_pdf(bytes: &[u8]) -> (usize, (f64, f64), String) {
    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};
    #[repr(C)]
    #[derive(Clone, Copy, Debug)]
    struct Rect {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    }
    unsafe impl objc::Encode for Rect {
        fn encode() -> objc::Encoding {
            unsafe { objc::Encoding::from_str("{CGRect={CGPoint=dd}{CGSize=dd}}") }
        }
    }
    // SAFETY: Foundation's and PDFKit's own messages; what is made is released here.
    unsafe {
        let data: *mut Object = msg_send![class!(NSData),
            dataWithBytes: bytes.as_ptr() as *const std::ffi::c_void
            length: bytes.len()];
        let document: *mut Object = msg_send![class!(PDFDocument), alloc];
        let document: *mut Object = msg_send![document, initWithData: data];
        assert!(!document.is_null(), "PDFKit reads the PDF");
        let pages: usize = msg_send![document, pageCount];
        let page: *mut Object = msg_send![document, pageAtIndex: 0usize];
        // kPDFDisplayBoxMediaBox
        let bounds: Rect = msg_send![page, boundsForBox: 0isize];
        let text: *mut Object = msg_send![document, string];
        let text = if text.is_null() {
            String::new()
        } else {
            let utf8: *const std::os::raw::c_char = msg_send![text, UTF8String];
            std::ffi::CStr::from_ptr(utf8)
                .to_string_lossy()
                .into_owned()
        };
        let _: () = msg_send![document, release];
        (pages, (bounds.width, bounds.height), text)
    }
}

/// The size `innerword_board` estimates for the PDF of the whiteboard `board`: its stored
/// scene's drawing and the margin.
fn estimated_size(sandbox: &Sandbox, board: &str) -> (f64, f64) {
    let raw = sandbox.ok(&["get", board, "--format", "json"]);
    innerword_board::Scene::from_stored(&raw)
        .expect("the stored scene")
        .scene
        .pdf_size(
            innerword_board::limits::PDF_PADDING,
            innerword_board::limits::PDF_MAX_SIDE,
        )
        .expect("a scene with elements")
}

/// The PDF of a whiteboard as the tool makes it for an agent: WebKit in the tool's child
/// (`innerword __board-pdf`), the bundle's export page, the stored scene and its pictures.
#[test]
fn the_real_child_makes_the_pdf_of_a_whiteboard() {
    let sandbox = Sandbox::new().real_page_check();
    // The extremes of the drawing are shapes, and its texts lie well inside them, so that
    // the estimate of text widths in Rust cannot move the bounds.
    sandbox.ok(&[
        "board",
        "new",
        "Plan",
        "--content",
        r##"[
          {"type":"rectangle","id":"api","x":100,"y":80,"width":240,"height":120,"backgroundColor":"#a5d8ff","fillStyle":"solid","label":{"text":"API"}},
          {"type":"ellipse","id":"db","x":560,"y":80,"width":220,"height":120,"label":{"text":"Postgres"}},
          {"type":"arrow","id":"reads","x":340,"y":140,"start":{"id":"api"},"end":{"id":"db"},"label":{"text":"reads"}},
          {"type":"rectangle","id":"base","x":100,"y":300,"width":680,"height":100,"label":{"text":"Backend"}}
        ]"##,
    ]);
    let pdf = sandbox.root().join("out/plan.pdf");
    let said = sandbox.ok(&["export", "Plan", pdf.to_str().unwrap()]);
    assert!(said.contains("(PDF, "), "{said}");
    let bytes = std::fs::read(&pdf).unwrap();
    let (pages, size, text) = read_pdf(&bytes);
    assert_eq!(pages, 1, "one page");
    let estimated = estimated_size(&sandbox, "Plan");
    assert!(
        (size.0 - estimated.0).abs() <= 2.0 && (size.1 - estimated.1).abs() <= 2.0,
        "the page {size:?} is the drawing and its margin, {estimated:?}"
    );
    for label in ["API", "Postgres", "reads", "Backend"] {
        assert!(text.contains(label), "the PDF's text has {label}: {text:?}");
    }

    // A picture an agent added is a picture of the PDF.
    let photo = sandbox.picture("photo.png", (80, 50), [200, 120, 40]);
    sandbox.ok(&[
        "board",
        "new",
        "Photo",
        "--content",
        &json!([
            {"type":"rectangle","id":"frame","x":0,"y":0,"width":200,"height":140},
            {"type":"image","id":"photo","x":20,"y":20,"path": photo.to_str().unwrap()}
        ])
        .to_string(),
    ]);
    let pdf = sandbox.root().join("out/photo.pdf");
    sandbox.ok(&["export", "Photo", pdf.to_str().unwrap()]);
    let bytes = std::fs::read(&pdf).unwrap();
    let (pages, size, _) = read_pdf(&bytes);
    assert_eq!(pages, 1);
    let estimated = estimated_size(&sandbox, "Photo");
    assert!(
        (size.0 - estimated.0).abs() <= 2.0 && (size.1 - estimated.1).abs() <= 2.0,
        "{size:?} {estimated:?}"
    );
    assert!(
        contains(&bytes, b"/Subtype /Image"),
        "the PDF holds the picture as an image"
    );
    // The child's directory went with it, and no half-written file is left beside the PDF.
    let mut left = Vec::new();
    for private in std::fs::read_dir(sandbox.tmp()).unwrap().flatten() {
        if !private
            .file_name()
            .to_string_lossy()
            .starts_with("inner-word-")
        {
            continue;
        }
        for entry in std::fs::read_dir(private.path()).unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("board-pdf") {
                left.push(name);
            }
        }
    }
    assert!(left.is_empty(), "{left:?}");
    let mut written: Vec<String> = std::fs::read_dir(sandbox.root().join("out"))
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    written.sort();
    assert_eq!(written, ["photo.pdf", "plan.pdf"]);
}

fn contains(bytes: &[u8], what: &[u8]) -> bool {
    bytes.windows(what.len()).any(|window| window == what)
}
