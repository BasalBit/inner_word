//! The delta between two scenes, and the merge by the base rule (section 4.5).

use innerword_board::{Scene, merge};
use serde_json::{Value, json};

/// A stored scene of rectangles: (id, version, index, x).
fn scene(elements: &[(&str, i64, &str, f64)], files: Value) -> Scene {
    let elements: Vec<Value> = elements
        .iter()
        .map(|(id, version, index, x)| {
            json!({"id": id, "type": "rectangle", "x": x, "y": 0, "width": 10, "height": 10,
                   "version": version, "versionNonce": version * 7, "index": index})
        })
        .collect();
    let text = json!({"type": "excalidraw", "version": 2, "elements": elements, "files": files})
        .to_string();
    Scene::from_stored(&text).unwrap().scene
}

fn ids(scene: &Scene) -> Vec<String> {
    scene
        .elements()
        .iter()
        .map(|e| e.id().to_string())
        .collect()
}

fn x_of(scene: &Scene, id: &str) -> f64 {
    scene.element(id).unwrap().x()
}

#[test]
fn the_delta_names_what_was_added_changed_and_removed() {
    let file = json!({"f1": {"mimeType": "image/png", "file": "0123456789abcdef0123456789abcdef", "created": 1}});
    let before = scene(
        &[
            ("a", 1, "a0", 0.0),
            ("b", 3, "a1", 0.0),
            ("c", 2, "a2", 0.0),
        ],
        json!({}),
    );
    let mut after = scene(
        &[
            ("a", 1, "a0", 0.0),
            ("b", 4, "a1", 50.0),
            ("d", 1, "a3", 0.0),
        ],
        file,
    );
    // A picture element names the new file, so the entry is the scene's.
    let text = after.to_stored().replace(
        "\"elements\":[",
        "\"elements\":[{\"id\":\"p\",\"type\":\"image\",\"fileId\":\"f1\",\"version\":1,\"index\":\"a4\"},",
    );
    after = Scene::from_stored(&text).unwrap().scene;
    let delta = after.delta_from(&before);
    let upserted: Vec<(String, Option<i64>)> = delta
        .upserted
        .iter()
        .map(|(e, v)| (e.id().to_string(), *v))
        .collect();
    assert_eq!(
        upserted,
        [
            ("p".to_string(), None),
            ("b".to_string(), Some(3)),
            ("d".to_string(), None)
        ]
    );
    assert_eq!(delta.removed, [("c".to_string(), 2)]);
    assert_eq!(delta.files.keys().collect::<Vec<_>>(), ["f1"]);
    assert!(!delta.is_empty());
    assert!(before.delta_from(&before).is_empty());

    // A change of the nonce alone counts.
    let mut nonce = before.clone();
    let text = nonce
        .to_stored()
        .replace("\"versionNonce\":21", "\"versionNonce\":22");
    nonce = Scene::from_stored(&text).unwrap().scene;
    assert_eq!(nonce.delta_from(&before).upserted.len(), 1);

    let wire: Value = serde_json::from_str(&delta.to_json(31, true)).unwrap();
    assert_eq!(wire["baseRev"], 31);
    assert_eq!(wire["undoable"], true);
    assert_eq!(wire["upserted"][0]["baseVersion"], Value::Null);
    assert_eq!(wire["upserted"][1]["baseVersion"], 3);
    assert_eq!(wire["upserted"][1]["element"]["x"].as_f64(), Some(50.0));
    assert_eq!(wire["removed"], json!([{"id": "c", "baseVersion": 2}]));
    assert_eq!(
        wire["files"],
        json!({"f1": {"mimeType": "image/png", "file": "0123456789abcdef0123456789abcdef"}})
    );
    let quiet: Value = serde_json::from_str(&before.delta_from(&before).to_json(0, false)).unwrap();
    assert_eq!(
        quiet,
        json!({"baseRev": 0, "upserted": [], "removed": [], "files": {}, "undoable": false})
    );
}

#[test]
fn the_base_rule_takes_theirs_where_the_writer_did_not_touch_the_element() {
    let base = scene(&[("a", 3, "a0", 0.0)], json!({}));
    let writer = base.clone();
    let theirs = scene(&[("a", 4, "a0", 40.0)], json!({}));
    let merged = merge(&base, &writer, &theirs);
    assert!(merged.conflicts.is_empty());
    assert_eq!(x_of(&merged.scene, "a"), 40.0);
    assert_eq!(merged.scene.element("a").unwrap().version(), 4);
}

#[test]
fn the_taken_copy_is_at_the_higher_version() {
    let base = scene(&[("a", 3, "a0", 0.0)], json!({}));
    let theirs = scene(&[("a", 2, "a0", 40.0)], json!({}));
    let merged = merge(&base, &base, &theirs);
    // A version that went down (a restored version) still rises above the writer's.
    assert_eq!(merged.scene.element("a").unwrap().version(), 4);
    assert_eq!(x_of(&merged.scene, "a"), 40.0);
}

#[test]
fn the_writer_s_change_stays_and_theirs_is_a_conflict() {
    let base = scene(&[("a", 3, "a0", 0.0)], json!({}));
    let writer = scene(&[("a", 5, "a0", 10.0)], json!({}));
    let theirs = scene(&[("a", 4, "a0", 40.0)], json!({}));
    let merged = merge(&base, &writer, &theirs);
    assert_eq!(merged.conflicts, ["a"]);
    assert_eq!(x_of(&merged.scene, "a"), 10.0);
}

#[test]
fn a_new_element_of_theirs_is_added() {
    let base = scene(&[("a", 1, "a0", 0.0)], json!({}));
    let writer = scene(&[("a", 1, "a0", 0.0), ("mine", 1, "a1", 0.0)], json!({}));
    let theirs = scene(&[("a", 1, "a0", 0.0), ("new", 1, "a2", 70.0)], json!({}));
    let merged = merge(&base, &writer, &theirs);
    assert!(merged.conflicts.is_empty());
    assert_eq!(ids(&merged.scene), ["a", "mine", "new"]);
}

#[test]
fn an_element_the_writer_removed_is_not_brought_back() {
    let base = scene(&[("a", 1, "a0", 0.0), ("b", 1, "a1", 0.0)], json!({}));
    let writer = scene(&[("a", 1, "a0", 0.0)], json!({}));
    let theirs = scene(&[("a", 1, "a0", 0.0), ("b", 2, "a1", 9.0)], json!({}));
    let merged = merge(&base, &writer, &theirs);
    assert_eq!(merged.conflicts, ["b"]);
    assert_eq!(ids(&merged.scene), ["a"]);
}

#[test]
fn an_id_the_writer_gave_a_new_element_stays_the_writer_s() {
    let base = scene(&[("a", 1, "a0", 0.0)], json!({}));
    let writer = scene(&[("a", 1, "a0", 0.0), ("x", 1, "a1", 1.0)], json!({}));
    let theirs = scene(&[("a", 1, "a0", 0.0), ("x", 1, "a1", 2.0)], json!({}));
    let merged = merge(&base, &writer, &theirs);
    assert_eq!(merged.conflicts, ["x"]);
    assert_eq!(x_of(&merged.scene, "x"), 1.0);
}

#[test]
fn a_removal_of_theirs_takes_an_untouched_element_away() {
    let base = scene(&[("a", 1, "a0", 0.0), ("b", 2, "a1", 0.0)], json!({}));
    let theirs = scene(&[("a", 1, "a0", 0.0)], json!({}));
    let merged = merge(&base, &base, &theirs);
    assert!(merged.conflicts.is_empty());
    assert_eq!(ids(&merged.scene), ["a"]);
}

#[test]
fn a_removal_of_an_element_the_writer_changed_is_a_conflict() {
    let base = scene(&[("a", 1, "a0", 0.0), ("b", 2, "a1", 0.0)], json!({}));
    let writer = scene(&[("a", 1, "a0", 0.0), ("b", 3, "a1", 5.0)], json!({}));
    let theirs = scene(&[("a", 1, "a0", 0.0)], json!({}));
    let merged = merge(&base, &writer, &theirs);
    assert_eq!(merged.conflicts, ["b"]);
    assert_eq!(ids(&merged.scene), ["a", "b"]);
}

#[test]
fn an_element_both_removed_is_simply_gone() {
    let base = scene(&[("a", 1, "a0", 0.0), ("b", 2, "a1", 0.0)], json!({}));
    let writer = scene(&[("a", 1, "a0", 0.0)], json!({}));
    let merged = merge(&base, &writer, &writer);
    assert!(merged.conflicts.is_empty());
    assert_eq!(ids(&merged.scene), ["a"]);
}

#[test]
fn the_merge_is_in_index_order_and_keeps_the_pictures_of_both() {
    let base = scene(&[("a", 1, "a1", 0.0)], json!({}));
    let mut writer = scene(&[("a", 1, "a1", 0.0), ("w", 1, "a3", 0.0)], json!({}));
    let mut theirs = scene(
        &[
            ("t", 1, "a0", 0.0),
            ("a", 1, "a1", 0.0),
            ("u", 1, "a2", 0.0),
        ],
        json!({}),
    );
    let picture = |scene: &Scene, element: &str, file_id: &str, hex: &str, index: &str| {
        let text = scene.to_stored().replace(
            "\"elements\":[",
            &format!("\"elements\":[{{\"id\":\"{element}\",\"type\":\"image\",\"fileId\":\"{file_id}\",\"version\":1,\"index\":\"{index}\"}},"),
        );
        let text = text.replace("\"files\":{", &format!("\"files\":{{\"{file_id}\":{{\"mimeType\":\"image/png\",\"file\":\"{hex}\",\"created\":1}}"));
        let text = text.replace("}\"source\"", "},\"source\"");
        Scene::from_stored(&text).unwrap().scene
    };
    writer = picture(
        &writer,
        "wp",
        "fw",
        "11111111111111111111111111111111",
        "a4",
    );
    theirs = picture(
        &theirs,
        "tp",
        "ft",
        "22222222222222222222222222222222",
        "a5",
    );
    let merged = merge(&base, &writer, &theirs);
    assert!(merged.conflicts.is_empty());
    assert_eq!(ids(&merged.scene), ["t", "a", "u", "w", "wp", "tp"]);
    assert_eq!(
        merged.scene.files().keys().collect::<Vec<_>>(),
        ["ft", "fw"]
    );
}
