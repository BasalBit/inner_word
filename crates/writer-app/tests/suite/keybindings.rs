//! Keyboard shortcuts the user can change.
//!
//! The first tests are the experiment everything else rests on: the keymap
//! every crate assembled in `writer_app::init` can be taken apart (keys,
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
    writer_app::keymap::defaults(cx)
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
        writer_app::init(cx);
        let parts = take_apart(cx);
        assert!(parts.len() > 300, "the whole keymap: {}", parts.len());

        let mut with_meta = 0;
        let mut with_input = 0;
        let mut chords = 0;
        for (original, parts) in writer_app::keymap::defaults(cx).iter().zip(&parts) {
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
            .filter(|binding| writer_app::menus::is_menu_key_binding(binding))
            .cloned()
            .collect::<Vec<_>>();
        let parts = take_apart(cx);
        assert_eq!(before.len(), menu_only.len() + parts.len());
        cx.clear_key_bindings();
        assert_eq!(cx.key_bindings().borrow().bindings().len(), 0);
        cx.bind_keys(menu_only.into_iter().chain(parts.iter().map(put_together)));
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
    use writer_app::menus;
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
        ];
        let context =
            std::rc::Rc::new(KeyBindingContextPredicate::parse(menus::MENU_KEY_CONTEXT).unwrap());
        let old: Vec<KeyBinding> = by_hand
            .iter()
            .map(|(name, keys)| {
                let action = cx.build_action(name, None).expect("a registered action");
                writer_app::keymap::bind(keys, action.as_ref(), Some(context.clone()), None, None)
                    .expect("a key GPUI reads")
            })
            .chain(writer_app::keymap::defaults(cx).iter().cloned())
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
use writer_app::{
    menus,
    settings::keys as setting_keys,
    shortcuts::{Place, collect_shortcuts},
};
use writer_keymap::{KeySequence, Target};
use writer_keymap_ui::{Choice, KeymapView, ModeKind};

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
    change: impl FnOnce(&mut writer_keymap::Keymap) -> R,
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
    cx.update(|_, cx| writer_ui::KeyHint::for_action(action, cx).map(|hint| hint.text()))
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
            .filter(|binding| !menus::is_menu_key_binding(binding))
            .map(|binding| {
                (
                    keys_of(binding),
                    binding.action().name().to_string(),
                    binding.predicate().map(|predicate| predicate.to_string()),
                )
            })
            .collect();
        let defaults: Vec<(String, String, Option<String>)> = writer_app::keymap::defaults(cx)
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
        assert_eq!(writer_app::keymap::changed_count(cx), 0);
        // Everything is handed over to be changed, except the two keys of
        // the window that stands in when the library cannot be opened.
        let model = writer_app::keymap::model(cx).expect("the keymap as data");
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
        hint(&writer_editor::actions::ToggleBold, vcx).as_deref(),
        Some("⌥⌘B")
    );
    assert_eq!(row_keys("Bold", Place::Text, vcx), ["cmd-alt-b"]);
    assert_eq!(
        menu_key(&writer_editor::actions::ToggleBold, vcx).as_deref(),
        Some("alt-cmd-b")
    );
    let (command_hint, changed) = vcx.update(|_, cx| {
        let commands = writer_app::commands::all(cx);
        let bold = commands
            .iter()
            .find(|command| command.action_name() == "editor::ToggleBold")
            .expect("Bold is a command");
        (
            bold.key_hint(cx).map(|hint| hint.text()),
            writer_app::keymap::changed_count(cx),
        )
    });
    assert_eq!(command_hint.as_deref(), Some("⌥⌘B"));
    assert_eq!(changed, 1);
    // The text menu, opened from the keyboard: its item for Bold shows
    // the key the keymap has when the menu is made.
    vcx.simulate_keystrokes("ctrl-enter");
    vcx.run_until_parked();
    let in_menu = vcx.update(|window, cx| {
        let menu = writer_ui::ContextMenu::current(window, cx).expect("the text menu is open");
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
            None => writer_ui::KeyHint::for_action(&writer_editor::actions::ToggleBold, cx)
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

    assert_eq!(hint(&writer_editor::actions::ToggleItalic, vcx), None);
    assert_eq!(menu_key(&writer_editor::actions::ToggleItalic, vcx), None);
    assert!(row_keys("Italic", Place::Text, vcx).is_empty());
    // The command is still there, and still runs from the palette.
    vcx.dispatch_action(writer_editor::actions::ToggleItalic);
    vcx.run_until_parked();
    assert!(is_italic(&workspace, "paragraph", vcx));
    // No binding of any kind is left for it: the menu bar has nothing to
    // answer `cmd-i` with.
    vcx.update(|_, cx| {
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        assert_eq!(
            keymap
                .bindings_for_action(&writer_editor::actions::ToggleItalic)
                .count(),
            0
        );
    });
}

#[gpui::test]
fn a_key_given_to_a_command_without_one_works(cx: &mut TestAppContext) {
    use writer_app::version_history::VersionHistory;
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
        hint(&writer_app::actions::ShowVersionHistory, vcx).as_deref(),
        Some("⌃⌥H")
    );
    assert_eq!(
        menu_key(&writer_app::actions::ShowVersionHistory, vcx).as_deref(),
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
            .any(|block| *block.kind() == writer_core::BlockKind::Divider)
    }));
    assert_eq!(row_keys("Divider", Place::Text, vcx), ["cmd-alt-d"]);
}

#[gpui::test]
fn a_key_bound_in_two_spellings_is_changed_as_one(cx: &mut TestAppContext) {
    let (_library, workspace, vcx) = workspace_in_the_text(cx);
    let numbered = Target::new("editor::ToggleNumberedList", Some("Editor"));
    let action = writer_editor::actions::ToggleNumberedList;
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
    assert_eq!(vcx.update(|_, cx| writer_app::keymap::changed_count(cx)), 1);

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
            .filter(|binding| !menus::is_menu_key_binding(binding))
            .map(keys_of)
            .collect();
        let defaults: Vec<String> = writer_app::keymap::defaults(cx)
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
    assert_eq!(vcx.update(|_, cx| writer_app::keymap::changed_count(cx)), 0);
    assert!(!is_italic(&workspace, "paragraph", vcx));

    // Cancel: nothing changes.
    choose(&view, Choice::Cancel, vcx);
    assert_eq!(view.read_with(vcx, |view, _| view.mode()), ModeKind::List);
    assert_eq!(vcx.update(|_, cx| writer_app::keymap::changed_count(cx)), 0);

    // Take it from that command.
    press(vcx, "enter");
    press(vcx, "cmd-i");
    choose(&view, Choice::TakeKey, vcx);
    assert_eq!(row_keys("Bold", Place::Text, vcx), ["cmd-i"]);
    assert!(row_keys("Italic", Place::Text, vcx).is_empty());
    assert_eq!(vcx.update(|_, cx| writer_app::keymap::changed_count(cx)), 2);
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
        let effective = writer_app::keymap::effective(cx).unwrap();
        assert!(effective.target(&italic).unwrap().keys.is_empty());
        assert!(
            writer_app::keymap::model(cx)
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
    assert_eq!(vcx.update(|_, cx| writer_app::keymap::changed_count(cx)), 0);
    // Nothing of what was pressed was typed or run.
    assert_eq!(
        editor_text(&workspace, vcx),
        "Keys\nA paragraph with a few words in it."
    );

    // The same as the model says it, and where no text is typed a letter
    // is a key like any other (the library moves with `j`).
    vcx.update(|_, cx| {
        use writer_keymap::Refusal;
        let model = writer_app::keymap::model(cx).unwrap();
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
            [writer_keymap::Warning::SystemUses(_)]
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
    assert_eq!(vcx.update(|_, cx| writer_app::keymap::changed_count(cx)), 2);
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
    assert_eq!(vcx.update(|_, cx| writer_app::keymap::changed_count(cx)), 1);
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
    assert_eq!(vcx.update(|_, cx| writer_app::keymap::changed_count(cx)), 1);
    choose(&view, Choice::ResetAll, vcx);
    assert_eq!(vcx.update(|_, cx| writer_app::keymap::changed_count(cx)), 0);
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
    assert_eq!(vcx.update(|_, cx| writer_app::keymap::changed_count(cx)), 1);
    assert!(row_keys("Italic", Place::Text, vcx).is_empty());
    assert!(stored(vcx).is_some());
    press(vcx, "escape");

    // And from the menu bar or the palette, which asks in a system dialog
    // that no key of the app can break.
    vcx.dispatch_action(writer_app::actions::ResetAllShortcuts);
    vcx.run_until_parked();
    assert!(vcx.has_pending_prompt());
    assert_eq!(vcx.update(|_, cx| writer_app::keymap::changed_count(cx)), 1);
    vcx.simulate_prompt_answer("Reset All");
    vcx.run_until_parked();
    assert_eq!(vcx.update(|_, cx| writer_app::keymap::changed_count(cx)), 0);
    assert_eq!(stored(vcx), None);
    // With nothing changed there is nothing to ask.
    vcx.dispatch_action(writer_app::actions::ResetAllShortcuts);
    vcx.run_until_parked();
    assert!(!vcx.has_pending_prompt());
    // The palette's own undo brings the last reset back as well.
    vcx.dispatch_action(writer_app::actions::UndoShortcutsChange);
    vcx.run_until_parked();
    assert_eq!(vcx.update(|_, cx| writer_app::keymap::changed_count(cx)), 1);
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
            .filter(|binding| !menus::is_menu_key_binding(binding))
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
        writer_app::keymap::set_changes(writer_keymap::Changes::new(), cx);
        assert_eq!(writer_app::keymap::changed_count(cx), 0);
    });

    // The second launch reads them before anything is drawn.
    let library = launch(cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    assert_eq!(installed(vcx), changed);
    assert_eq!(vcx.update(|_, cx| writer_app::keymap::changed_count(cx)), 4);
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
        hint(&writer_editor::actions::ToggleBold, vcx).as_deref(),
        Some("⌥⌘B")
    );
    assert_eq!(
        menu_key(&writer_app::actions::ShowVersionHistory, vcx).as_deref(),
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
        assert_eq!(writer_app::keymap::changed_count(cx), 1);
        let inactive = writer_app::keymap::effective(cx)
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
    assert_eq!(vcx.update(|_, cx| writer_app::keymap::changed_count(cx)), 0);
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
    cx.dispatch_action(writer_app::actions::ExportShortcuts);
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
    cx.update(|_, cx| writer_app::workspace::choose_import_file_beforehand(Some(path), cx));
    cx.dispatch_action(writer_app::actions::ImportShortcuts);
    cx.run_until_parked();
}

#[gpui::test]
fn an_exported_file_imported_into_another_library_gives_the_same_keymap(cx: &mut TestAppContext) {
    let out = tempfile::tempdir().expect("a temporary directory");
    let file = out.path().join("The Writer Shortcuts.json");
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
        writer_app::keymap::model(cx)
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
    assert_eq!(vcx.update(|_, cx| writer_app::keymap::changed_count(cx)), 4);
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
            keymap.set_changes(writer_keymap::Changes::new());
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
    assert_eq!(vcx.update(|_, cx| writer_app::keymap::changed_count(cx)), 1);

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
    assert_eq!(vcx.update(|_, cx| writer_app::keymap::changed_count(cx)), 1);
    assert!(
        toasts(&workspace, vcx)
            .iter()
            .any(|toast| toast == "Imported the keys of one command"),
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
    use writer_editor_tools::ToolKind;
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
            .bindings_for_action(&writer_editor_tools::actions::ToggleCaseSensitive)
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
        let effective = writer_app::keymap::effective(cx).unwrap();
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
    assert_eq!(vcx.update(|_, cx| writer_app::keymap::changed_count(cx)), 0);
    // Removed from the row: from both.
    press(vcx, "cmd-backspace");
    vcx.update(|_, cx| {
        let effective = writer_app::keymap::effective(cx).unwrap();
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
    assert_eq!(vcx.update(|_, cx| writer_app::keymap::changed_count(cx)), 0);
    press(vcx, "enter");
    press(vcx, "cmd-alt-c");
    let (title, says, _) = question(&view, vcx);
    assert_eq!(title, "⌥⌘C is taken", "{says}");
    choose(&view, Choice::TakeKey, vcx);
    vcx.update(|_, cx| {
        let effective = writer_app::keymap::effective(cx).unwrap();
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
    use writer_app::settings_view::{SettingRow, SettingsView};
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
    assert!(
        says.ends_with("One command has a key of your own."),
        "{says}"
    );
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
        writer_app::commands::all(cx)
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
    vcx.dispatch_action(writer_app::actions::ShowShortcuts);
    vcx.run_until_parked();
    assert!(list_open(vcx));
    // The same command closes it again.
    vcx.dispatch_action(writer_app::actions::ShowShortcuts);
    vcx.run_until_parked();
    assert!(!has_modal(&workspace, vcx));

    // The menu bar: The Writer has the four items, Help the list.
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
                &writer_app::actions::ShowShortcuts as &dyn Action,
                &writer_app::actions::ImportShortcuts,
                &writer_app::actions::ExportShortcuts,
                &writer_app::actions::ResetAllShortcuts,
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
    use writer_app::{agents_view, controls, keymap, settings_view, version_history};
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
    use writer_editor::actions as editor;
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
        menu_key(&writer_editor_tools::actions::FindPrevious, vcx),
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
                .filter(|binding| !menus::is_menu_key_binding(binding))
                .map(keys_of)
                .collect();
            assert!(real.contains(&keys_of(native)), "{name}: {real:?}");
        }
    });
}

/// The steps of a scripted run: a key set without the list, and what the
/// state line says.
#[gpui::test]
fn a_script_sets_keys_and_reads_the_state(cx: &mut TestAppContext) {
    use writer_app::automation::{Script, ShortcutsStep, Step, describe_state};
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
    use writer_app::automation::describe_state;
    use writer_app::settings_view::{SettingRow, SettingsView};
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
    assert_eq!(vcx.update(|_, cx| writer_app::keymap::changed_count(cx)), 2);
    // ... which is one row, and one command to the user.
    assert_eq!(view.read_with(vcx, |view, _| view.changed_count()), 1);
    assert_eq!(
        vcx.update(|_, cx| writer_app::workspace::changed_command_count(cx)),
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
    assert!(
        says.ends_with("One command has a key of your own."),
        "{says}"
    );
    press(vcx, "escape");
    // The system dialog of the menu bar's "reset all".
    vcx.dispatch_action(writer_app::actions::ResetAllShortcuts);
    vcx.run_until_parked();
    let (_, detail) = vcx.pending_prompt().expect("the dialog is shown");
    assert_eq!(detail, "One command has another key than its default.");
    vcx.simulate_prompt_answer("Cancel");
    vcx.run_until_parked();

    // A file with that change has the command twice, once for each key
    // context. The preview of its import shows it once, and the toast
    // counts it once.
    let out = tempfile::tempdir().expect("a temporary directory");
    let file = out.path().join("The Writer Shortcuts.json");
    export_to(&file, vcx);
    workspace.update_in(vcx, |workspace, window, cx| {
        workspace.reset_all_shortcuts_now(window, cx)
    });
    vcx.run_until_parked();
    assert_eq!(vcx.update(|_, cx| writer_app::keymap::changed_count(cx)), 0);
    import_from(&file, vcx);
    let view = list(&workspace, vcx);
    let (_, says, _) = question(&view, vcx);
    assert!(says.starts_with("1 command gets other keys."), "{says}");
    let rows = view.read_with(vcx, |view, _| view.preview_rows());
    assert_eq!(rows.len(), 2, "a heading and one row: {rows:?}");
    assert_eq!(rows[0], "Keys that change (1)");
    choose(&view, Choice::ImportMerge, vcx);
    assert_eq!(vcx.update(|_, cx| writer_app::keymap::changed_count(cx)), 2);
    assert_eq!(
        toasts(&workspace, vcx),
        ["Imported the keys of one command"]
    );
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
    let ask = |target: &Target, keys: &str, cx: &mut VisualTestContext| -> String {
        let view = open_list_at(&workspace, target, cx);
        press(cx, "enter");
        press(cx, keys);
        let (_, says, _) = question(&view, cx);
        choose(&view, Choice::Cancel, cx);
        press(cx, "escape");
        says
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
    let says = ask(&bold(), "ctrl-y", vcx);
    assert!(says.contains(vim_says), "{says}");
    assert!(says.contains("the shortcut does not run"), "{says}");
    assert!(ask(&history, "ctrl-y", vcx).contains(vim_says));
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
    let file = out.path().join("The Writer Shortcuts.json");
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
    vcx.dispatch_action(writer_app::actions::ShowShortcuts);
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
    vcx.dispatch_action(writer_app::actions::ResetAllShortcuts);
    vcx.run_until_parked();
    assert!(vcx.has_pending_prompt());
    vcx.simulate_prompt_answer("Reset All");
    vcx.run_until_parked();
    assert_eq!(installed(vcx), defaults);
}
