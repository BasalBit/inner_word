//! XML text helpers for the DOCX parts.

/// Whether XML 1.0 allows the character at all.
pub(crate) fn is_xml_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..='\u{10ffff}')
}

/// Escapes text for element content. Characters XML forbids are removed,
/// and so is `\r`.
pub(crate) fn text(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' => {}
            c if is_xml_char(c) => out.push(c),
            _ => {}
        }
    }
    out
}

/// Escapes text for a double-quoted attribute value. Line breaks and tabs
/// become spaces: no attribute this crate writes can hold them.
pub(crate) fn attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\t' | '\n' | '\r' => out.push(' '),
            c if is_xml_char(c) => out.push(c),
            _ => {}
        }
    }
    out
}

/// The XML declaration every part starts with.
pub(crate) const DECLARATION: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";

/// A hyperlink target Word accepts: whitespace, control characters and the
/// characters URIs forbid are percent-encoded. Word refuses to open a
/// package whose relationship targets are not valid URIs.
pub(crate) fn uri(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.trim().chars() {
        let forbidden = c.is_whitespace()
            || c.is_control()
            || !is_xml_char(c)
            || matches!(c, '"' | '<' | '>' | '\\' | '^' | '`' | '{' | '|' | '}');
        if forbidden {
            let mut buf = [0u8; 4];
            for b in c.encode_utf8(&mut buf).bytes() {
                out.push_str(&format!("%{b:02X}"));
            }
        } else {
            out.push(c);
        }
    }
    // A lone "%" that does not start an escape would be invalid too.
    let bytes = out.as_bytes();
    let mut fixed = String::with_capacity(out.len());
    for (i, c) in out.char_indices() {
        if c == '%' {
            let valid = bytes.get(i + 1).is_some_and(u8::is_ascii_hexdigit) && bytes.get(i + 2).is_some_and(u8::is_ascii_hexdigit);
            if !valid {
                fixed.push_str("%25");
                continue;
            }
        }
        fixed.push(c);
    }
    fixed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escaping() {
        assert_eq!(text("a < b && c > d \"q\" 'r'"), "a &lt; b &amp;&amp; c &gt; d \"q\" 'r'");
        assert_eq!(attr("a < \"b\" & c\nd"), "a &lt; &quot;b&quot; &amp; c d");
        assert_eq!(text("a\u{0}b\u{1}c\u{b}d\u{c}e\u{1f}f\u{fffe}g\u{ffff}h\ri"), "abcdefghi");
        assert_eq!(text("tab\tnl\nemoji 😀 rtl שלום"), "tab\tnl\nemoji 😀 rtl שלום");
        assert_eq!(attr("\u{7}x"), "x");
    }

    #[test]
    fn uris() {
        assert_eq!(uri("https://example.com/a?b=c&d=e#f"), "https://example.com/a?b=c&d=e#f");
        assert_eq!(uri(" a b\tc "), "a%20b%09c");
        assert_eq!(uri("x<y>\"{z}|\\^`"), "x%3Cy%3E%22%7Bz%7D%7C%5C%5E%60");
        assert_eq!(uri("100%"), "100%25");
        assert_eq!(uri("a%20b%zz"), "a%20b%25zz");
        assert_eq!(uri("https://example.com/日本"), "https://example.com/日本");
        assert_eq!(uri("a\u{0}b"), "a%00b");
    }
}
