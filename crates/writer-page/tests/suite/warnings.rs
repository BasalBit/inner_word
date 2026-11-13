//! What is not fetched is said: absolute addresses not kept, claude.ai's own storage.

use std::collections::BTreeMap;
use std::sync::Arc;

use writer_page::{KnownPieces, bundle, read_input};

use super::support::*;

#[test]
fn an_absolute_address_that_is_not_kept_is_said() {
    let bundle = page(
        &[(
            "index.html",
            r#"<div data-endpoint="https://api.example.com/v2/items">x</div>
<form action="https://forms.example.com/submit"></form>
<a href="https://example.com/read-more">a link is not said</a>
<meta property="og:image" content="https://example.com/og.png">
<svg xmlns="http://www.w3.org/2000/svg"></svg>
<script>
fetch("https://api.example.com/grades").then(r => r.json());
const ns = "http://www.w3.org/2000/svg";
const user = `https://api.example.com/users/${id}`;
</script>
<style>.x::after { content: "https://example.com/in-css" }</style>"#,
        )],
        &[],
    );
    assert_eq!(
        bundle.warnings,
        vec![
            "it names https://api.example.com/v2/items, which is not kept: if the page asks for it later (a button), that part will not work",
            "it names https://api.example.com/grades, which is not kept: if the page asks for it later (a button), that part will not work",
            "it names https://api.example.com/users/…, which is not kept: if the page asks for it later (a button), that part will not work",
            "it names https://example.com/in-css, which is not kept: if the page asks for it later (a button), that part will not work",
            "it names https://forms.example.com/submit, which is not kept: if the page asks for it later (a button), that part will not work",
        ]
    );
}

#[test]
fn many_addresses_are_counted_after_ten() {
    let script: String = (0..14).map(|i| format!("fetch('https://api.example.com/{i}');\n")).collect();
    let bundle = page(&[("index.html", &format!("<script>{script}</script>"))], &[]);
    assert_eq!(bundle.warnings.len(), 11);
    assert_eq!(bundle.warnings[10], "it names 4 more addresses on the web that are not kept");
}

#[test]
fn an_address_that_was_fetched_is_not_said() {
    let bundle = page(
        &[("index.html", r#"<script>const lib = "https://cdn.jsdelivr.net/npm/a@1/a.js";</script>"#)],
        &[("https://cdn.jsdelivr.net/npm/a@1/a.js", "text/javascript", "1")],
    );
    assert!(bundle.warnings.is_empty(), "{:?}", bundle.warnings);
}

#[test]
fn claude_storage_is_said() {
    for script in ["const db = window.claude.db;", "await claude.use('db')"] {
        let bundle = page(&[("index.html", &format!("<script>{script}</script>"))], &[]);
        assert_eq!(
            bundle.warnings,
            vec!["it uses claude.ai's storage (window.claude): what it saved there is not in this copy"],
            "{script}"
        );
    }
}

#[test]
fn the_republish_hook_of_claude_ai_keeps_nothing_and_is_not_said() {
    let bundle = page(
        &[("index.html", "<script>const hot = window.claude && window.claude.hot; if (hot) hot.snapshot(() => 1);</script>")],
        &[],
    );
    assert!(bundle.warnings.is_empty(), "{:?}", bundle.warnings);
}

#[test]
fn known_pieces_are_taken_and_not_fetched() {
    let dir = folder(&[(
        "index.html",
        r#"<img src="https://claude.ai/api/assets/abc123"><img src="https://cdn.test/other.png">"#,
    )]);
    let saved = temp();
    write(saved.path(), "asset-abc123.png", "the picture the agent read");
    let mut map = BTreeMap::new();
    map.insert("https://claude.ai/api/assets/abc123".to_string(), saved.path().join("asset-abc123.png"));
    // The inner fetcher knows the other address only: asking it for the asset would be
    // a 404, and a refusal.
    let inner = Arc::new(web(&[("https://cdn.test/other.png", "image/png", "other")]));
    let known = KnownPieces::read(&map, inner).unwrap();
    let bundle = bundle(read_input(dir.path(), None).unwrap(), &known).unwrap();
    let path = stored(&bundle, "https://claude.ai/api/assets/abc123");
    assert_eq!(served(&bundle, &path), "the picture the agent read");
    let file = bundle.served.iter().find(|f| f.path == path).unwrap();
    assert_eq!(file.media_type, "image/png", "the type comes from the file's ending");
    assert_eq!(served(&bundle, &stored(&bundle, "https://cdn.test/other.png")), "other");
}

#[test]
fn a_known_piece_must_be_a_file() {
    let mut map = BTreeMap::new();
    map.insert("https://claude.ai/api/assets/x".to_string(), std::path::PathBuf::from("/nonexistent/writer-page/x.png"));
    assert!(KnownPieces::read(&map, Arc::new(web(&[]))).is_err());
    let mut map = BTreeMap::new();
    map.insert("ftp://example.com/x".to_string(), std::path::PathBuf::from("/tmp"));
    assert!(KnownPieces::read(&map, Arc::new(web(&[]))).is_err());
}
