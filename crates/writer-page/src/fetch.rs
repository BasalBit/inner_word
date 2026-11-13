//! Fetching the pieces a page names on the web: once each, only from public addresses,
//! with `/usr/bin/curl` (docs/pages-design.md, section 3, "Fetching each external once").

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fs;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use url::Url;

use crate::error::PageError;
use crate::input::too_large;
use crate::limits::{FETCH_TIMEOUT, MAX_FILE_BYTES, MAX_REDIRECTS};
use crate::media::{OCTET_STREAM, media_type};
use crate::resolve::normal_address;

pub trait Fetcher: Send + Sync {
    /// GET `url` (absolute http or https). Ok only for status 200.
    fn fetch(&self, url: &str) -> Result<Fetched, FetchError>;
}

#[derive(Clone, Debug)]
pub struct Fetched {
    /// The last address asked (after redirects).
    pub final_url: String,
    pub media_type: Option<String>,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct FetchError {
    pub url: String,
    /// "HTTP 404", "no network: Could not resolve host", ...
    pub reason: String,
}

/// The reason of an address on this Mac or its local network (`FetchError::local`).
pub const LOCAL_ADDRESS: &str = "is on this Mac or its local network; only addresses on the internet are fetched";

impl FetchError {
    pub fn new(url: impl Into<String>, reason: impl Into<String>) -> FetchError {
        FetchError { url: url.into(), reason: reason.into() }
    }

    /// The refusal of an address on this Mac or its local network.
    pub fn local(url: impl Into<String>) -> FetchError {
        FetchError::new(url, LOCAL_ADDRESS)
    }

    pub fn is_local(&self) -> bool {
        self.reason == LOCAL_ADDRESS
    }

    /// The problem as the sentence of a refusal says it.
    pub fn sentence(&self) -> String {
        if self.is_local() {
            format!("{} {}", self.url, self.reason)
        } else {
            format!("{} could not be fetched: {}", self.url, self.reason)
        }
    }
}

/// The user agent of Safari 17.5 on macOS (the one `archive.py` used), which makes Google
/// Fonts serve WOFF2.
pub const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Safari/605.1.15";

/// The default fetcher: `/usr/bin/curl`, public addresses only (each host resolved here
/// and curl pinned to the address that was checked), redirects followed here so that
/// every hop is checked, GET only, status 200 only, at most 32 MB.
pub struct CurlFetcher {
    program: PathBuf,
    user_agent: String,
    allow_loopback: bool,
    time_limit: Duration,
    work_dir: Option<PathBuf>,
}

static FETCH_COUNTER: AtomicUsize = AtomicUsize::new(0);

impl Default for CurlFetcher {
    fn default() -> Self {
        CurlFetcher::new()
    }
}

impl CurlFetcher {
    /// Public addresses only; `-q` first; redirects followed here.
    pub fn new() -> CurlFetcher {
        CurlFetcher::with_program(PathBuf::from("/usr/bin/curl"))
    }

    pub fn with_program(program: PathBuf) -> CurlFetcher {
        CurlFetcher {
            program,
            user_agent: USER_AGENT.to_string(),
            allow_loopback: false,
            time_limit: FETCH_TIMEOUT,
            work_dir: None,
        }
    }

    /// Tests only: their server is on 127.0.0.1. Addresses on this Mac (loopback, and
    /// the name `localhost`) are fetched; the local network, link-local, shared and
    /// `.local` addresses are still refused, so that a redirect from the test server to
    /// one is seen refused.
    pub fn allowing_local(mut self) -> CurlFetcher {
        self.allow_loopback = true;
        self
    }

    /// Tests: another time limit per address than FETCH_TIMEOUT.
    pub fn with_time_limit(mut self, limit: Duration) -> CurlFetcher {
        self.time_limit = limit;
        self
    }

    /// Where the temporary files of a fetch go (the system's temporary directory
    /// otherwise); each fetch makes a folder of its own there and removes it.
    pub fn with_work_dir(mut self, dir: PathBuf) -> CurlFetcher {
        self.work_dir = Some(dir);
        self
    }

    /// Whether an address may be asked, and the address its host is pinned to.
    fn check(&self, url: &Url) -> Result<Option<SocketAddr>, FetchError> {
        let shown = url.to_string();
        let host = match url.host() {
            Some(host) => host,
            None => return Err(FetchError::new(shown, "it has no host")),
        };
        let port = url.port_or_known_default().unwrap_or(80);
        let addresses: Vec<IpAddr> = match host {
            url::Host::Ipv4(ip) => vec![IpAddr::V4(ip)],
            url::Host::Ipv6(ip) => vec![IpAddr::V6(ip)],
            url::Host::Domain(name) => {
                let name = name.trim_end_matches('.').to_ascii_lowercase();
                let loopback_name = name == "localhost" || name.ends_with(".localhost");
                if loopback_name && !self.allow_loopback {
                    return Err(FetchError::local(shown));
                }
                if !loopback_name && (!name.contains('.') || name.ends_with(".local") || name.ends_with(".internal") || name.ends_with(".lan") || name.ends_with(".home.arpa")) {
                    return Err(FetchError::local(shown));
                }
                match (name.as_str(), port).to_socket_addrs() {
                    Ok(found) => found.map(|a| a.ip()).collect(),
                    Err(_) => return Err(FetchError::new(shown, "no network: Could not resolve host")),
                }
            }
        };
        if addresses.is_empty() {
            return Err(FetchError::new(shown, "no network: Could not resolve host"));
        }
        for ip in &addresses {
            let loopback = is_loopback(ip);
            if (loopback && !self.allow_loopback) || (!loopback && !is_public(ip)) {
                return Err(FetchError::local(shown));
            }
        }
        match url.host() {
            Some(url::Host::Domain(_)) => Ok(Some(SocketAddr::new(addresses[0], port))),
            _ => Ok(None),
        }
    }

    fn once(&self, url: &Url, pin: Option<SocketAddr>, dir: &Path) -> Result<Answer, FetchError> {
        let shown = url.to_string();
        let headers = dir.join("headers");
        let body = dir.join("body");
        let mut command = Command::new(&self.program);
        command
            .arg("-q")
            .args(["--silent", "--show-error", "--globoff"])
            .args(["--proto", "=http,https"])
            .args(["--connect-timeout", &self.time_limit.as_secs().clamp(1, 10).to_string()])
            .args(["--max-time", &format!("{:.3}", self.time_limit.as_secs_f64())])
            .args(["--max-filesize", &MAX_FILE_BYTES.to_string()])
            .args(["--user-agent", &self.user_agent]);
        if let (Some(pin), Some(host)) = (pin, url.host_str()) {
            let address = match pin.ip() {
                IpAddr::V4(ip) => ip.to_string(),
                IpAddr::V6(ip) => format!("[{ip}]"),
            };
            command.args(["--resolve", &format!("{host}:{}:{address}", pin.port())]);
        }
        command
            .arg("--dump-header")
            .arg(&headers)
            .arg("--output")
            .arg(&body)
            .args(["--write-out", "%{http_code}\t%{content_type}"])
            .arg(shown.as_str())
            .stdin(Stdio::null());
        let output = command
            .output()
            .map_err(|error| FetchError::new(&shown, format!("curl could not be run: {error}")))?;
        let code = output.status.code().unwrap_or(-1);
        if code != 0 {
            let _ = fs::remove_file(&body);
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(FetchError::new(&shown, curl_reason(code, &stderr, self.time_limit)));
        }
        let written = String::from_utf8_lossy(&output.stdout).into_owned();
        let mut parts = written.splitn(2, '\t');
        let status: u16 = parts.next().unwrap_or("").trim().parse().unwrap_or(0);
        let content_type = parts.next().unwrap_or("").trim().to_string();
        let location = fs::read_to_string(&headers).ok().and_then(|text| header(&text, "location"));
        Ok(Answer { status, content_type, location, body })
    }
}

struct Answer {
    status: u16,
    content_type: String,
    location: Option<String>,
    body: PathBuf,
}

/// curl's own sentence, without "curl: (6) ".
fn curl_reason(code: i32, stderr: &str, limit: Duration) -> String {
    let last = stderr.lines().rev().find(|line| !line.trim().is_empty()).unwrap_or("").trim();
    let sentence = match last.find(") ") {
        Some(at) if last.starts_with("curl: (") => last[at + 2..].to_string(),
        _ => last.to_string(),
    };
    match code {
        6 => format!("no network: {}", if sentence.is_empty() { "Could not resolve host".to_string() } else { sentence }),
        7 => format!("no connection: {sentence}"),
        28 => format!("it took longer than {} s", limit.as_secs().max(1)),
        63 => "over 32 MB".to_string(),
        _ if sentence.is_empty() => format!("curl failed (exit code {code})"),
        _ => sentence,
    }
}

/// The last value of a header in curl's dump (the last response's block).
fn header(dump: &str, name: &str) -> Option<String> {
    let mut value = None;
    for line in dump.lines() {
        if line.starts_with("HTTP/") {
            value = None;
            continue;
        }
        if let Some((key, rest)) = line.split_once(':') {
            if key.trim().eq_ignore_ascii_case(name) {
                value = Some(rest.trim().to_string());
            }
        }
    }
    value
}

impl Fetcher for CurlFetcher {
    fn fetch(&self, url: &str) -> Result<Fetched, FetchError> {
        let mut current = match Url::parse(url) {
            Ok(parsed) if parsed.scheme() == "http" || parsed.scheme() == "https" => parsed,
            _ => return Err(FetchError::new(url, "it is not an http or https address")),
        };
        current.set_fragment(None);
        let base = self.work_dir.clone().unwrap_or_else(std::env::temp_dir);
        let dir = base.join(format!(
            "writer-page-fetch-{}-{}",
            std::process::id(),
            FETCH_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).map_err(|error| FetchError::new(url, format!("no temporary folder: {error}")))?;
        let result = (|| {
            for hop in 0..=MAX_REDIRECTS {
                let pin = match self.check(&current) {
                    Ok(pin) => pin,
                    Err(mut error) => {
                        if hop > 0 {
                            // A redirect led there: the address the page names is said,
                            // and where it was sent.
                            error.reason = format!("it is sent on to {}, which {}", error.url, error.reason);
                        }
                        error.url = url.to_string();
                        return Err(error);
                    }
                };
                let answer = self.once(&current, pin, &dir).map_err(|mut error| {
                    error.url = url.to_string();
                    error
                })?;
                if matches!(answer.status, 301 | 302 | 303 | 307 | 308) {
                    let _ = fs::remove_file(&answer.body);
                    let Some(location) = answer.location else {
                        return Err(FetchError::new(url, format!("HTTP {} without a Location", answer.status)));
                    };
                    current = match current.join(&location) {
                        Ok(next) if next.scheme() == "http" || next.scheme() == "https" => next,
                        _ => return Err(FetchError::new(url, format!("it is sent on to {location}, which is no http or https address"))),
                    };
                    current.set_fragment(None);
                    continue;
                }
                if answer.status != 200 {
                    let _ = fs::remove_file(&answer.body);
                    return Err(FetchError::new(url, format!("HTTP {}", answer.status)));
                }
                let bytes = fs::read(&answer.body).unwrap_or_default();
                let _ = fs::remove_file(&answer.body);
                if bytes.len() as u64 > MAX_FILE_BYTES {
                    return Err(FetchError::new(url, "over 32 MB"));
                }
                let media = if answer.content_type.is_empty() {
                    media_type(current.path()).to_string()
                } else {
                    answer.content_type
                };
                return Ok(Fetched { final_url: current.to_string(), media_type: Some(media), bytes });
            }
            Err(FetchError::new(url, format!("too many redirects (more than {MAX_REDIRECTS})")))
        })();
        let _ = fs::remove_dir_all(&dir);
        result
    }
}

fn is_loopback(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback(),
        IpAddr::V6(v6) => v6.is_loopback() || v6.to_ipv4_mapped().map(|v4| v4.is_loopback()).unwrap_or(false),
    }
}

/// An address on the internet: not loopback, unspecified, private, link-local, shared,
/// multicast, broadcast, documentation or reserved.
pub(crate) fn is_public(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public_v4(&v4);
            }
            let s = v6.segments();
            // NAT64 (64:ff9b::/96) carries an IPv4 address.
            if s[0] == 0x64 && s[1] == 0xff9b && s[2..6].iter().all(|&x| x == 0) {
                let v4 = Ipv4Addr::new((s[6] >> 8) as u8, s[6] as u8, (s[7] >> 8) as u8, s[7] as u8);
                return is_public_v4(&v4);
            }
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00 // unique local fc00::/7
                || (s[0] & 0xffc0) == 0xfe80 // link-local fe80::/10
                || (s[0] & 0xffc0) == 0xfec0 // site-local fec0::/10
                || (s[0] == 0x2001 && s[1] == 0x0db8) // documentation
                || is_v4_compatible(v6))
        }
    }
}

fn is_v4_compatible(v6: &Ipv6Addr) -> bool {
    let s = v6.segments();
    s[..6].iter().all(|&x| x == 0)
}

fn is_public_v4(v4: &Ipv4Addr) -> bool {
    let o = v4.octets();
    !(v4.is_loopback()
        || v4.is_unspecified()
        || v4.is_private()
        || v4.is_link_local()
        || v4.is_multicast()
        || v4.is_broadcast()
        || v4.is_documentation()
        || o[0] == 0 // "this network"
        || (o[0] == 100 && (o[1] & 0xc0) == 64) // shared 100.64/10
        || (o[0] == 198 && (o[1] & 0xfe) == 18) // benchmarking 198.18/15
        || (o[0] == 192 && o[1] == 0 && o[2] == 0) // protocol assignments
        || o[0] >= 240) // reserved
}

/// Pieces the caller already has (`externals`: address -> local file), read now, before
/// anything is fetched; every other address goes to `inner`.
pub struct KnownPieces {
    known: HashMap<String, Fetched>,
    inner: Arc<dyn Fetcher>,
}

impl KnownPieces {
    pub fn read(map: &BTreeMap<String, PathBuf>, inner: Arc<dyn Fetcher>) -> Result<KnownPieces, PageError> {
        let mut known = HashMap::new();
        for (address, file) in map {
            let Some(normal) = normal_address(address) else {
                return Err(PageError::input(format!(
                    "{address} is not an http or https address, so no file stands for it"
                )));
            };
            let meta = fs::metadata(file).map_err(|error| PageError::io(file, error))?;
            if !meta.is_file() {
                return Err(PageError::input(format!("{} is not a file", file.display())));
            }
            if meta.len() > MAX_FILE_BYTES {
                return Err(PageError::Incomplete {
                    name: file_label(file),
                    problems: vec![too_large(&file.display().to_string(), meta.len())],
                });
            }
            let bytes = fs::read(file).map_err(|error| PageError::io(file, error))?;
            let name = file.to_string_lossy();
            let media = match media_type(&name) {
                OCTET_STREAM => media_type(Url::parse(&normal).map(|u| u.path().to_string()).unwrap_or_default().as_str()),
                found => found,
            };
            known.insert(normal.clone(), Fetched { final_url: normal, media_type: Some(media.to_string()), bytes });
        }
        Ok(KnownPieces { known, inner })
    }
}

fn file_label(file: &Path) -> String {
    file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| file.display().to_string())
}

impl Fetcher for KnownPieces {
    fn fetch(&self, url: &str) -> Result<Fetched, FetchError> {
        let key = normal_address(url).unwrap_or_else(|| url.to_string());
        match self.known.get(&key) {
            Some(found) => Ok(found.clone()),
            None => self.inner.fetch(url),
        }
    }
}

/// Fetches each address once (without its fragment) for all the pages of one command;
/// errors are kept too. What it holds is let go, oldest first, past 256 MB.
pub struct CachingFetcher {
    inner: Arc<dyn Fetcher>,
    cache: Mutex<Cache>,
}

#[derive(Default)]
struct Cache {
    answers: HashMap<String, Result<Fetched, FetchError>>,
    order: VecDeque<String>,
    bytes: u64,
}

const CACHE_BYTES: u64 = 256 * 1024 * 1024;

impl CachingFetcher {
    pub fn new(inner: Arc<dyn Fetcher>) -> CachingFetcher {
        CachingFetcher { inner, cache: Mutex::new(Cache::default()) }
    }
}

impl Fetcher for CachingFetcher {
    fn fetch(&self, url: &str) -> Result<Fetched, FetchError> {
        let key = normal_address(url).unwrap_or_else(|| url.to_string());
        if let Some(found) = self.cache.lock().unwrap_or_else(|e| e.into_inner()).answers.get(&key) {
            return found.clone();
        }
        let answer = self.inner.fetch(url);
        let size = answer.as_ref().map(|f| f.bytes.len() as u64).unwrap_or(0);
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if !cache.answers.contains_key(&key) {
            cache.answers.insert(key.clone(), answer.clone());
            cache.order.push_back(key);
            cache.bytes += size;
            while cache.bytes > CACHE_BYTES {
                let Some(oldest) = cache.order.pop_front() else { break };
                if let Some(Ok(gone)) = cache.answers.remove(&oldest) {
                    cache.bytes -= gone.bytes.len() as u64;
                }
            }
        }
        answer
    }
}

/// Address to answer, in memory: the fetcher of every test that runs no server. An
/// address it does not have answers "HTTP 404".
pub struct MapFetcher(pub HashMap<String, Fetched>);

impl MapFetcher {
    /// An answer for `url` with these bytes and media type, at that address.
    pub fn with(mut self, url: &str, media_type: &str, bytes: impl Into<Vec<u8>>) -> MapFetcher {
        self.0.insert(
            url.to_string(),
            Fetched { final_url: url.to_string(), media_type: Some(media_type.to_string()), bytes: bytes.into() },
        );
        self
    }

    pub fn new() -> MapFetcher {
        MapFetcher(HashMap::new())
    }
}

impl Default for MapFetcher {
    fn default() -> Self {
        MapFetcher::new()
    }
}

impl Fetcher for MapFetcher {
    fn fetch(&self, url: &str) -> Result<Fetched, FetchError> {
        if let Some(found) = self.0.get(url) {
            return Ok(found.clone());
        }
        let bare = url.split('#').next().unwrap_or(url);
        if let Some(found) = self.0.get(bare) {
            return Ok(found.clone());
        }
        if let Some(normal) = normal_address(url) {
            if let Some(found) = self.0.get(&normal) {
                return Ok(found.clone());
            }
            if let Some(found) = self.0.iter().find(|(key, _)| normal_address(key).as_deref() == Some(normal.as_str())) {
                return Ok(found.1.clone());
            }
        }
        Err(FetchError::new(url, "HTTP 404"))
    }
}
