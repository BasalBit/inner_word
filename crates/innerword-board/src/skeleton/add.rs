//! `add`: skeletons made into elements (section 7.2).

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};

use super::fields::{
    ARROWHEADS, At, FILL_FIELDS, FILL_STYLES, NEVER_FROM_AGENTS, STROKE_STYLES, STYLE_FIELDS, normalize_angle,
};
use super::make::{
    self, DEFAULT_DIMENSION, FRAME_PADDING, Snap, TextStyle, add_bound, bump, invisibly_small_points, is_bindable,
};
use super::{Added, AddedElement, BoardError, Pictures, Skeleton, check_limits, known_ids, new_id};
use crate::geometry::element_bounds;
use crate::limits::{MAX_REQUEST_ELEMENTS, PICTURE_MAX_SIDE};
use crate::metrics;
use crate::scene::NEVER_KEPT;
use crate::{Element, ElementType, FileEntry, Making, Scene};

const COMMON: [&str; 7] = ["type", "id", "x", "y", "angle", "link", "locked"];

/// The fields each type may have besides the common ones and the style.
fn type_fields(kind: &str) -> &'static [&'static str] {
    match kind {
        "rectangle" | "ellipse" | "diamond" => &["width", "height", "label"],
        "text" => &["text", "fontSize", "fontFamily", "textAlign", "verticalAlign"],
        "arrow" => &["width", "height", "points", "start", "end", "startArrowhead", "endArrowhead", "label", "elbowed"],
        "line" => &["width", "height", "points"],
        "freedraw" => &["points"],
        "frame" => &["children", "width", "height", "name"],
        "image" => &["path", "width", "height"],
        _ => &[],
    }
}

const LABEL_FIELDS: [&str; 6] = ["text", "fontSize", "fontFamily", "textAlign", "verticalAlign", "strokeColor"];

/// One skeleton, read and checked.
struct Planned {
    at: At,
    kind: String,
    id: String,
    element: Element,
    label: Option<(TextStyle, String, Option<String>)>,
    start: Option<String>,
    end: Option<String>,
    children: Option<Vec<String>>,
    frame_box: [Option<f64>; 4],
}

pub(super) fn add(
    scene: &mut Scene,
    skeletons: &[Skeleton],
    pictures: &Pictures,
    making: &mut dyn Making,
) -> Result<Added, BoardError> {
    if skeletons.len() > MAX_REQUEST_ELEMENTS {
        return Err(BoardError::TooLarge { what: "request", limit: MAX_REQUEST_ELEMENTS });
    }
    // The ids given: unique, and none already on the whiteboard.
    let mut taken: BTreeSet<String> = scene.elements.iter().map(|e| e.id().to_string()).collect();
    let mut given: BTreeSet<String> = BTreeSet::new();
    for (n, skeleton) in skeletons.iter().enumerate() {
        if let Some(value) = skeleton.0.get("id") {
            let id = At(Some(n)).id(value, "id")?;
            if taken.contains(&id) {
                return Err(At(Some(n)).invalid("id", format!("the whiteboard already has an element {id}; give another id")));
            }
            if !given.insert(id.clone()) {
                return Err(At(Some(n)).invalid("id", format!("two elements of the request have the id {id}")));
            }
        }
    }
    taken.extend(given.iter().cloned());

    let mut planned = Vec::with_capacity(skeletons.len());
    for (n, skeleton) in skeletons.iter().enumerate() {
        planned.push(plan(At(Some(n)), &skeleton.0, pictures, &mut taken, making)?);
    }
    let request_ids: BTreeMap<String, usize> = planned.iter().enumerate().map(|(n, p)| (p.id.clone(), n)).collect();

    // Every id an arrow's end or a frame's children name is on the whiteboard or in the
    // request.
    for p in &planned {
        for (field, id) in [("start", &p.start), ("end", &p.end)] {
            if let Some(id) = id {
                check_named(p.at, field, id, &p.id, scene, &planned, &request_ids, true)?;
            }
        }
        if let Some(children) = &p.children {
            for id in children {
                check_named(p.at, "children", id, &p.id, scene, &planned, &request_ids, false)?;
            }
        }
    }

    // The elements in the order Excalidraw's converter leaves them: the request's in order,
    // then the labels.
    let mut touched: BTreeSet<String> = BTreeSet::new();
    let mut labels: Vec<(usize, Element)> = Vec::new();
    for (n, p) in planned.iter_mut().enumerate() {
        if let Some((style, text, color)) = p.label.take() {
            let label_id = new_id(&mut taken, making);
            let mut label = make::text_element(&label_id, 0.0, 0.0, &text, &style, making);
            label.set("containerId", Value::String(p.id.clone()));
            let stroke = color.unwrap_or_else(|| p.element.str_field("strokeColor").unwrap_or("#1e1e1e").to_string());
            label.set("strokeColor", Value::String(stroke));
            add_bound(&mut p.element, &label_id, "text");
            make::lay_out_label(&mut p.element, &mut label);
            labels.push((n, label));
        }
    }

    // Arrows bound to their ends.
    for n in 0..planned.len() {
        if planned[n].start.is_none() && planned[n].end.is_none() {
            if planned[n].kind == "arrow" {
                // Every new arrow loses half a pixel at each end, as in Excalidraw.
                let mut arrow = planned[n].element.clone();
                make::snap_arrow(&mut arrow, None, None, Snap::New);
                planned[n].element = arrow;
            }
            continue;
        }
        let start = planned[n].start.clone();
        let end = planned[n].end.clone();
        let find = |id: &str| -> Element {
            match request_ids.get(id) {
                Some(&i) => planned[i].element.clone(),
                None => scene.element(id).cloned().unwrap_or_else(|| Element(Map::new())),
            }
        };
        let start_element = start.as_deref().map(find);
        let end_element = end.as_deref().map(find);
        let mut arrow = planned[n].element.clone();
        make::snap_arrow(&mut arrow, start_element.as_ref(), end_element.as_ref(), Snap::New);
        let arrow_id = planned[n].id.clone();
        planned[n].element = arrow;
        for id in [start, end].into_iter().flatten() {
            match request_ids.get(&id) {
                Some(&i) => add_bound(&mut planned[i].element, &arrow_id, "arrow"),
                None => {
                    if let Some(target) = scene.element_mut(&id) {
                        add_bound(target, &arrow_id, "arrow");
                        touched.insert(id.clone());
                    }
                }
            }
        }
    }
    // Labels of arrows follow the arrow's ends.
    for (n, label) in labels.iter_mut() {
        if planned[*n].element.kind() == ElementType::Arrow {
            make::place_label(&planned[*n].element, label);
        }
    }

    // Frames hold their children.
    for n in 0..planned.len() {
        let Some(children) = planned[n].children.clone() else { continue };
        let frame_id = planned[n].id.clone();
        let mut members: Vec<Element> = Vec::new();
        for child in &children {
            let mut follow: Vec<String> = vec![child.clone()];
            let bound: Vec<String> = match request_ids.get(child) {
                Some(&i) => planned[i].element.bound_elements().iter().map(|(id, _)| id.to_string()).collect(),
                None => scene
                    .element(child)
                    .map(|e| e.bound_elements().iter().map(|(id, _)| id.to_string()).collect())
                    .unwrap_or_default(),
            };
            follow.extend(bound);
            for id in follow {
                if let Some(&i) = request_ids.get(&id) {
                    planned[i].element.set("frameId", Value::String(frame_id.clone()));
                    members.push(planned[i].element.clone());
                } else if let Some((_, label)) = labels.iter_mut().find(|(_, l)| l.id() == id) {
                    label.set("frameId", Value::String(frame_id.clone()));
                    members.push(label.clone());
                } else if let Some(element) = scene.element_mut(&id) {
                    element.set("frameId", Value::String(frame_id.clone()));
                    touched.insert(id.clone());
                    members.push(element.clone());
                }
            }
        }
        let bounds = members.iter().filter_map(element_bounds).fold(None, |acc: Option<[f64; 4]>, b| {
            Some(match acc {
                None => b,
                Some(a) => [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])],
            })
        });
        let [given_x, given_y, given_w, given_h] = planned[n].frame_box;
        let (mut x, mut y, mut w, mut h) = (0.0, 0.0, DEFAULT_DIMENSION, DEFAULT_DIMENSION);
        if let Some(b) = bounds {
            x = b[0] - FRAME_PADDING;
            y = b[1] - FRAME_PADDING;
            w = b[2] - b[0] + FRAME_PADDING * 2.0;
            h = b[3] - b[1] + FRAME_PADDING * 2.0;
        }
        let frame = &mut planned[n].element;
        frame.set_f64("x", given_x.unwrap_or(x));
        frame.set_f64("y", given_y.unwrap_or(y));
        frame.set_f64("width", given_w.unwrap_or(w));
        frame.set_f64("height", given_h.unwrap_or(h));
    }

    // Pictures' entries.
    for p in &planned {
        if p.kind == "image"
            && let Some(file_id) = p.element.file_id()
            && !scene.files.contains_key(file_id)
            && let Some(picture) = pictures.values().find(|pic| pic.file.as_str() == file_id)
        {
            scene.files.insert(
                file_id.to_string(),
                FileEntry { mime_type: picture.mime_type.clone(), file: Some(picture.file.clone()), created: making.now_ms() },
            );
        }
    }

    for id in &touched {
        if let Some(element) = scene.element_mut(id) {
            bump(element, making);
        }
    }

    // Indices after the last one, in order.
    let mut last = crate::fractional::greatest(scene.elements.iter().map(|e| e.index())).map(str::to_string);
    let mut added = Vec::new();
    let label_of: BTreeMap<usize, String> = labels.iter().map(|(n, l)| (*n, l.id().to_string())).collect();
    for (n, p) in planned.into_iter().enumerate() {
        let mut element = p.element;
        let index = crate::fractional::key_after(last.as_deref());
        element.set("index", Value::String(index.clone()));
        last = Some(index);
        added.push(AddedElement {
            id: p.id,
            r#type: p.kind,
            with: label_of.get(&n).cloned().into_iter().collect(),
        });
        scene.elements.push(element);
    }
    for (_, mut label) in labels {
        let index = crate::fractional::key_after(last.as_deref());
        label.set("index", Value::String(index.clone()));
        last = Some(index);
        scene.elements.push(label);
    }
    check_limits(scene)?;
    Ok(Added { elements: added, warnings: Vec::new() })
}

#[allow(clippy::too_many_arguments)]
fn check_named(
    at: At,
    field: &str,
    id: &str,
    own: &str,
    scene: &Scene,
    planned: &[Planned],
    request_ids: &BTreeMap<String, usize>,
    binding: bool,
) -> Result<(), BoardError> {
    if id == own {
        return Err(at.invalid(field, format!("an element cannot name itself in {field}")));
    }
    let element = match request_ids.get(id) {
        Some(&i) => &planned[i].element,
        None => match scene.element(id) {
            Some(e) => e,
            None => return Err(BoardError::NotFound { id: id.to_string(), known: known_ids(scene, id) }),
        },
    };
    if binding {
        if !is_bindable(element) {
            return Err(at.invalid(
                field,
                format!(
                    "{id} is {}: an arrow binds to a rectangle, an ellipse, a diamond, a text, a picture or a frame",
                    describe(element)
                ),
            ));
        }
    } else {
        if element.kind() == ElementType::Frame {
            return Err(at.invalid(field, format!("{id} is a frame: a frame cannot hold a frame")));
        }
        if element.container_id().is_some() {
            return Err(at.invalid(field, format!("{id} is a label: name its container, and the label goes with it")));
        }
    }
    Ok(())
}

fn describe(element: &Element) -> String {
    if element.container_id().is_some() {
        return "a label".to_string();
    }
    let kind = element.type_name();
    let article = if kind.starts_with(['a', 'e', 'i', 'o', 'u']) { "an" } else { "a" };
    format!("{article} {kind}")
}

fn plan(
    at: At,
    map: &Map<String, Value>,
    pictures: &Pictures,
    taken: &mut BTreeSet<String>,
    making: &mut dyn Making,
) -> Result<Planned, BoardError> {
    let kind = match map.get("type") {
        Some(Value::String(kind)) => kind.clone(),
        Some(_) => return Err(at.invalid("type", "type must be a string")),
        None => return Err(at.invalid("type", "every element needs a type")),
    };
    if NEVER_KEPT.contains(&kind.as_str()) || kind == "selection" {
        return Err(at.invalid("type", format!("a whiteboard does not take elements of type {kind}")));
    }
    let own = type_fields(&kind);
    if own.is_empty() {
        return Err(at.invalid(
            "type",
            format!("{kind} is not a type of element; use rectangle, ellipse, diamond, text, arrow, line, freedraw, frame or image"),
        ));
    }
    // Every field known.
    for key in map.keys() {
        if let Some((_, why)) = NEVER_FROM_AGENTS.iter().find(|(name, _)| name == key) {
            return Err(at.invalid(key, *why));
        }
        if kind == "text" && (key == "width" || key == "height") {
            return Err(at.invalid(key, "a text has no width or height: they follow its text and its fontSize"));
        }
        if kind == "line" && ["start", "end", "startArrowhead", "endArrowhead", "label"].contains(&key.as_str()) {
            return Err(at.invalid(key, format!("a line has no {key}; use an arrow")));
        }
        if kind == "image" && FILL_FIELDS.contains(&key.as_str()) {
            return Err(at.invalid(key, format!("{key} means nothing on a picture")));
        }
        let known = COMMON.contains(&key.as_str()) || STYLE_FIELDS.contains(&key.as_str()) || own.contains(&key.as_str());
        if !known {
            return Err(at.invalid(key, format!("a {kind} has no field {key}")));
        }
    }
    let id = match map.get("id") {
        Some(value) => at.id(value, "id")?,
        None => new_id(taken, making),
    };
    let x = at.coordinate(map, "x")?;
    let y = at.coordinate(map, "y")?;
    let width = at.size(map, "width")?;
    let height = at.size(map, "height")?;
    let mut planned = Planned {
        at,
        kind: kind.clone(),
        id: id.clone(),
        element: Element(Map::new()),
        label: None,
        start: None,
        end: None,
        children: None,
        frame_box: [None; 4],
    };
    let mut element = match kind.as_str() {
        "rectangle" | "ellipse" | "diamond" => {
            let x = at.required(x, "x", &kind)?;
            let y = at.required(y, "y", &kind)?;
            let label = read_label(at, map)?;
            // A shape with a label and no size is made to fit the label, as Excalidraw does.
            let w = width.unwrap_or(if label.is_some() { 0.0 } else { DEFAULT_DIMENSION });
            let h = height.unwrap_or(if label.is_some() { 0.0 } else { DEFAULT_DIMENSION });
            planned.label = label;
            make::base(&kind, &id, x, y, w, h, making)
        }
        "text" => {
            let x = at.required(x, "x", &kind)?;
            let y = at.required(y, "y", &kind)?;
            let text = match at.string(map, "text")? {
                Some(text) => at.text(text, "text")?,
                None => return Err(at.invalid("text", "a text needs text")),
            };
            let style = TextStyle {
                font_size: at.font_size(map, "fontSize")?.unwrap_or(metrics::DEFAULT_FONT_SIZE),
                font_family: at.font_family(map, "fontFamily")?.unwrap_or(metrics::DEFAULT_FONT_FAMILY),
                text_align: at.one_of(map, "textAlign", &["left", "center", "right"])?.unwrap_or_else(|| "left".into()),
                vertical_align: at
                    .one_of(map, "verticalAlign", &["top", "middle", "bottom"])?
                    .unwrap_or_else(|| "top".into()),
            };
            make::text_element(&id, x, y, &text, &style, making)
        }
        "arrow" | "line" => {
            let x = at.required(x, "x", &kind)?;
            let y = at.required(y, "y", &kind)?;
            let points = at.points(map, "points")?;
            let start = at.end(map, "start")?.flatten();
            let end = at.end(map, "end")?.flatten();
            if points.is_none() && width.is_none() && height.is_none() && start.is_none() && end.is_none() {
                return Err(at.invalid("points", format!("an {kind} needs points, or a width and height, or start and end")));
            }
            if kind == "arrow" {
                if let Some(Value::Bool(true)) = map.get("elbowed") {
                    return Err(at.invalid("elbowed", "elbow arrows are not made by agents; leave elbowed out"));
                }
                if map.get("elbowed").is_some_and(|v| !matches!(v, Value::Bool(false))) {
                    return Err(at.invalid("elbowed", "elbowed must be false, or left out"));
                }
            }
            let points = points.unwrap_or_else(|| {
                let w = width.unwrap_or(DEFAULT_DIMENSION);
                let h = height.unwrap_or(0.0);
                vec![(0.0, 0.0), (w, h)]
            });
            if invisibly_small_points(&points) {
                return Err(at.invalid("points", format!("the {kind}'s points all lie within 1 px of each other: it cannot be seen")));
            }
            let mut element = make::base(&kind, &id, x, y, 0.0, 0.0, making);
            make::set_points(&mut element, x, y, &points);
            element.set("lastCommittedPoint", Value::Null);
            element.set("startBinding", Value::Null);
            element.set("endBinding", Value::Null);
            if kind == "arrow" {
                let read_head = |field: &str, default: Option<&str>| -> Result<Value, BoardError> {
                    match map.get(field) {
                        None => Ok(default.map(|d| Value::String(d.into())).unwrap_or(Value::Null)),
                        Some(Value::Null) => Ok(Value::Null),
                        Some(_) => Ok(Value::String(at.one_of(map, field, &ARROWHEADS)?.unwrap_or_default())),
                    }
                };
                element.set("startArrowhead", read_head("startArrowhead", None)?);
                element.set("endArrowhead", read_head("endArrowhead", Some("arrow"))?);
                element.set("elbowed", Value::Bool(false));
                planned.label = read_label(at, map)?;
                planned.start = start;
                planned.end = end;
            } else {
                element.set("startArrowhead", Value::Null);
                element.set("endArrowhead", Value::Null);
            }
            element
        }
        "freedraw" => {
            let x = at.required(x, "x", &kind)?;
            let y = at.required(y, "y", &kind)?;
            let points = at.points(map, "points")?.ok_or_else(|| at.invalid("points", "a freedraw needs points"))?;
            if invisibly_small_points(&points) {
                return Err(at.invalid("points", "the freedraw's points all lie within 1 px of each other: it cannot be seen"));
            }
            let mut element = make::base(&kind, &id, x, y, 0.0, 0.0, making);
            make::set_freedraw_points(&mut element, &points);
            element.set("pressures", Value::Array(Vec::new()));
            element.set("simulatePressure", Value::Bool(true));
            element.set("lastCommittedPoint", Value::Null);
            element
        }
        "frame" => {
            let children = read_children(at, map)?;
            if children.is_none() && (x.is_none() || y.is_none() || width.is_none() || height.is_none()) {
                return Err(at.invalid("children", "a frame needs children, or x, y, width and height"));
            }
            planned.frame_box = [x, y, width, height];
            planned.children = children;
            let mut element = make::base(&kind, &id, x.unwrap_or(0.0), y.unwrap_or(0.0), width.unwrap_or(0.0), height.unwrap_or(0.0), making);
            let name = match at.string(map, "name")? {
                Some(name) if !name.trim().is_empty() => Value::String(at.text(name, "name")?),
                _ => Value::Null,
            };
            element.set("name", name);
            element
        }
        "image" => {
            let x = at.required(x, "x", &kind)?;
            let y = at.required(y, "y", &kind)?;
            let path = at.string(map, "path")?.ok_or_else(|| at.invalid("path", "a picture needs the path of a picture file on this Mac"))?;
            let picture = pictures
                .get(path)
                .ok_or_else(|| at.invalid("path", format!("the picture {path} was not read; give the path of a picture file on this Mac")))?;
            let (w, h) = picture_size(picture, width, height);
            let mut element = make::base(&kind, &id, x, y, w, h, making);
            element.set("strokeColor", Value::String("transparent".into()));
            element.set("status", Value::String("saved".into()));
            element.set("fileId", Value::String(picture.file.as_str().to_string()));
            element.set("scale", Value::Array(vec![Value::from(1), Value::from(1)]));
            element.set("crop", Value::Null);
            element
        }
        _ => unreachable_kind(at, &kind)?,
    };
    apply_common(at, map, &mut element)?;
    planned.element = element;
    Ok(planned)
}

fn unreachable_kind(at: At, kind: &str) -> Result<Element, BoardError> {
    Err(at.invalid("type", format!("{kind} is not a type of element")))
}

/// The common fields: angle, style, groups, link, locked.
pub(super) fn apply_common(at: At, map: &Map<String, Value>, element: &mut Element) -> Result<(), BoardError> {
    if let Some(angle) = at.number(map, "angle")? {
        element.set_f64("angle", normalize_angle(angle));
    }
    if let Some(color) = at.color(map, "strokeColor")? {
        element.set("strokeColor", Value::String(color));
    }
    if let Some(color) = at.color(map, "backgroundColor")? {
        element.set("backgroundColor", Value::String(color));
    }
    if let Some(fill) = at.one_of(map, "fillStyle", &FILL_STYLES)? {
        element.set("fillStyle", Value::String(fill));
    }
    if let Some(width) = at.number(map, "strokeWidth")? {
        if !(width > 0.0 && width <= 100.0) {
            return Err(at.invalid("strokeWidth", "strokeWidth must be more than 0 and at most 100"));
        }
        element.set_f64("strokeWidth", width);
    }
    if let Some(style) = at.one_of(map, "strokeStyle", &STROKE_STYLES)? {
        element.set("strokeStyle", Value::String(style));
    }
    if let Some(roughness) = at.number(map, "roughness")? {
        if ![0.0, 1.0, 2.0].contains(&roughness) {
            return Err(at.invalid("roughness", "roughness must be 0, 1 or 2"));
        }
        element.set_f64("roughness", roughness);
    }
    if let Some(opacity) = at.number(map, "opacity")? {
        if !(0.0..=100.0).contains(&opacity) {
            return Err(at.invalid("opacity", "opacity must be between 0 and 100"));
        }
        element.set_f64("opacity", opacity);
    }
    if let Some(roundness) = at.roundness(map, "roundness")? {
        element.set("roundness", roundness);
    }
    if let Some(groups) = at.group_ids(map, "groupIds")? {
        element.set("groupIds", Value::Array(groups.into_iter().map(Value::String).collect()));
    }
    if let Some(link) = at.link(map, "link")? {
        element.set("link", link.map(Value::String).unwrap_or(Value::Null));
    }
    if let Some(locked) = at.boolean(map, "locked")? {
        element.set("locked", Value::Bool(locked));
    }
    Ok(())
}

fn read_label(at: At, map: &Map<String, Value>) -> Result<Option<(TextStyle, String, Option<String>)>, BoardError> {
    let Some(value) = map.get("label") else { return Ok(None) };
    let Value::Object(label) = value else {
        return Err(at.invalid("label", "label must be {\"text\": \"…\"}"));
    };
    for key in label.keys() {
        if !LABEL_FIELDS.contains(&key.as_str()) {
            return Err(at.invalid("label", format!("a label has no field {key}")));
        }
    }
    let text = match at.string(label, "text").map_err(|_| at.invalid("label", "a label's text must be a string"))? {
        Some(text) => at.text(text, "label")?,
        None => return Err(at.invalid("label", "a label needs text")),
    };
    let style = TextStyle {
        font_size: at.font_size(label, "fontSize").map_err(rename("label"))?.unwrap_or(metrics::DEFAULT_FONT_SIZE),
        font_family: at.font_family(label, "fontFamily").map_err(rename("label"))?.unwrap_or(metrics::DEFAULT_FONT_FAMILY),
        text_align: at
            .one_of(label, "textAlign", &["left", "center", "right"])
            .map_err(rename("label"))?
            .unwrap_or_else(|| "center".into()),
        vertical_align: at
            .one_of(label, "verticalAlign", &["top", "middle", "bottom"])
            .map_err(rename("label"))?
            .unwrap_or_else(|| "middle".into()),
    };
    let color = at.color(label, "strokeColor").map_err(rename("label"))?;
    Ok(Some((style, text, color)))
}

/// An error about a field of a label, named `label.<field>`.
fn rename(prefix: &'static str) -> impl Fn(BoardError) -> BoardError {
    move |error| match error {
        BoardError::Invalid { element, field, message } => BoardError::Invalid {
            element,
            field: Some(match field {
                Some(f) => format!("{prefix}.{f}"),
                None => prefix.to_string(),
            }),
            message,
        },
        other => other,
    }
}

fn read_children(at: At, map: &Map<String, Value>) -> Result<Option<Vec<String>>, BoardError> {
    match map.get("children") {
        None => Ok(None),
        Some(Value::Array(items)) => {
            let mut out = Vec::new();
            for item in items {
                let id = at.id(item, "children")?;
                if !out.contains(&id) {
                    out.push(id);
                }
            }
            Ok(Some(out))
        }
        Some(_) => Err(at.invalid("children", "children must be a list of ids")),
    }
}

/// The drawn size of a picture (section 7.2): its own size on the page, at most 1,440 on
/// a side; with one of width and height, the other follows its proportions.
pub(crate) fn picture_size(picture: &super::Picture, width: Option<f64>, height: Option<f64>) -> (f64, f64) {
    let (px, py) = (picture.pixels.0.max(1) as f64, picture.pixels.1.max(1) as f64);
    let density = if picture.density == 0 { 72.0 } else { picture.density as f64 };
    match (width, height) {
        (Some(w), Some(h)) => (w, h),
        (Some(w), None) => (w, (w * py / px).max(1.0)),
        (None, Some(h)) => ((h * px / py).max(1.0), h),
        (None, None) => {
            let (mut w, mut h) = (px * 72.0 / density, py * 72.0 / density);
            let longest = w.max(h);
            if longest > PICTURE_MAX_SIDE {
                let scale = PICTURE_MAX_SIDE / longest;
                w *= scale;
                h *= scale;
            }
            (w.max(1.0), h.max(1.0))
        }
    }
}
