//! Agents that write while the app is open: the assembled workspace with
//! the real listener of the channel (`writer_app::live`), and the real
//! client of `writer-agent` asking from a second thread, as the `writer`
//! tool asks from its own process. Every test has a temporary data
//! directory, whose lock the test holds as the app does.
//!
//! The rule the tests hold: nothing the writer typed is lost, nothing the
//! agent wrote is lost, and the writer sees and can take back what an
//! agent did.

use std::{
    sync::mpsc,
    time::{Duration, Instant},
};

use gpui::{Entity, TestAppContext, VisualTestContext};
use writer_agent::{
    self as agent, EditRequest, InsertRequest, ListVersionsRequest, ReplaceDocumentRequest,
    WriteOptions,
};
use writer_app::{
    SaveState, live,
    settings::keys,
    single_instance::{self, Acquired, InstanceLock},
    workspace::{AUTOSAVE_DELAY, AgentUndo, Offer, Subject},
};
use writer_core::{Position, Selection};
use writer_library::Library;

use super::*;

/// The app open on a library in a temporary data directory, with the
/// channel for agents open.
struct Desk {
    dir: tempfile::TempDir,
    library: Entity<Library>,
    workspace: Entity<Workspace>,
    _lock: InstanceLock,
}

impl Desk {
    /// The library as a tool sees it: the real client, which finds the
    /// app through the lock file of the data directory.
    fn tool(&self) -> agent::Library {
        agent::Library::at(self.dir.path()).with_app_patience(Duration::from_secs(20))
    }
}

fn claude() -> WriteOptions {
    WriteOptions::by("Claude")
}

/// A library on disk in a data directory this test holds the lock of.
fn library_on_disk(cx: &mut TestAppContext) -> (tempfile::TempDir, InstanceLock, Entity<Library>) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    let lock = match single_instance::acquire(dir.path()).unwrap() {
        Acquired::Lock(lock) => lock,
        Acquired::Taken(copy) => panic!("the lock of a new directory is taken: {copy:?}"),
    };
    let library = cx.new(|_| {
        Library::open(dir.path().join(writer_library::store::DATABASE_FILE_NAME)).unwrap()
    });
    library.update(cx, |library, cx| {
        library
            .set_setting(keys::WELCOME_CREATED, &true, cx)
            .unwrap();
    });
    (dir, lock, library)
}

/// Opens the workspace over `library` and the channel for agents.
fn open_desk(
    dir: tempfile::TempDir,
    lock: InstanceLock,
    library: Entity<Library>,
    cx: &mut TestAppContext,
) -> (Desk, &mut VisualTestContext) {
    let (workspace, vcx) = open_workspace(&library, cx);
    let window = vcx
        .update(|window, _| window.window_handle())
        .downcast::<Workspace>()
        .unwrap();
    vcx.update(|_, cx| live::open(dir.path(), window, cx))
        .expect("the channel opens");
    (
        Desk {
            dir,
            library,
            workspace,
            _lock: lock,
        },
        vcx,
    )
}

/// The app open on one document: "Notes" with two paragraphs.
fn desk_with_notes(cx: &mut TestAppContext) -> (Desk, DocumentId, &mut VisualTestContext) {
    let (dir, lock, library) = library_on_disk(cx);
    let id = add_document(&library, None, &notes(), cx);
    let (desk, vcx) = open_desk(dir, lock, library, cx);
    assert_eq!(
        desk.workspace
            .read_with(vcx, |workspace, _| workspace.open_document_id()),
        Some(id)
    );
    (desk, id, vcx)
}

fn notes() -> Document {
    Document::from_blocks(vec![
        Block::with_text(BlockKind::heading(1), "Notes"),
        Block::paragraph("The first paragraph is short."),
        Block::paragraph("The second paragraph is the last."),
    ])
}

/// Runs `ask` the way a tool asks: on another thread, with the real
/// client, while the main thread of the app goes on. Returns its answer.
fn ask<T: Send + 'static>(
    cx: &mut VisualTestContext,
    ask: impl FnOnce() -> T + Send + 'static,
) -> T {
    let asked = asking(ask);
    answer(cx, asked)
}

/// A question that is on its way.
struct Asked<T>(mpsc::Receiver<T>, std::thread::JoinHandle<()>);

/// Starts `ask` on another thread and returns at once.
fn asking<T: Send + 'static>(ask: impl FnOnce() -> T + Send + 'static) -> Asked<T> {
    let (sender, receiver) = mpsc::channel();
    let thread = std::thread::spawn(move || {
        sender.send(ask()).ok();
    });
    Asked(receiver, thread)
}

/// Lets the app work until `asked` has its answer.
fn answer<T>(cx: &mut VisualTestContext, asked: Asked<T>) -> T {
    let started = Instant::now();
    loop {
        cx.run_until_parked();
        if let Ok(answer) = asked.0.try_recv() {
            asked.1.join().unwrap();
            cx.run_until_parked();
            return answer;
        }
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "the agent got no answer"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// The notes at the bottom: the message and the label of the button.
fn notes_shown(desk: &Desk, cx: &mut VisualTestContext) -> Vec<(String, Option<String>)> {
    desk.workspace.read_with(cx, |workspace, cx| {
        workspace
            .toast_layer()
            .read(cx)
            .toasts()
            .map(|toast| {
                (
                    toast.message.to_string(),
                    toast.action_label().map(|label| label.to_string()),
                )
            })
            .collect()
    })
}

/// Presses the button of the note that says `message`.
fn press_note(desk: &Desk, message: &str, cx: &mut VisualTestContext) {
    let layer = desk
        .workspace
        .read_with(cx, |workspace, _| workspace.toast_layer().clone());
    let id = layer.read_with(cx, |layer, _| {
        layer
            .ids()
            .into_iter()
            .zip(layer.toasts())
            .find(|(_, toast)| toast.message.as_ref() == message)
            .map(|(id, _)| id)
            .unwrap_or_else(|| panic!("no note says {message:?}"))
    });
    let ran = layer.update_in(cx, |layer, window, cx| layer.run_action(id, window, cx));
    assert!(ran, "the note {message:?} has no button");
    cx.run_until_parked();
}

fn selection(desk: &Desk, cx: &mut VisualTestContext) -> Selection {
    desk.workspace
        .read_with(cx, |workspace, cx| workspace.editor().read(cx).selection())
}

fn set_caret(desk: &Desk, block: usize, offset: usize, cx: &mut VisualTestContext) {
    let editor = desk
        .workspace
        .read_with(cx, |workspace, _| workspace.editor().clone());
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.set_caret(Position::new(block, offset)), cx);
    });
    cx.run_until_parked();
}

fn save_state(desk: &Desk, cx: &mut VisualTestContext) -> Option<SaveState> {
    desk.workspace
        .read_with(cx, |workspace, _| workspace.save_state())
}

fn stored(desk: &Desk, id: DocumentId, cx: &mut VisualTestContext) -> String {
    let content = desk
        .library
        .read_with(cx, |library, _| library.document(id).unwrap().content);
    Document::from_json(&content).unwrap().plain_text()
}

fn version_labels(desk: &Desk, id: DocumentId, cx: &mut VisualTestContext) -> Vec<String> {
    desk.library.read_with(cx, |library, _| {
        library
            .store()
            .snapshot_labels(id)
            .unwrap()
            .into_values()
            .collect()
    })
}

fn edit(document: DocumentId, old: &str, new: &str) -> EditRequest {
    EditRequest {
        document: document.into(),
        old_text: old.into(),
        new_text: new.into(),
        write: claude(),
        ..EditRequest::default()
    }
}

// ----- the text of the open document ---------------------------------------------------

#[gpui::test]
fn an_agent_edits_the_open_document_while_it_has_unsaved_typing(cx: &mut TestAppContext) {
    let (desk, id, cx) = desk_with_notes(cx);
    // The writer is at the end of the last paragraph and types.
    set_caret(&desk, 2, "The second paragraph is the last.".len(), cx);
    type_text(&desk.workspace, " And more.", cx);
    assert_eq!(save_state(&desk, cx), Some(SaveState::Edited));
    assert!(!stored(&desk, id, cx).contains("And more."));
    let caret = selection(&desk, cx);

    let tool = desk.tool();
    let changed = ask(cx, move || {
        tool.edit(&edit(id, "is short", "was short once"))
    })
    .expect("the app made the change");
    assert!(changed.written.changed);

    // Both are in the text, and both are in the library.
    let expected = "Notes\nThe first paragraph was short once.\n\
                    The second paragraph is the last. And more.";
    assert_eq!(editor_text(&desk.workspace, cx), expected);
    assert_eq!(stored(&desk, id, cx), expected);
    // Nothing is left to save, and nothing is written a second time.
    assert_eq!(save_state(&desk, cx), Some(SaveState::Saved));
    cx.executor().advance_clock(AUTOSAVE_DELAY * 3);
    cx.run_until_parked();
    assert_eq!(save_state(&desk, cx), Some(SaveState::Saved));
    // The caret is where the writer left it, and the keyboard is theirs.
    assert_eq!(selection(&desk, cx), caret);
    assert!(editor_focused(&desk.workspace, cx));
    // The version from before the agent's edit holds the typing (the app
    // kept it when it saved for the agent, a moment before the edit).
    let versions = desk
        .library
        .read_with(cx, |library, _| library.snapshots(id).unwrap());
    assert_eq!(versions.len(), 1);
    let kept = desk.library.read_with(cx, |library, _| {
        Document::from_json(&library.snapshot(versions[0].id).unwrap().content)
            .unwrap()
            .plain_text()
    });
    assert_eq!(
        kept,
        "Notes\nThe first paragraph is short.\nThe second paragraph is the last. And more."
    );

    // The undo key takes the agent's edit out, then the writer's typing.
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(
        editor_text(&desk.workspace, cx),
        "Notes\nThe first paragraph is short.\nThe second paragraph is the last. And more."
    );
    assert_eq!(save_state(&desk, cx), Some(SaveState::Edited));
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(editor_text(&desk.workspace, cx), notes().plain_text());
    // And what the writer undid is what the library gets.
    cx.executor().advance_clock(AUTOSAVE_DELAY * 3);
    cx.run_until_parked();
    assert_eq!(stored(&desk, id, cx), notes().plain_text());
}

#[gpui::test]
fn the_note_names_the_agent_and_its_undo_is_the_undo_key(cx: &mut TestAppContext) {
    let (desk, id, cx) = desk_with_notes(cx);
    let tool = desk.tool();
    ask(cx, move || tool.edit(&edit(id, "is short", "is long"))).unwrap();

    assert_eq!(
        notes_shown(&desk, cx),
        [(
            "Claude changed \u{201c}Notes\u{201d}".to_string(),
            Some("Undo".to_string())
        )]
    );
    // The version kept before the edit says whose edit it was, as it
    // does when the tool writes while the app is closed.
    assert_eq!(
        version_labels(&desk, id, cx),
        ["Before an edit by Claude".to_string()]
    );
    let last = desk
        .workspace
        .read_with(cx, |workspace, _| workspace.last_agent_changes().cloned())
        .unwrap();
    assert_eq!(last.who, "Claude");
    assert_eq!(last.count, 1);
    assert_eq!(last.subject, Subject::Document(id));
    assert!(matches!(
        last.undo,
        Some(AgentUndo::Text {
            document,
            steps: 1,
            ..
        }) if document == id
    ));

    press_note(&desk, "Claude changed \u{201c}Notes\u{201d}", cx);
    assert_eq!(editor_text(&desk.workspace, cx), notes().plain_text());
    assert_eq!(
        notes_shown(&desk, cx),
        [("Undid Claude\u{2019}s change".to_string(), None)]
    );
    // It was a step of the undo history: redo brings it back.
    cx.simulate_keystrokes("cmd-shift-z");
    assert!(editor_text(&desk.workspace, cx).contains("is long"));

    // An agent that gives no name is "An agent".
    let tool = desk.tool();
    ask(cx, move || {
        tool.edit(&EditRequest {
            write: WriteOptions::default(),
            ..edit(id, "the last", "not the last")
        })
    })
    .unwrap();
    assert!(
        notes_shown(&desk, cx).contains(&(
            "An agent changed \u{201c}Notes\u{201d}".to_string(),
            Some("Undo".to_string())
        )),
        "{:?}",
        notes_shown(&desk, cx)
    );
}

#[gpui::test]
fn an_agent_edits_the_paragraph_the_caret_is_in(cx: &mut TestAppContext) {
    let (desk, id, cx) = desk_with_notes(cx);
    // The caret is before "short", with "is" selected up to it.
    let before_short = "The first paragraph is ".len();
    let editor = desk
        .workspace
        .read_with(cx, |workspace, _| workspace.editor().clone());
    editor.update(cx, |editor, cx| {
        editor.edit(
            |state| {
                state.set_selection(Selection::new(
                    Position::new(1, before_short - 3),
                    Position::new(1, before_short),
                ))
            },
            cx,
        );
    });
    cx.run_until_parked();

    let tool = desk.tool();
    ask(cx, move || {
        tool.edit(&edit(id, "The first", "This very first"))
    })
    .unwrap();

    assert_eq!(
        editor_text(&desk.workspace, cx),
        "Notes\nThis very first paragraph is short.\nThe second paragraph is the last."
    );
    // The selection is on the same word, moved with the text before it.
    let moved = "This very first paragraph is ".len();
    assert_eq!(
        selection(&desk, cx),
        Selection::new(Position::new(1, moved - 3), Position::new(1, moved))
    );
    // Typing goes where the writer was.
    cx.simulate_input("was ");
    assert_eq!(
        editor_text(&desk.workspace, cx),
        "Notes\nThis very first paragraph was short.\nThe second paragraph is the last."
    );
}

#[gpui::test]
fn an_agent_replaces_the_whole_text(cx: &mut TestAppContext) {
    let (desk, id, cx) = desk_with_notes(cx);
    set_caret(&desk, 2, "The second ".len(), cx);
    type_text(&desk.workspace, "typed ", cx);

    let tool = desk.tool();
    ask(cx, move || {
        tool.replace_document(&ReplaceDocumentRequest {
            document: id.into(),
            markdown: "# Other notes\n\nOne paragraph, and nothing of what was there.".into(),
            write: claude(),
        })
    })
    .unwrap();

    let rewritten = "Other notes\nOne paragraph, and nothing of what was there.";
    assert_eq!(editor_text(&desk.workspace, cx), rewritten);
    assert_eq!(stored(&desk, id, cx), rewritten);
    // The title above the text and in the sidebar follows.
    assert_eq!(
        desk.workspace.read_with(cx, |workspace, _| workspace
            .document_title()
            .map(String::from)),
        Some("Other notes".to_string())
    );
    assert_eq!(
        notes_shown(&desk, cx),
        [(
            "Claude rewrote \u{201c}Other notes\u{201d}".to_string(),
            Some("Undo".to_string())
        )]
    );
    // The caret is in the text that is there.
    let caret = selection(&desk, cx).head;
    assert!(caret.block < 2, "{caret:?}");
    // One step back is the text with the writer's typing, which the
    // version kept before the change has too.
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(
        editor_text(&desk.workspace, cx),
        "Notes\nThe first paragraph is short.\nThe second typed paragraph is the last."
    );
    let tool = desk.tool();
    let versions = ask(cx, move || {
        tool.versions(&ListVersionsRequest {
            document: id.into(),
        })
    })
    .unwrap();
    assert_eq!(versions.versions.len(), 1);
}

#[gpui::test]
fn twenty_edits_in_a_row_are_one_note_and_one_undo(cx: &mut TestAppContext) {
    let (desk, id, cx) = desk_with_notes(cx);
    type_text(&desk.workspace, "Mine. ", cx);

    // Twenty requests, one behind the other, while the app only gets to
    // them now and then.
    let tool = desk.tool();
    let all = asking(move || {
        (0..20)
            .map(|round| {
                tool.insert(&InsertRequest {
                    document: id.into(),
                    markdown: format!("Line {round}."),
                    write: claude(),
                    ..InsertRequest::default()
                })
                .map(|changed| changed.written.changed)
            })
            .collect::<Vec<_>>()
    });
    let done = answer(cx, all);
    assert_eq!(done.len(), 20);
    assert!(
        done.iter().all(|changed| matches!(changed, Ok(true))),
        "{done:?}"
    );

    let lines: String = (0..20).map(|round| format!("\nLine {round}.")).collect();
    let expected = format!(
        "Mine. Notes\nThe first paragraph is short.\nThe second paragraph is the last.{lines}"
    );
    assert_eq!(editor_text(&desk.workspace, cx), expected);
    assert_eq!(stored(&desk, id, cx), expected);
    // One note, which counts them.
    assert_eq!(
        notes_shown(&desk, cx),
        [(
            "Claude made 20 changes to \u{201c}Mine. Notes\u{201d}".to_string(),
            Some("Undo".to_string())
        )]
    );
    assert_eq!(
        desk.workspace
            .read_with(cx, |workspace, _| workspace.agent_change_count()),
        20
    );

    // Its Undo takes all twenty out and leaves the writer's own typing.
    press_note(
        &desk,
        "Claude made 20 changes to \u{201c}Mine. Notes\u{201d}",
        cx,
    );
    assert_eq!(
        editor_text(&desk.workspace, cx),
        "Mine. Notes\nThe first paragraph is short.\nThe second paragraph is the last."
    );
    assert_eq!(
        notes_shown(&desk, cx),
        [("Undid Claude\u{2019}s 20 changes".to_string(), None)]
    );
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(editor_text(&desk.workspace, cx), notes().plain_text());
}

#[gpui::test]
fn after_the_writer_typed_the_note_no_longer_offers_undo(cx: &mut TestAppContext) {
    let (desk, id, cx) = desk_with_notes(cx);
    let tool = desk.tool();
    ask(cx, move || tool.edit(&edit(id, "is short", "is long"))).unwrap();
    assert_eq!(
        desk.workspace
            .read_with(cx, |workspace, cx| workspace.agent_change_offer(cx)),
        Some(Offer::Undo)
    );

    // The writer goes on typing: the agent's change is no longer the
    // newest step, and Undo would take out the writer's own word.
    type_text(&desk.workspace, "More. ", cx);
    assert_eq!(
        notes_shown(&desk, cx),
        [("Claude changed \u{201c}Notes\u{201d}".to_string(), None)]
    );
    // The command says so and leads to the version from before.
    let undone = desk
        .workspace
        .update(cx, |workspace, cx| workspace.undo_agent_change(cx));
    assert!(!undone);
    assert!(editor_text(&desk.workspace, cx).contains("More. "));
    assert!(editor_text(&desk.workspace, cx).contains("is long"));
    let shown = notes_shown(&desk, cx);
    assert_eq!(shown.len(), 1, "{shown:?}");
    assert!(shown[0].0.contains("version history"), "{shown:?}");
    assert_eq!(shown[0].1.as_deref(), Some("Version History"));
}
