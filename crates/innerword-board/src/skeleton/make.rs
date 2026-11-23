//! Making Excalidraw 0.18.1's elements: the fields `newElement` and its siblings write, a
//! label laid out in its container (`redrawTextBoundingBox`), an arrow's ends put on the
//! outlines of the elements it is bound to.

use serde_json::{Map, Value};

use crate::geometry::{self, Point, Shape};
use crate::json::num;
use crate::metrics;
use crate::{Element, ElementType, Making};

/// Excalidraw's `BOUND_TEXT_PADDING`.
pub(crate) const BOUND_TEXT_PADDING: f64 = 5.0;
/// Excalidraw's `ARROW_LABEL_WIDTH_FRACTION` and `ARROW_LABEL_FONT_SIZE_TO_MIN_WIDTH_RATIO`.
const ARROW_LABEL_WIDTH_FRACTION: f64 = 0.7;
const ARROW_LABEL_FONT_SIZE_TO_MIN_WIDTH_RATIO: f64 = 11.0;
/// The padding a frame made from its children has around them (`convertToExcalidrawElements`).
pub(crate) const FRAME_PADDING: f64 = 10.0;
/// The gap between a bound arrow's end and the outline (Excalidraw's least, `max(1, …)`).
pub(crate) const BINDING_GAP: f64 = 1.0;
/// The default size of a shape, an image and a linear element (`DEFAULT_DIMENSION`).
pub(crate) const DEFAULT_DIMENSION: f64 = 100.0;

/// The fields every element has (`_newElementBase`, as `restoreElements` leaves them).
pub(crate) fn base(
    kind: &str,
    id: &str,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    making: &mut dyn Making,
) -> Element {
    let mut map = Map::new();
    map.insert("id".into(), Value::String(id.to_string()));
    map.insert("type".into(), Value::String(kind.to_string()));
    map.insert("x".into(), num(x));
    map.insert("y".into(), num(y));
    map.insert("width".into(), num(width));
    map.insert("height".into(), num(height));
    map.insert("angle".into(), Value::from(0));
    map.insert("strokeColor".into(), Value::String("#1e1e1e".into()));
    map.insert(
        "backgroundColor".into(),
        Value::String("transparent".into()),
    );
    map.insert("fillStyle".into(), Value::String("solid".into()));
    map.insert("strokeWidth".into(), Value::from(2));
    map.insert("strokeStyle".into(), Value::String("solid".into()));
    map.insert("roughness".into(), Value::from(1));
    map.insert("opacity".into(), Value::from(100));
    map.insert("groupIds".into(), Value::Array(Vec::new()));
    map.insert("frameId".into(), Value::Null);
    map.insert("index".into(), Value::Null);
    map.insert("roundness".into(), Value::Null);
    map.insert("seed".into(), Value::from(making.seed()));
    map.insert("version".into(), Value::from(1));
    map.insert("versionNonce".into(), Value::from(making.nonce()));
    map.insert("isDeleted".into(), Value::Bool(false));
    map.insert("boundElements".into(), Value::Array(Vec::new()));
    map.insert("updated".into(), Value::from(making.now_ms()));
    map.insert("link".into(), Value::Null);
    map.insert("locked".into(), Value::Bool(false));
    Element(map)
}

/// What a text is laid out with.
#[derive(Clone, Debug)]
pub(crate) struct TextStyle {
    pub font_size: f64,
    pub font_family: u8,
    pub text_align: String,
    pub vertical_align: String,
}

/// A text element (`newTextElement`): measured, placed by its alignment from `x`, `y`
/// (a centred text is centred on `x`, a middle one on `y`).
pub(crate) fn text_element(
    id: &str,
    x: f64,
    y: f64,
    text: &str,
    style: &TextStyle,
    making: &mut dyn Making,
) -> Element {
    let (width, height) = metrics::text_size(text, style.font_family, style.font_size);
    let offset_x = match style.text_align.as_str() {
        "center" => width / 2.0,
        "right" => width,
        _ => 0.0,
    };
    let offset_y = if style.vertical_align == "middle" {
        height / 2.0
    } else {
        0.0
    };
    let mut element = base(
        "text",
        id,
        x - offset_x,
        y - offset_y,
        width,
        height,
        making,
    );
    set_text_fields(&mut element, text, style);
    element
}

pub(crate) fn set_text_fields(element: &mut Element, text: &str, style: &TextStyle) {
    element.set("text", Value::String(text.to_string()));
    element.set_f64("fontSize", style.font_size);
    element.set("fontFamily", Value::from(style.font_family));
    element.set("textAlign", Value::String(style.text_align.clone()));
    element.set("verticalAlign", Value::String(style.vertical_align.clone()));
    if !element.json().contains_key("containerId") {
        element.set("containerId", Value::Null);
    }
    element.set("originalText", Value::String(text.to_string()));
    element.set("autoResize", Value::Bool(true));
    element.set_f64("lineHeight", metrics::line_height(style.font_family));
}

pub(crate) fn text_style_of(element: &Element) -> TextStyle {
    TextStyle {
        font_size: Some(element.f64_field("fontSize"))
            .filter(|s| *s > 0.0)
            .unwrap_or(metrics::DEFAULT_FONT_SIZE),
        font_family: element
            .json()
            .get("fontFamily")
            .and_then(Value::as_u64)
            .map(|f| f as u8)
            .unwrap_or(metrics::DEFAULT_FONT_FAMILY),
        text_align: element.str_field("textAlign").unwrap_or("left").to_string(),
        vertical_align: element
            .str_field("verticalAlign")
            .unwrap_or("top")
            .to_string(),
    }
}

/// The text a text element was given, unwrapped (`originalText`, else `text`).
pub(crate) fn original_text(element: &Element) -> String {
    element
        .str_field("originalText")
        .filter(|t| !t.is_empty())
        .or_else(|| element.text())
        .unwrap_or("")
        .to_string()
}

/// `computeContainerDimensionForBoundText`.
fn container_dimension_for_text(dimension: f64, kind: &ElementType) -> f64 {
    let dimension = dimension.ceil();
    let padding = BOUND_TEXT_PADDING * 2.0;
    match kind {
        ElementType::Ellipse => ((dimension + padding) / std::f64::consts::SQRT_2 * 2.0).round(),
        ElementType::Arrow => dimension + padding * 8.0,
        ElementType::Diamond => 2.0 * (dimension + padding),
        _ => dimension + padding,
    }
}

/// `getBoundTextMaxWidth`.
fn bound_text_max_width(container: &Element, font_size: f64) -> f64 {
    let width = container.width();
    match container.kind() {
        ElementType::Arrow => (ARROW_LABEL_WIDTH_FRACTION * width)
            .max(font_size * ARROW_LABEL_FONT_SIZE_TO_MIN_WIDTH_RATIO),
        ElementType::Ellipse => {
            (width / 2.0 * std::f64::consts::SQRT_2).round() - BOUND_TEXT_PADDING * 2.0
        }
        ElementType::Diamond => (width / 2.0).round() - BOUND_TEXT_PADDING * 2.0,
        _ => width - BOUND_TEXT_PADDING * 2.0,
    }
}

/// `getBoundTextMaxHeight`.
fn bound_text_max_height(container: &Element, text_height: f64) -> f64 {
    let height = container.height();
    match container.kind() {
        ElementType::Arrow => {
            if height - BOUND_TEXT_PADDING * 8.0 * 2.0 <= 0.0 {
                text_height
            } else {
                height
            }
        }
        ElementType::Ellipse => {
            (height / 2.0 * std::f64::consts::SQRT_2).round() - BOUND_TEXT_PADDING * 2.0
        }
        ElementType::Diamond => (height / 2.0).round() - BOUND_TEXT_PADDING * 2.0,
        _ => height - BOUND_TEXT_PADDING * 2.0,
    }
}

/// `getContainerCoords`: the top left corner of the room a label has.
fn container_coords(container: &Element) -> Point {
    let mut offset_x = BOUND_TEXT_PADDING;
    let mut offset_y = BOUND_TEXT_PADDING;
    match container.kind() {
        ElementType::Ellipse => {
            offset_x += container.width() / 2.0 * (1.0 - std::f64::consts::SQRT_2 / 2.0);
            offset_y += container.height() / 2.0 * (1.0 - std::f64::consts::SQRT_2 / 2.0);
        }
        ElementType::Diamond => {
            offset_x += container.width() / 4.0;
            offset_y += container.height() / 4.0;
        }
        _ => {}
    }
    (container.x() + offset_x, container.y() + offset_y)
}

/// The point of a linear element where its label is centred: its middle point, or the
/// middle of its middle segment (straight segments).
pub(crate) fn linear_label_center(element: &Element) -> Point {
    let points = element.points();
    let (x, y) = (element.x(), element.y());
    if points.is_empty() {
        return (x, y);
    }
    if points.len() % 2 == 1 {
        let p = points[points.len() / 2];
        return (x + p.0, y + p.1);
    }
    let i = points.len() / 2 - 1;
    let (a, b) = (points[i], points[i + 1]);
    (x + (a.0 + b.0) / 2.0, y + (a.1 + b.1) / 2.0)
}

/// `redrawTextBoundingBox` for a label: its text wrapped to the room its container has,
/// measured, the container made taller (or wider, for a word that cannot be broken) when
/// it needs to be, the label placed by its alignment and given the container's angle.
/// Returns whether the container's size changed.
pub(crate) fn lay_out_label(container: &mut Element, label: &mut Element) -> bool {
    let style = text_style_of(label);
    let original = original_text(label);
    let max_width = bound_text_max_width(container, style.font_size);
    let wrapped = metrics::wrap(&original, style.font_family, style.font_size, max_width);
    let (width, height) = metrics::text_size(&wrapped, style.font_family, style.font_size);
    label.set("text", Value::String(wrapped));
    label.set_f64("width", width);
    label.set_f64("height", height);
    let kind = container.kind();
    let mut resized = false;
    if kind != ElementType::Arrow {
        if height > bound_text_max_height(container, height) {
            container.set_f64("height", container_dimension_for_text(height, &kind));
            resized = true;
        }
        if width > max_width {
            container.set_f64("width", container_dimension_for_text(width, &kind));
            resized = true;
        }
    }
    place_label(container, label);
    resized
}

/// `computeBoundTextPosition`, and the label's angle.
pub(crate) fn place_label(container: &Element, label: &mut Element) {
    let (width, height) = (label.width(), label.height());
    let (x, y) = if container.kind().is_linear() {
        let (cx, cy) = linear_label_center(container);
        (cx - width / 2.0, cy - height / 2.0)
    } else {
        let (cx, cy) = container_coords(container);
        let room_h = bound_text_max_height(container, height);
        let room_w = bound_text_max_width(container, label.f64_field("fontSize"));
        let y = match label.str_field("verticalAlign").unwrap_or("middle") {
            "top" => cy,
            "bottom" => cy + (room_h - height),
            _ => cy + (room_h / 2.0 - height / 2.0),
        };
        let x = match label.str_field("textAlign").unwrap_or("center") {
            "left" => cx,
            "right" => cx + (room_w - width),
            _ => cx + (room_w / 2.0 - width / 2.0),
        };
        (x, y)
    };
    label.set_f64("x", x);
    label.set_f64("y", y);
    if !container.kind().is_linear() {
        label.set_f64("angle", container.angle());
    }
}

/// A free text measured again after its text or font changed (`getAdjustedDimensions`
/// for an unrotated text): the side its alignment names stays where it was.
pub(crate) fn remeasure_free_text(element: &mut Element) {
    let style = text_style_of(element);
    let text = original_text(element);
    let (width, height) = metrics::text_size(&text, style.font_family, style.font_size);
    let (old_w, old_h) = (element.width(), element.height());
    let dx = match style.text_align.as_str() {
        "center" => (old_w - width) / 2.0,
        "right" => old_w - width,
        _ => 0.0,
    };
    let dy = if style.vertical_align == "middle" && style.text_align == "center" {
        (old_h - height) / 2.0
    } else {
        0.0
    };
    element.set("text", Value::String(text));
    element.set_f64("x", element.x() + dx);
    element.set_f64("y", element.y() + dy);
    element.set_f64("width", width);
    element.set_f64("height", height);
}

/// Points made relative to their first (which becomes 0, 0), the element moved by as much,
/// its width and height those of the points (`getNormalizedPoints`, `getSizeFromPoints`).
pub(crate) fn set_points(element: &mut Element, x: f64, y: f64, points: &[Point]) {
    let first = points.first().copied().unwrap_or((0.0, 0.0));
    let shifted: Vec<Point> = points
        .iter()
        .map(|p| (p.0 - first.0, p.1 - first.1))
        .collect();
    let (min_x, max_x, min_y, max_y) = extent(&shifted);
    element.set_f64("x", x + first.0);
    element.set_f64("y", y + first.1);
    element.set_f64("width", max_x - min_x);
    element.set_f64("height", max_y - min_y);
    element.set(
        "points",
        Value::Array(
            shifted
                .iter()
                .map(|p| Value::Array(vec![num(p.0), num(p.1)]))
                .collect(),
        ),
    );
}

/// Freedraw points are kept as given (they need not start at 0, 0); width and height are
/// theirs.
pub(crate) fn set_freedraw_points(element: &mut Element, points: &[Point]) {
    let (min_x, max_x, min_y, max_y) = extent(points);
    element.set_f64("width", max_x - min_x);
    element.set_f64("height", max_y - min_y);
    element.set(
        "points",
        Value::Array(
            points
                .iter()
                .map(|p| Value::Array(vec![num(p.0), num(p.1)]))
                .collect(),
        ),
    );
}

fn extent(points: &[Point]) -> (f64, f64, f64, f64) {
    let min_x = points.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
    let max_x = points.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
    let min_y = points.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
    let max_y = points.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
    if min_x.is_finite() {
        (min_x, max_x, min_y, max_y)
    } else {
        (0.0, 0.0, 0.0, 0.0)
    }
}

/// Whether every point lies within 1 px of the first: a line that cannot be seen.
pub(crate) fn invisibly_small_points(points: &[Point]) -> bool {
    let Some(first) = points.first() else {
        return true;
    };
    points
        .iter()
        .all(|p| (p.0 - first.0).abs() < 1.0 && (p.1 - first.1).abs() < 1.0)
}

/// The global points of a linear element.
pub(crate) fn global_points(element: &Element) -> Vec<Point> {
    let (x, y) = (element.x(), element.y());
    element
        .points()
        .into_iter()
        .map(|p| (x + p.0, y + p.1))
        .collect()
}

/// Whether an element can have an arrow bound to it (`isBindableElement`: a shape, a
/// free text, a picture, a frame).
pub(crate) fn is_bindable(element: &Element) -> bool {
    match element.kind() {
        ElementType::Rectangle
        | ElementType::Ellipse
        | ElementType::Diamond
        | ElementType::Image
        | ElementType::Frame => true,
        ElementType::Text => element.container_id().is_none(),
        _ => false,
    }
}

/// How an arrow's ends are placed on the elements they are bound to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Snap {
    /// As `convertToExcalidrawElements` leaves a new arrow: on the outline, then both ends
    /// moved half a pixel inwards on each axis.
    New,
    /// As Excalidraw moves a bound end when its element moved: the binding's gap off the
    /// outline.
    Gap,
}

/// Puts the arrow's bound ends on the outlines of `start` and `end` and writes the
/// bindings (`startBinding`, `endBinding` with focus and gap). An end whose line misses
/// its element is aimed at the element's centre. Returns false when the ends cannot be
/// put anywhere sensible (the two elements overlap so that no line leaves one for the
/// other); the arrow is then left where it was, bound all the same.
pub(crate) fn snap_arrow(
    arrow: &mut Element,
    start: Option<&Element>,
    end: Option<&Element>,
    snap: Snap,
) -> bool {
    let mut points = global_points(arrow);
    if points.len() < 2 {
        return false;
    }
    let last = points.len() - 1;
    let start_shape = start.map(Shape::of);
    let end_shape = end.map(Shape::of);
    let mut placed = true;
    if points.len() == 2 {
        let (mut a, mut z) = (points[0], points[1]);
        if let Some(s) = &start_shape
            && s.crossings(a, z).is_empty()
        {
            a = s.center();
        }
        if let Some(e) = &end_shape
            && e.crossings(a, z).is_empty()
        {
            z = e.center();
        }
        if let Some(s) = &start_shape
            && s.crossings(a, z).is_empty()
        {
            a = s.center();
        }
        let direction = (z.0 - a.0, z.1 - a.1);
        if direction.0 == 0.0 && direction.1 == 0.0 {
            placed = false;
        } else {
            let mut t_start = 0.0;
            let mut t_end = 1.0;
            if let Some(e) = &end_shape {
                // Where the line enters the end's element, coming from the start.
                let entries: Vec<f64> = e.crossings(a, z);
                let from = start_shape
                    .as_ref()
                    .map(|s| exit_parameter(s, a, z, f64::INFINITY))
                    .unwrap_or(0.0);
                match entries
                    .iter()
                    .copied()
                    .filter(|t| *t > from + 1e-9)
                    .reduce(f64::min)
                {
                    Some(t) => t_end = t,
                    None => placed = false,
                }
            }
            if let Some(s) = &start_shape {
                let exits: Vec<f64> = s.crossings(a, z);
                match exits
                    .iter()
                    .copied()
                    .filter(|t| *t < t_end - 1e-9)
                    .reduce(f64::max)
                {
                    Some(t) => t_start = t,
                    None => placed = false,
                }
            }
            if placed {
                points[0] = (a.0 + direction.0 * t_start, a.1 + direction.1 * t_start);
                points[1] = (a.0 + direction.0 * t_end, a.1 + direction.1 * t_end);
            }
        }
    } else {
        if let Some(s) = &start_shape {
            match end_on_outline(s, points[1], points[0]) {
                Some(p) => points[0] = p,
                None => placed = false,
            }
        }
        if let Some(e) = &end_shape {
            match end_on_outline(e, points[last - 1], points[last]) {
                Some(p) => points[last] = p,
                None => placed = false,
            }
        }
    }
    if !placed {
        points = global_points(arrow);
    }
    // The bindings, measured on the outline points (Excalidraw's focus and gap).
    if let (Some(element), Some(shape)) = (start, &start_shape) {
        let focus = geometry::focus(shape, points[1], points[0]);
        arrow.set("startBinding", binding(element.id(), focus));
    }
    if let (Some(element), Some(shape)) = (end, &end_shape) {
        let focus = geometry::focus(shape, points[last - 1], points[last]);
        arrow.set("endBinding", binding(element.id(), focus));
    }
    if placed {
        match snap {
            Snap::New => half_pixel_in(&mut points),
            Snap::Gap => {
                if start.is_some() {
                    points[0] = step_back(points[0], points[1], BINDING_GAP);
                }
                if end.is_some() {
                    points[last] = step_back(points[last], points[last - 1], BINDING_GAP);
                }
            }
        }
    }
    let (x0, y0) = points[0];
    let relative: Vec<Point> = points.iter().map(|p| (p.0 - x0, p.1 - y0)).collect();
    set_points(arrow, x0, y0, &relative);
    placed
}

/// The largest crossing before `limit`: where the line leaves the shape.
fn exit_parameter(shape: &Shape, a: Point, z: Point, limit: f64) -> f64 {
    shape
        .crossings(a, z)
        .into_iter()
        .filter(|t| *t < limit)
        .reduce(f64::max)
        .unwrap_or(0.0)
}

/// The end of a segment from `adjacent` towards `end`, put where it meets the shape's
/// outline first; aimed at the shape's centre when the segment's line misses it.
fn end_on_outline(shape: &Shape, adjacent: Point, end: Point) -> Option<Point> {
    let mut target = end;
    if shape.crossings(adjacent, target).is_empty() {
        target = shape.center();
    }
    let direction = (target.0 - adjacent.0, target.1 - adjacent.1);
    if direction.0 == 0.0 && direction.1 == 0.0 {
        return None;
    }
    let t = shape
        .crossings(adjacent, target)
        .into_iter()
        .filter(|t| *t > 1e-9)
        .reduce(f64::min)?;
    Some((adjacent.0 + direction.0 * t, adjacent.1 + direction.1 * t))
}

/// `p` moved `distance` towards `towards`.
fn step_back(p: Point, towards: Point, distance: f64) -> Point {
    let (dx, dy) = (towards.0 - p.0, towards.1 - p.1);
    let length = (dx * dx + dy * dy).sqrt();
    if length <= distance {
        return p;
    }
    (p.0 + dx / length * distance, p.1 + dy / length * distance)
}

/// The half pixel `bindLinearElementToElement` takes off each end of a new arrow, on each
/// axis the arrow runs along.
fn half_pixel_in(points: &mut [Point]) {
    let last = points.len() - 1;
    let delta = 0.5;
    let (end, before) = (points[last], points[last - 1]);
    if end.0 > before.0 {
        points[0].0 += delta;
        points[last].0 -= delta;
    }
    if end.0 < before.0 {
        points[0].0 -= delta;
        points[last].0 += delta;
    }
    if end.1 > before.1 {
        points[0].1 += delta;
        points[last].1 -= delta;
    }
    if end.1 < before.1 {
        points[0].1 -= delta;
        points[last].1 += delta;
    }
}

fn binding(element_id: &str, focus: f64) -> Value {
    let mut map = Map::new();
    map.insert("elementId".into(), Value::String(element_id.to_string()));
    map.insert("focus".into(), num(focus));
    map.insert("gap".into(), num(BINDING_GAP));
    Value::Object(map)
}

/// Adds `{id, type}` to the element's `boundElements` when it is not there.
pub(crate) fn add_bound(element: &mut Element, id: &str, kind: &str) {
    let list = element
        .json_mut()
        .entry("boundElements")
        .or_insert_with(|| Value::Array(Vec::new()));
    if !list.is_array() {
        *list = Value::Array(Vec::new());
    }
    if let Value::Array(items) = list
        && !items
            .iter()
            .any(|item| item.get("id").and_then(Value::as_str) == Some(id))
    {
        let mut entry = Map::new();
        entry.insert("id".into(), Value::String(id.to_string()));
        entry.insert("type".into(), Value::String(kind.to_string()));
        items.push(Value::Object(entry));
    }
}

/// Takes `id` out of the element's `boundElements`; whether it was there.
pub(crate) fn remove_bound(element: &mut Element, id: &str) -> bool {
    if let Some(Value::Array(items)) = element.json_mut().get_mut("boundElements") {
        let before = items.len();
        items.retain(|item| item.get("id").and_then(Value::as_str) != Some(id));
        return items.len() != before;
    }
    false
}

/// The id of a container's label (its bound text).
pub(crate) fn label_id(element: &Element) -> Option<String> {
    element
        .bound_elements()
        .into_iter()
        .find(|(_, kind)| *kind == "text")
        .map(|(id, _)| id.to_string())
}

/// A new version of a changed element: `version` + 1, a new `versionNonce`, `updated` now.
pub(crate) fn bump(element: &mut Element, making: &mut dyn Making) {
    let version = element.version().max(0) + 1;
    element.set("version", Value::from(version));
    element.set("versionNonce", Value::from(making.nonce()));
    element.set("updated", Value::from(making.now_ms()));
}
