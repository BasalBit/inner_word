//! The references of a script: static `import … from "…"`, `import "…"`, `export … from
//! "…"`; `import("…")` and `` import(`…`) `` without `${`; `new Worker("…")`, `new
//! SharedWorker("…")`, `importScripts("…")`; string literals that are an absolute address
//! on a known CDN. Every other string literal is kept for what the bundler says about it.
//! A small lexer (strings, template literals with their substitutions, comments, regular
//! expression literals told from division by the token before), not a parser.

use std::ops::Range;

/// Hosts whose absolute addresses in a string literal of a script are fetched.
pub(crate) const KNOWN_CDNS: &[&str] = &[
    "cdnjs.cloudflare.com",
    "cdn.jsdelivr.net",
    "unpkg.com",
    "esm.sh",
    "cdn.skypack.dev",
    "ga.jspm.io",
    "cdn.tailwindcss.com",
    "code.jquery.com",
    "fonts.googleapis.com",
    "fonts.gstatic.com",
];

#[derive(Clone, Debug)]
pub(crate) struct JsString {
    /// The bytes between the quotes (or backticks).
    pub span: Range<usize>,
    /// The value with its escapes decoded; for a template with substitutions, the text
    /// before the first `${`.
    pub value: String,
    /// A template literal with a `${…}` in it.
    pub substituted: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum JsKind {
    /// A module specifier (`import`, `export … from`, `import()`): resolved against the
    /// module's own address.
    Specifier,
    /// `new Worker`, `new SharedWorker`, `importScripts`, a CDN address in a string.
    Plain,
}

#[derive(Clone, Debug)]
pub(crate) struct JsRef {
    pub string: usize,
    pub kind: JsKind,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct JsScan {
    pub strings: Vec<JsString>,
    pub refs: Vec<JsRef>,
    pub source_maps: Vec<Range<usize>>,
}

impl JsScan {
    /// The strings that are not references.
    pub fn other_strings(&self) -> impl Iterator<Item = &JsString> {
        let used: std::collections::HashSet<usize> = self.refs.iter().map(|r| r.string).collect();
        self.strings
            .iter()
            .enumerate()
            .filter(move |(i, _)| !used.contains(i))
            .map(|(_, s)| s)
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Ident(String),
    Str(usize),
    Punct(u8),
    Other,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Brace {
    Block,
    Template,
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b == b'$' || b >= 0x80 || b == b'\\'
}

fn is_ident(b: u8) -> bool {
    is_ident_start(b) || b.is_ascii_digit()
}

/// Keywords after which a `/` begins a regular expression.
fn expression_follows(word: &str) -> bool {
    matches!(
        word,
        "return"
            | "typeof"
            | "case"
            | "do"
            | "else"
            | "in"
            | "of"
            | "new"
            | "delete"
            | "void"
            | "throw"
            | "instanceof"
            | "yield"
            | "await"
    )
}

pub(crate) fn scan_js(bytes: &[u8]) -> JsScan {
    let mut scan = JsScan::default();
    let tokens = lex(bytes, &mut scan);
    find_refs(bytes, &tokens, &mut scan);
    scan
}

fn lex(bytes: &[u8], scan: &mut JsScan) -> Vec<Tok> {
    let n = bytes.len();
    let mut tokens = Vec::new();
    let mut braces: Vec<Brace> = Vec::new();
    let mut regex_allowed = true;
    let mut i = 0;
    while i < n {
        let b = bytes[i];
        if matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) {
            i += 1;
            continue;
        }
        if b == b'/' && i + 1 < n && bytes[i + 1] == b'/' {
            let end = bytes[i..]
                .iter()
                .position(|&c| c == b'\n')
                .map(|e| i + e)
                .unwrap_or(n);
            let text = String::from_utf8_lossy(&bytes[i + 2..end]);
            if text.starts_with("# sourceMappingURL=") || text.starts_with("@ sourceMappingURL=") {
                scan.source_maps.push(i..end);
            }
            i = end;
            continue;
        }
        if b == b'/' && i + 1 < n && bytes[i + 1] == b'*' {
            let end = crate::media::find(&bytes[i + 2..], b"*/")
                .map(|e| i + 2 + e + 2)
                .unwrap_or(n);
            i = end;
            continue;
        }
        if b == b'/' {
            if regex_allowed {
                i = regex_end(bytes, i);
                tokens.push(Tok::Other);
                regex_allowed = false;
            } else {
                tokens.push(Tok::Punct(b'/'));
                regex_allowed = true;
                i += 1;
            }
            continue;
        }
        if b == b'"' || b == b'\'' {
            let (end, value) = string(bytes, i);
            let inner_end = if end > i + 1 && bytes[end - 1] == b {
                end - 1
            } else {
                end
            };
            scan.strings.push(JsString {
                span: i + 1..inner_end,
                value,
                substituted: false,
            });
            tokens.push(Tok::Str(scan.strings.len() - 1));
            regex_allowed = false;
            i = end;
            continue;
        }
        if b == b'`' {
            let (end, value, opened) = template_chunk(bytes, i + 1);
            if opened {
                // A `${` ended the chunk: the template goes on after the matching `}`.
                scan.strings.push(JsString {
                    span: i + 1..end - 2,
                    value,
                    substituted: true,
                });
                tokens.push(Tok::Str(scan.strings.len() - 1));
                braces.push(Brace::Template);
                regex_allowed = true;
            } else {
                let inner_end = if end > i + 1 && bytes[end - 1] == b'`' {
                    end - 1
                } else {
                    end
                };
                scan.strings.push(JsString {
                    span: i + 1..inner_end,
                    value,
                    substituted: false,
                });
                tokens.push(Tok::Str(scan.strings.len() - 1));
                regex_allowed = false;
            }
            i = end;
            continue;
        }
        if b == b'{' {
            braces.push(Brace::Block);
            tokens.push(Tok::Punct(b));
            regex_allowed = true;
            i += 1;
            continue;
        }
        if b == b'}' {
            match braces.pop() {
                Some(Brace::Template) => {
                    // The next chunk of the template: up to another `${`, or its backtick.
                    let (end, _, opened) = template_chunk(bytes, i + 1);
                    if opened {
                        braces.push(Brace::Template);
                        regex_allowed = true;
                    } else {
                        tokens.push(Tok::Other);
                        regex_allowed = false;
                    }
                    i = end;
                }
                _ => {
                    tokens.push(Tok::Punct(b));
                    regex_allowed = true;
                    i += 1;
                }
            }
            continue;
        }
        if is_ident_start(b) {
            let mut j = i;
            while j < n && is_ident(bytes[j]) {
                if bytes[j] == b'\\' {
                    j += 1;
                }
                j += 1;
            }
            let j = j.min(n);
            let word = String::from_utf8_lossy(&bytes[i..j]).into_owned();
            regex_allowed = expression_follows(&word);
            tokens.push(Tok::Ident(word));
            i = j;
            continue;
        }
        if b.is_ascii_digit() || (b == b'.' && i + 1 < n && bytes[i + 1].is_ascii_digit()) {
            let mut j = i + 1;
            while j < n
                && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'.' || bytes[j] == b'_')
            {
                j += 1;
            }
            tokens.push(Tok::Other);
            regex_allowed = false;
            i = j;
            continue;
        }
        tokens.push(Tok::Punct(b));
        regex_allowed = !matches!(b, b')' | b']');
        i += 1;
    }
    tokens
}

/// Where a regular expression literal that begins at `at` ends (after its flags). One
/// that runs into a line break ends there.
fn regex_end(bytes: &[u8], at: usize) -> usize {
    let n = bytes.len();
    let mut j = at + 1;
    let mut in_class = false;
    while j < n {
        let c = bytes[j];
        if c == b'\\' {
            j += 2;
            continue;
        }
        if c == b'\n' {
            return j;
        }
        if in_class {
            if c == b']' {
                in_class = false;
            }
        } else if c == b'[' {
            in_class = true;
        } else if c == b'/' {
            j += 1;
            while j < n && is_ident(bytes[j]) {
                j += 1;
            }
            return j;
        }
        j += 1;
    }
    n
}

/// A quoted string from its quote at `at`: where it ends and its decoded value. One
/// that runs into a line break ends there.
fn string(bytes: &[u8], at: usize) -> (usize, String) {
    let n = bytes.len();
    let quote = bytes[at];
    let mut k = at + 1;
    let mut out: Vec<u8> = Vec::new();
    while k < n {
        let b = bytes[k];
        if b == quote {
            return (k + 1, String::from_utf8_lossy(&out).into_owned());
        }
        if b == b'\n' {
            break;
        }
        if b == b'\\' && k + 1 < n {
            k = escape(bytes, k + 1, &mut out);
            continue;
        }
        out.push(b);
        k += 1;
    }
    (k, String::from_utf8_lossy(&out).into_owned())
}

/// A chunk of a template literal from `at` (after the backtick or the `}` of a
/// substitution): where it ends, its value, and whether a `${` ended it (else the closing
/// backtick did, or the end of the text).
fn template_chunk(bytes: &[u8], at: usize) -> (usize, String, bool) {
    let n = bytes.len();
    let mut k = at;
    let mut out: Vec<u8> = Vec::new();
    while k < n {
        let b = bytes[k];
        if b == b'`' {
            return (k + 1, String::from_utf8_lossy(&out).into_owned(), false);
        }
        if b == b'$' && k + 1 < n && bytes[k + 1] == b'{' {
            return (k + 2, String::from_utf8_lossy(&out).into_owned(), true);
        }
        if b == b'\\' && k + 1 < n {
            k = escape(bytes, k + 1, &mut out);
            continue;
        }
        out.push(b);
        k += 1;
    }
    (n, String::from_utf8_lossy(&out).into_owned(), false)
}

/// An escape after its backslash at `at`, written into `out` as UTF-8; where it ends.
fn escape(bytes: &[u8], at: usize, out: &mut Vec<u8>) -> usize {
    let n = bytes.len();
    let c = bytes[at];
    let mut push = |ch: char| {
        let mut buf = [0u8; 4];
        out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
    };
    match c {
        b'n' => push('\n'),
        b't' => push('\t'),
        b'r' => push('\r'),
        b'b' => push('\u{8}'),
        b'f' => push('\u{c}'),
        b'v' => push('\u{b}'),
        b'0' if !(at + 1 < n && bytes[at + 1].is_ascii_digit()) => push('\0'),
        b'\r' => {
            if at + 1 < n && bytes[at + 1] == b'\n' {
                return at + 2;
            }
        }
        b'\n' => {}
        b'x' => {
            if at + 2 < n {
                if let Some(code) = hex_value(&bytes[at + 1..at + 3]) {
                    push(char::from_u32(code).unwrap_or('\u{FFFD}'));
                    return at + 3;
                }
            }
            push('x');
        }
        b'u' => {
            if at + 1 < n && bytes[at + 1] == b'{' {
                if let Some(close) = bytes[at + 2..].iter().position(|&b| b == b'}') {
                    if let Some(code) = hex_value(&bytes[at + 2..at + 2 + close]) {
                        push(char::from_u32(code).unwrap_or('\u{FFFD}'));
                        return at + 3 + close;
                    }
                }
            } else if at + 4 < n {
                if let Some(code) = hex_value(&bytes[at + 1..at + 5]) {
                    push(char::from_u32(code).unwrap_or('\u{FFFD}'));
                    return at + 5;
                }
            }
            push('u');
        }
        _ => {
            let len = crate::css::utf8_len(c).min(n - at);
            out.extend_from_slice(&bytes[at..at + len]);
            return at + len;
        }
    }
    at + 1
}

fn hex_value(bytes: &[u8]) -> Option<u32> {
    if bytes.is_empty() || bytes.len() > 6 || !bytes.iter().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(std::str::from_utf8(bytes).ok()?, 16).ok()
}

fn find_refs(_bytes: &[u8], tokens: &[Tok], scan: &mut JsScan) {
    let mut used = vec![false; scan.strings.len()];
    let after_dot = |i: usize| i > 0 && tokens[i - 1] == Tok::Punct(b'.');
    let string_at = |i: usize| match tokens.get(i) {
        Some(Tok::Str(s)) => Some(*s),
        _ => None,
    };
    for i in 0..tokens.len() {
        let Tok::Ident(word) = &tokens[i] else {
            continue;
        };
        if after_dot(i) {
            continue;
        }
        let mut add = |s: usize, kind: JsKind, scan: &mut JsScan| {
            if !used[s] && !scan.strings[s].substituted {
                used[s] = true;
                scan.refs.push(JsRef { string: s, kind });
            }
        };
        match word.as_str() {
            "import" => {
                if let Some(s) = string_at(i + 1) {
                    add(s, JsKind::Specifier, scan);
                } else if tokens.get(i + 1) == Some(&Tok::Punct(b'(')) {
                    if let Some(s) = string_at(i + 2) {
                        if matches!(
                            tokens.get(i + 3),
                            Some(Tok::Punct(b')')) | Some(Tok::Punct(b','))
                        ) {
                            add(s, JsKind::Specifier, scan);
                        }
                    }
                }
            }
            "from" => {
                if let Some(s) = string_at(i + 1) {
                    add(s, JsKind::Specifier, scan);
                }
            }
            "new" => {
                if let Some(Tok::Ident(class)) = tokens.get(i + 1) {
                    if (class == "Worker" || class == "SharedWorker")
                        && tokens.get(i + 2) == Some(&Tok::Punct(b'('))
                    {
                        if let Some(s) = string_at(i + 3) {
                            add(s, JsKind::Plain, scan);
                        }
                    }
                }
            }
            "importScripts" => {
                if tokens.get(i + 1) == Some(&Tok::Punct(b'(')) {
                    let mut k = i + 2;
                    while let Some(s) = string_at(k) {
                        add(s, JsKind::Plain, scan);
                        if tokens.get(k + 1) == Some(&Tok::Punct(b',')) {
                            k += 2;
                        } else {
                            break;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    for (s, string) in scan.strings.iter().enumerate() {
        if used[s] || string.substituted {
            continue;
        }
        if is_cdn_address(&string.value) {
            used[s] = true;
            scan.refs.push(JsRef {
                string: s,
                kind: JsKind::Plain,
            });
        }
    }
    scan.refs.sort_by_key(|r| scan.strings[r.string].span.start);
}

/// An absolute http(s) address on one of the known CDNs.
pub(crate) fn is_cdn_address(text: &str) -> bool {
    let lower = text.trim().to_ascii_lowercase();
    let rest = if let Some(rest) = lower.strip_prefix("https://") {
        rest
    } else if let Some(rest) = lower.strip_prefix("http://") {
        rest
    } else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit('@').next().unwrap_or(host);
    let host = host.split(':').next().unwrap_or(host);
    KNOWN_CDNS.contains(&host) && rest.len() > host.len() + 1
}
