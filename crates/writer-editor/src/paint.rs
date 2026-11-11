//! Painting one visual row of text.
//!
//! The editor paints glyphs itself instead of calling `WrappedLine::paint`:
//! a row is the unit that moves to the next page, gets justified and is
//! skipped when it is off screen, and GPUI's line painter knows none of
//! that. The glyph positions come from the block layout
//! (`BlockLayout::for_each_glyph`), which is also what hit testing uses, so
//! what is painted and what a click finds cannot drift apart.
//!
//! Colours and decorations are not baked into the shaped text. They come
//! from the block's `Decorations`: text colour per byte range, and marks
//! with a background (highlight, inline code), an underline or a
//! strikethrough.

use std::rc::Rc;

use gpui::{App, Bounds, Hsla, Pixels, Point, Window, fill, point, px, size};

use crate::layout::{BlockLayout, style::MarkShape};

/// A visual row ready to paint.
pub(crate) struct PlannedRow {
    pub(crate) layout: Rc<BlockLayout>,
    pub(crate) row: usize,
    /// Window position of the row's top at the left edge of the text box.
    pub(crate) origin: Point<Pixels>,
}

/// Rounds to half a pixel: whole device pixels on a Retina display, so
/// thin lines stay sharp.
pub(crate) fn snap(value: Pixels) -> Pixels {
    px((f32::from(value) * 2.).round() / 2.)
}

impl PlannedRow {
    /// The marks that touch this row, clipped to it: `(mark index, bytes)`.
    fn marks(&self) -> impl Iterator<Item = (usize, std::ops::Range<usize>)> + '_ {
        let row = &self.layout.rows[self.row];
        let marks = &self.layout.decorations.marks;
        let first = marks.partition_point(|mark| mark.range.end <= row.range.start);
        marks[first..]
            .iter()
            .enumerate()
            .take_while(|(_, mark)| mark.range.start < row.range.end)
            .filter_map(move |(ix, mark)| {
                let start = mark.range.start.max(row.range.start);
                let end = mark.range.end.min(row.range.end);
                (start < end).then_some((first + ix, start..end))
            })
    }

    /// Paints what lies behind the row's text: highlight colours and the
    /// chips of inline code.
    pub(crate) fn paint_backgrounds(&self, window: &mut Window) {
        let layout = &self.layout;
        let (ink_top, ink_height) = layout.ink_box();
        for (ix, range) in self.marks() {
            let Some((color, shape)) = layout.decorations.marks[ix].background else {
                continue;
            };
            let Some((x0, x1)) = layout.span_in_row(self.row, &range) else {
                continue;
            };
            let (grow, radius) = match shape {
                MarkShape::Highlight => (px(0.), px(2.)),
                MarkShape::Code => (px(1.5), px(3.5)),
            };
            window.paint_quad(
                fill(
                    Bounds::new(
                        point(self.origin.x + x0 - grow, self.origin.y + ink_top),
                        size(x1 - x0 + grow * 2., ink_height),
                    ),
                    color,
                )
                .corner_radii(radius),
            );
        }
    }

    /// Paints the row's glyphs, underlines and strikethroughs.
    pub(crate) fn paint_text(&self, window: &mut Window, _cx: &mut App) {
        let layout = &self.layout;
        let row = &layout.rows[self.row];
        let Some(line) = layout.lines.get(row.line) else {
            return;
        };
        let baseline = self.origin.y + layout.baseline;
        let colors = &layout.decorations.colors;
        // Colour runs are walked in step with the glyphs; a glyph that goes
        // backwards (right-to-left text) looks its colour up instead.
        let mut run_ix = colors.partition_point(|(end, _)| *end <= row.range.start);
        let mut run_start = if run_ix == 0 { 0 } else { colors[run_ix - 1].0 };
        let ink = layout.ink;
        layout.for_each_glyph(self.row, |run, glyph, x| {
            let offset = line.start + glyph.index;
            if offset < run_start {
                run_ix = colors.partition_point(|(end, _)| *end <= offset);
                run_start = if run_ix == 0 { 0 } else { colors[run_ix - 1].0 };
            }
            while run_ix < colors.len() && colors[run_ix].0 <= offset {
                run_start = colors[run_ix].0;
                run_ix += 1;
            }
            let color: Hsla = colors.get(run_ix).map_or(ink, |(_, color)| *color);
            let origin = point(self.origin.x + x, baseline);
            if glyph.is_emoji {
                window
                    .paint_emoji(origin, run.font_id, glyph.id, layout.font_size)
                    .ok();
            } else {
                window
                    .paint_glyph(origin, run.font_id, glyph.id, layout.font_size, color)
                    .ok();
            }
        });

        let thickness = px((f32::from(layout.font_size) / 16.).round().max(1.));
        for (ix, range) in self.marks() {
            let mark = &layout.decorations.marks[ix];
            if mark.underline.is_none() && mark.strikethrough.is_none() {
                continue;
            }
            let Some((x0, x1)) = layout.span_in_row(self.row, &range) else {
                continue;
            };
            let mut line_at = |y: Pixels, color: Hsla| {
                window.paint_quad(fill(
                    Bounds::new(point(self.origin.x + x0, snap(y)), size(x1 - x0, thickness)),
                    color,
                ));
            };
            if let Some(color) = mark.underline {
                line_at(baseline + (layout.descent * 0.5).max(px(1.5)), color);
            }
            if let Some(color) = mark.strikethrough {
                line_at(baseline - layout.x_height * 0.5 - thickness / 2., color);
            }
        }
    }
}
