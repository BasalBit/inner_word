//! The verdict: `prepare` keeps a page whole or refuses it, with every reason of section 3.

use writer_page::{Blocked, CheckReport, LoadReport, PageError, limits, prepare, static_check};

use super::support::*;

fn two_pages() -> writer_page::Bundle {
    page(
        &[
            ("index.html", "<title>Atlas</title><h1>The atlas</h1><p>Shown.</p><a href=\"b.html\">b</a>"),
            ("b.html", "<title>Second</title><p>More.</p>"),
            ("img/a.png", "a"),
        ],
        &[],
    )
}

fn load(path: &str, title: &str, text: &str) -> LoadReport {
    LoadReport {
        path: path.to_string(),
        title: title.to_string(),
        text: text.to_string(),
        finished: true,
        requests: 3,
        ..LoadReport::default()
    }
}

fn report(loads: Vec<LoadReport>) -> CheckReport {
    CheckReport { engine: "WebKit 621.1".to_string(), elapsed_ms: 1234, loads, failure: None }
}

fn whole() -> CheckReport {
    report(vec![load("index.html", "Atlas", "The atlas\nShown."), load("b.html", "Second", "More.")])
}

fn refused(report: &CheckReport) -> (String, Vec<String>) {
    match prepare(two_pages(), report) {
        Err(PageError::Incomplete { name, problems }) => (name, problems.into_iter().map(|p| p.reason).collect()),
        other => panic!("not refused: {other:?}"),
    }
}

#[test]
fn a_whole_page_is_prepared() {
    let prepared = prepare(two_pages(), &whole()).unwrap();
    assert_eq!(prepared.title_found, "Atlas");
    assert_eq!(prepared.title(), "Atlas");
    assert_eq!(prepared.text, "The atlas\nShown.\n\nSecond\nMore.");
    assert_eq!(prepared.entry, "index.html");
    assert_eq!(prepared.checked.engine, "WebKit 621.1");
    assert_eq!(prepared.checked.requests, 6);
    assert_eq!(prepared.checked.elapsed_ms, 1234);
    // Served and original are the same bytes here: each blob once.
    assert_eq!(prepared.blobs.len(), 3);
    assert_eq!(prepared.served.len(), 3);
    assert_eq!(prepared.original.len(), 3);
    assert_eq!(prepared.distinct_bytes(), prepared.blobs.iter().map(|b| b.size).sum::<u64>());
    for blob in &prepared.blobs {
        assert_eq!(blob.file, writer_page::file_id_hex(&blob.data));
        assert_eq!(blob.size, blob.data.len() as u64);
        assert!(!blob.stored);
    }
}

#[test]
fn the_title_falls_back_to_the_heading_then_to_the_name() {
    let mut r = whole();
    r.loads[0].title = "   ".to_string();
    r.loads[0].first_heading = "  The atlas  ".to_string();
    assert_eq!(prepare(two_pages(), &r).unwrap().title(), "The atlas");
    r.loads[0].first_heading.clear();
    let prepared = prepare(two_pages(), &r).unwrap();
    assert_eq!(prepared.title_found, "");
    assert_eq!(prepared.title(), prepared.name);
    r.loads[0].title = "x".repeat(300);
    assert_eq!(prepare(two_pages(), &r).unwrap().title_found.chars().count(), limits::MAX_TITLE_CHARS);
}

#[test]
fn every_reason_of_refusal() {
    let mut r = whole();
    r.loads[0].missing = vec!["audio/18.mp3".to_string()];
    r.loads[0].blocked = vec![
        Blocked { url: "https://api.example.com/grades".to_string(), directive: "connect-src".to_string() },
        Blocked { url: "https://fonts.gstatic.com/x.woff2".to_string(), directive: "font-src".to_string() },
        Blocked { url: "https://example.com/".to_string(), directive: "navigation".to_string() },
    ];
    r.loads[1].unwatched_frames = vec!["a.svg".to_string()];
    r.loads[1].blocked = vec![Blocked { url: "https://cdn.test/x.png".to_string(), directive: "img-src".to_string() }];
    let (name, reasons) = refused(&r);
    assert_eq!(name, "Atlas");
    assert_eq!(
        reasons,
        vec![
            "the page asks for audio/18.mp3, which is not among its files",
            "the page asks for https://api.example.com/grades at run time (data it fetches), which cannot be kept",
            "the page asks for https://fonts.gstatic.com/x.woff2 at run time (a font), which cannot be kept",
            "the page goes to https://example.com/ at run time, which cannot be kept",
            "b.html asks for https://cdn.test/x.png at run time (a picture), which cannot be kept",
            "b.html shows a.svg in a frame, whose requests cannot be watched",
        ]
    );
}

#[test]
fn the_entry_must_load() {
    let mut r = whole();
    r.loads[0].finished = false;
    assert_eq!(refused(&r).1, vec!["the page did not finish loading"]);
    r.loads[0].navigation_error = Some("the file could not be read".to_string());
    assert_eq!(refused(&r).1, vec!["the page failed to load: the file could not be read"]);
    r.loads[0].crashed = true;
    assert_eq!(refused(&r).1, vec!["the page stopped working while it was checked (its web process crashed)"]);
}

#[test]
fn running_out_of_time_before_every_html_file_is_a_refusal() {
    let r = report(vec![load("index.html", "Atlas", "x")]);
    assert_eq!(refused(&r).1, vec!["the page could not be checked in time: 1 of its HTML files was not loaded within 120 s"]);
}

#[test]
fn a_checker_that_failed_is_no_verdict() {
    let failed = CheckReport { engine: String::new(), elapsed_ms: 0, loads: vec![], failure: Some("no window server".to_string()) };
    match prepare(two_pages(), &failed) {
        Err(error @ PageError::CheckFailed { .. }) => {
            assert_eq!(error.sentence(), "the page could not be checked: no window server")
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn script_errors_are_warnings() {
    let mut r = whole();
    r.loads[0].script_errors = (1..=5).map(|i| format!("TypeError {i}")).collect();
    r.loads[1].crashed = true;
    let prepared = prepare(two_pages(), &r).unwrap();
    assert_eq!(
        prepared.warnings,
        vec![
            "a script of index.html failed: TypeError 1",
            "a script of index.html failed: TypeError 2",
            "a script of index.html failed: TypeError 3",
            "and 2 more script errors in index.html",
            "b.html stopped working while it was checked",
        ]
    );
}

#[test]
fn the_text_is_cut_at_its_limit() {
    let mut r = whole();
    r.loads[0].text = "é".repeat(limits::MAX_TEXT_CHARS + 10);
    let prepared = prepare(two_pages(), &r).unwrap();
    assert_eq!(prepared.text.chars().count(), limits::MAX_TEXT_CHARS);
    assert!(prepared.warnings.iter().any(|w| w.contains("2,000,000 characters")), "{:?}", prepared.warnings);
}

#[test]
fn the_bundle_warnings_travel_with_the_page() {
    let bundle = page(&[("index.html", "<script>window.claude.db</script>")], &[]);
    let prepared = prepare(bundle.clone(), &static_check(&bundle)).unwrap();
    assert_eq!(prepared.warnings, bundle.warnings);
}

#[test]
fn a_prepared_page_travels_as_json_with_its_bytes_in_base64() {
    let prepared = prepare(two_pages(), &whole()).unwrap();
    let json = serde_json::to_string(&prepared).unwrap();
    assert!(json.contains("\"data\":\"PHRpdGxl") || json.contains("\"data\":\""), "{json}");
    assert!(!json.contains("\"stored\""), "stored never travels");
    let back: writer_page::Prepared = serde_json::from_str(&json).unwrap();
    assert_eq!(back, prepared);
    // A request that says its blobs are stored is not believed.
    let forged = json.replacen("\"size\"", "\"stored\":true,\"size\"", 1);
    let back: writer_page::Prepared = serde_json::from_str(&forged).unwrap();
    assert!(back.blobs.iter().all(|b| !b.stored));
    // Debug prints sizes, not bytes.
    let debug = format!("{:?}", prepared.blobs[0]);
    assert!(debug.contains("bytes"), "{debug}");
}
