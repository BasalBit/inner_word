//! Input to served files: every reference found, each piece from the web fetched once and
//! stored under `_external/`, every reference to one rewritten to the stored copy
//! (docs/pages-design.md, section 3).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt;
use std::ops::Range;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use url::Url;

use crate::check::static_title;
use crate::error::{PageError, Problem, count};
use crate::fetch::{FetchError, Fetched, Fetcher};
use crate::html::quoted_attr;
use crate::importmap::{ImportMap, MapKind};
use crate::input::{Input, too_heavy, too_large, too_many_files};
use crate::limits::{ALSO_LOADS, FETCH_BUDGET, FETCH_PARALLEL, MAX_DEPTH, MAX_EXTERNALS, MAX_FILE_BYTES, MAX_FILES, MAX_PAGE_BYTES};
use crate::media::{Kind, essence, file_id_hex, find, is_html_type, media_type, served_type, OCTET_STREAM};
use crate::naming::{external_dir, external_path};
use crate::resolve::{
    Resolved, absolute_reference, as_specifier, local_base, local_root, normal_address, relative_reference, resolve,
    root_reference,
};
use crate::scan::{Ctx, Expect, Place, Ref, Scan, Write, scan_file};

/// The served files of a page, the original files, and what was fetched.
#[derive(Clone)]
pub struct Bundle {
    pub name: String,
    pub entry: String,
    pub fingerprint: String,
    /// Sorted by path.
    pub served: Vec<BundleFile>,
    /// The input as it was given, sorted by path.
    pub original: Vec<BundleFile>,
    /// Sorted by address.
    pub externals: Vec<External>,
    /// The input's, the addresses not kept, window.claude (section 3).
    pub warnings: Vec<String>,
}

#[derive(Clone)]
pub struct BundleFile {
    pub path: String,
    pub media_type: String,
    pub bytes: Arc<[u8]>,
}

impl fmt::Debug for BundleFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BundleFile {{ path: {:?}, media_type: {:?}, bytes: {} }}", self.path, self.media_type, self.bytes.len())
    }
}

impl fmt::Debug for Bundle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Bundle")
            .field("name", &self.name)
            .field("entry", &self.entry)
            .field("fingerprint", &self.fingerprint)
            .field("served", &self.served)
            .field("original", &self.original)
            .field("externals", &self.externals)
            .field("warnings", &self.warnings)
            .finish()
    }
}

/// A piece from the web: the address the page names (without its fragment) and the
/// served path of its stored copy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct External {
    pub url: String,
    pub path: String,
}

impl Bundle {
    /// The served HTML files: the entry first, then the others by path.
    pub fn html_paths(&self) -> Vec<String> {
        let mut out = vec![self.entry.clone()];
        for file in &self.served {
            if file.path != self.entry && is_html_type(&file.media_type) {
                out.push(file.path.clone());
            }
        }
        out
    }

    /// The distinct bytes of the page, served and original files together.
    pub fn distinct_bytes(&self) -> u64 {
        let mut seen: HashSet<String> = HashSet::new();
        let mut total = 0;
        for file in self.served.iter().chain(self.original.iter()) {
            if seen.insert(file_id_hex(&file.bytes)) {
                total += file.bytes.len() as u64;
            }
        }
        total
    }

    /// The served file at a path.
    pub fn served_file(&self, path: &str) -> Option<&BundleFile> {
        self.served.iter().find(|file| file.path == path)
    }
}

pub(crate) fn too_many_html(html: usize) -> Problem {
    Problem::new(
        "the page",
        format!("the page has {} HTML pages; at most {} can be checked", count(html), count(ALSO_LOADS + 1)),
    )
}

struct LocalFile {
    path: String,
    bytes: Arc<[u8]>,
    scan: Option<Scan>,
}

struct Piece {
    fetched: Fetched,
    kind: Kind,
    media_type: String,
    scan: Option<Scan>,
    path: String,
}

pub fn bundle(input: Input, fetcher: &dyn Fetcher) -> Result<Bundle, PageError> {
    let entry_bytes = input.files.iter().find(|f| f.path == input.entry).map(|f| f.bytes.clone());
    let shown = entry_bytes.as_deref().and_then(static_title).unwrap_or_else(|| input.name.clone());
    let refuse = |problems: Vec<Problem>| PageError::Incomplete { name: shown.clone(), problems };

    if entry_bytes.is_none() {
        return Err(PageError::input(format!("the page has no file {}", input.entry)));
    }
    let html = input.files.iter().filter(|f| Kind::of_path(&f.path) == Kind::Html).count();
    if html > ALSO_LOADS + 1 {
        return Err(refuse(vec![too_many_html(html)]));
    }
    if input.files.len() > MAX_FILES {
        return Err(refuse(vec![too_many_files(input.files.len())]));
    }
    for file in &input.files {
        if file.bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(refuse(vec![too_large(&file.path, file.bytes.len() as u64)]));
        }
    }

    // The page's own files and their references.
    let locals: Vec<LocalFile> = input
        .files
        .iter()
        .map(|file| {
            let kind = Kind::of_path(&file.path);
            let scan = kind.is_text().then(|| scan_file(kind, &file.bytes, &local_base(&file.path)));
            LocalFile { path: file.path.clone(), bytes: file.bytes.clone(), scan }
        })
        .collect();
    let maps = absolute_maps(locals.iter().filter_map(|l| l.scan.as_ref()));

    let mut wanted: BTreeMap<String, Expect> = BTreeMap::new();
    for scan in locals.iter().filter_map(|l| l.scan.as_ref()) {
        want_from(scan, &maps, &mut wanted, &HashSet::new());
    }

    // Fetching, level by level: what a fetched style sheet or module names comes next.
    let started = Instant::now();
    let mut pieces: BTreeMap<String, Piece> = BTreeMap::new();
    let mut problems: Vec<Problem> = Vec::new();
    let mut seen: HashSet<String> = wanted.keys().cloned().collect();
    let mut depth = 0;
    while !wanted.is_empty() {
        depth += 1;
        if seen.len() > MAX_EXTERNALS {
            problems.push(Problem::new(
                "the page",
                format!(
                    "the page names more than {} pieces on the web; at most {} can be kept",
                    count(MAX_EXTERNALS),
                    count(MAX_EXTERNALS)
                ),
            ));
            break;
        }
        if depth > MAX_DEPTH {
            let first = wanted.keys().next().cloned().unwrap_or_default();
            problems.push(Problem::new(
                first.clone(),
                format!(
                    "the pieces the page names on the web name others more than {MAX_DEPTH} deep (style sheets that import style sheets, modules that import modules), so {first} and what it needs were not fetched"
                ),
            ));
            break;
        }
        let level: Vec<(String, Expect)> = std::mem::take(&mut wanted).into_iter().collect();
        let urls: Vec<String> = level.iter().map(|(url, _)| url.clone()).collect();
        let results = fetch_all(fetcher, &urls, started);
        for ((url, expect), result) in level.into_iter().zip(results) {
            let fetched = match result {
                Ok(fetched) => fetched,
                Err(error) => {
                    problems.push(Problem::new(url.clone(), error.sentence()));
                    continue;
                }
            };
            if fetched.bytes.len() as u64 > MAX_FILE_BYTES {
                problems.push(too_large(&url, fetched.bytes.len() as u64));
                continue;
            }
            let kind = kind_of(&fetched, expect);
            let media = piece_type(&fetched, kind);
            let scan = if kind.is_text() {
                Url::parse(&fetched.final_url).ok().map(|base| scan_file(kind, &fetched.bytes, &base))
            } else {
                None
            };
            if let Some(scan) = &scan {
                let mut found = BTreeMap::new();
                want_from(scan, &maps, &mut found, &seen);
                for (next, expect) in found {
                    if seen.insert(next.clone()) {
                        wanted.insert(next, expect);
                    }
                }
            }
            pieces.insert(url, Piece { fetched, kind, media_type: media, scan, path: String::new() });
        }
    }
    if !problems.is_empty() {
        return Err(refuse(problems));
    }

    // Where each piece is stored.
    let mut taken: HashMap<String, String> = locals.iter().map(|l| (l.path.clone(), String::new())).collect();
    for (url, piece) in pieces.iter_mut() {
        piece.path = external_path(url, piece.kind, &mut taken);
    }
    let rewriter = Rewriter { stored: pieces.iter().map(|(url, piece)| (url.clone(), piece.path.clone())).collect() };

    // The served files: the page's own, rewritten where they name a piece, and the pieces.
    let mut served: Vec<BundleFile> = Vec::with_capacity(locals.len() + pieces.len());
    for local in &locals {
        let bytes: Arc<[u8]> = match &local.scan {
            Some(scan) => match rewrite(&local.bytes, scan, &local.path, &rewriter, false, false) {
                Some(new) => Arc::from(new),
                None => local.bytes.clone(),
            },
            None => local.bytes.clone(),
        };
        served.push(BundleFile { path: local.path.clone(), media_type: served_type(&local.path, &bytes), bytes });
    }
    for piece in pieces.values() {
        let bytes: Vec<u8> = match &piece.scan {
            Some(scan) => rewrite(&piece.fetched.bytes, scan, &piece.path, &rewriter, true, false)
                .unwrap_or_else(|| piece.fetched.bytes.clone()),
            None => piece.fetched.bytes.clone(),
        };
        served.push(BundleFile { path: piece.path.clone(), media_type: piece.media_type.clone(), bytes: Arc::from(bytes) });
    }
    served.sort_by(|a, b| a.path.cmp(&b.path));
    let original: Vec<BundleFile> = input
        .files
        .iter()
        .map(|file| BundleFile {
            path: file.path.clone(),
            media_type: served_type(&file.path, &file.bytes),
            bytes: file.bytes.clone(),
        })
        .collect();
    let externals: Vec<External> =
        pieces.iter().map(|(url, piece)| External { url: url.clone(), path: piece.path.clone() }).collect();

    let mut warnings = input.warnings.clone();
    warnings.extend(not_kept(&locals, &pieces));
    if locals.iter().any(|l| l.scan.is_some() && uses_claude(&l.bytes)) {
        warnings.push(
            "it uses claude.ai's storage (window.claude): what it saved there is not in this copy".to_string(),
        );
    }

    let bundle = Bundle {
        name: input.name,
        entry: input.entry,
        fingerprint: input.fingerprint,
        served,
        original,
        externals,
        warnings,
    };

    // The limits of the whole, now that it is whole.
    let mut over = Vec::new();
    for file in &bundle.served {
        if file.bytes.len() as u64 > MAX_FILE_BYTES {
            over.push(too_large(&file.path, file.bytes.len() as u64));
        }
    }
    if bundle.served.len() > MAX_FILES {
        over.push(too_many_files(bundle.served.len()));
    }
    let html = bundle.html_paths().len();
    if html > ALSO_LOADS + 1 {
        over.push(too_many_html(html));
    }
    let bytes = bundle.distinct_bytes();
    if bytes > MAX_PAGE_BYTES {
        over.push(too_heavy(bytes));
    }
    if !over.is_empty() {
        return Err(refuse(over));
    }
    Ok(bundle)
}

/// The import maps of the page with their addresses made absolute.
fn absolute_maps<'a>(scans: impl Iterator<Item = &'a Scan>) -> Vec<ImportMap> {
    let mut out = Vec::new();
    for scan in scans {
        for (map, base) in scan.all_maps() {
            let mut map = map.clone();
            for item in &mut map.items {
                let join = |text: &str| base.join(text.trim()).map(|u| u.to_string()).unwrap_or_else(|_| text.to_string());
                match item.kind {
                    MapKind::ScopeKey => {
                        item.value = join(&item.value);
                        item.scope = item.value.clone();
                    }
                    MapKind::ScopeValue => {
                        item.value = join(&item.value);
                        item.scope = join(&item.scope);
                    }
                    MapKind::Import => item.value = join(&item.value),
                }
            }
            out.push(map);
        }
    }
    out
}

/// The address on the web a reference has fetched, when it has one fetched.
fn fetch_target(reference: &Ref) -> Option<String> {
    match reference.ctx {
        Ctx::Link | Ctx::MapScope => return None,
        Ctx::MapValue if reference.raw.trim_end().ends_with('/') => return None,
        _ => {}
    }
    match resolve(&reference.raw, &reference.base) {
        Resolved::External { url, .. } => Some(url),
        _ => None,
    }
}

/// Every address a scan has fetched that is not in `seen`, into `wanted`.
fn want_from(scan: &Scan, maps: &[ImportMap], wanted: &mut BTreeMap<String, Expect>, seen: &HashSet<String>) {
    for reference in scan.all_refs() {
        if let Some(url) = fetch_target(reference) {
            if !seen.contains(&url) {
                wanted.entry(url).or_insert(reference.expect);
            }
        }
    }
    for (specifier, referrer) in scan.all_bare() {
        for map in maps {
            let Some(target) = map.resolve(specifier, referrer.as_str()) else { continue };
            if let Resolved::External { url, .. } = resolve(&target, &local_root()) {
                if !target.trim_end().ends_with('/') && !seen.contains(&url) {
                    wanted.entry(url).or_insert(Expect::Js);
                }
            }
        }
    }
}

fn fetch_all(fetcher: &dyn Fetcher, urls: &[String], started: Instant) -> Vec<Result<Fetched, FetchError>> {
    let results: Vec<Mutex<Option<Result<Fetched, FetchError>>>> = urls.iter().map(|_| Mutex::new(None)).collect();
    let next = AtomicUsize::new(0);
    let workers = FETCH_PARALLEL.min(urls.len());
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::SeqCst);
                    if i >= urls.len() {
                        break;
                    }
                    let result = if started.elapsed() >= FETCH_BUDGET {
                        Err(FetchError::new(
                            &urls[i],
                            format!("the {} s for fetching the pieces of the page ran out", FETCH_BUDGET.as_secs()),
                        ))
                    } else {
                        fetcher.fetch(&urls[i])
                    };
                    *results[i].lock().unwrap_or_else(|e| e.into_inner()) = Some(result);
                }
            });
        }
    });
    results
        .into_iter()
        .zip(urls)
        .map(|(slot, url)| {
            slot.into_inner()
                .unwrap_or_else(|e| e.into_inner())
                .unwrap_or_else(|| Err(FetchError::new(url, "it was not fetched")))
        })
        .collect()
}

/// What a fetched piece is: by its media type, else by the ending of its address, else
/// by where it is named (a style sheet, a script) when the type says nothing.
fn kind_of(fetched: &Fetched, expect: Expect) -> Kind {
    let media = fetched.media_type.clone().unwrap_or_default();
    let by_type = Kind::of_type(&media);
    if by_type != Kind::Other {
        return by_type;
    }
    let path = Url::parse(&fetched.final_url).map(|u| u.path().to_ascii_lowercase()).unwrap_or_default();
    if path.ends_with(".css") || fetched.final_url.contains("fonts.googleapis.com/css") {
        return Kind::Css;
    }
    if path.ends_with(".js") || path.ends_with(".mjs") || path.ends_with("/+esm") {
        return Kind::Js;
    }
    let vague = matches!(essence(&media).as_str(), "" | "text/plain" | OCTET_STREAM | "binary/octet-stream");
    match expect {
        Expect::Css if vague => Kind::Css,
        Expect::Js if vague => Kind::Js,
        _ => Kind::Other,
    }
}

/// The media type a piece is served with: what the server said, else by its ending;
/// a style sheet and a script always with a type the browser takes for one.
fn piece_type(fetched: &Fetched, kind: Kind) -> String {
    let given = fetched.media_type.clone().filter(|t| !t.trim().is_empty()).unwrap_or_else(|| {
        let path = Url::parse(&fetched.final_url).map(|u| u.path().to_string()).unwrap_or_default();
        media_type(&path).to_string()
    });
    match kind {
        Kind::Css if Kind::of_type(&given) != Kind::Css => "text/css".to_string(),
        Kind::Js if Kind::of_type(&given) != Kind::Js => "text/javascript".to_string(),
        _ => given.trim().to_string(),
    }
}

struct Rewriter {
    stored: HashMap<String, String>,
}

fn has_scheme(text: &str) -> bool {
    match text.find(':') {
        Some(colon) => {
            let scheme = &text[..colon];
            !scheme.is_empty()
                && scheme.chars().next().map(|c| c.is_ascii_alphabetic()).unwrap_or(false)
                && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        }
        None => false,
    }
}

impl Rewriter {
    /// The reference written in place of `reference` in the file at `file`, and whether
    /// it now names a stored copy; None leaves it as it is.
    fn new_reference(&self, reference: &Ref, file: &str, base_removed: bool) -> Option<(String, bool)> {
        let raw = reference.raw.trim();
        let protocol_relative = raw.starts_with("//") || raw.starts_with("\\\\");
        let relative_again = base_removed && !has_scheme(raw);
        let specifier = matches!(reference.ctx, Ctx::Specifier | Ctx::MapValue | Ctx::MapScope);
        let finish = |text: String| if specifier { as_specifier(text) } else { text };
        match resolve(raw, &reference.base) {
            Resolved::Nothing | Resolved::Outside => None,
            Resolved::Local { path, fragment } => {
                if !relative_again {
                    return None;
                }
                let mut text = relative_reference(file, &path, fragment.as_deref());
                if text.is_empty() || text.starts_with('#') {
                    text = format!("./{text}");
                }
                Some((finish(text), false))
            }
            Resolved::External { url, fragment } => {
                let prefix = reference.ctx == Ctx::MapScope || (reference.ctx == Ctx::MapValue && raw.ends_with('/'));
                if prefix {
                    let dir = external_dir(&url)?;
                    return Some((as_specifier(relative_reference(file, &dir, None)), false));
                }
                if reference.ctx != Ctx::Link {
                    if let Some(path) = self.stored.get(&url) {
                        let text = if reference.root_relative {
                            root_reference(path, fragment.as_deref())
                        } else {
                            relative_reference(file, path, fragment.as_deref())
                        };
                        return Some((finish(text), true));
                    }
                }
                if relative_again || protocol_relative {
                    return Some((absolute_reference(&url, fragment.as_deref()), false));
                }
                None
            }
        }
    }
}

/// The text with its references rewritten; None when nothing changes. `piece`: a
/// fetched file, which also loses its source map comments.
fn rewrite(
    bytes: &[u8],
    scan: &Scan,
    file: &str,
    rewriter: &Rewriter,
    piece: bool,
    inherited_base_removed: bool,
) -> Option<Vec<u8>> {
    let base_removed = inherited_base_removed || scan.base_removed;
    let mut edits: Vec<(Range<usize>, Vec<u8>)> = Vec::new();
    for reference in &scan.refs {
        let Some((text, stored)) = rewriter.new_reference(reference, file, base_removed) else {
            continue;
        };
        let written = match &reference.write {
            Write::Raw => text,
            Write::Attr { before, after } => quoted_attr(&format!("{before}{text}{after}")),
        };
        edits.push((reference.span.clone(), written.into_bytes()));
        if stored {
            for span in &reference.with {
                edits.push((span.clone(), Vec::new()));
            }
        }
    }
    for nested in &scan.nested {
        if let Some(new) = rewrite(nested.text.as_bytes(), &nested.scan, file, rewriter, false, base_removed) {
            edits.push((nested.span.clone(), quoted_attr(&String::from_utf8_lossy(&new)).into_bytes()));
        }
    }
    for span in &scan.removals {
        edits.push((span.clone(), Vec::new()));
    }
    if piece {
        for span in &scan.source_maps {
            edits.push((span.clone(), Vec::new()));
        }
    }
    if edits.is_empty() {
        return None;
    }
    edits.sort_by_key(|(span, _)| (span.start, span.end));
    let mut out = Vec::with_capacity(bytes.len() + 64);
    let mut at = 0;
    for (span, text) in edits {
        if span.start < at || span.end > bytes.len() {
            continue; // overlaps an edit before it
        }
        out.extend_from_slice(&bytes[at..span.start]);
        out.extend_from_slice(&text);
        at = span.end;
    }
    out.extend_from_slice(&bytes[at..]);
    Some(out)
}

/// Whether a file uses claude.ai's storage or tools (`window.claude`, `claude.use(`). The
/// hook claude.ai calls when a page is republished (`window.claude.hot`, as in `const hot
/// = window.claude && window.claude.hot`) keeps nothing, and is not counted.
fn uses_claude(bytes: &[u8]) -> bool {
    if find(bytes, b"claude").is_none() {
        return false;
    }
    let mut text = String::from_utf8_lossy(bytes).into_owned();
    for hook in ["window.claude && window.claude.hot", "window.claude?.hot", "window.claude.hot"] {
        text = text.replace(hook, "");
    }
    text.contains("window.claude") || text.contains("claude.use(")
}

/// The most addresses not kept that are named one by one.
const NOT_KEPT_NAMED: usize = 10;

/// "it names https://api.example.com/grades, which is not kept: …" for every absolute
/// address in the page's own scripts, styles and `data-*` attributes (and a form's
/// action, a refresh) that was not fetched.
fn not_kept(locals: &[LocalFile], pieces: &BTreeMap<String, Piece>) -> Vec<String> {
    let mut named: Vec<String> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut note = |shown: String, key: String| {
        if !pieces.contains_key(&key) && seen.insert(key) {
            named.push(shown);
        }
    };
    for scan in locals.iter().filter_map(|l| l.scan.as_ref()) {
        for literal in scan.all_literals() {
            if !matches!(literal.place, Place::Script | Place::Css | Place::DataAttr) {
                continue;
            }
            let value = literal.value.trim();
            if !is_named_address(value) {
                continue;
            }
            let key = normal_address(value).unwrap_or_else(|| value.to_string());
            let shown = if literal.substituted { format!("{value}…") } else { value.to_string() };
            note(clip(&shown), key);
        }
        for reference in scan.all_refs() {
            if !reference.warn_external {
                continue;
            }
            if let Resolved::External { url, .. } = resolve(&reference.raw, &reference.base) {
                note(clip(&url), url.clone());
            }
        }
    }
    let mut out: Vec<String> = named
        .iter()
        .take(NOT_KEPT_NAMED)
        .map(|address| {
            format!("it names {address}, which is not kept: if the page asks for it later (a button), that part will not work")
        })
        .collect();
    if named.len() > NOT_KEPT_NAMED {
        let more = named.len() - NOT_KEPT_NAMED;
        out.push(format!(
            "it names {} more {} on the web that {} not kept",
            count(more),
            if more == 1 { "address" } else { "addresses" },
            if more == 1 { "is" } else { "are" }
        ));
    }
    out
}

fn clip(text: &str) -> String {
    if text.chars().count() <= 200 {
        return text.to_string();
    }
    let cut: String = text.chars().take(199).collect();
    format!("{cut}…")
}

/// An absolute http(s) address with a host that has a dot, and no namespace name of the
/// W3C (`http://www.w3.org/2000/svg`), which nobody fetches.
fn is_named_address(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let rest = match lower.strip_prefix("https://").or_else(|| lower.strip_prefix("http://")) {
        Some(rest) => rest,
        None => return false,
    };
    if text.chars().any(char::is_whitespace) {
        return false;
    }
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    if !host.contains('.') || host.starts_with('.') || host.ends_with('.') {
        return false;
    }
    !(host == "www.w3.org" || host == "w3.org")
}
