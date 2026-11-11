//! Layout of one block: shaped lines, visual rows, hit testing.
//!
//! A block is shaped with `shape_text` (never `shape_line`: it loses per-run
//! fonts and panics on `\n`, see `docs/gpui-notes.md`). `shape_text` returns
//! one wrapped line per hard line (the text between `\n`s); each wraps into
//! one or more visual [`Row`]s. All offsets in this module are byte offsets
//! into the block's text.
//!
//! The editor paints rows itself (see `crate::paint`) instead of using
//! `WrappedLine::paint`. That is what makes justified text, rows that move
//! to the next page and per-row culling possible, and it keeps hit testing
//! and painting on one set of numbers: every x in this module already
//! includes the row's alignment shift and justification.
//!
//! Coordinates: x is measured from the left edge of the text box
//! ([`BlockLayout::text_left`] from the column's left edge), y from the top
//! of the block's box. The caller adds the block's position.

use std::{ops::Range, sync::Arc};

use gpui::{
    Bounds, Hsla, Pixels, ShapedGlyph, ShapedRun, SharedString, WindowTextSystem,
    WrappedLineLayout, point, px, size,
};
use writer_core::{Alignment, Block, BlockKind, text};

use super::style::{BlockMetrics, Decorations, Group, LayoutStyle};

/// Narrowest text box a block is ever wrapped to.
const MIN_TEXT_WIDTH: f32 = 40.;
/// Width of the selection stub that stands for a selected line break, as a
/// fraction of the font size.
const NEWLINE_STUB: f32 = 0.35;
/// A justified row is left alone when its gaps would grow by more than
/// this many font sizes each.
const MAX_JUSTIFY_STRETCH: f32 = 1.2;

/// What pagination and the block stack need to know about a block: how
/// many rows of which height, and the space around them. All in layout
/// pixels. Cheap to keep for every block of a book.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Measure {
    pub(crate) rows: u32,
    pub(crate) line_height: f32,
    pub(crate) pad_top: f32,
    pub(crate) pad_bottom: f32,
    pub(crate) space_before: f32,
    pub(crate) space_after: f32,
    pub(crate) group: Group,
    pub(crate) group_gap: f32,
    /// Headings stay on the page of the block that follows them.
    pub(crate) keep_with_next: bool,
    /// The block moves to the next page as a whole when it can.
    pub(crate) keep_together: bool,
    /// The next block starts a new page (page view).
    pub(crate) page_break: bool,
}

impl Default for Measure {
    fn default() -> Self {
        Self {
            rows: 1,
            line_height: 20.,
            pad_top: 0.,
            pad_bottom: 0.,
            space_before: 0.,
            space_after: 0.,
            group: Group::None,
            group_gap: 0.,
            keep_with_next: false,
            keep_together: false,
            page_break: false,
        }
    }
}

impl Measure {
    pub(crate) fn new(metrics: &BlockMetrics, kind: &BlockKind, rows: usize) -> Self {
        let heading = kind.heading_level().is_some();
        Self {
            rows: rows.clamp(1, u32::MAX as usize) as u32,
            line_height: f32::from(metrics.line_height).max(1.),
            pad_top: metrics.pad_top.into(),
            pad_bottom: metrics.pad_bottom.into(),
            space_before: metrics.space_before.into(),
            space_after: metrics.space_after.into(),
            group: metrics.group,
            group_gap: metrics.group_gap.into(),
            keep_with_next: heading,
            keep_together: heading || kind.is_void(),
            page_break: matches!(kind, BlockKind::PageBreak),
        }
    }

    /// Height of the block's box: padding and rows, without outer space.
    pub(crate) fn height(&self) -> f32 {
        self.pad_top + self.rows as f32 * self.line_height + self.pad_bottom
    }

    /// What has to fit on a sheet for the block to start there: all of a
    /// block that stays together or has no more than three rows, otherwise
    /// its first two rows (no single row is left behind at the bottom of a
    /// sheet).
    pub(crate) fn lead(&self) -> f32 {
        if self.keep_together || self.rows <= 3 {
            self.height()
        } else {
            self.pad_top + 2. * self.line_height
        }
    }

    /// A guess at the measure of a block that was never shaped:
    /// `advance` is the average glyph advance per byte and font size.
    pub(crate) fn estimate(
        block: &Block,
        style: &LayoutStyle,
        column_width: Pixels,
        advance: f32,
    ) -> Self {
        let metrics = style.metrics(block);
        let rows = if block.is_void() {
            1
        } else {
            let width = f32::from(text_width(&metrics, column_width)).max(1.);
            let per_byte = (f32::from(metrics.font_size) * advance).max(0.01);
            let fit = (width / per_byte).max(1.);
            let text = block.text();
            if text.len() as f32 <= fit && !block.kind().is_code() {
                // The common case needs no scan: one short line (a soft
                // break in it is rare and only costs one row of error).
                1
            } else {
                text.split('\n')
                    .map(|line| ((line.len() as f32 / fit).ceil() as usize).max(1))
                    .sum()
            }
        };
        Self::new(&metrics, block.kind(), rows)
    }
}

fn text_width(metrics: &BlockMetrics, column_width: Pixels) -> Pixels {
    (column_width - metrics.text_left - metrics.right_inset).max(px(MIN_TEXT_WIDTH))
}

/// One hard line of a block.
#[derive(Debug)]
pub(crate) struct Line {
    pub(crate) layout: Arc<WrappedLineLayout>,
    pub(crate) text: SharedString,
    /// Block offset of the line's first byte.
    pub(crate) start: usize,
}

/// A position in a line's glyph runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct GlyphPos {
    pub(crate) run: usize,
    pub(crate) glyph: usize,
}

/// How a justified row is stretched.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Justify {
    /// Extra advance of every space before `end`.
    pub(crate) extra: Pixels,
    /// Block offset where the row's trailing white space starts.
    pub(crate) end: usize,
}

/// One visual row of a block.
#[derive(Clone, Debug)]
pub(crate) struct Row {
    /// Bytes of the block text on this row. `end` excludes the `\n` that
    /// ends a hard line; for a soft-wrapped row it equals the next row's
    /// start.
    pub(crate) range: Range<usize>,
    /// Index into [`BlockLayout::lines`].
    pub(crate) line: usize,
    /// x of the row's first glyph inside the unwrapped line.
    pub(crate) x_start: Pixels,
    /// Natural advance width of the row, trailing spaces included.
    pub(crate) width: Pixels,
    /// Top of the row, from the top of the block's box.
    pub(crate) y: Pixels,
    /// The row ends at a soft wrap, not at the end of its hard line.
    pub(crate) soft_wrapped: bool,
    /// First glyph of the row and the one after its last.
    pub(crate) glyphs: (GlyphPos, GlyphPos),
    /// Shift from the block's alignment.
    pub(crate) dx: Pixels,
    pub(crate) justify: Option<Justify>,
}

/// The layout of one block at one width.
#[derive(Debug)]
pub(crate) struct BlockLayout {
    /// One entry per hard line. Empty for void blocks.
    pub(crate) lines: Vec<Line>,
    /// Visual rows, top to bottom. Never empty.
    pub(crate) rows: Vec<Row>,
    pub(crate) font_size: Pixels,
    pub(crate) line_height: Pixels,
    /// Left edge of the text box, from the column's left edge.
    pub(crate) text_left: Pixels,
    /// Width of the text box (the wrap width).
    pub(crate) text_width: Pixels,
    pub(crate) align: Alignment,
    /// Divider or page break: no text, one caret position.
    pub(crate) void: bool,
    pub(crate) measure: Measure,
    pub(crate) decorations: Decorations,
    /// The baseline, from the top of a row. One baseline for the whole
    /// block (from the block's own font), so rows with fallback glyphs do
    /// not wobble.
    pub(crate) baseline: Pixels,
    pub(crate) ascent: Pixels,
    pub(crate) descent: Pixels,
    pub(crate) x_height: Pixels,
    /// Colour of text without a colour run (never happens for valid runs).
    pub(crate) ink: Hsla,
    /// Sum of the unwrapped line widths: calibrates height estimates.
    pub(crate) natural_width: Pixels,
}

/// Shapes the hard lines of `block`.
fn shape_lines(
    block: &Block,
    style: &LayoutStyle,
    metrics: &BlockMetrics,
    text_width: Pixels,
    text_system: &WindowTextSystem,
) -> Vec<Line> {
    let text: SharedString = block.text().to_string().into();
    let runs = style.shape_runs(block);
    let shaped = text_system
        .shape_text(text, metrics.font_size, &runs, Some(text_width), None)
        .map(|lines| lines.into_vec())
        .unwrap_or_default();
    let mut start = 0;
    shaped
        .into_iter()
        .map(|line| {
            let entry = Line {
                layout: Arc::clone(&*line),
                text: line.text.clone(),
                start,
            };
            start += line.len() + 1;
            entry
        })
        .collect()
}

/// The measure of `block` at `column_width`, without keeping its layout.
pub(crate) fn measure_block(
    block: &Block,
    style: &LayoutStyle,
    column_width: Pixels,
    text_system: &WindowTextSystem,
) -> (Measure, Pixels) {
    let metrics = style.metrics(block);
    if block.is_void() {
        return (Measure::new(&metrics, block.kind(), 1), px(0.));
    }
    let lines = shape_lines(
        block,
        style,
        &metrics,
        text_width(&metrics, column_width),
        text_system,
    );
    let rows = lines
        .iter()
        .map(|line| line.layout.wrap_boundaries().len() + 1)
        .sum();
    let natural = lines
        .iter()
        .fold(px(0.), |sum, line| sum + line.layout.unwrapped_layout.width);
    (Measure::new(&metrics, block.kind(), rows), natural)
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
        let text_width = text_width(&metrics, column_width);
        let font_id = text_system.resolve_font(&style.block_font(block));
        let ascent = text_system.ascent(font_id, metrics.font_size);
        // Positive on macOS line layouts, negative from font metrics.
        let descent = text_system.descent(font_id, metrics.font_size).abs();
        let x_height = text_system.x_height(font_id, metrics.font_size);
        let baseline = (metrics.line_height - ascent - descent) / 2. + ascent;
        let mut layout = BlockLayout {
            lines: Vec::new(),
            rows: Vec::new(),
            font_size: metrics.font_size,
            line_height: metrics.line_height,
            text_left: metrics.text_left,
            text_width,
            align: block.alignment(),
            void: block.is_void(),
            measure: Measure::default(),
            decorations: Decorations::default(),
            baseline,
            ascent,
            descent,
            x_height,
            ink: style.colors.ink,
            natural_width: px(0.),
        };
        let mut y = metrics.pad_top;
        if !layout.void {
            layout.decorations = style.decorations(block);
            layout.lines = shape_lines(block, style, &metrics, text_width, text_system);
            for (line_ix, line) in layout.lines.iter().enumerate() {
                let unwrapped = &line.layout.unwrapped_layout;
                layout.natural_width += unwrapped.width;
                let mut start = 0;
                let mut start_pos = GlyphPos { run: 0, glyph: 0 };
                let mut x_start = px(0.);
                for boundary in line.layout.wrap_boundaries() {
                    let Some(glyph) = unwrapped
                        .runs
                        .get(boundary.run_ix)
                        .and_then(|run| run.glyphs.get(boundary.glyph_ix))
                    else {
                        continue;
                    };
                    let end_pos = GlyphPos {
                        run: boundary.run_ix,
                        glyph: boundary.glyph_ix,
                    };
                    layout.rows.push(Row {
                        range: line.start + start..line.start + glyph.index,
                        line: line_ix,
                        x_start,
                        width: glyph.position.x - x_start,
                        y,
                        soft_wrapped: true,
                        glyphs: (start_pos, end_pos),
                        dx: px(0.),
                        justify: None,
                    });
                    start = glyph.index;
                    start_pos = end_pos;
                    x_start = glyph.position.x;
                    y += metrics.line_height;
                }
                layout.rows.push(Row {
                    range: line.start + start..line.start + line.text.len(),
                    line: line_ix,
                    x_start,
                    width: unwrapped.width - x_start,
                    y,
                    soft_wrapped: false,
                    glyphs: (
                        start_pos,
                        GlyphPos {
                            run: unwrapped.runs.len(),
                            glyph: 0,
                        },
                    ),
                    dx: px(0.),
                    justify: None,
                });
                y += metrics.line_height;
            }
        }
        if layout.rows.is_empty() {
            // A void block, or a shaping failure: one empty row keeps every
            // caller's "there is always a row" assumption true.
            layout.rows.push(Row {
                range: 0..0,
                line: 0,
                x_start: px(0.),
                width: px(0.),
                y,
                soft_wrapped: false,
                glyphs: (GlyphPos { run: 0, glyph: 0 }, GlyphPos { run: 0, glyph: 0 }),
                dx: px(0.),
                justify: None,
            });
        }
        layout.measure = Measure::new(&metrics, block.kind(), layout.rows.len());
        layout.align_rows();
        layout
    }

    /// Computes each row's alignment shift and justification.
    fn align_rows(&mut self) {
        if self.align == Alignment::Left || self.void {
            return;
        }
        for row_ix in 0..self.rows.len() {
            let row = &self.rows[row_ix];
            let Some(line) = self.lines.get(row.line) else {
                continue;
            };
            let local = row.range.start - line.start..row.range.end - line.start;
            let row_text = line.text.get(local.clone()).unwrap_or("");
            // Trailing white space hangs past the edge it is aligned to.
            let trimmed = local.start + row_text.trim_end().len();
            let visible = self.natural_x(row, trimmed);
            let slack = self.text_width - visible;
            let (dx, justify) = match self.align {
                Alignment::Center => (slack / 2., None),
                Alignment::Right => (slack, None),
                Alignment::Justify if row.soft_wrapped && slack > px(0.) => {
                    let gaps = line.text[local.start..trimmed]
                        .bytes()
                        .filter(|byte| *byte == b' ')
                        .count();
                    let extra = slack / gaps.max(1) as f32;
                    if gaps > 0 && extra <= self.font_size * MAX_JUSTIFY_STRETCH {
                        (
                            px(0.),
                            Some(Justify {
                                extra,
                                end: line.start + trimmed,
                            }),
                        )
                    } else {
                        (px(0.), None)
                    }
                }
                _ => (px(0.), None),
            };
            let row = &mut self.rows[row_ix];
            row.dx = dx.max(px(0.));
            row.justify = justify;
        }
    }

    /// The glyph runs of a row: `(run, glyphs of that run on the row)`.
    fn row_runs<'a>(
        &'a self,
        row: &Row,
    ) -> impl Iterator<Item = (&'a ShapedRun, &'a [ShapedGlyph])> {
        let runs: &[ShapedRun] = self
            .lines
            .get(row.line)
            .map_or(&[], |line| line.layout.unwrapped_layout.runs.as_slice());
        let (first, last) = row.glyphs;
        let end_run = if last.glyph > 0 {
            last.run + 1
        } else {
            last.run
        };
        let end_run = end_run.min(runs.len());
        let first_run = first.run.min(end_run);
        runs[first_run..end_run]
            .iter()
            .enumerate()
            .map(move |(ix, run)| {
                let run_ix = first_run + ix;
                let from = if run_ix == first.run { first.glyph } else { 0 };
                let to = if run_ix == last.run {
                    last.glyph
                } else {
                    run.glyphs.len()
                };
                let to = to.min(run.glyphs.len());
                (run, &run.glyphs[from.min(to)..to])
            })
    }

    /// x of the line-local byte `index` on `row` without alignment or
    /// justification, from the row's first glyph.
    fn natural_x(&self, row: &Row, index: usize) -> Pixels {
        for (_, glyphs) in self.row_runs(row) {
            for glyph in glyphs {
                if glyph.index >= index {
                    return glyph.position.x - row.x_start;
                }
            }
        }
        row.width
    }

    /// Calls `f(run, glyph, x)` for every glyph of row `row_ix`, with the x
    /// of the glyph's origin from the left edge of the text box.
    pub(crate) fn for_each_glyph(
        &self,
        row_ix: usize,
        mut f: impl FnMut(&ShapedRun, &ShapedGlyph, Pixels),
    ) {
        let Some(row) = self.rows.get(row_ix) else {
            return;
        };
        let Some(line) = self.lines.get(row.line) else {
            return;
        };
        let bytes = line.text.as_bytes();
        let mut shift = px(0.);
        for (run, glyphs) in self.row_runs(row) {
            for glyph in glyphs {
                f(run, glyph, glyph.position.x - row.x_start + row.dx + shift);
                if let Some(justify) = row.justify
                    && bytes.get(glyph.index) == Some(&b' ')
                    && line.start + glyph.index < justify.end
                {
                    shift += justify.extra;
                }
            }
        }
    }

    /// The row a caret at `offset` is drawn on. An offset at a soft wrap
    /// belongs to the following row (there is no caret affinity).
    pub(crate) fn row_for_offset(&self, offset: usize) -> usize {
        self.rows
            .partition_point(|row| row.range.start <= offset)
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
        let index = offset - line.start;
        let mut x = self.natural_x(row, index) + row.dx;
        if let Some(justify) = row.justify {
            let end = offset.min(justify.end);
            let spaces = line.text.as_bytes()[row.range.start - line.start..end - line.start]
                .iter()
                .filter(|byte| **byte == b' ')
                .count();
            x += justify.extra * spaces as f32;
        }
        x
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
        let row_end = row.range.end - line.start;
        // Candidate boundaries: every glyph start, then the row's end.
        let mut best = row.range.start - line.start;
        let mut best_distance = None;
        let mut consider = |index: usize, at: Pixels| {
            let distance = (at - x).abs();
            if best_distance.is_none_or(|best| distance < best) {
                best_distance = Some(distance);
                best = index;
            }
        };
        self.for_each_glyph(row_ix, |_, glyph, at| consider(glyph.index, at));
        consider(row_end, self.x_in_row(row_ix, row.range.end));
        // Glyphs inside a grapheme cluster are not caret positions.
        let mut index = text::floor_grapheme_boundary(&line.text, best.min(row_end));
        index = index.max(row.range.start - line.start);
        if row.soft_wrapped && index == row_end && row_end > row.range.start - line.start {
            index =
                text::prev_grapheme_boundary(&line.text, index).max(row.range.start - line.start);
        }
        line.start + index
    }

    /// The offset of the character whose box contains `x` on row `row_ix`,
    /// or `None` when `x` is left or right of the row's text.
    pub(crate) fn offset_under_x(&self, row_ix: usize, x: Pixels) -> Option<usize> {
        let row = self.rows.get(row_ix)?;
        let line = self.lines.get(row.line)?;
        let end_x = self.x_in_row(row_ix, row.range.end);
        if x >= end_x {
            return None;
        }
        let mut found = None;
        self.for_each_glyph(row_ix, |_, glyph, at| {
            if at <= x {
                found = Some(glyph.index);
            }
        });
        found.map(|index| line.start + text::floor_grapheme_boundary(&line.text, index))
    }

    /// The caret line box at `offset`: zero width, one row high.
    pub(crate) fn caret_bounds(&self, offset: usize) -> (usize, Bounds<Pixels>) {
        let row_ix = self.row_for_offset(offset);
        let row = &self.rows[row_ix];
        (
            row_ix,
            Bounds::new(
                point(self.text_left + self.x_in_row(row_ix, offset), row.y),
                size(px(0.), self.line_height),
            ),
        )
    }

    /// The horizontal extent of the bytes of `range` on row `row_ix`, from
    /// the left edge of the text box, or `None` when the row has none of
    /// them. A selected line break (the range continues past the end of a
    /// hard line) adds a short stub after the row's text.
    pub(crate) fn span_in_row(
        &self,
        row_ix: usize,
        range: &Range<usize>,
    ) -> Option<(Pixels, Pixels)> {
        let row = self.rows.get(row_ix)?;
        let start = range.start.max(row.range.start);
        let end = range.end.min(row.range.end);
        let covers_break =
            !row.soft_wrapped && range.start <= row.range.end && range.end > row.range.end;
        if start > end || (start == end && !covers_break) {
            return None;
        }
        let x0 = self.x_in_row(row_ix, start);
        let mut x1 = self.x_in_row(row_ix, end);
        if covers_break {
            x1 += self.font_size * NEWLINE_STUB;
        }
        Some((x0, x1))
    }

    /// One rectangle per row for the bytes in `range`: `(row, rectangle)`,
    /// block-local, one row high. Only rows inside `window` are looked at
    /// (pass `0..usize::MAX` for all). A void block is one rectangle across
    /// its row.
    pub(crate) fn rects_for_range(
        &self,
        range: Range<usize>,
        window: Range<usize>,
    ) -> Vec<(usize, Bounds<Pixels>)> {
        if self.void {
            let row = &self.rows[0];
            return vec![(
                0,
                Bounds::new(
                    point(self.text_left, row.y),
                    size(self.text_width, self.line_height),
                ),
            )];
        }
        let first = self.row_for_offset(range.start).max(window.start);
        let mut rects = Vec::new();
        for row_ix in first..self.rows.len().min(window.end) {
            let row = &self.rows[row_ix];
            if row.range.start > range.end {
                break;
            }
            if let Some((x0, x1)) = self.span_in_row(row_ix, &range) {
                rects.push((
                    row_ix,
                    Bounds::new(
                        point(self.text_left + x0, row.y),
                        size(x1 - x0, self.line_height),
                    ),
                ));
            }
        }
        rects
    }

    /// Top and height of the ink of a row (ascent to descent), from the top
    /// of the row: what backgrounds behind text cover.
    pub(crate) fn ink_box(&self) -> (Pixels, Pixels) {
        let pad = (self.font_size * 0.08).max(px(1.));
        let top = (self.baseline - self.ascent - pad).max(px(0.));
        let bottom = (self.baseline + self.descent + pad).min(self.line_height);
        (top, (bottom - top).max(px(1.)))
    }
}
