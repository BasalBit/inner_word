//! A rebuilt keymap: the user's own shortcuts reach GPUI by clearing its
//! keymap and binding it again with other keystrokes
//! (`innerword_ui::keymap::replace_key` does that for one key). The quick chat
//! obeys and shows the new keys at once.

use gpui::{Action, TestAppContext, VisualTestContext};
use innerword_assistant_quick::InsertBelow;
use innerword_assistant_ui::{Composer, DismissChat};
use innerword_core::TextRange;
use innerword_editor::actions::Newline;
use innerword_ui::{KeyHint, keymap};

use crate::support::*;

fn hint(action: &dyn Action, cx: &mut VisualTestContext) -> Option<String> {
    cx.update(|_, cx| KeyHint::for_action(action, cx).map(|hint| hint.text()))
}

fn rebind(action: &dyn Action, from: &str, to: &str, cx: &mut VisualTestContext) -> usize {
    let changed = cx.update(|_, cx| keymap::replace_key(action, from, Some(to), cx));
    cx.run_until_parked();
    changed
}

fn opened(cx: &mut TestAppContext) -> (Fixture, &mut VisualTestContext) {
    let (fx, cx) = open_quick(
        paragraphs(&["The quick brown fox", "jumps over", "the lazy dog"]),
        cx,
    );
    select(&fx.editor, TextRange::in_block(0, 4, 9), cx);
    fx.toggle(cx);
    assert!(fx.is_open(cx));
    assert!(fx.composer_focused(cx));
    (fx, cx)
}

#[gpui::test]
fn the_key_that_closes_the_chat(cx: &mut TestAppContext) {
    let (fx, cx) = opened(cx);
    let close_key = |cx: &mut VisualTestContext| {
        fx.quick.update_in(cx, |quick, window, cx| {
            quick.close_key(window, cx).map(|key| key.text())
        })
    };
    assert_eq!(close_key(cx).as_deref(), Some("Esc"));

    // "Close the chat" is bound in the popover here, and by the shared
    // chat pieces in the editors of every chat: each binding is replaced.
    let changed = rebind(&DismissChat, "escape", "ctrl-g", cx);
    assert!(changed >= 2, "{changed} bindings");
    assert_eq!(close_key(cx).as_deref(), Some("⌃G"));
    assert!(fx.is_open(cx), "a rebuilt keymap closes nothing");

    cx.simulate_input("half a quest");
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(fx.is_open(cx), "the old key closes nothing");
    cx.simulate_keystrokes("ctrl-g");
    cx.run_until_parked();
    assert!(!fx.is_open(cx));
    assert!(fx.editor_focused(cx));
    assert_eq!(selected_text(&fx.editor, cx), "quick");
}

#[gpui::test]
fn the_keys_that_send_and_insert(cx: &mut TestAppContext) {
    let (fx, cx) = opened(cx);
    // Sending is the key of a new paragraph in the composer.
    assert_eq!(rebind(&Newline, "enter", "ctrl-j", cx), 1);
    assert_eq!(
        cx.update(|_, cx| Composer::send_key(cx).map(|k| k.text()))
            .as_deref(),
        Some("⌃J")
    );
    fx.fake.reply("swift");
    cx.simulate_input("Another word?");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(fx.fake.sent().is_empty(), "the old key sends nothing");
    cx.simulate_keystrokes("ctrl-j");
    cx.run_until_parked();
    assert_eq!(fx.fake.sent().len(), 1);
    assert_eq!(fx.answers(cx), ["swift"]);

    // Insert Below: bound in the popover and in the editors of the chat,
    // where `cmd-enter` is the editor's own key otherwise.
    assert_eq!(hint(&InsertBelow, cx).as_deref(), Some("⌘↩"));
    assert_eq!(rebind(&InsertBelow, "cmd-enter", "cmd-alt-i", cx), 2);
    assert_eq!(hint(&InsertBelow, cx).as_deref(), Some("⌥⌘I"));
    let before = text(&fx.editor, cx);
    cx.simulate_keystrokes("cmd-enter");
    cx.run_until_parked();
    assert_eq!(text(&fx.editor, cx), before, "the old key inserts nothing");
    cx.simulate_keystrokes("cmd-alt-i");
    cx.run_until_parked();
    assert_eq!(
        text(&fx.editor, cx),
        "The quick brown fox\nswift\njumps over\nthe lazy dog"
    );
}
