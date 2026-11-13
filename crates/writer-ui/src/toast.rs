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
//! A message that does not fit on one line wraps, onto at most
//! [`MESSAGE_LINES`] lines; what does not fit there ends in an ellipsis,
//! whatever the message is made of (a path is one long word). No message
//! is ever cut off at the edge of its toast. Put what matters first.
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

use std::{cell::RefCell, rc::Rc, time::Duration};

use gpui::{
    Action, App, AvailableSpace, Bounds, Context, Edges, Element, ElementId, Entity, Global,
    GlobalElementId, InspectorElementId, LayoutId, Pixels, SharedString, Size, Style, Task,
    TextAlign, TextRun, WeakEntity, Window, WrappedLine, div, prelude::*, px,
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
/// The most lines the message of a toast takes. A longer message ends in
/// an ellipsis on the last of them.
pub const MESSAGE_LINES: usize = 3;

/// What ends a message that was cut.
const ELLIPSIS: &str = "\u{2026}";

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
    /// The message as it was last drawn.
    drawn: Rc<RefCell<Option<DrawnMessage>>>,
    _timer: Task<()>,
}

/// The message of a toast as it was drawn: what a test holds against the
/// room the toast had.
#[derive(Clone, Debug, PartialEq)]
pub struct DrawnMessage {
    /// The text that was drawn: the message, or its beginning and an
    /// ellipsis when the whole did not fit on [`MESSAGE_LINES`] lines.
    pub text: String,
    /// The width the message had to fit in.
    pub width: Pixels,
    /// The width of each line that was drawn.
    pub lines: Vec<Pixels>,
}

impl DrawnMessage {
    /// Whether the message was cut and ends in an ellipsis.
    pub fn is_cut(&self) -> bool {
        self.text.ends_with(ELLIPSIS)
    }
}

/// The message of a toast: text that wraps to the width it is given, on
/// at most [`MESSAGE_LINES`] lines, the last of which ends in an ellipsis
/// when there is more.
///
/// GPUI's own `line_clamp(n).text_ellipsis()` does not do this: it stops
/// wrapping after `n - 1` breaks and leaves the rest of the text on the
/// last line, and it cuts the text at `n` times the width as if every line
/// were full. A long word that goes to the second line whole (a path)
/// leaves the first line half empty, and the last line then runs past the
/// edge of the toast, where it is clipped without an ellipsis (seen in a
/// capture of 0.5.0: "Exported the keyboard shortcuts to /private/tmp/…").
/// Here the wrapping is GPUI's (`shape_text`), and the last line is
/// shortened until what GPUI shapes fits.
struct Message {
    text: SharedString,
    drawn: Rc<RefCell<Option<DrawnMessage>>>,
}

/// What [`Message`] lays out and then paints.
#[derive(Default)]
struct LaidMessage {
    line: Option<WrappedLine>,
    /// The size `line` takes.
    size: Size<Pixels>,
    line_height: Pixels,
}

/// The byte index at which each line of `line` after the first begins.
fn line_starts(line: &WrappedLine) -> Vec<usize> {
    line.wrap_boundaries()
        .iter()
        .filter_map(|boundary| {
            line.unwrapped_layout
                .runs
                .get(boundary.run_ix)
                .and_then(|run| run.glyphs.get(boundary.glyph_ix))
                .map(|glyph| glyph.index)
        })
        .collect()
}

/// The width of each line `line` is drawn on.
fn line_widths(line: &WrappedLine) -> Vec<Pixels> {
    let layout = &line.unwrapped_layout;
    let mut starts: Vec<Pixels> = vec![px(0.)];
    starts.extend(line.wrap_boundaries().iter().filter_map(|boundary| {
        layout
            .runs
            .get(boundary.run_ix)
            .and_then(|run| run.glyphs.get(boundary.glyph_ix))
            .map(|glyph| glyph.position.x)
    }));
    let mut widths = Vec::with_capacity(starts.len());
    for (ix, start) in starts.iter().enumerate() {
        let end = starts.get(ix + 1).copied().unwrap_or(layout.width);
        widths.push(end - *start);
    }
    widths
}

/// `text` shaped for `width`: wrapped as GPUI wraps, and when that takes
/// more than `max_lines` lines, cut so that it takes `max_lines` and ends
/// in an ellipsis.
fn fit_message(
    text: &SharedString,
    width: Pixels,
    max_lines: usize,
    font_size: Pixels,
    run: &TextRun,
    window: &mut Window,
    cx: &mut App,
) -> Option<WrappedLine> {
    let shape = |text: SharedString, window: &mut Window| {
        let run = TextRun {
            len: text.len(),
            ..run.clone()
        };
        window
            .text_system()
            .shape_text(text, font_size, &[run], Some(width), None)
            .ok()
            .and_then(|mut lines| (!lines.is_empty()).then(|| lines.swap_remove(0)))
    };
    let whole = shape(text.clone(), window)?;
    let starts = line_starts(&whole);
    if starts.len() < max_lines {
        return Some(whole);
    }
    // Where the last line that is drawn begins; everything from there on
    // has to fit on it, with the ellipsis.
    let start = match max_lines.checked_sub(2) {
        Some(ix) => starts.get(ix).copied().unwrap_or(0),
        None => 0,
    };
    let head = &text[..start];
    let rest: SharedString = text[start..].trim_end().to_string().into();
    let mut tail = cx
        .text_system()
        .line_wrapper(run.font.clone(), font_size)
        .truncate_line(rest, width, ELLIPSIS, &mut Vec::new())
        .to_string();
    // The wrapper measures character by character and GPUI shapes the
    // whole line: one more character goes until what is shaped fits.
    loop {
        let shaped = shape(format!("{head}{tail}").into(), window)?;
        let body = tail.strip_suffix(ELLIPSIS).unwrap_or(&tail);
        if line_starts(&shaped).len() < max_lines || body.is_empty() {
            return Some(shaped);
        }
        let mut shorter: String = body.to_string();
        shorter.pop();
        tail = format!("{}{ELLIPSIS}", shorter.trim_end());
    }
}

impl IntoElement for Message {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for Message {
    type RequestLayoutState = Rc<RefCell<LaidMessage>>;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        _: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let text_style = window.text_style();
        let font_size = text_style.font_size.to_pixels(window.rem_size());
        let line_height = text_style.line_height_in_pixels(window.rem_size());
        // One paragraph: a line break in a message is a space.
        let text: SharedString = if self.text.contains('\n') {
            self.text.replace('\n', " ").into()
        } else {
            self.text.clone()
        };
        let run = text_style.to_run(text.len());
        let laid = Rc::new(RefCell::new(LaidMessage {
            line: None,
            size: Size::default(),
            line_height,
        }));
        let layout_id = window.request_measured_layout(Style::default(), {
            let laid = laid.clone();
            let drawn = self.drawn.clone();
            move |known, available, window, cx| {
                let width = known.width.or(match available.width {
                    AvailableSpace::Definite(width) => Some(width),
                    _ => None,
                });
                let line = match width {
                    Some(width) => {
                        fit_message(&text, width, MESSAGE_LINES, font_size, &run, window, cx)
                    }
                    // Asked how wide the message would like to be. Once it
                    // was laid out for a width, that stands (GPUI's own
                    // text answers the same way); before that, the whole
                    // of it on one line.
                    None if laid.borrow().line.is_some() => return laid.borrow().size,
                    None => window
                        .text_system()
                        .shape_text(
                            text.clone(),
                            font_size,
                            std::slice::from_ref(&run),
                            None,
                            None,
                        )
                        .ok()
                        .and_then(|mut lines| (!lines.is_empty()).then(|| lines.swap_remove(0))),
                };
                let Some(line) = line else {
                    *laid.borrow_mut() = LaidMessage {
                        line: None,
                        size: Size::default(),
                        line_height,
                    };
                    return Size::default();
                };
                let widths = line_widths(&line);
                let size = Size {
                    width: widths.iter().copied().fold(px(0.), Pixels::max).ceil(),
                    height: line_height * widths.len(),
                };
                *drawn.borrow_mut() = Some(DrawnMessage {
                    text: line.text.to_string(),
                    // Without a width to fit in, it had all it wanted.
                    width: width.unwrap_or(size.width),
                    lines: widths,
                });
                *laid.borrow_mut() = LaidMessage {
                    line: Some(line),
                    size,
                    line_height,
                };
                size
            }
        });
        (layout_id, laid)
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Window,
        _: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        laid: &mut Self::RequestLayoutState,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let laid = laid.borrow();
        if let Some(line) = &laid.line {
            line.paint(
                bounds.origin,
                laid.line_height,
                TextAlign::Left,
                Some(bounds),
                window,
                cx,
            )
            .ok();
        }
    }
}

/// See the [module docs](self).
pub struct ToastLayer {
    entries: Vec<Entry>,
    next_id: u64,
    insets: Edges<Pixels>,
    /// Where each toast was drawn in the last frame, in window
    /// coordinates (written by the stack's prepaint).
    drawn_bounds: Rc<RefCell<Vec<(ToastId, Bounds<Pixels>)>>>,
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
            drawn_bounds: Rc::default(),
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
            drawn: Rc::default(),
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

    /// The message of toast `id` as it was last drawn: the text (cut and
    /// ended with an ellipsis when it did not fit), the room it had and
    /// the width of each of its lines. `None` before the toast was drawn.
    pub fn drawn_message(&self, id: ToastId) -> Option<DrawnMessage> {
        self.entries
            .iter()
            .find(|entry| entry.id == id)
            .and_then(|entry| entry.drawn.borrow().clone())
    }

    /// Where each visible toast was drawn in the last frame, in window
    /// coordinates, oldest first; empty without toasts. A toast pushed
    /// since the last frame is not among them until it is drawn.
    ///
    /// Toasts only: the open context menu, which this layer draws too, is
    /// never among them. (What a native view over the window cuts holes
    /// for, so that the toasts above it can be seen and clicked.)
    pub fn toast_bounds(&self) -> Vec<Bounds<Pixels>> {
        let drawn = self.drawn_bounds.borrow();
        self.entries
            .iter()
            .filter_map(|entry| {
                drawn
                    .iter()
                    .find(|(id, _)| *id == entry.id)
                    .map(|(_, bounds)| *bounds)
            })
            .collect()
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
            self.drawn_bounds.borrow_mut().clear();
            // The layer is mounted over the whole window content by every
            // root view, which makes it the host of the context menu too.
            return div().child(ContextMenuLayer).into_any_element();
        }
        let theme = cx.theme();
        let colors = theme.colors;
        let ids = self.ids();
        let drawn_bounds = self.drawn_bounds.clone();
        div()
            .on_children_prepainted(move |bounds, _, _| {
                // The children are the toasts, in order, and then the
                // context menu's layer, which is left out.
                *drawn_bounds.borrow_mut() = ids.iter().copied().zip(bounds).collect();
            })
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
                    .child(div().flex_1().min_w_0().overflow_hidden().child(Message {
                        text: entry.toast.message.clone(),
                        drawn: entry.drawn.clone(),
                    }))
                    .children(action)
            }))
            .child(ContextMenuLayer)
            .into_any_element()
    }
}
