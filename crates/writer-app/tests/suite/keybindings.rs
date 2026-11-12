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

fn take_apart(cx: &gpui::App) -> Vec<Parts> {
    let keymap = cx.key_bindings();
    let keymap = keymap.borrow();
    keymap
        .bindings()
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
        // Without the rebuild `init` does itself: the keymap as the crates
        // bound it.
        writer_app::init(cx);
        let parts = take_apart(cx);
        assert!(parts.len() > 300, "the whole keymap: {}", parts.len());

        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        let mut with_meta = 0;
        let mut with_input = 0;
        let mut chords = 0;
        for (original, parts) in keymap.bindings().zip(&parts) {
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
        // index or an action input (nothing loads a keymap file), a few are
        // sequences of two keys.
        assert_eq!(with_meta, 0);
        assert_eq!(with_input, 0);
        assert!(chords >= 3, "the menu bar's two-key placeholders: {chords}");
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
        let parts = take_apart(cx);
        cx.clear_key_bindings();
        assert_eq!(cx.key_bindings().borrow().bindings().len(), 0);
        cx.bind_keys(parts.iter().map(put_together));
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
