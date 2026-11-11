//! Layout of one block: shaped lines, visual rows, hit testing.
//!
//! A block is shaped with `shape_text` (never `shape_line`: it loses per-run
//! fonts and panics on `\n`, see `docs/gpui-notes.md`). `shape_text` returns
//! one `WrappedLine` per hard line (the text between `\n`s); each wraps into
//! one or more visual [`Row`]s. All offsets in this module are byte offsets
//! into the block's text.
//!
//! Coordinates: x is measured from the left edge of the text column, y from
//! the top of the block. The caller adds the column origin and the block's
//! position in the document.

use std::ops::Range;

use gpui::{Bounds, Pixels, SharedString, TextAlign, WindowTextSystem, WrappedLine, point, px, size};
use writer_core::{Alignment, Block, text};

use super::style::LayoutStyle;

/// Narrowest text box a block is ever wrapped to.
const MIN_TEXT_WIDTH: Pixels = px(40.);
/// Width of the selection stub that stands for a selected line break.
const NEWLINE_SELECTION_WIDTH: Pixels = px(6.);

/// One visual row of a block.
#[derive(Clone, Debug)]
pub(crate) struct Row {
    /// Bytes of the block text on this row. `end` excludes the `\n` that
    /// ends a hard line; for a soft-wrapped row it equals the next row's
    /// start.
    pub(crate) range: Range<usize>,
    /// Index into [`BlockLayout::lines`].
    pub(crate) line: usize,
    /// Block offset at which that hard line starts.
    pub(crate) line_start: usize,
    /// x of the row's first glyph inside the unwrapped line.
    pub(crate) x_start: Pixels,
    /// Advance width of the row.
    pub(crate) width: Pixels,
    /// Top of the row, from the top of the block.
    pub(crate) y: Pixels,
    /// The row ends at a soft wrap, not at the end of its hard line.
    pub(crate) soft_wrapped: bool,
}

/// The layout of one block at one width.
#[derive(Debug)]
pub(crate) struct BlockLayout {
    /// One shaped line per hard line. Empty for void blocks.
    pub(crate) lines: Vec<WrappedLine>,
    /// Visual rows, top to bottom. Never empty.
    pub(crate) rows: Vec<Row>,
    pub(crate) font_size: Pixels,
    pub(crate) line_height: Pixels,
    /// Left edge of the text box, from the column's left edge.
    pub(crate) text_left: Pixels,
    /// Width of the text box (the wrap width).
    pub(crate) text_width: Pixels,
    /// Total height including the spacing above and below.
    pub(crate) height: Pixels,
    pub(crate) align: Alignment,
    /// Divider or page break: no text, one caret position.
    pub(crate) void: bool,
}

impl BlockLayout {
    /// Shapes `block` for a text column `column_width` wide.
    pub(crate) fn shape(
        block: &Block,
        style: &LayoutStyle,
        column_width: Pixels,
        text_system: &WindowTextSystem,
    ) -> Self {
        let metrics = style.metrics(block);
        let text_width =
            (column_width - metrics.text_left - metrics.right_inset).max(MIN_TEXT_WIDTH);
        let mut layout = BlockLayout {
            lines: Vec::new(),
            rows: Vec::new(),
            font_size: metrics.font_size,
            line_height: metrics.line_height,
            text_left: metrics.text_left,
            text_width,
            height: px(0.),
            align: block.alignment(),
            void: block.is_void(),
        };
        let mut y = metrics.space_before;
        if !layout.void {
            let text: SharedString = block.text().to_string().into();
            let runs = style.runs(block);
            layout.lines = text_system
                .shape_text(text, metrics.font_size, &runs, Some(text_width), None)
                .map(|lines| lines.into_vec())
                .unwrap_or_default();
            let mut line_start = 0;
            for (line_ix, line) in layout.lines.iter().enumerate() {
                let unwrapped = &line.unwrapped_layout;
                let mut start = 0;
                let mut x_start = px(0.);
                for boundary in line.wrap_boundaries() {
                    let Some(glyph) = unwrapped
                        .runs
                        .get(boundary.run_ix)
                        .and_then(|run| run.glyphs.get(boundary.glyph_ix))
                    else {
                        continue;
                    };
                    layout.rows.push(Row {
                        range: line_start + start..line_start + glyph.index,
                        line: line_ix,
                        line_start,
                        x_start,
                        width: glyph.position.x - x_start,
                        y,
                        soft_wrapped: true,
                    });
                    start = glyph.index;
                    x_start = glyph.position.x;
                    y += metrics.line_height;
                }
                layout.rows.push(Row {
                    range: line_start + start..line_start + line.len(),
                    line: line_ix,
                    line_start,
                    x_start,
                    width: unwrapped.width - x_start,
                    y,
                    soft_wrapped: false,
                });
                y += metrics.line_height;
                line_start += line.len() + 1;
            }
        }
        if layout.rows.is_empty() {
            // A void block, or a shaping failure: one empty row keeps every
            // caller's "there is always a row" assumption true.
            layout.rows.push(Row {
                range: 0..0,
                line: 0,
                line_start: 0,
                x_start: px(0.),
                width: px(0.),
                y,
                soft_wrapped: false,
            });
            y += metrics.line_height;
        }
        layout.height = y + metrics.space_after;
        layout
    }

    /// GPUI's alignment for painting. Status: skeleton (justify paints as
    /// left-aligned).
    pub(crate) fn text_align(&self) -> TextAlign {
        match self.align {
            Alignment::Left | Alignment::Justify => TextAlign::Left,
            Alignment::Center => TextAlign::Center,
            Alignment::Right => TextAlign::Right,
        }
    }

    /// Horizontal shift of a row inside the text box from its alignment
    /// (matches what `WrappedLine::paint` does).
    fn align_dx(&self, row: &Row) -> Pixels {
        match self.align {
            Alignment::Left | Alignment::Justify => px(0.),
            Alignment::Center => (self.text_width - row.width) / 2.,
            Alignment::Right => self.text_width - row.width,
        }
    }

    /// The row a caret at `offset` is drawn on. An offset at a soft wrap
    /// belongs to the following row (there is no caret affinity).
    pub(crate) fn row_for_offset(&self, offset: usize) -> usize {
        self.rows
            .partition_point(|row| row.range.start <= offset)
            .saturating_sub(1)
    }

    /// The row containing `y` (from the block's top), clamped.
    pub(crate) fn row_at_y(&self, y: Pixels) -> usize {
        self.rows
            .partition_point(|row| row.y <= y)
            .saturating_sub(1)
    }

    /// x of `offset` on row `row_ix`, from the left edge of the text box.
    pub(crate) fn x_in_row(&self, row_ix: usize, offset: usize) -> Pixels {
        let Some(row) = self.rows.get(row_ix) else {
            return px(0.);
        };
        let Some(line) = self.lines.get(row.line) else {
            return px(0.);
        };
        let offset = offset.clamp(row.range.start, row.range.end);
        line.unwrapped_layout.x_for_index(offset - row.line_start) - row.x_start
            + self.align_dx(row)
    }

    /// The caret offset on row `row_ix` closest to `x` (from the left edge
    /// of the text box). On a soft-wrapped row the result stays before the
    /// wrap, so the caret stays on this row.
    pub(crate) fn offset_for_x(&self, row_ix: usize, x: Pixels) -> usize {
        let Some(row) = self.rows.get(row_ix) else {
            return 0;
        };
        let Some(line) = self.lines.get(row.line) else {
            return row.range.start;
        };
        let local_x = x - self.align_dx(row) + row.x_start;
        let mut offset = row.line_start + line.unwrapped_layout.closest_index_for_x(local_x);
        offset = offset.clamp(row.range.start, row.range.end);
        if row.soft_wrapped && offset == row.range.end && row.range.end > row.range.start {
            offset = row.line_start
                + text::prev_grapheme_boundary(&line.text, offset - row.line_start);
            offset = offset.max(row.range.start);
        }
        offset
    }

    /// The caret line box at `offset`: zero width, one row high.
    pub(crate) fn caret_bounds(&self, offset: usize) -> Bounds<Pixels> {
        let row_ix = self.row_for_offset(offset);
        let row = &self.rows[row_ix];
        Bounds::new(
            point(self.text_left + self.x_in_row(row_ix, offset), row.y),
            size(px(0.), self.line_height),
        )
    }

    /// One rectangle per row for the bytes in `range`. A selected line break
    /// (the range continues past the end of a hard line) shows as a short
    /// stub after the row's text. A void block is one rectangle across its
    /// row when `range` is `0..0`.
    pub(crate) fn rects_for_range(&self, range: Range<usize>) -> Vec<Bounds<Pixels>> {
        if self.void {
            let row = &self.rows[0];
            return vec![Bounds::new(
                point(self.text_left, row.y),
                size(self.text_width, self.line_height),
            )];
        }
        let mut rects = Vec::new();
        for (row_ix, row) in self.rows.iter().enumerate() {
            if row.range.start > range.end {
                break;
            }
            let start = range.start.max(row.range.start);
            let end = range.end.min(row.range.end);
            let covers_break =
                !row.soft_wrapped && range.start <= row.range.end && range.end > row.range.end;
            if start > end || (start == end && !covers_break) {
                continue;
            }
            let x0 = self.x_in_row(row_ix, start);
            let mut x1 = self.x_in_row(row_ix, end);
            if covers_break {
                x1 += NEWLINE_SELECTION_WIDTH;
            }
            rects.push(Bounds::new(
                point(self.text_left + x0, row.y),
                size(x1 - x0, self.line_height),
            ));
        }
        rects
    }
}
