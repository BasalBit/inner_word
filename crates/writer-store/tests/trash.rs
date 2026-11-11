mod common;

use common::*;
use writer_store::{EmptiedTrash, StoreError, TrashEntry};

#[test]
fn trash_and_restore_document() {
    let (mut store, clock) = store();
    let f = store.create_folder(None, "f").unwrap();
    let a = doc(&mut store, Some(f.id), "a", "apple");
    let b = doc(&mut store, Some(f.id), "b", "banana");
    clock.advance_ms(5);
    store.trash_document(a.id).unwrap();
    assert_eq!(doc_titles(&store, Some(f.id)), vec!["b"]);
    assert!(store.search("apple", 10).unwrap().is_empty());
    assert!(!store.document_exists(a.id).unwrap());
    assert_eq!(store.documents().unwrap().len(), 1);
    let trash = store.trash().unwrap();
    assert_eq!(trash.len(), 1);
    assert!(matches!(&trash[0], TrashEntry::Document(d) if d.id == a.id));
    assert_eq!(trash[0].trashed_at(), START + 5);
    // Trashing again fails: it's not live.
    assert!(matches!(
        store.trash_document(a.id),
        Err(StoreError::NotFound { .. })
    ));

    let restored = store.restore_document(a.id).unwrap();
    assert_eq!(restored.folder_id, Some(f.id));
    assert_eq!(restored.trashed_at, None);
    assert_eq!(doc_titles(&store, Some(f.id)), vec!["b", "a"]);
    assert_eq!(store.search("apple", 10).unwrap().len(), 1);
    assert!(store.trash().unwrap().is_empty());
    assert!(matches!(
        store.restore_document(b.id),
        Err(StoreError::NotInTrash { .. })
    ));
}

#[test]
fn restore_goes_to_root_when_parent_is_gone() {
    let (mut store, _) = store();
    let f = store.create_folder(None, "f").unwrap();
    let a = doc(&mut store, Some(f.id), "a", "x");
    store.trash_document(a.id).unwrap();
    store.trash_folder(f.id).unwrap();
    // Folder still trashed: restore to the root.
    let restored = store.restore_document(a.id).unwrap();
    assert_eq!(restored.folder_id, None);
    store.trash_document(a.id).unwrap();
    store.restore_folder(f.id).unwrap();
    // The separately trashed doc stays in the trash, even though f is back.
    assert_eq!(store.trash().unwrap().len(), 1);
    store.trash_folder(f.id).unwrap();
    store.purge_folder(f.id).unwrap();
    // Folder purged: doc is still in the trash, restores to the root.
    assert_eq!(store.trash().unwrap().len(), 1);
    assert_eq!(store.restore_document(a.id).unwrap().folder_id, None);
}

#[test]
fn nested_folder_trash_and_restore() {
    let (mut store, clock) = store();
    let top = store.create_folder(None, "top").unwrap();
    let root_sibling = store.create_folder(None, "sibling").unwrap();
    let mid = store.create_folder(Some(top.id), "mid").unwrap();
    let leaf = store.create_folder(Some(mid.id), "leaf").unwrap();
    let other = store.create_folder(Some(mid.id), "other").unwrap();
    doc(&mut store, Some(top.id), "t1", "topdoc");
    doc(&mut store, Some(mid.id), "m1", "middoc");
    doc(&mut store, Some(leaf.id), "l1", "leafdoc");
    let earlier = doc(&mut store, Some(leaf.id), "l2", "earliertrashed");
    store.trash_document(earlier.id).unwrap();
    store.trash_folder(other.id).unwrap();

    clock.advance_ms(1000);
    store.trash_folder(mid.id).unwrap();
    let tree = store.library_tree().unwrap();
    let top_node = tree.find_folder(top.id).unwrap();
    assert!(top_node.folders.is_empty());
    assert_eq!(top_node.documents.len(), 1);
    assert!(tree.find_folder(leaf.id).is_none());
    assert!(store.search("leafdoc", 10).unwrap().is_empty());
    assert!(store.search("middoc", 10).unwrap().is_empty());
    assert_eq!(store.documents().unwrap().len(), 1);

    let trash = store.trash().unwrap();
    assert_eq!(trash.len(), 3, "{trash:?}");
    match &trash[0] {
        TrashEntry::Folder {
            folder,
            folder_count,
            document_count,
        } => {
            assert_eq!(folder.id, mid.id);
            assert_eq!(*folder_count, 1); // leaf (other was trashed separately)
            assert_eq!(*document_count, 2); // m1, l1
        }
        other => panic!("unexpected {other:?}"),
    }
    // Nested items cannot be restored on their own.
    assert!(matches!(
        store.restore_folder(leaf.id),
        Err(StoreError::NotInTrash { .. })
    ));

    let restored = store.restore_folder(mid.id).unwrap();
    assert_eq!(restored.parent_id, Some(top.id));
    let tree = store.library_tree().unwrap();
    let mid_node = tree.find_folder(mid.id).unwrap();
    assert_eq!(mid_node.folders.len(), 1);
    assert_eq!(mid_node.folders[0].folder.id, leaf.id);
    assert_eq!(mid_node.folders[0].documents.len(), 1);
    assert_eq!(mid_node.documents.len(), 1);
    assert_eq!(store.search("leafdoc", 10).unwrap().len(), 1);
    // Separately trashed items stay trashed.
    assert_eq!(store.trash().unwrap().len(), 2);
    assert!(store.search("earliertrashed", 10).unwrap().is_empty());

    // Parent trashed meanwhile: restore to root.
    store.trash_folder(mid.id).unwrap();
    store.trash_folder(top.id).unwrap();
    let back = store.restore_folder(mid.id).unwrap();
    assert_eq!(back.parent_id, None);
    assert_eq!(folder_names(&store, None), vec!["sibling", "mid"]);
    let _ = root_sibling;
}

#[test]
fn purge_and_empty_trash() {
    let (mut store, _) = store();
    let f = store.create_folder(None, "f").unwrap();
    let sub = store.create_folder(Some(f.id), "sub").unwrap();
    let a = doc(&mut store, Some(sub.id), "a", "x");
    let b = doc(&mut store, None, "b", "y");
    let c = doc(&mut store, None, "c", "z");
    store.create_snapshot(b.id).unwrap();
    store.trash_folder(f.id).unwrap();
    store.trash_document(b.id).unwrap();
    store.trash_document(c.id).unwrap();

    assert!(matches!(
        store.purge_folder(sub.id),
        Err(StoreError::NotInTrash { .. })
    ));
    store.purge_document(b.id).unwrap();
    assert!(matches!(
        store.document(b.id),
        Err(StoreError::NotFound { .. })
    ));
    assert!(store.snapshots(b.id).unwrap().is_empty());
    store.purge_folder(f.id).unwrap();
    assert!(matches!(
        store.folder(sub.id),
        Err(StoreError::NotFound { .. })
    ));
    assert!(matches!(
        store.document(a.id),
        Err(StoreError::NotFound { .. })
    ));
    assert_eq!(store.trash().unwrap().len(), 1);

    let g = store.create_folder(None, "g").unwrap();
    store.create_folder(Some(g.id), "g1").unwrap();
    doc(&mut store, Some(g.id), "gd", "w");
    store.trash_folder(g.id).unwrap();
    let emptied = store.empty_trash().unwrap();
    assert_eq!(
        emptied,
        EmptiedTrash {
            folders: 2,
            documents: 2
        }
    );
    assert!(store.trash().unwrap().is_empty());
    assert!(store.search("w", 10).unwrap().is_empty());
}

#[test]
fn trashing_renumbers_siblings() {
    let (mut store, _) = store();
    let a = store.create_folder(None, "a").unwrap();
    store.create_folder(None, "b").unwrap();
    store.trash_folder(a.id).unwrap();
    let c = store.create_folder(None, "c").unwrap();
    store.restore_folder(a.id).unwrap();
    assert_eq!(folder_names(&store, None), vec!["b", "c", "a"]);
    store.reorder_folder(c.id, 0).unwrap();
    assert_eq!(folder_names(&store, None), vec!["c", "b", "a"]);
}
