//! Which bytes are kept as an SVG, and lone surrogates.

use innerword_board::{FileHex, export_dir, limits, sniff_svg, well_formed_json};

const NS: &str = "http://www.w3.org/2000/svg";

#[test]
fn an_svg_document_is_one_after_its_prolog() {
    let plain =
        format!(r#"<svg xmlns="{NS}" width="10" height="10"><rect width="10" height="10"/></svg>"#);
    assert!(sniff_svg(plain.as_bytes()));
    let full = format!(
        "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!-- made by hand -->\n<!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG 1.1//EN\" \"http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd\" [ <!ENTITY a \"b>\"> ]>\n\n<svg\n  viewBox='0 0 1 1'\n  xmlns='{NS}'>\n</svg>"
    );
    assert!(sniff_svg(full.as_bytes()));
    let prefixed = format!(r#"<s:svg xmlns:s="{NS}"><s:rect/></s:svg>"#);
    assert!(sniff_svg(prefixed.as_bytes()));
}

#[test]
fn what_is_not_an_svg_document_is_not_one() {
    // An HTML file, also one that holds an SVG or claims the type.
    let html = format!(r#"<!DOCTYPE html><html><body><svg xmlns="{NS}"></svg></body></html>"#);
    assert!(!sniff_svg(html.as_bytes()));
    // An svg root outside the SVG namespace, or bound to another prefix.
    assert!(!sniff_svg(b"<svg width=\"1\"></svg>"));
    assert!(!sniff_svg(
        format!(r#"<s:svg xmlns="{NS}"></s:svg>"#).as_bytes()
    ));
    assert!(!sniff_svg(b"<svgx xmlns=\"http://www.w3.org/2000/svg\"/>"));
    // A PNG, text, nothing, an unfinished comment.
    assert!(!sniff_svg(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR"));
    assert!(!sniff_svg(
        b"hello <svg xmlns=\"http://www.w3.org/2000/svg\"/>"
    ));
    assert!(!sniff_svg(b""));
    assert!(!sniff_svg(
        b"<!-- <svg xmlns=\"http://www.w3.org/2000/svg\"/>"
    ));
    // Not UTF-8.
    let mut latin = format!(r#"<svg xmlns="{NS}"><text>"#).into_bytes();
    latin.push(0xE9);
    latin.extend_from_slice(b"</text></svg>");
    assert!(!sniff_svg(&latin));
}

#[test]
fn an_svg_of_more_than_4_mb_is_not_kept_as_it_came() {
    let head = format!(r#"<svg xmlns="{NS}"><desc>"#);
    let tail = "</desc></svg>";
    let fill = limits::MAX_SVG_BYTES - head.len() - tail.len();
    let exactly = format!("{head}{}{tail}", "x".repeat(fill));
    assert_eq!(exactly.len(), limits::MAX_SVG_BYTES);
    assert!(sniff_svg(exactly.as_bytes()));
    let over = format!("{head}{}{tail}", "x".repeat(fill + 1));
    assert!(!sniff_svg(over.as_bytes()));
    let five = format!("{head}{}{tail}", "x".repeat(5 * 1024 * 1024));
    assert!(!sniff_svg(five.as_bytes()));
}

#[test]
fn well_formed_text_parses() {
    let lone = format!("{{\"t\":\"x{}y\"}}", ["\\", "ud83d"].concat());
    assert!(serde_json::from_str::<serde_json::Value>(&lone).is_err());
    let mended: serde_json::Value = serde_json::from_str(&well_formed_json(&lone)).unwrap();
    assert_eq!(mended["t"], "x\u{fffd}y");
}

#[test]
fn the_export_folder_holds_the_scene_and_the_pictures() {
    let dir = tempfile::tempdir().unwrap();
    let a = FileHex::parse("0123456789abcdef0123456789abcdef").unwrap();
    let b = FileHex::parse("ffffffffffffffffffffffffffffffff").unwrap();
    let mut files = vec![(a.clone(), b"one".to_vec()), (b.clone(), b"two".to_vec())].into_iter();
    export_dir::write(dir.path(), "{\"elements\":[]}", &mut files).unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.path().join(export_dir::SCENE_FILE)).unwrap(),
        "{\"elements\":[]}"
    );
    assert_eq!(
        std::fs::read(dir.path().join(export_dir::FILES_DIR).join(a.as_str())).unwrap(),
        b"one"
    );
    assert_eq!(
        std::fs::read(dir.path().join("_files").join(b.as_str())).unwrap(),
        b"two"
    );
    assert_eq!(export_dir::SCENE_FILE, "scene.json");
    // Without pictures, no folder of them.
    let empty = tempfile::tempdir().unwrap();
    export_dir::write(empty.path(), "{}", &mut std::iter::empty()).unwrap();
    assert!(!empty.path().join(export_dir::FILES_DIR).exists());
}

#[test]
fn a_file_id_is_32_hex_digits() {
    assert_eq!(
        FileHex::parse("0123456789ABCDEF0123456789abcdef")
            .unwrap()
            .as_str(),
        "0123456789abcdef0123456789abcdef"
    );
    assert!(FileHex::parse("0123456789abcdef0123456789abcde").is_none());
    assert!(FileHex::parse("0123456789abcdef0123456789abcdeg").is_none());
    let read: Result<FileHex, _> = serde_json::from_str("\"nothex\"");
    assert!(read.is_err());
    assert_eq!(innerword_board::EXCALIDRAW_VERSION, "0.18.1");
}
