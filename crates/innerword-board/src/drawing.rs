//! What the assistant proposes: a drawing from an answer, in the agents' own formats
//! (`docs/assistant-draws-design.md`, sections 2 and 5; `docs/whiteboard-design.md`, 7.2
//! and 7.3), read and checked here ([`Drawing::from_json`]) before any whiteboard is
//! looked at.

use std::collections::{BTreeSet, HashSet};

use serde_json::{Map, Value};

use crate::geometry::element_bounds;
use crate::limits::{MAX_ID_CHARS, MAX_REQUEST_ELEMENTS};
use crate::skeleton::{ElementChange, Skeleton, is_id, limit_words};
use crate::{AddedElement, BoardError, Bounds, Making, Pictures, Scene};

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
    /// Two elements of its `add` have the id `id`.
    IdTwice { id: String },
    /// Its `add` gives `id`, which a drawing before it (an earlier block of the answer)
    /// adds already: the blocks are drawn together, and the converter takes an id once.
    AddedAgain { id: String },
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
            DrawingError::IdTwice { id } => {
                write!(f, "it adds two elements with the id {}", cut(id, 40))
            }
            DrawingError::AddedAgain { id } => write!(f, "it adds {} again", cut(id, 40)),
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
        let mut added: HashSet<String> = HashSet::new();
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
                if let Some(id) = id.as_str()
                    && !added.insert(id.to_string())
                {
                    return Err(DrawingError::IdTwice { id: id.to_string() });
                }
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

    /// The ids its `add` gives its elements, in order (an element without one is left
    /// out).
    pub fn added_ids(&self) -> Vec<&str> {
        self.add
            .iter()
            .filter_map(|s| s.0.get("id").and_then(Value::as_str))
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

// ----- applying a drawing ---------------------------------------------------------------

/// The room kept free around a placed drawing.
pub const PLACE_MARGIN: f64 = 40.0;
/// The distance from the drawn area of a drawing put beside it.
pub const PLACE_GAP: f64 = 100.0;
/// The most places in view [`place`] tries.
const MAX_PLACES: usize = 2_500;

/// Where a drawing that stands on its own was put (section 5.3).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    /// How far its elements were moved from where the answer wrote them.
    pub dx: f64,
    pub dy: f64,
    /// Whether all of it lies within the view given (`false` without a view).
    pub in_view: bool,
}

/// What [`draw`] did.
#[derive(Clone, Debug, PartialEq)]
pub struct Drawn {
    /// The elements added, as `add` reports them (ids as made).
    pub added: Vec<AddedElement>,
    /// Every element its changes changed, followers included, after the changes that set
    /// what an element already had were left out.
    pub changed: Vec<String>,
    /// Changes left out because they set what their element already had (by index in
    /// `update`).
    pub unchanged: Vec<usize>,
    /// The elements removed, labels included.
    pub removed: Vec<String>,
    /// Arrows left unbound by a removal.
    pub unbound: Vec<String>,
    /// Ids its removals named that the whiteboard no longer has: left out.
    pub already_gone: Vec<String>,
    /// Ids the answer gave new elements that the whiteboard had: (as written, as made).
    pub renamed: Vec<(String, String)>,
    /// `Some` for a drawing that stands on its own (only `add`, naming nothing that was on
    /// the whiteboard); `None`: it stays where it was written.
    pub placement: Option<Placement>,
    /// The converter's warnings ("db sticks out of frame backend: …"), after placement.
    pub warnings: Vec<String>,
}

impl Drawn {
    /// The elements the drawing put on the whiteboard or changed there (labels and
    /// followers included), each once: what the writer is to see after it (section 14).
    pub fn shown_ids(&self) -> Vec<String> {
        let mut seen: HashSet<&str> = HashSet::new();
        self.added
            .iter()
            .flat_map(|e| std::iter::once(&e.id).chain(e.with.iter()))
            .chain(self.changed.iter())
            .filter(|id| seen.insert(id.as_str()))
            .cloned()
            .collect()
    }

    /// Whether the scene is as it was: nothing added, every change left out (it set what
    /// its element had) and every removal gone already. Then [`draw`] left the scene
    /// untouched, and the caller keeps no version, saves nothing and adopts nothing.
    pub fn is_nothing(&self) -> bool {
        self.added.is_empty() && self.changed.is_empty() && self.removed.is_empty()
    }
}

/// Why nothing was drawn.
#[derive(Clone, Debug, PartialEq)]
pub enum DrawError {
    /// It names elements the scene does not have: `changed` by the ids of its changes,
    /// `named` by references (an arrow's `start` or `end`, a frame's `children`, in `add`
    /// or in `update`) that are neither in the scene nor among its own adds.
    Gone {
        changed: Vec<String>,
        named: Vec<String>,
    },
    /// The converter's refusal of one entry of the drawing: `list` is "add" or "update",
    /// `index` the entry's place in that list (of the blocks joined), with the field and
    /// the converter's words.
    Entry {
        list: &'static str,
        index: usize,
        field: Option<String>,
        message: String,
    },
    /// The converter's refusal of the drawing as a whole: the whiteboard's limits for
    /// agents, and what no entry is named for.
    Board(BoardError),
}

impl DrawError {
    /// A refusal of the converter about the entry `index` of `list` (its index there, as
    /// the converter was given the list): [`DrawError::Entry`] for an entry it names,
    /// `index` mapping it to the drawing's own list; else [`DrawError::Board`].
    fn of_entry(
        list: &'static str,
        index: impl Fn(usize) -> Option<usize>,
    ) -> impl Fn(BoardError) -> DrawError {
        move |error| match error {
            BoardError::Invalid {
                element: Some(n),
                field,
                message,
            } => match index(n) {
                Some(index) => DrawError::Entry {
                    list,
                    index,
                    field,
                    message,
                },
                None => DrawError::Board(BoardError::Invalid {
                    element: None,
                    field,
                    message,
                }),
            },
            other => DrawError::Board(other),
        }
    }
}

/// "api", "api and db", "a, b, c, d, e and 3 more".
fn ids_listed(ids: &[String]) -> String {
    let shown: Vec<&str> = ids.iter().take(5).map(String::as_str).collect();
    if ids.len() > 5 {
        format!("{} and {} more", shown.join(", "), ids.len() - 5)
    } else if shown.len() > 1 {
        format!(
            "{} and {}",
            shown[..shown.len() - 1].join(", "),
            shown[shown.len() - 1]
        )
    } else {
        shown.join("")
    }
}

/// The open whiteboard's words: "the answer changes api and db, which the whiteboard does
/// not have" / "the answer joins arrows or frames to cache, which the whiteboard does not
/// have" (an id may have been removed meanwhile, or never been there: the model may have
/// invented it); "the element at index 3 of add (fillStyle): …"; [`BoardError`]'s words.
impl std::fmt::Display for DrawError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DrawError::Gone { changed, named } => {
                let mut clauses = Vec::new();
                if !changed.is_empty() {
                    clauses.push(format!(
                        "the answer changes {}, which the whiteboard does not have",
                        ids_listed(changed)
                    ));
                }
                if !named.is_empty() {
                    clauses.push(format!(
                        "the answer joins arrows or frames to {}, which the whiteboard does not have",
                        ids_listed(named)
                    ));
                }
                f.write_str(&clauses.join("; "))
            }
            DrawError::Entry {
                list,
                index,
                field,
                message,
            } => {
                let what = if *list == "update" {
                    "the change"
                } else {
                    "the element"
                };
                write!(f, "{what} at index {index} of {list}")?;
                if let Some(field) = field {
                    write!(f, " ({field})")?;
                }
                write!(f, ": {message}")
            }
            DrawError::Board(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for DrawError {}

impl From<BoardError> for DrawError {
    fn from(error: BoardError) -> Self {
        DrawError::Board(error)
    }
}

/// The ids an element or a change names besides its own: `start.id`, `end.id`, and the
/// entries of `children`.
fn references(map: &Map<String, Value>) -> Vec<String> {
    let mut out = Vec::new();
    for field in ["start", "end"] {
        if let Some(Value::Object(end)) = map.get(field)
            && let Some(id) = end.get("id").and_then(Value::as_str)
        {
            out.push(id.to_string());
        }
    }
    if let Some(Value::Array(children)) = map.get("children") {
        out.extend(
            children
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string),
        );
    }
    out
}

/// Renames the references of an element of `add` (its `start`, `end` and `children`)
/// that name a renamed id.
fn rename_references(map: &mut Map<String, Value>, renamed: &[(String, String)]) {
    let new_name = |id: &str| {
        renamed
            .iter()
            .find(|(old, _)| old == id)
            .map(|(_, new)| new.clone())
    };
    for field in ["start", "end"] {
        if let Some(Value::Object(end)) = map.get_mut(field)
            && let Some(new) = end.get("id").and_then(Value::as_str).and_then(new_name)
        {
            end.insert("id".into(), Value::String(new));
        }
    }
    if let Some(Value::Array(children)) = map.get_mut("children") {
        for child in children {
            if let Some(new) = child.as_str().and_then(new_name) {
                *child = Value::String(new);
            }
        }
    }
}

/// A free name for `id`: `<stem>-<n>` for the smallest `n` from 2 that `taken` does not
/// have, `<stem>` cut so that the name is at most [`MAX_ID_CHARS`].
fn free_name(id: &str, taken: &HashSet<String>) -> String {
    let mut n = 2usize;
    loop {
        let suffix = format!("-{n}");
        let room = MAX_ID_CHARS.saturating_sub(suffix.len());
        // Ids are ASCII (`is_id`), so a byte is a character.
        let stem = &id[..id.len().min(room)];
        let name = format!("{stem}{suffix}");
        if !taken.contains(&name) {
            return name;
        }
        n += 1;
    }
}

/// The union of rectangles `[x1, y1, x2, y2]`.
fn union(rects: impl IntoIterator<Item = [f64; 4]>) -> Option<[f64; 4]> {
    rects.into_iter().reduce(|a, b| {
        [
            a[0].min(b[0]),
            a[1].min(b[1]),
            a[2].max(b[2]),
            a[3].max(b[3]),
        ]
    })
}

fn rect_of(bounds: Bounds) -> [f64; 4] {
    [
        bounds.x,
        bounds.y,
        bounds.x + bounds.width,
        bounds.y + bounds.height,
    ]
}

fn bounds_of(rect: [f64; 4]) -> Bounds {
    Bounds {
        x: rect[0],
        y: rect[1],
        width: rect[2] - rect[0],
        height: rect[3] - rect[1],
    }
}

/// Applies `drawing` to `scene`, whole or not at all (on an error the scene is as it was;
/// `docs/assistant-draws-design.md`, section 5):
/// 1. removals of ids the scene does not have are left out (`already_gone`);
/// 2. a change of an id the scene does not have, or a reference (`start`, `end`,
///    `children`, in `add` and in `update`) to an id that is neither in the scene nor
///    among the drawing's own adds, is [`DrawError::Gone`] (every such id, each once);
/// 3. an id of `add` that the scene has is renamed, with every reference to it inside
///    `add`; `update` and `remove`, their `start`, `end` and `children` included, always
///    mean the whiteboard's element when the whiteboard has that id;
/// 4. [`add`](crate::add) with no pictures, then, for a drawing that stands on its own,
///    [`place`] and the added elements moved by its offset (one rigid move);
/// 5. [`update`](crate::update) with only the changes that change something
///    ([`changes_that_change`](crate::changes_that_change)), then
///    [`remove`](crate::remove);
/// 6. the warnings of frames computed again after the move.
///
/// **Renaming**: the new name is `<stem>-<n>` for the smallest `n` from 2 that is free
/// against every id of the scene, every id written in `add` and every name given by an
/// earlier renaming; `<stem>` is the id cut so that the name is at most 64 characters.
///
/// The limits are the agents'. `view` is where the writer is looking, in scene
/// coordinates (`None`: unknown, or a new whiteboard).
pub fn draw(
    scene: &mut Scene,
    drawing: &Drawing,
    view: Option<Bounds>,
    making: &mut dyn Making,
) -> Result<Drawn, DrawError> {
    let on_board: HashSet<String> = scene
        .elements()
        .iter()
        .map(|e| e.id().to_string())
        .collect();
    let written: HashSet<String> = drawing
        .add
        .iter()
        .filter_map(|s| s.0.get("id").and_then(Value::as_str))
        .map(str::to_string)
        .collect();

    // 1, 2: what is gone.
    let mut already_gone = Vec::new();
    let mut to_remove = Vec::new();
    for id in drawing.removed_ids() {
        if on_board.contains(&id) {
            to_remove.push(id);
        } else {
            already_gone.push(id);
        }
    }
    let mut gone_changed: Vec<String> = Vec::new();
    let mut gone_named: Vec<String> = Vec::new();
    let note = |list: &mut Vec<String>, id: &str| {
        if !list.iter().any(|known| known == id) {
            list.push(id.to_string());
        }
    };
    for change in &drawing.update {
        if let Some(id) = change.0.get("id").and_then(Value::as_str)
            && !on_board.contains(id)
        {
            note(&mut gone_changed, id);
        }
    }
    for map in drawing
        .add
        .iter()
        .map(|s| &s.0)
        .chain(drawing.update.iter().map(|c| &c.0))
    {
        for id in references(map) {
            if !on_board.contains(&id) && !written.contains(&id) {
                note(&mut gone_named, &id);
            }
        }
    }
    if !gone_changed.is_empty() || !gone_named.is_empty() {
        return Err(DrawError::Gone {
            changed: gone_changed,
            named: gone_named,
        });
    }

    // 3: ids of new elements that the whiteboard has.
    let mut taken: HashSet<String> = on_board.iter().cloned().collect();
    taken.extend(written.iter().cloned());
    let mut renamed: Vec<(String, String)> = Vec::new();
    for skeleton in &drawing.add {
        if let Some(id) = skeleton.0.get("id").and_then(Value::as_str)
            && on_board.contains(id)
            && !renamed.iter().any(|(old, _)| old == id)
        {
            let name = free_name(id, &taken);
            taken.insert(name.clone());
            renamed.push((id.to_string(), name));
        }
    }
    let skeletons: Vec<Skeleton> = drawing
        .add
        .iter()
        .map(|skeleton| {
            let mut map = skeleton.0.clone();
            if let Some(new) = map
                .get("id")
                .and_then(Value::as_str)
                .and_then(|id| renamed.iter().find(|(old, _)| old == id))
                .map(|(_, new)| new.clone())
            {
                map.insert("id".into(), Value::String(new));
            }
            rename_references(&mut map, &renamed);
            Skeleton(map)
        })
        .collect();

    // 4: add, and place what stands on its own.
    let mut work = scene.clone();
    let added = if skeletons.is_empty() {
        Vec::new()
    } else {
        crate::add(&mut work, &skeletons, &Pictures::new(), making)
            .map_err(DrawError::of_entry("add", Some))?
            .elements
    };
    let added_ids: Vec<String> = added
        .iter()
        .flat_map(|e| std::iter::once(e.id.clone()).chain(e.with.iter().cloned()))
        .collect();
    let stands_alone = !added.is_empty()
        && drawing.update.is_empty()
        && drawing.remove.is_empty()
        && skeletons
            .iter()
            .all(|s| references(&s.0).iter().all(|id| !on_board.contains(id)));
    let mut placement = None;
    if stands_alone {
        let mine: HashSet<&str> = added_ids.iter().map(String::as_str).collect();
        let group = union(
            work.elements()
                .iter()
                .filter(|e| mine.contains(e.id()))
                .filter_map(element_bounds),
        );
        if let Some(group) = group {
            let occupied: Vec<Bounds> = scene
                .elements()
                .iter()
                .filter_map(element_bounds)
                .map(bounds_of)
                .collect();
            let placed = place(bounds_of(group), &occupied, view);
            if placed.dx != 0.0 || placed.dy != 0.0 {
                for element in work.elements.iter_mut() {
                    if mine.contains(element.id()) {
                        let (x, y) = (element.x(), element.y());
                        element.set_f64("x", x + placed.dx);
                        element.set_f64("y", y + placed.dy);
                    }
                }
            }
            placement = Some(placed);
        }
    }
    let mut warnings: Vec<String> = crate::frames::sticking_out(&work, &added_ids);

    // 5: the changes that change something, then the removals.
    let mut changed = Vec::new();
    let mut unchanged = Vec::new();
    if !drawing.update.is_empty() {
        let before = work.clone();
        crate::update(&mut work, &drawing.update, making)
            .map_err(DrawError::of_entry("update", Some))?;
        // The runs judged here are slices of `update`: an index of theirs names no entry.
        let kept = crate::changes_that_change(&drawing.update, &before, &work, |scene, changes| {
            crate::update(scene, changes, making).map(|_| ())
        })
        .map_err(DrawError::of_entry("update", |_| None))?;
        let effective_at: Vec<usize> = kept
            .iter()
            .enumerate()
            .filter(|(_, kept)| **kept)
            .map(|(n, _)| n)
            .collect();
        let effective: Vec<ElementChange> = effective_at
            .iter()
            .map(|n| drawing.update[*n].clone())
            .collect();
        unchanged = kept
            .iter()
            .enumerate()
            .filter(|(_, kept)| !**kept)
            .map(|(n, _)| n)
            .collect();
        work = before;
        if !effective.is_empty() {
            let updated =
                crate::update(&mut work, &effective, making)
                    .map_err(DrawError::of_entry("update", |n| {
                        effective_at.get(n).copied()
                    }))?;
            changed = updated.changed;
            warnings.extend(updated.warnings);
        }
    }
    let (removed, unbound) = if to_remove.is_empty() {
        (Vec::new(), Vec::new())
    } else {
        let removed = crate::remove(&mut work, &to_remove, making)?;
        (removed.removed, removed.unbound)
    };

    let mut seen: BTreeSet<String> = BTreeSet::new();
    warnings.retain(|warning| seen.insert(warning.clone()));
    let drawn = Drawn {
        added,
        changed,
        unchanged,
        removed,
        unbound,
        already_gone,
        renamed,
        placement,
        warnings,
    };
    if !drawn.is_nothing() {
        *scene = work;
    }
    Ok(drawn)
}

/// Whether two rectangles `[x1, y1, x2, y2]` overlap (touching edges do not).
fn meets(a: &[f64; 4], b: &[f64; 4]) -> bool {
    a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3]
}

/// Where a group of elements with bounds `group` goes, given the bounds of what is on the
/// whiteboard and the view (section 5.3), with `PLACE_MARGIN` free around it:
/// 1. where it was written, when that is free and wholly in the view;
/// 2. else the free place in the view nearest to where it was written (a grid of places
///    over the view shrunk by the margin, at most 2,500, and the place as written clamped
///    into it), nearest first, then by `y`, then by `x`;
/// 3. else (it is larger than the view, or the view is full) as written when free, else
///    beside the drawn area, `PLACE_GAP` to its right.
///
/// Without a view: as written when free, else beside the drawn area.
pub fn place(group: Bounds, occupied: &[Bounds], view: Option<Bounds>) -> Placement {
    let m = PLACE_MARGIN;
    let (w, h) = (group.width.max(0.0), group.height.max(0.0));
    let rects: Vec<[f64; 4]> = occupied.iter().map(|b| rect_of(*b)).collect();
    let free_among = |rects: &[[f64; 4]], x: f64, y: f64| {
        let grown = [x - m, y - m, x + w + m, y + h + m];
        !rects.iter().any(|r| meets(&grown, r))
    };
    let in_view = |x: f64, y: f64| {
        view.is_some_and(|v| {
            x >= v.x && y >= v.y && x + w <= v.x + v.width && y + h <= v.y + v.height
        })
    };
    let at = |x: f64, y: f64| Placement {
        dx: x - group.x,
        dy: y - group.y,
        in_view: in_view(x, y),
    };
    let free_as_written = free_among(&rects, group.x, group.y);
    if free_as_written && in_view(group.x, group.y) {
        return Placement {
            dx: 0.0,
            dy: 0.0,
            in_view: true,
        };
    }
    if let Some(v) = view {
        let (x_min, x_max) = (v.x + m, v.x + v.width - m - w);
        let (y_min, y_max) = (v.y + m, v.y + v.height - m - h);
        if x_min <= x_max && y_min <= y_max {
            let near_view = [v.x - m, v.y - m, v.x + v.width + m, v.y + v.height + m];
            let nearby: Vec<[f64; 4]> = rects
                .iter()
                .filter(|r| meets(r, &near_view))
                .copied()
                .collect();
            let mut step = (v.width.min(v.height) / 24.0).max(16.0);
            let count = |step: f64| {
                let nx = ((x_max - x_min) / step).floor() as usize + 1;
                let ny = ((y_max - y_min) / step).floor() as usize + 1;
                nx.saturating_mul(ny)
            };
            while count(step) > MAX_PLACES {
                step *= 1.25;
            }
            let wanted = (group.x.clamp(x_min, x_max), group.y.clamp(y_min, y_max));
            let nx = ((x_max - x_min) / step).floor() as usize + 1;
            let ny = ((y_max - y_min) / step).floor() as usize + 1;
            let mut places: Vec<(f64, f64)> = Vec::with_capacity(nx * ny + 1);
            places.push(wanted);
            for j in 0..ny {
                for i in 0..nx {
                    places.push((x_min + i as f64 * step, y_min + j as f64 * step));
                }
            }
            let distance = |p: &(f64, f64)| (p.0 - wanted.0).hypot(p.1 - wanted.1);
            places.sort_by(|a, b| {
                distance(a)
                    .total_cmp(&distance(b))
                    .then(a.1.total_cmp(&b.1))
                    .then(a.0.total_cmp(&b.0))
            });
            if let Some((x, y)) = places
                .into_iter()
                .find(|(x, y)| free_among(&nearby, *x, *y))
            {
                return Placement {
                    dx: x - group.x,
                    dy: y - group.y,
                    in_view: true,
                };
            }
        }
    }
    if free_as_written {
        return at(group.x, group.y);
    }
    match union(rects.iter().copied()) {
        Some(drawn) => at(drawn[2] + PLACE_GAP, drawn[1]),
        None => at(group.x, group.y),
    }
}

// ----- bringing a drawing into view -----------------------------------------------------

/// Where the whiteboard's view is: Excalidraw's scroll and zoom, and the size of its canvas
/// in CSS pixels. A scene point `(x, y)` is drawn at `((x + scroll_x) * zoom, (y +
/// scroll_y) * zoom)` of the canvas.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub scroll_x: f64,
    pub scroll_y: f64,
    pub zoom: f64,
    pub width: f64,
    pub height: f64,
}

/// What of the canvas Excalidraw's own toolbars and islands cover, in CSS pixels from
/// each edge: what is under them is not seen.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Insets {
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub left: f64,
}

/// The room [`reveal`] keeps between what it brings into view and the edges of what is
/// seen, in CSS pixels.
pub const REVEAL_MARGIN: f64 = 24.0;
/// The smallest zoom [`reveal`] goes to (Excalidraw's own).
const MIN_ZOOM: f64 = 0.1;

/// The bounds of the elements of `scene` named by `ids` (rotation included), together.
pub fn bounds_of_ids(scene: &Scene, ids: &[String]) -> Option<Bounds> {
    let wanted: HashSet<&str> = ids.iter().map(String::as_str).collect();
    union(
        scene
            .elements()
            .iter()
            .filter(|e| wanted.contains(e.id()))
            .filter_map(element_bounds),
    )
    .map(bounds_of)
}

/// Where the view goes so that `group` (scene coordinates) is seen whole, in the canvas of
/// `view` less `insets` (section 14 of `docs/assistant-draws-design.md`); `None` when it is
/// seen whole already, or the canvas has no room. The zoom is kept, and the view scrolls
/// as little as brings `group` in, `REVEAL_MARGIN` from the edges; only a group that
/// cannot be seen whole at the zoom makes it smaller (never larger), to the zoom at which
/// it fits, centred. The elements never move.
pub fn reveal(group: Bounds, view: Viewport, insets: Insets) -> Option<Viewport> {
    let z = view.zoom;
    if !(z.is_finite() && z > 0.0) {
        return None;
    }
    let (left, top) = (insets.left, insets.top);
    let (right, bottom) = (view.width - insets.right, view.height - insets.bottom);
    let (room_w, room_h) = (right - left, bottom - top);
    if !(room_w > 0.0 && room_h > 0.0) {
        return None;
    }
    let (w, h) = (group.width.max(0.0), group.height.max(0.0));
    let screen = |x: f64, y: f64| ((x + view.scroll_x) * z, (y + view.scroll_y) * z);
    let (x0, y0) = screen(group.x, group.y);
    let (x1, y1) = screen(group.x + w, group.y + h);
    let seen = |a: f64, b: f64| a >= b - 0.5;
    if seen(x0, left) && seen(right, x1) && seen(y0, top) && seen(bottom, y1) {
        return None;
    }
    let m = REVEAL_MARGIN.min(room_w / 4.0).min(room_h / 4.0);
    let (fit_w, fit_h) = (room_w - 2.0 * m, room_h - 2.0 * m);
    if w * z <= fit_w && h * z <= fit_h {
        // One axis at a time, as little as brings it in.
        let shift = |start: f64, end: f64, low: f64, high: f64| {
            if start < low - 0.5 {
                low + m - start
            } else if end > high + 0.5 {
                high - m - end
            } else {
                0.0
            }
        };
        let dx = shift(x0, x1, left, right);
        let dy = shift(y0, y1, top, bottom);
        return Some(Viewport {
            scroll_x: view.scroll_x + dx / z,
            scroll_y: view.scroll_y + dy / z,
            ..view
        });
    }
    let fit = |room: f64, size: f64| {
        if size > 0.0 {
            room / size
        } else {
            f64::INFINITY
        }
    };
    let zoom = fit(fit_w, w).min(fit(fit_h, h)).min(z).max(MIN_ZOOM);
    // Centred in what is seen.
    let (cx, cy) = (left + room_w / 2.0, top + room_h / 2.0);
    Some(Viewport {
        scroll_x: cx / zoom - (group.x + w / 2.0),
        scroll_y: cy / zoom - (group.y + h / 2.0),
        zoom,
        ..view
    })
}
