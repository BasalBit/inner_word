//! A polite request to end the process is a request to quit.
//!
//! `SIGTERM` (what `kill`, a logout that has run out of patience and most
//! process supervisors send), `SIGHUP` (the terminal the app was started
//! from went away) and `SIGINT` (`ctrl-c` in that terminal) end a process
//! on the spot by default. For Inner Word that would drop whatever was
//! typed since the last autosave, up to a few seconds of text. With
//! [`quit_on_termination_signals`] they do what `cmd-q` does instead: the
//! app quits through GPUI, which runs the quit hooks, and the workspace's
//! hook saves the open document first.
//!
//! How: the signal handler may do almost nothing, so it only writes one
//! byte to a pipe ([`listen`]). A background task waits at the other end
//! and, when the byte arrives, asks the app to quit on the main thread.
//!
//! `SIGKILL` cannot be caught; what was typed since the last autosave is
//! lost then, as it is in a power cut.

use std::{
    io,
    os::fd::RawFd,
    sync::atomic::{AtomicI32, Ordering},
};

use gpui::{App, AppContext as _};

/// The write end of the pipe the handler writes to.
static PIPE_WRITE: AtomicI32 = AtomicI32::new(-1);

extern "C" fn on_signal(_: libc::c_int) {
    let fd = PIPE_WRITE.load(Ordering::Relaxed);
    if fd >= 0 {
        let byte = 1u8;
        // SAFETY: `write` is async-signal-safe; the descriptor stays open
        // for the life of the process. A full pipe (EAGAIN) is fine: a
        // byte is already waiting.
        unsafe {
            libc::write(fd, (&raw const byte).cast(), 1);
        }
    }
}

/// Catches `signals` and returns the read end of a pipe that gets one byte
/// for every signal caught. A signal that the process was started with as
/// "ignored" stays ignored (a shell does that to `SIGINT` for background
/// jobs).
///
/// Call it once per process: a second call returns the same pipe.
pub fn listen(signals: &[libc::c_int]) -> io::Result<RawFd> {
    static PIPE_READ: AtomicI32 = AtomicI32::new(-1);
    let mut read = PIPE_READ.load(Ordering::SeqCst);
    if read < 0 {
        let mut fds = [0 as libc::c_int; 2];
        // SAFETY: `fds` has room for the two descriptors.
        if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        // The handler must never block: make the write end non-blocking.
        // SAFETY: the descriptors were just created.
        unsafe {
            let flags = libc::fcntl(fds[1], libc::F_GETFL);
            libc::fcntl(fds[1], libc::F_SETFL, flags | libc::O_NONBLOCK);
            libc::fcntl(fds[0], libc::F_SETFD, libc::FD_CLOEXEC);
            libc::fcntl(fds[1], libc::F_SETFD, libc::FD_CLOEXEC);
        }
        read = fds[0];
        PIPE_READ.store(read, Ordering::SeqCst);
        PIPE_WRITE.store(fds[1], Ordering::SeqCst);
    }
    for &signal in signals {
        // SAFETY: installing a handler that only calls `write`.
        unsafe {
            let handler = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
            let previous = libc::signal(signal, handler);
            if previous == libc::SIG_IGN {
                libc::signal(signal, libc::SIG_IGN);
            }
        }
    }
    Ok(read)
}

/// Blocks until a byte arrives at `fd` (see [`listen`]). Returns `false`
/// when the pipe failed instead.
pub fn wait(fd: RawFd) -> bool {
    let mut byte = 0u8;
    loop {
        // SAFETY: reading one byte into a one-byte buffer.
        let read = unsafe { libc::read(fd, (&raw mut byte).cast(), 1) };
        if read == 1 {
            return true;
        }
        if read < 0 && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
            continue;
        }
        return false;
    }
}

/// From now on `SIGTERM`, `SIGHUP` and `SIGINT` quit the app the way
/// `cmd-q` does, so the open document is saved first. For the real app
/// only (`main.rs`): the wait occupies a background thread for the life of
/// the process.
pub fn quit_on_termination_signals(cx: &mut App) {
    let fd = match listen(&[libc::SIGTERM, libc::SIGHUP, libc::SIGINT]) {
        Ok(fd) => fd,
        Err(error) => {
            eprintln!("Inner Word: cannot watch for termination signals: {error}");
            return;
        }
    };
    let signalled = cx.background_spawn(async move { wait(fd) });
    cx.spawn(async move |cx| {
        if signalled.await {
            // As `cmd-q`: an open whiteboard is asked for its last gesture first.
            cx.update(crate::quit::request).ok();
        }
    })
    .detach();
}
