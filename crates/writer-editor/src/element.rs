//! The custom element that lays out, paints and wires up an [`Editor`].
//!
//! - `request_layout`: fills its container (full mode) or measures its
//!   content height for the offered width (compact mode).
//! - `prepaint`: installs the frame's layout environment, shapes the blocks
//!   in the viewport (and only those), applies pending scroll requests and
//!   builds a [`FramePlan`]: everything to paint, as owned data.
//! - `paint`: registers the platform input handler and the mouse listeners,
//!   then paints the plan back to front: desk or canvas, sheets, block
//!   decorations, comment tints and highlight layers, run backgrounds,
//!   selection, glyphs, markers, caret, scroll bar.
//!
//! Per block kind details are in `decorations`; rows of text are painted by
//! `paint`.

use std::time::Instant;

use gpui::{
    App, AvailableSpace, Bounds, BoxShadow, ContentMask, Context, Corners, CursorStyle,
    DispatchPhase, Element, ElementId, ElementInputHandler, Entity, GlobalElementId, Hitbox,
    HitboxBehavior, Hsla, InspectorElementId, IntoElement, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, PaintQuad, Path, Pixels, Point, ScrollWheelEvent, ShapedLine,
    Style, Window, fill, point, px, relative, size,
};
use writer_core::BlockKind;
use writer_ui::theme::{self, ActiveTheme};

use crate::{
    config::{CaretStyle, EditorMode},
    decorations::{self, BlockContext, CheckboxHit},
    editor::Editor,
    geometry::{caret_rect, drawn_caret, shifted},
    layout::{Lay, LayoutEnv, LayoutStyle, PageGeometry, pages::DESK_PADDING},
    paint::{PlannedRow, snap},
    scroll::ScrollbarGeometry,
};

/// Narrowest text column of a full editor.
const MIN_COLUMN_WIDTH: Pixels = px(120.);
/// Width assumed for a compact editor when the layout offers none.
const FALLBACK_COMPACT_WIDTH: Pixels = px(240.);
/// Width of the scroll bar thumb and its distance from the edge.
const SCROLLBAR_WIDTH: Pixels = px(6.);
const SCROLLBAR_INSET: Pixels = px(3.);
/// Width of the strip at the right edge that belongs to the scroll bar.
pub(crate) const SCROLLBAR_ZONE: Pixels = px(14.);
const SCROLLBAR_MIN_THUMB: Pixels = px(28.);

pub(crate) struct EditorElement {
    editor: Entity<Editor>,
}

impl EditorElement {
    pub(crate) fn new(editor: Entity<Editor>) -> Self {
        Self { editor }
    }
}

/// A single shaped line outside the document text: list numbers, labels,
/// the placeholder, page numbers.
pub(crate) struct PlannedMarker {
    pub(crate) line: ShapedLine,
    pub(crate) origin: Point<Pixels>,
    pub(crate) line_height: Pixels,
}

/// Everything one frame paints, back to front.
pub(crate) struct FramePlan {
    pub(crate) hitbox: Hitbox,
    /// Fill of the whole element: the canvas, or the desk in page view.
    pub(crate) background: Option<Hsla>,
    /// Page view: the sheets on screen.
    pub(crate) sheets: Vec<Bounds<Pixels>>,
    pub(crate) sheet_color: Hsla,
    pub(crate) sheet_shadows: Vec<BoxShadow>,
    /// Block decorations, comment tints, highlight layers.
    pub(crate) back: Vec<PaintQuad>,
    pub(crate) rows: Vec<PlannedRow>,
    /// Selection rectangles: over the run backgrounds, under the glyphs.
    pub(crate) selection: Vec<PaintQuad>,
    pub(crate) markers: Vec<PlannedMarker>,
    /// Stroked shapes over the decorations (checkbox ticks).
    pub(crate) paths: Vec<(Path<Pixels>, Hsla)>,
    /// The caret and anything else over the text.
    pub(crate) front: Vec<PaintQuad>,
    /// Page view: the sheets (the text cursor shows over them, the arrow
    /// over the desk).
    pub(crate) sheet_hitboxes: Vec<Hitbox>,
    /// Areas with a pointing-hand cursor (checkboxes).
    pub(crate) pointer_hitboxes: Vec<Hitbox>,
    /// The scroll bar strip (arrow cursor).
    pub(crate) scrollbar_hitbox: Option<Hitbox>,
    /// The pointer is over a link with `cmd` held.
    pub(crate) link_cursor: bool,
    /// Line height of body text, for wheel events that scroll by lines.
    pub(crate) row_height: Pixels,
}

impl FramePlan {
    fn new(hitbox: Hitbox) -> Self {
        Self {
            hitbox,
            background: None,
            sheets: Vec::new(),
            sheet_color: gpui::transparent_black(),
            sheet_shadows: Vec::new(),
            back: Vec::new(),
            rows: Vec::new(),
            selection: Vec::new(),
            markers: Vec::new(),
            paths: Vec::new(),
            front: Vec::new(),
            sheet_hitboxes: Vec::new(),
            pointer_hitboxes: Vec::new(),
            scrollbar_hitbox: None,
            link_cursor: false,
            row_height: px(20.),
        }
    }
}

impl Editor {
    /// Installs the layout environment for a viewport. Called from prepaint
    /// with the element's bounds and from the compact measure pass with the
    /// offered width. Nothing is shaped here.
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
        let document = self.state.document();
        let settings = document.settings();
        let paged = settings.page_view && !compact;
        let env = if paged {
            let geometry = PageGeometry::new(&settings.page, viewport);
            LayoutEnv {
                text_system: window.text_system().clone(),
                style: LayoutStyle::new(theme, &settings.typography, false, true, geometry.scale),
                viewport,
                column_left: geometry.sheet_left + geometry.margin_left,
                column_width: geometry.content_width,
                pad_top: px(DESK_PADDING),
                pad_bottom: px(DESK_PADDING),
                page: Some(geometry),
            }
        } else if compact {
            LayoutEnv {
                text_system: window.text_system().clone(),
                style: LayoutStyle::new(theme, &settings.typography, true, false, 1.),
                viewport,
                column_left: viewport.left(),
                column_width: viewport.size.width,
                pad_top: px(0.),
                pad_bottom: px(0.),
                page: None,
            }
        } else {
            let column_width = theme::DOCUMENT_COLUMN_WIDTH
                .min(viewport.size.width - theme::SPACE_8 * 2.)
                .max(MIN_COLUMN_WIDTH);
            let inset = ((viewport.size.width - column_width) / 2.)
                .max(px(0.))
                .round();
            LayoutEnv {
                text_system: window.text_system().clone(),
                style: LayoutStyle::new(theme, &settings.typography, false, false, 1.),
                viewport,
                column_left: viewport.left() + inset,
                column_width,
                pad_top: theme::SPACE_12,
                // Room to scroll the last lines up to a comfortable height.
                pad_bottom: (viewport.size.height * 0.4).round(),
                page: None,
            }
        };
        let mut layout = self.layout.borrow_mut();
        if layout.revision() != self.state.revision() || layout.count() != document.block_count() {
            layout.rebuild(document, self.state.revision());
        }
        layout.set_env(env, document);
    }

    /// Builds the paint plan for the blocks in the viewport.
    pub(crate) fn plan_frame(&mut self, hitbox: Hitbox, window: &mut Window) -> FramePlan {
        let focused = self.focus_handle.is_focused(window);
        let active = window.is_window_active() || self.draw_as_active;
        let compact = self.config.mode.is_compact();
        let mut plan = FramePlan::new(hitbox);
        let mut checkboxes: Vec<CheckboxHit> = Vec::new();
        let mut scrollbar = None;
        let mut caret_painted = false;
        let document = self.state.document();
        let selection = self.state.selection();
        let caret_position = self.caret_position();
        let caret_upstream = self.caret_is_upstream();

        {
            let mut guard = self.layout.borrow_mut();
            let layout = &mut *guard;
            let Some(viewport) = layout.env().map(|env| env.viewport) else {
                return plan;
            };
            layout.begin_frame();
            let visible = layout.stabilize(document, viewport.size.height);
            let mut lay = Lay { layout, document };
            let scroll_y = lay.scroll_y();
            let origin = lay.origin();
            let (colors, surface, paged) = {
                let env = lay.env();
                plan.row_height = env.style.base_line_height();
                (env.style.colors, env.style.surface(), env.page.is_some())
            };

            // Canvas, or the desk with its sheets.
            if paged {
                plan.background = Some(colors.desk);
                let pages = lay.layout.page_count();
                let env = lay.env();
                if let Some(geometry) = &env.page {
                    plan.sheet_color = colors.page;
                    plan.sheet_shadows = page_shadows(colors.shadow, colors.appearance.is_dark());
                    let label_size = env.style.px(f32::from(theme::UI_TEXT_XS)).max(px(8.));
                    let label_height = (label_size * 1.4).round();
                    for page in geometry.page_at(scroll_y, env.pad_top)..pages {
                        let top = viewport.top() + geometry.sheet_top(page, env.pad_top) - scroll_y;
                        if top >= viewport.bottom() {
                            break;
                        }
                        let sheet = Bounds::new(
                            point(geometry.sheet_left, top),
                            size(geometry.sheet_width, geometry.sheet_height),
                        );
                        plan.sheets.push(sheet);
                        let visible = sheet.intersect(&viewport);
                        if visible.size.width > px(0.) && visible.size.height > px(0.) {
                            plan.sheet_hitboxes
                                .push(window.insert_hitbox(visible, HitboxBehavior::Normal));
                        }
                        // The page number, centred in the bottom margin.
                        if geometry.margin_bottom >= label_height * 1.5 {
                            let label = decorations::shape_label(
                                window,
                                (page + 1).to_string().into(),
                                env.style.ui_font.clone(),
                                label_size,
                                colors.faint,
                            );
                            plan.markers.push(PlannedMarker {
                                origin: point(
                                    (sheet.left() + (sheet.size.width - label.width) / 2.).round(),
                                    (sheet.bottom() - (geometry.margin_bottom + label_height) / 2.)
                                        .round(),
                                ),
                                line_height: label_height,
                                line: label,
                            });
                        }
                    }
                }
            } else if !compact {
                plan.background = Some(colors.canvas);
            }

            let selected = (!selection.is_collapsed()).then(|| selection.range());
            let selection_color = if focused && active {
                colors.selection
            } else {
                colors.selection_inactive
            };
            let ordinals = document.numbered_ordinals(visible.clone());
            let view_top = scroll_y;
            let view_bottom = scroll_y + viewport.size.height;

            for index in visible.clone() {
                let Some(block) = document.block(index) else {
                    continue;
                };
                let Some(placed) = lay.placed(index) else {
                    continue;
                };
                let env = lay.env();
                let block_layout = &placed.layout;
                // Only the rows in the viewport are planned: a long block
                // costs what is visible of it.
                let rows = placed.row_at(view_top)..placed.row_at(view_bottom) + 1;

                decorations::plan_block(
                    &mut plan,
                    &mut checkboxes,
                    &BlockContext {
                        index,
                        block,
                        placed: &placed,
                        origin,
                        ordinal: ordinals.get(index - visible.start).copied().flatten(),
                        quote_continues: matches!(block.kind(), BlockKind::Quote)
                            && document
                                .block(index + 1)
                                .is_some_and(|next| matches!(next.kind(), BlockKind::Quote)),
                        env,
                    },
                    window,
                );

                // Comment anchors.
                if self.config.comments_visible && !compact {
                    for (range, attrs) in block.spans() {
                        // Threads the consumer hides (resolved ones) do
                        // not tint.
                        if attrs
                            .comments
                            .iter()
                            .all(|id| self.hidden_comments.contains(id))
                        {
                            continue;
                        }
                        let active_thread = self.active_comment.is_some_and(|id| {
                            attrs.comments.contains(&id) && !self.hidden_comments.contains(&id)
                        });
                        let color = if active_thread {
                            colors.comment_active
                        } else {
                            colors.comment
                        };
                        for rect in placed.ink_rects(range, rows.clone()) {
                            let rect = shifted(rect, origin);
                            plan.back.push(fill(rect, color).corner_radii(px(2.)));
                            if active_thread {
                                plan.back.push(fill(
                                    Bounds::new(
                                        point(rect.left(), snap(rect.bottom() - px(1.5))),
                                        size(rect.size.width, px(1.5)),
                                    ),
                                    colors.comment_underline,
                                ));
                            }
                        }
                    }
                }

                // Highlight layers, in layer order.
                for layer in self.highlights.layers() {
                    for range in layer.ranges_in_block(index) {
                        let Some(local) = document.range_in_block(*range, index) else {
                            continue;
                        };
                        for rect in placed.ink_rects(local, rows.clone()) {
                            let rect = shifted(rect, origin);
                            if let Some(background) = layer.style.background {
                                plan.back.push(fill(rect, background).corner_radii(px(2.)));
                            }
                            if let Some(underline) = layer.style.underline {
                                plan.back.push(fill(
                                    Bounds::new(
                                        point(rect.left(), snap(rect.bottom() - px(1.))),
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
                        for rect in placed.rects(local.start..end, rows.clone()) {
                            plan.selection
                                .push(fill(shifted(rect, origin), selection_color));
                        }
                    }
                }

                // Text of an IME composition: underlined, and underlined
                // twice as thick where the input method has a clause
                // selected.
                if let Some(marked) = &self.marked
                    && marked.block == index
                {
                    let thin = px((f32::from(block_layout.font_size) / 16.).round().max(1.));
                    let clause = selected
                        .and_then(|range| document.range_in_block(range, index))
                        .map(|local| {
                            local.start.max(marked.range.start)..local.end.min(marked.range.end)
                        })
                        .filter(|clause| clause.start < clause.end);
                    let mut underline = |range: std::ops::Range<usize>, thickness: Pixels| {
                        for rect in placed.rects(range, rows.clone()) {
                            let rect = shifted(rect, origin);
                            let y = rect.top() + block_layout.baseline + block_layout.descent * 0.6;
                            plan.front.push(fill(
                                Bounds::new(
                                    point(rect.left(), snap(y)),
                                    size(rect.size.width, thickness),
                                ),
                                colors.ink,
                            ));
                        }
                    };
                    underline(marked.range.clone(), thin);
                    if let Some(clause) = clause {
                        underline(clause, thin * 2.);
                    }
                }

                // Text.
                if !block_layout.void {
                    for row in rows.clone() {
                        plan.rows.push(PlannedRow {
                            layout: placed.layout.clone(),
                            row,
                            origin: point(
                                origin.x + block_layout.text_left,
                                origin.y + placed.row_top(row),
                            ),
                        });
                    }
                }

                // Caret: shown for a collapsed selection, always for the
                // block and underline styles (Vim keeps its cursor in visual
                // mode), and always where a caret override puts it. Hidden
                // in the off phase of its blink and while the window is in
                // the background. It is painted after the selection and the
                // text, so a selection tint never covers it.
                let head = caret_position;
                let style = self.config.caret_style;
                if focused
                    && active
                    && self.blink_on
                    && head.block == index
                    && (selection.is_collapsed()
                        || style != CaretStyle::Bar
                        || self.caret_override.is_some())
                {
                    caret_painted = true;
                    let rect =
                        caret_rect(&placed, block.text(), head.offset, style, caret_upstream);
                    let color = match style {
                        // A block caret lets the character show through.
                        CaretStyle::Block => colors.caret.alpha(0.45),
                        _ => colors.caret,
                    };
                    plan.front.push(
                        fill(
                            shifted(drawn_caret(rect, block_layout, style), origin),
                            color,
                        )
                        .corner_radii(px(1.)),
                    );
                }
            }

            // Placeholder over an empty document.
            if document.block_count() == 1
                && document.blocks()[0].is_empty()
                && !document.blocks()[0].is_void()
                && !self.config.placeholder.is_empty()
                && let Some(placed) = lay.placed(0)
            {
                let env = lay.env();
                let line = decorations::shape_label(
                    window,
                    self.config.placeholder.clone(),
                    env.style.body_font.clone(),
                    placed.layout.font_size,
                    colors.faint,
                );
                plan.markers.push(PlannedMarker {
                    line,
                    origin: point(
                        origin.x + placed.layout.text_left,
                        origin.y + placed.row_top(0),
                    ),
                    line_height: placed.layout.line_height,
                });
            }
            let _ = surface;

            // The overlay scroll bar.
            let max_scroll = lay.layout.max_scroll();
            if max_scroll > px(0.) {
                let track = Bounds::new(
                    point(
                        viewport.right() - SCROLLBAR_INSET - SCROLLBAR_WIDTH,
                        viewport.top() + SCROLLBAR_INSET,
                    ),
                    size(
                        SCROLLBAR_WIDTH,
                        (viewport.size.height - SCROLLBAR_INSET * 2.).max(px(0.)),
                    ),
                );
                let content = max_scroll + viewport.size.height;
                let thumb_height = (track.size.height * (viewport.size.height / content))
                    .max(SCROLLBAR_MIN_THUMB.min(track.size.height));
                let travel = track.size.height - thumb_height;
                let thumb = Bounds::new(
                    point(track.left(), track.top() + travel * (scroll_y / max_scroll)),
                    size(SCROLLBAR_WIDTH, thumb_height),
                );
                scrollbar = Some(ScrollbarGeometry {
                    track,
                    thumb,
                    max_scroll,
                });
                if self.scrollbar.is_shown() {
                    let strength = if self.scrollbar.is_active() {
                        0.5
                    } else {
                        0.28
                    };
                    plan.front.push(
                        fill(thumb, colors.ink.alpha(strength)).corner_radii(SCROLLBAR_WIDTH / 2.),
                    );
                }
                if !compact {
                    plan.scrollbar_hitbox = Some(window.insert_hitbox(
                        Bounds::new(
                            point(viewport.right() - SCROLLBAR_ZONE, viewport.top()),
                            size(SCROLLBAR_ZONE, viewport.size.height),
                        ),
                        HitboxBehavior::Normal,
                    ));
                }
            }
        }

        for checkbox in &checkboxes {
            plan.pointer_hitboxes
                .push(window.insert_hitbox(checkbox.bounds, HitboxBehavior::Normal));
        }
        plan.link_cursor = self.link_hover;
        self.checkbox_hits = checkboxes;
        self.scrollbar.geometry = scrollbar;
        self.caret_painted = caret_painted;
        plan
    }
}

/// The soft shadow under a sheet of paper.
fn page_shadows(color: Hsla, dark: bool) -> Vec<BoxShadow> {
    let strength = if dark { 2. } else { 1. };
    vec![
        BoxShadow {
            color: color.alpha(0.05 * strength),
            offset: point(px(0.), px(1.)),
            blur_radius: px(2.),
            spread_radius: px(0.),
        },
        BoxShadow {
            color: color.alpha(0.07 * strength),
            offset: point(px(0.), px(6.)),
            blur_radius: px(18.),
            spread_radius: px(0.),
        },
    ]
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
            let started = Instant::now();
            editor.install_layout_env(Some(bounds), bounds.size.width, window, cx);
            editor.settle_scroll();
            let plan = editor.plan_frame(hitbox, window);
            editor.after_frame(started.elapsed(), cx);
            plan
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
        let started = Instant::now();
        let (focus_handle, accepts_text) = {
            let editor = self.editor.read(cx);
            (
                editor.focus_handle.clone(),
                editor.text_input_enabled && !editor.config.read_only,
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
        // Later requests win where hitboxes overlap. The text cursor shows
        // over text surfaces: the whole editor, or the sheets in page view
        // (the desk around them has the arrow).
        let text_cursor = if plan.link_cursor {
            CursorStyle::PointingHand
        } else {
            CursorStyle::IBeam
        };
        if plan.background.is_some() && !plan.sheet_hitboxes.is_empty() {
            window.set_cursor_style(CursorStyle::Arrow, &plan.hitbox);
            for hitbox in &plan.sheet_hitboxes {
                window.set_cursor_style(text_cursor, hitbox);
            }
        } else {
            window.set_cursor_style(text_cursor, &plan.hitbox);
        }
        for hitbox in &plan.pointer_hitboxes {
            window.set_cursor_style(CursorStyle::PointingHand, hitbox);
        }
        if let Some(hitbox) = &plan.scrollbar_hitbox {
            window.set_cursor_style(CursorStyle::Arrow, hitbox);
        }
        self.register_mouse(&plan.hitbox, plan.row_height, window);

        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            if let Some(background) = plan.background {
                window.paint_quad(fill(bounds, background));
            }
            for sheet in &plan.sheets {
                window.paint_shadows(
                    *sheet,
                    Corners::all(theme::RADIUS_PAGE),
                    &plan.sheet_shadows,
                );
                window.paint_quad(fill(*sheet, plan.sheet_color).corner_radii(theme::RADIUS_PAGE));
            }
            for quad in plan.back.drain(..) {
                window.paint_quad(quad);
            }
            for row in &plan.rows {
                row.paint_backgrounds(window);
            }
            for quad in plan.selection.drain(..) {
                window.paint_quad(quad);
            }
            for row in &plan.rows {
                row.paint_text(window, cx);
            }
            for marker in &plan.markers {
                marker
                    .line
                    .paint(marker.origin, marker.line_height, window, cx)
                    .ok();
            }
            for (path, color) in plan.paths.drain(..) {
                window.paint_path(path, color);
            }
            for quad in plan.front.drain(..) {
                window.paint_quad(quad);
            }
        });
        self.editor.read(cx).perf.set(
            self.editor
                .read(cx)
                .perf
                .get()
                .with_paint(started.elapsed()),
        );
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
        let move_hitbox = hitbox.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            let hovered = move_hitbox.is_hovered(window);
            let busy = editor.read(cx).mouse_busy();
            if busy && !event.dragging() {
                // The button was released somewhere we did not see.
                editor.update(cx, |editor, cx| editor.mouse_up(cx));
            } else if busy {
                editor.update(cx, |editor, cx| editor.mouse_dragged(event, cx));
            } else if hovered || editor.read(cx).has_hover_state() {
                editor.update(cx, |editor, cx| editor.mouse_moved(event, hovered, cx));
            }
        });

        let editor = self.editor.clone();
        window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
            if phase == DispatchPhase::Bubble && editor.read(cx).mouse_busy() {
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

impl Editor {
    /// Bookkeeping at the end of a frame's prepaint.
    fn after_frame(&mut self, prepaint: std::time::Duration, cx: &mut Context<Self>) {
        self.perf.set(self.perf.get().with_prepaint(prepaint));
        if self.layout.borrow().pending() {
            self.start_refining(cx);
        }
    }
}
