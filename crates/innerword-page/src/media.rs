//! Names of bytes and their media types.

use sha2::{Digest, Sha256};

/// First 16 bytes of the SHA-256, 32 lowercase hex digits:
/// `innerword_store::FileId::of(bytes).to_hex()`.
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

/// The media type the first bytes of a file declare, for the binary formats a page
/// loads (fonts, pictures, sound and film, PDF, WebAssembly, archives): what the file is,
/// whatever its name or a server says. None for text, and for bytes no signature names.
/// Every signature that is made of letters is held to the byte after it that no text has
/// (a TrueType font begins with "true" and then its table count, which begins with 0).
pub fn sniffed_type(bytes: &[u8]) -> Option<&'static str> {
    let at = |from: usize, sig: &[u8]| bytes.len() >= from + sig.len() && &bytes[from..from + sig.len()] == sig;
    let byte = |i: usize| bytes.get(i).copied();
    let found = if at(0, b"wOF2") {
        "font/woff2"
    } else if at(0, b"wOFF") {
        "font/woff"
    } else if at(0, b"OTTO") && byte(4) == Some(0) {
        "font/otf"
    } else if (at(0, &[0, 1, 0, 0]) || at(0, b"true")) && byte(4) == Some(0) {
        "font/ttf"
    } else if at(0, b"ttcf") && byte(4) == Some(0) {
        "font/collection"
    } else if bytes.len() >= 36 && at(34, b"LP") && byte(8).map(|b| b <= 2).unwrap_or(false) && byte(9) == Some(0) {
        // Embedded OpenType: the magic number 0x504C at offset 34, after the sizes and
        // a format version whose last two bytes are 0x0001, 0x0002 or 0x0000.
        "application/vnd.ms-fontobject"
    } else if at(0, b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if at(0, &[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if at(0, b"GIF87a") || at(0, b"GIF89a") {
        "image/gif"
    } else if at(0, b"RIFF") && at(8, b"WEBP") {
        "image/webp"
    } else if at(0, b"RIFF") && at(8, b"WAVE") {
        "audio/wav"
    } else if at(4, b"ftyp") && bytes.len() >= 12 {
        match &bytes[8..12] {
            b"avif" | b"avis" => "image/avif",
            b"heic" | b"heix" | b"hevc" | b"hevx" | b"heim" | b"heis" => "image/heic",
            b"mif1" | b"msf1" => "image/heif",
            b"M4A " | b"M4B " => "audio/mp4",
            b"qt  " => "video/quicktime",
            _ => "video/mp4",
        }
    } else if (at(0, &[0, 0, 1, 0]) || at(0, &[0, 0, 2, 0])) && bytes.len() >= 6 {
        "image/x-icon"
    } else if at(0, b"BM") && bytes.len() >= 14 && at(6, &[0, 0, 0, 0]) {
        "image/bmp"
    } else if at(0, b"II*\0") || at(0, b"MM\0*") {
        "image/tiff"
    } else if at(0, b"ID3") && byte(3).map(|v| (2..=4).contains(&v)).unwrap_or(false) {
        "audio/mpeg"
    } else if byte(0) == Some(0xFF) && matches!(byte(1), Some(0xFB | 0xFA | 0xF3 | 0xF2 | 0xE3 | 0xE2)) {
        "audio/mpeg"
    } else if byte(0) == Some(0xFF) && matches!(byte(1), Some(0xF1 | 0xF9)) {
        "audio/aac"
    } else if at(0, b"OggS") && byte(4) == Some(0) {
        "audio/ogg"
    } else if at(0, b"fLaC") && byte(4).map(|b| b & 0x7F == 0).unwrap_or(false) {
        "audio/flac"
    } else if at(0, b"MThd") && at(4, &[0, 0, 0, 6]) {
        "audio/midi"
    } else if at(0, &[0x1A, 0x45, 0xDF, 0xA3]) {
        "video/webm"
    } else if at(0, b"%PDF-") {
        "application/pdf"
    } else if at(0, b"\0asm") {
        "application/wasm"
    } else if at(0, b"glTF") && at(5, &[0, 0, 0]) {
        "model/gltf-binary"
    } else if at(0, b"PK\x03\x04") {
        "application/zip"
    } else {
        return None;
    };
    Some(found)
}

/// Whether bytes may be text: a byte order mark, or none of the bytes that no text has
/// (the "binary data bytes" of the WHATWG's MIME sniffing: controls other than tab,
/// line feed, form feed, carriage return and escape) in the first 1445 bytes.
pub(crate) fn looks_like_text(bytes: &[u8]) -> bool {
    if has_bom(bytes) {
        return true;
    }
    !bytes.iter().take(1445).any(|&b| matches!(b, 0x00..=0x08 | 0x0B | 0x0E..=0x1A | 0x1C..=0x1F))
}

/// Whether text begins as an HTML document does, by the patterns of the WHATWG's MIME
/// sniffing: after white space, `<!DOCTYPE HTML`, `<HTML`, `<HEAD`, `<BODY`, `<SCRIPT`,
/// `<IFRAME`, `<H1`, `<DIV`, `<FONT`, `<TABLE`, `<A`, `<STYLE`, `<TITLE`, `<B`, `<BR`,
/// `<P` or `<!--`, each followed by a space or `>`, in any case.
pub(crate) fn looks_like_html(bytes: &[u8]) -> bool {
    if !looks_like_text(bytes) {
        return false;
    }
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(bytes);
    let start = bytes.iter().position(|b| !matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0C)).unwrap_or(bytes.len());
    let rest = &bytes[start..];
    const TAGS: &[&[u8]] = &[
        b"<!doctype html", b"<html", b"<head", b"<body", b"<script", b"<iframe", b"<h1", b"<div", b"<font",
        b"<table", b"<a", b"<style", b"<title", b"<b", b"<br", b"<p", b"<!--",
    ];
    TAGS.iter().any(|tag| {
        rest.len() > tag.len()
            && rest[..tag.len()].eq_ignore_ascii_case(tag)
            && (matches!(rest[tag.len()], b' ' | b'>' | b'\t' | b'\n' | b'\r' | 0x0C) || *tag == b"<!--")
    })
}

/// Whether text is an SVG document: it begins (after white space, an XML declaration,
/// comments and a doctype) with `<svg`, or names `<svg` in its first 4096 bytes after an
/// XML declaration and is no HTML.
pub(crate) fn looks_like_svg(bytes: &[u8]) -> bool {
    if !looks_like_text(bytes) || looks_like_html(bytes) {
        return false;
    }
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(bytes);
    let head: Vec<u8> = bytes.iter().take(4096).map(u8::to_ascii_lowercase).collect();
    let start = head.iter().position(|b| !b.is_ascii_whitespace()).unwrap_or(head.len());
    let rest = &head[start..];
    (rest.starts_with(b"<svg") || rest.starts_with(b"<?xml") || rest.starts_with(b"<!--") || rest.starts_with(b"<!doctype svg"))
        && find(rest, b"<svg").is_some()
}

/// Whether a media type is one of text (the type of a file that is read as characters).
pub(crate) fn is_text_type(media_type: &str) -> bool {
    let essence = essence(media_type);
    essence.starts_with("text/")
        || essence.ends_with("+xml")
        || essence.ends_with("+json")
        || matches!(
            essence.as_str(),
            "application/xml" | "application/json" | "application/javascript" | "application/x-javascript"
                | "application/ecmascript" | "application/x-subrip"
        )
}

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

    /// What a file of the page is: by its ending, unless its bytes are of a binary
    /// format (a font named `.html` is a font, and is never read as a document).
    pub(crate) fn of_file(path: &str, bytes: &[u8]) -> Kind {
        if sniffed_type(bytes).is_some() {
            return Kind::Other;
        }
        Kind::of_path(path)
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

/// The media type a file of the page is served with: its ending's type (or what its
/// bytes are, when they are of a binary format: `sniffed_type`), and for text
/// (HTML, CSS, JavaScript, plain text) without a byte order mark whose bytes are valid
/// UTF-8 `; charset=utf-8`, except HTML that declares its own charset in a `<meta>`
/// (WebKit then reads the page's declaration, as it does for a file that is not UTF-8).
pub(crate) fn served_type(path: &str, bytes: &[u8]) -> String {
    // Bytes of a binary format are what their signature says, whatever their ending;
    // and bytes that are no text are never served as HTML.
    if let Some(found) = sniffed_type(bytes) {
        return found.to_string();
    }
    let base = match media_type(path) {
        "text/html" | "application/xhtml+xml" if !looks_like_text(bytes) => OCTET_STREAM,
        found => found,
    };
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

