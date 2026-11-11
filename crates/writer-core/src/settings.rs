//! Per-document settings: page setup, typography defaults, page view.

use crate::block::BlockKind;

/// Points per inch.
pub const POINTS_PER_INCH: f32 = 72.0;

/// Paper size of the page setup.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum PaperSize {
    /// ISO A4, 210 × 297 mm.
    #[default]
    A4,
    /// US Letter, 8.5 × 11 in.
    UsLetter,
    /// A custom portrait size in points.
    Custom {
        /// Width in points.
        width: f32,
        /// Height in points.
        height: f32,
    },
}

impl PaperSize {
    /// Smallest custom dimension in points (1 inch).
    pub const MIN_DIMENSION: f32 = 72.0;
    /// Largest custom dimension in points (200 inches).
    pub const MAX_DIMENSION: f32 = 14_400.0;

    /// Portrait `(width, height)` in points.
    pub fn portrait_size(self) -> (f32, f32) {
        match self {
            PaperSize::A4 => (595.276, 841.89),
            PaperSize::UsLetter => (612.0, 792.0),
            PaperSize::Custom { width, height } => (width, height),
        }
    }

    fn sanitized(self) -> PaperSize {
        match self {
            PaperSize::Custom { width, height } => {
                let fix = |v: f32| {
                    if v.is_finite() {
                        v.clamp(Self::MIN_DIMENSION, Self::MAX_DIMENSION)
                    } else {
                        Self::MIN_DIMENSION
                    }
                };
                PaperSize::Custom {
                    width: fix(width),
                    height: fix(height),
                }
            }
            p => p,
        }
    }
}

/// Page orientation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Orientation {
    /// Taller than wide.
    #[default]
    Portrait,
    /// Wider than tall.
    Landscape,
}

/// Page margins in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Margins {
    /// Top margin.
    pub top: f32,
    /// Right margin.
    pub right: f32,
    /// Bottom margin.
    pub bottom: f32,
    /// Left margin.
    pub left: f32,
}

impl Margins {
    /// The same margin on all four sides.
    pub const fn uniform(points: f32) -> Margins {
        Margins {
            top: points,
            right: points,
            bottom: points,
            left: points,
        }
    }
}

impl Default for Margins {
    /// One inch on every side.
    fn default() -> Margins {
        Margins::uniform(POINTS_PER_INCH)
    }
}

/// Page setup: paper, orientation and margins.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PageSetup {
    /// Paper size.
    pub paper: PaperSize,
    /// Orientation.
    pub orientation: Orientation,
    /// Margins in points.
    pub margins: Margins,
}

impl PageSetup {
    /// `(width, height)` of the sheet in points, orientation applied.
    pub fn page_size(&self) -> (f32, f32) {
        let (w, h) = self.paper.portrait_size();
        match self.orientation {
            Orientation::Portrait => (w, h),
            Orientation::Landscape => (h, w),
        }
    }

    /// `(width, height)` of the text area (page minus margins) in points.
    pub fn content_size(&self) -> (f32, f32) {
        let (w, h) = self.page_size();
        (
            (w - self.margins.left - self.margins.right).max(0.0),
            (h - self.margins.top - self.margins.bottom).max(0.0),
        )
    }

    /// Clamps every value into a usable range: finite, non-negative margins
    /// that leave at least an inch of text area in each direction.
    pub fn sanitized(self) -> PageSetup {
        let paper = self.paper.sanitized();
        let mut s = PageSetup { paper, ..self };
        let (w, h) = s.page_size();
        let fix = |v: f32| if v.is_finite() { v.max(0.0) } else { POINTS_PER_INCH };
        let mut m = Margins {
            top: fix(self.margins.top),
            right: fix(self.margins.right),
            bottom: fix(self.margins.bottom),
            left: fix(self.margins.left),
        };
        let shrink = |a: &mut f32, b: &mut f32, total: f32| {
            let room = (total - POINTS_PER_INCH).max(0.0);
            if *a + *b > room {
                let scale = if *a + *b > 0.0 { room / (*a + *b) } else { 0.0 };
                *a *= scale;
                *b *= scale;
            }
        };
        shrink(&mut m.left, &mut m.right, w);
        shrink(&mut m.top, &mut m.bottom, h);
        s.margins = m;
        s
    }
}

/// Typography defaults of the document.
#[derive(Clone, Debug, PartialEq)]
pub struct Typography {
    /// Body font family (default `"Charter"`).
    pub body_font: String,
    /// Base text size in points (default 17). On screen one point is one
    /// logical pixel.
    pub base_size: f32,
    /// Line height as a multiple of the text size (default 1.6).
    pub line_height: f32,
    /// Space after each paragraph in points (default 12).
    pub paragraph_spacing: f32,
}

impl Default for Typography {
    fn default() -> Typography {
        Typography {
            body_font: "Charter".to_string(),
            base_size: 17.0,
            line_height: 1.6,
            paragraph_spacing: 12.0,
        }
    }
}

impl Typography {
    /// The text size in points of a block of `kind` without an override.
    /// This is the single source of truth for heading sizes; the editor and
    /// the exporters both use it.
    ///
    /// Heading factors of the base size: 2.0, 1.6, 1.35, 1.15, 1.0, 0.9;
    /// code blocks 0.88; everything else 1.0. Rounded to whole points.
    pub fn size_for(&self, kind: &BlockKind) -> f32 {
        let factor = match kind {
            BlockKind::Heading { level } => match level {
                1 => 2.0,
                2 => 1.6,
                3 => 1.35,
                4 => 1.15,
                5 => 1.0,
                _ => 0.9,
            },
            BlockKind::CodeBlock { .. } => 0.88,
            _ => 1.0,
        };
        (self.base_size * factor).round()
    }

    /// Clamps values into usable ranges and replaces an empty font name.
    pub fn sanitized(self) -> Typography {
        let d = Typography::default();
        let fix = |v: f32, lo: f32, hi: f32, def: f32| {
            if v.is_finite() { v.clamp(lo, hi) } else { def }
        };
        let font = self.body_font.trim();
        Typography {
            body_font: if font.is_empty() {
                d.body_font
            } else {
                font.to_string()
            },
            base_size: fix(self.base_size, 6.0, 72.0, d.base_size),
            line_height: fix(self.line_height, 0.8, 4.0, d.line_height),
            paragraph_spacing: fix(self.paragraph_spacing, 0.0, 144.0, d.paragraph_spacing),
        }
    }
}

/// All per-document settings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DocumentSettings {
    /// Page setup (used by page view and print-like exports).
    pub page: PageSetup,
    /// Typography defaults.
    pub typography: Typography,
    /// Whether the document is shown paginated.
    pub page_view: bool,
}

impl DocumentSettings {
    /// Clamps every value into a usable range.
    pub fn sanitized(self) -> DocumentSettings {
        DocumentSettings {
            page: self.page.sanitized(),
            typography: self.typography.sanitized(),
            page_view: self.page_view,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let s = DocumentSettings::default();
        assert_eq!(s.page.paper, PaperSize::A4);
        assert_eq!(s.page.margins, Margins::uniform(72.0));
        assert_eq!(s.typography.body_font, "Charter");
        assert_eq!(s.typography.base_size, 17.0);
        assert_eq!(s.typography.line_height, 1.6);
        assert!(!s.page_view);
        assert_eq!(s.clone().sanitized(), s);
    }

    #[test]
    fn page_sizes_and_sanitizing() {
        let mut p = PageSetup {
            paper: PaperSize::UsLetter,
            orientation: Orientation::Landscape,
            ..Default::default()
        };
        assert_eq!(p.page_size(), (792.0, 612.0));
        assert_eq!(p.content_size(), (648.0, 468.0));
        p.margins = Margins::uniform(1000.0);
        let s = p.sanitized();
        let (w, h) = s.content_size();
        assert!(w >= 71.9 && h >= 71.9);
        let c = PageSetup {
            paper: PaperSize::Custom {
                width: f32::NAN,
                height: -5.0,
            },
            ..Default::default()
        }
        .sanitized();
        assert_eq!(c.page_size(), (72.0, 72.0));
    }

    #[test]
    fn heading_sizes() {
        let t = Typography::default();
        assert_eq!(t.size_for(&BlockKind::Paragraph), 17.0);
        assert_eq!(t.size_for(&BlockKind::heading(1)), 34.0);
        assert!(t.size_for(&BlockKind::heading(2)) > t.size_for(&BlockKind::heading(3)));
    }
}
