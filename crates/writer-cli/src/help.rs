//! The help texts, generated from the command table ([`crate::spec`]).
//!
//! The help is the manual: an agent reads `writer --help` before anything else, so the
//! overview names every command on one line, and the help of a command says exactly what
//! it takes and shows command lines that work as written.

use writer_mcp::ErrorKind;

use crate::spec::{self, COMMANDS, Content, Group, Spec};

/// The version of the app this binary belongs to.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// `writer --version`.
pub fn version() -> String {
    format!("writer {VERSION}\n")
}

/// The text that says how documents and folders are named. Shared by the overview and
/// by the help of every command that takes one.
const ADDRESSING: &str = "\
A document or a folder is named by its id (doc:12, folder:3; every command prints them)
or by its path in the library (\"The Salt Road/Part one/Chapter 1\"; the last part alone
is enough when no other item has that name; a path that starts with / starts at the top).
A name that fits several items is an error that lists them with their ids.";

/// What the help of every command that takes Markdown says about images.
const IMAGES_IN_MARKDOWN: &str = "\
An image is a line of its own: ![description](image:<id>) names an image the library has
(\"writer get\" prints them so), ![description](picture.png) brings one in from a file. A
relative path is taken from the folder of --file, else from where the command runs; an
address on the web stays a link.
";

/// `writer --help`: every command on one line.
pub fn overview() -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "writer {VERSION}: the library of The Writer for agents and scripts. It works on the\n\
library itself; the app does not have to be running, and may be.\n\n\
Usage: writer <command> [arguments] [options]      writer <command> --help\n"
    ));
    let width = COMMANDS
        .iter()
        .map(|spec| left_column(spec).chars().count())
        .max()
        .unwrap_or(0);
    for group in Group::ALL {
        out.push_str(&format!("{}\n", group.heading()));
        for spec in COMMANDS.iter().filter(|spec| spec.group == group) {
            let left = left_column(spec);
            let pad = width - left.chars().count();
            out.push_str(&format!("  {left}{}  {}\n", " ".repeat(pad), spec.summary));
        }
    }
    out.push_str("Options for every command\n");
    for (option, meaning) in [
        (
            "--library <dir>",
            "The library to work on (default: the app's own, or $THE_WRITER_DATA_DIR)",
        ),
        (
            "--json",
            "Print the result as JSON (an error: one JSON line on standard error)",
        ),
        (
            "--dry-run",
            "Say what a command would change, and change nothing",
        ),
        (
            "--author <name>",
            "Who makes the change: labels the version kept before it, signs a comment",
        ),
        (
            "-h, --help",
            "This list; after a command, its help with examples",
        ),
        ("-V, --version", "The version"),
    ] {
        out.push_str(&format!("  {option:<15}  {meaning}\n"));
    }
    out.push('\n');
    out.push_str(ADDRESSING);
    out.push_str(
        "\n\nWhile The Writer is open, a change is carried out by the app: the user sees it in the\n\
window at once and can undo it. When the app does not make it, the command ends with\n\
exit code 5 and says why and what to do. Text is given with --content, --file <path> or\n\
--stdin. Nothing is lost: the trash and the versions bring things back.\n\n",
    );
    out.push_str(&exit_codes());
    out
}

/// A few words for each exit code.
fn meaning(kind: ErrorKind) -> &'static str {
    match kind {
        ErrorKind::Failed => "failed",
        ErrorKind::Invalid => "wrong command line or request",
        ErrorKind::NotFound => "not found",
        ErrorKind::Ambiguous => "a name fits several items",
        ErrorKind::AppIsOpen => "The Writer is open and did not make the change",
        ErrorKind::NoMatch => "the text is not in the document",
        ErrorKind::NotUnique => "the text occurs several times",
        ErrorKind::ConfirmationRequired => "needs --permanently",
        ErrorKind::WrongState => "in the trash, or not, and the opposite is needed",
        ErrorKind::LibraryNotFound => "no library there",
        ErrorKind::Busy => "another tool is writing",
        ErrorKind::FileExists => "the file exists",
        ErrorKind::Unreadable => "the document cannot be read",
    }
}

/// The exit codes, as the overview prints them.
pub fn exit_codes() -> String {
    let mut entries = vec!["0 done".to_string()];
    entries.extend(
        ErrorKind::ALL
            .iter()
            .map(|kind| format!("{} {}", kind.exit_code(), meaning(*kind))),
    );
    let mut out = String::from("Exit codes:");
    let mut line = out.len();
    let last = entries.len() - 1;
    for (index, entry) in entries.into_iter().enumerate() {
        let entry = format!(" {entry}{}", if index < last { "," } else { "." });
        if line + entry.len() > 96 {
            out.push_str("\n ");
            line = 1;
        }
        line += entry.len();
        out.push_str(&entry);
    }
    out.push('\n');
    out
}

/// The command with what follows it, as the overview shows it. A group's `list` is shown
/// the short way (`comments <document>`).
fn left_column(spec: &Spec) -> String {
    let name = match spec.name {
        "image list" => "images",
        "comment list" => "comments",
        "history list" => "history",
        "agents list" => "agents",
        name => name,
    };
    if spec.synopsis.is_empty() {
        name.to_string()
    } else {
        format!("{name} {}", spec.synopsis)
    }
}

/// `writer <command> --help`.
pub fn command(spec: &Spec) -> String {
    let mut out = String::new();
    out.push_str(&format!("writer {}: {}\n\n", spec.name, spec.summary));
    let (head, tail) = (spec::usage_head(spec), spec::usage_tail(spec));
    if tail.is_empty() {
        out.push_str(&format!("Usage: {head}\n"));
    } else if head.chars().count() + tail.chars().count() <= 88 {
        out.push_str(&format!("Usage: {head} {tail}\n"));
    } else {
        out.push_str(&format!("Usage: {head}\n         {tail}\n"));
    }
    if !spec.aliases.is_empty() {
        let aliases: Vec<String> = spec
            .aliases
            .iter()
            .map(|alias| format!("writer {alias}"))
            .collect();
        out.push_str(&format!("Also:  {}\n", aliases.join(", ")));
    }
    out.push('\n');
    out.push_str(&wrap(spec.about, 88));

    let mut rows: Vec<(String, String)> = Vec::new();
    for positional in spec.positionals {
        rows.push((
            format!("<{}>", positional.name),
            positional.help.to_string(),
        ));
    }
    if !rows.is_empty() {
        out.push_str("\nArguments\n");
        out.push_str(&table(&rows));
    }

    let mut rows: Vec<(String, String)> = Vec::new();
    for option in spec.options {
        rows.push((option_label(option), option.help.to_string()));
    }
    if spec.content != Content::None {
        for option in &spec::CONTENT_OPTIONS {
            rows.push((option_label(option), option.help.to_string()));
        }
    }
    if spec.dry_run {
        rows.push((
            "--dry-run".into(),
            "Say what would change, and change nothing".into(),
        ));
    }
    if spec.writes {
        rows.push((
            "--author <name>".into(),
            "Who makes the change: labels the version kept before it, signs a comment".into(),
        ));
    }
    rows.push(("--json".into(), "Print the result as JSON".into()));
    rows.push((
        "--library <dir>".into(),
        "The library to work on instead of the app's own".into(),
    ));
    out.push_str("\nOptions\n");
    out.push_str(&table(&rows));

    match spec.content {
        Content::None => {}
        Content::Optional => out.push_str(
            "\nThe text is Markdown. Standard input is read only with --stdin or --file -.\n",
        ),
        Content::Required => out.push_str(
            "\nThe text is Markdown. Without --content and --file it is read from standard input\n\
when that is a pipe or a file.\n",
        ),
    }
    if spec.content != Content::None {
        out.push_str(IMAGES_IN_MARKDOWN);
    }
    if spec.writes {
        out.push_str(
            "\nThis command changes the library. While The Writer is open the app makes the change\n\
and shows it to the user at once. Exit code 5: the app is open and did not make it (the\n\
message says why: changes by agents are switched off in the app, or it is another\n\
version than this tool, or it did not answer).\n",
        );
    }
    if spec
        .positionals
        .iter()
        .any(|p| matches!(p.name, "document" | "folder" | "item"))
    {
        out.push('\n');
        out.push_str(ADDRESSING);
        out.push('\n');
    }

    // The other commands of the same family (`comment add` beside `comment list`): whoever
    // asks for the help of one usually wants to know the others exist.
    if let Some((family, _)) = spec.name.split_once(' ') {
        let siblings: Vec<&Spec> = COMMANDS
            .iter()
            .filter(|other| other.name != spec.name)
            .filter(|other| {
                other
                    .name
                    .split_once(' ')
                    .is_some_and(|(first, _)| first == family)
            })
            .collect();
        let rows: Vec<(String, String)> = siblings
            .iter()
            .map(|other| {
                (
                    format!("writer {}", left_column(other)),
                    other.summary.to_string(),
                )
            })
            .collect();
        out.push_str("\nSee also\n");
        out.push_str(&table(&rows));
    }

    out.push_str(if spec.examples.len() == 1 {
        "\nExample\n"
    } else {
        "\nExamples\n"
    });
    for example in spec.examples {
        out.push_str(&format!("  {example}\n"));
    }
    out
}

fn option_label(option: &spec::Opt) -> String {
    match option.value {
        Some(value) => format!("--{} <{value}>", option.name),
        None => format!("--{}", option.name),
    }
}

/// Two columns, the second wrapped under itself.
fn table(rows: &[(String, String)]) -> String {
    let width = rows
        .iter()
        .map(|(left, _)| left.chars().count())
        .max()
        .unwrap_or(0);
    let mut out = String::new();
    for (left, right) in rows {
        let pad = " ".repeat(width - left.chars().count());
        let indent = " ".repeat(width + 4);
        let wrapped = wrap(right, 88usize.saturating_sub(width + 4).max(30));
        for (index, line) in wrapped.lines().enumerate() {
            if index == 0 {
                out.push_str(&format!("  {left}{pad}  {line}\n"));
            } else {
                out.push_str(&format!("{indent}{line}\n"));
            }
        }
    }
    out
}

/// Breaks a paragraph into lines of at most `width` characters, at spaces.
fn wrap(text: &str, width: usize) -> String {
    let mut out = String::new();
    let mut line = 0;
    for word in text.split_whitespace() {
        let length = word.chars().count();
        if line > 0 && line + 1 + length > width {
            out.push('\n');
            line = 0;
        }
        if line > 0 {
            out.push(' ');
            line += 1;
        }
        out.push_str(word);
        line += length;
    }
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_overview_names_every_command_on_one_line_and_fits_a_screen() {
        let overview = overview();
        for spec in COMMANDS {
            let left = left_column(spec);
            let line = overview
                .lines()
                .find(|line| line.trim_start().starts_with(&left) && line.contains(spec.summary))
                .unwrap_or_else(|| panic!("{} is not in the overview", spec.name));
            assert!(line.chars().count() <= 124, "{line}");
        }
        for word in [
            "--library",
            "--json",
            "--dry-run",
            "--help",
            "--version",
            "doc:12",
            "Exit codes",
            "exit code 5",
        ] {
            assert!(
                overview.contains(word),
                "the overview does not mention {word}"
            );
        }
        // (70 before the two commands about images.)
        assert!(
            overview.lines().count() <= 72,
            "the overview has {} lines",
            overview.lines().count()
        );
        assert!(
            overview.contains("\n  --json           Print"),
            "the options are indented"
        );
        assert!(overview.contains("5 The Writer is open and did not make the change"));
        assert!(overview.contains("While The Writer is open, a change is carried out by the app"));
        assert!(overview.contains("13 the document cannot be read."));
        assert!(overview.is_ascii(), "the overview is plain ASCII");
    }

    #[test]
    fn the_help_of_a_command_has_usage_options_and_examples() {
        for spec in COMMANDS {
            let help = command(spec);
            assert!(
                help.contains(&format!("Usage: writer {}", spec.name)),
                "{}",
                spec.name
            );
            if spec.name == "insert" {
                assert!(help.contains("--end-of <heading>)\n         (--content <markdown> | --file <path> | --stdin) [options]"));
            }
            for option in spec::options(spec) {
                assert!(
                    help.contains(&format!("--{}", option.name)),
                    "{}: --{}",
                    spec.name,
                    option.name
                );
            }
            for positional in spec.positionals {
                assert!(
                    help.contains(&format!("<{}>", positional.name)),
                    "{}: <{}>",
                    spec.name,
                    positional.name
                );
            }
            for example in spec.examples {
                assert!(help.contains(example), "{}: {example}", spec.name);
            }
            assert_eq!(help.contains("--dry-run"), spec.dry_run, "{}", spec.name);
            assert_eq!(
                help.contains("--author <name>"),
                spec.writes,
                "{}",
                spec.name
            );
            assert!(help.is_ascii(), "{}: the help is plain ASCII", spec.name);
            for line in help.lines() {
                assert!(line.chars().count() <= 120, "{}: {line}", spec.name);
            }
        }
    }

    #[test]
    fn wrapping() {
        assert_eq!(wrap("one two three", 7), "one two\nthree\n");
        assert_eq!(wrap("a", 1), "a\n");
        assert_eq!(wrap("abcdefgh ij", 4), "abcdefgh\nij\n");
    }
}
