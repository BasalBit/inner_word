//! Inputs: a single file and what its references name (its folder never walked), a folder
//! taken whole, the entry, the fingerprint.

use std::fs;

use writer_page::{PageError, bundle, read_input};

use super::support::*;

fn input_paths(input: &writer_page::Input) -> Vec<String> {
    input.files.iter().map(|f| f.path.clone()).collect()
}

#[test]
fn a_single_file_takes_what_it_names_and_never_walks_its_folder() {
    let dir = folder(&[
        (
            "explainer.html",
            r#"<link rel="stylesheet" href="css/site.css"><img src="img/a.png"><a href="more.html">more</a>
<script>const notes = "notes.txt"; const intro = "audio/01-intro.mp3"; const tracks = ["audio/02.mp3", "audio/03.mp3"];</script>"#,
        ),
        ("css/site.css", "body { background: url(../img/bg.png) }"),
        ("img/a.png", "a"),
        ("img/bg.png", "bg"),
        ("more.html", r#"<img src="img/more.png">"#),
        ("img/more.png", "more"),
        ("notes.txt", "private notes"),
        ("audio/01-intro.mp3", "mp3"),
        ("audio/02.mp3", "mp3"),
        ("audio/03.mp3", "mp3"),
        ("unrelated.pdf", "%PDF"),
        ("taxes/2025.csv", "1,2,3"),
    ]);
    let input = read_input(&dir.path().join("explainer.html"), None).unwrap();
    assert!(input.single_file);
    assert_eq!(input.name, "explainer");
    assert_eq!(input.entry, "explainer.html");
    assert_eq!(
        input_paths(&input),
        vec!["css/site.css", "explainer.html", "img/a.png", "img/bg.png", "img/more.png", "more.html"]
    );
    // notes.txt is named in a string and has the ending of a file a page could load, so
    // it is said, as the audio files are; none of them is taken.
    assert_eq!(
        input.warnings,
        vec!["explainer.html names audio/01-intro.mp3 and 3 more files beside it that it may load later; they were not taken. To keep them, add the folder."]
    );
}

#[test]
fn one_file_beside_is_said_in_the_singular() {
    let dir = folder(&[("page.html", r#"<script>new Audio("narration.mp3")</script>"#), ("narration.mp3", "x")]);
    let input = read_input(&dir.path().join("page.html"), None).unwrap();
    assert_eq!(
        input.warnings,
        vec!["page.html names narration.mp3 beside it, which it may load later; it was not taken. To keep it, add the folder."]
    );
}

#[test]
fn the_same_page_as_a_folder_takes_everything() {
    let dir = folder(&[
        ("index.html", r#"<script>const intro = "audio/01-intro.mp3";</script>"#),
        ("audio/01-intro.mp3", "mp3"),
        ("notes.txt", "notes"),
    ]);
    let input = read_input(dir.path(), None).unwrap();
    assert!(!input.single_file);
    assert_eq!(input_paths(&input), vec!["audio/01-intro.mp3", "index.html", "notes.txt"]);
    assert!(input.warnings.is_empty());
}

#[test]
fn a_folder_leaves_out_dot_names_packages_and_links() {
    let dir = folder(&[
        ("index.html", "<p>x</p>"),
        (".DS_Store", "x"),
        (".git/config", "x"),
        ("__MACOSX/index.html", "x"),
        ("Thumbs.db", "x"),
        ("Tool.app/Contents/Info.plist", "x"),
        ("Photos Library.photoslibrary/db", "x"),
        ("img/.hidden.png", "x"),
        ("img/shown.png", "x"),
        ("notes.v1/readme.txt", "a folder with a dot in its name is no package"),
    ]);
    std::os::unix::fs::symlink(dir.path().join("img/shown.png"), dir.path().join("link.png")).unwrap();
    let input = read_input(dir.path(), None).unwrap();
    assert_eq!(input_paths(&input), vec!["img/shown.png", "index.html", "notes.v1/readme.txt"]);
    assert_eq!(input.warnings, vec!["the folder has a link to another file or folder, which was left out"]);
}

#[test]
fn the_entry_of_a_folder() {
    let only = folder(&[("report.html", "x"), ("sub/index.html", "y")]);
    assert_eq!(read_input(only.path(), None).unwrap().entry, "report.html", "the one HTML file at the top");

    let index = folder(&[("a.html", "x"), ("index.html", "y"), ("b.htm", "z")]);
    assert_eq!(read_input(index.path(), None).unwrap().entry, "index.html");

    let htm = folder(&[("a.html", "x"), ("index.htm", "y")]);
    assert_eq!(read_input(htm.path(), None).unwrap().entry, "index.htm");

    let given = folder(&[("index.html", "x"), ("offline/index.html", "y")]);
    assert_eq!(read_input(given.path(), Some("offline/index.html")).unwrap().entry, "offline/index.html");
    assert_eq!(read_input(given.path(), Some("./offline/index.html")).unwrap().entry, "offline/index.html");

    let several = folder(&[("a.html", "x"), ("b.html", "y"), ("c.html", "z")]);
    let error = read_input(several.path(), None).unwrap_err();
    assert_eq!(
        error.sentence(),
        "the folder has 3 pages at its top and no index.html: say which one opens with --entry"
    );

    let missing = read_input(given.path(), Some("nope.html")).unwrap_err();
    assert_eq!(missing.sentence(), "the folder has no file nope.html");

    let none = folder(&[("a.txt", "x")]);
    assert!(matches!(read_input(none.path(), None), Err(PageError::Input { .. })));
}

#[test]
fn a_reference_that_leaves_the_folder_of_a_single_file_is_refused() {
    let dir = folder(&[("site/index.html", r#"<link rel="stylesheet" href="../vendor/a.css">"#), ("vendor/a.css", "a{}")]);
    let error = read_input(&dir.path().join("site/index.html"), None).unwrap_err();
    assert_eq!(
        error.sentence(),
        "index.html names ../vendor/a.css, outside its folder: add the folder that holds both and say which page opens with --entry."
    );
    // The folder that holds both, with the entry named, is a page.
    let input = read_input(dir.path(), Some("site/index.html")).unwrap();
    let bundle = bundle(input, &web(&[])).unwrap();
    assert_eq!(paths(&bundle), vec!["site/index.html", "vendor/a.css"]);
}

#[test]
fn an_address_is_no_input() {
    for address in ["https://claude.ai/artifact/abc", "http://example.com/page.html"] {
        let error = read_input(std::path::Path::new(address), None).unwrap_err();
        assert_eq!(
            error.sentence(),
            "a page is added from a file or a folder on this Mac; fetching a live site is not supported. Save the page first (the skill says how)"
        );
    }
    let dir = folder(&[("notes.txt", "x")]);
    assert!(matches!(read_input(&dir.path().join("notes.txt"), None), Err(PageError::Input { .. })));
    assert!(matches!(read_input(&dir.path().join("gone.html"), None), Err(PageError::Input { .. })));
}

#[test]
fn the_fingerprint_is_stable_and_sensitive() {
    let files = [("index.html", "<p>one</p>"), ("img/a.png", "a")];
    let a = folder(&files);
    let b = folder(&files);
    let first = read_input(a.path(), None).unwrap().fingerprint;
    assert_eq!(first.len(), 64);
    assert_eq!(read_input(a.path(), None).unwrap().fingerprint, first, "the same input, the same fingerprint");
    assert_eq!(read_input(b.path(), None).unwrap().fingerprint, first, "the folder's name and place do not count");

    write(b.path(), "img/a.png", "b");
    assert_ne!(read_input(b.path(), None).unwrap().fingerprint, first, "a byte of a file");
    let c = folder(&[("index.html", "<p>one</p>"), ("img/b.png", "a")]);
    assert_ne!(read_input(c.path(), None).unwrap().fingerprint, first, "the path of a file");
    let d = folder(&[("index.html", "<p>one</p>"), ("img/a.png", "a"), ("x.css", "")]);
    assert_ne!(read_input(d.path(), None).unwrap().fingerprint, first, "a file more");
}

#[test]
fn a_single_file_under_two_names_has_one_fingerprint() {
    let dir = folder(&[("Orbitarium.html", "<h1>Orbitarium</h1>"), ("orbitarium (1).html", "<h1>Orbitarium</h1>")]);
    let one = read_input(&dir.path().join("Orbitarium.html"), None).unwrap();
    let two = read_input(&dir.path().join("orbitarium (1).html"), None).unwrap();
    assert_eq!(one.fingerprint, two.fingerprint);
    assert_eq!(one.name, "Orbitarium");
    assert_eq!(two.name, "orbitarium (1)");
    // And as index.html in a folder of its own.
    let folder_of_its_own = folder(&[("index.html", "<h1>Orbitarium</h1>")]);
    assert_eq!(read_input(folder_of_its_own.path(), None).unwrap().fingerprint, one.fingerprint);
}

#[test]
fn a_folder_is_read_as_bytes() {
    let dir = folder(&[]);
    write(dir.path(), "index.html", b"\xff\xfe<\x00p\x00>\x00");
    let input = read_input(dir.path(), None).unwrap();
    assert_eq!(&input.files[0].bytes[..], b"\xff\xfe<\x00p\x00>\x00");
    let _ = fs::metadata(dir.path()).unwrap();
}
