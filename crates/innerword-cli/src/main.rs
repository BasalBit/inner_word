//! `innerword`: the library of Inner Word from the command line, for agents and scripts.
//!
//! The tool works on the library directly, through the command layer (`innerword-agent`); the
//! app does not have to be running. While it is, everything that reads works as it does,
//! and a change is handed to the app, which makes it and shows it (`innerword_agent::live`;
//! nothing in this crate knows of it). It ships inside the app bundle
//! (`Inner Word.app/Contents/MacOS/innerword`) and is called by its full path.
//!
//! * [`spec`] is the table of commands; [`parse`] reads a command line against it and
//!   [`help`] prints it, so the help is the manual and cannot drift from the parser.
//! * [`run`] turns a command into one operation of `innerword_mcp::Toolbox`, the same table
//!   the MCP server (`innerword mcp`) offers as tools.
//! * [`agents`] is `innerword agents ...`, over `innerword-agent-setup`.
//!
//! # Output
//!
//! The result goes to standard output and nothing else does: text by default, JSON with
//! `--json`. A failure goes to standard error (`writer: <message>`, or with `--json` one
//! line `{"error": {"code": ..., "message": ...}}`) and sets the exit code:
//!
//! | Code | Meaning |
//! |---|---|
//! | 0 | done |
//! | 1 | failed (the library, a file, anything unexpected) |
//! | 2 | the command line or the request is wrong |
//! | 3 | not found |
//! | 4 | a name fits several items (the message lists them with their ids) |
//! | 5 | Inner Word is open and did not make the change (the message says why) |
//! | 6 | the text to replace or to comment on is not in the document |
//! | 7 | that text occurs several times |
//! | 8 | erasing for good needs `--permanently` |
//! | 9 | the item is in the trash, or is not, and the command needs the opposite |
//! | 10 | there is no library at that place |
//! | 11 | another tool is writing to the library |
//! | 12 | the file an export would write exists |
//! | 13 | the stored document cannot be read |
//! | 14 | the page could not be kept whole; nothing was saved |
//! | 15 | a page where a document is needed, or the reverse |
//!
//! The codes are `innerword_agent::ErrorKind::exit_code`.
//!
//! # Pages
//!
//! `innerword page add` reads, fetches and checks a page in this process and its child: the
//! check loads the page in WebKit, which runs only in `innerword __page-check` (and the PDF
//! of a page in `innerword __page-pdf`), never in the tool or the MCP server themselves
//! ([`page_check`]). [`main`] installs that checker on every library it opens.

mod agents;
mod help;
mod page_check;
mod pages;
mod parse;
mod run;
mod spec;

use std::io::Write;
use std::process::ExitCode;
use std::sync::Arc;

use innerword_mcp::{ErrorKind, Failure, Library, Reply, Toolbox};
use serde_json::json;

use parse::{Action, Globals};

fn main() -> ExitCode {
    let mut arguments = Vec::new();
    for argument in std::env::args_os().skip(1) {
        match argument.into_string() {
            Ok(argument) => arguments.push(argument),
            Err(_) => {
                return fail(false, &Failure::invalid("an argument is not valid UTF-8"));
            }
        }
    }
    // The children of a page's check and of its PDF are programs of their own.
    if let Some(code) = page_check::run(&arguments) {
        return code;
    }
    // `--json` shapes the error output too, also when the command line cannot be read.
    let wants_json = arguments
        .iter()
        .take_while(|argument| *argument != "--")
        .any(|argument| argument == "--json");
    let (globals, action) = match parse::parse(arguments) {
        Ok(parsed) => parsed,
        Err(parse::UsageError(message)) => return fail(wants_json, &Failure::invalid(message)),
    };
    match action {
        Action::Overview => print(&help::overview()),
        Action::Help(spec) => print(&help::command(spec)),
        Action::Version => print(&help::version()),
        Action::Run(spec, parsed) => {
            if spec.name == "mcp" {
                return serve(&globals);
            }
            if spec.name == "page add" {
                return add_pages(spec, &parsed, &globals);
            }
            let outcome = if spec.name.starts_with("agents ") {
                agents::run(spec, &parsed, &globals)
            } else {
                library(&globals).and_then(|library| {
                    // An image file named by a relative path is looked for from the
                    // folder of the file the text came from, else from here. (The MCP
                    // server is given no such folder: it takes whole paths.)
                    let library = match run::base_dir(&parsed) {
                        Some(folder) => library.with_base_dir(folder),
                        None => library,
                    };
                    run::run(&Toolbox::new(library), spec, &parsed, &globals)
                })
            };
            match outcome {
                Ok(reply) => print(&render(&reply, globals.json)),
                Err(failure) => fail_in(Some(spec), globals.json, &failure),
            }
        }
    }
}

/// The library the command works on: `--library`, else the app's own. Its pages are
/// checked (and printed) by this program, run again as a child.
fn library(globals: &Globals) -> Result<Library, Failure> {
    Library::locate(globals.library.as_deref())
        .map(|library| {
            library.with_page_checker(Arc::new(innerword_agent::ProcessChecker::current_exe()))
        })
        .map_err(Failure::from)
}

/// `innerword page add`: every page is tried, and each has its line. One page named on the
/// command line is a command like any other (its failure goes to standard error); several
/// pages, or a list, print a line for each, the failures among them, and end with the
/// exit code of the first page that failed.
fn add_pages(spec: &'static spec::Spec, parsed: &parse::Parsed, globals: &Globals) -> ExitCode {
    let library = match library(globals) {
        Ok(library) => library,
        Err(failure) => return fail_in(Some(spec), globals.json, &failure),
    };
    let library = match run::base_dir(parsed) {
        Some(folder) => library.with_base_dir(folder),
        None => library,
    };
    match pages::add(&Toolbox::new(library), spec, parsed, globals) {
        Err(failure) => fail_in(Some(spec), globals.json, &failure),
        Ok(added) => {
            let printed = print(&render(&added.reply, globals.json));
            let Some(failure) = added.first_failure else {
                return printed;
            };
            let mut stderr = std::io::stderr().lock();
            let hint = hint_in(Some(spec), &failure);
            let _ = if globals.json {
                let mut data = failure.data.clone();
                if let (Some(hint), Some(fields)) = (hint, data.as_object_mut()) {
                    fields.insert("hint".into(), json!(hint));
                }
                writeln!(stderr, "{}", json!({ "error": data }))
            } else {
                let summary = format!(
                    "innerword: {} of {} {} not added (the lines above say why); the exit code is \
                     the first one's",
                    added.failed,
                    added.tried,
                    if added.tried == 1 {
                        "page was"
                    } else {
                        "pages were"
                    }
                );
                match hint {
                    Some(hint) => writeln!(stderr, "{summary}\n{hint}"),
                    None => writeln!(stderr, "{summary}"),
                }
            };
            ExitCode::from(failure.kind.exit_code())
        }
    }
}

/// `innerword mcp`: the MCP server on standard input and output.
fn serve(globals: &Globals) -> ExitCode {
    let library = match library(globals) {
        Ok(library) => library,
        Err(failure) => return fail(false, &failure),
    };
    match innerword_mcp::serve_stdio(library) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => fail(
            false,
            &Failure::new(
                ErrorKind::Failed,
                format!("the MCP server stopped: {error}"),
            ),
        ),
    }
}

/// What a reply prints: its text, or its data as JSON.
fn render(reply: &Reply, as_json: bool) -> String {
    if !as_json {
        return reply.text.clone();
    }
    let mut out =
        serde_json::to_string_pretty(&reply.data).unwrap_or_else(|_| reply.data.to_string());
    out.push('\n');
    out
}

/// Writes the result to standard output. A reader that went away (`innerword list | head`)
/// is not a failure of the command.
fn print(text: &str) -> ExitCode {
    let mut stdout = std::io::stdout().lock();
    match stdout
        .write_all(text.as_bytes())
        .and_then(|()| stdout.flush())
    {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(error) => fail(
            false,
            &Failure::new(
                ErrorKind::Failed,
                format!("the result cannot be written: {error}"),
            ),
        ),
    }
}

/// What the command line adds to a message of the command layer: the layer serves the
/// MCP server too and names its fields ("occurrence", "permanently"), and somebody at a
/// command line has to know the option to type next.
fn hint(kind: ErrorKind) -> Option<&'static str> {
    match kind {
        ErrorKind::AppIsOpen => Some(
            "Reading works while Inner Word is open (list, search, get, find, comments, \
             history, export), and so does every command with --dry-run. \"innerword status\" \
             says whether the app takes changes.",
        ),
        ErrorKind::LibraryNotFound => Some("Here that option is --library <dir>."),
        ErrorKind::NoMatch => Some(
            "\"innerword find <document> <text>\" shows where a text stands; \"innerword get \
             <document> --format text\" prints the text as it is matched (an image has no \
             text there; in Markdown it is its line).",
        ),
        ErrorKind::NotUnique => Some(
            "Here: --occurrence <n> takes the one with that number; \"innerword edit\" also has \
             --all for every one of them.",
        ),
        ErrorKind::ConfirmationRequired => Some("Here that option is --permanently."),
        ErrorKind::FileExists => Some("Here: --overwrite replaces the file."),
        _ => None,
    }
}

/// The hint after an export of a page in a format of a text.
const EXPORT_PAGE: &str = "Here: a page is written as a folder (\"innerword export <page> <folder>\"), as \
a PDF (a path ending in .pdf) or as its text (.txt).";

/// The hint after a text operation on a page: how a newer copy of a page comes in.
const NEWER_PAGE: &str = "Here: \"innerword page add <file or folder> --source-url <address>\" with the \
address the page came from brings a newer copy (\"innerword info <page>\" shows the address).";

/// The hint for a failure of `spec`, the command that ran: the options a message of the
/// command layer means are the options of that command.
fn hint_in(spec: Option<&spec::Spec>, failure: &Failure) -> Option<&'static str> {
    let has = |name: &str| spec.is_some_and(|spec| spec.options.iter().any(|o| o.name == name));
    // Several headings with one text are told apart by number, items by id.
    let headings = failure.kind == ErrorKind::Ambiguous
        && failure.data["candidates"][0]["kind"] == json!("heading");
    if headings {
        return Some(if has("heading-occurrence") {
            "Here: --heading-occurrence <n> takes the heading with that number."
        } else {
            "Here: --occurrence <n> takes the heading with that number."
        });
    }
    // A place at a heading (`innerword image add --at`).
    if failure.kind == ErrorKind::Invalid && has("at") && has("heading") {
        if failure.message == innerword_mcp::POSITION_NEEDS_A_HEADING {
            return Some(
                "Here: --heading <text> names it; \"innerword info <document>\" lists the headings.",
            );
        }
        if failure.message == innerword_mcp::POSITION_TAKES_NO_HEADING {
            return Some(
                "Here: --at before-heading, after-heading or end-of-section, with --heading \
                 <text>; without --heading the place is --at end or --at start.",
            );
        }
    }
    if failure.kind == ErrorKind::WrongKind
        && failure
            .message
            .contains(innerword_agent::PAGE_IS_NOT_A_TEXT_FILE)
    {
        return Some(EXPORT_PAGE);
    }
    if failure.kind == ErrorKind::WrongKind && failure.message.contains(" is a page: ") {
        return Some(NEWER_PAGE);
    }
    hint(failure.kind)
}

/// Writes a failure to standard error and gives its exit code.
fn fail(as_json: bool, failure: &Failure) -> ExitCode {
    fail_in(None, as_json, failure)
}

/// [`fail`] for a failure of the command `spec`.
fn fail_in(spec: Option<&spec::Spec>, as_json: bool, failure: &Failure) -> ExitCode {
    let mut stderr = std::io::stderr().lock();
    let hint = hint_in(spec, failure);
    let _ = if as_json {
        let mut data = failure.data.clone();
        if let (Some(hint), Some(fields)) = (hint, data.as_object_mut()) {
            fields.insert("hint".into(), json!(hint));
        }
        writeln!(stderr, "{}", json!({ "error": data }))
    } else if let Some(hint) = hint {
        writeln!(stderr, "innerword: {}\n{hint}", failure.message)
    } else {
        writeln!(stderr, "innerword: {}", failure.message)
    };
    ExitCode::from(failure.kind.exit_code())
}

#[cfg(test)]
mod tests {
    use super::parse::{self, Action};
    use super::spec;

    /// Splits a command line the way a shell does, as far as the examples go: words,
    /// single and double quotes, a backslash before a character.
    fn shell_words(line: &str) -> Vec<String> {
        let mut words = Vec::new();
        let mut word = String::new();
        let mut in_word = false;
        let mut quote: Option<char> = None;
        let mut chars = line.chars();
        while let Some(c) = chars.next() {
            match (quote, c) {
                (Some(open), c) if c == open => quote = None,
                (Some('"'), '\\') => word.extend(chars.next()),
                (Some(_), c) => word.push(c),
                (None, '"' | '\'') => {
                    quote = Some(c);
                    in_word = true;
                }
                (None, '\\') => {
                    word.extend(chars.next());
                    in_word = true;
                }
                (None, c) if c.is_whitespace() => {
                    if in_word {
                        words.push(std::mem::take(&mut word));
                        in_word = false;
                    }
                }
                (None, c) => {
                    word.push(c);
                    in_word = true;
                }
            }
        }
        assert!(quote.is_none(), "a quote is left open in: {line}");
        if in_word {
            words.push(word);
        }
        words
    }

    /// The `innerword ...` commands of a line that may be a pipeline.
    fn writer_commands(line: &str) -> Vec<Vec<String>> {
        let words = shell_words(line);
        words
            .split(|word| word == "|")
            .filter(|command| command.first().is_some_and(|first| first == "innerword"))
            .map(|command| command[1..].to_vec())
            .collect()
    }

    #[track_caller]
    fn assert_reads(line: &str) {
        let commands = writer_commands(line);
        assert!(!commands.is_empty(), "no innerword command in: {line}");
        for command in commands {
            match parse::parse(command) {
                Ok((_, Action::Run(..))) => {}
                other => panic!("this tool cannot read the command line \"{line}\": {other:?}"),
            }
        }
    }

    #[test]
    fn shell_words_are_split_as_a_shell_splits_them() {
        assert_eq!(
            shell_words(r#"innerword edit "Notes/Plan" --old 'a "b"' --new "c \"d\"" x\ y"#),
            vec![
                "innerword",
                "edit",
                "Notes/Plan",
                "--old",
                "a \"b\"",
                "--new",
                "c \"d\"",
                "x y"
            ]
        );
        assert_eq!(
            shell_words(r#"innerword set doc:1 --content """#),
            vec!["innerword", "set", "doc:1", "--content", ""]
        );
    }

    #[test]
    fn every_example_in_the_help_is_a_command_line_this_tool_reads() {
        for spec in spec::COMMANDS {
            for example in spec.examples {
                assert_reads(example);
            }
            // The first example of a command is an example of that command.
            let first = writer_commands(spec.examples[0]);
            let ran = first.iter().any(|command| {
                matches!(parse::parse(command.clone()), Ok((_, Action::Run(found, _))) if found.name == spec.name)
            });
            assert!(
                ran,
                "the first example of \"innerword {}\" runs another command",
                spec.name
            );
        }
    }

    #[test]
    fn every_command_line_in_the_skill_is_one_this_tool_reads() {
        // The skill is what agents are told (crates/innerword-agent-setup). A command line in
        // it that this tool cannot read would be a wasted turn for every agent, every time.
        let lines = innerword_agent_setup::example_lines();
        assert!(lines.len() >= 20, "the skill has examples: {}", lines.len());
        for line in lines {
            if shell_words(line) == ["innerword", "--help"] {
                continue;
            }
            assert_reads(line);
        }
        for command in innerword_agent_setup::mentioned_commands() {
            let first = command.first().map(String::as_str).unwrap_or_default();
            let second = command.get(1).map(String::as_str);
            assert!(
                !matches!(spec::find(first, second), spec::Found::Unknown),
                "the skill mentions \"innerword {}\", which is not a command",
                command.join(" ")
            );
        }
    }
}
