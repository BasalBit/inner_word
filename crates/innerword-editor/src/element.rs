//! The custom element that lays out, paints and wires up an [`Editor`].
//!
//! - `request_layout`: fills its container (full mode) or measures its
//!   content height for the offered width (compact mode).
//! - `prepaint`: installs the frame's layout environment, shapes the blocks
//!   in the viewport (and only those), applies pending scroll requests and
//!   builds a [`FramePlan`]: everything to paint, as owned data.
//! - `paint`: registers the platform input handler and the mouse listeners,
//!   then paints the plan back to front: desk or canvas, sheets, block
//!   decorations, comment tints and highlight layers, pictures, run
//!   backgrounds, selection, glyphs, markers, caret (or the outline and
//!   the handles of a selected image), scroll bar, and over everything
//!   the chip that says how wide an image is being made.
//!
//! Per block kind details are in `decorations`; rows of text are painted by
//! `paint`.

use std::{sync::Arc, time::Instant};

use gpui::{
    App, AvailableSpace, Bounds, BoxShadow, ContentMask, Context, Corners, CursorStyle,
    DispatchPhase, Element, ElementId, ElementInputHandler, Entity, FileDropEvent, GlobalElementId,
    Hitbox, HitboxBehavior, Hsla, InspectorElementId, IntoElement, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point, RenderImage,
    ScrollWheelEvent, ShapedLine, Style, UnderlineStyle, Window, fill, point, px, relative, size,
};
use innerword_core::{BlockKind, ImageWidth};
use innerword_ui::theme::{self, ActiveTheme};

use crate::{
    config::{CaretStyle, EditorMode},
    decorations::{self, BlockContext, CheckboxHit, PageNumber, ScreenSink},
    editor::Editor,
    geometry::{caret_rect, drawn_caret},
    images::{HANDLE_SIZE, ImageStatus, OUTLINE_WIDTH, handles_of},
    layout::{
        Lay, LayoutEnv, LayoutStyle, PageGeometry,
        pages::{DESK_PADDING, DESK_SIDE_PADDING},
    },
    mouse::Drag,
    paint::{PlannedRow, snap},
    scroll::{PendingScroll, ScrollbarGeometry},
    spelling::{self, PlannedWave},
    tick::PlannedTick,
};

/// Narrowest text column of a full editor.
const MIN_COLUMN_WIDTH: Pixels = px(120.);
/// Width assumed for a compact editor when the layout offers none and
/// none was ever given to it.
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

/// The picture of an image block in a frame.
pub(crate) struct PlannedImage {
    /// Where it is drawn, in window coordinates (on whole device pixels).
    pub(crate) bounds: Bounds<Pixels>,
    /// The decoded picture; `None` while there is none (the block's
    /// placeholder is among the decorations then).
    pub(crate) picture: Option<Arc<RenderImage>>,
    /// Painted over it while it is dragged to another place: the colour
    /// of the surface it lies on, half transparent, which is what dims a
    /// picture (GPUI paints an image at the opacity of its element, and
    /// only GPUI's own elements can set one).
    pub(crate) veil: Option<Hsla>,
}

/// What a frame painted for an image block.
#[cfg(test)]
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PaintedImage {
    pub(crate) block: usize,
    /// Where the picture or its placeholder is, in window coordinates.
    pub(crate) bounds: Bounds<Pixels>,
    /// The level of the picture that was painted (see `images::cache`);
    /// `None` for a placeholder.
    pub(crate) level: Option<u8>,
    /// What the placeholder says.
    pub(crate) label: Option<String>,
    /// The colour of the outline of a selected image.
    pub(crate) outline: Option<Hsla>,
    /// Its handles.
    pub(crate) handles: Vec<Bounds<Pixels>>,
    /// The colour that dims it while it is dragged.
    pub(crate) veil: Option<Hsla>,
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
    /// The pictures of image blocks: over the decorations, under the
    /// selection, so a selection tints a picture as it tints text.
    pub(crate) images: Vec<PlannedImage>,
    pub(crate) rows: Vec<PlannedRow>,
    /// Selection rectangles: over the run backgrounds, under the glyphs.
    pub(crate) selection: Vec<PaintQuad>,
    pub(crate) markers: Vec<PlannedMarker>,
    /// The wavy underlines of misspelled words, in the text's layer.
    pub(crate) spelling: Vec<PlannedWave>,
    pub(crate) spelling_color: Hsla,
    /// The ticks of checked to-do boxes, over the text: sprites, never
    /// GPUI paths (see `crate::tick`).
    pub(crate) ticks: Vec<PlannedTick>,
    /// The caret and anything else over the text.
    pub(crate) front: Vec<PaintQuad>,
    /// Over everything: the chip beside the handle of an image that is
    /// being resized, which says its width.
    pub(crate) chip: Option<(PaintQuad, PlannedMarker)>,
    /// The pictures (the arrow cursor shows over them).
    pub(crate) image_hitboxes: Vec<Hitbox>,
    /// The handles of the selected image, each with the cursor of its
    /// diagonal.
    pub(crate) handle_hitboxes: Vec<(Hitbox, CursorStyle)>,
    /// The cursor of the whole editor while the pointer drags an image
    /// or one of its handles.
    pub(crate) drag_cursor: Option<CursorStyle>,
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
            images: Vec::new(),
            rows: Vec::new(),
            selection: Vec::new(),
            markers: Vec::new(),
            spelling: Vec::new(),
            spelling_color: gpui::transparent_black(),
            ticks: Vec::new(),
            front: Vec::new(),
            chip: None,
            image_hitboxes: Vec::new(),
            handle_hitboxes: Vec::new(),
            drag_cursor: None,
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
                style: LayoutStyle::new(theme, &settings.typography, false, true)
                    .with_page_content(settings.page.content_size()),
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
                // Flow view shows no pages, and a picture is still as large
                // in proportion to its column as it is on a page.
                style: LayoutStyle::new(theme, &settings.typography, false, false)
                    .with_page_content(settings.page.content_size()),
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
        #[cfg(test)]
        let mut painted_images: Vec<PaintedImage> = Vec::new();
        #[cfg(test)]
        let mut painted_drop_line = None;
        // The textures of pictures that left the cache go with them.
        for picture in self.images.take_evicted() {
            window.drop_image(picture).ok();
        }
        // And those of the ticks the last frame did not paint.
        for picture in self.ticks.begin_frame() {
            window.drop_image(picture).ok();
        }
        self.images.begin_frame();
        let mut image_handles = None;
        let scale_factor = window.scale_factor();
        let document = self.state.document();
        let selection = self.state.selection();
        let caret_position = self.caret_position();
        let caret_upstream = self.caret_is_upstream();
        // The image the caret is on, and what the pointer is doing to an
        // image.
        let selected_image = self.state.selected_image().map(|(index, _)| index);
        let (moving, resizing, drop_at) = match &self.drag {
            Some(Drag::MoveImage(moving)) if moving.started => {
                plan.drag_cursor = Some(CursorStyle::ClosedHand);
                (Some(moving.block), None, moving.drop)
            }
            Some(Drag::ResizeImage(resize)) => {
                plan.drag_cursor = Some(handle_cursor(resize.corner));
                (
                    None,
                    Some((resize.block, resize.corner, resize.width)),
                    None,
                )
            }
            _ => (None, None, None),
        };
        let drop_at = self.file_drop.or(drop_at);

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
                    // The page number, where the bottom margin has room
                    // for one.
                    let margin = view.len(geometry.margin_bottom);
                    let number = PageNumber::in_margin(margin, view.len(theme::UI_TEXT_XS));
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
                        if let Some(number) = number {
                            let label = decorations::shape_label(
                                window.text_system(),
                                (page + 1).to_string().into(),
                                env.style.ui_font.clone(),
                                number.font_size,
                                colors.faint,
                            );
                            plan.markers.push(PlannedMarker {
                                origin: point(
                                    (sheet.left() + (sheet.size.width - label.width) / 2.).round(),
                                    number.top(sheet, margin),
                                ),
                                line_height: number.line_height,
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
                    &mut ScreenSink {
                        plan: &mut plan,
                        checkboxes: &mut checkboxes,
                        ticks: &mut self.ticks,
                        scale_factor,
                    },
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
                        // (A picture is tinted as a picture: its box, and
                        // nothing under it.)
                        if continues
                            && block_layout.image.is_none()
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
                // An image has no caret: the caret on it is the outline
                // around it, planned with the picture below.
                if focused
                    && active
                    && self.blink_on
                    && head.block == index
                    && block_layout.image.is_none()
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

                // The picture of an image block, or its placeholder.
                if let (Some(picture), Some(image)) = (placed.image_rect(), block.as_image()) {
                    // On whole device pixels, where the sprite is put
                    // anyway: the outline then fits the picture.
                    let rect = view.rect(picture);
                    let bounds = Bounds::from_corners(
                        point(snap(rect.left()), snap(rect.top())),
                        point(snap(rect.right()), snap(rect.bottom())),
                    );
                    let device_width = f32::from(bounds.size.width) * scale_factor;
                    let (found, status) = self.images.picture(image, device_width);
                    #[cfg(test)]
                    let mut painted = PaintedImage {
                        block: index,
                        bounds,
                        level: found.as_ref().map(|(_, level)| *level),
                        label: None,
                        outline: None,
                        handles: Vec::new(),
                        veil: None,
                    };
                    if found.is_none() {
                        // The box at the picture's size: nothing moves
                        // when the picture arrives.
                        plan.back.push(
                            fill(bounds, colors.hover)
                                .border_widths(px(1.))
                                .border_color(colors.hairline),
                        );
                        let text = match status {
                            ImageStatus::Missing => "Missing image",
                            ImageStatus::Broken => "This image cannot be shown",
                            _ if image.description.is_empty() => "Image",
                            _ => image.description.as_str(),
                        };
                        let font_size = view.len(theme::UI_TEXT_SM);
                        let line_height = (font_size * 1.4).round();
                        let room = bounds.size.width - view.len(px(16.));
                        if bounds.size.height >= line_height
                            && let Some(line) = decorations::shape_label_within(
                                window.text_system(),
                                text,
                                lay.env().style.ui_font.clone(),
                                font_size,
                                colors.faint,
                                room,
                            )
                        {
                            #[cfg(test)]
                            {
                                painted.label = Some(line.text.to_string());
                            }
                            let middle = bounds.center();
                            plan.markers.push(PlannedMarker {
                                origin: point(
                                    (middle.x - line.width / 2.).round(),
                                    (middle.y - line_height / 2.).round(),
                                ),
                                line_height,
                                line,
                            });
                        }
                    }
                    let veil = (moving == Some(block.id())).then(|| surface.alpha(0.5));
                    plan.images.push(PlannedImage {
                        bounds,
                        picture: found.map(|(picture, _)| picture),
                        veil,
                    });
                    let shown = bounds.intersect(&viewport);
                    if shown.size.width > px(0.) && shown.size.height > px(0.) {
                        plan.image_hitboxes
                            .push(window.insert_hitbox(shown, HitboxBehavior::Normal));
                    }
                    // Selected: an outline just outside the picture, and
                    // a handle at each corner while it can be resized. In
                    // window pixels at every zoom, like the caret.
                    if selected_image == Some(index) {
                        let live = focused && active;
                        let color = if live {
                            colors.accent
                        } else {
                            colors.border_strong
                        };
                        plan.front.push(
                            fill(bounds.dilate(OUTLINE_WIDTH), gpui::transparent_black())
                                .border_widths(OUTLINE_WIDTH)
                                .border_color(color),
                        );
                        #[cfg(test)]
                        {
                            painted.outline = Some(color);
                        }
                        // (Not while it is dragged elsewhere: there is
                        // nothing to take hold of then.)
                        if live && !self.config.read_only && !compact && moving.is_none() {
                            let handles = handles_of(bounds);
                            for (corner, handle) in handles.iter().enumerate() {
                                plan.front.push(
                                    fill(*handle, surface)
                                        .border_widths(px(1.))
                                        .border_color(colors.accent),
                                );
                                plan.handle_hitboxes.push((
                                    window.insert_hitbox(
                                        handle.dilate(HANDLE_SLACK),
                                        HitboxBehavior::Normal,
                                    ),
                                    handle_cursor(corner),
                                ));
                            }
                            image_handles = Some((block.id(), handles));
                            #[cfg(test)]
                            {
                                painted.handles = handles.to_vec();
                            }
                        }
                    }
                    // The width an image is being made, beside the handle
                    // that is held.
                    if let Some((_, corner, width)) =
                        resizing.filter(|(resized, ..)| *resized == block.id())
                    {
                        plan.chip = Some(plan_chip(
                            window,
                            lay.env(),
                            width,
                            handles_of(bounds)[corner].center(),
                            corner < 2,
                            viewport,
                        ));
                    }
                    #[cfg(test)]
                    {
                        painted.veil = veil;
                        painted_images.push(painted);
                    }
                }
            }

            // Where a picture that is dragged, or files that are dragged
            // in, would go: a line between two blocks.
            if let Some(boundary) = drop_at
                && let Some(line) = crate::images::drop_line(&mut lay, boundary)
            {
                #[cfg(test)]
                {
                    painted_drop_line = Some(line);
                }
                plan.front
                    .push(fill(line, colors.accent).corner_radii(px(1.)));
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
                    window.text_system(),
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
            self.painted_images = painted_images;
            self.painted_drop_line = painted_drop_line;
            self.painted_chip = plan
                .chip
                .as_ref()
                .map(|(_, label)| label.line.text.to_string());
        }
        // The ticks this frame did not paint go at the start of the next.
        self.ticks.end_frame();
        #[cfg(test)]
        {
            self.painted_ticks = plan
                .ticks
                .iter()
                .map(|tick| (tick.bounds, tick.picture.clone()))
                .collect();
        }
        self.checkbox_hits = checkboxes;
        self.image_handles = image_handles;
        self.scrollbar.geometry = scrollbar;
        self.caret_painted = caret_painted;
        plan
    }
}

/// How far around a handle of the selected image a press still takes it,
/// in window pixels.
pub(crate) const HANDLE_SLACK: Pixels = px(3.);

/// The cursor over a corner handle (0 to 3: top left, top right, bottom
/// left, bottom right): the arrow of its diagonal.
fn handle_cursor(corner: usize) -> CursorStyle {
    if corner == 0 || corner == 3 {
        CursorStyle::ResizeUpLeftDownRight
    } else {
        CursorStyle::ResizeUpRightDownLeft
    }
}

/// The chip that says how wide an image is being made ("62 %"): beside
/// the handle that is held (`at`, its centre), outside the picture (over
/// a top handle, under a bottom one), and inside the viewport. In the
/// interface's size at every zoom.
fn plan_chip(
    window: &Window,
    env: &LayoutEnv,
    width: ImageWidth,
    at: Point<Pixels>,
    above: bool,
    viewport: Bounds<Pixels>,
) -> (PaintQuad, PlannedMarker) {
    let colors = env.style.colors;
    let line = decorations::shape_label(
        window.text_system(),
        format!("{} %", width.percent()).into(),
        env.style.ui_font.clone(),
        theme::UI_TEXT_SM,
        colors.ink,
    );
    let chip = size(line.width + theme::SPACE_2 * 2., px(22.));
    let away = HANDLE_SIZE + theme::SPACE_1;
    let top = if above {
        at.y - away - chip.height
    } else {
        at.y + away
    };
    let margin = theme::SPACE_1;
    let left = (at.x - chip.width / 2.)
        .min(viewport.right() - margin - chip.width)
        .max(viewport.left() + margin);
    let top = top
        .min(viewport.bottom() - margin - chip.height)
        .max(viewport.top() + margin);
    let bounds = Bounds::new(point(left.round(), top.round()), chip);
    let border = if colors.appearance.is_dark() {
        colors.border_strong
    } else {
        colors.hairline
    };
    (
        fill(bounds, colors.elevated)
            .corner_radii(theme::RADIUS_SM)
            .border_widths(px(1.))
            .border_color(border),
        PlannedMarker {
            origin: point(bounds.left() + theme::SPACE_2, bounds.top()),
            line_height: chip.height,
            line,
        },
    )
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
        //
        // Taffy also asks without a width (min-content, max-content), and
        // the answer to such a question can be the size the editor is
        // drawn at: in the side chat's composer the last question of every
        // frame had no width (each call printed, in the app and in the
        // test), and the editor came out as wide as it answered. So a
        // width once given (offered here, or drawn at) stands for such a
        // question, as GPUI's own text answers (`elements/text.rs`), and
        // the fallback is for an editor that was never given one.
        // Answering with the fallback (240 px) every time drew the side
        // chat's text wrapped at 240 px inside a field 305 px wide, its
        // last rows under the field's border (`docs/gpui-notes.md`, "A
        // measure function asked without a width").
        let editor = self.editor.clone();
        let layout_id =
            window.request_measured_layout(style, move |known, available, window, cx| {
                let given = known.width.or(match available.width {
                    AvailableSpace::Definite(width) => Some(width),
                    _ => None,
                });
                let (width, height) = editor.update(cx, |editor, cx| {
                    if given.is_some() {
                        editor.compact_width = given;
                    }
                    let width = editor.compact_width.unwrap_or(FALLBACK_COMPACT_WIDTH);
                    (width, editor.measure_compact_height(width, window, cx))
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
            if editor.config.mode.is_compact() {
                editor.compact_width = Some(bounds.size.width);
            }
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
        // A picture is an object, not text: the arrow, and over the
        // handles of the selected one the arrows of their diagonals.
        for hitbox in &plan.image_hitboxes {
            window.set_cursor_style(CursorStyle::Arrow, hitbox);
        }
        for (hitbox, cursor) in &plan.handle_hitboxes {
            window.set_cursor_style(*cursor, hitbox);
        }
        for hitbox in &plan.pointer_hitboxes {
            window.set_cursor_style(CursorStyle::PointingHand, hitbox);
        }
        if let Some(hitbox) = &plan.scrollbar_hitbox {
            window.set_cursor_style(CursorStyle::Arrow, hitbox);
        }
        // While the pointer drags an image or a handle, the cursor says
        // so wherever it is over the editor.
        if let Some(cursor) = plan.drag_cursor {
            window.set_cursor_style(cursor, &plan.hitbox);
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
            for image in plan.images.drain(..) {
                if let Some(picture) = image.picture {
                    // A picture the atlas cannot take (it never gets one
                    // that large) is left out, not an error of the frame.
                    window
                        .paint_image(image.bounds, Corners::default(), picture, 0, false)
                        .ok();
                }
                if let Some(veil) = image.veil {
                    window.paint_quad(fill(image.bounds, veil));
                }
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
            for tick in plan.ticks.drain(..) {
                window
                    .paint_image(tick.bounds, Corners::default(), tick.picture, 0, false)
                    .ok();
            }
            for quad in plan.front.drain(..) {
                window.paint_quad(quad);
            }
            if let Some((quad, label)) = plan.chip.take() {
                window.paint_quad(quad);
                label
                    .line
                    .paint(label.origin, label.line_height, window, cx)
                    .ok();
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

        // Files that were dragged over the window left it again: the line
        // that showed where they would go goes too.
        let editor = self.editor.clone();
        window.on_mouse_event(move |event: &FileDropEvent, phase, _, cx| {
            if phase == DispatchPhase::Bubble
                && matches!(event, FileDropEvent::Exited)
                && editor.read(cx).file_drop.is_some()
            {
                editor.update(cx, |editor, cx| editor.set_file_drop(None, cx));
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
        // The pictures the frame wanted and did not have.
        self.load_wanted_images(cx);
    }
}
