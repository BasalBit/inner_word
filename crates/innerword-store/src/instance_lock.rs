//! One writer per library: the lock of a data directory.
//!
//! The app keeps the whole open document in memory and writes all of it when it saves.
//! Anything else that wrote to the same library meanwhile (a second copy of the app, the
//! `innerword` command line tool on behalf of an agent) would be overwritten by the app's next
//! save, or would overwrite it, without either side noticing. So whoever is going to write
//! takes the lock of the data directory first:
//!
//! - **The app** takes it when it starts and keeps it until the process ends
//!   ([`acquire`], or [`acquire_waiting`] to ride out a tool that is writing right now).
//! - **A tool** takes it for the duration of one write ([`acquire_waiting`] with
//!   [`HolderKind::Tool`]) and treats a lock held by the app as "the app is open: do not
//!   write". Reading needs no lock; [`holder`] only asks who has it.
//!
//! The lock is an exclusive `flock` on the file [`LOCK_FILE_NAME`] in the data directory,
//! tried without waiting. It belongs to the open file, not to the file's content: when the
//! process ends, however it ends, the system releases it, so a lock file left behind by a
//! crash is never mistaken for a running copy.
//!
//! The file holds one line that says who has the lock: the process id, followed by the
//! word `tool` when the holder is not the app (`"4711\n"`, `"4712 tool\n"`). App versions
//! from before this module existed (0.2.0 and earlier) take the same lock on the same file
//! and write the bare process id, so they and this module see each other.
//!
//! An app that takes changes from tools while it runs says where it listens in a second
//! line ([`announce_channel`], read with [`app_channel`]): `"channel <path of a socket>\n"`.
//! Because the line is in the lock file, it is always the line of whoever holds the lock
//! now: the next holder writes the file anew, and a file left behind by a crash has no
//! holder at all.
//!
//! ```no_run
//! use std::time::Duration;
//! use innerword_store::instance_lock::{self, Acquired, HolderKind};
//!
//! let data_dir = innerword_store::data_dir().unwrap();
//! match instance_lock::acquire_waiting(&data_dir, HolderKind::Tool, Duration::from_secs(10)).unwrap() {
//!     Acquired::Lock(lock) => {
//!         // ... write to the library ...
//!         lock.release();
//!     }
//!     Acquired::Taken(copy) => eprintln!("the library is in use by process {:?}", copy.pid),
//! }
//! ```

use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    os::fd::AsRawFd,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// The name of the lock file in the data directory.
pub const LOCK_FILE_NAME: &str = "instance.lock";

/// The word after the process id in the lock file when the holder is a tool.
const TOOL_MARK: &str = "tool";

/// The word a line of the lock file begins with when it names the app's channel.
const CHANNEL_MARK: &str = "channel ";

/// How long [`acquire_waiting`] sleeps between two tries.
const RETRY_INTERVAL: Duration = Duration::from_millis(20);

/// Who holds (or asks for) the lock of a data directory.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum HolderKind {
    /// The app: it holds the lock for as long as it runs.
    App,
    /// A tool working on the library without the app (the `innerword` command line tool, its
    /// MCP server): it holds the lock for one write, a fraction of a second.
    Tool,
}

/// The lock of a data directory, held by this process until it is released, dropped, or
/// the process ends.
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

    /// Gives the lock up now. Dropping the lock does the same.
    pub fn release(self) {
        drop(self);
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

/// The process that already has a data directory locked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunningCopy {
    /// Its process id, when the lock file names one.
    pub pid: Option<u32>,
    /// The data directory in question.
    pub data_dir: PathBuf,
    /// Whether it is the app or a tool. A lock file without the tool mark is the app's
    /// (that is what every version of the app writes).
    pub kind: HolderKind,
}

/// What an attempt to take the lock found.
#[derive(Debug)]
pub enum Acquired {
    /// This process now has the data directory to itself.
    Lock(InstanceLock),
    /// Another process has it.
    Taken(RunningCopy),
}

impl Acquired {
    /// The lock, when it was taken.
    pub fn lock(self) -> Option<InstanceLock> {
        match self {
            Acquired::Lock(lock) => Some(lock),
            Acquired::Taken(_) => None,
        }
    }
}

/// Takes the lock of `data_dir` for the app, without waiting, creating the directory when
/// it does not exist yet. The same as [`acquire_as`] with [`HolderKind::App`].
///
/// `Ok(Acquired::Taken(..))` means another process holds it. An `Err` means the lock could
/// not be tried at all (the directory cannot be created or written): the caller decides;
/// the library will most likely fail to open for the same reason and say so.
pub fn acquire(data_dir: &Path) -> io::Result<Acquired> {
    acquire_as(data_dir, HolderKind::App)
}

/// Takes the lock of `data_dir` as `kind`, without waiting, creating the directory when it
/// does not exist yet. See [`acquire`] for the results.
pub fn acquire_as(data_dir: &Path, kind: HolderKind) -> io::Result<Acquired> {
    std::fs::create_dir_all(data_dir)?;
    acquire_in_place(data_dir, kind)
}

/// [`acquire_as`] for a data directory that must be there already: the directory is never
/// created (the lock file in it is, when it is missing), and a directory that is not there
/// is an error of kind `NotFound`. The move of the library to its new name
/// ([`crate::data_dir`]) takes the lock of the old folder so, because creating it again
/// while another process moves it away would leave an empty library under the old name.
pub(crate) fn acquire_in_place(data_dir: &Path, kind: HolderKind) -> io::Result<Acquired> {
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
        return Ok(Acquired::Taken(read_holder(&mut file, data_dir)));
    }
    // Ours: say who we are. A failure to write the line does not undo the lock; what
    // another process says about the holder is then only less specific.
    let line = match kind {
        HolderKind::App => format!("{}\n", std::process::id()),
        HolderKind::Tool => format!("{} {TOOL_MARK}\n", std::process::id()),
    };
    let written = file
        .set_len(0)
        .and_then(|()| file.seek(SeekFrom::Start(0)).map(|_| ()))
        .and_then(|()| file.write_all(line.as_bytes()))
        .and_then(|()| file.flush());
    if let Err(error) = written {
        eprintln!(
            "Inner Word: could not write {}: {error}",
            path.to_string_lossy()
        );
    }
    Ok(Acquired::Lock(InstanceLock { file, path }))
}

/// Takes the lock of `data_dir` as `kind`, waiting up to `patience` for a holder that will
/// let go soon.
///
/// A tool holds the lock for one write only, so while the lock file names a tool the
/// attempt is repeated (every 20 ms) until `patience` runs out. A lock held by the app is
/// not waited for beyond a moment (a quarter of a second at most, and never longer than
/// `patience`): the app keeps it for as long as it runs. That moment covers the instant in
/// which a new holder has the lock but has not yet written its line into the file.
///
/// `Ok(Acquired::Taken(..))` names the holder that was still there at the end.
pub fn acquire_waiting(
    data_dir: &Path,
    kind: HolderKind,
    patience: Duration,
) -> io::Result<Acquired> {
    /// How long a lock that claims to be the app's is tried again.
    const APP_GRACE: Duration = Duration::from_millis(250);
    let started = Instant::now();
    loop {
        let copy = match acquire_as(data_dir, kind)? {
            Acquired::Lock(lock) => return Ok(Acquired::Lock(lock)),
            Acquired::Taken(copy) => copy,
        };
        let waited = started.elapsed();
        let limit = match copy.kind {
            HolderKind::Tool => patience,
            HolderKind::App => patience.min(APP_GRACE),
        };
        if waited >= limit {
            return Ok(Acquired::Taken(copy));
        }
        std::thread::sleep(RETRY_INTERVAL.min(limit - waited));
    }
}

/// Who holds the lock of `data_dir` right now, or `None` when nobody does.
///
/// Only looks: nothing is created (a data directory or lock file that does not exist has
/// no holder) and, on macOS, no lock is taken even for an instant, so asking never gets in
/// the way of a copy that is starting. On other systems the lock is tried and released at
/// once.
///
/// The answer is a moment old by the time it is read. Use it to tell someone that the app
/// is open; before writing, take the lock.
pub fn holder(data_dir: &Path) -> io::Result<Option<RunningCopy>> {
    let path = data_dir.join(LOCK_FILE_NAME);
    let mut file = match File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !is_locked_by_another(&file)? {
        return Ok(None);
    }
    Ok(Some(read_holder(&mut file, data_dir)))
}

/// Whether the app has `data_dir` open right now: [`holder`] names a holder that is not a
/// tool.
pub fn app_is_running(data_dir: &Path) -> io::Result<bool> {
    Ok(holder(data_dir)?.is_some_and(|copy| copy.kind == HolderKind::App))
}

/// Where the app that holds the lock of a data directory takes requests from tools. See
/// [`app_channel`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppChannel {
    /// The app's process id, when the lock file names it.
    pub pid: Option<u32>,
    /// The socket it listens on.
    pub path: PathBuf,
}

/// Writes into the lock file of `data_dir` where the app listens for requests from tools
/// (`Some(path)`), or that it no longer does (`None`). Only the app that holds the lock
/// calls this: it fails with [`io::ErrorKind::PermissionDenied`] when the lock file does
/// not name this process as its holder, and nothing is written then.
///
/// The path must not contain a line break.
pub fn announce_channel(data_dir: &Path, channel: Option<&Path>) -> io::Result<()> {
    let path = data_dir.join(LOCK_FILE_NAME);
    let mut file = OpenOptions::new().read(true).write(true).open(&path)?;
    let held = is_locked_by_another(&file)?;
    let holder = read_holder(&mut file, data_dir);
    if !held || holder.pid != Some(std::process::id()) || holder.kind != HolderKind::App {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "this process does not hold the lock of the data directory as the app",
        ));
    }
    let mut text = format!("{}\n", std::process::id());
    if let Some(channel) = channel {
        let channel = channel.to_str().filter(|text| !text.contains(['\n', '\r']));
        let Some(channel) = channel else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the path of the channel cannot be written on one line",
            ));
        };
        text.push_str(CHANNEL_MARK);
        text.push_str(channel);
        text.push('\n');
    }
    // The first line stays what it was, so a reader that comes between the two steps
    // reads the holder it would have read before.
    file.seek(SeekFrom::Start(0))?;
    file.write_all(text.as_bytes())?;
    file.set_len(text.len() as u64)?;
    file.flush()
}

/// Where the app that has `data_dir` open takes requests from tools: `None` when nobody
/// holds the lock, when a tool holds it, and when the app that holds it announced no
/// channel (every version before 0.6.0, and an app that could not open one).
///
/// Only looks, like [`holder`]. A socket file may be left behind by an app that crashed;
/// this function never names it, because the lock that app held is gone with it.
pub fn app_channel(data_dir: &Path) -> io::Result<Option<AppChannel>> {
    let path = data_dir.join(LOCK_FILE_NAME);
    let mut file = match File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !is_locked_by_another(&file)? {
        return Ok(None);
    }
    let mut text = String::new();
    file.seek(SeekFrom::Start(0)).ok();
    file.read_to_string(&mut text).ok();
    let (pid, kind) = parse_holder(&text);
    if kind != HolderKind::App {
        return Ok(None);
    }
    Ok(parse_channel(&text).map(|path| AppChannel { pid, path }))
}

/// When the holder of the lock of `data_dir` last wrote into the lock file: the moment
/// it took the lock, or the moment it last said where it listens ([`announce_channel`]).
/// `None` when nobody holds the lock.
///
/// A tool uses it to tell an app that has only just started, and will open its channel in
/// a moment, from one that never will.
pub fn held_since(data_dir: &Path) -> io::Result<Option<std::time::SystemTime>> {
    let path = data_dir.join(LOCK_FILE_NAME);
    let file = match File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !is_locked_by_another(&file)? {
        return Ok(None);
    }
    Ok(Some(file.metadata()?.modified()?))
}

/// The channel a lock file's content names.
fn parse_channel(text: &str) -> Option<PathBuf> {
    text.lines()
        .skip(1)
        .find_map(|line| line.strip_prefix(CHANNEL_MARK))
        .map(str::trim_end)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
}

/// Whether some other open file holds a lock on `file`.
#[cfg(target_os = "macos")]
fn is_locked_by_another(file: &File) -> io::Result<bool> {
    // On macOS `flock` locks and `fcntl` record locks are kept in one list per file, so
    // asking "would a write lock on the whole file be refused?" reports an `flock` held
    // through any other open file (the module's tests hold this true).
    let mut request = libc::flock {
        l_start: 0,
        l_len: 0,
        l_pid: 0,
        l_type: libc::F_WRLCK as libc::c_short,
        l_whence: libc::SEEK_SET as libc::c_short,
    };
    // SAFETY: the descriptor is open and `request` is a valid `struct flock` to fill in.
    let answered = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETLK, &mut request) };
    if answered == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(request.l_type != libc::F_UNLCK as libc::c_short)
}

/// Whether some other open file holds a lock on `file`.
#[cfg(not(target_os = "macos"))]
fn is_locked_by_another(file: &File) -> io::Result<bool> {
    // Elsewhere record locks do not see `flock` locks: try the lock and let go at once.
    // SAFETY: the descriptor belongs to `file`, which is open.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        // SAFETY: as above.
        unsafe {
            libc::flock(file.as_raw_fd(), libc::LOCK_UN);
        }
        return Ok(false);
    }
    let error = io::Error::last_os_error();
    if error.kind() == io::ErrorKind::WouldBlock {
        Ok(true)
    } else {
        Err(error)
    }
}

/// Reads the holder's line from the lock file.
fn read_holder(file: &mut File, data_dir: &Path) -> RunningCopy {
    let mut text = String::new();
    file.seek(SeekFrom::Start(0)).ok();
    file.read_to_string(&mut text).ok();
    let (pid, kind) = parse_holder(&text);
    RunningCopy {
        pid,
        data_dir: data_dir.to_path_buf(),
        kind,
    }
}

/// The process id and the kind a lock file's content names.
fn parse_holder(text: &str) -> (Option<u32>, HolderKind) {
    // The first line only: an app may say more below it (its channel).
    let mut words = text.lines().next().unwrap_or_default().split_whitespace();
    let pid = words.next().and_then(|word| word.parse().ok());
    let kind = match words.next() {
        Some(TOOL_MARK) => HolderKind::Tool,
        _ => HolderKind::App,
    };
    (pid, kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lock(acquired: Acquired) -> InstanceLock {
        match acquired {
            Acquired::Lock(lock) => lock,
            Acquired::Taken(copy) => panic!("the lock is taken: {copy:?}"),
        }
    }

    fn taken(acquired: Acquired) -> RunningCopy {
        match acquired {
            Acquired::Taken(copy) => copy,
            Acquired::Lock(_) => panic!("the lock was free"),
        }
    }

    #[test]
    fn the_second_attempt_finds_the_first() {
        let dir = tempfile::tempdir().unwrap();
        let first = lock(acquire(dir.path()).unwrap());
        assert_eq!(first.path(), dir.path().join(LOCK_FILE_NAME));
        let copy = taken(acquire(dir.path()).unwrap());
        assert_eq!(copy.pid, Some(std::process::id()));
        assert_eq!(copy.kind, HolderKind::App);
        assert_eq!(copy.data_dir, dir.path());
        // A tool is refused in the same way, and learns that it is the app.
        let copy = taken(acquire_as(dir.path(), HolderKind::Tool).unwrap());
        assert_eq!(copy.kind, HolderKind::App);
        drop(first);
        let again = lock(acquire(dir.path()).unwrap());
        again.release();
        lock(acquire_as(dir.path(), HolderKind::Tool).unwrap());
    }

    /// The line an app from before this module wrote, and reads, is the bare process id.
    #[test]
    fn the_app_writes_the_line_older_versions_read() {
        let dir = tempfile::tempdir().unwrap();
        let held = lock(acquire(dir.path()).unwrap());
        let text = std::fs::read_to_string(held.path()).unwrap();
        assert_eq!(text, format!("{}\n", std::process::id()));
        assert_eq!(text.trim().parse::<u32>().ok(), Some(std::process::id()));
        drop(held);

        let held = lock(acquire_as(dir.path(), HolderKind::Tool).unwrap());
        let text = std::fs::read_to_string(held.path()).unwrap();
        assert_eq!(text, format!("{} tool\n", std::process::id()));
        let copy = taken(acquire(dir.path()).unwrap());
        assert_eq!(copy.kind, HolderKind::Tool);
        assert_eq!(copy.pid, Some(std::process::id()));
    }

    #[test]
    fn holder_lines() {
        assert_eq!(parse_holder("4711\n"), (Some(4711), HolderKind::App));
        assert_eq!(parse_holder("4711 tool\n"), (Some(4711), HolderKind::Tool));
        assert_eq!(parse_holder(""), (None, HolderKind::App));
        assert_eq!(parse_holder("not a number"), (None, HolderKind::App));
        assert_eq!(
            parse_holder("  12   tool  extra"),
            (Some(12), HolderKind::Tool)
        );
        assert_eq!(parse_holder("12 other"), (Some(12), HolderKind::App));
        // The channel of an app is on a line of its own and is not the holder's kind.
        assert_eq!(
            parse_holder("12\nchannel /tmp/tool\n"),
            (Some(12), HolderKind::App)
        );
        assert_eq!(
            parse_channel("12\nchannel /tmp/a b/x.sock\n"),
            Some(PathBuf::from("/tmp/a b/x.sock"))
        );
        assert_eq!(parse_channel("12\n"), None);
        assert_eq!(parse_channel("12 tool\n"), None);
        assert_eq!(
            parse_channel("channel /x\n"),
            None,
            "the first line is the holder"
        );
    }

    #[test]
    fn the_lock_file_says_since_when_it_is_held() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(held_since(dir.path()).unwrap(), None, "no lock file");
        let held = lock(acquire(dir.path()).unwrap());
        let since = held_since(dir.path()).unwrap().expect("held");
        let age = since.elapsed().unwrap_or_default();
        assert!(age < Duration::from_secs(5), "{age:?}");
        // The file is a plain file: whoever holds the lock may be older than a moment.
        let an_hour_ago = std::time::SystemTime::now() - Duration::from_secs(3600);
        File::options()
            .write(true)
            .open(held.path())
            .unwrap()
            .set_modified(an_hour_ago)
            .unwrap();
        let since = held_since(dir.path()).unwrap().expect("held");
        assert!(since.elapsed().unwrap() >= Duration::from_secs(3599));
        drop(held);
        assert_eq!(held_since(dir.path()).unwrap(), None, "nobody holds it");
    }

    #[test]
    fn the_app_announces_its_channel_in_the_lock_file() {
        let dir = tempfile::tempdir().unwrap();
        // Nobody holds the lock: nothing to announce in, nothing to read.
        assert_eq!(app_channel(dir.path()).unwrap(), None);
        assert!(announce_channel(dir.path(), Some(Path::new("/tmp/x.sock"))).is_err());

        let held = lock(acquire(dir.path()).unwrap());
        assert_eq!(app_channel(dir.path()).unwrap(), None, "none announced yet");
        let socket = Path::new("/var/folders/a b/T/inner-word-501/4711-0.sock");
        announce_channel(dir.path(), Some(socket)).unwrap();
        assert_eq!(
            std::fs::read_to_string(held.path()).unwrap(),
            format!("{}\nchannel {}\n", std::process::id(), socket.display())
        );
        assert_eq!(
            app_channel(dir.path()).unwrap(),
            Some(AppChannel {
                pid: Some(std::process::id()),
                path: socket.to_path_buf(),
            })
        );
        // Whoever asks who holds the lock is told what it was told before.
        let copy = holder(dir.path()).unwrap().expect("held");
        assert_eq!(copy.pid, Some(std::process::id()));
        assert_eq!(copy.kind, HolderKind::App);
        let copy = taken(acquire_as(dir.path(), HolderKind::Tool).unwrap());
        assert_eq!(copy.kind, HolderKind::App);

        // A shorter path leaves nothing of the longer one behind; none takes the line out.
        announce_channel(dir.path(), Some(Path::new("/s"))).unwrap();
        assert_eq!(
            app_channel(dir.path()).unwrap().map(|channel| channel.path),
            Some(PathBuf::from("/s"))
        );
        assert!(announce_channel(dir.path(), Some(Path::new("/a\nb"))).is_err());
        announce_channel(dir.path(), None).unwrap();
        assert_eq!(app_channel(dir.path()).unwrap(), None);
        assert_eq!(
            std::fs::read_to_string(held.path()).unwrap(),
            format!("{}\n", std::process::id())
        );

        // The lock file of an app that crashed names a channel and has no holder: nobody
        // is sent there. The next holder writes the file anew.
        announce_channel(dir.path(), Some(socket)).unwrap();
        drop(held);
        assert!(
            std::fs::read_to_string(dir.path().join(LOCK_FILE_NAME))
                .unwrap()
                .contains("channel ")
        );
        assert_eq!(app_channel(dir.path()).unwrap(), None);
        let tool = lock(acquire_as(dir.path(), HolderKind::Tool).unwrap());
        assert_eq!(app_channel(dir.path()).unwrap(), None);
        assert!(
            announce_channel(dir.path(), Some(socket)).is_err(),
            "a tool has no channel"
        );
        drop(tool);
        let again = lock(acquire(dir.path()).unwrap());
        assert_eq!(app_channel(dir.path()).unwrap(), None);
        drop(again);
    }

    #[test]
    fn holder_only_looks() {
        let parent = tempfile::tempdir().unwrap();
        let dir = parent.path().join("not there yet");
        assert_eq!(holder(&dir).unwrap(), None);
        assert!(!app_is_running(&dir).unwrap());
        assert!(!dir.exists(), "asking must not create the data directory");

        let held = lock(acquire(&dir).unwrap());
        let copy = holder(&dir).unwrap().expect("the lock is held");
        assert_eq!(copy.pid, Some(std::process::id()));
        assert_eq!(copy.kind, HolderKind::App);
        assert!(app_is_running(&dir).unwrap());
        // Asking took nothing away from the holder and gave nothing to the asker.
        taken(acquire(&dir).unwrap());
        drop(held);

        assert_eq!(holder(&dir).unwrap(), None);
        assert!(!app_is_running(&dir).unwrap());
        // Asking did not leave a lock behind either.
        let tool = lock(acquire_as(&dir, HolderKind::Tool).unwrap());
        let copy = holder(&dir).unwrap().expect("the lock is held");
        assert_eq!(copy.kind, HolderKind::Tool);
        assert!(!app_is_running(&dir).unwrap());
        drop(tool);
    }

    #[test]
    fn a_lock_file_left_behind_is_not_a_holder() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(LOCK_FILE_NAME), "99999\n").unwrap();
        assert_eq!(holder(dir.path()).unwrap(), None);
        lock(acquire(dir.path()).unwrap());
    }

    #[test]
    fn waiting_rides_out_a_tool_but_not_the_app() {
        let dir = tempfile::tempdir().unwrap();
        // A tool that lets go after a moment.
        let tool = lock(acquire_as(dir.path(), HolderKind::Tool).unwrap());
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            drop(tool);
        });
        let started = Instant::now();
        let ours =
            lock(acquire_waiting(dir.path(), HolderKind::App, Duration::from_secs(20)).unwrap());
        assert!(started.elapsed() >= Duration::from_millis(100));
        assert!(started.elapsed() < Duration::from_secs(10));
        release.join().unwrap();

        // The app does not let go: a tool gives up after the moment of grace, long before
        // its patience is over.
        let started = Instant::now();
        let copy =
            taken(acquire_waiting(dir.path(), HolderKind::Tool, Duration::from_secs(20)).unwrap());
        assert_eq!(copy.kind, HolderKind::App);
        assert!(started.elapsed() < Duration::from_secs(5));
        drop(ours);

        // A tool that does not let go in time is reported, with no patience at all at once.
        let tool = lock(acquire_as(dir.path(), HolderKind::Tool).unwrap());
        let started = Instant::now();
        let copy = taken(acquire_waiting(dir.path(), HolderKind::Tool, Duration::ZERO).unwrap());
        assert_eq!(copy.kind, HolderKind::Tool);
        assert!(started.elapsed() < Duration::from_secs(1));
        let copy = taken(
            acquire_waiting(dir.path(), HolderKind::Tool, Duration::from_millis(100)).unwrap(),
        );
        assert_eq!(copy.kind, HolderKind::Tool);
        drop(tool);
    }

    /// The variable that turns [`the_child_holds_the_lock`] into the other process of
    /// [`another_process_holds_and_releases_it`].
    const CHILD_DIR: &str = "INNERWORD_STORE_LOCK_TEST_DIR";

    /// Not a test of its own: run by [`another_process_holds_and_releases_it`] in a second
    /// process, where it takes the lock as a tool and keeps it until its input closes.
    #[test]
    fn the_child_holds_the_lock() {
        let Some(dir) = std::env::var_os(CHILD_DIR) else {
            return;
        };
        let _held = lock(acquire_as(Path::new(&dir), HolderKind::Tool).unwrap());
        println!("\nheld");
        io::stdout().flush().unwrap();
        let mut rest = String::new();
        io::stdin().read_to_string(&mut rest).ok();
    }

    /// The lock is seen from another process, and released when that process ends.
    #[test]
    fn another_process_holds_and_releases_it() {
        use std::process::{Command, Stdio};

        let dir = tempfile::tempdir().unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "instance_lock::tests::the_child_holds_the_lock",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD_DIR, dir.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut output = io::BufReader::new(child.stdout.take().unwrap());
        let mut held = false;
        let mut line = String::new();
        while io::BufRead::read_line(&mut output, &mut line).unwrap() > 0 {
            if line.trim() == "held" {
                held = true;
                break;
            }
            line.clear();
        }
        assert!(held, "the other process did not take the lock");

        let copy = holder(dir.path()).unwrap().expect("held by the child");
        assert_eq!(copy.kind, HolderKind::Tool);
        assert_eq!(copy.pid, Some(child.id()));
        let copy = taken(acquire(dir.path()).unwrap());
        assert_eq!(copy.pid, Some(child.id()));

        // The child ends (its input closes): the system releases the lock.
        drop(child.stdin.take());
        assert!(child.wait().unwrap().success());
        assert_eq!(holder(dir.path()).unwrap(), None);
        lock(acquire(dir.path()).unwrap());
    }
}
