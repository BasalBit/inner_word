//! `update`: changes by id and what follows them (section 7.3).

use innerword_board::{
    BoardError, ElementChange, HashedMaking, Pictures, Scene, add, limits, update,
};
use serde_json::{Value, json};

use super::common::*;
use super::scene::picture_map;

fn changes(json: &str) -> Vec<ElementChange> {
    serde_json::from_str(json).unwrap()
}

fn later() -> HashedMaking {
    HashedMaking::new(&[b"12", b"later"], 1_795_432_999_000)
}

fn changed(scene: &Scene, json: &str) -> (Scene, Vec<String>) {
    let mut scene = scene.clone();
    let done = update(&mut scene, &changes(json), &mut later()).unwrap_or_else(|e| panic!("{e}"));
    (scene, done.changed)
}

fn refused(scene: &Scene, json: &str) -> BoardError {
    let mut copy = scene.clone();
    let error = update(&mut copy, &changes(json), &mut later()).unwrap_err();
    assert_eq!(&copy, scene, "a refused change changes nothing");
    error
}

fn label_of(scene: &Scene, id: &str) -> String {
    scene
        .element(id)
        .unwrap()
        .bound_elements()
        .iter()
        .find(|(_, k)| *k == "text")
        .unwrap()
        .0
        .to_string()
}

/// A bound end lies on its element's outline, give or take the gap.
fn end_near(scene: &Scene, arrow: &str, which: usize) -> (f64, f64) {
    let a = scene.element(arrow).unwrap();
    let p = a.points()[which];
    (a.x() + p.0, a.y() + p.1)
}

fn board() -> Scene {
    drawn(
        r#"[{"type":"rectangle","id":"api","x":0,"y":0,"width":100,"height":100,"label":{"text":"API"}},
           {"type":"rectangle","id":"db","x":300,"y":0,"width":100,"height":100},
           {"type":"arrow","id":"reads","x":100,"y":50,"width":200,"start":{"id":"api"},"end":{"id":"db"},"label":{"text":"reads"}},
           {"type":"text","id":"note","x":0,"y":200,"text":"A note"},
           {"type":"line","id":"rule","x":0,"y":300,"points":[[0,0],[100,0],[100,50]]},
           {"type":"frame","id":"f","children":["db"],"name":"Data"}]"#,
    )
}

#[test]
fn a_shape_moved_takes_its_label_and_the_ends_of_its_arrows() {
    let scene = board();
    let label = label_of(&scene, "api");
    let (after, changed) = changed(&scene, r#"[{"id":"api","dy":200}]"#);
    let api = after.element("api").unwrap();
    assert_eq!((api.x(), api.y()), (0.0, 200.0));
    let moved = after.element(&label).unwrap();
    let before = scene.element(&label).unwrap();
    assert_eq!((moved.x(), moved.y() - 200.0), (before.x(), before.y()));
    // The arrow's start followed onto the moved rectangle's outline, aimed at its centre.
    let start = end_near(&after, "reads", 0);
    assert!(
        (100.0..=102.0).contains(&start.0) && (200.0..=300.0).contains(&start.1),
        "{start:?}"
    );
    assert_eq!(
        after.element("reads").unwrap().binding("start"),
        Some("api")
    );
    for id in ["api", &label, "reads"] {
        assert!(changed.contains(&id.to_string()), "{id} in {changed:?}");
        let e = after.element(id).unwrap();
        assert_eq!(
            e.version(),
            scene.element(id).unwrap().version() + 1,
            "{id}"
        );
        assert_ne!(
            e.version_nonce(),
            scene.element(id).unwrap().version_nonce()
        );
        assert_eq!(e.json()["updated"], 1_795_432_999_000i64);
    }
    // What nothing touched is as it was.
    assert_eq!(after.element("note"), scene.element("note"));
    // Absolute: the same.
    let (absolute, _) = self::changed(&scene, r#"[{"id":"api","x":0,"y":200}]"#);
    assert_eq!(absolute.element("api"), after.element("api"));
}

#[test]
fn a_frame_moved_takes_what_it_holds() {
    let scene = board();
    let (after, changed) = changed(&scene, r#"[{"id":"f","dx":-50,"dy":10}]"#);
    let db = after.element("db").unwrap();
    assert_eq!((db.x(), db.y()), (250.0, 10.0));
    let f = after.element("f").unwrap();
    assert_eq!(
        (f.x(), f.y()),
        (
            scene.element("f").unwrap().x() - 50.0,
            scene.element("f").unwrap().y() + 10.0
        )
    );
    assert!(changed.contains(&"db".to_string()));
    // The arrow's end followed db.
    let end = end_near(&after, "reads", 1);
    assert!(end.0 < 251.0 && end.0 > 240.0, "{end:?}");
}

#[test]
fn a_shape_resized_wraps_its_label_again() {
    let scene = drawn(
        r#"[{"type":"rectangle","id":"r","x":0,"y":0,"width":300,"height":60,"label":{"text":"Words that fit on one line"}}]"#,
    );
    let label = label_of(&scene, "r");
    assert_eq!(
        scene
            .element(&label)
            .unwrap()
            .text()
            .unwrap()
            .lines()
            .count(),
        1
    );
    let (after, _) = changed(&scene, r#"[{"id":"r","width":80}]"#);
    let text = after.element(&label).unwrap();
    assert!(text.text().unwrap().lines().count() > 2);
    assert!(
        after.element("r").unwrap().height() > 60.0,
        "the shape grew for its label"
    );
    assert!((text.x() + text.width() / 2.0 - 40.0).abs() < 1e-9);
}

#[test]
fn text_is_changed_and_measured_again() {
    let scene = board();
    let (after, _) = changed(
        &scene,
        r#"[{"id":"note","text":"A much longer note than before","fontSize":30}]"#,
    );
    let note = after.element("note").unwrap();
    assert_eq!(note.text(), Some("A much longer note than before"));
    assert_eq!(
        note.json()["originalText"],
        "A much longer note than before"
    );
    assert!(note.width() > scene.element("note").unwrap().width());
    assert_eq!(note.height(), 30.0 * 1.25);
    assert_eq!((note.x(), note.y()), (0.0, 200.0));
    // A label is changed through its container.
    let label = label_of(&scene, "api");
    let (after, changed) = changed(
        &scene,
        r#"[{"id":"api","text":"Gateway","fontFamily":"Nunito"}]"#,
    );
    assert_eq!(after.element(&label).unwrap().text(), Some("Gateway"));
    assert_eq!(after.element(&label).unwrap().json()["fontFamily"], 6);
    assert!(changed.contains(&label));
    // Or by its own id.
    let (after, _) = self::changed(&scene, &format!(r#"[{{"id":"{label}","text":"Edge"}}]"#));
    assert_eq!(after.element(&label).unwrap().text(), Some("Edge"));
}

#[test]
fn a_shape_without_a_label_gets_one() {
    let scene = board();
    let (after, changed) = changed(&scene, r#"[{"id":"db","text":"Postgres 17"}]"#);
    let label = label_of(&after, "db");
    let text = after.element(&label).unwrap();
    assert_eq!(text.json()["originalText"], "Postgres 17");
    assert_eq!(text.text().unwrap().replace('\n', " "), "Postgres 17");
    assert_eq!(text.container_id(), Some("db"));
    assert_eq!(
        text.frame_id(),
        Some("f"),
        "the label is in its container's frame"
    );
    assert!(changed.contains(&label));
    assert!(text.index().unwrap() > scene.elements().last().unwrap().index().unwrap());
    // A font without text, on a shape without a label: refused.
    let error = refused(&scene, r#"[{"id":"db","fontSize":30}]"#);
    assert!(
        matches!(error, BoardError::Invalid { field: Some(ref f), .. } if f == "fontSize"),
        "{error:?}"
    );
}

#[test]
fn style_link_lock_and_name() {
    let scene = board();
    let (after, _) = changed(
        &scene,
        r##"[{"id":"api","backgroundColor":"#ffc9c9","fillStyle":"zigzag","strokeWidth":1,"opacity":30,"link":"mailto:a@b.c","locked":true},
            {"id":"f","name":"Storage"},
            {"id":"reads","endArrowhead":"triangle","startArrowhead":"bar"}]"##,
    );
    let api = after.element("api").unwrap().json();
    assert_eq!(api["backgroundColor"], "#ffc9c9");
    assert_eq!(api["fillStyle"], "zigzag");
    assert_eq!(api["strokeWidth"], 1);
    assert_eq!(api["opacity"], 30);
    assert_eq!(api["link"], "mailto:a@b.c");
    assert_eq!(api["locked"], true);
    assert_eq!(after.element("f").unwrap().name(), Some("Storage"));
    assert_eq!(
        after.element("reads").unwrap().json()["endArrowhead"],
        "triangle"
    );
    let (cleared, _) = self::changed(&after, r#"[{"id":"api","link":null}]"#);
    assert_eq!(cleared.element("api").unwrap().json()["link"], Value::Null);
    // A style change leaves the label where it was.
    let label = label_of(&scene, "api");
    assert_eq!(
        after.element(&label).unwrap().x(),
        scene.element(&label).unwrap().x()
    );
}

#[test]
fn an_arrow_is_bound_again_or_unbound() {
    let mut scene = board();
    add(
        &mut scene,
        &skeletons(r#"[{"type":"ellipse","id":"cache","x":300,"y":300,"width":100,"height":100}]"#),
        &Pictures::new(),
        &mut making(),
    )
    .unwrap();
    let (after, changed) = changed(&scene, r#"[{"id":"reads","end":{"id":"cache"}}]"#);
    assert_eq!(
        after.element("reads").unwrap().binding("end"),
        Some("cache")
    );
    assert!(
        !after
            .element("db")
            .unwrap()
            .bound_elements()
            .iter()
            .any(|(id, _)| *id == "reads")
    );
    assert!(
        after
            .element("cache")
            .unwrap()
            .bound_elements()
            .contains(&("reads", "arrow"))
    );
    for id in ["db", "cache", "reads"] {
        assert!(changed.contains(&id.to_string()), "{id}");
    }
    let end = end_near(&after, "reads", 1);
    let on = ((end.0 - 350.0) / 50.0).powi(2) + ((end.1 - 350.0) / 50.0).powi(2);
    assert!(on > 0.98 && on < 1.1, "{end:?} {on}");
    let (loose, _) = self::changed(&after, r#"[{"id":"reads","end":null}]"#);
    assert_eq!(
        loose.element("reads").unwrap().json()["endBinding"],
        Value::Null
    );
    assert!(
        !loose
            .element("cache")
            .unwrap()
            .bound_elements()
            .iter()
            .any(|(id, _)| *id == "reads")
    );
    // An end the whiteboard does not have; an end that is no shape.
    assert!(matches!(
        refused(&scene, r#"[{"id":"reads","end":{"id":"nowhere"}}]"#),
        BoardError::NotFound { .. }
    ));
    assert!(matches!(
        refused(&scene, r#"[{"id":"reads","end":{"id":"rule"}}]"#),
        BoardError::Invalid { .. }
    ));
}

#[test]
fn points_are_replaced_or_scaled() {
    let scene = board();
    let (after, _) = changed(&scene, r#"[{"id":"rule","width":200,"height":25}]"#);
    assert_eq!(
        after.element("rule").unwrap().points(),
        vec![(0.0, 0.0), (200.0, 0.0), (200.0, 25.0)]
    );
    let (after, _) = changed(&scene, r#"[{"id":"rule","points":[[10,10],[60,10]]}]"#);
    let rule = after.element("rule").unwrap();
    assert_eq!(rule.points(), vec![(0.0, 0.0), (50.0, 0.0)]);
    assert_eq!(
        (rule.x(), rule.y(), rule.width(), rule.height()),
        (10.0, 310.0, 50.0, 0.0)
    );
    let error = refused(&scene, r#"[{"id":"rule","points":[[0,0],[0.5,0.5]]}]"#);
    assert!(matches!(error, BoardError::Invalid { field: Some(ref f), .. } if f == "points"));
}

#[test]
fn a_picture_is_moved_and_resized_but_keeps_its_file() {
    let mut scene = Scene::empty();
    let pictures = picture_map(
        "p.png",
        "0123456789abcdef0123456789abcdef",
        "image/png",
        (400, 200),
        72,
    );
    add(
        &mut scene,
        &skeletons(r#"[{"type":"image","id":"p","x":0,"y":0,"path":"p.png"}]"#),
        &pictures,
        &mut making(),
    )
    .unwrap();
    let (after, _) = changed(&scene, r#"[{"id":"p","width":100,"dx":5,"angle":0.5}]"#);
    let p = after.element("p").unwrap();
    assert_eq!(
        (p.x(), p.width(), p.height(), p.angle()),
        (5.0, 100.0, 50.0, 0.5)
    );
    assert_eq!(p.file_id(), Some("0123456789abcdef0123456789abcdef"));
    for field in ["path", "fileId", "backgroundColor", "strokeColor"] {
        let json = format!(r#"[{{"id":"p","{field}":"x"}}]"#);
        let error = refused(&scene, &json);
        assert!(
            matches!(error, BoardError::Invalid { field: Some(ref f), .. } if f == field),
            "{field}: {error:?}"
        );
    }
}

#[test]
fn each_refusal_of_a_change() {
    let scene = board();
    let label = label_of(&scene, "api");
    let cases: Vec<(String, usize, &str)> = vec![
        (r#"[{"dx":1}]"#.into(), 0, "id"),
        (r#"[{"id":"api","x":1,"dx":1}]"#.into(), 0, "dx"),
        (r#"[{"id":"api","type":"ellipse"}]"#.into(), 0, "type"),
        (r#"[{"id":"api","colour":"red"}]"#.into(), 0, "colour"),
        (r#"[{"id":"api","customData":{}}]"#.into(), 0, "customData"),
        (r#"[{"id":"api","label":{"text":"x"}}]"#.into(), 0, "label"),
        (r#"[{"id":"note","width":40}]"#.into(), 0, "width"),
        (r#"[{"id":"f","angle":1}]"#.into(), 0, "angle"),
        (r#"[{"id":"api","name":"x"}]"#.into(), 0, "name"),
        (
            r#"[{"id":"api","points":[[0,0],[9,9]]}]"#.into(),
            0,
            "points",
        ),
        (r#"[{"id":"rule","end":{"id":"api"}}]"#.into(), 0, "end"),
        (r#"[{"id":"api","width":0}]"#.into(), 0, "width"),
        (r#"[{"id":"note","text":"  "}]"#.into(), 0, "text"),
        (r#"[{"id":"api","opacity":200}]"#.into(), 0, "opacity"),
        (r#"[{"id":"api","dx":1},{"id":"db","f":1}]"#.into(), 1, "f"),
        (format!(r#"[{{"id":"{label}","dx":4}}]"#), 0, "dx"),
        (r#"[{"id":"f","children":["api"]}]"#.into(), 0, "children"),
    ];
    for (json, index, field) in cases {
        let error = refused(&scene, &json);
        match &error {
            BoardError::Invalid {
                element, field: f, ..
            } => {
                assert_eq!(
                    (*element, f.as_deref()),
                    (Some(index), Some(field)),
                    "{json}: {error}"
                );
            }
            other => panic!("{json}: {other:?}"),
        }
    }
    let error = refused(&scene, r#"[{"id":"aip","dx":1}]"#);
    match error {
        BoardError::NotFound { id, known } => {
            assert_eq!(id, "aip");
            assert_eq!(known[0], "api");
            assert!(known.len() <= 20);
        }
        other => panic!("{other:?}"),
    }
    let many: Vec<Value> = (0..=limits::MAX_REQUEST_ELEMENTS)
        .map(|_| json!({"id": "api", "dx": 1}))
        .collect();
    let error = refused(&scene, &Value::Array(many).to_string());
    assert_eq!(
        error,
        BoardError::TooLarge {
            what: "request",
            limit: limits::MAX_REQUEST_ELEMENTS
        }
    );
}
