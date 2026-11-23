//! Excalidraw's shape library as the library's setting `board.shape_library` keeps it
//! (`docs/whiteboard-design.md`, section 4.15).

use serde_json::Value;

use crate::limits::{MAX_LIBRARY_BYTES, MAX_LIBRARY_ITEMS};
use crate::{Element, SOURCE, SceneError, json, well_formed_json};

/// The kinds of element no item of the library may hold: a picture (an item never names
/// stored bytes) and the kinds a whiteboard never keeps.
const NOT_IN_LIBRARY: [&str; 4] = ["image", "iframe", "embeddable", "magicframe"];

/// The shape library: one for every whiteboard of the library, newest item first as
/// Excalidraw keeps them.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapeLibrary {
    pub(crate) items: Vec<LibraryItem>,
}

/// A library that was read, with what was left out.
#[derive(Clone, Debug, PartialEq)]
pub struct ParsedLibrary {
    pub library: ShapeLibrary,
    pub warnings: Vec<String>,
}

/// One item of the shape library: elements kept together to be put on a canvas again.
#[derive(Clone, Debug, PartialEq)]
pub struct LibraryItem {
    pub(crate) id: String,
    pub(crate) status: String,
    pub(crate) name: Option<String>,
    pub(crate) created: i64,
    pub(crate) elements: Vec<Element>,
}

impl LibraryItem {
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The name the writer gave it.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// When it was made, in milliseconds since 1970.
    pub fn created(&self) -> i64 {
        self.created
    }

    /// Excalidraw's `status` (`unpublished`, `published`).
    pub fn status(&self) -> &str {
        &self.status
    }

    pub fn elements(&self) -> &[Element] {
        &self.elements
    }

    /// Its name, or "Shape 3: rectangle, arrow, text" without one (the picker's row): `index`
    /// is its place in the list from 0, the title counts from 1; the kinds of its elements
    /// in the order they first come, a label counted as text.
    pub fn title(&self, index: usize) -> String {
        if let Some(name) = self
            .name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
        {
            return name.to_string();
        }
        let mut kinds: Vec<&str> = Vec::new();
        for element in &self.elements {
            let kind = element.type_name();
            if !kind.is_empty() && !kinds.contains(&kind) {
                kinds.push(kind);
            }
        }
        if kinds.is_empty() {
            format!("Shape {}", index + 1)
        } else {
            format!("Shape {}: {}", index + 1, kinds.join(", "))
        }
    }

    fn write(&self, out: &mut String) {
        out.push_str("{\"created\":");
        out.push_str(&self.created.to_string());
        out.push_str(",\"elements\":[");
        for (n, element) in self.elements.iter().enumerate() {
            if n > 0 {
                out.push(',');
            }
            json::write_object(out, element.json());
        }
        out.push_str("],\"id\":");
        json::write_string(out, &self.id);
        if let Some(name) = &self.name {
            out.push_str(",\"name\":");
            json::write_string(out, name);
        }
        out.push_str(",\"status\":");
        json::write_string(out, &self.status);
        out.push('}');
    }
}

impl ShapeLibrary {
    /// A library without items (the setting absent).
    pub fn empty() -> ShapeLibrary {
        ShapeLibrary { items: Vec::new() }
    }

    /// The stored form (`{"type":"excalidrawlib","version":2,…,"libraryItems":[…]}`). An
    /// item with a picture, an `iframe`, an `embeddable` or a `magicframe` is left out, with
    /// a warning.
    pub fn from_stored(json: &str) -> Result<ParsedLibrary, SceneError> {
        let text = well_formed_json(json);
        let value: Value =
            serde_json::from_str(&text).map_err(|e| SceneError::NotJson(e.to_string()))?;
        let Value::Object(map) = value else {
            return Err(SceneError::NotAScene(
                "the shape library is not an object".into(),
            ));
        };
        let items = match map.get("libraryItems") {
            Some(Value::Array(items)) => items,
            Some(_) => {
                return Err(SceneError::NotAScene(
                    "its libraryItems are not a list".into(),
                ));
            }
            None => return Err(SceneError::NotAScene("it has no libraryItems".into())),
        };
        Ok(read_items(items))
    }

    /// What the glue posts (Excalidraw's `LibraryItems`, a list); lenient as
    /// [`crate::Scene::from_view`]. `TooLarge` over [`MAX_LIBRARY_BYTES`] (what was posted or
    /// what would be stored) or [`MAX_LIBRARY_ITEMS`].
    pub fn from_view(items_json: &str) -> Result<ParsedLibrary, SceneError> {
        if items_json.len() > MAX_LIBRARY_BYTES {
            return Err(SceneError::TooLarge {
                what: "shape library",
                limit: MAX_LIBRARY_BYTES,
            });
        }
        let text = well_formed_json(items_json);
        let value: Value =
            serde_json::from_str(&text).map_err(|e| SceneError::NotJson(e.to_string()))?;
        let items = match &value {
            Value::Array(items) => items,
            Value::Object(map) => match map.get("libraryItems").or_else(|| map.get("items")) {
                Some(Value::Array(items)) => items,
                _ => {
                    return Err(SceneError::NotAScene(
                        "the shape library has no items".into(),
                    ));
                }
            },
            _ => {
                return Err(SceneError::NotAScene(
                    "the shape library is not a list".into(),
                ));
            }
        };
        if items.len() > MAX_LIBRARY_ITEMS {
            return Err(SceneError::TooLarge {
                what: "shape library",
                limit: MAX_LIBRARY_ITEMS,
            });
        }
        let parsed = read_items(items);
        if parsed.library.to_stored().len() > MAX_LIBRARY_BYTES {
            return Err(SceneError::TooLarge {
                what: "shape library",
                limit: MAX_LIBRARY_BYTES,
            });
        }
        Ok(parsed)
    }

    /// The stored form, minified, every object's keys sorted.
    pub fn to_stored(&self) -> String {
        let mut out = String::from("{\"libraryItems\":[");
        for (n, item) in self.items.iter().enumerate() {
            if n > 0 {
                out.push(',');
            }
            item.write(&mut out);
        }
        out.push_str("],\"source\":");
        json::write_string(&mut out, SOURCE);
        out.push_str(",\"type\":\"excalidrawlib\",\"version\":2}");
        out
    }

    pub fn items(&self) -> &[LibraryItem] {
        &self.items
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

impl Default for ShapeLibrary {
    fn default() -> ShapeLibrary {
        ShapeLibrary::empty()
    }
}

fn read_items(items: &[Value]) -> ParsedLibrary {
    let mut warnings = Vec::new();
    let mut out: Vec<LibraryItem> = Vec::new();
    for (n, item) in items.iter().enumerate() {
        let Value::Object(item) = item else {
            warnings.push(format!(
                "Item {n} of the shape library was left out: it is not an object."
            ));
            continue;
        };
        let Some(id) = item
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
        else {
            warnings.push(format!(
                "Item {n} of the shape library was left out: it has no id."
            ));
            continue;
        };
        if out.iter().any(|i| i.id == id) {
            warnings.push(format!(
                "A second item {id} of the shape library was left out."
            ));
            continue;
        }
        let Some(Value::Array(elements)) = item.get("elements") else {
            warnings.push(format!(
                "The item {id} of the shape library was left out: it has no elements."
            ));
            continue;
        };
        let elements: Vec<Element> = elements
            .iter()
            .filter_map(|e| match e {
                Value::Object(map) => Some(Element(map.clone())),
                _ => None,
            })
            .filter(|e| !e.is_deleted())
            .collect();
        if let Some(kind) = elements
            .iter()
            .map(Element::type_name)
            .find(|k| NOT_IN_LIBRARY.contains(k))
        {
            warnings.push(format!(
                "The item {id} of the shape library was left out: it holds an element of type {kind}, which the shape library does not keep."
            ));
            continue;
        }
        if elements.is_empty() {
            warnings.push(format!(
                "The item {id} of the shape library was left out: it has no elements."
            ));
            continue;
        }
        out.push(LibraryItem {
            id: id.to_string(),
            status: item
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("unpublished")
                .to_string(),
            name: item.get("name").and_then(Value::as_str).map(str::to_string),
            created: item
                .get("created")
                .and_then(Value::as_f64)
                .map(|c| c as i64)
                .unwrap_or(0),
            elements,
        });
    }
    ParsedLibrary {
        library: ShapeLibrary { items: out },
        warnings,
    }
}
