//! The calls: their bodies against the glue, their arguments and their answers.

use innerword_board_web::calls::{
    self, Collected, CollectedScene, LibraryAction, PictureToInsert, Zoom,
};

use super::web;

/// Every body, with the name of the call it runs.
fn bodies() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![
        ("collect", calls::COLLECT, "calls.js"),
        ("files", calls::FILES, "calls.js"),
        ("adopt", calls::ADOPT, "calls.js"),
        ("undo", calls::UNDO, "calls.js"),
        ("redo", calls::REDO, "calls.js"),
        ("selectAll", calls::SELECT_ALL, "calls.js"),
        ("zoom", calls::ZOOM, "calls.js"),
        ("search", calls::SEARCH, "calls.js"),
        ("center", calls::CENTER, "calls.js"),
        ("insertPictures", calls::INSERT_PICTURES, "calls.js"),
        ("insertElements", calls::INSERT_ELEMENTS, "calls.js"),
        ("state", calls::STATE, "calls.js"),
        ("mermaid", calls::MERMAID, "calls.js"),
        ("library", calls::LIBRARY, "calls.js"),
        ("keys", calls::KEYS, "calls.js"),
        ("exportLayout", calls::EXPORT_LAYOUT, "export.js"),
        ("convertSkeletons", calls::CONVERT_SKELETONS, "export.js"),
    ]
}

#[test]
fn every_body_calls_a_call_the_glue_has() {
    for (name, body, file) in bodies() {
        assert_eq!(
            body,
            format!("return await window.innerwordBoard.call(\"{name}\", arg);")
        );
        let glue = std::fs::read_to_string(web().join("src/glue").join(file)).unwrap();
        let defined =
            glue.contains(&format!("\n  {name}(")) || glue.contains(&format!("\n  async {name}("));
        assert!(defined, "{file} has no call {name}");
    }
}

#[test]
fn the_export_argument_is_the_pdfs_margin_and_largest_side() {
    let argument: serde_json::Value = serde_json::from_str(&calls::export_argument()).unwrap();
    assert_eq!(
        argument,
        serde_json::json!({ "padding": 32, "maxSide": 14400 })
    );
    assert_eq!(calls::EXPORT_PADDING, innerword_board::limits::PDF_PADDING);
    assert_eq!(
        calls::EXPORT_MAX_SIDE,
        innerword_board::limits::PDF_MAX_SIDE
    );
}

#[test]
fn the_arguments() {
    let json = |text: String| serde_json::from_str::<serde_json::Value>(&text).unwrap();
    assert_eq!(
        json(calls::files_argument(&["a".into(), "b".into()])),
        serde_json::json!({ "ids": ["a", "b"] })
    );
    assert_eq!(
        json(calls::steps_argument(3)),
        serde_json::json!({ "steps": 3 })
    );
    assert_eq!(
        json(calls::zoom_argument(Zoom::In)),
        serde_json::json!({ "in": true })
    );
    assert_eq!(
        json(calls::zoom_argument(Zoom::Out)),
        serde_json::json!({ "out": true })
    );
    assert_eq!(
        json(calls::zoom_argument(Zoom::To(1.25))),
        serde_json::json!({ "to": 1.25 })
    );
    assert_eq!(
        json(calls::zoom_argument(Zoom::Fit)),
        serde_json::json!({ "fit": true })
    );
    assert_eq!(
        json(calls::zoom_argument(Zoom::View {
            zoom: 0.9,
            scroll_x: -40.0,
            scroll_y: 12.5
        })),
        serde_json::json!({ "to": 0.9, "scrollX": -40.0, "scrollY": 12.5 })
    );
    assert_eq!(
        json(calls::search_argument(Some("a \"quoted\"\nline"))),
        serde_json::json!({ "query": "a \"quoted\"\nline" })
    );
    assert_eq!(json(calls::search_argument(None)), serde_json::json!({}));
    assert_eq!(
        json(calls::keys_argument(&["cmd-k", "cmd-alt-["])),
        serde_json::json!({ "givenUp": ["cmd-k", "cmd-alt-["] })
    );
    assert_eq!(
        json(calls::keys_argument(&[])),
        serde_json::json!({ "givenUp": [] })
    );
    assert_eq!(
        json(calls::mermaid_argument(Some("flowchart LR\n A --> B"))),
        serde_json::json!({ "insert": "flowchart LR\n A --> B" })
    );
    assert_eq!(json(calls::mermaid_argument(None)), serde_json::json!({}));
    assert_eq!(
        json(calls::library_argument(&LibraryAction::Toggle)),
        serde_json::json!({ "toggle": true })
    );
    assert_eq!(
        json(calls::library_argument(&LibraryAction::AddSelection)),
        serde_json::json!({ "addSelection": true })
    );
    assert_eq!(
        json(calls::library_argument(&LibraryAction::Insert("i1".into()))),
        serde_json::json!({ "insert": "i1" })
    );
    assert_eq!(
        json(calls::insert_elements_argument(
            r#"[{"id":"r","type":"rectangle"}]"#,
            true,
            Some("t")
        )),
        serde_json::json!({ "elements": [{ "id": "r", "type": "rectangle" }], "select": true, "editText": "t" })
    );
    assert_eq!(
        json(calls::insert_elements_argument("[]", false, None))["editText"],
        serde_json::Value::Null
    );
    let picture = PictureToInsert {
        file: "3f2a9c1b5d6e7f80a1b2c3d4e5f60718".into(),
        mime_type: "image/png".into(),
        data_url: "data:image/png;base64,AA==".into(),
        width: 120.0,
        height: 80.0,
    };
    assert_eq!(
        json(calls::insert_pictures_argument(
            std::slice::from_ref(&picture),
            Some((10.0, 20.5))
        )),
        serde_json::json!({ "pictures": [{ "file": picture.file, "mimeType": "image/png", "dataURL": picture.data_url, "width": 120.0, "height": 80.0 }], "at": { "x": 10.0, "y": 20.5 } })
    );
    assert_eq!(
        json(calls::insert_pictures_argument(&[], None))["at"],
        serde_json::Value::Null
    );
    assert_eq!(
        json(calls::convert_skeletons_argument(r#"[{"type":"text"}]"#)),
        serde_json::json!({ "skeletons": [{ "type": "text" }] })
    );
}

const STATUS: &str = r#"{"rev":4,"posted":4,"editing":false,"composing":false,"pointer":"up","zoom":1,"scrollX":0,"scrollY":0,"elements":2,"selected":0,"tooLarge":false}"#;

#[test]
fn a_collect_answer_of_each_kind() {
    let whole: Collected = calls::answer(&format!(
        r#"{{"rev":4,"scene":{{"elements":[]}},"files":[{{"id":"f","mimeType":"image/png","dataURL":"data:,"}}],"library":{{"lrev":2,"items":[]}},"status":{STATUS}}}"#
    ))
    .unwrap();
    assert!(
        matches!(&whole.scene, CollectedScene::Whole(raw) if raw.get() == r#"{"elements":[]}"#)
    );
    assert_eq!(whole.files[0].id, "f");
    assert_eq!(whole.files[0].mime_type, "image/png");
    assert_eq!(whole.library.as_ref().unwrap().lrev, 2);
    assert_eq!(whole.status.rev, 4);

    let none: Collected = calls::answer(&format!(
        r#"{{"rev":4,"scene":null,"files":[],"library":null,"status":{STATUS}}}"#
    ))
    .unwrap();
    assert!(matches!(none.scene, CollectedScene::None));
    assert!(none.library.is_none());

    let parts: Collected = calls::answer(&format!(
        r#"{{"rev":9,"scene":"parts","files":[],"library":null,"status":{STATUS}}}"#
    ))
    .unwrap();
    assert!(matches!(parts.scene, CollectedScene::Parts));

    assert!(
        calls::answer::<Collected>(&format!(
            r#"{{"rev":9,"scene":"whole","files":[],"library":null,"status":{STATUS}}}"#
        ))
        .is_err()
    );
}

#[test]
fn the_other_answers() {
    let adopted: calls::Adopted =
        calls::answer(r#"{"rev":32,"same":false,"conflicts":["api","reads"]}"#).unwrap();
    assert_eq!(
        (adopted.rev, adopted.same, adopted.conflicts.len()),
        (32, false, 2)
    );
    let posted: calls::FilesPosted = calls::answer(r#"{"posted":["a"],"missing":["b"]}"#).unwrap();
    assert_eq!(
        (posted.posted, posted.missing),
        (vec!["a".to_string()], vec!["b".to_string()])
    );
    let center: calls::Center =
        calls::answer(r#"{"x":390,"y":254.5,"selected":["a","b"]}"#).unwrap();
    assert_eq!(
        (center.x, center.y, center.selected.len()),
        (390.0, 254.5, 2)
    );
    let library: calls::LibraryAnswered = calls::answer(r#"{"lrev":1,"added":null,"refused":"Support for adding images to the library coming soon!","ids":[]}"#).unwrap();
    assert!(library.added.is_none() && library.refused.is_some());
    let library: calls::LibraryAnswered =
        calls::answer(r#"{"lrev":2,"added":"x","refused":null}"#).unwrap();
    assert_eq!(library.added.as_deref(), Some("x"));
    assert!(library.ids.is_empty());
    let zoomed: calls::Zoomed = calls::answer(r#"{"zoom":1.1}"#).unwrap();
    assert_eq!(zoomed.zoom, 1.1);
    let inserted: calls::Inserted = calls::answer(r#"{"ids":["a"]}"#).unwrap();
    assert_eq!(inserted.ids, ["a"]);
    let revised: calls::Revised = calls::answer(r#"{"rev":3}"#).unwrap();
    assert_eq!(revised.rev, 3);
    let size: calls::ExportSize = calls::answer(r#"{"width":664,"height":413.5}"#).unwrap();
    assert_eq!(
        size,
        calls::ExportSize {
            width: 664.0,
            height: 413.5
        }
    );
    let status: innerword_board_web::bridge::Status = calls::answer(STATUS).unwrap();
    assert_eq!(status.elements, 2);
    assert!(
        calls::answer::<calls::Adopted>("{}")
            .unwrap_err()
            .contains("whiteboard")
    );
}
