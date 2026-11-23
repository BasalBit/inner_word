//! `WebPage`: the live view of an open page, a native subview above GPUI's view.

use std::{cell::RefCell, ffi::c_void, rc::Rc};

use block::ConcreteBlock;
use objc::{
    class, msg_send,
    runtime::{BOOL, NO, YES},
    sel, sel_impl,
};

use crate::{
    Appearance, Bitmap, EditCommand, PageEvents, PrintedPdf, Rect, ViewConfig, ViewExtras,
    ViewStats, bridge, config, delegate, frame, http,
    ns::{
        self, Id, NIL, NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSRect, describe,
        error_text, guarded, is_boolean, is_kind, kCAFillRuleEvenOdd, nsstring, release, retain,
        string_of,
    },
    print, rules,
    scheme::{self, RequestLog, SchemeState},
    view::{self, ViewHost, ViewShared},
};

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPathCreateMutable() -> *mut c_void;
    fn CGPathAddRect(path: *mut c_void, transform: *const c_void, rect: NSRect);
    fn CGPathRelease(path: *mut c_void);
}

/// Where a parked view lies: far to the left of any window's content.
const PARKED_X: f64 = -100_000.;

/// `NSWindowAbove`.
const ABOVE: isize = 1;
/// `NSViewMinYMargin` and `NSViewMaxYMargin`: the margin that grows when the superview
/// grows, so that the view keeps its distance from the top.
const MIN_Y_MARGIN: usize = 8;
const MAX_Y_MARGIN: usize = 32;

/// The live view of an open page: a `WKWebView` (a subclass of it) that lies above
/// GPUI's view in GPUI's superview (the window's content view), serves the page's files
/// from its `PageSource`, and has no way to the network, to the user's files or to the
/// app (section 5 of the design). Main thread only; not `Send`.
pub struct WebPage {
    web_view: Id,
    gpui_view: Id,
    delegate: Id,
    shared: Rc<ViewShared>,
}

impl WebPage {
    /// Puts a view above `gpui_view` (an `NSView*`, retained until Drop) in its
    /// superview and loads the entry once the rule list is compiled (at once when the
    /// process has it already). The view has no frame until [`WebPage::set_frame`].
    /// When the rule list cannot be had, `events.failed` says why and nothing loads.
    ///
    /// # Safety
    ///
    /// `gpui_view` must be a valid `NSView *`. On the main thread, after
    /// [`crate::available`] answered `Ok`.
    pub unsafe fn new(
        gpui_view: *mut c_void,
        config: ViewConfig,
        events: Rc<dyn PageEvents>,
    ) -> WebPage {
        // SAFETY: the caller's promise, passed on.
        unsafe { WebPage::new_with(gpui_view, config, ViewExtras::default(), events) }
    }

    /// As [`WebPage::new`], with what only a whiteboard's view has ([`ViewExtras`]): the
    /// bridge's handler (added before the web view is made), no navigation after the
    /// entry, the view's own policy. `ViewExtras::default()` is a page's view.
    ///
    /// # Safety
    ///
    /// As [`WebPage::new`].
    pub unsafe fn new_with(
        gpui_view: *mut c_void,
        config: ViewConfig,
        extras: ViewExtras,
        events: Rc<dyn PageEvents>,
    ) -> WebPage {
        let gpui_view = gpui_view as Id;
        // SAFETY: the caller's promise: a live NSView, on the main thread; WebKit's
        // and AppKit's own selectors with the types their headers declare.
        unsafe {
            retain(gpui_view);
            let log = Rc::new(RequestLog::default());
            let shared = Rc::new(ViewShared::new(
                events,
                &config.host,
                &config.entry,
                log.clone(),
            ));
            shared.gpui_view.set(gpui_view);
            shared.one_document.set(extras.one_document);
            let handler = scheme::new_handler(
                SchemeState::new(config.source.clone(), &config.host, &config.entry, log)
                    .with_policy(extras.csp.unwrap_or(crate::CONTENT_SECURITY_POLICY)),
            );
            let configuration = config::configuration(handler, config::view_data_store());
            release(handler);
            if let Some(bridge) = extras.bridge {
                bridge::install(configuration, bridge, &shared);
            }
            let web_view =
                view::new_web_view(NSRect::default(), configuration, Some(shared.clone()));
            release(configuration);
            config::settle_web_view(web_view);
            shared.web_view.set(web_view);
            let delegate = delegate::new_delegate(Rc::new(ViewHost(shared.clone())));
            let _: () = msg_send![web_view, setNavigationDelegate: delegate];
            let _: () = msg_send![web_view, setUIDelegate: delegate];
            let page = WebPage {
                web_view,
                gpui_view,
                delegate,
                shared: shared.clone(),
            };
            page.set_appearance(config.appearance, config.background);
            let superview: Id = msg_send![gpui_view, superview];
            if !superview.is_null() {
                let flipped: BOOL = msg_send![superview, isFlipped];
                let mask = if flipped != NO {
                    MAX_Y_MARGIN
                } else {
                    MIN_Y_MARGIN
                };
                let _: () = msg_send![web_view, setAutoresizingMask: mask];
                let _: () = msg_send![superview,
                    addSubview: web_view
                    positioned: ABOVE
                    relativeTo: gpui_view];
            }
            let weak = Rc::downgrade(&shared);
            rules::when_ready(&config.rules_dir, move |result| {
                let Some(shared) = weak.upgrade() else {
                    return;
                };
                let web_view = shared.web_view.get();
                if web_view.is_null() {
                    return;
                }
                match result {
                    Ok(list) => {
                        config::add_rules(web_view, list);
                        load_entry(&shared);
                    }
                    Err(why) => shared
                        .events
                        .failed(format!("The page cannot be shown offline: {why}.")),
                }
            });
            page
        }
    }

    /// Puts the view on `frame`, in points from the top left of its superview (the
    /// window's content view, where GPUI's view lies). The same frame again sends
    /// nothing to AppKit.
    pub fn set_frame(&self, frame: Rect) {
        // SAFETY: AppKit's own selectors on live views.
        unsafe {
            let superview: Id = msg_send![self.web_view, superview];
            if superview.is_null() {
                return;
            }
            let bounds: NSRect = msg_send![superview, bounds];
            let flipped: BOOL = msg_send![superview, isFlipped];
            let target = frame::to_appkit(frame, bounds.size.height, flipped != NO);
            let current: NSRect = msg_send![self.web_view, frame];
            if current != target {
                let _: () = msg_send![self.web_view, setFrame: target];
                self.apply_holes();
            }
        }
    }

    pub fn set_hidden(&self, hidden: bool) {
        // SAFETY: NSView's own.
        unsafe {
            let _: () = msg_send![self.web_view, setHidden: ns::yes_no(hidden)];
        }
    }

    /// In the view's own points, top left origin; empty: no mask. A hole is not drawn
    /// (a layer mask with the even-odd rule) and not hit (`hitTest:` answers nil
    /// inside it, so AppKit asks GPUI's view, which lies under it).
    pub fn set_holes(&self, holes: &[Rect]) {
        {
            let mut current = self.shared.holes.borrow_mut();
            if current.as_slice() == holes {
                return;
            }
            *current = holes.to_vec();
        }
        self.apply_holes();
    }

    fn apply_holes(&self) {
        let holes = self.shared.holes.borrow().clone();
        // SAFETY: AppKit's, CoreAnimation's and CoreGraphics' own; the path is
        // released here, the mask is held by the layer.
        unsafe {
            let _: () = msg_send![self.web_view, setWantsLayer: YES];
            let layer: Id = msg_send![self.web_view, layer];
            if layer.is_null() {
                return;
            }
            if holes.is_empty() {
                let _: () = msg_send![layer, setMask: NIL];
                return;
            }
            let bounds: NSRect = msg_send![self.web_view, bounds];
            let flipped: BOOL = msg_send![self.web_view, isFlipped];
            let path = CGPathCreateMutable();
            CGPathAddRect(path, std::ptr::null(), bounds);
            for hole in &holes {
                let mut rect = frame::to_appkit(*hole, bounds.size.height, flipped != NO);
                rect.origin.x += bounds.origin.x;
                rect.origin.y += bounds.origin.y;
                CGPathAddRect(path, std::ptr::null(), rect);
            }
            let mask: Id = msg_send![class!(CAShapeLayer), layer];
            let _: () = msg_send![mask, setFrame: bounds];
            let _: () = msg_send![mask, setPath: path];
            let _: () = msg_send![mask, setFillRule: kCAFillRuleEvenOdd];
            CGPathRelease(path);
            let _: () = msg_send![layer, setMask: mask];
        }
    }

    /// Makes the web view its window's first responder. `PageEvents::wants_focus` is not
    /// called for it.
    pub fn focus(&self) {
        // SAFETY: AppKit's own; the flag is cleared on every way out.
        unsafe {
            let window: Id = msg_send![self.web_view, window];
            if window.is_null() {
                return;
            }
            self.shared.focusing.set(true);
            let _: BOOL = msg_send![window, makeFirstResponder: self.web_view];
            self.shared.focusing.set(false);
        }
    }

    /// Gives the first responder to GPUI's view, when the web view has it.
    pub fn unfocus(&self) {
        // SAFETY: AppKit's own.
        unsafe {
            let window: Id = msg_send![self.web_view, window];
            if window.is_null() {
                return;
            }
            let first: Id = msg_send![window, firstResponder];
            if first != self.web_view {
                return;
            }
            let gpui_window: Id = msg_send![self.gpui_view, window];
            let next = if gpui_window == window {
                self.gpui_view
            } else {
                NIL
            };
            self.shared.focusing.set(true);
            let _: BOOL = msg_send![window, makeFirstResponder: next];
            self.shared.focusing.set(false);
        }
    }

    /// Whether the web view is its window's first responder.
    pub fn is_focused(&self) -> bool {
        // SAFETY: AppKit's own.
        unsafe {
            let window: Id = msg_send![self.web_view, window];
            if window.is_null() {
                return false;
            }
            let first: Id = msg_send![window, firstResponder];
            first == self.web_view
        }
    }

    /// A still picture of the view as it is now, as BGRA pixels at the size of the
    /// backing store (device pixels); `None` when WebKit made none. `done` is called on
    /// the main thread, from WebKit's completion handler.
    pub fn snapshot(&self, done: impl FnOnce(Option<Bitmap>) + 'static) {
        let slot = RefCell::new(Some(done));
        let block = ConcreteBlock::new(move |image: Id, _error: Id| {
            // SAFETY: an NSImage or nil.
            let bitmap = unsafe { crate::pixels::bitmap_of(image) };
            let done = slot.borrow_mut().take();
            if let Some(done) = done {
                guarded("a snapshot's end", move || done(bitmap));
            }
        })
        .copy();
        // SAFETY: WebKit's own; nil takes the whole visible view.
        unsafe {
            let _: () = msg_send![self.web_view,
                takeSnapshotWithConfiguration: NIL
                completionHandler: &*block];
        }
    }

    /// Finds `query` in the page (case insensitive, wrapping), marks and scrolls to the
    /// match after the current one (or before it, `backwards`); `done(found)`.
    pub fn find(&self, query: &str, backwards: bool, done: impl FnOnce(bool) + 'static) {
        self.find_case(query, backwards, false, done);
    }

    /// [`WebPage::find`] with the case to match said: the find strip's "case sensitive".
    pub fn find_case(
        &self,
        query: &str,
        backwards: bool,
        case_sensitive: bool,
        done: impl FnOnce(bool) + 'static,
    ) {
        if query.is_empty() {
            self.clear_find();
            ns::after(std::time::Duration::ZERO, move || done(false));
            return;
        }
        let slot = RefCell::new(Some(done));
        let block = ConcreteBlock::new(move |result: Id| {
            // SAFETY: a WKFindResult or nil.
            let found = unsafe {
                if result.is_null() {
                    false
                } else {
                    let found: BOOL = msg_send![result, matchFound];
                    found != NO
                }
            };
            let done = slot.borrow_mut().take();
            if let Some(done) = done {
                guarded("a find's end", move || done(found));
            }
        })
        .copy();
        // SAFETY: WebKit's own selectors (macOS 11).
        unsafe {
            let configuration: Id = msg_send![class!(WKFindConfiguration), new];
            let _: () = msg_send![configuration, setBackwards: ns::yes_no(backwards)];
            let _: () = msg_send![configuration, setCaseSensitive: ns::yes_no(case_sensitive)];
            let _: () = msg_send![configuration, setWraps: YES];
            let _: () = msg_send![self.web_view,
                findString: nsstring(query)
                withConfiguration: configuration
                completionHandler: &*block];
            release(configuration);
        }
    }

    /// Takes the mark of the last find away (the match WebKit selected).
    pub fn clear_find(&self) {
        // SAFETY: WebKit's own; the script runs in a world of the client, which the
        // page's scripts cannot see.
        unsafe {
            let world: Id = msg_send![class!(WKContentWorld), defaultClientWorld];
            let _: () = msg_send![self.web_view,
                evaluateJavaScript: nsstring("window.getSelection().removeAllRanges()")
                inFrame: NIL
                inContentWorld: world
                completionHandler: std::ptr::null_mut::<c_void>()];
        }
    }

    /// The page's zoom (`pageZoom`; 1.0 is the page's own size).
    pub fn set_zoom(&self, factor: f64) {
        // SAFETY: WebKit's own (macOS 11).
        unsafe {
            let _: () = msg_send![self.web_view, setPageZoom: factor];
        }
    }

    /// The view's appearance (a page that follows `prefers-color-scheme` follows it)
    /// and the colour around and under the page (sRGB, 0 to 1).
    pub fn set_appearance(&self, appearance: Appearance, background: (f64, f64, f64)) {
        // SAFETY: AppKit's and WebKit's own; `setUnderPageBackgroundColor:` (12.0) only
        // where it answers.
        unsafe {
            let named = match appearance {
                Appearance::Light => Some(NSAppearanceNameAqua),
                Appearance::Dark => Some(NSAppearanceNameDarkAqua),
                Appearance::System => None,
            };
            let object: Id = match named {
                Some(name) => msg_send![class!(NSAppearance), appearanceNamed: name],
                None => NIL,
            };
            let _: () = msg_send![self.web_view, setAppearance: object];
            if ns::responds(self.web_view, sel!(setUnderPageBackgroundColor:)) {
                let (r, g, b) = background;
                let color: Id = msg_send![class!(NSColor),
                    colorWithSRGBRed: r
                    green: g
                    blue: b
                    alpha: 1.0f64];
                let _: () = msg_send![self.web_view, setUnderPageBackgroundColor: color];
            }
        }
    }

    /// The Edit menu's command, on the page: its selection, or a field of it.
    pub fn edit(&self, command: EditCommand) {
        // SAFETY: NSResponder's and NSUndoManager's own.
        unsafe {
            let action = match command {
                EditCommand::Copy => sel!(copy:),
                EditCommand::Cut => sel!(cut:),
                EditCommand::Paste => sel!(paste:),
                EditCommand::SelectAll => sel!(selectAll:),
                EditCommand::Undo | EditCommand::Redo => {
                    let manager: Id = msg_send![self.web_view, undoManager];
                    if manager.is_null() {
                        return;
                    }
                    if command == EditCommand::Undo {
                        let can: BOOL = msg_send![manager, canUndo];
                        if can != NO {
                            let _: () = msg_send![manager, undo];
                        }
                    } else {
                        let can: BOOL = msg_send![manager, canRedo];
                        if can != NO {
                            let _: () = msg_send![manager, redo];
                        }
                    }
                    return;
                }
            };
            let _: BOOL = msg_send![self.web_view, tryToPerform: action with: NIL];
        }
    }

    /// Loads the entry again (not the file the reader went to).
    pub fn reload(&self) {
        load_entry(&self.shared);
    }

    /// The view as it is now, printed into a PDF on the user's default paper.
    /// `done` runs on the main thread when the print operation has run.
    ///
    /// Call it outside any GPUI update (the print job's task, between two awaits):
    /// AppKit may call the window's delegate meanwhile.
    pub fn print_to_pdf(&self, done: impl FnOnce(Result<PrintedPdf, String>) + 'static) {
        // SAFETY: AppKit's own; the web view is live.
        unsafe {
            let window: Id = msg_send![self.web_view, window];
            print::print_web_view(self.web_view, window, Box::new(done));
        }
    }

    /// Evaluates `script` in the page's own world; `done` gets the value as text (a
    /// string as it is, a number or a boolean written out, an array or an object as
    /// JSON, "" for nothing), or the error's words.
    pub fn evaluate(&self, script: &str, done: impl FnOnce(Result<String, String>) + 'static) {
        let slot = RefCell::new(Some(done));
        let block = ConcreteBlock::new(move |result: Id, error: Id| {
            let answer = if error.is_null() {
                // SAFETY: what WebKit hands over: a property-list value or nil.
                Ok(unsafe { js_text(result) })
            } else {
                // SAFETY: an NSError.
                Err(unsafe { error_text(error) })
            };
            let done = slot.borrow_mut().take();
            if let Some(done) = done {
                guarded("a script's end", move || done(answer));
            }
        })
        .copy();
        // SAFETY: WebKit's own.
        unsafe {
            let _: () = msg_send![self.web_view,
                evaluateJavaScript: nsstring(script)
                completionHandler: &*block];
        }
    }

    pub fn stats(&self) -> ViewStats {
        ViewStats {
            served: self.shared.log.served.get(),
            missing: self.shared.log.missing_count.get(),
            blocked_navigations: self.shared.blocked_navigations.get(),
            messages_received: self.shared.messages_received.get(),
            messages_dropped: self.shared.messages_dropped.get(),
        }
    }

    /// Runs `body` as an async function in the page world
    /// (`callAsyncJavaScript:arguments:inFrame:inContentWorld:completionHandler:`), with
    /// one argument, `arg`: `argument_json` as a string (WebKit passes values, so nothing
    /// is pasted into the script's source; the body parses it). WebKit awaits the
    /// function's promise. `done` gets the answer as text (a string as it is; anything
    /// else written out as [`WebPage::evaluate`] does), or `Err` with the exception's
    /// message; on the main thread, inside WebKit's block, guarded.
    pub fn call_async(
        &self,
        body: &str,
        argument_json: &str,
        done: impl FnOnce(Result<String, String>) + 'static,
    ) {
        // SAFETY: the live web view, on the main thread.
        unsafe { call_async_in(self.web_view, body, argument_json, Box::new(done)) }
    }

    /// Out of sight, still drawn (a whiteboard's view whose last scene is being
    /// collected while another item opens): its frame moves out of the window's visible
    /// content (x = -100,000, its size kept) and its window occlusion detection is
    /// turned off, so that its timers and animation frames run on. It is not hidden (a
    /// hidden view stops animation frames, throttles timers and tells the page
    /// `visibilitychange`). The first responder goes to GPUI's view, when the web view
    /// has it. [`WebPage::set_frame`] brings it back.
    pub fn park(&self) {
        // SAFETY: AppKit's own selectors on a live view; the private switch only where
        // it answers.
        unsafe {
            if ns::responds(self.web_view, sel!(_setWindowOcclusionDetectionEnabled:)) {
                let _: () = msg_send![self.web_view, _setWindowOcclusionDetectionEnabled: NO];
            }
            let current: NSRect = msg_send![self.web_view, frame];
            let parked = NSRect::new(
                PARKED_X,
                current.origin.y,
                current.size.width,
                current.size.height,
            );
            let _: () = msg_send![self.web_view, setFrame: parked];
            // WebKit reads the switch when it next asks whether the view is visible: in a
            // window that is occluded already, that is now (hidden and shown again in one
            // turn of the run loop, which WebKit hands its page as one change).
            let _: () = msg_send![self.web_view, setHidden: YES];
            let _: () = msg_send![self.web_view, setHidden: NO];
        }
        self.unfocus();
    }

    /// The web view itself (an `NSView *`), for tests that hit-test it or send it
    /// events.
    #[doc(hidden)]
    pub fn native_view(&self) -> *mut c_void {
        self.web_view.cast()
    }
}

/// Loads the page's entry in its web view.
fn load_entry(shared: &ViewShared) {
    let web_view = shared.web_view.get();
    if web_view.is_null() {
        return;
    }
    let url = http::url_of(&shared.host, &shared.entry);
    shared.own_load.set(true);
    // SAFETY: Foundation's and WebKit's own; the navigation is retained while it is
    // the last one.
    unsafe {
        let request = delegate::request_for(&url);
        if request.is_null() {
            shared.events.failed(format!(
                "The page's entry {} cannot be loaded.",
                shared.entry
            ));
            return;
        }
        let navigation: Id = msg_send![web_view, loadRequest: request];
        let previous = shared.navigation.replace(retain(navigation));
        release(previous);
    }
}

/// How many bridge handlers are alive in this process.
pub(crate) fn live_bridges() -> usize {
    bridge::live()
}

/// What a call hands its answer to.
pub(crate) type CallDone = Box<dyn FnOnce(Result<String, String>)>;

/// `callAsyncJavaScript` in the page world of `web_view`'s main frame, `arg` being
/// `argument_json` as a string (see [`WebPage::call_async`]).
pub(crate) unsafe fn call_async_in(web_view: Id, body: &str, argument_json: &str, done: CallDone) {
    let slot = RefCell::new(Some(done));
    let block = ConcreteBlock::new(move |result: Id, error: Id| {
        let answer = if error.is_null() {
            // SAFETY: what WebKit hands over: a property-list value or nil.
            Ok(unsafe { answer_text(result) })
        } else {
            // SAFETY: an NSError.
            Err(unsafe { exception_message(error) })
        };
        let done = slot.borrow_mut().take();
        if let Some(done) = done {
            guarded("a call's end", move || done(answer));
        }
    })
    .copy();
    // SAFETY: WebKit's own (macOS 11), asked for by `check_selectors`; the dictionary
    // and its strings are autoreleased and copied by WebKit.
    unsafe {
        let keys = [nsstring("arg")];
        let values = [nsstring(argument_json)];
        let arguments: Id = msg_send![class!(NSDictionary),
            dictionaryWithObjects: values.as_ptr()
            forKeys: keys.as_ptr()
            count: 1usize];
        let world: Id = msg_send![class!(WKContentWorld), pageWorld];
        let _: () = msg_send![web_view,
            callAsyncJavaScript: nsstring(body)
            arguments: arguments
            inFrame: NIL
            inContentWorld: world
            completionHandler: &*block];
    }
}

/// The answer of a call as text: a string exactly as it is (a NUL in it included),
/// anything else as [`js_text`] writes it.
unsafe fn answer_text(value: Id) -> String {
    // SAFETY: questions every NSObject answers, then the class's own.
    unsafe {
        if is_kind(value, "NSString") {
            return bridge::utf8_of(value, true).unwrap_or_default();
        }
        js_text(value)
    }
}

/// The message of the exception a script threw (`WKJavaScriptExceptionMessage`), else
/// the error's own words.
pub(crate) unsafe fn exception_message(error: Id) -> String {
    // SAFETY: NSError's own selectors; `userInfo` is a dictionary.
    unsafe {
        let info: Id = msg_send![error, userInfo];
        if !info.is_null() {
            let message: Id =
                msg_send![info, objectForKey: nsstring("WKJavaScriptExceptionMessage")];
            let message = string_of(message);
            if !message.is_empty() {
                return message;
            }
        }
        error_text(error)
    }
}

/// A value WebKit hands back from a script, as text.
pub(crate) unsafe fn js_text(value: Id) -> String {
    // SAFETY: questions every NSObject answers, then the class's own.
    unsafe {
        if value.is_null() || is_kind(value, "NSNull") {
            return if value.is_null() {
                String::new()
            } else {
                "null".to_string()
            };
        }
        if is_kind(value, "NSString") {
            return string_of(value);
        }
        if is_boolean(value) {
            let truth: BOOL = msg_send![value, boolValue];
            return if truth != NO { "true" } else { "false" }.to_string();
        }
        if is_kind(value, "NSNumber") {
            let text: Id = msg_send![value, stringValue];
            return string_of(text);
        }
        let valid: BOOL = msg_send![class!(NSJSONSerialization), isValidJSONObject: value];
        if valid != NO {
            let data: Id = msg_send![class!(NSJSONSerialization),
                dataWithJSONObject: value
                options: 0usize
                error: std::ptr::null_mut::<Id>()];
            if !data.is_null() {
                let text: Id = msg_send![class!(NSString), alloc];
                let text: Id = msg_send![text, initWithData: data encoding: 4usize];
                let out = string_of(text);
                release(text);
                return out;
            }
        }
        describe(value)
    }
}

/// Out of its superview; the first responder given back only when `[webView window]`
/// is non-nil and its first responder is the web view; GPUI's view released; released.
impl Drop for WebPage {
    fn drop(&mut self) {
        // SAFETY: the references `new` took, given back once; AppKit's own selectors.
        unsafe {
            self.shared.web_view.set(NIL);
            self.shared.gpui_view.set(NIL);
            let _: () = msg_send![self.web_view, setNavigationDelegate: NIL];
            let _: () = msg_send![self.web_view, setUIDelegate: NIL];
            let _: () = msg_send![self.web_view, stopLoading];
            // The bridge's handler goes with the view (the controller holds it).
            let configuration: Id = msg_send![self.web_view, configuration];
            let controller: Id = msg_send![configuration, userContentController];
            let _: () = msg_send![controller, removeAllScriptMessageHandlers];
            let window: Id = msg_send![self.web_view, window];
            if !window.is_null() {
                let first: Id = msg_send![window, firstResponder];
                if first == self.web_view {
                    let gpui_window: Id = msg_send![self.gpui_view, window];
                    let next = if gpui_window == window {
                        self.gpui_view
                    } else {
                        NIL
                    };
                    self.shared.focusing.set(true);
                    let _: BOOL = msg_send![window, makeFirstResponder: next];
                    self.shared.focusing.set(false);
                }
            }
            let _: () = msg_send![self.web_view, removeFromSuperview];
            view::detach(self.web_view);
            release(self.shared.navigation.replace(NIL));
            release(self.gpui_view);
            release(self.delegate);
            release(self.web_view);
        }
    }
}
