//! The references of a style sheet: `@import "…"`, `@import url(…)`, `url(…)` quoted or
//! not, the strings of `image-set(…)` and `-webkit-image-set(…)`. Comments are skipped;
//! `/*# sourceMappingURL=… */` comments are noted (a fetched style sheet loses them).

use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CssKind {
    Import,
    Url,
    /// A `url(…)` inside an `@font-face` rule: a font.
    Font,
    ImageSet,
    /// Any other string: not a reference (it may name an address, which is said).
    Plain,
}

#[derive(Clone, Debug)]
pub(crate) struct CssItem {
    /// The bytes of the address (inside its quotes, or the bare text of `url(…)`).
    pub span: Range<usize>,
    /// The address with its escapes decoded.
    pub value: String,
    pub kind: CssKind,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct CssScan {
    pub items: Vec<CssItem>,
    pub source_maps: Vec<Range<usize>>,
}

fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0c)
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b == b'-' || b >= 0x80 || b == b'\\'
}

fn is_ident(b: u8) -> bool {
    is_ident_start(b) || b.is_ascii_digit()
}

pub(crate) fn scan_css(bytes: &[u8]) -> CssScan {
    let n = bytes.len();
    let mut scan = CssScan::default();
    let mut i = 0;
    let mut pending_import = false;
    let mut depth: usize = 0;
    let mut image_sets: Vec<usize> = Vec::new();
    // Braces open, and the brace depth inside an `@font-face` rule while in one.
    let mut braces: usize = 0;
    let mut pending_font_face = false;
    let mut font_face: Option<usize> = None;
    while i < n {
        let b = bytes[i];
        if b == b'/' && i + 1 < n && bytes[i + 1] == b'*' {
            let end = crate::media::find(&bytes[i + 2..], b"*/").map(|e| i + 2 + e + 2).unwrap_or(n);
            let inner = String::from_utf8_lossy(&bytes[i + 2..end.saturating_sub(2).max(i + 2)]);
            let inner = inner.trim_start();
            if inner.starts_with("# sourceMappingURL=") || inner.starts_with("@ sourceMappingURL=") {
                scan.source_maps.push(i..end);
            }
            i = end;
            continue;
        }
        if b == b'"' || b == b'\'' {
            let (end, value) = string(bytes, i);
            let kind = if image_sets.last() == Some(&depth) {
                CssKind::ImageSet
            } else if pending_import {
                CssKind::Import
            } else {
                CssKind::Plain
            };
            let inner_end = if end > i + 1 && bytes[end - 1] == b {
                end - 1
            } else {
                end
            };
            scan.items.push(CssItem { span: i + 1..inner_end, value, kind });
            pending_import = false;
            i = end;
            continue;
        }
        if b == b'@' {
            let mut j = i + 1;
            while j < n && is_ident(bytes[j]) {
                j += 1;
            }
            if bytes[i + 1..j].eq_ignore_ascii_case(b"import") {
                pending_import = true;
            }
            if bytes[i + 1..j].eq_ignore_ascii_case(b"font-face") {
                pending_font_face = true;
            }
            i = j;
            continue;
        }
        if is_ident_start(b) && !(b == b'-' && i + 1 < n && bytes[i + 1].is_ascii_digit()) {
            let mut j = i;
            while j < n && is_ident(bytes[j]) {
                if bytes[j] == b'\\' {
                    j += 1;
                }
                j += 1;
            }
            let j = j.min(n);
            if j < n && bytes[j] == b'(' {
                let name = String::from_utf8_lossy(&bytes[i..j]).to_ascii_lowercase();
                if name == "url" {
                    let (mut item, end) = url_function(bytes, j + 1, pending_import);
                    if let Some(item) = item.as_mut().filter(|item| item.kind == CssKind::Url && font_face.is_some()) {
                        item.kind = CssKind::Font;
                    }
                    if let Some(item) = item {
                        scan.items.push(item);
                    }
                    pending_import = false;
                    i = end;
                    continue;
                }
                depth += 1;
                if name == "image-set" || name == "-webkit-image-set" {
                    image_sets.push(depth);
                }
                i = j + 1;
                continue;
            }
            i = j;
            continue;
        }
        match b {
            b'(' => depth += 1,
            b')' => {
                if image_sets.last() == Some(&depth) {
                    image_sets.pop();
                }
                depth = depth.saturating_sub(1);
            }
            b'{' => {
                pending_import = false;
                braces += 1;
                if pending_font_face && font_face.is_none() {
                    font_face = Some(braces);
                }
                pending_font_face = false;
            }
            b'}' => {
                pending_import = false;
                pending_font_face = false;
                if font_face == Some(braces) {
                    font_face = None;
                }
                braces = braces.saturating_sub(1);
            }
            b';' => {
                pending_import = false;
                pending_font_face = false;
            }
            _ => {}
        }
        i += 1;
    }
    scan
}

/// The inside of `url(` from `at`: the item, and where scanning goes on.
fn url_function(bytes: &[u8], at: usize, import: bool) -> (Option<CssItem>, usize) {
    let n = bytes.len();
    let kind = if import { CssKind::Import } else { CssKind::Url };
    let mut k = at;
    while k < n && is_space(bytes[k]) {
        k += 1;
    }
    if k < n && (bytes[k] == b'"' || bytes[k] == b'\'') {
        let quote = bytes[k];
        let (end, value) = string(bytes, k);
        let inner_end = if end > k + 1 && bytes[end - 1] == quote { end - 1 } else { end };
        let mut after = end;
        while after < n && bytes[after] != b')' && bytes[after] != b';' && bytes[after] != b'}' {
            after += 1;
        }
        if after < n && bytes[after] == b')' {
            after += 1;
        }
        return (Some(CssItem { span: k + 1..inner_end, value, kind }), after);
    }
    let start = k;
    let mut value = String::new();
    while k < n && bytes[k] != b')' {
        if bytes[k] == b'\\' && k + 1 < n {
            let (ch, used) = escape(bytes, k + 1);
            if let Some(ch) = ch {
                value.push(ch);
            }
            k += 1 + used;
            continue;
        }
        k += 1;
    }
    let mut end = k;
    while end > start && is_space(bytes[end - 1]) {
        end -= 1;
    }
    if value.is_empty() || !bytes[start..end].contains(&b'\\') {
        value = String::from_utf8_lossy(&bytes[start..end]).into_owned();
    }
    let after = if k < n { k + 1 } else { n };
    if start == end {
        return (None, after);
    }
    (Some(CssItem { span: start..end, value: value.trim().to_string(), kind }), after)
}

/// A CSS string from its opening quote at `at`: where it ends (after the closing quote,
/// or at the line break of an unterminated one) and its value.
fn string(bytes: &[u8], at: usize) -> (usize, String) {
    let n = bytes.len();
    let quote = bytes[at];
    let mut k = at + 1;
    let mut raw: Vec<u8> = Vec::new();
    let mut decoded = String::new();
    while k < n {
        let b = bytes[k];
        if b == quote {
            k += 1;
            break;
        }
        if b == b'\n' {
            break;
        }
        if b == b'\\' && k + 1 < n {
            if bytes[k + 1] == b'\n' {
                k += 2;
                continue;
            }
            decoded.push_str(&String::from_utf8_lossy(&raw));
            raw.clear();
            let (ch, used) = escape(bytes, k + 1);
            if let Some(ch) = ch {
                decoded.push(ch);
            }
            k += 1 + used;
            continue;
        }
        raw.push(b);
        k += 1;
    }
    decoded.push_str(&String::from_utf8_lossy(&raw));
    (k, decoded)
}

/// A CSS escape after its backslash: the character and the bytes it took.
fn escape(bytes: &[u8], at: usize) -> (Option<char>, usize) {
    let n = bytes.len();
    let mut k = at;
    while k < n && k - at < 6 && bytes[k].is_ascii_hexdigit() {
        k += 1;
    }
    if k > at {
        let code = u32::from_str_radix(std::str::from_utf8(&bytes[at..k]).unwrap_or("0"), 16).unwrap_or(0xFFFD);
        if k < n && is_space(bytes[k]) {
            k += 1;
        }
        return (char::from_u32(code).or(Some('\u{FFFD}')), k - at);
    }
    // One character, which may be several bytes of UTF-8.
    let len = utf8_len(bytes[at]).min(n - at);
    let ch = std::str::from_utf8(&bytes[at..at + len]).ok().and_then(|s| s.chars().next());
    (ch, len)
}

pub(crate) fn utf8_len(first: u8) -> usize {
    match first {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        _ => 1,
    }
}
