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
    for key in ["a", "space", "escape", "up", "tab"] {
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
