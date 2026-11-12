//! One running copy of The Writer per library.
//!
//! Each copy keeps the whole open document in memory and writes all of it
//! when it saves. Two copies on one library file would therefore overwrite
//! each other's text without either noticing: the later save wins, and
//! both windows say "Saved". That can happen without anyone meaning it,
//! with an installed copy running while another one is started from a
//! build folder, or with the same data directory named twice.
//!
//! So the first thing a launch does is take the lock of its data
//! directory: an exclusive `flock` on the file [`LOCK_FILE_NAME`] in it,
//! held for as long as the process lives. A second launch on the same
//! directory finds the lock taken, does not open the library, and shows a
//! window that says so (see `main.rs` and
//! [`crate::window::already_running_text`]).
//!
//! The lock itself is `writer_store::instance_lock`, shared with the
//! `writer` command line tool: the tool refuses to change the library
//! while the app holds the lock, and takes the lock itself for the moment
//! of each write. A launch that finds the lock held by the tool therefore
//! waits for it ([`TOOL_PATIENCE`]) instead of concluding that another
//! copy is open; a write takes a fraction of a second. A lock held by the
//! app is not waited for (beyond the quarter of a second the shared module
//! allows for a holder that has not written its line yet).
//!
//! The lock belongs to the open file, not to the file's content: when the
//! process ends, however it ends, the system releases it. A lock file left
//! behind by a crash is therefore never mistaken for a running copy. The
//! file names the process that has the lock, and says when it is the tool,
//! so the message can name it and the other copy can be brought to the
//! front.
//!
//! A copy built before this guard existed takes no lock and is not seen.

use std::{io, path::Path, time::Duration};

use writer_library::store::instance_lock;
pub use writer_library::store::instance_lock::{
    Acquired, HolderKind, InstanceLock, LOCK_FILE_NAME, RunningCopy,
};

/// How long a launch waits for the command line tool to finish the write
/// it holds the lock for, before it says that the library is in use.
pub const TOOL_PATIENCE: Duration = Duration::from_secs(3);

/// The lock of the data directory, kept by the app for as long as it runs.
#[derive(Debug)]
pub struct HeldLock(pub InstanceLock);

impl gpui::Global for HeldLock {}

/// Takes the lock of `data_dir` for this process, creating the directory
/// when it does not exist yet, and waiting up to [`TOOL_PATIENCE`] when
/// the command line tool has it for a write.
///
/// `Ok(Acquired::Taken(..))` means another process still holds it: a
/// running copy of the app, or (its `kind` says so) a tool that did not
/// let go in time. An `Err` means the lock could not be tried at all (the
/// directory cannot be created or written): the caller decides; the
/// library will most likely fail to open for the same reason and say so.
pub fn acquire(data_dir: &Path) -> io::Result<Acquired> {
    acquire_with_patience(data_dir, TOOL_PATIENCE)
}

/// [`acquire`] with another limit on the wait for a tool.
pub fn acquire_with_patience(data_dir: &Path, patience: Duration) -> io::Result<Acquired> {
    instance_lock::acquire_waiting(data_dir, HolderKind::App, patience)
}

/// Brings the copy with process id `pid` to the front. Returns whether a
/// running application with that id was found and asked.
#[cfg(target_os = "macos")]
// objc 0.2's msg_send! expands to a `cfg(feature = "cargo-clippy")` check.
#[allow(unexpected_cfgs)]
pub fn bring_to_front(pid: u32) -> bool {
    use objc::{
        class, msg_send,
        runtime::{BOOL, NO, Object},
        sel, sel_impl,
    };
    /// `NSApplicationActivateIgnoringOtherApps`.
    const IGNORING_OTHER_APPS: usize = 1 << 1;
    // SAFETY: plain message sends to AppKit classes on the main thread; a
    // nil result is checked before it is used.
    unsafe {
        let app: *mut Object = msg_send![
            class!(NSRunningApplication),
            runningApplicationWithProcessIdentifier: pid as i32
        ];
        if app.is_null() {
            return false;
        }
        let activated: BOOL = msg_send![app, activateWithOptions: IGNORING_OTHER_APPS];
        activated != NO
    }
}

/// Brings the copy with process id `pid` to the front. Not available on
/// this platform.
#[cfg(not(target_os = "macos"))]
pub fn bring_to_front(_pid: u32) -> bool {
    false
}
