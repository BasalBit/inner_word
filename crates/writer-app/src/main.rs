//! The Writer: entry point.

use gpui::{AnyWindowHandle, App, AppContext, Application};
use writer_app::{automation, welcome, window};
use writer_library::Library;

fn main() {
    let script = match automation::Script::from_env() {
        Ok(script) => script,
        Err(err) => {
            eprintln!(
                "The Writer: invalid {}: {err:#}",
                automation::AUTOMATION_ENV
            );
            std::process::exit(2);
        }
    };
    if std::env::var_os(writer_library::DATA_DIR_ENV).is_some_and(|dir| !dir.is_empty())
        && let Ok(dir) = writer_library::data_dir()
    {
        eprintln!("The Writer: using data directory {}", dir.display());
    }

    Application::new()
        .with_assets(writer_ui::assets::Assets)
        .run(move |cx: &mut App| {
            writer_app::init(cx);
            // Held keys repeat while Vim mode is on: the real app stores
            // that in its user defaults. A scripted run leaves the user's
            // preferences alone unless it is told otherwise.
            let scripted_opt_in = std::env::var_os(writer_app::press_and_hold::MANAGE_ENV)
                .is_some_and(|value| !value.is_empty());
            if script.is_none() || scripted_opt_in {
                writer_app::press_and_hold::manage_user_default(cx);
            }
            let Some(main_window) = open_window(cx) else {
                cx.quit();
                return;
            };
            match script {
                // A scripted run must not take the keyboard away from
                // whoever is using the machine: their typing would land in
                // this window. Keystrokes of the script are dispatched
                // inside the app and do not need it to be frontmost.
                Some(script) => {
                    if automation::foreground_requested() {
                        cx.activate(true);
                    }
                    automation::run(script, main_window, cx);
                }
                None => cx.activate(true),
            }
        });
}

/// Opens the main window, or the message window when the library cannot be
/// opened. `None` when no window could be opened at all.
fn open_window(cx: &mut App) -> Option<AnyWindowHandle> {
    let library = match Library::open_default() {
        Ok(library) => library,
        Err(error) => {
            eprintln!("The Writer: cannot open the library: {error}");
            let (title, message, detail) = window::library_error_text(&error);
            return match window::open_error_window(title, message, detail, cx) {
                Ok(handle) => Some(handle.into()),
                Err(err) => {
                    eprintln!("The Writer: cannot open a window: {err:#}");
                    None
                }
            };
        }
    };
    let library = cx.new(|_| library);
    if welcome::demo_seed_requested() {
        seed_demo(&library, cx);
    }
    match window::open_main_window(library, cx) {
        Ok(handle) => Some(handle.into()),
        Err(err) => {
            eprintln!("The Writer: cannot open the main window: {err:#}");
            None
        }
    }
}

/// `THE_WRITER_SEED_DEMO`: fill an empty library with the demo documents.
fn seed_demo(library: &gpui::Entity<Library>, cx: &mut App) {
    let empty = {
        let library = library.read(cx);
        let tree = library.tree();
        tree.folders.is_empty() && tree.documents.is_empty()
    };
    if !empty {
        return;
    }
    if let Err(error) = library.update(cx, welcome::seed_demo_library) {
        eprintln!("The Writer: could not create the demo library: {error}");
    }
}
