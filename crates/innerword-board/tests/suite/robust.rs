//! Whatever an agent sends, the converter answers and never panics: it runs on the app's
//! main thread. A deterministic stream of odd requests, each run through add, update and
//! remove, and every scene it leaves read back.

use innerword_board::{ElementChange, HashedMaking, Scene, Skeleton, add, remove, update};
use serde_json::{Map, Value, json};

use super::scene::picture_map;

struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }
    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[(self.next() as usize) % items.len()]
    }
}

fn values() -> Vec<Value> {
    vec![
        json!(0),
        json!(1),
        json!(-1),
        json!(0.5),
        json!(100),
        json!(1e7),
        json!(-250.25),
        json!(null),
        json!(true),
        json!("a"),
        json!(""),
        json!("#1e1e1e"),
        json!("center"),
        json!("Nunito"),
        json!([]),
        json!([[0, 0], [10, 10]]),
        json!([[0, 0], [0, 0]]),
        json!([[0, 0]]),
        json!({"id": "s0"}),
        json!({"id": "s1"}),
        json!({"id": "nope"}),
        json!({"type": "rectangle"}),
        json!({"text": "label"}),
        json!({"text": " "}),
        json!(["s0", "s1"]),
        json!("pic.png"),
        json!({"type": 3}),
    ]
}

const TYPES: [&str; 12] = [
    "rectangle",
    "ellipse",
    "diamond",
    "text",
    "arrow",
    "line",
    "freedraw",
    "frame",
    "image",
    "iframe",
    "x",
    "selection",
];
const FIELDS: [&str; 30] = [
    "x",
    "y",
    "width",
    "height",
    "angle",
    "text",
    "label",
    "start",
    "end",
    "points",
    "children",
    "path",
    "name",
    "fontSize",
    "fontFamily",
    "textAlign",
    "verticalAlign",
    "strokeColor",
    "backgroundColor",
    "fillStyle",
    "opacity",
    "roughness",
    "roundness",
    "link",
    "locked",
    "groupIds",
    "startArrowhead",
    "elbowed",
    "dx",
    "id",
];

#[test]
fn odd_requests_are_answered_never_panicked_on() {
    let mut rng = Lcg(7);
    let values = values();
    let pictures = picture_map(
        "pic.png",
        "0123456789abcdef0123456789abcdef",
        "image/png",
        (64, 32),
        72,
    );
    let mut scene = Scene::empty();
    let mut accepted = 0;
    for round in 0..250u32 {
        let count = 1 + (rng.next() % 4) as usize;
        let mut list = Vec::new();
        let existing: Vec<String> = scene
            .elements()
            .iter()
            .filter(|e| e.container_id().is_none())
            .map(|e| e.id().to_string())
            .collect();
        for n in 0..count {
            let kind = *rng.pick(&TYPES);
            let mut map = valid(kind, &mut rng, &existing);
            map.insert("id".into(), json!(format!("s{round}_{n}")));
            // Now and then something odd.
            for _ in 0..(rng.next() % 3).saturating_sub(1) {
                let field = *rng.pick(&FIELDS);
                map.insert(field.into(), rng.pick(&values).clone());
            }
            list.push(Skeleton(map));
        }
        let mut making = HashedMaking::new(&[&round.to_le_bytes()], 1);
        if add(&mut scene, &list, &pictures, &mut making).is_ok() {
            accepted += 1;
        }
        // Changes of what is there.
        let ids: Vec<String> = scene
            .elements()
            .iter()
            .map(|e| e.id().to_string())
            .collect();
        if !ids.is_empty() {
            let mut change = Map::new();
            change.insert("id".into(), json!(rng.pick(&ids)));
            let good: [(&str, Value); 8] = [
                ("dx", json!((rng.next() % 200) as f64 - 100.0)),
                ("dy", json!((rng.next() % 200) as f64 - 100.0)),
                ("width", json!(20 + rng.next() % 200)),
                ("height", json!(20 + rng.next() % 200)),
                ("text", json!("changed words here")),
                ("angle", json!(0.3)),
                ("end", json!({"id": rng.pick(&ids)})),
                ("backgroundColor", json!("#ffc9c9")),
            ];
            for _ in 0..(1 + rng.next() % 3) {
                let (field, value) = rng.pick(&good).clone();
                change.insert(field.into(), value);
            }
            if rng.next().is_multiple_of(4) {
                let field = *rng.pick(&FIELDS);
                if field != "id" {
                    change.insert(field.into(), rng.pick(&values).clone());
                }
            }
            let _ = update(&mut scene, &[ElementChange(change)], &mut making);
            if rng.next().is_multiple_of(5) {
                let id = rng.pick(&ids).clone();
                let _ = remove(&mut scene, &[id], &mut making);
            }
        }
        // Whatever is left reads back as it was written.
        let stored = scene.to_stored();
        assert_eq!(
            Scene::from_stored(&stored).unwrap().scene.to_stored(),
            stored
        );
        let _ = scene.listing(&innerword_board::ListingHeader {
            reference: "board:1",
            title: "t",
            folder: None,
        });
        let _ = scene.text();
        let _ = scene.pdf_size(32.0, 14_400.0);
    }
    assert!(accepted > 40, "{accepted}");
    assert!(scene.elements().len() > 30, "{}", scene.elements().len());
}

/// A skeleton of the type that is mostly right.
fn valid(kind: &str, rng: &mut Lcg, existing: &[String]) -> Map<String, Value> {
    let x = (rng.next() % 1500) as f64;
    let y = (rng.next() % 1500) as f64;
    let w = 20 + rng.next() % 300;
    let h = 20 + rng.next() % 200;
    let mut map = Map::new();
    map.insert("type".into(), json!(kind));
    map.insert("x".into(), json!(x));
    map.insert("y".into(), json!(y));
    match kind {
        "rectangle" | "ellipse" | "diamond" => {
            map.insert("width".into(), json!(w));
            map.insert("height".into(), json!(h));
            if rng.next().is_multiple_of(2) {
                map.insert("label".into(), json!({"text": "A label that may wrap"}));
            }
        }
        "text" => {
            map.insert("text".into(), json!("Some text\nand more"));
        }
        "arrow" | "line" => {
            map.insert("points".into(), json!([[0, 0], [w, h], [w * 2, 0]]));
            if kind == "arrow" && !existing.is_empty() {
                map.remove("points");
                map.insert("width".into(), json!(w));
                map.insert("start".into(), json!({"id": rng.pick(existing)}));
                map.insert("end".into(), json!({"id": rng.pick(existing)}));
                map.insert("label".into(), json!({"text": "goes"}));
            }
        }
        "freedraw" => {
            map.insert("points".into(), json!([[0, 0], [5, 9], [w, h]]));
        }
        "frame" => {
            if existing.len() > 2 {
                map.insert(
                    "children".into(),
                    json!([rng.pick(existing), rng.pick(existing)]),
                );
            } else {
                map.insert("width".into(), json!(w));
                map.insert("height".into(), json!(h));
            }
        }
        "image" => {
            map.insert("path".into(), json!("pic.png"));
        }
        _ => {}
    }
    map
}
