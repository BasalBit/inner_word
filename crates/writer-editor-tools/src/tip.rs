//! Tooltips of the tools' buttons: what the button does and its key.
//!
//! The buttons are for the mouse; the tooltip is where a mouse user learns
//! the key that does the same thing.

use gpui::{AnyView, App, AppContext, Context, IntoElement, Render, SharedString, Window, div, px};
use gpui::{InteractiveElement, ParentElement, Styled};
use writer_ui::{
    KeyHint, picker, text_field,
    theme::{ActiveTheme, RADIUS_MD, UI_TEXT_SM, UI_TEXT_XS},
};

use crate::actions::CloseTool;

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

/// The key that applies what a field of the tools holds (the address in
/// the link field, the text in the replace field): whatever the keymap
/// binds a text field's submit to. The fields of the tools take that key
/// themselves, so no binding of this crate says it, and none is written
/// here either: the keymap can be rebuilt with other keys.
pub(crate) fn apply_hint(cx: &App) -> Option<KeyHint> {
    KeyHint::for_action(&text_field::Submit, cx)
}

/// The key that takes the marked row of a menu with a filter field (the
/// font picker's "Apply"): the key of a list's confirm. The filter field
/// leaves that key to the list.
pub(crate) fn take_hint(cx: &App) -> Option<KeyHint> {
    KeyHint::for_action(&picker::Confirm, cx)
}

/// The key that closes the open tool: the one of [`CloseTool`], which
/// works from the document and from a field of the tools alike (a field
/// has no binding of its own for a key that was given to `CloseTool`), or
/// else the key a text field cancels on.
pub(crate) fn close_hint(cx: &App) -> Option<KeyHint> {
    KeyHint::for_action(&CloseTool, cx).or_else(|| KeyHint::for_action(&text_field::Cancel, cx))
}

/// The second way of taking a row of a menu with a filter field (the font
/// picker's "Set as document font"): the key of the picker's second
/// confirm.
pub(crate) fn secondary_hint(cx: &App) -> Option<KeyHint> {
    KeyHint::for_action(&picker::SecondaryConfirm, cx)
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
