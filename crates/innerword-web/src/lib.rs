//! WebKit for Inner Word's pages: the live view of an open page in the window
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
//!   `innerword-page://<host>/`, and every answer carries [`CONTENT_SECURITY_POLICY`]
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
//! * no bridge to the app in a page's view: no script message handler, no user script
//!   (the check has both, in a content world of its own). A whiteboard's view
//!   ([`ViewExtras`]) has one handler ([`Bridge`]), checked for every message: its name,
//!   the main frame, the view's own origin, a string under the size cap; whatever fails is
//!   dropped and counted, and what passes only reaches [`PageEvents::message`]. The main
//!   frame check cannot tell a frame of the view's own origin that posts through `top`
//!   from the main frame, so a view with the bridge loads no frame at all (every
//!   subframe navigation is cancelled, `srcdoc` included: the whiteboard's policies
//!   refuse a frame of a file and an inline script, not a `srcdoc` frame's script of the
//!   view's own origin). It also loads no other document after its entry and has a stricter
//!   policy ([`BOARD_CONTENT_SECURITY_POLICY`]).
//!
//! Everything runs on the main thread, except [`PageSource::file`], which is called on
//! a background queue. Every callback from WebKit or AppKit reaches the owner on the
//! main thread, inside AppKit's own call: an owner under GPUI only posts to a channel
//! there and never enters GPUI.

// `objc`'s macros ask for a cfg this crate does not have.
#![allow(unexpected_cfgs)]

use std::{collections::HashMap, fmt, path::PathBuf, sync::Arc, time::Duration};

mod bridge;
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
mod script;
mod selectors;
mod view;

pub use check::{
    check, check_blocking, check_blocking_with_policy, check_with_policy, print_blocking,
    print_offscreen,
};
pub use page::WebPage;
pub use script::{run_script, run_script_blocking};
pub use selectors::{available, check_selectors};

/// The scheme a page's files are served under: `innerword-page://page-12/index.html` in
/// the view, `innerword-page://check/index.html` in the check.
pub const SCHEME: &str = "innerword-page";

/// The policy on every answer of the scheme handler, whatever its type.
pub const CONTENT_SECURITY_POLICY: &str = "default-src 'self' data: blob: 'unsafe-inline' 'unsafe-eval'; form-action 'none'; base-uri 'self'";

/// The policy of every answer of a whiteboard's view (`ViewExtras::csp`): the bundle is
/// our own code and needs neither an inline script nor eval, so neither runs, nor does
/// any frame (an `iframe` element of a pasted scene renders its `srcdoc` as a frame,
/// which inherits this policy and could run nothing even before the glue deletes it).
pub const BOARD_CONTENT_SECURITY_POLICY: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self' data:; connect-src 'self' data: blob:; worker-src 'self' blob:; frame-src 'none'; child-src 'none'; object-src 'none'; form-action 'none'; base-uri 'self'";

/// The longest side of a PDF [`run_script`] makes, in points: Acrobat's limit, and the
/// whiteboard's export scales its drawing down to it (`docs/whiteboard-design.md`,
/// section 5; threat 12 of section 8). The rectangle is capped here too, whatever the page
/// answers, so a broken or forged export page cannot have WebKit draw a page of any size.
pub const MAX_PDF_SIDE: f64 = 14_400.0;

/// The policy of the whiteboard's export page: the board's, with WebAssembly allowed
/// for Excalidraw's font subsetter (`'wasm-unsafe-eval'`, which allows WebAssembly and
/// not eval). Where WebKit does not know that keyword (before Safari 16's WebKit, 614,
/// which a Mac of macOS 12.3 can still have) it is ignored and WebAssembly is refused:
/// [`board_export_policy`] then answers
/// [`BOARD_EXPORT_CONTENT_SECURITY_POLICY_BEFORE_WASM_KEYWORD`].
pub const BOARD_EXPORT_CONTENT_SECURITY_POLICY: &str = "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self' data:; connect-src 'self' data: blob:; worker-src 'self' blob:; frame-src 'none'; child-src 'none'; object-src 'none'; form-action 'none'; base-uri 'self'";

/// The export page's policy for a WebKit that does not know `'wasm-unsafe-eval'`:
/// `'unsafe-eval'` in its place (an older WebKit governs WebAssembly by that keyword).
/// Only the export page ever has it; it loads a stored scene that `innerword_board` read
/// and filtered, and has no bridge.
pub const BOARD_EXPORT_CONTENT_SECURITY_POLICY_BEFORE_WASM_KEYWORD: &str = "default-src 'self'; script-src 'self' 'unsafe-eval'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self' data:; connect-src 'self' data: blob:; worker-src 'self' blob:; frame-src 'none'; child-src 'none'; object-src 'none'; form-action 'none'; base-uri 'self'";

/// The first WebKit (Safari 16's) that knows the keyword `'wasm-unsafe-eval'`.
pub const WASM_KEYWORD_SINCE_WEBKIT: u32 = 614;

/// The export page's policy for the WebKit of this Mac:
/// [`BOARD_EXPORT_CONTENT_SECURITY_POLICY`] from WebKit 614 on, else
/// [`BOARD_EXPORT_CONTENT_SECURITY_POLICY_BEFORE_WASM_KEYWORD`]. Asked of WebKit's
/// bundle (no web view is made).
pub fn board_export_policy() -> &'static str {
    export_policy_for(check::engine_major())
}

fn export_policy_for(webkit_major: Option<u32>) -> &'static str {
    match webkit_major {
        Some(major) if major < WASM_KEYWORD_SINCE_WEBKIT => {
            BOARD_EXPORT_CONTENT_SECURITY_POLICY_BEFORE_WASM_KEYWORD
        }
        _ => BOARD_EXPORT_CONTENT_SECURITY_POLICY,
    }
}

/// The content rule list: every load of another kind of address is blocked.
pub const BLOCK_RULES: &str = r#"[
 {"trigger":{"url-filter":"^https?:"},"action":{"type":"block"}},
 {"trigger":{"url-filter":"^wss?:"},"action":{"type":"block"}},
 {"trigger":{"url-filter":"^ftp:"},"action":{"type":"block"}},
 {"trigger":{"url-filter":"^file:"},"action":{"type":"block"}}
]"#;

/// The one identifier the rule list is compiled under; a change of BLOCK_RULES changes it.
pub const RULE_LIST_ID: &str = "innerword-page-block-v1";

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

/// A whiteboard's view: its one script message handler (`docs/whiteboard-design.md`,
/// section 4.2), under `name`, in the page world, for the main frame of the view's own
/// origin only, for a string of at most `max_bytes` UTF-8 bytes (a lone surrogate in it
/// made U+FFFD). A view with a bridge loads no frame inside its page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bridge {
    pub name: &'static str,
    pub max_bytes: usize,
}

/// What only a whiteboard's view has. `Default` is a page's view, made exactly as
/// [`WebPage::new`] makes it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ViewExtras {
    /// The one script message handler; `None`: no bridge (a page's view).
    pub bridge: Option<Bridge>,
    /// No main-frame navigation but the view's own loads of its entry (the first one,
    /// [`WebPage::reload`]): a page's `location =`, `location.reload()`, a link, a form,
    /// `window.open` into the view, also to its own files, are cancelled. No
    /// [`PageEvents::alert`] either.
    pub one_document: bool,
    /// The policy of every answer of the view's scheme handler, in place of
    /// [`CONTENT_SECURITY_POLICY`].
    pub csp: Option<&'static str>,
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
    /// to the address of the link the reader acted on (the one under the pointer at a real
    /// mouseDown:, the one with the focus at a real `enter`), within 1 s of a real
    /// mouseUp:/keyDown: of the view (and `_isUserInitiated` where it answers), once per
    /// act; or the menu's "Open Link in Browser". A script's click never comes here, also
    /// not one in a listener of the reader's click or key that clicks another link.
    fn open_externally(&self, url: String);
    /// A page's `alert`: at most one in 5 s comes here; the others are answered and
    /// dropped.
    fn alert(&self, message: String);
    /// The owner's items for the right-click menu; `link` is the link under the pointer.
    /// A title may end in a tab and the keymap's keystroke (`"Find in Page…\tcmd-f"`):
    /// the menu draws it as the item's key.
    fn menu_items(&self, link: Option<&str>) -> Vec<MenuEntry>;
    fn menu_chosen(&self, id: u32);
    /// A message of the bridge ([`ViewExtras::bridge`]) that passed every check, as the
    /// page posted it. Only posts; nothing it says is run. A view without a bridge never
    /// calls it.
    fn message(&self, _body: String) {}
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
    /// Messages of the bridge that passed every check and reached
    /// [`PageEvents::message`].
    pub messages_received: u64,
    /// Messages of the bridge that failed a check (another name, a frame that is not
    /// the main one, another origin, too large, not a string) and were dropped.
    pub messages_dropped: u64,
}

// ----- one script, offscreen

/// What [`run_script`] loads and runs: the source's `entry` offscreen at a host of its
/// own (`board-export`), every answer with `csp`, then one `callAsyncJavaScript` in the
/// page world with `script` as its body and `argument_json` as its argument `arg` (a
/// string: the script parses it).
#[derive(Clone)]
pub struct ScriptInput {
    pub source: Arc<dyn PageSource>,
    pub entry: String,
    /// A body for `callAsyncJavaScript`: it may `await`, reads `arg`, and returns a
    /// string (or anything WebKit hands back: written out as `WebPage::evaluate` does).
    pub script: String,
    pub argument_json: String,
    /// The script answers `{"width": w, "height": h}` (CSS pixels): after the fonts and
    /// two animation frames, a one-page PDF of `(0, 0, ceil(w), ceil(h))` is made, each
    /// side at most [`MAX_PDF_SIDE`] whatever the page answered.
    pub pdf: bool,
    pub csp: &'static str,
    /// For everything together: the load, the script, the PDF.
    pub limit: Duration,
    /// Where the rule list is compiled and kept.
    pub rules_dir: PathBuf,
}

/// What [`run_script`] answers: the script's value as text, and the PDF when asked.
#[derive(Clone, Debug)]
pub struct ScriptOutput {
    pub value: String,
    pub pdf: Option<PrintedPdf>,
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

    /// How many handlers of a whiteboard's bridge are alive in this process (each goes
    /// with its view).
    pub fn live_bridges() -> usize {
        crate::page::live_bridges()
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

    /// Whether the data store of the views keeps nothing on disk.
    pub fn view_store_is_not_persistent() -> bool {
        use objc::{msg_send, runtime::BOOL, sel, sel_impl};
        // SAFETY: WebKit's public `-[WKWebsiteDataStore isPersistent]`, on the main
        // thread (the test program's).
        let persistent: BOOL = unsafe { msg_send![crate::config::view_data_store(), isPersistent] };
        persistent == objc::runtime::NO
    }
}

#[cfg(test)]
mod policy_tests {
    use super::*;

    #[test]
    fn the_export_page_takes_unsafe_eval_only_where_webkit_does_not_know_the_wasm_keyword() {
        assert_eq!(
            export_policy_for(Some(613)),
            BOARD_EXPORT_CONTENT_SECURITY_POLICY_BEFORE_WASM_KEYWORD
        );
        assert_eq!(
            export_policy_for(Some(614)),
            BOARD_EXPORT_CONTENT_SECURITY_POLICY
        );
        assert_eq!(
            export_policy_for(Some(621)),
            BOARD_EXPORT_CONTENT_SECURITY_POLICY
        );
        assert_eq!(
            export_policy_for(None),
            BOARD_EXPORT_CONTENT_SECURITY_POLICY
        );
    }

    #[test]
    fn the_board_policies_differ_only_in_what_runs_webassembly() {
        assert_eq!(
            BOARD_EXPORT_CONTENT_SECURITY_POLICY.replace(" 'wasm-unsafe-eval'", ""),
            BOARD_CONTENT_SECURITY_POLICY
        );
        assert_eq!(
            BOARD_EXPORT_CONTENT_SECURITY_POLICY_BEFORE_WASM_KEYWORD.replace(" 'unsafe-eval'", ""),
            BOARD_CONTENT_SECURITY_POLICY
        );
        for policy in [
            BOARD_CONTENT_SECURITY_POLICY,
            BOARD_EXPORT_CONTENT_SECURITY_POLICY,
        ] {
            // Inline styles only (Excalidraw and Mermaid set them); no inline script.
            assert_eq!(policy.matches("'unsafe-inline'").count(), 1);
            assert!(policy.contains("style-src 'self' 'unsafe-inline'"));
            assert!(policy.contains("script-src 'self'"));
            assert!(policy.contains("frame-src 'none'"));
            assert!(!policy.contains("http"));
        }
        assert!(!BOARD_CONTENT_SECURITY_POLICY.contains("eval"));
    }
}

#[cfg(test)]
mod rule_list_tests {
    use super::{BLOCK_RULES, RULE_LIST_ID};

    /// FNV-1a, 64 bits: enough to notice that the rules changed.
    fn fingerprint(text: &str) -> u64 {
        text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
        })
    }

    /// The compiled rule list is looked up by its identifier and compiled again only
    /// when it is missing: rules changed under the same identifier would leave every
    /// library on the old ones. Whoever changes `BLOCK_RULES` changes `RULE_LIST_ID`
    /// (`-v2`, ...) and the two values here.
    #[test]
    fn the_rule_list_identifier_changes_with_the_rules() {
        assert_eq!(
            (RULE_LIST_ID, fingerprint(BLOCK_RULES)),
            ("innerword-page-block-v1", 0x2f94_723f_e271_e28d),
            "BLOCK_RULES changed: give RULE_LIST_ID a new version and pin both here"
        );
    }
}
