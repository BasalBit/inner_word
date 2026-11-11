//! Small pointer controls of the app's own views: tooltips and the close
//! control of a modal.
//!
//! Everything here is for the mouse. A control never takes the keyboard
//! away from what has it (a press on it does not move focus), and its
//! tooltip names the key that does the same thing, which is where someone
//! working with the mouse learns it.

use gpui::{
    AnyView, App, AppContext, Context, IntoElement, MouseButton, Render, SharedString, Stateful,
    Window, div, prelude::*, px,
};
use writer_ui::{
    Icon, IconName, KeyHint,
    theme::{ActiveTheme, RADIUS_MD, UI_TEXT_SM, UI_TEXT_XS},
};

/// A small floating label with an optional key hint.
pub struct Tip {
    label: SharedString,
    hint: Option<KeyHint>,
}

impl Render for Tip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let colors = theme.colors;
        // GPUI puts the tooltip at the pointer; the margin keeps it clear
        // of the arrow.
        div().pt(px(6.)).child(
            div()
                .debug_selector(|| "app-tip".into())
                .flex()
                .items_center()
                .gap(px(8.))
                .h(px(24.))
                .px(px(8.))
                .rounded(RADIUS_MD)
                .bg(colors.elevated)
                .border_1()
                .border_color(colors.border_strong)
                .shadow(theme.shadow_popover())
                .font_family(theme.fonts.ui.clone())
                .text_size(UI_TEXT_SM)
                .line_height(px(16.))
                .text_color(colors.ink)
                .whitespace_nowrap()
                .child(self.label.clone())
                .children(self.hint.clone().map(|hint| hint.size(UI_TEXT_XS))),
        )
    }
}

/// A tooltip builder for `.tooltip(..)`: `label`, followed by `hint`.
pub fn tip(
    label: impl Into<SharedString>,
    hint: Option<KeyHint>,
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let label = label.into();
    move |_, cx| {
        let label = label.clone();
        let hint = hint.clone();
        cx.new(|_| Tip { label, hint }).into()
    }
}

/// The debug selector of [`close_control`] (tests click it).
pub const CLOSE_CONTROL: &str = "modal-close";

/// The close control of a modal: a small cross at the right of its header.
/// `on_close` runs on a click. The tooltip says `Close` and shows `escape`.
pub fn close_control(
    on_close: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
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
        .tooltip(tip("Close", KeyHint::parse("escape")))
        // The keyboard stays where it is while the pointer is pressed.
        .on_mouse_down(MouseButton::Left, |_, window, cx| {
            window.prevent_default();
            cx.stop_propagation();
        })
        .on_click(on_close)
}
