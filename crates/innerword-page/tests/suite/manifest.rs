//! The manifest (the content of a page row), the normal form of a source address, the
//! check directory, the fake check and the export of a page as a folder.

use std::collections::HashMap;
use std::fs;

use innerword_page::{
    Manifest, PageError, Which, check_request, file_id_hex, media_type, normal_source_url, prepare,
    read_check_dir, static_check, write_check_dir, write_folder, write_served_dir,
};

use super::support::*;

fn orbitarium() -> innerword_page::Bundle {
    page(
        &[
            (
                "index.html",
                r#"<html><head><title>Orbitarium</title><link rel="stylesheet" href="https://fonts.test/css?family=A"></head><body><h1>Orbitarium</h1><p>A map of a repository.</p><img src="img/map.png"><a href="notes.html">notes</a></body></html>"#,
            ),
            ("notes.html", "<title>Notes</title><p>Notes.</p>"),
            ("img/map.png", "PNGDATA"),
        ],
        &[
            (
                "https://fonts.test/css?family=A",
                "text/css; charset=utf-8",
                "@font-face{src:url(https://fonts.test/a.woff2)}",
            ),
            ("https://fonts.test/a.woff2", "font/woff2", "WOFF2"),
        ],
    )
}

fn manifest_of(bundle: &innerword_page::Bundle) -> (Manifest, HashMap<String, Vec<u8>>) {
    let prepared = prepare(bundle.clone(), &static_check(bundle)).unwrap();
    let blobs: HashMap<String, Vec<u8>> = prepared
        .blobs
        .iter()
        .map(|b| (b.file.clone(), b.data.clone()))
        .collect();
    (
        prepared.manifest(Some("https://claude.ai/artifact/abc"), 1_794_306_980_000),
        blobs,
    )
}

#[test]
fn the_manifest_round_trips_and_names_every_blob_under_file() {
    let bundle = orbitarium();
    let (manifest, blobs) = manifest_of(&bundle);
    let json = manifest.to_json();
    assert!(
        json.starts_with(r#"{"page":1,"entry":"index.html","files":[{"path":"#),
        "{json}"
    );
    for id in manifest.file_ids() {
        assert!(
            json.contains(&format!(r#""file":"{id}""#)),
            "the store's scan finds {id}"
        );
        assert!(blobs.contains_key(&id));
    }
    assert_eq!(Manifest::from_json(&json).unwrap(), manifest);
    assert_eq!(
        manifest.source_url.as_deref(),
        Some("https://claude.ai/artifact/abc")
    );
    assert_eq!(manifest.added_at, 1_794_306_980_000);
    assert_eq!(manifest.checked.at, 1_794_306_980_000);
    assert_eq!(
        manifest.text,
        "Orbitarium\nA map of a repository.\nnotes\n\nNotes\nNotes."
    );
    assert_eq!(manifest.fingerprint, bundle.fingerprint);
    // Served: the rewritten index.html, the image, the notes, the two pieces.
    let served: Vec<&str> = manifest.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(
        served,
        vec![
            "_external/fonts.test/a.woff2",
            &stored(&bundle, "https://fonts.test/css?family=A"),
            "img/map.png",
            "index.html",
            "notes.html",
        ]
    );
    let original: Vec<&str> = manifest.original.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(original, vec!["img/map.png", "index.html", "notes.html"]);
    // The picture is one blob, served and original.
    let picture = manifest.served("img/map.png").unwrap();
    assert_eq!(picture.file, manifest.original[0].file);
    assert_eq!(picture.media_type, "image/png");
    assert_eq!(picture.size, 7);
    assert_ne!(
        manifest.served("index.html").unwrap().file,
        manifest.original[1].file,
        "index.html was rewritten"
    );
    let distinct: u64 = blobs.values().map(|b| b.len() as u64).sum();
    assert_eq!(manifest.bytes(), distinct);
}

#[test]
fn a_served_file_is_found_as_an_address_names_it() {
    let bundle = page(
        &[
            ("index.html", "x"),
            ("my pictures/café.png", "c"),
            ("docs/index.html", "d"),
        ],
        &[],
    );
    let (manifest, _) = manifest_of(&bundle);
    assert_eq!(
        manifest.served("my pictures/café.png").unwrap().path,
        "my pictures/café.png"
    );
    assert_eq!(
        manifest.served("my%20pictures/caf%C3%A9.png").unwrap().path,
        "my pictures/café.png"
    );
    assert_eq!(
        manifest.served("/index.html?x=1#top").unwrap().path,
        "index.html"
    );
    assert_eq!(manifest.served("docs/").unwrap().path, "docs/index.html");
    assert!(manifest.served("nope.png").is_none());
}

#[test]
fn stored_content_that_is_no_manifest_is_unreadable() {
    for text in [
        "",
        "{}",
        r#"{"blocks":[]}"#,
        r#"{"page":9,"entry":"index.html","files":[],"original":[],"externals":[],"source_url":null,"fingerprint":"","added_at":0,"checked":{"engine":"","at":0,"requests":0,"warnings":[]},"text":""}"#,
    ] {
        assert!(
            matches!(Manifest::from_json(text), Err(PageError::Unreadable { .. })),
            "{text}"
        );
    }
    let newer = Manifest::from_json(r#"{"page":9,"entry":"index.html","files":[],"original":[],"externals":[],"source_url":null,"fingerprint":"","added_at":0,"checked":{"engine":"","at":0,"requests":0,"warnings":[]},"text":""}"#)
        .unwrap_err();
    assert!(
        newer.sentence().contains("newer version"),
        "{}",
        newer.sentence()
    );
}

#[test]
fn normal_source_addresses() {
    let cases = [
        (
            "https://claude.ai/artifact/ExampleArtifactId00001",
            "https://claude.ai/artifact/ExampleArtifactId00001",
        ),
        (
            "HTTPS://Claude.AI/artifact/ExampleArtifactId00001/",
            "https://claude.ai/artifact/ExampleArtifactId00001",
        ),
        (
            "https://claude.ai:443/artifact/x#section",
            "https://claude.ai/artifact/x",
        ),
        (
            "http://example.com:80/a/?q=1&b=2#f",
            "http://example.com/a?q=1&b=2",
        ),
        ("http://example.com:8080/", "http://example.com:8080"),
        ("  https://example.com  ", "https://example.com"),
        ("https://example.com/A/B", "https://example.com/A/B"),
    ];
    for (given, normal) in cases {
        assert_eq!(normal_source_url(given).unwrap(), normal, "{given}");
        assert_eq!(
            normal_source_url(normal).unwrap(),
            normal,
            "the normal form of a normal form is itself"
        );
    }
    for bad in [
        "ftp://example.com/x",
        "claude.ai/artifact/x",
        "file:///Users/x/page.html",
        "mailto:a@b.c",
        "",
    ] {
        let error = normal_source_url(bad).unwrap_err();
        assert_eq!(
            error.sentence(),
            "the source address must be an http or https address",
            "{bad}"
        );
    }
}

#[test]
fn the_check_directory_holds_the_request_and_the_served_files() {
    let bundle = orbitarium();
    let dir = temp();
    let check = dir.path().join("check-1");
    write_check_dir(&bundle, &check).unwrap();
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        fs::metadata(&check).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let (request, files) = read_check_dir(&check).unwrap();
    assert_eq!(request, check_request(&bundle));
    assert_eq!(request.entry, "index.html");
    assert_eq!(request.also, vec!["notes.html"]);
    assert_eq!(request.settle_min_ms, 1_000);
    assert_eq!(request.settle_max_ms, 3_000);
    assert_eq!(request.limit_ms, 120_000);
    assert_eq!(files.len(), bundle.served.len());
    for file in &bundle.served {
        let (media, bytes) = &files[&file.path];
        assert_eq!(media, &file.media_type);
        assert_eq!(&bytes[..], &file.bytes[..]);
    }
}

#[test]
fn a_stored_page_writes_the_same_layout_for_a_pdf() {
    let bundle = orbitarium();
    let (manifest, blobs) = manifest_of(&bundle);
    let dir = temp();
    let out = dir.path().join("pdf");
    write_served_dir(&manifest, &out, &mut |id| Ok(blobs.get(id).cloned())).unwrap();
    let (request, files) = read_check_dir(&out).unwrap();
    assert_eq!(request.entry, "index.html");
    assert_eq!(request.also, vec!["notes.html"]);
    assert_eq!(files.len(), manifest.files.len());
    // A file the library does not have is left out.
    let missing = manifest.served("img/map.png").unwrap().file.clone();
    let out = dir.path().join("pdf-missing");
    write_served_dir(&manifest, &out, &mut |id| {
        Ok(if id == missing {
            None
        } else {
            blobs.get(id).cloned()
        })
    })
    .unwrap();
    let (_, files) = read_check_dir(&out).unwrap();
    assert!(!files.contains_key("img/map.png"));
}

#[test]
fn the_fake_check_reads_the_text_without_scripts_styles_and_templates() {
    let bundle = page(
        &[(
            "index.html",
            r#"<!doctype html><html><head><title> The &amp; Title </title><style>body { color: red }</style>
<script>document.write("from a script")</script></head>
<body><h1>Heading <em>one</em></h1><p>First&nbsp;paragraph.</p>
<template><p>in a template</p></template><noscript>no script</noscript>
<ul><li>one</li><li>two</li></ul><textarea>typed text</textarea></body></html>"#,
        )],
        &[],
    );
    let report = static_check(&bundle);
    assert_eq!(report.engine, innerword_page::STATIC_ENGINE);
    assert_eq!(report.loads.len(), 1);
    let load = &report.loads[0];
    assert_eq!(load.title, "The & Title");
    assert_eq!(load.first_heading, "Heading one");
    assert_eq!(
        load.text,
        "Heading one\nFirst\u{a0}paragraph.\none\ntwo\ntyped text"
    );
    assert!(!load.text.contains("from a script"));
    assert!(!load.text.contains("red"));
    assert!(load.finished && load.blocked.is_empty() && load.missing.is_empty());
}

#[test]
fn a_page_is_written_as_a_folder_both_ways() {
    let bundle = orbitarium();
    let (manifest, blobs) = manifest_of(&bundle);
    let dir = temp();
    let served_dir = dir.path().join("Orbitarium");
    let written = write_folder(&manifest, Which::Served, &served_dir, false, &mut |id| {
        Ok(blobs.get(id).cloned())
    })
    .unwrap();
    assert_eq!(written.files, manifest.files.len());
    assert!(written.missing.is_empty());
    for file in &manifest.files {
        let bytes = fs::read(served_dir.join(&file.path)).unwrap();
        assert_eq!(file_id_hex(&bytes), file.file);
    }
    let index = fs::read_to_string(served_dir.join("index.html")).unwrap();
    assert!(
        index.contains("_external/fonts.test/"),
        "the served page names its stored pieces"
    );

    let original_dir = dir.path().join("original");
    let written = write_folder(
        &manifest,
        Which::Original,
        &original_dir,
        false,
        &mut |id| Ok(blobs.get(id).cloned()),
    )
    .unwrap();
    assert_eq!(written.files, 3);
    let index = fs::read_to_string(original_dir.join("index.html")).unwrap();
    assert!(
        index.contains("https://fonts.test/css?family=A"),
        "the original as it was added"
    );

    // A folder that holds the files already: refused without overwrite, nothing written.
    let again = write_folder(&manifest, Which::Served, &served_dir, false, &mut |_| {
        panic!("nothing is read")
    });
    match again {
        Err(PageError::Io { error, .. }) => {
            assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists)
        }
        other => panic!("{other:?}"),
    }
    let written = write_folder(&manifest, Which::Served, &served_dir, true, &mut |id| {
        Ok(blobs.get(id).cloned())
    })
    .unwrap();
    assert_eq!(written.files, manifest.files.len());

    // Bytes the library does not have are said, not written.
    let png = manifest
        .files
        .iter()
        .find(|f| f.path == "img/map.png")
        .unwrap()
        .file
        .clone();
    let missing_dir = dir.path().join("missing");
    let written = write_folder(&manifest, Which::Served, &missing_dir, false, &mut |id| {
        Ok(if id == png {
            None
        } else {
            blobs.get(id).cloned()
        })
    })
    .unwrap();
    assert_eq!(written.files, manifest.files.len() - 1);
    assert_eq!(written.missing, vec!["img/map.png"]);
    assert!(!missing_dir.join("img/map.png").exists());
}

#[test]
fn a_page_whose_files_cannot_be_read_is_not_written_at_all() {
    let bundle = orbitarium();
    let (manifest, blobs) = manifest_of(&bundle);
    let dir = temp();

    // None of its files (a damaged file of the library set aside): nothing, no folder.
    let none = dir.path().join("none");
    let error =
        write_folder(&manifest, Which::Served, &none, false, &mut |_| Ok(None)).unwrap_err();
    assert!(matches!(error, PageError::Unreadable { .. }), "{error:?}");
    assert!(
        error
            .sentence()
            .contains("does not have its entry index.html, so nothing was written"),
        "{}",
        error.sentence()
    );
    assert!(!none.exists(), "no folder is made");
    let original =
        write_folder(&manifest, Which::Original, &none, false, &mut |_| Ok(None)).unwrap_err();
    assert!(
        original.sentence().contains("nothing was written"),
        "{}",
        original.sentence()
    );
    assert!(!none.exists());

    // Its entry missing: what is written would not open.
    let entry = manifest
        .files
        .iter()
        .find(|f| f.path == manifest.entry)
        .unwrap()
        .file
        .clone();
    let no_entry = dir.path().join("no-entry");
    let error = write_folder(&manifest, Which::Served, &no_entry, false, &mut |id| {
        Ok(if id == entry {
            None
        } else {
            blobs.get(id).cloned()
        })
    })
    .unwrap_err();
    assert!(
        error.sentence().contains("entry index.html"),
        "{}",
        error.sentence()
    );
    assert!(!no_entry.exists());

    // A read that fails after others succeeded (a library of a newer version, busy):
    // nothing written, and the sentence does not blame the folder written to.
    let failing = dir.path().join("deep/failing");
    let mut reads = 0;
    let error = write_folder(&manifest, Which::Served, &failing, false, &mut |id| {
        reads += 1;
        if reads == manifest.files.len() {
            Err(std::io::Error::other(
                "this library was created by a newer version of Inner Word",
            ))
        } else {
            Ok(blobs.get(id).cloned())
        }
    })
    .unwrap_err();
    let sentence = error.sentence();
    assert!(sentence.contains("could not be read from the library (this library was created by a newer version of Inner Word), so nothing was written"), "{sentence}");
    assert!(
        !sentence.contains(&failing.display().to_string()),
        "{sentence}"
    );
    assert!(!dir.path().join("deep").exists(), "no folder is made");

    // A write that fails half way takes back what it made.
    let half = dir.path().join("half");
    fs::create_dir_all(half.join("img/map.png")).unwrap(); // a folder where a file goes
    let error = write_folder(&manifest, Which::Served, &half, true, &mut |id| {
        Ok(blobs.get(id).cloned())
    })
    .unwrap_err();
    assert!(matches!(error, PageError::Io { .. }), "{error:?}");
    let left: Vec<String> = walk(&half);
    assert_eq!(left, vec!["img/map.png/"], "only what was there before");
}

fn walk(dir: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![(dir.to_path_buf(), String::new())];
    while let Some((at, prefix)) = stack.pop() {
        let mut entries: Vec<_> = fs::read_dir(&at).unwrap().map(|e| e.unwrap()).collect();
        entries.sort_by_key(|e| e.file_name());
        let mut empty = true;
        for entry in entries {
            empty = false;
            let name = format!("{prefix}{}", entry.file_name().to_string_lossy());
            if entry.file_type().unwrap().is_dir() {
                stack.push((entry.path(), format!("{name}/")));
            } else {
                out.push(name);
            }
        }
        if empty && !prefix.is_empty() {
            out.push(prefix);
        }
    }
    out.sort();
    out
}

#[test]
fn media_types_by_ending() {
    for (path, media) in [
        ("index.html", "text/html"),
        ("a/b.HTM", "text/html"),
        ("x.css", "text/css"),
        ("m.mjs", "text/javascript"),
        ("f.woff2", "font/woff2"),
        ("s.mp3", "audio/mpeg"),
        ("p.svg", "image/svg+xml"),
        ("noending", "application/octet-stream"),
        (".hidden", "application/octet-stream"),
    ] {
        assert_eq!(media_type(path), media, "{path}");
    }
    assert_eq!(file_id_hex(b"").len(), 32);
    assert_eq!(file_id_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223");
}
