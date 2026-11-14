//! The skill against the real `writer` binary: every command the skill shows exists, and
//! every option its examples use is one the command takes.
//!
//! The binary is the one of this build (`cargo build -p writer-cli` into the directory this
//! test was built into); the test builds it first, so it never checks a stale one. It is
//! only ever asked for help, and with a temporary library directory in its environment, so
//! the user's library is not opened whatever the binary does.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use writer_agent_setup::{SkillSpec, example_lines, mentioned_commands};

/// `<target dir>/<profile>`, from the path of this test's own executable
/// (`<target dir>/<profile>/deps/<name>-<hash>`).
fn profile_dir() -> PathBuf {
    let exe = std::env::current_exe().expect("the path of the test executable");
    exe.parent()
        .and_then(Path::parent)
        .expect("target/<profile>/deps/<test>")
        .to_path_buf()
}

/// The `writer` binary of this build, built now.
fn writer() -> &'static Path {
    static BINARY: OnceLock<PathBuf> = OnceLock::new();
    BINARY.get_or_init(|| {
        let profile_dir = profile_dir();
        let target_dir = profile_dir.parent().expect("the target directory");
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let mut build = Command::new(cargo);
        build
            .args(["build", "-p", "writer-cli", "--bin", "writer"])
            .arg("--target-dir")
            .arg(target_dir);
        if profile_dir
            .file_name()
            .is_some_and(|name| name == "release")
        {
            build.arg("--release");
        }
        let output = build.output().expect("run cargo build -p writer-cli");
        assert!(
            output.status.success(),
            "cargo build -p writer-cli failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let binary = profile_dir.join("writer");
        assert!(binary.is_file(), "no writer binary at {}", binary.display());
        binary
    })
}

/// The help of a command, or of the tool when `words` is empty. Fails when the tool does.
fn help(words: &[String]) -> String {
    let library = tempfile::tempdir().expect("a temporary library directory");
    let output = Command::new(writer())
        .args(words)
        .arg("--help")
        .env("THE_WRITER_DATA_DIR", library.path())
        .env("HOME", library.path())
        .output()
        .expect("run writer");
    assert!(
        output.status.success(),
        "writer {} --help failed ({}): {}",
        words.join(" "),
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("help is text")
}

/// The words of an example line the way the shell would split them, without running it.
fn shell_words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut started = false;
    let mut quote: Option<char> = None;
    for c in line.chars() {
        match quote {
            Some(open) if c == open => quote = None,
            Some(_) => word.push(c),
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                started = true;
            }
            None if c.is_whitespace() => {
                if started || !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
                started = false;
            }
            None => word.push(c),
        }
    }
    assert!(quote.is_none(), "an open quote in: {line}");
    if started || !word.is_empty() {
        words.push(word);
    }
    words
}

#[test]
fn every_command_the_skill_shows_exists_in_the_binary() {
    let commands = mentioned_commands();
    assert!(commands.len() >= 25, "{commands:?}");
    for command in &commands {
        let text = help(command);
        // The help of a command starts with its full name. A word the tool does not know
        // after a group (`comment frobnicate`) gets the help of the group's list instead,
        // which this catches.
        let heading = format!("writer {}:", command.join(" "));
        assert!(
            text.starts_with(&heading),
            "the skill shows \"writer {}\", and the tool answers its --help with: {}",
            command.join(" "),
            text.lines().next().unwrap_or_default()
        );
    }
}

#[test]
fn every_option_the_examples_use_is_one_the_command_takes() {
    let commands = mentioned_commands();
    let overview = help(&[]);
    let mut checked = 0;
    for line in example_lines() {
        let words = shell_words(line);
        assert_eq!(words[0], "writer");
        // A line without a command asks the tool itself for something (`writer --help`).
        let none = Vec::new();
        let command = commands
            .iter()
            .filter(|command| words[1..].starts_with(command))
            .max_by_key(|command| command.len())
            .unwrap_or(&none);
        assert!(
            !command.is_empty() || words[1].starts_with("--"),
            "no command the skill shows in: {line}"
        );
        // The options a help text lists: the lines that start with one (`--old <text>`,
        // `-h, --help`), from the command's own help and from the options of every command.
        let text = help(command);
        let options: Vec<&str> = text
            .lines()
            .chain(overview.lines())
            .map(str::trim_start)
            .filter(|help_line| help_line.starts_with('-'))
            .filter_map(|help_line| {
                help_line
                    .split([' ', ','])
                    .find(|word| word.starts_with("--"))
            })
            .collect();
        for word in &words[1 + command.len()..] {
            if let Some(option) = word.strip_prefix("--") {
                assert!(
                    options.contains(&word.as_str()),
                    "\"{line}\" uses --{option}, which \"writer {} --help\" does not list",
                    command.join(" ")
                );
                checked += 1;
            }
        }
    }
    assert!(checked >= 20, "{checked} options checked");
}

/// The exit codes the skill explains are the ones the tool documents: every code of the
/// tool has a line in the skill, the skill explains no code the tool does not have, and
/// each line speaks of what the tool says the code means.
#[test]
fn the_exit_codes_of_the_skill_are_the_ones_of_the_tool() {
    let overview = help(&[]);
    let codes = overview
        .split("Exit codes:")
        .nth(1)
        .expect("the overview lists the exit codes");
    let codes: String = codes.split_whitespace().collect::<Vec<_>>().join(" ");
    // "0 done, 1 failed, 2 wrong command line or request, ...": a code is a number that
    // opens the list or follows ", ", and its meaning runs to the next one.
    let mut of_the_tool: Vec<(u8, String)> = Vec::new();
    for part in codes.trim_end_matches('.').split(", ") {
        let (number, meaning) = part.split_once(' ').unwrap_or((part, ""));
        match number.parse::<u8>() {
            Ok(code) => of_the_tool.push((code, meaning.to_string())),
            // A comma inside a meaning ("9 in the trash, or not, and ...").
            Err(_) => {
                let last = of_the_tool.last_mut().expect("the list opens with a code");
                last.1.push_str(", ");
                last.1.push_str(part);
            }
        }
    }
    let numbers: Vec<u8> = of_the_tool.iter().map(|(code, _)| *code).collect();
    assert_eq!(
        numbers,
        (0..=15).collect::<Vec<u8>>(),
        "the exit codes of the tool changed: bring the skill up to date ({codes})"
    );

    let skill = SkillSpec::new("/Applications/The Writer.app/Contents/MacOS/writer")
        .unwrap()
        .render();
    let section = skill
        .split("## Exit codes")
        .nth(1)
        .expect("the skill has a part about exit codes");
    let of_the_skill: Vec<(u8, &str)> = section
        .lines()
        .filter_map(|line| line.strip_prefix("- "))
        .filter_map(|line| line.split_once(": "))
        .filter_map(|(number, meaning)| Some((number.parse::<u8>().ok()?, meaning)))
        .collect();
    assert_eq!(
        of_the_skill
            .iter()
            .map(|(code, _)| *code)
            .collect::<Vec<u8>>(),
        numbers,
        "the skill explains other exit codes than the tool has"
    );
    // A word of what the tool says each code means, which the line of the skill has too.
    for (code, word) in [
        (0, "done"),
        (1, "failed"),
        (2, "command line"),
        (3, "not found"),
        (4, "a name fits several items"),
        (5, "The Writer is open"),
        (6, "not in the document"),
        (7, "occurs"),
        (8, "--permanently"),
        (9, "in the trash"),
        (10, "no library"),
        (11, "another tool is writing"),
        (12, "exists"),
        (13, "cannot be read"),
        (14, "kept whole"),
        (15, "a page where a document is needed"),
    ] {
        let tool = &of_the_tool[code as usize].1;
        let skill = of_the_skill[code as usize].1;
        assert!(
            tool.contains(word),
            "the tool no longer says \"{word}\" for exit code {code}: {tool}"
        );
        assert!(
            skill.contains(word),
            "the skill does not say \"{word}\" for exit code {code}: {skill}"
        );
    }
}

/// The examples of pages run against the built tool, as an agent runs them: through the
/// shell (which turns `~` into the home directory), with the fake check
/// (`THE_WRITER_PAGE_CHECK=fake`, which a debug build reads), and a library, a home and a
/// temporary directory that exist only for this test. Each one has to succeed.
#[test]
fn the_examples_of_pages_run_against_the_built_tool() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let root = root.path().canonicalize().unwrap();
    let (home, library, work, tmp) = (
        root.join("home"),
        root.join("library"),
        root.join("work"),
        root.join("tmp"),
    );
    for dir in [&home, &library, &work, &tmp, &home.join("Desktop")] {
        std::fs::create_dir_all(dir).unwrap();
    }
    let explainer = home.join("Downloads/explainer");
    std::fs::create_dir_all(&explainer).unwrap();
    std::fs::write(
        explainer.join("index.html"),
        "<!doctype html><title>Explainer</title><h1>How tides work</h1><p>The moon.</p>",
    )
    .unwrap();
    let second = work.join("second");
    std::fs::create_dir_all(&second).unwrap();
    std::fs::write(
        second.join("index.html"),
        "<title>Second</title><p>Two.</p>",
    )
    .unwrap();
    std::fs::write(
        work.join("pages.json"),
        r#"[{"path": "second", "source_url": "https://claude.ai/artifact/second"}]"#,
    )
    .unwrap();
    let run = |line: &str| {
        let output = Command::new("/bin/sh")
            .arg("-c")
            .arg(line)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &home)
            .env("TMPDIR", &tmp)
            .env("THE_WRITER_DATA_DIR", &library)
            .env("THE_WRITER_PAGE_CHECK", "fake")
            .current_dir(&work)
            .output()
            .expect("run the shell");
        assert!(
            output.status.success(),
            "{line}: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).expect("output is text")
    };
    let tool = format!("'{}'", writer().display());
    // The examples name page:12: eleven documents come first.
    for n in 1..=11 {
        run(&format!("{tool} create \"Note {n}\" --content \"Note.\""));
    }
    let examples: Vec<&str> = example_lines()
        .into_iter()
        .filter(|line| line.contains("page:12") || line.starts_with("writer page"))
        .collect();
    assert!(examples.len() >= 5, "{examples:?}");
    let mut said = Vec::new();
    for line in examples {
        let line = line.replacen("writer", &tool, 1);
        said.push(run(&line));
    }
    let all = said.concat();
    assert!(
        all.contains("added page:12 \u{201c}Explainer\u{201d} in Claude Pages"),
        "{all}"
    );
    assert!(
        all.contains("added page:13 \u{201c}Second\u{201d}"),
        "{all}"
    );
    assert!(all.contains("How tides work"), "{all}");
    assert!(home.join("Desktop/page.pdf").is_file());
}

#[test]
fn shell_words_are_split_as_the_shell_splits_them() {
    assert_eq!(
        shell_words("writer comment add doc:12 --quote \"a b\" --body 'c d' --author Claude"),
        vec![
            "writer", "comment", "add", "doc:12", "--quote", "a b", "--body", "c d", "--author",
            "Claude"
        ]
    );
    assert_eq!(
        shell_words("writer edit doc:1 --new \"\""),
        vec!["writer", "edit", "doc:1", "--new", ""]
    );
}
