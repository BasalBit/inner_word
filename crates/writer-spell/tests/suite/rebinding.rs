//! A rebuilt keymap: the user's own shortcuts reach GPUI by clearing its
//! keymap and binding it again with other keystrokes
//! (`writer_ui::keymap::replace_key` does that for one key). The spelling
//! keys obey it at once, and the commands show the new keys.

use std::rc::Rc;

use gpui::{Action, TestAppContext, VisualTestContext};
use writer_spell::actions::{NextMisspelling, PreviousMisspelling};
use writer_ui::keymap;

use super::support::*;

/// The key the palette and the shortcuts list show for a command of this
/// crate.
fn command_key(title: &str, cx: &mut VisualTestContext) -> Option<String> {
    cx.update(|_, cx| {
        writer_spell::commands()
            .into_iter()
            .find(|command| command.title.as_ref() == title)
            .unwrap_or_else(|| panic!("no command {title:?}"))
            .key_hint(cx)
            .map(|hint| hint.text())
    })
}

fn rebind(action: &dyn Action, from: &str, to: &str, cx: &mut VisualTestContext) -> usize {
    let changed = cx.update(|_, cx| keymap::replace_key(action, from, Some(to), cx));
    cx.run_until_parked();
    changed
}

#[gpui::test]
fn the_keys_of_next_and_previous_misspelling_that_were_changed(cx: &mut TestAppContext) {
    setup(cx);
    let service = Rc::new(english());
    let document = paragraphs(&["The teh ship and a wrod.", "A sentense here."]);
    let (_checker, editor, cx) = open(document, &service, cx);
    caret(&editor, 0, 0, cx);
    let next = command_title(&NextMisspelling, cx);
    let previous = command_title(&PreviousMisspelling, cx);
    assert_eq!(command_key(&next, cx).as_deref(), Some("⌘;"));
    assert_eq!(command_key(&previous, cx).as_deref(), Some("⇧⌘;"));

    assert_eq!(rebind(&NextMisspelling, "cmd-;", "f7", cx), 1);
    cx.simulate_keystrokes("cmd-;");
    assert_eq!(selected_text(&editor, cx), "", "the old key finds nothing");
    cx.simulate_keystrokes("f7");
    assert_eq!(selected_text(&editor, cx), "teh");
    cx.simulate_keystrokes("f7");
    assert_eq!(selected_text(&editor, cx), "wrod");
    assert_eq!(command_key(&next, cx).as_deref(), Some("F7"));

    // The key that is bound in two spellings (what a keyboard reports for
    // shift with a punctuation key, and the readable one): each is a
    // binding of its own.
    assert_eq!(
        rebind(&PreviousMisspelling, "cmd-shift-;", "shift-f7", cx),
        1
    );
    assert_eq!(rebind(&PreviousMisspelling, "cmd-:", "shift-f7", cx), 1);
    cx.simulate_keystrokes("cmd-shift-; cmd-:");
    assert_eq!(selected_text(&editor, cx), "wrod", "the old keys");
    cx.simulate_keystrokes("shift-f7");
    assert_eq!(selected_text(&editor, cx), "teh");
    assert_eq!(command_key(&previous, cx).as_deref(), Some("⇧F7"));
}

/// The title of the command that runs `action`.
fn command_title(action: &dyn Action, cx: &mut VisualTestContext) -> String {
    cx.update(|_, _| {
        writer_spell::commands()
            .into_iter()
            .find(|command| command.action().partial_eq(action))
            .expect("a command for the action")
            .title
            .to_string()
    })
}
