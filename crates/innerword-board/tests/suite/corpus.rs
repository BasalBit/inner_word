//! The corpus of skeletons and their golden scenes (`tests/corpus/README.md`).

use innerword_board::{HashedMaking, Pictures, Scene, add};

use super::common::*;

const NOW: i64 = 1_795_432_196_771;

#[test]
fn the_corpus_gives_its_golden_scenes() {
    let folder = crate_path("tests/corpus");
    let bless = std::env::var_os("INNERWORD_BOARD_BLESS").is_some();
    let mut names: Vec<String> = std::fs::read_dir(&folder)
        .unwrap()
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| name.ends_with(".json"))
        .collect();
    names.sort();
    assert!(names.len() >= 5, "{names:?}");
    let mut stale = Vec::new();
    for name in &names {
        let skeletons = skeletons(&std::fs::read_to_string(folder.join(name)).unwrap());
        let mut scene = Scene::empty();
        let mut making = HashedMaking::new(&[b"corpus", name.as_bytes()], NOW);
        add(&mut scene, &skeletons, &Pictures::new(), &mut making)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let made = pretty(&scene.to_stored());
        let golden = folder.join("golden").join(name);
        if bless {
            std::fs::write(&golden, &made).unwrap();
            continue;
        }
        let expected = std::fs::read_to_string(&golden).unwrap_or_default();
        if expected != made {
            stale.push(name.clone());
        }
        // Every skeleton's id is an element of the scene.
        for skeleton in &skeletons {
            let id = skeleton.0["id"].as_str().unwrap();
            assert!(scene.element(id).is_some(), "{name}: {id}");
        }
    }
    assert!(
        stale.is_empty(),
        "these conversions differ from tests/corpus/golden: {stale:?}; after a deliberate change run the tests with INNERWORD_BOARD_BLESS=1 and read the diff"
    );
}

fn pretty(stored: &str) -> String {
    let value: serde_json::Value = serde_json::from_str(stored).unwrap();
    let mut text = serde_json::to_string_pretty(&value).unwrap();
    text.push('\n');
    text
}
