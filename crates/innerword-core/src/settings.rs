//! Per-document settings: page setup, typography defaults, page view.

use crate::block::{Block, BlockKind};

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
        let fix = |v: f32| {
            if v.is_finite() {
                v.max(0.0)
            } else {
                POINTS_PER_INCH
            }
        };
        let mut m = Margins {
            top: fix(self.margins.top),
            right: fix(self.margins.right),
            bottom: fix(self.margins.bottom),
            left: fix(self.margins.left),
        };
        let shrink = |a: &mut f32, b: &mut f32, total: f32| {
            let room = (total - POINTS_PER_INCH).max(0.0);
            // The slack keeps this a fixed point: scaled margins can exceed
            // `room` by a rounding error and must not be scaled again.
            if *a + *b > room + room * 1e-5 + 1e-4 {
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

/// The loosest line height (a multiple of the text size) a heading is set
/// at, whatever the document's line height: see
/// [`Typography::line_height_for`].
pub const HEADING_MAX_LINE_HEIGHT: f32 = 1.3;

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

    /// The line height (a multiple of the text size) a block of `kind` is
    /// set at: [`Typography::line_height`], except that a heading is never
    /// set looser than [`HEADING_MAX_LINE_HEIGHT`]. Large type with the
    /// leading of body text falls apart into separate lines; a heading of
    /// two lines has to read as one thing. The editor's page view and the
    /// DOCX export both follow this.
    pub fn line_height_for(&self, kind: &BlockKind) -> f32 {
        if kind.heading_level().is_some() {
            self.line_height.min(HEADING_MAX_LINE_HEIGHT)
        } else {
            self.line_height
        }
    }

    /// The text size in points `block` is shown at: its override, else
    /// [`Typography::size_for`] its kind.
    pub fn size_of(&self, block: &Block) -> f32 {
        block
            .text_size()
            .unwrap_or_else(|| self.size_for(block.kind()))
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

/// The language a document is spell-checked in, when the document says so
/// itself ([`DocumentSettings::spelling_language`]).
///
/// The identifier of a language is whatever the spell checker of the
/// platform calls it (`"en_GB"`, `"de"`, `"pt_BR"`); this crate does not know
/// which languages exist, it only keeps the identifier in a tidy shape (see
/// [`SpellingLanguage::language`]).
///
/// As one string ([`SpellingLanguage::as_setting_str`],
/// [`SpellingLanguage::from_setting_str`]) it is `"auto"` or the identifier;
/// the document format and the app's own setting both store it that way.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum SpellingLanguage {
    /// The checker detects the language of each paragraph by itself.
    Automatic,
    /// One language for the whole document, by its identifier. Build it
    /// with [`SpellingLanguage::language`], which rejects what cannot be an
    /// identifier.
    Language(String),
}

impl SpellingLanguage {
    /// How [`SpellingLanguage::Automatic`] is written as a setting string.
    pub const AUTOMATIC: &'static str = "auto";
    /// The longest identifier that is kept, in bytes.
    pub const MAX_IDENTIFIER_LEN: usize = 40;

    /// A fixed language. The identifier is trimmed; it has to start with an
    /// ASCII letter, consist of ASCII letters, digits, `_` and `-` only and
    /// be at most [`SpellingLanguage::MAX_IDENTIFIER_LEN`] bytes long,
    /// otherwise there is no such language (`None`). An identifier that
    /// spells "auto" or "automatic" (in any case) is
    /// [`SpellingLanguage::Automatic`].
    pub fn language(identifier: &str) -> Option<SpellingLanguage> {
        let id = identifier.trim();
        if id.eq_ignore_ascii_case(Self::AUTOMATIC) || id.eq_ignore_ascii_case("automatic") {
            return Some(SpellingLanguage::Automatic);
        }
        let well_formed = !id.is_empty()
            && id.len() <= Self::MAX_IDENTIFIER_LEN
            && id.as_bytes()[0].is_ascii_alphabetic()
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        well_formed.then(|| SpellingLanguage::Language(id.to_string()))
    }

    /// Reads the one-string form: `"auto"` or a language identifier. The
    /// same as [`SpellingLanguage::language`]; `None` for anything else
    /// (callers treat that as "not set").
    pub fn from_setting_str(value: &str) -> Option<SpellingLanguage> {
        SpellingLanguage::language(value)
    }

    /// The one-string form: `"auto"` or the language identifier.
    pub fn as_setting_str(&self) -> &str {
        match self {
            SpellingLanguage::Automatic => Self::AUTOMATIC,
            SpellingLanguage::Language(id) => id,
        }
    }

    /// The identifier of the fixed language; `None` for
    /// [`SpellingLanguage::Automatic`].
    pub fn identifier(&self) -> Option<&str> {
        match self {
            SpellingLanguage::Automatic => None,
            SpellingLanguage::Language(id) => Some(id),
        }
    }

    /// Whether this is [`SpellingLanguage::Automatic`].
    pub fn is_automatic(&self) -> bool {
        matches!(self, SpellingLanguage::Automatic)
    }

    fn sanitized(self) -> Option<SpellingLanguage> {
        match self {
            SpellingLanguage::Automatic => Some(SpellingLanguage::Automatic),
            SpellingLanguage::Language(id) => SpellingLanguage::language(&id),
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
    /// The language this document is spell-checked in. `None` (the default)
    /// means the document follows the app's setting; otherwise the document
    /// overrides it, with automatic detection or with one fixed language.
    /// A fixed language is also the proofing language of a DOCX export.
    pub spelling_language: Option<SpellingLanguage>,
}

impl DocumentSettings {
    /// Clamps every value into a usable range. A spelling language whose
    /// identifier cannot be one is dropped (the document follows the app's
    /// setting again).
    pub fn sanitized(self) -> DocumentSettings {
        DocumentSettings {
            page: self.page.sanitized(),
            typography: self.typography.sanitized(),
            page_view: self.page_view,
            spelling_language: self.spelling_language.and_then(SpellingLanguage::sanitized),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings_are_set_tighter_than_loose_body_text() {
        let mut t = Typography::default();
        assert_eq!(t.line_height_for(&BlockKind::Paragraph), 1.6);
        assert_eq!(t.line_height_for(&BlockKind::heading(1)), 1.3);
        assert_eq!(t.line_height_for(&BlockKind::heading(6)), 1.3);
        // Double spacing leaves headings at their own leading; a tight
        // document tightens them too.
        t.line_height = 2.0;
        assert_eq!(t.line_height_for(&BlockKind::Quote), 2.0);
        assert_eq!(
            t.line_height_for(&BlockKind::heading(2)),
            HEADING_MAX_LINE_HEIGHT
        );
        t.line_height = 1.1;
        assert_eq!(t.line_height_for(&BlockKind::heading(2)), 1.1);
    }

    #[test]
    fn spelling_language_identifiers() {
        use SpellingLanguage::{Automatic, Language};
        assert_eq!(
            SpellingLanguage::language("en_GB"),
            Some(Language("en_GB".into()))
        );
        assert_eq!(
            SpellingLanguage::language("  pt-BR \n"),
            Some(Language("pt-BR".into()))
        );
        assert_eq!(
            SpellingLanguage::language("de"),
            Some(Language("de".into()))
        );
        assert_eq!(SpellingLanguage::language("auto"), Some(Automatic));
        assert_eq!(SpellingLanguage::language(" Automatic "), Some(Automatic));
        for bad in [
            "",
            "   ",
            "en GB",
            "en/GB",
            "1en",
            "_en",
            "é",
            "en\u{0}",
            "x".repeat(41).as_str(),
        ] {
            assert_eq!(SpellingLanguage::language(bad), None, "{bad:?}");
        }
        assert!(SpellingLanguage::language(&"x".repeat(40)).is_some());
        // The one-string form goes there and back.
        for l in [Automatic, Language("en_AU".into())] {
            assert_eq!(
                SpellingLanguage::from_setting_str(l.as_setting_str()),
                Some(l.clone())
            );
            assert_eq!(l.is_automatic(), l.identifier().is_none());
        }
        assert_eq!(Language("fr".into()).identifier(), Some("fr"));
    }

    #[test]
    fn spelling_language_is_sanitized_with_the_settings() {
        let with = |l: Option<SpellingLanguage>| DocumentSettings {
            spelling_language: l,
            ..Default::default()
        };
        assert_eq!(DocumentSettings::default().spelling_language, None);
        let fixed = with(Some(SpellingLanguage::Language("en_GB".into())));
        assert_eq!(fixed.clone().sanitized(), fixed);
        let auto = with(Some(SpellingLanguage::Automatic));
        assert_eq!(auto.clone().sanitized(), auto);
        // A hand-built identifier that cannot be one is dropped; one that
        // only needs trimming is kept; "auto" is the automatic choice.
        assert_eq!(
            with(Some(SpellingLanguage::Language("no such".into()))).sanitized(),
            with(None)
        );
        assert_eq!(
            with(Some(SpellingLanguage::Language(" de ".into())))
                .sanitized()
                .spelling_language,
            Some(SpellingLanguage::Language("de".into()))
        );
        assert_eq!(
            with(Some(SpellingLanguage::Language("AUTO".into()))).sanitized(),
            auto
        );
        // Sanitising is a fixed point.
        let s = with(Some(SpellingLanguage::Language(" de ".into()))).sanitized();
        assert_eq!(s.clone().sanitized(), s);
    }

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
        assert_eq!(
            t.size_of(&Block::with_text(BlockKind::heading(1), "x")),
            34.0
        );
        assert_eq!(
            t.size_of(&Block::paragraph("x").with_text_size(Some(22.0))),
            22.0
        );
    }
}
