//! Frames as the screen and the PDF show them (`docs/whiteboard-design.md`, section 5): a
//! frame clips its children, the page holds the frames with their names and the elements in
//! no frame, and an agent is told when what it drew sticks out of its frame.

use innerword_board::{ElementChange, Pictures, Scene, add, update};

use super::common::{drawn, making, skeletons};

/// 9.4's drawing, cut down: two shapes and an arrow in the frame "backend".
const BACKEND: &str = r#"[
  {"type":"rectangle","id":"api","x":100,"y":100,"width":160,"height":80,"label":{"text":"API"}},
  {"type":"ellipse","id":"db","x":400,"y":100,"width":160,"height":80,"label":{"text":"Postgres"}},
  {"type":"arrow","id":"reads","x":260,"y":140,"width":140,"height":0,"start":{"id":"api"},"end":{"id":"db"},"label":{"text":"reads"}},
  {"type":"frame","id":"backend","children":["api","db","reads"],"name":"backend"}
]"#;

fn changes(json: &str) -> Vec<ElementChange> {
    serde_json::from_str(json).unwrap()
}

fn frame_box(scene: &Scene, id: &str) -> (f64, f64, f64, f64) {
    let f = scene.element(id).unwrap();
    (f.x(), f.y(), f.width(), f.height())
}

#[test]
fn the_page_is_the_frame_with_its_name_whatever_sticks_out_of_it() {
    let mut scene = drawn(BACKEND);
    assert_eq!(frame_box(&scene, "backend"), (90.0, 90.0, 480.0, 100.0));
    // The frame and its name above it (Helvetica 14 at 1.25, 3 above the frame), the margin.
    let b = scene.pdf_bounds().unwrap();
    assert_eq!(
        (b.x, b.y, b.width, b.height),
        (90.0, 90.0 - 20.5, 480.0, 120.5)
    );
    assert_eq!(scene.pdf_size(32.0, 14_400.0), Some((544.0, 185.0)));
    // The ellipse moved half out of the frame: the screen cuts it at the frame's edge, so
    // the page does not grow (Excalidraw's export measures the frames and the elements in
    // no frame). The drawn area an agent is told still holds all of it.
    update(
        &mut scene,
        &changes(r#"[{"id":"db","dy":60}]"#),
        &mut making(),
    )
    .unwrap();
    assert_eq!(scene.pdf_size(32.0, 14_400.0), Some((544.0, 185.0)));
    let drawn_area = scene.bounds().unwrap();
    assert_eq!(drawn_area.y + drawn_area.height, 240.0, "{drawn_area:?}");
}

#[test]
fn an_element_in_no_frame_and_a_name_of_two_lines_count_for_the_page() {
    let scene = drawn(
        r#"[
  {"type":"rectangle","id":"a","x":0,"y":0,"width":100,"height":100},
  {"type":"frame","id":"f","children":["a"],"name":"one\ntwo"},
  {"type":"rectangle","id":"free","x":0,"y":300,"width":50,"height":50}
]"#,
    );
    let (fx, fy, fw, _) = frame_box(&scene, "f");
    let b = scene.pdf_bounds().unwrap();
    // Two lines of 17.5 and the 3 between the name and the frame.
    assert_eq!((b.x, b.y), (fx.min(0.0), fy - 38.0));
    assert_eq!(b.y + b.height, 350.0);
    assert!(b.x + b.width >= fx + fw);
    // An element whose frame is not there is in no frame.
    let mut alone = drawn(r#"[{"type":"rectangle","id":"a","x":0,"y":0,"width":10,"height":10}]"#);
    let stored = alone
        .to_stored()
        .replace(r#""frameId":null"#, r#""frameId":"gone""#);
    alone = Scene::from_stored(&stored).unwrap().scene;
    assert_eq!(alone.pdf_size(0.0, 14_400.0), Some((10.0, 10.0)));
}

#[test]
fn an_agent_is_told_what_sticks_out_of_its_frame() {
    let mut scene = drawn(BACKEND);
    // Moved half out: the ellipse (its label too) is named once, with the frame.
    let moved = update(
        &mut scene,
        &changes(r#"[{"id":"db","dy":60}]"#),
        &mut making(),
    )
    .unwrap();
    assert_eq!(
        moved.warnings,
        vec!["db sticks out of frame backend: what is outside is not shown.".to_string()]
    );
    // Moved back in: no warning.
    let back = update(
        &mut scene,
        &changes(r#"[{"id":"db","dy":-60}]"#),
        &mut making(),
    )
    .unwrap();
    assert!(back.warnings.is_empty(), "{:?}", back.warnings);
    // A change of something else says nothing of an element it did not touch.
    update(
        &mut scene,
        &changes(r#"[{"id":"db","dx":200}]"#),
        &mut making(),
    )
    .unwrap();
    let other = update(
        &mut scene,
        &changes(r##"[{"id":"api","strokeColor":"#e03131"}]"##),
        &mut making(),
    )
    .unwrap();
    assert!(other.warnings.is_empty(), "{:?}", other.warnings);
    // A frame made smaller tells of each child it leaves sticking out.
    let mut scene = drawn(BACKEND);
    let smaller = update(
        &mut scene,
        &changes(r#"[{"id":"backend","width":200}]"#),
        &mut making(),
    )
    .unwrap();
    assert_eq!(
        smaller.warnings,
        vec![
            "db sticks out of frame backend: what is outside is not shown.".to_string(),
            "reads sticks out of frame backend: what is outside is not shown.".to_string(),
        ]
    );
}

#[test]
fn an_element_added_in_a_frame_or_a_frame_added_around_it_is_told_when_it_sticks_out() {
    // A frame of a given size around children that do not fit in it.
    let mut scene = Scene::empty();
    let added = add(
        &mut scene,
        &skeletons(
            r#"[
  {"type":"rectangle","id":"wide","x":0,"y":0,"width":300,"height":50,"label":{"text":"Wide"}},
  {"type":"rectangle","id":"small","x":10,"y":10,"width":20,"height":20},
  {"type":"frame","id":"f","x":0,"y":0,"width":100,"height":100,"children":["wide","small"]}
]"#,
        ),
        &Pictures::new(),
        &mut making(),
    )
    .unwrap();
    assert_eq!(
        added.warnings,
        vec!["wide sticks out of frame f: what is outside is not shown.".to_string()]
    );
    // A frame that fits its children (its size from them) says nothing.
    let mut fitted = Scene::empty();
    let fine = add(
        &mut fitted,
        &skeletons(BACKEND),
        &Pictures::new(),
        &mut making(),
    )
    .unwrap();
    assert!(fine.warnings.is_empty(), "{:?}", fine.warnings);
}
