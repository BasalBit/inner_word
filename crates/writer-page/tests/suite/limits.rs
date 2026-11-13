//! The limits of a page, and pieces that cannot be fetched: refused, with the reason.

use std::fs::File;

use writer_page::{PageError, bundle, limits, read_input};

use super::support::*;

/// A file of `bytes` bytes that takes no room on the disk.
fn sparse(dir: &std::path::Path, path: &str, bytes: u64) {
    let full = dir.join(path);
    std::fs::create_dir_all(full.parent().unwrap()).unwrap();
    File::create(full).unwrap().set_len(bytes).unwrap();
}

#[test]
fn a_hundred_and_two_html_files_are_refused() {
    let dir = folder(&[("index.html", "<p>index</p>")]);
    for i in 0..100 {
        write(dir.path(), &format!("pages/{i}.html"), "<p>x</p>");
    }
    let input = read_input(dir.path(), None).unwrap();
    assert!(bundle(input, &web(&[])).is_ok(), "101 HTML files can be checked");
    write(dir.path(), "pages/one-more.html", "<p>x</p>");
    let error = bundle(read_input(dir.path(), None).unwrap(), &web(&[])).unwrap_err();
    assert_eq!(problems(&error), vec!["the page has 102 HTML pages; at most 101 can be checked"]);
    assert!(error.sentence().ends_with("Nothing was saved."), "{}", error.sentence());
}

#[test]
fn a_file_over_32_mb_is_refused_before_it_is_read() {
    let dir = folder(&[("index.html", "<video src=film.mp4></video>")]);
    sparse(dir.path(), "film.mp4", 33 * 1024 * 1024);
    let error = read_input(dir.path(), None).unwrap_err();
    assert_eq!(problems(&error), vec!["film.mp4 is 33 MB; a file can be 32 MB at most"]);

    let single = folder(&[("page.html", "<video src=film.mp4></video>")]);
    sparse(single.path(), "film.mp4", limits::MAX_FILE_BYTES + 1);
    let error = read_input(&single.path().join("page.html"), None).unwrap_err();
    assert_eq!(problems(&error), vec!["film.mp4 is over 32 MB; a file can be 32 MB at most"]);
}

#[test]
fn a_folder_over_64_mb_or_2000_files_is_refused() {
    let heavy = folder(&[("index.html", "x")]);
    for i in 0..3 {
        sparse(heavy.path(), &format!("media/{i}.bin"), 22 * 1024 * 1024);
    }
    let error = read_input(heavy.path(), None).unwrap_err();
    assert_eq!(problems(&error), vec!["the folder holds more than 64 MB; a page can be 64 MB at most"]);

    let many = folder(&[("index.html", "x")]);
    for i in 0..limits::MAX_FILES {
        write(many.path(), &format!("f/{i}.txt"), "");
    }
    let error = read_input(many.path(), None).unwrap_err();
    assert_eq!(problems(&error), vec!["the folder has more than 2,000 files; a page can have 2,000 at most"]);
}

#[test]
fn more_than_500_pieces_on_the_web_are_refused_before_anything_is_fetched() {
    let html: String = (0..=limits::MAX_EXTERNALS).map(|i| format!("<img src=\"https://cdn.test/{i}.png\">")).collect();
    // An empty web: a fetch would be a 404 among the problems.
    let error = try_page(&[("index.html", &html)], &[]).unwrap_err();
    assert_eq!(problems(&error), vec!["the page names more than 500 pieces on the web; at most 500 can be kept"]);
}

#[test]
fn a_piece_over_32_mb_and_a_page_over_64_mb_are_refused() {
    let big = vec![b'x'; 33 * 1024 * 1024];
    let mut fetcher = web(&[]).with("https://cdn.test/big.bin", "application/octet-stream", big);
    let dir = folder(&[("index.html", r#"<embed src="https://cdn.test/big.bin">"#)]);
    let error = bundle(read_input(dir.path(), None).unwrap(), &fetcher).unwrap_err();
    assert_eq!(problems(&error), vec!["https://cdn.test/big.bin is 33 MB; a file can be 32 MB at most"]);

    for i in 0..3u8 {
        fetcher = fetcher.with(&format!("https://cdn.test/{i}.bin"), "application/octet-stream", vec![i; 22 * 1024 * 1024]);
    }
    let dir = folder(&[(
        "index.html",
        r#"<embed src="https://cdn.test/0.bin"><embed src="https://cdn.test/1.bin"><embed src="https://cdn.test/2.bin">"#,
    )]);
    let error = bundle(read_input(dir.path(), None).unwrap(), &fetcher).unwrap_err();
    assert_eq!(problems(&error), vec!["the page weighs 66 MB; a page can be 64 MB at most"]);
}

#[test]
fn pieces_that_name_pieces_more_than_eight_deep_are_refused() {
    let mut answers: Vec<(String, String)> = Vec::new();
    for level in 0..10 {
        answers.push((format!("https://cdn.test/{level}.css"), format!("@import \"{}.css\";", level + 1)));
    }
    let answers: Vec<(&str, &str, &str)> = answers.iter().map(|(u, b)| (u.as_str(), "text/css", b.as_str())).collect();
    let error = try_page(&[("index.html", r#"<link rel="stylesheet" href="https://cdn.test/0.css">"#)], &answers).unwrap_err();
    let reasons = problems(&error);
    assert_eq!(reasons.len(), 1, "{reasons:?}");
    assert!(reasons[0].contains("more than 8 deep"), "{}", reasons[0]);
    assert!(reasons[0].contains("https://cdn.test/8.css"), "{}", reasons[0]);
}

#[test]
fn a_piece_that_cannot_be_fetched_is_named_and_the_rest_counted() {
    let html: String = (0..7).map(|i| format!("<img src=\"https://cdn.test/{i}.png\">")).collect();
    let error = try_page(&[("index.html", &format!("<title>Seven</title>{html}"))], &[]).unwrap_err();
    let PageError::Incomplete { name, problems } = &error else { panic!("{error:?}") };
    assert_eq!(name, "Seven", "a refusal names the page by its title");
    assert_eq!(problems.len(), 7);
    assert_eq!(problems[0].what, "https://cdn.test/0.png");
    assert_eq!(problems[0].reason, "https://cdn.test/0.png could not be fetched: HTTP 404");
    assert_eq!(
        error.sentence(),
        "“Seven” was not added: it could not be kept whole. https://cdn.test/0.png could not be fetched: HTTP 404; https://cdn.test/1.png could not be fetched: HTTP 404; https://cdn.test/2.png could not be fetched: HTTP 404; https://cdn.test/3.png could not be fetched: HTTP 404; https://cdn.test/4.png could not be fetched: HTTP 404; and 2 more. Nothing was saved."
    );
}
