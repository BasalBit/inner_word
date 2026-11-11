//! [`ConfirmDialog`]: a title, a message, and confirm / cancel buttons.
//! Shown in a [`crate::ModalLayer`].
//!
//! ```ignore
//! use writer_ui::{ConfirmDialog, ModalLayer};
//!
//! let dialog = cx.new(|cx| {
//!     ConfirmDialog::new("Delete “Chapter 3”?", "It moves to the trash.", cx)
//!         .confirm_label("Delete")
//!         .danger()
//!         .on_confirm(move |window, cx| { /* delete */ })
//! });
//! ConfirmDialog::show(dialog, &modal_layer, window, cx);   // centred
//! // or subscribe: cx.subscribe(&dialog, |this, _, event: &ConfirmDialogEvent, cx| ..)
//! ```
//!
//! Keyboard: focus starts on the confirm button; `enter` (or `space`) presses
//! the focused button, `tab` / `shift-tab` / `left` / `right` move between
//! the two buttons, `escape` cancels. The dialog dismisses itself after
//! either choice.

use gpui::{
    App, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, KeyBinding,
    SharedString, Window, actions, div, prelude::*, px,
};

use crate::{
    button::Button,
    modal::{ModalLayer, ModalPlacement},
    theme::{ActiveTheme, RADIUS_LG, UI_TEXT_BASE, UI_TEXT_LG},
};

pub const KEY_CONTEXT: &str = "ConfirmDialog";

actions!(
    confirm_dialog,
    [
        /// Confirm the dialog.
        Confirm,
        /// Cancel the dialog.
        Cancel,
        /// Move focus to the other button.
        SwitchButton
    ]
);

pub fn key_bindings() -> Vec<KeyBinding> {
    let c = Some(KEY_CONTEXT);
    vec![
        KeyBinding::new("escape", Cancel, c),
        KeyBinding::new("tab", SwitchButton, c),
        KeyBinding::new("shift-tab", SwitchButton, c),
        KeyBinding::new("left", SwitchButton, c),
        KeyBinding::new("right", SwitchButton, c),
        // Only reached when neither button has focus (buttons bind enter in
        // their own deeper "Button" context).
        KeyBinding::new("enter", Confirm, c),
    ]
}

/// The outcome, emitted just before [`DismissEvent`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfirmDialogEvent {
    Confirmed,
    Cancelled,
}

type Callback = Box<dyn FnOnce(&mut Window, &mut App)>;

/// See the [module docs](self).
pub struct ConfirmDialog {
    title: SharedString,
    message: SharedString,
    confirm_label: SharedString,
    cancel_label: SharedString,
    danger: bool,
    root_focus: FocusHandle,
    confirm_focus: FocusHandle,
    cancel_focus: FocusHandle,
    on_confirm: Option<Callback>,
    on_cancel: Option<Callback>,
    done: bool,
}

impl EventEmitter<DismissEvent> for ConfirmDialog {}
impl EventEmitter<ConfirmDialogEvent> for ConfirmDialog {}

impl Focusable for ConfirmDialog {
    /// The confirm button, so a modal layer focuses it on open.
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.confirm_focus.clone()
    }
}

impl ConfirmDialog {
    pub fn new(
        title: impl Into<SharedString>,
        message: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            title: title.into(),
            message: message.into(),
            confirm_label: "OK".into(),
            cancel_label: "Cancel".into(),
            danger: false,
            root_focus: cx.focus_handle(),
            confirm_focus: cx.focus_handle(),
            cancel_focus: cx.focus_handle(),
            on_confirm: None,
            on_cancel: None,
            done: false,
        }
    }

    pub fn confirm_label(mut self, label: impl Into<SharedString>) -> Self {
        self.confirm_label = label.into();
        self
    }

    pub fn cancel_label(mut self, label: impl Into<SharedString>) -> Self {
        self.cancel_label = label.into();
        self
    }

    /// Destructive action: the confirm button is drawn in the danger colour.
    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }

    pub fn on_confirm(mut self, callback: impl FnOnce(&mut Window, &mut App) + 'static) -> Self {
        self.on_confirm = Some(Box::new(callback));
        self
    }

    pub fn on_cancel(mut self, callback: impl FnOnce(&mut Window, &mut App) + 'static) -> Self {
        self.on_cancel = Some(Box::new(callback));
        self
    }

    /// Shows `dialog` centred in `layer`.
    pub fn show(dialog: Entity<Self>, layer: &Entity<ModalLayer>, window: &mut Window, cx: &mut App) {
        layer.update(cx, |layer, cx| {
            layer.show_with_placement(dialog, ModalPlacement::Center, window, cx)
        });
    }

    /// Confirms programmatically (as if the confirm button was pressed).
    pub fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.finish(true, window, cx);
    }

    /// Cancels programmatically.
    pub fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.finish(false, window, cx);
    }

    fn finish(&mut self, confirmed: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.done {
            return;
        }
        self.done = true;
        let callback = if confirmed {
            self.on_confirm.take()
        } else {
            self.on_cancel.take()
        };
        cx.emit(if confirmed {
            ConfirmDialogEvent::Confirmed
        } else {
            ConfirmDialogEvent::Cancelled
        });
        // Close first so focus is restored before the callback runs (the
        // callback may open another modal).
        cx.emit(DismissEvent);
        if let Some(callback) = callback {
            window.defer(cx, callback);
        }
    }

    fn on_confirm_action(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm(window, cx);
    }

    fn on_cancel_action(&mut self, _: &Cancel, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel(window, cx);
    }

    fn on_switch(&mut self, _: &SwitchButton, window: &mut Window, _: &mut Context<Self>) {
        if self.confirm_focus.is_focused(window) {
            window.focus(&self.cancel_focus);
        } else {
            window.focus(&self.confirm_focus);
        }
    }
}

impl Render for ConfirmDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let colors = theme.colors;
        let confirm = Button::new("confirm", self.confirm_label.clone())
            .variant(if self.danger {
                crate::ButtonVariant::Danger
            } else {
                crate::ButtonVariant::Primary
            })
            .track_focus(&self.confirm_focus)
            .on_click(cx.listener(|this, _, window, cx| this.confirm(window, cx)));
        let cancel = Button::new("cancel", self.cancel_label.clone())
            .track_focus(&self.cancel_focus)
            .on_click(cx.listener(|this, _, window, cx| this.cancel(window, cx)));

        div()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.root_focus)
            .on_action(cx.listener(Self::on_confirm_action))
            .on_action(cx.listener(Self::on_cancel_action))
            .on_action(cx.listener(Self::on_switch))
            .w(px(380.))
            .max_w_full()
            .p(px(20.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .bg(colors.elevated)
            .border_1()
            .border_color(colors.hairline)
            .rounded(RADIUS_LG)
            .shadow(theme.shadow_overlay())
            .font_family(theme.fonts.ui.clone())
            .child(
                div()
                    .text_size(UI_TEXT_LG)
                    .line_height(px(20.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(colors.ink)
                    .child(self.title.clone()),
            )
            .child(
                div()
                    .text_size(UI_TEXT_BASE)
                    .line_height(px(19.))
                    .text_color(colors.muted)
                    .child(self.message.clone()),
            )
            .child(
                div()
                    .mt(px(12.))
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(cancel)
                    .child(confirm),
            )
    }
}
