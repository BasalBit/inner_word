//! The bridge: what the page posts (`docs/whiteboard-design.md`, 4.4).
//!
//! Every message is one JSON string posted to the view's one script message handler,
//! [`crate::BRIDGE_NAME`], with `"v": 1` and a `"kind"`. The app reads it with [`parse`];
//! a message it cannot read is dropped and logged. Nothing a message says is ever run,
//! opened, or taken as a command: the worst one can do is offer a scene for this
//! whiteboard, picture bytes, a shape library, or ask for the list of shortcuts.
//!
//! The scene of a `scene` message and the items of a `library` message stay raw JSON
//! ([`RawValue`]): the main thread reads the head of a message and hands the rest on.

use serde::de::Deserializer;
use serde::{Deserialize, Serialize, Serializer};
use serde_json::value::RawValue;

/// The longest message the view takes (a scene part of 16 MB with room; a picture of 4 MB,
/// Excalidraw's most, is 5.4 MB of base64). The page never posts a longer one.
pub const MAX_MESSAGE_BYTES: usize = 24 * 1024 * 1024;
/// A scene whose JSON is longer than this (UTF-8) comes as [`Message::ScenePart`]s.
pub const PART_BYTES: usize = 16 * 1024 * 1024;
/// The version of the messages (`"v"`).
pub const VERSION: u64 = 1;
/// The start of the one [`Message::Error`] the page posts when it cannot start (`board.json`
/// missing or not a stored scene, Excalidraw failing to mount): it then never posts
/// [`Message::Ready`], and the app shows that the whiteboard stopped working instead of a
/// canvas, rather than waiting for a `ready` that does not come.
pub const START_FAILED: &str = "the whiteboard could not start";

/// One message of the page.
#[derive(Debug)]
pub enum Message {
    /// The scene is on the canvas (after the first load and after every reload): the
    /// version of Excalidraw, how many elements, and the `ContentStamp` of the stored scene
    /// it loaded (the session's base).
    Ready {
        excalidraw: String,
        elements: u64,
        stamp: String,
    },
    /// The drawing changed: the scene as the page has it (`elements`, `appState`, `files`
    /// without `file` and without `dataURL`), and its rev.
    Scene { rev: u64, scene: Box<RawValue> },
    /// One slice of a scene longer than one message: `count` slices of one `rev`, `index`
    /// from 0, whose texts joined are the scene's JSON. A newer `rev` drops an unfinished set.
    ScenePart {
        rev: u64,
        index: u32,
        count: u32,
        text: String,
    },
    /// A picture the app has not had (a paste), before the first scene that names it.
    /// `mime_type` is a hint only: the bytes decide what is stored.
    File {
        id: String,
        mime_type: String,
        data_url: String,
    },
    /// What the page holds that the app may not have yet.
    Status(Status),
    /// The shape library changed: Excalidraw's `LibraryItems` and the library's own counter.
    ///
    /// `too_large`: the library is longer than one message ([`MAX_MESSAGE_BYTES`]), and
    /// `items` is then JSON `null` (which `innerword_board::ShapeLibrary::from_view`
    /// refuses, so it can never be stored as an empty library). The app keeps the library
    /// it stored and says it is too large to keep (4.15), as for one over
    /// `MAX_LIBRARY_BYTES`; the page does not count it as posted, so `collect` brings it
    /// whole at the end.
    Library {
        lrev: u64,
        items: Box<RawValue>,
        too_large: bool,
    },
    /// Excalidraw's `?` or its menu's Keyboard Shortcuts: the app opens its list of shortcuts
    /// at "On a whiteboard" (and nothing else).
    Help,
    /// An error of the page (at most one a second), for standard error.
    Error { message: String },
}

impl Message {
    /// The message as the page writes it (for the app's fake engine, which posts what the
    /// glue would). [`parse`] of it gives the message back.
    pub fn to_body(&self) -> String {
        fn text(value: &str) -> String {
            serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
        }
        match self {
            Message::Ready {
                excalidraw,
                elements,
                stamp,
            } => format!(
                r#"{{"v":1,"kind":"ready","excalidraw":{},"elements":{elements},"stamp":{}}}"#,
                text(excalidraw),
                text(stamp)
            ),
            Message::Scene { rev, scene } => format!(
                r#"{{"v":1,"kind":"scene","rev":{rev},"scene":{}}}"#,
                scene.get()
            ),
            Message::ScenePart {
                rev,
                index,
                count,
                text: part,
            } => format!(
                r#"{{"v":1,"kind":"scenePart","rev":{rev},"index":{index},"count":{count},"text":{}}}"#,
                text(part)
            ),
            Message::File {
                id,
                mime_type,
                data_url,
            } => format!(
                r#"{{"v":1,"kind":"file","id":{},"mimeType":{},"dataURL":{}}}"#,
                text(id),
                text(mime_type),
                text(data_url)
            ),
            Message::Status(status) => {
                let fields = serde_json::to_string(status).unwrap_or_else(|_| "{}".to_string());
                format!(r#"{{"v":1,"kind":"status",{}"#, &fields[1..])
            }
            Message::Library {
                lrev,
                too_large: true,
                ..
            } => {
                format!(r#"{{"v":1,"kind":"library","lrev":{lrev},"items":null,"tooLarge":true}}"#)
            }
            Message::Library {
                lrev,
                items,
                too_large: false,
            } => format!(
                r#"{{"v":1,"kind":"library","lrev":{lrev},"items":{}}}"#,
                items.get()
            ),
            Message::Help => r#"{"v":1,"kind":"help"}"#.to_string(),
            Message::Error { message } => {
                format!(r#"{{"v":1,"kind":"error","message":{}}}"#, text(message))
            }
        }
    }
}

/// What the page holds, as its last `status` said (and `collect` answers).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    /// The rev of the drawing on the canvas.
    pub rev: u64,
    /// The last rev the app was given (a `scene` post or a `collect`).
    pub posted: u64,
    /// A text element is being edited.
    pub editing: bool,
    /// An input method is composing in it.
    pub composing: bool,
    /// The pointer is down on the page (a gesture is in its middle).
    pub pointer_down: bool,
    /// Excalidraw's canvas zoom (1.0 is 100 %).
    pub zoom: f64,
    pub scroll_x: f64,
    pub scroll_y: f64,
    /// Elements on the canvas (not deleted).
    pub elements: u64,
    /// Elements selected.
    pub selected: u64,
    /// The scene is over `MAX_VIEW_SCENE_BYTES`: it is not posted; the app asks for it at the
    /// end, for the recovery file.
    pub too_large: bool,
    /// `libraryWaiting`: the shape library changed since the app was last given it
    /// (Excalidraw's onLibraryChange, or the `library` call): its `library` message is
    /// waiting its 300 ms, or the library is too long for one message (posted as
    /// `tooLarge`, which does not count as given). `collect` brings it in its `library`
    /// field, and the status it answers (and posts) says false from then on. Not part of
    /// [`Status::holds_more`], which is about the drawing: a deliberate end asks `collect`
    /// when either holds (4.6, 4.15).
    pub library_waiting: bool,
}

impl Status {
    /// The page holds something the app has not been given: a newer rev, a text being
    /// edited, or a gesture in its middle. Every deliberate end asks the page first then.
    pub fn holds_more(&self) -> bool {
        self.rev > self.posted || self.editing || self.pointer_down
    }
}

/// `pointer` on the wire.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Pointer {
    Down,
    #[default]
    #[serde(other)]
    Up,
}

/// [`Status`] as the page writes it (camelCase, `pointer: "down" | "up"`). Lenient: a field
/// the page left out is its default, and a number it wrote as `null` (`NaN` in JSON) is 0.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireStatus {
    #[serde(default, deserialize_with = "count")]
    rev: u64,
    #[serde(default, deserialize_with = "count")]
    posted: u64,
    #[serde(default)]
    editing: bool,
    #[serde(default)]
    composing: bool,
    #[serde(default)]
    pointer: Pointer,
    #[serde(default, deserialize_with = "number")]
    zoom: f64,
    #[serde(default, deserialize_with = "number")]
    scroll_x: f64,
    #[serde(default, deserialize_with = "number")]
    scroll_y: f64,
    #[serde(default, deserialize_with = "count")]
    elements: u64,
    #[serde(default, deserialize_with = "count")]
    selected: u64,
    #[serde(default)]
    too_large: bool,
    #[serde(default)]
    library_waiting: bool,
}

fn number<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f64, D::Error> {
    Ok(Option::<f64>::deserialize(deserializer)?
        .filter(|n| n.is_finite())
        .unwrap_or(0.0))
}

fn count<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
    let value = Option::<f64>::deserialize(deserializer)?.unwrap_or(0.0);
    Ok(if value.is_finite() && value > 0.0 {
        value as u64
    } else {
        0
    })
}

impl From<WireStatus> for Status {
    fn from(wire: WireStatus) -> Status {
        Status {
            rev: wire.rev,
            posted: wire.posted,
            editing: wire.editing,
            composing: wire.composing,
            pointer_down: wire.pointer == Pointer::Down,
            zoom: wire.zoom,
            scroll_x: wire.scroll_x,
            scroll_y: wire.scroll_y,
            elements: wire.elements,
            selected: wire.selected,
            too_large: wire.too_large,
            library_waiting: wire.library_waiting,
        }
    }
}

impl From<&Status> for WireStatus {
    fn from(status: &Status) -> WireStatus {
        WireStatus {
            rev: status.rev,
            posted: status.posted,
            editing: status.editing,
            composing: status.composing,
            pointer: if status.pointer_down {
                Pointer::Down
            } else {
                Pointer::Up
            },
            zoom: status.zoom,
            scroll_x: status.scroll_x,
            scroll_y: status.scroll_y,
            elements: status.elements,
            selected: status.selected,
            too_large: status.too_large,
            library_waiting: status.library_waiting,
        }
    }
}

impl<'de> Deserialize<'de> for Status {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Status, D::Error> {
        WireStatus::deserialize(deserializer).map(Status::from)
    }
}

impl Serialize for Status {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        WireStatus::from(self).serialize(serializer)
    }
}

/// Every field a message of any kind has, read in one pass over the body (a scene is
/// skipped over as raw JSON, not parsed).
#[derive(Deserialize)]
struct Head {
    v: Option<u64>,
    kind: Option<String>,
    rev: Option<u64>,
    scene: Option<Box<RawValue>>,
    index: Option<u32>,
    count: Option<u32>,
    text: Option<String>,
    id: Option<String>,
    #[serde(rename = "mimeType")]
    mime_type: Option<String>,
    #[serde(rename = "dataURL")]
    data_url: Option<String>,
    lrev: Option<u64>,
    items: Option<Box<RawValue>>,
    #[serde(rename = "tooLarge")]
    too_large: Option<bool>,
    message: Option<String>,
    excalidraw: Option<String>,
    elements: Option<u64>,
    stamp: Option<String>,
}

/// Reads one message of the page. An error says what is wrong with it, for standard error;
/// the message is then dropped.
pub fn parse(body: &str) -> Result<Message, String> {
    let mended = well_formed(body);
    let body: &str = &mended;
    let head: Head = serde_json::from_str(body)
        .map_err(|error| format!("a message of the whiteboard is not one: {error}"))?;
    match head.v {
        Some(VERSION) => {}
        Some(other) => {
            return Err(format!(
                "a message of the whiteboard has version {other}, this app reads {VERSION}"
            ));
        }
        None => return Err("a message of the whiteboard has no version".to_string()),
    }
    let kind = head
        .kind
        .as_deref()
        .ok_or("a message of the whiteboard has no kind")?;
    let needs = |field: &str| format!("a {kind} message of the whiteboard has no {field}");
    Ok(match kind {
        "ready" => Message::Ready {
            excalidraw: head.excalidraw.unwrap_or_default(),
            elements: head.elements.unwrap_or(0),
            stamp: head.stamp.unwrap_or_default(),
        },
        "scene" => {
            let scene = head.scene.ok_or_else(|| needs("scene"))?;
            if !scene.get().trim_start().starts_with('{') {
                return Err("a scene message of the whiteboard holds no scene".to_string());
            }
            Message::Scene {
                rev: head.rev.ok_or_else(|| needs("rev"))?,
                scene,
            }
        }
        "scenePart" => {
            let index = head.index.ok_or_else(|| needs("index"))?;
            let count = head.count.ok_or_else(|| needs("count"))?;
            if count == 0 || index >= count {
                return Err(format!(
                    "a scenePart message of the whiteboard is part {index} of {count}"
                ));
            }
            Message::ScenePart {
                rev: head.rev.ok_or_else(|| needs("rev"))?,
                index,
                count,
                text: head.text.ok_or_else(|| needs("text"))?,
            }
        }
        "file" => Message::File {
            id: head
                .id
                .filter(|id| !id.is_empty())
                .ok_or_else(|| needs("id"))?,
            mime_type: head.mime_type.unwrap_or_default(),
            data_url: head.data_url.ok_or_else(|| needs("dataURL"))?,
        },
        "status" => Message::Status(serde_json::from_str(body).map_err(|error| {
            format!("a status message of the whiteboard cannot be read: {error}")
        })?),
        "library" => {
            let lrev = head.lrev.ok_or_else(|| needs("lrev"))?;
            if head.too_large == Some(true) {
                Message::Library {
                    lrev,
                    items: RawValue::from_string("null".to_string())
                        .map_err(|error| error.to_string())?,
                    too_large: true,
                }
            } else {
                let items = head.items.ok_or_else(|| needs("items"))?;
                if !items.get().trim_start().starts_with('[') {
                    return Err(
                        "a library message of the whiteboard holds no list of items".to_string()
                    );
                }
                Message::Library {
                    lrev,
                    items,
                    too_large: false,
                }
            }
        }
        "help" => Message::Help,
        "error" => Message::Error {
            message: head.message.unwrap_or_default(),
        },
        other => {
            return Err(format!(
                "a message of the whiteboard of an unknown kind: {other}"
            ));
        }
    })
}

/// The text with every lone surrogate escape (`\ud800` without its pair) made the escape of U+FFFD, as
/// the glue does before it posts: serde_json refuses one in a string. An escaped backslash
/// is passed over whole, so `\\ud800` (a backslash and the letters) stays.
pub(crate) fn well_formed(text: &str) -> std::borrow::Cow<'_, str> {
    if !text.contains("\\u") && !text.contains("\\U") {
        return std::borrow::Cow::Borrowed(text);
    }
    let bytes = text.as_bytes();
    let mut out: Option<String> = None;
    let mut copied = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'\\' || i + 1 >= bytes.len() {
            i += 1;
            continue;
        }
        if bytes[i + 1] != b'u' {
            i += 2;
            continue;
        }
        let unit = |at: usize| -> Option<u16> {
            let hex = text.get(at + 2..at + 6)?;
            u16::from_str_radix(hex, 16)
                .ok()
                .filter(|_| hex.bytes().all(|b| b.is_ascii_hexdigit()))
        };
        let Some(first) = unit(i) else {
            i += 2;
            continue;
        };
        let high = (0xd800..=0xdbff).contains(&first);
        let low = (0xdc00..=0xdfff).contains(&first);
        let paired = high
            && bytes.get(i + 6) == Some(&b'\\')
            && bytes.get(i + 7) == Some(&b'u')
            && unit(i + 6).is_some_and(|second| (0xdc00..=0xdfff).contains(&second));
        if paired {
            i += 12;
            continue;
        }
        if high || low {
            let out = out.get_or_insert_with(|| String::with_capacity(text.len()));
            out.push_str(&text[copied..i]);
            out.push_str("\\ufffd");
            copied = i + 6;
        }
        i += 6;
    }
    match out {
        Some(mut out) => {
            out.push_str(&text[copied..]);
            std::borrow::Cow::Owned(out)
        }
        None => std::borrow::Cow::Borrowed(text),
    }
}
