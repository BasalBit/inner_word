//! Rows of text and labels: CoreText's glyphs at the layout's places.
//!
//! The editor's layout decides where every glyph of a row starts
//! ([`TextRow::glyphs`]): alignment, justification and the smaller size of inline
//! code are in those numbers already. This module supplies the outlines. It makes
//! one `CTLine` of the row's text in the fonts of its runs, and draws every glyph
//! of every run of that line, in the run's own font (which is where an emoji or a
//! CJK character comes from when the row's font has none), at the place the
//! layout names for its cluster.
//!
//! With the font the layout shaped with, CoreText gives the glyphs the layout got
//! (it is the same call on the same text), and they land where the screen shows
//! them. With another font the text is still whole and every cluster still starts
//! where the layout put it; only the spacing inside is off, and the row is
//! counted ([`crate::Report::rows_off`]).
//!
//! CoreText counts in UTF-16 code units; the page counts in UTF-8 bytes.
//! [`Units`] is the map between them.

use core_foundation::{
    attributed_string::CFMutableAttributedString,
    base::{CFRange, TCFType},
    string::CFString,
};
use core_graphics::{base::CGFloat, font::CGGlyph, geometry::CGPoint};
use core_text::{
    font::{CTFont, CTFontRef},
    line::CTLine,
    run::CTRun,
    string_attributes::kCTFontAttributeName,
};

use crate::{
    fonts::at_size,
    page::{Anchor, Color, GlyphStart, Label, TextRow},
    pdf::PdfWriter,
};

#[link(name = "CoreText", kind = "framework")]
unsafe extern "C" {
    fn CTFontDrawGlyphs(
        font: CTFontRef,
        glyphs: *const CGGlyph,
        positions: *const CGPoint,
        count: usize,
        context: *mut core_graphics::sys::CGContext,
    );
}

/// A width counts as the layout's when it differs by no more than this many
/// points, or by this share of it.
const WIDTH_SLACK_POINTS: f64 = 0.5;
const WIDTH_SLACK_SHARE: f64 = 0.005;

/// UTF-16 code units of a text to UTF-8 bytes and back.
struct Units {
    /// The byte offset of the character each UTF-16 unit belongs to, and the
    /// length of the text at the end.
    byte_of: Vec<u32>,
}

impl Units {
    fn new(text: &str) -> Self {
        let mut byte_of = Vec::with_capacity(text.len() + 1);
        for (byte, character) in text.char_indices() {
            for _ in 0..character.len_utf16() {
                byte_of.push(byte as u32);
            }
        }
        byte_of.push(text.len() as u32);
        Self { byte_of }
    }

    /// The length of the text in UTF-16 units.
    fn len(&self) -> usize {
        self.byte_of.len() - 1
    }

    /// The byte offset of the character the UTF-16 unit `unit` belongs to.
    fn byte(&self, unit: isize) -> u32 {
        let unit = usize::try_from(unit).unwrap_or(0).min(self.len());
        self.byte_of[unit]
    }

    /// The UTF-16 unit the character at byte `byte` starts at (a byte inside a
    /// character counts as the next one).
    fn unit(&self, byte: usize) -> usize {
        self.byte_of
            .partition_point(|at| (*at as usize) < byte)
            .min(self.len())
    }
}

/// One glyph of a line: what CoreText says about it.
struct LineGlyph {
    glyph: CGGlyph,
    /// Where CoreText puts it, from the line's origin (y up).
    x: f64,
    y: f64,
    /// The byte in the row's text its cluster starts at.
    byte: u32,
}

/// The glyphs of one run of a line and the font they are in.
struct LineRun {
    font: Option<CTFont>,
    glyphs: Vec<LineGlyph>,
}

/// A line of `text`, with `fonts` set on the stretches of bytes they are for
/// (`(end, font)`, consecutive from the start; text behind the last end keeps
/// the last font).
fn shape(text: &str, units: &Units, fonts: &[(usize, CTFont)]) -> (Vec<LineRun>, f64) {
    let mut string = CFMutableAttributedString::new();
    string.replace_str(&CFString::new(text), CFRange::init(0, 0));
    // What the string really holds (it may drop a character it dislikes, a
    // byte order mark for one).
    let length = string.char_len();
    let mut start = 0;
    for (index, (end, font)) in fonts.iter().enumerate() {
        let end = if index + 1 == fonts.len() {
            units.len()
        } else {
            units.unit((*end).min(text.len())).max(start)
        };
        let (from, to) = (start as isize, (end as isize).min(length));
        if to > from {
            // SAFETY: a range inside the string, CoreText's own attribute
            // name and a valid font.
            unsafe {
                string.set_attribute(CFRange::init(from, to - from), kCTFontAttributeName, font);
            }
        }
        start = end;
    }
    let line = CTLine::new_with_attributed_string(string.as_concrete_TypeRef());
    let width = line.get_typographic_bounds().width;
    let runs = line
        .glyph_runs()
        .iter()
        .map(|run| line_run(&run, units))
        .collect();
    (runs, width)
}

fn line_run(run: &CTRun, units: &Units) -> LineRun {
    let font = run.attributes().and_then(|attributes| {
        // SAFETY: CoreText's own attribute name.
        let key = unsafe { kCTFontAttributeName };
        attributes
            .find(key)
            .and_then(|value| value.downcast::<CTFont>())
    });
    let glyphs = run.glyphs();
    let positions = run.positions();
    let indices = run.string_indices();
    let glyphs = glyphs
        .iter()
        .zip(positions.iter())
        .zip(indices.iter())
        .map(|((glyph, position), index)| LineGlyph {
            glyph: *glyph,
            x: position.x,
            y: position.y,
            byte: units.byte(*index),
        })
        .collect();
    LineRun { font, glyphs }
}

/// The glyph starts of a row by byte: one per cluster, the first the layout
/// named for it.
fn starts_by_byte(glyphs: &[GlyphStart]) -> Vec<GlyphStart> {
    let mut starts = glyphs.to_vec();
    // Stable: of two glyphs of one cluster the first stays first.
    starts.sort_by_key(|start| start.byte);
    starts.dedup_by_key(|start| start.byte);
    starts
}

impl PdfWriter {
    /// Draws one row of text and counts it.
    pub(crate) fn draw_row(&mut self, row: &TextRow, page_height: f32) {
        self.report.rows += 1;
        let wanted = f64::from(row.width);
        let slack = WIDTH_SLACK_POINTS.max(wanted.abs() * WIDTH_SLACK_SHARE);
        if row.text.is_empty() || row.runs.is_empty() {
            // Nothing to draw; a row without text is as wide as nothing.
            if wanted.abs() > slack {
                self.report.rows_off += 1;
            }
            return;
        }
        let units = Units::new(&row.text);
        let fonts: Vec<(usize, CTFont)> = row
            .runs
            .iter()
            .map(|run| (run.end, self.fonts.font(&run.font, row.size)))
            .collect();
        let (runs, width) = shape(&row.text, &units, &fonts);
        if (width - wanted).abs() > slack {
            self.report.rows_off += 1;
        }

        // Where the layout starts each cluster, and the CoreText x of the
        // first glyph of each: a glyph keeps its distance from that one.
        let starts = starts_by_byte(&row.glyphs);
        let segment = |byte: u32| {
            starts
                .partition_point(|start| start.byte <= byte)
                .saturating_sub(1)
        };
        let mut anchors: Vec<Option<(u32, f64)>> = vec![None; starts.len()];
        for glyph in runs.iter().flat_map(|run| &run.glyphs) {
            if starts.is_empty() {
                break;
            }
            let anchor = &mut anchors[segment(glyph.byte)];
            if anchor.is_none_or(|(byte, _)| glyph.byte < byte) {
                *anchor = Some((glyph.byte, glyph.x));
            }
        }

        let size = f64::from(row.size);
        let left = f64::from(row.x);
        let baseline = f64::from(page_height) - f64::from(row.baseline);
        // One batch per stretch of glyphs in one colour at one size.
        let mut ids: Vec<CGGlyph> = Vec::new();
        let mut places: Vec<CGPoint> = Vec::new();
        for run in &runs {
            let Some(font) = &run.font else {
                continue;
            };
            // The run's font at the sizes its glyphs are drawn at (inline
            // code is smaller than the row it stands in).
            let mut scaled: Vec<(u32, CTFont)> = Vec::new();
            let mut batch: Option<(usize, u32)> = None;
            for glyph in &run.glyphs {
                let (x, y, scale) = if starts.is_empty() {
                    (left + glyph.x, baseline + glyph.y, 1.)
                } else {
                    let index = segment(glyph.byte);
                    let start = starts[index];
                    let scale = f64::from(start.scale);
                    let anchor = anchors[index].map_or(glyph.x, |(_, x)| x);
                    (
                        left + f64::from(start.x) + (glyph.x - anchor) * scale,
                        baseline + glyph.y * scale,
                        start.scale,
                    )
                };
                let color = row
                    .runs
                    .partition_point(|run| run.end <= glyph.byte as usize)
                    .min(row.runs.len() - 1);
                let key = (color, scale.to_bits());
                if batch != Some(key) {
                    if let Some((color, scale)) = batch {
                        let font = font_at(font, &mut scaled, scale, size);
                        self.draw_glyphs(&font, &ids, &places, row.runs[color].color);
                    }
                    ids.clear();
                    places.clear();
                    batch = Some(key);
                }
                ids.push(glyph.glyph);
                places.push(CGPoint::new(x, y));
            }
            if let Some((color, scale)) = batch {
                let font = font_at(font, &mut scaled, scale, size);
                self.draw_glyphs(&font, &ids, &places, row.runs[color].color);
            }
            ids.clear();
            places.clear();
        }
    }

    /// Draws a single line the writer places itself.
    pub(crate) fn draw_label(&mut self, label: &Label, page_height: f32) {
        if label.text.is_empty() {
            return;
        }
        let units = Units::new(&label.text);
        let font = self.fonts.font(&label.font, label.size);
        let (runs, width) = shape(&label.text, &units, &[(label.text.len(), font)]);
        let left = f64::from(label.x)
            - match label.anchor {
                Anchor::Start => 0.,
                Anchor::Middle => width / 2.,
                Anchor::End => width,
            };
        let baseline = f64::from(page_height) - f64::from(label.baseline);
        for run in &runs {
            let Some(font) = &run.font else {
                continue;
            };
            let ids: Vec<CGGlyph> = run.glyphs.iter().map(|glyph| glyph.glyph).collect();
            let places: Vec<CGPoint> = run
                .glyphs
                .iter()
                .map(|glyph| CGPoint::new(left + glyph.x, baseline + glyph.y))
                .collect();
            self.draw_glyphs(font, &ids, &places, label.color);
        }
    }

    /// Draws glyphs of one font in one colour, each at its own place, and
    /// remembers the font's name.
    fn draw_glyphs(&mut self, font: &CTFont, ids: &[CGGlyph], places: &[CGPoint], color: Color) {
        if ids.is_empty() {
            return;
        }
        self.set_fill(color);
        let Some(context) = &self.context else {
            return;
        };
        // SAFETY: a valid font and context, and two arrays of the same length.
        unsafe {
            CTFontDrawGlyphs(
                font.as_concrete_TypeRef(),
                ids.as_ptr(),
                places.as_ptr(),
                ids.len(),
                context.raw,
            );
        }
        let address = font.as_concrete_TypeRef() as usize;
        if !self.named_fonts.contains_key(&address) {
            // The fonts of a document are a handful; CoreText's fallback fonts
            // may be many objects for one font.
            if self.named_fonts.len() >= 4096 {
                self.named_fonts.clear();
            }
            self.drawn_fonts.insert(font.postscript_name());
            self.named_fonts.insert(address, font.clone());
        }
    }
}

/// `font` at `scale` of `size`: itself at the size it was shaped at, a copy at
/// any other.
fn font_at(font: &CTFont, scaled: &mut Vec<(u32, CTFont)>, scale: u32, size: f64) -> CTFont {
    if scale == 1f32.to_bits() {
        return font.clone();
    }
    if let Some((_, font)) = scaled.iter().find(|(bits, _)| *bits == scale) {
        return font.clone();
    }
    let copy = at_size(font, size * f64::from(f32::from_bits(scale)) as CGFloat);
    scaled.push((scale, copy.clone()));
    copy
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_units_and_utf8_bytes_map_onto_each_other() {
        // a (1 byte, 1 unit), é (2, 1), 漢 (3, 1), 😀 (4, 2), b.
        let text = "aé漢😀b";
        let units = Units::new(text);
        assert_eq!(units.len(), 6);
        assert_eq!(
            (0..=6).map(|unit| units.byte(unit)).collect::<Vec<_>>(),
            vec![0, 1, 3, 6, 6, 10, 11]
        );
        // Out of range on either side is the nearest end.
        assert_eq!(units.byte(-1), 0);
        assert_eq!(units.byte(99), 11);
        for (byte, unit) in [(0, 0), (1, 1), (3, 2), (6, 3), (10, 5), (11, 6)] {
            assert_eq!(units.unit(byte), unit, "byte {byte}");
        }
        // A byte inside a character is the next character.
        assert_eq!(units.unit(2), 2);
        assert_eq!(units.unit(7), 5);
        assert_eq!(units.unit(99), 6);
        assert_eq!(Units::new("").len(), 0);
    }

    #[test]
    fn a_cluster_starts_where_its_first_glyph_was_named() {
        let start = |byte: u32, x: f32| GlyphStart { byte, x, scale: 1. };
        // Right-to-left bytes, and a cluster of two glyphs at byte 4.
        let starts = starts_by_byte(&[start(6, 0.), start(4, 10.), start(4, 12.), start(0, 30.)]);
        assert_eq!(starts, vec![start(0, 30.), start(4, 10.), start(6, 0.)]);
    }
}
