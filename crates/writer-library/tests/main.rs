//! Tests for the `Library` model (events, tree cache, errors). The panel's tests are in
//! `panel/mod.rs` (one test binary for the crate).

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{AppContext as _, Entity, TestAppContext};
use writer_library::{
    Clock, DocumentContent, DocumentId, FolderId, Library, LibraryEvent, StoreError, TrashEntry,
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
