//! The converter's cross-check (`docs/whiteboard-design.md`, 9.2 B11): every skeleton file
//! of `innerword-board`'s corpus (`tests/corpus/`, its `README.md` says how it is made) is
//! made into elements twice, by `innerword_board::add` on an empty scene and by Excalidraw's
//! own `convertToExcalidrawElements` then `restoreElements(…, null, {refreshDimensions,
//! repairBindings})` on the export page (`convertSkeletons`), and the two are held together:
//!
//! - the same elements, a label matched by its container (the two sides give a label an id
//!   of its own), in the same order of their fractional indices;
//! - the same fields with the same values, except `seed`, `versionNonce`, `updated`,
//!   `index` and `version`, and a label's id wherever it is named (`boundElements`);
//! - a text's width within 3 % (Rust sums the advance widths of the bundled fonts; WebKit
//!   shapes the text, with the font's kerning), and what follows from a width by Excalidraw's
//!   own rules within what that width differs by: the place of a centred text (half the
//!   difference) or a right-aligned one (all of it), never of a left-aligned one, the size
//!   of a shape fitted to its label and the place of its label;
//! - the ends of a bound arrow within 1 px.
//!
//! Known and deliberate, and only noted (the corpus's `README.md`, `innerword-board`'s
//! report): a freedraw's `width`, `height`, `pressures` and `simulatePressure`, which
//! Excalidraw's converter leaves at 0 and out where the writer's own drawing has them.
//!
//! When the two disagree beyond that, each difference is named in the failure: the file,
//! the element, the field, our value and Excalidraw's. `INNERWORD_WEB_CROSSCHECK_DUMP=<dir>`
//! writes both conversions of every file there, for whoever looks into one.

use std::collections::{BTreeMap, BTreeSet};

use super::*;

use innerword_board::{HashedMaking, Pictures, Scene, Skeleton, add, metrics};
use innerword_board_web::Assets;
use serde_json::Value;

/// The corpus of `innerword-board`.
fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../innerword-board/tests/corpus")
}

/// Fields whose values are made, not converted (section 9.2 B11).
const NOT_COMPARED: &[&str] = &["seed", "versionNonce", "updated", "index", "version"];

/// A freedraw's fields that the two converters write differently on purpose (the corpus's
/// `README.md`).
const FREEDRAW_KNOWN: &[&str] = &["width", "height", "pressures", "simulatePressure"];

/// How far a text's width may lie from Excalidraw's, as a share of Excalidraw's.
const TEXT_WIDTH_SHARE: f64 = 0.03;

/// How far a bound arrow's ends may lie from Excalidraw's, in pixels.
const BOUND_END_PX: f64 = 1.0;

/// Below this two numbers are the same (Excalidraw's floats against Rust's rounding).
const SAME: f64 = 0.001;

/// The skeletons as Excalidraw's converter takes them. An agent's skeleton is Excalidraw's
/// own form (section 7.2) but for two things the agents' form adds, which Excalidraw's
/// converter does not read: a frame given by its size alone may leave `children` out
/// (Excalidraw reads `children` of every frame and throws without it; an empty list means
/// the same to it), and a `fontFamily`, of a text or of a label, may be the family's name
/// (Excalidraw keeps a name as it is and measures the text in a fallback font: it reads
/// only the number).
fn for_excalidraw(skeletons: &[Skeleton]) -> String {
    fn family_as_number(map: &mut serde_json::Map<String, Value>) {
        if let Some(Value::String(name)) = map.get("fontFamily")
            && let Some(number) = metrics::family_by_name(name)
        {
            map.insert("fontFamily".to_string(), Value::from(number));
        }
    }
    let list: Vec<Value> = skeletons
        .iter()
        .map(|skeleton| {
            let mut map = skeleton.0.clone();
            if map.get("type").and_then(Value::as_str) == Some("frame")
                && !map.contains_key("children")
            {
                map.insert("children".to_string(), Value::Array(Vec::new()));
            }
            family_as_number(&mut map);
            if let Some(Value::Object(label)) = map.get_mut("label") {
                family_as_number(label);
            }
            Value::Object(map)
        })
        .collect();
    Value::Array(list).to_string()
}

/// How an element is matched across the two conversions: its id, or for a label the id of
/// its container.
fn key_of(element: &Value) -> String {
    match element["containerId"].as_str() {
        Some(container) if element["type"] == "text" => format!("label of {container}"),
        _ => element["id"].as_str().unwrap_or("?").to_string(),
    }
}

/// One difference beyond what is allowed.
struct Difference {
    element: String,
    field: String,
    ours: String,
    theirs: String,
    why: String,
}

impl std::fmt::Display for Difference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}.{}: ours {} Excalidraw's {} ({})",
            self.element, self.field, self.ours, self.theirs, self.why
        )
    }
}

fn short(value: Option<&Value>) -> String {
    let text = value.map_or("(absent)".to_string(), Value::to_string);
    if text.chars().count() > 160 {
        format!("{}…", text.chars().take(160).collect::<String>())
    } else {
        text
    }
}

/// What the two conversions of one file say, element by element.
struct Comparison {
    differences: Vec<Difference>,
    /// Differences that are allowed, said in a note.
    allowed: Vec<String>,
    /// The largest share by which a text's width differed.
    widest_text: f64,
}

/// `boundElements` as a set of (type, key): a label named by its container's key, so that
/// the two sides' own ids of a label compare equal, and the order left aside (the corpus's
/// `README.md`: an arrow given before its element comes first on one side).
fn bound_set(value: &Value, label_of: &BTreeMap<String, String>) -> BTreeSet<(String, String)> {
    value
        .as_array()
        .map(|list| {
            list.iter()
                .map(|entry| {
                    let id = entry["id"].as_str().unwrap_or("?").to_string();
                    let id = label_of.get(&id).cloned().unwrap_or(id);
                    (entry["type"].as_str().unwrap_or("?").to_string(), id)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The largest difference between two lists of points, or None when their lengths differ
/// or a value is not a number.
fn points_apart(ours: &Value, theirs: &Value) -> Option<f64> {
    let (a, b) = (ours.as_array()?, theirs.as_array()?);
    if a.len() != b.len() {
        return None;
    }
    let mut most: f64 = 0.;
    for (p, q) in a.iter().zip(b) {
        for axis in 0..2 {
            most = most.max((p[axis].as_f64()? - q[axis].as_f64()?).abs());
        }
    }
    Some(most)
}

/// Two values alike, numbers within [`SAME`] at any depth (a binding's `focus`).
fn same_value(x: Option<&Value>, y: Option<&Value>) -> bool {
    match (x, y) {
        (Some(Value::Number(p)), Some(Value::Number(q))) => {
            (p.as_f64().unwrap_or(f64::NAN) - q.as_f64().unwrap_or(f64::NAN)).abs() <= SAME
        }
        (Some(Value::Array(p)), Some(Value::Array(q))) => {
            p.len() == q.len() && p.iter().zip(q).all(|(a, b)| same_value(Some(a), Some(b)))
        }
        (Some(Value::Object(p)), Some(Value::Object(q))) => {
            p.len() == q.len() && p.iter().all(|(k, a)| same_value(Some(a), q.get(k)))
        }
        _ => x == y,
    }
}

fn compare(ours: &[Value], theirs: &[Value]) -> Comparison {
    let mut out = Comparison {
        differences: Vec::new(),
        allowed: Vec::new(),
        widest_text: 0.,
    };
    let by_key = |list: &[Value]| -> BTreeMap<String, Value> {
        list.iter().map(|e| (key_of(e), e.clone())).collect()
    };
    let (mine, excalidraws) = (by_key(ours), by_key(theirs));
    // The ids of labels, on each side, to their key.
    let labels = |list: &[Value]| -> BTreeMap<String, String> {
        list.iter()
            .filter(|e| e["type"] == "text" && e["containerId"].is_string())
            .map(|e| (e["id"].as_str().unwrap_or("?").to_string(), key_of(e)))
            .collect()
    };
    let (my_labels, their_labels) = (labels(ours), labels(theirs));

    for key in mine.keys().filter(|k| !excalidraws.contains_key(*k)) {
        out.differences.push(Difference {
            element: key.clone(),
            field: "(element)".to_string(),
            ours: "made".to_string(),
            theirs: "(absent)".to_string(),
            why: "only ours has it".to_string(),
        });
    }
    for key in excalidraws.keys().filter(|k| !mine.contains_key(*k)) {
        out.differences.push(Difference {
            element: key.clone(),
            field: "(element)".to_string(),
            ours: "(absent)".to_string(),
            theirs: "made".to_string(),
            why: "only Excalidraw's has it".to_string(),
        });
    }

    // The order of the fractional indices, of the elements both sides have.
    let both = |key: &String| mine.contains_key(key) && excalidraws.contains_key(key);
    let order = |list: &[Value]| -> Vec<String> {
        let mut sorted: Vec<&Value> = list.iter().collect();
        sorted.sort_by(|a, b| a["index"].as_str().cmp(&b["index"].as_str()));
        sorted.into_iter().map(key_of).filter(both).collect()
    };
    let (my_order, their_order) = (order(ours), order(theirs));
    if my_order != their_order {
        out.differences.push(Difference {
            element: "(the scene)".to_string(),
            field: "index".to_string(),
            ours: format!("{my_order:?}"),
            theirs: format!("{their_order:?}"),
            why: "the elements are not in the same order of their indices".to_string(),
        });
    }

    // How much a text's width differs (Excalidraw's minus ours), by key: what follows
    // from it may differ by as much.
    let mut width_apart: BTreeMap<String, f64> = BTreeMap::new();
    for (key, a) in &mine {
        if let Some(b) = excalidraws.get(key)
            && a["type"] == "text"
            && let (Some(x), Some(y)) = (a["width"].as_f64(), b["width"].as_f64())
        {
            width_apart.insert(key.clone(), (x - y).abs());
        }
    }
    let label_apart = |container: &str| {
        width_apart
            .get(&format!("label of {container}"))
            .copied()
            .unwrap_or(0.)
    };

    for (key, a) in &mine {
        let Some(b) = excalidraws.get(key) else {
            continue;
        };
        let kind = a["type"].as_str().unwrap_or("?").to_string();
        let is_label = kind == "text" && a["containerId"].is_string();
        let bound = a["startBinding"].is_object() || a["endBinding"].is_object();
        let fields: BTreeSet<&String> = a
            .as_object()
            .into_iter()
            .chain(b.as_object())
            .flat_map(|map| map.keys())
            .collect();
        for field in fields {
            let field = field.as_str();
            if NOT_COMPARED.contains(&field) || (is_label && field == "id") {
                continue;
            }
            let (x, y) = (a.get(field), b.get(field));
            let differ = |why: &str| Difference {
                element: key.clone(),
                field: field.to_string(),
                ours: short(x),
                theirs: short(y),
                why: why.to_string(),
            };
            if kind == "freedraw" && FREEDRAW_KNOWN.contains(&field) {
                if x != y {
                    out.allowed.push(format!(
                        "{key}.{field}: ours {} Excalidraw's {} (a freedraw's, known)",
                        short(x),
                        short(y)
                    ));
                }
                continue;
            }
            if field == "boundElements" {
                let (p, q) = (
                    bound_set(x.unwrap_or(&Value::Null), &my_labels),
                    bound_set(y.unwrap_or(&Value::Null), &their_labels),
                );
                if p != q {
                    out.differences
                        .push(differ("not the same elements bound to it"));
                }
                continue;
            }
            if field == "points" {
                let apart = points_apart(x.unwrap_or(&Value::Null), y.unwrap_or(&Value::Null));
                let allowed = if bound { BOUND_END_PX } else { SAME };
                match apart {
                    Some(apart) if apart <= allowed => {}
                    Some(apart) => out
                        .differences
                        .push(differ(&format!("a point {apart:.3} px from Excalidraw's"))),
                    None => out
                        .differences
                        .push(differ("not the same number of points")),
                }
                continue;
            }
            match (x.and_then(Value::as_f64), y.and_then(Value::as_f64)) {
                (Some(p), Some(q)) => {
                    let apart = (p - q).abs();
                    // What this field may differ by, and why.
                    let (slack, because) = match (kind.as_str(), field) {
                        ("text", "width") => {
                            let share = apart / q.abs().max(1.);
                            out.widest_text = out.widest_text.max(share);
                            (q.abs() * TEXT_WIDTH_SHARE, "a text's width, 3 %")
                        }
                        ("text", "x") => {
                            // Where a text stands follows from its width only where it
                            // is aligned by its middle or its right edge: a left-aligned
                            // text keeps its x. A label stands in its container, whose
                            // own place and width may differ too: the text area starts
                            // at the container's x plus a share of its width (a padding
                            // in a rectangle, 15 % of it in an ellipse, a quarter in a
                            // diamond), so a left label may move by half the container's
                            // difference, a centred one by half of both, a right one by
                            // both.
                            let text_apart = width_apart.get(key).copied().unwrap_or(0.);
                            let (container_x, container_width) = if is_label {
                                let container = a["containerId"].as_str().unwrap_or("");
                                mine.get(container)
                                    .zip(excalidraws.get(container))
                                    .map(|(c, d)| {
                                        let apart = |field: &str| {
                                            (c[field].as_f64().unwrap_or(0.)
                                                - d[field].as_f64().unwrap_or(0.))
                                            .abs()
                                        };
                                        (apart("x"), apart("width"))
                                    })
                                    .unwrap_or((0., 0.))
                            } else {
                                (0., 0.)
                            };
                            match a["textAlign"].as_str() {
                                Some("center") => (
                                    container_x + (container_width + text_apart) / 2.,
                                    "a centred text's place, by half its width's difference",
                                ),
                                Some("right") => (
                                    container_x + container_width + text_apart,
                                    "a right-aligned text's place, by its width's difference",
                                ),
                                _ if is_label => (
                                    container_x + container_width / 2.,
                                    "a left-aligned label's place, by its container's only",
                                ),
                                _ => (0., "a left-aligned text keeps its place"),
                            }
                        }
                        ("rectangle" | "ellipse" | "diamond", "width") => (
                            2. * label_apart(key),
                            "a shape fitted to its label, by the label's width",
                        ),
                        ("arrow" | "line", "x" | "y" | "width" | "height") if bound => {
                            (BOUND_END_PX, "a bound arrow's ends, 1 px")
                        }
                        _ => (0., "the same value"),
                    };
                    if apart > slack + SAME {
                        out.differences.push(differ(because));
                    }
                }
                _ => {
                    if !same_value(x, y) {
                        out.differences.push(differ("the same value"));
                    }
                }
            }
        }
    }
    out
}

pub(super) fn the_converter_against_excalidraw(report: &mut Report, rules: &Path) {
    let assets = match Assets::find() {
        Ok(assets) => assets,
        Err(why) => {
            report.check(false, &format!("the whiteboard's bundle is found ({why})"));
            return;
        }
    };
    let mut names: Vec<String> = std::fs::read_dir(corpus())
        .map(|list| {
            list.filter_map(|entry| entry.ok()?.file_name().into_string().ok())
                .filter(|name| name.ends_with(".json"))
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    report.check(
        names.len() >= 5,
        &format!("the converter's corpus is found ({names:?})"),
    );
    kerning_measured(report, &assets, rules);
    let dump = std::env::var_os("INNERWORD_WEB_CROSSCHECK_DUMP").map(PathBuf::from);
    for name in &names {
        let text = std::fs::read_to_string(corpus().join(name)).unwrap_or_default();
        let skeletons: Vec<Skeleton> = match serde_json::from_str(&text) {
            Ok(list) => list,
            Err(why) => {
                report.check(false, &format!("{name} is a list of skeletons ({why})"));
                continue;
            }
        };
        let mut scene = Scene::empty();
        let mut making = HashedMaking::new(&[b"crosscheck", name.as_bytes()], 1_795_432_196_771);
        if let Err(why) = add(&mut scene, &skeletons, &Pictures::new(), &mut making) {
            report.check(
                false,
                &format!("{name}: innerword_board::add takes it ({why})"),
            );
            continue;
        }
        let ours: Vec<Value> = scene
            .elements()
            .iter()
            .map(|e| Value::Object(e.json().clone()))
            .collect();
        let theirs = match bundle::convert(&assets, rules, &for_excalidraw(&skeletons)) {
            Ok(Value::Array(list)) => list,
            other => {
                report.check(
                    false,
                    &format!("{name}: Excalidraw's own converter takes it ({other:?})"),
                );
                continue;
            }
        };
        if let Some(dir) = &dump {
            std::fs::create_dir_all(dir).ok();
            for (side, list) in [("ours", &ours), ("theirs", &theirs)] {
                std::fs::write(
                    dir.join(format!("{side}-{name}")),
                    serde_json::to_string_pretty(list).unwrap_or_default(),
                )
                .ok();
            }
        }
        if name == "arrows.json" {
            the_comparison_sees_a_difference(report, &ours, &theirs);
        }
        if name == "text.json" {
            a_text_may_move_only_as_it_is_aligned(report, &ours, &theirs);
        }
        let comparison = compare(&ours, &theirs);
        for allowed in &comparison.allowed {
            report.note(&format!("{name}: {allowed}"));
        }
        report.check(
            comparison.differences.is_empty(),
            &format!(
                "{name}: innerword_board::add makes what Excalidraw's own converter makes ({} elements; texts within {:.1} % of Excalidraw's widths){}",
                ours.len(),
                comparison.widest_text * 100.,
                comparison
                    .differences
                    .iter()
                    .map(|d| format!("\n    {d}"))
                    .collect::<String>()
            ),
        );
    }
}

/// Words whose pairs fonts kern the most (a capital beside a round or slanted letter, a
/// letter beside a full stop), and a few plain ones.
const KERNED_WORDS: &[&str] = &[
    "AV",
    "AWAY",
    "To",
    "Ty",
    "Yo",
    "Yes?",
    "We",
    "LT",
    "P.",
    "Tokyo",
    "WAVE",
    "Typography",
    "Postgres",
    "minimum",
    "Hello, world",
];

/// How far innerword-board's text widths, which sum the fonts' advance widths, lie from
/// what Excalidraw measures in WebKit, which applies the fonts' pair kerning: each family
/// with widths at 20 px, on [`KERNED_WORDS`]. Only noted (the worst cases, for whoever
/// decides whether innerword-board applies kerning); the corpus's own texts are held at 3 %
/// above.
fn kerning_measured(report: &mut Report, assets: &Assets, rules: &Path) {
    let mut skeletons = Vec::new();
    let mut probes = Vec::new();
    for &(family, family_name) in metrics::FAMILIES {
        if !metrics::has_widths(family) {
            continue;
        }
        for (n, word) in KERNED_WORDS.iter().enumerate() {
            let id = format!("k{family}-{n}");
            skeletons.push(serde_json::json!({
                "type": "text", "id": id, "x": 0, "y": 40 * probes.len(),
                "text": word, "fontFamily": family, "fontSize": 20
            }));
            probes.push((
                id,
                family_name,
                *word,
                metrics::text_size(word, family, 20.).0,
            ));
        }
    }
    let theirs = match bundle::convert(assets, rules, &Value::Array(skeletons).to_string()) {
        Ok(Value::Array(list)) => list,
        other => {
            report.check(
                false,
                &format!("the kerning probe is converted by Excalidraw ({other:?})"),
            );
            return;
        }
    };
    let mut apart: Vec<(f64, String)> = probes
        .iter()
        .filter_map(|(id, family, word, ours)| {
            let theirs = theirs.iter().find(|e| e["id"] == id.as_str())?["width"].as_f64()?;
            Some((
                (ours - theirs) / theirs.max(1.),
                format!("{word:?} in {family}: ours {ours:.2} Excalidraw's {theirs:.2}"),
            ))
        })
        .collect();
    report.check(
        apart.len() == probes.len(),
        &format!(
            "the kerning probe measures every word in every family with widths ({} of {})",
            apart.len(),
            probes.len()
        ),
    );
    apart.sort_by(|a, b| b.0.abs().total_cmp(&a.0.abs()));
    let over = apart
        .iter()
        .filter(|(share, _)| share.abs() > TEXT_WIDTH_SHARE)
        .count();
    report.note(&format!(
        "kerning: {over} of {} words lie more than 3 % from Excalidraw's widths; the worst: {}",
        apart.len(),
        apart
            .iter()
            .take(8)
            .map(|(share, what)| format!("{what} ({:+.1} %)", share * 100.))
            .collect::<Vec<_>>()
            .join("; ")
    ));
    let mut by_family: BTreeMap<&str, f64> = BTreeMap::new();
    for (id, family, _, ours) in &probes {
        if let Some(theirs) = theirs
            .iter()
            .find(|e| e["id"] == id.as_str())
            .and_then(|e| e["width"].as_f64())
        {
            let share = ((ours - theirs) / theirs.max(1.)).abs();
            let worst = by_family.entry(family).or_insert(0.);
            *worst = worst.max(share);
        }
    }
    report.note(&format!(
        "kerning: the worst word of each family: {}",
        by_family
            .iter()
            .map(|(family, share)| format!("{family} {:.1} %", share * 100.))
            .collect::<Vec<_>>()
            .join(", ")
    ));
}

/// The comparison is not blind: three differences planted in Excalidraw's side (a colour, a
/// bound arrow's end 2 px off, an element left out) are the three it names.
fn the_comparison_sees_a_difference(report: &mut Report, ours: &[Value], theirs: &[Value]) {
    let mut planted: Vec<Value> = theirs.to_vec();
    for element in planted.iter_mut() {
        if element["id"] == "api" {
            element["strokeColor"] = Value::from("#e03131");
        }
        if element["id"] == "reads"
            && let Some(x) = element["points"][1][0].as_f64()
        {
            element["points"][1][0] = Value::from(x + 2.);
        }
    }
    planted.retain(|element| element["id"] != "gate");
    let found = compare(ours, &planted).differences;
    let named: Vec<String> = found
        .iter()
        .map(|d| format!("{}.{}", d.element, d.field))
        .collect();
    report.check(
        named.len() == 3
            && ["api.strokeColor", "reads.points", "gate.(element)"]
                .iter()
                .all(|n| named.iter().any(|m| m == n)),
        &format!("the cross-check names a difference planted in Excalidraw's side ({named:?})"),
    );
}

/// The place of a text is held by its alignment. Planted in Excalidraw's side: each of
/// three texts 2 % wider than ours (within the 3 % a width may differ), and moved from our
/// place by a share of that difference. A left-aligned text moved by half of it is named
/// (its place has no slack), a centred one moved by 0.6 of it is named (its slack is half),
/// and a right-aligned one moved by 0.9 of it is not (its slack is all of it).
fn a_text_may_move_only_as_it_is_aligned(report: &mut Report, ours: &[Value], theirs: &[Value]) {
    let planted_at = [("title", 0.5), ("centred", 0.6), ("right", 0.9)];
    let mut planted: Vec<Value> = theirs.to_vec();
    for element in planted.iter_mut() {
        for (id, share) in planted_at {
            if element["id"] != id {
                continue;
            }
            let Some(mine) = ours.iter().find(|e| e["id"] == id) else {
                continue;
            };
            let (x, width) = (
                mine["x"].as_f64().unwrap_or(0.),
                mine["width"].as_f64().unwrap_or(0.),
            );
            element["width"] = Value::from(width * 1.02);
            element["x"] = Value::from(x + share * width * 0.02);
        }
    }
    let mut named: Vec<String> = compare(ours, &planted)
        .differences
        .iter()
        .map(|d| format!("{}.{}", d.element, d.field))
        .collect();
    named.sort();
    report.check(
        named == ["centred.x", "title.x"],
        &format!(
            "the cross-check gives a text's place slack only as it is aligned: a left one none, a centred one half its width's difference, a right one all of it ({named:?})"
        ),
    );
}
