//! The stored scene, the view's scene, and what is read of a scene.

use std::collections::BTreeMap;

use innerword_board::{ElementType, FileHex, ListingHeader, Scene, SceneError, limits};
use serde_json::{Value, json};

use super::common::*;

fn header() -> ListingHeader<'static> {
    ListingHeader {
        reference: "board:12",
        title: "Architecture",
        folder: Some("Sketches"),
    }
}

#[test]
fn the_stored_form_round_trips_every_field_of_a_scene_excalidraw_made() {
    let text = fixture("scout-scene.json");
    let original: Value = serde_json::from_str(&text).unwrap();
    let parsed = Scene::from_stored(&text).unwrap();
    assert!(parsed.warnings.is_empty(), "{:?}", parsed.warnings);
    let scene = parsed.scene;
    let stored = scene.to_stored();
    let again: Value = serde_json::from_str(&stored).unwrap();
    // Every element, every field (unknown ones such as customData too), in z-order.
    assert_eq!(again["elements"], original["elements"]);
    assert_eq!(again["appState"], original["appState"]);
    assert_eq!(again["files"], original["files"]);
    assert_eq!(again["type"], "excalidraw");
    assert_eq!(again["version"], 2);
    assert_eq!(again["source"], "inner-word");
    // Read again, the same scene; written again, the same bytes.
    let reread = Scene::from_stored(&stored).unwrap().scene;
    assert_eq!(reread, scene);
    assert_eq!(reread.to_stored(), stored);
    // Minified, and the collector's token is there for the picture.
    assert!(!stored.contains('\n'));
    assert!(stored.contains(r#""file":"3f2a9c1b5d6e7f80a1b2c3d4e5f60718""#));
    assert_eq!(
        scene.stored_files(),
        vec![FileHex::parse("3f2a9c1b5d6e7f80a1b2c3d4e5f60718").unwrap()]
    );
    assert_eq!(
        scene.named_file_ids(),
        vec!["6c2b0a9d8e7f60514233241506f7e8d9".to_string()]
    );
    assert_eq!(scene.elements().len(), 11);
    assert!(!scene.is_empty());
}

#[test]
fn deleted_elements_and_the_kinds_never_kept_are_left_out_of_a_stored_scene() {
    let text = json!({
        "type": "excalidraw", "version": 2, "source": "x",
        "elements": [
            {"id": "a", "type": "rectangle", "x": 0, "y": 0, "width": 10, "height": 10, "version": 1},
            {"id": "gone", "type": "rectangle", "isDeleted": true, "version": 4},
            {"id": "web", "type": "iframe", "customData": {"generationData": {"html": "<script>x</script>"}}},
            {"id": "e", "type": "embeddable", "link": "https://example.com"},
            {"id": "m", "type": "magicframe"}
        ],
        "appState": {"viewBackgroundColor": "#fff"}
    })
    .to_string();
    let parsed = Scene::from_stored(&text).unwrap();
    let ids: Vec<&str> = parsed.scene.elements().iter().map(|e| e.id()).collect();
    assert_eq!(ids, ["a"]);
    assert_eq!(parsed.warnings.len(), 3, "{:?}", parsed.warnings);
    assert!(parsed.warnings.iter().all(|w| w.contains("web embeds")));
    // The canvas settings: what was given, Excalidraw's defaults for the rest.
    let app: Value = serde_json::from_str(&parsed.scene.to_stored()).unwrap();
    assert_eq!(
        app["appState"],
        json!({"viewBackgroundColor": "#fff", "gridSize": 20, "gridStep": 5, "gridModeEnabled": false})
    );
}

fn view_scene() -> String {
    let mut elements = vec![
        json!({"id": "pic", "type": "image", "x": 0, "y": 0, "width": 10, "height": 10, "fileId": "known", "status": "saved"}),
        json!({"id": "lost", "type": "image", "x": 20, "y": 0, "width": 10, "height": 10, "fileId": "unknown", "status": "pending"}),
        json!({"id": "future", "type": "hexagon", "x": 0, "y": 40, "width": 30, "height": 30, "sides": 6}),
        json!({"id": "odd", "type": "rectangle", "x": 0, "y": 80, "width": null, "height": 20, "newField": {"a": [1, 2]}}),
        json!({"id": "dead", "type": "ellipse", "isDeleted": true}),
        json!({"id": "embed", "type": "embeddable", "x": 0, "y": 0}),
    ];
    elements.push(json!({"id": "t", "type": "text", "x": 0, "y": 120, "text": "PLACEHOLDER", "originalText": "PLACEHOLDER"}));
    let text = json!({
        "elements": elements,
        "appState": {"viewBackgroundColor": "#123456", "gridSize": 10, "gridStep": 4, "gridModeEnabled": true, "zoom": {"value": 2}, "scrollX": 5, "theme": "dark"},
        "files": {"known": {"id": "known", "mimeType": "image/webp", "created": 77}, "orphan": {"id": "orphan", "mimeType": "image/png", "created": 1}}
    })
    .to_string();
    // A lone surrogate, as JSON.stringify writes half a character pair.
    text.replace("PLACEHOLDER", "a\\ud800b")
}

#[test]
fn the_view_s_scene_is_read_leniently() {
    let stored = |id: &str| match id {
        "known" => Some((
            FileHex::parse("0123456789abcdef0123456789ABCDEF").unwrap(),
            "image/png".to_string(),
        )),
        _ => None,
    };
    let parsed = Scene::from_view(&view_scene(), &stored).unwrap();
    let scene = &parsed.scene;
    let ids: Vec<&str> = scene.elements().iter().map(|e| e.id()).collect();
    assert_eq!(ids, ["pic", "lost", "future", "odd", "t"]);
    // An unknown type and an unknown field are kept as they came, and a null number too.
    assert_eq!(
        scene.element("future").unwrap().kind(),
        ElementType::Other("hexagon".into())
    );
    assert_eq!(scene.element("future").unwrap().json()["sides"], 6);
    assert_eq!(
        scene.element("odd").unwrap().json()["newField"],
        json!({"a": [1, 2]})
    );
    assert_eq!(scene.element("odd").unwrap().json()["width"], Value::Null);
    assert_eq!(scene.element("odd").unwrap().width(), 0.0);
    // The lone half is mended.
    assert_eq!(scene.element("t").unwrap().text(), Some("a\u{fffd}b"));
    // Pictures: the stored one with the type of its stored bytes (never the page's), the
    // unknown one left out of files (its element stays), an entry no element names gone.
    let files = scene.files();
    assert_eq!(files.len(), 1);
    assert_eq!(files["known"].mime_type, "image/png");
    assert_eq!(
        files["known"].file.as_ref().unwrap().as_str(),
        "0123456789abcdef0123456789abcdef"
    );
    assert_eq!(files["known"].created, 77);
    assert!(
        parsed.warnings.iter().any(|w| w.contains("unknown")),
        "{:?}",
        parsed.warnings
    );
    assert!(
        parsed.warnings.iter().any(|w| w.contains("embed")),
        "{:?}",
        parsed.warnings
    );
    // Only the four canvas settings are kept.
    let stored_json: Value = serde_json::from_str(&scene.to_stored()).unwrap();
    assert_eq!(
        stored_json["appState"],
        json!({"viewBackgroundColor": "#123456", "gridSize": 10, "gridStep": 4, "gridModeEnabled": true})
    );
}

#[test]
fn the_view_s_scene_fails_only_on_what_is_not_a_scene() {
    let none = |_: &str| None;
    assert!(matches!(
        Scene::from_view("{not json", &none),
        Err(SceneError::NotJson(_))
    ));
    assert!(matches!(
        Scene::from_view("[1, 2]", &none),
        Err(SceneError::NotAScene(_))
    ));
    assert!(matches!(
        Scene::from_view(r#"{"appState": {}}"#, &none),
        Err(SceneError::NotAScene(_))
    ));
    assert!(matches!(
        Scene::from_view(r#"{"elements": {}}"#, &none),
        Err(SceneError::NotAScene(_))
    ));
    let odd = Scene::from_view(
        r#"{"elements": [1, "x", null, {"type": "rectangle"}]}"#,
        &none,
    )
    .unwrap();
    assert!(odd.scene.is_empty());
    assert_eq!(odd.warnings.len(), 4);
    let empty = Scene::from_view(r#"{"elements": []}"#, &none).unwrap();
    assert_eq!(empty.scene, Scene::empty());
    let huge = format!(
        "{{\"elements\": [], \"pad\": \"{}\"}}",
        "x".repeat(limits::MAX_VIEW_SCENE_BYTES)
    );
    assert_eq!(
        Scene::from_view(&huge, &none),
        Err(SceneError::TooLarge {
            what: "drawing",
            limit: limits::MAX_VIEW_SCENE_BYTES
        })
    );
}

#[test]
fn an_empty_scene_is_written_as_excalidraw_s_envelope() {
    let stored = Scene::empty().to_stored();
    assert_eq!(
        stored,
        r##"{"appState":{"gridModeEnabled":false,"gridSize":20,"gridStep":5,"viewBackgroundColor":"#ffffff"},"elements":[],"files":{},"source":"inner-word","type":"excalidraw","version":2}"##
    );
    assert!(Scene::empty().is_empty());
    assert_eq!(Scene::empty().text(), "");
    assert_eq!(Scene::empty().bounds(), None);
    assert_eq!(Scene::empty().pdf_size(32.0, 14_400.0), None);
}

#[test]
fn the_text_is_in_reading_order() {
    let scene = drawn(
        r#"[
        {"type":"text","id":"low","x":0,"y":300,"text":"Bottom"},
        {"type":"rectangle","id":"right","x":400,"y":100,"width":200,"height":80,"label":{"text":"Right box"}},
        {"type":"rectangle","id":"left","x":0,"y":100,"width":200,"height":80,"label":{"text":"Left box"}},
        {"type":"text","id":"top","x":500,"y":0,"text":"Top\nline two"},
        {"type":"frame","id":"f","x":-20,"y":50,"width":700,"height":200,"name":"Area"},
        {"type":"text","id":"blank","x":0,"y":0,"text":"x"}
    ]"#,
    );
    // The label "x" is placed at the very top left: first. Then by top edge, then left edge.
    assert_eq!(
        scene.text(),
        "x\nTop\nline two\nArea\nLeft box\nRight box\nBottom"
    );
    let summary = scene.summary();
    assert_eq!(summary.words, 10);
    assert_eq!(summary.texts, 5);
    // A label is part of its shape, as the listing's header counts (section 7.4).
    assert_eq!(summary.elements, 6);
    assert_eq!(summary.labels, 2);
    assert_eq!(summary.by_type["text"], 3);
    assert_eq!(summary.by_type["rectangle"], 2);
    assert_eq!(summary.by_type["frame"], 1);
}

#[test]
fn the_listing_of_a_whiteboard() {
    let mut scene = drawn(
        r##"[
        {"type":"rectangle","id":"api","x":100,"y":80,"width":200,"height":90,"backgroundColor":"#a5d8ff","fillStyle":"solid","label":{"text":"API"}},
        {"type":"ellipse","id":"db","x":420,"y":80,"width":160,"height":90,"strokeColor":"#1971c2","label":{"text":"Postgres"}},
        {"type":"arrow","id":"reads","x":300,"y":125,"start":{"id":"api"},"end":{"id":"db"},"label":{"text":"reads"}},
        {"type":"line","id":"rule","x":100,"y":200,"points":[[0,0],[240,0],[480,10]],"strokeStyle":"dashed"},
        {"type":"freedraw","id":"scribble","x":100,"y":240,"points":[[0,0],[10,5],[20,0]]},
        {"type":"diamond","id":"choice","x":700,"y":80,"width":100,"height":100,"angle":0.5235987755982988,"link":"https://example.com/a","locked":true,"groupIds":["g1"],"opacity":60},
        {"type":"frame","id":"backend","children":["api","db","reads"],"name":"Backend"}
    ]"##,
    );
    let header = header();
    let text = scene.listing(&header);
    let expected = r#"board:12 “Architecture” in Sketches: 7 elements, 0 pictures; drawn area 703 x 175 at (90, 70)
rectangle  api       at (100, 80)    200 x 90                  fill #a5d8ff solid  label "API"
ellipse    db        at (420, 80)    160 x 90                  stroke #1971c2  label "Postgres"
arrow      reads     from api to db  (301, 125) -> (420, 125)  label "reads"
line       rule      at (100, 200)   (100, 200) -> (580, 210)  3 points  dashed
freedraw   scribble  at (100, 240)   20 x 5                    3 points
diamond    choice    at (700, 80)    100 x 100                 rotated 30°  opacity 60  groups g1  link https://example.com/a  locked
frame      backend   at (90, 70)     500 x 110                 "Backend"  holds api, db, reads"#;
    assert_eq!(text, expected, "\n{text}");
    // Without a folder, and empty.
    let root = ListingHeader {
        reference: "board:3",
        title: "",
        folder: None,
    };
    assert_eq!(Scene::empty().listing(&root), "board:3 “Untitled”: empty");
    // A picture: stored, or missing when the library says so.
    let pictures = picture_map(
        "photo.jpg",
        "3f2a9c1b5d6e7f80a1b2c3d4e5f60718",
        "image/jpeg",
        (2880, 1920),
        72,
    );
    innerword_board::add(
        &mut scene,
        &skeletons(
            r#"[{"type":"image","id":"photo","x":900,"y":0,"path":"photo.jpg","width":300}]"#,
        ),
        &pictures,
        &mut making(),
    )
    .unwrap();
    let text = scene.listing(&header);
    assert!(
        text.contains("7 elements") || text.contains("8 elements, 1 picture"),
        "{text}"
    );
    assert!(
        text.contains(
            "image      photo     at (900, 0)     300 x 200                 picture stored (jpeg)"
        ),
        "{text}"
    );
    let missing = scene.listing_with(&header, &|_| false);
    assert!(missing.contains("picture missing"), "{missing}");
    let listed = scene.listed_with(&|_| false);
    let photo = listed.iter().find(|e| e.id == "photo").unwrap();
    assert_eq!(photo.picture.as_deref(), Some("missing"));
}

pub fn picture_map(
    path: &str,
    file: &str,
    mime: &str,
    pixels: (u32, u32),
    density: u32,
) -> innerword_board::Pictures {
    let mut map = BTreeMap::new();
    map.insert(
        path.to_string(),
        innerword_board::Picture {
            file: FileHex::parse(file).unwrap(),
            mime_type: mime.into(),
            pixels,
            density,
        },
    );
    map
}

#[test]
fn the_listed_elements_say_what_agents_need() {
    let scene = drawn(
        r##"[
        {"type":"rectangle","id":"api","x":100,"y":80,"width":200,"height":90,"label":{"text":"API"}},
        {"type":"ellipse","id":"db","x":420,"y":80,"width":160,"height":90},
        {"type":"arrow","id":"reads","x":300,"y":125,"start":{"id":"api"},"end":{"id":"db"}},
        {"type":"text","id":"title","x":100,"y":20,"text":"System overview","fontSize":28,"fontFamily":"Nunito"},
        {"type":"frame","id":"backend","children":["api","db"],"name":"Backend"}
    ]"##,
    );
    let listed = scene.listed();
    assert_eq!(listed.len(), 6);
    let api = &listed[0];
    assert_eq!(api.label.as_deref(), Some("API"));
    assert_eq!(api.frame.as_deref(), Some("backend"));
    let reads = listed.iter().find(|e| e.id == "reads").unwrap();
    assert_eq!(
        (reads.start.as_deref(), reads.end.as_deref()),
        (Some("api"), Some("db"))
    );
    assert_eq!(reads.points.as_ref().unwrap().len(), 2);
    let title = listed.iter().find(|e| e.id == "title").unwrap();
    assert_eq!(title.text.as_deref(), Some("System overview"));
    assert_eq!(title.style.font_family.as_deref(), Some("Nunito"));
    assert_eq!(title.style.font_size, Some(28.0));
    let label = listed
        .iter()
        .find(|e| e.container.as_deref() == Some("api"))
        .unwrap();
    assert_eq!(label.text.as_deref(), Some("API"));
    // The JSON form leaves out what is not there.
    let json = serde_json::to_value(api).unwrap();
    assert_eq!(json["type"], "rectangle");
    assert!(json.get("picture").is_none());
    assert!(json.get("container").is_none());
}

#[test]
fn bounds_and_the_size_of_the_pdf() {
    let scene = drawn(
        r#"[
        {"type":"rectangle","id":"a","x":100,"y":50,"width":200,"height":100},
        {"type":"ellipse","id":"b","x":-50,"y":300,"width":100,"height":60}
    ]"#,
    );
    let b = scene.bounds().unwrap();
    assert_eq!((b.x, b.y, b.width, b.height), (-50.0, 50.0, 350.0, 310.0));
    assert_eq!(scene.pdf_size(32.0, 14_400.0), Some((414.0, 374.0)));
    // A side past the limit: the whole scaled down.
    let wide = drawn(r#"[{"type":"rectangle","id":"a","x":0,"y":0,"width":28736,"height":100}]"#);
    let (w, h) = wide.pdf_size(32.0, 14_400.0).unwrap();
    assert_eq!(w, 14_400.0);
    assert_eq!(h, ((100.0 + 64.0) * 14_400.0 / 28_800.0f64).ceil());
    // A rotated rectangle's bounds are its corners'.
    let turned = drawn(
        r#"[{"type":"rectangle","id":"a","x":0,"y":0,"width":100,"height":100,"angle":0.7853981633974483}]"#,
    );
    let b = turned.bounds().unwrap();
    assert!(
        (b.width - 100.0 * std::f64::consts::SQRT_2).abs() < 1e-9,
        "{b:?}"
    );
}

/// A rectangle whose `customData` (or a field Excalidraw does not know) nests `depth`
/// arrays: what a pasted Excalidraw JSON can carry onto the canvas, which keeps both.
fn deep(field: &str, depth: usize) -> String {
    format!(
        r#"{{"id":"deep","type":"rectangle","x":0,"y":0,"width":10,"height":10,"{field}":{}1{}}}"#,
        "[".repeat(depth),
        "]".repeat(depth)
    )
}

#[test]
fn a_value_nested_deeper_than_json_is_read_never_makes_the_drawing_unreadable() {
    let none = |_: &str| None;
    for field in ["customData", "somethingNew"] {
        let scene = format!(
            r#"{{"elements":[{},{{"id":"plain","type":"ellipse","x":0,"y":0,"width":5,"height":5}}],"files":{{}}}}"#,
            deep(field, 130)
        );
        // serde_json alone refuses it: this is what made every save fail.
        assert!(serde_json::from_str::<Value>(&scene).is_err());
        let parsed = Scene::from_view(&scene, &none).unwrap();
        let ids: Vec<&str> = parsed.scene.elements().iter().map(|e| e.id()).collect();
        assert_eq!(ids, ["deep", "plain"], "{field}");
        assert!(
            parsed.warnings.iter().any(|w| w.contains("nested deeper")),
            "{:?}",
            parsed.warnings
        );
        // What is kept is read back, and stored and read again as it is.
        let stored = parsed.scene.to_stored();
        let again = Scene::from_stored(&stored).unwrap();
        assert_eq!(again.scene, parsed.scene);
        assert!(again.warnings.is_empty(), "{:?}", again.warnings);
        let mut value = &parsed.scene.element("deep").unwrap().json()[field];
        let mut levels = 0;
        while let Value::Array(items) = value {
            levels += 1;
            value = &items[0];
        }
        assert_eq!(*value, Value::Null, "{field}");
        assert!((90..=100).contains(&levels), "{field}: {levels}");
    }
    // A string with brackets in it is no nesting, and a scene within the limit is untouched.
    let brackets = format!(
        r#"{{"elements":[{{"id":"s","type":"text","x":0,"y":0,"text":"{}\"{}","customData":{{"a":[[1]]}}}}]}}"#,
        "[".repeat(300),
        "{".repeat(300)
    );
    let parsed = Scene::from_view(&brackets, &none).unwrap();
    assert!(parsed.warnings.is_empty(), "{:?}", parsed.warnings);
    assert_eq!(
        parsed.scene.element("s").unwrap().json()["customData"],
        json!({"a": [[1]]})
    );
    let fine = format!(r#"{{"elements":[{}]}}"#, deep("customData", 60));
    let parsed = Scene::from_view(&fine, &none).unwrap();
    assert!(parsed.warnings.is_empty(), "{:?}", parsed.warnings);
}

#[test]
fn a_link_is_kept_only_for_the_web_mail_and_the_drawing_s_own_elements() {
    let none = |_: &str| None;
    let links = [
        ("https://example.com/a", true),
        ("HTTP://example.com", true),
        ("mailto:someone@example.com", true),
        ("innerword-page://board-4-1700000000000/index.html?element=abc", true),
        ("null/index.html?element=abc", true),
        ("example.com", true),
        ("smb://attacker.example/share", false),
        ("file:///Users/someone/secret.txt", false),
        ("zoommtg://zoom.us/join?confno=1", false),
        ("  FiLe:///etc/hosts", false),
        ("fi\tle:///etc/hosts", false),
        ("about:blank", false),
        ("x-apple.systempreferences:com.apple", false),
    ];
    let elements: Vec<Value> = links
        .iter()
        .enumerate()
        .map(|(n, (link, _))| {
            json!({"id": format!("e{n}"), "type": "rectangle", "x": 0, "y": 0, "width": 10, "height": 10, "link": link})
        })
        .collect();
    let scene = json!({ "elements": elements }).to_string();
    for parsed in [
        Scene::from_view(&scene, &none).unwrap(),
        Scene::from_stored(&scene).unwrap(),
    ] {
        for (n, (link, kept)) in links.iter().enumerate() {
            let element = parsed.scene.element(&format!("e{n}")).unwrap();
            if *kept {
                assert_eq!(element.link(), Some(*link), "{link}");
            } else {
                assert_eq!(element.json()["link"], Value::Null, "{link}");
            }
        }
        let dropped = links.iter().filter(|(_, kept)| !kept).count();
        assert_eq!(
            parsed
                .warnings
                .iter()
                .filter(|w| w.contains("The link of the element"))
                .count(),
            dropped,
            "{:?}",
            parsed.warnings
        );
    }
}
