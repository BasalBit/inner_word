mod common;

use common::*;
use writer_store::{FolderId, ItemKind, StoreError};

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
