//! Tab navigation between focusable widgets.
//!
//! GPUI has no built-in `tab` binding. Wrap a form, settings page or panel in
//! [`tab_navigation`] and `tab` / `shift-tab` move focus through the tab stops
//! inside the window ([`crate::TextField`], [`crate::Button`],
//! [`crate::Switch`], [`crate::SegmentedControl`] ... are tab stops), in tab
//! index order, then render order.
//!
//! ```ignore
//! innerword_ui::focus::tab_navigation(div().flex_col())
//!     .child(self.name_field.clone())
//!     .child(Switch::new("vim", self.vim))
//!     .child(Button::new("save", "Save").primary())
//! ```
//!
//! The bindings live in the `"TabNavigation"` key context, so a deeper
//! context that binds `tab` (the editor's indent) still wins inside it.

use gpui::{InteractiveElement, KeyBinding, actions};

pub const KEY_CONTEXT: &str = "TabNavigation";

actions!(
    focus,
    [
        /// Focus the next tab stop.
        FocusNext,
        /// Focus the previous tab stop.
        FocusPrev
    ]
);

pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("tab", FocusNext, Some(KEY_CONTEXT)),
        KeyBinding::new("shift-tab", FocusPrev, Some(KEY_CONTEXT)),
    ]
}

/// Adds the `"TabNavigation"` key context and the `tab` / `shift-tab`
/// handlers to `element`. It replaces any key context already set on the
/// element, so wrap an element that needs its own context in a parent div.
pub fn tab_navigation<E: InteractiveElement>(element: E) -> E {
    element
        .key_context(KEY_CONTEXT)
        .on_action(|_: &FocusNext, window, _| window.focus_next())
        .on_action(|_: &FocusPrev, window, _| window.focus_prev())
}
