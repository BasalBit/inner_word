//! Text measured with the advance widths of the bundled fonts (`docs/whiteboard-design.md`,
//! section 7.1): close to Excalidraw's own measure, not exact (no kerning, no shaping).
//! Excalidraw measures every text again whenever it loads a scene.
//!
//! The widths are read from [`METRICS_JSON`], a copy of `dist/font-metrics.json` of
//! `innerword-board-web`, which its build writes from the font files: for each family a
//! scene can name, its units per em and the advance of every character from U+0020 to
//! U+024F. A character outside that range is 0.6 em wide, a CJK character 1 em. Line
//! heights are Excalidraw 0.18.1's own per family (`FONT_METADATA`), so a height is exact.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde_json::Value;

/// The copy of `innerword-board-web`'s `dist/font-metrics.json` (a test of that crate holds
/// the two equal).
pub const METRICS_JSON: &str = include_str!("metrics/font-metrics.json");

/// Excalidraw's font families (`FONT_FAMILY`), with the names a skeleton may give.
pub const FAMILIES: &[(u8, &str)] = &[
    (1, "Virgil"),
    (2, "Helvetica"),
    (3, "Cascadia"),
    (5, "Excalifont"),
    (6, "Nunito"),
    (7, "Lilita One"),
    (8, "Comic Shanns"),
    (9, "Liberation Sans"),
];

/// The family Excalidraw gives a text without one (Excalifont).
pub const DEFAULT_FONT_FAMILY: u8 = 5;
/// The size Excalidraw gives a text without one.
pub const DEFAULT_FONT_SIZE: f64 = 20.0;

/// The family's number for a name or a number given as text (`"Nunito"`, `"6"`),
/// ignoring case.
pub fn family_by_name(name: &str) -> Option<u8> {
    let name = name.trim();
    if let Ok(number) = name.parse::<u8>() {
        return FAMILIES.iter().find(|(n, _)| *n == number).map(|(n, _)| *n);
    }
    FAMILIES.iter().find(|(_, family)| family.eq_ignore_ascii_case(name)).map(|(n, _)| *n)
}

/// The family's name (`5` is "Excalifont"); a number Excalidraw does not know is written
/// as "font 4".
pub fn family_name(family: u8) -> String {
    FAMILIES
        .iter()
        .find(|(n, _)| *n == family)
        .map(|(_, name)| name.to_string())
        .unwrap_or_else(|| format!("font {family}"))
}

/// Excalidraw's line height of a family, as a multiple of the font size (`getLineHeight`:
/// a family it does not know has Excalifont's).
pub fn line_height(font_family: u8) -> f64 {
    match font_family {
        1 => 1.25,
        2 => 1.15,
        3 => 1.2,
        5 => 1.25,
        6 => 1.35,
        7 => 1.15,
        8 => 1.25,
        9 => 1.15,
        _ => 1.25,
    }
}

struct Family {
    units_per_em: f64,
    /// Advance by character, in font units.
    advances: HashMap<char, f64>,
}

struct Table {
    families: HashMap<u8, Family>,
}

fn table() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(|| parse(METRICS_JSON))
}

/// Reads the metrics file. Its form (written by `innerword-board-web`'s build): an object
/// with `families`, each `{ "family": <number>, "unitsPerEm": <n>, "first": <code point>,
/// "advances": [<advance of first>, <of first + 1>, …] }`; also accepted: `advances` as an
/// object from a character (or its code point) to its advance. Anything it cannot read
/// leaves that family to the fallback widths.
fn parse(json: &str) -> Table {
    let mut families = HashMap::new();
    let Ok(value) = serde_json::from_str::<Value>(json) else {
        return Table { families };
    };
    let list: Vec<&Value> = match value.get("families") {
        Some(Value::Array(items)) => items.iter().collect(),
        Some(Value::Object(map)) => map.values().collect(),
        _ => Vec::new(),
    };
    for item in list {
        let Some(number) = item.get("family").and_then(Value::as_u64) else { continue };
        let units_per_em = item.get("unitsPerEm").and_then(Value::as_f64).filter(|u| *u > 0.0).unwrap_or(1000.0);
        let mut advances = HashMap::new();
        match item.get("advances") {
            Some(Value::Array(widths)) => {
                let first = item.get("first").and_then(Value::as_u64).unwrap_or(0x20) as u32;
                for (n, width) in widths.iter().enumerate() {
                    if let (Some(c), Some(w)) = (char::from_u32(first + n as u32), width.as_f64()) {
                        advances.insert(c, w);
                    }
                }
            }
            Some(Value::Object(map)) => {
                for (key, width) in map {
                    let c = if key.chars().count() == 1 {
                        key.chars().next()
                    } else {
                        key.trim_start_matches("U+").parse::<u32>().ok().and_then(char::from_u32).or_else(|| {
                            u32::from_str_radix(key.trim_start_matches("U+"), 16).ok().and_then(char::from_u32)
                        })
                    };
                    if let (Some(c), Some(w)) = (c, width.as_f64()) {
                        advances.insert(c, w);
                    }
                }
            }
            _ => {}
        }
        families.insert(number as u8, Family { units_per_em, advances });
    }
    Table { families }
}

/// Whether the character is CJK (Excalidraw's `CJK.CHAR` scripts: Han, Hiragana, Katakana,
/// Hangul, and the fullwidth forms).
pub(crate) fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x11FF | 0x2E80..=0x2FDF | 0x3000..=0x303F | 0x3040..=0x30FF | 0x3100..=0x31FF
        | 0x3200..=0x9FFF | 0xA960..=0xA97F | 0xAC00..=0xD7FF | 0xF900..=0xFAFF
        | 0xFF00..=0xFFEF | 0x20000..=0x3FFFF)
}

/// The width of one character at `font_size`.
pub(crate) fn char_width(c: char, font_family: u8, font_size: f64) -> f64 {
    if c == '\n' || c == '\r' {
        return 0.0;
    }
    if is_cjk(c) {
        return font_size;
    }
    let family = table().families.get(&font_family).or_else(|| table().families.get(&DEFAULT_FONT_FAMILY));
    if let Some(family) = family
        && let Some(advance) = family.advances.get(&c)
    {
        return advance / family.units_per_em * font_size;
    }
    if is_zero_width(c) {
        return 0.0;
    }
    0.6 * font_size
}

fn is_zero_width(c: char) -> bool {
    matches!(c as u32, 0x200B..=0x200F | 0x2060..=0x2064 | 0xFE00..=0xFE0F | 0x0300..=0x036F | 0xFEFF)
}

/// The width of one line (no line breaks in it).
pub(crate) fn line_width(line: &str, font_family: u8, font_size: f64) -> f64 {
    line.chars().map(|c| char_width(c, font_family, font_size)).sum()
}

/// Excalidraw's `normalizeText`: line ends as `\n`, a tab as eight spaces.
pub fn normalize_text(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n").replace('\t', "        ")
}

/// The size of a text (`measureText`): the widest line, and the lines times the size times
/// the family's line height. An empty line counts as a space, as Excalidraw counts it.
pub fn text_size(text: &str, font_family: u8, font_size: f64) -> (f64, f64) {
    let text = normalize_text(text);
    let lines: Vec<&str> = text.split('\n').collect();
    let width = lines
        .iter()
        .map(|line| if line.is_empty() { line_width(" ", font_family, font_size) } else { line_width(line, font_family, font_size) })
        .fold(0.0, f64::max);
    let height = font_size * line_height(font_family) * lines.len() as f64;
    (width, height)
}

/// Excalidraw's `wrapText`: each line that is wider than `max_width` broken between words
/// (after white space and hyphens, around CJK characters), a word wider than the line
/// broken between its characters, white space at a break dropped. A width that is not
/// finite or below 0 leaves the text as it is.
pub fn wrap(text: &str, font_family: u8, font_size: f64, max_width: f64) -> String {
    if !max_width.is_finite() || max_width < 0.0 {
        return text.to_string();
    }
    let mut lines: Vec<String> = Vec::new();
    for original in text.split('\n') {
        if line_width(original, font_family, font_size) <= max_width {
            lines.push(original.to_string());
            continue;
        }
        lines.extend(wrap_line(original, font_family, font_size, max_width));
    }
    lines.join("\n")
}

/// The pieces a line breaks into: a run of white space is its own pieces, a hyphen ends a
/// piece, a CJK character is a piece of its own.
fn tokens(line: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    for c in line.chars() {
        if c.is_whitespace() || is_cjk(c) {
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
            out.push(c.to_string());
        } else {
            current.push(c);
            if c == '-' {
                out.push(std::mem::take(&mut current));
            }
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

fn wrap_line(line: &str, font_family: u8, font_size: f64, max_width: f64) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_width = 0.0;
    let all = tokens(line);
    let mut i = 0;
    while i < all.len() {
        let token = &all[i];
        let test_width = current_width + line_width(token, font_family, font_size);
        let blank = token.chars().all(char::is_whitespace);
        if blank || test_width <= max_width {
            current.push_str(token);
            current_width = test_width;
            i += 1;
            continue;
        }
        if current.is_empty() {
            let mut pieces = wrap_word(token, font_family, font_size, max_width);
            let trailing = pieces.pop().unwrap_or_default();
            lines.extend(pieces);
            current_width = line_width(&trailing, font_family, font_size);
            current = trailing;
            i += 1;
        } else {
            lines.push(current.trim_end().to_string());
            current = String::new();
            current_width = 0.0;
        }
    }
    if !current.is_empty() {
        lines.push(trim_line(&current, font_family, font_size, max_width));
    }
    lines
}

fn wrap_word(word: &str, font_family: u8, font_size: f64, max_width: f64) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_width = 0.0;
    for c in word.chars() {
        let w = char_width(c, font_family, font_size);
        if current_width + w <= max_width {
            current.push(c);
            current_width += w;
            continue;
        }
        if !current.is_empty() {
            lines.push(std::mem::take(&mut current));
        }
        current.push(c);
        current_width = w;
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

fn trim_line(line: &str, font_family: u8, font_size: f64, max_width: f64) -> String {
    if line_width(line, font_family, font_size) <= max_width {
        return line.to_string();
    }
    let trimmed = line.trim_end();
    let spaces = &line[trimmed.len()..];
    let mut out = trimmed.to_string();
    let mut width = line_width(trimmed, font_family, font_size);
    for c in spaces.chars() {
        let w = char_width(c, font_family, font_size);
        if width + w > max_width {
            break;
        }
        out.push(c);
        width += w;
    }
    out
}

/// Whether the metrics file had widths for this family.
pub fn has_widths(font_family: u8) -> bool {
    table().families.get(&font_family).is_some_and(|f| !f.advances.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heights_are_excalidraws() {
        assert_eq!(text_size("a", 5, 20.0).1, 25.0);
        assert_eq!(text_size("a\nb", 6, 20.0).1, 54.0);
        assert_eq!(text_size("a", 99, 20.0).1, 25.0);
    }

    #[test]
    fn wrapping_breaks_between_words() {
        let one = line_width("word", 5, 20.0);
        let wrapped = wrap("word word word", 5, 20.0, one * 2.0);
        assert!(wrapped.lines().count() >= 2, "{wrapped:?}");
        assert!(wrapped.lines().all(|l| line_width(l, 5, 20.0) <= one * 2.0 + 1e-9), "{wrapped:?}");
        assert_eq!(wrap("short", 5, 20.0, -10.0), "short");
        let letters = wrap("abcdefghij", 5, 20.0, one / 2.0);
        assert!(letters.lines().count() > 2);
        assert_eq!(letters.replace('\n', ""), "abcdefghij");
    }

    #[test]
    fn families_by_name() {
        assert_eq!(family_by_name("nunito"), Some(6));
        assert_eq!(family_by_name("5"), Some(5));
        assert_eq!(family_by_name("4"), None);
        assert_eq!(family_by_name("Comic Sans"), None);
        assert_eq!(family_name(3), "Cascadia");
    }
}
