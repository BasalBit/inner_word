//! The "Keyboard shortcuts" view in a window of its own, over a small
//! pretend app (the host of the tests, `tests/suite/host.rs`): for looking
//! at it and for visual QA.
//!
//! ```bash
//! cargo run -p writer-keymap-ui --example shortcuts
//! ```
//!
//! `cmd-/` opens and closes the view; every change made in it is installed
//! at once, the view's own keys included. `cmd-n` and `cmd-q` only count
//! (the numbers behind the view show that neither runs while a key is
//! recorded).
//!
//! Visual QA: set `THE_WRITER_AUTOMATION` (see `automation.rs`), e.g.
//! `THE_WRITER_AUTOMATION="wait:800; keys:cmd-/; screenshot:/tmp/k.png; quit"`.
//! The run stays in the background and captures only its own window.

mod automation;
#[path = "../../tests/suite/host.rs"]
mod host;

use gpui::{
    App, Application, Bounds, Focusable, WindowBounds, WindowOptions, prelude::*, px, size,
};
use writer_ui::theme::{self, ThemeMode};

use host::Host;

fn main() {
    Application::new()
        .with_assets(writer_ui::assets::Assets)
        .run(|cx: &mut App| {
            theme::init(ThemeMode::System, cx);
            host::init(cx);
            let bounds = Bounds::centered(None, size(px(980.), px(760.)), cx);
            let window = cx
                .open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(bounds)),
                        ..Default::default()
                    },
                    |window, cx| {
                        theme::observe_system_appearance(window, cx).detach();
                        let host = cx.new(|cx| Host::new(window, cx));
                        let focus = host.read(cx).focus_handle(cx);
                        window.focus(&focus);
                        host
                    },
                )
                .expect("the window opens");
            cx.on_window_closed(|cx| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            if std::env::var(automation::ENV).is_ok() {
                automation::run_from_env(window, cx);
            } else {
                cx.activate(true);
            }
        });
}
