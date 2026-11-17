//! The MCP side: for each agent, the ready-to-paste command or configuration entry that adds
//! Inner Word's MCP server (`innerword mcp`, over standard input and output) to it.
//!
//! Nothing here edits an agent's configuration or runs an agent's command. The user copies
//! the command and runs it, or pastes the entry into the file that is named.
//!
//! The server was registered as [`LEGACY_MCP_SERVER_NAME`] when the app was called The
//! Writer. Beside the instructions that add it, [`McpInstructions::old_entry`] says in one
//! line how that entry is taken away: the agent's own remove command with the old name
//! where the table has one, or which entry to delete from which file. Nothing is checked
//! for it, since that would mean reading the agent's configuration.
//!
//! The templates of [`crate::agents`] take one thing, the absolute path of the `innerword`
//! binary. It contains a space (`Inner Word.app`), so in a command it is quoted for the
//! shell ([`crate::shell::quote`]) and in a configuration entry it is a string literal of
//! the file's format ([`crate::shell::string_literal`]).

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::agents::{AGENTS, Agent, AgentId, ConfigFormat, MCP_SERVER_NAME, Source};
use crate::error::SetupError;
use crate::legacy::{LEGACY_APP_NAME, LEGACY_MCP_SERVER_NAME};
use crate::shell;

/// The placeholder of the templates.
const PLACEHOLDER: &str = "{writer}";

/// The command lines for one agent, ready to paste into Terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct McpCommandLines {
    /// Adds the server.
    pub add: String,
    /// Removes it again, when the agent has a command for that.
    pub remove: Option<String>,
    /// Shows whether it is registered, when the agent has a command for that.
    pub check: Option<String>,
}

/// The configuration entry for one agent, ready to paste.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct McpConfigEntry {
    /// The file the entry belongs in, absolute, when the agent documents one path and the
    /// home directory was given.
    pub file: Option<PathBuf>,
    /// The same file as a person reads it (`~/.cursor/mcp.json`).
    pub file_display: Option<String>,
    /// How the agent itself opens the file (a menu path or a palette command).
    pub open_with: Option<&'static str>,
    pub format: ConfigFormat,
    /// The entry as a complete, valid file of that format.
    pub snippet: String,
    /// Where the entry goes in a file that already has content.
    pub placement: &'static str,
}

/// How the entry the app had people add when it was called The Writer is removed from one
/// agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct McpOldEntry {
    /// The name of that entry ([`LEGACY_MCP_SERVER_NAME`]).
    pub server_name: &'static str,
    /// The agent's own command that removes it, when the agent has a remove command: the
    /// table's, with the old name in place of the new one.
    pub command: Option<String>,
    /// One plain line for a person: the command, or the entry to delete and the file.
    pub line: String,
}

/// How Inner Word's MCP server is added to one agent, with the real path of the binary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct McpInstructions {
    pub agent: AgentId,
    /// The agent's name ("Claude Code").
    pub name: &'static str,
    /// The name the server is registered under ([`MCP_SERVER_NAME`]).
    pub server_name: &'static str,
    /// The command line, when the agent has one.
    pub command: Option<McpCommandLines>,
    /// The configuration entry, when the agent documents one. An agent may have both: the
    /// command is the easier way.
    pub config: Option<McpConfigEntry>,
    /// One line a person needs beside it: a restart, a scope, a caveat.
    pub note: Option<&'static str>,
    /// How to remove the entry of the app's old name, which this agent may still have.
    pub old_entry: McpOldEntry,
    /// The documentation this was taken from; the first one is the page to link to.
    pub sources: Vec<Source>,
}

impl McpInstructions {
    /// The address of the agent's own documentation about MCP servers, for a link.
    pub fn docs_url(&self) -> Option<&'static str> {
        self.sources
            .iter()
            .find(|source| source.has_link())
            .map(|source| source.url)
    }
}

/// The instructions for one agent, or `None` when nothing is known about MCP for it.
///
/// `binary` is the absolute path of the `innerword` tool. `home` is the home directory the
/// configuration file is shown under; without it `file` is `None` and `file_display` still
/// reads `~/...`.
pub fn mcp_instructions(
    agent: AgentId,
    binary: &Path,
    home: Option<&Path>,
) -> Result<Option<McpInstructions>, SetupError> {
    if !binary.is_absolute() {
        return Err(SetupError::BinaryNotAbsolute(binary.to_path_buf()));
    }
    Ok(render(agent.agent(), binary, home))
}

/// The instructions for every agent that has any, in the order of the table.
pub fn all_mcp_instructions(
    binary: &Path,
    home: Option<&Path>,
) -> Result<Vec<McpInstructions>, SetupError> {
    if !binary.is_absolute() {
        return Err(SetupError::BinaryNotAbsolute(binary.to_path_buf()));
    }
    Ok(AGENTS
        .iter()
        .filter_map(|agent| render(agent, binary, home))
        .collect())
}

fn render(agent: &'static Agent, binary: &Path, home: Option<&Path>) -> Option<McpInstructions> {
    if !agent.mcp.is_known() {
        return None;
    }
    let path = binary.to_string_lossy();
    let quoted = shell::quote(&path);
    let literal = shell::string_literal(&path);
    let mut sources: Vec<Source> = Vec::new();
    let mut add_sources = |list: &[Source]| {
        for source in list {
            if !sources.contains(source) {
                sources.push(*source);
            }
        }
    };
    let command = agent.mcp.command.map(|command| {
        add_sources(command.sources);
        McpCommandLines {
            add: command.add.replace(PLACEHOLDER, &quoted),
            remove: command.remove.map(str::to_string),
            check: command.check.map(str::to_string),
        }
    });
    let config = agent.mcp.config.map(|config| {
        add_sources(config.sources);
        McpConfigEntry {
            file: config
                .home_relative
                .zip(home)
                .map(|(relative, home)| home.join(relative)),
            file_display: config.home_relative.map(|relative| format!("~/{relative}")),
            open_with: config.open_with,
            format: config.format,
            snippet: config.snippet.replace(PLACEHOLDER, &literal),
            placement: config.placement,
        }
    });
    let old_entry = old_entry(agent, config.as_ref());
    Some(McpInstructions {
        agent: agent.id,
        name: agent.name,
        server_name: MCP_SERVER_NAME,
        command,
        config,
        note: agent.mcp.note,
        old_entry,
        sources,
    })
}

/// A command of the table with the old name of the server in place of the new one, word for
/// word.
fn with_old_name(command: &str) -> String {
    command
        .split(' ')
        .map(|word| {
            if word == MCP_SERVER_NAME {
                LEGACY_MCP_SERVER_NAME
            } else {
                word
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn old_entry(agent: &Agent, config: Option<&McpConfigEntry>) -> McpOldEntry {
    let lead = format!("If you added the server when the app was called {LEGACY_APP_NAME}");
    let command = agent
        .mcp
        .command
        .and_then(|command| command.remove)
        .map(with_old_name);
    let line = match (&command, config) {
        (Some(command), _) => format!("{lead}, remove that entry: {command}"),
        (None, Some(config)) => {
            let place = match (&config.file_display, config.open_with) {
                (Some(file), _) => file.clone(),
                (None, Some(open_with)) => format!("the file opened by: {open_with}"),
                (None, None) => "its configuration file".to_string(),
            };
            format!("{lead}, delete its \"{LEGACY_MCP_SERVER_NAME}\" entry from {place}.")
        }
        (None, None) => format!(
            "{lead}, remove its \"{LEGACY_MCP_SERVER_NAME}\" entry from the agent's settings."
        ),
    };
    McpOldEntry {
        server_name: LEGACY_MCP_SERVER_NAME,
        command,
        line,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    const BINARY: &str = "/Applications/Inner Word.app/Contents/MacOS/innerword";

    #[test]
    fn a_relative_binary_is_refused() {
        assert!(matches!(
            mcp_instructions(AgentId::ClaudeCode, Path::new("innerword"), None),
            Err(SetupError::BinaryNotAbsolute(_))
        ));
        assert!(matches!(
            all_mcp_instructions(Path::new("bin/innerword"), None),
            Err(SetupError::BinaryNotAbsolute(_))
        ));
    }

    #[test]
    fn the_commands_for_the_agents_with_one() {
        let of = |id: AgentId| {
            mcp_instructions(id, Path::new(BINARY), None)
                .unwrap()
                .unwrap()
                .command
                .unwrap()
        };
        assert_eq!(
            of(AgentId::ClaudeCode).add,
            "claude mcp add --scope user inner-word -- '/Applications/Inner Word.app/Contents/MacOS/innerword' mcp"
        );
        assert_eq!(
            of(AgentId::ClaudeCode).remove.as_deref(),
            Some("claude mcp remove inner-word")
        );
        assert_eq!(
            of(AgentId::Codex).add,
            "codex mcp add inner-word -- '/Applications/Inner Word.app/Contents/MacOS/innerword' mcp"
        );
        // A path without anything the shell reads needs no quotes.
        let plain = mcp_instructions(AgentId::Codex, Path::new("/usr/local/bin/innerword"), None)
            .unwrap()
            .unwrap();
        assert_eq!(
            plain.command.unwrap().add,
            "codex mcp add inner-word -- /usr/local/bin/innerword mcp"
        );
    }

    /// Whatever the agent's own command is, the shell hands it the path as one word and
    /// `mcp` as the next: the agent's command is replaced by a function that prints its
    /// arguments one per line.
    #[test]
    fn every_command_gives_the_shell_the_path_as_one_word() {
        for binary in [
            BINARY,
            "/Users/o'brien/My Apps/Inner Word.app/Contents/MacOS/innerword",
            "/tmp/a $HOME `x`/innerword",
        ] {
            for instructions in all_mcp_instructions(Path::new(binary), None).unwrap() {
                let Some(command) = instructions.command else {
                    continue;
                };
                let program = command.add.split_whitespace().next().unwrap().to_string();
                let script = format!("{program}() {{ printf '%s\\n' \"$@\"; }}\n{}", command.add);
                let output = Command::new("/bin/sh")
                    .arg("-c")
                    .arg(&script)
                    .output()
                    .expect("run /bin/sh");
                assert!(
                    output.status.success(),
                    "{}: {}",
                    instructions.name,
                    command.add
                );
                let printed = String::from_utf8_lossy(&output.stdout);
                let words: Vec<&str> = printed.lines().collect();
                let at = words.iter().position(|word| *word == binary);
                let at = at.unwrap_or_else(|| {
                    panic!(
                        "{}: the path is not one word in {words:?}",
                        instructions.name
                    )
                });
                assert_eq!(words.get(at + 1), Some(&"mcp"), "{}", command.add);
                assert_eq!(at + 2, words.len(), "{}: mcp is the last word", command.add);
                assert!(words[..at].contains(&MCP_SERVER_NAME), "{}", command.add);
            }
        }
    }

    /// Every JSON entry is a valid JSON file in which the path is exactly the path, and
    /// `mcp` is its one argument.
    #[test]
    fn every_json_entry_parses_and_carries_the_path() {
        for binary in [
            BINARY,
            "/Users/a \"b\" \\ c/Inner Word.app/Contents/MacOS/innerword",
        ] {
            let mut seen = 0;
            for instructions in all_mcp_instructions(Path::new(binary), None).unwrap() {
                let Some(config) = instructions.config else {
                    continue;
                };
                if config.format != ConfigFormat::Json {
                    continue;
                }
                seen += 1;
                let value: serde_json::Value = serde_json::from_str(&config.snippet)
                    .unwrap_or_else(|error| {
                        panic!("{}: {error}\n{}", instructions.name, config.snippet)
                    });
                let server = find_server(&value)
                    .unwrap_or_else(|| panic!("{}: no inner-word entry", instructions.name));
                match &server["command"] {
                    serde_json::Value::String(command) => {
                        assert_eq!(command, binary, "{}", instructions.name);
                        assert_eq!(
                            server["args"],
                            serde_json::json!(["mcp"]),
                            "{}",
                            instructions.name
                        );
                    }
                    serde_json::Value::Array(words) => {
                        assert_eq!(
                            words,
                            &vec![serde_json::json!(binary), serde_json::json!("mcp")]
                        );
                    }
                    other => panic!("{}: command is {other}", instructions.name),
                }
            }
            assert!(seen >= 6, "{seen} JSON entries");
        }
    }

    fn find_server(value: &serde_json::Value) -> Option<&serde_json::Value> {
        let object = value.as_object()?;
        if let Some(server) = object.get(MCP_SERVER_NAME) {
            return Some(server);
        }
        object.values().find_map(find_server)
    }

    /// The TOML entry of Codex, read the way TOML reads a basic string.
    #[test]
    fn the_toml_entry_of_codex() {
        let codex = mcp_instructions(AgentId::Codex, Path::new(BINARY), None)
            .unwrap()
            .unwrap();
        let config = codex.config.unwrap();
        assert_eq!(config.format, ConfigFormat::Toml);
        assert_eq!(
            config.snippet,
            "[mcp_servers.inner-word]\ncommand = \"/Applications/Inner Word.app/Contents/MacOS/innerword\"\nargs = [\"mcp\"]\n"
        );
        assert_eq!(config.file_display.as_deref(), Some("~/.codex/config.toml"));
        // Quotes and backslashes are escaped as TOML basic strings (and JSON) escape them.
        let odd = mcp_instructions(AgentId::Codex, Path::new("/a \"b\" \\ c/innerword"), None)
            .unwrap()
            .unwrap();
        assert!(
            odd.config
                .unwrap()
                .snippet
                .contains(r#"command = "/a \"b\" \\ c/innerword""#)
        );
    }

    #[test]
    fn the_configuration_file_is_shown_under_the_home_that_was_given() {
        let home = Path::new("/Users/somebody");
        let cursor = mcp_instructions(AgentId::Cursor, Path::new(BINARY), Some(home))
            .unwrap()
            .unwrap();
        let config = cursor.config.unwrap();
        assert_eq!(
            config.file,
            Some(PathBuf::from("/Users/somebody/.cursor/mcp.json"))
        );
        assert_eq!(config.file_display.as_deref(), Some("~/.cursor/mcp.json"));
        assert!(cursor.command.is_none());

        let desktop = mcp_instructions(AgentId::ClaudeDesktop, Path::new(BINARY), Some(home))
            .unwrap()
            .unwrap();
        let config = desktop.config.unwrap();
        assert_eq!(
            config.file,
            Some(PathBuf::from(
                "/Users/somebody/Library/Application Support/Claude/claude_desktop_config.json"
            ))
        );
        assert!(desktop.note.unwrap().contains("Quit Claude"));

        // Zed documents no path: the file is reached through the editor.
        let zed = mcp_instructions(AgentId::Zed, Path::new(BINARY), Some(home))
            .unwrap()
            .unwrap();
        let config = zed.config.unwrap();
        assert_eq!(config.file, None);
        assert!(config.open_with.is_some());
        assert!(config.snippet.contains("context_servers"));
    }

    /// Every agent that may have the old entry gets one line that says how to remove it: the
    /// agent's own remove command with the old name, or the entry and the file.
    #[test]
    fn the_old_entry_is_removed_with_the_agents_own_command_or_by_hand() {
        let of = |id: AgentId| {
            mcp_instructions(id, Path::new(BINARY), None)
                .unwrap()
                .unwrap()
                .old_entry
        };
        let claude = of(AgentId::ClaudeCode);
        assert_eq!(claude.server_name, "the-writer");
        assert_eq!(
            claude.command.as_deref(),
            Some("claude mcp remove the-writer")
        );
        assert_eq!(
            claude.line,
            "If you added the server when the app was called The Writer, remove that entry: \
             claude mcp remove the-writer"
        );
        assert_eq!(
            of(AgentId::GeminiCli).command.as_deref(),
            Some("gemini mcp remove --scope user the-writer")
        );
        assert_eq!(
            of(AgentId::Codex).command.as_deref(),
            Some("codex mcp remove the-writer")
        );

        // No remove command in the table: the entry is named, and the file.
        let cursor = of(AgentId::Cursor);
        assert_eq!(cursor.command, None);
        assert_eq!(
            cursor.line,
            "If you added the server when the app was called The Writer, delete its \
             \"the-writer\" entry from ~/.cursor/mcp.json."
        );
        let opencode = of(AgentId::OpenCode);
        assert_eq!(opencode.command, None, "OpenCode has no remove command");
        assert!(
            opencode.line.contains("~/.config/opencode/opencode.json"),
            "{}",
            opencode.line
        );
        let zed = of(AgentId::Zed);
        assert!(
            zed.line
                .contains("the file opened by: zed: open settings file"),
            "{}",
            zed.line
        );

        for instructions in all_mcp_instructions(Path::new(BINARY), None).unwrap() {
            let old = &instructions.old_entry;
            assert!(old.line.contains("\"the-writer\"") || old.line.ends_with(" the-writer"));
            assert!(!old.line.contains(MCP_SERVER_NAME), "{}", old.line);
            // Every remove command of the table names the server as one word, so that the
            // old name takes its place.
            if let Some(remove) = instructions
                .command
                .as_ref()
                .and_then(|c| c.remove.as_ref())
            {
                assert_eq!(
                    remove
                        .split(' ')
                        .filter(|word| *word == MCP_SERVER_NAME)
                        .count(),
                    1,
                    "{remove}"
                );
                assert_eq!(old.command.as_deref(), Some(with_old_name(remove).as_str()));
            }
            let json = serde_json::to_value(&instructions).unwrap();
            assert_eq!(json["old_entry"]["server_name"], "the-writer");
        }
    }

    #[test]
    fn an_agent_without_known_mcp_setup_has_no_instructions_and_the_rest_have_a_link() {
        assert_eq!(
            mcp_instructions(AgentId::Pi, Path::new(BINARY), None).unwrap(),
            None
        );
        let all = all_mcp_instructions(Path::new(BINARY), None).unwrap();
        assert!(
            all.iter()
                .all(|instructions| instructions.agent != AgentId::Pi)
        );
        for instructions in &all {
            assert!(instructions.command.is_some() || instructions.config.is_some());
            let url = instructions
                .docs_url()
                .unwrap_or_else(|| panic!("{} has no link", instructions.name));
            assert!(url.starts_with("https://"));
            assert_eq!(instructions.server_name, "inner-word");
            let json = serde_json::to_value(instructions).unwrap();
            assert_eq!(json["agent"], instructions.agent.key());
        }
    }
}
