//! Pages written as text in the tests, in temporary folders, and the answers of the web
//! from memory (`MapFetcher`): no test of this file reaches the network.

use std::fs;
use std::path::Path;

use tempfile::TempDir;
use innerword_page::{Bundle, MapFetcher, PageError, bundle, read_input};

pub fn temp() -> TempDir {
    tempfile::Builder::new().prefix("innerword-page-test-").tempdir().expect("a temporary folder")
}

pub fn write(dir: &Path, path: &str, contents: impl AsRef<[u8]>) {
    let full = dir.join(path);
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(full, contents).unwrap();
}

/// A folder with these files (path, contents).
pub fn folder(files: &[(&str, &str)]) -> TempDir {
    let dir = temp();
    for (path, contents) in files {
        write(dir.path(), path, contents);
    }
    dir
}

/// The answers of the web: (address, media type, body).
pub fn web(answers: &[(&str, &str, &str)]) -> MapFetcher {
    let mut fetcher = MapFetcher::new();
    for (url, media, body) in answers {
        fetcher = fetcher.with(url, media, body.as_bytes().to_vec());
    }
    fetcher
}

/// The page a folder of these files makes with these answers of the web.
pub fn page(files: &[(&str, &str)], answers: &[(&str, &str, &str)]) -> Bundle {
    try_page(files, answers).unwrap_or_else(|error| panic!("the page was refused: {}", error.sentence()))
}

pub fn try_page(files: &[(&str, &str)], answers: &[(&str, &str, &str)]) -> Result<Bundle, PageError> {
    let dir = folder(files);
    let input = read_input(dir.path(), None)?;
    bundle(input, &web(answers))
}

/// The text of a served file.
pub fn served(bundle: &Bundle, path: &str) -> String {
    let file = bundle
        .served
        .iter()
        .find(|f| f.path == path)
        .unwrap_or_else(|| panic!("{path} is not served; served: {:?}", paths(bundle)));
    String::from_utf8_lossy(&file.bytes).into_owned()
}

pub fn paths(bundle: &Bundle) -> Vec<String> {
    bundle.served.iter().map(|f| f.path.clone()).collect()
}

pub fn external_urls(bundle: &Bundle) -> Vec<String> {
    bundle.externals.iter().map(|e| e.url.clone()).collect()
}

/// The stored path of an address.
pub fn stored(bundle: &Bundle, url: &str) -> String {
    bundle
        .externals
        .iter()
        .find(|e| e.url == url)
        .unwrap_or_else(|| panic!("{url} was not fetched; fetched: {:?}", external_urls(bundle)))
        .path
        .clone()
}

pub fn problems(error: &PageError) -> Vec<String> {
    match error {
        PageError::Incomplete { problems, .. } => problems.iter().map(|p| p.reason.clone()).collect(),
        other => panic!("not a refusal of a page that is not whole: {other:?}"),
    }
}
