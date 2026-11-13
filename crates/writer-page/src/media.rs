//! Names of bytes and their media types.

use sha2::{Digest, Sha256};

/// First 16 bytes of the SHA-256, 32 lowercase hex digits:
/// `writer_store::FileId::of(bytes).to_hex()`.
pub fn file_id_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes)[..16])
}

/// Whether `text` is a file id: 32 lowercase hex digits.
pub fn is_file_id(text: &str) -> bool {
    text.len() == 32 && text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 15) as usize] as char);
    }
    out
}

pub(crate) fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

pub(crate) fn sha1_hex(text: &str) -> String {
    sha1_smol::Sha1::from(text.as_bytes()).digest().to_string()
}

/// The media type served for a path, by its ending ("text/html", "font/woff2",
/// "audio/mpeg", ...); "application/octet-stream" for an ending it does not know.
pub fn media_type(path: &str) -> &'static str {
    let name = path.rsplit('/').next().unwrap_or(path);
    let Some((_, ending)) = name.rsplit_once('.') else {
        return OCTET_STREAM;
    };
    match ending.to_ascii_lowercase().as_str() {
        "html" | "htm" => "text/html",
        "xhtml" | "xht" => "application/xhtml+xml",
        "css" => "text/css",
        "js" | "mjs" | "cjs" => "text/javascript",
        "json" | "map" => "application/json",
        "webmanifest" => "application/manifest+json",
        "xml" | "xsl" => "application/xml",
        "txt" | "md" | "markdown" => "text/plain",
        "csv" => "text/csv",
        "tsv" => "text/tab-separated-values",
        "vtt" => "text/vtt",
        "srt" => "application/x-subrip",
        "svg" | "svgz" => "image/svg+xml",
        "png" => "image/png",
        "apng" => "image/apng",
        "jpg" | "jpeg" | "jpe" | "jfif" | "pjpeg" | "pjp" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "ico" | "cur" => "image/x-icon",
        "bmp" => "image/bmp",
        "tif" | "tiff" => "image/tiff",
        "heic" => "image/heic",
        "heif" => "image/heif",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        "wav" => "audio/wav",
        "ogg" | "oga" | "opus" => "audio/ogg",
        "flac" => "audio/flac",
        "weba" => "audio/webm",
        "mid" | "midi" => "audio/midi",
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "ogv" => "video/ogg",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "eot" => "application/vnd.ms-fontobject",
        "pdf" => "application/pdf",
        "wasm" => "application/wasm",
        "glb" => "model/gltf-binary",
        "gltf" => "model/gltf+json",
        "zip" => "application/zip",
        _ => OCTET_STREAM,
    }
}

pub(crate) const OCTET_STREAM: &str = "application/octet-stream";

/// What a file is to the bundler: whether it is scanned for references, and how.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Html,
    Svg,
    Css,
    Js,
    Other,
}

impl Kind {
    pub(crate) fn of_type(media_type: &str) -> Kind {
        let essence = essence(media_type);
        match essence.as_str() {
            "text/html" | "application/xhtml+xml" => Kind::Html,
            "image/svg+xml" => Kind::Svg,
            "text/css" => Kind::Css,
            "text/javascript" | "application/javascript" | "application/x-javascript"
            | "text/ecmascript" | "application/ecmascript" | "text/jsx" | "text/babel" => Kind::Js,
            _ => Kind::Other,
        }
    }

    pub(crate) fn of_path(path: &str) -> Kind {
        if path.to_ascii_lowercase().ends_with(".svgz") {
            return Kind::Other;
        }
        Kind::of_type(media_type(path))
    }

    pub(crate) fn is_text(self) -> bool {
        self != Kind::Other
    }
}

/// The media type without its parameters, lower case ("text/css; charset=utf-8" ->
/// "text/css").
pub(crate) fn essence(media_type: &str) -> String {
    media_type.split(';').next().unwrap_or("").trim().to_ascii_lowercase()
}

/// Whether the file is HTML by its media type.
pub(crate) fn is_html_type(media_type: &str) -> bool {
    matches!(essence(media_type).as_str(), "text/html" | "application/xhtml+xml")
}

/// The media type a file of the page is served with: its ending's type, and for text
/// (HTML, CSS, JavaScript, plain text) without a byte order mark whose bytes are valid
/// UTF-8 `; charset=utf-8`, except HTML that declares its own charset in a `<meta>`
/// (WebKit then reads the page's declaration, as it does for a file that is not UTF-8).
pub(crate) fn served_type(path: &str, bytes: &[u8]) -> String {
    let base = media_type(path);
    let wants_charset = matches!(
        base,
        "text/html" | "text/css" | "text/javascript" | "text/plain" | "text/csv" | "text/vtt"
            | "text/tab-separated-values"
    );
    if !wants_charset || has_bom(bytes) || std::str::from_utf8(bytes).is_err() {
        return base.to_string();
    }
    if base == "text/html" && declares_charset(bytes) {
        return base.to_string();
    }
    format!("{base}; charset=utf-8")
}

fn has_bom(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xEF, 0xBB, 0xBF])
        || bytes.starts_with(&[0xFE, 0xFF])
        || bytes.starts_with(&[0xFF, 0xFE])
}

/// A `<meta charset>` or `<meta http-equiv="content-type" content="...; charset=...">`
/// within the first 4096 bytes (the window a browser's prescan reads, generously).
fn declares_charset(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(4096)];
    let lower: Vec<u8> = head.iter().map(|b| b.to_ascii_lowercase()).collect();
    let mut at = 0;
    while let Some(found) = find(&lower[at..], b"<meta") {
        let start = at + found;
        let end = find(&lower[start..], b">").map(|e| start + e).unwrap_or(lower.len());
        if find(&lower[start..end], b"charset").is_some() {
            return true;
        }
        at = end.max(start + 5);
        if at >= lower.len() {
            break;
        }
    }
    false
}

pub(crate) fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|window| window == needle)
}
