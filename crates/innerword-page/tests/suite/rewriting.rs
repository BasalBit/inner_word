//! Rewriting: the stored copy of a piece is named by a relative path from the file that
//! names it, so the served files also work as an exported folder.

use super::support::*;

const CSS: &str = "text/css";
const JS: &str = "text/javascript";

#[test]
fn relative_paths_from_nested_files() {
    let bundle = page(
        &[
            ("index.html", r#"<link rel="stylesheet" href="https://cdn.test/a.css"><a href="chapters/one/page.html">one</a>"#),
            ("chapters/one/page.html", r#"<img src="https://cdn.test/img/b.png"><link rel="stylesheet" href="../../styles/local.css">"#),
            ("styles/local.css", "body { background: url(https://cdn.test/img/bg.png) }"),
        ],
        &[
            ("https://cdn.test/a.css", CSS, "a{}"),
            ("https://cdn.test/img/b.png", "image/png", "b"),
            ("https://cdn.test/img/bg.png", "image/png", "bg"),
        ],
    );
    assert!(served(&bundle, "index.html").contains(r#"href="_external/cdn.test/a.css""#));
    let page_html = served(&bundle, "chapters/one/page.html");
    assert!(page_html.contains(r#"src="../../_external/cdn.test/img/b.png""#), "{page_html}");
    assert!(page_html.contains(r#"href="../../styles/local.css""#), "a local reference is left as it is");
    let local = served(&bundle, "styles/local.css");
    assert_eq!(local, "body { background: url(../_external/cdn.test/img/bg.png) }");
}

#[test]
fn import_chains_resolve_against_the_address_of_the_sheet() {
    let bundle = page(
        &[("index.html", r#"<link rel="stylesheet" href="https://cdn.test/css/main.css">"#)],
        &[
            ("https://cdn.test/css/main.css", CSS, r#"@import "parts/base.css"; .m { background: url(../img/m.png) }"#),
            ("https://cdn.test/css/parts/base.css", CSS, r#"@import url("/shared/reset.css"); .b{}"#),
            ("https://cdn.test/shared/reset.css", CSS, "* { margin: 0 }"),
            ("https://cdn.test/img/m.png", "image/png", "m"),
        ],
    );
    let main = served(&bundle, "_external/cdn.test/css/main.css");
    assert_eq!(main, r#"@import "parts/base.css"; .m { background: url(../img/m.png) }"#, "relative pieces stay relative");
    let base = served(&bundle, "_external/cdn.test/css/parts/base.css");
    assert_eq!(base, r#"@import url("../../shared/reset.css"); .b{}"#);
    assert_eq!(
        external_urls(&bundle),
        vec![
            "https://cdn.test/css/main.css",
            "https://cdn.test/css/parts/base.css",
            "https://cdn.test/img/m.png",
            "https://cdn.test/shared/reset.css",
        ]
    );
}

#[test]
fn an_es_module_graph_with_absolute_path_imports_on_a_cdn() {
    let bundle = page(
        &[(
            "index.html",
            r#"<script type="module">import React from "https://esm.test/react@18.3.1"; import { createRoot } from "https://esm.test/react-dom@18.3.1/client";</script>"#,
        )],
        &[
            (
                "https://esm.test/react@18.3.1",
                JS,
                r#"/* esm.sh - react@18.3.1 */
export * from "/react@18.3.1/es2022/react.mjs";
export { default } from "/react@18.3.1/es2022/react.mjs";
//# sourceMappingURL=react.mjs.map"#,
            ),
            ("https://esm.test/react@18.3.1/es2022/react.mjs", JS, "export default {}; export const version = '18.3.1';"),
            ("https://esm.test/react-dom@18.3.1/client", JS, r#"import "/react-dom@18.3.1/es2022/client.mjs"; export * from "/react-dom@18.3.1/es2022/client.mjs";"#),
            ("https://esm.test/react-dom@18.3.1/es2022/client.mjs", JS, r#"import "/react@18.3.1/es2022/react.mjs"; export const createRoot = 1;"#),
        ],
    );
    let index = served(&bundle, "index.html");
    assert!(index.contains(r#"import React from "./_external/esm.test/react@18.3.1.js""#), "{index}");
    assert!(index.contains(r#"from "./_external/esm.test/react-dom@18.3.1/client.js""#), "{index}");
    let entry = served(&bundle, "_external/esm.test/react@18.3.1.js");
    assert!(entry.contains(r#"export * from "./react@18.3.1/es2022/react.mjs";"#), "{entry}");
    assert!(!entry.contains("sourceMappingURL"));
    let client = served(&bundle, "_external/esm.test/react-dom@18.3.1/es2022/client.mjs");
    assert!(client.contains(r#"import "../../react@18.3.1/es2022/react.mjs";"#), "{client}");
    let file = bundle.served.iter().find(|f| f.path == "_external/esm.test/react@18.3.1.js").unwrap();
    assert_eq!(file.media_type, JS);
}

#[test]
fn base_href_resolves_every_relative_reference_and_goes() {
    let bundle = page(
        &[
            (
                "index.html",
                r#"<html><head><base href="https://cdn.test/app/"><link rel="stylesheet" href="style.css"></head><body><img src="img/a.png"><a href="about.html">about</a><a href="https://example.com/x">x</a></body></html>"#,
            ),
        ],
        &[("https://cdn.test/app/style.css", CSS, "s{}"), ("https://cdn.test/app/img/a.png", "image/png", "a")],
    );
    let index = served(&bundle, "index.html");
    assert_eq!(
        index,
        r#"<html><head><link rel="stylesheet" href="_external/cdn.test/app/style.css"></head><body><img src="_external/cdn.test/app/img/a.png"><a href="https://cdn.test/app/about.html">about</a><a href="https://example.com/x">x</a></body></html>"#
    );
}

#[test]
fn a_local_base_is_taken_out_and_references_are_relative_to_the_file() {
    let bundle = page(
        &[
            ("index.html", r#"<base href="assets/"><img src="a.png"><link rel="stylesheet" href="../top.css">"#),
            ("assets/a.png", "a"),
            ("top.css", "t{}"),
        ],
        &[],
    );
    assert_eq!(served(&bundle, "index.html"), r#"<img src="assets/a.png"><link rel="stylesheet" href="top.css">"#);
}

#[test]
fn names_of_stored_pieces_follow_archive_py() {
    let bundle = page(
        &[(
            "index.html",
            r#"<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Geist:wght@400..700&amp;family=Geist+Mono:wght@400..600&amp;display=swap">
<script src="https://cdn.test/lib"></script>
<link rel="stylesheet" href="https://cdn.test/styles.php">
<img src="https://cdn.test/dir/">
<img src="https://cdn.test/a%20b%C3%A9.png">
<img src="https://cdn.test:8443/port.png">"#,
        )],
        &[
            ("https://fonts.googleapis.com/css2?family=Geist:wght@400..700&family=Geist+Mono:wght@400..600&display=swap", "text/css; charset=utf-8", "x{}"),
            ("https://cdn.test/lib", "application/javascript", "1"),
            ("https://cdn.test/styles.php", "text/css", "y{}"),
            ("https://cdn.test/dir/", "image/png", "d"),
            ("https://cdn.test/a%20b%C3%A9.png", "image/png", "e"),
            ("https://cdn.test:8443/port.png", "image/png", "p"),
        ],
    );
    // As the archive of saved artifacts names it: the query's SHA-1, first 10 hex digits, before
    // the ending.
    assert_eq!(
        stored(&bundle, "https://fonts.googleapis.com/css2?family=Geist:wght@400..700&family=Geist+Mono:wght@400..600&display=swap"),
        "_external/fonts.googleapis.com/css2__a31d67e5de.css"
    );
    assert_eq!(stored(&bundle, "https://cdn.test/lib"), "_external/cdn.test/lib.js");
    assert_eq!(stored(&bundle, "https://cdn.test/styles.php"), "_external/cdn.test/styles.php.css");
    assert_eq!(stored(&bundle, "https://cdn.test/dir/"), "_external/cdn.test/dir/index");
    assert_eq!(stored(&bundle, "https://cdn.test/a%20b%C3%A9.png"), "_external/cdn.test/a_b_.png");
    assert_eq!(stored(&bundle, "https://cdn.test:8443/port.png"), "_external/cdn.test_8443/port.png");
}

#[test]
fn two_addresses_that_come_out_the_same_are_told_apart() {
    let bundle = page(
        &[("index.html", r#"<img src="https://cdn.test/a b.png"><img src="https://cdn.test/a_b.png">"#)],
        &[("https://cdn.test/a%20b.png", "image/png", "1"), ("https://cdn.test/a_b.png", "image/png", "2")],
    );
    let first = stored(&bundle, "https://cdn.test/a%20b.png");
    let second = stored(&bundle, "https://cdn.test/a_b.png");
    assert_ne!(first, second);
    assert!(first == "_external/cdn.test/a_b.png" || second == "_external/cdn.test/a_b.png");
    let other = if first == "_external/cdn.test/a_b.png" { &second } else { &first };
    assert!(other.starts_with("_external/cdn.test/a_b__") && other.ends_with(".png") && other.len() == "_external/cdn.test/a_b__12345678.png".len(), "{other}");
}

#[test]
fn a_file_of_the_page_is_never_overwritten_by_a_piece() {
    let bundle = page(
        &[("index.html", r#"<img src="https://cdn.test/x.png">"#), ("_external/cdn.test/x.png", "mine")],
        &[("https://cdn.test/x.png", "image/png", "theirs")],
    );
    assert_eq!(served(&bundle, "_external/cdn.test/x.png"), "mine");
    let path = stored(&bundle, "https://cdn.test/x.png");
    assert_ne!(path, "_external/cdn.test/x.png");
    assert_eq!(served(&bundle, &path), "theirs");
}

#[test]
fn a_cdn_string_in_a_script_file_is_named_from_the_root() {
    let bundle = page(
        &[
            ("index.html", r#"<script src="js/app.js"></script>"#),
            ("js/app.js", r#"const worker = "https://cdn.jsdelivr.net/npm/x@1/w.js"; import("https://cdn.test/m.js");"#),
        ],
        &[("https://cdn.jsdelivr.net/npm/x@1/w.js", JS, "1"), ("https://cdn.test/m.js", JS, "2")],
    );
    let app = served(&bundle, "js/app.js");
    assert!(app.contains(r#""/_external/cdn.jsdelivr.net/npm/x@1/w.js""#), "a plain string is resolved by the document: {app}");
    assert!(app.contains(r#"import("../_external/cdn.test/m.js")"#), "a specifier by the module: {app}");
}

#[test]
fn a_file_in_another_encoding_comes_out_byte_for_byte() {
    let dir = folder(&[]);
    // Latin-1 bytes (é is 0xE9) around a reference.
    let mut html = b"<meta charset=\"iso-8859-1\"><p>caf\xe9</p><img src=\"https://cdn.test/c.png\"><p>\xe9t\xe9</p>".to_vec();
    write(dir.path(), "index.html", &html);
    let input = innerword_page::read_input(dir.path(), None).unwrap();
    let bundle = innerword_page::bundle(input, &web(&[("https://cdn.test/c.png", "image/png", "c")])).unwrap();
    let file = bundle.served.iter().find(|f| f.path == "index.html").unwrap();
    let expected = {
        let at = html.windows(22).position(|w| w == b"https://cdn.test/c.png").unwrap();
        html.splice(at..at + 22, b"_external/cdn.test/c.png".iter().copied());
        html
    };
    assert_eq!(&file.bytes[..], &expected[..]);
    assert_eq!(file.media_type, "text/html", "a page that is not UTF-8 is served without a charset");
}

#[test]
fn utf8_html_without_a_declaration_is_served_as_utf8() {
    let bundle = page(&[("index.html", "<p>café</p>"), ("b.html", "<meta charset=utf-8><p>x</p>"), ("c.css", "a{}")], &[]);
    let types: Vec<(String, String)> = bundle.served.iter().map(|f| (f.path.clone(), f.media_type.clone())).collect();
    assert_eq!(
        types,
        vec![
            ("b.html".to_string(), "text/html".to_string()),
            ("c.css".to_string(), "text/css; charset=utf-8".to_string()),
            ("index.html".to_string(), "text/html; charset=utf-8".to_string()),
        ]
    );
}
