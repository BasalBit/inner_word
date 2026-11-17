//! The assistant is a writing assistant and not a coding agent: what the program is
//! started with, what it is sent, and what the recordings of the real program (made
//! with the user's own configuration in place: skills, a session hook, plugins, a
//! claude.ai connector) show it had.

use serde_json::{Value, json};
use innerword_assistant::{
    ConversationOptions, QUICK_SYSTEM_PROMPT, Selection, library_tools, read_only_tools,
};

use super::support::{Stage, Talk, fixture, value_after};

fn init_of(recording: &str) -> Value {
    std::fs::read_to_string(fixture(recording))
        .unwrap()
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|line| line["type"] == "system" && line["subtype"] == "init")
        .expect("the recording begins a turn")
}

#[test]
fn the_program_is_started_with_our_prompt_and_nothing_of_the_users_setup() {
    let stage = Stage::new(json!({"turns": [{"text": "ok", "pieces": 1}]}));
    let talk = Talk::start(
        &stage.backend(),
        ConversationOptions::new(QUICK_SYSTEM_PROMPT)
            .with_selection(Selection::named("haiku", "low")),
    );
    talk.ask("Hello?");
    let launch = stage.first_conversation();
    let argv: Vec<String> = launch["argv"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        value_after(&argv, "--system-prompt"),
        Some(QUICK_SYSTEM_PROMPT)
    );
    assert_eq!(value_after(&argv, "--tools"), Some(""), "no built-in tool");
    assert_eq!(
        value_after(&argv, "--setting-sources"),
        Some(""),
        "no settings file, so no hook"
    );
    assert_eq!(
        value_after(&argv, "--mcp-config"),
        Some(r#"{"mcpServers":{}}"#)
    );
    assert_eq!(value_after(&argv, "--permission-mode"), Some("dontAsk"));
    for flag in [
        "--strict-mcp-config",
        "--disable-slash-commands",
        "--no-session-persistence",
    ] {
        assert!(argv.iter().any(|arg| arg == flag), "{flag}");
    }
    for flag in [
        "--bare",
        "--resume",
        "--continue",
        "--session-id",
        "--add-dir",
        "--dangerously-skip-permissions",
        "--append-system-prompt",
    ] {
        assert!(!argv.iter().any(|arg| arg == flag), "{flag}");
    }
    // The environment turns off what flags do not reach.
    let env = &launch["env"];
    for (name, value) in [
        ("CLAUDE_CODE_DISABLE_CLAUDE_MDS", "1"),
        ("CLAUDE_CODE_DISABLE_AUTO_MEMORY", "1"),
        ("CLAUDE_CODE_SKIP_PROMPT_HISTORY", "1"),
        ("CLAUDE_CODE_DISABLE_ADVISOR_TOOL", "1"),
        ("ENABLE_CLAUDEAI_MCP_SERVERS", "false"),
        ("ENABLE_TOOL_SEARCH", "false"),
        ("CLAUDE_CODE_MAX_RETRIES", "4"),
    ] {
        assert_eq!(env[name], value, "{name}");
    }
    // Nothing marks the process as started from inside a Claude Code session (this
    // test may well run inside one), and nothing overrules the chosen effort.
    for name in [
        "CLAUDECODE",
        "CLAUDE_CODE_ENTRYPOINT",
        "CLAUDE_CODE_CHILD_SESSION",
        "CLAUDE_CODE_SESSION_ID",
        "CLAUDE_CODE_SESSION_ATTENDED",
        "CLAUDE_CODE_BRIDGE_SESSION_ID",
        "CLAUDE_CODE_MESSAGING_SOCKET",
        "CLAUDE_CODE_MESSAGING_TOKEN",
        "CLAUDE_CODE_EXECPATH",
        "CLAUDE_CODE_EFFORT_LEVEL",
        "CLAUDE_PID",
        "CLAUDE_EFFORT",
    ] {
        assert!(env.get(name).is_none(), "{name} reached the program");
    }
    // It runs in a neutral folder: not the home, not a project, not this repository.
    let cwd = launch["cwd"].as_str().unwrap();
    assert!(cwd.ends_with("inner-word-assistant"), "{cwd}");
    assert!(!cwd.contains(env!("CARGO_MANIFEST_DIR")));
}

#[test]
fn a_message_reaches_the_program_exactly_as_it_was_written() {
    let stage = Stage::new(json!({"turns": [{"echo": true, "pieces": 2}]}));
    let talk = Talk::start(&stage.backend(), ConversationOptions::new("x"));
    // What a writer types: a path after an @, a line that begins with a slash, a
    // hash, an exclamation mark, quotes, a tab, another alphabet, an emoji.
    let text = "Is @~/.ssh/config or @/etc/hosts the right image here?\n/clear the table, she said.\n# not a heading\n! not a command\n\"Ça va?\" — Ναι.\t🙂";
    let answer = talk.ask(text);
    assert_eq!(answer.text, text, "the stand-in echoes what it was sent");
    let sent = &stage.received()[0];
    assert_eq!(sent["type"], "user");
    // The mark that tells the program to deliver the text as written: without it the
    // program replaces every @path with the contents of that file and runs /commands.
    assert_eq!(sent["client_composed"], true);
    assert_eq!(sent["message"]["role"], "user");
    assert_eq!(
        sent["message"]["content"],
        json!([{"type": "text", "text": text}])
    );
}

#[test]
fn the_side_chat_is_given_the_read_only_tools_and_no_other() {
    let stage = Stage::new(json!({"turns": [{"text": "ok", "pieces": 1}]}));
    let tools = library_tools(stage.path().join("innerword"), stage.path().join("library"));
    let talk = Talk::start(
        &stage.backend(),
        ConversationOptions::new("x").with_tools(Some(tools)),
    );
    talk.ask("Hello?");
    let argv = &stage.conversations()[0];
    let allowed: Vec<&str> = value_after(argv, "--allowedTools")
        .unwrap()
        .split(',')
        .collect();
    let denied: Vec<&str> = value_after(argv, "--disallowedTools")
        .unwrap()
        .split(',')
        .collect();
    let expected: Vec<String> = read_only_tools()
        .iter()
        .map(|tool| format!("mcp__inner-word__{tool}"))
        .collect();
    assert_eq!(allowed, expected);
    for tool in &denied {
        assert!(
            !allowed.contains(tool),
            "{tool} is both allowed and withheld"
        );
    }
    for writes in [
        "create_document",
        "replace_document",
        "edit_document",
        "append_to_document",
        "trash_document",
        "move_folder",
        "restore_version",
        "export",
        "add_comment",
    ] {
        assert!(
            denied.contains(&format!("mcp__inner-word__{writes}").as_str()),
            "{writes} is not withheld"
        );
    }
}

#[test]
fn the_real_program_had_no_tool_no_skill_no_command_and_no_server_in_the_quick_chat() {
    // Recorded 2026-11-12 from Claude Code 2.1.322 with the user's own configuration in
    // place and the flags and variables of this crate.
    let init = init_of("quick-turn.jsonl");
    assert_eq!(init["tools"], json!([]));
    assert_eq!(init["mcp_servers"], json!([]));
    assert_eq!(init["skills"], json!([]));
    assert_eq!(init["slash_commands"], json!([]));
    assert_eq!(init["permissionMode"], "dontAsk");
    assert_eq!(init["model"], "claude-haiku-5-5");
    assert!(init.get("memory_paths").is_none(), "no memory folder");
    // The only plugins are the ones built into the program.
    for plugin in init["plugins"].as_array().unwrap() {
        assert_eq!(plugin["path"], "builtin", "{plugin}");
    }
    // The same three turns later in one process, the last after an interrupt.
    let later = init_of("three-turns.jsonl");
    assert_eq!(later["tools"], json!([]));
    assert_eq!(later["mcp_servers"], json!([]));
}

#[test]
fn the_real_program_had_exactly_the_read_only_tools_in_the_side_chat() {
    let init = init_of("side-tool-turn.jsonl");
    let mut had: Vec<String> = init["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool.as_str().unwrap().to_string())
        .collect();
    let mut expected: Vec<String> = read_only_tools()
        .iter()
        .map(|tool| format!("mcp__inner-word__{tool}"))
        .collect();
    had.sort();
    expected.sort();
    assert_eq!(
        had, expected,
        "the tools the model was offered are the server's read-only tools and nothing else"
    );
    assert_eq!(
        init["mcp_servers"],
        json!([{"name": "inner-word", "status": "connected", "source": "dynamic"}])
    );
    assert_eq!(init["skills"], json!([]));
    assert_eq!(init["slash_commands"], json!([]));
}
