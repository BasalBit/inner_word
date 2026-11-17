//! The small controls of the chat: icon buttons with a tooltip (what the
//! button does and its key) and text buttons.
//!
//! They are made for the pointer. None of them takes the keyboard when it
//! is pressed: the keyboard stays where it was (in the composer, in the
//! text), so a click on "Copy" does not move the caret anywhere. From the
//! keyboard the same things are done with the keys the tooltips name, or
//! by moving to the control (the header and the context row are places the
//! keyboard can be, see [`crate::AssistantPanel`]), where `focused` draws
//! the mark of the keyboard ([`keyboard_mark`]).
//!
//! A control only draws itself. The caller attaches the click handler.

use gpui::{
    Div, ElementId, FontWeight, Hsla, MouseButton, Pixels, SharedString, Stateful, div, prelude::*,
    px,
};
use innerword_ui::{
    Icon, IconName, KeyHint, Tooltip,
    theme::{RADIUS_SM, ThemeColors, UI_TEXT_SM},
};

/// Side of an icon button.
pub const ICON_BUTTON: Pixels = px(22.);

/// The mark of the keyboard on a small control: an accent outline on a
/// tint of the accent. (The kit's halo is made for fields and full-size
/// buttons; around a 22px control it reads as a filled square.) The
/// control must have a border of its own, so that nothing moves.
pub fn keyboard_mark<E: Styled>(control: E, focused: bool, colors: &ThemeColors) -> E {
    if focused {
        control
            .border_color(colors.accent)
            .bg(colors.accent_subtle)
            .text_color(colors.ink)
    } else {
        control
    }
}

/// Keeps a press from moving the keyboard to whatever is under it.
pub(crate) fn keep_focus(element: Stateful<Div>) -> Stateful<Div> {
    element.on_mouse_down(MouseButton::Left, |_, window, _| {
        window.prevent_default();
    })
}

/// A square button with one icon. `label` and `hint` are its tooltip.
/// `name` is its debug selector (tests find it by that).
#[allow(clippy::too_many_arguments)]
pub fn icon_button(
    id: impl Into<ElementId>,
    name: impl Into<String>,
    icon: IconName,
    label: impl Into<SharedString>,
    hint: Option<KeyHint>,
    disabled: bool,
    focused: bool,
    colors: &ThemeColors,
) -> Stateful<Div> {
    let name = name.into();
    let hover = colors.hover;
    let tint = if disabled { colors.faint } else { colors.muted };
    let ink = colors.ink;
    let button = div()
        .id(id)
        .debug_selector(move || name)
        .group("icon-button")
        .relative()
        .flex_none()
        .size(ICON_BUTTON)
        .flex()
        .items_center()
        .justify_center()
        .rounded(RADIUS_SM)
        .border_1()
        .border_color(gpui::transparent_black())
        .map(|button| keyboard_mark(button, focused, colors))
        .map(|button| {
            if disabled {
                button.opacity(0.55).cursor_default()
            } else {
                button
                    .cursor_pointer()
                    .hover(move |style| style.bg(hover).text_color(ink))
            }
        })
        .child(Icon::new(icon).size(px(14.)).color(tint))
        .tooltip(Tooltip::new(label).key_hint(hint).build());
    keep_focus(button)
}

/// The look of a [`text_button`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    /// Filled with the accent: the one thing to do here.
    Primary,
    /// Outlined.
    Quiet,
    /// Only the words, in the muted colour, until hovered: the actions
    /// under an answer.
    Ghost,
}

/// A small button with a label, an optional icon before it and, in its
/// tooltip, the key that does the same. A disabled button is drawn faded
/// and the caller attaches no handler to it.
#[allow(clippy::too_many_arguments)]
pub fn text_button(
    id: impl Into<ElementId>,
    name: impl Into<String>,
    label: impl Into<SharedString>,
    icon: Option<IconName>,
    tooltip: Option<(SharedString, Option<KeyHint>)>,
    look: Look,
    disabled: bool,
    focused: bool,
    colors: &ThemeColors,
) -> Stateful<Div> {
    let name = name.into();
    let (bg, fg, border, hover_bg, hover_fg): (Hsla, Hsla, Hsla, Hsla, Hsla) = match look {
        Look::Primary => (
            colors.accent,
            colors.on_accent,
            colors.accent,
            colors.accent_hover,
            colors.on_accent,
        ),
        Look::Quiet => (
            colors.elevated,
            colors.ink,
            colors.border_strong,
            colors.hover,
            colors.ink,
        ),
        Look::Ghost => (
            gpui::transparent_black(),
            colors.muted,
            gpui::transparent_black(),
            colors.hover,
            colors.ink,
        ),
    };
    let button = div()
        .id(id)
        .debug_selector(move || name)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(4.))
        .h(px(22.))
        .px(px(if look == Look::Ghost { 6. } else { 8. }))
        .rounded(RADIUS_SM)
        .whitespace_nowrap()
        .text_size(UI_TEXT_SM)
        .line_height(px(16.))
        .text_color(fg)
        .bg(bg)
        .border_1()
        .border_color(border)
        .when(look == Look::Primary, |button| {
            button.font_weight(FontWeight::MEDIUM)
        })
        .map(|button| keyboard_mark(button, focused, colors))
        .map(|button| {
            if disabled {
                button.opacity(0.45).cursor_default()
            } else {
                button
                    .cursor_pointer()
                    .hover(move |style| style.bg(hover_bg).text_color(hover_fg))
            }
        })
        .children(icon.map(|icon| Icon::new(icon).size(px(13.)).color(fg)))
        .child(label.into());
    let button = match tooltip {
        Some((text, hint)) => button.tooltip(Tooltip::new(text).key_hint(hint).build()),
        None => button,
    };
    keep_focus(button)
}
