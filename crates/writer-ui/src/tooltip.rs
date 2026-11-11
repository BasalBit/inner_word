//! [`Tooltip`]: what a control is called and the key that does the same,
//! shown when the pointer rests on it. A mouse user finds out what an icon
//! does; a keyboard user learns the shortcut.
//!
//! ```ignore
//! use writer_ui::{Button, IconName, Tooltip};
//!
//! // kit buttons: the tooltip shows the text and the button's key hint
//! Button::icon_only("close", IconName::Close)
//!     .tooltip("Close")
//!     .key_hint(KeyHint::for_action(&Close, cx))
//!
//! // any element with an id
//! div().id("page-view").tooltip(Tooltip::text("Page view"))
//! div().id("bold").tooltip(Tooltip::for_action("Bold", &ToggleBold))   // key from the keymap
//! div().id("x").tooltip(Tooltip::new("Resolve").key_hint(hint).build())
//! ```
//!
//! GPUI shows a tooltip after the pointer has rested for half a second and
//! hides it when the pointer leaves or a button is pressed.

use gpui::{Action, AnyView, App, Context, SharedString, Window, div, prelude::*, px};

use crate::{
    key_hint::KeyHint,
    theme::{ActiveTheme, RADIUS_MD, UI_TEXT_SM},
};

/// See the [module docs](self).
#[derive(Clone, Debug, PartialEq)]
pub struct Tooltip {
    title: SharedString,
    key_hint: Option<KeyHint>,
}

impl Tooltip {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            key_hint: None,
        }
    }

    /// The key shown after the text. Accepts an `Option` so
    /// `KeyHint::for_action(..)` can be passed directly.
    pub fn key_hint(mut self, hint: impl Into<Option<KeyHint>>) -> Self {
        self.key_hint = hint.into();
        self
    }

    pub fn title(&self) -> &SharedString {
        &self.title
    }

    pub fn hint(&self) -> Option<&KeyHint> {
        self.key_hint.as_ref()
    }

    /// The argument of `.tooltip(..)` on an element with an id.
    pub fn build(self) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
        move |_, cx| cx.new(|_| self.clone()).into()
    }

    /// A tooltip with text only.
    pub fn text(title: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView {
        Self::new(title).build()
    }

    /// A tooltip with the key that is bound to `action` when it is shown.
    pub fn for_action<T: Into<SharedString>>(
        title: T,
        action: &dyn Action,
    ) -> impl Fn(&mut Window, &mut App) -> AnyView + use<T> {
        let title = title.into();
        let action = action.boxed_clone();
        move |window, cx| {
            let hint = KeyHint::for_action_focused(action.as_ref(), window)
                .or_else(|| KeyHint::for_action(action.as_ref(), cx));
            let tooltip = Tooltip::new(title.clone()).key_hint(hint);
            cx.new(|_| tooltip).into()
        }
    }
}

impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let colors = theme.colors;
        // GPUI puts the tooltip's corner at the pointer; keep it clear of
        // the cursor's arrow.
        div().pt(px(18.)).pl(px(6.)).child(
            div()
                .debug_selector(|| "tooltip".into())
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(8.))
                .py(px(4.))
                .rounded(RADIUS_MD)
                .bg(colors.elevated)
                .border_1()
                .border_color(if colors.appearance.is_dark() {
                    colors.border_strong
                } else {
                    colors.hairline
                })
                .shadow(theme.shadow_popover())
                .font_family(theme.fonts.ui.clone())
                .text_size(UI_TEXT_SM)
                .line_height(px(16.))
                .text_color(colors.ink)
                .whitespace_nowrap()
                .child(self.title.clone())
                .children(self.key_hint.clone()),
        )
    }
}
