//! The socket of the channel: where it lies, the app's listener, and the lines that go
//! over it.

use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use serde_json::Value;
use writer_store::instance_lock;

use super::{Answer, AnswerLine, LONGEST_WAIT, PROTOCOL_VERSION, Refusal, Request};
use crate::error::Error;

/// The longest path a socket may have here. The system's limit is 104 bytes with the
/// byte that ends it (macOS; Linux has 108).
const LONGEST_SOCKET_PATH: usize = 100;

/// The longest line that is read from the channel: a request can hold a book.
pub(crate) const LONGEST_LINE: usize = 256 * 1024 * 1024;

/// How long the listener waits for the next piece of a request before it gives the
/// connection up, so that one that never sends its line does not keep the others waiting.
const REQUEST_IDLE: Duration = Duration::from_secs(1);

/// How long a whole request may take to arrive.
const REQUEST_LIMIT: Duration = Duration::from_secs(30);

/// How long writing an answer may take.
const ANSWER_LIMIT: Duration = Duration::from_secs(5);

/// How often the listener renews the times of its socket. macOS clears out of the user's
/// temporary directory, every night, what was not used for three days
/// (`com.apple.bsd.dirhelper`, `CLEAN_FILES_OLDER_THAN_DAYS`), and a writer leaves the app
/// open for longer than that; connecting to a socket does not count as using it.
const KEEP_FRESH_EVERY: Duration = Duration::from_secs(3600);

/// The user this process runs as.
fn user_id() -> u32 {
    // SAFETY: `getuid` has no arguments and cannot fail.
    unsafe { libc::getuid() }
}

/// Whether a process with this id exists.
fn process_exists(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    // SAFETY: signal 0 sends nothing; it only asks whether the process could be sent one.
    let answered = unsafe { libc::kill(pid, 0) };
    answered == 0 || io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

/// `dir`, made sure to be a directory of this user that nobody else can enter. It is
/// created when it is missing; one that belongs to somebody else, or is a link, is refused.
fn private_dir(dir: &Path) -> io::Result<()> {
    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    let found = std::fs::symlink_metadata(dir)?;
    if !found.is_dir() || found.uid() != user_id() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{} is not a directory of this user", dir.display()),
        ));
    }
    if found.mode() & 0o077 != 0 {
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// The directory the sockets of this user lie in: `the-writer-<uid>` in the user's
/// temporary directory, or in `/tmp` when that one's path is too long for a socket.
fn socket_dir(longest_name: usize) -> io::Result<PathBuf> {
    let name = format!("the-writer-{}", user_id());
    let mut last = None;
    for base in [std::env::temp_dir(), PathBuf::from("/tmp")] {
        let dir = base.join(&name);
        if dir.as_os_str().len() + 1 + longest_name > LONGEST_SOCKET_PATH {
            continue;
        }
        match private_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(error) => last = Some(error),
        }
    }
    Err(last.unwrap_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "no directory with a path short enough for a socket",
        )
    }))
}

/// Removes the sockets that apps which no longer run left in `dir`.
fn sweep(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let pid = name
            .to_str()
            .and_then(|name| name.strip_suffix(".sock"))
            .and_then(|name| name.split_once('-'))
            .and_then(|(pid, _)| pid.parse::<u32>().ok());
        let is_socket = entry
            .file_type()
            .is_ok_and(|file_type| file_type.is_socket());
        if let Some(pid) = pid
            && is_socket
            && !process_exists(pid)
        {
            std::fs::remove_file(entry.path()).ok();
        }
    }
}

/// Reads one line (without its line break) from `stream`. `Ok(None)` when the other side
/// closed the connection without sending anything; an error of the kind
/// [`io::ErrorKind::UnexpectedEof`] when it closed in the middle of a line, and of the
/// kind [`io::ErrorKind::TimedOut`] when nothing came for `idle` or the line was not
/// whole by `deadline`.
pub(crate) fn read_line(
    stream: &mut UnixStream,
    idle: Duration,
    deadline: Instant,
) -> io::Result<Option<String>> {
    let mut line: Vec<u8> = Vec::new();
    let mut buffer = [0u8; 16 * 1024];
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(io::ErrorKind::TimedOut.into());
        }
        let wait = left.min(idle).max(Duration::from_millis(1));
        if stream.set_read_timeout(Some(wait)).is_err() {
            // macOS refuses to set an option on a socket whose other end is closed
            // already. What that end sent before it closed is still there to read: an
            // app that answers and hangs up at once is not one whose answer was lost.
            // Reading without waiting takes what is there and cannot hang.
            stream.set_nonblocking(true)?;
        }
        let count = match stream.read(&mut buffer) {
            Ok(count) => count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                return Err(io::ErrorKind::TimedOut.into());
            }
            Err(error) => return Err(error),
        };
        if count == 0 {
            return if line.is_empty() {
                Ok(None)
            } else {
                Err(io::ErrorKind::UnexpectedEof.into())
            };
        }
        if let Some(end) = buffer[..count].iter().position(|&byte| byte == b'\n') {
            line.extend_from_slice(&buffer[..end]);
            return String::from_utf8(line)
                .map(Some)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "not UTF-8"));
        }
        line.extend_from_slice(&buffer[..count]);
        if line.len() > LONGEST_LINE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "the line is too long",
            ));
        }
    }
}

/// Writes one line of JSON to `stream`.
pub(crate) fn write_line(
    stream: &mut UnixStream,
    line: &impl serde::Serialize,
    limit: Duration,
) -> io::Result<()> {
    let mut text = serde_json::to_string(line).map_err(io::Error::other)?;
    text.push('\n');
    stream.set_write_timeout(Some(limit.max(Duration::from_millis(1))))?;
    stream.write_all(text.as_bytes())?;
    stream.flush()
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Whether the process at the other end of `stream` runs as this user.
#[cfg(target_os = "macos")]
fn same_user(stream: &UnixStream) -> bool {
    use std::os::fd::AsRawFd;
    let (mut uid, mut gid): (libc::uid_t, libc::gid_t) = (0, 0);
    // SAFETY: the descriptor is open, and both pointers are to values of the right type.
    let answered = unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) };
    answered == 0 && uid == user_id()
}

/// Whether the process at the other end of `stream` runs as this user. Elsewhere the
/// directory the socket lies in is what keeps other users out.
#[cfg(not(target_os = "macos"))]
fn same_user(_stream: &UnixStream) -> bool {
    true
}

/// Tells a tool whose request nobody will read that the app is going away.
fn say_quitting(stream: &mut UnixStream) {
    let line = AnswerLine {
        version: PROTOCOL_VERSION,
        answer: Answer::refused(Refusal::Quitting),
    };
    write_line(stream, &line, Duration::from_millis(100)).ok();
}

/// Sets the times of the file at `path` (a socket, a directory) to now.
fn freshen(path: &Path) {
    use std::os::unix::ffi::OsStrExt;
    let Ok(path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return;
    };
    // SAFETY: `path` is a string that ends in a zero byte, and no times are passed
    // (which means: now).
    unsafe {
        libc::utimes(path.as_ptr(), std::ptr::null());
    }
}

/// What the listener and its closers share.
#[derive(Debug)]
struct Shared {
    listener: UnixListener,
    path: PathBuf,
    data_dir: PathBuf,
    closed: AtomicBool,
    /// Wakes the thread that keeps the socket fresh, so that it ends with the listener.
    over: (Mutex<bool>, Condvar),
}

impl Shared {
    /// Keeps the socket (and the directory it lies in) from looking unused, for as long
    /// as the listener is open. Runs on a thread of its own.
    fn keep_fresh(&self, every: Duration) {
        let (over, wake) = &self.over;
        let mut is_over = locked(over);
        while !*is_over {
            is_over = wake
                .wait_timeout(is_over, every)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
            if !*is_over {
                freshen(&self.path);
                if let Some(dir) = self.path.parent() {
                    freshen(dir);
                }
            }
        }
    }

    fn close(&self) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        *locked(&self.over.0) = true;
        self.over.1.notify_all();
        // From now on the tools are not sent here.
        instance_lock::announce_channel(&self.data_dir, None).ok();
        // Those that are waiting in the queue are told, so that none of them waits for
        // an app that went away.
        if self.listener.set_nonblocking(true).is_ok() {
            while let Ok((mut stream, _)) = self.listener.accept() {
                say_quitting(&mut stream);
            }
        }
        // Whoever waits in `accept` wakes up for this connection and sees that it is over.
        drop(UnixStream::connect(&self.path));
        std::fs::remove_file(&self.path).ok();
    }
}

/// The app's end of the channel: the socket tools hand their requests to.
///
/// Bound once the app holds the lock of its data directory. [`Listener::accept`] is a
/// plain blocking call for a thread of its own; [`Closer::close`] ends it from anywhere.
/// Dropping the listener closes it too.
#[derive(Debug)]
pub struct Listener {
    shared: Arc<Shared>,
}

impl Listener {
    /// Opens the socket and writes its path into the lock file of `data_dir`, where the
    /// tools look for it.
    ///
    /// The caller is the app and holds the lock of `data_dir`
    /// (`writer_store::instance_lock`): binding fails with
    /// [`io::ErrorKind::PermissionDenied`] otherwise. An app that cannot bind goes on
    /// without the channel; tools are then told that it takes no changes.
    pub fn bind(data_dir: &Path) -> io::Result<Listener> {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let pid = std::process::id();
        let name = format!("{pid}-{}.sock", COUNTER.fetch_add(1, Ordering::Relaxed));
        let dir = socket_dir(name.len().max(24))?;
        sweep(&dir);
        let path = dir.join(name);
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let listener = UnixListener::bind(&path)?;
        let ready = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .and_then(|()| instance_lock::announce_channel(data_dir, Some(&path)));
        if let Err(error) = ready {
            std::fs::remove_file(&path).ok();
            return Err(error);
        }
        let shared = Arc::new(Shared {
            listener,
            path,
            data_dir: data_dir.to_path_buf(),
            closed: AtomicBool::new(false),
            over: (Mutex::new(false), Condvar::new()),
        });
        let kept = shared.clone();
        // Without the thread the socket is as good as any file in a temporary directory:
        // good for three days.
        std::thread::Builder::new()
            .name("writer-live-socket".into())
            .spawn(move || kept.keep_fresh(KEEP_FRESH_EVERY))
            .ok();
        Ok(Listener { shared })
    }

    /// The path of the socket.
    pub fn path(&self) -> &Path {
        &self.shared.path
    }

    /// A handle that closes the listener from another thread.
    pub fn closer(&self) -> Closer {
        Closer {
            shared: self.shared.clone(),
        }
    }

    /// Waits for the next request and returns it, read and checked; `None` once the
    /// listener is closed.
    ///
    /// What cannot be a request is answered here and never returned: a line that cannot
    /// be read, a version of the protocol this build does not speak
    /// ([`Refusal::Protocol`]), a connection of another user. A connection that sends
    /// nothing for a second is given up.
    ///
    /// The requests are returned in the order they arrived; whoever takes them answers
    /// them one after the other, and none is lost: a tool that connects while another
    /// request is being carried out waits its turn in the socket's queue.
    pub fn accept(&self) -> Option<Incoming> {
        loop {
            if self.shared.closed.load(Ordering::SeqCst) {
                return None;
            }
            let mut stream = match self.shared.listener.accept() {
                Ok((stream, _)) => stream,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => {
                    if self.shared.closed.load(Ordering::SeqCst) {
                        return None;
                    }
                    // Out of descriptors, most likely: not for long.
                    std::thread::sleep(Duration::from_millis(50));
                    continue;
                }
            };
            if self.shared.closed.load(Ordering::SeqCst) {
                // The connection that woke this thread up, or a tool that came with it.
                say_quitting(&mut stream);
                return None;
            }
            // A connection taken over from a listener that does not block would not
            // block either.
            stream.set_nonblocking(false).ok();
            if !same_user(&stream) {
                continue;
            }
            let arrived = Instant::now();
            let line = match read_line(&mut stream, REQUEST_IDLE, arrived + REQUEST_LIMIT) {
                Ok(Some(line)) => line,
                // Nothing was sent, or not a whole line: there is nothing to answer.
                Ok(None) | Err(_) => continue,
            };
            match parse_request(&line) {
                Ok(request) => return Some(Incoming::new(request, stream)),
                Err(answer) => {
                    write_line(&mut stream, &answer, ANSWER_LIMIT).ok();
                }
            }
        }
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.shared.close();
    }
}

/// Reads a request, or says what to answer to a line that is none.
fn parse_request(line: &str) -> Result<Request, Box<AnswerLine>> {
    let answer = |answer: Answer| {
        Box::new(AnswerLine {
            version: PROTOCOL_VERSION,
            answer,
        })
    };
    let unreadable = |error: &dyn std::fmt::Display| {
        answer(Answer::failed(&Error::invalid(format!(
            "the request cannot be read: {error}"
        ))))
    };
    let value: Value = serde_json::from_str(line).map_err(|error| unreadable(&error))?;
    let version = value
        .get("version")
        .and_then(Value::as_u64)
        .ok_or_else(|| unreadable(&"it names no version of the protocol"))?;
    if version != u64::from(PROTOCOL_VERSION) {
        return Err(answer(Answer::refused(Refusal::Protocol {
            app: PROTOCOL_VERSION,
            tool: u32::try_from(version).unwrap_or(u32::MAX),
        })));
    }
    serde_json::from_value(value).map_err(|error| unreadable(&error))
}

/// Closes a [`Listener`] from anywhere: the app's quit hook has one.
#[derive(Clone, Debug)]
pub struct Closer {
    shared: Arc<Shared>,
}

impl Closer {
    /// Closes the listener: its [`accept`](Listener::accept) returns `None`, the socket
    /// is removed, and the lock file no longer names it, so tools stop coming. Quick, and
    /// safe to call again.
    pub fn close(&self) {
        self.shared.close();
    }

    /// Whether the listener is closed.
    pub fn is_closed(&self) -> bool {
        self.shared.closed.load(Ordering::SeqCst)
    }
}

/// How far a request has come.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Stage {
    /// Nobody has taken it up yet.
    Waiting,
    /// The app is carrying it out.
    Begun,
    /// The tool has its answer (from the app, or because the time ran out).
    Answered,
}

#[derive(Debug)]
struct Slot {
    state: Mutex<(Stage, UnixStream)>,
    moved: Condvar,
}

impl Slot {
    /// Sends `answer` unless the tool has one already.
    fn answer(&self, answer: Answer) -> io::Result<()> {
        let mut state = locked(&self.state);
        if state.0 == Stage::Answered {
            return Ok(());
        }
        state.0 = Stage::Answered;
        self.moved.notify_all();
        let line = AnswerLine {
            version: PROTOCOL_VERSION,
            answer,
        };
        let written = write_line(&mut state.1, &line, ANSWER_LIMIT);
        state.1.shutdown(std::net::Shutdown::Both).ok();
        written
    }
}

/// A request that arrived, with the connection its answer goes back on.
///
/// It travels from the thread that accepted it to the thread that carries it out
/// (`Send`). It is answered exactly once: by [`Incoming::answer`], by the clock when the
/// tool's patience runs out before anybody began ([`Refusal::TooLate`]), or, when it is
/// dropped without an answer, with [`Refusal::Quitting`].
#[derive(Debug)]
pub struct Incoming {
    request: Request,
    slot: Arc<Slot>,
}

impl Incoming {
    fn new(request: Request, stream: UnixStream) -> Incoming {
        let slot = Arc::new(Slot {
            state: Mutex::new((Stage::Waiting, stream)),
            moved: Condvar::new(),
        });
        // A request cannot ask for more patience than there is (nor for a time that
        // cannot be added to now).
        let deadline = Instant::now() + Duration::from_millis(request.wait_ms).min(LONGEST_WAIT);
        let watched = slot.clone();
        // Tells the tool that it was not served in time, at the moment its patience ends,
        // so that it never waits for an answer that says nothing happened.
        let watch = std::thread::Builder::new()
            .name("writer-live-deadline".into())
            .spawn(move || {
                let mut state = locked(&watched.state);
                while state.0 == Stage::Waiting {
                    let left = deadline.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        drop(state);
                        watched.answer(Answer::refused(Refusal::TooLate)).ok();
                        return;
                    }
                    state = watched
                        .moved
                        .wait_timeout(state, left)
                        .unwrap_or_else(PoisonError::into_inner)
                        .0;
                }
            });
        if watch.is_err() {
            // No thread to watch the clock: the request is served whenever it is served.
        }
        Incoming { request, slot }
    }

    /// What was asked.
    pub fn request(&self) -> &Request {
        &self.request
    }

    /// Says that the request is being carried out now. **Call it right before the first
    /// step (saving the open document), and do nothing when it says no**: `false` means
    /// the tool's patience ran out while the request waited, the tool was told that
    /// nothing was changed, and so nothing must be.
    ///
    /// From `true` on the tool waits for the answer.
    pub fn begin(&self) -> bool {
        let mut state = locked(&self.slot.state);
        match state.0 {
            Stage::Waiting => {
                state.0 = Stage::Begun;
                self.slot.moved.notify_all();
                true
            }
            Stage::Begun => true,
            Stage::Answered => false,
        }
    }

    /// Whether the tool has its answer already: its patience ran out. Looking is not
    /// beginning; see [`Incoming::begin`].
    pub fn expired(&self) -> bool {
        locked(&self.slot.state).0 == Stage::Answered
    }

    /// Sends the answer and closes the connection. An answer for a tool that was told
    /// already that its time ran out is not sent, and is no error.
    pub fn answer(self, answer: Answer) -> io::Result<()> {
        self.slot.answer(answer)
    }

    /// Answers that the app does not carry the request out. See [`Answer::refused`].
    pub fn refuse(self, refusal: Refusal) -> io::Result<()> {
        self.answer(Answer::refused(refusal))
    }
}

impl Drop for Incoming {
    fn drop(&mut self) {
        // Nobody answered: the app is going away with the request still in its hands.
        self.slot.answer(Answer::refused(Refusal::Quitting)).ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_socket_lies_where_only_the_user_gets_and_its_path_is_short() {
        let dir = socket_dir(24).unwrap();
        let found = std::fs::symlink_metadata(&dir).unwrap();
        assert!(found.is_dir());
        assert_eq!(found.uid(), user_id());
        assert_eq!(found.mode() & 0o077, 0, "{:o}", found.mode());
        assert!(dir.as_os_str().len() + 25 <= LONGEST_SOCKET_PATH, "{dir:?}");
    }

    #[test]
    fn a_directory_of_somebody_else_or_a_link_is_not_taken() {
        let parent = tempfile::tempdir().unwrap();
        let real = parent.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let link = parent.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(private_dir(&link).is_err(), "a link could lead anywhere");
        let file = parent.path().join("file");
        std::fs::write(&file, "x").unwrap();
        assert!(private_dir(&file).is_err());
        // One of the user's own that others could enter is closed to them.
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o755)).unwrap();
        private_dir(&real).unwrap();
        assert_eq!(std::fs::metadata(&real).unwrap().mode() & 0o777, 0o700);
    }

    #[test]
    fn processes_that_are_gone_are_told_from_those_that_run() {
        assert!(process_exists(std::process::id()));
        let mut child = std::process::Command::new("/usr/bin/true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        assert!(!process_exists(pid));
    }

    /// The other side may have written its line and closed before this side gets to
    /// read (an app that answers at once, a tool under load): the line is read all the
    /// same. On macOS setting the timeout of such a socket fails.
    #[test]
    fn a_line_is_read_after_the_other_side_has_closed() {
        let deadline = || Instant::now() + Duration::from_secs(5);
        let idle = Duration::from_secs(1);
        let (mut reader, mut writer) = UnixStream::pair().unwrap();
        writer.write_all(b"{\"answer\":\"done\"}\n").unwrap();
        drop(writer);
        assert_eq!(
            read_line(&mut reader, idle, deadline()).unwrap().as_deref(),
            Some("{\"answer\":\"done\"}")
        );
        // Nothing sent and closed: nothing, at once.
        let (mut reader, writer) = UnixStream::pair().unwrap();
        drop(writer);
        let started = Instant::now();
        assert_eq!(read_line(&mut reader, idle, deadline()).unwrap(), None);
        // Half a line and closed: cut off.
        let (mut reader, mut writer) = UnixStream::pair().unwrap();
        writer.write_all(b"{\"answer\":").unwrap();
        drop(writer);
        assert_eq!(
            read_line(&mut reader, idle, deadline()).unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
        assert!(started.elapsed() < Duration::from_millis(500));
        // A line in two pieces, with more after it, from a side that stays: the line.
        let (mut reader, mut writer) = UnixStream::pair().unwrap();
        let sender = std::thread::spawn(move || {
            writer.write_all(b"one ").unwrap();
            std::thread::sleep(Duration::from_millis(50));
            writer.write_all(b"line\nand more").unwrap();
            writer
        });
        assert_eq!(
            read_line(&mut reader, idle, deadline()).unwrap().as_deref(),
            Some("one line")
        );
        let writer = sender.join().unwrap();
        // Silence from a side that stays: the time runs out.
        let started = Instant::now();
        let silent = read_line(&mut reader, Duration::from_millis(100), deadline());
        assert_eq!(silent.unwrap_err().kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() >= Duration::from_millis(90));
        let short = Instant::now() + Duration::from_millis(100);
        assert_eq!(
            read_line(&mut reader, idle, short).unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
        drop(writer);
    }

    /// The socket lies in a directory the system clears of what was not used for three
    /// days: the listener renews its times, for as long as it is open.
    #[test]
    fn the_socket_is_kept_from_looking_unused() {
        use std::os::unix::ffi::OsStrExt;
        let dir = tempfile::Builder::new()
            .prefix("tw")
            .tempdir_in(std::env::temp_dir())
            .unwrap();
        let path = dir.path().join("app.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let age = |path: &Path| {
            let found = std::fs::metadata(path).unwrap();
            (
                found.modified().unwrap().elapsed().unwrap_or_default(),
                found.accessed().unwrap().elapsed().unwrap_or_default(),
            )
        };
        // Four days ago.
        let make_old = |path: &Path| {
            let then = libc::timeval {
                tv_sec: (std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs()
                    - 4 * 86_400) as libc::time_t,
                tv_usec: 0,
            };
            let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
            // SAFETY: a path that ends in a zero byte and two times.
            assert_eq!(
                unsafe { libc::utimes(name.as_ptr(), [then, then].as_ptr()) },
                0
            );
        };
        let days = Duration::from_secs(3 * 86_400);
        make_old(&path);
        make_old(dir.path());
        assert!(age(&path).0 > days && age(&path).1 > days);

        let shared = Arc::new(Shared {
            listener,
            path: path.clone(),
            data_dir: dir.path().to_path_buf(),
            closed: AtomicBool::new(false),
            over: (Mutex::new(false), Condvar::new()),
        });
        let kept = shared.clone();
        let keeper = std::thread::spawn(move || kept.keep_fresh(Duration::from_millis(30)));
        std::thread::sleep(Duration::from_millis(200));
        let minute = Duration::from_secs(60);
        assert!(
            age(&path).0 < minute && age(&path).1 < minute,
            "{:?}",
            age(&path)
        );
        assert!(age(dir.path()).0 < minute, "{:?}", age(dir.path()));
        // It ends with the listener, at once.
        make_old(&path);
        let started = Instant::now();
        shared.close();
        keeper.join().unwrap();
        assert!(started.elapsed() < Duration::from_millis(500));
        assert!(!path.exists());
    }

    #[test]
    fn sockets_of_apps_that_are_gone_are_swept_away() {
        let dir = tempfile::Builder::new()
            .prefix("tw")
            .tempdir_in(std::env::temp_dir())
            .unwrap();
        let mut child = std::process::Command::new("/usr/bin/true").spawn().unwrap();
        let gone = child.id();
        child.wait().unwrap();
        let left_behind = dir.path().join(format!("{gone}-0.sock"));
        let alive = dir.path().join(format!("{}-7.sock", std::process::id()));
        let not_a_socket = dir.path().join(format!("{gone}-1.sock"));
        let other = dir.path().join("notes.txt");
        drop(UnixListener::bind(&left_behind).unwrap());
        let listening = UnixListener::bind(&alive).unwrap();
        std::fs::write(&not_a_socket, "x").unwrap();
        std::fs::write(&other, "x").unwrap();
        sweep(dir.path());
        assert!(
            !left_behind.exists(),
            "the socket of a process that is gone"
        );
        assert!(alive.exists(), "the socket of a process that runs");
        assert!(
            not_a_socket.exists() && other.exists(),
            "nothing that is no socket"
        );
        drop(listening);
    }

    #[test]
    fn requests_of_another_version_and_lines_that_are_none_are_answered() {
        let refused = parse_request(r#"{"version": 99, "op": "teleport"}"#).unwrap_err();
        assert_eq!(
            refused.answer,
            Answer::refused(Refusal::Protocol {
                app: PROTOCOL_VERSION,
                tool: 99
            })
        );
        // A tool of 0.6.0, which speaks version 1, is refused whole: it would not send
        // the images a request of today brings, and must not be half understood.
        let older = parse_request(r#"{"version": 1, "wait_ms": 5, "op": "flush"}"#).unwrap_err();
        assert_eq!(
            older.answer,
            Answer::refused(Refusal::Protocol {
                app: PROTOCOL_VERSION,
                tool: 1
            })
        );
        assert_eq!(older.version, PROTOCOL_VERSION);
        let version = format!(r#"{{"version": {PROTOCOL_VERSION}}}"#);
        let sing = format!(r#"{{"version": {PROTOCOL_VERSION}, "op": "sing"}}"#);
        for line in ["", "not json", "{}", version.as_str(), sing.as_str()] {
            let Answer::Failed { error } = parse_request(line).unwrap_err().answer else {
                panic!("{line}");
            };
            assert_eq!(error.code, "invalid", "{line}");
            assert!(
                error.message.starts_with("the request cannot be read"),
                "{line}"
            );
        }
        let request = parse_request(&format!(
            r#"{{"version": {PROTOCOL_VERSION}, "wait_ms": 5, "op": "flush"}}"#
        ))
        .unwrap();
        assert!(request.is_flush());
        assert_eq!(request.wait_ms, 5);
    }
}
