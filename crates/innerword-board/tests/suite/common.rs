//! What the tests share.

use innerword_board::{HashedMaking, Pictures, Scene, Skeleton, add};

/// The path of a file of the crate.
pub fn crate_path(rest: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rest)
}

pub fn fixture(name: &str) -> String {
    std::fs::read_to_string(crate_path(&format!("tests/fixtures/{name}"))).unwrap()
}

pub fn making() -> HashedMaking {
    HashedMaking::new(&[b"12", b"stamp", b"request"], 1_795_432_196_771)
}

pub fn skeletons(json: &str) -> Vec<Skeleton> {
    serde_json::from_str(json).unwrap()
}

/// `add` on an empty scene; panics on a refusal.
pub fn drawn(json: &str) -> Scene {
    let mut scene = Scene::empty();
    add(
        &mut scene,
        &skeletons(json),
        &Pictures::new(),
        &mut making(),
    )
    .unwrap();
    scene
}
