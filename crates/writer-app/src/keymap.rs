//! The keymap as the app's own thing: the defaults every crate bound, the
//! user's changes to them, and the keymap that is installed.
//!
//! Every crate binds its keys in its own `init`, in the order
//! [`crate::init`] calls them. When they are all bound, [`take_defaults`]
//! reads the assembled keymap, in order, and keeps it as the defaults.
//! From then on the keymap GPUI has is always one this module built:
//!
//! 1. the bindings that only give menu bar items their key
//!    ([`crate::menus::menu_key_bindings`], computed from what follows);
//! 2. the defaults in their order, each one as it was bound, replaced by
//!    the user's key where the user changed it, or left out where the user
//!    removed it. A changed key stands where its default stood, so every
//!    precedence fact of the keymap holds as before: a binding without a
//!    context still ranks like the deepest context, and among the bindings
//!    at the focused element's own level the later one still wins.
//!
//! A binding is rebuilt from its parts with [`gpui::KeyBinding::load`]: the
//! action (a copy), the key context (the same predicate), no key
//! equivalents, the same mapper `KeyBinding::new` uses. A test holds that a
//! binding rebuilt this way is the binding it was.

use std::rc::Rc;

use gpui::{
    Action, App, DummyKeyboardMapper, Global, KeyBinding, KeyBindingContextPredicate, KeyContext,
    Keymap,
};
use writer_ui::KeyHint;

use crate::menus;

/// See the [module docs](self).
#[derive(Default)]
pub struct AppKeymap {
    /// What every crate bound in `init`, in order.
    defaults: Vec<KeyBinding>,
}

impl Global for AppKeymap {}

/// Takes the keymap as it is assembled right now as the defaults and
/// installs it again, rebuilt. Called once, at the end of [`crate::init`],
/// before the menu bar is made.
pub fn take_defaults(cx: &mut App) {
    let defaults: Vec<KeyBinding> = {
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        keymap.bindings().cloned().collect()
    };
    cx.set_global(AppKeymap { defaults });
    install(cx);
}

/// The default bindings, in the order the crates bound them.
pub fn defaults(cx: &App) -> &[KeyBinding] {
    cx.try_global::<AppKeymap>()
        .map(|keymap| keymap.defaults.as_slice())
        .unwrap_or_default()
}

/// The keys of a binding as text GPUI parses again (`cmd-shift-p`, and
/// `ctrl-x ctrl-s` for a sequence).
pub fn keys_of(binding: &KeyBinding) -> String {
    binding
        .keystrokes()
        .iter()
        .map(|key| key.unparse())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The key context of a binding as text (`None`: it has none).
pub fn context_of(binding: &KeyBinding) -> Option<String> {
    binding.predicate().map(|predicate| predicate.to_string())
}

/// The key the app shows for `action` in one place of the window: where
/// the key contexts `place` hold (one name, or several from the outside in:
/// `"FindBar > TextField"`). Among the bindings of the action that apply
/// there, one bound for a key context comes before one that works anywhere,
/// and the one bound last before the others, as everywhere a key is shown.
/// `None` when the action has no key there (the user may have taken it
/// away): whatever names the key then leaves it out.
///
/// For the texts that name a key outside a list of commands: the footer
/// of a modal, a tooltip, a sentence. They are drawn from the keymap each
/// time, so they follow a change at once.
pub fn hint_in(action: &dyn Action, place: &str, cx: &App) -> Option<KeyHint> {
    let stack: Vec<KeyContext> = place
        .split('>')
        .filter_map(|name| KeyContext::parse(name.trim()).ok())
        .collect();
    let keymap = cx.key_bindings();
    let keymap = keymap.borrow();
    let mut anywhere = None;
    let mut here = None;
    for binding in keymap.bindings_for_action(action) {
        match binding.predicate() {
            None => anywhere = Some(binding),
            Some(predicate) if predicate.depth_of(&stack).is_some() => here = Some(binding),
            Some(_) => {}
        }
    }
    here.or(anywhere).map(KeyHint::from_binding)
}

/// The keys of several actions in one place, as one text: `"↑ ↓"` for
/// "previous" and "next". An action without a key there is left out;
/// `None` when none of them has one.
pub fn keys_text_in(actions: &[&dyn Action], place: &str, cx: &App) -> Option<String> {
    let keys: Vec<String> = actions
        .iter()
        .filter_map(|action| hint_in(*action, place, cx))
        .map(|hint| hint.text())
        .collect();
    (!keys.is_empty()).then(|| keys.join(" "))
}

/// `binding` on other keys: the same action, key context, meta index and
/// action input. `None` when `keys` is not a key GPUI can read.
pub fn rebound(binding: &KeyBinding, keys: &str) -> Option<KeyBinding> {
    bind(
        keys,
        binding.action(),
        binding.predicate(),
        binding.meta(),
        binding.action_input(),
    )
}

/// A binding made from its parts, the way `KeyBinding::new` makes one.
pub fn bind(
    keys: &str,
    action: &dyn Action,
    predicate: Option<Rc<KeyBindingContextPredicate>>,
    meta: Option<gpui::KeyBindingMetaIndex>,
    input: Option<gpui::SharedString>,
) -> Option<KeyBinding> {
    // No key at all is not a binding.
    keys.split_whitespace().next()?;
    let mut binding = KeyBinding::load(
        keys,
        action.boxed_clone(),
        predicate,
        false,
        input,
        &DummyKeyboardMapper,
    )
    .ok()?;
    if let Some(meta) = meta {
        binding.set_meta(meta);
    }
    Some(binding)
}

/// The bindings that run, in order: the defaults with the user's changes.
/// Every one is built from its parts, changed or not, so there is one way
/// a binding gets into the keymap.
fn effective(keymap: &AppKeymap) -> Vec<KeyBinding> {
    keymap
        .defaults
        .iter()
        .map(|binding| rebound(binding, &keys_of(binding)).unwrap_or_else(|| binding.clone()))
        .collect()
}

/// Clears GPUI's keymap and binds it again: the menu bar's own bindings
/// first, then the effective ones in the order of the defaults. Binding
/// keys refreshes every window.
fn install(cx: &mut App) {
    let effective = cx
        .try_global::<AppKeymap>()
        .map(effective)
        .unwrap_or_default();
    let for_menus = menus::menu_key_bindings(&Keymap::new(effective.clone()));
    cx.clear_key_bindings();
    cx.bind_keys(for_menus.into_iter().chain(effective));
}
