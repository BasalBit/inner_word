//! Whiteboards, as the app sees them (`docs/whiteboard-design.md`): where the
//! files of a whiteboard's view come from ([`LibraryBoardSource`]: the bundle
//! of Excalidraw, `board.json`, `library.json` and the pictures), the host of
//! its origin ([`board_host`]), what a picture the page posts is made into
//! before it is stored ([`prepare_picture`]), the keys GPUI is kept off on a
//! whiteboard unless the user gave one of them to a command
//! ([`whiteboard_keys`]), and the fake engine's canvas for the
//! app's tests ([`FakeBoard`], driven through [`crate::pages::FakePages`]).
//!
//! The session of an open whiteboard, its autosave, its calls and its
//! commands are the workspace's (`workspace/board.rs`). WebKit is
//! `innerword-web`'s, the scene in Rust `innerword-board`'s, the bundle and
//! its contract `innerword-board-web`'s.
//!
//! - **One engine for pages and whiteboards**: a whiteboard is shown by the
//!   engine that shows pages ([`crate::pages::engine`]); without one it shows
//!   its listing of elements, and there is no New Whiteboard.
//! - **The source answers what the library holds now**: `board.json` is
//!   read from a cell the session shares with the source ([`BoardCell`]),
//!   which the session replaces after every save and every outside write it
//!   takes over, so a load at any time (the first, a reload after a crash,
//!   Reload whiteboard) starts from the stored scene.
//! - **The bridge's messages only post**: the view's `message` callback
//!   parses the body (`innerword_board_web::bridge::parse`) and posts it to
//!   the view's inbox; nothing a message says is run.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::Path,
    rc::Rc,
    sync::{Arc, Mutex},
};

use gpui::{App, Global, KeyBinding};
use innerword_board::{Scene, ShapeLibrary};
use innerword_board_web::{
    Assets,
    bridge::{Message, Status},
    calls,
};
use innerword_library::{
    DocumentId, Library,
    store::{FileId, ImageFile},
};
use innerword_web::{PageSource, ServedFile};
use serde_json::{Map, Value, json};

/// The key context of the element of an open whiteboard.
pub const BOARD_VIEW: &str = "BoardView";

/// The setting of the library that keeps Excalidraw's shape library, one for
/// every whiteboard (section 4.15).
pub const SHAPE_LIBRARY_SETTING: &str = "board.shape_library";

/// What a run without a page engine says over a whiteboard.
pub const NOT_SHOWN_IN_THIS_RUN: &str = "Whiteboards are not shown in this run.";

/// How long a deliberate end of a whiteboard's session waits for the page to
/// hand over what it has not posted yet (`collect`).
pub const VIEW_COLLECT_PATIENCE: std::time::Duration = std::time::Duration::from_secs(1);

/// How long the view has to answer an `adopt` before the app merges in Rust
/// and reloads it.
pub const ADOPT_PATIENCE: std::time::Duration = std::time::Duration::from_secs(3);

/// Answers of background saves of whiteboards held back (tests: a save that
/// wrote and whose answer has not reached the main thread yet).
#[derive(Default)]
struct HeldSaveAnswers {
    held: bool,
    waiting: Vec<crate::pages::AnswerTx<()>>,
}

impl Global for HeldSaveAnswers {}

/// From now on the answer of a background save of a whiteboard waits, after
/// its write, until [`release_board_saves`] (tests).
pub fn hold_board_saves(cx: &mut App) {
    cx.default_global::<HeldSaveAnswers>().held = true;
}

/// Lets the answers held by [`hold_board_saves`] through, and every later one.
pub fn release_board_saves(cx: &mut App) {
    let held = cx.default_global::<HeldSaveAnswers>();
    held.held = false;
    for waiter in held.waiting.drain(..) {
        waiter.send(());
    }
}

/// Whether the saves of whiteboards fail (tests: the library busy past its
/// timeout, or the disk full, without waiting five seconds for a lock).
#[derive(Default)]
struct FailingSaves(bool);

impl Global for FailingSaves {}

/// From now on every save of a whiteboard fails as a store error does, until
/// called again with `false` (tests).
pub fn fail_board_saves(cx: &mut App, fail: bool) {
    cx.default_global::<FailingSaves>().0 = fail;
}

/// Whether [`fail_board_saves`] is on.
pub(crate) fn board_saves_fail(cx: &App) -> bool {
    cx.try_global::<FailingSaves>()
        .is_some_and(|failing| failing.0)
}

/// The message of a save [`fail_board_saves`] fails.
pub const FAILED_ON_PURPOSE: &str = "the library is busy";

/// What a background save of a whiteboard waits for before its answer is
/// taken: nothing, unless a test holds the answers.
pub(crate) fn board_save_gate(cx: &mut App) -> Option<crate::pages::Answer<()>> {
    let held = cx.try_global::<HeldSaveAnswers>()?;
    if !held.held {
        return None;
    }
    let (tx, rx) = crate::pages::answer();
    cx.global_mut::<HeldSaveAnswers>().waiting.push(tx);
    Some(rx)
}

/// The sentence of pages' engine (`innerword_web::available`, or that this
/// run shows none) worded for whiteboards: "Pages need macOS 12.3 or later."
/// is "Whiteboards need macOS 12.3 or later.", "This macOS does not have
/// what pages need to stay offline (…)" is "… what whiteboards need …".
pub fn whiteboards_sentence(pages: &str) -> String {
    if pages == crate::pages::NOT_SHOWN_IN_THIS_RUN {
        return NOT_SHOWN_IN_THIS_RUN.to_string();
    }
    pages
        .replacen("Pages need", "Whiteboards need", 1)
        .replacen(
            "what pages need to stay offline",
            "what whiteboards need",
            1,
        )
        .replacen("what pages need", "what whiteboards need", 1)
}

/// The host of a whiteboard's origin, `innerword-page://<host>/`: its id and
/// the moment its row was made (`board-12-1731600000000`), never a page's
/// origin (`page-…`), for the reasons [`crate::pages::page_host`] gives.
pub fn board_host(id: DocumentId, created_at: i64) -> String {
    format!("board-{}-{created_at}", id.0)
}

/// What a whiteboard without a page engine says above its listing: the
/// sentence of `innerword_web::available`, worded for whiteboards, or that
/// this run shows none.
pub fn not_shown_sentence(cx: &App) -> String {
    whiteboards_sentence(&crate::pages::not_shown_sentence(cx))
}

// ----- the bundle -----------------------------------------------------------------

/// The bundle of Excalidraw, looked for once per run.
struct Bundle(Result<Arc<Assets>, String>);
impl Global for Bundle {}

/// The bundle of the whiteboard's page (`innerword_board_web::Assets::find`:
/// `Contents/Resources/Whiteboard` in the app bundle, the crate's `dist/`
/// otherwise), found once. `Err` says why there is none.
pub fn assets(cx: &mut App) -> Result<Arc<Assets>, String> {
    if !cx.has_global::<Bundle>() {
        let found = Assets::find().map(Arc::new);
        cx.set_global(Bundle(found));
    }
    cx.global::<Bundle>().0.clone()
}

// ----- what the view's source serves -------------------------------------------------

/// What the source of an open whiteboard's view serves that changes while
/// it is open: the stored scene and its stamp, where the writer last looked,
/// the stored shape library, and the pictures it may serve. Shared between
/// the session and the source (the scheme handler reads it on a background
/// queue).
#[derive(Clone, Debug, Default)]
pub struct BoardCell {
    /// The `ContentStamp` of `scene`, as 32 hex digits.
    pub stamp: String,
    /// The stored scene (the row's content).
    pub scene: Arc<str>,
    /// The row's `view_state`, or `{}`.
    pub view: Arc<str>,
    /// The stored shape library (`{}` when there is none).
    pub library: Arc<str>,
    /// The `FileId`s (hex) the source answers under `_files/`.
    pub files: HashSet<String>,
    /// The whiteboard's own keys the user gave a command
    /// ([`WhiteboardKeys::given_up`]), which no tooltip of Excalidraw's names.
    pub keys_given_up: Vec<&'static str>,
}

/// A [`BoardCell`] shared with a view's source.
pub type SharedCell = Arc<Mutex<BoardCell>>;

impl BoardCell {
    /// `board.json`: `{"stamp", "scene", "view", "keysGivenUp"}`, one
    /// answer so that its parts belong together.
    pub fn board_json(&self) -> String {
        format!(
            r#"{{"stamp":{},"scene":{},"view":{},"keysGivenUp":{}}}"#,
            serde_json::to_string(&self.stamp).unwrap_or_else(|_| "\"\"".to_string()),
            raw_or(&self.scene, "null"),
            raw_or(&self.view, "{}"),
            serde_json::to_string(&self.keys_given_up).unwrap_or_else(|_| "[]".to_string())
        )
    }
}

/// `text` when it is JSON, else `fallback` (a stored value that cannot be
/// read is never pasted into an answer).
fn raw_or<'a>(text: &'a str, fallback: &'a str) -> &'a str {
    if serde_json::from_str::<serde::de::IgnoredAny>(text).is_ok() {
        text
    } else {
        fallback
    }
}

/// Where the pictures of a whiteboard are read from.
enum Pictures {
    /// `images.db` beside the library, through a read-only connection of the
    /// source's own (WAL lets it read while another connection writes).
    Disk(Mutex<ImageFile>),
    /// A library in memory (tests): every stored file the source may serve,
    /// read through the library's store when it is asked first.
    Memory(Mutex<HashMap<String, Option<Arc<[u8]>>>>),
}

/// The files of an open whiteboard's view, or of its export page.
pub struct LibraryBoardSource {
    assets: Arc<Assets>,
    cell: SharedCell,
    pictures: Pictures,
    /// The bundle's files read so far (about 1.5 MB at start).
    cache: Mutex<HashMap<String, Arc<[u8]>>>,
}

impl LibraryBoardSource {
    /// The source of a whiteboard of `library` whose changing parts are in
    /// `cell`.
    pub fn new(assets: Arc<Assets>, library: &Library, cell: SharedCell) -> LibraryBoardSource {
        let pictures = match library.path() {
            Some(database) => Pictures::Disk(Mutex::new(ImageFile::read_only(database))),
            None => {
                // Read what the cell names now, through the store.
                let names: Vec<String> = cell
                    .lock()
                    .map(|cell| cell.files.iter().cloned().collect())
                    .unwrap_or_default();
                let read: HashMap<String, Option<Arc<[u8]>>> = names
                    .into_iter()
                    .map(|hex| {
                        let bytes = FileId::parse(&hex)
                            .and_then(|id| library.store().file_bytes(&id).ok().flatten())
                            .map(Arc::from);
                        (hex, bytes)
                    })
                    .collect();
                Pictures::Memory(Mutex::new(read))
            }
        };
        LibraryBoardSource {
            assets,
            cell,
            pictures,
            cache: Mutex::default(),
        }
    }

    /// Hands a library in memory's files to the source as they are stored
    /// later (tests that serve pictures stored after the view was made).
    pub fn remember(&self, hex: &str, bytes: &[u8]) {
        if let Pictures::Memory(read) = &self.pictures
            && let Ok(mut read) = read.lock()
        {
            read.insert(hex.to_string(), Some(Arc::from(bytes)));
        }
    }

    fn picture(&self, hex: &str) -> Option<ServedFile> {
        let allowed = self.cell.lock().ok()?.files.contains(hex);
        if !allowed {
            return None;
        }
        let bytes: Arc<[u8]> = match &self.pictures {
            Pictures::Disk(file) => {
                let id = FileId::parse(hex)?;
                Arc::from(file.lock().ok()?.file_bytes(&id).ok()??)
            }
            Pictures::Memory(read) => read.lock().ok()?.get(hex).cloned()??,
        };
        Some(ServedFile {
            media_type: picture_type(&bytes).to_string(),
            bytes,
        })
    }

    fn bundle_file(&self, path: &str) -> Option<ServedFile> {
        let info = self.assets.get(path)?;
        let media_type = info.media_type.clone();
        if let Some(bytes) = self.cache.lock().ok()?.get(path) {
            return Some(ServedFile {
                media_type,
                bytes: bytes.clone(),
            });
        }
        let bytes: Arc<[u8]> = Arc::from(self.assets.read(path)?.ok()?);
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(path.to_string(), bytes.clone());
        }
        Some(ServedFile { media_type, bytes })
    }
}

impl PageSource for LibraryBoardSource {
    fn file(&self, path: &str) -> Option<ServedFile> {
        let path = path.trim_start_matches('/');
        let json = |text: String| ServedFile {
            media_type: "application/json".to_string(),
            bytes: Arc::from(text.into_bytes()),
        };
        if path == innerword_board_web::BOARD_PATH {
            return Some(json(self.cell.lock().ok()?.board_json()));
        }
        if path == innerword_board_web::LIBRARY_PATH {
            let library = self.cell.lock().ok()?.library.clone();
            return Some(json(raw_or(&library, "{}").to_string()));
        }
        if path == innerword_board_web::SCENE_PATH {
            let scene = self.cell.lock().ok()?.scene.clone();
            return Some(json(scene.to_string()));
        }
        if let Some(hex) = path.strip_prefix(innerword_board_web::FILES_PREFIX) {
            return self.picture(hex);
        }
        self.bundle_file(path)
    }
}

/// The type a stored picture is served and recorded with: the type of its
/// bytes (`image/svg+xml` for what `sniff_svg` takes, else PNG, JPEG or
/// GIF by their signature), never `text/html`.
pub fn picture_type(bytes: &[u8]) -> &'static str {
    if innerword_board::sniff_svg(bytes) {
        return "image/svg+xml";
    }
    match innerword_page::sniffed_type(bytes) {
        Some(kind) if kind.starts_with("image/") => kind,
        _ => "application/octet-stream",
    }
}

// ----- pictures -----------------------------------------------------------------------

/// A picture made into what the library stores (section 2, "Pictures").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedPicture {
    /// The `FileId` of `bytes`, as hex.
    pub file: String,
    /// The type of `bytes`.
    pub mime_type: String,
    pub bytes: Vec<u8>,
    /// Width and height in pixels (`None` for an SVG).
    pub pixels: Option<(u32, u32)>,
    /// Pixels per inch (72 when the file says nothing, and for an SVG).
    pub density: u32,
}

/// What the library stores of `bytes`: an SVG as it came, anything else
/// through `innerword_image::prepare` (upright, at most 4096 pixels and 16 MB,
/// no GPS position). `Err` is the sentence of a refusal about `subject`.
pub fn prepare_picture(bytes: &[u8], subject: &str) -> Result<PreparedPicture, String> {
    if innerword_board::sniff_svg(bytes) {
        return Ok(PreparedPicture {
            file: FileId::of(bytes).to_hex(),
            mime_type: "image/svg+xml".to_string(),
            bytes: bytes.to_vec(),
            pixels: None,
            density: 72,
        });
    }
    let prepared = innerword_image::prepare(bytes).map_err(|error| error.sentence(subject))?;
    let mime_type = picture_type(&prepared.bytes).to_string();
    Ok(PreparedPicture {
        file: FileId::of(&prepared.bytes).to_hex(),
        mime_type,
        pixels: Some(prepared.pixels),
        density: prepared.density,
        bytes: prepared.bytes,
    })
}

/// The bytes of the picture file at `path`, read only when it is a plain
/// file no larger than `innerword_image` takes (a named pipe would wait for
/// a writer, a film would be read whole only to be refused). `Err` is the
/// sentence about `subject`.
pub fn read_picture_file(path: &Path, subject: &str) -> Result<Vec<u8>, String> {
    use std::io::Read as _;
    let limit = innerword_image::MAX_INPUT_BYTES;
    let too_large =
        || innerword_image::PrepareError::TooLarge { animated: false }.sentence(subject);
    let unreadable = |error: std::io::Error| format!("{subject} could not be read ({error}).");
    // Asked before the file is opened: opening a named pipe waits.
    let metadata = std::fs::metadata(path).map_err(unreadable)?;
    if !metadata.is_file() {
        return Err(innerword_image::PrepareError::NotAnImage.sentence(subject));
    }
    if metadata.len() > limit {
        return Err(too_large());
    }
    let file = std::fs::File::open(path).map_err(unreadable)?;
    if !file.metadata().map_err(unreadable)?.is_file() {
        return Err(innerword_image::PrepareError::NotAnImage.sentence(subject));
    }
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(unreadable)?;
    if bytes.len() as u64 > limit {
        return Err(too_large());
    }
    Ok(bytes)
}

/// The bytes of a `data:` URL (base64, or percent-encoded text such as an
/// SVG Mermaid makes); `None` for anything else.
pub fn data_url_bytes(url: &str) -> Option<Vec<u8>> {
    use base64::Engine as _;
    let rest = url.strip_prefix("data:")?;
    let (head, data) = rest.split_once(',')?;
    if head.ends_with(";base64") {
        let data: String = data.chars().filter(|ch| !ch.is_whitespace()).collect();
        base64::engine::general_purpose::STANDARD
            .decode(data.as_bytes())
            .ok()
    } else {
        Some(percent_decode(data))
    }
}

/// `%XX` as the byte `XX`; anything else (a `%` not followed by two hex
/// digits, a character of several bytes) as it is. Read by bytes: the text
/// comes from a page and a `%` may stand before any character.
fn percent_decode(text: &str) -> Vec<u8> {
    fn hex(byte: u8) -> Option<u8> {
        char::from(byte).to_digit(16).map(|digit| digit as u8)
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut ix = 0;
    while ix < bytes.len() {
        if bytes[ix] == b'%'
            && let (Some(high), Some(low)) = (
                bytes.get(ix + 1).copied().and_then(hex),
                bytes.get(ix + 2).copied().and_then(hex),
            )
        {
            out.push(high << 4 | low);
            ix += 3;
            continue;
        }
        out.push(bytes[ix]);
        ix += 1;
    }
    out
}

/// A `data:` URL of `bytes` of type `mime_type`.
pub fn data_url_of(mime_type: &str, bytes: &[u8]) -> String {
    use base64::Engine as _;
    format!(
        "data:{mime_type};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

/// The size a picture of `pixels` at `density` is put on a canvas at: its
/// own size in points, up to 1,440 on its longer side (Excalidraw's own
/// limit for a picture it is given).
pub fn drawn_size(pixels: Option<(u32, u32)>, density: u32) -> (f64, f64) {
    let (width, height) = match pixels {
        Some((width, height)) => {
            let scale = 72.0 / f64::from(density.max(1));
            (f64::from(width) * scale, f64::from(height) * scale)
        }
        None => (400.0, 300.0),
    };
    let longer = width.max(height).max(1.0);
    let limit = innerword_board::limits::PICTURE_MAX_SIDE;
    if longer > limit {
        let scale = limit / longer;
        (width * scale, height * scale)
    } else {
        (width.max(1.0), height.max(1.0))
    }
}

// ----- the whiteboard's own keys --------------------------------------------------------

/// The whiteboard's own keys as the installed keymap leaves them: each one
/// **held** for Excalidraw, or **given up** to a command the user gave that
/// key where a whiteboard has the keyboard. [`whiteboard_keys`] is the one
/// place that decides it; the keymap keeps what it decided at every install
/// (`crate::keymap::whiteboard_keys`), and the four places that need it ask
/// that: the fixed bindings ([`WhiteboardKeys::fixed_bindings`]), the menu
/// bar while a whiteboard is open (`menus::menu_key_bindings`), the fixed
/// rows "On a whiteboard" (`shortcuts`) and, through the same keys, the
/// question about a key (`workspace/shortcuts.rs`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WhiteboardKeys {
    keys: Vec<WhiteboardKey>,
}

/// One of the whiteboard's own keys, and who has it on a whiteboard.
#[derive(Clone, Debug, PartialEq)]
pub struct WhiteboardKey {
    /// The key and what Excalidraw does with it.
    pub key: innerword_board_web::keys::BoardKey,
    /// `None`: held, Excalidraw's on a whiteboard. Otherwise the command
    /// the user gave this key, which runs on a whiteboard in its place.
    pub given_to: Option<innerword_keymap::Target>,
}

impl WhiteboardKeys {
    /// Every key held: what a keymap without the user's changes gives.
    pub fn all_held() -> Self {
        Self {
            keys: innerword_board_web::keys::WHITEBOARD_KEYS
                .iter()
                .map(|key| WhiteboardKey {
                    key: *key,
                    given_to: None,
                })
                .collect(),
        }
    }

    /// Every key with who has it, in the order of `WHITEBOARD_KEYS`.
    pub fn keys(&self) -> &[WhiteboardKey] {
        &self.keys
    }

    /// The fixed bindings that keep GPUI off the keys that are held: each,
    /// in every spelling the keymap gives it, bound to `gpui::NoAction` in
    /// `BoardView`. Not defaults, not rows of the list's commands, not
    /// entries of the shortcuts file: `crate::keymap` appends them after
    /// every other binding each time it installs the keymap, so that among
    /// the bindings at the board's depth they come last and win (a
    /// `NoAction` that wins disables every binding under it). GPUI then
    /// leaves such a chord to the web view, which hands it to Excalidraw. A
    /// key given up has none, so the user's command runs on it there.
    pub fn fixed_bindings(&self) -> Vec<KeyBinding> {
        let mut bindings = Vec::new();
        for key in self.keys.iter().filter(|key| key.given_to.is_none()) {
            for spelling in key_spellings(key.key.keys) {
                if let Ok(binding) = std::panic::catch_unwind(|| {
                    KeyBinding::new(&spelling, gpui::NoAction, Some(BOARD_VIEW))
                }) {
                    bindings.push(binding);
                }
            }
        }
        bindings
    }

    /// The entry of `keys` (one keystroke in any spelling or order of its
    /// modifiers), when it is one of the whiteboard's own keys.
    pub fn find(&self, keys: &str) -> Option<&WhiteboardKey> {
        let wanted = gpui::Keystroke::parse(keys).ok()?;
        self.keys.iter().find(|key| {
            key_spellings(key.key.keys).iter().any(|spelling| {
                gpui::Keystroke::parse(spelling).is_ok_and(|known| {
                    known.key == wanted.key && known.modifiers == wanted.modifiers
                })
            })
        })
    }

    /// Whether `keys` is a key the whiteboard holds for Excalidraw from the
    /// menu item of `action`: one of its own keys, not given to `action`.
    /// (A key given to another command is not that item's on a whiteboard
    /// either: only the command that has it there shows it.)
    pub fn holds_from(&self, keys: &str, action: &dyn gpui::Action) -> bool {
        self.find(keys).is_some_and(|key| match &key.given_to {
            None => true,
            Some(target) => target.command.action != action.name(),
        })
    }

    /// The keys given up to a command of the user's, in the keymap's normal
    /// form as `WHITEBOARD_KEYS` writes them: what the whiteboard's page is
    /// told so that no tooltip or menu item of Excalidraw's names them
    /// (`board.json`'s `keysGivenUp`, then the call
    /// `innerword_board_web::calls::KEYS` after an install).
    pub fn given_up(&self) -> Vec<&'static str> {
        self.keys
            .iter()
            .filter(|key| key.given_to.is_some())
            .map(|key| key.key.keys)
            .collect()
    }

    /// Whether `keys` is held for Excalidraw (one of its own keys, given to
    /// no command).
    pub fn holds(&self, keys: &str) -> bool {
        self.find(keys).is_some_and(|key| key.given_to.is_none())
    }
}

/// Which of the whiteboard's own keys are held, given the keymap that runs
/// (`installed`: the bindings in order, the fixed ones among them or not,
/// they are left out here) and what the keymap says the user made
/// (`effective`).
///
/// **The rule.** A key of `WHITEBOARD_KEYS` is given up when the binding
/// GPUI would run for it where a whiteboard has the keyboard (the key
/// contexts `[Workspace, BoardView]`, by GPUI's own ranking,
/// `Keymap::bindings_for_input`) is a key the user made (a changed or an
/// added key, `Origin::is_users`), in every spelling that matches anything
/// there. So a command the user gave the key that works anywhere, or one
/// bound in `BoardView`, has it on a whiteboard; Excalidraw's function of
/// that key is then reached from its own menus and toolbar. A key whose
/// winner is a default (the window's comment keys and Vim mode, today) and
/// a key nothing binds there stay held, as they always were.
pub fn whiteboard_keys(
    installed: &[KeyBinding],
    effective: &innerword_keymap::Effective,
) -> WhiteboardKeys {
    use innerword_keymap::KeySequence;
    let users: Vec<(innerword_keymap::Target, KeySequence)> = effective
        .bindings()
        .iter()
        .filter(|binding| binding.origin.is_users())
        .map(|binding| (binding.target.clone(), binding.keys.clone()))
        .collect();
    if users.is_empty() {
        return WhiteboardKeys::all_held();
    }
    let keymap = gpui::Keymap::new(
        installed
            .iter()
            .filter(|binding| !is_fixed_binding(binding))
            .cloned()
            .collect(),
    );
    let contexts = crate::pages::board_contexts();
    let users_key = |binding: &KeyBinding| -> Option<innerword_keymap::Target> {
        let target = crate::keymap::target_of(binding);
        let keys = KeySequence::parse(&crate::keymap::keys_of(binding)).ok()?;
        users
            .iter()
            .any(|(their_target, their_keys)| *their_target == target && *their_keys == keys)
            .then_some(target)
    };
    let mut held = WhiteboardKeys::all_held();
    for key in &mut held.keys {
        let mut given: Option<innerword_keymap::Target> = None;
        let mut all_users = true;
        for spelling in key_spellings(key.key.keys) {
            let Ok(keystroke) = gpui::Keystroke::parse(&spelling) else {
                continue;
            };
            let (bindings, _) =
                keymap.bindings_for_input(std::slice::from_ref(&keystroke), &contexts);
            let Some(winner) = bindings.first() else {
                continue;
            };
            match users_key(winner) {
                Some(target) => {
                    given.get_or_insert(target);
                }
                None => all_users = false,
            }
        }
        if all_users {
            key.given_to = given;
        }
    }
    held
}

/// Every spelling of a key in the keymap's normal form (`cmd-shift-,` and
/// `cmd-<`), the readable one last.
pub fn key_spellings(keys: &str) -> Vec<String> {
    match innerword_keymap::KeySequence::parse(keys) {
        Ok(sequence) => sequence.spellings(),
        Err(_) => vec![keys.to_string()],
    }
}

/// Whether `keys` (one keystroke, in any order of its modifiers: GPUI writes
/// `alt-cmd-c` where the keymap's normal form is `cmd-alt-c`) is one of the
/// whiteboard's own keys, in any of its spellings.
pub fn is_whiteboard_keys(keys: &str) -> bool {
    let Ok(wanted) = gpui::Keystroke::parse(keys) else {
        return false;
    };
    innerword_board_web::keys::WHITEBOARD_KEYS
        .iter()
        .any(|key| {
            key_spellings(key.keys).iter().any(|spelling| {
                gpui::Keystroke::parse(spelling).is_ok_and(|known| {
                    known.key == wanted.key && known.modifiers == wanted.modifiers
                })
            })
        })
}

// ----- the fake engine's canvas ---------------------------------------------------------

/// How the fake's next `adopt` fails.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FakeFailure {
    /// It throws: the call answers an error.
    Throws,
    /// It never answers.
    NeverAnswers,
}

/// A call of the app that the fake keeps for later.
type Done = Box<dyn FnOnce(Result<String, String>)>;

/// The fake engine's whiteboard: a scene held as JSON, what the app asked of
/// it, and the levers a test pulls ([`crate::pages::FakePages`]'s board
/// methods). Its `adopt` applies the base rule of section 4.5.
#[derive(Default)]
pub struct FakeBoard {
    /// The open whiteboard's id while one is shown.
    pub(crate) open: Option<DocumentId>,
    /// `(board, board.json)` of every whiteboard view made, in order.
    pub(crate) opened: Vec<(DocumentId, String)>,
    /// The `library.json` each view was served.
    pub(crate) library_served: Vec<String>,
    /// The stamp of the scene the view loaded.
    pub(crate) stamp: String,
    /// The canvas: `{"type","version","source","elements","appState","files"}`
    /// as the page would post it (files without `file`).
    pub(crate) canvas: Value,
    /// The canvas's versions of each element at the moments the app was
    /// given a rev (`versionsAt`).
    pub(crate) versions_at: VecDeque<(u64, HashMap<String, i64>)>,
    pub(crate) rev: u64,
    pub(crate) posted: u64,
    pub(crate) status: Status,
    /// Every call made, in words: "collect", "undo 1", "zoom in", …
    pub(crate) calls: Vec<String>,
    pub(crate) collects: usize,
    pub(crate) collects_held: bool,
    /// Collects whose answer waits (made when they were asked).
    pub(crate) waiting_collects: Vec<(Done, String)>,
    pub(crate) fail_adopts: Option<FakeFailure>,
    /// Adopts are answered only at [`crate::pages::FakePages::release_adopts`].
    pub(crate) adopts_held: bool,
    /// Adopts whose answer waits, with their argument.
    pub(crate) waiting_adopts: Vec<(Done, String)>,
    /// The canvas before each adopt that was one step of the undo, newest
    /// last: what Excalidraw's undo brings back.
    pub(crate) undo_stack: Vec<Value>,
    /// Adopts that never answer, kept so that dropping them does not answer.
    pub(crate) unanswered: Vec<Done>,
    pub(crate) adopted: Vec<String>,
    pub(crate) parked: bool,
    /// Pictures of the canvas the app has not been posted a `file` for.
    pub(crate) unposted_files: HashMap<String, (String, String)>,
    /// The next scenes name pictures whose `file` message never came.
    pub(crate) forget_file_messages: bool,
    /// The shape library: its items as JSON, and its counter.
    pub(crate) library: Value,
    pub(crate) lrev: u64,
    pub(crate) library_posted: u64,
    /// What `center` answers.
    pub(crate) selected: Vec<String>,
    /// The PDFs of whiteboards made, and whether the next ones fail.
    pub(crate) pdfs_made: usize,
    pub(crate) fail_pdfs: bool,
}

impl FakeBoard {
    /// The open view loads again: what its source serves now.
    pub(crate) fn reload(&mut self, board_json: &str, library_json: &str) {
        let Some(id) = self.open else {
            return;
        };
        let opened = self.opened.len();
        self.load(id, board_json, library_json);
        self.opened.truncate(opened);
    }

    /// A view of a whiteboard is made: it loads `board_json`.
    pub(crate) fn load(&mut self, id: DocumentId, board_json: &str, library_json: &str) {
        self.open = Some(id);
        self.opened.push((id, board_json.to_string()));
        self.library_served.push(library_json.to_string());
        let served: Value = serde_json::from_str(board_json).unwrap_or(Value::Null);
        self.stamp = served
            .get("stamp")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let mut scene = served.get("scene").cloned().unwrap_or_else(|| json!({}));
        if let Some(files) = scene.get_mut("files").and_then(Value::as_object_mut) {
            for entry in files.values_mut() {
                if let Some(entry) = entry.as_object_mut() {
                    entry.remove("file");
                }
            }
        }
        self.canvas = view_scene(scene);
        self.rev = 0;
        self.posted = 0;
        self.versions_at.clear();
        let versions = self.versions();
        self.versions_at.push_back((0, versions));
        self.status = Status {
            zoom: 1.0,
            elements: self.element_count(),
            ..Status::default()
        };
        let library: Value = serde_json::from_str(library_json).unwrap_or(Value::Null);
        self.library = library
            .get("libraryItems")
            .cloned()
            .unwrap_or_else(|| json!([]));
        self.lrev = 0;
        self.library_posted = 0;
        self.parked = false;
        self.unposted_files.clear();
        // A view that loads again has no undo history.
        self.undo_stack.clear();
    }

    /// The canvas's elements that are not deleted.
    pub(crate) fn element_count(&self) -> u64 {
        elements_of(&self.canvas)
            .iter()
            .filter(|element| {
                !element
                    .get("isDeleted")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            })
            .count() as u64
    }

    pub(crate) fn versions(&self) -> HashMap<String, i64> {
        elements_of(&self.canvas)
            .iter()
            .filter_map(|element| {
                Some((
                    element.get("id")?.as_str()?.to_string(),
                    element.get("version").and_then(Value::as_i64).unwrap_or(1),
                ))
            })
            .collect()
    }

    /// The canvas changed (a gesture, a merge): a new rev.
    pub(crate) fn changed(&mut self) {
        self.rev += 1;
        self.status.rev = self.rev;
        self.status.elements = self.element_count();
    }

    /// The app was given `rev`: the versions at that moment are kept.
    pub(crate) fn given(&mut self, rev: u64) {
        self.posted = self.posted.max(rev);
        self.status.posted = self.posted;
        let versions = self.versions();
        self.versions_at.retain(|(at, _)| *at != rev);
        self.versions_at.push_back((rev, versions));
        while self.versions_at.len() > 8 {
            self.versions_at.pop_front();
        }
    }

    /// The scene as the page posts it, as text.
    pub(crate) fn scene_text(&self) -> String {
        let mut scene = self.canvas.clone();
        if let Some(elements) = scene.get_mut("elements").and_then(Value::as_array_mut) {
            elements.retain(|element| {
                !element
                    .get("isDeleted")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            });
        }
        scene.to_string()
    }

    /// The base rule of section 4.5 on the canvas: `delta` is
    /// `innerword_board::Delta::to_json`'s.
    pub(crate) fn adopt(&mut self, delta: &str) -> Result<String, String> {
        let delta: Value = serde_json::from_str(delta).map_err(|error| error.to_string())?;
        let base_rev = delta.get("baseRev").and_then(Value::as_u64).unwrap_or(0);
        let at_base: Option<HashMap<String, i64>> = self
            .versions_at
            .iter()
            .find(|(rev, _)| *rev == base_rev)
            .map(|(_, versions)| versions.clone());
        let untouched = |id: &str, version: i64| {
            at_base
                .as_ref()
                .is_some_and(|versions| versions.get(id) == Some(&version))
        };
        let canvas_was_at_base = self.rev == base_rev;
        if delta
            .get("undoable")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            self.undo_stack.push(self.canvas.clone());
        }
        let mut conflicts = Vec::new();
        let mut set_versions_all_theirs = true;
        let mut elements: Vec<Value> = elements_of(&self.canvas).to_vec();
        let position = |elements: &[Value], id: &str| {
            elements
                .iter()
                .position(|element| element.get("id").and_then(Value::as_str) == Some(id))
        };
        for upserted in delta
            .get("upserted")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
        {
            let Some(incoming) = upserted.get("element").cloned() else {
                continue;
            };
            let id = incoming
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let base_version = upserted.get("baseVersion").and_then(Value::as_i64);
            let theirs_version = incoming.get("version").and_then(Value::as_i64).unwrap_or(1);
            match (position(&elements, &id), base_version) {
                (Some(ix), Some(_)) => {
                    let canvas_version = elements[ix]
                        .get("version")
                        .and_then(Value::as_i64)
                        .unwrap_or(1);
                    if untouched(&id, canvas_version) {
                        let mut incoming = incoming;
                        let version = theirs_version.max(canvas_version + 1);
                        set_versions_all_theirs &= version == theirs_version;
                        incoming["version"] = json!(version);
                        elements[ix] = incoming;
                    } else {
                        conflicts.push(id);
                    }
                }
                (None, None) => elements.push(incoming),
                (None, Some(_)) => conflicts.push(id),
                (Some(_), None) => conflicts.push(id),
            }
        }
        for removed in delta
            .get("removed")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
        {
            let id = removed
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if let Some(ix) = position(&elements, &id) {
                let canvas_version = elements[ix]
                    .get("version")
                    .and_then(Value::as_i64)
                    .unwrap_or(1);
                if untouched(&id, canvas_version) {
                    elements[ix]["isDeleted"] = json!(true);
                    elements[ix]["version"] = json!(canvas_version + 1);
                    set_versions_all_theirs = false;
                } else {
                    conflicts.push(id);
                }
            }
        }
        if let Some(files) = delta.get("files").and_then(Value::as_object) {
            let canvas_files = self.canvas["files"]
                .as_object_mut()
                .map(std::mem::take)
                .unwrap_or_default();
            let mut canvas_files = canvas_files;
            for (id, entry) in files {
                canvas_files.insert(
                    id.clone(),
                    json!({
                        "mimeType": entry.get("mimeType").cloned().unwrap_or(Value::Null),
                        "created": 0,
                    }),
                );
            }
            self.canvas["files"] = Value::Object(canvas_files);
        }
        self.canvas["elements"] = Value::Array(elements);
        self.changed();
        let same = conflicts.is_empty() && canvas_was_at_base && set_versions_all_theirs;
        let rev = self.rev;
        if same {
            self.given(rev);
        }
        Ok(json!({ "rev": rev, "same": same, "conflicts": conflicts }).to_string())
    }

    /// Excalidraw's undo of `steps` steps the adopts made: the canvas as it
    /// was before them, each element that differs from it now at a newer
    /// version (an element the adopts added is deleted). Returns whether
    /// there was such a step.
    pub(crate) fn undo(&mut self, steps: u64) -> bool {
        let mut restored = None;
        for _ in 0..steps {
            match self.undo_stack.pop() {
                Some(canvas) => restored = Some(canvas),
                None => break,
            }
        }
        let Some(restored) = restored else {
            return false;
        };
        let now: Vec<Value> = elements_of(&self.canvas).to_vec();
        let mut elements: Vec<Value> = elements_of(&restored).to_vec();
        let version_of =
            |element: &Value| element.get("version").and_then(Value::as_i64).unwrap_or(1);
        for element in &mut elements {
            let id = element
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let current = now
                .iter()
                .find(|known| known.get("id").and_then(Value::as_str) == Some(id.as_str()));
            if let Some(current) = current
                && current != element
            {
                let version = version_of(current).max(version_of(element)) + 1;
                element["version"] = json!(version);
            }
        }
        for current in &now {
            let id = current.get("id").and_then(Value::as_str);
            let kept = elements
                .iter()
                .any(|element| element.get("id").and_then(Value::as_str) == id);
            if !kept {
                let mut gone = current.clone();
                gone["isDeleted"] = json!(true);
                gone["version"] = json!(version_of(current) + 1);
                elements.push(gone);
            }
        }
        self.canvas["elements"] = Value::Array(elements);
        self.changed();
        true
    }

    /// `collect`'s answer, and what it gives the app.
    pub(crate) fn collect(&mut self) -> String {
        let rev = self.rev;
        let scene = if rev > self.posted {
            serde_json::from_str::<Value>(&self.scene_text()).unwrap_or(Value::Null)
        } else {
            Value::Null
        };
        let files: Vec<Value> = std::mem::take(&mut self.unposted_files)
            .into_iter()
            .map(
                |(id, (mime, data_url))| json!({ "id": id, "mimeType": mime, "dataURL": data_url }),
            )
            .collect();
        let library = (self.lrev > self.library_posted)
            .then(|| json!({ "lrev": self.lrev, "items": self.library.clone() }));
        self.library_posted = self.lrev;
        self.given(rev);
        self.status.editing = false;
        self.status.pointer_down = false;
        self.status.library_waiting = false;
        json!({
            "rev": rev,
            "scene": scene,
            "files": files,
            "library": library,
            "status": status_value(&self.status),
        })
        .to_string()
    }

    /// `insertElements`: an element whose id the canvas has is replaced, the
    /// others are put on top.
    pub(crate) fn insert_elements(&mut self, argument: &str) -> Vec<String> {
        let argument: Value = serde_json::from_str(argument).unwrap_or(Value::Null);
        let incoming = argument
            .get("elements")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut elements: Vec<Value> = elements_of(&self.canvas).to_vec();
        let mut ids = Vec::new();
        for element in incoming {
            let Some(id) = element
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_string)
            else {
                continue;
            };
            match elements
                .iter()
                .position(|known| known.get("id").and_then(Value::as_str) == Some(id.as_str()))
            {
                Some(ix) => elements[ix] = element,
                None => elements.push(element),
            }
            ids.push(id);
        }
        self.canvas["elements"] = Value::Array(elements);
        if argument
            .get("select")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            self.selected = ids.clone();
            self.status.selected = ids.len() as u64;
        }
        self.changed();
        ids
    }
}

/// A scene with the envelope and the three parts the page always has.
fn view_scene(scene: Value) -> Value {
    let mut map = match scene {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    map.entry("type").or_insert_with(|| json!("excalidraw"));
    map.entry("version").or_insert_with(|| json!(2));
    map.entry("source")
        .or_insert_with(|| json!(innerword_board::SOURCE));
    map.entry("elements").or_insert_with(|| json!([]));
    map.entry("appState").or_insert_with(|| json!({}));
    map.entry("files").or_insert_with(|| json!({}));
    Value::Object(map)
}

fn elements_of(scene: &Value) -> &[Value] {
    scene
        .get("elements")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

/// A `status` as the page writes it.
pub(crate) fn status_value(status: &Status) -> Value {
    serde_json::to_value(status).unwrap_or(Value::Null)
}

/// What a call's body is: the name of the glue's call it runs.
pub fn call_name(body: &str) -> &'static str {
    let known: [(&str, &'static str); 19] = [
        (crate::workspace::board::HOLD_KEYS, "holdKeys"),
        (crate::workspace::board::OPEN_TEXT, "openText"),
        (calls::COLLECT, "collect"),
        (calls::FILES, "files"),
        (calls::ADOPT, "adopt"),
        (calls::UNDO, "undo"),
        (calls::REDO, "redo"),
        (calls::SELECT_ALL, "selectAll"),
        (calls::ZOOM, "zoom"),
        (calls::SEARCH, "search"),
        (calls::CENTER, "center"),
        (calls::INSERT_PICTURES, "insertPictures"),
        (calls::INSERT_ELEMENTS, "insertElements"),
        (calls::STATE, "state"),
        (calls::MERMAID, "mermaid"),
        (calls::LIBRARY, "library"),
        (calls::KEYS, "keys"),
        (calls::EXPORT_LAYOUT, "exportLayout"),
        (calls::CONVERT_SKELETONS, "convertSkeletons"),
    ];
    known
        .iter()
        .find(|(known, _)| *known == body)
        .map(|(_, name)| *name)
        .unwrap_or("unknown")
}

/// The fixed one-page PDF the fake engine makes of `scene`: a page of the
/// size Rust estimates (`Scene::pdf_size`), drawn by `innerword-print`.
pub fn fake_board_pdf(scene: &Scene) -> innerword_web::PrintedPdf {
    let (width, height) = scene
        .pdf_size(
            innerword_board::limits::PDF_PADDING,
            innerword_board::limits::PDF_MAX_SIDE,
        )
        .unwrap_or((64.0, 64.0));
    let mut writer = innerword_print::PdfWriter::new(&innerword_print::JobInfo {
        title: "Whiteboard".to_string(),
        creator: format!("Inner Word {}", env!("CARGO_PKG_VERSION")),
    });
    writer.add_page(&innerword_print::Page {
        width: width as f32,
        height: height as f32,
        items: Vec::new(),
    });
    let pdf = writer.finish();
    innerword_web::PrintedPdf {
        bytes: pdf.bytes().to_vec(),
        pages: 1,
        paper: (width, height),
    }
}

/// The empty shape library as the setting keeps it.
pub fn empty_library_json() -> String {
    ShapeLibrary::empty().to_stored()
}

/// Reads the stored shape library of `library` (the setting as raw JSON;
/// `{}` when there is none).
pub fn stored_library(library: &Library) -> Arc<str> {
    match library.setting::<Box<serde_json::value::RawValue>>(SHAPE_LIBRARY_SETTING) {
        Ok(Some(raw)) => Arc::from(raw.get()),
        _ => Arc::from("{}"),
    }
}

/// The folder of the data directory where the drawing of a whiteboard that
/// could not be saved is written (`Recovered/whiteboard-<id>-<ms>.json`).
pub fn recovery_dir(library_db: &Path) -> Option<std::path::PathBuf> {
    Some(library_db.parent()?.join("Recovered"))
}

// ----- the fake engine's levers ---------------------------------------------------------

fn raw(text: String) -> Box<serde_json::value::RawValue> {
    serde_json::value::RawValue::from_string(text).unwrap_or_else(|_| {
        serde_json::value::RawValue::from_string("null".to_string()).expect("null is JSON")
    })
}

impl crate::pages::FakePages {
    /// Posts `message` as the open whiteboard's page would.
    fn post_board(&self, message: Message) {
        let events = self.state.borrow().events.clone();
        if let Some(events) = events {
            events.message(message.to_body());
        }
    }

    /// `(board, the board.json it was served)` of every whiteboard view made.
    pub fn boards_opened(&self) -> Vec<(DocumentId, String)> {
        self.board.borrow().opened.clone()
    }

    /// The `library.json` each whiteboard view was served.
    pub fn library_served(&self) -> Vec<String> {
        self.board.borrow().library_served.clone()
    }

    /// The open whiteboard's page posts `ready` with the stamp it loaded.
    pub fn board_ready(&self) {
        let (stamp, elements, status) = {
            let board = self.board.borrow();
            (
                board.stamp.clone(),
                board.element_count(),
                board.status.clone(),
            )
        };
        self.post_board(Message::Ready {
            excalidraw: innerword_board_web::EXCALIDRAW_VERSION.to_string(),
            elements,
            stamp,
        });
        self.post_board(Message::Status(status));
    }

    /// The writer drew: the canvas becomes `scene_json` (a scene, or an array
    /// of elements), and the page posts it with the next rev, as the glue does
    /// at a gesture's end.
    pub fn draw(&self, scene_json: &str) {
        let (rev, text, status) = {
            let mut board = self.board.borrow_mut();
            board.set_canvas(scene_json);
            board.changed();
            let rev = board.rev;
            board.given(rev);
            (rev, board.scene_text(), board.status.clone())
        };
        self.post_board_pictures();
        self.post_board(Message::Scene {
            rev,
            scene: raw(text),
        });
        self.post_board(Message::Status(status));
    }

    /// The canvas changed and the page has not posted it yet (the status says
    /// `rev > posted`).
    pub fn draw_unposted(&self, scene_json: &str) {
        let status = {
            let mut board = self.board.borrow_mut();
            board.set_canvas(scene_json);
            board.changed();
            board.status.clone()
        };
        self.post_board(Message::Status(status));
    }

    /// A post that left the page before an adopt: `scene_json` at `rev`.
    pub fn post_stale(&self, scene_json: &str, rev: u64) {
        let scene = view_scene(scene_value(scene_json)).to_string();
        self.post_board(Message::Scene {
            rev,
            scene: raw(scene),
        });
    }

    /// The writer pasted a picture: it is on the canvas's files and the page
    /// posts it (`file`), unless [`FakePages::forget_file_messages`] said
    /// otherwise.
    pub fn paste_picture(&self, id: &str, mime: &str, bytes: &[u8]) {
        let data_url = data_url_of(mime, bytes);
        let forget = {
            let mut board = self.board.borrow_mut();
            if let Some(files) = board.canvas.get_mut("files").and_then(Value::as_object_mut) {
                files.insert(id.to_string(), json!({ "mimeType": mime, "created": 0 }));
            }
            board
                .unposted_files
                .insert(id.to_string(), (mime.to_string(), data_url.clone()));
            board.forget_file_messages
        };
        if !forget {
            self.post_board_pictures();
        }
    }

    /// Posts a `file` for every picture of the canvas not posted yet.
    fn post_board_pictures(&self) {
        let pictures: Vec<(String, (String, String))> = {
            let mut board = self.board.borrow_mut();
            if board.forget_file_messages {
                return;
            }
            std::mem::take(&mut board.unposted_files)
                .into_iter()
                .collect()
        };
        for (id, (mime_type, data_url)) in pictures {
            self.post_board(Message::File {
                id,
                mime_type,
                data_url,
            });
        }
    }

    /// From now on the page posts no `file` for a picture it is given (the
    /// scene names a picture whose `file` never came).
    pub fn forget_file_messages(&self) {
        self.board.borrow_mut().forget_file_messages = true;
    }

    /// The page posts `status`.
    pub fn set_status(&self, status: Status) {
        self.board.borrow_mut().status = status.clone();
        self.post_board(Message::Status(status));
    }

    /// From now on the answer of `collect` waits for
    /// [`FakePages::release_collects`].
    pub fn hold_collects(&self) {
        self.board.borrow_mut().collects_held = true;
    }

    /// Answers the collects that wait, and every later one at once.
    pub fn release_collects(&self) {
        let waiting = {
            let mut board = self.board.borrow_mut();
            board.collects_held = false;
            std::mem::take(&mut board.waiting_collects)
        };
        for (done, answer) in waiting {
            done(Ok(answer));
        }
    }

    /// The next `adopt` fails as `how` says.
    pub fn fail_adopts(&self, how: FakeFailure) {
        self.board.borrow_mut().fail_adopts = Some(how);
    }

    /// From now on the answer of `adopt` waits for
    /// [`FakePages::release_adopts`].
    pub fn hold_adopts(&self) {
        self.board.borrow_mut().adopts_held = true;
    }

    /// Answers the adopts that wait (each takes its delta in then, as the
    /// glue does once its pictures and fonts are loaded), and every later one
    /// at once.
    pub fn release_adopts(&self) {
        let waiting = {
            let mut board = self.board.borrow_mut();
            board.adopts_held = false;
            std::mem::take(&mut board.waiting_adopts)
        };
        for (done, argument) in waiting {
            self.answer_adopt(&argument, done);
        }
    }

    /// The adopt of `argument` taken into the canvas and answered; a merge
    /// that is not the stored scene is posted as a change of the writer's.
    fn answer_adopt(&self, argument: &str, done: Done) {
        let answer = self.board.borrow_mut().adopt(argument);
        let same = answer
            .as_ref()
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(text).ok())
            .and_then(|value| value.get("same").and_then(Value::as_bool))
            .unwrap_or(true);
        done(answer);
        if !same {
            // The merge is posted as a change of the writer's.
            let (rev, text, status) = {
                let mut board = self.board.borrow_mut();
                let rev = board.rev;
                board.given(rev);
                (rev, board.scene_text(), board.status.clone())
            };
            self.post_board(Message::Scene {
                rev,
                scene: raw(text),
            });
            self.post_board(Message::Status(status));
        }
    }

    /// How many collects were asked.
    pub fn collects_asked(&self) -> usize {
        self.board.borrow().collects
    }

    /// The deltas adopted, in order.
    pub fn adopted(&self) -> Vec<String> {
        self.board.borrow().adopted.clone()
    }

    /// The fake canvas's scene now.
    pub fn canvas(&self) -> String {
        self.board.borrow().scene_text()
    }

    /// The calls the app made, in words.
    pub fn board_calls(&self) -> Vec<String> {
        self.board.borrow().calls.clone()
    }

    /// The shape library changed: the page posts `library` with the next
    /// counter, as `onLibraryChange` does 300 ms later, with the statuses
    /// the glue posts around it (`libraryWaiting` true at the change, false
    /// once the change is posted).
    pub fn change_library(&self, items_json: &str) {
        let waiting = self.library_changed(items_json);
        self.post_board(Message::Status(waiting));
        let (lrev, items, status) = {
            let mut board = self.board.borrow_mut();
            board.library_posted = board.lrev;
            board.status.library_waiting = false;
            (board.lrev, board.library.to_string(), board.status.clone())
        };
        self.post_board(Message::Library {
            lrev,
            items: raw(items),
            too_large: false,
        });
        self.post_board(Message::Status(status));
    }

    /// The shape library changed and the page has not posted it yet (inside
    /// the glue's 300 ms): the status it posts at once says
    /// `libraryWaiting`, and a `collect` brings the change.
    pub fn change_library_unposted(&self, items_json: &str) {
        let waiting = self.library_changed(items_json);
        self.post_board(Message::Status(waiting));
    }

    /// The canvas's library is `items_json` with the next counter, and the
    /// status says a change waits; returns that status.
    fn library_changed(&self, items_json: &str) -> Status {
        let mut board = self.board.borrow_mut();
        board.library = serde_json::from_str(items_json).unwrap_or_else(|_| json!([]));
        board.lrev += 1;
        board.status.library_waiting = true;
        board.status.clone()
    }

    /// Whether the closing view is parked.
    pub fn parked(&self) -> bool {
        self.board.borrow().parked
    }

    /// The fixed PDF the fake engine makes of `scene`.
    pub fn board_pdf(&self, scene: &Scene) -> innerword_web::PrintedPdf {
        fake_board_pdf(scene)
    }

    /// The PDF of a whiteboard, as the app asks the engine for it: counted,
    /// and refused while [`FakePages::fail_board_pdfs`] says so.
    pub fn make_board_pdf(&self, scene: &Scene) -> Result<innerword_web::PrintedPdf, String> {
        let mut board = self.board.borrow_mut();
        if board.fail_pdfs {
            return Err("the export page did not answer".to_string());
        }
        board.pdfs_made += 1;
        Ok(fake_board_pdf(scene))
    }

    /// From now on the PDF of a whiteboard fails (`fail`), or is made.
    pub fn fail_board_pdfs(&self, fail: bool) {
        self.board.borrow_mut().fail_pdfs = fail;
    }

    /// How many PDFs of whiteboards were made.
    pub fn board_pdfs_made(&self) -> usize {
        self.board.borrow().pdfs_made
    }

    /// The writer selected the elements `ids` (what `center` answers).
    pub fn select(&self, ids: &[&str]) {
        let status = {
            let mut board = self.board.borrow_mut();
            board.selected = ids.iter().map(|id| id.to_string()).collect();
            board.status.selected = ids.len() as u64;
            board.status.clone()
        };
        self.post_board(Message::Status(status));
    }

    /// The page's `?`: it asks the app for its list of shortcuts.
    pub fn press_help(&self) {
        self.post_board(Message::Help);
    }

    /// The page cannot start (`board.json` unreadable).
    pub fn fail_start(&self) {
        self.post_board(Message::Error {
            message: format!(
                "{}: board.json cannot be read",
                innerword_board_web::bridge::START_FAILED
            ),
        });
    }
}

impl FakeBoard {
    fn set_canvas(&mut self, scene_json: &str) {
        let incoming = scene_value(scene_json);
        let files = self.canvas.get("files").cloned();
        let mut scene = view_scene(incoming);
        // A scene that names no files keeps the canvas's.
        if scene["files"].as_object().is_some_and(Map::is_empty)
            && let Some(files) = files
        {
            scene["files"] = files;
        }
        self.canvas = scene;
    }
}

/// A scene the test gives: a whole scene, or an array of its elements.
fn scene_value(text: &str) -> Value {
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Array(elements)) => json!({ "elements": elements }),
        Ok(value) => value,
        Err(_) => json!({}),
    }
}

impl crate::pages::FakeView {
    /// A view of whiteboard `id`: it loads `board_json` and `library_json`
    /// as the real view fetches them.
    #[allow(clippy::too_many_arguments)]
    pub fn new_board(
        pages: Rc<crate::pages::FakePages>,
        id: DocumentId,
        host: &str,
        cell: SharedCell,
        events: Rc<dyn innerword_web::PageEvents>,
        inbox: crate::pages::SharedInbox,
    ) -> crate::pages::FakeView {
        let mut view =
            crate::pages::FakeView::new(pages.clone(), id, host, "index.html", "", events, inbox);
        let (board_json, library_json) = match cell.lock() {
            Ok(cell) => (cell.board_json(), cell.library.to_string()),
            Err(_) => (String::new(), String::new()),
        };
        pages
            .board
            .borrow_mut()
            .load(id, &board_json, &library_json);
        view.board = Some(id);
        view.board_cell = Some(cell);
        view
    }

    /// Parks the view (a closing whiteboard).
    pub fn park(&self) {
        self.pages.board.borrow_mut().parked = true;
    }

    /// Runs the glue's call `body` on the fake canvas.
    pub fn call(
        &self,
        body: &str,
        argument: &str,
        done: impl FnOnce(Result<String, String>) + 'static,
    ) {
        let pages = self.pages.clone();
        let name = call_name(body);
        let argument_value: Value = serde_json::from_str(argument).unwrap_or(Value::Null);
        match name {
            "collect" => {
                let mut board = pages.board.borrow_mut();
                board.collects += 1;
                board.calls.push("collect".to_string());
                let answer = board.collect();
                if board.collects_held {
                    board.waiting_collects.push((Box::new(done), answer));
                    return;
                }
                drop(board);
                done(Ok(answer));
            }
            "adopt" => {
                let (failure, held) = {
                    let mut board = pages.board.borrow_mut();
                    board.calls.push("adopt".to_string());
                    board.adopted.push(argument.to_string());
                    (board.fail_adopts.take(), board.adopts_held)
                };
                match failure {
                    Some(FakeFailure::Throws) => done(Err("the merge threw".to_string())),
                    Some(FakeFailure::NeverAnswers) => {
                        pages.board.borrow_mut().unanswered.push(Box::new(done));
                    }
                    None if held => {
                        pages
                            .board
                            .borrow_mut()
                            .waiting_adopts
                            .push((Box::new(done), argument.to_string()));
                    }
                    None => pages.answer_adopt(argument, Box::new(done)),
                }
            }
            "files" => {
                let ids: Vec<String> = argument_value
                    .get("ids")
                    .and_then(Value::as_array)
                    .map(|ids| {
                        ids.iter()
                            .filter_map(|id| id.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                let (posted, missing, messages) = {
                    let mut board = pages.board.borrow_mut();
                    board.calls.push(format!("files {}", ids.join(",")));
                    let mut posted = Vec::new();
                    let mut missing = Vec::new();
                    let mut messages = Vec::new();
                    for id in ids {
                        match board.unposted_files.remove(&id) {
                            Some((mime_type, data_url)) => {
                                messages.push(Message::File {
                                    id: id.clone(),
                                    mime_type,
                                    data_url,
                                });
                                posted.push(id);
                            }
                            None => missing.push(id),
                        }
                    }
                    (posted, missing, messages)
                };
                for message in messages {
                    pages.post_board(message);
                }
                done(Ok(
                    json!({ "posted": posted, "missing": missing }).to_string()
                ));
            }
            "undo" | "redo" => {
                let steps = argument_value
                    .get("steps")
                    .and_then(Value::as_u64)
                    .unwrap_or(1);
                let (rev, undone) = {
                    let mut board = pages.board.borrow_mut();
                    board.calls.push(format!("{name} {steps}"));
                    // An undo of what adopts made takes the canvas back; the
                    // page posts it as a change of the writer's.
                    let undone = name == "undo" && board.undo(steps);
                    (board.rev, undone)
                };
                done(Ok(json!({ "rev": rev }).to_string()));
                if undone {
                    pages.post_board_scene_now();
                }
            }
            "zoom" => {
                let words = if argument_value.get("in").is_some() {
                    "in".to_string()
                } else if argument_value.get("out").is_some() {
                    "out".to_string()
                } else if argument_value.get("fit").is_some() {
                    "fit".to_string()
                } else {
                    format!(
                        "to {}",
                        argument_value
                            .get("to")
                            .and_then(Value::as_f64)
                            .unwrap_or(1.0)
                    )
                };
                let zoom = {
                    let mut board = pages.board.borrow_mut();
                    board.calls.push(format!("zoom {words}"));
                    if let Some(to) = argument_value.get("to").and_then(Value::as_f64) {
                        board.status.zoom = to;
                    }
                    board.status.zoom
                };
                done(Ok(json!({ "zoom": zoom }).to_string()));
            }
            "search" => {
                let query = argument_value
                    .get("query")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                pages
                    .board
                    .borrow_mut()
                    .calls
                    .push(format!("search {query}").trim_end().to_string());
                done(Ok("{}".to_string()));
            }
            "keys" => {
                let given_up: Vec<&str> = argument_value
                    .get("givenUp")
                    .and_then(Value::as_array)
                    .map(|keys| keys.iter().filter_map(Value::as_str).collect())
                    .unwrap_or_default();
                pages.board.borrow_mut().calls.push(
                    format!("keys {}", given_up.join(","))
                        .trim_end()
                        .to_string(),
                );
                done(Ok("{}".to_string()));
            }
            "center" => {
                let selected = {
                    let mut board = pages.board.borrow_mut();
                    board.calls.push("center".to_string());
                    board.selected.clone()
                };
                done(Ok(
                    json!({ "x": 0.0, "y": 0.0, "selected": selected }).to_string()
                ));
            }
            "insertPictures" => {
                let pictures = argument_value
                    .get("pictures")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let ids: Vec<String> = pictures
                    .iter()
                    .filter_map(|picture| picture.get("file").and_then(Value::as_str))
                    .map(str::to_string)
                    .collect();
                {
                    let mut board = pages.board.borrow_mut();
                    board
                        .calls
                        .push(format!("insertPictures {}", pictures.len()));
                    for picture in &pictures {
                        let file = picture
                            .get("file")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        let mime = picture.get("mimeType").cloned().unwrap_or(Value::Null);
                        if let Some(files) =
                            board.canvas.get_mut("files").and_then(Value::as_object_mut)
                        {
                            files.insert(
                                file.to_string(),
                                json!({ "mimeType": mime, "created": 0 }),
                            );
                        }
                        if let Some(elements) = board
                            .canvas
                            .get_mut("elements")
                            .and_then(Value::as_array_mut)
                        {
                            elements.push(json!({
                                "id": format!("picture-{file}"),
                                "type": "image",
                                "x": 0, "y": 0,
                                "width": picture.get("width").cloned().unwrap_or(json!(100)),
                                "height": picture.get("height").cloned().unwrap_or(json!(100)),
                                "fileId": file,
                                "status": "saved",
                                "version": 1,
                                "versionNonce": 1,
                                "isDeleted": false,
                            }));
                        }
                    }
                    board.changed();
                }
                done(Ok(json!({ "ids": ids }).to_string()));
                pages.post_board_scene_now();
            }
            "insertElements" => {
                let ids = {
                    let mut board = pages.board.borrow_mut();
                    let ids = board.insert_elements(argument);
                    board.calls.push(format!("insertElements {}", ids.len()));
                    ids
                };
                done(Ok(json!({ "ids": ids }).to_string()));
                pages.post_board_scene_now();
            }
            "state" => {
                let status = pages.board.borrow().status.clone();
                done(Ok(status_value(&status).to_string()));
            }
            "holdKeys" => {
                pages.board.borrow_mut().calls.push("holdKeys".to_string());
                done(Ok("{}".to_string()));
            }
            "openText" => {
                let id = argument_value
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let status = {
                    let mut board = pages.board.borrow_mut();
                    match &id {
                        Some(id) => {
                            board.calls.push(format!("openText {id}"));
                            board.status.editing = true;
                        }
                        None => board.calls.push("openText".to_string()),
                    }
                    board.status.clone()
                };
                done(Ok(json!({ "editing": id.is_some() }).to_string()));
                pages.post_board(Message::Status(status));
            }
            "mermaid" => {
                let words = match argument_value.get("insert").and_then(Value::as_str) {
                    Some(definition) => format!("mermaid {definition}"),
                    None => "mermaid".to_string(),
                };
                pages.board.borrow_mut().calls.push(words);
                done(Ok("{}".to_string()));
            }
            "library" => {
                let words = argument_value
                    .as_object()
                    .and_then(|map| map.keys().next().cloned())
                    .unwrap_or_default();
                let lrev = {
                    let mut board = pages.board.borrow_mut();
                    board.calls.push(format!("library {words}"));
                    // As the glue: the selection becomes an item, the status
                    // says a change waits, and the change is posted a moment
                    // later (never, here: a collect takes it).
                    if words == "addSelection" {
                        let id = format!("added-{}", board.lrev + 1);
                        let shape = json!({
                            "id": format!("{id}-shape"), "type": "rectangle", "x": 0.0,
                            "y": 0.0, "width": 100.0, "height": 50.0, "angle": 0,
                            "strokeColor": "#1e1e1e", "backgroundColor": "transparent",
                            "fillStyle": "solid", "strokeWidth": 2, "strokeStyle": "solid",
                            "roughness": 1, "opacity": 100, "groupIds": [], "frameId": null,
                            "roundness": null, "seed": 1, "version": 1, "versionNonce": 8,
                            "isDeleted": false, "boundElements": null, "updated": 1,
                            "link": null, "locked": false, "index": "a0",
                        });
                        let item = json!({ "id": id, "status": "unpublished",
                            "created": board.lrev + 1, "elements": [shape] });
                        if let Some(items) = board.library.as_array_mut() {
                            items.insert(0, item);
                        } else {
                            board.library = json!([item]);
                        }
                        board.lrev += 1;
                        // The glue posts a status at once when a change
                        // starts to wait for its post.
                        board.status.library_waiting = true;
                    }
                    (
                        board.lrev,
                        (words == "addSelection").then(|| board.status.clone()),
                    )
                };
                let (lrev, waiting) = lrev;
                if let Some(status) = waiting {
                    pages.post_board(Message::Status(status));
                }
                done(Ok(
                    json!({ "lrev": lrev, "added": null, "refused": null }).to_string()
                ));
            }
            other => {
                pages.board.borrow_mut().calls.push(other.to_string());
                done(Err(format!("the fake whiteboard has no call {other}")));
            }
        }
    }
}

impl crate::pages::FakePages {
    /// The page posts the canvas as it is (a gesture ended).
    fn post_board_scene_now(&self) {
        let (rev, text, status) = {
            let mut board = self.board.borrow_mut();
            let rev = board.rev;
            board.given(rev);
            (rev, board.scene_text(), board.status.clone())
        };
        self.post_board(Message::Scene {
            rev,
            scene: raw(text),
        });
        self.post_board(Message::Status(status));
    }
}

/// Whether `binding` is one of the fixed bindings that keep GPUI off the
/// whiteboard's own keys ([`WhiteboardKeys::fixed_bindings`]): no default, no row of a
/// command, no entry of the shortcuts file.
pub fn is_fixed_binding(binding: &KeyBinding) -> bool {
    binding.action().name() == gpui::Action::name(&gpui::NoAction)
        && binding
            .predicate()
            .is_some_and(|predicate| predicate.to_string() == BOARD_VIEW)
}

impl crate::pages::FakePages {
    /// The page posts `json` as the scene of `rev`, as it is (a scene the app
    /// cannot read: `{"elements": 5}`).
    pub fn post_scene_raw(&self, json: &str, rev: u64) {
        self.post_board(Message::Scene {
            rev,
            scene: raw(json.to_string()),
        });
    }

    /// The status the open whiteboard's page holds now.
    pub fn board_status(&self) -> Status {
        self.board.borrow().status.clone()
    }
}
