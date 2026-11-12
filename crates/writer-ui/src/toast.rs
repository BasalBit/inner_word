//! [`ToastLayer`] and [`Toast`]: brief, non-blocking messages ("Exported to
//! ~/Documents/Essay.md") stacked at the bottom of the window. Toasts never
//! take focus and dismiss themselves after a few seconds; clicking one
//! dismisses it early (it lights up under the pointer).
//!
//! A toast can carry one action, a small button after the message, so that
//! what the message offers can be done with the mouse:
//!
//! ```ignore
//! // dispatches the action to the element that has the keyboard, shows its key
//! Toast::new("Moved “Draft” to the trash.").action("Undo", UndoLibraryChange)
//! // or runs a closure
//! Toast::new("Exported Essay.md").on_action("Show in Finder", move |_, cx| cx.reveal_path(&path))
//! ```
//!
//! A message that does not fit on one line wraps onto a second one; what
//! does not fit there ends in an ellipsis. Put what matters first.
//!
//! The stack sits [`DEFAULT_BOTTOM_INSET`] above the window's bottom edge,
//! clear of a status bar, and is centred in the window. A root view with
//! side panels centres it on its main column with [`ToastLayer::set_insets`].
//!
//! The layer also draws the window's open [`crate::ContextMenu`] (it is the
//! one layer every root view mounts over its whole content).
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

use std::{rc::Rc, time::Duration};

use gpui::{
    Action, App, Context, Edges, Entity, Global, Pixels, SharedString, Task, WeakEntity, Window,
    div, prelude::*, px,
};

use crate::{
    context_menu::ContextMenuLayer,
    icon::{Icon, IconName},
    key_hint::KeyHint,
    theme::{ActiveTheme, RADIUS_LG, RADIUS_SM, UI_TEXT_BASE},
};

/// How long a toast stays by default.
pub const DEFAULT_DURATION: Duration = Duration::from_secs(4);
/// At most this many toasts are shown; older ones are dropped.
pub const MAX_VISIBLE: usize = 4;
/// The gap between the stack and the bottom edge of the window: a status
/// bar (26px in the app) and some air above it.
pub const DEFAULT_BOTTOM_INSET: Pixels = px(44.);
/// The widest a toast gets.
pub const MAX_WIDTH: Pixels = px(520.);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ToastKind {
    #[default]
    Info,
    /// Shows a check mark in the success colour.
    Success,
    /// Shows an alert mark in the danger colour.
    Error,
}

type ActionHandler = Rc<dyn Fn(&mut Window, &mut App)>;

/// The button of a toast.
#[derive(Clone)]
struct ToastAction {
    label: SharedString,
    action: Option<Rc<dyn Action>>,
    handler: Option<ActionHandler>,
}

/// One message. Build it, then [`ToastLayer::push`] it.
#[derive(Clone)]
pub struct Toast {
    pub message: SharedString,
    pub kind: ToastKind,
    pub duration: Duration,
    action: Option<ToastAction>,
}

impl PartialEq for Toast {
    fn eq(&self, other: &Self) -> bool {
        self.message == other.message
            && self.kind == other.kind
            && self.duration == other.duration
            && self.action_label() == other.action_label()
    }
}

impl std::fmt::Debug for Toast {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Toast")
            .field("message", &self.message)
            .field("kind", &self.kind)
            .field("duration", &self.duration)
            .field("action", &self.action_label())
            .finish()
    }
}

impl Toast {
    pub fn new(message: impl Into<SharedString>) -> Self {
        Self {
            message: message.into(),
            kind: ToastKind::Info,
            duration: DEFAULT_DURATION,
            action: None,
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

    /// A button after the message that dispatches `action` to the element
    /// that has the keyboard (as the action's key does) and closes the
    /// toast. The key bound to the action is shown on the button.
    pub fn action(mut self, label: impl Into<SharedString>, action: impl Action) -> Self {
        self.action = Some(ToastAction {
            label: label.into(),
            action: Some(Rc::new(action)),
            handler: None,
        });
        self
    }

    /// A button after the message that runs `handler` and closes the toast.
    pub fn on_action(
        mut self,
        label: impl Into<SharedString>,
        handler: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        self.action = Some(ToastAction {
            label: label.into(),
            action: None,
            handler: Some(Rc::new(handler)),
        });
        self
    }

    /// The label of the toast's button.
    pub fn action_label(&self) -> Option<&SharedString> {
        self.action.as_ref().map(|action| &action.label)
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
pub struct ToastLayer {
    entries: Vec<Entry>,
    next_id: u64,
    insets: Edges<Pixels>,
}

impl Default for ToastLayer {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            next_id: 0,
            insets: Edges {
                bottom: DEFAULT_BOTTOM_INSET,
                ..Edges::default()
            },
        }
    }
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

    /// Where the stack sits: `bottom` above the window's bottom edge, and
    /// centred between `left` and `right` (the widths of what flanks the
    /// main column: a sidebar, a side panel), so a toast is centred on the
    /// text and not on the window. `top` is not used.
    pub fn set_insets(&mut self, insets: Edges<Pixels>, cx: &mut Context<Self>) {
        if self.insets != insets {
            self.insets = insets;
            cx.notify();
        }
    }

    pub fn insets(&self) -> Edges<Pixels> {
        self.insets
    }

    /// Runs the button of toast `id` and closes the toast. Returns whether
    /// the toast has one. (What a click on the button does.)
    pub fn run_action(&mut self, id: ToastId, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(action) = self
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .and_then(|entry| entry.toast.action.clone())
        else {
            return false;
        };
        self.dismiss(id, cx);
        if let Some(action) = action.action {
            window.dispatch_action(action.boxed_clone(), cx);
        }
        if let Some(handler) = action.handler {
            window.defer(cx, move |window, cx| handler(window, cx));
        }
        true
    }

    /// The ids of the visible toasts, oldest first.
    pub fn ids(&self) -> Vec<ToastId> {
        self.entries.iter().map(|entry| entry.id).collect()
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.entries.is_empty() {
            // The layer is mounted over the whole window content by every
            // root view, which makes it the host of the context menu too.
            return div().child(ContextMenuLayer).into_any_element();
        }
        let theme = cx.theme();
        let colors = theme.colors;
        div()
            .absolute()
            .bottom(self.insets.bottom)
            .left(self.insets.left)
            .right(self.insets.right)
            .flex()
            .flex_col()
            .items_center()
            .gap(px(8.))
            .px(px(16.))
            .children(self.entries.iter().map(|entry| {
                let id = entry.id;
                let icon = match entry.toast.kind {
                    ToastKind::Info => None,
                    ToastKind::Success => Some(Icon::new(IconName::Check).color(colors.success)),
                    ToastKind::Error => Some(Icon::new(IconName::Alert).color(colors.danger)),
                };
                let action = entry.toast.action.as_ref().map(|action| {
                    // The key that does the same from where the keyboard
                    // is now (an action can be bound in several places,
                    // each with a key of its own).
                    let hint = action
                        .action
                        .as_ref()
                        .and_then(|action| KeyHint::for_action_here(action.as_ref(), window, cx));
                    div()
                        .id("toast-action")
                        .debug_selector(|| format!("toast-action-{}", id.0))
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .h(px(22.))
                        .px(px(8.))
                        .rounded(RADIUS_SM)
                        .border_1()
                        .border_color(colors.border_strong)
                        .text_color(colors.ink)
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .whitespace_nowrap()
                        .cursor_pointer()
                        .hover(|style| style.bg(colors.hover))
                        .active(|style| style.bg(colors.hover).opacity(0.8))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            // not also a click on the toast around the button
                            cx.stop_propagation();
                            this.run_action(id, window, cx);
                        }))
                        .child(action.label.clone())
                        .children(hint)
                });
                div()
                    .id(("toast", id.0 as usize))
                    .debug_selector(|| format!("toast-{}", id.0))
                    .occlude()
                    .max_w(MAX_WIDTH)
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
                    .hover(|style| style.border_color(colors.border_strong))
                    .on_click(cx.listener(move |this, _, _, cx| this.dismiss(id, cx)))
                    .children(icon)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .line_clamp(2)
                            .text_ellipsis()
                            .child(entry.toast.message.clone()),
                    )
                    .children(action)
            }))
            .child(ContextMenuLayer)
            .into_any_element()
    }
}
