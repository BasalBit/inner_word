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

// ----- applying a drawing ---------------------------------------------------------------

use innerword_board::{Bounds, DrawError, Placement, Scene, draw, place};

use super::common::{drawn, making};

/// A whiteboard with a labelled rectangle `api`, an ellipse `db`, a rectangle `cache` and
/// an arrow `reads` from `api` to `db`.
fn board() -> Scene {
    drawn(
        r#"[{"type": "rectangle", "id": "api", "x": 100, "y": 80, "width": 200, "height": 90, "label": {"text": "API"}},
            {"type": "ellipse", "id": "db", "x": 420, "y": 80, "width": 160, "height": 90},
            {"type": "rectangle", "id": "cache", "x": 100, "y": 300, "width": 120, "height": 60},
            {"type": "arrow", "id": "reads", "x": 300, "y": 125, "start": {"id": "api"}, "end": {"id": "db"}}]"#,
    )
}

fn apply(
    scene: &Scene,
    json: &str,
    view: Option<Bounds>,
) -> (Scene, Result<innerword_board::Drawn, DrawError>) {
    let mut scene = scene.clone();
    let result = draw(&mut scene, &read(json), view, &mut making());
    (scene, result)
}

fn view(x: f64, y: f64, width: f64, height: f64) -> Option<Bounds> {
    Some(Bounds {
        x,
        y,
        width,
        height,
    })
}

fn bounds_of(scene: &Scene, ids: &[&str]) -> [f64; 4] {
    let mut out = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for id in ids {
        let e = scene.element(id).unwrap();
        out[0] = out[0].min(e.x());
        out[1] = out[1].min(e.y());
        out[2] = out[2].max(e.x() + e.width());
        out[3] = out[3].max(e.y() + e.height());
    }
    out
}

#[test]
fn a_drawing_adds_changes_and_removes_whole_or_not_at_all() {
    let before = board();
    let (after, result) = apply(
        &before,
        r##"{"add": [{"type": "ellipse", "id": "queue", "x": 700, "y": 80, "width": 120, "height": 80},
                     {"type": "arrow", "id": "sends", "x": 580, "y": 125, "start": {"id": "db"}, "end": {"id": "queue"}}],
            "update": [{"id": "api", "backgroundColor": "#a5d8ff", "fillStyle": "solid"}],
            "remove": ["cache"]}"##,
        None,
    );
    let drawn = result.unwrap();
    let added: Vec<&str> = drawn.added.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(added, vec!["queue", "sends"]);
    assert!(drawn.changed.contains(&"api".to_string()));
    assert_eq!(drawn.removed, vec!["cache"]);
    assert!(drawn.unchanged.is_empty());
    assert!(drawn.renamed.is_empty());
    assert_eq!(drawn.placement, None, "it names what is there: it stays");
    assert!(!drawn.is_nothing());
    assert_eq!(
        after.element("api").unwrap().json()["backgroundColor"],
        "#a5d8ff"
    );
    assert!(after.element("cache").is_none());
    assert_eq!(after.element("sends").unwrap().binding("start"), Some("db"));

    // A change the converter refuses: nothing at all, the scene as it was.
    let mut scene = before.clone();
    let error = draw(
        &mut scene,
        &read(
            r#"{"add": [{"type": "ellipse", "id": "queue", "x": 700, "y": 80}],
                "update": [{"id": "api", "fillStyle": "plaid"}], "remove": ["cache"]}"#,
        ),
        None,
        &mut making(),
    )
    .unwrap_err();
    assert!(matches!(error, DrawError::Board(_)), "{error:?}");
    assert!(error.to_string().contains("fillStyle"), "{error}");
    assert_eq!(scene.to_stored(), before.to_stored());
}

#[test]
fn removing_what_is_gone_is_left_out() {
    let (after, result) = apply(&board(), r#"{"remove": ["cache", "zz", "cache"]}"#, None);
    let drawn = result.unwrap();
    assert_eq!(drawn.removed, vec!["cache"]);
    assert_eq!(drawn.already_gone, vec!["zz"]);
    assert!(after.element("cache").is_none());
}

#[test]
fn changing_what_is_gone_draws_nothing() {
    let before = board();
    let (after, result) = apply(
        &before,
        r#"{"add": [{"type": "text", "x": 0, "y": 0, "text": "x"}], "update": [{"id": "zz", "dx": 4}, {"id": "api", "dx": 4}, {"id": "yy", "dx": 1}]}"#,
        None,
    );
    let error = result.unwrap_err();
    assert_eq!(
        error,
        DrawError::Gone {
            changed: vec!["zz".into(), "yy".into()],
            named: vec![]
        }
    );
    assert_eq!(
        error.to_string(),
        "the answer changes zz and yy, which are no longer on the whiteboard"
    );
    assert_eq!(after.to_stored(), before.to_stored());
}

#[test]
fn an_arrow_to_what_is_gone_draws_nothing() {
    let (_, result) = apply(
        &board(),
        r#"[{"type": "arrow", "x": 0, "y": 0, "start": {"id": "api"}, "end": {"id": "cache2"}},
            {"type": "frame", "x": 0, "y": 0, "children": ["new", "cache2", "gone"]},
            {"type": "rectangle", "id": "new", "x": 0, "y": 0}]"#,
        None,
    );
    let error = result.unwrap_err();
    assert_eq!(
        error,
        DrawError::Gone {
            changed: vec![],
            named: vec!["cache2".into(), "gone".into()]
        }
    );
    assert_eq!(
        error.to_string(),
        "the answer joins arrows or frames to cache2 and gone, which are no longer on the whiteboard"
    );
}

#[test]
fn a_change_that_joins_to_what_is_gone_draws_nothing() {
    let (_, result) = apply(
        &board(),
        r#"{"update": [{"id": "reads", "start": {"id": "zz"}}]}"#,
        None,
    );
    let error = result.unwrap_err();
    assert_eq!(
        error,
        DrawError::Gone {
            changed: vec![],
            named: vec!["zz".into()]
        }
    );
    assert_eq!(
        error.to_string(),
        "the answer joins arrows or frames to zz, which is no longer on the whiteboard"
    );
    // A change joined to a new element of the same drawing is drawn.
    let (after, result) = apply(
        &board(),
        r#"{"add": [{"type": "ellipse", "id": "store", "x": 420, "y": 300}],
            "update": [{"id": "reads", "end": {"id": "store"}}]}"#,
        None,
    );
    result.unwrap();
    assert_eq!(
        after.element("reads").unwrap().binding("end"),
        Some("store")
    );
}

#[test]
fn an_id_the_whiteboard_has_is_renamed_with_its_references() {
    let (after, result) = apply(
        &board(),
        r#"{"add": [{"type": "rectangle", "id": "api", "x": 100, "y": 600, "label": {"text": "API v2"}},
                    {"type": "arrow", "id": "next", "x": 200, "y": 500, "start": {"id": "api"}, "end": {"id": "db"}},
                    {"type": "frame", "id": "v2", "x": 0, "y": 0, "children": ["api", "next"]}],
            "update": [{"id": "api", "dx": 10}, {"id": "reads", "end": {"id": "api"}}],
            "remove": []}"#,
        None,
    );
    let drawn = result.unwrap();
    assert_eq!(
        drawn.renamed,
        vec![("api".to_string(), "api-2".to_string())]
    );
    let added: Vec<&str> = drawn.added.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(added, vec!["api-2", "next", "v2"]);
    // Inside add, the new element.
    assert_eq!(
        after.element("next").unwrap().binding("start"),
        Some("api-2")
    );
    assert_eq!(after.element("next").unwrap().binding("end"), Some("db"));
    assert_eq!(after.element("api-2").unwrap().frame_id(), Some("v2"));
    // In update, the whiteboard's.
    assert_eq!(after.element("api").unwrap().x(), 110.0);
    assert_eq!(after.element("reads").unwrap().binding("end"), Some("api"));
    assert_eq!(after.element("api").unwrap().frame_id(), None);
}

#[test]
fn a_renaming_is_free_against_everything() {
    let scene = drawn(
        r#"[{"type": "rectangle", "id": "a", "x": 0, "y": 0}, {"type": "rectangle", "id": "a-2", "x": 200, "y": 0}]"#,
    );
    let (_, result) = apply(
        &scene,
        r#"[{"type": "ellipse", "id": "a", "x": 0, "y": 400}, {"type": "ellipse", "id": "a-2", "x": 200, "y": 400},
            {"type": "ellipse", "id": "a-3-x", "x": 400, "y": 400}]"#,
        None,
    );
    let drawn = result.unwrap();
    assert_eq!(
        drawn.renamed,
        vec![
            ("a".to_string(), "a-3".to_string()),
            ("a-2".to_string(), "a-2-2".to_string())
        ]
    );
    // A name written in add is never given to a renamed element.
    let (_, result) = apply(
        &scene,
        r#"[{"type": "ellipse", "id": "a", "x": 0, "y": 400}, {"type": "ellipse", "id": "a-3", "x": 200, "y": 400}]"#,
        None,
    );
    assert_eq!(
        result.unwrap().renamed,
        vec![("a".to_string(), "a-4".to_string())]
    );
}

#[test]
fn a_renaming_stays_within_64_characters() {
    let long = "x".repeat(64);
    let scene = drawn(&format!(
        r#"[{{"type": "rectangle", "id": "{long}", "x": 0, "y": 0}}]"#
    ));
    let (after, result) = apply(
        &scene,
        &format!(r#"[{{"type": "ellipse", "id": "{long}", "x": 0, "y": 400}}]"#),
        None,
    );
    let drawn = result.unwrap();
    let new = &drawn.renamed[0].1;
    assert_eq!(new.len(), 64);
    assert_eq!(new, &format!("{}-2", "x".repeat(62)));
    assert!(after.element(new).is_some());
}

#[test]
fn a_change_that_sets_what_an_element_has_is_left_out() {
    let before = board();
    let db_version = before.element("db").unwrap().version();
    let (after, result) = apply(
        &before,
        r#"{"update": [{"id": "db", "backgroundColor": "transparent"}, {"id": "api", "dx": 5}]}"#,
        None,
    );
    let drawn = result.unwrap();
    assert_eq!(drawn.unchanged, vec![0]);
    assert!(drawn.changed.contains(&"api".to_string()));
    assert!(!drawn.changed.contains(&"db".to_string()));
    assert_eq!(after.element("db").unwrap().version(), db_version);
    assert_eq!(after.element("api").unwrap().x(), 105.0);
}

#[test]
fn a_drawing_of_nothing_is_nothing() {
    let before = board();
    for json in [
        r#"{"remove": ["zz", "yy"]}"#,
        r#"{"update": [{"id": "db", "backgroundColor": "transparent"}, {"id": "api", "x": 100}]}"#,
        r#"{"update": [{"id": "db", "backgroundColor": "transparent"}], "remove": ["gone"]}"#,
    ] {
        let mut scene = before.clone();
        let drawn = draw(&mut scene, &read(json), None, &mut making()).unwrap();
        assert!(drawn.is_nothing(), "{json}");
        assert_eq!(scene.to_stored(), before.to_stored(), "{json}");
    }
}

#[test]
fn a_drawing_written_free_and_in_view_stays() {
    let (after, result) = apply(
        &board(),
        r#"[{"type": "ellipse", "id": "q", "x": 700, "y": 80, "width": 100, "height": 60}]"#,
        view(0.0, 0.0, 1600.0, 900.0),
    );
    assert_eq!(
        result.unwrap().placement,
        Some(Placement {
            dx: 0.0,
            dy: 0.0,
            in_view: true
        })
    );
    assert_eq!(after.element("q").unwrap().x(), 700.0);
}

#[test]
fn a_drawing_that_covers_the_drawing_moves_to_the_nearest_free_place_in_view() {
    let before = board();
    let v = view(0.0, 0.0, 1600.0, 900.0);
    let (after, result) = apply(
        &before,
        r#"[{"type": "rectangle", "id": "q", "x": 120, "y": 90, "width": 100, "height": 60, "label": {"text": "Queue"}},
            {"type": "rectangle", "id": "w", "x": 260, "y": 90, "width": 100, "height": 60},
            {"type": "arrow", "id": "to", "x": 220, "y": 120, "start": {"id": "q"}, "end": {"id": "w"}}]"#,
        v,
    );
    let drawn = result.unwrap();
    let placed = drawn.placement.unwrap();
    assert!(placed.in_view);
    assert!(placed.dx != 0.0 || placed.dy != 0.0);
    // Moved as a whole.
    let q = after.element("q").unwrap();
    let w = after.element("w").unwrap();
    assert_eq!(q.x() + 140.0, w.x());
    assert_eq!(q.y(), w.y());
    assert_eq!(after.element("to").unwrap().binding("end"), Some("w"));
    // Free with its margin, and in view.
    let group = bounds_of(&after, &["q", "w", "to"]);
    for id in ["api", "db", "cache", "reads"] {
        let e = before.element(id).unwrap();
        let other = [e.x(), e.y(), e.x() + e.width(), e.y() + e.height()];
        let grown = [
            group[0] - 40.0,
            group[1] - 40.0,
            group[2] + 40.0,
            group[3] + 40.0,
        ];
        assert!(
            !(grown[0] < other[2]
                && other[0] < grown[2]
                && grown[1] < other[3]
                && other[1] < grown[3]),
            "{id} is under the drawing"
        );
    }
    assert!(group[0] >= 0.0 && group[1] >= 0.0 && group[2] <= 1600.0 && group[3] <= 900.0);
}

#[test]
fn a_drawing_written_outside_the_view_comes_into_it() {
    let (after, result) = apply(
        &board(),
        r#"[{"type": "ellipse", "id": "q", "x": 5000, "y": 5000, "width": 100, "height": 60}]"#,
        view(-200.0, -100.0, 1000.0, 600.0),
    );
    let placed = result.unwrap().placement.unwrap();
    assert!(placed.in_view);
    let q = after.element("q").unwrap();
    // The nearest place to where it was written: the bottom right of the view.
    assert!(
        q.x() + 100.0 <= 800.0 - 40.0 + 1e-9 && q.x() + 100.0 > 700.0,
        "{}",
        q.x()
    );
    assert!(
        q.y() + 60.0 <= 500.0 - 40.0 + 1e-9 && q.y() + 60.0 > 400.0,
        "{}",
        q.y()
    );
}

#[test]
fn a_drawing_that_names_what_is_there_never_moves() {
    let (after, result) = apply(
        &board(),
        r#"[{"type": "ellipse", "id": "q", "x": 120, "y": 90, "width": 100, "height": 60},
            {"type": "arrow", "x": 220, "y": 120, "start": {"id": "api"}, "end": {"id": "q"}}]"#,
        view(0.0, 0.0, 1600.0, 900.0),
    );
    assert_eq!(result.unwrap().placement, None);
    assert_eq!(after.element("q").unwrap().x(), 120.0);
    // Nor does one with changes or removals.
    let (after, result) = apply(
        &board(),
        r#"{"add": [{"type": "ellipse", "id": "q", "x": 120, "y": 90}], "remove": ["cache"]}"#,
        view(0.0, 0.0, 1600.0, 900.0),
    );
    assert_eq!(result.unwrap().placement, None);
    assert_eq!(after.element("q").unwrap().x(), 120.0);
}

#[test]
fn a_drawing_larger_than_the_view_goes_beside_the_drawing() {
    let (after, result) = apply(
        &board(),
        r#"[{"type": "rectangle", "id": "big", "x": 0, "y": 0, "width": 3000, "height": 200}]"#,
        view(0.0, 0.0, 1000.0, 600.0),
    );
    let placed = result.unwrap().placement.unwrap();
    assert!(!placed.in_view);
    let drawn_area = board().bounds().unwrap();
    let big = after.element("big").unwrap();
    assert_eq!(big.x(), drawn_area.x + drawn_area.width + 100.0);
    assert_eq!(big.y(), drawn_area.y);
}

#[test]
fn without_a_view_a_drawing_stays_where_it_is_free() {
    let (after, result) = apply(
        &board(),
        r#"[{"type": "ellipse", "id": "q", "x": 2000, "y": 2000}]"#,
        None,
    );
    assert_eq!(
        result.unwrap().placement,
        Some(Placement {
            dx: 0.0,
            dy: 0.0,
            in_view: false
        })
    );
    assert_eq!(after.element("q").unwrap().x(), 2000.0);
    // Where it covers the drawing: beside it.
    let (after, result) = apply(
        &board(),
        r#"[{"type": "ellipse", "id": "q", "x": 100, "y": 80}]"#,
        None,
    );
    assert!(!result.unwrap().placement.unwrap().in_view);
    let drawn_area = board().bounds().unwrap();
    assert_eq!(
        after.element("q").unwrap().x(),
        drawn_area.x + drawn_area.width + 100.0
    );
}

#[test]
fn an_empty_whiteboard() {
    let empty = Scene::empty();
    let (after, result) = apply(
        &empty,
        r#"[{"type": "ellipse", "id": "q", "x": 0, "y": 0, "width": 100, "height": 50}]"#,
        view(-500.0, -300.0, 1000.0, 600.0),
    );
    assert_eq!(result.unwrap().placement.unwrap().dx, 0.0);
    assert_eq!(after.element("q").unwrap().x(), 0.0);
    let (after, result) = apply(
        &empty,
        r#"[{"type": "ellipse", "id": "q", "x": 9000, "y": 0, "width": 100, "height": 50}]"#,
        view(-500.0, -300.0, 1000.0, 600.0),
    );
    assert!(result.unwrap().placement.unwrap().in_view);
    assert_eq!(after.element("q").unwrap().x(), 500.0 - 40.0 - 100.0);
    // A new whiteboard: no view, as written.
    let (after, result) = apply(
        &empty,
        r#"[{"type": "ellipse", "id": "q", "x": 9000, "y": 0}]"#,
        None,
    );
    assert!(!result.unwrap().placement.unwrap().in_view);
    assert_eq!(after.element("q").unwrap().x(), 9000.0);
}

#[test]
fn place_takes_the_nearest_free_place_on_a_grid() {
    let b = |x: f64, y: f64, width: f64, height: f64| Bounds {
        x,
        y,
        width,
        height,
    };
    // The grid: from the view's corner plus the margin, a step of a 24th of the view.
    let placed = place(
        b(0.0, 0.0, 50.0, 50.0),
        &[b(0.0, 0.0, 100.0, 100.0)],
        Some(b(0.0, 0.0, 1000.0, 1000.0)),
    );
    assert!(placed.in_view);
    assert!((placed.dx - 165.0).abs() < 1e-9, "{placed:?}");
    assert!((placed.dy - 40.0).abs() < 1e-9, "{placed:?}");
    // Touching the margin is free; meeting it is not.
    let placed = place(
        b(140.0, 0.0, 50.0, 50.0),
        &[b(0.0, 0.0, 100.0, 100.0)],
        Some(b(0.0, -100.0, 1000.0, 1000.0)),
    );
    assert_eq!(
        placed,
        Placement {
            dx: 0.0,
            dy: 0.0,
            in_view: true
        }
    );
    let placed = place(
        b(139.0, 0.0, 50.0, 50.0),
        &[b(0.0, 0.0, 100.0, 100.0)],
        Some(b(0.0, -100.0, 1000.0, 1000.0)),
    );
    assert!(placed.dx != 0.0 || placed.dy != 0.0);
    // A full view: beside the drawn area.
    let placed = place(
        b(0.0, 0.0, 50.0, 50.0),
        &[b(-100.0, -100.0, 400.0, 400.0)],
        Some(b(0.0, 0.0, 200.0, 200.0)),
    );
    assert_eq!(
        placed,
        Placement {
            dx: 400.0,
            dy: -100.0,
            in_view: false
        }
    );
    // A huge view stays quick: at most 2,500 places are tried.
    let started = std::time::Instant::now();
    let many: Vec<Bounds> = (0..2_000)
        .map(|n| {
            b(
                (n % 50) as f64 * 2_000.0,
                (n / 50) as f64 * 2_000.0,
                1_990.0,
                1_990.0,
            )
        })
        .collect();
    let placed = place(
        b(0.0, 0.0, 30.0, 30.0),
        &many,
        Some(b(0.0, 0.0, 100_000.0, 80_000.0)),
    );
    assert!(!placed.in_view || placed.dx != 0.0 || placed.dy != 0.0);
    assert!(started.elapsed().as_secs() < 10);
}

#[test]
fn the_same_drawing_gives_the_same_elements() {
    let json = r#"{"add": [{"type": "ellipse", "x": 700, "y": 80, "label": {"text": "Q"}},
                           {"type": "arrow", "x": 580, "y": 125, "start": {"id": "db"}}],
                  "update": [{"id": "api", "text": "Gateway"}]}"#;
    let (a, _) = apply(&board(), json, view(0.0, 0.0, 1000.0, 600.0));
    let (b, _) = apply(&board(), json, view(0.0, 0.0, 1000.0, 600.0));
    assert_eq!(a.to_stored(), b.to_stored());
}

#[test]
fn the_warnings_of_frames_are_said_after_the_move() {
    let (after, result) = apply(
        &board(),
        r#"[{"type": "frame", "id": "f", "x": 100, "y": 80, "width": 100, "height": 100, "name": "Box", "children": ["c"]},
            {"type": "rectangle", "id": "c", "x": 150, "y": 150, "width": 100, "height": 100}]"#,
        view(0.0, 0.0, 1600.0, 900.0),
    );
    let drawn = result.unwrap();
    let placed = drawn.placement.unwrap();
    assert!(
        placed.dx != 0.0 || placed.dy != 0.0,
        "it covered the drawing"
    );
    assert_eq!(
        drawn.warnings,
        vec!["c sticks out of frame f: what is outside is not shown.".to_string()]
    );
    assert_eq!(
        after.element("c").unwrap().x() - after.element("f").unwrap().x(),
        50.0
    );
}
