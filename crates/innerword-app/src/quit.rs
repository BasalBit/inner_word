//! Quitting the app the app's own way (`cmd-q`, Inner Word > Quit, a signal
//! in the terminal): a workspace that holds a whiteboard whose page has not
//! posted its last gesture first asks the page for it, saves, and only then
//! quits (`docs/whiteboard-design.md`, section 4.6, "Getting what the page
//! has not posted yet"). A quit the system starts (log out, the Dock) reaches
//! only the quit hooks, whose 100 ms cannot wait for WebKit.

use gpui::{App, Global};

use crate::workspace::{End, Workspace};

/// A quit was asked and is under way: a second one quits at once.
#[derive(Default)]
struct Requested(bool);
impl Global for Requested {}

/// Quits as `cmd-q` does: through every workspace window's
/// [`Workspace::end_then`], which saves what a whiteboard's page still holds
/// first; at once when there is none (the window that says the library is in
/// use) or when a quit is already under way.
pub fn request(cx: &mut App) {
    if cx.default_global::<Requested>().0 {
        cx.quit();
        return;
    }
    cx.set_global(Requested(true));
    let workspace = cx
        .windows()
        .into_iter()
        .find_map(|window| window.downcast::<Workspace>());
    let Some(workspace) = workspace else {
        cx.quit();
        return;
    };
    let ended = workspace.update(cx, |workspace, window, cx| {
        workspace.end_then(End::Quit, window, cx);
    });
    if ended.is_err() {
        cx.quit();
    }
}

/// Whether a quit is under way (tests).
pub fn requested(cx: &App) -> bool {
    cx.try_global::<Requested>()
        .is_some_and(|requested| requested.0)
}
