//! The shape library as the setting keeps it (section 4.15).

use innerword_board::{SceneError, ShapeLibrary, limits};
use serde_json::{Value, json};

use super::common::*;

#[test]
fn a_library_excalidraw_wrote_round_trips_without_its_pictures_and_embeds() {
    let parsed = ShapeLibrary::from_stored(&fixture("excalidraw-library.json")).unwrap();
    let ids: Vec<&str> = parsed.library.items().iter().map(|i| i.id()).collect();
    assert_eq!(ids, ["item-service", "item-flow"]);
    assert_eq!(parsed.warnings.len(), 2, "{:?}", parsed.warnings);
    assert!(parsed.warnings[0].contains("image"));
    assert!(parsed.warnings[1].contains("iframe"));
    let service = &parsed.library.items()[0];
    assert_eq!(service.name(), Some("Service box"));
    assert_eq!(service.created(), 1_763_900_000_123);
    assert_eq!(service.status(), "unpublished");
    assert_eq!(
        service.elements().len(),
        2,
        "the deleted element is left out"
    );
    let flow = &parsed.library.items()[1];
    assert_eq!(flow.elements()[0].json()["futureField"], json!([1, 2, 3]));
    assert_eq!(flow.status(), "published");

    let stored = parsed.library.to_stored();
    let value: Value = serde_json::from_str(&stored).unwrap();
    assert_eq!(value["type"], "excalidrawlib");
    assert_eq!(value["version"], 2);
    assert_eq!(value["source"], "inner-word");
    assert!(value["libraryItems"][1].get("name").is_none());
    let again = ShapeLibrary::from_stored(&stored).unwrap();
    assert!(again.warnings.is_empty());
    assert_eq!(again.library, parsed.library);
    assert_eq!(again.library.to_stored(), stored);
    // The setting never names stored bytes.
    assert!(!stored.contains("\"file\""));
}

#[test]
fn the_picker_s_titles() {
    let library = ShapeLibrary::from_stored(&fixture("excalidraw-library.json"))
        .unwrap()
        .library;
    assert_eq!(library.items()[0].title(0), "Service box");
    assert_eq!(library.items()[1].title(1), "Shape 2: arrow, diamond");
}

#[test]
fn what_the_view_posts_is_read_leniently_and_within_the_limits() {
    let items = format!(
        "[{{\"id\":\"i1\",\"status\":\"unpublished\",\"created\":5,\"name\":\"a{}b\",\"elements\":[{{\"id\":\"r\",\"type\":\"rectangle\",\"width\":null}}]}}, 4, {{\"elements\":[]}}]",
        ["\\", "udc00"].concat()
    );
    let parsed = ShapeLibrary::from_view(&items).unwrap();
    assert_eq!(parsed.library.items().len(), 1);
    assert_eq!(parsed.library.items()[0].name(), Some("a\u{fffd}b"));
    assert_eq!(parsed.warnings.len(), 2);
    assert!(matches!(
        ShapeLibrary::from_view("{oops"),
        Err(SceneError::NotJson(_))
    ));
    assert!(matches!(
        ShapeLibrary::from_view("\"text\""),
        Err(SceneError::NotAScene(_))
    ));
    assert_eq!(
        ShapeLibrary::from_view("[]").unwrap().library,
        ShapeLibrary::empty()
    );

    let too_many: Vec<Value> = (0..=limits::MAX_LIBRARY_ITEMS)
        .map(|n| json!({"id": format!("i{n}"), "elements": [{"id": "r", "type": "rectangle"}]}))
        .collect();
    assert_eq!(
        ShapeLibrary::from_view(&Value::Array(too_many).to_string()),
        Err(SceneError::TooLarge {
            what: "shape library",
            limit: limits::MAX_LIBRARY_ITEMS
        })
    );
    let heavy = json!([{"id": "big", "elements": [{"id": "r", "type": "rectangle", "pad": "x".repeat(limits::MAX_LIBRARY_BYTES)}]}]);
    assert_eq!(
        ShapeLibrary::from_view(&heavy.to_string()),
        Err(SceneError::TooLarge {
            what: "shape library",
            limit: limits::MAX_LIBRARY_BYTES
        })
    );
    assert_eq!(
        SceneError::TooLarge {
            what: "shape library",
            limit: limits::MAX_LIBRARY_BYTES
        }
        .to_string(),
        "the shape library is larger than 8 MB"
    );
    assert!(ShapeLibrary::empty().is_empty());
    assert_eq!(
        ShapeLibrary::empty().to_stored(),
        r#"{"libraryItems":[],"source":"inner-word","type":"excalidrawlib","version":2}"#
    );
}

#[test]
fn an_item_nested_deeper_than_json_is_read_is_kept_with_the_deep_part_left_out() {
    let deep = format!("{}1{}", "[".repeat(140), "]".repeat(140));
    let items = format!(
        r#"[{{"id":"deep","status":"unpublished","created":1,"elements":[{{"id":"r","type":"rectangle","x":0,"y":0,"width":10,"height":10,"customData":{deep}}}]}}]"#
    );
    assert!(serde_json::from_str::<Value>(&items).is_err());
    let parsed = ShapeLibrary::from_view(&items).unwrap();
    assert_eq!(parsed.library.items().len(), 1);
    assert!(
        parsed.warnings.iter().any(|w| w.contains("nested deeper")),
        "{:?}",
        parsed.warnings
    );
    let again = ShapeLibrary::from_stored(&parsed.library.to_stored()).unwrap();
    assert_eq!(again.library, parsed.library);
}
