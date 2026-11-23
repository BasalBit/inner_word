//! WebKit, run for real: the check, the view in an offscreen window, the print.
//!
//! This is a program of its own and not a set of `#[test]`s: WebKit wants the main
//! thread of the process, and the default test harness runs every test on another one.
//! `cargo test -p innerword-web` runs it with the unit tests; it exits with a failure when
//! anything below does not hold.
//!
//! Nothing here reaches the internet: every address a page names is a server of this
//! program on 127.0.0.1, which counts what reaches it (it must be nothing). The
//! application object has the activation policy "prohibited" (no Dock icon, never
//! active); its windows are borderless, far off screen and never key, and the events a
//! case needs are handed to its window directly (`-[NSWindow sendEvent:]`), never
//! posted to the system. Media are silence.

#![allow(unexpected_cfgs)]

use std::{
    cell::{Cell, RefCell},
    ffi::c_void,
    io::{Read, Write},
    net::{TcpListener, UdpSocket},
    path::{Path, PathBuf},
    process::ExitCode,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use innerword_web::{
    Appearance, CheckInput, CheckOutcome, EditCommand, KeyPress, MemorySource, MenuEntry,
    PageEvents, PageSource, PdfInput, Rect, ServedFile, ViewConfig, ViewExtras, WebPage, available,
    check_blocking, check_selectors, menu, print_offscreen, testing,
};
use objc::{
    Encode, Encoding, class,
    declare::ClassDecl,
    msg_send,
    runtime::{BOOL, NO, Object, Sel, YES},
    sel, sel_impl,
};

type Id = *mut Object;
const NIL: Id = std::ptr::null_mut();

#[path = "webkit/board.rs"]
mod board;
#[path = "webkit/bundle.rs"]
mod bundle;
#[path = "webkit/crosscheck.rs"]
mod crosscheck;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct NSPoint {
    x: f64,
    y: f64,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct NSSize {
    width: f64,
    height: f64,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct NSRect {
    origin: NSPoint,
    size: NSSize,
}
unsafe impl Encode for NSPoint {
    fn encode() -> Encoding {
        unsafe { Encoding::from_str("{CGPoint=dd}") }
    }
}
unsafe impl Encode for NSRect {
    fn encode() -> Encoding {
        unsafe { Encoding::from_str("{CGRect={CGPoint=dd}{CGSize=dd}}") }
    }
}

#[link(name = "PDFKit", kind = "framework")]
unsafe extern "C" {}

/// The text of a PDF, as PDFKit reads it.
fn pdf_text(bytes: &[u8]) -> String {
    unsafe {
        let data: Id = msg_send![class!(NSData),
            dataWithBytes: bytes.as_ptr() as *const c_void
            length: bytes.len()];
        let document: Id = msg_send![class!(PDFDocument), alloc];
        let document: Id = msg_send![document, initWithData: data];
        if document.is_null() {
            return String::new();
        }
        let text: Id = msg_send![document, string];
        let out = if text.is_null() {
            String::new()
        } else {
            let utf8: *const std::os::raw::c_char = msg_send![text, UTF8String];
            std::ffi::CStr::from_ptr(utf8)
                .to_string_lossy()
                .into_owned()
        };
        let _: () = msg_send![document, release];
        out
    }
}

// ---------------------------------------------------------------------------
// What every case uses.

struct Report {
    failures: Vec<String>,
}

impl Report {
    fn check(&mut self, holds: bool, what: &str) {
        if holds {
            println!("webkit: ok: {what}");
        } else {
            println!("webkit: FAILED: {what}");
            self.failures.push(what.to_string());
        }
    }

    fn note(&self, what: &str) {
        println!("webkit: note: {what}");
    }
}

/// A server on 127.0.0.1 that answers every request with a small body and counts
/// what reached it.
struct Server {
    port: u16,
    hits: Arc<Mutex<Vec<String>>>,
}

impl Server {
    fn start() -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a port on 127.0.0.1");
        let port = listener.local_addr().expect("its address").port();
        let hits = Arc::new(Mutex::new(Vec::new()));
        let seen = hits.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let seen = seen.clone();
                std::thread::spawn(move || {
                    let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
                    let mut buffer = [0u8; 4096];
                    let read = stream.read(&mut buffer).unwrap_or(0);
                    let text = String::from_utf8_lossy(&buffer[..read]).to_string();
                    let line = text.lines().next().unwrap_or("(a connection)").to_string();
                    seen.lock().unwrap().push(line);
                    let _ = stream.write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                    );
                });
            }
        });
        Server { port, hits }
    }

    fn hits(&self) -> Vec<String> {
        self.hits.lock().unwrap().clone()
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}/{path}", self.port)
    }
}

/// A UDP socket on 127.0.0.1 that counts the packets it gets.
struct Udp {
    port: u16,
    packets: Arc<AtomicUsize>,
}

impl Udp {
    fn start() -> Udp {
        let socket = UdpSocket::bind("127.0.0.1:0").expect("a UDP port on 127.0.0.1");
        let port = socket.local_addr().expect("its address").port();
        let packets = Arc::new(AtomicUsize::new(0));
        let count = packets.clone();
        std::thread::spawn(move || {
            let mut buffer = [0u8; 2048];
            while socket.recv_from(&mut buffer).is_ok() {
                count.fetch_add(1, Ordering::SeqCst);
            }
        });
        Udp { port, packets }
    }
}

fn file(media_type: &str, bytes: &[u8]) -> ServedFile {
    ServedFile {
        media_type: media_type.to_string(),
        bytes: Arc::from(bytes),
    }
}

fn html(text: &str) -> ServedFile {
    file("text/html", text.as_bytes())
}

fn source(files: Vec<(&str, ServedFile)>) -> Arc<dyn PageSource> {
    Arc::new(MemorySource::new(
        files
            .into_iter()
            .map(|(path, file)| (path.to_string(), file)),
    ))
}

/// A WAV file of silence (a second at 8 kHz, 8 bits).
fn silence() -> Vec<u8> {
    let samples = 8000u32;
    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + samples).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&8000u32.to_le_bytes());
    wav.extend_from_slice(&8000u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&8u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&samples.to_le_bytes());
    wav.extend(std::iter::repeat_n(128u8, samples as usize));
    wav
}

/// A folder of this run's own under the system's temporary directory.
fn scratch() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "inner-word-webkit-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos())
    ));
    std::fs::create_dir_all(&dir).expect("a folder for the test");
    dir
}

fn input(source: Arc<dyn PageSource>, entry: &str, also: &[&str], rules: &Path) -> CheckInput {
    CheckInput {
        source,
        entry: entry.to_string(),
        also: also.iter().map(|path| path.to_string()).collect(),
        settle_min: Duration::from_millis(1000),
        settle_max: Duration::from_millis(3000),
        limit: Duration::from_secs(60),
        rules_dir: rules.to_path_buf(),
    }
}

fn blocked_paths(outcome: &CheckOutcome) -> Vec<(String, String)> {
    outcome
        .loads
        .iter()
        .flat_map(|load| load.blocked.iter().cloned())
        .collect()
}

fn ns_string(text: &str) -> Id {
    unsafe {
        let object: Id = msg_send![class!(NSString), alloc];
        let object: Id = msg_send![object,
            initWithBytes: text.as_ptr() as *const c_void
            length: text.len()
            encoding: 4usize];
        msg_send![object, autorelease]
    }
}

fn wait(duration: Duration) {
    testing::run_until(duration, || false);
}

// ---------------------------------------------------------------------------
// A stand-in for GPUI's view: takes the keys handed to it, beeps for nothing.

extern "C" fn gpui_key_down(this: &Object, _: Sel, event: Id) {
    unsafe {
        let code: u16 = msg_send![event, keyCode];
        let count = *this.get_ivar::<usize>("keysDown");
        (*(this as *const Object as *mut Object)).set_ivar::<usize>("keysDown", count + 1);
        let _ = code;
    }
}

extern "C" fn gpui_key_up(_this: &Object, _: Sel, _event: Id) {}

extern "C" fn gpui_accepts(_this: &Object, _: Sel) -> BOOL {
    YES
}

fn gpui_view_class() -> &'static objc::runtime::Class {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let mut decl = ClassDecl::new("InnerWordWebTestGpuiView", class!(NSView)).unwrap();
        decl.add_ivar::<usize>("keysDown");
        unsafe {
            decl.add_method(
                sel!(keyDown:),
                gpui_key_down as extern "C" fn(&Object, Sel, Id),
            );
            decl.add_method(sel!(keyUp:), gpui_key_up as extern "C" fn(&Object, Sel, Id));
            decl.add_method(
                sel!(acceptsFirstResponder),
                gpui_accepts as extern "C" fn(&Object, Sel) -> BOOL,
            );
        }
        decl.register();
    });
    objc::runtime::Class::get("InnerWordWebTestGpuiView").unwrap()
}

fn gpui_keys(view: Id) -> usize {
    unsafe { *(*view).get_ivar::<usize>("keysDown") }
}

/// What a view told its owner.
#[derive(Default)]
struct Events {
    loaded: Cell<usize>,
    failed: RefCell<Vec<String>>,
    crashed: Cell<usize>,
    wants_focus: Cell<usize>,
    keys_asked: RefCell<Vec<KeyPress>>,
    opened: RefCell<Vec<String>>,
    alerts: RefCell<Vec<String>>,
    menu_asked: RefCell<Vec<Option<String>>>,
    chosen: RefCell<Vec<u32>>,
    messages: RefCell<Vec<String>>,
}

impl PageEvents for Events {
    fn loaded(&self) {
        self.loaded.set(self.loaded.get() + 1);
    }
    fn failed(&self, message: String) {
        self.failed.borrow_mut().push(message);
    }
    fn crashed(&self) {
        self.crashed.set(self.crashed.get() + 1);
    }
    fn wants_focus(&self) {
        self.wants_focus.set(self.wants_focus.get() + 1);
    }
    fn takes_key(&self, key: &KeyPress) -> bool {
        self.keys_asked.borrow_mut().push(key.clone());
        // ctrl-return: the page's context menu, the owner's key.
        key.ctrl && key.key_code == 36
    }
    fn open_externally(&self, url: String) {
        self.opened.borrow_mut().push(url);
    }
    fn alert(&self, message: String) {
        self.alerts.borrow_mut().push(message);
    }
    fn menu_items(&self, link: Option<&str>) -> Vec<MenuEntry> {
        self.menu_asked.borrow_mut().push(link.map(str::to_string));
        vec![
            MenuEntry {
                id: 1,
                title: "Find in Page…\tcmd-f".to_string(),
                enabled: true,
                separator_before: false,
            },
            MenuEntry {
                id: 2,
                title: "Reload Page".to_string(),
                enabled: true,
                separator_before: false,
            },
        ]
    }
    fn menu_chosen(&self, id: u32) {
        self.chosen.borrow_mut().push(id);
    }
    fn message(&self, body: String) {
        self.messages.borrow_mut().push(body);
    }
}

/// A window far off screen with a stand-in for GPUI's view filling its content view.
struct Host {
    window: Id,
    gpui: Id,
}

impl Host {
    fn new() -> Host {
        let window = testing::offscreen_window() as Id;
        unsafe {
            // As GPUI's windows do.
            let _: () = msg_send![window, setAcceptsMouseMovedEvents: YES];
            let content: Id = msg_send![window, contentView];
            let bounds: NSRect = msg_send![content, bounds];
            let gpui: Id = msg_send![gpui_view_class(), alloc];
            let gpui: Id = msg_send![gpui, initWithFrame: bounds];
            let _: () = msg_send![content, addSubview: gpui];
            Host { window, gpui }
        }
    }

    fn content_height(&self) -> f64 {
        unsafe {
            let content: Id = msg_send![self.window, contentView];
            let bounds: NSRect = msg_send![content, bounds];
            bounds.size.height
        }
    }

    fn first_responder(&self) -> Id {
        unsafe { msg_send![self.window, firstResponder] }
    }

    /// A page in this window, on the whole content view, loaded.
    fn page(
        &self,
        files: Vec<(&str, ServedFile)>,
        entry: &str,
        rules: &Path,
    ) -> (WebPage, Rc<Events>) {
        self.page_at("page-12", files, entry, rules)
    }

    /// A view with what only a whiteboard's view has, at `view_host`, on the whole content
    /// view, loaded.
    fn board(
        &self,
        view_host: &str,
        files: Vec<(&str, ServedFile)>,
        extras: ViewExtras,
        rules: &Path,
    ) -> (WebPage, Rc<Events>) {
        self.board_from(view_host, source(files), extras, rules)
    }

    /// The same over a source of the caller's.
    fn board_from(
        &self,
        view_host: &str,
        files: Arc<dyn PageSource>,
        extras: ViewExtras,
        rules: &Path,
    ) -> (WebPage, Rc<Events>) {
        let events = Rc::new(Events::default());
        let page = unsafe {
            WebPage::new_with(
                self.gpui.cast(),
                ViewConfig {
                    host: view_host.to_string(),
                    entry: "index.html".to_string(),
                    source: files,
                    rules_dir: rules.to_path_buf(),
                    appearance: Appearance::Light,
                    background: (1., 1., 1.),
                },
                extras,
                events.clone(),
            )
        };
        page.set_frame(Rect {
            x: 0.,
            y: 0.,
            width: 1280.,
            height: self.content_height(),
        });
        let loaded = testing::run_until(Duration::from_secs(15), || events.loaded.get() > 0);
        if !loaded {
            println!(
                "webkit: note: the board's view did not report loaded ({:?})",
                events.failed.borrow()
            );
        }
        (page, events)
    }

    /// The same at a host of its own (`page-<id>`: the origin of the page).
    fn page_at(
        &self,
        page_host: &str,
        files: Vec<(&str, ServedFile)>,
        entry: &str,
        rules: &Path,
    ) -> (WebPage, Rc<Events>) {
        let events = Rc::new(Events::default());
        let page = unsafe {
            WebPage::new(
                self.gpui.cast(),
                ViewConfig {
                    host: page_host.to_string(),
                    entry: entry.to_string(),
                    source: source(files),
                    rules_dir: rules.to_path_buf(),
                    appearance: Appearance::Light,
                    background: (0.98, 0.97, 0.95),
                },
                events.clone(),
            )
        };
        page.set_frame(Rect {
            x: 0.,
            y: 0.,
            width: 1280.,
            height: self.content_height(),
        });
        let loaded = testing::run_until(Duration::from_secs(15), || events.loaded.get() > 0);
        if !loaded {
            println!(
                "webkit: note: the page did not report loaded ({:?})",
                events.failed.borrow()
            );
        }
        (page, events)
    }

    /// Hands a mouse event to the window at a point of the page (CSS pixels from the
    /// top left of the view, which fills the content view).
    fn mouse(&self, kind: usize, x: f64, y: f64) {
        unsafe {
            let number: isize = msg_send![self.window, windowNumber];
            let location = NSPoint {
                x,
                y: self.content_height() - y,
            };
            let event: Id = msg_send![class!(NSEvent),
                mouseEventWithType: kind
                location: location
                modifierFlags: 0usize
                timestamp: testing::uptime()
                windowNumber: number
                context: NIL
                eventNumber: 0isize
                clickCount: 1isize
                pressure: 1.0f32];
            let _: () = msg_send![self.window, sendEvent: event];
        }
    }

    /// Hands a key event to the window.
    fn key(&self, down: bool, code: u16, characters: &str, flags: usize) {
        unsafe {
            let number: isize = msg_send![self.window, windowNumber];
            let event: Id = msg_send![class!(NSEvent),
                keyEventWithType: if down { 10usize } else { 11usize }
                location: NSPoint::default()
                modifierFlags: flags
                timestamp: testing::uptime()
                windowNumber: number
                context: NIL
                characters: ns_string(characters)
                charactersIgnoringModifiers: ns_string(characters)
                isARepeat: NO
                keyCode: code];
            let _: () = msg_send![self.window, sendEvent: event];
        }
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        unsafe {
            let _: () = msg_send![self.gpui, release];
            let _: () = msg_send![self.window, orderOut: NIL];
            let _: () = msg_send![self.window, close];
            let _: () = msg_send![self.window, release];
        }
    }
}

/// Evaluates a script in the page and waits for its answer.
fn eval(page: &WebPage, script: &str) -> Result<String, String> {
    let slot: Rc<RefCell<Option<Result<String, String>>>> = Rc::new(RefCell::new(None));
    let answer = slot.clone();
    page.evaluate(script, move |result| *answer.borrow_mut() = Some(result));
    testing::run_until(Duration::from_secs(10), || slot.borrow().is_some());
    let result = slot.borrow_mut().take();
    result.unwrap_or_else(|| Err("no answer in 10 s".to_string()))
}

/// Waits until a script answers something other than "" or "null".
fn eval_until(page: &WebPage, script: &str, limit: Duration) -> String {
    let start = Instant::now();
    loop {
        match eval(page, script) {
            Ok(text) if !text.is_empty() && text != "null" => return text,
            _ => {}
        }
        if start.elapsed() > limit {
            return String::new();
        }
        wait(Duration::from_millis(50));
    }
}

/// The middle of an element of the page, in CSS pixels from the view's top left.
fn middle_of(page: &WebPage, id: &str) -> Option<(f64, f64)> {
    let text = eval(
        page,
        &format!(
            "(() => {{ const r = document.getElementById('{id}').getBoundingClientRect(); \
             return JSON.stringify([r.left + r.width / 2, r.top + r.height / 2]); }})()"
        ),
    )
    .ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    Some((value[0].as_f64()?, value[1].as_f64()?))
}

// ---------------------------------------------------------------------------
// The cases.

fn main() -> ExitCode {
    testing::program_application();
    let mut report = Report {
        failures: Vec::new(),
    };
    let dir = scratch();
    let rules = dir.join("WebKit");
    let server = Server::start();

    // Whether this Mac has what pages need.
    match check_selectors() {
        Ok(()) => report.check(true, "every class and selector the crate sends is there"),
        Err(missing) => report.check(false, &format!("not there: {missing}")),
    }
    match available() {
        Ok(()) => report.check(true, "pages are available on this Mac"),
        Err(why) => {
            report.check(false, &format!("pages are not available: {why}"));
            return finish(report, &dir);
        }
    }

    // `INNERWORD_WEB_ONLY=<words>` runs the cases whose name has them (for a person
    // looking into one); every case runs otherwise.
    let only = std::env::var("INNERWORD_WEB_ONLY").unwrap_or_default();
    let wanted = |name: &str| only.is_empty() || name.contains(only.as_str());
    if wanted("rule_list") {
        the_rule_list_is_compiled_once(&mut report, &dir);
    }
    if wanted("probe") {
        the_probe_blocks_and_names_every_kind(&mut report, &server, &rules);
    }
    if wanted("webrtc") {
        webrtc_is_off(&mut report, &rules);
    }
    if wanted("svg") {
        an_svg_frame_is_never_silent(&mut report, &server, &rules);
    }
    if wanted("text") {
        the_text_is_read_whole(&mut report, &rules);
        the_text_keeps_apart_what_layout_shows_apart(&mut report, &rules);
    }
    if wanted("plays") {
        nothing_plays_in_the_check(&mut report, &rules);
    }
    if wanted("settling") {
        settling_and_the_limit(&mut report, &server, &rules);
    }
    if wanted("view") {
        the_view(&mut report, &server, &rules);
    }
    if wanted("gate") {
        the_gate_opens_only_the_link_the_reader_acted_on(&mut report, &server, &rules);
    }
    if wanted("lazy") {
        lazy_pieces_are_asked_for(&mut report, &server, &rules);
    }
    if wanted("switched") {
        pages_switched_during_loads(&mut report, &rules);
    }
    if wanted("printed") {
        printed_offscreen(&mut report, &rules);
    }
    if wanted("storage") {
        storage_is_the_pages_own_and_not_kept(&mut report, &rules);
    }
    // A whiteboard's view and its export page (`docs/whiteboard-design.md`, 4.2).
    if wanted("bridge") {
        board::the_bridge(&mut report, &rules);
        board::frames_never_reach_the_bridge(&mut report, &rules);
    }
    if wanted("call") {
        board::call_async(&mut report, &rules);
    }
    if wanted("one_document") {
        board::one_document(&mut report, &rules);
    }
    if wanted("board_policy") {
        board::the_board_policy(&mut report, &server, &rules);
    }
    if wanted("park") {
        board::park(&mut report, &rules);
    }
    if wanted("run_script") {
        board::run_script(&mut report, &rules);
    }
    // Excalidraw itself: the bundle of innerword-board-web under the whiteboard's policies.
    if wanted("bundle") {
        bundle::the_bundle(&mut report, &server, &rules);
    }
    // What the glue keeps of the writer's while an outside write is adopted, and its start
    // and shape library at their limits (B3's review fixes).
    if wanted("bundle_glue") {
        bundle::the_glue_keeps_what_the_writer_does(&mut report, &rules);
    }
    // A GPUI overlay over a whiteboard: the page is told nothing and keeps drawing.
    if wanted("board_cover") {
        bundle::a_covered_whiteboard_is_told_nothing(&mut report, &rules);
    }
    // A whiteboard's PDF against what the screen shows.
    if wanted("board_pdf") {
        bundle::the_pdf_is_the_screen(&mut report, &rules);
    }
    // The Edit menu on a whiteboard, with a pasteboard of the test's own.
    if wanted("board_edit") {
        bundle::the_edit_menu_on_a_whiteboard(&mut report, &rules);
    }
    // The converter of agents' skeletons (innerword-board) against Excalidraw's own.
    if wanted("crosscheck") {
        crosscheck::the_converter_against_excalidraw(&mut report, &rules);
    }

    report.check(
        server.hits().is_empty(),
        &format!(
            "nothing reached the server on 127.0.0.1 ({:?})",
            server.hits()
        ),
    );
    finish(report, &dir)
}

fn finish(report: Report, dir: &Path) -> ExitCode {
    std::fs::remove_dir_all(dir).ok();
    if report.failures.is_empty() {
        println!("webkit: every check held");
        ExitCode::SUCCESS
    } else {
        println!("webkit: {} of the checks failed:", report.failures.len());
        for failure in &report.failures {
            println!("webkit:   {failure}");
        }
        ExitCode::FAILURE
    }
}

fn the_rule_list_is_compiled_once(report: &mut Report, dir: &Path) {
    let rules = dir.join("rules-twice");
    for round in 0..2 {
        testing::forget_rule_lists();
        let events = Rc::new(Events::default());
        let host = Host::new();
        let (page, events_seen) =
            host.page(vec![("index.html", html("<p>x</p>"))], "index.html", &rules);
        let _ = events;
        report.check(
            events_seen.loaded.get() > 0,
            &format!(
                "a page loads with the rule list of a fresh store (round {})",
                round + 1
            ),
        );
        drop(page);
    }
    let names: Rc<RefCell<Option<Vec<String>>>> = Rc::new(RefCell::new(None));
    let answer = names.clone();
    testing::rule_lists_in(&rules, move |found| *answer.borrow_mut() = Some(found));
    testing::run_until(Duration::from_secs(10), || names.borrow().is_some());
    let names = names.borrow().clone().unwrap_or_default();
    report.check(
        names == vec![innerword_web::RULE_LIST_ID.to_string()],
        &format!("the rule list compiled twice leaves one list in its store ({names:?})"),
    );
}

fn the_probe_blocks_and_names_every_kind(report: &mut Report, server: &Server, rules: &Path) {
    let page = format!(
        r#"<!doctype html><html><head><title>Probe</title>
<link rel="stylesheet" href="{css}">
<style>
 @font-face {{ font-family: Far; src: url({font}); }}
 .bg {{ background-image: url({bg}); }}
 body {{ font-family: Far, serif; }}
</style></head>
<body>
<h1>The probe</h1>
<div class="bg">background</div>
<img src="{img}">
<audio src="{audio}" autoplay preload="auto"></audio>
<iframe src="{frame}"></iframe>
<iframe src="inner.html"></iframe>
<p id="module">module not loaded</p>
<p id="worker">worker not loaded</p>
<script type="module" src="./mod.js"></script>
<script>
 fetch('{fetch}').catch(() => {{}});
 try {{ new WebSocket('{ws}'); }} catch (e) {{}}
 try {{ new EventSource('{es}'); }} catch (e) {{}}
 try {{ navigator.sendBeacon('{beacon}', 'x'); }} catch (e) {{}}
 try {{ const x = new XMLHttpRequest(); x.open('GET', '{xhr}'); x.send(); }} catch (e) {{}}
 const w = new Worker('w.js');
 w.onmessage = (e) => {{ document.getElementById('worker').textContent = 'worker ' + e.data; }};
</script>
</body></html>"#,
        css = server.url("a.css"),
        font = server.url("f.woff2"),
        bg = server.url("bg.png"),
        img = server.url("b.png"),
        audio = server.url("s.mp3"),
        frame = server.url("c"),
        fetch = server.url("d"),
        ws = server.url("e").replacen("http:", "ws:", 1),
        es = server.url("es"),
        beacon = server.url("beacon"),
        xhr = server.url("xhr"),
    );
    let inner = format!(
        "<!doctype html><p>inner</p><script>fetch('{}').catch(() => {{}});</script>",
        server.url("frominner")
    );
    let worker = format!(
        "fetch('{}').catch(() => {{}}); postMessage('loaded');",
        server.url("fromworker")
    );
    let files = vec![
        ("index.html", html(&page)),
        ("inner.html", html(&inner)),
        (
            "mod.js",
            file(
                "text/javascript",
                b"document.getElementById('module').textContent = 'module loaded';",
            ),
        ),
        ("w.js", file("text/javascript", worker.as_bytes())),
    ];
    let outcome = check_blocking(input(source(files), "index.html", &[], rules));
    report.check(
        outcome.failure.is_none(),
        &format!("the probe's check ran ({:?})", outcome.failure),
    );
    let Some(load) = outcome.loads.first() else {
        report.check(false, "the probe's check loaded its entry");
        return;
    };
    report.check(load.finished, "the probe's page finished loading");
    let blocked = blocked_paths(&outcome);
    for (path, kind) in [
        ("a.css", "a style sheet"),
        ("f.woff2", "a font"),
        ("bg.png", "a CSS background"),
        ("b.png", "a picture"),
        ("s.mp3", "an audio element"),
        ("/c", "a frame of another site"),
        ("/d", "fetch"),
        ("/e", "a WebSocket"),
        ("/es", "an EventSource"),
        ("/beacon", "sendBeacon"),
        ("/xhr", "XMLHttpRequest"),
        ("/frominner", "a fetch from a frame of the page"),
    ] {
        // A frame of another origin is reported by its origin alone.
        let origin = format!("127.0.0.1:{}", server.port);
        let found = blocked.iter().find(|(url, directive)| {
            (url.ends_with(path) && url.contains("127.0.0.1"))
                || (path == "/c" && url.ends_with(&origin) && directive == "frame-src")
        });
        report.check(
            found.is_some_and(|(_, directive)| !directive.is_empty()),
            &format!("{kind} is blocked and named with its directive ({found:?})"),
        );
    }
    report.check(
        load.text.contains("module loaded"),
        "the page's own module loads under the policy",
    );
    report.check(
        load.text.contains("worker loaded"),
        &format!(
            "the page's own worker loads under the policy ({:?})",
            load.text
        ),
    );
    report.check(
        load.missing.is_empty(),
        &format!("nothing is missing ({:?})", load.missing),
    );
    report.check(
        load.requests >= 4,
        &format!("the requests were counted ({})", load.requests),
    );
    report.note(&format!("blocked: {blocked:?}"));
    report.check(
        server.hits().is_empty(),
        &format!("no request reached the server ({:?})", server.hits()),
    );
}

fn webrtc_is_off(report: &mut Report, rules: &Path) {
    let udp = Udp::start();
    let page = format!(
        r#"<!doctype html><title>RTC</title><p id="rtc">rtc not asked</p>
<script>
 document.getElementById('rtc').textContent = 'rtc:' + typeof RTCPeerConnection + ':' + typeof webkitRTCPeerConnection;
 try {{
   const pc = new RTCPeerConnection({{ iceServers: [
     {{ urls: 'stun:127.0.0.1:{port}' }},
     {{ urls: 'turn:127.0.0.1:{port}?transport=udp', username: 'u', credential: 'c' }} ] }});
   pc.createDataChannel('x');
   pc.createOffer().then((offer) => pc.setLocalDescription(offer));
 }} catch (e) {{}}
</script>"#,
        port = udp.port
    );
    let outcome = check_blocking(input(
        source(vec![("index.html", html(&page))]),
        "index.html",
        &[],
        rules,
    ));
    let text = outcome
        .loads
        .first()
        .map(|load| load.text.clone())
        .unwrap_or_default();
    report.check(
        text.contains("rtc:undefined:undefined"),
        &format!("RTCPeerConnection is not defined in the check ({text:?})"),
    );
    let host = Host::new();
    let (view, _) = host.page(vec![("index.html", html(&page))], "index.html", rules);
    let kind = eval(&view, "typeof RTCPeerConnection");
    report.check(
        kind.as_deref() == Ok("undefined"),
        &format!("RTCPeerConnection is not defined in the view ({kind:?})"),
    );
    let media = eval(&view, "typeof navigator.mediaDevices");
    report.note(&format!("navigator.mediaDevices in the view: {media:?}"));
    wait(Duration::from_secs(2));
    drop(view);
    report.check(
        udp.packets.load(Ordering::SeqCst) == 0,
        &format!(
            "no UDP packet reached 127.0.0.1 ({} came)",
            udp.packets.load(Ordering::SeqCst)
        ),
    );
}

fn an_svg_frame_is_never_silent(report: &mut Report, server: &Server, rules: &Path) {
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20"><script>fetch('{}').catch(() => {{}});</script><rect width="10" height="10"/></svg>"#,
        server.url("fromsvg")
    );
    let files = vec![
        (
            "index.html",
            html(
                "<!doctype html><title>SVG</title><iframe src=\"a.svg\"></iframe><img src=\"nothere.png\">",
            ),
        ),
        ("a.svg", file("image/svg+xml", svg.as_bytes())),
    ];
    let outcome = check_blocking(input(source(files), "index.html", &[], rules));
    let Some(load) = outcome.loads.first() else {
        report.check(false, "the SVG page was checked");
        return;
    };
    let reported = load
        .blocked
        .iter()
        .any(|(url, _)| url.ends_with("/fromsvg"));
    let unwatched = load.unwatched_frames.contains(&"a.svg".to_string());
    report.note(&format!(
        "an iframed SVG's fetch: reported as blocked {reported}, frame unwatched {unwatched}"
    ));
    report.check(
        reported || unwatched,
        "an iframed SVG whose script fetches is never silent (blocked, or the frame unwatched)",
    );
    report.check(
        load.missing == vec!["nothere.png".to_string()],
        &format!("a 404 is reported as missing ({:?})", load.missing),
    );
}

fn the_text_is_read_whole(report: &mut Report, rules: &Path) {
    let page = r#"<!doctype html><html><head><title>Before</title></head><body>
<h1>Heading One</h1>
<p>Shown paragraph.</p>
<details><summary>More</summary><p>Inside the details.</p></details>
<div hidden>Hidden chapter text.</div>
<p style="display:none">Not displayed either.</p>
<div style="display:none"><span>Closed 2 days ago</span><span>Closed 7 days ago</span><b>Fak</b><b>turano</b><span>05 / 05</span></div>
<p id="late"></p><p id="raf"></p><p id="forged"></p>
<script>
 document.title = 'Written by script';
 document.getElementById('late').textContent = 'Written at load';
 requestAnimationFrame(() => requestAnimationFrame(() => {
   document.getElementById('raf').textContent = 'Written in a frame';
 }));
 try {
   window.webkit.messageHandlers.writerCheck.postMessage(JSON.stringify(
     { kind: 'blocked', url: 'https://forged.example/x', directive: 'img-src' }));
   document.getElementById('forged').textContent = 'the handler was reached';
 } catch (e) {
   document.getElementById('forged').textContent = 'no handler in the page';
 }
 try { undefinedFunction(); } catch (e) {}
 setTimeout(() => { throw new Error('a late error'); }, 10);
</script></body></html>"#;
    let other = "<!doctype html><title>Second</title><p>The second file.</p>";
    let outcome = check_blocking(input(
        source(vec![
            ("index.html", html(page)),
            ("two/other.html", html(other)),
        ]),
        "index.html",
        &["two/other.html"],
        rules,
    ));
    let Some(load) = outcome.loads.first() else {
        report.check(false, "the text page was checked");
        return;
    };
    report.check(
        load.title == "Written by script",
        &format!(
            "the title is read after a script wrote it ({:?})",
            load.title
        ),
    );
    report.check(
        load.first_heading == "Heading One",
        &format!("the first heading is read ({:?})", load.first_heading),
    );
    let text = &load.text;
    report.note(&format!("the text: {text:?}"));
    for shown in [
        "Shown paragraph.",
        "More",
        "Written at load",
        "Written in a frame",
    ] {
        report.check(text.contains(shown), &format!("the text has {shown:?}"));
    }
    let (shown_part, rest) = text.split_once("\n\n").unwrap_or((text, ""));
    for hidden in [
        "Inside the details.",
        "Hidden chapter text.",
        "Not displayed either.",
        "Closed 2 days ago Closed 7 days ago Fakturano 05 / 05",
    ] {
        report.check(
            rest.contains(hidden) && !shown_part.contains(hidden),
            &format!("{hidden:?} is in the text after what is shown"),
        );
    }
    report.check(
        text.contains("no handler in the page")
            && !load.blocked.iter().any(|(url, _)| url.contains("forged")),
        "a forged postMessage of the page does not reach the check's handler",
    );
    report.note(&format!("script errors: {:?}", load.script_errors));
    report.check(
        outcome.loads.len() == 2,
        &format!("every HTML file is loaded ({})", outcome.loads.len()),
    );
    if let Some(second) = outcome.loads.get(1) {
        report.check(
            second.path == "two/other.html"
                && second.title == "Second"
                && second.text.contains("The second file."),
            &format!(
                "the other HTML file is read ({:?}, {:?})",
                second.title, second.text
            ),
        );
    }
    report.check(
        !outcome.engine.is_empty(),
        &format!("the engine is named ({})", outcome.engine),
    );
}

fn the_text_keeps_apart_what_layout_shows_apart(report: &mut Report, rules: &Path) {
    // No white space in the source between the parts of each line below.
    let page = r##"<!doctype html><html><head><title>Junctions</title><style>
 body { font: 16px sans-serif; margin: 20px; }
 .tag { margin-left: 12px; text-transform: uppercase; font-size: 11px; }
 .chip { display: inline-block; padding: 2px 8px; border: 1px solid #ccc; }
 .gutter { display: inline-block; width: 3em; color: #999; }
 .axis span { display: inline-block; width: 60px; }
 .stack span { display: inline-block; width: 100%; }
 .wrap { width: 80px; white-space: pre-wrap; word-break: break-all; font: 14px monospace; }
 code { padding: 0 6px; background: #eee; }
</style></head><body>
<p id="tag"><b>Unlimited members</b><span class="tag">Recommended</span></p>
<p id="chip"><span class="chip">AI-era</span>No visible dates</p>
<pre id="code"><span class="gutter">18</span>import x from 'y';</pre>
<div class="axis"><span>0</span><span>50</span><span>100</span></div>
<div class="stack"><span>First line</span><span>Second line</span></div>
<p id="word"><b>Fak</b><i>turano</i> and <a href="#">link</a>, (<code>inline</code>) <code>call</code>.</p>
<pre class="wrap"><span>aaaaaaaaaaaaaaaaaaaa</span><span>bbbbbbbbbbbbbbbbbbbb</span></pre>
</body></html>"##;
    let outcome = check_blocking(input(
        source(vec![("index.html", html(page))]),
        "index.html",
        &[],
        rules,
    ));
    let Some(load) = outcome.loads.first() else {
        report.check(false, "the page of junctions was checked");
        return;
    };
    let text = &load.text;
    report.note(&format!("the text of junctions: {text:?}"));
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    for apart in [
        "members RECOMMENDED",
        "AI-era No visible dates",
        "18 import",
        "0 50 100",
        "First line Second line",
    ] {
        report.check(
            flat.contains(apart),
            &format!("{apart:?} is apart in the text"),
        );
    }
    report.check(
        text.contains(
            "First line
Second line",
        ),
        "two boxes on two lines are two lines",
    );
    for together in [
        "Fakturano and link, (inline) call.",
        "aaaaaaaaaaaaaaaaaaaabbbbbbbbbbbbbbbbbbbb",
    ] {
        report.check(
            flat.contains(together),
            &format!("{together:?} stays as it is (a word, punctuation, a line that wraps)"),
        );
    }
    let bare: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    report.check(
        bare.contains("UnlimitedmembersRECOMMENDEDAI-eraNovisibledates18importxfrom'y';050100FirstlineSecondline")
            && !text.contains('\u{fdd0}')
            && !text.contains('\u{fdd1}'),
        "only white space is added, and no mark is left",
    );
}

fn storage_is_the_pages_own_and_not_kept(report: &mut Report, rules: &Path) {
    let page = || {
        vec![(
            "index.html",
            html("<!doctype html><title>Storage</title><p>storage</p>"),
        )]
    };
    let host = Host::new();
    let (twelve, _) = host.page_at("page-12", page(), "index.html", rules);
    let set = eval(
        &twelve,
        "(() => { try { localStorage.setItem('writer-test', 'kept by 12');          sessionStorage.setItem('writer-test', 'session of 12');          return String(localStorage.getItem('writer-test')); }          catch (e) { return 'refused: ' + e; } })()",
    );
    report.check(
        set.as_deref() == Ok("kept by 12"),
        &format!("a page can keep something in its storage ({set:?})"),
    );
    let idb = eval_until(
        &twelve,
        "(() => { if (window.__idb) return window.__idb; window.__idb = '';          try { const open = indexedDB.open('writer-test', 1);          open.onupgradeneeded = () => open.result.createObjectStore('s');          open.onsuccess = () => { const tx = open.result.transaction('s', 'readwrite');            tx.objectStore('s').put('idb of 12', 'k'); tx.oncomplete = () => { window.__idb = 'stored'; }; };          open.onerror = () => { window.__idb = 'refused'; }; } catch (e) { window.__idb = 'refused: ' + e; }          return ''; })()",
        Duration::from_secs(5),
    );
    report.note(&format!("IndexedDB in page-12: {idb:?}"));

    let thirteen = Host::new();
    let (other, _) = thirteen.page_at("page-13", page(), "index.html", rules);
    let seen = eval(
        &other,
        "(() => { try { return String(localStorage.getItem('writer-test')); }          catch (e) { return 'refused: ' + e; } })()",
    );
    report.check(
        seen.as_deref() == Ok("null"),
        &format!("another page does not see what a page kept ({seen:?})"),
    );
    if idb == "stored" {
        let other_idb = eval_until(
            &other,
            "(() => { if (window.__seen) return window.__seen; window.__seen = '';              indexedDB.databases().then(list => { window.__seen = JSON.stringify(list.map(d => d.name)); })              .catch(e => { window.__seen = 'refused: ' + e; }); return ''; })()",
            Duration::from_secs(5),
        );
        report.check(
            other_idb == "[]",
            &format!("another page has none of a page's databases ({other_idb:?})"),
        );
    }

    let again = Host::new();
    let (same, _) = again.page_at("page-12", page(), "index.html", rules);
    let kept = eval(
        &same,
        "(() => { try { return String(localStorage.getItem('writer-test')); }          catch (e) { return 'refused: ' + e; } })()",
    );
    report.check(
        kept.as_deref() == Ok("kept by 12"),
        &format!("the same page shown again while the app runs has what it kept ({kept:?})"),
    );
    drop((twelve, other, same));
    drop((host, thirteen, again));
    wait(Duration::from_millis(500));

    // Nothing a page keeps is written beside the rule list (the app's <data dir>/WebKit).
    let mut found = Vec::new();
    let mut stack = vec![rules.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                found.push(
                    path.strip_prefix(rules)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
    }
    report.check(
        !found.is_empty() && found.iter().all(|name| name.starts_with("ContentRuleList")),
        &format!("the WebKit folder holds the rule list and nothing a page kept ({found:?})"),
    );
    report.check(
        testing::view_store_is_not_persistent(),
        "the views' data store is not persistent",
    );
}

fn nothing_plays_in_the_check(report: &mut Report, rules: &Path) {
    let page = r#"<!doctype html><title>Audio</title><audio id="a" src="silence.wav" autoplay></audio><p id="state">not asked</p>
<script>
 const a = document.getElementById('a');
 a.play().catch(() => {});
 setTimeout(() => { document.getElementById('state').textContent = 'paused=' + a.paused; }, 600);
</script>"#;
    let files = vec![
        ("index.html", html(page)),
        ("silence.wav", file("audio/wav", &silence())),
    ];
    let outcome = check_blocking(input(source(files.clone()), "index.html", &[], rules));
    let text = outcome
        .loads
        .first()
        .map(|load| load.text.clone())
        .unwrap_or_default();
    report.check(
        text.contains("paused=true"),
        &format!("an audio element does not play in the check ({text:?})"),
    );
    let host = Host::new();
    let (view, _) = host.page(files, "index.html", rules);
    wait(Duration::from_millis(800));
    let paused = eval(&view, "document.getElementById('a').paused");
    report.check(
        paused.as_deref() == Ok("true"),
        &format!("an audio element does not play in the view ({paused:?})"),
    );
}

/// A source whose one file takes long to come.
struct Slow {
    inner: MemorySource,
    slow: String,
    delay: Duration,
}

impl PageSource for Slow {
    fn file(&self, path: &str) -> Option<ServedFile> {
        if path == self.slow {
            std::thread::sleep(self.delay);
        }
        self.inner.file(path)
    }
}

fn settling_and_the_limit(report: &mut Report, _server: &Server, rules: &Path) {
    // A page that keeps asking for its own files settles at settle_max.
    let busy = r#"<!doctype html><title>Busy</title><p>busy</p>
<script>setInterval(() => fetch('tick.txt').catch(() => {}), 100);</script>"#;
    let mut busy_input = input(
        source(vec![
            ("index.html", html(busy)),
            ("tick.txt", file("text/plain", b"t")),
        ]),
        "index.html",
        &[],
        rules,
    );
    busy_input.settle_min = Duration::from_millis(300);
    busy_input.settle_max = Duration::from_millis(1500);
    let started = Instant::now();
    let outcome = check_blocking(busy_input);
    let took = started.elapsed();
    report.check(
        outcome
            .loads
            .first()
            .is_some_and(|load| load.finished && load.text.contains("busy")),
        "a busy page is read",
    );
    report.check(
        took < Duration::from_millis(1500 + 2500),
        &format!("settling ends within settle_max ({took:?} in all, 1.5 s to settle)"),
    );

    // A page whose picture never comes does not finish loading: the check ends at the limit.
    let slow = Slow {
        inner: MemorySource::new([
            (
                "index.html".to_string(),
                html("<!doctype html><title>Slow</title><img src=\"slow.png\">"),
            ),
            ("slow.png".to_string(), file("image/png", b"x")),
        ]),
        slow: "slow.png".to_string(),
        delay: Duration::from_secs(20),
    };
    let mut slow_input = input(Arc::new(slow), "index.html", &["never.html"], rules);
    slow_input.limit = Duration::from_secs(3);
    let started = Instant::now();
    let outcome = check_blocking(slow_input);
    let took = started.elapsed();
    report.check(
        outcome.loads.len() == 1 && !outcome.loads[0].finished && outcome.failure.is_none(),
        &format!(
            "a page that never finishes loading fails at the limit ({} loads, finished {:?})",
            outcome.loads.len(),
            outcome.loads.first().map(|load| load.finished)
        ),
    );
    report.check(
        took >= Duration::from_secs(3) && took < Duration::from_secs(6),
        &format!("the check ends at its limit ({took:?})"),
    );
}

fn the_view(report: &mut Report, server: &Server, rules: &Path) {
    let page = format!(
        r#"<!doctype html><html><head><title>View</title>
<style>body {{ margin: 0; font: 16px sans-serif; }} a {{ display: inline-block; padding: 20px; }}</style></head>
<body>
<h1 id="head">The harbour at night</h1>
<p><a id="out" href="{out}">A link to another site</a></p>
<p><a id="own" href="second.html">A file of the page</a></p>
<p>Some words to find: lighthouse.</p>
<input id="field" value="">
<script>
 window.__ranges = null;
 (async () => {{
   const ask = async (range) => {{
     const r = await fetch('big.bin', {{ headers: {{ Range: range }} }});
     const body = new Uint8Array(await r.arrayBuffer());
     return [r.status, body.length, r.headers.get('Content-Range'), body[0]];
   }};
   window.__ranges = JSON.stringify([await ask('bytes=0-1'), await ask('bytes=100-'),
                                     await ask('bytes=-10'), await ask('bytes=500-600')]);
 }})();
</script>
</body></html>"#,
        out = server.url("out")
    );
    let big: Vec<u8> = (0..200u32).map(|n| n as u8).collect();
    let files = vec![
        ("index.html", html(&page)),
        (
            "second.html",
            html("<!doctype html><title>Second</title><p>second</p>"),
        ),
        ("big.bin", file("application/octet-stream", &big)),
    ];
    let host = Host::new();
    let (view, events) = host.page(files, "index.html", rules);
    report.check(
        events.loaded.get() > 0,
        "a view in an offscreen window loads",
    );
    let title = eval(&view, "document.title");
    report.check(
        title.as_deref() == Ok("View"),
        &format!("its script runs ({title:?})"),
    );

    // Ranges.
    let ranges = eval_until(&view, "window.__ranges", Duration::from_secs(5));
    let expected = serde_json::json!([
        [206, 2, "bytes 0-1/200", 0],
        [206, 100, "bytes 100-199/200", 100],
        [206, 10, "bytes 190-199/200", 190],
        [416, 0, "bytes */200", null]
    ]);
    let got: serde_json::Value = serde_json::from_str(&ranges).unwrap_or_default();
    report.check(
        got == expected,
        &format!("ranges are answered as HTTP answers them ({ranges})"),
    );

    // A script's click on a link to another site opens nothing.
    let opened_before = events.opened.borrow().len();
    let _ = eval(&view, "document.getElementById('out').click()");
    let _ = eval(
        &view,
        "setTimeout(() => document.getElementById('out').click(), 100); 1",
    );
    let _ = eval(
        &view,
        "document.getElementById('out').dispatchEvent(new MouseEvent('click', { bubbles: true })); 1",
    );
    wait(Duration::from_millis(800));
    report.check(
        events.opened.borrow().len() == opened_before,
        &format!(
            "a link a script clicks does not reach open_externally ({:?})",
            events.opened.borrow()
        ),
    );
    let popup = eval(
        &view,
        &format!("String(window.open('{}'))", server.url("popup")),
    );
    wait(Duration::from_millis(300));
    report.check(
        events.opened.borrow().len() == opened_before,
        &format!("window.open to another site does nothing ({popup:?})"),
    );
    wait(Duration::from_millis(600));
    report.check(
        server.hits().is_empty(),
        &format!(
            "neither a script's click nor window.open opens a connection to the site ({:?})",
            server.hits()
        ),
    );
    report.check(
        view.stats().blocked_navigations >= 1,
        &format!("the cancelled navigations are counted ({:?})", view.stats()),
    );

    // alert, at most one in 5 s.
    let _ = eval(
        &view,
        "alert('The page says hello'); alert('a second one'); 1",
    );
    wait(Duration::from_millis(200));
    report.check(
        *events.alerts.borrow() == vec!["The page says hello".to_string()],
        &format!(
            "alert reaches alert, at most one in 5 s ({:?})",
            events.alerts.borrow()
        ),
    );

    // focus() does not call wants_focus; a focus of AppKit's does.
    view.unfocus();
    let before = events.wants_focus.get();
    view.focus();
    report.check(
        view.is_focused(),
        "focus() makes the web view first responder",
    );
    report.check(
        events.wants_focus.get() == before,
        "focus() does not call wants_focus",
    );
    view.unfocus();
    report.check(
        !view.is_focused() && host.first_responder() == host.gpui,
        "unfocus() gives the first responder to GPUI's view",
    );
    unsafe {
        let _: BOOL = msg_send![host.window, makeFirstResponder: view.native_view() as Id];
    }
    report.check(
        events.wants_focus.get() == before + 1,
        "a focus that did not come from focus() calls wants_focus",
    );

    // Keys: ctrl-return is asked and goes to GPUI's view; tab moves to a link.
    view.focus();
    let keys_before = gpui_keys(host.gpui);
    host.key(true, 36, "\r", menu_flag_ctrl());
    host.key(false, 36, "\r", menu_flag_ctrl());
    wait(Duration::from_millis(100));
    let asked = events
        .keys_asked
        .borrow()
        .iter()
        .any(|key| key.ctrl && key.key_code == 36);
    report.check(asked, "takes_key is asked for ctrl-return");
    report.check(
        gpui_keys(host.gpui) == keys_before + 1,
        "a key the owner takes goes to GPUI's view",
    );
    let _ = eval(
        &view,
        "document.activeElement && document.activeElement.blur(); 1",
    );
    host.key(true, 48, "\t", 0);
    host.key(false, 48, "\t", 0);
    wait(Duration::from_millis(300));
    let active = eval(
        &view,
        "document.activeElement ? document.activeElement.tagName : ''",
    );
    report.check(
        active.as_deref() == Ok("A"),
        &format!("tab moves the focus to a link ({active:?})"),
    );

    // A real click on the link reaches open_externally, and nothing goes out.
    wait(Duration::from_millis(1200));
    if let Some((x, y)) = middle_of(&view, "out") {
        host.mouse(1, x, y); // left mouse down
        host.mouse(2, x, y); // left mouse up
        testing::run_until(Duration::from_secs(3), || {
            events.opened.borrow().len() > opened_before
        });
        report.check(
            events.opened.borrow().last().map(String::as_str) == Some(server.url("out").as_str()),
            &format!(
                "a click on the link reaches open_externally ({:?})",
                events.opened.borrow()
            ),
        );
        wait(Duration::from_millis(600));
        report.check(
            server.hits().is_empty(),
            &format!(
                "the page itself opens no connection to the link's site ({:?})",
                server.hits()
            ),
        );
    } else {
        report.check(false, "the link has a place on the page");
    }
    // A click on a link to a file of the page goes there in the view.
    wait(Duration::from_millis(1200));
    let loaded = events.loaded.get();
    if let Some((x, y)) = middle_of(&view, "own") {
        host.mouse(1, x, y);
        host.mouse(2, x, y);
        testing::run_until(Duration::from_secs(5), || events.loaded.get() > loaded);
        let title = eval(&view, "document.title");
        report.check(
            title.as_deref() == Ok("Second"),
            &format!("a link to a file of the page opens it in the view ({title:?})"),
        );
    }
    view.reload();
    testing::run_until(Duration::from_secs(5), || events.loaded.get() > loaded + 1);
    let title = eval(&view, "document.title");
    report.check(
        title.as_deref() == Ok("View"),
        &format!("reload() loads the entry again ({title:?})"),
    );

    // Find, zoom, snapshot.
    let found = find(&view, "lighthouse", false);
    report.check(found == Some(true), "find finds a word of the page");
    let found = find(&view, "nowhere-to-be-found", false);
    report.check(found == Some(false), "find does not find what is not there");
    view.clear_find();
    let width = eval(&view, "window.innerWidth");
    view.set_zoom(2.0);
    wait(Duration::from_millis(300));
    let zoomed = eval(&view, "window.innerWidth");
    report.check(
        width.as_deref() == Ok("1280") && zoomed.as_deref() == Ok("640"),
        &format!("the zoom draws the page twice as large ({width:?} then {zoomed:?})"),
    );
    view.set_zoom(1.0);
    view.set_appearance(Appearance::Dark, (0.1, 0.1, 0.1));
    let dark = eval(&view, "matchMedia('(prefers-color-scheme: dark)').matches");
    report.check(
        dark.as_deref() == Ok("true"),
        &format!("the appearance reaches the page ({dark:?})"),
    );
    view.set_appearance(Appearance::Light, (0.98, 0.97, 0.95));
    let picture: Rc<RefCell<Option<Option<innerword_web::Bitmap>>>> = Rc::new(RefCell::new(None));
    let answer = picture.clone();
    view.snapshot(move |bitmap| *answer.borrow_mut() = Some(bitmap));
    testing::run_until(Duration::from_secs(5), || picture.borrow().is_some());
    let bitmap = picture.borrow_mut().take().flatten();
    report.check(
        bitmap.as_ref().is_some_and(|bitmap| {
            bitmap.width > 0
                && bitmap.height > 0
                && bitmap.bgra.len() == (bitmap.width * bitmap.height * 4) as usize
                && bitmap.bgra.chunks(4).any(|pixel| pixel[0] < 128)
        }),
        &format!("a snapshot is a picture with something on it ({bitmap:?})"),
    );

    // Edit commands reach the page.
    let _ = eval(&view, "document.getElementById('field').focus(); 1");
    view.focus();
    view.edit(EditCommand::SelectAll);
    wait(Duration::from_millis(200));
    let selected = eval(
        &view,
        "String(window.getSelection()).length > 0 || document.activeElement.id",
    );
    report.note(&format!("select all in the page: {selected:?}"));

    // Holes: not hit inside, the web view outside.
    let hole = Rect {
        x: 100.,
        y: 600.,
        width: 300.,
        height: 60.,
    };
    view.set_holes(&[hole]);
    let inside = hit(&host, 200., 630.);
    let outside = hit(&host, 600., 300.);
    let web_view = view.native_view() as Id;
    report.check(
        inside == host.gpui,
        "a point inside a hole hits GPUI's view, not the web view",
    );
    report.check(
        is_within(outside, web_view),
        "a point outside the holes hits the web view",
    );
    view.set_holes(&[]);
    let cleared = hit(&host, 200., 630.);
    report.check(
        is_within(cleared, web_view),
        "with the holes cleared the page is hit there again",
    );

    // A cover (a GPUI modal or context menu over the view): the view stays shown and
    // live under the backdrop's colour, takes no event, and its page is not told that
    // it is hidden.
    let _ = eval(
        &view,
        "window.__seen = [];
         document.addEventListener('visibilitychange', () => window.__seen.push(document.visibilityState));
         for (const kind of ['pointerdown', 'pointerup', 'mousedown', 'click']) {
           window.addEventListener(kind, () => window.__seen.push(kind), true);
         }
         1",
    );
    // (An offscreen window is occluded: its page may be hidden already. What counts is
    // that the cover changes nothing of it.)
    let visibility = eval(&view, "document.visibilityState");
    let started = Instant::now();
    view.set_cover(Some((0., 0., 0., 0.45)));
    let took = started.elapsed();
    report.check(
        hit(&host, 600., 300.) == host.gpui && hit(&host, 200., 630.) == host.gpui,
        "a covered view is hit nowhere: GPUI's view is",
    );
    host.mouse(1, 600., 300.); // left mouse down
    host.mouse(2, 600., 300.); // left mouse up
    wait(Duration::from_millis(300));
    let seen = eval(&view, "JSON.stringify(window.__seen)");
    let visibility_now = eval(&view, "document.visibilityState");
    report.check(
        seen.as_deref() == Ok("[]") && visibility_now == visibility,
        &format!(
            "a covered page gets no press and no visibilitychange ({seen:?}, {visibility:?} then {visibility_now:?})"
        ),
    );
    let (hidden, dim_alpha) = unsafe {
        let hidden: BOOL = msg_send![web_view, isHidden];
        let subviews: Id = msg_send![web_view, subviews];
        let last: Id = msg_send![subviews, lastObject];
        let layer: Id = msg_send![last, layer];
        let colour: *const c_void = if layer.is_null() {
            std::ptr::null()
        } else {
            msg_send![layer, backgroundColor]
        };
        let alpha = if colour.is_null() {
            0.
        } else {
            CGColorGetAlpha(colour)
        };
        (hidden != NO, alpha)
    };
    report.check(
        !hidden && (dim_alpha - 0.45).abs() < 0.01,
        &format!(
            "a covered view stays shown, its topmost subview the cover's colour (hidden {hidden}, alpha {dim_alpha})"
        ),
    );
    report.check(
        view.cover() == Some((0., 0., 0., 0.45)),
        "the view says what it is covered with",
    );
    report.note(&format!("covering the view took {took:?}"));
    view.set_cover(None);
    host.mouse(1, 600., 300.);
    host.mouse(2, 600., 300.);
    wait(Duration::from_millis(300));
    let seen = eval(&view, "JSON.stringify(window.__seen)");
    let topmost_is_ours = unsafe {
        let subviews: Id = msg_send![web_view, subviews];
        let last: Id = msg_send![subviews, lastObject];
        let layer: Id = if last.is_null() {
            NIL
        } else {
            msg_send![last, layer]
        };
        let colour: *const c_void = if layer.is_null() {
            std::ptr::null()
        } else {
            msg_send![layer, backgroundColor]
        };
        !colour.is_null() && (CGColorGetAlpha(colour) - 0.45).abs() < 0.01
    };
    report.check(
        is_within(hit(&host, 600., 300.), web_view) && !topmost_is_ours,
        "uncovered, the page is hit again and the cover's colour is gone",
    );
    report.check(
        seen.as_deref()
            .is_ok_and(|seen| seen.contains("pointerdown")),
        &format!("uncovered, a press reaches the page again ({seen:?})"),
    );

    // The right-click menu, where WebKit can be driven to open it.
    testing::record_menus(true);
    testing::choose_in_menus(Some("Open Link in Browser"));
    let opened_before = events.opened.borrow().len();
    wait(Duration::from_millis(1200));
    if let Some((x, y)) = middle_of(&view, "out") {
        host.mouse(5, x, y); // mouse moved: the link under the pointer
        wait(Duration::from_millis(200));
        host.mouse(3, x, y); // right mouse down
        testing::run_until(Duration::from_secs(3), || {
            !testing::recorded_menus().is_empty()
        });
        host.mouse(4, x, y);
        let menus = testing::recorded_menus();
        match menus.first() {
            None => report
                .note("WebKit's menu could not be opened here (person check 8 holds the filter)"),
            Some(items) => {
                report.note(&format!("the menu on a link: {items:?}"));
                report.check(
                    !items.iter().any(|(identifier, action, _)| {
                        menu::REMOVED.contains(&identifier.as_str())
                            || menu::REMOVED_ACTIONS.contains(&action.as_str())
                    }),
                    "WebKit's menu on a link has none of the removed items",
                );
                report.check(
                    items.iter().any(|(_, _, title)| title == "Find in Page…")
                        && items.iter().any(|(_, _, title)| title == "Reload Page"),
                    "the owner's items are at the end of the menu",
                );
                report.check(
                    events.menu_asked.borrow().last() == Some(&Some(server.url("out"))),
                    &format!(
                        "the owner is told the link under the pointer ({:?})",
                        events.menu_asked.borrow()
                    ),
                );
                report.check(
                    items
                        .iter()
                        .any(|(_, _, title)| title == "Open Link in Browser"),
                    "the menu on a link to another site has Open Link in Browser",
                );
                report.check(
                    events.opened.borrow().len() == opened_before + 1
                        && events.opened.borrow().last() == Some(&server.url("out")),
                    &format!(
                        "choosing Open Link in Browser reaches open_externally ({:?})",
                        events.opened.borrow()
                    ),
                );
            }
        }
    }
    testing::record_menus(false);
    testing::choose_in_menus(None);

    // Print from a timer's callback.
    let printed: Rc<RefCell<Option<Result<innerword_web::PrintedPdf, String>>>> =
        Rc::new(RefCell::new(None));
    let answer = printed.clone();
    let view = Rc::new(view);
    let printing = view.clone();
    testing::after(Duration::from_millis(10), move || {
        printing.print_to_pdf(move |result| *answer.borrow_mut() = Some(result));
    });
    testing::run_until(Duration::from_secs(20), || printed.borrow().is_some());
    let result = printed.borrow_mut().take();
    report.check(
        matches!(&result, Some(Ok(pdf)) if pdf.pages >= 1 && pdf.bytes.starts_with(b"%PDF")),
        &format!("the view prints into a PDF with pages ({result:?})"),
    );
    if let Some(Ok(pdf)) = &result {
        let text = pdf_text(&pdf.bytes);
        report.check(
            text.contains("The harbour at night"),
            &format!(
                "the printed page has the page's words ({:?})",
                text.chars().take(80).collect::<String>()
            ),
        );
    }
    let view = Rc::try_unwrap(view).ok().expect("the one view");

    // A page dropped after its superview left the window does not crash.
    unsafe {
        let content: Id = msg_send![host.window, contentView];
        let container: Id = msg_send![class!(NSView), alloc];
        let bounds: NSRect = msg_send![content, bounds];
        let container: Id = msg_send![container, initWithFrame: bounds];
        let _: () = msg_send![content, addSubview: container];
        let inner: Id = msg_send![gpui_view_class(), alloc];
        let inner: Id = msg_send![inner, initWithFrame: bounds];
        let _: () = msg_send![container, addSubview: inner];
        let events = Rc::new(Events::default());
        let orphan = WebPage::new(
            inner.cast(),
            ViewConfig {
                host: "page-13".to_string(),
                entry: "index.html".to_string(),
                source: source(vec![("index.html", html("<p>orphan</p>"))]),
                rules_dir: rules.to_path_buf(),
                appearance: Appearance::System,
                background: (1., 1., 1.),
            },
            events.clone(),
        );
        orphan.set_frame(Rect {
            x: 0.,
            y: 0.,
            width: 400.,
            height: 300.,
        });
        testing::run_until(Duration::from_secs(5), || events.loaded.get() > 0);
        orphan.focus();
        let _: () = msg_send![container, removeFromSuperview];
        drop(orphan);
        let _: () = msg_send![container, release];
        let _: () = msg_send![inner, release];
        wait(Duration::from_millis(200));
        report.check(
            true,
            "a page dropped after its superview left the window does not crash",
        );
    }
    view.focus();
    drop(view);
    report.check(
        host.first_responder() == host.gpui,
        "a dropped page gives the first responder back to GPUI's view",
    );
}

/// A page whose own listeners click another link on every click and every key of the
/// reader: only the link the reader clicked, or pressed enter on, opens (ISO-4).
fn the_gate_opens_only_the_link_the_reader_acted_on(
    report: &mut Report,
    server: &Server,
    rules: &Path,
) {
    let page = format!(
        r#"<!doctype html><html><head><title>Gate</title>
<style>body {{ margin: 0; font: 16px sans-serif; }} a, p {{ display: inline-block; padding: 20px; margin: 0; }}</style></head>
<body>
<p><a id="real" href="{real}">The link the reader opens</a></p>
<p><a id="other" href="{other}">The link the page wants opened</a></p>
<p id="text">Plain words to click on</p>
<p><input id="field" value=""></p>
<svg width="200" height="40"><a id="drawn" href="{drawn}"><text x="10" y="25">A drawn link</text></a></svg>
<p><iframe id="frame" src="inner.html" style="width: 400px; height: 80px; border: 0"></iframe></p>
<script>
 const other = () => document.getElementById('other').click();
 document.addEventListener('click', (event) => {{ if (event.target.id !== 'other') other(); }}, true);
 document.addEventListener('keydown', () => other(), true);
 document.addEventListener('mouseup', () => setTimeout(other, 50));
</script>
</body></html>"#,
        real = server.url("real"),
        other = server.url("other"),
        drawn = server.url("drawn"),
    );
    let inner = format!(
        r#"<!doctype html><style>body {{ margin: 0; }} a {{ display: inline-block; padding: 20px; }}</style>
<a id="framed" href="{framed}" target="_blank">A link in a frame</a>"#,
        framed = server.url("framed")
    );
    let host = Host::new();
    let (view, events) = host.page(
        vec![("index.html", html(&page)), ("inner.html", html(&inner))],
        "index.html",
        rules,
    );
    let real = server.url("real");
    let other = server.url("other");
    let opened = || events.opened.borrow().clone();
    let settle = || wait(Duration::from_millis(700));
    let click_on = |id: &str| -> bool {
        match middle_of(&view, id) {
            Some((x, y)) => {
                host.mouse(1, x, y);
                host.mouse(2, x, y);
                true
            }
            None => false,
        }
    };

    // A click on plain words: the page's listeners click another link, nothing opens.
    report.check(click_on("text"), "the words have a place on the page");
    settle();
    report.check(
        opened().is_empty(),
        &format!(
            "a click on words does not open the link the page clicks ({:?})",
            opened()
        ),
    );
    // A key typed in a field, and enter in it: nothing opens.
    view.focus();
    let _ = eval(&view, "document.getElementById('field').focus(); 1");
    host.key(true, 0, "a", 0);
    host.key(false, 0, "a", 0);
    host.key(true, 36, "\r", 0);
    host.key(false, 36, "\r", 0);
    settle();
    report.check(
        opened().is_empty(),
        &format!(
            "a key in a field does not open the link the page clicks ({:?})",
            opened()
        ),
    );
    // Enter on the focused link opens that link and no other.
    let _ = eval(&view, "document.getElementById('real').focus(); 1");
    host.key(true, 36, "\r", 0);
    host.key(false, 36, "\r", 0);
    testing::run_until(Duration::from_secs(3), || !opened().is_empty());
    settle();
    report.check(
        opened() == vec![real.clone()],
        &format!(
            "enter on a link opens that link and no other ({:?})",
            opened()
        ),
    );
    // A click on the link opens it, and the page's clicks of the other link do not.
    report.check(click_on("real"), "the link has a place on the page");
    testing::run_until(Duration::from_secs(3), || opened().len() > 1);
    settle();
    report.check(
        opened() == vec![real.clone(), real.clone()],
        &format!(
            "a click on a link opens that link and no other ({:?})",
            opened()
        ),
    );
    // A link of an SVG drawing is a link too.
    report.check(click_on("drawn"), "the drawn link has a place on the page");
    testing::run_until(Duration::from_secs(3), || opened().len() > 2);
    settle();
    report.check(
        opened().last() == Some(&server.url("drawn")) && opened().len() == 3,
        &format!("a click on a link of a drawing opens it ({:?})", opened()),
    );
    // A link in a frame of the page's own files that opens in a window of its own (one
    // that would open in the frame itself is the frame's navigation to another site,
    // which the page's policy, frame-src, stops before WebKit asks).
    let framed = eval(
        &view,
        "(() => { const f = document.getElementById('frame').getBoundingClientRect(); \
         const r = document.getElementById('frame').contentDocument.getElementById('framed').getBoundingClientRect(); \
         return JSON.stringify([f.left + r.left + r.width / 2, f.top + r.top + r.height / 2]); })()",
    )
    .ok()
    .and_then(|text| serde_json::from_str::<Vec<f64>>(&text).ok());
    match framed {
        Some(point) if point.len() == 2 => {
            host.mouse(1, point[0], point[1]);
            host.mouse(2, point[0], point[1]);
            testing::run_until(Duration::from_secs(3), || opened().len() > 3);
            settle();
            report.check(
                opened().last() == Some(&server.url("framed")) && opened().len() == 4,
                &format!(
                    "a click on a link in a frame of the page opens it ({:?})",
                    opened()
                ),
            );
        }
        _ => report.check(
            false,
            &format!("the link in the frame has a place ({framed:?})"),
        ),
    }
    report.check(
        !opened().contains(&other),
        &format!(
            "the link only the page clicked never opened ({:?})",
            opened()
        ),
    );
}

/// A picture and a frame under `loading="lazy"` far below the first screen are asked
/// for by the check: one that is not among the page's files is missing, one on the web
/// is blocked (ISO-3).
fn lazy_pieces_are_asked_for(report: &mut Report, server: &Server, rules: &Path) {
    let page = format!(
        r#"<!doctype html><html><head><title>Lazy</title></head><body>
<h1>Lazy</h1><div style="height: 6000px">tall</div>
<img loading="lazy" src="pics/late.png" alt="late">
<iframe loading="lazy" src="frames/late.html"></iframe>
<img loading="lazy" src="{web}" alt="from the web">
<div id="later"></div>
<script>
 const img = document.createElement('img');
 img.loading = 'lazy';
 img.src = 'pics/added.png';
 document.getElementById('later').appendChild(img);
</script>
</body></html>"#,
        web = server.url("lazy.png")
    );
    let png = writer_png();
    let missing = |files: Vec<(&str, ServedFile)>| {
        let outcome = check_blocking(input(source(files), "index.html", &[], rules));
        let load = outcome.loads.first().cloned();
        (outcome, load)
    };
    let (outcome, load) = missing(vec![("index.html", html(&page))]);
    report.check(
        outcome.failure.is_none(),
        &format!("the lazy check ran ({:?})", outcome.failure),
    );
    let load = load.unwrap_or_default();
    let mut gone = load.missing.clone();
    gone.sort();
    report.check(
        gone == vec!["frames/late.html", "pics/added.png", "pics/late.png"],
        &format!(
            "lazy pieces that are not there are missing ({:?})",
            load.missing
        ),
    );
    let blocked = blocked_paths(&outcome);
    report.check(
        blocked.iter().any(|(url, _)| url.ends_with("/lazy.png")),
        &format!("a lazy picture on the web is blocked ({blocked:?})"),
    );
    let (_, load) = missing(vec![
        ("index.html", html(&page)),
        ("pics/late.png", file("image/png", &png)),
        ("pics/added.png", file("image/png", &png)),
        (
            "frames/late.html",
            html("<!doctype html><title>Late</title><p>late</p>"),
        ),
    ]);
    let load = load.unwrap_or_default();
    report.check(
        load.finished && load.missing.is_empty(),
        &format!("with them there nothing is missing ({:?})", load.missing),
    );
}

/// A PNG of one pixel.
fn writer_png() -> Vec<u8> {
    vec![
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f,
        0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8,
        0xcf, 0xc0, 0xf0, 0x1f, 0x00, 0x05, 0x00, 0x01, 0xff, 0x89, 0x99, 0x3d, 0x1d, 0x00, 0x00,
        0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ]
}

fn menu_flag_ctrl() -> usize {
    1 << 18
}

fn find(view: &WebPage, query: &str, backwards: bool) -> Option<bool> {
    let slot: Rc<Cell<Option<bool>>> = Rc::new(Cell::new(None));
    let answer = slot.clone();
    view.find(query, backwards, move |found| answer.set(Some(found)));
    testing::run_until(Duration::from_secs(5), || slot.get().is_some());
    slot.get()
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGColorGetAlpha(color: *const c_void) -> f64;
}

/// What the content view hits at a point of the page (from the top left).
fn hit(host: &Host, x: f64, y: f64) -> Id {
    unsafe {
        let content: Id = msg_send![host.window, contentView];
        let point = NSPoint {
            x,
            y: host.content_height() - y,
        };
        msg_send![content, hitTest: point]
    }
}

fn is_within(view: Id, ancestor: Id) -> bool {
    if view.is_null() {
        return false;
    }
    unsafe {
        let within: BOOL = msg_send![view, isDescendantOf: ancestor];
        within != NO
    }
}

fn pages_switched_during_loads(report: &mut Report, rules: &Path) {
    // Media make many range requests; a page that goes while they are read must not
    // be answered (answering a stopped task raises an exception).
    let page = r#"<!doctype html><title>Media</title>
<audio src="a.wav" preload="auto"></audio><audio src="b.wav" preload="auto"></audio>
<img src="slow1.png"><img src="slow2.png">"#;
    let host = Host::new();
    for round in 0..8 {
        let slow = Slow {
            inner: MemorySource::new([
                ("index.html".to_string(), html(page)),
                ("a.wav".to_string(), file("audio/wav", &silence())),
                ("b.wav".to_string(), file("audio/wav", &silence())),
                ("slow1.png".to_string(), file("image/png", b"x")),
                ("slow2.png".to_string(), file("image/png", b"x")),
            ]),
            slow: if round % 2 == 0 { "slow1.png" } else { "a.wav" }.to_string(),
            delay: Duration::from_millis(250),
        };
        let events = Rc::new(Events::default());
        let page = unsafe {
            WebPage::new(
                host.gpui.cast(),
                ViewConfig {
                    host: format!("page-{round}"),
                    entry: "index.html".to_string(),
                    source: Arc::new(slow),
                    rules_dir: rules.to_path_buf(),
                    appearance: Appearance::Light,
                    background: (1., 1., 1.),
                },
                events,
            )
        };
        page.set_frame(Rect {
            x: 0.,
            y: 0.,
            width: 800.,
            height: 600.,
        });
        wait(Duration::from_millis(60 + round * 15));
        drop(page);
    }
    wait(Duration::from_millis(800));
    report.check(
        true,
        "pages switched fast while their media and pictures load raise nothing",
    );
}

fn printed_offscreen(report: &mut Report, rules: &Path) {
    let page = "<!doctype html><title>Printed</title><h1>A page on paper</h1><p>Its words.</p>";
    let slot: Rc<RefCell<Option<Result<innerword_web::PrintedPdf, String>>>> =
        Rc::new(RefCell::new(None));
    let answer = slot.clone();
    print_offscreen(
        PdfInput {
            source: source(vec![("index.html", html(page))]),
            entry: "index.html".to_string(),
            settle_max: Duration::from_millis(1000),
            limit: Duration::from_secs(30),
            rules_dir: rules.to_path_buf(),
        },
        move |result| *answer.borrow_mut() = Some(result),
    );
    testing::run_until(Duration::from_secs(40), || slot.borrow().is_some());
    let result = slot.borrow_mut().take();
    report.check(
        matches!(&result, Some(Ok(pdf)) if pdf.pages >= 1 && testing::pdf_pages(&pdf.bytes).is_some()),
        &format!("print_offscreen gives a PDF with pages ({result:?})"),
    );
    if let Some(Ok(pdf)) = &result {
        report.note(&format!(
            "printed offscreen: {} pages of {:?} pt",
            pdf.pages, pdf.paper
        ));
        let text = pdf_text(&pdf.bytes);
        report.check(
            text.contains("A page on paper") && text.contains("Its words."),
            &format!("the page printed offscreen has its words ({text:?})"),
        );
    }
    // The same for a program, as the child `innerword __page-pdf` runs it.
    let blocking = innerword_web::print_blocking(PdfInput {
        source: source(vec![("index.html", html(page))]),
        entry: "index.html".to_string(),
        settle_max: Duration::from_millis(500),
        limit: Duration::from_secs(30),
        rules_dir: rules.to_path_buf(),
    });
    report.check(
        matches!(&blocking, Ok(pdf) if pdf.pages >= 1),
        &format!("print_blocking gives a PDF with pages ({blocking:?})"),
    );
    // A page that does not load is no PDF.
    let failed = innerword_web::print_blocking(PdfInput {
        source: source(vec![]),
        entry: "missing.html".to_string(),
        settle_max: Duration::from_millis(300),
        limit: Duration::from_secs(10),
        rules_dir: rules.to_path_buf(),
    });
    report.check(
        matches!(&failed, Err(why) if why.contains("missing.html")),
        &format!("a page whose entry is not there is not printed ({failed:?})"),
    );
}
