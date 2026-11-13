//! The user's archive of claude.ai artifacts, read only: every original bundled with the
//! real fetcher, and every piece `archive.py` fetched for it is found and fetched again.
//! This one reaches the internet, so it never runs by default:
//! `CARGO_TARGET_DIR=target-pure cargo test -p writer-page -- --ignored archive`.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use writer_page::{CachingFetcher, CurlFetcher, bundle, read_input, write_check_dir};

use super::support::temp;

fn archive() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    home.join("page archive")
}

fn normal(url: &str) -> String {
    url::Url::parse(url).map(|mut u| {
        u.set_fragment(None);
        u.to_string()
    })
    .unwrap_or_else(|_| url.to_string())
}

#[test]
#[ignore = "reaches the internet and reads the user's archive (read only)"]
fn archive_every_external_of_the_originals_is_found_and_fetched() {
    let root = archive();
    let fetcher = CachingFetcher::new(Arc::new(CurlFetcher::new()));
    let mut folders: Vec<PathBuf> = std::fs::read_dir(&root)
        .expect("the archive")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.join("manifest.json").is_file() && p.join("original").is_dir())
        .collect();
    folders.sort();
    assert!(!folders.is_empty());
    let mut report = Vec::new();
    for folder in folders {
        let name = folder.file_name().unwrap().to_string_lossy().into_owned();
        let manifest: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(folder.join("manifest.json")).unwrap()).unwrap();
        let wanted: BTreeSet<String> = manifest["externals"]
            .as_object()
            .map(|m| m.keys().map(|k| normal(k)).collect())
            .unwrap_or_default();
        let not_kept: BTreeSet<String> = manifest["not_kept"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().or_else(|| v["url"].as_str())).map(normal).collect())
            .unwrap_or_default();
        let input = read_input(&folder.join("original"), None).unwrap_or_else(|e| panic!("{name}: {}", e.sentence()));
        let started = std::time::Instant::now();
        let bundled = match bundle(input, &fetcher) {
            Ok(bundled) => bundled,
            Err(error) => {
                report.push(format!("{name}: refused: {}", error.sentence()));
                continue;
            }
        };
        let fetched: BTreeSet<String> = bundled.externals.iter().map(|e| normal(&e.url)).collect();
        let missed: Vec<&String> = wanted.iter().filter(|u| !fetched.contains(*u) && !not_kept.contains(*u)).collect();
        let dir = temp();
        write_check_dir(&bundled, &dir.path().join("check")).unwrap();
        report.push(format!(
            "{name}: {} files, {} pieces ({} in the archive), {} MB, {:.1} s, {} warnings{}",
            bundled.served.len(),
            bundled.externals.len(),
            wanted.len(),
            bundled.distinct_bytes() / (1024 * 1024),
            started.elapsed().as_secs_f64(),
            bundled.warnings.len(),
            if missed.is_empty() { String::new() } else { format!("; MISSED {missed:?}") }
        ));
    }
    let text = report.join("\n");
    println!("{text}");
    assert!(!text.contains("MISSED"), "{text}");
}

/// The pieces `archive.py` stored, answered from the archive itself (no network): an
/// address the archive names answers with its stored copy at
/// `https://archive.invalid/<path in the archive>`, so that what that copy names (archive.py
/// rewrote it to relative paths) is asked there again and answered from the archive too.
struct FromTheArchive {
    folder: PathBuf,
    map: std::collections::HashMap<String, String>,
    asked: std::sync::Mutex<BTreeSet<String>>,
}

const ARCHIVE_HOST: &str = "https://archive.invalid/";

impl writer_page::Fetcher for FromTheArchive {
    fn fetch(&self, url: &str) -> Result<writer_page::Fetched, writer_page::FetchError> {
        let key = normal(url);
        self.asked.lock().unwrap().insert(key.clone());
        let path = match self.map.get(&key) {
            Some(path) => path.clone(),
            None => match key.strip_prefix(ARCHIVE_HOST) {
                Some(path) => url::Url::parse(&key)
                    .map(|u| percent_decode(u.path().trim_start_matches('/')))
                    .unwrap_or_else(|_| path.to_string()),
                None => return Err(writer_page::FetchError::new(url, "not in the archive")),
            },
        };
        let bytes = std::fs::read(self.folder.join(&path))
            .map_err(|e| writer_page::FetchError::new(url, format!("{path}: {e}")))?;
        Ok(writer_page::Fetched {
            final_url: format!("{ARCHIVE_HOST}{path}"),
            media_type: Some(writer_page::media_type(&path).to_string()),
            bytes,
        })
    }
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&text[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[test]
#[ignore = "reads the user's archive (read only, no network)"]
fn archive_offline_every_external_of_the_originals_is_found() {
    let root = archive();
    let mut folders: Vec<PathBuf> = std::fs::read_dir(&root)
        .expect("the archive")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.join("manifest.json").is_file() && p.join("original").is_dir())
        .collect();
    folders.sort();
    let mut report = Vec::new();
    for folder in folders {
        let name = folder.file_name().unwrap().to_string_lossy().into_owned();
        let manifest: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(folder.join("manifest.json")).unwrap()).unwrap();
        let map: std::collections::HashMap<String, String> = manifest["externals"]
            .as_object()
            .map(|m| m.iter().filter_map(|(k, v)| v.as_str().map(|v| (normal(k), v.to_string()))).collect())
            .unwrap_or_default();
        let fetcher = FromTheArchive { folder: folder.clone(), map: map.clone(), asked: Default::default() };
        let input = read_input(&folder.join("original"), None).unwrap_or_else(|e| panic!("{name}: {}", e.sentence()));
        let outcome = bundle(input, &fetcher);
        let asked = fetcher.asked.lock().unwrap().clone();
        let missed: Vec<&String> = map
            .iter()
            .filter(|(url, path)| !asked.contains(*url) && !asked.contains(&normal(&format!("{ARCHIVE_HOST}{path}"))))
            .map(|(url, _)| url)
            .collect();
        match outcome {
            Ok(bundled) => report.push(format!(
                "{name}: {} files, {} pieces ({} in the archive), {} warnings{}{}",
                bundled.served.len(),
                bundled.externals.len(),
                map.len(),
                bundled.warnings.len(),
                if missed.is_empty() { String::new() } else { format!("; MISSED {missed:?}") },
                bundled.warnings.iter().map(|w| format!("\n    {w}")).collect::<String>()
            )),
            Err(error) => report.push(format!(
                "{name}: not bundled from the archive alone: {}{}",
                error.sentence(),
                if missed.is_empty() { String::new() } else { format!("; MISSED {missed:?}") }
            )),
        }
    }
    let text = report.join("\n");
    println!("{text}");
    assert!(!text.contains("MISSED"), "{text}");
}
