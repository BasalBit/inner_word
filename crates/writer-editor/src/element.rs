//! The custom element that lays out, paints and wires up an [`Editor`].
//!
//! - `request_layout`: fills its container (full mode) or measures its
//!   content height for the offered width (compact mode).
//! - `prepaint`: installs the frame's layout environment, lays out the
//!   document (cached per block), applies pending scroll requests and
//!   builds a [`FramePlan`]: everything to paint, as owned data, for the
//!   visible blocks only.
//! - `paint`: registers the platform input handler and the mouse listeners,
//!   then paints the plan back to front: canvas, block decorations, comment
//!   tints and highlight layers, text-run backgrounds, selection, glyphs,
//!   markers, caret.
//!
//! Painting details per block kind are in `decorations`.
//!
//! Status: skeleton for rendering polish. Not done: page view sheets,
//! justified text, caret blinking, marked-text underline, a scroll bar,
//! virtualised layout. The flow column, wrapping, inline attributes, block
//! kinds, selection, caret, highlights and comment tints are real.

use gpui::{
    App, AvailableSpace, Bounds, ContentMask, CursorStyle, DispatchPhase, Element, ElementId,
    ElementInputHandler, Entity, GlobalElementId, Hitbox, HitboxBehavior, Hsla, InspectorElementId,
    IntoElement, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad,
    Pixels, Point, ScrollWheelEvent, ShapedLine, SharedString, Style, TextAlign, TextRun, Window,
    WrappedLine, fill, point, px, relative, size,
};
use writer_ui::theme::{self, ActiveTheme};

use crate::{
    config::{CaretStyle, EditorMode},
    decorations,
    editor::Editor,
    geometry::{caret_rect, shifted},
    layout::{LayoutEnv, LayoutStyle},
};

/// Narrowest text column of a full editor.
const MIN_COLUMN_WIDTH: Pixels = px(120.);
/// Width assumed for a compact editor when the layout offers none.
const FALLBACK_COMPACT_WIDTH: Pixels = px(240.);

pub(crate) struct EditorElement {
    editor: Entity<Editor>,
}

impl EditorElement {
    pub(crate) fn new(editor: Entity<Editor>) -> Self {
        Self { editor }
    }
}

/// A hard line of text ready to paint.
pub(crate) struct PlannedLine {
    pub(crate) line: WrappedLine,
    pub(crate) origin: Point<Pixels>,
    pub(crate) line_height: Pixels,
    pub(crate) align: TextAlign,
    /// The text box: its width is what alignment is computed against.
    pub(crate) bounds: Bounds<Pixels>,
}

/// A single shaped line outside the document text: list markers, labels,
/// the placeholder.
pub(crate) struct PlannedMarker {
    pub(crate) line: ShapedLine,
    pub(crate) origin: Point<Pixels>,
    pub(crate) line_height: Pixels,
}

/// Everything one frame paints, back to front.
pub(crate) struct FramePlan {
    pub(crate) hitbox: Hitbox,
    /// Fill of the whole element (the canvas, in full mode).
    pub(crate) background: Option<Hsla>,
    /// Block decorations, comment tints, highlight layers.
    pub(crate) back: Vec<PaintQuad>,
    pub(crate) lines: Vec<PlannedLine>,
    /// Selection rectangles: over the run backgrounds, under the glyphs.
    pub(crate) selection: Vec<PaintQuad>,
    pub(crate) markers: Vec<PlannedMarker>,
    /// The caret and anything else over the text.
    pub(crate) front: Vec<PaintQuad>,
}

impl Editor {
    /// Installs the layout environment for a viewport and lays the document
    /// out. Called from prepaint with the element's bounds and from the
    /// compact measure pass with the offered width.
    pub(crate) fn install_layout_env(
        &mut self,
        viewport: Option<Bounds<Pixels>>,
        width: Pixels,
        window: &mut Window,
        cx: &App,
    ) {
        let theme = cx.theme();
        let compact = self.config.mode.is_compact();
        let viewport = viewport.unwrap_or_else(|| {
            // Measuring: keep the last known origin, assume the tallest
            // allowed viewport.
            let origin = self
                .layout
                .borrow()
                .env()
                .map(|env| env.viewport.origin)
                .unwrap_or_default();
            let height = match self.config.mode {
                EditorMode::Compact(options) => options.max_height,
                EditorMode::Full => px(0.),
            };
            Bounds::new(origin, size(width, height))
        });
        let style = LayoutStyle::new(theme, &self.state.document().settings().typography, compact);
        let (column_left, column_width, pad_top, pad_bottom) = if compact {
            (viewport.left(), viewport.size.width, px(0.), px(0.))
        } else {
            let column_width = theme::DOCUMENT_COLUMN_WIDTH
                .min(viewport.size.width - theme::SPACE_8 * 2.)
                .max(MIN_COLUMN_WIDTH);
            let inset = ((viewport.size.width - column_width) / 2.)
                .max(px(0.))
                .round();
            (
                viewport.left() + inset,
                column_width,
                theme::SPACE_12,
                // Room to scroll the last lines up to a comfortable height.
                (viewport.size.height * 0.4).round(),
            )
        };
        let mut layout = self.layout.borrow_mut();
        layout.set_env(LayoutEnv {
            text_system: window.text_system().clone(),
            style,
            viewport,
            column_left,
            column_width,
            pad_top,
            pad_bottom,
        });
        layout.sync(self.state.document(), self.state.revision());
    }

    /// Builds the paint plan for the blocks in the viewport.
    pub(crate) fn plan_frame(&self, hitbox: Hitbox, window: &mut Window) -> FramePlan {
        let focused = self.focus_handle.is_focused(window);
        let compact = self.config.mode.is_compact();
        let mut plan = FramePlan {
            hitbox,
            background: None,
            back: Vec::new(),
            lines: Vec::new(),
            selection: Vec::new(),
            markers: Vec::new(),
            front: Vec::new(),
        };
        let document = self.state.document();
        let mut layout = self.layout.borrow_mut();
        if !layout.sync(document, self.state.revision()) {
            return plan;
        }
        let layout = &*layout;
        let Some(env) = layout.env() else {
            return plan;
        };
        let colors = env.style.colors;
        if !compact {
            plan.background = Some(colors.canvas);
        }

        let selection = self.state.selection();
        let selected = (!selection.is_collapsed()).then(|| selection.range());
        let selection_color = if focused {
            colors.selection
        } else {
            colors.selection_inactive
        };

        // The blocks that intersect the viewport.
        let first = layout.block_at_y(self.scroll_y);
        let bottom = self.scroll_y + env.viewport.size.height;
        let mut last = first;
        while last < layout.block_count() && layout.block_top(last) < bottom {
            last += 1;
        }
        let ordinals = document.numbered_ordinals(first..last);

        for index in first..last {
            let (Some(block), Some(block_layout)) = (document.block(index), layout.block(index))
            else {
                continue;
            };
            let origin = self.block_origin(layout, env, index);
            let ordinal = ordinals.get(index - first).copied().flatten();
            decorations::plan_block(&mut plan, block, block_layout, origin, ordinal, env, window);

            // Comment anchors.
            if self.config.comments_visible {
                for (range, attrs) in block.spans() {
                    if attrs.comments.is_empty() {
                        continue;
                    }
                    let active = self
                        .active_comment
                        .is_some_and(|id| attrs.comments.contains(&id));
                    let color = if active {
                        colors.comment_active
                    } else {
                        colors.comment
                    };
                    for rect in block_layout.rects_for_range(range) {
                        plan.back.push(fill(shifted(rect, origin), color));
                    }
                }
            }

            // Highlight layers, in layer order.
            for layer in self.highlights.layers() {
                for range in layer.ranges_in_block(index) {
                    let Some(local) = document.range_in_block(*range, index) else {
                        continue;
                    };
                    for rect in block_layout.rects_for_range(local) {
                        let rect = shifted(rect, origin);
                        if let Some(background) = layer.style.background {
                            plan.back.push(fill(rect, background));
                        }
                        if let Some(underline) = layer.style.underline {
                            plan.back.push(fill(
                                Bounds::new(
                                    point(rect.left(), rect.bottom() - px(2.)),
                                    size(rect.size.width, px(1.)),
                                ),
                                underline,
                            ));
                        }
                    }
                }
            }

            // Selection.
            if let Some(range) = selected
                && let Some(local) = document.range_in_block(range, index)
            {
                // A selection that continues into the next block covers
                // this block's final line break.
                let end = if index < range.end.block {
                    local.end + 1
                } else {
                    local.end
                };
                if block.is_void() || local.start < end {
                    for rect in block_layout.rects_for_range(local.start..end) {
                        plan.selection
                            .push(fill(shifted(rect, origin), selection_color));
                    }
                }
            }

            // Text.
            let align = block_layout.text_align();
            let mut row_ix = 0;
            for (line_ix, line) in block_layout.lines.iter().enumerate() {
                // The first row of this hard line gives its top.
                while row_ix < block_layout.rows.len() && block_layout.rows[row_ix].line < line_ix {
                    row_ix += 1;
                }
                let Some(row) = block_layout.rows.get(row_ix) else {
                    break;
                };
                let line_origin = point(origin.x + block_layout.text_left, origin.y + row.y);
                plan.lines.push(PlannedLine {
                    line: line.clone(),
                    origin: line_origin,
                    line_height: block_layout.line_height,
                    align,
                    bounds: Bounds::new(
                        line_origin,
                        size(block_layout.text_width, block_layout.line_height),
                    ),
                });
            }

            // Caret: shown for a collapsed selection, and always for the
            // block and underline styles (Vim keeps its cursor in visual
            // mode).
            let head = selection.head;
            if focused
                && head.block == index
                && (selection.is_collapsed() || self.config.caret_style != CaretStyle::Bar)
            {
                let rect = caret_rect(
                    block_layout,
                    block.text(),
                    head.offset,
                    self.config.caret_style,
                );
                let color = match self.config.caret_style {
                    // A block caret lets the character show through.
                    CaretStyle::Block => Hsla {
                        a: 0.45,
                        ..colors.caret
                    },
                    _ => colors.caret,
                };
                plan.front.push(fill(shifted(rect, origin), color));
            }
        }

        // Placeholder over an empty document.
        if self.is_empty()
            && !self.config.placeholder.is_empty()
            && let Some(block_layout) = layout.block(0)
        {
            let text: SharedString = self.config.placeholder.clone();
            let run = TextRun {
                len: text.len(),
                font: env.style.body_font.clone(),
                color: colors.faint,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let origin = self.block_origin(layout, env, 0);
            plan.markers.push(PlannedMarker {
                line: window
                    .text_system()
                    .shape_line(text, block_layout.font_size, &[run], None),
                origin: point(
                    origin.x + block_layout.text_left,
                    origin.y + block_layout.rows[0].y,
                ),
                line_height: block_layout.line_height,
            });
        }
        plan
    }
}

impl IntoElement for EditorElement {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for EditorElement {
    type RequestLayoutState = ();
    type PrepaintState = FramePlan;

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
        if !self.editor.read(cx).config.mode.is_compact() {
            style.size.height = relative(1.).into();
            return (window.request_layout(style, [], cx), ());
        }
        // Compact: the height follows the content at the offered width.
        let editor = self.editor.clone();
        let layout_id =
            window.request_measured_layout(style, move |known, available, window, cx| {
                let width = known
                    .width
                    .or(match available.width {
                        AvailableSpace::Definite(width) => Some(width),
                        _ => None,
                    })
                    .unwrap_or(FALLBACK_COMPACT_WIDTH);
                let height = editor.update(cx, |editor, cx| {
                    editor.measure_compact_height(width, window, cx)
                });
                size(width, height)
            });
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> FramePlan {
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        self.editor.update(cx, |editor, cx| {
            editor.install_layout_env(Some(bounds), bounds.size.width, window, cx);
            editor.flush_scroll();
            editor.clamp_scroll();
            editor.plan_frame(hitbox, window)
        })
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        plan: &mut FramePlan,
        window: &mut Window,
        cx: &mut App,
    ) {
        let (focus_handle, accepts_text, row_height) = {
            let editor = self.editor.read(cx);
            (
                editor.focus_handle.clone(),
                editor.text_input_enabled && !editor.config.read_only,
                editor
                    .layout
                    .borrow()
                    .env()
                    .map_or(px(20.), |env| env.style.base_line_height()),
            )
        };
        // Platform text input (typing, IME). Skipped while the editor
        // refuses text: nothing typed reaches the document then.
        if accepts_text {
            window.handle_input(
                &focus_handle,
                ElementInputHandler::new(bounds, self.editor.clone()),
                cx,
            );
        }
        window.set_cursor_style(CursorStyle::IBeam, &plan.hitbox);
        self.register_mouse(&plan.hitbox, row_height, window);

        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            if let Some(background) = plan.background {
                window.paint_quad(fill(bounds, background));
            }
            for quad in plan.back.drain(..) {
                window.paint_quad(quad);
            }
            for line in &plan.lines {
                line.line
                    .paint_background(
                        line.origin,
                        line.line_height,
                        line.align,
                        Some(line.bounds),
                        window,
                        cx,
                    )
                    .ok();
            }
            for quad in plan.selection.drain(..) {
                window.paint_quad(quad);
            }
            for line in &plan.lines {
                line.line
                    .paint(
                        line.origin,
                        line.line_height,
                        line.align,
                        Some(line.bounds),
                        window,
                        cx,
                    )
                    .ok();
            }
            for marker in &plan.markers {
                marker
                    .line
                    .paint(marker.origin, marker.line_height, window, cx)
                    .ok();
            }
            for quad in plan.front.drain(..) {
                window.paint_quad(quad);
            }
        });
    }
}

impl EditorElement {
    /// Mouse listeners live for one frame; they are registered on the
    /// window, so a drag keeps working outside the editor's bounds.
    fn register_mouse(&self, hitbox: &Hitbox, row_height: Pixels, window: &mut Window) {
        let editor = self.editor.clone();
        let down_hitbox = hitbox.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble
                && event.button == MouseButton::Left
                && down_hitbox.is_hovered(window)
            {
                editor.update(cx, |editor, cx| editor.mouse_down(event, window, cx));
            }
        });

        let editor = self.editor.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
            if phase == DispatchPhase::Bubble && editor.read(cx).selecting {
                if event.dragging() {
                    editor.update(cx, |editor, cx| editor.mouse_dragged(event, cx));
                } else {
                    editor.update(cx, |editor, cx| editor.mouse_up(cx));
                }
            }
        });

        let editor = self.editor.clone();
        window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
            if phase == DispatchPhase::Bubble && editor.read(cx).selecting {
                editor.update(cx, |editor, cx| editor.mouse_up(cx));
            }
        });

        let editor = self.editor.clone();
        let scroll_hitbox = hitbox.clone();
        window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble && scroll_hitbox.should_handle_scroll(window) {
                let delta = event.delta.pixel_delta(row_height);
                editor.update(cx, |editor, cx| editor.scroll_wheel(delta.y, cx));
            }
        });
    }
}
