//! How a view sees keystrokes before anything else does.
//!
//! GPUI calls the callbacks of `App::intercept_keystrokes` for every key
//! press of every window before it looks for a binding
//! (`Window::dispatch_key_event`); a callback that stops the propagation
//! keeps the key from the bindings, from the menu bar (a key equivalent
//! that the window handled is not passed on to AppKit's menus) and from
//! the text input. One callback serves every [`KeymapView`]: each open
//! view registers itself for its window here and is asked about each key
//! of that window ([`KeymapView::intercept`]).
//!
//! All interceptors are called for a key, also after one of them stopped
//! the propagation, in the order they were registered. The open context
//! menu's is registered before this one (`writer_ui::init`); a view never
//! waits for a key while a menu is open (it closes the menu first).

use std::collections::HashMap;

use gpui::{App, Global, WeakEntity, Window, WindowId};

use crate::view::KeymapView;

#[derive(Default)]
struct Recorders {
    by_window: HashMap<WindowId, WeakEntity<KeymapView>>,
    installed: bool,
}

impl Global for Recorders {}

/// Installs the callback (once).
pub(crate) fn init(cx: &mut App) {
    if std::mem::replace(&mut cx.default_global::<Recorders>().installed, true) {
        return;
    }
    cx.intercept_keystrokes(|event, window, cx| {
        let id = window.window_handle().window_id();
        let Some(view) = cx
            .try_global::<Recorders>()
            .and_then(|recorders| recorders.by_window.get(&id).cloned())
        else {
            return;
        };
        let Some(view) = view.upgrade() else {
            cx.global_mut::<Recorders>().by_window.remove(&id);
            return;
        };
        let keystroke = event.keystroke.clone();
        if view.update(cx, |view, cx| view.intercept(&keystroke, window, cx)) {
            cx.stop_propagation();
        }
    })
    .detach();
}

/// Makes `view` the one that is asked about the keys of `window`.
pub(crate) fn register(view: WeakEntity<KeymapView>, window: &Window, cx: &mut App) {
    init(cx);
    cx.global_mut::<Recorders>()
        .by_window
        .insert(window.window_handle().window_id(), view);
}

/// Forgets the view of the window `id`, if it is `view`.
pub(crate) fn unregister(view: &WeakEntity<KeymapView>, id: WindowId, cx: &mut App) {
    if let Some(recorders) = cx.try_global::<Recorders>()
        && recorders.by_window.get(&id).is_some_and(|have| have == view)
    {
        cx.global_mut::<Recorders>().by_window.remove(&id);
    }
}
