//! Headless tests of the `LibraryPanel`: every flow is driven with keystrokes (and a few
//! with the mouse) against an in-memory library.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    AppContext as _, Context, Entity, FocusHandle, Focusable, Modifiers, MouseButton,
    MouseDownEvent, MouseUpEvent, Render, ScrollDelta, ScrollWheelEvent, TestAppContext,
    TouchPhase, VisualTestContext, Window, div, point, prelude::*, px,
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
                + writer_library::ROOT_ROW_HEIGHT
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
            "Duplicate",
            "Move to folder",
            "Undo library change",
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
    assert!(f.toasts(cx).is_empty());

    // cmd-d on a folder copies it with everything in it, right after it, and selects the
    // copy (collapsed, so a toast says what is inside).
    f.panel
        .update(cx, |p, cx| p.select(Some(Folder(f.folder)), cx));
    cx.simulate_keystrokes("cmd-d");
    let folders = root_folders(cx);
    assert_eq!(folders.len(), 3);
    let copied = folders[1];
    assert_eq!(folders, vec![f.folder, copied, other]);
    assert_eq!(f.folder_name(copied, cx), "Book copy");
    assert_eq!(f.selected(cx), Some(Folder(copied)));
    assert_eq!(
        f.toasts(cx),
        vec!["Duplicated “Book” with 5 documents, 1 subfolder."]
    );
    let titles = |folder: FolderId, cx: &mut VisualTestContext| -> Vec<String> {
        f.documents(Some(folder), cx)
            .into_iter()
            .map(|id| f.title(id, cx))
            .collect()
    };
    assert_eq!(titles(copied, cx), titles(f.folder, cx));
    assert_eq!(titles(copied, cx), vec!["Chapter", "B", "B copy", "C"]);
    let inner_copy = f.library.read_with(cx, |l, _| {
        l.tree().find_folder(copied).unwrap().folders[0].folder.id
    });
    assert_ne!(inner_copy, f.inner);
    assert_eq!(titles(inner_copy, cx), vec!["Idea"]);
    // The originals are where they were.
    assert_eq!(f.folder_of(f.in_inner, cx), Some(f.inner));
    // An empty folder is duplicated without a list of contents; the API reports the copy.
    let copy = f
        .panel
        .update(cx, |p, cx| p.duplicate_item(Folder(other), cx));
    let Some(Folder(copy)) = copy else {
        panic!("no copy: {copy:?}");
    };
    assert_eq!(root_folders(cx), vec![f.folder, copied, other, copy]);
    assert_eq!(f.toasts(cx), vec!["Duplicated “Other”."]);
    // duplicate_selected still answers with a document only.
    assert_eq!(f.panel.update(cx, |p, cx| p.duplicate_selected(cx)), None);
    assert_eq!(root_folders(cx).len(), 5);
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

    // A click below the last row selects the library itself: new items go to the top.
    cx.simulate_click(Fixture::row_point(8, 120.), none);
    assert_eq!(f.selected(cx), None);
    assert!(f.panel.read_with(cx, |p, _| p.is_root_row_selected()));
    assert_eq!(f.panel.read_with(cx, |p, _| p.selected_folder()), None);
    // So does a click on the "Library" row.
    cx.simulate_click(Fixture::row_point(1, 120.), none);
    assert!(!f.panel.read_with(cx, |p, _| p.is_root_row_selected()));
    cx.simulate_click(
        point(
            px(120.),
            writer_library::HEADER_HEIGHT + writer_library::ROOT_ROW_HEIGHT / 2.,
        ),
        none,
    );
    assert!(f.panel.read_with(cx, |p, _| p.is_root_row_selected()));
    assert!(f.panel_focused(cx));

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
    // The same walk through the list, timed on the small library first and on the large
    // one later: comparing the two keeps the test honest on a busy machine.
    let walk = |cx: &mut VisualTestContext| {
        let started = Instant::now();
        cx.simulate_keystrokes("end");
        for _ in 0..100 {
            cx.simulate_keystrokes("up");
        }
        cx.simulate_keystrokes("home");
        for _ in 0..100 {
            cx.simulate_keystrokes("j");
        }
        started.elapsed()
    };
    let small = walk(cx);

    // 3000 documents in 30 folders, all expanded.
    let mut ids = Vec::new();
    let started = Instant::now();
    for folder_ix in 0..30 {
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
    assert_eq!(seeded, 3000);
    cx.run_until_parked();
    eprintln!("seeding 3000 documents took {:?}", started.elapsed());
    f.panel.update(cx, |p, cx| {
        for folder in &ids {
            p.set_expanded(*folder, true, cx);
        }
    });
    cx.run_until_parked();
    assert_eq!(f.visible(cx).len(), 2 + 30 + 3000);

    // Walking through the large list redraws only the rows on screen: 600 times the rows
    // must not cost anywhere near 600 times the time.
    let large = walk(cx);
    eprintln!("202 selection moves: {small:?} over 5 rows, {large:?} over 3032 rows");
    assert!(
        large < small * 20 + Duration::from_secs(2),
        "202 selection moves took {small:?} over 5 rows and {large:?} over 3032 rows"
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
    eprintln!("typing a 14 character filter over 3032 rows took {filtering:?}");
    assert_eq!(f.visible(cx).len(), 1 + 10);
    assert!(
        filtering < Duration::from_secs(20),
        "filtering took {filtering:?}"
    );
    cx.simulate_keystrokes("escape");
    assert_eq!(f.visible(cx).len(), 2 + 30 + 3000);
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

// ----- the "Library" row ------------------------------------------------------------------

#[gpui::test]
fn the_library_row_is_the_root(cx: &mut TestAppContext) {
    let (f, cx) = fixture(cx);
    let on_root =
        |cx: &mut VisualTestContext| f.panel.read_with(cx, |p, _| p.is_root_row_selected());
    let target = |cx: &mut VisualTestContext| f.panel.read_with(cx, |p, _| p.selected_folder());
    // Every top-level item is a folder: without the row there would be no way to say
    // "at the top" from the keyboard.
    f.panel.update(cx, |p, cx| {
        p.move_item(Document(f.at_root), Some(f.folder), cx)
            .unwrap();
        p.set_expanded(f.folder, false, cx);
        p.set_new_document_content("{}");
    });
    cx.run_until_parked();
    f.toasts(cx);
    assert_eq!(f.visible(cx), vec![Folder(f.folder)]);
    assert_eq!(f.selected(cx), Some(Folder(f.folder)));
    assert_eq!(target(cx), Some(f.folder));

    // up from the first row selects the library itself, and stays there.
    cx.simulate_keystrokes("up");
    assert!(on_root(cx));
    assert_eq!(f.selected(cx), None);
    assert_eq!(target(cx), None);
    cx.simulate_keystrokes("up k home");
    assert!(!on_root(cx), "home is the first row");
    cx.simulate_keystrokes("up up");
    assert!(on_root(cx));

    // n and shift-n now create at the top level.
    cx.simulate_keystrokes("n");
    cx.simulate_input("Loose note");
    cx.simulate_keystrokes("enter");
    let loose = f.documents(None, cx);
    assert_eq!(loose.len(), 1);
    assert_eq!(f.title(loose[0], cx), "Loose note");
    assert_eq!(f.selected(cx), Some(Document(loose[0])));
    f.events();

    // left from a top-level row goes to its parent: the library.
    cx.simulate_keystrokes("left");
    assert!(on_root(cx));
    cx.simulate_keystrokes("shift-n");
    cx.simulate_input("Blog");
    cx.simulate_keystrokes("enter");
    let Some(Folder(blog)) = f.selected(cx) else {
        panic!("the new folder is not selected");
    };
    assert_eq!(
        f.library
            .read_with(cx, |l, _| l.folder(blog).unwrap().parent_id),
        None
    );
    // An open folder is closed first; the next left leaves it.
    f.panel
        .update(cx, |p, cx| p.select(Some(Folder(f.folder)), cx));
    cx.simulate_keystrokes("right left");
    assert_eq!(f.selected(cx), Some(Folder(f.folder)));
    cx.simulate_keystrokes("h");
    assert!(on_root(cx));

    // down and right lead back into the tree, at its first row.
    cx.simulate_keystrokes("down");
    assert_eq!(f.selected(cx), Some(Folder(f.folder)));
    cx.simulate_keystrokes("up l");
    assert_eq!(f.selected(cx), Some(Folder(f.folder)));
    cx.simulate_keystrokes("up pagedown");
    assert_eq!(f.selected(cx), Some(Folder(f.folder)));

    // The library itself cannot be renamed, moved, duplicated, opened or trashed.
    let before = f.library.read_with(cx, |l, _| l.tree());
    cx.simulate_keystrokes("up r f2 cmd-d m enter alt-up alt-down cmd-backspace");
    assert!(on_root(cx));
    assert_eq!(f.renaming(cx), None);
    assert!(
        !f.panel
            .read_with(cx, |p, cx| p.modal_layer().read(cx).has_modal())
    );
    assert!(f.toasts(cx).is_empty());
    assert!(f.events().is_empty());
    assert_eq!(f.library.read_with(cx, |l, _| l.tree()), before);

    // While typing a filter the arrows stay among the matches.
    cx.simulate_keystrokes("/");
    cx.simulate_input("blog");
    assert_eq!(f.selected(cx), Some(Folder(blog)));
    cx.simulate_keystrokes("up up");
    assert_eq!(f.selected(cx), Some(Folder(blog)));
    assert!(!on_root(cx));
    cx.simulate_keystrokes("escape");

    // The root row is "nothing selected" for the stored selection, and it survives the
    // panel losing and getting the keyboard.
    cx.simulate_keystrokes("home up");
    cx.executor().advance_clock(Duration::from_millis(500));
    cx.run_until_parked();
    let stored: Option<LibraryItem> = f.library.read_with(cx, |l, _| {
        l.setting_or(writer_library::SELECTED_ITEM_SETTING, Some(Folder(blog)))
    });
    assert_eq!(stored, None);
    cx.update(|window, _| window.blur());
    cx.run_until_parked();
    let panel = f.panel.clone();
    cx.update(|window, cx| window.focus(&panel.focus_handle(cx)));
    cx.run_until_parked();
    assert!(on_root(cx));

    // The API says the same.
    f.panel.update(cx, |p, cx| p.select(Some(Folder(blog)), cx));
    assert!(!on_root(cx));
    f.panel.update(cx, |p, cx| p.select_root(cx));
    assert!(on_root(cx));
    assert_eq!(f.selected(cx), None);
}

#[gpui::test]
fn an_empty_library(cx: &mut TestAppContext) {
    cx.update(|cx| {
        writer_ui::theme::init(writer_ui::ThemeMode::Light, cx);
        writer_ui::init(cx);
        writer_library::init(cx);
    });
    let (library, _) = library_from(Library::in_memory().unwrap(), cx);
    let (panel, cx) = cx.add_window_view({
        let library = library.clone();
        move |window, cx| {
            let mut panel = LibraryPanel::new(library, cx);
            panel.set_new_document_content("{}");
            window.focus(&panel.focus_handle(cx));
            panel
        }
    });
    cx.run_until_parked();
    let state = |cx: &mut VisualTestContext| {
        panel.read_with(cx, |p, _| {
            (
                p.selected(),
                p.is_root_row_selected(),
                p.is_trash_row_selected(),
            )
        })
    };
    assert_eq!(state(cx), (None, false, false));
    // Nothing to select but the two fixed rows; no key does harm.
    cx.simulate_keystrokes("down");
    assert_eq!(state(cx), (None, false, true));
    cx.simulate_keystrokes("up");
    assert_eq!(state(cx), (None, true, false));
    cx.simulate_keystrokes(
        "up left right home end pageup pagedown enter r m cmd-d alt-up cmd-backspace",
    );
    assert_eq!(state(cx), (None, true, false));
    cx.simulate_keystrokes("down");
    assert_eq!(state(cx), (None, false, true));
    cx.simulate_keystrokes("up cmd-z");
    // The first document.
    cx.simulate_keystrokes("n");
    cx.simulate_input("First");
    cx.simulate_keystrokes("enter");
    assert_eq!(panel.read_with(cx, |p, _| p.visible_items().len()), 1);
    assert_eq!(
        library.read_with(cx, |l, _| l.tree().documents[0].title.clone()),
        "First"
    );
    // Trashing the only item leaves the list empty and the keyboard in the panel.
    cx.simulate_keystrokes("cmd-backspace");
    assert_eq!(state(cx), (None, false, false));
    assert!(cx.update(|window, cx| panel.focus_handle(cx).is_focused(window)));
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(panel.read_with(cx, |p, _| p.visible_items().len()), 1);
}

// ----- undo -------------------------------------------------------------------------------

#[gpui::test]
fn undo_covers_renames_moves_and_reorders(cx: &mut TestAppContext) {
    let (f, cx) = fixture(cx);
    let b = f.create(Some(f.folder), "B", cx);
    let c = f.create(Some(f.folder), "C", cx);
    cx.run_until_parked();
    f.panel.update(cx, |p, cx| {
        p.set_new_document_content("{}");
        p.reveal_document(f.in_folder, cx);
    });
    assert!(!f.panel.read_with(cx, |p, _| p.can_undo()));

    // A rename, of a document and of a folder.
    cx.simulate_keystrokes("r");
    cx.simulate_input("Opening");
    cx.simulate_keystrokes("enter");
    assert_eq!(f.title(f.in_folder, cx), "Opening");
    assert!(f.panel.read_with(cx, |p, _| p.can_undo()));
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(f.title(f.in_folder, cx), "Chapter");
    assert_eq!(f.toasts(cx), vec!["Renamed “Opening” back to “Chapter”."]);
    assert_eq!(f.selected(cx), Some(Document(f.in_folder)));
    f.panel
        .update(cx, |p, cx| p.select(Some(Folder(f.inner)), cx));
    cx.simulate_keystrokes("f2");
    cx.simulate_input("Scraps");
    cx.simulate_keystrokes("enter cmd-z");
    assert_eq!(f.folder_name(f.inner, cx), "Notes");
    assert_eq!(f.toasts(cx), vec!["Renamed “Scraps” back to “Notes”."]);
    // A document that had no title gets none back.
    f.panel
        .update(cx, |p, cx| p.select(Some(Document(f.at_root)), cx));
    cx.simulate_keystrokes("r");
    cx.simulate_input("Named");
    cx.simulate_keystrokes("enter cmd-z");
    assert_eq!(f.title(f.at_root, cx), "");
    assert_eq!(f.toasts(cx), vec!["Renamed “Named” back to “Untitled”."]);
    // An unchanged or cancelled name is not a change.
    cx.simulate_keystrokes("r enter r escape cmd-z");
    assert_eq!(f.toasts(cx), vec!["Nothing to undo."]);

    // A run of reorders is one change: back to where it started.
    f.panel
        .update(cx, |p, cx| p.select(Some(Document(f.in_folder)), cx));
    cx.simulate_keystrokes("alt-down alt-down alt-up alt-down");
    assert_eq!(f.documents(Some(f.folder), cx), vec![b, c, f.in_folder]);
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(f.documents(Some(f.folder), cx), vec![f.in_folder, b, c]);
    assert_eq!(f.toasts(cx), vec!["Moved “Chapter” back to its place."]);
    assert_eq!(f.selected(cx), Some(Document(f.in_folder)));
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(f.toasts(cx), vec!["Nothing to undo."]);
    // Folders too.
    let other = f
        .library
        .update(cx, |l, cx| l.create_folder(None, "Other", cx).unwrap().id);
    cx.run_until_parked();
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
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(root_folders(cx), vec![f.folder, other]);
    f.toasts(cx);

    // A move to another folder: back into the old folder, at the old place.
    f.panel.update(cx, |p, cx| {
        p.move_item(Document(b), None, cx).unwrap();
    });
    assert_eq!(f.documents(None, cx), vec![f.at_root, b]);
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(f.documents(Some(f.folder), cx), vec![f.in_folder, b, c]);
    assert_eq!(f.documents(None, cx), vec![f.at_root]);
    assert_eq!(f.toasts(cx), vec!["Moved “B” back to Book."]);
    assert_eq!(f.selected(cx), Some(Document(b)));
    // With the picker, a folder, out to the root and back.
    f.panel
        .update(cx, |p, cx| p.select(Some(Folder(f.inner)), cx));
    cx.simulate_keystrokes("m");
    cx.simulate_input("library root");
    cx.simulate_keystrokes("enter");
    assert_eq!(root_folders(cx), vec![f.folder, other, f.inner]);
    assert_eq!(f.toasts(cx), vec!["Moved “Notes” to the library root."]);
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(root_folders(cx), vec![f.folder, other]);
    assert_eq!(f.folder_of(f.in_inner, cx), Some(f.inner));
    assert_eq!(f.toasts(cx), vec!["Moved “Notes” back to Book."]);

    // Changes of different kinds come back in reverse order.
    f.panel.update(cx, |p, cx| p.select(Some(Document(c)), cx));
    cx.simulate_keystrokes("r");
    cx.simulate_input("C2");
    cx.simulate_keystrokes("enter");
    f.panel.update(cx, |p, cx| {
        p.move_item(Document(c), Some(f.inner), cx).unwrap();
    });
    cx.simulate_keystrokes("cmd-backspace");
    cx.run_until_parked();
    f.toasts(cx);
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(f.toasts(cx), vec!["Restored “C2” to Book / Notes."]);
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(f.toasts(cx), vec!["Moved “C2” back to Book."]);
    assert_eq!(f.documents(Some(f.folder), cx), vec![f.in_folder, b, c]);
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(f.toasts(cx), vec!["Renamed “C2” back to “C”."]);
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(f.toasts(cx), vec!["Nothing to undo."]);

    // Giving a new item its first name is not a rename to take back.
    cx.simulate_keystrokes("n");
    cx.simulate_input("Fresh");
    cx.simulate_keystrokes("enter shift-n");
    cx.simulate_input("Shelf");
    cx.simulate_keystrokes("enter cmd-z");
    assert_eq!(f.toasts(cx), vec!["Nothing to undo."]);
    f.events();

    // What can no longer be taken back is skipped: the folder the document came from is
    // deleted for good, and the document itself after a rename.
    f.panel.update(cx, |p, cx| {
        p.move_item(Document(b), None, cx).unwrap();
        p.select(Some(Document(f.at_root)), cx);
    });
    cx.simulate_keystrokes("r");
    cx.simulate_input("Doomed");
    cx.simulate_keystrokes("enter");
    f.library.update(cx, |l, cx| {
        l.trash_folder(f.folder, cx).unwrap();
        l.purge_folder(f.folder, cx).unwrap();
        l.trash_document(f.at_root, cx).unwrap();
        l.purge_document(f.at_root, cx).unwrap();
    });
    cx.run_until_parked();
    f.toasts(cx);
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(f.toasts(cx), vec!["Nothing to undo."]);
    assert_eq!(f.folder_of(b, cx), None);
}

// ----- a rename and the virtualised list ---------------------------------------------------

#[gpui::test]
fn the_row_being_renamed_stays_on_screen(cx: &mut TestAppContext) {
    let (f, cx) = fixture(cx);
    let ids: Vec<DocumentId> = f.library.update(cx, |l, cx| {
        (0..400)
            .map(|ix| {
                l.create_document(None, &content(&format!("Note {ix:03}"), "x"), cx)
                    .unwrap()
                    .id
            })
            .collect()
    });
    cx.run_until_parked();
    // The rename field is drawn (it is part of the last frame) and has the keyboard.
    let field_is_live = |cx: &mut VisualTestContext| {
        let panel = f.panel.clone();
        cx.update(|window, cx| {
            let panel = panel.read(cx);
            panel.rename_field().is_some_and(|field| {
                let field = field.focus_handle(cx);
                field.is_focused(window) && panel.focus_handle(cx).contains(&field, window)
            })
        })
    };
    let wheel = |delta: f32, cx: &mut VisualTestContext| {
        cx.simulate_event(ScrollWheelEvent {
            position: Fixture::row_point(3, 120.),
            delta: ScrollDelta::Pixels(point(px(0.), px(delta))),
            modifiers: Modifiers::default(),
            touch_phase: TouchPhase::Moved,
        });
        cx.run_until_parked();
    };

    // Without a rename the wheel scrolls rows out of the list as usual.
    let target = ids[4];
    f.panel
        .update(cx, |p, cx| p.select(Some(Document(target)), cx));
    cx.run_until_parked();
    f.panel.update_in(cx, |p, window, cx| {
        p.begin_rename(Document(target), window, cx)
    });
    cx.run_until_parked();
    assert!(field_is_live(cx));

    // Scrolling far down, and back up past the top, cannot take the row away.
    for delta in [-800., -6000., -6000., 9000., -300.] {
        wheel(delta, cx);
        assert_eq!(f.renaming(cx), Some(Document(target)));
        assert!(field_is_live(cx), "after scrolling by {delta}");
    }
    cx.simulate_input("Kept while scrolling");
    cx.simulate_keystrokes("enter");
    assert_eq!(f.title(target, cx), "Kept while scrolling");
    assert!(f.panel_focused(cx));
    // After the rename the list scrolls freely again.
    wheel(-6000., cx);
    assert_eq!(f.renaming(cx), None);

    // A row far down the list: the rename brings it into view, and the library changing
    // above it (rows inserted, an autosave elsewhere) does not push it out.
    let far = ids[350];
    f.panel.update_in(cx, |p, window, cx| {
        p.begin_rename(Document(far), window, cx)
    });
    cx.run_until_parked();
    assert!(field_is_live(cx));
    f.library.update(cx, |l, cx| {
        for ix in 0..60 {
            l.create_folder(None, &format!("Shelf {ix:02}"), cx)
                .unwrap();
        }
    });
    cx.run_until_parked();
    assert_eq!(f.renaming(cx), Some(Document(far)));
    assert!(field_is_live(cx));
    cx.simulate_input("Far and kept");
    cx.simulate_keystrokes("enter");
    assert_eq!(f.title(far, cx), "Far and kept");

    // When the row leaves the list altogether (its folder is closed under it), the rename
    // ends, the typed name is kept and the keyboard goes to the list.
    f.panel.update_in(cx, |p, window, cx| {
        p.begin_rename(Document(f.in_inner), window, cx)
    });
    cx.run_until_parked();
    cx.simulate_input("Closed over");
    f.panel
        .update(cx, |p, cx| p.set_expanded(f.folder, false, cx));
    cx.run_until_parked();
    assert_eq!(f.renaming(cx), None);
    assert_eq!(f.title(f.in_inner, cx), "Closed over");
    assert!(f.panel_focused(cx));
    assert_eq!(f.selected(cx), Some(Folder(f.folder)));
    // So does a filter set from outside that hides the row.
    f.panel.update_in(cx, |p, window, cx| {
        p.begin_rename(Document(f.in_folder), window, cx)
    });
    cx.run_until_parked();
    cx.simulate_input("Filtered away");
    f.panel.update(cx, |p, cx| p.set_filter("note 00", cx));
    cx.run_until_parked();
    assert_eq!(f.renaming(cx), None);
    assert_eq!(f.title(f.in_folder, cx), "Filtered away");
    // A name that was not changed is not written again when its row is hidden.
    f.panel.update(cx, |p, cx| p.clear_filter(cx));
    let before = f
        .library
        .read_with(cx, |l, _| l.document_meta(f.in_inner).unwrap());
    f.clock.advance_ms(60_000);
    f.panel.update_in(cx, |p, window, cx| {
        p.begin_rename(Document(f.in_inner), window, cx)
    });
    cx.run_until_parked();
    f.panel
        .update(cx, |p, cx| p.set_expanded(f.folder, false, cx));
    cx.run_until_parked();
    assert_eq!(f.renaming(cx), None);
    assert_eq!(
        f.library
            .read_with(cx, |l, _| l.document_meta(f.in_inner).unwrap()),
        before
    );
}

// ----- the panel off screen ----------------------------------------------------------------

/// A window root whose sidebar can be closed, as the app's can.
struct HidingShell {
    panel: Entity<LibraryPanel>,
    editor: FocusHandle,
    sidebar: bool,
}

impl Render for HidingShell {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        writer_library::forward_actions(div().size_full().flex(), &self.panel)
            .when(self.sidebar, |el| {
                el.child(div().w(px(280.)).h_full().child(self.panel.clone()))
            })
            .child(div().flex_1().track_focus(&self.editor))
    }
}

#[gpui::test]
fn commands_while_the_panel_is_hidden(cx: &mut TestAppContext) {
    cx.update(|cx| {
        writer_ui::theme::init(writer_ui::ThemeMode::Light, cx);
        writer_ui::init(cx);
        writer_library::init(cx);
    });
    let (library, _) = library_from(Library::in_memory().unwrap(), cx);
    let folder = library.update(cx, |l, cx| {
        let folder = l.create_folder(None, "Book", cx).unwrap().id;
        l.create_document(Some(folder), &content("Chapter", "x"), cx)
            .unwrap();
        folder
    });
    let events: Rc<RefCell<Vec<LibraryPanelEvent>>> = Rc::default();
    let (shell, cx) = cx.add_window_view({
        let library = library.clone();
        let events = events.clone();
        move |window, cx| {
            let panel = cx.new(|cx| LibraryPanel::new(library, cx));
            let editor = cx.focus_handle();
            // The host gives the keyboard to the editor when the panel lets go of it.
            cx.subscribe_in(&panel, window, {
                let editor = editor.clone();
                move |_, _, event: &LibraryPanelEvent, window, _| {
                    events.borrow_mut().push(event.clone());
                    if *event == LibraryPanelEvent::Dismissed {
                        window.focus(&editor);
                    }
                }
            })
            .detach();
            window.focus(&editor);
            HidingShell {
                panel,
                editor,
                sidebar: false,
            }
        }
    });
    let panel = shell.read_with(cx, |shell, _| shell.panel.clone());
    let editor = shell.read_with(cx, |shell, _| shell.editor.clone());
    cx.run_until_parked();
    let on_screen = |cx: &mut VisualTestContext| {
        let panel = panel.clone();
        cx.update(|window, cx| panel.read(cx).is_on_screen(window))
    };
    let editor_focused = |cx: &mut VisualTestContext| {
        let editor = editor.clone();
        cx.update(|window, _| editor.is_focused(window))
    };
    let wait = |cx: &mut VisualTestContext| {
        cx.executor().advance_clock(Duration::from_millis(700));
        cx.run_until_parked();
    };
    let take_events = || events.borrow_mut().drain(..).collect::<Vec<_>>();
    assert!(!on_screen(cx));
    panel.update(cx, |p, cx| p.select(Some(Folder(folder)), cx));

    // A host that does not show the panel on request: the command still does what it can
    // (the folder is created), the rename ends, and the keyboard comes back to the editor
    // instead of staying on a field that is not drawn.
    cx.dispatch_action(writer_library::NewFolder);
    let Some(Folder(created)) = panel.read_with(cx, |p, _| p.renaming()) else {
        panic!("not naming the new folder");
    };
    assert!(!editor_focused(cx));
    wait(cx);
    assert_eq!(panel.read_with(cx, |p, _| p.renaming()), None);
    assert_eq!(take_events(), vec![LibraryPanelEvent::Dismissed]);
    assert!(editor_focused(cx));
    assert_eq!(
        library.read_with(cx, |l, _| l.folder(created).unwrap().name),
        "New folder"
    );
    // The same for the filter and the trash view.
    cx.dispatch_action(writer_library::FocusFilter);
    wait(cx);
    assert_eq!(take_events(), vec![LibraryPanelEvent::Dismissed]);
    assert!(editor_focused(cx));
    cx.dispatch_action(writer_library::ShowTrash);
    wait(cx);
    assert_eq!(take_events(), vec![LibraryPanelEvent::Dismissed]);
    assert!(editor_focused(cx));
    panel.update(cx, |p, cx| p.show_tree(cx));
    // Commands that do not need the keyboard in the panel are not disturbed.
    panel.update(cx, |p, cx| p.select(Some(Folder(created)), cx));
    cx.dispatch_action(writer_library::MoveToTrash);
    wait(cx);
    assert!(take_events().is_empty());
    assert!(editor_focused(cx));
    cx.dispatch_action(writer_library::UndoTrash);
    cx.run_until_parked();

    // A host that shows the panel when asked: the command works as if the sidebar had
    // been open.
    panel.update(cx, |p, _| {
        let shell = shell.downgrade();
        p.set_reveal_handler(move |_, cx| {
            shell
                .update(cx, |shell, cx| {
                    shell.sidebar = true;
                    cx.notify();
                })
                .ok();
        });
    });
    panel.update(cx, |p, cx| p.select(Some(Folder(folder)), cx));
    cx.dispatch_action(writer_library::Rename);
    cx.run_until_parked();
    assert!(shell.read_with(cx, |shell, _| shell.sidebar));
    assert!(on_screen(cx));
    assert_eq!(
        panel.read_with(cx, |p, _| p.renaming()),
        Some(Folder(folder))
    );
    wait(cx);
    assert_eq!(
        panel.read_with(cx, |p, _| p.renaming()),
        Some(Folder(folder))
    );
    assert!(take_events().is_empty());
    cx.simulate_input("Shown");
    cx.simulate_keystrokes("enter");
    assert_eq!(
        library.read_with(cx, |l, _| l.folder(folder).unwrap().name),
        "Shown"
    );
    assert!(cx.update(|window, cx| panel.focus_handle(cx).is_focused(window)));

    // Closed again by the host, the filter command opens it again.
    cx.update(|window, _| window.focus(&editor));
    shell.update(cx, |shell, cx| {
        shell.sidebar = false;
        cx.notify();
    });
    cx.run_until_parked();
    assert!(!on_screen(cx));
    cx.dispatch_action(writer_library::FocusFilter);
    cx.run_until_parked();
    assert!(on_screen(cx));
    cx.simulate_input("chap");
    assert_eq!(panel.read_with(cx, |p, _| p.filter().to_string()), "chap");
    wait(cx);
    assert!(take_events().is_empty());
}

// ----- at the edges -----------------------------------------------------------------------

#[gpui::test]
fn moves_and_reorders_at_the_edges(cx: &mut TestAppContext) {
    let (f, cx) = fixture(cx);
    let empty = f
        .library
        .update(cx, |l, cx| l.create_folder(None, "Empty", cx).unwrap().id);
    let last = f.create(None, "Last", cx);
    cx.run_until_parked();
    let folders = |parent: Option<FolderId>, cx: &mut VisualTestContext| -> Vec<FolderId> {
        f.library.read_with(cx, |l, _| {
            let tree = l.tree();
            match parent {
                None => tree.folders.iter().map(|n| n.folder.id).collect(),
                Some(id) => tree
                    .find_folder(id)
                    .unwrap()
                    .folders
                    .iter()
                    .map(|n| n.folder.id)
                    .collect(),
            }
        })
    };

    // Into an empty folder: it opens, the document is its only row, and stays selected.
    f.panel
        .update(cx, |p, cx| p.reveal_document(f.in_folder, cx));
    cx.simulate_keystrokes("m");
    cx.simulate_input("empty");
    cx.simulate_keystrokes("enter");
    assert_eq!(f.documents(Some(empty), cx), vec![f.in_folder]);
    assert_eq!(f.documents(Some(f.folder), cx), vec![]);
    assert_eq!(f.selected(cx), Some(Document(f.in_folder)));
    assert!(f.panel.read_with(cx, |p, _| p.is_expanded(empty)));
    assert_eq!(f.toasts(cx), vec!["Moved “Chapter” to Empty."]);
    // The only child has nowhere to go among its siblings.
    cx.simulate_keystrokes("alt-up alt-down");
    assert_eq!(f.documents(Some(empty), cx), vec![f.in_folder]);
    assert!(f.toasts(cx).is_empty());

    // Out to the root: after the documents that are there.
    cx.simulate_keystrokes("m");
    cx.simulate_input("library");
    cx.simulate_keystrokes("enter");
    assert_eq!(f.documents(None, cx), vec![f.at_root, last, f.in_folder]);
    assert_eq!(f.documents(Some(empty), cx), vec![]);
    assert_eq!(f.selected(cx), Some(Document(f.in_folder)));
    f.toasts(cx);
    // The folder it left is empty again: right does not step into anything.
    f.panel
        .update(cx, |p, cx| p.select(Some(Folder(empty)), cx));
    cx.simulate_keystrokes("right right");
    assert_eq!(f.selected(cx), Some(Folder(empty)));

    // Last among the root documents: down is the edge, up walks to the first place and
    // stops there, never crossing into the folders above.
    f.panel
        .update(cx, |p, cx| p.select(Some(Document(f.in_folder)), cx));
    cx.simulate_keystrokes("alt-down");
    assert_eq!(f.documents(None, cx), vec![f.at_root, last, f.in_folder]);
    cx.simulate_keystrokes("alt-up alt-up alt-up alt-up");
    assert_eq!(f.documents(None, cx), vec![f.in_folder, f.at_root, last]);
    assert_eq!(folders(None, cx), vec![f.folder, empty]);
    // The first folder cannot go above the first place, the last not below the last.
    f.panel
        .update(cx, |p, cx| p.select(Some(Folder(f.folder)), cx));
    cx.simulate_keystrokes("alt-up");
    assert_eq!(folders(None, cx), vec![f.folder, empty]);
    cx.simulate_keystrokes("alt-down alt-down alt-down");
    assert_eq!(folders(None, cx), vec![empty, f.folder]);
    assert_eq!(f.documents(None, cx), vec![f.in_folder, f.at_root, last]);
    assert_eq!(f.selected(cx), Some(Folder(f.folder)));

    // A folder with its contents into the empty folder, and out to the root again.
    cx.simulate_keystrokes("m");
    cx.simulate_input("empty");
    cx.simulate_keystrokes("enter");
    assert_eq!(folders(None, cx), vec![empty]);
    assert_eq!(folders(Some(empty), cx), vec![f.folder]);
    assert_eq!(f.folder_of(f.in_inner, cx), Some(f.inner));
    assert_eq!(f.selected(cx), Some(Folder(f.folder)));
    cx.simulate_keystrokes("m");
    cx.simulate_input("library root");
    cx.simulate_keystrokes("enter");
    assert_eq!(folders(None, cx), vec![empty, f.folder]);
    assert_eq!(folders(Some(empty), cx), vec![]);
    f.toasts(cx);

    // The API refuses what the picker does not offer, and says nothing was moved.
    let result = f
        .panel
        .update(cx, |p, cx| p.move_item(Folder(f.folder), Some(f.inner), cx));
    assert!(result.is_err());
    assert_eq!(folders(None, cx), vec![empty, f.folder]);
    // A reorder is refused while a filter hides the siblings.
    f.panel.update(cx, |p, cx| {
        p.set_filter("last", cx);
        p.select(Some(Document(last)), cx);
    });
    assert!(!f.panel.update(cx, |p, cx| p.reorder_selected(-1, cx)));
    assert_eq!(f.documents(None, cx), vec![f.in_folder, f.at_root, last]);
}

#[gpui::test]
fn trashing_the_folder_of_the_open_document(cx: &mut TestAppContext) {
    let (f, cx) = fixture(cx);
    // The editor shows "Idea", two folders down; the selection follows it.
    f.panel
        .update(cx, |p, cx| p.set_active_document(Some(f.in_inner), cx));
    cx.run_until_parked();
    assert_eq!(f.selected(cx), Some(Document(f.in_inner)));
    let library_events: Rc<RefCell<Vec<writer_library::LibraryEvent>>> = Rc::default();
    cx.update(|_, cx| {
        let events = library_events.clone();
        cx.subscribe(
            &f.library,
            move |_, event: &writer_library::LibraryEvent, _| {
                events.borrow_mut().push(event.clone())
            },
        )
        .detach();
    });

    // Trash the outer folder from the keyboard: left goes to "Notes", closes it, then
    // goes on to "Book".
    cx.simulate_keystrokes("left left left");
    assert_eq!(f.selected(cx), Some(Folder(f.folder)));
    cx.simulate_keystrokes("cmd-backspace");
    cx.run_until_parked();
    assert_eq!(f.visible(cx), vec![Document(f.at_root)]);
    // The app is told which documents went, so it can close the open one.
    let trashed = library_events
        .borrow()
        .iter()
        .find_map(|event| match event {
            writer_library::LibraryEvent::FolderTrashed { id, documents } => {
                Some((*id, documents.clone()))
            }
            _ => None,
        })
        .expect("no FolderTrashed event");
    assert_eq!(trashed.0, f.folder);
    assert!(trashed.1.contains(&f.in_inner) && trashed.1.contains(&f.in_folder));
    // The panel no longer marks it as open, and the selection is on a row that exists.
    assert_eq!(f.panel.read_with(cx, |p, _| p.active_document()), None);
    assert_eq!(f.selected(cx), Some(Document(f.at_root)));
    assert!(f.panel_focused(cx));
    assert_eq!(
        f.toasts(cx),
        vec!["Moved “Book” (2 documents, 1 subfolder) to the trash. Press ⌘Z to undo."]
    );

    // The editor's last autosave of the trashed document arrives late: the text is kept,
    // the list does not change.
    f.library.update(cx, |l, cx| {
        l.save_document(f.in_inner, &content("Idea", "the last words typed"), cx)
            .unwrap()
    });
    cx.run_until_parked();
    assert_eq!(f.visible(cx), vec![Document(f.at_root)]);

    // Undo brings the folder back with the late save in it; opening is the app's call.
    cx.simulate_keystrokes("cmd-z");
    cx.run_until_parked();
    assert_eq!(f.selected(cx), Some(Folder(f.folder)));
    assert_eq!(f.folder_of(f.in_inner, cx), Some(f.inner));
    assert_eq!(
        f.library
            .read_with(cx, |l, _| l.document_meta(f.in_inner).unwrap().word_count),
        4
    );
    assert_eq!(f.panel.read_with(cx, |p, _| p.active_document()), None);

    // The same when only the open document's own folder goes, and when the document does.
    f.panel
        .update(cx, |p, cx| p.set_active_document(Some(f.in_inner), cx));
    f.panel
        .update(cx, |p, cx| p.trash_item(Folder(f.inner), cx));
    cx.run_until_parked();
    assert_eq!(f.panel.read_with(cx, |p, _| p.active_document()), None);
    assert!(f.selected(cx).is_some());
    cx.simulate_keystrokes("cmd-z");
    f.panel
        .update(cx, |p, cx| p.set_active_document(Some(f.in_inner), cx));
    cx.simulate_keystrokes("cmd-backspace");
    cx.run_until_parked();
    assert_eq!(f.panel.read_with(cx, |p, _| p.active_document()), None);
    assert_eq!(f.selected(cx), Some(Document(f.in_folder)));
}

#[gpui::test]
fn restoring_when_the_original_folder_is_gone(cx: &mut TestAppContext) {
    let (f, cx) = fixture(cx);
    let location = |item: LibraryItem, cx: &mut VisualTestContext| {
        f.panel.read_with(cx, |p, _| p.trash_location(item))
    };
    // "Idea" is trashed on its own, then its folder "Notes", then the folder is deleted
    // for good.
    f.library.update(cx, |l, cx| {
        l.trash_document(f.in_inner, cx).unwrap();
    });
    cx.run_until_parked();
    assert_eq!(
        location(Document(f.in_inner), cx).as_deref(),
        Some("Book / Notes")
    );
    f.library
        .update(cx, |l, cx| l.trash_folder(f.inner, cx).unwrap());
    cx.run_until_parked();
    // The folder is in the trash itself: a restore of the document would go to the root,
    // and the trash view says so.
    assert_eq!(
        location(Document(f.in_inner), cx).as_deref(),
        Some("Library root")
    );
    f.library
        .update(cx, |l, cx| l.purge_folder(f.inner, cx).unwrap());
    cx.run_until_parked();
    assert_eq!(
        f.panel.read_with(cx, |p, _| p.trash_items()),
        vec![Document(f.in_inner)]
    );
    assert_eq!(
        location(Document(f.in_inner), cx).as_deref(),
        Some("Library root")
    );
    f.toasts(cx);

    // Restore it from the trash view.
    cx.simulate_keystrokes("t enter");
    assert_eq!(f.folder_of(f.in_inner, cx), None);
    assert_eq!(f.documents(None, cx), vec![f.at_root, f.in_inner]);
    assert_eq!(f.toasts(cx), vec!["Restored “Idea” to the library root."]);
    assert!(f.panel.read_with(cx, |p, _| p.trash_items()).is_empty());
    // Back in the tree it is selected and visible.
    cx.simulate_keystrokes("escape");
    assert_eq!(f.selected(cx), Some(Document(f.in_inner)));
    assert!(f.visible(cx).contains(&Document(f.in_inner)));

    // The folder is only in the trash, not deleted: the document still goes to the root,
    // and restoring the folder afterwards does not take it back.
    f.library.update(cx, |l, cx| {
        l.trash_document(f.in_folder, cx).unwrap();
        l.trash_folder(f.folder, cx).unwrap();
    });
    cx.run_until_parked();
    f.toasts(cx);
    // cmd-z undoes the folder first, so undo the document through the trash view instead.
    cx.simulate_keystrokes("t");
    f.panel.read_with(cx, |p, _| {
        assert_eq!(
            p.trash_items(),
            vec![Folder(f.folder), Document(f.in_folder)]
        );
        assert_eq!(
            p.trash_location(Document(f.in_folder)).as_deref(),
            Some("Library root")
        );
    });
    cx.simulate_keystrokes("down r");
    assert_eq!(f.folder_of(f.in_folder, cx), None);
    assert_eq!(
        f.toasts(cx),
        vec!["Restored “Chapter” to the library root."]
    );
    cx.simulate_keystrokes("r");
    assert_eq!(f.toasts(cx), vec!["Restored “Book” to the library root."]);
    assert_eq!(f.folder_of(f.in_folder, cx), None);
    assert_eq!(f.documents(Some(f.folder), cx), vec![]);
    // The trash is empty; restore and delete have nothing to act on.
    cx.simulate_keystrokes("r enter cmd-backspace");
    assert!(f.toasts(cx).is_empty());
    assert!(
        !f.panel
            .read_with(cx, |p, cx| p.modal_layer().read(cx).has_modal())
    );
}

#[gpui::test]
fn a_filter_with_no_results(cx: &mut TestAppContext) {
    let (f, cx) = fixture(cx);
    f.panel.update(cx, |p, cx| {
        p.set_new_document_content("{}");
        p.select(Some(Folder(f.folder)), cx);
    });
    let state = |cx: &mut VisualTestContext| {
        f.panel.read_with(cx, |p, _| {
            (
                p.visible_items().len(),
                p.selected(),
                p.is_root_row_selected(),
                p.is_trash_row_selected(),
            )
        })
    };
    cx.simulate_keystrokes("/");
    cx.simulate_input("zzz");
    assert_eq!(state(cx), (0, None, false, false));
    // In the field nothing can be selected or opened, and enter keeps the keyboard there
    // so the text can be corrected.
    cx.simulate_keystrokes("down up home end pagedown pageup enter");
    assert_eq!(state(cx), (0, None, false, false));
    assert!(f.events().is_empty());
    assert!(!f.panel_focused(cx));
    // One character less and the matches are back, the first one selected.
    cx.simulate_keystrokes("backspace backspace backspace");
    cx.simulate_input("ide");
    assert_eq!(f.selected(cx), Some(Document(f.in_inner)));
    cx.simulate_input("zz");
    assert_eq!(state(cx).0, 0);

    // tab moves to the (empty) list with the filter on: only the fixed rows are there.
    cx.simulate_keystrokes("tab");
    assert!(f.panel_focused(cx));
    cx.simulate_keystrokes("down");
    assert_eq!(state(cx), (0, None, false, true));
    cx.simulate_keystrokes("up");
    assert_eq!(state(cx), (0, None, true, false));
    cx.simulate_keystrokes("j k right left enter r cmd-d m alt-up cmd-backspace");
    assert_eq!(state(cx).0, 0);
    assert_eq!(f.renaming(cx), None);
    assert!(f.toasts(cx).is_empty());
    assert!(f.events().is_empty());
    // A new document can still be made (at the top level); the filter gives way so that
    // its row can be named.
    cx.simulate_keystrokes("up n");
    assert!(matches!(f.renaming(cx), Some(Document(_))));
    assert_eq!(f.panel.read_with(cx, |p, _| p.filter().to_string()), "");
    cx.simulate_input("Found nothing");
    cx.simulate_keystrokes("enter");
    assert_eq!(f.documents(None, cx).len(), 2);
    f.events();

    // escape in the list clears a filter that matches nothing and keeps the keyboard.
    cx.simulate_keystrokes("/");
    cx.simulate_input("qqq");
    cx.simulate_keystrokes("tab escape");
    assert_eq!(f.panel.read_with(cx, |p, _| p.filter().to_string()), "");
    assert_eq!(f.visible(cx).len(), 3);
    assert!(f.panel_focused(cx));
    assert!(f.events().is_empty());
}

#[gpui::test]
fn long_names_and_deep_nesting(cx: &mut TestAppContext) {
    let (f, cx) = fixture(cx);
    let long =
        "A very long working title that keeps going well past the edge of any sidebar ".repeat(8);
    let long = long.trim();
    // A document and a folder with very long names, the folder twelve levels down.
    let document = f.create(None, long, cx);
    let mut parent = f.inner;
    let mut chain = Vec::new();
    for depth in 0..12 {
        // The deepest one ends in letters no other name has, for the move picker.
        let tail = if depth == 11 { " zq" } else { "" };
        parent = f.library.update(cx, |l, cx| {
            l.create_folder(Some(parent), &format!("{long} {depth}{tail}"), cx)
                .unwrap()
                .id
        });
        chain.push(parent);
    }
    let deep = f.create(Some(parent), "Deep", cx);
    cx.run_until_parked();
    // Everything draws, at every depth, with the long rows selected.
    assert!(f.panel.update(cx, |p, cx| p.reveal_document(deep, cx)));
    cx.run_until_parked();
    assert_eq!(f.visible(cx).len(), 2 + 12 + 1 + 1 + 3);
    cx.simulate_keystrokes("up up home end");
    assert_eq!(f.selected(cx), Some(Document(document)));

    // The name is whole in the rename field; a longer one is stored whole.
    cx.simulate_keystrokes("r");
    let field_text = f.panel.read_with(cx, |p, cx| {
        p.rename_field().unwrap().read(cx).text().to_string()
    });
    assert_eq!(field_text, long);
    cx.simulate_keystrokes("end");
    cx.simulate_input(" and then some");
    cx.simulate_keystrokes("enter");
    assert_eq!(f.title(document, cx), format!("{long} and then some"));

    // The filter finds words anywhere in a long name, and in the deep folder names.
    cx.simulate_keystrokes("/");
    cx.simulate_input("then some");
    assert_eq!(f.visible(cx), vec![Document(document)]);
    cx.simulate_keystrokes("escape /");
    cx.simulate_input("sidebar 11");
    // The matching folder, what is in it, and the fourteen folders that lead to it.
    assert_eq!(f.visible(cx).len(), 2 + 11 + 1 + 1);
    assert_eq!(f.selected(cx), Some(Folder(chain[11])));
    cx.simulate_keystrokes("escape");

    // Messages carry the whole name; the move picker lists the whole path.
    f.panel
        .update(cx, |p, cx| p.select(Some(Document(document)), cx));
    cx.simulate_keystrokes("m");
    cx.simulate_input("zq");
    cx.simulate_keystrokes("enter");
    assert_eq!(f.folder_of(document, cx), Some(chain[11]));
    let toast = f.toasts(cx).pop().unwrap();
    assert!(toast.starts_with("Moved “A very long working title"));
    assert!(toast.contains(" / "));
    cx.simulate_keystrokes("cmd-backspace");
    cx.run_until_parked();
    let toast = f.toasts(cx).pop().unwrap();
    assert!(toast.ends_with("and then some” to the trash. Press ⌘Z to undo."));
    // The trash view shows it with the long path it came from.
    cx.simulate_keystrokes("t");
    cx.run_until_parked();
    let location = f
        .panel
        .read_with(cx, |p, _| p.trash_location(Document(document)).unwrap());
    assert_eq!(location.matches(" / ").count(), 13);
    cx.simulate_keystrokes("enter escape");
    assert_eq!(f.selected(cx), Some(Document(document)));
}

#[gpui::test]
fn several_hundred_siblings(cx: &mut TestAppContext) {
    let (f, cx) = fixture(cx);
    const COUNT: usize = 600;
    let ids: Vec<DocumentId> = f.library.update(cx, |l, cx| {
        (0..COUNT)
            .map(|ix| {
                l.create_document(Some(f.inner), &content(&format!("Scene {ix:03}"), "x"), cx)
                    .unwrap()
                    .id
            })
            .collect()
    });
    cx.run_until_parked();
    // "Idea" was there first; the scenes follow it.
    let order = |cx: &mut VisualTestContext| f.documents(Some(f.inner), cx);
    assert_eq!(order(cx).len(), COUNT + 1);
    f.panel.update(cx, |p, cx| {
        p.reveal_document(ids[COUNT - 1], cx);
    });
    cx.run_until_parked();
    assert_eq!(f.visible(cx).len(), 4 + COUNT + 1);

    // The last sibling: down is the edge; up moves it one place at a time.
    cx.simulate_keystrokes("alt-down");
    assert_eq!(order(cx)[COUNT], ids[COUNT - 1]);
    let started = Instant::now();
    for _ in 0..25 {
        cx.simulate_keystrokes("alt-up");
    }
    let reordering = started.elapsed();
    eprintln!(
        "25 reorders among {} siblings took {reordering:?}",
        COUNT + 1
    );
    assert_eq!(order(cx)[COUNT - 25], ids[COUNT - 1]);
    assert_eq!(order(cx)[COUNT], ids[COUNT - 2]);
    assert_eq!(f.selected(cx), Some(Document(ids[COUNT - 1])));
    assert!(
        reordering < Duration::from_secs(20),
        "25 reorders took {reordering:?}"
    );
    // One undo takes the whole run back.
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(order(cx)[COUNT], ids[COUNT - 1]);
    assert_eq!(order(cx)[COUNT - 1], ids[COUNT - 2]);
    f.toasts(cx);

    // The first sibling: up is the edge.
    f.panel
        .update(cx, |p, cx| p.select(Some(Document(f.in_inner)), cx));
    cx.simulate_keystrokes("alt-up");
    assert_eq!(order(cx)[0], f.in_inner);
    cx.simulate_keystrokes("alt-down");
    assert_eq!(&order(cx)[..2], &[ids[0], f.in_inner]);

    // In the middle: a copy lands right after the original, a trashed row hands the
    // selection to the next one, and undo puts it back in its place.
    let middle = ids[300];
    f.panel
        .update(cx, |p, cx| p.select(Some(Document(middle)), cx));
    cx.simulate_keystrokes("cmd-d");
    let now = order(cx);
    let at = now.iter().position(|id| *id == middle).unwrap();
    let copy = now[at + 1];
    assert_eq!(f.title(copy, cx), "Scene 300 copy");
    assert_eq!(f.selected(cx), Some(Document(copy)));
    assert_eq!(now[at + 2], ids[301]);
    cx.simulate_keystrokes("cmd-backspace");
    cx.run_until_parked();
    assert_eq!(f.selected(cx), Some(Document(ids[301])));
    assert_eq!(order(cx).len(), COUNT + 1);
    f.toasts(cx);

    // Moving one out to the root and back keeps its place among the hundreds.
    f.panel.update(cx, |p, cx| {
        p.move_item(Document(middle), None, cx).unwrap();
    });
    assert_eq!(order(cx).len(), COUNT);
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(order(cx)[at], middle);
    assert_eq!(order(cx)[at + 1], ids[301]);

    // Page keys and the ends still land on rows.
    cx.simulate_keystrokes("end");
    assert_eq!(f.selected(cx), Some(Document(f.at_root)));
    cx.simulate_keystrokes("pageup pageup");
    assert!(matches!(f.selected(cx), Some(Document(_))));
    cx.simulate_keystrokes("home");
    assert_eq!(f.selected(cx), Some(Folder(f.folder)));
}

#[gpui::test]
fn an_autosave_touches_one_row(cx: &mut TestAppContext) {
    let (f, cx) = fixture(cx);
    f.panel.update(cx, |p, cx| {
        p.reveal_document(f.in_inner, cx);
        p.set_row_detail(RowDetail::Words, cx);
    });
    cx.run_until_parked();
    let detail = |id: DocumentId, cx: &mut VisualTestContext| {
        f.panel
            .read_with(cx, |p, cx| p.item_detail(Document(id), cx))
    };
    assert_eq!(detail(f.in_inner, cx).as_deref(), Some("1"));
    let rows = f.visible(cx);

    // The editor saves the open document: its row shows the new count, nothing else moves.
    f.library.update(cx, |l, cx| {
        l.save_document(f.in_inner, &content("Idea", "one two three four five"), cx)
            .unwrap()
    });
    cx.run_until_parked();
    assert_eq!(detail(f.in_inner, cx).as_deref(), Some("5"));
    assert_eq!(detail(f.in_folder, cx).as_deref(), Some("1"));
    assert_eq!(f.visible(cx), rows);
    assert_eq!(f.selected(cx), Some(Document(f.in_inner)));
    // The edit time follows too.
    f.panel
        .update(cx, |p, cx| p.set_row_detail(RowDetail::Edited, cx));
    f.clock.advance_ms(3 * 60_000);
    assert_eq!(detail(f.in_inner, cx).as_deref(), Some("3m"));
    f.library.update(cx, |l, cx| {
        l.save_document(f.in_inner, &content("Idea", "one two"), cx)
            .unwrap()
    });
    cx.run_until_parked();
    assert_eq!(detail(f.in_inner, cx).as_deref(), Some("now"));

    // A row that is not on screen (its folder is closed) needs nothing; opening the
    // folder later shows the saved numbers.
    f.panel.update(cx, |p, cx| {
        p.set_expanded(f.inner, false, cx);
        p.set_row_detail(RowDetail::Words, cx);
    });
    f.library.update(cx, |l, cx| {
        l.save_document(f.in_inner, &content("Idea", "a b c d e f g"), cx)
            .unwrap()
    });
    cx.run_until_parked();
    assert_eq!(detail(f.in_inner, cx), None);
    f.panel
        .update(cx, |p, cx| p.set_expanded(f.inner, true, cx));
    assert_eq!(detail(f.in_inner, cx).as_deref(), Some("7"));

    // Under a filter the row is updated just the same.
    f.panel.update(cx, |p, cx| p.set_filter("idea", cx));
    f.library.update(cx, |l, cx| {
        l.save_document(f.in_inner, &content("Idea", "a b"), cx)
            .unwrap()
    });
    cx.run_until_parked();
    assert_eq!(detail(f.in_inner, cx).as_deref(), Some("2"));
    // A save that changes the title is a change of the list: the filter is applied again.
    f.library.update(cx, |l, cx| {
        l.save_document(f.in_inner, &content("Thought", "a b"), cx)
            .unwrap()
    });
    cx.run_until_parked();
    assert!(f.visible(cx).is_empty());
    f.panel.update(cx, |p, cx| p.clear_filter(cx));
    assert!(f.visible(cx).contains(&Document(f.in_inner)));
}
