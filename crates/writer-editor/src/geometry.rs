//! Geometry queries: where document positions are on screen, and back.
//!
//! All results are in window coordinates, at the zoom the view has (see
//! `crate::zoom`), and are `None` until the editor was
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

use std::cell::RefCell;

use gpui::{Bounds, Pixels, Point, point, px, size};

use writer_core::{EditorState, Position, TextRange, text};

use crate::{
    config::CaretStyle,
    editor::Editor,
    layout::{BlockLayout, DocumentLayout, Lay, Placed},
};

/// Width of the bar caret.
pub(crate) const CARET_WIDTH: Pixels = px(2.);
/// A range over more blocks than this is only resolved to rectangles near
/// the viewport (see [`Editor::range_bounds`]).
const WIDE_RANGE_BLOCKS: usize = 256;
/// How many blocks around the viewport a wide range still resolves.
const WIDE_RANGE_MARGIN: usize = 64;

/// Runs `f` on `layout`, brought up to date with `state` first. `None`
/// before the first layout. `state` must be the state the layout belongs
/// to (the editor's own); it is a parameter so that this also works while
/// the editor lends the state out mutably (see `Editor::edit_with_rows`).
pub(crate) fn with_layout_of<R>(
    layout: &RefCell<DocumentLayout>,
    state: &EditorState,
    f: impl FnOnce(&mut Lay) -> R,
) -> Option<R> {
    let mut layout = layout.borrow_mut();
    layout.env()?;
    let document = state.document();
    if layout.revision() != state.revision() || layout.count() != document.block_count() {
        // A change that has not come through the mutation seam (yet).
        layout.rebuild(document, state.revision());
    }
    let mut lay = Lay {
        layout: &mut layout,
        document,
    };
    Some(f(&mut lay))
}

impl Editor {
    /// Runs `f` on the up-to-date layout. `None` before the first layout.
    pub(crate) fn with_layout<R>(&self, f: impl FnOnce(&mut Lay) -> R) -> Option<R> {
        with_layout_of(&self.layout, &self.state, f)
    }

    /// The line box at a position: zero width, one row high.
    pub fn position_bounds(&self, position: Position) -> Option<Bounds<Pixels>> {
        let position = self.state.document().clamp_position(position);
        self.with_layout(|lay| {
            let placed = lay.placed(position.block)?;
            Some(lay.view().rect(placed.line_box(position.offset)))
        })
        .flatten()
    }

    /// The caret in the configured [`CaretStyle`]: a thin bar, a block
    /// over the character, or an underline, one row high. It is at
    /// [`Editor::caret_position`]: the selection head, or the position set
    /// with [`Editor::set_caret_override`]. Anchor popups and IME windows
    /// to this. (The caret that is painted is a little shorter than the
    /// row: as tall as the text.)
    pub fn caret_bounds(&self) -> Option<Bounds<Pixels>> {
        let head = self.caret_position();
        let upstream = self.caret_is_upstream();
        let document = self.state.document();
        let block_text = document.block(head.block).map_or("", |block| block.text());
        self.with_layout(|lay| {
            let placed = lay.placed(head.block)?;
            let view = lay.view();
            Some(view.rect(caret_rect(
                &placed,
                block_text,
                head.offset,
                self.config.caret_style,
                upstream,
                view.zoom,
            )))
        })
        .flatten()
    }

    /// Whether the caret has an affinity: a bar caret sits between two
    /// characters, so at a soft wrap it can be at the end of one row or at
    /// the start of the next. A block or underline caret sits on a
    /// character and a caret override is a position someone else chose;
    /// neither has one.
    pub(crate) fn caret_has_affinity(&self) -> bool {
        self.config.caret_style == CaretStyle::Bar && self.caret_override.is_none()
    }

    /// Whether the caret, when it is at a soft wrap, is drawn at the end
    /// of the row that ends there instead of at the start of the next one.
    pub(crate) fn caret_is_upstream(&self) -> bool {
        self.caret_upstream && self.caret_has_affinity()
    }

    /// The line box of the caret: like [`Editor::position_bounds`] of
    /// [`Editor::caret_position`], on the row the caret's affinity puts it
    /// on.
    pub(crate) fn caret_line_bounds(&self) -> Option<Bounds<Pixels>> {
        let head = self.caret_position();
        let upstream = self.caret_is_upstream();
        self.with_layout(|lay| {
            let placed = lay.placed(head.block)?;
            Some(lay.view().rect(placed.caret_box(head.offset, upstream)))
        })
        .flatten()
    }

    /// One rectangle per visual row covered by `range` (empty for an empty
    /// range), each one row high. A painted selection covers the same
    /// rows and, in addition, the space between two blocks it runs
    /// through.
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
                let view = lay.view();
                rects.extend(
                    placed
                        .rects(local.start..end, 0..usize::MAX)
                        .into_iter()
                        .map(|rect| view.rect(rect)),
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
    ///
    /// The result is never the end of a soft-wrapped row (that position is
    /// also the start of the next row, where a caret without an affinity
    /// would be drawn): past the end of such a row it is the position
    /// before the row's last character.
    pub fn position_at_point(&self, point: Point<Pixels>) -> Option<Position> {
        self.hit_test(point, false).map(|(position, _)| position)
    }

    /// Where a click at a window point puts the caret: the position and
    /// whether the caret is upstream there (the point is past the end of a
    /// soft-wrapped row, see [`Editor::caret_is_upstream`]). Without a
    /// caret affinity this is [`Editor::position_at_point`].
    pub(crate) fn caret_at_point(&self, point: Point<Pixels>) -> Option<(Position, bool)> {
        self.hit_test(point, self.caret_has_affinity())
    }

    fn hit_test(&self, point: Point<Pixels>, affine: bool) -> Option<(Position, bool)> {
        let document = self.state.document();
        self.with_layout(|lay| {
            let y = lay.view().layout_y(point.y);
            let index = lay.block_at(y);
            let placed = lay.placed(index)?;
            // Shaping the block may have moved it: ask again.
            let view = lay.view();
            let row = placed.row_at(view.layout_y(point.y));
            let x = view.layout_x(point.x) - placed.layout.text_left;
            let (offset, upstream) = if affine {
                placed.layout.caret_for_x(row, x)
            } else {
                (placed.layout.offset_for_x(row, x), false)
            };
            Some((
                document.clamp_position(Position::new(index, offset)),
                upstream,
            ))
        })
        .flatten()
    }

    /// The position of the character directly under a window point, or
    /// `None` when the point is not over text (in a margin, past the end
    /// of a row, between blocks).
    pub(crate) fn position_under_point(&self, point: Point<Pixels>) -> Option<Position> {
        self.with_layout(|lay| {
            let y = lay.view().layout_y(point.y);
            let index = lay.block_at(y);
            let placed = lay.placed(index)?;
            let view = lay.view();
            let y = view.layout_y(point.y);
            let row = placed.row_at(y);
            let row_top = placed.row_top(row);
            if placed.layout.void || y < row_top || y >= row_top + placed.layout.line_height {
                return None;
            }
            let offset = placed
                .layout
                .offset_under_x(row, view.layout_x(point.x) - placed.layout.text_left)?;
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
            let bottom = lay.view_bottom();
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
    let bottom = lay.view_bottom();
    let first = lay.block_at(top);
    let last = lay.block_at(bottom);
    first..last + 1
}

/// The caret rectangle at `offset` for a caret style: column-local x,
/// content-space y, one row high, in layout units. `upstream` puts a bar
/// caret at a soft wrap on the row that ends there. `zoom` is the zoom of
/// the view the rectangle will be drawn in: the bar and the underline
/// keep their thickness on screen ([`CARET_WIDTH`] window pixels) at
/// every zoom, as an insertion point does everywhere on a Mac.
pub(crate) fn caret_rect(
    placed: &Placed,
    block_text: &str,
    offset: usize,
    style: CaretStyle,
    upstream: bool,
    zoom: f32,
) -> Bounds<Pixels> {
    let line_box = placed.caret_box(offset, upstream && style == CaretStyle::Bar);
    let thickness = CARET_WIDTH / zoom.max(0.01);
    if style == CaretStyle::Bar {
        return Bounds::new(
            point(line_box.origin.x - thickness / 2., line_box.origin.y),
            size(thickness, line_box.size.height),
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
                line_box.origin.y + line_box.size.height - thickness,
            ),
            size(width, thickness),
        ),
        _ => Bounds::new(line_box.origin, size(width, line_box.size.height)),
    }
}

/// The caret as it is painted: [`caret_rect`] reduced to the height of the
/// text (the row's ink), so it does not tower over a line with generous
/// leading. In layout units, like the rectangle it is given.
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
