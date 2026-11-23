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
        AddRectangle, AddToShapeLibrary, BOARD_SAVE_DELAY, InsertFromShapeLibrary,
        LIBRARY_TOO_LARGE, MermaidToDrawing, NO_COMMENTS_ON_A_BOARD, NOT_RELOADED, RELOADED,
        Recovered, ToggleShapeLibrary,
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
    // A rename while a save is in flight (Save Now asks the page first, and
    // its answer waits) keeps the new title.
    fake.hold_collects();
    fake.draw_unposted(&scene(&[rect("a", 10.0, 2)]));
    vcx.run_until_parked();
    vcx.update(|window, cx| window.dispatch_action(Box::new(innerword_app::actions::SaveNow), cx));
    vcx.run_until_parked();
    assert!(fake.collects_asked() >= 1, "the save is under way");
    library.update(vcx, |l, cx| l.rename_document(board, "Plan B", cx).unwrap());
    fake.release_collects();
    settle(vcx);
    assert_eq!(stored(&library, board, vcx).element("a").unwrap().x(), 10.0);
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
fn quit_through_its_listener_saves_what_the_page_holds(cx: &mut TestAppContext) {
    let (fake, library, workspace, board, _, vcx) = open_board(cx);
    fake.draw_unposted(&scene(&[rect("unposted", 0.0, 1)]));
    vcx.run_until_parked();
    // `cmd-q`, Inner Word > Quit: the app-wide listener.
    vcx.update(|window, cx| window.dispatch_action(Box::new(innerword_app::Quit), cx));
    vcx.run_until_parked();
    assert!(vcx.update(|_, cx| innerword_app::quit::requested(cx)));
    assert!(fake.collects_asked() >= 1, "the page was asked first");
    assert_eq!(ids(&stored(&library, board, vcx)), ["unposted"]);
    let _ = workspace;
}

#[gpui::test]
fn quit_request_as_the_signal_task_calls_it_saves_what_the_page_holds(cx: &mut TestAppContext) {
    let (fake, library, _workspace, board, _, vcx) = open_board(cx);
    fake.draw_unposted(&scene(&[rect("unposted", 0.0, 1)]));
    vcx.run_until_parked();
    vcx.update(|_, cx| innerword_app::quit::request(cx));
    vcx.run_until_parked();
    assert!(fake.collects_asked() >= 1, "the page was asked first");
    assert_eq!(ids(&stored(&library, board, vcx)), ["unposted"]);
}

/// A whiteboard whose last gesture is still being collected (parked, its
/// collect held) while `end` closes the window or quits: it is saved first.
fn a_closing_whiteboard_is_saved_by(end: &str, cx: &mut TestAppContext) {
    let (fake, library, workspace, board, text, vcx) = open_board(cx);
    fake.hold_collects();
    fake.draw_unposted(&scene(&[rect("late", 0.0, 1)]));
    vcx.run_until_parked();
    workspace.update_in(vcx, |w, window, cx| {
        w.open_document(text, true, window, cx);
    });
    vcx.run_until_parked();
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.board_closing()),
        Some(board)
    );
    match end {
        "cmd-w" => vcx.simulate_keystrokes("cmd-w"),
        "close button" => assert!(!vcx.simulate_close(), "the window waits"),
        "Quit" => {
            vcx.update(|window, cx| window.dispatch_action(Box::new(innerword_app::Quit), cx))
        }
        "quit::request" => vcx.update(|_, cx| innerword_app::quit::request(cx)),
        other => panic!("no way {other}"),
    }
    vcx.run_until_parked();
    assert!(ids(&stored(&library, board, vcx)).is_empty(), "not yet");
    fake.release_collects();
    for _ in 0..10 {
        vcx.executor().advance_clock(Duration::from_millis(20));
        vcx.run_until_parked();
    }
    assert_eq!(ids(&stored(&library, board, vcx)), ["late"], "{end}");
}

#[gpui::test]
fn cmd_w_saves_a_closing_whiteboard(cx: &mut TestAppContext) {
    a_closing_whiteboard_is_saved_by("cmd-w", cx);
}

#[gpui::test]
fn the_close_button_saves_a_closing_whiteboard(cx: &mut TestAppContext) {
    a_closing_whiteboard_is_saved_by("close button", cx);
}

#[gpui::test]
fn quit_through_its_listener_saves_a_closing_whiteboard(cx: &mut TestAppContext) {
    a_closing_whiteboard_is_saved_by("Quit", cx);
}

#[gpui::test]
fn quit_request_saves_a_closing_whiteboard(cx: &mut TestAppContext) {
    a_closing_whiteboard_is_saved_by("quit::request", cx);
}

#[gpui::test]
fn two_whiteboards_closing_at_once_are_both_saved(cx: &mut TestAppContext) {
    let (fake, library, workspace, first, text, vcx) = open_board(cx);
    fake.hold_collects();
    fake.draw_unposted(&scene(&[rect("first-late", 0.0, 1)]));
    vcx.run_until_parked();
    let second = workspace
        .update_in(vcx, |w, window, cx| w.new_board(Some("Second"), window, cx))
        .unwrap();
    vcx.run_until_parked();
    fake.board_ready();
    fake.draw_unposted(&scene(&[rect("second-late", 0.0, 1)]));
    vcx.run_until_parked();
    workspace.update_in(vcx, |w, window, cx| {
        w.open_document(text, true, window, cx);
    });
    vcx.run_until_parked();
    // The first is not ended with what the app holds of it when the second
    // begins to close.
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.boards_closing()),
        vec![first, second]
    );
    fake.release_collects();
    settle(vcx);
    assert_eq!(ids(&stored(&library, first, vcx)), ["first-late"]);
    assert_eq!(ids(&stored(&library, second, vcx)), ["second-late"]);
    assert!(workspace.read_with(vcx, |w, _| w.boards_closing().is_empty()));
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

// ----- overlays ---------------------------------------------------------------------------

/// Draws the next frame: in a test window the frame a paint asks for
/// (`request_animation_frame`) comes only when something draws.
fn next_frame(vcx: &mut VisualTestContext) {
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
}

/// The holes of `overlays` (window coordinates) in the whiteboard's view.
fn holes_of(
    overlays: &[gpui::Bounds<gpui::Pixels>],
    vcx: &mut VisualTestContext,
) -> Vec<innerword_web::Rect> {
    let area = vcx
        .debug_bounds("page-area")
        .expect("the whiteboard's area");
    innerword_app::workspace::page_holes_in(area, overlays)
}

/// A modal over a whiteboard (quick open, the palette, the settings, the
/// list of shortcuts) is up in the frame it opens, over the live canvas:
/// the view is covered (dimmed with the backdrop, no event) and never
/// hidden, so its page is not told it is hidden and posts nothing because
/// of the overlay (a stroke it holds stays unposted, the app asks it for
/// nothing and saves nothing). The modal is seen through a hole once GPUI
/// has drawn it; closing it gives the canvas the keyboard back.
#[gpui::test]
fn a_modal_over_a_whiteboard_is_up_at_once_and_its_page_is_told_nothing(cx: &mut TestAppContext) {
    let (fake, library, workspace, board, _, vcx) = open_board(cx);
    fake.draw(&scene(&[rect("a", 0.0, 1)]));
    settle(vcx);
    // A stroke the page has not posted yet.
    fake.draw_unposted(&scene(&[rect("a", 30.0, 2)]));
    vcx.run_until_parked();
    let status = fake.board_status();
    assert!(status.rev > status.posted, "{status:?}");
    let collects = fake.collects_asked();
    let focused = fake.first_responder_is_page();
    for key in ["cmd-p", "cmd-shift-p", "cmd-,", "cmd-/"] {
        vcx.simulate_keystrokes(key);
        vcx.run_until_parked();
        assert!(
            workspace.read_with(vcx, |w, cx| w.modal_layer().read(cx).has_modal()),
            "{key} opens a modal"
        );
        let cover = fake
            .cover()
            .unwrap_or_else(|| panic!("{key}: the view is covered"));
        assert!(cover.3 > 0., "{key}: under the backdrop ({cover:?})");
        assert!(!fake.hidden(), "{key}: the canvas stays shown");
        assert!(
            !fake.first_responder_is_page(),
            "{key}: the modal has the keys"
        );
        let line = state_line(&workspace, vcx);
        assert!(line.contains("overlay=dimmed:"), "{key}: {line}");
        next_frame(vcx);
        let modal = vcx.debug_bounds("modal").expect("the modal is drawn");
        assert_eq!(fake.holes(), holes_of(&[modal], vcx), "{key}");
        vcx.simulate_keystrokes("escape");
        vcx.run_until_parked();
        assert!(
            !workspace.read_with(vcx, |w, cx| w.modal_layer().read(cx).has_modal()),
            "{key}: escape closes it"
        );
        assert_eq!(fake.cover(), None, "{key}");
        assert!(fake.holes().is_empty(), "{key}");
        assert_eq!(fake.first_responder_is_page(), focused, "{key}");
    }
    assert_eq!(fake.times_hidden(), 0, "no overlay hid the canvas");
    assert_eq!(fake.board_status(), status, "the page posted nothing");
    assert_eq!(fake.collects_asked(), collects, "nothing was asked of it");
    assert_eq!(
        stored(&library, board, vcx).element("a").unwrap().x(),
        0.0,
        "nothing was saved"
    );
}

/// The whiteboard's menu (`ctrl-enter`): the canvas is covered without a
/// dim (a context menu has no backdrop) and takes no event, so a press
/// outside the menu reaches GPUI and closes it; the menu is seen through a
/// hole once drawn. A toast that comes meanwhile is seen too.
#[gpui::test]
fn a_context_menu_over_a_whiteboard_covers_it_without_a_dim(cx: &mut TestAppContext) {
    let (fake, _library, workspace, _board, _, vcx) = open_board(cx);
    workspace.update_in(vcx, |w, window, _| {
        if let Some(focus) = w.page_focus_handle() {
            window.focus(&focus);
        }
    });
    vcx.run_until_parked();
    vcx.simulate_keystrokes("ctrl-enter");
    vcx.run_until_parked();
    assert!(vcx.update(|window, cx| innerword_ui::ContextMenu::is_open(window, cx)));
    assert_eq!(fake.cover(), Some((0., 0., 0., 0.)));
    assert!(!fake.hidden());
    assert!(state_line(&workspace, vcx).contains("overlay=covered:"));
    next_frame(vcx);
    let menu = vcx
        .update(|window, cx| {
            innerword_ui::ContextMenu::current(window, cx).and_then(|menu| menu.read(cx).bounds())
        })
        .expect("the menu was drawn");
    assert_eq!(fake.holes(), holes_of(&[menu], vcx));
    // A toast while the menu is open: a hole of its own.
    workspace.update(vcx, |w, cx| {
        w.show_toast(innerword_ui::Toast::new("Claude changed “Plan”"), cx)
    });
    vcx.run_until_parked();
    next_frame(vcx);
    next_frame(vcx);
    let toast = workspace.read_with(vcx, |w, cx| w.toast_layer().read(cx).toast_bounds());
    assert_eq!(toast.len(), 1);
    assert_eq!(fake.holes(), holes_of(&[toast[0], menu], vcx));
    assert!(state_line(&workspace, vcx).contains("overlay=covered:2"));
    vcx.update(innerword_ui::ContextMenu::dismiss);
    vcx.run_until_parked();
    assert_eq!(fake.cover(), None);
    assert!(state_line(&workspace, vcx).contains("overlay=holes:1"));
    assert_eq!(fake.times_hidden(), 0);
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
    // The picker lists the held items (once the page can insert one).
    fake.board_ready();
    vcx.run_until_parked();
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
    // The title's menu has the shape library's three items (section 4.15).
    workspace.update_in(vcx, |w, window, cx| {
        assert!(w.open_menu(innerword_app::workspace::MenuKind::Title, None, window, cx));
    });
    vcx.run_until_parked();
    let labels = vcx.update(|window, cx| {
        innerword_ui::ContextMenu::current(window, cx)
            .map(|menu| menu.read(cx).labels())
            .unwrap_or_default()
    });
    for wanted in [
        "Shape Library",
        "Add Selection to Shape Library",
        "Version History…",
    ] {
        assert!(
            labels.iter().any(|label| label == wanted),
            "{wanted}: {labels:?}"
        );
    }
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
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
        "Add from Shape Library…",
        "Shape Library",
        "Add Selection to Shape Library",
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
    // What the status bar shows: the save state and the elements, counted
    // as the listing counts them, of the stored scene (not the page's own
    // count, which has the labels in it).
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.board_save_label()),
        Some(("Saved".to_string(), false))
    );
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.board_elements_label()),
        Some("0 elements".to_string())
    );
    let mut labelled = rect("a", 0.0, 2);
    labelled["boundElements"] = json!([{ "id": "label", "type": "text" }]);
    fake.draw_unposted(&scene(&[rect("a", 0.0, 1)]));
    fake.draw(&scene(&[labelled, label("label", "a", "Server")]));
    vcx.run_until_parked();
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.board_save_label()),
        Some(("Edited".to_string(), false))
    );
    settle(vcx);
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.board_save_label()),
        Some(("Saved".to_string(), false))
    );
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.board_elements_label()),
        Some("1 element".to_string()),
        "a label is part of its shape"
    );
    let labels = workspace.update(vcx, |w, cx| {
        w.menu_items(innerword_app::workspace::MenuKind::Save, cx)
            .len()
    });
    assert_eq!(labels, 1, "Save Now");
    // The menu of the counts says the same, in plain words.
    workspace.update_in(vcx, |w, window, cx| {
        assert!(w.open_menu(
            innerword_app::workspace::MenuKind::Statistics,
            None,
            window,
            cx
        ));
    });
    vcx.run_until_parked();
    let labels = vcx.update(|window, cx| {
        innerword_ui::ContextMenu::current(window, cx)
            .map(|menu| menu.read(cx).labels())
            .unwrap_or_default()
    });
    for wanted in ["1 element with 1 label", "1 rectangle", "1 word"] {
        assert!(
            labels.iter().any(|label| label == wanted),
            "{wanted}: {labels:?}"
        );
    }
    assert!(
        !labels.iter().any(|label| label.contains("picture")),
        "{labels:?}"
    );
}

/// A text bound to `container` (its label), as Excalidraw keeps it.
fn label(id: &str, container: &str, text: &str) -> Value {
    json!({
        "id": id, "type": "text", "x": 10.0, "y": 10.0, "width": 80.0, "height": 25.0,
        "angle": 0, "strokeColor": "#1e1e1e", "backgroundColor": "transparent",
        "fillStyle": "solid", "strokeWidth": 2, "strokeStyle": "solid", "roughness": 1,
        "opacity": 100, "groupIds": [], "frameId": null, "roundness": null,
        "seed": 1, "version": 1, "versionNonce": 1, "isDeleted": false,
        "boundElements": null, "updated": 1, "link": null, "locked": false,
        "index": "a9", "text": text, "originalText": text, "fontSize": 20,
        "fontFamily": 5, "textAlign": "center", "verticalAlign": "middle",
        "containerId": container, "autoResize": true, "lineHeight": 1.25,
    })
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
    let giving = listing.giving_up(&target, &key);
    assert_eq!(giving.len(), 1, "the whiteboard has ⌘D");
    let (giver, what) = giving[0];
    assert_eq!(what, "Duplicate");
    assert_eq!(
        giver.warning.as_ref(),
        innerword_app::workspace::BOARD_GIVES_KEY_UP
    );
    // A command of the whiteboard's own place is concerned too, one of the
    // text is not.
    let add = innerword_keymap::Target::new("board::AddText", Some("BoardView"));
    assert_eq!(listing.giving_up(&add, &key).len(), 1);
    let bold = innerword_keymap::Target::new("editor::ToggleBold", Some("Editor"));
    assert!(listing.giving_up(&bold, &key).is_empty());
    let free = innerword_keymap::KeySequence::parse("cmd-alt-y").unwrap();
    assert!(listing.giving_up(&target, &free).is_empty());
    // Nothing sees a whiteboard's key first any more: it is given up.
    assert!(listing.seeing_first(&target, &key).is_empty());
}

/// Gives the command palette `keys` as the list does (`None`: its default
/// back).
fn give_the_palette(
    workspace: &Entity<Workspace>,
    keys: Option<&[&str]>,
    vcx: &mut VisualTestContext,
) {
    let keys: Option<Vec<innerword_keymap::KeySequence>> = keys.map(|keys| {
        keys.iter()
            .map(|keys| innerword_keymap::KeySequence::parse(keys).unwrap())
            .collect()
    });
    workspace
        .update_in(vcx, |w, window, cx| {
            w.set_keys_of_command(
                "workspace::ToggleCommandPalette",
                None,
                keys.as_deref(),
                window,
                cx,
            )
        })
        .expect("the palette is a row");
    vcx.run_until_parked();
}

fn palette_open(workspace: &Entity<Workspace>, vcx: &mut VisualTestContext) -> bool {
    workspace.read_with(vcx, |w, cx| {
        w.modal_layer()
            .read(cx)
            .active::<innerword_app::palette::CommandPalette>()
            .is_some()
    })
}

/// Presses `keys` with the keyboard on the whiteboard, as GPUI's view gets
/// a `cmd` chord before the web view, and says whether the palette opened
/// (and closes it again).
fn palette_opens_on_the_board(
    workspace: &Entity<Workspace>,
    keys: &str,
    vcx: &mut VisualTestContext,
) -> bool {
    let focus = workspace.read_with(vcx, |w, _| w.page_focus_handle().unwrap());
    vcx.update(|window, _| window.focus(&focus));
    vcx.run_until_parked();
    vcx.simulate_keystrokes(keys);
    vcx.run_until_parked();
    let open = palette_open(workspace, vcx);
    if open {
        vcx.update(|window, cx| window.dispatch_action(Box::new(innerword_ui::modal::Dismiss), cx));
        vcx.run_until_parked();
    }
    assert!(!palette_open(workspace, vcx));
    open
}

/// The key a menu bar item shows now, by its name.
fn menu_key(name: &str, vcx: &mut VisualTestContext) -> Option<String> {
    vcx.update(|_, cx| {
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        let (_, action) = innerword_app::menus::menu_actions(&innerword_app::menus::app_menus())
            .into_iter()
            .find(|(item, _)| item == name)
            .unwrap_or_else(|| panic!("the menu bar has {name}"));
        innerword_app::menus::native_menu_binding(&keymap, action.as_ref())
            .filter(|binding| binding.keystrokes().len() == 1)
            .map(innerword_app::keymap::keys_of)
    })
}

/// The fixed row "On a whiteboard" titled `title`: its keys and its note.
fn board_row(title: &str, vcx: &mut VisualTestContext) -> (String, String) {
    vcx.update(|_, cx| {
        innerword_app::shortcuts::collect_shortcuts(cx)
            .into_iter()
            .find(|row| row.place == innerword_app::shortcuts::Place::Board && row.title == title)
            .map(|row| (row.keys, row.fixed.unwrap_or_default().to_string()))
            .unwrap_or_else(|| panic!("a row {title:?} on a whiteboard"))
    })
}

/// The user gave the command palette ⌘K, one of Excalidraw's keys (Link of
/// the selected element): on a whiteboard ⌘K opens the palette, the menu
/// bar keeps it beside Command Palette…, and the fixed row of Excalidraw's
/// Link says who has it. Without the change, and once it is reset, ⌘K is
/// Excalidraw's again (GPUI's `NoAction`) and the palette does not open.
#[gpui::test]
fn a_whiteboard_key_the_user_gave_a_command_runs_it_there(cx: &mut TestAppContext) {
    let (fake, _library, workspace, _board, _, vcx) = open_board(cx);
    let link = "Link of the selected element";
    // Without the change: Excalidraw's.
    assert!(!fake.press("cmd-k"), "GPUI leaves ⌘K to the page");
    assert!(!palette_opens_on_the_board(&workspace, "cmd-k", vcx));
    assert_eq!(
        menu_key("Command Palette…", vcx).as_deref(),
        Some("cmd-shift-p")
    );
    assert_eq!(menu_key("Link…", vcx), None);
    assert_eq!(
        board_row(link, vcx),
        (
            "⌘K".to_string(),
            innerword_app::shortcuts::BOARD_KEY_NOTE.to_string()
        )
    );

    give_the_palette(&workspace, Some(&["cmd-k"]), vcx);
    // GPUI takes ⌘K on the whiteboard and runs the palette: from GPUI's
    // view, and when the web view hands the key on.
    assert!(palette_opens_on_the_board(&workspace, "cmd-k", vcx));
    assert!(fake.press("cmd-k"), "GPUI takes ⌘K on a whiteboard");
    vcx.run_until_parked();
    assert!(palette_open(&workspace, vcx), "the key handed on opens it");
    vcx.update(|window, cx| window.dispatch_action(Box::new(innerword_ui::modal::Dismiss), cx));
    vcx.run_until_parked();
    // The rest of Excalidraw's keys are still its own.
    assert!(!fake.press("cmd-d"));
    assert!(!fake.press("cmd-alt-c"));
    // The menu bar keeps the user's key beside the palette, and only there.
    assert_eq!(menu_key("Command Palette…", vcx).as_deref(), Some("cmd-k"));
    assert_eq!(menu_key("Link…", vcx), None);
    // The fixed row of Excalidraw's Link says what changed.
    let (keys, note) = board_row(link, vcx);
    assert_eq!(keys, "", "⌘K does not do Excalidraw's link there any more");
    assert_eq!(
        note,
        innerword_app::shortcuts::board_key_given_note("⌘K", "Command palette", link)
    );
    assert_eq!(board_row("Duplicate", vcx).0, "⌘D");

    // A key with a second spelling: shift with a punctuation key reaches
    // GPUI as the shifted character.
    give_the_palette(&workspace, Some(&["cmd-shift-."]), vcx);
    assert!(palette_opens_on_the_board(&workspace, "cmd->", vcx));
    assert!(palette_opens_on_the_board(&workspace, "cmd-shift-.", vcx));
    assert!(
        !palette_opens_on_the_board(&workspace, "cmd-k", vcx),
        "⌘K is held again"
    );
    assert!(fake.press("cmd->"));
    vcx.run_until_parked();
    vcx.update(|window, cx| window.dispatch_action(Box::new(innerword_ui::modal::Dismiss), cx));
    vcx.run_until_parked();
    assert!(board_row("Larger font", vcx).1.contains("Command palette"));

    // Reset: the hold is back.
    give_the_palette(&workspace, None, vcx);
    assert!(!fake.press("cmd-k"));
    assert!(!fake.press("cmd->"));
    assert!(!palette_opens_on_the_board(&workspace, "cmd-k", vcx));
    assert!(!palette_opens_on_the_board(&workspace, "cmd->", vcx));
    assert!(palette_opens_on_the_board(&workspace, "cmd-shift-p", vcx));
    assert_eq!(
        menu_key("Command Palette…", vcx).as_deref(),
        Some("cmd-shift-p")
    );
    assert_eq!(
        board_row(link, vcx),
        (
            "⌘K".to_string(),
            innerword_app::shortcuts::BOARD_KEY_NOTE.to_string()
        )
    );
}

/// The keys given up reach the page, so that Excalidraw's tooltips and menu
/// items stop naming them: in `board.json` at every load (`keysGivenUp`),
/// and through the call `keys` when the keymap is installed again while the
/// whiteboard is open (a key given, a key given back). An install that
/// changes nothing for the whiteboard tells it nothing.
#[gpui::test]
fn the_keys_given_up_reach_the_page_at_the_load_and_after_a_change(cx: &mut TestAppContext) {
    let (fake, _library, workspace, board, text, vcx) = open_board(cx);
    let keys_served = |fake: &FakePages| -> Value {
        let served: Value = serde_json::from_str(&fake.boards_opened().last().unwrap().1).unwrap();
        served["keysGivenUp"].clone()
    };
    let keys_calls = |fake: &FakePages| -> Vec<String> {
        fake.board_calls()
            .into_iter()
            .filter(|call| call.starts_with("keys"))
            .collect()
    };
    assert_eq!(
        keys_served(&fake),
        json!([]),
        "nothing given up at the load"
    );
    assert!(keys_calls(&fake).is_empty());

    give_the_palette(&workspace, Some(&["cmd-k"]), vcx);
    assert_eq!(keys_calls(&fake), ["keys cmd-k"], "told after the change");
    // A reload is served the keys as they are now.
    let served = workspace
        .update(vcx, |w, cx| {
            w.board_served(innerword_board_web::BOARD_PATH, cx)
        })
        .unwrap();
    let served: Value = serde_json::from_slice(&served.bytes).unwrap();
    assert_eq!(served["keysGivenUp"], json!(["cmd-k"]));

    // A change that leaves the whiteboard's keys as they were tells it nothing.
    give_the_palette(&workspace, Some(&["cmd-k", "cmd-shift-p"]), vcx);
    assert_eq!(keys_calls(&fake), ["keys cmd-k"]);

    // Opened again: the load has them.
    workspace.update_in(vcx, |w, window, cx| w.open_document(text, true, window, cx));
    vcx.run_until_parked();
    workspace.update_in(vcx, |w, window, cx| {
        w.open_document(board, true, window, cx)
    });
    vcx.run_until_parked();
    fake.board_ready();
    vcx.run_until_parked();
    assert_eq!(fake.boards_opened().len(), 2);
    assert_eq!(keys_served(&fake), json!(["cmd-k"]), "told at the load");
    assert_eq!(keys_calls(&fake), ["keys cmd-k"], "and not again by a call");

    // Given back: the page names ⌘K again.
    give_the_palette(&workspace, None, vcx);
    assert_eq!(keys_calls(&fake), ["keys cmd-k", "keys"]);
}

/// A key the user gave a command where a default still wins on a whiteboard
/// stays held: ⌥⌘C is the window's "Show or hide comments" by default (bound
/// after the palette's, so it wins where both apply) and Excalidraw's copy
/// styles. Given to the palette as a second key beside the comments', it is
/// held for Excalidraw as before, and the menu bar shows it nowhere there.
#[gpui::test]
fn a_whiteboard_key_a_default_wins_stays_held(cx: &mut TestAppContext) {
    let (fake, _library, workspace, _board, _, vcx) = open_board(cx);
    give_the_palette(&workspace, Some(&["cmd-shift-p", "cmd-alt-c"]), vcx);
    let given = |vcx: &mut VisualTestContext| {
        vcx.update(|_, cx| {
            innerword_app::keymap::whiteboard_keys(cx)
                .keys()
                .iter()
                .filter(|key| key.given_to.is_some())
                .map(|key| key.key.keys)
                .collect::<Vec<_>>()
        })
    };
    assert!(given(vcx).is_empty(), "{:?}", given(vcx));
    assert!(!fake.press("cmd-alt-c"));
    assert!(!palette_opens_on_the_board(&workspace, "cmd-alt-c", vcx));
    assert_eq!(menu_key("Show or Hide Comments", vcx), None);
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
    // The first launch ends: its window goes before the next one opens.
    vcx.update(|window, _| window.remove_window());
    vcx.run_until_parked();
    drop(workspace);
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

// ----- the review's cases --------------------------------------------------------------------

/// A whiteboard made straight in the store (as the tool makes one): its
/// stored scene is `scene_json`, and `files` the pictures the store is told
/// it names (a picture it names without them is one the library lost).
fn make_board(
    library: &Entity<Library>,
    folder: Option<innerword_library::FolderId>,
    title: &str,
    scene_json: &str,
    files: &[innerword_library::store::FileId],
    cx: &mut VisualTestContext,
) -> DocumentId {
    let scene = Scene::from_stored(scene_json)
        .expect("a stored scene")
        .scene;
    let content = scene.to_stored();
    let text = scene.text();
    library.update(cx, |l, cx| {
        let (id, done) = l.change_outside(
            |store| {
                let made = store
                    .create_board(
                        folder,
                        title,
                        &innerword_library::store::BoardContent {
                            scene: &content,
                            files,
                            plain_text: &text,
                            word_count: 0,
                            char_count: 0,
                        },
                    )
                    .expect("the whiteboard is made");
                let id = made.meta.id;
                (id, vec![OutsideChange::DocumentCreated(id)])
            },
            cx,
        );
        done.unwrap();
        id
    })
}

/// A stored scene of `elements` with `files` (Excalidraw's id to the stored
/// `FileId` hex).
fn stored_scene(elements: &[Value], files: &[(&str, &str)]) -> String {
    let files: serde_json::Map<String, Value> = files
        .iter()
        .map(|(id, hex)| {
            (
                id.to_string(),
                json!({ "mimeType": "image/png", "file": hex, "created": 1 }),
            )
        })
        .collect();
    json!({
        "type": "excalidraw", "version": 2, "source": "inner-word",
        "elements": elements, "appState": { "viewBackgroundColor": "#ffffff" }, "files": files,
    })
    .to_string()
}

fn picture(id: &str, file: &str) -> Value {
    let mut element = rect(id, 0.0, 1);
    element["type"] = json!("image");
    element["fileId"] = json!(file);
    element["status"] = json!("saved");
    element
}

fn library_items(library: &Entity<Library>, cx: &mut VisualTestContext) -> Vec<String> {
    let stored: Option<Box<serde_json::value::RawValue>> = library.read_with(cx, |l, _| {
        l.setting(innerword_app::boards::SHAPE_LIBRARY_SETTING)
            .unwrap()
    });
    let Some(stored) = stored else {
        return Vec::new();
    };
    innerword_board::ShapeLibrary::from_stored(stored.get())
        .unwrap()
        .library
        .items()
        .iter()
        .map(|item| item.id().to_string())
        .collect()
}

fn item(id: &str, created: i64) -> Value {
    json!({ "id": id, "status": "unpublished", "created": created,
        "elements": [rect(&format!("{id}-shape"), 0.0, 1)] })
}

#[gpui::test]
fn the_shape_library_of_a_second_view_and_after_a_reload_is_kept(cx: &mut TestAppContext) {
    let (fake, library, workspace, _board, _, vcx) = open_board(cx);
    fake.change_library(&json!([item("one", 1)]).to_string());
    vcx.run_until_parked();
    assert_eq!(library_items(&library, vcx), ["one"]);
    // A second whiteboard's view counts its changes from 0 again.
    workspace.update_in(vcx, |w, window, cx| w.new_board(Some("Second"), window, cx));
    vcx.run_until_parked();
    fake.board_ready();
    vcx.run_until_parked();
    assert!(fake.library_served().last().unwrap().contains("one"));
    fake.change_library(&json!([item("two", 2), item("one", 1)]).to_string());
    vcx.run_until_parked();
    let mut kept = library_items(&library, vcx);
    kept.sort();
    assert_eq!(kept, ["one", "two"], "the second view's change is stored");
    // So does the same view after Reload whiteboard.
    vcx.update(|window, cx| window.dispatch_action(Box::new(ReloadPage), cx));
    vcx.run_until_parked();
    assert_eq!(fake.reloads(), 1);
    fake.board_ready();
    vcx.run_until_parked();
    fake.change_library(&json!([item("three", 3), item("two", 2), item("one", 1)]).to_string());
    vcx.run_until_parked();
    let mut kept = library_items(&library, vcx);
    kept.sort();
    assert_eq!(kept, ["one", "three", "two"], "and after a reload");
    let line = state_line(&workspace, vcx);
    assert!(line.contains("library=3/saved"), "{line}");
}

#[gpui::test]
fn a_shape_library_change_not_posted_comes_with_the_collect_at_quit(cx: &mut TestAppContext) {
    let (fake, library, _workspace, board, _, vcx) = open_board(cx);
    fake.change_library_unposted(&json!([item("late", 1)]).to_string());
    fake.draw_unposted(&scene(&[rect("a", 0.0, 1)]));
    vcx.run_until_parked();
    vcx.update(|window, cx| window.dispatch_action(Box::new(innerword_app::Quit), cx));
    vcx.run_until_parked();
    assert_eq!(library_items(&library, vcx), ["late"]);
    assert_eq!(ids(&stored(&library, board, vcx)), ["a"]);
}

#[gpui::test]
fn a_shape_library_change_waiting_its_post_is_kept_by_a_switch_at_once(cx: &mut TestAppContext) {
    // A change made in Excalidraw's own sidebar, the drawing untouched: the
    // page posts it 300 ms later, and the writer opens a text before that.
    let (fake, library, workspace, _board, text, vcx) = open_board(cx);
    fake.change_library_unposted(&json!([item("sidebar", 1)]).to_string());
    vcx.run_until_parked();
    assert!(workspace.read_with(vcx, |w, _| w.board_holds_more()));
    let collects = fake.collects_asked();
    workspace.update_in(vcx, |w, window, cx| {
        w.open_document(text, true, window, cx);
    });
    vcx.run_until_parked();
    assert_eq!(fake.collects_asked(), collects + 1, "the page is asked");
    assert_eq!(library_items(&library, vcx), ["sidebar"]);
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.open_document_id()),
        Some(text)
    );
}

#[gpui::test]
fn a_shape_library_change_a_call_answered_is_collected_without_the_status(cx: &mut TestAppContext) {
    // An older glue's status has no `libraryWaiting`: the call's answer
    // alone says the page holds a change.
    let (fake, library, workspace, _board, text, vcx) = open_board(cx);
    let focus = workspace.read_with(vcx, |w, _| w.page_focus_handle().unwrap());
    vcx.update(|window, _| window.focus(&focus));
    vcx.update(|window, cx| window.dispatch_action(Box::new(AddToShapeLibrary), cx));
    vcx.run_until_parked();
    let mut status = fake.board_status();
    status.library_waiting = false;
    fake.set_status(status);
    vcx.run_until_parked();
    assert!(workspace.read_with(vcx, |w, _| w.board_holds_more()));
    workspace.update_in(vcx, |w, window, cx| {
        w.open_document(text, true, window, cx);
    });
    vcx.run_until_parked();
    assert_eq!(library_items(&library, vcx), ["added-1"]);
}

#[gpui::test]
fn an_item_of_the_shape_library_with_a_picture_is_dropped(cx: &mut TestAppContext) {
    let (fake, library, _workspace, _board, _, vcx) = open_board(cx);
    let with_picture = json!({ "id": "pictured", "status": "unpublished", "created": 2,
        "elements": [picture("p", "f1")] });
    fake.change_library(&json!([with_picture, item("plain", 1)]).to_string());
    vcx.run_until_parked();
    assert_eq!(library_items(&library, vcx), ["plain"]);
}

#[gpui::test]
fn the_shape_library_picker_lists_the_held_items_and_enter_inserts(cx: &mut TestAppContext) {
    let (fake, _library, workspace, _board, _, vcx) = open_board(cx);
    let mut named = item("named", 2);
    named["name"] = json!("Server");
    fake.change_library(&json!([named, item("plain", 1)]).to_string());
    vcx.run_until_parked();
    let focus = workspace.read_with(vcx, |w, _| w.page_focus_handle().unwrap());
    vcx.update(|window, _| window.focus(&focus));
    vcx.update(|window, cx| window.dispatch_action(Box::new(InsertFromShapeLibrary), cx));
    vcx.run_until_parked();
    let rows = workspace.read_with(vcx, |w, cx| {
        w.modal_layer()
            .read(cx)
            .active::<Picker<innerword_app::workspace::ShapeLibraryDelegate>>()
            .map(|picker| {
                use innerword_ui::PickerDelegate as _;
                picker.read(cx).delegate.entry_count()
            })
    });
    assert_eq!(rows, Some(2), "the two items, newest first");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert!(
        fake.board_calls()
            .iter()
            .any(|call| call == "library insert"),
        "{:?}",
        fake.board_calls()
    );
    assert!(!workspace.read_with(vcx, |w, cx| w.modal_layer().read(cx).has_modal()));
}

#[gpui::test]
fn a_flush_while_a_background_save_is_answered_takes_its_write_as_the_base(
    cx: &mut TestAppContext,
) {
    init(cx);
    let fake = cx.update(pages::use_fake);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.db");
    let library = cx.new(|_| Library::open(&path).expect("the library opens"));
    library.update(cx, |library, cx| {
        library
            .set_setting(innerword_app::settings::keys::WELCOME_CREATED, &true, cx)
            .unwrap();
    });
    let (workspace, vcx) = open_workspace(&library, cx);
    let board = workspace
        .update_in(vcx, |w, window, cx| w.new_board(Some("Plan"), window, cx))
        .unwrap();
    vcx.run_until_parked();
    fake.board_ready();
    let mut status = fake.board_status();
    status.zoom = 1.5;
    status.scroll_x = -40.0;
    fake.set_status(status);
    vcx.run_until_parked();
    // A save in the background that writes and whose answer is held.
    vcx.update(|_, cx| innerword_app::boards::hold_board_saves(cx));
    fake.draw(&scene(&[rect("a", 0.0, 1)]));
    vcx.executor().advance_clock(BOARD_SAVE_DELAY);
    vcx.run_until_parked();
    assert_eq!(ids(&stored(&library, board, vcx)), ["a"], "written");
    // Where the writer looks is stored on the scene that save wrote.
    let row = library.read_with(vcx, |l, _| l.document(board).unwrap());
    let view: Value = serde_json::from_str(row.view_state.as_deref().unwrap()).unwrap();
    assert_eq!(
        view["at"].as_str().unwrap(),
        innerword_library::store::ContentStamp::of(&row.content).to_hex()
    );
    // The writer moves it, and the window goes before the answer came.
    fake.draw(&scene(&[rect("a", 80.0, 2)]));
    vcx.run_until_parked();
    assert!(
        workspace.update(vcx, |w, cx| w.flush(cx)),
        "the flush saves"
    );
    assert_eq!(stored(&library, board, vcx).element("a").unwrap().x(), 80.0);
    assert!(
        fake.adopted().is_empty(),
        "its own write is no outside change"
    );
    vcx.update(|_, cx| innerword_app::boards::release_board_saves(cx));
    settle(vcx);
    assert_eq!(stored(&library, board, vcx).element("a").unwrap().x(), 80.0);
    assert!(state_line(&workspace, vcx).contains("dirty=no"));
}

#[gpui::test]
fn a_flush_that_finds_the_row_changed_merges_and_keeps_both(cx: &mut TestAppContext) {
    let (fake, library, workspace, board, _, vcx) = open_board(cx);
    fake.draw(&scene(&[rect("mine", 0.0, 1)]));
    settle(vcx);
    // Held and not saved yet; the row changes under it.
    fake.draw(&scene(&[rect("mine", 0.0, 1), rect("new", 600.0, 1)]));
    vcx.run_until_parked();
    outside_write(
        &library,
        board,
        &[rect("mine", 0.0, 1), rect("theirs", 300.0, 1)],
        vcx,
    );
    assert!(workspace.update(vcx, |w, cx| w.flush(cx)));
    let mut kept = ids(&stored(&library, board, vcx));
    kept.sort();
    assert_eq!(kept, ["mine", "new", "theirs"]);
    // The view takes the merge as an outside write.
    vcx.run_until_parked();
    assert_eq!(fake.adopted().len(), 1);
}

#[gpui::test]
fn a_failed_adopt_keeps_the_strokes_made_while_it_was_waited_for(cx: &mut TestAppContext) {
    let (fake, library, workspace, board, _, vcx) = open_board(cx);
    fake.draw(&scene(&[rect("mine", 0.0, 1)]));
    settle(vcx);
    outside_write(
        &library,
        board,
        &[rect("mine", 0.0, 1), rect("theirs", 300.0, 1)],
        vcx,
    );
    fake.fail_adopts(FakeFailure::NeverAnswers);
    workspace.update(vcx, |w, cx| w.adopt_outside_write(board, cx));
    vcx.run_until_parked();
    // A stroke posted while the adopt is waited for, and one not posted.
    fake.draw(&scene(&[rect("mine", 0.0, 1), rect("late", 600.0, 1)]));
    fake.draw_unposted(&scene(&[
        rect("mine", 0.0, 1),
        rect("late", 600.0, 1),
        rect("later", 900.0, 1),
    ]));
    vcx.run_until_parked();
    vcx.executor()
        .advance_clock(innerword_app::boards::ADOPT_PATIENCE);
    vcx.run_until_parked();
    let mut kept = ids(&stored(&library, board, vcx));
    kept.sort();
    assert_eq!(kept, ["late", "later", "mine", "theirs"]);
    assert_eq!(fake.reloads(), 1, "the view loads the merge");
    let canvas = fake.canvas();
    assert!(
        canvas.contains("theirs") && canvas.contains("later"),
        "{canvas}"
    );
}

#[gpui::test]
fn an_outside_write_that_changes_no_element_keeps_what_the_app_holds(cx: &mut TestAppContext) {
    let (fake, library, _workspace, board, _, vcx) = open_board(cx);
    fake.draw(&scene(&[rect("a", 0.0, 1)]));
    settle(vcx);
    // The writer moves it; the row gets another stamp with the same elements.
    fake.draw(&scene(&[rect("a", 70.0, 2)]));
    outside_write(&library, board, &[rect("a", 0.0, 1)], vcx);
    settle(vcx);
    assert_eq!(stored(&library, board, vcx).element("a").unwrap().x(), 70.0);
}

#[gpui::test]
fn a_search_hit_on_a_closed_whiteboard_searches_once_it_is_ready(cx: &mut TestAppContext) {
    let (fake, _library, workspace, board, text, vcx) = open_board(cx);
    let mut label = rect("t", 0.0, 1);
    label["type"] = json!("text");
    label["text"] = json!("Zanzibar");
    fake.draw(&scene(&[label]));
    settle(vcx);
    workspace.update_in(vcx, |w, window, cx| {
        w.open_document(text, true, window, cx);
    });
    vcx.run_until_parked();
    workspace.update_in(vcx, |w, window, cx| {
        assert!(w.open_search_hit(board, "Zanzibar", window, cx));
    });
    vcx.run_until_parked();
    assert!(
        !fake
            .board_calls()
            .iter()
            .any(|call| call == "search Zanzibar"),
        "not before the page is ready"
    );
    fake.board_ready();
    vcx.run_until_parked();
    assert!(
        fake.board_calls()
            .iter()
            .any(|call| call == "search Zanzibar"),
        "{:?}",
        fake.board_calls()
    );
}

#[gpui::test]
fn add_text_holds_the_keys_until_the_new_text_is_open(cx: &mut TestAppContext) {
    let (fake, _library, _workspace, _board, _, vcx) = open_board(cx);
    vcx.update(|window, cx| {
        window.dispatch_action(Box::new(innerword_app::workspace::AddText), cx)
    });
    vcx.run_until_parked();
    let calls = fake.board_calls();
    let at = |name: &str| {
        calls
            .iter()
            .position(|call| call.starts_with(name))
            .unwrap_or_else(|| panic!("{name}: {calls:?}"))
    };
    assert!(at("holdKeys") < at("center"), "{calls:?}");
    assert!(at("center") < at("insertElements"), "{calls:?}");
    assert!(at("insertElements") < at("openText "), "{calls:?}");
    let canvas: Value = serde_json::from_str(&fake.canvas()).unwrap();
    let text = canvas["elements"][0]["id"].as_str().unwrap().to_string();
    assert!(calls.contains(&format!("openText {text}")), "{calls:?}");
    assert!(fake.board_status().editing);
}

#[gpui::test]
fn add_arrow_with_two_shapes_selected_binds_them(cx: &mut TestAppContext) {
    let (fake, library, workspace, board, _, vcx) = open_board(cx);
    fake.draw(&scene(&[rect("a", 0.0, 1), rect("b", 400.0, 1)]));
    settle(vcx);
    fake.select(&["a", "b"]);
    vcx.run_until_parked();
    vcx.update(|window, cx| {
        window.dispatch_action(Box::new(innerword_app::workspace::AddArrow), cx)
    });
    vcx.run_until_parked();
    settle(vcx);
    let saved = stored(&library, board, vcx);
    let arrow = saved
        .elements()
        .iter()
        .find(|element| element.json()["type"] == "arrow")
        .unwrap_or_else(|| {
            panic!(
                "an arrow: {:?} {:?}",
                toasts(&workspace, vcx),
                fake.board_calls()
            )
        });
    assert_eq!(arrow.json()["startBinding"]["elementId"], "a");
    assert_eq!(arrow.json()["endBinding"]["elementId"], "b");
    let bound = |id: &str| {
        saved.element(id).unwrap().json()["boundElements"]
            .as_array()
            .is_some_and(|bound| bound.iter().any(|b| b["id"] == arrow.id()))
    };
    assert!(bound("a") && bound("b"), "both shapes list the arrow");
}

#[gpui::test]
fn a_crash_saves_at_once_and_reloads_from_the_saved_scene(cx: &mut TestAppContext) {
    let (fake, library, workspace, board, _, vcx) = open_board(cx);
    fake.draw(&scene(&[rect("a", 0.0, 1)]));
    // No timer: the crash saves what the app holds at once.
    vcx.run_until_parked();
    assert!(
        ids(&stored(&library, board, vcx)).is_empty(),
        "not saved yet"
    );
    fake.crash();
    vcx.run_until_parked();
    assert_eq!(ids(&stored(&library, board, vcx)), ["a"]);
    assert_eq!(fake.reloads(), 1);
    assert!(
        fake.canvas().contains("\"a\""),
        "the view loads the saved scene"
    );
    fake.board_ready();
    vcx.run_until_parked();
    let line = state_line(&workspace, vcx);
    assert!(
        line.contains("state=loaded") && line.contains("dirty=no"),
        "{line}"
    );
}

#[gpui::test]
fn the_first_save_after_an_open_keeps_every_stored_picture(cx: &mut TestAppContext) {
    init(cx);
    let fake = cx.update(pages::use_fake);
    let library = library(cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    let png = innerword_image::fixtures::png(30, 20, [10, 200, 30]);
    let file = library.update(vcx, |l, _| l.put_file(&png).unwrap()).id;
    let hex = file.to_hex();
    let board = make_board(
        &library,
        None,
        "Pictured",
        &stored_scene(&[picture("pic", "f1")], &[("f1", &hex)]),
        &[file],
        vcx,
    );
    workspace.update_in(vcx, |w, window, cx| {
        w.open_document(board, true, window, cx);
    });
    vcx.run_until_parked();
    fake.board_ready();
    vcx.run_until_parked();
    // The page posts the picture's element with no bytes and no `file`.
    fake.draw(&scene(&[picture("pic", "f1"), rect("b", 300.0, 1)]));
    settle(vcx);
    let saved = stored(&library, board, vcx);
    assert_eq!(ids(&saved).len(), 2);
    assert_eq!(
        saved
            .files()
            .get("f1")
            .and_then(|entry| entry.file.clone())
            .map(|f| f.as_str().to_string()),
        Some(hex)
    );
    assert!(
        fake.board_calls()
            .iter()
            .all(|call| !call.starts_with("files"))
    );
}

#[gpui::test]
fn a_picture_the_library_lost_does_not_stop_the_saves(cx: &mut TestAppContext) {
    init(cx);
    let fake = cx.update(pages::use_fake);
    let library = library(cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    let lost = innerword_library::store::FileId::of(b"bytes the library no longer has").to_hex();
    let board = make_board(
        &library,
        None,
        "Lost",
        &stored_scene(&[picture("pic", "f1")], &[("f1", &lost)]),
        &[],
        vcx,
    );
    workspace.update_in(vcx, |w, window, cx| {
        w.open_document(board, true, window, cx);
    });
    vcx.run_until_parked();
    fake.board_ready();
    vcx.run_until_parked();
    fake.draw(&scene(&[picture("pic", "f1"), rect("b", 300.0, 1)]));
    settle(vcx);
    fake.draw(&scene(&[picture("pic", "f1"), rect("b", 350.0, 2)]));
    settle(vcx);
    let saved = stored(&library, board, vcx);
    assert_eq!(saved.element("b").unwrap().x(), 350.0);
    assert_eq!(
        saved
            .files()
            .get("f1")
            .and_then(|entry| entry.file.clone())
            .map(|f| f.as_str().to_string()),
        Some(lost),
        "the entry of the lost picture stays"
    );
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.board_save_label()),
        Some(("Saved".to_string(), false))
    );
}

#[gpui::test]
fn a_whiteboard_changed_while_it_was_closed_opens_fitted(cx: &mut TestAppContext) {
    let (fake, library, workspace, board, text, vcx) = open_board(cx);
    let mut status = fake.board_status();
    status.zoom = 2.0;
    status.scroll_x = -120.0;
    fake.set_status(status);
    fake.draw(&scene(&[rect("a", 0.0, 1)]));
    settle(vcx);
    let reopen = |vcx: &mut VisualTestContext| {
        workspace.update_in(vcx, |w, window, cx| {
            w.open_document(text, true, window, cx);
        });
        vcx.run_until_parked();
        workspace.update_in(vcx, |w, window, cx| {
            w.open_document(board, true, window, cx);
        });
        vcx.run_until_parked();
        let served: Value = serde_json::from_str(&fake.boards_opened().last().unwrap().1).unwrap();
        (
            served["stamp"].as_str().unwrap().to_string(),
            served["view"]["at"].as_str().map(str::to_string),
        )
    };
    let (stamp, at) = reopen(vcx);
    assert_eq!(at.as_deref(), Some(stamp.as_str()), "where the writer was");
    fake.board_ready();
    vcx.run_until_parked();
    // Closed again, then an agent changes it.
    workspace.update_in(vcx, |w, window, cx| {
        w.open_document(text, true, window, cx);
    });
    vcx.run_until_parked();
    outside_write(
        &library,
        board,
        &[rect("a", 0.0, 1), rect("theirs", 300.0, 1)],
        vcx,
    );
    let (stamp, at) = reopen(vcx);
    assert_ne!(
        at.as_deref(),
        Some(stamp.as_str()),
        "fitted: the view is of another scene"
    );
}

#[gpui::test]
fn without_an_engine_a_whiteboard_shows_its_listing_and_why(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    let board = make_board(
        &library,
        None,
        "Plan",
        &stored_scene(&[rect("api", 0.0, 1)], &[]),
        &[],
        vcx,
    );
    workspace.update_in(vcx, |w, window, cx| {
        assert!(w.open_document(board, true, window, cx));
    });
    vcx.run_until_parked();
    let line = state_line(&workspace, vcx);
    assert!(
        line.contains("state=failed:")
            && line.contains(innerword_app::boards::NOT_SHOWN_IN_THIS_RUN),
        "{line}"
    );
    let listing = workspace.read_with(vcx, |w, cx| w.board_listing_text(cx));
    assert!(
        listing.contains("api") && listing.contains("rectangle"),
        "{listing}"
    );
}

#[gpui::test]
fn the_menu_bar_leaves_the_whiteboards_keys_to_it_and_has_them_back_after(cx: &mut TestAppContext) {
    let (_fake, _library, workspace, _board, text, vcx) = open_board(cx);
    let keys = |vcx: &mut VisualTestContext| -> Vec<(String, Option<String>)> {
        vcx.update(|_, cx| {
            let keymap = cx.key_bindings();
            let keymap = keymap.borrow();
            innerword_app::menus::menu_actions(&innerword_app::menus::app_menus())
                .into_iter()
                .map(|(name, action)| {
                    let key = innerword_app::menus::native_menu_binding(&keymap, action.as_ref())
                        .filter(|binding| binding.keystrokes().len() == 1)
                        .map(innerword_app::keymap::keys_of);
                    (name, key)
                })
                .collect()
        })
    };
    let on_a_board = keys(vcx);
    // The Edit menu's six keep theirs: AppKit offers them to the menu only
    // after the page left them, and copy and paste reach Excalidraw only as
    // the `copy:` and `paste:` those items send.
    let kept = ["Undo", "Redo", "Cut", "Copy", "Paste", "Select All"];
    for (name, key) in &on_a_board {
        if let Some(key) = key
            && !kept.contains(&name.as_str())
        {
            assert!(
                !innerword_app::boards::is_whiteboard_keys(key),
                "{name} shows {key}, a key of the whiteboard's"
            );
        }
    }
    for wanted in kept {
        let key = on_a_board
            .iter()
            .find(|(name, _)| name == wanted)
            .and_then(|(_, key)| key.clone());
        assert!(key.is_some(), "{wanted} keeps its key on a whiteboard");
    }
    workspace.update_in(vcx, |w, window, cx| {
        w.open_document(text, true, window, cx);
    });
    vcx.run_until_parked();
    let in_a_text = keys(vcx);
    for wanted in [
        "Show or Hide Comments",
        "Turn Vim Mode On or Off",
        "Next Comment",
        "Undo",
    ] {
        let after = in_a_text
            .iter()
            .find(|(name, _)| name == wanted)
            .and_then(|(_, key)| key.clone())
            .unwrap_or_else(|| panic!("{wanted} has a key in a text"));
        let before = on_a_board
            .iter()
            .find(|(name, _)| name == wanted)
            .and_then(|(_, key)| key.clone());
        if innerword_app::boards::is_whiteboard_keys(&after) && wanted != "Undo" {
            assert_eq!(
                before, None,
                "{wanted} ({after}) shows no key on a whiteboard"
            );
        } else {
            assert_eq!(before, Some(after), "{wanted}");
        }
    }
}

/// A folder "Part one" with a chapter, open, and the whiteboards
/// `boards` (title, elements) in it, made by the store.
fn folder_with_boards<'a>(
    engine: bool,
    boards: &[(&str, Vec<Value>)],
    cx: &'a mut TestAppContext,
) -> (
    Option<Rc<FakePages>>,
    Entity<Library>,
    Entity<Workspace>,
    innerword_library::FolderId,
    Vec<DocumentId>,
    &'a mut VisualTestContext,
) {
    init(cx);
    let fake = engine.then(|| cx.update(pages::use_fake));
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
    let ids = boards
        .iter()
        .map(|(title, elements)| {
            make_board(
                &library,
                Some(folder),
                title,
                &stored_scene(elements, &[]),
                &[],
                vcx,
            )
        })
        .collect();
    workspace.update_in(vcx, |w, window, cx| {
        w.open_document(text, false, window, cx);
    });
    vcx.run_until_parked();
    (fake, library, workspace, folder, ids, vcx)
}

/// Chooses the folder's PDF and `chosen` as its file; lets it run.
fn export_folder_pdf(
    workspace: &Entity<Workspace>,
    folder: innerword_library::FolderId,
    chosen: &std::path::Path,
    vcx: &mut VisualTestContext,
) {
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
        let chosen = chosen.to_path_buf();
        move |_| Some(chosen)
    });
    vcx.run_until_parked();
}

fn run_export(workspace: &Entity<Workspace>, vcx: &mut VisualTestContext) {
    for _ in 0..400 {
        vcx.run_until_parked();
        if workspace.read_with(vcx, |w, _| w.pdf_exporting()).is_none() {
            break;
        }
        vcx.executor().advance_clock(Duration::from_millis(5));
    }
}

#[gpui::test]
fn a_folders_pdf_collects_the_open_whiteboard_first(cx: &mut TestAppContext) {
    let (fake, library, workspace, folder, boards, vcx) =
        folder_with_boards(true, &[("Drawn", vec![rect("a", 0.0, 1)])], cx);
    let fake = fake.unwrap();
    workspace.update_in(vcx, |w, window, cx| {
        w.open_document(boards[0], true, window, cx);
    });
    vcx.run_until_parked();
    fake.board_ready();
    fake.draw_unposted(&scene(&[rect("a", 0.0, 1), rect("b", 300.0, 1)]));
    vcx.run_until_parked();
    let out = tempfile::tempdir().unwrap();
    let chosen = out.path().join("Part one.pdf");
    export_folder_pdf(&workspace, folder, &chosen, vcx);
    run_export(&workspace, vcx);
    assert!(fake.collects_asked() >= 1);
    assert_eq!(ids(&stored(&library, boards[0], vcx)), ["a", "b"]);
    let pdf = innerword_print::PdfFile::open(&std::fs::read(&chosen).unwrap()).unwrap();
    assert_eq!(pdf.page_count(), 2);
    let last = pdf.page_size(1);
    // Both rectangles (400 x 50) and the margin: the unposted one is in it.
    assert_eq!((last.0.round(), last.1.round()), (464.0, 114.0));
}

#[gpui::test]
fn a_folders_pdf_whose_whiteboard_fails_writes_nothing(cx: &mut TestAppContext) {
    let (fake, _library, workspace, folder, _boards, vcx) =
        folder_with_boards(true, &[("Drawn", vec![rect("a", 0.0, 1)])], cx);
    fake.unwrap().fail_board_pdfs(true);
    let out = tempfile::tempdir().unwrap();
    let chosen = out.path().join("Part one.pdf");
    export_folder_pdf(&workspace, folder, &chosen, vcx);
    run_export(&workspace, vcx);
    assert!(!chosen.exists(), "nothing was written");
    let report = workspace.update(vcx, |w, _| w.take_pdf_export_report());
    assert!(
        matches!(&report, Some(Err(why)) if why.contains("\u{201c}Drawn\u{201d} could not be made")),
        "{report:?}"
    );
}

#[gpui::test]
fn cancel_between_two_whiteboards_of_a_folders_pdf_writes_nothing(cx: &mut TestAppContext) {
    let (fake, _library, workspace, folder, _boards, vcx) = folder_with_boards(
        true,
        &[
            ("One", vec![rect("a", 0.0, 1)]),
            ("Two", vec![rect("b", 0.0, 1)]),
        ],
        cx,
    );
    let fake = fake.unwrap();
    let out = tempfile::tempdir().unwrap();
    let chosen = out.path().join("Part one.pdf");
    export_folder_pdf(&workspace, folder, &chosen, vcx);
    // The chapter's pages, then the first whiteboard: stop between the two.
    for _ in 0..400 {
        if fake.board_pdfs_made() >= 1 {
            break;
        }
        vcx.executor().advance_clock(Duration::from_millis(1));
        vcx.run_until_parked();
    }
    assert_eq!(fake.board_pdfs_made(), 1, "between the two whiteboards");
    vcx.update(|window, cx| {
        window.dispatch_action(Box::new(innerword_app::actions::CancelPdfExport), cx)
    });
    run_export(&workspace, vcx);
    assert_eq!(fake.board_pdfs_made(), 1, "the second is not made");
    assert!(!chosen.exists(), "nothing was written");
}

#[gpui::test]
fn without_an_engine_a_folders_pdf_leaves_its_whiteboards_out_and_says_why(
    cx: &mut TestAppContext,
) {
    let (_fake, _library, workspace, folder, _boards, vcx) =
        folder_with_boards(false, &[("Drawn", vec![rect("a", 0.0, 1)])], cx);
    let out = tempfile::tempdir().unwrap();
    let chosen = out.path().join("Part one.pdf");
    export_folder_pdf(&workspace, folder, &chosen, vcx);
    run_export(&workspace, vcx);
    let pdf = innerword_print::PdfFile::open(&std::fs::read(&chosen).unwrap()).unwrap();
    assert_eq!(pdf.page_count(), 1, "the chapter alone");
    let said = toasts(&workspace, vcx);
    assert!(
        said.iter().any(
            |t| t.contains("1 whiteboard was left out: whiteboards are not shown in this run.")
        ),
        "{said:?}"
    );
}

#[gpui::test]
fn an_add_command_asks_the_page_first_when_it_holds_more(cx: &mut TestAppContext) {
    let (fake, _library, _workspace, _board, _, vcx) = open_board(cx);
    fake.draw(&scene(&[rect("a", 0.0, 1), rect("b", 400.0, 1)]));
    settle(vcx);
    // The writer moved a shape; the page has not posted it yet.
    fake.draw_unposted(&scene(&[rect("a", 50.0, 2), rect("b", 400.0, 1)]));
    fake.select(&["a", "b"]);
    vcx.run_until_parked();
    vcx.update(|window, cx| {
        window.dispatch_action(Box::new(innerword_app::workspace::AddArrow), cx)
    });
    vcx.run_until_parked();
    let calls = fake.board_calls();
    let collect = calls
        .iter()
        .rposition(|call| call == "collect")
        .expect("a collect");
    let center = calls
        .iter()
        .rposition(|call| call == "center")
        .expect("center");
    assert!(collect < center, "{calls:?}");
    // The shape the arrow binds keeps the writer's move.
    let canvas: Value = serde_json::from_str(&fake.canvas()).unwrap();
    let a = canvas["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|element| element["id"] == "a")
        .unwrap();
    assert_eq!(a["x"], 50.0);
}

// ----- what the final review found --------------------------------------------------------

#[test]
fn a_percent_before_a_character_of_several_bytes_is_kept_as_it_is() {
    use innerword_app::boards::data_url_bytes;
    assert_eq!(
        data_url_bytes("data:image/png,%€"),
        Some("%€".as_bytes().to_vec())
    );
    assert_eq!(
        data_url_bytes("data:image/png,%a€"),
        Some("%a€".as_bytes().to_vec())
    );
    assert_eq!(
        data_url_bytes("data:text/plain,%41%2"),
        Some(b"A%2".to_vec())
    );
    assert_eq!(data_url_bytes("data:text/plain,%4"), Some(b"%4".to_vec()));
}

#[test]
fn the_engines_sentence_is_worded_for_whiteboards() {
    use innerword_app::boards::whiteboards_sentence;
    assert_eq!(
        whiteboards_sentence("Pages need macOS 12.3 or later."),
        "Whiteboards need macOS 12.3 or later."
    );
    assert_eq!(
        whiteboards_sentence(
            "This macOS does not have what pages need to stay offline (_setPeerConnectionEnabled:)"
        ),
        "This macOS does not have what whiteboards need (_setPeerConnectionEnabled:)"
    );
}

/// From now on the saves of whiteboards fail (or no longer do).
fn saves_fail(fail: bool, cx: &mut VisualTestContext) {
    cx.update(|_, cx| innerword_app::boards::fail_board_saves(cx, fail));
}

/// What a library in memory kept of a drawing that could not be saved.
fn recovered_json(recovered: Option<Recovered>) -> String {
    match recovered {
        Some(Recovered::Memory(json)) => json,
        other => panic!("no copy in memory: {other:?}"),
    }
}

#[gpui::test]
fn closing_the_window_while_the_save_fails_keeps_the_drawing_in_the_recovery_copy(
    cx: &mut TestAppContext,
) {
    let (fake, _library, workspace, _board, _, vcx) = open_board(cx);
    saves_fail(true, vcx);
    fake.draw(&scene(&[rect("kept", 0.0, 1)]));
    settle(vcx);
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.board_recovered()),
        None,
        "one failed save writes no copy yet"
    );
    vcx.simulate_keystrokes("cmd-w");
    vcx.run_until_parked();
    let recovered = workspace.read_with(cx, |w, _| w.board_recovered());
    assert!(recovered_json(recovered).contains("kept"));
}

#[gpui::test]
fn quitting_while_the_save_fails_keeps_the_drawing_in_the_recovery_copy(cx: &mut TestAppContext) {
    let (fake, _library, workspace, _board, _, vcx) = open_board(cx);
    saves_fail(true, vcx);
    // Posted, its save not yet due: the quit hook's flush is the first try.
    fake.draw(&scene(&[rect("kept", 0.0, 1)]));
    vcx.run_until_parked();
    cx.quit();
    cx.run_until_parked();
    let recovered = workspace.read_with(cx, |w, _| w.board_recovered());
    assert!(recovered_json(recovered).contains("kept"));
}

#[gpui::test]
fn reload_whiteboard_while_the_save_fails_keeps_the_view_and_the_drawing(cx: &mut TestAppContext) {
    let (fake, library, workspace, board, _, vcx) = open_board(cx);
    saves_fail(true, vcx);
    fake.draw(&scene(&[rect("kept", 0.0, 1)]));
    settle(vcx);
    vcx.update(|window, cx| window.dispatch_action(Box::new(ReloadPage), cx));
    vcx.run_until_parked();
    assert_eq!(fake.reloads(), 0, "the view is not loaded over the drawing");
    assert!(
        toasts(&workspace, vcx).iter().any(|t| t == NOT_RELOADED),
        "{:?}",
        toasts(&workspace, vcx)
    );
    assert!(state_line(&workspace, vcx).contains("dirty=yes"));
    // The library comes back: the retry saves it.
    saves_fail(false, vcx);
    vcx.executor()
        .advance_clock(innerword_app::workspace::RETRY_DELAY + Duration::from_millis(200));
    vcx.run_until_parked();
    assert_eq!(ids(&stored(&library, board, vcx)), ["kept"]);
}

/// The recovery files of a library on disk, each one's text.
fn recovery_files(dir: &std::path::Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir.join("Recovered")) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
        .collect()
}

#[gpui::test]
fn a_newer_drawing_goes_to_the_recovery_file_after_an_earlier_copy(cx: &mut TestAppContext) {
    init(cx);
    let fake = cx.update(pages::use_fake);
    let dir = tempfile::tempdir().unwrap();
    let library = cx.new(|_| Library::open(dir.path().join("library.db")).unwrap());
    library.update(cx, |library, cx| {
        library
            .set_setting(innerword_app::settings::keys::WELCOME_CREATED, &true, cx)
            .unwrap();
    });
    let text = add_document(&library, None, &simple_document("Notes", "Words."), cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    let board = workspace
        .update_in(vcx, |w, window, cx| w.new_board(Some("Plan"), window, cx))
        .unwrap();
    vcx.run_until_parked();
    fake.board_ready();
    vcx.run_until_parked();
    // Three failed saves in a row: the first drawing goes to a copy.
    saves_fail(true, vcx);
    fake.draw(&scene(&[rect("early", 0.0, 1)]));
    settle(vcx);
    for _ in 0..2 {
        vcx.executor()
            .advance_clock(innerword_app::workspace::RETRY_DELAY + Duration::from_millis(200));
        vcx.run_until_parked();
    }
    let files = recovery_files(dir.path());
    assert_eq!(files.len(), 1, "{files:?}");
    assert!(files[0].contains("early"));
    // The library comes back and saves it.
    saves_fail(false, vcx);
    vcx.executor()
        .advance_clock(innerword_app::workspace::RETRY_DELAY + Duration::from_millis(200));
    vcx.run_until_parked();
    assert_eq!(ids(&stored(&library, board, vcx)), ["early"]);
    // A later stroke whose last save fails as the writer leaves: it is
    // written too, beside the earlier copy.
    saves_fail(true, vcx);
    fake.draw(&scene(&[rect("early", 0.0, 1), rect("late", 200.0, 1)]));
    vcx.run_until_parked();
    workspace.update_in(vcx, |w, window, cx| {
        w.open_document(text, true, window, cx);
    });
    vcx.run_until_parked();
    let files = recovery_files(dir.path());
    assert!(
        files.iter().any(|file| file.contains("late")),
        "the newer drawing has a copy: {files:?}"
    );
}

#[gpui::test]
fn a_shape_library_change_the_page_has_not_posted_is_kept_when_another_item_opens(
    cx: &mut TestAppContext,
) {
    let (fake, library, workspace, _board, text, vcx) = open_board(cx);
    let focus = workspace.read_with(vcx, |w, _| w.page_focus_handle().unwrap());
    vcx.update(|window, _| window.focus(&focus));
    // Add selection to shape library: the page posts the change 300 ms
    // later; no change of the scene is outstanding.
    vcx.update(|window, cx| window.dispatch_action(Box::new(AddToShapeLibrary), cx));
    vcx.run_until_parked();
    assert!(library_items(&library, vcx).is_empty(), "not posted yet");
    workspace.update_in(vcx, |w, window, cx| {
        w.open_document(text, true, window, cx);
    });
    vcx.run_until_parked();
    assert!(fake.collects_asked() >= 1, "the page was asked for it");
    assert_eq!(library_items(&library, vcx), ["added-1"]);
}

#[gpui::test]
fn the_commands_that_open_something_in_the_page_give_it_the_keyboard(cx: &mut TestAppContext) {
    let (_fake, _library, workspace, _board, _, vcx) = open_board(cx);
    for action in [
        Box::new(AddRectangle) as Box<dyn gpui::Action>,
        Box::new(innerword_app::workspace::AddText),
        Box::new(innerword_editor_tools::actions::Find),
        Box::new(MermaidToDrawing),
        Box::new(ToggleShapeLibrary),
    ] {
        let name = action.name();
        // The keyboard in the library.
        vcx.simulate_keystrokes("cmd-0");
        vcx.run_until_parked();
        assert!(
            state_line(&workspace, vcx).contains("focus=elsewhere"),
            "{name}: {}",
            state_line(&workspace, vcx)
        );
        vcx.update(|window, cx| window.dispatch_action(action, cx));
        vcx.run_until_parked();
        vcx.executor().advance_clock(Duration::from_secs(3));
        vcx.run_until_parked();
        let line = state_line(&workspace, vcx);
        assert!(line.contains("focus=board"), "{name}: {line}");
    }
}

#[gpui::test]
fn a_whiteboard_that_is_not_drawn_offers_nothing_that_draws(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    let board = make_board(
        &library,
        None,
        "Plan",
        &stored_scene(&[rect("api", 0.0, 1), rect("db", 200.0, 1)], &[]),
        &[],
        vcx,
    );
    workspace.update_in(vcx, |w, window, cx| {
        assert!(w.open_document(board, true, window, cx));
    });
    vcx.run_until_parked();
    // Its listing's count, no zoom, and the menu bar keeps every key.
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.board_elements_label()),
        Some("2 elements".to_string())
    );
    assert_eq!(
        vcx.update(|_, cx| innerword_app::menus::open_item(cx)),
        innerword_app::menus::OpenItemKind::Other
    );
    vcx.update(|window, cx| window.dispatch_action(Box::new(AddRectangle), cx));
    vcx.run_until_parked();
    let said = toasts(&workspace, vcx);
    assert!(
        said.iter()
            .any(|t| t == innerword_app::boards::NOT_SHOWN_IN_THIS_RUN),
        "{said:?}"
    );
    assert!(
        !said
            .iter()
            .any(|t| t == innerword_app::workspace::STILL_LOADING),
        "{said:?}"
    );
}

#[gpui::test]
fn opening_another_item_after_reopening_a_closing_whiteboard_keeps_that_item(
    cx: &mut TestAppContext,
) {
    let (fake, library, workspace, board, text, vcx) = open_board(cx);
    let form = innerword_app::document_io::StoredForm::of(&simple_document("Other", "More."), None);
    let other = library.update(vcx, |l, cx| {
        l.create_document(None, &form.content(), cx).unwrap().id
    });
    fake.hold_collects();
    fake.draw_unposted(&scene(&[rect("late", 0.0, 1)]));
    vcx.run_until_parked();
    workspace.update_in(vcx, |w, window, cx| {
        w.open_document(text, true, window, cx);
        // Opened again while it is closing: it would open once saved...
        w.open_document(board, true, window, cx);
        // ...but the writer went on to another item.
        w.open_document(other, true, window, cx);
    });
    vcx.run_until_parked();
    fake.release_collects();
    settle(vcx);
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.open_document_id()),
        Some(other)
    );
    assert_eq!(workspace.read_with(vcx, |w, _| w.open_board_id()), None);
}

// ----- the assistant's drawings (docs/assistant-draws-design.md, section 5) ------------
//
// The side chat on the fake engine of the assistant (`FakeBackend`: no test runs
// Claude Code), the whiteboard on the fake page engine.

mod drawings {
    use std::{rc::Rc, sync::Arc};

    use gpui::TestAppContext;
    use innerword_app::{
        assistant::conversations,
        boards::fail_board_saves,
        pages::{self, FakePages},
        workspace::{
            ASSISTANT_DRAWING_LABEL, BOARD_BUSY, CHANGES_NOTHING, CLOSED_BEFORE_DRAWN, NOT_UNDONE,
            ONLY_GONE, PUT_BESIDE, PUT_IN_VIEW, SHOWN_IN_THE_SIDE_CHAT, STILL_LOADING,
            conflicts_sentence, drew_sentence,
        },
    };
    use innerword_assistant::{FakeBackend, MessageId};
    use innerword_assistant_ui::{AssistantPanel, DRAWN_ALREADY};
    use serde_json::{Value, json};

    use super::{ids, scene, settle, stored, toasts};
    use crate::suite::*;

    /// A shape as Excalidraw keeps it at `(x, y)`, `width` x `height`, with
    /// its label when `label` is given.
    pub(super) fn shape(
        kind: &str,
        id: &str,
        at: (f64, f64, f64, f64),
        index: &str,
        label: Option<&str>,
    ) -> Vec<Value> {
        let (x, y, width, height) = at;
        let mut shape = json!({
            "id": id, "type": kind, "x": x, "y": y, "width": width, "height": height,
            "angle": 0, "strokeColor": "#1e1e1e", "backgroundColor": "transparent",
            "fillStyle": "solid", "strokeWidth": 2, "strokeStyle": "solid", "roughness": 1,
            "opacity": 100, "groupIds": [], "frameId": null, "roundness": null,
            "seed": 1, "version": 1, "versionNonce": 3, "isDeleted": false,
            "boundElements": null, "updated": 1, "link": null, "locked": false,
            "index": index,
        });
        let Some(text) = label else {
            return vec![shape];
        };
        let label_id = format!("{id}-label");
        shape["boundElements"] = json!([{ "type": "text", "id": label_id }]);
        let label = json!({
            "id": label_id, "type": "text", "x": x + 10.0, "y": y + height / 2.0 - 12.5,
            "width": width - 20.0, "height": 25.0, "angle": 0, "strokeColor": "#1e1e1e",
            "backgroundColor": "transparent", "fillStyle": "solid", "strokeWidth": 2,
            "strokeStyle": "solid", "roughness": 1, "opacity": 100, "groupIds": [],
            "frameId": null, "roundness": null, "seed": 2, "version": 1, "versionNonce": 5,
            "isDeleted": false, "boundElements": null, "updated": 1, "link": null,
            "locked": false, "index": format!("{index}V"), "text": text, "originalText": text,
            "fontSize": 20, "fontFamily": 5, "textAlign": "center", "verticalAlign": "middle",
            "containerId": id, "autoResize": true, "lineHeight": 1.25,
        });
        vec![shape, label]
    }

    /// The rectangle "API" the examples draw beside.
    fn api() -> Vec<Value> {
        shape(
            "rectangle",
            "api",
            (100.0, 80.0, 200.0, 90.0),
            "a0",
            Some("API"),
        )
    }

    /// An answer that holds `drawing` as its block.
    pub(super) fn answer_with(drawing: &str) -> String {
        format!("Here is the drawing.\n\n```whiteboard\n{drawing}\n```\n")
    }

    /// The design's example: a database the API reads from, the API blue.
    pub(super) const DB_AND_BLUE: &str = r##"{"add": [
  {"type": "ellipse", "id": "db", "x": 420, "y": 80, "width": 160, "height": 90, "label": {"text": "Postgres"}},
  {"type": "arrow", "id": "reads", "x": 300, "y": 125, "start": {"id": "api"}, "end": {"id": "db"}, "label": {"text": "reads"}}],
 "update": [{"id": "api", "backgroundColor": "#a5d8ff", "fillStyle": "solid"}]}"##;

    pub(super) struct Desk<'a> {
        pub fake: FakeBackend,
        pub pages: Rc<FakePages>,
        pub library: Entity<Library>,
        pub workspace: Entity<Workspace>,
        pub board: DocumentId,
        pub text: DocumentId,
        pub cx: &'a mut VisualTestContext,
    }

    /// A workspace with the assistant on its fake engine and the whiteboard
    /// "Plan" open and ready, the rectangle "API" drawn and saved; a text
    /// "Notes" beside it.
    pub(super) fn desk(cx: &mut TestAppContext) -> Desk<'_> {
        init(cx);
        let fake = FakeBackend::new();
        cx.update(|cx| innerword_app::assistant::set_backend(Arc::new(fake.clone()), cx));
        let pages = cx.update(pages::use_fake);
        let library = library(cx);
        let text = add_document(&library, None, &simple_document("Notes", "Words."), cx);
        let (workspace, vcx) = open_workspace(&library, cx);
        vcx.run_until_parked();
        let board = workspace
            .update_in(vcx, |w, window, cx| w.new_board(Some("Plan"), window, cx))
            .expect("a whiteboard is made");
        vcx.run_until_parked();
        pages.board_ready();
        pages.draw(&scene(&api()));
        settle(vcx);
        Desk {
            fake,
            pages,
            library,
            workspace,
            board,
            text,
            cx: vcx,
        }
    }

    impl Desk<'_> {
        pub fn side(&mut self) -> Entity<AssistantPanel> {
            self.workspace
                .read_with(self.cx, |w, _| w.side_chat().expect("an engine").clone())
        }

        /// Goes to the side chat (`cmd-3`), asks, and lets `answer` come.
        pub fn ask(&mut self, answer: &str) -> MessageId {
            self.cx.simulate_keystrokes("cmd-3");
            self.cx.run_until_parked();
            let panel = self.side();
            self.fake.reply(answer);
            panel.update_in(self.cx, |panel, window, cx| {
                panel.send("Draw it, please.", window, cx)
            });
            self.cx.run_until_parked();
            panel.read_with(self.cx, |panel, _| {
                panel.transcript().last_answer().expect("an answer").id
            })
        }

        pub fn on_canvas(&mut self) -> bool {
            self.workspace
                .update_in(self.cx, |w, window, _| w.page_has_focus(window))
        }

        pub fn in_chat(&mut self) -> bool {
            self.workspace.update_in(self.cx, |w, window, cx| {
                w.assistant_panel_has_focus(window, cx)
            })
        }

        pub fn toasts(&mut self) -> Vec<String> {
            toasts(&self.workspace, self.cx)
        }

        pub fn has_toast(&mut self, words: &str) -> bool {
            self.toasts().iter().any(|toast| toast == words)
        }

        pub fn toast_starting(&mut self, start: &str) -> Option<String> {
            self.toasts()
                .into_iter()
                .find(|toast| toast.starts_with(start))
        }

        /// Presses the button of the toast that starts with `start`.
        pub fn press_toast(&mut self, start: &str) {
            let layer = self
                .workspace
                .read_with(self.cx, |workspace, _| workspace.toast_layer().clone());
            let id = layer.read_with(self.cx, |layer, _| {
                layer
                    .ids()
                    .into_iter()
                    .zip(layer.toasts())
                    .find(|(_, toast)| toast.message.starts_with(start))
                    .map(|(id, _)| id)
                    .unwrap_or_else(|| panic!("no toast says {start:?}"))
            });
            let ran = layer.update_in(self.cx, |layer, window, cx| {
                layer.run_action(id, window, cx)
            });
            assert!(ran, "the toast {start:?} has no button");
            self.cx.run_until_parked();
        }

        pub fn versions_before_drawings(&mut self) -> usize {
            let board = self.board;
            self.library.read_with(self.cx, |library, _| {
                library
                    .store()
                    .snapshot_labels(board)
                    .unwrap()
                    .into_values()
                    .filter(|label| label == ASSISTANT_DRAWING_LABEL)
                    .count()
            })
        }

        pub fn drawn(&mut self, answer: MessageId) -> bool {
            self.side().read_with(self.cx, |panel, _| {
                panel
                    .transcript()
                    .message(answer)
                    .is_some_and(|message| message.drawn_at.is_some())
            })
        }

        pub fn stored_ids(&mut self) -> Vec<String> {
            ids(&stored(&self.library, self.board, self.cx))
        }

        /// The fake canvas's elements by id.
        pub fn canvas_ids(&mut self) -> Vec<String> {
            let canvas: Value = serde_json::from_str(&self.pages.canvas()).unwrap();
            canvas["elements"]
                .as_array()
                .unwrap()
                .iter()
                .map(|element| element["id"].as_str().unwrap().to_string())
                .collect()
        }

        /// The text the side chat shows for answer `id`.
        pub fn shown(&mut self, id: MessageId) -> String {
            let panel = self.side();
            panel.read_with(self.cx, |panel, cx| {
                panel
                    .messages()
                    .read(cx)
                    .answer_editor(id)
                    .expect("the answer is shown")
                    .read(cx)
                    .document()
                    .plain_text()
            })
        }
    }

    /// Clicks the control drawn under `selector`.
    fn click(selector: &str, cx: &mut VisualTestContext) {
        let selector: &'static str = Box::leak(selector.to_string().into_boxed_str());
        let bounds = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("nothing is drawn as {selector:?}"));
        cx.simulate_click(bounds.center(), gpui::Modifiers::none());
        cx.run_until_parked();
    }

    #[gpui::test]
    fn the_assistant_draws_on_the_open_whiteboard_in_one_step_of_its_undo(cx: &mut TestAppContext) {
        let mut desk = desk(cx);
        let answer = desk.ask(&answer_with(DB_AND_BLUE));
        assert!(desk.in_chat());
        // The line, never the JSON.
        let shown = desk.shown(answer);
        assert!(
            shown.contains("Drawing: adds an ellipse and an arrow; changes the rectangle"),
            "{shown}"
        );
        assert!(!shown.contains("\"type\""), "{shown}");

        // The button under the answer.
        click(&format!("assistant-action-draw-{}", answer.0), desk.cx);
        let adopted = desk.pages.adopted();
        assert_eq!(adopted.len(), 1, "one adopt");
        assert!(adopted[0].contains("\"undoable\":true"), "{}", adopted[0]);
        // The library has it, and the whiteboard as it was as a version.
        let stored_ids = desk.stored_ids();
        for id in ["api", "db", "reads"] {
            assert!(stored_ids.contains(&id.to_string()), "{stored_ids:?}");
        }
        let scene = stored(&desk.library, desk.board, desk.cx);
        assert_eq!(
            scene.element("api").unwrap().json()["backgroundColor"],
            json!("#a5d8ff")
        );
        assert_eq!(desk.versions_before_drawings(), 1);
        // The canvas took it in: the keyboard is on it, the toast says so.
        assert!(desk.on_canvas(), "the keyboard went to the canvas");
        assert!(
            desk.has_toast(&drew_sentence("Plan", 2, 1, 0)),
            "{:?}",
            desk.toasts()
        );
        // The answer is marked drawn, also in the stored conversation.
        assert!(desk.drawn(answer));
        let stored_answer = desk.library.read_with(desk.cx, |library, _| {
            conversations::load(library, desk.board)
                .last_answer()
                .cloned()
                .unwrap()
        });
        assert!(stored_answer.drawn_at.is_some());
        let report = desk
            .workspace
            .read_with(desk.cx, |w, _| w.last_drawing_report().map(str::to_string))
            .unwrap();
        assert!(
            report.starts_with("draw: 2 added, 1 changed, 0 removed, version kept"),
            "{report}"
        );
        assert!(
            report.ends_with("keyboard to the canvas after the adopt answered"),
            "{report}"
        );

        // The Edit menu's Undo on the canvas: one step takes it all out.
        desk.cx.update(|window, cx| {
            window.dispatch_action(Box::new(innerword_editor::actions::Undo), cx)
        });
        desk.cx.run_until_parked();
        assert!(
            desk.pages.board_calls().contains(&"undo 1".to_string()),
            "{:?}",
            desk.pages.board_calls()
        );
        let canvas = desk.canvas_ids();
        let deleted: Value = serde_json::from_str(&desk.pages.canvas()).unwrap();
        assert!(!canvas.contains(&"db".to_string()), "{canvas:?} {deleted}");
        settle(desk.cx);
        assert_eq!(desk.stored_ids(), ["api", "api-label"]);
    }

    #[gpui::test]
    fn the_keyboard_moves_only_when_the_canvas_has_the_drawing(cx: &mut TestAppContext) {
        let mut desk = desk(cx);
        desk.ask(&answer_with(DB_AND_BLUE));
        desk.pages.hold_adopts();
        desk.cx.simulate_keystrokes("cmd-shift-d");
        desk.cx.run_until_parked();
        assert_eq!(desk.pages.adopted().len(), 1, "the adopt was sent");
        // Not answered yet: the keyboard is still in the side chat, and its
        // cmd-z is the composer's, not the canvas's.
        assert!(desk.in_chat());
        desk.cx.simulate_keystrokes("cmd-z");
        desk.cx.run_until_parked();
        assert!(
            !desk
                .pages
                .board_calls()
                .iter()
                .any(|call| call.starts_with("undo")),
            "{:?}",
            desk.pages.board_calls()
        );
        assert!(desk.toast_starting("Drew on").is_none());
        desk.pages.release_adopts();
        desk.cx.run_until_parked();
        assert!(desk.on_canvas());
        assert!(desk.toast_starting("Drew on").is_some());

        // A second drawing; the writer goes to the library before the canvas
        // answers, and the keyboard stays there.
        desk.ask(&answer_with(
            r#"{"add": [{"type": "text", "id": "note", "x": 100, "y": 300, "text": "Reads only."}]}"#,
        ));
        desk.pages.hold_adopts();
        desk.cx.simulate_keystrokes("cmd-shift-d");
        desk.cx.run_until_parked();
        desk.cx.simulate_keystrokes("cmd-0");
        desk.cx.run_until_parked();
        desk.pages.release_adopts();
        desk.cx.run_until_parked();
        assert!(!desk.on_canvas());
        assert!(!desk.in_chat());
        assert!(desk.stored_ids().contains(&"note".to_string()));
    }

    #[gpui::test]
    fn a_drawing_waits_for_what_the_page_holds(cx: &mut TestAppContext) {
        let mut desk = desk(cx);
        desk.ask(&answer_with(DB_AND_BLUE));
        // A stroke the page has not posted.
        let mut elements = api();
        elements.extend(shape(
            "rectangle",
            "late",
            (100.0, 400.0, 100.0, 50.0),
            "a1",
            None,
        ));
        desk.pages.draw_unposted(&scene(&elements));
        desk.cx.run_until_parked();
        let asked = desk.pages.collects_asked();
        desk.cx.simulate_keystrokes("cmd-shift-d");
        desk.cx.run_until_parked();
        assert!(
            desk.pages.collects_asked() > asked,
            "the page was asked first"
        );
        let stored_ids = desk.stored_ids();
        assert!(stored_ids.contains(&"late".to_string()), "{stored_ids:?}");
        assert!(stored_ids.contains(&"db".to_string()), "{stored_ids:?}");
        // The writer's stroke is on the canvas beside the drawing.
        let canvas = desk.canvas_ids();
        assert!(canvas.contains(&"late".to_string()) && canvas.contains(&"db".to_string()));
    }

    #[gpui::test]
    fn two_presses_draw_once(cx: &mut TestAppContext) {
        let mut desk = desk(cx);
        let answer = desk.ask(&answer_with(DB_AND_BLUE));
        desk.pages.hold_collects();
        let mut elements = api();
        elements.extend(shape(
            "rectangle",
            "late",
            (100.0, 400.0, 100.0, 50.0),
            "a1",
            None,
        ));
        desk.pages.draw_unposted(&scene(&elements));
        desk.cx.run_until_parked();
        // The button, then the key before the page answered.
        click(&format!("assistant-action-draw-{}", answer.0), desk.cx);
        desk.cx.simulate_keystrokes("cmd-shift-d");
        desk.cx.run_until_parked();
        assert!(desk.has_toast(BOARD_BUSY), "{:?}", desk.toasts());
        desk.pages.release_collects();
        desk.cx.run_until_parked();
        assert_eq!(desk.pages.adopted().len(), 1);
        assert_eq!(desk.versions_before_drawings(), 1);
    }

    #[gpui::test]
    fn an_outside_change_during_the_collect_is_waited_for(cx: &mut TestAppContext) {
        let mut desk = desk(cx);
        desk.ask(&answer_with(DB_AND_BLUE));
        desk.pages.hold_collects();
        let mut elements = api();
        elements.extend(shape(
            "rectangle",
            "late",
            (100.0, 400.0, 100.0, 50.0),
            "a1",
            None,
        ));
        desk.pages.draw_unposted(&scene(&elements));
        desk.cx.run_until_parked();
        desk.cx.simulate_keystrokes("cmd-shift-d");
        desk.cx.run_until_parked();
        // Meanwhile a write from outside (an agent's, a restore) starts its own
        // adopt, which the view has not answered when the page hands over.
        desk.pages.hold_adopts();
        let board = desk.board;
        let content = desk
            .library
            .read_with(desk.cx, |l, _| l.document(board).unwrap().content);
        let mut theirs = innerword_board::Scene::from_stored(&content).unwrap().scene;
        let mut making = innerword_board::HashedMaking::new(&[b"outside".as_slice()], 1);
        let skeleton = json!({"type": "rectangle", "id": "agent", "x": 0, "y": 700});
        innerword_board::add(
            &mut theirs,
            &[innerword_board::Skeleton(
                skeleton.as_object().unwrap().clone(),
            )],
            &innerword_board::Pictures::new(),
            &mut making,
        )
        .unwrap();
        let written = theirs.to_stored();
        let text = theirs.text();
        let (_, told) = desk.library.update(desk.cx, |library, cx| {
            library.change_outside(
                |store| {
                    let content = innerword_library::store::BoardContent {
                        scene: &written,
                        files: &[],
                        plain_text: &text,
                        word_count: 0,
                        char_count: 0,
                    };
                    store.save_board(board, &content, None).unwrap();
                    (
                        (),
                        vec![innerword_library::OutsideChange::DocumentSaved(board)],
                    )
                },
                cx,
            )
        });
        told.unwrap();
        desk.workspace
            .update(desk.cx, |w, cx| w.adopt_outside_write(board, cx));
        desk.cx.run_until_parked();
        let outside = desk.pages.adopted().len();
        assert_eq!(outside, 1, "the outside write's adopt");
        desk.pages.release_collects();
        desk.cx.run_until_parked();
        assert!(desk.has_toast(BOARD_BUSY), "{:?}", desk.toasts());
        assert_eq!(desk.pages.adopted().len(), outside, "the drawing sent none");
        assert_eq!(desk.versions_before_drawings(), 0);
        desk.pages.release_adopts();
        desk.cx.run_until_parked();
    }

    #[gpui::test]
    fn a_drawing_that_changes_what_is_gone_draws_nothing(cx: &mut TestAppContext) {
        let mut desk = desk(cx);
        let answer = desk.ask(&answer_with(
            r##"{"update": [{"id": "cache", "backgroundColor": "#ffc9c9"}],
                 "add": [{"type": "arrow", "id": "to", "x": 0, "y": 0, "start": {"id": "api"}, "end": {"id": "queue"}}]}"##,
        ));
        desk.cx.simulate_keystrokes("cmd-shift-d");
        desk.cx.run_until_parked();
        let said = desk.toast_starting("Nothing was drawn").expect("a toast");
        assert_eq!(
            said,
            "Nothing was drawn: the answer changes cache, which is no longer on the whiteboard; \
             the answer joins arrows or frames to queue, which is no longer on the whiteboard. \
             Ask again: the assistant is sent the whiteboard as it is now."
        );
        assert!(desk.pages.adopted().is_empty());
        assert_eq!(desk.versions_before_drawings(), 0);
        assert!(!desk.drawn(answer));
    }

    #[gpui::test]
    fn a_drawing_of_nothing_keeps_no_version(cx: &mut TestAppContext) {
        let mut desk = desk(cx);
        let answer = desk.ask(&answer_with(r#"{"remove": ["ghost"]}"#));
        desk.cx.simulate_keystrokes("cmd-shift-d");
        desk.cx.run_until_parked();
        assert!(desk.has_toast(ONLY_GONE), "{:?}", desk.toasts());
        assert!(!desk.drawn(answer));
        let answer = desk.ask(&answer_with(r#"{"update": [{"id": "api", "x": 100}]}"#));
        desk.cx.simulate_keystrokes("cmd-shift-d");
        desk.cx.run_until_parked();
        assert!(desk.has_toast(CHANGES_NOTHING), "{:?}", desk.toasts());
        assert!(!desk.drawn(answer));
        assert!(desk.pages.adopted().is_empty());
        assert_eq!(desk.versions_before_drawings(), 0);
    }

    #[gpui::test]
    fn a_failed_save_leaves_no_version(cx: &mut TestAppContext) {
        let mut desk = desk(cx);
        let answer = desk.ask(&answer_with(DB_AND_BLUE));
        desk.cx.update(|_, cx| fail_board_saves(cx, true));
        desk.cx.simulate_keystrokes("cmd-shift-d");
        desk.cx.run_until_parked();
        desk.cx.update(|_, cx| fail_board_saves(cx, false));
        assert!(
            desk.has_toast("Nothing was drawn: it could not be saved (the library is busy)."),
            "{:?}",
            desk.toasts()
        );
        assert_eq!(desk.versions_before_drawings(), 0);
        assert!(desk.pages.adopted().is_empty());
        assert!(!desk.drawn(answer));
        assert_eq!(desk.stored_ids(), ["api", "api-label"]);
    }

    #[gpui::test]
    fn closing_the_whiteboard_during_the_collect_says_so(cx: &mut TestAppContext) {
        let mut desk = desk(cx);
        desk.ask(&answer_with(DB_AND_BLUE));
        desk.pages.hold_collects();
        let mut elements = api();
        elements.extend(shape(
            "rectangle",
            "late",
            (100.0, 400.0, 100.0, 50.0),
            "a1",
            None,
        ));
        desk.pages.draw_unposted(&scene(&elements));
        desk.cx.run_until_parked();
        desk.cx.simulate_keystrokes("cmd-shift-d");
        desk.cx.run_until_parked();
        let text = desk.text;
        desk.workspace.update_in(desk.cx, |w, window, cx| {
            w.open_document(text, true, window, cx);
        });
        desk.cx.run_until_parked();
        desk.pages.release_collects();
        settle(desk.cx);
        assert!(desk.has_toast(CLOSED_BEFORE_DRAWN), "{:?}", desk.toasts());
        assert_eq!(desk.versions_before_drawings(), 0);
        // The writer's stroke was kept all the same.
        assert!(desk.stored_ids().contains(&"late".to_string()));
        assert!(!desk.stored_ids().contains(&"db".to_string()));
    }

    #[gpui::test]
    fn a_drawing_the_converter_refuses_offers_the_message(cx: &mut TestAppContext) {
        let mut desk = desk(cx);
        let refused = answer_with(
            r#"{"add": [{"type": "rectangle", "id": "x", "x": 0, "y": 400, "link": "javascript:alert(1)"}]}"#,
        );
        desk.ask(&refused);
        desk.cx.simulate_keystrokes("cmd-shift-d");
        desk.cx.run_until_parked();
        let said = desk.toast_starting("Nothing was drawn: ").expect("a toast");
        assert!(said.contains("http://, https:// or mailto:"), "{said}");
        let panel = desk.side();
        let offered = panel.read_with(desk.cx, |panel, cx| panel.composer().read(cx).text(cx));
        assert!(
            offered.starts_with("Your drawing could not be used: ")
                && offered.ends_with(". Please write it again."),
            "{offered}"
        );
        assert!(desk.pages.adopted().is_empty());
        // A composer with the writer's own text is left alone.
        panel.update(desk.cx, |panel, cx| {
            panel
                .composer()
                .update(cx, |composer, cx| composer.set_text("My own words", cx))
        });
        desk.cx.simulate_keystrokes("cmd-shift-d");
        desk.cx.run_until_parked();
        let kept = panel.read_with(desk.cx, |panel, cx| panel.composer().read(cx).text(cx));
        assert_eq!(kept, "My own words");
    }

    /// Where the view is: Excalidraw's scroll and zoom, as the page posts
    /// them.
    fn look_at(desk: &mut Desk<'_>, zoom: f64, scroll_x: f64, scroll_y: f64) {
        let mut status = desk.pages.board_status();
        status.zoom = zoom;
        status.scroll_x = scroll_x;
        status.scroll_y = scroll_y;
        desk.pages.set_status(status);
        desk.cx.run_until_parked();
    }

    #[gpui::test]
    fn a_drawing_that_covers_the_drawing_lands_free_in_view(cx: &mut TestAppContext) {
        let mut desk = desk(cx);
        look_at(&mut desk, 1.0, 0.0, 0.0);
        // A legend written over the API.
        desk.ask(&answer_with(
            r#"{"add": [{"type": "rectangle", "id": "legend", "x": 120, "y": 90, "width": 120, "height": 60}]}"#,
        ));
        desk.cx.simulate_keystrokes("cmd-shift-d");
        desk.cx.run_until_parked();
        let scene = stored(&desk.library, desk.board, desk.cx);
        let legend = scene.element("legend").expect("drawn");
        let (x, y) = (legend.x(), legend.y());
        // Clear of the API grown by the margin, and in the view.
        let clear = x + 120.0 <= 100.0 - 40.0
            || x >= 300.0 + 40.0
            || y + 60.0 <= 80.0 - 40.0
            || y >= 170.0 + 40.0;
        assert!(clear, "the legend at ({x}, {y}) covers the API");
        assert!(x >= 0.0 && y >= 0.0, "in view: ({x}, {y})");
        let said = desk.toast_starting("Drew on").unwrap();
        assert!(said.ends_with(PUT_IN_VIEW), "{said}");
        assert!(
            !desk.pages.board_calls().contains(&"zoom fit".to_string()),
            "it was seen: no zoom"
        );
    }

    #[gpui::test]
    fn one_larger_than_the_view_zooms_to_fit(cx: &mut TestAppContext) {
        let mut desk = desk(cx);
        // Zoomed far in: the view is a few dozen points across.
        look_at(&mut desk, 30.0, -100.0, -80.0);
        desk.ask(&answer_with(
            r#"{"add": [{"type": "rectangle", "id": "big", "x": 100, "y": 80, "width": 600, "height": 400}]}"#,
        ));
        desk.cx.simulate_keystrokes("cmd-shift-d");
        desk.cx.run_until_parked();
        let scene = stored(&desk.library, desk.board, desk.cx);
        let big = scene.element("big").expect("drawn");
        assert!(big.x() >= 300.0 + 100.0, "beside the drawing: {}", big.x());
        let said = desk.toast_starting("Drew on").unwrap();
        assert!(said.ends_with(PUT_BESIDE), "{said}");
        let calls = desk.pages.board_calls();
        let adopt = calls.iter().position(|call| call == "adopt").unwrap();
        let fit = calls
            .iter()
            .position(|call| call == "zoom fit")
            .expect("the view zooms to fit");
        assert!(adopt < fit, "after the adopt: {calls:?}");
    }

    #[gpui::test]
    fn conflicts_are_said_once_the_view_answered(cx: &mut TestAppContext) {
        let mut desk = desk(cx);
        desk.ask(&answer_with(DB_AND_BLUE));
        desk.pages.hold_adopts();
        desk.cx.simulate_keystrokes("cmd-shift-d");
        desk.cx.run_until_parked();
        // The writer moves the API before the canvas takes the drawing in.
        let mut moved = api();
        moved[0]["x"] = json!(140.0);
        moved[0]["version"] = json!(2);
        desk.pages.draw(&scene(&moved));
        desk.cx.run_until_parked();
        desk.pages.release_adopts();
        settle(desk.cx);
        let said = desk.toast_starting("Drew on").unwrap();
        let conflicts = conflicts_sentence(&["api".to_string()]).unwrap();
        assert!(said.contains(&conflicts), "{said}");
        // The writer's copy of the API stays.
        let scene = stored(&desk.library, desk.board, desk.cx);
        assert_eq!(scene.element("api").unwrap().x(), 140.0);
    }

    #[gpui::test]
    fn the_toast_offers_undo_while_the_canvas_has_not_moved_on(cx: &mut TestAppContext) {
        let mut desk = desk(cx);
        desk.ask(&answer_with(DB_AND_BLUE));
        desk.cx.simulate_keystrokes("cmd-shift-d");
        desk.cx.run_until_parked();
        desk.press_toast("Drew on");
        assert!(desk.pages.board_calls().contains(&"undo 1".to_string()));
        assert!(!desk.canvas_ids().contains(&"db".to_string()));

        // Drawn again, then the writer draws: the toast's Undo says why not.
        desk.ask(&answer_with(
            r#"{"add": [{"type": "text", "id": "note", "x": 100, "y": 300, "text": "Reads only."}]}"#,
        ));
        desk.cx.simulate_keystrokes("cmd-shift-d");
        desk.cx.run_until_parked();
        let undos = desk
            .pages
            .board_calls()
            .iter()
            .filter(|call| call.starts_with("undo"))
            .count();
        let mut elements = api();
        elements.extend(shape(
            "rectangle",
            "mine",
            (100.0, 500.0, 100.0, 50.0),
            "a5",
            None,
        ));
        desk.pages.draw(&scene(&elements));
        desk.cx.run_until_parked();
        desk.press_toast("Drew on");
        assert!(desk.has_toast(NOT_UNDONE), "{:?}", desk.toasts());
        let after = desk
            .pages
            .board_calls()
            .iter()
            .filter(|call| call.starts_with("undo"))
            .count();
        assert_eq!(after, undos, "nothing was undone");
    }

    #[gpui::test]
    fn a_whiteboard_that_is_not_ready_is_not_drawn_on(cx: &mut TestAppContext) {
        let mut desk = desk(cx);
        desk.ask(&answer_with(DB_AND_BLUE));
        // A second whiteboard, its page still loading.
        let second = desk
            .workspace
            .update_in(desk.cx, |w, window, cx| {
                w.new_board(Some("Second"), window, cx)
            })
            .unwrap();
        desk.cx.run_until_parked();
        let answer = desk.ask(&answer_with(DB_AND_BLUE));
        let panel = desk.side();
        panel.update(desk.cx, |panel, cx| panel.draw(answer, true, cx));
        desk.cx.run_until_parked();
        assert!(desk.has_toast(STILL_LOADING), "{:?}", desk.toasts());
        assert!(desk.pages.adopted().is_empty());
        // A page that could not start: no drawing is offered, the palette
        // leaves the command out, and the key says why.
        desk.pages.fail_start();
        desk.cx.run_until_parked();
        assert_eq!(
            panel.read_with(desk.cx, |panel, _| panel.drawing_target()),
            innerword_assistant::DrawingTarget::None
        );
        let titles = palette_lists(&desk.workspace, desk.cx);
        assert!(
            !titles
                .iter()
                .any(|title| title == "Draw answer on whiteboard"
                    || title == "New whiteboard from answer"),
            "{titles:?}"
        );
        assert_eq!(
            desk.workspace.read_with(desk.cx, |w, _| w.open_board_id()),
            Some(second)
        );
    }

    /// The titles the palette lists now.
    pub(super) fn palette_lists(
        workspace: &Entity<Workspace>,
        cx: &mut VisualTestContext,
    ) -> Vec<String> {
        workspace.update_in(cx, |w, window, cx| w.toggle_command_palette(window, cx));
        cx.run_until_parked();
        let palette = workspace
            .read_with(cx, |w, cx| {
                w.modal_layer()
                    .read(cx)
                    .active::<innerword_app::palette::CommandPalette>()
            })
            .expect("the palette is open");
        let titles = palette.read_with(cx, |palette, _| palette.delegate.listed_titles());
        workspace.update_in(cx, |w, window, cx| w.dismiss_modal(window, cx));
        cx.run_until_parked();
        titles
    }

    #[gpui::test]
    fn an_answer_drawn_before_needs_a_second_press(cx: &mut TestAppContext) {
        let mut desk = desk(cx);
        let answer = desk.ask(&answer_with(DB_AND_BLUE));
        desk.cx.simulate_keystrokes("cmd-shift-d");
        desk.cx.run_until_parked();
        assert_eq!(desk.pages.adopted().len(), 1);
        // The key, again: it says so first and draws at the second press.
        desk.cx.simulate_keystrokes("cmd-3");
        desk.cx.run_until_parked();
        desk.cx.simulate_keystrokes("cmd-shift-d");
        desk.cx.run_until_parked();
        assert!(desk.has_toast(DRAWN_ALREADY), "{:?}", desk.toasts());
        assert_eq!(desk.pages.adopted().len(), 1);
        desk.cx.simulate_keystrokes("cmd-3");
        desk.cx.run_until_parked();
        desk.cx.simulate_keystrokes("cmd-shift-d");
        desk.cx.run_until_parked();
        assert_eq!(desk.pages.adopted().len(), 2);
        // Its ids are the whiteboard's now: the new elements were renamed.
        let ids = desk.stored_ids();
        assert!(ids.contains(&"db-2".to_string()), "{ids:?}");
        // The button draws at once.
        desk.cx.simulate_keystrokes("cmd-3");
        desk.cx.run_until_parked();
        click(&format!("assistant-action-draw-{}", answer.0), desk.cx);
        assert_eq!(desk.pages.adopted().len(), 3);
        assert_eq!(desk.versions_before_drawings(), 3);
    }

    #[gpui::test]
    fn the_palette_shows_before_it_draws(cx: &mut TestAppContext) {
        let mut desk = desk(cx);
        let answer = desk.ask(&answer_with(DB_AND_BLUE));
        // The side chat closed, the keyboard on the canvas.
        desk.cx.simulate_keystrokes("cmd-3");
        desk.cx.run_until_parked();
        desk.workspace.update_in(desk.cx, |w, window, cx| {
            w.set_assistant_panel_open(false, window, cx)
        });
        let board = desk.board;
        desk.workspace.update_in(desk.cx, |w, window, cx| {
            w.open_document(board, true, window, cx);
        });
        desk.cx.run_until_parked();
        assert!(desk.on_canvas());
        let titles = palette_lists(&desk.workspace, desk.cx);
        assert!(
            titles.contains(&"Draw answer on whiteboard".to_string()),
            "{titles:?}"
        );
        let run = |desk: &mut Desk<'_>| {
            let command = desk.cx.update(|_, cx| {
                innerword_app::commands::all(cx)
                    .into_iter()
                    .find(|command| command.title.as_ref() == "New whiteboard from answer")
                    .expect("the command")
            });
            desk.workspace
                .update_in(desk.cx, |w, window, cx| w.run_command(&command, window, cx));
            desk.cx.run_until_parked();
        };
        run(&mut desk);
        // The side chat came with the answer marked; nothing was drawn.
        assert_eq!(
            desk.workspace
                .read_with(desk.cx, |w, _| w.right_panel_shown()),
            Some(innerword_app::workspace::RightPanel::Assistant)
        );
        let marked = desk
            .side()
            .read_with(desk.cx, |panel, cx| panel.messages().read(cx).selected());
        assert_eq!(marked, Some(answer));
        assert!(
            desk.has_toast(SHOWN_IN_THE_SIDE_CHAT),
            "{:?}",
            desk.toasts()
        );
        assert!(desk.pages.adopted().is_empty());
        assert!(desk.on_canvas(), "the keyboard stayed");
        // The second run draws.
        run(&mut desk);
        assert_eq!(desk.pages.adopted().len(), 1);
        assert!(desk.drawn(answer));
    }

    /// The answer's menu has "Draw on Whiteboard", last, and it draws.
    #[gpui::test]
    fn the_answers_menu_draws_on_the_whiteboard(cx: &mut TestAppContext) {
        let mut desk = desk(cx);
        let answer = desk.ask(&answer_with(DB_AND_BLUE));
        let panel = desk.side();
        panel.update_in(desk.cx, |panel, window, cx| {
            panel.messages().update(cx, |list, cx| {
                list.open_menu(
                    answer,
                    gpui::point(gpui::px(300.), gpui::px(200.)),
                    true,
                    window,
                    cx,
                )
            })
        });
        desk.cx.run_until_parked();
        let labels = desk
            .cx
            .update(|window, cx| {
                innerword_ui::ContextMenu::current(window, cx).map(|menu| menu.read(cx).labels())
            })
            .expect("the answer's menu is open");
        let last = labels.iter().rev().find(|label| *label != "-").unwrap();
        assert!(last.starts_with("Draw on Whiteboard"), "{labels:?}");
        let at = desk.cx.update(|window, cx| {
            let menu = innerword_ui::ContextMenu::current(window, cx).unwrap();
            let menu = menu.read(cx);
            let ix = menu
                .labels()
                .iter()
                .position(|label| label.starts_with("Draw on Whiteboard"))
                .unwrap();
            menu.item_bounds(ix).expect("the menu was drawn").center()
        });
        desk.cx
            .simulate_mouse_move(at, None, gpui::Modifiers::default());
        desk.cx.simulate_click(at, gpui::Modifiers::default());
        desk.cx.run_until_parked();
        assert_eq!(desk.pages.adopted().len(), 1);
        assert!(desk.drawn(answer));
    }

    #[gpui::test]
    fn the_line_names_the_whiteboards_elements(cx: &mut TestAppContext) {
        let mut desk = desk(cx);
        // A picture beside the API.
        let png = innerword_image::fixtures::png(40, 30, [10, 120, 200]);
        desk.pages.paste_picture("pasted", "image/png", &png);
        let mut elements = api();
        let mut picture = shape("image", "pic", (400.0, 80.0, 40.0, 30.0), "a1", None).remove(0);
        picture["fileId"] = json!("pasted");
        picture["status"] = json!("saved");
        elements.push(picture);
        desk.pages.draw(&scene(&elements));
        settle(desk.cx);
        let answer = desk.ask(&answer_with(
            r#"{"update": [{"id": "api", "x": 120}], "remove": ["pic"]}"#,
        ));
        let shown = desk.shown(answer);
        assert!(
            shown
                .contains("Drawing: changes the rectangle \u{201c}API\u{201d}; removes a picture."),
            "{shown}"
        );
        // Drawn: the toast counts what changed and what was removed.
        desk.cx.simulate_keystrokes("cmd-shift-d");
        desk.cx.run_until_parked();
        assert!(
            desk.has_toast(&drew_sentence("Plan", 0, 1, 1)),
            "{:?}",
            desk.toasts()
        );
        assert!(!desk.stored_ids().contains(&"pic".to_string()));
    }
}
