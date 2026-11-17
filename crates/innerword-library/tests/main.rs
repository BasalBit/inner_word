//! Tests for the `Library` model (events, tree cache, errors). The panel's tests are in
//! `panel/mod.rs` (one test binary for the crate).

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{AppContext as _, Entity, TestAppContext};
use innerword_library::{
    Clock, DocumentContent, DocumentId, FolderId, ImageFormat, Library, LibraryEvent, NewImage,
    StoreError, StoredImageId, TrashEntry,
};

mod panel;

type Events = Rc<RefCell<Vec<LibraryEvent>>>;

fn content<'a>(title: &'a str, text: &'a str) -> DocumentContent<'a> {
    DocumentContent {
        content: "{}",
        title,
        plain_text: text,
        word_count: text.split_whitespace().count() as u64,
        char_count: text.chars().count() as u64,
    }
}

fn library(cx: &mut TestAppContext) -> (Entity<Library>, Events) {
    library_from(Library::in_memory().unwrap(), cx)
}

fn library_from(library: Library, cx: &mut TestAppContext) -> (Entity<Library>, Events) {
    let events: Events = Rc::default();
    let library = cx.new(|_| library);
    cx.update(|cx| {
        let events = events.clone();
        cx.subscribe(&library, move |_, event: &LibraryEvent, _| {
            events.borrow_mut().push(event.clone())
        })
        .detach();
    });
    (library, events)
}

/// Drains the recorded events (after letting the effect queue flush).
fn take(events: &Events, cx: &mut TestAppContext) -> Vec<LibraryEvent> {
    cx.run_until_parked();
    events.borrow_mut().drain(..).collect()
}

#[gpui::test]
fn folders_emit_events_and_refresh_the_tree(cx: &mut TestAppContext) {
    let (library, events) = library(cx);
    assert!(library.read_with(cx, |l, _| l.tree().folders.is_empty()));

    let (a, b) = library.update(cx, |l, cx| {
        let a = l.create_folder(None, "Novel", cx).unwrap();
        // The cache is fresh inside the same update.
        assert_eq!(l.tree().folders.len(), 1);
        let b = l.create_folder(Some(a.id), "Part one", cx).unwrap();
        (a.id, b.id)
    });
    assert_eq!(
        take(&events, cx),
        vec![
            LibraryEvent::FolderCreated(a),
            LibraryEvent::TreeChanged,
            LibraryEvent::FolderCreated(b),
            LibraryEvent::TreeChanged,
        ]
    );
    library.read_with(cx, |l, _| {
        let tree = l.tree();
        assert_eq!(tree.folders[0].folders[0].folder.name, "Part one");
        assert_eq!(l.folder_path(Some(b)), vec!["Novel", "Part one"]);
        assert!(l.folder_path(None).is_empty());
    });

    library.update(cx, |l, cx| l.rename_folder(b, "Part 1", cx).unwrap());
    assert_eq!(
        take(&events, cx),
        vec![LibraryEvent::FolderRenamed(b), LibraryEvent::TreeChanged]
    );

    let c = library.update(cx, |l, cx| l.create_folder(None, "Blog", cx).unwrap().id);
    take(&events, cx);
    // Move "Part 1" to the root, in front of everything.
    library.update(cx, |l, cx| l.move_folder(b, None, Some(0), cx).unwrap());
    assert_eq!(
        take(&events, cx),
        vec![LibraryEvent::FolderMoved(b), LibraryEvent::TreeChanged]
    );
    let order = |l: &Library| {
        l.tree()
            .folders
            .iter()
            .map(|n| n.folder.id)
            .collect::<Vec<_>>()
    };
    assert_eq!(library.read_with(cx, |l, _| order(l)), vec![b, a, c]);
    library.update(cx, |l, cx| l.reorder_folder(b, 2, cx).unwrap());
    assert_eq!(library.read_with(cx, |l, _| order(l)), vec![a, c, b]);
}

#[gpui::test]
fn documents_emit_events_and_refresh_the_tree(cx: &mut TestAppContext) {
    let (library, events) = library(cx);
    let folder = library.update(cx, |l, cx| l.create_folder(None, "Drafts", cx).unwrap().id);
    let doc = library.update(cx, |l, cx| {
        l.create_document(Some(folder), &content("One", "first words"), cx)
            .unwrap()
            .id
    });
    take(&events, cx);

    // A save that keeps the title: DocumentSaved only, but the cached counts are fresh.
    library.update(cx, |l, cx| {
        l.save_document(doc, &content("One", "first words and more"), cx)
            .unwrap()
    });
    assert_eq!(
        take(&events, cx),
        vec![LibraryEvent::DocumentSaved {
            id: doc,
            title_changed: false
        }]
    );
    library.read_with(cx, |l, _| {
        assert_eq!(l.tree().folders[0].documents[0].word_count, 4);
        let loaded = l.document(doc).unwrap();
        assert_eq!(loaded.content, "{}");
        assert_eq!(loaded.meta.title, "One");
    });

    // A save that changes the title also changes the tree.
    library.update(cx, |l, cx| {
        l.save_document(doc, &content("Uno", "first"), cx).unwrap()
    });
    assert_eq!(
        take(&events, cx),
        vec![
            LibraryEvent::DocumentSaved {
                id: doc,
                title_changed: true
            },
            LibraryEvent::TreeChanged
        ]
    );
    assert_eq!(
        library.read_with(cx, |l, _| l.tree().folders[0].documents[0].title.clone()),
        "Uno"
    );

    library.update(cx, |l, cx| l.rename_document(doc, "Eins", cx).unwrap());
    assert_eq!(
        take(&events, cx),
        vec![
            LibraryEvent::DocumentRenamed(doc),
            LibraryEvent::TreeChanged
        ]
    );

    let copy = library.update(cx, |l, cx| l.duplicate_document(doc, cx).unwrap());
    assert_eq!(copy.title, "Eins copy");
    assert_eq!(
        take(&events, cx),
        vec![
            LibraryEvent::DocumentCreated(copy.id),
            LibraryEvent::TreeChanged
        ]
    );

    library.update(cx, |l, cx| {
        l.move_document(copy.id, None, None, cx).unwrap()
    });
    assert_eq!(
        take(&events, cx),
        vec![
            LibraryEvent::DocumentMoved(copy.id),
            LibraryEvent::TreeChanged
        ]
    );
    library.read_with(cx, |l, _| {
        let tree = l.tree();
        assert_eq!(tree.documents[0].id, copy.id);
        assert_eq!(tree.folders[0].documents.len(), 1);
    });

    // Move back, to the front of the folder.
    library.update(cx, |l, cx| {
        l.move_document(copy.id, Some(folder), Some(0), cx).unwrap()
    });
    library.read_with(cx, |l, _| {
        let ids: Vec<_> = l.tree().folders[0].documents.iter().map(|d| d.id).collect();
        assert_eq!(ids, vec![copy.id, doc]);
    });
    library.update(cx, |l, cx| l.reorder_document(copy.id, 1, cx).unwrap());
    library.read_with(cx, |l, _| {
        let ids: Vec<_> = l
            .documents_in_folder(Some(folder))
            .unwrap()
            .iter()
            .map(|d| d.id)
            .collect();
        assert_eq!(ids, vec![doc, copy.id]);
    });

    // View state: stored, no event.
    take(&events, cx);
    library.update(cx, |l, _| l.save_view_state(doc, r#"{"caret":3}"#).unwrap());
    assert!(take(&events, cx).is_empty());
    assert_eq!(
        library.read_with(cx, |l, _| l.view_state(doc).unwrap()),
        Some(r#"{"caret":3}"#.to_string())
    );
}

#[gpui::test]
fn trash_restore_and_purge(cx: &mut TestAppContext) {
    let (library, events) = library(cx);
    let (folder, inner, d1, d2, d3) = library.update(cx, |l, cx| {
        let folder = l.create_folder(None, "Book", cx).unwrap().id;
        let inner = l.create_folder(Some(folder), "Notes", cx).unwrap().id;
        let d1 = l
            .create_document(Some(folder), &content("A", "a"), cx)
            .unwrap()
            .id;
        let d2 = l
            .create_document(Some(inner), &content("B", "b"), cx)
            .unwrap()
            .id;
        let d3 = l.create_document(None, &content("C", "c"), cx).unwrap().id;
        (folder, inner, d1, d2, d3)
    });
    take(&events, cx);

    library.update(cx, |l, cx| l.trash_document(d3, cx).unwrap());
    assert_eq!(
        take(&events, cx),
        vec![
            LibraryEvent::DocumentTrashed(d3),
            LibraryEvent::TrashChanged,
            LibraryEvent::TreeChanged
        ]
    );
    assert!(library.read_with(cx, |l, _| l.tree().documents.is_empty()));
    assert!(!library.read_with(cx, |l, _| l.document_exists(d3).unwrap()));

    // A save on a trashed document works and does not report a tree change.
    library.update(cx, |l, cx| {
        l.save_document(d3, &content("C2", "c"), cx).unwrap()
    });
    assert_eq!(
        take(&events, cx),
        vec![LibraryEvent::DocumentSaved {
            id: d3,
            title_changed: true
        }]
    );

    library.update(cx, |l, cx| l.trash_folder(folder, cx).unwrap());
    let got = take(&events, cx);
    assert_eq!(got.len(), 3);
    match &got[0] {
        LibraryEvent::FolderTrashed { id, documents } => {
            assert_eq!(*id, folder);
            let mut documents = documents.clone();
            documents.sort();
            assert_eq!(documents, vec![d1, d2]);
        }
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(
        got[1..],
        [LibraryEvent::TrashChanged, LibraryEvent::TreeChanged]
    );
    assert!(library.read_with(cx, |l, _| l.tree().folders.is_empty()));
    let trash = library.read_with(cx, |l, _| l.trash().unwrap());
    assert_eq!(trash.len(), 2);
    assert!(matches!(
        &trash[0],
        TrashEntry::Folder {
            document_count: 2,
            folder_count: 1,
            ..
        }
    ));

    library.update(cx, |l, cx| l.restore_folder(folder, cx).unwrap());
    assert_eq!(
        take(&events, cx),
        vec![
            LibraryEvent::FolderRestored(folder),
            LibraryEvent::TrashChanged,
            LibraryEvent::TreeChanged
        ]
    );
    library.read_with(cx, |l, _| {
        let tree = l.tree();
        assert_eq!(tree.find_folder(inner).unwrap().documents[0].id, d2);
    });

    library.update(cx, |l, cx| l.restore_document(d3, cx).unwrap());
    assert_eq!(
        take(&events, cx),
        vec![
            LibraryEvent::DocumentRestored(d3),
            LibraryEvent::TrashChanged,
            LibraryEvent::TreeChanged
        ]
    );

    library.update(cx, |l, cx| {
        l.trash_document(d3, cx).unwrap();
        l.trash_folder(inner, cx).unwrap();
    });
    take(&events, cx);
    library.update(cx, |l, cx| l.purge_document(d3, cx).unwrap());
    assert_eq!(
        take(&events, cx),
        vec![LibraryEvent::DocumentPurged(d3), LibraryEvent::TrashChanged]
    );
    library.update(cx, |l, cx| l.purge_folder(inner, cx).unwrap());
    assert_eq!(
        take(&events, cx),
        vec![
            LibraryEvent::FolderPurged(inner),
            LibraryEvent::TrashChanged
        ]
    );

    library.update(cx, |l, cx| l.trash_document(d1, cx).unwrap());
    take(&events, cx);
    let emptied = library.update(cx, |l, cx| l.empty_trash(cx).unwrap());
    assert_eq!((emptied.folders, emptied.documents), (0, 1));
    assert_eq!(take(&events, cx), vec![LibraryEvent::TrashChanged]);
    assert!(library.read_with(cx, |l, _| l.trash().unwrap().is_empty()));
}

#[gpui::test]
fn errors_are_returned_and_emit_nothing(cx: &mut TestAppContext) {
    let (library, events) = library(cx);
    let folder = library.update(cx, |l, cx| l.create_folder(None, "A", cx).unwrap().id);
    let child = library.update(cx, |l, cx| {
        l.create_folder(Some(folder), "B", cx).unwrap().id
    });
    take(&events, cx);
    let before = library.read_with(cx, |l, _| l.tree());

    library.update(cx, |l, cx| {
        assert!(matches!(
            l.create_folder(None, "   ", cx),
            Err(StoreError::EmptyName)
        ));
        assert!(matches!(
            l.move_folder(folder, Some(child), None, cx),
            Err(StoreError::FolderCycle)
        ));
        assert!(matches!(
            l.rename_folder(FolderId(999), "x", cx),
            Err(StoreError::NotFound { .. })
        ));
        assert!(matches!(
            l.save_document(DocumentId(999), &content("t", "x"), cx),
            Err(StoreError::NotFound { .. })
        ));
        assert!(matches!(
            l.trash_document(DocumentId(999), cx),
            Err(StoreError::NotFound { .. })
        ));
        assert!(matches!(
            l.trash_folder(FolderId(999), cx),
            Err(StoreError::NotFound { .. })
        ));
        assert!(matches!(
            l.restore_folder(folder, cx),
            Err(StoreError::NotInTrash { .. })
        ));
        assert!(matches!(
            l.document(DocumentId(999)),
            Err(StoreError::NotFound { .. })
        ));
        assert!(l.create_snapshot(DocumentId(999), cx).is_err());
    });
    assert!(take(&events, cx).is_empty());
    assert_eq!(library.read_with(cx, |l, _| l.tree()), before);
}

#[gpui::test]
fn opening_a_file_that_is_not_a_library_fails(_cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.db");
    std::fs::write(&path, b"this is not a database").unwrap();
    assert!(Library::open(&path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"this is not a database");
}

#[gpui::test]
fn a_library_on_disk_persists(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested").join("library.db");
    let (library, _events) = library_from(Library::open(&path).unwrap(), cx);
    assert_eq!(
        library.read_with(cx, |l, _| l.path().map(|p| p.to_path_buf())),
        Some(path.clone())
    );
    let doc = library.update(cx, |l, cx| {
        l.set_setting("theme", "dark", cx).unwrap();
        l.create_document(None, &content("Kept", "still here"), cx)
            .unwrap()
            .id
    });
    drop(library);
    cx.run_until_parked();

    let reopened = Library::open(&path).unwrap();
    assert_eq!(reopened.tree().documents[0].id, doc);
    assert_eq!(
        reopened.setting::<String>("theme").unwrap().as_deref(),
        Some("dark")
    );
}

#[gpui::test]
fn settings_events_carry_the_key(cx: &mut TestAppContext) {
    let (library, events) = library(cx);
    library.update(cx, |l, cx| {
        l.set_setting("editor.vim_mode", &true, cx).unwrap();
        l.set_setting("theme", "dark", cx).unwrap();
    });
    assert_eq!(
        take(&events, cx),
        vec![
            LibraryEvent::SettingChanged {
                key: "editor.vim_mode".into()
            },
            LibraryEvent::SettingChanged {
                key: "theme".into()
            },
        ]
    );
    library.read_with(cx, |l, _| {
        assert_eq!(l.setting::<bool>("editor.vim_mode").unwrap(), Some(true));
        assert_eq!(
            l.setting::<String>("theme").unwrap().as_deref(),
            Some("dark")
        );
        assert_eq!(l.setting::<u32>("missing").unwrap(), None);
        // A type mismatch is an error from `setting` and the default from `setting_or`.
        assert!(l.setting::<u32>("theme").is_err());
        assert_eq!(l.setting_or("theme", 7u32), 7);
        assert!(l.setting_or("editor.vim_mode", false));
        assert_eq!(l.setting_keys().unwrap(), vec!["editor.vim_mode", "theme"]);
    });
    library.update(cx, |l, cx| {
        assert!(l.remove_setting("theme", cx).unwrap());
        assert!(!l.remove_setting("theme", cx).unwrap());
    });
    assert_eq!(
        take(&events, cx),
        vec![LibraryEvent::SettingChanged {
            key: "theme".into()
        }]
    );
}

#[gpui::test]
fn snapshots_search_recents_and_quick_open(cx: &mut TestAppContext) {
    let clock = Clock::manual(1_700_000_000_000);
    let (library, events) = library_from(Library::in_memory_with_clock(clock.clone()).unwrap(), cx);
    let (inner, d1, d2) = library.update(cx, |l, cx| {
        let outer = l.create_folder(None, "Novel", cx).unwrap().id;
        let inner = l.create_folder(Some(outer), "Part one", cx).unwrap().id;
        let d1 = l
            .create_document(
                Some(inner),
                &content("Storm", "a dark and stormy night"),
                cx,
            )
            .unwrap()
            .id;
        clock.advance_ms(1_000);
        let d2 = l
            .create_document(None, &content("Calm", "a quiet morning"), cx)
            .unwrap()
            .id;
        (inner, d1, d2)
    });
    take(&events, cx);

    // Snapshot if stale: the first call takes one, the second does not.
    let minute = Duration::from_secs(60);
    let first = library.update(cx, |l, cx| l.snapshot_if_stale(d1, minute, cx).unwrap());
    assert!(first.is_some());
    assert_eq!(take(&events, cx), vec![LibraryEvent::SnapshotsChanged(d1)]);
    let second = library.update(cx, |l, cx| l.snapshot_if_stale(d1, minute, cx).unwrap());
    assert!(second.is_none());
    assert!(take(&events, cx).is_empty());
    // Old enough and changed: another one.
    clock.advance_ms(120_000);
    library.update(cx, |l, cx| {
        let changed = DocumentContent {
            content: r#"{"v":2}"#,
            ..content("Storm", "a dark and stormy night, again")
        };
        l.save_document(d1, &changed, cx).unwrap();
        assert!(l.snapshot_if_stale(d1, minute, cx).unwrap().is_some());
    });
    take(&events, cx);
    let snapshots = library.read_with(cx, |l, _| l.snapshots(d1).unwrap());
    assert_eq!(snapshots.len(), 2);
    let manual = library.update(cx, |l, cx| l.create_snapshot(d1, cx).unwrap());
    assert_eq!(
        library.read_with(cx, |l, _| l.snapshot(manual.id).unwrap().content),
        r#"{"v":2}"#
    );
    library.update(cx, |l, cx| l.delete_snapshot(manual.id, cx).unwrap());
    assert_eq!(
        take(&events, cx),
        vec![
            LibraryEvent::SnapshotsChanged(d1),
            LibraryEvent::SnapshotsChanged(d1)
        ]
    );

    library.read_with(cx, |l, _| {
        let hits = l.search("storm", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].document_id, d1);
        assert!(l.search("", 10).unwrap().is_empty());
        assert!(l.search("\"(*", 10).is_ok());

        // d1 was saved last.
        let recent = l.recent_documents(1).unwrap();
        assert_eq!(recent[0].id, d1);

        let all = l.all_documents().unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].meta.id, d1);
        assert_eq!(all[0].folder_path, vec!["Novel", "Part one"]);
        assert_eq!(all[0].folder_label(), "Novel / Part one");
        assert_eq!(all[1].meta.id, d2);
        assert!(all[1].folder_path.is_empty());
        assert_eq!(all[0].meta.folder_id, Some(inner));
    });
}

#[gpui::test]
fn a_content_save_updates_the_cached_tree_in_place(cx: &mut TestAppContext) {
    let clock = Clock::manual(1_700_000_000_000);
    let (library, events) = library_from(Library::in_memory_with_clock(clock.clone()).unwrap(), cx);
    let (folder, nested, at_root, sibling) = library.update(cx, |l, cx| {
        let folder = l.create_folder(None, "Book", cx).unwrap().id;
        let inner = l.create_folder(Some(folder), "Part one", cx).unwrap().id;
        let nested = l
            .create_document(Some(inner), &content("Chapter", "one two"), cx)
            .unwrap()
            .id;
        let sibling = l
            .create_document(Some(inner), &content("Other chapter", "x"), cx)
            .unwrap()
            .id;
        let at_root = l
            .create_document(None, &content("Notes", "x"), cx)
            .unwrap()
            .id;
        (folder, nested, at_root, sibling)
    });
    take(&events, cx);
    // The address of the cached tree, without keeping it alive: a reload makes a new tree,
    // an update in place keeps this one.
    let address = |cx: &mut TestAppContext| {
        library.read_with(cx, |l, _| std::sync::Arc::as_ptr(&l.tree()) as usize)
    };
    let cached = address(cx);

    // An autosave: same title, more words, a later time.
    clock.advance_ms(60_000);
    for id in [nested, at_root] {
        let saved = library.update(cx, |l, cx| {
            let saved = l
                .save_document(id, &content_of(id, nested, "one two three four"), cx)
                .unwrap();
            // Fresh inside the same update, as after every write.
            assert_eq!(l.tree().find_document(id), Some(&saved));
            saved
        });
        assert_eq!(saved.word_count, 4);
        assert_eq!(saved.updated_at, 1_700_000_060_000);
        assert_eq!(
            take(&events, cx),
            vec![LibraryEvent::DocumentSaved {
                id,
                title_changed: false
            }]
        );
    }
    assert_eq!(address(cx), cached, "a content save reloaded the tree");
    // A title that changes with the text is taken in place too; views are told.
    library.update(cx, |l, cx| {
        l.save_document(
            nested,
            &content("Chapter one", "one two three four five"),
            cx,
        )
        .unwrap()
    });
    assert_eq!(
        take(&events, cx),
        vec![
            LibraryEvent::DocumentSaved {
                id: nested,
                title_changed: true
            },
            LibraryEvent::TreeChanged
        ]
    );
    assert_eq!(address(cx), cached, "a content save reloaded the tree");
    // What is cached is exactly what a reload would read, and nothing else moved.
    library.read_with(cx, |l, _| {
        let tree = l.tree();
        assert_eq!(*tree, l.store().library_tree().unwrap());
        let chapters = tree.find_folder(folder).unwrap().folders[0]
            .documents
            .iter()
            .map(|d| (d.id, d.title.as_str(), d.word_count))
            .collect::<Vec<_>>();
        assert_eq!(
            chapters,
            vec![(nested, "Chapter one", 5), (sibling, "Other chapter", 1)]
        );
    });

    // Someone who took the tree before a save keeps what they took.
    let before = library.read_with(cx, |l, _| l.tree());
    library.update(cx, |l, cx| {
        l.save_document(at_root, &content("Notes", "a b c d e f"), cx)
            .unwrap()
    });
    assert_eq!(before.find_document(at_root).unwrap().word_count, 4);
    assert_eq!(
        library.read_with(cx, |l, _| l
            .tree()
            .find_document(at_root)
            .unwrap()
            .word_count),
        6
    );
    drop(before);
    take(&events, cx);

    // A trashed document is saved (an autosave racing a delete loses no text) and stays
    // out of the tree; its title changing is not a change of the tree.
    library.update(cx, |l, cx| l.trash_document(at_root, cx).unwrap());
    take(&events, cx);
    let tree = library.read_with(cx, |l, _| l.tree());
    library.update(cx, |l, cx| {
        l.save_document(at_root, &content("Last words", "kept"), cx)
            .unwrap()
    });
    assert_eq!(
        take(&events, cx),
        vec![LibraryEvent::DocumentSaved {
            id: at_root,
            title_changed: true
        }]
    );
    library.read_with(cx, |l, _| {
        assert_eq!(*l.tree(), *tree);
        assert_eq!(l.document(at_root).unwrap().meta.title, "Last words");
    });
    // Restored, it shows what was saved while it was in the trash.
    library.update(cx, |l, cx| l.restore_document(at_root, cx).unwrap());
    assert_eq!(
        library.read_with(cx, |l, _| l
            .tree()
            .find_document(at_root)
            .unwrap()
            .title
            .clone()),
        "Last words"
    );
    take(&events, cx);

    // A document that does not exist: an error, no event, nothing changed.
    let result = library.update(cx, |l, cx| {
        l.save_document(DocumentId(4040), &content("x", "x"), cx)
    });
    assert!(matches!(result, Err(StoreError::NotFound { .. })));
    assert!(take(&events, cx).is_empty());
}

/// The content of a test save: the nested document keeps its title, the other one its own.
fn content_of(id: DocumentId, nested: DocumentId, text: &str) -> DocumentContent<'_> {
    content(if id == nested { "Chapter" } else { "Notes" }, text)
}

#[gpui::test]
fn duplicating_a_folder_is_one_change(cx: &mut TestAppContext) {
    let (library, events) = library(cx);
    let (book, part) = library.update(cx, |l, cx| {
        let book = l.create_folder(None, "Book", cx).unwrap().id;
        let part = l.create_folder(Some(book), "Part one", cx).unwrap().id;
        l.create_document(Some(part), &content("Chapter", "harbour"), cx)
            .unwrap();
        l.create_document(Some(book), &content("Outline", "x"), cx)
            .unwrap();
        (book, part)
    });
    take(&events, cx);
    let copy = library.update(cx, |l, cx| {
        let copy = l.duplicate_folder(book, cx).unwrap();
        // The tree is fresh inside the same update.
        assert!(l.tree().find_folder(copy.id).is_some());
        copy
    });
    assert_eq!(copy.name, "Book copy");
    assert_eq!(
        take(&events, cx),
        vec![
            LibraryEvent::FolderCreated(copy.id),
            LibraryEvent::TreeChanged
        ]
    );
    library.read_with(cx, |l, _| {
        let tree = l.tree();
        assert_eq!(
            tree.folders
                .iter()
                .map(|n| n.folder.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Book", "Book copy"]
        );
        let copied = tree.find_folder(copy.id).unwrap();
        assert_eq!(copied.documents[0].title, "Outline");
        assert_eq!(copied.folders[0].folder.name, "Part one");
        assert_ne!(copied.folders[0].folder.id, part);
        assert_eq!(copied.folders[0].documents[0].title, "Chapter");
        assert_eq!(
            l.folder_path(Some(copied.folders[0].folder.id)),
            vec!["Book copy", "Part one"]
        );
        // Quick open and search see the copies.
        assert_eq!(l.all_documents().unwrap().len(), 4);
        assert_eq!(l.search("harbour", 10).unwrap().len(), 2);
    });
    // A folder that is not live cannot be duplicated, and nothing is announced.
    library.update(cx, |l, cx| l.trash_folder(copy.id, cx).unwrap());
    take(&events, cx);
    let result = library.update(cx, |l, cx| l.duplicate_folder(copy.id, cx));
    assert!(matches!(result, Err(StoreError::NotFound { .. })));
    assert!(take(&events, cx).is_empty());
    // Trashing a folder still reports every document that went with it.
    library.update(cx, |l, cx| l.trash_folder(book, cx).unwrap());
    let trashed = take(&events, cx);
    assert!(matches!(
        &trashed[0],
        LibraryEvent::FolderTrashed { id, documents } if *id == book && documents.len() == 2
    ));
}

// ----- the panel's text helpers ------------------------------------------------------------

#[test]
fn names_and_paths_are_shortened_for_one_line_messages() {
    use innerword_library::internals::{short, short_path};
    assert_eq!(short("Chapter", 26), "Chapter");
    assert_eq!(short("exactly ten", 11), "exactly ten");
    assert_eq!(short("A very long working title", 12), "A very long…");
    // No space is left dangling in front of the ellipsis, and characters are not bytes.
    assert_eq!(short("A very long working title", 7), "A very…");
    assert_eq!(short("Grüße aus Köln und Zürich", 10), "Grüße aus…");
    assert_eq!(short("書きかけの原稿について", 5), "書きかけ…");

    let path = |names: &[&str]| names.iter().map(|n| n.to_string()).collect::<Vec<_>>();
    let salt = path(&["The Salt Road", "Part one", "Scenes by the harbour"]);
    assert_eq!(
        short_path(&salt, 60),
        "The Salt Road / Part one / Scenes by the harbour"
    );
    // Too long as a whole: the end is kept, as many folders as fit.
    assert_eq!(
        short_path(&salt, 40),
        "… / Part one / Scenes by the harbour"
    );
    assert_eq!(short_path(&salt, 30), "… / Scenes by the harbour");
    // Even the last folder is too long: the start of its name.
    assert_eq!(short_path(&salt, 16), "… / Scenes by t…");
    assert_eq!(short_path(&path(&["Blog"]), 30), "Blog");
    assert_eq!(short_path(&[], 30), "");
    for max in 8..70 {
        let shown = short_path(&salt, max);
        assert!(shown.chars().count() <= max, "{shown:?} is over {max}");
    }
}

#[test]
fn a_match_at_the_end_of_a_long_name_is_brought_into_view() {
    use innerword_library::internals::label_around_match;
    let label = "A paragraph about the sound the radiator makes when the heating comes on";
    let at = label.find("radiator").unwrap();
    let positions: Vec<usize> = (at..at + "radiator".len()).collect();
    // The row shows about 20 characters: "radiator" starts at character 32.
    let (shown, moved) = label_around_match(label, &positions, 20).unwrap();
    assert_eq!(shown, "…the radiator makes when the heating comes on");
    // The positions still mark the same letters.
    let marked: String = moved
        .iter()
        .map(|&p| shown[p..].chars().next().unwrap())
        .collect();
    assert_eq!(marked, "radiator");
    // A wide row shows the match anyway: nothing changes.
    assert_eq!(label_around_match(label, &positions, 60), None);
    // So does a match at the start, and a label without a match.
    assert_eq!(label_around_match(label, &[0, 2, 3], 10), None);
    assert_eq!(label_around_match(label, &[], 10), None);
    // Two words of the filter: the first one decides, the second keeps its mark if it is
    // still in the text.
    let makes = label.find("makes").unwrap();
    let mut both = positions.clone();
    both.extend(makes..makes + 5);
    let (shown, moved) = label_around_match(label, &both, 20).unwrap();
    let marked: String = moved
        .iter()
        .map(|&p| shown[p..].chars().next().unwrap())
        .collect();
    assert_eq!(marked, "radiatormakes");
    // No word boundary near the match (one long word), and characters of several bytes.
    let label = "Donaudampfschifffahrtsgesellschaftskapitänsmütze";
    let at = label.find("mütze").unwrap();
    let positions: Vec<usize> = label[at..]
        .char_indices()
        .map(|(offset, _)| at + offset)
        .collect();
    let (shown, moved) = label_around_match(label, &positions, 16).unwrap();
    assert!(
        shown.starts_with('…') && shown.ends_with("mütze"),
        "{shown}"
    );
    assert!(shown.chars().count() <= 16, "{shown}");
    let marked: String = moved
        .iter()
        .map(|&p| shown[p..].chars().next().unwrap())
        .collect();
    assert_eq!(marked, "mütze");
    // A row with no room at all does not panic.
    assert!(label_around_match(label, &positions, 0).is_some());
}

#[gpui::test]
fn a_save_through_another_connection_is_taken_in_place(cx: &mut TestAppContext) {
    use innerword_library::store::{OpenOptions, Store};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.db");
    let clock = Clock::manual(1_700_000_000_000);
    let options = || OpenOptions {
        clock: clock.clone(),
        daily_backup: false,
        ..OpenOptions::default()
    };
    let store = Store::open_with(&path, options()).unwrap();
    let (library, events) = library_from(Library::from_store(store).unwrap(), cx);
    let (folder, doc, other) = library.update(cx, |l, cx| {
        let folder = l.create_folder(None, "Book", cx).unwrap().id;
        let doc = l
            .create_document(Some(folder), &content("Chapter", "one two"), cx)
            .unwrap()
            .id;
        let other = l
            .create_document(None, &content("Notes", "x"), cx)
            .unwrap()
            .id;
        (folder, doc, other)
    });
    take(&events, cx);
    // The app's background autosave: a second connection to the same file.
    let mut background = Store::open_with(&path, options()).unwrap();
    let address = |cx: &mut TestAppContext| {
        library.read_with(cx, |l, _| std::sync::Arc::as_ptr(&l.tree()) as usize)
    };
    let cached = address(cx);

    // Same title: the entry is replaced in place and only DocumentSaved is emitted.
    clock.advance_ms(2_000);
    let saved = background
        .save_document(doc, &content("Chapter", "one two three four"))
        .unwrap();
    library.update(cx, |l, cx| l.document_saved_elsewhere(&saved, cx).unwrap());
    assert_eq!(
        take(&events, cx),
        vec![LibraryEvent::DocumentSaved {
            id: doc,
            title_changed: false
        }]
    );
    assert_eq!(address(cx), cached, "the tree was reloaded");
    library.read_with(cx, |l, _| {
        assert_eq!(l.tree().find_document(doc), Some(&saved));
        assert_eq!(*l.tree(), l.store().library_tree().unwrap());
    });
    // A new title: the tree's views are told.
    clock.advance_ms(2_000);
    let saved = background
        .save_document(doc, &content("Chapter one", "one two three four"))
        .unwrap();
    library.update(cx, |l, cx| l.document_saved_elsewhere(&saved, cx).unwrap());
    assert_eq!(
        take(&events, cx),
        vec![
            LibraryEvent::DocumentSaved {
                id: doc,
                title_changed: true
            },
            LibraryEvent::TreeChanged
        ]
    );
    assert_eq!(address(cx), cached, "the tree was reloaded");
    // The library's own next save compares with what it now knows.
    library.update(cx, |l, cx| {
        l.save_document(doc, &content("Chapter one", "five words in this text"), cx)
            .unwrap()
    });
    assert_eq!(
        take(&events, cx),
        vec![LibraryEvent::DocumentSaved {
            id: doc,
            title_changed: false
        }]
    );

    // The document was renamed here while a save with the old title was on its way: the
    // late note must not put the old title back into the cache.
    clock.advance_ms(2_000);
    let late = background
        .save_document(doc, &content("Chapter one", "six words in this text now"))
        .unwrap();
    clock.advance_ms(2_000);
    library.update(cx, |l, cx| {
        l.rename_document(doc, "Renamed here", cx).unwrap()
    });
    take(&events, cx);
    library.update(cx, |l, cx| l.document_saved_elsewhere(&late, cx).unwrap());
    library.read_with(cx, |l, _| {
        let tree = l.tree();
        assert_eq!(*tree, l.store().library_tree().unwrap());
        let now = tree.find_document(doc).unwrap();
        assert_eq!((now.title.as_str(), now.word_count), ("Renamed here", 6));
    });
    assert_eq!(
        take(&events, cx),
        vec![LibraryEvent::DocumentSaved {
            id: doc,
            title_changed: false
        }]
    );

    // The document was moved while the save was on its way: the note still leaves the
    // cache equal to the database (by reading it again).
    clock.advance_ms(2_000);
    let saved = background
        .save_document(other, &content("Notes", "a b c"))
        .unwrap();
    library.update(cx, |l, cx| {
        l.move_document(other, Some(folder), Some(0), cx).unwrap();
        l.document_saved_elsewhere(&saved, cx).unwrap();
    });
    library.read_with(cx, |l, _| {
        let tree = l.tree();
        assert_eq!(*tree, l.store().library_tree().unwrap());
        let moved = tree.find_document(other).unwrap();
        assert_eq!((moved.folder_id, moved.word_count), (Some(folder), 3));
    });
    take(&events, cx);

    // A trashed document: saved (no text lost), not in the tree, nothing else changes.
    library.update(cx, |l, cx| l.trash_document(other, cx).unwrap());
    take(&events, cx);
    let before = library.read_with(cx, |l, _| l.tree());
    let saved = background
        .save_document(other, &content("Last words", "kept"))
        .unwrap();
    assert!(saved.trashed_at.is_some());
    library.update(cx, |l, cx| l.document_saved_elsewhere(&saved, cx).unwrap());
    assert_eq!(
        take(&events, cx),
        vec![LibraryEvent::DocumentSaved {
            id: other,
            title_changed: false
        }]
    );
    library.read_with(cx, |l, _| {
        assert_eq!(*l.tree(), *before);
        assert_eq!(l.document(other).unwrap().meta.title, "Last words");
    });
}

#[gpui::test]
fn changes_made_in_the_store_from_outside_reach_the_tree_and_the_subscribers(
    cx: &mut TestAppContext,
) {
    use innerword_library::OutsideChange;
    let (library, events) = library(cx);
    let (folder, kept, inside) = library.update(cx, |l, cx| {
        let folder = l.create_folder(None, "Novel", cx).unwrap().id;
        let kept = l
            .create_document(None, &content("Kept", "a b"), cx)
            .unwrap();
        let inside = l
            .create_document(Some(folder), &content("Inside", "c"), cx)
            .unwrap();
        (folder, kept.id, inside.id)
    });
    take(&events, cx);

    // Nothing changed: nothing is emitted, the result comes back.
    let (answer, refreshed) = library.update(cx, |l, cx| {
        l.change_outside(|store| (store.documents().unwrap().len(), Vec::new()), cx)
    });
    assert_eq!(answer, 2);
    refreshed.unwrap();
    assert!(take(&events, cx).is_empty());

    // Created, written (with another title), renamed, moved: one tree reload, the events
    // of each in the order given, one TreeChanged at the end.
    let (created, refreshed) = library.update(cx, |l, cx| {
        l.change_outside(
            |store| {
                let created = store
                    .create_document(Some(folder), &content("By an agent", "x y z"))
                    .unwrap()
                    .id;
                let part = store.create_folder(Some(folder), "Part one").unwrap().id;
                store
                    .save_document(kept, &content("Kept and changed", "a b c d"))
                    .unwrap();
                store.rename_document(inside, "Renamed").unwrap();
                store.move_document(inside, Some(part)).unwrap();
                (
                    created,
                    vec![
                        OutsideChange::DocumentCreated(created),
                        OutsideChange::FolderCreated(part),
                        OutsideChange::DocumentSaved(kept),
                        OutsideChange::DocumentRenamed(inside),
                        OutsideChange::DocumentMoved(inside),
                    ],
                )
            },
            cx,
        )
    });
    refreshed.unwrap();
    let part = library.read_with(cx, |l, _| {
        let tree = l.tree();
        assert_eq!(*tree, l.store().library_tree().unwrap());
        assert_eq!(tree.find_document(kept).unwrap().word_count, 4);
        assert_eq!(tree.find_document(inside).unwrap().title, "Renamed");
        tree.folders[0].folders[0].folder.id
    });
    assert_eq!(
        take(&events, cx),
        vec![
            LibraryEvent::DocumentCreated(created),
            LibraryEvent::FolderCreated(part),
            LibraryEvent::DocumentSaved {
                id: kept,
                title_changed: true
            },
            LibraryEvent::DocumentRenamed(inside),
            LibraryEvent::DocumentMoved(inside),
            LibraryEvent::TreeChanged,
        ]
    );

    // A save that keeps the title is a save and nothing else.
    library
        .update(cx, |l, cx| {
            l.change_outside(
                |store| {
                    store
                        .save_document(kept, &content("Kept and changed", "a"))
                        .unwrap();
                    ((), vec![OutsideChange::DocumentSaved(kept)])
                },
                cx,
            )
        })
        .1
        .unwrap();
    assert_eq!(
        take(&events, cx),
        vec![LibraryEvent::DocumentSaved {
            id: kept,
            title_changed: false
        }]
    );
    assert_eq!(
        library.read_with(cx, |l, _| l.tree().find_document(kept).unwrap().word_count),
        1
    );

    // A folder goes to the trash: the documents that went with it are named, from the
    // tree as it was. A version kept and a setting written are passed on.
    library
        .update(cx, |l, cx| {
            l.change_outside(
                |store| {
                    store.trash_folder(folder).unwrap();
                    store.set_setting("some.key", &1).unwrap();
                    (
                        (),
                        vec![
                            OutsideChange::FolderTrashed(folder),
                            OutsideChange::SnapshotsChanged(kept),
                            OutsideChange::SettingChanged("some.key".into()),
                        ],
                    )
                },
                cx,
            )
        })
        .1
        .unwrap();
    let seen = take(&events, cx);
    assert_eq!(seen.len(), 5, "{seen:?}");
    match &seen[0] {
        LibraryEvent::FolderTrashed { id, documents } => {
            assert_eq!(*id, folder);
            let mut documents = documents.clone();
            documents.sort();
            let mut expected = vec![created, inside];
            expected.sort();
            assert_eq!(documents, expected);
        }
        other => panic!("expected FolderTrashed, got {other:?}"),
    }
    assert_eq!(
        seen[1..],
        [
            LibraryEvent::SnapshotsChanged(kept),
            LibraryEvent::SettingChanged {
                key: "some.key".into()
            },
            LibraryEvent::TrashChanged,
            LibraryEvent::TreeChanged,
        ]
    );
    assert!(library.read_with(cx, |l, _| l.tree().folders.is_empty()));

    // Restored, and a live document deleted for good: whoever shows it hears that it
    // left the tree before it hears that it is gone.
    library
        .update(cx, |l, cx| {
            l.change_outside(
                |store| {
                    store.restore_folder(folder).unwrap();
                    store.trash_document(kept).unwrap();
                    store.purge_document(kept).unwrap();
                    (
                        (),
                        vec![
                            OutsideChange::FolderRestored(folder),
                            OutsideChange::DocumentPurged(kept),
                        ],
                    )
                },
                cx,
            )
        })
        .1
        .unwrap();
    assert_eq!(
        take(&events, cx),
        vec![
            LibraryEvent::FolderRestored(folder),
            LibraryEvent::DocumentTrashed(kept),
            LibraryEvent::DocumentPurged(kept),
            LibraryEvent::TrashChanged,
            LibraryEvent::TreeChanged,
        ]
    );
    library.read_with(cx, |l, _| {
        assert_eq!(*l.tree(), l.store().library_tree().unwrap());
        assert!(l.tree().find_document(kept).is_none());
        assert!(l.tree().find_document(inside).is_some());
    });

    // A document in the trash that is written or deleted there: the trash listing
    // changed, the tree did not.
    library.update(cx, |l, cx| l.trash_document(inside, cx).unwrap());
    take(&events, cx);
    library
        .update(cx, |l, cx| {
            l.change_outside(
                |store| {
                    store
                        .save_document(inside, &content("Other name", "c"))
                        .unwrap();
                    ((), vec![OutsideChange::DocumentSaved(inside)])
                },
                cx,
            )
        })
        .1
        .unwrap();
    assert_eq!(
        take(&events, cx),
        vec![
            LibraryEvent::DocumentSaved {
                id: inside,
                title_changed: false
            },
            LibraryEvent::TrashChanged,
        ]
    );
    library
        .update(cx, |l, cx| {
            l.change_outside(
                |store| {
                    store.purge_document(inside).unwrap();
                    store.empty_trash().unwrap();
                    (
                        (),
                        vec![
                            OutsideChange::DocumentPurged(inside),
                            OutsideChange::TrashEmptied,
                        ],
                    )
                },
                cx,
            )
        })
        .1
        .unwrap();
    assert_eq!(
        take(&events, cx),
        vec![
            LibraryEvent::DocumentPurged(inside),
            LibraryEvent::TrashChanged,
        ]
    );
}

/// The bytes of an image are stored through the library and nobody is told: no event,
/// no notification. The reads go through the store.
#[gpui::test]
fn storing_an_image_emits_nothing_and_the_bytes_are_read_through_the_store(
    cx: &mut TestAppContext,
) {
    fn picture(bytes: &[u8]) -> NewImage<'_> {
        NewImage {
            bytes,
            format: ImageFormat::Png,
            width: 640,
            height: 480,
            density: 144,
        }
    }
    let (library, events) = library(cx);
    let notified = Rc::new(RefCell::new(0));
    cx.update(|cx| {
        let notified = notified.clone();
        cx.observe(&library, move |_, _| *notified.borrow_mut() += 1)
            .detach();
    });
    let bytes = b"\x89PNG the bytes of a picture".to_vec();

    let info = library.update(cx, |l, _| l.put_image(&picture(&bytes)).unwrap());
    assert_eq!(info.id, StoredImageId::of(&bytes));
    assert_eq!(
        (info.format, info.width, info.height, info.density),
        (ImageFormat::Png, 640, 480, 144)
    );
    assert_eq!(info.byte_len, bytes.len() as u64);
    assert!(take(&events, cx).is_empty());
    assert_eq!(*notified.borrow(), 0);

    // Read through the store, as every reader does.
    library.read_with(cx, |l, _| {
        assert_eq!(
            l.store().image_bytes(&info.id).unwrap(),
            Some(bytes.clone())
        );
        assert_eq!(l.store().image_info(&info.id).unwrap(), Some(info.clone()));
        assert_eq!(l.store().image_count().unwrap(), 1);
    });

    // The same bytes again are the same image; a touch finds it; neither says anything.
    let again = library.update(cx, |l, _| l.put_image(&picture(&bytes)).unwrap());
    assert_eq!(again, info);
    let touched = library.update(cx, |l, _| l.touch_image(&info.id).unwrap());
    assert_eq!(touched, Some(info.clone()));
    let unknown = StoredImageId::of(b"no such picture");
    assert_eq!(
        library.update(cx, |l, _| l.touch_image(&unknown).unwrap()),
        None
    );
    assert!(take(&events, cx).is_empty());
    assert_eq!(*notified.borrow(), 0);
    assert_eq!(
        library.read_with(cx, |l, _| l.store().image_count().unwrap()),
        1
    );
}

/// A library on disk keeps the bytes in a second file beside the database, made when the
/// first image is stored, and has them again when it is opened the next time.
#[gpui::test]
fn the_images_of_a_library_on_disk_are_beside_it(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.db");
    let images = dir.path().join("images.db");
    let bytes = b"GIF89a the bytes of another picture".to_vec();
    let id = {
        let (library, events) = library_from(Library::open(&path).unwrap(), cx);
        library.update(cx, |l, cx| {
            l.create_document(None, &content("Words", "no picture yet"), cx)
                .unwrap()
        });
        assert!(
            !images.exists(),
            "a library without images has no second file"
        );
        take(&events, cx);
        let info = library.update(cx, |l, _| {
            l.put_image(&NewImage {
                bytes: &bytes,
                format: ImageFormat::Gif,
                width: 10,
                height: 10,
                density: 72,
            })
            .unwrap()
        });
        assert!(images.exists());
        assert!(take(&events, cx).is_empty());
        info.id
    };
    cx.run_until_parked();
    let (library, _) = library_from(Library::open(&path).unwrap(), cx);
    library.read_with(cx, |l, _| {
        assert_eq!(l.store().image_bytes(&id).unwrap(), Some(bytes.clone()));
    });
}
