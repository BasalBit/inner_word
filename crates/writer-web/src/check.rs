//! The check of a page (section 3 of the design, "Whole or nothing"), and a page
//! printed offscreen.
//!
//! An offscreen window (borderless, far off screen, out of the Windows menu and of
//! window cycling, never key) holds a web view configured as the view of the window
//! is, plus muted media, with a data store of its own. The check adds a probe and its
//! message handler in a content world of its own (`writer-check`): a listener for
//! `securitypolicyviolation` in every frame, at document start, which the page's
//! scripts cannot reach, remove or forge. Each HTML file is loaded at
//! `writer-page://check/<path>`; after `load` the check waits until no request was
//! answered or blocked for 500 ms (at least `settle_min`, at most `settle_max`), and
//! reads the title, the first heading and the text in the same world.
//!
//! Everything here runs on the main thread from the run loop: WebKit's callbacks only
//! write down what happened, a timer moves the check on, and `done` is called from a
//! timer of its own, outside any caller's borrow.

use std::{
    cell::RefCell,
    collections::{HashSet, VecDeque},
    ffi::c_void,
    rc::{Rc, Weak},
    sync::Once,
    time::{Duration, Instant},
};

use block::ConcreteBlock;
use objc::{
    class,
    declare::ClassDecl,
    msg_send,
    runtime::{Class, NO, Object, Protocol, Sel, YES},
    sel, sel_impl,
};

use crate::{
    CheckInput, CheckOutcome, LoadOutcome, PdfInput, PrintedPdf, config, delegate, http,
    ns::{self, Id, NIL, NSRect, error_text, nsstring, release, retain, string_of},
    print, rules,
    scheme::{self, RequestLog, SchemeState},
    view,
};

/// The host of the check's files.
const HOST: &str = "check";
/// The content world of the probe and of reading the text.
const WORLD: &str = "writer-check";
/// The name of the probe's message handler (in that world only).
const HANDLER: &str = "writerCheck";
/// How long nothing may have been answered or blocked for a load to have settled.
const QUIET: Duration = Duration::from_millis(500);
/// How often the check looks at where it stands.
const POLL: Duration = Duration::from_millis(20);
/// The size of the offscreen window: a page as a laptop shows it.
const WINDOW: (f64, f64) = (1280., 900.);
/// At most this many script errors are kept per load.
const SCRIPT_ERRORS: usize = 20;

/// The probe: in every frame, at document start, in the check's own world.
const PROBE: &str = r#"(() => {
  const handlers = window.webkit && window.webkit.messageHandlers;
  const handler = handlers && handlers.writerCheck;
  if (!handler) return;
  const post = (message) => { try { handler.postMessage(JSON.stringify(message)); } catch (_) {} };
  window.addEventListener('securitypolicyviolation', (event) => {
    post({ kind: 'blocked', url: String(event.blockedURI || ''),
           directive: String(event.effectiveDirective || event.violatedDirective || '') });
  }, true);
  window.addEventListener('error', (event) => {
    if (typeof ErrorEvent !== 'undefined' && event instanceof ErrorEvent) {
      post({ kind: 'error', message: String(event.message || ''),
             source: String(event.filename || ''), line: Number(event.lineno || 0) });
    }
  }, true);
  window.addEventListener('unhandledrejection', (event) => {
    const reason = event.reason;
    post({ kind: 'error', message: 'Unhandled rejection: ' + String(reason && reason.message || reason),
           source: '', line: 0 });
  });
  post({ kind: 'ready', type: String(document.contentType || '') });
})();"#;

/// The text of a load: `innerText` (the text as shown, in reading order), with the
/// words that layout shows apart kept apart, then a blank line and the text that is
/// not shown (a closed `<details>`, a tab or a chapter not
/// shown, `hidden`, `display: none`): the text nodes outside `script`, `style`,
/// `noscript`, `template` and `head` that have no layout box (their element has no
/// client rectangles or is not visible to `checkVisibility()`, they lie in a closed
/// `<details>` outside its `<summary>`, or they themselves have no rectangles), one
/// line per block, the text of two elements in it a space apart unless it reads as one
/// word, a line left out when the shown text has it already.
///
/// `innerText` leaves margins, padding and gaps out: a tag after a word
/// (`<b>members</b><span class=tag>Recommended</span>`), a chip, the gutter of a code
/// listing and the labels of a chart's axis come out glued to their neighbour
/// ("membersRecommended", "18import", "050100"). So each junction of two shown text
/// nodes without white space between them is measured first: where the second starts
/// more than a quarter of an em (and twice the letter spacing) to the right of the
/// first on the same line, or on another line with a box that is not inline between
/// them (not a line of text that wraps), the first node gets a noncharacter
/// (U+FDD0 for a space, U+FDD1 for a line) at its end; `innerText` is read once, the
/// nodes are given back their text, and a mark followed by a word becomes a space or a
/// line (one followed by white space goes). Only white space is ever added. A junction
/// after an opening bracket or before closing punctuation is left as it is.
const TEXT: &str = r#"(() => {
  const root = document.body || document.documentElement;
  const skip = new Set(['SCRIPT', 'STYLE', 'NOSCRIPT', 'TEMPLATE', 'HEAD', 'TITLE']);
  const textNodes = () => document.createTreeWalker(root, NodeFilter.SHOW_TEXT, { acceptNode(node) {
    for (let element = node.parentElement; element; element = element.parentElement) {
      if (skip.has(element.tagName)) return NodeFilter.FILTER_REJECT;
    }
    return NodeFilter.FILTER_ACCEPT;
  } });
  const readShown = () => {
    if (!root) return '';
    const visible = [];
    const walk = textNodes();
    for (let node = walk.nextNode(); node; node = walk.nextNode()) {
      if (!node.data.trim()) continue;
      const range = document.createRange();
      range.selectNodeContents(node);
      if (range.getClientRects().length) visible.push(node);
    }
    // The rectangle of the first or the last character of a node (a whole code point).
    const edge = (node, first) => {
      const data = node.data;
      let start, end;
      if (first) {
        start = data.search(/\S/);
        end = start + (data.codePointAt(start) > 0xffff ? 2 : 1);
      } else {
        end = data.length;
        start = end - 1;
        if (start > 0 && /[\udc00-\udfff]/.test(data[start])) start -= 1;
      }
      const range = document.createRange();
      range.setStart(node, start);
      range.setEnd(node, end);
      const rects = range.getClientRects();
      return rects.length ? rects[first ? 0 : rects.length - 1] : null;
    };
    // Whether an element that is not inline stands between two nodes.
    const boxBetween = (a, b) => {
      let common = a.parentElement;
      while (common && !common.contains(b)) common = common.parentElement;
      for (const node of [a, b]) {
        for (let element = node.parentElement; element && element !== common; element = element.parentElement) {
          if (getComputedStyle(element).display !== 'inline') return true;
        }
      }
      return false;
    };
    const marks = [];
    for (let k = 1; k < visible.length; k++) {
      const a = visible[k - 1], b = visible[k];
      if (!/\S$/.test(a.data) || !/^\S/.test(b.data)) continue;
      if (/[(\[{\u201c\u2018\u00ab]$/.test(a.data) || /^[.,;:!?)\]}%\u201d\u2019\u00bb]/.test(b.data)) continue;
      const ra = edge(a, false), rb = edge(b, true);
      if (!ra || !rb) continue;
      const overlap = Math.min(ra.bottom, rb.bottom) - Math.max(ra.top, rb.top);
      const sameLine = overlap > 0.3 * Math.min(ra.height, rb.height);
      if (sameLine) {
        const style = getComputedStyle(a.parentElement);
        const size = parseFloat(style.fontSize) || 16;
        const spacing = Math.max(0, parseFloat(style.letterSpacing) || 0);
        if (rb.left - ra.right > 0.25 * size + 2 * spacing) marks.push([a, '\ufdd0']);
      } else if (boxBetween(a, b)) {
        marks.push([a, '\ufdd1']);
      }
    }
    const before = marks.map(([node]) => node.data);
    let text;
    try {
      for (const [node, mark] of marks) node.data = node.data + mark;
      text = String(root.innerText || root.textContent || '');
    } finally {
      marks.forEach(([node], i) => { node.data = before[i]; });
    }
    return text.replace(/\ufdd0(?=\S)/g, ' ').replace(/\ufdd1(?=\S)/g, '\n').replace(/[\ufdd0\ufdd1]/g, '');
  };
  const shown = readShown();
  const flat = shown.replace(/\s+/g, ' ');
  const blocks = new Set(['P', 'DIV', 'LI', 'UL', 'OL', 'DL', 'DT', 'DD', 'H1', 'H2', 'H3',
    'H4', 'H5', 'H6', 'SECTION', 'ARTICLE', 'ASIDE', 'HEADER', 'FOOTER', 'NAV', 'MAIN',
    'BLOCKQUOTE', 'PRE', 'TABLE', 'TBODY', 'THEAD', 'TFOOT', 'TR', 'TD', 'TH', 'CAPTION',
    'FIGURE', 'FIGCAPTION', 'DETAILS', 'SUMMARY', 'FORM', 'FIELDSET', 'LEGEND', 'ADDRESS',
    'BODY', 'HTML', 'BUTTON', 'LABEL', 'OPTION', 'DIALOG', 'HR', 'BR']);
  const lines = [];
  const seen = new Set();
  let block = null;
  let last = null;
  let buffer = '';
  const flush = () => {
    const line = buffer.replace(/\s+/g, ' ').trim();
    buffer = '';
    if (line && !seen.has(line) && !flat.includes(line)) { seen.add(line); lines.push(line); }
  };
  const inClosedDetails = (node) => {
    for (let element = node.parentElement; element; element = element.parentElement) {
      if (element.tagName === 'SUMMARY' && element.parentElement && element.parentElement.tagName === 'DETAILS') return false;
      if (element.tagName === 'DETAILS' && !element.open) return true;
    }
    return false;
  };
  const hidden = (node) => {
    const parent = node.parentElement;
    if (!parent) return false;
    if (parent.getClientRects().length === 0) return true;
    if (typeof parent.checkVisibility === 'function' && !parent.checkVisibility()) return true;
    if (inClosedDetails(node)) return true;
    const range = document.createRange();
    range.selectNodeContents(node);
    return range.getClientRects().length === 0;
  };
  const blockOf = (node) => {
    for (let element = node.parentElement; element; element = element.parentElement) {
      if (blocks.has(element.tagName)) return element;
    }
    return null;
  };
  if (root) {
    const walker = textNodes();
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      if (!node.data.trim()) continue;
      if (!hidden(node)) { flush(); block = null; continue; }
      const owner = blockOf(node);
      if (owner !== block) { flush(); block = owner; }
      // Text of two elements in one block is two words unless it reads as one (a word
      // continued in lower case, punctuation): hidden labels, a tab's items and the
      // titles of a chart are no "agoClosed" in the text that search and agents read.
      if (buffer && node.parentElement !== last && /\S$/.test(buffer)
          && /^\S/.test(node.data) && !(/[a-z]$/.test(buffer) && /^[a-z]/.test(node.data))
          && !/^[.,;:!?)\]}%]/.test(node.data)) buffer += ' ';
      buffer += node.data;
      last = node.parentElement;
    }
    flush();
  }
  const heading = document.querySelector('h1');
  const text = lines.length ? shown.replace(/\s+$/, '') + '\n\n' + lines.join('\n') : shown;
  return JSON.stringify({
    title: String(document.title || '').trim(),
    heading: heading ? String(heading.innerText || heading.textContent || '').replace(/\s+/g, ' ').trim() : '',
    text: text,
  });
})()"#;

/// The engine, as the manifest records it: "WebKit 621.1".
pub(crate) fn engine() -> String {
    // SAFETY: Foundation's own; the bundle of WKWebView is WebKit's.
    let version = unsafe {
        let bundle: Id = msg_send![class!(NSBundle), bundleForClass: class!(WKWebView)];
        if bundle.is_null() {
            String::new()
        } else {
            let version: Id =
                msg_send![bundle, objectForInfoDictionaryKey: nsstring("CFBundleVersion")];
            string_of(version)
        }
    };
    let mut parts = version.split('.');
    match (parts.next(), parts.next()) {
        (Some(major), Some(minor)) if !major.is_empty() => {
            // WebKit's bundle version leads with the system's own number (21621 is
            // 621 of macOS 27's WebKit).
            let major = if major.len() > 3 {
                &major[major.len() - 3..]
            } else {
                major
            };
            format!("WebKit {major}.{minor}")
        }
        _ => "WebKit".to_string(),
    }
}

// ---------------------------------------------------------------------------
// The offscreen window and its web view.

/// `NSWindowCollectionBehaviorTransient | NSWindowCollectionBehaviorIgnoresCycle`.
const OUT_OF_THE_WAY: usize = (1 << 3) | (1 << 6);

/// A borderless window far off screen (owned by the caller: +1), ordered in behind
/// every other window so that WebKit draws into it, never key, never in the Windows
/// menu or in window cycling.
pub(crate) unsafe fn offscreen_window() -> Id {
    // SAFETY: AppKit's own selectors with the types its headers declare.
    unsafe {
        let rect = NSRect::new(-20_000., -20_000., WINDOW.0, WINDOW.1);
        let window: Id = msg_send![class!(NSWindow), alloc];
        let window: Id = msg_send![window,
            initWithContentRect: rect
            styleMask: 0usize
            backing: 2usize
            defer: NO];
        let _: () = msg_send![window, setReleasedWhenClosed: NO];
        let _: () = msg_send![window, setExcludedFromWindowsMenu: YES];
        let _: () = msg_send![window, setCollectionBehavior: OUT_OF_THE_WAY];
        let _: () = msg_send![window, setHasShadow: NO];
        let _: () = msg_send![window, orderBack: NIL];
        window
    }
}

/// A web view of the check's configuration (owned by the caller: +1) in `window`'s
/// content view, with occlusion detection off where WebKit has the switch (an
/// offscreen window is occluded, and an occluded page runs no animation frames).
unsafe fn checked_web_view(window: Id, configuration: Id) -> Id {
    // SAFETY: AppKit's and WebKit's own; the private switch only where it answers.
    unsafe {
        let content: Id = msg_send![window, contentView];
        let bounds: NSRect = msg_send![content, bounds];
        let web_view = view::new_web_view(bounds, configuration, None);
        config::settle_web_view(web_view);
        config::mute(web_view);
        if ns::responds(web_view, sel!(_setWindowOcclusionDetectionEnabled:)) {
            let _: () = msg_send![web_view, _setWindowOcclusionDetectionEnabled: NO];
        }
        let _: () = msg_send![content, addSubview: web_view];
        web_view
    }
}

unsafe fn tear_down(window: Id, web_view: Id, delegate_object: Id) {
    // SAFETY: the references their makers took, given back once.
    unsafe {
        if !web_view.is_null() {
            let configuration: Id = msg_send![web_view, configuration];
            let controller: Id = msg_send![configuration, userContentController];
            let _: () = msg_send![controller, removeAllScriptMessageHandlers];
            let _: () = msg_send![controller, removeAllUserScripts];
            let _: () = msg_send![web_view, setNavigationDelegate: NIL];
            let _: () = msg_send![web_view, setUIDelegate: NIL];
            let _: () = msg_send![web_view, stopLoading];
            let _: () = msg_send![web_view, removeFromSuperview];
            release(web_view);
        }
        release(delegate_object);
        if !window.is_null() {
            let _: () = msg_send![window, orderOut: NIL];
            let _: () = msg_send![window, close];
            release(window);
        }
    }
}

// ---------------------------------------------------------------------------
// The probe's message handler.

const MESSAGES_CLASS: &str = "WriterWebCheckMessages";
const MESSAGES_STATE: &str = "writerReceiver";

type Receiver = Box<dyn Fn(&str, &str)>;

fn messages_class() -> &'static Class {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let Some(mut decl) = ClassDecl::new(MESSAGES_CLASS, class!(NSObject)) else {
            return;
        };
        if let Some(protocol) = Protocol::get("WKScriptMessageHandler") {
            decl.add_protocol(protocol);
        }
        decl.add_ivar::<*mut c_void>(MESSAGES_STATE);
        // SAFETY: the signatures of the selectors.
        unsafe {
            decl.add_method(
                sel!(userContentController:didReceiveScriptMessage:),
                received as extern "C" fn(&Object, Sel, Id, Id),
            );
            decl.add_method(
                sel!(dealloc),
                messages_dealloc as extern "C" fn(&mut Object, Sel),
            );
        }
        decl.register();
    });
    Class::get(MESSAGES_CLASS).expect("the check's message handler class")
}

fn new_messages(receiver: Receiver) -> Id {
    // SAFETY: `new` on a subclass of NSObject; the instance variable is ours.
    unsafe {
        let object: Id = msg_send![messages_class(), new];
        let receiver: *mut Receiver = Box::into_raw(Box::new(receiver));
        (*object).set_ivar::<*mut c_void>(MESSAGES_STATE, receiver.cast());
        object
    }
}

extern "C" fn messages_dealloc(this: &mut Object, _: Sel) {
    // SAFETY: the box `new_messages` made, freed once.
    unsafe {
        let receiver = *this.get_ivar::<*mut c_void>(MESSAGES_STATE);
        if !receiver.is_null() {
            this.set_ivar::<*mut c_void>(MESSAGES_STATE, std::ptr::null_mut());
            drop(Box::from_raw(receiver as *mut Receiver));
        }
        let _: () = msg_send![super(this, class!(NSObject)), dealloc];
    }
}

extern "C" fn received(this: &Object, _: Sel, _controller: Id, message: Id) {
    // SAFETY: a WKScriptMessage; its frame's request names the frame.
    unsafe {
        let receiver = *this.get_ivar::<*mut c_void>(MESSAGES_STATE);
        let Some(receiver) = (receiver as *const Receiver).as_ref() else {
            return;
        };
        let body: Id = msg_send![message, body];
        let body = string_of(body);
        let frame: Id = msg_send![message, frameInfo];
        let url = if frame.is_null() {
            String::new()
        } else {
            let request: Id = msg_send![frame, request];
            let url: Id = if request.is_null() {
                NIL
            } else {
                msg_send![request, URL]
            };
            let absolute: Id = if url.is_null() {
                NIL
            } else {
                msg_send![url, absoluteString]
            };
            string_of(absolute)
        };
        receiver(&body, &url);
    }
}

// ---------------------------------------------------------------------------
// The check.

/// One load of one HTML file.
struct Loading {
    navigation: Id,
    finished_at: Option<Instant>,
    /// The text was asked for.
    reading: bool,
    /// Over: failed, crashed, or read.
    ended: bool,
    min: Duration,
    max: Duration,
    outcome: LoadOutcome,
    /// Every frame's document that is not the main frame's: (URL, media type).
    frames: Vec<(String, String)>,
    /// The frames the probe reported from (URLs).
    ready: HashSet<String>,
    blocked_seen: HashSet<(String, String)>,
}

impl Loading {
    fn new(path: &str, min: Duration, max: Duration) -> Loading {
        Loading {
            navigation: NIL,
            finished_at: None,
            reading: false,
            ended: false,
            min,
            max,
            outcome: LoadOutcome {
                path: path.to_string(),
                ..LoadOutcome::default()
            },
            frames: Vec::new(),
            ready: HashSet::new(),
            blocked_seen: HashSet::new(),
        }
    }

    fn block(&mut self, url: String, directive: String) {
        if url.is_empty() {
            return;
        }
        if self.blocked_seen.insert((url.clone(), directive.clone())) {
            self.outcome.blocked.push((url, directive));
        }
    }

    /// The load is over: what the log and the frames say goes into the outcome.
    fn close(&mut self, log: &RequestLog) {
        self.outcome.requests = log.answered.get();
        self.outcome.missing = log.missing.borrow().clone();
        let unwatched: Vec<String> = self
            .frames
            .iter()
            .filter(|(url, media_type)| {
                watched_type(media_type) && !self.ready.contains(&without_fragment(url))
            })
            .map(|(url, _)| display(url))
            .collect();
        for frame in unwatched {
            if !self.outcome.unwatched_frames.contains(&frame) {
                self.outcome.unwatched_frames.push(frame);
            }
        }
    }
}

/// A document type that runs script and is no HTML: what the probe must report from.
fn watched_type(media_type: &str) -> bool {
    let essence = media_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    matches!(
        essence.as_str(),
        "image/svg+xml" | "application/xhtml+xml" | "text/xml" | "application/xml"
    ) || essence.ends_with("+xml")
}

fn without_fragment(url: &str) -> String {
    url.split('#').next().unwrap_or("").to_string()
}

/// A frame as a report names it: the path of a file of the page, else its address.
fn display(url: &str) -> String {
    match http::split_url(url) {
        Some((_, path)) => path,
        None => url.to_string(),
    }
}

struct CheckRun {
    started: Instant,
    limit: Duration,
    settle: (Duration, Duration),
    also_settle: (Duration, Duration),
    queue: VecDeque<String>,
    loads: Vec<LoadOutcome>,
    current: Option<Loading>,
    window: Id,
    web_view: Id,
    delegate: Id,
    world: Id,
    log: Rc<RequestLog>,
    timer: Option<ns::Timer>,
    done: Option<Box<dyn FnOnce(CheckOutcome)>>,
    engine: String,
}

type Run = Rc<RefCell<CheckRun>>;

/// The delegate's host for the check.
struct CheckHost(Weak<RefCell<CheckRun>>);

impl CheckHost {
    fn with_load(&self, work: impl FnOnce(&mut Loading, &RequestLog)) {
        if let Some(run) = self.0.upgrade() {
            let mut run = run.borrow_mut();
            let log = run.log.clone();
            if let Some(load) = run.current.as_mut() {
                work(load, &log);
            }
        }
    }
}

impl delegate::Host for CheckHost {
    fn own_host(&self) -> String {
        HOST.to_string()
    }

    fn last_reader_event(&self) -> Option<f64> {
        None
    }

    fn finished(&self, navigation: Id) {
        self.with_load(|load, log| {
            if load.navigation == navigation && load.finished_at.is_none() {
                load.finished_at = Some(Instant::now());
                load.outcome.finished = true;
                log.touch();
            }
        });
    }

    fn failed(&self, navigation: Id, message: String) {
        self.with_load(|load, _| {
            if load.navigation == navigation && !load.reading {
                load.outcome.navigation_error = Some(message);
                load.ended = true;
            }
        });
    }

    fn terminated(&self) {
        self.with_load(|load, _| {
            load.outcome.crashed = true;
            load.ended = true;
        });
    }

    fn cancelled(&self, url: &str) {
        self.with_load(|load, log| {
            load.block(url.to_string(), "navigation".to_string());
            log.touch();
        });
    }

    fn open_externally(&self, url: String) {
        // Nobody clicks in a check: a link that would leave is a navigation blocked.
        self.cancelled(&url);
    }

    fn alert(&self, _message: String) {}

    fn frame_response(&self, url: &str, media_type: &str, main_frame: bool) {
        if main_frame {
            return;
        }
        self.with_load(|load, _| {
            load.frames.push((url.to_string(), media_type.to_string()));
        });
    }
}

/// What a probe's message says, written into the load.
fn take_message(run: &Weak<RefCell<CheckRun>>, body: &str, frame_url: &str) {
    let Ok(message) = serde_json::from_str::<serde_json::Value>(body) else {
        return;
    };
    let Some(run) = run.upgrade() else {
        return;
    };
    let mut run = run.borrow_mut();
    let log = run.log.clone();
    let Some(load) = run.current.as_mut() else {
        return;
    };
    let text = |key: &str| {
        message
            .get(key)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    match message.get("kind").and_then(|kind| kind.as_str()) {
        Some("ready") => {
            load.ready.insert(without_fragment(frame_url));
        }
        Some("blocked") => {
            load.block(text("url"), text("directive"));
            log.touch();
        }
        Some("error") if load.outcome.script_errors.len() < SCRIPT_ERRORS => {
            let source = text("source");
            let line = message.get("line").and_then(|v| v.as_u64()).unwrap_or(0);
            let place = match (source.is_empty(), line) {
                (true, _) => String::new(),
                (false, 0) => format!(" ({})", display(&source)),
                (false, line) => format!(" ({}:{line})", display(&source)),
            };
            load.outcome
                .script_errors
                .push(format!("{}{place}", text("message")));
        }
        _ => {}
    }
}

/// Checks a page: loads its entry and every other HTML file with the network blocked,
/// and reports what each load asked for, what was blocked and missing, and its text.
///
/// On the main thread of a process whose run loop runs (the app). `done` is called
/// from the run loop, outside any caller's borrow (the app's `done` only posts to a
/// channel). `CheckOutcome::failure` is set only when the check itself could not run
/// (no WebKit for pages here, no block list); a load that did not finish, failed or
/// crashed is in its `LoadOutcome`, and when the limit runs out the loads not done
/// are missing from `loads` (an entry still loading is there, not finished).
pub fn check(input: CheckInput, done: impl FnOnce(CheckOutcome) + 'static) {
    let started = Instant::now();
    let fail = move |why: String, done: Box<dyn FnOnce(CheckOutcome)>| {
        let outcome = CheckOutcome {
            engine: String::new(),
            elapsed: started.elapsed(),
            loads: Vec::new(),
            failure: Some(why),
        };
        ns::after(Duration::ZERO, move || done(outcome));
    };
    let done: Box<dyn FnOnce(CheckOutcome)> = Box::new(done);
    if !ns::on_main_thread() {
        done(CheckOutcome {
            failure: Some("the check was asked for off the main thread".to_string()),
            ..CheckOutcome::default()
        });
        return;
    }
    if let Err(why) = crate::available() {
        fail(why, done);
        return;
    }
    let CheckInput {
        source,
        entry,
        also,
        settle_min,
        settle_max,
        limit,
        rules_dir,
    } = input;
    let log = Rc::new(RequestLog::default());
    let run: Run = Rc::new(RefCell::new(CheckRun {
        started,
        limit,
        settle: (settle_min, settle_max.max(settle_min)),
        also_settle: (
            settle_min.min(Duration::from_millis(300)),
            settle_max.max(settle_min).min(Duration::from_secs(1)),
        ),
        queue: std::iter::once(entry.clone()).chain(also).collect(),
        loads: Vec::new(),
        current: None,
        window: NIL,
        web_view: NIL,
        delegate: NIL,
        world: NIL,
        log: log.clone(),
        timer: None,
        done: Some(done),
        engine: engine(),
    }));
    rules::when_ready(&rules_dir, move |result| {
        let list = match result {
            Ok(list) => list,
            Err(why) => {
                let done = run.borrow_mut().done.take();
                if let Some(done) = done {
                    fail(format!("the page could not be checked: {why}"), done);
                }
                return;
            }
        };
        // SAFETY: on the main thread; WebKit's and AppKit's own selectors with the
        // types their headers declare; what is made here is held by the run and
        // released by `finish`.
        unsafe {
            let handler = scheme::new_handler(SchemeState::new(source, HOST, &entry, log));
            let store = config::fresh_data_store();
            let configuration = config::configuration(handler, store);
            release(store);
            release(handler);
            let controller: Id = msg_send![configuration, userContentController];
            let _: () = msg_send![controller, addContentRuleList: list];
            let world: Id = msg_send![class!(WKContentWorld), worldWithName: nsstring(WORLD)];
            let script: Id = msg_send![class!(WKUserScript), alloc];
            let script: Id = msg_send![script,
                initWithSource: nsstring(PROBE)
                injectionTime: 0isize
                forMainFrameOnly: NO
                inContentWorld: world];
            let _: () = msg_send![controller, addUserScript: script];
            release(script);
            let weak = Rc::downgrade(&run);
            let messages = new_messages(Box::new(move |body, url| take_message(&weak, body, url)));
            let _: () = msg_send![controller,
                addScriptMessageHandler: messages
                contentWorld: world
                name: nsstring(HANDLER)];
            release(messages);

            let window = offscreen_window();
            let web_view = checked_web_view(window, configuration);
            release(configuration);
            let delegate_object = delegate::new_delegate(Rc::new(CheckHost(Rc::downgrade(&run))));
            let _: () = msg_send![web_view, setNavigationDelegate: delegate_object];
            let _: () = msg_send![web_view, setUIDelegate: delegate_object];
            {
                let mut state = run.borrow_mut();
                state.window = window;
                state.web_view = web_view;
                state.delegate = delegate_object;
                state.world = retain(world);
            }
            let ticking = run.clone();
            let timer = ns::every(POLL, move || tick(&ticking));
            run.borrow_mut().timer = Some(timer);
        }
    });
}

/// What the check does next.
enum Next {
    Wait,
    Load(Id, String),
    Read(Id, Id),
    Finish,
}

fn tick(run: &Run) {
    let next = {
        let mut state = run.borrow_mut();
        let state = &mut *state;
        if state.done.is_none() {
            Next::Wait
        } else if state.started.elapsed() >= state.limit {
            // Out of time: an entry still loading is reported as not finished; a later
            // file not done is missing from the loads.
            if let Some(mut load) = state.current.take() {
                if state.loads.is_empty() {
                    load.outcome.finished = false;
                    load.close(&state.log);
                    state.loads.push(load.outcome);
                }
                // SAFETY: the reference the load took.
                unsafe { release(load.navigation) };
            }
            Next::Finish
        } else {
            match state.current.as_mut() {
                None => match state.queue.pop_front() {
                    Some(path) => {
                        let (min, max) = if state.loads.is_empty() {
                            state.settle
                        } else {
                            state.also_settle
                        };
                        state.current = Some(Loading::new(&path, min, max));
                        state.log.reset();
                        Next::Load(state.web_view, path)
                    }
                    None => Next::Finish,
                },
                Some(load) if load.ended => {
                    let mut load = state.current.take().expect("a load");
                    load.close(&state.log);
                    // SAFETY: the reference the load took.
                    unsafe { release(load.navigation) };
                    state.loads.push(load.outcome);
                    Next::Wait
                }
                Some(load) => match load.finished_at {
                    Some(at) if !load.reading => {
                        let since = at.elapsed();
                        let quiet = state
                            .log
                            .last_activity
                            .get()
                            .map_or(since, |last| last.elapsed().min(since));
                        if since >= load.max || (since >= load.min && quiet >= QUIET) {
                            load.reading = true;
                            Next::Read(state.web_view, state.world)
                        } else {
                            Next::Wait
                        }
                    }
                    _ => Next::Wait,
                },
            }
        }
    };
    match next {
        Next::Wait => {}
        Next::Load(web_view, path) => {
            // SAFETY: the run's web view; WebKit's own.
            let navigation = unsafe {
                let request = delegate::request_for(&http::url_of(HOST, &path));
                if request.is_null() {
                    NIL
                } else {
                    let navigation: Id = msg_send![web_view, loadRequest: request];
                    retain(navigation)
                }
            };
            let mut state = run.borrow_mut();
            if let Some(load) = state.current.as_mut() {
                load.navigation = navigation;
                if navigation.is_null() {
                    load.outcome.navigation_error =
                        Some(format!("{path} cannot be named as an address"));
                    load.ended = true;
                }
            } else {
                // SAFETY: the reference taken above.
                unsafe { release(navigation) };
            }
        }
        Next::Read(web_view, world) => read_text(run, web_view, world),
        Next::Finish => finish(run),
    }
}

fn read_text(run: &Run, web_view: Id, world: Id) {
    let weak = Rc::downgrade(run);
    let block = ConcreteBlock::new(move |result: Id, error: Id| {
        let Some(run) = weak.upgrade() else {
            return;
        };
        // SAFETY: a string (what the script returns) or nil, and an NSError or nil.
        let (text, error) = unsafe {
            (
                string_of(result),
                (!error.is_null()).then(|| error_text(error)),
            )
        };
        let mut state = run.borrow_mut();
        let Some(load) = state.current.as_mut() else {
            return;
        };
        match (error, serde_json::from_str::<serde_json::Value>(&text)) {
            (None, Ok(value)) => {
                let field = |key: &str| {
                    value
                        .get(key)
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string()
                };
                load.outcome.title = field("title");
                load.outcome.first_heading = field("heading");
                load.outcome.text = field("text");
            }
            (error, _) => {
                if load.outcome.script_errors.len() < SCRIPT_ERRORS {
                    load.outcome.script_errors.push(format!(
                        "the text of the page could not be read: {}",
                        error.unwrap_or_else(|| "no answer".to_string())
                    ));
                }
            }
        }
        load.ended = true;
    })
    .copy();
    // SAFETY: WebKit's own (macOS 11); nil is the main frame.
    unsafe {
        let _: () = msg_send![web_view,
            evaluateJavaScript: nsstring(TEXT)
            inFrame: NIL
            inContentWorld: world
            completionHandler: &*block];
    }
}

fn finish(run: &Run) {
    let (done, outcome, window, web_view, delegate_object, world, timer) = {
        let mut state = run.borrow_mut();
        let Some(done) = state.done.take() else {
            return;
        };
        let outcome = CheckOutcome {
            engine: state.engine.clone(),
            elapsed: state.started.elapsed(),
            loads: std::mem::take(&mut state.loads),
            failure: None,
        };
        (
            done,
            outcome,
            std::mem::replace(&mut state.window, NIL),
            std::mem::replace(&mut state.web_view, NIL),
            std::mem::replace(&mut state.delegate, NIL),
            std::mem::replace(&mut state.world, NIL),
            state.timer.take(),
        )
    };
    if let Some(timer) = &timer {
        timer.stop();
    }
    // SAFETY: the references `check` took, given back once.
    unsafe {
        tear_down(window, web_view, delegate_object);
        release(world);
    }
    // The timer is let go of after its own call has returned.
    ns::after(Duration::ZERO, move || {
        drop(timer);
        done(outcome);
    });
}

/// For a program (the tool's child `writer __page-check`): an `NSApplication` with the
/// activation policy "prohibited" (no Dock icon, no menu), the check run on this
/// thread's run loop, returned when it is done. Call it on the main thread.
pub fn check_blocking(input: CheckInput) -> CheckOutcome {
    ns::program_application();
    let limit = input.limit;
    let slot: Rc<RefCell<Option<CheckOutcome>>> = Rc::new(RefCell::new(None));
    let answer = slot.clone();
    check(input, move |outcome| *answer.borrow_mut() = Some(outcome));
    ns::run_until(limit + Duration::from_secs(10), || slot.borrow().is_some());
    let outcome = slot.borrow_mut().take();
    outcome.unwrap_or_else(|| CheckOutcome {
        failure: Some("the check did not come to an end".to_string()),
        ..CheckOutcome::default()
    })
}

// ---------------------------------------------------------------------------
// A page printed offscreen.

/// What a print hands its PDF (or why there is none) to.
type PrintDone = Box<dyn FnOnce(Result<PrintedPdf, String>)>;

/// Loads the page as `check` loads it (the same configuration, muted, a data store of
/// its own, an offscreen window), waits until it has settled (at most `settle_max`
/// after `load`), then prints it with WebKit's print operation as a save job on the
/// user's default paper. Main thread; `done` as for `check`.
pub fn print_offscreen(input: PdfInput, done: impl FnOnce(Result<PrintedPdf, String>) + 'static) {
    let done: PrintDone = Box::new(done);
    if !ns::on_main_thread() {
        done(Err(
            "the page was asked to print off the main thread".to_string()
        ));
        return;
    }
    if let Err(why) = crate::available() {
        ns::after(Duration::ZERO, move || done(Err(why)));
        return;
    }
    let PdfInput {
        source,
        entry,
        settle_max,
        limit,
        rules_dir,
    } = input;
    let started = Instant::now();
    rules::when_ready(&rules_dir, move |result| {
        let list = match result {
            Ok(list) => list,
            Err(why) => {
                ns::after(Duration::ZERO, move || {
                    done(Err(format!("the page could not be printed: {why}")))
                });
                return;
            }
        };
        // SAFETY: on the main thread; WebKit's and AppKit's own selectors; what is made
        // here is held by `printing` and released when the print has run.
        unsafe {
            let log = Rc::new(RequestLog::default());
            let handler = scheme::new_handler(SchemeState::new(source, HOST, &entry, log.clone()));
            let store = config::fresh_data_store();
            let configuration = config::configuration(handler, store);
            release(store);
            release(handler);
            let controller: Id = msg_send![configuration, userContentController];
            let _: () = msg_send![controller, addContentRuleList: list];
            let window = offscreen_window();
            let web_view = checked_web_view(window, configuration);
            release(configuration);
            let run: Run = Rc::new(RefCell::new(CheckRun {
                started,
                limit,
                settle: (settle_max.min(QUIET), settle_max),
                also_settle: (Duration::ZERO, Duration::ZERO),
                queue: VecDeque::new(),
                loads: Vec::new(),
                current: Some(Loading::new(&entry, settle_max.min(QUIET), settle_max)),
                window,
                web_view,
                delegate: NIL,
                world: NIL,
                log: log.clone(),
                timer: None,
                done: None,
                engine: String::new(),
            }));
            let delegate_object = delegate::new_delegate(Rc::new(CheckHost(Rc::downgrade(&run))));
            let _: () = msg_send![web_view, setNavigationDelegate: delegate_object];
            let _: () = msg_send![web_view, setUIDelegate: delegate_object];
            run.borrow_mut().delegate = delegate_object;
            log.reset();
            let request = delegate::request_for(&http::url_of(HOST, &entry));
            let navigation: Id = msg_send![web_view, loadRequest: request];
            if let Some(load) = run.borrow_mut().current.as_mut() {
                load.navigation = retain(navigation);
            }
            let slot: Rc<RefCell<Option<PrintDone>>> = Rc::new(RefCell::new(Some(done)));
            let ticking = run.clone();
            let timer = ns::every(POLL, move || print_tick(&ticking, &slot));
            run.borrow_mut().timer = Some(timer);
        }
    });
}

fn print_tick(run: &Run, slot: &Rc<RefCell<Option<PrintDone>>>) {
    enum Step {
        Wait,
        Print,
        Fail(String),
    }
    let step = {
        let mut state = run.borrow_mut();
        let state = &mut *state;
        let elapsed = state.started.elapsed();
        match state.current.as_mut() {
            None => Step::Wait,
            Some(load) if load.reading => Step::Wait,
            Some(load) if load.outcome.crashed => {
                Step::Fail("the page stopped working while it loaded".to_string())
            }
            Some(load) if load.outcome.navigation_error.is_some() => Step::Fail(format!(
                "the page could not be loaded: {}",
                load.outcome.navigation_error.clone().unwrap_or_default()
            )),
            Some(_) if elapsed >= state.limit => Step::Fail(format!(
                "the page did not finish loading in {} s",
                state.limit.as_secs()
            )),
            Some(load)
                if load.finished_at.is_some()
                    && state.log.missing.borrow().contains(&load.outcome.path) =>
            {
                Step::Fail(format!(
                    "the page's entry {} is not among its files",
                    load.outcome.path
                ))
            }
            Some(load) => match load.finished_at {
                Some(at) => {
                    let since = at.elapsed();
                    let quiet = state
                        .log
                        .last_activity
                        .get()
                        .map_or(since, |last| last.elapsed().min(since));
                    if since >= load.max || (since >= load.min && quiet >= QUIET) {
                        load.reading = true;
                        Step::Print
                    } else {
                        Step::Wait
                    }
                }
                None => Step::Wait,
            },
        }
    };
    let end = |run: &Run| {
        let (window, web_view, delegate_object, navigation, timer) = {
            let mut state = run.borrow_mut();
            (
                std::mem::replace(&mut state.window, NIL),
                std::mem::replace(&mut state.web_view, NIL),
                std::mem::replace(&mut state.delegate, NIL),
                state.current.take().map_or(NIL, |load| load.navigation),
                state.timer.take(),
            )
        };
        if let Some(timer) = &timer {
            timer.stop();
        }
        // SAFETY: the references `print_offscreen` took, given back once.
        unsafe {
            release(navigation);
            tear_down(window, web_view, delegate_object);
        }
        ns::after(Duration::ZERO, move || drop(timer));
    };
    match step {
        Step::Wait => {}
        Step::Fail(why) => {
            end(run);
            let done = slot.borrow_mut().take();
            if let Some(done) = done {
                ns::after(Duration::ZERO, move || done(Err(why)));
            }
        }
        Step::Print => {
            let (web_view, window) = {
                let state = run.borrow();
                (state.web_view, state.window)
            };
            let run = run.clone();
            let slot = slot.clone();
            // SAFETY: the run's web view in its window, both held until `end`.
            unsafe {
                print::print_web_view(
                    web_view,
                    window,
                    Box::new(move |result| {
                        end(&run);
                        let done = slot.borrow_mut().take();
                        if let Some(done) = done {
                            done(result);
                        }
                    }),
                );
            }
        }
    }
}

/// For a program (the child `writer __page-pdf`): as [`check_blocking`], for
/// [`print_offscreen`].
pub fn print_blocking(input: PdfInput) -> Result<PrintedPdf, String> {
    ns::program_application();
    let limit = input.limit;
    let slot: Rc<RefCell<Option<Result<PrintedPdf, String>>>> = Rc::new(RefCell::new(None));
    let answer = slot.clone();
    print_offscreen(input, move |result| *answer.borrow_mut() = Some(result));
    ns::run_until(limit + Duration::from_secs(30), || slot.borrow().is_some());
    let result = slot.borrow_mut().take();
    result.unwrap_or_else(|| Err("the print did not come to an end".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_of_xml_types_are_watched() {
        assert!(watched_type("image/svg+xml"));
        assert!(watched_type("application/xhtml+xml; charset=utf-8"));
        assert!(watched_type("text/xml"));
        assert!(watched_type("application/xml"));
        assert!(watched_type("application/atom+xml"));
        assert!(!watched_type("text/html"));
        assert!(!watched_type("image/png"));
    }

    #[test]
    fn a_frame_is_named_by_its_path() {
        assert_eq!(display("writer-page://check/art/a%20b.svg"), "art/a b.svg");
        assert_eq!(display("https://example.com/x"), "https://example.com/x");
        assert_eq!(
            without_fragment("writer-page://check/a.svg#x"),
            "writer-page://check/a.svg"
        );
    }
}
