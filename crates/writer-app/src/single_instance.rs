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
//! The lock belongs to the open file, not to the file's content: when the
//! process ends, however it ends, the system releases it. A lock file left
//! behind by a crash is therefore never mistaken for a running copy. The
//! file holds the process id of the copy that has the lock, so the message
//! can name it and the other copy can be brought to the front.
//!
//! A copy built before this guard existed takes no lock and is not seen.

use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    os::fd::AsRawFd,
    path::{Path, PathBuf},
};

/// The name of the lock file in the data directory.
pub const LOCK_FILE_NAME: &str = "instance.lock";

/// The lock of a data directory, held by this process until it is dropped
/// (or the process ends).
#[derive(Debug)]
pub struct InstanceLock {
    file: File,
    path: PathBuf,
}

impl InstanceLock {
    /// The lock file.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for InstanceLock {
    fn drop(&mut self) {
        // Closing the file would release the lock too; saying it is clearer.
        // SAFETY: the descriptor is open for as long as `self.file` lives.
        unsafe {
            libc::flock(self.file.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

impl gpui::Global for InstanceLock {}

/// The copy that already has a data directory open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunningCopy {
    /// Its process id, when the lock file names one.
    pub pid: Option<u32>,
    /// The data directory both copies were asked to use.
    pub data_dir: PathBuf,
}

/// What [`acquire`] found.
#[derive(Debug)]
pub enum Acquired {
    /// This process now has the data directory to itself.
    Lock(InstanceLock),
    /// Another running copy has it.
    Taken(RunningCopy),
}

/// Takes the lock of `data_dir` for this process, creating the directory
/// when it does not exist yet.
///
/// `Ok(Acquired::Taken(..))` means another running copy holds it. An `Err`
/// means the lock could not be tried at all (the directory cannot be
/// created or written): the caller decides; the library will most likely
/// fail to open for the same reason and say so.
pub fn acquire(data_dir: &Path) -> io::Result<Acquired> {
    std::fs::create_dir_all(data_dir)?;
    let path = data_dir.join(LOCK_FILE_NAME);
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)?;
    // SAFETY: the descriptor belongs to `file`, which is open.
    let locked = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0;
    if !locked {
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::WouldBlock {
            return Err(error);
        }
        let mut text = String::new();
        file.read_to_string(&mut text).ok();
        return Ok(Acquired::Taken(RunningCopy {
            pid: text.trim().parse().ok(),
            data_dir: data_dir.to_path_buf(),
        }));
    }
    // Ours: say who we are. A failure to write the id does not undo the
    // lock; the message of a second copy is then only less specific.
    let written = file
        .set_len(0)
        .and_then(|()| file.seek(SeekFrom::Start(0)).map(|_| ()))
        .and_then(|()| writeln!(file, "{}", std::process::id()))
        .and_then(|()| file.flush());
    if let Err(error) = written {
        eprintln!(
            "The Writer: could not write {}: {error}",
            path.to_string_lossy()
        );
    }
    Ok(Acquired::Lock(InstanceLock { file, path }))
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
