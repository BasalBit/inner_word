//! Document sessions: first run, autosave, reload, switching, titles,
//! trash, failed saves, lenient loading.

use gpui::{AppContext as _, TestAppContext};
use writer_app::{
    SaveState, Workspace,
    actions::UseTitleFromText,
    settings::keys,
    workspace::{AUTOSAVE_DELAY, MAX_AUTOSAVE_WAIT, RETRY_DELAY},
};
use writer_core::{Document, Movement, Position};
use writer_library::{DocumentContent, Library, LibraryItem};

use super::*;

#[gpui::test]
fn first_run_creates_the_welcome_document_once(cx: &mut TestAppContext) {
    init(cx);
    let library = cx.new(|_| Library::in_memory().unwrap());
    let (workspace, vcx) = open_workspace(&library, cx);
    let title = workspace.read_with(vcx, |workspace, _| {
        workspace.document_title().map(str::to_string)
    });
    assert_eq!(title.as_deref(), Some("Welcome to The Writer"));
    assert!(editor_text(&workspace, vcx).contains("command palette"));
    assert!(editor_focused(&workspace, vcx));
    let count = |cx: &mut TestAppContext| {
        library.read_with(cx, |library, _| library.all_documents().unwrap().len())
    };
    assert_eq!(count(cx), 1);

    // The user throws it away; the next launch does not bring it back.
    let id = library.read_with(cx, |library, _| library.all_documents().unwrap()[0].meta.id);
    library.update(cx, |library, cx| library.trash_document(id, cx).unwrap());
    cx.run_until_parked();
    let (second, vcx) = open_workspace(&library, cx);
    assert_eq!(second.read_with(vcx, |w, _| w.open_document_id()), None);
    assert_eq!(count(cx), 0);
}

#[gpui::test]
fn autosave_is_debounced_and_the_session_is_restored(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    let id = add_document(&library, None, &simple_document("Notes", "First."), cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.open_document_id()),
        Some(id)
    );
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.save_state()),
        Some(SaveState::Saved)
    );

    // Real typing: through the key path and the platform input handler.
    let editor = workspace.read_with(vcx, |w, _| w.editor().clone());
    editor.update(vcx, |editor, cx| {
        editor.edit(|state| state.move_caret(Movement::DocEnd, false), cx);
    });
    vcx.simulate_input(" Second.");
    assert_eq!(editor_text(&workspace, vcx), "Notes\nFirst. Second.");
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.save_state()),
        Some(SaveState::Edited)
    );
    // Not yet in the library: the timer has not fired.
    assert_eq!(stored_text(&library, id, vcx), "Notes\nFirst.");

    vcx.executor().advance_clock(AUTOSAVE_DELAY / 2);
    vcx.run_until_parked();
    assert_eq!(stored_text(&library, id, vcx), "Notes\nFirst.");
    vcx.executor().advance_clock(AUTOSAVE_DELAY);
    vcx.run_until_parked();
    assert_eq!(stored_text(&library, id, vcx), "Notes\nFirst. Second.");
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.save_state()),
        Some(SaveState::Saved)
    );
    // The derived fields went with it.
    let meta = library.read_with(vcx, |library, _| library.document_meta(id).unwrap());
    assert_eq!(meta.title, "Notes");
    assert_eq!(meta.word_count, 3);
    // And the first save took a version snapshot.
    assert_eq!(
        library.read_with(vcx, |library, _| library.snapshots(id).unwrap().len()),
        1
    );
    let caret = editor.read_with(vcx, |editor, _| editor.selection().head);
    // Moving the caret alone (no edit) is remembered at the next flush too.
    editor.update(vcx, |editor, cx| {
        editor.edit(|state| state.set_caret(Position::new(1, 3)), cx);
    });
    workspace.update(vcx, |workspace, cx| workspace.flush(cx));
    let stored_view = library.read_with(vcx, |library, _| library.view_state(id).unwrap());
    let stored_view: writer_editor::ViewState =
        serde_json::from_str(&stored_view.expect("a view state was stored")).unwrap();
    assert_eq!(stored_view.selection().head, Position::new(1, 3));
    editor.update(vcx, |editor, cx| {
        editor.edit(|state| state.set_caret(caret), cx);
    });

    // "Quit and relaunch": a second workspace over the same library opens
    // the same document with the caret where it was.
    workspace.update(vcx, |workspace, cx| workspace.flush(cx));
    let (second, vcx) = open_workspace(&library, cx);
    assert_eq!(second.read_with(vcx, |w, _| w.open_document_id()), Some(id));
    assert_eq!(editor_text(&second, vcx), "Notes\nFirst. Second.");
    let restored = second.read_with(vcx, |w, cx| w.editor().read(cx).selection().head);
    assert_eq!(restored, caret);
    assert_eq!(restored, Position::new(1, "First. Second.".len()));
}

#[gpui::test]
fn switching_documents_flushes_and_keeps_both_intact(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    let a = add_document(&library, None, &simple_document("Alpha", "One."), cx);
    let b = add_document(&library, None, &simple_document("Beta", "Two."), cx);
    let (workspace, vcx) = open_workspace(&library, cx);

    workspace.update_in(vcx, |workspace, window, cx| {
        assert!(workspace.open_document(a, true, window, cx));
    });
    type_text(&workspace, "A-edit ", vcx);
    // Switch without waiting for the autosave timer.
    workspace.update_in(vcx, |workspace, window, cx| {
        assert!(workspace.open_document(b, true, window, cx));
    });
    vcx.run_until_parked();
    assert!(stored_text(&library, a, vcx).contains("A-edit"));
    assert_eq!(editor_text(&workspace, vcx), "Beta\nTwo.");

    type_text(&workspace, "B-edit ", vcx);
    workspace.update_in(vcx, |workspace, window, cx| {
        assert!(workspace.open_document(a, true, window, cx));
    });
    vcx.run_until_parked();
    assert!(stored_text(&library, b, vcx).contains("B-edit"));
    assert!(!stored_text(&library, b, vcx).contains("A-edit"));
    assert!(editor_text(&workspace, vcx).contains("A-edit"));
    assert!(!stored_text(&library, a, vcx).contains("B-edit"));

    // The old document's pending timer must not write into the new one.
    vcx.executor().advance_clock(AUTOSAVE_DELAY * 3);
    vcx.run_until_parked();
    assert!(!stored_text(&library, a, vcx).contains("B-edit"));
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.open_document_id()),
        Some(a)
    );
    assert_eq!(
        library.read_with(vcx, |library, _| {
            library
                .setting::<writer_library::DocumentId>(keys::LAST_DOCUMENT)
                .unwrap()
        }),
        Some(a)
    );
}

#[gpui::test]
fn losing_focus_and_closing_flush(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    let id = add_document(&library, None, &simple_document("Draft", "Text."), cx);
    let (workspace, vcx) = open_workspace(&library, cx);

    type_text(&workspace, "one ", vcx);
    assert!(!stored_text(&library, id, vcx).contains("one"));
    vcx.deactivate_window();
    vcx.run_until_parked();
    assert!(stored_text(&library, id, vcx).contains("one"));

    vcx.update(|window, _| window.activate_window());
    type_text(&workspace, "two ", vcx);
    assert!(!stored_text(&library, id, vcx).contains("two"));
    vcx.simulate_keystrokes("cmd-w");
    vcx.run_until_parked();
    assert!(stored_text(&library, id, cx).contains("two"));
}

#[gpui::test]
fn title_follows_the_text_until_the_user_renames(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    let id = workspace
        .update_in(vcx, |workspace, window, cx| {
            workspace.new_document(window, cx)
        })
        .unwrap();
    let title = |cx: &mut gpui::VisualTestContext| {
        library.read_with(cx, |library, _| library.document_meta(id).unwrap().title)
    };
    assert_eq!(title(vcx), "Untitled");

    type_text(&workspace, "A working title", vcx);
    workspace.update(vcx, |workspace, cx| workspace.flush(cx));
    assert_eq!(title(vcx), "A working title");
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.document_title().map(str::to_string)),
        Some("A working title".to_string())
    );

    // The sidebar renames through the library.
    library.update(vcx, |library, cx| {
        library.rename_document(id, "Chosen name", cx).unwrap()
    });
    vcx.run_until_parked();
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.document_title().map(str::to_string)),
        Some("Chosen name".to_string())
    );
    type_text(&workspace, " that changed", vcx);
    vcx.executor().advance_clock(AUTOSAVE_DELAY * 2);
    vcx.run_until_parked();
    assert!(stored_text(&library, id, vcx).contains("that changed"));
    assert_eq!(title(vcx), "Chosen name");

    // The chosen name survives a relaunch.
    let (second, vcx) = open_workspace(&library, cx);
    type_text(&second, "!", vcx);
    second.update(vcx, |workspace, cx| workspace.flush(cx));
    assert_eq!(title(vcx), "Chosen name");

    // Back to deriving on request.
    vcx.dispatch_action(UseTitleFromText);
    vcx.run_until_parked();
    assert!(title(vcx).starts_with('!') || title(vcx).contains("A working title"));
    assert_ne!(title(vcx), "Chosen name");
}

#[gpui::test]
fn trashing_the_open_document_saves_it_and_moves_on(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    let a = add_document(&library, None, &simple_document("Keep", "Kept."), cx);
    let folder = library.update(cx, |library, cx| {
        library.create_folder(None, "Drafts", cx).unwrap().id
    });
    let b = add_document(
        &library,
        Some(folder),
        &simple_document("Gone", "Soon."),
        cx,
    );
    let (workspace, vcx) = open_workspace(&library, cx);
    workspace.update_in(vcx, |workspace, window, cx| {
        workspace.open_document(b, true, window, cx);
    });
    type_text(&workspace, "last words ", vcx);

    // Trash the whole folder with the open document inside.
    library.update(vcx, |library, cx| library.trash_folder(folder, cx).unwrap());
    vcx.run_until_parked();
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.open_document_id()),
        Some(a)
    );
    assert!(editor_focused(&workspace, vcx));
    // What was typed went into the trashed document, not into thin air.
    assert!(stored_text(&library, b, vcx).contains("last words"));
    assert!(!stored_text(&library, a, vcx).contains("last words"));

    // Trash the last one: the empty state, and the app keeps working.
    library.update(vcx, |library, cx| library.trash_document(a, cx).unwrap());
    vcx.run_until_parked();
    assert_eq!(workspace.read_with(vcx, |w, _| w.open_document_id()), None);
    assert_eq!(workspace.read_with(vcx, |w, _| w.save_state()), None);
    vcx.simulate_keystrokes("cmd-n");
    vcx.run_until_parked();
    assert!(
        workspace
            .read_with(vcx, |w, _| w.open_document_id())
            .is_some()
    );
    assert!(editor_focused(&workspace, vcx));
}

#[gpui::test]
fn a_failed_save_is_reported_kept_and_retried(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.db");
    let library = cx.new(|_| Library::open(&path).unwrap());
    library.update(cx, |library, cx| {
        library
            .set_setting(keys::WELCOME_CREATED, &true, cx)
            .unwrap();
    });
    let id = add_document(&library, None, &simple_document("Fragile", "Text."), cx);
    let (workspace, vcx) = open_workspace(&library, cx);

    // Behind the app's back the row disappears, so the next save fails.
    {
        let mut other = writer_library::store::Store::open(&path).unwrap();
        other.trash_document(id).unwrap();
        other.purge_document(id).unwrap();
    }
    type_text(&workspace, "precious ", vcx);
    vcx.executor().advance_clock(AUTOSAVE_DELAY * 2);
    vcx.run_until_parked();

    let state = workspace.read_with(vcx, |w, _| w.save_state()).unwrap();
    assert!(matches!(state, SaveState::Failed(_)), "{state:?}");
    let toasts = |cx: &mut gpui::VisualTestContext| {
        workspace.read_with(cx, |w, cx| w.toast_layer().read(cx).toasts().count())
    };
    assert_eq!(toasts(vcx), 1);
    // Nothing was dropped: the text is still in the editor, still dirty.
    assert!(editor_text(&workspace, vcx).contains("precious"));
    let status = workspace.update(vcx, |w, cx| w.status_line(cx));
    assert!(status.save_failed);

    // It keeps retrying without stacking up toasts.
    vcx.executor().advance_clock(RETRY_DELAY * 3);
    vcx.run_until_parked();
    assert!(matches!(
        workspace.read_with(vcx, |w, _| w.save_state()).unwrap(),
        SaveState::Failed(_)
    ));
    assert!(toasts(vcx) <= 1);
    assert!(editor_text(&workspace, vcx).contains("precious"));
}

#[gpui::test]
fn unreadable_content_opens_as_plain_text_and_keeps_the_original(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    let id = library.update(cx, |library, cx| {
        library
            .create_document(
                None,
                &DocumentContent {
                    content: "this was never JSON\nsecond line",
                    title: "Odd one",
                    plain_text: "",
                    word_count: 0,
                    char_count: 0,
                },
                cx,
            )
            .unwrap()
            .id
    });
    let (workspace, vcx) = open_workspace(&library, cx);
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.open_document_id()),
        Some(id)
    );
    assert_eq!(
        editor_text(&workspace, vcx),
        "this was never JSON\nsecond line"
    );
    // The original is in the version history before anything is saved.
    let snapshots = library.read_with(vcx, |library, _| library.snapshots(id).unwrap());
    assert_eq!(snapshots.len(), 1);
    let original = library.read_with(vcx, |library, _| {
        library.snapshot(snapshots[0].id).unwrap().content
    });
    assert_eq!(original, "this was never JSON\nsecond line");
    // Opening alone rewrites nothing.
    workspace.update(vcx, |workspace, cx| workspace.flush(cx));
    let stored = library.read_with(vcx, |library, _| library.document(id).unwrap().content);
    assert_eq!(stored, "this was never JSON\nsecond line");
    // After an edit it becomes a real document.
    type_text(&workspace, "x", vcx);
    workspace.update(vcx, |workspace, cx| workspace.flush(cx));
    let stored = library.read_with(vcx, |library, _| library.document(id).unwrap().content);
    assert!(Document::from_json(&stored).is_ok());
}

#[gpui::test]
fn new_document_and_new_folder(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    let folder = library.update(cx, |library, cx| {
        library.create_folder(None, "Essays", cx).unwrap().id
    });
    add_document(&library, None, &simple_document("Loose", "Page."), cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    let panel = workspace.read_with(vcx, |w, _| w.panel().clone());

    // cmd-n creates in the folder selected in the sidebar, opens the
    // document and puts focus in the editor.
    panel.update(vcx, |panel, cx| {
        panel.select(Some(LibraryItem::Folder(folder)), cx)
    });
    vcx.simulate_keystrokes("cmd-0");
    assert!(!editor_focused(&workspace, vcx));
    vcx.simulate_keystrokes("cmd-n");
    vcx.run_until_parked();
    let id = workspace
        .read_with(vcx, |w, _| w.open_document_id())
        .unwrap();
    let meta = library.read_with(vcx, |library, _| library.document_meta(id).unwrap());
    assert_eq!(meta.folder_id, Some(folder));
    assert_eq!(meta.title, "Untitled");
    assert!(editor_focused(&workspace, vcx));
    assert_eq!(
        panel.read_with(vcx, |panel, _| panel.active_document()),
        Some(id)
    );
    vcx.simulate_input("Fresh page");
    assert_eq!(editor_text(&workspace, vcx), "Fresh page");

    // cmd-shift-n creates a folder beside it and starts naming it.
    let before = library.read_with(vcx, |library, _| library.folders().unwrap().len());
    vcx.simulate_keystrokes("cmd-shift-n");
    vcx.run_until_parked();
    let folders = library.read_with(vcx, |library, _| library.folders().unwrap());
    assert_eq!(folders.len(), before + 1);
    let created = folders.iter().find(|f| f.id != folder).unwrap();
    assert_eq!(created.parent_id, Some(folder));
    assert_eq!(
        panel.read_with(vcx, |panel, _| panel.renaming()),
        Some(LibraryItem::Folder(created.id))
    );
    assert!(!editor_focused(&workspace, vcx));
    // The typed text of the new document was not lost on the way.
    assert!(
        stored_text(&library, id, vcx).contains("Fresh page") || {
            workspace.update(vcx, |workspace, cx| workspace.flush(cx));
            stored_text(&library, id, vcx).contains("Fresh page")
        }
    );
    let _: &gpui::Entity<Workspace> = &workspace;
}

#[gpui::test]
fn steady_typing_is_saved_without_a_pause(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    let id = add_document(&library, None, &simple_document("Flow", ""), cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    // A key every half second: never a pause as long as the debounce.
    let step = AUTOSAVE_DELAY / 2;
    let mut typed = 0;
    while step * typed <= MAX_AUTOSAVE_WAIT + AUTOSAVE_DELAY {
        type_text(&workspace, "x", vcx);
        vcx.executor().advance_clock(step);
        vcx.run_until_parked();
        typed += 1;
    }
    let stored = stored_text(&library, id, vcx);
    let saved = stored.matches('x').count();
    assert!(saved > 0, "nothing was saved during {typed} keystrokes");
    assert!(saved <= typed as usize);
    // And the pause at the end saves the rest.
    vcx.executor().advance_clock(AUTOSAVE_DELAY * 2);
    vcx.run_until_parked();
    assert_eq!(
        stored_text(&library, id, vcx).matches('x').count(),
        typed as usize
    );
}
