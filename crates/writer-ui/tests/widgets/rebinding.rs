//! A rebuilt keymap: the user's own shortcuts reach GPUI by clearing its
//! keymap and binding it again with other keystrokes. The widgets obey and
//! show the new keys at once, and nothing in them still knows the old ones.

use gpui::{Action, KeyBinding, TestAppContext, VisualTestContext, point, px};
use writer_ui::{
    ContextMenu, ContextMenuItem, KeyHint, TextFieldEvent,
    context_menu::{self, OpenContextMenu},
    keymap, picker, text_field,
};

use super::{DOCS, Names, field_window, picker_window, root_window, selected_label};

fn hint(action: &dyn Action, cx: &mut VisualTestContext) -> Option<String> {
    cx.update(|_, cx| KeyHint::for_action(action, cx).map(|hint| hint.text()))
}

#[gpui::test]
fn a_text_field_obeys_and_shows_a_rebuilt_keymap(cx: &mut TestAppContext) {
    let (_field, cx, events) = field_window(cx, "x");
    events.borrow_mut().clear();
    assert_eq!(hint(&text_field::Submit, cx).as_deref(), Some("↩"));

    let changed =
        cx.update(|_, cx| keymap::replace_key(&text_field::Submit, "enter", Some("ctrl-s"), cx));
    assert_eq!(changed, 1);
    cx.run_until_parked();

    // (Without a binding the test platform hands `enter` to the text input
    // as a line break, which a single-line field takes as a space.)
    let submitted = |events: &[TextFieldEvent]| {
        events
            .iter()
            .filter(|event| **event == TextFieldEvent::Submitted)
            .count()
    };
    cx.simulate_keystrokes("enter");
    assert_eq!(
        submitted(&events.borrow()),
        0,
        "the old key submits no more"
    );
    cx.simulate_keystrokes("ctrl-s");
    assert_eq!(submitted(&events.borrow()), 1);
    assert_eq!(hint(&text_field::Submit, cx).as_deref(), Some("⌃S"));

    // And back: the keymap as it was.
    cx.update(|_, cx| keymap::replace_key(&text_field::Submit, "ctrl-s", Some("enter"), cx));
    cx.simulate_keystrokes("enter");
    assert_eq!(submitted(&events.borrow()), 2);
    assert_eq!(hint(&text_field::Submit, cx).as_deref(), Some("↩"));
}

#[gpui::test]
fn a_picker_obeys_and_shows_a_rebuilt_keymap(cx: &mut TestAppContext) {
    let (picker, cx, dismiss_events) = picker_window(cx, Names::new(DOCS));
    assert_eq!(selected_label(&picker, cx).as_deref(), Some("Chapter one"));
    assert_eq!(hint(&picker::SelectNext, cx).as_deref(), Some("↓"));

    cx.update(|_, cx| {
        assert_eq!(
            keymap::replace_key(&picker::SelectNext, "down", Some("ctrl-j"), cx),
            1
        );
        assert_eq!(
            keymap::replace_key(&picker::Dismiss, "escape", Some("ctrl-g"), cx),
            1
        );
    });
    cx.run_until_parked();

    cx.simulate_keystrokes("ctrl-j");
    assert_eq!(selected_label(&picker, cx).as_deref(), Some("Chapter two"));
    cx.simulate_keystrokes("down");
    assert_eq!(
        selected_label(&picker, cx).as_deref(),
        Some("Chapter two"),
        "the old key moves no more"
    );
    // The second key of the action was not touched.
    cx.simulate_keystrokes("ctrl-n");
    assert_eq!(
        selected_label(&picker, cx).as_deref(),
        Some("Notes on style")
    );
    assert_eq!(hint(&picker::SelectNext, cx).as_deref(), Some("⌃J"));

    cx.simulate_keystrokes("escape");
    assert_eq!(dismiss_events.get(), 0, "the old key closes no more");
    cx.simulate_keystrokes("ctrl-g");
    assert_eq!(dismiss_events.get(), 1);
}

#[gpui::test]
fn a_key_can_be_taken_from_an_action(cx: &mut TestAppContext) {
    let (picker, cx, _) = picker_window(cx, Names::new(DOCS));
    cx.update(|_, cx| {
        assert_eq!(
            keymap::replace_key(&picker::SelectNext, "down", None, cx),
            1
        );
        assert_eq!(
            keymap::replace_key(&picker::SelectNext, "ctrl-n", None, cx),
            1
        );
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("down ctrl-n");
    assert_eq!(selected_label(&picker, cx).as_deref(), Some("Chapter one"));
    assert_eq!(hint(&picker::SelectNext, cx), None);
    // The bindings around it stand where they stood.
    cx.simulate_keystrokes("end");
    assert_eq!(selected_label(&picker, cx).as_deref(), Some("Outline"));
    cx.simulate_keystrokes("up");
    assert_eq!(
        selected_label(&picker, cx).as_deref(),
        Some("Letter to the editor")
    );
}

#[gpui::test]
fn the_order_of_the_bindings_survives_a_rebuild(cx: &mut TestAppContext) {
    let (_picker, cx, _) = picker_window(cx, Names::new(DOCS));
    let names = |cx: &mut VisualTestContext| {
        cx.update(|_, cx| {
            cx.key_bindings()
                .borrow()
                .bindings()
                .map(|binding| {
                    (
                        binding.action().name(),
                        binding
                            .predicate()
                            .map(|predicate| predicate.to_string())
                            .unwrap_or_default(),
                    )
                })
                .collect::<Vec<_>>()
        })
    };
    let before = names(cx);
    cx.update(|_, cx| keymap::replace_key(&picker::Confirm, "enter", Some("ctrl-y"), cx));
    assert_eq!(names(cx), before);
}

#[gpui::test]
fn the_key_that_opens_menus_closes_an_open_one_whatever_it_is(cx: &mut TestAppContext) {
    let (_field, cx, _) = field_window(cx, "x");
    let open = |cx: &mut VisualTestContext| {
        cx.update(|window, cx| {
            ContextMenu::open(
                point(px(10.), px(10.)),
                vec![ContextMenuItem::action("Select All", text_field::SelectAll)],
                window,
                cx,
            )
        });
        cx.run_until_parked();
    };
    let is_open = |cx: &mut VisualTestContext| {
        cx.update(|window, cx| ContextMenu::current(window, cx).is_some())
    };
    let key = |source: &str| gpui::Keystroke::parse(source).unwrap();

    // The defaults: both keys of the text field's menu.
    for keys in ["ctrl-enter", "shift-f10"] {
        open(cx);
        assert!(is_open(cx));
        cx.simulate_keystrokes(keys);
        assert!(!is_open(cx), "{keys} closes the menu");
    }
    cx.update(|_, cx| {
        assert!(context_menu::opens_a_context_menu(&key("ctrl-enter"), cx));
        assert!(!context_menu::opens_a_context_menu(&key("ctrl-m"), cx));
        assert!(!context_menu::opens_a_context_menu(&key("enter"), cx));
    });

    cx.update(|_, cx| {
        assert_eq!(
            keymap::replace_key(&OpenContextMenu, "ctrl-enter", Some("ctrl-m"), cx),
            1
        );
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        assert!(!context_menu::opens_a_context_menu(&key("ctrl-enter"), cx));
        assert!(context_menu::opens_a_context_menu(&key("ctrl-m"), cx));
    });
    // The new key opens the field's menu and closes it again.
    cx.simulate_keystrokes("ctrl-m");
    assert!(is_open(cx), "the new key opens the menu of the field");
    cx.simulate_keystrokes("ctrl-m");
    assert!(!is_open(cx), "and closes it");
    // The old key is one more key the open menu swallows.
    cx.simulate_keystrokes("ctrl-enter");
    assert!(!is_open(cx), "the old key opens nothing");
    open(cx);
    cx.simulate_keystrokes("ctrl-enter");
    assert!(is_open(cx), "the old key closes nothing");
    // The menu's own keys are not bindings: they work as before.
    cx.simulate_keystrokes("down escape");
    assert!(!is_open(cx));
}

#[gpui::test]
fn a_key_is_asked_of_the_keymap_where_the_keyboard_is(cx: &mut TestAppContext) {
    let (_field, cx, _) = field_window(cx, "x");
    let key = |source: &str| gpui::Keystroke::parse(source).unwrap();
    cx.update(|window, cx| {
        // In the field: its own bindings.
        assert!(keymap::keystroke_runs(
            &key("enter"),
            &text_field::Submit,
            window,
            cx
        ));
        assert!(!keymap::keystroke_runs(
            &key("enter"),
            &text_field::Cancel,
            window,
            cx
        ));
        assert!(!keymap::keystroke_runs(
            &key("ctrl-s"),
            &text_field::Submit,
            window,
            cx
        ));
        // Not what is bound somewhere else.
        assert!(!keymap::keystroke_runs(
            &key("enter"),
            &picker::Confirm,
            window,
            cx
        ));
        // A place given by name.
        let list = [keymap::context("Picker")];
        assert!(keymap::keystroke_runs_in(
            &key("enter"),
            &picker::Confirm,
            &list,
            cx
        ));
        let found = keymap::action_for_keystroke_in(&key("ctrl-n"), &list, cx, |action| {
            action.as_any().is::<picker::SelectNext>()
        });
        assert!(found.is_some());
        let none = keymap::action_for_keystroke_in(&key("ctrl-n"), &list, cx, |action| {
            action.as_any().is::<picker::SelectPrev>()
        });
        assert!(none.is_none());
    });
    cx.update(|_, cx| keymap::replace_key(&text_field::Submit, "enter", Some("ctrl-s"), cx));
    cx.run_until_parked();
    cx.update(|window, cx| {
        assert!(keymap::keystroke_runs(
            &key("ctrl-s"),
            &text_field::Submit,
            window,
            cx
        ));
        assert!(!keymap::keystroke_runs(
            &key("enter"),
            &text_field::Submit,
            window,
            cx
        ));
    });
}

/// An action bound in two places, each with a key that can be changed by
/// itself: a control that belongs to one place names that place's key, not
/// the binding that happens to have been added last.
#[gpui::test]
fn a_hint_is_asked_in_the_place_it_applies(cx: &mut TestAppContext) {
    let (root, cx) = root_window(cx);
    cx.update(|_, cx| {
        cx.bind_keys([
            KeyBinding::new("ctrl-enter", OpenContextMenu, Some("Editor")),
            KeyBinding::new("ctrl-enter", OpenContextMenu, Some("Elsewhere")),
        ])
    });
    cx.run_until_parked();
    let at = |cx: &mut VisualTestContext| {
        cx.update(|window, cx| {
            let focus = root.read(cx).editor_focus.clone();
            (
                KeyHint::for_action_at(&OpenContextMenu, &focus, window, cx).map(|k| k.text()),
                KeyHint::for_action_here(&OpenContextMenu, window, cx).map(|k| k.text()),
                KeyHint::for_action(&OpenContextMenu, cx).map(|k| k.text()),
            )
        })
    };
    let key = |text: &str| Some(text.to_string());
    assert_eq!(at(cx), (key("⌃↩"), key("⌃↩"), key("⌃↩")));

    // The other place gets another key: nothing changes here.
    cx.update(|_, cx| {
        let bindings: Vec<KeyBinding> = cx.key_bindings().borrow().bindings().cloned().collect();
        cx.clear_key_bindings();
        cx.bind_keys(bindings.into_iter().map(|binding| {
            let elsewhere = binding
                .predicate()
                .is_some_and(|predicate| predicate.to_string() == "Elsewhere");
            if elsewhere {
                KeyBinding::new("ctrl-m", OpenContextMenu, Some("Elsewhere"))
            } else {
                binding
            }
        }));
    });
    cx.run_until_parked();
    assert_eq!(
        at(cx),
        (key("⌃↩"), key("⌃↩"), key("⌃M")),
        "the keymap-wide answer is the other place's key"
    );

    // A place without a binding of the action falls back to the keymap.
    cx.update(|_, cx| keymap::replace_key(&OpenContextMenu, "ctrl-enter", None, cx));
    cx.run_until_parked();
    assert_eq!(at(cx), (key("⌃M"), key("⌃M"), key("⌃M")));
}
