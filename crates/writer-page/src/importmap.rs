//! `<script type=importmap>`: where its addresses stand in the JSON (the values of
//! `imports`, the keys and values of every scope in `scopes`), so that only their bytes
//! are replaced, and the map as data, for the bare specifiers of the page's modules.

use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum MapKind {
    /// A value of `imports`.
    Import,
    /// A key of `scopes` (an address prefix).
    ScopeKey,
    /// A value inside a scope.
    ScopeValue,
}

#[derive(Clone, Debug)]
pub(crate) struct MapItem {
    /// The bytes between the quotes.
    pub span: Range<usize>,
    pub value: String,
    pub kind: MapKind,
    /// The specifier the value is for (Import, ScopeValue).
    pub key: String,
    /// The scope it is in (ScopeKey: itself; ScopeValue: its scope).
    pub scope: String,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ImportMap {
    pub items: Vec<MapItem>,
}

struct Frame {
    object: bool,
    key: Option<String>,
    expecting_key: bool,
}

pub(crate) fn scan_import_map(bytes: &[u8]) -> ImportMap {
    let n = bytes.len();
    let mut map = ImportMap::default();
    let mut frames: Vec<Frame> = Vec::new();
    let mut i = 0;
    while i < n {
        let b = bytes[i];
        match b {
            b'{' | b'[' => {
                frames.push(Frame { object: b == b'{', key: None, expecting_key: b == b'{' });
                i += 1;
            }
            b'}' | b']' => {
                frames.pop();
                i += 1;
            }
            b',' => {
                if let Some(top) = frames.last_mut() {
                    if top.object {
                        top.expecting_key = true;
                    }
                }
                i += 1;
            }
            b':' => {
                if let Some(top) = frames.last_mut() {
                    top.expecting_key = false;
                }
                i += 1;
            }
            b'"' => {
                let (end, value) = json_string(bytes, i);
                let span = i + 1..end.saturating_sub(1).max(i + 1);
                let depth = frames.len();
                let is_key = frames.last().map(|f| f.object && f.expecting_key).unwrap_or(false);
                let root_key = frames.first().and_then(|f| f.key.clone()).unwrap_or_default();
                if is_key {
                    if depth == 2 && root_key == "scopes" {
                        map.items.push(MapItem {
                            span,
                            value: value.clone(),
                            kind: MapKind::ScopeKey,
                            key: String::new(),
                            scope: value.clone(),
                        });
                    }
                    if let Some(top) = frames.last_mut() {
                        top.key = Some(value);
                    }
                } else if depth == 2 && root_key == "imports" {
                    let key = frames[1].key.clone().unwrap_or_default();
                    map.items.push(MapItem { span, value, kind: MapKind::Import, key, scope: String::new() });
                } else if depth == 3 && root_key == "scopes" {
                    let scope = frames[1].key.clone().unwrap_or_default();
                    let key = frames[2].key.clone().unwrap_or_default();
                    map.items.push(MapItem { span, value, kind: MapKind::ScopeValue, key, scope });
                }
                i = end;
            }
            _ => i += 1,
        }
    }
    map
}

/// A JSON string from its quote: where it ends (after the closing quote) and its value.
fn json_string(bytes: &[u8], at: usize) -> (usize, String) {
    let n = bytes.len();
    let mut k = at + 1;
    while k < n && bytes[k] != b'"' {
        if bytes[k] == b'\\' {
            k += 1;
        }
        k += 1;
    }
    let end = (k + 1).min(n);
    let raw = &bytes[at..end];
    let value = serde_json::from_slice::<String>(raw)
        .unwrap_or_else(|_| String::from_utf8_lossy(&bytes[at + 1..k.min(n)]).into_owned());
    (end, value)
}

impl ImportMap {
    /// The address a bare specifier stands for, from a module at `referrer`: the scopes
    /// whose prefix the referrer has (the longest first), then `imports`; an exact key,
    /// else the longest key ending in `/` that the specifier begins with.
    pub fn resolve(&self, specifier: &str, referrer: &str) -> Option<String> {
        let mut scopes: Vec<&str> = self
            .items
            .iter()
            .filter(|item| item.kind == MapKind::ScopeKey && referrer.starts_with(item.value.as_str()))
            .map(|item| item.value.as_str())
            .collect();
        scopes.sort_by_key(|scope| std::cmp::Reverse(scope.len()));
        for scope in scopes {
            let entries = self
                .items
                .iter()
                .filter(|item| item.kind == MapKind::ScopeValue && item.scope == scope);
            if let Some(found) = lookup(entries, specifier) {
                return Some(found);
            }
        }
        lookup(self.items.iter().filter(|item| item.kind == MapKind::Import), specifier)
    }
}

fn lookup<'a>(entries: impl Iterator<Item = &'a MapItem>, specifier: &str) -> Option<String> {
    let mut best: Option<(&MapItem, usize)> = None;
    for item in entries {
        if item.key == specifier {
            return Some(item.value.clone());
        }
        if item.key.ends_with('/') && specifier.starts_with(item.key.as_str()) {
            if best.map(|(_, len)| item.key.len() > len).unwrap_or(true) {
                best = Some((item, item.key.len()));
            }
        }
    }
    best.map(|(item, len)| format!("{}{}", item.value, &specifier[len..]))
}
