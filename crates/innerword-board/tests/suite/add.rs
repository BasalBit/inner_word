//! `add`: every type and field of a skeleton, every refusal (section 7.2).

use innerword_board::{BoardError, HashedMaking, Pictures, Scene, add, limits, picture_paths};
use serde_json::{Value, json};

use super::common::*;
use super::scene::picture_map;

fn refused(json: &str) -> BoardError {
    let mut scene = Scene::empty();
    add(
        &mut scene,
        &skeletons(json),
        &Pictures::new(),
        &mut making(),
    )
    .unwrap_err()
}

fn refused_on(scene: &Scene, json: &str) -> BoardError {
    let mut scene = scene.clone();
    let before = scene.clone();
    let error = add(
        &mut scene,
        &skeletons(json),
        &Pictures::new(),
        &mut making(),
    )
    .unwrap_err();
    assert_eq!(scene, before, "a refused request changes nothing");
    error
}

fn invalid_at(error: &BoardError) -> (Option<usize>, Option<String>) {
    match error {
        BoardError::Invalid { element, field, .. } => (*element, field.clone()),
        other => panic!("not Invalid: {other:?}"),
    }
}

#[test]
fn a_shape_has_every_field_excalidraw_writes() {
    let scene = drawn(r#"[{"type":"rectangle","id":"box","x":10,"y":20}]"#);
    let element = scene.element("box").unwrap().json().clone();
    let keys: Vec<&str> = element.keys().map(String::as_str).collect();
    let mut expected = vec![
        "angle",
        "backgroundColor",
        "boundElements",
        "fillStyle",
        "frameId",
        "groupIds",
        "height",
        "id",
        "index",
        "isDeleted",
        "link",
        "locked",
        "opacity",
        "roughness",
        "roundness",
        "seed",
        "strokeColor",
        "strokeStyle",
        "strokeWidth",
        "type",
        "updated",
        "version",
        "versionNonce",
        "width",
        "x",
        "y",
    ];
    expected.sort();
    let mut keys = keys;
    keys.sort();
    assert_eq!(keys, expected);
    assert_eq!(element["width"], 100);
    assert_eq!(element["height"], 100);
    assert_eq!(element["strokeColor"], "#1e1e1e");
    assert_eq!(element["backgroundColor"], "transparent");
    assert_eq!(element["fillStyle"], "solid");
    assert_eq!(element["strokeWidth"], 2);
    assert_eq!(element["roughness"], 1);
    assert_eq!(element["opacity"], 100);
    assert_eq!(element["roundness"], Value::Null);
    assert_eq!(element["version"], 1);
    assert_eq!(element["isDeleted"], false);
    assert_eq!(element["boundElements"], json!([]));
    assert_eq!(element["updated"], 1_795_432_196_771i64);
    assert_eq!(element["index"], "a0");
    let seed = element["seed"].as_i64().unwrap();
    assert!((0..1 << 31).contains(&seed));
}

#[test]
fn every_common_field_is_taken() {
    let scene = drawn(
        r##"[{"type":"diamond","id":"d","x":0,"y":0,"width":80,"height":60,"angle":-1.5707963267948966,
          "strokeColor":"#e03131","backgroundColor":"#ffc9c9","fillStyle":"cross-hatch","strokeWidth":4,
          "strokeStyle":"dotted","roughness":0,"opacity":50,"roundness":{"type":2},"groupIds":["g1","g2"],
          "link":"https://example.com/x","locked":true}]"##,
    );
    let d = scene.element("d").unwrap().json();
    assert!((d["angle"].as_f64().unwrap() - 3.0 * std::f64::consts::FRAC_PI_2).abs() < 1e-9);
    assert_eq!(d["strokeColor"], "#e03131");
    assert_eq!(d["backgroundColor"], "#ffc9c9");
    assert_eq!(d["fillStyle"], "cross-hatch");
    assert_eq!(d["strokeWidth"], 4);
    assert_eq!(d["strokeStyle"], "dotted");
    assert_eq!(d["roughness"], 0);
    assert_eq!(d["opacity"], 50);
    assert_eq!(d["roundness"], json!({"type": 2}));
    assert_eq!(d["groupIds"], json!(["g1", "g2"]));
    assert_eq!(d["link"], "https://example.com/x");
    assert_eq!(d["locked"], true);
}

#[test]
fn a_text_is_measured_and_placed_by_its_alignment() {
    let scene = drawn(
        r#"[{"type":"text","id":"t","x":100,"y":50,"text":"Hello\tthere\r\nsecond line","fontSize":30,"fontFamily":"Nunito"},
           {"type":"text","id":"c","x":300,"y":200,"text":"Centred","textAlign":"center","verticalAlign":"middle","fontFamily":8}]"#,
    );
    let t = scene.element("t").unwrap().json();
    assert_eq!(t["text"], "Hello        there\nsecond line");
    assert_eq!(t["originalText"], t["text"]);
    assert_eq!(t["fontFamily"], 6);
    assert_eq!(t["fontSize"], 30);
    assert_eq!(t["lineHeight"], 1.35);
    assert_eq!(t["height"].as_f64(), Some(30.0 * 1.35 * 2.0));
    assert_eq!(t["autoResize"], true);
    assert_eq!(t["containerId"], Value::Null);
    assert_eq!(t["textAlign"], "left");
    assert_eq!(t["verticalAlign"], "top");
    assert_eq!(
        (t["x"].as_f64(), t["y"].as_f64()),
        (Some(100.0), Some(50.0))
    );
    let c = scene.element("c").unwrap();
    // Centred on the point given, as Excalidraw's newTextElement places it.
    assert!((c.x() + c.width() / 2.0 - 300.0).abs() < 1e-9);
    assert!((c.y() + c.height() / 2.0 - 200.0).abs() < 1e-9);
    assert_eq!(c.json()["lineHeight"], 1.25);
}

#[test]
fn lines_arrows_and_freedraw() {
    let scene = drawn(
        r#"[{"type":"line","id":"l","x":10,"y":10,"points":[[5,5],[105,5],[105,55]]},
           {"type":"arrow","id":"a","x":0,"y":200,"width":120,"height":40,"startArrowhead":"dot","endArrowhead":null},
           {"type":"arrow","id":"plain","x":0,"y":300,"width":100},
           {"type":"freedraw","id":"f","x":0,"y":400,"points":[[0,30],[10,0],[20,30]]}]"#,
    );
    let l = scene.element("l").unwrap().json();
    // Points start at 0, 0; the element moved by as much.
    assert_eq!((l["x"].as_f64(), l["y"].as_f64()), (Some(15.0), Some(15.0)));
    assert_eq!(l["points"], json!([[0, 0], [100, 0], [100, 50]]));
    assert_eq!(
        (l["width"].as_f64(), l["height"].as_f64()),
        (Some(100.0), Some(50.0))
    );
    assert_eq!(l["startArrowhead"], Value::Null);
    assert_eq!(l["endArrowhead"], Value::Null);
    assert_eq!(l["startBinding"], Value::Null);
    assert_eq!(l["lastCommittedPoint"], Value::Null);
    assert!(l.get("elbowed").is_none());
    let a = scene.element("a").unwrap().json();
    assert_eq!(a["startArrowhead"], "dot");
    assert_eq!(a["endArrowhead"], Value::Null);
    assert_eq!(a["elbowed"], false);
    // A new arrow loses half a pixel at each end, as convertToExcalidrawElements leaves it.
    assert_eq!((a["x"].as_f64(), a["y"].as_f64()), (Some(0.5), Some(200.5)));
    assert_eq!(a["points"], json!([[0, 0], [119, 39]]));
    let plain = scene.element("plain").unwrap().json();
    assert_eq!(plain["endArrowhead"], "arrow");
    assert_eq!(plain["points"], json!([[0, 0], [99, 0]]));
    let f = scene.element("f").unwrap().json();
    assert_eq!(f["points"], json!([[0, 30], [10, 0], [20, 30]]));
    assert_eq!(
        (f["width"].as_f64(), f["height"].as_f64()),
        (Some(20.0), Some(30.0))
    );
    assert_eq!(f["pressures"], json!([]));
    assert_eq!(f["simulatePressure"], true);
}

#[test]
fn an_arrow_binds_to_the_edges_of_a_rectangle_an_ellipse_and_a_diamond() {
    let scene = drawn(
        r#"[{"type":"rectangle","id":"r","x":0,"y":0,"width":100,"height":100},
           {"type":"ellipse","id":"e","x":300,"y":0,"width":100,"height":100},
           {"type":"diamond","id":"d","x":300,"y":300,"width":100,"height":100},
           {"type":"arrow","id":"re","x":50,"y":50,"width":300,"start":{"id":"r"},"end":{"id":"e"}},
           {"type":"arrow","id":"rd","x":60,"y":60,"points":[[0,0],[300,280]],"start":{"id":"r"},"end":{"id":"d"}}]"#,
    );
    // Rectangle to ellipse, on the line through both centres: from x 100 to x 300, half a
    // pixel in at each end.
    let re = scene.element("re").unwrap();
    assert_eq!((re.x(), re.y()), (100.5, 50.0));
    assert_eq!(re.points(), vec![(0.0, 0.0), (199.0, 0.0)]);
    let binding = re.json()["startBinding"].clone();
    assert_eq!(binding, json!({"elementId": "r", "focus": 0, "gap": 1}));
    assert_eq!(
        re.json()["endBinding"],
        json!({"elementId": "e", "focus": 0, "gap": 1})
    );
    // Both shapes list the arrow.
    for id in ["r", "e"] {
        assert!(
            scene
                .element(id)
                .unwrap()
                .bound_elements()
                .contains(&("re", "arrow")),
            "{id}"
        );
    }
    // Rectangle to diamond along the line given: the end is on the diamond's slanted edge.
    let rd = scene.element("rd").unwrap();
    let points = rd.points();
    let end = (rd.x() + points[1].0 + 0.5, rd.y() + points[1].1 + 0.5);
    let on_outline = (end.0 - 350.0).abs() / 50.0 + (end.1 - 350.0).abs() / 50.0;
    assert!((on_outline - 1.0).abs() < 1e-9, "{end:?}");
    let start = (rd.x() - 0.5, rd.y() - 0.5);
    assert!(
        start.0 == 100.0 || start.1 == 100.0,
        "{start:?} on the rectangle's edge"
    );
    // The line passes beside the diamond's centre: a focus, as Excalidraw measures it.
    let focus = rd.json()["endBinding"]["focus"].as_f64().unwrap();
    assert!(focus != 0.0 && focus.abs() < 1.0, "{focus}");
    assert_eq!(rd.json()["endBinding"]["gap"], 1);
}

#[test]
fn an_arrow_whose_line_misses_its_element_is_aimed_at_its_centre() {
    let scene = drawn(
        r#"[{"type":"ellipse","id":"e","x":400,"y":300,"width":100,"height":100},
           {"type":"arrow","id":"a","x":0,"y":0,"width":100,"end":{"id":"e"}}]"#,
    );
    let a = scene.element("a").unwrap();
    let points = a.points();
    let end = (a.x() + points[1].0 + 0.5, a.y() + points[1].1 + 0.5);
    // On the ellipse, on the line from the start to the centre.
    let on = ((end.0 - 450.0) / 50.0).powi(2) + ((end.1 - 350.0) / 50.0).powi(2);
    assert!((on - 1.0).abs() < 1e-6, "{end:?}");
    assert!(((end.1) / (end.0) - 350.0 / 450.0).abs() < 1e-2);
    assert_eq!(a.json()["endBinding"]["focus"], 0);
    assert_eq!(a.json()["startBinding"], Value::Null);
}

#[test]
fn an_arrow_binds_to_an_element_already_on_the_whiteboard() {
    let mut scene =
        drawn(r#"[{"type":"rectangle","id":"old","x":0,"y":0,"width":100,"height":100}]"#);
    let before = scene.element("old").unwrap().version();
    add(
        &mut scene,
        &skeletons(
            r#"[{"type":"arrow","id":"a","x":100,"y":50,"width":200,"start":{"id":"old"}}]"#,
        ),
        &Pictures::new(),
        &mut making(),
    )
    .unwrap();
    let old = scene.element("old").unwrap();
    assert_eq!(old.version(), before + 1);
    assert!(old.bound_elements().contains(&("a", "arrow")));
    assert_eq!(scene.element("a").unwrap().binding("start"), Some("old"));
}

#[test]
fn a_label_is_wrapped_and_its_shape_grown() {
    let scene = drawn(
        r##"[{"type":"rectangle","id":"narrow","x":0,"y":0,"width":80,"height":40,"label":{"text":"A label of many words"}},
           {"type":"ellipse","id":"fit","x":0,"y":200,"label":{"text":"Postgres"}},
           {"type":"diamond","id":"dfit","x":0,"y":400,"label":{"text":"Yes"}},
           {"type":"rectangle","id":"styled","x":300,"y":0,"width":200,"height":100,"strokeColor":"#1971c2",
            "label":{"text":"Left top","textAlign":"left","verticalAlign":"top","fontSize":16,"fontFamily":"Comic Shanns"}}]"##,
    );
    let narrow = scene.element("narrow").unwrap();
    let label_id = narrow
        .bound_elements()
        .iter()
        .find(|(_, k)| *k == "text")
        .unwrap()
        .0
        .to_string();
    let label = scene.element(&label_id).unwrap();
    let text = label.text().unwrap();
    assert!(text.lines().count() > 1, "{text:?}");
    assert_eq!(text.replace('\n', " "), "A label of many words");
    assert_eq!(label.json()["originalText"], "A label of many words");
    // Taller than 40 for its lines, the width kept.
    assert_eq!(narrow.width(), 80.0);
    assert!(narrow.height() > 40.0);
    assert_eq!(narrow.height(), (label.height()).ceil() + 10.0);
    assert!(label.width() <= 80.0 - 10.0 + 1e-9);
    assert_eq!(label.container_id(), Some("narrow"));
    assert_eq!(label.json()["textAlign"], "center");
    assert_eq!(label.json()["verticalAlign"], "middle");
    // Centred in the room the label has.
    assert!((label.x() + label.width() / 2.0 - 40.0).abs() < 1e-9);
    assert!((label.y() + label.height() / 2.0 - narrow.height() / 2.0).abs() < 1e-9);
    // A shape without a size fits its label, as Excalidraw sizes it.
    let fit = scene.element("fit").unwrap();
    let fit_label = scene.element(fit.bound_elements()[0].0).unwrap();
    let expected = ((fit_label.width().ceil() + 10.0) / std::f64::consts::SQRT_2 * 2.0).round();
    assert_eq!(fit.width(), expected);
    let dfit = scene.element("dfit").unwrap();
    let d_label = scene.element(dfit.bound_elements()[0].0).unwrap();
    assert_eq!(dfit.width(), 2.0 * (d_label.width().ceil() + 10.0));
    // A label's own style.
    let styled = scene.element("styled").unwrap();
    let s_label = scene.element(styled.bound_elements()[0].0).unwrap();
    assert_eq!((s_label.x(), s_label.y()), (305.0, 5.0));
    assert_eq!(s_label.json()["fontFamily"], 8);
    assert_eq!(s_label.json()["strokeColor"], "#1971c2");
}

#[test]
fn an_arrow_s_label_sits_at_its_middle() {
    let scene =
        drawn(r#"[{"type":"arrow","id":"a","x":0,"y":0,"width":300,"label":{"text":"goes"}}]"#);
    let a = scene.element("a").unwrap();
    let label = scene.element(a.bound_elements()[0].0).unwrap();
    assert!((label.x() + label.width() / 2.0 - 150.0).abs() < 1e-9);
    assert!((label.y() + label.height() / 2.0).abs() < 1e-9);
    assert_eq!(label.json()["angle"], 0);
}

#[test]
fn a_frame_holds_its_children_and_is_sized_from_them() {
    let scene = drawn(
        r#"[{"type":"rectangle","id":"a","x":100,"y":100,"width":100,"height":50,"label":{"text":"A"}},
           {"type":"rectangle","id":"b","x":300,"y":200,"width":100,"height":50},
           {"type":"arrow","id":"ab","x":200,"y":125,"start":{"id":"a"},"end":{"id":"b"}},
           {"type":"frame","id":"f","children":["a","b"],"name":"Group"},
           {"type":"frame","id":"g","x":0,"y":500,"width":300,"height":200}]"#,
    );
    let f = scene.element("f").unwrap();
    assert_eq!(
        (f.x(), f.y(), f.width(), f.height()),
        (90.0, 90.0, 320.0, 170.0)
    );
    assert_eq!(f.name(), Some("Group"));
    // The children, their labels and the arrows bound to them are in the frame.
    for id in ["a", "b", "ab"] {
        assert_eq!(scene.element(id).unwrap().frame_id(), Some("f"), "{id}");
    }
    let label = scene.element("a").unwrap().bound_elements()[0]
        .0
        .to_string();
    assert_eq!(scene.element(&label).unwrap().frame_id(), Some("f"));
    let g = scene.element("g").unwrap();
    assert_eq!(
        (g.x(), g.y(), g.width(), g.height()),
        (0.0, 500.0, 300.0, 200.0)
    );
    assert_eq!(g.json()["name"], Value::Null);
}

fn pictures() -> Pictures {
    let mut map = picture_map(
        "photo.jpg",
        "3f2a9c1b5d6e7f80a1b2c3d4e5f60718",
        "image/jpeg",
        (2880, 1920),
        144,
    );
    map.extend(picture_map(
        "wide.png",
        "0123456789abcdef0123456789abcdef",
        "image/png",
        (4000, 1000),
        72,
    ));
    map.extend(picture_map(
        "small.svg",
        "ffffffffffffffffffffffffffffffff",
        "image/svg+xml",
        (200, 100),
        72,
    ));
    map
}

#[test]
fn a_picture_has_its_own_size_or_the_one_given() {
    let mut scene = Scene::empty();
    let made = add(
        &mut scene,
        &skeletons(
            r#"[{"type":"image","id":"retina","x":0,"y":0,"path":"photo.jpg"},
               {"type":"image","id":"wide","x":0,"y":0,"path":"wide.png"},
               {"type":"image","id":"small","x":0,"y":0,"path":"small.svg"},
               {"type":"image","id":"w","x":0,"y":0,"path":"photo.jpg","width":300},
               {"type":"image","id":"h","x":0,"y":0,"path":"wide.png","height":50},
               {"type":"image","id":"both","x":0,"y":0,"path":"small.svg","width":10,"height":90,"angle":1,"opacity":40,"link":"https://x.org","locked":true,"groupIds":["g"]}]"#,
        ),
        &pictures(),
        &mut making(),
    )
    .unwrap();
    assert_eq!(made.elements.len(), 6);
    let size = |id: &str| {
        let e = scene.element(id).unwrap();
        (e.width(), e.height())
    };
    assert_eq!(size("retina"), (1440.0, 960.0));
    assert_eq!(size("wide"), (1440.0, 360.0));
    assert_eq!(size("small"), (200.0, 100.0));
    assert_eq!(size("w"), (300.0, 200.0));
    assert_eq!(size("h"), (200.0, 50.0));
    assert_eq!(size("both"), (10.0, 90.0));
    let element = scene.element("retina").unwrap().json();
    assert_eq!(element["fileId"], "3f2a9c1b5d6e7f80a1b2c3d4e5f60718");
    assert_eq!(element["status"], "saved");
    assert_eq!(element["scale"], json!([1, 1]));
    assert_eq!(element["crop"], Value::Null);
    assert_eq!(element["strokeColor"], "transparent");
    // One entry per picture, named by its stored bytes, though two elements use it.
    let files = scene.files();
    assert_eq!(files.len(), 3);
    let entry = &files["3f2a9c1b5d6e7f80a1b2c3d4e5f60718"];
    assert_eq!(entry.mime_type, "image/jpeg");
    assert_eq!(
        entry.file.as_ref().unwrap().as_str(),
        "3f2a9c1b5d6e7f80a1b2c3d4e5f60718"
    );
    assert_eq!(entry.created, 1_795_432_196_771);
    assert_eq!(
        files["ffffffffffffffffffffffffffffffff"].mime_type,
        "image/svg+xml"
    );
    assert_eq!(scene.stored_files().len(), 3);
}

#[test]
fn the_paths_to_read_first() {
    let list = skeletons(
        r#"[{"type":"image","path":"a.png","x":0,"y":0},{"type":"rectangle","x":0,"y":0},
           {"type":"image","path":"b.heic","x":0,"y":0},{"type":"image","path":"a.png","x":9,"y":9}]"#,
    );
    assert_eq!(picture_paths(&list), ["a.png", "b.heic"]);
}

#[test]
fn a_picture_is_refused_without_its_file_or_with_a_fill() {
    let mut scene = Scene::empty();
    let error = add(
        &mut scene,
        &skeletons(r#"[{"type":"rectangle","x":0,"y":0},{"type":"image","x":0,"y":0,"path":"nowhere.png"}]"#),
        &pictures(),
        &mut making(),
    )
    .unwrap_err();
    assert_eq!(invalid_at(&error), (Some(1), Some("path".into())));
    for field in [
        "strokeColor",
        "backgroundColor",
        "fillStyle",
        "strokeWidth",
        "strokeStyle",
        "roughness",
    ] {
        let json =
            format!(r#"[{{"type":"image","x":0,"y":0,"path":"photo.jpg","{field}":"solid"}}]"#);
        let error = add(&mut scene, &skeletons(&json), &pictures(), &mut making()).unwrap_err();
        assert_eq!(invalid_at(&error), (Some(0), Some(field.into())), "{field}");
    }
    let error = add(
        &mut scene,
        &skeletons(r#"[{"type":"image","x":0,"y":0}]"#),
        &pictures(),
        &mut making(),
    )
    .unwrap_err();
    assert_eq!(invalid_at(&error), (Some(0), Some("path".into())));
    assert!(scene.is_empty());
}

#[test]
fn each_refusal_names_the_element_and_the_field() {
    let cases: &[(&str, usize, &str)] = &[
        (r#"[{"type":"hexagon","x":0,"y":0}]"#, 0, "type"),
        (r#"[{"type":"iframe","x":0,"y":0}]"#, 0, "type"),
        (r#"[{"type":"embeddable","x":0,"y":0}]"#, 0, "type"),
        (r#"[{"type":"magicframe","x":0,"y":0}]"#, 0, "type"),
        (r#"[{"type":"selection","x":0,"y":0}]"#, 0, "type"),
        (r#"[{"x":0,"y":0}]"#, 0, "type"),
        (
            r#"[{"type":"rectangle","x":0,"y":0},{"type":"rectangle","x":0,"y":0,"colour":"red"}]"#,
            1,
            "colour",
        ),
        (
            r#"[{"type":"rectangle","x":0,"y":0,"customData":{}}]"#,
            0,
            "customData",
        ),
        (
            r#"[{"type":"rectangle","x":0,"y":0,"fileId":"x"}]"#,
            0,
            "fileId",
        ),
        (
            r#"[{"type":"rectangle","x":0,"y":0,"status":"saved"}]"#,
            0,
            "status",
        ),
        (
            r#"[{"type":"image","x":0,"y":0,"scale":[1,1]}]"#,
            0,
            "scale",
        ),
        (r#"[{"type":"image","x":0,"y":0,"crop":null}]"#, 0, "crop"),
        (
            r#"[{"type":"arrow","x":0,"y":0,"width":10,"elbowed":true}]"#,
            0,
            "elbowed",
        ),
        (
            r#"[{"type":"text","x":0,"y":0,"text":"a","width":10}]"#,
            0,
            "width",
        ),
        (
            r#"[{"type":"text","x":0,"y":0,"text":"a","height":10}]"#,
            0,
            "height",
        ),
        (
            r#"[{"type":"text","x":0,"y":0,"text":"a","lineHeight":1.2}]"#,
            0,
            "lineHeight",
        ),
        (
            r#"[{"type":"rectangle","id":"a","x":0,"y":0},{"type":"ellipse","id":"a","x":0,"y":0}]"#,
            1,
            "id",
        ),
        (
            r#"[{"type":"rectangle","id":"has space","x":0,"y":0}]"#,
            0,
            "id",
        ),
        (
            r#"[{"type":"rectangle","x":0,"y":0,"opacity":101}]"#,
            0,
            "opacity",
        ),
        (
            r#"[{"type":"rectangle","x":0,"y":0,"roughness":3}]"#,
            0,
            "roughness",
        ),
        (
            r#"[{"type":"rectangle","x":0,"y":0,"fillStyle":"dots"}]"#,
            0,
            "fillStyle",
        ),
        (
            r#"[{"type":"rectangle","x":0,"y":0,"strokeStyle":"wavy"}]"#,
            0,
            "strokeStyle",
        ),
        (
            r#"[{"type":"rectangle","x":0,"y":0,"strokeColor":"url(#x)"}]"#,
            0,
            "strokeColor",
        ),
        (
            r#"[{"type":"rectangle","x":0,"y":0,"strokeWidth":0}]"#,
            0,
            "strokeWidth",
        ),
        (
            r#"[{"type":"rectangle","x":0,"y":0,"link":"javascript:alert(1)"}]"#,
            0,
            "link",
        ),
        (r#"[{"type":"rectangle","x":2000000,"y":0}]"#, 0, "x"),
        (r#"[{"type":"rectangle","x":"10","y":0}]"#, 0, "x"),
        (r#"[{"type":"rectangle","y":0}]"#, 0, "x"),
        (
            r#"[{"type":"rectangle","x":0,"y":0,"roundness":{"type":7}}]"#,
            0,
            "roundness",
        ),
        (
            r#"[{"type":"rectangle","x":0,"y":0,"groupIds":"g"}]"#,
            0,
            "groupIds",
        ),
        (
            r#"[{"type":"rectangle","x":0,"y":0,"locked":"yes"}]"#,
            0,
            "locked",
        ),
        (
            r#"[{"type":"text","x":0,"y":0,"text":"a","fontFamily":"Comic Sans"}]"#,
            0,
            "fontFamily",
        ),
        (
            r#"[{"type":"text","x":0,"y":0,"text":"a","fontFamily":4}]"#,
            0,
            "fontFamily",
        ),
        (
            r#"[{"type":"text","x":0,"y":0,"text":"a","fontSize":0}]"#,
            0,
            "fontSize",
        ),
        (
            r#"[{"type":"text","x":0,"y":0,"text":"a","textAlign":"justify"}]"#,
            0,
            "textAlign",
        ),
        (r#"[{"type":"text","x":0,"y":0}]"#, 0, "text"),
        (
            r#"[{"type":"rectangle","x":0,"y":0,"label":{"text":"a","colour":"red"}}]"#,
            0,
            "label",
        ),
        (
            r#"[{"type":"rectangle","x":0,"y":0,"label":"a"}]"#,
            0,
            "label",
        ),
        (
            r#"[{"type":"rectangle","x":0,"y":0,"label":{"text":"a","fontSize":-1}}]"#,
            0,
            "label.fontSize",
        ),
        (
            r#"[{"type":"line","x":0,"y":0,"width":10,"start":{"id":"x"}}]"#,
            0,
            "start",
        ),
        (
            r#"[{"type":"line","x":0,"y":0,"width":10,"endArrowhead":"arrow"}]"#,
            0,
            "endArrowhead",
        ),
        (r#"[{"type":"arrow","x":0,"y":0}]"#, 0, "points"),
        (
            r#"[{"type":"arrow","x":0,"y":0,"width":10,"endArrowhead":"star"}]"#,
            0,
            "endArrowhead",
        ),
        (
            r#"[{"type":"arrow","x":0,"y":0,"points":[[0,0]]}]"#,
            0,
            "points",
        ),
        (
            r#"[{"type":"arrow","x":0,"y":0,"points":[[0,0],["a",1]]}]"#,
            0,
            "points",
        ),
        (
            r#"[{"type":"rectangle","id":"r","x":0,"y":0},{"type":"arrow","x":0,"y":0,"width":10,"start":{"type":"rectangle"}}]"#,
            1,
            "start",
        ),
        (
            r#"[{"type":"rectangle","id":"r","x":0,"y":0},{"type":"arrow","x":0,"y":0,"width":10,"start":{"id":"r","x":4}}]"#,
            1,
            "start",
        ),
        (
            r#"[{"type":"line","id":"l","x":0,"y":0,"width":10},{"type":"arrow","x":0,"y":0,"width":10,"end":{"id":"l"}}]"#,
            1,
            "end",
        ),
        (
            r#"[{"type":"arrow","id":"self","x":0,"y":0,"width":10,"end":{"id":"self"}}]"#,
            0,
            "end",
        ),
        (
            r#"[{"type":"frame","id":"f1","x":0,"y":0,"width":9,"height":9},{"type":"frame","children":["f1"]}]"#,
            1,
            "children",
        ),
        (r#"[{"type":"frame","name":"no size"}]"#, 0, "children"),
        (r#"[{"type":"freedraw","x":0,"y":0}]"#, 0, "points"),
    ];
    for (json, index, field) in cases {
        let error = refused(json);
        assert_eq!(
            invalid_at(&error),
            (Some(*index), Some(field.to_string())),
            "{json}: {error}"
        );
        let text = error.to_string();
        assert!(
            text.starts_with(&format!("the element at index {index} ({field}): ")),
            "{text}"
        );
    }
}

#[test]
fn nothing_invisibly_small_is_made() {
    let cases: &[(&str, &str)] = &[
        (
            r#"[{"type":"line","x":0,"y":0,"points":[[0,0],[0.5,0.9]]}]"#,
            "points",
        ),
        (
            r#"[{"type":"arrow","x":0,"y":0,"points":[[0,0],[0.2,0],[0,0.3]]}]"#,
            "points",
        ),
        (
            r#"[{"type":"freedraw","x":0,"y":0,"points":[[0,0],[0.9,0.9]]}]"#,
            "points",
        ),
        (
            r#"[{"type":"rectangle","x":0,"y":0,"width":0.5,"height":100}]"#,
            "width",
        ),
        (
            r#"[{"type":"ellipse","x":0,"y":0,"width":100,"height":0}]"#,
            "height",
        ),
        (
            r#"[{"type":"frame","x":0,"y":0,"width":100,"height":0.9}]"#,
            "height",
        ),
        (r#"[{"type":"text","x":0,"y":0,"text":"   \n\t"}]"#, "text"),
        (r#"[{"type":"text","x":0,"y":0,"text":""}]"#, "text"),
        (
            r#"[{"type":"rectangle","x":0,"y":0,"label":{"text":"  "}}]"#,
            "label",
        ),
        (
            r#"[{"type":"arrow","x":0,"y":0,"width":0.5,"height":0.5}]"#,
            "points",
        ),
        (
            r#"[{"type":"line","x":0,"y":0,"width":0,"height":0}]"#,
            "points",
        ),
    ];
    for (json, field) in cases {
        let error = refused(json);
        assert_eq!(
            invalid_at(&error),
            (Some(0), Some(field.to_string())),
            "{json}: {error}"
        );
    }
}

#[test]
fn an_id_the_whiteboard_has_or_does_not_have() {
    let scene = drawn(
        r#"[{"type":"rectangle","id":"api","x":0,"y":0},{"type":"ellipse","id":"apis","x":200,"y":0}]"#,
    );
    let error = refused_on(&scene, r#"[{"type":"rectangle","id":"api","x":0,"y":0}]"#);
    assert_eq!(invalid_at(&error), (Some(0), Some("id".into())));
    // An end or a child the whiteboard does not have: NotFound, with the nearest ids first.
    let error = refused_on(
        &scene,
        r#"[{"type":"arrow","x":0,"y":0,"width":10,"end":{"id":"ap"}}]"#,
    );
    match &error {
        BoardError::NotFound { id, known } => {
            assert_eq!(id, "ap");
            assert_eq!(known, &["api".to_string(), "apis".to_string()]);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        error.to_string(),
        "the whiteboard has no element ap (it has api, apis)"
    );
    let error = refused_on(&scene, r#"[{"type":"frame","children":["api","nope"]}]"#);
    assert!(matches!(error, BoardError::NotFound { ref id, .. } if id == "nope"));
}

#[test]
fn the_limits_for_agents() {
    let many: Vec<Value> = (0..=limits::MAX_REQUEST_ELEMENTS)
        .map(|n| json!({"type":"rectangle","x":n,"y":0}))
        .collect();
    let error = refused(&Value::Array(many).to_string());
    assert_eq!(
        error,
        BoardError::TooLarge {
            what: "request",
            limit: 2_000
        }
    );
    assert_eq!(
        error.to_string(),
        "a request may hold at most 2,000 elements"
    );
    let long = "x".repeat(limits::MAX_TEXT_CHARS + 1);
    let error = refused(&json!([{"type":"text","x":0,"y":0,"text":long}]).to_string());
    assert_eq!(
        error,
        BoardError::TooLarge {
            what: "text",
            limit: limits::MAX_TEXT_CHARS
        }
    );
    // A whiteboard at the most elements an agent may leave takes none more.
    let elements: Vec<Value> = (0..limits::MAX_ELEMENTS)
        .map(|n| json!({"id": format!("e{n}"), "type": "rectangle", "x": n, "y": 0, "width": 1, "height": 1, "index": "a0"}))
        .collect();
    let full = Scene::from_stored(&json!({"elements": elements}).to_string())
        .unwrap()
        .scene;
    let error = refused_on(&full, r#"[{"type":"rectangle","x":0,"y":0}]"#);
    assert_eq!(
        error,
        BoardError::TooLarge {
            what: "elements",
            limit: limits::MAX_ELEMENTS
        }
    );
    // And a stored scene over 16 MB after the change is refused (a writer's drawing may be
    // that large; an agent's change to it is not taken).
    let heavy = json!({"elements": [{"id": "big", "type": "freedraw", "x": 0, "y": 0, "points": [[0, 0], [5, 5]],
                                     "note": "x".repeat(limits::MAX_SCENE_BYTES)}]});
    let heavy = Scene::from_stored(&heavy.to_string()).unwrap().scene;
    let error = refused_on(&heavy, r#"[{"type":"rectangle","x":0,"y":0}]"#);
    assert_eq!(
        error,
        BoardError::TooLarge {
            what: "whiteboard",
            limit: limits::MAX_SCENE_BYTES
        }
    );
    assert_eq!(
        error.to_string(),
        "the whiteboard would be larger than 16 MB"
    );
}

#[test]
fn the_same_request_gives_the_same_elements_and_another_board_other_ids() {
    let request = r#"[{"type":"rectangle","x":0,"y":0,"label":{"text":"A"}},{"type":"arrow","x":0,"y":0,"width":50}]"#;
    let once = drawn(request).to_stored();
    let twice = drawn(request).to_stored();
    assert_eq!(once, twice);
    let mut other = Scene::empty();
    add(
        &mut other,
        &skeletons(request),
        &Pictures::new(),
        &mut HashedMaking::new(&[b"13", b"stamp", b"request"], 1_795_432_196_771),
    )
    .unwrap();
    let a = drawn(request);
    assert_ne!(a.elements()[0].id(), other.elements()[0].id());
    assert_eq!(a.elements()[0].id().len(), 21);
}

#[test]
fn new_elements_go_on_top_in_index_order() {
    let mut scene = Scene::from_stored(
        &json!({"elements": [
            {"id": "x", "type": "rectangle", "index": "a5", "x": 0, "y": 0, "width": 9, "height": 9},
            {"id": "y", "type": "rectangle", "index": "Zz", "x": 0, "y": 0, "width": 9, "height": 9},
            {"id": "z", "type": "rectangle", "x": 0, "y": 0, "width": 9, "height": 9}
        ]})
        .to_string(),
    )
    .unwrap()
    .scene;
    let made = add(
        &mut scene,
        &skeletons(r#"[{"type":"rectangle","id":"n1","x":0,"y":0,"label":{"text":"L"}},{"type":"ellipse","id":"n2","x":0,"y":0}]"#),
        &Pictures::new(),
        &mut making(),
    )
    .unwrap();
    let ids: Vec<&str> = scene.elements().iter().map(|e| e.id()).collect();
    assert_eq!(&ids[..5], ["x", "y", "z", "n1", "n2"]);
    let indices: Vec<&str> = scene.elements()[3..]
        .iter()
        .map(|e| e.index().unwrap())
        .collect();
    assert_eq!(indices, ["a6", "a7", "a8"]);
    assert_eq!(made.elements[0].with.len(), 1);
    assert_eq!(made.elements[0].r#type, "rectangle");
    assert_eq!(ids[5], made.elements[0].with[0]);
}

#[test]
fn a_refusal_names_the_type_with_the_right_article() {
    let cases = [
        (r#"[{"type":"arrow","y":0,"width":10}]"#, "an arrow needs x"),
        (r#"[{"type":"ellipse","x":0}]"#, "an ellipse needs y"),
        (r#"[{"type":"rectangle","y":0}]"#, "a rectangle needs x"),
        (
            r#"[{"type":"line","x":0,"y":0}]"#,
            "a line needs points, or a width and height, or start and end",
        ),
        (
            r#"[{"type":"arrow","x":0,"y":0}]"#,
            "an arrow needs points, or a width and height, or start and end",
        ),
    ];
    for (json, sentence) in cases {
        let text = refused(json).to_string();
        assert!(text.ends_with(sentence), "{json}: {text}");
    }
}
