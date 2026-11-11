//! Scrolling: the scroll offset, scroll requests and autoscroll.
//!
//! The editor scrolls itself (it is not inside a GPUI scroll container):
//! `Editor::scroll_y` is how far the content is moved up, in pixels. A scroll
//! request that needs geometry (bring the caret into view, put a position at
//! the top) is applied at once when a layout exists and otherwise kept until
//! the next frame's prepaint.
//!
//! Status: skeleton for the parts the implementer adds: scroll bar, smooth
//! or animated scrolling, drag-selection autoscroll.

use gpui::{Context, Pixels, px};
use writer_core::Position;

use crate::editor::Editor;

/// Where a position should end up in the viewport after a change or a
/// scroll request.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Autoscroll {
    /// Do not scroll.
    None,
    /// Scroll the least amount that brings the position into view (nothing
    /// when it already is).
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
    /// Bring a position (`None`: the selection head) into view.
    Position(Option<Position>, Autoscroll),
    /// Put the point `offset` pixels below the top of block `block` at the
    /// top of the viewport (restoring a saved view).
    BlockOffset { block: usize, offset: Pixels },
}

/// Space kept between the caret row and the viewport edge by
/// [`Autoscroll::Fit`].
const FIT_MARGIN: Pixels = px(8.);

impl Editor {
    /// How far the content is scrolled, in pixels from its top.
    pub fn scroll_offset(&self) -> Pixels {
        self.scroll_y
    }

    /// Scrolls so the caret (the selection head) is where `autoscroll`
    /// says: [`Autoscroll::Top`], [`Autoscroll::Center`] and
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
        let before = self.scroll_y;
        self.pending_scroll = None;
        self.scroll_y += delta;
        self.clamp_scroll();
        if self.scroll_y != before {
            cx.notify();
        }
    }

    /// Line height of body text: the unit of [`Editor::scroll_by_rows`].
    fn row_height(&self) -> Option<Pixels> {
        self.layout
            .borrow()
            .env()
            .map(|env| env.style.base_line_height())
    }

    /// Records a scroll request and applies it now if a layout exists.
    pub(crate) fn request_scroll(&mut self, request: PendingScroll) {
        if matches!(request, PendingScroll::Position(_, Autoscroll::None)) {
            return;
        }
        self.pending_scroll = Some(request);
        self.flush_scroll();
    }

    /// Applies the pending scroll request if a layout exists. Called by
    /// [`Editor::request_scroll`] and by the element's prepaint.
    pub(crate) fn flush_scroll(&mut self) {
        let Some(request) = self.pending_scroll else {
            return;
        };
        let head = self.state.selection().head;
        let document = self.state.document();
        let scroll_y = self.scroll_y;
        let target = self.with_layout(|layout, env| {
            let viewport_height = env.viewport.size.height;
            match request {
                PendingScroll::BlockOffset { block, offset } => {
                    Some(layout.block_top(block.min(layout.block_count())) + offset)
                }
                PendingScroll::Position(position, mode) => {
                    let position = document.clamp_position(position.unwrap_or(head));
                    let block = layout.block(position.block)?;
                    let row = block.caret_bounds(position.offset);
                    let top = layout.block_top(position.block) + row.origin.y;
                    let bottom = top + row.size.height;
                    let margin = FIT_MARGIN.min(viewport_height / 4.);
                    match mode {
                        Autoscroll::None => None,
                        Autoscroll::Fit => {
                            if top - margin < scroll_y {
                                // The first row of the document shows its
                                // top padding too.
                                let first_row = position.block == 0
                                    && block.row_for_offset(position.offset) == 0;
                                Some(if first_row { px(0.) } else { top - margin })
                            } else if bottom + margin > scroll_y + viewport_height {
                                Some(bottom + margin - viewport_height)
                            } else {
                                None
                            }
                        }
                        Autoscroll::Top => Some(top),
                        Autoscroll::Center => Some((top + bottom) / 2. - viewport_height / 2.),
                        Autoscroll::Bottom => Some(bottom - viewport_height),
                    }
                }
            }
        });
        // No layout yet: keep the request for the first frame.
        let Some(target) = target else {
            return;
        };
        self.pending_scroll = None;
        if let Some(target) = target {
            self.scroll_y = target;
        }
        self.clamp_scroll();
    }

    /// Keeps the scroll offset inside the content.
    pub(crate) fn clamp_scroll(&mut self) {
        let max = self.with_layout(|layout, env| {
            (layout.content_height() - env.viewport.size.height).max(px(0.))
        });
        if let Some(max) = max {
            self.scroll_y = self.scroll_y.clamp(px(0.), max);
        }
    }
}
