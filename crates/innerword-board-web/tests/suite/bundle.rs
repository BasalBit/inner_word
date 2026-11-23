//! dist/ against its manifest, its build info, its notices and what it is built from.

use std::collections::{BTreeMap, BTreeSet};

use super::{dist, files_under, read_json, sha256_hex, web};

#[test]
fn every_file_of_dist_is_in_the_manifest_with_its_size_and_sha256_and_nothing_else() {
    let manifest = read_json(&dist().join("MANIFEST.json"));
    let listed: BTreeMap<String, &serde_json::Value> = manifest["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (f["path"].as_str().unwrap().to_string(), f))
        .collect();
    let on_disk: BTreeSet<String> = files_under(&dist())
        .into_iter()
        .filter(|p| p != "MANIFEST.json")
        .collect();
    let in_manifest: BTreeSet<String> = listed.keys().cloned().collect();
    assert_eq!(
        on_disk, in_manifest,
        "dist/ and MANIFEST.json name different files"
    );
    for (path, entry) in &listed {
        let bytes = std::fs::read(dist().join(path)).unwrap();
        assert_eq!(
            entry["bytes"].as_u64().unwrap(),
            bytes.len() as u64,
            "{path}: size"
        );
        assert_eq!(
            entry["sha256"].as_str().unwrap(),
            sha256_hex(&bytes),
            "{path}: sha256"
        );
    }
}

#[test]
fn every_file_has_the_media_type_of_its_ending() {
    let types = [
        (".html", "text/html; charset=utf-8"),
        (".js", "text/javascript; charset=utf-8"),
        (".css", "text/css; charset=utf-8"),
        (".woff2", "font/woff2"),
        (".wasm", "application/wasm"),
        (".json", "application/json"),
        (".svg", "image/svg+xml"),
        (".png", "image/png"),
        (".txt", "text/plain; charset=utf-8"),
    ];
    let manifest = read_json(&dist().join("MANIFEST.json"));
    for entry in manifest["files"].as_array().unwrap() {
        let path = entry["path"].as_str().unwrap();
        let expected = types
            .iter()
            .find(|(ending, _)| path.ends_with(ending))
            .unwrap_or_else(|| panic!("{path}: an ending the bundle may not have"))
            .1;
        assert_eq!(entry["type"].as_str().unwrap(), expected, "{path}");
    }
}

/// A source changed without `node build.mjs` fails here (and `node build.mjs --check`
/// holds the rest: that dist/ is what the inputs build).
#[test]
fn the_build_info_names_every_input_under_web_with_its_sha256() {
    let info = read_json(&dist().join("BUILD-INFO.json"));
    let inputs = info["inputs"].as_object().unwrap();
    let mut expected: BTreeSet<String> = [
        "package.json",
        "package-lock.json",
        ".npmrc",
        "build.mjs",
        "bundled-inside.json",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    for dir in ["src", "fonts-licenses", "licenses"] {
        for file in files_under(&web().join(dir)) {
            expected.insert(format!("{dir}/{file}"));
        }
    }
    let listed: BTreeSet<String> = inputs.keys().cloned().collect();
    assert_eq!(
        listed, expected,
        "BUILD-INFO.json and web/ name different inputs: run node build.mjs"
    );
    for (path, sha) in inputs {
        let bytes = std::fs::read(web().join(path)).unwrap();
        assert_eq!(
            sha.as_str().unwrap(),
            sha256_hex(&bytes),
            "web/{path} changed since dist/ was built: run node build.mjs"
        );
    }
}

#[test]
fn every_patch_was_applied_as_often_as_it_must_be() {
    let info = read_json(&dist().join("BUILD-INFO.json"));
    let applied: BTreeMap<&str, u64> = info["patches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| (p["name"].as_str().unwrap(), p["applied"].as_u64().unwrap()))
        .collect();
    assert_eq!(applied["no font CDN"], 1);
    assert_eq!(applied["no Xiaolai"], 1);
    assert!(applied["locales other than English left out"] >= 50);
    assert_eq!(applied["no eval in the font subsetter: named functions"], 1);
    assert_eq!(applied["no eval in the font subsetter: dynamic callers"], 1);
    assert_eq!(applied["no eval in the font subsetter: invokers"], 1);
}

#[test]
fn the_versions_agree() {
    let package = read_json(&web().join("package.json"));
    assert_eq!(
        package["dependencies"]["@excalidraw/excalidraw"],
        innerword_board_web::EXCALIDRAW_VERSION
    );
    let info = read_json(&dist().join("BUILD-INFO.json"));
    assert_eq!(info["excalidraw"], innerword_board_web::EXCALIDRAW_VERSION);
    assert_eq!(
        innerword_board::EXCALIDRAW_VERSION,
        innerword_board_web::EXCALIDRAW_VERSION
    );
    let metrics = read_json(&dist().join("font-metrics.json"));
    assert_eq!(
        metrics["excalidraw"],
        innerword_board_web::EXCALIDRAW_VERSION
    );
    // Every version in package.json is exact: no ranges.
    for section in ["dependencies", "devDependencies"] {
        for (name, version) in package[section].as_object().unwrap() {
            let version = version.as_str().unwrap();
            assert!(
                version.chars().all(|c| c.is_ascii_digit() || c == '.'),
                "{name} {version} is not an exact version"
            );
        }
    }
}

#[test]
fn the_scene_file_of_the_export_page_is_the_one_the_tool_writes() {
    assert_eq!(
        innerword_board_web::SCENE_PATH,
        innerword_board::export_dir::SCENE_FILE
    );
    assert_eq!(
        innerword_board_web::FILES_PREFIX,
        format!("{}/", innerword_board::export_dir::FILES_DIR)
    );
}

#[test]
fn innerword_board_keeps_the_same_font_metrics() {
    let ours = std::fs::read_to_string(dist().join(innerword_board_web::FONT_METRICS)).unwrap();
    assert_eq!(
        innerword_board::metrics::METRICS_JSON,
        ours,
        "copy dist/font-metrics.json into crates/innerword-board/src/metrics/ (web/UPGRADING.md)"
    );
}

#[test]
fn the_font_metrics_cover_every_family_a_scene_can_name() {
    let metrics = read_json(&dist().join("font-metrics.json"));
    let first = metrics["first"].as_u64().unwrap();
    let last = metrics["last"].as_u64().unwrap();
    assert_eq!((first, last), (0x20, 0x24f));
    let families = metrics["families"].as_object().unwrap();
    let numbers: Vec<&str> = families.keys().map(String::as_str).collect();
    assert_eq!(numbers, ["1", "3", "5", "6", "7", "8", "9"]);
    for (number, family) in families {
        let advances = family["advances"].as_array().unwrap();
        assert_eq!(advances.len() as u64, last - first + 1, "family {number}");
        // Every family has the letters of English.
        for c in ('A'..='Z').chain('a'..='z') {
            assert!(
                advances[(c as u64 - first) as usize].as_u64().unwrap() > 0,
                "family {number} has no {c}"
            );
        }
        assert!(family["unitsPerEm"].as_u64().unwrap() >= 1000);
    }
}

#[test]
fn xiaolai_is_left_out_and_named_nowhere() {
    assert!(!dist().join("fonts").join("Xiaolai").exists());
    for file in files_under(&dist()) {
        if file.ends_with(".woff2") {
            continue;
        }
        let text =
            String::from_utf8_lossy(&std::fs::read(dist().join(&file)).unwrap()).into_owned();
        assert!(
            !text.contains("fonts/Xiaolai/"),
            "{file} names a Xiaolai file"
        );
    }
}

#[test]
fn no_html_file_has_an_inline_script_or_an_inline_event_handler() {
    for file in files_under(&dist())
        .into_iter()
        .filter(|f| f.ends_with(".html"))
    {
        let html = std::fs::read_to_string(dist().join(&file)).unwrap();
        let lower = html.to_lowercase();
        let mut at = 0;
        while let Some(found) = lower[at..].find("<script") {
            let start = at + found;
            let end = start + lower[start..].find('>').unwrap();
            let tag = &lower[start..=end];
            assert!(tag.contains("src="), "{file}: a script without src: {tag}");
            let close = end + 1 + lower[end + 1..].find("</script>").unwrap();
            assert!(
                lower[end + 1..close].trim().is_empty(),
                "{file}: a script with a body"
            );
            at = close;
        }
        // An attribute on…= is an inline handler.
        let bytes = lower.as_bytes();
        for (i, window) in lower.match_indices(" on") {
            let rest = &lower[i + window.len()..];
            let name: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphabetic())
                .collect();
            let after = rest[name.len()..].trim_start();
            assert!(
                name.is_empty() || !after.starts_with('='),
                "{file}: an inline handler on{name} (at byte {i} of {})",
                bytes.len()
            );
        }
        assert!(
            !lower.contains("javascript:"),
            "{file}: a javascript: address"
        );
    }
}

#[test]
fn the_two_pages_load_the_same_script() {
    for page in [
        innerword_board_web::ENTRY,
        innerword_board_web::EXPORT_ENTRY,
    ] {
        let html = std::fs::read_to_string(dist().join(page)).unwrap();
        assert!(
            html.contains(r#"<script type="module" src="./assets/main.js"></script>"#),
            "{page}"
        );
    }
    let manifest = innerword_board_web::Assets::at(&dist()).unwrap();
    assert!(manifest.get("assets/main.js").is_some());
    assert!(manifest.get("assets/main.css").is_some());
}

#[test]
fn the_notices_name_every_package_and_every_font() {
    let notices = std::fs::read_to_string(dist().join(innerword_board_web::NOTICES)).unwrap();
    let info = read_json(&dist().join("BUILD-INFO.json"));
    // Every package esbuild's metafile shows with code in the bundle (written by build.mjs
    // apart from the notices, before any license is read), on a line of its own.
    let packages = info["metafilePackages"].as_array().unwrap();
    assert!(
        packages.len() >= 40,
        "only {} packages in the metafile",
        packages.len()
    );
    for package in packages {
        let (name, version) = package.as_str().unwrap().rsplit_once('@').unwrap();
        assert!(
            notices
                .lines()
                .any(|l| l.starts_with(&format!("{name} {version}: "))),
            "NOTICES.txt does not name {name} {version}"
        );
    }
    // Every package a package carries inside its own files (its source maps), under it.
    let mut carried = 0;
    for (carrier, names) in info["carriedInside"].as_object().unwrap() {
        let (carrier_name, carrier_version) = carrier.rsplit_once('@').unwrap();
        assert!(
            packages.iter().any(|p| p == carrier),
            "{carrier} carries packages and is not in the metafile"
        );
        for name in names.as_array().unwrap() {
            let name = name.as_str().unwrap();
            let inside = format!("(inside {carrier_name} {carrier_version}");
            assert!(
                notices
                    .lines()
                    .any(|l| l.starts_with(&format!("{name} ")) && l.contains(&inside)),
                "NOTICES.txt does not name {name} inside {carrier}"
            );
            carried += 1;
        }
    }
    assert!(
        carried >= 10,
        "only {carried} packages carried inside others"
    );
    for folder in std::fs::read_dir(dist().join("fonts")).unwrap() {
        let folder = folder.unwrap().file_name().to_string_lossy().into_owned();
        assert!(
            notices.contains(&format!("fonts/{folder}/")),
            "NOTICES.txt does not name the font folder {folder}"
        );
    }
    assert!(notices.contains("SIL OPEN FONT LICENSE Version 1.1"));
    assert!(notices.contains("Copyright (c) 2018 Shannon Miwa"));
    // DOMPurify is under (MPL-2.0 OR Apache-2.0), taken under Apache-2.0.
    let dompurify = notices
        .lines()
        .find(|l| l.starts_with("dompurify "))
        .expect("DOMPurify is in the bundle");
    assert!(
        dompurify.contains("Apache-2.0") && dompurify.contains("taken under Apache-2.0"),
        "{dompurify}"
    );
}

#[test]
fn the_notices_name_what_packages_carry_inside_their_files() {
    let notices = std::fs::read_to_string(dist().join(innerword_board_web::NOTICES)).unwrap();
    let declared = read_json(&web().join("bundled-inside.json"));
    let mut named = 0;
    for (carrier, inside) in declared["carriers"].as_object().unwrap() {
        let carrier_name = carrier.rsplit_once('@').unwrap().0;
        for name in inside["fromNodeModules"].as_array().into_iter().flatten() {
            let name = name.as_str().unwrap();
            assert!(
                notices.contains(&format!("{name} (inside {carrier_name} ")),
                "{name} inside {carrier}"
            );
            named += 1;
        }
        for name in inside["withATextOfOurs"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(k, _)| k)
        {
            assert!(
                notices.contains(&format!("{name} "))
                    && notices.contains(&format!("(inside {carrier_name} ")),
                "{name} inside {carrier}"
            );
            named += 1;
        }
    }
    assert!(named >= 10);
    for name in ["langium", "chevrotain"] {
        assert!(
            notices.contains(&format!("{name} (inside @mermaid-js/parser ")),
            "{name}"
        );
    }
}

#[test]
fn mermaid_ships_whole() {
    let info = read_json(&dist().join("BUILD-INFO.json"));
    let chunks: Vec<&str> = info["mermaidChunks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap())
        .collect();
    assert!(
        chunks.len() >= 50,
        "only {} chunks of Mermaid",
        chunks.len()
    );
    let assets = innerword_board_web::Assets::at(&dist()).unwrap();
    let mut bytes = 0;
    for chunk in &chunks {
        let info = assets
            .get(chunk)
            .unwrap_or_else(|| panic!("{chunk} is not in MANIFEST.json"));
        bytes += info.bytes;
    }
    assert!(bytes > 2_000_000, "Mermaid's chunks are {bytes} bytes");
    // No stub stands for it (the prototype's --no-mermaid is not carried over).
    for file in files_under(&dist().join("assets")) {
        let text = std::fs::read_to_string(dist().join("assets").join(&file)).unwrap();
        assert!(
            !text.contains("Mermaid is not in this build"),
            "{file} is a stub of Mermaid"
        );
    }
    let all: String = chunks
        .iter()
        .map(|c| std::fs::read_to_string(dist().join(c)).unwrap())
        .collect();
    assert!(all.contains("flowchart") && all.contains("sequenceDiagram"));
}

/// Where `name` stands as an identifier of its own in minified code (not part of a longer
/// name, not a property after a dot), with the text that follows it.
fn identifier_uses<'a>(text: &'a str, name: &str) -> Vec<&'a str> {
    let part = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'$';
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(at) = text[from..].find(name).map(|i| i + from) {
        let end = at + name.len();
        let before = at.checked_sub(1).map(|i| bytes[i]);
        let after = bytes.get(end).copied();
        if !before.is_some_and(|c| part(c) || c == b'.') && !after.is_some_and(part) {
            found.push(&text[end..(end + 40).min(text.len())]);
        }
        from = end;
    }
    found
}

#[test]
fn nothing_in_the_bundle_evaluates_a_string() {
    // The live view's policy has no eval, and the export page's only WebAssembly's: the
    // font subsetter's `new Function`s are patched out at the build. What is left is
    // counted exactly, so that a new one (an upgrade of a package) fails here and is read.
    let mut function_calls = 0;
    let mut evals = Vec::new();
    for file in files_under(&dist().join("assets")) {
        if !file.ends_with(".js") {
            continue;
        }
        let text = std::fs::read_to_string(dist().join("assets").join(&file)).unwrap();
        assert!(!text.contains("new Function"), "{file} has new Function");
        assert!(!text.contains("(Function,"), "{file} builds a Function");
        for timer in ["setTimeout(", "setInterval("] {
            for quote in ['"', '\'', '`'] {
                assert!(
                    !text.contains(&format!("{timer}{quote}")),
                    "{file} runs a string in {timer}"
                );
            }
        }
        for quote in ['"', '\'', '`'] {
            assert!(
                !text.contains(&format!(".constructor({quote}")),
                "{file} makes a function of a string"
            );
        }
        for after in identifier_uses(&text, "Function") {
            if after.starts_with('(') {
                // lodash's `Function("return this")()`, never reached in a window or a
                // worker: `self` is there and is taken first.
                assert!(
                    after.starts_with(r#"("return this")()"#),
                    "{file} calls Function{after}"
                );
                function_calls += 1;
            }
        }
        for after in identifier_uses(&text, "eval") {
            evals.push(format!("{file}: eval{after}"));
        }
    }
    assert_eq!(
        function_calls, 6,
        "lodash's Function(\"return this\"), one per copy of it in the bundle"
    );
    // chevrotain's toFastProperties: `return t; (0,eval)(t)`, after its `return` and given
    // an object, never a string.
    assert_eq!(evals.len(), 1, "{evals:#?}");
    assert!(
        evals[0].contains("eval)(") && evals[0].contains(r#""toFastProperties""#),
        "{evals:#?}"
    );
}

#[test]
fn the_glue_and_the_rust_half_agree_on_the_bridge() {
    let bridge = std::fs::read_to_string(web().join("src/glue/bridge.js")).unwrap();
    assert!(bridge.contains(&format!(
        r#"export const HANDLER = "{}";"#,
        innerword_board_web::BRIDGE_NAME
    )));
    assert!(bridge.contains("export const MAX_MESSAGE_BYTES = 24 * 1024 * 1024;"));
    assert_eq!(
        innerword_board_web::bridge::MAX_MESSAGE_BYTES,
        24 * 1024 * 1024
    );
    assert!(bridge.contains("export const PART_BYTES = 16 * 1024 * 1024;"));
    assert_eq!(innerword_board_web::bridge::PART_BYTES, 16 * 1024 * 1024);
    assert!(bridge.contains("export const MAX_VIEW_SCENE_BYTES = 256 * 1024 * 1024;"));
    assert_eq!(
        innerword_board::limits::MAX_VIEW_SCENE_BYTES,
        256 * 1024 * 1024
    );
    let board = std::fs::read_to_string(web().join("src/glue/board.jsx")).unwrap();
    for path in [
        innerword_board_web::BOARD_PATH,
        innerword_board_web::LIBRARY_PATH,
    ] {
        assert!(
            board.contains(&format!(r#""./{path}""#)),
            "the glue does not fetch {path}"
        );
    }
    let export = std::fs::read_to_string(web().join("src/glue/export.js")).unwrap();
    assert!(export.contains(&format!(r#""./{}""#, innerword_board_web::SCENE_PATH)));
    let pictures = std::fs::read_to_string(web().join("src/glue/pictures.js")).unwrap();
    assert!(pictures.contains(&format!(
        "`./{}${{file}}`",
        innerword_board_web::FILES_PREFIX
    )));
}
