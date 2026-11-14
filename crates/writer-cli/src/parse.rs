//! The command line as the command table ([`crate::spec`]) reads it.
//!
//! The grammar is the common one, so that an agent's first guess is right:
//!
//! * `writer [global options] <command> [arguments] [options]`; the global options
//!   (`--library <dir>`, `--author <name>`, `--json`, `--dry-run`, `-h` / `--help`,
//!   `-V` / `--version`) may stand anywhere.
//! * An option with a value is `--name value` or `--name=value`. A value may begin with a
//!   dash when it is written `--name=-value`.
//! * `--` ends the options: everything after it is an argument, whatever it looks like.
//! * A single `-` is an argument (standard input, where a file is expected).
//!
//! Nothing is guessed: an option the command does not have, an option given twice, a
//! missing or a surplus argument are all usage errors (exit code 2) that name the command's
//! help.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::spec::{self, Content, Spec};

/// What applies to every command.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Globals {
    /// `--library <dir>`: the data directory to work on instead of the default one.
    pub library: Option<PathBuf>,
    /// `--json`: print the result as JSON.
    pub json: bool,
    /// `--dry-run`: say what a write would change without changing it.
    pub dry_run: bool,
    /// `--author <name>`: who makes the change (labels the version kept before it, signs
    /// a comment).
    pub author: Option<String>,
}

/// The arguments and options of one command.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Parsed {
    /// The arguments, one place for each argument of the command's table: `None` for an
    /// optional one that was left out.
    pub positionals: Vec<Option<String>>,
    /// The words after the first that an argument which takes many words was given
    /// (`page add a.html b.html`: `b.html`).
    pub more: Vec<String>,
    /// The options with a value, by name (without the dashes).
    pub values: BTreeMap<&'static str, String>,
    /// The switches that were given.
    pub flags: Vec<&'static str>,
}

impl Parsed {
    /// The argument at the place `index` of the command's table, when given.
    pub fn arg(&self, index: usize) -> Option<&str> {
        self.positionals
            .get(index)
            .and_then(|argument| argument.as_deref())
    }

    /// Every word the arguments from the place `index` on were given: the argument at that
    /// place and, when it takes many words, the rest.
    pub fn args_from(&self, index: usize) -> Vec<&str> {
        let mut words: Vec<&str> = self
            .positionals
            .iter()
            .skip(index)
            .filter_map(|argument| argument.as_deref())
            .collect();
        words.extend(self.more.iter().map(String::as_str));
        words
    }

    /// The value of the option `name`, when given.
    pub fn value(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(String::as_str)
    }

    /// Whether the switch `name` was given.
    pub fn flag(&self, name: &str) -> bool {
        self.flags.contains(&name)
    }
}

/// What the command line asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// `writer`, `writer --help`, `writer help`: the list of commands.
    Overview,
    /// `writer <command> --help`, `writer help <command>`: one command's help.
    Help(&'static Spec),
    /// `writer --version`.
    Version,
    /// A command to run.
    Run(&'static Spec, Parsed),
}

/// A command line that cannot be read. The message is complete; the caller adds nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsageError(pub String);

impl UsageError {
    fn of(spec: &Spec, problem: impl std::fmt::Display) -> Self {
        Self(format!(
            "{problem}\nusage: {}\nSee \"writer {} --help\".",
            spec::usage(spec),
            spec.name
        ))
    }
}

/// One token of the command line, sorted.
enum Token {
    /// `--name` or `--name=value`.
    Long(String, Option<String>),
    /// `-h`, `-V` or another single-dash word.
    Short(String),
    /// Anything else.
    Word(String),
}

fn token(raw: String, options_ended: bool) -> Token {
    if options_ended || raw == "-" {
        return Token::Word(raw);
    }
    // An option is `--name`, `--name=value` or `-x`, the name in lower case letters and
    // dashes. Anything else that begins with a dash ("- a list item", "-5", "--- a rule")
    // is text, not a misspelt option.
    if let Some(long) = raw.strip_prefix("--") {
        let (name, value) = match long.split_once('=') {
            Some((name, value)) => (name, Some(value)),
            None => (long, None),
        };
        let is_name = name.starts_with(|c: char| c.is_ascii_lowercase())
            && name.chars().all(|c| c.is_ascii_lowercase() || c == '-');
        if is_name {
            return Token::Long(name.to_string(), value.map(str::to_string));
        }
        return Token::Word(raw);
    }
    if let Some(short) = raw.strip_prefix('-')
        && !short.is_empty()
        && short.chars().all(|c| c.is_ascii_alphabetic())
    {
        return Token::Short(raw);
    }
    Token::Word(raw)
}

/// Reads a command line (without the program name).
pub fn parse(args: impl IntoIterator<Item = String>) -> Result<(Globals, Action), UsageError> {
    let mut globals = Globals::default();
    let mut help = false;
    let mut version = false;
    // What is left for the command once the global options are taken out: words, and the
    // options that are not global, in order.
    let mut rest: Vec<Token> = Vec::new();
    let mut options_ended = false;
    let mut args = args.into_iter();
    while let Some(raw) = args.next() {
        if !options_ended && raw == "--" {
            options_ended = true;
            continue;
        }
        match token(raw, options_ended) {
            Token::Long(name, value) => match name.as_str() {
                "help" => help = true,
                "version" => version = true,
                "json" => globals.json = true,
                "dry-run" => globals.dry_run = true,
                "library" => {
                    let Some(dir) = value.or_else(|| args.next()) else {
                        return Err(UsageError("--library needs a directory".into()));
                    };
                    if dir.is_empty() {
                        return Err(UsageError("--library needs a directory".into()));
                    }
                    globals.library = Some(PathBuf::from(dir));
                }
                "author" => {
                    let Some(name) = value.or_else(|| args.next()) else {
                        return Err(UsageError("--author needs a name".into()));
                    };
                    globals.author = Some(name);
                }
                _ => {
                    // An option of the command. Whether it takes a value is known only
                    // once the command is: keep the next token with it when it has none.
                    rest.push(Token::Long(name, value));
                }
            },
            Token::Short(short) => match short.as_str() {
                "-h" => help = true,
                "-V" => version = true,
                _ => rest.push(Token::Short(short)),
            },
            Token::Word(word) => rest.push(Token::Word(word)),
        }
    }
    if version {
        return Ok((globals, Action::Version));
    }

    // The command is the first word; a group's subcommand the second.
    let mut rest = rest.into_iter().peekable();
    let command = match rest.next() {
        None => return Ok((globals, Action::Overview)),
        Some(Token::Word(word)) => word,
        Some(Token::Long(name, _)) => {
            return Err(UsageError(format!(
                "unknown option --{name} (before the command, only --library, --author, --json, --dry-run, --help and --version are read)\nSee \"writer --help\"."
            )));
        }
        Some(Token::Short(short)) => {
            return Err(UsageError(format!(
                "unknown option {short}\nSee \"writer --help\"."
            )));
        }
    };
    if command == "help" {
        // `writer help`, `writer help edit`, `writer help comment add`.
        let words: Vec<String> = rest
            .filter_map(|token| match token {
                Token::Word(word) => Some(word),
                _ => None,
            })
            .collect();
        return match words.as_slice() {
            [] => Ok((globals, Action::Overview)),
            [first, more @ ..] => {
                let second = more.first().map(String::as_str);
                match spec::find(first, second) {
                    spec::Found::Command(spec, _) => Ok((globals, Action::Help(spec))),
                    spec::Found::Unknown => Err(unknown_command(first)),
                }
            }
        };
    }
    let second = match rest.peek() {
        Some(Token::Word(word)) => Some(word.clone()),
        _ => None,
    };
    let spec = match spec::find(&command, second.as_deref()) {
        spec::Found::Command(spec, used_second) => {
            if used_second {
                rest.next();
            }
            spec
        }
        spec::Found::Unknown => return Err(unknown_command(&command)),
    };
    if help {
        return Ok((globals, Action::Help(spec)));
    }

    let mut parsed = Parsed::default();
    let mut words: Vec<String> = Vec::new();
    while let Some(token) = rest.next() {
        match token {
            Token::Word(word) => words.push(word),
            Token::Short(short) => {
                return Err(UsageError::of(spec, format!("unknown option {short}")));
            }
            Token::Long(name, value) => {
                let Some(option) = spec::option(spec, &name) else {
                    let known: Vec<String> = spec::options(spec)
                        .iter()
                        .map(|option| format!("--{}", option.name))
                        .collect();
                    let known = if known.is_empty() {
                        "it has no options of its own".to_string()
                    } else {
                        format!("its options: {}", known.join(", "))
                    };
                    return Err(UsageError::of(
                        spec,
                        format!("\"writer {}\" has no option --{name} ({known})", spec.name),
                    ));
                };
                if option.value.is_some() {
                    let value = match value {
                        Some(value) => value,
                        None => match rest.next() {
                            Some(Token::Word(word)) => word,
                            // `--new --flag`: a value that begins with dashes is taken as
                            // written. It is what the writer of the command meant, and
                            // `--name=value` is there for whoever wants to be exact.
                            Some(Token::Long(next, None)) => format!("--{next}"),
                            Some(Token::Long(next, Some(next_value))) => {
                                format!("--{next}={next_value}")
                            }
                            Some(Token::Short(short)) => short,
                            None => {
                                return Err(UsageError::of(
                                    spec,
                                    format!(
                                        "--{name} needs a value: --{name} <{}>",
                                        option.value.unwrap_or("value")
                                    ),
                                ));
                            }
                        },
                    };
                    if parsed.values.insert(option.name, value).is_some() {
                        return Err(UsageError::of(spec, format!("--{name} is given twice")));
                    }
                } else {
                    if value.is_some() {
                        return Err(UsageError::of(spec, format!("--{name} takes no value")));
                    }
                    if !parsed.flags.contains(&option.name) {
                        parsed.flags.push(option.name);
                    }
                }
            }
        }
    }

    let required = spec.positionals.iter().filter(|p| p.required).count();
    if words.len() < required {
        // The first required argument that the words given do not reach.
        let missing = spec
            .positionals
            .iter()
            .filter(|p| p.required)
            .nth(words.len())
            .unwrap_or(&spec.positionals[0]);
        return Err(UsageError::of(
            spec,
            format!("<{}> is missing: {}", missing.name, missing.help),
        ));
    }
    let takes_many = spec.positionals.last().is_some_and(|last| last.many);
    if words.len() > spec.positionals.len() && !takes_many {
        let surplus = &words[spec.positionals.len()];
        let hint = if spec.positionals.is_empty() {
            format!("\"writer {}\" takes no arguments", spec.name)
        } else {
            format!(
                "\"writer {}\" takes {} (a name with spaces needs quotes)",
                spec.name,
                spec.positionals
                    .iter()
                    .map(|p| if p.required {
                        format!("<{}>", p.name)
                    } else {
                        format!("[<{}>]", p.name)
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            )
        };
        return Err(UsageError::of(
            spec,
            format!("unexpected argument \"{surplus}\": {hint}"),
        ));
    }
    // An optional argument takes a word only when there are more words than the required
    // arguments still to come need, so one may also stand before a required one
    // (`history show [<document>] <version>`).
    let mut spare = words.len() - required;
    let mut words = words.into_iter();
    for positional in spec.positionals {
        if positional.required {
            parsed.positionals.push(words.next());
        } else if spare > 0 {
            spare -= 1;
            parsed.positionals.push(words.next());
        } else {
            parsed.positionals.push(None);
        }
    }
    parsed.more = words.collect();
    if spec.content != Content::None {
        let sources = ["content", "file"]
            .iter()
            .filter(|name| parsed.values.contains_key(**name))
            .count()
            + usize::from(parsed.flag("stdin"));
        if sources > 1 {
            return Err(UsageError::of(
                spec,
                "give the text once: --content, --file or --stdin",
            ));
        }
    }
    Ok((globals, Action::Run(spec, parsed)))
}

fn unknown_command(word: &str) -> UsageError {
    let near = spec::nearest(word);
    let hint = match near {
        Some(name) => format!(" Did you mean \"writer {name}\"?"),
        None => String::new(),
    };
    UsageError(format!(
        "unknown command \"{word}\".{hint}\nSee \"writer --help\" for the list of commands."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(line: &[&str]) -> Result<(Globals, Action), UsageError> {
        parse(line.iter().map(|word| word.to_string()))
    }

    fn command(line: &[&str]) -> (Globals, &'static str, Parsed) {
        match run(line) {
            Ok((globals, Action::Run(spec, parsed))) => (globals, spec.name, parsed),
            other => panic!("{line:?} is not a command: {other:?}"),
        }
    }

    fn error(line: &[&str]) -> String {
        match run(line) {
            Err(UsageError(message)) => message,
            other => panic!("{line:?} is not an error: {other:?}"),
        }
    }

    #[test]
    fn no_arguments_help_and_version() {
        assert_eq!(run(&[]).unwrap().1, Action::Overview);
        assert_eq!(run(&["--help"]).unwrap().1, Action::Overview);
        assert_eq!(run(&["-h"]).unwrap().1, Action::Overview);
        assert_eq!(run(&["help"]).unwrap().1, Action::Overview);
        assert_eq!(run(&["--version"]).unwrap().1, Action::Version);
        assert_eq!(run(&["-V"]).unwrap().1, Action::Version);
        assert_eq!(run(&["list", "--version"]).unwrap().1, Action::Version);
        for line in [
            &["edit", "--help"][..],
            &["help", "edit"],
            &["--help", "edit"],
            &["edit", "x", "-h"],
        ] {
            match run(line).unwrap().1 {
                Action::Help(spec) => assert_eq!(spec.name, "edit"),
                other => panic!("{line:?}: {other:?}"),
            }
        }
        match run(&["help", "comment", "add"]).unwrap().1 {
            Action::Help(spec) => assert_eq!(spec.name, "comment add"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn global_options_stand_anywhere() {
        let (globals, name, parsed) = command(&["--library", "/tmp/a", "list", "--json"]);
        assert_eq!(
            globals.library.as_deref(),
            Some(std::path::Path::new("/tmp/a"))
        );
        assert!(globals.json);
        assert!(!globals.dry_run);
        assert_eq!(name, "list");
        assert_eq!(parsed.positionals, vec![None]);

        let (globals, name, parsed) =
            command(&["trash", "Notes/Plan", "--dry-run", "--library=/tmp/b"]);
        assert_eq!(
            globals.library.as_deref(),
            Some(std::path::Path::new("/tmp/b"))
        );
        assert!(globals.dry_run);
        assert_eq!(name, "trash");
        assert_eq!(parsed.arg(0), Some("Notes/Plan"));

        let (globals, name, _) = command(&[
            "comment", "add", "D", "--quote", "a", "--body", "b", "--author", "Claude",
        ]);
        assert_eq!(globals.author.as_deref(), Some("Claude"));
        assert_eq!(name, "comment add");
        assert!(error(&["list", "--author"]).contains("--author needs a name"));
        assert!(error(&["list", "--library"]).contains("--library needs a directory"));
        assert!(error(&["--library=", "list"]).contains("--library needs a directory"));
    }

    #[test]
    fn options_with_values_in_both_spellings() {
        let (_, name, parsed) = command(&["edit", "Draft", "--old", "teh", "--new=the", "--all"]);
        assert_eq!(name, "edit");
        assert_eq!(parsed.arg(0), Some("Draft"));
        assert_eq!(parsed.value("old"), Some("teh"));
        assert_eq!(parsed.value("new"), Some("the"));
        assert!(parsed.flag("all"));
        assert!(!parsed.flag("stdin"));

        // A value that begins with dashes, and an empty one.
        let (_, _, parsed) = command(&["edit", "Draft", "--old=-- x", "--new", ""]);
        assert_eq!(parsed.value("old"), Some("-- x"));
        assert_eq!(parsed.value("new"), Some(""));
        let (_, _, parsed) = command(&["edit", "Draft", "--old", "--dash", "--new", "-x"]);
        assert_eq!(parsed.value("old"), Some("--dash"));
        assert_eq!(parsed.value("new"), Some("-x"));

        // Text that begins with a dash but is no option: a list item, a rule, a number.
        let (_, name, parsed) =
            command(&["create", "- a title", "--content", "--- \n- one\n- two"]);
        assert_eq!(name, "create");
        assert_eq!(parsed.arg(0), Some("- a title"));
        assert_eq!(parsed.value("content"), Some("--- \n- one\n- two"));
        let (_, _, parsed) = command(&["search", "-5"]);
        assert_eq!(parsed.arg(0), Some("-5"));
        let (_, _, parsed) = command(&["append", "D", "--content", "--Not an option=really"]);
        assert_eq!(parsed.value("content"), Some("--Not an option=really"));
    }

    #[test]
    fn a_double_dash_ends_the_options() {
        let (_, name, parsed) = command(&["search", "--", "--json"]);
        assert_eq!(name, "search");
        assert_eq!(parsed.arg(0), Some("--json"));
        let (globals, _, parsed) = command(&["append", "Draft", "--file", "-"]);
        assert!(!globals.json);
        assert_eq!(parsed.value("file"), Some("-"));
    }

    #[test]
    fn groups_take_their_subcommand_or_list() {
        assert_eq!(
            command(&["comment", "add", "Draft", "--quote", "a", "--body", "b"]).1,
            "comment add"
        );
        assert_eq!(command(&["comments", "Draft"]).1, "comment list");
        assert_eq!(command(&["comment", "list", "Draft"]).1, "comment list");
        assert_eq!(command(&["comment", "Draft"]).1, "comment list");
        assert_eq!(command(&["history", "Draft"]).1, "history list");
        assert_eq!(command(&["history", "show", "3"]).1, "history show");
        // An optional argument before a required one: the document may be named or not.
        let (_, _, one) = command(&["history", "show", "3"]);
        assert_eq!((one.arg(0), one.arg(1)), (None, Some("3")));
        let (_, _, two) = command(&["history", "restore", "doc:12", "3"]);
        assert_eq!((two.arg(0), two.arg(1)), (Some("doc:12"), Some("3")));
        assert!(error(&["history", "show"]).contains("<version> is missing"));
        assert!(
            error(&["history", "show", "a", "b", "c"]).contains("takes [<document>] <version>")
        );
        assert_eq!(
            command(&["history", "restore", "3", "--dry-run"]).1,
            "history restore"
        );
        assert_eq!(command(&["agents"]).1, "agents list");
        assert_eq!(command(&["agents", "--json"]).1, "agents list");
        assert_eq!(command(&["agents", "install", "--all"]).1, "agents install");
    }

    #[test]
    fn the_last_argument_can_take_many_words() {
        let (_, name, parsed) = command(&["page", "add", "a.html", "b", "c d", "--dry-run"]);
        assert_eq!(name, "page add");
        assert_eq!(parsed.args_from(0), ["a.html", "b", "c d"]);
        let (_, _, none) = command(&["page", "add", "--list", "-"]);
        assert!(none.args_from(0).is_empty());
        assert_eq!(none.value("list"), Some("-"));
        // Another command still says that a word is one too many.
        assert!(error(&["get", "a", "b"]).contains("unexpected argument \"b\""));
        assert_eq!(command(&["pages"]).1, "page list");
        assert_eq!(command(&["pages", "Claude Pages"]).2.arg(0), Some("Claude Pages"));
        assert_eq!(command(&["page", "list", "--json"]).1, "page list");
    }

    #[test]
    fn aliases() {
        assert_eq!(command(&["ls"]).1, "list");
        assert_eq!(command(&["tree"]).1, "list");
        assert_eq!(command(&["cat", "Draft"]).1, "get");
        assert_eq!(command(&["read", "Draft"]).1, "get");
        assert_eq!(command(&["delete", "Draft"]).1, "trash");
        assert_eq!(command(&["rm", "Draft"]).1, "trash");
        assert_eq!(command(&["mv", "Draft", "Notes"]).1, "move");
        assert_eq!(command(&["new", "Draft"]).1, "create");
    }

    #[test]
    fn usage_errors_say_what_is_wrong_and_where_the_help_is() {
        let unknown = error(&["serch", "x"]);
        assert!(unknown.contains("unknown command \"serch\""), "{unknown}");
        assert!(
            unknown.contains("Did you mean \"writer search\"?"),
            "{unknown}"
        );
        assert!(unknown.contains("writer --help"), "{unknown}");

        let option = error(&["edit", "Draft", "--old", "a", "--neu", "b"]);
        assert!(option.contains("has no option --neu"), "{option}");
        assert!(option.contains("--old"), "{option}");
        assert!(option.contains("usage: writer edit"), "{option}");
        assert!(option.contains("writer edit --help"), "{option}");

        let missing = error(&["get"]);
        assert!(missing.contains("<document> is missing"), "{missing}");
        let surplus = error(&["get", "My", "Draft"]);
        assert!(
            surplus.contains("unexpected argument \"Draft\""),
            "{surplus}"
        );
        assert!(surplus.contains("needs quotes"), "{surplus}");
        let twice = error(&["edit", "D", "--old", "a", "--old", "b", "--new", "c"]);
        assert!(twice.contains("--old is given twice"), "{twice}");
        let no_value = error(&["edit", "D", "--old"]);
        assert!(no_value.contains("--old needs a value"), "{no_value}");
        let flag_value = error(&["edit", "D", "--all=yes"]);
        assert!(flag_value.contains("--all takes no value"), "{flag_value}");
        let sources = error(&["append", "D", "--content", "a", "--stdin"]);
        assert!(sources.contains("give the text once"), "{sources}");
        let short = error(&["get", "D", "-x"]);
        assert!(short.contains("unknown option -x"), "{short}");
        let before = error(&["--frobnicate", "list"]);
        assert!(before.contains("unknown option --frobnicate"), "{before}");
    }
}
