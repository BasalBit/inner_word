//! Pages: library items of kind `page`, whose content is a manifest naming files whose
//! bytes are in `images.db`. Created, saved, found by the address they came from, kept
//! apart from texts both ways, duplicated, trashed and restored, searched, versioned.

mod common;

use common::*;
use innerword_store::{
    DocumentKind, DocumentMeta, FileId, FolderId, OpenOptions, PageContent, Store, StoreError,
};

// ----- helpers -------------------------------------------------------------------------

const ORBITARIUM: &str = "https://claude.ai/artifact/ExampleArtifactId00001";
const OTHER: &str = "https://claude.ai/artifact/other";

/// The manifest of a page that serves these files under these paths, as `innerword-page`
/// writes it (the same file can stand under two paths, as the original and the served
/// copy of an MP3 do).
fn manifest(files: &[(&str, FileId)], text: &str) -> String {
    let entries: Vec<String> = files
        .iter()
        .map(|(path, id)| {
            format!(
                r#"{{"path":"{path}","file":"{id}","type":"application/octet-stream","size":1}}"#
            )
        })
        .collect();
    let entries = entries.join(",");
    format!(
        r#"{{"page":1,"entry":"index.html","files":[{entries}],"original":[{entries}],"externals":[],"source_url":null,"fingerprint":"","added_at":0,"checked":{{"engine":"WebKit 621.1","at":0,"requests":0,"warnings":[]}},"text":{}}}"#,
        serde_json::to_string(text).unwrap()
    )
}

fn page_of<'a>(manifest: &'a str, title: &'a str, text: &'a str) -> PageContent<'a> {
    PageContent {
        manifest,
        title,
        plain_text: text,
        word_count: text.split_whitespace().count() as u64,
        char_count: text.chars().count() as u64,
        source_url: None,
    }
}

/// Stores the files of a page and creates it, with an address or without.
fn add_page(
    store: &mut Store,
    folder: Option<FolderId>,
    title: &str,
    text: &str,
    source_url: Option<&str>,
) -> DocumentMeta {
    let html = store
        .put_file(format!("<h1>{title}</h1>").as_bytes())
        .unwrap()
        .id;
    let font = store.put_file(b"wOF2 a font many pages share").unwrap().id;
    let json = manifest(
        &[("index.html", html), ("_external/font.woff2", font)],
        text,
    );
    store
        .create_page(
            folder,
            &PageContent {
                source_url,
                ..page_of(&json, title, text)
            },
        )
        .unwrap()
}

fn page_ids(store: &Store) -> Vec<i64> {
    store.pages().unwrap().into_iter().map(|p| p.id.0).collect()
}

// ----- creating and saving -------------------------------------------------------------

#[test]
fn a_page_is_an_item_among_the_documents_of_its_folder() {
    let (mut store, _) = store();
    let folder = store.create_folder(None, "Claude Pages").unwrap();
    let text = doc(&mut store, Some(folder.id), "Notes", "Some notes");
    let page = add_page(
        &mut store,
        Some(folder.id),
        "Reading Orbitarium",
        "Orbitarium as built at a3c2306",
        Some(ORBITARIUM),
    );

    assert_eq!(page.kind, DocumentKind::Page);
    assert_eq!(page.source_url.as_deref(), Some(ORBITARIUM));
    assert_eq!(page.title, "Reading Orbitarium");
    assert_eq!((page.word_count, page.char_count), (5, 30));
    assert_eq!(page.folder_id, Some(folder.id));
    assert_eq!(page.created_at, START);
    assert_eq!(page.updated_at, START);
    assert_eq!(page.trashed_at, None);
    assert!(page.position > text.position);
    // A text is a text, as every row was before pages.
    assert_eq!(text.kind, DocumentKind::Text);
    assert_eq!(text.source_url, None);

    // In the folder after the text, in the tree, among the documents and the recent.
    assert_eq!(
        doc_titles(&store, Some(folder.id)),
        ["Notes", "Reading Orbitarium"]
    );
    let tree = store.library_tree().unwrap();
    let node = tree
        .folders
        .iter()
        .find(|f| f.folder.id == folder.id)
        .unwrap();
    assert_eq!(node.documents[1], page);
    assert!(store.documents().unwrap().contains(&page));
    assert_eq!(store.document_meta(page.id).unwrap(), page);

    // Its content is the manifest, as it was handed in; it has no view state.
    let loaded = store.document(page.id).unwrap();
    assert!(
        loaded
            .content
            .starts_with(r#"{"page":1,"entry":"index.html""#)
    );
    assert_eq!(loaded.meta, page);
    assert_eq!(loaded.view_state, None);

    // The list of pages has it and not the text.
    assert_eq!(page_ids(&store), vec![page.id.0]);
}

#[test]
fn a_page_is_created_at_the_top_and_not_in_a_folder_that_is_gone() {
    let (mut store, _) = store();
    let page = add_page(&mut store, None, "At the top", "words", None);
    assert_eq!(page.folder_id, None);
    assert_eq!(doc_titles(&store, None), ["At the top"]);

    let folder = store.create_folder(None, "Gone").unwrap();
    store.trash_folder(folder.id).unwrap();
    let json = manifest(&[], "x");
    for folder in [Some(folder.id), Some(FolderId(999))] {
        assert!(matches!(
            store.create_page(folder, &page_of(&json, "Nowhere", "x")),
            Err(StoreError::NotFound { .. })
        ));
    }
    assert_eq!(page_ids(&store), vec![page.id.0]);
}

/// The last line of defence of "whole or nothing": a manifest that names a file whose
/// bytes the library does not have is refused, and nothing is written.
#[test]
fn a_page_that_names_a_file_the_library_does_not_have_is_not_written() {
    let (mut store, clock) = store();
    let there = store.put_file(b"stored").unwrap().id;
    let lost = FileId::of(b"never stored");
    let also_lost = FileId::of(b"never stored either");
    let json = manifest(
        &[
            ("a.css", there),
            ("b.woff2", lost),
            ("c.mp3", also_lost),
            ("d.mp3", lost),
        ],
        "Text",
    );
    let mut expected = vec![lost, also_lost];
    expected.sort();
    match store.create_page(None, &page_of(&json, "Half", "Text")) {
        Err(StoreError::FilesMissing { ids }) => assert_eq!(ids, expected),
        other => panic!("expected FilesMissing, got {other:?}"),
    }
    assert!(store.documents().unwrap().is_empty());
    assert!(store.pages().unwrap().is_empty());
    assert!(store.search("Text", 10).unwrap().is_empty());

    // The message says how many and names them.
    let message = StoreError::FilesMissing {
        ids: expected.clone(),
    }
    .to_string();
    assert!(message.contains("2 files"), "{message}");
    assert!(message.contains(&lost.to_hex()), "{message}");

    // A save that would name a missing file changes nothing either.
    let whole = add_page(&mut store, None, "Whole", "Whole text", None);
    let before = store.document(whole.id).unwrap();
    clock.advance_ms(MINUTE);
    assert!(matches!(
        store.save_page(whole.id, &page_of(&json, "Half", "Text")),
        Err(StoreError::FilesMissing { .. })
    ));
    assert_eq!(store.document(whole.id).unwrap(), before);

    // Stored, the same page is taken.
    store.put_file(b"never stored").unwrap();
    store.put_file(b"never stored either").unwrap();
    let half = store
        .create_page(None, &page_of(&json, "Now whole", "Text"))
        .unwrap();
    assert_eq!(half.kind, DocumentKind::Page);
}

#[test]
fn a_page_without_files_needs_no_images_file() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("library.db");
    let mut store = Store::open_with(
        &database,
        OpenOptions {
            daily_backup: false,
            ..OpenOptions::default()
        },
    )
    .unwrap();
    let json = manifest(&[], "Nothing but text");
    let page = store
        .create_page(None, &page_of(&json, "Plain", "Nothing but text"))
        .unwrap();
    assert_eq!(page.kind, DocumentKind::Page);
    assert!(!innerword_store::images_path(&database).exists());
    // One that names a file, with no images.db at all, is refused.
    let named = manifest(&[("index.html", FileId::of(b"x"))], "x");
    assert!(matches!(
        store.create_page(None, &page_of(&named, "Named", "x")),
        Err(StoreError::FilesMissing { .. })
    ));
    assert!(!innerword_store::images_path(&database).exists());
}

#[test]
fn a_page_is_saved_with_new_content_and_keeps_its_place() {
    let (mut store, clock) = store();
    let folder = store.create_folder(None, "Pages").unwrap();
    let page = add_page(
        &mut store,
        Some(folder.id),
        "Orbitarium",
        "the old words",
        Some(ORBITARIUM),
    );
    let after_it = doc(&mut store, Some(folder.id), "After", "x");

    clock.advance_ms(HOUR);
    let html = store.put_file(b"<h1>new</h1>").unwrap().id;
    let json = manifest(&[("index.html", html)], "the new words");
    let saved = store
        .save_page(
            page.id,
            &PageContent {
                title: "Orbitarium, updated",
                source_url: Some(OTHER),
                ..page_content(&json, "the new words")
            },
        )
        .unwrap();
    assert_eq!(saved.title, "Orbitarium, updated");
    assert_eq!(saved.updated_at, START + HOUR);
    assert_eq!(saved.created_at, START);
    assert_eq!(
        (saved.folder_id, saved.position),
        (page.folder_id, page.position)
    );
    assert_eq!(saved.source_url.as_deref(), Some(OTHER));
    assert_eq!((saved.word_count, saved.char_count), (3, 13));
    assert_eq!(store.document(page.id).unwrap().content, json);
    assert_eq!(
        doc_titles(&store, Some(folder.id)),
        ["Orbitarium, updated", "After"]
    );
    assert!(store.document_exists(after_it.id).unwrap());

    // The search index follows the text.
    assert!(store.search("old", 10).unwrap().is_empty());
    assert_eq!(store.search("new", 10).unwrap()[0].document_id, page.id);

    // The same title and text again: saved, and found as before.
    clock.advance_ms(HOUR);
    let again = store
        .save_page(
            page.id,
            &PageContent {
                title: "Orbitarium, updated",
                ..page_content(&json, "the new words")
            },
        )
        .unwrap();
    assert_eq!(again.updated_at, START + 2 * HOUR);
    assert_eq!(again.source_url, None);
    assert_eq!(store.search("new", 10).unwrap()[0].document_id, page.id);
}

fn page_content<'a>(manifest: &'a str, text: &'a str) -> PageContent<'a> {
    page_of(manifest, "", text)
}

#[test]
fn a_trashed_page_is_saved_too() {
    let (mut store, _) = store();
    let page = add_page(&mut store, None, "In the trash", "old", None);
    store.trash_document(page.id).unwrap();
    let json = manifest(&[], "new");
    let saved = store
        .save_page(page.id, &page_content(&json, "new"))
        .unwrap();
    assert!(saved.trashed_at.is_some());
    assert_eq!(store.document(page.id).unwrap().content, json);
}

// ----- the kinds kept apart -------------------------------------------------------------

/// A code path that forgets to ask the kind fails loudly: it cannot save the plain-text
/// recovery of a manifest over the page, nor a page over a text.
#[test]
fn a_text_is_never_saved_over_a_page_nor_a_page_over_a_text() {
    let (mut store, _) = store();
    let page = add_page(&mut store, None, "A page", "Its words", None);
    let text = doc(&mut store, None, "A text", "Its words");
    let page_before = store.document(page.id).unwrap();
    let text_before = store.document(text.id).unwrap();

    match store.save_document(page.id, &content("A page", "Its words", r#"{"blocks":[]}"#)) {
        Err(StoreError::WrongKind { id, kind }) => {
            assert_eq!((id, kind), (page.id.0, DocumentKind::Page));
        }
        other => panic!("expected WrongKind, got {other:?}"),
    }
    let json = manifest(&[], "x");
    match store.save_page(text.id, &page_of(&json, "A text", "x")) {
        Err(StoreError::WrongKind { id, kind }) => {
            assert_eq!((id, kind), (text.id.0, DocumentKind::Text));
        }
        other => panic!("expected WrongKind, got {other:?}"),
    }
    assert_eq!(store.document(page.id).unwrap(), page_before);
    assert_eq!(store.document(text.id).unwrap(), text_before);

    // The messages say what the item is.
    let message = StoreError::WrongKind {
        id: page.id.0,
        kind: DocumentKind::Page,
    }
    .to_string();
    assert!(message.contains("is a page"), "{message}");

    // A missing item is missing for both.
    assert!(matches!(
        store.save_page(innerword_store::DocumentId(999), &page_of(&json, "x", "x")),
        Err(StoreError::NotFound { .. })
    ));
    assert!(matches!(
        store.save_document(innerword_store::DocumentId(999), &content("x", "x", "{}")),
        Err(StoreError::NotFound { .. })
    ));
}

#[test]
fn a_kind_is_written_and_read_by_its_name() {
    assert_eq!(DocumentKind::default(), DocumentKind::Text);
    for kind in [DocumentKind::Text, DocumentKind::Page] {
        assert_eq!(DocumentKind::parse(kind.as_str()), Some(kind));
        assert_eq!(
            serde_json::to_string(&kind).unwrap(),
            format!("\"{}\"", kind.as_str())
        );
    }
    assert_eq!(DocumentKind::Page.as_str(), "page");
    assert_eq!(DocumentKind::parse("Page"), None);
    assert_eq!(DocumentKind::parse("video"), None);

    // Metadata written before pages (without the two fields) reads as a text.
    let old = r#"{"id":1,"folder_id":null,"title":"Old","word_count":1,"char_count":3,
        "position":0,"created_at":1,"updated_at":2,"trashed_at":null}"#;
    let meta: DocumentMeta = serde_json::from_str(old).unwrap();
    assert_eq!((meta.kind, meta.source_url), (DocumentKind::Text, None));
}

/// A value of `kind` this build does not know is an error of that row, as an unreadable
/// id is; the other rows read as before.
#[test]
fn an_unknown_kind_is_an_error_of_that_row() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("library.db");
    let mut store = Store::open_with(
        &database,
        OpenOptions {
            daily_backup: false,
            ..OpenOptions::default()
        },
    )
    .unwrap();
    let odd = doc(&mut store, None, "Odd", "x");
    let fine = doc(&mut store, None, "Fine", "y");
    let conn = rusqlite::Connection::open(&database).unwrap();
    conn.execute(
        "UPDATE documents SET kind = 'video' WHERE id = ?1",
        [odd.id.0],
    )
    .unwrap();
    drop(conn);
    let error = store.document_meta(odd.id).unwrap_err().to_string();
    assert!(error.contains("not a kind of library item"), "{error}");
    assert_eq!(store.document_meta(fine.id).unwrap().title, "Fine");
}

// ----- the address ---------------------------------------------------------------------

#[test]
fn a_page_is_found_by_its_address_live_and_in_the_trash() {
    let (mut store, clock) = store();
    assert_eq!(store.page_by_source_url(ORBITARIUM).unwrap(), None);
    assert_eq!(store.trashed_page_by_source_url(ORBITARIUM).unwrap(), None);

    let first = add_page(&mut store, None, "First", "one", Some(ORBITARIUM));
    clock.advance_ms(MINUTE);
    let second = add_page(&mut store, None, "Second", "two", Some(ORBITARIUM));
    clock.advance_ms(MINUTE);
    let other = add_page(&mut store, None, "Other", "three", Some(OTHER));
    add_page(&mut store, None, "Without", "four", None);
    // A text never has an address, whatever its content says.
    doc(&mut store, None, ORBITARIUM, ORBITARIUM);

    // The one updated last; compared as given.
    assert_eq!(
        store.page_by_source_url(ORBITARIUM).unwrap().map(|p| p.id),
        Some(second.id)
    );
    assert_eq!(
        store.page_by_source_url(OTHER).unwrap().map(|p| p.id),
        Some(other.id)
    );
    assert_eq!(
        store
            .page_by_source_url("https://claude.ai/artifact/exampleartifactid00001")
            .unwrap(),
        None
    );
    clock.advance_ms(MINUTE);
    let json = manifest(&[], "one again");
    store
        .save_page(
            first.id,
            &PageContent {
                source_url: Some(ORBITARIUM),
                ..page_of(&json, "First", "one again")
            },
        )
        .unwrap();
    assert_eq!(
        store.page_by_source_url(ORBITARIUM).unwrap().map(|p| p.id),
        Some(first.id)
    );

    // In the trash: the live one is found, and the trashed one only as trashed.
    store.trash_document(first.id).unwrap();
    assert_eq!(
        store.page_by_source_url(ORBITARIUM).unwrap().map(|p| p.id),
        Some(second.id)
    );
    assert_eq!(
        store
            .trashed_page_by_source_url(ORBITARIUM)
            .unwrap()
            .map(|p| p.id),
        Some(first.id)
    );
    // Both in the trash (the second with its folder... at the root: on its own), the one
    // trashed last.
    clock.advance_ms(MINUTE);
    store.trash_document(second.id).unwrap();
    assert_eq!(store.page_by_source_url(ORBITARIUM).unwrap(), None);
    let trashed = store
        .trashed_page_by_source_url(ORBITARIUM)
        .unwrap()
        .unwrap();
    assert_eq!(trashed.id, second.id);
    assert!(trashed.trashed_at.is_some());

    // Restored, it is live again.
    store.restore_document(first.id).unwrap();
    assert_eq!(
        store.page_by_source_url(ORBITARIUM).unwrap().map(|p| p.id),
        Some(first.id)
    );
    assert_eq!(
        store
            .trashed_page_by_source_url(ORBITARIUM)
            .unwrap()
            .map(|p| p.id),
        Some(second.id)
    );
}

#[test]
fn a_page_trashed_with_its_folder_is_found_in_the_trash() {
    let (mut store, _) = store();
    let folder = store.create_folder(None, "Pages").unwrap();
    let page = add_page(
        &mut store,
        Some(folder.id),
        "Inside",
        "words",
        Some(ORBITARIUM),
    );
    store.trash_folder(folder.id).unwrap();
    assert_eq!(store.page_by_source_url(ORBITARIUM).unwrap(), None);
    assert_eq!(
        store
            .trashed_page_by_source_url(ORBITARIUM)
            .unwrap()
            .map(|p| p.id),
        Some(page.id)
    );
    store.restore_folder(folder.id).unwrap();
    assert_eq!(
        store.page_by_source_url(ORBITARIUM).unwrap().map(|p| p.id),
        Some(page.id)
    );
}

// ----- the item operations -------------------------------------------------------------

#[test]
fn a_duplicate_of_a_page_is_a_page_without_an_address() {
    let (mut store, clock) = store();
    let folder = store.create_folder(None, "Pages").unwrap();
    let page = add_page(
        &mut store,
        Some(folder.id),
        "Orbitarium",
        "the words",
        Some(ORBITARIUM),
    );
    let after = doc(&mut store, Some(folder.id), "After", "x");
    clock.advance_ms(MINUTE);
    let copy = store.duplicate_document(page.id).unwrap();
    assert_eq!(copy.kind, DocumentKind::Page);
    assert_eq!(copy.source_url, None);
    assert_eq!(copy.title, "Orbitarium copy");
    assert_eq!(
        (copy.word_count, copy.char_count),
        (page.word_count, page.char_count)
    );
    assert_eq!(copy.created_at, START + MINUTE);
    assert_eq!(
        store.document(copy.id).unwrap().content,
        store.document(page.id).unwrap().content
    );
    assert_eq!(
        doc_titles(&store, Some(folder.id)),
        ["Orbitarium", "Orbitarium copy", "After"]
    );
    assert!(store.document_exists(after.id).unwrap());
    // The address finds the original, never the copy.
    assert_eq!(
        store.page_by_source_url(ORBITARIUM).unwrap().map(|p| p.id),
        Some(page.id)
    );
    // The copy's text is found as the page's is.
    let hits = store.search("words", 10).unwrap();
    assert_eq!(hits.len(), 2);
    // A text is copied as a text.
    let text = doc(&mut store, None, "Text", "x");
    assert_eq!(
        store.duplicate_document(text.id).unwrap().kind,
        DocumentKind::Text
    );
}

#[test]
fn a_folder_that_is_copied_copies_its_pages_as_pages() {
    let (mut store, _) = store();
    let folder = store.create_folder(None, "Pages").unwrap();
    let page = add_page(
        &mut store,
        Some(folder.id),
        "Orbitarium",
        "words",
        Some(ORBITARIUM),
    );
    doc(&mut store, Some(folder.id), "Notes", "x");
    let copy = store.duplicate_folder(folder.id).unwrap();
    let inside = store.documents_in_folder(Some(copy.id)).unwrap();
    assert_eq!(
        inside
            .iter()
            .map(|d| (d.title.as_str(), d.kind))
            .collect::<Vec<_>>(),
        [
            ("Orbitarium", DocumentKind::Page),
            ("Notes", DocumentKind::Text)
        ]
    );
    assert_eq!(inside[0].source_url, None);
    assert_eq!(
        store.document(inside[0].id).unwrap().content,
        store.document(page.id).unwrap().content
    );
    assert_eq!(
        store.page_by_source_url(ORBITARIUM).unwrap().map(|p| p.id),
        Some(page.id)
    );
}

#[test]
fn a_page_goes_to_the_trash_and_comes_back_as_it_was() {
    let (mut store, clock) = store();
    let page = add_page(
        &mut store,
        None,
        "Orbitarium",
        "the words",
        Some(ORBITARIUM),
    );
    clock.advance_ms(MINUTE);
    store.trash_document(page.id).unwrap();
    let listed = store.pages().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].trashed_at, Some(START + MINUTE));
    assert!(store.documents().unwrap().is_empty());
    let entries = store.trash().unwrap();
    assert!(
        matches!(&entries[0], innerword_store::TrashEntry::Document(meta) if meta.kind == DocumentKind::Page)
    );

    let restored = store.restore_document(page.id).unwrap();
    assert_eq!(restored.kind, DocumentKind::Page);
    assert_eq!(restored.source_url.as_deref(), Some(ORBITARIUM));
    assert_eq!(restored.trashed_at, None);
    assert_eq!(store.document(page.id).unwrap().meta, restored);

    // A rename is a rename.
    clock.advance_ms(MINUTE);
    store.rename_document(page.id, "Renamed").unwrap();
    let renamed = store.document_meta(page.id).unwrap();
    assert_eq!(
        (renamed.title.as_str(), renamed.kind),
        ("Renamed", DocumentKind::Page)
    );

    // Deleted for good, it is no page any more; its files stay for the collector.
    store.trash_document(page.id).unwrap();
    store.purge_document(page.id).unwrap();
    assert!(store.pages().unwrap().is_empty());
    assert_eq!(store.trashed_page_by_source_url(ORBITARIUM).unwrap(), None);
    assert_eq!(store.file_count().unwrap(), 2);
}

#[test]
fn the_list_of_pages_is_every_page_most_recently_updated_first() {
    let (mut store, clock) = store();
    let a = add_page(&mut store, None, "A", "a", None);
    clock.advance_ms(MINUTE);
    let b = add_page(&mut store, None, "B", "b", Some(OTHER));
    clock.advance_ms(MINUTE);
    let c = add_page(&mut store, None, "C", "c", None);
    doc(&mut store, None, "A text", "x");
    clock.advance_ms(MINUTE);
    store.trash_document(b.id).unwrap();
    clock.advance_ms(MINUTE);
    let json = manifest(&[], "a again");
    store
        .save_page(a.id, &page_content(&json, "a again"))
        .unwrap();
    assert_eq!(page_ids(&store), vec![a.id.0, c.id.0, b.id.0]);
    let pages = store.pages().unwrap();
    assert!(pages.iter().all(|p| p.kind == DocumentKind::Page));
    assert_eq!(pages[2].trashed_at, Some(START + 3 * MINUTE));
}

// ----- search and versions -------------------------------------------------------------

#[test]
fn search_finds_what_a_page_showed_and_says_it_is_a_page() {
    let (mut store, _) = store();
    let page = add_page(
        &mut store,
        None,
        "Reading Orbitarium",
        "The narration of every chapter, with its seventeen recordings",
        None,
    );
    let text = doc(
        &mut store,
        None,
        "Notes",
        "Seventeen notes about recordings",
    );
    let hits = store.search("recordings", 10).unwrap();
    assert_eq!(hits.len(), 2);
    let page_hit = hits.iter().find(|h| h.document_id == page.id).unwrap();
    assert_eq!(page_hit.kind, DocumentKind::Page);
    assert!(page_hit.snippet.contains("recordings"));
    let text_hit = hits.iter().find(|h| h.document_id == text.id).unwrap();
    assert_eq!(text_hit.kind, DocumentKind::Text);
    // By its title too.
    assert_eq!(
        store.search("orbitarium", 10).unwrap()[0].document_id,
        page.id
    );
}

#[test]
fn a_version_of_a_page_holds_its_manifest_and_is_brought_back_by_a_save() {
    let (mut store, clock) = store();
    let page = add_page(
        &mut store,
        None,
        "Orbitarium",
        "as it was",
        Some(ORBITARIUM),
    );
    let old = store.document(page.id).unwrap().content;
    let old_files: Vec<FileId> = store.referenced_files().unwrap().into_iter().collect();
    clock.advance_ms(MINUTE);
    let version = store
        .create_labeled_snapshot(page.id, "Before an update by Claude")
        .unwrap();
    assert_eq!(version.title, "Orbitarium");
    assert_eq!(
        store.snapshot_label(version.id).unwrap().as_deref(),
        Some("Before an update by Claude")
    );

    // The update names other files only.
    let html = store.put_file(b"<h1>new</h1>").unwrap().id;
    let new = manifest(&[("index.html", html)], "as it is now");
    clock.advance_ms(MINUTE);
    store
        .save_page(
            page.id,
            &PageContent {
                source_url: Some(ORBITARIUM),
                ..page_content(&new, "as it is now")
            },
        )
        .unwrap();
    let snapshot = store.snapshot(version.id).unwrap();
    assert_eq!(snapshot.content, old);
    // The version keeps the files it names in use.
    let named = store.referenced_files().unwrap();
    for id in old_files {
        assert!(named.contains(&id));
    }
    assert!(named.contains(&html));

    // Restoring is saving that manifest again, after keeping the current one.
    store.create_labeled_snapshot(page.id, "").unwrap();
    let restored = store
        .save_page(
            page.id,
            &PageContent {
                title: "Orbitarium",
                source_url: Some(ORBITARIUM),
                ..page_content(&snapshot.content, "as it was")
            },
        )
        .unwrap();
    assert_eq!(restored.kind, DocumentKind::Page);
    assert_eq!(store.document(page.id).unwrap().content, old);
    assert_eq!(store.snapshots(page.id).unwrap().len(), 2);
}

#[test]
fn a_library_on_disk_keeps_its_pages_across_a_reopening() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("library.db");
    let options = || OpenOptions {
        daily_backup: false,
        ..OpenOptions::default()
    };
    let (page, bytes) = {
        let mut store = Store::open_with(&database, options()).unwrap();
        let bytes = b"<h1>Kept</h1>".to_vec();
        let id = store.put_file(&bytes).unwrap().id;
        let json = manifest(&[("index.html", id)], "Kept");
        let page = store
            .create_page(
                None,
                &PageContent {
                    source_url: Some(ORBITARIUM),
                    ..page_of(&json, "Kept", "Kept")
                },
            )
            .unwrap();
        store.close().unwrap();
        (page, bytes)
    };
    let store = Store::open_with(&database, options()).unwrap();
    assert_eq!(store.schema_version().unwrap(), 4);
    assert_eq!(store.document_meta(page.id).unwrap(), page);
    assert_eq!(
        store.page_by_source_url(ORBITARIUM).unwrap().map(|p| p.id),
        Some(page.id)
    );
    assert_eq!(store.file_bytes(&FileId::of(&bytes)).unwrap(), Some(bytes));
    // A tool that only reads sees the page as it is.
    let reader = Store::open_read_only(&database).unwrap().store().unwrap();
    assert_eq!(reader.pages().unwrap(), vec![page.clone()]);
    assert_eq!(reader.file_count().unwrap(), 1);
}
