//! From a parsed command line to an operation of the command layer.
//!
//! Every command about the library is one operation of [`writer_mcp::Toolbox`], the same
//! table the MCP server offers as tools: this module only turns arguments and options
//! into the JSON object the operation takes. What the command prints is the operation's
//! text, or with `--json` its data.

use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};
use writer_mcp::{ErrorKind, Failure, Item, Reply, Toolbox};

use crate::parse::{Globals, Parsed};
use crate::spec::{Content, Spec};

/// The arguments of an operation, as they are put together.
struct Arguments(Map<String, Value>);

impl Arguments {
    fn new() -> Self {
        Arguments(Map::new())
    }

    fn set(&mut self, name: &str, value: impl Into<Value>) -> &mut Self {
        self.0.insert(name.to_string(), value.into());
        self
    }

    /// Sets `name` when the option or argument was given.
    fn set_some(&mut self, name: &str, value: Option<&str>) -> &mut Self {
        if let Some(value) = value {
            self.set(name, value);
        }
        self
    }

    /// Sets `name` to `true` when the switch was given.
    fn set_flag(&mut self, name: &str, given: bool) -> &mut Self {
        if given {
            self.set(name, true);
        }
        self
    }

    /// Sets `name` to the number an option names.
    fn set_number(
        &mut self,
        name: &str,
        option: &str,
        value: Option<&str>,
    ) -> Result<&mut Self, Failure> {
        if let Some(value) = value {
            let number: u64 = value.trim().parse().map_err(|_| {
                Failure::invalid(format!(
                    "{option} must be a whole number, and \"{value}\" is not"
                ))
            })?;
            self.set(name, number);
        }
        Ok(self)
    }
}

/// Reads all of standard input as text.
fn read_stdin() -> Result<String, Failure> {
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
    Ok(text)
}

/// Reads a file as text; `-` is standard input.
fn read_file(path: &str) -> Result<String, Failure> {
    if path == "-" {
        return read_stdin();
    }
    std::fs::read_to_string(path)
        .map_err(|error| Failure::new(ErrorKind::Failed, format!("{path} cannot be read: {error}")))
}

/// The text a command was given with `--content`, `--file` or `--stdin`. A command that
/// needs text and was given none of them reads standard input when that is a pipe or a
/// file; `None` when there is no text.
fn content(spec: &Spec, parsed: &Parsed) -> Result<Option<String>, Failure> {
    if let Some(text) = parsed.value("content") {
        return Ok(Some(text.to_string()));
    }
    if let Some(path) = parsed.value("file") {
        return read_file(path).map(Some);
    }
    if parsed.flag("stdin") {
        return read_stdin().map(Some);
    }
    match spec.content {
        Content::None | Content::Optional => Ok(None),
        Content::Required => {
            let piped = if std::io::stdin().is_terminal() {
                String::new()
            } else {
                read_stdin()?
            };
            if piped.is_empty() {
                return Err(Failure::invalid(format!(
                    "no text was given: use --content <markdown>, --file <path> or --stdin (or pipe the text in)\nSee \"writer {} --help\".",
                    spec.name
                )));
            }
            Ok(Some(piped))
        }
    }
}

/// The directory a relative path of an image file is taken from: the directory of the
/// file the text came from (`--file`, or `--new-file` of an edit; for `page add`, the list
/// of pages), else the directory the tool was started in. `None` when that directory
/// cannot be told.
pub fn base_dir(parsed: &Parsed) -> Option<PathBuf> {
    let started_in = std::env::current_dir().ok();
    let named = ["file", "new-file", "list"]
        .into_iter()
        .find_map(|option| parsed.value(option))
        .filter(|path| *path != "-");
    match named {
        Some(file) => {
            let folder = Path::new(file).parent().unwrap_or(Path::new(""));
            if folder.is_absolute() {
                Some(folder.to_path_buf())
            } else {
                Some(started_in?.join(folder))
            }
        }
        None => started_in,
    }
}

/// One of the words an option takes, or the failure that names them.
fn one_of<'a>(
    spec: &Spec,
    option: &str,
    value: &'a str,
    words: &[&'a str],
) -> Result<&'a str, Failure> {
    let given = value.trim().to_ascii_lowercase();
    words
        .iter()
        .copied()
        .find(|word| *word == given)
        .ok_or_else(|| {
            Failure::invalid(format!(
                "--{option} is one of {}, and \"{value}\" is none of them\nusage: {}\nSee \"writer {} --help\".",
                words.join(", "),
                crate::spec::usage(spec),
                spec.name
            ))
        })
}

/// An option that may come from the command line or from a file (`--old` / `--old-file`).
fn text_option(parsed: &Parsed, name: &str, file: &str) -> Result<Option<String>, Failure> {
    match (parsed.value(name), parsed.value(file)) {
        (Some(_), Some(_)) => Err(Failure::invalid(format!(
            "give --{name} or --{file}, not both"
        ))),
        (Some(text), None) => Ok(Some(text.to_string())),
        (None, Some(path)) => read_file(path).map(Some),
        (None, None) => Ok(None),
    }
}

fn required_option<'a>(parsed: &'a Parsed, spec: &Spec, name: &str) -> Result<&'a str, Failure> {
    parsed.value(name).ok_or_else(|| {
        Failure::invalid(format!(
            "--{name} is missing\nusage: {}\nSee \"writer {} --help\".",
            crate::spec::usage(spec),
            spec.name
        ))
    })
}

/// The place among its siblings, for `create` and `move`.
fn placement(arguments: &mut Arguments, parsed: &Parsed) {
    arguments
        .set_flag("first", parsed.flag("first"))
        .set_some("before", parsed.value("before"))
        .set_some("after", parsed.value("after"));
}

/// What the command line adds when the file of `writer image add` is not there: where a
/// path that does not start at the top was looked for.
pub fn missing_file(failure: Failure, file: &str) -> Failure {
    let relative = !file.starts_with('/') && !file.starts_with('~');
    if failure.kind != ErrorKind::Invalid
        || !failure.message.starts_with("there is no file ")
        || !relative
    {
        return failure;
    }
    let started_in = std::env::current_dir()
        .map(|dir| dir.display().to_string())
        .unwrap_or_else(|_| "the directory the command runs in".to_string());
    Failure::invalid(format!(
        "{}\nA path that does not begin with / or ~/ is taken from where the command runs: {started_in}",
        failure.message
    ))
}

/// Runs a command about the library.
pub fn run(
    toolbox: &Toolbox,
    spec: &Spec,
    parsed: &Parsed,
    globals: &Globals,
) -> Result<Reply, Failure> {
    let mut arguments = Arguments::new();
    if spec.dry_run && globals.dry_run {
        arguments.set("dry_run", true);
    }
    if spec.writes {
        arguments.set_some("author", globals.author.as_deref());
    }
    let call = |name: &str, arguments: Arguments| -> Result<Reply, Failure> {
        toolbox.run(name, &arguments.0, None).unwrap_or_else(|| {
            Err(Failure::new(
                ErrorKind::Failed,
                format!("there is no operation {name}"),
            ))
        })
    };
    let arg = |index: usize| parsed.arg(index).unwrap_or_default();

    match spec.name {
        // ----- look around -----------------------------------------------------------
        "status" => call("library_status", arguments),
        "list" => {
            if parsed.flag("trash") {
                if parsed.arg(0).is_some()
                    || parsed.flag("folders")
                    || parsed.value("depth").is_some()
                {
                    return Err(Failure::invalid(
                        "--trash lists the whole trash: it takes no folder, --depth or --folders",
                    ));
                }
                return call("list_trash", arguments);
            }
            arguments
                .set_some("folder", parsed.arg(0))
                .set_flag("folders_only", parsed.flag("folders"))
                .set_number("depth", "--depth", parsed.value("depth"))?;
            call("list_library", arguments)
        }
        "search" => {
            arguments
                .set("query", arg(0))
                .set_some("folder", parsed.value("in"))
                .set_number("limit", "--limit", parsed.value("limit"))?;
            call("search_library", arguments)
        }
        "recent" => {
            arguments.set_number("limit", "--limit", parsed.value("limit"))?;
            call("recent_documents", arguments)
        }
        "info" => {
            if arg(0).trim() == "/" {
                arguments.set("folder", "/");
                return call("folder_info", arguments);
            }
            toolbox.run_on_item(
                arg(0),
                false,
                "document_info",
                "folder_info",
                &arguments.0,
                |_, _| Ok(()),
            )
        }
        // ----- documents -------------------------------------------------------------
        "get" => {
            arguments
                .set("document", arg(0))
                .set_some("format", parsed.value("format"))
                .set_some("heading", parsed.value("section"))
                .set_flag("comments", parsed.flag("comments"))
                .set_number(
                    "heading_occurrence",
                    "--occurrence",
                    parsed.value("occurrence"),
                )?;
            call("read_document", arguments)
        }
        "find" => {
            arguments
                .set("document", arg(0))
                .set("text", arg(1))
                .set_number("limit", "--limit", parsed.value("limit"))?;
            call("find_in_document", arguments)
        }
        "create" => {
            arguments
                .set_some("title", parsed.arg(0))
                .set_some("folder", parsed.value("in"))
                .set_flag("create_folders", parsed.flag("create-folders"));
            placement(&mut arguments, parsed);
            if let Some(text) = content(spec, parsed)? {
                arguments.set("markdown", text);
            }
            call("create_document", arguments)
        }
        "set" => {
            arguments.set("document", arg(0));
            arguments.set("markdown", content(spec, parsed)?.unwrap_or_default());
            call("replace_document", arguments)
        }
        "edit" => {
            let old = text_option(parsed, "old", "old-file")?;
            let new = text_option(parsed, "new", "new-file")?;
            let (Some(old), Some(new)) = (old, new) else {
                return Err(Failure::invalid(format!(
                    "--old and --new are both needed (an empty --new \"\" deletes)\nusage: {}\nSee \"writer edit --help\".",
                    crate::spec::usage(spec)
                )));
            };
            arguments
                .set("document", arg(0))
                .set("old_text", old)
                .set("new_text", new)
                .set_flag("all", parsed.flag("all"))
                .set_flag("literal", parsed.flag("literal"))
                .set_number("occurrence", "--occurrence", parsed.value("occurrence"))?;
            call("edit_document", arguments)
        }
        "append" => {
            arguments.set("document", arg(0));
            arguments.set("markdown", content(spec, parsed)?.unwrap_or_default());
            call("append_to_document", arguments)
        }
        "insert" => {
            let places = [
                ("before", "before_heading"),
                ("after", "after_heading"),
                ("end-of", "end_of_section"),
            ];
            let mut chosen: Vec<(&str, Option<&str>)> = Vec::new();
            if parsed.flag("at-start") {
                chosen.push(("start", None));
            }
            for (option, position) in places {
                if let Some(heading) = parsed.value(option) {
                    chosen.push((position, Some(heading)));
                }
            }
            let [(position, heading)] = chosen.as_slice() else {
                return Err(Failure::invalid(format!(
                    "say where, once: --at-start, --before <heading>, --after <heading> or --end-of <heading>\nusage: {}\nSee \"writer insert --help\".",
                    crate::spec::usage(spec)
                )));
            };
            arguments
                .set("document", arg(0))
                .set("position", *position)
                .set_some("heading", *heading)
                .set_number(
                    "heading_occurrence",
                    "--occurrence",
                    parsed.value("occurrence"),
                )?;
            arguments.set("markdown", content(spec, parsed)?.unwrap_or_default());
            call("insert_in_document", arguments)
        }
        "replace-section" => {
            arguments
                .set("document", arg(0))
                .set("heading", arg(1))
                .set_flag("include_heading", parsed.flag("with-heading"))
                .set_number(
                    "heading_occurrence",
                    "--occurrence",
                    parsed.value("occurrence"),
                )?;
            arguments.set("markdown", content(spec, parsed)?.unwrap_or_default());
            call("replace_section", arguments)
        }
        "image list" => {
            arguments.set("document", arg(0));
            call("list_images", arguments)
        }
        "page list" => {
            if let Some(folder) = parsed.arg(0).filter(|folder| folder.trim() != "/") {
                arguments.set("folder", folder);
            }
            call("list_pages", arguments)
        }
        "image add" => {
            arguments
                .set("document", arg(0))
                .set("path", arg(1))
                .set_some("description", parsed.value("alt"))
                .set_some("heading", parsed.value("heading"))
                .set_number(
                    "heading_occurrence",
                    "--heading-occurrence",
                    parsed.value("heading-occurrence"),
                )?;
            if let Some(align) = parsed.value("align") {
                arguments.set(
                    "align",
                    one_of(spec, "align", align, &["left", "center", "right"])?,
                );
            }
            if let Some(place) = parsed.value("at") {
                let places = [
                    "end",
                    "start",
                    "before-heading",
                    "after-heading",
                    "end-of-section",
                ];
                // The library's own spelling (before_heading, which its messages use)
                // is the same place.
                let dashed = place.replace('_', "-");
                let chosen = match one_of(spec, "at", &dashed, &places) {
                    Ok(word) => word.to_string(),
                    Err(_) => one_of(spec, "at", place, &places)?.to_string(),
                };
                arguments.set("position", chosen.replace('-', "_"));
            }
            if let Some(width) = parsed.value("width") {
                // "50" and "50%" are both a way to write it.
                let percent = width.trim().trim_end_matches('%').trim();
                match percent.parse::<u64>() {
                    Ok(percent @ 5..=100) => {
                        arguments.set("width_percent", percent);
                    }
                    _ => {
                        return Err(Failure::invalid(format!(
                            "--width is a share of the text column in percent, from 5 to 100, and \"{width}\" is not\nusage: {}\nSee \"writer image add --help\".",
                            crate::spec::usage(spec)
                        )));
                    }
                }
            }
            call("add_image", arguments).map_err(|failure| missing_file(failure, arg(1)))
        }
        "duplicate" => {
            arguments
                .set("document", arg(0))
                .set_some("title", parsed.value("title"))
                .set_some("folder", parsed.value("in"));
            call("duplicate_document", arguments)
        }
        "export" => {
            let path = match (parsed.arg(1), parsed.value("to")) {
                (Some(_), Some(_)) => {
                    return Err(Failure::invalid(format!(
                        "give the path once: as <path> or with --to, not both\nusage: {}\nSee \"writer export --help\".",
                        crate::spec::usage(spec)
                    )));
                }
                (Some(path), None) | (None, Some(path)) => path,
                (None, None) => {
                    return Err(Failure::invalid(format!(
                        "say where to write it: writer export <item> <path> (or --to <path>)\nusage: {}\nSee \"writer export --help\".",
                        crate::spec::usage(spec)
                    )));
                }
            };
            arguments
                .set("path", path)
                .set_some("format", parsed.value("format"))
                .set_flag("overwrite", parsed.flag("overwrite"))
                .set_flag("original", parsed.flag("original"));
            if arg(0).trim() == "/" {
                arguments.set("folder", "/");
                return call("export", arguments);
            }
            toolbox.run_on_item(arg(0), false, "export", "export", &arguments.0, |_, _| {
                Ok(())
            })
        }
        // ----- documents and folders -------------------------------------------------
        "mkdir" => {
            arguments
                .set("path", arg(0))
                .set_some("parent", parsed.value("in"));
            call("create_folder", arguments)
        }
        "rename" => {
            let follow_text = parsed.flag("follow-text");
            let name = parsed.arg(1).map(str::to_string);
            if follow_text == name.is_some() {
                return Err(Failure::invalid(format!(
                    "give the new name, or --follow-text for a document, not {}\nusage: {}\nSee \"writer rename --help\".",
                    if follow_text { "both" } else { "neither" },
                    crate::spec::usage(spec)
                )));
            }
            toolbox.run_on_item(
                arg(0),
                false,
                "rename_document",
                "rename_folder",
                &arguments.0,
                |item, arguments| {
                    match (item, name) {
                        (Item::Document { .. }, Some(name)) => {
                            arguments.insert("title".into(), json!(name));
                        }
                        (Item::Document { .. }, None) => {
                            arguments.insert("follow_text".into(), json!(true));
                        }
                        (Item::Folder { .. }, Some(name)) => {
                            arguments.insert("name".into(), json!(name));
                        }
                        (Item::Folder { .. }, None) => {
                            return Err(Failure::invalid(
                                "--follow-text is for a document: a folder has no text to follow",
                            ));
                        }
                    }
                    Ok(())
                },
            )
        }
        "move" => {
            let destination = parsed.arg(1).map(str::to_string);
            let create_folders = parsed.flag("create-folders");
            placement(&mut arguments, parsed);
            if destination.is_none()
                && arguments
                    .0
                    .keys()
                    .all(|key| !matches!(key.as_str(), "first" | "before" | "after"))
            {
                return Err(Failure::invalid(format!(
                    "say where to: a folder, or a place with --first, --before or --after\nusage: {}\nSee \"writer move --help\".",
                    crate::spec::usage(spec)
                )));
            }
            toolbox.run_on_item(arg(0), false, "move_document", "move_folder", &arguments.0, |item, arguments| {
                match item {
                    Item::Document { .. } => {
                        if let Some(destination) = destination {
                            arguments.insert("folder".into(), json!(destination));
                        }
                        if create_folders {
                            arguments.insert("create_folders".into(), json!(true));
                        }
                    }
                    Item::Folder { .. } => {
                        if create_folders {
                            return Err(Failure::invalid(
                                "--create-folders is for moving a document; create the folder with \"writer mkdir\" first",
                            ));
                        }
                        if let Some(destination) = destination {
                            arguments.insert("parent".into(), json!(destination));
                        }
                    }
                }
                Ok(())
            })
        }
        "trash" => {
            if !parsed.flag("permanently") {
                return toolbox.run_on_item(
                    arg(0),
                    false,
                    "trash_document",
                    "trash_folder",
                    &arguments.0,
                    |_, _| Ok(()),
                );
            }
            arguments.set("permanently", true);
            // What is erased for good is usually in the trash already.
            let in_trash = match toolbox.resolve(arg(0), true) {
                Ok(_) => true,
                Err(failure)
                    if matches!(failure.kind, ErrorKind::NotFound | ErrorKind::WrongState) =>
                {
                    false
                }
                Err(failure) => return Err(failure),
            };
            toolbox.run_on_item(
                arg(0),
                in_trash,
                "delete_document",
                "delete_folder",
                &arguments.0,
                |_, _| Ok(()),
            )
        }
        "restore" => toolbox.run_on_item(
            arg(0),
            true,
            "restore_document",
            "restore_folder",
            &arguments.0,
            |_, _| Ok(()),
        ),
        "empty-trash" => {
            arguments.set_flag("permanently", parsed.flag("permanently"));
            call("empty_trash", arguments)
        }
        // ----- comments --------------------------------------------------------------
        "comment list" => {
            arguments
                .set("document", arg(0))
                .set_flag("open_only", parsed.flag("open"));
            call("list_comments", arguments)
        }
        "comment add" => {
            arguments
                .set("document", arg(0))
                .set("quote", required_option(parsed, spec, "quote")?)
                .set("body", required_option(parsed, spec, "body")?)
                .set_number("occurrence", "--occurrence", parsed.value("occurrence"))?;
            call("add_comment", arguments)
        }
        "comment reply" => {
            arguments
                .set("document", arg(0))
                .set("body", required_option(parsed, spec, "body")?)
                .set_number("comment", "<thread>", parsed.arg(1))?;
            call("reply_to_comment", arguments)
        }
        "comment resolve" | "comment reopen" => {
            arguments
                .set("document", arg(0))
                .set_number("comment", "<thread>", parsed.arg(1))?;
            call(
                if spec.name == "comment resolve" {
                    "resolve_comment"
                } else {
                    "reopen_comment"
                },
                arguments,
            )
        }
        "comment delete" => {
            arguments
                .set("document", arg(0))
                .set_number("comment", "<thread>", parsed.arg(1))?
                .set_number("reply", "--reply", parsed.value("reply"))?;
            call("delete_comment", arguments)
        }
        // ----- history ---------------------------------------------------------------
        "history list" => {
            arguments.set("document", arg(0));
            call("list_versions", arguments)
        }
        "history show" => {
            arguments
                .set_some("format", parsed.value("format"))
                .set_number("version", "<version>", parsed.arg(1))?;
            let reply = call("read_version", arguments)?;
            // The document named with it is a check: the version must be one of its own.
            if let Some(document) = parsed.arg(0) {
                let mut named = Arguments::new();
                named.set("document", document);
                let info = call("document_info", named)?;
                if info.data["document"]["id"] != reply.data["document"]["id"] {
                    return Err(Failure::invalid(format!(
                        "version {} is a version of doc:{} \"{}\", not of doc:{} \"{}\"",
                        arg(1),
                        reply.data["document"]["id"],
                        reply.data["document"]["path"].as_str().unwrap_or_default(),
                        info.data["document"]["id"],
                        info.data["document"]["path"].as_str().unwrap_or_default(),
                    )));
                }
            }
            Ok(reply)
        }
        "history restore" => {
            arguments.set_some("document", parsed.arg(0)).set_number(
                "version",
                "<version>",
                parsed.arg(1),
            )?;
            call("restore_version", arguments)
        }
        other => Err(Failure::new(
            ErrorKind::Failed,
            format!("\"writer {other}\" is not a command about the library"),
        )),
    }
}
