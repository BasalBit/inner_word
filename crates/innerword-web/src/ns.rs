//! The Objective-C runtime as this crate uses it: strings, geometry, timers on the
//! main run loop, and running that loop in a program.
//!
//! Structs that come back through `msg_send!` here are `NSPoint` (two doubles: an
//! ordinary return everywhere) and `NSRect` (four: `objc` 0.2 takes
//! `objc_msgSend_stret` for it on x86_64, which is what that platform wants).

use std::{
    cell::RefCell,
    ffi::{CStr, c_void},
    os::raw::c_char,
    panic::{AssertUnwindSafe, catch_unwind},
    time::{Duration, Instant},
};

use block::ConcreteBlock;
use objc::{
    Encode, Encoding, class, msg_send,
    rc::autoreleasepool,
    runtime::{BOOL, NO, Object, YES},
    sel, sel_impl,
};

pub(crate) type Id = *mut Object;
pub(crate) const NIL: Id = std::ptr::null_mut();

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct NSPoint {
    pub x: f64,
    pub y: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct NSSize {
    pub width: f64,
    pub height: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct NSRect {
    pub origin: NSPoint,
    pub size: NSSize,
}

impl NSRect {
    pub(crate) fn new(x: f64, y: f64, width: f64, height: f64) -> NSRect {
        NSRect {
            origin: NSPoint { x, y },
            size: NSSize { width, height },
        }
    }
}

// SAFETY: the type encodings of these structs on 64-bit macOS.
unsafe impl Encode for NSPoint {
    fn encode() -> Encoding {
        unsafe { Encoding::from_str("{CGPoint=dd}") }
    }
}
unsafe impl Encode for NSSize {
    fn encode() -> Encoding {
        unsafe { Encoding::from_str("{CGSize=dd}") }
    }
}
unsafe impl Encode for NSRect {
    fn encode() -> Encoding {
        unsafe { Encoding::from_str("{CGRect={CGPoint=dd}{CGSize=dd}}") }
    }
}

/// `NSOperatingSystemVersion`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct OsVersion {
    pub major: isize,
    pub minor: isize,
    pub patch: isize,
}

// SAFETY: three NSIntegers, as Foundation declares the struct.
unsafe impl Encode for OsVersion {
    fn encode() -> Encoding {
        unsafe { Encoding::from_str("{_NSOperatingSystemVersion=qqq}") }
    }
}

#[link(name = "Foundation", kind = "framework")]
unsafe extern "C" {
    pub(crate) static NSRunLoopCommonModes: Id;
    pub(crate) fn NSStringFromSelector(selector: *const c_void) -> Id;
}

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {
    pub(crate) static NSAppearanceNameAqua: Id;
    pub(crate) static NSAppearanceNameDarkAqua: Id;
    /// `NSPrintJobDispositionValue`: the job is saved to a file.
    pub(crate) static NSPrintSaveJob: Id;
    /// `NSPrintInfoAttributeKey`: the URL a save job writes to.
    pub(crate) static NSPrintJobSavingURL: Id;
}

#[link(name = "WebKit", kind = "framework")]
unsafe extern "C" {}

#[link(name = "QuartzCore", kind = "framework")]
unsafe extern "C" {
    pub(crate) static kCAFillRuleEvenOdd: Id;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFRunLoopDefaultMode: *const c_void;
    fn CFRunLoopRunInMode(mode: *const c_void, seconds: f64, return_after_source: u8) -> i32;
    fn CFGetTypeID(object: *const c_void) -> usize;
    fn CFBooleanGetTypeID() -> usize;
}

/// `text` as an autoreleased `NSString`.
pub(crate) fn nsstring(text: &str) -> Id {
    // SAFETY: Foundation's own initializer with the types it declares; the bytes are
    // copied before this returns.
    unsafe {
        let object: Id = msg_send![class!(NSString), alloc];
        let object: Id = msg_send![object,
            initWithBytes: text.as_ptr() as *const c_void
            length: text.len()
            encoding: 4usize];
        if object.is_null() {
            return msg_send![class!(NSString), string];
        }
        msg_send![object, autorelease]
    }
}

/// Whether `object` is of the class named (or a subclass).
pub(crate) unsafe fn is_kind(object: Id, class_name: &str) -> bool {
    if object.is_null() {
        return false;
    }
    let Some(class) = objc::runtime::Class::get(class_name) else {
        return false;
    };
    // SAFETY: NSObject's question, on a live object.
    let answer: BOOL = unsafe { msg_send![object, isKindOfClass: class] };
    answer != NO
}

/// An `NSString` as a Rust string ("" for nil, and for an object that is no string).
pub(crate) unsafe fn string_of(object: Id) -> String {
    // SAFETY: asked only of a string.
    unsafe {
        if !is_kind(object, "NSString") {
            return String::new();
        }
        let utf8: *const c_char = msg_send![object, UTF8String];
        if utf8.is_null() {
            String::new()
        } else {
            CStr::from_ptr(utf8).to_string_lossy().into_owned()
        }
    }
}

/// What an object says of itself (`description`); "nil" for nil.
pub(crate) unsafe fn describe(object: Id) -> String {
    if object.is_null() {
        return "nil".to_string();
    }
    // SAFETY: NSObject's own; the answer is a string.
    unsafe {
        let description: Id = msg_send![object, description];
        string_of(description)
    }
}

/// The words of an `NSError`: its localized description, and the message of a
/// JavaScript exception where WebKit gives one.
pub(crate) unsafe fn error_text(error: Id) -> String {
    if error.is_null() {
        return "an unknown error".to_string();
    }
    // SAFETY: NSError's own selectors; `userInfo` is a dictionary.
    unsafe {
        let description: Id = msg_send![error, localizedDescription];
        let mut text = string_of(description);
        let info: Id = msg_send![error, userInfo];
        if !info.is_null() {
            let message: Id =
                msg_send![info, objectForKey: nsstring("WKJavaScriptExceptionMessage")];
            let message = string_of(message);
            if !message.is_empty() && !text.contains(&message) {
                text = format!("{text} ({message})");
            }
        }
        text
    }
}

/// The domain and code of an `NSError`.
pub(crate) unsafe fn error_code(error: Id) -> (String, isize) {
    if error.is_null() {
        return (String::new(), 0);
    }
    // SAFETY: NSError's own selectors.
    unsafe {
        let domain: Id = msg_send![error, domain];
        let code: isize = msg_send![error, code];
        (string_of(domain), code)
    }
}

/// Whether `object` answers `selector`.
pub(crate) unsafe fn responds(object: Id, selector: objc::runtime::Sel) -> bool {
    if object.is_null() {
        return false;
    }
    // SAFETY: NSObject's question.
    let answer: BOOL = unsafe { msg_send![object, respondsToSelector: selector] };
    answer != NO
}

pub(crate) unsafe fn retain(object: Id) -> Id {
    if object.is_null() {
        return object;
    }
    // SAFETY: a live object.
    unsafe { msg_send![object, retain] }
}

pub(crate) unsafe fn release(object: Id) {
    if !object.is_null() {
        // SAFETY: a reference the caller holds.
        unsafe {
            let _: () = msg_send![object, release];
        }
    }
}

pub(crate) fn yes_no(value: bool) -> BOOL {
    if value { YES } else { NO }
}

pub(crate) fn on_main_thread() -> bool {
    // SAFETY: a class method of Foundation that takes nothing.
    let main: BOOL = unsafe { msg_send![class!(NSThread), isMainThread] };
    main != NO
}

/// The system's uptime in seconds: the clock of `NSEvent.timestamp`.
pub(crate) fn uptime() -> f64 {
    // SAFETY: Foundation's own.
    unsafe {
        let info: Id = msg_send![class!(NSProcessInfo), processInfo];
        msg_send![info, systemUptime]
    }
}

/// Whether the object is a CoreFoundation boolean (`@YES` and `@NO` are).
pub(crate) unsafe fn is_boolean(object: Id) -> bool {
    // SAFETY: CoreFoundation's own functions on a live object.
    !object.is_null() && unsafe { CFGetTypeID(object as *const c_void) == CFBooleanGetTypeID() }
}

/// Runs `work`, and keeps a panic from travelling into Objective-C.
pub(crate) fn guarded(what: &str, work: impl FnOnce()) {
    if catch_unwind(AssertUnwindSafe(work)).is_err() {
        eprintln!("innerword-web: {what} panicked");
    }
}

/// A repeating timer of the main run loop; stops when dropped.
pub(crate) struct Timer(Id);

impl Timer {
    pub(crate) fn stop(&self) {
        // SAFETY: NSTimer's own; a timer may be invalidated more than once.
        unsafe {
            let _: () = msg_send![self.0, invalidate];
        }
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        self.stop();
        // SAFETY: the reference `every` took.
        unsafe { release(self.0) };
    }
}

/// An `NSTimer` with a block, added to the main run loop in its common modes (so that
/// it fires while a menu is tracked or a modal print operation runs too).
fn timer(interval: Duration, repeats: bool, block: &block::Block<(Id,), ()>) -> Id {
    // SAFETY: Foundation's own selectors with the types they declare; the timer
    // copies the block, the run loop holds the timer.
    unsafe {
        let timer: Id = msg_send![class!(NSTimer),
            timerWithTimeInterval: interval.as_secs_f64()
            repeats: yes_no(repeats)
            block: block];
        let run_loop: Id = msg_send![class!(NSRunLoop), mainRunLoop];
        let _: () = msg_send![run_loop, addTimer: timer forMode: NSRunLoopCommonModes];
        timer
    }
}

/// Runs `work` once on the main run loop after `delay`: outside the caller's stack,
/// and outside any borrow the caller holds.
pub(crate) fn after(delay: Duration, work: impl FnOnce() + 'static) {
    let slot = RefCell::new(Some(work));
    let block = ConcreteBlock::new(move |_timer: Id| {
        let work = slot.borrow_mut().take();
        if let Some(work) = work {
            guarded("a timer", work);
        }
    })
    .copy();
    timer(delay, false, &block);
}

/// Runs `work` on the main run loop every `interval` until the timer is dropped.
pub(crate) fn every(interval: Duration, work: impl Fn() + 'static) -> Timer {
    let block = ConcreteBlock::new(move |_timer: Id| guarded("a timer", &work)).copy();
    let timer = timer(interval, true, &block);
    // SAFETY: a live timer; the reference is the Timer's.
    Timer(unsafe { retain(timer) })
}

/// Runs the main run loop (of a program: the test, the tool's child) until `done`
/// answers true or `limit` has passed; true when `done` did.
pub(crate) fn run_until(limit: Duration, done: impl Fn() -> bool) -> bool {
    let start = Instant::now();
    loop {
        if done() {
            return true;
        }
        if start.elapsed() >= limit {
            return false;
        }
        let finished = autoreleasepool(|| {
            // SAFETY: CoreFoundation's own; runs the current thread's run loop.
            unsafe { CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.01, 1) }
        });
        // `kCFRunLoopRunFinished`: nothing to wait for in that mode yet.
        if finished == 1 {
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

/// The application object of a program, with the activation policy "prohibited" (no
/// Dock icon, no menu bar, never active). An application that runs already (the app's
/// own, under GPUI) is left as it is.
pub(crate) fn program_application() {
    // SAFETY: AppKit's own selectors, on the main thread.
    unsafe {
        let app: Id = msg_send![class!(NSApplication), sharedApplication];
        let running: BOOL = msg_send![app, isRunning];
        if running == NO {
            // NSApplicationActivationPolicyProhibited
            let _: BOOL = msg_send![app, setActivationPolicy: 2isize];
        }
    }
}
