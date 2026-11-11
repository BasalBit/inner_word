//! Scrolling: the scroll position, scroll requests, autoscroll and the
//! overlay scroll bar.
//!
//! The editor scrolls itself (it is not inside a GPUI scroll container).
//! The position is anchored to a block (see `layout::ScrollPosition`), so
//! laying out or editing text above the viewport never moves what is on
//! screen; [`Editor::scroll_offset`] reports it in pixels for the current
//! layout.
//!
//! A scroll request that needs geometry (bring the caret into view, put a
//! position at the top) is applied at once when a layout exists, and again
//! in the next frame's prepaint, after the blocks around the target were
//! shaped (their heights were estimates before). Without a layout it waits
//! for the first frame.

use std::time::Duration;

use gpui::{Bounds, Context, Pixels, Task, px};
use writer_core::Position;

use crate::{editor::Editor, layout::ScrollPosition};

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
    /// Put the caret's row `viewport_y` pixels below the top of the
    /// viewport (page-wise movement, switching between flow and page
    /// view).
    CaretAt { viewport_y: Pixels },
}

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

impl Editor {
    /// How far the content is scrolled, in pixels from its top. Blocks
    /// that were never laid out count with an estimated height, so this
    /// number can change a little while the background layout catches up
    /// even though the view does not move.
    pub fn scroll_offset(&self) -> Pixels {
        self.with_layout(|lay| lay.scroll_y()).unwrap_or(px(0.))
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
        self.scroll_by_pixels(row_height * rows as f32, cx);
    }

    /// Scrolls by viewport pages (`1.0` a page down, `-0.5` half a page up)
    /// without moving the caret. A page is [`Editor::rows_per_page`] rows.
    pub fn scroll_by_pages(&mut self, pages: f32, cx: &mut Context<Self>) {
        let (Some(row_height), Some(rows)) = (self.row_height(), self.rows_per_page()) else {
            return;
        };
        let rows = (rows as f32 * pages).round();
        self.scroll_by_pixels(row_height * rows, cx);
    }

    /// Scrolls by pixels (positive: later text comes into view).
    pub fn scroll_by_pixels(&mut self, delta: Pixels, cx: &mut Context<Self>) {
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

    /// Scrolls to an absolute pixel offset (the scroll bar thumb).
    pub(crate) fn scroll_to_pixels(&mut self, y: Pixels, cx: &mut Context<Self>) {
        self.pending_scroll = None;
        self.with_layout(|lay| {
            let max = lay.layout.max_scroll();
            lay.layout.set_scroll_y(y.clamp(px(0.), max));
        });
        cx.notify();
    }

    /// Line height of body text: the unit of [`Editor::scroll_by_rows`].
    fn row_height(&self) -> Option<Pixels> {
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

    /// Applies a scroll request to the current layout. Returns whether the
    /// scroll position changed; `None` without a layout.
    fn apply_scroll(&mut self, request: PendingScroll) -> Option<bool> {
        let head = self.caret_position();
        let head_upstream = self.caret_is_upstream();
        let document = self.state.document();
        let compact = self.config.mode.is_compact();
        self.with_layout(|lay| {
            let viewport_height = lay.env().viewport.size.height;
            let (position, mode, viewport_y) = match request {
                PendingScroll::Position(position, mode) => (position, mode, px(0.)),
                PendingScroll::CaretAt { viewport_y } => (None, Autoscroll::Top, viewport_y),
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
            let target = match mode {
                Autoscroll::None => None,
                Autoscroll::Fit => {
                    if top - margin < scroll_y {
                        // The first row of the document shows what is above
                        // it too (padding, the top of the sheet).
                        let first_row = position.block == 0
                            && placed.layout.row_for_caret(position.offset, upstream) == 0;
                        Some(if first_row { px(0.) } else { top - margin })
                    } else if bottom + margin > scroll_y + viewport_height {
                        Some(bottom + margin - viewport_height)
                    } else {
                        None
                    }
                }
                Autoscroll::Top => Some(top - viewport_y),
                Autoscroll::Center => Some((top + bottom) / 2. - viewport_height / 2.),
                Autoscroll::Bottom => Some(bottom - viewport_height),
            };
            let Some(target) = target else {
                return false;
            };
            let max = lay.layout.max_scroll();
            lay.layout.set_scroll_y(target.clamp(px(0.), max));
            (lay.scroll_y() - scroll_y).abs() > px(0.01)
        })
    }

    /// Prepaint: shapes what is in the viewport and applies the pending
    /// scroll request until both agree (shaping changes heights, which
    /// moves the target).
    pub(crate) fn settle_scroll(&mut self) {
        let Some(height) = self.viewport_bounds().map(|viewport| viewport.size.height) else {
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

    /// The scroll position as a block and an offset into it (what a saved
    /// view stores).
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
