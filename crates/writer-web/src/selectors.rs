//! Whether this Mac has what pages need: macOS 12.3, and every class and selector this
//! crate sends. A selector that is not there is an exception when it is sent, not a
//! compile error, and an exception ends the app: so the runtime is asked first.

use std::ffi::{CString, c_char, c_void};

use objc::{
    class, msg_send,
    rc::autoreleasepool,
    runtime::{BOOL, Class, NO, Sel},
    sel, sel_impl,
};

use crate::ns::{Id, OsVersion};

/// Every class and selector this crate sends, with whether it goes to the class
/// itself. The private switches of WebRTC and media devices are among them (without
/// them a page could send UDP that neither the policy nor the rule list sees), and so
/// is the data store's `allowsServerPreconnect` (without it a link a script clicks
/// makes WebKit connect to its host before the policy is asked). Optional ones, asked
/// with `respondsToSelector:` where they are sent: `setInspectable:`, `_setPageMuted:`,
/// `_setWindowOcclusionDetectionEnabled:`, `_isUserInitiated`, `absoluteLinkURL`,
/// `_setDNSPrefetchingEnabled:`.
const USED: &[(&str, bool, &str)] = &[
    // Foundation
    ("NSString", true, "alloc"),
    ("NSString", true, "string"),
    ("NSString", false, "initWithData:encoding:"),
    ("NSString", false, "UTF8String"),
    ("NSObject", false, "isKindOfClass:"),
    ("NSObject", false, "respondsToSelector:"),
    ("NSObject", false, "description"),
    ("NSError", false, "localizedDescription"),
    ("NSError", false, "userInfo"),
    ("NSError", false, "domain"),
    ("NSError", false, "code"),
    ("NSDictionary", true, "dictionaryWithObjects:forKeys:count:"),
    ("NSDictionary", false, "objectForKey:"),
    ("NSMutableDictionary", false, "setObject:forKey:"),
    ("NSArray", false, "count"),
    ("NSArray", false, "objectAtIndex:"),
    ("NSNumber", false, "boolValue"),
    ("NSNumber", false, "stringValue"),
    ("NSData", true, "dataWithBytes:length:"),
    ("NSJSONSerialization", true, "isValidJSONObject:"),
    (
        "NSJSONSerialization",
        true,
        "dataWithJSONObject:options:error:",
    ),
    ("NSThread", true, "isMainThread"),
    ("NSProcessInfo", true, "processInfo"),
    ("NSProcessInfo", false, "systemUptime"),
    ("NSProcessInfo", false, "isOperatingSystemAtLeastVersion:"),
    ("NSBundle", true, "bundleForClass:"),
    ("NSBundle", false, "objectForInfoDictionaryKey:"),
    ("NSTimer", true, "timerWithTimeInterval:repeats:block:"),
    ("NSTimer", false, "invalidate"),
    ("NSRunLoop", true, "mainRunLoop"),
    ("NSRunLoop", false, "addTimer:forMode:"),
    ("NSURL", true, "URLWithString:"),
    ("NSURL", true, "fileURLWithPath:"),
    ("NSURL", true, "fileURLWithPath:isDirectory:"),
    ("NSURL", false, "absoluteString"),
    ("NSURLRequest", true, "requestWithURL:"),
    ("NSURLRequest", false, "URL"),
    ("NSURLRequest", false, "HTTPMethod"),
    ("NSURLRequest", false, "valueForHTTPHeaderField:"),
    ("NSURLResponse", false, "URL"),
    ("NSURLResponse", false, "MIMEType"),
    (
        "NSHTTPURLResponse",
        false,
        "initWithURL:statusCode:HTTPVersion:headerFields:",
    ),
    ("NSUndoManager", false, "canUndo"),
    ("NSUndoManager", false, "canRedo"),
    ("NSUndoManager", false, "undo"),
    ("NSUndoManager", false, "redo"),
    // AppKit
    ("NSApplication", true, "sharedApplication"),
    ("NSApplication", false, "isRunning"),
    ("NSApplication", false, "setActivationPolicy:"),
    (
        "NSWindow",
        false,
        "initWithContentRect:styleMask:backing:defer:",
    ),
    ("NSWindow", false, "setReleasedWhenClosed:"),
    ("NSWindow", false, "setExcludedFromWindowsMenu:"),
    ("NSWindow", false, "setCollectionBehavior:"),
    ("NSWindow", false, "setHasShadow:"),
    ("NSWindow", false, "orderBack:"),
    ("NSWindow", false, "orderOut:"),
    ("NSWindow", false, "close"),
    ("NSWindow", false, "contentView"),
    ("NSWindow", false, "makeFirstResponder:"),
    ("NSWindow", false, "firstResponder"),
    ("NSView", false, "superview"),
    ("NSView", false, "window"),
    ("NSView", false, "isFlipped"),
    ("NSView", false, "bounds"),
    ("NSView", false, "frame"),
    ("NSView", false, "setFrame:"),
    ("NSView", false, "setHidden:"),
    ("NSView", false, "setWantsLayer:"),
    ("NSView", false, "layer"),
    ("NSView", false, "setAutoresizingMask:"),
    ("NSView", false, "addSubview:"),
    ("NSView", false, "addSubview:positioned:relativeTo:"),
    ("NSView", false, "removeFromSuperview"),
    ("NSView", false, "convertPoint:fromView:"),
    ("NSView", false, "setAppearance:"),
    ("NSView", false, "willOpenMenu:withEvent:"),
    ("NSView", false, "hitTest:"),
    ("NSResponder", false, "keyDown:"),
    ("NSResponder", false, "keyUp:"),
    ("NSResponder", false, "mouseUp:"),
    ("NSResponder", false, "mouseDown:"),
    ("NSResponder", false, "rightMouseDown:"),
    ("NSResponder", false, "mouseMoved:"),
    ("NSResponder", false, "becomeFirstResponder"),
    ("NSResponder", false, "tryToPerform:with:"),
    ("NSResponder", false, "undoManager"),
    ("NSEvent", false, "keyCode"),
    ("NSEvent", false, "characters"),
    ("NSEvent", false, "charactersIgnoringModifiers"),
    ("NSEvent", false, "modifierFlags"),
    ("NSEvent", false, "timestamp"),
    ("NSEvent", false, "locationInWindow"),
    ("NSMenu", false, "numberOfItems"),
    ("NSMenu", false, "itemAtIndex:"),
    ("NSMenu", false, "removeItemAtIndex:"),
    ("NSMenu", false, "removeAllItems"),
    ("NSMenu", false, "addItem:"),
    ("NSMenu", false, "cancelTrackingWithoutAnimation"),
    ("NSMenu", false, "performActionForItemAtIndex:"),
    ("NSMenuItem", true, "separatorItem"),
    ("NSMenuItem", false, "initWithTitle:action:keyEquivalent:"),
    ("NSMenuItem", false, "setKeyEquivalentModifierMask:"),
    ("NSMenuItem", false, "setTarget:"),
    ("NSMenuItem", false, "setTag:"),
    ("NSMenuItem", false, "tag"),
    ("NSMenuItem", false, "setEnabled:"),
    ("NSMenuItem", false, "isSeparatorItem"),
    ("NSMenuItem", false, "title"),
    ("NSMenuItem", false, "identifier"),
    ("NSMenuItem", false, "action"),
    ("NSImage", false, "CGImageForProposedRect:context:hints:"),
    ("NSAppearance", true, "appearanceNamed:"),
    ("NSColor", true, "colorWithSRGBRed:green:blue:alpha:"),
    ("NSPrintInfo", true, "sharedPrintInfo"),
    ("NSPrintInfo", false, "copy"),
    ("NSPrintInfo", false, "setJobDisposition:"),
    ("NSPrintInfo", false, "dictionary"),
    ("NSPrintOperation", false, "setShowsPrintPanel:"),
    ("NSPrintOperation", false, "setShowsProgressPanel:"),
    ("NSPrintOperation", false, "view"),
    (
        "NSPrintOperation",
        false,
        "runOperationModalForWindow:delegate:didRunSelector:contextInfo:",
    ),
    // QuartzCore
    ("CAShapeLayer", true, "layer"),
    ("CAShapeLayer", false, "setFrame:"),
    ("CAShapeLayer", false, "setPath:"),
    ("CAShapeLayer", false, "setFillRule:"),
    ("CALayer", false, "setMask:"),
    // WebKit
    ("WKWebsiteDataStore", false, "_initWithConfiguration:"),
    (
        "_WKWebsiteDataStoreConfiguration",
        false,
        "initNonPersistentConfiguration",
    ),
    (
        "_WKWebsiteDataStoreConfiguration",
        false,
        "setAllowsServerPreconnect:",
    ),
    (
        "WKWebViewConfiguration",
        false,
        "setURLSchemeHandler:forURLScheme:",
    ),
    ("WKWebViewConfiguration", false, "setWebsiteDataStore:"),
    (
        "WKWebViewConfiguration",
        false,
        "setMediaTypesRequiringUserActionForPlayback:",
    ),
    (
        "WKWebViewConfiguration",
        false,
        "setAllowsAirPlayForMediaPlayback:",
    ),
    ("WKWebViewConfiguration", false, "preferences"),
    ("WKWebViewConfiguration", false, "userContentController"),
    (
        "WKPreferences",
        false,
        "setJavaScriptCanOpenWindowsAutomatically:",
    ),
    (
        "WKPreferences",
        false,
        "setFraudulentWebsiteWarningEnabled:",
    ),
    ("WKPreferences", false, "setElementFullscreenEnabled:"),
    ("WKPreferences", false, "setTabFocusesLinks:"),
    ("WKPreferences", false, "_setPeerConnectionEnabled:"),
    ("WKPreferences", false, "_setMediaDevicesEnabled:"),
    ("WKWebView", false, "initWithFrame:configuration:"),
    ("WKWebView", false, "configuration"),
    ("WKWebView", false, "setNavigationDelegate:"),
    ("WKWebView", false, "setUIDelegate:"),
    ("WKWebView", false, "setAllowsLinkPreview:"),
    (
        "WKWebView",
        false,
        "setAllowsBackForwardNavigationGestures:",
    ),
    ("WKWebView", false, "setAllowsMagnification:"),
    ("WKWebView", false, "loadRequest:"),
    ("WKWebView", false, "stopLoading"),
    ("WKWebView", false, "setPageZoom:"),
    ("WKWebView", false, "pageZoom"),
    ("WKWebView", false, "setUnderPageBackgroundColor:"),
    (
        "WKWebView",
        false,
        "takeSnapshotWithConfiguration:completionHandler:",
    ),
    (
        "WKWebView",
        false,
        "findString:withConfiguration:completionHandler:",
    ),
    ("WKWebView", false, "evaluateJavaScript:completionHandler:"),
    (
        "WKWebView",
        false,
        "evaluateJavaScript:inFrame:inContentWorld:completionHandler:",
    ),
    ("WKWebView", false, "printOperationWithPrintInfo:"),
    ("WKUserContentController", false, "addContentRuleList:"),
    ("WKUserContentController", false, "addUserScript:"),
    (
        "WKUserContentController",
        false,
        "addScriptMessageHandler:contentWorld:name:",
    ),
    (
        "WKUserContentController",
        false,
        "removeAllScriptMessageHandlers",
    ),
    ("WKUserContentController", false, "removeAllUserScripts"),
    (
        "WKUserScript",
        false,
        "initWithSource:injectionTime:forMainFrameOnly:inContentWorld:",
    ),
    ("WKContentWorld", true, "worldWithName:"),
    ("WKContentWorld", true, "defaultClientWorld"),
    ("WKContentRuleListStore", true, "storeWithURL:"),
    (
        "WKContentRuleListStore",
        false,
        "lookUpContentRuleListForIdentifier:completionHandler:",
    ),
    (
        "WKContentRuleListStore",
        false,
        "compileContentRuleListForIdentifier:encodedContentRuleList:completionHandler:",
    ),
    (
        "WKContentRuleListStore",
        false,
        "getAvailableContentRuleListIdentifiers:",
    ),
    ("WKFindConfiguration", false, "setBackwards:"),
    ("WKFindConfiguration", false, "setCaseSensitive:"),
    ("WKFindConfiguration", false, "setWraps:"),
    ("WKFindResult", false, "matchFound"),
    ("WKNavigationAction", false, "request"),
    ("WKNavigationAction", false, "targetFrame"),
    ("WKNavigationAction", false, "navigationType"),
    ("WKNavigationAction", false, "shouldPerformDownload"),
    ("WKFrameInfo", false, "isMainFrame"),
    ("WKFrameInfo", false, "request"),
    ("WKNavigationResponse", false, "response"),
    ("WKNavigationResponse", false, "isForMainFrame"),
    ("WKNavigationResponse", false, "canShowMIMEType"),
    ("WKScriptMessage", false, "body"),
    ("WKScriptMessage", false, "frameInfo"),
];

/// What a scheme task is asked for: a protocol, so asked of the protocol.
const TASK: &[&str] = &[
    "request",
    "didReceiveResponse:",
    "didReceiveData:",
    "didFinish",
];

/// The protocols the classes of this crate adopt and WebKit's own classes register
/// with the runtime. (`WKScriptMessageHandler` is registered only by a class that
/// adopts it, and WebKit asks no handler whether it conforms; `NSString`'s
/// initializers are a class cluster's, answered by its concrete classes.)
const PROTOCOLS: &[&str] = &[
    "WKURLSchemeHandler",
    "WKURLSchemeTask",
    "WKNavigationDelegate",
    "WKUIDelegate",
];

#[repr(C)]
struct MethodDescription {
    name: *const c_void,
    types: *const c_char,
}

unsafe extern "C" {
    fn objc_getProtocol(name: *const c_char) -> *const c_void;
    fn protocol_getMethodDescription(
        protocol: *const c_void,
        selector: Sel,
        required: BOOL,
        instance: BOOL,
    ) -> MethodDescription;
}

/// Asks the runtime for every class and selector this crate sends (a wrong one is an
/// exception, not a compile error), as `writer_print::check_selectors` does: the
/// required ones, the private WebRTC switches among them. The optional ones
/// (`_setWindowOcclusionDetectionEnabled:`, `_isUserInitiated`, `_setPageMuted:`,
/// `inspectable`) are asked with `respondsToSelector:` where they are sent, not here.
/// `Err` names what is missing, comma separated (`_setPeerConnectionEnabled:`,
/// `WKContentWorld`).
pub fn check_selectors() -> Result<(), String> {
    let mut missing: Vec<String> = Vec::new();
    autoreleasepool(|| {
        for &(class_name, of_class, selector) in USED {
            let Some(class) = Class::get(class_name) else {
                if !missing.iter().any(|name| name == class_name) {
                    missing.push(class_name.to_string());
                }
                continue;
            };
            let sel = Sel::register(selector);
            // SAFETY: both questions are NSObject's and take a selector.
            let answers: BOOL = unsafe {
                if of_class {
                    msg_send![class, respondsToSelector: sel]
                } else {
                    msg_send![class, instancesRespondToSelector: sel]
                }
            };
            if answers == NO {
                missing.push(selector.to_string());
            }
        }
        for &protocol in PROTOCOLS {
            let name = CString::new(protocol).expect("a protocol's name");
            // SAFETY: the runtime's own lookup by name.
            if unsafe { objc_getProtocol(name.as_ptr()) }.is_null() {
                missing.push(protocol.to_string());
            }
        }
        let name = CString::new("WKURLSchemeTask").expect("a protocol's name");
        // SAFETY: the runtime's own; a protocol that is there.
        let protocol = unsafe { objc_getProtocol(name.as_ptr()) };
        if !protocol.is_null() {
            for &selector in TASK {
                let sel = Sel::register(selector);
                let found = [true, false].iter().any(|&required| {
                    // SAFETY: the runtime's own question of a protocol.
                    let description = unsafe {
                        protocol_getMethodDescription(
                            protocol,
                            sel,
                            crate::ns::yes_no(required),
                            crate::ns::yes_no(true),
                        )
                    };
                    !description.name.is_null()
                });
                if !found {
                    missing.push(format!("WKURLSchemeTask {selector}"));
                }
            }
        }
    });
    if missing.is_empty() {
        Ok(())
    } else {
        Err(missing.join(", "))
    }
}

/// Whether this is macOS `major.minor` or later.
pub(crate) fn at_least(major: isize, minor: isize) -> bool {
    // SAFETY: Foundation's own (macOS 10.10), with the struct it declares.
    unsafe {
        let info: Id = msg_send![class!(NSProcessInfo), processInfo];
        if !crate::ns::responds(info, sel!(isOperatingSystemAtLeastVersion:)) {
            return false;
        }
        let version = OsVersion {
            major,
            minor,
            patch: 0,
        };
        let answer: BOOL = msg_send![info, isOperatingSystemAtLeastVersion: version];
        answer != NO
    }
}

/// macOS 12.3 or later and every class and selector the crate sends (the private
/// `_setPeerConnectionEnabled:` and `_setMediaDevicesEnabled:` of WKPreferences
/// included). Asked before anything of WebKit is made; `Err` is the sentence to show.
pub fn available() -> Result<(), String> {
    if !at_least(12, 3) {
        return Err("Pages need macOS 12.3 or later.".to_string());
    }
    check_selectors().map_err(|missing| {
        format!("This macOS does not have what pages need to stay offline ({missing})")
    })
}
