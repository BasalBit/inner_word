//! The shell: sidebar, focus keys, mount points, status bar, window bounds.

use gpui::{AnyView, Context, IntoElement, Render, TestAppContext, Window, div, prelude::*, px};
use writer_app::{
    settings::{DEFAULT_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH, MIN_SIDEBAR_WIDTH, keys},
    window::{StoredBounds, StoredWindowMode, stored_window_bounds},
};
use writer_core::{Position, Selection};

use super::*;

struct Marker(&'static str);

impl Render for Marker {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let name = self.0;
        div()
            .debug_selector(move || name.to_string())
            .h(px(20.))
            .w_full()
            .child(name)
    }
}

fn marker(name: &'static str, cx: &mut VisualTestContext) -> AnyView {
    cx.update(|_, cx| cx.new(|_| Marker(name)).into())
}

fn panel_focused(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> bool {
    workspace.update_in(cx, |workspace, window, cx| {
        use gpui::Focusable as _;
        workspace
            .panel()
            .focus_handle(cx)
            .contains_focused(window, cx)
    })
}

#[gpui::test]
fn sidebar_focus_keys_and_persistence(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    add_document(&library, None, &simple_document("One", "Text."), cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    let setting = |key: &'static str, cx: &mut VisualTestContext| -> Option<f32> {
        library.read_with(cx, |library, _| library.setting::<f32>(key).unwrap())
    };

    // cmd-0 and cmd-1 move between the library and the editor; escape in
    // the library goes back to the text.
    assert!(editor_focused(&workspace, vcx));
    vcx.simulate_keystrokes("cmd-0");
    assert!(panel_focused(&workspace, vcx));
    vcx.simulate_keystrokes("cmd-1");
    assert!(editor_focused(&workspace, vcx));
    vcx.simulate_keystrokes("cmd-0 escape");
    assert!(editor_focused(&workspace, vcx));

    // Focus never ends up nowhere: if whatever had it goes away, the
    // editor takes it and the app keys keep working.
    vcx.update(|window, _| window.blur());
    workspace.update(vcx, |_, cx| cx.notify());
    vcx.run_until_parked();
    assert!(editor_focused(&workspace, vcx));

    // Keyboard resize, clamped and stored.
    vcx.simulate_keystrokes("cmd-alt-right");
    let width = workspace.read_with(vcx, |w, _| w.settings().sidebar_width);
    assert!(width > DEFAULT_SIDEBAR_WIDTH);
    assert_eq!(setting(keys::SIDEBAR_WIDTH, vcx), Some(width));
    for _ in 0..40 {
        vcx.simulate_keystrokes("cmd-alt-left");
    }
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.settings().sidebar_width),
        MIN_SIDEBAR_WIDTH
    );
    workspace.update(vcx, |workspace, cx| workspace.set_sidebar_width(9999., cx));
    assert_eq!(setting(keys::SIDEBAR_WIDTH, vcx), Some(MAX_SIDEBAR_WIDTH));

    // Mouse resize: drag the edge.
    let edge = gpui::point(px(MAX_SIDEBAR_WIDTH), px(300.));
    vcx.simulate_mouse_down(edge, gpui::MouseButton::Left, gpui::Modifiers::default());
    vcx.simulate_mouse_move(
        gpui::point(px(300.), px(320.)),
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    vcx.simulate_mouse_up(
        gpui::point(px(300.), px(320.)),
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.settings().sidebar_width),
        300.
    );
    assert_eq!(setting(keys::SIDEBAR_WIDTH, vcx), Some(300.));

    // cmd-alt-s hides it; hiding while the library has focus hands focus
    // to the editor; cmd-0 brings it back.
    vcx.simulate_keystrokes("cmd-0 cmd-alt-s");
    assert!(!workspace.read_with(vcx, |w, _| w.settings().sidebar_visible));
    assert!(editor_focused(&workspace, vcx));
    assert_eq!(
        library.read_with(vcx, |library, _| {
            library.setting::<bool>(keys::SIDEBAR_VISIBLE).unwrap()
        }),
        Some(false)
    );
    vcx.simulate_keystrokes("cmd-0");
    assert!(workspace.read_with(vcx, |w, _| w.settings().sidebar_visible));
    assert!(panel_focused(&workspace, vcx));
}

#[gpui::test]
fn mount_points_render_what_is_plugged_in(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    add_document(&library, None, &simple_document("One", "Text."), cx);
    let (workspace, vcx) = open_workspace(&library, cx);

    let host = marker("editor-host", vcx);
    let right = marker("right-panel-view", vcx);
    let find = marker("find-bar", vcx);
    let command_line = marker("vim-command-line", vcx);
    let mode = marker("vim-mode", vcx);
    workspace.update(vcx, |workspace, cx| {
        workspace.set_editor_host(Some(host), cx);
        workspace.set_right_panel(Some(right), cx);
        workspace.set_right_panel_width(px(300.), cx);
        workspace.set_editor_strip("find", Some(find), cx);
        workspace.set_editor_strip("vim", Some(command_line), cx);
        workspace.set_status_item("vim", Some(mode), cx);
    });
    vcx.run_until_parked();
    let bounds = |name: &'static str, cx: &mut VisualTestContext| {
        cx.debug_bounds(name)
            .unwrap_or_else(|| panic!("{name} was not rendered"))
    };
    let window_size = vcx.update(|window, _| window.viewport_size());
    let host_bounds = bounds("editor-host", vcx);
    let right_bounds = bounds("right-panel-view", vcx);
    let find_bounds = bounds("find-bar", vcx);
    let line_bounds = bounds("vim-command-line", vcx);
    let mode_bounds = bounds("vim-mode", vcx);
    // The right panel is at the right edge, 300 wide (minus its hairline).
    assert_eq!(right_bounds.right(), window_size.width);
    assert!((f32::from(right_bounds.size.width) - 299.).abs() < 1.5);
    // The strips stack under the editor area in insertion order, between
    // the sidebar and the right panel.
    assert!(host_bounds.top() < find_bounds.top());
    assert_eq!(find_bounds.bottom(), line_bounds.top());
    assert!(find_bounds.right() <= right_bounds.left());
    assert!(f32::from(find_bounds.left()) >= DEFAULT_SIDEBAR_WIDTH);
    // The status item is in the status bar, at the bottom, left of centre.
    assert!(mode_bounds.top() >= line_bounds.bottom());
    assert!(mode_bounds.bottom() <= window_size.height);
    assert!(mode_bounds.left() < window_size.width / 2.);

    // Removing by key.
    workspace.update(vcx, |workspace, cx| {
        workspace.set_editor_strip("find", None, cx);
        workspace.set_right_panel(None, cx);
    });
    vcx.run_until_parked();
    assert!(!workspace.read_with(vcx, |w, _| w.has_right_panel()));
    let line_after = bounds("vim-command-line", vcx);
    assert_eq!(line_after.bottom(), line_bounds.bottom());
}

#[gpui::test]
fn status_bar_counts_words_selection_and_pages(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    add_document(
        &library,
        None,
        &simple_document("Count me", "One two three four five."),
        cx,
    );
    let (workspace, vcx) = open_workspace(&library, cx);
    let status = workspace.update(vcx, |w, cx| w.status_line(cx));
    assert_eq!(status.save, "Saved");
    assert_eq!(status.words, "7 words");
    assert_eq!(status.characters, "32 characters");
    assert_eq!(status.reading, "1 min read");
    assert_eq!(status.limit, None);

    // With a selection the counts are the selection's.
    let editor = workspace.read_with(vcx, |w, _| w.editor().clone());
    editor.update(vcx, |editor, cx| {
        editor.edit(
            |state| state.set_selection(Selection::new(Position::new(1, 0), Position::new(1, 7))),
            cx,
        );
    });
    vcx.run_until_parked();
    let status = workspace.update(vcx, |w, cx| w.status_line(cx));
    assert_eq!(status.words, "2 of 7 words");
    assert_eq!(status.characters, "7 characters selected");
    assert_eq!(status.reading, "");

    // Page x of y appears in page view.
    assert_eq!(status.page, None);
    vcx.simulate_keystrokes("cmd-alt-p");
    let status = workspace.update(vcx, |w, cx| w.status_line(cx));
    assert_eq!(status.page.as_deref(), Some("Page 1 of 1"));
    assert_eq!(status.save, "Edited");

    // The automation hook's `state` step reads the same things.
    let state = workspace.update_in(vcx, |workspace, window, cx| {
        writer_app::automation::describe_state(workspace, window, cx)
    });
    for part in [
        "title=\"Count me\"",
        "save=Some(Edited)",
        "focus=editor",
        "modal=false",
        "page_view=true",
        "page=Some(\"Page 1 of 1\")",
    ] {
        assert!(state.contains(part), "{part} in {state}");
    }

    // No document: an empty status line.
    workspace.update_in(vcx, |workspace, window, cx| {
        workspace.close_document(window, cx)
    });
    let status = workspace.update(vcx, |w, cx| w.status_line(cx));
    assert_eq!(status, Default::default());
}

#[gpui::test]
fn window_bounds_are_stored_and_read_back(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    // Nothing stored yet.
    assert!(vcx.update(|_, cx| stored_window_bounds(library.read(cx), cx).is_none()));

    // A resize is stored after it settles.
    vcx.simulate_resize(gpui::size(px(900.), px(700.)));
    vcx.executor()
        .advance_clock(std::time::Duration::from_secs(2));
    vcx.run_until_parked();
    let stored: Option<StoredBounds> = library.read_with(vcx, |library, _| {
        library.setting(keys::WINDOW_BOUNDS).unwrap()
    });
    let stored = stored.expect("the bounds were stored");
    assert_eq!((stored.width, stored.height), (900., 700.));
    assert_eq!(stored.mode, StoredWindowMode::Windowed);
    let restored = vcx.update(|_, cx| stored_window_bounds(library.read(cx), cx));
    assert_eq!(
        restored.map(|bounds| bounds.get_bounds().size),
        Some(gpui::size(px(900.), px(700.)))
    );

    // Bounds that make no sense are ignored at the next launch.
    library.update(vcx, |library, cx| {
        library
            .set_setting(
                keys::WINDOW_BOUNDS,
                &StoredBounds {
                    width: 10.,
                    ..stored
                },
                cx,
            )
            .unwrap()
    });
    assert!(vcx.update(|_, cx| stored_window_bounds(library.read(cx), cx).is_none()));
    let _ = workspace;
}

#[gpui::test]
fn menu_bar_follows_the_macos_layout(cx: &mut TestAppContext) {
    init(cx);
    let menus = writer_app::menus::app_menus();
    let names: Vec<String> = menus.iter().map(|menu| menu.name.to_string()).collect();
    assert_eq!(
        names,
        ["The Writer", "File", "Edit", "View", "Window", "Help"]
    );
    let items = |menu: &gpui::Menu| -> Vec<String> {
        menu.items
            .iter()
            .filter_map(|item| match item {
                gpui::MenuItem::Action { name, .. } => Some(name.to_string()),
                _ => None,
            })
            .collect()
    };
    let app = items(&menus[0]);
    assert_eq!(app.first().map(String::as_str), Some("About The Writer"));
    assert!(app.contains(&"Settings…".to_string()));
    assert_eq!(app.last().map(String::as_str), Some("Quit The Writer"));
    assert_eq!(
        items(&menus[2]),
        [
            "Undo",
            "Redo",
            "Cut",
            "Copy",
            "Paste",
            "Paste as Plain Text",
            "Select All",
            "Find…",
            "Find and Replace…",
            "Find Next",
            "Find Previous",
            "Add Comment"
        ]
    );
    assert!(items(&menus[3]).contains(&"Page View".to_string()));
    assert_eq!(items(&menus[5]), ["Keyboard Shortcuts"]);
    // Every menu action has a handler somewhere: it is either a registered
    // command or one of the window and app actions.
    let known: Vec<&'static str> = cx.update(|cx| {
        writer_app::commands::all(cx)
            .iter()
            .map(|command| command.action_name())
            .collect()
    });
    let extra = [
        "app::Hide",
        "app::HideOthers",
        "app::ShowAll",
        "app::Minimize",
        "app::Zoom",
        "editor::Undo",
        "editor::Redo",
        "editor::Cut",
        "editor::Copy",
        "editor::Paste",
        "editor::PasteAsPlainText",
        "editor::SelectAll",
    ];
    for menu in &menus {
        for item in &menu.items {
            if let gpui::MenuItem::Action { name, action, .. } = item {
                assert!(
                    known.contains(&action.name()) || extra.contains(&action.name()),
                    "menu item {name} dispatches {}, which nothing handles",
                    action.name()
                );
            }
        }
    }
}

#[gpui::test]
fn extensions_can_mount_when_a_workspace_is_created(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    add_document(&library, None, &simple_document("One", "Text."), cx);
    // What an extension crate's glue does at startup: wait for the
    // workspace, then plug its views in.
    let had_window = std::rc::Rc::new(std::cell::Cell::new(None));
    let _subscription = cx.update(|cx| {
        let had_window = had_window.clone();
        cx.observe_new::<Workspace>(move |workspace, window, cx| {
            had_window.set(Some(window.is_some()));
            let item: AnyView = cx.new(|_| Marker("mounted-at-creation")).into();
            workspace.set_status_item("probe", Some(item), cx);
            // The session is already restored at this point.
            assert!(workspace.open_document_id().is_some());
        })
    });
    let (_workspace, vcx) = open_workspace(&library, cx);
    assert_eq!(had_window.get(), Some(true));
    assert!(vcx.debug_bounds("mounted-at-creation").is_some());
}

#[gpui::test]
fn library_commands_work_from_the_editor_and_renaming_pins_the_title(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    let id = add_document(&library, None, &simple_document("Draft one", "Text."), cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    let panel = workspace.read_with(vcx, |w, _| w.panel().clone());
    workspace.update(vcx, |workspace, cx| {
        workspace.write_setting(keys::NEW_DOCUMENT_FONT, "Georgia", cx)
    });

    // With the sidebar hidden and focus in the text, "Rename" from the
    // palette brings the sidebar back and starts renaming the open document.
    vcx.simulate_keystrokes("cmd-alt-s");
    assert!(editor_focused(&workspace, vcx));
    vcx.simulate_keystrokes("cmd-shift-p");
    vcx.simulate_input("rename");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert!(workspace.read_with(vcx, |w, _| w.settings().sidebar_visible));
    assert_eq!(
        panel.read_with(vcx, |panel, _| panel.renaming()),
        Some(writer_library::LibraryItem::Document(id))
    );
    vcx.simulate_keystrokes("cmd-a");
    vcx.simulate_input("Chosen in the sidebar");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    let title = |id, cx: &mut VisualTestContext| {
        library.read_with(cx, |library, _| library.document_meta(id).unwrap().title)
    };
    assert_eq!(title(id, vcx), "Chosen in the sidebar");
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.document_title().map(str::to_string)),
        Some("Chosen in the sidebar".to_string())
    );
    // Editing the text no longer changes the title.
    vcx.simulate_keystrokes("cmd-1");
    assert!(editor_focused(&workspace, vcx));
    vcx.simulate_input("Another first line. ");
    workspace.update(vcx, |workspace, cx| workspace.flush(cx));
    assert!(stored_text(&library, id, vcx).starts_with("Another first line."));
    assert_eq!(title(id, vcx), "Chosen in the sidebar");

    // The panel's own "new document" key stores a document with the
    // user's defaults, and naming it there pins that name too.
    vcx.simulate_keystrokes("cmd-0 n");
    vcx.run_until_parked();
    vcx.simulate_input("Named at birth");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    let created = library.read_with(vcx, |library, _| {
        library
            .all_documents()
            .unwrap()
            .into_iter()
            .find(|entry| entry.meta.id != id)
            .expect("the panel created a document")
            .meta
    });
    assert_eq!(created.title, "Named at birth");
    let content = library.read_with(vcx, |library, _| {
        library.document(created.id).unwrap().content
    });
    let document = Document::from_json(&content).unwrap();
    assert_eq!(document.settings().typography.body_font, "Georgia");
    // It opens in the editor; writing in it keeps the given name.
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.open_document_id()),
        Some(created.id)
    );
    type_text(&workspace, "Body text", vcx);
    workspace.update(vcx, |workspace, cx| workspace.flush(cx));
    assert_eq!(title(created.id, vcx), "Named at birth");
    assert_eq!(title(id, vcx), "Chosen in the sidebar");
}
