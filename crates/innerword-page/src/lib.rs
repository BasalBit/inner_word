//! Pages: an HTML page (with its CSS, JavaScript, fonts, pictures, sounds) made into the
//! files the library keeps (docs/pages-design.md, sections 2 and 3, and section 12 WP2).
//!
//! The way through, for one page:
//!
//! 1. `read_input`: a file (and the local files it names, never a walk of its folder) or
//!    a folder taken whole, with its fingerprint.
//! 2. `bundle`: every reference found (HTML and SVG, CSS, JavaScript, import maps), every
//!    piece it names on the web fetched once through a `Fetcher` and stored under
//!    `_external/<host>/…`, every reference to one rewritten to the stored copy, by a
//!    relative path.
//! 3. The check, which is not here: `check_request` and `write_check_dir` say what it is
//!    asked, `CheckReport` is what it answers (WebKit, in `innerword-web`; `static_check`
//!    in tests).
//! 4. `prepare`: whole or nothing. A `Prepared` page holds its blobs (distinct bytes),
//!    its paths, its text and its title; `Prepared::manifest` is the content of its row.
//!
//! No GPUI, no WebKit, no store: the crate depends on nothing of ours. It fetches with
//! `/usr/bin/curl` (`CurlFetcher`), only from public addresses.

mod base64_bytes;
mod bundle;
mod check;
mod css;
mod error;
mod export;
mod fetch;
mod html;
mod importmap;
mod input;
mod js;
mod manifest;
mod media;
mod naming;
mod prepare;
mod resolve;
mod scan;
mod source_url;

pub mod limits;

pub use crate::bundle::{Bundle, BundleFile, External, bundle};
pub use crate::check::{
    Blocked, CheckReport, CheckRequest, LoadReport, STATIC_ENGINE, check_request, read_check_dir, static_check,
    write_check_dir, write_served_dir,
};
pub use crate::error::{PROBLEMS_NAMED, PageError, Problem, megabytes, reasons_sentence};
pub use crate::export::{Which, Written, write_folder};
pub use crate::fetch::{
    CachingFetcher, CurlFetcher, FetchError, Fetched, Fetcher, KnownPieces, LOCAL_ADDRESS, MapFetcher, USER_AGENT,
};
pub use crate::input::{Input, InputFile, read_input};
pub use crate::limits::*;
pub use crate::manifest::{Checked, Manifest, ManifestFile};
pub use crate::media::{file_id_hex, is_file_id, media_type, sniffed_type};
pub use crate::prepare::{Blob, CheckSummary, PathEntry, Prepared, prepare};
pub use crate::source_url::normal_source_url;

/// The folder of the served files that holds the pieces fetched from the web.
pub const EXTERNAL_DIR: &str = "_external";
/// The version of the manifest this crate writes (its `"page"`).
pub const MANIFEST_VERSION: u32 = 1;
/// The hosts whose absolute addresses in a string of a script are fetched.
pub const KNOWN_CDNS: &[&str] = crate::js::KNOWN_CDNS;
