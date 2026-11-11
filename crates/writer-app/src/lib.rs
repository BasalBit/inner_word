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
//! | [`settings`], [`settings_view`] | setting keys and the typed snapshot; the settings modal, with the section for the open document |
//! | [`version_history`] | the snapshots of the open document, a preview, restore |
//! | [`export`] | the export picker and flow (the only user of `writer-export`) |
//! | [`document_io`] | stored form of a document, lenient loading |
//! | [`character_limit`] | the character-limit indicator, counted block by block |
//! | [`controls`] | tooltips and the close control of a modal: the small things for the pointer |
//! | [`welcome`] | the welcome document, the demo library for QA |
//! | [`window`] | main window options, stored bounds, the error window |
//! | [`menus`] | the native menu bar |
//! | [`single_instance`] | one running copy per library: the lock of the data directory |
//! | [`signals`] | `SIGTERM`, `SIGHUP` and `SIGINT` quit the way `cmd-q` does, saving first |
//! | [`press_and_hold`] | held keys repeat while Vim mode is on (the `ApplePressAndHoldEnabled` user default) |
//! | [`automation`], [`screenshot`] | the visual-QA script hook |

pub mod actions;
pub mod automation;
pub mod character_limit;
pub mod commands;
pub mod controls;
pub mod document_io;
pub mod export;
pub mod library_search;
pub mod menus;
pub mod palette;
pub mod press_and_hold;
pub mod quick_open;
pub mod screenshot;
pub mod settings;
pub mod settings_view;
pub mod shortcuts;
pub mod signals;
pub mod single_instance;
pub mod version_history;
pub mod welcome;
pub mod window;
pub mod workspace;

use gpui::{Action as _, App};

pub use actions::{CloseWindow, Quit};
pub use settings::Settings;
pub use workspace::{SaveState, Workspace, WorkspaceEvent};

/// App-wide setup shared by the binary and the tests. In this order:
///
/// 1. theme ([`writer_ui::theme::init`], System until the stored setting is
///    read by the workspace);
/// 2. the app's context-free key bindings ([`actions::key_bindings`]);
/// 3. the widget, editor, library-panel, settings-modal and
///    version-history bindings (later bindings win ties, so a focused
///    widget keeps its own keys);
/// 4. the comments, Vim and editor-tools bindings and global handlers. The
///    tools come last: inside the find bar `cmd-alt-c` is "case sensitive",
///    and it must win over the window-wide "show or hide comments";
/// 5. the command registry: every crate's commands (see
///    [`register_commands`] for the three that are left out);
/// 6. the app-global actions (quit, hide) and the menu bar;
/// 7. quit when the last window closes.
pub fn init(cx: &mut App) {
    writer_ui::theme::init(writer_ui::ThemeMode::System, cx);

    cx.bind_keys(actions::key_bindings());
    writer_ui::init(cx);
    writer_editor::init(cx);
    writer_library::init(cx);
    cx.bind_keys(settings_view::key_bindings());
    cx.bind_keys(version_history::key_bindings());
    writer_comments::init(cx);
    writer_vim_mode::init(cx);
    writer_editor_tools::init(cx);

    register_commands(cx);

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

/// Fills the command registry from every crate, in the order the palette
/// and the shortcuts overlay list the categories.
///
/// Three published commands are left out, because the app has the same
/// thing and the palette should not list it twice:
///
/// - the Vim crate's "Toggle Vim mode": the app's own toggle
///   ([`actions::ToggleVimMode`], "Turn Vim mode on or off") is the one
///   bound to `cmd-alt-v` and the one that stores the setting;
/// - the library panel's "New document" and "New folder": the app's
///   [`actions::NewDocument`] (`cmd-n`) and [`actions::NewFolder`]
///   (`cmd-shift-n`) work from anywhere. The panel's own keys (`n`,
///   `shift-n` in the sidebar) still work and are listed in the shortcuts
///   overlay under "In the library".
fn register_commands(cx: &mut App) {
    commands::register(actions::commands(), cx);
    // The editor and its tools title their commands Like This; the rest of
    // the app says them like this.
    commands::register(commands::in_sentence_case(writer_editor::commands()), cx);
    commands::register(
        commands::in_sentence_case(writer_editor_tools::commands()),
        cx,
    );
    commands::register(writer_comments::commands(), cx);
    commands::register(
        writer_vim_mode::commands()
            .into_iter()
            .filter(|command| command.action_name() != writer_vim_mode::ToggleVimMode.name()),
        cx,
    );
    commands::register(
        writer_library::commands().into_iter().filter(|command| {
            let name = command.action_name();
            name != writer_library::NewDocument.name() && name != writer_library::NewFolder.name()
        }),
        cx,
    );
}
