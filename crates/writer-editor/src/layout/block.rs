//! Layout of one block: shaped lines, visual rows, hit testing.
//!
//! A block is shaped with `shape_text` (never `shape_line`: it loses per-run
//! fonts and panics on `\n`, see `docs/gpui-notes.md`). `shape_text` returns
//! one line per hard line (the text between `\n`s), unwrapped; the editor
//! breaks each into one or more visual [`Row`]s itself (see [`super::wrap`]
//! for the rules and for why GPUI's own wrapping is not used). All offsets
//! in this module are byte offsets into the block's text.
//!
//! Inline code is shaped at the block's text size, like everything else
//! on its line, and drawn smaller, in a chip with a little padding. A row
//! therefore knows its stretches of inline code ([`Row::code`]): every x
//! this module hands out is where things are drawn, not where the shaper
//! put them.
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
//!
//! # Very long blocks
//!
//! Shaping costs time in proportion to the text, and a block is shaped
//! again whenever it changes, so a paragraph of a megabyte would cost a
//! megabyte of shaping per keystroke. Blocks longer than
//! [`PIECEWISE_MIN`] are therefore shaped in **pieces** of about
//! [`PIECE_BYTES`], each a run of whole rows:
//!
//! - Wrapping is greedy: where a row ends depends only on the text from
//!   the row's start to the first word that does not fit. So a slice of the
//!   text that starts at a row start wraps into the same rows as the whole
//!   text would, except for its last row, which the end of the slice may
//!   have cut short. A piece is a slice minus that last row; the next
//!   slice starts where the dropped row started.
//! - When the block changes, the pieces of the old layout whose text and
//!   fonts are untouched are taken over: those before the change as they
//!   are, those after it as soon as the newly shaped rows meet one of
//!   their starts (rows after an edit fall back into step with the old
//!   ones within a few rows). An edit therefore shapes one or two pieces,
//!   whatever the length of the block.
//! - Sometimes the rows do not fall back into step: in text without
//!   spaces, or with rows that are all full, one more character moves
//!   every row after it. Then the edit stops after [`SEAM_PATIENCE`]
//!   pieces and joins the old pieces at a **seam**: the row before it ends
//!   early, as if at a line break. That is still a consistent layout
//!   (everything that is painted and hit-tested agrees), just not the
//!   one a fresh wrap would give, and the seam is hundreds of rows below
//!   the edit. The background layout then continues the reflow from the
//!   seam ([`Reflow::Mend`]) until the rows meet or the block ends.
//!
//! A side effect: glyph positions inside a piece are small numbers, where
//! positions along one megabyte-long line would run into the millions of
//! pixels and lose their fractions in an `f32`.

use std::{
    collections::BTreeMap,
    ops::{Bound, Range},
    sync::Arc,
};

use gpui::{
    Bounds, Font, Hsla, Pixels, ShapedGlyph, ShapedRun, SharedString, Size, TextRun,
    WindowTextSystem, WrappedLineLayout, point, px, size,
};
use writer_core::{Alignment, Block, BlockKind, ImageId, text};

use super::{
    style::{BlockMetrics, Decorations, Group, LayoutStyle, Marker},
    wrap::{self, INLINE_CODE_PAD, INLINE_CODE_SCALE, WrapGlyph},
};

/// Narrowest text box a block is ever wrapped to.
const MIN_TEXT_WIDTH: f32 = 40.;
/// Width of the selection stub that stands for a selected line break, as a
/// fraction of the font size.
const NEWLINE_STUB: f32 = 0.35;
/// A justified row is left alone when its gaps would grow by more than
/// this many font sizes each.
const MAX_JUSTIFY_STRETCH: f32 = 1.2;
/// Blocks with more text than this are shaped in pieces.
const PIECEWISE_MIN: usize = 32 * 1024;
/// How much text one piece is shaped from.
const PIECE_BYTES: usize = 16 * 1024;
/// And how many hard lines at most: every line is a call to the system's
/// shaper, which costs more than its few dozen characters (code is all
/// short lines).
const PIECE_LINES: usize = 48;
/// How many pieces an edit shapes past the first untouched one while it
/// waits for the new rows to meet the old ones, before it leaves a seam.
const SEAM_PATIENCE: usize = 2;
/// The same for one step of mending in the background.
const MEND_PATIENCE: usize = 8;

/// What shaping a long block does about the rows after a change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reflow {
    /// After an edit: reflow a few pieces, then leave a seam.
    Edit,
    /// In the background: continue the reflow at the first seam.
    Mend,
}

/// The limits of the real app, for a test that wants them.
#[cfg(test)]
pub(crate) const REAL_PIECE_LIMITS: (usize, usize) = (PIECEWISE_MIN, PIECE_BYTES);

#[cfg(test)]
thread_local! {
    /// `(PIECEWISE_MIN, PIECE_BYTES)` for tests. Small by default, so that
    /// ordinary test documents go through the piecewise path.
    pub(crate) static PIECE_LIMITS: std::cell::Cell<(usize, usize)> =
        const { std::cell::Cell::new((200, 150)) };
    /// Bytes of text handed to the shaper so far.
    pub(crate) static SHAPED_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// `(longest block shaped as a whole, bytes per piece)`.
fn piece_limits() -> (usize, usize) {
    #[cfg(test)]
    {
        PIECE_LIMITS.with(|limits| limits.get())
    }
    #[cfg(not(test))]
    {
        (PIECEWISE_MIN, PIECE_BYTES)
    }
}

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
        marker: Marker,
    ) -> Self {
        let metrics = style.metrics(block, marker, column_width);
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

/// One row of [`BlockLayout::row_fingerprint`].
#[cfg(test)]
#[derive(Clone, Debug)]
pub(crate) struct RowPrint {
    pub(crate) range: Range<usize>,
    pub(crate) soft_wrapped: bool,
    /// `(block offset, x)` of every glyph.
    pub(crate) glyphs: Vec<(usize, f32)>,
    pub(crate) y: f32,
}

#[cfg(test)]
impl PartialEq for RowPrint {
    /// Equal up to the rounding of positions measured from different
    /// origins.
    fn eq(&self, other: &Self) -> bool {
        let close = |a: f32, b: f32| (a - b).abs() < 0.05;
        self.range == other.range
            && self.soft_wrapped == other.soft_wrapped
            && close(self.y, other.y)
            && self.glyphs.len() == other.glyphs.len()
            && self
                .glyphs
                .iter()
                .zip(&other.glyphs)
                .all(|(a, b)| a.0 == b.0 && close(a.1, b.1))
    }
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

/// A stretch of inline code on a row: shaped at the block's text size,
/// drawn at [`INLINE_CODE_SCALE`] of it between two paddings.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CodeFragment {
    /// Bytes of the row's hard line (offsets in the line's text).
    pub(crate) range: Range<usize>,
    /// Shaped x of its first glyph, from the row's first glyph.
    start_x: Pixels,
    /// Its shaped width.
    width: Pixels,
    /// How much narrower than shaped the row is drawn up to here (what
    /// the fragments before this one save; a short chip can cost room).
    before: Pixels,
}

impl CodeFragment {
    /// How much narrower than shaped the fragment is drawn.
    fn saving(&self, pad: Pixels) -> Pixels {
        self.width * (1. - INLINE_CODE_SCALE) - pad * 2.
    }
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
    /// The inline code on the row, left to right (empty for most rows).
    pub(crate) code: Vec<CodeFragment>,
}

/// Where the picture of an image block is, in the block's box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ImageBox {
    /// The stored bytes the picture is made of.
    pub(crate) image: ImageId,
    /// Its left edge from the column's left edge: the block's alignment is
    /// in it.
    pub(crate) x: Pixels,
    /// As it is drawn: within the column and within a page's text area
    /// (`LayoutStyle::image_size`).
    pub(crate) size: Size<Pixels>,
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
    /// Divider, page break or image: no text, one caret position.
    pub(crate) void: bool,
    /// The picture of an image block. Its row is as high as the picture
    /// (rounded up to whole points), and the picture stands at the top of
    /// it.
    pub(crate) image: Option<ImageBox>,
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
    /// Padding inside an inline code chip.
    pub(crate) code_pad: Pixels,
    /// Top and height of an inline code chip, from the top of a row
    /// (the ink of the code at the size it is drawn).
    pub(crate) code_box: (Pixels, Pixels),
    /// Sum of the unwrapped line widths: calibrates height estimates.
    pub(crate) natural_width: Pixels,
    /// The pieces of a block that was shaped piecewise (empty otherwise).
    pieces: Vec<Piece>,
    /// What the pieces were shaped from.
    source: Option<Source>,
}

/// A run of whole rows of a block that was shaped in pieces.
#[derive(Clone, Debug)]
pub(crate) struct Piece {
    /// The bytes of the block's text whose rows the piece holds. Starts
    /// and ends at row boundaries.
    range: Range<usize>,
    /// How far the text was shaped to find those rows: the piece and the
    /// row after it that was dropped. The rows are valid while the text
    /// and the fonts of `range.start..slice_end` stay as they are.
    slice_end: usize,
    /// The piece's entries in [`BlockLayout::lines`] and
    /// [`BlockLayout::rows`].
    lines: Range<usize>,
    rows: Range<usize>,
    /// The piece ends at a seam: its last row was cut where the next
    /// piece starts instead of where it would wrap.
    seam: bool,
}

/// What a piecewise layout was shaped from: enough to tell what a later
/// version of the block changed.
#[derive(Debug)]
struct Source {
    text: SharedString,
    runs: Vec<(usize, Font)>,
}

/// The fonts of a block as `(end offset, font)` runs covering its text.
pub(crate) fn font_runs(block: &Block, style: &LayoutStyle) -> Vec<(usize, Font)> {
    let mut end = 0;
    style
        .shape_runs(block)
        .into_iter()
        .map(|run| {
            end += run.len;
            (end, run.font)
        })
        .collect()
}

/// Shapes slices of one block's text.
struct Shaper<'a> {
    text: &'a str,
    runs: &'a [(usize, Font)],
    font_size: Pixels,
    text_width: Pixels,
    ink: Hsla,
    /// The inline code of the block (see `LayoutStyle::code_ranges`).
    code: &'a [Range<usize>],
    text_system: &'a WindowTextSystem,
}

/// The hard lines and the rows of a slice. Row offsets are block offsets;
/// `Row::line` counts from the slice's first line; `Row::y` is not set.
struct Shaped {
    lines: Vec<Line>,
    rows: Vec<Row>,
}

impl Shaper<'_> {
    /// Shapes `text[range]`, which must start at the start of a row.
    fn shape(&self, range: Range<usize>) -> Shaped {
        #[cfg(test)]
        SHAPED_BYTES.with(|bytes| bytes.set(bytes.get() + range.len()));
        let slice: SharedString = self.text[range.clone()].to_string().into();
        // The fonts of the slice. Colours and decorations are painted by
        // the editor itself and are not part of the runs.
        let mut runs = Vec::new();
        let mut run_ix = self.runs.partition_point(|(end, _)| *end <= range.start);
        let mut at = range.start;
        while at < range.end {
            let Some((end, font)) = self.runs.get(run_ix) else {
                break;
            };
            let upto = (*end).min(range.end);
            runs.push(TextRun {
                len: upto - at,
                font: font.clone(),
                color: self.ink,
                background_color: None,
                underline: None,
                strikethrough: None,
            });
            at = upto;
            run_ix += 1;
        }
        // Unwrapped: the rows are broken below, by the editor's own rules
        // and with the widths that are drawn.
        let shaped = self
            .text_system
            .shape_text(slice, self.font_size, &runs, None, None)
            .map(|lines| lines.into_vec())
            .unwrap_or_default();
        let pad = self.font_size * INLINE_CODE_PAD;
        let mut lines = Vec::with_capacity(shaped.len());
        let mut rows = Vec::new();
        let mut line_start = range.start;
        let mut glyphs: Vec<WrapGlyph> = Vec::new();
        let mut positions: Vec<GlyphPos> = Vec::new();
        for (line_ix, line) in shaped.into_iter().enumerate() {
            let layout = Arc::clone(&*line);
            let unwrapped = &layout.unwrapped_layout;
            let length = line.text.len();
            let line_end = GlyphPos {
                run: unwrapped.runs.len(),
                glyph: 0,
            };
            // The inline code that touches this line.
            let first_code = self.code.partition_point(|code| code.end <= line_start);
            let code = &self.code[first_code..];
            let has_code = code
                .first()
                .is_some_and(|code| code.start < line_start + length);
            if !has_code && unwrapped.width <= self.text_width {
                // The common case: one row, nothing to break or squeeze.
                rows.push(Row {
                    range: line_start..line_start + length,
                    line: line_ix,
                    x_start: px(0.),
                    width: unwrapped.width,
                    y: px(0.),
                    soft_wrapped: false,
                    glyphs: (GlyphPos { run: 0, glyph: 0 }, line_end),
                    dx: px(0.),
                    justify: None,
                    code: Vec::new(),
                });
            } else {
                glyphs.clear();
                positions.clear();
                for (run_ix, run) in unwrapped.runs.iter().enumerate() {
                    for (glyph_ix, glyph) in run.glyphs.iter().enumerate() {
                        let offset = line_start + glyph.index;
                        let in_code = has_code && {
                            let at = code.partition_point(|code| code.end <= offset);
                            code.get(at).is_some_and(|code| code.start <= offset)
                        };
                        glyphs.push(WrapGlyph {
                            index: glyph.index,
                            x: glyph.position.x,
                            code: in_code,
                        });
                        positions.push(GlyphPos {
                            run: run_ix,
                            glyph: glyph_ix,
                        });
                    }
                }
                let breaks =
                    wrap::wrap_line(&line.text, &glyphs, unwrapped.width, self.text_width, pad);
                let mut from = 0;
                for cut in breaks.into_iter().chain([glyphs.len()]) {
                    let soft_wrapped = cut < glyphs.len();
                    let (start, x_start, start_pos) = match glyphs.get(from) {
                        Some(glyph) if from > 0 => (glyph.index, glyph.x, positions[from]),
                        _ => (0, px(0.), GlyphPos { run: 0, glyph: 0 }),
                    };
                    let (end, end_x, end_pos) = match glyphs.get(cut) {
                        Some(glyph) => (glyph.index, glyph.x, positions[cut]),
                        None => (length, unwrapped.width, line_end),
                    };
                    rows.push(Row {
                        range: line_start + start..line_start + end,
                        line: line_ix,
                        x_start,
                        width: end_x - x_start,
                        y: px(0.),
                        soft_wrapped,
                        glyphs: (start_pos, end_pos),
                        dx: px(0.),
                        justify: None,
                        code: if has_code {
                            code_fragments(&glyphs[from..cut], end, end_x, x_start, pad)
                        } else {
                            Vec::new()
                        },
                    });
                    from = cut;
                }
            }
            lines.push(Line {
                layout,
                text: line.text.clone(),
                start: line_start,
            });
            line_start += length + 1;
        }
        Shaped { lines, rows }
    }

    /// Shapes a slice of about `piece_bytes` from `pos` (a row start) and
    /// says how many of its rows are whole: all of them when the slice
    /// reaches the end of the text, otherwise all but the last one, which
    /// the end of the slice may have cut short. Returns the shaped slice,
    /// where it ended and the number of whole rows.
    ///
    /// The next slice starts at the first row that is not whole. That row
    /// must not begin with a space: the wrapping would treat it as the
    /// indentation of a new line, which it is not.
    fn piece(&self, pos: usize, piece_bytes: usize) -> (Shaped, usize, usize) {
        let len = self.text.len();
        let mut bytes = piece_bytes.max(1);
        let mut lines = PIECE_LINES;
        loop {
            let mut slice_end = text::ceil_char_boundary(self.text, pos.saturating_add(bytes));
            // Not more hard lines than a piece should have.
            if let Some((at, _)) = self.text.as_bytes()[pos..slice_end]
                .iter()
                .enumerate()
                .filter(|(_, byte)| **byte == b'\n')
                .nth(lines.saturating_sub(1))
            {
                slice_end = pos + at + 1;
            }
            let shaped = self.shape(pos..slice_end);
            if slice_end >= len {
                let whole = shaped.rows.len();
                return (shaped, slice_end, whole);
            }
            let mut whole = shaped.rows.len().saturating_sub(1);
            // (A row that starts a hard line may begin with a space: there
            // it is indentation in the whole text too.)
            while whole > 0
                && shaped.rows[whole - 1].soft_wrapped
                && self.text.as_bytes().get(shaped.rows[whole].range.start) == Some(&b' ')
            {
                whole -= 1;
            }
            if whole > 0 {
                return (shaped, slice_end, whole);
            }
            // One row as long as the slice: look further.
            bytes = bytes.saturating_mul(2);
            lines = lines.saturating_mul(2);
        }
    }
}

/// The stretches of inline code among the glyphs of one row. `end` and
/// `end_x` are the line offset and the shaped x where the row ends,
/// `x_start` the shaped x of its first glyph.
fn code_fragments(
    glyphs: &[WrapGlyph],
    end: usize,
    end_x: Pixels,
    x_start: Pixels,
    pad: Pixels,
) -> Vec<CodeFragment> {
    let mut fragments: Vec<CodeFragment> = Vec::new();
    let mut before = px(0.);
    let mut at = 0;
    while at < glyphs.len() {
        if !glyphs[at].code {
            at += 1;
            continue;
        }
        let mut after = at;
        while after < glyphs.len() && glyphs[after].code {
            after += 1;
        }
        let (range_end, x_end) = glyphs
            .get(after)
            .map_or((end, end_x), |glyph| (glyph.index, glyph.x));
        let fragment = CodeFragment {
            range: glyphs[at].index..range_end,
            start_x: glyphs[at].x - x_start,
            width: x_end - glyphs[at].x,
            before,
        };
        before += fragment.saving(pad);
        fragments.push(fragment);
        at = after;
    }
    fragments
}

/// How many bytes two texts have in common at their start.
fn common_prefix(a: &[u8], b: &[u8]) -> usize {
    const CHUNK: usize = 512;
    let limit = a.len().min(b.len());
    let mut at = 0;
    while at + CHUNK <= limit && a[at..at + CHUNK] == b[at..at + CHUNK] {
        at += CHUNK;
    }
    while at < limit && a[at] == b[at] {
        at += 1;
    }
    at
}

/// How many bytes two texts have in common at their end.
fn common_suffix(a: &[u8], b: &[u8]) -> usize {
    const CHUNK: usize = 512;
    let limit = a.len().min(b.len());
    let mut count = 0;
    while count + CHUNK <= limit
        && a[a.len() - count - CHUNK..a.len() - count]
            == b[b.len() - count - CHUNK..b.len() - count]
    {
        count += CHUNK;
    }
    while count < limit && a[a.len() - 1 - count] == b[b.len() - 1 - count] {
        count += 1;
    }
    count
}

/// How far from the start two sets of font runs agree.
fn runs_prefix(a: &[(usize, Font)], b: &[(usize, Font)], limit: usize) -> usize {
    let (mut i, mut j, mut at) = (0, 0, 0);
    while at < limit {
        let (Some((end_a, font_a)), Some((end_b, font_b))) = (a.get(i), b.get(j)) else {
            break;
        };
        if font_a != font_b {
            break;
        }
        at = (*end_a).min(*end_b);
        if *end_a <= at {
            i += 1;
        }
        if *end_b <= at {
            j += 1;
        }
    }
    at.min(limit)
}

/// How far from the end two sets of font runs agree (the texts they cover
/// are `len_a` and `len_b` long).
fn runs_suffix(
    a: &[(usize, Font)],
    len_a: usize,
    b: &[(usize, Font)],
    len_b: usize,
    limit: usize,
) -> usize {
    let (mut i, mut j, mut at) = (a.len(), b.len(), 0);
    while at < limit && i > 0 && j > 0 {
        if a[i - 1].1 != b[j - 1].1 {
            break;
        }
        // How far from the end each run reaches back.
        let start_a = if i >= 2 { a[i - 2].0 } else { 0 };
        let start_b = if j >= 2 { b[j - 2].0 } else { 0 };
        let reach_a = len_a.saturating_sub(start_a);
        let reach_b = len_b.saturating_sub(start_b);
        at = reach_a.min(reach_b);
        if reach_a <= at {
            i -= 1;
        }
        if reach_b <= at {
            j -= 1;
        }
    }
    at.min(limit)
}

/// The measure of `block` at `column_width`, without keeping its layout.
/// Also returns the sum of its line widths (for calibrating estimates).
pub(crate) fn measure_block(
    block: &Block,
    style: &LayoutStyle,
    column_width: Pixels,
    text_system: &WindowTextSystem,
    marker: Marker,
) -> (Measure, Pixels) {
    let metrics = style.metrics(block, marker, column_width);
    if block.is_void() {
        return (Measure::new(&metrics, block.kind(), 1), px(0.));
    }
    let runs = font_runs(block, style);
    let code = style.code_ranges(block);
    let shaper = Shaper {
        text: block.text(),
        runs: &runs,
        font_size: metrics.font_size,
        text_width: text_width(&metrics, column_width),
        ink: style.colors.ink,
        code: &code,
        text_system,
    };
    let (whole_limit, piece_bytes) = piece_limits();
    let len = shaper.text.len();
    let mut rows = 0;
    let mut natural = px(0.);
    let mut pos = 0;
    loop {
        let (shaped, _, whole) = if len <= whole_limit {
            let shaped = shaper.shape(0..len);
            let whole = shaped.rows.len();
            (shaped, len, whole)
        } else {
            shaper.piece(pos, piece_bytes)
        };
        rows += whole;
        natural = shaped.lines.iter().fold(natural, |sum, line| {
            sum + line.layout.unwrapped_layout.width
        });
        match shaped.rows.get(whole) {
            Some(next) => pos = next.range.start,
            None => break,
        }
    }
    (Measure::new(&metrics, block.kind(), rows), natural)
}

impl BlockLayout {
    /// Shapes `block` for a text column `column_width` wide. `marker` is
    /// the gutter its numbered list needs ([`Marker::NONE`] otherwise).
    /// `previous` is an earlier layout of the same block, if one is at
    /// hand: a very long block takes over what did not change from it
    /// (see the module documentation).
    pub(crate) fn shape(
        block: &Block,
        style: &LayoutStyle,
        column_width: Pixels,
        text_system: &WindowTextSystem,
        marker: Marker,
        previous: Option<&BlockLayout>,
        reflow: Reflow,
    ) -> Self {
        let metrics = style.metrics(block, marker, column_width);
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
            image: block.as_image().map(|image| {
                let size = style.image_size(image, column_width);
                let room = (column_width - size.width).max(px(0.));
                ImageBox {
                    image: image.id,
                    x: match block.alignment() {
                        Alignment::Center => room / 2.,
                        Alignment::Right => room,
                        Alignment::Left | Alignment::Justify => px(0.),
                    },
                    size,
                }
            }),
            measure: Measure::default(),
            decorations: Decorations::default(),
            baseline,
            ascent,
            descent,
            x_height,
            ink: style.colors.ink,
            code_pad: metrics.font_size * INLINE_CODE_PAD,
            code_box: (px(0.), metrics.line_height),
            natural_width: px(0.),
            pieces: Vec::new(),
            source: None,
        };
        // Rows that were shaped now (the others came with their alignment
        // from `previous`).
        let mut fresh: Vec<Range<usize>> = Vec::new();
        if !layout.void {
            layout.decorations = style.decorations(block);
            let runs = font_runs(block, style);
            let code = style.code_ranges(block);
            if !code.is_empty() {
                // The chip is as high as the code is drawn, not as the
                // body text around it.
                let size = metrics.font_size * INLINE_CODE_SCALE;
                let mono = text_system.resolve_font(&style.mono_font);
                let up = text_system.ascent(mono, size);
                let down = text_system.descent(mono, size).abs();
                let pad = (size * 0.1).max(px(1.));
                let top = (baseline - up - pad).max(px(0.));
                let bottom = (baseline + down + pad).min(metrics.line_height);
                layout.code_box = (top, (bottom - top).max(px(1.)));
            }
            let shaper = Shaper {
                text: block.text(),
                runs: &runs,
                font_size: metrics.font_size,
                text_width,
                ink: style.colors.ink,
                code: &code,
                text_system,
            };
            let (whole_limit, piece_bytes) = piece_limits();
            if shaper.text.len() <= whole_limit {
                let shaped = shaper.shape(0..shaper.text.len());
                layout.lines = shaped.lines;
                layout.rows = shaped.rows;
                fresh.push(0..layout.rows.len());
            } else {
                // Rows carry their alignment and sit in a text box of one
                // width at one font size: an old layout is only of use
                // when it agrees on all of that.
                let previous = previous.filter(|old| {
                    old.source.is_some()
                        && old.font_size == layout.font_size
                        && old.text_width == layout.text_width
                        && old.align == layout.align
                });
                fresh = layout.shape_pieces(&shaper, previous, piece_bytes, reflow);
                layout.source = Some(Source {
                    text: shaper.text.to_string().into(),
                    runs,
                });
            }
            layout.natural_width = layout
                .lines
                .iter()
                .fold(px(0.), |sum, line| sum + line.layout.unwrapped_layout.width);
        }
        if layout.rows.is_empty() {
            // A void block, or a shaping failure: one empty row keeps every
            // caller's "there is always a row" assumption true.
            layout.rows.push(Row {
                range: 0..0,
                line: 0,
                x_start: px(0.),
                width: px(0.),
                y: px(0.),
                soft_wrapped: false,
                glyphs: (GlyphPos { run: 0, glyph: 0 }, GlyphPos { run: 0, glyph: 0 }),
                dx: px(0.),
                justify: None,
                code: Vec::new(),
            });
            layout.pieces.clear();
            layout.source = None;
        }
        for (ix, row) in layout.rows.iter_mut().enumerate() {
            row.y = metrics.pad_top + metrics.line_height * ix as f32;
        }
        layout.measure = Measure::new(&metrics, block.kind(), layout.rows.len());
        for rows in fresh {
            layout.align_rows(rows);
        }
        // The row before a seam is short by accident: it is not spread
        // over the column.
        for piece in &layout.pieces {
            if piece.seam
                && let Some(row) = layout.rows.get_mut(piece.rows.end.wrapping_sub(1))
            {
                row.justify = None;
            }
        }
        layout
    }

    /// Whether the block has a seam: a place where an edit stopped
    /// reflowing its rows (see the module documentation).
    pub(crate) fn has_seam(&self) -> bool {
        self.pieces.iter().any(|piece| piece.seam)
    }

    /// Fills `lines`, `rows` and `pieces` for a long text: what `previous`
    /// has of it is taken over, the rest is shaped. Returns the ranges of
    /// the rows that were shaped.
    fn shape_pieces(
        &mut self,
        shaper: &Shaper,
        previous: Option<&BlockLayout>,
        piece_bytes: usize,
        reflow: Reflow,
    ) -> Vec<Range<usize>> {
        let len = shaper.text.len();
        let mut fresh = Vec::new();
        let mut pos = 0;
        // Where (in the new text) an untouched old piece starts: once the
        // new rows reach one of these, the old pieces from there on are
        // the layout of the rest.
        let mut resume: BTreeMap<usize, usize> = BTreeMap::new();
        let mut delta = 0isize;
        let old = previous.and_then(|old| Some((old, old.source.as_ref()?)));
        if let Some((old, source)) = old {
            let (before, after) = (source.text.as_bytes(), shaper.text.as_bytes());
            let limit = before.len().min(after.len());
            let prefix =
                common_prefix(before, after).min(runs_prefix(&source.runs, shaper.runs, limit));
            let suffix = common_suffix(&before[prefix..], &after[prefix..])
                .min(runs_suffix(
                    &source.runs,
                    before.len(),
                    shaper.runs,
                    after.len(),
                    limit,
                ))
                .min(limit - prefix);
            let same_text = prefix == before.len() && before.len() == after.len();
            // Pieces before the change, with everything they were shaped
            // from untouched. The last piece was also shaped from the fact
            // that the text ended there: it only stays when nothing
            // changed at all. Mending starts over at the first seam.
            let kept = old
                .pieces
                .iter()
                .take_while(|piece| {
                    piece.slice_end <= prefix
                        && (piece.slice_end < before.len() || same_text)
                        && !(piece.seam && reflow == Reflow::Mend)
                })
                .count();
            for piece in &old.pieces[..kept] {
                self.adopt(old, piece, 0, len);
            }
            pos = self.pieces.last().map_or(0, |piece| piece.range.end);
            delta = after.len() as isize - before.len() as isize;
            let untouched_from = if same_text { 0 } else { before.len() - suffix };
            for (ix, piece) in old.pieces.iter().enumerate().skip(kept) {
                // (Mending starts at a piece that ends in a seam: taking
                // that very piece over again would mend nothing.)
                let mending_this = reflow == Reflow::Mend && ix == kept;
                if piece.range.start >= untouched_from && !mending_this {
                    resume.insert((piece.range.start as isize + delta) as usize, ix);
                }
            }
        }
        let patience = match reflow {
            Reflow::Edit => SEAM_PATIENCE,
            Reflow::Mend => MEND_PATIENCE,
        };
        // Pieces shaped across the start of an untouched old piece without
        // the rows meeting there.
        let mut missed = 0;
        while pos < len {
            let Some((old, _)) = old else {
                // Nothing to take over: shape to the end.
                let (shaped, slice_end, whole) = shaper.piece(pos, piece_bytes);
                let end = shaped.rows.get(whole).map_or(len, |row| row.range.start);
                let first_row = self.rows.len();
                self.push_piece(shaped, whole, pos..end, slice_end, false);
                fresh.push(first_row..self.rows.len());
                pos = end;
                continue;
            };
            if let Some(ix) = resume.get(&pos) {
                for piece in &old.pieces[*ix..] {
                    self.adopt(old, piece, delta, len);
                }
                break;
            }
            // Out of patience: a seam at the next untouched piece. The
            // text up to there is shaped as if it ended there.
            let next = resume
                .range((Bound::Excluded(pos), Bound::Unbounded))
                .next()
                .map(|(start, ix)| (*start, *ix));
            if missed >= patience
                && let Some((start, ix)) = next
                && start - pos <= piece_bytes.saturating_mul(4)
            {
                let mut shaped = shaper.shape(pos..start);
                let seam = shaper.text.as_bytes()[start - 1] != b'\n';
                let mut whole = shaped.rows.len();
                if seam {
                    if let Some(last) = shaped.rows.last_mut() {
                        last.soft_wrapped = true;
                    }
                } else {
                    // The slice ends with a line break: its last row is
                    // the (still empty) start of the next line, which
                    // belongs to the next piece. No seam there at all.
                    whole = whole.saturating_sub(1);
                }
                let first_row = self.rows.len();
                self.push_piece(shaped, whole, pos..start, start, seam);
                fresh.push(first_row..self.rows.len());
                for piece in &old.pieces[ix..] {
                    self.adopt(old, piece, delta, len);
                }
                break;
            }
            let (shaped, slice_end, whole) = shaper.piece(pos, piece_bytes);
            // The first row start (after this piece's own) at which the
            // old layout can take over. Every row up to the first one
            // that is not whole starts where it would in the full text.
            let mut keep = whole;
            let mut resumed = None;
            for row in 1..=whole.min(shaped.rows.len().saturating_sub(1)) {
                if let Some(ix) = resume.get(&shaped.rows[row].range.start) {
                    keep = row;
                    resumed = Some(*ix);
                    break;
                }
            }
            let end = shaped.rows.get(keep).map_or(len, |row| row.range.start);
            let first_row = self.rows.len();
            self.push_piece(shaped, keep, pos..end, slice_end, false);
            fresh.push(first_row..self.rows.len());
            if let Some(ix) = resumed {
                for piece in &old.pieces[ix..] {
                    self.adopt(old, piece, delta, len);
                }
                break;
            }
            if resume
                .range((Bound::Excluded(pos), Bound::Included(end)))
                .next()
                .is_some()
            {
                missed += 1;
            }
            pos = end;
        }
        // The end of the text. What was taken over may stop short of it in
        // two ways.
        match self.rows.last_mut() {
            // The text now ends where an old piece ended in a seam: that
            // row is the last row of the block, not one that wraps.
            Some(last) if last.soft_wrapped && last.range.end >= len => {
                last.soft_wrapped = false;
                if let Some(piece) = self.pieces.last_mut() {
                    piece.seam = false;
                    piece.slice_end = len;
                }
            }
            // The text now ends with a line break that used to be followed
            // by more: the empty line after it is a row too.
            Some(last) if !last.soft_wrapped && last.range.end + 1 == len => {
                let shaped = shaper.shape(len..len);
                let first_row = self.rows.len();
                let whole = shaped.rows.len();
                self.push_piece(shaped, whole, len..len, len, false);
                fresh.push(first_row..self.rows.len());
            }
            _ => {}
        }
        fresh
    }

    /// Appends the first `keep` rows of a shaped slice as a piece.
    fn push_piece(
        &mut self,
        shaped: Shaped,
        keep: usize,
        range: Range<usize>,
        slice_end: usize,
        seam: bool,
    ) {
        let keep = keep.min(shaped.rows.len());
        if keep == 0 {
            return;
        }
        let line_base = self.lines.len();
        let row_base = self.rows.len();
        let lines = shaped.rows[keep - 1].line + 1;
        self.lines.extend(shaped.lines.into_iter().take(lines));
        self.rows
            .extend(shaped.rows.into_iter().take(keep).map(|mut row| {
                row.line += line_base;
                row
            }));
        self.pieces.push(Piece {
            range,
            slice_end,
            lines: line_base..self.lines.len(),
            rows: row_base..self.rows.len(),
            seam,
        });
    }

    /// Appends a piece of an older layout of the same block, `delta` bytes
    /// further on in a text that is now `len` long.
    fn adopt(&mut self, old: &BlockLayout, piece: &Piece, delta: isize, len: usize) {
        let shift = |offset: usize| (offset as isize + delta).max(0) as usize;
        let line_base = self.lines.len();
        let row_base = self.rows.len();
        self.lines
            .extend(old.lines[piece.lines.clone()].iter().map(|line| Line {
                layout: line.layout.clone(),
                text: line.text.clone(),
                start: shift(line.start),
            }));
        self.rows
            .extend(old.rows[piece.rows.clone()].iter().map(|row| {
                let mut row = row.clone();
                row.range = shift(row.range.start)..shift(row.range.end);
                row.line = row.line - piece.lines.start + line_base;
                if let Some(justify) = &mut row.justify {
                    justify.end = shift(justify.end);
                }
                row
            }));
        self.pieces.push(Piece {
            range: shift(piece.range.start)..shift(piece.range.end),
            slice_end: shift(piece.slice_end).min(len),
            lines: line_base..self.lines.len(),
            rows: row_base..self.rows.len(),
            seam: piece.seam,
        });
    }

    /// Computes the alignment shift and the justification of the rows in
    /// `rows`.
    fn align_rows(&mut self, rows: Range<usize>) {
        if self.align == Alignment::Left || self.void {
            return;
        }
        for row_ix in rows.start..rows.end.min(self.rows.len()) {
            let row = &self.rows[row_ix];
            let Some(line) = self.lines.get(row.line) else {
                continue;
            };
            let local = row.range.start - line.start..row.range.end - line.start;
            let row_text = line.text.get(local.clone()).unwrap_or("");
            // Trailing white space hangs past the edge it is aligned to.
            let trimmed = local.start + row_text.trim_end().len();
            let visible = self.drawn_x(row, trimmed);
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

    /// Where the line-local byte `index` of `row` is drawn, from the row's
    /// first glyph, without alignment or justification: its shaped x,
    /// moved by what the inline code before it saves. A position at
    /// either end of a stretch of code is outside its chip.
    fn drawn_x(&self, row: &Row, index: usize) -> Pixels {
        let shaped = self.natural_x(row, index);
        let mut saved = px(0.);
        for code in &row.code {
            if index <= code.range.start {
                return shaped - code.before;
            }
            if index < code.range.end {
                return code.start_x - code.before
                    + self.code_pad
                    + (shaped - code.start_x) * INLINE_CODE_SCALE;
            }
            saved = code.before + code.saving(self.code_pad);
        }
        shaped - saved
    }

    /// Where the glyph at the line-local byte `index`, shaped at `shaped`
    /// (from the row's first glyph), is drawn, and at what fraction of the
    /// block's text size.
    fn drawn_glyph(&self, row: &Row, index: usize, shaped: Pixels) -> (Pixels, f32) {
        let mut saved = px(0.);
        for code in &row.code {
            if index < code.range.start {
                return (shaped - code.before, 1.);
            }
            if index < code.range.end {
                let x = code.start_x - code.before
                    + self.code_pad
                    + (shaped - code.start_x) * INLINE_CODE_SCALE;
                return (x, INLINE_CODE_SCALE);
            }
            saved = code.before + code.saving(self.code_pad);
        }
        (shaped - saved, 1.)
    }

    /// Shaped x of the line-local byte `index` on `row`, from the row's
    /// first glyph.
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

    /// Calls `f(run, glyph, x, scale)` for every glyph of row `row_ix`,
    /// with the x of the glyph's origin from the left edge of the text box
    /// and the fraction of the block's text size it is drawn at (inline
    /// code is smaller).
    pub(crate) fn for_each_glyph(
        &self,
        row_ix: usize,
        mut f: impl FnMut(&ShapedRun, &ShapedGlyph, Pixels, f32),
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
                let shaped = glyph.position.x - row.x_start;
                let (x, scale) = if row.code.is_empty() {
                    (shaped, 1.)
                } else {
                    self.drawn_glyph(row, glyph.index, shaped)
                };
                f(run, glyph, x + row.dx + shift, scale);
                if let Some(justify) = row.justify
                    && bytes.get(glyph.index) == Some(&b' ')
                    && line.start + glyph.index < justify.end
                {
                    shift += justify.extra;
                }
            }
        }
    }

    /// The row a position at `offset` is on. An offset at a soft wrap
    /// belongs to the row that starts there (see [`Self::row_for_caret`]
    /// for the caret, which can also sit at the end of the row before).
    pub(crate) fn row_for_offset(&self, offset: usize) -> usize {
        self.rows
            .partition_point(|row| row.range.start <= offset)
            .saturating_sub(1)
    }

    /// The row a caret at `offset` is drawn on. An offset at a soft wrap is
    /// both the end of one row and the start of the next: `upstream` picks
    /// the row that ends there. Everywhere else `upstream` changes
    /// nothing.
    pub(crate) fn row_for_caret(&self, offset: usize, upstream: bool) -> usize {
        let row = self.row_for_offset(offset);
        if upstream
            && row > 0
            && self.rows[row].range.start == offset
            && self.rows[row - 1].soft_wrapped
        {
            row - 1
        } else {
            row
        }
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
        let mut x = self.drawn_x(row, index) + row.dx;
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
    /// wrap, so that the position belongs to this row whatever the caret's
    /// affinity (see [`Self::caret_for_x`] for the caret itself).
    pub(crate) fn offset_for_x(&self, row_ix: usize, x: Pixels) -> usize {
        let (offset, upstream) = self.caret_for_x(row_ix, x);
        if !upstream {
            return offset;
        }
        let (Some(row), Some(line)) = (
            self.rows.get(row_ix),
            self.rows
                .get(row_ix)
                .and_then(|row| self.lines.get(row.line)),
        ) else {
            return offset;
        };
        let before = text::prev_grapheme_boundary(&line.text, offset - line.start);
        (line.start + before).max(row.range.start)
    }

    /// The caret on row `row_ix` closest to `x` (from the left edge of the
    /// text box): its offset, and whether it is the end of a soft-wrapped
    /// row. That offset is also the start of the next row, so the caret
    /// needs upstream affinity to be drawn on this one.
    pub(crate) fn caret_for_x(&self, row_ix: usize, x: Pixels) -> (usize, bool) {
        let Some(row) = self.rows.get(row_ix) else {
            return (0, false);
        };
        let Some(line) = self.lines.get(row.line) else {
            return (row.range.start, false);
        };
        let row_end = row.range.end - line.start;
        // The white space a row wraps at hangs past the text box, and a
        // caret in it is drawn at the edge: that is where it is found.
        let edge = if row.soft_wrapped {
            self.text_width
        } else {
            Pixels::MAX
        };
        // Candidate boundaries: every glyph start, then the row's end
        // (which wins a tie: at the edge, the caret goes to the end).
        let mut best = row.range.start - line.start;
        let mut best_distance = None;
        let mut consider = |index: usize, at: Pixels, last: bool| {
            let distance = (at.min(edge) - x).abs();
            if best_distance.is_none_or(|best| distance < best || (last && distance == best)) {
                best_distance = Some(distance);
                best = index;
            }
        };
        self.for_each_glyph(row_ix, |_, glyph, at, _| consider(glyph.index, at, false));
        consider(row_end, self.x_in_row(row_ix, row.range.end), true);
        // Glyphs inside a grapheme cluster are not caret positions.
        let mut index = text::floor_grapheme_boundary(&line.text, best.min(row_end));
        index = index.max(row.range.start - line.start);
        let upstream =
            row.soft_wrapped && index == row_end && row_end > row.range.start - line.start;
        (line.start + index, upstream)
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
        self.for_each_glyph(row_ix, |_, glyph, at, _| {
            if at <= x {
                found = Some(glyph.index);
            }
        });
        found.map(|index| line.start + text::floor_grapheme_boundary(&line.text, index))
    }

    /// The line box of the position at `offset`: zero width, one row high.
    pub(crate) fn caret_bounds(&self, offset: usize) -> (usize, Bounds<Pixels>) {
        self.caret_bounds_on(offset, false)
    }

    /// The line box of a caret at `offset` with the given affinity (see
    /// [`Self::row_for_caret`]). At the end of a soft-wrapped row the
    /// white space the row wraps at hangs past the text box; the caret
    /// stays inside it.
    pub(crate) fn caret_bounds_on(&self, offset: usize, upstream: bool) -> (usize, Bounds<Pixels>) {
        if let Some(picture) = self.image_bounds() {
            // The one position of a picture is its left edge, and as high
            // as the picture: what follows a caret (scrolling, a popup)
            // then follows the picture.
            return (
                0,
                Bounds::new(picture.origin, size(px(0.), picture.size.height)),
            );
        }
        let row_ix = self.row_for_caret(offset, upstream);
        let row = &self.rows[row_ix];
        let mut x = self.x_in_row(row_ix, offset);
        if row.soft_wrapped {
            // The white space a row wraps at hangs past the text box.
            x = x.min(self.text_width);
        }
        (
            row_ix,
            Bounds::new(
                point(self.text_left + x, row.y),
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
        let mut x0 = self.x_in_row(row_ix, start);
        let mut x1 = self.x_in_row(row_ix, end);
        if row.soft_wrapped {
            // The white space a row wraps at hangs past the text box;
            // what is painted for it stays inside.
            x0 = x0.min(self.text_width);
            x1 = x1.min(self.text_width);
        }
        if covers_break {
            x1 += self.font_size * NEWLINE_STUB;
        }
        Some((x0, x1))
    }

    /// The picture of an image block: x from the column's left edge, y from
    /// the top of the block's box.
    pub(crate) fn image_bounds(&self) -> Option<Bounds<Pixels>> {
        let picture = self.image?;
        Some(Bounds::new(
            point(self.text_left + picture.x, self.rows[0].y),
            picture.size,
        ))
    }

    /// One rectangle per row for the bytes in `range`: `(row, rectangle)`,
    /// block-local, one row high. Only rows inside `window` are looked at
    /// (pass `0..usize::MAX` for all). A void block is one rectangle across
    /// its row; an image is its picture.
    pub(crate) fn rects_for_range(
        &self,
        range: Range<usize>,
        window: Range<usize>,
    ) -> Vec<(usize, Bounds<Pixels>)> {
        if let Some(picture) = self.image_bounds() {
            return vec![(0, picture)];
        }
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

    /// Everything about the rows that hit testing and painting use, for
    /// comparing two layouts of the same text: per row its bytes, how it
    /// ends, where it is, and the x of every glyph.
    #[cfg(test)]
    pub(crate) fn row_fingerprint(&self) -> Vec<RowPrint> {
        (0..self.rows.len())
            .map(|row_ix| {
                let row = &self.rows[row_ix];
                let line_start = self.lines.get(row.line).map_or(0, |line| line.start);
                let mut glyphs = Vec::new();
                self.for_each_glyph(row_ix, |_, glyph, x, _| {
                    glyphs.push((line_start + glyph.index, f32::from(x)));
                });
                RowPrint {
                    range: row.range.clone(),
                    soft_wrapped: row.soft_wrapped,
                    glyphs,
                    y: f32::from(row.y),
                }
            })
            .collect()
    }

    /// Whether the rows cover the text without gaps: each one starts
    /// where the one before it ended (after the line break, at the end of
    /// a hard line). Holds for every layout, also one with seams.
    #[cfg(test)]
    pub(crate) fn rows_are_contiguous(&self, len: usize) -> bool {
        let mut at = 0;
        for row in &self.rows {
            if row.range.start != at || row.range.end < row.range.start {
                return false;
            }
            at = row.range.end + usize::from(!row.soft_wrapped);
        }
        at == len + 1
    }

    /// The pieces and the first place where the rows do not follow each
    /// other, for a failing test to print.
    #[cfg(test)]
    pub(crate) fn describe(&self) -> String {
        let mut out = String::new();
        for piece in &self.pieces {
            out.push_str(&format!(
                "piece {:?} shaped to {} rows {:?} lines {:?} seam {}\n",
                piece.range, piece.slice_end, piece.rows, piece.lines, piece.seam
            ));
        }
        let mut at = 0;
        for (ix, row) in self.rows.iter().enumerate() {
            if row.range.start != at {
                out.push_str(&format!(
                    "row {ix} starts at {} instead of {at}: {:?} soft {} after {:?}\n",
                    row.range.start,
                    row.range,
                    row.soft_wrapped,
                    ix.checked_sub(1)
                        .map(|prev| (&self.rows[prev].range, self.rows[prev].soft_wrapped)),
                ));
                break;
            }
            at = row.range.end + usize::from(!row.soft_wrapped);
        }
        for row in self.rows.iter().rev().take(3).rev() {
            out.push_str(&format!(
                "row {:?} soft {} line {}\n",
                row.range, row.soft_wrapped, row.line
            ));
        }
        out
    }

    /// Number of pieces the block was shaped in (0: as a whole).
    #[cfg(test)]
    pub(crate) fn piece_count(&self) -> usize {
        self.pieces.len()
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
