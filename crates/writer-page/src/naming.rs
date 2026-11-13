//! Where a piece from the web is stored among the served files:
//! `_external/<host>/<path segments>`, the scheme of `archive.py`'s `local_path_for`, which
//! produced the archive.

use std::collections::HashMap;

use url::Url;

use crate::EXTERNAL_DIR;
use crate::media::{Kind, sha1_hex};
use crate::resolve::decode;

/// Every character outside `[A-Za-z0-9._@+-]` written as `_`.
fn safe(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '@' | '+' | '-') { c } else { '_' })
        .collect()
}

fn netloc(url: &Url) -> String {
    let host = url.host_str().unwrap_or("");
    match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    }
}

fn segments(url: &Url) -> Vec<String> {
    url.path()
        .split('/')
        .filter(|s| !s.is_empty() && *s != "." && *s != "..")
        .map(|s| safe(&decode(s)))
        .collect()
}

/// Python's `os.path.splitext`: the ending from the last dot that is not the first
/// character.
fn split_ending(name: &str) -> (String, String) {
    let trimmed = name.trim_start_matches('.');
    let lead = name.len() - trimmed.len();
    match trimmed.rfind('.') {
        Some(at) => {
            let at = lead + at;
            (name[..at].to_string(), name[at..].to_string())
        }
        None => (name.to_string(), String::new()),
    }
}

/// The stored path of the piece at `url`. `taken` holds every path given out (and the
/// page's own files), each with the address it stands for; two addresses that come out
/// the same get `__<8 hex of the address>`.
pub(crate) fn external_path(url: &str, kind: Kind, taken: &mut HashMap<String, String>) -> String {
    let parsed = Url::parse(url).expect("an external is an absolute address");
    let mut parts = segments(&parsed);
    if parts.is_empty() || parsed.path().ends_with('/') {
        parts.push("index".to_string());
    }
    let name = parts.pop().unwrap_or_default();
    let (mut base, mut ending) = split_ending(&name);
    if let Some(query) = parsed.query().filter(|q| !q.is_empty()) {
        base = format!("{base}__{}", &sha1_hex(query)[..10]);
    }
    if kind == Kind::Css && ending != ".css" {
        base.push_str(&ending);
        ending = ".css".to_string();
    }
    if kind == Kind::Js && ending != ".js" && ending != ".mjs" {
        base.push_str(&ending);
        ending = ".js".to_string();
    }
    parts.push(format!("{base}{ending}"));
    let mut path = format!("{EXTERNAL_DIR}/{}/{}", safe(&netloc(&parsed)), parts.join("/"));
    if taken.get(&path).map(|owner| owner != url).unwrap_or(false) {
        let mark = &sha1_hex(url)[..8];
        path = if ending.is_empty() {
            format!("{path}__{mark}")
        } else {
            format!("{}__{mark}{ending}", &path[..path.len() - ending.len()])
        };
    }
    taken.insert(path.clone(), url.to_string());
    path
}

/// The folder the pieces under an address prefix are stored in
/// (`https://unpkg.com/three@0.160.0/examples/jsm/` ->
/// `_external/unpkg.com/three@0.160.0/examples/jsm/`), for the prefixes of an import map.
pub(crate) fn external_dir(url: &str) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    let parts = segments(&parsed);
    let mut path = format!("{EXTERNAL_DIR}/{}/", safe(&netloc(&parsed)));
    for part in parts {
        path.push_str(&part);
        path.push('/');
    }
    Some(path)
}
