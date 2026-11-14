//! The custom element that paints a [`TextField`]'s text, selection and caret
//! and registers the platform input handler.

use gpui::{
    App, Bounds, ContentMask, DispatchPhase, Element, ElementId, ElementInputHandler, Entity,
    GlobalElementId, InspectorElementId, IntoElement, LayoutId, MouseMoveEvent, MouseUpEvent,
    PaintQuad, Pixels, ShapedLine, SharedString, Style, TextRun, UnderlineStyle, Window, fill,
    point, px, relative, size,
};

use super::TextField;
use crate::theme::ActiveTheme;

const CARET_WIDTH: Pixels = px(2.);

pub(super) struct TextFieldElement {
    field: Entity<TextField>,
}

impl TextFieldElement {
    pub(super) fn new(field: Entity<TextField>) -> Self {
        Self { field }
    }
}

pub(super) struct PrepaintState {
    line: ShapedLine,
    placeholder: Option<ShapedLine>,
    selection: Option<PaintQuad>,
    caret: Option<PaintQuad>,
    scroll_x: Pixels,
}

impl IntoElement for TextFieldElement {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for TextFieldElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

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
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = window.line_height().into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> PrepaintState {
        let colors = cx.theme().colors;
        let field = self.field.read(cx);
        let focused = field.focus_handle.is_focused(window);
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let text: SharedString = field.text.clone().into();
        let base_run = TextRun {
            len: text.len(),
            font: style.font(),
            color: style.color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };

        let runs = match field.marked.as_ref() {
            Some(marked) if marked.end <= text.len() => [
                TextRun {
                    len: marked.start,
                    ..base_run.clone()
                },
                TextRun {
                    len: marked.end - marked.start,
                    underline: Some(UnderlineStyle {
                        color: Some(style.color),
                        thickness: px(1.),
                        wavy: false,
                    }),
                    ..base_run.clone()
                },
                TextRun {
                    len: text.len() - marked.end,
                    ..base_run.clone()
                },
            ]
            .into_iter()
            .filter(|run| run.len > 0)
            .collect::<Vec<_>>(),
            _ => vec![base_run.clone()],
        };
        let line = window
            .text_system()
            .shape_line(text.clone(), font_size, &runs, None);

        let placeholder = (text.is_empty() && !field.placeholder.is_empty()).then(|| {
            let placeholder = field.placeholder.clone();
            window.text_system().shape_line(
                placeholder.clone(),
                font_size,
                &[TextRun {
                    len: placeholder.len(),
                    color: colors.faint,
                    ..base_run.clone()
                }],
                None,
            )
        });

        // Keep the caret inside the visible width.
        let cursor = field.cursor();
        let caret_x = line.x_for_index(cursor);
        let width = bounds.size.width;
        let mut scroll_x = field.scroll_x;
        if caret_x - scroll_x > width - CARET_WIDTH {
            scroll_x = caret_x - width + CARET_WIDTH;
        }
        if caret_x < scroll_x {
            scroll_x = caret_x;
        }
        let max_scroll = (line.width + CARET_WIDTH - width).max(px(0.));
        scroll_x = scroll_x.clamp(px(0.), max_scroll);
        // Like AppKit, an unfocused field shows the start of its text.
        if !focused {
            scroll_x = px(0.);
        }

        let left = bounds.left() - scroll_x;
        let selection = (!field.selection.is_empty()).then(|| {
            fill(
                Bounds::from_corners(
                    point(left + line.x_for_index(field.selection.start), bounds.top()),
                    point(
                        left + line.x_for_index(field.selection.end),
                        bounds.bottom(),
                    ),
                ),
                if focused {
                    colors.selection
                } else {
                    colors.selection_inactive
                },
            )
        });
        // Hidden while the window is in the background, as the editor's
        // caret is (the blink is stopped then too).
        let caret = (focused
            && window.is_window_active()
            && field.selection.is_empty()
            && field.caret_visible()
            && !field.is_disabled())
        .then(|| {
            // Slightly shorter than the line box, centred, like AppKit.
            let inset = (bounds.size.height * 0.08).round();
            fill(
                Bounds::new(
                    point(left + caret_x - px(0.5), bounds.top() + inset),
                    size(CARET_WIDTH, bounds.size.height - inset * 2.),
                ),
                colors.caret,
            )
        });

        PrepaintState {
            line,
            placeholder,
            selection,
            caret,
            scroll_x,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        prepaint: &mut PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.field.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.field.clone()),
            cx,
        );

        let line_height = window.line_height();
        let origin = point(bounds.left() - prepaint.scroll_x, bounds.top());
        // Let the caret and the selection edge show at the very right.
        let mask = Bounds::new(
            bounds.origin,
            size(bounds.size.width + CARET_WIDTH, bounds.size.height),
        );
        window.with_content_mask(Some(ContentMask { bounds: mask }), |window| {
            if let Some(selection) = prepaint.selection.take() {
                window.paint_quad(selection);
            }
            if let Some(placeholder) = prepaint.placeholder.as_ref() {
                placeholder
                    .paint(bounds.origin, line_height, window, cx)
                    .ok();
            } else {
                prepaint.line.paint(origin, line_height, window, cx).ok();
            }
            if let Some(caret) = prepaint.caret.take() {
                window.paint_quad(caret);
            }
        });

        // Drag selection continues outside the field until the button is up.
        if self.field.read(cx).selecting {
            let field = self.field.clone();
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                if phase == DispatchPhase::Bubble {
                    if event.dragging() {
                        field.update(cx, |field, cx| field.drag_to(event.position, cx));
                    } else {
                        field.update(cx, |field, cx| field.end_drag(cx));
                    }
                }
            });
            let field = self.field.clone();
            window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                if phase == DispatchPhase::Bubble {
                    field.update(cx, |field, cx| field.end_drag(cx));
                }
            });
        }

        let line = prepaint.line.clone();
        let scroll_x = prepaint.scroll_x;
        self.field.update(cx, |field, _| {
            field.last_layout = Some(line);
            field.last_bounds = Some(bounds);
            field.scroll_x = scroll_x;
        });
    }
}
