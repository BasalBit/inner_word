//! [`Switch`] and [`Checkbox`]: on/off rows with a label and an optional
//! description.
//!
//! Both are stateless components: the parent owns the `bool` and flips it in
//! `on_change`.
//!
//! ```ignore
//! use writer_ui::{Switch, Checkbox};
//!
//! Switch::new("vim", self.vim_enabled)
//!     .label("Vim mode")
//!     .description("Modal editing with normal, insert and visual modes")
//!     .on_change(cx.listener(|this, on: &bool, _window, cx| {
//!         this.vim_enabled = *on;
//!         cx.notify();
//!     }))
//!
//! Checkbox::new("spell", self.spellcheck).label("Check spelling").on_change(..)
//! ```
//!
//! Keyboard: the row is a tab stop; `space` or `enter` toggles it
//! (`button::Press` in the `"Toggle"` key context). Clicking anywhere on the
//! row toggles too.

use std::rc::Rc;

use gpui::{App, ElementId, FocusHandle, KeyBinding, SharedString, Window, div, prelude::*, px};

use crate::{
    button::Press,
    icon::{Icon, IconName},
    theme::{ActiveTheme, RADIUS_MD, RADIUS_SM, UI_TEXT_BASE, UI_TEXT_SM, focus_ring},
};

pub const KEY_CONTEXT: &str = "Toggle";

pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("space", Press, Some(KEY_CONTEXT)),
        KeyBinding::new("enter", Press, Some(KEY_CONTEXT)),
    ]
}

type ChangeHandler = Rc<dyn Fn(&bool, &mut Window, &mut App)>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Switch,
    Checkbox,
}

struct ToggleRow {
    id: ElementId,
    kind: Kind,
    checked: bool,
    label: Option<SharedString>,
    description: Option<SharedString>,
    disabled: bool,
    focus_handle: Option<FocusHandle>,
    on_change: Option<ChangeHandler>,
}

impl ToggleRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement + use<> {
        let colors = cx.theme().colors;
        let checked = self.checked;
        let disabled = self.disabled;
        let focused = self
            .focus_handle
            .as_ref()
            .is_some_and(|handle| handle.is_focused(window));
        let toggle = {
            let on_change = self.on_change.clone();
            move |window: &mut Window, cx: &mut App| {
                if let Some(handler) = on_change.as_ref() {
                    handler(&!checked, window, cx);
                }
            }
        };
        let toggle = Rc::new(toggle);

        let control = match self.kind {
            Kind::Switch => {
                let track = if checked {
                    colors.accent
                } else {
                    colors.border_strong
                };
                div()
                    .flex_none()
                    .w(px(30.))
                    .h(px(18.))
                    .p(px(2.))
                    .rounded_full()
                    .bg(track)
                    .flex()
                    .when(checked, |el| el.justify_end())
                    .child(
                        div()
                            .size(px(14.))
                            .rounded_full()
                            .bg(if colors.appearance.is_dark() {
                                colors.ink
                            } else {
                                colors.page
                            })
                            .shadow(cx.theme().shadow_popover()),
                    )
                    .into_any_element()
            }
            Kind::Checkbox => div()
                .flex_none()
                .size(px(16.))
                .rounded(RADIUS_SM)
                .border_1()
                .border_color(if checked {
                    colors.accent
                } else {
                    colors.border_strong
                })
                .bg(if checked { colors.accent } else { colors.page })
                .flex()
                .items_center()
                .justify_center()
                .when(checked, |el| {
                    el.child(
                        Icon::new(IconName::Check)
                            .size(px(12.))
                            .color(colors.on_accent),
                    )
                })
                .into_any_element(),
        };

        let ring = focus_ring(&colors);
        let focus_bg = colors.hover;
        let press = toggle.clone();
        div()
            .id(self.id)
            .key_context(KEY_CONTEXT)
            .map(|el| match &self.focus_handle {
                Some(handle) => el.track_focus(handle),
                None => el.tab_index(0),
            })
            .flex()
            .items_center()
            .gap(px(10.))
            .py(px(6.))
            .px(px(8.))
            .rounded(RADIUS_MD)
            .when(focused, |el| el.bg(focus_bg).shadow(ring.clone()))
            .focus(move |style| style.bg(focus_bg).shadow(ring))
            .map(|el| {
                if disabled {
                    el.opacity(0.45)
                } else {
                    el.cursor_pointer()
                        .hover(move |style| style.bg(focus_bg))
                        // press and release on the row; `enter` / `space` go
                        // through `Press` (see `Button`)
                        .on_click(move |event, window, cx| {
                            if matches!(event, gpui::ClickEvent::Mouse(_)) {
                                toggle(window, cx)
                            }
                        })
                        .on_action(move |_: &Press, window, cx| press(window, cx))
                }
            })
            .map(|el| {
                let text = div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .when_some(self.label, |el, label| {
                        el.child(
                            div()
                                .text_size(UI_TEXT_BASE)
                                .line_height(px(18.))
                                .text_color(colors.ink)
                                .child(label),
                        )
                    })
                    .when_some(self.description, |el, description| {
                        el.child(
                            div()
                                .text_size(UI_TEXT_SM)
                                .line_height(px(16.))
                                .text_color(colors.muted)
                                .child(description),
                        )
                    });
                match self.kind {
                    // Checkbox first, then the text (macOS order).
                    Kind::Checkbox => el.child(control).child(text),
                    // Text first, switch at the trailing edge (settings rows).
                    Kind::Switch => el.child(text).child(control),
                }
            })
    }
}

macro_rules! toggle_component {
    ($name:ident, $kind:expr, $doc:literal) => {
        #[doc = $doc]
        #[derive(IntoElement)]
        pub struct $name(ToggleRow);

        impl $name {
            pub fn new(id: impl Into<ElementId>, checked: bool) -> Self {
                Self(ToggleRow {
                    id: id.into(),
                    kind: $kind,
                    checked,
                    label: None,
                    description: None,
                    disabled: false,
                    focus_handle: None,
                    on_change: None,
                })
            }

            pub fn label(mut self, label: impl Into<SharedString>) -> Self {
                self.0.label = Some(label.into());
                self
            }

            /// Secondary text under the label.
            pub fn description(mut self, description: impl Into<SharedString>) -> Self {
                self.0.description = Some(description.into());
                self
            }

            pub fn disabled(mut self, disabled: bool) -> Self {
                self.0.disabled = disabled;
                self
            }

            /// Use this focus handle instead of an automatic one (make it a tab stop).
            pub fn track_focus(mut self, handle: &FocusHandle) -> Self {
                self.0.focus_handle = Some(handle.clone());
                self
            }

            /// Called with the new value when toggled.
            pub fn on_change(
                mut self,
                handler: impl Fn(&bool, &mut Window, &mut App) + 'static,
            ) -> Self {
                self.0.on_change = Some(Rc::new(handler));
                self
            }
        }

        impl RenderOnce for $name {
            fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
                self.0.render(window, cx)
            }
        }
    };
}

toggle_component!(
    Switch,
    Kind::Switch,
    "An on/off switch row: label and description on the left, the switch at the trailing edge."
);
toggle_component!(
    Checkbox,
    Kind::Checkbox,
    "A checkbox row: the box, then the label and description."
);
