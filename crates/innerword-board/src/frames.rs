//! Frames as the screen and the PDF show them (`docs/whiteboard-design.md`, section 5): a
//! frame clips its children at its edges, and its name stands above it.
//!
//! Excalidraw 0.18.1 draws a frame's children clipped to the frame on the canvas, and its
//! `exportToSvg` (the PDF's path) clips them the same way once the export page has mended
//! the attribute Excalidraw writes for that (the glue's `export.js`). The size of the
//! export is Excalidraw's `getCanvasSize(getRootElements(…))`: the frames, the elements in
//! no frame, and the name of each frame, a text Excalidraw adds above it; a frame's
//! children add nothing, what of them is outside the frame is not drawn. [`Scene::pdf_bounds`]
//! is that rule in Rust.

use std::collections::{BTreeSet, HashMap};

use crate::geometry::{element_bounds, rotate};
use crate::{Element, ElementType, Scene};

/// Excalidraw's `FRAME_STYLE`: the name of a frame stands `nameOffsetY` above it, in
/// Helvetica at `nameFontSize` with `nameLineHeight`.
const NAME_OFFSET_Y: f64 = 3.0;
const NAME_FONT_SIZE: f64 = 14.0;
const NAME_LINE_HEIGHT: f64 = 1.25;

/// How far an element may pass its frame's edge and still count as inside, in pixels
/// (the rounding of a converted size, not a drawing that sticks out).
const SLACK: f64 = 0.5;

/// The box of a frame's name as Excalidraw's export adds it (`addFrameLabelsAsTextElements`):
/// at the frame's left edge, as many lines high as the name has (`"Frame"` for a frame
/// without one), ending `NAME_OFFSET_Y` above the frame, never wider than the frame (a
/// longer name is cut). Its width is the frame's here: it never passes the frame's sides,
/// and only its height counts for the page.
pub(crate) fn name_bounds(frame: &Element) -> [f64; 4] {
    let lines = frame
        .name()
        .map(|name| {
            name.replace("\r\n", "\n")
                .replace('\r', "\n")
                .split('\n')
                .count()
        })
        .unwrap_or(1)
        .max(1);
    let height = lines as f64 * NAME_FONT_SIZE * NAME_LINE_HEIGHT;
    let (x, y) = (frame.x(), frame.y());
    let bottom = y - NAME_OFFSET_Y;
    [x, bottom - height, x + frame.width().max(0.0), bottom]
}

/// The frames of a scene, by id.
fn frames(scene: &Scene) -> HashMap<&str, &Element> {
    scene
        .elements()
        .iter()
        .filter(|e| e.kind() == ElementType::Frame)
        .map(|e| (e.id(), e))
        .collect()
}

/// Whether an element counts for the page: a frame, or an element whose `frameId` names no
/// frame of the scene (Excalidraw's `getRootElements`).
fn is_root(element: &Element, frames: &HashMap<&str, &Element>) -> bool {
    element.kind() == ElementType::Frame
        || element
            .frame_id()
            .is_none_or(|frame| !frames.contains_key(frame))
}

impl Scene {
    /// What the whiteboard's PDF holds, the margin aside (section 5): the bounds of every
    /// frame with its name above it and of every element in no frame, rotation included. A
    /// frame's children are clipped at its edges and add nothing. `None`: nothing is drawn.
    pub fn pdf_bounds(&self) -> Option<crate::Bounds> {
        let frames = frames(self);
        let mut acc: Option<[f64; 4]> = None;
        let mut join = |b: [f64; 4]| {
            acc = Some(match acc {
                None => b,
                Some(a) => [
                    a[0].min(b[0]),
                    a[1].min(b[1]),
                    a[2].max(b[2]),
                    a[3].max(b[3]),
                ],
            });
        };
        for element in self.elements().iter().filter(|e| is_root(e, &frames)) {
            if let Some(b) = element_bounds(element) {
                join(b);
            }
            if element.kind() == ElementType::Frame {
                join(name_bounds(element));
            }
        }
        acc.map(|b| crate::Bounds {
            x: b[0],
            y: b[1],
            width: b[2] - b[0],
            height: b[3] - b[1],
        })
    }
}

/// Whether an element lies wholly inside a frame (its bounds, rotation included, within
/// the frame's rectangle, turned with the frame).
fn inside(element: &Element, frame: &Element) -> bool {
    let Some([x1, y1, x2, y2]) = element_bounds(element) else {
        return true;
    };
    let (fx, fy, fw, fh) = (frame.x(), frame.y(), frame.width(), frame.height());
    let center = (fx + fw / 2.0, fy + fh / 2.0);
    [(x1, y1), (x2, y1), (x2, y2), (x1, y2)]
        .iter()
        .all(|&corner| {
            let (px, py) = rotate(corner, center, -frame.angle());
            px >= fx - SLACK && px <= fx + fw + SLACK && py >= fy - SLACK && py <= fy + fh + SLACK
        })
}

/// The warnings of an operation that left elements sticking out of their frames (section
/// 7.3): for each element among `ids`, and each child of a frame among `ids`, that is not
/// wholly inside the frame it belongs to, or whose label is not, one sentence naming the
/// element (a label by its container) and the frame, in the order of the scene.
pub(crate) fn sticking_out(scene: &Scene, ids: &[String]) -> Vec<String> {
    let frames = frames(scene);
    if frames.is_empty() {
        return Vec::new();
    }
    let touched: BTreeSet<&str> = ids.iter().map(String::as_str).collect();
    // A label speaks for its container, which is the element the agent names.
    let speaker = |element: &'_ Element| -> String {
        element
            .container_id()
            .filter(|c| scene.element(c).is_some())
            .unwrap_or(element.id())
            .to_string()
    };
    let mut told: BTreeSet<String> = BTreeSet::new();
    let mut out = Vec::new();
    for element in scene.elements() {
        if element.kind() == ElementType::Frame {
            continue;
        }
        let Some(frame) = element.frame_id().and_then(|f| frames.get(f)) else {
            continue;
        };
        let container = element.container_id();
        let concerned = touched.contains(element.id())
            || touched.contains(frame.id())
            || container.is_some_and(|c| touched.contains(c));
        if !concerned || inside(element, frame) {
            continue;
        }
        let who = speaker(element);
        if told.insert(who.clone()) {
            out.push(format!(
                "{who} sticks out of frame {}: what is outside is not shown.",
                frame.id()
            ));
        }
    }
    out
}
