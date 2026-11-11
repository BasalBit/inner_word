//! From document attributes to fonts, colours and block metrics.
//!
//! Everything that decides how a block is shaped and spaced lives in
//! [`LayoutStyle`]: a snapshot of the theme, the document typography, the
//! editor mode and the page scale. Two styles with the same *shape*
//! ([`LayoutStyle::same_shape`]) wrap and measure a block identically, so
//! measured heights survive a pure colour change (a theme switch) and are
//! dropped exactly when fonts, sizes or the scale change.
//!
//! All lengths a style hands out are in layout pixels: document points
//! times [`LayoutStyle::scale`]. The scale is 1 everywhere except in page
//! view when the window is narrower than the page.

use std::ops::Range;

use gpui::{Font, FontStyle, FontWeight, Hsla, Pixels, Rgba, TextRun, px};
use writer_core::{Block, BlockKind, InlineAttrs, Typography};
use writer_ui::theme::{self, HighlightColor, Theme, ThemeColors};

/// Horizontal step of one indent level, in points.
pub(crate) const INDENT_STEP: f32 = 24.;
/// Space left of a list item's text for its marker.
pub(crate) const MARKER_GUTTER: f32 = 26.;
/// Space between a list number and the item's text.
pub(crate) const MARKER_GAP: f32 = 7.;
/// Space left of a quote's text (the bar sits in it).
pub(crate) const QUOTE_GUTTER: f32 = 20.;
/// Horizontal inner padding of a code block.
pub(crate) const CODE_PADDING: f32 = 14.;
/// Vertical inner padding of a code block.
const CODE_PADDING_V: f32 = 10.;
/// Line height ratio of compact editors.
const COMPACT_LINE_HEIGHT: f32 = 1.45;

/// Everything shaping depends on besides the block itself and the width.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LayoutStyle {
    /// Compact mode: interface font and size, tight spacing.
    pub(crate) compact: bool,
    /// Body font (document font, or the interface font in compact mode).
    pub(crate) body_font: Font,
    pub(crate) mono_font: Font,
    pub(crate) ui_font: Font,
    pub(crate) colors: ThemeColors,
    pub(crate) typography: Typography,
    /// Layout pixels per document point.
    pub(crate) scale: f32,
    /// Page view: text sits on the page colour instead of the canvas.
    pub(crate) paged: bool,
}

/// Sizes and spacing of one block, in layout pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct BlockMetrics {
    pub(crate) font_size: Pixels,
    pub(crate) line_height: Pixels,
    /// Left edge of the text box, from the column's left edge.
    pub(crate) text_left: Pixels,
    /// Space kept free right of the text box.
    pub(crate) right_inset: Pixels,
    /// Inner padding above the first row and below the last one (part of
    /// the block's box: the code panel).
    pub(crate) pad_top: Pixels,
    pub(crate) pad_bottom: Pixels,
    /// Space outside the box, towards the neighbouring blocks.
    pub(crate) space_before: Pixels,
    pub(crate) space_after: Pixels,
    /// Blocks of the same group sit closer together.
    pub(crate) group: Group,
    /// Space between two neighbours of the same group.
    pub(crate) group_gap: Pixels,
}

/// What the numbers of a numbered list need to fit left of the text: the
/// digit count of the list's largest number and the widths of a digit and
/// of the period, in ems of the body font. All items of a list share one
/// marker, so their text lines up.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Marker {
    /// Digits of the largest number in the list; 0 for lists whose numbers
    /// have one digit, which fit the standard gutter.
    pub(crate) digits: u8,
    pub(crate) digit_em: f32,
    pub(crate) period_em: f32,
}

impl Marker {
    /// The standard gutter.
    pub(crate) const NONE: Self = Self {
        digits: 0,
        digit_em: 0.,
        period_em: 0.,
    };

    /// Digits to keep for a list whose largest number is `count`.
    pub(crate) fn digits_for(count: usize) -> u8 {
        if count < 10 {
            0
        } else {
            count.ilog10() as u8 + 1
        }
    }

    /// Gutter in points for text of `font_points`: at least the standard
    /// one, wider when the widest number of the list needs it.
    fn gutter(&self, font_points: f32) -> f32 {
        if self.digits == 0 {
            return MARKER_GUTTER;
        }
        let label = (self.digits as f32 * self.digit_em + self.period_em) * font_points;
        MARKER_GUTTER.max((label + MARKER_GAP).ceil())
    }
}

/// Runs of neighbouring blocks that read as one unit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Group {
    #[default]
    None,
    List,
    Quote,
}

/// How a background behind a span is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum MarkShape {
    /// A highlight: the ink box of the row.
    Highlight,
    /// Inline code: a rounded chip around the text.
    Code,
}

/// A decorated span of a block: background, underline, strikethrough.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Mark {
    pub(crate) range: Range<usize>,
    pub(crate) background: Option<(Hsla, MarkShape)>,
    pub(crate) underline: Option<Hsla>,
    pub(crate) strikethrough: Option<Hsla>,
}

/// Colours and decorations of a block's text, by byte offset.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Decorations {
    /// `(end offset, colour)` of each colour run, in order. Covers the
    /// whole text.
    pub(crate) colors: Vec<(usize, Hsla)>,
    /// Spans that paint more than glyphs, in order.
    pub(crate) marks: Vec<Mark>,
}

impl LayoutStyle {
    pub(crate) fn new(
        theme: &Theme,
        typography: &Typography,
        compact: bool,
        paged: bool,
        scale: f32,
    ) -> Self {
        let document_font = theme.document_font();
        let body_font = if compact {
            theme.ui_font()
        } else if typography.body_font.is_empty()
            || typography.body_font == Typography::default().body_font
        {
            // The default body font goes through the theme's resolved stack
            // (Charter, falling back to what is installed).
            document_font
        } else {
            Font {
                family: typography.body_font.clone().into(),
                ..document_font
            }
        };
        Self {
            compact,
            body_font,
            mono_font: theme.mono_font(),
            ui_font: theme.ui_font(),
            colors: theme.colors,
            typography: typography.clone(),
            scale: if scale.is_finite() && scale > 0. {
                scale
            } else {
                1.
            },
            paged,
        }
    }

    /// Whether `other` wraps and measures every block exactly as this style
    /// does (only colours may differ).
    pub(crate) fn same_shape(&self, other: &Self) -> bool {
        self.compact == other.compact
            && self.body_font == other.body_font
            && self.mono_font == other.mono_font
            && self.typography == other.typography
            && self.scale == other.scale
            && self.paged == other.paged
    }

    /// Document points to layout pixels.
    pub(crate) fn px(&self, points: f32) -> Pixels {
        px(points * self.scale)
    }

    /// The surface the text sits on.
    pub(crate) fn surface(&self) -> Hsla {
        if self.paged {
            self.colors.page
        } else {
            self.colors.canvas
        }
    }

    /// Body text size in points (the size of a plain paragraph).
    fn base_points(&self) -> f32 {
        if self.compact {
            f32::from(theme::UI_TEXT_BASE)
        } else {
            self.typography.base_size
        }
    }

    /// Line height of a plain paragraph: the unit of row-wise scrolling.
    pub(crate) fn base_line_height(&self) -> Pixels {
        self.px(self.line_points(self.base_points(), false))
    }

    /// Line height in whole points for a text size.
    fn line_points(&self, font_points: f32, heading: bool) -> f32 {
        let ratio = if self.compact {
            COMPACT_LINE_HEIGHT
        } else if heading {
            // Headings are set tighter than body text; the rule is the
            // document model's, so that an export can follow it.
            self.typography
                .line_height
                .min(writer_core::HEADING_MAX_LINE_HEIGHT)
        } else {
            self.typography.line_height
        };
        (font_points * ratio).round().max(1.)
    }

    /// The metrics of `block`. `marker` widens the gutter of a numbered
    /// item to the widest number of its list ([`Marker::NONE`]: the
    /// standard gutter).
    pub(crate) fn metrics(&self, block: &Block, marker: Marker) -> BlockMetrics {
        let kind = block.kind();
        let heading = kind.heading_level().filter(|_| !self.compact);
        let font_points = if self.compact {
            self.base_points()
        } else {
            self.typography.size_of(block)
        };
        let base_line = self.line_points(self.base_points(), false);
        let mut line_points = self.line_points(font_points, heading.is_some());
        let spacing = if self.compact {
            0.
        } else {
            self.typography.paragraph_spacing.round()
        };
        let mut text_left = INDENT_STEP * block.indent() as f32;
        let mut right_inset = 0.;
        let (mut pad_top, mut pad_bottom) = (0., 0.);
        let mut space_before = 0.;
        let mut space_after = spacing;
        let mut group = Group::None;
        let mut group_gap = 0.;
        match kind {
            BlockKind::Heading { level } if !self.compact => {
                // More air above a heading than below: it belongs to what
                // follows.
                let (before, after) = match level {
                    1 => (0.9, 0.5),
                    2 => (0.9, 0.35),
                    3 => (0.75, 0.3),
                    _ => (0.6, 0.3),
                };
                space_before = (base_line * before).round();
                space_after = (base_line * after).round();
            }
            BlockKind::BulletItem | BlockKind::NumberedItem | BlockKind::TodoItem { .. } => {
                text_left += if matches!(kind, BlockKind::NumberedItem) {
                    marker.gutter(font_points)
                } else {
                    MARKER_GUTTER
                };
                group = Group::List;
                group_gap = (spacing * 0.33).round();
            }
            BlockKind::Quote => {
                text_left += QUOTE_GUTTER;
                group = Group::Quote;
                group_gap = (spacing * 0.5).round();
            }
            BlockKind::CodeBlock { .. } => {
                text_left += CODE_PADDING;
                right_inset = CODE_PADDING;
                pad_top = CODE_PADDING_V;
                pad_bottom = CODE_PADDING_V;
                if !self.compact {
                    space_before = 4.;
                    space_after = spacing + 4.;
                }
            }
            BlockKind::Divider => {
                // A short row with the rule in its middle.
                line_points = (base_line * 0.6).round().max(1.);
            }
            BlockKind::PageBreak => line_points = base_line,
            _ => {}
        }
        BlockMetrics {
            font_size: self.px(font_points),
            line_height: self.px(line_points),
            text_left: self.px(text_left),
            right_inset: self.px(right_inset),
            pad_top: self.px(pad_top),
            pad_bottom: self.px(pad_bottom),
            space_before: self.px(space_before),
            space_after: self.px(space_after),
            group,
            group_gap: self.px(group_gap),
        }
    }

    /// The font a block's metrics (baseline, x-height) come from.
    pub(crate) fn block_font(&self, block: &Block) -> Font {
        if block.kind().is_code() {
            self.mono_font.clone()
        } else {
            self.body_font.clone()
        }
    }

    /// The font of a span.
    pub(crate) fn font_for(&self, block: &Block, attrs: &InlineAttrs) -> Font {
        let mut font = if attrs.code || block.kind().is_code() {
            self.mono_font.clone()
        } else if let Some(family) = attrs.font.as_ref().filter(|_| !self.compact) {
            Font {
                family: family.clone().into(),
                ..self.body_font.clone()
            }
        } else {
            self.body_font.clone()
        };
        if attrs.bold || (block.kind().heading_level().is_some() && !self.compact) {
            font.weight = FontWeight::BOLD;
        }
        if attrs.italic {
            font.style = FontStyle::Italic;
        }
        font
    }

    /// The text colour of a span.
    pub(crate) fn color_for(&self, block: &Block, attrs: &InlineAttrs) -> Hsla {
        let checked = matches!(block.kind(), BlockKind::TodoItem { checked: true });
        if attrs.link.is_some() && !checked {
            return self.colors.accent;
        }
        if let Some(rgb) = attrs.color.filter(|_| !checked) {
            return Rgba {
                r: rgb.r as f32 / 255.,
                g: rgb.g as f32 / 255.,
                b: rgb.b as f32 / 255.,
                a: 1.,
            }
            .into();
        }
        match block.kind() {
            BlockKind::TodoItem { checked: true } => self.colors.faint,
            BlockKind::Quote => self.colors.muted,
            _ => self.colors.ink,
        }
    }

    /// The runs `shape_text` needs: one per attribute span, fonts only. The
    /// lengths cover the whole text, soft line breaks included. Colours and
    /// decorations are painted by the editor itself (see [`Decorations`]),
    /// so they are left out here, which also keeps GPUI's layout cache key
    /// independent of the theme.
    pub(crate) fn shape_runs(&self, block: &Block) -> Vec<TextRun> {
        let mut runs: Vec<TextRun> = Vec::new();
        for (range, attrs) in block.spans() {
            let font = self.font_for(block, attrs);
            match runs.last_mut() {
                Some(last) if last.font == font => last.len += range.len(),
                _ => runs.push(TextRun {
                    len: range.len(),
                    font,
                    color: self.colors.ink,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                }),
            }
        }
        runs
    }

    /// The byte ranges of inline code in a block, in order, neighbours
    /// merged. Inline code is drawn smaller than it is shaped and in a
    /// chip (see `layout::wrap`); a code block is not inline code, and a
    /// compact editor holds plain text.
    pub(crate) fn code_ranges(&self, block: &Block) -> Vec<Range<usize>> {
        let mut ranges: Vec<Range<usize>> = Vec::new();
        if self.compact || block.kind().is_code() {
            return ranges;
        }
        for (range, attrs) in block.spans() {
            if !attrs.code || range.is_empty() {
                continue;
            }
            match ranges.last_mut() {
                Some(last) if last.end == range.start => last.end = range.end,
                _ => ranges.push(range),
            }
        }
        ranges
    }

    /// Colours, backgrounds, underlines and strikethroughs of a block.
    pub(crate) fn decorations(&self, block: &Block) -> Decorations {
        let checked = matches!(block.kind(), BlockKind::TodoItem { checked: true });
        let mut decorations = Decorations::default();
        for (range, attrs) in block.spans() {
            let color = self.color_for(block, attrs);
            match decorations.colors.last_mut() {
                Some((end, last)) if *last == color => *end = range.end,
                _ => decorations.colors.push((range.end, color)),
            }
            let background = attrs
                .highlight
                .and_then(|highlight| HighlightColor::parse(highlight.name()))
                .map(|highlight| (self.colors.highlight.get(highlight), MarkShape::Highlight))
                .or_else(|| {
                    (attrs.code && !block.kind().is_code())
                        .then_some((self.colors.hover, MarkShape::Code))
                });
            let underline = (attrs.underline || attrs.link.is_some()).then_some(color);
            let strikethrough = (attrs.strikethrough || checked).then_some(color);
            if background.is_none() && underline.is_none() && strikethrough.is_none() {
                continue;
            }
            match decorations.marks.last_mut() {
                Some(last)
                    if last.range.end == range.start
                        && last.background == background
                        && last.underline == underline
                        && last.strikethrough == strikethrough =>
                {
                    last.range.end = range.end;
                }
                _ => decorations.marks.push(Mark {
                    range,
                    background,
                    underline,
                    strikethrough,
                }),
            }
        }
        decorations
    }
}
