//! The one script message handler of a whiteboard's view (`docs/whiteboard-design.md`,
//! section 4.2, and threat 2 of section 8: "the bridge as a door into the app").
//!
//! It is added to the view's user content controller in the page world before the web
//! view is made, under the bridge's name. Every message is checked, in this order, and
//! dropped and counted when a check fails: the name; the main frame; the main frame's
//! origin is `innerword-page://<the view's own host>`; the body is a string of at most
//! `max_bytes` UTF-8 bytes. What passes reaches `PageEvents::message`, which only posts.
//!
//! **A body is never refused for a lone surrogate** (half of a UTF-16 pair, which UTF-8
//! cannot hold): it becomes U+FFFD, as the glue makes every message well formed before it
//! posts and `innerword_board::well_formed_json` mends an escape it still meets (section
//! 4.4: the transport never refuses a writer's drawing). The cap is of the text as it
//! arrives, after that. The answers of `call_async` are read the same way ([`utf8_of`]).
//!
//! **The main-frame check does not tell a same-origin subframe apart**: a frame of the
//! view's own origin (a `srcdoc` frame inherits it) can call
//! `top.webkit.messageHandlers.<name>.postMessage`, and WebKit reports that message as the
//! main frame's, since the handler object it used is the main frame's. What keeps such a
//! frame from the bridge is that it never runs: a view with a bridge loads no subframe at
//! all (the navigation delegate cancels every subframe navigation of such a view,
//! `view::ViewHost::subframe_may_load`: a file, `srcdoc`, `data:`, `about:blank` with a
//! source). The whiteboard's policies alone would not be enough: they refuse a frame of a
//! file (`frame-src 'none'`) and every inline script, but a `srcdoc` frame inherits the
//! view's origin and may load a script of it (`script-src 'self'`; seen in the WebKit
//! program, `frames_never_reach_the_bridge`). A frame the page makes itself without a
//! source (an empty `about:blank`) has only what the page's own script puts in it, and a
//! message it posts through its own handlers is a subframe's and dropped.
//!
//! The handler holds a `Weak` of the view's shared state: the controller retains its
//! handlers, and a strong reference would keep the view's state alive with it.

use std::{
    ffi::c_void,
    rc::{Rc, Weak},
    sync::{
        Once,
        atomic::{AtomicUsize, Ordering},
    },
};

use objc::{
    class,
    declare::ClassDecl,
    msg_send,
    runtime::{BOOL, Class, NO, Object, Protocol, Sel},
    sel, sel_impl,
};

use crate::{
    Bridge, SCHEME,
    ns::{self, Id, NIL, is_kind, nsstring, release, string_of},
    view::ViewShared,
};

const CLASS: &str = "InnerWordWebBoardMessages";
const STATE: &str = "innerwordBridge";

/// What the handler object holds.
struct BridgeState {
    shared: Weak<ViewShared>,
    bridge: Bridge,
}

/// Handlers alive in this process (made and not yet deallocated).
static LIVE: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn live() -> usize {
    LIVE.load(Ordering::SeqCst)
}

fn handler_class() -> &'static Class {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let Some(mut decl) = ClassDecl::new(CLASS, class!(NSObject)) else {
            return;
        };
        if let Some(protocol) = Protocol::get("WKScriptMessageHandler") {
            decl.add_protocol(protocol);
        }
        decl.add_ivar::<*mut c_void>(STATE);
        // SAFETY: the signatures of the selectors.
        unsafe {
            decl.add_method(
                sel!(userContentController:didReceiveScriptMessage:),
                received as extern "C" fn(&Object, Sel, Id, Id),
            );
            decl.add_method(sel!(dealloc), dealloc as extern "C" fn(&mut Object, Sel));
        }
        decl.register();
    });
    Class::get(CLASS).expect("the bridge's message handler class")
}

/// Adds the bridge's handler to `configuration`'s user content controller, in the page
/// world, under `bridge.name`. The controller holds the handler; `WebPage`'s `Drop`
/// removes it.
pub(crate) unsafe fn install(configuration: Id, bridge: Bridge, shared: &Rc<ViewShared>) {
    // SAFETY: `new` on a subclass of NSObject; WebKit's own selectors (macOS 11), asked
    // for by `check_selectors`.
    unsafe {
        let handler: Id = msg_send![handler_class(), new];
        let state = Box::into_raw(Box::new(BridgeState {
            shared: Rc::downgrade(shared),
            bridge,
        }));
        (*handler).set_ivar::<*mut c_void>(STATE, state.cast());
        LIVE.fetch_add(1, Ordering::SeqCst);
        let controller: Id = msg_send![configuration, userContentController];
        let world: Id = msg_send![class!(WKContentWorld), pageWorld];
        let _: () = msg_send![controller,
            addScriptMessageHandler: handler
            contentWorld: world
            name: nsstring(bridge.name)];
        release(handler);
    }
}

extern "C" fn dealloc(this: &mut Object, _: Sel) {
    // SAFETY: the box `install` made, freed once, then NSObject's dealloc.
    unsafe {
        let state = *this.get_ivar::<*mut c_void>(STATE);
        if !state.is_null() {
            this.set_ivar::<*mut c_void>(STATE, std::ptr::null_mut());
            drop(Box::from_raw(state as *mut BridgeState));
            LIVE.fetch_sub(1, Ordering::SeqCst);
        }
        let _: () = msg_send![super(this, class!(NSObject)), dealloc];
    }
}

extern "C" fn received(this: &Object, _: Sel, _controller: Id, message: Id) {
    // SAFETY: the box `install` made; it lives until `dealloc`.
    let state = unsafe {
        let state = *this.get_ivar::<*mut c_void>(STATE);
        (state as *const BridgeState).as_ref()
    };
    let Some(state) = state else {
        return;
    };
    let Some(shared) = state.shared.upgrade() else {
        return;
    };
    // SAFETY: a WKScriptMessage WebKit hands over, on the main thread.
    let body = unsafe { checked_body(message, &state.bridge, &shared.host) };
    match body {
        Some(body) => {
            shared
                .messages_received
                .set(shared.messages_received.get() + 1);
            let events = shared.events.clone();
            ns::guarded("a message of the bridge", move || events.message(body));
        }
        None => shared
            .messages_dropped
            .set(shared.messages_dropped.get() + 1),
    }
}

/// The body of a message that passes every check, else None.
unsafe fn checked_body(message: Id, bridge: &Bridge, own_host: &str) -> Option<String> {
    // SAFETY: WebKit's own selectors on a live message (macOS 11), and Foundation's.
    unsafe {
        let name: Id = msg_send![message, name];
        if string_of(name) != bridge.name {
            return None;
        }
        let frame: Id = msg_send![message, frameInfo];
        if frame.is_null() {
            return None;
        }
        let main: BOOL = msg_send![frame, isMainFrame];
        if main == NO {
            return None;
        }
        let origin: Id = msg_send![frame, securityOrigin];
        if origin.is_null() {
            return None;
        }
        let protocol: Id = msg_send![origin, protocol];
        let host: Id = msg_send![origin, host];
        if !string_of(protocol).eq_ignore_ascii_case(SCHEME)
            || !string_of(host).eq_ignore_ascii_case(own_host)
        {
            return None;
        }
        let body: Id = msg_send![message, body];
        if !is_kind(body, "NSString") {
            return None;
        }
        // A string has at least as many UTF-8 bytes as UTF-16 units: one that is longer
        // in units is refused before it is copied.
        let units: usize = msg_send![body, length];
        if units > bridge.max_bytes {
            return None;
        }
        utf8_of(body).filter(|text| text.len() <= bridge.max_bytes)
    }
}

/// The exact text of an `NSString` (a NUL in it included, which `UTF8String` would end
/// at), a lone surrogate in it made U+FFFD (the module's notes say why). Strict UTF-8 first,
/// which every well-formed string passes at Foundation's speed; a string it refuses is read
/// as its UTF-16 units and decoded with replacement. None only for nil.
pub(crate) unsafe fn utf8_of(string: Id) -> Option<String> {
    if string.is_null() {
        return None;
    }
    // SAFETY: Foundation's own selectors on a string; the data is autoreleased and read
    // before this returns; the buffer has room for every unit the string has.
    unsafe {
        let data: Id = msg_send![string,
            dataUsingEncoding: 4usize
            allowLossyConversion: NO];
        if data != NIL {
            let length: usize = msg_send![data, length];
            if length == 0 {
                return Some(String::new());
            }
            let bytes: *const u8 = msg_send![data, bytes];
            if !bytes.is_null() {
                let slice = std::slice::from_raw_parts(bytes, length);
                return Some(String::from_utf8_lossy(slice).into_owned());
            }
        }
        let units: usize = msg_send![string, length];
        let mut buffer: Vec<u16> = vec![0; units];
        let _: () = msg_send![string,
            getCharacters: buffer.as_mut_ptr()
            range: NSRange { location: 0, length: units }];
        Some(String::from_utf16_lossy(&buffer))
    }
}

/// `NSRange`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct NSRange {
    location: usize,
    length: usize,
}

// SAFETY: the layout and encoding of NSRange on 64-bit Apple platforms.
unsafe impl objc::Encode for NSRange {
    fn encode() -> objc::Encoding {
        // SAFETY: the type's own layout.
        unsafe { objc::Encoding::from_str("{_NSRange=QQ}") }
    }
}
