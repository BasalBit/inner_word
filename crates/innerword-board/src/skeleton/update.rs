//! `update`: changes of elements by id, and what follows them (section 7.3).

use std::collections::BTreeSet;

use serde_json::{Map, Value};

use super::add::apply_common;
use super::fields::{ARROWHEADS, At, FILL_FIELDS, NEVER_FROM_AGENTS, STYLE_FIELDS, normalize_angle};
use super::make::{self, Snap, TextStyle, add_bound, bump, invisibly_small_points, is_bindable, label_id, remove_bound};
use super::{BoardError, ElementChange, Updated, check_limits, known_ids, new_id};
use crate::limits::MAX_REQUEST_ELEMENTS;
use crate::metrics;
use crate::{Element, ElementType, Making, Scene};

const CHANGE_FIELDS: [&str; 20] = [
    "id",
    "x",
    "y",
    "dx",
    "dy",
    "width",
    "height",
    "angle",
    "text",
    "fontSize",
    "fontFamily",
    "textAlign",
    "verticalAlign",
    "link",
    "locked",
    "name",
    "start",
    "end",
    "points",
    "startArrowhead",
];
const MORE_FIELDS: [&str; 1] = ["endArrowhead"];
const LABEL_CHANGE_FIELDS: [&str; 8] =
    ["id", "text", "fontSize", "fontFamily", "textAlign", "verticalAlign", "strokeColor", "opacity"];
const FONT_FIELDS: [&str; 4] = ["fontSize", "fontFamily", "textAlign", "verticalAlign"];

pub(super) fn update(scene: &mut Scene, changes: &[ElementChange], making: &mut dyn Making) -> Result<Updated, BoardError> {
    if changes.len() > MAX_REQUEST_ELEMENTS {
        return Err(BoardError::TooLarge { what: "request", limit: MAX_REQUEST_ELEMENTS });
    }
    let mut changed: Vec<String> = Vec::new();
    let mut warnings = Vec::new();
    let mut taken: BTreeSet<String> = scene.elements.iter().map(|e| e.id().to_string()).collect();
    for (n, change) in changes.iter().enumerate() {
        let at = At(Some(n));
        let map = &change.0;
        let id = match map.get("id") {
            Some(value) => at.id(value, "id")?,
            None => return Err(at.invalid("id", "a change names its element by id")),
        };
        let Some(element) = scene.element(&id).cloned() else {
            return Err(BoardError::NotFound { id: id.clone(), known: known_ids(scene, &id) });
        };
        check_fields(at, map, &element)?;
        let mut touched = Touched::default();
        if element.container_id().is_some() {
            change_label(at, map, scene, &id, making, &mut touched)?;
        } else {
            change_element(at, map, scene, &id, &mut taken, making, &mut touched, &mut warnings)?;
        }
        for id in touched.ids {
            if let Some(element) = scene.element_mut(&id) {
                bump(element, making);
            }
            if !changed.contains(&id) {
                changed.push(id);
            }
        }
    }
    check_limits(scene)?;
    Ok(Updated { changed, warnings })
}

/// The ids an operation changed, in order.
#[derive(Default)]
pub(super) struct Touched {
    pub ids: Vec<String>,
}

impl Touched {
    pub(super) fn add(&mut self, id: &str) {
        if !self.ids.iter().any(|i| i == id) {
            self.ids.push(id.to_string());
        }
    }
}

fn check_fields(at: At, map: &Map<String, Value>, element: &Element) -> Result<(), BoardError> {
    let kind = element.kind();
    for key in map.keys() {
        if let Some((_, why)) = NEVER_FROM_AGENTS.iter().find(|(name, _)| name == key) {
            return Err(at.invalid(key, *why));
        }
        match key.as_str() {
            "type" => return Err(at.invalid(key, "the type of an element cannot change; remove it and add another")),
            "path" => {
                return Err(at.invalid(key, "the file of a picture cannot change; remove the element and add another"));
            }
            "label" => return Err(at.invalid(key, "a label's text is changed with text")),
            "children" => return Err(at.invalid(key, "a frame's children are not changed; remove the frame and add another")),
            _ => {}
        }
        if element.container_id().is_some() {
            if !LABEL_CHANGE_FIELDS.contains(&key.as_str()) {
                return Err(at.invalid(
                    key,
                    format!(
                        "{} is a label: it moves with its container {}; change the container",
                        element.id(),
                        element.container_id().unwrap_or("")
                    ),
                ));
            }
            continue;
        }
        let known = CHANGE_FIELDS.contains(&key.as_str())
            || MORE_FIELDS.contains(&key.as_str())
            || STYLE_FIELDS.contains(&key.as_str());
        if !known {
            return Err(at.invalid(key, format!("a change has no field {key}")));
        }
        let fits = match key.as_str() {
            "width" | "height" => kind != ElementType::Text,
            "text" | "fontSize" | "fontFamily" | "textAlign" | "verticalAlign" => {
                kind == ElementType::Text || kind.is_shape() || kind == ElementType::Arrow
            }
            "name" => kind == ElementType::Frame,
            "start" | "end" | "startArrowhead" | "endArrowhead" => kind == ElementType::Arrow,
            "points" => kind.is_linear() || kind == ElementType::Freedraw,
            "angle" => kind != ElementType::Frame,
            k if FILL_FIELDS.contains(&k) => kind != ElementType::Image,
            _ => true,
        };
        if !fits {
            let why = match key.as_str() {
                "width" | "height" => "a text's size follows its text; change fontSize".to_string(),
                "angle" => "a frame is not rotated".to_string(),
                k if FILL_FIELDS.contains(&k) => format!("{k} means nothing on a picture"),
                k => format!("{} is {}, which has no {k}", element.id(), element.type_name()),
            };
            return Err(at.invalid(key, why));
        }
    }
    for (absolute, relative) in [("x", "dx"), ("y", "dy")] {
        if map.contains_key(absolute) && map.contains_key(relative) {
            return Err(at.invalid(relative, format!("give {absolute} or {relative}, not both")));
        }
    }
    Ok(())
}

/// A change of a label: its text, font, colour, opacity; then laid out again in its
/// container.
fn change_label(
    at: At,
    map: &Map<String, Value>,
    scene: &mut Scene,
    id: &str,
    making: &mut dyn Making,
    touched: &mut Touched,
) -> Result<(), BoardError> {
    let container_id = scene.element(id).and_then(|e| e.container_id()).unwrap_or("").to_string();
    {
        let label = scene.element_mut(id).ok_or_else(|| at.invalid("id", "the label is gone"))?;
        apply_text_fields(at, map, label)?;
        if let Some(color) = at.color(map, "strokeColor")? {
            label.set("strokeColor", Value::String(color));
        }
        if let Some(opacity) = at.number(map, "opacity")? {
            if !(0.0..=100.0).contains(&opacity) {
                return Err(at.invalid("opacity", "opacity must be between 0 and 100"));
            }
            label.set_f64("opacity", opacity);
        }
    }
    touched.add(id);
    if scene.element(&container_id).is_some() {
        relayout_label(scene, &container_id, touched, making);
    } else if let Some(label) = scene.element_mut(id) {
        make::remeasure_free_text(label);
    }
    Ok(())
}

/// The text and font fields of a change, set on a text element (not laid out).
fn apply_text_fields(at: At, map: &Map<String, Value>, text: &mut Element) -> Result<(), BoardError> {
    if let Some(value) = map.get("text") {
        let Some(value) = value.as_str() else { return Err(at.invalid("text", "text must be a string")) };
        let value = at.text(value, "text")?;
        text.set("originalText", Value::String(value.clone()));
        text.set("text", Value::String(value));
    }
    if let Some(size) = at.font_size(map, "fontSize")? {
        text.set_f64("fontSize", size);
    }
    if let Some(family) = at.font_family(map, "fontFamily")? {
        text.set("fontFamily", Value::from(family));
        text.set_f64("lineHeight", metrics::line_height(family));
    }
    if let Some(align) = at.one_of(map, "textAlign", &["left", "center", "right"])? {
        text.set("textAlign", Value::String(align));
    }
    if let Some(align) = at.one_of(map, "verticalAlign", &["top", "middle", "bottom"])? {
        text.set("verticalAlign", Value::String(align));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn change_element(
    at: At,
    map: &Map<String, Value>,
    scene: &mut Scene,
    id: &str,
    taken: &mut BTreeSet<String>,
    making: &mut dyn Making,
    touched: &mut Touched,
    warnings: &mut Vec<String>,
) -> Result<(), BoardError> {
    let before = scene.element(id).cloned().ok_or_else(|| at.invalid("id", "the element is gone"))?;
    let kind = before.kind();
    let mut element = before.clone();

    // Where it goes.
    let x = at.coordinate(map, "x")?;
    let y = at.coordinate(map, "y")?;
    let dx = at.coordinate(map, "dx")?.unwrap_or(0.0);
    let dy = at.coordinate(map, "dy")?.unwrap_or(0.0);
    let move_x = x.map(|x| x - before.x()).unwrap_or(dx);
    let move_y = y.map(|y| y - before.y()).unwrap_or(dy);

    // Its size.
    let width = at.size(map, "width")?;
    let height = at.size(map, "height")?;
    let mut resized = false;
    if width.is_some() || height.is_some() {
        resized = true;
        match kind {
            ElementType::Arrow | ElementType::Line | ElementType::Freedraw => {
                let points = element.points();
                let (old_w, old_h) = (element.width(), element.height());
                let scale_x = match width {
                    Some(w) if old_w > 0.0 => w / old_w,
                    Some(_) => {
                        warnings.push(format!("The width of {id} was left: its points lie on one vertical line."));
                        1.0
                    }
                    None => 1.0,
                };
                let scale_y = match height {
                    Some(h) if old_h > 0.0 => h / old_h,
                    Some(_) => {
                        warnings.push(format!("The height of {id} was left: its points lie on one horizontal line."));
                        1.0
                    }
                    None => 1.0,
                };
                let scaled: Vec<(f64, f64)> = points.iter().map(|p| (p.0 * scale_x, p.1 * scale_y)).collect();
                set_any_points(&mut element, &kind, &scaled);
            }
            ElementType::Image => {
                let (old_w, old_h) = (element.width().max(1.0), element.height().max(1.0));
                let (w, h) = match (width, height) {
                    (Some(w), Some(h)) => (w, h),
                    (Some(w), None) => (w, (w * old_h / old_w).max(1.0)),
                    (None, Some(h)) => ((h * old_w / old_h).max(1.0), h),
                    (None, None) => (old_w, old_h),
                };
                element.set_f64("width", w);
                element.set_f64("height", h);
            }
            _ => {
                if let Some(w) = width {
                    element.set_f64("width", w);
                }
                if let Some(h) = height {
                    element.set_f64("height", h);
                }
            }
        }
    }
    if let Some(points) = at.points(map, "points")? {
        if invisibly_small_points(&points) {
            return Err(at.invalid("points", "the points all lie within 1 px of each other: the element could not be seen"));
        }
        set_any_points(&mut element, &kind, &points);
        resized = true;
    }
    if move_x != 0.0 || move_y != 0.0 {
        element.set_f64("x", element.x() + move_x);
        element.set_f64("y", element.y() + move_y);
    }
    let mut turned = false;
    if let Some(angle) = at.number(map, "angle")? {
        element.set_f64("angle", normalize_angle(angle));
        turned = true;
    }
    apply_common(at, map, &mut element)?;
    if let Some(name) = map.get("name") {
        let value = match name {
            Value::Null => Value::Null,
            Value::String(text) if text.trim().is_empty() => Value::Null,
            Value::String(text) => Value::String(at.text(text, "name")?),
            _ => return Err(at.invalid("name", "name must be a string")),
        };
        element.set("name", value);
    }
    for field in ["startArrowhead", "endArrowhead"] {
        match map.get(field) {
            None => {}
            Some(Value::Null) => element.set(field, Value::Null),
            Some(_) => {
                let head = at.one_of(map, field, &ARROWHEADS)?.unwrap_or_default();
                element.set(field, Value::String(head));
            }
        }
    }

    // A free text: its text and font, measured again.
    if kind == ElementType::Text {
        let mut text = element.clone();
        apply_text_fields(at, map, &mut text)?;
        if FONT_FIELDS.iter().chain(["text"].iter()).any(|f| map.contains_key(*f)) {
            make::remeasure_free_text(&mut text);
        }
        element = text;
    }

    // Bindings of an arrow.
    let mut rebind = false;
    if kind == ElementType::Arrow {
        for (field, key) in [("start", "startBinding"), ("end", "endBinding")] {
            let Some(new_end) = at.end(map, field)? else { continue };
            rebind = true;
            let old = element.binding(field).map(str::to_string);
            if let Some(old) = &old
                && Some(old) != new_end.as_ref()
                && let Some(target) = scene.element_mut(old)
            {
                let other = if field == "start" { element.binding("end") } else { element.binding("start") };
                if other != Some(old.as_str()) && remove_bound(target, id) {
                    touched.add(old);
                }
            }
            match new_end {
                None => element.set(key, Value::Null),
                Some(target_id) => {
                    if target_id == id {
                        return Err(at.invalid(field, "an arrow cannot be bound to itself"));
                    }
                    let Some(target) = scene.element(&target_id) else {
                        return Err(BoardError::NotFound { id: target_id.clone(), known: known_ids(scene, &target_id) });
                    };
                    if !is_bindable(target) {
                        return Err(at.invalid(
                            field,
                            format!("{target_id} is a {}: an arrow binds to a shape, a text, a picture or a frame", target.type_name()),
                        ));
                    }
                    // The binding itself is written by the snap below.
                    let mut placeholder = Map::new();
                    placeholder.insert("elementId".into(), Value::String(target_id.clone()));
                    element.set(key, Value::Object(placeholder));
                    if let Some(target) = scene.element_mut(&target_id) {
                        add_bound(target, id, "arrow");
                        touched.add(&target_id);
                    }
                }
            }
        }
    }

    let moved = move_x != 0.0 || move_y != 0.0;
    store(scene, id, element);
    touched.add(id);

    // A label: created, rewritten, or laid out again.
    let wants_text = map.contains_key("text") && kind != ElementType::Text;
    let font_only = FONT_FIELDS.iter().any(|f| map.contains_key(*f)) && kind != ElementType::Text;
    if wants_text || font_only {
        match scene.element(id).and_then(label_id) {
            Some(label) => {
                if let Some(text) = scene.element_mut(&label) {
                    apply_text_fields(at, map, text)?;
                }
                touched.add(&label);
            }
            None => {
                if !wants_text {
                    return Err(at.invalid(
                        FONT_FIELDS.iter().find(|f| map.contains_key(**f)).copied().unwrap_or("fontSize"),
                        format!("{id} has no label; give it text first"),
                    ));
                }
                let text = at.text(map.get("text").and_then(Value::as_str).unwrap_or(""), "text")?;
                let style = TextStyle {
                    font_size: at.font_size(map, "fontSize")?.unwrap_or(metrics::DEFAULT_FONT_SIZE),
                    font_family: at.font_family(map, "fontFamily")?.unwrap_or(metrics::DEFAULT_FONT_FAMILY),
                    text_align: at.one_of(map, "textAlign", &["left", "center", "right"])?.unwrap_or_else(|| "center".into()),
                    vertical_align: at
                        .one_of(map, "verticalAlign", &["top", "middle", "bottom"])?
                        .unwrap_or_else(|| "middle".into()),
                };
                let label_id = new_id(taken, making);
                let mut label = make::text_element(&label_id, 0.0, 0.0, &text, &style, making);
                label.set("containerId", Value::String(id.to_string()));
                let container = scene.element(id).cloned().unwrap_or_else(|| Element(Map::new()));
                label.set("strokeColor", Value::String(container.str_field("strokeColor").unwrap_or("#1e1e1e").to_string()));
                if let Some(frame) = container.frame_id() {
                    label.set("frameId", Value::String(frame.to_string()));
                }
                let last = crate::fractional::greatest(scene.elements.iter().map(|e| e.index())).map(str::to_string);
                label.set("index", Value::String(crate::fractional::key_after(last.as_deref())));
                if let Some(container) = scene.element_mut(id) {
                    add_bound(container, &label_id, "text");
                }
                scene.elements.push(label);
                touched.add(&label_id);
            }
        }
    }

    // What follows.
    let geometry = moved || resized || turned || wants_text || font_only || rebind;
    if geometry {
        follow(scene, id, move_x, move_y, moved, touched, making);
    } else if scene.element(id).and_then(label_id).is_some() {
        // A style change: the label keeps its place.
    }
    if kind == ElementType::Frame && moved {
        let children: Vec<String> = scene
            .elements
            .iter()
            .filter(|e| e.frame_id() == Some(id) && e.container_id().is_none())
            .map(|e| e.id().to_string())
            .collect();
        // Every child moves first, then what follows each.
        for child in &children {
            if let Some(element) = scene.element_mut(child) {
                element.set_f64("x", element.x() + move_x);
                element.set_f64("y", element.y() + move_y);
            }
            touched.add(child);
        }
        for child in &children {
            follow(scene, child, move_x, move_y, true, touched, making);
        }
    }

    // Nothing it made may be invisibly small.
    if let Some(element) = scene.element(id)
        && (element.kind().is_linear() || element.kind() == ElementType::Freedraw)
        && invisibly_small_points(&element.points())
    {
        return Err(at.invalid("points", format!("{id} would be invisibly small")));
    }
    Ok(())
}

fn set_any_points(element: &mut Element, kind: &ElementType, points: &[(f64, f64)]) {
    if *kind == ElementType::Freedraw {
        make::set_freedraw_points(element, points);
    } else {
        let (x, y) = (element.x(), element.y());
        make::set_points(element, x, y, points);
    }
}

fn store(scene: &mut Scene, id: &str, element: Element) {
    if let Some(slot) = scene.element_mut(id) {
        *slot = element;
    }
}

/// What follows a change of the element's place, size, angle or label: its label laid out
/// again, the arrows bound to it put back on its outline, and (for an arrow) its own ends
/// put back on theirs.
pub(super) fn follow(
    scene: &mut Scene,
    id: &str,
    _dx: f64,
    _dy: f64,
    _moved: bool,
    touched: &mut Touched,
    making: &mut dyn Making,
) {
    let Some(element) = scene.element(id).cloned() else { return };
    if element.kind() == ElementType::Arrow {
        resnap(scene, id, touched);
    }
    if label_id(&element).is_some() {
        relayout_label(scene, id, touched, making);
    }
    if is_bindable(&element) {
        let arrows: Vec<String> = scene
            .elements
            .iter()
            .filter(|e| e.kind() == ElementType::Arrow && (e.binding("start") == Some(id) || e.binding("end") == Some(id)))
            .map(|e| e.id().to_string())
            .collect();
        for arrow in arrows {
            resnap(scene, &arrow, touched);
        }
    }
}

/// The label of a container laid out again; when the container grew, what is bound to it
/// follows.
fn relayout_label(scene: &mut Scene, container_id: &str, touched: &mut Touched, making: &mut dyn Making) {
    let Some(mut container) = scene.element(container_id).cloned() else { return };
    let Some(label_id) = label_id(&container) else { return };
    let Some(mut label) = scene.element(&label_id).cloned() else { return };
    let resized = make::lay_out_label(&mut container, &mut label);
    store(scene, &label_id, label);
    touched.add(&label_id);
    if resized {
        store(scene, container_id, container);
        touched.add(container_id);
        let arrows: Vec<String> = scene
            .elements
            .iter()
            .filter(|e| {
                e.kind() == ElementType::Arrow
                    && (e.binding("start") == Some(container_id) || e.binding("end") == Some(container_id))
            })
            .map(|e| e.id().to_string())
            .collect();
        for arrow in arrows {
            resnap(scene, &arrow, touched);
        }
    }
    let _ = making;
}

/// An arrow's bound ends put back on the outlines of their elements, its label at its
/// middle.
fn resnap(scene: &mut Scene, arrow_id: &str, touched: &mut Touched) {
    let Some(mut arrow) = scene.element(arrow_id).cloned() else { return };
    let start = arrow.binding("start").and_then(|id| scene.element(id)).cloned();
    let end = arrow.binding("end").and_then(|id| scene.element(id)).cloned();
    if start.is_none() && end.is_none() {
        // Its label still follows it.
    } else {
        make::snap_arrow(&mut arrow, start.as_ref(), end.as_ref(), Snap::Gap);
    }
    if let Some(label) = label_id(&arrow)
        && let Some(mut text) = scene.element(&label).cloned()
    {
        make::place_label(&arrow, &mut text);
        store(scene, &label, text);
        touched.add(&label);
    }
    store(scene, arrow_id, arrow);
    touched.add(arrow_id);
}
