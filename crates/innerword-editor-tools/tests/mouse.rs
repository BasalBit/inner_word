//! The mouse is never required, but it works: rows and buttons can be
//! clicked, and clicks on an overlay do not fall through to the document.

use gpui::{Bounds, Modifiers, Pixels, TestAppContext, VisualTestContext};
use innerword_core::{Block, BlockKind, Document, InlineAttrs, TextRange};
use innerword_editor_tools::ToolKind;

use crate::support::*;

fn bounds(selector: &'static str, cx: &mut VisualTestContext) -> Bounds<Pixels> {
    cx.debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} was not drawn"))
}

fn click(selector: &'static str, cx: &mut VisualTestContext) {
    let target = bounds(selector, cx).center();
    cx.simulate_click(target, Modifiers::none());
    cx.run_until_parked();
}

#[gpui::test]
fn menu_rows_can_be_hovered_and_clicked(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(paragraphs(&["Hello brave world", "Second", "Third"]), cx);
    select(&editor, TextRange::in_block(0, 6, 11), cx);

    cx.simulate_keystrokes("cmd-.");
    // Entries: header "Text", Bold, Italic, Underline, ...
    let underline = bounds("menu-row-3", cx).center();
    cx.simulate_mouse_move(underline, None, Modifiers::none());
    assert_eq!(selected(&tools, cx).as_deref(), Some("Underline"));

    click("menu-row-2", cx);
    assert_eq!(active(&tools, cx), None);
    assert!(editor_focused(&editor, cx));
    editor.read_with(cx, |editor, _| {
        let block = &editor.document().blocks()[0];
        assert!(block.attrs_at(6).unwrap().italic);
        assert!(!block.attrs_at(6).unwrap().underline);
        // The click did not reach the text under the menu.
        assert_eq!(editor.selection().range(), TextRange::in_block(0, 6, 11));
    });

    // A click on a header does nothing and keeps the menu.
    cx.simulate_keystrokes("cmd-.");
    click("menu-row-0", cx);
    assert_eq!(active(&tools, cx), Some(ToolKind::Format));
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.selection().range(), TextRange::in_block(0, 6, 11));
    });
    // Typing still goes to the filter after the click.
    cx.simulate_input("bold");
    assert_eq!(selected(&tools, cx).as_deref(), Some("Bold"));
    assert_eq!(text(&editor, cx), "Hello brave world\nSecond\nThird");
}

#[gpui::test]
fn insert_menu_rows_can_be_clicked(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(Document::new(), cx);

    cx.simulate_input("/");
    // Entries: Heading 1, Heading 2, ...
    click("menu-row-0", cx);
    assert_eq!(active(&tools, cx), None);
    assert!(editor_focused(&editor, cx));
    editor.read_with(cx, |editor, _| {
        assert_eq!(
            editor.document().blocks()[0].kind().heading_level(),
            Some(1)
        );
        assert_eq!(editor.document().blocks()[0].text(), "");
    });
}

#[gpui::test]
fn find_bar_buttons_work_and_keep_focus_in_the_field(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(paragraphs(&["cat Cat cat", "concat"]), cx);
    let status = |cx: &mut VisualTestContext| tools.read_with(cx, |tools, _| tools.find_status());

    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("cat");
    assert_eq!(status(cx), "1 of 4");
    click("find-next", cx);
    assert_eq!(status(cx), "2 of 4");
    click("find-previous", cx);
    assert_eq!(status(cx), "1 of 4");
    click("find-case", cx);
    assert_eq!(status(cx), "1 of 3");
    click("find-word", cx);
    assert_eq!(status(cx), "1 of 2");
    click("find-case", cx);
    click("find-word", cx);
    assert_eq!(status(cx), "1 of 4");
    // Focus never left the query field.
    cx.simulate_input("x");
    assert_eq!(
        tools.read_with(cx, |tools, _| tools.find_query().to_string()),
        "catx"
    );
    cx.simulate_keystrokes("backspace");

    // The disclosure shows the replace row; its buttons replace.
    click("find-disclosure", cx);
    assert!(tools.read_with(cx, |tools, _| tools.find_replace_visible()));
    cx.simulate_keystrokes("tab");
    cx.simulate_input("dog");
    click("find-replace-one", cx);
    assert_eq!(text(&editor, cx), "dog Cat cat\nconcat");
    click("find-replace-all", cx);
    assert_eq!(text(&editor, cx), "dog dog dog\ncondog");
    assert_eq!(status(cx), "Replaced 3");

    click("find-close", cx);
    assert_eq!(active(&tools, cx), None);
    assert!(editor_focused(&editor, cx));
}

#[gpui::test]
fn link_popover_actions_can_be_clicked(cx: &mut TestAppContext) {
    setup(cx);
    let link = InlineAttrs {
        link: Some("https://example.com".into()),
        ..InlineAttrs::default()
    };
    let document = Document::from_blocks(vec![
        Block::new(BlockKind::Paragraph).with_span("linked", link),
    ]);
    let (tools, editor, cx) = open(document, cx);
    caret(&editor, 0, 3, cx);
    let link_at = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |editor, _| {
            editor.document().blocks()[0]
                .attrs_at(0)
                .and_then(|attrs| attrs.link.clone())
        })
    };

    cx.simulate_keystrokes("cmd-k");
    click("link-open", cx);
    assert_eq!(cx.opened_url().as_deref(), Some("https://example.com"));
    assert_eq!(active(&tools, cx), Some(ToolKind::Link));
    click("link-cancel", cx);
    assert_eq!(active(&tools, cx), None);
    assert!(editor_focused(&editor, cx));
    assert!(link_at(cx).is_some());

    cx.simulate_keystrokes("cmd-k cmd-a");
    cx.simulate_input("https://other.example");
    click("link-apply", cx);
    assert_eq!(link_at(cx).as_deref(), Some("https://other.example"));
    assert!(editor_focused(&editor, cx));

    cx.simulate_keystrokes("cmd-k");
    click("link-remove", cx);
    assert_eq!(link_at(cx), None);
    assert_eq!(active(&tools, cx), None);
    assert!(editor_focused(&editor, cx));
}

/// A press at the place of a document position (a little inside the
/// character after it).
fn click_text(
    editor: &gpui::Entity<innerword_editor::Editor>,
    block: usize,
    offset: usize,
    cx: &mut VisualTestContext,
) {
    let at = editor.read_with(cx, |editor, _| {
        editor
            .position_bounds(innerword_core::Position::new(block, offset))
            .unwrap()
    });
    cx.simulate_click(
        gpui::point(at.left() + gpui::px(1.), at.center().y),
        Modifiers::none(),
    );
    cx.run_until_parked();
}

#[gpui::test]
fn a_click_outside_closes_a_popup(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(
        paragraphs(&[
            "Hello brave world",
            "",
            "",
            "",
            "",
            "",
            "",
            "",
            "",
            "",
            "",
            "",
            "Last line here",
        ]),
        cx,
    );
    let query =
        |cx: &mut VisualTestContext| tools.read_with(cx, |tools, _| tools.find_query().to_string());

    // In the text: the menu closes and the click does what a click does.
    select(&editor, TextRange::in_block(0, 6, 11), cx);
    cx.simulate_keystrokes("cmd-.");
    assert_eq!(active(&tools, cx), Some(ToolKind::Format));
    click_text(&editor, 12, 4, cx);
    assert_eq!(active(&tools, cx), None);
    assert!(editor_focused(&editor, cx));
    editor.read_with(cx, |editor, _| {
        assert!(!editor.has_popup());
        assert_eq!(
            editor.selection(),
            innerword_core::Selection::caret(innerword_core::Position::new(12, 4))
        );
    });

    // On something that does not take the keyboard (a button of the find
    // bar): the popup closes all the same, the button does its work, and
    // the keyboard is in the text, not in a field that is gone.
    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("l");
    cx.simulate_keystrokes("cmd-t");
    assert_eq!(active(&tools, cx), Some(ToolKind::Font));
    let before = tools.read_with(cx, |tools, _| tools.find_current_match());
    click("find-next", cx);
    assert_eq!(
        active(&tools, cx),
        Some(ToolKind::Find),
        "only the bar is left"
    );
    editor.read_with(cx, |editor, _| assert!(!editor.has_popup()));
    assert_ne!(
        tools.read_with(cx, |tools, _| tools.find_current_match()),
        before
    );
    assert!(editor_focused(&editor, cx));

    // On a field: the popup closes and the keyboard is in that field.
    cx.simulate_keystrokes("cmd-k");
    assert_eq!(active(&tools, cx), Some(ToolKind::Link));
    click("find-query-frame", cx);
    assert_eq!(active(&tools, cx), Some(ToolKind::Find));
    assert!(!editor_focused(&editor, cx));
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("brave");
    assert_eq!(query(cx), "brave");
    assert_eq!(text(&editor, cx).lines().next(), Some("Hello brave world"));
    cx.simulate_keystrokes("escape");

    // The insert menu has no field and leaves the keyboard in the text; a
    // click on the window's chrome closes it and keeps what was typed.
    caret(&editor, 1, 0, cx);
    cx.simulate_keystrokes("cmd-f");
    cx.update(|window, cx| editor.read(cx).focus(window));
    cx.run_until_parked();
    cx.simulate_input("/he");
    assert_eq!(active(&tools, cx), Some(ToolKind::Insert));
    click("find-case", cx);
    assert_eq!(active(&tools, cx), Some(ToolKind::Find));
    editor.read_with(cx, |editor, _| {
        assert!(!editor.has_popup());
        assert_eq!(editor.document().blocks()[1].text(), "/he");
    });
    assert!(editor_focused(&editor, cx));

    // A click inside a popup is not a click outside.
    cx.simulate_keystrokes("escape");
    select(&editor, TextRange::in_block(0, 6, 11), cx);
    cx.simulate_keystrokes("cmd-k");
    click("link-popover", cx);
    assert_eq!(active(&tools, cx), Some(ToolKind::Link));
    cx.simulate_input("example.com");
    assert_eq!(
        tools
            .read_with(cx, |tools, cx| tools.link_field_text(cx))
            .as_deref(),
        Some("example.com")
    );
}

#[gpui::test]
fn both_uses_of_a_font_can_be_chosen_with_the_mouse(cx: &mut TestAppContext) {
    use innerword_ui::ContextMenu;
    setup(cx);
    cx.update(|cx| {
        innerword_editor_tools::set_font_families(["Baskerville", "Georgia", "Menlo", "Zapfino"], cx)
    });
    let (tools, editor, cx) = open_with_menus(paragraphs(&["Set this in another face"]), cx);
    select(&editor, TextRange::in_block(0, 4, 8), cx);
    let body_font = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |editor, _| {
            editor.document().settings().typography.body_font.clone()
        })
    };
    let font_at = |offset: usize, cx: &mut VisualTestContext| {
        editor.read_with(cx, |editor, _| {
            editor.document().blocks()[0]
                .attrs_at(offset)
                .and_then(|attrs| attrs.font.clone())
        })
    };
    let menu_labels = |cx: &mut VisualTestContext| {
        cx.update(|window, cx| ContextMenu::current(window, cx).map(|menu| menu.read(cx).labels()))
    };
    let row_of = |label: &str, cx: &mut VisualTestContext| {
        rows(&tools, cx)
            .iter()
            .rposition(|row| row.label == label)
            .unwrap()
    };

    // The row under the pointer shows a small "more" button; it opens the
    // row's menu with both things a family can be used for.
    cx.simulate_keystrokes("cmd-t");
    let georgia = row_of("Georgia", cx);
    let row = cx
        .debug_bounds(Box::leak(format!("menu-row-{georgia}").into_boxed_str()))
        .unwrap();
    cx.simulate_mouse_move(row.center(), None, Modifiers::none());
    assert_eq!(selected(&tools, cx).as_deref(), Some("Georgia"));
    let more = cx
        .debug_bounds(Box::leak(
            format!("menu-row-more-{georgia}").into_boxed_str(),
        ))
        .expect("the selected row has a more button");
    cx.simulate_click(more.center(), Modifiers::none());
    cx.run_until_parked();
    assert_eq!(
        menu_labels(cx),
        Some(vec![
            "Apply to the Selection".to_string(),
            "Set as Document Font".to_string()
        ])
    );
    assert_eq!(active(&tools, cx), Some(ToolKind::Font), "the picker stays");
    assert_eq!(
        font_at(5, cx),
        None,
        "the click on the button applied nothing"
    );

    // A click on the menu's row (outside the picker) chooses it.
    let item = cx.update(|window, cx| {
        ContextMenu::current(window, cx)
            .unwrap()
            .read(cx)
            .item_bounds(1)
            .unwrap()
            .center()
    });
    cx.simulate_mouse_move(item, None, Modifiers::none());
    cx.simulate_click(item, Modifiers::none());
    cx.run_until_parked();
    assert_eq!(body_font(cx), "Georgia");
    assert_eq!(font_at(5, cx), None);
    assert_eq!(active(&tools, cx), None);
    assert!(editor_focused(&editor, cx));

    // A right-click on a row opens the same menu.
    cx.simulate_keystrokes("cmd-t");
    let menlo = row_of("Menlo", cx);
    let row = cx
        .debug_bounds(Box::leak(format!("menu-row-{menlo}").into_boxed_str()))
        .unwrap();
    cx.simulate_mouse_move(row.center(), None, Modifiers::none());
    cx.simulate_mouse_down(row.center(), gpui::MouseButton::Right, Modifiers::none());
    cx.simulate_mouse_up(row.center(), gpui::MouseButton::Right, Modifiers::none());
    cx.run_until_parked();
    assert_eq!(menu_labels(cx).map(|labels| labels.len()), Some(2));
    assert!(cx.update(|window, cx| ContextMenu::choose("Apply to the Selection", window, cx)));
    cx.run_until_parked();
    assert_eq!(font_at(5, cx).as_deref(), Some("Menlo"));
    assert_eq!(body_font(cx), "Georgia");
    assert_eq!(active(&tools, cx), None);

    // Escape closes the row's menu first, then the picker.
    cx.simulate_keystrokes("cmd-t");
    let row = cx
        .debug_bounds(Box::leak(format!("menu-row-{menlo}").into_boxed_str()))
        .unwrap();
    cx.simulate_mouse_down(row.center(), gpui::MouseButton::Right, Modifiers::none());
    cx.simulate_mouse_up(row.center(), gpui::MouseButton::Right, Modifiers::none());
    cx.run_until_parked();
    assert!(menu_labels(cx).is_some());
    cx.simulate_keystrokes("escape");
    assert!(menu_labels(cx).is_none());
    assert_eq!(active(&tools, cx), Some(ToolKind::Font));
    cx.simulate_keystrokes("escape");
    assert_eq!(active(&tools, cx), None);
}
