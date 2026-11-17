//! Two fingers pinched on the trackpad zoom the text.
//!
//! GPUI 0.2.2 delivers no magnify event: its window only turns key, mouse
//! and scroll events into its own. AppKit, though, lets an app look at
//! every event it is about to dispatch (`+[NSEvent
//! addLocalMonitorForEventsMatchingMask:handler:]`), and that is all this
//! is: a monitor for magnify events and nothing else, which reads what the
//! pinch says (how much, where) and hands the event on untouched. Scroll
//! events never come near it, so scrolling cannot be disturbed by it.
//!
//! The monitor runs inside AppKit's event dispatch, where GPUI must not be
//! entered. It only puts what it read into a channel; a task of the app
//! takes it from there and calls [`Workspace::pinch`], which zooms the
//! text around the pointer when the pointer is on the text.
//!
//! Only the real app watches ([`watch`], from `main`), and
//! `INNER_WORD_PINCH=off` in its environment keeps it from watching.
//!
//! # What is known about it, and how
//!
//! No script can pinch, so no real magnify event has been read by anything
//! but the fingers of whoever tried. Everything up to that is held:
//!
//! - What is asked of the event is what AppKit declares (`NSEvent.h`,
//!   `NSWindow.h`, `NSView.h` of the macOS SDK): `type` is an
//!   `NSEventType` (an unsigned word, magnify is 30, its mask bit 30 of an
//!   `unsigned long long`), `window` an object that may be nil,
//!   `magnification` a `CGFloat` that is "valid for events of type
//!   NSEventTypeMagnify" (and is asked of nothing else), `locationInWindow`
//!   an `NSPoint`; `windowNumber` is an `NSInteger`, `contentView` an
//!   object that may be nil, `frame` an `NSRect`. A test reads the return
//!   types of these seven methods from the Objective-C runtime of the Mac
//!   it runs on and compares them with the types this file reads
//!   ([`appkit_return_types`]).
//! - How they are read: `objc` 0.2 sends through `objc_msgSend` and, on
//!   x86_64, through `objc_msgSend_stret` for a result larger than sixteen
//!   bytes, which is the `NSRect` and not the `NSPoint`; arm64 has the one
//!   entry point. The block is copied to the heap, AppKit keeps its own
//!   reference for as long as the monitor is installed, and it returns the
//!   event it was given, which is what tells AppKit to go on with it.
//! - The reading itself runs without an event: [`stand_in::Event`] is an
//!   object of a class declared with the Objective-C runtime that answers
//!   the same four messages with the same types. A test hands one to the
//!   block the way AppKit calls a block and sees the zoom follow; a
//!   scripted run (`zoom:pinch:<magnification>`) hands one whose window is
//!   the real `NSWindow` to the very block AppKit holds.

use std::ffi::c_void;
use std::sync::mpsc::{Receiver, channel};

use gpui::{App, Global, Pixels, Point, Window, WindowHandle, point, px};

use crate::workspace::Workspace;

/// What a magnify event says.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pinch {
    /// The window the fingers were over, by its number.
    pub window: isize,
    /// How much the pinch asks for since the last event: the change of
    /// magnification, a small number, positive when the fingers part.
    pub magnification: f32,
    /// Where the pointer is, in the window's own coordinates: from its
    /// left edge and from the top of its content.
    pub x: f32,
    /// See [`Pinch::x`].
    pub y: f32,
}

impl Pinch {
    /// The factor a magnification stands for: what the zoom is multiplied
    /// by. One event never more than doubles or halves it.
    pub fn factor(magnification: f32) -> f32 {
        if !magnification.is_finite() {
            return 1.;
        }
        (1. + magnification).clamp(0.5, 2.)
    }
}

/// Several events that arrived together are one pinch: the product of
/// their factors, at the place of the last one. `None` for no events.
pub fn combined(pinches: &[Pinch]) -> Option<(f32, f32, f32)> {
    let last = pinches.last()?;
    let factor: f32 = pinches
        .iter()
        .map(|pinch| Pinch::factor(pinch.magnification))
        .product();
    Some((factor, last.x, last.y))
}

/// The monitor: the block AppKit calls with every magnify event it is
/// about to dispatch. It reads the event into the channel it was made
/// with and hands the event back.
pub struct Monitor {
    handler: native::Handler,
}

impl Monitor {
    /// A monitor AppKit does not have yet, and where its pinches come out.
    pub fn new() -> (Monitor, Receiver<Pinch>) {
        let (sender, receiver) = channel::<Pinch>();
        let monitor = Monitor {
            handler: native::handler(sender),
        };
        (monitor, receiver)
    }

    /// Gives the block to AppKit, which calls it from then on until the
    /// app quits. Returns whether AppKit took it. On the main thread.
    fn install(&self) -> bool {
        native::install(&self.handler)
    }

    /// Calls the block the way AppKit calls it, with `event`, and returns
    /// what the block hands back: the event AppKit would go on with.
    ///
    /// # Safety
    ///
    /// `event` is null or a live Objective-C object that answers what is
    /// asked of an `NSEvent` here (`type`, and for a magnify event
    /// `window`, `magnification` and `locationInWindow`), with the types
    /// `NSEvent` answers with. On the main thread.
    pub unsafe fn hand(&self, event: *mut c_void) -> *mut c_void {
        // SAFETY: the caller's.
        unsafe { native::call(&self.handler, event) }
    }
}

/// The monitor AppKit has: kept so that a scripted run can hand the same
/// block a stand-in for an event ([`script`]).
struct Installed {
    monitor: Monitor,
}

impl Global for Installed {}

/// Whether the environment asks for no pinch: `INNER_WORD_PINCH=off` (or
/// `0`, `no`, `false`).
fn switched_off() -> bool {
    std::env::var("INNER_WORD_PINCH").is_ok_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "off" | "0" | "no" | "false"
        )
    })
}

/// Has pinches on the trackpad over `window` zoom its text, from now
/// until the app quits. Called by `main` once the main window is open.
pub fn watch(window: WindowHandle<Workspace>, cx: &mut App) {
    if switched_off() {
        return;
    }
    let number = window
        .update(cx, |_, window, _| crate::screenshot::window_number(window))
        .ok()
        .and_then(Result::ok);
    let Some(number) = number else {
        // No native window to watch (it is not on screen): no pinch.
        return;
    };
    let (monitor, receiver) = Monitor::new();
    if !monitor.install() {
        return;
    }
    follow(window, number as isize, receiver, cx);
    cx.set_global(Installed { monitor });
}

/// Takes the pinches a monitor read from `receiver` and zooms the text of
/// `window` with those that are over the window numbered `number`, until
/// the monitor is gone or the window is.
pub fn follow(
    window: WindowHandle<Workspace>,
    number: isize,
    receiver: Receiver<Pinch>,
    cx: &mut App,
) {
    cx.spawn(async move |cx| {
        let mut receiver = receiver;
        loop {
            // Waiting for the next pinch occupies a background thread, as
            // waiting for a signal does (`crate::signals`).
            let (pinches, back) = cx
                .background_executor()
                .spawn(async move { next(receiver) })
                .await;
            let Some(pinches) = pinches else {
                break;
            };
            receiver = back;
            let here: Vec<Pinch> = pinches
                .into_iter()
                .filter(|pinch| pinch.window == number)
                .collect();
            let Some((factor, x, y)) = combined(&here) else {
                continue;
            };
            let alive = window.update(cx, |workspace, _, cx| {
                workspace.pinch(factor, point(px(x), px(y)), cx);
            });
            if alive.is_err() {
                // The window is gone.
                break;
            }
        }
    })
    .detach();
}

/// Blocks until a pinch arrives, and takes every other one that is
/// already waiting with it. `None` when nobody will send any more.
fn next(receiver: Receiver<Pinch>) -> (Option<Vec<Pinch>>, Receiver<Pinch>) {
    let Ok(first) = receiver.recv() else {
        return (None, receiver);
    };
    let mut pinches = vec![first];
    while let Ok(more) = receiver.try_recv() {
        pinches.push(more);
    }
    (Some(pinches), receiver)
}

/// A scripted run's pinch (`zoom:pinch:<magnification>`): the block AppKit
/// holds is handed a stand-in for a magnify event over the real window,
/// with the pointer at `at` (from the left edge and the top of the
/// content, as the app counts). The zoom follows a moment later, the way
/// it does for fingers. Fails when the app is not watching for pinches.
pub fn script(
    magnification: f32,
    at: Point<Pixels>,
    window: &Window,
    cx: &App,
) -> anyhow::Result<()> {
    let Some(installed) = cx.try_global::<Installed>() else {
        anyhow::bail!("the app is not watching for pinches");
    };
    let view = crate::screenshot::native_view(window)?;
    let event = stand_in::Event::over_the_window_of(
        view,
        stand_in::MAGNIFY,
        f64::from(magnification),
        f64::from(f32::from(at.x)),
        f64::from(f32::from(at.y)),
    )?;
    // SAFETY: the stand-in answers what the block asks of an event, with
    // the types an `NSEvent` answers with; it is alive until the end of
    // this function, and a scripted step runs on the main thread.
    let back = unsafe { installed.monitor.hand(event.as_ptr()) };
    if back != event.as_ptr() {
        anyhow::bail!("the monitor did not hand the event on");
    }
    Ok(())
}

/// The return types AppKit declares for what the monitor asks, as the
/// Objective-C runtime of this Mac has them, each beside the type this
/// file reads the answer as: `(the method, what the runtime says, what is
/// read)`. They are equal on a Mac this code is right for; a test says so.
pub fn appkit_return_types() -> Vec<(&'static str, String, &'static str)> {
    native::appkit_return_types()
}

#[cfg(target_os = "macos")]
// objc 0.2's msg_send! expands to a `cfg(feature = "cargo-clippy")` check.
#[allow(unexpected_cfgs)]
mod native {
    use std::ffi::c_void;
    use std::sync::mpsc::Sender;

    use block::{ConcreteBlock, RcBlock};
    use objc::{
        Encode, Encoding, class, msg_send,
        runtime::{Class, Object, Sel},
        sel, sel_impl,
    };

    use super::Pinch;

    /// `NSEventTypeMagnify`.
    pub(super) const MAGNIFY: usize = 30;
    /// `NSEventMaskMagnify`.
    const MAGNIFY_MASK: u64 = 1 << MAGNIFY;

    /// `NSPoint`.
    #[repr(C)]
    #[derive(Clone, Copy)]
    pub(super) struct NsPoint {
        pub(super) x: f64,
        pub(super) y: f64,
    }

    /// `NSSize`.
    #[repr(C)]
    #[derive(Clone, Copy)]
    pub(super) struct NsSize {
        pub(super) width: f64,
        pub(super) height: f64,
    }

    /// `NSRect`.
    #[repr(C)]
    #[derive(Clone, Copy)]
    pub(super) struct NsRect {
        pub(super) origin: NsPoint,
        pub(super) size: NsSize,
    }

    /// What the runtime says an `NSPoint` is.
    pub(super) const POINT: &str = "{CGPoint=dd}";
    /// What the runtime says an `NSRect` is.
    pub(super) const RECT: &str = "{CGRect={CGPoint=dd}{CGSize=dd}}";

    // SAFETY: the encodings of the two structs as the Objective-C runtime
    // writes them on a 64-bit Mac (`CGFloat` is a double there).
    unsafe impl Encode for NsPoint {
        fn encode() -> Encoding {
            unsafe { Encoding::from_str(POINT) }
        }
    }

    // SAFETY: as above.
    unsafe impl Encode for NsRect {
        fn encode() -> Encoding {
            unsafe { Encoding::from_str(RECT) }
        }
    }

    /// The block AppKit calls: `NSEvent *(^)(NSEvent *)`.
    pub(super) type Handler = RcBlock<(*mut Object,), *mut Object>;

    /// The block: every magnify event it is called with is read into
    /// `sender`, and whatever it is called with is handed back unchanged.
    pub(super) fn handler(sender: Sender<Pinch>) -> Handler {
        let block = ConcreteBlock::new(move |event: *mut Object| -> *mut Object {
            if let Some(pinch) = read(event) {
                // Nobody listening any more is fine.
                let _ = sender.send(pinch);
            }
            event
        });
        // Copied to the heap, where it lives while anything refers to it.
        block.copy()
    }

    /// Installs the monitor with `handler`. Returns whether AppKit took
    /// it. AppKit copies the block it is given, which for one on the heap
    /// is one more reference: the monitor keeps the block alive by itself.
    pub(super) fn install(handler: &Handler) -> bool {
        // SAFETY: a class method of AppKit, called on the main thread with
        // a mask of its type (`NSEventMask`, an `unsigned long long`) and
        // a block of the type it documents (`NSEvent *(^)(NSEvent *)`).
        let monitor: *mut Object = unsafe {
            msg_send![class!(NSEvent), addLocalMonitorForEventsMatchingMask: MAGNIFY_MASK
                                                                    handler: &**handler]
        };
        !monitor.is_null()
    }

    /// Calls the block the way AppKit does.
    ///
    /// # Safety
    ///
    /// See [`super::Monitor::hand`].
    pub(super) unsafe fn call(handler: &Handler, event: *mut c_void) -> *mut c_void {
        // SAFETY: the block takes one object and returns one; what it does
        // with the object is the caller's promise.
        let back = unsafe { handler.call((event as *mut Object,)) };
        back as *mut c_void
    }

    /// What a magnify event says. `None` for anything else, and for an
    /// event that has no window.
    fn read(event: *mut Object) -> Option<Pinch> {
        if event.is_null() {
            return None;
        }
        // SAFETY: `event` is the live `NSEvent` AppKit handed the monitor,
        // on the main thread; these are its plain getters. `magnification`
        // is only asked of a magnify event (it raises for others).
        unsafe {
            let kind: usize = msg_send![event, type];
            if kind != MAGNIFY {
                return None;
            }
            let window: *mut Object = msg_send![event, window];
            if window.is_null() {
                return None;
            }
            let number: isize = msg_send![window, windowNumber];
            let magnification: f64 = msg_send![event, magnification];
            let location: NsPoint = msg_send![event, locationInWindow];
            let view: *mut Object = msg_send![window, contentView];
            if view.is_null() {
                return None;
            }
            let frame: NsRect = msg_send![view, frame];
            Some(Pinch {
                window: number,
                magnification: magnification as f32,
                x: location.x as f32,
                // AppKit counts from the bottom, the app from the top.
                y: (frame.size.height - location.y) as f32,
            })
        }
    }

    /// See [`super::appkit_return_types`]. The second of each pair is the
    /// encoding of the Rust type [`read`] takes the answer as.
    pub(super) fn appkit_return_types() -> Vec<(&'static str, String, &'static str)> {
        let returns = |class: &Class, selector: Sel| -> String {
            class
                .instance_method(selector)
                .map(|method| method.return_type().as_str().to_string())
                .unwrap_or_else(|| "no such method".to_string())
        };
        let (event, window, view) = (class!(NSEvent), class!(NSWindow), class!(NSView));
        vec![
            // usize
            ("-[NSEvent type]", returns(event, sel!(type)), "Q"),
            // *mut Object
            ("-[NSEvent window]", returns(event, sel!(window)), "@"),
            // f64
            (
                "-[NSEvent magnification]",
                returns(event, sel!(magnification)),
                "d",
            ),
            // NsPoint
            (
                "-[NSEvent locationInWindow]",
                returns(event, sel!(locationInWindow)),
                POINT,
            ),
            // isize
            (
                "-[NSWindow windowNumber]",
                returns(window, sel!(windowNumber)),
                "q",
            ),
            // *mut Object
            (
                "-[NSWindow contentView]",
                returns(window, sel!(contentView)),
                "@",
            ),
            // NsRect
            ("-[NSView frame]", returns(view, sel!(frame)), RECT),
        ]
    }
}

#[cfg(not(target_os = "macos"))]
mod native {
    use std::ffi::c_void;
    use std::sync::mpsc::Sender;

    use super::Pinch;

    pub(super) const MAGNIFY: usize = 30;

    pub(super) struct Handler;

    pub(super) fn handler(_: Sender<Pinch>) -> Handler {
        Handler
    }

    pub(super) fn install(_: &Handler) -> bool {
        false
    }

    pub(super) unsafe fn call(_: &Handler, event: *mut c_void) -> *mut c_void {
        event
    }

    pub(super) fn appkit_return_types() -> Vec<(&'static str, String, &'static str)> {
        Vec::new()
    }
}

/// Stand-ins for what AppKit hands the monitor, for trying the monitor
/// without a pinch: objects of classes declared with the Objective-C
/// runtime that answer the messages the monitor sends, with the types
/// `NSEvent`, `NSWindow` and `NSView` answer with, and with values the
/// caller chose. Nothing here is an event: nothing is posted, and AppKit
/// never sees these objects.
#[cfg(target_os = "macos")]
// objc 0.2's msg_send! expands to a `cfg(feature = "cargo-clippy")` check.
#[allow(unexpected_cfgs)]
pub mod stand_in {
    use std::ffi::c_void;
    use std::sync::Once;

    use objc::{
        class,
        declare::ClassDecl,
        msg_send,
        runtime::{Class, Object, Sel},
        sel, sel_impl,
    };

    use super::native::{NsPoint, NsRect, NsSize};
    use crate::screenshot::NativeView;

    /// `NSEventTypeMagnify`.
    pub const MAGNIFY: usize = super::native::MAGNIFY;
    /// `NSEventTypeScrollWheel`, an event that is not a pinch.
    pub const SCROLL_WHEEL: usize = 22;

    const EVENT_CLASS: &str = "InnerWordStandInEvent";
    const WINDOW_CLASS: &str = "InnerWordStandInWindow";
    const VIEW_CLASS: &str = "InnerWordStandInView";

    /// A stand-in for an `NSEvent`. It answers `type`, `window`,
    /// `locationInWindow` and, like an `NSEvent`, `magnification` only
    /// when it is a magnify event: asked of any other, the process stops,
    /// as it would with AppKit's exception.
    pub struct Event {
        event: *mut Object,
        /// The made-up window and its view, where the window is not a
        /// real one.
        owned: Vec<*mut Object>,
    }

    impl Event {
        /// An event of type `kind` over a made-up window with the number
        /// `window_number`, whose content is `content_height` points
        /// high; the pointer `x` from the left and `y_from_bottom` from
        /// the bottom of the content, as AppKit counts.
        pub fn over_a_made_up_window(
            kind: usize,
            magnification: f64,
            x: f64,
            y_from_bottom: f64,
            window_number: isize,
            content_height: f64,
        ) -> Event {
            // SAFETY: objects of classes declared below, made and filled
            // in the way the runtime documents.
            unsafe {
                let view: *mut Object = msg_send![view_class(), new];
                (*view).set_ivar::<f64>("width", 1000.);
                (*view).set_ivar::<f64>("height", content_height);
                let window: *mut Object = msg_send![window_class(), new];
                (*window).set_ivar::<isize>("number", window_number);
                (*window).set_ivar::<usize>("view", view as usize);
                let mut event = Event::new(kind, magnification, x, y_from_bottom, window);
                event.owned = vec![window, view];
                event
            }
        }

        /// An event of type `kind` with no window at all.
        pub fn without_a_window(kind: usize, magnification: f64) -> Event {
            // SAFETY: as above.
            unsafe { Event::new(kind, magnification, 0., 0., std::ptr::null_mut()) }
        }

        /// An event of type `kind` over the real window `view` is in,
        /// with the pointer `x` from the left and `y` from the top of its
        /// content, as the app counts.
        pub fn over_the_window_of(
            view: NativeView,
            kind: usize,
            magnification: f64,
            x: f64,
            y: f64,
        ) -> anyhow::Result<Event> {
            let view = view.as_ptr() as *mut Object;
            // SAFETY: `view` is the live `NSView` of a GPUI window, on the
            // main thread; `window`, `contentView` and `frame` are plain
            // getters of AppKit.
            unsafe {
                let window: *mut Object = msg_send![view, window];
                if window.is_null() {
                    anyhow::bail!("the view is not in a window");
                }
                let content: *mut Object = msg_send![window, contentView];
                if content.is_null() {
                    anyhow::bail!("the window has no content view");
                }
                let frame: NsRect = msg_send![content, frame];
                Ok(Event::new(
                    kind,
                    magnification,
                    x,
                    frame.size.height - y,
                    window,
                ))
            }
        }

        /// # Safety
        ///
        /// `window` is null or an object that answers `windowNumber` and
        /// `contentView` as an `NSWindow` does, alive as long as the
        /// event is.
        unsafe fn new(
            kind: usize,
            magnification: f64,
            x: f64,
            y_from_bottom: f64,
            window: *mut Object,
        ) -> Event {
            // SAFETY: a new object of the class declared below.
            unsafe {
                let event: *mut Object = msg_send![event_class(), new];
                (*event).set_ivar::<usize>("kind", kind);
                (*event).set_ivar::<f64>("magnification", magnification);
                (*event).set_ivar::<f64>("x", x);
                (*event).set_ivar::<f64>("y", y_from_bottom);
                (*event).set_ivar::<usize>("window", window as usize);
                Event {
                    event,
                    owned: Vec::new(),
                }
            }
        }

        /// The object, to hand to [`super::Monitor::hand`].
        pub fn as_ptr(&self) -> *mut c_void {
            self.event as *mut c_void
        }
    }

    impl Drop for Event {
        fn drop(&mut self) {
            // SAFETY: each was made with `new` here and is released once.
            unsafe {
                let _: () = msg_send![self.event, release];
                for object in self.owned.drain(..) {
                    let _: () = msg_send![object, release];
                }
            }
        }
    }

    /// The return types of what the stand-ins answer, in the order and
    /// the form of [`super::appkit_return_types`]: `(the method, what the
    /// runtime says)`.
    pub fn return_types() -> Vec<(&'static str, String)> {
        let returns = |class: &Class, selector: Sel| -> String {
            class
                .instance_method(selector)
                .map(|method| method.return_type().as_str().to_string())
                .unwrap_or_else(|| "no such method".to_string())
        };
        let (event, window, view) = (event_class(), window_class(), view_class());
        vec![
            ("type", returns(event, sel!(type))),
            ("window", returns(event, sel!(window))),
            ("magnification", returns(event, sel!(magnification))),
            ("locationInWindow", returns(event, sel!(locationInWindow))),
            ("windowNumber", returns(window, sel!(windowNumber))),
            ("contentView", returns(window, sel!(contentView))),
            ("frame", returns(view, sel!(frame))),
        ]
    }

    extern "C" fn event_type(this: &Object, _: Sel) -> usize {
        // SAFETY: an ivar of this class, of this type.
        unsafe { *this.get_ivar::<usize>("kind") }
    }

    extern "C" fn event_window(this: &Object, _: Sel) -> *mut Object {
        // SAFETY: as above.
        unsafe { *this.get_ivar::<usize>("window") as *mut Object }
    }

    extern "C" fn event_magnification(this: &Object, _: Sel) -> f64 {
        // SAFETY: as above.
        unsafe {
            if *this.get_ivar::<usize>("kind") != MAGNIFY {
                // An `NSEvent` raises here, and an exception that crosses
                // the block ends the app. So does this.
                eprintln!("magnification was asked of an event that is not a magnify event");
                std::process::abort();
            }
            *this.get_ivar::<f64>("magnification")
        }
    }

    extern "C" fn event_location(this: &Object, _: Sel) -> NsPoint {
        // SAFETY: as above.
        unsafe {
            NsPoint {
                x: *this.get_ivar::<f64>("x"),
                y: *this.get_ivar::<f64>("y"),
            }
        }
    }

    extern "C" fn window_number(this: &Object, _: Sel) -> isize {
        // SAFETY: as above.
        unsafe { *this.get_ivar::<isize>("number") }
    }

    extern "C" fn window_content_view(this: &Object, _: Sel) -> *mut Object {
        // SAFETY: as above.
        unsafe { *this.get_ivar::<usize>("view") as *mut Object }
    }

    extern "C" fn view_frame(this: &Object, _: Sel) -> NsRect {
        // SAFETY: as above.
        unsafe {
            NsRect {
                origin: NsPoint { x: 0., y: 0. },
                size: NsSize {
                    width: *this.get_ivar::<f64>("width"),
                    height: *this.get_ivar::<f64>("height"),
                },
            }
        }
    }

    fn event_class() -> &'static Class {
        static DECLARED: Once = Once::new();
        DECLARED.call_once(|| {
            let mut class =
                ClassDecl::new(EVENT_CLASS, class!(NSObject)).expect("the class is declared once");
            class.add_ivar::<usize>("kind");
            class.add_ivar::<f64>("magnification");
            class.add_ivar::<f64>("x");
            class.add_ivar::<f64>("y");
            class.add_ivar::<usize>("window");
            // SAFETY: each function has the signature of its method.
            unsafe {
                class.add_method(
                    sel!(type),
                    event_type as extern "C" fn(&Object, Sel) -> usize,
                );
                class.add_method(
                    sel!(window),
                    event_window as extern "C" fn(&Object, Sel) -> *mut Object,
                );
                class.add_method(
                    sel!(magnification),
                    event_magnification as extern "C" fn(&Object, Sel) -> f64,
                );
                class.add_method(
                    sel!(locationInWindow),
                    event_location as extern "C" fn(&Object, Sel) -> NsPoint,
                );
            }
            class.register();
        });
        Class::get(EVENT_CLASS).expect("the class was declared")
    }

    fn window_class() -> &'static Class {
        static DECLARED: Once = Once::new();
        DECLARED.call_once(|| {
            let mut class =
                ClassDecl::new(WINDOW_CLASS, class!(NSObject)).expect("the class is declared once");
            class.add_ivar::<isize>("number");
            class.add_ivar::<usize>("view");
            // SAFETY: each function has the signature of its method.
            unsafe {
                class.add_method(
                    sel!(windowNumber),
                    window_number as extern "C" fn(&Object, Sel) -> isize,
                );
                class.add_method(
                    sel!(contentView),
                    window_content_view as extern "C" fn(&Object, Sel) -> *mut Object,
                );
            }
            class.register();
        });
        Class::get(WINDOW_CLASS).expect("the class was declared")
    }

    fn view_class() -> &'static Class {
        static DECLARED: Once = Once::new();
        DECLARED.call_once(|| {
            let mut class =
                ClassDecl::new(VIEW_CLASS, class!(NSObject)).expect("the class is declared once");
            class.add_ivar::<f64>("width");
            class.add_ivar::<f64>("height");
            // SAFETY: the function has the signature of the method.
            unsafe {
                class.add_method(
                    sel!(frame),
                    view_frame as extern "C" fn(&Object, Sel) -> NsRect,
                );
            }
            class.register();
        });
        Class::get(VIEW_CLASS).expect("the class was declared")
    }
}

/// Without AppKit there is nothing to stand in for.
#[cfg(not(target_os = "macos"))]
pub mod stand_in {
    use std::ffi::c_void;

    use crate::screenshot::NativeView;

    /// `NSEventTypeMagnify`.
    pub const MAGNIFY: usize = super::native::MAGNIFY;

    /// Nothing: there is no AppKit here.
    pub struct Event;

    impl Event {
        /// Always an error here.
        pub fn over_the_window_of(
            _: NativeView,
            _: usize,
            _: f64,
            _: f64,
            _: f64,
        ) -> anyhow::Result<Event> {
            anyhow::bail!("pinches are only known on macOS")
        }

        /// A null pointer.
        pub fn as_ptr(&self) -> *mut c_void {
            std::ptr::null_mut()
        }
    }
}
