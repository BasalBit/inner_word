//! WebKit for The Writer's pages: the live view of an open page in the window
//! ([`WebPage`]), the check that decides whether a page is whole offline ([`check`],
//! [`check_blocking`]), and a page printed to a PDF ([`print_offscreen`],
//! [`print_blocking`], [`WebPage::print_to_pdf`]). No GPUI: the app hands this crate
//! GPUI's `NSView` and hears from it through [`PageEvents`].
//!
//! A page is untrusted content; it may have been written by a model that read
//! anything. Every web view made here gives it no way to the network, to the user's
//! files or to the app (`docs/pages-design.md`, section 5):
//!
//! * its files come from a [`PageSource`] through a `WKURLSchemeHandler` on
//!   `writer-page://<host>/`, and every answer carries [`CONTENT_SECURITY_POLICY`]
//!   (`default-src 'self' data: blob:`: nothing from anywhere else);
//! * a content rule list blocks every `http(s)`, `ws(s)`, `ftp` and `file` load again
//!   ([`BLOCK_RULES`]);
//! * WebRTC and media devices are off (private switches, which [`available`] asks
//!   for: neither the policy nor the rule list sees ICE);
//! * the data stores open no connection ahead of a navigation (WebKit preconnects to
//!   the host of a link while it asks the policy, also for a link a script clicks:
//!   `allowsServerPreconnect`, private, also asked for by [`available`]), and DNS
//!   prefetching is off;
//! * a navigation goes only to the page's own files; a link to another site opens in
//!   the reader's browser only when the reader clicked it or pressed a key on it in the
//!   second before ([`PageEvents::open_externally`]); no window, download, panel,
//!   camera, microphone, link preview, full screen or inspector;
//! * no autoplay; the check's media are muted;
//! * no bridge to the app in the view: no script message handler, no user script (the
//!   check has both, in a content world of its own).
//!
//! Everything runs on the main thread, except [`PageSource::file`], which is called on
//! a background queue. Every callback from WebKit or AppKit reaches the owner on the
//! main thread, inside AppKit's own call: an owner under GPUI only posts to a channel
//! there and never enters GPUI.

// `objc`'s macros ask for a cfg this crate does not have.
#![allow(unexpected_cfgs)]

use std::{collections::HashMap, fmt, path::PathBuf, sync::Arc, time::Duration};

mod check;
mod config;
mod delegate;
mod frame;
mod http;
pub mod menu;
mod ns;
mod page;
mod pixels;
mod policy;
mod print;
mod rules;
mod scheme;
mod selectors;
mod view;

pub use check::{check, check_blocking, print_blocking, print_offscreen};
pub use page::WebPage;
pub use selectors::{available, check_selectors};

/// The scheme a page's files are served under: `writer-page://page-12/index.html` in
/// the view, `writer-page://check/index.html` in the check.
pub const SCHEME: &str = "writer-page";

/// The policy on every answer of the scheme handler, whatever its type.
pub const CONTENT_SECURITY_POLICY: &str = "default-src 'self' data: blob: 'unsafe-inline' 'unsafe-eval'; form-action 'none'; base-uri 'self'";

/// The content rule list: every load of another kind of address is blocked.
pub const BLOCK_RULES: &str = r#"[
 {"trigger":{"url-filter":"^https?:"},"action":{"type":"block"}},
 {"trigger":{"url-filter":"^wss?:"},"action":{"type":"block"}},
 {"trigger":{"url-filter":"^ftp:"},"action":{"type":"block"}},
 {"trigger":{"url-filter":"^file:"},"action":{"type":"block"}}
]"#;

/// The one identifier the rule list is compiled under; a change of BLOCK_RULES changes it.
pub const RULE_LIST_ID: &str = "writer-page-block-v1";

/// Where the bytes of a page come from. Called on a background queue.
pub trait PageSource: Send + Sync + 'static {
    /// The file at `path` (no leading slash, percent-decoded); None answers 404.
    fn file(&self, path: &str) -> Option<ServedFile>;
}

/// One file as it is served: its media type (from the manifest) and its bytes.
#[derive(Clone)]
pub struct ServedFile {
    pub media_type: String,
    pub bytes: Arc<[u8]>,
}

impl fmt::Debug for ServedFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServedFile")
            .field("media_type", &self.media_type)
            .field("bytes", &self.bytes.len())
            .finish()
    }
}

/// A page's files in memory.
pub struct MemorySource {
    files: HashMap<String, ServedFile>,
}

impl MemorySource {
    pub fn new(files: impl IntoIterator<Item = (String, ServedFile)>) -> MemorySource {
        MemorySource {
            files: files
                .into_iter()
                .map(|(path, file)| (path.trim_start_matches('/').to_string(), file))
                .collect(),
        }
    }
}

impl PageSource for MemorySource {
    fn file(&self, path: &str) -> Option<ServedFile> {
        self.files.get(path.trim_start_matches('/')).cloned()
    }
}

/// A rectangle in points, origin at the top left of the window's content view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

// ----- the check

/// What a check loads: the entry, then every other HTML file of the page.
#[derive(Clone)]
pub struct CheckInput {
    pub source: Arc<dyn PageSource>,
    pub entry: String,
    /// The other HTML files, each loaded after the entry (settling 0.3 s to 1 s, or
    /// less when `settle_min` / `settle_max` are less).
    pub also: Vec<String>,
    /// After `load`, the check waits until nothing was answered or blocked for 500 ms,
    /// at least `settle_min` and at most `settle_max`.
    pub settle_min: Duration,
    pub settle_max: Duration,
    /// For the whole check, every load together.
    pub limit: Duration,
    /// Where the rule list is compiled and kept.
    pub rules_dir: PathBuf,
}

/// What a check found.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CheckOutcome {
    /// "WebKit 621.1".
    pub engine: String,
    pub elapsed: Duration,
    /// One per file loaded, the entry first. When the limit ran out, the files not
    /// loaded by then are missing here (an entry still loading is here, not finished).
    pub loads: Vec<LoadOutcome>,
    /// Set only when the check itself could not run (WebKit cannot show pages here, the
    /// block list could not be compiled); the sentence says why.
    pub failure: Option<String>,
}

/// What one load of one HTML file showed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LoadOutcome {
    pub path: String,
    /// `document.title` after the scripts ran, trimmed.
    pub title: String,
    /// The text of the first `<h1>`.
    pub first_heading: String,
    /// The text as shown, then a blank line and the text that is not shown.
    pub text: String,
    /// `load` came.
    pub finished: bool,
    pub navigation_error: Option<String>,
    /// The WebContent process ended while it loaded.
    pub crashed: bool,
    /// Every path the scheme handler answered 404, once each.
    pub missing: Vec<String>,
    /// Every request that was blocked: (address, directive). The directive is the
    /// policy's (`font-src`, `connect-src`, ...) or `navigation` for a navigation the
    /// policy cancelled.
    pub blocked: Vec<(String, String)>,
    /// The first 20 script errors.
    pub script_errors: Vec<String>,
    /// Requests the scheme handler answered.
    pub requests: usize,
    /// Frames of SVG, XHTML or XML whose document the probe did not report from (so
    /// their requests could not be watched).
    pub unwatched_frames: Vec<String>,
}

// ----- a PDF of a page as it was made

/// What a page printed offscreen is loaded from.
#[derive(Clone)]
pub struct PdfInput {
    pub source: Arc<dyn PageSource>,
    pub entry: String,
    pub settle_max: Duration,
    pub limit: Duration,
    pub rules_dir: PathBuf,
}

/// A PDF WebKit printed.
#[derive(Clone)]
pub struct PrintedPdf {
    pub bytes: Vec<u8>,
    pub pages: usize,
    /// The size of the first page in points.
    pub paper: (f64, f64),
}

impl fmt::Debug for PrintedPdf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PrintedPdf")
            .field("bytes", &self.bytes.len())
            .field("pages", &self.pages)
            .field("paper", &self.paper)
            .finish()
    }
}

// ----- the view

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Appearance {
    Light,
    Dark,
    System,
}

/// What a view shows.
#[derive(Clone)]
pub struct ViewConfig {
    /// The host of the page's origin: `page-<document id>`.
    pub host: String,
    pub entry: String,
    pub source: Arc<dyn PageSource>,
    pub rules_dir: PathBuf,
    pub appearance: Appearance,
    /// The colour under the page (`underPageBackgroundColor`), sRGB 0..1.
    pub background: (f64, f64, f64),
}

/// What the view tells its owner, on the main thread, from inside AppKit's callbacks.
/// The owner never enters GPUI in any of these: the notifying ones only post to a
/// channel; `takes_key` and `menu_items` answer from a snapshot the owner keeps
/// (section 6, "AppKit's callbacks never enter GPUI").
pub trait PageEvents: 'static {
    /// A load of the main frame finished: the entry, a reload, or another file of the
    /// page the reader went to.
    fn loaded(&self);
    fn failed(&self, message: String);
    /// The WebContent process ended. The view does not reload by itself: the owner
    /// calls `WebPage::reload` (or shows that the page stopped working).
    fn crashed(&self);
    /// The view became first responder by a click while the owner's focus was elsewhere
    /// (never for a focus that `WebPage::focus` asked for).
    fn wants_focus(&self);
    /// A key the view received: true hands it to the owner's view instead of the page.
    fn takes_key(&self, key: &KeyPress) -> bool;
    /// A link to another site (http, https, mailto) that the reader opened: a navigation
    /// within 1 s of a real mouseUp:/keyDown: of the view (and `_isUserInitiated` where it
    /// answers), or the menu's "Open Link in Browser". A script's click never comes here.
    fn open_externally(&self, url: String);
    /// A page's `alert`: at most one in 5 s comes here; the others are answered and
    /// dropped.
    fn alert(&self, message: String);
    /// The owner's items for the right-click menu; `link` is the link under the pointer.
    /// A title may end in a tab and the keymap's keystroke (`"Find in Page…\tcmd-f"`):
    /// the menu draws it as the item's key.
    fn menu_items(&self, link: Option<&str>) -> Vec<MenuEntry>;
    fn menu_chosen(&self, id: u32);
}

/// A key as AppKit reported it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyPress {
    pub key_code: u16,
    pub characters: String,
    pub characters_ignoring_modifiers: String,
    pub cmd: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub function: bool,
}

/// One of the owner's items of the right-click menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuEntry {
    pub id: u32,
    /// The title; after a tab, the key to draw beside it (see `PageEvents::menu_items`).
    pub title: String,
    pub enabled: bool,
    pub separator_before: bool,
}

/// Pixels of a snapshot: BGRA, alpha not premultiplied, rows from the top.
#[derive(Clone)]
pub struct Bitmap {
    /// Device pixels.
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

impl fmt::Debug for Bitmap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Bitmap")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("bytes", &self.bgra.len())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditCommand {
    Copy,
    Cut,
    Paste,
    SelectAll,
    Undo,
    Redo,
}

/// What the view's scheme handler and policy did since it was made.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ViewStats {
    /// Files answered (200, 206).
    pub served: usize,
    /// Requests answered 404.
    pub missing: usize,
    /// Navigations the policy cancelled.
    pub blocked_navigations: usize,
}

/// Starts compiling (or looking up) the rule list of `rules_dir` in the background, so
/// that the first page opens without waiting for it (the app calls it at launch). On
/// the main thread.
pub fn compile_rules(rules_dir: &std::path::Path) {
    if ns::on_main_thread() {
        rules::when_ready(rules_dir, |_| {});
    }
}

/// For this crate's test program: switches and questions no other caller needs.
#[doc(hidden)]
pub mod testing {
    use std::{path::Path, time::Duration};

    /// From now on WebKit's right-click menus are recorded (after the filter and the
    /// owner's items) and never shown.
    pub fn record_menus(on: bool) {
        crate::view::MENU_RECORD.with(|record| record.borrow_mut().on = on);
    }

    /// In every menu recorded from now on, the item of this title is chosen first.
    pub fn choose_in_menus(title: Option<&str>) {
        crate::view::MENU_RECORD
            .with(|record| record.borrow_mut().choose = title.map(str::to_string));
    }

    /// The menus recorded so far: per menu, its items as (identifier, action, title).
    pub fn recorded_menus() -> Vec<Vec<(String, String, String)>> {
        crate::view::MENU_RECORD.with(|record| record.borrow().menus.clone())
    }

    /// The identifiers of the rule lists in the store of `dir`.
    pub fn rule_lists_in(dir: &Path, then: impl FnOnce(Vec<String>) + 'static) {
        crate::rules::identifiers_in(dir, then);
    }

    /// The process forgets the rule lists it holds (the next view or check asks the
    /// store again).
    pub fn forget_rule_lists() {
        crate::rules::forget();
    }

    /// Runs the main run loop until `done` or `limit`; true when `done` answered true.
    pub fn run_until(limit: Duration, done: impl Fn() -> bool) -> bool {
        crate::ns::run_until(limit, done)
    }

    /// Makes this program's application object, with the activation policy
    /// "prohibited".
    pub fn program_application() {
        crate::ns::program_application();
    }

    /// The system's uptime: the clock of `NSEvent.timestamp`.
    pub fn uptime() -> f64 {
        crate::ns::uptime()
    }

    /// The pages of a PDF and the size of its first page.
    pub fn pdf_pages(bytes: &[u8]) -> Option<(usize, (f64, f64))> {
        crate::pixels::pdf_pages(bytes)
    }

    /// Runs `work` once on the main run loop after `delay` (from a timer's callback).
    pub fn after(delay: Duration, work: impl FnOnce() + 'static) {
        crate::ns::after(delay, work);
    }

    /// A borderless window far off screen (+1), as the check's.
    pub fn offscreen_window() -> *mut std::ffi::c_void {
        // SAFETY: on the main thread (the test program's).
        unsafe { crate::check::offscreen_window().cast() }
    }
}
