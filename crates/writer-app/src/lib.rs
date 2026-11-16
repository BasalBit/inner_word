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
//! | [`palette`], [`quick_open`], [`library_search`] | the picker modals |
//! | [`keymap`] | the keymap as the app's own thing: the defaults every crate bound, the user's changes (`writer-keymap`), the keymap that is installed, the lookups for texts that name a key |
//! | [`shortcuts`] | the rows of "Keyboard shortcuts": every command in every place it has a key or can be given one (the list itself is `writer-keymap-ui`, mounted by [`workspace`], which also stores the changes and has the export and import of the shortcuts file) |
//! | [`settings`], [`settings_view`] | setting keys and the typed snapshot; the settings modal, with the section for the open document |
//! | [`version_history`] | the snapshots of the open document, a preview, restore |
//! | [`assistant`] | the assistant in the app: which engine it runs on and whether it is there, the model and effort of its two chats, the conversations it keeps per document, what the user applies to the text, the fake engine of a scripted run (the chats themselves are `writer-assistant-quick` and `writer-assistant-ui`, mounted by [`workspace`]) |
//! | [`agents`], [`agents_view`] | the `writer` command line tool beside the app, the skill and the MCP server for the agents on this Mac; the Agents panel |
//! | [`live`] | agents that write while the app is open: the channel on which the `writer` tool hands the app a change, the thread that listens, the queue to the main thread (what the workspace does with a request is `workspace/live.rs`) |
//! | [`export`] | the export picker and flow (the only user of `writer-export`) |
//! | [`pdf_export`] | "Export as PDF…" and the picker's PDF rows: a document, or a folder as one file, made of the pages "Print…" makes (the same pass, slices and writer, `workspace/print.rs`), written into the file chosen first |
//! | [`printing`] | where a print job goes: the macOS print panel, a recorder in a test, a directory in a scripted run (the command "Print…" itself is `workspace/print.rs`; the pages are the editor's print pass, the PDF and the print operation `writer-print`'s) |
//! | [`images`], [`pasteboard`] | images: the provider the editor of the document gets (the bytes of a picture, a file, a paste or a drop made into stored bytes, off the main thread), the pictures for the exporters, the collector of bytes nothing names any more; the app's own look at the system pasteboard for pictures GPUI's clipboard does not show |
//! | [`document_io`] | stored form of a document, lenient loading |
//! | [`view_memory`] | the caret position and the chosen-title flag of a document, checked against the document as it is now |
//! | [`character_limit`] | the character-limit indicator, counted block by block |
//! | [`controls`] | the close control of a modal |
//! | [`welcome`] | the welcome document, the demo library for QA |
//! | [`window`] | main window options, stored bounds, the error window |
//! | [`menus`] | the native menu bar |
//! | [`single_instance`] | one running copy per library: the lock of the data directory |
//! | [`signals`] | `SIGTERM`, `SIGHUP` and `SIGINT` quit the way `cmd-q` does, saving first |
//! | [`press_and_hold`] | held keys repeat while Vim mode is on (the `ApplePressAndHoldEnabled` user default) |
//! | [`spelling`] | which spell service the workspaces use, the languages, the language picker |
//! | [`zoom`], [`pinch`] | the zoom of the text: its commands and keys, how it is stored, how large the display is (life size); a pinch on the trackpad, which AppKit has to be asked for |
//! | [`automation`], [`screenshot`] | the visual-QA script hook |

pub mod actions;
pub mod agents;
pub mod agents_view;
pub mod assistant;
pub mod automation;
pub mod character_limit;
pub mod commands;
pub mod controls;
pub mod document_io;
pub mod export;
pub mod font_choice;
pub mod images;
pub mod keymap;
pub mod library_search;
pub mod live;
pub mod menus;
pub mod page_export;
pub mod page_report;
pub mod pages;
pub mod palette;
pub mod pasteboard;
pub mod pdf_export;
pub mod pinch;
pub mod press_and_hold;
pub mod printing;
pub mod quick_open;
pub mod screenshot;
pub mod settings;
pub mod settings_view;
pub mod shortcuts;
pub mod signals;
pub mod single_instance;
pub mod spelling;
pub mod version_history;
pub mod view_memory;
pub mod welcome;
pub mod window;
pub mod workspace;
pub mod zoom;

use gpui::{Action as _, App};

pub use actions::{CloseWindow, Quit};
pub use settings::Settings;
pub use workspace::{SaveState, Workspace, WorkspaceEvent};

/// App-wide setup shared by the binary and the tests. In this order:
///
/// 1. theme ([`writer_ui::theme::init`], System until the stored setting is
///    read by the workspace);
/// 2. the app's context-free key bindings ([`actions::key_bindings`]);
/// 3. the widget, editor, library-panel, settings-modal, version-history
///    and agents-panel bindings, and those of the list of keyboard
///    shortcuts (later bindings win ties, so a focused widget keeps its own
///    keys);
/// 4. the comments, Vim and editor-tools bindings and global handlers. The
///    tools come after the comments: inside the find bar `cmd-alt-c` is
///    "case sensitive", and it must win over the window-wide "show or hide
///    comments". Then the chats of the assistant, whose editors (the
///    composer, the answers) bind keys that are the editor's own otherwise
///    (`cmd-enter`, `escape`, `tab`);
/// 5. the command registry: every crate's commands (see
///    [`register_commands`] for the three that are left out; the
///    assistant's are registered and hidden until a workspace finds its
///    engine);
/// 6. the keymap: what was bound so far becomes the defaults, and the
///    keymap that runs from here on is the one [`keymap`] installs (the
///    defaults with the user's changes, once a workspace has read them
///    from its library; before them the bindings that only give menu bar
///    items their key). **A crate binds its keys before this point**: a
///    binding added later is lost at the next change of a key;
/// 7. the app-global actions (quit, hide) and the menu bar;
/// 8. quit when the last window closes.
pub fn init(cx: &mut App) {
    writer_ui::theme::init(writer_ui::ThemeMode::System, cx);

    cx.bind_keys(actions::key_bindings());
    cx.bind_keys(zoom::key_bindings());
    writer_ui::init(cx);
    writer_editor::init(cx);
    writer_spell::init(cx);
    writer_library::init(cx);
    cx.bind_keys(settings_view::key_bindings());
    cx.bind_keys(version_history::key_bindings());
    cx.bind_keys(agents_view::key_bindings());
    // The list of keyboard shortcuts: after the widgets, whose text field
    // binds some of the same keys at the same depth.
    writer_keymap_ui::init(cx);
    writer_comments::init(cx);
    writer_vim_mode::init(cx);
    writer_editor_tools::init(cx);
    // The chats of the assistant have editors of their own (the composer,
    // the answers): their keys come after the editor's and the tools'.
    writer_assistant_ui::init(cx);
    writer_assistant_quick::init(cx);
    // An open page: its context menu's keys, and the find keys once more
    // (the tools bind them where a text is, which a page is not).
    cx.bind_keys(workspace::page_key_bindings());

    register_commands(cx);

    // Every crate has bound its keys and named its commands: the keys are
    // the defaults from here on, and the keymap that runs is the one
    // `keymap` installs (the defaults with the user's changes, behind the
    // menu bar's own bindings).
    keymap::take_defaults(cx);

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
/// and the list of keyboard shortcuts have the categories.
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
    commands::register(zoom::commands(), cx);
    // Beside "Agents": what an agent changed last, to undo and to go to.
    commands::register(live::commands(), cx);
    // Pages: importing them, and what works on an open one.
    commands::register(actions::page_commands(), cx);
    // The editor and its tools title their commands Like This; the rest of
    // the app says them like this.
    commands::register(commands::in_sentence_case(writer_editor::commands()), cx);
    commands::register(
        commands::in_sentence_case(writer_editor_tools::commands()),
        cx,
    );
    // Spelling: the checker's commands, then the app's "Spelling language"
    // in the same category.
    commands::register(commands::in_sentence_case(writer_spell::commands()), cx);
    commands::register(actions::spelling_commands(), cx);
    commands::register(writer_comments::commands(), cx);
    // The assistant's commands are listed only while the assistant is
    // shown: hidden until a workspace learns that its engine is there.
    commands::register(actions::assistant_commands(), cx);
    commands::register(
        commands::in_sentence_case(writer_assistant_ui::commands()),
        cx,
    );
    commands::set_category_hidden(actions::CATEGORY_ASSISTANT, true, cx);
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
