//! The navigation and UI delegate of every web view of this crate: one object that is
//! both, over a [`Host`] (the view's or the check's) that says what the policy needs
//! and hears what happened.
//!
//! WebKit holds its delegates weakly: whoever sets one keeps it (`WebPage`, the check)
//! and sets the web view's delegates to nil before letting it go.

use std::{ffi::c_void, rc::Rc, sync::Once};

use block::Block;
use objc::{
    class,
    declare::ClassDecl,
    msg_send,
    runtime::{BOOL, Class, NO, Object, Protocol, Sel},
    sel, sel_impl,
};

use crate::{
    ns::{Id, NIL, error_code, error_text, nsstring, responds, string_of, uptime},
    policy::{self, Decision, Navigation, NewWindow},
};

/// What the delegate asks of its owner and tells it. Every method runs on the main
/// thread, inside WebKit's callbacks.
pub(crate) trait Host {
    /// The host of the page's own files (`page-12`, `check`).
    fn own_host(&self) -> String;
    /// When the reader last clicked or pressed a key in the view (NSEvent time).
    fn last_reader_event(&self) -> Option<f64>;
    fn finished(&self, navigation: Id);
    fn failed(&self, navigation: Id, message: String);
    fn terminated(&self);
    /// A navigation (or a new window) the policy cancelled.
    fn cancelled(&self, url: &str);
    fn open_externally(&self, url: String);
    fn alert(&self, message: String);
    /// The response of a frame's document: its URL and media type.
    fn frame_response(&self, _url: &str, _media_type: &str, _main_frame: bool) {}
    /// The link under the pointer (WebKit's private hover callback, where it comes).
    fn hovered_link(&self, _link: Option<String>) {}
}

const CLASS: &str = "WriterWebDelegate";
const HOST: &str = "writerHost";

/// `WKNavigationActionPolicy` and `WKNavigationResponsePolicy`.
const POLICY_CANCEL: isize = 0;
const POLICY_ALLOW: isize = 1;
/// `WKNavigationTypeLinkActivated`.
const LINK_ACTIVATED: isize = 0;
/// `WKPermissionDecisionDeny`.
const PERMISSION_DENY: isize = 2;

fn delegate_class() -> &'static Class {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let Some(mut decl) = ClassDecl::new(CLASS, class!(NSObject)) else {
            return;
        };
        for name in ["WKNavigationDelegate", "WKUIDelegate"] {
            if let Some(protocol) = Protocol::get(name) {
                decl.add_protocol(protocol);
            }
        }
        decl.add_ivar::<*mut c_void>(HOST);
        // SAFETY: each function has the signature its selector's encoding says
        // (objects as `Id`, blocks as pointers, NSInteger as isize).
        unsafe {
            decl.add_method(
                sel!(webView:decidePolicyForNavigationAction:decisionHandler:),
                decide_action as extern "C" fn(&Object, Sel, Id, Id, *mut c_void),
            );
            decl.add_method(
                sel!(webView:decidePolicyForNavigationResponse:decisionHandler:),
                decide_response as extern "C" fn(&Object, Sel, Id, Id, *mut c_void),
            );
            decl.add_method(
                sel!(webView:didFinishNavigation:),
                did_finish as extern "C" fn(&Object, Sel, Id, Id),
            );
            decl.add_method(
                sel!(webView:didFailNavigation:withError:),
                did_fail as extern "C" fn(&Object, Sel, Id, Id, Id),
            );
            decl.add_method(
                sel!(webView:didFailProvisionalNavigation:withError:),
                did_fail as extern "C" fn(&Object, Sel, Id, Id, Id),
            );
            decl.add_method(
                sel!(webViewWebContentProcessDidTerminate:),
                did_terminate as extern "C" fn(&Object, Sel, Id),
            );
            decl.add_method(
                sel!(webView:createWebViewWithConfiguration:forNavigationAction:windowFeatures:),
                create_web_view as extern "C" fn(&Object, Sel, Id, Id, Id, Id) -> Id,
            );
            decl.add_method(
                sel!(webView:runJavaScriptAlertPanelWithMessage:initiatedByFrame:completionHandler:),
                alert_panel as extern "C" fn(&Object, Sel, Id, Id, Id, *mut c_void),
            );
            decl.add_method(
                sel!(webView:runJavaScriptConfirmPanelWithMessage:initiatedByFrame:completionHandler:),
                confirm_panel as extern "C" fn(&Object, Sel, Id, Id, Id, *mut c_void),
            );
            decl.add_method(
                sel!(webView:runJavaScriptTextInputPanelWithPrompt:defaultText:initiatedByFrame:completionHandler:),
                prompt_panel as extern "C" fn(&Object, Sel, Id, Id, Id, Id, *mut c_void),
            );
            decl.add_method(
                sel!(webView:runOpenPanelWithParameters:initiatedByFrame:completionHandler:),
                open_panel as extern "C" fn(&Object, Sel, Id, Id, Id, *mut c_void),
            );
            decl.add_method(
                sel!(webView:requestMediaCapturePermissionForOrigin:initiatedByFrame:type:decisionHandler:),
                media_capture as extern "C" fn(&Object, Sel, Id, Id, Id, isize, *mut c_void),
            );
            // Private, where WebKit calls it: the element under the pointer, for the
            // link of the right-click menu.
            decl.add_method(
                sel!(_webView:mouseDidMoveOverElement:withFlags:userInfo:),
                mouse_over as extern "C" fn(&Object, Sel, Id, Id, usize, Id),
            );
            decl.add_method(sel!(dealloc), dealloc as extern "C" fn(&mut Object, Sel));
        }
        decl.register();
    });
    Class::get(CLASS).expect("the delegate's class")
}

/// A delegate (owned by the caller: +1) over `host`.
pub(crate) fn new_delegate(host: Rc<dyn Host>) -> Id {
    // SAFETY: `new` on a subclass of NSObject; the instance variable is ours.
    unsafe {
        let delegate: Id = msg_send![delegate_class(), new];
        let host: *mut Rc<dyn Host> = Box::into_raw(Box::new(host));
        (*delegate).set_ivar::<*mut c_void>(HOST, host.cast());
        delegate
    }
}

/// The delegate's host, while it has one.
fn host_of(this: &Object) -> Option<Rc<dyn Host>> {
    // SAFETY: the box `new_delegate` made; it lives until `dealloc`.
    unsafe {
        let host = *this.get_ivar::<*mut c_void>(HOST);
        (host as *const Rc<dyn Host>).as_ref().cloned()
    }
}

extern "C" fn dealloc(this: &mut Object, _: Sel) {
    // SAFETY: the box `new_delegate` made, freed once, then NSObject's dealloc.
    unsafe {
        let host = *this.get_ivar::<*mut c_void>(HOST);
        if !host.is_null() {
            this.set_ivar::<*mut c_void>(HOST, std::ptr::null_mut());
            drop(Box::from_raw(host as *mut Rc<dyn Host>));
        }
        let _: () = msg_send![super(this, class!(NSObject)), dealloc];
    }
}

/// Calls a decision handler `void (^)(NSInteger)`.
unsafe fn decide(handler: *mut c_void, value: isize) {
    if handler.is_null() {
        return;
    }
    // SAFETY: WebKit's block of that signature.
    unsafe {
        let block = &*(handler as *const Block<(isize,), ()>);
        block.call((value,));
    }
}

unsafe fn url_of_request(request: Id) -> String {
    // SAFETY: an NSURLRequest or nil.
    unsafe {
        if request.is_null() {
            return String::new();
        }
        let url: Id = msg_send![request, URL];
        if url.is_null() {
            return String::new();
        }
        let absolute: Id = msg_send![url, absoluteString];
        string_of(absolute)
    }
}

/// Whether the reader acted for `action` (the gate of `policy::reader_acted`, with
/// `_isUserInitiated` where the navigation action answers it).
unsafe fn reader_acted(host: &dyn Host, action: Id) -> bool {
    // SAFETY: the private selector is sent only where it is answered.
    let user_initiated = unsafe {
        if responds(action, sel!(_isUserInitiated)) {
            let value: BOOL = msg_send![action, _isUserInitiated];
            Some(value != NO)
        } else {
            None
        }
    };
    policy::reader_acted(host.last_reader_event(), uptime(), user_initiated)
}

extern "C" fn decide_action(
    this: &Object,
    _: Sel,
    _web_view: Id,
    action: Id,
    handler: *mut c_void,
) {
    // SAFETY: WebKit's objects of the declared types; the handler is called once.
    unsafe {
        let Some(host) = host_of(this) else {
            decide(handler, POLICY_CANCEL);
            return;
        };
        let request: Id = msg_send![action, request];
        let url = url_of_request(request);
        let target: Id = msg_send![action, targetFrame];
        let main_frame = if target.is_null() {
            true
        } else {
            let main: BOOL = msg_send![target, isMainFrame];
            main != NO
        };
        let kind: isize = msg_send![action, navigationType];
        let download = if responds(action, sel!(shouldPerformDownload)) {
            let download: BOOL = msg_send![action, shouldPerformDownload];
            download != NO
        } else {
            false
        };
        let link = kind == LINK_ACTIVATED;
        let reader = link && reader_acted(&*host, action);
        let own_host = host.own_host();
        let decision = policy::decide(&Navigation {
            url: &url,
            own_host: &own_host,
            main_frame,
            link,
            download,
            reader,
        });
        match decision {
            Decision::Allow => decide(handler, POLICY_ALLOW),
            Decision::Cancel => {
                decide(handler, POLICY_CANCEL);
                host.cancelled(&url);
            }
            Decision::OpenOutside => {
                decide(handler, POLICY_CANCEL);
                host.open_externally(url);
            }
        }
    }
}

extern "C" fn decide_response(
    this: &Object,
    _: Sel,
    _web_view: Id,
    navigation_response: Id,
    handler: *mut c_void,
) {
    // SAFETY: WebKit's objects of the declared types; the handler is called once.
    unsafe {
        let can_show: BOOL = msg_send![navigation_response, canShowMIMEType];
        let response: Id = msg_send![navigation_response, response];
        let main: BOOL = msg_send![navigation_response, isForMainFrame];
        let (url, media_type) = if response.is_null() {
            (String::new(), String::new())
        } else {
            let url: Id = msg_send![response, URL];
            let absolute: Id = if url.is_null() {
                NIL
            } else {
                msg_send![url, absoluteString]
            };
            let media_type: Id = msg_send![response, MIMEType];
            (string_of(absolute), string_of(media_type))
        };
        if let Some(host) = host_of(this) {
            host.frame_response(&url, &media_type, main != NO);
            if can_show == NO {
                // Something WebKit cannot show would be a download.
                decide(handler, POLICY_CANCEL);
                host.cancelled(&url);
                return;
            }
        }
        decide(
            handler,
            if can_show == NO {
                POLICY_CANCEL
            } else {
                POLICY_ALLOW
            },
        );
    }
}

extern "C" fn did_finish(this: &Object, _: Sel, _web_view: Id, navigation: Id) {
    if let Some(host) = host_of(this) {
        host.finished(navigation);
    }
}

extern "C" fn did_fail(this: &Object, _: Sel, _web_view: Id, navigation: Id, error: Id) {
    // SAFETY: an NSError.
    let (domain, code) = unsafe { error_code(error) };
    // A navigation the policy cancelled ends this way too; that is no failure of the
    // page ("Frame load interrupted", and a load that was stopped).
    if (domain == "WebKitErrorDomain" && code == 102)
        || (domain == "NSURLErrorDomain" && code == -999)
    {
        return;
    }
    // SAFETY: an NSError.
    let message = unsafe { error_text(error) };
    if let Some(host) = host_of(this) {
        host.failed(navigation, message);
    }
}

extern "C" fn did_terminate(this: &Object, _: Sel, _web_view: Id) {
    if let Some(host) = host_of(this) {
        host.terminated();
    }
}

extern "C" fn create_web_view(
    this: &Object,
    _: Sel,
    web_view: Id,
    _configuration: Id,
    action: Id,
    _features: Id,
) -> Id {
    // SAFETY: WebKit's objects; nil is the answer that opens no window.
    unsafe {
        let Some(host) = host_of(this) else {
            return NIL;
        };
        let request: Id = msg_send![action, request];
        let url = url_of_request(request);
        let reader = reader_acted(&*host, action);
        match policy::new_window(&url, &host.own_host(), reader) {
            NewWindow::LoadHere => {
                let _: Id = msg_send![web_view, loadRequest: request];
            }
            NewWindow::OpenOutside => host.open_externally(url),
            NewWindow::Nothing => host.cancelled(&url),
        }
        NIL
    }
}

extern "C" fn alert_panel(
    this: &Object,
    _: Sel,
    _web_view: Id,
    message: Id,
    _frame: Id,
    done: *mut c_void,
) {
    // SAFETY: an NSString; the block `void (^)(void)` is called once, at once.
    unsafe {
        if let Some(host) = host_of(this) {
            host.alert(string_of(message));
        }
        if !done.is_null() {
            let block = &*(done as *const Block<(), ()>);
            block.call(());
        }
    }
}

extern "C" fn confirm_panel(
    _this: &Object,
    _: Sel,
    _web_view: Id,
    _message: Id,
    _frame: Id,
    done: *mut c_void,
) {
    // SAFETY: the block `void (^)(BOOL)`, answered "Cancel" at once.
    unsafe {
        if !done.is_null() {
            let block = &*(done as *const Block<(BOOL,), ()>);
            block.call((NO,));
        }
    }
}

extern "C" fn prompt_panel(
    _this: &Object,
    _: Sel,
    _web_view: Id,
    _prompt: Id,
    _default: Id,
    _frame: Id,
    done: *mut c_void,
) {
    // SAFETY: the block `void (^)(NSString *)`, answered with nothing at once.
    unsafe {
        if !done.is_null() {
            let block = &*(done as *const Block<(Id,), ()>);
            block.call((NIL,));
        }
    }
}

extern "C" fn open_panel(
    _this: &Object,
    _: Sel,
    _web_view: Id,
    _parameters: Id,
    _frame: Id,
    done: *mut c_void,
) {
    // SAFETY: the block `void (^)(NSArray<NSURL *> *)`: no panel, no files.
    unsafe {
        if !done.is_null() {
            let block = &*(done as *const Block<(Id,), ()>);
            block.call((NIL,));
        }
    }
}

extern "C" fn media_capture(
    _this: &Object,
    _: Sel,
    _web_view: Id,
    _origin: Id,
    _frame: Id,
    _kind: isize,
    done: *mut c_void,
) {
    // SAFETY: the block `void (^)(WKPermissionDecision)`: denied without asking.
    unsafe { decide(done, PERMISSION_DENY) };
}

extern "C" fn mouse_over(
    this: &Object,
    _: Sel,
    _web_view: Id,
    element: Id,
    _flags: usize,
    _info: Id,
) {
    // SAFETY: a `_WKHitTestResult` or nil; its selector is asked for first.
    unsafe {
        let Some(host) = host_of(this) else {
            return;
        };
        let link = if responds(element, sel!(absoluteLinkURL)) {
            let url: Id = msg_send![element, absoluteLinkURL];
            if url.is_null() {
                None
            } else {
                let absolute: Id = msg_send![url, absoluteString];
                Some(string_of(absolute)).filter(|link| !link.is_empty())
            }
        } else {
            None
        };
        host.hovered_link(link);
    }
}

/// Makes `url` into a request (autoreleased).
pub(crate) unsafe fn request_for(url: &str) -> Id {
    // SAFETY: Foundation's own; nil when the URL cannot be read.
    unsafe {
        let url: Id = msg_send![class!(NSURL), URLWithString: nsstring(url)];
        if url.is_null() {
            return NIL;
        }
        msg_send![class!(NSURLRequest), requestWithURL: url]
    }
}
