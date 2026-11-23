//! What the app asks the page (`docs/whiteboard-design.md`, 4.5 and section 5).
//!
//! Each constant is a body for `innerword_web::WebPage::call_async` (the live view) or
//! `ScriptInput::script` (the export page): an async function body that WebKit runs with
//! one argument, `arg`, the JSON text of the call's argument, and whose promise it awaits.
//! Every body answers JSON text; [`answer`] reads it into the types below. The page runs its
//! calls one after another, in the order the app made them.
//!
//! The calls of the live view (`index.html`) wait until the page posted `ready`, except
//! [`STATE`], which is answered at once, outside the order of the others (it reads and
//! changes nothing, and must answer when the page could not start). [`EXPORT_LAYOUT`] and
//! [`CONVERT_SKELETONS`] are the export page's (`export.html`).

use serde::Deserialize;
use serde::de::{self, Deserializer};
use serde_json::json;
use serde_json::value::RawValue;

macro_rules! body {
    ($name:literal) => {
        concat!(
            "return await window.innerwordBoard.call(\"",
            $name,
            "\", arg);"
        )
    };
}

/// `{}` -> [`Collected`]: everything the app has not been posted yet. Never waits on an
/// animation frame or a timer (a parked view answers it).
pub const COLLECT: &str = body!("collect");
/// [`files_argument`] -> [`FilesPosted`]: a `file` message posted for each id the canvas has.
pub const FILES: &str = body!("files");
/// A delta (`innerword_board::Delta::to_json`) -> [`Adopted`]: an outside write merged by the
/// base rule.
pub const ADOPT: &str = body!("adopt");
/// [`steps_argument`] -> [`Revised`]: Excalidraw's undo (of the text being edited, else of
/// the drawing).
pub const UNDO: &str = body!("undo");
/// [`steps_argument`] -> [`Revised`].
pub const REDO: &str = body!("redo");
/// `{}` -> `{}`: select all (the text being edited, else the drawing).
pub const SELECT_ALL: &str = body!("selectAll");
/// [`zoom_argument`] -> [`Zoomed`]: Excalidraw's canvas zoom, never the web view's.
pub const ZOOM: &str = body!("zoom");
/// [`search_argument`] -> `{}`: Excalidraw's own search, with the query typed in.
pub const SEARCH: &str = body!("search");
/// `{}` -> [`Center`]: the middle of the view in scene coordinates, and the selection.
pub const CENTER: &str = body!("center");
/// [`insert_pictures_argument`] -> [`Inserted`]: pictures the app stored, on the canvas.
pub const INSERT_PICTURES: &str = body!("insertPictures");
/// [`insert_elements_argument`] -> [`Inserted`]: elements `innerword_board` made (the Add
/// commands, the scripted `board:draw:`), one step of the undo, selected.
pub const INSERT_ELEMENTS: &str = body!("insertElements");
/// `{}` -> the page's [`crate::bridge::Status`] (the scripted `state` line). Answered at
/// once, also before `ready` and behind a call that waits for it.
pub const STATE: &str = body!("state");
/// [`mermaid_argument`] -> `{}` (the dialog opened) or [`Inserted`] (a definition converted).
pub const MERMAID: &str = body!("mermaid");
/// [`library_argument`] -> [`LibraryAnswered`]: the shape library's commands.
pub const LIBRARY: &str = body!("library");
/// [`keys_argument`] -> `{}`: the whiteboard's own keys the app gave up to commands of the
/// user's, all of them: from then on no tooltip or menu item of Excalidraw's names one, and
/// one left out is named again. The view is told them first in `board.json`
/// (`keysGivenUp`, [`crate::BOARD_PATH`]); the app calls this when its keymap is installed
/// again while the whiteboard is open.
pub const KEYS: &str = body!("keys");
/// The export page: [`export_argument`] -> [`ExportSize`]. The page then shows the SVG of the
/// stored scene alone at (0, 0); its PDF is the rect (0, 0, ceil(width), ceil(height)).
pub const EXPORT_LAYOUT: &str = body!("exportLayout");
/// The export page: [`convert_skeletons_argument`] -> `{"elements": [...]}`, Excalidraw's own
/// `convertToExcalidrawElements` then `restoreElements(…, null, {refreshDimensions,
/// repairBindings})`. For the converter's cross-check only (B11).
pub const CONVERT_SKELETONS: &str = body!("convertSkeletons");

/// The margin of a whiteboard's PDF around its drawing, in points.
pub const EXPORT_PADDING: f64 = 32.0;
/// The longest side of a whiteboard's PDF page, in points (Acrobat's limit): a larger
/// drawing is scaled down whole.
pub const EXPORT_MAX_SIDE: f64 = 14_400.0;

/// `{"padding":32,"maxSide":14400}`.
pub fn export_argument() -> String {
    json!({ "padding": EXPORT_PADDING as u64, "maxSide": EXPORT_MAX_SIDE as u64 }).to_string()
}

/// `{"ids": [...]}` (Excalidraw file ids).
pub fn files_argument(ids: &[String]) -> String {
    json!({ "ids": ids }).to_string()
}

/// `{"steps": n}`.
pub fn steps_argument(steps: u32) -> String {
    json!({ "steps": steps }).to_string()
}

/// A change of the canvas zoom.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Zoom {
    /// Excalidraw's own step in (+10 points).
    In,
    /// Excalidraw's own step out.
    Out,
    /// A zoom (1.0 is 100 %), kept between 10 % and 3,000 %.
    To(f64),
    /// The whole drawing in view (as Excalidraw's `shift-1`).
    Fit,
}

/// `{"in": true}`, `{"out": true}`, `{"to": 1.25}` or `{"fit": true}`.
pub fn zoom_argument(zoom: Zoom) -> String {
    match zoom {
        Zoom::In => json!({ "in": true }),
        Zoom::Out => json!({ "out": true }),
        Zoom::To(value) => json!({ "to": value }),
        Zoom::Fit => json!({ "fit": true }),
    }
    .to_string()
}

/// `{"query": "…"}`, or `{}` to open the search as it is.
pub fn search_argument(query: Option<&str>) -> String {
    match query {
        Some(query) => json!({ "query": query }),
        None => json!({}),
    }
    .to_string()
}

/// A picture the app stored, to be put on the canvas by [`INSERT_PICTURES`].
#[derive(Clone, Debug, PartialEq)]
pub struct PictureToInsert {
    /// The `FileId` hex of the stored bytes, which is also its Excalidraw file id.
    pub file: String,
    pub mime_type: String,
    /// `data:<type>;base64,…` of the stored bytes.
    pub data_url: String,
    /// The size it is drawn at, in scene units (its own size up to 1,440 on its longer side).
    pub width: f64,
    pub height: f64,
}

/// `{"pictures": [...], "at": {"x","y"} or null}`: `at` is a point of the view in CSS
/// pixels from its top left (a drop); without it the pictures go to the middle of the view.
pub fn insert_pictures_argument(pictures: &[PictureToInsert], at: Option<(f64, f64)>) -> String {
    let pictures: Vec<_> = pictures
        .iter()
        .map(|p| json!({ "file": p.file, "mimeType": p.mime_type, "dataURL": p.data_url, "width": p.width, "height": p.height }))
        .collect();
    json!({ "pictures": pictures, "at": at.map(|(x, y)| json!({ "x": x, "y": y })) }).to_string()
}

/// `{"elements": <elements_json>, "select": select, "editText": "<id>" or null}`:
/// `elements_json` is a JSON array of Excalidraw elements (an element whose id the canvas
/// has replaces its copy; the others are added on top).
pub fn insert_elements_argument(
    elements_json: &str,
    select: bool,
    edit_text: Option<&str>,
) -> String {
    format!(
        r#"{{"elements":{},"select":{select},"editText":{}}}"#,
        elements_json,
        serde_json::to_string(&edit_text).unwrap_or_else(|_| "null".to_string())
    )
}

/// `{}` (open Excalidraw's Mermaid dialog) or `{"insert": "<definition>"}`.
pub fn mermaid_argument(insert: Option<&str>) -> String {
    match insert {
        Some(definition) => json!({ "insert": definition }),
        None => json!({}),
    }
    .to_string()
}

/// What the shape library's call does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LibraryAction {
    Show,
    Hide,
    Toggle,
    /// The selection as a new item, as Excalidraw's "Add to library" makes it.
    AddSelection,
    /// A copy of the item with this id at the middle of the view, selected.
    Insert(String),
}

/// `{"show": true}`, `{"hide": true}`, `{"toggle": true}`, `{"addSelection": true}` or
/// `{"insert": "<item id>"}`.
pub fn library_argument(action: &LibraryAction) -> String {
    match action {
        LibraryAction::Show => json!({ "show": true }),
        LibraryAction::Hide => json!({ "hide": true }),
        LibraryAction::Toggle => json!({ "toggle": true }),
        LibraryAction::AddSelection => json!({ "addSelection": true }),
        LibraryAction::Insert(id) => json!({ "insert": id }),
    }
    .to_string()
}

/// `{"givenUp": ["cmd-k", ...]}`: keys of [`crate::keys::WHITEBOARD_KEYS`] in
/// `innerword_keymap`'s normal form, as they are written there.
pub fn keys_argument(given_up: &[&str]) -> String {
    json!({ "givenUp": given_up }).to_string()
}

/// `{"skeletons": <skeletons_json>}` (a JSON array).
pub fn convert_skeletons_argument(skeletons_json: &str) -> String {
    format!(r#"{{"skeletons":{skeletons_json}}}"#)
}

/// Reads the answer of a call.
pub fn answer<'a, T: Deserialize<'a>>(text: &'a str) -> Result<T, String> {
    serde_json::from_str(text)
        .map_err(|error| format!("the whiteboard's answer cannot be read: {error}"))
}

/// The scene of a [`COLLECT`] answer.
#[derive(Debug)]
pub enum CollectedScene {
    /// Nothing the app has not been given (`null`).
    None,
    /// The scene (`{…}`), as a `scene` message holds it.
    Whole(Box<RawValue>),
    /// `"parts"`: longer than one message, posted as `scenePart`s before the answer; the app
    /// waits for all of them.
    Parts,
}

impl<'de> Deserialize<'de> for CollectedScene {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<CollectedScene, D::Error> {
        let raw = Box::<RawValue>::deserialize(deserializer)?;
        let text = raw.get().trim();
        if text == "null" {
            Ok(CollectedScene::None)
        } else if text == "\"parts\"" {
            Ok(CollectedScene::Parts)
        } else if text.starts_with('{') {
            Ok(CollectedScene::Whole(raw))
        } else {
            Err(de::Error::custom(
                "a collected scene is null, \"parts\" or an object",
            ))
        }
    }
}

/// The answer of [`COLLECT`].
#[derive(Debug, Deserialize)]
pub struct Collected {
    /// The rev of the canvas (and of the scene, when there is one).
    pub rev: u64,
    pub scene: CollectedScene,
    /// Pictures the scene names that the app has not had.
    pub files: Vec<CollectedFile>,
    /// The shape library, when it changed since its last post.
    pub library: Option<CollectedLibrary>,
    pub status: crate::bridge::Status,
}

/// A picture in a [`Collected`].
#[derive(Debug, Deserialize)]
pub struct CollectedFile {
    pub id: String,
    #[serde(rename = "mimeType")]
    pub mime_type: String,
    #[serde(rename = "dataURL")]
    pub data_url: String,
}

/// The shape library in a [`Collected`]: Excalidraw's `LibraryItems`, raw.
#[derive(Debug, Deserialize)]
pub struct CollectedLibrary {
    pub lrev: u64,
    pub items: Box<RawValue>,
}

/// The answer of [`LIBRARY`].
#[derive(Debug, Deserialize)]
pub struct LibraryAnswered {
    pub lrev: u64,
    /// The id of the item an `addSelection` made.
    pub added: Option<String>,
    /// Why nothing was done, in Excalidraw's words where it has some (a selection with a
    /// picture), for a toast.
    pub refused: Option<String>,
    /// The elements an `insert` put on the canvas.
    #[serde(default)]
    pub ids: Vec<String>,
}

/// The answer of [`FILES`].
#[derive(Debug, Deserialize)]
pub struct FilesPosted {
    pub posted: Vec<String>,
    pub missing: Vec<String>,
}

/// The answer of [`ADOPT`].
#[derive(Debug, Deserialize)]
pub struct Adopted {
    /// The rev of the merged canvas.
    pub rev: u64,
    /// The canvas holds exactly the outside write: no conflict, every element of the delta
    /// at the version the delta gave it, and the canvas was at the delta's `baseRev` itself
    /// when it was merged (nothing of the writer's since the scene the change was made on,
    /// posted or not: a post that left the page after the app wrote is one the app drops
    /// while it adopts). Then this rev counts as saved. Otherwise the page posts the merge
    /// as a change of the writer's, at this rev or above (a new rev when this one was
    /// posted before), once no gesture is under way.
    pub same: bool,
    /// The ids of the incoming elements that were not taken (the writer changed them).
    pub conflicts: Vec<String>,
}

/// The answer of [`CENTER`].
#[derive(Debug, Deserialize)]
pub struct Center {
    pub x: f64,
    pub y: f64,
    /// The selected elements, in the order they were selected.
    #[serde(default)]
    pub selected: Vec<String>,
}

/// The answer of [`UNDO`] and [`REDO`].
#[derive(Debug, Deserialize)]
pub struct Revised {
    pub rev: u64,
}

/// The answer of [`ZOOM`].
#[derive(Debug, Deserialize)]
pub struct Zoomed {
    pub zoom: f64,
}

/// The answer of [`INSERT_PICTURES`], [`INSERT_ELEMENTS`] and of [`MERMAID`] with `insert`.
#[derive(Debug, Deserialize)]
pub struct Inserted {
    pub ids: Vec<String>,
}

/// The answer of [`EXPORT_LAYOUT`]: the size of the SVG the page shows, in CSS pixels
/// (which a PDF makes points).
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
pub struct ExportSize {
    pub width: f64,
    pub height: f64,
}
