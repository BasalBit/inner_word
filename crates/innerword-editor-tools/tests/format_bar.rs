//! The floating format bar: it comes up by itself over a selection made
//! with the mouse, its buttons do what their keys do and keep the selection
//! and the bar, and it goes away as soon as the keyboard takes over.

use std::{cell::Cell, rc::Rc};

use gpui::{
    AppContext, Bounds, Entity, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, Pixels,
    Point, TestAppContext, VisualTestContext, point, px, size,
};
use innerword_core::{
    Block, BlockKind, Document, Highlight, InlineAttrs, Position, Selection, TextRange,
};
use innerword_editor::{
    CompactOptions, Editor, EditorConfig, PopupAnchor, PopupPlacement, actions as editor_actions,
};
use innerword_editor_tools::{
    EditorTools, FORMAT_BAR_BUTTONS, FORMAT_BAR_DELAY, FormatBarButton, FormatBarItem, Mark,
    TEXT_COLORS, ToolKind,
};
use innerword_ui::{ContextMenu, ContextMenuItem, IconName, KeyHint};

use crate::support::*;

const LONG: &str = "The quick brown fox jumps over the lazy dog and keeps running through \
    the wet grass until the river, where it stops to drink and to look at the far bank.";

fn document() -> Document {
    Document::from_blocks(vec![
        Block::paragraph("Hello brave new world"),
        Block::paragraph(LONG),
        Block::paragraph("Third paragraph here"),
    ])
}

/// A window point inside the character at a document position.
fn inside(
    editor: &Entity<Editor>,
    block: usize,
    offset: usize,
    cx: &mut VisualTestContext,
) -> Point<Pixels> {
    let bounds = editor.read_with(cx, |editor, _| {
        editor
            .position_bounds(Position::new(block, offset))
            .unwrap()
    });
    point(bounds.left() + px(2.), bounds.center().y)
}

fn press(at: Point<Pixels>, clicks: usize, modifiers: Modifiers, cx: &mut VisualTestContext) {
    cx.simulate_event(MouseDownEvent {
        position: at,
        modifiers,
        button: MouseButton::Left,
        click_count: clicks,
        first_mouse: false,
    });
}

fn release(at: Point<Pixels>, clicks: usize, modifiers: Modifiers, cx: &mut VisualTestContext) {
    cx.simulate_event(MouseUpEvent {
        position: at,
        modifiers,
        button: MouseButton::Left,
        click_count: clicks,
    });
    cx.run_until_parked();
}

/// A drag with the left button from one character to another, without the
/// wait for the bar.
fn drag_between(from: Point<Pixels>, to: Point<Pixels>, cx: &mut VisualTestContext) {
    cx.simulate_mouse_move(from, None, Modifiers::none());
    press(from, 1, Modifiers::none(), cx);
    let middle = point((from.x + to.x) / 2., (from.y + to.y) / 2.);
    cx.simulate_mouse_move(middle, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(to, MouseButton::Left, Modifiers::none());
    release(to, 1, Modifiers::none(), cx);
}

/// The bar appears a moment after the button is released.
fn wait(cx: &mut VisualTestContext) {
    cx.executor().advance_clock(FORMAT_BAR_DELAY);
    cx.run_until_parked();
}

/// Selects `range` of one block by dragging over it and waits for the bar.
fn drag(
    editor: &Entity<Editor>,
    block: usize,
    range: std::ops::Range<usize>,
    cx: &mut VisualTestContext,
) {
    let from = inside(editor, block, range.start, cx);
    let to = inside(editor, block, range.end, cx);
    // Past the middle of the character before the end, so the selection
    // ends at `range.end`.
    drag_between(from, point(to.x - px(3.), to.y), cx);
    wait(cx);
}

/// `count` clicks in a row on one spot (2: a word, 3: a block).
fn clicks(at: Point<Pixels>, count: usize, modifiers: Modifiers, cx: &mut VisualTestContext) {
    cx.simulate_mouse_move(at, None, modifiers);
    for click in 1..=count {
        press(at, click, modifiers, cx);
        release(at, click, modifiers, cx);
    }
}

fn visible(tools: &Entity<EditorTools>, cx: &mut VisualTestContext) -> bool {
    tools.read_with(cx, |tools, _| tools.format_bar_visible())
}

/// Where the bar is drawn.
#[track_caller]
fn bar(tools: &Entity<EditorTools>, cx: &mut VisualTestContext) -> Bounds<Pixels> {
    tools
        .read_with(cx, |tools, _| tools.format_bar_bounds())
        .expect("the format bar is drawn")
}

fn items(tools: &Entity<EditorTools>, cx: &mut VisualTestContext) -> Vec<FormatBarItem> {
    tools.read_with(cx, |tools, cx| tools.format_bar_items(cx))
}

#[track_caller]
fn item(tools: &Entity<EditorTools>, id: &str, cx: &mut VisualTestContext) -> FormatBarItem {
    items(tools, cx)
        .into_iter()
        .find(|item| item.id == id)
        .unwrap_or_else(|| panic!("no button {id}"))
}

fn state(tools: &Entity<EditorTools>, id: &str, cx: &mut VisualTestContext) -> Mark {
    item(tools, id, cx).state
}

/// A real click on a button of the bar.
#[track_caller]
fn click_button(id: &'static str, cx: &mut VisualTestContext) {
    let at = cx
        .debug_bounds(id)
        .unwrap_or_else(|| panic!("{id} was not drawn"))
        .center();
    cx.simulate_mouse_move(at, None, Modifiers::none());
    cx.simulate_click(at, Modifiers::none());
    cx.run_until_parked();
}

/// Chooses a row of the menu a button of the bar dropped.
#[track_caller]
fn choose(title: &str, cx: &mut VisualTestContext) {
    let chosen = cx.update(|window, cx| ContextMenu::choose(title, window, cx));
    assert!(chosen, "{title:?} cannot be chosen");
    cx.run_until_parked();
}

fn menu_labels(tools: &Entity<EditorTools>, cx: &mut VisualTestContext) -> Vec<String> {
    tools
        .read_with(cx, |tools, _| tools.format_bar_menu().cloned())
        .expect("a menu of the bar is open")
        .read_with(cx, |menu, _| menu.labels())
}

fn menu_item(
    tools: &Entity<EditorTools>,
    title: &str,
    cx: &mut VisualTestContext,
) -> ContextMenuItem {
    tools
        .read_with(cx, |tools, _| tools.format_bar_menu().cloned())
        .expect("a menu of the bar is open")
        .read_with(cx, |menu, _| {
            menu.items()
                .iter()
                .find(|item| item.title().as_ref() == title)
                .cloned()
                .unwrap_or_else(|| panic!("no item {title:?}"))
        })
}

fn attrs(
    editor: &Entity<Editor>,
    block: usize,
    offset: usize,
    cx: &mut VisualTestContext,
) -> InlineAttrs {
    editor.read_with(cx, |editor, _| {
        editor.document().blocks()[block]
            .attrs_at(offset)
            .cloned()
            .unwrap()
    })
}

fn kind(editor: &Entity<Editor>, block: usize, cx: &mut VisualTestContext) -> BlockKind {
    editor.read_with(cx, |editor, _| {
        editor.document().blocks()[block].kind().clone()
    })
}

fn selection(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> TextRange {
    editor.read_with(cx, |editor, _| editor.selection().range())
}

fn viewport(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> Bounds<Pixels> {
    editor.read_with(cx, |editor, _| editor.viewport_bounds().unwrap())
}

/// The rows of the selection on screen.
fn selection_rows(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> Vec<Bounds<Pixels>> {
    editor.read_with(cx, |editor, _| {
        editor.range_bounds(editor.selection().range()).unwrap()
    })
}

/// A popup of another crate.
struct Blank;

impl gpui::Render for Blank {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        _: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        gpui::div()
    }
}

fn brave() -> TextRange {
    TextRange::in_block(0, 6, 11)
}

// ---------------------------------------------------------------------------
// Appearing
// ---------------------------------------------------------------------------

#[gpui::test]
fn a_drag_shows_the_bar_after_the_button_is_released(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open_with_menus(document(), cx);
    assert!(tools.read_with(cx, |tools, _| tools.format_bar_enabled()));
    assert!(!visible(&tools, cx));
    assert!(items(&tools, cx).is_empty());

    let from = inside(&editor, 0, 6, cx);
    let to = inside(&editor, 0, 11, cx);
    let to = point(to.x - px(3.), to.y);
    cx.simulate_mouse_move(from, None, Modifiers::none());
    press(from, 1, Modifiers::none(), cx);
    cx.simulate_mouse_move(to, MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
    assert_eq!(selection(&editor, cx), brave());
    // Not while the button is down, however long.
    cx.executor().advance_clock(FORMAT_BAR_DELAY * 4);
    cx.run_until_parked();
    assert!(!visible(&tools, cx));

    release(to, 1, Modifiers::none(), cx);
    // Not at once either: a moment later.
    assert!(!visible(&tools, cx));
    assert_eq!(
        tools.read_with(cx, |tools, _| tools.format_bar_bounds()),
        None
    );
    wait(cx);
    assert!(visible(&tools, cx));
    // The bar is none of the tools that take the keyboard.
    assert_eq!(active(&tools, cx), None);
    assert!(editor_focused(&editor, cx));
    assert_eq!(selection(&editor, cx), brave());

    let ids: Vec<String> = items(&tools, cx).into_iter().map(|item| item.id).collect();
    assert_eq!(ids, FORMAT_BAR_BUTTONS);
    let bar = bar(&tools, cx);
    assert_eq!(cx.debug_bounds("format-bar"), Some(bar));
    for button in FORMAT_BAR_BUTTONS {
        let bounds = cx
            .debug_bounds(button)
            .unwrap_or_else(|| panic!("{button} was not drawn"));
        assert!(bar.contains(&bounds.center()), "{button} is on the bar");
    }
}

#[gpui::test]
fn double_triple_and_shift_clicks_show_the_bar(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open_with_menus(document(), cx);

    // A double click selects the word.
    let at = inside(&editor, 0, 8, cx);
    clicks(at, 2, Modifiers::none(), cx);
    assert_eq!(selection(&editor, cx), brave());
    assert!(!visible(&tools, cx));
    wait(cx);
    assert!(visible(&tools, cx));

    // The third click selects the block: the bar steps aside for the
    // press and comes back for the new selection.
    press(at, 3, Modifiers::none(), cx);
    cx.run_until_parked();
    assert!(!visible(&tools, cx));
    release(at, 3, Modifiers::none(), cx);
    wait(cx);
    assert!(visible(&tools, cx));
    assert_eq!(selection(&editor, cx), TextRange::in_block(0, 0, 21));

    // A triple click in one go: the bar of the double click on the way
    // never shows.
    let at = inside(&editor, 2, 3, cx);
    cx.simulate_mouse_move(at, None, Modifiers::none());
    for click in 1..=3 {
        press(at, click, Modifiers::none(), cx);
        release(at, click, Modifiers::none(), cx);
        assert!(!visible(&tools, cx), "after click {click}");
    }
    wait(cx);
    assert!(visible(&tools, cx));
    assert_eq!(selection(&editor, cx), TextRange::in_block(2, 0, 20));

    // A click puts the caret (no bar), a shift-click extends from it.
    clicks(inside(&editor, 0, 0, cx), 1, Modifiers::none(), cx);
    wait(cx);
    assert!(!visible(&tools, cx));
    let end = inside(&editor, 0, 5, cx);
    clicks(point(end.x - px(3.), end.y), 1, Modifiers::shift(), cx);
    assert_eq!(selection(&editor, cx), TextRange::in_block(0, 0, 5));
    wait(cx);
    assert!(visible(&tools, cx));
}

#[gpui::test]
fn a_keyboard_selection_does_not_show_the_bar(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open_with_menus(document(), cx);

    caret(&editor, 0, 6, cx);
    cx.simulate_keystrokes("shift-right shift-right alt-shift-right");
    assert!(!selection(&editor, cx).is_empty());
    wait(cx);
    assert!(!visible(&tools, cx));
    cx.simulate_keystrokes("cmd-a");
    wait(cx);
    assert!(!visible(&tools, cx));

    // Nor does a selection another crate makes (find, a comment thread).
    select(&editor, brave(), cx);
    wait(cx);
    assert!(!visible(&tools, cx));

    // A click that only places the caret shows nothing.
    clicks(inside(&editor, 1, 4, cx), 1, Modifiers::none(), cx);
    wait(cx);
    assert!(!visible(&tools, cx));
    assert!(selection(&editor, cx).is_empty());

    // Moving the pointer over a keyboard selection does not either.
    select(&editor, brave(), cx);
    let over = inside(&editor, 0, 8, cx);
    cx.simulate_mouse_move(over, None, Modifiers::none());
    wait(cx);
    assert!(!visible(&tools, cx));
}

#[gpui::test]
fn not_in_read_only_or_compact_editors(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open_with_menus(document(), cx);
    editor.update(cx, |editor, cx| editor.set_read_only(true, cx));
    cx.run_until_parked();
    drag(&editor, 0, 6..11, cx);
    assert_eq!(selection(&editor, cx), brave());
    assert!(!visible(&tools, cx));
    assert!(!tools.update_in(cx, |tools, window, cx| tools.show_format_bar(window, cx)));

    // Made editable again, the next mouse selection shows it.
    editor.update(cx, |editor, cx| editor.set_read_only(false, cx));
    cx.run_until_parked();
    drag(&editor, 0, 6..11, cx);
    assert!(visible(&tools, cx));
    // Read-only under a showing bar: the bar goes.
    editor.update(cx, |editor, cx| editor.set_read_only(true, cx));
    cx.run_until_parked();
    assert!(!visible(&tools, cx));
}

#[gpui::test]
fn a_compact_editor_has_no_bar(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, cx) = cx.add_window_view(|window, cx| {
        let editor = cx.new(|cx| {
            let mut editor = Editor::new(
                Document::new(),
                EditorConfig::compact(CompactOptions::default()),
                window,
                cx,
            );
            editor.set_plain_text("A comment being written", cx);
            editor
        });
        editor.read(cx).focus(window);
        EditorTools::new(editor, window, cx)
    });
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    let editor = tools.read_with(cx, |tools, _| tools.editor().clone());

    drag(&editor, 0, 2..9, cx);
    assert_eq!(selection(&editor, cx), TextRange::in_block(0, 2, 9));
    assert!(!visible(&tools, cx));
    assert!(!tools.update_in(cx, |tools, window, cx| tools.show_format_bar(window, cx)));
}

// ---------------------------------------------------------------------------
// The buttons
// ---------------------------------------------------------------------------

#[gpui::test]
fn the_toggles_do_what_their_keys_do_and_keep_selection_and_bar(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open_with_menus(document(), cx);
    drag(&editor, 0, 6..11, cx);

    type Has = fn(&InlineAttrs) -> bool;
    let toggles: [(&'static str, &str, &str, Has); 5] = [
        ("format-bar-bold", "Bold", "cmd-b", |a| a.bold),
        ("format-bar-italic", "Italic", "cmd-i", |a| a.italic),
        ("format-bar-underline", "Underline", "cmd-u", |a| {
            a.underline
        }),
        (
            "format-bar-strikethrough",
            "Strikethrough",
            "cmd-shift-x",
            |a| a.strikethrough,
        ),
        ("format-bar-code", "Inline code", "cmd-e", |a| a.code),
    ];
    for (id, label, key, has) in toggles {
        let before = item(&tools, id, cx);
        assert_eq!(before.label, label);
        assert_eq!(before.state, Mark::None, "{id}");
        assert!(before.enabled && !before.menu && !before.host);
        let hint = before.key_hint.expect("a key in the tooltip");

        click_button(id, cx);
        assert!(has(&attrs(&editor, 0, 8, cx)), "{id} applies");
        assert!(
            !has(&attrs(&editor, 0, 2, cx)),
            "{id} only to the selection"
        );
        assert_eq!(selection(&editor, cx), brave(), "{id} keeps the selection");
        assert!(visible(&tools, cx), "{id} keeps the bar");
        assert!(editor_focused(&editor, cx), "{id} leaves the keyboard");
        assert_eq!(state(&tools, id, cx), Mark::Check, "{id} shows its state");

        // One undo step, and the bar follows it at once.
        cx.simulate_keystrokes("cmd-z");
        assert!(!has(&attrs(&editor, 0, 8, cx)), "{id} is one undo step");
        assert!(visible(&tools, cx));
        assert_eq!(state(&tools, id, cx), Mark::None);

        // The key does the same, and the tooltip names that key.
        cx.simulate_keystrokes(key);
        assert!(has(&attrs(&editor, 0, 8, cx)), "{key}");
        assert!(visible(&tools, cx), "{key} keeps the bar");
        assert_eq!(state(&tools, id, cx), Mark::Check);
        assert_eq!(hint, KeyHint::parse(key).unwrap().text(), "{id}");

        // And the button takes it off again.
        click_button(id, cx);
        assert!(!has(&attrs(&editor, 0, 8, cx)), "{id} toggles off");
        assert_eq!(state(&tools, id, cx), Mark::None);
        assert!(visible(&tools, cx));
    }

    // Several in a row, each its own undo step.
    click_button("format-bar-bold", cx);
    click_button("format-bar-italic", cx);
    let now = attrs(&editor, 0, 8, cx);
    assert!(now.bold && now.italic);
    cx.simulate_keystrokes("cmd-z");
    let now = attrs(&editor, 0, 8, cx);
    assert!(now.bold && !now.italic);
    // Typing still replaces the selection (and the bar goes).
    cx.simulate_input("X");
    assert_eq!(text(&editor, cx).lines().next(), Some("Hello X new world"));
    assert!(!visible(&tools, cx));
}

#[gpui::test]
fn the_toggles_show_on_off_and_mixed(cx: &mut TestAppContext) {
    setup(cx);
    let bold = InlineAttrs {
        bold: true,
        ..InlineAttrs::default()
    };
    let document = Document::from_blocks(vec![
        Block::new(BlockKind::Paragraph)
            .with_span("plain ", InlineAttrs::default())
            .with_span("heavy", bold)
            .with_span(" plain", InlineAttrs::default()),
        Block::paragraph("Second"),
    ]);
    let (tools, editor, cx) = open_with_menus(document, cx);

    // All of the selection is bold.
    drag(&editor, 0, 6..11, cx);
    assert_eq!(state(&tools, "format-bar-bold", cx), Mark::Check);
    assert_eq!(state(&tools, "format-bar-italic", cx), Mark::None);
    // A part of it is.
    drag(&editor, 0, 2..9, cx);
    assert_eq!(selection(&editor, cx), TextRange::in_block(0, 2, 9));
    assert_eq!(state(&tools, "format-bar-bold", cx), Mark::Mixed);
    // None of it is.
    drag(&editor, 0, 0..5, cx);
    assert_eq!(state(&tools, "format-bar-bold", cx), Mark::None);

    // A mixed selection becomes bold all over, as with the key.
    drag(&editor, 0, 2..9, cx);
    click_button("format-bar-bold", cx);
    assert_eq!(state(&tools, "format-bar-bold", cx), Mark::Check);
    assert!(attrs(&editor, 0, 3, cx).bold && attrs(&editor, 0, 8, cx).bold);
    assert!(visible(&tools, cx));
}

#[gpui::test]
fn highlight_and_text_colour_open_their_choices(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open_with_menus(document(), cx);
    drag(&editor, 0, 6..11, cx);

    let highlight = item(&tools, "format-bar-highlight", cx);
    assert!(highlight.menu && highlight.enabled);
    assert_eq!(highlight.state, Mark::None);
    assert_eq!(highlight.detail, None);

    click_button("format-bar-highlight", cx);
    assert_eq!(
        menu_labels(&tools, cx),
        [
            "Yellow", "Green", "Blue", "Pink", "Orange", "Purple", "-", "None"
        ]
    );
    // The menu drops from its button and leaves the bar where it is.
    assert!(visible(&tools, cx));
    assert!(editor_focused(&editor, cx));
    let button = cx.debug_bounds("format-bar-highlight").unwrap();
    let menu = tools
        .read_with(cx, |tools, _| tools.format_bar_menu().cloned())
        .unwrap();
    let panel = menu.read_with(cx, |menu, _| menu.bounds()).unwrap();
    assert!(panel.top() >= button.bottom());
    assert!((panel.left() - button.left()).abs() < px(1.));
    // Each colour shows itself; the state of the selection is checked.
    assert!(menu_item(&tools, "Green", cx).swatch_color().is_some());
    assert_eq!(menu_item(&tools, "Green", cx).is_checked(), Some(false));
    assert_eq!(menu_item(&tools, "None", cx).is_checked(), Some(true));
    assert!(
        menu_item(&tools, "Yellow", cx)
            .explicit_key_hint()
            .is_some()
    );

    choose("Green", cx);
    assert_eq!(attrs(&editor, 0, 8, cx).highlight, Some(Highlight::Green));
    assert_eq!(attrs(&editor, 0, 2, cx).highlight, None);
    assert_eq!(selection(&editor, cx), brave());
    assert!(visible(&tools, cx));
    assert!(tools.read_with(cx, |tools, _| tools.format_bar_menu().is_none()));
    let highlight = item(&tools, "format-bar-highlight", cx);
    assert_eq!(highlight.state, Mark::Check);
    assert_eq!(highlight.detail.as_deref(), Some("Green"));

    // One undo step.
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(attrs(&editor, 0, 8, cx).highlight, None);
    assert_eq!(state(&tools, "format-bar-highlight", cx), Mark::None);
    cx.simulate_keystrokes("cmd-shift-z");

    // A real click on a row of the menu.
    click_button("format-bar-highlight", cx);
    assert_eq!(menu_item(&tools, "Green", cx).is_checked(), Some(true));
    let none = tools
        .read_with(cx, |tools, _| tools.format_bar_menu().cloned())
        .unwrap()
        .read_with(cx, |menu, _| menu.item_bounds(7))
        .expect("the row is drawn");
    cx.simulate_mouse_move(none.center(), None, Modifiers::none());
    cx.simulate_click(none.center(), Modifiers::none());
    cx.run_until_parked();
    assert_eq!(attrs(&editor, 0, 8, cx).highlight, None);
    assert_eq!(selection(&editor, cx), brave());
    assert!(visible(&tools, cx));
    assert!(editor_focused(&editor, cx));

    // Text colour.
    let colour = item(&tools, "format-bar-color", cx);
    assert!(colour.menu);
    assert_eq!(colour.label, "Text colour");
    click_button("format-bar-color", cx);
    let mut expected = vec!["Default".to_string()];
    expected.extend(TEXT_COLORS.iter().map(|(name, _)| name.to_string()));
    assert_eq!(menu_labels(&tools, cx), expected);
    assert_eq!(menu_item(&tools, "Default", cx).is_checked(), Some(true));
    assert!(menu_item(&tools, "Blue", cx).swatch_color().is_some());
    choose("Blue", cx);
    let blue = TEXT_COLORS
        .iter()
        .find(|(name, _)| *name == "Blue")
        .unwrap()
        .1;
    assert_eq!(attrs(&editor, 0, 8, cx).color, Some(blue));
    assert!(visible(&tools, cx));
    assert_eq!(selection(&editor, cx), brave());
    let colour = item(&tools, "format-bar-color", cx);
    assert_eq!(colour.state, Mark::Check);
    assert_eq!(colour.detail.as_deref(), Some("Blue"));
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(attrs(&editor, 0, 8, cx).color, None);
}

#[gpui::test]
fn the_menu_of_a_button_closes_without_closing_the_bar(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open_with_menus(document(), cx);
    drag(&editor, 0, 6..11, cx);
    let menu_open = |cx: &mut VisualTestContext| {
        tools.read_with(cx, |tools, _| tools.format_bar_menu().is_some())
    };

    // Escape closes the menu first, the bar with the next one.
    click_button("format-bar-turn-into", cx);
    assert!(menu_open(cx));
    cx.simulate_keystrokes("escape");
    assert!(!menu_open(cx));
    assert!(visible(&tools, cx));
    assert_eq!(selection(&editor, cx), brave());

    // A press outside the menu only closes the menu: in the text it does
    // not move the caret, on the button it does not open the menu again.
    click_button("format-bar-turn-into", cx);
    let elsewhere = inside(&editor, 2, 4, cx);
    cx.simulate_mouse_move(elsewhere, None, Modifiers::none());
    cx.simulate_click(elsewhere, Modifiers::none());
    cx.run_until_parked();
    assert!(!menu_open(cx));
    assert!(visible(&tools, cx));
    assert_eq!(selection(&editor, cx), brave());

    click_button("format-bar-highlight", cx);
    assert!(menu_open(cx));
    click_button("format-bar-highlight", cx);
    assert!(!menu_open(cx));
    assert!(visible(&tools, cx));

    // The keyboard drives the open menu, as it does every menu.
    click_button("format-bar-highlight", cx);
    cx.simulate_keystrokes("down down enter");
    assert_eq!(attrs(&editor, 0, 8, cx).highlight, Some(Highlight::Green));
    assert!(visible(&tools, cx));
    assert!(!menu_open(cx));

    // When the bar goes, its menu goes with it.
    click_button("format-bar-color", cx);
    assert!(menu_open(cx));
    tools.update(cx, |tools, cx| tools.dismiss_format_bar(cx));
    cx.run_until_parked();
    assert!(!menu_open(cx));
    assert!(!cx.update(|window, cx| ContextMenu::is_open(window, cx)));
}

#[gpui::test]
fn turn_into_opens_the_block_kinds(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open_with_menus(document(), cx);
    drag(&editor, 0, 6..11, cx);

    let turn = item(&tools, "format-bar-turn-into", cx);
    assert!(turn.menu);
    assert_eq!(turn.label, "Turn into");
    assert_eq!(turn.detail.as_deref(), Some("Paragraph"));

    click_button("format-bar-turn-into", cx);
    assert_eq!(
        menu_labels(&tools, cx),
        [
            "Paragraph",
            "Heading 1",
            "Heading 2",
            "Heading 3",
            "Heading 4",
            "Heading 5",
            "Heading 6",
            "Bullet List",
            "Numbered List",
            "To-do List",
            "Quote",
            "Code Block",
        ]
    );
    assert_eq!(menu_item(&tools, "Paragraph", cx).is_checked(), Some(true));
    assert_eq!(menu_item(&tools, "Heading 2", cx).is_checked(), Some(false));
    assert!(
        menu_item(&tools, "Heading 2", cx)
            .explicit_key_hint()
            .is_some()
    );

    choose("Heading 2", cx);
    assert_eq!(kind(&editor, 0, cx), BlockKind::heading(2));
    assert_eq!(kind(&editor, 1, cx), BlockKind::Paragraph);
    assert_eq!(selection(&editor, cx), brave());
    assert!(visible(&tools, cx));
    assert!(editor_focused(&editor, cx));
    assert_eq!(
        item(&tools, "format-bar-turn-into", cx).detail.as_deref(),
        Some("Heading 2")
    );
    // One undo step; the bar follows.
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(kind(&editor, 0, cx), BlockKind::Paragraph);
    assert_eq!(
        item(&tools, "format-bar-turn-into", cx).detail.as_deref(),
        Some("Paragraph")
    );

    // The rest of the kinds, one after the other on the same selection.
    let kinds: [(&str, BlockKind, &str); 5] = [
        ("Bullet List", BlockKind::BulletItem, "Bullet list"),
        ("Numbered List", BlockKind::NumberedItem, "Numbered list"),
        ("To-do List", BlockKind::todo(), "To-do list"),
        ("Quote", BlockKind::Quote, "Quote"),
        ("Code Block", BlockKind::code(), "Code block"),
    ];
    for (row, expected, name) in kinds {
        click_button("format-bar-turn-into", cx);
        choose(row, cx);
        assert!(kind(&editor, 0, cx).same_type(&expected), "{row}");
        assert!(visible(&tools, cx), "{row} keeps the bar");
        assert!(!selection(&editor, cx).is_empty(), "{row}");
        assert_eq!(
            item(&tools, "format-bar-turn-into", cx).detail.as_deref(),
            Some(name)
        );
    }
    // In a code block the inline buttons cannot apply.
    assert!(!item(&tools, "format-bar-bold", cx).enabled);
    assert!(!item(&tools, "format-bar-highlight", cx).enabled);
    assert!(!item(&tools, "format-bar-link", cx).enabled);
    assert!(item(&tools, "format-bar-turn-into", cx).enabled);
    assert!(item(&tools, "format-bar-more", cx).enabled);
    let before = text(&editor, cx);
    click_button("format-bar-bold", cx);
    assert!(!attrs(&editor, 0, 8, cx).bold);
    assert_eq!(text(&editor, cx), before);
    assert!(!tools.update_in(cx, |tools, window, cx| {
        tools.press_format_bar_button("format-bar-bold", window, cx)
    }));
    click_button("format-bar-turn-into", cx);
    choose("Paragraph", cx);
    assert_eq!(kind(&editor, 0, cx), BlockKind::Paragraph);
    assert!(item(&tools, "format-bar-bold", cx).enabled);

    // A selection over blocks of different kinds.
    editor.update(cx, |editor, cx| {
        editor.edit(
            |state| {
                state.set_caret(Position::new(2, 0));
                state.set_block_kind(BlockKind::Quote)
            },
            cx,
        );
    });
    cx.run_until_parked();
    let from = inside(&editor, 0, 6, cx);
    let to = inside(&editor, 2, 5, cx);
    drag_between(from, to, cx);
    wait(cx);
    assert!(visible(&tools, cx));
    assert_eq!(
        item(&tools, "format-bar-turn-into", cx).detail.as_deref(),
        Some("Mixed")
    );
    click_button("format-bar-turn-into", cx);
    choose("Heading 3", cx);
    for block in 0..3 {
        assert_eq!(kind(&editor, block, cx), BlockKind::heading(3));
    }
    assert!(visible(&tools, cx));
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(kind(&editor, 0, cx), BlockKind::Paragraph);
    assert_eq!(kind(&editor, 2, cx), BlockKind::Quote);
}

#[gpui::test]
fn link_and_more_open_their_tools_in_the_bars_place(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open_with_menus(document(), cx);

    drag(&editor, 0, 6..11, cx);
    let link = item(&tools, "format-bar-link", cx);
    assert_eq!(link.label, "Link");
    assert!(link.key_hint.is_some() && !link.menu);
    assert_eq!(link.state, Mark::None);
    click_button("format-bar-link", cx);
    assert_eq!(active(&tools, cx), Some(ToolKind::Link));
    assert!(!visible(&tools, cx));
    cx.simulate_input("example.com");
    cx.simulate_keystrokes("enter");
    assert_eq!(
        attrs(&editor, 0, 8, cx).link.as_deref(),
        Some("https://example.com")
    );
    assert_eq!(active(&tools, cx), None);
    assert!(editor_focused(&editor, cx));
    // The bar is back with the next mouse selection, and shows the link.
    wait(cx);
    assert!(!visible(&tools, cx));
    drag(&editor, 0, 6..11, cx);
    assert_eq!(state(&tools, "format-bar-link", cx), Mark::Check);
    drag(&editor, 0, 2..9, cx);
    assert_eq!(state(&tools, "format-bar-link", cx), Mark::Mixed);

    let more = item(&tools, "format-bar-more", cx);
    assert_eq!(more.label, "More formatting");
    assert!(more.key_hint.is_some());
    click_button("format-bar-more", cx);
    assert_eq!(active(&tools, cx), Some(ToolKind::Format));
    assert!(!visible(&tools, cx));
    assert_eq!(selection(&editor, cx), TextRange::in_block(0, 2, 9));
    // It is the full format menu, with its filter.
    cx.simulate_input("strike");
    assert_eq!(selected(&tools, cx).as_deref(), Some("Strikethrough"));
    cx.simulate_keystrokes("enter");
    assert!(attrs(&editor, 0, 8, cx).strikethrough);
    assert_eq!(active(&tools, cx), None);
    assert!(editor_focused(&editor, cx));
}

#[gpui::test]
fn the_hosts_buttons_appear_and_run(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open_with_menus(document(), cx);
    let ran = Rc::new(Cell::new(0));
    let asked = Rc::new(Cell::new(0));
    tools.update(cx, |tools, _| {
        let (ran, asked) = (ran.clone(), asked.clone());
        let weak = tools.editor().downgrade();
        tools.set_format_bar_extension(move |_, cx| {
            asked.set(asked.get() + 1);
            // The host may read the editor (and the tools) here.
            let selected = weak
                .upgrade()
                .is_some_and(|editor| !editor.read(cx).selection().is_collapsed());
            let count = ran.get();
            let ran = ran.clone();
            vec![
                // An action of the text: it arrives as its key would.
                FormatBarButton::action("Select All", editor_actions::SelectAll)
                    .icon(IconName::Comment),
                FormatBarButton::new("Count", move |_, _| ran.set(ran.get() + 1))
                    .active(count > 0)
                    .disabled(!selected),
                FormatBarButton::new("Never", |_, _| panic!("a disabled button ran"))
                    .disabled(true),
            ]
        });
    });

    drag(&editor, 0, 6..11, cx);
    assert_eq!(asked.get(), 1);
    let ids: Vec<String> = items(&tools, cx).into_iter().map(|item| item.id).collect();
    // Between "turn into" and "more".
    assert_eq!(
        ids[8..],
        [
            "format-bar-turn-into",
            "format-bar-extra-0",
            "format-bar-extra-1",
            "format-bar-extra-2",
            "format-bar-more"
        ]
    );
    let first = item(&tools, "format-bar-extra-0", cx);
    assert_eq!(first.label, "Select All");
    assert!(first.host && first.enabled && !first.menu);
    // An action button shows the key of its action.
    assert_eq!(first.key_hint.as_deref(), Some("⌘A"));
    let count = item(&tools, "format-bar-extra-1", cx);
    assert_eq!(count.label, "Count");
    assert_eq!(count.state, Mark::None);
    assert!(count.enabled);
    assert!(!item(&tools, "format-bar-extra-2", cx).enabled);
    let bar = bar(&tools, cx);
    for id in [
        "format-bar-extra-0",
        "format-bar-extra-1",
        "format-bar-extra-2",
    ] {
        assert!(bar.contains(&cx.debug_bounds(id).unwrap().center()));
    }

    // A closure button runs, keeps selection and bar, and the host is
    // asked again for the state of its buttons.
    click_button("format-bar-extra-1", cx);
    assert_eq!(ran.get(), 1);
    assert_eq!(asked.get(), 2);
    assert!(visible(&tools, cx));
    assert_eq!(selection(&editor, cx), brave());
    assert!(editor_focused(&editor, cx));
    assert_eq!(state(&tools, "format-bar-extra-1", cx), Mark::Check);
    // A disabled one does not.
    click_button("format-bar-extra-2", cx);
    assert!(visible(&tools, cx));

    // The action button: the action ran in the text. (It changes the
    // selection, which is not the bar's any more.)
    click_button("format-bar-extra-0", cx);
    let all = editor.read_with(cx, |editor, _| {
        let document = editor.document();
        TextRange::new(Position::new(0, 0), document.block_range(2).end)
    });
    assert_eq!(selection(&editor, cx), all);
    assert!(!visible(&tools, cx));
    assert!(editor_focused(&editor, cx));

    // The same through the scripting entry point.
    drag(&editor, 0, 6..11, cx);
    assert!(tools.update_in(cx, |tools, window, cx| {
        tools.press_format_bar_button("format-bar-extra-1", window, cx)
    }));
    cx.run_until_parked();
    assert_eq!(ran.get(), 2);
    assert!(!tools.update_in(cx, |tools, window, cx| {
        tools.press_format_bar_button("format-bar-extra-2", window, cx)
            || tools.press_format_bar_button("format-bar-nothing", window, cx)
    }));
}

// ---------------------------------------------------------------------------
// Going away
// ---------------------------------------------------------------------------

#[gpui::test]
fn the_bar_goes_when_the_keyboard_takes_over(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open_with_menus(document(), cx);
    let show = |cx: &mut VisualTestContext| {
        drag(&editor, 0, 6..11, cx);
        assert!(visible(&tools, cx));
    };

    // The selection changes from the keyboard.
    show(cx);
    cx.simulate_keystrokes("shift-right");
    assert!(!visible(&tools, cx));
    assert_eq!(selection(&editor, cx), TextRange::in_block(0, 6, 12));
    // It does not come back by itself.
    wait(cx);
    assert!(!visible(&tools, cx));

    // The selection collapses from the keyboard.
    show(cx);
    cx.simulate_keystrokes("right");
    assert!(!visible(&tools, cx));
    assert!(selection(&editor, cx).is_empty());

    // The user types.
    show(cx);
    cx.simulate_input("z");
    assert!(!visible(&tools, cx));
    assert_eq!(text(&editor, cx).lines().next(), Some("Hello z new world"));
    cx.simulate_keystrokes("cmd-z");

    // Escape: the bar goes, the selection stays.
    show(cx);
    cx.simulate_keystrokes("escape");
    assert!(!visible(&tools, cx));
    assert_eq!(selection(&editor, cx), brave());
    assert!(editor_focused(&editor, cx));

    // Escape during the moment before the bar appears.
    let from = inside(&editor, 0, 6, cx);
    let to = inside(&editor, 0, 11, cx);
    drag_between(from, point(to.x - px(3.), to.y), cx);
    cx.simulate_keystrokes("escape");
    wait(cx);
    assert!(!visible(&tools, cx));

    // A key that changes the text but not the selection keeps it.
    show(cx);
    cx.simulate_keystrokes("cmd-b");
    assert!(visible(&tools, cx));
    assert_eq!(state(&tools, "format-bar-bold", cx), Mark::Check);
    cx.simulate_keystrokes("cmd-b");
}

#[gpui::test]
fn escape_closes_one_thing_at_a_time(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open_with_menus(document(), cx);

    // The find bar is open, the keyboard back in the text.
    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("brave");
    editor.update_in(cx, |editor, window, _| editor.focus(window));
    cx.run_until_parked();
    assert_eq!(active(&tools, cx), Some(ToolKind::Find));

    drag(&editor, 2, 0..5, cx);
    assert!(visible(&tools, cx));
    cx.simulate_keystrokes("escape");
    assert!(!visible(&tools, cx));
    assert_eq!(active(&tools, cx), Some(ToolKind::Find));
    cx.simulate_keystrokes("escape");
    assert_eq!(active(&tools, cx), None);
}

#[gpui::test]
fn the_bar_goes_on_a_press_elsewhere(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open_with_menus(document(), cx);

    // A click in the text collapses the selection.
    drag(&editor, 0, 6..11, cx);
    clicks(inside(&editor, 2, 4, cx), 1, Modifiers::none(), cx);
    assert!(!visible(&tools, cx));
    wait(cx);
    assert!(!visible(&tools, cx));
    assert!(selection(&editor, cx).is_empty());

    // A press on the find bar (above the editor), on a button that does
    // not take the keyboard: the bar goes at the press, whatever the
    // press does there.
    cx.simulate_keystrokes("cmd-f");
    editor.update_in(cx, |editor, window, _| editor.focus(window));
    cx.run_until_parked();
    drag(&editor, 0, 6..11, cx);
    assert!(visible(&tools, cx));
    let button = cx.debug_bounds("find-case").unwrap().center();
    cx.simulate_mouse_move(button, None, Modifiers::none());
    press(button, 1, Modifiers::none(), cx);
    cx.run_until_parked();
    assert!(!visible(&tools, cx));
    release(button, 1, Modifiers::none(), cx);
    wait(cx);
    assert!(!visible(&tools, cx));
    // The selection was not touched by that press.
    assert_eq!(selection(&editor, cx), brave());

    // A press on the bar itself, between two buttons, does nothing: not
    // to the bar, not to the text under it, not to the keyboard.
    drag(&editor, 1, 100..110, cx);
    assert!(visible(&tools, cx));
    let before = selection(&editor, cx);
    let bar = bar(&tools, cx);
    let edge = point(bar.left() + px(1.5), bar.center().y);
    cx.simulate_mouse_move(edge, None, Modifiers::none());
    cx.simulate_click(edge, Modifiers::none());
    cx.run_until_parked();
    assert!(visible(&tools, cx));
    assert_eq!(selection(&editor, cx), before);
    assert!(editor_focused(&editor, cx));
}

#[gpui::test]
fn a_press_that_leaves_the_selection_alone_keeps_the_bar(cx: &mut TestAppContext) {
    setup(cx);
    let document = Document::from_blocks(vec![
        Block::paragraph("First"),
        Block::paragraph("Second"),
        Block::paragraph("Hello brave new world"),
        Block::paragraph("Fourth"),
        Block::with_text(BlockKind::TodoItem { checked: false }, "Buy milk"),
    ]);
    let (tools, editor, cx) = open_with_menus(document, cx);
    drag(&editor, 2, 6..11, cx);
    assert!(visible(&tools, cx));
    let brave = TextRange::in_block(2, 6, 11);

    // The checkbox of a to-do item: the editor toggles it and leaves the
    // caret alone.
    let text_start = editor.read_with(cx, |editor, _| {
        editor.position_bounds(Position::new(4, 0)).unwrap()
    });
    let checkbox = point(text_start.left() - px(16.), text_start.center().y);
    assert!(!bar(&tools, cx).contains(&checkbox));
    cx.simulate_mouse_move(checkbox, None, Modifiers::none());
    cx.simulate_click(checkbox, Modifiers::none());
    cx.run_until_parked();
    assert_eq!(kind(&editor, 4, cx), BlockKind::TodoItem { checked: true });
    assert_eq!(selection(&editor, cx), brave);
    assert!(visible(&tools, cx));
    wait(cx);
    assert!(visible(&tools, cx));
}

#[gpui::test]
fn the_bar_goes_when_something_else_opens(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open_with_menus(document(), cx);
    let show = |cx: &mut VisualTestContext| {
        drag(&editor, 0, 6..11, cx);
        assert!(visible(&tools, cx));
    };

    // The popup tools, from their keys.
    for (key, tool) in [
        ("cmd-.", ToolKind::Format),
        ("cmd-k", ToolKind::Link),
        ("cmd-t", ToolKind::Font),
    ] {
        show(cx);
        cx.simulate_keystrokes(key);
        assert_eq!(active(&tools, cx), Some(tool));
        assert!(!visible(&tools, cx), "{key}");
        cx.simulate_keystrokes("escape");
        assert_eq!(active(&tools, cx), None);
        // Closing the tool does not bring the bar back.
        wait(cx);
        assert!(!visible(&tools, cx), "{key}");
        assert_eq!(selection(&editor, cx), brave());
    }

    // The find bar takes the keyboard.
    show(cx);
    cx.simulate_keystrokes("cmd-f");
    assert!(!visible(&tools, cx));
    cx.simulate_keystrokes("escape");

    // The context menu of the text: a right-click inside the selection.
    show(cx);
    let at = inside(&editor, 0, 8, cx);
    cx.simulate_mouse_move(at, None, Modifiers::none());
    cx.simulate_event(MouseDownEvent {
        position: at,
        modifiers: Modifiers::none(),
        button: MouseButton::Right,
        click_count: 1,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        position: at,
        modifiers: Modifiers::none(),
        button: MouseButton::Right,
        click_count: 1,
    });
    cx.run_until_parked();
    assert_eq!(active(&tools, cx), Some(ToolKind::ContextMenu));
    assert!(!visible(&tools, cx));
    assert_eq!(selection(&editor, cx), brave());
    cx.simulate_keystrokes("escape");
    wait(cx);
    assert!(!visible(&tools, cx));

    // ... and from the keyboard.
    show(cx);
    cx.simulate_keystrokes("ctrl-enter");
    assert_eq!(active(&tools, cx), Some(ToolKind::ContextMenu));
    assert!(!visible(&tools, cx));
    cx.simulate_keystrokes("escape");

    // A menu of someone else (the host's title menu, say).
    show(cx);
    cx.update(|window, cx| {
        ContextMenu::open(
            point(px(40.), px(40.)),
            vec![ContextMenuItem::new("Something", |_, _| {})],
            window,
            cx,
        );
    });
    cx.run_until_parked();
    assert!(!visible(&tools, cx));
    cx.simulate_keystrokes("escape");

    // A popup another crate shows through the editor.
    show(cx);
    let popup = cx.new(|_| Blank);
    editor.update(cx, |editor, cx| {
        editor.show_popup(popup.into(), PopupAnchor::Caret, PopupPlacement::Below, cx)
    });
    cx.run_until_parked();
    assert!(!visible(&tools, cx));
    editor.update(cx, |editor, cx| editor.hide_popup(cx));
    cx.run_until_parked();
    assert!(!visible(&tools, cx));
}

#[gpui::test]
fn the_bar_goes_with_the_window_the_keyboard_and_the_document(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open_with_menus(document(), cx);
    let show = |cx: &mut VisualTestContext| {
        drag(&editor, 0, 6..11, cx);
        assert!(visible(&tools, cx));
    };

    // The window is deactivated.
    show(cx);
    cx.deactivate_window();
    cx.run_until_parked();
    assert!(!visible(&tools, cx));
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    assert!(!visible(&tools, cx));
    assert_eq!(selection(&editor, cx), brave());

    // The keyboard leaves the text.
    show(cx);
    cx.update(|window, _| window.blur());
    cx.run_until_parked();
    assert!(!visible(&tools, cx));
    editor.update_in(cx, |editor, window, _| editor.focus(window));
    cx.run_until_parked();

    // Another document is loaded.
    show(cx);
    editor.update(cx, |editor, cx| {
        editor.set_document(paragraphs(&["Another document entirely"]), cx);
        editor.edit(
            |state| state.set_selection(Selection::from_range(TextRange::in_block(0, 0, 7))),
            cx,
        );
    });
    cx.run_until_parked();
    wait(cx);
    assert!(!visible(&tools, cx));
    // The next mouse selection brings it back.
    drag(&editor, 0, 8..16, cx);
    assert!(visible(&tools, cx));

    // `close` closes it with everything else.
    tools.update_in(cx, |tools, window, cx| tools.close(window, cx));
    cx.run_until_parked();
    assert!(!visible(&tools, cx));
}

#[gpui::test]
fn the_switch_turns_the_bar_off_and_on(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open_with_menus(document(), cx);
    let enabled =
        |cx: &mut VisualTestContext| tools.read_with(cx, |tools, _| tools.format_bar_enabled());
    assert!(enabled(cx));

    // Off under a showing bar: it goes.
    drag(&editor, 0, 6..11, cx);
    assert!(visible(&tools, cx));
    tools.update(cx, |tools, cx| tools.set_format_bar_enabled(false, cx));
    cx.run_until_parked();
    assert!(!enabled(cx));
    assert!(!visible(&tools, cx));
    assert_eq!(selection(&editor, cx), brave());

    // Off: no mouse selection brings it up, and nothing else does.
    drag(&editor, 0, 0..5, cx);
    assert!(!visible(&tools, cx));
    clicks(inside(&editor, 0, 8, cx), 2, Modifiers::none(), cx);
    wait(cx);
    assert!(!visible(&tools, cx));
    assert!(!tools.update_in(cx, |tools, window, cx| tools.show_format_bar(window, cx)));
    // The selection itself works as ever.
    assert_eq!(selection(&editor, cx), brave());

    // On again: the selection that is there does not show it, the next
    // mouse selection does.
    tools.update(cx, |tools, cx| tools.set_format_bar_enabled(true, cx));
    cx.run_until_parked();
    assert!(enabled(cx));
    wait(cx);
    assert!(!visible(&tools, cx));
    drag(&editor, 0, 0..5, cx);
    assert!(visible(&tools, cx));
}

#[gpui::test]
fn the_bar_can_be_shown_and_dismissed_by_the_host(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open_with_menus(document(), cx);
    // Not without a selection.
    assert!(!tools.update_in(cx, |tools, window, cx| tools.show_format_bar(window, cx)));
    select(&editor, brave(), cx);
    assert!(tools.update_in(cx, |tools, window, cx| tools.show_format_bar(window, cx)));
    cx.run_until_parked();
    assert!(visible(&tools, cx));
    assert!(tools.update_in(cx, |tools, window, cx| {
        tools.press_format_bar_button("format-bar-italic", window, cx)
    }));
    cx.run_until_parked();
    assert!(attrs(&editor, 0, 8, cx).italic);
    assert!(visible(&tools, cx));
    tools.update(cx, |tools, cx| tools.dismiss_format_bar(cx));
    cx.run_until_parked();
    assert!(!visible(&tools, cx));
    assert_eq!(selection(&editor, cx), brave());
    assert!(!tools.update_in(cx, |tools, window, cx| {
        tools.press_format_bar_button("format-bar-italic", window, cx)
    }));
}

// ---------------------------------------------------------------------------
// Where it sits
// ---------------------------------------------------------------------------

#[gpui::test]
fn the_bar_sits_over_the_selection_inside_the_viewport(cx: &mut TestAppContext) {
    setup(cx);
    let mut lines: Vec<String> = (0..60)
        .map(|n| format!("Paragraph number {n} of many"))
        .collect();
    lines[1] = LONG.repeat(3);
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    let (tools, editor, cx) = open_with_menus(paragraphs(&lines), cx);
    let view = viewport(&editor, cx);
    let inside_viewport = |bar: Bounds<Pixels>| {
        bar.left() >= view.left()
            && bar.right() <= view.right()
            && bar.top() >= view.top()
            && bar.bottom() <= view.bottom()
    };
    let clear_of =
        |bar: Bounds<Pixels>, rows: &[Bounds<Pixels>]| rows.iter().all(|row| !row.intersects(&bar));

    // In the middle of the text: over the first row, centred on it.
    drag(&editor, 5, 10..16, cx);
    let rows = selection_rows(&editor, cx);
    let at = bar(&tools, cx);
    assert!(inside_viewport(at));
    assert!(at.bottom() <= rows[0].top(), "over the selection");
    assert!(rows[0].top() - at.bottom() <= px(12.), "just over it");
    assert!(
        (at.center().x - rows[0].center().x).abs() <= px(1.),
        "centred"
    );
    assert!(clear_of(at, &rows));

    // In the first row of the document there is no room above: under it.
    drag(&editor, 0, 10..16, cx);
    let rows = selection_rows(&editor, cx);
    let at = bar(&tools, cx);
    assert!(inside_viewport(at));
    assert!(at.top() >= rows[0].bottom(), "under the selection");
    assert!(at.top() - rows[0].bottom() <= px(12.));
    assert!((at.center().x - rows[0].center().x).abs() <= px(1.));
    assert!(clear_of(at, &rows));

    // Several rows: over the first one, never on the selection. (The bar
    // of the selection above lies over the start of this one: a click
    // further down puts it away first.)
    clicks(inside(&editor, 12, 2, cx), 1, Modifiers::none(), cx);
    let from = inside(&editor, 1, 30, cx);
    let to = inside(&editor, 3, 8, cx);
    drag_between(from, to, cx);
    wait(cx);
    let rows = selection_rows(&editor, cx);
    assert!(rows.len() > 3);
    let at = bar(&tools, cx);
    assert!(inside_viewport(at));
    assert!(at.bottom() <= rows[0].top());
    assert!(clear_of(at, &rows));

    // A whole paragraph (triple click): centred on its first row.
    clicks(inside(&editor, 6, 3, cx), 3, Modifiers::none(), cx);
    wait(cx);
    let rows = selection_rows(&editor, cx);
    let at = bar(&tools, cx);
    assert!(at.bottom() <= rows[0].top());
    assert!(inside_viewport(at));
}

#[gpui::test]
fn the_bar_keeps_inside_a_narrow_editor(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open_with_menus(document(), cx);
    cx.simulate_resize(size(px(560.), px(400.)));
    cx.run_until_parked();
    let view = viewport(&editor, cx);
    assert!(view.size.width <= px(560.));

    // At the left edge of the text the bar cannot be centred: it stays
    // inside the viewport.
    drag(&editor, 0, 0..2, cx);
    let rows = selection_rows(&editor, cx);
    let at = bar(&tools, cx);
    assert!(at.left() >= view.left() + px(4.), "{at:?} in {view:?}");
    assert!(at.right() <= view.right());
    assert!(
        at.center().x > rows[0].center().x,
        "pushed in from the edge"
    );
    assert!(at.top() >= rows[0].bottom() || at.bottom() <= rows[0].top());

    // At the right edge.
    let end = editor.read_with(cx, |editor, _| {
        let rows = editor
            .range_bounds(TextRange::in_block(1, 0, LONG.len()))
            .unwrap();
        // The last character of the first row of the long paragraph.
        editor
            .position_at_point(point(rows[0].right() - px(4.), rows[0].center().y))
            .unwrap()
    });
    let from = inside(&editor, 1, end.offset - 2, cx);
    let to = inside(&editor, 1, end.offset, cx);
    drag_between(from, to, cx);
    wait(cx);
    assert!(!selection(&editor, cx).is_empty());
    let rows = selection_rows(&editor, cx);
    let at = bar(&tools, cx);
    assert!(at.right() <= view.right() - px(4.), "{at:?} in {view:?}");
    assert!(at.left() >= view.left());
    assert!(at.bottom() <= rows[0].top() || at.top() >= rows[0].bottom());
    let regular = at.size.width;
    assert!(regular < view.size.width);

    // The editor of the smallest window with the library beside it: the
    // bar is drawn tight and still fits, with room to its edges.
    clicks(inside(&editor, 2, 3, cx), 1, Modifiers::none(), cx);
    cx.simulate_resize(size(px(380.), px(400.)));
    cx.run_until_parked();
    let view = viewport(&editor, cx);
    assert!(view.size.width <= px(380.));
    tools.update(cx, |tools, _| {
        tools.set_format_bar_extension(|_, _| {
            vec![FormatBarButton::new("Comment", |_, _| {}).icon(IconName::Comment)]
        })
    });
    drag(&editor, 0, 0..2, cx);
    let at = bar(&tools, cx);
    assert!(at.size.width < regular, "tight: {at:?}");
    assert!(at.left() >= view.left() + px(4.), "{at:?} in {view:?}");
    assert!(at.right() <= view.right() - px(4.), "{at:?} in {view:?}");
    assert_eq!(items(&tools, cx).len(), FORMAT_BAR_BUTTONS.len() + 1);

    // Squeezed narrower than the bar, it leaves the editor sideways but
    // not the window, and keeps clear of the selection.
    clicks(inside(&editor, 2, 3, cx), 1, Modifiers::none(), cx);
    cx.simulate_resize(size(px(240.), px(400.)));
    cx.run_until_parked();
    drag(&editor, 0, 0..2, cx);
    let rows = selection_rows(&editor, cx);
    let at = bar(&tools, cx);
    assert!(at.left() >= px(0.), "{at:?}");
    assert!(at.top() >= rows[0].bottom() || at.bottom() <= rows[0].top());
}

#[gpui::test]
fn the_bar_follows_scrolling(cx: &mut TestAppContext) {
    setup(cx);
    let lines: Vec<String> = (0..80)
        .map(|n| format!("Paragraph number {n} of many"))
        .collect();
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    let (tools, editor, cx) = open_with_menus(paragraphs(&lines), cx);
    let view = viewport(&editor, cx);

    drag(&editor, 8, 10..16, cx);
    let before = bar(&tools, cx);
    let row = selection_rows(&editor, cx)[0];
    assert!(before.bottom() <= row.top());

    // The text moves, the bar moves with it in the same frame.
    editor.update(cx, |editor, cx| editor.scroll_by_pixels(px(60.), cx));
    cx.run_until_parked();
    assert!(visible(&tools, cx));
    let after = bar(&tools, cx);
    let moved = selection_rows(&editor, cx)[0];
    assert_eq!(moved.top(), row.top() - px(60.));
    assert_eq!(after.top(), before.top() - px(60.));
    assert_eq!(after.left(), before.left());

    // The row reaches the top edge: the bar goes under the selection
    // rather than out of the viewport.
    editor.update(cx, |editor, cx| {
        editor.scroll_by_pixels(moved.top() - view.top() - px(10.), cx)
    });
    cx.run_until_parked();
    let row = selection_rows(&editor, cx)[0];
    let at = bar(&tools, cx);
    assert!(at.top() >= view.top());
    assert!(
        at.top() >= row.bottom(),
        "under the selection: {at:?} {row:?}"
    );

    // Scrolled out of sight, the bar is not drawn; it has not gone away.
    editor.update(cx, |editor, cx| editor.scroll_by_pixels(px(400.), cx));
    cx.run_until_parked();
    assert!(visible(&tools, cx));
    assert_eq!(
        tools.read_with(cx, |tools, _| tools.format_bar_bounds()),
        None
    );
    editor.update(cx, |editor, cx| editor.scroll_by_pixels(px(-400.), cx));
    cx.run_until_parked();
    assert_eq!(bar(&tools, cx), at);

    // A selection that fills the viewport from top to bottom: the bar
    // keeps to the top edge, inside the viewport.
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.set_caret(Position::new(0, 0)), cx);
        editor.scroll_to_caret(innerword_editor::Autoscroll::Fit, cx);
    });
    cx.run_until_parked();
    let top = inside(&editor, 0, 3, cx);
    cx.simulate_mouse_move(top, None, Modifiers::none());
    press(top, 1, Modifiers::none(), cx);
    release(top, 1, Modifiers::none(), cx);
    let last = editor.read_with(cx, |editor, _| editor.document().block_count() - 1);
    editor.update(cx, |editor, cx| editor.scroll_by_pixels(px(900.), cx));
    cx.run_until_parked();
    let visible_range = editor.read_with(cx, |editor, _| editor.visible_range().unwrap());
    assert!(visible_range.start.block > 0 && visible_range.end.block < last);
    let below = inside(&editor, visible_range.end.block, 3, cx);
    clicks(below, 1, Modifiers::shift(), cx);
    editor.update(cx, |editor, cx| editor.scroll_by_pixels(px(-200.), cx));
    cx.run_until_parked();
    wait(cx);
    assert!(visible(&tools, cx));
    let at = bar(&tools, cx);
    assert!(at.top() >= view.top() && at.top() <= view.top() + px(12.));
    assert!(at.bottom() <= view.bottom());
}

#[gpui::test]
fn the_buttons_have_tooltips_with_their_keys(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open_with_menus(document(), cx);
    drag(&editor, 1, 100..110, cx);
    assert!(visible(&tools, cx));

    let expected = [
        ("format-bar-bold", "Bold", true),
        ("format-bar-italic", "Italic", true),
        ("format-bar-underline", "Underline", true),
        ("format-bar-strikethrough", "Strikethrough", true),
        ("format-bar-code", "Inline code", true),
        ("format-bar-highlight", "Highlight", true),
        ("format-bar-color", "Text colour", false),
        ("format-bar-link", "Link", true),
        ("format-bar-turn-into", "Turn into", false),
        ("format-bar-more", "More formatting", true),
    ];
    for (id, label, has_key) in expected {
        let item = item(&tools, id, cx);
        assert_eq!(item.label, label);
        assert_eq!(item.key_hint.is_some(), has_key, "{id}");
    }

    // Resting the pointer on a button shows its tooltip.
    let pointer = cx.debug_bounds("format-bar-italic").unwrap().center();
    cx.simulate_mouse_move(pointer, None, Modifiers::none());
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(600));
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let tip = cx.debug_bounds("tool-tip").expect("a tooltip");
    assert!((tip.origin.x - pointer.x).abs() < px(40.));
    assert!(visible(&tools, cx));
    assert!(editor_focused(&editor, cx));
}
