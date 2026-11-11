//! Geometry queries: where document positions are on screen, and back.
//!
//! All results are in window coordinates and are `None` until the editor was
//! laid out once (painted, or measured in compact mode). After that they are
//! always answered for the *current* document: a query first brings the
//! layout up to date (see [`crate::layout`]), so it is valid right after an
//! [`Editor::edit`] in the same handler. Positions outside the viewport give
//! bounds outside [`Editor::viewport_bounds`]; nothing is clipped.

use gpui::{Bounds, Pixels, Point, point, px, size};
use writer_core::{Position, TextRange, text};

use crate::{
    config::CaretStyle,
    editor::Editor,
    layout::{BlockLayout, DocumentLayout, LayoutEnv},
};

/// Width of the bar caret.
pub(crate) const CARET_WIDTH: Pixels = px(2.);

impl Editor {
    /// Runs `f` on the up-to-date layout. `None` before the first layout.
    pub(crate) fn with_layout<R>(
        &self,
        f: impl FnOnce(&DocumentLayout, &LayoutEnv) -> R,
    ) -> Option<R> {
        let mut layout = self.layout.borrow_mut();
        if !layout.sync(self.state.document(), self.state.revision()) {
            return None;
        }
        let layout = &*layout;
        let env = layout.env()?;
        Some(f(layout, env))
    }

    /// Window origin of block `index`'s layout box (its x is the column's
    /// left edge).
    pub(crate) fn block_origin(
        &self,
        layout: &DocumentLayout,
        env: &LayoutEnv,
        index: usize,
    ) -> Point<Pixels> {
        point(
            env.column_left,
            env.viewport.top() + layout.block_top(index) - self.scroll_y,
        )
    }

    /// The line box at a position: zero width, one row high.
    pub fn position_bounds(&self, position: Position) -> Option<Bounds<Pixels>> {
        let position = self.state.document().clamp_position(position);
        self.with_layout(|layout, env| {
            let block = layout.block(position.block)?;
            let origin = self.block_origin(layout, env, position.block);
            Some(shifted(block.caret_bounds(position.offset), origin))
        })
        .flatten()
    }

    /// The caret as it is drawn (the selection head, in the configured
    /// [`CaretStyle`]): a thin bar, a block over the character, or an
    /// underline. Anchor popups and IME windows to this.
    pub fn caret_bounds(&self) -> Option<Bounds<Pixels>> {
        let head = self.state.selection().head;
        let document = self.state.document();
        let block_text = document.block(head.block).map_or("", |block| block.text());
        self.with_layout(|layout, env| {
            let block = layout.block(head.block)?;
            let origin = self.block_origin(layout, env, head.block);
            Some(shifted(
                caret_rect(block, block_text, head.offset, self.config.caret_style),
                origin,
            ))
        })
        .flatten()
    }

    /// One rectangle per visual row covered by `range` (empty for an empty
    /// range). These are the rectangles a selection over the range would
    /// paint.
    pub fn range_bounds(&self, range: TextRange) -> Option<Vec<Bounds<Pixels>>> {
        let document = self.state.document();
        let range = TextRange::new(
            document.clamp_position(range.start),
            document.clamp_position(range.end),
        );
        self.with_layout(|layout, env| {
            let mut rects = Vec::new();
            if range.is_empty() {
                return rects;
            }
            for index in range.start.block..=range.end.block {
                let (Some(block), Some(local)) =
                    (layout.block(index), document.range_in_block(range, index))
                else {
                    continue;
                };
                // A range that goes on into the next block covers this
                // block's final line break.
                let end = if index < range.end.block {
                    local.end + 1
                } else {
                    local.end
                };
                let origin = self.block_origin(layout, env, index);
                rects.extend(
                    block
                        .rects_for_range(local.start..end)
                        .into_iter()
                        .map(|rect| shifted(rect, origin)),
                );
            }
            rects
        })
    }

    /// The smallest rectangle around the selection, or the caret when the
    /// selection is collapsed.
    pub fn selection_bounds(&self) -> Option<Bounds<Pixels>> {
        let selection = self.state.selection();
        if selection.is_collapsed() {
            return self.caret_bounds();
        }
        let rects = self.range_bounds(selection.range())?;
        let mut rects = rects.into_iter();
        let first = rects.next()?;
        Some(rects.fold(first, |all, rect| all.union(&rect)))
    }

    /// The caret position closest to a window point: the block and row
    /// under the point's y (clamped to the document), the nearest character
    /// boundary to its x.
    pub fn position_at_point(&self, point: Point<Pixels>) -> Option<Position> {
        let document = self.state.document();
        self.with_layout(|layout, env| {
            let y = point.y - env.viewport.top() + self.scroll_y;
            let index = layout.block_at_y(y);
            let block = layout.block(index)?;
            let row = block.row_at_y(y - layout.block_top(index));
            let offset = block.offset_for_x(row, point.x - env.column_left - block.text_left);
            Some(document.clamp_position(Position::new(index, offset)))
        })
        .flatten()
    }

    /// The editor's visible area.
    pub fn viewport_bounds(&self) -> Option<Bounds<Pixels>> {
        self.layout.borrow().env().map(|env| env.viewport)
    }

    /// The range of positions on screen: from the start of the first
    /// visible row to the end of the last one, partially visible rows
    /// included.
    pub fn visible_range(&self) -> Option<TextRange> {
        let document = self.state.document();
        self.with_layout(|layout, env| {
            let edge = |y: Pixels, end: bool| {
                let index = layout.block_at_y(y);
                let block = layout.block(index)?;
                let row = &block.rows[block.row_at_y(y - layout.block_top(index))];
                let offset = if end { row.range.end } else { row.range.start };
                Some(document.clamp_position(Position::new(index, offset)))
            };
            let top = self.scroll_y;
            let bottom = top + env.viewport.size.height;
            Some(TextRange::new(edge(top, false)?, edge(bottom, true)?))
        })
        .flatten()
    }
}

/// `rect` (block-local) moved to the block's window origin.
pub(crate) fn shifted(rect: Bounds<Pixels>, origin: Point<Pixels>) -> Bounds<Pixels> {
    Bounds::new(rect.origin + origin, rect.size)
}

/// The caret rectangle at `offset`, block-local, for a caret style.
pub(crate) fn caret_rect(
    block: &BlockLayout,
    block_text: &str,
    offset: usize,
    style: CaretStyle,
) -> Bounds<Pixels> {
    let line_box = block.caret_bounds(offset);
    if style == CaretStyle::Bar {
        return Bounds::new(
            point(line_box.origin.x - CARET_WIDTH / 2., line_box.origin.y),
            size(CARET_WIDTH, line_box.size.height),
        );
    }
    // Block and underline carets cover the character at the caret, or half
    // an em at the end of a row.
    let row_ix = block.row_for_offset(offset);
    let row = &block.rows[row_ix];
    let next = text::next_grapheme_boundary(block_text, offset).min(row.range.end);
    let mut width = block.x_in_row(row_ix, next) - block.x_in_row(row_ix, offset);
    if width <= px(0.) {
        width = block.font_size * 0.5;
    }
    match style {
        CaretStyle::Underline => Bounds::new(
            point(
                line_box.origin.x,
                line_box.origin.y + line_box.size.height - CARET_WIDTH,
            ),
            size(width, CARET_WIDTH),
        ),
        _ => Bounds::new(line_box.origin, size(width, line_box.size.height)),
    }
}
