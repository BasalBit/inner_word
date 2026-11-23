//! Who checks a page: a headless WebKit load of its files with the network blocked, which
//! decides whether it is whole and reads the text it shows (docs/pages-design.md,
//! sections 3 and 4). The same checker prints a page and a whiteboard as PDFs
//! (docs/whiteboard-design.md, section 5).
//!
//! WebKit never runs in this crate's process. The `innerword` tool runs itself as a child
//! (`innerword __page-check <dir>`, [`ProcessChecker`]), so that the MCP server, which is the
//! same program, never has WebKit in the process that reads its JSON-RPC stream. A
//! [`Library`](crate::Library) has no checker until one is installed
//! ([`Library::with_page_checker`](crate::Library::with_page_checker)): a library made in
//! a test never runs `<test binary> __page-check`, which libtest would read as a filter.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use innerword_page::{Bundle, CheckReport};

use crate::error::{Error, Result};

/// Checks pages and prints them, as WebKit does.
pub trait PageChecker: Send + Sync {
    /// The check of a bundled page: what each load of its HTML files saw. A checker that
    /// could not run at all fails with `failed` ("the page could not be checked: …").
    fn check(&self, bundle: &Bundle) -> Result<CheckReport>;
    /// A PDF of a page as WebKit prints it (`innerword export <page> x.pdf`): `dir` was
    /// written by innerword_page::write_served_dir.
    fn print(&self, dir: &Path, out: &Path) -> Result<()>;
    /// The PDF of a whiteboard (`innerword export <board> x.pdf`): one page exactly the
    /// size of the drawing and a margin, made by the whiteboard's export page in WebKit.
    /// `dir` was written by `innerword_board::export_dir::write` (`scene.json` and the
    /// pictures under `_files/`). [`ProcessChecker`] runs `innerword __board-pdf <dir>
    /// <out>`; [`StaticChecker`] writes a fixed page of the size Rust estimates.
    fn print_board(&self, dir: &Path, out: &Path) -> Result<()>;
}

/// The variable that makes [`ProcessChecker`] answer in its own process with
/// `innerword_page::static_check` (and the fixed one-page PDF): `INNER_WORD_PAGE_CHECK=fake`.
/// Read in a debug build only; the released tool always runs the real check, so nobody
/// can switch "whole or nothing" off with a variable.
pub const FAKE_CHECK_ENV: &str = "INNER_WORD_PAGE_CHECK";

/// The argument a child of the check is started with: `<program> __page-check <dir>`.
pub const CHECK_ARGUMENT: &str = "__page-check";

/// The argument a child that prints a page is started with: `<program> __page-pdf <dir>
/// <out.pdf>`.
pub const PDF_ARGUMENT: &str = "__page-pdf";

/// The argument a child that makes the PDF of a whiteboard is started with: `<program>
/// __board-pdf <dir> <out.pdf>`, `dir` as `innerword_board::export_dir::write` writes it.
pub const BOARD_PDF_ARGUMENT: &str = "__board-pdf";

/// When the child that makes the PDF of a whiteboard is killed: 60 seconds (the load of
/// the export page and its PDF take about a second; the child gives itself 50).
pub const BOARD_PDF_KILL_MS: u64 = 60_000;

/// Whether this run answers with the fake check: a debug build with
/// `INNER_WORD_PAGE_CHECK=fake`.
fn fake_check() -> bool {
    cfg!(debug_assertions) && std::env::var(FAKE_CHECK_ENV).is_ok_and(|value| value == "fake")
}

/// Runs `<program> __page-check <dir>` (and `__page-pdf <dir> <out>`) with stdin null,
/// stdout piped, stderr kept; kills the child at CHECK_KILL_MS. program:
/// std::env::current_exe(). With INNER_WORD_PAGE_CHECK=fake, in a debug build only, it
/// answers with innerword_page::static_check itself (and a fixed one-page PDF).
#[derive(Clone, Debug)]
pub struct ProcessChecker {
    /// The program; `None` when the path of this program could not be found out.
    program: Option<PathBuf>,
    /// When the child is killed.
    limit: Duration,
}

impl ProcessChecker {
    /// The program that runs: the tool runs itself.
    pub fn current_exe() -> ProcessChecker {
        ProcessChecker {
            program: std::env::current_exe().ok(),
            limit: Duration::from_millis(innerword_page::CHECK_KILL_MS),
        }
    }

    /// Another program that answers `__page-check` and `__page-pdf` as the tool does (a
    /// test's build of the tool).
    pub fn with_program(program: PathBuf) -> ProcessChecker {
        ProcessChecker {
            program: Some(program),
            limit: Duration::from_millis(innerword_page::CHECK_KILL_MS),
        }
    }

    /// The same checker with another time after which the child is killed
    /// (`CHECK_KILL_MS`, 135 seconds, unless changed). For tests of a child that hangs.
    pub fn with_time_limit(mut self, limit: Duration) -> ProcessChecker {
        self.limit = limit;
        self
    }

    fn program(&self, what: &str) -> Result<&Path> {
        self.program.as_deref().ok_or_else(|| {
            Error::failed(format!(
                "{what}: the path of this program is not known, so it cannot run its checker"
            ))
        })
    }
}

/// A directory of its own for one check (or one PDF), under `$TMPDIR/inner-word-<uid>/`
/// (which only this user can enter), removed when it goes.
pub(crate) struct WorkDir {
    pub(crate) path: PathBuf,
}

impl WorkDir {
    pub(crate) fn new(prefix: &str) -> Result<WorkDir> {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        // SAFETY: `getuid` has no arguments and cannot fail.
        let uid = unsafe { libc::getuid() };
        let parent = std::env::temp_dir().join(format!("inner-word-{uid}"));
        crate::live::private_dir(&parent).map_err(|source| Error::Io {
            path: parent.clone(),
            source,
        })?;
        let path = parent.join(format!(
            "{prefix}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        // Left over from a process of the same id that was killed.
        std::fs::remove_dir_all(&path).ok();
        Ok(WorkDir { path })
    }
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.path).ok();
    }
}

/// What a child said: its exit, what it printed, the last line of its standard error.
struct Ran {
    success: bool,
    code: Option<i32>,
    stdout: String,
    last_error_line: Option<String>,
}

/// Reads all of a pipe on a thread of its own, so that a child that writes much to the
/// other pipe never waits for this one. What it read arrives once the pipe is closed.
fn drain(pipe: Option<impl Read + Send + 'static>) -> std::sync::mpsc::Receiver<Vec<u8>> {
    let (send, read) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut out = Vec::new();
        if let Some(mut pipe) = pipe {
            pipe.read_to_end(&mut out).ok();
        }
        send.send(out).ok();
    });
    read
}

/// How long the output of a child that ended is waited for: a process it started may
/// hold its pipes open.
const OUTPUT_PATIENCE: Duration = Duration::from_secs(2);

/// Waits for `child` until `limit`; kills it then. `None`: it was killed.
fn wait_or_kill(
    child: &mut Child,
    limit: Duration,
) -> std::io::Result<Option<std::process::ExitStatus>> {
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        if started.elapsed() >= limit {
            child.kill().ok();
            child.wait().ok();
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Runs `program` with `args`: standard input closed (a child never reads the stream of
/// the MCP server), standard output and standard error read.
fn run(program: &Path, args: &[&std::ffi::OsStr], limit: Duration, what: &str) -> Result<Ran> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            Error::failed(format!(
                "{what}: its checker could not be started ({error})"
            ))
        })?;
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    let status = wait_or_kill(&mut child, limit).map_err(|error| {
        Error::failed(format!(
            "{what}: its checker could not be watched ({error})"
        ))
    })?;
    let Some(status) = status else {
        return Err(Error::failed(format!(
            "{what}: it did not finish within {} s",
            limit.as_secs().max(1)
        )));
    };
    let read = |pipe: std::sync::mpsc::Receiver<Vec<u8>>| {
        String::from_utf8_lossy(&pipe.recv_timeout(OUTPUT_PATIENCE).unwrap_or_default())
            .into_owned()
    };
    let stdout = read(stdout);
    let stderr = read(stderr);
    Ok(Ran {
        success: status.success(),
        code: status.code(),
        stdout,
        last_error_line: stderr
            .lines()
            .map(str::trim)
            .rfind(|line| !line.is_empty())
            .map(str::to_string),
    })
}

/// Why a child that failed failed, as the end of a sentence.
fn why(ran: &Ran) -> String {
    match (&ran.last_error_line, ran.code) {
        (Some(line), _) => line.clone(),
        (None, Some(code)) => format!("its checker stopped with exit code {code}"),
        (None, None) => "its checker was stopped by a signal".to_string(),
    }
}

impl PageChecker for ProcessChecker {
    fn check(&self, bundle: &Bundle) -> Result<CheckReport> {
        if fake_check() {
            return Ok(innerword_page::static_check(bundle));
        }
        const WHAT: &str = "the page could not be checked";
        let program = self.program(WHAT)?;
        let dir = WorkDir::new("check")?;
        innerword_page::write_check_dir(bundle, &dir.path).map_err(|source| Error::Io {
            path: dir.path.clone(),
            source,
        })?;
        let ran = run(
            program,
            &[CHECK_ARGUMENT.as_ref(), dir.path.as_os_str()],
            self.limit,
            WHAT,
        )?;
        if !ran.success {
            return Err(Error::failed(format!("{WHAT}: {}", why(&ran))));
        }
        // One line of JSON; anything printed before it is not the answer.
        ran.stdout
            .lines()
            .map(str::trim)
            .rfind(|line| !line.is_empty())
            .and_then(|line| serde_json::from_str::<CheckReport>(line).ok())
            .ok_or_else(|| {
                Error::failed(format!(
                    "{WHAT}: its checker answered nothing that can be read{}",
                    ran.last_error_line
                        .as_ref()
                        .map(|line| format!(" ({line})"))
                        .unwrap_or_default()
                ))
            })
    }

    fn print_board(&self, dir: &Path, out: &Path) -> Result<()> {
        if fake_check() {
            return write_board_pdf(dir, out);
        }
        const WHAT: &str = "the PDF of the whiteboard could not be made";
        let program = self.program(WHAT)?;
        let ran = run(
            program,
            &[
                BOARD_PDF_ARGUMENT.as_ref(),
                dir.as_os_str(),
                out.as_os_str(),
            ],
            self.limit.min(Duration::from_millis(BOARD_PDF_KILL_MS)),
            WHAT,
        )?;
        if !ran.success {
            return Err(Error::failed(format!("{WHAT}: {}", why(&ran))));
        }
        if !out.is_file() {
            return Err(Error::failed(format!("{WHAT}: its printer wrote no file")));
        }
        Ok(())
    }

    fn print(&self, dir: &Path, out: &Path) -> Result<()> {
        if fake_check() {
            return write_fixed_pdf(out);
        }
        const WHAT: &str = "the PDF of the page could not be made";
        let program = self.program(WHAT)?;
        let ran = run(
            program,
            &[PDF_ARGUMENT.as_ref(), dir.as_os_str(), out.as_os_str()],
            self.limit,
            WHAT,
        )?;
        if !ran.success {
            return Err(Error::failed(format!("{WHAT}: {}", why(&ran))));
        }
        if !out.is_file() {
            return Err(Error::failed(format!("{WHAT}: its printer wrote no file")));
        }
        Ok(())
    }
}

/// `innerword_page::static_check`, for tests: the text of each HTML file with its tags
/// taken out, nothing blocked; its PDF is a fixed page.
#[derive(Clone, Copy, Debug, Default)]
pub struct StaticChecker;

impl PageChecker for StaticChecker {
    fn check(&self, bundle: &Bundle) -> Result<CheckReport> {
        Ok(innerword_page::static_check(bundle))
    }

    fn print(&self, _dir: &Path, out: &Path) -> Result<()> {
        write_fixed_pdf(out)
    }

    fn print_board(&self, dir: &Path, out: &Path) -> Result<()> {
        write_board_pdf(dir, out)
    }
}

fn write_fixed_pdf(out: &Path) -> Result<()> {
    std::fs::write(out, fixed_pdf()).map_err(|source| Error::Io {
        path: out.to_path_buf(),
        source,
    })
}

/// What the fake checkers make of a whiteboard: one empty page of the size Rust estimates
/// for its PDF (`Scene::pdf_size`, the drawing's bounds and the margin), or A4 when the
/// scene in `dir` cannot be read.
fn write_board_pdf(dir: &Path, out: &Path) -> Result<()> {
    let size = std::fs::read_to_string(dir.join(innerword_board::export_dir::SCENE_FILE))
        .ok()
        .and_then(|scene| innerword_board::Scene::from_stored(&scene).ok())
        .and_then(|parsed| {
            parsed.scene.pdf_size(
                innerword_board::limits::PDF_PADDING,
                innerword_board::limits::PDF_MAX_SIDE,
            )
        });
    let bytes = match size {
        Some((width, height)) => pdf_of_size(width, height),
        None => fixed_pdf(),
    };
    std::fs::write(out, bytes).map_err(|source| Error::Io {
        path: out.to_path_buf(),
        source,
    })
}

/// A PDF of one empty A4 page: what the fake checkers print.
pub(crate) fn fixed_pdf() -> Vec<u8> {
    pdf_of_size(595.28, 841.89)
}

/// A PDF of one empty page `width` x `height` points.
fn pdf_of_size(width: f64, height: f64) -> Vec<u8> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} {height}] /Resources << >> \
             /Contents 4 0 R >>"
        ),
        "<< /Length 0 >>\nstream\n\nendstream".to_string(),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (number, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n{object}\nendobj\n", number + 1).into_bytes());
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).into_bytes());
    for offset in offsets {
        out.extend(format!("{offset:010} 00000 n \n").into_bytes());
    }
    out.extend(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .into_bytes(),
    );
    out
}
