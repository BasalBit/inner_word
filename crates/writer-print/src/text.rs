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
//! With the font the layout shaped with, CoreText gives the glyphs the layout got,
//! and they land where the screen shows them. It is the same call on nearly the
//! same text: the layout shaped the whole hard line and cut the row out of it,
//! the writer shapes the row alone, so what depends on the rest of the line (the
//! stop of a tab, a contextual form or kerning at the row's ends) can come out
//! differently, and every cluster is put where the layout put it all the same.
//! With another font the text is still whole and every cluster still starts
//! where the layout put it; only the spacing inside clusters is off, and the
//! row is counted ([`crate::Report::rows_off`], judged by `advances_apart`
//! inside the row and not by the row's width).
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

/// How far CoreText's advances inside a row are from the layout's, in points:
/// the sum over every pair of neighbouring clusters of the difference between
/// the distance the layout put between them and the distance CoreText puts
/// between them. `None` when the row has no pair that can be compared.
///
/// The layout shaped the whole hard line and cut the row out of it; the writer
/// shapes the row alone. In the layout's face the two agree on everything that
/// does not depend on what stands outside the row, so only that is compared,
/// and what depends on more is left out: a tab (its stop is measured from the
/// start of the line), the row's first and last cluster (contextual forms and
/// kerning across the row's ends), a space (justification widens it), and a
/// pair that straddles a change of size (inline code) or that CoreText orders
/// the other way (the row alone may take another direction). A face that is not
/// the layout's moves nearly every pair.
fn advances_apart(
    text: &str,
    starts: &[GlyphStart],
    anchors: &[Option<(u32, f64)>],
) -> Option<f64> {
    if starts.len() < 4 {
        return None;
    }
    let last = starts.len() - 1;
    let is_blank = |index: usize| {
        text.get(starts[index].byte as usize..)
            .and_then(|rest| rest.chars().next())
            .is_none_or(char::is_whitespace)
    };
    // The clusters in the order the layout put them in, left to right.
    let mut order: Vec<usize> = (0..starts.len()).collect();
    order.sort_by(|a, b| starts[*a].x.total_cmp(&starts[*b].x));
    let mut apart = 0.;
    let mut compared = 0;
    for pair in order.windows(2) {
        let (left, right) = (pair[0], pair[1]);
        if [left, right]
            .iter()
            .any(|index| *index == 0 || *index == last || is_blank(*index))
        {
            continue;
        }
        let (start, end) = (starts[left], starts[right]);
        if start.scale != end.scale {
            continue;
        }
        let (Some((_, from)), Some((_, to))) = (anchors[left], anchors[right]) else {
            continue;
        };
        let core_text = (to - from) * f64::from(start.scale);
        if core_text < 0. {
            continue;
        }
        apart += (f64::from(end.x - start.x) - core_text).abs();
        compared += 1;
    }
    (compared > 0).then_some(apart)
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
        let off = match advances_apart(&row.text, &starts, &anchors) {
            Some(apart) => apart > slack,
            // Nothing inside the row to compare: its width, unless a tab
            // makes that depend on where the row stands in its line.
            None => !row.text.contains('\t') && (width - wanted).abs() > slack,
        };
        if off {
            self.report.rows_off += 1;
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
    use std::sync::Arc;

    use super::*;
    use crate::{FontSpec, JobInfo, RowRun, fonts::Fonts, page::Color};

    fn family(name: &str) -> Arc<FontSpec> {
        Arc::new(FontSpec {
            family: name.to_string(),
            weight: 400.,
            italic: false,
            fallbacks: Vec::new(),
        })
    }

    /// The rows of one hard line as the editor's layout makes them on a Mac:
    /// CoreText shapes the whole line in `shaped_in`, the line is wrapped after
    /// a space where it is wider than `room`, and every row is cut out of the
    /// shaped line (its glyphs keep their places, measured from the row's
    /// start). The rows name `named` as their font.
    fn rows_of_a_line(
        line: &str,
        shaped_in: &Arc<FontSpec>,
        named: &Arc<FontSpec>,
        size: f32,
        room: f64,
    ) -> Vec<TextRow> {
        let font = Fonts::new().font(shaped_in, size);
        let units = Units::new(line);
        let (runs, width) = shape(line, &units, &[(line.len(), font)]);
        let glyphs: Vec<&LineGlyph> = runs.iter().flat_map(|run| &run.glyphs).collect();
        // The x a byte starts at: its first glyph's, or the end of the line.
        let x_of = |byte: usize| {
            glyphs
                .iter()
                .filter(|glyph| glyph.byte as usize >= byte)
                .map(|glyph| glyph.x)
                .reduce(f64::min)
                .unwrap_or(width)
        };
        let breaks: Vec<usize> = line
            .char_indices()
            .filter(|(_, character)| *character == ' ')
            .map(|(byte, _)| byte + 1)
            .chain([line.len()])
            .collect();
        let mut rows = Vec::new();
        let mut start = 0;
        while start < line.len() {
            let from = x_of(start);
            let end = breaks
                .iter()
                .copied()
                .filter(|end| *end > start)
                .take_while(|end| x_of(*end) - from <= room)
                .last()
                .unwrap_or_else(|| breaks.iter().copied().find(|end| *end > start).unwrap());
            let starts = glyphs
                .iter()
                .filter(|glyph| (start..end).contains(&(glyph.byte as usize)))
                .map(|glyph| GlyphStart {
                    byte: glyph.byte - start as u32,
                    x: (glyph.x - from) as f32,
                    scale: 1.,
                })
                .collect();
            rows.push(TextRow {
                text: line[start..end].to_string(),
                x: 72.,
                baseline: 100.,
                size,
                runs: vec![RowRun {
                    end: end - start,
                    font: named.clone(),
                    color: Color {
                        r: 0.,
                        g: 0.,
                        b: 0.,
                    },
                }],
                glyphs: starts,
                width: (x_of(end) - from) as f32,
            });
            start = end;
        }
        rows
    }

    /// How many of `rows` the writer counts as off.
    fn rows_off(rows: &[TextRow]) -> usize {
        let mut writer = PdfWriter::new(&JobInfo {
            title: "Rows".to_string(),
            creator: "The Writer 0.0.0".to_string(),
        });
        for row in rows {
            writer.draw_row(row, 842.);
        }
        assert_eq!(writer.report.rows, rows.len());
        writer.report.rows_off
    }

    const PROSE: &str = "Affluent officials flicked through the office files, \
        fifty-five of them, and the fiery waffles were quite effective. \
        The quick brown fox jumps over the lazy dog; Sphinx of black quartz, \
        judge my vow. Typography takes time, and fine kerning AV To Ty Wa \
        is what a careful setter looks for in every line of a long book.";

    #[test]
    fn a_row_cut_out_of_its_line_is_the_layouts_in_the_layouts_face() {
        for name in [
            "Charter",
            "Zapfino",
            "Helvetica Neue",
            "Menlo",
            "Hoefler Text",
        ] {
            let face = family(name);
            let rows = rows_of_a_line(PROSE, &face, &face, 14., 300.);
            assert!(rows.len() > 3, "{name}: {} rows", rows.len());
            assert_eq!(rows_off(&rows), 0, "{name}");
        }
    }

    #[test]
    fn a_tab_on_a_row_that_does_not_start_its_line_is_no_other_face() {
        let line = "build:\tcargo build --release -p writer-app && cargo test \
            -p writer-print\tand then\tthe tabs\tland on\trows further on";
        for name in ["Menlo", "Charter"] {
            let face = family(name);
            let rows = rows_of_a_line(line, &face, &face, 12., 200.);
            assert!(
                rows.iter().skip(1).any(|row| row.text.contains('\t')),
                "{name}: no tab on a later row"
            );
            assert_eq!(rows_off(&rows), 0, "{name}");
        }
    }

    #[test]
    fn rows_measured_in_one_face_and_set_in_another_are_counted() {
        for (measured, set) in [
            ("Charter", "Helvetica Neue"),
            ("Helvetica Neue", "Charter"),
            ("Menlo", "Charter"),
            ("Charter", "Zapfino"),
        ] {
            let rows = rows_of_a_line(PROSE, &family(measured), &family(set), 14., 300.);
            assert_eq!(rows_off(&rows), rows.len(), "{measured} set in {set}");
        }
    }

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
