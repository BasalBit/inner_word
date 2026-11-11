//! Inline (character-level) attributes.

use std::collections::BTreeSet;
use std::fmt;

/// Identifier of a comment thread, unique within one document.
///
/// Comment ids are minted by the document and never reused within one
/// editing session, so an id that was deleted and restored through undo
/// cannot collide with a newer thread.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CommentId(pub u64);

impl fmt::Display for CommentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "c{}", self.0)
    }
}

/// The small named palette available for text highlights.
///
/// The UI themes the actual colours; [`Highlight::reference_rgb`] gives a
/// neutral reference colour for exporters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Highlight {
    /// Yellow (the default highlight, also produced by the `==text==` input rule).
    Yellow,
    /// Green.
    Green,
    /// Blue.
    Blue,
    /// Pink.
    Pink,
    /// Orange.
    Orange,
    /// Purple.
    Purple,
}

impl Highlight {
    /// Every highlight colour, in menu order.
    pub const ALL: [Highlight; 6] = [
        Highlight::Yellow,
        Highlight::Green,
        Highlight::Blue,
        Highlight::Pink,
        Highlight::Orange,
        Highlight::Purple,
    ];

    /// Lower-case stable name, used in the serialised format (`"yellow"`).
    pub fn name(self) -> &'static str {
        match self {
            Highlight::Yellow => "yellow",
            Highlight::Green => "green",
            Highlight::Blue => "blue",
            Highlight::Pink => "pink",
            Highlight::Orange => "orange",
            Highlight::Purple => "purple",
        }
    }

    /// Parses a name produced by [`Highlight::name`] (case-insensitive).
    pub fn from_name(name: &str) -> Option<Highlight> {
        let name = name.trim();
        Highlight::ALL
            .into_iter()
            .find(|h| h.name().eq_ignore_ascii_case(name))
    }

    /// A light reference colour suitable for print export. The editor UI
    /// may use its own themed values instead.
    pub fn reference_rgb(self) -> Rgb {
        match self {
            Highlight::Yellow => Rgb::new(0xff, 0xf1, 0x76),
            Highlight::Green => Rgb::new(0xc5, 0xe8, 0xb7),
            Highlight::Blue => Rgb::new(0xbb, 0xde, 0xfb),
            Highlight::Pink => Rgb::new(0xf8, 0xbb, 0xd0),
            Highlight::Orange => Rgb::new(0xff, 0xd5, 0x9e),
            Highlight::Purple => Rgb::new(0xd9, 0xc5, 0xf0),
        }
    }
}

/// An opaque 24-bit sRGB colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rgb {
    /// Red channel.
    pub r: u8,
    /// Green channel.
    pub g: u8,
    /// Blue channel.
    pub b: u8,
}

impl Rgb {
    /// Creates a colour from its channels.
    pub const fn new(r: u8, g: u8, b: u8) -> Rgb {
        Rgb { r, g, b }
    }

    /// Formats as `#rrggbb` (lower case).
    pub fn to_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    /// Parses `#rrggbb`, `rrggbb`, `#rgb` or `rgb` (case-insensitive).
    pub fn from_hex(s: &str) -> Option<Rgb> {
        let s = s.trim();
        let s = s.strip_prefix('#').unwrap_or(s);
        if !s.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        match s.len() {
            6 => {
                let v = u32::from_str_radix(s, 16).ok()?;
                Some(Rgb::new((v >> 16) as u8, (v >> 8) as u8, v as u8))
            }
            3 => {
                let v = u16::from_str_radix(s, 16).ok()?;
                let expand = |n: u16| ((n & 0xf) as u8) * 17;
                Some(Rgb::new(expand(v >> 8), expand(v >> 4), expand(v)))
            }
            _ => None,
        }
    }
}

/// The on/off character styles that can be toggled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InlineStyle {
    /// Bold (`cmd-b`, `**text**`).
    Bold,
    /// Italic (`cmd-i`, `*text*`, `_text_`).
    Italic,
    /// Underline (`cmd-u`).
    Underline,
    /// Strikethrough (`cmd-shift-x`, `~~text~~`).
    Strikethrough,
    /// Inline code (`cmd-e`, `` `text` ``).
    Code,
}

impl InlineStyle {
    /// All styles, in a stable order.
    pub const ALL: [InlineStyle; 5] = [
        InlineStyle::Bold,
        InlineStyle::Italic,
        InlineStyle::Underline,
        InlineStyle::Strikethrough,
        InlineStyle::Code,
    ];
}

/// The attributes of one run of characters.
///
/// All fields are public and plain data; the invariants that matter (runs are
/// normalised, anchors reference existing threads) are maintained by
/// [`Document`](crate::Document) and [`EditorState`](crate::EditorState).
/// `Default` is plain text.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct InlineAttrs {
    /// Bold weight.
    pub bold: bool,
    /// Italic style.
    pub italic: bool,
    /// Underline.
    pub underline: bool,
    /// Strikethrough.
    pub strikethrough: bool,
    /// Inline code (monospace).
    pub code: bool,
    /// Background highlight from the named palette.
    pub highlight: Option<Highlight>,
    /// Text colour; `None` is the theme's ink colour.
    pub color: Option<Rgb>,
    /// Font family name; `None` is the document's body font.
    pub font: Option<String>,
    /// Link target URL.
    pub link: Option<String>,
    /// Comment threads anchored on this text.
    pub comments: BTreeSet<CommentId>,
}

impl InlineAttrs {
    /// True when no attribute is set (plain text).
    pub fn is_plain(&self) -> bool {
        *self == InlineAttrs::default()
    }

    /// Whether the given toggle style is on.
    pub fn has_style(&self, style: InlineStyle) -> bool {
        match style {
            InlineStyle::Bold => self.bold,
            InlineStyle::Italic => self.italic,
            InlineStyle::Underline => self.underline,
            InlineStyle::Strikethrough => self.strikethrough,
            InlineStyle::Code => self.code,
        }
    }

    /// Turns the given toggle style on or off.
    pub fn set_style(&mut self, style: InlineStyle, on: bool) {
        match style {
            InlineStyle::Bold => self.bold = on,
            InlineStyle::Italic => self.italic = on,
            InlineStyle::Underline => self.underline = on,
            InlineStyle::Strikethrough => self.strikethrough = on,
            InlineStyle::Code => self.code = on,
        }
    }

    /// Builder: returns `self` with `style` turned on.
    pub fn with(mut self, style: InlineStyle) -> InlineAttrs {
        self.set_style(style, true);
        self
    }

    /// What "clear formatting" leaves behind: every character style
    /// (bold, italic, underline, strikethrough, code, highlight, colour and
    /// font family) is removed; the link and the comment anchors are kept.
    pub fn cleared(&self) -> InlineAttrs {
        InlineAttrs {
            link: self.link.clone(),
            comments: self.comments.clone(),
            ..InlineAttrs::default()
        }
    }

    /// The subset of attributes a code block keeps: only comment anchors.
    pub(crate) fn code_block_subset(&self) -> InlineAttrs {
        InlineAttrs {
            comments: self.comments.clone(),
            ..InlineAttrs::default()
        }
    }

    /// Normalises user-supplied strings: empty font / link become `None`.
    pub(crate) fn sanitize(&mut self) {
        if let Some(f) = &self.font {
            let t = f.trim();
            self.font = if t.is_empty() { None } else { Some(t.to_string()) };
        }
        if let Some(l) = &self.link {
            let t = l.trim();
            self.link = if t.is_empty() { None } else { Some(t.to_string()) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb_hex_round_trip() {
        let c = Rgb::new(0x12, 0xab, 0xff);
        assert_eq!(c.to_hex(), "#12abff");
        assert_eq!(Rgb::from_hex("#12ABff"), Some(c));
        assert_eq!(Rgb::from_hex("12abff"), Some(c));
        assert_eq!(Rgb::from_hex("#fff"), Some(Rgb::new(255, 255, 255)));
        assert_eq!(Rgb::from_hex("#ggg"), None);
        assert_eq!(Rgb::from_hex("#12345"), None);
        assert_eq!(Rgb::from_hex("+12345"), None);
    }

    #[test]
    fn highlight_names() {
        for h in Highlight::ALL {
            assert_eq!(Highlight::from_name(h.name()), Some(h));
        }
        assert_eq!(Highlight::from_name("YELLOW"), Some(Highlight::Yellow));
        assert_eq!(Highlight::from_name("teal"), None);
    }

    #[test]
    fn cleared_keeps_link_and_comments() {
        let mut a = InlineAttrs {
            bold: true,
            italic: true,
            code: true,
            highlight: Some(Highlight::Blue),
            color: Some(Rgb::new(1, 2, 3)),
            font: Some("Menlo".into()),
            link: Some("https://example.com".into()),
            ..Default::default()
        };
        a.comments.insert(CommentId(3));
        let c = a.cleared();
        assert!(!c.bold && !c.italic && !c.code);
        assert_eq!(c.highlight, None);
        assert_eq!(c.color, None);
        assert_eq!(c.font, None);
        assert_eq!(c.link.as_deref(), Some("https://example.com"));
        assert!(c.comments.contains(&CommentId(3)));
    }
}
