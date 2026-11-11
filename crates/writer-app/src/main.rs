//! The Writer: entry point.

use gpui::{App, Application};
use writer_app::{automation, data_dir, window};

fn main() {
    let data_dir = match data_dir::ensure_data_dir() {
        Ok(dir) => dir,
        Err(err) => {
            eprintln!("The Writer: cannot create the data directory: {err}");
            std::process::exit(1);
        }
    };
    if data_dir::is_overridden() {
        eprintln!("The Writer: using data directory {}", data_dir.display());
    }

    let script = match automation::Script::from_env() {
        Ok(script) => script,
        Err(err) => {
            eprintln!("The Writer: invalid {}: {err:#}", automation::AUTOMATION_ENV);
            std::process::exit(2);
        }
    };

    Application::new().run(move |cx: &mut App| {
        writer_app::init(cx);
        let main_window = match window::open_main_window(cx) {
            Ok(handle) => handle,
            Err(err) => {
                eprintln!("The Writer: cannot open the main window: {err:#}");
                cx.quit();
                return;
            }
        };
        cx.activate(true);
        if let Some(script) = script {
            automation::run(script, main_window.into(), cx);
        }
    });
}
