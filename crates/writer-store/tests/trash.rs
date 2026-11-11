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
    // Back in its place, in front of "b".
    assert_eq!(doc_titles(&store, Some(f.id)), vec!["a", "b"]);
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
    // "c" was created after "b" (the trash left no gap in the positions), and "a" comes
    // back to the first place it had.
    store.restore_folder(a.id).unwrap();
    assert_eq!(folder_names(&store, None), vec!["a", "b", "c"]);
    store.reorder_folder(c.id, 0).unwrap();
    assert_eq!(folder_names(&store, None), vec!["c", "a", "b"]);
}

#[test]
fn a_restore_puts_the_item_back_in_its_place() {
    let (mut store, _) = store();
    let book = store.create_folder(None, "Book").unwrap();
    let ids: Vec<_> = ["1", "2", "3", "4"]
        .iter()
        .map(|title| doc(&mut store, Some(book.id), title, "x").id)
        .collect();
    let titles = |store: &writer_store::Store| doc_titles(store, Some(book.id));

    // Undo of one trash: the chapter is back between its neighbours, not at the end.
    store.trash_document(ids[1]).unwrap();
    assert_eq!(titles(&store), vec!["1", "3", "4"]);
    let restored = store.restore_document(ids[1]).unwrap();
    assert_eq!(titles(&store), vec!["1", "2", "3", "4"]);
    assert_eq!(restored.position, 1);
    // The first and the last.
    store.trash_document(ids[0]).unwrap();
    store.restore_document(ids[0]).unwrap();
    store.trash_document(ids[3]).unwrap();
    store.restore_document(ids[3]).unwrap();
    assert_eq!(titles(&store), vec!["1", "2", "3", "4"]);
    // Several, undone newest first, as cmd-z does.
    for ix in [1, 2, 0] {
        store.trash_document(ids[ix]).unwrap();
    }
    assert_eq!(titles(&store), vec!["4"]);
    for ix in [0, 2, 1] {
        store.restore_document(ids[ix]).unwrap();
    }
    assert_eq!(titles(&store), vec!["1", "2", "3", "4"]);
    // The others were reordered meanwhile: it lands at its old index among them.
    store.trash_document(ids[1]).unwrap();
    store.reorder_document(ids[3], 0).unwrap();
    assert_eq!(titles(&store), vec!["4", "1", "3"]);
    store.restore_document(ids[1]).unwrap();
    assert_eq!(titles(&store), vec!["4", "2", "1", "3"]);
    // Fewer are left than its old index: it goes last.
    store.trash_document(ids[3]).unwrap();
    store.trash_document(ids[1]).unwrap();
    store.trash_document(ids[0]).unwrap();
    assert_eq!(titles(&store), vec!["3"]);
    store.restore_document(ids[1]).unwrap();
    assert_eq!(titles(&store), vec!["2", "3"]);
    store.restore_document(ids[0]).unwrap();
    store.restore_document(ids[3]).unwrap();
    // Positions stay a clean sequence whatever happened.
    let positions: Vec<i64> = store
        .documents_in_folder(Some(book.id))
        .unwrap()
        .iter()
        .map(|d| d.position)
        .collect();
    assert_eq!(positions, vec![0, 1, 2, 3]);

    // Folders: the same among their sibling folders, with their contents.
    let a = store.create_folder(None, "A").unwrap();
    let c = store.create_folder(None, "C").unwrap();
    assert_eq!(folder_names(&store, None), vec!["Book", "A", "C"]);
    store.trash_folder(a.id).unwrap();
    store.trash_folder(book.id).unwrap();
    assert_eq!(folder_names(&store, None), vec!["C"]);
    store.restore_folder(book.id).unwrap();
    store.restore_folder(a.id).unwrap();
    assert_eq!(folder_names(&store, None), vec!["Book", "A", "C"]);
    assert_eq!(titles(&store).len(), 4);

    // When the folder it was in is gone, it goes to the end of the root.
    let loose = doc(&mut store, None, "Loose", "x");
    let inner = doc(&mut store, Some(c.id), "Inner", "x");
    store.trash_document(inner.id).unwrap();
    store.trash_folder(c.id).unwrap();
    let restored = store.restore_document(inner.id).unwrap();
    assert_eq!(restored.folder_id, None);
    assert_eq!(doc_titles(&store, None), vec!["Loose", "Inner"]);
    assert_eq!(store.document_meta(loose.id).unwrap().position, 0);
    // The same when that folder was deleted for good, which leaves the trashed document
    // without a folder: the root is not where it was, so it does not jump the queue there.
    // Also for a folder that was trashed on its own inside the deleted one.
    store.restore_folder(c.id).unwrap();
    let first = doc(&mut store, Some(c.id), "First in C", "x");
    let sub = store.create_folder(Some(c.id), "Sub").unwrap();
    doc(&mut store, Some(sub.id), "In sub", "x");
    store.trash_document(first.id).unwrap();
    store.trash_folder(sub.id).unwrap();
    store.trash_folder(c.id).unwrap();
    store.purge_folder(c.id).unwrap();
    assert_eq!(store.trash().unwrap().len(), 2);
    let restored = store.restore_document(first.id).unwrap();
    assert_eq!(restored.folder_id, None);
    assert_eq!(
        doc_titles(&store, None),
        vec!["Loose", "Inner", "First in C"]
    );
    let restored = store.restore_folder(sub.id).unwrap();
    assert_eq!(restored.parent_id, None);
    assert_eq!(folder_names(&store, None), vec!["Book", "A", "Sub"]);
    assert_eq!(doc_titles(&store, Some(sub.id)), vec!["In sub"]);
    // A root document trashed and restored still goes back to its own place.
    store.trash_document(loose.id).unwrap();
    store.restore_document(loose.id).unwrap();
    assert_eq!(
        doc_titles(&store, None),
        vec!["Loose", "Inner", "First in C"]
    );
}
