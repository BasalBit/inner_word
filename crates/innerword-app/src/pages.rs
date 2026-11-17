//! Pages, as the app sees them: what shows an open page (the engine, a seam
//! like [`crate::printing::PrintTarget`]), where the bytes of a page come
//! from while it is shown ([`LibraryPageSource`]), what a page hands on to
//! GPUI of the keys it receives ([`PageKeys`]), and the helper of a scripted
//! run that stores a folder that is offline already as a page
//! ([`make_page`]).
//!
//! The view itself, the session of an open page, its element, focus,
//! overlays, title and status items, are the workspace's
//! (`workspace/page.rs`). WebKit is `innerword-web`'s, the manifest and the
//! bundler `innerword-page`'s.
//!
//! - **Which engine a workspace shows pages with** ([`engine`]): `main`
//!   installs WebKit ([`use_webkit`]) in every run of the real binary,
//!   scripted or not, unless `INNER_WORD_PAGES=off` or
//!   [`innerword_web::available`] says this macOS cannot keep a page offline.
//!   A test installs the fake ([`use_fake`]), which draws the page's text in
//!   a GPUI element and records what the workspace asked of the view. Without
//!   an engine a page shows its text in a quiet column, with the sentence
//!   that says why ([`not_shown_sentence`]); there is no import then.
//! - **AppKit's callbacks never enter GPUI.** Every callback of the view
//!   only posts to the page's [`Inbox`]; a task of the workspace drains it.
//!   `takes_key` and `menu_items`, which must answer at once, answer from
//!   [`PageKeys`], which the workspace refreshes whenever the keymap is
//!   installed and whenever a page opens.

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, VecDeque},
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    rc::Rc,
    sync::{Arc, Mutex},
    task::{Context as TaskContext, Poll, Waker},
};

use gpui::{Action, App, Entity, Global, KeyBinding, KeyContext, Keymap, Keystroke, Modifiers};
use innerword_library::{
    DocumentId, FolderId, Library, PageContent,
    store::{FileId, ImageFile},
};
use innerword_page::Manifest;
use innerword_web::{Bitmap, KeyPress, MenuEntry, PageSource, ServedFile};

/// The environment variable that keeps a run of the real binary from
/// showing pages with WebKit (`INNER_WORD_PAGES=off`).
pub const PAGES_ENV: &str = "INNER_WORD_PAGES";

/// What a run without a page engine says over a page.
pub const NOT_SHOWN_IN_THIS_RUN: &str = "Pages are not shown in this run.";

/// The key context of the element of an open page.
pub const PAGE_VIEW: &str = "PageView";

/// The most bytes of whole files a view keeps in memory for the life of
/// the view (a media file is asked for in ranges, many times).
pub const SOURCE_CACHE_BYTES: u64 = 64 * 1024 * 1024;

// ----- the engine -------------------------------------------------------------------

/// What shows the pages of a workspace.
#[derive(Clone)]
pub enum PageEngine {
    /// The real view and check (`innerword-web`). `main` installs it in every
    /// run of the real binary, scripted or not, unless `INNER_WORD_PAGES=off`
    /// or [`innerword_web::available`] fails ([`use_webkit`] asks it and
    /// installs nothing then).
    WebKit,
    /// Tests: a GPUI element that draws the page's text, and a check that
    /// answers from the HTML (`innerword_page::static_check`, or what
    /// [`FakePages::set_check`] says). Records what was asked.
    Fake(Rc<FakePages>),
}

struct Installed(PageEngine);
impl Global for Installed {}

/// Why there is no engine: the sentence of `innerword_web::available`.
struct Unavailable(String);
impl Global for Unavailable {}

/// Installs WebKit as the engine of pages, when this macOS can keep a page
/// offline. `Err` is `innerword_web::available`'s sentence, and nothing is
/// installed then. Also starts compiling the rule list in the background,
/// so the first page opens without waiting for it.
pub fn use_webkit(cx: &mut App) -> Result<(), String> {
    if let Err(why) = innerword_web::available() {
        cx.set_global(Unavailable(why.clone()));
        return Err(why);
    }
    cx.set_global(Installed(PageEngine::WebKit));
    // The data directory by name: only a real launch gets here (`main`),
    // and the library of the old name was brought over already or is
    // brought over now, as the window's launch does. A test never calls
    // this, so no test reaches the user's data directory through it.
    if let Ok(dir) = innerword_library::data_dir() {
        innerword_web::compile_rules(&dir.join(RULES_DIR_NAME));
    }
    Ok(())
}

/// Installs the fake engine (a test, before it opens a workspace) and
/// returns what it records.
pub fn use_fake(cx: &mut App) -> Rc<FakePages> {
    let fake = Rc::new(FakePages::default());
    cx.set_global(Installed(PageEngine::Fake(fake.clone())));
    fake
}

/// The installed engine; `None`: a page shows its text in a quiet column
/// and says why ([`not_shown_sentence`]).
pub fn engine(cx: &App) -> Option<PageEngine> {
    cx.try_global::<Installed>()
        .map(|installed| installed.0.clone())
}

/// What a page says over its text in a run without a page engine.
pub fn not_shown_sentence(cx: &App) -> String {
    cx.try_global::<Unavailable>()
        .map(|why| why.0.clone())
        .unwrap_or_else(|| NOT_SHOWN_IN_THIS_RUN.to_string())
}

/// Whether the view of a page keeps drawing while the window is covered
/// ([`draw_when_covered`]).
struct DrawWhenCovered;
impl Global for DrawWhenCovered {}

/// A scripted run stays in the background, under other windows, and takes
/// screenshots of its window: there WebKit must not stop drawing a page it
/// finds covered (it does after a moment, and the page is blank in every
/// picture). `main` calls this for a scripted run; a person's run keeps
/// WebKit's own behaviour.
pub fn draw_when_covered(cx: &mut App) {
    cx.set_global(DrawWhenCovered);
}

/// See [`draw_when_covered`].
pub fn draws_when_covered(cx: &App) -> bool {
    cx.has_global::<DrawWhenCovered>()
}

/// Tells the web view at `native` to keep drawing while its window is
/// covered, where WebKit answers the (private, optional) switch.
#[allow(unexpected_cfgs)]
pub fn keep_drawing_when_covered(native: *mut std::ffi::c_void) {
    use objc::{
        msg_send,
        runtime::{BOOL, NO, Object},
        sel, sel_impl,
    };
    let view = native as *mut Object;
    if view.is_null() {
        return;
    }
    // SAFETY: a live WKWebView on the main thread; the selector is asked
    // before it is sent.
    unsafe {
        let answers: BOOL =
            msg_send![view, respondsToSelector: sel!(_setWindowOcclusionDetectionEnabled:)];
        if answers != NO {
            let _: () = msg_send![view, _setWindowOcclusionDetectionEnabled: NO];
        }
    }
}

/// Tells the web view at `native` to take no drops: files dragged over a
/// page then reach GPUI's view under it, and the page element adds them
/// as pages (WebKit would otherwise take a dropped file itself, and its
/// navigation to the file is cancelled: nothing would happen).
#[allow(unexpected_cfgs)]
pub fn take_no_drops(native: *mut std::ffi::c_void) {
    use objc::{
        msg_send,
        runtime::{BOOL, NO, Object},
        sel, sel_impl,
    };
    let view = native as *mut Object;
    if view.is_null() {
        return;
    }
    // SAFETY: a live WKWebView on the main thread; the selector is asked
    // before it is sent.
    unsafe {
        let answers: BOOL = msg_send![view, respondsToSelector: sel!(unregisterDraggedTypes)];
        if answers != NO {
            let _: () = msg_send![view, unregisterDraggedTypes];
        }
    }
}

/// Whether the environment asks for no pages (`INNER_WORD_PAGES=off`).
pub fn switched_off_by_env() -> bool {
    std::env::var(PAGES_ENV).is_ok_and(|value| value.trim().eq_ignore_ascii_case("off"))
}

/// The host of a page's origin, `innerword-page://<host>/`: the page's id and
/// the moment its row was made (`page-12-1731600000000`). Every view of a
/// run shares one store of website data, kept per origin (local storage,
/// IndexedDB, cookies), and SQLite gives the id of a page erased for good
/// to the next row: with the id alone a new page would read what the
/// erased one left. An update of the page (an agent's, a restored version,
/// adding it again) keeps its row and so its origin.
pub fn page_host(id: DocumentId, created_at: i64) -> String {
    format!("page-{}-{created_at}", id.0)
}

/// The folder of the data directory where the rule list is kept.
const RULES_DIR_NAME: &str = "WebKit";

/// Where the rule list of the views is compiled and kept: `WebKit/` in the
/// data directory of the library (`library_db`'s folder), so a test or a
/// scripted run with `INNER_WORD_DATA_DIR` writes only there. `None` for a
/// library without a file (a library in memory, in tests): the callers
/// take a folder in the temporary directory then. It never asks for the
/// default data directory: that would be the user's own library in a test,
/// which the first look moves from its old name ("The Writer").
pub fn rules_dir_of(library_db: Option<&Path>) -> Option<PathBuf> {
    Some(library_db?.parent()?.join(RULES_DIR_NAME))
}

// ----- what the view tells the workspace --------------------------------------------

/// What a view told its owner, waiting for the workspace's task.
#[derive(Debug)]
pub enum PageEvent {
    Loaded,
    Failed(String),
    Crashed,
    WantsFocus,
    OpenExternally(String),
    Alert(String),
    MenuChosen(u32),
    /// A snapshot that was asked for (`None`: it could not be made).
    Snapshot(Option<Bitmap>),
    /// A find that was asked for: the query and whether it was found.
    Found {
        query: String,
        found: bool,
    },
    /// A key the fake view handed on (`FakePages::press`), as GPUI's view
    /// gets the real one.
    KeyHandedOn(Keystroke),
}

/// The events of one view, in order, and the task that waits for them.
#[derive(Default)]
pub struct Inbox {
    events: VecDeque<PageEvent>,
    waker: Option<Waker>,
    closed: bool,
}

/// A shared [`Inbox`].
pub type SharedInbox = Rc<RefCell<Inbox>>;

/// Posts `event` and wakes the task that drains the inbox. Never enters
/// GPUI: waking only schedules the task on the main thread's queue.
pub fn post(inbox: &SharedInbox, event: PageEvent) {
    let waker = {
        let mut inbox = inbox.borrow_mut();
        inbox.events.push_back(event);
        inbox.waker.take()
    };
    if let Some(waker) = waker {
        waker.wake();
    }
}

/// Closes the inbox: the task that drains it ends.
pub fn close(inbox: &SharedInbox) {
    let waker = {
        let mut inbox = inbox.borrow_mut();
        inbox.closed = true;
        inbox.waker.take()
    };
    if let Some(waker) = waker {
        waker.wake();
    }
}

/// Takes a `WantsFocus` that is waiting, if there is one (the frame that
/// is drawn before the task ran must not take the first responder back
/// from a page the reader just clicked into).
pub fn take_wants_focus(inbox: &SharedInbox) -> bool {
    let mut inbox = inbox.borrow_mut();
    let before = inbox.events.len();
    inbox
        .events
        .retain(|event| !matches!(event, PageEvent::WantsFocus));
    inbox.events.len() != before
}

/// Resolves to every event that is waiting once there is at least one, or
/// to `None` when the inbox was closed.
pub fn next_events(inbox: SharedInbox) -> NextEvents {
    NextEvents(inbox)
}

/// See [`next_events`].
pub struct NextEvents(SharedInbox);

impl Future for NextEvents {
    type Output = Option<Vec<PageEvent>>;

    fn poll(self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Self::Output> {
        let mut inbox = self.0.borrow_mut();
        if !inbox.events.is_empty() {
            return Poll::Ready(Some(inbox.events.drain(..).collect()));
        }
        if inbox.closed {
            return Poll::Ready(None);
        }
        inbox.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

/// One answer that arrives later, from inside AppKit: the sending half only
/// notes it and wakes whoever waits (`Answer`).
pub fn answer<T>() -> (AnswerTx<T>, Answer<T>) {
    let slot = Rc::new(RefCell::new(Slot {
        value: None,
        gone: false,
        waker: None,
    }));
    (AnswerTx(slot.clone()), Answer(slot))
}

struct Slot<T> {
    value: Option<T>,
    gone: bool,
    waker: Option<Waker>,
}

/// The half of [`answer`] that is told.
pub struct AnswerTx<T>(Rc<RefCell<Slot<T>>>);

impl<T> AnswerTx<T> {
    pub fn send(self, value: T) {
        self.0.borrow_mut().value = Some(value);
        // Dropping `self` wakes.
    }
}

impl<T> Drop for AnswerTx<T> {
    fn drop(&mut self) {
        let waker = {
            let mut slot = self.0.borrow_mut();
            slot.gone = true;
            slot.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

/// The half of [`answer`] that waits: `None` when the sender went without
/// an answer.
pub struct Answer<T>(Rc<RefCell<Slot<T>>>);

impl<T> Future for Answer<T> {
    type Output = Option<T>;

    fn poll(self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Option<T>> {
        let mut slot = self.0.borrow_mut();
        if let Some(value) = slot.value.take() {
            return Poll::Ready(Some(value));
        }
        if slot.gone {
            return Poll::Ready(None);
        }
        slot.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

/// What the view of a page is told by WebKit, for the workspace: every
/// notifying callback posts to the inbox, `takes_key` and `menu_items`
/// answer from the [`PageKeys`] snapshot.
pub struct ViewEvents {
    pub inbox: SharedInbox,
    pub keys: Rc<RefCell<PageKeys>>,
}

impl innerword_web::PageEvents for ViewEvents {
    fn loaded(&self) {
        post(&self.inbox, PageEvent::Loaded);
    }

    fn failed(&self, message: String) {
        post(&self.inbox, PageEvent::Failed(message));
    }

    fn crashed(&self) {
        post(&self.inbox, PageEvent::Crashed);
    }

    fn wants_focus(&self) {
        post(&self.inbox, PageEvent::WantsFocus);
    }

    fn takes_key(&self, key: &KeyPress) -> bool {
        self.keys.borrow().takes(key)
    }

    fn open_externally(&self, url: String) {
        post(&self.inbox, PageEvent::OpenExternally(url));
    }

    fn alert(&self, message: String) {
        post(&self.inbox, PageEvent::Alert(message));
    }

    fn menu_items(&self, _link: Option<&str>) -> Vec<MenuEntry> {
        self.keys.borrow().menu_entries()
    }

    fn menu_chosen(&self, id: u32) {
        post(&self.inbox, PageEvent::MenuChosen(id));
    }
}

// ----- the keys a page hands on -----------------------------------------------------

/// One of the app's items of WebKit's right-click menu.
pub struct PageMenuItem {
    pub title: String,
    /// The key the keymap has for it where a page has the keyboard
    /// (keystroke text, `cmd-f`).
    pub key: Option<String>,
    pub enabled: bool,
    pub separator_before: bool,
    pub action: Box<dyn Action>,
}

/// What the view of a page asks while AppKit waits for an answer: which keys
/// go to GPUI instead of the page, and the app's items of the right-click
/// menu. Written by the workspace, never read from a GPUI global.
pub struct PageKeys {
    /// The installed bindings (a copy).
    keymap: Keymap,
    /// Where the keyboard is while the page has it.
    contexts: Vec<KeyContext>,
    pub menu: Vec<PageMenuItem>,
}

impl Default for PageKeys {
    fn default() -> Self {
        PageKeys {
            keymap: Keymap::new(Vec::new()),
            contexts: page_contexts(),
            menu: Vec::new(),
        }
    }
}

/// The key contexts from the window's root to the element of a page.
pub fn page_contexts() -> Vec<KeyContext> {
    [crate::workspace::KEY_CONTEXT, PAGE_VIEW]
        .into_iter()
        .filter_map(|name| KeyContext::parse(name).ok())
        .collect()
}

impl PageKeys {
    /// The keys as the keymap installed now has them.
    pub fn refresh_keys(&mut self, cx: &App) {
        let bindings: Vec<KeyBinding> = cx.key_bindings().borrow().bindings().cloned().collect();
        self.keymap = Keymap::new(bindings);
    }

    /// Whether GPUI has a binding for `key` where a page has the keyboard:
    /// then the view hands it to GPUI's view instead of the page.
    pub fn takes(&self, key: &KeyPress) -> bool {
        let Some(keystroke) = keystroke_of(key) else {
            return false;
        };
        self.takes_keystroke(&keystroke)
    }

    /// [`PageKeys::takes`] for a keystroke as GPUI spells it.
    pub fn takes_keystroke(&self, keystroke: &Keystroke) -> bool {
        let (bindings, _) = self
            .keymap
            .bindings_for_input(std::slice::from_ref(keystroke), &self.contexts);
        !bindings.is_empty()
    }

    /// The menu's entries, ids in order from 1; a title carries its key after
    /// a tab, as `innerword_web` draws it.
    pub fn menu_entries(&self) -> Vec<MenuEntry> {
        self.menu
            .iter()
            .enumerate()
            .map(|(ix, item)| MenuEntry {
                id: ix as u32 + 1,
                title: match &item.key {
                    Some(key) => format!("{}\t{key}", item.title),
                    None => item.title.clone(),
                },
                enabled: item.enabled,
                separator_before: item.separator_before,
            })
            .collect()
    }

    /// The action of the menu item `id` (from [`PageKeys::menu_entries`]).
    pub fn menu_action(&self, id: u32) -> Option<Box<dyn Action>> {
        let item = self.menu.get((id as usize).checked_sub(1)?)?;
        item.enabled.then(|| item.action.boxed_clone())
    }
}

/// The key the installed keymap has for `action` where a page has the
/// keyboard, as keystroke text (`cmd-f`); the binding added last wins, as
/// for every hint.
pub fn key_at_page(action: &dyn Action, cx: &App) -> Option<String> {
    let contexts = page_contexts();
    let keymap = cx.key_bindings();
    let keymap = keymap.borrow();
    keymap
        .bindings_for_action(action)
        .rfind(|binding| {
            let (matched, _) = keymap.bindings_for_input(binding.keystrokes(), &contexts);
            matched
                .iter()
                .any(|found| found.action().partial_eq(binding.action()))
        })
        .map(crate::keymap::keys_of)
}

/// A key as AppKit reported it, as GPUI spells it.
pub fn keystroke_of(key: &KeyPress) -> Option<Keystroke> {
    let named = match key.key_code {
        36 | 76 => Some("enter"),
        48 => Some("tab"),
        49 => Some("space"),
        51 => Some("backspace"),
        53 => Some("escape"),
        117 => Some("delete"),
        123 => Some("left"),
        124 => Some("right"),
        125 => Some("down"),
        126 => Some("up"),
        115 => Some("home"),
        119 => Some("end"),
        116 => Some("pageup"),
        121 => Some("pagedown"),
        122 => Some("f1"),
        120 => Some("f2"),
        99 => Some("f3"),
        118 => Some("f4"),
        96 => Some("f5"),
        97 => Some("f6"),
        98 => Some("f7"),
        100 => Some("f8"),
        101 => Some("f9"),
        109 => Some("f10"),
        103 => Some("f11"),
        111 => Some("f12"),
        _ => None,
    };
    let key_name = match named {
        Some(name) => name.to_string(),
        None => {
            let ch = key.characters_ignoring_modifiers.chars().next()?;
            ch.to_lowercase().collect()
        }
    };
    Some(Keystroke {
        modifiers: Modifiers {
            control: key.ctrl,
            alt: key.alt,
            shift: key.shift,
            platform: key.cmd,
            function: key.function && named.is_none(),
        },
        key: key_name,
        key_char: None,
    })
}

/// The `KeyPress` a key of `text` (`ctrl-enter`, `shift-f10`, `a`) would be,
/// as AppKit reports it: for the fake view, which is handed keys by name.
pub fn key_press_of(text: &str) -> Option<KeyPress> {
    let keystroke = Keystroke::parse(text).ok()?;
    let code = |name: &str| -> u16 {
        match name {
            "enter" => 36,
            "tab" => 48,
            "space" => 49,
            "backspace" => 51,
            "escape" => 53,
            "delete" => 117,
            "left" => 123,
            "right" => 124,
            "down" => 125,
            "up" => 126,
            "home" => 115,
            "end" => 119,
            "pageup" => 116,
            "pagedown" => 121,
            "f1" => 122,
            "f2" => 120,
            "f3" => 99,
            "f4" => 118,
            "f5" => 96,
            "f6" => 97,
            "f7" => 98,
            "f8" => 100,
            "f9" => 101,
            "f10" => 109,
            "f11" => 103,
            "f12" => 111,
            _ => 0,
        }
    };
    let key_code = code(&keystroke.key);
    let characters = if key_code == 0 {
        keystroke.key.clone()
    } else {
        String::new()
    };
    Some(KeyPress {
        key_code,
        characters: characters.clone(),
        characters_ignoring_modifiers: characters,
        cmd: keystroke.modifiers.platform,
        ctrl: keystroke.modifiers.control,
        alt: keystroke.modifiers.alt,
        shift: keystroke.modifiers.shift,
        function: keystroke.key.starts_with('f') && key_code != 0,
    })
}

// ----- where a page's bytes come from -----------------------------------------------

/// The served files of a page for its view: looked up in the manifest, read
/// through a read-only connection of its own to `images.db` (WAL lets it
/// read while the import or an agent's listener writes through another
/// connection, so a page's requests never queue behind a writer). Whole
/// files it read are kept in memory up to [`SOURCE_CACHE_BYTES`] for the
/// life of the view.
pub struct LibraryPageSource {
    manifest: Manifest,
    file: Mutex<ImageFile>,
    cache: Mutex<Cache>,
}

#[derive(Default)]
struct Cache {
    bytes: HashMap<FileId, Arc<[u8]>>,
    total: u64,
}

impl LibraryPageSource {
    /// The source of `manifest` in `library`: over `images.db` beside the
    /// library on disk, or, for a library in memory (tests), every file read
    /// now through the library's store into memory.
    pub fn of(library: &Library, manifest: &Manifest) -> Arc<dyn PageSource> {
        match library.path() {
            Some(database) => Arc::new(LibraryPageSource {
                manifest: manifest.clone(),
                file: Mutex::new(ImageFile::read_only(database)),
                cache: Mutex::default(),
            }),
            None => {
                let files = manifest.files.iter().filter_map(|file| {
                    let id = FileId::parse(&file.file)?;
                    let bytes = library.store().file_bytes(&id).ok().flatten()?;
                    Some((
                        file.path.clone(),
                        ServedFile {
                            media_type: file.media_type.clone(),
                            bytes: Arc::from(bytes),
                        },
                    ))
                });
                Arc::new(innerword_web::MemorySource::new(files.collect::<Vec<_>>()))
            }
        }
    }
}

impl PageSource for LibraryPageSource {
    fn file(&self, path: &str) -> Option<ServedFile> {
        let entry = self.manifest.served(path)?;
        let id = FileId::parse(&entry.file)?;
        if let Some(bytes) = self.cache.lock().ok()?.bytes.get(&id) {
            return Some(ServedFile {
                media_type: entry.media_type.clone(),
                bytes: bytes.clone(),
            });
        }
        let bytes: Arc<[u8]> = Arc::from(self.file.lock().ok()?.file_bytes(&id).ok()??);
        if let Ok(mut cache) = self.cache.lock()
            && cache.total + bytes.len() as u64 <= SOURCE_CACHE_BYTES
        {
            cache.total += bytes.len() as u64;
            cache.bytes.insert(id, bytes.clone());
        }
        Some(ServedFile {
            media_type: entry.media_type.clone(),
            bytes,
        })
    }
}

/// How many files of `manifest` the library does not have (a row read each,
/// no bytes).
pub fn missing_files(library: &Library, manifest: &Manifest) -> usize {
    manifest
        .file_ids()
        .iter()
        .filter(|hex| {
            FileId::parse(hex)
                .map(|id| !matches!(library.store().file_info(&id), Ok(Some(_))))
                .unwrap_or(true)
        })
        .count()
}

// ----- a folder that is offline already, stored as a page (scripted runs) -----------

/// Stores the folder (or file) at `path` as a page in `folder` of the
/// library, as it is: its files are served as they are (no reference is
/// rewritten, nothing is fetched) and the check is `static_check` (no
/// WebKit). For scripted runs only (`page:make:`): the archive's artifact
/// folders, whose `offline/index.html` names its pieces in `vendor/`, are
/// offline already. A page whose served files would name the web is still
/// stored: the view blocks what it asks of the web. The real way in is the
/// import (`page:import:`).
pub fn make_page(
    library: &Entity<Library>,
    path: &Path,
    entry: Option<&str>,
    folder: Option<FolderId>,
    cx: &mut App,
) -> Result<innerword_library::DocumentMeta, String> {
    let input = innerword_page::read_input(path, entry).map_err(|error| error.sentence())?;
    let files = input
        .files
        .iter()
        .map(|file| (file.path.clone(), file.bytes.to_vec()))
        .collect();
    let mut bundle = offline_bundle(&input.name, &input.entry, files);
    bundle.fingerprint = input.fingerprint.clone();
    bundle.warnings = input.warnings.clone();
    store_offline_bundle(library, bundle, folder, None, cx)
}

/// A page of `files` (path, bytes) served as they are, opening with
/// `entry`: what `page:make:` stores, and what tests make pages of.
pub fn offline_bundle(
    name: &str,
    entry: &str,
    files: Vec<(String, Vec<u8>)>,
) -> innerword_page::Bundle {
    let mut files: Vec<innerword_page::BundleFile> = files
        .into_iter()
        .map(|(path, bytes)| innerword_page::BundleFile {
            media_type: innerword_page::media_type(&path).to_string(),
            path,
            bytes: Arc::from(bytes),
        })
        .collect();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let fingerprint = {
        let mut all = Vec::new();
        for file in &files {
            all.extend_from_slice(file.path.as_bytes());
            all.push(0);
            all.extend_from_slice(innerword_page::file_id_hex(&file.bytes).as_bytes());
        }
        innerword_page::file_id_hex(&all)
    };
    innerword_page::Bundle {
        name: name.to_string(),
        entry: entry.to_string(),
        fingerprint,
        served: files.clone(),
        original: files,
        externals: Vec::new(),
        warnings: Vec::new(),
    }
}

/// Stores `bundle` as a page in `folder` (with the address it came from,
/// when given), checked by `static_check` (no WebKit): its files first,
/// then the row.
pub fn store_offline_bundle(
    library: &Entity<Library>,
    bundle: innerword_page::Bundle,
    folder: Option<FolderId>,
    source_url: Option<&str>,
    cx: &mut App,
) -> Result<innerword_library::DocumentMeta, String> {
    let report = innerword_page::static_check(&bundle);
    let prepared = innerword_page::prepare(bundle, &report).map_err(|error| error.sentence())?;
    let manifest = prepared.manifest(source_url, now_ms());
    let text = prepared.text.clone();
    let stats = innerword_core::Fragment::from_plain_text(&text)
        .to_document()
        .stats();
    let title = prepared.title();
    library.update(cx, |library, cx| {
        for blob in &prepared.blobs {
            library
                .put_file(&blob.data)
                .map_err(|error| format!("its files could not be stored: {error}"))?;
        }
        let json = manifest.to_json();
        library
            .create_page(
                folder,
                &PageContent {
                    manifest: &json,
                    title: &title,
                    plain_text: &text,
                    word_count: stats.words as u64,
                    char_count: stats.characters as u64,
                    source_url,
                },
                cx,
            )
            .map_err(|error| format!("it could not be stored: {error}"))
    })
}

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

// ----- adding pages: the seams of the import ----------------------------------------

/// Whether "Import Page…" may open the macOS open panel, and the paths a
/// test named instead.
#[derive(Default)]
struct ImportChooser {
    panel: bool,
    beforehand: VecDeque<Vec<PathBuf>>,
}

impl Global for ImportChooser {}

/// From now on "Import Page…" asks for files and folders in the macOS open
/// panel. Called by `main` in a launch that is not scripted, and by nothing
/// else.
pub fn use_open_panel(cx: &mut App) {
    cx.default_global::<ImportChooser>().panel = true;
}

/// Names the files and folders the next "Import Page…" takes, instead of
/// asking for them in a panel (once; a second call names those of the one
/// after). For tests.
pub fn choose_beforehand(paths: Vec<PathBuf>, cx: &mut App) {
    cx.default_global::<ImportChooser>()
        .beforehand
        .push_back(paths);
}

/// What "Import Page…" takes: the paths a test named, or what the person
/// chose in the open panel (files and folders, several), or nothing (a run
/// that was not told to open the panel: a test, a scripted run).
pub(crate) fn choose_inputs(cx: &mut App) -> gpui::Task<Option<Vec<PathBuf>>> {
    let chooser = cx.default_global::<ImportChooser>();
    if let Some(paths) = chooser.beforehand.pop_front() {
        return gpui::Task::ready(Some(paths).filter(|paths| !paths.is_empty()));
    }
    if !chooser.panel {
        return gpui::Task::ready(None);
    }
    let receiver = cx.prompt_for_paths(gpui::PathPromptOptions {
        files: true,
        directories: true,
        multiple: true,
        prompt: Some("Import".into()),
    });
    cx.spawn(async move |_| match receiver.await {
        Ok(Ok(paths)) => paths.filter(|paths| !paths.is_empty()),
        _ => None,
    })
}

/// The fetcher a test installed ([`set_fetcher`]).
struct InstalledFetcher(Arc<dyn innerword_page::Fetcher>);
impl Global for InstalledFetcher {}

/// The fetcher the import asks for what a page names on the web, instead
/// of `curl` (tests: a `innerword_page::MapFetcher`, or one of their own that
/// waits). Each gesture wraps it in a `innerword_page::CachingFetcher`.
pub fn set_fetcher(fetcher: Arc<dyn innerword_page::Fetcher>, cx: &mut App) {
    cx.set_global(InstalledFetcher(fetcher));
}

/// What one gesture of adding pages fetches with: each address once
/// (`CachingFetcher`) through the fetcher a test installed, else `curl`
/// for public addresses only. A run on the fake engine never reaches the
/// network: without a fetcher of the test's, nothing is fetched (every
/// external is "not there"). `INNER_WORD_FETCH_LOCAL=1` lets a debug build
/// fetch from this Mac (a scripted run against a server on 127.0.0.1), as
/// the tool's does; a release build does not read it.
pub(crate) fn fetcher_for_a_gesture(cx: &App) -> Arc<dyn innerword_page::Fetcher> {
    let inner: Arc<dyn innerword_page::Fetcher> = match cx.try_global::<InstalledFetcher>() {
        Some(installed) => installed.0.clone(),
        None if matches!(engine(cx), Some(PageEngine::Fake(_))) => {
            Arc::new(innerword_page::MapFetcher::new())
        }
        None => {
            let local = cfg!(debug_assertions)
                && std::env::var(innerword_agent::FETCH_LOCAL_ENV).is_ok_and(|value| value == "1");
            if local {
                Arc::new(innerword_page::CurlFetcher::new().allowing_local())
            } else {
                Arc::new(innerword_page::CurlFetcher::new())
            }
        }
    };
    Arc::new(innerword_page::CachingFetcher::new(inner))
}

/// Whether `path` is something the import takes as a page: an `.html` or
/// `.htm` file, or a folder (whose `index.html`, or one HTML file at its
/// top, opens the page). Everything else among dropped files is left out.
pub fn is_page_input(path: &Path) -> bool {
    if path.is_dir() {
        return true;
    }
    path.is_file()
        && path.extension().is_some_and(|ending| {
            ending.eq_ignore_ascii_case("html") || ending.eq_ignore_ascii_case("htm")
        })
}

/// What a page is called before its title is known: the name of the file
/// without its ending, or of the folder.
pub fn input_name(path: &Path) -> String {
    let name = if path.is_dir() {
        path.file_name()
    } else {
        path.file_stem()
    };
    name.map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Why pages cannot be added in this run (no page engine): the sentence of
/// `innerword_web::available`, or that this run shows no pages.
pub fn cannot_add_sentence(cx: &App) -> String {
    match cx.try_global::<Unavailable>() {
        Some(why) => format!("Pages cannot be added: {}", why.0),
        None => "Pages cannot be added in this run.".to_string(),
    }
}

/// What WebKit's check found, as the bundler reads it (the same mapping as
/// the tool's child, `innerword __page-check`).
pub fn report_of(outcome: innerword_web::CheckOutcome) -> innerword_page::CheckReport {
    innerword_page::CheckReport {
        engine: outcome.engine,
        elapsed_ms: u64::try_from(outcome.elapsed.as_millis()).unwrap_or(u64::MAX),
        loads: outcome
            .loads
            .into_iter()
            .map(|load| innerword_page::LoadReport {
                path: load.path,
                title: load.title,
                first_heading: load.first_heading,
                text: load.text,
                finished: load.finished,
                navigation_error: load.navigation_error,
                crashed: load.crashed,
                missing: load.missing,
                blocked: load
                    .blocked
                    .into_iter()
                    .map(|(url, directive)| innerword_page::Blocked { url, directive })
                    .collect(),
                script_errors: load.script_errors,
                requests: load.requests,
                unwatched_frames: load.unwatched_frames,
            })
            .collect(),
        failure: outcome.failure,
    }
}

/// Checks `bundle` with the installed engine: WebKit loads its served files
/// offscreen with the network blocked (on the main thread: call it outside
/// any update, from a task between two awaits; its answer comes from the
/// run loop), the fake answers with what the test set
/// ([`FakePages::set_check`]) or with `innerword_page::static_check`.
pub(crate) fn check_bundle(
    engine: &PageEngine,
    bundle: &innerword_page::Bundle,
    rules_dir: PathBuf,
) -> Answer<innerword_page::CheckReport> {
    let (tx, rx) = answer();
    match engine {
        PageEngine::WebKit => {
            let request = innerword_page::check_request(bundle);
            let source = innerword_web::MemorySource::new(bundle.served.iter().map(|file| {
                (
                    file.path.clone(),
                    ServedFile {
                        media_type: file.media_type.clone(),
                        bytes: file.bytes.clone(),
                    },
                )
            }));
            let input = innerword_web::CheckInput {
                source: Arc::new(source),
                entry: request.entry,
                also: request.also,
                settle_min: std::time::Duration::from_millis(request.settle_min_ms),
                settle_max: std::time::Duration::from_millis(request.settle_max_ms),
                limit: std::time::Duration::from_millis(request.limit_ms),
                rules_dir,
            };
            innerword_web::check(input, move |outcome| tx.send(report_of(outcome)));
        }
        PageEngine::Fake(fake) => fake.check(bundle, tx),
    }
    rx
}

// ----- the fake engine --------------------------------------------------------------

/// What the fake engine records, and the levers a test pulls. One open view
/// at a time, as the real engine.
#[derive(Default)]
pub struct FakePages {
    state: RefCell<FakeState>,
    /// Snapshots handed to `Window::drop_image` by the workspace.
    dropped: Cell<usize>,
}

#[derive(Default)]
struct FakeState {
    opened: Vec<(DocumentId, String)>,
    /// The origin host of each view made, in order.
    hosts: Vec<String>,
    /// The events of the open view.
    events: Option<Rc<dyn innerword_web::PageEvents>>,
    /// The inbox of the open view (snapshots and finds answer through it).
    inbox: Option<SharedInbox>,
    zoom: Option<u16>,
    finds: Vec<(String, bool)>,
    snapshots_made: usize,
    first_responder_is_page: bool,
    edits: Vec<&'static str>,
    check: Option<innerword_page::CheckReport>,
    reloads: usize,
    hidden: bool,
    holes: Vec<innerword_web::Rect>,
    appearance: Option<innerword_web::Appearance>,
    /// What a find answers: whether the page's text holds the query.
    text: String,
    evaluated: Vec<String>,
    /// Checks are answered only when the test lets them ([`FakePages::hold_checks`]).
    checks_held: bool,
    /// The checks that wait for that.
    waiting_checks: Vec<(AnswerTx<innerword_page::CheckReport>, innerword_page::CheckReport)>,
    /// How many checks were asked for.
    checks: usize,
}

impl FakePages {
    /// The pages a view was made for, with the entry it loaded, in order.
    pub fn opened(&self) -> Vec<(DocumentId, String)> {
        self.state.borrow().opened.clone()
    }

    /// The host of the origin each view was given (`innerword-page://<host>/`),
    /// in the order the views were made.
    pub fn hosts(&self) -> Vec<String> {
        self.state.borrow().hosts.clone()
    }

    /// What WebKit tells the open view's workspace, kept by a test to post
    /// as WebKit would (also after the view went).
    pub fn events(&self) -> Option<Rc<dyn innerword_web::PageEvents>> {
        self.state.borrow().events.clone()
    }

    /// The open view reports "loaded".
    pub fn finish_loading(&self) {
        let events = self.state.borrow().events.clone();
        if let Some(events) = events {
            events.loaded();
        }
    }

    /// The open view reports that its WebContent process ended.
    pub fn crash(&self) {
        let events = self.state.borrow().events.clone();
        if let Some(events) = events {
            events.crashed();
        }
    }

    /// The open view reports a click into the page while the keyboard was
    /// elsewhere.
    pub fn click_into_page(&self) {
        let events = self.state.borrow().events.clone();
        if let Some(events) = events {
            self.state.borrow_mut().first_responder_is_page = true;
            events.wants_focus();
        }
    }

    /// The reader chose item `id` of WebKit's menu (ids from 1, in the order
    /// of the app's items).
    pub fn choose_in_menu(&self, id: u32) {
        let events = self.state.borrow().events.clone();
        if let Some(events) = events {
            events.menu_chosen(id);
        }
    }

    /// The app's items WebKit's menu would get now: their titles (with the
    /// key after a tab).
    pub fn menu_titles(&self) -> Vec<String> {
        let events = self.state.borrow().events.clone();
        events
            .map(|events| {
                events
                    .menu_items(None)
                    .into_iter()
                    .map(|entry| entry.title)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The zoom the view was given last, in percent.
    pub fn zoom(&self) -> Option<u16> {
        self.state.borrow().zoom
    }

    /// Every find that was asked: (query, backwards).
    pub fn finds(&self) -> Vec<(String, bool)> {
        self.state.borrow().finds.clone()
    }

    pub fn snapshots_made(&self) -> usize {
        self.state.borrow().snapshots_made
    }

    pub fn snapshots_dropped(&self) -> usize {
        self.dropped.get()
    }

    /// Whether the view is the window's first responder.
    pub fn first_responder_is_page(&self) -> bool {
        self.state.borrow().first_responder_is_page
    }

    /// The edit commands sent to the view: "copy", "select_all", ...
    pub fn edits(&self) -> Vec<&'static str> {
        self.state.borrow().edits.clone()
    }

    /// The next check's answer (the import, WP9).
    pub fn set_check(&self, report: innerword_page::CheckReport) {
        self.state.borrow_mut().check = Some(report);
    }

    /// The answer [`FakePages::set_check`] left, taken.
    pub fn take_check(&self) -> Option<innerword_page::CheckReport> {
        self.state.borrow_mut().check.take()
    }

    /// From now on a check is answered only at [`FakePages::release_checks`]
    /// (a test holds the import in "checking").
    pub fn hold_checks(&self) {
        self.state.borrow_mut().checks_held = true;
    }

    /// Answers the checks that wait, and every later one at once.
    pub fn release_checks(&self) {
        let waiting = {
            let mut state = self.state.borrow_mut();
            state.checks_held = false;
            std::mem::take(&mut state.waiting_checks)
        };
        for (tx, report) in waiting {
            tx.send(report);
        }
    }

    /// How many checks the import asked for.
    pub fn checks_asked(&self) -> usize {
        self.state.borrow().checks
    }

    /// The fake's check of `bundle`: what [`FakePages::set_check`] left, else
    /// `innerword_page::static_check`; held while [`FakePages::hold_checks`] says so.
    fn check(&self, bundle: &innerword_page::Bundle, tx: AnswerTx<innerword_page::CheckReport>) {
        let report = self
            .take_check()
            .unwrap_or_else(|| innerword_page::static_check(bundle));
        let mut state = self.state.borrow_mut();
        state.checks += 1;
        if state.checks_held {
            state.waiting_checks.push((tx, report));
        } else {
            drop(state);
            tx.send(report);
        }
    }

    /// A key the view received: when the app takes it (`takes_key`), it is
    /// handed on to GPUI as the real view hands it to GPUI's view. Returns
    /// whether it was handed on.
    pub fn press(&self, key: &str) -> bool {
        let (events, inbox) = {
            let state = self.state.borrow();
            (state.events.clone(), state.inbox.clone())
        };
        let (Some(events), Some(inbox), Some(press)) = (events, inbox, key_press_of(key)) else {
            return false;
        };
        if !events.takes_key(&press) {
            return false;
        }
        let Ok(keystroke) = Keystroke::parse(key) else {
            return false;
        };
        post(&inbox, PageEvent::KeyHandedOn(keystroke));
        true
    }

    /// How often the view loaded its entry again.
    pub fn reloads(&self) -> usize {
        self.state.borrow().reloads
    }

    /// Whether the view is hidden (a snapshot stands in, or the page stopped
    /// working).
    pub fn hidden(&self) -> bool {
        self.state.borrow().hidden
    }

    /// The holes the view has, in its own points.
    pub fn holes(&self) -> Vec<innerword_web::Rect> {
        self.state.borrow().holes.clone()
    }

    pub fn appearance(&self) -> Option<innerword_web::Appearance> {
        self.state.borrow().appearance
    }

    /// The scripts evaluated in the view.
    pub fn evaluated(&self) -> Vec<String> {
        self.state.borrow().evaluated.clone()
    }

    pub(crate) fn note_dropped(&self) {
        self.dropped.set(self.dropped.get() + 1);
    }
}

/// The fake engine's view of one page.
pub struct FakeView {
    pages: Rc<FakePages>,
}

impl FakeView {
    /// A view of page `id`, its text `text`, loading `entry`.
    pub fn new(
        pages: Rc<FakePages>,
        id: DocumentId,
        host: &str,
        entry: &str,
        text: &str,
        events: Rc<dyn innerword_web::PageEvents>,
        inbox: SharedInbox,
    ) -> FakeView {
        {
            let mut state = pages.state.borrow_mut();
            state.opened.push((id, entry.to_string()));
            state.hosts.push(host.to_string());
            state.events = Some(events);
            state.inbox = Some(inbox);
            state.zoom = Some(100);
            state.hidden = false;
            state.holes.clear();
            state.first_responder_is_page = false;
            state.text = text.to_string();
        }
        FakeView { pages }
    }

    fn post(&self, event: PageEvent) {
        let inbox = self.pages.state.borrow().inbox.clone();
        if let Some(inbox) = inbox {
            post(&inbox, event);
        }
    }

    pub fn set_hidden(&self, hidden: bool) {
        self.pages.state.borrow_mut().hidden = hidden;
    }

    pub fn set_holes(&self, holes: &[innerword_web::Rect]) {
        self.pages.state.borrow_mut().holes = holes.to_vec();
    }

    pub fn focus(&self) {
        self.pages.state.borrow_mut().first_responder_is_page = true;
    }

    pub fn unfocus(&self) {
        self.pages.state.borrow_mut().first_responder_is_page = false;
    }

    pub fn is_focused(&self) -> bool {
        self.pages.state.borrow().first_responder_is_page
    }

    pub fn snapshot(&self) {
        self.pages.state.borrow_mut().snapshots_made += 1;
        self.post(PageEvent::Snapshot(Some(Bitmap {
            width: 2,
            height: 2,
            bgra: vec![255; 16],
        })));
    }

    pub fn find(&self, query: &str, backwards: bool, case_sensitive: bool) {
        let found = {
            let mut state = self.pages.state.borrow_mut();
            state.finds.push((query.to_string(), backwards));
            !query.is_empty()
                && if case_sensitive {
                    state.text.contains(query)
                } else {
                    state.text.to_lowercase().contains(&query.to_lowercase())
                }
        };
        self.post(PageEvent::Found {
            query: query.to_string(),
            found,
        });
    }

    pub fn set_zoom(&self, percent: u16) {
        self.pages.state.borrow_mut().zoom = Some(percent);
    }

    pub fn set_appearance(&self, appearance: innerword_web::Appearance) {
        self.pages.state.borrow_mut().appearance = Some(appearance);
    }

    pub fn edit(&self, command: innerword_web::EditCommand) {
        use innerword_web::EditCommand as E;
        let name = match command {
            E::Copy => "copy",
            E::Cut => "cut",
            E::Paste => "paste",
            E::SelectAll => "select_all",
            E::Undo => "undo",
            E::Redo => "redo",
        };
        self.pages.state.borrow_mut().edits.push(name);
    }

    pub fn reload(&self) {
        self.pages.state.borrow_mut().reloads += 1;
    }

    pub fn evaluate(&self, script: &str) {
        self.pages
            .state
            .borrow_mut()
            .evaluated
            .push(script.to_string());
    }
}

impl Drop for FakeView {
    fn drop(&mut self) {
        let mut state = self.pages.state.borrow_mut();
        state.events = None;
        state.inbox = None;
        state.first_responder_is_page = false;
    }
}

/// A fixed one-page PDF: what the fake engine prints and exports.
pub fn fake_pdf() -> Vec<u8> {
    let body = b"%PDF-1.4\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj\n3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 595.28 841.89]>>endobj\ntrailer<</Root 1 0 R>>\n%%EOF\n";
    body.to_vec()
}
