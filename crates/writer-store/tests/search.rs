mod common;

use common::*;
use writer_store::{MATCH_END, MATCH_START};

#[test]
fn fts5_is_available_and_finds_words() {
    let (mut store, _) = store();
    let f = store.create_folder(None, "f").unwrap();
    let d = doc(
        &mut store,
        Some(f.id),
        "Storm notes",
        "It was a dark and stormy night; the rain fell in torrents.",
    );
    let hits = store.search("rain", 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].document_id, d.id);
    assert_eq!(hits[0].folder_id, Some(f.id));
    assert_eq!(hits[0].title, "Storm notes");
    assert!(
        hits[0]
            .snippet
            .contains(&format!("{MATCH_START}rain{MATCH_END}")),
        "{}",
        hits[0].snippet
    );
    let parts = hits[0].snippet_parts();
    assert!(parts.iter().any(|p| p.is_match && p.text == "rain"));
}

#[test]
fn prefix_matching_on_last_word() {
    let (mut store, _) = store();
    doc(&mut store, None, "a", "the quick brown fox");
    doc(&mut store, None, "b", "quintessential reading");
    assert_eq!(store.search("qu", 10).unwrap().len(), 2);
    assert_eq!(store.search("qui", 10).unwrap().len(), 2);
    assert_eq!(store.search("quic", 10).unwrap().len(), 1);
    // Only the last word is a prefix.
    assert_eq!(store.search("qu brown", 10).unwrap().len(), 0);
    assert_eq!(store.search("quick bro", 10).unwrap().len(), 1);
    // Trailing space ends the word: no prefix.
    assert_eq!(store.search("quic ", 10).unwrap().len(), 0);
}

#[test]
fn title_matches_rank_above_body_matches() {
    let (mut store, clock) = store();
    let body = doc(
        &mut store,
        None,
        "Shopping",
        "remember the lighthouse keeper",
    );
    clock.advance_ms(10);
    let title = doc(&mut store, None, "Lighthouse", "a story by the sea");
    let hits = store.search("lighthouse", 10).unwrap();
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].document_id, title.id);
    assert_eq!(hits[1].document_id, body.id);
    assert!(hits[0].rank > hits[1].rank);
    // Title-only match still has a snippet of the body text, without markers.
    assert!(!hits[0].snippet.contains(MATCH_START));
}

#[test]
fn more_occurrences_rank_higher() {
    let (mut store, _) = store();
    let few = doc(
        &mut store,
        None,
        "x",
        "whale and some other words here to pad it out",
    );
    let many = doc(&mut store, None, "y", "whale whale whale and some words");
    let hits = store.search("whale", 10).unwrap();
    assert_eq!(hits[0].document_id, many.id);
    assert_eq!(hits[1].document_id, few.id);
}

#[test]
fn hostile_queries_never_error() {
    let (mut store, _) = store();
    doc(
        &mut store,
        None,
        "Notes",
        "AND OR NOT near quoted \"stuff\" c++ e-mail",
    );
    let nasty = [
        "\"",
        "\"\"",
        "\"unterminated",
        "'; DROP TABLE documents; --",
        "AND",
        "OR OR OR",
        "NOT",
        "NEAR(a b)",
        "a NEAR/3 b",
        "*",
        "**a",
        "a*b",
        "title:notes",
        "{title plain_text}: x",
        "^start",
        "(",
        ")",
        "a)(",
        "-",
        "--",
        "+",
        "c++",
        "e-mail",
        "\\",
        "\u{0}",
        "\u{E000}\u{E001}",
        "🙂",
        "     ",
        "",
        "%_",
        "\"AND\"",
    ];
    for q in nasty {
        let result = store.search(q, 10);
        assert!(result.is_ok(), "query {q:?} failed: {result:?}");
    }
    assert_eq!(store.search("AND", 10).unwrap().len(), 1);
    assert_eq!(store.search("\"quoted stuff\"", 10).unwrap().len(), 1);
    assert_eq!(store.search("e-mail", 10).unwrap().len(), 1);
    assert_eq!(store.search("title:notes", 10).unwrap().len(), 0);
    assert_eq!(store.search("notes", 10).unwrap().len(), 1);
}

#[test]
fn unicode_and_diacritics() {
    let (mut store, _) = store();
    doc(&mut store, None, "Café", "Une crème brûlée à Paris");
    doc(&mut store, None, "Русский", "Привет мир");
    assert_eq!(store.search("creme brulee", 10).unwrap().len(), 1);
    assert_eq!(store.search("brû", 10).unwrap().len(), 1);
    assert_eq!(store.search("cafe", 10).unwrap().len(), 1);
    assert_eq!(store.search("привет", 10).unwrap().len(), 1);
    assert_eq!(store.search("ПРИВ", 10).unwrap().len(), 1);
}

#[test]
fn index_follows_saves_renames_and_deletes() {
    let (mut store, _) = store();
    let d = doc(&mut store, None, "First", "alpha");
    store
        .save_document(d.id, &content("Second", "beta", "{}"))
        .unwrap();
    assert!(store.search("alpha", 10).unwrap().is_empty());
    assert!(store.search("First", 10).unwrap().is_empty());
    assert_eq!(store.search("beta", 10).unwrap().len(), 1);
    store.rename_document(d.id, "Gamma").unwrap();
    assert_eq!(store.search("gamma", 10).unwrap().len(), 1);
    assert!(store.search("second", 10).unwrap().is_empty());
    store.trash_document(d.id).unwrap();
    store.purge_document(d.id).unwrap();
    assert!(store.search("beta", 10).unwrap().is_empty());
    store.rebuild_search_index().unwrap();
    assert!(store.search("beta", 10).unwrap().is_empty());
}

#[test]
fn limit_is_respected() {
    let (mut store, _) = store();
    for i in 0..20 {
        doc(&mut store, None, &format!("d{i}"), "common word");
    }
    assert_eq!(store.search("common", 5).unwrap().len(), 5);
    assert_eq!(store.search("common", 0).unwrap().len(), 0);
}
