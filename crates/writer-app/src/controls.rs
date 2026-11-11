//! Small pointer controls of the app's own views: the close control of a
//! modal.
//!
//! Everything here is for the mouse. A control never takes the keyboard
//! away from what has it (a press on it does not move focus), and its
//! tooltip names the key that does the same thing, which is where someone
//! working with the mouse learns it.

use gpui::{App, ClickEvent, MouseButton, Stateful, Window, div, prelude::*, px};
use writer_ui::{
    Icon, IconName, KeyHint, Tooltip,
    theme::{ActiveTheme, RADIUS_MD},
};

/// The debug selector of [`close_control`] (tests click it).
pub const CLOSE_CONTROL: &str = "modal-close";

/// The close control of a modal: a small cross at the right of its header.
/// `on_close` runs on a click. The tooltip says `Close` and shows `escape`.
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
        .tooltip(
            Tooltip::new("Close")
                .key_hint(KeyHint::parse("escape"))
                .build(),
        )
        // The keyboard stays where it is while the pointer is pressed.
        .on_mouse_down(MouseButton::Left, |_, window, cx| {
            window.prevent_default();
            cx.stop_propagation();
        })
        .on_click(on_close)
}
