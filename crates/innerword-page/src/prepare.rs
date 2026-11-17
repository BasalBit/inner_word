//! The verdict: a page that can be stored, or why not (docs/pages-design.md, section 3,
//! "Whole or nothing: the check" and "Extracting the text").

use std::collections::{HashMap, HashSet};
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::MANIFEST_VERSION;
use crate::bundle::{Bundle, External, too_many_html};
use crate::check::{CheckReport, LoadReport, clip_title};
use crate::error::{PageError, Problem, count};
use crate::input::too_heavy;
use crate::limits::{ALSO_LOADS, CHECK_LIMIT_MS, MAX_PAGE_BYTES, MAX_TEXT_CHARS};
use crate::manifest::{Checked, Manifest, ManifestFile};
use crate::media::file_id_hex;

/// A page that was checked whole, with everything the library stores of it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Prepared {
    pub name: String,
    pub fingerprint: String,
    /// "" when the page gives none.
    pub title_found: String,
    pub text: String,
    pub entry: String,
    /// Distinct bytes.
    pub blobs: Vec<Blob>,
    pub served: Vec<PathEntry>,
    pub original: Vec<PathEntry>,
    pub externals: Vec<External>,
    pub checked: CheckSummary,
    pub warnings: Vec<String>,
}

/// Bytes of the page, once each.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Blob {
    /// file_id_hex of the bytes.
    pub file: String,
    pub size: u64,
    /// Empty once stored (and in a dry run's request).
    #[serde(with = "crate::base64_bytes")]
    pub data: Vec<u8>,
    /// Set only by the app's listener, after parsing.
    #[serde(skip)]
    pub stored: bool,
}

impl fmt::Debug for Blob {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Blob {{ file: {:?}, size: {}, data: {} bytes, stored: {} }}",
            self.file,
            self.size,
            self.data.len(),
            self.stored
        )
    }
}

/// A path of the page and the blob it is served from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathEntry {
    pub path: String,
    pub media_type: String,
    /// Index into `Prepared::blobs`.
    pub blob: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckSummary {
    pub engine: String,
    pub requests: usize,
    pub elapsed_ms: u64,
}

/// Script errors of one load said by name; the rest are counted.
const SCRIPT_ERRORS_NAMED: usize = 3;

/// How the check names what a blocked request was for, by its directive.
fn directive_words(directive: &str) -> String {
    let words = match directive.trim().to_ascii_lowercase().as_str() {
        "font-src" => "a font",
        "img-src" => "a picture",
        "script-src" | "script-src-elem" | "script-src-attr" => "a script",
        "style-src" | "style-src-elem" | "style-src-attr" => "a style sheet",
        "connect-src" => "data it fetches",
        "frame-src" | "child-src" | "frame-ancestors" => "a frame",
        "media-src" => "media",
        "object-src" => "an embedded object",
        "worker-src" => "a worker",
        "manifest-src" => "a manifest",
        "prefetch-src" => "a prefetch",
        "form-action" => "a form it sends",
        "default-src" => "something it loads",
        other => return other.to_string(),
    };
    words.to_string()
}

pub fn prepare(bundle: Bundle, report: &CheckReport) -> Result<Prepared, PageError> {
    if report.loads.is_empty() {
        return Err(PageError::CheckFailed {
            reason: report
                .failure
                .clone()
                .unwrap_or_else(|| "the check loaded nothing".to_string()),
        });
    }
    let entry = bundle.entry.clone();
    let html = bundle.html_paths();
    let entry_load = report.loads.iter().find(|load| load.path == entry);

    let title_found = entry_load
        .map(|load| clip_title(&load.title))
        .filter(|title| !title.is_empty())
        .or_else(|| {
            entry_load
                .map(|load| clip_title(&load.first_heading))
                .filter(|h| !h.is_empty())
        })
        .unwrap_or_default();
    let shown = if title_found.is_empty() {
        bundle.name.clone()
    } else {
        title_found.clone()
    };

    let mut problems: Vec<Problem> = Vec::new();
    let mut warnings: Vec<String> = bundle.warnings.clone();
    if html.len() > ALSO_LOADS + 1 {
        problems.push(too_many_html(html.len()));
    }
    if let Some(failure) = &report.failure {
        problems.push(Problem::new(
            "the page",
            format!("the page could not be checked whole: {failure}"),
        ));
    }
    match entry_load {
        None => problems.push(Problem::new(
            entry.clone(),
            format!("the page could not be checked: {entry} was not loaded"),
        )),
        Some(load) => {
            if load.crashed {
                problems.push(Problem::new(
                    entry.clone(),
                    "the page stopped working while it was checked (its web process crashed)",
                ));
            } else if let Some(error) = &load.navigation_error {
                problems.push(Problem::new(
                    entry.clone(),
                    format!("the page failed to load: {error}"),
                ));
            } else if !load.finished {
                problems.push(Problem::new(
                    entry.clone(),
                    "the page did not finish loading",
                ));
            }
        }
    }
    let loaded: HashSet<&str> = report.loads.iter().map(|load| load.path.as_str()).collect();
    let not_loaded = html
        .iter()
        .filter(|path| !loaded.contains(path.as_str()))
        .count();
    if entry_load.is_some() && not_loaded > 0 {
        problems.push(Problem::new(
            "the page",
            format!(
                "the page could not be checked in time: {} of its HTML files {} not loaded within {} s",
                count(not_loaded),
                if not_loaded == 1 { "was" } else { "were" },
                CHECK_LIMIT_MS / 1000
            ),
        ));
    }
    for load in ordered_loads(report, &html) {
        let subject = if load.path == entry {
            "the page".to_string()
        } else {
            load.path.clone()
        };
        for path in &load.missing {
            problems.push(Problem::new(
                path.clone(),
                format!("{subject} asks for {path}, which is not among its files"),
            ));
        }
        for blocked in &load.blocked {
            let reason = if blocked.directive.trim().eq_ignore_ascii_case("navigation") {
                format!(
                    "{subject} goes to {} at run time, which cannot be kept",
                    blocked.url
                )
            } else {
                format!(
                    "{subject} asks for {} at run time ({}), which cannot be kept",
                    blocked.url,
                    directive_words(&blocked.directive)
                )
            };
            problems.push(Problem::new(blocked.url.clone(), reason));
        }
        for frame in &load.unwatched_frames {
            problems.push(Problem::new(
                frame.clone(),
                format!("{subject} shows {frame} in a frame, whose requests cannot be watched"),
            ));
        }
        if load.path != entry {
            if load.crashed {
                warnings.push(format!(
                    "{} stopped working while it was checked",
                    load.path
                ));
            } else if let Some(error) = &load.navigation_error {
                warnings.push(format!(
                    "{} failed to load while it was checked: {error}",
                    load.path
                ));
            }
        }
        for error in load.script_errors.iter().take(SCRIPT_ERRORS_NAMED) {
            warnings.push(format!("a script of {} failed: {error}", load.path));
        }
        if load.script_errors.len() > SCRIPT_ERRORS_NAMED {
            let more = load.script_errors.len() - SCRIPT_ERRORS_NAMED;
            warnings.push(format!(
                "and {} more script {} in {}",
                count(more),
                if more == 1 { "error" } else { "errors" },
                load.path
            ));
        }
    }
    let mut seen_reasons = HashSet::new();
    problems.retain(|problem| seen_reasons.insert(problem.reason.clone()));
    if !problems.is_empty() {
        return Err(PageError::Incomplete {
            name: shown,
            problems,
        });
    }

    // The text: the entry's, then each other HTML file's after a blank line, under its title.
    let mut text = entry_load.map(|load| load.text.clone()).unwrap_or_default();
    for load in ordered_loads(report, &html) {
        if load.path == entry {
            continue;
        }
        let heading = if load.title.trim().is_empty() {
            load.path.clone()
        } else {
            load.title.trim().to_string()
        };
        text.push_str("\n\n");
        text.push_str(&heading);
        text.push('\n');
        text.push_str(&load.text);
    }
    if text.chars().count() > MAX_TEXT_CHARS {
        text = text.chars().take(MAX_TEXT_CHARS).collect();
        warnings.push(format!(
            "its text is longer than {} characters; the rest is not kept for search and reading",
            count(MAX_TEXT_CHARS)
        ));
    }

    let mut blobs: Vec<Blob> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut entries =
        |files: &[crate::bundle::BundleFile], blobs: &mut Vec<Blob>| -> Vec<PathEntry> {
            files
                .iter()
                .map(|file| {
                    let id = file_id_hex(&file.bytes);
                    let blob = *index.entry(id.clone()).or_insert_with(|| {
                        blobs.push(Blob {
                            file: id,
                            size: file.bytes.len() as u64,
                            data: file.bytes.to_vec(),
                            stored: false,
                        });
                        blobs.len() - 1
                    });
                    PathEntry {
                        path: file.path.clone(),
                        media_type: file.media_type.clone(),
                        blob,
                    }
                })
                .collect()
        };
    let served = entries(&bundle.served, &mut blobs);
    let original = entries(&bundle.original, &mut blobs);

    let prepared = Prepared {
        name: bundle.name,
        fingerprint: bundle.fingerprint,
        title_found,
        text,
        entry,
        blobs,
        served,
        original,
        externals: bundle.externals,
        checked: CheckSummary {
            engine: report.engine.clone(),
            requests: report.loads.iter().map(|load| load.requests).sum(),
            elapsed_ms: report.elapsed_ms,
        },
        warnings,
    };
    let bytes = prepared.distinct_bytes();
    if bytes > MAX_PAGE_BYTES {
        return Err(PageError::Incomplete {
            name: shown,
            problems: vec![too_heavy(bytes)],
        });
    }
    Ok(prepared)
}

/// The loads of the report in the order of the page's HTML files (the entry first),
/// then any other the report has.
fn ordered_loads<'a>(report: &'a CheckReport, html: &[String]) -> Vec<&'a LoadReport> {
    let mut out: Vec<&LoadReport> = Vec::new();
    for path in html {
        if let Some(load) = report.loads.iter().find(|load| &load.path == path) {
            out.push(load);
        }
    }
    for load in &report.loads {
        if !html.contains(&load.path) {
            out.push(load);
        }
    }
    out
}

impl Prepared {
    pub fn distinct_bytes(&self) -> u64 {
        self.blobs.iter().map(|blob| blob.size).sum()
    }

    /// The title rule's middle steps: title_found, else name.
    pub fn title(&self) -> String {
        if self.title_found.trim().is_empty() {
            self.name.clone()
        } else {
            self.title_found.clone()
        }
    }

    pub fn manifest(&self, source_url: Option<&str>, now_ms: i64) -> Manifest {
        let file = |entry: &PathEntry| {
            let blob = &self.blobs[entry.blob];
            ManifestFile {
                path: entry.path.clone(),
                file: blob.file.clone(),
                media_type: entry.media_type.clone(),
                size: blob.size,
            }
        };
        let mut files: Vec<ManifestFile> = self.served.iter().map(file).collect();
        let mut original: Vec<ManifestFile> = self.original.iter().map(file).collect();
        files.sort_by(|a, b| a.path.cmp(&b.path));
        original.sort_by(|a, b| a.path.cmp(&b.path));
        Manifest {
            page: MANIFEST_VERSION,
            entry: self.entry.clone(),
            files,
            original,
            externals: self.externals.clone(),
            source_url: source_url.map(str::to_string),
            fingerprint: self.fingerprint.clone(),
            added_at: now_ms,
            checked: Checked {
                engine: self.checked.engine.clone(),
                at: now_ms,
                requests: self.checked.requests,
                warnings: self.warnings.clone(),
            },
            text: self.text.clone(),
        }
    }
}
