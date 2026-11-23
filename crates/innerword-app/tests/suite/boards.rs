//! Whiteboards in the assembled app (`docs/whiteboard-design.md`, sections 4
//! to 6), on the fake page engine: no test makes a `WKWebView`. The fake's
//! canvas is a scene held in Rust; its `adopt` applies the base rule.

use std::{rc::Rc, time::Duration};

use gpui::TestAppContext;
use innerword_app::{
    actions::{Export, ExportPdf, Print, ReloadPage},
    automation::describe_state,
    boards::FakeFailure,
    export::{
        ExportChoice, ExportDelegate, ExportDestination, ExportScope, ExportTarget, FileFormat,
    },
    pages::{self, FakePages},
    printing,
    workspace::{
        AddRectangle, AddToShapeLibrary, BOARD_SAVE_DELAY, End, InsertFromShapeLibrary,
        LIBRARY_TOO_LARGE, MermaidToDrawing, NO_COMMENTS_ON_A_BOARD, RELOADED, Recovered,
        ToggleShapeLibrary,
    },
};
use innerword_board::Scene;
use innerword_library::{DocumentKind, OutsideChange};
use innerword_ui::Picker;
use serde_json::{Value, json};

use super::*;

// ----- helpers --------------------------------------------------------------------------

/// A rectangle element as Excalidraw keeps it.
fn rect(id: &str, x: f64, version: i64) -> Value {
    json!({
        "id": id, "type": "rectangle", "x": x, "y": 0.0, "width": 100.0, "height": 50.0,
        "angle": 0, "strokeColor": "#1e1e1e", "backgroundColor": "transparent",
        "fillStyle": "solid", "strokeWidth": 2, "strokeStyle": "solid", "roughness": 1,
        "opacity": 100, "groupIds": [], "frameId": null, "roundness": null,
        "seed": 1, "version": version, "versionNonce": version * 7 + 1, "isDeleted": false,
        "boundElements": null, "updated": 1, "link": null, "locked": false,
        "index": format!("a{}", id.len()),
    })
}

/// A scene of `elements` as the page posts it.
fn scene(elements: &[Value]) -> String {
    json!({
        "type": "excalidraw", "version": 2, "source": "inner-word",
        "elements": elements, "appState": { "viewBackgroundColor": "#ffffff" }, "files": {},
    })
    .to_string()
}

/// The stored scene of whiteboard `id`.
fn stored(library: &Entity<Library>, id: DocumentId, cx: &mut VisualTestContext) -> Scene {
    let content = library.read_with(cx, |library, _| library.document(id).unwrap().content);
    Scene::from_stored(&content).expect("a stored scene").scene
}

fn ids(scene: &Scene) -> Vec<String> {
    scene
        .elements()
        .iter()
        .map(|e| e.id().to_string())
        .collect()
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

fn state_line(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> String {
    workspace.update_in(cx, |workspace, window, cx| {
        describe_state(workspace, window, cx)
    })
}

/// Lets the save timer and what follows run.
fn settle(cx: &mut VisualTestContext) {
    for _ in 0..6 {
        cx.executor().advance_clock(BOARD_SAVE_DELAY);
        cx.run_until_parked();
    }
}

/// A workspace on the fake engine with a text "Notes" and a whiteboard
/// "Plan" made by New Whiteboard, open and ready.
fn open_board(
    cx: &mut TestAppContext,
) -> (
    Rc<FakePages>,
    Entity<Library>,
    Entity<Workspace>,
    DocumentId,
    DocumentId,
    &mut VisualTestContext,
) {
    init(cx);
    let fake = cx.update(pages::use_fake);
    let library = library(cx);
    let text = add_document(&library, None, &simple_document("Notes", "Words."), cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    let board = workspace
        .update_in(vcx, |w, window, cx| w.new_board(Some("Plan"), window, cx))
        .expect("a whiteboard is made");
    vcx.run_until_parked();
    fake.board_ready();
    vcx.run_until_parked();
    (fake, library, workspace, board, text, vcx)
}

// ----- opening and saving ---------------------------------------------------------------

#[gpui::test]
fn new_whiteboard_makes_one_and_opens_it_in_the_view(cx: &mut TestAppContext) {
    let (fake, library, workspace, board, _, vcx) = open_board(cx);
    let meta = library.read_with(vcx, |l, _| l.document_meta(board).unwrap());
    assert_eq!(meta.kind, DocumentKind::Board);
    assert_eq!(meta.title, "Plan");
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.open_board_id()),
        Some(board)
    );
    let opened = fake.boards_opened();
    assert_eq!(opened.len(), 1);
    let served: Value = serde_json::from_str(&opened[0].1).unwrap();
    assert_eq!(served["stamp"].as_str().unwrap().len(), 32);
    assert!(served["scene"]["elements"].as_array().unwrap().is_empty());
    let line = state_line(&workspace, vcx);
    assert!(line.contains("board=[id="), "{line}");
    assert!(line.contains("state=loaded"), "{line}");
    assert!(line.contains("dirty=no"), "{line}");
    // The menu bar keeps the whiteboard's own keys out of it.
    assert_eq!(
        vcx.update(|_, cx| innerword_app::menus::open_item(cx)),
        innerword_app::menus::OpenItemKind::Board
    );
}

#[gpui::test]
fn a_gesture_is_saved_and_the_own_save_neither_adopts_nor_reloads(cx: &mut TestAppContext) {
    let (fake, library, workspace, board, _, vcx) = open_board(cx);
    fake.draw(&scene(&[rect("a", 0.0, 1)]));
    settle(vcx);
    assert_eq!(ids(&stored(&library, board, vcx)), ["a"]);
    assert!(
        fake.adopted().is_empty(),
        "its own save is no outside change"
    );
    assert_eq!(fake.reloads(), 0);
    assert_eq!(fake.boards_opened().len(), 1);
    let line = state_line(&workspace, vcx);
    assert!(
        line.contains("dirty=no") && line.contains("saved=1"),
        "{line}"
    );
    // A rename while saving keeps the new title.
    library.update(vcx, |l, cx| l.rename_document(board, "Plan B", cx).unwrap());
    fake.draw(&scene(&[rect("a", 10.0, 2)]));
    settle(vcx);
    let meta = library.read_with(vcx, |l, _| l.document_meta(board).unwrap());
    assert_eq!(meta.title, "Plan B");
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.page_title().map(str::to_string)),
        Some("Plan B".into())
    );
}

#[gpui::test]
fn switching_away_while_the_post_has_not_come_parks_the_view_and_keeps_the_stroke(
    cx: &mut TestAppContext,
) {
    let (fake, library, workspace, board, text, vcx) = open_board(cx);
    fake.hold_collects();
    fake.draw_unposted(&scene(&[rect("late", 0.0, 1)]));
    vcx.run_until_parked();
    workspace.update_in(vcx, |w, window, cx| {
        w.open_document(text, true, window, cx);
    });
    vcx.run_until_parked();
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.open_document_id()),
        Some(text)
    );
    assert!(fake.parked(), "the closing view is parked");
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.board_closing()),
        Some(board)
    );
    assert!(ids(&stored(&library, board, vcx)).is_empty(), "not yet");
    // Opening the same whiteboard meanwhile waits for its save.
    workspace.update_in(vcx, |w, window, cx| {
        w.open_document(board, true, window, cx);
    });
    vcx.run_until_parked();
    assert_eq!(fake.boards_opened().len(), 1, "no second view yet");
    fake.release_collects();
    vcx.run_until_parked();
    assert_eq!(ids(&stored(&library, board, vcx)), ["late"]);
    assert_eq!(workspace.read_with(vcx, |w, _| w.board_closing()), None);
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.open_board_id()),
        Some(board)
    );
    assert_eq!(fake.boards_opened().len(), 2, "opened again once saved");
}

#[gpui::test]
fn closing_the_window_saves_what_the_page_holds(cx: &mut TestAppContext) {
    let (fake, library, _workspace, board, _, vcx) = open_board(cx);
    fake.draw_unposted(&scene(&[rect("unposted", 0.0, 1)]));
    vcx.run_until_parked();
    vcx.simulate_keystrokes("cmd-w");
    vcx.run_until_parked();
    assert_eq!(ids(&stored(&library, board, vcx)), ["unposted"]);
    assert!(fake.collects_asked() >= 1);
}

#[gpui::test]
fn the_close_button_waits_for_the_page_then_closes(cx: &mut TestAppContext) {
    let (fake, library, workspace, board, _, vcx) = open_board(cx);
    fake.draw_unposted(&scene(&[rect("unposted", 0.0, 1)]));
    vcx.run_until_parked();
    assert!(
        !vcx.simulate_close(),
        "the window stays while the page holds more"
    );
    vcx.run_until_parked();
    assert_eq!(ids(&stored(&library, board, vcx)), ["unposted"]);
    let _ = workspace;
}

#[gpui::test]
fn quitting_through_the_request_saves_what_the_page_holds(cx: &mut TestAppContext) {
    let (fake, library, workspace, board, _, vcx) = open_board(cx);
    fake.draw_unposted(&scene(&[rect("unposted", 0.0, 1)]));
    vcx.run_until_parked();
    // As the signal task and the Quit listener call it.
    workspace.update_in(vcx, |w, window, cx| w.end_then(End::Quit, window, cx));
    vcx.run_until_parked();
    assert_eq!(ids(&stored(&library, board, vcx)), ["unposted"]);
}

#[gpui::test]
fn the_quit_hook_alone_saves_what_the_app_holds(cx: &mut TestAppContext) {
    let (fake, library, _workspace, board, _, vcx) = open_board(cx);
    fake.draw(&scene(&[rect("posted", 0.0, 1)]));
    // No timer: the quit hook's synchronous flush writes it.
    vcx.run_until_parked();
    cx.quit();
    cx.run_until_parked();
    let content = library.read_with(cx, |l, _| l.document(board).unwrap().content);
    let scene = Scene::from_stored(&content).unwrap().scene;
    assert_eq!(ids(&scene), ["posted"]);
}

// ----- the compare-and-swap -------------------------------------------------------------

/// An outside write (as an agent's request makes it), straight into the
/// store.
fn outside_write(
    library: &Entity<Library>,
    board: DocumentId,
    elements: &[Value],
    cx: &mut VisualTestContext,
) {
    let scene = Scene::from_stored(
        &json!({
            "type": "excalidraw", "version": 2, "source": "inner-word",
            "elements": elements, "appState": {}, "files": {},
        })
        .to_string(),
    )
    .unwrap()
    .scene;
    let content = scene.to_stored();
    let text = scene.text();
    library.update(cx, |l, cx| {
        l.change_outside(
            |store| {
                let saved = store.save_board(
                    board,
                    &innerword_library::store::BoardContent {
                        scene: &content,
                        files: &[],
                        plain_text: &text,
                        word_count: 0,
                        char_count: 0,
                    },
                    None,
                );
                let changes = vec![OutsideChange::DocumentSaved(board)];
                (saved.unwrap(), changes)
            },
            cx,
        )
        .1
        .unwrap();
    });
}

#[gpui::test]
fn a_save_that_finds_the_row_changed_adopts_and_keeps_both(cx: &mut TestAppContext) {
    let (fake, library, _workspace, board, _, vcx) = open_board(cx);
    fake.draw(&scene(&[rect("mine", 0.0, 1)]));
    settle(vcx);
    outside_write(
        &library,
        board,
        &[rect("mine", 0.0, 1), rect("theirs", 300.0, 1)],
        vcx,
    );
    vcx.run_until_parked();
    // The writer draws on: the save is stale, the stored scene is adopted by
    // the base rule, and the merge the page posts is saved.
    fake.draw(&scene(&[rect("mine", 50.0, 2)]));
    settle(vcx);
    assert_eq!(fake.adopted().len(), 1);
    let saved = stored(&library, board, vcx);
    let mut kept = ids(&saved);
    kept.sort();
    assert_eq!(kept, ["mine", "theirs"]);
    assert_eq!(
        saved.element("mine").unwrap().x(),
        50.0,
        "the writer's move stays"
    );
}

#[gpui::test]
fn a_post_from_before_an_adopt_is_not_saved_over_it(cx: &mut TestAppContext) {
    let (fake, library, workspace, board, _, vcx) = open_board(cx);
    fake.draw(&scene(&[rect("mine", 0.0, 1)]));
    settle(vcx);
    let content = library.read_with(vcx, |l, _| l.document(board).unwrap().content);
    outside_write(
        &library,
        board,
        &[rect("mine", 0.0, 1), rect("theirs", 300.0, 1)],
        vcx,
    );
    let after = library.read_with(vcx, |l, _| l.document(board).unwrap().content);
    assert_ne!(content, after);
    workspace.update(vcx, |w, cx| w.adopt_outside_write(board, cx));
    // A post that left the page before the adopt: dropped.
    fake.post_stale(&scene(&[rect("mine", 0.0, 1)]), 1);
    settle(vcx);
    let mut kept = ids(&stored(&library, board, vcx));
    kept.sort();
    assert_eq!(kept, ["mine", "theirs"], "the outside write stays");
}

#[gpui::test]
fn a_failed_adopt_merges_in_rust_saves_and_reloads(cx: &mut TestAppContext) {
    let (fake, library, _workspace, board, _, vcx) = open_board(cx);
    fake.draw(&scene(&[rect("mine", 0.0, 1)]));
    settle(vcx);
    outside_write(
        &library,
        board,
        &[rect("mine", 0.0, 1), rect("theirs", 300.0, 1)],
        vcx,
    );
    fake.fail_adopts(FakeFailure::Throws);
    fake.draw(&scene(&[rect("mine", 0.0, 1), rect("new", 600.0, 1)]));
    settle(vcx);
    assert_eq!(fake.reloads(), 1, "the view loads the merge");
    let mut kept = ids(&stored(&library, board, vcx));
    kept.sort();
    assert_eq!(kept, ["mine", "new", "theirs"]);
}

// ----- pictures -------------------------------------------------------------------------

#[gpui::test]
fn a_pasted_picture_is_stored_before_the_save_that_names_it(cx: &mut TestAppContext) {
    let (fake, library, _workspace, board, _, vcx) = open_board(cx);
    let png = innerword_image::fixtures::png(40, 30, [10, 120, 200]);
    let file = innerword_library::store::FileId::of(&png).to_hex();
    fake.paste_picture("pasted", "image/png", &png);
    let mut picture = rect("pic", 0.0, 1);
    picture["type"] = json!("image");
    picture["fileId"] = json!("pasted");
    fake.draw(&scene(&[picture]));
    settle(vcx);
    let saved = stored(&library, board, vcx);
    let entry = saved.files().get("pasted").expect("a files entry");
    assert_eq!(
        entry.file.as_ref().map(|f| f.as_str().to_string()),
        Some(file.clone())
    );
    assert_eq!(entry.mime_type, "image/png");
    let id = innerword_library::store::FileId::parse(&file).unwrap();
    assert!(library.read_with(vcx, |l, _| l.store().file_info(&id).unwrap().is_some()));
}

#[gpui::test]
fn a_scene_naming_a_picture_whose_file_never_came_asks_for_it(cx: &mut TestAppContext) {
    let (fake, library, _workspace, board, _, vcx) = open_board(cx);
    fake.forget_file_messages();
    let png = innerword_image::fixtures::png(20, 20, [200, 10, 10]);
    fake.paste_picture("late", "image/png", &png);
    let mut picture = rect("pic", 0.0, 1);
    picture["type"] = json!("image");
    picture["fileId"] = json!("late");
    fake.draw(&scene(&[picture]));
    settle(vcx);
    assert!(fake.board_calls().iter().any(|call| call == "files late"));
    let saved = stored(&library, board, vcx);
    assert!(
        saved
            .files()
            .get("late")
            .and_then(|e| e.file.clone())
            .is_some()
    );
}

#[gpui::test]
fn an_html_file_posted_as_an_svg_is_refused(cx: &mut TestAppContext) {
    let (fake, library, workspace, board, _, vcx) = open_board(cx);
    fake.paste_picture(
        "bad",
        "image/svg+xml",
        b"<!doctype html><html><body>no</body></html>",
    );
    let mut picture = rect("pic", 0.0, 1);
    picture["type"] = json!("image");
    picture["fileId"] = json!("bad");
    fake.draw(&scene(&[picture]));
    settle(vcx);
    assert!(
        !toasts(&workspace, vcx).is_empty(),
        "a toast names the refusal"
    );
    let saved = stored(&library, board, vcx);
    assert_eq!(ids(&saved), ["pic"], "the element stays");
    assert!(
        saved.files().get("bad").is_none(),
        "no entry for what was refused"
    );
}

#[gpui::test]
fn a_scene_that_cannot_be_read_goes_to_the_recovery_copy(cx: &mut TestAppContext) {
    let (fake, _library, workspace, _board, _, vcx) = open_board(cx);
    fake.post_scene_raw(r#"{"elements": 5}"#, 1);
    settle(vcx);
    let recovered = workspace.read_with(vcx, |w, _| w.board_recovered());
    assert!(
        matches!(recovered, Some(Recovered::Memory(_))),
        "{recovered:?}"
    );
    assert!(state_line(&workspace, vcx).contains("recovered=yes"));
}

// ----- commands ---------------------------------------------------------------------------

#[gpui::test]
fn reload_whiteboard_collects_saves_and_loads_again(cx: &mut TestAppContext) {
    let (fake, library, workspace, board, _, vcx) = open_board(cx);
    fake.draw_unposted(&scene(&[rect("a", 0.0, 1)]));
    vcx.run_until_parked();
    vcx.update(|window, cx| window.dispatch_action(Box::new(ReloadPage), cx));
    vcx.run_until_parked();
    assert_eq!(fake.reloads(), 1);
    assert_eq!(ids(&stored(&library, board, vcx)), ["a"]);
    assert!(toasts(&workspace, vcx).iter().any(|t| t == RELOADED));
}

#[gpui::test]
fn the_edit_menu_and_the_add_commands_reach_the_page(cx: &mut TestAppContext) {
    let (fake, _library, workspace, _board, _, vcx) = open_board(cx);
    let focus = workspace.read_with(vcx, |w, _| w.page_focus_handle().unwrap());
    vcx.update(|window, _| window.focus(&focus));
    for action in [
        Box::new(innerword_editor::actions::Undo) as Box<dyn gpui::Action>,
        Box::new(innerword_editor::actions::Redo),
        Box::new(innerword_editor::actions::SelectAll),
        Box::new(innerword_editor::actions::Copy),
    ] {
        vcx.update(|window, cx| window.dispatch_action(action, cx));
        vcx.run_until_parked();
    }
    let calls = fake.board_calls();
    assert!(calls.contains(&"undo 1".to_string()), "{calls:?}");
    assert!(calls.contains(&"redo 1".to_string()), "{calls:?}");
    assert!(calls.contains(&"selectAll".to_string()), "{calls:?}");
    assert!(fake.edits().contains(&"copy"), "copy goes to the view");
    vcx.update(|window, cx| window.dispatch_action(Box::new(AddRectangle), cx));
    vcx.run_until_parked();
    let calls = fake.board_calls();
    assert!(calls.contains(&"center".to_string()), "{calls:?}");
    assert!(
        calls.iter().any(|call| call == "insertElements 1"),
        "{calls:?}"
    );
    let canvas: Value = serde_json::from_str(&fake.canvas()).unwrap();
    assert_eq!(canvas["elements"][0]["type"], "rectangle");
    for action in [
        Box::new(MermaidToDrawing) as Box<dyn gpui::Action>,
        Box::new(ToggleShapeLibrary),
        Box::new(AddToShapeLibrary),
    ] {
        vcx.update(|window, cx| window.dispatch_action(action, cx));
        vcx.run_until_parked();
    }
    let calls = fake.board_calls();
    assert!(calls.contains(&"mermaid".to_string()), "{calls:?}");
    assert!(calls.contains(&"library toggle".to_string()), "{calls:?}");
    assert!(
        calls.contains(&"library addSelection".to_string()),
        "{calls:?}"
    );
}

#[gpui::test]
fn commands_of_a_text_say_what_a_whiteboard_is(cx: &mut TestAppContext) {
    let (_fake, _library, workspace, _board, _, vcx) = open_board(cx);
    vcx.update(|window, cx| {
        window.dispatch_action(Box::new(innerword_comments::ToggleComments), cx)
    });
    vcx.run_until_parked();
    assert!(toasts(&workspace, vcx).contains(&NO_COMMENTS_ON_A_BOARD.to_string()));
    assert!(
        !workspace.read_with(vcx, |w, _| w.has_right_panel()),
        "no comments panel opens"
    );
}

#[gpui::test]
fn the_whiteboards_own_keys_are_left_to_the_page(cx: &mut TestAppContext) {
    let (fake, _library, _workspace, _board, _, vcx) = open_board(cx);
    vcx.run_until_parked();
    for key in [
        "cmd-g",
        "cmd-alt-c",
        "cmd-alt-[",
        "cmd-d",
        "cmd-shift-,",
        "cmd-<",
    ] {
        assert!(!fake.press(key), "{key} is the whiteboard's");
    }
    for key in ["ctrl-enter", "shift-f10", "cmd-shift-p", "cmd-f", "cmd-s"] {
        assert!(fake.press(key), "{key} is the window's");
    }
}

#[gpui::test]
fn help_from_the_page_opens_the_list_at_on_a_whiteboard(cx: &mut TestAppContext) {
    let (fake, _library, workspace, _board, _, vcx) = open_board(cx);
    fake.press_help();
    vcx.run_until_parked();
    let list = workspace
        .read_with(vcx, |w, cx| w.shortcuts_list(cx))
        .expect("the list is open");
    assert_eq!(
        list.read_with(vcx, |list, cx| list.query(cx)),
        "On a whiteboard"
    );
}

// ----- PDF, print, export -----------------------------------------------------------------

#[gpui::test]
fn the_export_picker_of_a_whiteboard_has_one_row(cx: &mut TestAppContext) {
    let (_fake, _library, workspace, _board, _, vcx) = open_board(cx);
    vcx.update(|window, cx| window.dispatch_action(Box::new(Export), cx));
    vcx.run_until_parked();
    let choices = workspace.read_with(vcx, |w, cx| {
        w.modal_layer()
            .read(cx)
            .active::<Picker<ExportDelegate>>()
            .map(|picker| picker.read(cx).delegate.choices())
    });
    assert_eq!(
        choices,
        Some(vec![ExportChoice {
            scope: ExportScope::Document,
            format: FileFormat::Pdf,
            destination: ExportDestination::File,
        }])
    );
}

#[gpui::test]
fn export_as_pdf_writes_the_page_of_the_drawing(cx: &mut TestAppContext) {
    let (fake, _library, workspace, _board, _, vcx) = open_board(cx);
    fake.draw(&scene(&[rect("a", 0.0, 1), rect("b", 300.0, 1)]));
    settle(vcx);
    let out = tempfile::tempdir().unwrap();
    let chosen = out.path().join("Plan.pdf");
    vcx.update(|window, cx| window.dispatch_action(Box::new(ExportPdf), cx));
    vcx.run_until_parked();
    assert!(vcx.did_prompt_for_new_path(), "the file is chosen first");
    vcx.simulate_new_path_selection({
        let chosen = chosen.clone();
        move |_| Some(chosen)
    });
    for _ in 0..200 {
        vcx.run_until_parked();
        if workspace.read_with(vcx, |w, _| w.pdf_exporting()).is_none() && chosen.exists() {
            break;
        }
        vcx.executor().advance_clock(Duration::from_millis(5));
    }
    let bytes = std::fs::read(&chosen).expect("the PDF is written");
    let pdf = innerword_print::PdfFile::open(&bytes).expect("a PDF");
    assert_eq!(pdf.page_count(), 1);
    let size = pdf.page_size(0);
    // The fake's page: the drawing (400 x 50) and a 32 pt margin.
    assert_eq!((size.0.round(), size.1.round()), (464.0, 114.0));
    let report = workspace.update(vcx, |w, _| w.take_pdf_export_report());
    assert!(matches!(report, Some(Ok(_))), "{report:?}");
}

#[gpui::test]
fn print_puts_the_drawing_on_the_users_paper(cx: &mut TestAppContext) {
    let (fake, _library, workspace, _board, _, vcx) = open_board(cx);
    let jobs = vcx.update(|_, cx| printing::record(cx));
    fake.draw(&scene(&[rect("a", 0.0, 1)]));
    settle(vcx);
    vcx.update(|window, cx| window.dispatch_action(Box::new(Print), cx));
    for _ in 0..100 {
        vcx.run_until_parked();
        if !jobs.borrow().is_empty() {
            break;
        }
        vcx.executor().advance_clock(Duration::from_millis(5));
    }
    let jobs = jobs.borrow();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].fit, printing::Fit::ShrinkOntoPaper);
    assert_eq!(jobs[0].pdf.report.pages, 1);
    let _ = workspace;
}

#[gpui::test]
fn an_empty_whiteboard_has_nothing_to_print(cx: &mut TestAppContext) {
    let (_fake, _library, workspace, _board, _, vcx) = open_board(cx);
    let jobs = vcx.update(|_, cx| printing::record(cx));
    // The window is drawn again with Print's handler (a target now).
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    vcx.update(|window, cx| window.dispatch_action(Box::new(Print), cx));
    for _ in 0..20 {
        vcx.executor().advance_clock(Duration::from_millis(5));
        vcx.run_until_parked();
    }
    assert!(jobs.borrow().is_empty());
    assert!(
        toasts(&workspace, vcx)
            .contains(&"\u{201c}Plan\u{201d} is empty: there is nothing to print.".to_string()),
        "{:?}",
        toasts(&workspace, vcx)
    );
}

#[gpui::test]
fn a_folders_pdf_holds_its_whiteboards_after_its_documents(cx: &mut TestAppContext) {
    init(cx);
    let fake = cx.update(pages::use_fake);
    let library = library(cx);
    let folder = library.update(cx, |l, cx| {
        l.create_folder(None, "Part one", cx).unwrap().id
    });
    let text = add_document(
        &library,
        Some(folder),
        &simple_document("Chapter", "Words."),
        cx,
    );
    let (workspace, vcx) = open_workspace(&library, cx);
    workspace.update_in(vcx, |w, window, cx| {
        w.open_document(text, false, window, cx);
    });
    vcx.run_until_parked();
    let make = |title: &str, vcx: &mut VisualTestContext| {
        workspace
            .update_in(vcx, |w, window, cx| w.new_board(Some(title), window, cx))
            .unwrap()
    };
    let plan = make("Drawn", vcx);
    vcx.run_until_parked();
    fake.board_ready();
    fake.draw(&scene(&[rect("a", 0.0, 1)]));
    settle(vcx);
    let _empty = make("Bare", vcx);
    vcx.run_until_parked();
    fake.board_ready();
    vcx.run_until_parked();
    for id in [plan] {
        let folder_of = library.read_with(vcx, |l, _| l.document_meta(id).unwrap().folder_id);
        assert_eq!(
            folder_of,
            Some(folder),
            "made in the folder of the selection"
        );
    }
    let out = tempfile::tempdir().unwrap();
    let chosen = out.path().join("Part one.pdf");
    workspace.update_in(vcx, |w, window, cx| {
        w.export_pdf(
            ExportTarget::Folder(folder),
            ExportScope::Folder,
            window,
            cx,
        )
    });
    vcx.run_until_parked();
    vcx.simulate_new_path_selection({
        let chosen = chosen.clone();
        move |_| Some(chosen)
    });
    for _ in 0..400 {
        vcx.run_until_parked();
        if workspace.read_with(vcx, |w, _| w.pdf_exporting()).is_none() && chosen.exists() {
            break;
        }
        vcx.executor().advance_clock(Duration::from_millis(5));
    }
    let bytes = std::fs::read(&chosen).expect("the PDF is written");
    let pdf = innerword_print::PdfFile::open(&bytes).unwrap();
    assert_eq!(
        pdf.page_count(),
        2,
        "the chapter's page, then the whiteboard's"
    );
    let last = pdf.page_size(1);
    assert_eq!((last.0.round(), last.1.round()), (164.0, 114.0));
    let said = toasts(&workspace, vcx);
    assert!(
        said.iter()
            .any(|t| t.contains("\u{201c}Bare\u{201d} is empty and was left out.")),
        "{said:?}"
    );
}

#[gpui::test]
fn a_folders_markdown_leaves_whiteboards_out_and_says_so(cx: &mut TestAppContext) {
    init(cx);
    let _fake = cx.update(pages::use_fake);
    let library = library(cx);
    let folder = library.update(cx, |l, cx| {
        l.create_folder(None, "Part one", cx).unwrap().id
    });
    let text = add_document(
        &library,
        Some(folder),
        &simple_document("Chapter", "Words."),
        cx,
    );
    let (workspace, vcx) = open_workspace(&library, cx);
    workspace.update_in(vcx, |w, window, cx| {
        w.open_document(text, false, window, cx);
        w.new_board(Some("Drawn"), window, cx);
        w.open_document(text, false, window, cx);
    });
    vcx.run_until_parked();
    let prepared = workspace
        .update(vcx, |w, cx| {
            w.prepare_export_of(
                ExportTarget::Folder(folder),
                ExportScope::Folder,
                innerword_export::ExportFormat::Markdown,
                cx,
            )
        })
        .expect("the folder's Markdown");
    assert_eq!(prepared.documents, 1);
    assert_eq!(prepared.boards_left_out, 1);
    assert_eq!(
        innerword_app::export::left_out_sentence(0, 1),
        "1 whiteboard was left out: a whiteboard is exported as a PDF, on its own or in the folder's PDF."
    );
}

// ----- the shape library ------------------------------------------------------------------

#[gpui::test]
fn the_shape_library_is_stored_and_served_to_the_next_whiteboard(cx: &mut TestAppContext) {
    let (fake, library, workspace, _board, _, vcx) = open_board(cx);
    let items = json!([{ "id": "item1", "status": "unpublished", "created": 5,
        "elements": [rect("s", 0.0, 1)] }]);
    fake.change_library(&items.to_string());
    vcx.run_until_parked();
    let stored: Option<Box<serde_json::value::RawValue>> = library.read_with(vcx, |l, _| {
        l.setting(innerword_app::boards::SHAPE_LIBRARY_SETTING)
            .unwrap()
    });
    let stored = stored.expect("the setting is there");
    let parsed = innerword_board::ShapeLibrary::from_stored(stored.get()).unwrap();
    assert_eq!(parsed.library.items().len(), 1);
    // The next whiteboard is served it.
    workspace.update_in(vcx, |w, window, cx| w.new_board(Some("Second"), window, cx));
    vcx.run_until_parked();
    let served = fake.library_served();
    assert!(served.last().unwrap().contains("item1"), "{served:?}");
    // The picker lists the held items.
    vcx.update(|window, cx| window.dispatch_action(Box::new(InsertFromShapeLibrary), cx));
    vcx.run_until_parked();
    assert!(workspace.read_with(vcx, |w, cx| w.modal_layer().read(cx).has_modal()));
}

#[gpui::test]
fn a_shape_library_over_the_limits_is_not_stored(cx: &mut TestAppContext) {
    let (fake, library, workspace, _board, _, vcx) = open_board(cx);
    let many: Vec<Value> = (0..1001)
        .map(|n| {
            json!({ "id": format!("i{n}"), "status": "unpublished", "created": n,
            "elements": [rect(&format!("s{n}"), 0.0, 1)] })
        })
        .collect();
    fake.change_library(&Value::Array(many).to_string());
    vcx.run_until_parked();
    let stored: Option<Box<serde_json::value::RawValue>> = library.read_with(vcx, |l, _| {
        l.setting(innerword_app::boards::SHAPE_LIBRARY_SETTING)
            .unwrap()
    });
    assert!(stored.is_none(), "nothing stored");
    assert!(toasts(&workspace, vcx).contains(&LIBRARY_TOO_LARGE.to_string()));
}

// ----- the version history ----------------------------------------------------------------

#[gpui::test]
fn restoring_a_version_adopts_it_into_the_view(cx: &mut TestAppContext) {
    let (fake, library, workspace, board, _, vcx) = open_board(cx);
    fake.draw(&scene(&[rect("first", 0.0, 1)]));
    settle(vcx);
    library.update(vcx, |l, cx| l.create_snapshot(board, cx).unwrap());
    let snapshot = library.read_with(vcx, |l, _| l.snapshots(board).unwrap()[0].id);
    fake.draw(&scene(&[rect("first", 0.0, 1), rect("second", 200.0, 1)]));
    settle(vcx);
    workspace.update_in(vcx, |w, window, cx| {
        assert!(w.restore_version(snapshot, window, cx));
    });
    settle(vcx);
    assert_eq!(
        fake.adopted().len(),
        1,
        "taken into the view, one step of its undo"
    );
    let delta: Value = serde_json::from_str(&fake.adopted()[0]).unwrap();
    assert_eq!(delta["undoable"], true);
    assert_eq!(ids(&stored(&library, board, vcx)), ["first"]);
}

// ----- no engine ----------------------------------------------------------------------------

#[gpui::test]
fn without_an_engine_a_whiteboard_shows_its_listing_and_none_is_made(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    assert_eq!(
        workspace.update_in(vcx, |w, window, cx| w.new_board(Some("Plan"), window, cx)),
        None
    );
    vcx.run_until_parked();
    assert!(
        !vcx.update(|window, cx| window.is_action_available(&innerword_app::actions::NewBoard, cx))
    );
}

// ----- menus, the status bar, the list of shortcuts ---------------------------------------

#[gpui::test]
fn the_title_menu_and_ctrl_enter_have_the_whiteboards_items(cx: &mut TestAppContext) {
    let (_fake, _library, workspace, _board, _, vcx) = open_board(cx);
    let title = workspace.update(vcx, |w, cx| {
        w.menu_items(innerword_app::workspace::MenuKind::Title, cx)
    });
    assert!(!title.is_empty());
    let focus = workspace.read_with(vcx, |w, _| w.page_focus_handle().unwrap());
    vcx.update(|window, _| window.focus(&focus));
    vcx.simulate_keystrokes("ctrl-enter");
    vcx.run_until_parked();
    let labels = vcx.update(|window, cx| {
        innerword_ui::ContextMenu::current(window, cx)
            .map(|menu| menu.read(cx).labels())
            .unwrap_or_default()
    });
    for wanted in [
        "Undo",
        "Redo",
        "Select All",
        "Add Rectangle",
        "Add Text",
        "Version History…",
    ] {
        assert!(
            labels.iter().any(|label| label == wanted),
            "{wanted}: {labels:?}"
        );
    }
}

#[gpui::test]
fn the_status_bar_says_the_save_state_and_the_elements(cx: &mut TestAppContext) {
    let (fake, _library, workspace, _board, _, vcx) = open_board(cx);
    let mut status = fake.board_status();
    status.elements = 14;
    fake.set_status(status);
    vcx.run_until_parked();
    let line = workspace.update(vcx, |w, cx| w.status_line(cx));
    let _ = line;
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.board_status().map(|s| s.elements)),
        Some(14)
    );
    let labels = workspace.update(vcx, |w, cx| {
        w.menu_items(innerword_app::workspace::MenuKind::Save, cx)
            .len()
    });
    assert_eq!(labels, 1, "Save Now");
}

#[gpui::test]
fn the_list_has_the_whiteboards_place_and_its_fixed_rows(cx: &mut TestAppContext) {
    let (_fake, _library, _workspace, _board, _, vcx) = open_board(cx);
    vcx.update(|_, cx| {
        let rows = innerword_app::shortcuts::collect_shortcuts(cx);
        let on_a_board: Vec<&innerword_app::shortcuts::Shortcut> = rows
            .iter()
            .filter(|row| row.place == innerword_app::shortcuts::Place::Board)
            .collect();
        let titled = |title: &str| on_a_board.iter().any(|row| row.title.as_ref() == title);
        for title in [
            "Show the whiteboard's context menu",
            "Find",
            "Add rectangle",
            "Mermaid to drawing…",
            "Insert from shape library…",
            "Duplicate",
            "Group",
            innerword_board_web::keys::TOOL_KEYS_ROW,
        ] {
            assert!(titled(title), "{title} is a row On a whiteboard");
        }
        let duplicate = on_a_board
            .iter()
            .find(|row| row.title.as_ref() == "Duplicate")
            .unwrap();
        assert!(duplicate.fixed.is_some(), "a fixed row");
        assert!(duplicate.target.is_none(), "no command");
        assert_eq!(duplicate.keys, "⌘D");
    });
}

#[gpui::test]
fn none_of_the_whiteboards_keys_is_a_key_the_window_keeps(cx: &mut TestAppContext) {
    let (fake, _library, _workspace, _board, _, vcx) = open_board(cx);
    // Every one of them is left to the page, also those the window binds
    // elsewhere (comments, Vim mode, find next).
    for key in innerword_board_web::keys::WHITEBOARD_KEYS {
        for spelling in innerword_app::boards::key_spellings(key.keys) {
            assert!(!fake.press(&spelling), "{spelling} ({})", key.what);
        }
    }
    // The window's own keys reach GPUI from a whiteboard.
    for key in [
        "cmd-shift-p",
        "cmd-p",
        "cmd-0",
        "cmd-1",
        "cmd-,",
        "cmd-/",
        "cmd-s",
        "cmd-shift-s",
    ] {
        assert!(fake.press(key), "{key}");
        vcx.run_until_parked();
        vcx.update(|window, cx| window.dispatch_action(Box::new(innerword_ui::modal::Dismiss), cx));
        vcx.run_until_parked();
    }
}

#[gpui::test]
fn a_key_given_to_a_window_command_that_the_whiteboard_uses_is_said(cx: &mut TestAppContext) {
    let (_fake, _library, workspace, _board, _, vcx) = open_board(cx);
    let listing = vcx.update(|_, cx| {
        workspace
            .read(cx)
            .shortcuts_listing_here(cx)
            .expect("a listing")
    });
    let target = innerword_keymap::Target::new("workspace::ToggleSidebar", None);
    let key = innerword_keymap::KeySequence::parse("cmd-d").unwrap();
    let seeing = listing.seeing_first(&target, &key);
    assert!(
        seeing.iter().any(|taker| taker.warning.as_ref()
            == innerword_app::workspace::BOARD_TAKES_KEYS_FIRST),
        "the whiteboard sees ⌘D first"
    );
    let free = innerword_keymap::KeySequence::parse("cmd-alt-y").unwrap();
    assert!(listing.seeing_first(&target, &free).is_empty());
}

// ----- a relaunch, the version history ------------------------------------------------------

#[gpui::test]
fn the_last_open_whiteboard_reopens_and_the_shape_library_comes_back(cx: &mut TestAppContext) {
    let (fake, library, workspace, board, _, vcx) = open_board(cx);
    fake.change_library(
        &json!([{ "id": "kept", "status": "unpublished", "created": 1,
            "elements": [rect("s", 0.0, 1)] }])
        .to_string(),
    );
    fake.draw(&scene(&[rect("a", 0.0, 1)]));
    settle(vcx);
    workspace.update(vcx, |w, cx| w.flush(cx));
    let library2 = library.clone();
    let (workspace2, vcx2) = open_workspace(&library2, cx);
    assert_eq!(
        workspace2.read_with(vcx2, |w, _| w.open_board_id()),
        Some(board)
    );
    assert!(fake.library_served().last().unwrap().contains("kept"));
}

#[gpui::test]
fn a_version_of_a_whiteboard_is_previewed_as_its_listing(cx: &mut TestAppContext) {
    let (fake, library, workspace, board, _, vcx) = open_board(cx);
    let mut shape = rect("api", 0.0, 1);
    shape["type"] = json!("ellipse");
    fake.draw(&scene(&[shape]));
    settle(vcx);
    library.update(vcx, |l, cx| l.create_snapshot(board, cx).unwrap());
    vcx.update(|window, cx| {
        window.dispatch_action(Box::new(innerword_app::actions::ShowVersionHistory), cx)
    });
    vcx.run_until_parked();
    let preview = workspace.read_with(vcx, |w, cx| {
        w.modal_layer()
            .read(cx)
            .active::<innerword_app::version_history::VersionHistory>()
            .map(|history| history.read(cx).preview_text())
    });
    let preview = preview.expect("the version history is open");
    assert!(preview.contains("ellipse"), "{preview}");
    assert!(preview.contains("api"), "{preview}");
}

// ----- pictures from files, Mermaid's pictures ------------------------------------------------

#[gpui::test]
fn insert_image_on_a_whiteboard_stores_the_file_and_puts_it_on_the_canvas(cx: &mut TestAppContext) {
    let (fake, library, _workspace, _board, _, vcx) = open_board(cx);
    let dir = tempfile::tempdir().unwrap();
    let png = innerword_image::fixtures::png(64, 48, [20, 160, 90]);
    let path = dir.path().join("harbour.png");
    std::fs::write(&path, &png).unwrap();
    let not_a_picture = dir.path().join("notes.txt");
    std::fs::write(&not_a_picture, b"words").unwrap();
    vcx.update(|_, cx| {
        innerword_app::images::choose_files_beforehand(
            vec![path.clone(), not_a_picture.clone()],
            cx,
        )
    });
    vcx.update(|window, cx| {
        window.dispatch_action(Box::new(innerword_editor::actions::InsertImage), cx)
    });
    for _ in 0..10 {
        vcx.executor().advance_clock(Duration::from_millis(10));
        vcx.run_until_parked();
    }
    assert!(
        fake.board_calls()
            .iter()
            .any(|call| call == "insertPictures 1")
    );
    let file = innerword_library::store::FileId::of(&png);
    assert!(
        library.read_with(vcx, |l, _| l.store().file_info(&file).unwrap().is_some()),
        "stored before it is on the canvas"
    );
}

#[gpui::test]
fn mermaid_to_drawing_and_its_svg_picture(cx: &mut TestAppContext) {
    let (fake, library, _workspace, board, _, vcx) = open_board(cx);
    vcx.update(|window, cx| window.dispatch_action(Box::new(MermaidToDrawing), cx));
    vcx.run_until_parked();
    assert!(fake.board_calls().contains(&"mermaid".to_string()));
    // A pie chart arrives as one SVG picture, posted as a pasted one is.
    let svg = br#"<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><circle r="4" cx="5" cy="5"/></svg>"#;
    fake.paste_picture("pie", "image/svg+xml", svg);
    let mut picture = rect("chart", 0.0, 1);
    picture["type"] = json!("image");
    picture["fileId"] = json!("pie");
    fake.draw(&scene(&[picture]));
    settle(vcx);
    let saved = stored(&library, board, vcx);
    let entry = saved.files().get("pie").expect("the picture's entry");
    assert_eq!(entry.mime_type, "image/svg+xml");
    assert_eq!(
        entry.file.as_ref().map(|f| f.as_str().to_string()),
        Some(innerword_library::store::FileId::of(svg).to_hex()),
        "stored as it came"
    );
}
