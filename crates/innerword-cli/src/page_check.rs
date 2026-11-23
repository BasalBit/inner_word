//! The two children of a page: `innerword __page-check <dir>` and `innerword __page-pdf <dir>
//! <out.pdf>` (docs/pages-design.md, section 4, "The child process").
//!
//! The tool never has WebKit in its own process: the MCP server is this same program and
//! reads its JSON-RPC stream on standard input. A page is checked, and printed, by the
//! tool running itself as a child (`innerword_agent::ProcessChecker`), with standard input
//! closed, in a directory that `innerword_page::write_check_dir` (or `write_served_dir`)
//! wrote: `request.json` and the served files under `files/`. The child answers on
//! standard output (the check: one line of JSON, a `innerword_page::CheckReport`) or in the
//! file it was named (the PDF), and says what went wrong on the last line of standard
//! error, which the parent puts into its sentence ("the page could not be checked: …").
//!
//! Neither command is in the table of commands or in the help: they are handled before the
//! command line is read, like a program of their own.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use innerword_agent::{CHECK_ARGUMENT, PDF_ARGUMENT};
use innerword_page::{Blocked, CheckReport, LoadReport};
use innerword_web::{CheckInput, CheckOutcome, LoadOutcome, MemorySource, PdfInput, ServedFile};

/// Runs a child when the command line is one: `Some` with its exit code, `None` for any
/// other command line.
pub fn run(arguments: &[String]) -> Option<ExitCode> {
    let (first, rest) = arguments.split_first()?;
    if first == CHECK_ARGUMENT {
        Some(finish(match rest {
            [dir] => check(Path::new(dir)),
            _ => Err(Failed::usage(format!(
                "usage: innerword {CHECK_ARGUMENT} <dir>"
            ))),
        }))
    } else if first == PDF_ARGUMENT {
        Some(finish(match rest {
            [dir, out] => pdf(Path::new(dir), Path::new(out)),
            _ => Err(Failed::usage(format!(
                "usage: innerword {PDF_ARGUMENT} <dir> <out.pdf>"
            ))),
        }))
    } else {
        None
    }
}

/// Why a child did not do its work: the clause its parent puts after "the page could not
/// be checked: ", and the exit code.
#[derive(Debug)]
pub(crate) struct Failed {
    reason: String,
    code: u8,
}

impl Failed {
    pub(crate) fn new(reason: impl Into<String>) -> Failed {
        Failed {
            reason: reason.into(),
            code: 1,
        }
    }

    pub(crate) fn usage(reason: impl Into<String>) -> Failed {
        Failed {
            reason: reason.into(),
            code: 2,
        }
    }

    /// Why, as the parent reads it.
    #[cfg(test)]
    pub(crate) fn reason(&self) -> &str {
        &self.reason
    }
}

pub(crate) fn finish(result: Result<(), Failed>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(failed) => {
            // One line: the parent reads the last one.
            let reason = failed
                .reason
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            eprintln!("{reason}");
            ExitCode::from(failed.code)
        }
    }
}

/// Where the rule list of WebKit is compiled and kept for the tool's children:
/// `$TMPDIR/inner-word-<uid>/WebKit/`, beside the directories of the checks (which only
/// this user can enter), under its one fixed identifier, so the store holds one list
/// however many checks run.
pub(crate) fn rules_dir() -> Result<PathBuf, Failed> {
    // SAFETY: `getuid` has no arguments and cannot fail.
    let uid = unsafe { libc::getuid() };
    let parent = std::env::temp_dir().join(format!("inner-word-{uid}"));
    let dir = parent.join("WebKit");
    std::fs::create_dir_all(&dir).map_err(|error| {
        Failed::new(format!(
            "the folder for WebKit's rules ({}) could not be made: {error}",
            dir.display()
        ))
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o700)).ok();
    }
    Ok(dir)
}

/// The served files of a check directory, as WebKit is handed them, and its request.
fn source(dir: &Path) -> Result<(innerword_page::CheckRequest, Arc<MemorySource>), Failed> {
    let (request, files) = innerword_page::read_check_dir(dir).map_err(|error| {
        Failed::new(format!(
            "its files could not be read from {} ({error})",
            dir.display()
        ))
    })?;
    let files = files.into_iter().map(|(path, (media_type, bytes))| {
        (
            path,
            ServedFile {
                media_type,
                bytes: Arc::from(bytes),
            },
        )
    });
    Ok((request, Arc::new(MemorySource::new(files))))
}

/// `innerword __page-check <dir>`: loads every HTML file of the page in WebKit with the
/// network blocked, and prints what each load saw as one line of JSON.
fn check(dir: &Path) -> Result<(), Failed> {
    innerword_web::available().map_err(Failed::new)?;
    let (request, source) = source(dir)?;
    let input = CheckInput {
        source,
        entry: request.entry,
        also: request.also,
        settle_min: Duration::from_millis(request.settle_min_ms),
        settle_max: Duration::from_millis(request.settle_max_ms),
        limit: Duration::from_millis(request.limit_ms),
        rules_dir: rules_dir()?,
    };
    let report = report(innerword_web::check_blocking(input));
    let line = serde_json::to_string(&report)
        .map_err(|error| Failed::new(format!("its report could not be written ({error})")))?;
    println!("{line}");
    Ok(())
}

/// `innerword __page-pdf <dir> <out.pdf>`: loads the page as the check does and writes
/// WebKit's print of it, on the default paper, into exactly `out`.
fn pdf(dir: &Path, out: &Path) -> Result<(), Failed> {
    innerword_web::available().map_err(Failed::new)?;
    let (request, source) = source(dir)?;
    let input = PdfInput {
        source,
        entry: request.entry,
        settle_max: Duration::from_millis(request.settle_max_ms),
        limit: Duration::from_millis(request.limit_ms),
        rules_dir: rules_dir()?,
    };
    let printed = innerword_web::print_blocking(input).map_err(Failed::new)?;
    std::fs::write(out, &printed.bytes)
        .map_err(|error| Failed::new(format!("{} could not be written ({error})", out.display())))
}

/// What WebKit found, as the bundler reads it.
fn report(outcome: CheckOutcome) -> CheckReport {
    CheckReport {
        engine: outcome.engine,
        elapsed_ms: u64::try_from(outcome.elapsed.as_millis()).unwrap_or(u64::MAX),
        loads: outcome.loads.into_iter().map(load).collect(),
        failure: outcome.failure,
    }
}

fn load(load: LoadOutcome) -> LoadReport {
    LoadReport {
        path: load.path,
        title: load.title,
        first_heading: load.first_heading,
        text: load.text,
        finished: load.finished,
        navigation_error: load.navigation_error,
        crashed: load.crashed,
        missing: load.missing,
        blocked: load
            .blocked
            .into_iter()
            .map(|(url, directive)| Blocked { url, directive })
            .collect(),
        script_errors: load.script_errors,
        requests: load.requests,
        unwatched_frames: load.unwatched_frames,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_outcome_is_the_report_the_bundler_reads() {
        let outcome = CheckOutcome {
            engine: "WebKit 625.1".into(),
            elapsed: Duration::from_millis(1234),
            loads: vec![LoadOutcome {
                path: "index.html".into(),
                title: "Orbitarium".into(),
                first_heading: "The sky".into(),
                text: "The sky at night.".into(),
                finished: true,
                navigation_error: None,
                crashed: false,
                missing: vec!["a.png".into()],
                blocked: vec![("https://x.test/data".into(), "connect-src".into())],
                script_errors: vec!["boom".into()],
                requests: 4,
                unwatched_frames: vec!["f.svg".into()],
            }],
            failure: None,
        };
        let report = report(outcome);
        assert_eq!(report.engine, "WebKit 625.1");
        assert_eq!(report.elapsed_ms, 1234);
        assert_eq!(report.failure, None);
        let load = &report.loads[0];
        assert_eq!(load.path, "index.html");
        assert_eq!(load.title, "Orbitarium");
        assert_eq!(load.first_heading, "The sky");
        assert_eq!(load.text, "The sky at night.");
        assert!(load.finished && !load.crashed);
        assert_eq!(load.missing, ["a.png"]);
        assert_eq!(
            load.blocked,
            [Blocked {
                url: "https://x.test/data".into(),
                directive: "connect-src".into()
            }]
        );
        assert_eq!(load.script_errors, ["boom"]);
        assert_eq!(load.requests, 4);
        assert_eq!(load.unwatched_frames, ["f.svg"]);
        // It travels as the one line the parent reads.
        let line = serde_json::to_string(&report).unwrap();
        assert!(!line.contains('\n'));
        assert_eq!(serde_json::from_str::<CheckReport>(&line).unwrap(), report);
    }

    #[test]
    fn other_command_lines_are_not_children() {
        assert!(run(&["list".to_string()]).is_none());
        assert!(run(&[]).is_none());
    }
}
