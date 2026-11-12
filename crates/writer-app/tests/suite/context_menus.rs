//! The context menus, end to end in the assembled workspace. Each menu is
//! composed in the crate its object lives in and the app adds its own items
//! through that crate's hook; here every one of them is opened the way a
//! person opens it, with a right-click where the object was drawn, and an
//! item is chosen with a click on its row.
//!
//! Also here: the second key of the context menu (`shift-f10`), and the
//! floating format bar over a selection made with the mouse, with the app's
//! "Add Comment" button and its switch in the settings.

use gpui::{
    Bounds, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, Pixels, Point, TestAppContext,
    point, px,
};
use writer_app::{
    export::{ExportPicker, ExportTarget},
    settings::keys,
    settings_view::{SettingRow, SettingsView},
    shortcuts::collect_shortcuts,
    version_history::VersionHistory,
    workspace::MenuKind,
};
use writer_core::{EditorState, Position, Selection};
use writer_editor_tools::{FORMAT_BAR_DELAY, Mark, ToolKind};
use writer_library::{
    DocumentId, FolderId, HEADER_HEIGHT, LibraryItem, ROOT_ROW_HEIGHT, ROW_HEIGHT,
};
use writer_ui::{ContextMenu, theme::TITLEBAR_HEIGHT};

use super::*;

fn bounds_of(selector: &'static str, cx: &mut VisualTestContext) -> Bounds<Pixels> {
    cx.debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn"))
}

fn right_click_at(at: Point<Pixels>, cx: &mut VisualTestContext) {
    cx.simulate_mouse_move(at, None, Modifiers::default());
    cx.simulate_mouse_down(at, MouseButton::Right, Modifiers::default());
    cx.simulate_mouse_up(at, MouseButton::Right, Modifiers::default());
    cx.run_until_parked();
}

/// The open context menu: its labels ("-" for a separator).
fn menu(cx: &mut VisualTestContext) -> Option<Vec<String>> {
    cx.update(|window, cx| ContextMenu::current(window, cx).map(|menu| menu.read(cx).labels()))
}

#[track_caller]
fn open_menu(what: &str, cx: &mut VisualTestContext) -> Vec<String> {
    menu(cx).unwrap_or_else(|| panic!("a right-click on {what} opens no menu"))
}

#[track_caller]
fn assert_has(labels: &[String], wanted: &[&str], what: &str) {
    for label in wanted {
        assert!(
            labels.iter().any(|have| have == label),
            "the menu of {what} has no {label:?}: {labels:?}"
        );
    }
}

/// Whether the item called `label` of the open menu is dimmed.
fn dimmed(label: &str, cx: &mut VisualTestContext) -> bool {
    cx.update(|window, cx| {
        let menu = ContextMenu::current(window, cx).expect("a menu is open");
        menu.read(cx)
            .items()
            .iter()
            .find(|item| item.title().as_ref() == label)
            .unwrap_or_else(|| panic!("no item {label:?}"))
            .is_disabled()
    })
}

/// Clicks the row of the item called `label` of the open menu, where the
/// menu drew it.
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

fn dismiss(cx: &mut VisualTestContext) {
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert_eq!(menu(cx), None, "escape closes the menu");
}

fn has_modal(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> bool {
    workspace.read_with(cx, |w, cx| w.modal_layer().read(cx).has_modal())
}

/// The middle of the library row that shows `item`, as the sidebar draws
/// it: under the title band, the panel's header and the "Library" row.
fn library_row(
    workspace: &Entity<Workspace>,
    item: LibraryItem,
    cx: &mut VisualTestContext,
) -> Point<Pixels> {
    let ix = workspace.read_with(cx, |w, cx| {
        w.panel()
            .read(cx)
            .visible_items()
            .iter()
            .position(|row| *row == item)
            .unwrap_or_else(|| panic!("{item:?} has no row"))
    });
    let sidebar = bounds_of("sidebar", cx);
    point(
        sidebar.left() + px(120.),
        sidebar.top()
            + TITLEBAR_HEIGHT
            + HEADER_HEIGHT
            + ROOT_ROW_HEIGHT
            + px(4.)
            + ROW_HEIGHT * ix as f32
            + ROW_HEIGHT / 2.,
    )
}

/// A point inside the character at `position` of the text.
fn in_text(
    workspace: &Entity<Workspace>,
    position: Position,
    cx: &mut VisualTestContext,
) -> Point<Pixels> {
    let bounds = workspace.read_with(cx, |w, cx| {
        w.editor()
            .read(cx)
            .position_bounds(position)
            .unwrap_or_else(|| panic!("{position:?} is not on screen"))
    });
    point(bounds.left() + px(2.), bounds.center().y)
}

fn selection(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Selection {
    workspace.read_with(cx, |w, cx| w.editor().read(cx).state().selection())
}

fn selected_text(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> String {
    workspace.read_with(cx, |w, cx| w.editor().read(cx).state().selected_text())
}

/// Heading, a paragraph with a comment on its first word and a link on
/// "the harbour", and a second paragraph.
fn fixture_document() -> Document {
    let body = "Salt came by the harbour road every spring.";
    let mut state = EditorState::new(Document::from_blocks(vec![
        Block::with_text(BlockKind::heading(1), "The Salt Road"),
        Block::paragraph(body),
        Block::paragraph("Nobody remembers who paid for it."),
    ]));
    state.set_selection(Selection::new(Position::new(1, 0), Position::new(1, 4)));
    state
        .add_comment("Sea salt or rock salt?", "Ada", 1_700_000_000_000)
        .expect("a comment on the first word");
    let link = body.find("the harbour").unwrap();
    state.set_selection(Selection::new(
        Position::new(1, link),
        Position::new(1, link + "the harbour".len()),
    ));
    assert!(state.set_link("https://example.com/harbour"));
    state.into_document()
}

struct Fixture {
    workspace: Entity<Workspace>,
    folder: FolderId,
    chapter: DocumentId,
    open: DocumentId,
}

fn fixture(cx: &mut TestAppContext) -> (Fixture, &mut VisualTestContext) {
    init(cx);
    let library = library(cx);
    let folder = library.update(cx, |library, cx| {
        library.create_folder(None, "Part one", cx).unwrap().id
    });
    let chapter = add_document(
        &library,
        Some(folder),
        &simple_document("Chapter one", "The first chapter."),
        cx,
    );
    let open = add_document(&library, None, &fixture_document(), cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    workspace.update_in(vcx, |w, window, cx| {
        w.open_document(open, true, window, cx);
        w.panel().update(cx, |panel, cx| {
            panel.set_expanded(folder, true, cx);
        });
    });
    vcx.run_until_parked();
    (
        Fixture {
            workspace,
            folder,
            chapter,
            open,
        },
        vcx,
    )
}

/// A right-click on every kind of object opens that object's menu, with the
/// items of the crate that owns it and the ones the app adds, and a click
/// on an item does the thing.
#[gpui::test]
fn a_right_click_on_every_object_opens_its_menu(cx: &mut TestAppContext) {
    let (fixture, vcx) = fixture(cx);
    let Fixture {
        workspace,
        folder,
        chapter,
        open,
    } = fixture;
    assert!(workspace.read_with(vcx, |w, _| w.has_right_panel()));

    // ----- a document row of the library -----------------------------------------
    let at = library_row(&workspace, LibraryItem::Document(chapter), vcx);
    right_click_at(at, vcx);
    let labels = open_menu("a document row", vcx);
    assert_has(
        &labels,
        &[
            "Open",
            "Rename",
            "Duplicate",
            "Move To…",
            // From the app, through the panel's hook.
            "Export…",
            "Version History…",
            "Move to Trash",
        ],
        "a document row",
    );
    // The right-click selected the row it was on; the open document stays.
    assert_eq!(
        workspace.read_with(vcx, |w, cx| w.panel().read(cx).selected()),
        Some(LibraryItem::Document(chapter))
    );
    click_item("Export…", vcx);
    let picker = workspace
        .read_with(vcx, |w, cx| {
            w.modal_layer().read(cx).active::<ExportPicker>()
        })
        .expect("Export… opens the export picker");
    assert_eq!(
        picker.read_with(vcx, |picker, _| picker.delegate.target()),
        ExportTarget::Document(chapter)
    );
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.open_document_id()),
        Some(open)
    );
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    assert!(!has_modal(&workspace, vcx));

    // One of the panel's own items, by a click: the row is renamed in place.
    right_click_at(at, vcx);
    click_item("Rename", vcx);
    assert!(workspace.read_with(vcx, |w, cx| w.panel().read(cx).renaming().is_some()));
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();

    // ----- a folder row ----------------------------------------------------------
    let at = library_row(&workspace, LibraryItem::Folder(folder), vcx);
    right_click_at(at, vcx);
    let labels = open_menu("a folder row", vcx);
    assert_has(
        &labels,
        &[
            "New Document Here",
            "New Folder Here",
            "Rename",
            "Export Folder…",
            "Move to Trash",
        ],
        "a folder row",
    );
    click_item("Export Folder…", vcx);
    let picker = workspace
        .read_with(vcx, |w, cx| {
            w.modal_layer().read(cx).active::<ExportPicker>()
        })
        .expect("Export Folder… opens the export picker");
    assert_eq!(
        picker.read_with(vcx, |picker, _| picker.delegate.target()),
        ExportTarget::Folder(folder)
    );
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();

    // ----- the empty area under the rows -----------------------------------------
    let sidebar = bounds_of("sidebar", vcx);
    let rows = workspace.read_with(vcx, |w, cx| w.panel().read(cx).visible_items().len());
    let last_row = library_row(
        &workspace,
        workspace.read_with(vcx, |w, cx| w.panel().read(cx).visible_items()[rows - 1]),
        vcx,
    );
    let empty = point(last_row.x, last_row.y + ROW_HEIGHT * 4.);
    assert!(empty.y < sidebar.bottom() - px(60.), "room under the rows");
    right_click_at(empty, vcx);
    let labels = open_menu("the empty library area", vcx);
    assert_has(
        &labels,
        &["New Document", "New Folder"],
        "the empty library area",
    );
    assert!(
        !labels.iter().any(|label| label == "Rename"),
        "nothing is selected there: {labels:?}"
    );
    let folders_before = workspace.read_with(vcx, |w, cx| {
        w.panel()
            .read(cx)
            .visible_items()
            .iter()
            .filter(|item| matches!(item, LibraryItem::Folder(_)))
            .count()
    });
    click_item("New Folder", vcx);
    // A new folder at the top level, its name ready to be typed.
    assert!(workspace.read_with(vcx, |w, cx| w.panel().read(cx).renaming().is_some()));
    vcx.simulate_input("Part two");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    let folders_after = workspace.read_with(vcx, |w, cx| {
        w.panel()
            .read(cx)
            .visible_items()
            .iter()
            .filter(|item| matches!(item, LibraryItem::Folder(_)))
            .count()
    });
    assert_eq!(folders_after, folders_before + 1);

    // ----- a selection in the text -----------------------------------------------
    // "spring" is selected with a double click, then right-clicked: the
    // selection stays and the menu is about it.
    vcx.simulate_keystrokes("cmd-1");
    let word = "Salt came by the harbour road every ".len();
    let at = in_text(&workspace, Position::new(1, word + 2), vcx);
    vcx.simulate_mouse_move(at, None, Modifiers::default());
    for clicks in 1..=2 {
        vcx.simulate_event(MouseDownEvent {
            position: at,
            modifiers: Modifiers::default(),
            button: MouseButton::Left,
            click_count: clicks,
            first_mouse: false,
        });
        vcx.simulate_event(MouseUpEvent {
            position: at,
            modifiers: Modifiers::default(),
            button: MouseButton::Left,
            click_count: clicks,
        });
    }
    vcx.run_until_parked();
    assert_eq!(selected_text(&workspace, vcx), "spring");
    right_click_at(at, vcx);
    let labels = open_menu("a selection", vcx);
    assert_eq!(selected_text(&workspace, vcx), "spring");
    assert_has(
        &labels,
        &[
            "Cut",
            "Copy",
            "Paste",
            "Bold",
            "Italic",
            "Highlight",
            "Link",
            "Turn Into",
            "Select All",
            // From the app, through the tools' hook.
            "Add Comment",
        ],
        "a selection",
    );
    assert!(!labels.iter().any(|label| label == "Go to Comment"));
    assert!(!labels.iter().any(|label| label == "Open Link"));
    assert_eq!(
        workspace.read_with(vcx, |w, cx| w.tools().read(cx).active_tool()),
        Some(ToolKind::ContextMenu)
    );
    click_item("Bold", vcx);
    assert_eq!(menu(vcx), None);
    let bold = workspace.read_with(vcx, |w, cx| {
        let document = w.editor().read(cx).document();
        document.blocks()[1]
            .attrs_at(word + 2)
            .is_some_and(|attrs| attrs.bold)
    });
    assert!(bold, "Bold in the menu makes the selection bold");
    assert!(editor_focused(&workspace, vcx));

    // "Add Comment" on the same selection opens the composer.
    right_click_at(at, vcx);
    assert!(!dimmed("Add Comment", vcx));
    click_item("Add Comment", vcx);
    assert!(workspace.read_with(vcx, |w, cx| w.comments().read(cx).is_composing()));
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    assert!(!workspace.read_with(vcx, |w, cx| w.comments().read(cx).is_composing()));

    // ----- a link ----------------------------------------------------------------
    vcx.simulate_keystrokes("cmd-1");
    let link = "Salt came by ".len();
    let at = in_text(&workspace, Position::new(1, link + 6), vcx);
    right_click_at(at, vcx);
    let labels = open_menu("a link", vcx);
    assert_has(
        &labels,
        &["Open Link", "Edit Link", "Remove Link", "Copy Link Address"],
        "a link",
    );
    click_item("Remove Link", vcx);
    let linked = workspace.read_with(vcx, |w, cx| {
        let document = w.editor().read(cx).document();
        document.blocks()[1]
            .attrs_at(link + 6)
            .is_some_and(|attrs| attrs.link.is_some())
    });
    assert!(!linked, "Remove Link takes the whole link off");
    vcx.simulate_keystrokes("cmd-z");
    vcx.run_until_parked();

    // On commented text the app's "Go to Comment" is there, and goes there.
    let at = in_text(&workspace, Position::new(1, 2), vcx);
    right_click_at(at, vcx);
    let labels = open_menu("commented text", vcx);
    assert_has(&labels, &["Add Comment", "Go to Comment"], "commented text");
    click_item("Go to Comment", vcx);
    assert!(workspace.update_in(vcx, |w, window, cx| {
        w.comments().read(cx).has_focus(window, cx)
    }));
    let thread = workspace
        .read_with(vcx, |w, cx| w.comments().read(cx).active_thread())
        .expect("the thread is active");

    // ----- a comment thread ------------------------------------------------------
    let card: &'static str = Box::leak(format!("comment-thread-{}", thread.0).into_boxed_str());
    let card = bounds_of(card, vcx);
    right_click_at(point(card.center().x, card.top() + px(14.)), vcx);
    let labels = open_menu("a comment thread", vcx);
    assert_has(
        &labels,
        &["Reply", "Edit", "Resolve", "Copy Text", "Delete"],
        "a comment thread",
    );
    click_item("Resolve", vcx);
    let resolved = workspace.read_with(vcx, |w, cx| {
        w.comments()
            .read(cx)
            .thread(thread)
            .map(|entry| entry.thread.resolved)
    });
    assert_eq!(
        resolved,
        Some(true),
        "Resolve in the menu resolves the thread"
    );
    vcx.simulate_keystrokes("cmd-z");
    vcx.run_until_parked();

    // ----- a text field ----------------------------------------------------------
    // The find bar's query field, with a word in it.
    vcx.simulate_keystrokes("cmd-1 cmd-f");
    vcx.simulate_input("salt");
    vcx.run_until_parked();
    let field = bounds_of("find-query-frame", vcx);
    right_click_at(point(field.left() + px(20.), field.center().y), vcx);
    let labels = open_menu("a text field", vcx);
    assert_eq!(labels, ["Cut", "Copy", "Paste", "-", "Select All"]);
    click_item("Select All", vcx);
    // The whole query is selected: typing replaces it.
    vcx.simulate_input("road");
    vcx.run_until_parked();
    assert_eq!(
        workspace.read_with(vcx, |w, cx| w.tools().read(cx).find_query().to_string()),
        "road"
    );
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();

    // ----- the title -------------------------------------------------------------
    let title = bounds_of(MenuKind::Title.selector(), vcx);
    right_click_at(title.center(), vcx);
    let labels = open_menu("the title", vcx);
    assert_eq!(
        labels,
        [
            "Rename…",
            "Export…",
            "Version History…",
            "Document Settings…",
            "-",
            "Reveal in Library"
        ]
    );
    click_item("Version History…", vcx);
    assert!(workspace.read_with(vcx, |w, cx| {
        w.modal_layer().read(cx).is_open::<VersionHistory>()
    }));
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();

    // ----- an item of the status bar ---------------------------------------------
    let counts = bounds_of(MenuKind::Statistics.selector(), vcx);
    right_click_at(counts.center(), vcx);
    let labels = open_menu("the counts in the status bar", vcx);
    assert_has(
        &labels,
        &[
            "Word Count",
            "Character Count",
            "Reading Time",
            "Character Limit",
        ],
        "the counts in the status bar",
    );
    click_item("Reading Time", vcx);
    assert!(!workspace.read_with(vcx, |w, _| w.settings().status_reading_time));
    assert_eq!(workspace.update(vcx, |w, cx| w.status_line(cx)).reading, "");

    let save = bounds_of(MenuKind::Save.selector(), vcx);
    right_click_at(save.center(), vcx);
    assert_eq!(
        open_menu("the save state", vcx),
        ["Save Now", "Version History…"]
    );
    dismiss(vcx);
}

/// `shift-f10` opens the same menus as `ctrl-enter`, wherever that is
/// bound, and the shortcuts overlay lists both keys.
#[gpui::test]
fn shift_f10_is_a_second_key_for_the_context_menu(cx: &mut TestAppContext) {
    let (fixture, vcx) = fixture(cx);
    let Fixture {
        workspace, chapter, ..
    } = fixture;

    // In the text.
    vcx.simulate_keystrokes("cmd-1");
    workspace.update(vcx, |w, cx| {
        w.editor().update(cx, |editor, cx| {
            editor.edit(|state| state.set_caret(Position::new(2, 3)), cx);
        });
    });
    for key in ["ctrl-enter", "shift-f10"] {
        vcx.simulate_keystrokes(key);
        vcx.run_until_parked();
        let labels = menu(vcx).unwrap_or_else(|| panic!("{key} opens the text menu"));
        assert_has(&labels, &["Paste", "Select All", "Add Comment"], key);
        // The same key closes it again.
        vcx.simulate_keystrokes(key);
        vcx.run_until_parked();
        assert_eq!(menu(vcx), None, "{key} pressed again closes the menu");
        assert!(editor_focused(&workspace, vcx));
    }

    // With Vim on, in Normal mode: the key is no Vim command.
    vcx.simulate_keystrokes("cmd-alt-v");
    vcx.run_until_parked();
    for key in ["ctrl-enter", "shift-f10"] {
        vcx.simulate_keystrokes(key);
        vcx.run_until_parked();
        assert!(menu(vcx).is_some(), "{key} opens the text menu with Vim on");
        dismiss(vcx);
    }
    vcx.simulate_keystrokes("cmd-alt-v");
    vcx.run_until_parked();

    // On a library row.
    workspace.update(vcx, |w, cx| {
        w.panel().update(cx, |panel, cx| {
            panel.reveal(LibraryItem::Document(chapter), cx);
        });
    });
    vcx.simulate_keystrokes("cmd-0 shift-f10");
    vcx.run_until_parked();
    let labels = menu(vcx).expect("shift-f10 opens the menu of the selected row");
    assert_has(&labels, &["Rename", "Export…"], "the selected row");
    dismiss(vcx);

    // On a comment thread.
    vcx.simulate_keystrokes("cmd-2 shift-f10");
    vcx.run_until_parked();
    let labels = menu(vcx).expect("shift-f10 opens the menu of the active thread");
    assert_has(&labels, &["Reply", "Resolve"], "the active thread");
    dismiss(vcx);

    // In a text field (the library's filter).
    vcx.simulate_keystrokes("cmd-0 /");
    vcx.simulate_input("chap");
    vcx.simulate_keystrokes("shift-f10");
    vcx.run_until_parked();
    assert_eq!(
        menu(vcx).expect("shift-f10 opens the menu of a text field"),
        ["Cut", "Copy", "Paste", "-", "Select All"]
    );
    dismiss(vcx);

    // The list of shortcuts has both keys on the row of the context menu,
    // in every place it opens in, and shows the one macOS uses.
    let rows: Vec<(String, String, String, Vec<String>)> = vcx.update(|_, cx| {
        collect_shortcuts(cx)
            .iter()
            .filter(|shortcut| shortcut.title.to_lowercase().contains("context menu"))
            .map(|shortcut| {
                (
                    shortcut.place.label().to_string(),
                    shortcut.title.to_string(),
                    shortcut.keys.clone(),
                    shortcut
                        .all_keys
                        .iter()
                        .map(|key| key.keys.to_string())
                        .collect(),
                )
            })
            .collect()
    });
    let places: Vec<&str> = rows.iter().map(|(place, ..)| place.as_str()).collect();
    assert_eq!(
        places,
        [
            "In the text",
            "In the comment list",
            "In the library",
            "In keyboard shortcuts",
            "In a text field"
        ],
        "{rows:?}"
    );
    for (_, _, keys, all) in &rows {
        assert_eq!(
            keys, "\u{2303}\u{21a9}",
            "ctrl-enter is the key shown: {rows:?}"
        );
        assert_eq!(all, &["shift-f10", "ctrl-enter"], "{rows:?}");
    }
}

fn press(at: Point<Pixels>, cx: &mut VisualTestContext) {
    cx.simulate_event(MouseDownEvent {
        position: at,
        modifiers: Modifiers::default(),
        button: MouseButton::Left,
        click_count: 1,
        first_mouse: false,
    });
}

fn release(at: Point<Pixels>, cx: &mut VisualTestContext) {
    cx.simulate_event(MouseUpEvent {
        position: at,
        modifiers: Modifiers::default(),
        button: MouseButton::Left,
        click_count: 1,
    });
    cx.run_until_parked();
}

/// Selects `range` of block `block` by dragging over it with the left
/// button, and waits for as long as the format bar takes to come up.
fn drag_over(
    workspace: &Entity<Workspace>,
    block: usize,
    range: std::ops::Range<usize>,
    cx: &mut VisualTestContext,
) {
    let from = in_text(workspace, Position::new(block, range.start), cx);
    let to = in_text(workspace, Position::new(block, range.end), cx);
    let to = point(to.x - px(3.), to.y);
    cx.simulate_mouse_move(from, None, Modifiers::default());
    press(from, cx);
    let middle = point((from.x + to.x) / 2., (from.y + to.y) / 2.);
    cx.simulate_mouse_move(middle, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(to, MouseButton::Left, Modifiers::default());
    release(to, cx);
    cx.executor().advance_clock(FORMAT_BAR_DELAY);
    cx.run_until_parked();
}

fn bar_visible(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> bool {
    workspace.read_with(cx, |w, cx| w.tools().read(cx).format_bar_visible())
}

/// A selection made with the mouse brings up the format bar; its buttons
/// format the selection; the app's "Add Comment" is on it; the switch in
/// the settings turns it off, at once and for the next launch.
#[gpui::test]
fn the_format_bar_comes_up_over_a_mouse_selection(cx: &mut TestAppContext) {
    let (fixture, vcx) = fixture(cx);
    let workspace = fixture.workspace;
    let library = workspace.read_with(vcx, |w, _| w.library().clone());
    assert!(workspace.read_with(vcx, |w, _| w.settings().format_bar));
    vcx.simulate_keystrokes("cmd-1");

    // A selection from the keyboard brings up nothing.
    vcx.simulate_keystrokes("shift-right shift-right");
    vcx.executor().advance_clock(FORMAT_BAR_DELAY * 2);
    vcx.run_until_parked();
    assert!(!bar_visible(&workspace, vcx));

    // A drag over "Nobody remembers" does.
    drag_over(&workspace, 2, 0..16, vcx);
    assert_eq!(selected_text(&workspace, vcx), "Nobody remembers");
    assert!(bar_visible(&workspace, vcx), "the bar is up after a drag");
    let bar = workspace
        .read_with(vcx, |w, cx| w.tools().read(cx).format_bar_bounds())
        .expect("the bar is drawn");
    assert!(vcx.debug_bounds("format-bar").is_some());
    // It never takes the keyboard.
    assert!(editor_focused(&workspace, vcx));

    // The buttons: the tools' own, then the app's "Add Comment" before
    // "more".
    let items = workspace.read_with(vcx, |w, cx| w.tools().read(cx).format_bar_items(cx));
    let ids: Vec<&str> = items.iter().map(|item| item.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "format-bar-bold",
            "format-bar-italic",
            "format-bar-underline",
            "format-bar-strikethrough",
            "format-bar-code",
            "format-bar-highlight",
            "format-bar-color",
            "format-bar-link",
            "format-bar-turn-into",
            "format-bar-extra-0",
            "format-bar-more",
        ]
    );
    let comment = &items[9];
    assert!(comment.host && comment.enabled, "{comment:?}");
    assert_eq!(comment.label, "Add Comment");
    // It shows the key of the action it runs (cmd-alt-m).
    assert_eq!(comment.key_hint.as_deref(), Some("\u{2325}\u{2318}M"));

    // A click on B makes the selection bold, and keeps selection and bar.
    let before = selection(&workspace, vcx);
    let bold = bounds_of("format-bar-bold", vcx);
    assert!(bar.contains(&bold.center()));
    vcx.simulate_mouse_move(bold.center(), None, Modifiers::default());
    vcx.simulate_click(bold.center(), Modifiers::default());
    vcx.run_until_parked();
    let is_bold = |cx: &mut VisualTestContext| {
        workspace.read_with(cx, |w, cx| {
            let block = &w.editor().read(cx).document().blocks()[2];
            (
                block.attrs_at(3).is_some_and(|attrs| attrs.bold),
                block.attrs_at(20).is_some_and(|attrs| attrs.bold),
            )
        })
    };
    assert_eq!(is_bold(vcx), (true, false), "only the selection is bold");
    assert_eq!(selection(&workspace, vcx), before);
    assert!(bar_visible(&workspace, vcx));
    assert!(editor_focused(&workspace, vcx));
    let state = workspace.read_with(vcx, |w, cx| {
        w.tools().read(cx).format_bar_items(cx)[0].state
    });
    assert_eq!(state, Mark::Check);
    // One undo step, like the key.
    vcx.simulate_keystrokes("cmd-z");
    vcx.run_until_parked();
    assert_eq!(is_bold(vcx), (false, false));

    // The app's button: the comment composer opens for the selection, and
    // the bar goes when the keyboard does.
    drag_over(&workspace, 2, 0..16, vcx);
    assert!(bar_visible(&workspace, vcx));
    let add_comment = bounds_of("format-bar-extra-0", vcx);
    vcx.simulate_mouse_move(add_comment.center(), None, Modifiers::default());
    vcx.simulate_click(add_comment.center(), Modifiers::default());
    vcx.run_until_parked();
    assert!(workspace.read_with(vcx, |w, cx| w.comments().read(cx).is_composing()));
    assert!(!bar_visible(&workspace, vcx));
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();

    // A press elsewhere in the window closes it: on the band above the
    // text, on the status bar.
    for selector in ["titlebar", "status-bar"] {
        vcx.simulate_keystrokes("cmd-1");
        drag_over(&workspace, 2, 0..16, vcx);
        assert!(bar_visible(&workspace, vcx));
        let bounds = bounds_of(selector, vcx);
        let at = point(bounds.left() + px(40.), bounds.center().y);
        vcx.simulate_mouse_move(at, None, Modifiers::default());
        vcx.simulate_click(at, Modifiers::default());
        vcx.run_until_parked();
        assert!(
            !bar_visible(&workspace, vcx),
            "a press on the {selector} closes the bar"
        );
        assert_eq!(selected_text(&workspace, vcx), "Nobody remembers");
    }

    // The setting. In the settings modal it is a switch under "Writing".
    vcx.simulate_keystrokes("cmd-1");
    drag_over(&workspace, 2, 0..16, vcx);
    assert!(bar_visible(&workspace, vcx));
    vcx.simulate_keystrokes("cmd-,");
    vcx.run_until_parked();
    let view = workspace
        .read_with(vcx, |w, cx| {
            w.modal_layer().read(cx).active::<SettingsView>()
        })
        .expect("the settings are open");
    let row = view.read_with(vcx, |view, cx| {
        view.rows(cx)
            .iter()
            .position(|row| *row == SettingRow::FormatBar)
    });
    let row = row.expect("the settings list the format bar");
    // A click on the row switches it off.
    let row: &'static str = Box::leak(format!("setting-{row}").into_boxed_str());
    let at = bounds_of(row, vcx).center();
    vcx.simulate_click(at, Modifiers::default());
    vcx.run_until_parked();
    assert_eq!(
        view.update_in(vcx, |view, window, cx| view.focused_row(window, cx)),
        Some(SettingRow::FormatBar)
    );
    assert!(!workspace.read_with(vcx, |w, _| w.settings().format_bar));
    assert!(!workspace.read_with(vcx, |w, cx| w.tools().read(cx).format_bar_enabled()));
    assert!(
        !bar_visible(&workspace, vcx),
        "turning it off closes the bar"
    );
    assert!(
        !library.read_with(vcx, |library, _| library.setting_or(keys::FORMAT_BAR, true)),
        "the choice is stored"
    );
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();

    // Off: a mouse selection brings up nothing.
    drag_over(&workspace, 2, 0..16, vcx);
    assert_eq!(selected_text(&workspace, vcx), "Nobody remembers");
    assert!(!bar_visible(&workspace, vcx));
    assert!(editor_focused(&workspace, vcx));

    // A window opened on the same library starts with it off.
    let (second, vcx) = open_workspace(&library, cx);
    assert!(!second.read_with(vcx, |w, cx| w.tools().read(cx).format_bar_enabled()));

    // And on again, live.
    second.update(vcx, |w, cx| w.write_setting(keys::FORMAT_BAR, &true, cx));
    vcx.run_until_parked();
    assert!(second.read_with(vcx, |w, cx| w.tools().read(cx).format_bar_enabled()));
}
