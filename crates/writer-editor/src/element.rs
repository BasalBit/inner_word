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
    Style, UnderlineStyle, Window, fill, point, px, relative, size,
};
use writer_core::BlockKind;
use writer_ui::theme::{self, ActiveTheme};

use crate::{
    config::{CaretStyle, EditorMode},
    decorations::{self, BlockContext, CheckboxHit},
    editor::Editor,
    geometry::{caret_rect, drawn_caret},
    layout::{
        Lay, LayoutEnv, LayoutStyle, PageGeometry,
        pages::{DESK_PADDING, DESK_SIDE_PADDING},
    },
    paint::{PlannedRow, snap},
    scroll::{PendingScroll, ScrollbarGeometry},
    spelling::{self, PlannedWave},
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
    /// A hairline around each sheet, where a shadow cannot show its edge
    /// (Night: the desk is nearly black).
    pub(crate) sheet_border: Option<Hsla>,
    /// Block decorations, comment tints, highlight layers.
    pub(crate) back: Vec<PaintQuad>,
    pub(crate) rows: Vec<PlannedRow>,
    /// Selection rectangles: over the run backgrounds, under the glyphs.
    pub(crate) selection: Vec<PaintQuad>,
    pub(crate) markers: Vec<PlannedMarker>,
    /// The wavy underlines of misspelled words, in the text's layer.
    pub(crate) spelling: Vec<PlannedWave>,
    pub(crate) spelling_color: Hsla,
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
            sheet_border: None,
            back: Vec::new(),
            rows: Vec::new(),
            selection: Vec::new(),
            markers: Vec::new(),
            spelling: Vec::new(),
            spelling_color: gpui::transparent_black(),
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
    /// offered width. Nothing is shaped here. Returns the zoom of the view
    /// in percent (100 for a compact editor).
    pub(crate) fn install_layout_env(
        &mut self,
        viewport: Option<Bounds<Pixels>>,
        width: Pixels,
        window: &mut Window,
        cx: &App,
    ) -> f32 {
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
        let mut shown = 100.;
        let env = if paged {
            // The sheets are the page setup, in points; the zoom only
            // says how large a point is drawn.
            let geometry = PageGeometry::new(&settings.page);
            let side = px(DESK_SIDE_PADDING);
            let (zoom, percent) = self.zoom_for(
                true,
                size(
                    (viewport.size.width - side * 2.).max(px(1.)),
                    viewport.size.height.max(px(1.)),
                ),
                geometry.sheet_width,
                geometry.sheet_height + px(DESK_PADDING * 2.),
            );
            shown = percent;
            let sheet_width = geometry.sheet_width * zoom;
            let room = viewport.size.width;
            let (sheet_left, max_scroll_x, rest_scroll_x) =
                if sheet_width + side * 2. <= room + px(0.5) {
                    let inset = ((room - sheet_width) / 2.).max(px(0.)).round();
                    (viewport.left() + inset, px(0.), px(0.))
                } else {
                    // Wider than the room: the view scrolls sideways. It
                    // rests with the text column in the middle, or with
                    // the column's left edge in view when the column alone
                    // is too wide.
                    let max = (sheet_width + side * 2. - room) / zoom;
                    let column = geometry.content_width * zoom;
                    let rest = if column <= room - side * 2. {
                        (side + geometry.margin_left * zoom - (room - column) / 2.) / zoom
                    } else {
                        geometry.margin_left
                    };
                    (viewport.left() + side, max, rest.clamp(px(0.), max))
                };
            LayoutEnv {
                text_system: window.text_system().clone(),
                style: LayoutStyle::new(theme, &settings.typography, false, true),
                viewport,
                zoom,
                column_left: sheet_left + geometry.margin_left * zoom,
                column_width: geometry.content_width,
                pad_top: px(DESK_PADDING),
                pad_bottom: px(DESK_PADDING),
                page: Some(geometry),
                max_scroll_x,
                rest_scroll_x,
            }
        } else if compact {
            LayoutEnv {
                text_system: window.text_system().clone(),
                style: if self.config.mode.is_formatted_compact() {
                    LayoutStyle::formatted_compact(theme, &settings.typography)
                } else {
                    LayoutStyle::new(theme, &settings.typography, true, false)
                },
                viewport,
                zoom: 1.,
                column_left: viewport.left(),
                column_width: viewport.size.width,
                pad_top: px(0.),
                pad_bottom: px(0.),
                page: None,
                max_scroll_x: px(0.),
                rest_scroll_x: px(0.),
            }
        } else {
            // The room the column has, in window pixels; the column
            // itself is in points and grows with the zoom until it fills
            // the room. Beyond that it keeps the room's width and holds
            // fewer points: the text reflows, and nothing scrolls
            // sideways.
            let room = (viewport.size.width - theme::SPACE_8 * 2.).max(MIN_COLUMN_WIDTH);
            let (zoom, percent) = self.zoom_for(
                false,
                size(room, viewport.size.height),
                theme::DOCUMENT_COLUMN_WIDTH,
                px(0.),
            );
            shown = percent;
            let column_width = theme::DOCUMENT_COLUMN_WIDTH.min(room / zoom);
            let inset = ((viewport.size.width - column_width * zoom) / 2.)
                .max(px(0.))
                .round();
            LayoutEnv {
                text_system: window.text_system().clone(),
                style: LayoutStyle::new(theme, &settings.typography, false, false),
                viewport,
                zoom,
                column_left: viewport.left() + inset,
                column_width,
                pad_top: theme::SPACE_12,
                // Room to scroll the last lines up to a comfortable height.
                pad_bottom: (viewport.size.height * 0.4).round() / zoom,
                page: None,
                max_scroll_x: px(0.),
                rest_scroll_x: px(0.),
            }
        };
        let mut layout = self.layout.borrow_mut();
        if layout.revision() != self.state.revision() || layout.count() != document.block_count() {
            layout.rebuild(document, self.state.revision());
        }
        // The room of a sheet that is wider than it changed (a panel
        // opened or closed beside the text, the window was resized), at
        // the same zoom: what the writer was at is brought back into view
        // sideways, unless something else is already on its way there.
        let room_changed = layout.env().is_some_and(|old| {
            old.page.is_some()
                && env.page.is_some()
                && env.max_scroll_x > px(0.)
                && (old.zoom - env.zoom).abs() < 0.0005
                && ((old.viewport.size.width - env.viewport.size.width).abs() > px(0.5)
                    || (old.viewport.left() - env.viewport.left()).abs() > px(0.5))
        });
        layout.set_env(env, document);
        drop(layout);
        if room_changed && self.pending_scroll.is_none() {
            self.pending_scroll = Some(PendingScroll::Sideways);
        }
        shown
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
        #[cfg(test)]
        let mut tints: Vec<Bounds<Pixels>> = Vec::new();
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
            let view_height = layout.env().map_or(px(0.), LayoutEnv::view_height);
            let visible = layout.stabilize(document, view_height);
            let mut lay = Lay { layout, document };
            let scroll_y = lay.scroll_y();
            // Everything below is planned in layout units and put into
            // the window through this one transform.
            let view = lay.view();
            let zoom = view.zoom;
            let (colors, surface, paged) = {
                let env = lay.env();
                plan.row_height = env.style.base_line_height() * zoom;
                (env.style.colors, env.style.surface(), env.page.is_some())
            };

            // Canvas, or the desk with its sheets.
            if paged {
                plan.background = Some(colors.desk);
                let pages = lay.layout.page_count();
                let env = lay.env();
                if let Some(geometry) = &env.page {
                    plan.sheet_color = colors.page;
                    plan.sheet_shadows = page_shadows(
                        colors.shadow,
                        colors.appearance.is_dark(),
                        self.zoom_shown / 100.,
                    );
                    // A shadow separates a white sheet from a light desk.
                    // On Night's desk it cannot be seen: there the sheet
                    // gets an edge that catches a little light.
                    plan.sheet_border = colors
                        .appearance
                        .is_dark()
                        .then_some(colors.border_strong.alpha(0.7));
                    // The page number: in the lower part of the bottom
                    // margin, so that the text above it keeps its air, and
                    // smaller when the margin is small. No number in a
                    // margin that has no room for one at all.
                    let margin = view.len(geometry.margin_bottom);
                    let label_size = view
                        .len(theme::UI_TEXT_XS)
                        .max(px(8.))
                        .min((margin * 0.5).max(px(6.)));
                    let label_height = (label_size * 1.4).round();
                    let numbered = margin >= label_height + px(2.);
                    for page in geometry.page_at(scroll_y, env.pad_top)..pages {
                        let sheet = view.rect(Bounds::new(
                            point(geometry.sheet_x(), geometry.sheet_top(page, env.pad_top)),
                            size(geometry.sheet_width, geometry.sheet_height),
                        ));
                        if sheet.top() >= viewport.bottom() {
                            break;
                        }
                        plan.sheets.push(sheet);
                        let visible = sheet.intersect(&viewport);
                        if visible.size.width > px(0.) && visible.size.height > px(0.) {
                            plan.sheet_hitboxes
                                .push(window.insert_hitbox(visible, HitboxBehavior::Normal));
                        }
                        if numbered {
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
                                    // Its middle two fifths of the way
                                    // up the margin.
                                    (sheet.bottom() - margin * 0.4 - label_height / 2.)
                                        .min(sheet.bottom() - label_height - px(1.))
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
            self.spelling.on_screen = visible.clone();
            plan.spelling_color = colors.spelling;
            let view_top = scroll_y;
            let view_bottom = scroll_y + view_height;

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
                        view,
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
                            let rect = view.rect(rect);
                            #[cfg(test)]
                            tints.push(rect);
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
                            let rect = view.rect(rect);
                            #[cfg(test)]
                            tints.push(rect);
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

                // Spelling marks: a wave under each marked word, row by row.
                if self.spelling.visible && !block_layout.void {
                    let marks = self.spelling.ranges_at(document, index);
                    if !marks.is_empty() {
                        // The wave is drawn as large as the text is.
                        let thickness = snap(spelling::WAVE_THICKNESS * zoom).max(px(1.));
                        let top = view.len(block_layout.baseline)
                            + (spelling::wave_offset(block_layout.font_size) * zoom).max(px(1.));
                        for range in marks {
                            for (row, rect) in placed.row_rects(range.clone(), rows.clone()) {
                                let row_bytes = &block_layout.rows[row].range;
                                let rect = if range.end > row_bytes.end {
                                    // The mark goes on in the next row: no
                                    // wave under the space the row was
                                    // broken at.
                                    let start = range.start.max(row_bytes.start);
                                    let kept = block
                                        .text()
                                        .get(start..row_bytes.end)
                                        .map_or(0, |piece| piece.trim_end().len());
                                    match placed
                                        .rects(start..start + kept, row..row + 1)
                                        .first()
                                        .filter(|_| kept > 0)
                                    {
                                        Some(rect) => *rect,
                                        None => continue,
                                    }
                                } else {
                                    rect
                                };
                                let rect = view.rect(rect);
                                plan.spelling.push(PlannedWave {
                                    origin: point(rect.left(), snap(rect.top() + top)),
                                    width: rect.size.width,
                                    thickness,
                                });
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
                        let mut rects = placed.row_rects(local.start..end, rows.clone());
                        // The selection is one shape: where it goes on in
                        // the next block, the space between the two blocks
                        // belongs to it (as tall as the gap, as wide as
                        // this block's last row), unless the next block
                        // starts on another sheet.
                        let continues = index + 1 < range.end.block
                            || (index + 1 == range.end.block
                                && (range.end.offset > 0
                                    || document
                                        .block(index + 1)
                                        .is_some_and(|next| next.is_void())));
                        if continues
                            && let Some((row, last)) = rects.last_mut()
                            && *row + 1 == block_layout.rows.len()
                        {
                            let next_top = lay.layout.first_row_top(index + 1);
                            if next_top > last.bottom()
                                && lay.page_at(last.top()) == lay.page_at(next_top)
                            {
                                last.size.height = next_top - last.top();
                            }
                        }
                        for (_, rect) in rects {
                            // Whole device pixels vertically: rows meet
                            // without a seam or an overlap.
                            let rect = view.rect(rect);
                            let (top, bottom) = (snap(rect.top()), snap(rect.bottom()));
                            plan.selection.push(fill(
                                Bounds::new(
                                    point(rect.left(), top),
                                    size(rect.size.width, bottom - top),
                                ),
                                selection_color,
                            ));
                        }
                    }
                }

                // Text of an IME composition: underlined, and underlined
                // twice as thick where the input method has a clause
                // selected.
                if let Some(marked) = &self.marked
                    && marked.block == index
                {
                    let thin = px((f32::from(view.len(block_layout.font_size)) / 16.)
                        .round()
                        .max(1.));
                    let clause = selected
                        .and_then(|range| document.range_in_block(range, index))
                        .map(|local| {
                            local.start.max(marked.range.start)..local.end.min(marked.range.end)
                        })
                        .filter(|clause| clause.start < clause.end);
                    let mut underline = |range: std::ops::Range<usize>, thickness: Pixels| {
                        for rect in placed.rects(range, rows.clone()) {
                            let rect = view.rect(rect);
                            let y = rect.top()
                                + view.len(block_layout.baseline + block_layout.descent * 0.6);
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
                            origin: view.point(point(block_layout.text_left, placed.row_top(row))),
                            zoom,
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
                    let rect = caret_rect(
                        &placed,
                        block.text(),
                        head.offset,
                        style,
                        caret_upstream,
                        zoom,
                    );
                    let color = match style {
                        // A block caret lets the character show through.
                        CaretStyle::Block => colors.caret.alpha(0.45),
                        _ => colors.caret,
                    };
                    plan.front.push(
                        fill(view.rect(drawn_caret(rect, block_layout, style)), color)
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
                    view.len(placed.layout.font_size),
                    colors.faint,
                );
                plan.markers.push(PlannedMarker {
                    line,
                    origin: view.point(point(placed.layout.text_left, placed.row_top(0))),
                    line_height: view.len(placed.layout.line_height),
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
                let content = max_scroll + view_height;
                let thumb_height = (track.size.height * (view_height / content))
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
        #[cfg(test)]
        {
            self.painted_selection = plan.selection.iter().map(|quad| quad.bounds).collect();
            self.painted_markers = plan
                .markers
                .iter()
                .map(|marker| {
                    Bounds::new(marker.origin, size(marker.line.width, marker.line_height))
                })
                .collect();
            self.painted_spelling = plan
                .spelling
                .iter()
                .map(|wave| Bounds::new(wave.origin, size(wave.width, wave.thickness * 3.)))
                .collect();
            self.painted_tints = tints;
            self.painted_rows = plan
                .rows
                .iter()
                .map(|row| (row.origin, row.layout.font_size * row.zoom))
                .collect();
            self.painted_sheets = plan.sheets.clone();
        }
        self.checkbox_hits = checkboxes;
        self.scrollbar.geometry = scrollbar;
        self.caret_painted = caret_painted;
        plan
    }
}

/// The soft shadow under a sheet of paper, as large as the sheet is
/// drawn (`scale` is 1 at 100 %).
fn page_shadows(color: Hsla, dark: bool, scale: f32) -> Vec<BoxShadow> {
    let strength = if dark { 2. } else { 1. };
    let scale = scale.clamp(0.25, 4.);
    vec![
        BoxShadow {
            color: color.alpha(0.05 * strength),
            offset: point(px(0.), px(1.) * scale),
            blur_radius: px(2.) * scale,
            spread_radius: px(0.),
        },
        BoxShadow {
            color: color.alpha(0.07 * strength),
            offset: point(px(0.), px(6.) * scale),
            blur_radius: px(18.) * scale,
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
            let zoom = editor.install_layout_env(Some(bounds), bounds.size.width, window, cx);
            editor.note_zoom_shown(zoom, cx);
            editor.settle_scroll();
            let plan = editor.plan_frame(hitbox, window);
            editor.prepaint_link_tip(window);
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
                let mut quad = fill(*sheet, plan.sheet_color).corner_radii(theme::RADIUS_PAGE);
                if let Some(border) = plan.sheet_border {
                    quad = quad.border_widths(px(1.)).border_color(border);
                }
                window.paint_quad(quad);
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
            // All the text is one layer: glyphs do not cover each other,
            // so the scene does not have to work out an order for each of
            // the thousands of them. (Within a layer quads are drawn
            // before glyphs, which puts underlines and strikethroughs
            // behind the letters they have the colour of.)
            window.paint_layer(bounds, |window| {
                // Under the glyphs, like the other underlines.
                let mut wave = UnderlineStyle {
                    color: Some(plan.spelling_color),
                    thickness: px(1.),
                    wavy: true,
                };
                for mark in &plan.spelling {
                    wave.thickness = mark.thickness;
                    window.paint_underline(mark.origin, mark.width, &wave);
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
            });
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
            if phase != DispatchPhase::Bubble || !down_hitbox.is_hovered(window) {
                return;
            }
            // The secondary button asks for the context menu. So does a
            // ctrl-click: GPUI's macOS platform already reports it as the
            // right button; a left press that still carries `ctrl` is
            // taken the same way.
            let secondary = event.button == MouseButton::Right
                || (event.button == MouseButton::Left && event.modifiers.control);
            if secondary {
                editor.update(cx, |editor, cx| editor.context_click(event, window, cx));
            } else if event.button == MouseButton::Left {
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
        window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
            // Only the button that started the drag ends it. (A release
            // GPUI reports as another button, the left one let go with
            // `ctrl` held, is caught by the next mouse move.)
            if phase == DispatchPhase::Bubble
                && event.button == MouseButton::Left
                && editor.read(cx).mouse_busy()
            {
                editor.update(cx, |editor, cx| editor.mouse_up(cx));
            }
        });

        let editor = self.editor.clone();
        let scroll_hitbox = hitbox.clone();
        window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble && scroll_hitbox.should_handle_scroll(window) {
                let delta = event.delta.pixel_delta(row_height);
                editor.update(cx, |editor, cx| {
                    if event.modifiers.platform && !editor.config.mode.is_compact() {
                        // `cmd` with the wheel zooms around the pointer:
                        // towards the top of the wheel is in.
                        editor.zoom_by(wheel_zoom(event.delta), event.position, cx);
                    } else {
                        editor.scroll_wheel(delta, event.modifiers.shift, cx);
                    }
                });
            }
        });
    }
}

/// How much `cmd` with the wheel zooms for a turn of it: a notch of a
/// wheel is seven percent at every zoom, a long swipe on the trackpad
/// doubles or halves.
pub(crate) fn wheel_zoom(delta: gpui::ScrollDelta) -> f32 {
    let turn = match delta {
        gpui::ScrollDelta::Pixels(delta) => f32::from(delta.y) / 400.,
        gpui::ScrollDelta::Lines(delta) => delta.y * 0.07,
    };
    turn.clamp(-1., 1.).exp()
}

impl Editor {
    /// Bookkeeping at the end of a frame's prepaint.
    fn after_frame(&mut self, prepaint: std::time::Duration, cx: &mut Context<Self>) {
        self.perf.set(self.perf.get().with_prepaint(prepaint));
        self.start_background_work(cx);
    }
}
