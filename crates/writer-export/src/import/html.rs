//! The little HTML the Markdown importer reads: one tag at a time, its
//! attributes, and the declarations of a `style` attribute.

use writer_core::{Alignment, Highlight, Rgb};

/// One HTML tag or comment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Tag {
    /// `<name attr="value">` or `<name/>`. Names are lower case.
    Open {
        name: String,
        attrs: Vec<(String, String)>,
    },
    /// `</name>`.
    Close { name: String },
    /// `<!-- inner -->`, with the inner text trimmed.
    Comment(String),
}

impl Tag {
    /// The value of an attribute of an opening tag.
    pub(crate) fn attr(&self, wanted: &str) -> Option<&str> {
        match self {
            Tag::Open { attrs, .. } => attrs
                .iter()
                .find(|(name, _)| name == wanted)
                .map(|(_, value)| value.as_str()),
            _ => None,
        }
    }
}

/// Parses the tag or comment `text` starts with and returns it with the
/// number of bytes it takes. `None` when `text` does not start with a
/// complete tag.
pub(crate) fn parse_tag(text: &str) -> Option<(Tag, usize)> {
    let rest = text.strip_prefix('<')?;
    if let Some(body) = rest.strip_prefix("!--") {
        let end = body.find("-->")?;
        return Some((Tag::Comment(body[..end].trim().to_string()), 4 + end + 3));
    }
    let bytes = text.as_bytes();
    let mut i = 1;
    let closing = bytes.get(i) == Some(&b'/');
    if closing {
        i += 1;
    }
    let name_start = i;
    if !bytes.get(i).is_some_and(u8::is_ascii_alphabetic) {
        return None;
    }
    while bytes
        .get(i)
        .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'-')
    {
        i += 1;
    }
    let name = text[name_start..i].to_ascii_lowercase();
    let skip_space = |i: &mut usize| {
        while bytes.get(*i).is_some_and(u8::is_ascii_whitespace) {
            *i += 1;
        }
    };
    if closing {
        skip_space(&mut i);
        return (bytes.get(i) == Some(&b'>')).then_some((Tag::Close { name }, i + 1));
    }
    let mut attrs = Vec::new();
    loop {
        skip_space(&mut i);
        match bytes.get(i)? {
            b'>' => return Some((Tag::Open { name, attrs }, i + 1)),
            b'/' => {
                i += 1;
                continue;
            }
            _ => {}
        }
        let start = i;
        while bytes
            .get(i)
            .is_some_and(|b| !b.is_ascii_whitespace() && !matches!(b, b'=' | b'>' | b'/'))
        {
            i += 1;
        }
        if i == start {
            // A stray `=`: not a tag we can read.
            return None;
        }
        let attr = text[start..i].to_ascii_lowercase();
        skip_space(&mut i);
        let mut value = String::new();
        if bytes.get(i) == Some(&b'=') {
            i += 1;
            skip_space(&mut i);
            match bytes.get(i)? {
                quote @ (b'"' | b'\'') => {
                    let from = i + 1;
                    let len = bytes[from..].iter().position(|b| b == quote)?;
                    value = unescape(&text[from..from + len]);
                    i = from + len + 1;
                }
                _ => {
                    let from = i;
                    while bytes
                        .get(i)
                        .is_some_and(|b| !b.is_ascii_whitespace() && *b != b'>')
                    {
                        i += 1;
                    }
                    value = unescape(&text[from..i]);
                }
            }
        }
        attrs.push((attr, value));
    }
}

/// The character references an attribute value commonly holds.
fn unescape(value: &str) -> String {
    if !value.contains('&') {
        return value.to_string();
    }
    value
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// The declarations of a `style` attribute: lower-case property names with
/// their trimmed values.
pub(crate) fn style_declarations(style: &str) -> Vec<(String, String)> {
    style
        .split(';')
        .filter_map(|d| {
            let (name, value) = d.split_once(':')?;
            let name = name.trim().to_ascii_lowercase();
            let value = value.trim();
            (!name.is_empty() && !value.is_empty()).then(|| (name, value.to_string()))
        })
        .collect()
}

/// A CSS colour: `#rrggbb`, `#rgb`, `rgb(r, g, b)` or one of the basic
/// colour names.
pub(crate) fn css_color(value: &str) -> Option<Rgb> {
    let v = value.trim().to_ascii_lowercase();
    if v.starts_with('#') {
        return Rgb::from_hex(&v);
    }
    if let Some(inner) = v.strip_prefix("rgb(").and_then(|r| r.strip_suffix(')')) {
        let parts: Vec<&str> = inner
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|p| !p.is_empty())
            .collect();
        if parts.len() != 3 {
            return None;
        }
        let channel = |p: &str| p.parse::<u8>().ok();
        return Some(Rgb::new(
            channel(parts[0])?,
            channel(parts[1])?,
            channel(parts[2])?,
        ));
    }
    let (r, g, b) = match v.as_str() {
        "black" => (0, 0, 0),
        "silver" => (192, 192, 192),
        "gray" | "grey" => (128, 128, 128),
        "white" => (255, 255, 255),
        "maroon" => (128, 0, 0),
        "red" => (255, 0, 0),
        "purple" => (128, 0, 128),
        "fuchsia" | "magenta" => (255, 0, 255),
        "green" => (0, 128, 0),
        "lime" => (0, 255, 0),
        "olive" => (128, 128, 0),
        "yellow" => (255, 255, 0),
        "navy" => (0, 0, 128),
        "blue" => (0, 0, 255),
        "teal" => (0, 128, 128),
        "aqua" | "cyan" => (0, 255, 255),
        "orange" => (255, 165, 0),
        "pink" => (255, 192, 203),
        "brown" => (165, 42, 42),
        _ => return None,
    };
    Some(Rgb::new(r, g, b))
}

/// The first family of a CSS `font-family` value, without its quotes.
pub(crate) fn css_font(value: &str) -> Option<String> {
    let first = value.split(',').next()?.trim();
    let name = first
        .strip_prefix('\'')
        .and_then(|n| n.strip_suffix('\''))
        .or_else(|| first.strip_prefix('"').and_then(|n| n.strip_suffix('"')))
        .unwrap_or(first)
        .trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// The highlight a background colour stands for: one of the palette's
/// names or reference colours, else yellow.
pub(crate) fn css_highlight(value: &str) -> Highlight {
    if let Some(h) = Highlight::from_name(value) {
        return h;
    }
    css_color(value)
        .and_then(|rgb| {
            Highlight::ALL
                .into_iter()
                .find(|h| h.reference_rgb() == rgb)
        })
        .unwrap_or(Highlight::Yellow)
}

/// An alignment name as HTML writes it.
pub(crate) fn html_alignment(value: &str) -> Option<Alignment> {
    match value.trim().to_ascii_lowercase().as_str() {
        "left" | "start" => Some(Alignment::Left),
        "center" | "centre" => Some(Alignment::Center),
        "right" | "end" => Some(Alignment::Right),
        "justify" => Some(Alignment::Justify),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(name: &str, attrs: &[(&str, &str)]) -> Tag {
        Tag::Open {
            name: name.to_string(),
            attrs: attrs
                .iter()
                .map(|(n, v)| (n.to_string(), v.to_string()))
                .collect(),
        }
    }

    #[test]
    fn tags() {
        assert_eq!(parse_tag("<u>"), Some((open("u", &[]), 3)));
        assert_eq!(parse_tag("<BR/>"), Some((open("br", &[]), 5)));
        assert_eq!(parse_tag("<br />x"), Some((open("br", &[]), 6)));
        assert_eq!(
            parse_tag("</Mark >"),
            Some((
                Tag::Close {
                    name: "mark".into()
                },
                8
            ))
        );
        assert_eq!(
            parse_tag("<span style=\"color:#102030;font-family:'A B'\">"),
            Some((
                open("span", &[("style", "color:#102030;font-family:'A B'")]),
                46
            ))
        );
        assert_eq!(
            parse_tag("<a href='x?a=1&amp;b=2' target=_blank hidden>"),
            Some((
                open(
                    "a",
                    &[("href", "x?a=1&b=2"), ("target", "_blank"), ("hidden", "")]
                ),
                45
            ))
        );
        assert_eq!(
            parse_tag("<!-- pagebreak -->rest"),
            Some((Tag::Comment("pagebreak".into()), 18))
        );
        assert_eq!(parse_tag("<!---->"), Some((Tag::Comment(String::new()), 7)));
        for bad in [
            "",
            "x",
            "<",
            "<>",
            "< u>",
            "<u",
            "<u x=\"y>",
            "</u x>",
            "<!-- x",
            "<3>",
        ] {
            assert_eq!(parse_tag(bad), None, "{bad:?}");
        }
        assert_eq!(open("a", &[("href", "u")]).attr("href"), Some("u"));
        assert_eq!(open("a", &[]).attr("href"), None);
    }

    #[test]
    fn styles() {
        assert_eq!(
            style_declarations("Color: #fff; font-family:'X' ;; bad; : x"),
            [
                ("color".to_string(), "#fff".to_string()),
                ("font-family".to_string(), "'X'".to_string())
            ]
        );
        assert_eq!(css_color("#102030"), Some(Rgb::new(16, 32, 48)));
        assert_eq!(css_color(" #FFF "), Some(Rgb::new(255, 255, 255)));
        assert_eq!(css_color("rgb(1, 2,3)"), Some(Rgb::new(1, 2, 3)));
        assert_eq!(css_color("Red"), Some(Rgb::new(255, 0, 0)));
        // A word made of hex digits is not a colour without its `#`.
        assert_eq!(css_color("bad"), None);
        assert_eq!(css_color("rgb(1,2)"), None);
        assert_eq!(css_color("rgb(1,2,300)"), None);
        assert_eq!(
            css_font("'Gill Sans', sans-serif"),
            Some("Gill Sans".into())
        );
        assert_eq!(css_font("\"A\""), Some("A".into()));
        assert_eq!(css_font("Menlo"), Some("Menlo".into()));
        assert_eq!(css_font("''"), None);
        assert_eq!(css_highlight("green"), Highlight::Green);
        assert_eq!(
            css_highlight(&Highlight::Pink.reference_rgb().to_hex()),
            Highlight::Pink
        );
        assert_eq!(css_highlight("#123456"), Highlight::Yellow);
        assert_eq!(html_alignment("Center"), Some(Alignment::Center));
        assert_eq!(html_alignment("middle"), None);
    }
}
