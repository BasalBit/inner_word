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

use gpui::{Entity, EntityInputHandler as _, Modifiers, TestAppContext, VisualTestContext};
use writer_agent::{
    self as agent, AddCommentRequest, CommentRequest, CreateDocumentRequest, CreateFolderRequest,
    DeleteCommentRequest, DeleteDocumentRequest, EditRequest, ErrorKind, InsertRequest,
    ListVersionsRequest, MoveDocumentRequest, ReadRequest, Refusal, RenameDocumentRequest,
    ReplaceDocumentRequest, ReplyRequest, RestoreDocumentRequest, RestoreVersionRequest, Target,
    TrashDocumentRequest, TrashFolderRequest, WriteOptions,
};
use writer_app::{
    SaveState,
    agents_view::{AgentsView, Row},
    automation::describe_state,
    live,
    settings::keys,
    settings_view::SettingsView,
    single_instance::{self, Acquired, InstanceLock},
    version_history::VersionHistory,
    workspace::{
        AUTOSAVE_DELAY, AgentUndo, CHANGED_OUTSIDE, Names, Offer, Subject, run_sentence, sentence,
    },
};
use writer_core::{Position, Selection};
use writer_library::{Library, LibraryItem};

use super::*;

/// The app open on a library in a temporary data directory, with the
/// channel for agents open.
struct Desk {
    dir: tempfile::TempDir,
    library: Entity<Library>,
    workspace: Entity<Workspace>,
    /// The lock of the data directory, for as long as "the app runs".
    lock: Option<InstanceLock>,
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
            lock: Some(lock),
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
    // kept it when it saved for the agent, a moment before the edit), and
    // says whose edit it was kept for.
    let versions = desk
        .library
        .read_with(cx, |library, _| library.snapshots(id).unwrap());
    assert_eq!(versions.len(), 1);
    assert_eq!(
        version_labels(&desk, id, cx),
        ["Before an edit by Claude".to_string()]
    );
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

// ----- names, places, the trash --------------------------------------------------------

/// The app open on "Notes", with a second document "Other" beside it.
fn desk_with_two(
    cx: &mut TestAppContext,
) -> (Desk, DocumentId, DocumentId, &mut VisualTestContext) {
    let (dir, lock, library) = library_on_disk(cx);
    let other = add_document(&library, None, &simple_document("Other", "Text."), cx);
    let open = add_document(&library, None, &notes(), cx);
    let (desk, vcx) = open_desk(dir, lock, library, cx);
    desk.workspace.update_in(vcx, |workspace, window, cx| {
        assert!(workspace.open_document(open, true, window, cx));
    });
    vcx.run_until_parked();
    (desk, open, other, vcx)
}

fn open_id(desk: &Desk, cx: &mut VisualTestContext) -> Option<DocumentId> {
    desk.workspace
        .read_with(cx, |workspace, _| workspace.open_document_id())
}

fn title(desk: &Desk, cx: &mut VisualTestContext) -> Option<String> {
    desk.workspace.read_with(cx, |workspace, _| {
        workspace.document_title().map(String::from)
    })
}

fn in_tree(desk: &Desk, id: DocumentId, cx: &mut VisualTestContext) -> bool {
    desk.library
        .read_with(cx, |library, _| library.tree().find_document(id).is_some())
}

fn last_note(desk: &Desk, cx: &mut VisualTestContext) -> (String, Option<String>) {
    notes_shown(desk, cx)
        .pop()
        .expect("a note about the change is up")
}

#[gpui::test]
fn an_agent_renames_the_open_document_and_another(cx: &mut TestAppContext) {
    let (desk, open, other, cx) = desk_with_two(cx);

    let tool = desk.tool();
    ask(cx, move || {
        tool.rename_document(&RenameDocumentRequest {
            document: open.into(),
            title: "Field notes".into(),
            write: claude(),
            ..RenameDocumentRequest::default()
        })
    })
    .unwrap();
    // The title above the text is the new name; the text is untouched.
    assert_eq!(title(&desk, cx).as_deref(), Some("Field notes"));
    assert_eq!(editor_text(&desk.workspace, cx), notes().plain_text());
    assert_eq!(
        last_note(&desk, cx),
        (
            "Claude renamed \u{201c}Notes\u{201d} to \u{201c}Field notes\u{201d}".to_string(),
            None
        )
    );
    // The name is kept the way a name the writer chose is: an edit of the
    // heading does not take it back.
    type_text(&desk.workspace, "My ", cx);
    cx.executor().advance_clock(AUTOSAVE_DELAY * 3);
    cx.run_until_parked();
    assert_eq!(title(&desk, cx).as_deref(), Some("Field notes"));
    assert_eq!(
        desk.library
            .read_with(cx, |library, _| library.document_meta(open).unwrap().title),
        "Field notes"
    );

    // The agent gives the title back to the text.
    let tool = desk.tool();
    ask(cx, move || {
        tool.rename_document(&RenameDocumentRequest {
            document: open.into(),
            follow_text: true,
            write: claude(),
            ..RenameDocumentRequest::default()
        })
    })
    .unwrap();
    assert_eq!(title(&desk, cx).as_deref(), Some("My Notes"));
    type_text(&desk.workspace, "All ", cx);
    assert_eq!(title(&desk, cx).as_deref(), Some("My All Notes"));

    // Another document: the sidebar follows, and the note leads to it.
    let tool = desk.tool();
    ask(cx, move || {
        tool.rename_document(&RenameDocumentRequest {
            document: other.into(),
            title: "Elsewhere".into(),
            write: claude(),
            ..RenameDocumentRequest::default()
        })
    })
    .unwrap();
    assert_eq!(
        desk.library.read_with(cx, |library, _| library
            .tree()
            .find_document(other)
            .unwrap()
            .title
            .clone()),
        "Elsewhere"
    );
    assert_eq!(open_id(&desk, cx), Some(open), "the open document stays");
    let note = "Claude made 3 changes in the library";
    assert_eq!(last_note(&desk, cx), (note.to_string(), None));
    // The command goes to the document the last change was about... which
    // a run about several things does not name.
    assert_eq!(
        desk.workspace
            .read_with(cx, |workspace, cx| workspace.agent_change_offer(cx)),
        None
    );
}

#[gpui::test]
fn a_change_to_another_document_offers_to_open_it(cx: &mut TestAppContext) {
    let (desk, open, other, cx) = desk_with_two(cx);
    let tool = desk.tool();
    ask(cx, move || {
        tool.insert(&InsertRequest {
            document: other.into(),
            markdown: "A paragraph from the agent.".into(),
            write: claude(),
            ..InsertRequest::default()
        })
    })
    .unwrap();
    assert_eq!(open_id(&desk, cx), Some(open));
    assert_eq!(editor_text(&desk.workspace, cx), notes().plain_text());
    assert_eq!(
        stored(&desk, other, cx),
        "Other\nText.\nA paragraph from the agent."
    );
    let note = "Claude changed \u{201c}Other\u{201d}";
    assert_eq!(
        last_note(&desk, cx),
        (note.to_string(), Some("Open".to_string()))
    );

    // The button opens it, with the keyboard in the text.
    press_note(&desk, note, cx);
    assert_eq!(open_id(&desk, cx), Some(other));
    assert!(editor_focused(&desk.workspace, cx));
    assert_eq!(
        editor_text(&desk.workspace, cx),
        "Other\nText.\nA paragraph from the agent."
    );
    assert!(notes_shown(&desk, cx).iter().all(|(text, _)| text != note));

    // The same from the palette's command, for the open document: the
    // caret goes to the paragraph that changed.
    let tool = desk.tool();
    ask(cx, move || {
        tool.edit(&edit(other, "from the agent", "by Claude"))
    })
    .unwrap();
    set_caret(&desk, 0, 0, cx);
    let went = desk.workspace.update_in(cx, |workspace, window, cx| {
        workspace.show_agent_change(window, cx)
    });
    assert!(went);
    assert_eq!(selection(&desk, cx), Selection::caret(Position::new(2, 0)));
}

#[gpui::test]
fn a_document_changed_while_another_was_open_says_so_when_it_is_opened(cx: &mut TestAppContext) {
    let (desk, open, other, cx) = desk_with_two(cx);
    // The writer was in "Other" before, at the end of its text.
    desk.workspace.update_in(cx, |workspace, window, cx| {
        assert!(workspace.open_document(other, true, window, cx));
    });
    set_caret(&desk, 1, "Text.".len(), cx);
    desk.workspace.update_in(cx, |workspace, window, cx| {
        assert!(workspace.open_document(open, true, window, cx));
    });
    cx.run_until_parked();

    let tool = desk.tool();
    ask(cx, move || {
        tool.replace_document(&ReplaceDocumentRequest {
            document: other.into(),
            markdown: "# Other\n\nRewritten from top to bottom by the agent.".into(),
            write: claude(),
        })
    })
    .unwrap();
    cx.executor().advance_clock(live::TOAST_DURATION * 2);
    cx.run_until_parked();

    // Opened later, it starts at its beginning (the remembered caret
    // belongs to a text that is gone) and says why.
    desk.workspace.update_in(cx, |workspace, window, cx| {
        assert!(workspace.open_document(other, true, window, cx));
    });
    cx.run_until_parked();
    assert_eq!(selection(&desk, cx), Selection::caret(Position::new(0, 0)));
    assert_eq!(
        notes_shown(&desk, cx),
        [(
            CHANGED_OUTSIDE.to_string(),
            Some("Version History".to_string())
        )]
    );
    assert!(CHANGED_OUTSIDE.contains("since you last had it open"));
}

#[gpui::test]
fn an_agent_moves_trashes_and_restores(cx: &mut TestAppContext) {
    let (desk, open, other, cx) = desk_with_two(cx);
    type_text(&desk.workspace, "Typed. ", cx);

    // Moved into a folder that is made for it: the document stays open.
    let tool = desk.tool();
    ask(cx, move || {
        tool.move_document(&MoveDocumentRequest {
            document: open.into(),
            folder: Some(Target::path("Drafts")),
            create_folders: true,
            write: claude(),
            ..MoveDocumentRequest::default()
        })
    })
    .unwrap();
    assert_eq!(open_id(&desk, cx), Some(open));
    let folder = desk.library.read_with(cx, |library, _| {
        library
            .tree()
            .find_document(open)
            .unwrap()
            .folder_id
            .expect("in the folder")
    });
    assert_eq!(
        desk.library
            .read_with(cx, |library, _| library.folder(folder).unwrap().name),
        "Drafts"
    );
    assert_eq!(
        last_note(&desk, cx),
        (
            "Claude moved \u{201c}Typed. Notes\u{201d}".to_string(),
            None
        )
    );
    cx.executor().advance_clock(live::TOAST_DURATION * 2);
    cx.run_until_parked();

    // Another document goes to the trash: one step brings it back.
    let tool = desk.tool();
    ask(cx, move || {
        tool.trash_document(&TrashDocumentRequest {
            document: other.into(),
            write: claude(),
        })
    })
    .unwrap();
    assert!(!in_tree(&desk, other, cx));
    let note = "Claude moved \u{201c}Other\u{201d} to the trash";
    assert_eq!(
        last_note(&desk, cx),
        (note.to_string(), Some("Undo".to_string()))
    );
    press_note(&desk, note, cx);
    assert!(in_tree(&desk, other, cx));
    cx.executor().advance_clock(live::TOAST_DURATION * 2);
    cx.run_until_parked();

    // The open document goes to the trash: it closes as it does when the
    // writer trashes it, what was typed is in it, and the next document
    // is shown with the keyboard still in the text.
    let tool = desk.tool();
    ask(cx, move || {
        tool.trash_document(&TrashDocumentRequest {
            document: open.into(),
            write: claude(),
        })
    })
    .unwrap();
    assert_eq!(open_id(&desk, cx), Some(other));
    assert!(editor_focused(&desk.workspace, cx));
    assert!(stored(&desk, open, cx).starts_with("Typed. Notes"));
    assert_eq!(
        last_note(&desk, cx),
        (
            "Claude moved \u{201c}Typed. Notes\u{201d} to the trash".to_string(),
            Some("Undo".to_string())
        )
    );
    cx.executor().advance_clock(live::TOAST_DURATION * 2);
    cx.run_until_parked();

    // The agent brings it back itself.
    let tool = desk.tool();
    ask(cx, move || {
        tool.restore_document(&RestoreDocumentRequest {
            document: open.into(),
            write: claude(),
        })
    })
    .unwrap();
    assert!(in_tree(&desk, open, cx));
    assert_eq!(open_id(&desk, cx), Some(other), "nothing is opened unasked");
    assert_eq!(
        last_note(&desk, cx),
        (
            "Claude brought \u{201c}Typed. Notes\u{201d} back from the trash".to_string(),
            Some("Open".to_string())
        )
    );
}

#[gpui::test]
fn an_agent_deletes_for_good(cx: &mut TestAppContext) {
    let (desk, open, other, cx) = desk_with_two(cx);
    type_text(&desk.workspace, "Typed. ", cx);

    // Without the confirmation nothing is deleted, and the agent is told
    // what the tool would have told it.
    let tool = desk.tool();
    let refused = ask(cx, move || {
        tool.delete_document(&DeleteDocumentRequest {
            document: other.into(),
            permanently: false,
            write: claude(),
        })
    })
    .unwrap_err();
    assert_eq!(refused.kind(), ErrorKind::ConfirmationRequired);
    assert!(in_tree(&desk, other, cx));
    assert!(notes_shown(&desk, cx).is_empty(), "nothing happened");

    // Another document.
    let tool = desk.tool();
    ask(cx, move || {
        tool.delete_document(&DeleteDocumentRequest {
            document: other.into(),
            permanently: true,
            write: claude(),
        })
    })
    .unwrap();
    assert!(
        desk.library
            .read_with(cx, |library, _| !library.document_exists(other).unwrap())
    );
    assert_eq!(open_id(&desk, cx), Some(open));
    assert_eq!(
        last_note(&desk, cx),
        (
            "Claude deleted \u{201c}Other\u{201d} for good".to_string(),
            None
        )
    );
    cx.executor().advance_clock(live::TOAST_DURATION * 2);
    cx.run_until_parked();

    // The open document: it closes, and the window shows the empty state.
    let tool = desk.tool();
    ask(cx, move || {
        tool.delete_document(&DeleteDocumentRequest {
            document: open.into(),
            permanently: true,
            write: claude(),
        })
    })
    .unwrap();
    assert_eq!(open_id(&desk, cx), None);
    assert!(
        desk.library
            .read_with(cx, |library, _| !library.document_exists(open).unwrap())
    );
    assert_eq!(
        last_note(&desk, cx),
        (
            "Claude deleted \u{201c}Typed. Notes\u{201d} for good".to_string(),
            None
        )
    );
    // Keys still have somewhere to go.
    cx.simulate_keystrokes("cmd-n");
    cx.run_until_parked();
    assert!(open_id(&desk, cx).is_some());
}

// ----- new documents and folders -------------------------------------------------------

#[gpui::test]
fn an_agent_creates_documents_and_folders(cx: &mut TestAppContext) {
    let (dir, lock, library) = library_on_disk(cx);
    let (book, ideas) = library.update(cx, |library, cx| {
        (
            library.create_folder(None, "Book", cx).unwrap().id,
            library.create_folder(None, "Ideas", cx).unwrap().id,
        )
    });
    let chapter = add_document(
        &library,
        Some(book),
        &simple_document("Chapter 1", "It began."),
        cx,
    );
    add_document(
        &library,
        Some(ideas),
        &simple_document("An idea", "Maybe."),
        cx,
    );
    let (desk, cx) = open_desk(dir, lock, library, cx);
    desk.workspace.update_in(cx, |workspace, window, cx| {
        assert!(workspace.open_document(chapter, true, window, cx));
    });
    // The writer has "Book" open in the sidebar, "Ideas" closed, and the
    // folder "Ideas" selected.
    let panel = desk
        .workspace
        .read_with(cx, |workspace, _| workspace.panel().clone());
    panel.update(cx, |panel, cx| {
        panel.set_expanded(book, true, cx);
        panel.set_expanded(ideas, false, cx);
        panel.select(Some(LibraryItem::Folder(ideas)), cx);
    });
    cx.run_until_parked();
    type_text(&desk.workspace, "Typed. ", cx);

    let tool = desk.tool();
    let (folder, second, deep) = ask(cx, move || {
        let folder = tool
            .create_folder(&CreateFolderRequest {
                path: "Book/Part two".into(),
                write: claude(),
                ..CreateFolderRequest::default()
            })
            .unwrap();
        let second = tool
            .create_document(&CreateDocumentRequest {
                markdown: "# Chapter 2\n\nIt went on.".into(),
                folder: Some(Target::path("Book")),
                write: claude(),
                ..CreateDocumentRequest::default()
            })
            .unwrap();
        let deep = tool
            .create_document(&CreateDocumentRequest {
                markdown: "A note.".into(),
                title: Some("Loose end".into()),
                folder: Some(Target::path("Ideas/Later")),
                create_folders: true,
                write: claude(),
                ..CreateDocumentRequest::default()
            })
            .unwrap();
        (folder, second, deep)
    });
    let part_two = folder.folder.id;
    let second = second.document.id;
    let deep = deep.document.id;

    // The sidebar shows what is in the open folder, at once.
    let (visible, selected, book_open, ideas_open) = panel.read_with(cx, |panel, _| {
        (
            panel.visible_items(),
            panel.selected(),
            panel.is_expanded(book),
            panel.is_expanded(ideas),
        )
    });
    assert!(
        visible.contains(&LibraryItem::Folder(part_two)),
        "{visible:?}"
    );
    assert!(
        visible.contains(&LibraryItem::Document(second)),
        "{visible:?}"
    );
    // What is in the closed folder is in the library and not unfolded.
    assert!(in_tree(&desk, deep, cx));
    assert!(
        !visible.contains(&LibraryItem::Document(deep)),
        "{visible:?}"
    );
    // The writer's selection and open folders are as they were.
    assert_eq!(selected, Some(LibraryItem::Folder(ideas)));
    assert!(book_open && !ideas_open);
    // The open document, its unsaved typing and the keyboard too.
    assert_eq!(open_id(&desk, cx), Some(chapter));
    assert_eq!(
        editor_text(&desk.workspace, cx),
        "Typed. Chapter 1\nIt began."
    );
    assert!(editor_focused(&desk.workspace, cx));
    assert_eq!(
        desk.library
            .read_with(cx, |library, _| library.document_meta(deep).unwrap().title),
        "Loose end"
    );
    assert_eq!(
        last_note(&desk, cx),
        ("Claude made 3 changes in the library".to_string(), None)
    );

    // One new document by itself is named, and can be opened.
    cx.executor().advance_clock(live::TOAST_DURATION * 2);
    cx.run_until_parked();
    let tool = desk.tool();
    let third = ask(cx, move || {
        tool.create_document(&CreateDocumentRequest {
            markdown: "# Chapter 3\n\nAnd on.".into(),
            folder: Some(Target::path("Book")),
            write: claude(),
            ..CreateDocumentRequest::default()
        })
    })
    .unwrap();
    let note = "Claude created \u{201c}Chapter 3\u{201d}";
    assert_eq!(
        last_note(&desk, cx),
        (note.to_string(), Some("Open".to_string()))
    );
    press_note(&desk, note, cx);
    assert_eq!(open_id(&desk, cx), Some(third.document.id));

    // A folder goes to the trash with what is in it, and comes back.
    cx.executor().advance_clock(live::TOAST_DURATION * 2);
    cx.run_until_parked();
    let tool = desk.tool();
    ask(cx, move || {
        tool.trash_folder(&TrashFolderRequest {
            folder: Target::path("Ideas"),
            write: claude(),
        })
    })
    .unwrap();
    assert!(!in_tree(&desk, deep, cx));
    let note = "Claude moved the folder \u{201c}Ideas\u{201d} to the trash";
    assert_eq!(
        last_note(&desk, cx),
        (note.to_string(), Some("Undo".to_string()))
    );
    press_note(&desk, note, cx);
    assert!(in_tree(&desk, deep, cx));
}

// ----- comments ------------------------------------------------------------------------

fn threads(desk: &Desk, cx: &mut VisualTestContext) -> Vec<(String, bool, usize)> {
    desk.workspace.read_with(cx, |workspace, cx| {
        workspace
            .editor()
            .read(cx)
            .document()
            .comments()
            .map(|thread| (thread.body.clone(), thread.resolved, thread.replies.len()))
            .collect()
    })
}

fn listed_threads(desk: &Desk, cx: &mut VisualTestContext) -> usize {
    desk.workspace.read_with(cx, |workspace, cx| {
        workspace.comments().read(cx).threads().len()
    })
}

#[gpui::test]
fn comments_of_an_agent_show_in_the_comments_panel(cx: &mut TestAppContext) {
    let (desk, id, cx) = desk_with_notes(cx);
    set_caret(&desk, 2, 0, cx);
    type_text(&desk.workspace, "Typed. ", cx);
    assert_eq!(listed_threads(&desk, cx), 0);
    assert!(
        !desk
            .workspace
            .read_with(cx, |workspace, _| workspace.has_right_panel())
    );

    // A comment on a quote of the text.
    let tool = desk.tool();
    let added = ask(cx, move || {
        tool.add_comment(&AddCommentRequest {
            document: id.into(),
            quote: "first paragraph".into(),
            body: "Is it the first?".into(),
            write: claude(),
            ..AddCommentRequest::default()
        })
    })
    .unwrap();
    let thread = added.thread.id;
    assert_eq!(
        threads(&desk, cx),
        [("Is it the first?".to_string(), false, 0)]
    );
    assert_eq!(listed_threads(&desk, cx), 1);
    // The panel comes up for it, as it does for a comment of the writer's;
    // the keyboard stays in the text, with what was typed.
    assert!(
        desk.workspace
            .read_with(cx, |workspace, _| workspace.has_right_panel())
    );
    assert!(editor_focused(&desk.workspace, cx));
    assert!(editor_text(&desk.workspace, cx).contains("Typed. The second"));
    assert_eq!(
        last_note(&desk, cx),
        (
            "Claude commented on \u{201c}Notes\u{201d}".to_string(),
            Some("Undo".to_string())
        )
    );

    // An answer, resolved, opened again.
    let tool = desk.tool();
    ask(cx, move || {
        tool.reply(&ReplyRequest {
            document: id.into(),
            comment: thread,
            body: "It is.".into(),
            write: claude(),
        })
        .unwrap();
        tool.resolve_comment(&CommentRequest {
            document: id.into(),
            comment: thread,
            write: claude(),
        })
    })
    .unwrap();
    assert_eq!(
        threads(&desk, cx),
        [("Is it the first?".to_string(), true, 1)]
    );
    assert_eq!(
        last_note(&desk, cx),
        (
            "Claude made 3 changes to \u{201c}Notes\u{201d}".to_string(),
            Some("Undo".to_string())
        )
    );
    // The stored document is what the editor has.
    let tool = desk.tool();
    let list = ask(cx, move || {
        tool.comments(&agent::ListCommentsRequest {
            document: id.into(),
            open_only: false,
        })
    })
    .unwrap();
    assert_eq!(list.threads.len(), 1);
    assert!(list.threads[0].resolved);
    assert_eq!(list.threads[0].replies.len(), 1);
    assert_eq!(
        list.threads[0].anchored_text.as_deref(),
        Some("first paragraph")
    );

    // Deleted: gone from the panel, and one step of undo brings it back.
    let tool = desk.tool();
    ask(cx, move || {
        tool.delete_comment(&DeleteCommentRequest {
            document: id.into(),
            comment: thread,
            reply: None,
            write: claude(),
        })
    })
    .unwrap();
    assert!(threads(&desk, cx).is_empty());
    assert_eq!(listed_threads(&desk, cx), 0);
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(
        threads(&desk, cx),
        [("Is it the first?".to_string(), true, 1)]
    );
    assert!(editor_text(&desk.workspace, cx).contains("Typed. The second"));
}

// ----- versions ------------------------------------------------------------------------

#[gpui::test]
fn an_agent_restores_a_version(cx: &mut TestAppContext) {
    let (desk, id, cx) = desk_with_notes(cx);
    let tool = desk.tool();
    let (first, restored) = ask(cx, move || {
        // The edit keeps the text as it was as a version; then the agent
        // goes back to that version.
        tool.edit(&edit(id, "is short", "is long")).unwrap();
        let versions = tool
            .versions(&ListVersionsRequest {
                document: id.into(),
            })
            .unwrap();
        let first = versions.versions.last().unwrap().id;
        let restored = tool.restore_version(&RestoreVersionRequest {
            version: first.0,
            write: claude(),
            ..RestoreVersionRequest::default()
        });
        (first, restored)
    });
    let restored = restored.unwrap();
    assert_eq!(restored.restored.id, first);
    assert_eq!(editor_text(&desk.workspace, cx), notes().plain_text());
    assert_eq!(stored(&desk, id, cx), notes().plain_text());
    assert_eq!(save_state(&desk, cx), Some(SaveState::Saved));
    // The text from before the restore is a version too, and says why.
    let labels = version_labels(&desk, id, cx);
    assert_eq!(labels.len(), 2, "{labels:?}");
    assert!(
        labels.iter().any(|label| label.contains("Claude")),
        "{labels:?}"
    );
    // Both changes are steps of undo, one on the other.
    assert_eq!(
        last_note(&desk, cx),
        (
            "Claude made 2 changes to \u{201c}Notes\u{201d}".to_string(),
            Some("Undo".to_string())
        )
    );
    cx.simulate_keystrokes("cmd-z");
    assert!(editor_text(&desk.workspace, cx).contains("is long"));
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(editor_text(&desk.workspace, cx), notes().plain_text());
}

// ----- while a dialog is open ----------------------------------------------------------

fn has_modal(desk: &Desk, cx: &mut VisualTestContext) -> bool {
    desk.workspace.read_with(cx, |workspace, cx| {
        workspace.modal_layer().read(cx).has_modal()
    })
}

#[gpui::test]
fn a_change_arrives_while_a_dialog_is_open(cx: &mut TestAppContext) {
    let (desk, id, cx) = desk_with_notes(cx);

    // The settings, with the section of this document.
    cx.simulate_keystrokes("cmd-,");
    cx.run_until_parked();
    assert!(desk.workspace.read_with(cx, |workspace, cx| {
        workspace
            .modal_layer()
            .read(cx)
            .active::<SettingsView>()
            .is_some()
    }));
    let tool = desk.tool();
    ask(cx, move || tool.edit(&edit(id, "is short", "is long"))).unwrap();
    assert!(editor_text(&desk.workspace, cx).contains("is long"));
    assert!(has_modal(&desk, cx), "the settings stay open");
    assert!(
        !editor_focused(&desk.workspace, cx),
        "and keep the keyboard"
    );
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(editor_focused(&desk.workspace, cx));

    // The command palette: what is typed goes on going into its field.
    cx.simulate_keystrokes("cmd-shift-p");
    cx.simulate_input("vers");
    let tool = desk.tool();
    ask(cx, move || tool.edit(&edit(id, "is long", "is longer"))).unwrap();
    assert!(editor_text(&desk.workspace, cx).contains("is longer"));
    assert!(has_modal(&desk, cx));
    cx.simulate_input("ion history");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();

    // The version history of the very document: the version kept before
    // the next change is listed while the list is open.
    let history = desk
        .workspace
        .read_with(cx, |workspace, cx| {
            workspace.modal_layer().read(cx).active::<VersionHistory>()
        })
        .expect("the palette opened the version history");
    let listed = history.read_with(cx, |history, _| history.versions().len());
    assert_eq!(listed, 2);
    let tool = desk.tool();
    ask(cx, move || tool.edit(&edit(id, "the last", "not the last"))).unwrap();
    assert!(editor_text(&desk.workspace, cx).contains("not the last"));
    let rows = history.read_with(cx, |history, _| history.row_labels());
    assert_eq!(rows.len(), 3, "{rows:?}");
    assert_eq!(rows[0].1, "Before an edit by Claude");
    assert!(has_modal(&desk, cx));
    // Restoring from the list works on the text as the agent left it.
    cx.simulate_keystrokes("down down enter");
    cx.run_until_parked();
    assert_eq!(editor_text(&desk.workspace, cx), notes().plain_text());
    assert_eq!(stored(&desk, id, cx), notes().plain_text());
}

// ----- an input method -----------------------------------------------------------------

/// Starts a composition at the caret: a dead key that waits for its letter.
fn compose(desk: &Desk, marked: &str, cx: &mut VisualTestContext) {
    let editor = desk
        .workspace
        .read_with(cx, |workspace, _| workspace.editor().clone());
    editor.update_in(cx, |editor, window, cx| {
        editor.replace_and_mark_text_in_range(None, marked, Some(1..1), window, cx);
    });
    assert!(editor.read_with(cx, |editor, _| editor.is_composing()));
}

fn composing(desk: &Desk, cx: &mut VisualTestContext) -> bool {
    desk.workspace.read_with(cx, |workspace, cx| {
        workspace.editor().read(cx).is_composing()
    })
}

/// Lets the app's clock run (the wait for a composition is a timer) until
/// `asked` has its answer, for at most `limit` of the app's time.
fn answer_in_time<T>(cx: &mut VisualTestContext, asked: Asked<T>, limit: Duration) -> T {
    let mut passed = Duration::ZERO;
    let started = Instant::now();
    loop {
        cx.run_until_parked();
        if let Ok(answer) = asked.0.try_recv() {
            asked.1.join().unwrap();
            cx.run_until_parked();
            return answer;
        }
        assert!(started.elapsed() < Duration::from_secs(60), "no answer");
        if passed < limit {
            cx.executor().advance_clock(live::COMPOSITION_POLL);
            passed += live::COMPOSITION_POLL;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[gpui::test]
fn a_change_waits_for_a_composition_to_end(cx: &mut TestAppContext) {
    let (desk, id, cx) = desk_with_notes(cx);
    set_caret(&desk, 2, "The second paragraph is the last.".len(), cx);
    compose(&desk, "\u{b4}", cx);

    let tool = desk.tool();
    let asked = asking(move || tool.edit(&edit(id, "is short", "is long")));
    // The request is there and waits: nothing is changed, and the
    // composition goes on.
    let started = Instant::now();
    while cx.update(|_, cx| live::waiting(cx)) == 0 {
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "no request came"
        );
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(2));
    }
    for _ in 0..10 {
        cx.executor().advance_clock(live::COMPOSITION_POLL);
        cx.run_until_parked();
    }
    assert!(composing(&desk, cx));
    assert!(editor_text(&desk.workspace, cx).contains("is short"));
    assert!(asked.0.try_recv().is_err(), "not answered yet");

    // The letter arrives; the change follows.
    let editor = desk
        .workspace
        .read_with(cx, |workspace, _| workspace.editor().clone());
    editor.update_in(cx, |editor, window, cx| {
        editor.replace_text_in_range(None, "\u{e9}", window, cx);
    });
    answer_in_time(cx, asked, Duration::from_secs(1)).expect("made after the composition");
    assert_eq!(
        editor_text(&desk.workspace, cx),
        "Notes\nThe first paragraph is long.\nThe second paragraph is the last.\u{e9}"
    );
    assert_eq!(stored(&desk, id, cx), editor_text(&desk.workspace, cx));
}

#[gpui::test]
fn a_composition_that_does_not_end_is_never_broken(cx: &mut TestAppContext) {
    let (desk, id, cx) = desk_with_notes(cx);
    set_caret(&desk, 2, 0, cx);
    compose(&desk, "\u{b4}", cx);

    let tool = desk.tool();
    let asked = asking(move || tool.edit(&edit(id, "is short", "is long")));
    let refused = answer_in_time(cx, asked, live::COMPOSITION_PATIENCE * 2).unwrap_err();
    assert_eq!(refused.kind(), ErrorKind::AppIsOpen);
    assert_eq!(refused.exit_code(), 5);
    assert!(
        matches!(
            &refused,
            agent::Error::AppIsOpen {
                why: Refusal::Composing,
                ..
            }
        ),
        "{refused:?}"
    );
    assert!(refused.to_string().contains("few seconds"), "{refused}");
    // Nothing was changed, and the writer is still composing.
    assert!(composing(&desk, cx));
    assert!(editor_text(&desk.workspace, cx).contains("is short"));
    assert!(stored(&desk, id, cx).contains("is short"));
    assert!(notes_shown(&desk, cx).is_empty());
    // A read does not wait for the composition.
    let tool = desk.tool();
    let read = ask(cx, move || {
        tool.read(&ReadRequest {
            document: id.into(),
            ..ReadRequest::default()
        })
    })
    .unwrap();
    assert!(read.content.contains("is short"));
    assert!(composing(&desk, cx));
}

// ----- saving --------------------------------------------------------------------------

#[gpui::test(iterations = 20)]
fn a_change_arrives_while_an_autosave_is_on_its_way(cx: &mut TestAppContext) {
    let (desk, id, cx) = desk_with_notes(cx);
    set_caret(&desk, 2, 0, cx);
    let editor = desk
        .workspace
        .read_with(cx, |workspace, _| workspace.editor().clone());
    // Typing; the autosave starts on the background thread; more typing
    // while it is on its way.
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.insert_text("One. "), cx);
    });
    desk.workspace
        .update(cx, |workspace, cx| workspace.autosave(cx));
    assert!(
        desk.workspace
            .read_with(cx, |workspace, _| workspace.is_saving_in_background())
    );
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.insert_text("Two. "), cx);
    });

    // Whichever of the two gets to the library first, the older copy of
    // the text never lands on the newer one.
    let tool = desk.tool();
    ask(cx, move || tool.edit(&edit(id, "is short", "is long"))).unwrap();
    let expected =
        "Notes\nThe first paragraph is long.\nOne. Two. The second paragraph is the last.";
    assert_eq!(editor_text(&desk.workspace, cx), expected);
    assert_eq!(stored(&desk, id, cx), expected);
    cx.executor().advance_clock(AUTOSAVE_DELAY * 8);
    cx.run_until_parked();
    assert_eq!(stored(&desk, id, cx), expected);
    assert_eq!(save_state(&desk, cx), Some(SaveState::Saved));
    assert!(
        !desk
            .workspace
            .read_with(cx, |workspace, _| workspace.is_saving_in_background())
    );
}

#[gpui::test]
fn a_read_sees_the_text_as_it_is_on_screen(cx: &mut TestAppContext) {
    let (desk, id, cx) = desk_with_notes(cx);
    type_text(&desk.workspace, "Not saved yet. ", cx);
    assert_eq!(save_state(&desk, cx), Some(SaveState::Edited));

    let tool = desk.tool();
    let read = ask(cx, move || {
        tool.read(&ReadRequest {
            document: id.into(),
            ..ReadRequest::default()
        })
    })
    .unwrap();
    assert!(
        read.content.contains("Not saved yet. Notes"),
        "{}",
        read.content
    );
    assert_eq!(save_state(&desk, cx), Some(SaveState::Saved));
    // Reading is no change: no note, nothing counted.
    assert!(notes_shown(&desk, cx).is_empty());
    assert_eq!(
        desk.workspace
            .read_with(cx, |workspace, _| workspace.agent_change_count()),
        0
    );
}

// ----- the switch ----------------------------------------------------------------------

fn may_change(desk: &Desk, cx: &mut VisualTestContext) -> bool {
    desk.workspace
        .read_with(cx, |workspace, cx| workspace.agents_may_change(cx))
}

#[gpui::test]
fn with_the_switch_off_agents_only_read(cx: &mut TestAppContext) {
    let (desk, id, cx) = desk_with_notes(cx);
    assert!(may_change(&desk, cx), "on unless it was switched off");
    let tool = desk.tool();
    assert!(ask(cx, move || tool.status()).unwrap().app_takes_changes);

    desk.workspace.update(cx, |workspace, cx| {
        workspace.set_agents_may_change(false, cx)
    });
    type_text(&desk.workspace, "Typed. ", cx);

    let tool = desk.tool();
    let refused = ask(cx, move || tool.edit(&edit(id, "is short", "is long"))).unwrap_err();
    assert_eq!(refused.kind(), ErrorKind::AppIsOpen);
    assert_eq!(refused.exit_code(), 5);
    assert!(
        matches!(
            &refused,
            agent::Error::AppIsOpen {
                why: Refusal::SwitchedOff,
                ..
            }
        ),
        "{refused:?}"
    );
    let said = refused.to_string();
    assert!(said.contains("nothing was changed"), "{said}");
    assert!(said.contains("Agents panel"), "{said}");
    assert_eq!(
        editor_text(&desk.workspace, cx),
        "Typed. Notes\nThe first paragraph is short.\nThe second paragraph is the last."
    );
    assert!(notes_shown(&desk, cx).is_empty());

    // Reading works as it always did, and sees what is on screen.
    let tool = desk.tool();
    let (status, read) = ask(cx, move || {
        (
            tool.status(),
            tool.read(&ReadRequest {
                document: id.into(),
                ..ReadRequest::default()
            }),
        )
    });
    let status = status.unwrap();
    assert!(status.app_is_open && !status.app_takes_changes);
    assert!(read.unwrap().content.contains("Typed. Notes"));

    // Switched on again, the change is made.
    desk.workspace.update(cx, |workspace, cx| {
        workspace.set_agents_may_change(true, cx)
    });
    let tool = desk.tool();
    ask(cx, move || tool.edit(&edit(id, "is short", "is long"))).unwrap();
    assert!(editor_text(&desk.workspace, cx).contains("is long"));
}

fn agents_panel(desk: &Desk, cx: &mut VisualTestContext) -> Entity<AgentsView> {
    desk.workspace
        .read_with(cx, |workspace, cx| {
            workspace.modal_layer().read(cx).active::<AgentsView>()
        })
        .expect("the Agents panel is open")
}

#[gpui::test]
fn the_switch_is_in_the_agents_panel_for_keys_and_pointer(cx: &mut TestAppContext) {
    let (desk, _, cx) = desk_with_notes(cx);
    cx.simulate_keystrokes("cmd-alt-a");
    cx.run_until_parked();
    let panel = agents_panel(&desk, cx);
    // No agents are looked for in a test without a home: the switch is
    // the library's and is there all the same.
    assert_eq!(
        panel.read_with(cx, |panel, _| panel.rows().to_vec()),
        [Row::Switch]
    );
    assert_eq!(
        panel.read_with(cx, |panel, cx| panel.button_labels(Row::Switch, cx)),
        ["Switch Off"]
    );
    assert!(cx.debug_bounds("agents-switch-state").is_some());

    // The keyboard: enter on the row.
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(!may_change(&desk, cx));
    assert_eq!(
        panel.read_with(cx, |panel, cx| panel.button_labels(Row::Switch, cx)),
        ["Switch On"]
    );
    assert!(
        notes_shown(&desk, cx)
            .iter()
            .any(|(text, _)| text == "Agents can only read while The Writer is open")
    );
    assert!(
        panel
            .read_with(cx, |panel, cx| panel.describe(cx))
            .contains("changes_while_open=off")
    );

    // The pointer: the button of the row.
    let button = cx
        .debug_bounds("agents-switch-0")
        .expect("the button of the switch is drawn");
    cx.simulate_click(button.center(), Modifiers::default());
    cx.run_until_parked();
    assert!(may_change(&desk, cx));
    assert!(
        notes_shown(&desk, cx)
            .iter()
            .any(|(text, _)| text == "Agents can make changes while The Writer is open")
    );
    // It is a setting of the library: it is there after a relaunch.
    assert_eq!(
        desk.library.read_with(cx, |library, _| library
            .setting::<bool>(live::CHANGES_SETTING)
            .unwrap()),
        Some(true)
    );
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(editor_focused(&desk.workspace, cx));
}

// ----- quitting ------------------------------------------------------------------------

#[gpui::test]
fn a_request_that_waits_when_the_app_quits_is_made_by_the_tool(cx: &mut TestAppContext) {
    let (mut desk, id, cx) = desk_with_notes(cx);
    assert!(cx.update(|_, cx| live::is_open(cx)));
    let socket = cx.update(|_, cx| live::socket(cx)).unwrap();
    assert!(socket.exists());

    // A request arrives and waits for the main thread, which is busy
    // quitting: the channel closes with the request in its queue.
    let tool = desk.tool();
    let asked = asking(move || tool.edit(&edit(id, "is short", "is long")));
    let started = Instant::now();
    while cx.update(|_, cx| live::waiting(cx)) == 0 {
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "no request came"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    cx.update(|_, cx| live::close(cx));
    assert!(!cx.update(|_, cx| live::is_open(cx)));
    assert!(!socket.exists(), "the socket is gone");
    // The app did not touch the text for it.
    assert!(editor_text(&desk.workspace, cx).contains("is short"));
    // The process ends, and with it the lock: the tool, which was told
    // that the app is quitting, makes the change itself. Once.
    drop(desk.lock.take());
    let changed = answer(cx, asked).expect("the tool wrote after the app was gone");
    assert!(changed.written.changed);
    assert_eq!(
        stored(&desk, id, cx),
        "Notes\nThe first paragraph is long.\nThe second paragraph is the last."
    );
}

#[gpui::test]
fn closing_the_channel_twice_and_opening_it_again(cx: &mut TestAppContext) {
    let (desk, id, cx) = desk_with_notes(cx);
    cx.update(|_, cx| {
        live::close(cx);
        live::close(cx);
    });
    // Without a channel the app is what it was before 0.6.0 to a tool.
    let tool = desk.tool();
    let refused = ask(cx, move || tool.edit(&edit(id, "is short", "is long"))).unwrap_err();
    assert_eq!(refused.exit_code(), 5);
    assert!(
        matches!(
            &refused,
            agent::Error::AppIsOpen {
                why: Refusal::OldVersion,
                ..
            }
        ),
        "{refused:?}"
    );
    assert!(editor_text(&desk.workspace, cx).contains("is short"));

    let window = cx
        .update(|window, _| window.window_handle())
        .downcast::<Workspace>()
        .unwrap();
    cx.update(|_, cx| live::open(desk.dir.path(), window, cx))
        .unwrap();
    let tool = desk.tool();
    ask(cx, move || tool.edit(&edit(id, "is short", "is long"))).unwrap();
    assert!(editor_text(&desk.workspace, cx).contains("is long"));
}

// ----- what a scripted run and the menus see ---------------------------------------------

#[gpui::test]
fn the_state_line_says_what_came_in(cx: &mut TestAppContext) {
    let (desk, id, cx) = desk_with_notes(cx);
    let live_of = |desk: &Desk, cx: &mut VisualTestContext| {
        desk.workspace.update_in(cx, |workspace, window, cx| {
            let line = describe_state(workspace, window, cx);
            let from = line.find("live=[").expect("the state line has the channel");
            let to = from + line[from..].find(']').unwrap() + 1;
            line[from..to].to_string()
        })
    };
    assert_eq!(
        live_of(&desk, cx),
        "live=[channel=open switch=on waiting=0 changes=0]"
    );
    let tool = desk.tool();
    ask(cx, move || {
        tool.edit(&edit(id, "is short", "is long")).unwrap();
        tool.edit(&edit(id, "the last", "not the last"))
    })
    .unwrap();
    let line = live_of(&desk, cx);
    assert!(
        line.starts_with(
            "live=[channel=open switch=on waiting=0 changes=2 last=\"Claude made 2 changes to \
             \u{201c}Notes\u{201d}\" run=2 offers=Undo cost=[save "
        ),
        "{line}"
    );
    // What the last change cost on the main thread, part by part.
    let cost = desk
        .workspace
        .read_with(cx, |workspace, _| workspace.last_agent_change_cost())
        .expect("a change was made");
    assert!(
        cost.change > Duration::ZERO && cost.window > Duration::ZERO,
        "{cost:?}"
    );
    desk.workspace.update(cx, |workspace, cx| {
        workspace.set_agents_may_change(false, cx)
    });
    cx.update(|_, cx| live::close(cx));
    assert!(live_of(&desk, cx).starts_with("live=[channel=closed switch=off waiting=0 changes=2"));
}

#[gpui::test]
fn the_last_change_has_commands_and_menu_items(cx: &mut TestAppContext) {
    let (desk, id, cx) = desk_with_notes(cx);
    let titles: Vec<String> = cx.update(|_, cx| {
        writer_app::commands::all(cx)
            .into_iter()
            .map(|command| command.title.to_string())
            .collect()
    });
    for title in [
        "Undo the last change by an agent",
        "Go to the last change by an agent",
    ] {
        assert!(titles.iter().any(|listed| listed == title), "{title}");
    }
    let menu_items: Vec<String> = writer_app::menus::app_menus()
        .into_iter()
        .flat_map(|menu| menu.items)
        .filter_map(|item| match item {
            gpui::MenuItem::Action { name, .. } => Some(name.to_string()),
            _ => None,
        })
        .collect();
    for item in [
        "Undo Last Change by an Agent",
        "Go to Last Change by an Agent",
    ] {
        assert!(menu_items.iter().any(|listed| listed == item), "{item}");
    }

    // Before any change both say that there is none.
    cx.dispatch_action(live::UndoAgentChange);
    cx.run_until_parked();
    assert_eq!(
        last_note(&desk, cx).0,
        "No agent has changed anything since The Writer was opened"
    );

    // From the palette, with the keyboard alone.
    let tool = desk.tool();
    ask(cx, move || tool.edit(&edit(id, "is short", "is long"))).unwrap();
    cx.simulate_keystrokes("cmd-shift-p");
    cx.simulate_input("undo the last change by an agent");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(editor_text(&desk.workspace, cx), notes().plain_text());
    assert!(editor_focused(&desk.workspace, cx));
}

// ----- the words -----------------------------------------------------------------------

#[test]
fn the_note_says_who_did_what() {
    let named = |document: &str| Names {
        document: Some(document.to_string()),
        ..Names::default()
    };
    let q = |name: &str| format!("\u{201c}{name}\u{201d}");
    assert_eq!(
        sentence("Claude", "edit", &named("Chapter 2")),
        format!("Claude changed {}", q("Chapter 2"))
    );
    assert_eq!(
        sentence("An agent", "insert", &named("")),
        format!("An agent changed {}", q("Untitled"))
    );
    assert_eq!(
        sentence(
            "Codex",
            "rename_document",
            &Names {
                document: Some("New".into()),
                document_before: Some("Old".into()),
                ..Names::default()
            }
        ),
        format!("Codex renamed {} to {}", q("Old"), q("New"))
    );
    assert_eq!(
        sentence(
            "Claude",
            "delete_document",
            &Names {
                document_before: Some("Gone".into()),
                ..Names::default()
            }
        ),
        format!("Claude deleted {} for good", q("Gone"))
    );
    assert_eq!(
        sentence(
            "Claude",
            "trash_folder",
            &Names {
                folder: Some("Drafts".into()),
                ..Names::default()
            }
        ),
        format!("Claude moved the folder {} to the trash", q("Drafts"))
    );
    assert_eq!(
        sentence("Claude", "empty_trash", &Names::default()),
        "Claude emptied the trash"
    );
    // An operation of a later engine still says something true.
    assert_eq!(
        sentence("Claude", "translate", &named("Notes")),
        format!("Claude changed {}", q("Notes"))
    );
    assert_eq!(
        sentence("Claude", "translate", &Names::default()),
        "Claude changed the library"
    );
    // Every operation of the engine that writes has words of its own.
    for operation in writer_agent::live::Operation::CHANGES {
        let names = Names {
            document: Some("D".into()),
            folder: Some("F".into()),
            ..Names::default()
        };
        let said = sentence("Claude", operation, &names);
        assert!(said.starts_with("Claude "), "{said}");
        if !matches!(*operation, "edit" | "insert" | "replace_section") {
            assert_ne!(said, format!("Claude changed {}", q("D")), "{operation}");
        }
    }
    // A long title does not push the button out of the note.
    let long = "A very long title ".repeat(8);
    let said = sentence("Claude", "edit", &named(&long));
    assert!(said.chars().count() < 80, "{said}");
    assert!(said.ends_with("\u{2026}\u{201d}"), "{said}");

    assert_eq!(
        run_sentence("Claude", 20, Some(&q("Chapter 2"))),
        format!("Claude made 20 changes to {}", q("Chapter 2"))
    );
    assert_eq!(
        run_sentence("An agent", 3, None),
        "An agent made 3 changes in the library"
    );
}

#[test]
fn a_script_waits_for_agents_and_sets_the_switch() {
    use writer_app::automation::{Script, Step};
    let script = Script::parse("agents:2; agents:1:500; agents:off; agents:on").unwrap();
    assert_eq!(
        script.steps,
        [
            Step::Agents(2, Duration::from_secs(10)),
            Step::Agents(1, Duration::from_millis(500)),
            Step::AgentsSwitch(false),
            Step::AgentsSwitch(true),
        ]
    );
    assert!(Script::parse("agents").is_err());
    assert!(Script::parse("agents:many").is_err());
    assert!(Script::parse("agents:1:soon").is_err());
}

// ----- the view, Vim, an app without a library -----------------------------------------

#[gpui::test]
fn the_view_stays_on_its_text_when_an_agent_writes_above_it(cx: &mut TestAppContext) {
    let (dir, lock, library) = library_on_disk(cx);
    let long = Document::from_blocks(
        std::iter::once(Block::with_text(BlockKind::heading(1), "Long"))
            .chain((1..400).map(|ix| Block::paragraph(&format!("Paragraph number {ix}."))))
            .collect(),
    );
    let id = add_document(&library, None, &long, cx);
    let (desk, cx) = open_desk(dir, lock, library, cx);
    // The writer is somewhere in the middle.
    set_caret(&desk, 250, 4, cx);
    let editor = desk
        .workspace
        .read_with(cx, |workspace, _| workspace.editor().clone());
    let top = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |editor, _| {
            let view = editor.view_state();
            (view.scroll_block, view.scroll_offset)
        })
    };
    let (block, offset) = top(cx);
    assert!(block > 100, "the view followed the caret: {block}");

    let tool = desk.tool();
    ask(cx, move || {
        tool.insert(&InsertRequest {
            document: id.into(),
            markdown: "One more at the top.\n\nAnd another.".into(),
            position: agent::InsertPosition::Start,
            write: claude(),
            ..InsertRequest::default()
        })
    })
    .unwrap();

    // The same paragraph is at the top of the view, and the caret is in
    // the paragraph it was in: both moved down by the two new ones.
    assert_eq!(top(cx), (block + 2, offset));
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().block_count(), 402);
        assert_eq!(editor.selection(), Selection::caret(Position::new(252, 4)));
        assert_eq!(
            editor.document().blocks()[252].text(),
            "Paragraph number 250."
        );
    });
    // "Go to the last change by an agent" is the way there.
    desk.workspace.update_in(cx, |workspace, window, cx| {
        workspace.show_agent_change(window, cx)
    });
    assert_eq!(selection(&desk, cx), Selection::caret(Position::new(0, 0)));
}

#[gpui::test]
fn with_vim_mode_on_the_change_is_one_undo_in_normal_mode(cx: &mut TestAppContext) {
    let (desk, id, cx) = desk_with_notes(cx);
    desk.workspace
        .update(cx, |workspace, cx| workspace.toggle_vim_mode(cx));
    cx.run_until_parked();
    // The writer is in the middle of an insert: "i", some text, no escape.
    cx.simulate_keystrokes("i");
    cx.simulate_input("Mine. ");
    let mode = |desk: &Desk, cx: &mut VisualTestContext| {
        desk.workspace.read_with(cx, |workspace, cx| {
            workspace.vim().read(cx).status().mode_label().to_string()
        })
    };
    let inserting = mode(&desk, cx);

    let tool = desk.tool();
    ask(cx, move || tool.edit(&edit(id, "is short", "is long"))).unwrap();
    assert_eq!(mode(&desk, cx), inserting, "the mode is the writer's");
    assert_eq!(
        editor_text(&desk.workspace, cx),
        "Mine. Notes\nThe first paragraph is long.\nThe second paragraph is the last."
    );
    // The insert goes on where it was.
    cx.simulate_input("Still mine. ");
    assert!(editor_text(&desk.workspace, cx).starts_with("Mine. Still mine. Notes"));
    // Back in Normal mode, "u" takes steps out one by one: the agent's
    // change is one of them and never half of the writer's insert.
    cx.simulate_keystrokes("escape u");
    let after_one = editor_text(&desk.workspace, cx);
    cx.simulate_keystrokes("u");
    let after_two = editor_text(&desk.workspace, cx);
    cx.simulate_keystrokes("u");
    let after_three = editor_text(&desk.workspace, cx);
    assert_eq!(
        after_three,
        notes().plain_text(),
        "{after_one:?} {after_two:?}"
    );
    assert!(
        after_one.contains("is long") && after_one.starts_with("Mine. Notes"),
        "{after_one:?}"
    );
    assert!(
        after_two.contains("is short") && after_two.starts_with("Mine. Notes"),
        "{after_two:?}"
    );
}

#[gpui::test]
fn an_app_without_a_library_tells_the_tool_so(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().unwrap();
    let _lock = match single_instance::acquire(dir.path()).unwrap() {
        Acquired::Lock(lock) => lock,
        Acquired::Taken(copy) => panic!("taken: {copy:?}"),
    };
    // The app holds the lock and shows its "cannot open the library"
    // window: no workspace, and a channel that only says why.
    let socket = cx
        .update(|cx| live::open_without_library(dir.path(), cx))
        .unwrap();
    assert!(socket.exists());
    assert!(!cx.update(|cx| live::is_open(cx)), "nothing takes changes");

    let tool = agent::Library::at(dir.path());
    let refused = std::thread::spawn(move || {
        tool.create_document(&CreateDocumentRequest {
            markdown: "Text.".into(),
            write: claude(),
            ..CreateDocumentRequest::default()
        })
    })
    .join()
    .unwrap()
    .unwrap_err();
    assert_eq!(refused.exit_code(), 5);
    assert!(
        matches!(
            &refused,
            agent::Error::AppIsOpen {
                why: Refusal::NoLibrary,
                ..
            }
        ),
        "{refused:?}"
    );
    assert!(
        refused.to_string().contains("no window with this library"),
        "{refused}"
    );

    cx.update(live::close);
    assert!(!socket.exists());
}

#[gpui::test]
fn what_was_noted_about_a_text_does_not_hold_for_the_next_one(cx: &mut TestAppContext) {
    let (desk, open, other, cx) = desk_with_two(cx);
    // An agent changes the open document: the note offers Undo, and the
    // editor holds what the library holds.
    let tool = desk.tool();
    ask(cx, move || tool.edit(&edit(open, "is short", "is long"))).unwrap();
    assert_eq!(
        desk.workspace
            .read_with(cx, |workspace, cx| workspace.agent_change_offer(cx)),
        Some(Offer::Undo)
    );

    // The writer opens another document. The note is about a document
    // that is not the open one now: it offers to open it.
    desk.workspace.update_in(cx, |workspace, window, cx| {
        assert!(workspace.open_document(other, true, window, cx));
    });
    cx.run_until_parked();
    assert_eq!(
        notes_shown(&desk, cx),
        [(
            "Claude changed \u{201c}Notes\u{201d}".to_string(),
            Some("Open".to_string())
        )]
    );
    // Every edit in the other document is the writer's and is saved, also
    // the one that brings its text to the revision the first one was at.
    for round in 0..6 {
        type_text(&desk.workspace, &format!("{round} "), cx);
        assert_eq!(
            save_state(&desk, cx),
            Some(SaveState::Edited),
            "edit {round}"
        );
        desk.workspace.update(cx, |workspace, cx| {
            assert!(workspace.flush(cx));
        });
        assert_eq!(save_state(&desk, cx), Some(SaveState::Saved));
        // And "undo the last change by an agent" takes none of them out.
        let undone = desk
            .workspace
            .update(cx, |workspace, cx| workspace.undo_agent_change(cx));
        assert!(!undone, "edit {round}");
    }
    assert_eq!(stored(&desk, other, cx), "0 1 2 3 4 5 Other\nText.");
    assert_eq!(editor_text(&desk.workspace, cx), "0 1 2 3 4 5 Other\nText.");
    // The agent's change is where it was made.
    assert!(stored(&desk, open, cx).contains("is long"));
}

#[gpui::test]
fn the_version_history_of_a_document_an_agent_trashes_closes(cx: &mut TestAppContext) {
    let (desk, open, other, cx) = desk_with_two(cx);
    desk.workspace.update_in(cx, |workspace, window, cx| {
        workspace.toggle_version_history(window, cx);
    });
    cx.run_until_parked();
    assert!(has_modal(&desk, cx));

    let tool = desk.tool();
    ask(cx, move || {
        tool.trash_document(&TrashDocumentRequest {
            document: open.into(),
            write: claude(),
        })
    })
    .unwrap();
    // The document closed, and the list of its versions with it: nothing
    // is left on screen that is about a document that is not there.
    assert_eq!(open_id(&desk, cx), Some(other));
    assert!(!has_modal(&desk, cx));
    assert!(editor_focused(&desk.workspace, cx));
    assert_eq!(editor_text(&desk.workspace, cx), "Other\nText.");
}
