//! Where things end up on screen (with the test platform's metrics).

use gpui::{TestAppContext, px, size};
use writer_core::TextRange;
use writer_editor_tools::{FIND_BUTTONS, FindBarPlacement};

use crate::support::*;

#[gpui::test]
fn the_find_bar_goes_above_or_below_the_editor(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(paragraphs(&["Hello there"]), cx);
    let window = cx.update(|window, _| window.viewport_size());
    let viewport = |cx: &mut gpui::VisualTestContext| {
        cx.run_until_parked();
        editor.read_with(cx, |editor, _| editor.viewport_bounds().unwrap())
    };

    // Without a bar the editor has the whole view.
    let full = viewport(cx);
    assert_eq!(full.size, window);

    cx.simulate_keystrokes("cmd-f");
    let bar = cx.debug_bounds("find-bar").unwrap();
    let below = viewport(cx);
    assert_eq!(bar.top(), px(0.));
    assert_eq!(bar.size.width, window.width);
    assert!(
        bar.size.height >= px(36.) && bar.size.height <= px(40.),
        "a slim bar"
    );
    assert_eq!(below.top(), bar.bottom());
    assert_eq!(below.bottom(), window.height);

    // The replace row makes it one row taller.
    cx.simulate_keystrokes("cmd-alt-f");
    let with_replace = cx.debug_bounds("find-bar").unwrap();
    assert_eq!(with_replace.size.height, bar.size.height + px(36.));
    assert_eq!(viewport(cx).top(), with_replace.bottom());

    tools.update(cx, |tools, cx| {
        tools.set_find_bar_placement(FindBarPlacement::Bottom, cx)
    });
    let above = viewport(cx);
    let bar = cx.debug_bounds("find-bar").unwrap();
    assert_eq!(above.top(), px(0.));
    assert_eq!(above.bottom(), bar.top());
    assert_eq!(bar.bottom(), window.height);

    // Closed: the editor has everything again.
    cx.simulate_keystrokes("escape");
    assert_eq!(viewport(cx).size, window);
}

#[gpui::test]
fn popups_sit_under_their_anchor_inside_the_editor(cx: &mut TestAppContext) {
    setup(cx);
    let (_tools, editor, cx) = open(paragraphs(&["Hello there, reader", "Second"]), cx);
    let viewport = editor.read_with(cx, |editor, _| editor.viewport_bounds().unwrap());

    // Format menu at a collapsed caret.
    caret(&editor, 0, 6, cx);
    cx.simulate_keystrokes("cmd-.");
    let caret_bounds = editor.read_with(cx, |editor, _| editor.caret_bounds().unwrap());
    let menu = cx.debug_bounds("tools-menu").unwrap();
    assert_eq!(menu.size.width, px(300.));
    assert!(menu.top() >= caret_bounds.bottom(), "below the caret");
    assert!(
        menu.top() - caret_bounds.bottom() < px(12.),
        "and close to it"
    );
    assert_eq!(menu.left(), caret_bounds.left());
    assert!(menu.bottom() <= viewport.bottom() && menu.right() <= viewport.right());
    // A query row, ten rows of 28px, and the panel's padding and border.
    assert!(menu.size.height > px(34. + 280.) && menu.size.height < px(34. + 280. + 16.));
    cx.simulate_keystrokes("escape");

    // Link popover under a selection.
    select(&editor, TextRange::in_block(0, 6, 11), cx);
    cx.simulate_keystrokes("cmd-k");
    let selection = editor.read_with(cx, |editor, _| editor.selection_bounds().unwrap());
    let popover = cx.debug_bounds("link-popover").unwrap();
    assert_eq!(popover.size.width, px(380.));
    assert!(popover.top() >= selection.bottom());
    assert_eq!(popover.left(), selection.left());
    assert!(popover.size.height > px(60.) && popover.size.height < px(72.));
    cx.simulate_keystrokes("escape");

    // The insert menu hangs from the slash, not from the moving caret.
    caret(&editor, 1, 6, cx);
    cx.simulate_input(" /");
    let slash = editor.read_with(cx, |editor, _| {
        editor
            .position_bounds(writer_core::Position::new(1, 7))
            .unwrap()
    });
    let menu = cx.debug_bounds("tools-menu").unwrap();
    assert_eq!(menu.left(), slash.left());
    assert_eq!(menu.size.width, px(280.));
    cx.simulate_input("head");
    let narrowed = cx.debug_bounds("tools-menu").unwrap();
    assert_eq!(narrowed.left(), slash.left());
    assert!(
        narrowed.size.height < menu.size.height,
        "six rows instead of eight"
    );
}

#[gpui::test]
fn the_find_bar_fits_a_narrow_editor_and_its_fields_line_up(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, _editor, cx) = open(paragraphs(&["Hello there", "and there"]), cx);
    cx.simulate_keystrokes("cmd-alt-f");
    cx.simulate_input("there");
    let window = cx.update(|window, _| window.viewport_size());

    let mut widths = Vec::new();
    for width in [window.width, px(700.), px(560.), px(440.), px(380.)] {
        cx.simulate_resize(size(width, px(500.)));
        // The bar decides on its key hints from the editor's last layout:
        // one frame to lay out at the new width, one to follow it.
        for _ in 0..2 {
            cx.run_until_parked();
            cx.update(|window, _| window.refresh());
        }
        cx.run_until_parked();
        let bounds = |selector: &'static str, cx: &mut gpui::VisualTestContext| {
            cx.debug_bounds(selector)
                .unwrap_or_else(|| panic!("{selector} is not drawn"))
        };
        let bar = bounds("find-bar", cx);
        assert_eq!(bar.size.width, width);
        // Every button is on screen, inside the bar.
        for button in FIND_BUTTONS {
            let button_bounds = bounds(button, cx);
            assert!(
                button_bounds.left() >= bar.left() && button_bounds.right() <= bar.right(),
                "{button} at {button_bounds:?} leaves the {width:?} bar"
            );
            assert!(button_bounds.size.height == px(24.));
        }
        // The two fields share their left and right edges and keep a
        // width one can type in.
        let query = bounds("find-query-frame", cx);
        let replace = bounds("find-replace-frame", cx);
        assert_eq!(query.left(), replace.left(), "at {width:?}");
        assert_eq!(query.right(), replace.right(), "at {width:?}");
        assert!(query.size.width >= px(96.), "{:?} at {width:?}", query.size);
        widths.push((width, bounds("find-next", cx).size.width));
    }
    // In the narrow bar the buttons lose the key hints beside them (the
    // tooltips keep them).
    let wide = widths[0].1;
    let narrow = widths.last().unwrap().1;
    assert!(narrow < wide, "{widths:?}");
    assert_eq!(narrow, px(24.));
    assert_eq!(
        tools
            .read_with(cx, |tools, cx| tools.find_button_tooltip("find-next", cx))
            .as_deref(),
        Some("Next match ⌘G")
    );
}

#[gpui::test]
fn the_find_label_floats_inside_the_editor_without_moving_it(cx: &mut TestAppContext) {
    setup(cx);
    let (_tools, editor, cx) = open(paragraphs(&["Hello there", "and there"]), cx);
    let viewport = |cx: &mut gpui::VisualTestContext| {
        cx.run_until_parked();
        editor.read_with(cx, |editor, _| editor.viewport_bounds().unwrap())
    };
    let full = viewport(cx);
    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("there");
    cx.simulate_keystrokes("escape");
    assert_eq!(viewport(cx), full);

    // cmd-g with the bar closed: the label is in the top right corner of
    // the editor, and the text does not move (no bar is put in).
    cx.simulate_keystrokes("cmd-g");
    let label = cx.debug_bounds("find-flash").expect("the label is drawn");
    assert_eq!(viewport(cx), full);
    assert!(label.top() > full.top() && label.bottom() < full.top() + px(48.));
    assert!(label.right() < full.right() && label.right() > full.right() - px(32.));
    assert!(label.size.width <= px(320.));
}
