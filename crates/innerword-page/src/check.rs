//! The check as data: what `innerword __page-check` is asked (a directory with
//! `request.json` and the served files) and what it answers, and the fake check of the
//! tests (docs/pages-design.md, sections 3 and 4).

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::bundle::Bundle;
use crate::error::PageError;
use crate::html::{Token, decode, tokenize};
use crate::limits::{ALSO_LOADS, CHECK_LIMIT_MS, MAX_TITLE_CHARS, SETTLE_MAX_MS, SETTLE_MIN_MS};
use crate::manifest::Manifest;
use crate::media::{is_html_type, looks_like_text, media_type, sniffed_type};

/// What the check is asked: the entry, the other HTML files, the media type of every
/// served path, and how long to wait.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CheckRequest {
    pub entry: String,
    /// Other HTML files, at most ALSO_LOADS.
    pub also: Vec<String>,
    /// path -> media type
    pub media_types: BTreeMap<String, String>,
    pub settle_min_ms: u64,
    pub settle_max_ms: u64,
    pub limit_ms: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CheckReport {
    pub engine: String,
    pub elapsed_ms: u64,
    pub loads: Vec<LoadReport>,
    /// The checker could not run (no window server, WebKit gone): the page is not
    /// judged ("the page could not be checked: …").
    pub failure: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LoadReport {
    pub path: String,
    pub title: String,
    pub first_heading: String,
    pub text: String,
    pub finished: bool,
    pub navigation_error: Option<String>,
    pub crashed: bool,
    /// Paths answered 404.
    pub missing: Vec<String>,
    pub blocked: Vec<Blocked>,
    /// At most 20.
    pub script_errors: Vec<String>,
    /// Answered by the scheme handler.
    pub requests: usize,
    /// Frames of SVG/XHTML/XML the probe did not report from.
    pub unwatched_frames: Vec<String>,
}

/// A request the policy blocked: its address and its directive ("font-src",
/// "connect-src", ..., "navigation" for a navigation that was cancelled).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Blocked {
    pub url: String,
    pub directive: String,
}

/// The engine `static_check` names in its report.
pub const STATIC_ENGINE: &str = "static (no WebKit)";

pub fn check_request(bundle: &Bundle) -> CheckRequest {
    let html = bundle.html_paths();
    CheckRequest {
        entry: bundle.entry.clone(),
        also: html.into_iter().skip(1).take(ALSO_LOADS).collect(),
        media_types: bundle.served.iter().map(|f| (f.path.clone(), f.media_type.clone())).collect(),
        settle_min_ms: SETTLE_MIN_MS,
        settle_max_ms: SETTLE_MAX_MS,
        limit_ms: CHECK_LIMIT_MS,
    }
}

/// A path of the page as a path under `dir`, refusing one that would leave it.
fn under(dir: &Path, path: &str) -> io::Result<PathBuf> {
    if path.is_empty()
        || path.starts_with('/')
        || path.split('/').any(|segment| segment.is_empty() || segment == "." || segment == "..")
        || path.contains('\0')
    {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("{path} is not a path of a page")));
    }
    Ok(path.split('/').fold(dir.to_path_buf(), |acc, segment| acc.join(segment)))
}

fn make_private_dir(dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn write_request(dir: &Path, request: &CheckRequest) -> io::Result<()> {
    let text = serde_json::to_string(request).map_err(io::Error::other)?;
    fs::write(dir.join("request.json"), text)
}

/// <dir>/request.json and <dir>/files/<path>. The directory is made (mode 0700) when it
/// is not there.
pub fn write_check_dir(bundle: &Bundle, dir: &Path) -> io::Result<()> {
    make_private_dir(dir)?;
    let files = dir.join("files");
    fs::create_dir_all(&files)?;
    for file in &bundle.served {
        let target = under(&files, &file.path)?;
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&target, &file.bytes)?;
    }
    write_request(dir, &check_request(bundle))
}

/// The same layout from a stored page (its served files), for `__page-pdf`. A file whose
/// bytes the library does not have is left out (the page then misses it, as the view
/// does).
pub fn write_served_dir(
    manifest: &Manifest,
    dir: &Path,
    bytes_of: &mut dyn FnMut(&str) -> io::Result<Option<Vec<u8>>>,
) -> Result<(), PageError> {
    make_private_dir(dir).map_err(|error| PageError::io(dir, error))?;
    let files = dir.join("files");
    fs::create_dir_all(&files).map_err(|error| PageError::io(&files, error))?;
    // The other documents of the page: HTML by its type and its bytes, and no piece from
    // the web (a page stored by 0.8.0 may have a font typed text/html among its pieces).
    let external: HashSet<&str> = manifest.externals.iter().map(|e| e.path.as_str()).collect();
    let mut also: Vec<String> = Vec::new();
    for file in &manifest.files {
        let target = under(&files, &file.path).map_err(|error| PageError::Unreadable { reason: error.to_string() })?;
        let bytes = bytes_of(&file.file).map_err(|error| PageError::io(&target, error))?;
        let document = file.path != manifest.entry
            && !external.contains(file.path.as_str())
            && is_html_type(&file.media_type)
            && bytes.as_deref().map(|b| sniffed_type(b).is_none() && looks_like_text(b)).unwrap_or(true);
        if document && also.len() < ALSO_LOADS {
            also.push(file.path.clone());
        }
        let Some(bytes) = bytes else { continue };
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|error| PageError::io(parent, error))?;
        }
        fs::write(&target, bytes).map_err(|error| PageError::io(&target, error))?;
    }
    let request = CheckRequest {
        entry: manifest.entry.clone(),
        also,
        media_types: manifest.files.iter().map(|f| (f.path.clone(), f.media_type.clone())).collect(),
        settle_min_ms: SETTLE_MIN_MS,
        settle_max_ms: SETTLE_MAX_MS,
        limit_ms: CHECK_LIMIT_MS,
    };
    write_request(dir, &request).map_err(|error| PageError::io(dir.join("request.json"), error))
}

/// What `write_check_dir` (or `write_served_dir`) wrote: the request, and every file
/// under `files/` as path -> (media type, bytes).
pub fn read_check_dir(dir: &Path) -> io::Result<(CheckRequest, BTreeMap<String, (String, Vec<u8>)>)> {
    let text = fs::read_to_string(dir.join("request.json"))?;
    let request: CheckRequest =
        serde_json::from_str(&text).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let mut files = BTreeMap::new();
    let root = dir.join("files");
    let mut stack: Vec<(PathBuf, String)> = vec![(root, String::new())];
    while let Some((folder, prefix)) = stack.pop() {
        let entries = match fs::read_dir(&folder) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound && prefix.is_empty() => continue,
            Err(error) => return Err(error),
        };
        for entry in entries {
            let entry = entry?;
            let Some(name) = entry.file_name().to_str().map(str::to_string) else { continue };
            let path = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                stack.push((entry.path(), path));
            } else if file_type.is_file() {
                let bytes = fs::read(entry.path())?;
                let media =
                    request.media_types.get(&path).cloned().unwrap_or_else(|| media_type(&path).to_string());
                files.insert(path, (media, bytes));
            }
        }
    }
    Ok((request, files))
}

/// The fake check (tests, and INNER_WORD_PAGE_CHECK=fake in a debug build only): the
/// entry's text with its tags and the contents of script, style, template and noscript
/// taken out, its <title>, nothing blocked; every other HTML file of the page read the
/// same way, as the real check loads them. Never a fallback for a run without WebKit.
pub fn static_check(bundle: &Bundle) -> CheckReport {
    let mut loads = Vec::new();
    for path in bundle.html_paths() {
        match bundle.served_file(&path) {
            Some(file) => {
                let (title, first_heading, text) = static_text(&file.bytes);
                loads.push(LoadReport { path, title, first_heading, text, finished: true, requests: 1, ..LoadReport::default() });
            }
            None => loads.push(LoadReport {
                path,
                navigation_error: Some("it is not among the files of the page".to_string()),
                ..LoadReport::default()
            }),
        }
    }
    CheckReport { engine: STATIC_ENGINE.to_string(), elapsed_ms: 0, loads, failure: None }
}

/// The `<title>` of an HTML file, trimmed, at most MAX_TITLE_CHARS.
pub(crate) fn static_title(bytes: &[u8]) -> Option<String> {
    let (title, _, _) = static_text(bytes);
    (!title.is_empty()).then_some(title)
}

const BLOCKS: &[&str] = &[
    "address", "article", "aside", "blockquote", "br", "dd", "details", "dialog", "div", "dl", "dt",
    "fieldset", "figcaption", "figure", "footer", "form", "h1", "h2", "h3", "h4", "h5", "h6", "header",
    "hr", "li", "main", "nav", "ol", "p", "pre", "section", "summary", "table", "tbody", "td", "tfoot",
    "th", "thead", "tr", "ul", "option", "legend", "caption", "body", "html", "button", "label",
];

/// (title, first h1, text) of an HTML file as the fake check reads it.
fn static_text(bytes: &[u8]) -> (String, String, String) {
    let tokens = tokenize(bytes, false);
    let mut title = String::new();
    let mut heading = String::new();
    let mut in_heading = false;
    let mut heading_done = false;
    let mut skip: Vec<String> = Vec::new();
    let mut out = String::new();
    for token in &tokens {
        match token {
            Token::Start(tag) => {
                if BLOCKS.contains(&tag.name.as_str()) {
                    out.push('\n');
                }
                if tag.name == "h1" && !heading_done {
                    in_heading = true;
                }
                // title, script, style and noscript are raw text (their contents are one
                // token, left out below); a template holds tags, so it is skipped to its end.
                if tag.name == "template" && !tag.self_closing {
                    skip.push(tag.name.clone());
                }
            }
            Token::End { name } => {
                if BLOCKS.contains(&name.as_str()) {
                    out.push('\n');
                }
                if name == "h1" && in_heading {
                    in_heading = false;
                    heading_done = true;
                }
                if skip.last() == Some(name) {
                    skip.pop();
                }
            }
            Token::Raw { element, span } => {
                if element == "title" && title.is_empty() {
                    title = collapse(&decode(&bytes[span.clone()]));
                } else if element == "textarea" && skip.is_empty() {
                    out.push_str(&decode(&bytes[span.clone()]));
                }
            }
            Token::Text(span) => {
                if !skip.is_empty() {
                    continue;
                }
                let text = decode(&bytes[span.clone()]);
                if in_heading {
                    heading.push_str(&text);
                }
                out.push_str(&text);
            }
            _ => {}
        }
    }
    let lines: Vec<String> = out.lines().map(collapse).filter(|line| !line.is_empty()).collect();
    (clip_title(&title), clip_title(&collapse(&heading)), lines.join("\n"))
}

fn collapse(text: &str) -> String {
    text.split_ascii_whitespace().collect::<Vec<_>>().join(" ")
}

pub(crate) fn clip_title(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= MAX_TITLE_CHARS {
        return trimmed.to_string();
    }
    trimmed.chars().take(MAX_TITLE_CHARS).collect::<String>().trim_end().to_string()
}
