//! A drawing from an answer of the assistant: read and checked (`Drawing`), applied whole
//! or not at all (`draw`), placed where it is free and seen (`place`)
//! (`docs/assistant-draws-design.md`, sections 2 and 5).

use innerword_board::{Drawing, DrawingError, is_id, limits};

fn read(json: &str) -> Drawing {
    Drawing::from_json(json).unwrap_or_else(|error| panic!("not read: {error}"))
}

fn refused(json: &str) -> DrawingError {
    Drawing::from_json(json).unwrap_err()
}

#[test]
fn a_drawing_reads_as_an_object_or_an_array() {
    let object = read(
        r#"{"title": "  Who   serves\n\t whom ", "add": [{"type": "ellipse", "id": "ada", "x": 0, "y": 0}],
            "update": [{"id": "api", "dx": 40}], "remove": ["db", "db", "cache"]}"#,
    );
    assert_eq!(object.title.as_deref(), Some("Who serves whom"));
    assert_eq!(object.add.len(), 1);
    assert_eq!(object.update.len(), 1);
    assert_eq!(object.remove, vec!["db", "db", "cache"]);
    assert_eq!(object.removed_ids(), vec!["db", "cache"]);
    assert_eq!(object.changed_ids(), vec!["api"]);
    assert!(object.has_changes());
    assert!(!object.is_empty());

    let array = read(
        r#"[{"type": "rectangle", "x": 0, "y": 0}, {"type": "text", "x": 0, "y": 0, "text": "Hi"}]"#,
    );
    assert_eq!(array.add.len(), 2);
    assert!(array.title.is_none());
    assert!(!array.has_changes());
    assert_eq!(array.added_types(), vec![("rectangle", 1), ("text", 1)]);

    // A title is cut at 120 characters; one of white space only is no title.
    let long = read(&format!(
        r#"{{"title": "{}", "remove": ["a"]}}"#,
        "x".repeat(300)
    ));
    assert_eq!(long.title.unwrap().chars().count(), 120);
    assert!(read(r#"{"title": "   ", "remove": ["a"]}"#).title.is_none());
}

#[test]
fn what_is_not_a_drawing_says_why() {
    let not_json = refused("{\"add\": [\n{\"type\": \"ellipse\"}\n{}]}");
    match &not_json {
        DrawingError::NotJson { line, column, .. } => {
            assert_eq!(*line, 3);
            assert_eq!(*column, 1);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        not_json.to_string(),
        "it is not JSON (line 3, column 1: expected `,` or `]`)"
    );
    assert_eq!(
        refused(r#"{"elements": []}"#).to_string(),
        "it is not a drawing: “elements” is not one of its parts (add, update, remove and title are)"
    );
    assert_eq!(
        refused(r#"{"add": {"type": "ellipse"}}"#).to_string(),
        "it is not a drawing: add must be a list of elements"
    );
    assert_eq!(
        refused(r#"{"remove": [12]}"#).to_string(),
        "it is not a drawing: remove must be a list of ids"
    );
    assert_eq!(
        refused(r#"{"update": "api"}"#).to_string(),
        "it is not a drawing: update must be a list of changes"
    );
    assert_eq!(
        refused(r#""a drawing""#).to_string(),
        "it is not a drawing: it must be an object with add, update and remove, or a list of elements"
    );
    assert_eq!(
        refused(r#"{"add": [], "title": "Plan"}"#),
        DrawingError::Empty
    );
    assert_eq!(
        refused("[]").to_string(),
        "it adds, changes and removes nothing"
    );
    let picture = refused(
        r#"[{"type": "ellipse", "x": 0, "y": 0}, {"type": "rectangle", "x": 0, "y": 0},
            {"type": "image", "x": 0, "y": 0, "path": "/etc/passwd"}]"#,
    );
    assert_eq!(picture, DrawingError::Picture { element: 2 });
    assert_eq!(
        picture.to_string(),
        "the element at index 2 of add is a picture, and a drawing from an answer cannot bring one"
    );
    for kind in [r#""type": "star","#, "", r#""type": 3,"#] {
        let json = format!(r#"[{{"type": "ellipse", "x": 0, "y": 0}}, {{{kind} "x": 0, "y": 0}}]"#);
        assert_eq!(
            refused(&json).to_string(),
            "the element at index 1 of add has no type it can draw (rectangle, ellipse, diamond, text, arrow, line, freedraw and frame are)",
            "{json}"
        );
    }
    let many: Vec<String> = (0..limits::MAX_REQUEST_ELEMENTS + 1)
        .map(|n| format!(r#"{{"type": "rectangle", "x": {n}, "y": 0}}"#))
        .collect();
    let too_large = refused(&format!("[{}]", many.join(",")));
    assert_eq!(
        too_large,
        DrawingError::TooLarge {
            list: "add",
            limit: 2_000
        }
    );
    assert_eq!(too_large.to_string(), "it adds more than 2,000 elements");
    let ids: Vec<String> = (0..2_001).map(|n| format!("\"e{n}\"")).collect();
    assert_eq!(
        refused(&format!(r#"{{"remove": [{}]}}"#, ids.join(","))).to_string(),
        "it removes more than 2,000 elements"
    );
}

#[test]
fn every_id_is_checked_when_the_block_is_read() {
    let bad = "**all** [x](https://e.x)";
    let cases = [
        format!(r#"[{{"type": "ellipse", "id": "{bad}", "x": 0, "y": 0}}]"#),
        format!(r#"[{{"type": "arrow", "x": 0, "y": 0, "start": {{"id": "{bad}"}}}}]"#),
        format!(r#"[{{"type": "frame", "x": 0, "y": 0, "children": ["a", "{bad}"]}}]"#),
        format!(r#"{{"update": [{{"id": "{bad}", "dx": 4}}]}}"#),
        format!(r#"{{"update": [{{"id": "arrow1", "end": {{"id": "{bad}"}}}}]}}"#),
        format!(r#"{{"remove": ["ok", "{bad}"]}}"#),
    ];
    for json in &cases {
        assert_eq!(
            refused(json),
            DrawingError::BadId {
                id: bad.to_string()
            },
            "{json}"
        );
    }
    assert_eq!(
        refused(&cases[0]).to_string(),
        "it names an element “**all** [x](https://e.x)”, and an id is 1 to 64 letters, digits, - and _"
    );
    let long = "a".repeat(65);
    assert_eq!(
        refused(&format!(r#"{{"remove": ["{long}"]}}"#)),
        DrawingError::BadId { id: long.clone() }
    );
    assert!(
        refused(&format!(r#"{{"remove": ["{long}"]}}"#))
            .to_string()
            .contains(&format!("“{}…”", "a".repeat(40)))
    );
    // An id that is no string is shown as its JSON.
    assert_eq!(
        refused(r#"{"update": [{"id": 12, "dx": 4}]}"#),
        DrawingError::BadId { id: "12".into() }
    );
    // A change names its element.
    assert!(
        refused(r#"{"update": [{"dx": 4}]}"#)
            .to_string()
            .contains("names no element")
    );
    assert!(is_id("api-2_x"));
    assert!(!is_id(""));
    assert!(!is_id("a b"));
    assert!(is_id(&"a".repeat(64)));
    assert!(!is_id(&"a".repeat(65)));
}

#[test]
fn blocks_join_in_order_with_the_first_title() {
    let one = read(r#"{"add": [{"type": "ellipse", "id": "a", "x": 0, "y": 0}], "remove": ["x"]}"#);
    let two = read(
        r#"{"title": "Second", "add": [{"type": "rectangle", "id": "b", "x": 0, "y": 0}], "update": [{"id": "c", "dx": 1}]}"#,
    );
    let three = read(r#"{"title": "Third", "remove": ["y"]}"#);
    let joined = Drawing::join(vec![one, two, three]).unwrap();
    assert_eq!(joined.title.as_deref(), Some("Second"));
    let ids: Vec<&str> = joined
        .add
        .iter()
        .map(|s| s.0["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["a", "b"]);
    assert_eq!(joined.changed_ids(), vec!["c"]);
    assert_eq!(joined.remove, vec!["x", "y"]);

    // Over the limit only once joined.
    let half = |from: usize| {
        let items: Vec<String> = (from..from + 1_500).map(|n| format!("\"e{n}\"")).collect();
        read(&format!(r#"{{"remove": [{}]}}"#, items.join(",")))
    };
    assert_eq!(
        Drawing::join(vec![half(0), half(1_500)]).unwrap_err(),
        DrawingError::TooLarge {
            list: "remove",
            limit: 2_000
        }
    );
    assert_eq!(Drawing::join(Vec::new()).unwrap_err(), DrawingError::Empty);
}

#[test]
fn what_a_drawing_adds_links_and_unlinks() {
    let drawing = read(
        r#"{"add": [
            {"type": "frame", "x": 0, "y": 0},
            {"type": "arrow", "x": 0, "y": 0, "link": "https://example.com/a"},
            {"type": "rectangle", "x": 0, "y": 0, "link": " https://example.com/a "},
            {"type": "freedraw", "x": 0, "y": 0},
            {"type": "rectangle", "x": 0, "y": 0},
            {"type": "ellipse", "x": 0, "y": 0, "link": "mailto:ada@example.org"}],
          "update": [{"id": "api", "link": null}, {"id": "db", "link": "http://b.example/"}]}"#,
    );
    assert_eq!(
        drawing.added_types(),
        vec![
            ("rectangle", 2),
            ("ellipse", 1),
            ("arrow", 1),
            ("freedraw", 1),
            ("frame", 1)
        ]
    );
    let (set, unset) = drawing.links();
    assert_eq!(
        set,
        vec![
            "https://example.com/a",
            "mailto:ada@example.org",
            "http://b.example/"
        ]
    );
    assert_eq!(unset, vec!["api"]);
}

#[test]
fn the_json_of_a_drawing_is_the_same_whatever_order_its_keys_were_written_in() {
    let a =
        read(r#"{"title": "T", "add": [{"type": "ellipse", "x": 1, "y": 2}], "remove": ["z"]}"#);
    let b =
        read(r#"{"remove": ["z"], "add": [{"y": 2, "x": 1, "type": "ellipse"}], "title": "T"}"#);
    assert_eq!(a.to_json(), b.to_json());
    assert_eq!(
        a.to_json(),
        r#"{"title":"T","add":[{"type":"ellipse","x":1,"y":2}],"update":[],"remove":["z"]}"#
    );
    // It reads back as the same drawing.
    assert_eq!(read(&a.to_json()), a);
}
