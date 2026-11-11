//! [`ToastLayer`] and [`Toast`]: brief, non-blocking messages ("Exported to
//! ~/Documents/Essay.md") stacked at the bottom of the window. Toasts never
//! take focus and dismiss themselves after a few seconds; clicking one
//! dismisses it early.
//!
//! ```ignore
//! use writer_ui::{Toast, ToastKind, ToastLayer};
//!
//! // the window root owns one layer and renders it over the content
//! let toasts = cx.new(|_| ToastLayer::new());
//! div().relative().size_full().child(content).child(modal_layer).child(toasts.clone())
//!
//! toasts.update(cx, |layer, cx| {
//!     layer.push(Toast::new("Exported to ~/Documents/Essay.md").kind(ToastKind::Success), cx);
//! });
//!
//! // or from anywhere, after installing the layer as the global one:
//! ToastLayer::set_global(&toasts, cx);
//! writer_ui::toast::show_toast(Toast::new("Copied link"), cx);
//! ```

use std::time::Duration;

use gpui::{
    App, Context, Entity, Global, MouseButton, SharedString, Task, WeakEntity, Window, div,
    prelude::*, px,
};

use crate::{
    context_menu::ContextMenuLayer,
    icon::{Icon, IconName},
    theme::{ActiveTheme, RADIUS_LG, UI_TEXT_BASE},
};

/// How long a toast stays by default.
pub const DEFAULT_DURATION: Duration = Duration::from_secs(4);
/// At most this many toasts are shown; older ones are dropped.
pub const MAX_VISIBLE: usize = 4;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ToastKind {
    #[default]
    Info,
    /// Shows a check mark in the success colour.
    Success,
    /// Shows an alert mark in the danger colour.
    Error,
}

/// One message. Build it, then [`ToastLayer::push`] it.
#[derive(Clone, Debug, PartialEq)]
pub struct Toast {
    pub message: SharedString,
    pub kind: ToastKind,
    pub duration: Duration,
}

impl Toast {
    pub fn new(message: impl Into<SharedString>) -> Self {
        Self {
            message: message.into(),
            kind: ToastKind::Info,
            duration: DEFAULT_DURATION,
        }
    }

    pub fn kind(mut self, kind: ToastKind) -> Self {
        self.kind = kind;
        self
    }

    pub fn success(self) -> Self {
        self.kind(ToastKind::Success)
    }

    pub fn error(self) -> Self {
        self.kind(ToastKind::Error)
    }

    pub fn duration(mut self, duration: Duration) -> Self {
        self.duration = duration;
        self
    }
}

/// Identifies a pushed toast, for [`ToastLayer::dismiss`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ToastId(u64);

struct Entry {
    id: ToastId,
    toast: Toast,
    _timer: Task<()>,
}

/// See the [module docs](self).
#[derive(Default)]
pub struct ToastLayer {
    entries: Vec<Entry>,
    next_id: u64,
}

struct GlobalToastLayer(WeakEntity<ToastLayer>);
impl Global for GlobalToastLayer {}

impl ToastLayer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Shows `toast`; it dismisses itself after `toast.duration`.
    pub fn push(&mut self, toast: Toast, cx: &mut Context<Self>) -> ToastId {
        let id = ToastId(self.next_id);
        self.next_id += 1;
        let duration = toast.duration;
        let timer = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(duration).await;
            this.update(cx, |this, cx| this.dismiss(id, cx)).ok();
        });
        self.entries.push(Entry {
            id,
            toast,
            _timer: timer,
        });
        if self.entries.len() > MAX_VISIBLE {
            let excess = self.entries.len() - MAX_VISIBLE;
            self.entries.drain(..excess);
        }
        cx.notify();
        id
    }

    pub fn dismiss(&mut self, id: ToastId, cx: &mut Context<Self>) {
        let before = self.entries.len();
        self.entries.retain(|entry| entry.id != id);
        if self.entries.len() != before {
            cx.notify();
        }
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.entries.clear();
        cx.notify();
    }

    /// The visible toasts, oldest first.
    pub fn toasts(&self) -> impl Iterator<Item = &Toast> {
        self.entries.iter().map(|entry| &entry.toast)
    }

    /// Makes `layer` the target of [`show_toast`].
    pub fn set_global(layer: &Entity<ToastLayer>, cx: &mut App) {
        cx.set_global(GlobalToastLayer(layer.downgrade()));
    }
}

/// Pushes `toast` to the layer installed with [`ToastLayer::set_global`].
/// Returns `None` when no layer is installed (or it was dropped).
pub fn show_toast(toast: Toast, cx: &mut App) -> Option<ToastId> {
    let layer = cx.try_global::<GlobalToastLayer>()?.0.upgrade()?;
    Some(layer.update(cx, |layer, cx| layer.push(toast, cx)))
}

impl Render for ToastLayer {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.entries.is_empty() {
            // The layer is mounted over the whole window content by every
            // root view, which makes it the host of the context menu too.
            return div().child(ContextMenuLayer).into_any_element();
        }
        let theme = cx.theme();
        let colors = theme.colors;
        div()
            .absolute()
            .bottom(px(24.))
            .left_0()
            .right_0()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(8.))
            .children(self.entries.iter().map(|entry| {
                let id = entry.id;
                let icon = match entry.toast.kind {
                    ToastKind::Info => None,
                    ToastKind::Success => Some(Icon::new(IconName::Check).color(colors.success)),
                    ToastKind::Error => Some(Icon::new(IconName::Alert).color(colors.danger)),
                };
                div()
                    .id(("toast", id.0 as usize))
                    .occlude()
                    .max_w(px(520.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(14.))
                    .py(px(9.))
                    .rounded(RADIUS_LG)
                    .bg(colors.elevated)
                    .border_1()
                    .border_color(colors.hairline)
                    .shadow(theme.shadow_overlay())
                    .font_family(theme.fonts.ui.clone())
                    .text_size(UI_TEXT_BASE)
                    .line_height(px(18.))
                    .text_color(colors.ink)
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| this.dismiss(id, cx)),
                    )
                    .children(icon)
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .child(entry.toast.message.clone()),
                    )
            }))
            .child(ContextMenuLayer)
            .into_any_element()
    }
}
