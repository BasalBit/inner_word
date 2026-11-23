//! The scene: Excalidraw's drawing as the library keeps it (`docs/whiteboard-design.md`,
//! section 2, "The stored scene").

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};

use crate::json::{self, as_f64};
use crate::{FileHex, SOURCE, limits, well_formed_json};

/// The kinds of element that are never kept: web embeds and AI frames (Excalidraw renders
/// an `iframe` element's `customData` as a frame's `srcdoc`).
pub(crate) const NEVER_KEPT: [&str; 3] = ["iframe", "embeddable", "magicframe"];

/// The four fields of `appState` a stored scene keeps, what `serializeAsJSON(…,
/// "database")` keeps, with Excalidraw's defaults.
pub(crate) fn default_app_state() -> Map<String, Value> {
    let mut map = Map::new();
    map.insert("viewBackgroundColor".into(), Value::String("#ffffff".into()));
    map.insert("gridSize".into(), Value::from(20));
    map.insert("gridStep".into(), Value::from(5));
    map.insert("gridModeEnabled".into(), Value::Bool(false));
    map
}

/// A whiteboard's drawing: its live elements in z-order, the canvas settings that belong to
/// the drawing, its pictures by Excalidraw's file id.
#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    pub(crate) elements: Vec<Element>,
    pub(crate) app_state: Map<String, Value>,
    pub(crate) files: BTreeMap<String, FileEntry>,
}

/// One element of a scene, every field kept as Excalidraw wrote it (unknown ones too).
#[derive(Clone, Debug, PartialEq)]
pub struct Element(pub(crate) Map<String, Value>);

/// The type of an element; a type this crate does not know is kept as it came.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ElementType {
    Rectangle,
    Ellipse,
    Diamond,
    Text,
    Arrow,
    Line,
    Freedraw,
    Image,
    Frame,
    Other(String),
}

/// A picture of the scene: the type of its stored bytes, their `FileId`, when it came.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    pub mime_type: String,
    /// The stored bytes; `None` only in a scene read from somewhere that did not name them.
    pub file: Option<FileHex>,
    pub created: i64,
}

/// A scene that was read, with what was left out or mended on the way.
#[derive(Clone, Debug, PartialEq)]
pub struct Parsed {
    pub scene: Scene,
    pub warnings: Vec<String>,
}

/// Why a scene (or a shape library) could not be read.
#[derive(Debug, Clone, PartialEq)]
pub enum SceneError {
    /// The text is not JSON; the parser's message.
    NotJson(String),
    /// JSON, but not a scene: what is missing.
    NotAScene(String),
    /// Over one of the limits.
    TooLarge { what: &'static str, limit: usize },
}

impl std::fmt::Display for SceneError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SceneError::NotJson(message) => write!(f, "the drawing is not JSON ({message})"),
            SceneError::NotAScene(message) => write!(f, "the drawing is not a whiteboard's scene ({message})"),
            SceneError::TooLarge { what, limit } => {
                write!(f, "the {what} is larger than {}", crate::skeleton::limit_words(*limit))
            }
        }
    }
}

impl std::error::Error for SceneError {}

impl ElementType {
    /// The type named `text` (Excalidraw's `type`).
    pub fn parse(text: &str) -> ElementType {
        match text {
            "rectangle" => ElementType::Rectangle,
            "ellipse" => ElementType::Ellipse,
            "diamond" => ElementType::Diamond,
            "text" => ElementType::Text,
            "arrow" => ElementType::Arrow,
            "line" => ElementType::Line,
            "freedraw" => ElementType::Freedraw,
            "image" => ElementType::Image,
            "frame" => ElementType::Frame,
            other => ElementType::Other(other.to_string()),
        }
    }

    /// Excalidraw's name of the type.
    pub fn as_str(&self) -> &str {
        match self {
            ElementType::Rectangle => "rectangle",
            ElementType::Ellipse => "ellipse",
            ElementType::Diamond => "diamond",
            ElementType::Text => "text",
            ElementType::Arrow => "arrow",
            ElementType::Line => "line",
            ElementType::Freedraw => "freedraw",
            ElementType::Image => "image",
            ElementType::Frame => "frame",
            ElementType::Other(name) => name,
        }
    }

    /// A rectangle, an ellipse or a diamond: what holds a label and binds an arrow.
    pub fn is_shape(&self) -> bool {
        matches!(self, ElementType::Rectangle | ElementType::Ellipse | ElementType::Diamond)
    }

    /// An arrow or a line.
    pub fn is_linear(&self) -> bool {
        matches!(self, ElementType::Arrow | ElementType::Line)
    }
}

impl Element {
    /// An element from its JSON object (every field kept).
    pub fn from_json(map: Map<String, Value>) -> Element {
        Element(map)
    }

    pub fn id(&self) -> &str {
        self.0.get("id").and_then(Value::as_str).unwrap_or("")
    }

    pub fn kind(&self) -> ElementType {
        ElementType::parse(self.type_name())
    }

    /// Excalidraw's `type`, as written.
    pub fn type_name(&self) -> &str {
        self.0.get("type").and_then(Value::as_str).unwrap_or("")
    }

    pub fn x(&self) -> f64 {
        as_f64(self.0.get("x"))
    }

    pub fn y(&self) -> f64 {
        as_f64(self.0.get("y"))
    }

    pub fn width(&self) -> f64 {
        as_f64(self.0.get("width"))
    }

    pub fn height(&self) -> f64 {
        as_f64(self.0.get("height"))
    }

    pub fn angle(&self) -> f64 {
        as_f64(self.0.get("angle"))
    }

    /// Excalidraw's `version`, which grows with every change (0 when it has none).
    pub fn version(&self) -> i64 {
        match self.0.get("version") {
            Some(Value::Number(n)) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)).unwrap_or(0),
            _ => 0,
        }
    }

    /// Excalidraw's `versionNonce` (0 when it has none).
    pub fn version_nonce(&self) -> i64 {
        match self.0.get("versionNonce") {
            Some(Value::Number(n)) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)).unwrap_or(0),
            _ => 0,
        }
    }

    /// The text of a text element (a label's too).
    pub fn text(&self) -> Option<&str> {
        self.0.get("text").and_then(Value::as_str)
    }

    /// The element a label is bound to.
    pub fn container_id(&self) -> Option<&str> {
        self.0.get("containerId").and_then(Value::as_str)
    }

    /// The frame the element is in.
    pub fn frame_id(&self) -> Option<&str> {
        self.0.get("frameId").and_then(Value::as_str)
    }

    /// Excalidraw's file id of a picture.
    pub fn file_id(&self) -> Option<&str> {
        self.0.get("fileId").and_then(Value::as_str)
    }

    pub fn link(&self) -> Option<&str> {
        self.0.get("link").and_then(Value::as_str)
    }

    /// The name of a frame.
    pub fn name(&self) -> Option<&str> {
        self.0.get("name").and_then(Value::as_str)
    }

    pub fn locked(&self) -> bool {
        self.0.get("locked").and_then(Value::as_bool).unwrap_or(false)
    }

    pub fn is_deleted(&self) -> bool {
        self.0.get("isDeleted").and_then(Value::as_bool).unwrap_or(false)
    }

    /// Excalidraw's fractional `index` (the order it keeps elements in).
    pub fn index(&self) -> Option<&str> {
        self.0.get("index").and_then(Value::as_str)
    }

    /// The ids in `boundElements` (labels and arrows bound to this element).
    pub fn bound_elements(&self) -> Vec<(&str, &str)> {
        self.0
            .get("boundElements")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        let id = item.get("id")?.as_str()?;
                        let kind = item.get("type").and_then(Value::as_str).unwrap_or("");
                        Some((id, kind))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The id of the element an arrow's end is bound to (`"start"` or `"end"`).
    pub fn binding(&self, end: &str) -> Option<&str> {
        let key = if end == "start" { "startBinding" } else { "endBinding" };
        self.0.get(key)?.get("elementId")?.as_str()
    }

    /// The points of a line, an arrow or a freedraw, relative to `x`, `y`.
    pub fn points(&self) -> Vec<(f64, f64)> {
        self.0
            .get("points")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|p| {
                        let p = p.as_array()?;
                        Some((as_f64(p.first()), as_f64(p.get(1))))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn group_ids(&self) -> Vec<&str> {
        self.0
            .get("groupIds")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default()
    }

    pub fn json(&self) -> &Map<String, Value> {
        &self.0
    }

    pub(crate) fn json_mut(&mut self) -> &mut Map<String, Value> {
        &mut self.0
    }

    pub(crate) fn set(&mut self, key: &str, value: Value) {
        self.0.insert(key.to_string(), value);
    }

    pub(crate) fn set_f64(&mut self, key: &str, value: f64) {
        self.0.insert(key.to_string(), json::num(value));
    }

    pub(crate) fn str_field(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(Value::as_str)
    }

    pub(crate) fn f64_field(&self, key: &str) -> f64 {
        as_f64(self.0.get(key))
    }
}

/// Reads the elements of a scene's JSON: deleted ones and the kinds never kept left out,
/// anything that is not an element left out with a warning.
fn read_elements(items: Vec<Value>, warnings: &mut Vec<String>) -> Vec<Element> {
    let mut elements = Vec::with_capacity(items.len());
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for (n, item) in items.into_iter().enumerate() {
        let Value::Object(map) = item else {
            warnings.push(format!("Element {n} was left out: it is not an object."));
            continue;
        };
        let element = Element(map);
        if element.is_deleted() {
            continue;
        }
        let id = element.id().to_string();
        if id.is_empty() {
            warnings.push(format!("Element {n} was left out: it has no id."));
            continue;
        }
        let kind = element.type_name().to_string();
        if NEVER_KEPT.contains(&kind.as_str()) {
            warnings.push(format!(
                "The element {id} was left out: a whiteboard does not keep elements of type {kind} (web embeds)."
            ));
            continue;
        }
        if !seen.insert(id.clone()) {
            warnings.push(format!("A second element with the id {id} was left out."));
            continue;
        }
        elements.push(element);
    }
    elements
}

fn read_app_state(value: Option<&Value>) -> Map<String, Value> {
    let mut state = default_app_state();
    if let Some(Value::Object(given)) = value {
        for key in ["viewBackgroundColor", "gridSize", "gridStep", "gridModeEnabled"] {
            if let Some(v) = given.get(key)
                && !v.is_null()
            {
                state.insert(key.to_string(), v.clone());
            }
        }
    }
    state
}

fn parse_object(json: &str) -> Result<Map<String, Value>, SceneError> {
    let text = well_formed_json(json);
    let value: Value = serde_json::from_str(&text).map_err(|e| SceneError::NotJson(e.to_string()))?;
    match value {
        Value::Object(map) => Ok(map),
        _ => Err(SceneError::NotAScene("it is not an object".into())),
    }
}

fn take_elements(map: &mut Map<String, Value>) -> Result<Vec<Value>, SceneError> {
    match map.remove("elements") {
        Some(Value::Array(items)) => Ok(items),
        Some(_) => Err(SceneError::NotAScene("its elements are not a list".into())),
        None => Err(SceneError::NotAScene("it has no elements".into())),
    }
}

impl Scene {
    /// A scene without elements, with Excalidraw's default canvas.
    pub fn empty() -> Scene {
        Scene { elements: Vec::new(), app_state: default_app_state(), files: BTreeMap::new() }
    }

    /// The stored form (`documents.content` of a whiteboard, a snapshot's content). Read as
    /// leniently as the view's form; a `files` entry keeps its `file` when it is the id of
    /// stored bytes.
    pub fn from_stored(json: &str) -> Result<Parsed, SceneError> {
        let mut map = parse_object(json)?;
        let mut warnings = Vec::new();
        let items = take_elements(&mut map)?;
        let elements = read_elements(items, &mut warnings);
        let app_state = read_app_state(map.get("appState"));
        let mut files = BTreeMap::new();
        if let Some(Value::Object(entries)) = map.get("files") {
            for (id, entry) in entries {
                let Value::Object(entry) = entry else { continue };
                let file = entry.get("file").and_then(Value::as_str).and_then(FileHex::parse);
                files.insert(
                    id.clone(),
                    FileEntry {
                        mime_type: entry.get("mimeType").and_then(Value::as_str).unwrap_or("").to_string(),
                        file,
                        created: entry.get("created").and_then(Value::as_f64).map(|v| v as i64).unwrap_or(0),
                    },
                );
            }
        }
        Ok(Parsed { scene: Scene { elements, app_state, files }, warnings })
    }

    /// A scene as the view posts it (files without `file` and without `dataURL`). Lenient:
    /// unknown element types and fields are kept, a value of an unexpected type is kept as
    /// it came, lone surrogates are mended; it fails only on what is not JSON or has no
    /// `elements` array (and over [`limits::MAX_VIEW_SCENE_BYTES`]). `stored` maps
    /// Excalidraw's file id to the stored bytes and their type (the type written into
    /// `files`); a picture it does not know is left out of `files` (its element stays).
    pub fn from_view(json: &str, stored: &dyn Fn(&str) -> Option<(FileHex, String)>) -> Result<Parsed, SceneError> {
        if json.len() > limits::MAX_VIEW_SCENE_BYTES {
            return Err(SceneError::TooLarge { what: "drawing", limit: limits::MAX_VIEW_SCENE_BYTES });
        }
        let mut map = parse_object(json)?;
        let mut warnings = Vec::new();
        let items = take_elements(&mut map)?;
        let elements = read_elements(items, &mut warnings);
        let app_state = read_app_state(map.get("appState"));
        let posted = match map.get("files") {
            Some(Value::Object(entries)) => entries.clone(),
            _ => Map::new(),
        };
        let mut scene = Scene { elements, app_state, files: BTreeMap::new() };
        for id in scene.named_file_ids() {
            match stored(&id) {
                Some((file, mime_type)) => {
                    let created = posted
                        .get(&id)
                        .and_then(|entry| entry.get("created"))
                        .and_then(Value::as_f64)
                        .map(|v| v as i64)
                        .unwrap_or(0);
                    scene.files.insert(id, FileEntry { mime_type, file: Some(file), created });
                }
                None => warnings.push(format!("The picture {id} is not stored; its element shows a missing picture.")),
            }
        }
        Ok(Parsed { scene, warnings })
    }

    /// The Excalidraw file ids the scene's pictures name, distinct, in z-order.
    pub fn named_file_ids(&self) -> Vec<String> {
        let mut seen = BTreeSet::new();
        let mut ids = Vec::new();
        for element in &self.elements {
            if element.kind() == ElementType::Image
                && let Some(id) = element.file_id()
                && seen.insert(id.to_string())
            {
                ids.push(id.to_string());
            }
        }
        ids
    }

    /// The stored form: Excalidraw's envelope, minified, every object's keys sorted.
    pub fn to_stored(&self) -> String {
        let mut out = String::with_capacity(64 + self.elements.len() * 640);
        out.push_str("{\"appState\":");
        json::write_object(&mut out, &self.app_state);
        out.push_str(",\"elements\":[");
        for (n, element) in self.elements.iter().enumerate() {
            if n > 0 {
                out.push(',');
            }
            json::write_object(&mut out, &element.0);
        }
        out.push_str("],\"files\":{");
        for (n, (id, entry)) in self.files.iter().enumerate() {
            if n > 0 {
                out.push(',');
            }
            json::write_string(&mut out, id);
            out.push_str(":{\"created\":");
            out.push_str(&entry.created.to_string());
            if let Some(file) = &entry.file {
                out.push_str(",\"file\":");
                json::write_string(&mut out, file.as_str());
            }
            out.push_str(",\"mimeType\":");
            json::write_string(&mut out, &entry.mime_type);
            out.push('}');
        }
        out.push_str("},\"source\":");
        json::write_string(&mut out, SOURCE);
        out.push_str(",\"type\":\"excalidraw\",\"version\":2}");
        out
    }

    /// The live elements, in z-order.
    pub fn elements(&self) -> &[Element] {
        &self.elements
    }

    pub fn element(&self, id: &str) -> Option<&Element> {
        self.elements.iter().find(|e| e.id() == id)
    }

    pub(crate) fn element_mut(&mut self, id: &str) -> Option<&mut Element> {
        self.elements.iter_mut().find(|e| e.id() == id)
    }

    /// The pictures, by Excalidraw's file id.
    pub fn files(&self) -> &BTreeMap<String, FileEntry> {
        &self.files
    }

    /// The four canvas settings of the drawing (`viewBackgroundColor`, `gridSize`,
    /// `gridStep`, `gridModeEnabled`).
    pub fn app_state(&self) -> &Map<String, Value> {
        &self.app_state
    }

    /// The `FileId`s the scene names, distinct and sorted: what the store checks.
    pub fn stored_files(&self) -> Vec<FileHex> {
        let set: BTreeSet<FileHex> = self.files.values().filter_map(|entry| entry.file.clone()).collect();
        set.into_iter().collect()
    }

    /// No element: nothing to export.
    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    /// The scene with its picture entries set from the elements: an entry no picture names
    /// any more is left out.
    pub(crate) fn drop_unnamed_files(&mut self) {
        let named: BTreeSet<String> = self.named_file_ids().into_iter().collect();
        self.files.retain(|id, _| named.contains(id));
    }
}

impl Default for Scene {
    fn default() -> Scene {
        Scene::empty()
    }
}
