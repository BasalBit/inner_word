//! [`SegmentedControl`]: pick one of a few options (theme, view mode).
//!
//! ```ignore
//! use writer_ui::SegmentedControl;
//!
//! SegmentedControl::new("theme", ["Paper", "Night", "System"], self.theme_ix)
//!     .on_select(cx.listener(|this, ix: &usize, _window, cx| {
//!         this.theme_ix = *ix;
//!         cx.notify();
//!     }))
//! ```
//!
//! Keyboard: the whole control is one tab stop; `left` / `right` move the
//! selection (in the `"SegmentedControl"` key context). Stateless: the parent
//! owns the selected index.

use std::rc::Rc;

use gpui::{
    App, ElementId, FocusHandle, KeyBinding, SharedString, Window, actions, div, prelude::*, px,
};

use crate::theme::{ActiveTheme, RADIUS_MD, RADIUS_SM, UI_TEXT_SM, focus_ring};

pub const KEY_CONTEXT: &str = "SegmentedControl";

actions!(
    segmented,
    [
        /// Select the previous segment.
        SelectPrevSegment,
        /// Select the next segment.
        SelectNextSegment
    ]
);

pub fn key_bindings() -> Vec<KeyBinding> {
    let c = Some(KEY_CONTEXT);
    vec![
        KeyBinding::new("left", SelectPrevSegment, c),
        KeyBinding::new("right", SelectNextSegment, c),
    ]
}

type SelectHandler = Rc<dyn Fn(&usize, &mut Window, &mut App)>;

/// See the [module docs](self).
#[derive(IntoElement)]
pub struct SegmentedControl {
    id: ElementId,
    options: Vec<SharedString>,
    selected: usize,
    focus_handle: Option<FocusHandle>,
    on_select: Option<SelectHandler>,
}

impl SegmentedControl {
    pub fn new(
        id: impl Into<ElementId>,
        options: impl IntoIterator<Item = impl Into<SharedString>>,
        selected: usize,
    ) -> Self {
        Self {
            id: id.into(),
            options: options.into_iter().map(Into::into).collect(),
            selected,
            focus_handle: None,
            on_select: None,
        }
    }

    /// Use this focus handle instead of an automatic one (make it a tab stop).
    pub fn track_focus(mut self, handle: &FocusHandle) -> Self {
        self.focus_handle = Some(handle.clone());
        self
    }

    /// Called with the newly selected index.
    pub fn on_select(mut self, handler: impl Fn(&usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_select = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for SegmentedControl {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.theme().colors;
        let selected = self.selected;
        let count = self.options.len();
        let focused = self
            .focus_handle
            .as_ref()
            .is_some_and(|handle| handle.is_focused(window));
        let ring = focus_ring(&colors);
        let on_prev = self.on_select.clone();
        let on_next = self.on_select.clone();

        div()
            .id(self.id)
            .key_context(KEY_CONTEXT)
            .map(|el| match &self.focus_handle {
                Some(handle) => el.track_focus(handle),
                None => el.tab_index(0),
            })
            .flex()
            .flex_none()
            .p(px(2.))
            .gap(px(2.))
            .rounded(RADIUS_MD)
            .bg(colors.hover)
            .border_1()
            .border_color(colors.hairline)
            .when(focused, |el| {
                el.border_color(colors.accent).shadow(ring.clone())
            })
            .focus({
                let accent = colors.accent;
                move |style| style.border_color(accent).shadow(ring)
            })
            .on_action(move |_: &SelectPrevSegment, window, cx| {
                if selected > 0
                    && let Some(handler) = on_prev.as_ref()
                {
                    handler(&(selected - 1), window, cx);
                }
            })
            .on_action(move |_: &SelectNextSegment, window, cx| {
                if selected + 1 < count
                    && let Some(handler) = on_next.as_ref()
                {
                    handler(&(selected + 1), window, cx);
                }
            })
            .children(self.options.into_iter().enumerate().map(|(ix, label)| {
                let is_selected = ix == selected;
                let handler = self.on_select.clone();
                div()
                    .id(ix)
                    .px(px(10.))
                    .h(px(22.))
                    .flex()
                    .items_center()
                    .rounded(RADIUS_SM)
                    .text_size(UI_TEXT_SM)
                    .line_height(px(16.))
                    .whitespace_nowrap()
                    .cursor_pointer()
                    .map(|el| {
                        if is_selected {
                            // On Night the elevated surface is too close to
                            // the track; use the stronger border tone.
                            el.bg(if colors.appearance.is_dark() {
                                colors.border_strong
                            } else {
                                colors.page
                            })
                            .text_color(colors.ink)
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .shadow(cx.theme().shadow_popover())
                        } else {
                            el.text_color(colors.muted).hover(|style| {
                                style
                                    .text_color(colors.ink)
                                    .bg(colors.border_strong.opacity(0.35))
                            })
                        }
                    })
                    .on_click(move |event, window, cx| {
                        if let (gpui::ClickEvent::Mouse(_), Some(handler)) =
                            (event, handler.as_ref())
                        {
                            handler(&ix, window, cx);
                        }
                    })
                    .child(label)
            }))
    }
}
