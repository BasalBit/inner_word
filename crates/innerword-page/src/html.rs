//! One small tokenizer for HTML and SVG (not a full HTML5 parser): comments, doctype,
//! CDATA, the raw-text elements, attributes quoted or not, character references decoded
//! for reading. It works on bytes, so a file in any encoding is read and written back as
//! it is; only what a reference occupies is ever replaced.

use std::ops::Range;

use crate::media::find;

#[derive(Clone, Debug)]
pub(crate) struct Tag {
    /// Lower case.
    pub name: String,
    /// From `<` to after `>`.
    pub span: Range<usize>,
    pub attrs: Vec<Attr>,
    pub self_closing: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct Attr {
    /// Lower case.
    pub name: String,
    /// The attribute with the white space before it (what removing it takes out).
    pub span: Range<usize>,
    pub value: Option<AttrValue>,
}

#[derive(Clone, Debug)]
pub(crate) struct AttrValue {
    /// The value as written, with its quotes.
    pub span: Range<usize>,
    /// The value with its character references decoded.
    pub decoded: String,
}

impl Tag {
    pub fn attr(&self, name: &str) -> Option<&Attr> {
        self.attrs.iter().find(|attr| attr.name == name)
    }

    pub fn value(&self, name: &str) -> Option<&str> {
        self.attr(name).and_then(|attr| attr.value.as_ref()).map(|value| value.decoded.as_str())
    }
}

#[derive(Clone, Debug)]
pub(crate) enum Token {
    Start(Tag),
    End { name: String },
    Text(Range<usize>),
    /// The contents of a raw-text element (`script`, `style`, `textarea`, `title`, ...).
    Raw { element: String, span: Range<usize> },
    Comment,
    /// Doctype, processing instruction, CDATA section.
    Other,
}

/// Elements whose contents are text to the tokenizer. In HTML as WebKit parses it with
/// scripts on (`noscript` and `iframe` too); in an XML document (SVG) only `script` and
/// `style`, and not when they close themselves (`<script href="a.js"/>`).
fn is_raw(name: &str, xml: bool) -> bool {
    if xml {
        return matches!(name, "script" | "style");
    }
    matches!(
        name,
        "script" | "style" | "textarea" | "title" | "xmp" | "noembed" | "noframes" | "noscript"
            | "iframe" | "plaintext"
    )
}

fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0c)
}

pub(crate) fn tokenize(bytes: &[u8], xml: bool) -> Vec<Token> {
    let n = bytes.len();
    let mut tokens = Vec::new();
    let mut i = 0;
    let mut text_start = 0;
    while i < n {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        let rest = &bytes[i..];
        let markup: Option<(Token, usize)> = if rest.starts_with(b"<!--") {
            let end = find(&bytes[i + 4..], b"-->").map(|e| i + 4 + e + 3).unwrap_or(n);
            Some((Token::Comment, end))
        } else if rest.starts_with(b"<![CDATA[") {
            let end = find(&bytes[i + 9..], b"]]>").map(|e| i + 9 + e + 3).unwrap_or(n);
            Some((Token::Other, end))
        } else if rest.starts_with(b"<!") || rest.starts_with(b"<?") {
            let end = bytes[i + 2..].iter().position(|&b| b == b'>').map(|e| i + 2 + e + 1).unwrap_or(n);
            Some((Token::Other, end))
        } else if rest.len() > 2 && rest[1] == b'/' && rest[2].is_ascii_alphabetic() {
            let mut j = i + 2;
            while j < n && !is_space(bytes[j]) && bytes[j] != b'>' && bytes[j] != b'/' {
                j += 1;
            }
            let name = lower(&bytes[i + 2..j]);
            let end = bytes[j..].iter().position(|&b| b == b'>').map(|e| j + e + 1).unwrap_or(n);
            Some((Token::End { name }, end))
        } else if rest.len() > 1 && rest[1].is_ascii_alphabetic() {
            let (tag, end) = start_tag(bytes, i);
            Some((Token::Start(tag), end))
        } else {
            None
        };
        let Some((token, end)) = markup else {
            i += 1;
            continue;
        };
        if text_start < i {
            tokens.push(Token::Text(text_start..i));
        }
        let raw = match &token {
            Token::Start(tag) if is_raw(&tag.name, xml) && !(xml && tag.self_closing) => {
                Some(tag.name.clone())
            }
            _ => None,
        };
        tokens.push(token);
        i = end;
        text_start = end;
        if let Some(element) = raw {
            let close = if element == "plaintext" { n } else { closing_tag(bytes, i, &element) };
            tokens.push(Token::Raw { element, span: i..close });
            i = close;
            text_start = close;
        }
    }
    if text_start < n {
        tokens.push(Token::Text(text_start..n));
    }
    tokens
}

/// Where `</name` (any case, followed by white space, `/` or `>`) begins, from `from`.
fn closing_tag(bytes: &[u8], from: usize, name: &str) -> usize {
    let n = bytes.len();
    let mut at = from;
    while at + 2 + name.len() <= n {
        let Some(found) = find(&bytes[at..], b"</") else {
            return n;
        };
        let start = at + found;
        let name_end = start + 2 + name.len();
        if name_end <= n
            && bytes[start + 2..name_end].eq_ignore_ascii_case(name.as_bytes())
            && (name_end == n || is_space(bytes[name_end]) || bytes[name_end] == b'>' || bytes[name_end] == b'/')
        {
            return start;
        }
        at = start + 2;
    }
    n
}

fn start_tag(bytes: &[u8], at: usize) -> (Tag, usize) {
    let n = bytes.len();
    let mut j = at + 1;
    while j < n && !is_space(bytes[j]) && bytes[j] != b'>' && bytes[j] != b'/' {
        j += 1;
    }
    let name = lower(&bytes[at + 1..j]);
    let mut attrs = Vec::new();
    let mut self_closing = false;
    loop {
        let lead = j;
        // White space and stray slashes between attributes.
        while j < n && (is_space(bytes[j]) || bytes[j] == b'/') {
            if bytes[j] == b'/' && j + 1 < n && bytes[j + 1] == b'>' {
                self_closing = true;
            }
            j += 1;
        }
        if j >= n {
            break;
        }
        if bytes[j] == b'>' {
            j += 1;
            break;
        }
        self_closing = false;
        let name_start = j;
        if bytes[j] == b'=' {
            j += 1;
        }
        while j < n && !is_space(bytes[j]) && bytes[j] != b'/' && bytes[j] != b'>' && bytes[j] != b'=' {
            j += 1;
        }
        let attr_name = lower(&bytes[name_start..j]);
        let mut k = j;
        while k < n && is_space(bytes[k]) {
            k += 1;
        }
        let mut value = None;
        if k < n && bytes[k] == b'=' {
            k += 1;
            while k < n && is_space(bytes[k]) {
                k += 1;
            }
            if k < n && (bytes[k] == b'"' || bytes[k] == b'\'') {
                let quote = bytes[k];
                let close = bytes[k + 1..].iter().position(|&b| b == quote).map(|e| k + 1 + e).unwrap_or(n);
                value = Some(AttrValue { span: k..(close + 1).min(n), decoded: decode(&bytes[k + 1..close]) });
                j = (close + 1).min(n);
            } else {
                let start = k;
                while k < n && !is_space(bytes[k]) && bytes[k] != b'>' {
                    k += 1;
                }
                value = Some(AttrValue { span: start..k, decoded: decode(&bytes[start..k]) });
                j = k;
            }
        }
        attrs.push(Attr { name: attr_name, span: lead..j, value });
    }
    (Tag { name, span: at..j, attrs, self_closing }, j)
}

fn lower(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).to_ascii_lowercase()
}

/// The text of `bytes` with its character references decoded (as UTF-8, lossily).
pub(crate) fn decode(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    if !text.contains('&') {
        return text.into_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest: &str = &text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        match reference(after) {
            Some((decoded, used)) => {
                out.push_str(&decoded);
                rest = &after[used..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// A character reference at the start of `text` (after the `&`): what it stands for and
/// how many bytes of `text` it took.
fn reference(text: &str) -> Option<(String, usize)> {
    let bytes = text.as_bytes();
    if bytes.first() == Some(&b'#') {
        let (radix, start) = if matches!(bytes.get(1), Some(b'x') | Some(b'X')) { (16, 2) } else { (10, 1) };
        let mut end = start;
        while end < bytes.len() && (bytes[end] as char).is_digit(radix) {
            end += 1;
        }
        if end == start {
            return None;
        }
        let code = u32::from_str_radix(&text[start..end], radix).ok()?;
        let ch = char::from_u32(code).filter(|&c| c != '\0').unwrap_or('\u{FFFD}');
        let used = if bytes.get(end) == Some(&b';') { end + 1 } else { end };
        return Some((ch.to_string(), used));
    }
    let mut end = 0;
    while end < bytes.len() && bytes[end].is_ascii_alphanumeric() && end < 32 {
        end += 1;
    }
    if end == 0 {
        return None;
    }
    let name = &text[..end];
    let terminated = bytes.get(end) == Some(&b';');
    let ch = match name {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" if terminated => "'",
        "nbsp" => "\u{a0}",
        _ if !terminated => return None,
        "copy" => "©",
        "reg" => "®",
        "trade" => "™",
        "mdash" => "—",
        "ndash" => "–",
        "hellip" => "…",
        "lsquo" => "‘",
        "rsquo" => "’",
        "sbquo" => "‚",
        "ldquo" => "“",
        "rdquo" => "”",
        "bdquo" => "„",
        "laquo" => "«",
        "raquo" => "»",
        "lsaquo" => "‹",
        "rsaquo" => "›",
        "middot" => "·",
        "bull" => "•",
        "times" => "×",
        "divide" => "÷",
        "deg" => "°",
        "plusmn" => "±",
        "minus" => "−",
        "euro" => "€",
        "pound" => "£",
        "yen" => "¥",
        "cent" => "¢",
        "sect" => "§",
        "para" => "¶",
        "larr" => "←",
        "rarr" => "→",
        "uarr" => "↑",
        "darr" => "↓",
        "harr" => "↔",
        "check" => "✓",
        "shy" => "\u{ad}",
        "thinsp" => "\u{2009}",
        "ensp" => "\u{2002}",
        "emsp" => "\u{2003}",
        "zwj" => "\u{200d}",
        "zwnj" => "\u{200c}",
        "tab" => "\t",
        "newline" => "\n",
        "colon" => ":",
        "sol" => "/",
        "num" => "#",
        "quest" => "?",
        "equals" => "=",
        "percnt" => "%",
        "lpar" => "(",
        "rpar" => ")",
        "comma" => ",",
        "period" => ".",
        _ => return None,
    };
    Some((ch.to_string(), if terminated { end + 1 } else { end }))
}

/// A value written back into an attribute in double quotes.
pub(crate) fn quoted_attr(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}
