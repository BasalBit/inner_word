//! What the assistant proposes: a drawing from an answer, in the agents' own formats
//! (`docs/assistant-draws-design.md`, sections 2 and 5; `docs/whiteboard-design.md`, 7.2
//! and 7.3), read and checked here ([`Drawing::from_json`]) before any whiteboard is
//! looked at.

use serde_json::{Map, Value};

use crate::limits::{MAX_ID_CHARS, MAX_REQUEST_ELEMENTS};
use crate::skeleton::{ElementChange, Skeleton, is_id, limit_words};

/// The most characters of a new whiteboard's title taken from a drawing.
pub const MAX_TITLE_CHARS: usize = 120;

/// The types of element a drawing may add: the ones the converter makes, without a
/// picture, in the order the summary counts them.
pub const DRAWABLE_TYPES: [&str; 8] = [
    "rectangle",
    "ellipse",
    "diamond",
    "text",
    "arrow",
    "line",
    "freedraw",
    "frame",
];

/// A drawing from an answer of the assistant: what it adds, changes and removes, in the
/// agents' own formats (`docs/whiteboard-design.md`, 7.2 and 7.3). Read with
/// [`Drawing::from_json`], applied with [`draw`](crate::draw).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Drawing {
    /// The title of a new whiteboard: one line, at most 120 characters. `None`: not given.
    pub title: Option<String>,
    pub add: Vec<Skeleton>,
    pub update: Vec<ElementChange>,
    pub remove: Vec<String>,
}

/// Why the JSON of a drawing cannot be used (no whiteboard was looked at).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DrawingError {
    /// Not JSON: where, within the text given, and serde_json's words.
    NotJson {
        line: usize,
        column: usize,
        message: String,
    },
    /// JSON, but not an object of `add`, `update`, `remove` and `title`, nor an array of
    /// skeletons; or an `add` entry without a type the converter makes: the whole clause.
    NotADrawing { message: String },
    /// An id, wherever the drawing names one, that is not 1 to `MAX_ID_CHARS` of
    /// `[A-Za-z0-9_-]`: the id as written (a value that is no string as its JSON).
    BadId { id: String },
    /// Nothing to add, change or remove.
    Empty,
    /// An `image` skeleton: a drawing from an answer brings no picture.
    Picture { element: usize },
    /// A list over `limits::MAX_REQUEST_ELEMENTS`: `list` is "add", "update" or "remove".
    TooLarge { list: &'static str, limit: usize },
}

impl std::fmt::Display for DrawingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DrawingError::NotJson {
                line,
                column,
                message,
            } => write!(
                f,
                "it is not JSON (line {line}, column {column}: {message})"
            ),
            DrawingError::NotADrawing { message } => f.write_str(message),
            DrawingError::BadId { id } => write!(
                f,
                "it names an element “{}”, and an id is 1 to {MAX_ID_CHARS} letters, digits, - and _",
                cut(id, 40)
            ),
            DrawingError::Empty => f.write_str("it adds, changes and removes nothing"),
            DrawingError::Picture { element } => write!(
                f,
                "the element at index {element} of add is a picture, and a drawing from an answer cannot bring one"
            ),
            DrawingError::TooLarge { list, limit } => {
                let verb = match *list {
                    "add" => "adds",
                    "update" => "changes",
                    _ => "removes",
                };
                write!(f, "it {verb} more than {} elements", limit_words(*limit))
            }
        }
    }
}

impl std::error::Error for DrawingError {}

/// `text` cut at `max` characters, with "…" when it was longer.
fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let mut out: String = text.chars().take(max).collect();
        out.push('…');
        out
    }
}

fn not_a_drawing(message: impl Into<String>) -> DrawingError {
    DrawingError::NotADrawing {
        message: message.into(),
    }
}

/// serde_json's words without the place it appends ("… at line 3 column 14").
fn serde_words(error: &serde_json::Error) -> String {
    let text = error.to_string();
    let suffix = format!(" at line {} column {}", error.line(), error.column());
    text.strip_suffix(&suffix).unwrap_or(&text).to_string()
}

/// An id as written: the string, or the JSON of a value that is no string.
fn id_as_written(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// Checks one id the drawing names.
fn check_id(value: &Value) -> Result<(), DrawingError> {
    match value {
        Value::String(text) if is_id(text) => Ok(()),
        other => Err(DrawingError::BadId {
            id: id_as_written(other),
        }),
    }
}

/// Checks the ids an element or a change names besides its own: the `id` of `start` and
/// `end`, and the entries of `children`. Other shapes of those fields are the converter's
/// to refuse, with its words.
fn check_references(map: &Map<String, Value>) -> Result<(), DrawingError> {
    for field in ["start", "end"] {
        if let Some(Value::Object(end)) = map.get(field)
            && let Some(id) = end.get("id")
        {
            check_id(id)?;
        }
    }
    if let Some(Value::Array(children)) = map.get("children") {
        for child in children {
            check_id(child)?;
        }
    }
    Ok(())
}

/// A title as a new whiteboard takes it: trimmed, runs of white space made one space, cut
/// at [`MAX_TITLE_CHARS`]; `None` when nothing is left.
fn clean_title(text: &str) -> Option<String> {
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let cut: String = joined.chars().take(MAX_TITLE_CHARS).collect();
    let cut = cut.trim_end().to_string();
    (!cut.is_empty()).then_some(cut)
}

impl Drawing {
    /// Reads the body of a drawing block (section 2.1): one object with any of `add`,
    /// `update`, `remove` and `title`, or a bare array read as `{"add": [...]}`. Every
    /// type of `add` and every id the drawing names is checked here; fields, values and
    /// ids a whiteboard does not have are judged when it is drawn.
    pub fn from_json(text: &str) -> Result<Drawing, DrawingError> {
        let value: Value = serde_json::from_str(text).map_err(|error| DrawingError::NotJson {
            line: error.line(),
            column: error.column(),
            message: serde_words(&error),
        })?;
        let mut object = match value {
            Value::Array(add) => {
                let mut map = Map::new();
                map.insert("add".into(), Value::Array(add));
                map
            }
            Value::Object(map) => map,
            _ => {
                return Err(not_a_drawing(
                    "it is not a drawing: it must be an object with add, update and remove, or a list of elements",
                ));
            }
        };
        let mut keys: Vec<&String> = object.keys().collect();
        keys.sort();
        if let Some(key) = keys
            .into_iter()
            .find(|key| !["add", "update", "remove", "title"].contains(&key.as_str()))
        {
            return Err(not_a_drawing(format!(
                "it is not a drawing: “{}” is not one of its parts (add, update, remove and title are)",
                cut(key, 40)
            )));
        }
        let list = |object: &mut Map<String, Value>,
                    key: &'static str,
                    what: &str|
         -> Result<Vec<Value>, DrawingError> {
            match object.remove(key) {
                None => Ok(Vec::new()),
                Some(Value::Array(items)) => {
                    if items.len() > MAX_REQUEST_ELEMENTS {
                        return Err(DrawingError::TooLarge {
                            list: key,
                            limit: MAX_REQUEST_ELEMENTS,
                        });
                    }
                    Ok(items)
                }
                Some(_) => Err(not_a_drawing(format!(
                    "it is not a drawing: {key} must be a list of {what}"
                ))),
            }
        };
        let add = list(&mut object, "add", "elements")?;
        let update = list(&mut object, "update", "changes")?;
        let remove = list(&mut object, "remove", "ids")?;
        let title = match object.remove("title") {
            None | Some(Value::Null) => None,
            Some(Value::String(title)) => clean_title(&title),
            Some(_) => return Err(not_a_drawing("it is not a drawing: title must be a string")),
        };

        let mut skeletons = Vec::with_capacity(add.len());
        for (n, item) in add.into_iter().enumerate() {
            let Value::Object(map) = item else {
                return Err(not_a_drawing(format!(
                    "it is not a drawing: the entry at index {n} of add is not an element (an object)"
                )));
            };
            let kind = map.get("type").and_then(Value::as_str);
            if kind == Some("image") {
                return Err(DrawingError::Picture { element: n });
            }
            if !kind.is_some_and(|kind| DRAWABLE_TYPES.contains(&kind)) {
                return Err(not_a_drawing(format!(
                    "the element at index {n} of add has no type it can draw (rectangle, ellipse, diamond, text, arrow, line, freedraw and frame are)"
                )));
            }
            if let Some(id) = map.get("id") {
                check_id(id)?;
            }
            check_references(&map)?;
            skeletons.push(Skeleton(map));
        }
        let mut changes = Vec::with_capacity(update.len());
        for (n, item) in update.into_iter().enumerate() {
            let Value::Object(map) = item else {
                return Err(not_a_drawing(format!(
                    "it is not a drawing: the entry at index {n} of update is not a change (an object)"
                )));
            };
            match map.get("id") {
                Some(id) => check_id(id)?,
                None => {
                    return Err(not_a_drawing(format!(
                        "the change at index {n} of update names no element (its \"id\")"
                    )));
                }
            }
            check_references(&map)?;
            changes.push(ElementChange(map));
        }
        let mut ids = Vec::with_capacity(remove.len());
        for item in remove {
            match item {
                Value::String(id) => {
                    if !is_id(&id) {
                        return Err(DrawingError::BadId { id });
                    }
                    ids.push(id);
                }
                _ => {
                    return Err(not_a_drawing(
                        "it is not a drawing: remove must be a list of ids",
                    ));
                }
            }
        }
        let drawing = Drawing {
            title,
            add: skeletons,
            update: changes,
            remove: ids,
        };
        if drawing.is_empty() {
            return Err(DrawingError::Empty);
        }
        Ok(drawing)
    }

    /// The drawings of several blocks as one, in order (section 2.2): the adds of all, then
    /// the changes, then the removals, and the first title given. `TooLarge` when a joined
    /// list is over the limit; `Empty` for nothing.
    pub fn join(drawings: Vec<Drawing>) -> Result<Drawing, DrawingError> {
        let mut out = Drawing::default();
        for drawing in drawings {
            if out.title.is_none() {
                out.title = drawing.title;
            }
            out.add.extend(drawing.add);
            out.update.extend(drawing.update);
            out.remove.extend(drawing.remove);
        }
        for (list, len) in [
            ("add", out.add.len()),
            ("update", out.update.len()),
            ("remove", out.remove.len()),
        ] {
            if len > MAX_REQUEST_ELEMENTS {
                return Err(DrawingError::TooLarge {
                    list,
                    limit: MAX_REQUEST_ELEMENTS,
                });
            }
        }
        if out.is_empty() {
            return Err(DrawingError::Empty);
        }
        Ok(out)
    }

    /// Whether it adds, changes and removes nothing.
    pub fn is_empty(&self) -> bool {
        self.add.is_empty() && self.update.is_empty() && self.remove.is_empty()
    }

    /// Whether it changes or removes anything: it needs a whiteboard that has those
    /// elements.
    pub fn has_changes(&self) -> bool {
        !self.update.is_empty() || !self.remove.is_empty()
    }

    /// What it adds by type, in the order of [`DRAWABLE_TYPES`], each with its count; a
    /// label is not counted apart.
    pub fn added_types(&self) -> Vec<(&'static str, usize)> {
        DRAWABLE_TYPES
            .iter()
            .filter_map(|kind| {
                let count = self
                    .add
                    .iter()
                    .filter(|s| s.0.get("type").and_then(Value::as_str) == Some(kind))
                    .count();
                (count > 0).then_some((*kind, count))
            })
            .collect()
    }

    /// The ids its changes name, each once, in order.
    pub fn changed_ids(&self) -> Vec<String> {
        distinct(
            self.update
                .iter()
                .filter_map(|c| c.0.get("id").and_then(Value::as_str)),
        )
    }

    /// The ids its removals name, each once, in order.
    pub fn removed_ids(&self) -> Vec<String> {
        distinct(self.remove.iter().map(String::as_str))
    }

    /// The addresses its adds and changes set as `link`, each once, in order; and the
    /// ids whose link a change removes (`"link": null`), each once.
    pub fn links(&self) -> (Vec<String>, Vec<String>) {
        let set = self
            .add
            .iter()
            .map(|s| &s.0)
            .chain(self.update.iter().map(|c| &c.0))
            .filter_map(|map| map.get("link").and_then(Value::as_str))
            .map(str::trim)
            .filter(|link| !link.is_empty());
        let unset = self
            .update
            .iter()
            .filter(|c| matches!(c.0.get("link"), Some(Value::Null)))
            .filter_map(|c| c.0.get("id").and_then(Value::as_str));
        (distinct(set), distinct(unset))
    }

    /// The drawing as compact JSON with its keys in one fixed order, whatever order the
    /// build's serde_json keeps (the seed of its `HashedMaking`, and tests).
    pub fn to_json(&self) -> String {
        let mut out = String::from("{");
        if let Some(title) = &self.title {
            out.push_str("\"title\":");
            out.push_str(&Value::String(title.clone()).to_string());
            out.push(',');
        }
        let lists: [(&str, Vec<Value>); 3] = [
            (
                "add",
                self.add
                    .iter()
                    .map(|s| Value::Object(s.0.clone()))
                    .collect(),
            ),
            (
                "update",
                self.update
                    .iter()
                    .map(|c| Value::Object(c.0.clone()))
                    .collect(),
            ),
            (
                "remove",
                self.remove.iter().cloned().map(Value::String).collect(),
            ),
        ];
        for (n, (key, items)) in lists.iter().enumerate() {
            if n > 0 {
                out.push(',');
            }
            out.push('"');
            out.push_str(key);
            out.push_str("\":");
            canonical(&Value::Array(items.clone()), &mut out);
        }
        out.push('}');
        out
    }
}

/// Each once, in the order first seen.
fn distinct<'a>(items: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for item in items {
        if !out.iter().any(|known| known == item) {
            out.push(item.to_string());
        }
    }
    out
}

/// JSON with the keys of every object sorted.
fn canonical(value: &Value, out: &mut String) {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (n, key) in keys.into_iter().enumerate() {
                if n > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(key.clone()).to_string());
                out.push(':');
                canonical(&map[key], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (n, item) in items.iter().enumerate() {
                if n > 0 {
                    out.push(',');
                }
                canonical(item, out);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}
