//! The `WKWebView` subclass every web view of this crate is made of, and what a view
//! of the window shares with it.
//!
//! The subclass overrides, for a view of the window (`WebPage`):
//!
//! * `keyDown:` / `keyUp:`: asks `PageEvents::takes_key` and hands a key the owner
//!   takes to GPUI's view instead of the page; stamps the time of the reader's key;
//! * `mouseUp:`: stamps the time of the reader's click (the gate of links);
//! * `hitTest:` and `mouseMoved:`: a point inside a hole is not the web view's, so
//!   AppKit hit-tests the next sibling, GPUI's view (a toast over the page);
//! * `becomeFirstResponder`: `PageEvents::wants_focus` for a focus that did not come
//!   from `WebPage::focus`;
//! * `willOpenMenu:withEvent:`: WebKit's right-click menu cut down
//!   ([`crate::menu`]) and extended with the owner's items.
//!
//! A web view without a view of the window behind it (the check's, the offscreen
//! print's) behaves as a plain `WKWebView` in all of them.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    ffi::c_void,
    rc::Rc,
    sync::Once,
    time::{Duration, Instant},
};

use block::ConcreteBlock;
use objc::{
    class,
    declare::ClassDecl,
    msg_send,
    runtime::{BOOL, Class, NO, Object, Sel, YES},
    sel, sel_impl,
};

use crate::{
    KeyPress, PageEvents, Rect,
    delegate::Host,
    frame, menu,
    ns::{self, Id, NIL, NSPoint, NSRect, nsstring, string_of},
    policy,
    scheme::RequestLog,
};

/// What a view of the window shares with its web view and its delegate.
pub(crate) struct ViewShared {
    pub events: Rc<dyn PageEvents>,
    pub host: String,
    pub entry: String,
    /// GPUI's view (retained by `WebPage` while it lives; null afterwards).
    pub gpui_view: Cell<Id>,
    /// The web view (null once `WebPage` dropped it).
    pub web_view: Cell<Id>,
    /// Set while `WebPage::focus` / `unfocus` move the first responder.
    pub focusing: Cell<bool>,
    /// The `timestamp` of the last real `mouseUp:` or `keyDown:` that reached the page.
    pub last_reader_event: Cell<Option<f64>>,
    /// Holes, in the view's own points from its top left.
    pub holes: RefCell<Vec<Rect>>,
    /// The link under the pointer, as WebKit reported it last.
    pub hovered_link: RefCell<Option<String>>,
    /// The link under the pointer when the right button (or a control-click) went down,
    /// as the page's own document says (asked before the click reaches the page).
    pub pointed_link: RefCell<Option<String>>,
    /// The link the open right-click menu is about.
    pub menu_link: RefCell<Option<String>>,
    /// Whether each of the owner's menu items is enabled, by id.
    pub menu_enabled: RefCell<HashMap<u32, bool>>,
    /// Key codes whose `keyDown:` went to GPUI: their `keyUp:` follows it there.
    pub taken_keys: RefCell<Vec<u16>>,
    pub log: Rc<RequestLog>,
    pub blocked_navigations: Cell<usize>,
    /// The navigation of the last load of the entry.
    pub navigation: Cell<Id>,
    pub last_alert: Cell<Option<Instant>>,
}

impl ViewShared {
    pub(crate) fn new(
        events: Rc<dyn PageEvents>,
        host: &str,
        entry: &str,
        log: Rc<RequestLog>,
    ) -> ViewShared {
        ViewShared {
            events,
            host: host.to_ascii_lowercase(),
            entry: entry.trim_start_matches('/').to_string(),
            gpui_view: Cell::new(NIL),
            web_view: Cell::new(NIL),
            focusing: Cell::new(false),
            last_reader_event: Cell::new(None),
            holes: RefCell::new(Vec::new()),
            hovered_link: RefCell::new(None),
            pointed_link: RefCell::new(None),
            menu_link: RefCell::new(None),
            menu_enabled: RefCell::new(HashMap::new()),
            taken_keys: RefCell::new(Vec::new()),
            log,
            blocked_navigations: Cell::new(0),
            navigation: Cell::new(NIL),
            last_alert: Cell::new(None),
        }
    }
}

/// At most one alert of a page reaches the owner in this time.
pub(crate) const ALERT_EVERY: Duration = Duration::from_secs(5);

/// The delegate's host for a view of the window.
pub(crate) struct ViewHost(pub Rc<ViewShared>);

impl Host for ViewHost {
    fn own_host(&self) -> String {
        self.0.host.clone()
    }

    fn last_reader_event(&self) -> Option<f64> {
        self.0.last_reader_event.get()
    }

    fn finished(&self, _navigation: Id) {
        // Every load of the main frame that finished: the entry, a reload, another
        // file of the page the reader went to.
        self.0.events.loaded();
    }

    fn failed(&self, _navigation: Id, message: String) {
        self.0.events.failed(message);
    }

    fn terminated(&self) {
        self.0.events.crashed();
    }

    fn cancelled(&self, _url: &str) {
        let blocked = &self.0.blocked_navigations;
        blocked.set(blocked.get() + 1);
    }

    fn open_externally(&self, url: String) {
        self.0.events.open_externally(url);
    }

    fn alert(&self, message: String) {
        let now = Instant::now();
        let recent = self
            .0
            .last_alert
            .get()
            .is_some_and(|at| now.duration_since(at) < ALERT_EVERY);
        if !recent {
            self.0.last_alert.set(Some(now));
            self.0.events.alert(message);
        }
    }

    fn hovered_link(&self, link: Option<String>) {
        *self.0.hovered_link.borrow_mut() = link;
    }
}

const CLASS: &str = "WriterWebView";
const SHARED: &str = "writerShared";
/// The tag of "Open Link in Browser" among the owner's items.
const OPEN_LINK_TAG: isize = -7;

fn web_view_class() -> &'static Class {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let Some(mut decl) = ClassDecl::new(CLASS, class!(WKWebView)) else {
            return;
        };
        decl.add_ivar::<*mut c_void>(SHARED);
        // SAFETY: each function has the signature its selector's encoding says.
        unsafe {
            decl.add_method(sel!(keyDown:), key_down as extern "C" fn(&Object, Sel, Id));
            decl.add_method(sel!(keyUp:), key_up as extern "C" fn(&Object, Sel, Id));
            decl.add_method(sel!(mouseUp:), mouse_up as extern "C" fn(&Object, Sel, Id));
            decl.add_method(
                sel!(mouseDown:),
                mouse_down as extern "C" fn(&Object, Sel, Id),
            );
            decl.add_method(
                sel!(rightMouseDown:),
                right_mouse_down as extern "C" fn(&Object, Sel, Id),
            );
            decl.add_method(
                sel!(mouseMoved:),
                mouse_moved as extern "C" fn(&Object, Sel, Id),
            );
            decl.add_method(
                sel!(hitTest:),
                hit_test as extern "C" fn(&Object, Sel, NSPoint) -> Id,
            );
            decl.add_method(
                sel!(becomeFirstResponder),
                become_first_responder as extern "C" fn(&Object, Sel) -> BOOL,
            );
            decl.add_method(
                sel!(willOpenMenu:withEvent:),
                will_open_menu as extern "C" fn(&Object, Sel, Id, Id),
            );
            decl.add_method(
                sel!(writerMenuItem:),
                menu_item_chosen as extern "C" fn(&Object, Sel, Id),
            );
            decl.add_method(
                sel!(validateMenuItem:),
                validate_menu_item as extern "C" fn(&Object, Sel, Id) -> BOOL,
            );
            decl.add_method(sel!(dealloc), dealloc as extern "C" fn(&mut Object, Sel));
        }
        decl.register();
    });
    Class::get(CLASS).expect("the web view's class")
}

/// A web view (owned by the caller: +1) of `configuration`, with `shared` behind it for
/// a view of the window.
pub(crate) unsafe fn new_web_view(
    frame: NSRect,
    configuration: Id,
    shared: Option<Rc<ViewShared>>,
) -> Id {
    // SAFETY: WebKit's designated initializer; the instance variable is ours.
    unsafe {
        let web_view: Id = msg_send![web_view_class(), alloc];
        let web_view: Id = msg_send![web_view, initWithFrame: frame configuration: configuration];
        if !web_view.is_null() {
            let shared = shared.map_or(std::ptr::null_mut(), |shared| {
                Rc::into_raw(shared) as *mut c_void
            });
            (*web_view).set_ivar::<*mut c_void>(SHARED, shared);
        }
        web_view
    }
}

/// Takes the view of the window away from its web view: from now on it behaves as a
/// plain `WKWebView`.
pub(crate) unsafe fn detach(web_view: Id) {
    // SAFETY: the instance variable `new_web_view` set.
    unsafe {
        let shared = *(*web_view).get_ivar::<*mut c_void>(SHARED);
        if !shared.is_null() {
            (*web_view).set_ivar::<*mut c_void>(SHARED, std::ptr::null_mut());
            drop(Rc::from_raw(shared as *const ViewShared));
        }
    }
}

fn shared_of(this: &Object) -> Option<Rc<ViewShared>> {
    // SAFETY: the instance variable `new_web_view` set; it holds a strong reference
    // until `detach` or `dealloc`.
    unsafe {
        let shared = *this.get_ivar::<*mut c_void>(SHARED);
        if shared.is_null() {
            return None;
        }
        let shared = shared as *const ViewShared;
        Rc::increment_strong_count(shared);
        Some(Rc::from_raw(shared))
    }
}

extern "C" fn dealloc(this: &mut Object, _: Sel) {
    // SAFETY: what `detach` does, then WKWebView's dealloc.
    unsafe {
        detach(this as *mut Object);
        let _: () = msg_send![super(this, class!(WKWebView)), dealloc];
    }
}

/// The key of an `NSEvent`.
pub(crate) unsafe fn key_press(event: Id) -> KeyPress {
    // SAFETY: NSEvent's own selectors, on a key event.
    unsafe {
        let key_code: u16 = msg_send![event, keyCode];
        let characters: Id = msg_send![event, characters];
        let ignoring: Id = msg_send![event, charactersIgnoringModifiers];
        let flags: usize = msg_send![event, modifierFlags];
        KeyPress {
            key_code,
            characters: string_of(characters),
            characters_ignoring_modifiers: string_of(ignoring),
            cmd: flags & menu::COMMAND != 0,
            ctrl: flags & menu::CONTROL != 0,
            alt: flags & menu::OPTION != 0,
            shift: flags & menu::SHIFT != 0,
            function: flags & (1 << 23) != 0,
        }
    }
}

extern "C" fn key_down(this: &Object, _: Sel, event: Id) {
    // SAFETY: AppKit's event; GPUI's view is retained while the shared state names it.
    unsafe {
        if let Some(shared) = shared_of(this) {
            let press = key_press(event);
            let gpui_view = shared.gpui_view.get();
            if !gpui_view.is_null() && shared.events.takes_key(&press) {
                shared.taken_keys.borrow_mut().push(press.key_code);
                let _: () = msg_send![gpui_view, keyDown: event];
                return;
            }
            let at: f64 = msg_send![event, timestamp];
            shared.last_reader_event.set(Some(at));
        }
        let _: () = msg_send![super(this, class!(WKWebView)), keyDown: event];
    }
}

extern "C" fn key_up(this: &Object, _: Sel, event: Id) {
    // SAFETY: as `key_down`.
    unsafe {
        if let Some(shared) = shared_of(this) {
            let key_code: u16 = msg_send![event, keyCode];
            let taken = {
                let mut taken = shared.taken_keys.borrow_mut();
                match taken.iter().position(|&code| code == key_code) {
                    Some(index) => {
                        taken.remove(index);
                        true
                    }
                    None => false,
                }
            };
            let gpui_view = shared.gpui_view.get();
            if taken && !gpui_view.is_null() {
                let _: () = msg_send![gpui_view, keyUp: event];
                return;
            }
        }
        let _: () = msg_send![super(this, class!(WKWebView)), keyUp: event];
    }
}

/// Asks the page which link lies under the pointer of `event` (in a world of the
/// client, which the page's scripts cannot reach), before the click goes on to the
/// page: WebKit answers it before it builds the menu of the click.
unsafe fn ask_for_the_link(this: &Object, shared: &Rc<ViewShared>, event: Id) {
    // SAFETY: AppKit's and WebKit's own selectors on a live view and event.
    unsafe {
        *shared.pointed_link.borrow_mut() = None;
        let this = this as *const Object as Id;
        let point: NSPoint = msg_send![event, locationInWindow];
        let local: NSPoint = msg_send![this, convertPoint: point fromView: NIL];
        let bounds: NSRect = msg_send![this, bounds];
        let flipped: BOOL = msg_send![this, isFlipped];
        let (x, y) = frame::to_top_left(local.x, local.y, bounds.size.height, flipped != NO);
        let zoom: f64 = msg_send![this, pageZoom];
        let zoom = if zoom > 0. { zoom } else { 1. };
        let script = format!(
            "(() => {{ const e = document.elementFromPoint({}, {}); \
             const a = e && e.closest ? e.closest('a[href]') : null; return a ? String(a.href) : ''; }})()",
            x / zoom,
            y / zoom
        );
        let weak = Rc::downgrade(shared);
        let block = ConcreteBlock::new(move |result: Id, _error: Id| {
            if let Some(shared) = weak.upgrade() {
                let link = crate::page::js_text(result);
                *shared.pointed_link.borrow_mut() = Some(link).filter(|link| !link.is_empty());
            }
        })
        .copy();
        let world: Id = msg_send![class!(WKContentWorld), defaultClientWorld];
        let _: () = msg_send![this,
            evaluateJavaScript: nsstring(&script)
            inFrame: NIL
            inContentWorld: world
            completionHandler: &*block];
    }
}

extern "C" fn right_mouse_down(this: &Object, _: Sel, event: Id) {
    // SAFETY: AppKit's event.
    unsafe {
        if let Some(shared) = shared_of(this) {
            ask_for_the_link(this, &shared, event);
        }
        let _: () = msg_send![super(this, class!(WKWebView)), rightMouseDown: event];
    }
}

extern "C" fn mouse_down(this: &Object, _: Sel, event: Id) {
    // SAFETY: AppKit's event; a control-click is a right-click.
    unsafe {
        if let Some(shared) = shared_of(this) {
            let flags: usize = msg_send![event, modifierFlags];
            if flags & menu::CONTROL != 0 {
                ask_for_the_link(this, &shared, event);
            }
        }
        let _: () = msg_send![super(this, class!(WKWebView)), mouseDown: event];
    }
}

extern "C" fn mouse_up(this: &Object, _: Sel, event: Id) {
    // SAFETY: AppKit's event.
    unsafe {
        if let Some(shared) = shared_of(this) {
            let at: f64 = msg_send![event, timestamp];
            shared.last_reader_event.set(Some(at));
        }
        let _: () = msg_send![super(this, class!(WKWebView)), mouseUp: event];
    }
}

/// Whether a point of the superview's coordinates is inside one of the holes.
unsafe fn in_a_hole(this: &Object, shared: &ViewShared, point: NSPoint, from_window: bool) -> bool {
    let holes = shared.holes.borrow();
    if holes.is_empty() {
        return false;
    }
    // SAFETY: NSView's own selectors.
    unsafe {
        let this = this as *const Object as Id;
        let local: NSPoint = if from_window {
            msg_send![this, convertPoint: point fromView: NIL]
        } else {
            let superview: Id = msg_send![this, superview];
            msg_send![this, convertPoint: point fromView: superview]
        };
        let bounds: NSRect = msg_send![this, bounds];
        let flipped: BOOL = msg_send![this, isFlipped];
        let (x, y) = frame::to_top_left(local.x, local.y, bounds.size.height, flipped != NO);
        frame::in_a_hole(&holes, x, y)
    }
}

extern "C" fn hit_test(this: &Object, _: Sel, point: NSPoint) -> Id {
    // SAFETY: NSView's own; nil sends AppKit on to the next sibling.
    unsafe {
        if let Some(shared) = shared_of(this)
            && in_a_hole(this, &shared, point, false)
        {
            return NIL;
        }
        msg_send![super(this, class!(WKWebView)), hitTest: point]
    }
}

extern "C" fn mouse_moved(this: &Object, _: Sel, event: Id) {
    // SAFETY: AppKit's event. Over a hole the page does not see the pointer, so it
    // sets no cursor there: GPUI's view, under the hole, has the pointer.
    unsafe {
        if let Some(shared) = shared_of(this) {
            let point: NSPoint = msg_send![event, locationInWindow];
            if in_a_hole(this, &shared, point, true) {
                return;
            }
        }
        let _: () = msg_send![super(this, class!(WKWebView)), mouseMoved: event];
    }
}

extern "C" fn become_first_responder(this: &Object, _: Sel) -> BOOL {
    // SAFETY: NSResponder's own; WKWebView's answer is passed on.
    unsafe {
        let became: BOOL = msg_send![super(this, class!(WKWebView)), becomeFirstResponder];
        if became != NO
            && let Some(shared) = shared_of(this)
            && !shared.focusing.get()
        {
            shared.events.wants_focus();
        }
        became
    }
}

/// What the test of the menu sees (`testing::record_menus`).
#[derive(Default)]
pub(crate) struct MenuRecord {
    pub on: bool,
    /// Every menu WebKit handed over: its items (identifier, action, title) as they
    /// stood after the filter and the owner's additions.
    pub menus: Vec<Vec<(String, String, String)>>,
    /// The title of an item to choose in every menu recorded.
    pub choose: Option<String>,
}

thread_local! {
    pub(crate) static MENU_RECORD: RefCell<MenuRecord> = RefCell::new(MenuRecord::default());
}

unsafe fn item_identifier(item: Id) -> String {
    // SAFETY: NSMenuItem's (NSUserInterfaceItemIdentification) own.
    unsafe {
        if !ns::responds(item, sel!(identifier)) {
            return String::new();
        }
        let identifier: Id = msg_send![item, identifier];
        string_of(identifier)
    }
}

unsafe fn item_action(item: Id) -> String {
    // SAFETY: NSMenuItem's own; a selector (possibly NULL) and Foundation's name of it.
    unsafe {
        let action: *const c_void = msg_send![item, action];
        if action.is_null() {
            return String::new();
        }
        string_of(ns::NSStringFromSelector(action))
    }
}

extern "C" fn will_open_menu(this: &Object, _: Sel, menu_object: Id, event: Id) {
    // SAFETY: AppKit's menu and event; the items made here are released here (the
    // menu holds them).
    unsafe {
        let _: () =
            msg_send![super(this, class!(WKWebView)), willOpenMenu: menu_object withEvent: event];
        let Some(shared) = shared_of(this) else {
            return;
        };
        // WebKit's items that open, download, reload, inspect, go full screen or share.
        let count: isize = msg_send![menu_object, numberOfItems];
        let mut on_a_link = false;
        for index in (0..count).rev() {
            let item: Id = msg_send![menu_object, itemAtIndex: index];
            let identifier = item_identifier(item);
            let action = item_action(item);
            if identifier == "WKMenuItemIdentifierCopyLink"
                || identifier == "WKMenuItemIdentifierOpenLink"
            {
                on_a_link = true;
            }
            if menu::is_removed(Some(&identifier), Some(&action)) {
                let _: () = msg_send![menu_object, removeItemAtIndex: index];
            }
        }
        tidy_separators(menu_object);

        let link = if on_a_link {
            let pointed = shared.pointed_link.borrow().clone();
            pointed.or_else(|| shared.hovered_link.borrow().clone())
        } else {
            None
        };
        *shared.menu_link.borrow_mut() = link.clone();
        let entries = shared.events.menu_items(link.as_deref());
        let outside = link.as_deref().filter(|link| policy::opens_outside(link));
        let count: isize = msg_send![menu_object, numberOfItems];
        if (outside.is_some() || !entries.is_empty()) && count > 0 {
            let separator: Id = msg_send![class!(NSMenuItem), separatorItem];
            let _: () = msg_send![menu_object, addItem: separator];
        }
        let this_id = this as *const Object as Id;
        if outside.is_some() {
            add_item(
                menu_object,
                this_id,
                "Open Link in Browser",
                OPEN_LINK_TAG,
                true,
            );
        }
        let mut enabled = shared.menu_enabled.borrow_mut();
        enabled.clear();
        for entry in &entries {
            if entry.separator_before {
                let separator: Id = msg_send![class!(NSMenuItem), separatorItem];
                let _: () = msg_send![menu_object, addItem: separator];
            }
            enabled.insert(entry.id, entry.enabled);
            add_item(
                menu_object,
                this_id,
                &entry.title,
                entry.id as isize,
                entry.enabled,
            );
        }
        drop(enabled);
        tidy_separators(menu_object);

        let recorded = MENU_RECORD.with(|record| {
            let mut record = record.borrow_mut();
            if !record.on {
                return false;
            }
            let count: isize = msg_send![menu_object, numberOfItems];
            let mut items = Vec::new();
            for index in 0..count {
                let item: Id = msg_send![menu_object, itemAtIndex: index];
                let title: Id = msg_send![item, title];
                items.push((item_identifier(item), item_action(item), string_of(title)));
            }
            record.menus.push(items);
            true
        });
        if recorded {
            // A test: the item it asked for is chosen, as the reader would; then nothing
            // is shown (an empty menu does not open), and should it open anyway, its
            // tracking ends at once.
            let choose = MENU_RECORD.with(|record| record.borrow().choose.clone());
            if let Some(choose) = choose {
                let count: isize = msg_send![menu_object, numberOfItems];
                for index in 0..count {
                    let item: Id = msg_send![menu_object, itemAtIndex: index];
                    let title: Id = msg_send![item, title];
                    if string_of(title) == choose {
                        let _: () = msg_send![menu_object, performActionForItemAtIndex: index];
                        break;
                    }
                }
            }
            let _: () = msg_send![menu_object, removeAllItems];
            let menu_object = ns::retain(menu_object) as usize;
            ns::after(Duration::from_millis(30), move || {
                let menu_object = menu_object as Id;
                let _: () = msg_send![menu_object, cancelTrackingWithoutAnimation];
                ns::release(menu_object);
            });
        }
    }
}

/// No separator at either end of the menu and never two in a row.
unsafe fn tidy_separators(menu_object: Id) {
    // SAFETY: NSMenu's and NSMenuItem's own.
    unsafe {
        let mut index: isize = msg_send![menu_object, numberOfItems];
        let mut previous_was_separator = true; // the end of the menu
        while index > 0 {
            index -= 1;
            let item: Id = msg_send![menu_object, itemAtIndex: index];
            let separator: BOOL = msg_send![item, isSeparatorItem];
            if separator != NO {
                if previous_was_separator || index == 0 {
                    let _: () = msg_send![menu_object, removeItemAtIndex: index];
                    continue;
                }
                previous_was_separator = true;
            } else {
                previous_was_separator = false;
            }
        }
    }
}

unsafe fn add_item(menu_object: Id, target: Id, text: &str, tag: isize, enabled: bool) {
    let (title, key) = menu::title_and_key(text);
    let (key, mask) = key.unwrap_or_default();
    // SAFETY: NSMenuItem's own; the menu retains the item.
    unsafe {
        let item: Id = msg_send![class!(NSMenuItem), alloc];
        let item: Id = msg_send![item,
            initWithTitle: nsstring(&title)
            action: sel!(writerMenuItem:)
            keyEquivalent: nsstring(&key)];
        if !key.is_empty() {
            let _: () = msg_send![item, setKeyEquivalentModifierMask: mask];
        }
        let _: () = msg_send![item, setTarget: target];
        let _: () = msg_send![item, setTag: tag];
        let _: () = msg_send![item, setEnabled: ns::yes_no(enabled)];
        let _: () = msg_send![menu_object, addItem: item];
        ns::release(item);
    }
}

extern "C" fn menu_item_chosen(this: &Object, _: Sel, sender: Id) {
    // SAFETY: the menu item AppKit hands over.
    unsafe {
        let Some(shared) = shared_of(this) else {
            return;
        };
        let tag: isize = msg_send![sender, tag];
        if tag == OPEN_LINK_TAG {
            // The reader chose it in the menu: that is the reader's action.
            let link = shared.menu_link.borrow().clone();
            if let Some(link) = link.filter(|link| policy::opens_outside(link)) {
                shared.events.open_externally(link);
            }
        } else if tag >= 0 {
            shared.events.menu_chosen(tag as u32);
        }
    }
}

extern "C" fn validate_menu_item(this: &Object, _: Sel, item: Id) -> BOOL {
    // SAFETY: AppKit's menu item; WKWebView's own validation for every other item.
    unsafe {
        if item_action(item) == "writerMenuItem:" {
            let tag: isize = msg_send![item, tag];
            if tag == OPEN_LINK_TAG {
                return YES;
            }
            let enabled = shared_of(this)
                .and_then(|shared| shared.menu_enabled.borrow().get(&(tag as u32)).copied())
                .unwrap_or(false);
            return ns::yes_no(enabled);
        }
        let wk = class!(WKWebView);
        if wk.instance_method(sel!(validateMenuItem:)).is_some() {
            return msg_send![super(this, wk), validateMenuItem: item];
        }
        if wk
            .instance_method(sel!(validateUserInterfaceItem:))
            .is_some()
        {
            return msg_send![super(this, wk), validateUserInterfaceItem: item];
        }
        YES
    }
}
