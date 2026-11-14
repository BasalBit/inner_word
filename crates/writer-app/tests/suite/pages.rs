//! Pages in the assembled app, on the fake engine (`pages::use_fake`): no
//! test makes a `WKWebView`. Opening a page (the sidebar, quick open, a
//! launch), the session meanwhile, the commands that do not apply, focus and
//! the first responder, the snapshot of an overlay and the holes of toasts,
//! find, zoom, theme, a crashed WebContent process, the keys a page hands
//! on, and a run without an engine.

use std::rc::Rc;

use gpui::TestAppContext;
use writer_app::{
    actions::{
        CopyPageAddress, OpenDocumentSettings, ReloadPage, SaveNow, TogglePageView, UseDarkTheme,
    },
    automation::describe_state,
    pages::{self, FakePages},
    palette::CommandPalette,
    settings::keys,
    workspace::{
        NO_COMMENTS_ON_A_PAGE, NO_FIT_ON_A_PAGE, NOT_FOR_A_PAGE, NOTHING_TO_SAVE_ON_A_PAGE,
        PageState,
    },
};
use writer_ui::Toast;

use super::*;

const HARBOUR: &str = "<!doctype html><html><head><title>Harbour Guide</title></head>\
<body><h1>The harbour</h1><p>The boats come by road every spring.</p>\
<script>document.title = 'Harbour Guide';</script></body></html>";

/// Stores a page of one HTML file (and `more` files beside it) and returns
/// its id.
fn add_page(
    library: &Entity<Library>,
    name: &str,
    html: &str,
    source_url: Option<&str>,
    cx: &mut TestAppContext,
) -> DocumentId {
    let bundle = pages::offline_bundle(
        name,
        "index.html",
        vec![("index.html".to_string(), html.as_bytes().to_vec())],
    );
    let library = library.clone();
    let source_url = source_url.map(str::to_string);
    cx.update(|cx| {
        pages::store_offline_bundle(&library, bundle, None, source_url.as_deref(), cx)
            .expect("the page is stored")
            .id
    })
}

/// A workspace on the fake engine with a text and a page, the page open
/// and its element focused.
fn open_page<'a>(
    cx: &'a mut TestAppContext,
) -> (
    Rc<FakePages>,
    Entity<Workspace>,
    DocumentId,
    DocumentId,
    &'a mut VisualTestContext,
) {
    init(cx);
    let fake = cx.update(pages::use_fake);
    let library = library(cx);
    let text = add_document(
        &library,
        None,
        &simple_document("Notes", "A short text."),
        cx,
    );
    let page = add_page(
        &library,
        "harbour",
        HARBOUR,
        Some("https://example.com/harbour"),
        cx,
    );
    let (workspace, vcx) = open_workspace(&library, cx);
    workspace.update_in(vcx, |workspace, window, cx| {
        assert!(workspace.open_document(page, true, window, cx));
    });
    vcx.run_until_parked();
    (fake, workspace, text, page, vcx)
}

fn state_line(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> String {
    workspace.update_in(cx, |workspace, window, cx| {
        describe_state(workspace, window, cx)
    })
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

fn page_focused(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> bool {
    workspace.update_in(cx, |workspace, window, _| workspace.page_has_focus(window))
}

fn available(action: &dyn gpui::Action, cx: &mut VisualTestContext) -> bool {
    cx.run_until_parked();
    cx.update(|window, cx| window.is_action_available(action, cx))
}

#[gpui::test]
fn a_page_opens_as_a_page_and_no_text_is_open_meanwhile(cx: &mut TestAppContext) {
    let (fake, workspace, text, page, vcx) = open_page(cx);
    workspace.read_with(vcx, |w, cx| {
        assert_eq!(w.open_page_id(), Some(page));
        assert_eq!(w.open_item_id(), Some(page));
        assert_eq!(
            w.open_document_id(),
            None,
            "no text session while a page is open"
        );
        assert!(w.editor().read(cx).is_read_only());
        assert_eq!(w.page_title(), Some("Harbour Guide"));
        assert_eq!(w.page_state(), Some(PageState::Loading));
    });
    // The view is made at the first frame, for the page's entry.
    assert_eq!(fake.opened(), vec![(page, "index.html".to_string())]);
    assert!(page_focused(&workspace, vcx));
    assert!(fake.first_responder_is_page());
    fake.finish_loading();
    vcx.run_until_parked();
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.page_state()),
        Some(PageState::Loaded)
    );
    let state = state_line(&workspace, vcx);
    assert!(
        state.contains(&format!(
            "web=[id={} state=loaded zoom=100% overlay=none",
            page.0
        )),
        "{state}"
    );
    assert!(state.contains("focus=page responder=web"), "{state}");
    // The page is the last open item.
    let last: Option<DocumentId> = workspace.read_with(vcx, |w, cx| {
        w.library().read(cx).setting(keys::LAST_DOCUMENT).unwrap()
    });
    assert_eq!(last, Some(page));

    // Opening a text closes the page and its view.
    workspace.update_in(vcx, |w, window, cx| {
        assert!(w.open_document(text, true, window, cx));
    });
    vcx.run_until_parked();
    workspace.read_with(vcx, |w, cx| {
        assert_eq!(w.open_page_id(), None);
        assert_eq!(w.open_document_id(), Some(text));
        assert!(!w.editor().read(cx).is_read_only());
    });
    assert!(!fake.first_responder_is_page());
    assert!(state_line(&workspace, vcx).contains("web=[none]"));
}

#[gpui::test]
fn a_launch_opens_the_page_that_was_open(cx: &mut TestAppContext) {
    init(cx);
    let fake = cx.update(pages::use_fake);
    let library = library(cx);
    add_document(
        &library,
        None,
        &simple_document("Notes", "A short text."),
        cx,
    );
    let page = add_page(&library, "harbour", HARBOUR, None, cx);
    library.update(cx, |library, cx| {
        library.set_setting(keys::LAST_DOCUMENT, &page, cx).unwrap();
    });
    let (workspace, vcx) = open_workspace(&library, cx);
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.open_page_id()),
        Some(page)
    );
    assert_eq!(fake.opened().len(), 1);
}

#[gpui::test]
fn the_sidebar_and_quick_open_open_a_page(cx: &mut TestAppContext) {
    init(cx);
    let fake = cx.update(pages::use_fake);
    let library = library(cx);
    let text = add_document(
        &library,
        None,
        &simple_document("Notes", "A short text."),
        cx,
    );
    let page = add_page(&library, "harbour", HARBOUR, None, cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    workspace.update_in(vcx, |w, window, cx| {
        w.open_document(text, true, window, cx);
    });
    vcx.run_until_parked();
    // The sidebar asks for the item as it does for a document.
    let panel = workspace.read_with(vcx, |w, _| w.panel().clone());
    panel.update(vcx, |_, cx| {
        cx.emit(writer_library::LibraryPanelEvent::OpenDocument {
            id: page,
            focus_editor: true,
        });
    });
    vcx.run_until_parked();
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.open_page_id()),
        Some(page)
    );
    assert!(page_focused(&workspace, vcx));
    // Quick open lists it and opens it.
    workspace.update_in(vcx, |w, window, cx| w.open_document(text, true, window, cx));
    vcx.run_until_parked();
    vcx.simulate_keystrokes("cmd-p");
    vcx.simulate_input("Harbour");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.open_page_id()),
        Some(page)
    );
    // The launch opened the page (the item edited last), then the sidebar,
    // then quick open: a view each time.
    assert_eq!(fake.opened().len(), 3);
}

#[gpui::test]
fn the_commands_of_a_text_are_not_there_on_a_page(cx: &mut TestAppContext) {
    let (_fake, workspace, _text, _page, vcx) = open_page(cx);
    for action in [
        Box::new(writer_editor::actions::ToggleBold) as Box<dyn gpui::Action>,
        Box::new(writer_editor::actions::ToggleItalic),
        Box::new(writer_editor_tools::actions::OpenFormatMenu),
    ] {
        assert!(
            !available(action.as_ref(), vcx),
            "{} has no handler on a page",
            action.name()
        );
    }
    // The palette leaves them out; find stays.
    vcx.simulate_keystrokes("cmd-shift-p");
    let titles = workspace.read_with(vcx, |w, cx| {
        let palette = w
            .modal_layer()
            .read(cx)
            .active::<CommandPalette>()
            .expect("the palette is open");
        palette.read(cx).delegate.listed_titles()
    });
    assert!(!titles.iter().any(|title| title == "Bold"), "{titles:?}");
    assert!(
        titles.iter().any(|title| title == "Reload page"),
        "{titles:?}"
    );
    assert!(titles.iter().any(|title| title == "Find"), "{titles:?}");
    vcx.simulate_keystrokes("escape");
}

#[gpui::test]
fn the_commands_of_a_document_say_what_a_page_is(cx: &mut TestAppContext) {
    let (_fake, workspace, _text, _page, vcx) = open_page(cx);
    let cases: Vec<(Box<dyn gpui::Action>, &str)> = vec![
        (Box::new(OpenDocumentSettings), NOT_FOR_A_PAGE),
        (Box::new(TogglePageView), NOT_FOR_A_PAGE),
        (Box::new(SaveNow), NOTHING_TO_SAVE_ON_A_PAGE),
        (Box::new(writer_comments::AddComment), NO_COMMENTS_ON_A_PAGE),
        (Box::new(writer_app::zoom::FitWidth), NO_FIT_ON_A_PAGE),
    ];
    for (action, sentence) in cases {
        workspace.update(vcx, |w, cx| {
            w.toast_layer().update(cx, |layer, cx| layer.clear(cx));
        });
        let name = action.name();
        let action = action.boxed_clone();
        vcx.update(|window, cx| window.dispatch_action(action, cx));
        vcx.run_until_parked();
        assert_eq!(
            toasts(&workspace, vcx),
            vec![sentence.to_string()],
            "{name}"
        );
    }
    // Nothing of a text happened meanwhile.
    workspace.read_with(vcx, |w, cx| {
        assert!(!w.modal_layer().read(cx).has_modal());
        assert!(w.open_document_id().is_none());
    });
}

#[gpui::test]
fn the_keyboard_and_the_first_responder_go_together(cx: &mut TestAppContext) {
    let (fake, workspace, _text, _page, vcx) = open_page(cx);
    assert!(fake.first_responder_is_page());
    vcx.simulate_keystrokes("cmd-0");
    vcx.run_until_parked();
    assert!(!page_focused(&workspace, vcx));
    assert!(!fake.first_responder_is_page(), "the library has the keys");
    vcx.simulate_keystrokes("cmd-1");
    vcx.run_until_parked();
    assert!(page_focused(&workspace, vcx));
    assert!(fake.first_responder_is_page());
    // A click into the page while the keyboard is in the library: the page
    // takes the keyboard.
    vcx.simulate_keystrokes("cmd-0");
    vcx.run_until_parked();
    fake.click_into_page();
    vcx.run_until_parked();
    assert!(page_focused(&workspace, vcx));
    assert!(fake.first_responder_is_page());
    // The Edit menu's items go to the view.
    vcx.dispatch_action(writer_editor::actions::Copy);
    vcx.dispatch_action(writer_editor::actions::SelectAll);
    vcx.run_until_parked();
    assert_eq!(fake.edits(), vec!["copy", "select_all"]);
}

#[gpui::test]
fn a_modal_over_the_page_has_one_snapshot_that_is_dropped(cx: &mut TestAppContext) {
    let (fake, workspace, _text, _page, vcx) = open_page(cx);
    for round in 1..=20 {
        vcx.simulate_keystrokes("cmd-shift-p");
        vcx.run_until_parked();
        assert!(fake.hidden(), "the snapshot stands in for the view");
        assert!(!fake.first_responder_is_page(), "the palette has the keys");
        assert!(state_line(&workspace, vcx).contains("overlay=snapshot"));
        vcx.simulate_keystrokes("escape");
        vcx.run_until_parked();
        assert!(!fake.hidden());
        assert!(fake.first_responder_is_page());
        assert_eq!(fake.snapshots_made(), round);
        assert_eq!(fake.snapshots_dropped(), round, "every snapshot is dropped");
    }
}

#[gpui::test]
fn a_toast_over_the_page_is_a_hole_in_the_view(cx: &mut TestAppContext) {
    let (fake, workspace, _text, _page, vcx) = open_page(cx);
    assert!(fake.holes().is_empty());
    workspace.update(vcx, |w, cx| {
        w.show_toast(Toast::new("Claude changed “Notes”"), cx)
    });
    vcx.run_until_parked();
    // The toast's bounds are known after it was drawn once.
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    assert_eq!(fake.holes().len(), 1, "one hole for one toast");
    assert!(state_line(&workspace, vcx).contains("overlay=holes:1"));
    assert!(!fake.hidden(), "the page stays live under a toast");
    workspace.update(vcx, |w, cx| {
        w.toast_layer().update(cx, |layer, cx| layer.clear(cx));
    });
    vcx.run_until_parked();
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    assert!(fake.holes().is_empty());
}

#[gpui::test]
fn find_zoom_and_theme_reach_the_view(cx: &mut TestAppContext) {
    let (fake, workspace, _text, _page, vcx) = open_page(cx);
    vcx.simulate_keystrokes("cmd-f");
    vcx.run_until_parked();
    assert!(
        !fake.first_responder_is_page(),
        "the strip's field has the keys"
    );
    vcx.simulate_input("boats");
    vcx.run_until_parked();
    assert_eq!(fake.finds().last(), Some(&("boats".to_string(), false)));
    assert_eq!(
        workspace.read_with(vcx, |w, cx| w.page_find(cx)),
        Some(("boats".to_string(), Some(true)))
    );
    vcx.simulate_keystrokes("cmd-g");
    assert_eq!(fake.finds().last(), Some(&("boats".to_string(), false)));
    vcx.simulate_keystrokes("cmd-shift-g");
    assert_eq!(fake.finds().last(), Some(&("boats".to_string(), true)));
    // shift-enter is the find bar's "previous".
    let before = fake.finds().len();
    vcx.simulate_keystrokes("shift-enter");
    assert_eq!(fake.finds().len(), before + 1);
    assert_eq!(fake.finds().last(), Some(&("boats".to_string(), true)));
    vcx.simulate_input("zz");
    vcx.run_until_parked();
    assert_eq!(
        workspace.read_with(vcx, |w, cx| w.page_find(cx)),
        Some(("boatszz".to_string(), Some(false)))
    );
    // escape closes the strip and gives the keyboard back to the page.
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    assert_eq!(workspace.read_with(vcx, |w, cx| w.page_find(cx)), None);
    assert!(page_focused(&workspace, vcx));
    assert!(fake.first_responder_is_page());

    vcx.simulate_keystrokes("cmd-=");
    vcx.run_until_parked();
    assert_eq!(fake.zoom(), Some(110));
    vcx.simulate_keystrokes("cmd--");
    vcx.simulate_keystrokes("cmd--");
    vcx.run_until_parked();
    assert_eq!(fake.zoom(), Some(90));
    vcx.dispatch_action(writer_app::zoom::ActualSize);
    vcx.run_until_parked();
    assert_eq!(fake.zoom(), Some(100));

    vcx.dispatch_action(UseDarkTheme);
    vcx.run_until_parked();
    assert_eq!(fake.appearance(), Some(writer_web::Appearance::Dark));
}

#[gpui::test]
fn a_crash_loads_again_and_a_second_shows_that_the_page_stopped(cx: &mut TestAppContext) {
    let (fake, workspace, _text, _page, vcx) = open_page(cx);
    fake.finish_loading();
    vcx.run_until_parked();
    fake.crash();
    vcx.run_until_parked();
    assert_eq!(fake.reloads(), 1, "the first crash loads the page again");
    assert!(!fake.hidden());
    fake.crash();
    vcx.run_until_parked();
    assert_eq!(fake.reloads(), 1);
    assert!(fake.hidden(), "a second crash hides the view");
    assert!(state_line(&workspace, vcx).contains("state=stopped"));
    vcx.dispatch_action(ReloadPage);
    vcx.run_until_parked();
    assert_eq!(fake.reloads(), 2);
    assert!(!fake.hidden());
}

#[gpui::test]
fn the_context_menu_key_is_taken_from_the_page(cx: &mut TestAppContext) {
    let (fake, workspace, _text, _page, vcx) = open_page(cx);
    // A letter, space and escape belong to the page.
    for key in [
        "a",
        "j",
        "k",
        "shift-a",
        "space",
        "enter",
        "escape",
        "up",
        "down",
        "left",
        "right",
        "tab",
        "shift-tab",
        "backspace",
        "pagedown",
        "pageup",
        "home",
        "end",
    ] {
        assert!(!fake.press(key), "{key} stays in the page");
    }
    // ctrl-enter and shift-f10 open the page's menu.
    assert!(fake.press("ctrl-enter"));
    vcx.run_until_parked();
    let labels = vcx.update(|window, cx| {
        writer_ui::ContextMenu::current(window, cx)
            .map(|menu| menu.read(cx).labels())
            .unwrap_or_default()
    });
    assert!(labels.iter().any(|label| label == "Copy"), "{labels:?}");
    assert!(
        labels.iter().any(|label| label == "Find in Page…"),
        "{labels:?}"
    );
    assert!(
        labels.iter().any(|label| label == "Copy Page Address"),
        "{labels:?}"
    );
    vcx.update(|window, cx| writer_ui::ContextMenu::dismiss(window, cx));
    vcx.run_until_parked();
    assert!(fake.press("shift-f10"));
    vcx.run_until_parked();
    assert!(vcx.update(|window, cx| writer_ui::ContextMenu::is_open(window, cx)));
    vcx.update(|window, cx| writer_ui::ContextMenu::dismiss(window, cx));
    vcx.run_until_parked();
    // WebKit's own menu gets the app's items, each with its key.
    let titles = fake.menu_titles();
    assert!(
        titles.contains(&"Find in Page…\tcmd-f".to_string()),
        "{titles:?}"
    );
    assert!(titles.contains(&"Reload Page".to_string()), "{titles:?}");
    // Choosing one runs its command.
    let find = titles
        .iter()
        .position(|title| title.starts_with("Find in Page"))
        .unwrap() as u32
        + 1;
    fake.choose_in_menu(find);
    vcx.run_until_parked();
    assert!(workspace.read_with(vcx, |w, cx| w.page_find(cx)).is_some());
}

#[gpui::test]
fn the_page_address_is_copied(cx: &mut TestAppContext) {
    let (_fake, _workspace, _text, _page, vcx) = open_page(cx);
    vcx.dispatch_action(CopyPageAddress);
    vcx.run_until_parked();
    assert_eq!(
        vcx.read_from_clipboard().and_then(|item| item.text()),
        Some("https://example.com/harbour".to_string())
    );
}

#[gpui::test]
fn a_page_that_goes_to_the_trash_closes(cx: &mut TestAppContext) {
    let (_fake, workspace, text, page, vcx) = open_page(cx);
    let library = workspace.read_with(vcx, |w, _| w.library().clone());
    library.update(vcx, |library, cx| library.trash_document(page, cx).unwrap());
    vcx.run_until_parked();
    workspace.read_with(vcx, |w, _| {
        assert_eq!(w.open_page_id(), None);
        assert_eq!(w.open_document_id(), Some(text));
    });
}

#[gpui::test]
fn without_an_engine_a_page_shows_its_text_and_says_why(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    let page = add_page(&library, "harbour", HARBOUR, None, cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    workspace.update_in(vcx, |w, window, cx| {
        assert!(w.open_document(page, true, window, cx));
    });
    vcx.run_until_parked();
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.page_state()),
        Some(PageState::Failed(pages::NOT_SHOWN_IN_THIS_RUN.to_string()))
    );
    assert!(
        workspace
            .read_with(vcx, |w, _| w.page_manifest().unwrap().text.clone())
            .contains("The boats come by road")
    );
}

#[gpui::test]
fn print_hands_the_pdf_of_the_page_to_the_target(cx: &mut TestAppContext) {
    init(cx);
    let jobs = cx.update(writer_app::printing::record);
    let fake = cx.update(pages::use_fake);
    let library = library(cx);
    let page = add_page(&library, "harbour", HARBOUR, None, cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    workspace.update_in(vcx, |w, window, cx| {
        w.open_document(page, true, window, cx);
    });
    vcx.run_until_parked();
    fake.finish_loading();
    assert!(available(&writer_app::actions::Print, vcx));
    vcx.dispatch_action(writer_app::actions::Print);
    vcx.executor()
        .advance_clock(std::time::Duration::from_millis(10));
    vcx.run_until_parked();
    let jobs = jobs.borrow();
    assert_eq!(jobs.len(), 1, "one job");
    assert_eq!(jobs[0].title, "Harbour Guide");
    assert_eq!(jobs[0].pdf.bytes(), pages::fake_pdf().as_slice());
    assert_eq!(workspace.read_with(vcx, |w, _| w.printing()), None);
}

#[gpui::test]
fn export_writes_the_folder_and_the_pdf_of_a_page(cx: &mut TestAppContext) {
    let (_fake, workspace, _text, _page, vcx) = open_page(cx);
    let out = tempfile::tempdir().unwrap();
    vcx.simulate_keystrokes("cmd-shift-s");
    let choices = workspace
        .read_with(vcx, |w, cx| {
            w.modal_layer()
                .read(cx)
                .active::<writer_app::page_export::PageExportPicker>()
                .map(|picker| picker.read(cx).delegate.choices())
        })
        .expect("the export of a page is open");
    use writer_app::page_export::PageExportChoice;
    assert_eq!(
        choices,
        vec![PageExportChoice::Folder, PageExportChoice::Pdf]
    );
    // The folder, through the save panel.
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert!(vcx.did_prompt_for_new_path());
    let folder = out.path().join("Harbour Guide");
    vcx.simulate_new_path_selection(|_| Some(folder.clone()));
    vcx.run_until_parked();
    let html = std::fs::read_to_string(folder.join("index.html")).expect("the entry is written");
    assert!(html.contains("The boats come by road"), "{html}");
    let said = toasts(&workspace, vcx);
    assert!(
        said.iter()
            .any(|toast| toast.starts_with("Exported “Harbour Guide”")
                && toast.contains("python3 -m http.server")),
        "{said:?}"
    );
    // The PDF.
    vcx.simulate_keystrokes("cmd-shift-s");
    vcx.simulate_keystrokes("down");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    let pdf = out.path().join("Harbour Guide.pdf");
    vcx.simulate_new_path_selection(|_| Some(pdf.clone()));
    vcx.run_until_parked();
    assert_eq!(std::fs::read(&pdf).unwrap(), pages::fake_pdf());
}

#[gpui::test]
fn the_versions_of_a_page_show_its_text_and_one_is_restored(cx: &mut TestAppContext) {
    let (fake, workspace, _text, page, vcx) = open_page(cx);
    let library = workspace.read_with(vcx, |w, _| w.library().clone());
    // The page as it was is kept as a version, and it is updated.
    let newer = pages::offline_bundle(
        "harbour",
        "index.html",
        vec![(
            "index.html".to_string(),
            b"<html><head><title>Harbour Guide</title></head><body><p>The ferry runs at noon.</p></body></html>".to_vec(),
        )],
    );
    library.update(vcx, |library, cx| {
        library.create_snapshot(page, cx).unwrap();
    });
    {
        let report = writer_page::static_check(&newer);
        let prepared = writer_page::prepare(newer, &report).unwrap();
        let manifest = prepared.manifest(None, 1).to_json();
        library.update(vcx, |library, cx| {
            for blob in &prepared.blobs {
                library.put_file(&blob.data).unwrap();
            }
            library
                .save_page(
                    page,
                    &writer_library::PageContent {
                        manifest: &manifest,
                        title: "Harbour Guide",
                        plain_text: &prepared.text,
                        word_count: 5,
                        char_count: 24,
                        source_url: None,
                    },
                    cx,
                )
                .unwrap();
        });
    }
    vcx.run_until_parked();
    // The open page follows: a view of the new page.
    assert_eq!(fake.opened().len(), 2);
    assert!(
        workspace
            .read_with(vcx, |w, _| w.page_manifest().unwrap().text.clone())
            .contains("ferry")
    );
    vcx.dispatch_action(writer_app::actions::ShowVersionHistory);
    vcx.run_until_parked();
    let history = workspace
        .read_with(vcx, |w, cx| {
            w.modal_layer()
                .read(cx)
                .active::<writer_app::version_history::VersionHistory>()
        })
        .expect("the version history is open");
    let preview = history.read_with(vcx, |history, _| history.preview_text());
    assert!(preview.starts_with("1 file"), "{preview}");
    assert!(preview.contains("The boats come by road"), "{preview}");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert!(
        workspace
            .read_with(vcx, |w, _| w.page_manifest().unwrap().text.clone())
            .contains("The boats come by road"),
        "the version is the page again"
    );
    let versions = library.read_with(vcx, |library, _| library.snapshots(page).unwrap().len());
    assert_eq!(versions, 2, "the page from before the restore is kept");
}

#[gpui::test]
fn a_bare_letter_is_refused_as_a_key_of_a_page_command(cx: &mut TestAppContext) {
    let (_fake, _workspace, _text, _page, vcx) = open_page(cx);
    vcx.update(|_, cx| {
        use writer_keymap::{KeySequence, Refusal, Target};
        let model = writer_app::keymap::model(cx).unwrap();
        let target = Target::new("page::OpenContextMenu", Some(pages::PAGE_VIEW));
        assert_eq!(
            model
                .check(&target, &KeySequence::parse("m").unwrap(), None)
                .refusal,
            Some(Refusal::TypesText)
        );
        assert_eq!(
            model
                .check(&target, &KeySequence::parse("ctrl-m").unwrap(), None)
                .refusal,
            None
        );
    });
}

#[gpui::test]
fn the_title_and_the_status_items_of_a_page_have_their_menus(cx: &mut TestAppContext) {
    let (_fake, workspace, _text, _page, vcx) = open_page(cx);
    let labels = |vcx: &mut VisualTestContext| {
        vcx.update(|window, cx| {
            writer_ui::ContextMenu::current(window, cx)
                .map(|menu| menu.read(cx).labels())
                .unwrap_or_default()
        })
    };
    let bounds = vcx
        .debug_bounds("title-chevron")
        .expect("the title has a chevron");
    vcx.simulate_click(bounds.center(), gpui::Modifiers::default());
    vcx.run_until_parked();
    let title = labels(vcx);
    for item in [
        "Rename…",
        "Reload Page",
        "Find in Page…",
        "Copy Page Address",
        "Export…",
    ] {
        assert!(
            title.iter().any(|label| label == item),
            "{item} in {title:?}"
        );
    }
    assert!(
        !title.iter().any(|label| label == "Document Settings…"),
        "{title:?}"
    );
    assert!(state_line(&workspace, vcx).contains("overlay=snapshot"));
    vcx.update(|window, cx| writer_ui::ContextMenu::dismiss(window, cx));
    vcx.run_until_parked();
    // The zoom in the status bar.
    let bounds = vcx
        .debug_bounds("status-zoom")
        .expect("the zoom is in the status bar");
    vcx.simulate_click(bounds.center(), gpui::Modifiers::default());
    vcx.run_until_parked();
    let zoom = labels(vcx);
    assert!(zoom.iter().any(|label| label == "300 %"), "{zoom:?}");
    assert!(!zoom.iter().any(|label| label == "Fit Width"), "{zoom:?}");
    vcx.update(|window, cx| writer_ui::ContextMenu::dismiss(window, cx));
    // The words of the page.
    assert!(
        vcx.debug_bounds("status-statistics").is_some(),
        "the words of the page are counted"
    );
}

#[gpui::test]
fn ctrl_enter_opens_the_menu_of_the_page_where_the_keyboard_is(cx: &mut TestAppContext) {
    let (_fake, _workspace, _text, _page, vcx) = open_page(cx);
    vcx.simulate_keystrokes("ctrl-enter");
    vcx.run_until_parked();
    let labels = vcx.update(|window, cx| {
        writer_ui::ContextMenu::current(window, cx)
            .map(|menu| menu.read(cx).labels())
            .unwrap_or_default()
    });
    assert_eq!(
        labels.first().map(String::as_str),
        Some("Copy"),
        "{labels:?}"
    );
    assert!(
        labels.iter().any(|label| label == "Version History…"),
        "{labels:?}"
    );
}

#[gpui::test]
fn the_sparkle_is_in_the_status_bar_of_a_page_while_the_assistant_is_shown(
    cx: &mut TestAppContext,
) {
    init(cx);
    let fake_engine = writer_assistant::FakeBackend::new();
    let backend = std::sync::Arc::new(fake_engine.clone());
    cx.update(|cx| writer_app::assistant::set_backend(backend, cx));
    cx.update(pages::use_fake);
    let library = library(cx);
    let page = add_page(&library, "harbour", HARBOUR, None, cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    workspace.update_in(vcx, |w, window, cx| {
        w.open_document(page, true, window, cx);
    });
    vcx.run_until_parked();
    assert!(workspace.read_with(vcx, |w, _| w.assistant_shown()));
    assert!(
        vcx.debug_bounds("status-assistant").is_some(),
        "the side chat can be opened over a page"
    );
}

#[gpui::test]
fn the_list_of_shortcuts_has_the_rows_of_pages(cx: &mut TestAppContext) {
    let (_fake, _workspace, _text, _page, vcx) = open_page(cx);
    vcx.update(|_, cx| {
        let rows = writer_app::shortcuts::collect_shortcuts(cx);
        // Without the assistant (no engine in this test): 353 rows, eleven
        // of them about pages; the file has every one of its 408 entries.
        assert_eq!(rows.len(), 353);
        let on_a_page: Vec<String> = rows
            .iter()
            .filter(|row| row.place == writer_app::shortcuts::Place::Page)
            .map(|row| row.title.to_string())
            .collect();
        assert_eq!(
            on_a_page,
            [
                "Find",
                "Find and replace",
                "Find next",
                "Find previous",
                "Show the page's context menu"
            ]
        );
        assert_eq!(
            writer_app::workspace::shortcuts_file_catalogue(cx)
                .entries()
                .len(),
            408
        );
    });
}

// ----- adding pages in the app -------------------------------------------------------

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use writer_app::{
    actions::{CancelAddingPages, ImportPage, ShowPageReport},
    page_report::PageReport,
    workspace::{ADDING_NOTE_DELAY, PageOutcome, PagesReport},
};
use writer_library::DocumentKind;
use writer_page::{Blocked, CheckReport, FetchError, Fetched, Fetcher, LoadReport, MapFetcher};

const GRADES: &str = "<!doctype html><html><head><title>Grades</title></head>\
<body><p>Marks.</p></body></html>";

/// Writes `files` (path in the folder, text) into a new temporary folder.
fn files_of(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (path, text) in files {
        let path = dir.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    dir
}

/// Lets the app work until no page is being added any more.
fn wait_until_added(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) {
    let started = Instant::now();
    loop {
        cx.run_until_parked();
        if !workspace.read_with(cx, |w, _| w.is_adding_pages()) {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "the pages were not added"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    cx.run_until_parked();
}

/// The pages of the library: id, title, folder.
fn pages_in(
    workspace: &Entity<Workspace>,
    cx: &mut VisualTestContext,
) -> Vec<(DocumentId, String, Option<writer_library::FolderId>)> {
    workspace.read_with(cx, |w, cx| {
        w.library()
            .read(cx)
            .all_documents()
            .unwrap()
            .into_iter()
            .filter(|entry| entry.kind == DocumentKind::Page)
            .map(|entry| (entry.meta.id, entry.meta.title, entry.meta.folder_id))
            .collect()
    })
}

/// The notes at the bottom with the labels of their buttons.
fn notes(
    workspace: &Entity<Workspace>,
    cx: &mut VisualTestContext,
) -> Vec<(String, Option<String>)> {
    workspace.read_with(cx, |w, cx| {
        w.toast_layer()
            .read(cx)
            .toasts()
            .map(|toast| {
                (
                    toast.message.to_string(),
                    toast.action_label().map(ToString::to_string),
                )
            })
            .collect()
    })
}

fn report(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> PagesReport {
    workspace
        .read_with(cx, |w, _| w.last_pages_report().cloned())
        .expect("a gesture ended")
}

/// A workspace on the fake engine whose fetcher answers `fetcher`.
fn import_desk<'a>(
    fetcher: Arc<dyn Fetcher>,
    cx: &'a mut TestAppContext,
) -> (
    Rc<FakePages>,
    Entity<Workspace>,
    DocumentId,
    &'a mut VisualTestContext,
) {
    init(cx);
    let fake = cx.update(pages::use_fake);
    cx.update(|cx| pages::set_fetcher(fetcher, cx));
    let library = library(cx);
    let text = add_document(
        &library,
        None,
        &simple_document("Notes", "A short text."),
        cx,
    );
    let (workspace, vcx) = open_workspace(&library, cx);
    (fake, workspace, text, vcx)
}

/// File > Import Page… takes what was chosen: a file and a folder, each a
/// page, fetched, checked and stored; the first opens; one summary.
#[gpui::test]
fn import_page_adds_a_file_and_a_folder_and_opens_the_first(cx: &mut TestAppContext) {
    let fetcher = MapFetcher::new().with(
        "https://fonts.example.test/geist.css",
        "text/css",
        "body { font-family: Geist }",
    );
    let (fake, workspace, text, vcx) = import_desk(Arc::new(fetcher), cx);
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.open_document_id()),
        Some(text)
    );
    let input = files_of(&[
        (
            "harbour.html",
            "<!doctype html><html><head><title>Harbour Guide</title>\
             <link rel=\"stylesheet\" href=\"https://fonts.example.test/geist.css\"></head>\
             <body><p>The boats come by road.</p></body></html>",
        ),
        (
            "Orbitarium/index.html",
            "<!doctype html><html><head><title>Orbitarium</title>\
             <link rel=\"stylesheet\" href=\"sky.css\"></head><body><h1>The sky</h1></body></html>",
        ),
        ("Orbitarium/sky.css", "h1 { color: navy }"),
    ]);
    let file = input.path().join("harbour.html");
    let folder = input.path().join("Orbitarium");
    assert!(available(&ImportPage, vcx));
    vcx.update(|_, cx| pages::choose_beforehand(vec![file.clone(), folder.clone()], cx));
    vcx.dispatch_action(ImportPage);
    assert!(workspace.read_with(vcx, |w, _| w.is_adding_pages()));
    assert!(available(&CancelAddingPages, vcx));
    wait_until_added(&workspace, vcx);

    let mut added = pages_in(&workspace, vcx);
    added.sort_by_key(|(id, _, _)| id.0);
    let titles: Vec<&str> = added.iter().map(|(_, title, _)| title.as_str()).collect();
    assert_eq!(titles, ["Harbour Guide", "Orbitarium"], "{added:?}");
    assert_eq!(fake.checks_asked(), 2);
    // The first page of the gesture opened.
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.open_page_id()),
        Some(added[0].0)
    );
    let report = report(&workspace, vcx);
    assert_eq!(report.added(), 2);
    match &report.pages[0].outcome {
        PageOutcome::Added {
            files, externals, ..
        } => {
            assert_eq!(*externals, 1, "the style sheet on the web is kept");
            assert_eq!(*files, 2);
        }
        other => panic!("{other:?}"),
    }
    // The page's files are in the library, the piece from the web among
    // them, and the view serves them.
    let manifest = workspace
        .read_with(vcx, |w, _| w.page_manifest().cloned())
        .unwrap();
    assert!(
        manifest
            .files
            .iter()
            .any(|file| file.path.starts_with("_external/fonts.example.test/")),
        "{manifest:?}"
    );
    assert_eq!(
        workspace.read_with(vcx, |w, cx| pages::missing_files(
            w.library().read(cx),
            &manifest
        )),
        0
    );
    // One summary, without a button (everything was added, the first is open).
    assert_eq!(
        notes(&workspace, vcx),
        [("2 pages added".to_string(), None)]
    );
    assert!(state_line(&workspace, vcx).contains("adding=[idle]"));
    assert!(!available(&CancelAddingPages, vcx));
}

/// A page that cannot be kept whole is not added; the summary says so and
/// Show Why opens the report with the whole sentence; `escape` closes it.
#[gpui::test]
fn a_refusal_is_summed_up_and_show_why_has_the_whole_sentence(cx: &mut TestAppContext) {
    let (fake, workspace, text, vcx) = import_desk(Arc::new(MapFetcher::new()), cx);
    let input = files_of(&[("grades.html", GRADES)]);
    fake.set_check(CheckReport {
        engine: "fake".into(),
        elapsed_ms: 3,
        loads: vec![LoadReport {
            path: "grades.html".into(),
            title: "Grades".into(),
            finished: true,
            blocked: vec![Blocked {
                url: "https://api.example.test/grades".into(),
                directive: "connect-src".into(),
            }],
            ..LoadReport::default()
        }],
        failure: None,
    });
    let path = input.path().join("grades.html");
    workspace.update_in(vcx, |w, window, cx| {
        w.import_pages(vec![path], None, window, cx).unwrap();
    });
    wait_until_added(&workspace, vcx);
    assert!(pages_in(&workspace, vcx).is_empty(), "nothing was saved");
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.open_document_id()),
        Some(text),
        "the text stays open"
    );
    let shown = notes(&workspace, vcx);
    assert_eq!(
        shown,
        [(
            "\u{201c}Grades\u{201d} was not added".to_string(),
            Some("Show Why".to_string())
        )]
    );
    let report = report(&workspace, vcx);
    let PageOutcome::NotAdded { sentence } = &report.pages[0].outcome else {
        panic!("{report:?}");
    };
    assert!(
        sentence.contains("https://api.example.test/grades"),
        "{sentence}"
    );
    assert!(sentence.ends_with("Nothing was saved."), "{sentence}");

    // The button of the summary is the command "Show the report of the
    // pages added last".
    let layer = workspace.read_with(vcx, |w, _| w.toast_layer().clone());
    let id = layer.read_with(vcx, |layer, _| layer.ids()[0]);
    layer.update_in(vcx, |layer, window, cx| {
        assert!(layer.run_action(id, window, cx));
    });
    vcx.run_until_parked();
    let dialog = workspace
        .read_with(vcx, |w, cx| w.modal_layer().read(cx).active::<PageReport>())
        .expect("Show Why opens the report");
    let copied = dialog.read_with(vcx, |dialog, _| dialog.text());
    assert!(copied.contains(sentence.as_str()), "{copied}");
    assert!(
        copied.contains("\u{201c}Grades\u{201d}: not added"),
        "{copied}"
    );
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    assert!(!workspace.read_with(vcx, |w, cx| w.modal_layer().read(cx).has_modal()));
    // From the palette too.
    vcx.dispatch_action(ShowPageReport);
    vcx.run_until_parked();
    assert!(workspace.read_with(vcx, |w, cx| {
        w.modal_layer().read(cx).active::<PageReport>().is_some()
    }));
}

/// Files dropped on the sidebar go into the folder under the pointer, on
/// an open page and on the text into the folder selected; what is no page
/// is left out and counted.
#[gpui::test]
fn pages_dropped_on_the_sidebar_a_page_and_the_text_are_added(cx: &mut TestAppContext) {
    let (_fake, workspace, text, vcx) = import_desk(Arc::new(MapFetcher::new()), cx);
    let folder = workspace.update(vcx, |w, cx| {
        w.library()
            .update(cx, |library, cx| {
                library.create_folder(None, "Claude Pages", cx)
            })
            .unwrap()
            .id
    });
    vcx.run_until_parked();
    let input = files_of(&[
        (
            "one.html",
            "<html><head><title>One</title></head><body>1</body></html>",
        ),
        (
            "two.htm",
            "<html><head><title>Two</title></head><body>2</body></html>",
        ),
        (
            "three.html",
            "<html><head><title>Three</title></head><body>3</body></html>",
        ),
        ("notes.txt", "not a page"),
    ]);
    let at = |name: &str| input.path().join(name);

    // The sidebar: the panel says where.
    let panel = workspace.read_with(vcx, |w, _| w.panel().clone());
    let dropped = vec![at("one.html"), at("notes.txt")];
    panel.update(vcx, |_, cx| {
        cx.emit(writer_library::LibraryPanelEvent::DropPaths {
            paths: dropped,
            folder: Some(folder),
        })
    });
    wait_until_added(&workspace, vcx);
    let added = pages_in(&workspace, vcx);
    assert_eq!(added.len(), 1);
    assert_eq!((added[0].1.as_str(), added[0].2), ("One", Some(folder)));
    let one = added[0].0;
    let report = report(&workspace, vcx);
    assert_eq!(
        writer_app::page_report::left_out_names(&report),
        ["notes.txt"]
    );
    assert_eq!(
        notes(&workspace, vcx).last().cloned(),
        Some((
            "Added \u{201c}One\u{201d}. 1 file was not a page and was left out".to_string(),
            Some("Show Why".to_string())
        ))
    );
    assert_eq!(workspace.read_with(vcx, |w, _| w.open_page_id()), Some(one));

    // On the open page.
    workspace.update_in(vcx, |w, window, cx| {
        w.files_dropped_on_page(vec![at("two.htm")], window, cx)
            .unwrap();
    });
    wait_until_added(&workspace, vcx);
    let added = pages_in(&workspace, vcx);
    let two = added
        .iter()
        .find(|(_, title, _)| title == "Two")
        .expect("Two")
        .0;
    assert_eq!(workspace.read_with(vcx, |w, _| w.open_page_id()), Some(two));

    // On the text: the editor hands the web page to the import of pages,
    // and nothing comes into the text.
    workspace.update_in(vcx, |w, window, cx| {
        assert!(w.open_document(text, true, window, cx));
    });
    vcx.run_until_parked();
    let before = stored_text(
        &workspace.read_with(vcx, |w, _| w.library().clone()),
        text,
        vcx,
    );
    let editor = workspace.read_with(vcx, |w, _| w.editor().clone());
    editor.update_in(vcx, |editor, window, cx| {
        editor.drop_files(
            &[at("three.html")],
            gpui::point(gpui::px(200.), gpui::px(200.)),
            window,
            cx,
        );
    });
    vcx.run_until_parked();
    wait_until_added(&workspace, vcx);
    assert!(
        pages_in(&workspace, vcx)
            .iter()
            .any(|(_, title, _)| title == "Three"),
        "the page dropped on the text was added"
    );
    // The page opened in place of the text, which is as it was: no
    // picture came in.
    let three = pages_in(&workspace, vcx)
        .into_iter()
        .find(|(_, title, _)| title == "Three")
        .unwrap()
        .0;
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.open_page_id()),
        Some(three)
    );
    assert_eq!(
        stored_text(
            &workspace.read_with(vcx, |w, _| w.library().clone()),
            text,
            vcx
        ),
        before
    );
    assert!(
        !notes(&workspace, vcx)
            .iter()
            .any(|(message, _)| message.contains("could not be used")),
        "{:?}",
        notes(&workspace, vcx)
    );
}

/// A fetcher that answers only when the test lets it.
#[derive(Default)]
struct HeldFetcher {
    open: Mutex<bool>,
    moved: Condvar,
    asked: AtomicUsize,
}

impl HeldFetcher {
    fn release(&self) {
        *self.open.lock().unwrap() = true;
        self.moved.notify_all();
    }

    fn wait_until_asked(&self, cx: &mut VisualTestContext) {
        let started = Instant::now();
        while self.asked.load(Ordering::SeqCst) == 0 {
            cx.run_until_parked();
            assert!(
                started.elapsed() < Duration::from_secs(30),
                "nothing was fetched"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        cx.run_until_parked();
    }
}

impl Fetcher for HeldFetcher {
    fn fetch(&self, url: &str) -> Result<Fetched, FetchError> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        let mut open = self.open.lock().unwrap();
        while !*open {
            open = self.moved.wait(open).unwrap();
        }
        Ok(Fetched {
            final_url: url.to_string(),
            media_type: Some("text/css".to_string()),
            bytes: b"body {}".to_vec(),
        })
    }
}

const HELD: &str = "<html><head><title>Held</title>\
<link rel=\"stylesheet\" href=\"https://cdn.example.test/held.css\"></head><body>x</body></html>";

/// While a fetch takes long the note says so with Cancel; Cancel stops it
/// and nothing is written.
#[gpui::test]
fn cancel_while_a_fetch_is_held_adds_nothing(cx: &mut TestAppContext) {
    let fetcher = Arc::new(HeldFetcher::default());
    let (fake, workspace, _text, vcx) = import_desk(fetcher.clone(), cx);
    let input = files_of(&[("held.html", HELD)]);
    workspace.update_in(vcx, |w, window, cx| {
        w.import_pages(vec![input.path().join("held.html")], None, window, cx)
            .unwrap();
    });
    fetcher.wait_until_asked(vcx);
    assert!(
        state_line(&workspace, vcx).contains("adding=[1/1 \"held\" fetching]"),
        "{}",
        state_line(&workspace, vcx)
    );
    assert!(
        notes(&workspace, vcx).is_empty(),
        "no note before its moment"
    );
    vcx.executor().advance_clock(ADDING_NOTE_DELAY);
    vcx.run_until_parked();
    assert_eq!(
        notes(&workspace, vcx),
        [(
            "Adding \u{201c}held\u{201d}\u{2026} (fetching what it loads from the web)".to_string(),
            Some("Cancel".to_string())
        )]
    );
    vcx.dispatch_action(CancelAddingPages);
    vcx.run_until_parked();
    assert!(!workspace.read_with(vcx, |w, _| w.is_adding_pages()));
    fetcher.release();
    std::thread::sleep(Duration::from_millis(50));
    vcx.run_until_parked();
    assert!(pages_in(&workspace, vcx).is_empty());
    assert_eq!(fake.checks_asked(), 0, "what was being fetched was dropped");
    assert_eq!(
        workspace.read_with(vcx, |w, cx| w
            .library()
            .read(cx)
            .store()
            .file_count()
            .unwrap()),
        0
    );
    assert_eq!(
        notes(&workspace, vcx),
        [("Stopped adding pages: none was added".to_string(), None)]
    );
    assert!(report(&workspace, vcx).cancelled);
}

/// The note does not go while pages are being added: pushed out by newer
/// notes it comes back, taken away by the writer it stops the work.
#[gpui::test]
fn the_note_comes_back_when_pushed_out_and_a_click_on_it_cancels(cx: &mut TestAppContext) {
    let fetcher = Arc::new(HeldFetcher::default());
    let (_fake, workspace, _text, vcx) = import_desk(fetcher.clone(), cx);
    let input = files_of(&[("held.html", HELD)]);
    workspace.update_in(vcx, |w, window, cx| {
        w.import_pages(vec![input.path().join("held.html")], None, window, cx)
            .unwrap();
    });
    fetcher.wait_until_asked(vcx);
    vcx.executor().advance_clock(ADDING_NOTE_DELAY);
    vcx.run_until_parked();
    let note = workspace
        .read_with(vcx, |w, _| w.adding_note())
        .expect("the note");
    for n in 0..4 {
        workspace.update(vcx, |w, cx| {
            w.show_toast(
                Toast::new(format!("Other {n}")).duration(Duration::from_secs(60)),
                cx,
            )
        });
    }
    vcx.run_until_parked();
    let again = workspace
        .read_with(vcx, |w, _| w.adding_note())
        .expect("shown again");
    assert_ne!(again, note);
    assert!(
        workspace.read_with(vcx, |w, cx| w.toast_layer().read(cx).ids().contains(&again)),
        "as the newest"
    );
    assert!(workspace.read_with(vcx, |w, _| w.is_adding_pages()));
    // A click on the note takes it away: the work stops.
    let layer = workspace.read_with(vcx, |w, _| w.toast_layer().clone());
    layer.update(vcx, |layer, cx| {
        layer.dismiss(again, cx);
        layer.dismiss(layer.ids()[0], cx);
    });
    vcx.run_until_parked();
    assert!(!workspace.read_with(vcx, |w, _| w.is_adding_pages()));
    fetcher.release();
    std::thread::sleep(Duration::from_millis(50));
    vcx.run_until_parked();
    assert!(pages_in(&workspace, vcx).is_empty());
}

/// A store that lost the files of a page (a write in the background that
/// did not land): the row is refused, the page is not added.
#[gpui::test]
fn a_page_whose_files_were_lost_is_not_added(cx: &mut TestAppContext) {
    let (_fake, workspace, _text, vcx) = import_desk(Arc::new(MapFetcher::new()), cx);
    workspace.read_with(vcx, |w, _| w.library_images().lose_page_files(true));
    let input = files_of(&[("grades.html", GRADES)]);
    workspace.update_in(vcx, |w, window, cx| {
        w.import_pages(vec![input.path().join("grades.html")], None, window, cx)
            .unwrap();
    });
    wait_until_added(&workspace, vcx);
    assert!(pages_in(&workspace, vcx).is_empty());
    let report = report(&workspace, vcx);
    let PageOutcome::NotAdded { sentence } = &report.pages[0].outcome else {
        panic!("{report:?}");
    };
    assert!(
        sentence.starts_with(
            "\u{201c}Grades\u{201d} was not added: its files could not be stored in the library"
        ),
        "{sentence}"
    );
}

/// A folder the bundler cannot use is refused in the app's words (no
/// option of the command line).
#[gpui::test]
fn an_input_that_cannot_be_used_says_so_in_the_apps_words(cx: &mut TestAppContext) {
    let (_fake, workspace, _text, vcx) = import_desk(Arc::new(MapFetcher::new()), cx);
    let input = files_of(&[
        ("Two pages/a.html", "<html><body>a</body></html>"),
        ("Two pages/b.html", "<html><body>b</body></html>"),
    ]);
    workspace.update_in(vcx, |w, window, cx| {
        w.import_pages(vec![input.path().join("Two pages")], None, window, cx)
            .unwrap();
    });
    wait_until_added(&workspace, vcx);
    let report = report(&workspace, vcx);
    let PageOutcome::NotAdded { sentence } = &report.pages[0].outcome else {
        panic!("{report:?}");
    };
    assert!(
        sentence.starts_with("\u{201c}Two pages\u{201d} was not added: "),
        "{sentence}"
    );
    assert!(!sentence.contains("--entry"), "{sentence}");
    assert!(sentence.ends_with("Nothing was saved."), "{sentence}");
}

/// Without a page engine nothing is imported: no Import Page…, and a drop
/// of a web page says why.
#[gpui::test]
fn without_an_engine_there_is_no_import(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    add_document(&library, None, &simple_document("Notes", "A text."), cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    assert!(!available(&ImportPage, vcx));
    vcx.simulate_keystrokes("cmd-shift-p");
    let titles: Vec<String> = workspace.read_with(vcx, |w, cx| {
        w.modal_layer()
            .read(cx)
            .active::<CommandPalette>()
            .expect("the palette")
            .read(cx)
            .delegate
            .listed_titles()
    });
    assert!(
        !titles.iter().any(|title| title == "Import page…"),
        "{titles:?}"
    );
    vcx.simulate_keystrokes("escape");
    let input = files_of(&[("grades.html", GRADES)]);
    let panel = workspace.read_with(vcx, |w, _| w.panel().clone());
    let dropped = vec![input.path().join("grades.html")];
    panel.update(vcx, |_, cx| {
        cx.emit(writer_library::LibraryPanelEvent::DropPaths {
            paths: dropped,
            folder: None,
        })
    });
    vcx.run_until_parked();
    assert!(
        toasts(&workspace, vcx)
            .iter()
            .any(|message| message == "Pages cannot be added in this run."),
        "{:?}",
        toasts(&workspace, vcx)
    );
    assert!(pages_in(&workspace, vcx).is_empty());
}

#[test]
fn the_summary_says_what_came_of_a_gesture() {
    let page = |name: &str, outcome: PageOutcome| writer_app::workspace::AddedPage {
        path: PathBuf::from(format!("/in/{name}.html")),
        name: name.to_string(),
        outcome,
    };
    let added = |name: &str, warnings: usize| {
        page(
            name,
            PageOutcome::Added {
                id: DocumentId(1),
                files: 3,
                bytes: 1_000_000,
                externals: 2,
                warnings: (0..warnings).map(|n| format!("warning {n}")).collect(),
            },
        )
    };
    let refused = |name: &str| {
        page(
            name,
            PageOutcome::NotAdded {
                sentence: format!(
                    "\u{201c}{name}\u{201d} was not added: it could not be kept whole. Nothing was saved."
                ),
            },
        )
    };
    let report = |pages: Vec<writer_app::workspace::AddedPage>,
                  left_out: usize,
                  cancelled: bool| PagesReport {
        pages,
        left_out: (0..left_out)
            .map(|n| PathBuf::from(format!("/in/{n}.txt")))
            .collect(),
        cancelled,
    };
    let one = report(vec![added("Orbitarium", 0)], 0, false);
    assert_eq!(one.summary(), "Added \u{201c}Orbitarium\u{201d}");
    assert!(!one.needs_why());
    let warned = report(vec![added("Desk", 1)], 0, false);
    assert_eq!(
        warned.summary(),
        "Added \u{201c}Desk\u{201d}, with 1 warning"
    );
    assert!(warned.needs_why());
    assert_eq!(
        report(vec![refused("Growth")], 0, false).summary(),
        "\u{201c}Growth\u{201d} was not added"
    );
    let mixed = report(vec![added("A", 0), added("B", 0), refused("C")], 3, false);
    assert_eq!(
        mixed.summary(),
        "2 pages added, 1 not added. 3 files were not pages and were left out"
    );
    assert_eq!(
        report(vec![refused("A"), refused("B")], 0, false).summary(),
        "None of the 2 pages was added"
    );
    assert_eq!(
        report(Vec::new(), 1, false).summary(),
        "1 file was not a page and was left out"
    );
    assert_eq!(
        report(
            vec![added("A", 0), page("B", PageOutcome::Cancelled)],
            0,
            true
        )
        .summary(),
        "Stopped adding pages: 1 page added, 1 not added"
    );
    let text = mixed.text();
    assert!(
        text.contains("\u{201c}C\u{201d}: not added\n  \u{201c}C\u{201d} was not added"),
        "{text}"
    );
    assert!(text.contains("/in/2.txt"), "{text}");
}

#[test]
fn a_page_input_is_an_html_file_or_a_folder() {
    let input = files_of(&[
        ("a.html", "x"),
        ("b.HTM", "x"),
        ("c.txt", "x"),
        ("d/index.html", "x"),
    ]);
    let at = |name: &str| input.path().join(name);
    assert!(pages::is_page_input(&at("a.html")));
    assert!(pages::is_page_input(&at("b.HTM")));
    assert!(!pages::is_page_input(&at("c.txt")));
    assert!(pages::is_page_input(&at("d")));
    assert!(!pages::is_page_input(Path::new("/no/such/page.html")));
    assert_eq!(pages::input_name(&at("a.html")), "a");
    assert_eq!(pages::input_name(&at("d")), "d");
}
