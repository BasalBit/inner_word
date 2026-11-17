//! The manifest: the content of a page row (docs/pages-design.md, section 2). Only this
//! crate writes it; everyone else reads it through `Manifest`. Every blob is named under
//! the key `"file"` with 32 hex digits: that is the token the store reads. Do not rename
//! it.

use std::collections::{BTreeSet, HashSet};

use serde::{Deserialize, Serialize};

use crate::bundle::External;
use crate::error::PageError;
use crate::media::is_file_id;
use crate::resolve::decode;
use crate::MANIFEST_VERSION;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub page: u32,
    pub entry: String,
    /// The served files, sorted by path.
    pub files: Vec<ManifestFile>,
    /// The input as it was given, sorted by path.
    pub original: Vec<ManifestFile>,
    pub externals: Vec<External>,
    pub source_url: Option<String>,
    /// The SHA-256 of the original input, in hex.
    pub fingerprint: String,
    /// When this content was taken (the add, or the update that brought it), unix ms.
    pub added_at: i64,
    pub checked: Checked,
    /// The rendered text, the same as the row's `plain_text`.
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestFile {
    pub path: String,
    /// The file id (32 lowercase hex digits) of the bytes.
    pub file: String,
    #[serde(rename = "type")]
    pub media_type: String,
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checked {
    pub engine: String,
    pub at: i64,
    pub requests: usize,
    pub warnings: Vec<String>,
}

impl Manifest {
    pub fn from_json(text: &str) -> Result<Manifest, PageError> {
        let manifest: Manifest = serde_json::from_str(text)
            .map_err(|error| PageError::Unreadable { reason: format!("it is no page manifest ({error})") })?;
        if manifest.page == 0 {
            return Err(PageError::Unreadable { reason: "it is no page manifest (no version)".to_string() });
        }
        if manifest.page > MANIFEST_VERSION {
            return Err(PageError::Unreadable {
                reason: format!(
                    "it was written by a newer version of Inner Word (page format {}, this version reads {})",
                    manifest.page, MANIFEST_VERSION
                ),
            });
        }
        for file in manifest.files.iter().chain(manifest.original.iter()) {
            if !is_file_id(&file.file) {
                return Err(PageError::Unreadable { reason: format!("{} names no stored file", file.path) });
            }
            if file.path.is_empty() || file.path.starts_with('/') || file.path.split('/').any(|s| s == "..") {
                return Err(PageError::Unreadable { reason: format!("{:?} is not a path of a page", file.path) });
            }
        }
        if !manifest.files.iter().any(|file| file.path == manifest.entry) {
            return Err(PageError::Unreadable { reason: format!("its entry {} is not among its files", manifest.entry) });
        }
        Ok(manifest)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("a manifest is JSON")
    }

    /// The served file at a path as a URL names it: exact, then percent-decoded (a
    /// leading `/`, a query and a fragment left out; a folder's `index.html` for a path
    /// that ends in `/`).
    pub fn served(&self, path: &str) -> Option<&ManifestFile> {
        let find = |wanted: &str| {
            self.files
                .binary_search_by(|file| file.path.as_str().cmp(wanted))
                .ok()
                .map(|at| &self.files[at])
                .or_else(|| self.files.iter().find(|file| file.path == wanted))
        };
        if let Some(found) = find(path) {
            return Some(found);
        }
        let bare = path.split(['?', '#']).next().unwrap_or("").trim_start_matches('/');
        let decoded = decode(bare);
        for candidate in [bare.to_string(), decoded.clone()] {
            if let Some(found) = find(&candidate) {
                return Some(found);
            }
            if candidate.is_empty() || candidate.ends_with('/') {
                if let Some(found) = find(&format!("{candidate}index.html")) {
                    return Some(found);
                }
            }
        }
        None
    }

    /// Every file id the manifest names, served and original.
    pub fn file_ids(&self) -> BTreeSet<String> {
        self.files.iter().chain(self.original.iter()).map(|file| file.file.clone()).collect()
    }

    /// The distinct bytes of the page, served and original together.
    pub fn bytes(&self) -> u64 {
        let mut seen = HashSet::new();
        self.files
            .iter()
            .chain(self.original.iter())
            .filter(|file| seen.insert(file.file.as_str()))
            .map(|file| file.size)
            .sum()
    }

    /// The served bytes alone (what the view loads).
    pub fn served_bytes(&self) -> u64 {
        let mut seen = HashSet::new();
        self.files.iter().filter(|file| seen.insert(file.file.as_str())).map(|file| file.size).sum()
    }
}
