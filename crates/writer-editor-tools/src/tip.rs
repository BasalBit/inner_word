//! Tooltips of the tools' buttons: what the button does and its key.
//!
//! The buttons are for the mouse; the tooltip is where a mouse user learns
//! the key that does the same thing.

use gpui::{AnyView, App, AppContext, Context, IntoElement, Render, SharedString, Window, div, px};
use gpui::{InteractiveElement, ParentElement, Styled};
use writer_ui::{
    KeyHint,
    theme::{ActiveTheme, RADIUS_MD, UI_TEXT_SM, UI_TEXT_XS},
};

/// A small floating label with an optional key hint.
pub(crate) struct Tip {
    label: SharedString,
    hint: Option<KeyHint>,
}

impl Render for Tip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let colors = theme.colors;
        // GPUI puts the tooltip at the pointer; the margin keeps it clear of
        // the arrow.
        div().pt(px(6.)).child(
            div()
                .debug_selector(|| "tool-tip".into())
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
                .child(self.label.clone())
                .children(self.hint.clone().map(|hint| hint.size(UI_TEXT_XS))),
        )
    }
}

/// A tooltip builder for `.tooltip(..)`: `label`, followed by `hint`.
pub(crate) fn tip(
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
