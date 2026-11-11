//! The small mouse controls of the panel: icon buttons with a tooltip, text buttons that
//! show their key, and the fills that rows and buttons share.
//!
//! They are for the mouse and none of them is a tab stop. A control only draws itself; the
//! caller attaches the click handler and stops the click there, so that it does not also
//! count as a click on the row around the button.

use gpui::{
    AnyView, App, AppContext, Context, ElementId, Hsla, InteractiveElement, IntoElement,
    ParentElement, Pixels, Render, SharedString, Stateful, StatefulInteractiveElement, Styled,
    Window, div, px,
};
use writer_ui::theme::{
    ActiveTheme, RADIUS_MD, RADIUS_SM, SPACE_1, ThemeColors, UI_TEXT_SM, UI_TEXT_XS,
};
use writer_ui::{Icon, IconName, KeyHint};

/// Side of an icon button.
pub(super) const ICON_BUTTON: Pixels = px(20.);

/// The fill of a row or a control under the pointer. The theme's own hover colour is
/// almost the sidebar's (1.06:1 in Paper); this one can be seen.
pub(super) fn hover_fill(colors: &ThemeColors) -> Hsla {
    colors.ink.opacity(0.055)
}

/// The fill of the selected row while the keyboard is somewhere else, and of a control
/// while it is pressed.
pub(super) fn rest_fill(colors: &ThemeColors) -> Hsla {
    colors.ink.opacity(0.09)
}

/// A small floating label with the key that does the same thing.
struct Tip {
    label: SharedString,
    hint: Option<KeyHint>,
}

impl Render for Tip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let colors = theme.colors;
        // GPUI puts the tooltip at the pointer; the margin keeps it clear of the arrow.
        div().pt(px(6.)).child(
            div()
                .debug_selector(|| "library-tip".into())
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

/// A tooltip builder for `.tooltip(..)`: `label`, followed by its key.
pub(super) fn tip(
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

/// A square button that shows only an icon; `label` (and `hint`, the key that does the
/// same) is its tooltip.
pub(super) fn icon_button(
    id: impl Into<ElementId>,
    icon: IconName,
    label: &'static str,
    hint: Option<KeyHint>,
    colors: &ThemeColors,
) -> Stateful<gpui::Div> {
    let (hover, pressed) = (hover_fill(colors), rest_fill(colors));
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(ICON_BUTTON)
        .rounded(RADIUS_SM)
        .hover(move |style| style.bg(hover))
        .active(move |style| style.bg(pressed))
        .tooltip(tip(label, hint))
        .child(Icon::new(icon).size(px(14.)).color(colors.muted))
}

/// A button that shows a word and the key that does the same.
pub(super) fn text_button(
    id: impl Into<ElementId>,
    label: &'static str,
    hint: Option<KeyHint>,
    colors: &ThemeColors,
) -> Stateful<gpui::Div> {
    let (hover, pressed) = (hover_fill(colors), rest_fill(colors));
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_1)
        .h(px(22.))
        .px(SPACE_1)
        .rounded(RADIUS_SM)
        .hover(move |style| style.bg(hover))
        .active(move |style| style.bg(pressed))
        .child(label)
        .children(hint)
}
