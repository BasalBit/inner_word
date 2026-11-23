//! What agents write: skeletons made into Excalidraw 0.18.1's elements, changes by id and
//! removals (`docs/whiteboard-design.md`, sections 7.1 to 7.3).

mod add;
mod fields;
pub(crate) mod make;
mod remove;
mod update;

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::limits::{MAX_ELEMENTS, MAX_SCENE_BYTES};
use crate::{Element, FileHex, Making, Scene};

/// An element as an agent writes it: a subset of Excalidraw's `ExcalidrawElementSkeleton`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Skeleton(pub Map<String, Value>);

/// A change of one element by id.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ElementChange(pub Map<String, Value>);

/// A picture an agent's `image` skeleton names, as the process that was asked made it
/// (section 7.2): keyed by the skeleton's `path` as written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    pub file: FileHex,
    pub mime_type: String,
    pub pixels: (u32, u32),
    /// Pixels to the inch the file declares (72 when it says nothing, and for an SVG).
    pub density: u32,
}

/// The pictures of a request, by the `path` their skeletons name.
pub type Pictures = BTreeMap<String, Picture>;

/// An element an agent added, with the ids of what came with it (its label).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AddedElement {
    pub id: String,
    pub r#type: String,
    pub with: Vec<String>,
}

/// What [`add`] did.
#[derive(Clone, Debug, PartialEq)]
pub struct Added {
    pub elements: Vec<AddedElement>,
    pub warnings: Vec<String>,
}

/// What [`update`] did: the ids of every element it changed (followers included).
#[derive(Clone, Debug, PartialEq)]
pub struct Updated {
    pub changed: Vec<String>,
    pub warnings: Vec<String>,
}

/// What [`remove`] did: the ids removed (labels included), the ids of arrows unbound.
#[derive(Clone, Debug, PartialEq)]
pub struct Removed {
    pub removed: Vec<String>,
    pub unbound: Vec<String>,
}

/// Why an agent's request was refused; nothing was changed.
#[derive(Clone, Debug, PartialEq)]
pub enum BoardError {
    /// Exit 2: the element's index in the request and the field, where there are.
    Invalid {
        element: Option<usize>,
        field: Option<String>,
        message: String,
    },
    /// Exit 3: an id the whiteboard does not have, with the ids it has (at most twenty,
    /// the nearest first).
    NotFound { id: String, known: Vec<String> },
    /// Exit 2: over one of the limits for agents.
    TooLarge { what: &'static str, limit: usize },
}

/// A limit in words: "16 MB", "2,000".
pub(crate) fn limit_words(limit: usize) -> String {
    if limit >= 1024 * 1024 && limit.is_multiple_of(1024 * 1024) {
        format!("{} MB", limit / (1024 * 1024))
    } else {
        let digits = limit.to_string();
        let mut out = String::new();
        for (n, c) in digits.chars().enumerate() {
            if n > 0 && (digits.len() - n).is_multiple_of(3) {
                out.push(',');
            }
            out.push(c);
        }
        out
    }
}

impl std::fmt::Display for BoardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BoardError::Invalid {
                element,
                field,
                message,
            } => {
                match (element, field) {
                    (Some(n), Some(field)) => write!(f, "the element at index {n} ({field}): ")?,
                    (Some(n), None) => write!(f, "the element at index {n}: ")?,
                    (None, Some(field)) => write!(f, "{field}: ")?,
                    (None, None) => {}
                }
                f.write_str(message)
            }
            BoardError::NotFound { id, known } => {
                write!(f, "the whiteboard has no element {id}")?;
                if !known.is_empty() {
                    write!(f, " (it has {})", known.join(", "))?;
                }
                Ok(())
            }
            BoardError::TooLarge { what, limit } => match *what {
                "request" => write!(
                    f,
                    "a request may hold at most {} elements",
                    limit_words(*limit)
                ),
                "elements" => write!(
                    f,
                    "a whiteboard may hold at most {} elements",
                    limit_words(*limit)
                ),
                "text" => write!(
                    f,
                    "a text may be at most {} characters long",
                    limit_words(*limit)
                ),
                what => write!(f, "the {what} would be larger than {}", limit_words(*limit)),
            },
        }
    }
}

impl std::error::Error for BoardError {}

/// Adds elements made from `skeletons` on top of the drawing (section 7.2): every value
/// derived from `making`, every refusal before anything is changed (on an error the scene
/// is as it was). An `image` skeleton whose path is not in `pictures` is refused (the
/// caller prepares every path of [`picture_paths`] first). An element added in a frame, or
/// a frame added around elements, that is not wholly inside it is a warning: what is
/// outside the frame is not shown.
pub fn add(
    scene: &mut Scene,
    skeletons: &[Skeleton],
    pictures: &Pictures,
    making: &mut dyn Making,
) -> Result<Added, BoardError> {
    let mut work = scene.clone();
    let mut added = add::add(&mut work, skeletons, pictures, making)?;
    let ids: Vec<String> = added
        .elements
        .iter()
        .flat_map(|e| std::iter::once(e.id.clone()).chain(e.with.iter().cloned()))
        .collect();
    added
        .warnings
        .extend(crate::frames::sticking_out(&work, &ids));
    *scene = work;
    Ok(added)
}

/// The paths the `image` skeletons name, distinct, in order: what the caller reads first.
pub fn picture_paths(skeletons: &[Skeleton]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for skeleton in skeletons {
        if skeleton.0.get("type").and_then(Value::as_str) == Some("image")
            && let Some(path) = skeleton.0.get("path").and_then(Value::as_str)
            && !out.iter().any(|p| p == path)
        {
            out.push(path.to_string());
        }
    }
    out
}

/// Changes elements by id (section 7.3), with what follows them. On an error the scene is
/// as it was. An element changed, or a child of a frame changed, that is left not wholly
/// inside its frame is a warning: what is outside the frame is not shown.
///
/// An elbow arrow (`elbowed: true`, which only the writer draws) keeps its route and its
/// bindings with their `fixedPoint` (section 7.1): its `start`, `end`, `points`, `width`,
/// `height` and `angle` are refused, and so are `x`, `y`, `dx` and `dy` while it is bound
/// (Excalidraw does not drag one alone either). When what it is bound to moves, it stays
/// where it is, bound, and Excalidraw routes it again when the writer next moves those
/// elements; when everything it is bound to moved by one distance in a change (a frame
/// that holds both its ends), it moves whole with them.
pub fn update(
    scene: &mut Scene,
    changes: &[ElementChange],
    making: &mut dyn Making,
) -> Result<Updated, BoardError> {
    let mut work = scene.clone();
    let mut updated = update::update(&mut work, changes, making)?;
    let out = crate::frames::sticking_out(&work, &updated.changed);
    updated.warnings.extend(out);
    *scene = work;
    Ok(updated)
}

/// Removes elements by id (section 7.3): a shape with its label, an arrow with its label, a
/// label alone; arrows bound to what goes stay, unbound; a frame's children stay; a
/// picture's entry goes when nothing names it. On an error the scene is as it was.
pub fn remove(
    scene: &mut Scene,
    ids: &[String],
    making: &mut dyn Making,
) -> Result<Removed, BoardError> {
    let mut work = scene.clone();
    let removed = remove::remove(&mut work, ids, making)?;
    *scene = work;
    Ok(removed)
}

/// Where each element of a scene stands, by id: made once per operation, so that a request
/// of 2,000 elements on a board of 20,000 does not walk the board for each element it
/// names (`Scene::element` does). An operation only replaces elements in place and appends;
/// [`Positions::push`] appends.
pub(crate) struct Positions(HashMap<String, usize>);

impl Positions {
    pub(crate) fn of(scene: &Scene) -> Positions {
        let mut map = HashMap::with_capacity(scene.elements.len());
        for (n, element) in scene.elements.iter().enumerate() {
            // The first of two with one id, as `Scene::element` finds it.
            map.entry(element.id().to_string()).or_insert(n);
        }
        Positions(map)
    }

    pub(crate) fn index(&self, id: &str) -> Option<usize> {
        self.0.get(id).copied()
    }

    pub(crate) fn get<'s>(&self, scene: &'s Scene, id: &str) -> Option<&'s Element> {
        scene.elements.get(self.index(id)?)
    }

    pub(crate) fn get_mut<'s>(&self, scene: &'s mut Scene, id: &str) -> Option<&'s mut Element> {
        scene.elements.get_mut(self.index(id)?)
    }

    pub(crate) fn push(&mut self, scene: &mut Scene, element: Element) {
        self.0
            .entry(element.id().to_string())
            .or_insert(scene.elements.len());
        scene.elements.push(element);
    }
}

/// A new id: from `making`, skipping any that is taken.
pub(crate) fn new_id(taken: &mut BTreeSet<String>, making: &mut dyn Making) -> String {
    loop {
        let id = making.id();
        if taken.insert(id.clone()) {
            return id;
        }
    }
}

/// The limits for agents, on the scene after a change.
pub(crate) fn check_limits(scene: &Scene) -> Result<(), BoardError> {
    if scene.elements().len() > MAX_ELEMENTS {
        return Err(BoardError::TooLarge {
            what: "elements",
            limit: MAX_ELEMENTS,
        });
    }
    if scene.to_stored().len() > MAX_SCENE_BYTES {
        return Err(BoardError::TooLarge {
            what: "whiteboard",
            limit: MAX_SCENE_BYTES,
        });
    }
    Ok(())
}

/// The ids a whiteboard has, as candidates for one it does not: at most twenty, the
/// nearest by name first, labels last.
pub(crate) fn known_ids(scene: &Scene, wanted: &str) -> Vec<String> {
    let mut ids: Vec<(bool, usize, usize, &str)> = scene
        .elements()
        .iter()
        .enumerate()
        .map(|(n, e)| {
            (
                e.container_id().is_some(),
                edit_distance(wanted, e.id()),
                n,
                e.id(),
            )
        })
        .collect();
    ids.sort();
    ids.into_iter()
        .take(20)
        .map(|(_, _, _, id)| id.to_string())
        .collect()
}

fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.to_lowercase().chars().collect();
    let b: Vec<char> = b.to_lowercase().chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut previous = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let current = row[j + 1];
            row[j + 1] = if ca == cb {
                previous
            } else {
                1 + previous.min(row[j]).min(row[j + 1])
            };
            previous = current;
        }
    }
    row[b.len()]
}
