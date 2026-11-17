//! The context menu of the text: a right-click (or `ctrl-enter`) opens the
//! menu that fits what is under the pointer, and each item does what its
//! key does.

use std::{cell::RefCell, rc::Rc};

use gpui::{
    Entity, Keystroke, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, Pixels, Point,
    TestAppContext, VisualTestContext, point, px,
};
use innerword_core::{
    Alignment, Block, BlockKind, CommentId, Document, Highlight, InlineAttrs, Position, Selection,
    TextRange,
};
use innerword_editor::{ContextMenuRequest, Editor};
use innerword_editor_tools::{EditorTools, TEXT_COLORS, ToolKind};
use innerword_ui::{ContextMenu, ContextMenuItem, KeyHint};

use crate::support::*;

const HELLO: usize = 0;
const LINKED: usize = 1;
const TODO: usize = 2;
const DONE: usize = 3;
const CODE: usize = 4;
const DIVIDER: usize = 5;
const EMPTY: usize = 6;

fn document() -> Document {
    let link = InlineAttrs {
        link: Some("https://example.com/docs".into()),
        ..InlineAttrs::default()
    };
    Document::from_blocks(vec![
        Block::paragraph("Hello brave world"),
        Block::new(BlockKind::Paragraph)
            .with_span("see ", InlineAttrs::default())
            .with_span("the docs", link)
            .with_span(" now", InlineAttrs::default()),
        Block::with_text(BlockKind::TodoItem { checked: false }, "Buy milk"),
        Block::with_text(BlockKind::TodoItem { checked: true }, "Write draft"),
        Block::with_text(BlockKind::code(), "let x = 1;"),
        Block::new(BlockKind::Divider),
        Block::paragraph(""),
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

fn right_click_at(at: Point<Pixels>, cx: &mut VisualTestContext) {
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
}

fn right_click(editor: &Entity<Editor>, block: usize, offset: usize, cx: &mut VisualTestContext) {
    let at = inside(editor, block, offset, cx);
    right_click_at(at, cx);
}

fn menu(tools: &Entity<EditorTools>, cx: &mut VisualTestContext) -> Entity<ContextMenu> {
    tools
        .read_with(cx, |tools, _| tools.text_menu().cloned())
        .expect("the text menu is open")
}

fn labels(tools: &Entity<EditorTools>, cx: &mut VisualTestContext) -> Vec<String> {
    menu(tools, cx).read_with(cx, |menu, _| menu.labels())
}

/// The item titled `title`: a root item, or `"Submenu > Item"`.
fn item(tools: &Entity<EditorTools>, title: &str, cx: &mut VisualTestContext) -> ContextMenuItem {
    menu(tools, cx).read_with(cx, |menu, _| {
        let find = |items: &[ContextMenuItem], title: &str| {
            items
                .iter()
                .find(|item| item.title().as_ref() == title)
                .cloned()
                .unwrap_or_else(|| panic!("no item {title:?}"))
        };
        match title.split_once(" > ") {
            None => find(menu.items(), title),
            Some((parent, child)) => {
                let parent = find(menu.items(), parent);
                find(parent.submenu_items().expect("a submenu"), child)
            }
        }
    })
}

fn submenu_labels(
    tools: &Entity<EditorTools>,
    title: &str,
    cx: &mut VisualTestContext,
) -> Vec<String> {
    item(tools, title, cx)
        .submenu_items()
        .unwrap()
        .iter()
        .map(|item| item.title().to_string())
        .collect()
}

/// Chooses an item of the open menu as a click on it would.
#[track_caller]
fn choose(title: &str, cx: &mut VisualTestContext) {
    let chosen = cx.update(|window, cx| ContextMenu::choose(title, window, cx));
    assert!(chosen, "{title:?} cannot be chosen");
    cx.run_until_parked();
}

fn click_at(at: Point<Pixels>, cx: &mut VisualTestContext) {
    cx.simulate_mouse_move(at, None, Modifiers::none());
    cx.simulate_click(at, Modifiers::none());
    cx.run_until_parked();
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

const CLIPBOARD: [&str; 4] = ["Cut", "Copy", "Paste", "Paste as Plain Text"];
const SELECTION: [&str; 10] = [
    "Bold",
    "Italic",
    "Underline",
    "Strikethrough",
    "Inline Code",
    "Highlight",
    "Text Colour",
    "Link",
    "Font",
    "Clear Formatting",
];
const TAIL: [&str; 8] = [
    "Turn Into",
    "Align",
    "Text Size",
    "-",
    "Insert",
    "-",
    "Find in Document",
    "Select All",
];

fn expected(parts: &[&[&str]]) -> Vec<String> {
    parts
        .iter()
        .flat_map(|part| part.iter().map(|label| label.to_string()))
        .collect()
}

#[gpui::test]
fn a_right_click_where_nothing_is_selected(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open_with_menus(document(), cx);
    assert!(!tools.read_with(cx, |tools, _| tools.text_menu_open()));

    // On an empty paragraph the click only puts the caret there: no
    // formatting of a selection, and nothing to cut or copy.
    right_click(&editor, EMPTY, 0, cx);
    assert_eq!(active(&tools, cx), Some(ToolKind::ContextMenu));
    assert_eq!(labels(&tools, cx), expected(&[&CLIPBOARD, &["-"], &TAIL]));
    assert!(item(&tools, "Cut", cx).is_disabled());
    assert!(item(&tools, "Copy", cx).is_disabled());
    assert!(
        item(&tools, "Paste", cx).is_disabled(),
        "nothing on the clipboard"
    );
    assert!(!item(&tools, "Select All", cx).is_disabled());
    // The menu is drawn at the pointer and does not take the keyboard.
    let at = inside(&editor, EMPTY, 0, cx);
    let bounds = menu(&tools, cx).read_with(cx, |menu, _| menu.bounds().unwrap());
    assert!(
        (bounds.origin.x - at.x).abs() < px(1.) && (bounds.origin.y - at.y).abs() < px(1.),
        "{bounds:?} {at:?}"
    );
    assert!(editor_focused(&editor, cx));
    assert_eq!(
        tools.read_with(cx, |tools, _| tools.text_menu_request().cloned().map(
            |request| { (request.block, request.has_selection, request.from_keyboard) }
        )),
        Some((EMPTY, false, false))
    );

    // The submenus: every kind with the block's own checked; the
    // alignments; the sizes; what can be inserted.
    assert_eq!(
        submenu_labels(&tools, "Turn Into", cx),
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
            "Code Block"
        ]
    );
    assert_eq!(
        item(&tools, "Turn Into > Paragraph", cx).is_checked(),
        Some(true)
    );
    assert_eq!(
        item(&tools, "Turn Into > Quote", cx).is_checked(),
        Some(false)
    );
    assert_eq!(
        submenu_labels(&tools, "Align", cx),
        ["Left", "Centre", "Right", "Justify"]
    );
    assert_eq!(item(&tools, "Align > Left", cx).is_checked(), Some(true));
    assert_eq!(
        submenu_labels(&tools, "Text Size", cx),
        ["Larger", "Smaller", "Reset"]
    );
    assert_eq!(
        submenu_labels(&tools, "Insert", cx),
        ["Divider", "Page Break", "Image…", "Today's Date", "Link"]
    );
    // An editor nobody gave a place for pictures takes none.
    assert!(item(&tools, "Insert > Image…", cx).is_disabled());
    assert!(!item(&tools, "Insert > Divider", cx).is_disabled());

    // Turn Into, with the mouse: a click opens the submenu, a click on
    // its row turns the block.
    let turn_into = menu(&tools, cx).read_with(cx, |menu, _| {
        let index = menu
            .labels()
            .iter()
            .position(|label| label == "Turn Into")
            .unwrap();
        menu.item_bounds(index).unwrap().center()
    });
    click_at(turn_into, cx);
    let quote = menu(&tools, cx).read_with(cx, |menu, _| {
        assert!(menu.open_submenu().is_some());
        menu.submenu_item_bounds(10).unwrap().center()
    });
    click_at(quote, cx);
    assert_eq!(kind(&editor, EMPTY, cx), BlockKind::Quote);
    assert_eq!(active(&tools, cx), None, "choosing closes the menu");
    assert!(editor_focused(&editor, cx));

    // Align and size, and they are checked the next time.
    right_click(&editor, EMPTY, 0, cx);
    assert_eq!(
        item(&tools, "Turn Into > Quote", cx).is_checked(),
        Some(true)
    );
    choose("Centre", cx);
    right_click(&editor, EMPTY, 0, cx);
    assert_eq!(item(&tools, "Align > Centre", cx).is_checked(), Some(true));
    choose("Larger", cx);
    editor.read_with(cx, |editor, _| {
        let block = &editor.document().blocks()[EMPTY];
        assert_eq!(block.alignment(), Alignment::Center);
        assert!(block.text_size().is_some());
    });
    right_click(&editor, EMPTY, 0, cx);
    choose("Reset", cx);
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().blocks()[EMPTY].text_size(), None)
    });

    // Insert.
    right_click(&editor, EMPTY, 0, cx);
    choose("Today's Date", cx);
    let today = innerword_editor_tools::today_string();
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().blocks()[EMPTY].text(), today)
    });
    right_click(&editor, HELLO, 5, cx);
    choose("Divider", cx);
    assert_eq!(
        editor.read_with(cx, |editor, _| editor
            .document()
            .blocks()
            .iter()
            .filter(|block| block.kind() == &BlockKind::Divider)
            .count()),
        2
    );
    cx.simulate_keystrokes("cmd-z");

    // Find in Document and Select All.
    right_click(&editor, HELLO, 5, cx);
    choose("Find in Document", cx);
    assert_eq!(active(&tools, cx), Some(ToolKind::Find));
    cx.simulate_keystrokes("escape");
    right_click(&editor, HELLO, 5, cx);
    choose("Select All", cx);
    editor.read_with(cx, |editor, _| {
        let all = editor.selection().range();
        assert_eq!(all.start, Position::new(0, 0));
        assert_eq!(all.end.block, editor.document().block_count() - 1);
    });
}

#[gpui::test]
fn a_right_click_on_a_word_or_a_selection(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open_with_menus(document(), cx);

    // On a word: the editor selects it, and the menu is the one of a
    // selection.
    right_click(&editor, HELLO, 8, cx);
    assert_eq!(selection(&editor, cx), TextRange::in_block(HELLO, 6, 11));
    assert_eq!(
        labels(&tools, cx),
        expected(&[&CLIPBOARD, &["-"], &SELECTION, &["-"], &TAIL])
    );
    assert!(!item(&tools, "Cut", cx).is_disabled());
    assert!(!item(&tools, "Copy", cx).is_disabled());
    assert_eq!(item(&tools, "Bold", cx).is_checked(), Some(false));
    assert_eq!(
        item(&tools, "Bold", cx).action_name(),
        Some("editor::ToggleBold")
    );
    assert_eq!(
        submenu_labels(&tools, "Highlight", cx),
        [
            "Yellow", "Green", "Blue", "Pink", "Orange", "Purple", "None"
        ]
    );
    assert_eq!(
        item(&tools, "Highlight > None", cx).is_checked(),
        Some(true)
    );
    assert_eq!(
        submenu_labels(&tools, "Text Colour", cx),
        [
            "Default", "Red", "Orange", "Green", "Blue", "Purple", "Grey"
        ]
    );
    assert_eq!(
        item(&tools, "Text Colour > Default", cx).is_checked(),
        Some(true)
    );

    // Bold with the mouse: move to the row, click.
    let bold = menu(&tools, cx).read_with(cx, |menu, _| {
        let index = menu
            .labels()
            .iter()
            .position(|label| label == "Bold")
            .unwrap();
        menu.item_bounds(index).unwrap().center()
    });
    click_at(bold, cx);
    assert!(attrs(&editor, HELLO, 8, cx).bold);
    assert!(!attrs(&editor, HELLO, 3, cx).bold);
    assert_eq!(active(&tools, cx), None);
    assert_eq!(
        selection(&editor, cx),
        TextRange::in_block(HELLO, 6, 11),
        "the selection is kept"
    );

    // A right-click inside the selection keeps it; the state shows.
    right_click(&editor, HELLO, 9, cx);
    assert_eq!(selection(&editor, cx), TextRange::in_block(HELLO, 6, 11));
    assert_eq!(item(&tools, "Bold", cx).is_checked(), Some(true));
    assert_eq!(item(&tools, "Italic", cx).is_checked(), Some(false));
    choose("Italic", cx);
    for (title, set) in [
        (
            "Underline",
            (|attrs: &InlineAttrs| attrs.underline) as fn(&InlineAttrs) -> bool,
        ),
        ("Strikethrough", |attrs| attrs.strikethrough),
        ("Inline Code", |attrs| attrs.code),
    ] {
        right_click(&editor, HELLO, 9, cx);
        choose(title, cx);
        assert!(set(&attrs(&editor, HELLO, 8, cx)), "{title}");
    }
    let now = attrs(&editor, HELLO, 8, cx);
    assert!(now.bold && now.italic);

    // Highlight and text colour from their submenus; None and Default
    // take them off again.
    right_click(&editor, HELLO, 9, cx);
    choose("Pink", cx);
    assert_eq!(
        attrs(&editor, HELLO, 8, cx).highlight,
        Some(Highlight::Pink)
    );
    right_click(&editor, HELLO, 9, cx);
    assert_eq!(
        item(&tools, "Highlight > Pink", cx).is_checked(),
        Some(true)
    );
    assert_eq!(
        item(&tools, "Highlight > None", cx).is_checked(),
        Some(false)
    );
    choose("None", cx);
    assert_eq!(attrs(&editor, HELLO, 8, cx).highlight, None);
    right_click(&editor, HELLO, 9, cx);
    choose("Red", cx);
    assert_eq!(attrs(&editor, HELLO, 8, cx).color, Some(TEXT_COLORS[0].1));
    right_click(&editor, HELLO, 9, cx);
    assert_eq!(
        item(&tools, "Text Colour > Red", cx).is_checked(),
        Some(true)
    );
    choose("Default", cx);
    assert_eq!(attrs(&editor, HELLO, 8, cx).color, None);

    // Clear Formatting.
    right_click(&editor, HELLO, 9, cx);
    choose("Clear Formatting", cx);
    assert_eq!(attrs(&editor, HELLO, 8, cx), InlineAttrs::default());

    // Link and Font open their tools on the selection.
    right_click(&editor, HELLO, 9, cx);
    choose("Link", cx);
    assert_eq!(active(&tools, cx), Some(ToolKind::Link));
    cx.simulate_input("example.org");
    cx.simulate_keystrokes("enter");
    assert_eq!(
        attrs(&editor, HELLO, 8, cx).link.as_deref(),
        Some("https://example.org")
    );
    cx.update(|_, cx| innerword_editor_tools::set_font_families(["Georgia", "Menlo"], cx));
    select(&editor, TextRange::in_block(HELLO, 0, 5), cx);
    right_click(&editor, HELLO, 2, cx);
    choose("Font", cx);
    assert_eq!(active(&tools, cx), Some(ToolKind::Font));
    cx.simulate_input("menlo");
    cx.simulate_keystrokes("enter");
    assert_eq!(attrs(&editor, HELLO, 2, cx).font.as_deref(), Some("Menlo"));

    // Copy, Cut and Paste run the editor's own actions.
    right_click(&editor, HELLO, 2, cx);
    choose("Copy", cx);
    assert_eq!(
        cx.read_from_clipboard()
            .and_then(|item| item.text())
            .as_deref(),
        Some("Hello")
    );
    right_click(&editor, HELLO, 2, cx);
    assert!(!item(&tools, "Paste", cx).is_disabled());
    choose("Cut", cx);
    assert_eq!(text(&editor, cx).lines().next(), Some(" brave world"));
    right_click(&editor, EMPTY, 0, cx);
    choose("Paste as Plain Text", cx);
    editor.read_with(cx, |editor, _| {
        let block = &editor.document().blocks()[EMPTY];
        assert_eq!(block.text(), "Hello");
        assert_eq!(block.attrs_at(1).unwrap().font, None, "plain");
    });
    cx.simulate_keystrokes("cmd-z");
    right_click(&editor, EMPTY, 0, cx);
    choose("Paste", cx);
    editor.read_with(cx, |editor, _| {
        let block = &editor.document().blocks()[EMPTY];
        assert_eq!(block.text(), "Hello");
        assert_eq!(block.attrs_at(1).unwrap().font.as_deref(), Some("Menlo"));
    });
}

#[gpui::test]
fn a_right_click_on_a_link(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open_with_menus(document(), cx);
    let link = |cx: &mut VisualTestContext| attrs(&editor, LINKED, 6, cx).link;

    // "see [the docs] now": on the word "docs" of the link.
    right_click(&editor, LINKED, 9, cx);
    assert_eq!(selection(&editor, cx), TextRange::in_block(LINKED, 8, 12));
    let all = labels(&tools, cx);
    assert_eq!(
        all[..5],
        [
            "Open Link",
            "Edit Link",
            "Remove Link",
            "Copy Link Address",
            "-"
        ]
    );
    assert!(
        !all.iter().any(|label| label == "Link"),
        "Edit Link is the link row here"
    );
    assert_eq!(
        tools.read_with(cx, |tools, _| tools
            .text_menu_request()
            .unwrap()
            .link
            .clone()),
        Some("https://example.com/docs".to_string())
    );

    choose("Open Link", cx);
    assert_eq!(cx.opened_url().as_deref(), Some("https://example.com/docs"));

    right_click(&editor, LINKED, 9, cx);
    choose("Copy Link Address", cx);
    assert_eq!(
        cx.read_from_clipboard()
            .and_then(|item| item.text())
            .as_deref(),
        Some("https://example.com/docs")
    );
    assert_eq!(text(&editor, cx).lines().nth(1), Some("see the docs now"));

    // Edit Link: the popover for the whole link, not for the word that
    // was clicked.
    right_click(&editor, LINKED, 9, cx);
    choose("Edit Link", cx);
    assert_eq!(active(&tools, cx), Some(ToolKind::Link));
    assert_eq!(selection(&editor, cx), TextRange::in_block(LINKED, 4, 12));
    assert_eq!(
        tools
            .read_with(cx, |tools, cx| tools.link_field_text(cx))
            .as_deref(),
        Some("https://example.com/docs")
    );
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("https://example.com/manual");
    cx.simulate_keystrokes("enter");
    assert_eq!(link(cx).as_deref(), Some("https://example.com/manual"));
    assert_eq!(
        attrs(&editor, LINKED, 4, cx).link.as_deref(),
        Some("https://example.com/manual"),
        "the whole link changed"
    );

    // Remove Link takes the whole link off and keeps the text.
    right_click(&editor, LINKED, 9, cx);
    choose("Remove Link", cx);
    assert_eq!(link(cx), None);
    assert_eq!(attrs(&editor, LINKED, 4, cx).link, None);
    assert_eq!(text(&editor, cx).lines().nth(1), Some("see the docs now"));
    // One undo step brings it back.
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(link(cx).as_deref(), Some("https://example.com/manual"));

    // Beside the link there are no link rows, and "Link" is back.
    right_click(&editor, LINKED, 14, cx);
    let all = labels(&tools, cx);
    assert_eq!(all[0], "Cut");
    assert!(all.iter().any(|label| label == "Link"));
}

#[gpui::test]
fn a_right_click_on_a_to_do(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open_with_menus(document(), cx);

    right_click(&editor, TODO, 5, cx);
    let all = labels(&tools, cx);
    let size = all.iter().position(|label| label == "Text Size").unwrap();
    assert_eq!(all[size + 1], "Mark as Done", "{all:?}");
    assert_eq!(
        item(&tools, "Mark as Done", cx).action_name(),
        Some("editor::ToggleTodoChecked")
    );
    assert_eq!(
        item(&tools, "Turn Into > To-do List", cx).is_checked(),
        Some(true)
    );
    choose("Mark as Done", cx);
    assert_eq!(
        kind(&editor, TODO, cx),
        BlockKind::TodoItem { checked: true }
    );

    // A ticked item offers the way back.
    right_click(&editor, DONE, 5, cx);
    assert!(
        labels(&tools, cx)
            .iter()
            .any(|label| label == "Mark as Not Done")
    );
    choose("Mark as Not Done", cx);
    assert_eq!(
        kind(&editor, DONE, cx),
        BlockKind::TodoItem { checked: false }
    );

    // On the checkbox itself: the click does not tick it, the menu does.
    let checkbox = {
        let row = editor.read_with(cx, |editor, _| {
            editor.position_bounds(Position::new(TODO, 0)).unwrap()
        });
        point(row.left() - px(16.), row.center().y)
    };
    right_click_at(checkbox, cx);
    assert_eq!(
        kind(&editor, TODO, cx),
        BlockKind::TodoItem { checked: true }
    );
    let request = tools.read_with(cx, |tools, _| tools.text_menu_request().cloned().unwrap());
    assert_eq!(request.block, TODO);
    if request.on_checkbox {
        assert!(!request.has_selection);
    }
    choose("Mark as Not Done", cx);
    assert_eq!(
        kind(&editor, TODO, cx),
        BlockKind::TodoItem { checked: false }
    );

    // Another block has no such row; turning it into a to-do does.
    right_click(&editor, HELLO, 8, cx);
    assert!(
        !labels(&tools, cx)
            .iter()
            .any(|label| label.starts_with("Mark as"))
    );
    choose("To-do List", cx);
    assert_eq!(
        kind(&editor, HELLO, cx),
        BlockKind::TodoItem { checked: false }
    );
}

#[gpui::test]
fn the_host_adds_its_items_for_the_request(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open_with_menus(document(), cx);
    // A comment thread on "brave".
    let thread = editor.update(cx, |editor, cx| {
        editor.edit(
            |state| {
                state.set_selection(Selection::new(
                    Position::new(HELLO, 6),
                    Position::new(HELLO, 11),
                ));
                let id = state.add_comment("Really brave?", "me", 0).unwrap();
                state.set_caret(Position::new(HELLO, 0));
                id
            },
            cx,
        )
    });
    cx.run_until_parked();

    // What the app does: Add Comment on a selection, Go to Comment on
    // commented text.
    let requests: Rc<RefCell<Vec<ContextMenuRequest>>> = Rc::default();
    let went_to: Rc<RefCell<Vec<CommentId>>> = Rc::default();
    let added: Rc<RefCell<Vec<TextRange>>> = Rc::default();
    tools.update(cx, |tools, _| {
        let (requests, went_to, added) = (requests.clone(), went_to.clone(), added.clone());
        let editor = tools.editor().clone();
        tools.set_text_menu_extension(move |request, _window, cx| {
            requests.borrow_mut().push(request.clone());
            // The extension may read the editor (and the tools view).
            let range = editor.read(cx).selection().range();
            let mut items = Vec::new();
            if request.has_selection {
                let added = added.clone();
                items.push(
                    ContextMenuItem::new("Add Comment", move |_, _| added.borrow_mut().push(range))
                        .key_hint(KeyHint::parse("cmd-alt-m")),
                );
            }
            if let Some(thread) = request.comments.first().copied() {
                let went_to = went_to.clone();
                items.push(ContextMenuItem::new("Go to Comment", move |_, _| {
                    went_to.borrow_mut().push(thread)
                }));
            }
            items
        })
    });

    // On commented text: both, after the menu's own items and a
    // separator.
    right_click(&editor, HELLO, 8, cx);
    let all = labels(&tools, cx);
    assert_eq!(
        all[all.len() - 4..],
        ["Select All", "-", "Add Comment", "Go to Comment"]
    );
    assert_eq!(requests.borrow().last().unwrap().comments, vec![thread]);
    choose("Go to Comment", cx);
    assert_eq!(*went_to.borrow(), [thread]);
    assert_eq!(active(&tools, cx), None);

    // On a word without a comment: Add Comment for the selection the
    // right-click made; it is still the selection when the item runs.
    right_click(&editor, HELLO, 2, cx);
    let all = labels(&tools, cx);
    assert_eq!(all[all.len() - 3..], ["Select All", "-", "Add Comment"]);
    choose("Add Comment", cx);
    assert_eq!(*added.borrow(), [TextRange::in_block(HELLO, 0, 5)]);
    assert_eq!(selection(&editor, cx), TextRange::in_block(HELLO, 0, 5));

    // Where the host has nothing to add, no separator is left behind.
    right_click(&editor, EMPTY, 0, cx);
    assert_eq!(
        labels(&tools, cx).last().map(String::as_str),
        Some("Select All")
    );
    assert_eq!(requests.borrow().len(), 3);
}

#[gpui::test]
fn the_host_puts_items_first_behind_a_separator(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open_with_menus(document(), cx);

    // What the app does for a misspelled word: suggestions first, as in
    // every Mac app. Here "brave" stands for the marked word.
    let chosen: Rc<RefCell<Vec<String>>> = Rc::default();
    let requests: Rc<RefCell<Vec<ContextMenuRequest>>> = Rc::default();
    tools.update(cx, |tools, _| {
        let (chosen, requests) = (chosen.clone(), requests.clone());
        tools.set_text_menu_leading_extension(move |request, _window, _cx| {
            requests.borrow_mut().push(request.clone());
            let on_brave = request.document_position.block == HELLO
                && (6..=11).contains(&request.document_position.offset);
            if !on_brave {
                return Vec::new();
            }
            let suggestion = |word: &'static str| {
                let chosen = chosen.clone();
                ContextMenuItem::new(word, move |_, _| chosen.borrow_mut().push(word.into()))
            };
            vec![
                suggestion("bravo"),
                suggestion("braver"),
                ContextMenuItem::separator(),
                suggestion("Learn Spelling"),
            ]
        });
        tools.set_text_menu_extension(|_, _, _| vec![ContextMenuItem::new("Last", |_, _| {})]);
    });

    // On the word: the host's items lead, then a separator, then the
    // menu's own, then what the host appends.
    right_click(&editor, HELLO, 8, cx);
    let all = labels(&tools, cx);
    assert_eq!(
        all[..6],
        ["bravo", "braver", "-", "Learn Spelling", "-", "Cut"]
    );
    assert_eq!(all[all.len() - 3..], ["Select All", "-", "Last"]);
    choose("braver", cx);
    assert_eq!(*chosen.borrow(), ["braver"]);
    assert_eq!(active(&tools, cx), None);

    // Elsewhere nothing is put first and no separator is left behind.
    right_click(&editor, EMPTY, 0, cx);
    assert_eq!(labels(&tools, cx).first().map(String::as_str), Some("Cut"));
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();

    // From the keyboard the first of the host's items is the marked row,
    // and enter chooses it.
    select(&editor, TextRange::in_block(HELLO, 8, 8), cx);
    cx.simulate_keystrokes("shift-f10");
    cx.run_until_parked();
    assert_eq!(labels(&tools, cx)[0], "bravo");
    assert!(requests.borrow().last().unwrap().from_keyboard);
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(*chosen.borrow(), ["braver", "bravo"]);
}

#[gpui::test]
fn the_keyboard_opens_the_same_menu_at_the_caret(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open_with_menus(document(), cx);

    // ctrl-enter leaves the selection alone and drops the menu from the
    // caret, with the first item that can be chosen marked.
    select(&editor, TextRange::in_block(HELLO, 6, 11), cx);
    cx.simulate_keystrokes("ctrl-enter");
    assert_eq!(active(&tools, cx), Some(ToolKind::ContextMenu));
    assert_eq!(selection(&editor, cx), TextRange::in_block(HELLO, 6, 11));
    assert!(tools.read_with(cx, |tools, _| {
        tools.text_menu_request().unwrap().from_keyboard
    }));
    assert_eq!(
        labels(&tools, cx),
        expected(&[&CLIPBOARD, &["-"], &SELECTION, &["-"], &TAIL])
    );
    let caret = editor.read_with(cx, |editor, _| editor.caret_bounds().unwrap());
    let (bounds, marked) = menu(&tools, cx).read_with(cx, |menu, _| {
        (
            menu.bounds().unwrap(),
            menu.selected_item().map(|item| item.title().to_string()),
        )
    });
    assert!(bounds.top() >= caret.bottom(), "below the caret");
    assert!(bounds.top() - caret.bottom() < px(12.));
    assert_eq!(marked.as_deref(), Some("Cut"));
    assert!(
        editor_focused(&editor, cx),
        "the keyboard stays in the text"
    );

    // Arrow keys and enter choose: Cut, Copy, [Paste is disabled], Bold.
    cx.simulate_keystrokes("down down");
    assert_eq!(
        menu(&tools, cx)
            .read_with(cx, |menu, _| menu
                .selected_item()
                .map(|item| item.title().to_string()))
            .as_deref(),
        Some("Bold")
    );
    cx.simulate_keystrokes("enter");
    assert_eq!(active(&tools, cx), None);
    assert!(attrs(&editor, HELLO, 8, cx).bold);
    assert_eq!(text(&editor, cx).lines().next(), Some("Hello brave world"));

    // Escape closes it and nothing changes; typing goes on in the text.
    cx.simulate_keystrokes("ctrl-enter");
    assert_eq!(active(&tools, cx), Some(ToolKind::ContextMenu));
    cx.simulate_keystrokes("escape");
    assert_eq!(active(&tools, cx), None);
    assert!(editor_focused(&editor, cx));
    assert_eq!(selection(&editor, cx), TextRange::in_block(HELLO, 6, 11));

    // A click elsewhere closes it too.
    cx.simulate_keystrokes("ctrl-enter");
    let far = inside(&editor, EMPTY, 0, cx);
    cx.simulate_click(far + point(px(300.), px(0.)), Modifiers::none());
    cx.run_until_parked();
    assert_eq!(active(&tools, cx), None);

    // It is in the palette's commands with its key.
    let command = cx.update(|_, _| {
        innerword_editor::commands()
            .into_iter()
            .find(|command| command.title.as_ref() == "Context Menu")
    });
    assert!(command.is_some());
}

#[gpui::test]
fn every_item_with_a_key_runs_what_the_key_runs(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open_with_menus(document(), cx);
    cx.write_to_clipboard(gpui::ClipboardItem::new_string("x".into()));

    // The menu of a selection on a to-do holds every action row there is,
    // except the link rows.
    select(&editor, TextRange::in_block(TODO, 0, 3), cx);
    right_click(&editor, TODO, 1, cx);
    let items: Vec<ContextMenuItem> = menu(&tools, cx).read_with(cx, |menu, _| {
        menu.items()
            .iter()
            .flat_map(|item| match item.submenu_items() {
                Some(items) => items.to_vec(),
                None => vec![item.clone()],
            })
            .collect()
    });
    cx.update(ContextMenu::dismiss);
    cx.run_until_parked();

    let mut with_key = Vec::new();
    for item in &items {
        let Some(action) = item.action_ref() else {
            continue;
        };
        let title = item.title().to_string();
        // The key the registry has for the action (what the row shows),
        // pressed with the keyboard in the text, runs that action.
        let Some(hint) = cx.update(|_, cx| KeyHint::for_action(action, cx)) else {
            continue;
        };
        let keystrokes: Vec<Keystroke> = hint.keystrokes().to_vec();
        let runs = cx.update(|window, cx| {
            let stack = window.context_stack();
            let keymap = cx.key_bindings();
            let keymap = keymap.borrow();
            let (bindings, _) = keymap.bindings_for_input(&keystrokes, &stack);
            bindings.first().map(|binding| binding.action().name())
        });
        assert_eq!(
            runs,
            Some(action.name()),
            "{title}: the key {} shown for it",
            hint.text()
        );
        with_key.push(title);
    }
    for expected in [
        "Cut",
        "Copy",
        "Paste",
        "Paste as Plain Text",
        "Bold",
        "Italic",
        "Underline",
        "Strikethrough",
        "Inline Code",
        "Clear Formatting",
        "Paragraph",
        "Heading 1",
        "Heading 6",
        "Bullet List",
        "Numbered List",
        "To-do List",
        "Quote",
        "Code Block",
        "Left",
        "Centre",
        "Right",
        "Justify",
        "Larger",
        "Smaller",
        "Mark as Not Done",
        "Page Break",
        "Link",
        "Find in Document",
        "Select All",
    ] {
        if expected == "Mark as Not Done" {
            assert!(
                with_key.iter().any(|title| title.starts_with("Mark as")),
                "{with_key:?}"
            );
            continue;
        }
        assert!(
            with_key.iter().any(|title| title == expected),
            "{expected} has an action and a key: {with_key:?}"
        );
    }
}

#[gpui::test]
fn what_cannot_apply_is_disabled(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open_with_menus(document(), cx);
    cx.write_to_clipboard(gpui::ClipboardItem::new_string("x".into()));

    // In a code block the text carries no inline formatting.
    right_click(&editor, CODE, 1, cx);
    assert!(tools.read_with(cx, |tools, _| {
        tools.text_menu_request().unwrap().has_selection
    }));
    for title in [
        "Bold",
        "Italic",
        "Highlight",
        "Text Colour",
        "Font",
        "Clear Formatting",
    ] {
        assert!(item(&tools, title, cx).is_disabled(), "{title} in code");
    }
    assert!(!item(&tools, "Copy", cx).is_disabled());
    assert!(!item(&tools, "Turn Into", cx).is_disabled());
    assert_eq!(
        item(&tools, "Turn Into > Code Block", cx).is_checked(),
        Some(true)
    );
    cx.update(ContextMenu::dismiss);
    cx.run_until_parked();
    assert_eq!(active(&tools, cx), None);

    // On a divider there is no text to turn into anything.
    let divider = editor.read_with(cx, |editor, _| {
        editor
            .position_bounds(Position::new(DIVIDER, 0))
            .unwrap()
            .center()
    });
    right_click_at(divider + point(px(40.), px(0.)), cx);
    let request = tools.read_with(cx, |tools, _| tools.text_menu_request().cloned().unwrap());
    if request.block == DIVIDER {
        for title in ["Turn Into", "Align", "Text Size"] {
            assert!(
                item(&tools, title, cx).is_disabled(),
                "{title} on a divider"
            );
        }
    }
    cx.update(ContextMenu::dismiss);
    cx.run_until_parked();

    // A read-only document: copy, find and select all are left.
    editor.update(cx, |editor, cx| editor.set_read_only(true, cx));
    right_click(&editor, HELLO, 8, cx);
    for title in [
        "Cut",
        "Paste",
        "Paste as Plain Text",
        "Bold",
        "Highlight",
        "Link",
        "Font",
        "Clear Formatting",
        "Turn Into",
        "Align",
        "Text Size",
        "Insert",
    ] {
        assert!(
            item(&tools, title, cx).is_disabled(),
            "{title} when read-only"
        );
    }
    for title in ["Copy", "Find in Document", "Select All"] {
        assert!(
            !item(&tools, title, cx).is_disabled(),
            "{title} when read-only"
        );
    }
    cx.update(ContextMenu::dismiss);
    cx.run_until_parked();
    right_click(&editor, LINKED, 9, cx);
    assert!(!item(&tools, "Open Link", cx).is_disabled());
    assert!(!item(&tools, "Copy Link Address", cx).is_disabled());
    assert!(item(&tools, "Edit Link", cx).is_disabled());
    assert!(item(&tools, "Remove Link", cx).is_disabled());
}

#[gpui::test]
fn the_menu_and_the_other_tools(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open_with_menus(document(), cx);

    // A right-click while a popup tool is open closes the tool and opens
    // the menu; the find bar stays.
    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("brave");
    select(&editor, TextRange::in_block(HELLO, 0, 5), cx);
    cx.update(|window, cx| editor.read(cx).focus(window));
    cx.simulate_keystrokes("cmd-.");
    assert_eq!(active(&tools, cx), Some(ToolKind::Format));
    // (Beside the selection: the format menu hangs below it.)
    right_click(&editor, HELLO, 14, cx);
    assert_eq!(active(&tools, cx), Some(ToolKind::ContextMenu));
    assert!(tools.read_with(cx, |tools, _| {
        tools.is_open(ToolKind::Find) && !tools.is_open(ToolKind::Format)
    }));
    editor.read_with(cx, |editor, _| assert!(!editor.has_popup()));

    // A second right-click moves the menu to the new place.
    right_click(&editor, TODO, 5, cx);
    assert_eq!(
        tools.read_with(cx, |tools, _| tools.text_menu_request().unwrap().block),
        TODO
    );
    assert!(
        labels(&tools, cx)
            .iter()
            .any(|label| label == "Mark as Done")
    );

    // Replacing the document closes a menu made for the old one.
    editor.update(cx, |editor, cx| {
        editor.set_document(paragraphs(&["New"]), cx)
    });
    cx.run_until_parked();
    assert_eq!(active(&tools, cx), Some(ToolKind::Find));
    assert!(!tools.read_with(cx, |tools, _| tools.text_menu_open()));
    assert!(!cx.update(|window, cx| ContextMenu::is_open(window, cx)));
}
