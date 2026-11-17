//! The element that places and draws the popover.
//!
//! [`crate::QuickChat`] renders this element and nothing else. It takes no
//! room. In the frame's prepaint, which runs after the editor's when the
//! view is mounted after the editor, it asks the editor where the text of
//! the chat is, decides the side and the size the popover may take, lays
//! the popover out and hands it to the window to be drawn on top of
//! everything (`Window::defer_draw`): the popover follows scrolling and
//! edits in the same frame. In the paint that follows, when the popover's
//! own prepaint has run, it looks whether the keyboard is still in the
//! popover and whether the conversation wants more room than it had.

use gpui::{
    App, Bounds, Element, ElementId, Entity, GlobalElementId, InspectorElementId, IntoElement,
    LayoutId, Pixels, Position as StylePosition, Style, Window,
};

use crate::popover::QuickChat;

/// Over the editor's own popups and the format bar, under context menus.
pub(crate) const DRAW_PRIORITY: usize = 2;

pub(crate) struct Layer {
    chat: Entity<QuickChat>,
}

impl Layer {
    pub(crate) fn new(chat: Entity<QuickChat>) -> Self {
        Self { chat }
    }
}

impl IntoElement for Layer {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for Layer {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        // Out of flow: the popover never takes room where it is mounted.
        let style = Style {
            position: StylePosition::Absolute,
            ..Style::default()
        };
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        QuickChat::prepaint_popover(&self.chat, window, cx);
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        QuickChat::after_prepaint(&self.chat, window, cx);
    }
}
