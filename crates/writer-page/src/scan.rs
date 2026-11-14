//! Every reference of one file, with where it stands, what it is for and how a new one
//! is written in its place (docs/pages-design.md, section 3, "Finding every reference").

use std::ops::Range;

use url::Url;

use crate::css::{CssKind, scan_css};
use crate::html::{Tag, Token, tokenize};
use crate::importmap::{ImportMap, MapKind, scan_import_map};
use crate::js::{JsKind, scan_js};
use crate::media::{Kind, find};
use crate::resolve::{Resolved, resolve};

/// What a reference is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ctx {
    /// A subresource: fetched when it is on the web, taken when it is a local file.
    Fetch,
    /// A link, a form, a frame, a refresh: a local file is taken; an address on the web
    /// is left as it is.
    Link,
    /// A module specifier (`import`, `export … from`, `import()`).
    Specifier,
    /// A string of a script: a CDN address, `new Worker`, `importScripts`.
    ScriptString,
    /// A value of an import map: fetched, unless it is a prefix (ends in `/`).
    MapValue,
    /// A scope of an import map (an address prefix): never fetched, rewritten to the
    /// folder its pieces are stored in.
    MapScope,
}

/// What a fetched piece is expected to be, from where it is named.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Expect {
    Css,
    Js,
    Any,
}

/// How a new reference is written in place of the old one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Write {
    /// The bytes of the span are replaced by the new reference.
    Raw,
    /// The span is an attribute value with its quotes; it becomes `"before new after"`,
    /// escaped.
    Attr { before: String, after: String },
}

#[derive(Clone, Debug)]
pub(crate) struct Ref {
    pub span: Range<usize>,
    pub raw: String,
    pub ctx: Ctx,
    pub expect: Expect,
    pub write: Write,
    /// What the reference is joined to.
    pub base: Url,
    /// Written from the root of the origin (`/_external/…`) rather than relative to the
    /// file: a string of a script file that is not a module specifier, which the page
    /// resolves against its document, not against the script.
    pub root_relative: bool,
    /// Removed with it when it is rewritten to a stored copy (the `integrity` of its
    /// element: the stored copy may differ from the bytes the hash was made of).
    pub with: Vec<Range<usize>>,
    /// A link that is said when it is an address on the web (a form's action, a refresh).
    pub warn_external: bool,
    /// A link the reader follows (`<a href>`, `<area href>`, a form's action, a refresh)
    /// rather than one the page loads by itself (a frame): a single file takes the
    /// target only when it is a page of its own (an HTML file).
    pub navigation: bool,
}

/// An attribute whose value is a text of its own (`style`, `srcset`, `srcdoc`).
#[derive(Clone, Debug)]
pub(crate) struct Nested {
    /// The attribute value with its quotes.
    pub span: Range<usize>,
    /// The value, decoded.
    pub text: String,
    pub scan: Scan,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Place {
    /// A string literal of a script.
    Script,
    /// A string of a style sheet that is no reference.
    Css,
    /// A `data-*` attribute.
    DataAttr,
    /// Any other attribute value.
    Attr,
}

/// A string literal that is no reference: what the bundler says about the page is
/// made of these (an address not kept, a file beside a single file).
#[derive(Clone, Debug)]
pub(crate) struct Literal {
    pub value: String,
    pub base: Url,
    pub place: Place,
    pub substituted: bool,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Scan {
    pub refs: Vec<Ref>,
    pub nested: Vec<Nested>,
    pub removals: Vec<Range<usize>>,
    /// `//# sourceMappingURL=` comments (removed from a fetched file only).
    pub source_maps: Vec<Range<usize>>,
    pub literals: Vec<Literal>,
    /// Bare module specifiers, with the address of the module that names them.
    pub bare: Vec<(String, Url)>,
    /// The import maps of an HTML document, with the base their addresses are joined to.
    pub maps: Vec<(ImportMap, Url)>,
    /// A `<base href>` was taken out: every relative reference is written again.
    pub base_removed: bool,
}

impl Scan {
    /// Every reference, nested ones included, with the scan it belongs to.
    pub fn all_refs(&self) -> Vec<&Ref> {
        let mut out: Vec<&Ref> = self.refs.iter().collect();
        for nested in &self.nested {
            out.extend(nested.scan.all_refs());
        }
        out
    }

    pub fn all_literals(&self) -> Vec<&Literal> {
        let mut out: Vec<&Literal> = self.literals.iter().collect();
        for nested in &self.nested {
            out.extend(nested.scan.all_literals());
        }
        out
    }

    pub fn all_bare(&self) -> Vec<&(String, Url)> {
        let mut out: Vec<&(String, Url)> = self.bare.iter().collect();
        for nested in &self.nested {
            out.extend(nested.scan.all_bare());
        }
        out
    }

    pub fn all_maps(&self) -> Vec<&(ImportMap, Url)> {
        let mut out: Vec<&(ImportMap, Url)> = self.maps.iter().collect();
        for nested in &self.nested {
            out.extend(nested.scan.all_maps());
        }
        out
    }
}

/// The scan of a file by its kind, at `base` (its own address).
pub(crate) fn scan_file(kind: Kind, bytes: &[u8], base: &Url) -> Scan {
    match kind {
        Kind::Html => scan_html(bytes, base, false),
        Kind::Svg => scan_html(bytes, base, true),
        Kind::Css => scan_style(bytes, base),
        Kind::Js => scan_script(bytes, base, base, true),
        Kind::Other => Scan::default(),
    }
}

fn simple(span: Range<usize>, raw: String, ctx: Ctx, expect: Expect, base: &Url) -> Ref {
    Ref {
        span,
        raw,
        ctx,
        expect,
        write: Write::Raw,
        base: base.clone(),
        root_relative: false,
        with: Vec::new(),
        warn_external: false,
        navigation: false,
    }
}

fn attr_ref(tag: &Tag, name: &str, ctx: Ctx, expect: Expect, base: &Url) -> Option<Ref> {
    let attr = tag.attr(name)?;
    let value = attr.value.as_ref()?;
    let mut reference = simple(value.span.clone(), value.decoded.clone(), ctx, expect, base);
    reference.write = Write::Attr { before: String::new(), after: String::new() };
    if ctx == Ctx::Fetch {
        if let Some(integrity) = tag.attr("integrity") {
            reference.with.push(integrity.span.clone());
        }
    }
    Some(reference)
}

/// A style sheet (a file, a `<style>`, a `style` attribute).
pub(crate) fn scan_style(bytes: &[u8], base: &Url) -> Scan {
    let css = scan_css(bytes);
    let mut scan = Scan { source_maps: css.source_maps, ..Scan::default() };
    for item in css.items {
        match item.kind {
            CssKind::Plain => scan.literals.push(Literal {
                value: item.value,
                base: base.clone(),
                place: Place::Css,
                substituted: false,
            }),
            CssKind::Import => scan.refs.push(simple(item.span, item.value, Ctx::Fetch, Expect::Css, base)),
            CssKind::Url | CssKind::ImageSet => {
                scan.refs.push(simple(item.span, item.value, Ctx::Fetch, Expect::Any, base))
            }
        }
    }
    scan
}

/// A script: a file at `own` (specifiers are joined to it) or an inline script of a
/// document at `document` (both the same then). `file` says which.
pub(crate) fn scan_script(bytes: &[u8], own: &Url, document: &Url, file: bool) -> Scan {
    let js = scan_js(bytes);
    let mut scan = Scan { source_maps: js.source_maps.clone(), ..Scan::default() };
    for reference in &js.refs {
        let string = &js.strings[reference.string];
        match reference.kind {
            JsKind::Specifier => {
                if is_bare(&string.value) {
                    scan.bare.push((string.value.clone(), own.clone()));
                    continue;
                }
                scan.refs.push(simple(string.span.clone(), string.value.clone(), Ctx::Specifier, Expect::Js, own));
            }
            JsKind::Plain => {
                let mut r = simple(string.span.clone(), string.value.clone(), Ctx::ScriptString, Expect::Any, document);
                r.root_relative = file;
                scan.refs.push(r);
            }
        }
    }
    for string in js.other_strings() {
        scan.literals.push(Literal {
            value: string.value.clone(),
            base: document.clone(),
            place: Place::Script,
            substituted: string.substituted,
        });
    }
    scan
}

/// A module specifier that is neither relative nor absolute (`react`, `lib/x.js`): the
/// import map says what it is.
fn is_bare(specifier: &str) -> bool {
    let s = specifier.trim();
    !(s.starts_with("./")
        || s.starts_with("../")
        || s.starts_with('/')
        || s.contains("://")
        || s.starts_with("data:")
        || s.starts_with("blob:"))
}

fn link_rels(tag: &Tag) -> Vec<String> {
    tag.value("rel")
        .unwrap_or("")
        .split_ascii_whitespace()
        .map(|rel| rel.to_ascii_lowercase())
        .collect()
}

const FETCHED_RELS: &[&str] = &[
    "stylesheet",
    "icon",
    "apple-touch-icon",
    "apple-touch-icon-precomposed",
    "mask-icon",
    "preload",
    "modulepreload",
    "manifest",
    "prefetch",
];
const DROPPED_RELS: &[&str] = &["preconnect", "dns-prefetch", "prerender"];

fn is_script_type(tag: &Tag) -> bool {
    let kind = tag.value("type").unwrap_or("").trim().to_ascii_lowercase();
    let kind = kind.split(';').next().unwrap_or("").trim().to_string();
    matches!(
        kind.as_str(),
        "" | "module" | "text/javascript" | "application/javascript" | "application/x-javascript"
            | "text/ecmascript" | "application/ecmascript" | "text/babel" | "text/jsx" | "jsx"
    )
}

/// An HTML document, or an SVG (`xml`), at `file_base`.
pub(crate) fn scan_html(bytes: &[u8], file_base: &Url, xml: bool) -> Scan {
    let tokens = tokenize(bytes, xml);
    let mut scan = Scan::default();

    // `<base href>`: the first one gives every relative reference its base; all go.
    let mut base = file_base.clone();
    let mut base_found = false;
    for token in &tokens {
        if let Token::Start(tag) = token {
            if tag.name == "base" && !xml {
                scan.removals.push(tag.span.clone());
                if base_found {
                    continue;
                }
                if let Some(href) = tag.value("href") {
                    if let Ok(url) = file_base.join(href.trim()) {
                        if !matches!(resolve(href, file_base), Resolved::Outside | Resolved::Nothing) {
                            base = url;
                            base_found = true;
                            scan.base_removed = true;
                        }
                    }
                }
            }
        }
    }

    let mut open_script: Option<Tag> = None;
    for token in &tokens {
        match token {
            Token::Start(tag) => {
                open_script = None;
                if tag.name == "base" && !xml {
                    continue;
                }
                if tag.name == "script" {
                    open_script = Some(tag.clone());
                }
                scan_tag(tag, &base, xml, &mut scan);
            }
            Token::Raw { element, span } => {
                let text = &bytes[span.clone()];
                match element.as_str() {
                    "script" => {
                        let tag = open_script.take();
                        let kind = tag.as_ref().and_then(|t| t.value("type")).unwrap_or("").trim().to_ascii_lowercase();
                        if kind == "importmap" {
                            let map = scan_import_map(text);
                            for item in &map.items {
                                let ctx = if item.kind == MapKind::ScopeKey { Ctx::MapScope } else { Ctx::MapValue };
                                scan.refs.push(simple(
                                    span.start + item.span.start..span.start + item.span.end,
                                    item.value.clone(),
                                    ctx,
                                    Expect::Js,
                                    &base,
                                ));
                            }
                            scan.maps.push((map, base.clone()));
                        } else if tag.as_ref().map(is_script_type).unwrap_or(true) {
                            let inner = scan_script(text, &base, &base, false);
                            merge(&mut scan, inner, span.start);
                        }
                    }
                    "style" => {
                        let inner = scan_style(text, &base);
                        merge(&mut scan, inner, span.start);
                    }
                    _ => {}
                }
            }
            _ => {
                open_script = None;
            }
        }
    }
    scan
}

/// Adds the scan of a slice that begins at `offset` of the file.
fn merge(scan: &mut Scan, inner: Scan, offset: usize) {
    let shift = |r: &Range<usize>| r.start + offset..r.end + offset;
    for mut reference in inner.refs {
        reference.span = shift(&reference.span);
        reference.with = reference.with.iter().map(shift).collect();
        scan.refs.push(reference);
    }
    scan.removals.extend(inner.removals.iter().map(shift));
    scan.source_maps.extend(inner.source_maps.iter().map(shift));
    scan.literals.extend(inner.literals);
    scan.bare.extend(inner.bare);
    scan.maps.extend(inner.maps);
    // Nested texts only come from attributes, never from a script or a style.
}

fn scan_tag(tag: &Tag, base: &Url, xml: bool, scan: &mut Scan) {
    let name = tag.name.as_str();
    let mut handled: Vec<&str> = Vec::new();
    match name {
        "link" => {
            handled.push("href");
            let rels = link_rels(tag);
            if rels.iter().any(|rel| FETCHED_RELS.contains(&rel.as_str())) {
                let as_ = tag.value("as").unwrap_or("").trim().to_ascii_lowercase();
                let expect = if rels.iter().any(|r| r == "stylesheet") || as_ == "style" {
                    Expect::Css
                } else if rels.iter().any(|r| r == "modulepreload") || as_ == "script" || as_ == "worker" {
                    Expect::Js
                } else {
                    Expect::Any
                };
                if let Some(r) = attr_ref(tag, "href", Ctx::Fetch, expect, base) {
                    scan.refs.push(r);
                }
            } else if rels.iter().any(|rel| DROPPED_RELS.contains(&rel.as_str())) {
                let external = tag
                    .value("href")
                    .map(|href| matches!(resolve(href, base), Resolved::External { .. }))
                    .unwrap_or(false);
                if external {
                    scan.removals.push(tag.span.clone());
                    return;
                }
            }
        }
        "a" | "area" => {
            handled.extend(["href", "xlink:href"]);
            for attr in ["href", "xlink:href"] {
                if let Some(mut r) = attr_ref(tag, attr, Ctx::Link, Expect::Any, base) {
                    r.navigation = true;
                    scan.refs.push(r);
                }
            }
        }
        "iframe" | "frame" => {
            handled.push("src");
            if let Some(r) = attr_ref(tag, "src", Ctx::Link, Expect::Any, base) {
                scan.refs.push(r);
            }
        }
        "meta" => {
            let refresh = tag.value("http-equiv").map(|v| v.trim().eq_ignore_ascii_case("refresh")).unwrap_or(false);
            if refresh {
                handled.push("content");
                if let Some(r) = refresh_ref(tag, base) {
                    scan.refs.push(r);
                }
            }
        }
        _ => {}
    }
    for attr in &tag.attrs {
        let attr_name = attr.name.as_str();
        if handled.contains(&attr_name) {
            continue;
        }
        let Some(value) = &attr.value else { continue };
        match attr_name {
            "src" | "poster" | "background" => {
                if let Some(r) = attr_ref(tag, attr_name, Ctx::Fetch, if name == "script" { Expect::Js } else { Expect::Any }, base) {
                    scan.refs.push(r);
                }
            }
            "data" if name == "object" => {
                if let Some(r) = attr_ref(tag, "data", Ctx::Fetch, Expect::Any, base) {
                    scan.refs.push(r);
                }
            }
            "href" | "xlink:href" => {
                // Outside `a`, `area` and `link` (handled above) an href names a resource:
                // SVG's `use`, `image`, `feImage`, `script`, a gradient, a pattern.
                if name != "base" && !(name == "link") {
                    if let Some(r) = attr_ref(tag, attr_name, Ctx::Fetch, if name == "script" { Expect::Js } else { Expect::Any }, base) {
                        scan.refs.push(r);
                    }
                }
            }
            "action" | "formaction" => {
                if let Some(mut r) = attr_ref(tag, attr_name, Ctx::Link, Expect::Any, base) {
                    r.warn_external = true;
                    r.navigation = true;
                    scan.refs.push(r);
                }
            }
            "srcset" | "imagesrcset" => {
                let text = value.decoded.clone();
                let inner = srcset(&text, base);
                scan.nested.push(Nested { span: value.span.clone(), text, scan: inner });
            }
            "style" => {
                let text = value.decoded.clone();
                let inner = scan_style(text.as_bytes(), base);
                scan.nested.push(Nested { span: value.span.clone(), text, scan: inner });
            }
            "srcdoc" if name == "iframe" => {
                let text = value.decoded.clone();
                let inner = scan_html(text.as_bytes(), base, false);
                scan.nested.push(Nested { span: value.span.clone(), text, scan: inner });
            }
            _ if xml && attr_name.starts_with("xmlns") => {}
            // An event handler (`onclick`) is a script: its strings are said as a
            // script's are (a file beside a single file, an address not kept), and an
            // address it names is kept as a script's is. It never runs in the check
            // unless the page is clicked, so the check cannot find what it asks for.
            _ if is_event_handler(attr_name) => {
                let text = value.decoded.clone();
                let inner = scan_script(text.as_bytes(), base, base, false);
                scan.nested.push(Nested { span: value.span.clone(), text, scan: inner });
            }
            _ => {
                let place = if attr_name.starts_with("data-") { Place::DataAttr } else { Place::Attr };
                scan.literals.push(Literal {
                    value: value.decoded.clone(),
                    base: base.clone(),
                    place,
                    substituted: false,
                });
            }
        }
    }
}

/// `onclick`, `onload`, `onpointerdown`: an attribute whose value is a script.
fn is_event_handler(name: &str) -> bool {
    // Bytes, not a slice of the str: an attribute's name can begin with a character of
    // several bytes (a page of the archive has one), and `name[..2]` panicked there.
    let bytes = name.as_bytes();
    bytes.len() > 2
        && bytes[..2].eq_ignore_ascii_case(b"on")
        && bytes.iter().all(|b| b.is_ascii_alphabetic())
}

/// `<meta http-equiv=refresh content="5; url=next.html">`.
fn refresh_ref(tag: &Tag, base: &Url) -> Option<Ref> {
    let attr = tag.attr("content")?;
    let value = attr.value.as_ref()?;
    let content = value.decoded.as_str();
    let lower = content.to_ascii_lowercase();
    let at = find(lower.as_bytes(), b"url")?;
    let mut k = at + 3;
    let bytes = content.as_bytes();
    while k < bytes.len() && bytes[k].is_ascii_whitespace() {
        k += 1;
    }
    if k >= bytes.len() || bytes[k] != b'=' {
        return None;
    }
    k += 1;
    while k < bytes.len() && bytes[k].is_ascii_whitespace() {
        k += 1;
    }
    let mut end = bytes.len();
    let mut start = k;
    if k < bytes.len() && (bytes[k] == b'"' || bytes[k] == b'\'') {
        let quote = bytes[k];
        start = k + 1;
        end = content[start..].find(quote as char).map(|e| start + e).unwrap_or(bytes.len());
    }
    let address = content[start..end].trim().to_string();
    if address.is_empty() {
        return None;
    }
    let mut r = simple(value.span.clone(), address, Ctx::Link, Expect::Any, base);
    r.write = Write::Attr { before: content[..start].to_string(), after: content[end..].to_string() };
    r.warn_external = true;
    r.navigation = true;
    Some(r)
}

/// The candidates of a `srcset`: each address, as a span of the decoded value.
fn srcset(text: &str, base: &Url) -> Scan {
    let bytes = text.as_bytes();
    let n = bytes.len();
    let mut scan = Scan::default();
    let mut i = 0;
    while i < n {
        while i < n && (bytes[i].is_ascii_whitespace() || bytes[i] == b',') {
            i += 1;
        }
        if i >= n {
            break;
        }
        let start = i;
        while i < n && !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut end = i;
        let mut ends_candidate = false;
        while end > start && bytes[end - 1] == b',' {
            end -= 1;
            ends_candidate = true;
        }
        if end > start {
            scan.refs.push(simple(start..end, text[start..end].to_string(), Ctx::Fetch, Expect::Any, base));
        }
        if !ends_candidate {
            // The descriptors, up to the comma that ends the candidate (outside parentheses).
            let mut depth = 0;
            while i < n {
                match bytes[i] {
                    b'(' => depth += 1,
                    b')' => depth = (depth as i32 - 1).max(0) as usize,
                    b',' if depth == 0 => break,
                    _ => {}
                }
                i += 1;
            }
        }
    }
    scan
}
