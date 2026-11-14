//! The media type of a stored file is decided by what its bytes are, by where the page
//! names it and by its ending, and the server's Content-Type is believed only where those
//! say nothing against it: Google Fonts answers some `/l/font?kit=…` addresses with
//! `text/html; charset=utf-8` for WOFF2 bytes, and 0.8.0 stored those fonts as HTML
//! files of the page, which the check then loaded and read for text (the bytes of seven
//! fonts became 366,000 characters of the page's text). And a piece from the web is
//! never a document of the page.

use std::collections::HashMap;

use writer_page::{
    Bundle, MapFetcher, Manifest, ManifestFile, bundle, check_request, prepare, read_check_dir, read_input,
    sniffed_type, static_check, write_served_dir,
};

use super::support::*;

/// The first bytes of a WOFF2 file (its signature, a flavor, a length), then bytes no
/// text has and a few letters a font holds (the names of its glyphs), which the check of
/// 0.8.0 read as the text of a page.
pub fn woff2(seed: u8) -> Vec<u8> {
    let mut bytes = b"wOF2\x00\x01\x00\x00\x00\x00\x02\x00".to_vec();
    for i in 0..400u32 {
        bytes.push(((i * 37 + seed as u32) % 256) as u8);
    }
    bytes.extend_from_slice(b"Copyright The Font Project Authors Regular Bold Italic glyphs");
    bytes
}

fn type_of(bundle: &Bundle, path: &str) -> String {
    bundle.served.iter().find(|f| f.path == path).unwrap_or_else(|| panic!("{path} is not served")).media_type.clone()
}

fn bundle_with(files: &[(&str, &str)], answers: MapFetcher) -> Bundle {
    let dir = folder(files);
    bundle(read_input(dir.path(), None).unwrap(), &answers)
        .unwrap_or_else(|error| panic!("the page was refused: {}", error.sentence()))
}

const GOOGLE_CSS: &str = "https://fonts.googleapis.com/css2?family=Bricolage+Grotesque";
const KIT_HTML: &str = "https://fonts.gstatic.com/l/font?kit=abc&skey=1&v=v9";
const KIT_WOFF2: &str = "https://fonts.gstatic.com/l/font?kit=def&skey=1&v=v9";

fn google_page() -> Bundle {
    let css = format!(
        "@font-face {{ font-family: 'A'; src: url({KIT_HTML}) format('woff2'); }}\n@font-face {{ font-family: 'B'; src: url({KIT_WOFF2}) format('woff2'); }}"
    );
    bundle_with(
        &[(
            "index.html",
            &format!(r#"<html><head><title>Branding</title><link rel="stylesheet" href="{GOOGLE_CSS}"></head><body><h1>Branding</h1><p>Four words of text.</p></body></html>"#),
        )],
        MapFetcher::new()
            .with(GOOGLE_CSS, "text/css; charset=utf-8", css.into_bytes())
            .with(KIT_HTML, "text/html; charset=utf-8", woff2(1))
            .with(KIT_WOFF2, "font/woff2", woff2(2)),
    )
}

#[test]
fn a_font_the_server_calls_html_is_stored_as_a_font_and_is_no_document_of_the_page() {
    let bundle = google_page();
    let kit = stored(&bundle, KIT_HTML);
    assert_eq!(type_of(&bundle, &kit), "font/woff2", "the bytes are WOFF2, whatever the server said");
    assert_eq!(type_of(&bundle, &stored(&bundle, KIT_WOFF2)), "font/woff2");
    assert_eq!(bundle.html_paths(), vec!["index.html".to_string()]);
    assert!(check_request(&bundle).also.is_empty(), "the check loads no font as a page");
    let prepared = prepare(bundle.clone(), &static_check(&bundle)).unwrap();
    let manifest = prepared.manifest(None, 1);
    assert_eq!(manifest.text, "Branding\nFour words of text.");
    assert!(manifest.files.iter().all(|f| !f.media_type.starts_with("text/html") || f.path == "index.html"));
}

#[test]
fn bytes_of_a_binary_format_are_what_their_signature_says() {
    let png = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR".to_vec();
    let jpeg = b"\xFF\xD8\xFF\xE0\x00\x10JFIF\x00".to_vec();
    let mp3 = b"ID3\x04\x00\x00\x00\x00\x00\x00".to_vec();
    let ttf = b"\x00\x01\x00\x00\x00\x0c\x00\x80".to_vec();
    let otf = b"OTTO\x00\x0c\x00\x80".to_vec();
    let woff = b"wOFF\x00\x01\x00\x00".to_vec();
    let bundle = bundle_with(
        &[(
            "index.html",
            r#"<img src="https://cdn.test/a"><img src="https://cdn.test/b.png"><audio src="https://cdn.test/c.html"></audio>
<style>@font-face{src:url(https://cdn.test/d)} @font-face{src:url("https://cdn.test/e.css")} @font-face{src:url(https://cdn.test/f)}</style>"#,
        )],
        MapFetcher::new()
            .with("https://cdn.test/a", "text/html", png.clone())
            .with("https://cdn.test/b.png", "image/png", jpeg)
            .with("https://cdn.test/c.html", "text/html; charset=utf-8", mp3)
            .with("https://cdn.test/d", "text/plain", ttf)
            .with("https://cdn.test/e.css", "text/css", otf)
            .with("https://cdn.test/f", "", woff),
    );
    let types: HashMap<&str, &str> = [
        ("https://cdn.test/a", "image/png"),
        ("https://cdn.test/b.png", "image/jpeg"),
        ("https://cdn.test/c.html", "audio/mpeg"),
        ("https://cdn.test/d", "font/ttf"),
        ("https://cdn.test/e.css", "font/otf"),
        ("https://cdn.test/f", "font/woff"),
    ]
    .into_iter()
    .collect();
    for (url, media) in types {
        assert_eq!(type_of(&bundle, &stored(&bundle, url)), media, "{url}");
    }
    assert_eq!(bundle.html_paths(), vec!["index.html".to_string()]);
}

#[test]
fn a_font_without_a_signature_is_never_text() {
    // A url() of @font-face answered "text/html" with bytes no signature names, text or
    // not: never HTML, never scanned.
    let bundle = bundle_with(
        &[("index.html", r#"<style>@font-face{src:url(https://cdn.test/kit?a)} @font-face{src:url(https://cdn.test/kit?b)}</style><link rel="preload" as="font" href="https://cdn.test/kit?c">"#)],
        MapFetcher::new()
            .with("https://cdn.test/kit?a", "text/html; charset=utf-8", b"<p>looks like a page</p><img src=https://cdn.test/never.png>".to_vec())
            .with("https://cdn.test/kit?b", "text/html", vec![0x00, 0x13, 0x77, 0x01, 0x02])
            .with("https://cdn.test/kit?c", "font/woff2", b"plain letters".to_vec()),
    );
    assert_eq!(type_of(&bundle, &stored(&bundle, "https://cdn.test/kit?a")), "application/octet-stream");
    assert_eq!(type_of(&bundle, &stored(&bundle, "https://cdn.test/kit?b")), "application/octet-stream");
    assert_eq!(type_of(&bundle, &stored(&bundle, "https://cdn.test/kit?c")), "font/woff2");
    assert!(!external_urls(&bundle).contains(&"https://cdn.test/never.png".to_string()), "a font is not scanned");
    assert_eq!(bundle.html_paths(), vec!["index.html".to_string()]);
}

#[test]
fn bytes_that_are_no_text_are_never_stored_as_html() {
    // An image answered "text/html" with bytes no signature names.
    let bundle = bundle_with(
        &[("index.html", r#"<img src="https://cdn.test/picture"><img src="https://cdn.test/other.webp">"#)],
        MapFetcher::new()
            .with("https://cdn.test/picture", "text/html; charset=utf-8", vec![0x00, 0x01, 0x02, 0x03, 0xFE])
            .with("https://cdn.test/other.webp", "text/html", vec![0x00, 0x01, 0x02, 0x03]),
    );
    assert_eq!(type_of(&bundle, &stored(&bundle, "https://cdn.test/picture")), "application/octet-stream");
    assert_eq!(type_of(&bundle, &stored(&bundle, "https://cdn.test/other.webp")), "image/webp", "the ending");
}

#[test]
fn a_script_or_a_style_sheet_the_server_calls_html_is_what_the_page_names() {
    let bundle = bundle_with(
        &[("index.html", r#"<link rel="stylesheet" href="https://cdn.test/style"><script src="https://cdn.test/lib"></script>"#)],
        MapFetcher::new()
            .with("https://cdn.test/style", "text/html", b"body{background:url(https://cdn.test/bg.png)}".to_vec())
            .with("https://cdn.test/lib", "text/html; charset=utf-8", b"import('https://cdn.test/more.js'); var a = 1;".to_vec())
            .with("https://cdn.test/bg.png", "image/png", b"\x89PNG\r\n\x1a\n".to_vec())
            .with("https://cdn.test/more.js", "text/javascript", b"export {}".to_vec()),
    );
    let style = stored(&bundle, "https://cdn.test/style");
    assert_eq!(type_of(&bundle, &style), "text/css");
    assert!(served(&bundle, &style).contains("url(bg.png)"), "{}", served(&bundle, &style));
    assert_eq!(type_of(&bundle, &stored(&bundle, "https://cdn.test/lib")), "text/javascript");
    assert!(external_urls(&bundle).contains(&"https://cdn.test/more.js".to_string()), "the script was scanned");
    assert_eq!(bundle.html_paths(), vec!["index.html".to_string()]);
}

#[test]
fn a_web_page_fetched_as_a_piece_is_kept_as_html_and_is_no_document_of_the_page() {
    let bundle = bundle_with(
        &[("index.html", r#"<p>The page.</p><object data="https://cdn.test/widget.html"></object>"#)],
        MapFetcher::new().with(
            "https://cdn.test/widget.html",
            "text/html; charset=utf-8",
            b"<!DOCTYPE html><title>Widget</title><p>A widget with many words of its own.</p>".to_vec(),
        ),
    );
    let widget = stored(&bundle, "https://cdn.test/widget.html");
    assert_eq!(type_of(&bundle, &widget), "text/html; charset=utf-8");
    assert_eq!(bundle.html_paths(), vec!["index.html".to_string()]);
    let report = static_check(&bundle);
    assert_eq!(report.loads.len(), 1);
    assert_eq!(report.loads[0].text, "The page.");
    // Text a server calls HTML that does not begin as HTML is plain text.
    let bundle = bundle_with(
        &[("index.html", r#"<object data="https://cdn.test/notes"></object>"#)],
        MapFetcher::new().with("https://cdn.test/notes", "text/html", b"just some words".to_vec()),
    );
    assert_eq!(type_of(&bundle, &stored(&bundle, "https://cdn.test/notes")), "text/plain");
}

#[test]
fn a_file_of_the_page_named_html_whose_bytes_are_a_font_is_a_font() {
    let dir = temp();
    write(dir.path(), "index.html", r#"<p>The page.</p><a href="fonts/a.html">a</a>"#);
    write(dir.path(), "fonts/a.html", woff2(3));
    write(dir.path(), "notes.html", "<p>Notes.</p>");
    let bundle = bundle(read_input(dir.path(), None).unwrap(), &MapFetcher::new()).unwrap();
    assert_eq!(type_of(&bundle, "fonts/a.html"), "font/woff2");
    assert_eq!(bundle.html_paths(), vec!["index.html".to_string(), "notes.html".to_string()]);
}

#[test]
fn a_page_stored_with_a_font_typed_html_is_not_loaded_as_a_document_for_a_pdf() {
    // What 0.8.0 stored: a piece from the web typed text/html whose bytes are a font.
    let bundle = google_page();
    let prepared = prepare(bundle.clone(), &static_check(&bundle)).unwrap();
    let mut manifest: Manifest = prepared.manifest(None, 1);
    let blobs: HashMap<String, Vec<u8>> = prepared.blobs.iter().map(|b| (b.file.clone(), b.data.clone())).collect();
    let kit = stored(&bundle, KIT_HTML);
    for file in manifest.files.iter_mut().filter(|f| f.path == kit) {
        file.media_type = "text/html; charset=utf-8".to_string();
    }
    // And a local document of the page beside it, which is one.
    let notes = b"<p>Notes.</p>".to_vec();
    let notes_id = writer_page::file_id_hex(&notes);
    manifest.files.push(ManifestFile {
        path: "notes.html".to_string(),
        file: notes_id.clone(),
        media_type: "text/html".to_string(),
        size: notes.len() as u64,
    });
    let mut blobs = blobs;
    blobs.insert(notes_id, notes);
    let dir = temp();
    let out = dir.path().join("pdf");
    write_served_dir(&manifest, &out, &mut |id| Ok(blobs.get(id).cloned())).unwrap();
    let (request, _) = read_check_dir(&out).unwrap();
    assert_eq!(request.also, vec!["notes.html".to_string()]);
}

#[test]
fn signatures_and_the_text_that_begins_like_one() {
    assert_eq!(sniffed_type(b"wOF2\x00\x01\x00\x00"), Some("font/woff2"));
    assert_eq!(sniffed_type(b"true\x00\x0c\x00\x80"), Some("font/ttf"));
    assert_eq!(sniffed_type(b"OTTO\x00\x0c"), Some("font/otf"));
    assert_eq!(sniffed_type(b"ID3\x03\x00"), Some("audio/mpeg"));
    assert_eq!(sniffed_type(b"\x00\x00\x00\x20ftypisom\x00\x00\x02\x00"), Some("video/mp4"));
    assert_eq!(sniffed_type(b"\x00\x00\x00\x1cftypavif"), Some("image/avif"));
    // Text that begins with the letters of a signature is text.
    for text in [&b"true || false"[..], b"OTTO the cat", b"ID3 tags", b"OggS and more", b"BM 2024 notes", b"glTF is JSON", b"MThd", b"<p>x</p>"] {
        assert_eq!(sniffed_type(text), None, "{}", String::from_utf8_lossy(text));
    }
}

#[test]
fn what_begins_as_html_or_svg_from_the_web() {
    // Every answer is called text/html by its server; only what begins as HTML stays so.
    let answers = [
        ("https://cdn.test/1", &b"  <!DOCTYPE html><p>x"[..], "text/html"),
        ("https://cdn.test/2", b"\xEF\xBB\xBF<html lang=en>", "text/html"),
        ("https://cdn.test/3", b"<!-- a comment --><div>", "text/html"),
        ("https://cdn.test/4", b"just words <p>", "text/plain"),
        ("https://cdn.test/5", b"<?xml version='1.0'?>\n<svg xmlns='http://www.w3.org/2000/svg'/>", "image/svg+xml"),
        ("https://cdn.test/6", b"\xFF\xFE<\x00p\x00>\x00", "text/plain"),
        ("https://cdn.test/7", b"abc\x00def", "application/octet-stream"),
    ];
    let mut html = String::new();
    let mut fetcher = MapFetcher::new();
    for (url, bytes, _) in &answers {
        html.push_str(&format!(r#"<object data="{url}"></object>"#));
        fetcher = fetcher.with(url, "text/html", bytes.to_vec());
    }
    let bundle = bundle_with(&[("index.html", &html)], fetcher);
    for (url, _, media) in answers {
        assert_eq!(type_of(&bundle, &stored(&bundle, url)), media, "{url}");
    }
    assert_eq!(bundle.html_paths(), vec!["index.html".to_string()]);
}

#[test]
fn a_file_of_the_page_is_served_as_what_its_bytes_are() {
    let dir = temp();
    write(dir.path(), "index.html", r#"<img src="a.png"><link rel=preload as=font href="kit"><a href="b.html">b</a><a href="c.html">c</a>"#);
    write(dir.path(), "a.png", b"\xFF\xD8\xFF\xE0\x00");
    write(dir.path(), "kit", b"wOF2\x00\x01");
    write(dir.path(), "b.html", b"\x00\x01\x02 bytes");
    write(dir.path(), "c.html", "<p>c</p>");
    let bundle = bundle(read_input(dir.path(), None).unwrap(), &MapFetcher::new()).unwrap();
    assert_eq!(type_of(&bundle, "a.png"), "image/jpeg");
    assert_eq!(type_of(&bundle, "kit"), "font/woff2");
    assert_eq!(type_of(&bundle, "b.html"), "application/octet-stream", "no text is never HTML");
    assert_eq!(type_of(&bundle, "c.html"), "text/html; charset=utf-8");
    assert_eq!(bundle.html_paths(), vec!["index.html".to_string(), "c.html".to_string()]);
}
