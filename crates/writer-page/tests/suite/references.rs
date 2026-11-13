//! Finding every reference: each construct of the table in section 3 of the design,
//! written as text, names a piece on the web that is fetched and rewritten to its
//! stored copy.

use super::support::*;

const CSS: &str = "text/css; charset=utf-8";
const JS: &str = "application/javascript; charset=utf-8";
const PNG: &str = "image/png";

/// `html` as index.html, with every address in `answers` on the web; each one must be
/// fetched and no longer named in the served index.html, and its stored path must be.
fn holds(html: &str, answers: &[(&str, &str, &str)]) -> writer_page::Bundle {
    let bundle = page(&[("index.html", html)], answers);
    let index = served(&bundle, "index.html");
    for (url, _, _) in answers {
        let path = stored(&bundle, url);
        assert!(index.contains(&path) || bundle.served.iter().any(|f| f.path != "index.html" && String::from_utf8_lossy(&f.bytes).contains(&path_tail(&path))),
            "{path} is not named in the page:\n{index}");
        assert!(!index.contains(*url), "{url} is still named in the page:\n{index}");
    }
    bundle
}

fn path_tail(path: &str) -> String {
    path.rsplit('/').next().unwrap().to_string()
}

#[test]
fn script_link_img_and_media_attributes() {
    let bundle = holds(
        r#"<!doctype html><html><head>
<script src="https://cdn.test/app.js"></script>
<link rel="stylesheet" href="https://cdn.test/site.css">
</head><body>
<img src="https://cdn.test/a.png" alt="a">
<img src=https://cdn.test/unquoted.png>
<IMG SRC='https://cdn.test/upper.png'>
<video poster="https://cdn.test/poster.png" src="https://cdn.test/v.mp4"><track src="https://cdn.test/t.vtt"><source src="https://cdn.test/v2.webm"></video>
<audio src="https://cdn.test/s.mp3"></audio>
<embed src="https://cdn.test/e.swf">
<object data="https://cdn.test/o.pdf"></object>
<input type="image" src="https://cdn.test/button.png">
</body></html>"#,
        &[
            ("https://cdn.test/app.js", JS, "console.log(1)"),
            ("https://cdn.test/site.css", CSS, "body{}"),
            ("https://cdn.test/a.png", PNG, "png"),
            ("https://cdn.test/unquoted.png", PNG, "png2"),
            ("https://cdn.test/upper.png", PNG, "png3"),
            ("https://cdn.test/poster.png", PNG, "png4"),
            ("https://cdn.test/v.mp4", "video/mp4", "mp4"),
            ("https://cdn.test/t.vtt", "text/vtt", "WEBVTT"),
            ("https://cdn.test/v2.webm", "video/webm", "webm"),
            ("https://cdn.test/s.mp3", "audio/mpeg", "mp3"),
            ("https://cdn.test/e.swf", "application/x-shockwave-flash", "swf"),
            ("https://cdn.test/o.pdf", "application/pdf", "%PDF"),
            ("https://cdn.test/button.png", PNG, "png5"),
        ],
    );
    let index = served(&bundle, "index.html");
    assert!(index.contains(r#"<script src="_external/cdn.test/app.js"></script>"#), "{index}");
    assert!(index.contains(r#"<img src="_external/cdn.test/unquoted.png">"#), "{index}");
    assert!(index.contains(r#"<IMG SRC="_external/cdn.test/upper.png">"#), "{index}");
}

#[test]
fn srcset_and_imagesrcset_name_each_candidate() {
    let bundle = holds(
        r#"<picture><source srcset="https://cdn.test/a-1x.webp 1x, https://cdn.test/a-2x.webp 2x" type="image/webp">
<img srcset="https://cdn.test/b-480.png 480w,https://cdn.test/b-800.png 800w" sizes="(max-width: 600px) 480px, 800px" src="https://cdn.test/b.png"></picture>
<link rel="preload" as="image" imagesrcset="https://cdn.test/c.png 1x, https://cdn.test/c2.png 2x">"#,
        &[
            ("https://cdn.test/a-1x.webp", "image/webp", "1"),
            ("https://cdn.test/a-2x.webp", "image/webp", "2"),
            ("https://cdn.test/b-480.png", PNG, "3"),
            ("https://cdn.test/b-800.png", PNG, "4"),
            ("https://cdn.test/b.png", PNG, "5"),
            ("https://cdn.test/c.png", PNG, "6"),
            ("https://cdn.test/c2.png", PNG, "7"),
        ],
    );
    let index = served(&bundle, "index.html");
    assert!(
        index.contains(r#"srcset="_external/cdn.test/a-1x.webp 1x, _external/cdn.test/a-2x.webp 2x""#),
        "{index}"
    );
    assert!(index.contains(r#"srcset="_external/cdn.test/b-480.png 480w,_external/cdn.test/b-800.png 800w""#), "{index}");
    assert!(index.contains(r#"sizes="(max-width: 600px) 480px, 800px""#), "{index}");
}

#[test]
fn every_link_rel_that_loads_something() {
    holds(
        r#"<link rel="icon" href="https://cdn.test/favicon.ico">
<link rel="shortcut icon" href="https://cdn.test/f2.ico">
<link rel="apple-touch-icon" href="https://cdn.test/touch.png">
<link rel="preload" as="font" href="https://cdn.test/f.woff2" crossorigin>
<link rel="modulepreload" href="https://cdn.test/m.js">
<link rel="manifest" href="https://cdn.test/site.webmanifest">
<link rel="prefetch" href="https://cdn.test/next.png">"#,
        &[
            ("https://cdn.test/favicon.ico", "image/x-icon", "ico"),
            ("https://cdn.test/f2.ico", "image/x-icon", "ico2"),
            ("https://cdn.test/touch.png", PNG, "touch"),
            ("https://cdn.test/f.woff2", "font/woff2", "woff2"),
            ("https://cdn.test/m.js", JS, "export {}"),
            ("https://cdn.test/site.webmanifest", "application/manifest+json", "{}"),
            ("https://cdn.test/next.png", PNG, "next"),
        ],
    );
}

#[test]
fn preconnect_dns_prefetch_and_prerender_to_another_origin_are_removed() {
    let bundle = page(
        &[(
            "index.html",
            r#"<head><link rel="preconnect" href="https://fonts.googleapis.com"><link rel="preconnect" href="https://fonts.gstatic.com" crossorigin><link rel="dns-prefetch" href="//cdn.test"><link rel="prerender" href="https://example.com/next"><link rel="canonical" href="https://example.com/page"></head>"#,
        )],
        &[],
    );
    let index = served(&bundle, "index.html");
    assert_eq!(index, r#"<head><link rel="canonical" href="https://example.com/page"></head>"#);
    assert!(bundle.externals.is_empty());
}

#[test]
fn style_sheets_inline_in_attributes_and_in_files() {
    let bundle = holds(
        r#"<style>
@import "https://cdn.test/one.css";
@import url(https://cdn.test/two.css) screen;
@import url('https://cdn.test/three.css');
/* url(https://cdn.test/commented.png) is not loaded */
body { background: url(https://cdn.test/bg.png) no-repeat; }
.a { background-image: url( "https://cdn.test/quoted.png" ); }
.b { background-image: image-set("https://cdn.test/set-1x.png" 1x, url(https://cdn.test/set-2x.png) 2x); }
.c { background-image: -webkit-image-set('https://cdn.test/wk.png' 1x); }
@font-face { font-family: X; src: local("X"), url(https://cdn.test/x.woff2) format("woff2"); }
</style>
<div style="background: url(&quot;https://cdn.test/attr.png&quot;)">x</div>"#,
        &[
            ("https://cdn.test/one.css", CSS, "a{}"),
            ("https://cdn.test/two.css", CSS, "b{}"),
            ("https://cdn.test/three.css", CSS, "c{}"),
            ("https://cdn.test/bg.png", PNG, "1"),
            ("https://cdn.test/quoted.png", PNG, "2"),
            ("https://cdn.test/set-1x.png", PNG, "3"),
            ("https://cdn.test/set-2x.png", PNG, "4"),
            ("https://cdn.test/wk.png", PNG, "5"),
            ("https://cdn.test/x.woff2", "font/woff2", "6"),
            ("https://cdn.test/attr.png", PNG, "7"),
        ],
    );
    let index = served(&bundle, "index.html");
    assert!(index.contains("url(https://cdn.test/commented.png) is not loaded"), "a comment is left as it is");
    assert!(index.contains(r#"<div style="background: url(&quot;_external/cdn.test/attr.png&quot;)">"#), "{index}");
    assert!(index.contains("@import \"_external/cdn.test/one.css\";"), "{index}");
    assert!(index.contains("@import url(_external/cdn.test/two.css) screen;"), "{index}");
    assert!(index.contains("local(\"X\")"), "{index}");
    assert!(!external_urls(&bundle).iter().any(|u| u.contains("commented")));
}

#[test]
fn module_imports_workers_and_import_scripts() {
    let bundle = holds(
        r#"<script type="module">
import { a } from "https://cdn.test/a.js";
import "https://cdn.test/side-effect.js";
export { b } from 'https://cdn.test/b.js';
export * from "https://cdn.test/star.js";
const lazy = await import("https://cdn.test/lazy.js");
const tick = await import(`https://cdn.test/tick.js`);
const w = new Worker("https://cdn.test/worker.js", { type: "module" });
const s = new SharedWorker('https://cdn.test/shared.js');
// import "https://cdn.test/commented.js";
/* import "https://cdn.test/also-commented.js"; */
const re = /["'`]import "https:\/\/cdn.test\/regex.js"/;
</script>
<script>importScripts("https://cdn.test/s1.js", "https://cdn.test/s2.js");</script>"#,
        &[
            ("https://cdn.test/a.js", JS, "export const a = 1;"),
            ("https://cdn.test/side-effect.js", JS, "1;"),
            ("https://cdn.test/b.js", JS, "export const b = 2;"),
            ("https://cdn.test/star.js", JS, "export const c = 3;"),
            ("https://cdn.test/lazy.js", JS, "export default 4;"),
            ("https://cdn.test/tick.js", JS, "export default 5;"),
            ("https://cdn.test/worker.js", JS, "onmessage = () => {};"),
            ("https://cdn.test/shared.js", JS, "onconnect = () => {};"),
            ("https://cdn.test/s1.js", JS, "1"),
            ("https://cdn.test/s2.js", JS, "2"),
        ],
    );
    let index = served(&bundle, "index.html");
    assert!(index.contains(r#"import { a } from "./_external/cdn.test/a.js";"#), "a specifier keeps ./: {index}");
    assert!(index.contains("import(`./_external/cdn.test/tick.js`)"), "{index}");
    assert!(index.contains(r#"new Worker("_external/cdn.test/worker.js""#), "{index}");
    for gone in ["commented.js", "also-commented.js", "regex.js"] {
        assert!(!external_urls(&bundle).iter().any(|u| u.contains(gone)), "{gone} was fetched");
    }
}

#[test]
fn a_dynamic_import_with_a_substitution_is_not_a_reference() {
    let bundle = page(&[("index.html", "<script type=module>const m = await import(`https://cdn.test/${name}.js`);</script>")], &[]);
    assert!(bundle.externals.is_empty());
}

#[test]
fn cdn_addresses_in_string_literals_of_scripts() {
    let bundle = holds(
        r#"<script>
window.PDFJS_WORKER = "https://cdnjs.cloudflare.com/ajax/libs/pdf.js/3.11.174/pdf.worker.min.js";
const font = 'https://fonts.gstatic.com/s/inter/v1/a.woff2';
const other = "https://api.example.com/data.json";
</script>"#,
        &[
            ("https://cdnjs.cloudflare.com/ajax/libs/pdf.js/3.11.174/pdf.worker.min.js", JS, "self.x=1"),
            ("https://fonts.gstatic.com/s/inter/v1/a.woff2", "font/woff2", "woff2"),
        ],
    );
    let index = served(&bundle, "index.html");
    assert!(index.contains("https://api.example.com/data.json"), "an address not on a known CDN is left");
    assert!(!external_urls(&bundle).iter().any(|u| u.contains("api.example.com")));
}

#[test]
fn import_maps_with_imports_and_scopes() {
    let bundle = page(
        &[(
            "index.html",
            r#"<script type="importmap">
{
  "imports": {
    "three": "https://cdn.test/three@0.160.0/build/three.module.js",
    "three/addons/": "https://cdn.test/three@0.160.0/examples/jsm/"
  },
  "scopes": {
    "https://cdn.test/three@0.160.0/": {
      "helper": "https://cdn.test/helper@1/helper.js"
    }
  }
}
</script>
<script type="module">
import * as THREE from "three";
import { OrbitControls } from "three/addons/controls/OrbitControls.js";
</script>"#,
        )],
        &[
            ("https://cdn.test/three@0.160.0/build/three.module.js", JS, "import 'helper'; export const T = 1;"),
            ("https://cdn.test/three@0.160.0/examples/jsm/controls/OrbitControls.js", JS, "import { T } from 'three'; export const OrbitControls = T;"),
            ("https://cdn.test/helper@1/helper.js", JS, "export const h = 1;"),
        ],
    );
    let index = served(&bundle, "index.html");
    assert!(index.contains(r#""three": "./_external/cdn.test/three@0.160.0/build/three.module.js""#), "{index}");
    assert!(index.contains(r#""three/addons/": "./_external/cdn.test/three@0.160.0/examples/jsm/""#), "{index}");
    assert!(index.contains(r#""./_external/cdn.test/three@0.160.0/": {"#), "the scope is the folder of its pieces: {index}");
    assert!(index.contains(r#""helper": "./_external/cdn.test/helper@1/helper.js""#), "{index}");
    assert!(index.contains(r#"import * as THREE from "three";"#), "a bare specifier stays bare");
    // The module the prefix maps is fetched and stored where the prefix now points.
    assert_eq!(
        stored(&bundle, "https://cdn.test/three@0.160.0/examples/jsm/controls/OrbitControls.js"),
        "_external/cdn.test/three@0.160.0/examples/jsm/controls/OrbitControls.js"
    );
}

#[test]
fn svg_references_inline_and_in_a_file() {
    let bundle = page(
        &[
            (
                "index.html",
                r##"<svg><use xlink:href="https://cdn.test/sprite.svg#icon"></use><use href="#local"></use><image href="https://cdn.test/photo.jpg"/></svg><img src="drawing.svg">"##,
            ),
            (
                "drawing.svg",
                r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"><image xlink:href="https://cdn.test/inside.png"/><style>@import url(https://cdn.test/svg.css);</style></svg>"#,
            ),
        ],
        &[
            ("https://cdn.test/sprite.svg", "image/svg+xml", "<svg/>"),
            ("https://cdn.test/photo.jpg", "image/jpeg", "jpg"),
            ("https://cdn.test/inside.png", PNG, "png"),
            ("https://cdn.test/svg.css", CSS, "x{}"),
        ],
    );
    let index = served(&bundle, "index.html");
    assert!(index.contains(r#"xlink:href="_external/cdn.test/sprite.svg#icon""#), "the fragment is kept: {index}");
    assert!(index.contains(r##"<use href="#local">"##));
    let drawing = served(&bundle, "drawing.svg");
    assert!(drawing.contains(r#"xlink:href="_external/cdn.test/inside.png""#), "{drawing}");
    assert!(drawing.contains("url(_external/cdn.test/svg.css)"), "{drawing}");
    assert!(drawing.contains(r#"xmlns="http://www.w3.org/2000/svg""#));
}

#[test]
fn srcdoc_is_html_written_back_escaped() {
    let bundle = holds(
        r#"<iframe srcdoc="<p>Hi</p><img src=&quot;https://cdn.test/in-srcdoc.png&quot;>"></iframe>"#,
        &[("https://cdn.test/in-srcdoc.png", PNG, "png")],
    );
    let index = served(&bundle, "index.html");
    assert!(
        index.contains(r#"srcdoc="<p>Hi</p><img src=&quot;_external/cdn.test/in-srcdoc.png&quot;>""#),
        "{index}"
    );
}

#[test]
fn character_references_are_decoded_for_reading() {
    let bundle = page(
        &[(
            "index.html",
            r#"<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Inter:wght@400;700&amp;display=swap">"#,
        )],
        &[(
            "https://fonts.googleapis.com/css2?family=Inter:wght@400;700&display=swap",
            CSS,
            "@font-face{src:url(https://fonts.gstatic.com/s/inter/v13/x.woff2) format('woff2')}",
        ), ("https://fonts.gstatic.com/s/inter/v13/x.woff2", "font/woff2", "woff2")],
    );
    let path = stored(&bundle, "https://fonts.googleapis.com/css2?family=Inter:wght@400;700&display=swap");
    assert!(path.starts_with("_external/fonts.googleapis.com/css2__") && path.ends_with(".css"), "{path}");
    let css = served(&bundle, &path);
    assert!(css.contains("url(../fonts.gstatic.com/s/inter/v13/x.woff2)"), "{css}");
}

#[test]
fn what_is_not_a_reference() {
    // An empty web: anything fetched would be a refusal.
    let bundle = page(
        &[(
            "index.html",
            r##"<!-- <script src="https://cdn.test/commented.js"></script> -->
<img src="data:image/png;base64,AAAA">
<a href="mailto:a@example.com">mail</a> <a href="tel:+1">call</a> <a href="javascript:void(0)">js</a>
<a href="#top">top</a> <iframe src="about:blank"></iframe>
<script>const u = URL.createObjectURL(new Blob([])); const b = "blob:https://x/1";</script>
<a href="https://example.com/elsewhere">a link to another site</a>
<form action="#"></form>"##,
        )],
        &[],
    );
    assert!(bundle.externals.is_empty());
    let index = served(&bundle, "index.html");
    assert!(index.contains(r#"<a href="https://example.com/elsewhere">"#), "a link is left as it is");
}

#[test]
fn an_iframe_of_another_site_is_left_for_the_check() {
    let bundle = page(&[("index.html", r#"<iframe src="https://www.youtube.com/embed/x"></iframe>"#)], &[]);
    assert!(bundle.externals.is_empty());
    assert!(served(&bundle, "index.html").contains("https://www.youtube.com/embed/x"));
}

#[test]
fn a_protocol_relative_reference_is_https() {
    let bundle = page(
        &[("index.html", r#"<script src="//cdn.test/p.js"></script><a href="//example.com/x">x</a>"#)],
        &[("https://cdn.test/p.js", JS, "1")],
    );
    let index = served(&bundle, "index.html");
    assert!(index.contains(r#"<script src="_external/cdn.test/p.js">"#), "{index}");
    assert!(index.contains(r#"<a href="https://example.com/x">"#), "a protocol-relative link points where it pointed: {index}");
}

#[test]
fn a_refresh_to_a_file_of_the_page_and_to_the_web() {
    let bundle = page(
        &[("index.html", r#"<meta http-equiv="refresh" content="5; url=https://example.com/next">"#)],
        &[],
    );
    assert!(bundle.externals.is_empty());
    assert!(
        bundle.warnings.iter().any(|w| w.contains("https://example.com/next")),
        "a refresh to the web is said: {:?}",
        bundle.warnings
    );
}

#[test]
fn integrity_goes_with_a_reference_to_a_stored_copy() {
    let bundle = page(
        &[(
            "index.html",
            r#"<script src="https://cdn.test/lib.js" integrity="sha384-abc" crossorigin="anonymous"></script>"#,
        )],
        &[("https://cdn.test/lib.js", JS, "var x = 1;\n//# sourceMappingURL=lib.js.map\n")],
    );
    let index = served(&bundle, "index.html");
    assert_eq!(index, r#"<script src="_external/cdn.test/lib.js" crossorigin="anonymous"></script>"#);
    let lib = served(&bundle, "_external/cdn.test/lib.js");
    assert!(!lib.contains("sourceMappingURL"), "a fetched file loses its source map comment: {lib}");
}
