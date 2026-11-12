//! Asking GPUI's keymap what a key does, for code that sees a key before
//! the keymap does.
//!
//! The keymap is the one place that says which key runs what, and it can
//! change while the app runs: the user's own shortcuts are applied by
//! clearing GPUI's keymap and binding it again with other keystrokes. So
//! nothing may remember a key or compare against one written in the code.
//! What is drawn asks the keymap when it is drawn ([`crate::KeyHint`]). A
//! handler that takes a key before the bindings are tried (an interceptor
//! on an editor, the open context menu) asks here whether the key it was
//! handed is a key of the action it stands in for:
//!
//! ```ignore
//! // The composer of a comment: its interceptor runs before the editor's
//! // own bindings, and still obeys whatever "submit" is bound to.
//! composer.read(cx).intercept_keys(InterceptPriority::DEFAULT, move |key, window, cx| {
//!     if keymap::keystroke_runs(key, &SubmitComment, window, cx) {
//!         ..
//!         return KeyDisposition::Consumed;
//!     }
//!     KeyDisposition::Pass
//! });
//! ```
//!
//! [`replace_key`] is the rebuild itself in miniature. The tests of every
//! crate use it to prove that the crate shows and obeys a rebuilt keymap
//! without a restart.

use gpui::{Action, App, DummyKeyboardMapper, KeyBinding, KeyContext, Keystroke, Window};

/// Whether `keystroke`, pressed where the keyboard is now, is a key of
/// `action`: among the bindings that match in the key contexts of the
/// focused element (as of the last frame) there is one that runs it.
///
/// A binding that a nearer one outranks counts too. That is what an
/// interceptor needs: it exists because another binding on the same key
/// would win (the editor's own `cmd-enter` inside the composer of a
/// comment), so "would GPUI run this action for this key" is the wrong
/// question, and "is this key bound to this action here" the right one.
///
/// Actions are compared by value, so an action with fields finds its own
/// binding. Only single keystrokes: a chord is never one key.
pub fn keystroke_runs(
    keystroke: &Keystroke,
    action: &dyn Action,
    window: &Window,
    cx: &App,
) -> bool {
    keystroke_runs_in(keystroke, action, &window.context_stack(), cx)
}

/// [`keystroke_runs`] for a place given by its key contexts, outermost
/// first, instead of by where the keyboard is.
pub fn keystroke_runs_in(
    keystroke: &Keystroke,
    action: &dyn Action,
    context_stack: &[KeyContext],
    cx: &App,
) -> bool {
    let keymap = cx.key_bindings();
    let keymap = keymap.borrow();
    let (bindings, _) = keymap.bindings_for_input(std::slice::from_ref(keystroke), context_stack);
    bindings
        .iter()
        .any(|binding| binding.action().partial_eq(action))
}

/// The first action, in the keymap's order of precedence, that `keystroke`
/// is bound to in `context_stack` and that `wanted` accepts.
///
/// For a list that is driven without having the keyboard (the insert menu
/// of the text, which is filtered by what is typed into the document): it
/// asks what the key would do in a list (`["Picker"]`) and takes the
/// answer if it is one of the list's own actions.
pub fn action_for_keystroke_in(
    keystroke: &Keystroke,
    context_stack: &[KeyContext],
    cx: &App,
    wanted: impl Fn(&dyn Action) -> bool,
) -> Option<Box<dyn Action>> {
    let keymap = cx.key_bindings();
    let keymap = keymap.borrow();
    let (bindings, _) = keymap.bindings_for_input(std::slice::from_ref(keystroke), context_stack);
    bindings
        .iter()
        .find(|binding| wanted(binding.action()))
        .map(|binding| binding.action().boxed_clone())
}

/// Whether `keystroke` is bound, in any key context, to an action that
/// `wanted` accepts.
///
/// For a handler that has no place of its own in the element tree (the
/// open context menu, which never takes the keyboard): it cannot ask what
/// the key does "here".
pub fn keystroke_is_bound_to(
    keystroke: &Keystroke,
    cx: &App,
    wanted: impl Fn(&dyn Action) -> bool,
) -> bool {
    let keymap = cx.key_bindings();
    let keymap = keymap.borrow();
    keymap.bindings().any(|binding| {
        binding.match_keystrokes(std::slice::from_ref(keystroke)) == Some(false)
            && wanted(binding.action())
    })
}

/// Rebuilds the whole keymap with `to` in place of `from` for `action`:
/// every binding is taken out and bound again in the order it had, and
/// each binding of `action` (compared by value) on the keystrokes `from`
/// gets the keystrokes `to` instead, where it stood, in the same key
/// context. `to: None` leaves the action without that key. Returns how
/// many bindings were changed.
///
/// This is how a change of the user's shortcuts reaches GPUI (clear, then
/// bind again), reduced to one key. Keeping the order keeps every rule of
/// precedence that depends on it.
///
/// # Panics
///
/// When `from` or `to` is not GPUI keystroke syntax (`"cmd-alt-s"`), as
/// `KeyBinding::new` does.
pub fn replace_key(action: &dyn Action, from: &str, to: Option<&str>, cx: &mut App) -> usize {
    let from: Vec<Keystroke> = from
        .split_whitespace()
        .map(|source| Keystroke::parse(source).expect("`from` is a keystroke"))
        .collect();
    let bindings: Vec<KeyBinding> = cx.key_bindings().borrow().bindings().cloned().collect();
    let mut changed = 0;
    let mut rebuilt = Vec::with_capacity(bindings.len());
    for binding in bindings {
        let is_it =
            binding.action().partial_eq(action) && binding.match_keystrokes(&from) == Some(false);
        if !is_it {
            rebuilt.push(binding);
            continue;
        }
        changed += 1;
        let Some(to) = to else {
            continue;
        };
        let mut replacement = KeyBinding::load(
            to,
            binding.action().boxed_clone(),
            binding.predicate(),
            false,
            binding.action_input(),
            &DummyKeyboardMapper,
        )
        .expect("`to` is a keystroke");
        if let Some(meta) = binding.meta() {
            replacement.set_meta(meta);
        }
        rebuilt.push(replacement);
    }
    cx.clear_key_bindings();
    cx.bind_keys(rebuilt);
    changed
}

/// One key context by name, for the `context_stack` of
/// [`keystroke_runs_in`] and [`action_for_keystroke_in`].
///
/// # Panics
///
/// When `name` is not a key context (`"Picker"`, `"Editor mode=full"`).
pub fn context(name: &str) -> KeyContext {
    KeyContext::parse(name).expect("a key context")
}
