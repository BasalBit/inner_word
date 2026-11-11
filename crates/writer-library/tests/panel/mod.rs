//! Headless tests of the `LibraryPanel`: every flow is driven with keystrokes (and a few
//! with the mouse) against an in-memory library.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    AppContext as _, Context, Entity, FocusHandle, Focusable, Modifiers, MouseButton,
    MouseDownEvent, MouseUpEvent, Render, TestAppContext, VisualTestContext, Window, div, point,
    prelude::*, px,
};
use writer_library::{
    Clock, DocumentId, FolderId, Library, LibraryItem, LibraryPanel, LibraryPanelEvent, RowDetail,
};
use writer_ui::{ConfirmDialog, ToastLayer};

use super::{content, library_from};
use LibraryItem::{Document, Folder};

const START: i64 = 1_700_000_000_000;

struct Fixture {
    library: Entity<Library>,
    panel: Entity<LibraryPanel>,
    events: Rc<RefCell<Vec<LibraryPanelEvent>>>,
    toasts: Entity<ToastLayer>,
    clock: Clock,
    /// "Book", at the root.
    folder: FolderId,
    /// "Notes", in "Book".
    inner: FolderId,
    /// "Chapter", in "Book".
    in_folder: DocumentId,
    /// "Idea", in "Notes".
    in_inner: DocumentId,
    /// An untitled document at the root.
    at_root: DocumentId,
}

impl Fixture {
    fn visible(&self, cx: &mut VisualTestContext) -> Vec<LibraryItem> {
        self.panel.read_with(cx, |p, _| p.visible_items())
    }

    fn selected(&self, cx: &mut VisualTestContext) -> Option<LibraryItem> {
        self.panel.read_with(cx, |p, _| p.selected())
    }

    fn renaming(&self, cx: &mut VisualTestContext) -> Option<LibraryItem> {
        self.panel.read_with(cx, |p, _| p.renaming())
    }

    /// The panel's events since the last call.
    fn events(&self) -> Vec<LibraryPanelEvent> {
        self.events.borrow_mut().drain(..).collect()
    }

    /// The messages of the toasts on screen; clears them.
    fn toasts(&self, cx: &mut VisualTestContext) -> Vec<String> {
        let messages = self.toasts.read_with(cx, |layer, _| {
            layer
                .toasts()
                .map(|toast| toast.message.to_string())
                .collect()
        });
        self.toasts.update(cx, |layer, cx| layer.clear(cx));
        messages
    }

    fn title(&self, id: DocumentId, cx: &mut VisualTestContext) -> String {
        self.library
            .read_with(cx, |l, _| l.document_meta(id).unwrap().title)
    }

    fn folder_name(&self, id: FolderId, cx: &mut VisualTestContext) -> String {
        self.library
            .read_with(cx, |l, _| l.folder(id).unwrap().name)
    }

    fn folder_of(&self, id: DocumentId, cx: &mut VisualTestContext) -> Option<FolderId> {
        self.library
            .read_with(cx, |l, _| l.document_meta(id).unwrap().folder_id)
    }

    /// The live documents directly in `folder`, in display order.
    fn documents(&self, folder: Option<FolderId>, cx: &mut VisualTestContext) -> Vec<DocumentId> {
        self.library.read_with(cx, |l, _| {
            l.documents_in_folder(folder)
                .unwrap()
                .into_iter()
                .map(|meta| meta.id)
                .collect()
        })
    }

    fn panel_focused(&self, cx: &mut VisualTestContext) -> bool {
        let panel = self.panel.clone();
        cx.update(|window, cx| panel.focus_handle(cx).is_focused(window))
    }

    fn create(
        &self,
        folder: Option<FolderId>,
        title: &str,
        cx: &mut VisualTestContext,
    ) -> DocumentId {
        self.library.update(cx, |l, cx| {
            l.create_document(folder, &content(title, "some words here"), cx)
                .unwrap()
                .id
        })
    }

    /// Where the middle of tree row `ix` is in the window, `x` pixels from the left.
    fn row_point(ix: usize, x: f32) -> gpui::Point<gpui::Pixels> {
        point(
            px(x),
            writer_library::HEADER_HEIGHT
                + px(4.)
                + writer_library::ROW_HEIGHT * ix as f32
                + writer_library::ROW_HEIGHT / 2.,
        )
    }
}

fn fixture(cx: &mut TestAppContext) -> (Fixture, &mut VisualTestContext) {
    cx.update(|cx| {
        writer_ui::theme::init(writer_ui::ThemeMode::Light, cx);
        writer_ui::init(cx);
        writer_library::init(cx);
    });
    let clock = Clock::manual(START);
    let (library, _) = library_from(Library::in_memory_with_clock(clock.clone()).unwrap(), cx);
    let (folder, inner, in_folder, in_inner, at_root) = library.update(cx, |l, cx| {
        let folder = l.create_folder(None, "Book", cx).unwrap().id;
        let inner = l.create_folder(Some(folder), "Notes", cx).unwrap().id;
        let in_folder = l
            .create_document(Some(folder), &content("Chapter", "x"), cx)
            .unwrap()
            .id;
        let in_inner = l
            .create_document(Some(inner), &content("Idea", "x"), cx)
            .unwrap()
            .id;
        let at_root = l.create_document(None, &content("", "x"), cx).unwrap().id;
        (folder, inner, in_folder, in_inner, at_root)
    });
    let toasts = cx.new(|_| ToastLayer::new());
    cx.update(|cx| ToastLayer::set_global(&toasts, cx));
    let events: Rc<RefCell<Vec<LibraryPanelEvent>>> = Rc::default();
    let (panel, cx) = cx.add_window_view({
        let library = library.clone();
        let events = events.clone();
        move |window, cx| {
            let panel = LibraryPanel::new(library, cx);
            cx.subscribe(&cx.entity(), move |_, _, event: &LibraryPanelEvent, _| {
                events.borrow_mut().push(event.clone())
            })
            .detach();
            window.focus(&panel.focus_handle(cx));
            panel
        }
    });
    cx.run_until_parked();
    (
        Fixture {
            library,
            panel,
            events,
            toasts,
            clock,
            folder,
            inner,
            in_folder,
            in_inner,
            at_root,
        },
        cx,
    )
}

#[gpui::test]
fn navigation_expand_collapse_and_open(cx: &mut TestAppContext) {
    let (f, cx) = fixture(cx);
    let target = |cx: &mut VisualTestContext| f.panel.read_with(cx, |p, _| p.selected_folder());

    // Folders start collapsed; nothing is selected, so new documents go to the root.
    assert_eq!(f.visible(cx), vec![Folder(f.folder), Document(f.at_root)]);
    assert_eq!(f.selected(cx), None);
    assert_eq!(target(cx), None);

    cx.simulate_keystrokes("down");
    assert_eq!(f.selected(cx), Some(Folder(f.folder)));
    assert_eq!(target(cx), Some(f.folder));
    cx.simulate_keystrokes("right");
    assert_eq!(
        f.visible(cx),
        vec![
            Folder(f.folder),
            Folder(f.inner),
            Document(f.in_folder),
            Document(f.at_root)
        ]
    );
    // right again steps into the folder; enter toggles a folder.
    cx.simulate_keystrokes("right");
    assert_eq!(f.selected(cx), Some(Folder(f.inner)));
    cx.simulate_keystrokes("enter");
    assert_eq!(f.visible(cx).len(), 5);
    cx.simulate_keystrokes("j");
    assert_eq!(f.selected(cx), Some(Document(f.in_inner)));
    assert_eq!(target(cx), Some(f.inner));

    // enter on a document asks the app to open it and focus the editor.
    cx.simulate_keystrokes("enter");
    assert_eq!(
        f.events(),
        vec![LibraryPanelEvent::OpenDocument {
            id: f.in_inner,
            focus_editor: true
        }]
    );

    // left steps out to the parent; down walks to the end, then onto the trash row.
    cx.simulate_keystrokes("left");
    assert_eq!(f.selected(cx), Some(Folder(f.inner)));
    cx.simulate_keystrokes("down down down");
    assert_eq!(f.selected(cx), Some(Document(f.at_root)));
    assert_eq!(target(cx), None);
    cx.simulate_keystrokes("down");
    assert_eq!(f.selected(cx), None);
    assert!(f.panel.read_with(cx, |p, _| p.is_trash_row_selected()));
    cx.simulate_keystrokes("down");
    assert!(f.panel.read_with(cx, |p, _| p.is_trash_row_selected()));
    cx.simulate_keystrokes("up");
    assert_eq!(f.selected(cx), Some(Document(f.at_root)));
    assert!(!f.panel.read_with(cx, |p, _| p.is_trash_row_selected()));
    cx.simulate_keystrokes("k");
    assert_eq!(f.selected(cx), Some(Document(f.in_folder)));

    cx.simulate_keystrokes("home");
    assert_eq!(f.selected(cx), Some(Folder(f.folder)));
    cx.simulate_keystrokes("end");
    assert_eq!(f.selected(cx), Some(Document(f.at_root)));
    cx.simulate_keystrokes("cmd-up");
    assert_eq!(f.selected(cx), Some(Folder(f.folder)));
    cx.simulate_keystrokes("cmd-down");
    assert_eq!(f.selected(cx), Some(Document(f.at_root)));
    // Vim's g g and G, and a page at a time (the whole list fits on one page here).
    cx.simulate_keystrokes("g g");
    assert_eq!(f.selected(cx), Some(Folder(f.folder)));
    cx.simulate_keystrokes("shift-g");
    assert_eq!(f.selected(cx), Some(Document(f.at_root)));
    cx.simulate_keystrokes("pageup");
    assert_eq!(f.selected(cx), Some(Folder(f.folder)));
    cx.simulate_keystrokes("pagedown");
    assert_eq!(f.selected(cx), Some(Document(f.at_root)));

    // Collapsing a folder never hides the selection: it moves to the folder.
    f.panel
        .update(cx, |p, cx| p.select(Some(Document(f.in_inner)), cx));
    cx.simulate_keystrokes("cmd-up left");
    assert_eq!(f.visible(cx), vec![Folder(f.folder), Document(f.at_root)]);
    assert_eq!(f.selected(cx), Some(Folder(f.folder)));
    // h and l are left and right.
    cx.simulate_keystrokes("l");
    assert_eq!(f.visible(cx).len(), 5);
    cx.simulate_keystrokes("l");
    assert_eq!(f.selected(cx), Some(Folder(f.inner)));
    cx.simulate_keystrokes("h");
    assert_eq!(f.visible(cx).len(), 4);
    cx.simulate_keystrokes("h");
    assert_eq!(f.selected(cx), Some(Folder(f.folder)));
    cx.simulate_keystrokes("h");
    assert_eq!(f.visible(cx), vec![Folder(f.folder), Document(f.at_root)]);

    assert!(f.events().is_empty());
    cx.simulate_keystrokes("escape");
    assert_eq!(f.events(), vec![LibraryPanelEvent::Dismissed]);

    // The expansion state is stored in the settings as it changes.
    let stored: Vec<FolderId> = f.library.read_with(cx, |l, _| {
        l.setting_or(writer_library::EXPANDED_FOLDERS_SETTING, Vec::new())
    });
    assert_eq!(stored, Vec::<FolderId>::new());
}

#[gpui::test]
fn every_command_has_a_key_in_the_panel_context(cx: &mut TestAppContext) {
    let (_f, cx) = fixture(cx);
    cx.update(|_, cx| {
        let commands = writer_library::commands();
        let titles: Vec<&str> = commands.iter().map(|c| c.title.as_ref()).collect();
        for expected in [
            "New document",
            "New folder",
            "Rename",
            "Duplicate document",
            "Move to folder",
            "Move to trash",
            "Show trash",
            "Empty trash",
            "Filter by name",
        ] {
            assert!(titles.contains(&expected), "{expected} is missing");
        }
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        for command in &commands {
            assert_eq!(command.category.as_ref(), "Library");
            let mut bindings = keymap.bindings_for_action(command.action_ref()).peekable();
            assert!(
                bindings.peek().is_some(),
                "{} has no binding",
                command.title
            );
            for binding in bindings {
                let predicate = binding
                    .predicate()
                    .map(|p| p.to_string())
                    .unwrap_or_default();
                assert!(
                    predicate.contains(writer_library::KEY_CONTEXT),
                    "{} is bound outside the panel's context",
                    command.title
                );
            }
        }
        // Hints show the canonical key, which is the binding added last.
        let hint = |command: &str| {
            commands
                .iter()
                .find(|c| c.title.as_ref() == command)
                .and_then(|c| c.key_hint(cx))
                .map(|hint| hint.text())
        };
        assert_eq!(hint("Rename").as_deref(), Some("R"));
        assert_eq!(hint("Filter by name").as_deref(), Some("/"));
        assert_eq!(hint("Move to trash").as_deref(), Some("⌘⌫"));
        assert_eq!(hint("New folder").as_deref(), Some("⇧N"));
    });
}

#[gpui::test]
fn reveal_and_follow_the_library(cx: &mut TestAppContext) {
    let (f, cx) = fixture(cx);

    // Reveal expands the ancestors and selects.
    assert!(
        f.panel
            .update(cx, |p, cx| p.reveal_document(f.in_inner, cx))
    );
    f.panel.read_with(cx, |p, _| {
        assert!(p.is_expanded(f.folder) && p.is_expanded(f.inner));
        assert_eq!(p.selected(), Some(Document(f.in_inner)));
        assert_eq!(p.selected_folder(), Some(f.inner));
        assert_eq!(p.visible_items().len(), 5);
    });
    assert!(
        !f.panel
            .update(cx, |p, cx| p.reveal_document(DocumentId(999), cx))
    );
    f.panel
        .update(cx, |p, cx| p.set_active_document(Some(f.in_inner), cx));

    // A new document shows up without any call to the panel.
    let new = f.create(Some(f.inner), "New", cx);
    cx.run_until_parked();
    assert!(f.visible(cx).contains(&Document(new)));

    // Trashing the selected document moves the selection to the row that takes its place.
    f.library
        .update(cx, |l, cx| l.trash_document(f.in_inner, cx).unwrap());
    cx.run_until_parked();
    f.panel.read_with(cx, |p, _| {
        assert_eq!(p.selected(), Some(Document(new)));
        assert_eq!(p.active_document(), None);
    });

    // Trashing a folder removes its rows; rename mode on a vanished item ends.
    assert!(f.panel.update_in(cx, |p, window, cx| p.begin_rename(
        Folder(f.inner),
        window,
        cx
    )));
    assert_eq!(f.renaming(cx), Some(Folder(f.inner)));
    f.library
        .update(cx, |l, cx| l.trash_folder(f.folder, cx).unwrap());
    cx.run_until_parked();
    f.panel.read_with(cx, |p, _| {
        assert_eq!(p.visible_items(), vec![Document(f.at_root)]);
        assert_eq!(p.renaming(), None);
        assert_eq!(p.selected(), Some(Document(f.at_root)));
        assert_eq!(p.selected_folder(), None);
    });
    // The keyboard is back in the list, not in the vanished text field.
    assert!(f.panel_focused(cx));
    let _ = f.in_folder;

    // A new panel over the same library restores the expansion state.
    f.library
        .update(cx, |l, cx| l.restore_folder(f.folder, cx).unwrap());
    let library = f.library.clone();
    let second = cx.new(|cx| LibraryPanel::new(library, cx));
    second.read_with(cx, |p, _| {
        assert!(p.is_expanded(f.folder) && p.is_expanded(f.inner))
    });
}

#[gpui::test]
fn selection_follows_the_open_document_and_persists(cx: &mut TestAppContext) {
    let (f, cx) = fixture(cx);

    // The editor shows a document deep in collapsed folders: the panel opens the way to it.
    f.panel
        .update(cx, |p, cx| p.set_active_document(Some(f.in_inner), cx));
    f.panel.read_with(cx, |p, _| {
        assert_eq!(p.active_document(), Some(f.in_inner));
        assert_eq!(p.selected(), Some(Document(f.in_inner)));
        assert!(p.is_expanded(f.folder) && p.is_expanded(f.inner));
    });
    // Moving the selection does not change what is open; telling the panel about the same
    // document again does not pull the selection back.
    cx.simulate_keystrokes("down");
    f.panel
        .update(cx, |p, cx| p.set_active_document(Some(f.in_inner), cx));
    assert_eq!(f.selected(cx), Some(Document(f.in_folder)));
    // Another document opens from elsewhere (quick open): the selection follows.
    f.panel
        .update(cx, |p, cx| p.set_active_document(Some(f.at_root), cx));
    assert_eq!(f.selected(cx), Some(Document(f.at_root)));
    // Closing the document keeps the selection.
    f.panel.update(cx, |p, cx| p.set_active_document(None, cx));
    assert_eq!(f.selected(cx), Some(Document(f.at_root)));
    // While a rename is in progress the selection stays with the field.
    f.panel.update_in(cx, |p, window, cx| {
        p.begin_rename(Folder(f.inner), window, cx)
    });
    f.panel
        .update(cx, |p, cx| p.set_active_document(Some(f.in_folder), cx));
    assert_eq!(f.selected(cx), Some(Folder(f.inner)));
    assert_eq!(f.renaming(cx), Some(Folder(f.inner)));
    cx.simulate_keystrokes("escape");
    assert_eq!(f.renaming(cx), None);

    // The selection is written to the settings a moment after it settles.
    cx.simulate_keystrokes("end up");
    assert_eq!(f.selected(cx), Some(Document(f.in_folder)));
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    let stored: Option<LibraryItem> = f.library.read_with(cx, |l, _| {
        l.setting_or(writer_library::SELECTED_ITEM_SETTING, None)
    });
    assert_eq!(stored, Some(Document(f.in_folder)));

    // A new panel (the next launch) comes back with the same selection and expansion.
    let library = f.library.clone();
    let second = cx.new(|cx| LibraryPanel::new(library, cx));
    second.read_with(cx, |p, _| {
        assert_eq!(p.selected(), Some(Document(f.in_folder)));
        assert!(p.is_expanded(f.folder) && p.is_expanded(f.inner));
        assert!(p.visible_items().contains(&Document(f.in_folder)));
    });

    // A stored selection that no longer exists is dropped.
    f.library
        .update(cx, |l, cx| l.trash_document(f.in_folder, cx).unwrap());
    f.library.update(cx, |l, cx| {
        l.set_setting(
            writer_library::SELECTED_ITEM_SETTING,
            &Some(Document(f.in_folder)),
            cx,
        )
        .unwrap()
    });
    let library = f.library.clone();
    let third = cx.new(|cx| LibraryPanel::new(library, cx));
    third.read_with(cx, |p, _| assert_eq!(p.selected(), None));
}

#[gpui::test]
fn focusing_the_panel_selects_a_row(cx: &mut TestAppContext) {
    let (f, cx) = fixture(cx);
    // Focus events are only delivered to an active window.
    cx.update(|window, _| {
        window.blur();
        window.activate_window();
    });
    cx.run_until_parked();
    assert_eq!(f.selected(cx), None);
    f.panel
        .update(cx, |p, cx| p.set_active_document(Some(f.at_root), cx));
    f.panel.update(cx, |p, cx| p.select(None, cx));
    let panel = f.panel.clone();
    cx.update(|window, cx| window.focus(&panel.focus_handle(cx)));
    cx.run_until_parked();
    // The open document if it is visible, else the first row.
    assert_eq!(f.selected(cx), Some(Document(f.at_root)));

    cx.update(|window, _| window.blur());
    cx.run_until_parked();
    f.panel.update(cx, |p, cx| {
        p.set_active_document(None, cx);
        p.select(None, cx)
    });
    cx.update(|window, cx| window.focus(&panel.focus_handle(cx)));
    cx.run_until_parked();
    assert_eq!(f.selected(cx), Some(Folder(f.folder)));
}

#[gpui::test]
fn create_and_rename_in_place(cx: &mut TestAppContext) {
    let (f, cx) = fixture(cx);
    let field_text = |cx: &mut VisualTestContext| {
        f.panel.read_with(cx, |p, cx| {
            p.rename_field()
                .map(|field| field.read(cx).text().to_string())
        })
    };

    // Until the app supplies the content of an empty document, `n` is left to the app.
    cx.simulate_keystrokes("down n");
    assert_eq!(f.renaming(cx), None);
    assert_eq!(f.documents(Some(f.folder), cx), vec![f.in_folder]);

    f.panel
        .update(cx, |p, _| p.set_new_document_content("{\"empty\":true}"));
    // n: a new document in the selected (collapsed) folder, renamed in place.
    cx.simulate_keystrokes("n");
    let Some(Document(new)) = f.renaming(cx) else {
        panic!("not renaming a document");
    };
    assert_eq!(f.documents(Some(f.folder), cx), vec![f.in_folder, new]);
    assert_eq!(f.selected(cx), Some(Document(new)));
    assert!(f.panel.read_with(cx, |p, _| p.is_expanded(f.folder)));
    assert_eq!(field_text(cx).as_deref(), Some(""));
    assert!(!f.panel_focused(cx));
    // Letters that are keys of the tree are plain text while the field has focus.
    cx.simulate_input("jkhl nrmti/ N");
    assert_eq!(field_text(cx).as_deref(), Some("jkhl nrmti/ N"));
    assert_eq!(f.selected(cx), Some(Document(new)));
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("Chapter two");
    assert!(f.events().is_empty());
    cx.simulate_keystrokes("enter");
    assert_eq!(f.renaming(cx), None);
    assert_eq!(f.title(new, cx), "Chapter two");
    assert_eq!(
        f.library
            .read_with(cx, |l, _| l.document(new).unwrap().content),
        "{\"empty\":true}"
    );
    // The new document is shown in the editor; the keyboard stays in the list.
    assert_eq!(
        f.events(),
        vec![LibraryPanelEvent::OpenDocument {
            id: new,
            focus_editor: false
        }]
    );
    assert!(f.panel_focused(cx));
    cx.simulate_keystrokes("enter");
    assert_eq!(
        f.events(),
        vec![LibraryPanelEvent::OpenDocument {
            id: new,
            focus_editor: true
        }]
    );

    // shift-n: a new folder next to the selected document, named in place.
    cx.simulate_keystrokes("shift-n");
    let Some(Folder(new_folder)) = f.renaming(cx) else {
        panic!("not renaming a folder");
    };
    assert_eq!(
        f.library
            .read_with(cx, |l, _| l.folder(new_folder).unwrap().parent_id),
        Some(f.folder)
    );
    // The placeholder name is selected, so typing replaces it.
    assert_eq!(field_text(cx).as_deref(), Some("New folder"));
    cx.simulate_input("Research");
    cx.simulate_keystrokes("enter");
    assert_eq!(f.folder_name(new_folder, cx), "Research");
    assert_eq!(f.selected(cx), Some(Folder(new_folder)));
    assert!(f.events().is_empty());

    // r renames the selection; escape cancels and does not leave the panel.
    cx.simulate_keystrokes("r");
    assert_eq!(f.renaming(cx), Some(Folder(new_folder)));
    assert_eq!(field_text(cx).as_deref(), Some("Research"));
    cx.simulate_input("Nope");
    cx.simulate_keystrokes("escape");
    assert_eq!(f.renaming(cx), None);
    assert_eq!(f.folder_name(new_folder, cx), "Research");
    assert!(f.events().is_empty());
    assert!(f.panel_focused(cx));

    // F2 renames too; an empty name keeps the old one.
    cx.simulate_keystrokes("f2 cmd-a backspace enter");
    assert_eq!(f.renaming(cx), None);
    assert_eq!(f.folder_name(new_folder, cx), "Research");

    // tab commits and returns to the list.
    cx.simulate_keystrokes("r");
    cx.simulate_input("Sources");
    cx.simulate_keystrokes("tab");
    assert_eq!(f.renaming(cx), None);
    assert_eq!(f.folder_name(new_folder, cx), "Sources");
    assert!(f.panel_focused(cx));

    // A new document that is left unnamed stays "Untitled", and is still opened.
    cx.simulate_keystrokes("n enter");
    let untitled = f.documents(Some(new_folder), cx)[0];
    assert_eq!(f.title(untitled, cx), "");
    assert_eq!(
        f.events(),
        vec![LibraryPanelEvent::OpenDocument {
            id: untitled,
            focus_editor: false
        }]
    );
    // Cancelling the name of a new document keeps the document and does not open it.
    cx.simulate_keystrokes("n");
    cx.simulate_input("Draft");
    cx.simulate_keystrokes("escape");
    assert_eq!(f.documents(Some(new_folder), cx).len(), 2);
    assert!(f.events().is_empty());

    // When the keyboard goes elsewhere (the editor, a modal), the rename ends and the
    // typed name is kept.
    f.panel.update_in(cx, |p, window, cx| {
        p.begin_rename(Document(f.in_inner), window, cx)
    });
    cx.simulate_input("Spark");
    cx.update(|window, _| window.blur());
    cx.run_until_parked();
    assert_eq!(f.renaming(cx), None);
    assert_eq!(f.title(f.in_inner, cx), "Spark");
    let panel = f.panel.clone();
    cx.update(|window, cx| window.focus(&panel.focus_handle(cx)));

    // Starting another rename commits the one in progress.
    f.panel.update_in(cx, |p, window, cx| {
        p.begin_rename(Document(f.in_folder), window, cx)
    });
    cx.simulate_input("First");
    f.panel.update_in(cx, |p, window, cx| {
        p.begin_rename(Folder(f.folder), window, cx)
    });
    assert_eq!(f.title(f.in_folder, cx), "First");
    assert_eq!(field_text(cx).as_deref(), Some("Book"));
    cx.simulate_keystrokes("escape");
    assert!(f.toasts(cx).is_empty());
}

#[gpui::test]
fn duplicate_and_reorder(cx: &mut TestAppContext) {
    let (f, cx) = fixture(cx);
    let b = f.create(Some(f.folder), "B", cx);
    let c = f.create(Some(f.folder), "C", cx);
    let other = f
        .library
        .update(cx, |l, cx| l.create_folder(None, "Other", cx).unwrap().id);
    cx.run_until_parked();
    f.panel
        .update(cx, |p, cx| p.reveal_document(f.in_folder, cx));
    assert_eq!(f.documents(Some(f.folder), cx), vec![f.in_folder, b, c]);

    // alt-down / alt-up move the document among its siblings and stop at the ends.
    cx.simulate_keystrokes("alt-down");
    assert_eq!(f.documents(Some(f.folder), cx), vec![b, f.in_folder, c]);
    assert_eq!(f.selected(cx), Some(Document(f.in_folder)));
    cx.simulate_keystrokes("alt-down alt-down");
    assert_eq!(f.documents(Some(f.folder), cx), vec![b, c, f.in_folder]);
    cx.simulate_keystrokes("alt-up alt-up alt-up");
    assert_eq!(f.documents(Some(f.folder), cx), vec![f.in_folder, b, c]);
    // The rows follow.
    assert_eq!(
        f.visible(cx),
        vec![
            Folder(f.folder),
            Folder(f.inner),
            Document(f.in_folder),
            Document(b),
            Document(c),
            Folder(other),
            Document(f.at_root)
        ]
    );

    // Folders reorder among folders.
    let root_folders = |cx: &mut VisualTestContext| {
        f.library.read_with(cx, |l, _| {
            l.tree()
                .folders
                .iter()
                .map(|n| n.folder.id)
                .collect::<Vec<_>>()
        })
    };
    f.panel
        .update(cx, |p, cx| p.select(Some(Folder(other)), cx));
    cx.simulate_keystrokes("alt-up");
    assert_eq!(root_folders(cx), vec![other, f.folder]);
    cx.simulate_keystrokes("alt-up alt-down alt-down");
    assert_eq!(root_folders(cx), vec![f.folder, other]);

    // cmd-d duplicates a document right after it and selects the copy.
    f.panel.update(cx, |p, cx| p.select(Some(Document(b)), cx));
    cx.simulate_keystrokes("cmd-d");
    let documents = f.documents(Some(f.folder), cx);
    assert_eq!(documents.len(), 4);
    let copy = documents[2];
    assert_eq!(&documents[..2], &[f.in_folder, b]);
    assert_eq!(f.title(copy, cx), "B copy");
    assert_eq!(f.selected(cx), Some(Document(copy)));
    // Folders are not duplicated; the panel says so.
    f.panel
        .update(cx, |p, cx| p.select(Some(Folder(other)), cx));
    cx.simulate_keystrokes("cmd-d");
    assert_eq!(root_folders(cx), vec![f.folder, other]);
    assert_eq!(f.toasts(cx), vec!["Only documents can be duplicated."]);
}

#[gpui::test]
fn move_with_the_folder_picker(cx: &mut TestAppContext) {
    let (f, cx) = fixture(cx);
    let has_modal = |cx: &mut VisualTestContext| {
        f.panel
            .read_with(cx, |p, cx| p.modal_layer().read(cx).has_modal())
    };

    // m on a document: pick "Notes" by typing.
    f.panel
        .update(cx, |p, cx| p.select(Some(Document(f.at_root)), cx));
    cx.simulate_keystrokes("m");
    assert!(has_modal(cx));
    cx.simulate_input("notes");
    cx.simulate_keystrokes("enter");
    assert!(!has_modal(cx));
    assert_eq!(f.folder_of(f.at_root, cx), Some(f.inner));
    // The moved item is revealed and stays selected; focus is back in the panel.
    f.panel.read_with(cx, |p, _| {
        assert_eq!(p.selected(), Some(Document(f.at_root)));
        assert!(p.is_expanded(f.folder) && p.is_expanded(f.inner));
    });
    assert!(f.panel_focused(cx));
    assert_eq!(f.toasts(cx), vec!["Moved “Untitled” to Book / Notes."]);

    // "Library root" is the first choice.
    cx.simulate_keystrokes("m");
    cx.simulate_input("root");
    cx.simulate_keystrokes("enter");
    assert_eq!(f.folder_of(f.at_root, cx), None);
    assert_eq!(f.toasts(cx), vec!["Moved “Untitled” to the library root."]);

    // Choosing the place the item is already in does nothing; escape closes the picker.
    cx.simulate_keystrokes("m enter");
    assert!(!has_modal(cx));
    assert!(f.toasts(cx).is_empty());
    cx.simulate_keystrokes("m escape");
    assert!(!has_modal(cx));
    assert!(f.panel_focused(cx));
    assert!(f.events().is_empty());

    // A folder cannot move into itself or its own subfolders: they are not offered.
    let other = f
        .library
        .update(cx, |l, cx| l.create_folder(None, "Other", cx).unwrap().id);
    cx.run_until_parked();
    f.panel
        .update(cx, |p, cx| p.select(Some(Folder(f.folder)), cx));
    cx.simulate_keystrokes("m");
    cx.simulate_input("notes");
    cx.simulate_keystrokes("enter");
    // No match, so enter chose nothing and the picker is still open.
    assert!(has_modal(cx));
    cx.simulate_keystrokes("cmd-a backspace");
    cx.simulate_input("other");
    cx.simulate_keystrokes("enter");
    assert!(!has_modal(cx));
    assert_eq!(
        f.library
            .read_with(cx, |l, _| l.folder(f.folder).unwrap().parent_id),
        Some(other)
    );
    assert_eq!(f.toasts(cx), vec!["Moved “Book” to Other."]);
    assert_eq!(f.selected(cx), Some(Folder(f.folder)));

    // The API moves without a picker.
    f.panel
        .update(cx, |p, cx| p.move_item(Document(f.in_inner), None, cx))
        .unwrap();
    assert_eq!(f.folder_of(f.in_inner, cx), None);
    assert!(
        f.panel
            .update(cx, |p, cx| p.move_item(Folder(other), Some(f.inner), cx))
            .is_err()
    );
}

#[gpui::test]
fn trash_with_undo(cx: &mut TestAppContext) {
    let (f, cx) = fixture(cx);
    f.panel
        .update(cx, |p, cx| p.reveal_document(f.in_inner, cx));
    assert_eq!(f.visible(cx).len(), 5);

    // cmd-backspace trashes the selection and names it; the row below takes its place.
    f.panel
        .update(cx, |p, cx| p.select(Some(Document(f.in_folder)), cx));
    cx.simulate_keystrokes("cmd-backspace");
    assert_eq!(f.documents(Some(f.folder), cx), vec![]);
    assert_eq!(f.selected(cx), Some(Document(f.at_root)));
    assert_eq!(
        f.toasts(cx),
        vec!["Moved “Chapter” to the trash. Press ⌘Z to undo."]
    );
    assert_eq!(
        f.panel.read_with(cx, |p, _| p.trash_items()),
        vec![Document(f.in_folder)]
    );

    // A folder goes with everything in it.
    f.panel
        .update(cx, |p, cx| p.select(Some(Folder(f.folder)), cx));
    cx.simulate_keystrokes("cmd-backspace");
    assert_eq!(f.visible(cx), vec![Document(f.at_root)]);
    assert_eq!(
        f.toasts(cx),
        vec!["Moved “Book” (1 document, 1 subfolder) to the trash. Press ⌘Z to undo."]
    );

    // cmd-z brings back the last one, revealed and selected; again for the one before.
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(f.selected(cx), Some(Folder(f.folder)));
    assert_eq!(f.documents(Some(f.inner), cx), vec![f.in_inner]);
    assert_eq!(f.toasts(cx), vec!["Restored “Book” to the library root."]);
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(f.documents(Some(f.folder), cx), vec![f.in_folder]);
    assert_eq!(f.selected(cx), Some(Document(f.in_folder)));
    assert_eq!(f.toasts(cx), vec!["Restored “Chapter” to Book."]);
    assert!(f.panel.read_with(cx, |p, _| p.trash_items()).is_empty());
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(f.toasts(cx), vec!["Nothing to undo."]);

    // Something trashed elsewhere (the app, another view) can be undone here too.
    f.library
        .update(cx, |l, cx| l.trash_document(f.at_root, cx).unwrap());
    cx.run_until_parked();
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(f.documents(None, cx), vec![f.at_root]);
    f.toasts(cx);

    // An item that was deleted for good in the meantime is skipped.
    f.panel
        .update(cx, |p, cx| p.trash_item(Document(f.in_folder), cx));
    f.panel
        .update(cx, |p, cx| p.trash_item(Document(f.at_root), cx));
    cx.run_until_parked();
    f.library
        .update(cx, |l, cx| l.purge_document(f.at_root, cx).unwrap());
    cx.run_until_parked();
    f.toasts(cx);
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(f.documents(Some(f.folder), cx), vec![f.in_folder]);
    assert_eq!(f.toasts(cx), vec!["Restored “Chapter” to Book."]);

    // With the trash row selected there is nothing to trash.
    cx.simulate_keystrokes("end down cmd-backspace");
    assert!(f.toasts(cx).is_empty());
    assert!(f.events().is_empty());
}

#[gpui::test]
fn trash_view_restore_delete_and_empty(cx: &mut TestAppContext) {
    let (f, cx) = fixture(cx);
    let has_modal = |cx: &mut VisualTestContext| {
        f.panel
            .read_with(cx, |p, cx| p.modal_layer().read(cx).has_modal())
    };
    let trash = |cx: &mut VisualTestContext| f.panel.read_with(cx, |p, _| p.trash_items());
    let trash_selected =
        |cx: &mut VisualTestContext| f.panel.read_with(cx, |p, _| p.selected_trash_item());
    let showing = |cx: &mut VisualTestContext| f.panel.read_with(cx, |p, _| p.is_showing_trash());

    let extra = f.create(Some(f.inner), "Extra", cx);
    f.clock.advance_ms(1000);
    f.library
        .update(cx, |l, cx| l.trash_document(f.in_inner, cx).unwrap());
    f.clock.advance_ms(1000);
    f.library
        .update(cx, |l, cx| l.trash_document(f.at_root, cx).unwrap());
    f.clock.advance_ms(1000);
    f.library
        .update(cx, |l, cx| l.trash_folder(f.folder, cx).unwrap());
    cx.run_until_parked();
    assert!(f.visible(cx).is_empty());

    // With an empty tree, down lands on the trash row; enter opens the trash view.
    cx.simulate_keystrokes("down");
    assert!(f.panel.read_with(cx, |p, _| p.is_trash_row_selected()));
    cx.simulate_keystrokes("enter");
    assert!(showing(cx));
    // Most recently trashed first, each with the place it came from.
    assert_eq!(
        trash(cx),
        vec![Folder(f.folder), Document(f.at_root), Document(f.in_inner)]
    );
    assert_eq!(trash_selected(cx), Some(Folder(f.folder)));
    f.panel.read_with(cx, |p, _| {
        assert_eq!(
            p.trash_location(Folder(f.folder)).as_deref(),
            Some("Library root")
        );
        assert_eq!(
            p.trash_location(Document(f.at_root)).as_deref(),
            Some("Library root")
        );
        // Its folder is in the trash too, so a restore would put it at the root.
        assert_eq!(
            p.trash_location(Document(f.in_inner)).as_deref(),
            Some("Library root")
        );
        assert_eq!(p.trash_location(Document(extra)), None);
    });

    // j / k / home / end move in the trash list; the tree's keys do nothing here.
    cx.simulate_keystrokes("j");
    assert_eq!(trash_selected(cx), Some(Document(f.at_root)));
    cx.simulate_keystrokes("end");
    assert_eq!(trash_selected(cx), Some(Document(f.in_inner)));
    cx.simulate_keystrokes("down n shift-n m t /");
    assert_eq!(trash_selected(cx), Some(Document(f.in_inner)));
    assert_eq!(f.renaming(cx), None);
    assert!(!has_modal(cx));
    cx.simulate_keystrokes("home");
    assert_eq!(trash_selected(cx), Some(Folder(f.folder)));

    // r restores the folder with what was trashed with it; the view stays on the trash.
    cx.simulate_keystrokes("r");
    assert!(showing(cx));
    assert_eq!(trash(cx), vec![Document(f.at_root), Document(f.in_inner)]);
    assert_eq!(trash_selected(cx), Some(Document(f.at_root)));
    assert_eq!(f.documents(Some(f.folder), cx), vec![f.in_folder]);
    assert_eq!(f.documents(Some(f.inner), cx), vec![extra]);
    assert_eq!(f.toasts(cx), vec!["Restored “Book” to the library root."]);
    // Now that "Notes" is back, the idea shows where it came from.
    assert_eq!(
        f.panel
            .read_with(cx, |p, _| p.trash_location(Document(f.in_inner))),
        Some("Book / Notes".to_string())
    );
    // enter restores as well; the restored item is selected in the tree behind.
    cx.simulate_keystrokes("down enter");
    assert_eq!(trash(cx), vec![Document(f.at_root)]);
    assert_eq!(f.documents(Some(f.inner), cx), vec![extra, f.in_inner]);
    assert_eq!(f.toasts(cx), vec!["Restored “Idea” to Book / Notes."]);
    assert_eq!(f.selected(cx), Some(Document(f.in_inner)));

    // cmd-backspace asks before deleting for good; escape keeps the item.
    cx.simulate_keystrokes("cmd-backspace");
    assert!(has_modal(cx));
    assert!(f.panel.read_with(cx, |p, cx| {
        p.modal_layer().read(cx).is_open::<ConfirmDialog>()
    }));
    cx.simulate_keystrokes("escape");
    assert!(!has_modal(cx));
    assert_eq!(trash(cx), vec![Document(f.at_root)]);
    assert!(showing(cx));
    assert!(f.panel_focused(cx));
    // The panel's keys do not act behind the dialog.
    cx.simulate_keystrokes("cmd-backspace r j escape");
    assert_eq!(trash(cx), vec![Document(f.at_root)]);
    // enter confirms.
    cx.simulate_keystrokes("cmd-backspace enter");
    assert!(!has_modal(cx));
    assert!(trash(cx).is_empty());
    assert!(
        !f.library
            .read_with(cx, |l, _| l.document_meta(f.at_root).is_ok())
    );
    assert_eq!(f.toasts(cx), vec!["Deleted “Untitled”."]);
    assert_eq!(trash_selected(cx), None);

    // escape (or left) goes back to the tree, with the panel still focused.
    cx.simulate_keystrokes("escape");
    assert!(!showing(cx));
    assert!(f.events().is_empty());
    assert!(f.panel_focused(cx));

    // t shows the trash from the tree; emptying an empty trash only says so.
    cx.simulate_keystrokes("t");
    assert!(showing(cx));
    cx.simulate_keystrokes("cmd-shift-backspace");
    assert!(!has_modal(cx));
    assert_eq!(f.toasts(cx), vec!["The trash is already empty."]);
    cx.simulate_keystrokes("left");
    assert!(!showing(cx));

    // Empty trash: one confirmation for everything.
    f.library.update(cx, |l, cx| {
        l.trash_document(extra, cx).unwrap();
        l.trash_folder(f.inner, cx).unwrap();
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("cmd-shift-backspace");
    assert!(has_modal(cx));
    cx.simulate_keystrokes("escape");
    assert_eq!(trash(cx).len(), 2);
    cx.simulate_keystrokes("cmd-shift-backspace enter");
    assert!(trash(cx).is_empty());
    assert_eq!(f.toasts(cx), vec!["Deleted 2 documents and 1 folder."]);
    // Nothing is left to undo.
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(f.toasts(cx), vec!["Nothing to undo."]);
    assert_eq!(f.visible(cx), vec![Folder(f.folder), Document(f.in_folder)]);
}

#[gpui::test]
fn filter_narrows_the_tree(cx: &mut TestAppContext) {
    let (f, cx) = fixture(cx);
    let chapter_two = f.create(Some(f.folder), "Chapter two", cx);
    let ideas = f.create(None, "Loose ideas", cx);
    cx.run_until_parked();
    let filter = |cx: &mut VisualTestContext| f.panel.read_with(cx, |p, _| p.filter().to_string());
    let filter_focused = |cx: &mut VisualTestContext| {
        let panel = f.panel.clone();
        cx.update(|window, cx| {
            panel
                .read(cx)
                .filter_field()
                .is_some_and(|field| field.focus_handle(cx).is_focused(window))
        })
    };

    // "/" focuses the filter; typing narrows the tree to the matches and the folders that
    // lead to them, whatever is collapsed.
    cx.simulate_keystrokes("/");
    assert!(filter_focused(cx));
    cx.simulate_input("chap");
    assert_eq!(filter(cx), "chap");
    assert_eq!(
        f.visible(cx),
        vec![
            Folder(f.folder),
            Document(f.in_folder),
            Document(chapter_two)
        ]
    );
    // The first match is selected, and the arrows move while the field keeps the keyboard.
    assert_eq!(f.selected(cx), Some(Document(f.in_folder)));
    cx.simulate_keystrokes("down");
    assert_eq!(f.selected(cx), Some(Document(chapter_two)));
    cx.simulate_keystrokes("down");
    assert_eq!(f.selected(cx), Some(Document(chapter_two)));
    assert!(!f.panel.read_with(cx, |p, _| p.is_trash_row_selected()));
    assert!(filter_focused(cx));
    // Letters keep going into the field; every word must match, in any case.
    cx.simulate_input("TER tw");
    assert_eq!(filter(cx), "chapTER tw");
    assert_eq!(f.visible(cx), vec![Folder(f.folder), Document(chapter_two)]);
    assert_eq!(f.selected(cx), Some(Document(chapter_two)));

    // enter opens the selected document.
    cx.simulate_keystrokes("enter");
    assert_eq!(
        f.events(),
        vec![LibraryPanelEvent::OpenDocument {
            id: chapter_two,
            focus_editor: true
        }]
    );
    // The persisted expansion was not touched by filtering.
    assert!(!f.panel.read_with(cx, |p, _| p.is_expanded(f.folder)));

    // escape clears the filter and goes back to the tree, keeping the selection in view.
    cx.simulate_keystrokes("escape");
    assert_eq!(filter(cx), "");
    assert!(f.panel_focused(cx));
    assert_eq!(f.selected(cx), Some(Document(chapter_two)));
    assert!(f.panel.read_with(cx, |p, _| p.is_expanded(f.folder)));
    assert!(f.events().is_empty());

    // A folder that matches keeps everything inside it.
    cx.simulate_keystrokes("/");
    cx.simulate_input("notes");
    assert_eq!(
        f.visible(cx),
        vec![Folder(f.folder), Folder(f.inner), Document(f.in_inner)]
    );
    assert_eq!(f.selected(cx), Some(Folder(f.inner)));
    // enter on a folder, or tab, continues in the tree with the filter still on.
    cx.simulate_keystrokes("enter");
    assert!(f.panel_focused(cx));
    assert_eq!(filter(cx), "notes");
    cx.simulate_keystrokes("j");
    assert_eq!(f.selected(cx), Some(Document(f.in_inner)));
    // Renaming works on the filtered rows.
    cx.simulate_keystrokes("r");
    cx.simulate_input("Notes on ideas");
    cx.simulate_keystrokes("enter");
    assert_eq!(f.title(f.in_inner, cx), "Notes on ideas");
    assert_eq!(filter(cx), "notes");
    // escape in the tree clears the filter first; the next one leaves the panel.
    cx.simulate_keystrokes("escape");
    assert_eq!(filter(cx), "");
    assert!(f.events().is_empty());
    cx.simulate_keystrokes("escape");
    assert_eq!(f.events(), vec![LibraryPanelEvent::Dismissed]);

    // No match: an empty list, and no selection.
    cx.simulate_keystrokes("cmd-f");
    assert!(filter_focused(cx));
    cx.simulate_input("zzz");
    assert!(f.visible(cx).is_empty());
    assert_eq!(f.selected(cx), None);
    cx.simulate_keystrokes("tab");
    assert!(f.panel_focused(cx));
    cx.simulate_keystrokes("escape");

    // The API filters too, and revealing an item the filter hides clears the filter.
    f.panel.update(cx, |p, cx| p.set_filter("ideas", cx));
    assert_eq!(
        f.visible(cx),
        vec![
            Folder(f.folder),
            Folder(f.inner),
            Document(f.in_inner),
            Document(ideas)
        ]
    );
    f.panel
        .update(cx, |p, cx| p.reveal_document(f.in_folder, cx));
    assert_eq!(filter(cx), "");
    assert_eq!(f.selected(cx), Some(Document(f.in_folder)));
    // A new item made while filtering is shown: the filter gives way.
    f.panel.update(cx, |p, cx| p.set_filter("ideas", cx));
    cx.simulate_keystrokes("shift-n");
    assert_eq!(filter(cx), "");
    assert!(matches!(f.renaming(cx), Some(Folder(_))));
    cx.simulate_keystrokes("escape");
}

#[gpui::test]
fn row_details(cx: &mut TestAppContext) {
    let (f, cx) = fixture(cx);
    f.panel
        .update(cx, |p, cx| p.reveal_document(f.in_inner, cx));
    let detail = |item: LibraryItem, cx: &mut VisualTestContext| {
        f.panel.read_with(cx, |p, cx| p.item_detail(item, cx))
    };

    // Documents show how long ago they were edited; folders how many documents they hold.
    assert_eq!(detail(Document(f.in_folder), cx).as_deref(), Some("now"));
    assert_eq!(detail(Folder(f.folder), cx).as_deref(), Some("2"));
    assert_eq!(detail(Folder(f.inner), cx).as_deref(), Some("1"));
    for (ms, expected) in [
        (59_000, "now"),
        (60_000, "1m"),
        (59 * 60_000, "59m"),
        (3 * 3_600_000, "3h"),
        (2 * 86_400_000, "2d"),
        (20 * 86_400_000, "2w"),
        (90 * 86_400_000, "3mo"),
        (800 * 86_400_000, "2y"),
    ] {
        f.clock.set_ms(START + ms);
        assert_eq!(detail(Document(f.in_folder), cx).as_deref(), Some(expected));
    }

    // A save refreshes the detail without any structural change.
    let text = "word ".repeat(1240);
    f.library.update(cx, |l, cx| {
        l.save_document(f.in_folder, &content("Chapter", &text), cx)
            .unwrap()
    });
    cx.run_until_parked();
    assert_eq!(detail(Document(f.in_folder), cx).as_deref(), Some("now"));

    // i switches to word counts, and the choice is stored.
    cx.simulate_keystrokes("i");
    assert_eq!(
        f.panel.read_with(cx, |p, _| p.row_detail()),
        RowDetail::Words
    );
    assert_eq!(detail(Document(f.in_folder), cx).as_deref(), Some("1,240"));
    assert_eq!(detail(Document(f.in_inner), cx).as_deref(), Some("1"));
    assert_eq!(
        f.library.read_with(cx, |l, _| l
            .setting::<String>(writer_library::ROW_DETAIL_SETTING)
            .unwrap()),
        Some("words".to_string())
    );
    let library = f.library.clone();
    let second = cx.new(|cx| LibraryPanel::new(library, cx));
    assert_eq!(
        second.read_with(cx, |p, _| p.row_detail()),
        RowDetail::Words
    );
    cx.simulate_keystrokes("i");
    assert_eq!(
        f.panel.read_with(cx, |p, _| p.row_detail()),
        RowDetail::Edited
    );

    // An empty folder has no detail; an item that is not on screen has none either.
    let empty = f
        .library
        .update(cx, |l, cx| l.create_folder(None, "Empty", cx).unwrap().id);
    cx.run_until_parked();
    assert_eq!(detail(Folder(empty), cx), None);
    cx.simulate_keystrokes("home left");
    assert_eq!(detail(Document(f.in_folder), cx), None);
}

#[gpui::test]
fn mouse(cx: &mut TestAppContext) {
    let (f, cx) = fixture(cx);
    let none = Modifiers::default();

    // A click on a folder row selects it; the disclosure triangle toggles it.
    cx.simulate_click(Fixture::row_point(0, 120.), none);
    assert_eq!(f.selected(cx), Some(Folder(f.folder)));
    assert_eq!(f.visible(cx).len(), 2);
    assert!(f.events().is_empty());
    cx.simulate_click(Fixture::row_point(0, 16.), none);
    assert_eq!(
        f.visible(cx),
        vec![
            Folder(f.folder),
            Folder(f.inner),
            Document(f.in_folder),
            Document(f.at_root)
        ]
    );
    // The nested folder's triangle is one indent further in.
    cx.simulate_click(Fixture::row_point(1, 30.), none);
    assert_eq!(f.visible(cx).len(), 5);
    cx.simulate_click(Fixture::row_point(1, 30.), none);
    assert_eq!(f.visible(cx).len(), 4);

    // A click on a document selects and opens it, and the list keeps the keyboard.
    cx.simulate_click(Fixture::row_point(2, 120.), none);
    assert_eq!(f.selected(cx), Some(Document(f.in_folder)));
    assert_eq!(
        f.events(),
        vec![LibraryPanelEvent::OpenDocument {
            id: f.in_folder,
            focus_editor: false
        }]
    );
    assert!(f.panel_focused(cx));

    // A double click renames in place.
    let position = Fixture::row_point(2, 120.);
    for click_count in [1, 2] {
        cx.simulate_event(MouseDownEvent {
            position,
            modifiers: none,
            button: MouseButton::Left,
            click_count,
            first_mouse: false,
        });
        cx.simulate_event(MouseUpEvent {
            position,
            modifiers: none,
            button: MouseButton::Left,
            click_count,
        });
    }
    assert_eq!(f.renaming(cx), Some(Document(f.in_folder)));
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("Opening");
    cx.simulate_keystrokes("enter");
    assert_eq!(f.title(f.in_folder, cx), "Opening");
    f.events();

    // A click below the last row clears the selection.
    cx.simulate_click(Fixture::row_point(8, 120.), none);
    assert_eq!(f.selected(cx), None);

    // The trash row at the bottom opens the trash view; "Back" returns.
    f.library
        .update(cx, |l, cx| l.trash_document(f.at_root, cx).unwrap());
    cx.run_until_parked();
    let viewport = cx.update(|window, _| window.viewport_size());
    cx.simulate_click(point(px(100.), viewport.height - px(16.)), none);
    assert!(f.panel.read_with(cx, |p, _| p.is_showing_trash()));
    // A click selects a trashed item, a double click restores it.
    let position = point(
        px(100.),
        writer_library::HEADER_HEIGHT + px(4.) + writer_library::TRASH_ROW_HEIGHT / 2.,
    );
    for click_count in [1, 2] {
        cx.simulate_event(MouseDownEvent {
            position,
            modifiers: none,
            button: MouseButton::Left,
            click_count,
            first_mouse: false,
        });
        cx.simulate_event(MouseUpEvent {
            position,
            modifiers: none,
            button: MouseButton::Left,
            click_count,
        });
    }
    assert_eq!(f.documents(None, cx), vec![f.at_root]);
    cx.simulate_click(
        point(viewport.width - px(30.), writer_library::HEADER_HEIGHT / 2.),
        none,
    );
    assert!(!f.panel.read_with(cx, |p, _| p.is_showing_trash()));
}

#[gpui::test]
fn a_large_library_stays_fast(cx: &mut TestAppContext) {
    let (f, cx) = fixture(cx);
    // 4000 documents in 40 folders, all expanded.
    let mut ids = Vec::new();
    let started = Instant::now();
    for folder_ix in 0..40 {
        let folder = f.library.update(cx, |l, cx| {
            l.create_folder(None, &format!("Folder {folder_ix:02}"), cx)
                .unwrap()
                .id
        });
        ids.push(folder);
    }
    let seeded = f.library.update(cx, |l, cx| {
        let mut count = 0;
        for (folder_ix, folder) in ids.iter().enumerate() {
            for doc_ix in 0..100 {
                l.create_document(
                    Some(*folder),
                    &content(&format!("Document {folder_ix:02}-{doc_ix:03}"), "x"),
                    cx,
                )
                .unwrap();
                count += 1;
            }
        }
        count
    });
    assert_eq!(seeded, 4000);
    cx.run_until_parked();
    eprintln!("seeding 4000 documents took {:?}", started.elapsed());
    f.panel.update(cx, |p, cx| {
        for folder in &ids {
            p.set_expanded(*folder, true, cx);
        }
    });
    cx.run_until_parked();
    assert_eq!(f.visible(cx).len(), 2 + 40 + 4000);

    // Walking through the list redraws only the rows on screen.
    let started = Instant::now();
    cx.simulate_keystrokes("end");
    for _ in 0..100 {
        cx.simulate_keystrokes("up");
    }
    cx.simulate_keystrokes("home");
    for _ in 0..100 {
        cx.simulate_keystrokes("j");
    }
    let navigation = started.elapsed();
    eprintln!("202 selection moves over 4042 rows took {navigation:?}");
    assert!(
        navigation < Duration::from_secs(10),
        "navigation took {navigation:?}"
    );

    // A page is a screenful: far more than one row, far less than the list.
    cx.simulate_keystrokes("home pagedown");
    let page = f.panel.read_with(cx, |p, _| {
        let selected = p.selected().unwrap();
        p.visible_items()
            .iter()
            .position(|item| *item == selected)
            .unwrap()
    });
    assert!((10..200).contains(&page), "a page moved {page} rows");
    cx.simulate_keystrokes("pagedown pageup pageup");
    assert_eq!(f.selected(cx), f.visible(cx).first().copied());

    // Filtering runs over every name on each keystroke.
    let started = Instant::now();
    cx.simulate_keystrokes("/");
    cx.simulate_input("document 17-04");
    let filtering = started.elapsed();
    eprintln!("typing a 14 character filter over 4042 rows took {filtering:?}");
    assert_eq!(f.visible(cx).len(), 1 + 10);
    assert!(
        filtering < Duration::from_secs(10),
        "filtering took {filtering:?}"
    );
    cx.simulate_keystrokes("escape");
    assert_eq!(f.visible(cx).len(), 2 + 40 + 4000);
}

// ----- commands from outside the panel ----------------------------------------------------

/// A window root like the app's: an "editor" that has the focus, and the panel beside it.
struct Shell {
    panel: Entity<LibraryPanel>,
    editor: FocusHandle,
}

impl Render for Shell {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        writer_library::forward_actions(div().size_full().flex(), &self.panel)
            .child(div().w(px(280.)).h_full().child(self.panel.clone()))
            .child(div().flex_1().track_focus(&self.editor))
    }
}

#[gpui::test]
fn commands_reach_the_panel_from_the_editor(cx: &mut TestAppContext) {
    cx.update(|cx| {
        writer_ui::theme::init(writer_ui::ThemeMode::Light, cx);
        writer_ui::init(cx);
        writer_library::init(cx);
    });
    let (library, _) = library_from(Library::in_memory().unwrap(), cx);
    let (folder, document) = library.update(cx, |l, cx| {
        let folder = l.create_folder(None, "Book", cx).unwrap().id;
        let document = l
            .create_document(Some(folder), &content("Chapter", "x"), cx)
            .unwrap()
            .id;
        (folder, document)
    });
    let (shell, cx) = cx.add_window_view({
        let library = library.clone();
        move |window, cx| {
            let panel = cx.new(|cx| LibraryPanel::new(library, cx));
            let editor = cx.focus_handle();
            window.focus(&editor);
            Shell { panel, editor }
        }
    });
    let panel = shell.read_with(cx, |shell, _| shell.panel.clone());
    let editor = shell.read_with(cx, |shell, _| shell.editor.clone());
    cx.run_until_parked();
    panel.update(cx, |p, cx| p.set_active_document(Some(document), cx));

    // The panel's plain keys mean nothing while the editor has the keyboard.
    cx.simulate_keystrokes("j shift-n r");
    assert_eq!(panel.read_with(cx, |p, _| p.renaming()), None);
    assert_eq!(library.read_with(cx, |l, _| l.folders().unwrap().len()), 1);

    // A palette command (an action dispatched at the focused element) reaches the panel.
    cx.dispatch_action(writer_library::NewFolder);
    let Some(Folder(new_folder)) = panel.read_with(cx, |p, _| p.renaming()) else {
        panic!("not renaming a folder");
    };
    assert_eq!(
        library.read_with(cx, |l, _| l.folder(new_folder).unwrap().parent_id),
        Some(folder)
    );
    // The rename field took the keyboard.
    cx.simulate_input("Drafts");
    cx.simulate_keystrokes("enter");
    assert_eq!(
        library.read_with(cx, |l, _| l.folder(new_folder).unwrap().name),
        "Drafts"
    );

    // Trash and undo from the editor act on the panel's selection.
    cx.update(|window, _| window.focus(&editor));
    panel.update(cx, |p, cx| p.select(Some(Document(document)), cx));
    cx.dispatch_action(writer_library::MoveToTrash);
    assert!(!library.read_with(cx, |l, _| l.document_exists(document).unwrap()));
    cx.run_until_parked();
    cx.dispatch_action(writer_library::UndoTrash);
    assert!(library.read_with(cx, |l, _| l.document_exists(document).unwrap()));

    // Without the content of an empty document the panel passes NewDocument on.
    cx.dispatch_action(writer_library::NewDocument);
    assert_eq!(
        library.read_with(cx, |l, _| l.all_documents().unwrap().len()),
        1
    );
    panel.update(cx, |p, _| p.set_new_document_content("{}"));
    cx.dispatch_action(writer_library::NewDocument);
    assert_eq!(
        library.read_with(cx, |l, _| l.all_documents().unwrap().len()),
        2
    );
    assert!(matches!(
        panel.read_with(cx, |p, _| p.renaming()),
        Some(Document(_))
    ));
    cx.simulate_keystrokes("escape");

    // Show trash and the filter move the keyboard into the panel.
    cx.update(|window, _| window.focus(&editor));
    cx.dispatch_action(writer_library::ShowTrash);
    assert!(panel.read_with(cx, |p, _| p.is_showing_trash()));
    assert!(cx.update(|window, cx| panel.focus_handle(cx).is_focused(window)));
    cx.update(|window, _| window.focus(&editor));
    cx.dispatch_action(writer_library::FocusFilter);
    assert!(!panel.read_with(cx, |p, _| p.is_showing_trash()));
    cx.simulate_input("chap");
    assert_eq!(panel.read_with(cx, |p, _| p.filter().to_string()), "chap");
}

/// A window root that owns the modal layer, as the app does.
struct SharedModalShell {
    panel: Entity<LibraryPanel>,
    modal_layer: Entity<writer_ui::ModalLayer>,
}

impl Render for SharedModalShell {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .relative()
            .size_full()
            .child(self.panel.clone())
            .child(self.modal_layer.clone())
    }
}

#[gpui::test]
fn the_panel_can_use_the_window_modal_layer(cx: &mut TestAppContext) {
    cx.update(|cx| {
        writer_ui::theme::init(writer_ui::ThemeMode::Light, cx);
        writer_ui::init(cx);
        writer_library::init(cx);
    });
    let (library, _) = library_from(Library::in_memory().unwrap(), cx);
    let (folder, document) = library.update(cx, |l, cx| {
        let folder = l.create_folder(None, "Book", cx).unwrap().id;
        let document = l
            .create_document(None, &content("Chapter", "x"), cx)
            .unwrap()
            .id;
        (folder, document)
    });
    let (shell, cx) = cx.add_window_view({
        let library = library.clone();
        move |window, cx| {
            let modal_layer = cx.new(|_| writer_ui::ModalLayer::new());
            let panel = cx.new(|cx| {
                let mut panel = LibraryPanel::new(library, cx);
                panel.set_modal_layer(modal_layer.clone(), cx);
                panel
            });
            window.focus(&panel.focus_handle(cx));
            SharedModalShell { panel, modal_layer }
        }
    });
    let panel = shell.read_with(cx, |shell, _| shell.panel.clone());
    let modal_layer = shell.read_with(cx, |shell, _| shell.modal_layer.clone());
    cx.run_until_parked();

    // The move picker opens in the window's layer and moves the document.
    panel.update(cx, |p, cx| p.select(Some(Document(document)), cx));
    cx.simulate_keystrokes("m");
    assert!(modal_layer.read_with(cx, |layer, _| layer.has_modal()));
    cx.simulate_input("book");
    cx.simulate_keystrokes("enter");
    assert!(!modal_layer.read_with(cx, |layer, _| layer.has_modal()));
    assert_eq!(
        library.read_with(cx, |l, _| l.document_meta(document).unwrap().folder_id),
        Some(folder)
    );
    assert!(cx.update(|window, cx| panel.focus_handle(cx).is_focused(window)));

    // So do the confirmations.
    cx.simulate_keystrokes("cmd-backspace t cmd-backspace");
    assert!(modal_layer.read_with(cx, |layer, _| layer.is_open::<ConfirmDialog>()));
    cx.simulate_keystrokes("enter");
    assert!(!modal_layer.read_with(cx, |layer, _| layer.has_modal()));
    assert!(library.read_with(cx, |l, _| l.trash().unwrap().is_empty()));
    assert!(library.read_with(cx, |l, _| l.document_meta(document).is_err()));
}
