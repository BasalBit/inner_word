//! What every command has in common: help, version, where output goes, exit codes for a
//! command line that cannot be read.

use serde_json::Value;

use super::harness::Sandbox;

/// The names of all commands, read from the overview the way an agent reads them.
fn commands_in_the_overview(overview: &str) -> Vec<String> {
    let mut names = Vec::new();
    for line in overview.lines() {
        let Some(rest) = line.strip_prefix("  ") else {
            continue;
        };
        if rest.starts_with('-') || rest.starts_with(' ') || rest.is_empty() {
            continue;
        }
        let Some((left, _summary)) = rest.split_once("  ") else {
            continue;
        };
        let words: Vec<&str> = left
            .split_whitespace()
            .take_while(|word| !word.starts_with(['<', '[', '-']))
            .collect();
        if !words.is_empty() && words[0].chars().all(|c| c.is_ascii_lowercase() || c == '-') {
            names.push(words.join(" "));
        }
    }
    names
}

#[test]
fn the_overview_lists_every_command_and_each_has_help_with_an_example() {
    let sandbox = Sandbox::new();
    let overview = sandbox.ok(&["--help"]);
    assert_eq!(sandbox.ok(&[]), overview, "no arguments: the overview");
    assert_eq!(sandbox.ok(&["help"]), overview);
    assert_eq!(sandbox.ok(&["-h"]), overview);
    assert!(overview.starts_with(&format!("writer {}", env!("CARGO_PKG_VERSION"))));
    assert!(overview.is_ascii(), "no decoration in the help");

    let names = commands_in_the_overview(&overview);
    for expected in [
        "list",
        "search",
        "recent",
        "info",
        "status",
        "get",
        "find",
        "create",
        "set",
        "edit",
        "append",
        "insert",
        "replace-section",
        "duplicate",
        "export",
        "mkdir",
        "rename",
        "move",
        "trash",
        "restore",
        "empty-trash",
        "comments",
        "comment add",
        "comment reply",
        "comment resolve",
        "comment reopen",
        "comment delete",
        "history",
        "history show",
        "history restore",
        "agents",
        "agents install",
        "agents update",
        "agents remove",
        "agents skill",
        "agents mcp",
        "mcp",
    ] {
        assert!(
            names.iter().any(|name| name == expected),
            "{expected} is not in the overview: {names:?}"
        );
    }
    assert_eq!(names.len(), 37, "{names:?}");

    for name in &names {
        let mut args: Vec<&str> = name.split(' ').collect();
        args.push("--help");
        let help = sandbox.ok(&args);
        assert!(help.contains("\nUsage: writer "), "{name}: {help}");
        assert!(help.contains("\nExample"), "{name} has no example: {help}");
        let example = help
            .lines()
            .skip_while(|line| !line.starts_with("Example"))
            .nth(1)
            .unwrap_or_default();
        assert!(
            example.contains("writer "),
            "{name}: the example is a command line: {example}"
        );
        // `writer help <command>` is the same text.
        let mut by_help = vec!["help"];
        by_help.extend(name.split(' '));
        assert_eq!(sandbox.ok(&by_help), help, "{name}");
        // The help needs no library and touches none.
        assert!(!sandbox.library().join("library.db").exists());
    }
}

#[test]
fn the_version_is_the_version_of_the_app() {
    let sandbox = Sandbox::new();
    let version = sandbox.ok(&["--version"]);
    assert_eq!(version, format!("writer {}\n", env!("CARGO_PKG_VERSION")));
    assert_eq!(sandbox.ok(&["-V"]), version);
    assert_eq!(
        sandbox.json(&["status"])["version"],
        env!("CARGO_PKG_VERSION")
    );
}

#[test]
fn a_command_line_that_cannot_be_read_is_exit_code_2_on_standard_error() {
    let sandbox = Sandbox::new();
    let unknown = sandbox.fail(&["serch", "x"], 2);
    assert!(unknown.contains("unknown command \"serch\""), "{unknown}");
    assert!(
        unknown.contains("Did you mean \"writer search\"?"),
        "{unknown}"
    );
    assert!(unknown.contains("writer --help"), "{unknown}");

    let missing = sandbox.fail(&["get"], 2);
    assert!(missing.contains("<document> is missing"), "{missing}");
    assert!(
        missing.contains("usage: writer get <document>"),
        "{missing}"
    );
    assert!(missing.contains("writer get --help"), "{missing}");

    let option = sandbox.fail(&["edit", "x", "--old", "a", "--neu", "b"], 2);
    assert!(option.contains("has no option --neu"), "{option}");

    for args in [
        &["edit", "x", "--old", "a"][..],
        &["insert", "x", "--content", "a"],
        &[
            "insert",
            "x",
            "--at-start",
            "--after",
            "H",
            "--content",
            "a",
        ],
        &["rename", "x"],
        &["move", "x"],
        &["export", "x"],
        &["comment", "add", "x", "--quote", "a"],
        &["comment", "resolve", "x", "three"],
        &["history", "show", "latest"],
        &["list", "--depth", "deep"],
        &["search", "a", "--limit", "0"],
        &["agents", "install"],
        &["agents", "install", "claude-code", "--all"],
        &["append", "x", "--content", "a", "--file", "b"],
        &["get", "x", "--format", "pdf"],
        &["append", "x"],
    ] {
        let message = sandbox.fail(args, 2);
        assert!(message.len() > "writer: ".len(), "{args:?}");
    }
    // None of that made a library.
    assert!(!sandbox.library().join("library.db").exists());
}

#[test]
fn with_json_an_error_is_one_json_line_on_standard_error() {
    let sandbox = Sandbox::new();
    for args in [
        &["get", "--json"][..],
        &["--json", "nonsense"],
        &["get", "Nothing here", "--json"],
    ] {
        let run = sandbox.run(args);
        assert_ne!(run.code, 0, "{args:?}");
        assert_eq!(run.stdout, "", "{args:?}");
        assert_eq!(run.stderr.lines().count(), 1, "{args:?}: {}", run.stderr);
        let error = run.error();
        assert!(error["code"].is_string(), "{args:?}: {error}");
        assert!(
            error["message"]
                .as_str()
                .is_some_and(|message| !message.is_empty()),
            "{args:?}: {error}"
        );
    }
    let usage = sandbox.run(&["get", "--json"]);
    assert_eq!(usage.code, 2);
    assert_eq!(usage.error()["code"], "invalid");
}

#[test]
fn a_reader_that_goes_away_is_not_an_error() {
    // `writer --help | head -1`: the pipe closes while the tool still writes.
    let sandbox = Sandbox::new();
    let mut child = sandbox.command().arg("--help").spawn().unwrap();
    drop(child.stdout.take());
    let status = child.wait().unwrap();
    assert!(status.success(), "{status}");
}

#[test]
fn json_output_is_one_document_and_nothing_else() {
    let sandbox = Sandbox::new();
    let run = sandbox.run(&["status", "--json"]);
    assert_eq!(run.code, 0);
    assert_eq!(run.stderr, "");
    let value: Value =
        serde_json::from_str(&run.stdout).expect("standard output is exactly one JSON document");
    assert!(value.is_object());
    assert!(run.stdout.ends_with("}\n"));
}

#[test]
fn a_failure_names_the_option_to_type_next() {
    // The messages of the command layer name fields ("occurrence", "permanently"); at the
    // command line the next step is an option, and the message has to say which.
    let sandbox = Sandbox::new();
    let doc = sandbox.create(
        "Chapter",
        None,
        "# Chapter\n\nThe letter came. The letter stayed.\n\n## Notes\n\none\n\n## Notes\n\ntwo\n",
    );
    let several = sandbox.fail(&["edit", &doc, "--old", "letter", "--new", "note"], 7);
    assert!(several.contains("--occurrence <n>"), "{several}");
    assert!(several.contains("--all"), "{several}");
    let none = sandbox.fail(&["edit", &doc, "--old", "telegram", "--new", "note"], 6);
    assert!(none.contains("writer find <document> <text>"), "{none}");
    assert!(none.contains("--format text"), "{none}");
    let headings = sandbox.fail(&["get", &doc, "--section", "Notes"], 4);
    assert!(headings.contains("2 headings"), "{headings}");
    assert!(headings.contains("--occurrence <n>"), "{headings}");
    assert!(
        !headings.contains("longer path"),
        "a heading has no path: {headings}"
    );
    let second = sandbox.ok(&["get", &doc, "--section", "Notes", "--occurrence", "2"]);
    assert!(
        second.contains("two") && !second.contains("one"),
        "{second}"
    );
    let target = sandbox.work().join("chapter.md");
    let target = target.to_str().unwrap();
    sandbox.ok(&["export", &doc, "--to", target]);
    let exists = sandbox.fail(&["export", &doc, "--to", target], 12);
    assert!(exists.contains("--overwrite"), "{exists}");
    sandbox.ok(&["export", &doc, "--to", target, "--overwrite"]);
    let unconfirmed = sandbox.fail(&["empty-trash"], 8);
    assert!(unconfirmed.contains("--permanently"), "{unconfirmed}");
    // With --json the hint is a field of its own beside the message.
    let run = sandbox.run(&["edit", &doc, "--old", "letter", "--new", "note", "--json"]);
    assert_eq!(run.code, 7);
    assert!(
        run.error()["hint"]
            .as_str()
            .is_some_and(|hint| hint.contains("--occurrence")),
        "{}",
        run.stderr
    );
    // An ambiguous name is told apart by id: no option is named for it.
    sandbox.create("Twin", None, "a");
    sandbox.create("Twin", Some("Folder"), "b");
    let twins = sandbox.fail(&["get", "Twin"], 4);
    assert!(
        twins.contains("doc:") && !twins.contains("--occurrence"),
        "{twins}"
    );
}

#[test]
fn the_dry_run_of_a_request_with_nothing_to_do_says_so() {
    // A dry run reports what the real run would: also that it would change nothing.
    let sandbox = Sandbox::new();
    let text = "# Chapter\n\nThe letter came.\n";
    let doc = sandbox.create("Draft", None, text);
    let dry = sandbox.ok(&["set", &doc, "--content", text, "--dry-run"]);
    assert!(dry.contains("would change nothing"), "{dry}");
    assert!(!dry.contains("Replaced"), "{dry}");
    let real = sandbox.ok(&["set", &doc, "--content", text]);
    assert!(real.starts_with("Nothing changed"), "{real}");
    let dry = sandbox.ok(&["rename", &doc, "Draft", "--dry-run"]);
    assert!(dry.contains("would change nothing"), "{dry}");
    // And one with something to do still reports it.
    let dry = sandbox.ok(&["rename", &doc, "Chapter one", "--dry-run"]);
    assert!(dry.starts_with("Dry run: nothing was changed."), "{dry}");
    assert!(dry.contains("Chapter one"), "{dry}");
}
