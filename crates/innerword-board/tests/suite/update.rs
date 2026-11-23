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

/// Two shapes and an elbow arrow between them as the writer draws one in Excalidraw 0.18.1:
/// an orthogonal route, bindings with `fixedPoint`, a label; `in_frame` names what a frame
/// around `a` and `b` holds besides them (the arrow, or nothing).
fn elbow_board(arrow_in_frame: bool) -> Scene {
    let base = drawn(
        r#"[{"type":"rectangle","id":"a","x":-100,"y":-50,"width":100,"height":100},
           {"type":"rectangle","id":"b","x":250,"y":100,"width":100,"height":100},
           {"type":"rectangle","id":"c","x":600,"y":0,"width":100,"height":100}]"#,
    );
    let mut value: Value = serde_json::from_str(&base.to_stored()).unwrap();
    let elements = value["elements"].as_array_mut().unwrap();
    for element in elements.iter_mut() {
        if element["id"] == "a" || element["id"] == "b" {
            element["boundElements"]
                .as_array_mut()
                .unwrap()
                .push(json!({"id": "elbow", "type": "arrow"}));
            element["frameId"] = json!("f");
        }
    }
    elements.push(json!({
        "id": "f", "type": "frame", "x": -120, "y": -70, "width": 500, "height": 290,
        "name": null, "version": 1, "versionNonce": 5, "index": "a3", "frameId": null,
        "boundElements": []
    }));
    elements.push(json!({
        "id": "elbow", "type": "arrow", "x": 0, "y": 0, "width": 250, "height": 150,
        "points": [[0, 0], [150, 0], [150, 150], [250, 150]], "elbowed": true,
        "startBinding": {"elementId": "a", "focus": 0, "gap": 0, "fixedPoint": [1.0001, 0.5001]},
        "endBinding": {"elementId": "b", "focus": 0, "gap": 0, "fixedPoint": [-0.0001, 0.5001]},
        "fixedSegments": [{"start": [150, 0], "end": [150, 150], "index": 2}],
        "startIsSpecial": false, "endIsSpecial": false,
        "startArrowhead": null, "endArrowhead": "arrow", "angle": 0,
        "version": 7, "versionNonce": 11, "index": "a4",
        "frameId": if arrow_in_frame { json!("f") } else { Value::Null },
        "boundElements": [{"id": "elbow-label", "type": "text"}]
    }));
    elements.push(json!({
        "id": "elbow-label", "type": "text", "x": 130, "y": 62.5, "width": 40, "height": 25,
        "text": "calls", "originalText": "calls", "fontSize": 20, "fontFamily": 5,
        "textAlign": "center", "verticalAlign": "middle", "lineHeight": 1.25,
        "autoResize": true, "containerId": "elbow", "angle": 0,
        "version": 3, "versionNonce": 13, "index": "a5",
        "frameId": if arrow_in_frame { json!("f") } else { Value::Null },
        "boundElements": null
    }));
    Scene::from_stored(&value.to_string()).unwrap().scene
}

#[test]
fn an_elbow_arrow_keeps_its_route_and_its_bindings_when_what_it_joins_changes() {
    let scene = elbow_board(false);
    let before = scene.element("elbow").unwrap().clone();
    let label = scene.element("elbow-label").unwrap().clone();
    for change in [
        r#"[{"id":"a","dx":40}]"#,
        r#"[{"id":"a","x":500,"y":500}]"#,
        r#"[{"id":"b","width":300}]"#,
        r#"[{"id":"a","angle":0.5}]"#,
        r#"[{"id":"b","text":"A label long enough to make the shape grow taller"}]"#,
        r#"[{"id":"a","dx":40},{"id":"b","dx":40}]"#,
    ] {
        let (after, changed) = changed(&scene, change);
        // Neither its points nor its bindings (their fixedPoint above all) nor its version:
        // Excalidraw routes it again when the writer next moves what it joins.
        assert_eq!(after.element("elbow"), Some(&before), "{change}");
        assert_eq!(after.element("elbow-label"), Some(&label), "{change}");
        assert!(!changed.contains(&"elbow".to_string()), "{change}");
    }
    let (after, _) = changed(&scene, r#"[{"id":"a","dx":40}]"#);
    let arrow = after.element("elbow").unwrap();
    assert_eq!(
        arrow.points(),
        vec![(0.0, 0.0), (150.0, 0.0), (150.0, 150.0), (250.0, 150.0)]
    );
    assert_eq!(
        arrow.json()["startBinding"],
        json!({"elementId": "a", "focus": 0, "gap": 0, "fixedPoint": [1.0001, 0.5001]})
    );
}

#[test]
fn an_elbow_arrow_moves_whole_with_a_frame_that_holds_both_its_ends() {
    for arrow_in_frame in [false, true] {
        let scene = elbow_board(arrow_in_frame);
        let (after, changed) = changed(&scene, r#"[{"id":"f","dx":30,"dy":-20}]"#);
        let (before, arrow) = (
            scene.element("elbow").unwrap(),
            after.element("elbow").unwrap(),
        );
        // Moved once, by as much as the frame, route and bindings as they were.
        assert_eq!((arrow.x(), arrow.y()), (30.0, -20.0), "{arrow_in_frame}");
        assert_eq!(arrow.points(), before.points());
        for key in ["startBinding", "endBinding", "fixedSegments", "elbowed"] {
            assert_eq!(arrow.json()[key], before.json()[key], "{key}");
        }
        assert_eq!(arrow.version(), before.version() + 1);
        assert!(changed.contains(&"elbow".to_string()));
        let (label, moved) = (
            scene.element("elbow-label").unwrap(),
            after.element("elbow-label").unwrap(),
        );
        assert_eq!((moved.x(), moved.y()), (label.x() + 30.0, label.y() - 20.0));
        assert!(changed.contains(&"elbow-label".to_string()));
    }
}

#[test]
fn the_route_and_the_ends_of_an_elbow_arrow_are_not_changed_by_agents() {
    let scene = elbow_board(false);
    for (change, field) in [
        (r#"[{"id":"elbow","end":{"id":"c"}}]"#, "end"),
        (r#"[{"id":"elbow","start":null}]"#, "start"),
        (r#"[{"id":"elbow","points":[[0,0],[100,100]]}]"#, "points"),
        (r#"[{"id":"elbow","width":400}]"#, "width"),
        (r#"[{"id":"elbow","height":10}]"#, "height"),
        (r#"[{"id":"elbow","angle":1}]"#, "angle"),
        (r#"[{"id":"elbow","dx":10}]"#, "dx"),
        (r#"[{"id":"elbow","x":10,"y":10}]"#, "x"),
    ] {
        let error = refused(&scene, change);
        assert!(
            matches!(&error, BoardError::Invalid { field: Some(f), message, .. }
                if f == field && message.contains("elbow arrow")),
            "{change}: {error:?}"
        );
    }
    // Its style, its label and its arrowheads are the agent's to change; its route stays.
    let (after, _) = changed(
        &scene,
        r##"[{"id":"elbow","strokeColor":"#e03131","text":"reads from","endArrowhead":"triangle"}]"##,
    );
    let (before, arrow) = (
        scene.element("elbow").unwrap(),
        after.element("elbow").unwrap(),
    );
    assert_eq!(arrow.json()["strokeColor"], "#e03131");
    assert_eq!(arrow.json()["endArrowhead"], "triangle");
    assert_eq!(
        after.element("elbow-label").unwrap().text(),
        Some("reads from")
    );
    for key in [
        "x",
        "y",
        "points",
        "startBinding",
        "endBinding",
        "fixedSegments",
    ] {
        assert_eq!(arrow.json()[key], before.json()[key], "{key}");
    }
    // One that is bound to nothing moves by itself.
    let mut value: Value = serde_json::from_str(&scene.to_stored()).unwrap();
    for element in value["elements"].as_array_mut().unwrap() {
        if element["id"] == "elbow" {
            element["startBinding"] = Value::Null;
            element["endBinding"] = Value::Null;
        }
    }
    let loose = Scene::from_stored(&value.to_string()).unwrap().scene;
    let (after, _) = changed(&loose, r#"[{"id":"elbow","dx":10}]"#);
    let arrow = after.element("elbow").unwrap();
    assert_eq!((arrow.x(), arrow.y()), (10.0, 0.0));
    assert_eq!(arrow.points(), loose.element("elbow").unwrap().points());
}

#[test]
fn a_binding_keeps_the_fields_it_had() {
    let scene = board();
    let text = scene.to_stored().replace(
        r#""startBinding":{"elementId":"api","#,
        r#""startBinding":{"elementId":"api","later":{"kept":true},"#,
    );
    assert_ne!(text, scene.to_stored());
    let scene = Scene::from_stored(&text).unwrap().scene;
    let (after, _) = changed(&scene, r#"[{"id":"api","dy":60}]"#);
    let binding = &after.element("reads").unwrap().json()["startBinding"];
    assert_eq!(binding["later"], json!({"kept": true}));
    assert_eq!(binding["elementId"], "api");
    assert_eq!(binding["gap"], 1);
    // The end was put back on the moved outline all the same.
    assert_ne!(
        after.element("reads").unwrap().y(),
        scene.element("reads").unwrap().y()
    );
}

#[test]
fn a_text_the_writer_made_narrower_keeps_its_width() {
    use innerword_board::metrics;
    // A free text as Excalidraw leaves it once the writer dragged its box narrower.
    let drawn_text = drawn(
        r#"[{"type":"text","id":"t","x":10,"y":20,"text":"one two three four five six seven"}]"#,
    );
    let mut value: Value = serde_json::from_str(&drawn_text.to_stored()).unwrap();
    let original = "one two three four five six seven";
    let wrapped = metrics::wrap(original, 5, 20.0, 90.0);
    let element = &mut value["elements"][0];
    element["autoResize"] = json!(false);
    element["width"] = json!(90);
    element["text"] = json!(wrapped);
    element["height"] = json!(metrics::text_size(&wrapped, 5, 20.0).1);
    let scene = Scene::from_stored(&value.to_string()).unwrap().scene;

    let longer = "eight nine ten eleven twelve thirteen";
    let (after, _) = changed(&scene, &format!(r#"[{{"id":"t","text":"{longer}"}}]"#));
    let t = after.element("t").unwrap();
    assert_eq!(t.width(), 90.0, "its width stays");
    let rewrapped = metrics::wrap(longer, 5, 20.0, 90.0);
    assert!(rewrapped.lines().count() > 2, "{rewrapped}");
    assert_eq!(t.text(), Some(rewrapped.as_str()));
    assert_eq!(t.json()["originalText"], longer);
    assert_eq!(t.height(), metrics::text_size(&rewrapped, 5, 20.0).1);
    assert_eq!((t.x(), t.y()), (10.0, 20.0));
    assert_eq!(t.json()["autoResize"], false);

    // A larger font: wrapped again to the same width.
    let (after, _) = changed(&scene, r#"[{"id":"t","fontSize":28}]"#);
    let t = after.element("t").unwrap();
    assert_eq!(t.width(), 90.0);
    assert_eq!(
        t.text(),
        Some(metrics::wrap(original, 5, 28.0, 90.0).as_str())
    );
}

#[test]
fn a_turned_text_keeps_the_corner_its_alignment_names() {
    // The top corner on the side of its alignment stays where it is on the canvas.
    let corner = |e: &innerword_board::Element, side: f64| {
        let (w, h, a) = (e.width(), e.height(), e.angle());
        let (cx, cy) = (e.x() + w / 2.0, e.y() + h / 2.0);
        let (px, py) = (side * w / 2.0, -h / 2.0);
        (
            cx + px * a.cos() - py * a.sin(),
            cy + px * a.sin() + py * a.cos(),
        )
    };
    for (align, side) in [("left", -1.0), ("center", 0.0), ("right", 1.0)] {
        let scene = drawn(&format!(
            r#"[{{"type":"text","id":"t","x":40,"y":60,"angle":0.7,"textAlign":"{align}","text":"short"}}]"#
        ));
        let (after, _) = changed(
            &scene,
            r#"[{"id":"t","text":"a good deal longer\nand on two lines"}]"#,
        );
        let (before, now) = (
            corner(scene.element("t").unwrap(), side),
            corner(after.element("t").unwrap(), side),
        );
        assert!(
            (before.0 - now.0).abs() < 1e-6 && (before.1 - now.1).abs() < 1e-6,
            "{align}: {before:?} {now:?}"
        );
    }
}

/// A board at the size the limits allow an agent to work on: 1,900 labelled shapes and
/// 7,600 arrows bound to them (11,400 elements), and a request of 2,000 moves. Each change
/// once walked the board several times (7 s in a release build); with the index it is a
/// moment even in a debug build.
#[test]
fn two_thousand_changes_on_a_large_board_take_a_moment() {
    let shapes = 1_900;
    let mut elements: Vec<Value> = Vec::new();
    for n in 0..shapes {
        let (x, y) = ((n % 50) as f64 * 300.0, (n / 50) as f64 * 300.0);
        elements.push(
            json!({"type": "rectangle", "id": format!("s{n}"), "x": x, "y": y,
            "width": 120, "height": 80, "label": {"text": format!("Shape {n}")}}),
        );
    }
    let mut scene = Scene::empty();
    for chunk in elements.chunks(1_000) {
        let request: Vec<innerword_board::Skeleton> =
            serde_json::from_value(Value::Array(chunk.to_vec())).unwrap();
        add(&mut scene, &request, &Pictures::new(), &mut making()).unwrap();
    }
    let mut arrows: Vec<Value> = Vec::new();
    for n in 0..shapes {
        for step in [1, 2, 50, 51] {
            let to = (n + step) % shapes;
            arrows.push(
                json!({"type": "arrow", "id": format!("a{n}-{step}"), "x": 0, "y": 0,
                "start": {"id": format!("s{n}")}, "end": {"id": format!("s{to}")}}),
            );
        }
    }
    for chunk in arrows.chunks(2_000) {
        let request: Vec<innerword_board::Skeleton> =
            serde_json::from_value(Value::Array(chunk.to_vec())).unwrap();
        add(&mut scene, &request, &Pictures::new(), &mut making()).unwrap();
    }
    assert_eq!(scene.elements().len(), 11_400);
    let moves: Vec<Value> = (0..2_000)
        .map(|n| json!({"id": format!("s{}", n % shapes), "dx": 3}))
        .collect();
    let started = std::time::Instant::now();
    let done = update(
        &mut scene,
        &changes(&Value::Array(moves).to_string()),
        &mut later(),
    )
    .unwrap();
    let took = started.elapsed();
    // Every shape, its label and the arrows bound to it: the whole board.
    assert_eq!(done.changed.len(), 11_400);
    assert!(
        took < std::time::Duration::from_secs(30),
        "2,000 changes took {took:?}"
    );
    eprintln!("2,000 changes on 11,400 elements: {took:?}");
}
