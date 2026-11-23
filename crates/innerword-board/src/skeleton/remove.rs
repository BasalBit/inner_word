//! `remove`: elements removed by id, with their labels (section 7.3).

use std::collections::BTreeSet;

use serde_json::Value;

use super::make::{bump, label_id, remove_bound};
use super::{BoardError, Removed, known_ids};
use crate::limits::MAX_REQUEST_ELEMENTS;
use crate::{ElementType, Making, Scene};

pub(super) fn remove(
    scene: &mut Scene,
    ids: &[String],
    making: &mut dyn Making,
) -> Result<Removed, BoardError> {
    if ids.len() > MAX_REQUEST_ELEMENTS {
        return Err(BoardError::TooLarge {
            what: "request",
            limit: MAX_REQUEST_ELEMENTS,
        });
    }
    for id in ids {
        if scene.element(id).is_none() {
            return Err(BoardError::NotFound {
                id: id.clone(),
                known: known_ids(scene, id),
            });
        }
    }
    // What goes: each id, and the label of each that is not a label.
    let mut going: Vec<String> = Vec::new();
    for id in ids {
        if !going.contains(id) {
            going.push(id.clone());
        }
        let element = scene.element(id).cloned();
        if let Some(element) = element
            && element.container_id().is_none()
            && let Some(label) = label_id(&element)
            && scene.element(&label).is_some()
            && !going.contains(&label)
        {
            going.push(label);
        }
    }
    let gone: BTreeSet<&str> = going.iter().map(String::as_str).collect();

    let mut unbound: Vec<String> = Vec::new();
    let mut changed: BTreeSet<String> = BTreeSet::new();
    for element in scene.elements.iter_mut() {
        let id = element.id().to_string();
        if gone.contains(id.as_str()) {
            continue;
        }
        let mut touched = false;
        // Arrows bound to what goes stay, unbound.
        if element.kind() == ElementType::Arrow {
            for key in ["startBinding", "endBinding"] {
                let bound_to = element
                    .json()
                    .get(key)
                    .and_then(|b| b.get("elementId"))
                    .and_then(Value::as_str);
                if bound_to.is_some_and(|b| gone.contains(b)) {
                    element.set(key, Value::Null);
                    touched = true;
                    if !unbound.contains(&id) {
                        unbound.push(id.clone());
                    }
                }
            }
        }
        // What goes leaves the boundElements of what stays.
        let bound: Vec<String> = element
            .bound_elements()
            .iter()
            .map(|(b, _)| b.to_string())
            .collect();
        for b in bound {
            if gone.contains(b.as_str()) && remove_bound(element, &b) {
                touched = true;
            }
        }
        // A label whose container goes went with it; a frame's children stay, out of it.
        if element.frame_id().is_some_and(|f| gone.contains(f)) {
            element.set("frameId", Value::Null);
            touched = true;
        }
        if element.container_id().is_some_and(|c| gone.contains(c)) {
            element.set("containerId", Value::Null);
            touched = true;
        }
        if touched {
            changed.insert(id);
        }
    }
    for id in &changed {
        if let Some(element) = scene.element_mut(id) {
            bump(element, making);
        }
    }
    scene.elements.retain(|e| !gone.contains(e.id()));
    scene.drop_unnamed_files();
    Ok(Removed {
        removed: going,
        unbound,
    })
}
