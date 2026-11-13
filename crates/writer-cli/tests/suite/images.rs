//! Images through the binary: `writer image add`, `writer images`, image files named in
//! the Markdown a command is given, the pictures of an export, and the same through the
//! MCP server.

use serde_json::{Value, json};

use super::harness::Sandbox;

const CHAPTER: &str =
    "# Chapter 1\n\nThe keeper could not swim.\n\n## The harbour\n\nGulls, rope, tar.\n";

fn library() -> Sandbox {
    let sandbox = Sandbox::new();
    assert_eq!(sandbox.create("Chapter 1", None, CHAPTER), "doc:1");
    sandbox
}

/// The line an image is in Markdown.
fn line(description: &str, id: &str) -> String {
    format!("![{description}](image:{id})")
}

/// The id the library gave the first image of a document.
fn first_id(sandbox: &Sandbox, document: &str) -> String {
    sandbox.json(&["images", document])["images"][0]["id"]
        .as_str()
        .expect("the document has an image")
        .to_string()
}

#[test]
fn an_image_is_added_with_every_option_and_listed() {
    let sandbox = library();
    sandbox.picture("harbour.png", (800, 500), [30, 90, 160]);

    // Tried first: the report of the real run, and nothing changed.
    let args = [
        "image",
        "add",
        "doc:1",
        "harbour.png",
        "--alt",
        "The harbour at dusk",
        "--width",
        "37",
        "--align",
        "right",
        "--at",
        "after-heading",
        "--heading",
        "The harbour",
        "--heading-occurrence",
        "1",
        "--author",
        "Claude",
    ];
    let mut tried = args.to_vec();
    tried.push("--dry-run");
    let dry = sandbox.ok(&tried);
    assert!(
        dry.starts_with(
            "Dry run: nothing was changed. The real run would report:\nAdded an image to doc:1 \
             \"Chapter 1\": blocks: 1 added; images: 1 added; 12 -> 12 words.\nUnder: \"The harbour\"\n\
             The image: ![The harbour at dusk](image:"
        ),
        "{dry}"
    );
    assert_eq!(sandbox.ok(&["get", "doc:1"]), CHAPTER);
    assert_eq!(
        sandbox.ok(&["images", "doc:1"]),
        "No images in doc:1 \"Chapter 1\".\n"
    );
    assert!(
        !sandbox.library().join("images.db").exists(),
        "a dry run stored a picture"
    );

    let added = sandbox.json(&args);
    assert_eq!(added["changed"], true);
    assert_eq!(
        (&added["images_added"], &added["blocks_added"]),
        (&json!(1), &json!(1))
    );
    let image = &added["image"];
    let id = image["id"].as_str().unwrap().to_string();
    assert_eq!(id.len(), 32);
    assert_eq!(
        *image,
        json!({
            "id": id, "block": 3, "format": "png", "pixels": [800, 500], "width_percent": 37,
            "align": "right", "description": "The harbour at dusk", "missing": false,
        })
    );
    assert_eq!(
        added["snapshot"]["label"], "Before an insertion by Claude",
        "the text as it was is kept"
    );

    // Printed, the document has the image as a line; listed, with what the line does
    // not say.
    assert_eq!(
        sandbox.ok(&["get", "doc:1"]),
        format!(
            "# Chapter 1\n\nThe keeper could not swim.\n\n## The harbour\n\n{}\n\nGulls, rope, tar.\n",
            line("The harbour at dusk", &id)
        )
    );
    assert_eq!(
        sandbox.ok(&["images", "doc:1"]),
        format!(
            "1 image in doc:1 \"Chapter 1\", in the order of the text:\n  block 3  {}, 800 x 500 \
             pixels (PNG), 37 % of the text column wide, at the right\nIn the Markdown of the \
             document each is the line shown here: keep it where the image should stay.\n",
            line("The harbour at dusk", &id)
        )
    );
    let listed = sandbox.json(&["image", "list", "doc:1"]);
    assert_eq!(listed["document"]["id"], 1);
    assert_eq!(listed["images"], json!([image]));
    assert_eq!(sandbox.json(&["get", "doc:1"])["images"], json!([image]));
    // As text the result says the same, and how to take the change back.
    sandbox.picture("small.png", (120, 60), [200, 40, 40]);
    let said = sandbox.ok(&["image", "add", "doc:1", "small.png"]);
    let small = sandbox.json(&["images", "doc:1"])["images"][1].clone();
    assert_eq!(
        said,
        format!(
            "Added an image to doc:1 \"Chapter 1\": blocks: 1 added; images: 1 added; 12 -> 12 words.\n\
             Under: \"The harbour\"\n\
             The image: {}, 120 x 60 pixels (PNG), 27 % of the text column wide, centered\n\
             The text as it was is kept as version 2 (Before an insertion by an agent); \"writer \
             history restore 2\" brings it back.\n",
            line("", small["id"].as_str().unwrap())
        )
    );
    assert_eq!(
        (&small["block"], &small["align"]),
        (&json!(5), &json!("center"))
    );

    // The file by its whole path, from the home folder, and with a percent sign.
    let whole = sandbox.work().join("harbour.png");
    let by_path = sandbox.json(&[
        "image",
        "add",
        "doc:1",
        whole.to_str().unwrap(),
        "--at",
        "start",
        "--width",
        "50%",
        "--align",
        "LEFT",
    ]);
    assert_eq!(
        (
            &by_path["image"]["block"],
            &by_path["image"]["width_percent"],
            &by_path["image"]["align"]
        ),
        (&json!(0), &json!(50), &json!("left"))
    );
    sandbox.picture_at(
        &sandbox.home().join("Pictures/home.png"),
        (30, 30),
        [9, 120, 9],
    );
    let from_home = sandbox.json(&[
        "image",
        "add",
        "doc:1",
        "~/Pictures/home.png",
        "--at",
        "end",
    ]);
    assert_eq!(from_home["image"]["pixels"], json!([30, 30]));
    assert_eq!(
        sandbox.json(&["images", "doc:1"])["images"]
            .as_array()
            .unwrap()
            .len(),
        4
    );

    // What is wrong on the command line is said in its words, with exit code 2.
    let wrong: [(&[&str], &str); 7] = [
        (
            &["image", "add", "doc:1", "nothing.png"],
            "writer: there is no file \u{201c}nothing.png\u{201d}; give the path of an image file\n\
             A path that does not begin with / or ~/ is taken from where the command runs: ",
        ),
        (
            &["image", "add", "doc:1", "harbour.png", "--at", "middle"],
            "writer: --at is one of end, start, before-heading, after-heading, end-of-section, and \
             \"middle\" is none of them\nusage: writer image add <document> <file> [options]\n",
        ),
        (
            &["image", "add", "doc:1", "harbour.png", "--align", "justify"],
            "writer: --align is one of left, center, right, and \"justify\" is none of them\n",
        ),
        (
            &["image", "add", "doc:1", "harbour.png", "--width", "3"],
            "writer: --width is a share of the text column in percent, from 5 to 100, and \"3\" is \
             not\n",
        ),
        (
            &[
                "image",
                "add",
                "doc:1",
                "harbour.png",
                "--at",
                "after-heading",
            ],
            "writer: this position is relative to a heading: name the heading\n",
        ),
        (
            &["image", "add", "doc:1"],
            "writer: <file> is missing: The image file",
        ),
        (
            &["image", "add", "doc:1", "/no/such/folder/harbour.png"],
            "writer: there is no file \u{201c}/no/such/folder/harbour.png\u{201d}; give the path of \
             an image file\n",
        ),
    ];
    for (args, message) in wrong {
        let said = sandbox.fail(args, 2);
        assert!(said.starts_with(message), "{args:?}: {said}");
    }
    let absolute = sandbox.fail(&["image", "add", "doc:1", "/no/such/folder/harbour.png"], 2);
    assert!(!absolute.contains("where the command runs"), "{absolute}");
    // A file that is no picture, and one that is a kind the library does not take.
    std::fs::write(sandbox.work().join("notes.pdf"), b"%PDF-1.7\n").unwrap();
    std::fs::write(sandbox.work().join("words.png"), b"words").unwrap();
    assert_eq!(
        sandbox.fail(&["image", "add", "doc:1", "notes.pdf"], 2),
        "writer: \u{201c}notes.pdf\u{201d} is a PDF file. Images can be PNG, JPEG, GIF, HEIC, TIFF, \
         WebP or BMP.\n"
    );
    assert_eq!(
        sandbox.fail(&["image", "add", "doc:1", "words.png"], 2),
        "writer: \u{201c}words.png\u{201d} is not an image.\n"
    );
    assert!(
        sandbox
            .fail(&["image", "add", "No such document", "harbour.png"], 3)
            .contains("there is no document")
    );
    assert_eq!(
        sandbox.json(&["images", "doc:1"])["images"]
            .as_array()
            .unwrap()
            .len(),
        4
    );

    // An edit whose old text runs across an image is refused in so many words; with the
    // image named as it reads, it is part of what is replaced and stays where the new
    // text names it, as it was.
    assert_eq!(
        sandbox.fail(
            &[
                "edit",
                "doc:1",
                "--old",
                "The harbour\n\nGulls",
                "--new",
                "x"
            ],
            2
        ),
        "writer: the text to replace runs across an image; read the place again and replace the \
         text before and after the image separately, or name the image as it reads in Markdown\n"
    );
    let image_line = line("The harbour at dusk", &id);
    let named = sandbox.json(&[
        "edit",
        "doc:1",
        "--old",
        &format!("## The harbour\n\n{image_line}\n\nGulls, rope, tar."),
        "--new",
        &format!("## The harbour\n\n{image_line}\n\nGulls, rope and tar."),
    ]);
    assert_eq!(named["matched_as"], "markdown");
    assert_eq!(
        (&named["images_added"], &named["images_removed"]),
        (&json!(0), &json!(0))
    );
    let kept = sandbox.json(&["images", "doc:1"])["images"][1].clone();
    assert_eq!(
        (&kept["width_percent"], &kept["align"], &kept["description"]),
        (&json!(37), &json!("right"), &json!("The harbour at dusk"))
    );

    // The help says that the one changes the library and that the other only reads.
    let help = sandbox.ok(&["image", "add", "--help"]);
    assert!(help.contains("This command changes the library."), "{help}");
    assert!(
        help.contains("--alt <text>") && help.contains("--heading-occurrence <n>"),
        "{help}"
    );
    let list = sandbox.ok(&["images", "--help"]);
    assert!(list.starts_with("writer image list: "), "{list}");
    assert!(
        !list.contains("This command changes the library."),
        "{list}"
    );
    assert_eq!(sandbox.ok(&["image", "--help"]), list);
    assert_eq!(sandbox.ok(&["help", "image", "add"]), help);
}

#[test]
fn an_image_file_named_in_the_text_is_brought_in_from_where_the_text_came_from() {
    let sandbox = library();
    // A text in a folder of its own, with a picture beside it; another picture where the
    // command runs, and one in the home folder.
    let texts = sandbox.root().join("texts");
    sandbox.picture_at(&texts.join("pictures/beside.png"), (60, 40), [220, 30, 30]);
    sandbox.picture("here.png", (40, 60), [30, 30, 220]);
    sandbox.picture_at(&sandbox.home().join("home.png"), (50, 50), [30, 160, 60]);
    std::fs::write(texts.join("notes.pdf"), b"%PDF-1.7\n").unwrap();
    std::fs::write(
        texts.join("colours.md"),
        "# Colours\n\n![Beside the text](pictures/beside.png)\n\n![At home](~/home.png)\n\n\
         ![Not there](missing.png)\n\n![No picture](notes.pdf)\n\n![Far](https://example.com/far.png)\n",
    )
    .unwrap();
    let file = texts.join("colours.md");
    let created = sandbox.json(&[
        "create",
        "--file",
        file.to_str().unwrap(),
        "--author",
        "Claude",
    ]);
    assert_eq!(
        created["warnings"],
        json!([
            "an image was kept as a link to it (it is at an address on the network, or inside a \
             heading, a quote or a table): line 11",
            "\u{201c}missing.png\u{201d} was not found. It was kept as a link.",
            "\u{201c}notes.pdf\u{201d} is a PDF file. Images can be PNG, JPEG, GIF, HEIC, TIFF, WebP or \
             BMP. It was kept as a link.",
        ])
    );
    let images = sandbox.json(&["images", "doc:2"])["images"].clone();
    let ids: Vec<&str> = images
        .as_array()
        .unwrap()
        .iter()
        .map(|image| image["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids.len(), 2);
    assert_eq!(images[0]["pixels"], json!([60, 40]));
    assert_eq!(images[1]["pixels"], json!([50, 50]));
    // An image that Markdown brings in stands at the left, at its own size.
    assert_eq!(
        (&images[0]["align"], &images[0]["width_percent"]),
        (&json!("left"), &json!(13))
    );
    assert_eq!(
        sandbox.ok(&["get", "doc:2"]),
        format!(
            "# Colours\n\n{}\n\n{}\n\n[Not there](missing.png)\n\n[No picture](notes.pdf)\n\n\
             [Far](https://example.com/far.png)\n",
            line("Beside the text", ids[0]),
            line("At home", ids[1]),
        )
    );
    // As text, the notes of the command.
    let said = sandbox.ok(&[
        "append",
        "doc:2",
        "--content",
        "![Here](here.png)\n\n![Nowhere](nowhere.png)",
    ]);
    assert!(
        said.contains("blocks: 2 added; images: 1 added;")
            && said.contains(
                "Note: \u{201c}nowhere.png\u{201d} was not found. It was kept as a link.\n"
            ),
        "{said}"
    );
    // Text that is piped in has no folder of its own: where the command runs.
    let piped = sandbox.run_with_input(
        &["insert", "doc:2", "--at-start", "--stdin", "--json"],
        "![Here again](here.png)\n",
    );
    assert_eq!(piped.code, 0, "{}", piped.stderr);
    let piped: Value = serde_json::from_str(&piped.stdout).unwrap();
    assert_eq!(piped["images_added"], 1);
    // The new text of an edit, from a file, in the folder of that file.
    std::fs::write(
        texts.join("new.md"),
        "# Colours\n\n![Once more](pictures/beside.png)\n",
    )
    .unwrap();
    let edited = sandbox.json(&[
        "edit",
        "doc:2",
        "--old",
        "# Colours",
        "--new-file",
        texts.join("new.md").to_str().unwrap(),
    ]);
    assert_eq!(edited["images_added"], 1, "{edited}");
    assert_eq!(
        sandbox.json(&["images", "doc:2"])["images"]
            .as_array()
            .unwrap()
            .len(),
        5
    );

    // Written back as it was read, a text keeps its images; without a line, the image
    // of that line goes, and the command says where it still is.
    let read = sandbox.ok(&["get", "doc:2"]);
    let same = sandbox.run_with_input(&["set", "doc:2", "--json"], &read);
    assert_eq!(same.code, 0, "{}", same.stderr);
    let same: Value = serde_json::from_str(&same.stdout).unwrap();
    assert_eq!(same["changed"], false, "{same}");
    let without: String = read
        .split("\n\n")
        .filter(|block| !block.contains("![Here again]"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let fewer = sandbox.run_with_input(&["set", "doc:2"], &without);
    assert_eq!(fewer.code, 0, "{}", fewer.stderr);
    assert!(
        fewer.stdout.contains("images: 1 removed;")
            && fewer.stdout.contains(
                "Note: 1 image was removed; it is in the version kept before this change\n"
            ),
        "{}",
        fewer.stdout
    );
}

#[test]
fn an_export_writes_the_pictures_beside_the_text() {
    let sandbox = library();
    sandbox.picture("harbour.png", (64, 40), [30, 90, 160]);
    sandbox.ok(&[
        "image",
        "add",
        "doc:1",
        "harbour.png",
        "--alt",
        "The harbour",
    ]);
    let id = first_id(&sandbox, "doc:1");
    let picture = sandbox
        .root()
        .join("work/out/chapter images")
        .join(format!("image-{}.png", &id[..8]));

    let tried = sandbox.json(&["export", "doc:1", "--to", "out/chapter.md", "--dry-run"]);
    assert_eq!(tried["assets"], json!([picture.to_str().unwrap()]));
    assert!(!sandbox.work().join("out").exists());
    let said = sandbox.ok(&["export", "doc:1", "--to", "out/chapter.md"]);
    assert!(
        said.starts_with("Exported \"Chapter 1\" (1 document) to "),
        "{said}"
    );
    assert!(
        said.ends_with(&format!(
            "Its picture is in {}:\n  image-{}.png\n",
            picture.parent().unwrap().display(),
            &id[..8]
        )),
        "{said}"
    );
    assert_eq!(
        std::fs::read(&picture).unwrap(),
        std::fs::read(sandbox.work().join("harbour.png")).unwrap()
    );
    let written = std::fs::read_to_string(sandbox.work().join("out/chapter.md")).unwrap();
    assert!(
        written.contains(&format!(
            "![The harbour](<chapter images/image-{}.png>)",
            &id[..8]
        )),
        "{written}"
    );
    // What was exported can be given back: the link is a path beside the file.
    let back = sandbox.json(&["create", "Back", "--file", "out/chapter.md"]);
    assert_eq!(back["warnings"], json!([]));
    assert_eq!(first_id(&sandbox, "Back"), id);
    // A Word document holds the picture; plain text says what it shows.
    sandbox.ok(&["export", "doc:1", "--to", "out/chapter.docx"]);
    let package = std::fs::read(sandbox.work().join("out/chapter.docx")).unwrap();
    let name = b"word/media/image1.png";
    assert!(package.windows(name.len()).any(|window| window == name));
    let text = sandbox.json(&["export", "doc:1", "--to", "out/chapter.txt"]);
    assert_eq!(text["assets"], json!([]));
    assert!(
        std::fs::read_to_string(sandbox.work().join("out/chapter.txt"))
            .unwrap()
            .contains("The harbour\n")
    );
}

#[test]
fn the_server_adds_an_image_from_a_whole_path_and_reads_images_as_lines() {
    let sandbox = library();
    let picture = sandbox.picture("harbour.png", (64, 40), [30, 90, 160]);
    let mut client = sandbox.mcp();
    client.initialize("2025-11-25", "Test Agent");

    let listed = client.request("tools/list", json!({}));
    let tools = listed["result"]["tools"].as_array().unwrap();
    let tool = tools
        .iter()
        .find(|tool| tool["name"] == "add_image")
        .expect("add_image is a tool of the server");
    assert_eq!(tool["annotations"]["readOnlyHint"], false);
    assert_eq!(tool["inputSchema"]["required"], json!(["document", "path"]));
    assert!(
        tools
            .iter()
            .all(|tool| tool["name"] != "list_images" && tool["name"] != "read_image"),
        "no tool was added that reads"
    );

    // The server is given no directory: a path has to be whole.
    let (message, error) =
        client.call_error("add_image", json!({ "document": 1, "path": "harbour.png" }));
    assert_eq!(error["code"], "invalid");
    assert_eq!(message, "give the absolute path of the image file");
    let (said, added) = client.call_ok(
        "add_image",
        json!({
            "document": 1, "path": picture.to_str().unwrap(), "description": "The harbour",
            "width_percent": 40, "align": "left", "position": "before_heading",
            "heading": "The harbour",
        }),
    );
    let id = added["image"]["id"].as_str().unwrap().to_string();
    assert_eq!(
        added["image"],
        json!({
            "id": id, "block": 2, "format": "png", "pixels": [64, 40], "width_percent": 40,
            "align": "left", "description": "The harbour", "missing": false,
        }),
        "the width that was asked for is the width (its own size would be 14 %)"
    );
    assert!(said.starts_with("Added an image to doc:1"), "{said}");
    assert!(
        said.contains("restore_version with version 1 brings it back"),
        "a tool result names tools: {said}"
    );
    // The same text and data as the command.
    let (read, data) = client.call_ok("read_document", json!({ "document": 1 }));
    assert!(read.contains(&line("The harbour", &id)), "{read}");
    assert_eq!(data["images"], json!([added["image"]]));
    assert_eq!(data, sandbox.json(&["get", "doc:1"]));

    // What the arguments cannot be is said before anything is read.
    for (arguments, said) in [
        (
            json!({ "document": 1, "path": picture.to_str().unwrap(), "width_percent": 2 }),
            "width_percent",
        ),
        (
            json!({ "document": 1, "path": picture.to_str().unwrap(), "align": "justify" }),
            "align",
        ),
        (
            json!({ "document": 1, "path": picture.to_str().unwrap(), "images": [] }),
            "images",
        ),
        (json!({ "document": 1 }), "path"),
    ] {
        let (message, error) = client.call_error("add_image", arguments);
        assert_eq!(error["code"], "invalid", "{message}");
        assert!(
            message.starts_with("add_image: ") && message.contains(said),
            "{message}"
        );
    }

    // A file named in Markdown: by its whole path it becomes an image, by a relative
    // one it stays a link and the warning says what to give.
    let (_, created) = client.call_ok(
        "create_document",
        json!({
            "title": "Pictures",
            "markdown": format!("![Whole]({})\n\n![Relative](harbour.png)\n", picture.display()),
        }),
    );
    assert_eq!(
        created["warnings"],
        json!([
            "\u{201c}harbour.png\u{201d} is a relative path, and nothing says what it is relative to: \
             give the absolute path of the image file. It was kept as a link."
        ])
    );
    let (pictures, _) = client.call_ok("read_document", json!({ "document": "Pictures" }));
    assert_eq!(
        pictures,
        format!("{}\n\n[Relative](harbour.png)\n", line("Whole", &id))
    );
    // And an export through the server lists the files of the pictures.
    let out = sandbox.root().join("out/pictures.md");
    let (_, exported) = client.call_ok(
        "export",
        json!({ "document": "Pictures", "path": out.to_str().unwrap() }),
    );
    assert_eq!(exported["assets"].as_array().unwrap().len(), 1);
    client.close();
}
