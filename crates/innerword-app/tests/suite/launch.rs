//! A launch shows the library as it is.
//!
//! The command line tool lets agents change the library while the app is
//! closed. Whatever the app stored at its last quit (which document was
//! open, the caret and the scroll position, the selection and the open
//! folders of the sidebar, "the title of this document was chosen") may
//! refer to a document that has since been renamed, moved, rewritten,
//! trashed or deleted. Each test here is two launches with a change in
//! between, made the way the tool makes it: through a connection of its
//! own to the library file, while no app has it open.

use std::time::Duration;

use gpui::{AppContext as _, TestAppContext};
use innerword_app::{
    SaveState,
    document_io::StoredForm,
    settings::keys,
    view_memory::{self, TextStamp, TitleFlag},
    workspace::{AUTOSAVE_DELAY, CHANGED_OUTSIDE},
};
use innerword_core::{Position, Selection};
use innerword_library::{
    Library, LibraryItem,
    store::{OpenOptions, Store},
};

use super::*;

/// A library file in a temporary directory.
struct Disk {
    dir: tempfile::TempDir,
}

impl Disk {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().expect("a temporary directory"),
        }
    }

    fn path(&self) -> std::path::PathBuf {
        self.dir.path().join("library.db")
    }

    /// A change made while the app is closed, through a connection of its
    /// own (as the `innerword` tool has).
    fn outside<R>(&self, change: impl FnOnce(&mut Store) -> R) -> R {
        let mut store = Store::open_with(
            self.path(),
            OpenOptions {
                daily_backup: false,
                ..OpenOptions::default()
            },
        )
        .expect("the library opens a second time");
        change(&mut store)
    }
}

/// One launch: a library of its own over the file, and a workspace.
fn launch<'a>(
    disk: &Disk,
    cx: &'a mut TestAppContext,
) -> (
    Entity<Library>,
    Entity<Workspace>,
    &'a mut VisualTestContext,
) {
    let library = cx.new(|_| Library::open(disk.path()).expect("the library opens"));
    library.update(cx, |library, cx| {
        library
            .set_setting(keys::WELCOME_CREATED, &true, cx)
            .unwrap();
    });
    let (workspace, vcx) = open_workspace(&library, cx);
    (library, workspace, vcx)
}

/// The end of a launch: everything is saved (the sidebar stores its
/// selection a moment after it changed), and the window goes.
fn quit(workspace: &Entity<Workspace>, vcx: &mut VisualTestContext) {
    vcx.executor().advance_clock(Duration::from_secs(2));
    vcx.run_until_parked();
    workspace.update(vcx, |workspace, cx| {
        assert!(workspace.flush(cx), "the last save works");
    });
    vcx.update(|window, _| window.remove_window());
    vcx.run_until_parked();
}

fn toasts(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Vec<String> {
    workspace.read_with(cx, |w, cx| {
        w.toast_layer()
            .read(cx)
            .toasts()
            .map(|toast| toast.message.to_string())
            .collect()
    })
}

fn selection(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Selection {
    workspace.read_with(cx, |w, cx| w.editor().read(cx).selection())
}

fn title(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Option<String> {
    workspace.read_with(cx, |w, _| w.document_title().map(str::to_string))
}

/// Lets the autosave run and waits for a save on the background thread.
fn let_it_save(workspace: &Entity<Workspace>, vcx: &mut VisualTestContext) {
    vcx.executor().advance_clock(AUTOSAVE_DELAY * 2);
    vcx.run_until_parked();
    for _ in 0..200 {
        if workspace.read_with(vcx, |w, _| w.save_state()) == Some(SaveState::Saved) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
        vcx.run_until_parked();
    }
    panic!("the document was not saved");
}

/// A first launch that leaves `document` open with the caret at `caret`
/// (and, with `anchor`, a selection from there), then quits.
fn first_launch(
    disk: &Disk,
    document: &innerword_core::Document,
    anchor: Option<Position>,
    caret: Position,
    cx: &mut TestAppContext,
) -> DocumentId {
    let (library, workspace, vcx) = launch(disk, cx);
    let id = add_document(&library, None, document, vcx);
    workspace.update_in(vcx, |workspace, window, cx| {
        assert!(workspace.open_document(id, true, window, cx));
    });
    let editor = workspace.read_with(vcx, |w, _| w.editor().clone());
    editor.update(vcx, |editor, cx| {
        editor.edit(
            |state| state.set_selection(Selection::new(anchor.unwrap_or(caret), caret)),
            cx,
        );
    });
    quit(&workspace, vcx);
    id
}

#[gpui::test]
fn unchanged_in_between_everything_comes_back(cx: &mut TestAppContext) {
    init(cx);
    let disk = Disk::new();
    let document = simple_document("Notes", "First sentence. Second sentence.");
    let id = first_launch(
        &disk,
        &document,
        Some(Position::new(1, 6)),
        Position::new(1, 14),
        cx,
    );

    // The stored view state carries the stamp of the text it was taken on.
    let stored = disk.outside(|store| store.view_state(id).unwrap().unwrap());
    let value: serde_json::Value = serde_json::from_str(&stored).unwrap();
    assert_eq!(value["text"]["blocks"], 2);
    assert_eq!(
        value["text"]["bytes"],
        "Notes".len() + "First sentence. Second sentence.".len()
    );

    let (_library, workspace, vcx) = launch(&disk, cx);
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.open_document_id()),
        Some(id)
    );
    assert_eq!(
        selection(&workspace, vcx),
        Selection::new(Position::new(1, 6), Position::new(1, 14))
    );
    assert!(toasts(&workspace, vcx).is_empty());
}

#[gpui::test]
fn renamed_in_between_the_name_is_shown_and_kept(cx: &mut TestAppContext) {
    init(cx);
    let disk = Disk::new();
    let document = simple_document("Chapter 2: The Harbour", "By morning the fog had gone.");
    let id = first_launch(&disk, &document, None, Position::new(1, 2), cx);

    // An agent renames the document. Its title no longer is the one its
    // text gives.
    disk.outside(|store| store.rename_document(id, "The Harbour at Dawn").unwrap());

    let (library, workspace, vcx) = launch(&disk, cx);
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.open_document_id()),
        Some(id)
    );
    assert_eq!(
        title(&workspace, vcx).as_deref(),
        Some("The Harbour at Dawn")
    );
    // The caret is where it was: the text is the same text.
    assert_eq!(selection(&workspace, vcx).head, Position::new(1, 2));
    assert!(toasts(&workspace, vcx).is_empty());

    // The first edit and its save keep the name (a save derives the title
    // from the text unless the title was chosen).
    type_text(&workspace, "Later: ", vcx);
    assert_eq!(
        title(&workspace, vcx).as_deref(),
        Some("The Harbour at Dawn")
    );
    let_it_save(&workspace, vcx);
    let meta = library.read_with(vcx, |library, _| library.document_meta(id).unwrap());
    assert_eq!(meta.title, "The Harbour at Dawn");
    assert_eq!(
        title(&workspace, vcx).as_deref(),
        Some("The Harbour at Dawn")
    );
    assert!(stored_text(&library, id, vcx).contains("Later: "));

    // "Take title from text" still leads back.
    workspace.update_in(vcx, |workspace, window, cx| {
        workspace.use_title_from_text(window, cx)
    });
    vcx.run_until_parked();
    assert_eq!(
        title(&workspace, vcx).as_deref(),
        Some("Chapter 2: The Harbour")
    );
}

#[gpui::test]
fn a_duplicate_still_takes_its_title_from_the_text(cx: &mut TestAppContext) {
    init(cx);
    let disk = Disk::new();
    let document = simple_document("Template", "Fill me in.");
    let id = first_launch(&disk, &document, None, Position::new(1, 0), cx);
    let copy = disk.outside(|store| store.duplicate_document(id).unwrap());
    assert_eq!(copy.title, "Template copy");

    let (library, workspace, vcx) = launch(&disk, cx);
    workspace.update_in(vcx, |workspace, window, cx| {
        assert!(workspace.open_document(copy.id, true, window, cx));
    });
    assert_eq!(title(&workspace, vcx).as_deref(), Some("Template copy"));
    type_text(&workspace, "x", vcx);
    let_it_save(&workspace, vcx);
    // As it always did: the copy's name is the library's, not a chosen one.
    let meta = library.read_with(vcx, |library, _| library.document_meta(copy.id).unwrap());
    assert_ne!(meta.title, "Template copy");
    assert!(
        library
            .read_with(vcx, |library, _| library
                .setting::<TitleFlag>(&keys::custom_title(copy.id))
                .unwrap())
            .is_none()
    );
}

#[gpui::test]
fn moved_in_between_the_sidebar_shows_where_it_is(cx: &mut TestAppContext) {
    init(cx);
    let disk = Disk::new();
    let document = simple_document("Chapter 1", "The ferry came in late.");
    let id = first_launch(&disk, &document, None, Position::new(1, 9), cx);

    // An agent files the document in a folder inside a new folder.
    let (outer, inner) = disk.outside(|store| {
        let outer = store.create_folder(None, "The Salt Road").unwrap().id;
        let inner = store.create_folder(Some(outer), "Part one").unwrap().id;
        store.move_document(id, Some(inner)).unwrap();
        (outer, inner)
    });

    let (library, workspace, vcx) = launch(&disk, cx);
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.open_document_id()),
        Some(id)
    );
    assert_eq!(selection(&workspace, vcx).head, Position::new(1, 9));
    assert!(toasts(&workspace, vcx).is_empty());
    let panel = workspace.read_with(vcx, |w, _| w.panel().clone());
    panel.read_with(vcx, |panel, _| {
        assert!(panel.is_expanded(outer) && panel.is_expanded(inner));
        assert_eq!(panel.selected(), Some(LibraryItem::Document(id)));
        assert_eq!(
            panel.visible_items(),
            vec![
                LibraryItem::Folder(outer),
                LibraryItem::Folder(inner),
                LibraryItem::Document(id)
            ]
        );
    });
    assert_eq!(
        library.read_with(vcx, |library, _| library.folder_path(Some(inner))),
        vec!["The Salt Road".to_string(), "Part one".to_string()]
    );
    // A new document goes where the open one is now.
    let created = workspace
        .update_in(vcx, |workspace, window, cx| {
            workspace.new_document(window, cx)
        })
        .expect("a new document");
    let meta = library.read_with(vcx, |library, _| library.document_meta(created).unwrap());
    assert_eq!(meta.folder_id, Some(inner));
}

#[gpui::test]
fn rewritten_in_between_the_caret_does_not_pretend(cx: &mut TestAppContext) {
    init(cx);
    let disk = Disk::new();
    let document = innerword_core::Document::from_blocks(vec![
        Block::with_text(BlockKind::heading(1), "Essay"),
        Block::paragraph("One."),
        Block::paragraph("Two."),
        Block::paragraph("Three, and this is the sentence that was selected."),
    ]);
    // A selection in the last paragraph: the next key would replace it.
    let id = first_launch(
        &disk,
        &document,
        Some(Position::new(3, 7)),
        Position::new(3, 50),
        cx,
    );

    // An agent replaces the whole text with a shorter one.
    let rewritten = simple_document("Essay", "A new first paragraph by somebody else.");
    disk.outside(|store| {
        store
            .save_document(id, &StoredForm::of(&rewritten, None).content())
            .unwrap();
    });

    let (library, workspace, vcx) = launch(&disk, cx);
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.open_document_id()),
        Some(id)
    );
    assert_eq!(
        editor_text(&workspace, vcx),
        "Essay\nA new first paragraph by somebody else."
    );
    // No remembered selection over text nobody here has seen: the document
    // opens at its start, and says why.
    assert_eq!(
        selection(&workspace, vcx),
        Selection::caret(Position::new(0, 0))
    );
    assert_eq!(toasts(&workspace, vcx), vec![CHANGED_OUTSIDE.to_string()]);
    let label = workspace.read_with(vcx, |w, cx| {
        w.toast_layer()
            .read(cx)
            .toasts()
            .next()
            .and_then(|toast| toast.action_label().map(|label| label.to_string()))
    });
    assert_eq!(label.as_deref(), Some("Version History"));
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.save_state()),
        Some(SaveState::Saved),
        "opening changes nothing"
    );

    // Typing adds to the new text; nothing of it is replaced.
    type_text(&workspace, "My ", vcx);
    assert_eq!(
        editor_text(&workspace, vcx),
        "My Essay\nA new first paragraph by somebody else."
    );
    let_it_save(&workspace, vcx);
    assert!(stored_text(&library, id, vcx).starts_with("My Essay"));
    quit(&workspace, vcx);

    // Told once: the view state now belongs to the text as it is.
    let (_library, workspace, vcx) = launch(&disk, cx);
    assert_eq!(selection(&workspace, vcx).head, Position::new(0, 3));
    assert!(toasts(&workspace, vcx).is_empty());
}

#[gpui::test]
fn a_view_state_from_an_earlier_version_is_clamped(cx: &mut TestAppContext) {
    init(cx);
    let disk = Disk::new();
    let document = simple_document("Notes", "Short.");
    let id = first_launch(&disk, &document, None, Position::new(1, 0), cx);
    // As 0.2.0 stored it (no stamp), and pointing far beyond the text.
    disk.outside(|store| {
        store
            .save_view_state(
                id,
                r#"{"anchor_block":40,"anchor_offset":900,"head_block":41,"head_offset":7,"scroll_block":38,"scroll_offset":120.0}"#,
            )
            .unwrap();
    });
    let (_library, workspace, vcx) = launch(&disk, cx);
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.open_document_id()),
        Some(id)
    );
    let selection = selection(&workspace, vcx);
    for position in [selection.anchor, selection.head] {
        assert!(position.block < 2, "{position:?} is in the document");
        assert!(position.offset <= "Short.".len());
    }
    assert!(toasts(&workspace, vcx).is_empty());
    // The window draws and takes text.
    type_text(&workspace, "!", vcx);
    assert!(editor_text(&workspace, vcx).contains('!'));
}

#[gpui::test]
fn the_stamp_tells_another_text_from_the_same_one(cx: &mut TestAppContext) {
    init(cx);
    let _ = cx;
    let document = simple_document("Notes", "Short.");
    let view = innerword_editor::ViewState {
        head_block: 1,
        head_offset: 3,
        anchor_block: 1,
        anchor_offset: 3,
        ..Default::default()
    };
    let stored = view_memory::stamped(&view, &document).unwrap();
    assert_eq!(
        TextStamp::of(&document),
        TextStamp {
            blocks: 2,
            bytes: 11
        }
    );
    // The same text: remembered.
    let same = view_memory::remembered(Some(&stored), &document);
    assert_eq!(same.view, Some(view));
    assert!(!same.text_changed);
    // A word of the same length in its place: every position still holds.
    let same_length = view_memory::remembered(Some(&stored), &simple_document("Notes", "Shirt."));
    assert_eq!(same_length.view, Some(view));
    // Another text: not remembered.
    let other = view_memory::remembered(Some(&stored), &simple_document("Notes", "Longer text."));
    assert_eq!(other.view, None);
    assert!(other.text_changed);
    // Nothing stored, or not a view state.
    assert_eq!(
        view_memory::remembered(None, &document),
        view_memory::Remembered::default()
    );
    assert_eq!(
        view_memory::remembered(Some("not json"), &document),
        view_memory::Remembered::default()
    );
}

#[gpui::test]
fn trashed_in_between_another_document_opens(cx: &mut TestAppContext) {
    init(cx);
    let disk = Disk::new();
    // Two documents; the second is open at quit.
    let (library, workspace, vcx) = launch(&disk, cx);
    let kept = add_document(&library, None, &simple_document("Kept", "Still here."), vcx);
    quit(&workspace, vcx);
    let trashed = first_launch(
        &disk,
        &simple_document("Thrown away", "Gone."),
        None,
        Position::new(1, 2),
        cx,
    );
    assert_eq!(
        disk.outside(|store| store.setting::<DocumentId>(keys::LAST_DOCUMENT).unwrap()),
        Some(trashed)
    );

    disk.outside(|store| store.trash_document(trashed).unwrap());

    let (library, workspace, vcx) = launch(&disk, cx);
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.open_document_id()),
        Some(kept)
    );
    assert_eq!(editor_text(&workspace, vcx), "Kept\nStill here.");
    let panel = workspace.read_with(vcx, |w, _| w.panel().clone());
    panel.read_with(vcx, |panel, _| {
        assert_eq!(panel.visible_items(), vec![LibraryItem::Document(kept)]);
        assert_eq!(panel.selected(), Some(LibraryItem::Document(kept)));
    });
    assert!(toasts(&workspace, vcx).is_empty());
    // It is in the trash, and comes back from there with its text.
    let in_trash = library.read_with(vcx, |library, _| library.trash().unwrap().len());
    assert_eq!(in_trash, 1);
    library.update(vcx, |library, cx| {
        library.restore_document(trashed, cx).unwrap();
    });
    vcx.run_until_parked();
    workspace.update_in(vcx, |workspace, window, cx| {
        assert!(workspace.open_document(trashed, true, window, cx));
    });
    assert_eq!(editor_text(&workspace, vcx), "Thrown away\nGone.");
    assert_eq!(selection(&workspace, vcx).head, Position::new(1, 2));
}

#[gpui::test]
fn its_folder_trashed_in_between_nothing_points_into_the_trash(cx: &mut TestAppContext) {
    init(cx);
    let disk = Disk::new();
    let (library, workspace, vcx) = launch(&disk, cx);
    let folder = library.update(vcx, |library, cx| {
        library.create_folder(None, "Drafts", cx).unwrap().id
    });
    let id = add_document(
        &library,
        Some(folder),
        &simple_document("Draft", "In a folder."),
        vcx,
    );
    workspace.update_in(vcx, |workspace, window, cx| {
        assert!(workspace.open_document(id, true, window, cx));
    });
    vcx.run_until_parked();
    let panel = workspace.read_with(vcx, |w, _| w.panel().clone());
    assert!(panel.read_with(vcx, |panel, _| panel.is_expanded(folder)));
    quit(&workspace, vcx);

    disk.outside(|store| store.trash_folder(folder).unwrap());

    // The only document is in the trash: the empty state, and a sidebar
    // without the folder.
    let (_library, workspace, vcx) = launch(&disk, cx);
    assert_eq!(workspace.read_with(vcx, |w, _| w.open_document_id()), None);
    let panel = workspace.read_with(vcx, |w, _| w.panel().clone());
    panel.read_with(vcx, |panel, _| {
        assert!(panel.visible_items().is_empty());
        assert_eq!(panel.selected(), None);
    });
    // The app works: a new document is created at the root.
    let created = workspace
        .update_in(vcx, |workspace, window, cx| {
            workspace.new_document(window, cx)
        })
        .expect("a new document");
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.open_document_id()),
        Some(created)
    );
}

#[gpui::test]
fn deleted_in_between_nothing_of_it_is_left(cx: &mut TestAppContext) {
    init(cx);
    let disk = Disk::new();
    let (library, workspace, vcx) = launch(&disk, cx);
    let kept = add_document(&library, None, &simple_document("Kept", "Still here."), vcx);
    let folder = library.update(vcx, |library, cx| {
        library.create_folder(None, "Old drafts", cx).unwrap().id
    });
    let deleted = add_document(
        &library,
        Some(folder),
        &simple_document("Old draft", "To be deleted."),
        vcx,
    );
    workspace.update_in(vcx, |workspace, window, cx| {
        assert!(workspace.open_document(deleted, true, window, cx));
    });
    // The writer named it in the sidebar: its title is a chosen one.
    library.update(vcx, |library, cx| {
        library.rename_document(deleted, "My own name", cx).unwrap();
    });
    vcx.run_until_parked();
    let key = keys::custom_title(deleted);
    assert!(
        library
            .read_with(vcx, |library, _| library
                .setting::<TitleFlag>(&key)
                .unwrap())
            .is_some()
    );
    quit(&workspace, vcx);

    // An agent deletes the folder and the document in it for good, and
    // then creates a document: the library gives it the id that is free.
    let reused = disk.outside(|store| {
        store.trash_folder(folder).unwrap();
        store.purge_folder(folder).unwrap();
        assert!(store.document_meta(deleted).is_err());
        let form = StoredForm::of(&simple_document("Brand new", "Written by an agent."), None);
        store.create_document(None, &form.content()).unwrap()
    });
    assert_eq!(reused.id, deleted, "the id is handed out again");

    let (library, workspace, vcx) = launch(&disk, cx);
    // A document that exists is open, and the sidebar lists what exists.
    let open = workspace.read_with(vcx, |w, _| w.open_document_id());
    assert!(open == Some(reused.id) || open == Some(kept));
    let panel = workspace.read_with(vcx, |w, _| w.panel().clone());
    panel.read_with(vcx, |panel, _| {
        let mut items = panel.visible_items();
        items.sort_by_key(|item| match item {
            LibraryItem::Document(id) => id.0,
            LibraryItem::Folder(id) => id.0 + 1_000_000,
        });
        let mut expected = vec![
            LibraryItem::Document(kept),
            LibraryItem::Document(reused.id),
        ];
        expected.sort_by_key(|item| match item {
            LibraryItem::Document(id) => id.0,
            LibraryItem::Folder(id) => id.0,
        });
        assert_eq!(items, expected);
        assert!(!panel.is_expanded(folder));
    });
    // The new document is not taken for the old one: its title is not "a
    // chosen one", it follows its text.
    workspace.update_in(vcx, |workspace, window, cx| {
        assert!(workspace.open_document(reused.id, true, window, cx));
    });
    assert_eq!(title(&workspace, vcx).as_deref(), Some("Brand new"));
    assert!(toasts(&workspace, vcx).is_empty());
    assert_eq!(
        library.read_with(vcx, |library, _| library
            .setting::<TitleFlag>(&key)
            .unwrap()),
        None,
        "the flag of the deleted document is gone"
    );
    type_text(&workspace, "Not so ", vcx);
    let_it_save(&workspace, vcx);
    let meta = library.read_with(vcx, |library, _| library.document_meta(reused.id).unwrap());
    assert_eq!(meta.title, "Not so Brand new");
}

#[gpui::test]
fn deleted_in_between_and_nothing_else_there(cx: &mut TestAppContext) {
    init(cx);
    let disk = Disk::new();
    let id = first_launch(
        &disk,
        &simple_document("Only one", "Text."),
        None,
        Position::new(1, 1),
        cx,
    );
    disk.outside(|store| {
        store.trash_document(id).unwrap();
        store.purge_document(id).unwrap();
    });
    let (library, workspace, vcx) = launch(&disk, cx);
    assert_eq!(workspace.read_with(vcx, |w, _| w.open_document_id()), None);
    assert_eq!(title(&workspace, vcx), None);
    let panel = workspace.read_with(vcx, |w, _| w.panel().clone());
    panel.read_with(vcx, |panel, _| {
        assert!(panel.visible_items().is_empty());
        assert_eq!(panel.selected(), None);
    });
    // No welcome document appears in place of it.
    assert!(library.read_with(vcx, |library, _| {
        library.all_documents().unwrap().is_empty()
    }));
    assert!(toasts(&workspace, vcx).is_empty());
}

#[gpui::test]
fn a_chosen_title_survives_a_rewrite_in_between(cx: &mut TestAppContext) {
    init(cx);
    let disk = Disk::new();
    let (library, workspace, vcx) = launch(&disk, cx);
    let id = add_document(&library, None, &simple_document("Heading", "Body."), vcx);
    workspace.update_in(vcx, |workspace, window, cx| {
        assert!(workspace.open_document(id, true, window, cx));
    });
    library.update(vcx, |library, cx| {
        library.rename_document(id, "My title", cx).unwrap();
    });
    vcx.run_until_parked();
    quit(&workspace, vcx);

    // The flag names the document it was set for.
    let created_at = disk.outside(|store| store.document_meta(id).unwrap().created_at);
    assert_eq!(
        disk.outside(|store| store.setting::<TitleFlag>(&keys::custom_title(id)).unwrap()),
        Some(TitleFlag::For(created_at))
    );

    // An agent rewrites the text and keeps the title, as the tool does.
    let rewritten = simple_document("Another heading", "Another body.");
    disk.outside(|store| {
        store
            .save_document(id, &StoredForm::of(&rewritten, Some("My title")).content())
            .unwrap();
    });
    let (library, workspace, vcx) = launch(&disk, cx);
    assert_eq!(title(&workspace, vcx).as_deref(), Some("My title"));
    assert_eq!(toasts(&workspace, vcx), vec![CHANGED_OUTSIDE.to_string()]);
    type_text(&workspace, "x", vcx);
    let_it_save(&workspace, vcx);
    let meta = library.read_with(vcx, |library, _| library.document_meta(id).unwrap());
    assert_eq!(meta.title, "My title");
}

#[gpui::test]
fn a_flag_as_earlier_versions_stored_it_still_holds(cx: &mut TestAppContext) {
    init(cx);
    let disk = Disk::new();
    let id = first_launch(
        &disk,
        &simple_document("Heading", "Body."),
        None,
        Position::new(1, 0),
        cx,
    );
    // 0.2.0 stored `true`; so may a tool that knows the key.
    disk.outside(|store| {
        store.rename_document(id, "Named in 0.2.0").unwrap();
        store.set_setting(&keys::custom_title(id), &true).unwrap();
        // A flag whose document is gone, too.
        store
            .set_setting(&keys::custom_title(DocumentId(987)), &true)
            .unwrap();
    });
    let (library, workspace, vcx) = launch(&disk, cx);
    assert_eq!(title(&workspace, vcx).as_deref(), Some("Named in 0.2.0"));
    type_text(&workspace, "x", vcx);
    let_it_save(&workspace, vcx);
    let meta = library.read_with(vcx, |library, _| library.document_meta(id).unwrap());
    assert_eq!(meta.title, "Named in 0.2.0");
    // Rewritten in the form that names its document.
    assert_eq!(
        library.read_with(vcx, |library, _| library
            .setting::<TitleFlag>(&keys::custom_title(id))
            .unwrap()),
        Some(TitleFlag::For(meta.created_at))
    );
    assert_eq!(
        library.read_with(vcx, |library, _| library
            .setting::<TitleFlag>(&keys::custom_title(DocumentId(987)))
            .unwrap()),
        None
    );
}

// ----- the lock of the data directory, shared with the command line tool -----

/// Holds the lock of `data` as the command line tool does, on another
/// thread, for `hold`. Returns once the lock is held.
fn tool_holds_the_lock(data: &std::path::Path, hold: Duration) -> std::thread::JoinHandle<()> {
    use innerword_app::single_instance::{Acquired, HolderKind};
    use innerword_library::store::instance_lock;
    let (held, is_held) = std::sync::mpsc::channel();
    let data = data.to_path_buf();
    let thread = std::thread::spawn(move || {
        let lock = match instance_lock::acquire_as(&data, HolderKind::Tool).unwrap() {
            Acquired::Lock(lock) => lock,
            Acquired::Taken(other) => panic!("the directory was taken: {other:?}"),
        };
        held.send(()).unwrap();
        std::thread::sleep(hold);
        lock.release();
    });
    is_held.recv().unwrap();
    thread
}

#[test]
fn a_launch_waits_for_a_tool_that_is_writing() {
    use innerword_app::single_instance::{self, Acquired};
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("Inner Word");

    // The tool has the lock for the moment of a write. The app starts in
    // that moment: it waits and then has the library to itself.
    let hold = Duration::from_millis(400);
    let tool = tool_holds_the_lock(&data, hold);
    let started = std::time::Instant::now();
    let acquired = single_instance::acquire(&data).unwrap();
    let waited = started.elapsed();
    let lock = match acquired {
        Acquired::Lock(lock) => lock,
        Acquired::Taken(other) => panic!("the app gave up on a tool: {other:?}"),
    };
    assert!(
        waited >= hold - Duration::from_millis(100),
        "the lock was only free after the write ({waited:?})"
    );
    assert!(waited < single_instance::TOOL_PATIENCE);
    tool.join().unwrap();
    // The app's line is in the file now, as every version wrote it.
    assert_eq!(
        std::fs::read_to_string(lock.path()).unwrap(),
        format!("{}\n", std::process::id())
    );

    // With the app holding it, a tool is refused at once and learns that
    // it is the app.
    let refused = innerword_library::store::instance_lock::acquire_waiting(
        &data,
        single_instance::HolderKind::Tool,
        Duration::from_secs(5),
    )
    .unwrap();
    let started = std::time::Instant::now();
    match refused {
        Acquired::Taken(copy) => {
            assert_eq!(copy.kind, single_instance::HolderKind::App);
            assert_eq!(copy.pid, Some(std::process::id()));
        }
        Acquired::Lock(_) => panic!("a tool got the lock of a running app"),
    }
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(innerword_library::store::instance_lock::app_is_running(&data).unwrap());
}

#[test]
fn a_second_copy_is_not_waited_for() {
    use innerword_app::single_instance::{self, Acquired, HolderKind};
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("Inner Word");
    let first = single_instance::acquire(&data).unwrap();
    assert!(matches!(first, Acquired::Lock(_)));

    let started = std::time::Instant::now();
    let second = single_instance::acquire(&data).unwrap();
    let waited = started.elapsed();
    match second {
        Acquired::Taken(running) => {
            assert_eq!(running.kind, HolderKind::App);
            let (title, _, _) = innerword_app::window::already_running_text(&running);
            assert_eq!(title, "Inner Word is already open");
        }
        Acquired::Lock(_) => panic!("the directory was locked twice"),
    }
    assert!(
        waited < Duration::from_secs(1),
        "a running copy keeps the lock: no point in waiting ({waited:?})"
    );
}

#[gpui::test]
fn a_tool_that_does_not_let_go_is_named(cx: &mut TestAppContext) {
    use innerword_app::single_instance::{self, Acquired, HolderKind};
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("Inner Word");
    let tool = tool_holds_the_lock(&data, Duration::from_millis(900));
    let acquired =
        single_instance::acquire_with_patience(&data, Duration::from_millis(150)).unwrap();
    let running = match acquired {
        Acquired::Taken(running) => running,
        Acquired::Lock(_) => panic!("the tool had the lock"),
    };
    assert_eq!(running.kind, HolderKind::Tool);
    assert_eq!(running.pid, Some(std::process::id()));
    let (title, message, detail) = innerword_app::window::already_running_text(&running);
    assert_eq!(title, "The library is being changed right now");
    assert!(message.contains("agent"), "{message}");
    assert!(message.contains("changed nothing"), "{message}");
    assert!(detail.contains(&data.display().to_string()));
    // The window has no "Show the Open Copy": there is no copy to show.
    let handle = cx
        .update(|cx| innerword_app::window::open_already_running_window(&running, false, cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        handle
            .update(cx, |view, _, _| view.way_out_label())
            .unwrap(),
        None
    );
    tool.join().unwrap();
    // Afterwards the library is free again.
    assert!(matches!(
        single_instance::acquire(&data).unwrap(),
        Acquired::Lock(_)
    ));
}

// ----- images ------------------------------------------------------------------------------

/// An agent adds a picture with the tool while the app is closed: the next
/// launch shows it.
#[gpui::test]
fn an_image_the_tool_added_while_the_app_was_closed_is_shown(cx: &mut TestAppContext) {
    use super::images::{file_in, id_of, images_in, let_pictures_arrive, png, status};
    init(cx);
    let disk = Disk::new();
    let id = first_launch(
        &disk,
        &simple_document("Harbour", "The boats came by road."),
        None,
        Position::new(1, 4),
        cx,
    );

    // The tool, as an agent runs it: the file is read, made into what the
    // library stores, and the document gets a block that names it.
    let files = tempfile::tempdir().unwrap();
    let bytes = png(640, 400, [50, 80, 190]);
    let path = file_in(files.path(), "chart.png", &bytes);
    let added = innerword_agent::Library::at(disk.dir.path())
        .add_image(&innerword_agent::AddImageRequest {
            document: innerword_agent::Target::Id(id.0),
            path: path.to_string_lossy().into_owned(),
            description: "Salt by month".into(),
            write: innerword_agent::WriteOptions::by("Claude"),
            ..Default::default()
        })
        .expect("the tool writes while no app has the library");
    assert_eq!(added.images_added, 1);

    let (library, workspace, vcx) = launch(&disk, cx);
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.open_document_id()),
        Some(id)
    );
    let found = images_in(&workspace, vcx);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].1.id, id_of(&bytes));
    assert_eq!(found[0].1.description, "Salt by month");
    let_pictures_arrive(vcx);
    assert_eq!(
        status(&workspace, found[0].1.id, vcx),
        innerword_editor::ImageStatus::Ready
    );
    // The text was changed outside: the document opens at its start and
    // says so, as for any change of an agent.
    assert_eq!(
        selection(&workspace, vcx),
        Selection::caret(Position::new(0, 0))
    );
    assert!(toasts(&workspace, vcx).contains(&CHANGED_OUTSIDE.to_string()));
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.save_state()),
        Some(SaveState::Saved)
    );
    // The version from before the agent's change is kept.
    let labels: Vec<String> = library.read_with(vcx, |library, _| {
        library
            .store()
            .snapshot_labels(id)
            .unwrap()
            .into_values()
            .collect()
    });
    assert!(
        labels.iter().any(|label| label.contains("Claude")),
        "{labels:?}"
    );
}

/// A document can name an image the library does not have (the file of
/// the pictures was lost, a library was put together from parts). It
/// opens with a placeholder in the picture's place, and what is saved
/// still names the image: the block is never dropped.
#[gpui::test]
fn a_document_whose_picture_is_missing_opens_with_a_placeholder_and_saves_unchanged(
    cx: &mut TestAppContext,
) {
    use super::images::{images_in, let_pictures_arrive, picture, status, stored_png};
    init(cx);
    let disk = Disk::new();
    let (id, image) = {
        let (library, workspace, vcx) = launch(&disk, cx);
        let image = stored_png(&library, 400, 300, [200, 60, 40], vcx);
        let document = Document::from_blocks(vec![
            Block::paragraph("Above."),
            picture(
                &image,
                50,
                innerword_core::Alignment::Right,
                "The harbour at dusk",
            ),
            Block::paragraph("Below."),
        ]);
        let id = add_document(&library, None, &document, vcx);
        workspace.update_in(vcx, |workspace, window, cx| {
            assert!(workspace.open_document(id, true, window, cx));
        });
        let_pictures_arrive(vcx);
        assert_eq!(
            status(&workspace, image.id, vcx),
            innerword_editor::ImageStatus::Ready
        );
        quit(&workspace, vcx);
        (id, image)
    };
    cx.run_until_parked();

    // The pictures are gone: `images.db` and what belongs to it.
    let aside = disk.dir.path().join("moved away");
    std::fs::create_dir_all(&aside).unwrap();
    let mut moved = 0;
    for name in ["images.db", "images.db-wal", "images.db-shm"] {
        let file = disk.dir.path().join(name);
        if file.exists() {
            std::fs::rename(&file, aside.join(name)).unwrap();
            moved += 1;
        }
    }
    assert!(moved >= 1, "there was a file of pictures");
    let stored_before = disk.outside(|store| store.document(id).unwrap().content);

    let (library, workspace, vcx) = launch(&disk, cx);
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.open_document_id()),
        Some(id)
    );
    // The block is there, with everything the document knows about it.
    let found = images_in(&workspace, vcx);
    assert_eq!(found.len(), 1);
    assert_eq!(
        (found[0].0, found[0].1.id, found[0].1.width.percent()),
        (1, image.id, 50)
    );
    assert_eq!(found[0].1.description, "The harbour at dusk");
    let_pictures_arrive(vcx);
    assert_eq!(
        status(&workspace, image.id, vcx),
        innerword_editor::ImageStatus::Missing
    );
    // Its box keeps its place: the picture's size is in the block.
    assert!(
        workspace
            .read_with(vcx, |w, cx| w.editor().read(cx).image_bounds(1))
            .is_some()
    );
    // Nothing to save, and nothing was written.
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.save_state()),
        Some(SaveState::Saved)
    );
    assert_eq!(
        library.read_with(vcx, |library, _| library.document(id).unwrap().content),
        stored_before
    );

    // The writer goes on writing: what is saved names the image as before.
    let editor = workspace.read_with(vcx, |w, _| w.editor().clone());
    editor.update(vcx, |editor, cx| {
        editor.edit(
            |state| {
                state.set_caret(Position::new(2, "Below.".len()));
                state.insert_text(" More.")
            },
            cx,
        );
    });
    let_it_save(&workspace, vcx);
    let saved = library.read_with(vcx, |library, _| library.document(id).unwrap().content);
    let reread = innerword_core::Document::from_json(&saved).unwrap();
    let kept = reread.block(1).and_then(|block| block.as_image()).cloned();
    assert_eq!(
        kept.as_ref().map(|image| (image.id, image.width.percent())),
        Some((image.id, 50))
    );
    assert_eq!(reread.plain_text(), "Above.\n\nBelow. More.");
    quit(&workspace, vcx);
    cx.run_until_parked();

    // The file of the pictures comes back (from a backup of the Mac):
    // the next launch shows the picture again.
    for name in ["images.db", "images.db-wal", "images.db-shm"] {
        let file = aside.join(name);
        if file.exists() {
            std::fs::rename(&file, disk.dir.path().join(name)).unwrap();
        }
    }
    let (_library, workspace, vcx) = launch(&disk, cx);
    let_pictures_arrive(vcx);
    assert_eq!(
        status(&workspace, image.id, vcx),
        innerword_editor::ImageStatus::Ready
    );
}
