//! The small mouse controls of the panel: icon buttons with a tooltip, text buttons that
//! show their key, and the fills that rows and buttons share.
//!
//! They are for the mouse and none of them is a tab stop. A control only draws itself; the
//! caller attaches the click handler and stops the click there, so that it does not also
//! count as a click on the row around the button.

use gpui::{
    AppContext, ElementId, Hsla, InteractiveElement, ParentElement, Pixels, Stateful,
    StatefulInteractiveElement, Styled, div, px,
};
use innerword_ui::theme::{RADIUS_SM, SPACE_1, ThemeColors};
use innerword_ui::{ContextMenu, Icon, IconName, KeyHint, Tooltip};

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
    let tooltip = Tooltip::new(label).key_hint(hint).build();
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
        .tooltip(move |window, cx| {
            // Not over the menu that the button has just opened.
            if ContextMenu::is_open(window, cx) {
                cx.new(|_| gpui::EmptyView).into()
            } else {
                tooltip(window, cx)
            }
        })
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
