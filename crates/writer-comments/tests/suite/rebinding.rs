//! A rebuilt keymap: the user's own shortcuts reach GPUI by clearing its
//! keymap and binding it again with other keystrokes
//! (`writer_ui::keymap::replace_key` does that for one key). The comments
//! obey and show the new keys at once: the keys that work anywhere, the
//! keys of the thread list, and the two keys the composer takes from its
//! editor before the editor's own bindings.

use gpui::{Action, TestAppContext, VisualTestContext};
use writer_comments::{
    AddComment, CancelComment, EditComment, ReplyToThread, SelectNextThread, SubmitComment,
    ToggleShowResolved,
};
use writer_ui::{KeyHint, keymap};

use super::support::*;

fn hint(action: &dyn Action, cx: &mut VisualTestContext) -> Option<String> {
    cx.update(|_, cx| KeyHint::for_action(action, cx).map(|hint| hint.text()))
}

fn rebind(action: &dyn Action, from: &str, to: &str, cx: &mut VisualTestContext) -> usize {
    let changed = cx.update(|_, cx| keymap::replace_key(action, from, Some(to), cx));
    cx.run_until_parked();
    changed
}

#[gpui::test]
fn the_key_that_starts_a_comment(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    f.select(0, 4, 9, cx);
    assert_eq!(hint(&AddComment, cx).as_deref(), Some("⌥⌘M"));
    assert_eq!(rebind(&AddComment, "cmd-alt-m", "cmd-alt-n", cx), 1);

    cx.simulate_keystrokes("cmd-alt-m");
    assert_eq!(f.focus(cx), Focus::Editor, "the old key starts nothing");
    f.panel
        .read_with(cx, |panel, _| assert!(!panel.is_composing()));
    cx.simulate_keystrokes("cmd-alt-n");
    assert_eq!(f.focus(cx), Focus::Composer);
    assert_eq!(hint(&AddComment, cx).as_deref(), Some("⌥⌘N"));
}

/// The composer takes the keys of "save" and "discard" from its editor
/// before the editor's own bindings (the editor has `cmd-enter` itself).
/// Which keys those are is the keymap's answer.
#[gpui::test]
fn the_composer_obeys_the_keys_of_save_and_discard_as_they_are(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    assert_eq!(hint(&SubmitComment, cx).as_deref(), Some("⌘↩"));
    assert_eq!(hint(&CancelComment, cx).as_deref(), Some("Esc"));
    assert_eq!(rebind(&SubmitComment, "cmd-enter", "cmd-s", cx), 1);
    assert_eq!(rebind(&CancelComment, "escape", "ctrl-g", cx), 1);
    assert_eq!(hint(&SubmitComment, cx).as_deref(), Some("⌘S"));
    assert_eq!(hint(&CancelComment, cx).as_deref(), Some("⌃G"));

    // Discard.
    f.select(1, 2, 8, cx);
    cx.simulate_keystrokes("cmd-alt-m");
    cx.simulate_input("Never mind");
    cx.simulate_keystrokes("escape");
    assert_eq!(f.focus(cx), Focus::Composer, "the old key discards nothing");
    assert_eq!(f.composer_text(cx), "Never mind");
    cx.simulate_keystrokes("ctrl-g");
    assert_eq!(f.focus(cx), Focus::Editor);
    assert_eq!(f.comment_count(cx), 0);
    f.panel
        .read_with(cx, |panel, _| assert!(!panel.is_composing()));

    // Save.
    cx.simulate_keystrokes("cmd-alt-m");
    cx.simulate_input("Tighten this");
    cx.simulate_keystrokes("cmd-enter");
    assert_eq!(f.comment_count(cx), 0, "the old key saves nothing");
    assert_eq!(f.focus(cx), Focus::Composer);
    assert_eq!(
        f.composer_text(cx),
        "Tighten this",
        "and the editor's own key on it does nothing in a composer"
    );
    cx.simulate_keystrokes("cmd-s");
    assert_eq!(f.comment_count(cx), 1);
    assert_eq!(f.focus(cx), Focus::Editor);
    f.panel.read_with(cx, |panel, _| {
        assert_eq!(panel.threads()[0].thread.body, "Tighten this");
    });
}

/// A key the editor binds itself can be given to "save": the composer's
/// interceptor comes before the editor's bindings for whatever key the
/// keymap names, as it did for `cmd-enter`.
#[gpui::test]
fn a_key_of_the_editor_given_to_save_wins_in_the_composer(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    // `cmd-b` is bold in an editor, and the composer is one.
    assert_eq!(rebind(&SubmitComment, "cmd-enter", "cmd-b", cx), 1);
    f.select(0, 4, 9, cx);
    cx.simulate_keystrokes("cmd-alt-m");
    cx.simulate_input("Bolder");
    cx.simulate_keystrokes("cmd-b");
    assert_eq!(f.comment_count(cx), 1);
    // In the text the key is still the editor's.
    assert_eq!(f.focus(cx), Focus::Editor);
    f.select(2, 0, 3, cx);
    cx.simulate_keystrokes("cmd-b");
    f.editor.read_with(cx, |editor, _| {
        assert!(editor.document().blocks()[2].attrs_at(0).unwrap().bold);
    });
    assert_eq!(f.comment_count(cx), 1);
}

#[gpui::test]
fn the_keys_of_the_thread_list(cx: &mut TestAppContext) {
    let (f, cx) = open(sample(), cx);
    let first = f.comment(0, 4, 9, "First", cx);
    let second = f.comment(1, 2, 8, "Second", cx);
    cx.simulate_keystrokes("cmd-2");
    assert_eq!(f.focus(cx), Focus::List);
    f.panel
        .update(cx, |panel, cx| panel.select_thread(Some(first), cx));
    cx.run_until_parked();

    // One of the two keys of an action, the other stays.
    assert_eq!(rebind(&SelectNextThread, "down", "ctrl-j", cx), 1);
    cx.simulate_keystrokes("down");
    assert_eq!(f.active(cx), Some(first), "the old key moves nothing");
    cx.simulate_keystrokes("ctrl-j");
    assert_eq!(f.active(cx), Some(second));
    cx.simulate_keystrokes("k j");
    assert_eq!(f.active(cx), Some(second));
    assert_eq!(hint(&SelectNextThread, cx).as_deref(), Some("⌃J"));

    // A single letter of the list.
    assert_eq!(rebind(&ToggleShowResolved, "s", "d", cx), 1);
    let shown =
        |cx: &mut VisualTestContext| f.panel.read_with(cx, |panel, _| panel.show_resolved());
    cx.simulate_keystrokes("s");
    assert!(!shown(cx), "the old key switches nothing");
    cx.simulate_keystrokes("d");
    assert!(shown(cx));
    assert_eq!(hint(&ToggleShowResolved, cx).as_deref(), Some("D"));
    cx.simulate_keystrokes("d");
    assert!(!shown(cx));

    // The key hints of the active card are read when it is drawn.
    assert_eq!(hint(&ReplyToThread, cx).as_deref(), Some("↩"));
    assert_eq!(rebind(&ReplyToThread, "enter", "ctrl-r", cx), 1);
    assert_eq!(rebind(&EditComment, "e", "f2", cx), 1);
    assert_eq!(hint(&ReplyToThread, cx).as_deref(), Some("⌃R"));
    assert_eq!(hint(&EditComment, cx).as_deref(), Some("F2"));
    cx.simulate_keystrokes("enter e");
    assert_eq!(f.focus(cx), Focus::List, "the old keys open no composer");
    cx.simulate_keystrokes("f2");
    assert_eq!(f.focus(cx), Focus::Composer);
    assert_eq!(f.composer_text(cx), "Second");
}
