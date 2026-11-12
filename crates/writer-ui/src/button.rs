//! [`Button`]: a focusable push button with primary / subtle / danger
//! variants, an optional icon and an optional [`KeyHint`].
//!
//! ```ignore
//! use writer_ui::{Button, ButtonVariant, KeyHint};
//!
//! Button::new("export", "Export")
//!     .variant(ButtonVariant::Primary)
//!     .key_hint(KeyHint::for_action(&Export, cx))
//!     .on_click(cx.listener(|this, _event, window, cx| this.export(window, cx)))
//! ```
//!
//! Keyboard: the button is a tab stop; `enter` or `space` presses it while it
//! has focus (on key down, through the `button::Press` action in the
//! `"Button"` key context). To control focus yourself (a dialog that focuses
//! its default button), pass a handle with [`Button::track_focus`].
//!
//! The click handler receives a [`ClickEvent`]: `ClickEvent::Mouse` for
//! clicks, `ClickEvent::Keyboard` for `enter` / `space`.
//!
//! The mouse: the button lights up under the pointer, darkens while pressed
//! and shows a pointing hand. A click needs the press and the release on the
//! button. An icon-only button has no words, so give it a
//! [`Button::tooltip`]: resting the pointer on it shows the name and the
//! button's key hint (which an icon-only button does not draw inline).
//!
//! ```ignore
//! Button::icon_only("close", IconName::Close)
//!     .tooltip("Close")
//!     .key_hint(KeyHint::for_action(&Close, cx))
//! ```
//!
//! A button can open a menu instead of doing one thing ([`Button::menu`]):
//! the visible control for what is otherwise behind a right-click. The
//! [`crate::ContextMenu`] drops down below the button; from the keyboard
//! (`enter` / `space`) it opens with its first item marked.
//!
//! ```ignore
//! Button::icon_only("row-more", IconName::More)
//!     .tooltip("More actions")
//!     .menu(move |_window, _cx| vec![
//!         ContextMenuItem::action("Rename", Rename),
//!         ContextMenuItem::action("Delete", Delete).danger(),
//!     ])
//! ```

use std::{cell::Cell, rc::Rc};

use gpui::{
    App, Bounds, ClickEvent, ElementId, FocusHandle, KeyBinding, KeyboardClickEvent, Pixels,
    SharedString, Window, actions, canvas, div, prelude::*, px,
};

use crate::{
    context_menu::{ContextMenu, ContextMenuItem, MenuAnchor},
    icon::{Icon, IconName},
    key_hint::KeyHint,
    theme::{ActiveTheme, RADIUS_MD, UI_TEXT_BASE, UI_TEXT_SM, focus_ring},
    tooltip::Tooltip,
};

pub const KEY_CONTEXT: &str = "Button";

actions!(
    button,
    [
        /// Press the focused button, switch or segment.
        Press
    ]
);

pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("enter", Press, Some(KEY_CONTEXT)),
        KeyBinding::new("space", Press, Some(KEY_CONTEXT)),
    ]
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonVariant {
    /// Filled accent: the one main action of a dialog or panel.
    Primary,
    /// Quiet: outlined, no fill until hovered. Secondary actions.
    #[default]
    Subtle,
    /// Filled danger red: destructive confirmations.
    Danger,
    /// Text only, no outline: toolbar-like icon buttons and inline links.
    Ghost,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonSize {
    /// 28px tall.
    #[default]
    Default,
    /// 22px tall, 12px text.
    Compact,
}

type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;
type MenuBuilder = Rc<dyn Fn(&mut Window, &mut App) -> Vec<ContextMenuItem>>;

/// See the [module docs](self).
#[derive(IntoElement)]
pub struct Button {
    id: ElementId,
    label: Option<SharedString>,
    icon: Option<IconName>,
    variant: ButtonVariant,
    size: ButtonSize,
    key_hint: Option<KeyHint>,
    disabled: bool,
    selected: bool,
    focus_handle: Option<FocusHandle>,
    tab_index: isize,
    pointer_only: bool,
    on_click: Option<ClickHandler>,
    full_width: bool,
    tooltip: Option<SharedString>,
    menu: Option<MenuBuilder>,
}

impl Button {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: Some(label.into()),
            icon: None,
            variant: ButtonVariant::default(),
            size: ButtonSize::default(),
            key_hint: None,
            disabled: false,
            selected: false,
            focus_handle: None,
            tab_index: 0,
            pointer_only: false,
            on_click: None,
            full_width: false,
            tooltip: None,
            menu: None,
        }
    }

    /// A square icon-only button (ghost variant by default).
    pub fn icon_only(id: impl Into<ElementId>, icon: IconName) -> Self {
        let mut button = Self::new(id, "");
        button.label = None;
        button.icon = Some(icon);
        button.variant = ButtonVariant::Ghost;
        button
    }

    pub fn variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }

    pub fn primary(self) -> Self {
        self.variant(ButtonVariant::Primary)
    }

    pub fn danger(self) -> Self {
        self.variant(ButtonVariant::Danger)
    }

    pub fn size(mut self, size: ButtonSize) -> Self {
        self.size = size;
        self
    }

    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Shows a key hint after the label. Accepts `Option` so
    /// `KeyHint::for_action(..)` can be passed directly.
    pub fn key_hint(mut self, hint: impl Into<Option<KeyHint>>) -> Self {
        self.key_hint = hint.into();
        self
    }

    /// What the button does, shown with its key hint when the pointer rests
    /// on it. Every icon-only button should have one.
    pub fn tooltip(mut self, text: impl Into<SharedString>) -> Self {
        self.tooltip = Some(text.into());
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Draws the button as toggled on (accent tint), e.g. an active filter.
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn full_width(mut self) -> Self {
        self.full_width = true;
        self
    }

    /// Use this focus handle instead of an automatic one. Make it a tab stop
    /// (`cx.focus_handle().tab_stop(true)`) to keep tab navigation.
    pub fn track_focus(mut self, handle: &FocusHandle) -> Self {
        self.focus_handle = Some(handle.clone());
        self
    }

    /// Tab order among siblings (default 0 = document order).
    pub fn tab_index(mut self, index: isize) -> Self {
        self.tab_index = index;
        self
    }

    /// Makes the button a control for the pointer only: it is no tab stop
    /// and a click does not give it the keyboard, which stays where it is
    /// (in the query field of a list whose keys do the same as the
    /// button). Without effect together with [`Button::track_focus`].
    pub fn pointer_only(mut self) -> Self {
        self.pointer_only = true;
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }

    /// Makes this a menu button: a click (or `enter` / `space`) opens a
    /// [`ContextMenu`] with the items `build` returns, below the button.
    /// Replaces [`Button::on_click`].
    pub fn menu(
        mut self,
        build: impl Fn(&mut Window, &mut App) -> Vec<ContextMenuItem> + 'static,
    ) -> Self {
        self.menu = Some(Rc::new(build));
        self
    }
}

impl RenderOnce for Button {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.theme().colors;
        let focused = self
            .focus_handle
            .as_ref()
            .is_some_and(|handle| handle.is_focused(window));
        let (height, text_size, pad) = match self.size {
            ButtonSize::Default => (px(28.), UI_TEXT_BASE, px(12.)),
            ButtonSize::Compact => (px(22.), UI_TEXT_SM, px(8.)),
        };
        let icon_only = self.label.is_none();
        let (bg, fg, border, hover_bg) = match self.variant {
            ButtonVariant::Primary => (
                colors.accent,
                colors.on_accent,
                colors.accent,
                colors.accent_hover,
            ),
            ButtonVariant::Danger => (
                colors.danger,
                colors.on_accent,
                colors.danger,
                colors.danger.opacity(0.88),
            ),
            ButtonVariant::Subtle => (
                colors.elevated,
                colors.ink,
                colors.border_strong,
                colors.hover,
            ),
            ButtonVariant::Ghost => (
                gpui::transparent_black(),
                colors.ink,
                gpui::transparent_black(),
                colors.hover,
            ),
        };
        let (bg, border) = if self.selected {
            (colors.accent_subtle, colors.accent)
        } else {
            (bg, border)
        };
        let filled = matches!(self.variant, ButtonVariant::Primary | ButtonVariant::Danger);
        let hint_color = if filled {
            fg.opacity(0.8)
        } else {
            colors.muted
        };
        let icon_color = if filled {
            fg
        } else if icon_only {
            colors.muted
        } else {
            colors.ink
        };
        let disabled = self.disabled;
        // Where the button is, for the menu that drops down below it.
        let bounds = Rc::new(Cell::new(Bounds::<Pixels>::default()));
        let handler = match self.menu {
            Some(build) => {
                let bounds = bounds.clone();
                Some(Rc::new(
                    move |event: &ClickEvent, window: &mut Window, cx: &mut App| {
                        let items = build(window, cx);
                        let menu =
                            ContextMenu::open(MenuAnchor::Below(bounds.get()), items, window, cx);
                        if matches!(event, ClickEvent::Keyboard(_)) {
                            menu.update(cx, |menu, cx| menu.select_first(cx));
                        }
                    },
                ) as ClickHandler)
            }
            None => self.on_click,
        };
        let on_click = handler.clone();
        let on_press = handler;

        div()
            .id(self.id)
            .key_context(KEY_CONTEXT)
            .map(|el| match &self.focus_handle {
                Some(handle) => el.track_focus(handle),
                // Disabled buttons are skipped by tab navigation.
                None if disabled || self.pointer_only => el,
                None => el.tab_index(self.tab_index),
            })
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .gap(px(6.))
            .h(height)
            .when(icon_only, |el| el.w(height))
            .when(!icon_only, |el| el.px(pad))
            .when(self.full_width, |el| el.w_full())
            .rounded(RADIUS_MD)
            .bg(bg)
            .border_1()
            .border_color(border)
            .text_size(text_size)
            .line_height(px(16.))
            .text_color(fg)
            .whitespace_nowrap()
            .when(filled, |el| el.font_weight(gpui::FontWeight::MEDIUM))
            .when(focused && self.variant != ButtonVariant::Ghost, |el| {
                el.shadow(focus_ring(&colors))
            })
            .map(|el| {
                if self.variant == ButtonVariant::Ghost {
                    el.focus(move |style| style.border_color(colors.accent))
                } else {
                    let ring = focus_ring(&colors);
                    let border = if filled { border } else { colors.accent };
                    el.focus(move |style| style.border_color(border).shadow(ring))
                }
            })
            .map(|el| {
                if disabled {
                    el.opacity(0.45).cursor_default()
                } else {
                    el.cursor_pointer()
                        .hover(move |style| style.bg(hover_bg))
                        .active(move |style| style.bg(hover_bg).opacity(0.8))
                }
            })
            .when_some(self.tooltip, |el, text| {
                el.tooltip(Tooltip::new(text).key_hint(self.key_hint.clone()).build())
            })
            .when(!disabled, |el| {
                // A click is a press and a release on the button: letting go
                // of a drag over it does nothing. GPUI also reports `enter`
                // and `space` released on a focused element as a click; the
                // keyboard presses on key down through `Press` instead.
                el.on_click(move |event, window, cx| {
                    if let (ClickEvent::Mouse(_), Some(handler)) = (event, on_click.as_ref()) {
                        handler(event, window, cx);
                    }
                })
                .on_action(move |_: &Press, window, cx| {
                    if let Some(handler) = on_press.as_ref() {
                        handler(
                            &ClickEvent::Keyboard(KeyboardClickEvent::default()),
                            window,
                            cx,
                        );
                    }
                })
            })
            .child(
                canvas(move |at, _, _| bounds.set(at), |_, _, _, _| {})
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
            )
            .when_some(self.icon, |el, icon| {
                el.child(Icon::new(icon).color(icon_color))
            })
            .when_some(self.label, |el, label| el.child(label))
            // an icon-only button is a square: its key is in the tooltip
            .when_some(self.key_hint.filter(|_| !icon_only), |el, hint| {
                el.child(hint.color(hint_color).size(UI_TEXT_SM))
            })
    }
}
