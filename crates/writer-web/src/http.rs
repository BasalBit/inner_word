//! What the scheme handler answers, as data: the path a URL names, the byte range
//! a request asks for, the status and the headers of every answer. No Objective-C
//! here, so all of it is tested without WebKit.

use crate::{CONTENT_SECURITY_POLICY, SCHEME, ServedFile};

/// The part of a file a request asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Range {
    /// No usable `Range` header: the whole file (200).
    Whole,
    /// `first..=last`, both inside the file (206).
    Part { first: u64, last: u64 },
    /// A range that starts past the end of the file (416).
    Unsatisfiable,
}

/// The range of a `Range` header for a file of `total` bytes. A header in a form
/// this does not read (another unit, several ranges, no numbers) is ignored, as
/// HTTP says, and the whole file is answered.
pub(crate) fn parse_range(header: Option<&str>, total: u64) -> Range {
    let Some(header) = header else {
        return Range::Whole;
    };
    let Some(spec) = header.trim().strip_prefix("bytes=") else {
        return Range::Whole;
    };
    if spec.contains(',') {
        return Range::Whole;
    }
    let Some((from, to)) = spec.split_once('-') else {
        return Range::Whole;
    };
    let (from, to) = (from.trim(), to.trim());
    match (from.is_empty(), to.is_empty()) {
        // `bytes=-n`: the last n bytes.
        (true, false) => {
            let Ok(n) = to.parse::<u64>() else {
                return Range::Whole;
            };
            if n == 0 || total == 0 {
                return Range::Unsatisfiable;
            }
            Range::Part {
                first: total - n.min(total),
                last: total - 1,
            }
        }
        // `bytes=a-`: from a to the end.
        (false, true) => {
            let Ok(first) = from.parse::<u64>() else {
                return Range::Whole;
            };
            if first >= total {
                return Range::Unsatisfiable;
            }
            Range::Part {
                first,
                last: total - 1,
            }
        }
        // `bytes=a-b`.
        (false, false) => {
            let (Ok(first), Ok(last)) = (from.parse::<u64>(), to.parse::<u64>()) else {
                return Range::Whole;
            };
            if last < first {
                return Range::Whole;
            }
            if first >= total {
                return Range::Unsatisfiable;
            }
            Range::Part {
                first,
                last: last.min(total - 1),
            }
        }
        (true, true) => Range::Whole,
    }
}

/// What one request is answered with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Answer {
    pub status: u16,
    pub headers: Vec<(&'static str, String)>,
    /// The bytes of the file that go out, as offsets (`start..end`); none for a 404,
    /// a 416 and a `HEAD`.
    pub body: Option<(usize, usize)>,
    /// The body of an answer that has no file behind it (404, 416).
    pub text: &'static str,
}

/// The headers every answer carries, whatever its type: the page's policy (on HTML so
/// that the page has it, on scripts so that a worker has it, on SVG and XML because
/// such a document in a frame runs script) and no DNS prefetching.
fn common_headers() -> Vec<(&'static str, String)> {
    vec![
        (
            "Content-Security-Policy",
            CONTENT_SECURITY_POLICY.to_string(),
        ),
        ("X-DNS-Prefetch-Control", "off".to_string()),
        ("Accept-Ranges", "bytes".to_string()),
        ("Cache-Control", "no-store".to_string()),
    ]
}

/// The answer to a request for a file (`None`: the page has no such file).
pub(crate) fn answer(file: Option<&ServedFile>, range: Option<&str>, head: bool) -> Answer {
    let mut headers = common_headers();
    let Some(file) = file else {
        const TEXT: &str = "This file is not among the files of the page.";
        headers.push(("Content-Type", "text/plain; charset=utf-8".to_string()));
        headers.push(("Content-Length", TEXT.len().to_string()));
        return Answer {
            status: 404,
            headers,
            body: None,
            text: if head { "" } else { TEXT },
        };
    };
    let total = file.bytes.len() as u64;
    headers.push(("Content-Type", served_type(&file.media_type, &file.bytes)));
    match parse_range(range, total) {
        Range::Whole => {
            headers.push(("Content-Length", total.to_string()));
            Answer {
                status: 200,
                headers,
                body: (!head).then_some((0, total as usize)),
                text: "",
            }
        }
        Range::Part { first, last } => {
            headers.push(("Content-Length", (last - first + 1).to_string()));
            headers.push(("Content-Range", format!("bytes {first}-{last}/{total}")));
            Answer {
                status: 206,
                headers,
                body: (!head).then_some((first as usize, last as usize + 1)),
                text: "",
            }
        }
        Range::Unsatisfiable => {
            headers.retain(|(name, _)| *name != "Content-Type");
            headers.push(("Content-Type", "text/plain; charset=utf-8".to_string()));
            headers.push(("Content-Range", format!("bytes */{total}")));
            headers.push(("Content-Length", "0".to_string()));
            Answer {
                status: 416,
                headers,
                body: None,
                text: "",
            }
        }
    }
}

/// The media type an answer is served with. The type comes from the manifest; HTML is
/// the one exception: what it says about its encoding is decided by its bytes. With a
/// byte order mark or a `<meta charset>` of its own it is served without a charset
/// (WebKit reads the page's declaration, which a charset in the header would
/// override), otherwise with `charset=utf-8` when its bytes are UTF-8, otherwise
/// without (WebKit guesses, as a browser does with a file).
pub(crate) fn served_type(media_type: &str, bytes: &[u8]) -> String {
    let essence = media_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if essence != "text/html" {
        return media_type.to_string();
    }
    let bom = bytes.starts_with(&[0xEF, 0xBB, 0xBF])
        || bytes.starts_with(&[0xFE, 0xFF])
        || bytes.starts_with(&[0xFF, 0xFE]);
    if !bom && is_binary(bytes) {
        // Bytes that are no text are never served as HTML: 0.8.0 stored some fonts of
        // Google Fonts as `text/html`, because their server says so, and a font loaded as
        // a document would be its bytes as text.
        return "application/octet-stream".to_string();
    }
    let declares = bom || declares_charset(bytes);
    if declares || std::str::from_utf8(bytes).is_err() {
        "text/html".to_string()
    } else {
        "text/html; charset=utf-8".to_string()
    }
}

/// Whether bytes hold one that no text has (the "binary data bytes" of the WHATWG's MIME
/// sniffing: controls other than tab, line feed, form feed, carriage return and escape)
/// in their first 1445 bytes.
fn is_binary(bytes: &[u8]) -> bool {
    bytes
        .iter()
        .take(1445)
        .any(|&b| matches!(b, 0x00..=0x08 | 0x0B | 0x0E..=0x1A | 0x1C..=0x1F))
}

/// Whether the first 1024 bytes of an HTML file have a `<meta>` that names a charset
/// (`<meta charset=...>` or `<meta http-equiv=content-type content="...; charset=...">`).
fn declares_charset(bytes: &[u8]) -> bool {
    let head: Vec<u8> = bytes
        .iter()
        .take(1024)
        .map(u8::to_ascii_lowercase)
        .collect();
    let mut rest = head.as_slice();
    while let Some(at) = find(rest, b"<meta") {
        let tag = &rest[at..];
        let end = tag.iter().position(|&b| b == b'>').unwrap_or(tag.len());
        if find(&tag[..end], b"charset").is_some() {
            return true;
        }
        rest = &tag[end.min(tag.len())..];
        if rest.is_empty() {
            break;
        }
    }
    false
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// The host and the path (percent-decoded, no leading slash, without query and
/// fragment) of a `writer-page:` URL; `None` for any other URL.
pub(crate) fn split_url(url: &str) -> Option<(String, String)> {
    let rest = url.strip_prefix(SCHEME)?.strip_prefix("://")?;
    let rest = rest.split(['?', '#']).next().unwrap_or("");
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    Some((host.to_ascii_lowercase(), percent_decode(path)))
}

/// `%xx` sequences as the bytes they stand for; bytes that are not UTF-8 afterwards
/// are replaced (no file of a page has such a name: the bundler reads names as text).
pub(crate) fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let (Some(high), Some(low)) = (hex(bytes[index + 1]), hex(bytes[index + 2]))
        {
            out.push(high * 16 + low);
            index += 3;
            continue;
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// A path of the page written into a URL: every byte outside the unreserved
/// characters and `/` percent-encoded.
pub(crate) fn percent_encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for &byte in path.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// The URL of a file of the page served under `host`.
pub(crate) fn url_of(host: &str, path: &str) -> String {
    format!(
        "{SCHEME}://{host}/{}",
        percent_encode_path(path.trim_start_matches('/'))
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn file(media_type: &str, bytes: &[u8]) -> ServedFile {
        ServedFile {
            media_type: media_type.to_string(),
            bytes: Arc::from(bytes),
        }
    }

    fn header<'a>(answer: &'a Answer, name: &str) -> Option<&'a str> {
        answer
            .headers
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| v.as_str())
    }

    #[test]
    fn ranges_are_read_as_http_reads_them() {
        assert_eq!(parse_range(None, 10), Range::Whole);
        assert_eq!(
            parse_range(Some("bytes=0-1"), 10),
            Range::Part { first: 0, last: 1 }
        );
        assert_eq!(
            parse_range(Some("bytes=4-"), 10),
            Range::Part { first: 4, last: 9 }
        );
        assert_eq!(
            parse_range(Some("bytes=-3"), 10),
            Range::Part { first: 7, last: 9 }
        );
        // More than the file: all of it.
        assert_eq!(
            parse_range(Some("bytes=-30"), 10),
            Range::Part { first: 0, last: 9 }
        );
        assert_eq!(
            parse_range(Some("bytes=2-100"), 10),
            Range::Part { first: 2, last: 9 }
        );
        assert_eq!(parse_range(Some("bytes=10-"), 10), Range::Unsatisfiable);
        assert_eq!(parse_range(Some("bytes=12-20"), 10), Range::Unsatisfiable);
        assert_eq!(parse_range(Some("bytes=-0"), 10), Range::Unsatisfiable);
        assert_eq!(parse_range(Some("bytes=0-"), 0), Range::Unsatisfiable);
        // What is not read is ignored.
        assert_eq!(parse_range(Some("bytes=5-2"), 10), Range::Whole);
        assert_eq!(parse_range(Some("bytes=0-1,4-5"), 10), Range::Whole);
        assert_eq!(parse_range(Some("items=0-1"), 10), Range::Whole);
        assert_eq!(parse_range(Some("bytes=a-b"), 10), Range::Whole);
        assert_eq!(parse_range(Some("bytes=-"), 10), Range::Whole);
    }

    #[test]
    fn every_answer_carries_the_policy() {
        let audio = file("audio/mpeg", &[7u8; 200]);
        for answer in [
            super::answer(Some(&audio), None, false),
            super::answer(Some(&audio), Some("bytes=0-1"), false),
            super::answer(Some(&audio), Some("bytes=500-"), false),
            super::answer(None, None, false),
            super::answer(Some(&file("image/svg+xml", b"<svg/>")), None, false),
        ] {
            assert_eq!(
                header(&answer, "Content-Security-Policy"),
                Some(CONTENT_SECURITY_POLICY)
            );
            assert_eq!(header(&answer, "X-DNS-Prefetch-Control"), Some("off"));
            assert_eq!(header(&answer, "Accept-Ranges"), Some("bytes"));
        }
    }

    #[test]
    fn a_range_is_answered_with_206_and_where_it_is() {
        let audio = file("audio/mpeg", &[7u8; 200]);
        let whole = answer(Some(&audio), None, false);
        assert_eq!(whole.status, 200);
        assert_eq!(whole.body, Some((0, 200)));
        assert_eq!(header(&whole, "Content-Length"), Some("200"));

        let part = answer(Some(&audio), Some("bytes=100-"), false);
        assert_eq!(part.status, 206);
        assert_eq!(part.body, Some((100, 200)));
        assert_eq!(header(&part, "Content-Range"), Some("bytes 100-199/200"));
        assert_eq!(header(&part, "Content-Length"), Some("100"));
        assert_eq!(header(&part, "Content-Type"), Some("audio/mpeg"));

        let tail = answer(Some(&audio), Some("bytes=-10"), false);
        assert_eq!(tail.body, Some((190, 200)));

        let outside = answer(Some(&audio), Some("bytes=300-400"), false);
        assert_eq!(outside.status, 416);
        assert_eq!(outside.body, None);
        assert_eq!(header(&outside, "Content-Range"), Some("bytes */200"));

        let missing = answer(None, None, false);
        assert_eq!(missing.status, 404);
        assert!(!missing.text.is_empty());

        let head = answer(Some(&audio), None, true);
        assert_eq!(head.status, 200);
        assert_eq!(head.body, None);
        assert_eq!(header(&head, "Content-Length"), Some("200"));
    }

    #[test]
    fn html_says_utf8_only_when_it_is_and_declares_nothing() {
        assert_eq!(
            served_type("text/html", "<p>Grüße</p>".as_bytes()),
            "text/html; charset=utf-8"
        );
        // A charset the manifest gave is not believed for HTML: the bytes decide.
        assert_eq!(
            served_type("text/html; charset=utf-8", b"<p>Gr\xfc\xdfe</p>"),
            "text/html"
        );
        assert_eq!(
            served_type(
                "text/html; charset=utf-8",
                b"<head><meta charset=\"windows-1252\"></head><p>Gr\xfc\xdfe</p>"
            ),
            "text/html"
        );
        assert_eq!(
            served_type(
                "text/html",
                b"<META HTTP-EQUIV=\"Content-Type\" CONTENT=\"text/html; charset=iso-8859-1\">"
            ),
            "text/html"
        );
        assert_eq!(
            served_type("text/html", b"\xEF\xBB\xBF<p>x</p>"),
            "text/html"
        );
        // Bytes that are no text (a font a server called HTML) are never served as HTML.
        assert_eq!(
            served_type("text/html; charset=utf-8", b"wOF2\x00\x01\x00\x00\x00\x00"),
            "application/octet-stream"
        );
        assert_eq!(
            served_type("text/html", b"\xFF\xFE<\x00p\x00>\x00"),
            "text/html"
        );
        // A <meta> without a charset is no declaration.
        assert_eq!(
            served_type("text/html", b"<meta name=viewport content=x><p>x</p>"),
            "text/html; charset=utf-8"
        );
        // Every other type is served as the manifest says.
        assert_eq!(
            served_type("text/css; charset=utf-8", b"a{}"),
            "text/css; charset=utf-8"
        );
        assert_eq!(served_type("font/woff2", b"\x00\x01"), "font/woff2");
    }

    #[test]
    fn a_url_names_its_host_and_a_decoded_path() {
        assert_eq!(
            split_url("writer-page://page-12/audio/01%20intro.mp3?x=1#t"),
            Some(("page-12".to_string(), "audio/01 intro.mp3".to_string()))
        );
        assert_eq!(
            split_url("writer-page://CHECK/"),
            Some(("check".to_string(), String::new()))
        );
        assert_eq!(
            split_url("writer-page://check"),
            Some(("check".to_string(), String::new()))
        );
        assert_eq!(
            split_url("writer-page://page-1/caf%C3%A9.html"),
            Some(("page-1".to_string(), "café.html".to_string()))
        );
        // A lone percent sign stays what it is.
        assert_eq!(
            split_url("writer-page://page-1/100%.html"),
            Some(("page-1".to_string(), "100%.html".to_string()))
        );
        assert_eq!(split_url("https://example.com/a"), None);
        assert_eq!(
            url_of("page-3", "offline/index page.html"),
            "writer-page://page-3/offline/index%20page.html"
        );
        assert_eq!(
            url_of("check", "a?b#c.html"),
            "writer-page://check/a%3Fb%23c.html"
        );
        let (_, path) = split_url(&url_of("page-3", "Grüße/ä b.html")).unwrap();
        assert_eq!(path, "Grüße/ä b.html");
    }
}
