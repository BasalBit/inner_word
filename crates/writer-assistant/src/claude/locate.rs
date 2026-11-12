//! Finding the `claude` program.
//!
//! An app started from the Finder has a minimal `PATH` (`/usr/bin:/bin:/usr/sbin:/sbin`),
//! so the program is not looked for on the `PATH` alone:
//!
//! 1. the folders of the `PATH` and the folders installers use, the same list the Agents
//!    panel detects agents with (`writer_agent_setup::Machine`: `~/.local/bin`, Homebrew,
//!    npm, bun, the Node versions of nvm, ...);
//! 2. failing that, the user's login shell is asked (`$SHELL -l -i -c ...`), which knows a
//!    `PATH` set in the shell's own files. The shell gets a few seconds; one that hangs or
//!    prints nonsense is given up on.

use std::path::{Path, PathBuf};
use std::time::Duration;

use writer_agent_setup::Machine;

use super::run::run_with_timeout;

/// The name of the program.
pub const PROGRAM: &str = "claude";

/// How long the login shell gets to answer.
const SHELL_PATIENCE: Duration = Duration::from_secs(6);

/// Where to look.
#[derive(Clone, Debug)]
pub struct Search {
    /// Folders to look in, in order.
    pub dirs: Vec<PathBuf>,
    /// The login shell to ask when the folders do not have the program; `None`: no
    /// shell is asked.
    pub shell: Option<PathBuf>,
}

impl Search {
    /// The search on this Mac: the `PATH`, the folders installers use, then the user's
    /// login shell (`$SHELL`, else `/bin/zsh`).
    pub fn this_mac() -> Search {
        let dirs = Machine::this_mac()
            .map(|machine| machine.command_dirs().to_vec())
            .unwrap_or_default();
        let shell = std::env::var_os("SHELL")
            .map(PathBuf::from)
            .filter(|shell| shell.is_absolute())
            .or_else(|| Some(PathBuf::from("/bin/zsh")));
        Search { dirs, shell }
    }

    /// A search that looks only below `home` and asks no shell: for tests.
    pub fn isolated(home: impl Into<PathBuf>) -> Search {
        Search {
            dirs: Machine::isolated(home).command_dirs().to_vec(),
            shell: None,
        }
    }

    /// Finds the program: the first folder that has it as an executable file, else what
    /// the login shell says.
    pub fn find(&self) -> Option<PathBuf> {
        find_in(&self.dirs).or_else(|| self.shell.as_deref().and_then(ask_shell))
    }
}

/// The first of `dirs` that holds the program as an executable file.
pub fn find_in(dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter()
        .map(|dir| dir.join(PROGRAM))
        .find(|candidate| is_executable(candidate))
}

/// Whether `path` is a file the user may run (symbolic links are followed: the native
/// installer makes `~/.local/bin/claude` a link to the current version).
pub fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

const MARK_OPEN: &str = "__THE_WRITER_FOUND__";
const MARK_CLOSE: &str = "__END__";

/// What the shell is asked. `whence -p` (zsh) and `type -P` (bash, fish) answer with
/// the file even when `claude` is also an alias or a function; `command -v` is the
/// fallback every POSIX shell has. The answer is printed between two marks so that
/// whatever else the shell's files print does not matter.
fn question() -> String {
    format!(
        "p=$(whence -p {PROGRAM} 2>/dev/null || type -P {PROGRAM} 2>/dev/null || command -v {PROGRAM} 2>/dev/null); \
         printf '{MARK_OPEN}%s{MARK_CLOSE}\\n' \"$p\""
    )
}

/// Asks a login shell where the program is.
pub fn ask_shell(shell: &Path) -> Option<PathBuf> {
    if !is_executable(shell) {
        return None;
    }
    let mut command = std::process::Command::new(shell);
    // -l: a login shell (reads the profile); -i: interactive (zsh keeps the PATH of most
    // users in .zshrc, which only an interactive shell reads).
    command.args(["-l", "-i", "-c", &question()]);
    let output = run_with_timeout(command, None, SHELL_PATIENCE)?;
    parse_shell_answer(&output.stdout)
}

/// The path between the marks, when it is an absolute path to an executable file.
pub fn parse_shell_answer(output: &str) -> Option<PathBuf> {
    let start = output.rfind(MARK_OPEN)? + MARK_OPEN.len();
    let rest = &output[start..];
    let end = rest.find(MARK_CLOSE)?;
    let path = PathBuf::from(rest[..end].trim());
    (path.is_absolute() && is_executable(&path)).then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    fn executable(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn the_program_is_found_in_the_first_folder_that_has_it_as_an_executable() {
        let home = tempfile::tempdir().unwrap();
        let first = home.path().join("first");
        let second = home.path().join("second");
        fs::create_dir_all(&first).unwrap();
        // A file that is not executable does not count.
        fs::write(first.join(PROGRAM), "not a program").unwrap();
        executable(&second.join(PROGRAM), "#!/bin/sh\n");
        assert_eq!(
            find_in(&[first.clone(), second.clone()]),
            Some(second.join(PROGRAM))
        );
        assert_eq!(find_in(&[first]), None);
        assert_eq!(find_in(&[]), None);
    }

    #[test]
    fn a_link_to_the_program_counts_and_a_dangling_one_does_not() {
        let home = tempfile::tempdir().unwrap();
        let real = home.path().join("versions/2.1.322");
        executable(&real, "#!/bin/sh\n");
        let bin = home.path().join(".local/bin");
        fs::create_dir_all(&bin).unwrap();
        std::os::unix::fs::symlink(&real, bin.join(PROGRAM)).unwrap();
        assert_eq!(find_in(std::slice::from_ref(&bin)), Some(bin.join(PROGRAM)));
        fs::remove_file(&real).unwrap();
        assert_eq!(find_in(&[bin]), None);
    }

    #[test]
    fn the_isolated_search_looks_in_the_installers_folders_below_the_home() {
        let home = tempfile::tempdir().unwrap();
        let search = Search::isolated(home.path());
        assert_eq!(search.find(), None);
        executable(&home.path().join(".local/bin").join(PROGRAM), "#!/bin/sh\n");
        assert_eq!(
            Search::isolated(home.path()).find(),
            Some(home.path().join(".local/bin").join(PROGRAM))
        );
    }

    #[test]
    fn the_login_shell_is_asked_when_no_folder_has_the_program() {
        let home = tempfile::tempdir().unwrap();
        let program = home.path().join("odd place/bin").join(PROGRAM);
        executable(&program, "#!/bin/sh\n");
        // A stand-in for a login shell whose own files put the odd place on the PATH
        // and print a greeting first, as real ones do.
        let shell = home.path().join("shell");
        executable(
            &shell,
            &format!(
                "#!/bin/sh\necho 'Welcome back'\nPATH=\"{}:$PATH\"\nexport PATH\nwhile [ \"$1\" != \"-c\" ]; do shift; done\neval \"$2\"\n",
                program.parent().unwrap().display()
            ),
        );
        let search = Search {
            dirs: vec![home.path().join("empty")],
            shell: Some(shell),
        };
        assert_eq!(search.find(), Some(program));
    }

    #[test]
    fn a_shell_that_does_not_know_the_program_or_prints_nonsense_finds_nothing() {
        let home = tempfile::tempdir().unwrap();
        let shell = home.path().join("shell");
        executable(&shell, "#!/bin/sh\necho 'claude: aliased to something'\n");
        assert_eq!(ask_shell(&shell), None);
        // Marks with a relative or missing path are not believed.
        assert_eq!(
            parse_shell_answer(&format!("{MARK_OPEN}claude{MARK_CLOSE}\n")),
            None
        );
        assert_eq!(
            parse_shell_answer(&format!("{MARK_OPEN}/no/such/claude{MARK_CLOSE}\n")),
            None
        );
        assert_eq!(
            parse_shell_answer(&format!("{MARK_OPEN}{MARK_CLOSE}\n")),
            None
        );
        assert_eq!(parse_shell_answer("nothing"), None);
        // A shell that is not there is not run.
        assert_eq!(ask_shell(&home.path().join("no-shell")), None);
    }

    #[test]
    fn a_shell_that_hangs_is_given_up_on() {
        let home = tempfile::tempdir().unwrap();
        let shell = home.path().join("shell");
        executable(&shell, "#!/bin/sh\nsleep 30\n");
        let mut command = std::process::Command::new(&shell);
        command.args(["-l", "-i", "-c", "true"]);
        let started = std::time::Instant::now();
        assert!(run_with_timeout(command, None, Duration::from_millis(300)).is_none());
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
