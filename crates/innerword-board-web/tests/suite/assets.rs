//! Where the bundle is found, and what it serves.

use std::path::Path;

use innerword_board_web::Assets;

use super::dist;

#[test]
fn outside_an_app_bundle_the_crates_dist_is_found() {
    let assets = Assets::find().unwrap();
    assert_eq!(assets.root().canonicalize().unwrap(), dist().canonicalize().unwrap());
    let assets = Assets::find_for(Path::new("/some/where/target/debug/inner-word")).unwrap();
    assert_eq!(assets.root().canonicalize().unwrap(), dist().canonicalize().unwrap());
}

fn copy_manifest_to(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::copy(dist().join("MANIFEST.json"), dir.join("MANIFEST.json")).unwrap();
    std::fs::copy(dist().join("index.html"), dir.join("index.html")).unwrap();
}

#[test]
fn inside_an_app_bundle_only_resources_whiteboard_is_found() {
    let temporary = tempfile::tempdir().unwrap();
    let app = temporary.path().join("Inner Word.app");
    let executable = app.join("Contents/MacOS/inner-word");
    std::fs::create_dir_all(executable.parent().unwrap()).unwrap();

    // Without Resources/Whiteboard the bundle has no whiteboard, though dist/ is there.
    let error = Assets::find_for(&executable).unwrap_err();
    assert!(error.starts_with("The whiteboard's files are missing from Inner Word ("), "{error}");
    assert!(error.contains("Resources/Whiteboard"), "{error}");

    let folder = app.join("Contents/Resources/Whiteboard");
    copy_manifest_to(&folder);
    let assets = Assets::find_for(&executable).unwrap();
    assert_eq!(assets.root(), folder);
    // The tool beside the app finds the same folder.
    let tool = Assets::find_for(&app.join("Contents/MacOS/innerword")).unwrap();
    assert_eq!(tool.root(), folder);
}

#[test]
fn a_folder_that_only_looks_like_a_bundle_is_not_one() {
    for path in ["/x/Inner Word/Contents/MacOS/inner-word", "/x/Inner Word.app/Contents/Other/inner-word", "/x/Inner Word.app/MacOS/inner-word"] {
        let assets = Assets::find_for(Path::new(path)).unwrap();
        assert_eq!(assets.root().canonicalize().unwrap(), dist().canonicalize().unwrap(), "{path}");
    }
}

#[test]
fn a_folder_without_a_manifest_is_refused() {
    let temporary = tempfile::tempdir().unwrap();
    std::fs::write(temporary.path().join("index.html"), "<!doctype html>").unwrap();
    let error = Assets::at(temporary.path()).unwrap_err();
    assert!(error.starts_with("The whiteboard's files are missing from Inner Word ("), "{error}");
    assert!(error.contains("MANIFEST.json"), "{error}");
}

#[test]
fn a_manifest_that_names_a_path_outside_its_folder_is_refused() {
    let temporary = tempfile::tempdir().unwrap();
    for bad in ["../secret", "/etc/passwd", "a/../../b", "a//b", ""] {
        let manifest = serde_json::json!({ "files": [{ "path": bad, "bytes": 1, "sha256": "00", "type": "text/plain" }] });
        std::fs::write(temporary.path().join("MANIFEST.json"), manifest.to_string()).unwrap();
        assert!(Assets::at(temporary.path()).is_err(), "{bad:?}");
    }
}

#[test]
fn only_the_files_of_the_manifest_are_served_with_their_types() {
    let assets = Assets::at(&dist()).unwrap();
    let main = assets.get("assets/main.js").unwrap();
    assert_eq!(main.media_type, "text/javascript; charset=utf-8");
    let bytes = assets.read("assets/main.js").unwrap().unwrap();
    assert_eq!(bytes.len() as u64, main.bytes);
    assert_eq!(super::sha256_hex(&bytes), main.sha256);
    assert_eq!(assets.get("index.html").unwrap().media_type, "text/html; charset=utf-8");
    assert!(assets.get("fonts/Excalifont/Excalifont-Regular-a88b72a24fb54c9f94e3b5fdaa7481c9.woff2").is_some_and(|f| f.media_type == "font/woff2"));

    // Not files of the bundle: the caller's to answer, or nobody's.
    for path in ["board.json", "library.json", "scene.json", "_files/0123456789abcdef0123456789abcdef", "MANIFEST.json", "../Cargo.toml", "/index.html", "assets/../index.html", "Index.html"] {
        assert!(assets.read(path).is_none(), "{path}");
        assert!(assets.get(path).is_none(), "{path}");
    }
    let paths: Vec<&str> = assets.paths().collect();
    assert!(paths.contains(&"index.html") && paths.contains(&"export.html") && paths.contains(&"NOTICES.txt"));
    assert!(paths.windows(2).all(|w| w[0] < w[1]));
}

#[test]
fn a_file_whose_size_is_not_the_manifests_is_not_served() {
    let temporary = tempfile::tempdir().unwrap();
    let manifest = serde_json::json!({ "files": [{ "path": "index.html", "bytes": 3, "sha256": "x", "type": "text/html; charset=utf-8" }] });
    std::fs::write(temporary.path().join("MANIFEST.json"), manifest.to_string()).unwrap();
    std::fs::write(temporary.path().join("index.html"), "<p>more than three bytes</p>").unwrap();
    let assets = Assets::at(temporary.path()).unwrap();
    assert!(assets.read("index.html").unwrap().is_err());
    std::fs::write(temporary.path().join("index.html"), "abc").unwrap();
    assert_eq!(assets.read("index.html").unwrap().unwrap(), b"abc");
}
