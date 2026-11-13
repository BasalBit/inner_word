//! The configuration every web view of this crate gets, the view's and the check's
//! alike (section 5 of the design: "Every way out, and the policy"), and the data
//! stores.

use objc::{
    class, msg_send,
    runtime::{NO, YES},
    sel, sel_impl,
};

use crate::{
    SCHEME,
    ns::{Id, nsstring, responds},
};

/// `WKAudiovisualMediaTypeAll`: every medium waits for a click of the reader.
const MEDIA_TYPES_ALL: usize = usize::MAX;

thread_local! {
    static VIEW_STORE: Id = {
        // SAFETY: kept for the life of the process.
        unsafe { new_data_store() }
    };
}

/// A data store that is not persistent and opens no connection ahead of a navigation
/// (owned by the caller: +1).
///
/// WebKit preconnects (DNS, TCP and TLS) to the target of a main-frame link
/// navigation while it asks the policy, also for a link a script clicks: a page could
/// make the Mac look up any name it likes, a channel out of the page. The
/// configuration's `allowsServerPreconnect` (private, among what `available` asks for)
/// switches that off.
unsafe fn new_data_store() -> Id {
    // SAFETY: WebKit's own selectors, asked for by `check_selectors`.
    unsafe {
        let configuration: Id = msg_send![class!(_WKWebsiteDataStoreConfiguration), alloc];
        let configuration: Id = msg_send![configuration, initNonPersistentConfiguration];
        let _: () = msg_send![configuration, setAllowsServerPreconnect: NO];
        let store: Id = msg_send![class!(WKWebsiteDataStore), alloc];
        let store: Id = msg_send![store, _initWithConfiguration: configuration];
        crate::ns::release(configuration);
        store
    }
}

/// The data store of every view of this process: not persistent, so a page's storage
/// lasts while the app runs and is gone at quit; each page is its own origin in it.
pub(crate) fn view_data_store() -> Id {
    VIEW_STORE.with(|store| *store)
}

/// A data store of its own, for one check (owned by the caller: +1).
pub(crate) fn fresh_data_store() -> Id {
    // SAFETY: see `new_data_store`.
    unsafe { new_data_store() }
}

/// A configuration (owned by the caller: +1) that serves the page through `handler`,
/// keeps its data in `data_store`, and leaves the page no way out: no autoplay, no
/// AirPlay, no window opened by script, no fraud lookups, no full screen of an
/// element, WebRTC and media devices off, `tab` reaching links. The rule list is
/// added later ([`add_rules`]), once it is compiled.
pub(crate) unsafe fn configuration(handler: Id, data_store: Id) -> Id {
    // SAFETY: WebKit's own selectors with the types its headers declare; the private
    // switches are among what `check_selectors` (and so `available`) asks for.
    unsafe {
        let config: Id = msg_send![class!(WKWebViewConfiguration), new];
        let _: () = msg_send![config, setURLSchemeHandler: handler forURLScheme: nsstring(SCHEME)];
        let _: () = msg_send![config, setWebsiteDataStore: data_store];
        let _: () = msg_send![config, setMediaTypesRequiringUserActionForPlayback: MEDIA_TYPES_ALL];
        let _: () = msg_send![config, setAllowsAirPlayForMediaPlayback: NO];
        let preferences: Id = msg_send![config, preferences];
        let _: () = msg_send![preferences, setJavaScriptCanOpenWindowsAutomatically: NO];
        let _: () = msg_send![preferences, setFraudulentWebsiteWarningEnabled: NO];
        let _: () = msg_send![preferences, setElementFullscreenEnabled: NO];
        let _: () = msg_send![preferences, setTabFocusesLinks: YES];
        let _: () = msg_send![preferences, _setPeerConnectionEnabled: NO];
        let _: () = msg_send![preferences, _setMediaDevicesEnabled: NO];
        // Every answer says `X-DNS-Prefetch-Control: off` already; where the switch
        // is there, no prefetching at all.
        if responds(preferences, sel!(_setDNSPrefetchingEnabled:)) {
            let _: () = msg_send![preferences, _setDNSPrefetchingEnabled: NO];
        }
        config
    }
}

/// The switches of a web view itself: no link previews (they would load the address),
/// no swipes back and forth, no magnification, no Web Inspector where the view has
/// the switch (macOS 13.3; its default before).
pub(crate) unsafe fn settle_web_view(web_view: Id) {
    // SAFETY: WebKit's own selectors; `setInspectable:` only where it answers.
    unsafe {
        let _: () = msg_send![web_view, setAllowsLinkPreview: NO];
        let _: () = msg_send![web_view, setAllowsBackForwardNavigationGestures: NO];
        let _: () = msg_send![web_view, setAllowsMagnification: NO];
        if responds(web_view, sel!(setInspectable:)) {
            let _: () = msg_send![web_view, setInspectable: NO];
        }
    }
}

/// Adds the compiled rule list to a web view's content controller.
pub(crate) unsafe fn add_rules(web_view: Id, rules: Id) {
    // SAFETY: WebKit's own selectors; the controller is the live one of the view.
    unsafe {
        let config: Id = msg_send![web_view, configuration];
        let controller: Id = msg_send![config, userContentController];
        let _: () = msg_send![controller, addContentRuleList: rules];
    }
}

/// Mutes the page's media, where the web view answers it (`_setPageMuted:` with
/// "audio muted"): for the check, which nobody listens to.
pub(crate) unsafe fn mute(web_view: Id) {
    // SAFETY: asked with respondsToSelector: first.
    unsafe {
        if responds(web_view, sel!(_setPageMuted:)) {
            let _: () = msg_send![web_view, _setPageMuted: 1usize];
        }
    }
}
