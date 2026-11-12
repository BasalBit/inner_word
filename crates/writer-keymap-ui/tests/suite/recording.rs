//! Changing a key: the view waits for the next keystroke, which reaches it
//! before anything else; what it says about the key; what the user then
//! chooses.

use gpui::{KeyDownEvent, Keystroke, Modifiers, ModifiersChangedEvent, TestAppContext};
use writer_keymap_ui::{Choice, ModeKind, Request, Tone};

use super::*;

fn mode(view: &Entity<KeymapView>, cx: &mut VisualTestContext) -> ModeKind {
    view.read_with(cx, |view, _| view.mode())
}

fn choices(view: &Entity<KeymapView>, cx: &mut VisualTestContext) -> Vec<String> {
    view.read_with(cx, |view, _| {
        view.choices().into_iter().map(|(_, label)| label).collect()
    })
}

fn title(view: &Entity<KeymapView>, cx: &mut VisualTestContext) -> String {
    view.read_with(cx, |view, _| view.question_title().unwrap_or_default())
}

fn mark(view: &Entity<KeymapView>, cx: &mut VisualTestContext) -> Tone {
    view.read_with(cx, |view, _| {
        view.question_mark().expect("a question is open")
    })
}

fn marked(view: &Entity<KeymapView>, cx: &mut VisualTestContext) -> Choice {
    view.read_with(cx, |view, _| {
        view.marked_choice().expect("a question is open")
    })
}

fn says(view: &Entity<KeymapView>, cx: &mut VisualTestContext) -> String {
    view.read_with(cx, |view, _| {
        view.question_lines()
            .iter()
            .map(|line| line.text.clone())
            .collect::<Vec<_>>()
            .join(" ")
    })
}

#[gpui::test]
fn enter_waits_for_a_key_shows_it_and_gives_it_on_confirmation(cx: &mut TestAppContext) {
    let (host, view, cx) = open(cx);
    select(&view, &bold(), cx);
    press(cx, "enter");
    assert_eq!(mode(&view, cx), ModeKind::Recording);
    assert_eq!(
        view.read_with(cx, |view, _| view.recording_target().cloned()),
        Some(bold())
    );
    assert!(describe(&view, cx).contains("for=\"Bold\" replacing=cmd-b"));
    assert!(requests(&host, cx).is_empty());

    press(cx, "cmd-alt-b");
    assert_eq!(mode(&view, cx), ModeKind::Question);
    view.read_with(cx, |view, _| {
        assert_eq!(view.question_title().as_deref(), Some("⌥⌘B is free"));
        let lines = view.question_lines();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].tone, Tone::Good);
        assert_eq!(view.marked_choice(), Some(Choice::UseKey));
    });
    assert_eq!(
        choices(&view, cx),
        vec!["Use ⌥⌘B", "Press Another Key", "Cancel"]
    );
    // Nothing is changed before the user says so.
    assert!(requests(&host, cx).is_empty());
    assert_eq!(keys_of(&host, &bold(), cx), vec!["cmd-b"]);

    press(cx, "enter");
    assert_eq!(mode(&view, cx), ModeKind::List);
    assert_eq!(
        requests(&host, cx),
        vec![Request::ReplaceKey {
            target: bold(),
            old: key("cmd-b"),
            new: key("cmd-alt-b"),
            take: Vec::new(),
        }]
    );
    // The host changed the keymap and handed the new state back: the row
    // shows the new key, is marked as changed, and stays selected.
    view.read_with(cx, |view, cx| {
        assert_eq!(view.selected_target(), Some(&bold()));
        assert_eq!(view.selected_keys(), vec![key("cmd-alt-b")]);
        assert_eq!(view.changed_count(), 1);
        let line = view.describe(cx);
        assert!(
            line.contains("keys=[cmd-alt-b] default=[cmd-b] (changed)"),
            "{line}"
        );
    });

    // And the new key is the one that runs the command, the old one not.
    press(cx, "escape");
    assert!(!host.read_with(cx, |host, cx| host.is_open(cx)));
    press(cx, "cmd-b");
    assert_eq!(host.read_with(cx, |host, _| host.bolds), 0);
    press(cx, "cmd-alt-b");
    assert_eq!(host.read_with(cx, |host, _| host.bolds), 1);
}

#[gpui::test]
fn no_binding_runs_and_nothing_is_typed_while_a_key_is_recorded(cx: &mut TestAppContext) {
    let (host, view, cx) = open(cx);
    select(&view, &bold(), cx);
    // Each of these does something everywhere else: the window's own keys,
    // the key that closes the view, the keys of the list, a character.
    for (pressed, shown) in [
        ("cmd-n", "⌘N"),
        ("cmd-q", "⌘Q"),
        ("cmd-/", "⌘/"),
        ("escape", "Esc"),
        ("enter", "↩"),
        ("tab", "⇥"),
        ("down", "↓"),
        ("cmd-i", "⌘I"),
        ("cmd-shift-d", "⇧⌘D"),
        ("x", "X"),
        ("space", "Space"),
        ("backspace", "⌫"),
    ] {
        view.update_in(cx, |view, window, cx| {
            assert!(view.change_key(window, cx));
        });
        assert_eq!(mode(&view, cx), ModeKind::Recording, "{pressed}");
        press(cx, pressed);
        assert_eq!(mode(&view, cx), ModeKind::Question, "{pressed} is recorded");
        let title = view.read_with(cx, |view, _| view.question_title().unwrap());
        assert!(title.starts_with(shown), "{pressed}: {title}");
        view.update_in(cx, |view, window, cx| view.cancel(window, cx));
    }
    host.read_with(cx, |host, cx| {
        assert_eq!(host.new_documents, 0, "cmd-n did not make a document");
        assert_eq!(host.quits, 0, "cmd-q did not quit");
        assert!(host.is_open(cx), "neither escape nor cmd-/ closed the view");
        assert!(host.requests.is_empty(), "nothing was asked for");
    });
    view.read_with(cx, |view, cx| {
        assert_eq!(view.query(cx), "", "nothing was typed into the query");
        assert_eq!(
            view.selected_target(),
            Some(&bold()),
            "the selection did not move"
        );
        assert!(!view.changed_only());
    });
    // The same keys do what they do once the view is not waiting.
    press(cx, "cmd-n");
    assert_eq!(host.read_with(cx, |host, _| host.new_documents), 1);
}

#[gpui::test]
fn a_real_key_event_is_taken_too(cx: &mut TestAppContext) {
    // What the platform sends: a key-down event through the window's event
    // path, with the character the key types.
    let (host, view, cx) = open(cx);
    select(&view, &bold(), cx);
    press(cx, "enter");
    cx.simulate_event(KeyDownEvent {
        keystroke: Keystroke {
            modifiers: Modifiers::none(),
            key: "k".into(),
            key_char: Some("k".into()),
        },
        is_held: false,
    });
    cx.run_until_parked();
    assert_eq!(mode(&view, cx), ModeKind::Question);
    assert_eq!(view.read_with(cx, |view, cx| view.query(cx)), "");
    // A bare character where text is typed: refused.
    assert_eq!(
        view.read_with(cx, |view, _| view.question_title().unwrap()),
        "K cannot be used"
    );
    drop(host);
}

#[gpui::test]
fn only_a_real_key_ends_the_waiting(cx: &mut TestAppContext) {
    let (host, view, cx) = open(cx);
    select(&view, &bold(), cx);
    press(cx, "enter");
    // Modifiers alone are not a key yet: the view shows them and waits.
    cx.simulate_event(ModifiersChangedEvent {
        modifiers: Modifiers {
            platform: true,
            shift: true,
            ..Modifiers::none()
        },
        capslock: Default::default(),
    });
    cx.run_until_parked();
    assert_eq!(mode(&view, cx), ModeKind::Recording);
    assert!(
        describe(&view, cx).contains("held=⇧⌘"),
        "{}",
        describe(&view, cx)
    );
    cx.simulate_keystrokes("shift");
    cx.simulate_keystrokes("platform");
    assert_eq!(mode(&view, cx), ModeKind::Recording);
    cx.simulate_event(ModifiersChangedEvent {
        modifiers: Modifiers::none(),
        capslock: Default::default(),
    });

    // The pointer leaves the waiting with its Cancel control.
    click(cx, "keymap-record-cancel");
    assert_eq!(mode(&view, cx), ModeKind::List);
    assert!(requests(&host, cx).is_empty());

    // From the keys: any key, and then the key that cancels the question.
    press(cx, "enter");
    assert_eq!(mode(&view, cx), ModeKind::Recording);
    press(cx, "escape");
    assert_eq!(
        mode(&view, cx),
        ModeKind::Question,
        "escape was recorded, not obeyed"
    );
    press(cx, "escape");
    assert_eq!(mode(&view, cx), ModeKind::List);
    assert!(host.read_with(cx, |host, cx| host.is_open(cx)));
    assert!(requests(&host, cx).is_empty());
    // The keyboard is back in the query field.
    cx.simulate_input("ital");
    assert_eq!(view.read_with(cx, |view, cx| view.query(cx)), "ital");
}

#[gpui::test]
fn a_key_that_is_held_down_is_not_taken_twice(cx: &mut TestAppContext) {
    let (host, view, cx) = open(cx);
    select(&view, &bold(), cx);
    // `enter` starts the waiting and is still down: its repeats are not
    // the new key.
    cx.simulate_keystrokes("enter");
    assert_eq!(mode(&view, cx), ModeKind::Recording);
    cx.simulate_keystrokes("enter enter enter");
    assert_eq!(
        mode(&view, cx),
        ModeKind::Recording,
        "the repeats of the key that started it"
    );
    // Released and pressed again, it is a key like any other.
    cx.simulate_event(gpui::KeyUpEvent {
        keystroke: Keystroke::parse("enter").unwrap(),
    });
    cx.simulate_keystrokes("enter");
    assert_eq!(mode(&view, cx), ModeKind::Question);
    assert!(
        view.read_with(cx, |view, _| view.question_title().unwrap())
            .starts_with("↩ is "),
        "enter was recorded"
    );
    // Still down: its repeats do not answer the question either.
    cx.simulate_keystrokes("enter enter");
    assert_eq!(mode(&view, cx), ModeKind::Question);
    assert!(requests(&host, cx).is_empty());
    press(cx, "escape");
    assert_eq!(mode(&view, cx), ModeKind::List);

    // A key recorded with the command key held: the release of the letter
    // is not reported on a Mac, the change of the modifiers is.
    cx.simulate_keystrokes("enter");
    cx.simulate_event(gpui::KeyUpEvent {
        keystroke: Keystroke::parse("enter").unwrap(),
    });
    cx.simulate_keystrokes("cmd-n");
    assert_eq!(mode(&view, cx), ModeKind::Question);
    cx.simulate_keystrokes("cmd-n cmd-n");
    assert_eq!(
        host.read_with(cx, |host, _| host.new_documents),
        0,
        "the repeats run nothing"
    );
    cx.simulate_event(ModifiersChangedEvent {
        modifiers: Modifiers::none(),
        capslock: Default::default(),
    });
    // Another key is never held back.
    press(cx, "escape");
    assert_eq!(mode(&view, cx), ModeKind::List);

    // The key that answers a question is not a key of the list while it is
    // still down: a held `enter` does not start changing the key again.
    press(cx, "enter f5");
    cx.simulate_keystrokes("enter");
    assert_eq!(mode(&view, cx), ModeKind::List, "the answer was taken");
    assert_eq!(keys_of(&host, &bold(), cx), vec!["f5"]);
    cx.simulate_keystrokes("enter enter");
    assert_eq!(mode(&view, cx), ModeKind::List, "its repeats do nothing");
    cx.simulate_event(gpui::KeyUpEvent {
        keystroke: Keystroke::parse("enter").unwrap(),
    });

    // A script's keys come without a release; it says so itself.
    cx.simulate_keystrokes("enter");
    view.update(cx, |view, _| view.key_released());
    cx.simulate_keystrokes("enter");
    assert_eq!(mode(&view, cx), ModeKind::Question);
}

#[gpui::test]
fn a_key_that_would_stop_typing_is_refused_with_the_reason(cx: &mut TestAppContext) {
    let (host, view, cx) = open(cx);
    select(&view, &bold(), cx);
    press(cx, "enter b");
    view.read_with(cx, |view, _| {
        assert_eq!(view.question_title().as_deref(), Some("B cannot be used"));
        let lines = view.question_lines();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].tone, Tone::Stop);
        assert!(
            lines[0].text.contains("types a character"),
            "{}",
            lines[0].text
        );
        // Nothing to use: press another key, or cancel.
        assert_eq!(view.marked_choice(), Some(Choice::PressAnotherKey));
    });
    assert_eq!(choices(&view, cx), vec!["Press Another Key", "Cancel"]);

    // The marked choice goes back to waiting; shift with a character is
    // refused the same way.
    press(cx, "enter");
    assert_eq!(mode(&view, cx), ModeKind::Recording);
    press(cx, "shift-b");
    assert_eq!(
        view.read_with(cx, |view, _| view.question_title().unwrap()),
        "⇧B cannot be used"
    );
    // `backspace` goes back to waiting as well.
    press(cx, "backspace");
    assert_eq!(mode(&view, cx), ModeKind::Recording);
    // A key macOS keeps for itself: the heading says who has it, the
    // sentence what it does there.
    press(cx, "cmd-tab");
    assert_eq!(title(&view, cx), "macOS keeps ⌘⇥");
    assert_eq!(mark(&view, cx), Tone::Stop);
    assert!(
        says(&view, cx).starts_with("In macOS ⌘⇥ switches between apps"),
        "{}",
        says(&view, cx)
    );
    assert_eq!(choices(&view, cx), vec!["Press Another Key", "Cancel"]);
    // The key the command has already: not "cannot be used", it is used.
    press(cx, "backspace cmd-b");
    assert_eq!(title(&view, cx), "“Bold” has ⌘B already");
    assert_eq!(mark(&view, cx), Tone::Plain);
    assert_eq!(says(&view, cx), "Nothing would change.");
    assert_eq!(choices(&view, cx), vec!["Press Another Key", "Cancel"]);
    press(cx, "escape");
    assert!(requests(&host, cx).is_empty());
    assert_eq!(keys_of(&host, &bold(), cx), vec!["cmd-b"]);
}

#[gpui::test]
fn a_taken_key_names_its_command_and_can_be_taken_from_it(cx: &mut TestAppContext) {
    let (host, view, cx) = open(cx);
    select(&view, &bold(), cx);
    press(cx, "enter cmd-i");
    view.read_with(cx, |view, _| {
        assert_eq!(view.question_title().as_deref(), Some("⌘I is taken"));
        let lines = view.question_lines();
        assert_eq!(lines[0].tone, Tone::Stop);
        assert_eq!(
            lines[0].text,
            "⌘I is the key of “Italic” in the text. Only one of the two can have it there."
        );
        // The view's own Import has the key too, in a place of its own:
        // said, and nothing to settle.
        assert!(
            lines[1].text.contains("“Import” in keyboard shortcuts"),
            "{}",
            lines[1].text
        );
        assert!(lines[1].text.ends_with("both work."));
        assert_eq!(view.marked_choice(), Some(Choice::TakeKey));
    });
    assert_eq!(
        choices(&view, cx),
        vec!["Take It from “Italic”", "Press Another Key", "Cancel"]
    );

    // Cancel first: nothing changes.
    press(cx, "escape");
    assert!(requests(&host, cx).is_empty());

    // Again, and take it.
    press(cx, "enter cmd-i enter");
    assert_eq!(keys_of(&host, &bold(), cx), vec!["cmd-i"]);
    assert!(
        keys_of(&host, &italic(), cx).is_empty(),
        "Italic is left without a key"
    );
    // The command in the place of its own kept it.
    assert_eq!(
        keys_of(
            &host,
            &Target::new(
                "keymap_view::Import",
                Some(writer_keymap_ui::LIST_KEY_CONTEXT)
            ),
            cx
        ),
        vec!["cmd-i"]
    );
    match requests(&host, cx).as_slice() {
        [
            Request::ReplaceKey {
                target, new, take, ..
            },
        ] => {
            assert_eq!(*target, bold());
            assert_eq!(*new, key("cmd-i"));
            assert_eq!(take.len(), 1);
            assert_eq!(take[0].target, italic());
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(view.read_with(cx, |view, _| view.changed_count()), 2);
}

#[gpui::test]
fn a_key_of_an_overlapping_place_can_be_kept_by_both_or_taken(cx: &mut TestAppContext) {
    let (host, view, cx) = open(cx);
    let comments = Target::new("demo::ToggleComments", None);
    // The window has cmd-alt-c for the comments; Bold applies in the text,
    // which is inside the window.
    select(&view, &bold(), cx);
    press(cx, "enter cmd-alt-c");
    view.read_with(cx, |view, _| {
        // Not "in another place": the heading says what is the matter,
        // the sentence under it where the other command applies.
        assert_eq!(
            view.question_title().as_deref(),
            Some("⌥⌘C is also the key of another command")
        );
        assert_eq!(view.question_mark(), Some(Tone::Plain));
        assert_eq!(view.marked_choice(), Some(Choice::KeepBoth));
    });
    assert_eq!(
        choices(&view, cx),
        vec![
            "Keep Both",
            "Take It from “Show or hide comments”",
            "Press Another Key",
            "Cancel"
        ]
    );
    let text = says(&view, cx);
    assert!(text.contains("“Show or hide comments” anywhere"), "{text}");
    assert!(text.contains("Where both apply"), "{text}");

    // Keep both.
    press(cx, "enter");
    assert_eq!(keys_of(&host, &bold(), cx), vec!["cmd-alt-c"]);
    assert_eq!(keys_of(&host, &comments, cx), vec!["cmd-alt-c"]);

    // Back, and this time take it: the second choice, reached with the
    // arrow.
    press(cx, "cmd-d");
    assert_eq!(keys_of(&host, &bold(), cx), vec!["cmd-b"]);
    press(cx, "enter cmd-alt-c right");
    assert_eq!(
        view.read_with(cx, |view, _| view.marked_choice()),
        Some(Choice::TakeKey)
    );
    press(cx, "left right tab shift-tab");
    assert_eq!(
        view.read_with(cx, |view, _| view.marked_choice()),
        Some(Choice::TakeKey)
    );
    press(cx, "enter");
    assert_eq!(keys_of(&host, &bold(), cx), vec!["cmd-alt-c"]);
    assert!(keys_of(&host, &comments, cx).is_empty());
}

/// A warning does not stand in the way, and it is not passed over
/// either: the heading says it (never "is free", never a tick), and the
/// key that confirms does not give the key. Giving it takes one step.
#[gpui::test]
fn a_warning_is_the_heading_and_confirming_does_not_pass_it_over(cx: &mut TestAppContext) {
    let (host, view, cx) = open(cx);
    select(&view, &bold(), cx);
    // Option with a character types a special character on a Mac.
    press(cx, "enter alt-b");
    assert_eq!(title(&view, cx), "⌥B types a special character");
    assert_eq!(mark(&view, cx), Tone::Warning);
    view.read_with(cx, |view, _| {
        let lines = view.question_lines();
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert_eq!(lines[0].tone, Tone::Warning);
        assert!(lines[0].text.contains("special character"), "{lines:?}");
    });
    assert_eq!(
        choices(&view, cx),
        vec!["Use ⌥B", "Press Another Key", "Cancel"]
    );
    assert_eq!(marked(&view, cx), Choice::PressAnotherKey);
    // The mark is drawn beside the heading.
    assert!(cx.debug_bounds("keymap-question-mark").is_some());
    let line = describe(&view, cx);
    assert!(
        line.contains(
            "question=\"⌥B types a special character\" mark=warning says=\"On a Mac keyboard"
        ),
        "{line}"
    );
    assert!(
        line.contains("choices=[Use ⌥B | *Press Another Key | Cancel]"),
        "{line}"
    );

    // Confirming goes back to waiting; nothing was given.
    press(cx, "enter");
    assert_eq!(mode(&view, cx), ModeKind::Recording);
    assert!(requests(&host, cx).is_empty());
    assert_eq!(keys_of(&host, &bold(), cx), vec!["cmd-b"]);

    // One step to the left, and it is given all the same.
    press(cx, "alt-b left");
    assert_eq!(marked(&view, cx), Choice::UseKey);
    press(cx, "enter");
    assert_eq!(keys_of(&host, &bold(), cx), vec!["alt-b"]);
}

/// A key macOS uses unless that was switched off (the picture of 0.5.0:
/// "⌘Space is free" above "the shortcut may not run").
#[gpui::test]
fn a_key_macos_uses_is_not_called_free(cx: &mut TestAppContext) {
    let (host, view, cx) = open(cx);
    select(&view, &bold(), cx);
    press(cx, "enter cmd-space");
    assert_eq!(title(&view, cx), "macOS uses ⌘Space");
    assert_eq!(mark(&view, cx), Tone::Warning);
    assert_eq!(
        says(&view, cx),
        "In macOS ⌘Space opens Spotlight. Unless that is switched off in System Settings, the \
         shortcut may not run."
    );
    assert_eq!(
        choices(&view, cx),
        vec!["Use ⌘Space", "Press Another Key", "Cancel"]
    );
    assert_eq!(marked(&view, cx), Choice::PressAnotherKey);
    press(cx, "enter");
    assert_eq!(mode(&view, cx), ModeKind::Recording);
    assert!(requests(&host, cx).is_empty());
    // With the pointer, on the button that says it.
    press(cx, "cmd-space");
    click(cx, "keymap-choice-0");
    assert_eq!(keys_of(&host, &bold(), cx), vec!["cmd-space"]);
}

/// Return and Esc are recorded like any key. Somebody who presses Return
/// again to "confirm" must not end up with Return as the key of Bold: a
/// key text is edited with is warned about, and the marked choice is not
/// the one that gives it.
#[gpui::test]
fn return_pressed_again_does_not_give_return_to_the_command(cx: &mut TestAppContext) {
    let (host, view, cx) = open(cx);
    select(&view, &bold(), cx);
    press(cx, "enter enter");
    assert_eq!(mode(&view, cx), ModeKind::Question);
    assert!(title(&view, cx).starts_with("↩ "), "{}", title(&view, cx));
    assert_ne!(mark(&view, cx), Tone::Good);
    assert!(
        says(&view, cx).contains("As a shortcut, ↩ would run the command"),
        "{}",
        says(&view, cx)
    );
    assert_eq!(marked(&view, cx), Choice::PressAnotherKey);
    // Again and again: waiting, question, waiting. Nothing is given.
    press(cx, "enter enter enter enter");
    assert!(requests(&host, cx).is_empty());
    assert_eq!(keys_of(&host, &bold(), cx), vec!["cmd-b"]);
    press(cx, "escape escape");
    assert_eq!(mode(&view, cx), ModeKind::List);
}

/// A command of the window given a key the text has for something of its
/// own: in the text the other command keeps the key, so the new one would
/// not work there.
#[gpui::test]
fn a_key_that_would_not_work_everywhere_says_so(cx: &mut TestAppContext) {
    let (host, view, cx) = open(cx);
    select(&view, &version_history(), cx);
    press(cx, "enter cmd-b");
    assert_eq!(title(&view, cx), "⌘B would not work everywhere");
    assert_eq!(mark(&view, cx), Tone::Warning);
    let text = says(&view, cx);
    assert!(
        text.contains("“Bold” in the text. Where both apply “Bold” keeps it"),
        "{text}"
    );
    assert_eq!(
        choices(&view, cx),
        vec![
            "Keep Both",
            "Take It from “Bold”",
            "Press Another Key",
            "Cancel"
        ]
    );
    assert_eq!(marked(&view, cx), Choice::PressAnotherKey);
    // Both ways of giving it are there for whoever wants them.
    press(cx, "left left");
    assert_eq!(marked(&view, cx), Choice::KeepBoth);
    press(cx, "enter");
    assert_eq!(keys_of(&host, &version_history(), cx), vec!["cmd-b"]);
    assert_eq!(keys_of(&host, &bold(), cx), vec!["cmd-b"]);
}

/// A key that only commands in places of their own have is free where
/// this command applies, and the heading says where.
#[gpui::test]
fn a_key_used_only_in_other_places_is_free_here(cx: &mut TestAppContext) {
    let (host, view, cx) = open(cx);
    select(&view, &bold(), cx);
    // The view's own Export has ⌘E, in the list of keyboard shortcuts.
    press(cx, "enter cmd-e");
    assert_eq!(title(&view, cx), "⌘E is free in the text");
    assert_eq!(mark(&view, cx), Tone::Good);
    let text = says(&view, cx);
    assert!(
        text.contains("“Export” in keyboard shortcuts") && text.ends_with("both work."),
        "{text}"
    );
    assert_eq!(
        choices(&view, cx),
        vec!["Use ⌘E", "Press Another Key", "Cancel"]
    );
    assert_eq!(marked(&view, cx), Choice::UseKey);
    press(cx, "enter");
    assert_eq!(keys_of(&host, &bold(), cx), vec!["cmd-e"]);
}

#[gpui::test]
fn every_choice_of_the_question_works_from_the_keys(cx: &mut TestAppContext) {
    let (host, view, cx) = open(cx);
    select(&view, &bold(), cx);
    press(cx, "enter cmd-alt-b");
    // The arrows and tab go round the choices.
    let marked =
        |cx: &mut VisualTestContext| view.read_with(cx, |view, _| view.marked_choice().unwrap());
    assert_eq!(marked(cx), Choice::UseKey);
    press(cx, "right");
    assert_eq!(marked(cx), Choice::PressAnotherKey);
    press(cx, "tab");
    assert_eq!(marked(cx), Choice::Cancel);
    press(cx, "right");
    assert_eq!(marked(cx), Choice::UseKey, "round to the first");
    press(cx, "left");
    assert_eq!(marked(cx), Choice::Cancel);
    // `enter` takes the marked one: cancel.
    press(cx, "enter");
    assert_eq!(mode(&view, cx), ModeKind::List);
    assert!(requests(&host, cx).is_empty());

    // "Press Another Key", marked and taken.
    press(cx, "enter cmd-alt-b right enter");
    assert_eq!(mode(&view, cx), ModeKind::Recording);
    press(cx, "cmd-alt-u enter");
    assert_eq!(keys_of(&host, &bold(), cx), vec!["cmd-alt-u"]);
}

/// What the keymap cannot know: something of the host's (Vim mode in the
/// text) may take a key before any binding sees it. The host says what it
/// is called, where it is and which keys it leaves alone; the question
/// about any other key for a command there has it as its heading (the
/// picture of 0.5.0: "⌃Y is free" with a tick above the note), and the
/// key that confirms does not give the key.
#[gpui::test]
fn the_question_says_when_something_of_the_host_sees_the_key_first(cx: &mut TestAppContext) {
    fn leaves(key: &KeySequence) -> bool {
        key.first().modifiers().command
    }
    let warning = "In the text it sees this key before any shortcut does.";
    let (host, view, cx) = open(cx);
    let listing = host
        .read_with(cx, |host, _| host.listing())
        .sees_keys_first("Vim mode", ["Editor"], leaves, warning);
    view.update_in(cx, |view, window, cx| view.set_listing(listing, window, cx));
    cx.run_until_parked();
    let ask = |target: &Target, keys: &str, cx: &mut VisualTestContext| {
        select(&view, target, cx);
        press(cx, "enter");
        press(cx, keys);
        let asked = view.read_with(cx, |view, _| {
            (
                view.question_title().unwrap(),
                view.question_mark().unwrap(),
                view.question_lines(),
                view.marked_choice().unwrap(),
            )
        });
        press(cx, "escape");
        asked
    };

    // A command of that place, a key that is not left alone.
    let (title, mark, lines, marked) = ask(&bold(), "ctrl-y", cx);
    assert_eq!(title, "⌃Y: Vim mode sees it first");
    assert_eq!(mark, Tone::Warning);
    // The host's sentence and nothing above it: no "No other command has
    // ⌃Y" with a tick.
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(
        (lines[0].tone, lines[0].text.as_str()),
        (Tone::Warning, warning)
    );
    assert_eq!(marked, Choice::PressAnotherKey);
    // A command that works anywhere is concerned too.
    let (title, _, lines, marked) = ask(&version_history(), "ctrl-y", cx);
    assert_eq!(title, "⌃Y: Vim mode sees it first");
    assert!(lines.iter().any(|line| line.text == warning));
    assert_eq!(marked, Choice::PressAnotherKey);
    // A key that is left to the keymap: free, with its tick, and confirmed
    // at once.
    let (title, mark, lines, marked) = ask(&bold(), "cmd-alt-y", cx);
    assert_eq!(title, "⌥⌘Y is free");
    assert_eq!(mark, Tone::Good);
    assert!(lines.iter().all(|line| line.text != warning), "{lines:?}");
    assert_eq!(marked, Choice::UseKey);
    // A key that is refused anyway: only the refusal.
    let (title, _, lines, _) = ask(&bold(), "y", cx);
    assert_eq!(title, "Y cannot be used");
    assert!(lines.iter().all(|line| line.text != warning), "{lines:?}");
    // A command of another place: nothing.
    let elsewhere = host.read_with(cx, |host, _| {
        host.keymap
            .defaults()
            .targets()
            .iter()
            .find(|target| {
                target
                    .context
                    .text()
                    .is_some_and(|context| context != "Editor")
            })
            .cloned()
            .expect("the host has a command outside the text")
    });
    let listed = view.read_with(cx, |view, _| view.row_of(&elsewhere).is_some());
    if listed {
        let (title, _, lines, _) = ask(&elsewhere, "ctrl-y", cx);
        assert!(!title.contains("Vim"), "{title}");
        assert!(lines.iter().all(|line| line.text != warning), "{lines:?}");
    }

    // Confirming does not give such a key; the choice that does is there.
    select(&view, &bold(), cx);
    press(cx, "enter ctrl-y enter");
    assert_eq!(mode(&view, cx), ModeKind::Recording);
    assert!(requests(&host, cx).is_empty());
    press(cx, "ctrl-y");
    assert!(view.update_in(cx, |view, window, cx| view.choose(
        Choice::UseKey,
        window,
        cx
    )));
    cx.run_until_parked();
    assert_eq!(keys_of(&host, &bold(), cx), vec!["ctrl-y"]);

    // A host that does not say what it is called: the heading says what
    // may happen.
    let listing = host
        .read_with(cx, |host, _| host.listing())
        .takes_keys_first(["Editor"], leaves, warning);
    view.update_in(cx, |view, window, cx| view.set_listing(listing, window, cx));
    cx.run_until_parked();
    let (title, mark, _, marked) = ask(&italic(), "ctrl-t", cx);
    assert_eq!(title, "⌃T may not run");
    assert_eq!(mark, Tone::Warning);
    assert_eq!(marked, Choice::PressAnotherKey);
}

/// A command the keymap holds in several key contexts of one place is one
/// command where the user is told how many are theirs: the question of
/// "reset all" counts as the list does.
#[gpui::test]
fn a_command_in_several_key_contexts_is_counted_once(cx: &mut TestAppContext) {
    use writer_keymap::Catalogue;
    let (host, view, cx) = open(cx);
    // Two targets that are one command to the user: one row, one name.
    let mut listing = host.read_with(cx, |host, _| host.listing());
    assert!(
        listing
            .keymap
            .replace_key(&bold(), &key("cmd-b"), key("f5"))
    );
    assert!(
        listing
            .keymap
            .replace_key(&italic(), &key("cmd-i"), key("f5"))
    );
    listing.rows.retain(|row| row.target != italic());
    let mut catalogue = Catalogue::new();
    catalogue.add(bold(), "Emphasis", "In the text");
    catalogue.add(italic(), "Emphasis", "In the text");
    for row in listing.rows.iter_mut().filter(|row| row.target == bold()) {
        row.title = "Emphasis".into();
    }
    let listing = listing.catalogue(catalogue);
    assert_eq!(listing.keymap.effective().changed().count(), 2);
    assert_eq!(listing.changed_commands(), 1);
    view.update_in(cx, |view, window, cx| view.set_listing(listing, window, cx));
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |view, _| view.changed_count()), 1);
    assert!(view.update_in(cx, |view, window, cx| view.reset_all(window, cx)));
    cx.run_until_parked();
    assert!(
        says(&view, cx).starts_with("1 command goes back"),
        "{}",
        says(&view, cx)
    );
}
