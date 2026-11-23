//! What is read of a scene: its text, summary, listing and bounds (`docs/whiteboard-design.md`,
//! sections 2 and 7.4).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The header of a listing: how the whiteboard is named.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ListingHeader<'a> {
    /// `board:12`.
    pub reference: &'a str,
    pub title: &'a str,
    /// The folder's path, `None` at the root.
    pub folder: Option<&'a str>,
}

/// What a whiteboard holds, counted.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BoardSummary {
    /// Elements, labels included.
    pub elements: u64,
    /// Elements by Excalidraw's type.
    pub by_type: BTreeMap<String, u64>,
    /// Text elements (free text and labels).
    pub texts: u64,
    /// Picture elements.
    pub pictures: u64,
    /// Words of [`crate::Scene::text`].
    pub words: u64,
    /// The drawn area, `None` for an empty whiteboard.
    pub bounds: Option<Bounds>,
}

/// A rectangle in scene coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// The style of a listed element: the fields that differ from Excalidraw's defaults are
/// what the listing writes; the JSON form has them all.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListedStyle {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stroke_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill_style: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stroke_width: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stroke_style: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roughness: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_family: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_align: Option<String>,
}

/// One element as agents read it (the JSON form of the listing).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ListedElement {
    pub id: String,
    #[serde(rename = "type")]
    pub r#type: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub angle: f64,
    /// The text of a text element (a label's too).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// The text of the label bound to a shape or an arrow.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// For a label: the element it is bound to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container: Option<String>,
    /// For an arrow or a line: the element its start is bound to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<String>,
    /// For a line, an arrow or a freedraw: its points, relative to `x`, `y`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub points: Option<Vec<[f64; 2]>>,
    /// The frame it is in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub groups: Vec<String>,
    pub style: ListedStyle,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    pub locked: bool,
    /// For a picture: `stored` or `missing` (no entry in the scene's files).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub picture: Option<String>,
}

use crate::geometry::element_bounds;
use crate::metrics;
use crate::skeleton::make::{global_points, label_id, original_text};
use crate::{Element, ElementType, FileHex, Scene};

impl Scene {
    /// The text of the whiteboard (section 2): the text of every text element (free text,
    /// the labels of shapes and arrows) and the name of every frame, in reading order (by
    /// the top edge of the element, a label by its container's, then by the left edge), one
    /// per line, empty texts left out. What the library indexes and counts.
    pub fn text(&self) -> String {
        let mut entries: Vec<(f64, f64, usize, String)> = Vec::new();
        for (n, element) in self.elements.iter().enumerate() {
            let text = match element.kind() {
                ElementType::Text => original_text(element),
                ElementType::Frame => element.name().unwrap_or("").to_string(),
                _ => continue,
            };
            if text.trim().is_empty() {
                continue;
            }
            let anchor = element.container_id().and_then(|c| self.element(c)).unwrap_or(element);
            let (top, left) = match element_bounds(anchor) {
                Some(b) => (b[1], b[0]),
                None => (anchor.y(), anchor.x()),
            };
            entries.push((top, left, n, text));
        }
        entries.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)).then(a.2.cmp(&b.2)));
        entries.into_iter().map(|(_, _, _, text)| text).collect::<Vec<_>>().join("\n")
    }

    /// What the whiteboard holds, counted.
    pub fn summary(&self) -> BoardSummary {
        let mut by_type = BTreeMap::new();
        let mut texts = 0;
        let mut pictures = 0;
        for element in &self.elements {
            *by_type.entry(element.type_name().to_string()).or_insert(0) += 1;
            match element.kind() {
                ElementType::Text => texts += 1,
                ElementType::Image => pictures += 1,
                _ => {}
            }
        }
        BoardSummary {
            elements: self.elements.len() as u64,
            by_type,
            texts,
            pictures,
            words: count_words(&self.text()),
            bounds: self.bounds(),
        }
    }

    /// The drawn area: the bounds of every element, rotation included (`None`: empty).
    pub fn bounds(&self) -> Option<Bounds> {
        let mut acc: Option<[f64; 4]> = None;
        for b in self.elements.iter().filter_map(element_bounds) {
            acc = Some(match acc {
                None => b,
                Some(a) => [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])],
            });
        }
        acc.map(|b| Bounds { x: b[0], y: b[1], width: b[2] - b[0], height: b[3] - b[1] })
    }

    /// The size of the whiteboard's PDF page as Rust estimates it (section 5): the drawn
    /// area and `padding` on each side, the whole scaled down when its longer side would
    /// pass `max_side`, each side rounded up as WebKit's page is. `None` for an empty
    /// whiteboard. Text sizes are Rust's estimates, so a page whose extremes are texts can
    /// differ from WebKit's by a few points.
    pub fn pdf_size(&self, padding: f64, max_side: f64) -> Option<(f64, f64)> {
        let bounds = self.bounds()?;
        let (mut width, mut height) = (bounds.width + 2.0 * padding, bounds.height + 2.0 * padding);
        let longest = width.max(height);
        if longest > max_side && longest > 0.0 {
            let scale = max_side / longest;
            width *= scale;
            height *= scale;
        }
        Some((width.ceil(), height.ceil()))
    }

    /// The listing of section 7.4: a header, then one element per line in z-order (ids
    /// first; a label with its container, not on a line of its own). A picture whose entry
    /// names stored bytes is "stored"; [`Scene::listing_with`] asks the library.
    pub fn listing(&self, header: &ListingHeader<'_>) -> String {
        self.listing_with(header, &|_| true)
    }

    /// The listing, with `has` saying whether the library has a picture's bytes.
    pub fn listing_with(&self, header: &ListingHeader<'_>, has: &dyn Fn(&FileHex) -> bool) -> String {
        let title = if header.title.trim().is_empty() { "Untitled" } else { header.title };
        let mut out = format!("{} “{}”", header.reference, title);
        if let Some(folder) = header.folder.filter(|f| !f.is_empty()) {
            out.push_str(" in ");
            out.push_str(folder);
        }
        let listed: Vec<&Element> = self.elements.iter().filter(|e| !self.is_label(e)).collect();
        if listed.is_empty() {
            out.push_str(": empty");
            return out;
        }
        let pictures = self.elements.iter().filter(|e| e.kind() == ElementType::Image).count();
        out.push_str(&format!(
            ": {}, {}",
            plural(listed.len(), "element", "elements"),
            plural(pictures, "picture", "pictures")
        ));
        if let Some(b) = self.bounds() {
            out.push_str(&format!(
                "; drawn area {} x {} at ({}, {})",
                whole(b.width),
                whole(b.height),
                whole(b.x),
                whole(b.y)
            ));
        }
        let rows: Vec<Vec<String>> = listed.iter().map(|e| self.row(e, has)).collect();
        let columns = 4;
        let mut widths = [0usize; 4];
        for row in &rows {
            for (n, cell) in row.iter().take(columns).enumerate() {
                widths[n] = widths[n].max(cell.chars().count());
            }
        }
        for row in rows {
            let mut line = String::new();
            for (n, cell) in row.iter().enumerate() {
                if n > 0 {
                    line.push_str("  ");
                }
                line.push_str(cell);
                if n < columns && n + 1 < row.len() {
                    let pad = widths[n] - cell.chars().count();
                    line.extend(std::iter::repeat_n(' ', pad));
                }
            }
            out.push('\n');
            out.push_str(line.trim_end());
        }
        out
    }

    /// A label: a text bound to a container that is on the whiteboard.
    fn is_label(&self, element: &Element) -> bool {
        element.kind() == ElementType::Text && element.container_id().is_some_and(|c| self.element(c).is_some())
    }

    fn label_text(&self, element: &Element) -> Option<String> {
        let id = label_id(element)?;
        let label = self.element(&id)?;
        Some(original_text(label))
    }

    fn picture_state(&self, element: &Element, has: &dyn Fn(&FileHex) -> bool) -> (bool, Option<String>) {
        let entry = element.file_id().and_then(|id| self.files.get(id));
        match entry {
            Some(entry) => {
                let stored = entry.file.as_ref().is_some_and(has);
                (stored, Some(entry.mime_type.clone()))
            }
            None => (false, None),
        }
    }

    fn row(&self, element: &Element, has: &dyn Fn(&FileHex) -> bool) -> Vec<String> {
        let kind = element.kind();
        let mut row = vec![element.type_name().to_string(), element.id().to_string()];
        let at = format!("at ({}, {})", whole(element.x()), whole(element.y()));
        let size = format!("{} x {}", whole(element.width()), whole(element.height()));
        let mut rest: Vec<String> = Vec::new();
        match kind {
            ElementType::Arrow | ElementType::Line => {
                let place = match (element.binding("start"), element.binding("end")) {
                    (Some(a), Some(b)) => format!("from {a} to {b}"),
                    (Some(a), None) => format!("from {a}"),
                    (None, Some(b)) => format!("to {b}"),
                    (None, None) => at,
                };
                row.push(place);
                let points = global_points(element);
                let ends = match (points.first(), points.last()) {
                    (Some(a), Some(b)) => format!("({}, {}) -> ({}, {})", whole(a.0), whole(a.1), whole(b.0), whole(b.1)),
                    _ => size,
                };
                row.push(ends);
                if points.len() > 2 {
                    rest.push(format!("{} points", points.len()));
                }
                push_stroke(element, &mut rest);
            }
            ElementType::Text => {
                row.push(at);
                row.push(size);
                rest.push(quote(&original_text(element)));
                let style = crate::skeleton::make::text_style_of(element);
                rest.push(format!("{} px {}", whole(style.font_size), metrics::family_name(style.font_family)));
                if style.text_align != "left" {
                    rest.push(format!("align {}", style.text_align));
                }
                push_stroke(element, &mut rest);
            }
            ElementType::Image => {
                row.push(at);
                row.push(size);
                let (stored, mime) = self.picture_state(element, has);
                rest.push(match (stored, mime) {
                    (true, Some(mime)) => format!("picture stored ({})", short_type(&mime)),
                    _ => "picture missing".to_string(),
                });
            }
            ElementType::Frame => {
                row.push(at);
                row.push(size);
                if let Some(name) = element.name().filter(|n| !n.trim().is_empty()) {
                    rest.push(quote(name));
                }
                let children: Vec<&str> = self
                    .elements
                    .iter()
                    .filter(|e| e.frame_id() == Some(element.id()) && !self.is_label(e))
                    .map(Element::id)
                    .collect();
                if !children.is_empty() {
                    rest.push(format!("holds {}", children.join(", ")));
                }
            }
            ElementType::Freedraw => {
                row.push(at);
                row.push(size);
                rest.push(format!("{} points", element.points().len()));
                push_stroke(element, &mut rest);
            }
            _ => {
                row.push(at);
                row.push(size);
                push_fill(element, &mut rest);
                push_stroke(element, &mut rest);
            }
        }
        if let Some(label) = self.label_text(element) {
            rest.push(format!("label {}", quote(&label)));
        }
        let degrees = element.angle().to_degrees();
        if degrees.round() != 0.0 && degrees.round() != 360.0 {
            rest.push(format!("rotated {}°", whole(degrees)));
        }
        let opacity = element.f64_field("opacity");
        if element.json().contains_key("opacity") && opacity != 100.0 {
            rest.push(format!("opacity {}", whole(opacity)));
        }
        let groups = element.group_ids();
        if !groups.is_empty() {
            rest.push(format!("groups {}", groups.join(", ")));
        }
        if let Some(link) = element.link().filter(|l| !l.is_empty()) {
            rest.push(format!("link {link}"));
        }
        if element.locked() {
            rest.push("locked".to_string());
        }
        row.extend(rest);
        row
    }

    /// The elements as agents read them (the JSON form of the listing), labels included, in
    /// z-order.
    pub fn listed(&self) -> Vec<ListedElement> {
        self.listed_with(&|_| true)
    }

    /// The elements as agents read them, with `has` saying whether the library has a
    /// picture's bytes.
    pub fn listed_with(&self, has: &dyn Fn(&FileHex) -> bool) -> Vec<ListedElement> {
        self.elements
            .iter()
            .map(|element| {
                let kind = element.kind();
                let text_like = kind == ElementType::Text;
                let style = ListedStyle {
                    stroke_color: element.str_field("strokeColor").map(str::to_string),
                    background_color: element.str_field("backgroundColor").map(str::to_string),
                    fill_style: element.str_field("fillStyle").map(str::to_string),
                    stroke_width: element.json().get("strokeWidth").and_then(Value::as_f64),
                    stroke_style: element.str_field("strokeStyle").map(str::to_string),
                    roughness: element.json().get("roughness").and_then(Value::as_f64),
                    opacity: element.json().get("opacity").and_then(Value::as_f64),
                    font_size: if text_like { element.json().get("fontSize").and_then(Value::as_f64) } else { None },
                    font_family: if text_like {
                        element.json().get("fontFamily").and_then(Value::as_u64).map(|f| metrics::family_name(f as u8))
                    } else {
                        None
                    },
                    text_align: if text_like { element.str_field("textAlign").map(str::to_string) } else { None },
                };
                let points = if kind.is_linear() || kind == ElementType::Freedraw {
                    Some(element.points().into_iter().map(|p| [p.0, p.1]).collect())
                } else {
                    None
                };
                let picture = if kind == ElementType::Image {
                    let (stored, _) = self.picture_state(element, has);
                    Some(if stored { "stored" } else { "missing" }.to_string())
                } else {
                    None
                };
                ListedElement {
                    id: element.id().to_string(),
                    r#type: element.type_name().to_string(),
                    x: element.x(),
                    y: element.y(),
                    width: element.width(),
                    height: element.height(),
                    angle: element.angle(),
                    text: if text_like { Some(original_text(element)) } else { None },
                    label: self.label_text(element),
                    container: element.container_id().map(str::to_string),
                    start: element.binding("start").map(str::to_string),
                    end: element.binding("end").map(str::to_string),
                    points,
                    frame: element.frame_id().map(str::to_string),
                    groups: element.group_ids().into_iter().map(str::to_string).collect(),
                    style,
                    link: element.link().filter(|l| !l.is_empty()).map(str::to_string),
                    locked: element.locked(),
                    picture,
                }
            })
            .collect()
    }
}

use serde_json::Value;

fn push_fill(element: &Element, rest: &mut Vec<String>) {
    let background = element.str_field("backgroundColor").unwrap_or("transparent");
    if background != "transparent" {
        let fill = element.str_field("fillStyle").unwrap_or("solid");
        rest.push(format!("fill {background} {fill}"));
    }
}

fn push_stroke(element: &Element, rest: &mut Vec<String>) {
    let stroke = element.str_field("strokeColor").unwrap_or("#1e1e1e");
    if stroke != "#1e1e1e" && stroke != "transparent" {
        rest.push(format!("stroke {stroke}"));
    }
    let style = element.str_field("strokeStyle").unwrap_or("solid");
    if style != "solid" {
        rest.push(style.to_string());
    }
}

/// A number of the listing: whole, -0 written 0.
fn whole(value: f64) -> String {
    let rounded = value.round();
    if rounded == 0.0 { "0".to_string() } else { format!("{}", rounded as i64) }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// A text quoted as JSON writes a string (a line break as `\n`).
fn quote(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| format!("\"{text}\""))
}

fn short_type(mime: &str) -> String {
    match mime {
        "image/png" => "png".into(),
        "image/jpeg" => "jpeg".into(),
        "image/gif" => "gif".into(),
        "image/svg+xml" => "svg".into(),
        "image/webp" => "webp".into(),
        other => other.to_string(),
    }
}

/// Words as the listing counts them: runs between white space that hold a letter or a
/// digit. (The library counts a whiteboard's text by the rule of a text.)
pub(crate) fn count_words(text: &str) -> u64 {
    text.split_whitespace().filter(|w| w.chars().any(char::is_alphanumeric)).count() as u64
}
