//! The delta between two stored scenes, and the merge by the base rule
//! (`docs/whiteboard-design.md`, section 4.5).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde_json::Value;

use crate::json;
use crate::{Element, FileEntry, Scene};

/// What changed from one scene to the next: what the view's `adopt` takes.
#[derive(Clone, Debug, PartialEq)]
pub struct Delta {
    /// Elements added or changed, each with its version in the scene before (`None`: new).
    pub upserted: Vec<(Element, Option<i64>)>,
    /// Elements gone, by id, with their version in the scene before.
    pub removed: Vec<(String, i64)>,
    /// Pictures that are new, by Excalidraw's file id.
    pub files: BTreeMap<String, FileEntry>,
}

/// The result of [`merge`]: the scene, and the ids of their changes that were not taken
/// because the writer changed those elements since the base.
#[derive(Clone, Debug, PartialEq)]
pub struct Merged {
    pub scene: Scene,
    pub conflicts: Vec<String>,
}

impl Delta {
    /// Nothing changed.
    pub fn is_empty(&self) -> bool {
        self.upserted.is_empty() && self.removed.is_empty() && self.files.is_empty()
    }

    /// What the glue's `adopt` takes (section 4.5): `{"baseRev", "upserted": [{"element",
    /// "baseVersion"}], "removed": [{"id", "baseVersion"}], "files": {id: {"mimeType",
    /// "file"}}, "undoable"}`.
    pub fn to_json(&self, base_rev: u64, undoable: bool) -> String {
        let mut out = String::new();
        out.push_str("{\"baseRev\":");
        out.push_str(&base_rev.to_string());
        out.push_str(",\"files\":{");
        for (n, (id, entry)) in self.files.iter().enumerate() {
            if n > 0 {
                out.push(',');
            }
            json::write_string(&mut out, id);
            out.push_str(":{\"file\":");
            match &entry.file {
                Some(file) => json::write_string(&mut out, file.as_str()),
                None => out.push_str("null"),
            }
            out.push_str(",\"mimeType\":");
            json::write_string(&mut out, &entry.mime_type);
            out.push('}');
        }
        out.push_str("},\"removed\":[");
        for (n, (id, version)) in self.removed.iter().enumerate() {
            if n > 0 {
                out.push(',');
            }
            out.push_str("{\"baseVersion\":");
            out.push_str(&version.to_string());
            out.push_str(",\"id\":");
            json::write_string(&mut out, id);
            out.push('}');
        }
        out.push_str("],\"undoable\":");
        out.push_str(if undoable { "true" } else { "false" });
        out.push_str(",\"upserted\":[");
        for (n, (element, version)) in self.upserted.iter().enumerate() {
            if n > 0 {
                out.push(',');
            }
            out.push_str("{\"baseVersion\":");
            match version {
                Some(v) => out.push_str(&v.to_string()),
                None => out.push_str("null"),
            }
            out.push_str(",\"element\":");
            json::write_object(&mut out, element.json());
            out.push('}');
        }
        out.push_str("]}");
        out
    }
}

impl Scene {
    /// What changed from `before` to `self`: elements added or changed (`version` or
    /// `versionNonce` differs), each with its version in `before` as the base version
    /// (`None`: new); elements gone, by id with their version in `before`; pictures that
    /// are new.
    pub fn delta_from(&self, before: &Scene) -> Delta {
        let old: HashMap<&str, &Element> = before.elements.iter().map(|e| (e.id(), e)).collect();
        let mut upserted = Vec::new();
        let mut present: BTreeSet<&str> = BTreeSet::new();
        for element in &self.elements {
            present.insert(element.id());
            match old.get(element.id()) {
                None => upserted.push((element.clone(), None)),
                Some(previous) => {
                    if previous.version() != element.version() || previous.version_nonce() != element.version_nonce() {
                        upserted.push((element.clone(), Some(previous.version())));
                    }
                }
            }
        }
        let removed = before
            .elements
            .iter()
            .filter(|e| !present.contains(e.id()))
            .map(|e| (e.id().to_string(), e.version()))
            .collect();
        let files = self
            .files
            .iter()
            .filter(|(id, entry)| before.files.get(*id) != Some(*entry))
            .map(|(id, entry)| (id.clone(), entry.clone()))
            .collect();
        Delta { upserted, removed, files }
    }
}

/// The base rule of section 4.5, on stored scenes: for each element their change touched,
/// the writer's copy where the writer changed it since `base`, else theirs.
///
/// | The writer's copy | Their upserted element | Their removed element |
/// |---|---|---|
/// | at its version in `base` | theirs, at its version or the writer's + 1, the higher | removed |
/// | at another version | the writer's stays; a conflict | the writer's stays; a conflict |
/// | none, new in theirs | theirs | — |
/// | none, in `base` (the writer removed it) | nothing; a conflict | nothing |
/// | present, new in theirs (the writer gave the id to a new element) | the writer's stays; a conflict | — |
///
/// The canvas settings are the writer's unless theirs changed them and the writer did not;
/// pictures are the writer's and their new ones, an entry nothing names left out. The
/// result is in the order of the elements' fractional indices.
pub fn merge(base: &Scene, writer: &Scene, theirs: &Scene) -> Merged {
    let delta = theirs.delta_from(base);
    let mut elements: Vec<Element> = writer.elements.clone();
    let mut conflicts = Vec::new();
    let position: HashMap<String, usize> = elements.iter().enumerate().map(|(n, e)| (e.id().to_string(), n)).collect();
    let mut appended: Vec<Element> = Vec::new();
    let mut drop: BTreeSet<String> = BTreeSet::new();
    for (incoming, base_version) in &delta.upserted {
        let id = incoming.id().to_string();
        match (position.get(&id), base_version) {
            (Some(&n), Some(v)) => {
                if elements[n].version() == *v {
                    let mut taken = incoming.clone();
                    let version = incoming.version().max(elements[n].version() + 1);
                    taken.set("version", Value::from(version));
                    elements[n] = taken;
                } else {
                    conflicts.push(id);
                }
            }
            (Some(_), None) => conflicts.push(id),
            (None, None) => appended.push(incoming.clone()),
            (None, Some(_)) => conflicts.push(id),
        }
    }
    for (id, base_version) in &delta.removed {
        if let Some(&n) = position.get(id) {
            if elements[n].version() == *base_version {
                drop.insert(id.clone());
            } else {
                conflicts.push(id.clone());
            }
        }
    }
    elements.retain(|e| !drop.contains(e.id()));
    elements.extend(appended);
    order_by_index(&mut elements);

    let app_state = if theirs.app_state != base.app_state && writer.app_state == base.app_state {
        theirs.app_state.clone()
    } else {
        writer.app_state.clone()
    };
    let mut files = writer.files.clone();
    for (id, entry) in &delta.files {
        files.entry(id.clone()).or_insert_with(|| entry.clone());
    }
    let mut scene = Scene { elements, app_state, files };
    scene.drop_unnamed_files();
    Merged { scene, conflicts }
}

/// Elements in the order of their fractional indices, as Excalidraw orders them; an
/// element without a valid index keeps its place after the one before it.
fn order_by_index(elements: &mut Vec<Element>) {
    if elements.iter().all(|e| e.index().is_some_and(crate::fractional::is_valid)) {
        elements.sort_by(|a, b| a.index().cmp(&b.index()));
    }
    // Indices that do not grow are made to (as Excalidraw's `syncInvalidIndices` does on a
    // load): an element whose index is not above the one before it gets the next key.
    let mut last: Option<String> = None;
    for element in elements.iter_mut() {
        let index = element.index().filter(|i| crate::fractional::is_valid(i)).map(str::to_string);
        let fine = match (&index, &last) {
            (Some(i), Some(l)) => i > l,
            (Some(_), None) => true,
            (None, _) => false,
        };
        if fine {
            last = index;
        } else {
            let key = crate::fractional::key_after(last.as_deref());
            element.set("index", Value::String(key.clone()));
            last = Some(key);
        }
    }
}
