//! [`ModalLayer`]: hosts at most one modal view above the window content.
//!
//! The layer draws a translucent backdrop, places the modal (near the top
//! like the command palette, or centred like a dialog), moves focus into it,
//! and when the modal is dismissed restores focus to whatever was focused
//! when it opened.
//!
//! A modal is any [`ManagedView`]: a view that is `Focusable`, `Render` and
//! emits [`DismissEvent`] when it wants to close. [`crate::Picker`] and
//! [`crate::ConfirmDialog`] are managed views.
//!
//! ```ignore
//! use writer_ui::{ModalLayer, ModalPlacement};
//!
//! // the window root owns one layer and renders it last, over the content
//! struct Root { modal_layer: Entity<ModalLayer>, /* .. */ }
//! let modal_layer = cx.new(|_| ModalLayer::new());
//! div().relative().size_full().child(content).child(self.modal_layer.clone())
//!
//! // open (replaces any open modal, keeping the original focus to restore)
//! self.modal_layer.update(cx, |layer, cx| {
//!     let picker = cx.new(|cx| Picker::modal(MyDelegate::new(), window, cx));
//!     layer.show(picker, window, cx);
//! });
//! // or toggle: closes it when a modal of the same type is open
//! layer.toggle(window, cx, |window, cx| Picker::modal(MyDelegate::new(), window, cx));
//!
//! // from inside the modal view: close it
//! cx.emit(DismissEvent);
//! // from outside
//! layer.dismiss(window, cx);
//! ```
//!
//! `escape` inside the layer dismisses the modal unless the modal handles
//! `escape` itself (binding `modal::Dismiss` in the `"ModalLayer"` context). A
//! click on the backdrop dismisses too.

use std::any::TypeId;

use gpui::{
    AnyView, Context, DismissEvent, Entity, Focusable, KeyBinding, ManagedView, MouseButton,
    Subscription, WeakFocusHandle, Window, actions, div, prelude::*, px, relative,
};

use crate::theme::ActiveTheme;

pub const KEY_CONTEXT: &str = "ModalLayer";

actions!(
    modal,
    [
        /// Close the open modal.
        Dismiss
    ]
);

pub fn key_bindings() -> Vec<KeyBinding> {
    vec![KeyBinding::new("escape", Dismiss, Some(KEY_CONTEXT))]
}

/// Where the modal sits in the window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ModalPlacement {
    /// Horizontally centred, its top at 12% of the window height (command
    /// palette, quick open, pickers).
    #[default]
    Top,
    /// Centred in the window (confirmation dialogs).
    Center,
}

struct ActiveModal {
    view: AnyView,
    type_id: TypeId,
    placement: ModalPlacement,
    previous_focus: Option<WeakFocusHandle>,
    _subscription: Subscription,
}

/// See the [module docs](self).
#[derive(Default)]
pub struct ModalLayer {
    active: Option<ActiveModal>,
}

impl ModalLayer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Shows `view` near the top of the window and focuses it.
    pub fn show<V: ManagedView>(
        &mut self,
        view: Entity<V>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_with_placement(view, ModalPlacement::Top, window, cx);
    }

    /// Shows `view` at `placement` and focuses it. An already open modal is
    /// closed first; focus will still return to where it was before the
    /// first one opened.
    pub fn show_with_placement<V: ManagedView>(
        &mut self,
        view: Entity<V>,
        placement: ModalPlacement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let previous_focus = match self.active.take() {
            Some(active) => active.previous_focus,
            None => window.focused(cx).map(|handle| handle.downgrade()),
        };
        let subscription =
            cx.subscribe_in(&view, window, |this, _, _: &DismissEvent, window, cx| {
                this.dismiss(window, cx);
            });
        let focus = view.focus_handle(cx);
        self.active = Some(ActiveModal {
            view: view.into(),
            type_id: TypeId::of::<V>(),
            placement,
            previous_focus,
            _subscription: subscription,
        });
        window.focus(&focus);
        cx.notify();
    }

    /// Closes the open modal of type `V`, or builds and shows a new one.
    pub fn toggle<V: ManagedView>(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        build: impl FnOnce(&mut Window, &mut Context<V>) -> V,
    ) {
        if self.is_open::<V>() {
            self.dismiss(window, cx);
        } else {
            let view = cx.new(|cx| build(window, cx));
            self.show(view, window, cx);
        }
    }

    /// Closes the modal (if any) and restores the focus it took.
    pub fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(active) = self.active.take() else {
            return;
        };
        if let Some(previous) = active.previous_focus.and_then(|weak| weak.upgrade()) {
            window.focus(&previous);
        }
        cx.notify();
    }

    /// Whether any modal is open.
    pub fn has_modal(&self) -> bool {
        self.active.is_some()
    }

    /// Whether a modal of type `V` is open.
    pub fn is_open<V: 'static>(&self) -> bool {
        self.active
            .as_ref()
            .is_some_and(|active| active.type_id == TypeId::of::<V>())
    }

    /// The open modal, if it is a `V`.
    pub fn active<V: 'static>(&self) -> Option<Entity<V>> {
        self.active
            .as_ref()
            .and_then(|active| active.view.clone().downcast::<V>().ok())
    }

    fn on_dismiss(&mut self, _: &Dismiss, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss(window, cx);
    }
}

impl Render for ModalLayer {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(active) = self.active.as_ref() else {
            return div().into_any_element();
        };
        let backdrop = cx.theme().colors.overlay_backdrop;
        div()
            .id("modal-layer")
            .key_context(KEY_CONTEXT)
            .on_action(cx.listener(Self::on_dismiss))
            .absolute()
            .inset_0()
            .occlude()
            .bg(backdrop)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.dismiss(window, cx)),
            )
            .flex()
            .flex_col()
            .items_center()
            .map(|el| match active.placement {
                ModalPlacement::Top => el.child(div().flex_none().h(relative(0.12))),
                ModalPlacement::Center => el.justify_center(),
            })
            .px(px(16.))
            .child(div().occlude().max_w_full().child(active.view.clone()))
            .into_any_element()
    }
}
