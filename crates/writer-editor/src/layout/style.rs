//! From document attributes to GPUI text runs and block metrics.
//!
//! Everything that decides how a block is shaped lives in [`LayoutStyle`]: a
//! snapshot of the theme, the document typography and the editor mode. Two
//! equal styles shape a block identically, so the layout cache is dropped
//! exactly when the style changes.

use gpui::{
    Font, FontStyle, FontWeight, Hsla, Pixels, Rgba, StrikethroughStyle, TextRun, UnderlineStyle,
    px,
};
use writer_core::{Block, BlockKind, InlineAttrs, Typography};
use writer_ui::theme::{self, HighlightColor, Theme, ThemeColors};

/// Horizontal step of one indent level.
pub(crate) const INDENT_STEP: Pixels = px(24.);
/// Space left of a list item's text for its marker.
pub(crate) const MARKER_GUTTER: Pixels = px(28.);
/// Space left of a quote's text (the bar sits in it).
pub(crate) const QUOTE_GUTTER: Pixels = px(20.);
/// Inner padding of a code block.
pub(crate) const CODE_PADDING: Pixels = px(12.);
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
}

/// Sizes and spacing of one block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct BlockMetrics {
    pub(crate) font_size: Pixels,
    pub(crate) line_height: Pixels,
    /// Left edge of the text box, from the column's left edge.
    pub(crate) text_left: Pixels,
    /// Space kept free right of the text box.
    pub(crate) right_inset: Pixels,
    /// Space above the first row (block spacing plus inner padding).
    pub(crate) space_before: Pixels,
    /// Space below the last row.
    pub(crate) space_after: Pixels,
}

impl LayoutStyle {
    pub(crate) fn new(theme: &Theme, typography: &Typography, compact: bool) -> Self {
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
        }
    }

    /// Body text size (the size of a plain paragraph).
    pub(crate) fn base_size(&self) -> Pixels {
        if self.compact {
            theme::UI_TEXT_BASE
        } else {
            px(self.typography.base_size)
        }
    }

    /// Line height of a plain paragraph: the unit of row-wise scrolling.
    pub(crate) fn base_line_height(&self) -> Pixels {
        self.line_height_for(self.base_size())
    }

    fn line_height_for(&self, font_size: Pixels) -> Pixels {
        let ratio = if self.compact {
            COMPACT_LINE_HEIGHT
        } else {
            self.typography.line_height
        };
        (font_size * ratio).round()
    }

    pub(crate) fn metrics(&self, block: &Block) -> BlockMetrics {
        let font_size = if self.compact {
            theme::UI_TEXT_BASE
        } else {
            px(self.typography.size_of(block))
        };
        let line_height = self.line_height_for(font_size);
        let kind = block.kind();
        let mut text_left = INDENT_STEP * block.indent() as f32;
        let mut right_inset = px(0.);
        let mut space_before = px(0.);
        let mut space_after = if self.compact {
            px(2.)
        } else {
            px(self.typography.paragraph_spacing)
        };
        match kind {
            BlockKind::Heading { .. } if !self.compact => {
                space_before = (line_height * 0.35).round();
            }
            BlockKind::BulletItem | BlockKind::NumberedItem | BlockKind::TodoItem { .. } => {
                text_left += MARKER_GUTTER;
                space_after = (space_after * 0.35).round();
            }
            BlockKind::Quote => text_left += QUOTE_GUTTER,
            BlockKind::CodeBlock { .. } => {
                text_left += CODE_PADDING;
                right_inset = CODE_PADDING;
                space_before = CODE_PADDING;
                space_after += CODE_PADDING;
            }
            _ => {}
        }
        BlockMetrics {
            font_size,
            line_height,
            text_left,
            right_inset,
            space_before,
            space_after,
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
        if attrs.link.is_some() {
            return self.colors.accent;
        }
        if let Some(rgb) = attrs.color {
            return Rgba {
                r: rgb.r as f32 / 255.,
                g: rgb.g as f32 / 255.,
                b: rgb.b as f32 / 255.,
                a: 1.,
            }
            .into();
        }
        match block.kind() {
            BlockKind::TodoItem { checked: true } | BlockKind::Quote => self.colors.muted,
            _ => self.colors.ink,
        }
    }

    /// One GPUI text run per attribute span of the block. The run lengths
    /// cover the whole text, soft line breaks included, as `shape_text`
    /// requires.
    pub(crate) fn runs(&self, block: &Block) -> Vec<TextRun> {
        let checked = matches!(block.kind(), BlockKind::TodoItem { checked: true });
        block
            .spans()
            .map(|(range, attrs)| {
                let color = self.color_for(block, attrs);
                let background_color = attrs
                    .highlight
                    .and_then(|highlight| HighlightColor::parse(highlight.name()))
                    .map(|highlight| self.colors.highlight.get(highlight))
                    .or_else(|| {
                        (attrs.code && !block.kind().is_code()).then_some(self.colors.hover)
                    });
                TextRun {
                    len: range.len(),
                    font: self.font_for(block, attrs),
                    color,
                    background_color,
                    underline: (attrs.underline || attrs.link.is_some()).then_some(
                        UnderlineStyle {
                            thickness: px(1.),
                            color: Some(color),
                            wavy: false,
                        },
                    ),
                    strikethrough: (attrs.strikethrough || checked).then_some(
                        StrikethroughStyle {
                            thickness: px(1.),
                            color: Some(color),
                        },
                    ),
                }
            })
            .collect()
    }
}
