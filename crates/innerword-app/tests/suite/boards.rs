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
