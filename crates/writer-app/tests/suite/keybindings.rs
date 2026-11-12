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
