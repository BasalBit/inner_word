//! Geometry queries: where document positions are on screen, and back.
//!
//! All results are in window coordinates and are `None` until the editor was
//! laid out once (painted, or measured in compact mode). After that they are
//! always answered for the *current* document: a query shapes the blocks it
//! needs on demand (see [`crate::layout`]), so it is valid right after an
//! [`Editor::edit`] in the same handler and for positions far outside the
//! viewport. Positions outside the viewport give bounds outside
//! [`Editor::viewport_bounds`]; nothing is clipped.
//!
//! Blocks that were never on screen count with an estimated height until
//! the background layout reaches them, so the window y of a position far
//! from the viewport can still move a little. Positions on screen are
//! exact.

use gpui::{Bounds, Pixels, Point, point, px, size};
use writer_core::{Position, TextRange, text};

use crate::{
    config::CaretStyle,
    editor::Editor,
    layout::{BlockLayout, Lay, Placed},
};

/// Width of the bar caret.
pub(crate) const CARET_WIDTH: Pixels = px(2.);
/// A range over more blocks than this is only resolved to rectangles near
/// the viewport (see [`Editor::range_bounds`]).
const WIDE_RANGE_BLOCKS: usize = 256;
/// How many blocks around the viewport a wide range still resolves.
const WIDE_RANGE_MARGIN: usize = 64;

impl Editor {
    /// Runs `f` on the up-to-date layout. `None` before the first layout.
    pub(crate) fn with_layout<R>(&self, f: impl FnOnce(&mut Lay) -> R) -> Option<R> {
        let mut layout = self.layout.borrow_mut();
        layout.env()?;
        let document = self.state.document();
        if layout.revision() != self.state.revision() || layout.count() != document.block_count() {
            // A change that did not come through the mutation seam.
            layout.rebuild(document, self.state.revision());
        }
        let mut lay = Lay {
            layout: &mut layout,
            document,
        };
        Some(f(&mut lay))
    }

    /// The line box at a position: zero width, one row high.
    pub fn position_bounds(&self, position: Position) -> Option<Bounds<Pixels>> {
        let position = self.state.document().clamp_position(position);
        self.with_layout(|lay| {
            let placed = lay.placed(position.block)?;
            Some(shifted(placed.line_box(position.offset), lay.origin()))
        })
        .flatten()
    }

    /// The caret (the selection head) in the configured [`CaretStyle`]: a
    /// thin bar, a block over the character, or an underline, one row
    /// high. Anchor popups and IME windows to this. (The caret that is
    /// painted is a little shorter than the row: as tall as the text.)
    pub fn caret_bounds(&self) -> Option<Bounds<Pixels>> {
        let head = self.state.selection().head;
        let document = self.state.document();
        let block_text = document.block(head.block).map_or("", |block| block.text());
        self.with_layout(|lay| {
            let placed = lay.placed(head.block)?;
            Some(shifted(
                caret_rect(&placed, block_text, head.offset, self.config.caret_style),
                lay.origin(),
            ))
        })
        .flatten()
    }

    /// One rectangle per visual row covered by `range` (empty for an empty
    /// range). These are the rectangles a selection over the range would
    /// paint.
    ///
    /// A range over more than a few hundred blocks is resolved only around
    /// the viewport (the rows a frame could show), so that asking about a
    /// whole book does not shape the whole book.
    pub fn range_bounds(&self, range: TextRange) -> Option<Vec<Bounds<Pixels>>> {
        let document = self.state.document();
        let range = TextRange::new(
            document.clamp_position(range.start),
            document.clamp_position(range.end),
        );
        self.with_layout(|lay| {
            let mut rects = Vec::new();
            if range.is_empty() {
                return rects;
            }
            let mut blocks = range.start.block..range.end.block + 1;
            if blocks.len() > WIDE_RANGE_BLOCKS {
                let visible = visible_blocks(lay);
                blocks = blocks
                    .start
                    .max(visible.start.saturating_sub(WIDE_RANGE_MARGIN))
                    ..blocks.end.min(visible.end + WIDE_RANGE_MARGIN);
            }
            for index in blocks {
                let Some(local) = document.range_in_block(range, index) else {
                    continue;
                };
                let Some(placed) = lay.placed(index) else {
                    continue;
                };
                // A range that goes on into the next block covers this
                // block's final line break.
                let end = if index < range.end.block {
                    local.end + 1
                } else {
                    local.end
                };
                let origin = lay.origin();
                rects.extend(
                    placed
                        .rects(local.start..end, 0..usize::MAX)
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
        let range = selection.range();
        let mut rects = self.range_bounds(range)?.into_iter();
        let first = rects.next()?;
        let mut all = rects.fold(first, |all, rect| all.union(&rect));
        // A selection over very many blocks: its ends are far outside the
        // viewport and were not resolved row by row.
        if range.end.block - range.start.block >= WIDE_RANGE_BLOCKS {
            for position in [range.start, range.end] {
                if let Some(bounds) = self.position_bounds(position) {
                    all = all.union(&bounds);
                }
            }
        }
        Some(all)
    }

    /// The caret position closest to a window point: the block and row
    /// under the point's y (clamped to the document), the nearest character
    /// boundary to its x.
    pub fn position_at_point(&self, point: Point<Pixels>) -> Option<Position> {
        let document = self.state.document();
        self.with_layout(|lay| {
            let y = point.y - lay.origin().y;
            let index = lay.block_at(y);
            let placed = lay.placed(index)?;
            let origin = lay.origin();
            let row = placed.row_at(point.y - origin.y);
            let offset = placed
                .layout
                .offset_for_x(row, point.x - origin.x - placed.layout.text_left);
            Some(document.clamp_position(Position::new(index, offset)))
        })
        .flatten()
    }

    /// The position of the character directly under a window point, or
    /// `None` when the point is not over text (in a margin, past the end
    /// of a row, between blocks).
    pub(crate) fn position_under_point(&self, point: Point<Pixels>) -> Option<Position> {
        self.with_layout(|lay| {
            let y = point.y - lay.origin().y;
            let index = lay.block_at(y);
            let placed = lay.placed(index)?;
            let origin = lay.origin();
            let y = point.y - origin.y;
            let row = placed.row_at(y);
            let row_top = placed.row_top(row);
            if placed.layout.void || y < row_top || y >= row_top + placed.layout.line_height {
                return None;
            }
            let offset = placed
                .layout
                .offset_under_x(row, point.x - origin.x - placed.layout.text_left)?;
            Some(Position::new(index, offset))
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
        self.with_layout(|lay| {
            let top = lay.scroll_y();
            let bottom = top + lay.env().viewport.size.height;
            let mut edge = |y: Pixels, end: bool| {
                let index = lay.block_at(y);
                let placed = lay.placed(index)?;
                let row = &placed.layout.rows[placed.row_at(y)];
                let offset = if end { row.range.end } else { row.range.start };
                Some(document.clamp_position(Position::new(index, offset)))
            };
            Some(TextRange::new(edge(top, false)?, edge(bottom, true)?))
        })
        .flatten()
    }
}

/// The blocks that intersect the viewport.
pub(crate) fn visible_blocks(lay: &mut Lay) -> std::ops::Range<usize> {
    let top = lay.scroll_y();
    let bottom = top + lay.env().viewport.size.height;
    let first = lay.block_at(top);
    let last = lay.block_at(bottom);
    first..last + 1
}

/// `rect` (column-local x, content-space y) moved to the window.
pub(crate) fn shifted(rect: Bounds<Pixels>, origin: Point<Pixels>) -> Bounds<Pixels> {
    Bounds::new(rect.origin + origin, rect.size)
}

/// The caret rectangle at `offset` for a caret style: column-local x,
/// content-space y, one row high.
pub(crate) fn caret_rect(
    placed: &Placed,
    block_text: &str,
    offset: usize,
    style: CaretStyle,
) -> Bounds<Pixels> {
    let line_box = placed.line_box(offset);
    if style == CaretStyle::Bar {
        return Bounds::new(
            point(line_box.origin.x - CARET_WIDTH / 2., line_box.origin.y),
            size(CARET_WIDTH, line_box.size.height),
        );
    }
    // Block and underline carets cover the character at the caret, or half
    // an em at the end of a row.
    let block = &placed.layout;
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

/// The caret as it is painted: [`caret_rect`] reduced to the height of the
/// text (the row's ink), so it does not tower over a line with generous
/// leading.
pub(crate) fn drawn_caret(
    rect: Bounds<Pixels>,
    block: &BlockLayout,
    style: CaretStyle,
) -> Bounds<Pixels> {
    if block.void {
        return rect;
    }
    let (ink_top, ink_height) = block.ink_box();
    match style {
        CaretStyle::Underline => {
            // Under the text rather than at the bottom of the row.
            let row_top = rect.bottom() - block.line_height;
            Bounds::new(
                point(
                    rect.left(),
                    row_top + ink_top + ink_height - rect.size.height,
                ),
                rect.size,
            )
        }
        _ => Bounds::new(
            point(rect.left(), rect.top() + ink_top),
            size(rect.size.width, ink_height),
        ),
    }
}
