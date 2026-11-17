//! Scrolling: the scroll position, scroll requests, autoscroll and the
//! overlay scroll bar.
//!
//! The editor scrolls itself (it is not inside a GPUI scroll container).
//! The position is anchored to a block (see `layout::ScrollPosition`), so
//! laying out or editing text above the viewport never moves what is on
//! screen; [`Editor::scroll_offset`] reports it in pixels for the current
//! layout.
//!
//! Inside, a scroll position is in layout units (the document's points),
//! like the layout itself; what comes in from the window (a wheel delta, a
//! distance from the top of the viewport) is in window pixels and is
//! divided by the zoom of the view where it enters. Sideways the view only
//! moves in page view, and only while a sheet is wider than the room it
//! has (see `crate::zoom`).
//!
//! A scroll request that needs geometry (bring the caret into view, put a
//! position at the top) is applied at once when a layout exists, and again
//! in the next frame's prepaint, after the blocks around the target were
//! shaped (their heights were estimates before). Without a layout it waits
//! for the first frame.

use std::time::Duration;

use gpui::{Bounds, Context, Pixels, Task, px};
use innerword_core::Position;

use crate::{
    editor::Editor,
    layout::{Lay, ScrollPosition, pages::DESK_SIDE_PADDING},
};

/// Where a position should end up in the viewport after a change or a
/// scroll request.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Autoscroll {
    /// Do not scroll.
    None,
    /// Scroll the least amount that brings the position into view with a
    /// little room around it (nothing when it already is).
    #[default]
    Fit,
    /// Put the position's row at the top of the viewport (Vim `zt`).
    Top,
    /// Put the position's row in the middle of the viewport (Vim `zz`).
    Center,
    /// Put the position's row at the bottom of the viewport (Vim `zb`).
    Bottom,
}

/// A scroll request waiting for a layout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum PendingScroll {
    /// Bring a position (`None`: the caret, see `Editor::caret_position`)
    /// into view.
    Position(Option<Position>, Autoscroll),
    /// Bring the caret into view after a change to the text, or after a
    /// jump to the end of the document. Like `Position(None,
    /// Autoscroll::Fit)`, but whoever writes at the very end of the
    /// document gets room under the last line instead of writing along
    /// the bottom edge of the window.
    Writing,
    /// Put the caret's row `viewport_y` window pixels below the top of the
    /// viewport (page-wise movement, switching between flow and page
    /// view, a change of zoom from the keyboard).
    CaretAt { viewport_y: Pixels },
    /// Put a point of the text back where it was in the viewport after
    /// the zoom changed under the pointer: the point `dy` layout units
    /// below the top of the row that `position` starts, and `x` layout
    /// units from the column's left edge, goes to `at` (window pixels from
    /// the viewport's top left corner).
    Anchor {
        position: Position,
        dy: Pixels,
        x: Pixels,
        at: gpui::Point<Pixels>,
    },
    /// Page view: put the sheet the caret is on in the middle of the
    /// viewport (the zoom became "fit page": the whole sheet shows).
    SheetOfCaret,
    /// Page view: bring the caret, or the start of the selection, into
    /// view sideways, and leave the view where it is up and down. The
    /// room of a sheet that is wider than it changed (a panel opened or
    /// closed beside the text, the window was resized): what the writer
    /// was at would otherwise be left beside the picture.
    Sideways,
}

/// Flow view: how far above the bottom of the viewport the last line of
/// the document is kept while it is written on, as a fraction of the
/// viewport's height. (The content ends with more empty space than this,
/// see `Editor::install_layout_env`.)
const WRITING_ROOM: f32 = 0.3;

/// How long the scroll bar stays after the last scroll.
const SCROLLBAR_LINGER: Duration = Duration::from_millis(1200);

/// Where the scroll bar was in the last frame, in window coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ScrollbarGeometry {
    pub(crate) track: Bounds<Pixels>,
    pub(crate) thumb: Bounds<Pixels>,
    pub(crate) max_scroll: Pixels,
}

/// The overlay scroll bar: shown while scrolling, while the pointer is
/// over its strip and while its thumb is dragged.
#[derive(Default)]
pub(crate) struct Scrollbar {
    /// Recently scrolled.
    lingering: bool,
    hide_task: Option<Task<()>>,
    /// The pointer is over the strip.
    pub(crate) hovered: bool,
    /// The thumb is being dragged: distance from its top to the pointer.
    pub(crate) drag: Option<Pixels>,
    /// `None` when the content fits.
    pub(crate) geometry: Option<ScrollbarGeometry>,
}

impl Scrollbar {
    pub(crate) fn is_shown(&self) -> bool {
        self.lingering || self.hovered || self.drag.is_some()
    }

    pub(crate) fn is_active(&self) -> bool {
        self.hovered || self.drag.is_some()
    }
}

/// Scrolls sideways the least that brings `x` (layout units from the
/// column's left edge) into view with a little room beside it; nothing
/// when it is in view, and nothing unless a sheet is wider than the room
/// it has. Says whether the view moved.
fn bring_into_view_sideways(lay: &mut Lay, x: Pixels) -> bool {
    let max_x = lay.layout.max_scroll_x();
    if max_x <= px(0.) {
        return false;
    }
    let env = lay.env();
    let zoom = env.zoom;
    let width = env.viewport.size.width / zoom;
    let before = lay.layout.scroll_x();
    // Column-local x of the viewport's left edge.
    let left = (env.viewport.left() - env.column_left) / zoom + before;
    // The desk that shows beside a sheet at either end is the room kept
    // beside the caret: at the start of a line the view is where it rests.
    let room = (px(DESK_SIDE_PADDING) / zoom).min(width / 4.);
    let slack = px(0.01);
    let target = if x - room < left - slack {
        before - (left - (x - room))
    } else if x + room > left + width + slack {
        before + (x + room - (left + width))
    } else {
        return false;
    };
    let target = target.clamp(px(0.), max_x);
    lay.layout.set_scroll_x(Some(target));
    (target - before).abs() > px(0.01)
}

impl Editor {
    /// How far the content is scrolled, in window pixels from its top (at
    /// the zoom the view has). Blocks that were never laid out count with
    /// an estimated height, so this number can change a little while the
    /// background layout catches up even though the view does not move.
    pub fn scroll_offset(&self) -> Pixels {
        self.with_layout(|lay| lay.scroll_y() * lay.env().zoom)
            .unwrap_or(px(0.))
    }

    /// How far the view is scrolled sideways, in window pixels: 0 except
    /// in page view while a sheet is wider than the room it has.
    pub fn horizontal_scroll_offset(&self) -> Pixels {
        self.with_layout(|lay| lay.layout.scroll_x() * lay.env().zoom)
            .unwrap_or(px(0.))
    }

    /// How far the view can be scrolled sideways, in window pixels.
    pub fn max_horizontal_scroll(&self) -> Pixels {
        self.with_layout(|lay| lay.layout.max_scroll_x() * lay.env().zoom)
            .unwrap_or(px(0.))
    }

    /// Scrolls so the caret (the selection head, or the position set with
    /// [`Editor::set_caret_override`]) is where `autoscroll` says: [`Autoscroll::Top`], [`Autoscroll::Center`] and
    /// [`Autoscroll::Bottom`] are Vim's `zt`, `zz`, `zb`.
    pub fn scroll_to_caret(&mut self, autoscroll: Autoscroll, cx: &mut Context<Self>) {
        self.request_scroll(PendingScroll::Position(None, autoscroll));
        cx.notify();
    }

    /// Scrolls so `position` is where `autoscroll` says, without moving the
    /// caret (jumping to a find match or a comment anchor).
    pub fn scroll_to_position(
        &mut self,
        position: Position,
        autoscroll: Autoscroll,
        cx: &mut Context<Self>,
    ) {
        self.request_scroll(PendingScroll::Position(Some(position), autoscroll));
        cx.notify();
    }

    /// Scrolls by whole rows (positive: content moves up, later text comes
    /// into view) without moving the caret. One row is the line height of
    /// body text. Vim `ctrl-e` / `ctrl-y`.
    pub fn scroll_by_rows(&mut self, rows: isize, cx: &mut Context<Self>) {
        let Some(row_height) = self.row_height() else {
            return;
        };
        self.scroll_by(row_height * rows as f32, cx);
    }

    /// Scrolls by viewport pages (`1.0` a page down, `-0.5` half a page up)
    /// without moving the caret. A page is [`Editor::rows_per_page`] rows.
    pub fn scroll_by_pages(&mut self, pages: f32, cx: &mut Context<Self>) {
        let (Some(row_height), Some(rows)) = (self.row_height(), self.rows_per_page()) else {
            return;
        };
        let rows = (rows as f32 * pages).round();
        self.scroll_by(row_height * rows, cx);
    }

    /// Scrolls by window pixels (positive: later text comes into view).
    pub fn scroll_by_pixels(&mut self, delta: Pixels, cx: &mut Context<Self>) {
        let zoom = self.view_zoom();
        self.scroll_by(delta / zoom, cx);
    }

    /// Scrolls sideways by window pixels (positive: what is further right
    /// comes into view). Does nothing unless a sheet is wider than the
    /// room it has.
    pub fn scroll_sideways_by_pixels(&mut self, delta: Pixels, cx: &mut Context<Self>) {
        let moved = self
            .with_layout(|lay| {
                let max = lay.layout.max_scroll_x();
                if max <= px(0.) {
                    return false;
                }
                let before = lay.layout.scroll_x();
                let target = (before + delta / lay.env().zoom).clamp(px(0.), max);
                lay.layout.set_scroll_x(Some(target));
                target != before
            })
            .unwrap_or(false);
        if moved {
            cx.notify();
        }
    }

    /// Window pixels per layout unit, as of the last layout (1 before it).
    pub(crate) fn view_zoom(&self) -> f32 {
        self.layout.borrow().env().map_or(1., |env| env.zoom)
    }

    /// Scrolls by layout units (positive: later text comes into view).
    pub(crate) fn scroll_by(&mut self, delta: Pixels, cx: &mut Context<Self>) {
        self.pending_scroll = None;
        let moved = self
            .with_layout(|lay| {
                let before = lay.scroll_y();
                let max = lay.layout.max_scroll();
                let target = (before + delta).clamp(px(0.), max);
                lay.layout.set_scroll_y(target);
                target != before
            })
            .unwrap_or(false);
        if moved {
            self.flash_scrollbar(cx);
            cx.notify();
        }
    }

    /// Scrolls to an absolute offset in layout units (the scroll bar
    /// thumb).
    pub(crate) fn scroll_to_y(&mut self, y: Pixels, cx: &mut Context<Self>) {
        self.pending_scroll = None;
        self.with_layout(|lay| {
            let max = lay.layout.max_scroll();
            lay.layout.set_scroll_y(y.clamp(px(0.), max));
        });
        cx.notify();
    }

    /// Line height of body text in layout units: the unit of
    /// [`Editor::scroll_by_rows`].
    pub(crate) fn row_height(&self) -> Option<Pixels> {
        self.layout
            .borrow()
            .env()
            .map(|env| env.style.base_line_height())
    }

    /// Shows the scroll bar for a moment.
    pub(crate) fn flash_scrollbar(&mut self, cx: &mut Context<Self>) {
        self.scrollbar.lingering = true;
        self.scrollbar.hide_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SCROLLBAR_LINGER).await;
            this.update(cx, |editor, cx| {
                editor.scrollbar.lingering = false;
                cx.notify();
            })
            .ok();
        }));
    }

    /// Records a scroll request and applies it now if a layout exists. It
    /// stays pending so the next frame can apply it again with the blocks
    /// around the target shaped.
    pub(crate) fn request_scroll(&mut self, request: PendingScroll) {
        if matches!(request, PendingScroll::Position(_, Autoscroll::None)) {
            return;
        }
        self.pending_scroll = Some(request);
        self.apply_scroll(request);
    }

    /// Records a scroll request for the next frame without applying it to
    /// the current layout (the layout is about to change shape).
    pub(crate) fn defer_scroll(&mut self, request: PendingScroll) {
        self.pending_scroll = Some(request);
    }

    /// The caret row's distance from the top of the viewport, when the row
    /// is on screen.
    pub(crate) fn caret_viewport_y(&self) -> Option<Pixels> {
        let viewport = self.viewport_bounds()?;
        let caret = self.caret_line_bounds()?;
        (caret.top() >= viewport.top() && caret.bottom() <= viewport.bottom())
            .then(|| caret.top() - viewport.top())
    }

    /// The top of the caret row's distance from the top of the viewport,
    /// when that top is on screen, also where the row goes on below the
    /// viewport (a picture that is only partly in view): where a page key
    /// keeps the caret.
    pub(crate) fn caret_top_viewport_y(&self) -> Option<Pixels> {
        let viewport = self.viewport_bounds()?;
        let caret = self.caret_line_bounds()?;
        (caret.top() >= viewport.top() && caret.top() < viewport.bottom())
            .then(|| caret.top() - viewport.top())
    }

    /// Applies a scroll request to the current layout. Returns whether the
    /// scroll position changed; `None` without a layout.
    fn apply_scroll(&mut self, request: PendingScroll) -> Option<bool> {
        let head = self.caret_position();
        let head_upstream = self.caret_is_upstream();
        let document = self.state.document();
        let selection = self.state.selection();
        let compact = self.config.mode.is_compact();
        self.with_layout(|lay| {
            let viewport_height = lay.env().view_height();
            let zoom = lay.env().zoom;
            if let PendingScroll::Anchor {
                position,
                dy,
                x,
                at,
            } = request
            {
                let position = document.clamp_position(position);
                let Some(placed) = lay.placed(position.block) else {
                    return false;
                };
                let row = placed.layout.row_for_offset(position.offset);
                let before = (lay.scroll_y(), lay.layout.scroll_x());
                let target = placed.row_top(row) + dy - at.y / zoom;
                let max = lay.layout.max_scroll();
                lay.layout.set_scroll_y(target.clamp(px(0.), max));
                let max_x = lay.layout.max_scroll_x();
                if max_x > px(0.) {
                    // The window x of `x` is the column's left edge, less
                    // what is scrolled, plus `x`, all at the zoom.
                    let env = lay.env();
                    let column = (env.column_left - env.viewport.left()) / zoom;
                    let target = column + x - at.x / zoom;
                    lay.layout.set_scroll_x(Some(target.clamp(px(0.), max_x)));
                } else {
                    lay.layout.set_scroll_x(None);
                }
                let after = (lay.scroll_y(), lay.layout.scroll_x());
                return (after.0 - before.0).abs() > px(0.01)
                    || (after.1 - before.1).abs() > px(0.01);
            }
            if request == PendingScroll::SheetOfCaret {
                let position = document.clamp_position(head);
                let Some(placed) = lay.placed(position.block) else {
                    return false;
                };
                let row = placed.caret_box(position.offset, head_upstream);
                let env = lay.env();
                let Some(geometry) = &env.page else {
                    return false;
                };
                let sheet = geometry.page_at(row.top(), env.pad_top);
                let above = ((viewport_height - geometry.sheet_height) / 2.).max(px(0.));
                let target = geometry.sheet_top(sheet, env.pad_top) - above;
                let before = lay.scroll_y();
                let max = lay.layout.max_scroll();
                lay.layout.set_scroll_y(target.clamp(px(0.), max));
                return (lay.scroll_y() - before).abs() > px(0.01);
            }
            if request == PendingScroll::Sideways {
                // The start of what is selected: where reading it begins.
                let (position, upstream) = if selection.is_collapsed() {
                    (head, head_upstream)
                } else {
                    (selection.start(), false)
                };
                let position = document.clamp_position(position);
                let Some(placed) = lay.placed(position.block) else {
                    return false;
                };
                let row = placed.caret_box(position.offset, upstream);
                // Only a row that shows: whoever scrolled away from the
                // caret to read elsewhere is left where they are.
                let scroll_y = lay.scroll_y();
                if row.bottom() <= scroll_y || row.top() >= scroll_y + viewport_height {
                    return false;
                }
                // A selection on one row is shown whole where the room
                // holds it: its end first, then its start, which wins
                // where both do not fit.
                let end = document.clamp_position(selection.end());
                let end_on_the_row = (!selection.is_collapsed() && end.block == position.block)
                    .then(|| placed.caret_box(end.offset, true))
                    .filter(|end| (end.top() - row.top()).abs() < px(0.5));
                let moved_to_end =
                    end_on_the_row.is_some_and(|end| bring_into_view_sideways(lay, end.left()));
                return bring_into_view_sideways(lay, row.left()) || moved_to_end;
            }
            let (position, mode, viewport_y) = match request {
                PendingScroll::Position(position, mode) => (position, mode, px(0.)),
                PendingScroll::Writing => (None, Autoscroll::Fit, px(0.)),
                PendingScroll::CaretAt { viewport_y } => (None, Autoscroll::Top, viewport_y / zoom),
                PendingScroll::Anchor { .. }
                | PendingScroll::SheetOfCaret
                | PendingScroll::Sideways => return false,
            };
            // The caret is on the row its affinity puts it on.
            let upstream = position.is_none() && head_upstream;
            let position = document.clamp_position(position.unwrap_or(head));
            let Some(placed) = lay.placed(position.block) else {
                return false;
            };
            let row = placed.caret_box(position.offset, upstream);
            let (top, bottom) = (row.top(), row.bottom());
            let scroll_y = lay.scroll_y();
            // Room kept around the caret row: a line of text, less in a
            // small viewport.
            let margin = if compact {
                px(2.)
            } else {
                lay.env().style.base_line_height()
            };
            let margin = margin.min(viewport_height / 4.);
            // Sideways: a sheet wider than its room follows the position
            // too, so whoever types never types out of sight.
            let moved_sideways = bring_into_view_sideways(lay, row.left());
            // Writing on the last line of a document in flow view: room
            // under it.
            let last_row = position.block + 1 == document.block_count()
                && placed.layout.row_for_caret(position.offset, upstream) + 1
                    == placed.layout.rows.len();
            let below = if request == PendingScroll::Writing
                && last_row
                && !compact
                && lay.env().page.is_none()
            {
                margin.max((viewport_height * WRITING_ROOM).round())
            } else {
                margin
            };
            let target = match mode {
                Autoscroll::None => None,
                Autoscroll::Fit => {
                    // The first row of the document shows what is above
                    // it too (padding, the top of the sheet).
                    let first_row = position.block == 0
                        && placed.layout.row_for_caret(position.offset, upstream) == 0;
                    let by_top = if first_row { px(0.) } else { top - margin };
                    if placed.layout.image.is_some()
                        && bottom - top > viewport_height - margin - below
                    {
                        // A picture higher than the viewport has room for:
                        // its top and its bottom cannot both be brought
                        // into view, and the two rules below would send
                        // the view back and forth. It is fitted by its
                        // top, and left alone where it fills the viewport
                        // already. (Rows of text are left to the two
                        // rules: none is higher than a viewport that
                        // scrolls.)
                        let fills = top <= scroll_y && bottom >= scroll_y + viewport_height;
                        (!fills).then_some(by_top)
                    } else if top - margin < scroll_y {
                        Some(by_top)
                    } else if bottom + below > scroll_y + viewport_height {
                        Some(bottom + below - viewport_height)
                    } else {
                        None
                    }
                }
                Autoscroll::Top => Some(top - viewport_y),
                Autoscroll::Center => Some((top + bottom) / 2. - viewport_height / 2.),
                Autoscroll::Bottom => Some(bottom - viewport_height),
            };
            let Some(target) = target else {
                return moved_sideways;
            };
            let max = lay.layout.max_scroll();
            lay.layout.set_scroll_y(target.clamp(px(0.), max));
            moved_sideways || (lay.scroll_y() - scroll_y).abs() > px(0.01)
        })
    }

    /// Prepaint: shapes what is in the viewport and applies the pending
    /// scroll request until both agree (shaping changes heights, which
    /// moves the target).
    pub(crate) fn settle_scroll(&mut self) {
        let Some(height) = self.layout.borrow().env().map(|env| env.view_height()) else {
            return;
        };
        let stabilize = |editor: &Self| {
            let mut layout = editor.layout.borrow_mut();
            layout.stabilize(editor.state.document(), height);
        };
        stabilize(self);
        if let Some(request) = self.pending_scroll.take() {
            for _ in 0..4 {
                if self.apply_scroll(request) != Some(true) {
                    break;
                }
                stabilize(self);
            }
        }
    }

    /// The scroll position as a block and an offset into it in layout
    /// units, which no zoom changes (what a saved view stores).
    pub(crate) fn scroll_anchor(&self) -> (usize, Pixels) {
        let position = self.layout.borrow().scroll_position();
        match position {
            ScrollPosition::Block { block, offset } => (block, offset),
            ScrollPosition::Top => {
                // The top is above the first block's box by its padding.
                let lead = self.with_layout(|lay| lay.layout.top(0)).unwrap_or(px(0.));
                (0, -lead)
            }
        }
    }

    pub(crate) fn set_scroll_anchor(&mut self, block: usize, offset: Pixels) {
        self.pending_scroll = None;
        let count = self.state.document().block_count();
        self.layout
            .borrow_mut()
            .set_scroll_position(ScrollPosition::Block {
                block: block.min(count.saturating_sub(1)),
                offset,
            });
    }
}
