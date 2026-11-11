//! The Writer application crate. `main.rs` is a thin entry point; the pieces
//! live here so examples and tests can reuse them.

pub mod automation;
pub mod data_dir;
pub mod screenshot;
pub mod window;

use gpui::{App, KeyBinding, Menu, MenuItem, actions};

actions!(
    app,
    [
        /// Quit The Writer.
        Quit,
        /// Close the current window.
        CloseWindow,
    ]
);

/// App-wide setup shared by the binary and the examples: theme, quit
/// handling, the application menu.
pub fn init(cx: &mut App) {
    writer_ui::theme::init(writer_ui::ThemeMode::System, cx);

    cx.bind_keys([
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-w", CloseWindow, None),
    ]);
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.set_menus(vec![Menu {
        name: "The Writer".into(),
        items: vec![MenuItem::action("Quit The Writer", Quit)],
    }]);

    // A single-window app: closing the last window quits (and runs the
    // on_app_quit hooks, which is where documents are flushed).
    cx.on_window_closed(|cx| {
        if cx.windows().is_empty() {
            cx.quit();
        }
    })
    .detach();
}
