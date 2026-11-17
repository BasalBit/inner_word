//! Saving without holding up the main thread: a library on disk is written
//! by a background task; the order of saves, the synchronous flush and
//! renames keep their guarantees.

use gpui::{AppContext as _, TestAppContext};
use innerword_app::{
    SaveState,
    settings::keys,
    workspace::{AUTOSAVE_DELAY, RETRY_DELAY},
};
use innerword_library::Library;

use super::*;

/// A library in a file (so it can be opened a second time for background
/// saves) that has already seen its first run.
fn file_library(cx: &mut TestAppContext) -> (tempfile::TempDir, Entity<Library>) {
    let dir = tempfile::tempdir().unwrap();
    let library = cx.new(|_| Library::open(dir.path().join("library.db")).unwrap());
    library.update(cx, |library, cx| {
        library
            .set_setting(keys::WELCOME_CREATED, &true, cx)
            .unwrap();
    });
    (dir, library)
}

/// Inserts `text` at the caret without letting any task run.
fn type_quietly(workspace: &Entity<Workspace>, text: &str, cx: &mut VisualTestContext) {
    let editor = workspace.read_with(cx, |workspace, _| workspace.editor().clone());
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.insert_text(text), cx);
    });
}

fn save_state(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Option<SaveState> {
    workspace.read_with(cx, |workspace, _| workspace.save_state())
}

fn saving(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> bool {
    workspace.read_with(cx, |workspace, _| workspace.is_saving_in_background())
}

#[gpui::test]
fn autosave_of_a_library_on_disk_runs_in_the_background(cx: &mut TestAppContext) {
    init(cx);
    let (_dir, library) = file_library(cx);
    let id = add_document(&library, None, &simple_document("Notes", "First."), cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.last_save_cost()),
        None,
        "opening saves nothing"
    );

    type_text(&workspace, "More ", vcx);
    assert_eq!(save_state(&workspace, vcx), Some(SaveState::Edited));
    vcx.executor().advance_clock(AUTOSAVE_DELAY * 2);
    vcx.run_until_parked();

    // In the library, complete: text, title, counts, a first version and
    // the view state.
    assert_eq!(save_state(&workspace, vcx), Some(SaveState::Saved));
    assert!(!saving(&workspace, vcx));
    assert_eq!(stored_text(&library, id, vcx), "More Notes\nFirst.");
    let meta = library.read_with(vcx, |library, _| library.document_meta(id).unwrap());
    assert_eq!(meta.title, "More Notes");
    assert_eq!(meta.word_count, 3);
    assert_eq!(
        library.read_with(vcx, |library, _| library.snapshots(id).unwrap().len()),
        1
    );
    assert!(
        library
            .read_with(vcx, |library, _| library.view_state(id).unwrap())
            .is_some()
    );
    // It was the background that wrote it.
    let cost = workspace
        .read_with(vcx, |w, _| w.last_save_cost())
        .expect("a save happened");
    assert!(cost.background.is_some(), "{cost:?}");
    assert!(cost.bytes > 0);
    // The library's own view of the document caught up: the tree it hands
    // to the sidebar and the title of the session.
    let tree = library.read_with(vcx, |library, _| library.tree());
    let listed = tree.documents.iter().find(|doc| doc.id == id).unwrap();
    assert_eq!(listed.title, "More Notes");
    assert_eq!(listed.word_count, 3);
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.document_title().map(str::to_string)),
        Some("More Notes".to_string())
    );

    // A library in memory cannot be opened twice: its saves are synchronous.
    let memory = super::library(cx);
    let other = add_document(&memory, None, &simple_document("Memo", "Short."), cx);
    let (second, vcx) = open_workspace(&memory, cx);
    type_text(&second, "A ", vcx);
    vcx.executor().advance_clock(AUTOSAVE_DELAY * 2);
    vcx.run_until_parked();
    assert_eq!(stored_text(&memory, other, vcx), "A Memo\nShort.");
    let cost = second.read_with(vcx, |w, _| w.last_save_cost()).unwrap();
    assert_eq!(cost.background, None);
}

#[gpui::test]
fn edits_made_while_a_save_runs_are_saved_next(cx: &mut TestAppContext) {
    init(cx);
    let (_dir, library) = file_library(cx);
    let id = add_document(&library, None, &simple_document("Notes", "Body."), cx);
    let (workspace, vcx) = open_workspace(&library, cx);

    // The timer fires: a save of "one " is started but has not run yet.
    type_quietly(&workspace, "one ", vcx);
    workspace.update(vcx, |workspace, cx| workspace.autosave(cx));
    assert!(saving(&workspace, vcx));
    assert_eq!(stored_text(&library, id, vcx), "Notes\nBody.");
    // A second timer while it runs starts nothing: one save at a time.
    type_quietly(&workspace, "two ", vcx);
    workspace.update(vcx, |workspace, cx| workspace.autosave(cx));
    assert!(saving(&workspace, vcx));

    // The save lands with the text it was started with. The session knows
    // that there is more: it is not "Saved".
    vcx.run_until_parked();
    assert!(!saving(&workspace, vcx));
    assert_eq!(stored_text(&library, id, vcx), "one Notes\nBody.");
    assert_eq!(save_state(&workspace, vcx), Some(SaveState::Edited));
    assert_eq!(editor_text(&workspace, vcx), "one two Notes\nBody.");

    // And the rest follows by itself, without another edit.
    vcx.executor().advance_clock(AUTOSAVE_DELAY * 2);
    vcx.run_until_parked();
    assert_eq!(stored_text(&library, id, vcx), "one two Notes\nBody.");
    assert_eq!(save_state(&workspace, vcx), Some(SaveState::Saved));
}

#[gpui::test]
fn a_flush_is_complete_and_nothing_older_follows_it(cx: &mut TestAppContext) {
    init(cx);
    let (_dir, library) = file_library(cx);
    let id = add_document(&library, None, &simple_document("Notes", "Body."), cx);
    let (workspace, vcx) = open_workspace(&library, cx);

    // A background save of the older text is on its way when the flush
    // (closing, quitting, switching away) comes.
    type_quietly(&workspace, "old ", vcx);
    workspace.update(vcx, |workspace, cx| workspace.autosave(cx));
    type_quietly(&workspace, "new ", vcx);
    assert!(workspace.update(vcx, |workspace, cx| workspace.flush(cx)));

    // When the flush returns the newest text is in the library.
    assert_eq!(stored_text(&library, id, vcx), "old new Notes\nBody.");
    assert_eq!(save_state(&workspace, vcx), Some(SaveState::Saved));
    assert!(!saving(&workspace, vcx));
    let words = library.read_with(vcx, |library, _| {
        library.document_meta(id).unwrap().word_count
    });

    // Whatever was still queued does not write its older copy afterwards.
    vcx.run_until_parked();
    vcx.executor().advance_clock(AUTOSAVE_DELAY * 4);
    vcx.run_until_parked();
    assert_eq!(stored_text(&library, id, vcx), "old new Notes\nBody.");
    let meta = library.read_with(vcx, |library, _| library.document_meta(id).unwrap());
    assert_eq!(meta.word_count, words);
    assert_eq!(meta.title, "old new Notes");
    assert_eq!(save_state(&workspace, vcx), Some(SaveState::Saved));
}

#[gpui::test]
fn switching_documents_settles_the_save_in_flight(cx: &mut TestAppContext) {
    init(cx);
    let (_dir, library) = file_library(cx);
    let a = add_document(&library, None, &simple_document("Alpha", "One."), cx);
    let b = add_document(&library, None, &simple_document("Beta", "Two."), cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    workspace.update_in(vcx, |workspace, window, cx| {
        workspace.open_document(a, true, window, cx);
    });

    type_quietly(&workspace, "first ", vcx);
    workspace.update(vcx, |workspace, cx| workspace.autosave(cx));
    type_quietly(&workspace, "second ", vcx);
    // Away to the other document while the save is still queued.
    workspace.update_in(vcx, |workspace, window, cx| {
        workspace.open_document(b, true, window, cx);
    });
    assert_eq!(stored_text(&library, a, vcx), "first second Alpha\nOne.");
    type_quietly(&workspace, "here ", vcx);
    vcx.run_until_parked();
    vcx.executor().advance_clock(AUTOSAVE_DELAY * 2);
    vcx.run_until_parked();

    // Each document has its own text; the abandoned save of the first one
    // touched neither.
    assert_eq!(stored_text(&library, a, vcx), "first second Alpha\nOne.");
    assert_eq!(stored_text(&library, b, vcx), "here Beta\nTwo.");
    assert_eq!(save_state(&workspace, vcx), Some(SaveState::Saved));
}

#[gpui::test]
fn a_rename_while_a_save_is_queued_is_kept(cx: &mut TestAppContext) {
    init(cx);
    let (_dir, library) = file_library(cx);
    let id = add_document(&library, None, &simple_document("Draft", "Body."), cx);
    let (workspace, vcx) = open_workspace(&library, cx);

    // The save was started while the title still followed the text...
    type_quietly(&workspace, "My ", vcx);
    workspace.update(vcx, |workspace, cx| workspace.autosave(cx));
    // ...and before it writes, the user names the document in the sidebar.
    library.update(vcx, |library, cx| {
        library.rename_document(id, "The Salt Road", cx).unwrap()
    });
    vcx.run_until_parked();

    // The text is saved, under the name the user chose.
    assert_eq!(stored_text(&library, id, vcx), "My Draft\nBody.");
    let meta = library.read_with(vcx, |library, _| library.document_meta(id).unwrap());
    assert_eq!(meta.title, "The Salt Road");
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.document_title().map(str::to_string)),
        Some("The Salt Road".to_string())
    );
    assert_eq!(save_state(&workspace, vcx), Some(SaveState::Saved));
    // And it stays that way through later saves.
    type_text(&workspace, "Again ", vcx);
    vcx.executor().advance_clock(AUTOSAVE_DELAY * 2);
    vcx.run_until_parked();
    let meta = library.read_with(vcx, |library, _| library.document_meta(id).unwrap());
    assert_eq!(meta.title, "The Salt Road");
}

#[gpui::test]
fn a_failed_background_save_recovers_when_the_library_does(cx: &mut TestAppContext) {
    init(cx);
    let (dir, library) = file_library(cx);
    let path = dir.path().join("library.db");
    let id = add_document(&library, None, &simple_document("Fragile", "Text."), cx);
    let (workspace, vcx) = open_workspace(&library, cx);

    // Let the window settle first (it stores its size and the sidebar its
    // selection a moment after opening), so that only the save meets the
    // lock.
    vcx.executor().advance_clock(AUTOSAVE_DELAY * 4);
    vcx.run_until_parked();

    // Another connection holds the write lock: the save cannot get in.
    // (The store waits five seconds for a lock, in real time: this test
    // takes that long.)
    let blocker = rusqlite_lock(&path);
    type_text(&workspace, "kept ", vcx);
    vcx.executor().advance_clock(AUTOSAVE_DELAY * 2);
    vcx.run_until_parked();
    let state = save_state(&workspace, vcx).unwrap();
    assert!(matches!(state, SaveState::Failed(_)), "{state:?}");
    assert!(editor_text(&workspace, vcx).contains("kept"));
    let toasts = |cx: &mut VisualTestContext| {
        workspace.read_with(cx, |w, cx| {
            w.toast_layer()
                .read(cx)
                .toasts()
                .map(|toast| toast.message.to_string())
                .collect::<Vec<_>>()
        })
    };
    assert_eq!(toasts(vcx).len(), 1, "{:?}", toasts(vcx));

    // The lock goes away; the retry succeeds and says so.
    drop(blocker);
    vcx.executor()
        .advance_clock(RETRY_DELAY + std::time::Duration::from_millis(200));
    vcx.run_until_parked();
    assert_eq!(save_state(&workspace, vcx), Some(SaveState::Saved));
    assert_eq!(stored_text(&library, id, vcx), "kept Fragile\nText.");
    assert!(
        toasts(vcx).iter().any(|message| message == "Saved"),
        "{:?}",
        toasts(vcx)
    );
}

/// Holds the write lock of the library file until dropped.
fn rusqlite_lock(path: &std::path::Path) -> impl Drop {
    struct Lock(std::process::Child);
    impl Drop for Lock {
        fn drop(&mut self) {
            use std::io::Write;
            // Ending the transaction and the shell releases the lock.
            if let Some(mut stdin) = self.0.stdin.take() {
                let _ = stdin.write_all(b"ROLLBACK;\n.quit\n");
            }
            let _ = self.0.wait();
        }
    }
    use std::io::Write;
    let mut child = std::process::Command::new("/usr/bin/sqlite3")
        .arg(path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("sqlite3 is part of macOS");
    {
        let stdin = child.stdin.as_mut().unwrap();
        stdin
            .write_all(b"BEGIN IMMEDIATE;\nSELECT 'locked';\n")
            .unwrap();
        stdin.flush().unwrap();
    }
    // Wait until the lock is held.
    use std::io::{BufRead, BufReader};
    let mut line = String::new();
    BufReader::new(child.stdout.as_mut().unwrap())
        .read_line(&mut line)
        .unwrap();
    assert_eq!(line.trim(), "locked");
    Lock(child)
}

#[gpui::test]
fn the_measuring_steps_measure_the_open_document(cx: &mut TestAppContext) {
    init(cx);
    let (_dir, library) = file_library(cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    // Nothing open: nothing to measure.
    let lines = workspace.update(vcx, |workspace, cx| workspace.measure_save_cost(2, cx));
    assert_eq!(lines, ["no document is open"]);

    let title = workspace
        .update_in(vcx, |workspace, window, cx| {
            workspace.open_generated_book(120, window, cx)
        })
        .unwrap();
    assert_eq!(title, "Chapter 1");
    assert!(editor_focused(&workspace, vcx));
    let blocks = workspace.read_with(vcx, |w, cx| w.editor().read(cx).document().block_count());
    assert_eq!(blocks, 120);

    let lines = workspace.update(vcx, |workspace, cx| workspace.measure_save_cost(3, cx));
    assert_eq!(lines.len(), 9, "{lines:?}");
    assert!(lines[0].starts_with("120 blocks, "), "{lines:?}");
    assert!(lines[1..].iter().all(|line| line.contains("median")));
    assert!(lines[8].starts_with("whole save on the main thread"));

    // The count by blocks is checked against the whole count inside.
    let text = editor_text(&workspace, vcx);
    let lines = workspace.update(vcx, |workspace, cx| workspace.measure_limit_cost(3, cx));
    assert_eq!(lines.len(), 4, "{lines:?}");
    assert!(lines[3].starts_with("block by block, after an edit"));
    // It typed one character per round to have something to recount.
    assert_eq!(editor_text(&workspace, vcx).len(), text.len() + 3);
}
