//! `innerword page add`: one operation of the command layer (`add_page`) for every page, in
//! the order they are given, each with its own line of output.
//!
//! Pages come from the command line (`innerword page add a.html b/`) or from a list
//! (`--list pages.json`, or `-` for standard input): `[{"path": "...", "entry": "...",
//! "title": "...", "source_url": "...", "externals": {"<address>": "<file>"}}]`, only
//! `path` required. A list is what an agent writes to copy many pages in one command,
//! each with the address it came from. Every page is tried; one that fails does not stop
//! the others.

use std::io::Read;

use serde_json::{Map, Value, json};
use innerword_mcp::{ErrorKind, Failure, Reply, Toolbox};

use crate::parse::{Globals, Parsed};
use crate::spec::Spec;

/// What `innerword page add` came to when it got as far as trying the pages.
pub struct Added {
    /// A line (or a few, with warnings) for each page; with `--json` `{"pages": [...]}`.
    pub reply: Reply,
    /// The failure of the first page that was not added, whose exit code is the command's.
    pub first_failure: Option<Failure>,
    /// How many pages were tried.
    pub tried: usize,
    /// How many of them were not added.
    pub failed: usize,
}

/// One page to add.
#[derive(Debug, Default, PartialEq)]
struct Entry {
    path: String,
    entry: Option<String>,
    title: Option<String>,
    source_url: Option<String>,
    externals: Map<String, Value>,
}

/// The fields an entry of a list can have.
const LIST_FIELDS: [&str; 5] = ["path", "entry", "title", "source_url", "externals"];

/// What the dry run of a page says before its line.
const DRY_RUN: &str = "Dry run: nothing was changed. The real run would report:\n";

fn usage(spec: &Spec) -> String {
    format!(
        "usage: {}\nSee \"innerword page add --help\".",
        crate::spec::usage(spec)
    )
}

/// Adds the pages the command line names. `Err` is a command that did not get as far as
/// trying the pages (a wrong command line, a list that cannot be read), or the failure of
/// the one page named on the command line.
pub fn add(
    toolbox: &Toolbox,
    spec: &Spec,
    parsed: &Parsed,
    globals: &Globals,
) -> Result<Added, Failure> {
    let named = parsed.args_from(0);
    let one_page_options: Vec<&str> = ["entry", "title", "source-url"]
        .into_iter()
        .filter(|option| parsed.value(option).is_some())
        .collect();
    let options_named = || {
        one_page_options
            .iter()
            .map(|option| format!("--{option}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let (entries, from_list) = match (parsed.value("list"), named.as_slice()) {
        (Some(_), [_, ..]) => {
            return Err(Failure::invalid(format!(
                "name the pages as files and folders, or in a list with --list, not both\n{}",
                usage(spec)
            )));
        }
        (None, []) => {
            return Err(Failure::invalid(format!(
                "name the page: an HTML file or the folder of a page, or a list of pages with \
                 --list <file.json>\n{}",
                usage(spec)
            )));
        }
        (Some(list), []) => {
            if !one_page_options.is_empty() {
                return Err(Failure::invalid(format!(
                    "{} {} for one page named on the command line: in a list, each page has \
                     its own (\"entry\", \"title\", \"source_url\")\n{}",
                    options_named(),
                    if one_page_options.len() == 1 {
                        "is"
                    } else {
                        "are"
                    },
                    usage(spec)
                )));
            }
            (read_list(list)?, true)
        }
        (None, [one]) => (
            vec![Entry {
                path: one.to_string(),
                entry: parsed.value("entry").map(str::to_string),
                title: parsed.value("title").map(str::to_string),
                source_url: parsed.value("source-url").map(str::to_string),
                externals: Map::new(),
            }],
            false,
        ),
        (None, several) => {
            if !one_page_options.is_empty() {
                return Err(Failure::invalid(format!(
                    "{} {} for one page, and {} are named: give each page its own in a list \
                     (--list <file.json>, where every page has its \"entry\", \"title\" and \
                     \"source_url\")\n{}",
                    options_named(),
                    if one_page_options.len() == 1 {
                        "is"
                    } else {
                        "are"
                    },
                    several.len(),
                    usage(spec)
                )));
            }
            (
                several
                    .iter()
                    .map(|path| Entry {
                        path: path.to_string(),
                        ..Entry::default()
                    })
                    .collect(),
                false,
            )
        }
    };

    let mut common = Map::new();
    if let Some(folder) = parsed.value("folder") {
        common.insert("folder".into(), json!(folder));
    }
    if parsed.flag("create-folders") {
        common.insert("create_folders".into(), json!(true));
    }
    if parsed.flag("include-trashed") {
        common.insert("include_trashed".into(), json!(true));
    }
    if globals.dry_run {
        common.insert("dry_run".into(), json!(true));
    }
    if let Some(author) = &globals.author {
        common.insert("author".into(), json!(author));
    }

    // Where a relative path of the list was taken from, for the line of a page that is not
    // there (a list on standard input has none: its paths are taken from here).
    let list_folder = parsed
        .value("list")
        .filter(|list| *list != "-")
        .and_then(|_| crate::run::base_dir(parsed));
    let mut results: Vec<(Entry, Result<Reply, Failure>)> = Vec::new();
    for entry in entries {
        let mut arguments = common.clone();
        arguments.insert("path".into(), json!(entry.path));
        for (name, value) in [
            ("entry", &entry.entry),
            ("title", &entry.title),
            ("source_url", &entry.source_url),
        ] {
            if let Some(value) = value {
                arguments.insert(name.into(), json!(value));
            }
        }
        if !entry.externals.is_empty() {
            arguments.insert("externals".into(), Value::Object(entry.externals.clone()));
        }
        let result = toolbox
            .run("add_page", &arguments, None)
            .unwrap_or_else(|| {
                Err(Failure::new(
                    ErrorKind::Failed,
                    "there is no operation add_page",
                ))
            })
            .map_err(|failure| {
                crate::run::missing_file_in(failure, &entry.path, list_folder.as_deref())
            });
        results.push((entry, result));
    }

    // One page named on the command line: a command like every other.
    if !from_list && results.len() == 1 {
        let (_, result) = results.pop().expect("one page");
        let reply = result?;
        let data = json!({ "pages": [reply.data] });
        return Ok(Added {
            reply: Reply {
                text: reply.text,
                data,
            },
            first_failure: None,
            tried: 1,
            failed: 0,
        });
    }

    let tried = results.len();
    let mut text = String::new();
    if globals.dry_run {
        text.push_str(DRY_RUN);
    }
    let mut data = Vec::new();
    let mut first_failure = None;
    let mut failed = 0;
    for (entry, result) in results {
        match result {
            Ok(reply) => {
                text.push_str(reply.text.strip_prefix(DRY_RUN).unwrap_or(&reply.text));
                data.push(with_input(reply.data, &entry.path));
            }
            Err(failure) => {
                failed += 1;
                text.push_str(&not_added(&entry.path, &failure));
                text.push('\n');
                data.push(with_input(failure.data.clone(), &entry.path));
                first_failure.get_or_insert(failure);
            }
        }
    }
    Ok(Added {
        reply: Reply {
            text,
            data: json!({ "pages": data }),
        },
        first_failure,
        tried,
        failed,
    })
}

/// The result of one page as data, with the input it was made from (`"input"`), so that a
/// caller can tell which line of its list each one is.
fn with_input(mut data: Value, path: &str) -> Value {
    if let Some(fields) = data.as_object_mut() {
        fields.insert("input".into(), json!(path));
    }
    data
}

/// The line of a page that was not added: `not added “Reading Orbitarium”: it could not be
/// kept whole. … Nothing was saved.`, or for a failure that has no name of a page yet,
/// `not added “~/Downloads/x”: there is no file …`.
fn not_added(path: &str, failure: &Failure) -> String {
    let message = failure
        .message
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if failure.kind == ErrorKind::PageIncomplete
        && let Some(rest) = message.strip_prefix('\u{201c}')
        && let Some((name, why)) = rest.split_once("\u{201d} was not added: ")
    {
        return format!("not added \u{201c}{name}\u{201d}: {why}");
    }
    format!("not added \u{201c}{path}\u{201d}: {message}")
}

/// Reads the list of `--list`: a file, or `-` for standard input.
fn read_list(list: &str) -> Result<Vec<Entry>, Failure> {
    let (text, name) = if list == "-" {
        let mut text = String::new();
        std::io::stdin()
            .lock()
            .read_to_string(&mut text)
            .map_err(|error| {
                Failure::new(
                    ErrorKind::Failed,
                    format!("standard input cannot be read: {error}"),
                )
            })?;
        (text, "the list on standard input".to_string())
    } else {
        let text = std::fs::read_to_string(list).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                Failure::invalid(format!("there is no list {list}"))
            } else {
                Failure::new(ErrorKind::Failed, format!("{list} cannot be read: {error}"))
            }
        })?;
        (text, format!("the list {list}"))
    };
    parse_list(&text, &name)
}

/// The pages of a list. `name` says which list it is in a message.
fn parse_list(text: &str, name: &str) -> Result<Vec<Entry>, Failure> {
    const SHAPE: &str = "[{\"path\": \"<file or folder>\", \"source_url\": \"...\", \"title\": \
                         \"...\", \"entry\": \"...\", \"externals\": {\"<address>\": \"<file>\"}}]";
    let value: Value = serde_json::from_str(text).map_err(|error| {
        Failure::invalid(format!(
            "{name} is not JSON ({error}); it is a list of pages: {SHAPE}"
        ))
    })?;
    let Value::Array(items) = value else {
        return Err(Failure::invalid(format!(
            "{name} is not a list; it is a list of pages: {SHAPE}"
        )));
    };
    if items.is_empty() {
        return Err(Failure::invalid(format!("{name} names no page")));
    }
    let mut entries = Vec::new();
    for (index, item) in items.into_iter().enumerate() {
        let which = format!("page {} of {name}", index + 1);
        let Value::Object(fields) = item else {
            return Err(Failure::invalid(format!(
                "{which} is not an object; each page is {{\"path\": \"<file or folder>\", ...}}"
            )));
        };
        for field in fields.keys() {
            if field == "prepared" {
                return Err(Failure::invalid(format!(
                    "{which} names \"prepared\", which a list cannot give: a page is always read, \
                     fetched and checked by this tool, from its path"
                )));
            }
            if !LIST_FIELDS.contains(&field.as_str()) {
                return Err(Failure::invalid(format!(
                    "{which} has the field \"{field}\", which a page of a list does not have (it \
                     has {})",
                    LIST_FIELDS
                        .iter()
                        .map(|field| format!("\"{field}\""))
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
        }
        let text_field = |field: &str| -> Result<Option<String>, Failure> {
            match fields.get(field) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::String(text)) => Ok(Some(text.clone())),
                Some(_) => Err(Failure::invalid(format!(
                    "\"{field}\" of {which} is not a text"
                ))),
            }
        };
        let path = text_field("path")?
            .filter(|path| !path.trim().is_empty())
            .ok_or_else(|| {
                Failure::invalid(format!(
                    "{which} has no \"path\": the HTML file or the folder of the page"
                ))
            })?;
        let externals = match fields.get("externals") {
            None | Some(Value::Null) => Map::new(),
            Some(Value::Object(map)) if map.values().all(Value::is_string) => map.clone(),
            Some(_) => {
                return Err(Failure::invalid(format!(
                    "\"externals\" of {which} is not an object of addresses and files \
                     ({{\"<address>\": \"<file>\"}})"
                )));
            }
        };
        entries.push(Entry {
            path,
            entry: text_field("entry")?,
            title: text_field("title")?,
            source_url: text_field("source_url")?,
            externals,
        });
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_list_is_read_with_what_each_page_has() {
        let entries = parse_list(
            r#"[{"path": "a", "source_url": "https://claude.ai/artifact/x", "title": "A",
                 "externals": {"https://a/b.png": "b.png"}},
                {"path": "c/", "entry": "x.html", "title": null}]"#,
            "the list l.json",
        )
        .unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].path, "a");
        assert_eq!(
            entries[0].source_url.as_deref(),
            Some("https://claude.ai/artifact/x")
        );
        assert_eq!(entries[0].title.as_deref(), Some("A"));
        assert_eq!(entries[0].externals["https://a/b.png"], "b.png");
        assert_eq!(entries[1].entry.as_deref(), Some("x.html"));
        assert_eq!(entries[1].title, None);
    }

    #[test]
    fn a_list_that_is_wrong_says_where() {
        let wrong = |text: &str| parse_list(text, "the list l.json").unwrap_err().message;
        assert!(wrong("{").contains("is not JSON"));
        assert!(wrong("{}").contains("is not a list"));
        assert!(wrong("[]").contains("names no page"));
        assert!(wrong("[1]").contains("page 1 of the list l.json is not an object"));
        assert!(wrong(r#"[{"title": "x"}]"#).contains("has no \"path\""));
        assert!(wrong(r#"[{"path": 3}]"#).contains("\"path\" of page 1"));
        assert!(wrong(r#"[{"path": "a", "folder": "x"}]"#).contains("the field \"folder\""));
        assert!(wrong(r#"[{"path": "a", "externals": {"x": 1}}]"#).contains("\"externals\""));
        let prepared = wrong(r#"[{"path": "a"}, {"path": "b", "prepared": {}}]"#);
        assert!(
            prepared.starts_with("page 2 of the list l.json names \"prepared\""),
            "{prepared}"
        );
        assert_eq!(
            parse_list("[]", "x").unwrap_err().kind,
            ErrorKind::Invalid,
            "a usage error"
        );
    }

    #[test]
    fn the_line_of_a_page_that_was_not_added_names_the_page() {
        let incomplete = Failure::new(
            ErrorKind::PageIncomplete,
            "\u{201c}Growth\u{201d} was not added: it could not be kept whole. The page asks for \
             https://claude.ai/api/x at run time. Nothing was saved.",
        );
        assert_eq!(
            not_added("growth/", &incomplete),
            "not added \u{201c}Growth\u{201d}: it could not be kept whole. The page asks for \
             https://claude.ai/api/x at run time. Nothing was saved."
        );
        let missing = Failure::invalid("there is no file or folder /x/y");
        assert_eq!(
            not_added("/x/y", &missing),
            "not added \u{201c}/x/y\u{201d}: there is no file or folder /x/y"
        );
    }
}
