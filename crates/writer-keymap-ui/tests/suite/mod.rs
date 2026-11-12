//! Helpers shared by the tests, and the tests by subject.

pub mod host;

mod closing;
mod import;
mod list;
mod pointer;
mod recording;
mod rows;

use gpui::{
    Bounds, Entity, Focusable, KeyUpEvent, Keystroke, Modifiers, MouseButton, Pixels,
    TestAppContext, VisualTestContext,
};
use writer_keymap::{KeySequence, Target};
use writer_keymap_ui::{KeymapView, Request};
use writer_ui::theme::{self, ThemeMode};

use host::Host;

/// A window with the host in it, the keyboard in its "document".
pub fn window(cx: &mut TestAppContext) -> (Entity<Host>, &mut VisualTestContext) {
    cx.update(|cx| {
        theme::init(ThemeMode::Light, cx);
        host::init(cx);
    });
    let (host, cx) = cx.add_window_view(|window, cx| {
        let host = Host::new(window, cx);
        let focus = host.focus_handle(cx);
        window.focus(&focus);
        host
    });
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    (host, cx)
}

/// The same with the view opened by its key.
pub fn open(cx: &mut TestAppContext) -> (Entity<Host>, Entity<KeymapView>, &mut VisualTestContext) {
    let (host, cx) = window(cx);
    cx.simulate_keystrokes("cmd-/");
    let view = host.read_with(cx, |host, _| host.view.clone().expect("the view is open"));
    (host, view, cx)
}

/// Presses and releases each key, as a keyboard does. (`simulate_keystrokes`
/// alone presses without ever letting go.)
pub fn press(cx: &mut VisualTestContext, keys: &str) {
    for key in keys.split_whitespace() {
        cx.simulate_keystrokes(key);
        cx.simulate_event(KeyUpEvent {
            keystroke: Keystroke::parse(key).unwrap(),
        });
    }
    cx.run_until_parked();
}

pub fn bounds(cx: &mut VisualTestContext, selector: &'static str) -> Bounds<Pixels> {
    cx.debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn"))
}

/// A click on what is drawn under `selector`.
pub fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let at = bounds(cx, selector).center();
    cx.simulate_click(at, Modifiers::none());
    cx.run_until_parked();
}

pub fn right_click(cx: &mut VisualTestContext, selector: &'static str) {
    let at = bounds(cx, selector).center();
    cx.simulate_mouse_down(at, MouseButton::Right, Modifiers::none());
    cx.simulate_mouse_up(at, MouseButton::Right, Modifiers::none());
    cx.run_until_parked();
}

/// The selector of the row of `target` (`keymap-row-<n>`), leaked: the
/// test context wants a `'static` name.
pub fn row_selector(
    view: &Entity<KeymapView>,
    target: &Target,
    cx: &mut VisualTestContext,
) -> &'static str {
    let row = view
        .read_with(cx, |view, _| view.row_of(target))
        .unwrap_or_else(|| panic!("{target} is not listed"));
    Box::leak(format!("keymap-row-{row}").into_boxed_str())
}

pub fn selector(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

pub fn key(text: &str) -> KeySequence {
    KeySequence::parse(text).unwrap()
}

pub fn bold() -> Target {
    Target::new("demo::Bold", Some("Editor"))
}

pub fn italic() -> Target {
    Target::new("demo::Italic", Some("Editor"))
}

pub fn version_history() -> Target {
    Target::new("demo::VersionHistory", None)
}

/// Selects the row of `target` (the list as it is, no query).
pub fn select(view: &Entity<KeymapView>, target: &Target, cx: &mut VisualTestContext) {
    let found = view.update(cx, |view, cx| view.select_target(target, cx));
    assert!(found, "{target} is not listed");
    cx.run_until_parked();
}

/// The keys the host's keymap gives `target` now, as text.
pub fn keys_of(host: &Entity<Host>, target: &Target, cx: &mut VisualTestContext) -> Vec<String> {
    host.read_with(cx, |host, _| {
        host.keymap
            .effective()
            .target(target)
            .map(|state| state.keys.iter().map(|key| key.keys.to_string()).collect())
            .unwrap_or_default()
    })
}

pub fn requests(host: &Entity<Host>, cx: &mut VisualTestContext) -> Vec<Request> {
    host.read_with(cx, |host, _| host.requests.clone())
}

pub fn describe(view: &Entity<KeymapView>, cx: &mut VisualTestContext) -> String {
    view.read_with(cx, |view, cx| view.describe(cx))
}
