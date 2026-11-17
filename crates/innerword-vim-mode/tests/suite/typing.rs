//! Insert and Replace mode: typing takes the editor's normal path (the
//! platform input handler, input rules, smart punctuation) and the engine
//! learns what was inserted.

use gpui::{EntityInputHandler, TestAppContext};
use innerword_editor::CaretStyle;
use innerword_vim_mode::{CaretShape, Mode};

use super::{at, doc, open, setup};

#[gpui::test]
fn insert_mode_types_through_the_input_handler(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["world"]), cx);

    cx.simulate_keystrokes("i");
    let status = rig.status(cx);
    assert_eq!(status.mode, Mode::Insert);
    assert_eq!(status.mode_label(), "INSERT");
    assert_eq!(status.caret, CaretShape::Bar);
    assert_eq!(rig.caret_style(cx), CaretStyle::Bar);
    assert!(rig.text_input(cx));
    assert_eq!(rig.context_mode(cx).as_deref(), Some("insert"));

    cx.simulate_input("hello ");
    assert_eq!(rig.text(cx), "hello world");
    assert_eq!(rig.head(cx), at(0, 6));

    cx.simulate_keystrokes("escape");
    assert_eq!(rig.mode(cx), Mode::Normal);
    assert_eq!(rig.caret_style(cx), CaretStyle::Block);
    assert!(!rig.text_input(cx));
    // escape steps back onto the last typed character.
    assert_eq!(rig.head(cx), at(0, 5));
    // The engine was told what the editor inserted.
    rig.vim.read_with(cx, |vim, _| {
        assert_eq!(vim.engine().last_inserted_text(), "hello ");
    });

    // So dot-repeat types it again.
    cx.simulate_keystrokes(".");
    assert_eq!(rig.text(cx), "hellohello  world");

    // One Insert session is one undo step.
    cx.simulate_keystrokes("u");
    assert_eq!(rig.text(cx), "hello world");
    cx.simulate_keystrokes("u");
    assert_eq!(rig.text(cx), "world");
    assert_eq!(rig.fallbacks(cx), 0);
}

#[gpui::test]
fn a_count_repeats_the_inserted_text(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&[""]), cx);
    cx.simulate_keystrokes("3 i");
    assert_eq!(rig.status(cx).pending_keys, "");
    cx.simulate_input("ab");
    cx.simulate_keystrokes("escape");
    assert_eq!(rig.text(cx), "ababab");
}

#[gpui::test]
fn enter_backspace_and_arrows_keep_working_in_insert_mode(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["one"]), cx);
    cx.simulate_keystrokes("shift-a");
    cx.simulate_input("x");
    cx.simulate_keystrokes("backspace enter");
    cx.simulate_input("two");
    assert_eq!(rig.text(cx), "one\ntwo");
    cx.simulate_keystrokes("left left");
    assert_eq!(rig.head(cx), at(1, 1));
    cx.simulate_keystrokes("shift-enter");
    assert_eq!(rig.text(cx), "one\nt\nwo");
    rig.editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().block_count(), 2);
    });
    // The engine's own Insert keys.
    cx.simulate_keystrokes("ctrl-w");
    assert_eq!(rig.mode(cx), Mode::Insert);
    cx.simulate_keystrokes("escape");
    assert_eq!(rig.mode(cx), Mode::Normal);
}

#[gpui::test]
fn open_line_and_change_word(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["alpha beta", "gamma"]), cx);
    cx.simulate_keystrokes("o");
    assert_eq!(rig.mode(cx), Mode::Insert);
    cx.simulate_input("inserted");
    cx.simulate_keystrokes("escape");
    assert_eq!(rig.text(cx), "alpha beta\ninserted\ngamma");

    cx.simulate_keystrokes("g g c w");
    assert_eq!(rig.mode(cx), Mode::Insert);
    cx.simulate_input("omega");
    cx.simulate_keystrokes("escape");
    assert_eq!(rig.text(cx), "omega beta\ninserted\ngamma");
    // Repeat the change on the next word.
    cx.simulate_keystrokes("w .");
    assert_eq!(rig.text(cx), "omega omega\ninserted\ngamma");
}

#[gpui::test]
fn input_rules_and_smart_punctuation_run_in_insert_mode(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&[""]), cx);
    cx.simulate_keystrokes("i");
    cx.simulate_input("# Title");
    rig.editor.read_with(cx, |editor, _| {
        let block = &editor.document().blocks()[0];
        assert_eq!(block.kind().heading_level(), Some(1));
        assert_eq!(block.text(), "Title");
    });
    cx.simulate_keystrokes("enter");
    cx.simulate_input("\"hi\" -- ok");
    cx.simulate_keystrokes("escape");
    assert_eq!(rig.text(cx), "Title\n\u{201c}hi\u{201d} \u{2014} ok");

    // Dot-repeat types the same keys through the same rules.
    cx.simulate_keystrokes("o");
    cx.simulate_input("- item");
    cx.simulate_keystrokes("escape");
    rig.editor.read_with(cx, |editor, _| {
        let block = &editor.document().blocks()[2];
        assert!(block.kind().is_list_item());
        assert_eq!(block.text(), "item");
    });
}

#[gpui::test]
fn replace_mode_overwrites(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["abcdef"]), cx);
    cx.simulate_keystrokes("shift-r");
    let status = rig.status(cx);
    assert_eq!(status.mode, Mode::Replace);
    assert_eq!(status.caret, CaretShape::Underline);
    assert_eq!(rig.caret_style(cx), CaretStyle::Underline);
    assert!(rig.text_input(cx));
    assert_eq!(rig.context_mode(cx).as_deref(), Some("replace"));

    cx.simulate_input("xy");
    assert_eq!(rig.text(cx), "xycdef");
    assert_eq!(rig.head(cx), at(0, 2));
    // Backspace gives back what was overwritten.
    cx.simulate_keystrokes("backspace");
    assert_eq!(rig.text(cx), "xbcdef");
    cx.simulate_keystrokes("escape");
    assert_eq!(rig.mode(cx), Mode::Normal);
    assert_eq!(rig.caret_style(cx), CaretStyle::Block);
    cx.simulate_keystrokes("u");
    assert_eq!(rig.text(cx), "abcdef");
}

#[gpui::test]
fn one_shot_normal_command_from_insert_mode(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["one two"]), cx);
    cx.simulate_keystrokes("shift-a ctrl-o");
    let status = rig.status(cx);
    assert_eq!(status.mode, Mode::Normal);
    assert!(status.insert_pending);
    assert_eq!(status.mode_label(), "(INSERT)");
    assert!(!rig.text_input(cx));
    cx.simulate_keystrokes("0");
    assert_eq!(rig.mode(cx), Mode::Insert);
    assert!(rig.text_input(cx));
    cx.simulate_input("zero ");
    assert_eq!(rig.text(cx), "zero one two");
}

#[gpui::test]
fn ime_composition_is_reported_when_committed(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["ab"]), cx);
    cx.simulate_keystrokes("a");
    assert_eq!(rig.mode(cx), Mode::Insert);

    // The input method marks text, updates it, then commits.
    rig.editor.update_in(cx, |editor, window, cx| {
        editor.replace_and_mark_text_in_range(None, "n", None, window, cx);
    });
    cx.run_until_parked();
    rig.editor.update_in(cx, |editor, window, cx| {
        editor.replace_and_mark_text_in_range(None, "ni", None, window, cx);
    });
    cx.run_until_parked();
    assert_eq!(rig.text(cx), "anib");
    assert_eq!(rig.mode(cx), Mode::Insert);
    rig.editor.update_in(cx, |editor, window, cx| {
        editor.replace_text_in_range(None, "\u{4f60}", window, cx);
    });
    cx.run_until_parked();
    assert_eq!(rig.text(cx), "a\u{4f60}b");
    cx.simulate_input("!");
    cx.simulate_keystrokes("escape");
    assert_eq!(rig.text(cx), "a\u{4f60}!b");
    rig.vim.read_with(cx, |vim, _| {
        assert_eq!(vim.engine().last_inserted_text(), "\u{4f60}!");
    });

    // The committed text is part of what dot-repeat types.
    cx.simulate_keystrokes("$ .");
    assert_eq!(rig.text(cx), "a\u{4f60}!b\u{4f60}!");
    // And the whole session, composition included, is one undo step.
    cx.simulate_keystrokes("u u");
    assert_eq!(rig.text(cx), "ab");
}

#[gpui::test]
fn keys_of_a_composition_do_not_reach_the_engine(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["ab"]), cx);
    cx.simulate_keystrokes("i");
    rig.editor.update_in(cx, |editor, window, cx| {
        editor.replace_and_mark_text_in_range(None, "ni", None, window, cx);
    });
    cx.run_until_parked();
    // On the real platform the input method takes these keys itself; a key
    // that still arrives while text is marked must not leave Insert mode.
    cx.simulate_keystrokes("escape");
    assert_eq!(rig.mode(cx), Mode::Insert);
}

#[gpui::test]
fn host_undo_and_paste_inside_an_insert_session(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["start"]), cx);
    cx.simulate_keystrokes("shift-a");
    cx.simulate_input(" one");
    // The editor's own undo, in the middle of the session.
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(rig.mode(cx), Mode::Insert);
    assert!(rig.text_input(cx));
    let after_undo = rig.text(cx);
    assert!(after_undo.starts_with("start"));
    cx.simulate_input(" two");
    // A paste from the clipboard is an outside change, not typed text.
    cx.write_to_clipboard(gpui::ClipboardItem::new_string("[pasted]".into()));
    cx.simulate_keystrokes("cmd-v");
    cx.simulate_input("!");
    cx.simulate_keystrokes("escape");
    assert_eq!(rig.mode(cx), Mode::Normal);
    assert_eq!(rig.text(cx), format!("{after_undo} two[pasted]!"));
    // Only what was typed after the paste repeats.
    rig.vim.read_with(cx, |vim, _| {
        assert_eq!(vim.engine().last_inserted_text(), "!");
    });
    // Undo still works from Normal mode, down to the original text.
    for _ in 0..8 {
        cx.simulate_keystrokes("u");
    }
    assert_eq!(rig.text(cx), "start");
    rig.editor
        .read_with(cx, |editor, _| editor.state().check_invariants().unwrap());
}

#[gpui::test]
fn a_document_replaced_under_an_open_command_line(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["old"]), cx);
    cx.simulate_keystrokes("/ o l");
    assert_eq!(rig.mode(cx), Mode::CommandLine);
    rig.editor
        .update(cx, |editor, cx| editor.set_document(doc(&["new one"]), cx));
    cx.run_until_parked();
    let status = rig.status(cx);
    assert_eq!(status.mode, Mode::Normal);
    assert_eq!(status.command_line, None);
    assert_eq!(status.pending_keys, "");
    cx.simulate_keystrokes("x");
    assert_eq!(rig.text(cx), "ew one");
}
