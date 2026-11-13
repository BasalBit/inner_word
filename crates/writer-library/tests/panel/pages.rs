//! Pages in the library: the model's `create_page` and `save_page` (the events of a
//! document, the guards of the store), the globe on a page's row, and files dropped on the
//! panel from outside the app (`LibraryPanelEvent::DropPaths` with the folder under them).

use std::path::PathBuf;

use gpui::{ExternalPaths, FileDropEvent, TestAppContext, VisualTestContext, point, px};
use writer_library::{
    DocumentId, DocumentKind, FileId, FolderId, HEADER_HEIGHT, Library, LibraryEvent,
    LibraryPanelEvent, PageContent, ROOT_ROW_HEIGHT, StoreError,
};
use writer_ui::IconName;

use super::LibraryItem::{Document, Folder};
use super::{Fixture, fixture};
use crate::{content, library, take};

/// A manifest that names `files`, the way a page's manifest names its files (the store
/// reads nothing in it but the `"file"` tokens).
fn manifest(files: &[FileId]) -> String {
    let entries: Vec<String> = files
        .iter()
        .map(|id| format!(r#"{{"path":"index.html","file":"{id}","type":"text/html"}}"#))
        .collect();
    format!(r#"{{"version":1,"files":[{}]}}"#, entries.join(","))
}

fn page<'a>(manifest: &'a str, title: &'a str, text: &'a str) -> PageContent<'a> {
    PageContent {
        manifest,
        title,
        plain_text: text,
        word_count: text.split_whitespace().count() as u64,
        char_count: text.chars().count() as u64,
        source_url: Some("https://claude.ai/artifact/abc"),
    }
}

/// Stores the bytes of a page's entry and creates the page in `folder`.
fn create_page(
    library: &gpui::Entity<Library>,
    folder: Option<FolderId>,
    title: &str,
    cx: &mut VisualTestContext,
) -> DocumentId {
    library.update(cx, |l, cx| {
        let file = l.put_file(format!("<h1>{title}</h1>").as_bytes()).unwrap();
        let manifest = manifest(&[file.id]);
        l.create_page(folder, &page(&manifest, title, "a page of words"), cx)
            .unwrap()
            .id
    })
}

#[gpui::test]
fn create_page_emits_the_events_of_a_new_document(cx: &mut TestAppContext) {
    let (library, events) = library(cx);
    let folder = library.update(cx, |l, cx| l.create_folder(None, "Pages", cx).unwrap().id);
    take(&events, cx);

    let file = library.update(cx, |l, _| l.put_file(b"<h1>Orbitarium</h1>").unwrap());
    // Storing the bytes tells nobody.
    assert_eq!(take(&events, cx), vec![]);
    let manifest = manifest(&[file.id]);
    let meta = library.update(cx, |l, cx| {
        let meta = l
            .create_page(
                Some(folder),
                &page(&manifest, "Orbitarium", "seventeen songs"),
                cx,
            )
            .unwrap();
        // The cache is fresh inside the same update.
        let cached = l.tree().find_document(meta.id).cloned().unwrap();
        assert_eq!(cached.kind, DocumentKind::Page);
        meta
    });
    assert_eq!(meta.kind, DocumentKind::Page);
    assert_eq!(meta.folder_id, Some(folder));
    assert_eq!(
        meta.source_url.as_deref(),
        Some("https://claude.ai/artifact/abc")
    );
    assert_eq!(
        take(&events, cx),
        vec![
            LibraryEvent::DocumentCreated(meta.id),
            LibraryEvent::TreeChanged
        ]
    );

    // The listing for quick open tells a page from a text.
    let text = library.update(cx, |l, cx| {
        l.create_document(None, &content("Notes", "x"), cx)
            .unwrap()
            .id
    });
    take(&events, cx);
    let kinds: Vec<(DocumentId, DocumentKind)> = library.read_with(cx, |l, _| {
        let mut kinds: Vec<_> = l
            .all_documents()
            .unwrap()
            .into_iter()
            .map(|entry| {
                assert_eq!(entry.kind, entry.meta.kind);
                (entry.meta.id, entry.kind)
            })
            .collect();
        kinds.sort_by_key(|(id, _)| id.0);
        kinds
    });
    assert_eq!(
        kinds,
        vec![(meta.id, DocumentKind::Page), (text, DocumentKind::Text)]
    );
    let entry = library.read_with(cx, |l, _| {
        l.all_documents()
            .unwrap()
            .into_iter()
            .find(|entry| entry.meta.id == meta.id)
            .unwrap()
    });
    assert_eq!(entry.folder_label(), "Pages");
}

#[gpui::test]
fn save_page_emits_the_events_of_a_save_and_keeps_its_guards(cx: &mut TestAppContext) {
    let (library, events) = library(cx);
    let first = library.update(cx, |l, _| l.put_file(b"one").unwrap());
    let first_manifest = manifest(&[first.id]);
    let id = library.update(cx, |l, cx| {
        l.create_page(None, &page(&first_manifest, "Page", "one"), cx)
            .unwrap()
            .id
    });
    take(&events, cx);

    // The same title: DocumentSaved alone, and the cached counts are fresh.
    let second = library.update(cx, |l, _| l.put_file(b"two").unwrap());
    let second_manifest = manifest(&[first.id, second.id]);
    library.update(cx, |l, cx| {
        l.save_page(id, &page(&second_manifest, "Page", "one two three"), cx)
            .unwrap();
        assert_eq!(l.tree().find_document(id).unwrap().word_count, 3);
    });
    assert_eq!(
        take(&events, cx),
        vec![LibraryEvent::DocumentSaved {
            id,
            title_changed: false
        }]
    );
    assert_eq!(
        library.read_with(cx, |l, _| l.document(id).unwrap().content),
        second_manifest
    );

    // Another title: the tree changed too.
    library.update(cx, |l, cx| {
        l.save_page(id, &page(&second_manifest, "Renamed page", "one"), cx)
            .unwrap()
    });
    assert_eq!(
        take(&events, cx),
        vec![
            LibraryEvent::DocumentSaved {
                id,
                title_changed: true
            },
            LibraryEvent::TreeChanged
        ]
    );
    assert_eq!(
        library.read_with(cx, |l, _| l.tree().find_document(id).unwrap().title.clone()),
        "Renamed page"
    );

    // A manifest that names a file the library does not have changes nothing and tells
    // nobody, and so does a page saved over a text, or a text over a page.
    let missing = FileId::of(b"never stored");
    let bad = manifest(&[first.id, missing]);
    let err = library.update(cx, |l, cx| {
        l.save_page(id, &page(&bad, "Other", "x"), cx).unwrap_err()
    });
    assert!(
        matches!(&err, StoreError::FilesMissing { ids } if ids == &vec![missing]),
        "{err:?}"
    );
    let err = library.update(cx, |l, cx| {
        l.create_page(None, &page(&bad, "Other", "x"), cx)
            .unwrap_err()
    });
    assert!(matches!(err, StoreError::FilesMissing { .. }), "{err:?}");
    let text = library.update(cx, |l, cx| {
        l.create_document(None, &content("Text", "x"), cx)
            .unwrap()
            .id
    });
    take(&events, cx);
    let err = library.update(cx, |l, cx| {
        l.save_page(text, &page(&second_manifest, "Text", "x"), cx)
            .unwrap_err()
    });
    assert!(
        matches!(
            err,
            StoreError::WrongKind {
                kind: DocumentKind::Text,
                ..
            }
        ),
        "{err:?}"
    );
    let err = library.update(cx, |l, cx| {
        l.save_document(id, &content("Renamed page", "x"), cx)
            .unwrap_err()
    });
    assert!(
        matches!(
            err,
            StoreError::WrongKind {
                kind: DocumentKind::Page,
                ..
            }
        ),
        "{err:?}"
    );
    assert_eq!(take(&events, cx), vec![]);
    library.read_with(cx, |l, _| {
        let meta = l.document_meta(id).unwrap();
        assert_eq!(meta.title, "Renamed page");
        assert_eq!(l.document(id).unwrap().content, second_manifest);
    });

    // A page in the trash is still saved, as a text is, and the tree is not told.
    library.update(cx, |l, cx| l.trash_document(id, cx).unwrap());
    take(&events, cx);
    library.update(cx, |l, cx| {
        l.save_page(id, &page(&first_manifest, "In the trash", "x"), cx)
            .unwrap()
    });
    assert_eq!(
        take(&events, cx),
        vec![LibraryEvent::DocumentSaved {
            id,
            title_changed: true
        }]
    );
}

#[gpui::test]
fn a_page_row_has_the_globe(cx: &mut TestAppContext) {
    let (f, cx) = fixture(cx);
    let at_root = create_page(&f.library, None, "Orbitarium", cx);
    let in_folder = create_page(&f.library, Some(f.folder), "Reading", cx);
    cx.run_until_parked();
    let icon = |item, cx: &mut VisualTestContext| f.panel.read_with(cx, |p, _| p.item_icon(item));

    assert_eq!(icon(Document(at_root), cx), Some(IconName::Globe));
    assert_eq!(icon(Document(f.at_root), cx), Some(IconName::Document));
    assert_eq!(icon(Folder(f.folder), cx), Some(IconName::Folder));
    // Not drawn while its folder is closed.
    assert_eq!(icon(Document(in_folder), cx), None);
    cx.simulate_keystrokes("down right");
    assert_eq!(icon(Folder(f.folder), cx), Some(IconName::FolderOpen));
    assert_eq!(icon(Document(in_folder), cx), Some(IconName::Globe));
    assert_eq!(icon(Document(f.in_folder), cx), Some(IconName::Document));

    // The name filter keeps it a page.
    cx.simulate_keystrokes("/");
    cx.simulate_input("orbit");
    cx.run_until_parked();
    assert_eq!(f.visible(cx), vec![Document(at_root)]);
    assert_eq!(icon(Document(at_root), cx), Some(IconName::Globe));
    cx.simulate_keystrokes("escape tab");

    // And so does the trash.
    f.library
        .update(cx, |l, cx| l.trash_document(at_root, cx).unwrap());
    f.library
        .update(cx, |l, cx| l.trash_document(f.at_root, cx).unwrap());
    cx.run_until_parked();
    assert_eq!(icon(Document(at_root), cx), Some(IconName::Globe));
    assert_eq!(icon(Document(f.at_root), cx), Some(IconName::Document));
}

/// A point on the panel well below the last row: the empty area.
fn empty_area() -> gpui::Point<gpui::Pixels> {
    Fixture::row_point(30, 120.)
}

fn library_row() -> gpui::Point<gpui::Pixels> {
    point(px(120.), HEADER_HEIGHT + ROOT_ROW_HEIGHT / 2.)
}

fn drop_at(
    f: &Fixture,
    paths: &[&str],
    at: gpui::Point<gpui::Pixels>,
    cx: &mut VisualTestContext,
) -> Vec<LibraryPanelEvent> {
    let paths: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
    f.panel.update(cx, |p, cx| p.files_dropped(paths, at, cx));
    cx.run_until_parked();
    f.events()
}

#[gpui::test]
fn a_drop_emits_drop_paths_with_the_folder_under_it(cx: &mut TestAppContext) {
    let (f, cx) = fixture(cx);
    let paths = ["/some/where/a.html", "/some/where/site"];
    let dropped = |folder: Option<FolderId>| {
        vec![LibraryPanelEvent::DropPaths {
            paths: paths.iter().map(PathBuf::from).collect(),
            folder,
        }]
    };
    // Rows: Book, the untitled document at the root.
    assert_eq!(f.visible(cx), vec![Folder(f.folder), Document(f.at_root)]);

    // On a folder row: that folder.
    assert_eq!(
        drop_at(&f, &paths, Fixture::row_point(0, 120.), cx),
        dropped(Some(f.folder))
    );
    // On a document at the top level, on the "Library" row, and on the empty area with
    // nothing selected: the top level.
    assert_eq!(
        drop_at(&f, &paths, Fixture::row_point(1, 120.), cx),
        dropped(None)
    );
    assert_eq!(drop_at(&f, &paths, library_row(), cx), dropped(None));
    assert_eq!(drop_at(&f, &paths, empty_area(), cx), dropped(None));

    // Open Book and select "Notes" in it: a document row in Book is Book's, the empty area
    // is the folder selected, and the "Library" row is still the top level.
    cx.simulate_keystrokes("down right down");
    cx.run_until_parked();
    assert_eq!(f.selected(cx), Some(Folder(f.inner)));
    assert_eq!(
        f.visible(cx),
        vec![
            Folder(f.folder),
            Folder(f.inner),
            Document(f.in_folder),
            Document(f.at_root)
        ]
    );
    assert_eq!(
        drop_at(&f, &paths, Fixture::row_point(2, 120.), cx),
        dropped(Some(f.folder))
    );
    assert_eq!(
        drop_at(&f, &paths, Fixture::row_point(1, 120.), cx),
        dropped(Some(f.inner))
    );
    assert_eq!(
        drop_at(&f, &paths, empty_area(), cx),
        dropped(Some(f.inner))
    );
    assert_eq!(drop_at(&f, &paths, library_row(), cx), dropped(None));

    // No paths, no event; and the panel took nothing itself.
    assert_eq!(drop_at(&f, &[], Fixture::row_point(0, 120.), cx), vec![]);
    assert_eq!(f.documents(Some(f.folder), cx), vec![f.in_folder]);
}

/// The listeners themselves, with the file drop GPUI makes of the system's: the place the
/// files would go is marked while they are over the panel, and the drop clears it. (GPUI's
/// `ExternalPaths` cannot be made with paths outside GPUI, so these files are none, and
/// a drop of none emits nothing; the paths are `a_drop_emits_drop_paths_...`'s.)
#[gpui::test]
fn files_dragged_over_the_panel_mark_where_they_would_go(cx: &mut TestAppContext) {
    let (f, cx) = fixture(cx);
    let indicator = |cx: &mut VisualTestContext| f.panel.read_with(cx, |p, _| p.drop_indicator());

    cx.simulate_event(FileDropEvent::Entered {
        position: Fixture::row_point(0, 120.),
        paths: ExternalPaths::default(),
    });
    cx.run_until_parked();
    assert_eq!(
        indicator(cx).as_deref(),
        Some(format!("into folder {}", f.folder.0).as_str())
    );
    cx.simulate_event(FileDropEvent::Pending {
        position: Fixture::row_point(1, 120.),
    });
    cx.run_until_parked();
    assert_eq!(indicator(cx).as_deref(), Some("into the library"));
    cx.simulate_event(FileDropEvent::Pending {
        position: Fixture::row_point(0, 120.),
    });
    cx.run_until_parked();
    // A folder the files rest on opens, as it does for a dragged row.
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    cx.run_until_parked();
    assert_eq!(f.visible(cx).len(), 4, "{:?}", f.visible(cx));

    cx.simulate_event(FileDropEvent::Submit {
        position: Fixture::row_point(0, 120.),
    });
    cx.run_until_parked();
    assert_eq!(indicator(cx), None);
    assert_eq!(f.events(), vec![]);

    // Files that leave the window leave no mark behind.
    cx.simulate_event(FileDropEvent::Entered {
        position: empty_area(),
        paths: ExternalPaths::default(),
    });
    cx.run_until_parked();
    assert_eq!(indicator(cx).as_deref(), Some("into the library"));
    cx.simulate_event(FileDropEvent::Exited);
    cx.run_until_parked();
    assert_eq!(indicator(cx), None);
    assert_eq!(f.events(), vec![]);
}
