//! Keyboard shortcuts the user can change.
//!
//! The first tests are the experiment everything else rests on: the keymap
//! every crate assembled in `innerword_app::init` can be taken apart (keys,
//! action, key context, action input) and put back from those parts, and
//! nothing changes.

use gpui::{
    Action, DummyKeyboardMapper, KeyBinding, KeyBindingContextPredicate, Keystroke, TestAppContext,
};

use super::*;

/// One binding of the keymap, as its parts.
struct Parts {
    keys: String,
    action: Box<dyn Action>,
    predicate: Option<std::rc::Rc<KeyBindingContextPredicate>>,
    meta: Option<gpui::KeyBindingMetaIndex>,
    input: Option<gpui::SharedString>,
}

fn keys_of(binding: &KeyBinding) -> String {
    binding
        .keystrokes()
        .iter()
        .map(|key| key.unparse())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The bindings every crate bound in `init`, as the crates bound them
/// (the app keeps them as the defaults), each taken apart.
fn take_apart(cx: &gpui::App) -> Vec<Parts> {
    innerword_app::keymap::defaults(cx)
        .iter()
        .map(|binding| Parts {
            keys: keys_of(binding),
            action: binding.action().boxed_clone(),
            predicate: binding.predicate(),
            meta: binding.meta(),
            input: binding.action_input(),
        })
        .collect()
}

fn put_together(parts: &Parts) -> KeyBinding {
    let mut binding = KeyBinding::load(
        &parts.keys,
        parts.action.boxed_clone(),
        parts.predicate.clone(),
        false,
        parts.input.clone(),
        &DummyKeyboardMapper,
    )
    .expect("a key the keymap had parses again");
    if let Some(meta) = parts.meta {
        binding.set_meta(meta);
    }
    binding
}

/// Every binding rebuilt from its parts is the binding it was: the same
/// keystrokes, the same action (by value), the same key context, the same
/// meta index and action input, in the same order.
#[gpui::test]
fn a_binding_rebuilt_from_its_parts_is_the_same_binding(cx: &mut TestAppContext) {
    cx.update(|cx| {
        innerword_app::init(cx);
        let parts = take_apart(cx);
        assert!(parts.len() > 300, "the whole keymap: {}", parts.len());

        let mut with_meta = 0;
        let mut with_input = 0;
        let mut chords = 0;
        for (original, parts) in innerword_app::keymap::defaults(cx).iter().zip(&parts) {
            let rebuilt = put_together(parts);
            assert_eq!(
                rebuilt.keystrokes(),
                original.keystrokes(),
                "keys of {}",
                original.action().name()
            );
            assert!(rebuilt.action().partial_eq(original.action()));
            assert_eq!(rebuilt.action().name(), original.action().name());
            assert_eq!(rebuilt.predicate(), original.predicate());
            assert_eq!(rebuilt.meta(), original.meta());
            assert_eq!(rebuilt.action_input(), original.action_input());
            // A key context also survives being written and read as text.
            if let Some(predicate) = original.predicate() {
                let text = predicate.to_string();
                let parsed = KeyBindingContextPredicate::parse(&text)
                    .unwrap_or_else(|error| panic!("{text:?} does not parse again: {error}"));
                assert_eq!(parsed, *predicate, "{text:?}");
            }
            // And every keystroke its own text.
            for key in original.keystrokes() {
                let text = key.unparse();
                let parsed = Keystroke::parse(&text)
                    .unwrap_or_else(|error| panic!("{text:?} does not parse again: {error}"));
                assert_eq!(&parsed, key.inner(), "{text:?}");
            }
            with_meta += usize::from(original.meta().is_some());
            with_input += usize::from(original.action_input().is_some());
            chords += usize::from(original.keystrokes().len() > 1);
        }
        // What the app's keymap has and has not: no binding carries a meta
        // index or an action input (nothing loads a keymap file), and one
        // is a sequence of two keys (`g g` in the library).
        assert_eq!(with_meta, 0);
        assert_eq!(with_input, 0);
        assert_eq!(chords, 1);
    });
}

/// The keymap cleared and bound again from the parts resolves every
/// documented key as before, and the precedence facts hold: a binding
/// without a context still beats the outer contexts, and the find bar's
/// `cmd-alt-c` still beats the window's.
#[gpui::test]
fn the_keymap_put_back_from_its_parts_resolves_every_key_as_before(cx: &mut TestAppContext) {
    init(cx);
    let before: Vec<(String, String, Option<String>)> = cx.update(|cx| {
        let before = describe_keymap(cx);
        // What `init` installed is the defaults behind the bindings that
        // only give menu bar items their key.
        let menu_only = cx
            .key_bindings()
            .borrow()
            .bindings()
            .filter(|binding| innerword_app::menus::is_menu_key_binding(binding))
            .cloned()
            .collect::<Vec<_>>();
        let parts = take_apart(cx);
        // And after them, the fixed bindings that keep GPUI off the
        // whiteboard's own keys.
        let fixed = innerword_app::boards::fixed_key_bindings();
        assert_eq!(before.len(), menu_only.len() + parts.len() + fixed.len());
        cx.clear_key_bindings();
        assert_eq!(cx.key_bindings().borrow().bindings().len(), 0);
        cx.bind_keys(
            menu_only
                .into_iter()
                .chain(parts.iter().map(put_together))
                .chain(fixed),
        );
        before
    });
    let after = cx.update(|cx| describe_keymap(cx));
    assert_eq!(before, after, "the same bindings in the same order");

    let library = library(cx);
    add_document(
        &library,
        None,
        &simple_document("Keys", "A paragraph with a few words in it."),
        cx,
    );
    let (workspace, vcx) = open_workspace(&library, cx);
    assert!(editor_focused(&workspace, vcx));
    keymap::assert_table(keymap::GLOBAL, "the text", vcx);
    keymap::assert_table(keymap::EDITOR, "the text", vcx);

    // The keys run, not only resolve.
    vcx.simulate_keystrokes("cmd-a cmd-b");
    assert!(is_bold(&workspace, "paragraph", vcx));
    // The find bar's own cmd-alt-c, bound later at the focused element's
    // level, wins over the window-wide "show or hide comments".
    vcx.simulate_keystrokes("cmd-f");
    keymap::assert_table(
        &[("cmd-alt-c", "editor_tools::ToggleCaseSensitive")],
        "the find bar",
        vcx,
    );
    vcx.simulate_keystrokes("cmd-alt-c");
    assert!(workspace.read_with(vcx, |workspace, cx| {
        workspace.tools().read(cx).find_options().case_sensitive
    }));
}

/// Whether the word `needle` of the open document is bold.
fn is_bold(workspace: &Entity<Workspace>, needle: &str, cx: &mut VisualTestContext) -> bool {
    workspace.read_with(cx, |workspace, cx| {
        let editor = workspace.editor().read(cx);
        editor
            .document()
            .blocks()
            .iter()
            .find_map(|block| {
                let offset = block.text().find(needle)?;
                Some(block.attrs_at(offset).is_some_and(|attrs| attrs.bold))
            })
            .unwrap_or_else(|| panic!("{needle:?} is not in the document"))
    })
}

/// The keymap as text: keys, action, key context, in order.
fn describe_keymap(cx: &gpui::App) -> Vec<(String, String, Option<String>)> {
    let keymap = cx.key_bindings();
    let keymap = keymap.borrow();
    keymap
        .bindings()
        .map(|binding| {
            (
                keys_of(binding),
                binding.action().name().to_string(),
                binding.predicate().map(|predicate| predicate.to_string()),
            )
        })
        .collect()
}

/// The keys beside the menu bar's items are computed from the keymap that
/// is installed. With the default keys they are the ones the menu bar
/// showed while that list was written by hand (0.4.0).
#[gpui::test]
fn with_the_default_keys_the_menu_bar_shows_what_it_showed_before(cx: &mut TestAppContext) {
    use innerword_app::menus;
    init(cx);
    cx.update(|cx| {
        let by_hand = [
            ("editor_tools::FindPrevious", "cmd-shift-g"),
            ("spell::PreviousMisspelling", "cmd-shift-;"),
            ("editor::ToggleNumberedList", "cmd-shift-7"),
            ("editor::ToggleBulletList", "cmd-shift-8"),
            ("editor::ToggleTodoList", "cmd-shift-9"),
            ("editor::ToggleCodeBlock", "cmd-shift-'"),
            ("editor::IncreaseTextSize", "cmd-shift-."),
            ("editor::DecreaseTextSize", "cmd-shift-,"),
            ("editor::InsertPageBreak", "cmd-enter cmd-enter"),
            ("editor::InsertSoftBreak", "shift-enter shift-enter"),
            (
                "editor::ToggleTodoChecked",
                "cmd-shift-enter cmd-shift-enter",
            ),
            // Since 0.6.0, the zoom: GPUI's own rule would show the key
            // that was bound first (`cmd-=`, and `cmd-)` as a real
            // keyboard reports shift with 0).
            ("zoom::ZoomIn", "cmd-+"),
            ("zoom::ActualSize", "cmd-shift-0"),
        ];
        let context =
            std::rc::Rc::new(KeyBindingContextPredicate::parse(menus::MENU_KEY_CONTEXT).unwrap());
        let old: Vec<KeyBinding> = by_hand
            .iter()
            .map(|(name, keys)| {
                let action = cx.build_action(name, None).expect("a registered action");
                innerword_app::keymap::bind(
                    keys,
                    action.as_ref(),
                    Some(context.clone()),
                    None,
                    None,
                )
                .expect("a key GPUI reads")
            })
            .chain(innerword_app::keymap::defaults(cx).iter().cloned())
            .collect();
        let old = gpui::Keymap::new(old);
        let now = cx.key_bindings();
        let now = now.borrow();
        let key = |keymap: &gpui::Keymap, action: &dyn Action| {
            menus::native_menu_binding(keymap, action).map(keys_of)
        };
        let actions = menus::menu_actions(&menus::app_menus());
        assert!(actions.len() > 90);
        for (name, action) in &actions {
            assert_eq!(
                key(&now, action.as_ref()),
                key(&old, action.as_ref()),
                "the key beside {name}"
            );
        }
        // And no more bindings are made for it than were written by hand.
        let computed: Vec<(String, String)> = now
            .bindings()
            .filter(|binding| menus::is_menu_key_binding(binding))
            .map(|binding| (binding.action().name().to_string(), keys_of(binding)))
            .collect();
        let mut by_hand: Vec<(String, String)> = by_hand
            .iter()
            .map(|(name, keys)| (name.to_string(), keys.to_string()))
            .collect();
        let mut sorted = computed.clone();
        sorted.sort();
        by_hand.sort();
        assert_eq!(sorted, by_hand);
    });
}

// ----- the user's changes ---------------------------------------------------------

use gpui::KeyUpEvent;
use innerword_app::{
    menus,
    settings::keys as setting_keys,
    shortcuts::{Place, collect_shortcuts},
};
use innerword_keymap::{KeySequence, Target};
use innerword_keymap_ui::{Choice, KeymapView, ModeKind};

fn key(text: &str) -> KeySequence {
    KeySequence::parse(text).expect("a key")
}

fn bold() -> Target {
    Target::new("editor::ToggleBold", Some("Editor"))
}

/// A workspace with a document open and the keyboard in its text.
fn workspace_in_the_text(
    cx: &mut TestAppContext,
) -> (Entity<Library>, Entity<Workspace>, &mut VisualTestContext) {
    init(cx);
    let library = library(cx);
    add_document(
        &library,
        None,
        &simple_document("Keys", "A paragraph with a few words in it."),
        cx,
    );
    let (workspace, vcx) = open_workspace(&library, cx);
    assert!(editor_focused(&workspace, vcx));
    (library, workspace, vcx)
}

/// Changes the keymap the way every change of the user gets in.
fn change<R>(
    workspace: &Entity<Workspace>,
    cx: &mut VisualTestContext,
    change: impl FnOnce(&mut innerword_keymap::Keymap) -> R,
) -> R {
    let result = workspace.update_in(cx, |workspace, window, cx| {
        workspace.change_keys(window, cx, change)
    });
    cx.run_until_parked();
    result.expect("the app has a keymap")
}

/// The action GPUI would run for `keys` where the keyboard is now.
fn runs(keys: &str, cx: &mut VisualTestContext) -> Option<String> {
    cx.run_until_parked();
    cx.update(|window, cx| {
        let keystrokes: Vec<Keystroke> = keys
            .split(' ')
            .map(|key| Keystroke::parse(key).expect("a valid keystroke"))
            .collect();
        let stack = window.context_stack();
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        let (bindings, _) = keymap.bindings_for_input(&keystrokes, &stack);
        bindings
            .first()
            .map(|binding| binding.action().name().to_string())
    })
}

/// Presses and releases each key, as a keyboard does (the list takes a
/// key that is pressed again without a release for a held one).
fn press(cx: &mut VisualTestContext, keys: &str) {
    for key in keys.split_whitespace() {
        cx.simulate_keystrokes(key);
        cx.simulate_event(KeyUpEvent {
            keystroke: Keystroke::parse(key).unwrap(),
        });
    }
    cx.run_until_parked();
}

fn list(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Entity<KeymapView> {
    workspace
        .read_with(cx, |workspace, cx| workspace.shortcuts_list(cx))
        .expect("the list of keyboard shortcuts is open")
}

/// Opens the list and selects the row of `target`.
fn open_list_at(
    workspace: &Entity<Workspace>,
    target: &Target,
    cx: &mut VisualTestContext,
) -> Entity<KeymapView> {
    if workspace
        .read_with(cx, |workspace, cx| workspace.shortcuts_list(cx))
        .is_none()
    {
        press(cx, "cmd-/");
    }
    let list = list(workspace, cx);
    assert!(
        list.update(cx, |list, cx| list.select_target(target, cx)),
        "{target} has a row"
    );
    cx.run_until_parked();
    list
}

fn question(
    list: &Entity<KeymapView>,
    cx: &mut VisualTestContext,
) -> (String, String, Vec<String>) {
    list.read_with(cx, |list, _| {
        (
            list.question_title().unwrap_or_default(),
            list.question_lines()
                .iter()
                .map(|line| line.text.clone())
                .collect::<Vec<_>>()
                .join(" "),
            list.choices().into_iter().map(|(_, label)| label).collect(),
        )
    })
}

fn choose(list: &Entity<KeymapView>, choice: Choice, cx: &mut VisualTestContext) {
    assert!(
        list.update_in(cx, |list, window, cx| list.choose(choice, window, cx)),
        "the question offers {choice:?}"
    );
    cx.run_until_parked();
}

/// The key beside the menu bar item of `action`, as GPUI picks it.
fn menu_key(action: &dyn Action, cx: &mut VisualTestContext) -> Option<String> {
    cx.update(|_, cx| {
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        menus::native_menu_binding(&keymap, action)
            .filter(|binding| binding.keystrokes().len() == 1)
            .map(keys_of)
    })
}

/// The key a hint shows for `action` (the palette, a context menu).
fn hint(action: &dyn Action, cx: &mut VisualTestContext) -> Option<String> {
    cx.update(|_, cx| innerword_ui::KeyHint::for_action(action, cx).map(|hint| hint.text()))
}

/// The keys of the row of `title` in `place`, as the list has them.
fn row_keys(title: &str, place: Place, cx: &mut VisualTestContext) -> Vec<String> {
    cx.update(|_, cx| {
        collect_shortcuts(cx)
            .into_iter()
            .find(|shortcut| shortcut.title.as_ref() == title && shortcut.place == place)
            .unwrap_or_else(|| panic!("{title} is not listed in {place:?}"))
            .all_keys
            .iter()
            .map(|key| key.keys.to_string())
            .collect()
    })
}

fn toasts(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Vec<String> {
    workspace.read_with(cx, |workspace, cx| {
        workspace
            .toast_layer()
            .read(cx)
            .toasts()
            .map(|toast| toast.message.to_string())
            .collect()
    })
}

/// Presses the button of the toast that has one.
fn press_toast_button(workspace: &Entity<Workspace>, label: &str, cx: &mut VisualTestContext) {
    let layer = workspace.read_with(cx, |workspace, _| workspace.toast_layer().clone());
    let id = layer.read_with(cx, |layer, _| {
        layer
            .ids()
            .into_iter()
            .zip(layer.toasts())
            .find(|(_, toast)| toast.action_label().map(|have| have.as_ref()) == Some(label))
            .map(|(id, _)| id)
            .unwrap_or_else(|| panic!("no toast has a button {label:?}"))
    });
    cx.update(|window, cx| layer.update(cx, |layer, cx| layer.run_action(id, window, cx)));
    cx.run_until_parked();
}

/// With no change installed the keymap that runs is the defaults, binding
/// for binding, behind the menu bar's own.
#[gpui::test]
fn without_changes_the_keymap_is_the_defaults(cx: &mut TestAppContext) {
    init(cx);
    cx.update(|cx| {
        let installed: Vec<(String, String, Option<String>)> = cx
            .key_bindings()
            .borrow()
            .bindings()
            .filter(|binding| {
                !menus::is_menu_key_binding(binding)
                    && !innerword_app::boards::is_fixed_binding(binding)
            })
            .map(|binding| {
                (
                    keys_of(binding),
                    binding.action().name().to_string(),
                    binding.predicate().map(|predicate| predicate.to_string()),
                )
            })
            .collect();
        let defaults: Vec<(String, String, Option<String>)> = innerword_app::keymap::defaults(cx)
            .iter()
            .map(|binding| {
                (
                    keys_of(binding),
                    binding.action().name().to_string(),
                    binding.predicate().map(|predicate| predicate.to_string()),
                )
            })
            .collect();
        assert_eq!(installed, defaults);
        assert_eq!(innerword_app::keymap::changed_count(cx), 0);
        // Everything is handed over to be changed, except the two keys of
        // the window that stands in when the library cannot be opened.
        let model = innerword_app::keymap::model(cx).expect("the keymap as data");
        assert_eq!(model.defaults().bindings().len(), defaults.len() - 2);
        assert!(model.defaults().knows(&bold()));
        assert!(
            model
                .defaults()
                .knows(&Target::new("workspace::ShowVersionHistory", None)),
            "a command without a key can be given one"
        );
        assert!(
            !model
                .defaults()
                .targets()
                .iter()
                .any(|target| target.context.text() == Some("ErrorWindow"))
        );
    });
}

#[gpui::test]
fn a_changed_key_runs_its_command_and_the_old_key_no_longer_does(cx: &mut TestAppContext) {
    let (_library, workspace, vcx) = workspace_in_the_text(cx);
    assert!(change(&workspace, vcx, |keymap| {
        keymap.replace_key(&bold(), &key("cmd-b"), key("cmd-alt-b"))
    }));
    assert_eq!(
        runs("cmd-alt-b", vcx).as_deref(),
        Some("editor::ToggleBold")
    );
    assert_eq!(runs("cmd-b", vcx), None);

    vcx.simulate_keystrokes("cmd-a cmd-b");
    assert!(
        !is_bold(&workspace, "paragraph", vcx),
        "the old key does nothing"
    );
    vcx.simulate_keystrokes("cmd-alt-b");
    assert!(
        is_bold(&workspace, "paragraph", vcx),
        "the new key runs Bold"
    );

    // Everything that shows the key shows the new one, at once: a hint of
    // the palette and of a menu, the list of shortcuts, the menu bar.
    assert_eq!(
        hint(&innerword_editor::actions::ToggleBold, vcx).as_deref(),
        Some("⌥⌘B")
    );
    assert_eq!(row_keys("Bold", Place::Text, vcx), ["cmd-alt-b"]);
    assert_eq!(
        menu_key(&innerword_editor::actions::ToggleBold, vcx).as_deref(),
        Some("alt-cmd-b")
    );
    let (command_hint, changed) = vcx.update(|_, cx| {
        let commands = innerword_app::commands::all(cx);
        let bold = commands
            .iter()
            .find(|command| command.action_name() == "editor::ToggleBold")
            .expect("Bold is a command");
        (
            bold.key_hint(cx).map(|hint| hint.text()),
            innerword_app::keymap::changed_count(cx),
        )
    });
    assert_eq!(command_hint.as_deref(), Some("⌥⌘B"));
    assert_eq!(changed, 1);
    // The text menu, opened from the keyboard: its item for Bold shows
    // the key the keymap has when the menu is made.
    vcx.simulate_keystrokes("ctrl-enter");
    vcx.run_until_parked();
    let in_menu = vcx.update(|window, cx| {
        let menu = innerword_ui::ContextMenu::current(window, cx).expect("the text menu is open");
        let menu = menu.read(cx);
        let item = menu
            .items()
            .iter()
            .flat_map(|item| {
                std::iter::once(item).chain(item.submenu_items().into_iter().flatten())
            })
            .find(|item| item.action_name() == Some("editor::ToggleBold"))
            .expect("the text menu has Bold");
        match item.explicit_key_hint() {
            Some(hint) => Some(hint.text()),
            // Looked up when the menu is drawn.
            None => innerword_ui::KeyHint::for_action(&innerword_editor::actions::ToggleBold, cx)
                .map(|hint| hint.text()),
        }
    });
    assert_eq!(in_menu.as_deref(), Some("⌥⌘B"));
    vcx.simulate_keystrokes("escape");
}

/// Whether the word `needle` of the open document is italic.
fn is_italic(workspace: &Entity<Workspace>, needle: &str, cx: &mut VisualTestContext) -> bool {
    workspace.read_with(cx, |workspace, cx| {
        let editor = workspace.editor().read(cx);
        editor
            .document()
            .blocks()
            .iter()
            .find_map(|block| {
                let offset = block.text().find(needle)?;
                Some(block.attrs_at(offset).is_some_and(|attrs| attrs.italic))
            })
            .unwrap_or_else(|| panic!("{needle:?} is not in the document"))
    })
}

fn has_modal(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> bool {
    workspace.read_with(cx, |workspace, cx| {
        workspace.modal_layer().read(cx).has_modal()
    })
}

#[gpui::test]
fn a_removed_key_does_nothing_and_is_shown_nowhere(cx: &mut TestAppContext) {
    let (_library, workspace, vcx) = workspace_in_the_text(cx);
    let italic = Target::new("editor::ToggleItalic", Some("Editor"));
    assert!(change(&workspace, vcx, |keymap| {
        keymap.remove_key(&italic, &key("cmd-i"))
    }));
    assert_eq!(runs("cmd-i", vcx), None);
    vcx.simulate_keystrokes("cmd-a cmd-i");
    assert!(!is_italic(&workspace, "paragraph", vcx));
    // Nothing was typed in its place either.
    assert_eq!(
        editor_text(&workspace, vcx),
        "Keys\nA paragraph with a few words in it."
    );

    assert_eq!(hint(&innerword_editor::actions::ToggleItalic, vcx), None);
    assert_eq!(
        menu_key(&innerword_editor::actions::ToggleItalic, vcx),
        None
    );
    assert!(row_keys("Italic", Place::Text, vcx).is_empty());
    // The command is still there, and still runs from the palette.
    vcx.dispatch_action(innerword_editor::actions::ToggleItalic);
    vcx.run_until_parked();
    assert!(is_italic(&workspace, "paragraph", vcx));
    // No binding of any kind is left for it: the menu bar has nothing to
    // answer `cmd-i` with.
    vcx.update(|_, cx| {
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        assert_eq!(
            keymap
                .bindings_for_action(&innerword_editor::actions::ToggleItalic)
                .count(),
            0
        );
    });
}

#[gpui::test]
fn a_key_given_to_a_command_without_one_works(cx: &mut TestAppContext) {
    use innerword_app::version_history::VersionHistory;
    let (_library, workspace, vcx) = workspace_in_the_text(cx);
    let history = Target::new("workspace::ShowVersionHistory", None);
    assert!(row_keys("Version history", Place::Anywhere, vcx).is_empty());
    assert!(change(&workspace, vcx, |keymap| {
        assert!(
            keymap
                .check(&history, &key("ctrl-alt-h"), None)
                .is_allowed()
        );
        keymap.add_key(&history, key("ctrl-alt-h"))
    }));
    let open = |cx: &mut VisualTestContext| {
        workspace.read_with(cx, |workspace, cx| {
            workspace.modal_layer().read(cx).is_open::<VersionHistory>()
        })
    };
    assert_eq!(
        runs("ctrl-alt-h", vcx).as_deref(),
        Some("workspace::ShowVersionHistory")
    );
    vcx.simulate_keystrokes("ctrl-alt-h");
    vcx.run_until_parked();
    assert!(open(vcx), "the key opens the version history from the text");
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    assert!(!has_modal(&workspace, vcx));
    // It works anywhere, like the other keys of the window.
    vcx.simulate_keystrokes("cmd-0 ctrl-alt-h");
    vcx.run_until_parked();
    assert!(open(vcx), "and from the library");
    vcx.simulate_keystrokes("escape cmd-1");
    vcx.run_until_parked();

    assert_eq!(
        hint(&innerword_app::actions::ShowVersionHistory, vcx).as_deref(),
        Some("⌃⌥H")
    );
    assert_eq!(
        menu_key(&innerword_app::actions::ShowVersionHistory, vcx).as_deref(),
        Some("ctrl-alt-h")
    );
    assert_eq!(
        row_keys("Version history", Place::Anywhere, vcx),
        ["ctrl-alt-h"]
    );

    // A command of the text without a key gets one where the keys of the
    // text work.
    let divider = Target::new("editor::InsertDivider", Some("Editor"));
    assert!(change(&workspace, vcx, |keymap| {
        keymap.add_key(&divider, key("cmd-alt-d"))
    }));
    assert!(editor_focused(&workspace, vcx));
    vcx.simulate_keystrokes("cmd-alt-d");
    vcx.run_until_parked();
    assert!(workspace.read_with(vcx, |workspace, cx| {
        workspace
            .editor()
            .read(cx)
            .document()
            .blocks()
            .iter()
            .any(|block| *block.kind() == innerword_core::BlockKind::Divider)
    }));
    assert_eq!(row_keys("Divider", Place::Text, vcx), ["cmd-alt-d"]);
}

#[gpui::test]
fn a_key_bound_in_two_spellings_is_changed_as_one(cx: &mut TestAppContext) {
    let (_library, workspace, vcx) = workspace_in_the_text(cx);
    let numbered = Target::new("editor::ToggleNumberedList", Some("Editor"));
    let action = innerword_editor::actions::ToggleNumberedList;
    // One key to the user, although two bindings make it.
    assert_eq!(row_keys("Numbered list", Place::Text, vcx), ["cmd-shift-7"]);
    assert_eq!(
        runs("cmd-&", vcx).as_deref(),
        Some("editor::ToggleNumberedList")
    );

    assert!(change(&workspace, vcx, |keymap| {
        keymap.replace_key(&numbered, &key("cmd-shift-7"), key("cmd-shift-1"))
    }));
    // Both spellings of the old key are gone, both of the new one work:
    // the one a test or a script sends and the one a keyboard sends.
    for old in ["cmd-shift-7", "cmd-&"] {
        assert_eq!(runs(old, vcx), None, "{old}");
    }
    for new in ["cmd-shift-1", "cmd-!"] {
        assert_eq!(
            runs(new, vcx).as_deref(),
            Some("editor::ToggleNumberedList"),
            "{new}"
        );
    }
    // Shown in the readable spelling, by hints and by the menu bar.
    assert_eq!(hint(&action, vcx).as_deref(), Some("⇧⌘1"));
    assert_eq!(menu_key(&action, vcx).as_deref(), Some("cmd-shift-1"));
    assert_eq!(row_keys("Numbered list", Place::Text, vcx), ["cmd-shift-1"]);
    assert_eq!(
        vcx.update(|_, cx| innerword_app::keymap::changed_count(cx)),
        1
    );

    // A key without a second spelling in the place of one with two, and
    // back to the default: two bindings again, where they stood.
    assert!(change(&workspace, vcx, |keymap| {
        keymap.replace_key(&numbered, &key("cmd-shift-1"), key("cmd-alt-l"))
    }));
    assert_eq!(runs("cmd-!", vcx), None);
    assert_eq!(
        runs("cmd-alt-l", vcx).as_deref(),
        Some("editor::ToggleNumberedList")
    );
    assert!(change(&workspace, vcx, |keymap| keymap.reset(&numbered)));
    vcx.update(|_, cx| {
        let installed: Vec<String> = cx
            .key_bindings()
            .borrow()
            .bindings()
            .filter(|binding| {
                !menus::is_menu_key_binding(binding)
                    && !innerword_app::boards::is_fixed_binding(binding)
            })
            .map(keys_of)
            .collect();
        let defaults: Vec<String> = innerword_app::keymap::defaults(cx)
            .iter()
            .map(keys_of)
            .collect();
        assert_eq!(installed, defaults, "back to the defaults, in their order");
    });
}

/// A key recorded in the list for a command: taken from the command that
/// has it, kept by both where the places differ, or left alone.
#[gpui::test]
fn a_conflict_is_shown_and_settled_each_way(cx: &mut TestAppContext) {
    let (_library, workspace, vcx) = workspace_in_the_text(cx);
    let italic = Target::new("editor::ToggleItalic", Some("Editor"));

    // Bold is given the key of Italic: taken, in the same place.
    let view = open_list_at(&workspace, &bold(), vcx);
    press(vcx, "enter");
    assert_eq!(
        view.read_with(vcx, |view, _| view.mode()),
        ModeKind::Recording
    );
    press(vcx, "cmd-i");
    assert_eq!(
        view.read_with(vcx, |view, _| view.mode()),
        ModeKind::Question
    );
    let (title, says, choices) = question(&view, vcx);
    assert_eq!(title, "⌘I is taken");
    assert!(says.contains("Italic"), "{says}");
    assert!(
        choices.iter().any(|choice| choice.contains("Italic")),
        "{choices:?}"
    );
    // Nothing changed yet, and nothing ran: the text is not italic.
    assert_eq!(
        vcx.update(|_, cx| innerword_app::keymap::changed_count(cx)),
        0
    );
    assert!(!is_italic(&workspace, "paragraph", vcx));

    // Cancel: nothing changes.
    choose(&view, Choice::Cancel, vcx);
    assert_eq!(view.read_with(vcx, |view, _| view.mode()), ModeKind::List);
    assert_eq!(
        vcx.update(|_, cx| innerword_app::keymap::changed_count(cx)),
        0
    );

    // Take it from that command.
    press(vcx, "enter");
    press(vcx, "cmd-i");
    choose(&view, Choice::TakeKey, vcx);
    assert_eq!(row_keys("Bold", Place::Text, vcx), ["cmd-i"]);
    assert!(row_keys("Italic", Place::Text, vcx).is_empty());
    assert_eq!(
        vcx.update(|_, cx| innerword_app::keymap::changed_count(cx)),
        2
    );
    // The list shows it at once.
    view.read_with(vcx, |view, _| {
        assert_eq!(view.selected_keys(), vec![key("cmd-i")]);
        assert_eq!(view.changed_count(), 2);
    });
    press(vcx, "escape");
    assert!(!has_modal(&workspace, vcx));
    assert!(editor_focused(&workspace, vcx));
    assert_eq!(runs("cmd-i", vcx).as_deref(), Some("editor::ToggleBold"));
    assert_eq!(runs("cmd-b", vcx), None);
    vcx.simulate_keystrokes("cmd-a cmd-i");
    assert!(is_bold(&workspace, "paragraph", vcx));
    assert!(!is_italic(&workspace, "paragraph", vcx));
    // The model agrees with what is installed.
    vcx.update(|_, cx| {
        let effective = innerword_app::keymap::effective(cx).unwrap();
        assert!(effective.target(&italic).unwrap().keys.is_empty());
        assert!(
            innerword_app::keymap::model(cx)
                .unwrap()
                .collisions()
                .is_empty()
        );
    });

    // A command of the window is given a key the text and the library
    // have for something of their own: different places, both keep it.
    let history = Target::new("workspace::ShowVersionHistory", None);
    let view = open_list_at(&workspace, &history, vcx);
    press(vcx, "enter");
    press(vcx, "cmd-backspace");
    let (title, says, choices) = question(&view, vcx);
    assert!(
        choices.contains(&"Keep Both".to_string()),
        "{title}: {says} {choices:?}"
    );
    choose(&view, Choice::KeepBoth, vcx);
    assert_eq!(
        row_keys("Version history", Place::Anywhere, vcx),
        ["cmd-backspace"]
    );
    press(vcx, "escape");
    assert!(editor_focused(&workspace, vcx));
    // In the text and in the library the key does what it did.
    assert_eq!(
        runs("cmd-backspace", vcx).as_deref(),
        Some("editor::DeleteToLineStart")
    );
    vcx.simulate_keystrokes("cmd-0");
    assert_eq!(
        runs("cmd-backspace", vcx).as_deref(),
        Some("library_panel::MoveToTrash")
    );
    // Where nothing else has it, it opens the version history: on a row
    // of the settings.
    vcx.simulate_keystrokes("cmd-1 cmd-,");
    vcx.run_until_parked();
    assert_eq!(
        runs("cmd-backspace", vcx).as_deref(),
        Some("workspace::ShowVersionHistory")
    );
}

#[gpui::test]
fn a_key_that_would_stop_text_from_being_typed_is_refused(cx: &mut TestAppContext) {
    let (_library, workspace, vcx) = workspace_in_the_text(cx);
    let view = open_list_at(&workspace, &bold(), vcx);

    // A bare character where text is typed.
    press(vcx, "enter");
    press(vcx, "b");
    let (title, says, choices) = question(&view, vcx);
    assert_eq!(choices, ["Press Another Key", "Cancel"], "{title}: {says}");
    assert!(says.to_lowercase().contains("typed"), "{says}");
    // Shift with a character is a character too.
    choose(&view, Choice::PressAnotherKey, vcx);
    assert_eq!(
        view.read_with(vcx, |view, _| view.mode()),
        ModeKind::Recording
    );
    press(vcx, "shift-b");
    let (_, says, choices) = question(&view, vcx);
    assert_eq!(choices, ["Press Another Key", "Cancel"], "{says}");
    // A key macOS keeps for itself.
    choose(&view, Choice::PressAnotherKey, vcx);
    press(vcx, "cmd-tab");
    let (title, says, choices) = question(&view, vcx);
    assert_eq!(choices, ["Press Another Key", "Cancel"], "{title}: {says}");
    assert!(says.contains("macOS"), "{says}");
    choose(&view, Choice::Cancel, vcx);
    assert_eq!(
        vcx.update(|_, cx| innerword_app::keymap::changed_count(cx)),
        0
    );
    // Nothing of what was pressed was typed or run.
    assert_eq!(
        editor_text(&workspace, vcx),
        "Keys\nA paragraph with a few words in it."
    );

    // The same as the model says it, and where no text is typed a letter
    // is a key like any other (the library moves with `j`).
    vcx.update(|_, cx| {
        use innerword_keymap::Refusal;
        let model = innerword_app::keymap::model(cx).unwrap();
        assert_eq!(
            model.check(&bold(), &key("b"), None).refusal,
            Some(Refusal::TypesText)
        );
        assert!(matches!(
            model.check(&bold(), &key("cmd-tab"), None).refusal,
            Some(Refusal::SystemKeeps(_))
        ));
        // A key macOS uses unless that was switched off is allowed, with a
        // warning.
        let spotlight = model.check(&bold(), &key("cmd-space"), None);
        assert_eq!(spotlight.refusal, None);
        assert!(matches!(
            spotlight.warnings.as_slice(),
            [innerword_keymap::Warning::SystemUses(_)]
        ));
        let anywhere = Target::new("workspace::ShowVersionHistory", None);
        assert_eq!(
            model.check(&anywhere, &key("x"), None).refusal,
            Some(Refusal::TypesText),
            "a command of the window applies in the text too"
        );
        let rename = Target::new(
            "library_panel::Rename",
            Some("LibraryPanel && mode == tree && !TextField"),
        );
        assert!(model.defaults().knows(&rename), "the library's rename");
        assert_eq!(model.check(&rename, &key("y"), None).refusal, None);
    });
}

#[gpui::test]
fn one_command_and_everything_can_be_put_back(cx: &mut TestAppContext) {
    let (library, workspace, vcx) = workspace_in_the_text(cx);
    let italic = Target::new("editor::ToggleItalic", Some("Editor"));
    change(&workspace, vcx, |keymap| {
        keymap.replace_key(&bold(), &key("cmd-b"), key("cmd-alt-b"));
        keymap.remove_key(&italic, &key("cmd-i"));
    });
    assert_eq!(
        vcx.update(|_, cx| innerword_app::keymap::changed_count(cx)),
        2
    );
    let stored = |cx: &mut VisualTestContext| -> Option<serde_json::Value> {
        library.read_with(cx, |library, _| {
            library
                .setting(setting_keys::KEYBOARD_SHORTCUTS)
                .expect("the settings can be read")
        })
    };
    assert!(stored(vcx).is_some());

    // One command, from its row: the key of "use default".
    let view = open_list_at(&workspace, &bold(), vcx);
    press(vcx, "cmd-d");
    assert_eq!(row_keys("Bold", Place::Text, vcx), ["cmd-b"]);
    assert_eq!(
        vcx.update(|_, cx| innerword_app::keymap::changed_count(cx)),
        1
    );
    assert_eq!(
        runs_in_text("cmd-b", vcx).as_deref(),
        Some("editor::ToggleBold")
    );
    assert_eq!(runs_in_text("cmd-alt-b", vcx), None);

    // Everything, from the list's control: it asks first.
    assert!(view.update_in(vcx, |view, window, cx| view.press("Reset All…", window, cx)));
    vcx.run_until_parked();
    assert_eq!(
        view.read_with(vcx, |view, _| view.mode()),
        ModeKind::Question
    );
    assert_eq!(
        vcx.update(|_, cx| innerword_app::keymap::changed_count(cx)),
        1
    );
    choose(&view, Choice::ResetAll, vcx);
    assert_eq!(
        vcx.update(|_, cx| innerword_app::keymap::changed_count(cx)),
        0
    );
    assert!(row_keys("Italic", Place::Text, vcx) == ["cmd-i"]);
    assert_eq!(
        stored(vcx),
        None,
        "nothing is stored while every key is the default"
    );
    assert!(
        toasts(&workspace, vcx)
            .iter()
            .any(|toast| toast.contains("back to its default")),
        "{:?}",
        toasts(&workspace, vcx)
    );
    // Undo, from the toast.
    press_toast_button(&workspace, "Undo", vcx);
    assert_eq!(
        vcx.update(|_, cx| innerword_app::keymap::changed_count(cx)),
        1
    );
    assert!(row_keys("Italic", Place::Text, vcx).is_empty());
    assert!(stored(vcx).is_some());
    press(vcx, "escape");

    // And from the menu bar or the palette, which asks in a system dialog
    // that no key of the app can break.
    vcx.dispatch_action(innerword_app::actions::ResetAllShortcuts);
    vcx.run_until_parked();
    assert!(vcx.has_pending_prompt());
    assert_eq!(
        vcx.update(|_, cx| innerword_app::keymap::changed_count(cx)),
        1
    );
    vcx.simulate_prompt_answer("Reset All");
    vcx.run_until_parked();
    assert_eq!(
        vcx.update(|_, cx| innerword_app::keymap::changed_count(cx)),
        0
    );
    assert_eq!(stored(vcx), None);
    // With nothing changed there is nothing to ask.
    vcx.dispatch_action(innerword_app::actions::ResetAllShortcuts);
    vcx.run_until_parked();
    assert!(!vcx.has_pending_prompt());
    // The palette's own undo brings the last reset back as well.
    vcx.dispatch_action(innerword_app::actions::UndoShortcutsChange);
    vcx.run_until_parked();
    assert_eq!(
        vcx.update(|_, cx| innerword_app::keymap::changed_count(cx)),
        1
    );
}

/// What `keys` runs with the keyboard in the text, wherever it is now.
fn runs_in_text(keys: &str, cx: &mut VisualTestContext) -> Option<String> {
    cx.update(|_, cx| {
        let keystrokes: Vec<Keystroke> = keys
            .split(' ')
            .map(|key| Keystroke::parse(key).expect("a valid keystroke"))
            .collect();
        let stack = [
            gpui::KeyContext::parse("Workspace").unwrap(),
            gpui::KeyContext::parse("EditorTools").unwrap(),
            gpui::KeyContext::parse("Editor").unwrap(),
        ];
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        let (bindings, _) = keymap.bindings_for_input(&keystrokes, &stack);
        bindings
            .first()
            .map(|binding| binding.action().name().to_string())
    })
}

// ----- storage, the file -----------------------------------------------------------

/// The keymap that runs, as text: every binding that is not the menu
/// bar's own.
fn installed(cx: &mut VisualTestContext) -> Vec<(String, String, Option<String>)> {
    cx.update(|_, cx| {
        cx.key_bindings()
            .borrow()
            .bindings()
            .filter(|binding| {
                !menus::is_menu_key_binding(binding)
                    && !innerword_app::boards::is_fixed_binding(binding)
            })
            .map(|binding| {
                (
                    keys_of(binding),
                    binding.action().name().to_string(),
                    binding.predicate().map(|predicate| predicate.to_string()),
                )
            })
            .collect()
    })
}

fn stored_shortcuts(
    library: &Entity<Library>,
    cx: &mut VisualTestContext,
) -> Option<serde_json::Value> {
    library.read_with(cx, |library, _| {
        library
            .setting(setting_keys::KEYBOARD_SHORTCUTS)
            .expect("the settings can be read")
    })
}

/// A few changes of every kind: a key replaced, one removed, one given to
/// a command that had none, and a key in two spellings moved.
fn make_some_changes(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) {
    change(workspace, cx, |keymap| {
        keymap.replace_key(&bold(), &key("cmd-b"), key("cmd-alt-b"));
        keymap.remove_key(
            &Target::new("editor::ToggleItalic", Some("Editor")),
            &key("cmd-i"),
        );
        keymap.add_key(
            &Target::new("workspace::ShowVersionHistory", None),
            key("ctrl-alt-h"),
        );
        keymap.replace_key(
            &Target::new("editor::ToggleNumberedList", Some("Editor")),
            &key("cmd-shift-7"),
            key("cmd-shift-1"),
        );
    });
}

#[gpui::test]
fn the_changes_are_stored_in_the_library_and_survive_a_relaunch(cx: &mut TestAppContext) {
    init(cx);
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("library.db");
    let launch = |cx: &mut TestAppContext| {
        let library = cx.new(|_| Library::open(&path).expect("the library opens"));
        library.update(cx, |library, cx| {
            library
                .set_setting(setting_keys::WELCOME_CREATED, &true, cx)
                .unwrap();
        });
        library
    };

    // The first launch: nothing stored, the default keys.
    let library = launch(cx);
    add_document(
        &library,
        None,
        &simple_document("Keys", "A paragraph with a few words in it."),
        cx,
    );
    let (workspace, vcx) = open_workspace(&library, cx);
    assert_eq!(stored_shortcuts(&library, vcx), None);
    let defaults = installed(vcx);
    make_some_changes(&workspace, vcx);
    let changed = installed(vcx);
    assert_ne!(changed, defaults);
    // One setting, which holds only what differs from the defaults.
    let stored = stored_shortcuts(&library, vcx).expect("the changes are stored");
    assert_eq!(stored["version"], 1);
    let entries = stored["changes"].as_array().expect("a list of changes");
    assert_eq!(entries.len(), 4, "{stored}");
    assert_eq!(entries[0]["command"], "editor::ToggleBold");
    assert_eq!(entries[0]["context"], "Editor");
    assert_eq!(entries[0]["default"], serde_json::json!(["cmd-b"]));
    assert_eq!(entries[0]["keys"], serde_json::json!(["cmd-alt-b"]));
    // The window goes, and with it the process, as far as a test can say:
    // the keymap of the app is the default one again.
    workspace.update(vcx, |workspace, cx| assert!(workspace.flush(cx)));
    vcx.update(|window, _| window.remove_window());
    vcx.run_until_parked();
    drop(library);
    cx.update(|cx| {
        innerword_app::keymap::set_changes(innerword_keymap::Changes::new(), cx);
        assert_eq!(innerword_app::keymap::changed_count(cx), 0);
    });

    // The second launch reads them before anything is drawn.
    let library = launch(cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    assert_eq!(installed(vcx), changed);
    assert_eq!(
        vcx.update(|_, cx| innerword_app::keymap::changed_count(cx)),
        4
    );
    assert!(editor_focused(&workspace, vcx));
    assert_eq!(
        runs("cmd-alt-b", vcx).as_deref(),
        Some("editor::ToggleBold")
    );
    assert_eq!(runs("cmd-b", vcx), None);
    assert_eq!(runs("cmd-i", vcx), None);
    assert_eq!(
        runs("ctrl-alt-h", vcx).as_deref(),
        Some("workspace::ShowVersionHistory")
    );
    assert_eq!(
        runs("cmd-!", vcx).as_deref(),
        Some("editor::ToggleNumberedList")
    );
    assert_eq!(
        hint(&innerword_editor::actions::ToggleBold, vcx).as_deref(),
        Some("⌥⌘B")
    );
    assert_eq!(
        menu_key(&innerword_app::actions::ShowVersionHistory, vcx).as_deref(),
        Some("ctrl-alt-h")
    );
    // Nothing was written by reading.
    assert_eq!(stored_shortcuts(&library, vcx), Some(stored));

    // Everything put back: the setting goes.
    workspace.update_in(vcx, |workspace, window, cx| {
        workspace.reset_all_shortcuts_now(window, cx)
    });
    vcx.run_until_parked();
    assert_eq!(installed(vcx), defaults);
    assert_eq!(stored_shortcuts(&library, vcx), None);
}

/// A value another version stored: fields this one does not know, an
/// entry for a command it does not have, an entry it cannot read. All of
/// it is kept when this version writes the setting again.
#[gpui::test]
fn what_a_later_version_stored_is_not_lost(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    add_document(&library, None, &simple_document("Keys", "Some words."), cx);
    let later = serde_json::json!({
        "version": 1,
        "profile": "travel keyboard",
        "changes": [
            {"command": "editor::ToggleBold", "context": "Editor",
             "default": ["cmd-b"], "keys": ["cmd-alt-b"], "since": "0.9.0"},
            {"command": "outline::ToggleOutline", "context": null,
             "default": ["cmd-alt-o"], "keys": ["f6"]},
            {"gesture": "three-finger swipe", "command": 7},
        ],
    });
    library.update(cx, |library, cx| {
        library
            .set_setting(setting_keys::KEYBOARD_SHORTCUTS, &later, cx)
            .unwrap();
    });
    let (workspace, vcx) = open_workspace(&library, cx);
    // What this version understands is in force.
    assert_eq!(
        runs("cmd-alt-b", vcx).as_deref(),
        Some("editor::ToggleBold")
    );
    assert_eq!(runs("cmd-b", vcx), None);
    assert_eq!(runs("f6", vcx), None);
    vcx.update(|_, cx| {
        assert_eq!(innerword_app::keymap::changed_count(cx), 1);
        let inactive = innerword_app::keymap::effective(cx)
            .unwrap()
            .inactive()
            .to_vec();
        assert_eq!(inactive.len(), 2, "{inactive:?}");
    });
    assert!(
        toasts(&workspace, vcx).is_empty(),
        "nothing to complain about"
    );

    // The user changes another key: the setting is written again.
    change(&workspace, vcx, |keymap| {
        keymap.remove_key(
            &Target::new("editor::ToggleItalic", Some("Editor")),
            &key("cmd-i"),
        )
    });
    let stored = stored_shortcuts(&library, vcx).expect("still stored");
    assert_eq!(stored["profile"], "travel keyboard");
    let entries = stored["changes"].as_array().unwrap();
    assert_eq!(entries.len(), 4, "{stored}");
    assert_eq!(entries[0]["since"], "0.9.0");
    assert_eq!(entries[1]["command"], "outline::ToggleOutline");
    assert_eq!(entries[1]["keys"], serde_json::json!(["f6"]));
    assert_eq!(entries[2]["gesture"], "three-finger swipe");
    assert_eq!(entries[3]["command"], "editor::ToggleItalic");

    // A stored value that is no such thing at all: the default keys, a
    // message, and the value is left alone until a key is changed.
    let broken = serde_json::json!(["not", "an", "object"]);
    library.update(vcx, |library, cx| {
        library
            .set_setting(setting_keys::KEYBOARD_SHORTCUTS, &broken, cx)
            .unwrap();
    });
    vcx.update(|window, _| window.remove_window());
    vcx.run_until_parked();
    let (workspace, vcx) = open_workspace(&library, cx);
    assert_eq!(
        vcx.update(|_, cx| innerword_app::keymap::changed_count(cx)),
        0
    );
    assert_eq!(runs("cmd-b", vcx).as_deref(), Some("editor::ToggleBold"));
    let said = toasts(&workspace, vcx);
    assert!(
        said.iter()
            .any(|toast| toast.contains("keyboard shortcuts cannot be read")),
        "{said:?}"
    );
    assert_eq!(stored_shortcuts(&library, vcx), Some(broken));
}

/// The shortcuts file of `workspace`, written through the save panel.
fn export_to(path: &std::path::Path, cx: &mut VisualTestContext) -> serde_json::Value {
    cx.dispatch_action(innerword_app::actions::ExportShortcuts);
    cx.run_until_parked();
    assert!(cx.did_prompt_for_new_path(), "the save panel is asked");
    let target = path.to_path_buf();
    cx.simulate_new_path_selection(move |_| Some(target));
    cx.run_until_parked();
    let text = std::fs::read_to_string(path).expect("the shortcuts file was written");
    serde_json::from_str(&text).expect("the shortcuts file is JSON")
}

/// Chooses `path` in the place of the open panel and reads it.
fn import_from(path: &std::path::Path, cx: &mut VisualTestContext) {
    let path = path.to_path_buf();
    cx.update(|_, cx| innerword_app::workspace::choose_import_file_beforehand(Some(path), cx));
    cx.dispatch_action(innerword_app::actions::ImportShortcuts);
    cx.run_until_parked();
}

#[gpui::test]
fn an_exported_file_imported_into_another_library_gives_the_same_keymap(cx: &mut TestAppContext) {
    let out = tempfile::tempdir().expect("a temporary directory");
    let file = out.path().join("Inner Word Shortcuts.json");
    let (_library, workspace, vcx) = workspace_in_the_text(cx);
    let defaults = installed(vcx);
    make_some_changes(&workspace, vcx);
    let changed = installed(vcx);

    // The file is complete and reads like a list of all keys.
    let json = export_to(&file, vcx);
    assert_eq!(json["format"], "the-writer-shortcuts");
    assert_eq!(json["version"], 1);
    assert_eq!(json["app_version"], env!("CARGO_PKG_VERSION"));
    let shortcuts = json["shortcuts"].as_array().expect("a list");
    let targets = vcx.update(|_, cx| {
        innerword_app::keymap::model(cx)
            .unwrap()
            .defaults()
            .targets()
            .len()
    });
    assert_eq!(shortcuts.len(), targets, "every command in every place");
    let entry = |command: &str, context: serde_json::Value| {
        shortcuts
            .iter()
            .find(|entry| entry["command"] == command && entry["context"] == context)
            .unwrap_or_else(|| panic!("{command} is not in the file"))
            .clone()
    };
    let in_file = entry("editor::ToggleBold", "Editor".into());
    assert_eq!(in_file["title"], "Bold");
    assert_eq!(in_file["where"], "In the text");
    assert_eq!(in_file["default"], serde_json::json!(["cmd-b"]));
    assert_eq!(in_file["keys"], serde_json::json!(["cmd-alt-b"]));
    let history = entry("workspace::ShowVersionHistory", serde_json::Value::Null);
    assert_eq!(history["title"], "Version history");
    assert_eq!(history["where"], "Anywhere");
    assert_eq!(history["default"], serde_json::json!([]));
    assert_eq!(history["keys"], serde_json::json!(["ctrl-alt-h"]));
    // A key without a descriptor has a name a person reads, and its place.
    let word = entry("editor::MoveWordLeft", "Editor".into());
    assert_eq!(word["title"], "Move word left");
    assert_eq!(word["where"], "In the text");
    // A key in two spellings is written once, in the readable one.
    let numbered = entry("editor::ToggleNumberedList", "Editor".into());
    assert_eq!(numbered["default"], serde_json::json!(["cmd-shift-7"]));
    assert_eq!(numbered["keys"], serde_json::json!(["cmd-shift-1"]));
    // Every entry says what and where in words.
    for entry in shortcuts {
        assert!(
            entry["title"]
                .as_str()
                .is_some_and(|title| !title.is_empty())
        );
        assert!(
            entry["where"]
                .as_str()
                .is_some_and(|place| !place.is_empty() && place != "Elsewhere"),
            "{entry}"
        );
    }
    assert!(
        toasts(&workspace, vcx)
            .iter()
            .any(|toast| toast.starts_with("Exported the keyboard shortcuts to ")),
        "{:?}",
        toasts(&workspace, vcx)
    );
    vcx.update(|window, _| window.remove_window());
    vcx.run_until_parked();

    // Another library: the default keys, until the file is imported.
    let library = library(cx);
    add_document(&library, None, &simple_document("Other", "Words."), cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    assert_eq!(installed(vcx), defaults);
    import_from(&file, vcx);
    // Nothing is applied by choosing the file: the list shows a preview.
    let view = list(&workspace, vcx);
    assert_eq!(
        view.read_with(vcx, |view, _| view.mode()),
        ModeKind::Question
    );
    assert_eq!(installed(vcx), defaults);
    let (title, says, choices) = question(&view, vcx);
    assert!(says.contains('4'), "four commands change: {title}: {says}");
    assert!(
        choices.iter().any(|choice| choice == "Cancel"),
        "{choices:?}"
    );
    let preview = view.read_with(vcx, |view, _| view.preview_rows());
    assert!(
        preview.iter().any(|row| row.contains("Bold")),
        "{preview:?}"
    );
    // The user has no changes of their own here: one way to import.
    let offered: Vec<Choice> = view.read_with(vcx, |view, _| {
        view.choices()
            .into_iter()
            .map(|(choice, _)| choice)
            .collect()
    });
    let way = offered
        .iter()
        .copied()
        .find(|choice| matches!(choice, Choice::ImportReplace | Choice::ImportMerge))
        .expect("the preview offers to import");
    choose(&view, way, vcx);
    assert_eq!(
        installed(vcx),
        changed,
        "the same keymap as where it came from"
    );
    assert_eq!(
        vcx.update(|_, cx| innerword_app::keymap::changed_count(cx)),
        4
    );
    assert!(stored_shortcuts(&library, vcx).is_some());
    let said = toasts(&workspace, vcx);
    assert!(
        said.iter()
            .any(|toast| toast == "Imported the keys of 4 commands"),
        "{said:?}"
    );

    // Undo, offered by the toast, puts back what was there before.
    press_toast_button(&workspace, "Undo", vcx);
    assert_eq!(installed(vcx), defaults);
    assert_eq!(stored_shortcuts(&library, vcx), None);
    view.read_with(vcx, |view, _| assert_eq!(view.changed_count(), 0));
}

#[gpui::test]
fn an_import_can_replace_the_users_changes_or_add_to_them(cx: &mut TestAppContext) {
    let out = tempfile::tempdir().expect("a temporary directory");
    let file = out.path().join("mine.json");
    let (_library, workspace, vcx) = workspace_in_the_text(cx);
    // The file changes Bold; written by hand, with only what matters.
    std::fs::write(
        &file,
        r#"{"format": "the-writer-shortcuts", "version": 1, "shortcuts": [
            {"command": "editor::ToggleBold", "context": "Editor",
             "default": ["cmd-b"], "keys": ["cmd-alt-b"]},
            {"command": "editor::ToggleUnderline", "context": "Editor",
             "default": ["cmd-u"], "keys": ["cmd-u"]}
        ]}"#,
    )
    .unwrap();
    let italic = Target::new("editor::ToggleItalic", Some("Editor"));
    let own = |workspace: &Entity<Workspace>, cx: &mut VisualTestContext| {
        change(workspace, cx, |keymap| {
            keymap.set_changes(innerword_keymap::Changes::new());
            keymap.replace_key(&italic, &key("cmd-i"), key("cmd-alt-i"));
            keymap.replace_key(&bold(), &key("cmd-b"), key("f5"));
        });
    };

    // Added to the user's own: the file wins where both change a command,
    // the rest of the user's changes stay.
    own(&workspace, vcx);
    import_from(&file, vcx);
    let view = list(&workspace, vcx);
    let offered: Vec<Choice> = view.read_with(vcx, |view, _| {
        view.choices()
            .into_iter()
            .map(|(choice, _)| choice)
            .collect()
    });
    assert!(offered.contains(&Choice::ImportMerge), "{offered:?}");
    assert!(offered.contains(&Choice::ImportReplace), "{offered:?}");
    choose(&view, Choice::ImportMerge, vcx);
    assert_eq!(row_keys("Bold", Place::Text, vcx), ["cmd-alt-b"]);
    assert_eq!(row_keys("Italic", Place::Text, vcx), ["cmd-alt-i"]);
    press_toast_button(&workspace, "Undo", vcx);
    assert_eq!(row_keys("Bold", Place::Text, vcx), ["f5"]);

    // In the place of the user's own: only what the file changes differs
    // from the defaults afterwards.
    import_from(&file, vcx);
    choose(&view, Choice::ImportReplace, vcx);
    assert_eq!(row_keys("Bold", Place::Text, vcx), ["cmd-alt-b"]);
    assert_eq!(row_keys("Italic", Place::Text, vcx), ["cmd-i"]);
    assert_eq!(
        vcx.update(|_, cx| innerword_app::keymap::changed_count(cx)),
        1
    );

    // Cancelled: nothing changes.
    own(&workspace, vcx);
    import_from(&file, vcx);
    choose(&view, Choice::Cancel, vcx);
    assert_eq!(row_keys("Bold", Place::Text, vcx), ["f5"]);
    assert_eq!(row_keys("Italic", Place::Text, vcx), ["cmd-alt-i"]);
}

#[gpui::test]
fn a_file_with_things_this_version_cannot_take_says_which_and_why(cx: &mut TestAppContext) {
    let out = tempfile::tempdir().expect("a temporary directory");
    let file = out.path().join("from elsewhere.json");
    let (_library, workspace, vcx) = workspace_in_the_text(cx);
    std::fs::write(
        &file,
        r#"{"format": "the-writer-shortcuts", "version": 1, "app_version": "9.0.0",
            "shortcuts": [
            {"command": "outline::ToggleOutline", "title": "Outline", "where": "Anywhere",
             "context": null, "default": [], "keys": ["cmd-alt-o"]},
            {"command": "editor::ToggleBold", "title": "Bold", "context": "Editor",
             "default": ["cmd-b"], "keys": ["cmd-alt-b"]},
            {"command": "editor::ToggleItalic", "title": "Italic", "context": "Editor",
             "default": ["cmd-i"], "keys": ["cmd-hyper-nothing-i-j"]},
            {"command": "editor::ToggleUnderline", "title": "Underline", "context": "Editor",
             "default": ["cmd-u"], "keys": ["u"]}
        ]}"#,
    )
    .unwrap();
    import_from(&file, vcx);
    let view = list(&workspace, vcx);
    assert_eq!(
        view.read_with(vcx, |view, _| view.mode()),
        ModeKind::Question
    );
    let preview = view
        .read_with(vcx, |view, _| view.preview_rows())
        .join("\n");
    // What changes, and each entry that is skipped with its reason: a
    // command this version does not have, a key that cannot be read, a
    // change a guard rail refuses.
    assert!(preview.contains("Bold"), "{preview}");
    assert!(preview.contains("Outline"), "{preview}");
    assert!(preview.contains("Italic"), "{preview}");
    assert!(preview.contains("Underline"), "{preview}");
    let (title, says, _) = question(&view, vcx);
    assert!(
        says.contains('3') || preview.contains("3 "),
        "three entries are skipped: {title}: {says}\n{preview}"
    );
    let offered: Vec<Choice> = view.read_with(vcx, |view, _| {
        view.choices()
            .into_iter()
            .map(|(choice, _)| choice)
            .collect()
    });
    let way = offered
        .iter()
        .copied()
        .find(|choice| matches!(choice, Choice::ImportReplace | Choice::ImportMerge))
        .expect("what can be taken is offered");
    choose(&view, way, vcx);
    // Only the change that can be taken was.
    assert_eq!(row_keys("Bold", Place::Text, vcx), ["cmd-alt-b"]);
    assert_eq!(row_keys("Italic", Place::Text, vcx), ["cmd-i"]);
    assert_eq!(row_keys("Underline", Place::Text, vcx), ["cmd-u"]);
    assert_eq!(
        vcx.update(|_, cx| innerword_app::keymap::changed_count(cx)),
        1
    );
    assert!(
        toasts(&workspace, vcx)
            .iter()
            .any(|toast| toast == "Imported the keys of 1 command"),
        "{:?}",
        toasts(&workspace, vcx)
    );
}

#[gpui::test]
fn a_file_that_is_not_a_shortcuts_file_is_said_to_be_so(cx: &mut TestAppContext) {
    let out = tempfile::tempdir().expect("a temporary directory");
    let (_library, workspace, vcx) = workspace_in_the_text(cx);
    let defaults = installed(vcx);

    // Not JSON: what is wrong and where.
    let notes = out.path().join("notes.txt");
    std::fs::write(&notes, "Chapter one\nThe ferry came in late.\n").unwrap();
    import_from(&notes, vcx);
    let view = list(&workspace, vcx);
    assert_eq!(
        view.read_with(vcx, |view, _| view.mode()),
        ModeKind::Question
    );
    let (title, says, choices) = question(&view, vcx);
    assert!(says.contains("not JSON"), "{title}: {says}");
    assert!(says.contains("ine 1"), "the line is named: {says}");
    assert_eq!(
        choices,
        ["Choose Another File…", "Close"],
        "nothing to import"
    );
    choose(&view, Choice::Cancel, vcx);
    assert_eq!(view.read_with(vcx, |view, _| view.mode()), ModeKind::List);

    // JSON, of another kind.
    let other = out.path().join("package.json");
    std::fs::write(&other, r#"{"name": "something", "version": "1.0.0"}"#).unwrap();
    import_from(&other, vcx);
    let (_, says, _) = question(&view, vcx);
    assert!(says.contains("not a shortcuts file"), "{says}");
    choose(&view, Choice::Cancel, vcx);

    // Not text at all.
    let picture = out.path().join("cover.png");
    std::fs::write(&picture, [0x89, 0x50, 0x4e, 0x47, 0xff, 0xfe, 0x00, 0x01]).unwrap();
    import_from(&picture, vcx);
    let (_, says, _) = question(&view, vcx);
    assert!(says.contains("not JSON"), "{says}");
    choose(&view, Choice::Cancel, vcx);

    // A file that is not there: said in a toast, the list stays as it is.
    press(vcx, "escape");
    assert!(!has_modal(&workspace, vcx));
    import_from(&out.path().join("gone.json"), vcx);
    let said = toasts(&workspace, vcx);
    assert!(
        said.iter()
            .any(|toast| toast.starts_with("Could not read ")),
        "{said:?}"
    );
    assert!(!has_modal(&workspace, vcx));
    assert_eq!(installed(vcx), defaults, "nothing was changed by any of it");
}

// ----- precedence, and rows of several key contexts ---------------------------------

/// The precedence facts of the keymap hold after a change: a changed key
/// stands where its default stood.
#[gpui::test]
fn the_find_bar_still_beats_the_window_after_a_change(cx: &mut TestAppContext) {
    use innerword_editor_tools::ToolKind;
    let (_library, workspace, vcx) = workspace_in_the_text(cx);
    // The window's "show or hide comments" moves to the key of the find
    // bar's "whole word".
    let comments = Target::new("comments::ToggleComments", None);
    assert!(change(&workspace, vcx, |keymap| {
        keymap.replace_key(&comments, &key("cmd-alt-c"), key("cmd-alt-w"))
    }));
    assert_eq!(
        runs("cmd-alt-w", vcx).as_deref(),
        Some("comments::ToggleComments")
    );
    assert_eq!(runs("cmd-alt-c", vcx), None);
    vcx.simulate_keystrokes("cmd-f");
    vcx.run_until_parked();
    assert!(workspace.read_with(vcx, |w, cx| w.tools().read(cx).is_open(ToolKind::Find)));
    // In the bar its own key wins, as `cmd-alt-c` did before; and what
    // was the window's key is the bar's alone now.
    assert_eq!(
        runs("cmd-alt-w", vcx).as_deref(),
        Some("editor_tools::ToggleWholeWord")
    );
    assert_eq!(
        runs("cmd-alt-c", vcx).as_deref(),
        Some("editor_tools::ToggleCaseSensitive")
    );
    vcx.simulate_keystrokes("cmd-alt-w");
    assert!(workspace.read_with(vcx, |w, cx| w.tools().read(cx).find_options().whole_word));
    assert!(workspace.read_with(vcx, |w, cx| w.comments().read(cx).is_visible()));

    // The bar's "case sensitive" is bound for the bar and for the field
    // in it: one row, and a change of the row reaches both.
    let rows: Vec<(String, usize)> = vcx.update(|_, cx| {
        collect_shortcuts(cx)
            .into_iter()
            .filter(|shortcut| shortcut.action_name() == "editor_tools::ToggleCaseSensitive")
            .map(|shortcut| (shortcut.place.label().to_string(), shortcut.targets.len()))
            .collect()
    });
    assert_eq!(rows, [("In the find bar".to_string(), 2)]);
    workspace
        .update_in(vcx, |workspace, window, cx| {
            workspace.set_keys_of_command(
                "editor_tools::ToggleCaseSensitive",
                None,
                Some(&[key("cmd-alt-x")]),
                window,
                cx,
            )
        })
        .expect("the command is listed");
    vcx.run_until_parked();
    assert_eq!(
        runs("cmd-alt-x", vcx).as_deref(),
        Some("editor_tools::ToggleCaseSensitive")
    );
    assert_eq!(runs("cmd-alt-c", vcx), None);
    vcx.simulate_keystrokes("cmd-alt-x");
    assert!(workspace.read_with(vcx, |w, cx| {
        w.tools().read(cx).find_options().case_sensitive
    }));
    // No binding of the old key is left in either context.
    vcx.update(|_, cx| {
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        let keys: Vec<String> = keymap
            .bindings_for_action(&innerword_editor_tools::actions::ToggleCaseSensitive)
            .map(keys_of)
            .collect();
        assert_eq!(keys, ["alt-cmd-x", "alt-cmd-x"]);
    });
}

/// The same through the list: the request the list makes for a row is
/// carried out in every key context the row stands for.
#[gpui::test]
fn a_row_of_several_key_contexts_is_changed_as_one(cx: &mut TestAppContext) {
    let (_library, workspace, vcx) = workspace_in_the_text(cx);
    let (target, targets) = vcx.update(|_, cx| {
        let row = collect_shortcuts(cx)
            .into_iter()
            .find(|shortcut| shortcut.action_name() == "editor_tools::ToggleWholeWord")
            .expect("whole word is listed");
        (row.target.clone().expect("a binding"), row.targets.clone())
    });
    assert_eq!(targets.len(), 2);
    // The row stands for the field, where the keyboard is and where the
    // key has to beat the window's.
    assert_eq!(target.context.text(), Some("FindBar > TextField"));
    let view = open_list_at(&workspace, &target, vcx);
    press(vcx, "enter");
    press(vcx, "cmd-alt-y");
    choose(&view, Choice::UseKey, vcx);
    vcx.update(|_, cx| {
        let effective = innerword_app::keymap::effective(cx).unwrap();
        for target in &targets {
            let keys: Vec<String> = effective
                .target(target)
                .unwrap()
                .keys
                .iter()
                .map(|key| key.keys.to_string())
                .collect();
            assert_eq!(keys, ["cmd-alt-y"], "{target}");
        }
    });
    // Back to the default, from the row: both again.
    press(vcx, "cmd-d");
    assert_eq!(
        vcx.update(|_, cx| innerword_app::keymap::changed_count(cx)),
        0
    );
    // Removed from the row: from both.
    press(vcx, "cmd-backspace");
    vcx.update(|_, cx| {
        let effective = innerword_app::keymap::effective(cx).unwrap();
        for target in &targets {
            assert!(
                effective.target(target).unwrap().keys.is_empty(),
                "{target}"
            );
        }
    });

    // A key taken from a command is taken from its whole row: "whole word"
    // gets the key of "case sensitive", which the bar binds twice as well.
    press(vcx, "cmd-d");
    assert_eq!(
        vcx.update(|_, cx| innerword_app::keymap::changed_count(cx)),
        0
    );
    press(vcx, "enter");
    press(vcx, "cmd-alt-c");
    let (title, says, _) = question(&view, vcx);
    assert_eq!(title, "⌥⌘C is taken", "{says}");
    choose(&view, Choice::TakeKey, vcx);
    vcx.update(|_, cx| {
        let effective = innerword_app::keymap::effective(cx).unwrap();
        let keys = |action: &str, context: &str| -> Vec<String> {
            effective
                .target(&Target::new(action, Some(context)))
                .unwrap_or_else(|| panic!("{action} in {context}"))
                .keys
                .iter()
                .map(|key| key.keys.to_string())
                .collect()
        };
        for context in ["FindBar", "FindBar > TextField"] {
            assert_eq!(
                keys("editor_tools::ToggleWholeWord", context),
                ["cmd-alt-c"],
                "{context}"
            );
            assert!(
                keys("editor_tools::ToggleCaseSensitive", context).is_empty(),
                "{context}"
            );
        }
        // The window keeps its own `cmd-alt-c`, which the bar's beats.
        assert_eq!(
            effective
                .target(&Target::new("comments::ToggleComments", None))
                .unwrap()
                .keys
                .len(),
            1
        );
    });
    // One row each in the list, as before.
    let rows: Vec<String> = vcx.update(|_, cx| {
        collect_shortcuts(cx)
            .into_iter()
            .filter(|shortcut| {
                [
                    "editor_tools::ToggleCaseSensitive",
                    "editor_tools::ToggleWholeWord",
                ]
                .contains(&shortcut.action_name())
            })
            .map(|shortcut| {
                format!(
                    "{} {:?}",
                    shortcut.action_name(),
                    shortcut
                        .all_keys
                        .iter()
                        .map(|key| key.keys.to_string())
                        .collect::<Vec<_>>()
                )
            })
            .collect()
    });
    assert_eq!(
        rows,
        [
            "editor_tools::ToggleCaseSensitive []",
            "editor_tools::ToggleWholeWord [\"cmd-alt-c\"]"
        ]
    );
}

// ----- the ways in, and what names a key --------------------------------------------

#[gpui::test]
fn the_list_is_reached_from_the_settings_the_palette_and_the_menu_bar(cx: &mut TestAppContext) {
    use innerword_app::settings_view::{SettingRow, SettingsView};
    let (_library, workspace, vcx) = workspace_in_the_text(cx);
    let list_open = |cx: &mut VisualTestContext| {
        workspace
            .read_with(cx, |workspace, cx| workspace.shortcuts_list(cx))
            .is_some()
    };

    // The settings have a row for it, with the keyboard and with a click.
    vcx.simulate_keystrokes("cmd-,");
    vcx.run_until_parked();
    let settings = workspace
        .read_with(vcx, |workspace, cx| {
            workspace.modal_layer().read(cx).active::<SettingsView>()
        })
        .expect("the settings are open");
    let (rows, says) = settings.read_with(vcx, |settings, cx| {
        (
            settings.rows(cx),
            settings
                .description(SettingRow::KeyboardShortcuts, false, cx)
                .to_string(),
        )
    });
    let row = rows
        .iter()
        .position(|row| *row == SettingRow::KeyboardShortcuts)
        .expect("the settings have a row for the keyboard shortcuts");
    assert!(!says.contains("of your own"), "nothing is changed: {says}");
    settings.update(vcx, |settings, cx| {
        settings.activate(SettingRow::KeyboardShortcuts, cx)
    });
    vcx.run_until_parked();
    assert!(
        list_open(vcx),
        "the row opens the list in the place of the settings"
    );
    press(vcx, "escape");
    assert!(!has_modal(&workspace, vcx));
    assert!(editor_focused(&workspace, vcx));

    // With the pointer; and the row says how many keys are the user's.
    change(&workspace, vcx, |keymap| {
        keymap.replace_key(&bold(), &key("cmd-b"), key("cmd-alt-b"))
    });
    vcx.simulate_keystrokes("cmd-,");
    vcx.run_until_parked();
    let settings = workspace
        .read_with(vcx, |workspace, cx| {
            workspace.modal_layer().read(cx).active::<SettingsView>()
        })
        .expect("the settings are open");
    let says = settings.read_with(vcx, |settings, cx| {
        settings
            .description(SettingRow::KeyboardShortcuts, false, cx)
            .to_string()
    });
    assert!(says.ends_with("1 command has keys of your own."), "{says}");
    // The row is far down the list: the keyboard takes it into view.
    for _ in 0..row {
        vcx.simulate_keystrokes("down");
    }
    vcx.run_until_parked();
    let selector: &'static str = Box::leak(format!("setting-{row}").into_boxed_str());
    let at = vcx
        .debug_bounds(selector)
        .expect("the row is drawn")
        .center();
    vcx.simulate_click(at, gpui::Modifiers::default());
    vcx.run_until_parked();
    assert!(list_open(vcx), "a click on the row opens the list");
    press(vcx, "escape");

    // The palette has the list and what goes with it.
    let titles: Vec<String> = vcx.update(|_, cx| {
        innerword_app::commands::all(cx)
            .iter()
            .map(|command| command.title.to_string())
            .collect()
    });
    for title in [
        "Keyboard shortcuts",
        "Import keyboard shortcuts",
        "Export keyboard shortcuts",
        "Reset all keyboard shortcuts",
        "Undo keyboard shortcuts import or reset",
    ] {
        assert!(titles.contains(&title.to_string()), "{title}");
    }
    vcx.dispatch_action(innerword_app::actions::ShowShortcuts);
    vcx.run_until_parked();
    assert!(list_open(vcx));
    // The same command closes it again.
    vcx.dispatch_action(innerword_app::actions::ShowShortcuts);
    vcx.run_until_parked();
    assert!(!has_modal(&workspace, vcx));

    // The menu bar: Inner Word has the four items, Help the list.
    let all = menus::app_menus();
    let app_menu: Vec<String> = all[0]
        .items
        .iter()
        .filter_map(|item| match item {
            gpui::MenuItem::Action { name, .. } => Some(name.to_string()),
            _ => None,
        })
        .collect();
    for item in [
        "Keyboard Shortcuts…",
        "Import Keyboard Shortcuts…",
        "Export Keyboard Shortcuts…",
        "Reset All Keyboard Shortcuts…",
    ] {
        assert!(
            app_menu.contains(&item.to_string()),
            "{item} in {app_menu:?}"
        );
    }
    // Every one of them is enabled: its action has a handler wherever the
    // keyboard is, also with the list open and with no document.
    for place in ["cmd-1", "cmd-0", "cmd-/"] {
        vcx.simulate_keystrokes(place);
        vcx.run_until_parked();
        vcx.update(|window, cx| {
            for action in [
                &innerword_app::actions::ShowShortcuts as &dyn Action,
                &innerword_app::actions::ImportShortcuts,
                &innerword_app::actions::ExportShortcuts,
                &innerword_app::actions::ResetAllShortcuts,
            ] {
                assert!(
                    window.is_action_available(action, cx),
                    "{} after {place}",
                    action.name()
                );
            }
        });
    }
}

/// The texts of the app's own modals that name a key take it from the
/// keymap: the footers, the tooltip of the close control, a sentence.
#[gpui::test]
fn the_footers_and_tooltips_of_the_modals_follow_the_keymap(cx: &mut TestAppContext) {
    use innerword_app::{agents_view, controls, keymap, settings_view, version_history};
    let (_library, workspace, vcx) = workspace_in_the_text(cx);
    let texts = |cx: &mut VisualTestContext| {
        cx.update(|_, cx| {
            (
                controls::close_key(cx).map(|hint| hint.text()),
                keymap::keys_text_in(
                    &[
                        &version_history::SelectPrevious,
                        &version_history::SelectNext,
                    ],
                    version_history::KEY_CONTEXT,
                    cx,
                ),
                keymap::keys_text_in(&[&agents_view::CopyRow], agents_view::KEY_CONTEXT, cx),
                keymap::keys_sentence_in(
                    &settings_view::Activate,
                    settings_view::ROW_KEY_CONTEXT,
                    cx,
                ),
                keymap::hint_in(&version_history::Restore, version_history::KEY_CONTEXT, cx)
                    .map(|hint| hint.text()),
            )
        })
    };
    assert_eq!(
        texts(vcx),
        (
            Some("Esc".to_string()),
            Some("↑ ↓".to_string()),
            Some("⌘C".to_string()),
            Some("↩ or Space".to_string()),
            Some("↩".to_string()),
        )
    );

    change(&workspace, vcx, |keymap| {
        let version = |action: &str| Target::new(action, Some(version_history::KEY_CONTEXT));
        keymap.replace_key(
            &Target::new("modal::Dismiss", Some("ModalLayer")),
            &key("escape"),
            key("cmd-escape"),
        );
        keymap.replace_key(
            &version("version_history::SelectNext"),
            &key("down"),
            key("j"),
        );
        keymap.remove_key(&version("version_history::SelectNext"), &key("ctrl-n"));
        keymap.set_keys(&version("version_history::Restore"), &[key("cmd-r")]);
        keymap.set_keys(
            &Target::new("agents::CopyRow", Some(agents_view::KEY_CONTEXT)),
            &[],
        );
        keymap.remove_key(
            &Target::new("settings::Activate", Some(settings_view::ROW_KEY_CONTEXT)),
            &key("enter"),
        );
    });
    assert_eq!(
        texts(vcx),
        (
            Some("⌘Esc".to_string()),
            Some("↑ J".to_string()),
            // A command without a key: the hint is left out.
            None,
            Some("Space".to_string()),
            Some("⌘R".to_string()),
        )
    );
}

/// The key beside a menu bar item is the key the user gave its command,
/// and none where the user took the key away or gave one the menu bar
/// cannot draw.
#[gpui::test]
fn the_menu_bar_shows_the_keys_the_user_chose(cx: &mut TestAppContext) {
    use innerword_editor::actions as editor;
    let (_library, workspace, vcx) = workspace_in_the_text(cx);
    let text = |action: &str| Target::new(action, Some("Editor"));
    assert_eq!(menu_key(&editor::InsertPageBreak, vcx), None, "cmd-enter");
    assert_eq!(menu_key(&editor::Indent, vcx).as_deref(), Some("cmd-]"));
    change(&workspace, vcx, |keymap| {
        // A key the menu bar can draw, for an item that showed none.
        keymap.replace_key(
            &text("editor::InsertPageBreak"),
            &key("cmd-enter"),
            key("cmd-alt-enter"),
        );
        keymap.add_key(&text("editor::InsertPageBreak"), key("f6"));
        // A key with return in it, for an item that showed one.
        keymap.replace_key(&text("editor::ToggleBold"), &key("cmd-b"), key("alt-enter"));
        // The second key of an item goes: the first cannot be drawn.
        keymap.remove_key(&text("editor::Indent"), &key("cmd-]"));
        // The key of a command that is listed in another place too.
        keymap.remove_key(
            &Target::new("editor_tools::FindPrevious", Some("EditorTools")),
            &key("cmd-shift-g"),
        );
    });
    // The key hints show the last one bound, the menu bar the one it can
    // draw.
    assert_eq!(
        row_keys("Page break", Place::Text, vcx),
        ["cmd-alt-enter", "f6"]
    );
    assert_eq!(
        menu_key(&editor::InsertPageBreak, vcx).as_deref(),
        Some("f6")
    );
    assert_eq!(menu_key(&editor::ToggleBold, vcx), None);
    assert_eq!(
        runs("alt-enter", vcx).as_deref(),
        Some("editor::ToggleBold")
    );
    assert_eq!(menu_key(&editor::Indent, vcx), None);
    assert_eq!(runs("tab", vcx).as_deref(), Some("editor::Indent"));
    // "Find Previous" keeps `shift-enter` in the find bar; the menu bar
    // does not show that key for it (AppKit would run the item from
    // anywhere on it).
    assert_eq!(
        menu_key(&innerword_editor_tools::actions::FindPrevious, vcx),
        None
    );
    // Every key the menu bar shows is a key that runs its item's command,
    // as before the change.
    vcx.update(|_, cx| {
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        for (name, action) in menus::menu_actions(&menus::app_menus()) {
            let Some(native) = menus::native_menu_binding(&keymap, action.as_ref()) else {
                continue;
            };
            if native.keystrokes().len() > 1 {
                continue;
            }
            assert!(
                menus::menu_can_show(native),
                "{name} shows {}",
                keys_of(native)
            );
            let real: Vec<String> = keymap
                .bindings_for_action(action.as_ref())
                .filter(|binding| {
                    !menus::is_menu_key_binding(binding)
                        && !innerword_app::boards::is_fixed_binding(binding)
                })
                .map(keys_of)
                .collect();
            assert!(real.contains(&keys_of(native)), "{name}: {real:?}");
        }
    });
}

/// What the menu bar draws beside an item for `binding`: AppKit writes the
/// modifiers it is handed and the character of the key, so shift with `=`
/// is "⇧⌘=" and the character `+` is "⌘+". `None` for a key with a name
/// (AppKit has glyphs of its own for those) and for a sequence.
fn drawn_in_the_menu_bar(binding: &KeyBinding) -> Option<String> {
    let [keystroke] = binding.keystrokes() else {
        return None;
    };
    let keystroke = keystroke.inner();
    if keystroke.key.chars().count() != 1 {
        return None;
    }
    let mut drawn = String::new();
    for (held, glyph) in [
        (keystroke.modifiers.control, '⌃'),
        (keystroke.modifiers.alt, '⌥'),
        (keystroke.modifiers.shift, '⇧'),
        (keystroke.modifiers.platform, '⌘'),
    ] {
        if held {
            drawn.push(glyph);
        }
    }
    drawn.push_str(&keystroke.key.to_uppercase());
    Some(drawn)
}

/// A key reads the same wherever it is shown: beside the item of the menu
/// bar, in the hints (the palette, the menus of the window) and as a chip
/// of its row in the list of keyboard shortcuts. 0.6.0 showed "Zoom In ⌘+"
/// in the menus and "⌘=" and "⇧⌘=" in the list, so whoever looked the key
/// up did not find the one the menu shows.
#[gpui::test]
fn a_key_reads_the_same_in_the_menu_bar_the_hints_and_the_list(cx: &mut TestAppContext) {
    use innerword_app::zoom;
    let (_library, workspace, vcx) = workspace_in_the_text(cx);
    let check = |vcx: &mut VisualTestContext| {
        vcx.update(|_, cx| {
            let rows = collect_shortcuts(cx);
            let keymap = cx.key_bindings();
            let keymap = keymap.borrow();
            let mut compared = 0;
            for (name, action) in menus::menu_actions(&menus::app_menus()) {
                let Some(drawn) = menus::native_menu_binding(&keymap, action.as_ref())
                    .and_then(drawn_in_the_menu_bar)
                else {
                    continue;
                };
                let chips: Vec<String> = rows
                    .iter()
                    .filter(|row| row.action().partial_eq(action.as_ref()))
                    .flat_map(|row| row.all_keys.iter())
                    .map(|key| innerword_app::shortcuts::key_text(&key.keys))
                    .collect();
                if chips.is_empty() {
                    // The window's own items (Hide, Minimize) are rows by
                    // their actions; an item without a row has no chip.
                    continue;
                }
                assert!(
                    chips.contains(&drawn),
                    "{name}: the menu bar draws {drawn}, the list shows {chips:?}"
                );
                compared += 1;
            }
            compared
        })
    };
    assert!(check(vcx) > 60, "{}", check(vcx));

    // The key this is about, by name.
    let zoom_in = vcx.update(|_, cx| {
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        menus::native_menu_binding(&keymap, &zoom::ZoomIn).and_then(drawn_in_the_menu_bar)
    });
    assert_eq!(zoom_in.as_deref(), Some("⌘+"));
    assert_eq!(hint(&zoom::ZoomIn, vcx).as_deref(), Some("⌘+"));
    let chips = |title: &str, place: Place, vcx: &mut VisualTestContext| -> Vec<String> {
        vcx.update(|_, cx| {
            collect_shortcuts(cx)
                .into_iter()
                .find(|row| row.title.as_ref() == title && row.place == place)
                .unwrap_or_else(|| panic!("{title} is not listed"))
                .all_keys
                .iter()
                .map(|key| innerword_app::shortcuts::key_text(&key.keys))
                .collect()
        })
    };
    // The row keeps its two keys in their order (a change of the row does
    // what it did); the second reads as the menu writes it.
    assert_eq!(
        row_keys("Zoom in", Place::Anywhere, vcx),
        ["cmd-=", "cmd-shift-="]
    );
    assert_eq!(chips("Zoom in", Place::Anywhere, vcx), ["⌘=", "⌘+"]);
    // The other keys made with shift and punctuation read as they did, in
    // the list and in the menu bar alike.
    assert_eq!(chips("Zoom to actual size", Place::Anywhere, vcx), ["⇧⌘0"]);
    assert_eq!(chips("Numbered list", Place::Text, vcx), ["⇧⌘7"]);
    assert_eq!(
        menu_key(&zoom::ActualSize, vcx).as_deref(),
        Some("cmd-shift-0")
    );

    // A key the user gives a command is no different: the plus key on
    // "Bold" reads "⌘+" in all three places, and the plus key on a real
    // keyboard (which reports the character) runs it.
    change(&workspace, vcx, |keymap| {
        keymap.replace_key(&bold(), &key("cmd-b"), key("cmd-shift-="));
        keymap.remove_key(&Target::new("zoom::ZoomIn", None), &key("cmd-shift-="));
    });
    assert!(check(vcx) > 60);
    let action = innerword_editor::actions::ToggleBold;
    assert_eq!(menu_key(&action, vcx).as_deref(), Some("cmd-+"));
    assert_eq!(hint(&action, vcx).as_deref(), Some("⌘+"));
    assert_eq!(chips("Bold", Place::Text, vcx), ["⌘+"]);
    assert_eq!(runs("cmd-+", vcx).as_deref(), Some("editor::ToggleBold"));
    assert_eq!(
        runs("cmd-shift-=", vcx).as_deref(),
        Some("editor::ToggleBold")
    );
    assert_eq!(chips("Zoom in", Place::Anywhere, vcx), ["⌘="]);
}

/// The steps of a scripted run: a key set without the list, and what the
/// state line says.
#[gpui::test]
fn a_script_sets_keys_and_reads_the_state(cx: &mut TestAppContext) {
    use innerword_app::automation::{Script, ShortcutsStep, Step, describe_state};
    let script = Script::parse(
        "shortcut:editor::ToggleBold=cmd-alt-b; \
         shortcut:editor::ToggleItalic = none; \
         shortcut:spell::NextMisspelling=cmd-\\;|f7; \
         shortcut:editor_tools::FindPrevious@In the find bar=default; \
         shortcuts:import:/tmp/keys.json; shortcuts:press:Replace My Changes; \
         shortcuts:reset; shortcuts:undo; menukey:Version History…",
    )
    .expect("the steps parse");
    assert_eq!(
        script.steps,
        [
            Step::Shortcut {
                action: "editor::ToggleBold".into(),
                place: None,
                keys: Some(vec![key("cmd-alt-b")]),
            },
            Step::Shortcut {
                action: "editor::ToggleItalic".into(),
                place: None,
                keys: Some(Vec::new()),
            },
            Step::Shortcut {
                action: "spell::NextMisspelling".into(),
                place: None,
                keys: Some(vec![key("cmd-;"), key("f7")]),
            },
            Step::Shortcut {
                action: "editor_tools::FindPrevious".into(),
                place: Some("In the find bar".into()),
                keys: None,
            },
            Step::Shortcuts(ShortcutsStep::Import("/tmp/keys.json".into())),
            Step::Shortcuts(ShortcutsStep::Press("Replace My Changes".into())),
            Step::Shortcuts(ShortcutsStep::Reset),
            Step::Shortcuts(ShortcutsStep::Undo),
            Step::MenuKey("Version History…".into()),
        ]
    );
    assert!(Script::parse("menukey").is_err());
    assert!(Script::parse("shortcut:editor::ToggleBold").is_err());
    assert!(Script::parse("shortcut:editor::ToggleBold=cmd-").is_err());
    assert!(Script::parse("shortcuts:dance").is_err());
    assert!(matches!(
        Script::parse("keyscost; keyscost:3").unwrap().steps[..],
        [Step::KeysCost(7), Step::KeysCost(3)]
    ));

    let (_library, workspace, vcx) = workspace_in_the_text(cx);
    let state = |cx: &mut VisualTestContext| {
        workspace.update_in(cx, |workspace, window, cx| {
            let line = describe_state(workspace, window, cx);
            let from = line.find("keys=[").expect("the state line has the keys");
            let to = line.find(" words=").expect("the counts follow");
            line[from..to].to_string()
        })
    };
    assert_eq!(state(vcx), "keys=[changed=0]");
    let set = |action: &str,
               place: Option<&str>,
               keys: Option<&[KeySequence]>,
               cx: &mut VisualTestContext| {
        let result = workspace.update_in(cx, |workspace, window, cx| {
            workspace.set_keys_of_command(action, place, keys, window, cx)
        });
        cx.run_until_parked();
        result
    };
    set("editor::ToggleBold", None, Some(&[key("cmd-alt-b")]), vcx).unwrap();
    set("editor::ToggleItalic", None, Some(&[]), vcx).unwrap();
    assert_eq!(state(vcx), "keys=[changed=2]");
    assert_eq!(
        runs("cmd-alt-b", vcx).as_deref(),
        Some("editor::ToggleBold")
    );
    assert_eq!(runs("cmd-i", vcx), None);
    // A command that is listed in two places: the first by default, the
    // other by its name.
    set(
        "editor_tools::FindPrevious",
        Some("In the find bar"),
        Some(&[key("f3")]),
        vcx,
    )
    .unwrap();
    assert_eq!(row_keys("Find previous", Place::FindBar, vcx), ["f3"]);
    assert_eq!(row_keys("Find previous", Place::Text, vcx), ["cmd-shift-g"]);
    set("editor::ToggleBold", None, None, vcx).unwrap();
    assert_eq!(state(vcx), "keys=[changed=2]");
    assert!(set("editor::NoSuchThing", None, None, vcx).is_err());
    assert!(set("editor::ToggleBold", Some("In the library"), None, vcx).is_err());

    // With the list open the line says what it shows.
    press(vcx, "cmd-/");
    let line = state(vcx);
    assert!(
        line.starts_with("keys=[changed=2 list=[mode=list rows="),
        "{line}"
    );
    assert!(line.contains(" changed=2 changed_only=false"), "{line}");
    press(vcx, "enter");
    assert!(
        state(vcx).contains("list=[mode=recording"),
        "{}",
        state(vcx)
    );
    press(vcx, "cmd-alt-9");
    let line = state(vcx);
    assert!(line.contains("list=[mode=question"), "{line}");
    assert!(line.contains("choices=[*Use ⌥⌘9"), "{line}");
}

// ----- what the check of the whole thing found --------------------------------------

/// Wherever the user is told how many keys are theirs, a command that is
/// bound in several key contexts of one place is one command, as it is one
/// row of the list. (The list said "1 key changed" and the question of
/// "reset all" in the same list "2 commands go back".)
#[gpui::test]
fn a_row_of_several_key_contexts_counts_as_one_command(cx: &mut TestAppContext) {
    use innerword_app::automation::describe_state;
    use innerword_app::settings_view::{SettingRow, SettingsView};
    let (_library, workspace, vcx) = workspace_in_the_text(cx);
    let target = vcx.update(|_, cx| {
        let row = collect_shortcuts(cx)
            .into_iter()
            .find(|shortcut| shortcut.action_name() == "editor_tools::ToggleWholeWord")
            .expect("whole word is listed");
        assert_eq!(row.targets.len(), 2, "the bar binds it twice");
        row.target.clone().expect("a binding")
    });
    let view = open_list_at(&workspace, &target, vcx);
    press(vcx, "enter");
    press(vcx, "cmd-alt-y");
    choose(&view, Choice::UseKey, vcx);
    // Two key contexts were changed ...
    assert_eq!(
        vcx.update(|_, cx| innerword_app::keymap::changed_count(cx)),
        2
    );
    // ... which is one row, and one command to the user.
    assert_eq!(view.read_with(vcx, |view, _| view.changed_count()), 1);
    assert_eq!(
        vcx.update(|_, cx| innerword_app::workspace::changed_command_count(cx)),
        1
    );
    // The question of "reset all", in the list.
    press(vcx, "cmd-shift-d");
    let (_, says, _) = question(&view, vcx);
    assert!(says.starts_with("1 command goes back"), "{says}");
    choose(&view, Choice::Cancel, vcx);
    // The state line of a scripted run.
    let line = workspace.update_in(vcx, |workspace, window, cx| {
        describe_state(workspace, window, cx)
    });
    assert!(line.contains("keys=[changed=1 list=["), "{line}");
    press(vcx, "escape");
    // The row of the settings.
    press(vcx, "cmd-,");
    let settings = workspace
        .read_with(vcx, |workspace, cx| {
            workspace.modal_layer().read(cx).active::<SettingsView>()
        })
        .expect("the settings are open");
    let says = settings.read_with(vcx, |settings, cx| {
        settings
            .description(SettingRow::KeyboardShortcuts, false, cx)
            .to_string()
    });
    assert!(says.ends_with("1 command has keys of your own."), "{says}");
    press(vcx, "escape");
    // The system dialog of the menu bar's "reset all".
    vcx.dispatch_action(innerword_app::actions::ResetAllShortcuts);
    vcx.run_until_parked();
    let (_, detail) = vcx.pending_prompt().expect("the dialog is shown");
    assert_eq!(
        detail,
        "1 command goes back to the default keys. Every change you made is removed."
    );
    vcx.simulate_prompt_answer("Cancel");
    vcx.run_until_parked();

    // A file with that change has the command twice, once for each key
    // context. The preview of its import shows it once, and the toast
    // counts it once.
    let out = tempfile::tempdir().expect("a temporary directory");
    let file = out.path().join("Inner Word Shortcuts.json");
    export_to(&file, vcx);
    workspace.update_in(vcx, |workspace, window, cx| {
        workspace.reset_all_shortcuts_now(window, cx)
    });
    vcx.run_until_parked();
    assert_eq!(
        vcx.update(|_, cx| innerword_app::keymap::changed_count(cx)),
        0
    );
    import_from(&file, vcx);
    let view = list(&workspace, vcx);
    let (_, says, _) = question(&view, vcx);
    assert!(says.starts_with("1 command gets other keys."), "{says}");
    let rows = view.read_with(vcx, |view, _| view.preview_rows());
    assert_eq!(rows.len(), 2, "a heading and one row: {rows:?}");
    assert!(rows[0].ends_with("(1)"), "{rows:?}");
    choose(&view, Choice::ImportMerge, vcx);
    assert_eq!(
        vcx.update(|_, cx| innerword_app::keymap::changed_count(cx)),
        2
    );
    assert_eq!(toasts(&workspace, vcx), ["Imported the keys of 1 command"]);
}

/// With Vim mode on, Vim sees the keys of the text before any binding
/// does. The keymap knows nothing of that, so the list says it where a
/// key is chosen: for a key Vim may take (not one with ⌘, a function key
/// or an option chord, which Vim leaves alone), for a command of the text
/// or one that works anywhere, and only while Vim mode is on.
#[gpui::test]
fn with_vim_mode_on_the_list_says_that_vim_sees_a_key_first(cx: &mut TestAppContext) {
    let (_library, workspace, vcx) = workspace_in_the_text(cx);
    let vim_says = "Vim mode is on";
    // The heading and the sentences of the question about `keys`.
    let ask_whole = |target: &Target, keys: &str, cx: &mut VisualTestContext| {
        let view = open_list_at(&workspace, target, cx);
        press(cx, "enter");
        press(cx, keys);
        let (title, says, _) = question(&view, cx);
        choose(&view, Choice::Cancel, cx);
        press(cx, "escape");
        (title, says)
    };
    let ask = |target: &Target, keys: &str, cx: &mut VisualTestContext| -> String {
        ask_whole(target, keys, cx).1
    };
    let history = Target::new("workspace::ShowVersionHistory", None);
    let library_row = vcx.update(|_, cx| {
        collect_shortcuts(cx)
            .into_iter()
            .find(|shortcut| shortcut.place == Place::Library && shortcut.target.is_some())
            .and_then(|shortcut| shortcut.target)
            .expect("the library has a key")
    });

    // Vim mode off: nothing about it.
    assert!(!ask(&bold(), "ctrl-y", vcx).contains(vim_says));

    workspace.update(vcx, |workspace, cx| workspace.toggle_vim_mode(cx));
    vcx.run_until_parked();
    assert!(workspace.read_with(vcx, |workspace, cx| workspace.vim().read(cx).is_attached()));
    // A key of the text that Vim has a command on, and one of the window.
    // The heading names who sees the key first, not only that it may not
    // run.
    let (title, says) = ask_whole(&bold(), "ctrl-y", vcx);
    assert_eq!(title, "⌃Y: Vim mode sees it first");
    assert!(says.contains(vim_says), "{says}");
    assert!(says.contains("the shortcut does not run"), "{says}");
    let (title, says) = ask_whole(&history, "ctrl-y", vcx);
    assert_eq!(title, "⌃Y: Vim mode sees it first");
    assert!(says.contains(vim_says), "{says}");
    // Keys Vim leaves to the keymap.
    for keys in ["cmd-alt-b", "f5", "alt-b"] {
        let says = ask(&bold(), keys, vcx);
        assert!(!says.contains(vim_says), "{keys}: {says}");
    }
    // A place Vim is not in.
    let says = ask(&library_row, "ctrl-y", vcx);
    assert!(!says.contains(vim_says), "{says}");
    // And what it says is so: in Normal mode the key never reaches the
    // command it was given to, while a key Vim leaves alone does.
    change(&workspace, vcx, |keymap| {
        keymap.set_keys(&history, &[key("ctrl-y"), key("f5")])
    });
    assert!(editor_focused(&workspace, vcx));
    press(vcx, "ctrl-y");
    assert!(!has_modal(&workspace, vcx), "Vim took ctrl-y");
    press(vcx, "f5");
    assert!(has_modal(&workspace, vcx), "f5 opens the version history");
}

/// The worst a file can do: it takes every key away, the keys of the list
/// and of its questions included. Nothing of the keyboard works then; the
/// pointer alone opens the list from the menu bar, puts everything back,
/// and the keys are there again.
#[gpui::test]
fn every_key_taken_away_is_put_back_with_the_pointer_alone(cx: &mut TestAppContext) {
    let out = tempfile::tempdir().expect("a temporary directory");
    let file = out.path().join("Inner Word Shortcuts.json");
    let (_library, workspace, vcx) = workspace_in_the_text(cx);
    let defaults = installed(vcx);
    let click = |selector: &'static str, cx: &mut VisualTestContext| {
        let bounds = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("nothing is drawn as {selector:?}"));
        cx.simulate_click(bounds.center(), gpui::Modifiers::none());
        cx.run_until_parked();
    };

    // The exported file, with every key taken out by hand.
    let mut exported = export_to(&file, vcx);
    for entry in exported["shortcuts"].as_array_mut().unwrap() {
        entry["keys"] = serde_json::json!([]);
    }
    std::fs::write(&file, serde_json::to_string_pretty(&exported).unwrap()).unwrap();
    import_from(&file, vcx);
    let view = list(&workspace, vcx);
    let (_, says, _) = question(&view, vcx);
    assert!(says.contains("commands get other keys"), "{says}");
    choose(&view, Choice::ImportMerge, vcx);
    // One toast says it (the one about the export made way for it): a
    // stack of them would lie over the lower part of the list.
    assert_eq!(
        toasts(&workspace, vcx).len(),
        1,
        "{:?}",
        toasts(&workspace, vcx)
    );
    // Nothing is left but the two keys of the message window that stands
    // in when the library cannot be opened, which are nobody's to change.
    let left = |cx: &mut VisualTestContext| -> Vec<(String, String, Option<String>)> {
        installed(cx)
            .into_iter()
            .filter(|(_, _, context)| context.as_deref() != Some("ErrorWindow"))
            .collect()
    };
    assert!(left(vcx).is_empty(), "a binding is left: {:?}", left(vcx));

    // The list is open and has no key: escape still closes it (it is not
    // a binding there), and no key opens it again.
    assert_eq!(view.read_with(vcx, |view, _| view.mode()), ModeKind::List);
    press(vcx, "down");
    press(vcx, "cmd-shift-d");
    assert_eq!(view.read_with(vcx, |view, _| view.mode()), ModeKind::List);
    press(vcx, "escape");
    assert!(!has_modal(&workspace, vcx));
    press(vcx, "cmd-/");
    press(vcx, "cmd-shift-p");
    assert!(!has_modal(&workspace, vcx), "no key does anything");

    // The menu bar still has the list (an item of the menu bar is its
    // action), and in the list the pointer does the rest.
    vcx.dispatch_action(innerword_app::actions::ShowShortcuts);
    vcx.run_until_parked();
    let view = list(&workspace, vcx);
    click("keymap-reset-all", vcx);
    assert_eq!(
        view.read_with(vcx, |view, _| view.mode()),
        ModeKind::Question
    );
    // The key that chooses is gone too: the choice is clicked.
    press(vcx, "enter");
    assert_eq!(
        view.read_with(vcx, |view, _| view.mode()),
        ModeKind::Question
    );
    click("keymap-choice-0", vcx);
    assert_eq!(view.read_with(vcx, |view, _| view.mode()), ModeKind::List);
    assert_eq!(installed(vcx), defaults, "every key is back");
    click("keymap-close", vcx);
    assert!(!has_modal(&workspace, vcx));
    assert_eq!(
        runs("cmd-/", vcx).as_deref(),
        Some("workspace::ShowShortcuts")
    );

    // The same from the menu bar's "Reset All Keyboard Shortcuts…", which
    // asks in a dialog of the system.
    press_toast_button(&workspace, "Undo", vcx);
    assert!(left(vcx).is_empty());
    vcx.dispatch_action(innerword_app::actions::ResetAllShortcuts);
    vcx.run_until_parked();
    assert!(vcx.has_pending_prompt());
    vcx.simulate_prompt_answer("Reset All");
    vcx.run_until_parked();
    assert_eq!(installed(vcx), defaults);
}

// ---------------------------------------------------------------------
// 0.5.1: what the keymap knows, and what the user is told
// ---------------------------------------------------------------------

/// A workspace like [`workspace_in_the_text`] with the assistant shown
/// (on the fake engine: no test runs Claude Code).
fn workspace_with_the_assistant(
    cx: &mut TestAppContext,
) -> (Entity<Library>, Entity<Workspace>, &mut VisualTestContext) {
    init(cx);
    let backend = std::sync::Arc::new(innerword_assistant::FakeBackend::new());
    cx.update(|cx| innerword_app::assistant::set_backend(backend, cx));
    let library = library(cx);
    add_document(
        &library,
        None,
        &simple_document("Keys", "A paragraph with a few words in it."),
        cx,
    );
    let (workspace, vcx) = open_workspace(&library, cx);
    vcx.run_until_parked();
    assert!(workspace.read_with(vcx, |workspace, _| workspace.assistant_shown()));
    (library, workspace, vcx)
}

/// The name of a place is a promise: two rows of the list under one place
/// name are in the same place to whoever reads it. Over the real keymap,
/// with every part of the app shown:
///
/// - for every pair of rows under one place name, the key of the one
///   given to the other is judged "taken" (or refused), and the row that
///   has the key is the one named;
/// - wherever a verdict says "another place" of a command, the place
///   names of the two differ.
///
/// (0.5.0 said of Underline on the key of "Add or edit link", both listed
/// "In the text": "also used in another place ... in the text", with Keep
/// Both as the marked choice.)
#[gpui::test]
fn two_rows_under_one_place_name_are_in_the_same_place(cx: &mut TestAppContext) {
    let (_library, _workspace, vcx) = workspace_with_the_assistant(cx);
    vcx.update(|_, cx| {
        let listing = innerword_app::workspace::shortcuts_listing(cx).unwrap();
        let keymap = &listing.keymap;
        let rows = collect_shortcuts(cx);
        let label = |target: &Target| -> &'static str {
            rows.iter()
                .find(|row| row.targets.contains(target))
                .map(|row| row.place.label())
                .unwrap_or_else(|| panic!("{target} is in no row"))
        };
        let contexts = |row: &innerword_app::shortcuts::Shortcut| -> String {
            let mut contexts: Vec<String> = row
                .targets
                .iter()
                .map(|target| match target.context.text() {
                    Some(context) => context.to_string(),
                    None => "anywhere".to_string(),
                })
                .collect();
            contexts.sort();
            contexts.join(" | ")
        };
        // What was found wrong, by the key contexts of the two rows.
        let mut wrong: std::collections::BTreeMap<String, Vec<String>> = Default::default();
        let mut pairs = 0;
        let mut taken = 0;

        // 1. Every pair of rows under one place name.
        for row in &rows {
            let Some(target) = &row.target else { continue };
            for other in &rows {
                if other.place != row.place
                    || other.target.is_none()
                    || other.action_name() == row.action_name()
                {
                    continue;
                }
                for key in &other.all_keys {
                    pairs += 1;
                    let verdict = keymap.check(target, &key.keys, None);
                    if verdict.refusal.is_some() {
                        continue;
                    }
                    let named = verdict
                        .taken
                        .iter()
                        .any(|holder| other.targets.contains(&holder.target));
                    if named {
                        taken += 1;
                    } else {
                        wrong
                            .entry(format!(
                                "not taken, {}: [{}] given a key of [{}]",
                                row.place.label(),
                                contexts(row),
                                contexts(other)
                            ))
                            .or_default()
                            .push(format!(
                                "{} given {} of {}",
                                row.title, key.keys, other.title
                            ));
                    }
                }
            }
        }

        // 2. Every key of the keymap, for one row of each kind (a place
        //    name and its key contexts: what is said of a holder's place
        //    depends on nothing else).
        let mut every_key: Vec<KeySequence> = rows
            .iter()
            .flat_map(|row| row.all_keys.iter().map(|key| key.keys.clone()))
            .collect();
        every_key.sort();
        every_key.dedup();
        let mut kinds: Vec<String> = Vec::new();
        let mut elsewhere = 0;
        for row in &rows {
            let Some(target) = &row.target else { continue };
            let kind = format!("{}: {}", row.place.label(), contexts(row));
            if kinds.contains(&kind) {
                continue;
            }
            kinds.push(kind);
            for key in &every_key {
                let verdict = keymap.check(target, key, None);
                for holder in &verdict.elsewhere {
                    elsewhere += 1;
                    if label(&holder.target) == row.place.label() {
                        wrong
                            .entry(format!(
                                "another place under the same name, {}: [{}] and [{}] ({:?})",
                                row.place.label(),
                                contexts(row),
                                holder.target.context,
                                holder.overlap
                            ))
                            .or_default()
                            .push(format!(
                                "{} given {} of {}",
                                row.title, key, holder.target.command.action
                            ));
                    }
                }
            }
        }

        let report: Vec<String> = wrong
            .iter()
            .map(|(what, cases)| format!("{what}: {} cases, e.g. {}", cases.len(), cases[0]))
            .collect();
        assert!(
            wrong.is_empty(),
            "{} kinds of pairs are wrong:\n{}",
            report.len(),
            report.join("\n")
        );
        // The test looked at something.
        assert!(pairs > 10_000, "{pairs}");
        assert!(taken > 5_000, "{taken}");
        assert!(kinds.len() > 30, "{}", kinds.len());
        assert!(elsewhere > 1_000, "{elsewhere}");
    });
}

/// The titles of the rows the list shows for the command `action`.
fn rows_of_command(action: &str, cx: &mut VisualTestContext) -> Vec<String> {
    cx.update(|_, cx| {
        collect_shortcuts(cx)
            .into_iter()
            .filter(|shortcut| shortcut.action_name() == action)
            .map(|shortcut| format!("{} ({})", shortcut.title, shortcut.place.label()))
            .collect()
    })
}

/// What the keymap, as the list has it, says about giving `target` `keys`.
fn verdict(target: &Target, keys: &str, cx: &mut VisualTestContext) -> innerword_keymap::Verdict {
    cx.update(|_, cx| {
        innerword_app::workspace::shortcuts_listing(cx)
            .expect("the app has a keymap")
            .keymap
            .check(target, &key(keys), None)
    })
}

fn switch_the_assistant(workspace: &Entity<Workspace>, on: bool, cx: &mut VisualTestContext) {
    workspace.update(cx, |workspace, cx| {
        workspace.write_setting(setting_keys::ASSISTANT_ENABLED, &on, cx)
    });
    cx.run_until_parked();
    assert_eq!(
        workspace.read_with(cx, |workspace, _| workspace.assistant_shown()),
        on
    );
}

fn version_history_is_open(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> bool {
    workspace.read_with(cx, |workspace, cx| {
        workspace
            .modal_layer()
            .read(cx)
            .active::<innerword_app::version_history::VersionHistory>()
            .is_some()
    })
}

/// While the assistant is not shown, nothing of it is: its commands are
/// no rows of the list, and their keys count for nothing in what the list
/// says about a key. The user uses those keys freely and is not told
/// about commands that are not there. (0.5.0 put a warning on the row of
/// a command that was given ⌥⌘N: "also the key of “New chat” in the
/// assistant's chats", with no assistant on the Mac.) The user's own
/// changes to the assistant's keys are kept, and when the assistant is
/// shown again the list as it is then says what collides.
#[gpui::test]
fn while_the_assistant_is_not_shown_its_keys_count_for_nothing(cx: &mut TestAppContext) {
    use innerword_keymap::VerdictKind;
    let (library, workspace, vcx) = workspace_with_the_assistant(cx);
    let history = Target::new("workspace::ShowVersionHistory", None);
    let ask = Target::new("workspace::AskAssistant", None);
    let new_chat = Target::new("assistant::NewChat", Some("AssistantPanel"));
    let clear_chat = Target::new("assistant::ClearChat", Some("AssistantPanel"));

    // Shown: its commands are rows, and their keys are taken.
    assert_eq!(
        rows_of_command("assistant::NewChat", vcx),
        ["New chat (In the assistant's chats)"]
    );
    assert_eq!(verdict(&history, "cmd-j", vcx).kind(), VerdictKind::Taken);
    assert_eq!(
        verdict(&history, "cmd-alt-k", vcx).kind(),
        VerdictKind::Taken
    );
    // A change of the user's own to one of them.
    change(&workspace, vcx, |keymap| {
        keymap.replace_key(&new_chat, &key("cmd-alt-n"), key("cmd-alt-y"))
    });
    let stored_before = stored_shortcuts(&library, vcx).expect("the change is stored");

    // Not shown.
    switch_the_assistant(&workspace, false, vcx);
    assert!(rows_of_command("assistant::NewChat", vcx).is_empty());
    assert!(rows_of_command("workspace::AskAssistant", vcx).is_empty());
    vcx.update(|_, cx| {
        let listing = innerword_app::workspace::shortcuts_listing(cx).unwrap();
        for target in [&ask, &new_chat, &clear_chat] {
            assert!(listing.keymap.is_absent(target), "{target}");
            assert!(
                listing.rows.iter().all(|row| row.target != *target),
                "{target}"
            );
        }
        assert!(!listing.keymap.is_absent(&history));
    });
    // Their keys are free: the default of one, the user's own of another,
    // and the key that opens the quick chat anywhere.
    for keys in ["cmd-j", "cmd-3", "cmd-alt-k", "cmd-alt-y", "cmd-shift-a"] {
        let verdict = verdict(&history, keys, vcx);
        assert!(
            verdict
                .taken
                .iter()
                .chain(&verdict.elsewhere)
                .all(|holder| !holder.target.command.action.contains("ssistant")),
            "{keys}: {verdict:?}"
        );
    }
    assert_eq!(verdict(&history, "cmd-j", vcx).kind(), VerdictKind::Fine);
    assert_eq!(
        verdict(&history, "cmd-alt-k", vcx).kind(),
        VerdictKind::Fine
    );

    // In the list: the key is simply used. No command of the assistant is
    // named, nothing is to be taken from anybody.
    let view = open_list_at(&workspace, &history, vcx);
    press(vcx, "enter");
    press(vcx, "cmd-j");
    let (title, says, choices) = question(&view, vcx);
    assert!(
        !says.to_lowercase().contains("assistant"),
        "{title}: {says}"
    );
    assert!(
        !choices.iter().any(|choice| choice.starts_with("Take It")),
        "{choices:?}"
    );
    choose(&view, Choice::UseKey, vcx);
    assert_eq!(row_keys("Version history", Place::Anywhere, vcx), ["cmd-j"]);
    // No row is warned, and the keymap has no conflict.
    let described = view.read_with(vcx, |view, cx| view.describe(cx));
    assert!(!described.contains("clash="), "{described}");
    vcx.update(|_, cx| {
        let listing = innerword_app::workspace::shortcuts_listing(cx).unwrap();
        assert!(listing.keymap.collisions().is_empty());
    });
    press(vcx, "escape");
    assert!(editor_focused(&workspace, vcx));
    // And the key does what the user gave it to: the hidden command's own
    // binding is in the keymap still, and does not eat the key.
    vcx.simulate_keystrokes("cmd-j");
    vcx.run_until_parked();
    assert!(version_history_is_open(&workspace, vcx));
    press(vcx, "escape");
    assert!(editor_focused(&workspace, vcx));

    // The user's change to the hidden command is untouched: stored,
    // installed, and in the file that is exported now.
    let stored = stored_shortcuts(&library, vcx).expect("changes are stored");
    let entry_of = |stored: &serde_json::Value, command: &str| -> serde_json::Value {
        stored["changes"]
            .as_array()
            .expect("the changes")
            .iter()
            .find(|entry| entry["command"] == command)
            .cloned()
            .unwrap_or_else(|| panic!("{command} is stored: {stored}"))
    };
    assert_eq!(
        entry_of(&stored, "assistant::NewChat"),
        entry_of(&stored_before, "assistant::NewChat")
    );
    vcx.update(|_, cx| {
        let effective = innerword_app::keymap::effective(cx).unwrap();
        assert_eq!(
            effective.target(&new_chat).unwrap().keys[0].keys,
            key("cmd-alt-y")
        );
        assert_eq!(effective.target(&ask).unwrap().keys[0].keys, key("cmd-j"));
    });
    let out = tempfile::tempdir().expect("a temporary directory");
    let exported = export_to(&out.path().join("Inner Word Shortcuts.json"), vcx);
    let entry = exported["shortcuts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["command"] == "assistant::NewChat")
        .expect("the file has the hidden command");
    assert_eq!(entry["keys"], serde_json::json!(["cmd-alt-y"]));
    assert_eq!(entry["where"], "In the assistant's chats");

    // Shown again: nothing was changed behind the user's back, and the
    // list as it is now says what collides.
    switch_the_assistant(&workspace, true, vcx);
    assert_eq!(row_keys("Version history", Place::Anywhere, vcx), ["cmd-j"]);
    assert_eq!(
        row_keys("Ask the assistant", Place::Anywhere, vcx),
        ["cmd-j"]
    );
    assert_eq!(
        row_keys("New chat", Place::AssistantChat, vcx),
        ["cmd-alt-y"]
    );
    vcx.update(|_, cx| {
        let listing = innerword_app::workspace::shortcuts_listing(cx).unwrap();
        let collisions = listing.keymap.collisions();
        assert_eq!(collisions.len(), 1, "{collisions:#?}");
        let pair = [&collisions[0].target, &collisions[0].holder.target];
        assert!(pair.contains(&&history) && pair.contains(&&ask), "{pair:?}");
    });
    let view = open_list_at(&workspace, &history, vcx);
    let described = view.read_with(vcx, |view, cx| view.describe(cx));
    assert!(
        described.contains("clash=") && described.contains("Ask the assistant"),
        "{described}"
    );
}

/// The shortcuts file has every command, the assistant's too while the
/// assistant is not shown, with the same entries in the same order: a
/// file means the same on every Mac, a change to a key of the assistant
/// is not lost because the assistant was off when the file was written,
/// and two files can be compared. (In 0.5.0 the assistant's entries came
/// at the end of a file written without the assistant, and among the
/// others in one written with it.)
#[gpui::test]
fn the_shortcuts_file_is_the_same_whether_the_assistant_is_shown_or_not(cx: &mut TestAppContext) {
    let out = tempfile::tempdir().expect("a temporary directory");
    let (_library, workspace, vcx) = workspace_with_the_assistant(cx);
    change(&workspace, vcx, |keymap| {
        keymap.replace_key(
            &Target::new("assistant::NewChat", Some("AssistantPanel")),
            &key("cmd-alt-n"),
            key("cmd-alt-y"),
        );
        keymap.replace_key(&bold(), &key("cmd-b"), key("cmd-alt-b"))
    });
    let shown = export_to(&out.path().join("shown.json"), vcx);
    switch_the_assistant(&workspace, false, vcx);
    let hidden = export_to(&out.path().join("hidden.json"), vcx);

    let entries = |file: &serde_json::Value| -> Vec<String> {
        file["shortcuts"]
            .as_array()
            .expect("the shortcuts")
            .iter()
            .map(|entry| format!("{} in {}", entry["command"], entry["context"]))
            .collect()
    };
    assert_eq!(entries(&shown), entries(&hidden), "the same order");
    assert_eq!(shown, hidden, "the same file");
    // Every key context of the keymap is an entry, and the assistant's
    // are among them, where the list has them when it shows them.
    let targets = vcx.update(|_, cx| {
        innerword_app::keymap::model(cx)
            .unwrap()
            .defaults()
            .targets()
            .len()
    });
    assert_eq!(entries(&hidden).len(), targets);
    let assistant: Vec<usize> = hidden["shortcuts"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .filter(|(_, entry)| entry["where"] == "In the assistant's chats")
        .map(|(index, _)| index)
        .collect();
    assert!(assistant.len() > 30, "{}", assistant.len());
    assert!(
        *assistant.last().unwrap() < targets - 20,
        "not at the end of the file: {assistant:?}"
    );
    let new_chat = hidden["shortcuts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["command"] == "assistant::NewChat")
        .unwrap();
    assert_eq!(new_chat["keys"], serde_json::json!(["cmd-alt-y"]));
}

/// A file that changes a key of a command that is not shown now: the
/// change is taken (the file means the same on every Mac), the preview
/// does not read as if the user could find that command, and no collision
/// with a hidden command is counted.
#[gpui::test]
fn an_import_names_a_hidden_command_as_not_shown(cx: &mut TestAppContext) {
    let out = tempfile::tempdir().expect("a temporary directory");
    let file = out.path().join("Inner Word Shortcuts.json");
    let (_library, workspace, vcx) = workspace_in_the_text(cx);
    assert!(!workspace.read_with(vcx, |workspace, _| workspace.assistant_shown()));
    let new_chat = Target::new("assistant::NewChat", Some("AssistantPanel"));
    let history = Target::new("workspace::ShowVersionHistory", None);

    // A file that gives "New chat" another key, and the version history
    // the key of "Ask the assistant".
    let mut exported = export_to(&file, vcx);
    for entry in exported["shortcuts"].as_array_mut().unwrap() {
        if entry["command"] == "assistant::NewChat" {
            entry["keys"] = serde_json::json!(["cmd-alt-y"]);
        }
        if entry["command"] == "workspace::ShowVersionHistory" {
            entry["keys"] = serde_json::json!(["cmd-j"]);
        }
    }
    std::fs::write(&file, exported.to_string()).unwrap();
    import_from(&file, vcx);
    let view = list(&workspace, vcx);
    let rows = view.read_with(vcx, |view, _| view.preview_rows());
    // The hidden command is named with where it stands: not shown now.
    vcx.update(|_, cx| {
        let listing = innerword_app::workspace::shortcuts_listing(cx).unwrap();
        let info = listing.catalogue.info(&new_chat).expect("it has a name");
        assert_eq!(info.title, "New chat");
        assert_eq!(info.place, "In the assistant's chats, not shown now");
        // A command that is shown has its place as it is.
        assert_eq!(listing.catalogue.info(&history).unwrap().place, "Anywhere");
    });
    assert!(
        rows.iter()
            .any(|row| row.contains("New chat") && row.contains("not shown now")),
        "{rows:?}"
    );
    assert!(
        rows.iter()
            .any(|row| row.contains("Version history") && !row.contains("not shown")),
        "{rows:?}"
    );
    // Two commands change; no collision with the hidden "Ask the
    // assistant" is counted.
    assert!(rows[0].ends_with("(2)"), "{rows:?}");
    assert_eq!(rows.len(), 3, "a heading and two rows: {rows:?}");
    assert!(
        !rows.iter().any(|row| row.contains("Ask the assistant")),
        "{rows:?}"
    );
    choose(&view, Choice::ImportMerge, vcx);
    vcx.update(|_, cx| {
        let effective = innerword_app::keymap::effective(cx).unwrap();
        assert_eq!(
            effective.target(&new_chat).unwrap().keys[0].keys,
            key("cmd-alt-y")
        );
        assert_eq!(
            effective.target(&history).unwrap().keys[0].keys,
            key("cmd-j")
        );
    });
    assert_eq!(toasts(&workspace, vcx), ["Imported the keys of 2 commands"]);
}

fn is_underlined(workspace: &Entity<Workspace>, needle: &str, cx: &mut VisualTestContext) -> bool {
    workspace.read_with(cx, |workspace, cx| {
        let editor = workspace.editor().read(cx);
        editor
            .document()
            .blocks()
            .iter()
            .find_map(|block| {
                let offset = block.text().find(needle)?;
                Some(block.attrs_at(offset).is_some_and(|attrs| attrs.underline))
            })
            .unwrap_or_else(|| panic!("{needle:?} is not in the document"))
    })
}

/// "Underline" and "Add or edit link" are both listed "In the text"; the
/// link's key is bound on the key context of the tools around the text.
/// 0.5.0 said of ⌘K for Underline: "also used in another place ... in the
/// text", with Keep Both marked, and an import of the same change listed
/// no collision. What really happens, held here: with both on ⌘K the key
/// underlines in the text and the link field can no longer be opened with
/// it there. So the key is taken, the list offers to take it and not to
/// keep both, and the import of the same change lists the same collision.
#[gpui::test]
fn a_command_of_the_text_on_a_key_of_the_tools_has_taken_it(cx: &mut TestAppContext) {
    use innerword_keymap::{ImportMode, Overlap, VerdictKind};
    let (_library, workspace, vcx) = workspace_in_the_text(cx);
    let underline = Target::new("editor::ToggleUnderline", Some("Editor"));
    let link = Target::new("editor_tools::OpenLink", Some("EditorTools"));
    assert_eq!(
        runs("cmd-k", vcx).as_deref(),
        Some("editor_tools::OpenLink")
    );

    // The question.
    let judged = verdict(&underline, "cmd-k", vcx);
    assert_eq!(judged.kind(), VerdictKind::Taken);
    assert!(judged.elsewhere.is_empty(), "{judged:?}");
    assert_eq!(judged.taken.len(), 1);
    assert_eq!(judged.taken[0].target, link);
    assert_eq!(judged.taken[0].overlap, Overlap::SamePlace);
    // The other way round, a tool on a key of the text: taken too.
    let judged = verdict(&link, "cmd-u", vcx);
    assert_eq!(judged.kind(), VerdictKind::Taken);
    assert_eq!(judged.taken[0].target, underline);
    // In the list: taken, with nothing to keep both.
    let view = open_list_at(&workspace, &underline, vcx);
    press(vcx, "enter");
    press(vcx, "cmd-k");
    let (title, says, choices) = question(&view, vcx);
    assert_eq!(title, "⌘K is taken", "{says}");
    assert!(says.contains("Add or edit link"), "{says}");
    assert!(!says.contains("another place"), "{says}");
    assert!(!choices.contains(&"Keep Both".to_string()), "{choices:?}");
    assert!(
        choices.iter().any(|choice| choice.starts_with("Take It")),
        "{choices:?}"
    );
    choose(&view, Choice::Cancel, vcx);
    press(vcx, "escape");
    assert!(editor_focused(&workspace, vcx));

    // The plan of an import of the same change names the same collision.
    let (text, conflicts) = vcx.update(|_, cx| {
        let mut model = innerword_app::keymap::model(cx).unwrap().clone();
        model.replace_key(&underline, &key("cmd-u"), key("cmd-k"));
        let catalogue = innerword_app::workspace::shortcuts_file_catalogue(cx);
        let text = model.export(&catalogue, "0.5.1");
        let plan = innerword_app::keymap::model(cx)
            .unwrap()
            .plan_import(&text)
            .expect("the file is read");
        (text, plan.outcome(ImportMode::Merge).conflicts.clone())
    });
    assert!(text.contains("editor::ToggleUnderline"));
    assert_eq!(conflicts.len(), 1, "{conflicts:#?}");
    assert_eq!(conflicts[0].target, underline);
    assert_eq!(conflicts[0].holder.target, link);

    // What really happens with both on the key (a script, or an import
    // carried out in spite of the collision, leaves the keymap so).
    workspace
        .update_in(vcx, |workspace, window, cx| {
            workspace.set_keys_of_command(
                "editor::ToggleUnderline",
                None,
                Some(&[key("cmd-k")]),
                window,
                cx,
            )
        })
        .expect("underline is listed");
    vcx.run_until_parked();
    // In the text the key underlines, and the link field stays closed.
    assert_eq!(
        runs("cmd-k", vcx).as_deref(),
        Some("editor::ToggleUnderline")
    );
    vcx.simulate_keystrokes("cmd-a cmd-k");
    vcx.run_until_parked();
    assert!(is_underlined(&workspace, "paragraph", vcx));
    assert!(
        editor_focused(&workspace, vcx),
        "no link field took the keyboard"
    );
    // The keymap says so about both rows.
    vcx.update(|_, cx| {
        let collisions = innerword_app::keymap::model(cx).unwrap().collisions();
        assert_eq!(collisions.len(), 1, "{collisions:#?}");
        assert_eq!(collisions[0].target, underline);
        assert_eq!(collisions[0].holder.target, link);
    });
    // The one place the tools' binding still answers is where the
    // keyboard is in something else the tools opened (the find bar's
    // field): nowhere a link is added from.
    vcx.simulate_keystrokes("cmd-f");
    vcx.run_until_parked();
    assert!(!editor_focused(&workspace, vcx));
    assert_eq!(
        runs("cmd-k", vcx).as_deref(),
        Some("editor_tools::OpenLink")
    );
}

/// What the app says in numbers about changed shortcuts counts commands
/// (a row of the list), never single keys, in one wording with a singular
/// and a plural: the row of the settings, the system dialog of "Reset All
/// Keyboard Shortcuts…", the toast after an import, the state line of a
/// scripted run.
#[gpui::test]
fn every_count_of_changed_shortcuts_counts_commands(cx: &mut TestAppContext) {
    use innerword_app::automation::describe_state;
    use innerword_app::settings_view::{SettingRow, SettingsView};
    use innerword_app::workspace::{
        commands_counted, imported_toast, own_keys_sentence, reset_all_detail,
    };
    // The wording, singular and plural.
    assert_eq!(commands_counted(1), "1 command");
    assert_eq!(commands_counted(4), "4 commands");
    assert_eq!(own_keys_sentence(0), None);
    assert_eq!(
        own_keys_sentence(1).as_deref(),
        Some("1 command has keys of your own.")
    );
    assert_eq!(
        own_keys_sentence(4).as_deref(),
        Some("4 commands have keys of your own.")
    );
    assert_eq!(
        reset_all_detail(1),
        "1 command goes back to the default keys. Every change you made is removed."
    );
    assert_eq!(
        reset_all_detail(4),
        "4 commands go back to the default keys. Every change you made is removed."
    );
    assert_eq!(imported_toast(1), "Imported the keys of 1 command");
    assert_eq!(imported_toast(5), "Imported the keys of 5 commands");

    // In the app: Italic with two keys of the user's is one command,
    // Underline left without a key is one, a row of two key contexts is
    // one. Three commands, five key contexts, four keys.
    let (_library, workspace, vcx) = workspace_in_the_text(cx);
    let italic = Target::new("editor::ToggleItalic", Some("Editor"));
    let underline = Target::new("editor::ToggleUnderline", Some("Editor"));
    change(&workspace, vcx, |keymap| {
        keymap.set_keys(&italic, &[key("cmd-alt-i"), key("f5")]);
        keymap.remove_key(&underline, &key("cmd-u"));
    });
    workspace
        .update_in(vcx, |workspace, window, cx| {
            workspace.set_keys_of_command(
                "editor_tools::ToggleWholeWord",
                None,
                Some(&[key("cmd-alt-y")]),
                window,
                cx,
            )
        })
        .expect("whole word is listed");
    vcx.run_until_parked();
    assert_eq!(
        vcx.update(|_, cx| innerword_app::keymap::changed_count(cx)),
        4
    );
    assert_eq!(
        vcx.update(|_, cx| innerword_app::workspace::changed_command_count(cx)),
        3
    );
    let line = workspace.update_in(vcx, |workspace, window, cx| {
        describe_state(workspace, window, cx)
    });
    assert!(line.contains("keys=[changed=3]"), "{line}");
    // The row of the settings.
    press(vcx, "cmd-,");
    let settings = workspace
        .read_with(vcx, |workspace, cx| {
            workspace.modal_layer().read(cx).active::<SettingsView>()
        })
        .expect("the settings are open");
    let says = settings.read_with(vcx, |settings, cx| {
        settings
            .description(SettingRow::KeyboardShortcuts, false, cx)
            .to_string()
    });
    assert!(
        says.ends_with(" 3 commands have keys of your own."),
        "{says}"
    );
    press(vcx, "escape");
    // The system dialog of the menu bar's and the palette's "reset all".
    vcx.dispatch_action(innerword_app::actions::ResetAllShortcuts);
    vcx.run_until_parked();
    let (question, detail) = vcx.pending_prompt().expect("the dialog is shown");
    assert_eq!(question, "Put every keyboard shortcut back to its default?");
    assert_eq!(
        detail,
        "3 commands go back to the default keys. Every change you made is removed."
    );
    vcx.simulate_prompt_answer("Cancel");
    vcx.run_until_parked();

    // The toast after an import counts the same way.
    let out = tempfile::tempdir().expect("a temporary directory");
    let file = out.path().join("Inner Word Shortcuts.json");
    export_to(&file, vcx);
    workspace.update_in(vcx, |workspace, window, cx| {
        workspace.reset_all_shortcuts_now(window, cx)
    });
    vcx.run_until_parked();
    import_from(&file, vcx);
    let view = list(&workspace, vcx);
    choose(&view, Choice::ImportMerge, vcx);
    assert_eq!(toasts(&workspace, vcx), ["Imported the keys of 3 commands"]);
}

/// The list against the keymap and the file. Every command in every key
/// context the keymap has is in exactly one row; a row can stand for
/// several key contexts, and one row (the `/` of the insert menu) is no
/// binding. The shortcuts file has one entry for every key context. So,
/// with every part of the app shown: rows = commands in places + 1, and
/// entries = rows - 1 + the key contexts that share a row with another.
/// (0.5.0 had three entries in the file that were no row of the list:
/// the menu bar's Hide, Hide Others and Minimize.)
#[gpui::test]
fn every_key_of_the_keymap_is_a_row_and_an_entry_of_the_file(cx: &mut TestAppContext) {
    let out = tempfile::tempdir().expect("a temporary directory");
    let (_library, _workspace, vcx) = workspace_with_the_assistant(cx);
    let exported = export_to(&out.path().join("Inner Word Shortcuts.json"), vcx);
    let entries = exported["shortcuts"].as_array().unwrap();
    vcx.update(|_, cx| {
        let rows = collect_shortcuts(cx);
        let model = innerword_app::keymap::model(cx).unwrap();
        let targets = model.defaults().targets();
        // Each key context of the keymap in one row, and only once.
        let mut in_rows: Vec<&Target> = Vec::new();
        for row in &rows {
            for target in &row.targets {
                assert!(!in_rows.contains(&target), "{target} is in two rows");
                in_rows.push(target);
            }
            assert_eq!(
                row.target.is_none(),
                row.targets.is_empty(),
                "{}",
                row.title
            );
            if let Some(target) = &row.target {
                assert_eq!(
                    &row.targets[0], target,
                    "{}: the first stands for it",
                    row.title
                );
            }
        }
        let missing: Vec<String> = targets
            .iter()
            .filter(|target| !in_rows.contains(target))
            .map(|target| target.to_string())
            .collect();
        assert!(missing.is_empty(), "in no row of the list: {missing:?}");
        assert_eq!(in_rows.len(), targets.len());

        // The file: one entry for each key context, in the order of the
        // rows.
        assert_eq!(entries.len(), targets.len());
        let of_the_file: Vec<String> = entries
            .iter()
            .map(|entry| format!("{} in {}", entry["command"], entry["context"]))
            .collect();
        let of_the_rows: Vec<String> = in_rows
            .iter()
            .map(|target| {
                format!(
                    "{} in {}",
                    serde_json::json!(target.command.action),
                    serde_json::json!(target.context.text())
                )
            })
            .collect();
        assert_eq!(of_the_file, of_the_rows);

        // The numbers, and where they come from.
        let fixed = rows.iter().filter(|row| row.target.is_none()).count();
        let sharing: usize = rows
            .iter()
            .map(|row| row.targets.len().saturating_sub(1))
            .sum();
        // The / that opens the insert menu, a row for each thing a
        // whiteboard's own key does, and its tool keys.
        let board_rows = {
            let mut whats: Vec<&str> = innerword_board_web::keys::WHITEBOARD_KEYS
                .iter()
                .map(|key| key.what)
                .collect();
            whats.dedup();
            whats.len()
        };
        assert_eq!(fixed, 1 + board_rows + 1);
        assert_eq!(entries.len(), rows.len() - fixed + sharing);
        // No two rows read the same: a title and a place are one row.
        let mut named: Vec<(String, &str)> = rows
            .iter()
            .map(|row| (row.title.to_string(), row.place.label()))
            .collect();
        named.sort();
        let before = named.len();
        named.dedup();
        assert_eq!(named.len(), before, "two rows with one title in one place");
        // Titles and places of the file are the rows'.
        let mut of_the_file: Vec<(String, String)> = entries
            .iter()
            .map(|entry| {
                (
                    entry["title"].as_str().unwrap().to_string(),
                    entry["where"].as_str().unwrap().to_string(),
                )
            })
            .collect();
        of_the_file.sort();
        of_the_file.dedup();
        assert_eq!(of_the_file.len(), rows.len() - fixed);
        // The keys of the menu bar that are no commands of the palette
        // are rows like the others, and cannot be run from the list.
        for title in [
            "Hide Inner Word",
            "Hide the other apps",
            "Minimize the window",
        ] {
            let row = rows
                .iter()
                .find(|row| row.title.as_ref() == title)
                .unwrap_or_else(|| panic!("{title} is a row"));
            assert_eq!(row.place, Place::Anywhere);
            assert!(!row.described, "{title} is not run from its row");
        }
        eprintln!(
            "rows={} (1 fixed) entries={} key contexts that share a row={} rows of several={}",
            rows.len(),
            entries.len(),
            sharing,
            rows.iter().filter(|row| row.targets.len() > 1).count()
        );
    });
}

/// A key is written the same by the keymap's own glyphs (what the reason
/// of a skipped entry of an import uses) and by the app's key hints (what
/// the list shows), for every key of the keymap and a few more.
#[gpui::test]
fn the_keymap_writes_a_key_as_the_list_shows_it(cx: &mut TestAppContext) {
    init(cx);
    cx.update(|cx| {
        let effective = innerword_app::keymap::effective(cx).unwrap();
        let mut keys: Vec<KeySequence> = effective
            .bindings()
            .iter()
            .map(|binding| binding.keys.clone())
            .collect();
        for more in [
            "i",
            "shift-b",
            "cmd-tab",
            "cmd-space",
            "fn-f",
            "ctrl-cmd-alt-shift-f12",
            "g g",
            "cmd-k cmd-shift-7",
            "insert",
            "cmd-pageup",
            "cmd-home",
            "shift-end",
            "cmd-delete",
            "alt-ö",
            "cmd--",
            "cmd-shift--",
            "f35",
        ] {
            keys.push(key(more));
        }
        keys.sort();
        keys.dedup();
        assert!(keys.len() > 150, "{}", keys.len());
        for keys in &keys {
            assert_eq!(
                keys.glyphs(),
                innerword_app::shortcuts::key_text(keys),
                "{keys}"
            );
            assert_eq!(keys.glyphs(), innerword_keymap_ui::glyphs(keys), "{keys}");
        }
    });
}

/// What GPUI does with a key of the window against a key the library has
/// "wherever the keyboard is in it" (the key context `LibraryPanel`
/// without `!TextField`: the arrows, the page keys, `tab`), which the
/// description of the places has only in part (`keymap::places`, "What is
/// not exact").
///
/// The library has the keyboard itself (on a row) or in its filter field.
/// A binding without a key context ranks like the deepest one. On a row
/// the library's binding is at that same rank and comes later: it keeps
/// the key, as the keymap says. In the filter field it lies one level
/// further out: the window's command gets a key that the field does not
/// bind itself. The keymap ranks the library's context as if the keyboard
/// were always on it, so its sentence about such a pair ("where both apply
/// the library's command keeps it") is true of the rows and not of the
/// filter field. Whether a key is taken does not rest on that sentence
/// for any default key: every key the library binds this way is also a
/// key of a list or a dialog that never has the keyboard itself, which a
/// key of the window does take.
#[gpui::test]
fn a_key_of_the_window_against_a_key_of_the_whole_library(cx: &mut TestAppContext) {
    use innerword_keymap::{Overlap, VerdictKind};
    let (_library, workspace, vcx) = workspace_in_the_text(cx);
    // A command of the window that has no key by default stands at the
    // front of the keymap: every binding of the library comes later.
    let history = Target::new("workspace::ShowVersionHistory", None);
    let focus_tree = Target::new("library_panel::FocusTree", Some("LibraryPanel"));

    // What the keymap says about the pair.
    let judged = verdict(&history, "tab", vcx);
    let library = judged
        .elsewhere
        .iter()
        .find(|holder| holder.target == focus_tree)
        .expect("the library's command is named");
    assert_eq!(library.overlap, Overlap::HolderWinsThere);

    // What GPUI does.
    change(&workspace, vcx, |keymap| {
        keymap.add_key(&history, key("tab"))
    });
    vcx.simulate_keystrokes("cmd-0");
    vcx.run_until_parked();
    assert_eq!(
        runs("tab", vcx).as_deref(),
        Some("library_panel::FocusTree"),
        "on a row the library keeps the key"
    );
    vcx.simulate_keystrokes("cmd-f");
    vcx.run_until_parked();
    let stack = vcx.update(|window, _| format!("{:?}", window.context_stack()));
    assert!(stack.ends_with("TextField]"), "{stack}");
    assert_eq!(
        runs("tab", vcx).as_deref(),
        Some("workspace::ShowVersionHistory"),
        "in the filter field the window's command gets it"
    );
    // A key the field binds itself is the field's first, there.
    change(&workspace, vcx, |keymap| {
        keymap.add_key(&history, key("pagedown"))
    });
    assert_eq!(
        runs("pagedown", vcx).as_deref(),
        Some("text_field::PageDown")
    );

    // The verdict's kind does not rest on the inexact sentence: for every
    // key the library has wherever the keyboard is in it, a command of
    // the window that asks for it is told "taken" (a list or a dialog has
    // the key too, and would lose it) or is refused.
    vcx.update(|_, cx| {
        let listing = innerword_app::workspace::shortcuts_listing(cx).unwrap();
        let effective = listing.keymap.effective();
        let comments = Target::new("comments::ToggleComments", None);
        let mut asked = 0;
        for state in effective.targets() {
            if state.target.context.text() != Some("LibraryPanel") {
                continue;
            }
            for default in &state.default_keys {
                for window_wide in [&history, &comments] {
                    asked += 1;
                    let judged = listing.keymap.check(window_wide, default, None);
                    assert!(
                        matches!(judged.kind(), VerdictKind::Taken | VerdictKind::Refused),
                        "{window_wide} given {default} of {}: {judged:?}",
                        state.target
                    );
                }
            }
        }
        assert!(asked >= 30, "{asked}");
    });
}

/// The toast after an export names the file in a form that fits: the
/// folders in the middle of a long path are left out, the file name is
/// whole, and the message is drawn inside the toast. (0.5.0: "Exported the
/// keyboard shortcuts to /private/tmp/claude-501/-Users-…", cut off at the
/// right edge of the toast.) The same for the export of a document.
#[gpui::test]
fn the_toast_after_an_export_names_a_path_that_fits(cx: &mut TestAppContext) {
    use innerword_app::export::{
        ExportChoice, ExportDestination, ExportScope, SHORT_PATH_CHARS, short_path,
    };
    use innerword_export::ExportFormat;
    let out = tempfile::tempdir().expect("a temporary directory");
    let folder = out
        .path()
        .join("Writing and other things")
        .join("Books of the last ten years")
        .join("The Harbour at the End of the World")
        .join("Exports for the editor");
    std::fs::create_dir_all(&folder).unwrap();
    let (_library, workspace, vcx) = workspace_in_the_text(cx);

    let drawn = |workspace: &Entity<Workspace>, cx: &mut VisualTestContext| {
        cx.run_until_parked();
        workspace.read_with(cx, |workspace, cx| {
            let layer = workspace.toast_layer().read(cx);
            let id = *layer.ids().last().expect("a toast is shown");
            (
                layer.toasts().last().unwrap().message.to_string(),
                layer.drawn_message(id).expect("the toast was drawn"),
            )
        })
    };
    let check = |message: &str, before: &str, file: &std::path::Path| {
        let path = message
            .strip_prefix(before)
            .unwrap_or_else(|| panic!("{message}"));
        let name = file.file_name().unwrap().to_string_lossy();
        assert_eq!(path, short_path(file), "{message}");
        assert!(path.ends_with(&format!("/{name}")), "{message}");
        assert!(path.contains("/\u{2026}/"), "{message}");
        assert!(path.chars().count() <= SHORT_PATH_CHARS, "{message}");
        assert!(!path.contains("Books of the last ten years"), "{message}");
    };

    // The shortcuts.
    let file = folder.join("Inner Word Shortcuts.json");
    export_to(&file, vcx);
    let (message, shown) = drawn(&workspace, vcx);
    check(&message, "Exported the keyboard shortcuts to ", &file);
    assert!(!shown.is_cut(), "{shown:?}");
    assert_eq!(shown.text, message);
    assert!(shown.lines.len() <= 2, "{shown:?}");
    for line in &shown.lines {
        assert!(*line <= shown.width + gpui::px(0.5), "{shown:?}");
    }

    // A document.
    workspace.update_in(vcx, |workspace, window, cx| {
        workspace
            .toast_layer()
            .update(cx, |layer, cx| layer.clear(cx));
        workspace.export(
            ExportChoice::new(
                ExportScope::Document,
                ExportFormat::Markdown,
                ExportDestination::File,
            ),
            window,
            cx,
        );
    });
    vcx.run_until_parked();
    let file = folder.join("Keys.md");
    let target = file.clone();
    vcx.simulate_new_path_selection(move |_| Some(target));
    vcx.run_until_parked();
    assert!(file.exists());
    let (message, shown) = drawn(&workspace, vcx);
    check(&message, "Exported \u{201c}Keys\u{201d} to ", &file);
    assert!(!shown.is_cut(), "{shown:?}");
    for line in &shown.lines {
        assert!(*line <= shown.width + gpui::px(0.5), "{shown:?}");
    }

    // A message that is far too long for any toast ends in an ellipsis
    // inside the toast (an error with a long reason).
    workspace.update(vcx, |workspace, cx| {
        workspace.toast_layer().update(cx, |layer, cx| {
            layer.clear(cx);
            layer.push(
                innerword_ui::Toast::new(format!(
                    "Could not write {}: {}",
                    short_path(&file),
                    "the volume is read-only and ".repeat(20)
                ))
                .error(),
                cx,
            );
        })
    });
    let (_, shown) = drawn(&workspace, vcx);
    assert!(shown.is_cut(), "{shown:?}");
    assert_eq!(shown.lines.len(), innerword_ui::toast::MESSAGE_LINES);
    for line in &shown.lines {
        assert!(*line <= shown.width + gpui::px(0.5), "{shown:?}");
    }
}

/// An open list of keyboard shortcuts follows when the assistant is found
/// (or switched off) behind it: its rows, and what it says about a key.
#[gpui::test]
fn an_open_list_follows_when_the_assistant_is_shown_or_hidden(cx: &mut TestAppContext) {
    let (_library, workspace, vcx) = workspace_with_the_assistant(cx);
    let history = Target::new("workspace::ShowVersionHistory", None);
    let new_chat = Target::new("assistant::NewChat", Some("AssistantPanel"));
    let view = open_list_at(&workspace, &new_chat, vcx);
    let with_it = view.read_with(vcx, |view, _| view.listed().len());
    switch_the_assistant(&workspace, false, vcx);
    let without_it = view.read_with(vcx, |view, _| view.listed().len());
    assert!(
        without_it + 25 < with_it,
        "{with_it} rows with the assistant, {without_it} without"
    );
    assert!(
        !view.update(vcx, |view, cx| view.select_target(&new_chat, cx)),
        "the row of a hidden command is gone"
    );
    // The open list asks the keymap that knows the assistant is hidden.
    assert!(view.update(vcx, |view, cx| view.select_target(&history, cx)));
    press(vcx, "enter");
    press(vcx, "cmd-alt-k");
    let (title, says, choices) = question(&view, vcx);
    assert!(
        !says.to_lowercase().contains("clear chat"),
        "{title}: {says}"
    );
    assert!(
        !choices.iter().any(|choice| choice.starts_with("Take It")),
        "{choices:?}"
    );
    choose(&view, Choice::Cancel, vcx);
    // And back.
    switch_the_assistant(&workspace, true, vcx);
    assert_eq!(view.read_with(vcx, |view, _| view.listed().len()), with_it);
    assert!(view.update(vcx, |view, cx| view.select_target(&new_chat, cx)));
}

// ----- the zoom (0.6.0) -------------------------------------------------------------

/// The six commands of the zoom that are named in the menus (in, out,
/// actual size, fit width, fit page, life size) are rows of the list like
/// any other: each is given another key there with the keyboard, the new
/// key runs it and the old one no longer does, and the menu bar and the
/// menu of the zoom in the status bar show the new key. A key another
/// command has is named with that command before anything changes.
#[gpui::test]
fn the_zoom_commands_are_given_other_keys_in_the_list(cx: &mut TestAppContext) {
    use gpui::{Modifiers, px, size};
    use innerword_app::zoom;
    use innerword_editor::Zoom;
    use innerword_ui::{ContextMenu, KeyHint};

    init(cx);
    // A display like this Mac's, so that there is a life size: 177 %.
    cx.update(|cx| zoom::set_display_density(Some(127.5), cx));
    let library = library(cx);
    add_document(
        &library,
        None,
        &simple_document("Keys", "A paragraph with a few words in it."),
        cx,
    );
    let (workspace, vcx) = open_workspace(&library, cx);
    vcx.simulate_resize(size(px(1900.), px(1100.)));
    vcx.run_until_parked();
    assert!(editor_focused(&workspace, vcx));
    let zoom_now =
        |cx: &mut VisualTestContext| -> Zoom { workspace.read_with(cx, |w, cx| w.zoom(cx)) };
    let target = |action: &str| Target::new(action, None);

    // The defaults: three have keys, three have none. Zoom in has two:
    // the plus key with `cmd`, with shift (which is how "+" is typed on
    // most keyboards) and without.
    let zoom_in_default = ["cmd-=", "cmd-shift-="];
    assert_eq!(row_keys("Zoom in", Place::Anywhere, vcx), zoom_in_default);
    assert_eq!(row_keys("Zoom out", Place::Anywhere, vcx), ["cmd--"]);
    assert_eq!(
        row_keys("Zoom to actual size", Place::Anywhere, vcx),
        ["cmd-shift-0"]
    );
    for title in ["Zoom to fit width", "Zoom to fit page", "Zoom to life size"] {
        assert!(row_keys(title, Place::Anywhere, vcx).is_empty(), "{title}");
    }

    // A key that is taken is said to be taken, and by whom: zoom out is
    // given the key of zoom in, and zoom in a key Bold has in the text.
    let view = open_list_at(&workspace, &target("zoom::ZoomOut"), vcx);
    press(vcx, "enter");
    press(vcx, "cmd-=");
    let (title, says, choices) = question(&view, vcx);
    assert_eq!(title, "⌘= is taken");
    assert!(says.contains("Zoom in"), "{says}");
    assert!(
        choices.iter().any(|choice| choice.contains("Zoom in")),
        "{choices:?}"
    );
    choose(&view, Choice::Cancel, vcx);
    assert!(view.update(vcx, |view, cx| {
        view.select_target(&target("zoom::ZoomIn"), cx)
    }));
    press(vcx, "enter");
    press(vcx, "cmd-b");
    let (title, says, choices) = question(&view, vcx);
    assert!(says.contains("Bold"), "{title}: {says}");
    assert!(
        choices.contains(&"Keep Both".to_string()),
        "{title}: {says} {choices:?}"
    );
    choose(&view, Choice::Cancel, vcx);
    press(vcx, "escape");
    assert_eq!(
        vcx.update(|_, cx| innerword_app::keymap::changed_count(cx)),
        0
    );

    // Each of the six gets a key of its own, in the list, with the
    // keyboard.
    let changes: [(&str, &str, &str, &str); 6] = [
        ("zoom::ZoomIn", "Zoom in", "cmd-alt-=", "⌥⌘="),
        ("zoom::ZoomOut", "Zoom out", "cmd-alt--", "⌥⌘-"),
        ("zoom::ActualSize", "Zoom to actual size", "f6", "F6"),
        ("zoom::FitWidth", "Zoom to fit width", "f7", "F7"),
        ("zoom::FitPage", "Zoom to fit page", "f8", "F8"),
        ("zoom::LifeSize", "Zoom to life size", "f9", "F9"),
    ];
    for (action, title, keys, _) in changes {
        let view = open_list_at(&workspace, &target(action), vcx);
        press(vcx, "enter");
        assert_eq!(
            view.read_with(vcx, |view, _| view.mode()),
            ModeKind::Recording,
            "{title}"
        );
        press(vcx, keys);
        let (heading, says, choices) = question(&view, vcx);
        assert!(heading.ends_with("is free"), "{title}: {heading}: {says}");
        choose(&view, Choice::UseKey, vcx);
        assert_eq!(
            view.read_with(vcx, |view, _| view.mode()),
            ModeKind::List,
            "{title}: {choices:?}"
        );
        // The key the row shows first is the one that is replaced: zoom
        // in keeps the second of its two.
        let expected = if action == "zoom::ZoomIn" {
            vec!["cmd-=", keys]
        } else {
            vec![keys]
        };
        assert_eq!(row_keys(title, Place::Anywhere, vcx), expected, "{title}");
        press(vcx, "escape");
        assert!(!has_modal(&workspace, vcx));
        assert!(editor_focused(&workspace, vcx));
    }
    // The second key of zoom in still zooms in, as its row says; then it
    // is taken away too.
    assert_eq!(runs("cmd-=", vcx).as_deref(), Some("zoom::ZoomIn"));
    assert_eq!(runs("cmd-shift-=", vcx), None);
    assert!(change(&workspace, vcx, |keymap| {
        keymap.remove_key(&target("zoom::ZoomIn"), &key("cmd-="))
    }));
    assert_eq!(row_keys("Zoom in", Place::Anywhere, vcx), ["cmd-alt-="]);
    assert_eq!(
        vcx.update(|_, cx| innerword_app::workspace::changed_command_count(cx)),
        6
    );

    // The new keys run the commands, and the old ones run nothing.
    for old in ["cmd-=", "cmd-+", "cmd--", "cmd-shift-0", "cmd-)"] {
        assert_eq!(runs(old, vcx), None, "{old}");
    }
    assert_eq!(zoom_now(vcx), Zoom::Percent(100));
    vcx.simulate_keystrokes("cmd-= cmd-+");
    assert_eq!(zoom_now(vcx), Zoom::Percent(100), "the old keys do nothing");
    vcx.simulate_keystrokes("cmd-alt-= cmd-alt-=");
    assert_eq!(zoom_now(vcx), Zoom::Percent(125));
    vcx.simulate_keystrokes("cmd-alt--");
    assert_eq!(zoom_now(vcx), Zoom::Percent(110));
    vcx.simulate_keystrokes("f6");
    assert_eq!(zoom_now(vcx), Zoom::Percent(100));
    vcx.simulate_keystrokes("f7");
    assert_eq!(zoom_now(vcx), Zoom::FitWidth);
    vcx.simulate_keystrokes("f9");
    assert_eq!(zoom_now(vcx), Zoom::Percent(177));
    // Fit page is for page view.
    vcx.simulate_keystrokes("cmd-alt-p");
    vcx.run_until_parked();
    vcx.simulate_keystrokes("f8");
    assert_eq!(zoom_now(vcx), Zoom::FitPage);
    // They work wherever the keyboard is, as the defaults do.
    vcx.simulate_keystrokes("cmd-0 f6");
    assert_eq!(zoom_now(vcx), Zoom::Percent(100));
    vcx.simulate_keystrokes("cmd-1");
    vcx.run_until_parked();

    // The menu bar shows them, and so do the hints of the palette.
    let in_the_menu_bar = [
        (
            menu_key(&zoom::ZoomIn, vcx),
            hint(&zoom::ZoomIn, vcx),
            "alt-cmd-=",
        ),
        (
            menu_key(&zoom::ZoomOut, vcx),
            hint(&zoom::ZoomOut, vcx),
            "alt-cmd--",
        ),
        (
            menu_key(&zoom::ActualSize, vcx),
            hint(&zoom::ActualSize, vcx),
            "f6",
        ),
        (
            menu_key(&zoom::FitWidth, vcx),
            hint(&zoom::FitWidth, vcx),
            "f7",
        ),
        (
            menu_key(&zoom::FitPage, vcx),
            hint(&zoom::FitPage, vcx),
            "f8",
        ),
        (
            menu_key(&zoom::LifeSize, vcx),
            hint(&zoom::LifeSize, vcx),
            "f9",
        ),
    ];
    for ((menu, hint, expected), (_, title, _, shown)) in in_the_menu_bar.iter().zip(changes) {
        assert_eq!(menu.as_deref(), Some(*expected), "{title} in the menu bar");
        assert_eq!(hint.as_deref(), Some(shown), "the hint of {title}");
    }

    // The menu of the zoom in the status bar: its items are the commands,
    // each drawn with the key the keymap has for it now.
    let at = vcx
        .debug_bounds("status-zoom")
        .expect("the zoom is in the status bar")
        .center();
    vcx.simulate_click(at, Modifiers::none());
    vcx.run_until_parked();
    let drawn: Vec<(String, Option<String>)> = vcx.update(|window, cx| {
        let menu = ContextMenu::current(window, cx).expect("the menu of the zoom is open");
        menu.read(cx)
            .items()
            .iter()
            .filter_map(|item| {
                let action = item.action_ref()?;
                Some((
                    action.name().to_string(),
                    KeyHint::for_action(action, cx).map(|hint| hint.text()),
                ))
            })
            .collect()
    });
    for (action, title, _, shown) in changes {
        let item = drawn
            .iter()
            .find(|(name, _)| name == action)
            .unwrap_or_else(|| panic!("{title} is not in the menu of the zoom: {drawn:?}"));
        assert_eq!(item.1.as_deref(), Some(shown), "{title} in the status bar");
    }
    // The sizes without a key show none.
    assert!(
        drawn
            .iter()
            .any(|(name, hint)| name == "zoom::ZoomTo150" && hint.is_none()),
        "{drawn:?}"
    );
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();

    // One of them back to its default: the row has its two keys again and
    // the others keep theirs.
    change(&workspace, vcx, |keymap| {
        keymap.reset(&target("zoom::ZoomIn"))
    });
    assert_eq!(row_keys("Zoom in", Place::Anywhere, vcx), zoom_in_default);
    assert_eq!(runs("cmd-alt-=", vcx), None);
    assert_eq!(runs("cmd-=", vcx).as_deref(), Some("zoom::ZoomIn"));
    assert_eq!(runs("f7", vcx).as_deref(), Some("zoom::FitWidth"));
    assert_eq!(menu_key(&zoom::ZoomIn, vcx).as_deref(), Some("cmd-+"));
}
