//! What a reference names: a file of the page, an address on the web, or nothing to
//! fetch. The files of a page stand under a made-up origin while they are resolved, so
//! that a reference is joined to its base by the rules of the web (the `url` crate) in
//! one way for local files and for pieces from the web.

use percent_encoding::{AsciiSet, CONTROLS, percent_decode_str, utf8_percent_encode};
use url::Url;

/// The origin the page's own files stand under while references are resolved.
const LOCAL_HOST: &str = "page.innerword-page.invalid";

/// Characters of a path that are written percent-encoded into a reference: everything
/// but letters, digits and `-._~!$&*+,;=@/`, so that a path is safe in an attribute, a
/// CSS `url(…)` without quotes and a string of a script alike.
const PATH: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'\'')
    .add(b'(')
    .add(b')')
    .add(b':')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');

const FRAGMENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'\'')
    .add(b'(')
    .add(b')')
    .add(b'<')
    .add(b'>')
    .add(b'\\')
    .add(b'`');

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Resolved {
    /// Not a reference: empty, a `#fragment`, `data:`, `blob:`, `about:`, `javascript:`,
    /// `mailto:`, `tel:` or any scheme but http(s).
    Nothing,
    /// A file of the page: its path (decoded, no query), and the fragment as written.
    Local { path: String, fragment: Option<String> },
    /// An address on the web without its fragment, and the fragment.
    External { url: String, fragment: Option<String> },
    /// A path that leaves the root of the page.
    Outside,
}

/// The address a file of the page stands at while its references are resolved.
pub(crate) fn local_base(path: &str) -> Url {
    Url::parse(&format!("https://{LOCAL_HOST}/{}", encode_path(path))).expect("a local address parses")
}

/// The root of the page.
pub(crate) fn local_root() -> Url {
    local_base("")
}

pub(crate) fn is_local(url: &Url) -> bool {
    url.host_str() == Some(LOCAL_HOST)
}

/// The path of a local address, decoded, without the leading slash.
pub(crate) fn local_path(url: &Url) -> String {
    decode(url.path().trim_start_matches('/'))
}

pub(crate) fn decode(text: &str) -> String {
    percent_decode_str(text).decode_utf8_lossy().into_owned()
}

pub(crate) fn encode_path(path: &str) -> String {
    utf8_percent_encode(path, PATH).to_string()
}

fn scheme_of(text: &str) -> Option<String> {
    let colon = text.find(':')?;
    let scheme = &text[..colon];
    let mut chars = scheme.chars();
    let first = chars.next()?;
    if !first.is_ascii_alphabetic() || !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')) {
        return None;
    }
    Some(scheme.to_ascii_lowercase())
}

/// What `raw` (a reference as the file has it, character references already decoded)
/// names, joined to `base`.
pub(crate) fn resolve(raw: &str, base: &Url) -> Resolved {
    let raw = raw.trim_matches(|c: char| c.is_ascii_whitespace());
    if raw.is_empty() || raw.starts_with('#') {
        return Resolved::Nothing;
    }
    if let Some(scheme) = scheme_of(raw) {
        if scheme != "http" && scheme != "https" {
            return Resolved::Nothing;
        }
        return match Url::parse(raw) {
            Ok(url) => classify(url),
            Err(_) => Resolved::Nothing,
        };
    }
    let protocol_relative = raw.starts_with("//") || raw.starts_with("\\\\");
    if is_local(base) {
        if protocol_relative {
            return match Url::parse(&format!("https:{raw}")) {
                Ok(url) => classify(url),
                Err(_) => Resolved::Nothing,
            };
        }
        if leaves_root(&local_path(base), raw) {
            return Resolved::Outside;
        }
    }
    match base.join(raw) {
        Ok(url) => classify(url),
        Err(_) => Resolved::Nothing,
    }
}

fn classify(mut url: Url) -> Resolved {
    let fragment = url.fragment().map(|f| f.to_string()).filter(|f| !f.is_empty());
    if is_local(&url) {
        return Resolved::Local { path: local_path(&url), fragment };
    }
    if url.scheme() != "http" && url.scheme() != "https" {
        return Resolved::Nothing;
    }
    if url.host_str().map(str::is_empty).unwrap_or(true) {
        return Resolved::Nothing;
    }
    url.set_fragment(None);
    Resolved::External { url: url.to_string(), fragment }
}

/// The normal form of an absolute address the bundler keys pieces by: as the `url`
/// crate writes it, without its fragment. None for one that is not http(s).
pub(crate) fn normal_address(text: &str) -> Option<String> {
    let mut url = Url::parse(text.trim()).ok()?;
    if url.scheme() != "http" && url.scheme() != "https" || url.host_str().is_none() {
        return None;
    }
    url.set_fragment(None);
    Some(url.to_string())
}

/// Whether a relative reference from a file at `from` (a local path) climbs above the
/// root of the page.
fn leaves_root(from: &str, raw: &str) -> bool {
    let path = raw.split(['?', '#']).next().unwrap_or("").replace('\\', "/");
    let mut depth: usize = if path.starts_with('/') {
        0
    } else {
        from.split('/').count().saturating_sub(1)
    };
    for segment in path.split('/') {
        let lower = segment.to_ascii_lowercase();
        match lower.as_str() {
            "" | "." | "%2e" => {}
            ".." | ".%2e" | "%2e." | "%2e%2e" => {
                if depth == 0 {
                    return true;
                }
                depth -= 1;
            }
            _ => depth += 1,
        }
    }
    false
}

/// The reference from the file at `from` to the file at `to` (both paths of the page):
/// relative, percent-encoded, and with the fragment when there is one.
pub(crate) fn relative_reference(from: &str, to: &str, fragment: Option<&str>) -> String {
    let mut from_dir: Vec<&str> = from.split('/').collect();
    from_dir.pop();
    let to_segments: Vec<&str> = to.split('/').collect();
    let (to_dir, name) = to_segments.split_at(to_segments.len() - 1);
    let mut common = 0;
    while common < from_dir.len() && common < to_dir.len() && from_dir[common] == to_dir[common] {
        common += 1;
    }
    let mut out = String::new();
    for _ in common..from_dir.len() {
        out.push_str("../");
    }
    let rest: Vec<&str> = to_dir[common..].iter().chain(name.iter()).copied().collect();
    out.push_str(&encode_path(&rest.join("/")));
    with_fragment(out, fragment)
}

/// The reference to a file of the page from the root of its origin (`/_external/…`).
pub(crate) fn root_reference(to: &str, fragment: Option<&str>) -> String {
    with_fragment(format!("/{}", encode_path(to)), fragment)
}

fn with_fragment(mut reference: String, fragment: Option<&str>) -> String {
    if let Some(fragment) = fragment {
        reference.push('#');
        reference.push_str(&utf8_percent_encode(fragment, FRAGMENT).to_string());
    }
    reference
}

/// A module specifier must begin with `./`, `../` or `/`, or it is bare.
pub(crate) fn as_specifier(reference: String) -> String {
    if reference.starts_with("./") || reference.starts_with("../") || reference.starts_with('/') {
        reference
    } else {
        format!("./{reference}")
    }
}

/// An address with its fragment, as written back where a reference to the web stays a
/// reference to the web (a link, when a `<base>` the file had is removed).
pub(crate) fn absolute_reference(url: &str, fragment: Option<&str>) -> String {
    with_fragment(url.to_string(), fragment)
}
