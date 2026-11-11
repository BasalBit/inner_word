//! Popup hosting: one arbitrary view floating at the caret, the selection or
//! a document position.
//!
//! The editor owns placement, the consumer owns content and focus. The popup
//! is laid out and positioned every frame by [`PopupElement`], a sibling of
//! the editor's text surface that prepaints after it, so it uses the
//! geometry of the same frame: the popup follows scrolling and edits without
//! lagging a frame. It is drawn with `window.defer_draw`, on top of the rest
//! of the window.
//!
//! Placement: the popup's left edge is aligned with the anchor's left edge,
//! on the preferred side ([`PopupPlacement`]) with a small gap. If it does
//! not fit on that side of the anchor inside the editor's viewport and the
//! other side has more room, it flips. It is then moved to stay inside the
//! viewport, also when the anchor has been scrolled out of view.
//!
//! Because the popup is not inside the editor's `"Editor"` key context, a
//! view that takes focus (a link field) gets its keys without the editor's
//! bindings interfering; a view that leaves focus in the editor (a menu) is
//! driven through [`crate::Editor::intercept_keys`].
//!
//! Focus inside the popup still counts as focus in the editor for
//! [`crate::EditorEvent::Focused`] and [`crate::EditorEvent::Blurred`]:
//! moving the keyboard from the text into the popup or back emits neither
//! ([`Editor::popup_focused`] tells where it is).

use gpui::{
    AnyView, App, AvailableSpace, Bounds, Context, Element, ElementId, Entity, GlobalElementId,
    InspectorElementId, IntoElement, LayoutId, Pixels, Position as StylePosition, Size, Style,
    Window, point, px,
};
use writer_core::Position;

use crate::editor::Editor;

/// What a popup is attached to. The anchor is re-read every frame, so
/// [`PopupAnchor::Caret`] and [`PopupAnchor::Selection`] follow the caret
/// and the selection as they change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PopupAnchor {
    /// The caret (the selection head): menus that open where you type.
    Caret,
    /// The rectangle around the whole selection: the format menu.
    Selection,
    /// A fixed document position, for example where a `/` was typed. A
    /// position does not move with edits before it; call
    /// [`Editor::set_popup_anchor`] if that matters.
    Position(Position),
}

/// The side of the anchor a popup prefers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum PopupPlacement {
    /// Under the anchor (flips above when there is no room).
    #[default]
    Below,
    /// Over the anchor (flips below when there is no room).
    Above,
}

pub(crate) struct Popup {
    pub(crate) view: AnyView,
    pub(crate) anchor: PopupAnchor,
    pub(crate) placement: PopupPlacement,
}

/// Distance between the anchor and the popup.
const POPUP_GAP: Pixels = px(6.);
/// Distance kept from the viewport edges.
const POPUP_MARGIN: Pixels = px(8.);

impl Editor {
    /// Shows `view` as the editor's popup, replacing any other. The editor
    /// places it (left edge on the anchor's left edge, on the preferred
    /// side, flipped when it does not fit there, always kept inside the
    /// editor's viewport, re-placed every frame) and never touches
    /// focus: to give the popup the keyboard, focus something inside the
    /// view after this call and restore the editor's focus when you hide
    /// it; to keep typing in the document, leave focus alone and register
    /// a key interceptor.
    ///
    /// ```ignore
    /// let menu = cx.new(|cx| FormatMenu::new(window, cx));
    /// editor.update(cx, |editor, cx| {
    ///     editor.show_popup(menu.clone().into(), PopupAnchor::Selection, PopupPlacement::Above, cx)
    /// });
    /// // later
    /// editor.update(cx, |editor, cx| editor.hide_popup(cx));
    /// ```
    pub fn show_popup(
        &mut self,
        view: AnyView,
        anchor: PopupAnchor,
        placement: PopupPlacement,
        cx: &mut Context<Self>,
    ) {
        self.popup = Some(Popup {
            view,
            anchor,
            placement,
        });
        cx.notify();
    }

    /// Hides the popup, if any. Does not change focus.
    pub fn hide_popup(&mut self, cx: &mut Context<Self>) {
        if self.popup.take().is_some() {
            cx.notify();
        }
    }

    /// Whether a popup is showing.
    pub fn has_popup(&self) -> bool {
        self.popup.is_some()
    }

    /// Whether the keyboard is inside the showing popup (something in its
    /// view is focused). The editor's own focus handle is then not
    /// focused ([`Editor::is_focused`] is false), but the editor does not
    /// count as blurred: [`crate::EditorEvent::Blurred`] is only emitted
    /// when the keyboard leaves both the text and the popup.
    pub fn popup_focused(&self, window: &Window, cx: &App) -> bool {
        self.popup.is_some() && self.popup_scope.contains_focused(window, cx)
    }

    /// The view shown as the popup, if any (compare its `entity_id()` to
    /// find out whether your popup is still the one showing).
    pub fn popup_view(&self) -> Option<&AnyView> {
        self.popup.as_ref().map(|popup| &popup.view)
    }

    /// The anchor of the showing popup.
    pub fn popup_anchor(&self) -> Option<PopupAnchor> {
        self.popup.as_ref().map(|popup| popup.anchor)
    }

    /// Moves the showing popup to another anchor.
    pub fn set_popup_anchor(&mut self, anchor: PopupAnchor, cx: &mut Context<Self>) {
        if let Some(popup) = &mut self.popup
            && popup.anchor != anchor
        {
            popup.anchor = anchor;
            cx.notify();
        }
    }

    /// The window rectangle the popup is attached to.
    fn popup_anchor_bounds(&self, anchor: PopupAnchor) -> Option<Bounds<Pixels>> {
        match anchor {
            PopupAnchor::Caret => self.caret_bounds(),
            PopupAnchor::Selection => self.selection_bounds(),
            PopupAnchor::Position(position) => self.position_bounds(position),
        }
    }
}

/// Where a popup of `size` goes for an anchor rectangle inside `viewport`.
pub(crate) fn place_popup(
    anchor: Bounds<Pixels>,
    size: Size<Pixels>,
    viewport: Bounds<Pixels>,
    placement: PopupPlacement,
) -> gpui::Point<Pixels> {
    let room_below = viewport.bottom() - anchor.bottom() - POPUP_GAP - POPUP_MARGIN;
    let room_above = anchor.top() - viewport.top() - POPUP_GAP - POPUP_MARGIN;
    let below = match placement {
        PopupPlacement::Below => size.height <= room_below || room_below >= room_above,
        PopupPlacement::Above => size.height > room_above && room_below > room_above,
    };
    let y = if below {
        anchor.bottom() + POPUP_GAP
    } else {
        anchor.top() - POPUP_GAP - size.height
    };
    // Stay inside the viewport; when the popup is larger than the viewport
    // its top-left corner wins.
    let max_x = (viewport.right() - POPUP_MARGIN - size.width).max(viewport.left() + POPUP_MARGIN);
    let max_y = (viewport.bottom() - POPUP_MARGIN - size.height).max(viewport.top() + POPUP_MARGIN);
    point(
        anchor.left().clamp(viewport.left() + POPUP_MARGIN, max_x),
        y.clamp(viewport.top() + POPUP_MARGIN, max_y),
    )
}

/// The zero-size element that lays out and positions the editor's popup.
pub(crate) struct PopupElement {
    editor: Entity<Editor>,
}

impl PopupElement {
    pub(crate) fn new(editor: Entity<Editor>) -> Self {
        Self { editor }
    }
}

impl IntoElement for PopupElement {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for PopupElement {
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
        // Out of flow: the popup never takes space in the editor.
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
        // The text surface was prepainted just before this element, so the
        // editor's geometry is this frame's.
        let editor = self.editor.read(cx);
        let Some(popup) = &editor.popup else {
            return;
        };
        let (Some(anchor), Some(viewport)) = (
            editor.popup_anchor_bounds(popup.anchor),
            editor.viewport_bounds(),
        ) else {
            return;
        };
        let placement = popup.placement;
        // The popup is drawn later, as a child of this element in the
        // focus tree. Giving this element a focus handle of its own lets
        // the editor ask "is the keyboard in my popup?" whatever the view
        // is made of.
        window.set_focus_handle(&editor.popup_scope, cx);
        let mut element = popup.view.clone().into_any_element();
        let size = element.layout_as_root(AvailableSpace::min_size(), window, cx);
        let origin = place_popup(anchor, size, viewport, placement);
        window.defer_draw(element, origin, 1);
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        _: &mut Window,
        _: &mut App,
    ) {
    }
}
