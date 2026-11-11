//! The Writer application crate: the window, the workspace shell and every
//! app-level command. `main.rs` is a thin entry point; everything lives here
//! so tests (and the assembly of the remaining crates) can reuse it.
//!
//! # Startup
//!
//! ```ignore
//! Application::new()
//!     .with_assets(writer_ui::assets::Assets)
//!     .run(|cx| {
//!         writer_app::init(cx);                               // theme, bindings, commands, menus
//!         // further crates: bind their keys, then
//!         // writer_app::commands::register(their_crate::commands(), cx);
//!         let library = cx.new(|_| Library::open_default()?);
//!         let window = writer_app::window::open_main_window(library, cx)?;
//!         // mount views: see `workspace` for the mount points
//!     });
//! ```
//!
//! # Modules
//!
//! | Module | Contents |
//! |---|---|
//! | [`workspace`] | [`Workspace`]: layout, mount points, document session and autosave, app action handlers, status bar |
//! | [`actions`] | app actions, their key bindings and command descriptors |
//! | [`commands`] | the command registry every crate registers into |
//! | [`palette`], [`quick_open`], [`library_search`], [`shortcuts`] | the picker modals |
//! | [`settings`], [`settings_view`] | setting keys and the typed snapshot; the settings modal |
//! | [`export`] | the export picker and flow (the only user of `writer-export`) |
//! | [`document_io`] | stored form of a document, lenient loading |
//! | [`welcome`] | the welcome document, the demo library for QA |
//! | [`window`] | main window options, stored bounds, the error window |
//! | [`menus`] | the native menu bar |
//! | [`automation`], [`screenshot`] | the visual-QA script hook |

pub mod actions;
pub mod automation;
pub mod commands;
pub mod document_io;
pub mod export;
pub mod library_search;
pub mod menus;
pub mod palette;
pub mod quick_open;
pub mod screenshot;
pub mod settings;
pub mod settings_view;
pub mod shortcuts;
pub mod welcome;
pub mod window;
pub mod workspace;

use gpui::App;

pub use actions::{CloseWindow, Quit};
pub use settings::Settings;
pub use workspace::{SaveState, Workspace, WorkspaceEvent};

/// App-wide setup shared by the binary and the tests. In this order:
///
/// 1. theme ([`writer_ui::theme::init`], System until the stored setting is
///    read by the workspace);
/// 2. the app's context-free key bindings ([`actions::key_bindings`]);
/// 3. the widget, editor, library-panel and settings-modal bindings (later
///    bindings win ties, so a focused widget keeps its own keys);
/// 4. the command registry: this crate's, the editor's and the library
///    panel's commands;
/// 5. the app-global actions (quit, hide) and the menu bar;
/// 6. quit when the last window closes.
///
/// Crates mounted later bind their own keys and call
/// [`commands::register`]; call [`menus::install`] again afterwards if their
/// bindings should show in the menu bar.
pub fn init(cx: &mut App) {
    writer_ui::theme::init(writer_ui::ThemeMode::System, cx);

    cx.bind_keys(actions::key_bindings());
    writer_ui::init(cx);
    writer_editor::init(cx);
    writer_library::init(cx);
    cx.bind_keys(settings_view::key_bindings());

    commands::register(actions::commands(), cx);
    commands::register(writer_editor::commands(), cx);
    commands::register(writer_library::commands(), cx);

    cx.on_action(|_: &actions::Quit, cx| cx.quit());
    cx.on_action(|_: &actions::Hide, cx| cx.hide());
    cx.on_action(|_: &actions::HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &actions::ShowAll, cx| cx.unhide_other_apps());
    menus::install(cx);

    // A single-window app: closing the last window quits (and runs the
    // on_app_quit hooks, where documents are flushed once more).
    cx.on_window_closed(|cx| {
        if cx.windows().is_empty() {
            cx.quit();
        }
    })
    .detach();
}
