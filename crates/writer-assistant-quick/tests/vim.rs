//! The quick chat over an editor with Vim mode attached.

use gpui::{AppContext, Entity, TestAppContext, VisualTestContext};
use writer_assistant::quick_prompt;
use writer_vim_mode::{Mode, VimMode};

use crate::support::*;

fn with_vim(cx: &mut TestAppContext) -> (Fixture, Entity<VimMode>, &mut VisualTestContext) {
    let (fx, cx) = open_quick(
        paragraphs(&["The quick brown fox", "jumps over", "the lazy dog"]),
        cx,
    );
    let editor = fx.editor.clone();
    let vim = cx.new(|cx| VimMode::new(editor, cx));
    vim.update(cx, |vim, cx| vim.set_attached(true, cx));
    cx.run_until_parked();
    caret(&fx.editor, 0, 4, cx);
    (fx, vim, cx)
}

fn mode(vim: &Entity<VimMode>, cx: &mut VisualTestContext) -> Mode {
    vim.read_with(cx, |vim, _| vim.mode())
}

const TEXT: &str = "The quick brown fox\njumps over\nthe lazy dog";

#[gpui::test]
fn from_normal_mode_typing_goes_to_the_composer(cx: &mut TestAppContext) {
    let (fx, vim, cx) = with_vim(cx);
    assert_eq!(mode(&vim, cx), Mode::Normal);

    fx.toggle(cx);
    assert!(fx.is_open(cx));
    assert!(fx.composer_focused(cx));
    // Nothing is selected in Normal mode: the question goes alone.
    assert_eq!(fx.quick.read_with(cx, |quick, _| quick.quote()), None);

    // Keys that are commands in Normal mode are letters here.
    cx.simulate_input("dd x :q");
    cx.run_until_parked();
    assert_eq!(fx.draft(cx), "dd x :q");
    assert_eq!(text(&fx.editor, cx), TEXT);
    assert_eq!(mode(&vim, cx), Mode::Normal);

    fx.fake.reply("An answer.");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(fx.fake.sent_texts(), vec!["dd x :q"]);
    assert_eq!(text(&fx.editor, cx), TEXT);

    // escape closes the popover; it does not reach Vim.
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(!fx.is_open(cx));
    assert!(fx.editor_focused(cx));
    assert_eq!(mode(&vim, cx), Mode::Normal);
    assert!(selection(&fx.editor, cx).is_empty());

    // Back in the text the keys are commands again.
    cx.simulate_keystrokes("x");
    cx.run_until_parked();
    assert_eq!(
        text(&fx.editor, cx),
        "The uick brown fox\njumps over\nthe lazy dog"
    );
}

#[gpui::test]
fn from_visual_mode_the_visual_selection_is_the_text(cx: &mut TestAppContext) {
    let (fx, vim, cx) = with_vim(cx);
    cx.simulate_keystrokes("v e");
    cx.run_until_parked();
    assert_eq!(mode(&vim, cx), Mode::Visual);
    assert_eq!(selected_text(&fx.editor, cx), "quick");

    fx.toggle(cx);
    assert!(fx.composer_focused(cx));
    assert_eq!(
        fx.quick.read_with(cx, |quick, _| quick.quote()).as_deref(),
        Some("quick")
    );
    fx.fake.reply("swift");
    fx.send("Another word?", cx);
    assert_eq!(
        fx.fake.sent_texts(),
        vec![quick_prompt("Another word?", Some("quick"))]
    );

    // Closing without a choice: Visual mode with the selection as it was.
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(!fx.is_open(cx));
    assert!(fx.editor_focused(cx));
    assert_eq!(mode(&vim, cx), Mode::Visual);
    assert_eq!(selected_text(&fx.editor, cx), "quick");
    assert_eq!(text(&fx.editor, cx), TEXT);

    // And escape leaves Visual mode, as it does.
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert_eq!(mode(&vim, cx), Mode::Normal);
    assert!(selection(&fx.editor, cx).is_empty());
}

#[gpui::test]
fn replacing_from_visual_mode_leaves_a_consistent_mode(cx: &mut TestAppContext) {
    let (fx, vim, cx) = with_vim(cx);
    cx.simulate_keystrokes("v e");
    cx.run_until_parked();
    fx.toggle(cx);
    fx.fake.reply("swift");
    fx.send("Another word?", cx);

    cx.simulate_keystrokes("cmd-shift-enter");
    cx.run_until_parked();
    assert_eq!(
        text(&fx.editor, cx),
        "The swift brown fox\njumps over\nthe lazy dog"
    );
    assert!(!fx.is_open(cx));
    assert!(fx.editor_focused(cx));
    // The changed text is selected, and Vim calls that what it is.
    assert_eq!(selected_text(&fx.editor, cx), "swift");
    assert_eq!(mode(&vim, cx), Mode::Visual);

    // A Vim command acts on it; one undo takes the replacement back.
    cx.simulate_keystrokes("escape u");
    cx.run_until_parked();
    assert_eq!(mode(&vim, cx), Mode::Normal);
    assert_eq!(text(&fx.editor, cx), TEXT);
}

#[gpui::test]
fn inserting_from_normal_mode_stays_in_normal_mode(cx: &mut TestAppContext) {
    let (fx, vim, cx) = with_vim(cx);
    fx.toggle(cx);
    fx.fake.reply("A new line.");
    fx.send("Write a line.", cx);

    cx.simulate_keystrokes("cmd-enter");
    cx.run_until_parked();
    assert_eq!(
        text(&fx.editor, cx),
        "The quick brown fox\nA new line.\njumps over\nthe lazy dog"
    );
    assert!(!fx.is_open(cx));
    assert!(fx.editor_focused(cx));
    // Either mode is fine as long as the editor agrees with it: Visual
    // has a selection, Normal has none.
    let now = mode(&vim, cx);
    let range = selection(&fx.editor, cx);
    match now {
        Mode::Normal => assert!(range.is_empty(), "Normal mode with a selection"),
        Mode::Visual | Mode::VisualLine => {
            assert_eq!(selected_text(&fx.editor, cx), "A new line.")
        }
        other => panic!("left in {other:?}"),
    }
    assert_eq!(range.start.block, 1, "the caret is in what was inserted");

    // Vim goes on working: `dd` removes the inserted line.
    cx.simulate_keystrokes("escape d d");
    cx.run_until_parked();
    assert_eq!(text(&fx.editor, cx), TEXT);
    assert_eq!(mode(&vim, cx), Mode::Normal);
}

#[gpui::test]
fn a_half_typed_command_does_not_wait_behind_the_popover(cx: &mut TestAppContext) {
    let (fx, vim, cx) = with_vim(cx);
    // `d` waits for its motion when the popover opens.
    cx.simulate_keystrokes("d");
    cx.run_until_parked();
    fx.toggle(cx);
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(!fx.is_open(cx));
    assert_eq!(mode(&vim, cx), Mode::Normal);
    // `w` moves; it does not finish a `dw`.
    cx.simulate_keystrokes("w");
    cx.run_until_parked();
    assert_eq!(text(&fx.editor, cx), TEXT);
}
