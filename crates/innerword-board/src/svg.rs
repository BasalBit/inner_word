//! Which bytes are an SVG document (`docs/whiteboard-design.md`, section 2, "Pictures").

use crate::limits::MAX_SVG_BYTES;

const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";

/// Bytes that are an SVG document: valid UTF-8 of at most [`MAX_SVG_BYTES`], an optional
/// byte-order mark, then only an XML declaration, comments, processing instructions, a
/// doctype and white space before a root element `svg` in the SVG namespace (`<svg
/// xmlns="http://www.w3.org/2000/svg">`, or a prefix bound to it). Such bytes are kept as
/// they came; every other byte goes through `innerword_image::prepare`.
pub fn sniff_svg(bytes: &[u8]) -> bool {
    if bytes.len() > MAX_SVG_BYTES {
        return false;
    }
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let mut rest = text.strip_prefix('\u{feff}').unwrap_or(text);
    loop {
        rest = rest.trim_start_matches([' ', '\t', '\r', '\n']);
        if let Some(after) = rest.strip_prefix("<?") {
            // The XML declaration or a processing instruction.
            match after.find("?>") {
                Some(end) => rest = &after[end + 2..],
                None => return false,
            }
        } else if let Some(after) = rest.strip_prefix("<!--") {
            match after.find("-->") {
                Some(end) => rest = &after[end + 3..],
                None => return false,
            }
        } else if rest.len() >= 9 && rest[..9].eq_ignore_ascii_case("<!DOCTYPE") {
            match doctype_end(&rest[9..]) {
                Some(end) => rest = &rest[9 + end..],
                None => return false,
            }
        } else if let Some(after) = rest.strip_prefix('<') {
            return root_is_svg(after);
        } else {
            return false;
        }
    }
}

/// The length of a doctype's rest up to and including its `>`, an internal subset in
/// brackets and quoted literals skipped.
fn doctype_end(text: &str) -> Option<usize> {
    let mut quote: Option<char> = None;
    let mut depth = 0usize;
    for (i, c) in text.char_indices() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None => match c {
                '"' | '\'' => quote = Some(c),
                '[' => depth += 1,
                ']' => depth = depth.saturating_sub(1),
                '>' if depth == 0 => return Some(i + 1),
                _ => {}
            },
        }
    }
    None
}

/// The start tag after its `<`: its name is `svg` (or `p:svg`) and an `xmlns` (or
/// `xmlns:p`) attribute binds that name to the SVG namespace.
fn root_is_svg(tag: &str) -> bool {
    let name_end = tag
        .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
        .unwrap_or(tag.len());
    let name = &tag[..name_end];
    let (prefix, local) = match name.split_once(':') {
        Some((prefix, local)) => (Some(prefix), local),
        None => (None, name),
    };
    if local != "svg" {
        return false;
    }
    let wanted = match prefix {
        Some(prefix) => format!("xmlns:{prefix}"),
        None => "xmlns".to_string(),
    };
    for (key, value) in attributes(&tag[name_end..]) {
        if key == wanted && value == SVG_NAMESPACE {
            return true;
        }
    }
    false
}

/// The attributes of a start tag, up to its `>`.
fn attributes(mut text: &str) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    loop {
        text = text.trim_start();
        if text.is_empty() || text.starts_with('>') || text.starts_with("/>") {
            return out;
        }
        let key_end = text
            .find(|c: char| c == '=' || c.is_whitespace() || c == '>')
            .unwrap_or(text.len());
        let key = &text[..key_end];
        text = text[key_end..].trim_start();
        let Some(after) = text.strip_prefix('=') else {
            if key.is_empty() {
                return out;
            }
            continue;
        };
        text = after.trim_start();
        let Some(quote) = text.chars().next().filter(|c| *c == '"' || *c == '\'') else {
            return out;
        };
        let Some(end) = text[1..].find(quote) else {
            return out;
        };
        out.push((key, &text[1..1 + end]));
        text = &text[1 + end + 1..];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attributes_are_read() {
        assert_eq!(
            attributes(r#" a="1" b='2' >"#),
            vec![("a", "1"), ("b", "2")]
        );
    }
}
