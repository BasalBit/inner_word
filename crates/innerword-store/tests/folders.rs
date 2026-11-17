mod common;

use common::*;
use innerword_store::{FolderId, ItemKind, StoreError};

#[test]
fn create_and_load_folders_with_unicode_names() {
    let (mut store, _) = store();
    let a = store.create_folder(None, "Brouillons — été 🌞").unwrap();
    let b = store.create_folder(Some(a.id), "書きかけ").unwrap();
    assert_eq!(a.parent_id, None);
    assert_eq!(a.created_at, START);
    assert_eq!(a.updated_at, START);
    assert_eq!(b.parent_id, Some(a.id));
    assert_eq!(store.folder(b.id).unwrap().name, "書きかけ");
    assert_eq!(folder_names(&store, None), vec!["Brouillons — été 🌞"]);
    assert_eq!(folder_names(&store, Some(a.id)), vec!["書きかけ"]);
    assert_eq!(store.folders().unwrap().len(), 2);
}

#[test]
fn blank_names_and_missing_parents_are_rejected() {
    let (mut store, _) = store();
    assert!(matches!(
        store.create_folder(None, "  \t"),
        Err(StoreError::EmptyName)
    ));
    assert!(matches!(
        store.create_folder(Some(FolderId(99)), "x"),
        Err(StoreError::NotFound {
            kind: ItemKind::Folder,
            id: 99
        })
    ));
    let f = store.create_folder(None, "x").unwrap();
    assert!(matches!(
        store.rename_folder(f.id, ""),
        Err(StoreError::EmptyName)
    ));
    assert!(matches!(
        store.folder(FolderId(1234)),
        Err(StoreError::NotFound { .. })
    ));
}

#[test]
fn rename_bumps_updated_at() {
    let (mut store, clock) = store();
    let f = store.create_folder(None, "Old").unwrap();
    clock.advance_ms(5_000);
    store.rename_folder(f.id, "New").unwrap();
    let f = store.folder(f.id).unwrap();
    assert_eq!(f.name, "New");
    assert_eq!(f.created_at, START);
    assert_eq!(f.updated_at, START + 5_000);
}

#[test]
fn siblings_append_in_creation_order_and_reorder() {
    let (mut store, _) = store();
    let ids: Vec<_> = ["a", "b", "c", "d"]
        .iter()
        .map(|n| store.create_folder(None, n).unwrap().id)
        .collect();
    store.reorder_folder(ids[3], 0).unwrap();
    assert_eq!(folder_names(&store, None), vec!["d", "a", "b", "c"]);
    store.reorder_folder(ids[3], 100).unwrap();
    assert_eq!(folder_names(&store, None), vec!["a", "b", "c", "d"]);
    store.reorder_folder(ids[0], 2).unwrap();
    assert_eq!(folder_names(&store, None), vec!["b", "c", "a", "d"]);
}

#[test]
fn move_between_parents_and_to_root() {
    let (mut store, clock) = store();
    let a = store.create_folder(None, "a").unwrap();
    let b = store.create_folder(None, "b").unwrap();
    let c = store.create_folder(Some(a.id), "c").unwrap();
    let d = store.create_folder(Some(a.id), "d").unwrap();
    clock.advance_ms(10);
    store.move_folder(c.id, Some(b.id)).unwrap();
    assert_eq!(folder_names(&store, Some(a.id)), vec!["d"]);
    assert_eq!(folder_names(&store, Some(b.id)), vec!["c"]);
    assert_eq!(store.folder(c.id).unwrap().updated_at, START + 10);
    store.move_folder(d.id, None).unwrap();
    assert_eq!(folder_names(&store, None), vec!["a", "b", "d"]);
    // Moving to the current parent is a no-op.
    store.move_folder(d.id, None).unwrap();
    assert_eq!(folder_names(&store, None), vec!["a", "b", "d"]);
}

#[test]
fn moving_into_itself_or_descendant_is_rejected() {
    let (mut store, _) = store();
    let a = store.create_folder(None, "a").unwrap();
    let b = store.create_folder(Some(a.id), "b").unwrap();
    let c = store.create_folder(Some(b.id), "c").unwrap();
    let d = store.create_folder(Some(c.id), "d").unwrap();
    for target in [a.id, b.id, c.id, d.id] {
        assert!(
            matches!(
                store.move_folder(a.id, Some(target)),
                Err(StoreError::FolderCycle)
            ),
            "moving a into {target} must fail"
        );
    }
    assert!(matches!(
        store.move_folder(b.id, Some(d.id)),
        Err(StoreError::FolderCycle)
    ));
    // Nothing changed.
    assert_eq!(store.folder(a.id).unwrap().parent_id, None);
    assert_eq!(store.folder(b.id).unwrap().parent_id, Some(a.id));
    // Moving a descendant up is fine.
    store.move_folder(d.id, Some(a.id)).unwrap();
    assert_eq!(folder_names(&store, Some(a.id)), vec!["b", "d"]);
}

#[test]
fn ordering_survives_many_moves_and_reorders() {
    let (mut store, _) = store();
    let parents: Vec<_> = (0..3)
        .map(|i| store.create_folder(None, &format!("p{i}")).unwrap().id)
        .collect();
    let mut model: Vec<Vec<String>> = vec![Vec::new(); 3];
    let mut ids = Vec::new();
    for i in 0..30 {
        let name = format!("f{i}");
        let id = store.create_folder(Some(parents[i % 3]), &name).unwrap().id;
        model[i % 3].push(name.clone());
        ids.push((id, name));
    }
    // Deterministic pseudo-random sequence of operations, mirrored in a simple model.
    let mut seed: u64 = 42;
    let mut next = || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (seed >> 33) as usize
    };
    for _ in 0..500 {
        let (id, name) = ids[next() % ids.len()].clone();
        let from = model.iter().position(|v| v.contains(&name)).unwrap();
        if next() % 2 == 0 {
            let to = next() % 3;
            store.move_folder(id, Some(parents[to])).unwrap();
            if to != from {
                model[from].retain(|n| n != &name);
                model[to].push(name);
            }
        } else {
            let index = next() % 12;
            store.reorder_folder(id, index).unwrap();
            model[from].retain(|n| n != &name);
            let index = index.min(model[from].len());
            model[from].insert(index, name);
        }
    }
    for (i, p) in parents.iter().enumerate() {
        assert_eq!(folder_names(&store, Some(*p)), model[i]);
        let positions: Vec<i64> = store
            .child_folders(Some(*p))
            .unwrap()
            .iter()
            .map(|f| f.position)
            .collect();
        let mut sorted = positions.clone();
        sorted.dedup();
        assert_eq!(sorted.len(), positions.len(), "positions are unique");
    }
}

#[test]
fn trashed_folders_cannot_be_targets_or_edited() {
    let (mut store, _) = store();
    let a = store.create_folder(None, "a").unwrap();
    let b = store.create_folder(None, "b").unwrap();
    store.trash_folder(a.id).unwrap();
    assert!(matches!(
        store.move_folder(b.id, Some(a.id)),
        Err(StoreError::NotFound { .. })
    ));
    assert!(matches!(
        store.rename_folder(a.id, "z"),
        Err(StoreError::NotFound { .. })
    ));
    assert!(matches!(
        store.create_folder(Some(a.id), "child"),
        Err(StoreError::NotFound { .. })
    ));
    // Still loadable, marked trashed.
    assert_eq!(store.folder(a.id).unwrap().trashed_at, Some(START));
}

#[test]
fn duplicate_copies_the_whole_live_subtree_next_to_the_original() {
    let (mut store, clock) = store();
    let before = store.create_folder(None, "Before").unwrap();
    let book = store.create_folder(None, "Book").unwrap();
    let after = store.create_folder(None, "After").unwrap();
    let part = store.create_folder(Some(book.id), "Part one").unwrap();
    let notes = store.create_folder(Some(book.id), "Notes").unwrap();
    let deep = store.create_folder(Some(part.id), "Scenes").unwrap();
    let empty = store.create_folder(Some(book.id), "Empty").unwrap();
    let outline = doc(&mut store, Some(book.id), "Outline", "the plan");
    let first = doc(&mut store, Some(part.id), "Chapter 1", "harbour at dawn");
    doc(&mut store, Some(part.id), "Chapter 2", "low tide");
    doc(&mut store, Some(deep.id), "Scene", "a letter arrives");
    store.save_view_state(first.id, "{\"caret\":3}").unwrap();
    store.create_snapshot(first.id).unwrap();
    // Trashed things inside the folder stay behind.
    let cut = doc(&mut store, Some(part.id), "Cut chapter", "gone");
    store.trash_document(cut.id).unwrap();
    store.trash_folder(notes.id).unwrap();
    // The order inside is not creation order.
    store.reorder_folder(empty.id, 0).unwrap();
    store.reorder_document(first.id, 1).unwrap();

    clock.advance_ms(HOUR);
    let copy = store.duplicate_folder(book.id).unwrap();
    assert_eq!(copy.name, "Book copy");
    assert_eq!(copy.parent_id, None);
    assert_eq!(copy.created_at, START + HOUR);
    assert_eq!(
        folder_names(&store, None),
        vec!["Before", "Book", "Book copy", "After"]
    );

    let tree = store.library_tree().unwrap();
    let original = tree.find_folder(book.id).unwrap();
    let copied = tree.find_folder(copy.id).unwrap();
    /// Names and titles of a subtree, without ids.
    fn shape(node: &innerword_store::FolderNode) -> String {
        let folders: Vec<String> = node.folders.iter().map(shape).collect();
        let documents: Vec<&str> = node.documents.iter().map(|d| d.title.as_str()).collect();
        format!(
            "{}[{}|{}]",
            node.folder.name,
            folders.join(","),
            documents.join(",")
        )
    }
    assert_eq!(
        shape(copied),
        "Book copy[Empty[|],Part one[Scenes[|Scene]|Chapter 2,Chapter 1]|Outline]"
    );
    assert_eq!(
        shape(copied).replacen("Book copy", "Book", 1),
        shape(original)
    );

    // The copies are documents of their own: content, text, counts and view state.
    let copied_first = copied.folders[1]
        .documents
        .iter()
        .find(|d| d.title == "Chapter 1")
        .unwrap();
    assert_ne!(copied_first.id, first.id);
    assert_eq!(copied_first.word_count, first.word_count);
    assert_eq!(copied_first.updated_at, START + HOUR);
    let loaded = store.document(copied_first.id).unwrap();
    assert_eq!(loaded.content, store.document(first.id).unwrap().content);
    assert_eq!(loaded.view_state.as_deref(), Some("{\"caret\":3}"));
    assert!(store.snapshots(copied_first.id).unwrap().is_empty());
    // Both are found by the search.
    assert_eq!(store.search("harbour", 10).unwrap().len(), 2);
    // The original is untouched.
    assert_eq!(store.document_meta(outline.id).unwrap(), outline);
    assert_eq!(store.folder(after.id).unwrap().parent_id, None);
    assert_eq!(store.folder(before.id).unwrap().parent_id, None);
    // The trash still holds exactly what was trashed.
    assert_eq!(store.trash().unwrap().len(), 2);

    // A nested folder is copied next to itself; a trashed or missing one is not found.
    let copy = store.duplicate_folder(part.id).unwrap();
    assert_eq!(copy.parent_id, Some(book.id));
    assert_eq!(
        folder_names(&store, Some(book.id)),
        vec!["Empty", "Part one", "Part one copy"]
    );
    assert!(matches!(
        store.duplicate_folder(notes.id),
        Err(StoreError::NotFound { .. })
    ));
    assert!(matches!(
        store.duplicate_folder(FolderId(4040)),
        Err(StoreError::NotFound { .. })
    ));
}
