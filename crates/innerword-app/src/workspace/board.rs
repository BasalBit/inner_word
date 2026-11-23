//! An open whiteboard: the board half of the open web item
//! (`docs/whiteboard-design.md`, sections 4 and 6).
//!
//! A whiteboard is shown by the view pages have (`page.rs`: the native view
//! above GPUI's, one focus, the inbox, the overlays, the theme), with what
//! only a whiteboard's view has: one script message handler (the bridge),
//! calls into the page, no navigation after the first load and a stricter
//! policy. What is here:
//!
//! - **Opening** ([`Workspace::open_board`]): the stored scene is read once,
//!   the picture map is seeded from it, the view's source is given a cell
//!   ([`crate::boards::BoardCell`]) that this session keeps up to date.
//! - **Saving: never lose a stroke** (section 4.6). The page posts the scene
//!   at the end of every gesture; the app holds the newest ([`Held`]) and
//!   saves it in the background, compare-and-swap against the stored scene
//!   the view's scene descends from ([`BoardState::stamp`]). A posted
//!   picture is stored before a save names it. A scene that cannot be saved
//!   goes to a recovery file. The synchronous flush writes what the app holds.
//! - **Deliberate ends** ask the page first ([`Workspace::collect_board`]):
//!   another item opened or the whiteboard closed while the page holds more
//!   than it posted parks the view ([`ClosingBoard`]) until its last scene is
//!   saved.
//! - **The calls** ([`Workspace::board_call`]): undo, redo, select all, the
//!   zoom, the search, the six Add commands, Mermaid, the shape library.

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use gpui::{
    Action, AnyElement, App, Context, KeyBinding, Task, Window, actions, div, prelude::*, px,
};
use innerword_board::{Scene, Skeleton};
use innerword_board_web::{
    bridge::{Message, Status},
    calls,
};
use innerword_editor_tools::actions as tools;
use innerword_library::{
    DocumentId, OutsideChange,
    store::{ContentStamp, FileId},
};
use innerword_ui::{ContextMenu, ContextMenuItem, Toast};

use super::{
    EditCommand, Workspace,
    page::{PageSession, PageState, PageViewHandle, WebItem},
    saver::{BoardSaveJob, BoardSaveOutcome, NotPrepared, StoredPictures},
    session::{RETRY_DELAY, SNAPSHOT_INTERVAL},
};
use crate::{
    boards::{self, BOARD_VIEW, SharedCell},
    pages,
};

actions!(
    board,
    [
        /// Open the app's menu of the open whiteboard.
        OpenContextMenu,
        /// Put a new text at the middle of the whiteboard, open for typing.
        AddText,
        /// Put a new rectangle at the middle of the whiteboard, selected.
        AddRectangle,
        /// Put a new ellipse at the middle of the whiteboard, selected.
        AddEllipse,
        /// Put a new diamond at the middle of the whiteboard, selected.
        AddDiamond,
        /// Put a new arrow at the middle of the whiteboard (between the two
        /// selected shapes, when two are selected).
        AddArrow,
        /// Put a new frame at the middle of the whiteboard (around the
        /// selected elements, when some are selected).
        AddFrame,
        /// Open Excalidraw's dialog that turns a Mermaid diagram into a
        /// drawing.
        MermaidToDrawing,
        /// Show or hide the shape library.
        ToggleShapeLibrary,
        /// Add the selection to the shape library.
        AddToShapeLibrary,
        /// Put a copy of an item of the shape library on the whiteboard.
        InsertFromShapeLibrary,
    ]
);

/// The keys of an open whiteboard: its menu (`shift-f10` first, so that
/// hints show `ctrl-enter`, as everywhere), and Find once more (a whiteboard
/// is not on the tools' path). `cmd-g` is Excalidraw's group here, so Find
/// Next and Previous are not bound.
pub fn board_key_bindings() -> Vec<KeyBinding> {
    let board = Some(BOARD_VIEW);
    vec![
        KeyBinding::new("shift-f10", OpenContextMenu, board),
        KeyBinding::new("ctrl-enter", OpenContextMenu, board),
        KeyBinding::new("cmd-f", tools::Find, board),
    ]
}

/// The document commands on a whiteboard (section 6.3).
pub const NO_SETTINGS_ON_A_BOARD: &str = "A whiteboard has no document settings.";
pub const TITLE_OF_A_BOARD: &str =
    "A whiteboard keeps the title it was given: rename it to change it.";
pub const NO_SPELLING_ON_A_BOARD: &str = "A whiteboard is not checked for spelling.";
pub const NO_COMMENTS_ON_A_BOARD: &str = "A whiteboard has no comments.";
pub const NO_VIM_ON_A_BOARD: &str = "Vim mode is for texts; a whiteboard has its own keys.";
pub const NO_PAPER_ON_A_BOARD: &str =
    "A whiteboard has no paper size: Zoom to Fit shows the whole drawing.";
/// What a whiteboard shows when its WebContent process ended twice in a row.
pub const BOARD_STOPPED: &str = "This whiteboard stopped working.";
/// What the Add commands say before the canvas is there.
pub const STILL_LOADING: &str = "The whiteboard is still loading.";
/// Reload whiteboard's toast.
pub const RELOADED: &str =
    "Reloaded. Undo starts again from here; Version History has the earlier states.";
/// The shape library over its limits.
pub const LIBRARY_TOO_LARGE: &str = "The shape library is too large to keep (over 8 MB). Remove \
     some of its items; until then the last library kept comes back.";
/// A scene over the soft limit.
pub const LARGE_BOARD: &str = "Large: saving this whiteboard takes longer";

/// How soon after a post the whiteboard is saved: the page posts at the end
/// of a gesture already, this only gathers a burst of them.
pub const BOARD_SAVE_DELAY: Duration = Duration::from_millis(150);

/// How often a wait for a picture still being stored looks again.
const PICTURE_POLL: Duration = Duration::from_millis(20);
/// How long a save waits for pictures still being stored.
const PICTURE_PATIENCE: Duration = Duration::from_secs(5);

/// A picture of the open whiteboard, by Excalidraw's file id (section 4.6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Picture {
    /// Its bytes are in the library (`FileId` hex) with their type.
    Stored { file: String, mime: String },
    /// The page posted it; it is being made and stored.
    Pending { data_url: Arc<str> },
    /// A scene named it without a `file` message: the page was asked.
    Asked,
    /// The page does not have it either, or the library lost it.
    Missing,
    /// It is no picture the library takes (the sentence).
    Refused(String),
}

/// The newest scene the page posted.
#[derive(Clone, Debug)]
pub(crate) struct Held {
    pub rev: u64,
    pub json: Arc<str>,
}

/// The parts of a scene longer than one message, as they come.
#[derive(Debug)]
struct Parts {
    rev: u64,
    texts: Vec<Option<String>>,
}

/// Where a drawing that could not be saved was written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Recovered {
    /// The recovery file.
    File(std::path::PathBuf),
    /// A library in memory keeps it here (tests read it).
    Memory(String),
}

/// A background save of a whiteboard that has not answered yet.
pub(crate) struct BoardInFlight {
    seq: u64,
    _task: Task<()>,
}

/// The board half of an open web item (section 4.6, "What the app holds").
pub(crate) struct BoardState {
    /// What the view's source serves, kept up to date.
    pub cell: SharedCell,
    /// The stamp of the stored scene the view's scene descends from.
    pub stamp: ContentStamp,
    /// That stored scene.
    pub base: Arc<str>,
    /// The view's rev the base came from.
    pub base_rev: u64,
    /// The page posted `ready`.
    pub ready: bool,
    pub held: Option<Held>,
    parts: Option<Parts>,
    /// The rev last saved.
    pub saved: u64,
    /// An outside write is being taken into the view: posts wait.
    pub adopting: bool,
    /// The newest post that came while an adopt was answered: the merge the
    /// page posts can come before the adopt's answer; it is taken when its
    /// rev is at or above the one the adopt answers.
    pending_post: Option<Held>,
    /// Posts below this rev are dropped (they left the page before a merge).
    pub floor: u64,
    pub status: Status,
    pub pictures: HashMap<String, Picture>,
    pub in_flight: Option<BoardInFlight>,
    pub save_task: Option<Task<()>>,
    /// The message of the last failed save.
    pub failure: Option<String>,
    failures_in_a_row: u32,
    pub recovered: Option<Recovered>,
    large_told: bool,
    /// The view state as last written.
    stored_view: Option<String>,
    ready_waiters: Vec<pages::AnswerTx<()>>,
    /// The collects in flight (each answered in its own task).
    pub collecting: usize,
    /// What the session's source is, to tell it of a picture stored later
    /// (a library in memory).
    source: Option<Arc<boards::LibraryBoardSource>>,
}

impl BoardState {
    /// Whether the app holds a scene it has not saved.
    pub fn dirty(&self) -> bool {
        self.held.as_ref().is_some_and(|held| held.rev > self.saved)
    }

    /// The pictures the app has stored, for a save.
    fn stored_pictures(&self) -> StoredPictures {
        self.pictures
            .iter()
            .filter_map(|(id, picture)| match picture {
                Picture::Stored { file, mime } => Some((
                    id.clone(),
                    (innerword_board::FileHex::parse(file)?, mime.clone()),
                )),
                _ => None,
            })
            .collect()
    }

    /// Whether a picture is on its way (a save waits for it).
    fn pictures_pending(&self) -> bool {
        self.pictures
            .values()
            .any(|picture| matches!(picture, Picture::Pending { .. } | Picture::Asked))
    }

    /// The view state to store: where the writer looks, with the stamp.
    fn view_state(&self) -> Option<String> {
        (self.status.zoom > 0.0).then(|| {
            serde_json::json!({
                "scrollX": self.status.scroll_x,
                "scrollY": self.status.scroll_y,
                "zoom": self.status.zoom,
                "at": self.stamp.to_hex(),
            })
            .to_string()
        })
    }

    /// The stored scene becomes the base (a save, an outside write).
    fn set_base(&mut self, stamp: ContentStamp, scene: Arc<str>, files: &[String]) {
        self.stamp = stamp;
        self.base = scene.clone();
        if let Ok(mut cell) = self.cell.lock() {
            cell.stamp = stamp.to_hex();
            cell.scene = scene;
            cell.files.extend(files.iter().cloned());
        }
    }

    /// Seeds the picture map from a stored scene: each entry with a `file`
    /// is `Stored`, so that no save drops a picture the scene had.
    fn seed_pictures(&mut self, scene: &Scene) {
        for (id, entry) in scene.files() {
            let picture = match &entry.file {
                Some(file) => Picture::Stored {
                    file: file.as_str().to_string(),
                    mime: entry.mime_type.clone(),
                },
                None => Picture::Missing,
            };
            let keep = matches!(
                self.pictures.get(id),
                Some(Picture::Pending { .. } | Picture::Stored { .. })
            );
            if !keep {
                self.pictures.insert(id.clone(), picture);
            }
        }
    }
}

/// A whiteboard whose session ended while its page held more than it had
/// posted: its view is parked, out of sight and still drawn, until what the
/// page hands over is saved (section 4.6).
pub(crate) struct ClosingBoard {
    pub session: PageSession,
    /// An item to open once it is done (the same whiteboard, opened again
    /// meanwhile): the id and whether to focus it.
    pub then_open: Option<(DocumentId, bool)>,
    _task: Option<Task<()>>,
}

/// The shape library as the app holds it (section 4.15): one for every
/// whiteboard, so the workspace's, not a session's.
#[derive(Default)]
pub(crate) struct ShapeLibraryState {
    /// The newest library the page posted: its counter and its items.
    pub held: Option<(u64, Arc<str>)>,
    /// The counter last stored.
    pub saved: u64,
    /// The stored setting (`{}` when there is none).
    pub stored: Option<Arc<str>>,
    /// A background save in flight.
    in_flight: Option<Task<()>>,
    /// "Too large" was said for this run of posts.
    pub too_large_told: bool,
}

impl ShapeLibraryState {
    /// Whether a posted library is not stored yet.
    pub fn dirty(&self) -> bool {
        self.held.as_ref().is_some_and(|(lrev, _)| *lrev > self.saved)
    }
}

impl Workspace {
    // ----- access --------------------------------------------------------------------

    /// The id of the open whiteboard.
    pub fn open_board_id(&self) -> Option<DocumentId> {
        self.page
            .as_ref()
            .filter(|page| matches!(page.item, WebItem::Board(_)))
            .map(|page| page.id)
    }

    /// The open whiteboard's board half.
    pub(crate) fn board(&self) -> Option<&BoardState> {
        match &self.page.as_ref()?.item {
            WebItem::Board(board) => Some(board),
            WebItem::Page(_) => None,
        }
    }

    /// The board half of whiteboard `id`: the open one, or the one closing.
    pub(crate) fn board_of_mut(&mut self, id: DocumentId) -> Option<&mut BoardState> {
        if self.page.as_ref().is_some_and(|page| page.id == id) {
            return match &mut self.page.as_mut()?.item {
                WebItem::Board(board) => Some(board),
                WebItem::Page(_) => None,
            };
        }
        let closing = self.closing_board.as_mut()?;
        if closing.session.id != id {
            return None;
        }
        match &mut closing.session.item {
            WebItem::Board(board) => Some(board),
            WebItem::Page(_) => None,
        }
    }

    fn board_of(&self, id: DocumentId) -> Option<&BoardState> {
        if let Some(page) = self.page.as_ref().filter(|page| page.id == id) {
            return match &page.item {
                WebItem::Board(board) => Some(board),
                WebItem::Page(_) => None,
            };
        }
        let closing = self.closing_board.as_ref()?;
        if closing.session.id != id {
            return None;
        }
        match &closing.session.item {
            WebItem::Board(board) => Some(board),
            WebItem::Page(_) => None,
        }
    }

    /// The session (open or closing) of whiteboard `id`.
    fn board_session(&self, id: DocumentId) -> Option<&PageSession> {
        if let Some(page) = self.page.as_ref().filter(|page| page.id == id) {
            return Some(page);
        }
        self.closing_board
            .as_ref()
            .map(|closing| &closing.session)
            .filter(|session| session.id == id)
    }

    /// The status the open whiteboard's page posted last.
    pub fn board_status(&self) -> Option<Status> {
        self.board().map(|board| board.status.clone())
    }

    /// Whether the open whiteboard's page posted `ready`.
    pub fn board_ready(&self) -> bool {
        self.board().is_some_and(|board| board.ready)
    }

    /// The scene of the open whiteboard as the app holds it: the newest
    /// post, else the stored scene it descends from (the view's form or the
    /// stored form; both read with `Scene::from_view`).
    pub fn board_scene_held(&self) -> Option<Arc<str>> {
        let board = self.board()?;
        Some(
            board
                .held
                .as_ref()
                .map(|held| held.json.clone())
                .unwrap_or_else(|| board.base.clone()),
        )
    }

    /// The scene of the open whiteboard as the app holds it, parsed.
    pub fn board_scene(&self) -> Option<Scene> {
        let board = self.board()?;
        let stored = board.stored_pictures();
        let json = self.board_scene_held()?;
        Scene::from_view(&json, &|id| stored.get(id).cloned())
            .or_else(|_| Scene::from_stored(&json))
            .ok()
            .map(|parsed| parsed.scene)
    }

    /// What was written where a drawing of the open whiteboard could not be
    /// saved.
    pub fn board_recovered(&self) -> Option<Recovered> {
        self.board().and_then(|board| board.recovered.clone())
    }

    /// Whether whiteboard `id` is closing (its view parked).
    pub fn board_closing(&self) -> Option<DocumentId> {
        self.closing_board.as_ref().map(|closing| closing.session.id)
    }

    // ----- opening -------------------------------------------------------------------

    /// Opens whiteboard `id`, closing the open item (saved first). With
    /// `focus` the canvas takes the keyboard. While the same whiteboard is
    /// still closing, it opens once its last scene is saved.
    pub fn open_board(
        &mut self,
        id: DocumentId,
        focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if let Some(closing) = self.closing_board.as_mut()
            && closing.session.id == id
        {
            closing.then_open = Some((id, focus));
            return true;
        }
        if self.open_page_id() != Some(id) && !self.load_board(id, window, cx) {
            return false;
        }
        if focus {
            self.dismiss_modal(window, cx);
            if let Some(focus) = self.page_focus_handle() {
                window.focus(&focus);
            }
        }
        true
    }

    fn load_board(&mut self, id: DocumentId, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let stored = match self.library.read(cx).document(id) {
            Ok(stored) => stored,
            Err(error) => {
                self.toast_error(format!("Could not open the whiteboard: {error}"), cx);
                return false;
            }
        };
        let scene = match Scene::from_stored(&stored.content) {
            Ok(parsed) => parsed.scene,
            Err(error) => {
                self.toast_error(
                    format!(
                        "Could not open “{}”: {error}",
                        super::session_title(&stored.meta.title)
                    ),
                    cx,
                );
                return false;
            }
        };
        let stamp = ContentStamp::of(&stored.content);
        let library_json = self.shape_library_json(cx);
        let cell: SharedCell = Arc::new(Mutex::new(boards::BoardCell {
            stamp: stamp.to_hex(),
            scene: Arc::from(stored.content.as_str()),
            view: Arc::from(stored.view_state.as_deref().unwrap_or("{}")),
            library: library_json.clone(),
            files: scene
                .stored_files()
                .iter()
                .map(|hex| hex.as_str().to_string())
                .collect(),
        }));
        let board = BoardState {
            cell: cell.clone(),
            stamp,
            base: Arc::from(stored.content.as_str()),
            base_rev: 0,
            ready: false,
            held: None,
            parts: None,
            saved: 0,
            adopting: false,
            pending_post: None,
            floor: 0,
            status: Status::default(),
            pictures: HashMap::new(),
            in_flight: None,
            save_task: None,
            failure: None,
            failures_in_a_row: 0,
            recovered: None,
            large_told: false,
            stored_view: stored.view_state.clone(),
            ready_waiters: Vec::new(),
            collecting: 0,
            source: None,
        };
        let mut board = Box::new(board);
        board.seed_pictures(&scene);
        let made = self.install_board_session(id, &stored, board, cell, &library_json, window, cx);
        if made {
            // The menu bar loses the keys that are the whiteboard's here.
            crate::menus::set_open_item(crate::menus::OpenItemKind::Board, cx);
        }
        made
    }

    /// The stored shape library, as the app holds it.
    pub(crate) fn shape_library_json(&mut self, cx: &mut Context<Self>) -> Arc<str> {
        if let Some(stored) = &self.shape_library.stored {
            return stored.clone();
        }
        let stored = boards::stored_library(self.library.read(cx));
        self.shape_library.stored = Some(stored.clone());
        stored
    }

    // ----- what the page posts ---------------------------------------------------------

    /// A message of whiteboard `id`'s bridge (the open one, or the one
    /// closing).
    pub(crate) fn on_board_message(
        &mut self,
        id: DocumentId,
        message: Message,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match message {
            Message::Ready {
                elements, stamp, ..
            } => {
                let Some(board) = self.board_of_mut(id) else {
                    return;
                };
                if stamp != board.stamp.to_hex() {
                    eprintln!(
                        "Inner Word: the whiteboard loaded the scene {stamp}, the app holds {}",
                        board.stamp.to_hex()
                    );
                }
                // A (re)load starts a new count of revs.
                board.ready = true;
                board.base_rev = 0;
                board.saved = 0;
                board.floor = 0;
                board.held = None;
                board.parts = None;
                board.status.elements = elements;
                board.status.rev = 0;
                board.status.posted = 0;
                for waiter in board.ready_waiters.drain(..) {
                    waiter.send(());
                }
                if let Some(page) = self.page.as_mut().filter(|page| page.id == id)
                    && !matches!(page.state, PageState::Crashed { again: true, .. })
                {
                    page.state = PageState::Loaded;
                }
            }
            Message::Scene { rev, scene } => {
                self.board_scene_posted(id, rev, Arc::from(scene.get()), cx);
            }
            Message::ScenePart {
                rev,
                index,
                count,
                text,
            } => {
                let whole = {
                    let Some(board) = self.board_of_mut(id) else {
                        return;
                    };
                    let parts = match board.parts.take() {
                        Some(parts) if parts.rev == rev && parts.texts.len() == count as usize => {
                            parts
                        }
                        Some(parts) if parts.rev > rev => {
                            board.parts = Some(parts);
                            return;
                        }
                        _ => Parts {
                            rev,
                            texts: vec![None; count as usize],
                        },
                    };
                    let mut parts = parts;
                    if let Some(slot) = parts.texts.get_mut(index as usize) {
                        *slot = Some(text);
                    }
                    if parts.texts.iter().all(Option::is_some) {
                        Some(parts.texts.into_iter().flatten().collect::<String>())
                    } else {
                        board.parts = Some(parts);
                        None
                    }
                };
                if let Some(json) = whole {
                    self.board_scene_posted(id, rev, Arc::from(json), cx);
                }
            }
            Message::File {
                id: file,
                mime_type: _,
                data_url,
            } => self.board_file_posted(id, file, Arc::from(data_url), cx),
            Message::Status(status) => {
                if let Some(board) = self.board_of_mut(id) {
                    board.status = status;
                }
            }
            Message::Library {
                lrev,
                items,
                too_large,
            } => self.shape_library_posted(lrev, items.get(), too_large, cx),
            Message::Help => self.open_shortcuts_on_a_whiteboard(window, cx),
            Message::Error { message } => {
                eprintln!("Inner Word: the whiteboard's page: {message}");
                let ready = self.board_ready();
                if message.starts_with(innerword_board_web::bridge::START_FAILED)
                    && !ready
                    && let Some(page) = self.page.as_mut().filter(|page| page.id == id)
                {
                    page.state = PageState::Failed(message);
                }
            }
        }
        cx.notify();
    }

    /// The page posted the scene at `rev`.
    fn board_scene_posted(
        &mut self,
        id: DocumentId,
        rev: u64,
        json: Arc<str>,
        cx: &mut Context<Self>,
    ) {
        let Some(board) = self.board_of_mut(id) else {
            return;
        };
        // While an adopt is answered a post waits: one from before the merge
        // is dropped when the answer comes, the merge itself is taken.
        if board.adopting {
            if board.pending_post.as_ref().is_none_or(|held| held.rev < rev) {
                board.pending_post = Some(Held { rev, json });
            }
            return;
        }
        // A post that left the page before a merge: the merged canvas holds
        // what it held, and it would write over the outside change.
        if rev < board.floor {
            return;
        }
        if board.held.as_ref().is_some_and(|held| held.rev > rev) {
            return;
        }
        board.status.posted = board.status.posted.max(rev);
        board.held = Some(Held { rev, json });
        self.schedule_board_save(id, BOARD_SAVE_DELAY, cx);
    }

    /// The page posted a picture the app has not had: it is made and stored
    /// in the background, and the save waits for it.
    fn board_file_posted(
        &mut self,
        id: DocumentId,
        file: String,
        data_url: Arc<str>,
        cx: &mut Context<Self>,
    ) {
        let Some(board) = self.board_of_mut(id) else {
            return;
        };
        if matches!(board.pictures.get(&file), Some(Picture::Stored { .. })) {
            return;
        }
        board.pictures.insert(
            file.clone(),
            Picture::Pending {
                data_url: data_url.clone(),
            },
        );
        let images = self.images.clone();
        cx.spawn(async move |this, cx| {
            let prepared = cx
                .background_spawn(async move {
                    let bytes = boards::data_url_bytes(&data_url)
                        .ok_or_else(|| "The pasted picture could not be read.".to_string())?;
                    boards::prepare_picture(&bytes, "The pasted picture")
                })
                .await;
            let prepared = match prepared {
                Ok(prepared) => prepared,
                Err(sentence) => {
                    this.update(cx, |this, cx| {
                        this.board_picture_done(id, &file, Err(sentence), cx)
                    })
                    .ok();
                    return;
                }
            };
            let stored = cx
                .update(|cx| images.put_files(vec![(prepared.file.clone(), prepared.bytes.clone())], cx))
                .ok();
            let result = match stored {
                Some(task) => task.await.map(|()| prepared),
                None => return,
            };
            this.update(cx, |this, cx| this.board_picture_done(id, &file, result, cx))
                .ok();
        })
        .detach();
    }

    /// A posted picture was stored (or refused).
    fn board_picture_done(
        &mut self,
        id: DocumentId,
        file: &str,
        result: Result<boards::PreparedPicture, String>,
        cx: &mut Context<Self>,
    ) {
        let refused = {
            let Some(board) = self.board_of_mut(id) else {
                return;
            };
            if !matches!(board.pictures.get(file), Some(Picture::Pending { .. })) {
                // Stored synchronously meanwhile (a quit).
                return;
            }
            match result {
                Ok(prepared) => {
                    if let Ok(mut cell) = board.cell.lock() {
                        cell.files.insert(prepared.file.clone());
                    }
                    if let Some(source) = &board.source {
                        source.remember(&prepared.file, &prepared.bytes);
                    }
                    board.pictures.insert(
                        file.to_string(),
                        Picture::Stored {
                            file: prepared.file,
                            mime: prepared.mime_type,
                        },
                    );
                    None
                }
                Err(sentence) => {
                    board
                        .pictures
                        .insert(file.to_string(), Picture::Refused(sentence.clone()));
                    Some(sentence)
                }
            }
        };
        if let Some(sentence) = refused {
            self.show_toast(Toast::new(sentence).duration(Duration::from_secs(8)), cx);
        }
        self.schedule_board_save(id, Duration::ZERO, cx);
    }

    // ----- saving ----------------------------------------------------------------------

    /// (Re)starts the timer of whiteboard `id`'s save.
    pub(crate) fn schedule_board_save(
        &mut self,
        id: DocumentId,
        delay: Duration,
        cx: &mut Context<Self>,
    ) {
        let Some(board) = self.board_of_mut(id) else {
            return;
        };
        board.save_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            this.update(cx, |this, cx| this.board_autosave(id, cx)).ok();
        }));
    }

    /// The save timer fired: save what is held, in the background when the
    /// library is a file.
    fn board_autosave(&mut self, id: DocumentId, cx: &mut Context<Self>) {
        let background = self.saver.is_background();
        let Some(board) = self.board_of_mut(id) else {
            return;
        };
        board.save_task = None;
        if board.in_flight.is_some() || board.adopting || !board.dirty() {
            return;
        }
        if board.pictures_pending() {
            // `board_picture_done` and the `files` answer save again.
            return;
        }
        if !background {
            self.save_board_asking(id, true, cx);
            return;
        }
        let Some(held) = board.held.clone() else {
            return;
        };
        let large = held.json.len() > innerword_board::limits::LARGE_VIEW_SCENE_BYTES
            && !board.large_told;
        if large {
            board.large_told = true;
        }
        let job_seq = self.saver.next_seq();
        let Some(board) = self.board_of_mut(id) else {
            return;
        };
        let job = BoardSaveJob {
            seq: job_seq,
            id,
            rev: held.rev,
            json: held.json.clone(),
            stored: board.stored_pictures(),
            known: board.pictures.keys().cloned().collect(),
            expect: board.stamp,
            view_state: board
                .view_state()
                .filter(|json| board.stored_view.as_deref() != Some(json.as_str())),
            snapshot_interval: SNAPSHOT_INTERVAL,
        };
        let rev = job.rev;
        let Some(work) = self.saver.work_board(job) else {
            self.save_board_now(id, cx);
            return;
        };
        let background = cx.background_spawn(async move { work() });
        let task = cx.spawn(async move |this, cx| {
            let outcome = background.await;
            this.update(cx, |this, cx| {
                this.board_save_finished(id, job_seq, rev, outcome, cx)
            })
            .ok();
        });
        if let Some(board) = self.board_of_mut(id) {
            board.in_flight = Some(BoardInFlight {
                seq: job_seq,
                _task: task,
            });
        }
        if large {
            self.show_toast(Toast::new(LARGE_BOARD).duration(Duration::from_secs(6)), cx);
        }
    }

    /// A background save of whiteboard `id` answered.
    fn board_save_finished(
        &mut self,
        id: DocumentId,
        seq: u64,
        rev: u64,
        outcome: BoardSaveOutcome,
        cx: &mut Context<Self>,
    ) {
        let Some(board) = self.board_of_mut(id) else {
            return;
        };
        if board.in_flight.as_ref().map(|save| save.seq) != Some(seq) {
            return;
        }
        board.in_flight = None;
        match outcome {
            BoardSaveOutcome::Superseded => {}
            BoardSaveOutcome::Saved {
                meta,
                stamp,
                scene,
                files,
                view_state,
                duration,
            } => {
                self.last_save = Some(super::SaveCost {
                    main_thread: Duration::ZERO,
                    background: Some(duration),
                    bytes: scene.len(),
                });
                let Some(board) = self.board_of_mut(id) else {
                    return;
                };
                board.set_base(stamp, scene, &files);
                board.base_rev = rev;
                board.saved = board.saved.max(rev);
                board.failures_in_a_row = 0;
                let recovered = board.failure.take().is_some();
                if let Some(view_state) = view_state {
                    board.stored_view = Some(view_state);
                }
                let again = board.dirty();
                if let Err(error) = self.library.update(cx, |library, cx| {
                    library.document_saved_elsewhere(&meta, cx)
                }) {
                    eprintln!("Inner Word: could not re-read the library: {error}");
                }
                if recovered {
                    self.show_toast(Toast::new("Saved").success(), cx);
                }
                if again {
                    self.schedule_board_save(id, Duration::ZERO, cx);
                }
            }
            BoardSaveOutcome::NeedsFiles(ids) => self.ask_for_board_files(id, ids, cx),
            BoardSaveOutcome::Stale => self.board_save_was_stale(id, cx),
            BoardSaveOutcome::Unsaveable(why) => self.board_unsaveable(id, &why, cx),
            BoardSaveOutcome::Failed(message) => self.board_save_failed(id, message, cx),
        }
        cx.notify();
    }

    /// Saves what whiteboard `id`'s session holds, now, on this thread.
    /// Returns `false` when it could not be saved (then it is retried, or a
    /// recovery file has it).
    pub(crate) fn save_board_now(&mut self, id: DocumentId, cx: &mut Context<Self>) -> bool {
        self.save_board_asking(id, false, cx)
    }

    /// [`Workspace::save_board_now`]; with `ask`, a scene that names pictures
    /// the app does not know asks the page for them first and is saved once
    /// they are there (an autosave), instead of being saved without them (a
    /// flush, which cannot wait).
    fn save_board_asking(&mut self, id: DocumentId, ask: bool, cx: &mut Context<Self>) -> bool {
        self.saver.settle();
        // A picture still being stored: made ready and stored through the
        // library's own connection, so the save names no bytes that are not
        // there (only a quit the system starts meets one).
        self.store_pending_pictures_now(id, cx);
        let Some(board) = self.board_of_mut(id) else {
            return true;
        };
        board.in_flight = None;
        board.save_task = None;
        if !board.dirty() || board.adopting {
            self.save_board_view_state(id, cx);
            return true;
        }
        let Some(held) = board.held.clone() else {
            return true;
        };
        // An id the page was asked for and did not answer: missing.
        for picture in board.pictures.values_mut() {
            if matches!(picture, Picture::Asked) {
                *picture = Picture::Missing;
            }
        }
        let stored = board.stored_pictures();
        let mut known: HashSet<String> = board.pictures.keys().cloned().collect();
        let expect = board.stamp;
        let prepared = match super::saver::prepare_board(&held.json, &stored, &known) {
            Ok(prepared) => prepared,
            Err(NotPrepared::NeedsFiles(ids)) if ask => {
                self.ask_for_board_files(id, ids, cx);
                return false;
            }
            Err(NotPrepared::NeedsFiles(ids)) => {
                // Not answered in time: saved without them (their elements
                // stay and show Excalidraw's missing picture).
                for unknown in &ids {
                    board.pictures.insert(unknown.clone(), Picture::Missing);
                    known.insert(unknown.clone());
                }
                match super::saver::prepare_board(&held.json, &stored, &known) {
                    Ok(prepared) => prepared,
                    Err(_) => return false,
                }
            }
            Err(NotPrepared::Unsaveable(why)) => {
                self.board_unsaveable(id, &why, cx);
                return false;
            }
        };
        let (result, _) = self.library.update(cx, |library, cx| {
            library.change_outside(
                |store| {
                    let result = store.save_board(id, &prepared.content(), Some(expect));
                    let changes = if result.is_ok() {
                        if let Err(error) = store.snapshot_if_stale(id, SNAPSHOT_INTERVAL) {
                            eprintln!("Inner Word: could not take a version snapshot: {error}");
                        }
                        vec![OutsideChange::DocumentSaved(id)]
                    } else {
                        Vec::new()
                    };
                    (result, changes)
                },
                cx,
            )
        });
        match result {
            Ok(saved) => {
                let files: Vec<String> = prepared.files.iter().map(FileId::to_hex).collect();
                let Some(board) = self.board_of_mut(id) else {
                    return true;
                };
                board.set_base(saved.stamp, Arc::from(prepared.scene), &files);
                board.base_rev = held.rev;
                board.saved = board.saved.max(held.rev);
                board.failures_in_a_row = 0;
                let recovered = board.failure.take().is_some();
                self.save_board_view_state(id, cx);
                if recovered {
                    self.show_toast(Toast::new("Saved").success(), cx);
                }
                cx.notify();
                true
            }
            Err(innerword_library::StoreError::Stale { .. }) => {
                self.board_save_was_stale(id, cx);
                false
            }
            Err(error) => {
                self.board_save_failed(id, error.to_string(), cx);
                false
            }
        }
    }

    /// Writes where the writer looks on whiteboard `id`, when it changed.
    fn save_board_view_state(&mut self, id: DocumentId, cx: &mut Context<Self>) {
        let Some(board) = self.board_of(id) else {
            return;
        };
        let Some(json) = board.view_state() else {
            return;
        };
        if board.stored_view.as_deref() == Some(json.as_str()) {
            return;
        }
        let saved = self
            .library
            .update(cx, |library, _| library.save_view_state(id, &json));
        if saved.is_ok()
            && let Some(board) = self.board_of_mut(id)
        {
            board.stored_view = Some(json);
        }
    }

    /// Stores, on this thread, every picture of whiteboard `id` that is
    /// still on its way.
    fn store_pending_pictures_now(&mut self, id: DocumentId, cx: &mut Context<Self>) {
        let pending: Vec<(String, Arc<str>)> = match self.board_of(id) {
            Some(board) => board
                .pictures
                .iter()
                .filter_map(|(file, picture)| match picture {
                    Picture::Pending { data_url } => Some((file.clone(), data_url.clone())),
                    _ => None,
                })
                .collect(),
            None => return,
        };
        for (file, data_url) in pending {
            let prepared = boards::data_url_bytes(&data_url)
                .ok_or_else(|| "The pasted picture could not be read.".to_string())
                .and_then(|bytes| boards::prepare_picture(&bytes, "The pasted picture"));
            let result = prepared.and_then(|prepared| {
                self.library
                    .update(cx, |library, _| library.put_file(&prepared.bytes))
                    .map(|_| prepared)
                    .map_err(|error| error.to_string())
            });
            if let Some(board) = self.board_of_mut(id) {
                let picture = match result {
                    Ok(prepared) => {
                        if let Ok(mut cell) = board.cell.lock() {
                            cell.files.insert(prepared.file.clone());
                        }
                        Picture::Stored {
                            file: prepared.file,
                            mime: prepared.mime_type,
                        }
                    }
                    Err(sentence) => Picture::Refused(sentence),
                };
                board.pictures.insert(file, picture);
            }
        }
    }

    /// A posted scene names pictures the app does not know: the page is
    /// asked for them (it posts a `file` for each it has); those it does
    /// not have are missing.
    fn ask_for_board_files(&mut self, id: DocumentId, ids: Vec<String>, cx: &mut Context<Self>) {
        let Some(board) = self.board_of_mut(id) else {
            return;
        };
        for file in &ids {
            board.pictures.entry(file.clone()).or_insert(Picture::Asked);
        }
        let answer = self.board_call(id, calls::FILES, calls::files_argument(&ids));
        cx.spawn(async move |this, cx| {
            let answer = match answer {
                Some(answer) => {
                    let timer = cx.background_executor().timer(boards::VIEW_COLLECT_PATIENCE);
                    crate::pages::within(answer, timer).await
                }
                None => None,
            };
            let missing: Vec<String> = match answer.flatten() {
                Some(Ok(text)) => calls::answer::<calls::FilesPosted>(&text)
                    .map(|posted| posted.missing)
                    .unwrap_or_default(),
                _ => ids.clone(),
            };
            this.update(cx, |this, cx| {
                if let Some(board) = this.board_of_mut(id) {
                    for file in &missing {
                        if matches!(board.pictures.get(file), Some(Picture::Asked) | None) {
                            board.pictures.insert(file.clone(), Picture::Missing);
                        }
                    }
                    // Those the page posted are on their way (`file`); any it
                    // neither posted nor named as missing are missing too.
                    let _ = &ids;
                }
                this.schedule_board_save(id, Duration::ZERO, cx);
            })
            .ok();
        })
        .detach();
    }

    /// A save found the row changed under the view (a write the app did not
    /// take over): the stored scene is taken into the view by the base rule,
    /// and the page posts the merge, which is saved on it.
    fn board_save_was_stale(&mut self, id: DocumentId, cx: &mut Context<Self>) {
        let Ok(stored) = self.library.read(cx).document(id) else {
            return;
        };
        self.adopt_stored_scene(id, &stored.content, true, cx);
    }

    /// Takes the stored scene `content` (written by someone else: an agent,
    /// a restored version, a write the app did not take over) into
    /// whiteboard `id`'s view by the base rule, one step of its undo when
    /// `undoable`. From here until the view answers, its posts are dropped.
    pub(crate) fn adopt_stored_scene(
        &mut self,
        id: DocumentId,
        content: &str,
        undoable: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(board) = self.board_of_mut(id) else {
            return;
        };
        let theirs = match Scene::from_stored(content) {
            Ok(parsed) => parsed.scene,
            Err(error) => {
                eprintln!("Inner Word: the stored whiteboard cannot be read: {error}");
                return;
            }
        };
        let before = Scene::from_stored(&board.base)
            .map(|parsed| parsed.scene)
            .unwrap_or_else(|_| Scene::empty());
        let delta = theirs.delta_from(&before);
        let base_rev = board.base_rev;
        let writer = board.held.clone();
        let stamp = ContentStamp::of(content);
        let files: Vec<String> = theirs
            .stored_files()
            .iter()
            .map(|hex| hex.as_str().to_string())
            .collect();
        board.set_base(stamp, Arc::from(content), &files);
        board.seed_pictures(&theirs);
        board.held = None;
        board.adopting = true;
        if delta.is_empty() {
            board.adopting = false;
            board.floor = board.status.rev;
            return;
        }
        let argument = delta.to_json(base_rev, undoable);
        let answer = self.board_call(id, calls::ADOPT, argument);
        let stored_content: Arc<str> = Arc::from(content);
        cx.spawn(async move |this, cx| {
            let answer = match answer {
                Some(answer) => {
                    let timer = cx.background_executor().timer(boards::ADOPT_PATIENCE);
                    crate::pages::within(answer, timer).await.flatten()
                }
                None => None,
            };
            let adopted = match answer {
                Some(Ok(text)) => calls::answer::<calls::Adopted>(&text).ok(),
                _ => None,
            };
            this.update(cx, |this, cx| match adopted {
                Some(adopted) => this.board_adopted(id, adopted, cx),
                None => this.board_adopt_failed(id, before, writer, stored_content, cx),
            })
            .ok();
        })
        .detach();
    }

    /// Whiteboard `id` was written by someone else (an agent's request, a
    /// write the app did not make): the stored scene is taken into its view
    /// by the base rule, one step of its undo, and from here until the view
    /// answers its posts are dropped (section 4.6, "After an outside write").
    pub fn adopt_outside_write(&mut self, id: DocumentId, cx: &mut Context<Self>) {
        let Ok(stored) = self.library.read(cx).document(id) else {
            return;
        };
        self.adopt_stored_scene(id, &stored.content, true, cx);
    }

    /// The view answered an adopt.
    fn board_adopted(&mut self, id: DocumentId, adopted: calls::Adopted, cx: &mut Context<Self>) {
        let Some(board) = self.board_of_mut(id) else {
            return;
        };
        board.adopting = false;
        board.floor = adopted.rev;
        board.base_rev = adopted.rev;
        if adopted.same {
            board.saved = board.saved.max(adopted.rev);
        }
        // The merge, posted before this answer came.
        let pending = board
            .pending_post
            .take()
            .filter(|held| held.rev >= adopted.rev && held.rev > board.saved);
        if let Some(held) = pending {
            board.held = Some(held);
            self.schedule_board_save(id, Duration::ZERO, cx);
        }
        if !adopted.conflicts.is_empty() {
            eprintln!(
                "Inner Word: the whiteboard kept the writer's copy of {}",
                adopted.conflicts.join(", ")
            );
        }
        cx.notify();
    }

    /// An adopt failed (a script error, a crash, no answer in time): the
    /// merge is made in Rust from the base, the writer's scene and the
    /// stored one, saved on the stored one, and the view reloads from it.
    fn board_adopt_failed(
        &mut self,
        id: DocumentId,
        base: Scene,
        writer: Option<Held>,
        theirs_content: Arc<str>,
        cx: &mut Context<Self>,
    ) {
        let Some(board) = self.board_of_mut(id) else {
            return;
        };
        board.adopting = false;
        board.pending_post = None;
        let theirs = Scene::from_stored(&theirs_content)
            .map(|parsed| parsed.scene)
            .unwrap_or_else(|_| Scene::empty());
        let stored = board.stored_pictures();
        let writer_scene = writer
            .as_ref()
            .and_then(|held| Scene::from_view(&held.json, &|id| stored.get(id).cloned()).ok())
            .map(|parsed| parsed.scene);
        let merged = match writer_scene {
            Some(writer) => innerword_board::merge(&base, &writer, &theirs).scene,
            None => theirs,
        };
        let prepared = super::saver::prepared_of(&merged);
        let expect = board.stamp;
        let (result, _) = self.library.update(cx, |library, cx| {
            library.change_outside(
                |store| {
                    let result = store.save_board(id, &prepared.content(), Some(expect));
                    let changes = if result.is_ok() {
                        vec![OutsideChange::DocumentSaved(id)]
                    } else {
                        Vec::new()
                    };
                    (result, changes)
                },
                cx,
            )
        });
        if let Ok(saved) = result {
            let files: Vec<String> = prepared.files.iter().map(FileId::to_hex).collect();
            if let Some(board) = self.board_of_mut(id) {
                board.set_base(saved.stamp, Arc::from(prepared.scene), &files);
            }
        }
        self.reload_board_view(id, cx);
    }

    /// The view of whiteboard `id` loads the stored scene again.
    fn reload_board_view(&mut self, id: DocumentId, cx: &mut Context<Self>) {
        let Some(page) = self.page.as_mut().filter(|page| page.id == id) else {
            return;
        };
        if let WebItem::Board(board) = &mut page.item {
            board.ready = false;
            board.held = None;
            board.saved = 0;
            board.floor = 0;
        }
        page.state = PageState::Loading;
        if let Some(view) = page.view.borrow().as_ref() {
            view.set_hidden(false);
            view.reload();
        }
        cx.notify();
    }

    /// A save failed: retried after a pause; after three in a row the
    /// drawing also goes to the recovery file.
    fn board_save_failed(&mut self, id: DocumentId, message: String, cx: &mut Context<Self>) {
        let Some(board) = self.board_of_mut(id) else {
            return;
        };
        board.failures_in_a_row += 1;
        let first = board.failure.is_none();
        board.failure = Some(message.clone());
        let third = board.failures_in_a_row >= 3;
        if third {
            self.write_board_recovery(id, &message, cx);
        } else if first {
            self.toast_error(
                format!("Not saved yet. Your drawing is kept and the save is retried. ({message})"),
                cx,
            );
        }
        self.schedule_board_save(id, RETRY_DELAY, cx);
    }

    /// A held scene that cannot be saved (not readable, too large): it goes
    /// to the recovery file, and the save state says so.
    fn board_unsaveable(&mut self, id: DocumentId, why: &str, cx: &mut Context<Self>) {
        let too_large = self
            .board_of(id)
            .and_then(|board| board.held.as_ref())
            .is_some_and(|held| held.json.len() > innerword_board::limits::MAX_VIEW_SCENE_BYTES);
        let reason = if too_large {
            "this whiteboard is too large to keep. Remove something from it".to_string()
        } else {
            why.to_string()
        };
        if let Some(board) = self.board_of_mut(id) {
            board.failure = Some(reason.clone());
        }
        self.write_board_recovery(id, &reason, cx);
    }

    /// Writes the held scene of whiteboard `id` to its recovery file
    /// (`<data directory>/Recovered/whiteboard-<id>-<ms>.json`; a library in
    /// memory keeps it in [`BoardState::recovered`]).
    fn write_board_recovery(&mut self, id: DocumentId, reason: &str, cx: &mut Context<Self>) {
        let Some(board) = self.board_of(id) else {
            return;
        };
        let Some(held) = board.held.clone() else {
            return;
        };
        let now = self.library.read(cx).clock().now_ms();
        let recovered = match self.library.read(cx).path().and_then(boards::recovery_dir) {
            Some(dir) => {
                let path = dir.join(format!("whiteboard-{}-{now}.json", id.0));
                let written = std::fs::create_dir_all(&dir)
                    .and_then(|()| std::fs::write(&path, held.json.as_bytes()));
                match written {
                    Ok(()) => Recovered::File(path),
                    Err(error) => {
                        eprintln!(
                            "Inner Word: the drawing of whiteboard {} could not be written to {}: {error}",
                            id.0,
                            path.display()
                        );
                        return;
                    }
                }
            }
            None => Recovered::Memory(held.json.to_string()),
        };
        let place = match &recovered {
            Recovered::File(path) => crate::export::short_path(path),
            Recovered::Memory(_) => "memory".to_string(),
        };
        let path = match &recovered {
            Recovered::File(path) => Some(path.clone()),
            Recovered::Memory(_) => None,
        };
        if let Some(board) = self.board_of_mut(id) {
            board.recovered = Some(recovered);
        }
        let mut toast = Toast::new(format!(
            "Not saved: {reason}. A copy of the drawing is in {place}."
        ))
        .duration(Duration::from_secs(12));
        if let Some(path) = path {
            toast = toast.on_action("Show in Finder", move |_, cx| cx.reveal_path(&path));
        }
        self.show_toast(toast, cx);
    }

    /// The synchronous save of the open and the closing whiteboard: what the
    /// app holds. Part of [`Workspace::flush`].
    pub(crate) fn flush_boards(&mut self, cx: &mut Context<Self>) -> bool {
        let mut saved = true;
        let ids: Vec<DocumentId> = self
            .open_board_id()
            .into_iter()
            .chain(self.board_closing())
            .collect();
        for id in ids {
            saved &= self.save_board_now(id, cx);
        }
        saved &= self.flush_shape_library(cx);
        saved
    }

    // ----- the shape library ------------------------------------------------------------

    /// The page posted the shape library.
    fn shape_library_posted(
        &mut self,
        lrev: u64,
        items: &str,
        too_large: bool,
        cx: &mut Context<Self>,
    ) {
        if too_large {
            self.shape_library_too_large(cx);
            return;
        }
        if self
            .shape_library
            .held
            .as_ref()
            .is_some_and(|(held, _)| *held >= lrev)
        {
            return;
        }
        self.shape_library.held = Some((lrev, Arc::from(items)));
        self.save_shape_library(cx);
    }

    fn shape_library_too_large(&mut self, cx: &mut Context<Self>) {
        if !self.shape_library.too_large_told {
            self.shape_library.too_large_told = true;
            self.show_toast(
                Toast::new(LIBRARY_TOO_LARGE).duration(Duration::from_secs(10)),
                cx,
            );
        }
    }

    /// Stores the held shape library, in the background when the library is
    /// a file.
    fn save_shape_library(&mut self, cx: &mut Context<Self>) {
        if !self.shape_library.dirty() || self.shape_library.in_flight.is_some() {
            return;
        }
        let Some((lrev, items)) = self.shape_library.held.clone() else {
            return;
        };
        let seq = self.saver.next_seq();
        let Some(work) = self.saver.work_shape_library(seq, items) else {
            self.flush_shape_library(cx);
            return;
        };
        let background = cx.background_spawn(async move { work() });
        self.shape_library.in_flight = Some(cx.spawn(async move |this, cx| {
            let outcome = background.await;
            this.update(cx, |this, cx| {
                this.shape_library.in_flight = None;
                this.shape_library_stored(lrev, outcome, cx);
                if this.shape_library.dirty() {
                    this.save_shape_library(cx);
                }
            })
            .ok();
        }));
    }

    fn shape_library_stored(
        &mut self,
        lrev: u64,
        outcome: Result<Option<String>, String>,
        cx: &mut Context<Self>,
    ) {
        match outcome {
            Ok(Some(stored)) => {
                self.shape_library.saved = self.shape_library.saved.max(lrev);
                self.shape_library.too_large_told = false;
                let stored: Arc<str> = Arc::from(stored);
                self.shape_library.stored = Some(stored.clone());
                if let Some(board) = self.page.as_mut().and_then(|page| match &page.item {
                    WebItem::Board(board) => Some(board),
                    WebItem::Page(_) => None,
                }) && let Ok(mut cell) = board.cell.lock()
                {
                    cell.library = stored;
                }
            }
            Ok(None) => {}
            Err(why) if why == super::saver::TOO_LARGE_LIBRARY => {
                // The last library kept stays; this one is not stored.
                self.shape_library.saved = self.shape_library.saved.max(lrev);
                self.shape_library_too_large(cx);
            }
            Err(why) => {
                self.shape_library.saved = self.shape_library.saved.max(lrev);
                eprintln!("Inner Word: the shape library was not stored: {why}");
            }
        }
    }

    /// Stores the held shape library now, on this thread.
    pub(crate) fn flush_shape_library(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.shape_library.dirty() {
            return true;
        }
        self.shape_library.in_flight = None;
        let Some((lrev, items)) = self.shape_library.held.clone() else {
            return true;
        };
        let outcome = super::saver::shape_library_to_store(&items).and_then(|stored| {
            let raw = serde_json::value::RawValue::from_string(stored.clone())
                .map_err(|error| error.to_string())?;
            self.library
                .update(cx, |library, cx| {
                    library.set_setting(boards::SHAPE_LIBRARY_SETTING, &raw, cx)
                })
                .map_err(|error| error.to_string())?;
            Ok(Some(stored))
        });
        let ok = outcome.is_ok();
        self.shape_library_stored(lrev, outcome, cx);
        ok
    }

    // ----- calls ------------------------------------------------------------------------

    /// Runs the glue's call `body` with `argument` in whiteboard `id`'s
    /// view (the open one, or the one closing); `None` when it has none.
    pub(crate) fn board_call(
        &self,
        id: DocumentId,
        body: &'static str,
        argument: String,
    ) -> Option<pages::Answer<Result<String, String>>> {
        let session = self.board_session(id)?;
        let view = session.view.borrow();
        let view = view.as_ref()?;
        let (tx, rx) = pages::answer();
        view.call(body, &argument, move |result| tx.send(result));
        Some(rx)
    }

    /// A call on the open whiteboard whose answer nobody waits for.
    pub(crate) fn board_call_and_forget(&self, body: &'static str, argument: String) {
        if let Some(id) = self.open_board_id() {
            drop(self.board_call(id, body, argument));
        }
    }

    /// Asks whiteboard `id`'s page for what it has not posted yet (at most
    /// [`boards::VIEW_COLLECT_PATIENCE`]), takes it in and saves; the task
    /// resolves to whether everything was saved.
    pub fn collect_board(&mut self, id: DocumentId, cx: &mut Context<Self>) -> Task<bool> {
        let answer = self.board_call(id, calls::COLLECT, "{}".to_string());
        if let Some(board) = self.board_of_mut(id) {
            board.collecting += 1;
        }
        cx.spawn(async move |this, cx| {
            let started = Instant::now();
            let answer = match answer {
                Some(answer) => {
                    let timer = cx.background_executor().timer(boards::VIEW_COLLECT_PATIENCE);
                    crate::pages::within(answer, timer).await.flatten()
                }
                None => None,
            };
            let parts_of = this
                .update(cx, |this, cx| this.take_collected(id, answer, cx))
                .ok()
                .flatten();
            // A scene in parts: they were posted before the answer and come
            // through the inbox.
            if let Some(rev) = parts_of {
                loop {
                    let held = this
                        .update(cx, |this, _| {
                            this.board_of(id)
                                .and_then(|board| board.held.as_ref().map(|held| held.rev))
                        })
                        .ok()
                        .flatten();
                    if held.is_some_and(|held| held >= rev)
                        || started.elapsed() > boards::VIEW_COLLECT_PATIENCE * 2
                    {
                        break;
                    }
                    cx.background_executor().timer(PICTURE_POLL).await;
                }
            }
            // Pictures still being stored are waited for.
            let waited = Instant::now();
            loop {
                let pending = this
                    .update(cx, |this, _| {
                        this.board_of(id)
                            .is_some_and(|board| board.pictures_pending())
                    })
                    .unwrap_or(false);
                if !pending || waited.elapsed() > PICTURE_PATIENCE {
                    break;
                }
                cx.background_executor().timer(PICTURE_POLL).await;
            }
            this.update(cx, |this, cx| {
                if let Some(board) = this.board_of_mut(id) {
                    board.collecting = board.collecting.saturating_sub(1);
                }
                let saved = this.save_board_now(id, cx);
                let library = this.flush_shape_library(cx);
                saved && library
            })
            .unwrap_or(false)
        })
    }

    /// Takes in what a `collect` answered. Returns the rev of a scene that
    /// comes in parts.
    fn take_collected(
        &mut self,
        id: DocumentId,
        answer: Option<Result<String, String>>,
        cx: &mut Context<Self>,
    ) -> Option<u64> {
        let text = match answer {
            Some(Ok(text)) => text,
            Some(Err(error)) => {
                eprintln!("Inner Word: the whiteboard did not hand over its scene: {error}");
                return None;
            }
            None => return None,
        };
        let collected = match calls::answer::<calls::Collected>(&text) {
            Ok(collected) => collected,
            Err(error) => {
                eprintln!("Inner Word: {error}");
                return None;
            }
        };
        for file in collected.files {
            self.board_file_posted(id, file.id, Arc::from(file.data_url), cx);
        }
        if let Some(library) = collected.library {
            self.shape_library_posted(library.lrev, library.items.get(), false, cx);
        }
        if let Some(board) = self.board_of_mut(id) {
            board.status = collected.status;
        }
        match collected.scene {
            calls::CollectedScene::Whole(scene) => {
                self.board_scene_posted(id, collected.rev, Arc::from(scene.get()), cx);
                None
            }
            calls::CollectedScene::Parts => Some(collected.rev),
            calls::CollectedScene::None => None,
        }
    }

    /// Resolves once the open whiteboard's page posted `ready` (at once
    /// when it has); `None` when no whiteboard is open.
    pub fn board_ready_answer(&mut self) -> Option<pages::Answer<()>> {
        let page = self.page.as_mut()?;
        let WebItem::Board(board) = &mut page.item else {
            return None;
        };
        let (tx, rx) = pages::answer();
        if board.ready {
            tx.send(());
        } else {
            board.ready_waiters.push(tx);
        }
        Some(rx)
    }

    // ----- the end of a session --------------------------------------------------------

    /// The session of a whiteboard ends (another item opened, the whiteboard
    /// closed, trashed or deleted). When its page holds more than it posted,
    /// the view is parked and asked for it, and the session goes only once
    /// that is saved; otherwise what the app holds is saved at once.
    pub(crate) fn end_board_session(
        &mut self,
        mut session: PageSession,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = session.id;
        // A whiteboard still closing is finished first: one parked view at a
        // time.
        self.finish_closing_board_now(window, cx);
        let holds_more = match &session.item {
            WebItem::Board(board) => {
                (board.ready && board.status.holds_more()) || board.status.too_large
            }
            WebItem::Page(_) => false,
        };
        let has_view = session.view.borrow().is_some()
            && !matches!(session.state, PageState::Crashed { again: true, .. });
        session.release_pictures(window);
        if holds_more && has_view {
            if let Some(view) = session.view.borrow().as_ref() {
                view.park();
            }
            self.closing_board = Some(ClosingBoard {
                session,
                then_open: None,
                _task: None,
            });
            let collect = self.collect_board(id, cx);
            let task = cx.spawn_in(window, async move |this, cx| {
                collect.await;
                this.update_in(cx, |this, window, cx| this.closing_board_done(id, window, cx))
                    .ok();
            });
            if let Some(closing) = self.closing_board.as_mut() {
                closing._task = Some(task);
            }
            return;
        }
        // Nothing outstanding: as a page's session, after saving what is held
        // (the session is the closing one for the moment of the save).
        self.closing_board = Some(ClosingBoard {
            session,
            then_open: None,
            _task: None,
        });
        self.save_board_now(id, cx);
        self.closing_board = None;
    }

    /// The closing whiteboard's last scene is saved: its view goes, and the
    /// item opened meanwhile (the same whiteboard again) opens.
    fn closing_board_done(&mut self, id: DocumentId, window: &mut Window, cx: &mut Context<Self>) {
        if self.board_closing() != Some(id) {
            return;
        }
        let Some(mut closing) = self.closing_board.take() else {
            return;
        };
        closing.session.release_pictures(window);
        let then_open = closing.then_open.take();
        drop(closing);
        if let Some((open, focus)) = then_open {
            self.open_document(open, focus, window, cx);
        }
        cx.notify();
    }

    /// Ends a whiteboard that is closing at once, with what the app holds of
    /// it (another end comes before its collect answered).
    pub(crate) fn finish_closing_board_now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.board_closing() else {
            return;
        };
        self.save_board_now(id, cx);
        self.closing_board_done(id, window, cx);
    }

    /// Whether a whiteboard is closing or the open one's page holds more
    /// than it posted (a deliberate end then asks it first).
    pub fn board_holds_more(&self) -> bool {
        self.closing_board.is_some()
            || self
                .board()
                .is_some_and(|board| board.ready && board.status.holds_more())
    }

    // ----- commands --------------------------------------------------------------------

    /// The Edit menu on a whiteboard with the keyboard: Undo, Redo and Select
    /// All go to the glue, Copy, Cut and Paste to the view (the page gets
    /// its clipboard events, which Excalidraw handles).
    pub(crate) fn edit_on_board(&mut self, command: EditCommand, window: &Window) -> bool {
        let Some(page) = self.page.as_ref() else {
            return false;
        };
        let WebItem::Board(_) = &page.item else {
            return false;
        };
        if !page.focus.is_focused(window) {
            return false;
        }
        let id = page.id;
        match command {
            EditCommand::Undo => drop(self.board_call(id, calls::UNDO, calls::steps_argument(1))),
            EditCommand::Redo => drop(self.board_call(id, calls::REDO, calls::steps_argument(1))),
            EditCommand::SelectAll => drop(self.board_call(id, calls::SELECT_ALL, "{}".into())),
            EditCommand::Copy | EditCommand::Cut | EditCommand::Paste => {
                let web = match command {
                    EditCommand::Copy => innerword_web::EditCommand::Copy,
                    EditCommand::Cut => innerword_web::EditCommand::Cut,
                    _ => innerword_web::EditCommand::Paste,
                };
                if let Some(view) = page.view.borrow().as_ref() {
                    view.edit(web);
                }
            }
        }
        true
    }

    /// One of the six Add commands: a new element made from `skeleton` at
    /// the middle of the view, selected (a text: open for typing), made by
    /// the converter agents use, against the scene the app holds.
    pub(crate) fn add_to_board(
        &mut self,
        skeleton: serde_json::Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.open_board_id() else {
            return;
        };
        if !self.board_ready() {
            self.say_about_page(STILL_LOADING, cx);
            return;
        }
        let center = self.board_call(id, calls::CENTER, "{}".to_string());
        let kind = skeleton
            .get("type")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();
        cx.spawn_in(window, async move |this, cx| {
            let center = match center {
                Some(center) => center.await.and_then(Result::ok),
                None => None,
            };
            let center = center.and_then(|text| calls::answer::<calls::Center>(&text).ok());
            this.update_in(cx, |this, window, cx| {
                let (x, y, selected) = match center {
                    Some(center) => (center.x, center.y, center.selected),
                    None => (0.0, 0.0, Vec::new()),
                };
                let skeleton = place_skeleton(skeleton, &kind, x, y, &selected);
                if let Err(why) = this.draw_on_board(vec![skeleton], kind == "text", window, cx) {
                    this.say_about_page(&format!("Nothing was added: {why}"), cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Converts `skeletons` with the converter agents use, against the scene
    /// the app holds, and gives them to the page as the writer's drawing
    /// (`insertElements`, selected; with `edit_text` the first text opens
    /// for typing). Returns the ids made, or why nothing was.
    pub fn draw_on_board(
        &mut self,
        skeletons: Vec<serde_json::Value>,
        edit_text: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Vec<String>, String> {
        let Some(id) = self.open_board_id() else {
            return Err("no whiteboard is open".to_string());
        };
        let mut scene = self.board_scene().unwrap_or_else(Scene::empty);
        let before: HashMap<String, i64> = scene
            .elements()
            .iter()
            .map(|element| (element.id().to_string(), element.version()))
            .collect();
        let skeletons: Vec<Skeleton> = skeletons
            .into_iter()
            .filter_map(|value| match value {
                serde_json::Value::Object(map) => Some(Skeleton(map)),
                _ => None,
            })
            .collect();
        let now = self.library.read(cx).clock().now_ms();
        let (stamp, rev) = match self.board() {
            Some(board) => (board.stamp.to_hex(), board.status.rev),
            None => (String::new(), 0),
        };
        let skeleton_json = serde_json::to_string(&skeletons).unwrap_or_default();
        let mut making = innerword_board::HashedMaking::new(
            &[
                stamp.as_bytes(),
                &rev.to_le_bytes(),
                &now.to_le_bytes(),
                skeleton_json.as_bytes(),
            ],
            now,
        );
        let added = innerword_board::add(
            &mut scene,
            &skeletons,
            &innerword_board::Pictures::new(),
            &mut making,
        )
        .map_err(|error| error.to_string())?;
        // The new elements, and those the change touched (a shape an arrow
        // binds now lists it).
        let changed: Vec<serde_json::Value> = scene
            .elements()
            .iter()
            .filter(|element| before.get(element.id()) != Some(&element.version()))
            .map(|element| serde_json::Value::Object(element.json().clone()))
            .collect();
        let ids: Vec<String> = added
            .elements
            .iter()
            .map(|element| element.id.clone())
            .collect();
        let elements = serde_json::to_string(&changed).unwrap_or_else(|_| "[]".to_string());
        let answer = self.board_call(
            id,
            calls::INSERT_ELEMENTS,
            calls::insert_elements_argument(&elements, true, None),
        );
        // A new text opens for typing as `enter` on it does. The glue's own
        // `editText` sets the selection and presses `enter` in one task,
        // before Excalidraw rendered the selection (its `enter` then opens
        // what was selected before): so the app presses `enter` itself once
        // the new text is selected on screen.
        if edit_text
            && added.elements.iter().any(|element| element.r#type == "text")
            && let Some(view) = self.board_native_view()
            && let Some(answer) = answer
        {
            let view = view as usize;
            cx.spawn(async move |_, cx| {
                answer.await;
                cx.background_executor()
                    .timer(Duration::from_millis(80))
                    .await;
                cx.update(|_| {
                    crate::board_steps::press(view as *mut std::ffi::c_void, "enter").ok();
                })
                .ok();
            })
            .detach();
        }
        Ok(ids)
    }

    /// "Mermaid to drawing…": Excalidraw's dialog; with `definition` (the
    /// scripted step) converts it as the dialog's Insert does.
    pub(crate) fn mermaid_on_board(&mut self, definition: Option<&str>) {
        self.board_call_and_forget(calls::MERMAID, calls::mermaid_argument(definition));
    }

    /// The zoom commands on a whiteboard: Excalidraw's canvas zoom.
    pub(crate) fn zoom_board(&mut self, zoom: calls::Zoom) {
        self.board_call_and_forget(calls::ZOOM, calls::zoom_argument(zoom));
    }

    /// Find on a whiteboard: Excalidraw's own search, with `query`.
    pub(crate) fn search_board(&mut self, query: Option<&str>) {
        self.board_call_and_forget(calls::SEARCH, calls::search_argument(query));
    }

    /// The shape library's call.
    pub(crate) fn shape_library_call(&mut self, action: calls::LibraryAction, cx: &mut Context<Self>) {
        let Some(id) = self.open_board_id() else {
            return;
        };
        let answer = self.board_call(id, calls::LIBRARY, calls::library_argument(&action));
        cx.spawn(async move |this, cx| {
            let Some(answer) = answer else {
                return;
            };
            let Some(Ok(text)) = answer.await else {
                return;
            };
            if let Ok(answered) = calls::answer::<calls::LibraryAnswered>(&text)
                && let Some(refused) = answered.refused
            {
                this.update(cx, |this, cx| this.say_about_page(&refused, cx))
                    .ok();
            }
        })
        .detach();
    }

    /// "Reload whiteboard": what the page holds is collected and saved, then
    /// the view loads the stored scene again.
    pub(crate) fn reload_board(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.open_board_id() else {
            return;
        };
        let collect = self.collect_board(id, cx);
        cx.spawn(async move |this, cx| {
            collect.await;
            this.update(cx, |this, cx| {
                this.reload_board_view(id, cx);
                this.show_toast(Toast::new(RELOADED).duration(Duration::from_secs(6)), cx);
            })
            .ok();
        })
        .detach();
    }

    /// The WebContent process of the open whiteboard ended: what the app
    /// holds is saved at once, then the view reloads from the stored scene
    /// (once; a second time within 10 s it shows that it stopped working).
    pub(crate) fn board_crashed(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.open_board_id() else {
            return;
        };
        self.save_board_now(id, cx);
        if let Some(board) = self.board_of_mut(id) {
            board.ready = false;
            board.held = None;
            board.saved = 0;
            board.floor = 0;
        }
    }

    /// The app's list of keyboard shortcuts, asked for by the page (its `?`).
    /// It opens nothing else: the bridge can ask for no other window.
    pub(crate) fn open_shortcuts_on_a_whiteboard(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(list) = self.show_shortcuts(window, cx) {
            list.update(cx, |list, cx| list.set_query("On a whiteboard", cx));
        }
    }

    /// The app's menu of the open whiteboard (`ctrl-enter`), over a picture
    /// of it.
    pub(crate) fn open_board_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(page) = self.page.as_ref() else {
            return;
        };
        let origin = page
            .bounds
            .get()
            .map(|bounds| bounds.origin + gpui::point(px(8.), px(8.)))
            .unwrap_or_default();
        let focus = page.focus.clone();
        let mut items = vec![
            ContextMenuItem::action("Undo", innerword_editor::actions::Undo),
            ContextMenuItem::action("Redo", innerword_editor::actions::Redo),
            ContextMenuItem::action("Select All", innerword_editor::actions::SelectAll),
            ContextMenuItem::separator(),
        ];
        items.extend(board_add_items());
        items.push(ContextMenuItem::separator());
        items.extend(board_title_items(self.assistant.shown, self.can_print(cx)));
        ContextMenu::open_for(&focus, origin, items, window, cx);
    }

    /// One line about the open whiteboard, for the `state` step, or `none`.
    pub fn describe_board(&self, window: &Window, cx: &App) -> String {
        let (Some(page), Some(board)) = (self.page.as_ref(), self.board()) else {
            return "none".to_string();
        };
        let state = match &page.state {
            _ if matches!(page.state, PageState::Crashed { again: true, .. }) => {
                "stopped".to_string()
            }
            PageState::Failed(message) => format!("failed:{message:?}"),
            PageState::Crashed { .. } => "crashed".to_string(),
            _ if board.ready => "loaded".to_string(),
            _ => "loading".to_string(),
        };
        let overlay = match &page.overlay {
            super::page::Overlay::None => "none".to_string(),
            super::page::Overlay::Waiting => "waiting".to_string(),
            super::page::Overlay::Snapshot(_) => "snapshot".to_string(),
            super::page::Overlay::Holes(holes) => format!("holes:{}", holes.len()),
        };
        let focus = if page.focus.is_focused(window) {
            "board"
        } else if page.focus.contains_focused(window, cx) {
            "inside"
        } else {
            "elsewhere"
        };
        let responder = match page.view.borrow().as_ref() {
            Some(view) if view.is_focused() => "web",
            Some(_) => "gpui",
            None => "none",
        };
        let stored = board
            .pictures
            .values()
            .filter(|picture| matches!(picture, Picture::Stored { .. }))
            .count();
        let library = match &self.shape_library.held {
            Some((_, items)) => {
                let count = serde_json::from_str::<Vec<serde_json::Value>>(items)
                    .map(|items| items.len())
                    .unwrap_or(0);
                let saved = if self.shape_library.dirty() {
                    "held"
                } else {
                    "saved"
                };
                format!("{count}/{saved}")
            }
            None => {
                let count = self
                    .shape_library
                    .stored
                    .as_ref()
                    .and_then(|stored| innerword_board::ShapeLibrary::from_stored(stored).ok())
                    .map(|parsed| parsed.library.items().len())
                    .unwrap_or(0);
                format!("{count}/saved")
            }
        };
        format!(
            "id={} state={state} elements={} rev={} posted={} saved={} dirty={} base={} \
             adopting={} editing={} composing={} pointer={} zoom={}% pictures={stored}/{} \
             recovered={} library={library} overlay={overlay} focus={focus} \
             responder={responder} closing={}",
            page.id.0,
            board.status.elements,
            board.status.rev,
            board.status.posted,
            board.saved,
            if board.dirty() { "yes" } else { "no" },
            &board.stamp.to_hex()[..8],
            if board.adopting { "yes" } else { "no" },
            if board.status.editing { "yes" } else { "no" },
            if board.status.composing { "yes" } else { "no" },
            if board.status.pointer_down { "down" } else { "up" },
            (board.status.zoom * 100.0).round() as i64,
            board.pictures.len(),
            if board.recovered.is_some() { "yes" } else { "no" },
            if self.closing_board.is_some() {
                "yes"
            } else {
                "no"
            },
        )
    }

    /// The save state of the open whiteboard, as the status bar says it.
    pub(crate) fn board_save_label(&self) -> Option<(String, bool)> {
        let board = self.board()?;
        if let Some(failure) = &board.failure {
            let text = match &board.recovered {
                Some(Recovered::File(path)) => format!(
                    "Not saved: {failure}. A copy of the drawing is in {}.",
                    crate::export::short_path(path)
                ),
                _ => format!("Not saved: {failure}"),
            };
            return Some((text, true));
        }
        if board.in_flight.is_some() || board.collecting > 0 {
            return Some(("Saving…".to_string(), false));
        }
        if board.dirty() || board.pictures_pending() {
            return Some(("Edited".to_string(), false));
        }
        Some(("Saved".to_string(), false))
    }

    /// The element count of the open whiteboard, as the status bar says it.
    pub(crate) fn board_elements_label(&self) -> Option<String> {
        let board = self.board()?;
        Some(super::counted(
            board.status.elements as usize,
            "element",
            "elements",
        ))
    }

    /// The element of an open whiteboard without a page engine: its listing
    /// in a quiet column.
    pub(crate) fn board_listing_text(&self, cx: &App) -> String {
        let Some(page) = self.page.as_ref() else {
            return String::new();
        };
        let scene = self.board_scene().unwrap_or_else(Scene::empty);
        let reference = format!("board:{}", page.id.0);
        let _ = cx;
        scene.listing(&innerword_board::ListingHeader {
            reference: &reference,
            title: &page.title,
            folder: None,
        })
    }
}

/// The six Add items, for the menus of a whiteboard.
pub(crate) fn board_add_items() -> Vec<ContextMenuItem> {
    vec![
        ContextMenuItem::action("Add Text", AddText),
        ContextMenuItem::action("Add Rectangle", AddRectangle),
        ContextMenuItem::action("Add Ellipse", AddEllipse),
        ContextMenuItem::action("Add Diamond", AddDiamond),
        ContextMenuItem::action("Add Arrow", AddArrow),
        ContextMenuItem::action("Add Frame", AddFrame),
        ContextMenuItem::action("Add from Mermaid…", MermaidToDrawing),
        ContextMenuItem::action("Add from Shape Library…", InsertFromShapeLibrary),
    ]
}

/// The title menu's items of a whiteboard after Add.
pub(crate) fn board_title_items(assistant: bool, print: bool) -> Vec<ContextMenuItem> {
    use crate::actions::*;
    let mut items = vec![
        ContextMenuItem::action("Shape Library", ToggleShapeLibrary),
        ContextMenuItem::action("Find on Whiteboard…", tools::Find),
        ContextMenuItem::action("Insert Picture…", innerword_editor::actions::InsertImage),
        ContextMenuItem::action("Zoom to Fit", crate::zoom::FitPage),
        ContextMenuItem::separator(),
    ];
    if print {
        items.push(ContextMenuItem::action("Print…", Print));
    }
    items.extend([
        ContextMenuItem::action("Export as PDF…", ExportPdf),
        ContextMenuItem::action("Version History…", ShowVersionHistory),
        ContextMenuItem::action("Reveal in Library", RevealInLibrary),
    ]);
    if assistant {
        items.push(ContextMenuItem::action("Assistant", ToggleAssistant));
    }
    items
}

/// A skeleton of an Add command at `(x, y)`, the middle of the view: a shape
/// centred there; an arrow between the two selected shapes, else a short
/// one; a frame around the selection, else an empty one.
fn place_skeleton(
    mut skeleton: serde_json::Value,
    kind: &str,
    x: f64,
    y: f64,
    selected: &[String],
) -> serde_json::Value {
    use serde_json::json;
    let Some(map) = skeleton.as_object_mut() else {
        return skeleton;
    };
    match kind {
        "arrow" if selected.len() == 2 => {
            map.insert("start".into(), json!({ "id": selected[0] }));
            map.insert("end".into(), json!({ "id": selected[1] }));
        }
        "arrow" => {
            map.insert("x".into(), json!(x - 100.0));
            map.insert("y".into(), json!(y));
            map.insert("width".into(), json!(200.0));
            map.insert("height".into(), json!(0.0));
        }
        "frame" if !selected.is_empty() => {
            map.insert("children".into(), json!(selected));
        }
        "frame" => {
            map.insert("x".into(), json!(x - 200.0));
            map.insert("y".into(), json!(y - 150.0));
            map.insert("width".into(), json!(400.0));
            map.insert("height".into(), json!(300.0));
        }
        "text" => {
            map.insert("x".into(), json!(x - 40.0));
            map.insert("y".into(), json!(y - 12.0));
        }
        _ => {
            let width = map.get("width").and_then(serde_json::Value::as_f64).unwrap_or(160.0);
            let height = map
                .get("height")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(100.0);
            map.insert("x".into(), json!(x - width / 2.0));
            map.insert("y".into(), json!(y - height / 2.0));
            map.insert("width".into(), json!(width));
            map.insert("height".into(), json!(height));
        }
    }
    skeleton
}

/// The skeleton each Add command starts from.
pub(crate) fn add_skeleton(action: &dyn Action) -> Option<serde_json::Value> {
    use serde_json::json;
    let name = action.name();
    let skeleton = if name == AddText.name() {
        json!({ "type": "text", "text": "Text" })
    } else if name == AddRectangle.name() {
        json!({ "type": "rectangle" })
    } else if name == AddEllipse.name() {
        json!({ "type": "ellipse" })
    } else if name == AddDiamond.name() {
        json!({ "type": "diamond" })
    } else if name == AddArrow.name() {
        json!({ "type": "arrow" })
    } else if name == AddFrame.name() {
        json!({ "type": "frame", "name": "Frame" })
    } else {
        return None;
    };
    Some(skeleton)
}

/// What the element of an open whiteboard draws where the view is not (the
/// fake engine, no engine): its listing.
pub(crate) fn render_listing(text: String, why: Option<String>, cx: &App) -> AnyElement {
    use innerword_ui::theme::{ActiveTheme, SPACE_3, SPACE_4, UI_TEXT_SM};
    let colors = cx.theme().colors;
    let shown: String = text.chars().take(20_000).collect();
    div()
        .id("board-listing")
        .size_full()
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .items_center()
        .child(
            div()
                .w(px(640.))
                .max_w_full()
                .py(SPACE_4)
                .px(SPACE_4)
                .flex()
                .flex_col()
                .gap(SPACE_3)
                .children(why.map(|why| {
                    div()
                        .text_size(UI_TEXT_SM)
                        .text_color(colors.faint)
                        .child(why)
                }))
                .child(
                    div()
                        .text_size(UI_TEXT_SM)
                        .text_color(colors.muted)
                        .whitespace_normal()
                        .child(shown),
                ),
        )
        .into_any_element()
}

/// Makes the closure the fake view's events use to post messages through
/// the inbox (kept here so `pages.rs` does not know the bridge's types).
pub(crate) fn post_board_message(inbox: &pages::SharedInbox, body: String) {
    match innerword_board_web::bridge::parse(&body) {
        Ok(message) => pages::post(inbox, pages::PageEvent::Board(message)),
        Err(error) => eprintln!("Inner Word: a message of the whiteboard was dropped: {error}"),
    }
}

impl PageViewHandle {
    /// Parks the view of a closing whiteboard (out of sight, still drawn).
    pub(crate) fn park(&self) {
        match self {
            PageViewHandle::WebKit(view) => view.park(),
            PageViewHandle::Fake(view) => view.park(),
        }
    }
}

impl Workspace {
    /// Puts whiteboard `id` in the window: its view's source over the
    /// library and the bundle (WebKit), or the fake engine's canvas.
    #[allow(clippy::too_many_arguments)]
    fn install_board_session(
        &mut self,
        id: DocumentId,
        stored: &innerword_library::Document,
        mut board: Box<BoardState>,
        cell: SharedCell,
        library_json: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let focus = self.leave_for_web_item(window, cx);
        let (inbox, keys, events) = super::page::web_plumbing(pages::PageKeys::for_board());
        let theme = super::page::theme_for(self.settings.theme, cx);
        let host = boards::board_host(id, stored.meta.created_at);
        let (seed, state) = match pages::engine(cx) {
            Some(pages::PageEngine::WebKit) => match boards::assets(cx) {
                Ok(assets) => {
                    let source = Arc::new(boards::LibraryBoardSource::new(
                        assets,
                        self.library.read(cx),
                        cell.clone(),
                    ));
                    board.source = Some(source.clone());
                    let source: Arc<dyn innerword_web::PageSource> = source;
                    let config = innerword_web::ViewConfig {
                        host,
                        entry: innerword_board_web::ENTRY.to_string(),
                        source,
                        rules_dir: self.web_rules_dir(cx),
                        appearance: theme.0,
                        background: theme.1,
                    };
                    let extras = innerword_web::ViewExtras {
                        bridge: Some(innerword_web::Bridge {
                            name: innerword_board_web::BRIDGE_NAME,
                            max_bytes: innerword_board_web::bridge::MAX_MESSAGE_BYTES,
                        }),
                        one_document: true,
                        csp: Some(innerword_web::BOARD_CONTENT_SECURITY_POLICY),
                    };
                    (
                        Some(super::page::ViewSeed::WebKit(config, extras, events)),
                        PageState::Loading,
                    )
                }
                Err(why) => (None, PageState::Failed(why)),
            },
            Some(pages::PageEngine::Fake(fake)) => {
                let board_json = cell
                    .lock()
                    .map(|cell| cell.board_json())
                    .unwrap_or_default();
                (
                    Some(super::page::ViewSeed::Fake(pages::FakeView::new_board(
                        fake,
                        id,
                        &host,
                        &board_json,
                        library_json,
                        events,
                        inbox.clone(),
                    ))),
                    PageState::Loading,
                )
            }
            None => (None, PageState::Failed(boards::not_shown_sentence(cx))),
        };
        self.install_web_session(
            super::page::WebSessionParts {
                id,
                title: stored.meta.title.clone(),
                item: WebItem::Board(board),
                seed,
                state,
                focus,
                inbox,
                keys,
                theme,
            },
            window,
            cx,
        );
        true
    }

    // ----- pictures from files -----------------------------------------------------------

    /// Files dropped on the open whiteboard: pictures go onto the canvas
    /// (where they were dropped), web pages are added as pages, anything
    /// else is left out with a toast.
    pub(crate) fn files_dropped_on_board(
        &mut self,
        paths: Vec<std::path::PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (web_pages, pictures): (Vec<_>, Vec<_>) =
            paths.into_iter().partition(|path| pages::is_page_input(path));
        if !web_pages.is_empty() {
            let folder = self.panel.read(cx).selected_folder();
            self.import_dropped(web_pages, folder, window, cx);
        }
        if !pictures.is_empty() {
            let at = window.mouse_position();
            let bounds = self.page.as_ref().and_then(|page| page.bounds.get());
            let at = bounds.map(|bounds| {
                (
                    f64::from(f32::from(at.x - bounds.origin.x)),
                    f64::from(f32::from(at.y - bounds.origin.y)),
                )
            });
            self.insert_board_pictures(pictures, at, cx).detach();
        }
    }

    /// Insert > Image… on a whiteboard: the files chosen (the open panel, or
    /// what a test named) go to the middle of the view.
    pub(crate) fn choose_board_pictures(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use innerword_editor::ImageProvider as _;
        let chosen = self.images.choose(window, cx);
        cx.spawn(async move |this, cx| {
            let Some(paths) = chosen.await else {
                return;
            };
            this.update(cx, |this, cx| this.insert_board_pictures(paths, None, cx).detach())
                .ok();
        })
        .detach();
    }

    /// Reads, makes and stores the pictures at `paths` in the background,
    /// then puts them on the canvas (at `at`, a point of the view, or its
    /// middle), selected, each at its own size up to 1,440. A file that is no
    /// picture the library takes is said in a toast.
    pub fn insert_board_pictures(
        &mut self,
        paths: Vec<std::path::PathBuf>,
        at: Option<(f64, f64)>,
        cx: &mut Context<Self>,
    ) -> Task<Vec<String>> {
        let Some(id) = self.open_board_id() else {
            return Task::ready(Vec::new());
        };
        let images = self.images.clone();
        cx.spawn(async move |this, cx| {
            let prepared: Vec<Result<boards::PreparedPicture, String>> = cx
                .background_spawn(async move {
                    paths
                        .iter()
                        .map(|path| {
                            let name = path
                                .file_name()
                                .map(|name| format!("“{}”", name.to_string_lossy()))
                                .unwrap_or_else(|| "The file".to_string());
                            let bytes = std::fs::read(path).map_err(|error| {
                                format!("{name} could not be read ({error}).")
                            })?;
                            boards::prepare_picture(&bytes, &name)
                        })
                        .collect()
                })
                .await;
            let mut refused = Vec::new();
            let mut ready = Vec::new();
            for picture in prepared {
                match picture {
                    Ok(picture) => ready.push(picture),
                    Err(sentence) => refused.push(sentence),
                }
            }
            let blobs: Vec<(String, Vec<u8>)> = ready
                .iter()
                .map(|picture| (picture.file.clone(), picture.bytes.clone()))
                .collect();
            let stored = match cx.update(|cx| images.put_files(blobs, cx)) {
                Ok(task) => task.await,
                Err(_) => return Vec::new(),
            };
            let answer = this
                .update(cx, |this, cx| {
                    for sentence in refused.iter().take(3) {
                        this.show_toast(
                            Toast::new(sentence.clone()).duration(Duration::from_secs(8)),
                            cx,
                        );
                    }
                    if refused.len() > 3 {
                        this.show_toast(
                            Toast::new(crate::images::more_refused(refused.len() - 3)),
                            cx,
                        );
                    }
                    if let Err(error) = stored {
                        this.toast_error(format!("The pictures could not be stored: {error}"), cx);
                        return None;
                    }
                    if ready.is_empty() {
                        return None;
                    }
                    let board = this.board_of_mut(id)?;
                    let mut inserted = Vec::new();
                    for picture in &ready {
                        if let Ok(mut cell) = board.cell.lock() {
                            cell.files.insert(picture.file.clone());
                        }
                        if let Some(source) = &board.source {
                            source.remember(&picture.file, &picture.bytes);
                        }
                        board.pictures.insert(
                            picture.file.clone(),
                            Picture::Stored {
                                file: picture.file.clone(),
                                mime: picture.mime_type.clone(),
                            },
                        );
                        let (width, height) = boards::drawn_size(picture.pixels, picture.density);
                        inserted.push(calls::PictureToInsert {
                            file: picture.file.clone(),
                            mime_type: picture.mime_type.clone(),
                            data_url: boards::data_url_of(&picture.mime_type, &picture.bytes),
                            width,
                            height,
                        });
                    }
                    this.board_call(
                        id,
                        calls::INSERT_PICTURES,
                        calls::insert_pictures_argument(&inserted, at),
                    )
                })
                .ok()
                .flatten();
            match answer {
                Some(answer) => answer
                    .await
                    .and_then(Result::ok)
                    .and_then(|text| calls::answer::<calls::Inserted>(&text).ok())
                    .map(|inserted| inserted.ids)
                    .unwrap_or_default(),
                None => Vec::new(),
            }
        })
    }

    // ----- the commands of an open whiteboard on the root --------------------------------

    /// The commands of an open whiteboard on the workspace root, and the
    /// commands of a document that would do something else on a whiteboard:
    /// they say what a whiteboard is, in one toast, instead (captured).
    pub(crate) fn board_actions(
        &self,
        root: gpui::Stateful<gpui::Div>,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        use crate::actions::*;
        use innerword_comments as comments;
        fn say<A: Action>(
            root: gpui::Stateful<gpui::Div>,
            sentence: &'static str,
            cx: &mut Context<Workspace>,
        ) -> gpui::Stateful<gpui::Div> {
            root.capture_action(cx.listener(move |this, _: &A, _, cx| {
                cx.stop_propagation();
                this.say_about_page(sentence, cx);
            }))
        }
        fn add<A: Action + Default>(
            root: gpui::Stateful<gpui::Div>,
            cx: &mut Context<Workspace>,
        ) -> gpui::Stateful<gpui::Div> {
            root.on_action(cx.listener(|this, action: &A, window, cx| {
                if let Some(skeleton) = add_skeleton(action) {
                    this.add_to_board(skeleton, window, cx);
                }
            }))
        }
        let mut root = root
            .on_action(cx.listener(|this, _: &ReloadPage, _, cx| this.reload_board(cx)))
            .on_action(cx.listener(|this, _: &SaveNow, _, cx| {
                if let Some(id) = this.open_board_id() {
                    this.collect_board(id, cx).detach();
                }
            }))
            .on_action(cx.listener(|this, _: &tools::Find, _, _| this.search_board(None)))
            .on_action(cx.listener(|this, _: &MermaidToDrawing, _, _| {
                this.mermaid_on_board(None)
            }))
            .on_action(cx.listener(|this, _: &ToggleShapeLibrary, _, cx| {
                this.shape_library_call(calls::LibraryAction::Toggle, cx)
            }))
            .on_action(cx.listener(|this, _: &AddToShapeLibrary, _, cx| {
                this.shape_library_call(calls::LibraryAction::AddSelection, cx)
            }))
            .on_action(cx.listener(|this, _: &InsertFromShapeLibrary, window, cx| {
                this.show_shape_library_picker(window, cx)
            }))
            .on_action(cx.listener(
                |this, _: &innerword_editor::actions::InsertImage, window, cx| {
                    this.choose_board_pictures(window, cx)
                },
            ))
            .on_action(cx.listener(|this, _: &OpenContextMenu, window, cx| {
                if let Some(focus) = this.page_focus_handle() {
                    window.focus(&focus);
                }
                this.open_board_menu(window, cx)
            }))
            // Duplicate of the open whiteboard while its page holds more than it
            // posted: the copy gets the last gesture.
            .capture_action(cx.listener(|this, _: &innerword_library::Duplicate, _, cx| {
                let Some(id) = this.open_board_id() else {
                    return;
                };
                if !this.board_holds_more() {
                    return;
                }
                cx.stop_propagation();
                let collect = this.collect_board(id, cx);
                let panel = this.panel.clone();
                cx.spawn(async move |_, cx| {
                    collect.await;
                    panel
                        .update(cx, |panel, cx| panel.duplicate_selected(cx))
                        .ok();
                })
                .detach();
            }));
        root = add::<AddText>(root, cx);
        root = add::<AddRectangle>(root, cx);
        root = add::<AddEllipse>(root, cx);
        root = add::<AddDiamond>(root, cx);
        root = add::<AddArrow>(root, cx);
        root = add::<AddFrame>(root, cx);
        root = say::<OpenDocumentSettings>(root, NO_SETTINGS_ON_A_BOARD, cx);
        root = say::<TogglePageView>(root, NO_SETTINGS_ON_A_BOARD, cx);
        root = say::<TogglePageOrientation>(root, NO_SETTINGS_ON_A_BOARD, cx);
        root = say::<IncreaseBodyTextSize>(root, NO_SETTINGS_ON_A_BOARD, cx);
        root = say::<DecreaseBodyTextSize>(root, NO_SETTINGS_ON_A_BOARD, cx);
        root = say::<IncreaseLineHeight>(root, NO_SETTINGS_ON_A_BOARD, cx);
        root = say::<DecreaseLineHeight>(root, NO_SETTINGS_ON_A_BOARD, cx);
        root = say::<UseTitleFromText>(root, TITLE_OF_A_BOARD, cx);
        root = say::<ChooseSpellingLanguage>(root, NO_SPELLING_ON_A_BOARD, cx);
        root = say::<comments::AddComment>(root, NO_COMMENTS_ON_A_BOARD, cx);
        root = say::<comments::NextComment>(root, NO_COMMENTS_ON_A_BOARD, cx);
        root = say::<comments::PreviousComment>(root, NO_COMMENTS_ON_A_BOARD, cx);
        root = say::<comments::FocusComments>(root, NO_COMMENTS_ON_A_BOARD, cx);
        root = say::<comments::ToggleComments>(root, NO_COMMENTS_ON_A_BOARD, cx);
        root = say::<comments::ToggleShowResolved>(root, NO_COMMENTS_ON_A_BOARD, cx);
        root = say::<ToggleVimMode>(root, NO_VIM_ON_A_BOARD, cx);
        root = say::<innerword_vim_mode::ToggleVimMode>(root, NO_VIM_ON_A_BOARD, cx);
        root = say::<crate::zoom::LifeSize>(root, NO_PAPER_ON_A_BOARD, cx);
        root = say::<crate::zoom::Automatic>(root, NO_PAPER_ON_A_BOARD, cx);
        root.capture_action(cx.listener(|this, _: &AskAssistant, _, cx| {
            cx.stop_propagation();
            let key = crate::keymap::hint_in(&FocusAssistant, "Workspace > BoardView", cx)
                .map(|hint| hint.text());
            let sentence = match key {
                Some(keys) => format!(
                    "The quick chat works in a text. Ask about this whiteboard in the side chat ({keys})."
                ),
                None => "The quick chat works in a text. Ask about this whiteboard in the side chat."
                    .to_string(),
            };
            this.show_toast(Toast::new(sentence).duration(Duration::from_secs(5)), cx);
        }))
    }

    /// The names of the actions the palette leaves out while a whiteboard is
    /// open: the commands of a text (no handler) and the document's commands
    /// that only say what a whiteboard is. Find, Save Now, the fits and the
    /// steps of the zoom work on a whiteboard.
    pub(crate) fn commands_not_for_the_open_board(&self, cx: &App) -> Vec<&'static str> {
        use crate::{actions::*, zoom};
        use innerword_spell::actions as spell;
        let mut names: Vec<&'static str> = self
            .text_commands
            .iter()
            .map(|action| action.name())
            .filter(|name| *name != innerword_editor::actions::InsertImage.name())
            .collect();
        let only_say: [&dyn Action; 21] = [
            &spell::NextMisspelling,
            &spell::PreviousMisspelling,
            &spell::ShowSuggestions,
            &spell::LearnSpelling,
            &spell::UnlearnSpelling,
            &spell::IgnoreSpelling,
            &OpenDocumentSettings,
            &TogglePageView,
            &TogglePageOrientation,
            &IncreaseBodyTextSize,
            &DecreaseBodyTextSize,
            &IncreaseLineHeight,
            &DecreaseLineHeight,
            &UseTitleFromText,
            &ChooseSpellingLanguage,
            &zoom::LifeSize,
            &zoom::Automatic,
            &CopyPageAddress,
            &OpenPageAddress,
            &ToggleVimMode,
            &AskAssistant,
        ];
        names.extend(only_say.iter().map(|action| action.name()));
        names.extend(
            crate::commands::all(cx)
                .iter()
                .filter(|command| command.category.as_ref() == innerword_comments::CATEGORY)
                .map(|command| command.action_name()),
        );
        let find: [&dyn Action; 1] = [&tools::Find];
        names.retain(|name| !find.iter().any(|action| action.name() == *name));
        names
    }

    /// The commands only a whiteboard has, which the palette leaves out
    /// while none is open.
    pub(crate) fn board_only_commands() -> Vec<&'static str> {
        let only: [&dyn Action; 10] = [
            &AddText,
            &AddRectangle,
            &AddEllipse,
            &AddDiamond,
            &AddArrow,
            &AddFrame,
            &MermaidToDrawing,
            &ToggleShapeLibrary,
            &AddToShapeLibrary,
            &InsertFromShapeLibrary,
        ];
        only.iter().map(|action| action.name()).collect()
    }

    /// The palette's title of a command while a whiteboard is open, where
    /// its own title names a text or a page.
    pub(crate) fn board_title_of(command: &mut innerword_ui::CommandDescriptor) {
        let title = match command.action_name() {
            name if name == crate::actions::RenameDocument.name() => "Rename whiteboard",
            name if name == crate::actions::FocusEditor.name() => "Go to whiteboard",
            name if name == crate::actions::ReloadPage.name() => "Reload whiteboard",
            _ => return,
        };
        let old = std::mem::replace(&mut command.title, title.into());
        command.keywords.push(old);
    }

    /// "Insert from shape library…": a picker of the stored library's items
    /// (each by its name, or "Shape 3: rectangle, arrow, text"), newest
    /// first; `enter` puts a copy at the middle of the view.
    pub(crate) fn show_shape_library_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let items: Vec<(String, String)> = {
            let source: Arc<str> = match &self.shape_library.held {
                Some((_, items)) => {
                    // The posted form is a list of items; read as a library.
                    Arc::from(format!(
                        r#"{{"type":"excalidrawlib","version":2,"source":"inner-word","libraryItems":{items}}}"#
                    ))
                }
                None => self.shape_library_json(cx),
            };
            match innerword_board::ShapeLibrary::from_stored(&source) {
                Ok(parsed) => {
                    let mut items: Vec<(i64, String, String)> = parsed
                        .library
                        .items()
                        .iter()
                        .enumerate()
                        .map(|(index, item)| (item.created(), item.title(index), item.id().to_string()))
                        .collect();
                    items.sort_by(|a, b| b.0.cmp(&a.0));
                    items.into_iter().map(|(_, title, id)| (title, id)).collect()
                }
                Err(_) => Vec::new(),
            }
        };
        if items.is_empty() {
            self.say_about_page(
                "The shape library is empty. Select shapes and choose “Add selection to shape library”.",
                cx,
            );
            return;
        }
        let workspace = cx.weak_entity();
        self.modal_layer.update(cx, |layer, cx| {
            layer.toggle(window, cx, |window, cx| {
                let delegate = ShapeLibraryDelegate::new(workspace, items);
                let rows = super::picker_rows(window, 12);
                innerword_ui::Picker::modal(delegate, window, cx)
                    .width(px(super::MODAL_WIDTH))
                    .max_visible_rows(rows)
            });
        });
    }
}

/// The picker of "Insert from shape library…".
pub struct ShapeLibraryDelegate {
    workspace: gpui::WeakEntity<Workspace>,
    /// (title, item id), newest first.
    items: Vec<(String, String)>,
    matches: Vec<usize>,
}

impl ShapeLibraryDelegate {
    fn new(workspace: gpui::WeakEntity<Workspace>, items: Vec<(String, String)>) -> Self {
        let matches = (0..items.len()).collect();
        Self {
            workspace,
            items,
            matches,
        }
    }
}

impl innerword_ui::PickerDelegate for ShapeLibraryDelegate {
    fn placeholder(&self) -> gpui::SharedString {
        "Insert from shape library".into()
    }

    fn update_matches(
        &mut self,
        query: &str,
        _: &mut Window,
        _: &mut Context<innerword_ui::Picker<Self>>,
    ) {
        let query = query.to_lowercase();
        self.matches = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, (title, _))| query.is_empty() || title.to_lowercase().contains(&query))
            .map(|(ix, _)| ix)
            .collect();
    }

    fn entry_count(&self) -> usize {
        self.matches.len()
    }

    fn entry(&self, ix: usize, _: &App) -> innerword_ui::PickerEntry {
        let title = self
            .matches
            .get(ix)
            .and_then(|ix| self.items.get(*ix))
            .map(|(title, _)| title.clone())
            .unwrap_or_default();
        innerword_ui::PickerItem::new(title).into()
    }

    fn confirm(
        &mut self,
        ix: usize,
        _secondary: bool,
        window: &mut Window,
        cx: &mut Context<innerword_ui::Picker<Self>>,
    ) -> bool {
        let Some((_, id)) = self
            .matches
            .get(ix)
            .and_then(|ix| self.items.get(*ix))
            .cloned()
        else {
            return false;
        };
        self.workspace
            .update(cx, |workspace, cx| {
                workspace.dismiss_modal(window, cx);
                if let Some(focus) = workspace.page_focus_handle() {
                    window.focus(&focus);
                }
                workspace.shape_library_call(calls::LibraryAction::Insert(id), cx);
            })
            .ok();
        true
    }
}

impl Workspace {
    /// "New Whiteboard": an empty whiteboard at the end of the folder
    /// selected in the library (`folder` when given), titled `title` (or
    /// "Untitled"), opened with the keyboard on the canvas. Without a page
    /// engine there is none.
    pub fn new_board(
        &mut self,
        title: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<DocumentId> {
        if pages::engine(cx).is_none() {
            self.say_about_page(&boards::not_shown_sentence(cx), cx);
            return None;
        }
        self.dismiss_modal(window, cx);
        let folder = self.panel.read(cx).selected_folder();
        let title = title.map(str::trim).unwrap_or_default().to_string();
        let prepared = super::saver::prepared_of(&Scene::empty());
        let (created, _) = self.library.update(cx, |library, cx| {
            library.change_outside(
                |store| {
                    let created = store
                        .create_board(folder, &title, &prepared.content())
                        // The selected folder may be gone (trashed elsewhere).
                        .or_else(|_| store.create_board(None, &title, &prepared.content()));
                    let changes = match &created {
                        Ok(saved) => vec![OutsideChange::DocumentCreated(saved.meta.id)],
                        Err(_) => Vec::new(),
                    };
                    (created, changes)
                },
                cx,
            )
        });
        let created = match created {
            Ok(created) => created,
            Err(error) => {
                self.toast_error(format!("Could not create the whiteboard: {error}"), cx);
                return None;
            }
        };
        let id = created.meta.id;
        if !title.is_empty() {
            // A name given is a name chosen (it never follows the text).
            let flag = crate::view_memory::TitleFlag::of(&created.meta);
            self.library
                .update(cx, |library, cx| {
                    library.set_setting(&crate::settings::keys::custom_title(id), &flag, cx)
                })
                .ok();
        }
        if !self.open_document(id, true, window, cx) {
            return None;
        }
        Some(id)
    }

    /// Makes version `snapshot` of the open whiteboard its scene: what the
    /// app holds is saved and kept as a version first, the old scene saved
    /// (compare-and-swap), and taken into the view as one step of its undo
    /// (section 6.5).
    pub(crate) fn restore_board_version(
        &mut self,
        snapshot: innerword_library::SnapshotId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(id) = self.open_board_id() else {
            return false;
        };
        let stored = match self.library.read(cx).snapshot(snapshot) {
            Ok(stored) if stored.meta.document_id == id => stored,
            Ok(_) => return false,
            Err(error) => {
                self.toast_error(format!("Could not read that version: {error}"), cx);
                return false;
            }
        };
        let restored = match Scene::from_stored(&stored.content) {
            Ok(parsed) => parsed.scene,
            Err(error) => {
                self.toast_error(format!("Could not read that version: {error}"), cx);
                return false;
            }
        };
        if !self.save_board_now(id, cx) {
            self.toast_error(
                "Nothing was restored: the drawing as it is now could not be saved.",
                cx,
            );
            return false;
        }
        let Some(expect) = self.board().map(|board| board.stamp) else {
            return false;
        };
        let prepared = super::saver::prepared_of(&restored);
        let (result, _) = self.library.update(cx, |library, cx| {
            library.change_outside(
                |store| {
                    if let Err(error) = store.snapshot_if_stale(id, Duration::ZERO) {
                        return (Err(error), Vec::new());
                    }
                    let result = store.save_board(id, &prepared.content(), Some(expect));
                    let changes = if result.is_ok() {
                        vec![OutsideChange::DocumentSaved(id)]
                    } else {
                        Vec::new()
                    };
                    (result, changes)
                },
                cx,
            )
        });
        match result {
            Ok(_) => {
                // A restore is an outside write: the view takes it by the base
                // rule, one step of its undo.
                self.adopt_stored_scene(id, &prepared.scene, true, cx);
                let when = crate::version_history::version_time_label(
                    crate::version_history::LocalTime::from_unix_ms(stored.meta.created_at),
                    crate::version_history::LocalTime::from_unix_ms(
                        self.library.read(cx).clock().now_ms(),
                    ),
                );
                self.dismiss_modal(window, cx);
                if let Some(focus) = self.page_focus_handle() {
                    window.focus(&focus);
                }
                self.show_toast(
                    Toast::new(format!(
                        "Restored {when}. The drawing from before is kept as a version."
                    ))
                    .success()
                    .duration(Duration::from_secs(6)),
                    cx,
                );
                true
            }
            Err(error) => {
                self.toast_error(format!("Nothing was restored: {error}"), cx);
                false
            }
        }
    }
}

/// What [`Workspace::end_then`] does once what a whiteboard's page holds is
/// saved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    /// `cmd-w`, File > Close Window, the window's close button.
    CloseWindow,
    /// `cmd-q`, Inner Word > Quit, a signal in the terminal.
    Quit,
}

impl Workspace {
    /// The one routine of closing the window and quitting (section 4.6): when
    /// the open whiteboard's page holds more than it posted, or a whiteboard
    /// is closing, the page is asked for it and it is saved first (at most a
    /// second for both together, and pictures still being stored are waited
    /// for); then everything is saved and the window goes or the app quits.
    /// A second request while one is under way ends at once, after the
    /// synchronous save.
    pub fn end_then(&mut self, end: End, window: &mut Window, cx: &mut Context<Self>) {
        if self.ending.is_some() || !self.board_holds_more() {
            self.ending = None;
            self.perform_end(end, window, cx);
            return;
        }
        self.ending = Some(end);
        let collect = self
            .open_board_id()
            .filter(|_| {
                self.board()
                    .is_some_and(|board| board.ready && board.status.holds_more())
            })
            .map(|id| self.collect_board(id, cx));
        cx.spawn_in(window, async move |this, cx| {
            let started = Instant::now();
            if let Some(collect) = collect {
                let timer = cx
                    .background_executor()
                    .timer(boards::VIEW_COLLECT_PATIENCE);
                crate::pages::within(collect, timer).await;
            }
            // The whiteboard that was closing saves by itself.
            loop {
                let closing = this
                    .update(cx, |this, _| this.closing_board.is_some())
                    .unwrap_or(false);
                if !closing || started.elapsed() > boards::VIEW_COLLECT_PATIENCE {
                    break;
                }
                cx.background_executor().timer(PICTURE_POLL).await;
            }
            this.update_in(cx, |this, window, cx| {
                if this.ending.take().is_some() {
                    this.finish_closing_board_now(window, cx);
                    this.perform_end(end, window, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Saves everything and closes the window or quits.
    fn perform_end(&mut self, end: End, window: &mut Window, cx: &mut Context<Self>) {
        self.keep_what_the_tools_hold(window, cx);
        self.flush(cx);
        self.store_assistant_conversation(cx);
        match end {
            End::CloseWindow => {
                self.store_zoom_now(cx);
                self.window_bounds = Some(window.window_bounds());
                self.save_window_bounds(cx);
                window.remove_window();
            }
            End::Quit => cx.quit(),
        }
    }

    /// Whether a close or a quit waits for a whiteboard's page.
    pub fn ending(&self) -> Option<End> {
        self.ending
    }
}
