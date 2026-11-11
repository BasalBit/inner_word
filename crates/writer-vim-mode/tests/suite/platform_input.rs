//! Dead keys and input methods for the command line and for commands that
//! wait for a character. The platform's calls are made on the
//! `EntityInputHandler` implementation, as AppKit would make them.

use gpui::{EntityInputHandler, TestAppContext};
use writer_vim_mode::Mode;

use super::{Rig, at, doc, open, setup};

/// option-e: AppKit marks an acute accent.
fn dead_key(rig: &Rig, accent: &str, cx: &mut gpui::VisualTestContext) {
    rig.vim.update_in(cx, |vim, window, cx| {
        vim.replace_and_mark_text_in_range(None, accent, Some(1..1), window, cx);
    });
    cx.run_until_parked();
}

/// The letter after it: the marked text is replaced by the composed one.
fn commit(rig: &Rig, text: &str, cx: &mut gpui::VisualTestContext) {
    rig.vim.update_in(cx, |vim, window, cx| {
        vim.replace_text_in_range(None, text, window, cx);
    });
    cx.run_until_parked();
}

fn line(rig: &Rig, cx: &mut gpui::VisualTestContext) -> String {
    let line = rig.status(cx).command_line.expect("an open command line");
    format!("{}{}", line.prefix, line.text)
}

#[gpui::test]
fn a_dead_key_composes_in_the_command_line(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["caf\u{e9} one", "cafe two"]), cx);
    assert!(!rig.vim.read_with(cx, |vim, _| vim.wants_text_input()));

    cx.simulate_keystrokes(": s / c a f");
    assert!(rig.vim.read_with(cx, |vim, _| vim.wants_text_input()));
    dead_key(&rig, "\u{b4}", cx);
    // The accent is shown, not typed.
    assert_eq!(line(&rig, cx), ":s/caf");
    assert_eq!(rig.status(cx).composing.as_deref(), Some("\u{b4}"));
    rig.vim.update_in(cx, |vim, window, cx| {
        // "s/caf" is five UTF-16 units, the accent the sixth.
        assert_eq!(vim.marked_text_range(window, cx), Some(5..6));
        let selected = vim.selected_text_range(false, window, cx).unwrap();
        assert_eq!(selected.range, 6..6);
        let mut adjusted = None;
        let text = vim.text_for_range(3..6, &mut adjusted, window, cx);
        assert_eq!(text.as_deref(), Some("af\u{b4}"));
    });
    commit(&rig, "\u{e9}", cx);
    assert_eq!(line(&rig, cx), ":s/caf\u{e9}");
    assert_eq!(rig.status(cx).composing, None);
    cx.simulate_keystrokes("/ t e a enter");
    assert_eq!(rig.text(cx), "tea one\ncafe two");
    assert_eq!(rig.mode(cx), Mode::Normal);
    // The document was never touched by the composition itself.
    cx.simulate_keystrokes("u");
    assert_eq!(rig.text(cx), "caf\u{e9} one\ncafe two");
    cx.simulate_keystrokes("u");
    assert_eq!(rig.text(cx), "caf\u{e9} one\ncafe two");
}

#[gpui::test]
fn a_dead_key_composes_in_a_search(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["one cafe", "two caf\u{e9}", "three"]), cx);
    cx.simulate_keystrokes("/ c a f");
    dead_key(&rig, "\u{b4}", cx);
    commit(&rig, "\u{e9}", cx);
    assert_eq!(line(&rig, cx), "/caf\u{e9}");
    cx.simulate_keystrokes("enter");
    assert_eq!(rig.mode(cx), Mode::Normal);
    assert_eq!(rig.head(cx), at(1, 4));
    assert_eq!(rig.status(cx).message, None);

    // In the middle of the line: at its cursor.
    cx.simulate_keystrokes("/ x y left");
    dead_key(&rig, "\u{a8}", cx);
    rig.vim.update_in(cx, |vim, window, cx| {
        assert_eq!(vim.marked_text_range(window, cx), Some(1..2));
    });
    commit(&rig, "\u{eb}", cx);
    assert_eq!(line(&rig, cx), "/x\u{eb}y");
    assert_eq!(rig.status(cx).command_line.unwrap().cursor, 3);
    cx.simulate_keystrokes("escape");
}

#[gpui::test]
fn a_dead_key_gives_r_and_f_their_character(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["cafe ol\u{e9} x"]), cx);
    // r, option-e, e.
    cx.simulate_keystrokes("3 l r");
    assert!(rig.vim.read_with(cx, |vim, _| vim.wants_text_input()));
    dead_key(&rig, "\u{b4}", cx);
    assert_eq!(rig.status(cx).pending_keys, "r", "r still waits");
    assert_eq!(rig.text(cx), "cafe ol\u{e9} x");
    commit(&rig, "\u{e9}", cx);
    assert_eq!(rig.text(cx), "caf\u{e9} ol\u{e9} x");
    assert_eq!(rig.status(cx).pending_keys, "");
    assert_eq!(rig.mode(cx), Mode::Normal);
    assert!(!rig.vim.read_with(cx, |vim, _| vim.wants_text_input()));
    // One undo step, as a typed r.
    cx.simulate_keystrokes("u");
    assert_eq!(rig.text(cx), "cafe ol\u{e9} x");

    // f, option-e, e: to the next é.
    cx.simulate_keystrokes("0 f");
    dead_key(&rig, "\u{b4}", cx);
    commit(&rig, "\u{e9}", cx);
    assert_eq!(rig.head(cx), at(0, 7));
    // With an operator.
    cx.simulate_keystrokes("0 d t");
    dead_key(&rig, "\u{b4}", cx);
    commit(&rig, "\u{e9}", cx);
    assert_eq!(rig.text(cx), "\u{e9} x");
}

#[gpui::test]
fn the_window_text_input_reaches_vim_only_when_wanted(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["cafe"]), cx);

    // While a composition is marked, a character key is left to the text
    // input, as the platform does it. In a test nothing stands between the
    // key and the window's input handler, so the key arriving in the
    // command line shows that the handler of the window is this one.
    cx.simulate_keystrokes(": s / a");
    dead_key(&rig, "\u{b4}", cx);
    cx.simulate_keystrokes("e");
    assert_eq!(line(&rig, cx), ":s/ae");
    assert_eq!(rig.status(cx).composing, None);
    assert_eq!(rig.text(cx), "cafe");
    cx.simulate_keystrokes("escape");
    assert_eq!(rig.mode(cx), Mode::Normal);

    // The same with the strip closed: the character r waits for.
    cx.simulate_keystrokes("r");
    dead_key(&rig, "\u{b4}", cx);
    cx.simulate_keystrokes("x");
    assert_eq!(rig.text(cx), "xafe");

    // Any other key ends a composition and is used up by that: it reaches
    // neither the command line nor the document.
    cx.simulate_keystrokes(": d");
    dead_key(&rig, "\u{b4}", cx);
    cx.simulate_keystrokes("backspace");
    assert_eq!(rig.status(cx).composing, None);
    assert_eq!(line(&rig, cx), ":d");
    assert_eq!(rig.text(cx), "xafe");
    cx.simulate_keystrokes("escape");
    cx.simulate_keystrokes("r");
    dead_key(&rig, "\u{b4}", cx);
    cx.simulate_keystrokes("backspace");
    assert_eq!(rig.text(cx), "xafe");
    assert_eq!(rig.status(cx).composing, None);
    cx.simulate_keystrokes("escape");

    // In plain Normal mode nothing from the platform is taken.
    assert_eq!(rig.status(cx).pending_keys, "");
    dead_key(&rig, "\u{b4}", cx);
    assert_eq!(rig.status(cx).composing, None);
    commit(&rig, "\u{e9}", cx);
    assert_eq!(rig.text(cx), "xafe");
    assert_eq!(rig.mode(cx), Mode::Normal);

    // Losing the keyboard ends a composition.
    cx.simulate_keystrokes("/ a");
    dead_key(&rig, "\u{b4}", cx);
    cx.update(|window, _| window.blur());
    cx.run_until_parked();
    assert_eq!(rig.status(cx).composing, None);
    assert_eq!(line(&rig, cx), "/a");
}
