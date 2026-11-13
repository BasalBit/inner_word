//! How the program is started: the flags and the environment that make a `claude`
//! process a writing assistant and nothing else.
//!
//! Every flag and variable here was checked against Claude Code 2.1.322 on 2026-11-12,
//! in two ways: against the real service with the user's own configuration in place
//! (the `system/init` event and the usage of the answer), and against a stand-in for
//! the service with a configuration directory full of marked customisations (a user
//! `CLAUDE.md`, a project `CLAUDE.md`, hooks, a skill, a command, user and project MCP
//! servers), where the request the program sends can be read whole. The table in
//! [`isolation_flags`] says what each one was seen to do.
//!
//! What stays and cannot be removed: the program puts one line of its own before the
//! system prompt ("You are a Claude agent, built on Anthropic's Claude Agent SDK."),
//! and adds a short note about its environment (working directory, platform, shell,
//! the model's name, today's date). The system prompts tell the assistant to ignore
//! the note.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::json;

use crate::backend::{Selection, ToolServer};

/// How often the program tries a request again before it gives up (connection lost,
/// the service overloaded). Four tries take about eight seconds; the program's own
/// default of ten would leave a writer waiting for minutes.
pub const MAX_RETRIES: &str = "4";

/// Variables of the app's own environment that must not reach the program: they mark
/// a process as started from inside a Claude Code session (when the app itself was
/// started from one), they hand it that session's own channels (its bridge, its
/// messaging socket and the token for it), or they overrule the model and effort the
/// user chose here. An app started from the Finder has none of them; one started from
/// a terminal that Claude Code runs has them all.
const REMOVED_ENV: [&str; 15] = [
    "CLAUDECODE",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_SESSION_ATTENDED",
    "CLAUDE_CODE_BRIDGE_SESSION_ID",
    "CLAUDE_CODE_MESSAGING_SOCKET",
    "CLAUDE_CODE_MESSAGING_TOKEN",
    "CLAUDE_CODE_EXECPATH",
    "CLAUDE_CODE_SSE_PORT",
    "CLAUDE_CODE_EFFORT_LEVEL",
    "CLAUDE_CODE_SAFE_MODE",
    "CLAUDE_CODE_SIMPLE",
    "CLAUDE_PID",
    "CLAUDE_EFFORT",
];

/// The variables every process of the assistant is started with, and what each does.
///
/// | Variable | Seen to do |
/// |---|---|
/// | `CLAUDE_CODE_DISABLE_CLAUDE_MDS=1` | no `CLAUDE.md` of the user or of a folder reaches the model |
/// | `CLAUDE_CODE_DISABLE_AUTO_MEMORY=1` | no memory folder is made or read (`memory_paths` is absent from `system/init`) |
/// | `CLAUDE_CODE_SKIP_PROMPT_HISTORY=1` | nothing is written to the prompt history or as a transcript |
/// | `ENABLE_CLAUDEAI_MCP_SERVERS=false` | the connectors of the user's claude.ai account are not fetched |
/// | `ENABLE_TOOL_SEARCH=false` | the tool list is sent as it is, without the program's placeholder tool |
/// | `CLAUDE_CODE_DISABLE_ADVISOR_TOOL=1` | the program's "advisor" (a second, larger model the answering model may consult on the user's allowance) is not offered: the line that announces it is gone from the debug log and the request is smaller |
/// | `CLAUDE_CODE_DISABLE_TERMINAL_TITLE=1` | no extra request for a session title |
/// | `CLAUDE_CODE_DISABLE_BUNDLED_SKILLS=1`, `CLAUDE_CODE_DISABLE_CRON=1`, `CLAUDE_CODE_DISABLE_BACKGROUND_TASKS=1`, `CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION=false`, `CLAUDE_CODE_AUTO_CONNECT_IDE=false` | features of the coding agent that have no place here stay off |
/// | `CLAUDE_CODE_MAX_RETRIES=4` | a lost connection is reported after about eight seconds |
pub fn isolation_env() -> Vec<(&'static str, &'static str)> {
    vec![
        ("CLAUDE_CODE_DISABLE_CLAUDE_MDS", "1"),
        ("CLAUDE_CODE_DISABLE_AUTO_MEMORY", "1"),
        ("CLAUDE_CODE_SKIP_PROMPT_HISTORY", "1"),
        ("ENABLE_CLAUDEAI_MCP_SERVERS", "false"),
        ("ENABLE_TOOL_SEARCH", "false"),
        ("CLAUDE_CODE_DISABLE_ADVISOR_TOOL", "1"),
        ("CLAUDE_CODE_DISABLE_TERMINAL_TITLE", "1"),
        ("CLAUDE_CODE_DISABLE_BUNDLED_SKILLS", "1"),
        ("CLAUDE_CODE_DISABLE_CRON", "1"),
        ("CLAUDE_CODE_DISABLE_BACKGROUND_TASKS", "1"),
        ("CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION", "false"),
        ("CLAUDE_CODE_AUTO_CONNECT_IDE", "false"),
        ("CLAUDE_CODE_MAX_RETRIES", MAX_RETRIES),
    ]
}

/// The flags every conversation is started with, and what each does.
///
/// | Flag | Seen to do |
/// |---|---|
/// | `-p --input-format stream-json --output-format stream-json --verbose` | one process answers message after message, as lines of JSON |
/// | `--include-partial-messages` | the text arrives as it is written (`stream_event`) |
/// | `--system-prompt <ours>` | replaces the coding agent's prompt: the request carries ours and one line the program adds |
/// | `--tools ""` | no built-in tool: `tools` in `system/init` is empty, the request carries none |
/// | `--strict-mcp-config --mcp-config <json>` | only the servers named here: none of the user's, of a project's or of the claude.ai account |
/// | `--disable-slash-commands` | no skill and no command: `skills` and `slash_commands` are empty |
/// | `--setting-sources ""` | no settings file is read: no hook runs, no plugin of the user loads, no permission rule of the user applies |
/// | `--no-session-persistence` | no transcript is written under `~/.claude/projects` |
/// | `--permission-mode dontAsk` | a tool that is not allowed is refused without a question |
///
/// `--bare` is not used: it never reads the subscription sign-in. `--safe-mode` is not
/// used: it also turns off the server given with `--mcp-config`.
pub fn isolation_flags() -> Vec<&'static str> {
    vec![
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--tools",
        "",
        "--strict-mcp-config",
        "--disable-slash-commands",
        "--setting-sources",
        "",
        "--no-session-persistence",
        "--permission-mode",
        "dontAsk",
    ]
}

/// The name the program gives a tool of an MCP server: `mcp__the-writer__read_document`.
pub fn prefixed_tool(server: &str, tool: &str) -> String {
    format!("mcp__{server}__{tool}")
}

/// The server's own name of a tool, from the program's: `read_document` from
/// `mcp__the-writer__read_document`. A name without the prefix is returned as it is.
pub fn bare_tool<'a>(server: Option<&str>, tool: &'a str) -> &'a str {
    if let Some(server) = server
        && let Some(rest) = tool.strip_prefix("mcp__")
        && let Some(rest) = rest.strip_prefix(server)
        && let Some(rest) = rest.strip_prefix("__")
    {
        return rest;
    }
    match tool
        .strip_prefix("mcp__")
        .and_then(|rest| rest.split_once("__"))
    {
        Some((_, bare)) => bare,
        None => tool,
    }
}

/// The value of `--mcp-config`: the one server of the conversation, or none.
pub fn mcp_config(tools: Option<&ToolServer>) -> String {
    match tools {
        None => json!({ "mcpServers": {} }).to_string(),
        Some(server) => {
            let mut entry = json!({
                "command": server.command.to_string_lossy(),
                "args": server.args,
            });
            if !server.env.is_empty() {
                let env: serde_json::Map<String, serde_json::Value> = server
                    .env
                    .iter()
                    .map(|(key, value)| (key.clone(), json!(value)))
                    .collect();
                entry["env"] = serde_json::Value::Object(env);
            }
            let mut servers = serde_json::Map::new();
            servers.insert(server.name.clone(), entry);
            json!({ "mcpServers": servers }).to_string()
        }
    }
}

/// The arguments of a conversation: the isolation flags, the system prompt, the model
/// and effort when they are named, and the tools.
///
/// With tools, the read-only ones are allowed by name (`--allowedTools`) and every
/// other tool of the server is withheld by name (`--disallowedTools`): the model is
/// not even told they exist, and a call to one is answered "No such tool available".
/// A denial outranks any rule that would allow it.
pub fn conversation_args(
    system_prompt: &str,
    selection: &Selection,
    tools: Option<&ToolServer>,
) -> Vec<String> {
    let mut args: Vec<String> = isolation_flags().into_iter().map(String::from).collect();
    args.push("--system-prompt".to_string());
    args.push(system_prompt.to_string());
    if let Some(model) = selection.model.id() {
        args.push("--model".to_string());
        args.push(model.to_string());
    }
    if let Some(effort) = selection.effort.id() {
        args.push("--effort".to_string());
        args.push(effort.to_string());
    }
    args.push("--mcp-config".to_string());
    args.push(mcp_config(tools));
    if let Some(server) = tools {
        let names = |tools: &[String]| {
            tools
                .iter()
                .map(|tool| prefixed_tool(&server.name, tool))
                .collect::<Vec<_>>()
                .join(",")
        };
        if !server.allowed.is_empty() {
            args.push("--allowedTools".to_string());
            args.push(names(&server.allowed));
        }
        if !server.denied.is_empty() {
            args.push("--disallowedTools".to_string());
            args.push(names(&server.denied));
        }
    }
    args
}

/// The arguments of the short-lived process that is only asked what it offers
/// (`initialize`): the isolation flags and nothing else. It is sent no message.
pub fn probe_args() -> Vec<String> {
    let mut args: Vec<String> = isolation_flags().into_iter().map(String::from).collect();
    args.push("--system-prompt".to_string());
    args.push("The Writer".to_string());
    args.push("--mcp-config".to_string());
    args.push(mcp_config(None));
    args
}

/// The `PATH` for the program: its own folder first, then what the app has, then the
/// system's folders (an app started from the Finder has little more than those).
fn path_for(program: &Path) -> OsString {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(dir) = program.parent().filter(|dir| dir.is_absolute()) {
        dirs.push(dir.to_path_buf());
    }
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    for dir in [
        "/opt/homebrew/bin",
        "/usr/local/bin",
        "/usr/bin",
        "/bin",
        "/usr/sbin",
        "/sbin",
    ] {
        dirs.push(PathBuf::from(dir));
    }
    let mut seen = std::collections::HashSet::new();
    dirs.retain(|dir| !dir.as_os_str().is_empty() && seen.insert(dir.clone()));
    std::env::join_paths(dirs).unwrap_or_else(|_| OsString::from("/usr/bin:/bin"))
}

/// The folder the assistant's processes run in when the caller names none: an empty
/// folder in the user's temporary directory, made on demand. Not the home, not a
/// project, and without the user's name in its path (the program tells the model its
/// working directory).
pub fn neutral_dir() -> PathBuf {
    let dir = std::env::temp_dir().join("the-writer-assistant");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// A command for `program` with the environment every process of the assistant gets:
/// the app's own (which carries the user's sign-in where that lives in variables),
/// without the markers of an enclosing Claude Code session, with the isolation
/// variables, and with a `PATH` that works when the app was started from the Finder.
/// It runs in `dir`, or in the neutral folder.
pub fn base_command(program: &Path, dir: Option<&Path>) -> Command {
    let mut command = Command::new(program);
    for name in REMOVED_ENV {
        command.env_remove(name);
    }
    for (name, value) in isolation_env() {
        command.env(name, value);
    }
    command.env("PATH", path_for(program));
    match dir {
        Some(dir) => {
            let _ = std::fs::create_dir_all(dir);
            command.current_dir(dir);
        }
        None => {
            command.current_dir(neutral_dir());
        }
    }
    command
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{EffortChoice, ModelChoice};
    use crate::tools::library_tools;

    fn value_after<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
        let at = args.iter().position(|arg| arg == flag)?;
        args.get(at + 1).map(String::as_str)
    }

    #[test]
    fn a_quick_conversation_has_no_tools_no_settings_and_no_session() {
        let args = conversation_args("You help.", &Selection::named("haiku", "low"), None);
        assert_eq!(value_after(&args, "--system-prompt"), Some("You help."));
        assert_eq!(value_after(&args, "--model"), Some("haiku"));
        assert_eq!(value_after(&args, "--effort"), Some("low"));
        assert_eq!(value_after(&args, "--tools"), Some(""));
        assert_eq!(value_after(&args, "--setting-sources"), Some(""));
        assert_eq!(value_after(&args, "--permission-mode"), Some("dontAsk"));
        assert_eq!(value_after(&args, "--input-format"), Some("stream-json"));
        assert_eq!(value_after(&args, "--output-format"), Some("stream-json"));
        assert_eq!(
            value_after(&args, "--mcp-config"),
            Some(r#"{"mcpServers":{}}"#)
        );
        for flag in [
            "-p",
            "--verbose",
            "--include-partial-messages",
            "--strict-mcp-config",
            "--disable-slash-commands",
            "--no-session-persistence",
        ] {
            assert!(args.iter().any(|arg| arg == flag), "{flag}");
        }
        // Nothing allows or withholds a tool where there is none, and the flags the
        // brief rules out are not there.
        for flag in [
            "--allowedTools",
            "--disallowedTools",
            "--bare",
            "--safe-mode",
            "--resume",
            "--continue",
            "--dangerously-skip-permissions",
        ] {
            assert!(!args.iter().any(|arg| arg == flag), "{flag}");
        }
    }

    #[test]
    fn the_agents_default_names_no_model_and_no_effort() {
        let args = conversation_args("x", &Selection::default(), None);
        assert!(!args.iter().any(|arg| arg == "--model"));
        assert!(!args.iter().any(|arg| arg == "--effort"));
        let args = conversation_args(
            "x",
            &Selection {
                model: ModelChoice::named("opus"),
                effort: EffortChoice::AgentDefault,
            },
            None,
        );
        assert_eq!(value_after(&args, "--model"), Some("opus"));
        assert!(!args.iter().any(|arg| arg == "--effort"));
    }

    #[test]
    fn the_side_chat_gets_the_library_server_with_its_read_only_tools_and_nothing_that_writes() {
        let tools = library_tools(
            "/Applications/The Writer.app/Contents/MacOS/writer",
            "/tmp/a library",
        );
        let args = conversation_args("x", &Selection::default(), Some(&tools));
        let config: serde_json::Value =
            serde_json::from_str(value_after(&args, "--mcp-config").unwrap()).unwrap();
        assert_eq!(
            config,
            serde_json::json!({"mcpServers": {"the-writer": {
                "command": "/Applications/The Writer.app/Contents/MacOS/writer",
                "args": ["--library", "/tmp/a library", "mcp"],
            }}})
        );
        let allowed: Vec<&str> = value_after(&args, "--allowedTools")
            .unwrap()
            .split(',')
            .collect();
        let denied: Vec<&str> = value_after(&args, "--disallowedTools")
            .unwrap()
            .split(',')
            .collect();
        assert!(allowed.contains(&"mcp__the-writer__read_document"));
        assert!(allowed.contains(&"mcp__the-writer__search_library"));
        assert!(denied.contains(&"mcp__the-writer__edit_document"));
        assert!(denied.contains(&"mcp__the-writer__trash_document"));
        // A tool that brings a file of the Mac into a document is one that writes.
        assert!(denied.contains(&"mcp__the-writer__add_image"));
        // Every tool of the server is on exactly one of the two lists: the twelve that
        // only read, and the others.
        assert_eq!(allowed.len(), 12);
        assert_eq!(allowed.len() + denied.len(), 36);
        assert!(allowed.iter().all(|tool| !denied.contains(tool)));
        // The built-in tools stay off with the server there.
        assert_eq!(value_after(&args, "--tools"), Some(""));
    }

    #[test]
    fn a_tool_is_named_with_and_without_the_programs_prefix() {
        assert_eq!(
            prefixed_tool("the-writer", "read_document"),
            "mcp__the-writer__read_document"
        );
        assert_eq!(
            bare_tool(Some("the-writer"), "mcp__the-writer__read_document"),
            "read_document"
        );
        assert_eq!(
            bare_tool(None, "mcp__the-writer__read_document"),
            "read_document"
        );
        assert_eq!(
            bare_tool(Some("the-writer"), "mcp__other__thing_one"),
            "thing_one"
        );
        assert_eq!(bare_tool(Some("the-writer"), "Bash"), "Bash");
    }

    #[test]
    fn the_environment_turns_the_coding_agent_off_and_keeps_the_sign_in() {
        let command = base_command(Path::new("/somewhere/bin/claude"), None);
        let env: std::collections::HashMap<String, Option<String>> = command
            .get_envs()
            .map(|(name, value)| {
                (
                    name.to_string_lossy().into_owned(),
                    value.map(|value| value.to_string_lossy().into_owned()),
                )
            })
            .collect();
        for (name, value) in isolation_env() {
            assert_eq!(env.get(name), Some(&Some(value.to_string())), "{name}");
        }
        // The markers of an enclosing session are taken out...
        for name in REMOVED_ENV {
            assert_eq!(env.get(name), Some(&None), "{name}");
        }
        // ...and nothing that carries a sign-in is touched.
        for name in [
            "ANTHROPIC_API_KEY",
            "CLAUDE_CONFIG_DIR",
            "CLAUDE_CODE_OAUTH_TOKEN",
            "HOME",
        ] {
            assert!(!env.contains_key(name), "{name}");
        }
        let path = env["PATH"].clone().unwrap();
        assert!(path.starts_with("/somewhere/bin:"), "{path}");
        assert!(path.contains("/usr/bin"), "{path}");
        // It runs in the neutral folder.
        let dir = command.get_current_dir().unwrap();
        assert_eq!(dir, neutral_dir());
        assert!(dir.is_dir());
        assert!(dir.ends_with("the-writer-assistant"));
    }
}
