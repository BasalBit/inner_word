//! The small mouse controls of the panel: icon buttons with a tooltip, and
//! text buttons that show their key.
//!
//! They are for the mouse. None of them is a tab stop and none takes the
//! keyboard when it is pressed: inside the thread list a press lands the
//! keyboard on the list (as a click on the card does); outside it (the
//! header, the composer under it) the keyboard stays where it was, so a
//! button that turns out to do nothing (no text to comment on) leaves the
//! caret in the text.
//!
//! A control only draws itself. The caller attaches the click handler and
//! stops the click there, so that it does not also count as a click on the
//! card around the button.

use gpui::{
    AnyView, App, Context, Div, ElementId, FontWeight, MouseButton, Render, SharedString, Stateful,
    Window, div, prelude::*, px,
};
use writer_ui::{
    Icon, IconName, KeyHint,
    theme::{ActiveTheme, RADIUS_MD, RADIUS_SM, ThemeColors, UI_TEXT_SM, UI_TEXT_XS},
};

/// Side of an icon button.
pub(crate) const ICON_BUTTON: gpui::Pixels = px(20.);

/// A small floating label with the key that does the same thing.
struct Tip {
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
                .debug_selector(|| "comment-tip".into())
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
fn tip(
    label: SharedString,
    hint: Option<KeyHint>,
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    move |_, cx| {
        let label = label.clone();
        let hint = hint.clone();
        cx.new(|_| Tip { label, hint }).into()
    }
}

/// How a control behaves when the mouse button goes down on it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Press {
    /// The keyboard goes where a click on the surroundings puts it (the
    /// thread list, for a control in a card).
    Bubble,
    /// The keyboard stays where it is.
    KeepFocus,
}

fn pressable(element: Stateful<Div>, press: Press) -> Stateful<Div> {
    match press {
        Press::Bubble => element,
        Press::KeepFocus => element.on_mouse_down(MouseButton::Left, |_, window, _| {
            window.prevent_default();
        }),
    }
}

/// A square button with one icon. `label` and `hint` are its tooltip: what
/// it does and the key that does the same. `name` is its debug selector.
pub(crate) fn icon_button(
    id: impl Into<ElementId>,
    name: String,
    icon: IconName,
    label: impl Into<SharedString>,
    hint: Option<KeyHint>,
    press: Press,
    colors: &ThemeColors,
) -> Stateful<Div> {
    let hover = colors.hover;
    let button = div()
        .id(id)
        .debug_selector(move || name)
        .flex_none()
        .size(ICON_BUTTON)
        .flex()
        .items_center()
        .justify_center()
        .rounded(RADIUS_SM)
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .child(Icon::new(icon).size(px(14.)).color(colors.muted))
        .tooltip(tip(label.into(), hint));
    pressable(button, press)
}

/// The look of a [`text_button`].
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Look {
    /// Filled with the accent: the one thing the composer is there for.
    Primary,
    /// Outlined.
    Quiet,
    /// Filled with the danger colour: a delete that was asked about.
    Danger,
    /// Only the words, in the muted colour of a hint, until hovered.
    Hint,
}

/// A button with a label and, after it, the key that does the same. A
/// disabled button is drawn faded and ignores the pointer (the caller
/// attaches no handler to it).
#[allow(clippy::too_many_arguments)]
pub(crate) fn text_button(
    id: impl Into<ElementId>,
    name: String,
    label: impl Into<SharedString>,
    hint: Option<KeyHint>,
    look: Look,
    disabled: bool,
    press: Press,
    colors: &ThemeColors,
) -> Stateful<Div> {
    let (bg, fg, border, hover_bg, hint_color) = match look {
        Look::Primary => (
            colors.accent,
            colors.on_accent,
            colors.accent,
            colors.accent_hover,
            colors.on_accent.opacity(0.8),
        ),
        Look::Danger => (
            colors.danger,
            colors.on_accent,
            colors.danger,
            colors.danger.opacity(0.88),
            colors.on_accent.opacity(0.8),
        ),
        Look::Quiet => (
            colors.elevated,
            colors.ink,
            colors.border_strong,
            colors.hover,
            colors.muted,
        ),
        Look::Hint => (
            gpui::transparent_black(),
            colors.muted,
            gpui::transparent_black(),
            colors.hover,
            colors.muted,
        ),
    };
    let is_hint = look == Look::Hint;
    let ink = colors.ink;
    let button = div()
        .id(id)
        .debug_selector(move || name)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(4.))
        .rounded(RADIUS_SM)
        .whitespace_nowrap()
        .text_color(fg)
        .map(|button| {
            if is_hint {
                // As tall as the hint text, with room to point at: the
                // margin takes the padding back so a row of them lines up
                // with the text above.
                button
                    .text_size(UI_TEXT_XS)
                    .px(px(3.))
                    .mx(px(-3.))
                    .py(px(1.))
            } else {
                button
                    .h(px(22.))
                    .px(px(8.))
                    .text_size(UI_TEXT_SM)
                    .line_height(px(16.))
                    .bg(bg)
                    .border_1()
                    .border_color(border)
                    .when(matches!(look, Look::Primary | Look::Danger), |button| {
                        button.font_weight(FontWeight::MEDIUM)
                    })
            }
        })
        .map(|button| {
            if disabled {
                button.opacity(0.45).cursor_default()
            } else {
                button.cursor_pointer().hover(move |style| {
                    let style = style.bg(hover_bg);
                    if is_hint {
                        style.text_color(ink)
                    } else {
                        style
                    }
                })
            }
        })
        .child(label.into())
        .children(hint.map(|hint| {
            hint.color(hint_color)
                .size(if is_hint { UI_TEXT_XS } else { UI_TEXT_SM })
        }));
    pressable(button, press)
}
