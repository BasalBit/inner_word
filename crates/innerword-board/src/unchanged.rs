//! A change that sets what an element already has changes nothing: the rule agents and
//! the assistant's drawings share (`docs/whiteboard-design.md`, 7.7; moved here from
//! `innerword-agent`).

use serde_json::Value;

use crate::{Element, ElementChange, Scene};

/// The fields of an element that every change of it sets anew: they say that it changed,
/// not what it is.
pub(crate) const STAMPS: [&str; 3] = ["version", "versionNonce", "updated"];

/// Whether two values are the same as JSON reads them (a number by its value: `100` and
/// `100.0` are one).
pub(crate) fn same_value(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(a), Value::Number(b)) => a == b || a.as_f64() == b.as_f64(),
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same_value(a, b))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(key, a)| b.get(key).is_some_and(|b| same_value(a, b)))
        }
        _ => a == b,
    }
}

/// Whether an element is what it was, its stamps ([`STAMPS`]) aside.
pub(crate) fn same_element(before: &Element, after: &Element) -> bool {
    let (before, after) = (before.json(), after.json());
    let fields = |map: &'_ serde_json::Map<String, Value>| {
        map.keys()
            .filter(|key| !STAMPS.contains(&key.as_str()))
            .count()
    };
    fields(before) == fields(after)
        && before
            .iter()
            .filter(|(key, _)| !STAMPS.contains(&key.as_str()))
            .all(|(key, value)| after.get(key).is_some_and(|other| same_value(value, other)))
}

/// Which changes of an update change something, each judged against the scene as it
/// stood just before it, as the update applies them one after the other (`apply` runs
/// some of them on a scene). A change is no change when it leaves its element and that
/// element's label as they were then: it set what the element already had.
///
/// Judging a change by the scene after the whole update would drop one that undoes what
/// another change of the request did (a frame moved and its child set back where it
/// was), and the drawing saved would not be the one asked for. So the whole update
/// (`before` to `after`) only says which changes may be no change (their elements end as
/// they were); those are judged where they stand. A run of them that leaves every element
/// it names as it was is no change at all (one request that sets every element as it is
/// costs one more update, not one per change); otherwise each is applied alone.
pub fn changes_that_change<E>(
    changes: &[ElementChange],
    before: &Scene,
    after: &Scene,
    mut apply: impl FnMut(&mut Scene, &[ElementChange]) -> Result<(), E>,
) -> Result<Vec<bool>, E> {
    let maybe: Vec<bool> = changes
        .iter()
        .map(|change| left_as_it_was(change, before, after))
        .collect();
    let mut kept = vec![true; changes.len()];
    if !maybe.contains(&true) {
        return Ok(kept);
    }
    let mut work = before.clone();
    let mut start = 0;
    while start < changes.len() {
        let mut end = start + 1;
        while end < changes.len() && maybe[end] == maybe[start] {
            end += 1;
        }
        let run = &changes[start..end];
        if !maybe[start] {
            apply(&mut work, run)?;
        } else {
            let was = work.clone();
            apply(&mut work, run)?;
            if run.iter().all(|change| left_as_it_was(change, &was, &work)) {
                kept[start..end].fill(false);
                work = was;
            } else {
                work = was;
                for (n, change) in run.iter().enumerate() {
                    let was = work.clone();
                    apply(&mut work, std::slice::from_ref(change))?;
                    if left_as_it_was(change, &was, &work) {
                        kept[start + n] = false;
                        work = was;
                    }
                }
            }
        }
        start = end;
    }
    Ok(kept)
}

/// Whether a change of an update left its element as it was: the element it names and
/// that element's label are what they were in `before` and in `after`. Between the scene
/// just before the change and just after it, such a change set what the element already
/// has, and is no change at all.
pub(crate) fn left_as_it_was(change: &ElementChange, before: &Scene, after: &Scene) -> bool {
    let Some(id) = change.0.get("id").and_then(Value::as_str) else {
        return false;
    };
    let label = |scene: &Scene| {
        scene.element(id).and_then(|element| {
            element
                .bound_elements()
                .into_iter()
                .find(|(_, kind)| *kind == "text")
                .map(|(label, _)| label.to_string())
        })
    };
    let same = |id: &str| match (before.element(id), after.element(id)) {
        (Some(was), Some(is)) => same_element(was, is),
        (None, None) => true,
        _ => false,
    };
    let (label_before, label_after) = (label(before), label(after));
    label_before == label_after && same(id) && label_before.as_deref().is_none_or(same)
}
