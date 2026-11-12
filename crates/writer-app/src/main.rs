//! The Writer: entry point.

use gpui::{AnyWindowHandle, App, AppContext, Application};
use writer_app::{
    automation,
    single_instance::{self, Acquired},
    welcome, window,
};
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
            // The Mac's own spell checker, and the check mark of "Check
            // Spelling While Typing" in the real menu bar. Only here: tests
            // install a fake service and have no menu bar.
            writer_app::spelling::use_system_service(cx);
            // How large this Mac's displays are: "Life Size", and the zoom a
            // page opens at. Only read.
            writer_app::zoom::use_this_mac(cx);
            writer_app::menus::manage_check_marks(cx);
            // kill, a closed terminal and ctrl-c quit like cmd-q: the open
            // document is saved first.
            writer_app::signals::quit_on_termination_signals(cx);
            // Held keys repeat while Vim mode is on: the real app stores
            // that in its user defaults. A scripted run leaves the user's
            // preferences alone unless it is told otherwise.
            let scripted_opt_in = std::env::var_os(writer_app::press_and_hold::MANAGE_ENV)
                .is_some_and(|value| !value.is_empty());
            if script.is_none() || scripted_opt_in {
                writer_app::press_and_hold::manage_user_default(cx);
            }
            // What the Agents panel looks at and writes to: this Mac. A
            // scripted run gets a home directory of its own or none.
            writer_app::agents::use_this_mac(script.is_some(), cx);
            // The assistant runs on Claude Code with the login the user
            // has in it. A scripted run never gets it (a script must not
            // be able to send a message: each one spends the user's own
            // allowance); it gets the fake engine when it asks for one,
            // and no assistant otherwise.
            if script.is_none() {
                writer_app::assistant::use_claude_code(cx);
            } else if let Some(fake) = writer_app::assistant::fake::from_env() {
                writer_app::assistant::fake::install(fake, cx);
            } else if writer_app::assistant::real_engine_requested() {
                // Asked for in so many words: see `assistant::REAL_ENV`.
                eprintln!(
                    "The Writer: this scripted run uses the real Claude Code; every message it \
                     sends spends the user's allowance"
                );
                writer_app::assistant::use_claude_code(cx);
            }
            // A scripted run stays in the background, also when it finds
            // another copy running.
            let may_activate = script.is_none() || automation::foreground_requested();
            let Some(main_window) = open_window(may_activate, cx) else {
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

/// Opens the main window, or a message window when another copy of The
/// Writer has the library open or when the library cannot be opened.
/// `None` when no window could be opened at all.
fn open_window(may_activate: bool, cx: &mut App) -> Option<AnyWindowHandle> {
    // One running copy per library: two would overwrite each other's text
    // (see `single_instance`). The lock is held until the process ends. A
    // command line tool that is writing right now is waited for.
    // The data directory, once this process holds its lock.
    let mut locked = None;
    if let Ok(data_dir) = writer_library::data_dir() {
        match single_instance::acquire(&data_dir) {
            Ok(Acquired::Lock(lock)) => {
                cx.set_global(single_instance::HeldLock(lock));
                locked = Some(data_dir);
            }
            Ok(Acquired::Taken(running)) => {
                let (title, message, detail) = window::already_running_text(&running);
                eprintln!("The Writer: {title}. {message}\n{detail}");
                return match window::open_already_running_window(&running, may_activate, cx) {
                    Ok(handle) => Some(handle.into()),
                    Err(err) => {
                        eprintln!("The Writer: cannot open a window: {err:#}");
                        None
                    }
                };
            }
            // The folder cannot be made or written: the library will say so.
            Err(error) => eprintln!(
                "The Writer: could not check for another running copy in {}: {error}",
                data_dir.display()
            ),
        }
    }
    let library = match Library::open_default() {
        Ok(library) => library,
        Err(error) => {
            eprintln!("The Writer: cannot open the library: {error}");
            // A tool that asks this copy for a change is told why it makes
            // none (the lock is held, and there is no library to change).
            if let Some(data_dir) = &locked {
                writer_app::live::open_without_library(data_dir, cx).ok();
            }
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
        Ok(handle) => {
            // Agents change the library through the app while it is open:
            // from here on the `writer` tool finds the app's channel in
            // the lock file. Without the lock there is no channel, and a
            // tool is told to ask for the app to be quit, as it always was.
            if let Some(data_dir) = &locked {
                writer_app::live::open_at_launch(data_dir, handle, cx);
            }
            // Two fingers pinched on the trackpad zoom the text.
            writer_app::pinch::watch(handle, cx);
            Some(handle.into())
        }
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
