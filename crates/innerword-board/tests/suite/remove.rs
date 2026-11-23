//! `remove`: elements removed by id (section 7.3).

use innerword_board::{BoardError, HashedMaking, Scene, add, remove};
use serde_json::Value;

use super::common::*;
use super::scene::picture_map;

fn removed(scene: &Scene, ids: &[&str]) -> (Scene, innerword_board::Removed) {
    let mut scene = scene.clone();
    let ids: Vec<String> = ids.iter().map(|s| s.to_string()).collect();
    let done = remove(&mut scene, &ids, &mut HashedMaking::new(&[b"rm"], 99)).unwrap();
    (scene, done)
}

fn board() -> Scene {
    drawn(
        r#"[{"type":"rectangle","id":"api","x":0,"y":0,"width":100,"height":100,"label":{"text":"API"}},
           {"type":"rectangle","id":"db","x":300,"y":0,"width":100,"height":100},
           {"type":"arrow","id":"reads","x":100,"y":50,"width":200,"start":{"id":"api"},"end":{"id":"db"},"label":{"text":"reads"}},
           {"type":"frame","id":"f","children":["api","db"],"name":"Area"}]"#,
    )
}

#[test]
fn a_shape_goes_with_its_label_and_its_arrows_stay_unbound() {
    let scene = board();
    let label = scene.element("api").unwrap().bound_elements()[0]
        .0
        .to_string();
    let (after, done) = removed(&scene, &["api"]);
    assert_eq!(done.removed, ["api".to_string(), label.clone()]);
    assert_eq!(done.unbound, ["reads"]);
    assert!(after.element("api").is_none() && after.element(&label).is_none());
    let reads = after.element("reads").unwrap();
    assert_eq!(reads.json()["startBinding"], Value::Null);
    assert_eq!(reads.binding("end"), Some("db"));
    assert_eq!(
        reads.version(),
        scene.element("reads").unwrap().version() + 1
    );
}

#[test]
fn an_arrow_goes_with_its_label_and_leaves_its_ends() {
    let scene = board();
    let label = scene.element("reads").unwrap().bound_elements()[0]
        .0
        .to_string();
    let (after, done) = removed(&scene, &["reads", "reads"]);
    assert_eq!(done.removed, ["reads".to_string(), label]);
    assert!(done.unbound.is_empty());
    for id in ["api", "db"] {
        let e = after.element(id).unwrap();
        assert!(
            !e.bound_elements().iter().any(|(b, _)| *b == "reads"),
            "{id}"
        );
        assert_eq!(e.version(), scene.element(id).unwrap().version() + 1);
    }
}

#[test]
fn a_label_goes_alone() {
    let scene = board();
    let label = scene.element("api").unwrap().bound_elements()[0]
        .0
        .to_string();
    let (after, done) = removed(&scene, &[&label]);
    assert_eq!(done.removed, std::slice::from_ref(&label));
    assert!(
        after
            .element("api")
            .unwrap()
            .bound_elements()
            .iter()
            .all(|(b, _)| *b != label)
    );
    assert_eq!(after.elements().len(), scene.elements().len() - 1);
}

#[test]
fn a_frame_goes_and_its_children_stay() {
    let scene = board();
    let (after, done) = removed(&scene, &["f"]);
    assert_eq!(done.removed, ["f"]);
    for id in ["api", "db", "reads"] {
        assert_eq!(after.element(id).unwrap().frame_id(), None, "{id}");
    }
}

#[test]
fn a_picture_s_entry_goes_when_nothing_names_it() {
    let mut scene = Scene::empty();
    let pictures = picture_map(
        "p.png",
        "0123456789abcdef0123456789abcdef",
        "image/png",
        (40, 20),
        72,
    );
    add(
        &mut scene,
        &skeletons(r#"[{"type":"image","id":"one","x":0,"y":0,"path":"p.png"},{"type":"image","id":"two","x":50,"y":0,"path":"p.png"}]"#),
        &pictures,
        &mut making(),
    )
    .unwrap();
    let (after, _) = removed(&scene, &["one"]);
    assert_eq!(after.files().len(), 1);
    let (after, _) = removed(&after, &["two"]);
    assert!(after.files().is_empty());
    assert!(after.stored_files().is_empty());
}

#[test]
fn an_id_the_whiteboard_does_not_have_removes_nothing() {
    let scene = board();
    let mut copy = scene.clone();
    let error = remove(
        &mut copy,
        &["api".into(), "dbx".into()],
        &mut HashedMaking::new(&[b"rm"], 1),
    )
    .unwrap_err();
    assert!(
        matches!(error, BoardError::NotFound { ref id, ref known } if id == "dbx" && known[0] == "db")
    );
    assert_eq!(copy, scene);
}
