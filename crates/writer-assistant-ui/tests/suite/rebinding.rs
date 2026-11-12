//! A rebuilt keymap: the user's own shortcuts reach GPUI by clearing its
//! keymap and binding it again with other keystrokes
//! (`writer_ui::keymap::replace_key` does that for one key). The chats obey
//! and show the new keys at once. A key of a chat is bound in every place
//! of the chat it has to work in (the panel, its message list, its
//! editors), and each of those bindings is replaced.

use gpui::{Action, TestAppContext, VisualTestContext};
use writer_assistant::FakeReply;
use writer_assistant_ui::{
    AnswerAction, ClearChat, Composer, CopyAnswer, NewChat, PanelArea, StopAnswer,
};
use writer_editor::actions::Newline;
use writer_ui::{KeyHint, keymap};

use super::support::{Focus, bounds, open_chat, open_panel};

fn hint(action: &dyn Action, cx: &mut VisualTestContext) -> Option<String> {
    cx.update(|_, cx| KeyHint::for_action(action, cx).map(|hint| hint.text()))
}

fn rebind(action: &dyn Action, from: &str, to: &str, cx: &mut VisualTestContext) -> usize {
    let changed = cx.update(|_, cx| keymap::replace_key(action, from, Some(to), cx));
    cx.run_until_parked();
    changed
}

/// Every place of the keymap `action` is bound in.
fn places(action: &dyn Action, cx: &mut VisualTestContext) -> Vec<String> {
    cx.update(|_, cx| {
        cx.key_bindings()
            .borrow()
            .bindings_for_action(action)
            .map(|binding| {
                binding
                    .predicate()
                    .map(|predicate| predicate.to_string())
                    .unwrap_or_default()
            })
            .collect()
    })
}

#[gpui::test]
fn the_key_that_sends_is_the_one_of_a_new_paragraph(cx: &mut TestAppContext) {
    let (chat, cx) = open_chat(cx);
    let send_key =
        |cx: &mut VisualTestContext| cx.update(|_, cx| Composer::send_key(cx).map(|k| k.text()));
    assert_eq!(send_key(cx).as_deref(), Some("↩"));

    assert_eq!(rebind(&Newline, "enter", "ctrl-j", cx), 1);
    assert_eq!(send_key(cx).as_deref(), Some("⌃J"));
    chat.fake.reply("An answer.");
    cx.simulate_input("A question");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(chat.ids(cx).is_empty(), "the old key sends nothing");
    cx.simulate_keystrokes("ctrl-j");
    cx.run_until_parked();
    assert_eq!(chat.ids(cx).len(), 2);
}

#[gpui::test]
fn the_keys_of_the_side_chat_that_were_changed(cx: &mut TestAppContext) {
    let (ws, cx) = open_panel(cx);
    ws.fake.reply("One.");
    ws.send("First", cx);
    assert_eq!(ws.texts(cx).len(), 2);

    // Clear: the header's control names the key, the key does it.
    assert_eq!(hint(&ClearChat, cx).as_deref(), Some("⌥⌘K"));
    assert_eq!(rebind(&ClearChat, "cmd-alt-k", "cmd-alt-l", cx), 1);
    assert_eq!(hint(&ClearChat, cx).as_deref(), Some("⌥⌘L"));
    cx.simulate_keystrokes("cmd-alt-k");
    cx.run_until_parked();
    assert_eq!(ws.texts(cx).len(), 2, "the old key clears nothing");
    cx.simulate_keystrokes("cmd-alt-l");
    cx.run_until_parked();
    assert!(ws.texts(cx).is_empty());

    assert_eq!(rebind(&NewChat, "cmd-alt-n", "cmd-alt-j", cx), 1);
    assert_eq!(hint(&NewChat, cx).as_deref(), Some("⌥⌘J"));

    // The key of an answer's action is asked when it is shown.
    let copy = AnswerAction::new("copy", "Copy").key_of(CopyAnswer);
    let copy_key =
        |cx: &mut VisualTestContext| cx.update(|_, cx| copy.key_hint(cx).map(|k| k.text()));
    assert_eq!(copy_key(cx).as_deref(), Some("⇧⌘C"));
    assert_eq!(rebind(&CopyAnswer, "cmd-shift-c", "cmd-alt-y", cx), 1);
    assert_eq!(copy_key(cx).as_deref(), Some("⌥⌘Y"));
}

/// "Stop the answer" is bound three times: in the panel, in the message
/// list and in the editors of the chat (where `cmd-.` would be the editor's
/// otherwise). The new key works in each of those places, the old one in
/// none.
#[gpui::test]
fn a_key_bound_in_every_place_of_the_chat(cx: &mut TestAppContext) {
    let (ws, cx) = open_panel(cx);
    assert_eq!(places(&StopAnswer, cx).len(), 3);
    assert_eq!(hint(&StopAnswer, cx).as_deref(), Some("⌘."));
    assert_eq!(rebind(&StopAnswer, "cmd-.", "ctrl-c", cx), 3);
    assert_eq!(places(&StopAnswer, cx).len(), 3);
    assert_eq!(hint(&StopAnswer, cx).as_deref(), Some("⌃C"));

    // In the composer.
    ws.fake.hold();
    ws.fake.push(FakeReply::pieces(["First ", "second."]));
    ws.send("Go", cx);
    ws.advance_to_text("First ", cx);
    assert!(matches!(ws.focus(cx), Focus::Panel(PanelArea::Composer)));
    cx.simulate_keystrokes("cmd-.");
    cx.run_until_parked();
    assert_eq!(ws.fake.stops(), 0, "the old key stops nothing");
    cx.simulate_keystrokes("ctrl-c");
    cx.run_until_parked();
    assert_eq!(ws.fake.stops(), 1);
    bounds("assistant-send", cx);

    // In the message list.
    ws.fake.push(FakeReply::pieces(["More ", "words."]));
    ws.send("Again", cx);
    ws.advance_to_text("More ", cx);
    ws.panel.update_in(cx, |panel, window, cx| {
        panel.focus_area(PanelArea::Messages, window, cx)
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("cmd-.");
    cx.run_until_parked();
    assert_eq!(ws.fake.stops(), 1, "the old key stops nothing");
    cx.simulate_keystrokes("ctrl-c");
    cx.run_until_parked();
    assert_eq!(ws.fake.stops(), 2);
}
