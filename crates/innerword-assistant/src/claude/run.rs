//! Running a program to its end with a time limit, and ending the group of processes a
//! program started.

use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// What a program printed and how it ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ran {
    /// Its standard output, as text.
    pub stdout: String,
    /// Its standard error, as text.
    pub stderr: String,
    /// Its exit code; `None` when a signal ended it.
    pub code: Option<i32>,
}

/// Runs `command` in a process group of its own, writes `input` to its standard input
/// (or gives it none), and waits for it for at most `timeout`. `None` when it could not
/// be started or did not end in time (it is killed then, with everything it started).
pub fn run_with_timeout(
    mut command: Command,
    input: Option<&str>,
    timeout: Duration,
) -> Option<Ran> {
    command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = command.spawn().ok()?;
    if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
        let input = input.to_string();
        // Written on a thread: a program that does not read must not block the caller.
        std::thread::spawn(move || {
            let _ = stdin.write_all(input.as_bytes());
        });
    }
    let stdout = child.stdout.take().map(read_all_on_a_thread);
    let stderr = child.stderr.take().map(read_all_on_a_thread);
    let code = match wait_until(&mut child, Instant::now() + timeout) {
        Some(status) => status,
        None => {
            end_group(&mut child, Duration::from_millis(200));
            return None;
        }
    };
    let collect = |reader: Option<std::thread::JoinHandle<String>>| {
        reader
            .and_then(|handle| handle.join().ok())
            .unwrap_or_default()
    };
    Some(Ran {
        stdout: collect(stdout),
        stderr: collect(stderr),
        code,
    })
}

fn read_all_on_a_thread<R: Read + Send + 'static>(
    mut reader: R,
) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = reader.read_to_end(&mut bytes);
        String::from_utf8_lossy(&bytes).into_owned()
    })
}

/// Waits for `child` until `deadline`. `Some(code)` when it ended (`code` is `None` for
/// a signal), `None` when it is still running.
pub fn wait_until(child: &mut Child, deadline: Instant) -> Option<Option<i32>> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status.code()),
            Ok(None) => {}
            Err(_) => return Some(None),
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Sends `signal` to the process group `child` leads (the program and what it started,
/// such as the tools' server).
pub fn signal_group(child: &Child, signal: i32) {
    let pid = child.id() as libc::pid_t;
    if pid > 1 {
        // SAFETY: kill(2) with a negative pid signals the process group with that id;
        // the child was started as the leader of its own group, and the id is not
        // reused while the Child is not yet waited for.
        unsafe {
            libc::kill(-pid, signal);
        }
    }
}

/// Ends `child` and its group: asks it to end, gives it `grace`, then kills it. Returns
/// its exit code when it has one.
pub fn end_group(child: &mut Child, grace: Duration) -> Option<i32> {
    if let Ok(Some(status)) = child.try_wait() {
        return status.code();
    }
    signal_group(child, libc::SIGTERM);
    if let Some(code) = wait_until(child, Instant::now() + grace) {
        // The leader is gone; what it started may not be.
        signal_group(child, libc::SIGKILL);
        return code;
    }
    signal_group(child, libc::SIGKILL);
    let _ = child.kill();
    wait_until(child, Instant::now() + Duration::from_secs(2)).flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_program_runs_to_its_end_with_its_output_and_code() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "cat; echo out; echo err >&2; exit 3"]);
        let ran = run_with_timeout(command, Some("in "), Duration::from_secs(10)).unwrap();
        assert_eq!(ran.stdout, "in out\n");
        assert_eq!(ran.stderr, "err\n");
        assert_eq!(ran.code, Some(3));
    }

    #[test]
    fn a_program_that_is_not_there_or_takes_too_long_is_none() {
        let missing = Command::new("/no/such/program");
        assert!(run_with_timeout(missing, None, Duration::from_secs(1)).is_none());
        let mut slow = Command::new("/bin/sh");
        slow.args(["-c", "sleep 30 & sleep 30"]);
        let started = Instant::now();
        assert!(run_with_timeout(slow, None, Duration::from_millis(200)).is_none());
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
