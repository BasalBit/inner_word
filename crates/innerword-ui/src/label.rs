//! Small text primitives: [`SectionLabel`], [`Divider`] and
//! [`HighlightedLabel`] (a label with fuzzy-match highlights).
//!
//! ```ignore
//! use innerword_ui::{SectionLabel, Divider, HighlightedLabel};
//!
//! div().child(SectionLabel::new("Folders"))
//!      .child(Divider::horizontal())
//!      .child(HighlightedLabel::new("New document", m.positions.clone()));
//! ```

use gpui::{
    App, FontWeight, HighlightStyle, Hsla, Pixels, SharedString, StyledText, Window, div,
    prelude::*, px,
};

use crate::theme::{ActiveTheme, UI_TEXT_BASE, UI_TEXT_XS};

/// A small semibold muted heading for groups of rows or settings.
#[derive(IntoElement, Clone, Debug)]
pub struct SectionLabel {
    text: SharedString,
}

impl SectionLabel {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self { text: text.into() }
    }
}

impl RenderOnce for SectionLabel {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        div()
            .text_size(UI_TEXT_XS)
            .line_height(px(16.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(cx.theme().colors.muted)
            .whitespace_nowrap()
            .child(self.text)
    }
}

/// A 1px hairline separator.
#[derive(IntoElement, Clone, Copy, Debug)]
pub struct Divider {
    vertical: bool,
    inset: Pixels,
}

impl Divider {
    /// Full-width horizontal line.
    pub fn horizontal() -> Self {
        Self {
            vertical: false,
            inset: px(0.),
        }
    }

    /// Full-height vertical line.
    pub fn vertical() -> Self {
        Self {
            vertical: true,
            inset: px(0.),
        }
    }

    /// Margin on both ends (left/right for horizontal, top/bottom for vertical).
    pub fn inset(mut self, inset: Pixels) -> Self {
        self.inset = inset;
        self
    }
}

impl RenderOnce for Divider {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let color = cx.theme().colors.hairline;
        if self.vertical {
            div()
                .flex_none()
                .w(px(1.))
                .h_full()
                .my(self.inset)
                .bg(color)
        } else {
            div()
                .flex_none()
                .h(px(1.))
                .w_full()
                .mx(self.inset)
                .bg(color)
        }
    }
}

/// A single-line label whose characters at `positions` (byte offsets, as
/// returned by [`crate::fuzzy`]) are drawn semibold in the accent colour.
#[derive(IntoElement, Clone, Debug)]
pub struct HighlightedLabel {
    text: SharedString,
    positions: Vec<usize>,
    color: Option<Hsla>,
    highlight_color: Option<Hsla>,
    size: Pixels,
}

impl HighlightedLabel {
    pub fn new(text: impl Into<SharedString>, positions: Vec<usize>) -> Self {
        Self {
            text: text.into(),
            positions,
            color: None,
            highlight_color: None,
            size: UI_TEXT_BASE,
        }
    }

    /// Base text colour (default: ink).
    pub fn color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }

    /// Colour of matched characters (default: accent).
    pub fn highlight_color(mut self, color: Hsla) -> Self {
        self.highlight_color = Some(color);
        self
    }

    pub fn size(mut self, size: Pixels) -> Self {
        self.size = size;
        self
    }
}

/// Groups byte positions into contiguous char ranges of `text`.
pub fn highlight_ranges(text: &str, positions: &[usize]) -> Vec<std::ops::Range<usize>> {
    let mut ranges: Vec<std::ops::Range<usize>> = Vec::new();
    for &start in positions {
        let Some(ch) = text.get(start..).and_then(|rest| rest.chars().next()) else {
            continue;
        };
        let end = start + ch.len_utf8();
        match ranges.last_mut() {
            Some(last) if last.end == start => last.end = end,
            _ => ranges.push(start..end),
        }
    }
    ranges
}

impl RenderOnce for HighlightedLabel {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.theme().colors;
        let style = HighlightStyle {
            color: Some(self.highlight_color.unwrap_or(colors.accent)),
            font_weight: Some(FontWeight::SEMIBOLD),
            ..Default::default()
        };
        let highlights: Vec<_> = highlight_ranges(&self.text, &self.positions)
            .into_iter()
            .map(|range| (range, style))
            .collect();
        div()
            .min_w_0()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_ellipsis()
            .text_size(self.size)
            .text_color(self.color.unwrap_or(colors.ink))
            .child(StyledText::new(self.text).with_highlights(highlights))
    }
}

#[cfg(test)]
mod tests {
    use super::highlight_ranges;

    #[test]
    fn ranges_merge_adjacent_chars() {
        assert_eq!(highlight_ranges("naïve", &[0, 1, 2, 5]), vec![0..4, 5..6]);
        assert_eq!(highlight_ranges("abc", &[1, 99]), vec![1..2]);
        assert!(highlight_ranges("abc", &[]).is_empty());
    }
}
