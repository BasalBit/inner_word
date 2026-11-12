//! The zoom of the text in the assembled app: the keys, the palette, the
//! menu bar, the item of the status bar and its menu, the wheel with
//! `cmd`, what is kept in the library, what a page starts with, and
//! everything the other crates anchor to the text (the tools, the menus,
//! the format bar, the quick chat, the comments, Vim's caret) at 100 %,
//! at a zoom below and at a zoom above, in both views.
//!
//! The geometry itself (lines, pages, positions, hit testing, sideways
//! scrolling) is held by the editor's own tests
//! (`crates/writer-editor/src/tests/zoom.rs`); here it is the assembly.

use std::{rc::Rc, sync::Arc, time::Duration};

use gpui::{
    Bounds, Modifiers, MouseButton, Pixels, ScrollDelta, ScrollWheelEvent, TestAppContext,
    TouchPhase, point, px, size,
};
use writer_app::{
    SaveState, assistant, commands, menus, spelling,
    workspace::{MenuKind, ZOOM_STORE_DELAY},
    zoom::{self, FLOW_ZOOM_KEY, PAGE_ZOOM_KEY},
};
use writer_assistant::FakeBackend;
use writer_core::{DocumentSettings, Position, Selection, TextRange};
use writer_editor::{CaretStyle, Zoom};
use writer_editor_tools::FORMAT_BAR_DELAY;
use writer_spell::FakeSpellService;
use writer_ui::ContextMenu;

use super::*;

/// The zooms the anchored things are looked at: 100 %, one below, one
/// above.
const ZOOMS: [u16; 3] = [100, 75, 150];

/// A document long enough for several pages: a heading, a paragraph with
/// a misspelled word, a long paragraph, and many more.
fn manuscript(page_view: bool) -> Document {
    let long = "The harbor was a plain grey fact and the boats came by road every spring. ";
    let mut blocks = vec![
        Block::with_text(BlockKind::heading(1), "The harbor"),
        Block::paragraph("She bought bread and salt and teh ropes."),
        Block::paragraph(long.repeat(4).trim_end()),
        Block::paragraph(""),
    ];
    for index in 0..90 {
        blocks.push(Block::paragraph(&format!(
            "Paragraph {index}. {}",
            long.trim_end()
        )));
    }
    Document::from_blocks(blocks).with_settings(DocumentSettings {
        page_view,
        ..DocumentSettings::default()
    })
}

/// A workspace over a fresh library with `document` open, in a window
/// with room for the text at 150 %.
fn open_document<'a>(
    document: &Document,
    cx: &'a mut TestAppContext,
) -> (
    Entity<Library>,
    Entity<Workspace>,
    &'a mut VisualTestContext,
) {
    let library = library(cx);
    let id = add_document(&library, None, document, cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    vcx.simulate_resize(size(px(1900.), px(1100.)));
    workspace.update_in(vcx, |w, window, cx| {
        w.open_document(id, true, window, cx);
    });
    vcx.run_until_parked();
    (library, workspace, vcx)
}

fn zoom_of(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Zoom {
    workspace.read_with(cx, |w, cx| w.zoom(cx))
}

fn percent_of(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> f32 {
    workspace.read_with(cx, |w, cx| w.zoom_percent(cx))
}

fn set_zoom(workspace: &Entity<Workspace>, zoom: Zoom, cx: &mut VisualTestContext) {
    workspace.update(cx, |w, cx| w.set_zoom(zoom, cx));
    cx.run_until_parked();
}

fn stored(library: &Entity<Library>, key: &str, cx: &mut VisualTestContext) -> Option<String> {
    library.read_with(cx, |library, _| library.setting::<String>(key).unwrap())
}

/// Lets the moment pass after which a change of zoom is written.
fn let_it_be_stored(cx: &mut VisualTestContext) {
    cx.executor()
        .advance_clock(ZOOM_STORE_DELAY + Duration::from_millis(50));
    cx.run_until_parked();
}

fn run_command(workspace: &Entity<Workspace>, title: &str, cx: &mut VisualTestContext) {
    let command = cx
        .update(|_, cx| {
            commands::all(cx)
                .into_iter()
                .find(|command| command.title.as_ref() == title)
        })
        .unwrap_or_else(|| panic!("the palette has no command {title:?}"));
    workspace.update_in(cx, |w, window, cx| w.run_command(&command, window, cx));
    cx.run_until_parked();
}

fn bounds_of(selector: &'static str, cx: &mut VisualTestContext) -> Bounds<Pixels> {
    cx.debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn"))
}

fn menu_labels(cx: &mut VisualTestContext) -> Option<Vec<String>> {
    cx.update(|window, cx| ContextMenu::current(window, cx).map(|menu| menu.read(cx).labels()))
}

/// Clicks the row of the item called `label` of the open context menu.
fn click_item(label: &str, cx: &mut VisualTestContext) {
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
    cx.simulate_mouse_move(at, None, Modifiers::default());
    cx.simulate_click(at, Modifiers::default());
    cx.run_until_parked();
}

/// The labels of the checked items of the open context menu.
fn checked_items(cx: &mut VisualTestContext) -> Vec<String> {
    cx.update(|window, cx| {
        let menu = ContextMenu::current(window, cx).expect("a menu is open");
        menu.read(cx)
            .items()
            .iter()
            .filter(|item| item.is_checked() == Some(true))
            .map(|item| item.title().to_string())
            .collect()
    })
}

fn caret(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Bounds<Pixels> {
    workspace.read_with(cx, |w, cx| {
        w.editor()
            .read(cx)
            .caret_bounds()
            .expect("the caret was laid out")
    })
}

fn position_bounds(
    workspace: &Entity<Workspace>,
    position: Position,
    cx: &mut VisualTestContext,
) -> Bounds<Pixels> {
    workspace.read_with(cx, |w, cx| {
        w.editor()
            .read(cx)
            .position_bounds(position)
            .expect("the position was laid out")
    })
}

fn select(workspace: &Entity<Workspace>, range: TextRange, cx: &mut VisualTestContext) {
    let editor = workspace.read_with(cx, |w, _| w.editor().clone());
    editor.update(cx, |editor, cx| {
        editor.edit(
            |state| state.set_selection(Selection::from_range(range)),
            cx,
        );
    });
    cx.run_until_parked();
}

fn selection_bounds(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Bounds<Pixels> {
    workspace.read_with(cx, |w, cx| {
        w.editor()
            .read(cx)
            .selection_bounds()
            .expect("the selection was laid out")
    })
}

/// Whether `popup` stands at `anchor`: right under it or right over it,
/// and over the same stretch of the line (or pushed in from an edge of
/// the window).
#[track_caller]
fn assert_at(popup: Bounds<Pixels>, anchor: Bounds<Pixels>, what: &str) {
    let under = popup.top() - anchor.bottom();
    let over = anchor.top() - popup.bottom();
    let gap = px(16.);
    assert!(
        (under >= px(-1.) && under <= gap) || (over >= px(-1.) && over <= gap),
        "{what}: {popup:?} is not at {anchor:?}"
    );
    let reach = px(48.);
    assert!(
        popup.left() <= anchor.right() + reach && popup.right() >= anchor.left() - reach,
        "{what}: {popup:?} is not beside {anchor:?}"
    );
}

fn escape(cx: &mut VisualTestContext) {
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
}

// ----- the commands ---------------------------------------------------------------------

#[gpui::test]
fn the_keys_and_the_palette_zoom_the_text_and_nothing_else(cx: &mut TestAppContext) {
    init(cx);
    let (library, workspace, cx) = open_document(&manuscript(false), cx);
    let text = editor_text(&workspace, cx);
    assert_eq!(zoom_of(&workspace, cx), Zoom::Percent(100));
    assert_eq!(workspace.read_with(cx, |w, cx| w.zoom_label(cx)), "100 %");
    let body = Position::new(1, 0);
    let at_100 = position_bounds(&workspace, body, cx);

    // In, as the keyboard has it: the key without shift, the key with
    // shift as a real keyboard reports it, and as a script spells it.
    for (keys, percent) in [("cmd-=", 110), ("cmd-+", 125), ("cmd-shift-=", 150)] {
        cx.simulate_keystrokes(keys);
        cx.run_until_parked();
        assert_eq!(zoom_of(&workspace, cx), Zoom::Percent(percent), "{keys}");
    }
    assert_eq!(workspace.read_with(cx, |w, cx| w.zoom_label(cx)), "150 %");
    // The text is drawn half as large again.
    let at_150 = position_bounds(&workspace, body, cx);
    assert!((at_150.size.height - at_100.size.height * 1.5).abs() < px(0.05));
    // Out, and back to 100 % with its key in both spellings.
    cx.simulate_keystrokes("cmd--");
    assert_eq!(zoom_of(&workspace, cx), Zoom::Percent(125));
    cx.simulate_keystrokes("cmd-shift-0");
    assert_eq!(zoom_of(&workspace, cx), Zoom::Percent(100));
    cx.simulate_keystrokes("cmd-- cmd--");
    assert_eq!(zoom_of(&workspace, cx), Zoom::Percent(80));
    cx.simulate_keystrokes("cmd-)");
    assert_eq!(zoom_of(&workspace, cx), Zoom::Percent(100));
    // `cmd-0` is still "Go to library", and the zoom keys work from there.
    cx.simulate_keystrokes("cmd-0");
    cx.run_until_parked();
    assert!(!editor_focused(&workspace, cx));
    assert_eq!(zoom_of(&workspace, cx), Zoom::Percent(100));
    cx.simulate_keystrokes("cmd-=");
    assert_eq!(zoom_of(&workspace, cx), Zoom::Percent(110));
    cx.simulate_keystrokes("cmd-1");
    cx.run_until_parked();

    // The palette: the same commands, fit width, and every step by name.
    for (title, expected) in [
        ("Zoom in", Zoom::Percent(125)),
        ("Zoom out", Zoom::Percent(110)),
        ("Zoom to 250 %", Zoom::Percent(250)),
        ("Zoom to 50 %", Zoom::Percent(50)),
        ("Zoom to fit width", Zoom::FitWidth),
        ("Actual size", Zoom::Percent(100)),
        ("Zoom to 175 %", Zoom::Percent(175)),
    ] {
        run_command(&workspace, title, cx);
        assert_eq!(zoom_of(&workspace, cx), expected, "{title}");
        assert!(
            editor_focused(&workspace, cx),
            "{title} leaves the keyboard in the text"
        );
    }
    // Fit page is for pages: in one column it says so and changes nothing.
    run_command(&workspace, "Zoom to fit page", cx);
    assert_eq!(zoom_of(&workspace, cx), Zoom::Percent(175));
    let toasts: Vec<String> = workspace.read_with(cx, |w, cx| {
        w.toast_layer()
            .read(cx)
            .toasts()
            .map(|toast| toast.message.to_string())
            .collect()
    });
    assert!(
        toasts.iter().any(|toast| toast.contains("Fit Page")),
        "{toasts:?}"
    );

    // None of it was an edit: the text, the save state, undo.
    assert_eq!(editor_text(&workspace, cx), text);
    workspace.read_with(cx, |w, cx| {
        assert_eq!(w.save_state(), Some(SaveState::Saved));
        assert_eq!(w.editor().read(cx).state().undo_depth(), 0);
        assert_eq!(
            w.editor()
                .read(cx)
                .document()
                .settings()
                .typography
                .base_size,
            17.
        );
    });
    // And the zoom is kept in the library, a moment after the last change.
    assert_eq!(stored(&library, FLOW_ZOOM_KEY, cx), None);
    let_it_be_stored(cx);
    assert_eq!(stored(&library, FLOW_ZOOM_KEY, cx), Some("175".to_string()));
    assert_eq!(stored(&library, PAGE_ZOOM_KEY, cx), None);
    workspace.read_with(cx, |w, _| {
        assert_eq!(w.save_state(), Some(SaveState::Saved))
    });
}

#[gpui::test]
fn the_commands_are_in_the_palette_the_list_of_shortcuts_and_the_menu_bar(cx: &mut TestAppContext) {
    init(cx);
    let titles: Vec<String> = cx.update(|cx| {
        commands::all(cx)
            .into_iter()
            .filter(|command| command.action_name().starts_with("zoom::"))
            .map(|command| format!("{}: {}", command.category, command.title))
            .collect()
    });
    assert_eq!(
        titles,
        [
            "View: Zoom in",
            "View: Zoom out",
            "View: Actual size",
            "View: Zoom to fit width",
            "View: Zoom to fit page",
            "View: Zoom to life size",
            "View: Zoom to 50 %",
            "View: Zoom to 75 %",
            "View: Zoom to 125 %",
            "View: Zoom to 150 %",
            "View: Zoom to 175 %",
            "View: Zoom to 200 %",
            "View: Zoom to 250 %",
            "View: Zoom to 300 %",
            "View: Zoom to 400 %",
        ]
    );

    // The View menu: in, out, actual size and fit width, then the rest
    // under "Zoom To".
    fn names_of(items: &[gpui::MenuItem], out: &mut Vec<String>) {
        for item in items {
            match item {
                gpui::MenuItem::Action { name, .. } => out.push(name.to_string()),
                gpui::MenuItem::Submenu(menu) => {
                    out.push(format!("{} >", menu.name));
                    names_of(&menu.items, out);
                }
                gpui::MenuItem::Separator => out.push("-".to_string()),
                _ => {}
            }
        }
    }
    let all = menus::app_menus();
    let view = all.iter().find(|menu| menu.name == "View").unwrap();
    let mut names = Vec::new();
    names_of(&view.items, &mut names);
    let from = names.iter().position(|name| name == "Zoom In").unwrap();
    assert_eq!(
        names[from - 2..from + 19],
        [
            "Turn Vim Mode On or Off",
            "-",
            "Zoom In",
            "Zoom Out",
            "Actual Size",
            "Fit Width",
            "Zoom To >",
            "Fit Page",
            "Life Size",
            "-",
            "50 %",
            "75 %",
            "100 %",
            "125 %",
            "150 %",
            "175 %",
            "200 %",
            "250 %",
            "300 %",
            "400 %",
            "-",
        ]
    );
    // The keys the menu bar shows are keys that run the command.
    cx.update(|cx| {
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        let key = |action: &dyn gpui::Action| {
            menus::native_menu_binding(&keymap, action).map(|binding| {
                binding
                    .keystrokes()
                    .iter()
                    .map(|keystroke| keystroke.unparse())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
        };
        assert_eq!(key(&zoom::ZoomIn).as_deref(), Some("cmd-+"));
        assert_eq!(key(&zoom::ZoomOut).as_deref(), Some("cmd--"));
        assert_eq!(key(&zoom::ActualSize).as_deref(), Some("cmd-shift-0"));
        assert_eq!(key(&zoom::FitWidth), None);
        assert_eq!(key(&zoom::ZoomTo150), None);
    });
}

#[gpui::test]
fn without_a_document_the_zoom_is_not_there(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    let (workspace, cx) = open_workspace(&library, cx);
    workspace.read_with(cx, |w, cx| {
        assert_eq!(w.open_document_id(), None);
        assert_eq!(w.zoom_label(cx), "");
    });
    assert!(cx.debug_bounds(MenuKind::Zoom.selector()).is_none());
    // The keys do nothing and the menu bar has the commands greyed out.
    cx.simulate_keystrokes("cmd-= cmd-- cmd-shift-0");
    cx.run_until_parked();
    cx.update(|window, cx| {
        assert!(!window.is_action_available(&zoom::ZoomIn, cx));
        assert!(!window.is_action_available(&zoom::FitWidth, cx));
        assert!(!window.is_action_available(&zoom::ZoomTo150, cx));
    });
    assert!(!workspace.update_in(cx, |w, window, cx| w.open_menu(
        MenuKind::Zoom,
        None,
        window,
        cx
    )));
}

// ----- the pointer ----------------------------------------------------------------------

#[gpui::test]
fn the_status_bar_shows_the_zoom_and_its_menu_sets_it(cx: &mut TestAppContext) {
    init(cx);
    cx.update(|cx| zoom::set_display_density(Some(127.5), cx));
    let (_library, workspace, cx) = open_document(&manuscript(false), cx);
    let item = bounds_of(MenuKind::Zoom.selector(), cx);
    let bar = bounds_of("status-bar", cx);
    assert!(item.top() >= bar.top() && item.bottom() <= bar.bottom());
    assert!(item.right() <= bar.right());

    // A click opens the menu above the item, with everything in it, and
    // the keyboard stays in the text.
    cx.simulate_click(item.center(), Modifiers::default());
    cx.run_until_parked();
    assert!(editor_focused(&workspace, cx));
    let labels = menu_labels(cx).expect("a click on the zoom opens its menu");
    assert_eq!(
        labels,
        [
            "Zoom In",
            "Zoom Out",
            "-",
            "Actual Size",
            "Fit Width",
            "Life Size (177 %)",
            "-",
            "50 %",
            "75 %",
            "125 %",
            "150 %",
            "175 %",
            "200 %",
            "250 %",
            "300 %",
            "400 %",
        ]
    );
    assert_eq!(checked_items(cx), ["Actual Size"]);
    let menu = bounds_of("context-menu", cx);
    assert!(
        menu.bottom() <= item.top() + px(1.),
        "{menu:?} over {item:?}"
    );
    // A click on a step sets it; the menu shows which one it is.
    click_item("150 %", cx);
    assert_eq!(menu_labels(cx), None);
    assert_eq!(zoom_of(&workspace, cx), Zoom::Percent(150));
    assert_eq!(workspace.read_with(cx, |w, cx| w.zoom_label(cx)), "150 %");

    // A right-click opens the same menu at the pointer.
    let item = bounds_of(MenuKind::Zoom.selector(), cx);
    cx.simulate_mouse_move(item.center(), None, Modifiers::default());
    cx.simulate_mouse_down(item.center(), MouseButton::Right, Modifiers::default());
    cx.simulate_mouse_up(item.center(), MouseButton::Right, Modifiers::default());
    cx.run_until_parked();
    assert_eq!(checked_items(cx), ["150 %"]);
    click_item("Fit Width", cx);
    assert_eq!(zoom_of(&workspace, cx), Zoom::FitWidth);
    // The status bar shows what the fit comes to.
    let shown = percent_of(&workspace, cx);
    assert!(shown > 150., "{shown}");
    assert_eq!(
        workspace.read_with(cx, |w, cx| w.zoom_label(cx)),
        format!("{} %", shown.round() as i32)
    );
    // In, out and life size from the menu.
    for (label, expected) in [
        ("Life Size (177 %)", Zoom::Percent(177)),
        ("Zoom Out", Zoom::Percent(175)),
        ("Zoom In", Zoom::Percent(200)),
        ("Actual Size", Zoom::Percent(100)),
    ] {
        assert!(workspace.update_in(cx, |w, window, cx| w.open_menu(
            MenuKind::Zoom,
            None,
            window,
            cx
        )));
        cx.run_until_parked();
        click_item(label, cx);
        assert_eq!(zoom_of(&workspace, cx), expected, "{label}");
    }
    // Page view has "Fit Page" too.
    cx.simulate_keystrokes("cmd-alt-p");
    cx.run_until_parked();
    assert!(workspace.update_in(cx, |w, window, cx| w.open_menu(
        MenuKind::Zoom,
        None,
        window,
        cx
    )));
    cx.run_until_parked();
    let labels = menu_labels(cx).unwrap();
    assert_eq!(
        labels[3..7],
        ["Actual Size", "Fit Width", "Fit Page", "Life Size (177 %)"]
    );
    click_item("Fit Page", cx);
    assert_eq!(zoom_of(&workspace, cx), Zoom::FitPage);
    // Escape and a click outside close the menu and change nothing.
    assert!(workspace.update_in(cx, |w, window, cx| w.open_menu(
        MenuKind::Zoom,
        None,
        window,
        cx
    )));
    cx.run_until_parked();
    assert_eq!(checked_items(cx), ["Fit Page"]);
    escape(cx);
    assert_eq!(menu_labels(cx), None);
    assert_eq!(zoom_of(&workspace, cx), Zoom::FitPage);
}

#[gpui::test]
fn cmd_with_the_wheel_zooms_under_the_pointer_and_is_stored_once(cx: &mut TestAppContext) {
    init(cx);
    let (library, workspace, cx) = open_document(&manuscript(true), cx);
    assert_eq!(zoom_of(&workspace, cx), Zoom::Percent(100));
    let editor = workspace.read_with(cx, |w, _| w.editor().clone());
    let at = position_bounds(&workspace, Position::new(2, 40), cx).center();
    let under = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |editor, _| editor.position_at_point(at).unwrap())
    };
    let before = under(cx);
    let writes = Rc::new(std::cell::Cell::new(0));
    let _subscription = cx.update(|_, cx| {
        let writes = writes.clone();
        cx.subscribe(&library, move |_, _: &writer_library::LibraryEvent, _| {
            writes.set(writes.get() + 1);
        })
    });
    // A swipe: many small turns of the wheel.
    for _ in 0..12 {
        cx.simulate_event(ScrollWheelEvent {
            position: at,
            delta: ScrollDelta::Pixels(point(px(0.), px(24.))),
            modifiers: Modifiers::command(),
            touch_phase: TouchPhase::Moved,
        });
        cx.run_until_parked();
    }
    let reached = percent_of(&workspace, cx);
    assert!(reached > 190. && reached < 215., "{reached}");
    // The line under the pointer is still under it (the sheet has room
    // in this window and stays in the middle of it: sideways the place
    // is kept where a sheet is wider than its room, which the editor's
    // own tests hold).
    let row_of = |position: Position, cx: &mut VisualTestContext| {
        editor.read_with(cx, |editor, _| editor.visual_row(position).unwrap().start)
    };
    let after = under(cx);
    assert_eq!(row_of(after, cx), row_of(before, cx), "{after:?}");
    // Nothing was written while the wheel turned; one write afterwards.
    assert_eq!(stored(&library, PAGE_ZOOM_KEY, cx), None);
    assert_eq!(writes.get(), 0);
    let_it_be_stored(cx);
    assert_eq!(
        stored(&library, PAGE_ZOOM_KEY, cx),
        Some((reached.round() as i32).to_string())
    );
    // Without `cmd` the wheel scrolls and the zoom stays.
    let scrolled = editor.read_with(cx, |editor, _| editor.scroll_offset());
    cx.simulate_event(ScrollWheelEvent {
        position: at,
        delta: ScrollDelta::Pixels(point(px(0.), px(-120.))),
        modifiers: Modifiers::none(),
        touch_phase: TouchPhase::Moved,
    });
    cx.run_until_parked();
    assert_eq!(percent_of(&workspace, cx), reached);
    assert_eq!(
        editor.read_with(cx, |editor, _| editor.scroll_offset()),
        scrolled + px(120.)
    );
    // A pinch is the same thing by another road; over the library or
    // under a dialog it does nothing.
    workspace.update(cx, |w, cx| w.pinch(0.5, at, cx));
    cx.run_until_parked();
    let pinched = percent_of(&workspace, cx);
    assert!((pinched - reached / 2.).abs() <= 1., "{pinched}");
    let sidebar = bounds_of("sidebar", cx).center();
    workspace.update(cx, |w, cx| w.pinch(2., sidebar, cx));
    cx.simulate_keystrokes("cmd-,");
    cx.run_until_parked();
    workspace.update(cx, |w, cx| w.pinch(2., at, cx));
    cx.run_until_parked();
    assert_eq!(percent_of(&workspace, cx), pinched);
}

// ----- what is kept ---------------------------------------------------------------------

/// A library file in a temporary directory, for two launches.
struct Disk {
    dir: tempfile::TempDir,
}

impl Disk {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().expect("a temporary directory"),
        }
    }

    fn launch<'a>(
        &self,
        cx: &'a mut TestAppContext,
    ) -> (
        Entity<Library>,
        Entity<Workspace>,
        &'a mut VisualTestContext,
    ) {
        let path = self.dir.path().join("library.db");
        let library = cx.new(|_| Library::open(path).expect("the library opens"));
        library.update(cx, |library, cx| {
            library
                .set_setting(writer_app::settings::keys::WELCOME_CREATED, &true, cx)
                .unwrap();
        });
        let (workspace, vcx) = open_workspace(&library, cx);
        vcx.simulate_resize(size(px(1900.), px(1100.)));
        vcx.run_until_parked();
        (library, workspace, vcx)
    }
}

#[gpui::test]
fn each_view_keeps_its_zoom_and_it_comes_back_after_a_relaunch(cx: &mut TestAppContext) {
    init(cx);
    let disk = Disk::new();
    let (library, workspace, vcx) = disk.launch(cx);
    let id = add_document_in(&library, &manuscript(false), vcx);
    workspace.update_in(vcx, |w, window, cx| {
        w.open_document(id, true, window, cx);
    });
    vcx.run_until_parked();
    // Flow view at 150 %, page view as wide as the window.
    set_zoom(&workspace, Zoom::Percent(150), vcx);
    vcx.simulate_keystrokes("cmd-alt-p");
    vcx.run_until_parked();
    assert_eq!(
        zoom_of(&workspace, vcx),
        Zoom::Percent(100),
        "page view has a zoom of its own"
    );
    set_zoom(&workspace, Zoom::FitWidth, vcx);
    let fit = percent_of(&workspace, vcx);
    assert!(fit > 200., "{fit}");
    vcx.simulate_keystrokes("cmd-alt-p");
    vcx.run_until_parked();
    assert_eq!(zoom_of(&workspace, vcx), Zoom::Percent(150));
    vcx.simulate_keystrokes("cmd-alt-p");
    vcx.run_until_parked();
    assert_eq!(zoom_of(&workspace, vcx), Zoom::FitWidth);
    let_it_be_stored(vcx);
    assert_eq!(
        stored(&library, FLOW_ZOOM_KEY, vcx),
        Some("150".to_string())
    );
    assert_eq!(
        stored(&library, PAGE_ZOOM_KEY, vcx),
        Some("fit-width".to_string())
    );
    // One more change, and the window goes before the moment has passed
    // after which it would have been written.
    vcx.simulate_keystrokes("cmd-alt-p");
    vcx.run_until_parked();
    vcx.simulate_keystrokes("cmd-=");
    assert_eq!(zoom_of(&workspace, vcx), Zoom::Percent(175));
    assert_eq!(
        stored(&library, FLOW_ZOOM_KEY, vcx),
        Some("150".to_string())
    );
    assert!(vcx.simulate_close(), "the window may close");
    vcx.run_until_parked();
    assert_eq!(
        stored(&library, FLOW_ZOOM_KEY, vcx),
        Some("175".to_string())
    );
    vcx.update(|window, _| window.remove_window());
    vcx.run_until_parked();
    drop(workspace);
    drop(library);

    // The next launch: flow view at 175 %, and page view follows the
    // window again, whatever size the window has now.
    let (_library, workspace, vcx) = disk.launch(cx);
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.open_document_id()),
        Some(id)
    );
    assert_eq!(zoom_of(&workspace, vcx), Zoom::Percent(175));
    vcx.simulate_keystrokes("cmd-alt-p");
    vcx.run_until_parked();
    assert_eq!(zoom_of(&workspace, vcx), Zoom::FitWidth);
    assert!((percent_of(&workspace, vcx) - fit).abs() < 0.5);
    vcx.simulate_resize(size(px(1200.), px(900.)));
    vcx.run_until_parked();
    let narrower = percent_of(&workspace, vcx);
    assert!(narrower < fit - 50., "{narrower} after {fit}");
    let sheet_fits = workspace.read_with(vcx, |w, cx| {
        w.editor().read(cx).max_horizontal_scroll() == px(0.)
    });
    assert!(sheet_fits);
}

fn add_document_in(
    library: &Entity<Library>,
    document: &Document,
    cx: &mut VisualTestContext,
) -> DocumentId {
    let form = writer_app::document_io::StoredForm::of(document, None);
    library.update(cx, |library, cx| {
        library
            .create_document(None, &form.content(), cx)
            .expect("create document")
            .id
    })
}

#[gpui::test]
fn a_page_opens_as_large_as_its_paper_until_a_zoom_is_chosen(cx: &mut TestAppContext) {
    init(cx);
    // A display like this Mac's: 127.5 pixels to the inch, where a point
    // is a 72nd of one. Life size is 177 %.
    cx.update(|cx| zoom::set_display_density(Some(127.5), cx));
    assert_eq!(zoom::life_size_of(127.5), Some(177));
    let disk = Disk::new();
    let (library, workspace, vcx) = disk.launch(cx);
    let id = add_document_in(&library, &manuscript(true), vcx);
    workspace.update_in(vcx, |w, window, cx| {
        w.open_document(id, true, window, cx);
    });
    vcx.run_until_parked();
    // The window has room for the sheet at life size: that is what shows.
    assert_eq!(zoom_of(&workspace, vcx), Zoom::FitWidthUpTo(177));
    assert_eq!(percent_of(&workspace, vcx), 177.);
    assert_eq!(workspace.read_with(vcx, |w, cx| w.zoom_label(cx)), "177 %");
    assert_eq!(workspace.read_with(vcx, |w, _| w.life_size()), Some(177));
    // An A4 sheet is 595.3 points wide: 1054 pixels now, and all of it is
    // in the window.
    let editor = workspace.read_with(vcx, |w, _| w.editor().clone());
    let column = editor.read_with(vcx, |editor, _| {
        let end = editor.document().blocks()[2].len();
        let start = editor.position_bounds(Position::new(2, 0)).unwrap().left();
        let viewport = editor.viewport_bounds().unwrap();
        assert_eq!(editor.max_horizontal_scroll(), px(0.));
        let _ = end;
        (start - viewport.left(), viewport.size.width)
    });
    // The text starts a margin (72 pt) inside the sheet, which is centred.
    let sheet_left = column.0 - px(72. * 1.77);
    assert!(
        (sheet_left - (column.1 - px(595.276 * 1.77)) / 2.).abs() <= px(1.),
        "{column:?}"
    );
    // In a narrower window the sheet is as wide as its room instead of
    // running out of it.
    vcx.simulate_resize(size(px(1100.), px(900.)));
    vcx.run_until_parked();
    let narrow = percent_of(&workspace, vcx);
    assert!(narrow < 177. && narrow > 60., "{narrow}");
    editor.read_with(vcx, |editor, _| {
        assert_eq!(editor.max_horizontal_scroll(), px(0.));
    });
    vcx.simulate_resize(size(px(1900.), px(1100.)));
    vcx.run_until_parked();
    assert_eq!(percent_of(&workspace, vcx), 177.);
    // Flow view starts at 100 %.
    vcx.simulate_keystrokes("cmd-alt-p");
    vcx.run_until_parked();
    assert_eq!(zoom_of(&workspace, vcx), Zoom::Percent(100));
    vcx.simulate_keystrokes("cmd-alt-p");
    vcx.run_until_parked();
    // It is not a choice, so nothing is kept for it...
    let_it_be_stored(vcx);
    assert_eq!(stored(&library, PAGE_ZOOM_KEY, vcx), None);
    assert_eq!(stored(&library, FLOW_ZOOM_KEY, vcx), None);
    // ...until the writer chooses: from then on the page has that zoom.
    vcx.simulate_keystrokes("cmd--");
    assert_eq!(zoom_of(&workspace, vcx), Zoom::Percent(175));
    let_it_be_stored(vcx);
    assert_eq!(
        stored(&library, PAGE_ZOOM_KEY, vcx),
        Some("175".to_string())
    );
    workspace.update(vcx, |w, cx| assert!(w.flush(cx)));
    vcx.update(|window, _| window.remove_window());
    vcx.run_until_parked();
    drop((workspace, editor, library));

    let (library, workspace, vcx) = disk.launch(cx);
    assert_eq!(zoom_of(&workspace, vcx), Zoom::Percent(175));
    // "Actual Size" is 100 % whatever the display.
    vcx.simulate_keystrokes("cmd-shift-0");
    assert_eq!(zoom_of(&workspace, vcx), Zoom::Percent(100));
    let_it_be_stored(vcx);
    assert_eq!(
        stored(&library, PAGE_ZOOM_KEY, vcx),
        Some("100".to_string())
    );
}

#[gpui::test]
fn on_a_display_that_does_not_say_its_size_a_page_opens_at_100(cx: &mut TestAppContext) {
    init(cx);
    let (_library, workspace, cx) = open_document(&manuscript(true), cx);
    assert_eq!(zoom_of(&workspace, cx), Zoom::Percent(100));
    assert_eq!(workspace.read_with(cx, |w, _| w.life_size()), None);
    // There is no life size to offer, and the command says why.
    assert!(workspace.update_in(cx, |w, window, cx| w.open_menu(
        MenuKind::Zoom,
        None,
        window,
        cx
    )));
    cx.run_until_parked();
    let labels = menu_labels(cx).unwrap();
    assert!(!labels.iter().any(|label| label.starts_with("Life Size")));
    escape(cx);
    run_command(&workspace, "Zoom to life size", cx);
    assert_eq!(zoom_of(&workspace, cx), Zoom::Percent(100));
    let said = workspace.read_with(cx, |w, cx| {
        w.toast_layer()
            .read(cx)
            .toasts()
            .any(|toast| toast.message.contains("life size is not known"))
    });
    assert!(said);
}

#[gpui::test]
fn what_is_stored_is_read_leniently(cx: &mut TestAppContext) {
    assert_eq!(zoom::from_stored("125"), Some(Zoom::Percent(125)));
    assert_eq!(zoom::from_stored(" 67 "), Some(Zoom::Percent(67)));
    assert_eq!(zoom::from_stored("133.4"), Some(Zoom::Percent(133)));
    assert_eq!(zoom::from_stored("fit-width"), Some(Zoom::FitWidth));
    assert_eq!(zoom::from_stored("fit-page"), Some(Zoom::FitPage));
    for unreadable in ["", "huge", "-3", "0", "49", "401", "NaN", "fit"] {
        assert_eq!(zoom::from_stored(unreadable), None, "{unreadable:?}");
    }
    for zoom in [
        Zoom::Percent(50),
        Zoom::Percent(177),
        Zoom::FitWidth,
        Zoom::FitPage,
    ] {
        let text = zoom::stored_form(zoom).unwrap();
        assert_eq!(zoom::from_stored(&text), Some(zoom));
    }
    assert_eq!(zoom::stored_form(Zoom::FitWidthUpTo(177)), None);
    // Life size from what a display says about itself.
    assert_eq!(zoom::density_of(3008., 599.).map(f32::round), Some(128.));
    assert_eq!(zoom::density_of(1728., 344.).map(f32::round), Some(128.));
    assert_eq!(zoom::density_of(1920., 0.), None);
    assert_eq!(zoom::density_of(1920., 30_000.), None);
    assert_eq!(zoom::life_size_of(72.), Some(100));
    assert_eq!(zoom::life_size_of(109.), Some(151));
    assert_eq!(zoom::life_size_of(20.), None);
    assert_eq!(zoom::first_page_zoom(Some(177)), Zoom::FitWidthUpTo(177));
    assert_eq!(zoom::first_page_zoom(None), Zoom::Percent(100));

    // A library that holds something this version cannot read: the views
    // have the zoom they start with, and what is there is left alone
    // until a zoom is chosen.
    init(cx);
    let library = library(cx);
    library.update(cx, |library, cx| {
        library.set_setting(FLOW_ZOOM_KEY, "enormous", cx).unwrap();
        library.set_setting(PAGE_ZOOM_KEY, &42, cx).unwrap();
    });
    let id = add_document(&library, None, &manuscript(false), cx);
    let (workspace, cx) = open_workspace(&library, cx);
    workspace.update_in(cx, |w, window, cx| {
        w.open_document(id, true, window, cx);
    });
    cx.run_until_parked();
    assert_eq!(zoom_of(&workspace, cx), Zoom::Percent(100));
    cx.simulate_keystrokes("cmd-=");
    let_it_be_stored(cx);
    assert_eq!(stored(&library, FLOW_ZOOM_KEY, cx), Some("110".to_string()));
}

// ----- what is anchored to the text ------------------------------------------------------

/// The fake spell checker: everything in [`manuscript`] is a word, except
/// "teh".
fn spell_service() -> Rc<FakeSpellService> {
    Rc::new(
        FakeSpellService::with_words([
            "the",
            "harbor",
            "was",
            "a",
            "plain",
            "grey",
            "fact",
            "and",
            "boats",
            "came",
            "by",
            "road",
            "every",
            "spring",
            "she",
            "bought",
            "bread",
            "salt",
            "ropes",
            "paragraph",
        ])
        .suggest("teh", ["the", "ten"]),
    )
}

/// A workspace with the fake spell checker and the fake engine of the
/// assistant, `page_view` on or off, at `percent`.
fn open_at<'a>(
    page_view: bool,
    cx: &'a mut TestAppContext,
) -> (
    Entity<Library>,
    Entity<Workspace>,
    &'a mut VisualTestContext,
) {
    let service = spell_service();
    cx.update(|cx| {
        spelling::set_service(service, cx);
        assistant::set_backend(Arc::new(FakeBackend::new()), cx);
    });
    let library = library(cx);
    library.update(cx, |library, cx| {
        library
            .set_setting(writer_app::settings::keys::SPELLING_LANGUAGE, "en", cx)
            .unwrap();
    });
    let id = add_document(&library, None, &manuscript(page_view), cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    vcx.simulate_resize(size(px(1900.), px(1100.)));
    workspace.update_in(vcx, |w, window, cx| {
        w.open_document(id, true, window, cx);
    });
    vcx.run_until_parked();
    // The checker's first slice and the look for the engine.
    vcx.executor().advance_clock(Duration::from_secs(2));
    vcx.run_until_parked();
    (library, workspace, vcx)
}

#[gpui::test]
fn the_tools_open_at_the_text_at_every_zoom(cx: &mut TestAppContext) {
    init(cx);
    for page_view in [false, true] {
        let (_library, workspace, cx) = open_at(page_view, cx);
        for percent in ZOOMS {
            let what = format!("page view {page_view}, {percent} %");
            set_zoom(&workspace, Zoom::Percent(percent), cx);
            let scale = f32::from(percent) / 100.;
            // "plain grey" in the long paragraph.
            let range = TextRange::in_block(2, 17, 27);
            select(&workspace, range, cx);
            let selection = selection_bounds(&workspace, cx);
            // The selection is as tall as a line is at this zoom.
            assert!(
                (selection.size.height - px(27.) * scale).abs() < px(0.05),
                "{what}: {selection:?}"
            );

            // The format menu, the font picker and the link popover come
            // up at the selection.
            for (keys, selector) in [
                ("cmd-.", "tools-menu"),
                ("cmd-t", "tools-menu"),
                ("cmd-k", "link-popover"),
            ] {
                cx.simulate_keystrokes(keys);
                cx.run_until_parked();
                let popup = bounds_of(selector, cx);
                assert_at(popup, selection, &format!("{what}, {keys}"));
                escape(cx);
                assert!(editor_focused(&workspace, cx), "{what}, after {keys}");
            }
            // The context menu from the keyboard opens at the caret.
            cx.simulate_keystrokes("ctrl-enter");
            cx.run_until_parked();
            assert!(menu_labels(cx).is_some(), "{what}");
            let menu = bounds_of("context-menu", cx);
            assert_at(menu, selection, &format!("{what}, ctrl-enter"));
            escape(cx);

            // The insert menu at a slash in an empty paragraph.
            select(&workspace, TextRange::in_block(3, 0, 0), cx);
            let at = caret(&workspace, cx);
            cx.simulate_input("/");
            cx.run_until_parked();
            let menu = bounds_of("tools-menu", cx);
            assert_at(menu, at, &format!("{what}, the insert menu"));
            escape(cx);
            cx.simulate_keystrokes("backspace");
            cx.run_until_parked();

            // Find: the match it goes to is selected, and on screen.
            cx.simulate_keystrokes("cmd-f");
            cx.simulate_input("spring");
            cx.run_until_parked();
            cx.simulate_keystrokes("enter");
            cx.run_until_parked();
            let found = workspace.read_with(cx, |w, cx| {
                let editor = w.editor().read(cx);
                (
                    editor.state().selected_text(),
                    editor.selection_bounds().unwrap(),
                    editor.viewport_bounds().unwrap(),
                )
            });
            assert_eq!(found.0, "spring", "{what}");
            assert!(
                found.1.top() >= found.2.top() && found.1.bottom() <= found.2.bottom(),
                "{what}: the match is at {:?}",
                found.1
            );
            escape(cx);
            assert!(editor_focused(&workspace, cx), "{what}");

            // The quick chat of the assistant at the selection.
            select(&workspace, range, cx);
            let selection = selection_bounds(&workspace, cx);
            cx.simulate_keystrokes("cmd-j");
            cx.run_until_parked();
            let chat = bounds_of("quick-chat", cx);
            assert_at(chat, selection, &format!("{what}, the quick chat"));
            escape(cx);
            cx.run_until_parked();
            assert!(editor_focused(&workspace, cx), "{what}");
        }
    }
}

#[gpui::test]
fn the_pointer_finds_the_text_at_every_zoom(cx: &mut TestAppContext) {
    init(cx);
    for page_view in [false, true] {
        let (_library, workspace, cx) = open_at(page_view, cx);
        let editor = workspace.read_with(cx, |w, _| w.editor().clone());
        // A comment on "bread" (the word at the caret).
        select(&workspace, TextRange::in_block(1, 13, 13), cx);
        cx.simulate_keystrokes("cmd-alt-m");
        cx.run_until_parked();
        cx.simulate_input("Which bread?");
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        cx.simulate_keystrokes("cmd-1");
        cx.run_until_parked();
        let thread = workspace.read_with(cx, |w, cx| {
            let threads = w.comments().read(cx).threads();
            assert_eq!(threads.len(), 1);
            threads[0].id
        });

        for percent in ZOOMS {
            let what = format!("page view {page_view}, {percent} %");
            set_zoom(&workspace, Zoom::Percent(percent), cx);
            let inside = |position: Position, cx: &mut VisualTestContext| {
                let bounds = position_bounds(&workspace, position, cx);
                point(bounds.left() + px(2.), bounds.center().y)
            };
            // A click puts the caret where the pointer is.
            let at = inside(Position::new(2, 40), cx);
            cx.simulate_click(at, Modifiers::default());
            cx.run_until_parked();
            assert_eq!(
                editor.read_with(cx, |editor, _| editor.selection()),
                Selection::caret(Position::new(2, 40)),
                "{what}"
            );
            // A click on the commented word makes its thread the active
            // one.
            let at = inside(Position::new(1, 14), cx);
            cx.simulate_click(at, Modifiers::default());
            cx.run_until_parked();
            assert_eq!(
                workspace.read_with(cx, |w, cx| w.comments().read(cx).active_thread()),
                Some(thread),
                "{what}"
            );
            // A right-click on the misspelled word: the menu opens at the
            // pointer and begins with what the checker suggests.
            let teh = inside(Position::new(1, 30), cx);
            cx.simulate_mouse_move(teh, None, Modifiers::default());
            cx.simulate_mouse_down(teh, MouseButton::Right, Modifiers::default());
            cx.simulate_mouse_up(teh, MouseButton::Right, Modifiers::default());
            cx.run_until_parked();
            let labels = menu_labels(cx).unwrap_or_else(|| panic!("{what}: no menu"));
            assert_eq!(labels[..2], ["the", "ten"], "{what}: {labels:?}");
            assert_eq!(
                editor.read_with(cx, |editor, _| editor.state().selected_text()),
                "teh",
                "{what}"
            );
            let menu = bounds_of("context-menu", cx);
            assert!(
                (menu.left() - teh.x).abs() <= px(4.) && (menu.top() - teh.y).abs() <= px(4.),
                "{what}: the menu is at {menu:?}, the pointer at {teh:?}"
            );
            escape(cx);

            // A drag over two words selects them, and the format bar comes
            // up over the selection.
            let (from, to) = (
                inside(Position::new(2, 4), cx),
                inside(Position::new(2, 16), cx),
            );
            cx.simulate_mouse_move(from, None, Modifiers::default());
            cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::default());
            cx.simulate_mouse_move(to, MouseButton::Left, Modifiers::default());
            cx.simulate_mouse_up(to, MouseButton::Left, Modifiers::default());
            cx.run_until_parked();
            assert_eq!(
                editor.read_with(cx, |editor, _| editor.state().selected_text()),
                "harbor was a",
                "{what}"
            );
            cx.executor()
                .advance_clock(FORMAT_BAR_DELAY + Duration::from_millis(20));
            cx.run_until_parked();
            let bar = bounds_of("format-bar", cx);
            let selection = selection_bounds(&workspace, cx);
            assert_at(bar, selection, &format!("{what}, the format bar"));
            escape(cx);
            cx.simulate_keystrokes("cmd-up");
            cx.run_until_parked();
        }
    }
}

#[gpui::test]
fn vim_moves_by_the_rows_on_screen_at_every_zoom(cx: &mut TestAppContext) {
    init(cx);
    for page_view in [false, true] {
        let (_library, workspace, cx) = open_document(&manuscript(page_view), cx);
        let editor = workspace.read_with(cx, |w, _| w.editor().clone());
        cx.simulate_keystrokes("cmd-alt-v");
        cx.run_until_parked();
        for percent in ZOOMS {
            let what = format!("page view {page_view}, {percent} %");
            let scale = f32::from(percent) / 100.;
            set_zoom(&workspace, Zoom::Percent(percent), cx);
            // The block caret covers the character it is on.
            select(&workspace, TextRange::in_block(2, 5, 5), cx);
            cx.simulate_keystrokes("escape");
            cx.run_until_parked();
            assert_eq!(
                editor.read_with(cx, |editor, _| editor.config().caret_style),
                CaretStyle::Block,
                "{what}"
            );
            let start = caret(&workspace, cx);
            let letter = position_bounds(&workspace, Position::new(2, 5), cx);
            assert!((start.left() - letter.left()).abs() < px(0.05), "{what}");
            // (One glyph of the test platform's font is 0.6 of its size.)
            assert!(
                (start.size.width - px(0.6 * 17.) * scale).abs() < px(0.05),
                "{what}: {start:?}"
            );
            assert!(
                (start.size.height - px(27.) * scale).abs() < px(0.05),
                "{what}"
            );
            // `gj` and `gk` go a row of the screen down and up, in the
            // same column; the paragraph has several rows at every zoom.
            cx.simulate_input("gj");
            cx.run_until_parked();
            let below = caret(&workspace, cx);
            let head = editor.read_with(cx, |editor, _| editor.selection().head);
            assert_eq!(head.block, 2, "{what}");
            assert!(head.offset > 5, "{what}: {head:?}");
            assert!(
                (below.top() - start.top() - px(27.) * scale).abs() < px(0.05),
                "{what}: {start:?} then {below:?}"
            );
            assert!((below.left() - start.left()).abs() < px(0.05), "{what}");
            cx.simulate_input("gk");
            cx.run_until_parked();
            assert_eq!(
                editor.read_with(cx, |editor, _| editor.selection().head),
                Position::new(2, 5),
                "{what}"
            );
            // `H` and `L`: the first and the last row on screen are on
            // screen.
            cx.simulate_input("L");
            cx.run_until_parked();
            let (last, viewport) = editor.read_with(cx, |editor, _| {
                (
                    editor.caret_bounds().unwrap(),
                    editor.viewport_bounds().unwrap(),
                )
            });
            assert!(
                last.bottom() <= viewport.bottom() + px(0.5)
                    && last.bottom() > viewport.bottom() - px(27. * 6.) * scale.max(1.),
                "{what}: the last row on screen is at {last:?} in {viewport:?}"
            );
            cx.simulate_input("H");
            cx.run_until_parked();
            let first = caret(&workspace, cx);
            assert!(first.top() >= viewport.top() - px(0.5), "{what}: {first:?}");
            cx.simulate_input("gg");
            cx.run_until_parked();
        }
    }
}

#[gpui::test]
fn the_pages_do_not_depend_on_the_zoom(cx: &mut TestAppContext) {
    init(cx);
    let (_library, workspace, cx) = open_document(&manuscript(true), cx);
    let editor = workspace.read_with(cx, |w, _| w.editor().clone());
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.set_caret(Position::new(60, 10)), cx);
    });
    cx.run_until_parked();
    // (Pages that were never on screen are estimates until the layout in
    // the background has been there: let it finish.)
    editor.update(cx, |editor, cx| editor.finish_layout(cx));
    cx.run_until_parked();
    let page = workspace.update(cx, |w, cx| w.status_line(cx).page);
    let (pages, row) = editor.read_with(cx, |editor, _| {
        (
            editor.page_count(),
            editor
                .visual_row(Position::new(60, 10))
                .map(|row| row.start),
        )
    });
    assert!(pages.unwrap() >= 6, "{pages:?}");
    assert!(page.as_deref().unwrap().starts_with("Page "), "{page:?}");
    for zoom in [
        Zoom::Percent(50),
        Zoom::Percent(75),
        Zoom::Percent(150),
        Zoom::Percent(400),
        Zoom::FitWidth,
        Zoom::FitPage,
        Zoom::Percent(100),
    ] {
        set_zoom(&workspace, zoom, cx);
        assert_eq!(
            workspace.update(cx, |w, cx| w.status_line(cx).page),
            page,
            "{zoom:?}"
        );
        editor.read_with(cx, |editor, _| {
            assert_eq!(editor.page_count(), pages, "{zoom:?}");
            assert_eq!(
                editor
                    .visual_row(Position::new(60, 10))
                    .map(|row| row.start),
                row,
                "{zoom:?}"
            );
        });
    }
    // The words and the reading time are the document's, not the view's.
    let words = workspace.update(cx, |w, cx| w.status_line(cx).words);
    set_zoom(&workspace, Zoom::Percent(300), cx);
    assert_eq!(workspace.update(cx, |w, cx| w.status_line(cx).words), words);
    // What is stored and exported does not know the zoom: the document
    // is the same, byte for byte.
    let document =
        |cx: &mut VisualTestContext| editor.read_with(cx, |editor, _| editor.document().to_json());
    let at_300 = document(cx);
    set_zoom(&workspace, Zoom::Percent(100), cx);
    assert_eq!(document(cx), at_300);
    workspace.read_with(cx, |w, _| {
        assert_eq!(w.save_state(), Some(SaveState::Saved))
    });
}

// ----- the measurement of a page ----------------------------------------------------------

#[gpui::test]
fn the_page_fit_step_counts_what_the_lines_of_a_page_hold(cx: &mut TestAppContext) {
    use writer_app::workspace::{PageFit, characters_per_row};
    use writer_core::PaperSize;

    // What a script writes.
    let fit = PageFit::parse("Times New Roman | 12 | 1.5 | A4 | /some/text.txt").unwrap();
    assert_eq!(fit.font, "Times New Roman");
    assert_eq!((fit.size, fit.line_height), (12., 1.5));
    assert_eq!(fit.paper, PaperSize::A4);
    assert_eq!(
        PageFit::parse("Arial|11|1.15|letter|t.txt").unwrap().paper,
        PaperSize::UsLetter
    );
    for wrong in [
        "Arial|11|1.15|a4",
        "Arial|big|1|a4|t",
        "Arial|11|1|a3|t",
        "",
    ] {
        assert!(PageFit::parse(wrong).is_err(), "{wrong:?}");
    }
    // Characters per full row: the space a row ends in counts, the last
    // row does not, and a character is a character whatever its bytes.
    let text = "aaaa bbbb cccc ééé dd";
    let starts = [0, 5, 10, 15];
    assert_eq!(&text[15..], "ééé dd");
    assert_eq!(characters_per_row(text, &starts), (4, 5., 5, 5));
    assert_eq!(characters_per_row("ééé ddd e", &[0, 7]), (2, 4., 4, 4));
    assert_eq!(characters_per_row("short", &[0]), (1, 5., 5, 5));

    // The step itself, with the font of the test platform: every glyph is
    // 0.6 of the size wide, so a line of 12 pt type on A4 with 2.54 cm
    // margins (451.3 pt) has room for 62 characters (and the space a line
    // ends in may hang over the edge: 63), and a page (697.9 pt) for 38
    // lines of 18 pt.
    init(cx);
    let library = library(cx);
    let (workspace, cx) = open_workspace(&library, cx);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("text.txt");
    let sentence = "The harbor was a plain grey fact and the boats came by road every spring. ";
    std::fs::write(&path, sentence.repeat(60)).unwrap();
    let fit = PageFit::parse(&format!("Charter|12|1.5|a4|{}", path.display())).unwrap();
    let measure = |cx: &mut VisualTestContext| {
        let text = workspace
            .update_in(cx, |w, window, cx| w.open_page_fit(&fit, window, cx))
            .expect("the document is made");
        cx.run_until_parked();
        workspace
            .update(cx, |w, cx| w.report_page_fit(&text, cx))
            .expect("the text is laid out")
    };
    let report = measure(cx);
    assert!(
        report[0].contains("Charter 12 pt, line spacing 1.5 (18.00 pt a line), A4")
            && report[0].contains("451.3 x 697.9 pt"),
        "{report:?}"
    );
    assert!(
        report[1].contains("is 73 rows; a full row holds 61.6 characters (58 to 63)"),
        "{report:?}"
    );
    assert!(
        report[2].starts_with("38 rows on the first page"),
        "{report:?}"
    );
    // The document is in page view at those settings, and the zoom the
    // page is drawn at is not part of any of it.
    workspace.read_with(cx, |w, cx| {
        let editor = w.editor().read(cx);
        assert!(editor.page_view());
        assert_eq!(editor.document().settings().typography.base_size, 12.);
    });
    set_zoom(&workspace, Zoom::Percent(200), cx);
    assert_eq!(measure(cx), report);
}

// ----- the pinch ------------------------------------------------------------------------

#[test]
fn a_pinch_is_a_factor_at_a_place() {
    use writer_app::pinch::{Pinch, combined};

    // The change of magnification an event carries is what the zoom is
    // multiplied by, less one; nonsense multiplies by one, and no single
    // event more than doubles or halves.
    assert_eq!(Pinch::factor(0.), 1.);
    assert!((Pinch::factor(0.05) - 1.05).abs() < 1e-6);
    assert!((Pinch::factor(-0.2) - 0.8).abs() < 1e-6);
    assert_eq!(Pinch::factor(7.), 2.);
    assert_eq!(Pinch::factor(-3.), 0.5);
    assert_eq!(Pinch::factor(f32::NAN), 1.);
    assert_eq!(Pinch::factor(f32::INFINITY), 1.);
    // Events that arrived together are one pinch, at the last place.
    let at = |magnification: f32, x: f32| Pinch {
        window: 7,
        magnification,
        x,
        y: 300.,
    };
    assert_eq!(combined(&[]), None);
    let (factor, x, y) = combined(&[at(0.1, 100.), at(0.1, 104.), at(-0.05, 108.)]).unwrap();
    assert!((factor - 1.1 * 1.1 * 0.95).abs() < 1e-5, "{factor}");
    assert_eq!((x, y), (108., 300.));
}
