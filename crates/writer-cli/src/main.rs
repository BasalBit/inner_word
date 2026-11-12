//! `writer`: the library of The Writer from the command line, for agents and scripts.
//!
//! The tool works on the library directly, through the command layer (`writer-agent`); the
//! app does not have to be running, and while it is, everything that reads still works and
//! every change is refused. It ships inside the app bundle
//! (`The Writer.app/Contents/MacOS/writer`) and is called by its full path.
//!
//! * [`spec`] is the table of commands; [`parse`] reads a command line against it and
//!   [`help`] prints it, so the help is the manual and cannot drift from the parser.
//! * [`run`] turns a command into one operation of `writer_mcp::Toolbox`, the same table
//!   the MCP server (`writer mcp`) offers as tools.
//! * [`agents`] is `writer agents ...`, over `writer-agent-setup`.
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
//! | 5 | The Writer is open: nothing was changed |
//! | 6 | the text to replace or to comment on is not in the document |
//! | 7 | that text occurs several times |
//! | 8 | erasing for good needs `--permanently` |
//! | 9 | the item is in the trash, or is not, and the command needs the opposite |
//! | 10 | there is no library at that place |
//! | 11 | another tool is writing to the library |
//! | 12 | the file an export would write exists |
//! | 13 | the stored document cannot be read |
//!
//! The codes are `writer_agent::ErrorKind::exit_code`.

mod agents;
mod help;
mod parse;
mod run;
mod spec;

use std::io::Write;
use std::process::ExitCode;

use serde_json::json;
use writer_mcp::{ErrorKind, Failure, Library, Reply, Toolbox};

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
            let outcome = if spec.name.starts_with("agents ") {
                agents::run(spec, &parsed, &globals)
            } else {
                library(&globals)
                    .and_then(|library| run::run(&Toolbox::new(library), spec, &parsed, &globals))
            };
            match outcome {
                Ok(reply) => print(&render(&reply, globals.json)),
                Err(failure) => fail(globals.json, &failure),
            }
        }
    }
}

/// The library the command works on: `--library`, else the app's own.
fn library(globals: &Globals) -> Result<Library, Failure> {
    Library::locate(globals.library.as_deref()).map_err(Failure::from)
}

/// `writer mcp`: the MCP server on standard input and output.
fn serve(globals: &Globals) -> ExitCode {
    let library = match library(globals) {
        Ok(library) => library,
        Err(failure) => return fail(false, &failure),
    };
    match writer_mcp::serve_stdio(library) {
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

/// Writes the result to standard output. A reader that went away (`writer list | head`)
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

/// Writes a failure to standard error and gives its exit code.
fn fail(as_json: bool, failure: &Failure) -> ExitCode {
    let mut stderr = std::io::stderr().lock();
    let _ = if as_json {
        writeln!(stderr, "{}", json!({ "error": failure.data }))
    } else {
        writeln!(stderr, "writer: {}", failure.message)
    };
    ExitCode::from(failure.kind.exit_code())
}
