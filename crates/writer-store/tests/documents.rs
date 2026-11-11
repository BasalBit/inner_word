mod common;

use common::*;
use writer_store::{DocumentContent, DocumentId, StoreError};

#[test]
fn create_load_and_save() {
    let (mut store, clock) = store();
    let f = store.create_folder(None, "Essays").unwrap();
    let json = r#"{"blocks":[{"text":"Grüße aus Köln 👋"}]}"#;
    let meta = store
        .create_document(Some(f.id), &content("Grüße", "Grüße aus Köln 👋", json))
        .unwrap();
    assert_eq!(meta.folder_id, Some(f.id));
    assert_eq!(meta.word_count, 4);
    assert_eq!(meta.char_count, 16);
    assert_eq!(meta.created_at, START);
    let loaded = store.document(meta.id).unwrap();
    assert_eq!(loaded.content, json);
    assert_eq!(loaded.view_state, None);
    assert_eq!(loaded.meta, meta);

    clock.advance_ms(1_000);
    let saved = store
        .save_document(
            meta.id,
            &DocumentContent {
                content: "{\"v\":2}",
                title: "Новый заголовок",
                plain_text: "один два три",
                word_count: 3,
                char_count: 12,
            },
        )
        .unwrap();
    assert_eq!(saved.title, "Новый заголовок");
    assert_eq!(saved.updated_at, START + 1_000);
    assert_eq!(saved.created_at, START);
    let loaded = store.document(meta.id).unwrap();
    assert_eq!(loaded.content, "{\"v\":2}");
    assert_eq!(loaded.meta.word_count, 3);
}

#[test]
fn missing_documents_are_not_found() {
    let (mut store, _) = store();
    let id = DocumentId(77);
    assert!(matches!(
        store.document(id),
        Err(StoreError::NotFound { .. })
    ));
    assert!(matches!(
        store.save_document(id, &content("t", "x", "{}")),
        Err(StoreError::NotFound { .. })
    ));
    assert!(matches!(
        store.save_view_state(id, "{}"),
        Err(StoreError::NotFound { .. })
    ));
    assert!(matches!(
        store.rename_document(id, "x"),
        Err(StoreError::NotFound { .. })
    ));
    assert!(!store.document_exists(id).unwrap());
}

#[test]
fn five_megabyte_document_round_trips() {
    let (mut store, _) = store();
    let paragraph = "Lorem ipsum dolor sit amet, ünïcødé 漢字 text. ";
    let mut text = String::new();
    while text.len() < 5 * 1024 * 1024 {
        text.push_str(paragraph);
    }
    let json = serde_json::to_string(&serde_json::json!({ "text": text })).unwrap();
    let meta = store
        .create_document(None, &content("Big", &text, &json))
        .unwrap();
    let loaded = store.document(meta.id).unwrap();
    assert_eq!(loaded.content.len(), json.len());
    assert_eq!(loaded.content, json);
    let hits = store.search("ünïcødé", 5).unwrap();
    assert_eq!(hits.len(), 1);
    // Saving it again works too.
    store
        .save_document(meta.id, &content("Big 2", &text, &json))
        .unwrap();
    assert_eq!(store.document_meta(meta.id).unwrap().title, "Big 2");
}

#[test]
fn view_state_does_not_bump_updated_at() {
    let (mut store, clock) = store();
    let d = doc(&mut store, None, "a", "text");
    clock.advance_ms(60_000);
    store
        .save_view_state(d.id, r#"{"caret":12,"scroll":340.5}"#)
        .unwrap();
    assert_eq!(
        store.view_state(d.id).unwrap().as_deref(),
        Some(r#"{"caret":12,"scroll":340.5}"#)
    );
    let meta = store.document_meta(d.id).unwrap();
    assert_eq!(meta.updated_at, START);
    assert_eq!(
        store.document(d.id).unwrap().view_state.as_deref(),
        Some(r#"{"caret":12,"scroll":340.5}"#)
    );
}

#[test]
fn rename_bumps_updated_at_but_move_does_not() {
    let (mut store, clock) = store();
    let f = store.create_folder(None, "f").unwrap();
    let d = doc(&mut store, None, "a", "text");
    clock.advance_ms(10);
    store.rename_document(d.id, "b").unwrap();
    assert_eq!(store.document_meta(d.id).unwrap().updated_at, START + 10);
    clock.advance_ms(10);
    store.move_document(d.id, Some(f.id)).unwrap();
    let meta = store.document_meta(d.id).unwrap();
    assert_eq!(meta.updated_at, START + 10);
    assert_eq!(meta.folder_id, Some(f.id));
    assert_eq!(meta.title, "b");
}

#[test]
fn move_reorder_and_listings() {
    let (mut store, clock) = store();
    let f = store.create_folder(None, "f").unwrap();
    let mut ids = Vec::new();
    for t in ["one", "two", "three", "four"] {
        clock.advance_ms(1);
        ids.push(doc(&mut store, None, t, t).id);
    }
    assert_eq!(
        doc_titles(&store, None),
        vec!["one", "two", "three", "four"]
    );
    store.reorder_document(ids[3], 1).unwrap();
    assert_eq!(
        doc_titles(&store, None),
        vec!["one", "four", "two", "three"]
    );
    store.move_document(ids[0], Some(f.id)).unwrap();
    store.move_document(ids[2], Some(f.id)).unwrap();
    assert_eq!(doc_titles(&store, None), vec!["four", "two"]);
    assert_eq!(doc_titles(&store, Some(f.id)), vec!["one", "three"]);
    store.reorder_document(ids[2], 0).unwrap();
    assert_eq!(doc_titles(&store, Some(f.id)), vec!["three", "one"]);
    store.move_document(ids[2], None).unwrap();
    assert_eq!(doc_titles(&store, None), vec!["four", "two", "three"]);
    assert_eq!(doc_titles(&store, Some(f.id)), vec!["one"]);

    // All documents: most recently edited first.
    let all: Vec<String> = store
        .documents()
        .unwrap()
        .into_iter()
        .map(|d| d.title)
        .collect();
    assert_eq!(all, vec!["four", "three", "two", "one"]);
    clock.advance_ms(100);
    store
        .save_document(ids[0], &content("one", "edited", "{}"))
        .unwrap();
    let recent: Vec<String> = store
        .recent_documents(2)
        .unwrap()
        .into_iter()
        .map(|d| d.title)
        .collect();
    assert_eq!(recent, vec!["one", "four"]);
}

#[test]
fn ordering_after_many_document_moves() {
    let (mut store, _) = store();
    let folders = [
        None,
        Some(store.create_folder(None, "a").unwrap().id),
        Some(store.create_folder(None, "b").unwrap().id),
    ];
    let mut model: Vec<Vec<String>> = vec![Vec::new(); 3];
    let mut docs = Vec::new();
    for i in 0..24 {
        let name = format!("d{i}");
        let id = doc(&mut store, folders[i % 3], &name, "x").id;
        model[i % 3].push(name.clone());
        docs.push((id, name));
    }
    let mut seed: u64 = 7;
    let mut next = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        (seed >> 33) as usize
    };
    for _ in 0..400 {
        let (id, name) = docs[next() % docs.len()].clone();
        let from = model.iter().position(|v| v.contains(&name)).unwrap();
        match next() % 3 {
            0 => {
                let to = next() % 3;
                store.move_document(id, folders[to]).unwrap();
                if to != from {
                    model[from].retain(|n| n != &name);
                    model[to].push(name);
                }
            }
            _ => {
                let index = next() % 10;
                store.reorder_document(id, index).unwrap();
                model[from].retain(|n| n != &name);
                let index = index.min(model[from].len());
                model[from].insert(index, name);
            }
        }
    }
    for (i, folder) in folders.iter().enumerate() {
        assert_eq!(doc_titles(&store, *folder), model[i]);
    }
}

#[test]
fn duplicate_places_copy_after_original() {
    let (mut store, clock) = store();
    let a = doc(&mut store, None, "a", "alpha text");
    let _b = doc(&mut store, None, "b", "beta");
    store.save_view_state(a.id, "{\"caret\":3}").unwrap();
    clock.advance_ms(50);
    let copy = store.duplicate_document(a.id).unwrap();
    assert_eq!(copy.title, "a copy");
    assert_eq!(copy.created_at, START + 50);
    assert_eq!(copy.word_count, 2);
    assert_eq!(doc_titles(&store, None), vec!["a", "a copy", "b"]);
    let loaded = store.document(copy.id).unwrap();
    assert_eq!(loaded.content, store.document(a.id).unwrap().content);
    assert_eq!(loaded.view_state.as_deref(), Some("{\"caret\":3}"));
    assert_eq!(store.search("alpha", 10).unwrap().len(), 2);

    let untitled = doc(&mut store, None, "", "x");
    assert_eq!(
        store.duplicate_document(untitled.id).unwrap().title,
        "Untitled copy"
    );
}

#[test]
fn moving_into_missing_or_trashed_folder_fails() {
    let (mut store, _) = store();
    let f = store.create_folder(None, "f").unwrap();
    let d = doc(&mut store, None, "a", "x");
    store.trash_folder(f.id).unwrap();
    assert!(matches!(
        store.move_document(d.id, Some(f.id)),
        Err(StoreError::NotFound { .. })
    ));
    assert!(matches!(
        doc_create_in(&mut store, f.id),
        Err(StoreError::NotFound { .. })
    ));
}

fn doc_create_in(
    store: &mut writer_store::Store,
    f: writer_store::FolderId,
) -> writer_store::Result<writer_store::DocumentMeta> {
    store.create_document(Some(f), &content("t", "x", "{}"))
}

#[test]
fn saving_a_trashed_document_still_works() {
    let (mut store, _) = store();
    let d = doc(&mut store, None, "a", "x");
    store.trash_document(d.id).unwrap();
    store
        .save_document(d.id, &content("a", "late autosave", "{\"late\":true}"))
        .unwrap();
    assert_eq!(store.document(d.id).unwrap().content, "{\"late\":true}");
    assert!(store.document(d.id).unwrap().meta.trashed_at.is_some());
    assert!(store.search("autosave", 10).unwrap().is_empty());
}

#[test]
fn library_tree_nests_in_display_order() {
    let (mut store, _) = store();
    let a = store.create_folder(None, "A").unwrap();
    let b = store.create_folder(None, "B").unwrap();
    let a1 = store.create_folder(Some(a.id), "A1").unwrap();
    let a1x = store.create_folder(Some(a1.id), "A1x").unwrap();
    doc(&mut store, None, "root doc", "x");
    doc(&mut store, Some(a.id), "in A 1", "x");
    doc(&mut store, Some(a.id), "in A 2", "x");
    doc(&mut store, Some(a1x.id), "deep", "x");
    let trashed = doc(&mut store, Some(b.id), "gone", "x");
    store.trash_document(trashed.id).unwrap();
    store.reorder_folder(b.id, 0).unwrap();

    let tree = store.library_tree().unwrap();
    let names: Vec<&str> = tree
        .folders
        .iter()
        .map(|n| n.folder.name.as_str())
        .collect();
    assert_eq!(names, vec!["B", "A"]);
    assert!(tree.folders[0].documents.is_empty());
    let a_node = &tree.folders[1];
    let a_docs: Vec<&str> = a_node.documents.iter().map(|d| d.title.as_str()).collect();
    assert_eq!(a_docs, vec!["in A 1", "in A 2"]);
    assert_eq!(a_node.folders[0].folder.name, "A1");
    assert_eq!(a_node.folders[0].folders[0].documents[0].title, "deep");
    assert_eq!(tree.documents.len(), 1);
    assert_eq!(tree.documents[0].title, "root doc");
    assert_eq!(tree.find_folder(a1x.id).unwrap().folder.name, "A1x");
}
