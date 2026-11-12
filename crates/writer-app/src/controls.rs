//! Small parts of the app's own modals: the close control, and the hints
//! of the footer.
//!
//! The close control is for the mouse. It never takes the keyboard away
//! from what has it (a press on it does not move focus), and its tooltip
//! names the key that does the same thing, which is where someone working
//! with the mouse learns it.
//!
//! No key is written here: the tooltip and the hints ask the keymap each
//! time they are drawn ([`crate::keymap::hint_in`]), so they show the keys
//! the user chose.

use gpui::{
    Action, App, ClickEvent, Div, MouseButton, SharedString, Stateful, Window, div, prelude::*, px,
};
use writer_ui::{
    Icon, IconName, Tooltip,
    theme::{ActiveTheme, RADIUS_MD},
};

/// The key that closes a modal, as the keymap has it.
pub fn close_key(cx: &App) -> Option<writer_ui::KeyHint> {
    crate::keymap::hint_in(
        &writer_ui::modal::Dismiss,
        writer_ui::modal::KEY_CONTEXT,
        cx,
    )
}

/// One hint of a modal's footer: the keys of `actions` where the key
/// context `place` holds, then what they do ("↑ ↓ Choose"). `None` when
/// none of the actions has a key there.
pub fn footer_hint(
    actions: &[&dyn Action],
    place: &str,
    what: impl Into<SharedString>,
    cx: &App,
) -> Option<Div> {
    let keys = crate::keymap::keys_text_in(actions, place, cx)?;
    let colors = cx.theme().colors;
    Some(
        div()
            .flex()
            .items_center()
            .gap(px(6.))
            .child(div().text_color(colors.ink).child(keys))
            .child(what.into()),
    )
}

/// The footer's "Esc Close".
pub fn close_hint(cx: &App) -> Option<Div> {
    footer_hint(
        &[&writer_ui::modal::Dismiss],
        writer_ui::modal::KEY_CONTEXT,
        "Close",
        cx,
    )
}

/// The debug selector of [`close_control`] (tests click it).
pub const CLOSE_CONTROL: &str = "modal-close";

/// The close control of a modal: a small cross at the right of its header.
/// `on_close` runs on a click. The tooltip says `Close` and shows the key
/// that closes a modal (`escape`).
pub fn close_control(
    on_close: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> Stateful<gpui::Div> {
    let colors = cx.theme().colors;
    div()
        .id("modal-close")
        .debug_selector(|| CLOSE_CONTROL.into())
        .flex_none()
        .size(px(24.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(RADIUS_MD)
        .cursor_pointer()
        .hover(|style| style.bg(colors.hover))
        .child(Icon::new(IconName::Close).color(colors.muted))
        .tooltip(Tooltip::new("Close").key_hint(close_key(cx)).build())
        // The keyboard stays where it is while the pointer is pressed.
        .on_mouse_down(MouseButton::Left, |_, window, cx| {
            window.prevent_default();
            cx.stop_propagation();
        })
        .on_click(on_close)
}
