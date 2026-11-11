//! Everything the sidebar offers to the mouse alone: the "+" button, the "more" buttons,
//! right-click menus on rows, on the library and in the trash, the same menus from the
//! keyboard (`ctrl-enter`), the hook for the app's own menu items, and drag and drop.
//! Driven with simulated mouse events against an in-memory library.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    Focusable, Modifiers, MouseButton, Pixels, Point, TestAppContext, VisualTestContext, point, px,
};
use writer_library::{
    DocumentId, FolderId, HEADER_HEIGHT, LibraryMenuTarget, LibraryPanelEvent, ROOT_ROW_HEIGHT,
    ROW_HEIGHT, TRASH_ROW_HEIGHT,
};
use writer_ui::{ContextMenu, ContextMenuItem, KeyHint, MenuAnchor};

use super::LibraryItem::{Document, Folder};
use super::{Fixture, fixture};

fn none() -> Modifiers {
    Modifiers::default()
}

fn right_click(at: Point<Pixels>, cx: &mut VisualTestContext) {
    cx.simulate_mouse_down(at, MouseButton::Right, none());
    cx.simulate_mouse_up(at, MouseButton::Right, none());
}

/// The titles of the open context menu ("-" is a separator), or `None` when none is open.
fn menu(cx: &mut VisualTestContext) -> Option<Vec<String>> {
    cx.update(|window, cx| ContextMenu::current(window, cx).map(|menu| menu.read(cx).labels()))
}

fn labels(titles: &[&str]) -> Option<Vec<String>> {
    Some(titles.iter().map(|title| title.to_string()).collect())
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
    cx.simulate_click(bounds.center(), none());
    cx.run_until_parked();
}

/// Whether the item titled `title` of the open menu is greyed out.
fn disabled(title: &str, cx: &mut VisualTestContext) -> bool {
    cx.update(|window, cx| {
        let menu = ContextMenu::current(window, cx).expect("a menu is open");
        let menu = menu.read(cx);
        menu.items()
            .iter()
            .find(|item| item.title().as_ref() == title)
            .unwrap_or_else(|| panic!("no item {title:?}"))
            .is_disabled()
    })
}

/// The centre of a control that carries a debug selector.
fn control(selector: &'static str, cx: &mut VisualTestContext) -> Point<Pixels> {
    cx.debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn"))
        .center()
}

fn library_row() -> Point<Pixels> {
    point(px(120.), HEADER_HEIGHT + ROOT_ROW_HEIGHT / 2.)
}

fn trash_row(cx: &mut VisualTestContext) -> Point<Pixels> {
    let viewport = cx.update(|window, _| window.viewport_size());
    point(px(100.), viewport.height - px(16.))
}

/// Row `ix` of the trash view.
fn trash_item(ix: usize) -> Point<Pixels> {
    point(
        px(100.),
        HEADER_HEIGHT + px(4.) + TRASH_ROW_HEIGHT * ix as f32 + TRASH_ROW_HEIGHT / 2.,
    )
}

/// A point in tree row `ix`, `fraction` of the way down the row.
fn in_row(ix: usize, fraction: f32) -> Point<Pixels> {
    let middle = Fixture::row_point(ix, 120.);
    point(middle.x, middle.y + ROW_HEIGHT * (fraction - 0.5))
}

/// Presses the mouse at `from` and drags to `to` (the button stays down).
fn drag(from: Point<Pixels>, to: Point<Pixels>, cx: &mut VisualTestContext) {
    cx.simulate_mouse_down(from, MouseButton::Left, none());
    // The drag begins once the pointer has left the place of the press.
    cx.simulate_mouse_move(from + point(px(4.), px(3.)), MouseButton::Left, none());
    cx.simulate_mouse_move(to, MouseButton::Left, none());
}

fn release(at: Point<Pixels>, cx: &mut VisualTestContext) {
    cx.simulate_mouse_up(at, MouseButton::Left, none());
    cx.run_until_parked();
}

fn indicator(f: &Fixture, cx: &mut VisualTestContext) -> Option<String> {
    f.panel.read_with(cx, |p, _| p.drop_indicator())
}

fn folders(f: &Fixture, parent: Option<FolderId>, cx: &mut VisualTestContext) -> Vec<FolderId> {
    f.library.read_with(cx, |l, _| {
        let tree = l.tree();
        let nodes = match parent {
            Some(id) => &tree.find_folder(id).unwrap().folders,
            None => &tree.folders,
        };
        nodes.iter().map(|node| node.folder.id).collect()
    })
}

fn has_modal(f: &Fixture, cx: &mut VisualTestContext) -> bool {
    f.panel
        .read_with(cx, |p, cx| p.modal_layer().read(cx).has_modal())
}

/// The fixture with everything open: rows are Book, Notes, Idea, Chapter, Untitled.
fn open_fixture(cx: &mut TestAppContext) -> (Fixture, &mut VisualTestContext) {
    let (f, cx) = fixture(cx);
    f.panel.update(cx, |p, cx| {
        p.set_new_document_content("{}");
        p.reveal_document(f.in_inner, cx);
    });
    cx.run_until_parked();
    assert_eq!(
        f.visible(cx),
        vec![
            Folder(f.folder),
            Folder(f.inner),
            Document(f.in_inner),
            Document(f.in_folder),
            Document(f.at_root),
        ]
    );
    f.events();
    (f, cx)
}

// ----- the header ---------------------------------------------------------------------------

#[gpui::test]
fn the_plus_button_creates_documents_and_folders(cx: &mut TestAppContext) {
    let (f, cx) = open_fixture(cx);
    let plus = control("library-add", cx);

    // With a folder selected the new folder goes into it, and is named in place.
    f.panel
        .update(cx, |p, cx| p.select(Some(Folder(f.folder)), cx));
    cx.simulate_click(plus, none());
    assert_eq!(menu(cx), labels(&["New Document", "New Folder"]));
    // The click is the button's: the filter beside it did not take the keyboard.
    assert!(f.panel_focused(cx));
    choose("New Folder", cx);
    assert_eq!(menu(cx), None);
    let Some(Folder(created)) = f.renaming(cx) else {
        panic!("the new folder is not being named");
    };
    assert_eq!(
        f.library
            .read_with(cx, |l, _| l.folder(created).unwrap().parent_id),
        Some(f.folder)
    );
    cx.simulate_input("Drafts");
    cx.simulate_keystrokes("enter");
    assert_eq!(f.folder_name(created, cx), "Drafts");

    // With the "Library" row selected the new document goes to the top level; once it
    // has a name it is opened.
    cx.simulate_click(library_row(), none());
    cx.simulate_click(plus, none());
    choose("New Document", cx);
    let Some(Document(document)) = f.renaming(cx) else {
        panic!("the new document is not being named");
    };
    assert_eq!(f.folder_of(document, cx), None);
    cx.simulate_input("Preface");
    cx.simulate_keystrokes("enter");
    assert_eq!(f.title(document, cx), "Preface");
    assert_eq!(
        f.events(),
        vec![LibraryPanelEvent::OpenDocument {
            id: document,
            focus_editor: false
        }]
    );

    // Escape closes the menu and nothing else; so does a click somewhere else.
    let before = f.visible(cx);
    cx.simulate_click(plus, none());
    assert!(menu(cx).is_some());
    cx.simulate_keystrokes("escape");
    assert_eq!(menu(cx), None);
    cx.simulate_click(plus, none());
    cx.simulate_click(Fixture::row_point(2, 400.), none());
    assert_eq!(menu(cx), None);
    assert_eq!(f.visible(cx), before);
    assert!(f.events().is_empty());

    // The button says what it is when the pointer rests on it.
    cx.simulate_mouse_move(plus, None, none());
    cx.executor().advance_clock(Duration::from_millis(900));
    cx.run_until_parked();
    assert!(cx.debug_bounds("library-tip").is_some());
}

#[gpui::test]
fn the_empty_library_offers_its_two_beginnings_as_buttons(cx: &mut TestAppContext) {
    let (f, cx) = fixture(cx);
    f.panel.update(cx, |p, _| p.set_new_document_content("{}"));
    f.library.update(cx, |l, cx| {
        l.trash_folder(f.folder, cx).unwrap();
        l.trash_document(f.at_root, cx).unwrap();
    });
    cx.run_until_parked();
    assert!(f.visible(cx).is_empty());

    let new_folder = control("library-empty-new-folder", cx);
    cx.simulate_click(new_folder, none());
    cx.run_until_parked();
    let Some(Folder(folder)) = f.renaming(cx) else {
        panic!("no folder is being named");
    };
    cx.simulate_input("Essays");
    cx.simulate_keystrokes("enter");
    assert_eq!(f.folder_name(folder, cx), "Essays");

    // And with only the trashed things left again: a document.
    f.library
        .update(cx, |l, cx| l.trash_folder(folder, cx).unwrap());
    cx.run_until_parked();
    let new_document = control("library-empty-new-document", cx);
    cx.simulate_click(new_document, none());
    cx.run_until_parked();
    assert!(matches!(f.renaming(cx), Some(Document(_))));
}

// ----- row menus ----------------------------------------------------------------------------

#[gpui::test]
fn a_right_click_on_a_document_opens_its_menu(cx: &mut TestAppContext) {
    let (f, cx) = open_fixture(cx);
    let chapter = Fixture::row_point(3, 120.);

    // The row that was clicked is selected (not opened), the list has the keyboard.
    f.panel
        .update(cx, |p, cx| p.select(Some(Document(f.at_root)), cx));
    right_click(chapter, cx);
    assert_eq!(f.selected(cx), Some(Document(f.in_folder)));
    assert!(f.panel_focused(cx));
    assert!(f.events().is_empty());
    assert_eq!(
        menu(cx),
        labels(&[
            "Open",
            "-",
            "Rename",
            "Duplicate",
            "Move To…",
            "-",
            "Move to Trash"
        ])
    );

    // Every item is an action with a key in the list, and the menu shows that key.
    let focus = f.panel.read_with(cx, |p, cx| p.focus_handle(cx));
    let keys: Vec<(String, Option<String>)> = cx.update(|window, cx| {
        let menu = ContextMenu::current(window, cx).unwrap();
        menu.read(cx)
            .items()
            .iter()
            .filter_map(|item| Some((item.title().to_string(), item.action_ref()?)))
            .map(|(title, action)| {
                let hint = KeyHint::for_action_in(action, &focus, window);
                (title, hint.map(|hint| hint.text()))
            })
            .collect()
    });
    assert_eq!(keys.len(), 5);
    for (title, key) in &keys {
        assert!(key.is_some(), "{title} shows no key");
    }
    assert_eq!(keys[2], ("Duplicate".to_string(), Some("⌘D".to_string())));

    // Open: what enter does (the editor gets the keyboard).
    choose("Open", cx);
    assert_eq!(
        f.events(),
        vec![LibraryPanelEvent::OpenDocument {
            id: f.in_folder,
            focus_editor: true
        }]
    );

    // Rename: in place.
    right_click(chapter, cx);
    choose("Rename", cx);
    assert_eq!(f.renaming(cx), Some(Document(f.in_folder)));
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("Opening");
    cx.simulate_keystrokes("enter");
    assert_eq!(f.title(f.in_folder, cx), "Opening");

    // Move To…: the folder picker.
    right_click(chapter, cx);
    choose("Move To…", cx);
    assert!(has_modal(&f, cx));
    cx.simulate_keystrokes("escape");
    assert!(!has_modal(&f, cx));

    // Duplicate: the copy lands after the original and is selected.
    right_click(chapter, cx);
    choose("Duplicate", cx);
    let in_book = f.documents(Some(f.folder), cx);
    assert_eq!(in_book.len(), 2);
    assert_eq!(in_book[0], f.in_folder);
    assert_eq!(f.selected(cx), Some(Document(in_book[1])));

    // Move to Trash: with the toast that offers the undo key, which works here.
    right_click(chapter, cx);
    assert_eq!(f.selected(cx), Some(Document(f.in_folder)));
    f.toasts(cx);
    choose("Move to Trash", cx);
    assert_eq!(f.documents(Some(f.folder), cx), vec![in_book[1]]);
    assert_eq!(
        f.toasts(cx),
        vec!["Moved “Opening” to the trash. Press ⌘Z to undo."]
    );
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(f.documents(Some(f.folder), cx), in_book);

    // A right click on another row moves the menu there.
    right_click(chapter, cx);
    right_click(Fixture::row_point(2, 120.), cx);
    assert_eq!(f.selected(cx), Some(Document(f.in_inner)));
    assert!(menu(cx).is_some());
    cx.simulate_keystrokes("escape");
}

#[gpui::test]
fn a_right_click_on_a_folder_opens_its_menu(cx: &mut TestAppContext) {
    let (f, cx) = open_fixture(cx);
    let book = Fixture::row_point(0, 120.);

    // A right click selects the folder and leaves it as open as it was.
    right_click(book, cx);
    assert_eq!(f.selected(cx), Some(Folder(f.folder)));
    assert_eq!(f.visible(cx).len(), 5);
    assert_eq!(
        menu(cx),
        labels(&[
            "New Document Here",
            "New Folder Here",
            "-",
            "Rename",
            "Duplicate",
            "Move To…",
            "Collapse All",
            "-",
            "Move to Trash"
        ])
    );

    // Collapse All closes the folder and the folders in it; then the line reads the
    // other way round.
    choose("Collapse All", cx);
    assert_eq!(f.visible(cx), vec![Folder(f.folder), Document(f.at_root)]);
    assert!(!f.panel.read_with(cx, |p, _| p.is_expanded(f.inner)));
    right_click(book, cx);
    assert!(menu(cx).unwrap().contains(&"Expand All".to_string()));
    choose("Expand All", cx);
    assert_eq!(f.visible(cx).len(), 5);
    assert!(f.panel.read_with(cx, |p, _| p.is_expanded(f.inner)));

    // New Document Here / New Folder Here create inside the folder that was clicked,
    // whatever was selected before.
    f.panel
        .update(cx, |p, cx| p.select(Some(Document(f.at_root)), cx));
    right_click(book, cx);
    choose("New Document Here", cx);
    let Some(Document(document)) = f.renaming(cx) else {
        panic!("no document is being named");
    };
    assert_eq!(f.folder_of(document, cx), Some(f.folder));
    cx.simulate_input("Epilogue");
    cx.simulate_keystrokes("enter");
    f.panel
        .update(cx, |p, cx| p.select(Some(Document(f.at_root)), cx));
    right_click(book, cx);
    choose("New Folder Here", cx);
    let Some(Folder(folder)) = f.renaming(cx) else {
        panic!("no folder is being named");
    };
    assert_eq!(folders(&f, Some(f.folder), cx), vec![f.inner, folder]);
    cx.simulate_input("Cuttings");
    cx.simulate_keystrokes("enter");

    // A folder with nothing in it has nothing to expand: the line is there, greyed.
    let cuttings = f
        .visible(cx)
        .iter()
        .position(|item| *item == Folder(folder))
        .unwrap();
    right_click(Fixture::row_point(cuttings, 120.), cx);
    assert!(disabled("Expand All", cx));
    assert!(!disabled("Rename", cx));

    // Duplicate copies the folder with what is in it; Move to Trash takes it away.
    right_click(book, cx);
    choose("Duplicate", cx);
    assert_eq!(folders(&f, None, cx).len(), 2);
    f.toasts(cx);
    right_click(book, cx);
    choose("Move to Trash", cx);
    assert_eq!(folders(&f, None, cx).len(), 1);
    assert!(f.toasts(cx)[0].starts_with("Moved “Book” and 3 documents to the trash."));
}

#[gpui::test]
fn the_more_button_opens_the_menu_of_its_row(cx: &mut TestAppContext) {
    let (f, cx) = open_fixture(cx);

    // On the selected row the button is always there.
    f.panel
        .update(cx, |p, cx| p.select(Some(Document(f.at_root)), cx));
    cx.run_until_parked();
    let more = control("library-row-more-4", cx);
    cx.simulate_click(more, none());
    assert_eq!(menu(cx).unwrap()[0], "Open");
    assert_eq!(f.selected(cx), Some(Document(f.at_root)));
    // The click was the button's: the document was not opened by it.
    assert!(f.events().is_empty());
    cx.simulate_keystrokes("escape");

    // On another row it appears while the pointer is on the row; a click on it selects
    // that row and opens its menu.
    cx.simulate_mouse_move(Fixture::row_point(1, 120.), None, none());
    cx.run_until_parked();
    let more = control("library-row-more-1", cx);
    cx.simulate_mouse_move(more, None, none());
    cx.simulate_click(more, none());
    assert_eq!(f.selected(cx), Some(Folder(f.inner)));
    assert_eq!(menu(cx).unwrap()[0], "New Document Here");
    // It did not count as a click on the folder: the folder is as open as it was.
    assert_eq!(f.visible(cx).len(), 5);
    choose("Rename", cx);
    assert_eq!(f.renaming(cx), Some(Folder(f.inner)));
    cx.simulate_keystrokes("escape");
}

// ----- the library itself -------------------------------------------------------------------

#[gpui::test]
fn the_empty_area_and_the_library_row_have_the_menu_of_the_library(cx: &mut TestAppContext) {
    let (f, cx) = open_fixture(cx);
    let library_menu = labels(&[
        "New Document",
        "New Folder",
        "-",
        "Collapse All",
        "-",
        "Show Trash",
    ]);

    // Below the rows: the library is what was clicked, so it is selected.
    right_click(Fixture::row_point(9, 200.), cx);
    assert!(f.panel.read_with(cx, |p, _| p.is_root_row_selected()));
    assert!(f.panel_focused(cx));
    assert_eq!(menu(cx), library_menu);
    choose("New Folder", cx);
    let Some(Folder(folder)) = f.renaming(cx) else {
        panic!("no folder is being named");
    };
    assert_eq!(folders(&f, None, cx), vec![f.folder, folder]);
    cx.simulate_keystrokes("escape");

    // On the "Library" row: the same menu.
    f.panel
        .update(cx, |p, cx| p.select(Some(Document(f.in_inner)), cx));
    right_click(library_row(), cx);
    assert!(f.panel.read_with(cx, |p, _| p.is_root_row_selected()));
    assert_eq!(menu(cx), library_menu);
    choose("New Document", cx);
    let Some(Document(document)) = f.renaming(cx) else {
        panic!("no document is being named");
    };
    assert_eq!(f.folder_of(document, cx), None);
    cx.simulate_keystrokes("escape");

    // Collapse All there folds the whole library; Show Trash goes to the trash view.
    right_click(library_row(), cx);
    choose("Collapse All", cx);
    assert!(!f.panel.read_with(cx, |p, _| p.is_expanded(f.folder)));
    assert!(!f.panel.read_with(cx, |p, _| p.is_expanded(f.inner)));
    right_click(library_row(), cx);
    assert!(menu(cx).unwrap().contains(&"Expand All".to_string()));
    choose("Show Trash", cx);
    assert!(f.panel.read_with(cx, |p, _| p.is_showing_trash()));
}

// ----- the trash ----------------------------------------------------------------------------

#[gpui::test]
fn the_trash_has_menus_and_buttons(cx: &mut TestAppContext) {
    let (f, cx) = open_fixture(cx);

    // The "Trash" row under the tree: with nothing in the trash, emptying is greyed.
    let trash = trash_row(cx);
    right_click(trash, cx);
    assert!(f.panel.read_with(cx, |p, _| p.is_trash_row_selected()));
    assert_eq!(menu(cx), labels(&["Show Trash", "-", "Empty Trash…"]));
    assert!(disabled("Empty Trash…", cx));
    cx.simulate_keystrokes("escape");

    f.library.update(cx, |l, cx| {
        l.trash_document(f.at_root, cx).unwrap();
        l.trash_document(f.in_folder, cx).unwrap();
    });
    cx.run_until_parked();
    right_click(trash, cx);
    assert!(!disabled("Empty Trash…", cx));
    choose("Show Trash", cx);
    assert!(f.panel.read_with(cx, |p, _| p.is_showing_trash()));
    // Most recently trashed first.
    assert_eq!(
        f.panel.read_with(cx, |p, _| p.trash_items()),
        vec![Document(f.in_folder), Document(f.at_root)]
    );

    // An item: the row that was clicked is selected; Restore and Delete Forever.
    right_click(trash_item(1), cx);
    assert_eq!(
        f.panel.read_with(cx, |p, _| p.selected_trash_item()),
        Some(Document(f.at_root))
    );
    assert_eq!(menu(cx), labels(&["Restore", "-", "Delete Forever…"]));
    // Delete Forever asks first.
    choose("Delete Forever…", cx);
    assert!(has_modal(&f, cx));
    cx.simulate_keystrokes("escape");
    assert_eq!(f.panel.read_with(cx, |p, _| p.trash_items()).len(), 2);
    right_click(trash_item(1), cx);
    choose("Restore", cx);
    assert_eq!(f.documents(None, cx), vec![f.at_root]);
    assert_eq!(
        f.panel.read_with(cx, |p, _| p.trash_items()),
        vec![Document(f.in_folder)]
    );

    // The "more" button of an item opens the same menu.
    let more = control("library-trash-more-0", cx);
    cx.simulate_click(more, none());
    assert_eq!(menu(cx), labels(&["Restore", "-", "Delete Forever…"]));
    cx.simulate_keystrokes("escape");

    // Beside the items: the trash as a whole.
    right_click(trash_item(5), cx);
    assert_eq!(menu(cx), labels(&["Back to Library", "-", "Empty Trash…"]));
    choose("Empty Trash…", cx);
    assert!(has_modal(&f, cx));
    cx.simulate_keystrokes("escape");
    right_click(trash_item(5), cx);
    choose("Back to Library", cx);
    assert!(!f.panel.read_with(cx, |p, _| p.is_showing_trash()));

    // The buttons under the list do what their keys do.
    let trash = trash_row(cx);
    cx.simulate_click(trash, none());
    assert!(f.panel.read_with(cx, |p, _| p.is_showing_trash()));
    cx.simulate_click(trash_item(0), none());
    let delete = control("library-trash-delete", cx);
    cx.simulate_click(delete, none());
    cx.run_until_parked();
    assert!(has_modal(&f, cx));
    cx.simulate_keystrokes("escape");
    let restore = control("library-trash-restore", cx);
    cx.simulate_click(restore, none());
    cx.run_until_parked();
    assert_eq!(f.documents(Some(f.folder), cx), vec![f.in_folder]);
    assert!(f.panel.read_with(cx, |p, _| p.trash_items()).is_empty());
    // Nothing left: "Empty" only says so.
    f.toasts(cx);
    let empty = control("library-trash-empty", cx);
    cx.simulate_click(empty, none());
    cx.run_until_parked();
    assert_eq!(f.toasts(cx), vec!["The trash is already empty."]);
    let back = control("library-trash-back", cx);
    cx.simulate_click(back, none());
    assert!(!f.panel.read_with(cx, |p, _| p.is_showing_trash()));
}

// ----- the keyboard -------------------------------------------------------------------------

#[gpui::test]
fn ctrl_enter_opens_the_menu_of_the_selected_row(cx: &mut TestAppContext) {
    let (f, cx) = open_fixture(cx);

    // On a row: its menu, at the row, with the first item marked so enter chooses.
    f.panel
        .update(cx, |p, cx| p.select(Some(Document(f.in_folder)), cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("ctrl-enter");
    assert_eq!(menu(cx).unwrap()[0], "Open");
    let (anchor, marked) = cx.update(|window, cx| {
        let menu = ContextMenu::current(window, cx).unwrap();
        let menu = menu.read(cx);
        (menu.anchor(), menu.selected_index())
    });
    assert_eq!(marked, Some(0));
    let row = Fixture::row_point(3, 0.);
    assert_eq!(
        anchor,
        MenuAnchor::Point(point(px(24.), row.y + ROW_HEIGHT / 2.))
    );
    // The keys go to the menu while it is open: down does not move the selection.
    cx.simulate_keystrokes("down");
    assert_eq!(f.selected(cx), Some(Document(f.in_folder)));
    cx.simulate_keystrokes("enter");
    assert_eq!(menu(cx), None);
    assert_eq!(f.renaming(cx), Some(Document(f.in_folder)));
    cx.simulate_keystrokes("escape");
    assert!(f.panel_focused(cx));

    // Escape closes it and the list has the keyboard as before.
    cx.simulate_keystrokes("ctrl-enter");
    assert!(menu(cx).is_some());
    cx.simulate_keystrokes("escape");
    assert_eq!(menu(cx), None);
    cx.simulate_keystrokes("up");
    assert_eq!(f.selected(cx), Some(Document(f.in_inner)));

    // On the "Library" row and on the "Trash" row.
    cx.simulate_keystrokes("home up ctrl-enter");
    assert_eq!(menu(cx).unwrap()[0], "New Document");
    cx.simulate_keystrokes("escape end down ctrl-enter");
    assert_eq!(menu(cx), labels(&["Show Trash", "-", "Empty Trash…"]));
    cx.simulate_keystrokes("escape");

    // In the trash view: the selected item.
    f.library
        .update(cx, |l, cx| l.trash_document(f.at_root, cx).unwrap());
    cx.run_until_parked();
    cx.simulate_keystrokes("t ctrl-enter");
    assert_eq!(menu(cx), labels(&["Restore", "-", "Delete Forever…"]));
    cx.simulate_keystrokes("enter");
    assert_eq!(f.documents(None, cx), vec![f.at_root]);
    // With the trash empty: the trash as a whole.
    cx.simulate_keystrokes("ctrl-enter");
    assert_eq!(menu(cx), labels(&["Back to Library", "-", "Empty Trash…"]));
    cx.simulate_keystrokes("enter");
    assert!(!f.panel.read_with(cx, |p, _| p.is_showing_trash()));
}

// ----- the app's own items ------------------------------------------------------------------

#[gpui::test]
fn the_app_adds_its_items_to_a_menu(cx: &mut TestAppContext) {
    let (f, cx) = open_fixture(cx);
    let asked: Rc<RefCell<Vec<LibraryMenuTarget>>> = Rc::default();
    let exported: Rc<RefCell<Vec<DocumentId>>> = Rc::default();
    f.panel.update(cx, |p, _| {
        let asked = asked.clone();
        let exported = exported.clone();
        p.set_menu_extension(move |target, _window, _cx| {
            asked.borrow_mut().push(target);
            match target {
                LibraryMenuTarget::Document(id) => {
                    let exported = exported.clone();
                    vec![
                        ContextMenuItem::new("Export…", move |_, _| {
                            exported.borrow_mut().push(id)
                        }),
                        ContextMenuItem::new("Version History", |_, _| {}),
                    ]
                }
                LibraryMenuTarget::Folder(_) => vec![ContextMenuItem::new("Export…", |_, _| {})],
                _ => Vec::new(),
            }
        });
    });

    // On a document: after the panel's own items, before the destructive one.
    right_click(Fixture::row_point(3, 120.), cx);
    assert_eq!(
        menu(cx),
        labels(&[
            "Open",
            "-",
            "Rename",
            "Duplicate",
            "Move To…",
            "-",
            "Export…",
            "Version History",
            "-",
            "Move to Trash"
        ])
    );
    choose("Export…", cx);
    assert_eq!(*exported.borrow(), vec![f.in_folder]);

    // On a folder.
    right_click(Fixture::row_point(0, 120.), cx);
    let folder_menu = menu(cx).unwrap();
    assert_eq!(
        folder_menu[folder_menu.len() - 4..],
        ["-", "Export…", "-", "Move to Trash"]
    );
    cx.simulate_keystrokes("escape");

    // Where the app adds nothing the menu is as it was; the app is still asked, with
    // what the menu is on. The "+" menu is the panel's alone.
    right_click(library_row(), cx);
    assert_eq!(menu(cx).unwrap().len(), 6);
    cx.simulate_keystrokes("escape");
    let plus = control("library-add", cx);
    cx.simulate_click(plus, none());
    assert_eq!(menu(cx), labels(&["New Document", "New Folder"]));
    cx.simulate_keystrokes("escape");
    f.library
        .update(cx, |l, cx| l.trash_document(f.at_root, cx).unwrap());
    cx.run_until_parked();
    cx.simulate_keystrokes("t");
    right_click(trash_item(0), cx);
    right_click(trash_item(4), cx);
    assert_eq!(
        *asked.borrow(),
        vec![
            LibraryMenuTarget::Document(f.in_folder),
            LibraryMenuTarget::Folder(f.folder),
            LibraryMenuTarget::Library,
            LibraryMenuTarget::Trashed(Document(f.at_root)),
            LibraryMenuTarget::Trash,
        ]
    );
}

// ----- clicks -------------------------------------------------------------------------------

#[gpui::test]
fn a_double_click_on_a_folder_renames_it_and_leaves_it_as_it_was(cx: &mut TestAppContext) {
    let (f, cx) = open_fixture(cx);
    let book = Fixture::row_point(0, 120.);
    for click_count in [1, 2] {
        cx.simulate_event(gpui::MouseDownEvent {
            position: book,
            modifiers: none(),
            button: MouseButton::Left,
            click_count,
            first_mouse: false,
        });
        cx.simulate_event(gpui::MouseUpEvent {
            position: book,
            modifiers: none(),
            button: MouseButton::Left,
            click_count,
        });
    }
    assert_eq!(f.renaming(cx), Some(Folder(f.folder)));
    // The first click closed the folder; the second, being about the name, opened it again.
    assert!(f.panel.read_with(cx, |p, _| p.is_expanded(f.folder)));
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("Novel");
    cx.simulate_keystrokes("enter");
    assert_eq!(f.folder_name(f.folder, cx), "Novel");
    assert_eq!(f.visible(cx).len(), 5);

    // Under a filter a click on a folder selects it and folds nothing (everything that
    // matches is shown).
    f.panel.update(cx, |p, cx| p.set_filter("e", cx));
    cx.run_until_parked();
    let rows = f.visible(cx);
    cx.simulate_click(Fixture::row_point(0, 120.), none());
    assert_eq!(f.selected(cx), Some(rows[0]));
    assert_eq!(f.visible(cx), rows);
}

// ----- drag and drop ------------------------------------------------------------------------

/// The open fixture plus three documents at the top level. Rows: 0 Book, 1 Notes, 2 Idea,
/// 3 Chapter, 4 Untitled, 5 One, 6 Two, 7 Three.
fn drag_fixture(cx: &mut TestAppContext) -> (Fixture, [DocumentId; 3], &mut VisualTestContext) {
    let (f, cx) = open_fixture(cx);
    let one = f.create(None, "One", cx);
    let two = f.create(None, "Two", cx);
    let three = f.create(None, "Three", cx);
    cx.run_until_parked();
    assert_eq!(f.visible(cx).len(), 8);
    f.toasts(cx);
    (f, [one, two, three], cx)
}

#[gpui::test]
fn dragging_a_document_between_rows_reorders_it(cx: &mut TestAppContext) {
    let (f, [one, two, three], cx) = drag_fixture(cx);
    assert_eq!(f.documents(None, cx), vec![f.at_root, one, two, three]);

    // Three onto the upper half of One: a line above One, and that is where it lands.
    drag(in_row(7, 0.5), in_row(5, 0.2), cx);
    assert_eq!(
        indicator(&f, cx),
        Some("between in the library at 1".to_string())
    );
    release(in_row(5, 0.2), cx);
    assert_eq!(f.documents(None, cx), vec![f.at_root, three, one, two]);
    assert_eq!(indicator(&f, cx), None);
    // The dropped row is selected and the list has the keyboard: cmd-z takes it back.
    assert_eq!(f.selected(cx), Some(Document(three)));
    assert!(f.panel_focused(cx));
    // A reorder is not announced, like alt-up.
    assert!(f.toasts(cx).is_empty());
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(f.documents(None, cx), vec![f.at_root, one, two, three]);
    assert_eq!(f.toasts(cx), vec!["Moved “Three” back to its place."]);

    // One onto the lower half of Two: after Two.
    drag(in_row(5, 0.5), in_row(6, 0.8), cx);
    assert_eq!(
        indicator(&f, cx),
        Some("between in the library at 3".to_string())
    );
    release(in_row(6, 0.8), cx);
    assert_eq!(f.documents(None, cx), vec![f.at_root, two, one, three]);

    // Right beside itself is where it already is: no line, and a drop changes nothing.
    drag(in_row(6, 0.5), in_row(5, 0.8), cx);
    assert_eq!(indicator(&f, cx), None);
    drag(in_row(6, 0.5), in_row(7, 0.2), cx);
    assert_eq!(indicator(&f, cx), None);
    release(in_row(7, 0.2), cx);
    assert_eq!(f.documents(None, cx), vec![f.at_root, two, one, three]);
    // Nor does a press and release on the row itself without moving away.
    drag(in_row(6, 0.5), in_row(6, 0.7), cx);
    assert_eq!(indicator(&f, cx), None);
    release(in_row(6, 0.7), cx);
    assert_eq!(f.documents(None, cx), vec![f.at_root, two, one, three]);

    // Out of a folder to a place among the top-level documents: a move to that place,
    // announced, and undone to where it was.
    drag(in_row(3, 0.5), in_row(5, 0.2), cx);
    assert_eq!(
        indicator(&f, cx),
        Some("between in the library at 1".to_string())
    );
    release(in_row(5, 0.2), cx);
    assert_eq!(
        f.documents(None, cx),
        vec![f.at_root, f.in_folder, two, one, three]
    );
    assert_eq!(f.documents(Some(f.folder), cx), vec![]);
    assert_eq!(f.toasts(cx), vec!["Moved “Chapter” to the library root."]);
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(f.documents(Some(f.folder), cx), vec![f.in_folder]);
    assert_eq!(f.documents(None, cx), vec![f.at_root, two, one, three]);
}

#[gpui::test]
fn dragging_onto_a_folder_moves_into_it(cx: &mut TestAppContext) {
    let (f, [one, two, _three], cx) = drag_fixture(cx);

    // A document onto a folder row, anywhere on the row: into the folder.
    for fraction in [0.1, 0.5, 0.9] {
        drag(in_row(5, 0.5), in_row(1, fraction), cx);
        assert_eq!(
            indicator(&f, cx),
            Some(format!("into folder {}", f.inner.0))
        );
        // Let go outside the tree: nothing happens, and the mark is gone.
        release(point(px(120.), HEADER_HEIGHT / 2.), cx);
        assert_eq!(indicator(&f, cx), None);
        assert_eq!(f.folder_of(one, cx), None);
    }
    drag(in_row(5, 0.5), in_row(1, 0.5), cx);
    release(in_row(1, 0.5), cx);
    assert_eq!(f.documents(Some(f.inner), cx), vec![f.in_inner, one]);
    assert_eq!(f.selected(cx), Some(Document(one)));
    assert_eq!(f.toasts(cx), vec!["Moved “One” to Book / Notes."]);
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(f.folder_of(one, cx), None);
    assert_eq!(f.toasts(cx), vec!["Moved “One” back to the library root."]);

    // Onto the folder it is in already: nothing to do, so no mark.
    drag(in_row(3, 0.5), in_row(0, 0.5), cx);
    assert_eq!(indicator(&f, cx), None);
    release(in_row(0, 0.5), cx);
    assert_eq!(f.documents(Some(f.folder), cx), vec![f.in_folder]);

    // A drag that rests on a closed folder opens it after a moment.
    cx.simulate_click(Fixture::row_point(0, 16.), none());
    assert_eq!(f.visible(cx).len(), 5);
    // Rows now: 0 Book (closed), 1 Untitled, 2 One, 3 Two, 4 Three.
    drag(in_row(3, 0.5), in_row(0, 0.5), cx);
    assert_eq!(
        indicator(&f, cx),
        Some(format!("into folder {}", f.folder.0))
    );
    assert!(!f.panel.read_with(cx, |p, _| p.is_expanded(f.folder)));
    cx.executor().advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
    assert!(!f.panel.read_with(cx, |p, _| p.is_expanded(f.folder)));
    cx.executor().advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
    assert!(f.panel.read_with(cx, |p, _| p.is_expanded(f.folder)));
    assert_eq!(f.visible(cx).len(), 8);
    // Now the folder's contents are there to drop between: onto Chapter's upper half.
    cx.simulate_mouse_move(in_row(3, 0.2), MouseButton::Left, none());
    assert_eq!(
        indicator(&f, cx),
        Some(format!("between in folder {} at 0", f.folder.0))
    );
    release(in_row(3, 0.2), cx);
    assert_eq!(f.documents(Some(f.folder), cx), vec![two, f.in_folder]);

    // A drag that only passes over a closed folder does not open it.
    cx.simulate_click(Fixture::row_point(1, 30.), none());
    assert!(!f.panel.read_with(cx, |p, _| p.is_expanded(f.inner)));
    let last = f.visible(cx).len() - 1;
    drag(in_row(last, 0.5), in_row(1, 0.5), cx);
    cx.simulate_mouse_move(in_row(last - 1, 0.2), MouseButton::Left, none());
    cx.executor().advance_clock(Duration::from_millis(900));
    cx.run_until_parked();
    assert!(!f.panel.read_with(cx, |p, _| p.is_expanded(f.inner)));
    release(point(px(120.), HEADER_HEIGHT / 2.), cx);
}

#[gpui::test]
fn dragging_folders(cx: &mut TestAppContext) {
    let (f, _, cx) = drag_fixture(cx);
    let zed = f
        .library
        .update(cx, |l, cx| l.create_folder(None, "Zed", cx).unwrap().id);
    cx.run_until_parked();
    // Rows: 0 Book, 1 Notes, 2 Idea, 3 Chapter, 4 Zed, 5 Untitled, 6 One, 7 Two, 8 Three.
    assert_eq!(f.visible(cx)[4], Folder(zed));

    // A folder cannot go into itself or into a folder inside it, nor beside one inside
    // it: no mark anywhere on those rows, and a drop does nothing.
    for (row, fraction) in [(0, 0.5), (1, 0.1), (1, 0.5), (1, 0.9), (2, 0.5), (3, 0.5)] {
        drag(in_row(0, 0.5), in_row(row, fraction), cx);
        assert_eq!(indicator(&f, cx), None, "Book onto row {row} at {fraction}");
        release(in_row(row, fraction), cx);
    }
    assert_eq!(folders(&f, None, cx), vec![f.folder, zed]);
    assert_eq!(folders(&f, Some(f.folder), cx), vec![f.inner]);
    assert!(f.toasts(cx).is_empty());

    // The middle of a folder row: into it. Its upper edge: before it.
    drag(in_row(4, 0.5), in_row(0, 0.1), cx);
    assert_eq!(
        indicator(&f, cx),
        Some("between in the library at 0".to_string())
    );
    cx.simulate_mouse_move(in_row(0, 0.5), MouseButton::Left, none());
    assert_eq!(
        indicator(&f, cx),
        Some(format!("into folder {}", f.folder.0))
    );
    // The lower edge of an open folder is inside it.
    cx.simulate_mouse_move(in_row(0, 0.9), MouseButton::Left, none());
    assert_eq!(
        indicator(&f, cx),
        Some(format!("into folder {}", f.folder.0))
    );
    cx.simulate_mouse_move(in_row(0, 0.1), MouseButton::Left, none());
    release(in_row(0, 0.1), cx);
    assert_eq!(folders(&f, None, cx), vec![zed, f.folder]);
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(folders(&f, None, cx), vec![f.folder, zed]);
    f.toasts(cx);

    // Zed (closed, empty) after Notes: its lower edge is after it.
    drag(in_row(4, 0.5), in_row(1, 0.5), cx);
    release(in_row(1, 0.5), cx);
    assert_eq!(folders(&f, Some(f.inner), cx), vec![zed]);
    assert_eq!(f.toasts(cx), vec!["Moved “Zed” to Book / Notes."]);
    // Rows: 0 Book, 1 Notes, 2 Zed, 3 Idea, 4 Chapter, 5 Untitled, ...
    assert_eq!(f.visible(cx)[2], Folder(zed));
    drag(in_row(2, 0.5), in_row(1, 0.1), cx);
    assert_eq!(
        indicator(&f, cx),
        Some(format!("between in folder {} at 0", f.folder.0))
    );
    release(in_row(1, 0.1), cx);
    assert_eq!(folders(&f, Some(f.folder), cx), vec![zed, f.inner]);
    f.toasts(cx);

    // Over a document a folder goes to that document's folder (folders come first, so
    // there is no place among the documents): here the top level.
    // Rows: 0 Book, 1 Zed, 2 Notes, 3 Idea, 4 Chapter, 5 Untitled, 6 One, ...
    assert_eq!(f.visible(cx)[1], Folder(zed));
    drag(in_row(1, 0.5), in_row(6, 0.5), cx);
    assert_eq!(indicator(&f, cx), Some("into the library".to_string()));
    release(in_row(6, 0.5), cx);
    assert_eq!(folders(&f, None, cx), vec![f.folder, zed]);
    assert_eq!(f.toasts(cx), vec!["Moved “Zed” to the library root."]);
}

#[gpui::test]
fn dragging_to_the_top_level_and_to_the_trash(cx: &mut TestAppContext) {
    let (f, [one, _two, _three], cx) = drag_fixture(cx);

    // Onto the empty area under the rows: to the top level.
    drag(in_row(3, 0.5), in_row(12, 0.5), cx);
    assert_eq!(indicator(&f, cx), Some("into the library".to_string()));
    release(in_row(12, 0.5), cx);
    assert_eq!(f.folder_of(f.in_folder, cx), None);
    assert_eq!(f.toasts(cx), vec!["Moved “Chapter” to the library root."]);
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(f.folder_of(f.in_folder, cx), Some(f.folder));
    f.toasts(cx);

    // Onto the "Library" row: the same. A folder too.
    drag(in_row(1, 0.5), library_row(), cx);
    assert_eq!(indicator(&f, cx), Some("into the library".to_string()));
    release(library_row(), cx);
    assert_eq!(folders(&f, None, cx), vec![f.folder, f.inner]);
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(folders(&f, Some(f.folder), cx), vec![f.inner]);
    f.toasts(cx);

    // What is at the top level already has nowhere to go there.
    drag(in_row(5, 0.5), in_row(12, 0.5), cx);
    assert_eq!(indicator(&f, cx), None);
    drag(in_row(5, 0.5), library_row(), cx);
    assert_eq!(indicator(&f, cx), None);
    release(library_row(), cx);
    assert!(f.toasts(cx).is_empty());

    // Onto the "Trash" row: to the trash, with the undo that works where the keyboard
    // now is.
    let trash = trash_row(cx);
    drag(in_row(5, 0.5), trash, cx);
    assert_eq!(indicator(&f, cx), Some("trash".to_string()));
    release(trash, cx);
    assert_eq!(
        f.panel.read_with(cx, |p, _| p.trash_items()),
        vec![Document(one)]
    );
    assert_eq!(
        f.toasts(cx),
        vec!["Moved “One” to the trash. Press ⌘Z to undo."]
    );
    assert!(f.panel_focused(cx));
    cx.simulate_keystrokes("cmd-z");
    assert!(f.panel.read_with(cx, |p, _| p.trash_items()).is_empty());

    // The row being renamed is not dragged (the mouse selects text in its field).
    f.panel
        .update(cx, |p, cx| p.select(Some(Document(f.in_folder)), cx));
    cx.simulate_keystrokes("r");
    assert_eq!(f.renaming(cx), Some(Document(f.in_folder)));
    drag(in_row(3, 0.5), in_row(0, 0.5), cx);
    assert_eq!(indicator(&f, cx), None);
    release(in_row(0, 0.5), cx);
    assert_eq!(f.folder_of(f.in_folder, cx), Some(f.folder));
}

#[gpui::test]
fn dragging_under_a_filter_only_moves_into_folders(cx: &mut TestAppContext) {
    let (f, [one, two, _three], cx) = drag_fixture(cx);
    f.panel.update(cx, |p, cx| p.set_filter("o", cx));
    cx.run_until_parked();
    // Matches: Book, Notes (and so everything in them), One, Two.
    let rows = f.visible(cx);
    let row = |item| rows.iter().position(|row| *row == item).unwrap();

    // Between two matches is not a place in the full list: no line.
    drag(
        in_row(row(Document(two)), 0.5),
        in_row(row(Document(one)), 0.2),
        cx,
    );
    assert_eq!(indicator(&f, cx), None);
    // Over a document in another folder: into that folder.
    cx.simulate_mouse_move(
        in_row(row(Document(f.in_inner)), 0.5),
        MouseButton::Left,
        none(),
    );
    assert_eq!(
        indicator(&f, cx),
        Some(format!("into folder {}", f.inner.0))
    );
    cx.simulate_mouse_move(
        in_row(row(Folder(f.folder)), 0.5),
        MouseButton::Left,
        none(),
    );
    assert_eq!(
        indicator(&f, cx),
        Some(format!("into folder {}", f.folder.0))
    );
    release(in_row(row(Folder(f.folder)), 0.5), cx);
    assert_eq!(f.folder_of(two, cx), Some(f.folder));
}
