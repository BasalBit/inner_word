//! Everything with the mouse alone: the menu bar as the map of the app, the
//! menus of the title and the status bar, what the app adds to the menus of
//! the text and the library, and modals that close and work on a click.
//!
//! Pointer events are simulated (`simulate_click`, `simulate_mouse_down`);
//! where a menu item is clicked, the click goes to where the item was drawn.

use gpui::{
    Bounds, Focusable as _, Modifiers, MouseButton, Pixels, Point, TestAppContext, point, px,
};
use writer_app::{
    actions::{NewDocument, OpenWelcomeDocument, SaveNow},
    export::{ExportPicker, ExportScope, ExportTarget},
    menus,
    settings_view::{SettingRow, SettingsView},
    shortcuts::{MENUS_NOTE, collect_shortcuts},
    version_history::VersionHistory,
    welcome,
    workspace::{AUTOSAVE_DELAY, MenuKind},
};
use writer_core::{Position, Selection};
use writer_editor::actions as editor;
use writer_export::{ExportFormat, SocialNetwork};
use writer_library::{Clock, Library, LibraryItem};
use writer_ui::ContextMenu;

use super::*;

fn centre(bounds: Bounds<Pixels>) -> Point<Pixels> {
    bounds.center()
}

fn bounds_of(selector: &'static str, cx: &mut VisualTestContext) -> Bounds<Pixels> {
    cx.debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn"))
}

fn click(selector: &'static str, cx: &mut VisualTestContext) {
    let at = centre(bounds_of(selector, cx));
    cx.simulate_click(at, Modifiers::default());
    cx.run_until_parked();
}

fn right_click_at(at: Point<Pixels>, cx: &mut VisualTestContext) {
    cx.simulate_mouse_down(at, MouseButton::Right, Modifiers::default());
    cx.simulate_mouse_up(at, MouseButton::Right, Modifiers::default());
    cx.run_until_parked();
}

fn right_click(selector: &'static str, cx: &mut VisualTestContext) {
    let at = centre(bounds_of(selector, cx));
    right_click_at(at, cx);
}

/// The labels of the open context menu ("-" for a separator).
fn menu_labels(cx: &mut VisualTestContext) -> Option<Vec<String>> {
    cx.update(|window, cx| ContextMenu::current(window, cx).map(|menu| menu.read(cx).labels()))
}

/// Clicks the item called `label` of the open context menu, where it was
/// drawn.
fn click_menu_item(label: &str, cx: &mut VisualTestContext) {
    let at = cx.update(|window, cx| {
        let menu = ContextMenu::current(window, cx).expect("a menu is open");
        let menu = menu.read(cx);
        let ix = menu
            .labels()
            .iter()
            .position(|item| item == label)
            .unwrap_or_else(|| panic!("no item {label:?} in {:?}", menu.labels()));
        menu.item_bounds(ix).expect("the menu was drawn").center()
    });
    cx.simulate_click(at, Modifiers::default());
    cx.run_until_parked();
}

/// Chooses an item that may be in a submenu.
fn choose(label: &str, cx: &mut VisualTestContext) {
    assert!(
        cx.update(|window, cx| ContextMenu::choose(label, window, cx)),
        "no open menu has an item {label:?}"
    );
    cx.run_until_parked();
}

fn has_modal(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> bool {
    workspace.read_with(cx, |w, cx| w.modal_layer().read(cx).has_modal())
}

fn panel_focused(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> bool {
    workspace.update_in(cx, |w, window, cx| {
        w.panel().focus_handle(cx).contains_focused(window, cx)
    })
}

fn unparsed(binding: &gpui::KeyBinding) -> String {
    binding
        .keystrokes()
        .iter()
        .map(|keystroke| keystroke.unparse())
        .collect::<Vec<_>>()
        .join(" ")
}

#[gpui::test]
fn the_menu_bar_is_the_map_of_the_app(cx: &mut TestAppContext) {
    init(cx);
    // The whole map, with the items of the assistant (which the menu bar
    // only has on a Mac where the assistant's engine is installed; its
    // commands are listed only then too).
    cx.update(|cx| {
        writer_app::commands::set_category_hidden(
            writer_app::actions::CATEGORY_ASSISTANT,
            false,
            cx,
        )
    });
    let all = menus::app_menus();
    let names: Vec<String> = all.iter().map(|menu| menu.name.to_string()).collect();
    assert_eq!(
        names,
        [
            "The Writer",
            "File",
            "Edit",
            "Format",
            "Insert",
            "View",
            "Window",
            "Help"
        ]
    );
    // What each menu has to offer at least. A name followed by ">" is a
    // submenu.
    fn names_of(items: &[gpui::MenuItem], out: &mut Vec<String>) {
        for item in items {
            match item {
                gpui::MenuItem::Action { name, .. } => out.push(name.to_string()),
                gpui::MenuItem::Submenu(menu) => {
                    out.push(format!("{} >", menu.name));
                    names_of(&menu.items, out);
                }
                _ => {}
            }
        }
    }
    let items = |menu: &str| {
        let menu = all.iter().find(|candidate| candidate.name == menu).unwrap();
        let mut out = Vec::new();
        names_of(&menu.items, &mut out);
        out
    };
    let wanted: [(&str, &[&str]); 7] = [
        (
            "The Writer",
            &[
                "About The Writer",
                "Settings…",
                "Keyboard Shortcuts…",
                "Import Keyboard Shortcuts…",
                "Export Keyboard Shortcuts…",
                "Reset All Keyboard Shortcuts…",
                "Quit The Writer",
            ],
        ),
        (
            "File",
            &[
                "New Document",
                "New Folder",
                "Open Document…",
                "Save Now",
                "Rename…",
                "Export…",
                "Export as PDF…",
                "Cancel PDF Export",
                "Print…",
                "Cancel Printing",
                "Version History…",
                "Document Settings…",
                "Close Window",
            ],
        ),
        (
            "Edit",
            &[
                "Undo",
                "Redo",
                "Cut",
                "Copy",
                "Paste",
                "Select All",
                "Find…",
                "Find and Replace…",
            ],
        ),
        (
            "Format",
            &[
                "Bold",
                "Italic",
                "Underline",
                "Strikethrough",
                "Inline Code",
                "Highlight",
                "Add or Edit Link…",
                "Font…",
                "Turn Into >",
                "Paragraph",
                "Heading 1",
                "Heading 6",
                "Bullet List",
                "Numbered List",
                "To-do List",
                "Quote",
                "Code Block",
                "Align >",
                "Left",
                "Centre",
                "Right",
                "Justify",
                "Image >",
                "Larger",
                "Smaller",
                "25 %",
                "50 %",
                "75 %",
                "100 %",
                "Original Size",
                "Description…",
                "Larger Text",
                "Smaller Text",
                "Clear Formatting",
            ],
        ),
        (
            "Insert",
            &[
                "Divider",
                "Page Break",
                "Today's Date",
                "Image…",
                "Link…",
                "Comment",
            ],
        ),
        (
            "View",
            &[
                "Show or Hide Library",
                "Show or Hide Comments",
                "Turn Page View On or Off",
                "Turn Vim Mode On or Off",
                "Theme >",
                "Paper",
                "Night",
                "Command Palette…",
            ],
        ),
        ("Help", &["Keyboard Shortcuts…", "Welcome Document"]),
    ];
    for (menu, wanted) in wanted {
        let have = items(menu);
        for item in wanted {
            assert!(
                have.contains(&item.to_string()),
                "{menu} has no {item}: {have:?}"
            );
        }
    }
    // macOS adds its own full screen item to a menu called View; the app
    // does not list a second one.
    assert!(
        !items("View")
            .iter()
            .any(|item| item.contains("Full Screen"))
    );

    // Every item is an existing action: a registered command, or one of
    // the window and Edit menu actions.
    let actions = menus::menu_actions(&all);
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
    ];
    for (name, action) in &actions {
        assert!(
            known.contains(&action.name()) || extra.contains(&action.name()),
            "menu item {name} dispatches {}, which is not a command",
            action.name()
        );
    }

    // The key beside every item is a key that really runs its action (the
    // one GPUI picks for the menu is not always the one the app shows).
    cx.update(|cx| {
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        let menu_only = menus::is_menu_key_binding;
        for binding in keymap.bindings().filter(|binding| menu_only(binding)) {
            // A binding that only gives a menu item its key belongs to a
            // menu item.
            assert!(
                actions
                    .iter()
                    .any(|(_, action)| action.partial_eq(binding.action())),
                "{} has a menu key and no menu item",
                binding.action().name()
            );
        }
        for (name, action) in &actions {
            let real: Vec<&gpui::KeyBinding> = keymap
                .bindings_for_action(action.as_ref())
                .filter(|binding| !menu_only(binding))
                .collect();
            let native = menus::native_menu_binding(&keymap, action.as_ref());
            let Some(shown_everywhere) = real.last() else {
                assert!(native.is_none(), "{name} has no key and the menu shows one");
                continue;
            };
            let native = native.unwrap_or_else(|| panic!("{name} shows no binding"));
            let keys: Vec<String> = real.iter().map(|binding| unparsed(binding)).collect();
            if native.keystrokes().len() > 1 {
                // No key in the menu: only where the keys the app shows
                // for the command are ones the menu bar cannot be given
                // (return is in them).
                assert!(
                    menus::shown_binding(&keymap, action.as_ref()).is_none(),
                    "{name} shows no key although it has {keys:?}"
                );
                assert!(
                    unparsed(shown_everywhere).contains("enter"),
                    "{name} shows no key although the app shows {}",
                    unparsed(shown_everywhere)
                );
                continue;
            }
            let key = unparsed(native);
            assert!(
                !key.contains("enter") && !key.ends_with("tab") && menus::menu_can_show(native),
                "{name} shows {key}, which the menu bar cannot draw"
            );
            assert!(
                keys.contains(&key),
                "{name} shows {key}; its keys are {keys:?} (the app shows {})",
                unparsed(shown_everywhere)
            );
        }
        // With the default keys, three items show none.
        let without: Vec<&str> = actions
            .iter()
            .filter(|(_, action)| {
                menus::native_menu_binding(&keymap, action.as_ref())
                    .is_some_and(|binding| binding.keystrokes().len() > 1)
            })
            .map(|(name, _)| name.as_str())
            .collect();
        assert_eq!(
            without,
            ["Tick or Untick To-do", "Page Break", "Line Break"],
            "the items whose only key has return in it"
        );
        // The reported one: "Find Previous" showed shift-enter.
        let find_previous = writer_editor_tools::actions::FindPrevious;
        assert_eq!(
            menus::native_menu_binding(&keymap, &find_previous).map(unparsed),
            Some("cmd-shift-g".to_string())
        );
    });

    // Enabled only when it can apply: without a document nothing for the
    // text is on offer; with one it is, wherever the keyboard is.
    let library = library(cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    let available = |action: &dyn gpui::Action, cx: &mut VisualTestContext| {
        cx.update(|window, cx| window.is_action_available(action, cx))
    };
    assert!(available(&NewDocument, vcx));
    assert!(!available(&editor::ToggleBold, vcx));
    assert!(!available(&writer_editor_tools::actions::Find, vcx));
    vcx.dispatch_action(NewDocument);
    vcx.run_until_parked();
    assert!(available(&editor::ToggleBold, vcx));
    assert!(available(&writer_editor_tools::actions::Find, vcx));
    vcx.simulate_keystrokes("cmd-0");
    assert!(panel_focused(&workspace, vcx));
    assert!(available(&editor::ToggleBold, vcx));
    assert!(available(&editor::SetHeading1, vcx));
}

#[gpui::test]
fn the_title_has_a_menu_and_a_chevron(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    let id = add_document(
        &library,
        None,
        &simple_document("A title", "Some text."),
        cx,
    );
    let (workspace, vcx) = open_workspace(&library, cx);

    // A press on the title band or on the status bar is not a press on
    // the text: the keyboard stays where it is.
    let band = bounds_of("titlebar", vcx);
    vcx.simulate_click(
        point(band.left() + px(40.), band.center().y),
        Modifiers::default(),
    );
    let bar = bounds_of("status-bar", vcx);
    vcx.simulate_click(bar.center(), Modifiers::default());
    vcx.run_until_parked();
    assert!(editor_focused(&workspace, vcx));

    // A right-click on the title.
    right_click(MenuKind::Title.selector(), vcx);
    assert_eq!(
        menu_labels(vcx).unwrap(),
        [
            "Rename…",
            "Export…",
            "Export as PDF…",
            "Version History…",
            "Document Settings…",
            "-",
            "Reveal in Library"
        ]
    );
    // The keyboard has not moved, and a click elsewhere closes the menu
    // and does nothing else.
    assert!(editor_focused(&workspace, vcx));
    vcx.simulate_click(point(px(600.), px(500.)), Modifiers::default());
    vcx.run_until_parked();
    assert_eq!(menu_labels(vcx), None);

    // The chevron beside the title opens the same menu, under the title.
    click("title-chevron", vcx);
    let title = bounds_of(MenuKind::Title.selector(), vcx);
    let menu = vcx.update(|window, cx| {
        ContextMenu::current(window, cx)
            .expect("the chevron opened the menu")
            .read(cx)
            .bounds()
            .expect("the menu is drawn")
    });
    assert!(menu.top() >= title.bottom(), "{menu:?} under {title:?}");

    // Rename: the row of the document in the library is a field.
    click_menu_item("Rename…", vcx);
    assert_eq!(menu_labels(vcx), None);
    assert_eq!(
        workspace.read_with(vcx, |w, cx| w.panel().read(cx).renaming()),
        Some(LibraryItem::Document(id))
    );
    vcx.simulate_input("Renamed");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.document_title().map(str::to_string)),
        Some("Renamed".to_string())
    );

    // The others open what their keys and palette commands open.
    vcx.simulate_keystrokes("cmd-1");
    click("title-chevron", vcx);
    click_menu_item("Export…", vcx);
    assert!(workspace.read_with(vcx, |w, cx| {
        w.modal_layer().read(cx).is_open::<ExportPicker>()
    }));
    vcx.simulate_keystrokes("escape");
    click("title-chevron", vcx);
    click_menu_item("Version History…", vcx);
    assert!(workspace.read_with(vcx, |w, cx| {
        w.modal_layer().read(cx).is_open::<VersionHistory>()
    }));
    vcx.simulate_keystrokes("escape");
    click("title-chevron", vcx);
    click_menu_item("Document Settings…", vcx);
    let settings = workspace
        .read_with(vcx, |w, cx| {
            w.modal_layer().read(cx).active::<SettingsView>()
        })
        .expect("the settings are open");
    assert_eq!(
        settings.update_in(vcx, |view, window, cx| view.focused_row(window, cx)),
        Some(SettingRow::DocumentPageView)
    );
    vcx.simulate_keystrokes("escape");

    // Reveal in the library: the row is selected and has the keyboard.
    workspace.update(vcx, |w, cx| {
        w.panel().update(cx, |panel, cx| panel.select(None, cx));
    });
    click("title-chevron", vcx);
    click_menu_item("Reveal in Library", vcx);
    assert!(panel_focused(&workspace, vcx));
    assert_eq!(
        workspace.read_with(vcx, |w, cx| w.panel().read(cx).selected()),
        Some(LibraryItem::Document(id))
    );

    // Without a document there is nothing to offer: no chevron, no menu.
    workspace.update_in(vcx, |w, window, cx| w.close_document(window, cx));
    vcx.run_until_parked();
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.menu_anchor(MenuKind::Title)),
        None
    );
    assert!(!workspace.update_in(vcx, |w, window, cx| {
        w.open_menu(
            MenuKind::Title,
            Some(point(px(600.), px(20.)).into()),
            window,
            cx,
        )
    }));
}

#[gpui::test]
fn the_status_bar_items_take_clicks(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    add_document(
        &library,
        None,
        &simple_document("Counted", "One two three four five."),
        cx,
    );
    let (workspace, vcx) = open_workspace(&library, cx);
    let settings =
        |cx: &mut VisualTestContext| workspace.read_with(cx, |w, _| w.settings().clone());
    let status = |cx: &mut VisualTestContext| workspace.update(cx, |w, cx| w.status_line(cx));

    // "Saved": a click offers to save now. Choosing it saves at once.
    type_text(&workspace, "More. ", vcx);
    assert_eq!(status(vcx).save, "Edited");
    click(MenuKind::Save.selector(), vcx);
    assert_eq!(menu_labels(vcx).unwrap(), ["Save Now", "Version History…"]);
    assert!(
        editor_focused(&workspace, vcx),
        "the keyboard stays in the text"
    );
    click_menu_item("Save Now", vcx);
    assert_eq!(status(vcx).save, "Saved");
    // The same from its key, which the menu shows.
    type_text(&workspace, "And more. ", vcx);
    assert_eq!(status(vcx).save, "Edited");
    vcx.simulate_keystrokes("cmd-s");
    assert_eq!(status(vcx).save, "Saved");
    assert!(vcx.update(|_, cx| writer_ui::KeyHint::for_action(&SaveNow, cx).is_some()));
    // A right-click opens the same menu at the pointer.
    right_click(MenuKind::Save.selector(), vcx);
    assert_eq!(menu_labels(vcx).unwrap(), ["Save Now", "Version History…"]);
    vcx.simulate_keystrokes("escape");
    assert_eq!(menu_labels(vcx), None);

    // The counts: what is shown.
    click(MenuKind::Statistics.selector(), vcx);
    assert_eq!(
        menu_labels(vcx).unwrap(),
        [
            "Shown in the status bar",
            "Word Count",
            "Character Count",
            "Reading Time",
            "-",
            "Character Limit"
        ]
    );
    click_menu_item("Reading Time", vcx);
    assert!(!settings(vcx).status_reading_time);
    assert_eq!(status(vcx).reading, "");
    assert_ne!(status(vcx).words, "");
    // Everything off: the counts are gone, and a right-click on the bar
    // itself brings the menu back.
    click(MenuKind::Statistics.selector(), vcx);
    click_menu_item("Word Count", vcx);
    click(MenuKind::Statistics.selector(), vcx);
    click_menu_item("Character Count", vcx);
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.menu_anchor(MenuKind::Statistics)),
        None
    );
    let bar = bounds_of("status-bar", vcx);
    right_click_at(point(bar.center().x, bar.center().y), vcx);
    click_menu_item("Word Count", vcx);
    assert!(settings(vcx).status_words);
    assert_eq!(status(vcx).words, "9 words");

    // The character limit, from the submenu; then its own indicator.
    click(MenuKind::Statistics.selector(), vcx);
    choose("X (280)", vcx);
    assert_eq!(settings(vcx).character_limit, Some(SocialNetwork::X));
    assert!(status(vcx).limit.is_some());
    click(MenuKind::Limit.selector(), vcx);
    let checked = vcx.update(|window, cx| {
        let menu = ContextMenu::current(window, cx).expect("the limit menu");
        menu.read(cx)
            .items()
            .iter()
            .filter(|item| item.is_checked() == Some(true))
            .map(|item| item.title().to_string())
            .collect::<Vec<_>>()
    });
    assert_eq!(checked, ["X (280)"]);
    click_menu_item("Off", vcx);
    assert_eq!(settings(vcx).character_limit, None);
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.menu_anchor(MenuKind::Limit)),
        None
    );

    // The page indicator: a click goes back to one column, a right-click
    // offers what belongs to pages.
    vcx.simulate_keystrokes("cmd-alt-p");
    assert!(status(vcx).page.is_some());
    right_click(MenuKind::Page.selector(), vcx);
    let labels = menu_labels(vcx).unwrap();
    assert_eq!(labels[0], "Page View");
    assert!(labels.contains(&"Document Settings…".to_string()));
    vcx.simulate_keystrokes("escape");
    click(MenuKind::Page.selector(), vcx);
    assert!(!workspace.read_with(vcx, |w, cx| w.editor().read(cx).page_view()));
    assert!(editor_focused(&workspace, vcx));

    // The Vim chip has the Vim crate's menu, with the app's toggle (the
    // one that has the key and stores the setting) and the settings in it.
    vcx.simulate_keystrokes("cmd-alt-v");
    assert!(settings(vcx).vim_mode);
    click("vim-mode-chip", vcx);
    let labels = menu_labels(vcx).expect("the chip opens its menu");
    assert!(labels.contains(&"Settings…".to_string()), "{labels:?}");
    click_menu_item("Turn off Vim mode", vcx);
    assert!(!settings(vcx).vim_mode);
}

#[gpui::test]
fn modals_close_and_work_on_a_click(cx: &mut TestAppContext) {
    init(cx);
    let clock = Clock::manual(1_800_000_000_000);
    let library = cx.new(|_| Library::in_memory_with_clock(clock.clone()).unwrap());
    library.update(cx, |library, cx| {
        library
            .set_setting(writer_app::settings::keys::WELCOME_CREATED, &true, cx)
            .unwrap();
    });
    let id = add_document(&library, None, &simple_document("Chapter", "Draft."), cx);
    let (workspace, vcx) = open_workspace(&library, cx);

    // Settings: the close control.
    vcx.simulate_keystrokes("cmd-,");
    assert!(has_modal(&workspace, vcx));
    click(writer_app::controls::CLOSE_CONTROL, vcx);
    assert!(!has_modal(&workspace, vcx));
    assert!(editor_focused(&workspace, vcx));

    // A click on a row switches it (row 1 is Vim mode).
    vcx.simulate_keystrokes("cmd-,");
    assert!(!workspace.read_with(vcx, |w, _| w.settings().vim_mode));
    click("setting-1", vcx);
    assert!(workspace.read_with(vcx, |w, _| w.settings().vim_mode));
    click("setting-1", vcx);
    assert!(!workspace.read_with(vcx, |w, _| w.settings().vim_mode));
    assert!(has_modal(&workspace, vcx), "a click inside does not close");
    // A click on the backdrop closes.
    vcx.simulate_click(point(px(40.), px(500.)), Modifiers::default());
    vcx.run_until_parked();
    assert!(!has_modal(&workspace, vcx));

    // The pickers have the same control, at the right of their query row.
    for open in ["cmd-shift-p", "cmd-p", "cmd-shift-f", "cmd-shift-s"] {
        vcx.simulate_keystrokes(open);
        assert!(has_modal(&workspace, vcx), "{open} opens a picker");
        let control = bounds_of(writer_app::controls::CLOSE_CONTROL, vcx);
        let window_size = vcx.update(|window, _| window.viewport_size());
        assert!(control.top() >= window_size.height * 0.12);
        assert!(control.bottom() <= window_size.height * 0.12 + px(48.));
        assert!(control.left() > window_size.width / 2.);
        vcx.simulate_input("a");
        click(writer_app::controls::CLOSE_CONTROL, vcx);
        assert!(!has_modal(&workspace, vcx), "the control closes {open}");
        assert!(editor_focused(&workspace, vcx));
    }

    // The list of keyboard shortcuts has a close control of its own, and
    // closes on a click outside like the others.
    vcx.simulate_keystrokes("cmd-/");
    assert!(has_modal(&workspace, vcx));
    vcx.simulate_input("a");
    click("keymap-close", vcx);
    assert!(!has_modal(&workspace, vcx), "the control closes the list");
    assert!(editor_focused(&workspace, vcx));
    vcx.simulate_keystrokes("cmd-/");
    assert!(has_modal(&workspace, vcx));
    vcx.simulate_click(point(px(40.), px(500.)), Modifiers::default());
    vcx.run_until_parked();
    assert!(
        !has_modal(&workspace, vcx),
        "a click outside closes the list"
    );
    assert!(editor_focused(&workspace, vcx));

    // Version history: rows are clicked, a button restores.
    let save = |text: &str, cx: &mut VisualTestContext| {
        type_text(&workspace, text, cx);
        cx.executor().advance_clock(AUTOSAVE_DELAY * 2);
        cx.run_until_parked();
    };
    save("One. ", vcx);
    let one = editor_text(&workspace, vcx);
    clock.advance_ms(20 * 60_000);
    save("Two. ", vcx);
    clock.advance_ms(20 * 60_000);
    save("Three. ", vcx);
    workspace.update_in(vcx, |w, window, cx| w.toggle_version_history(window, cx));
    vcx.run_until_parked();
    let history = workspace
        .read_with(vcx, |w, cx| {
            w.modal_layer().read(cx).active::<VersionHistory>()
        })
        .expect("the version history is open");
    let count = history.read_with(vcx, |history, _| history.row_labels().len());
    assert!(count >= 2, "{count} versions");
    // The oldest version is the text after "One.".
    let oldest: &'static str = Box::leak(format!("version-{}", count - 1).into_boxed_str());
    click(oldest, vcx);
    assert!(
        history
            .read_with(vcx, |history, _| history.preview_text())
            .contains("One."),
    );
    assert!(has_modal(&workspace, vcx));
    // The button does what enter does.
    click("version-restore", vcx);
    assert!(!has_modal(&workspace, vcx));
    assert_eq!(editor_text(&workspace, vcx), one);

    // A double click on a row restores it too.
    workspace.update_in(vcx, |w, window, cx| w.toggle_version_history(window, cx));
    vcx.run_until_parked();
    let history = workspace
        .read_with(vcx, |w, cx| {
            w.modal_layer().read(cx).active::<VersionHistory>()
        })
        .expect("the version history is open");
    let newest_text = history.read_with(vcx, |history, cx| {
        let _ = cx;
        history.preview_text()
    });
    let row = centre(bounds_of("version-0", vcx));
    vcx.simulate_event(gpui::MouseDownEvent {
        button: MouseButton::Left,
        position: row,
        modifiers: Modifiers::default(),
        click_count: 2,
        first_mouse: false,
    });
    vcx.simulate_event(gpui::MouseUpEvent {
        button: MouseButton::Left,
        position: row,
        modifiers: Modifiers::default(),
        click_count: 2,
    });
    vcx.run_until_parked();
    assert!(!has_modal(&workspace, vcx));
    assert_eq!(
        editor_text(&workspace, vcx).replace('\n', " ").trim(),
        newest_text.replace('\n', " ").trim()
    );

    // And the close control of the version history.
    workspace.update_in(vcx, |w, window, cx| w.toggle_version_history(window, cx));
    vcx.run_until_parked();
    assert!(has_modal(&workspace, vcx));
    click(writer_app::controls::CLOSE_CONTROL, vcx);
    assert!(!has_modal(&workspace, vcx));
    let _ = id;
}

#[gpui::test]
fn the_text_menu_offers_comments(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    add_document(
        &library,
        None,
        &assembly::commented_document("Noted", "Alpha beta gamma delta.", "A first note"),
        cx,
    );
    let (workspace, vcx) = open_workspace(&library, cx);
    let text_menu = |cx: &mut VisualTestContext| {
        workspace.read_with(cx, |w, cx| {
            w.tools()
                .read(cx)
                .text_menu()
                .map(|menu| menu.read(cx).labels())
        })
    };
    let point_of = |position: Position, cx: &mut VisualTestContext| {
        workspace.read_with(cx, |w, cx| {
            w.editor()
                .read(cx)
                .position_bounds(position)
                .expect("the position is on screen")
                .center()
        })
    };
    // The comment is on the first word of the paragraph ("Alpha"); the
    // last word ("delta") has none.
    let commented = Position::new(1, 2);
    let plain = workspace.read_with(vcx, |w, cx| {
        let end = w.editor().read(cx).document().blocks()[1].text().len();
        Position::new(1, end - 3)
    });

    // A right-click on a word without a comment: the word is selected and
    // the menu offers to comment it.
    let at = point_of(plain, vcx);
    right_click_at(at, vcx);
    let labels = text_menu(vcx).expect("the text menu is open");
    assert!(labels.contains(&"Add Comment".to_string()), "{labels:?}");
    assert!(!labels.contains(&"Go to Comment".to_string()), "{labels:?}");
    choose("Add Comment", vcx);
    assert!(workspace.read_with(vcx, |w, cx| w.comments().read(cx).is_composing()));
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();

    // On commented text: "Go to Comment" puts the keyboard on its thread.
    vcx.simulate_keystrokes("cmd-1");
    let at = point_of(commented, vcx);
    right_click_at(at, vcx);
    let labels = text_menu(vcx).expect("the text menu is open");
    assert!(labels.contains(&"Go to Comment".to_string()), "{labels:?}");
    choose("Go to Comment", vcx);
    assert!(workspace.update_in(vcx, |w, window, cx| {
        w.comments().read(cx).has_focus(window, cx)
    }));
    assert!(workspace.read_with(vcx, |w, cx| w.comments().read(cx).active_thread().is_some()));

    // The same menu from the keyboard, with the same items.
    vcx.simulate_keystrokes("cmd-1");
    workspace.update(vcx, |w, cx| {
        w.editor().update(cx, |editor, cx| {
            editor.edit(
                |state| state.set_selection(Selection::new(plain, plain)),
                cx,
            );
        });
    });
    vcx.simulate_keystrokes("ctrl-enter");
    vcx.run_until_parked();
    let labels = text_menu(vcx).expect("ctrl-enter opens the text menu");
    assert!(labels.contains(&"Add Comment".to_string()), "{labels:?}");
    vcx.simulate_keystrokes("escape");
    assert_eq!(text_menu(vcx), None);

    // With nothing to anchor a comment to (the caret in an empty
    // paragraph) the item is there, dimmed.
    workspace.update(vcx, |w, cx| {
        w.editor().update(cx, |editor, cx| {
            editor.edit(
                |state| {
                    let end = state.document().end();
                    state.set_caret(end);
                    state.insert_text("\n");
                },
                cx,
            );
        });
    });
    vcx.simulate_keystrokes("ctrl-enter");
    vcx.run_until_parked();
    let add_comment = workspace.read_with(vcx, |w, cx| {
        let menu = w
            .tools()
            .read(cx)
            .text_menu()
            .expect("the text menu")
            .read(cx);
        menu.items()
            .iter()
            .find(|item| item.title().as_ref() == "Add Comment")
            .map(|item| item.is_disabled())
    });
    assert_eq!(add_comment, Some(true));
    vcx.simulate_keystrokes("escape");
}

#[gpui::test]
fn library_rows_offer_export_and_version_history(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    let folder = library.update(cx, |library, cx| {
        library.create_folder(None, "Part one", cx).unwrap().id
    });
    let first = add_document(
        &library,
        Some(folder),
        &simple_document("Chapter one", "The first chapter."),
        cx,
    );
    add_document(
        &library,
        Some(folder),
        &simple_document("Chapter two", "The second chapter."),
        cx,
    );
    let loose = add_document(&library, None, &simple_document("Loose", "On its own."), cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    workspace.update_in(vcx, |w, window, cx| {
        w.open_document(loose, true, window, cx)
    });
    vcx.run_until_parked();

    // The menu of a document row (opened from the keyboard here; the
    // panel's own tests click it open).
    let select = |item: LibraryItem, cx: &mut VisualTestContext| {
        workspace.update(cx, |w, cx| {
            w.panel().update(cx, |panel, cx| {
                panel.reveal(item, cx);
            });
        });
        cx.simulate_keystrokes("cmd-0");
        cx.simulate_keystrokes("ctrl-enter");
        cx.run_until_parked();
    };
    select(LibraryItem::Document(first), vcx);
    let labels = menu_labels(vcx).expect("the row menu is open");
    assert!(labels.contains(&"Export…".to_string()), "{labels:?}");
    assert!(
        labels.contains(&"Version History…".to_string()),
        "{labels:?}"
    );

    // Export: the picker is about that document, and the open document
    // stays the open one.
    choose("Export…", vcx);
    let picker = workspace
        .read_with(vcx, |w, cx| {
            w.modal_layer().read(cx).active::<ExportPicker>()
        })
        .expect("the export picker is open");
    assert_eq!(
        picker.read_with(vcx, |picker, _| picker.delegate.target()),
        ExportTarget::Document(first)
    );
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.open_document_id()),
        Some(loose)
    );
    let exported = workspace
        .update(vcx, |w, cx| {
            w.prepare_export_of(
                ExportTarget::Document(first),
                ExportScope::Document,
                ExportFormat::Markdown,
                cx,
            )
        })
        .unwrap();
    assert_eq!(
        String::from_utf8(exported.bytes).unwrap(),
        "# Chapter one\n\nThe first chapter.\n"
    );
    vcx.simulate_keystrokes("escape");

    // A folder row: the whole folder as one file.
    select(LibraryItem::Folder(folder), vcx);
    let labels = menu_labels(vcx).expect("the folder menu is open");
    assert!(labels.contains(&"Export Folder…".to_string()), "{labels:?}");
    choose("Export Folder…", vcx);
    let picker = workspace
        .read_with(vcx, |w, cx| {
            w.modal_layer().read(cx).active::<ExportPicker>()
        })
        .expect("the export picker is open");
    let (target, choices) = picker.read_with(vcx, |picker, _| {
        (picker.delegate.target(), picker.delegate.choices())
    });
    assert_eq!(target, ExportTarget::Folder(folder));
    assert!(
        choices
            .iter()
            .all(|choice| choice.scope == ExportScope::Folder),
        "{choices:?}"
    );
    let exported = workspace
        .update(vcx, |w, cx| {
            w.prepare_export_of(
                ExportTarget::Folder(folder),
                ExportScope::Folder,
                ExportFormat::PlainText,
                cx,
            )
        })
        .unwrap();
    assert_eq!(exported.documents, 2);
    let text = String::from_utf8(exported.bytes).unwrap();
    assert!(text.contains("The first chapter.") && text.contains("The second chapter."));
    vcx.simulate_keystrokes("escape");

    // Version history of a row: that document is opened, with its history.
    select(LibraryItem::Document(first), vcx);
    choose("Version History…", vcx);
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.open_document_id()),
        Some(first)
    );
    assert!(workspace.read_with(vcx, |w, cx| {
        w.modal_layer().read(cx).is_open::<VersionHistory>()
    }));
}

#[gpui::test]
fn the_welcome_document_and_the_overlay_speak_of_the_mouse(cx: &mut TestAppContext) {
    init(cx);
    // The welcome document has a part for the mouse.
    let text = welcome::welcome_document().plain_text();
    assert!(text.contains("With the mouse"));
    assert!(text.contains("Right-click"));
    assert!(text.contains("menu bar"));

    // "Welcome document" opens it, and writes it again when it is gone.
    let library = cx.new(|_| Library::in_memory().unwrap());
    let (workspace, vcx) = open_workspace(&library, cx);
    let welcome_id = workspace
        .read_with(vcx, |w, _| w.open_document_id())
        .expect("the first run opens the welcome document");
    workspace.update_in(vcx, |w, window, cx| w.new_document(window, cx));
    vcx.run_until_parked();
    vcx.dispatch_action(OpenWelcomeDocument);
    vcx.run_until_parked();
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.open_document_id()),
        Some(welcome_id)
    );
    library.update(vcx, |library, cx| {
        library.trash_document(welcome_id, cx).unwrap();
    });
    vcx.run_until_parked();
    vcx.dispatch_action(OpenWelcomeDocument);
    vcx.run_until_parked();
    let again = workspace
        .read_with(vcx, |w, _| w.open_document_id())
        .expect("a welcome document is open");
    assert_ne!(again, welcome_id);
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.document_title().map(str::to_string)),
        Some(welcome::WELCOME_TITLE.to_string())
    );

    // The shortcuts overlay: its first line names the menus, and it lists
    // the insert menu although no key binding opens it.
    vcx.simulate_keystrokes("cmd-/");
    let list = workspace
        .read_with(vcx, |w, cx| w.shortcuts_list(cx))
        .expect("the list is open");
    let first = list
        .read_with(vcx, |list, _| list.headings().into_iter().next())
        .unwrap_or_default();
    let insert = vcx.update(|_, cx| {
        collect_shortcuts(cx)
            .into_iter()
            .find(|shortcut| shortcut.title.starts_with("Insert menu ("))
            .map(|shortcut| {
                (
                    shortcut.title.to_string(),
                    shortcut.keys.clone(),
                    shortcut.fixed.is_some(),
                )
            })
    });
    assert_eq!(first, MENUS_NOTE);
    let (title, keys, fixed) = insert.expect("the insert menu is listed");
    assert!(title.contains("start of a line"), "{title}");
    assert_eq!(keys, "/");
    assert!(fixed, "a character that is typed is not a key to change");
}

#[gpui::test]
fn tooltips_say_what_a_control_is_and_keep_out_of_the_way_of_its_menu(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    add_document(
        &library,
        None,
        &simple_document("Tip", "Some text here."),
        cx,
    );
    let (workspace, vcx) = open_workspace(&library, cx);
    let rest = std::time::Duration::from_millis(800);

    // Resting the pointer on an item of the status bar says what it is.
    let item = bounds_of(MenuKind::Save.selector(), vcx);
    vcx.simulate_mouse_move(item.center(), None, Modifiers::default());
    vcx.executor().advance_clock(rest);
    vcx.run_until_parked();
    let before = vcx.debug_bounds("tooltip").expect("a tooltip is shown");

    // A click opens the menu above the item. The pointer is still on the
    // item; no tooltip comes back over the menu.
    vcx.simulate_click(item.center(), Modifiers::default());
    vcx.run_until_parked();
    let menu = vcx.update(|window, cx| {
        ContextMenu::current(window, cx)
            .expect("the save menu is open")
            .read(cx)
            .bounds()
            .expect("the menu is drawn")
    });
    assert!(menu.bottom() <= item.top(), "{menu:?} above {item:?}");
    assert!(
        before.intersects(&menu),
        "the tooltip would have covered the menu: {before:?} over {menu:?}"
    );
    // (A tooltip is drawn where the pointer is when it comes up: one that
    // came up now would be somewhere else than the one before.)
    vcx.simulate_mouse_move(
        item.center() + point(px(12.), px(0.)),
        None,
        Modifiers::default(),
    );
    vcx.executor().advance_clock(rest);
    vcx.run_until_parked();
    assert_eq!(
        vcx.debug_bounds("tooltip"),
        Some(before),
        "a tooltip came up while the menu is open"
    );

    // With the menu closed the tooltip is back.
    vcx.simulate_keystrokes("escape");
    vcx.simulate_mouse_move(
        item.center() + point(px(-12.), px(0.)),
        None,
        Modifiers::default(),
    );
    vcx.executor().advance_clock(rest);
    vcx.run_until_parked();
    assert_ne!(vcx.debug_bounds("tooltip"), Some(before));
    let _ = workspace;
}

// ----- an image's description --------------------------------------------------------------------

/// What an image shows is written with the pointer alone (and the words):
/// a right-click on the picture, "Description…", and the button "Done".
/// A click somewhere else instead of the button keeps what was typed too;
/// "Remove" and "Cancel" are buttons.
#[gpui::test]
fn the_description_of_an_image_with_the_pointer_alone(cx: &mut TestAppContext) {
    use super::images::{images_in, picture, stored_png};
    use writer_core::Alignment;
    use writer_editor_tools::ToolKind;

    init(cx);
    let library = library(cx);
    let image = stored_png(&library, 400, 240, [60, 160, 90], cx);
    let document = Document::from_blocks(vec![
        picture(&image, 60, Alignment::Center, ""),
        Block::paragraph("The boats came by road every spring."),
    ]);
    let id = add_document(&library, None, &document, cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    workspace.update_in(vcx, |w, window, cx| {
        w.open_document(id, true, window, cx);
    });
    vcx.run_until_parked();
    let on_the_picture = |cx: &mut VisualTestContext| {
        workspace
            .read_with(cx, |w, cx| w.editor().read(cx).image_bounds(0))
            .expect("the picture is laid out")
            .center()
    };
    let description =
        |cx: &mut VisualTestContext| images_in(&workspace, cx)[0].1.description.clone();
    let tool = |cx: &mut VisualTestContext| {
        workspace.read_with(cx, |w, cx| w.tools().read(cx).active_tool())
    };
    let open = |cx: &mut VisualTestContext| {
        right_click_at(on_the_picture(cx), cx);
        click_menu_item("Description…", cx);
        assert_eq!(tool(cx), Some(ToolKind::ImageDescription));
        // The field has the keyboard: what is typed goes into it.
        assert!(!editor_focused(&workspace, cx));
    };

    // Done.
    open(vcx);
    vcx.simulate_input("The harbour at dusk");
    assert_eq!(description(vcx), "", "not before it is kept");
    click("image-description-done", vcx);
    assert_eq!(tool(vcx), None);
    assert_eq!(description(vcx), "The harbour at dusk");
    assert!(editor_focused(&workspace, vcx));

    // A click in the text instead of the button: what was typed is kept.
    open(vcx);
    let field = workspace.read_with(vcx, |w, cx| {
        w.tools().read(cx).image_description_field_text(cx)
    });
    assert_eq!(field.as_deref(), Some("The harbour at dusk"));
    // The field opens with its text selected: what is typed takes its
    // place.
    vcx.simulate_input("The harbour, from the pier");
    let typed = workspace
        .read_with(vcx, |w, cx| {
            w.tools().read(cx).image_description_field_text(cx)
        })
        .expect("the field is open");
    assert!(typed.ends_with("The harbour, from the pier"), "{typed:?}");
    assert_ne!(typed, "The harbour at dusk");
    assert_eq!(
        description(vcx),
        "The harbour at dusk",
        "not before it is kept"
    );
    let in_the_text = workspace.read_with(vcx, |w, cx| {
        let bounds = w
            .editor()
            .read(cx)
            .position_bounds(Position::new(1, 10))
            .expect("the paragraph is on screen");
        point(bounds.left() + px(2.), bounds.center().y)
    });
    vcx.simulate_click(in_the_text, Modifiers::default());
    vcx.run_until_parked();
    assert_eq!(tool(vcx), None);
    assert_eq!(
        description(vcx),
        typed,
        "a click outside keeps the description"
    );
    assert!(editor_focused(&workspace, vcx));

    // Cancel: nothing changes.
    open(vcx);
    vcx.simulate_input(" that nobody wants");
    click("image-description-cancel", vcx);
    assert_eq!(tool(vcx), None);
    assert_eq!(description(vcx), typed);

    // Remove.
    open(vcx);
    click("image-description-remove", vcx);
    assert_eq!(tool(vcx), None);
    assert_eq!(description(vcx), "");
    // One step of undo each.
    vcx.simulate_keystrokes("cmd-z");
    assert_eq!(description(vcx), typed);
}
