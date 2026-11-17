//! Key interception: other crates see key-down events of a focused editor
//! before its key bindings and before text input, and may consume them.
//!
//! # Mechanism
//!
//! GPUI 0.2.2 dispatches a key in this order (see `docs/gpui-notes.md`,
//! "Actions and keys"): `App::intercept_keystrokes` callbacks, then keymap
//! bindings, then `on_key_down` listeners, then the platform input handler.
//! Only the first stage sees *every* key: a bound key never reaches
//! `on_key_down`, and bindings win over text input. So each editor installs
//! one `cx.intercept_keystrokes` callback. When the editor's own focus handle
//! is focused in the window that got the key, the callback offers the
//! keystroke to the editor's interceptors in priority order. The first one
//! that returns [`KeyDisposition::Consumed`] ends the dispatch
//! (`cx.stop_propagation()`): no binding, no action, no text input.
//! [`KeyDisposition::Pass`] lets the key continue to the next interceptor
//! and then to normal dispatch.
//!
//! Interceptors run outside any entity update, so a handler may freely
//! `update` the editor, its own entity, or anything else.
//!
//! Not offered to interceptors: the synthetic lone-modifier keystrokes GPUI
//! generates (`shift`, `control`, `alt`, `platform`, `function` pressed and
//! released alone) and keystrokes with an empty key. While an IME composition
//! is active the platform gives keys to the IME first, so interceptors only
//! see what the IME did not take.
//!
//! The registry is separate from the `Editor` entity (shared through an
//! `Rc`), which is what makes registration possible through `&Editor` and
//! unregistration possible from a `Subscription` drop.

use std::{cell::RefCell, rc::Rc};

use gpui::{App, Keystroke, Subscription, Window};

/// What an interceptor did with a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KeyDisposition {
    /// The key is handled. Nothing else sees it: no other interceptor, no
    /// key binding, no text input.
    Consumed,
    /// Not interested. The key goes to the next interceptor, then to the
    /// normal dispatch (bindings, then text input).
    Pass,
}

/// Order in which interceptors see a key: higher first; among equals the
/// one registered last goes first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InterceptPriority(pub i32);

impl InterceptPriority {
    /// An open menu or popover over the editor (insert menu, format menu,
    /// link popover). Sees keys before the modal layer, so `up` / `down` /
    /// `enter` / `tab` / `escape` drive the menu even in Vim normal mode.
    pub const OVERLAY: Self = Self(200);
    /// A modal editing layer (Vim).
    pub const MODAL: Self = Self(100);
    /// Anything else.
    pub const DEFAULT: Self = Self(0);
}

type Handler = dyn FnMut(&Keystroke, &mut Window, &mut App) -> KeyDisposition;

struct Entry {
    id: u64,
    priority: InterceptPriority,
    handler: Rc<RefCell<Box<Handler>>>,
}

#[derive(Default)]
struct Registry {
    next_id: u64,
    entries: Vec<Entry>,
}

/// The interceptors of one editor. Cheap to clone (shared).
#[derive(Clone, Default)]
pub(crate) struct Interceptors {
    registry: Rc<RefCell<Registry>>,
}

impl Interceptors {
    /// Adds an interceptor. Dropping the subscription removes it.
    pub(crate) fn register(
        &self,
        priority: InterceptPriority,
        handler: impl FnMut(&Keystroke, &mut Window, &mut App) -> KeyDisposition + 'static,
    ) -> Subscription {
        let id = {
            let mut registry = self.registry.borrow_mut();
            let id = registry.next_id;
            registry.next_id += 1;
            registry.entries.push(Entry {
                id,
                priority,
                handler: Rc::new(RefCell::new(Box::new(handler))),
            });
            // Highest priority first; among equals the newest first.
            registry
                .entries
                .sort_by(|a, b| b.priority.cmp(&a.priority).then(b.id.cmp(&a.id)));
            id
        };
        let registry = Rc::downgrade(&self.registry);
        Subscription::new(move || {
            if let Some(registry) = registry.upgrade() {
                registry.borrow_mut().entries.retain(|entry| entry.id != id);
            }
        })
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.registry.borrow().entries.is_empty()
    }

    /// Offers the keystroke to the interceptors in order. Handlers may
    /// register or drop interceptors while they run: one removed during the
    /// dispatch is skipped, one added during the dispatch waits for the next
    /// key.
    pub(crate) fn dispatch(
        &self,
        keystroke: &Keystroke,
        window: &mut Window,
        cx: &mut App,
    ) -> KeyDisposition {
        let snapshot: Vec<(u64, Rc<RefCell<Box<Handler>>>)> = self
            .registry
            .borrow()
            .entries
            .iter()
            .map(|entry| (entry.id, entry.handler.clone()))
            .collect();
        for (id, handler) in snapshot {
            let still_registered = self
                .registry
                .borrow()
                .entries
                .iter()
                .any(|entry| entry.id == id);
            if !still_registered {
                continue;
            }
            // A handler that is already running (re-entrant dispatch from
            // inside itself) is skipped rather than borrowed twice.
            let Ok(mut handler) = handler.try_borrow_mut() else {
                continue;
            };
            if handler(keystroke, window, cx) == KeyDisposition::Consumed {
                return KeyDisposition::Consumed;
            }
        }
        KeyDisposition::Pass
    }
}

/// GPUI turns "a modifier pressed and released alone" into a keystroke named
/// after the modifier. Those are not key presses a consumer cares about.
pub(crate) fn is_synthetic_keystroke(keystroke: &Keystroke) -> bool {
    keystroke.key.is_empty()
        || (keystroke.key_char.is_none()
            && matches!(
                keystroke.key.as_str(),
                "shift" | "control" | "alt" | "platform" | "function"
            ))
}
