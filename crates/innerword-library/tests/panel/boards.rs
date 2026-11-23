//! Whiteboards in the library: the model's `create_board` and `save_board` (the events of
//! a document, the compare-and-swap and the guards of the store), the whiteboard's icon on
//! its row (live, in an open folder, under the filter, in the trash, under a drag), and
//! "New Whiteboard": its items in the panel's menus once the app gave the scene of an
//! empty one, and the naming in place that opens it with the keyboard on its canvas.

use gpui::{Modifiers, MouseButton, TestAppContext, VisualTestContext, point, px};
use innerword_library::{
    BoardContent, ContentStamp, DocumentId, DocumentKind, FileId, FolderId, HEADER_HEIGHT, Library,
    LibraryEvent, LibraryPanelEvent, NewBoard, ROOT_ROW_HEIGHT, StoreError, item_icon,
};
use innerword_ui::{ContextMenu, IconName};

use super::LibraryItem::{Document, Folder};
use super::{Fixture, fixture};
use crate::{content, library, take};

/// The scene of an empty whiteboard, as the app hands it to the panel (the library never
/// reads a scene).
const EMPTY_SCENE: &str = r##"{"type":"excalidraw","version":2,"source":"inner-word","elements":[],"appState":{"viewBackgroundColor":"#ffffff","gridSize":20,"gridStep":5,"gridModeEnabled":false},"files":{}}"##;

/// A scene that names `files` as a whiteboard's pictures (the store reads nothing in it:
/// the ids it checks are given beside it).
fn scene(words: &str, files: &[FileId]) -> String {
    let entries: Vec<String> = files
        .iter()
        .map(|id| format!(r#""{id}":{{"mimeType":"image/png","file":"{id}","created":1}}"#))
        .collect();
    format!(
        r#"{{"type":"excalidraw","version":2,"source":"inner-word","elements":[{{"id":"t","type":"text","text":"{words}"}}],"appState":{{}},"files":{{{}}}}}"#,
        entries.join(",")
    )
}

fn board<'a>(scene: &'a str, files: &'a [FileId], text: &'a str) -> BoardContent<'a> {
    BoardContent {
        scene,
        files,
        plain_text: text,
        word_count: text.split_whitespace().count() as u64,
        char_count: text.chars().count() as u64,
    }
}

/// Makes a whiteboard titled `title` in `folder`.
fn create_board(
    library: &gpui::Entity<Library>,
    folder: Option<FolderId>,
    title: &str,
    cx: &mut VisualTestContext,
) -> DocumentId {
    library.update(cx, |l, cx| {
        l.create_board(folder, title, &board(EMPTY_SCENE, &[], ""), cx)
            .unwrap()
            .meta
            .id
    })
}

fn menu(cx: &mut VisualTestContext) -> Option<Vec<String>> {
    cx.update(|window, cx| ContextMenu::current(window, cx).map(|menu| menu.read(cx).labels()))
}

fn labels(titles: &[&str]) -> Option<Vec<String>> {
    Some(titles.iter().map(|title| title.to_string()).collect())
}

fn right_click(at: gpui::Point<gpui::Pixels>, cx: &mut VisualTestContext) {
    cx.simulate_mouse_down(at, MouseButton::Right, Modifiers::default());
    cx.simulate_mouse_up(at, MouseButton::Right, Modifiers::default());
    cx.run_until_parked();
}

/// Clicks the item titled `title` of the open menu, where it is drawn.
fn choose(title: &str, cx: &mut VisualTestContext) {
    let bounds = cx.update(|window, cx| {
        let menu = ContextMenu::current(window, cx).expect("a menu is open");
        let menu = menu.read(cx);
        let ix = menu
            .labels()
            .iter()
            .position(|label| label == title)
            .unwrap_or_else(|| panic!("no item {title:?} in {:?}", menu.labels()));
        menu.item_bounds(ix).expect("the menu is drawn")
    });
    cx.simulate_click(bounds.center(), Modifiers::default());
    cx.run_until_parked();
}

fn library_row() -> gpui::Point<gpui::Pixels> {
    point(px(120.), HEADER_HEIGHT + ROOT_ROW_HEIGHT / 2.)
}

// ----- the model ----------------------------------------------------------------------------

#[gpui::test]
fn create_board_emits_the_events_of_a_new_document(cx: &mut TestAppContext) {
    let (library, events) = library(cx);
    let folder = library.update(cx, |l, cx| l.create_folder(None, "Plans", cx).unwrap().id);
    take(&events, cx);

    // Storing the bytes of a picture tells nobody.
    let picture = library.update(cx, |l, _| l.put_file(b"a picture").unwrap());
    assert_eq!(take(&events, cx), vec![]);
    let files = [picture.id];
    let scene = scene("API Postgres", &files);
    let saved = library.update(cx, |l, cx| {
        let saved = l
            .create_board(
                Some(folder),
                "Architecture",
                &board(&scene, &files, "API\nPostgres"),
                cx,
            )
            .unwrap();
        // The cache is fresh inside the same update.
        let cached = l.tree().find_document(saved.meta.id).cloned().unwrap();
        assert_eq!(cached.kind, DocumentKind::Board);
        assert_eq!(cached.title, "Architecture");
        assert_eq!(cached.word_count, 2);
        saved
    });
    assert_eq!(saved.meta.kind, DocumentKind::Board);
    assert_eq!(saved.meta.folder_id, Some(folder));
    assert_eq!(saved.stamp, ContentStamp::of(&scene));
    assert_eq!(
        take(&events, cx),
        vec![
            LibraryEvent::DocumentCreated(saved.meta.id),
            LibraryEvent::TreeChanged
        ]
    );
    library.read_with(cx, |l, _| {
        assert_eq!(l.document(saved.meta.id).unwrap().content, scene);
        // The listing for quick open tells a whiteboard from a text.
        let entry = l
            .all_documents()
            .unwrap()
            .into_iter()
            .find(|entry| entry.meta.id == saved.meta.id)
            .unwrap();
        assert_eq!(entry.kind, DocumentKind::Board);
        assert_eq!(entry.folder_label(), "Plans");
    });

    // A picture the library does not have refuses the whiteboard: nothing made, nobody
    // told.
    let missing = FileId::of(b"never stored");
    let files = [missing];
    let bad = self::scene("x", &files);
    let err = library.update(cx, |l, cx| {
        l.create_board(None, "Other", &board(&bad, &files, "x"), cx)
            .unwrap_err()
    });
    assert!(
        matches!(&err, StoreError::FilesMissing { ids } if ids == &vec![missing]),
        "{err:?}"
    );
    // A folder that is not there: the same.
    let err = library.update(cx, |l, cx| {
        l.create_board(
            Some(FolderId(9_999)),
            "Lost",
            &board(EMPTY_SCENE, &[], ""),
            cx,
        )
        .unwrap_err()
    });
    assert!(matches!(err, StoreError::NotFound { .. }), "{err:?}");
    assert_eq!(take(&events, cx), vec![]);
    assert_eq!(
        library.read_with(cx, |l, _| l.store().boards().unwrap().len()),
        1
    );
}

#[gpui::test]
fn save_board_emits_a_save_and_never_changes_the_title(cx: &mut TestAppContext) {
    let (library, events) = library(cx);
    let first = library.update(cx, |l, cx| {
        l.create_board(None, "Flow", &board(EMPTY_SCENE, &[], ""), cx)
            .unwrap()
    });
    let id = first.meta.id;
    take(&events, cx);

    // A save that descends from the stored scene: DocumentSaved alone (a save never
    // changes the title, so the tree is not told), and the cached counts are fresh.
    let picture = library.update(cx, |l, _| l.put_file(b"a picture").unwrap());
    let files = [picture.id];
    let second = scene("one two three", &files);
    let saved = library.update(cx, |l, cx| {
        let saved = l
            .save_board(
                id,
                &board(&second, &files, "one two three"),
                Some(first.stamp),
                cx,
            )
            .unwrap();
        assert_eq!(l.tree().find_document(id).unwrap().word_count, 3);
        saved
    });
    assert_eq!(saved.stamp, ContentStamp::of(&second));
    assert_eq!(saved.meta.title, "Flow");
    assert_eq!(
        take(&events, cx),
        vec![LibraryEvent::DocumentSaved {
            id,
            title_changed: false
        }]
    );

    // A save that descends from a scene that is not the stored one any more (a late post
    // of the view after an agent's change): refused, nothing written, nobody told.
    let third = scene("late", &[]);
    let err = library.update(cx, |l, cx| {
        l.save_board(id, &board(&third, &[], "late"), Some(first.stamp), cx)
            .unwrap_err()
    });
    assert!(
        matches!(err, StoreError::Stale { stored, .. } if stored == saved.stamp),
        "{err:?}"
    );
    assert_eq!(take(&events, cx), vec![]);
    assert_eq!(
        library.read_with(cx, |l, _| l.document(id).unwrap().content),
        second
    );
    // Without a stamp to compare (an agent, under one turn of the main thread) it writes.
    library.update(cx, |l, cx| {
        l.save_board(id, &board(&third, &[], "late"), None, cx)
            .unwrap()
    });
    take(&events, cx);

    // A rename, and the next save keeps the new title.
    library.update(cx, |l, cx| l.rename_document(id, "Renamed", cx).unwrap());
    take(&events, cx);
    library.update(cx, |l, cx| {
        l.save_board(id, &board(&second, &files, "one two three"), None, cx)
            .unwrap()
    });
    assert_eq!(
        take(&events, cx),
        vec![LibraryEvent::DocumentSaved {
            id,
            title_changed: false
        }]
    );
    assert_eq!(
        library.read_with(cx, |l, _| l.tree().find_document(id).unwrap().title.clone()),
        "Renamed"
    );

    // A new picture the library does not have, a whiteboard saved over a text, a text or
    // a page saved over a whiteboard: refused, nothing changed, nobody told.
    let missing = FileId::of(b"never stored");
    let files = [picture.id, missing];
    let bad = scene("x", &files);
    let err = library.update(cx, |l, cx| {
        l.save_board(id, &board(&bad, &files, "x"), None, cx)
            .unwrap_err()
    });
    assert!(
        matches!(&err, StoreError::FilesMissing { ids } if ids == &vec![missing]),
        "{err:?}"
    );
    let text = library.update(cx, |l, cx| {
        l.create_document(None, &content("Text", "x"), cx)
            .unwrap()
            .id
    });
    take(&events, cx);
    let err = library.update(cx, |l, cx| {
        l.save_board(text, &board(EMPTY_SCENE, &[], ""), None, cx)
            .unwrap_err()
    });
    assert!(
        matches!(
            err,
            StoreError::WrongKind {
                kind: DocumentKind::Text,
                saving: DocumentKind::Board,
                ..
            }
        ),
        "{err:?}"
    );
    let err = library.update(cx, |l, cx| {
        l.save_document(id, &content("Renamed", "x"), cx)
            .unwrap_err()
    });
    assert!(
        matches!(
            err,
            StoreError::WrongKind {
                kind: DocumentKind::Board,
                saving: DocumentKind::Text,
                ..
            }
        ),
        "{err:?}"
    );
    assert_eq!(take(&events, cx), vec![]);
    assert_eq!(
        library.read_with(cx, |l, _| l.document(id).unwrap().content),
        second
    );

    // A whiteboard in the trash is still saved, and the tree is not told.
    library.update(cx, |l, cx| l.trash_document(id, cx).unwrap());
    take(&events, cx);
    library.update(cx, |l, cx| {
        l.save_board(id, &board(EMPTY_SCENE, &[], ""), None, cx)
            .unwrap()
    });
    assert_eq!(
        take(&events, cx),
        vec![LibraryEvent::DocumentSaved {
            id,
            title_changed: false
        }]
    );
}

// ----- the row ------------------------------------------------------------------------------

#[gpui::test]
fn a_whiteboard_row_has_its_icon(cx: &mut TestAppContext) {
    let (f, cx) = fixture(cx);
    let at_root = create_board(&f.library, None, "Architecture", cx);
    let in_folder = create_board(&f.library, Some(f.folder), "Flow", cx);
    cx.run_until_parked();
    let icon = |item, cx: &mut VisualTestContext| f.panel.read_with(cx, |p, _| p.item_icon(item));

    // The icon of each kind, for whatever else draws an item beside its name.
    assert_eq!(item_icon(DocumentKind::Text), IconName::Document);
    assert_eq!(item_icon(DocumentKind::Page), IconName::Globe);
    assert_eq!(item_icon(DocumentKind::Board), IconName::Whiteboard);

    assert_eq!(icon(Document(at_root), cx), Some(IconName::Whiteboard));
    assert_eq!(icon(Document(f.at_root), cx), Some(IconName::Document));
    // Not drawn while its folder is closed; in the open folder it is.
    assert_eq!(icon(Document(in_folder), cx), None);
    cx.simulate_keystrokes("down right");
    assert_eq!(icon(Folder(f.folder), cx), Some(IconName::FolderOpen));
    assert_eq!(icon(Document(in_folder), cx), Some(IconName::Whiteboard));
    assert_eq!(icon(Document(f.in_folder), cx), Some(IconName::Document));

    // The one open in the editor (marked as active) keeps it.
    f.panel
        .update(cx, |p, cx| p.set_active_document(Some(at_root), cx));
    cx.run_until_parked();
    assert_eq!(icon(Document(at_root), cx), Some(IconName::Whiteboard));

    // The name filter keeps it a whiteboard.
    cx.simulate_keystrokes("/");
    cx.simulate_input("archi");
    cx.run_until_parked();
    assert_eq!(f.visible(cx), vec![Document(at_root)]);
    assert_eq!(icon(Document(at_root), cx), Some(IconName::Whiteboard));
    cx.simulate_keystrokes("escape tab");
    cx.run_until_parked();

    // The ghost of a drag has it too.
    let row = f
        .visible(cx)
        .iter()
        .position(|item| *item == Document(at_root))
        .unwrap();
    let from = Fixture::row_point(row, 120.);
    cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(
        from + point(px(4.), px(3.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.simulate_mouse_move(
        from + point(px(10.), px(40.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("library-drag-ghost-whiteboard").is_some(),
        "the ghost of a dragged whiteboard has no whiteboard"
    );
    assert!(cx.debug_bounds("library-drag-ghost-document").is_none());
    cx.simulate_mouse_up(from, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();

    // And so does the trash.
    f.library
        .update(cx, |l, cx| l.trash_document(at_root, cx).unwrap());
    f.library
        .update(cx, |l, cx| l.trash_document(f.at_root, cx).unwrap());
    cx.run_until_parked();
    assert_eq!(icon(Document(at_root), cx), Some(IconName::Whiteboard));
    assert_eq!(icon(Document(f.at_root), cx), Some(IconName::Document));
}

// ----- New Whiteboard -----------------------------------------------------------------------

#[gpui::test]
fn the_new_whiteboard_items_are_there_only_when_the_app_makes_whiteboards(cx: &mut TestAppContext) {
    let (f, cx) = fixture(cx);
    f.panel.update(cx, |p, _| p.set_new_document_content("{}"));
    let book = Fixture::row_point(0, 120.);
    let plus = cx
        .debug_bounds("library-add")
        .expect("the + button is drawn")
        .center();

    // Without the scene of an empty whiteboard (the app has no page engine): no item,
    // and NewBoard is left to the app.
    assert!(!f.panel.read_with(cx, |p, _| p.makes_boards()));
    right_click(book, cx);
    assert_eq!(
        menu(cx),
        labels(&[
            "New Document Here",
            "New Folder Here",
            "-",
            "Rename",
            "Duplicate",
            "Move To…",
            "Expand All",
            "-",
            "Move to Trash"
        ])
    );
    cx.simulate_keystrokes("escape");
    right_click(library_row(), cx);
    assert_eq!(
        menu(cx),
        labels(&[
            "New Document",
            "New Folder",
            "-",
            "Expand All",
            "-",
            "Show Trash"
        ])
    );
    cx.simulate_keystrokes("escape");
    cx.simulate_click(plus, Modifiers::default());
    assert_eq!(menu(cx), labels(&["New Document", "New Folder"]));
    cx.simulate_keystrokes("escape");
    let boards = |cx: &mut VisualTestContext| {
        f.library
            .read_with(cx, |l, _| l.store().boards().unwrap().len())
    };
    cx.dispatch_action(NewBoard);
    cx.run_until_parked();
    assert_eq!(boards(cx), 0);
    assert_eq!(f.renaming(cx), None);

    // With it: one item more in each of the three menus, after New Folder.
    f.panel
        .update(cx, |p, _| p.set_new_board_content(EMPTY_SCENE));
    assert!(f.panel.read_with(cx, |p, _| p.makes_boards()));
    right_click(book, cx);
    assert_eq!(
        menu(cx),
        labels(&[
            "New Document Here",
            "New Folder Here",
            "New Whiteboard Here",
            "-",
            "Rename",
            "Duplicate",
            "Move To…",
            "Expand All",
            "-",
            "Move to Trash"
        ])
    );
    cx.simulate_keystrokes("escape");
    right_click(library_row(), cx);
    assert_eq!(
        menu(cx),
        labels(&[
            "New Document",
            "New Folder",
            "New Whiteboard",
            "-",
            "Expand All",
            "-",
            "Show Trash"
        ])
    );
    cx.simulate_keystrokes("escape");
    cx.simulate_click(plus, Modifiers::default());
    assert_eq!(
        menu(cx),
        labels(&["New Document", "New Folder", "New Whiteboard"])
    );
    cx.simulate_keystrokes("escape");
    assert_eq!(boards(cx), 0);
    assert!(f.toasts(cx).is_empty());
}

#[gpui::test]
fn new_whiteboard_names_in_place_and_opens_on_the_canvas(cx: &mut TestAppContext) {
    let (f, cx) = fixture(cx);
    f.panel
        .update(cx, |p, _| p.set_new_board_content(EMPTY_SCENE));
    let field_text = |cx: &mut VisualTestContext| {
        f.panel.read_with(cx, |p, cx| {
            p.rename_field()
                .map(|field| field.read(cx).text().to_string())
        })
    };
    let kind = |id: DocumentId, cx: &mut VisualTestContext| {
        f.library
            .read_with(cx, |l, _| l.document_meta(id).unwrap().kind)
    };

    // A folder row's "New Whiteboard Here": at the end of that folder, named in place.
    right_click(Fixture::row_point(0, 120.), cx);
    choose("New Whiteboard Here", cx);
    let Some(Document(named)) = f.renaming(cx) else {
        panic!("no whiteboard is being named");
    };
    assert_eq!(kind(named, cx), DocumentKind::Board);
    assert_eq!(f.documents(Some(f.folder), cx), vec![f.in_folder, named]);
    assert_eq!(f.selected(cx), Some(Document(named)));
    assert_eq!(field_text(cx).as_deref(), Some(""));
    assert_eq!(
        f.library
            .read_with(cx, |l, _| l.document(named).unwrap().content),
        EMPTY_SCENE
    );
    assert!(f.events().is_empty());
    // `enter` takes the name and opens it with the keyboard on its canvas.
    cx.simulate_input("Architecture");
    cx.simulate_keystrokes("enter");
    assert_eq!(f.renaming(cx), None);
    assert_eq!(f.title(named, cx), "Architecture");
    assert_eq!(
        f.events(),
        vec![LibraryPanelEvent::OpenDocument {
            id: named,
            focus_editor: true
        }]
    );
    // Naming it is no change to undo.
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(f.title(named, cx), "Architecture");
    f.toasts(cx);

    // The library's menu: at the top level. `escape` leaves it "Untitled" and opens it
    // all the same.
    right_click(library_row(), cx);
    choose("New Whiteboard", cx);
    let Some(Document(untitled)) = f.renaming(cx) else {
        panic!("no whiteboard is being named");
    };
    assert_eq!(f.folder_of(untitled, cx), None);
    assert_eq!(f.documents(None, cx), vec![f.at_root, untitled]);
    cx.simulate_input("Not this");
    cx.simulate_keystrokes("escape");
    assert_eq!(f.renaming(cx), None);
    assert_eq!(f.title(untitled, cx), "");
    assert_eq!(kind(untitled, cx), DocumentKind::Board);
    assert_eq!(
        f.events(),
        vec![LibraryPanelEvent::OpenDocument {
            id: untitled,
            focus_editor: true
        }]
    );

    // The action itself (the app's File > New Whiteboard calls it), with a document
    // selected: in that document's folder. When the keyboard goes elsewhere the typed name
    // is kept and it opens, leaving the keyboard where it went.
    f.panel
        .update(cx, |p, cx| p.select(Some(Document(f.in_folder)), cx));
    cx.dispatch_action(NewBoard);
    cx.run_until_parked();
    let Some(Document(blurred)) = f.renaming(cx) else {
        panic!("no whiteboard is being named");
    };
    assert_eq!(f.folder_of(blurred, cx), Some(f.folder));
    cx.simulate_input("Sketch");
    cx.update(|window, _| window.blur());
    cx.run_until_parked();
    assert_eq!(f.renaming(cx), None);
    assert_eq!(f.title(blurred, cx), "Sketch");
    assert_eq!(
        f.events(),
        vec![LibraryPanelEvent::OpenDocument {
            id: blurred,
            focus_editor: false
        }]
    );

    // A whiteboard that went to the trash while it was named is not opened.
    let panel = f.panel.clone();
    cx.update(|window, cx| window.focus(&gpui::Focusable::focus_handle(panel.read(cx), cx)));
    let trashed = f
        .panel
        .update_in(cx, |p, window, cx| p.new_board(window, cx))
        .expect("a whiteboard is made");
    f.library
        .update(cx, |l, cx| l.trash_document(trashed, cx).unwrap());
    cx.run_until_parked();
    f.panel.update(cx, |p, cx| p.cancel_rename(cx));
    assert!(f.events().is_empty());

    // A new document still behaves as before: escape keeps it and does not open it.
    f.panel.update(cx, |p, _| p.set_new_document_content("{}"));
    cx.simulate_keystrokes("n");
    assert!(matches!(f.renaming(cx), Some(Document(_))));
    cx.simulate_keystrokes("escape");
    assert!(f.events().is_empty());
    assert!(f.toasts(cx).is_empty());
    assert_eq!(
        f.library
            .read_with(cx, |l, _| l.store().boards().unwrap().len()),
        4
    );
}
