//! Normal, Visual and operator-pending mode through the real key path;
//! attaching and detaching.

use gpui::{
    EntityInputHandler, Keystroke, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, Pixels,
    Point, TestAppContext, VisualTestContext, point, px,
};
use writer_core::Selection;
use writer_editor::{CaretStyle, InterceptPriority, KeyDisposition};
use writer_vim_mode::{CaretShape, Mode, VimEvent};

use super::{at, doc, open, open_detached, setup};

#[gpui::test]
fn attaching_enters_normal_mode(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open_detached(doc(&["hello"]), cx);
    // Detached: nothing of the editor is touched.
    assert!(!rig.status(cx).attached);
    assert_eq!(rig.caret_style(cx), CaretStyle::Bar);
    assert!(rig.text_input(cx));
    assert_eq!(rig.context_mode(cx), None);

    // The caret is after the last character, where Normal mode cannot be.
    cx.simulate_keystrokes("cmd-right");
    assert_eq!(rig.head(cx), at(0, 5));

    rig.vim.update(cx, |vim, cx| vim.attach(cx));
    cx.run_until_parked();
    let status = rig.status(cx);
    assert!(status.attached);
    assert_eq!(status.mode, Mode::Normal);
    assert_eq!(status.mode_label(), "NORMAL");
    assert_eq!(status.caret, CaretShape::Block);
    assert_eq!(rig.caret_style(cx), CaretStyle::Block);
    assert!(!rig.text_input(cx));
    assert_eq!(rig.context_mode(cx).as_deref(), Some("normal"));
    // On the last character.
    assert_eq!(rig.head(cx), at(0, 4));
    let events = rig.take_events();
    assert!(events.contains(&VimEvent::Attached));
    assert!(events.contains(&VimEvent::StatusChanged));
}

#[gpui::test]
fn normal_mode_commands_edit_the_document(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["hello brave world", "second line"]), cx);

    cx.simulate_keystrokes("w d w");
    assert_eq!(rig.text(cx), "hello world\nsecond line");
    assert_eq!(rig.head(cx), at(0, 6));
    assert_eq!(rig.mode(cx), Mode::Normal);

    cx.simulate_keystrokes("x");
    assert_eq!(rig.text(cx), "hello orld\nsecond line");
    cx.simulate_keystrokes("u");
    assert_eq!(rig.text(cx), "hello world\nsecond line");
    cx.simulate_keystrokes("u");
    assert_eq!(rig.text(cx), "hello brave world\nsecond line");
    cx.simulate_keystrokes("ctrl-r");
    assert_eq!(rig.text(cx), "hello world\nsecond line");

    // Linewise: blocks move as blocks.
    cx.simulate_keystrokes("d d");
    assert_eq!(rig.text(cx), "second line");
    cx.simulate_keystrokes("p");
    assert_eq!(rig.text(cx), "second line\nhello world");
    assert_eq!(rig.head(cx).block, 1);

    // Counts, `$`, `0`, `G`, `g g`.
    cx.simulate_keystrokes("g g $");
    assert_eq!(rig.head(cx), at(0, 10));
    cx.simulate_keystrokes("0 2 l");
    assert_eq!(rig.head(cx), at(0, 2));
    cx.simulate_keystrokes("shift-g");
    assert_eq!(rig.head(cx).block, 1);

    // A character that is no command is swallowed, not typed.
    let before = rig.text(cx);
    cx.simulate_keystrokes("q");
    assert_eq!(rig.text(cx), before);
    assert_eq!(rig.fallbacks(cx), 0);
}

#[gpui::test]
fn operator_pending_shows_pending_keys_and_an_underline_caret(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["one two three four"]), cx);

    cx.simulate_keystrokes("2 d");
    let status = rig.status(cx);
    assert_eq!(status.mode, Mode::OperatorPending);
    assert_eq!(status.pending_keys, "2d");
    assert_eq!(status.caret, CaretShape::Underline);
    assert_eq!(rig.caret_style(cx), CaretStyle::Underline);
    assert_eq!(rig.context_mode(cx).as_deref(), Some("operator"));

    cx.simulate_keystrokes("w");
    assert_eq!(rig.text(cx), "three four");
    let status = rig.status(cx);
    assert_eq!(status.mode, Mode::Normal);
    assert_eq!(status.pending_keys, "");
    assert_eq!(rig.caret_style(cx), CaretStyle::Block);

    // escape cancels a pending command.
    cx.simulate_keystrokes("d escape");
    assert_eq!(rig.status(cx).pending_keys, "");
    assert_eq!(rig.mode(cx), Mode::Normal);
    assert_eq!(rig.text(cx), "three four");

    // `r` waits for its character with an underline caret.
    cx.simulate_keystrokes("r");
    assert_eq!(rig.caret_style(cx), CaretStyle::Underline);
    cx.simulate_keystrokes("T");
    assert_eq!(rig.text(cx), "Three four");
    assert_eq!(rig.caret_style(cx), CaretStyle::Block);
}

#[gpui::test]
fn visual_mode_selects_characters_and_marks_the_cursor(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["hello world"]), cx);

    cx.simulate_keystrokes("v");
    assert_eq!(rig.mode(cx), Mode::Visual);
    assert_eq!(rig.status(cx).mode_label(), "VISUAL");
    assert_eq!(rig.context_mode(cx).as_deref(), Some("visual"));
    // The character under the cursor is selected.
    assert_eq!(rig.selected_text(cx), "h");
    assert_eq!(rig.cursor(cx), at(0, 0));

    cx.simulate_keystrokes("e");
    assert_eq!(rig.selected_text(cx), "hello");
    assert_eq!(rig.selection(cx), Selection::new(at(0, 0), at(0, 5)));
    // The Vim cursor is on the "o"; the head of the editor's selection is
    // after it, so the editor is told to draw its block caret on the "o".
    assert_eq!(rig.cursor(cx), at(0, 4));
    assert_eq!(rig.head(cx), at(0, 5));
    assert_eq!(rig.caret_override(cx), Some(at(0, 4)));
    assert_eq!(rig.drawn_caret(cx), at(0, 4));
    assert_eq!(rig.caret_style(cx), CaretStyle::Block);
    assert_eq!(rig.status(cx).caret, CaretShape::Block);

    // The editor's own commands work on the Visual selection.
    cx.simulate_keystrokes("cmd-b");
    rig.editor.read_with(cx, |editor, _| {
        let block = &editor.document().blocks()[0];
        assert!(block.attrs_at(0).unwrap().bold);
        assert!(block.attrs_at(4).unwrap().bold);
        assert!(!block.attrs_at(5).unwrap().bold);
    });
    assert_eq!(rig.mode(cx), Mode::Visual);
    assert_eq!(rig.selected_text(cx), "hello");

    cx.simulate_keystrokes("d");
    assert_eq!(rig.text(cx), " world");
    assert_eq!(rig.mode(cx), Mode::Normal);
    assert_eq!(rig.caret_override(cx), None);
    assert_eq!(rig.caret_style(cx), CaretStyle::Block);
    assert!(rig.selection(cx).is_collapsed());
}

#[gpui::test]
fn backward_visual_selection_keeps_the_block_caret(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["hello"]), cx);
    cx.simulate_keystrokes("$ v h h");
    assert_eq!(rig.selected_text(cx), "llo");
    // The head of the editor's selection is the cursor: its block caret
    // is in the right place.
    assert_eq!(rig.cursor(cx), at(0, 2));
    assert_eq!(rig.head(cx), at(0, 2));
    assert_eq!(rig.caret_style(cx), CaretStyle::Block);
    assert_eq!(rig.caret_override(cx), None);

    // `o` swaps the ends: now the cursor is the last selected character.
    cx.simulate_keystrokes("o");
    assert_eq!(rig.cursor(cx), at(0, 4));
    assert_eq!(rig.caret_override(cx), Some(at(0, 4)));

    cx.simulate_keystrokes("escape");
    assert_eq!(rig.mode(cx), Mode::Normal);
    assert!(rig.selection(cx).is_collapsed());
    assert_eq!(rig.caret_override(cx), None);
}

#[gpui::test]
fn visual_line_mode_selects_whole_blocks(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["one", "two", "three"]), cx);
    cx.simulate_keystrokes("l shift-v");
    assert_eq!(rig.mode(cx), Mode::VisualLine);
    assert_eq!(rig.status(cx).mode_label(), "VISUAL LINE");
    assert_eq!(rig.context_mode(cx).as_deref(), Some("visual_line"));
    assert_eq!(rig.selected_text(cx), "one");
    // The cursor stays where it was inside the line.
    assert_eq!(rig.cursor(cx), at(0, 1));
    assert_eq!(rig.caret_override(cx), Some(at(0, 1)));

    cx.simulate_keystrokes("j");
    assert_eq!(rig.selection(cx), Selection::new(at(0, 0), at(1, 3)));
    assert_eq!(rig.cursor(cx), at(1, 1));
    assert_eq!(rig.caret_override(cx), Some(at(1, 1)));

    cx.simulate_keystrokes("d");
    assert_eq!(rig.text(cx), "three");
    assert_eq!(rig.mode(cx), Mode::Normal);
    cx.simulate_keystrokes("shift-p");
    assert_eq!(rig.text(cx), "one\ntwo\nthree");
    assert_eq!(rig.fallbacks(cx), 0);
}

#[gpui::test]
fn cmd_shortcuts_reach_the_editor_in_normal_mode(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["hello world", "next"]), cx);

    cx.simulate_keystrokes("d w");
    assert_eq!(rig.text(cx), "world\nnext");
    // Undo and redo through the editor's own bindings.
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(rig.text(cx), "hello world\nnext");
    assert_eq!(rig.mode(cx), Mode::Normal);
    cx.simulate_keystrokes("cmd-shift-z");
    assert_eq!(rig.text(cx), "world\nnext");

    // Block commands.
    cx.simulate_keystrokes("cmd-alt-1");
    rig.editor.read_with(cx, |editor, _| {
        assert_eq!(
            editor.document().blocks()[0].kind().heading_level(),
            Some(1)
        );
    });

    // Select all becomes a Visual selection; escape leaves it.
    cx.simulate_keystrokes("cmd-a");
    assert_eq!(rig.mode(cx), Mode::Visual);
    assert_eq!(rig.selected_text(cx), "world\nnext");
    cx.simulate_keystrokes("escape");
    assert_eq!(rig.mode(cx), Mode::Normal);
    assert!(rig.selection(cx).is_collapsed());

    // Paste from the clipboard at the caret.
    cx.simulate_keystrokes("g g v e cmd-c escape 0 cmd-v");
    assert_eq!(rig.text(cx), "worldworld\nnext");
    assert_eq!(rig.mode(cx), Mode::Normal);
    // After the paste the caret is back on a character.
    let head = rig.head(cx);
    assert!(head.offset < "worldworld".len());
}

#[gpui::test]
fn shifted_arrows_start_visual_mode(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["hello world"]), cx);
    // Left to the editor by the engine: its selection becomes Visual mode.
    cx.simulate_keystrokes("shift-right shift-right");
    assert_eq!(rig.mode(cx), Mode::Visual);
    assert_eq!(rig.selected_text(cx), "he");
    cx.simulate_keystrokes("d");
    assert_eq!(rig.text(cx), "llo world");
    assert_eq!(rig.mode(cx), Mode::Normal);
}

#[gpui::test]
fn option_chords_keep_their_editor_meaning(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["first", "second"]), cx);
    // Move block down.
    cx.simulate_keystrokes("alt-down");
    assert_eq!(rig.text(cx), "second\nfirst");
    assert_eq!(rig.mode(cx), Mode::Normal);
}

#[gpui::test]
fn platform_text_is_refused_outside_insert_mode(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["hello"]), cx);
    // Dictation, the emoji palette or an input method would call the
    // platform input handler directly.
    rig.editor.update_in(cx, |editor, window, cx| {
        editor.replace_text_in_range(None, "dictated", window, cx);
        editor.replace_and_mark_text_in_range(None, "ni", None, window, cx);
    });
    cx.run_until_parked();
    assert_eq!(rig.text(cx), "hello");
    for mode_keys in ["v", "escape d", "escape :"] {
        cx.simulate_keystrokes(mode_keys);
        assert!(!rig.text_input(cx), "after {mode_keys}");
    }
    cx.simulate_keystrokes("escape i");
    assert!(rig.text_input(cx));
    cx.simulate_keystrokes("escape");
    assert!(!rig.text_input(cx));
}

#[gpui::test]
fn detaching_restores_plain_editing(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["hello"]), cx);
    cx.simulate_keystrokes("v l");
    assert_eq!(rig.mode(cx), Mode::Visual);
    rig.take_events();

    rig.vim.update(cx, |vim, cx| vim.detach(cx));
    cx.run_until_parked();
    assert!(!rig.vim.read_with(cx, |vim, _| vim.is_attached()));
    assert!(!rig.status(cx).attached);
    assert_eq!(rig.caret_style(cx), CaretStyle::Bar);
    assert!(rig.text_input(cx));
    assert_eq!(rig.context_mode(cx), None);
    assert_eq!(rig.caret_override(cx), None);
    assert!(rig.selection(cx).is_collapsed());
    assert!(rig.take_events().contains(&VimEvent::Detached));

    // Keys are text again.
    cx.simulate_keystrokes("cmd-right");
    cx.simulate_input("jk dd");
    assert_eq!(rig.text(cx), "hellojk dd");
    cx.simulate_keystrokes("escape");
    assert_eq!(rig.text(cx), "hellojk dd");

    // And back on: Normal mode again, the same engine (registers survive).
    cx.simulate_keystrokes("cmd-left");
    rig.vim.update(cx, |vim, cx| vim.toggle(cx));
    cx.run_until_parked();
    assert_eq!(rig.mode(cx), Mode::Normal);
    assert_eq!(rig.caret_style(cx), CaretStyle::Block);
    cx.simulate_keystrokes("x");
    assert_eq!(rig.text(cx), "ellojk dd");
}

#[gpui::test]
fn detaching_in_insert_mode_ends_the_undo_step(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["x"]), cx);
    cx.simulate_keystrokes("shift-a");
    cx.simulate_input(" one");
    rig.vim.update(cx, |vim, cx| vim.detach(cx));
    cx.run_until_parked();
    assert!(rig.text_input(cx));
    cx.simulate_input("two");
    assert_eq!(rig.text(cx), "x onetwo");
    // The Insert session was closed as its own undo step: one undo takes
    // back only what was typed after the detach.
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(rig.text(cx), "x one");
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(rig.text(cx), "x");
}

#[gpui::test]
fn an_open_menu_gets_escape_before_vim(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["hello"]), cx);
    cx.simulate_keystrokes("i");
    assert_eq!(rig.mode(cx), Mode::Insert);

    // What the insert menu does while it is open.
    let closed = std::rc::Rc::new(std::cell::Cell::new(0));
    let menu = rig.editor.read_with(cx, |editor, _| {
        editor.intercept_keys(InterceptPriority::OVERLAY, {
            let closed = closed.clone();
            move |key, _, _| {
                if key.key == "escape" {
                    closed.set(closed.get() + 1);
                    KeyDisposition::Consumed
                } else {
                    KeyDisposition::Pass
                }
            }
        })
    });
    cx.simulate_keystrokes("escape");
    assert_eq!(closed.get(), 1);
    assert_eq!(rig.mode(cx), Mode::Insert);
    // Printable keys still reach the document through Vim's Insert mode.
    cx.simulate_input("ab");
    assert_eq!(rig.text(cx), "abhello");

    drop(menu);
    cx.simulate_keystrokes("escape");
    assert_eq!(closed.get(), 1);
    assert_eq!(rig.mode(cx), Mode::Normal);
}

#[gpui::test]
fn changes_from_outside_are_synced(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["hello world", "second"]), cx);

    // A caret put after the last character (a click past the line end).
    rig.set_caret(at(0, 11), cx);
    assert_eq!(rig.head(cx), at(0, 10));
    assert_eq!(rig.mode(cx), Mode::Normal);

    // A selection made by something else (find, a mouse drag).
    rig.editor.update(cx, |editor, cx| {
        editor.edit(
            |state| state.set_selection(Selection::new(at(0, 0), at(0, 5))),
            cx,
        );
    });
    cx.run_until_parked();
    assert_eq!(rig.mode(cx), Mode::Visual);
    assert_eq!(rig.cursor(cx), at(0, 4));
    assert_eq!(rig.caret_override(cx), Some(at(0, 4)));
    cx.simulate_keystrokes("y");
    assert_eq!(rig.mode(cx), Mode::Normal);
    rig.vim.read_with(cx, |vim, _| {
        assert_eq!(vim.engine().register('"').unwrap().text(), "hello");
    });

    // A pending command is dropped when the caret moves under it.
    cx.simulate_keystrokes("d");
    assert_eq!(rig.mode(cx), Mode::OperatorPending);
    rig.set_caret(at(1, 2), cx);
    assert_eq!(rig.mode(cx), Mode::Normal);
    assert_eq!(rig.status(cx).pending_keys, "");
    cx.simulate_keystrokes("x");
    assert_eq!(rig.text(cx), "hello world\nseond");
}

#[gpui::test]
fn a_replaced_document_starts_over_in_normal_mode(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["old text"]), cx);
    cx.simulate_keystrokes("y y i");
    cx.simulate_input("typing ");
    assert_eq!(rig.mode(cx), Mode::Insert);

    rig.editor.update(cx, |editor, cx| {
        editor.set_document(doc(&["new document", "line two"]), cx);
    });
    cx.run_until_parked();
    assert_eq!(rig.mode(cx), Mode::Normal);
    assert!(!rig.text_input(cx));
    assert_eq!(rig.caret_style(cx), CaretStyle::Block);
    assert_eq!(rig.head(cx), at(0, 0));

    // Keys are commands on the new document; registers survived.
    cx.simulate_keystrokes("j p");
    assert_eq!(rig.text(cx), "new document\nline two\nold text");
    // The Insert session of the old document left no open undo group: this
    // undo takes back the put alone.
    cx.simulate_keystrokes("u");
    assert_eq!(rig.text(cx), "new document\nline two");
}

#[gpui::test]
fn a_read_only_editor_refuses_changes(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["hello world"]), cx);
    rig.editor
        .update(cx, |editor, cx| editor.set_read_only(true, cx));
    cx.run_until_parked();

    cx.simulate_keystrokes("x");
    assert_eq!(rig.text(cx), "hello world");
    let message = rig.status(cx).message.expect("an error");
    assert!(message.is_error);
    assert!(message.text.starts_with("E21"), "{}", message.text);

    // Moving and yanking still work, and the next command clears the error.
    cx.simulate_keystrokes("w");
    assert_eq!(rig.head(cx), at(0, 6));
    assert_eq!(rig.status(cx).message, None);
    cx.simulate_keystrokes("y w");
    rig.vim.read_with(cx, |vim, _| {
        assert_eq!(vim.engine().register('"').unwrap().text(), "world");
    });

    rig.editor
        .update(cx, |editor, cx| editor.set_read_only(false, cx));
    cx.simulate_keystrokes("x");
    assert_eq!(rig.text(cx), "hello orld");
}

fn click(cx: &mut VisualTestContext, at: Point<Pixels>, count: usize) {
    cx.simulate_event(MouseDownEvent {
        position: at,
        modifiers: Modifiers::none(),
        button: MouseButton::Left,
        click_count: count,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        position: at,
        modifiers: Modifiers::none(),
        button: MouseButton::Left,
        click_count: count,
    });
    cx.run_until_parked();
}

#[gpui::test]
fn the_mouse_works_in_normal_mode(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["hello world", "second line"]), cx);
    let point_of = |cx: &mut VisualTestContext, block, offset| {
        rig.editor.read_with(cx, |editor, _| {
            editor.position_bounds(at(block, offset)).unwrap().center()
        })
    };

    // A click inside a word: the caret is on that character.
    let target = point_of(cx, 1, 3);
    click(cx, target, 1);
    assert_eq!(rig.head(cx), at(1, 3));
    assert_eq!(rig.mode(cx), Mode::Normal);

    // A click past the end of a line: on its last character, not after it.
    let target = point_of(cx, 0, 11) + point(px(40.), px(0.));
    click(cx, target, 1);
    assert_eq!(rig.head(cx), at(0, 10));
    cx.simulate_keystrokes("x");
    assert_eq!(rig.text(cx), "hello worl\nsecond line");

    // A double click selects a word: Visual mode, ready for an operator.
    let target = point_of(cx, 1, 2);
    click(cx, target, 2);
    assert_eq!(rig.mode(cx), Mode::Visual);
    assert_eq!(rig.selected_text(cx), "second");
    cx.simulate_keystrokes("c");
    assert_eq!(rig.mode(cx), Mode::Insert);
    cx.simulate_input("first");
    cx.simulate_keystrokes("escape");
    assert_eq!(rig.text(cx), "hello worl\nfirst line");

    // In Insert mode a click just moves the caret, also past the line end.
    cx.simulate_keystrokes("i");
    let target = point_of(cx, 0, 10) + point(px(40.), px(0.));
    click(cx, target, 1);
    assert_eq!(rig.mode(cx), Mode::Insert);
    assert_eq!(rig.head(cx), at(0, 10));
    cx.simulate_input("d");
    assert_eq!(rig.text(cx), "hello world\nfirst line");
}

#[gpui::test]
fn a_key_that_types_nothing_is_not_reported_as_typed(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["abc"]), cx);
    cx.simulate_keystrokes("i");
    cx.simulate_input("x");
    // Something swallows the next key after Vim let it through (a binding,
    // a lower interceptor): no text arrives for it.
    let swallow = rig.editor.read_with(cx, |editor, _| {
        editor.intercept_keys(InterceptPriority::DEFAULT, |_, _, _| {
            KeyDisposition::Consumed
        })
    });
    cx.simulate_input("y");
    drop(swallow);
    cx.simulate_input("z");
    cx.simulate_keystrokes("escape");
    assert_eq!(rig.text(cx), "xzabc");
    rig.vim.read_with(cx, |vim, _| {
        assert_eq!(vim.engine().last_inserted_text(), "xz");
    });
}

/// A key press as the platform reports it: the key by its Latin position
/// and the character the layout types for it.
fn press(cx: &mut VisualTestContext, key: &str, typed: &str, modifiers: Modifiers) {
    cx.update(|window, cx| {
        window.dispatch_keystroke(
            Keystroke {
                modifiers,
                key: key.into(),
                key_char: Some(typed.into()),
            },
            cx,
        );
    });
    cx.run_until_parked();
}

#[gpui::test]
fn commands_work_on_other_keyboard_layouts(cx: &mut TestAppContext) {
    setup(cx);
    let (rig, cx) = open(doc(&["один два", "три {x} y"]), cx);

    // A Russian layout: the "j" key types "о", "w" types "ц", "x" types "ч".
    press(cx, "j", "о", Modifiers::none());
    assert_eq!(rig.head(cx).block, 1);
    press(cx, "w", "ц", Modifiers::none());
    assert_eq!(rig.head(cx), at(1, 7));
    // A German layout: option-8 types "{".
    press(cx, "$", "$", Modifiers::none());
    press(
        cx,
        "8",
        "{",
        Modifiers {
            alt: true,
            ..Modifiers::none()
        },
    );
    assert_eq!(rig.head(cx).block, 0, "the paragraph motion ran");

    // Where a character is wanted, the typed one counts: `f` + "д", a
    // search, and of course Insert mode.
    press(cx, "f", "а", Modifiers::none());
    press(cx, "l", "д", Modifiers::none());
    assert_eq!(rig.head(cx), at(0, "о".len()));
    press(cx, "/", "/", Modifiers::none());
    press(cx, "n", "т", Modifiers::none());
    assert_eq!(rig.status(cx).command_line.unwrap().text, "т");
    cx.simulate_keystrokes("enter");
    assert_eq!(rig.head(cx), at(1, 0));
    press(cx, "i", "ш", Modifiers::none());
    assert_eq!(rig.mode(cx), Mode::Insert);
    press(cx, "j", "о", Modifiers::none());
    cx.simulate_keystrokes("escape");
    assert_eq!(rig.text(cx), "один два\nотри {x} y");
    rig.vim.read_with(cx, |vim, _| {
        assert_eq!(vim.engine().last_inserted_text(), "о");
    });
}
