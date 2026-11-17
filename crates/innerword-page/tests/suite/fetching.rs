//! `CurlFetcher` against a server on 127.0.0.1 in a thread (no test reaches the
//! internet), `CachingFetcher`, and the address check that keeps the tool from asking
//! this Mac or its local network.

use std::collections::HashMap;
use std::io::{Read, Write as _};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use innerword_page::{
    CachingFetcher, CurlFetcher, FetchError, Fetched, Fetcher, USER_AGENT, bundle, read_input,
};

use super::support::*;

#[derive(Clone, Debug)]
struct Seen {
    path: String,
    headers: HashMap<String, String>,
}

struct Server {
    port: u16,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Server {
    fn start() -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen: Arc<Mutex<Vec<Seen>>> = Arc::default();
        let log = seen.clone();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let log = log.clone();
                thread::spawn(move || answer(stream, port, log));
            }
        });
        Server { port, seen }
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    fn requests(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

fn answer(mut stream: TcpStream, port: u16, log: Arc<Mutex<Vec<Seen>>>) {
    let mut request = Vec::new();
    let mut buf = [0u8; 4096];
    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => request.extend_from_slice(&buf[..n]),
        }
    }
    let text = String::from_utf8_lossy(&request).into_owned();
    let mut lines = text.split("\r\n");
    let path = lines
        .next()
        .unwrap_or("")
        .split(' ')
        .nth(1)
        .unwrap_or("/")
        .to_string();
    let headers: HashMap<String, String> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    log.lock().unwrap().push(Seen {
        path: path.clone(),
        headers,
    });
    let respond = |stream: &mut TcpStream, status: &str, extra: &str, body: &[u8]| {
        let head = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n",
            body.len()
        );
        let _ = stream.write_all(head.as_bytes());
        let _ = stream.write_all(body);
    };
    match path.as_str() {
        "/ok.css" => respond(&mut stream, "200 OK", "Content-Type: text/css; charset=utf-8\r\n", b"a{}"),
        "/page.css" => respond(
            &mut stream,
            "200 OK",
            "Content-Type: text/css\r\n",
            format!("@font-face{{src:url(/font.woff2)}} body{{background:url(http://127.0.0.1:{port}/bg.png)}}").as_bytes(),
        ),
        "/font.woff2" => respond(&mut stream, "200 OK", "Content-Type: font/woff2\r\n", b"wOF2"),
        // As fonts.gstatic.com answers some kits: WOFF2 bytes called HTML.
        "/fonts.css" => respond(
            &mut stream,
            "200 OK",
            "Content-Type: text/css; charset=utf-8\r\n",
            b"@font-face{font-family:A;src:url(/l/font?kit=html&v=9) format('woff2')}@font-face{font-family:B;src:url(/l/font?kit=woff2&v=9) format('woff2')}",
        ),
        "/l/font?kit=html&v=9" => respond(&mut stream, "200 OK", "Content-Type: text/html; charset=utf-8\r\n", &super::types::woff2(7)),
        "/l/font?kit=woff2&v=9" => respond(&mut stream, "200 OK", "Content-Type: font/woff2\r\n", &super::types::woff2(8)),
        "/bg.png" => respond(&mut stream, "200 OK", "", b"PNG"),
        "/redirect" => respond(&mut stream, "302 Found", "Location: /ok.css\r\n", b""),
        "/to-private" => respond(&mut stream, "301 Moved Permanently", "Location: http://10.0.0.1/x\r\n", b""),
        "/slow" => thread::sleep(Duration::from_secs(5)),
        "/big" => respond(&mut stream, "200 OK", "Content-Type: application/octet-stream\r\n", &vec![b'x'; 33 * 1024 * 1024]),
        "/big-unannounced" => {
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n");
            let chunk = vec![b'y'; 1024 * 1024];
            for _ in 0..33 {
                if stream.write_all(&chunk).is_err() {
                    break;
                }
            }
        }
        p if p.starts_with("/loop/") => {
            let n: usize = p[6..].parse().unwrap_or(0);
            respond(&mut stream, "302 Found", &format!("Location: /loop/{}\r\n", n + 1), b"");
        }
        _ => respond(&mut stream, "404 Not Found", "", b"not here"),
    }
}

fn fetcher(work: &Path) -> CurlFetcher {
    CurlFetcher::new()
        .allowing_local()
        .with_work_dir(work.to_path_buf())
}

fn left_behind(work: &Path) -> Vec<String> {
    std::fs::read_dir(work)
        .map(|d| {
            d.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn a_200_with_the_user_agent_of_safari() {
    let server = Server::start();
    let work = temp();
    let fetched = fetcher(work.path()).fetch(&server.url("/ok.css")).unwrap();
    assert_eq!(fetched.bytes, b"a{}");
    assert_eq!(
        fetched.media_type.as_deref(),
        Some("text/css; charset=utf-8")
    );
    assert_eq!(fetched.final_url, server.url("/ok.css"));
    let seen = server.requests();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        seen[0].headers.get("user-agent").map(String::as_str),
        Some(USER_AGENT)
    );
    assert!(
        left_behind(work.path()).is_empty(),
        "every temporary file is gone"
    );
}

#[test]
fn a_404_is_an_error_and_leaves_nothing() {
    let server = Server::start();
    let work = temp();
    let error = fetcher(work.path())
        .fetch(&server.url("/nothing.png"))
        .unwrap_err();
    assert_eq!(error.reason, "HTTP 404");
    assert_eq!(error.url, server.url("/nothing.png"));
    assert_eq!(
        error.sentence(),
        format!(
            "{} could not be fetched: HTTP 404",
            server.url("/nothing.png")
        )
    );
    assert!(left_behind(work.path()).is_empty());
}

#[test]
fn redirects_are_followed_here_at_most_five() {
    let server = Server::start();
    let work = temp();
    let fetched = fetcher(work.path())
        .fetch(&server.url("/redirect"))
        .unwrap();
    assert_eq!(fetched.final_url, server.url("/ok.css"));
    assert_eq!(fetched.bytes, b"a{}");
    let error = fetcher(work.path())
        .fetch(&server.url("/loop/0"))
        .unwrap_err();
    assert_eq!(error.reason, "too many redirects (more than 5)");
    let loops = server
        .requests()
        .iter()
        .filter(|s| s.path.starts_with("/loop/"))
        .count();
    assert_eq!(loops, 6, "the address and five redirects were asked");
    assert!(left_behind(work.path()).is_empty());
}

#[test]
fn a_server_that_does_not_answer_in_time() {
    let server = Server::start();
    let work = temp();
    let error = fetcher(work.path())
        .with_time_limit(Duration::from_secs(1))
        .fetch(&server.url("/slow"))
        .unwrap_err();
    assert_eq!(error.reason, "it took longer than 1 s");
    assert!(left_behind(work.path()).is_empty());
}

#[test]
fn a_body_over_32_mb_is_refused_and_its_file_is_gone() {
    let server = Server::start();
    let work = temp();
    for path in ["/big", "/big-unannounced"] {
        let error = fetcher(work.path()).fetch(&server.url(path)).unwrap_err();
        assert_eq!(error.reason, "over 32 MB", "{path}");
        assert!(left_behind(work.path()).is_empty(), "{path}");
    }
}

#[test]
fn a_curlrc_in_home_changes_nothing() {
    let server = Server::start();
    let work = temp();
    let home = temp();
    write(
        home.path(),
        ".curlrc",
        "user-agent = \"from-curlrc\"\nheader = \"X-From-Curlrc: 1\"\nlocation\n",
    );
    let wrapper = home.path().join("curl-with-home.sh");
    write(
        home.path(),
        "curl-with-home.sh",
        format!(
            "#!/bin/sh\nHOME='{}' exec /usr/bin/curl \"$@\"\n",
            home.path().display()
        ),
    );
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
    let curl = CurlFetcher::with_program(wrapper)
        .allowing_local()
        .with_work_dir(work.path().to_path_buf());
    let fetched = curl.fetch(&server.url("/redirect")).unwrap();
    assert_eq!(fetched.bytes, b"a{}");
    for seen in server.requests() {
        assert_eq!(
            seen.headers.get("user-agent").map(String::as_str),
            Some(USER_AGENT)
        );
        assert!(!seen.headers.contains_key("x-from-curlrc"));
    }
}

#[test]
fn addresses_on_this_mac_or_its_network_are_refused_before_anything_is_asked() {
    let server = Server::start();
    let curl = CurlFetcher::new();
    for address in [
        server.url("/ok.css"),
        "http://localhost/x".to_string(),
        "http://10.0.0.1/x".to_string(),
        "http://[::1]/x".to_string(),
        "http://x.local/x".to_string(),
        "http://192.168.1.1/router.png".to_string(),
        "http://172.16.0.1/x".to_string(),
        "http://169.254.169.254/latest/meta-data".to_string(),
        "http://100.64.0.1/x".to_string(),
        "http://[fe80::1]/x".to_string(),
        "http://[fd00::1]/x".to_string(),
        "http://[::ffff:10.0.0.1]/x".to_string(),
        "http://0.0.0.0/x".to_string(),
        "http://printer/x".to_string(),
    ] {
        let error = curl.fetch(&address).unwrap_err();
        assert!(error.is_local(), "{address}: {}", error.reason);
        assert_eq!(
            error.sentence(),
            format!(
                "{address} is on this Mac or its local network; only addresses on the internet are fetched"
            )
        );
    }
    assert!(
        server.requests().is_empty(),
        "no request reached the server"
    );
}

#[test]
fn a_redirect_to_a_private_address_is_refused() {
    let server = Server::start();
    let work = temp();
    let error = fetcher(work.path())
        .fetch(&server.url("/to-private"))
        .unwrap_err();
    assert_eq!(error.url, server.url("/to-private"));
    assert_eq!(
        error.reason,
        "it is sent on to http://10.0.0.1/x, which is on this Mac or its local network; only addresses on the internet are fetched"
    );
}

#[test]
fn a_page_bundled_through_curl() {
    let server = Server::start();
    let work = temp();
    let dir = folder(&[(
        "index.html",
        &format!(
            r#"<link rel="stylesheet" href="{}">"#,
            server.url("/page.css")
        ),
    )]);
    let bundle = bundle(read_input(dir.path(), None).unwrap(), &fetcher(work.path())).unwrap();
    let css = served(
        &bundle,
        "_external/127.0.0.1_PORT/page.css"
            .replace("PORT", &server.port.to_string())
            .as_str(),
    );
    assert_eq!(
        css,
        "@font-face{src:url(font.woff2)} body{background:url(bg.png)}"
    );
    assert_eq!(bundle.externals.len(), 3);
}

/// Counts the fetches that reach it.
struct Counting {
    inner: innerword_page::MapFetcher,
    asked: AtomicUsize,
}

impl Fetcher for Counting {
    fn fetch(&self, url: &str) -> Result<Fetched, FetchError> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        self.inner.fetch(url)
    }
}

#[test]
fn each_address_is_fetched_once_per_command() {
    let counting = Arc::new(Counting {
        inner: web(&[("https://cdn.test/a.css", "text/css", "a{}")]),
        asked: AtomicUsize::new(0),
    });
    let caching = CachingFetcher::new(counting.clone());
    for _ in 0..3 {
        assert!(caching.fetch("https://cdn.test/a.css").is_ok());
        assert!(caching.fetch("https://cdn.test/a.css#part").is_ok());
        assert!(caching.fetch("https://cdn.test/gone.css").is_err());
    }
    assert_eq!(
        counting.asked.load(Ordering::SeqCst),
        2,
        "one answer and one error, each asked once"
    );
    // Two pages of one command share the font.
    let a = folder(&[(
        "index.html",
        r#"<link rel="stylesheet" href="https://cdn.test/a.css">"#,
    )]);
    let b = folder(&[(
        "index.html",
        r#"<link rel="stylesheet" href="https://cdn.test/a.css"><p>b</p>"#,
    )]);
    bundle(read_input(a.path(), None).unwrap(), &caching).unwrap();
    bundle(read_input(b.path(), None).unwrap(), &caching).unwrap();
    assert_eq!(counting.asked.load(Ordering::SeqCst), 2);
}

#[test]
fn a_font_a_server_calls_html_is_stored_as_a_font_through_curl() {
    let server = Server::start();
    let work = temp();
    let kit = server.url("/l/font?kit=html&v=9");
    let fetched = fetcher(work.path()).fetch(&kit).unwrap();
    assert_eq!(
        fetched.media_type.as_deref(),
        Some("text/html; charset=utf-8"),
        "what the server said"
    );
    let dir = folder(&[(
        "index.html",
        &format!(
            r#"<title>Branding</title><link rel="stylesheet" href="{}"><p>The words of the page.</p>"#,
            server.url("/fonts.css")
        ),
    )]);
    let bundle = bundle(read_input(dir.path(), None).unwrap(), &fetcher(work.path())).unwrap();
    for url in [kit, server.url("/l/font?kit=woff2&v=9")] {
        let path = stored(&bundle, &url);
        let file = bundle.served.iter().find(|f| f.path == path).unwrap();
        assert_eq!(file.media_type, "font/woff2", "{url}");
    }
    assert_eq!(bundle.html_paths(), vec!["index.html".to_string()]);
    assert!(innerword_page::check_request(&bundle).also.is_empty());
    let prepared =
        innerword_page::prepare(bundle.clone(), &innerword_page::static_check(&bundle)).unwrap();
    assert_eq!(prepared.manifest(None, 1).text, "The words of the page.");
}
