//! What is known about the AI agents a writer may have on a Mac.
//!
//! One [`Agent`] per product: how to tell that it is installed, where it reads skills in the
//! Agent Skills format (a folder with a `SKILL.md`), and how a local stdio MCP server is
//! added to it. Every fact carries the [`Source`] it was taken from and the date it was
//! read, so the interface can link to it and a later reader can see how old it is.
//!
//! This field changes monthly. The table was researched on [`RESEARCHED_ON`] from each
//! product's own documentation (and, where the documentation was silent or the product is
//! installed on the development Mac, from its source code or by running its command against
//! a throwaway home directory). What could not be established is absent: an agent without
//! an MCP command has none here, and the interface shows only what is in the table.
//!
//! Templates use one placeholder, `{writer}`: the absolute path of the `innerword` binary. In
//! a command it is replaced by the path quoted for the shell; in a configuration snippet by
//! the path as a string literal of that format ([`crate::mcp`]).

use serde::Serialize;

/// The day the table was last checked against the documentation (ISO 8601).
pub const RESEARCHED_ON: &str = "2026-11-12";

/// The name the MCP server is registered under in every agent.
pub const MCP_SERVER_NAME: &str = "inner-word";

/// A product in the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AgentId {
    ClaudeCode,
    ClaudeDesktop,
    Codex,
    Cursor,
    VsCode,
    CopilotCli,
    GeminiCli,
    Antigravity,
    OpenCode,
    DevinDesktop,
    Zed,
    Pi,
}

impl AgentId {
    /// Every agent of the table, in its order.
    pub fn all() -> impl Iterator<Item = AgentId> {
        AGENTS.iter().map(|agent| agent.id)
    }

    /// The stable, lowercase name used on the command line and in JSON (`claude-code`).
    pub fn key(self) -> &'static str {
        match self {
            AgentId::ClaudeCode => "claude-code",
            AgentId::ClaudeDesktop => "claude-desktop",
            AgentId::Codex => "codex",
            AgentId::Cursor => "cursor",
            AgentId::VsCode => "vscode",
            AgentId::CopilotCli => "copilot-cli",
            AgentId::GeminiCli => "gemini-cli",
            AgentId::Antigravity => "antigravity",
            AgentId::OpenCode => "opencode",
            AgentId::DevinDesktop => "devin-desktop",
            AgentId::Zed => "zed",
            AgentId::Pi => "pi",
        }
    }

    /// The agent with that key. Case, spaces, underscores and hyphens do not matter, and a
    /// few other spellings people type are understood (`claude`, `vscode`, `copilot`,
    /// `opencode`, `gemini`, `windsurf`, `devin`, `chatgpt`).
    pub fn from_key(key: &str) -> Option<AgentId> {
        let plain: String = key
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .map(|c| c.to_ascii_lowercase())
            .collect();
        if let Some(agent) = AGENTS
            .iter()
            .find(|agent| agent.id.key().replace('-', "") == plain)
        {
            return Some(agent.id);
        }
        Some(match plain.as_str() {
            "claude" => AgentId::ClaudeCode,
            "claudeapp" | "claudedesktopapp" => AgentId::ClaudeDesktop,
            "chatgpt" | "codexcli" => AgentId::Codex,
            "code" | "visualstudiocode" | "githubcopilot" => AgentId::VsCode,
            "copilot" | "githubcopilotcli" => AgentId::CopilotCli,
            "gemini" => AgentId::GeminiCli,
            "agy" | "antigravitycli" => AgentId::Antigravity,
            "windsurf" | "devin" => AgentId::DevinDesktop,
            _ => return None,
        })
    }

    /// The table entry of this agent.
    pub fn agent(self) -> &'static Agent {
        AGENTS
            .iter()
            .find(|agent| agent.id == self)
            .expect("every AgentId has an entry in AGENTS")
    }
}

/// An id is its key in JSON (`"claude-code"`).
impl Serialize for AgentId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.key())
    }
}

impl std::fmt::Display for AgentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.agent().name)
    }
}

/// Where a fact comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SourceKind {
    /// The product's own documentation.
    Documentation,
    /// The product's published source code.
    SourceCode,
    /// The Homebrew cask of the product (the names of its application bundle and command).
    PackageRegistry,
    /// Seen by running the product's own command on the development Mac, against a
    /// throwaway home directory. `url` is empty; `title` says what was run.
    TriedLocally,
}

/// The origin of one fact: what was read, where, and when.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Source {
    pub kind: SourceKind,
    /// What the page is, or for [`SourceKind::TriedLocally`] what was run and seen.
    pub title: &'static str,
    /// The address to link to. Empty for [`SourceKind::TriedLocally`].
    pub url: &'static str,
    /// The day it was read (ISO 8601).
    pub read_on: &'static str,
}

impl Source {
    /// Whether there is a page to open.
    pub fn has_link(&self) -> bool {
        !self.url.is_empty()
    }
}

const fn doc(title: &'static str, url: &'static str) -> Source {
    Source {
        kind: SourceKind::Documentation,
        title,
        url,
        read_on: RESEARCHED_ON,
    }
}

const fn code(title: &'static str, url: &'static str) -> Source {
    Source {
        kind: SourceKind::SourceCode,
        title,
        url,
        read_on: RESEARCHED_ON,
    }
}

const fn cask(title: &'static str, url: &'static str) -> Source {
    Source {
        kind: SourceKind::PackageRegistry,
        title,
        url,
        read_on: RESEARCHED_ON,
    }
}

const fn tried(title: &'static str) -> Source {
    Source {
        kind: SourceKind::TriedLocally,
        title,
        url: "",
        read_on: RESEARCHED_ON,
    }
}

/// One way to tell that an agent is installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DetectRule {
    /// An application bundle of this name in an applications folder (`Claude.app`).
    App(&'static str),
    /// A command of this name in one of the folders commands are installed to (`claude`).
    Command(&'static str),
    /// A file or folder at this path below the home directory (`.claude`).
    HomePath(&'static str),
}

/// A [`DetectRule`] and where it comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Detection {
    pub rule: DetectRule,
    pub source: Source,
}

/// How firmly a skills directory belongs to an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DirStanding {
    /// The agent's documentation names it as a place it reads skills from.
    Documented,
    /// Still read, but called deprecated by the agent: the skill is found there and can be
    /// updated or removed there, and nothing new is installed into it unasked.
    Deprecated,
}

/// A user-level directory an agent reads skills from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SkillsDir {
    /// The directory, relative to the home directory (`.claude/skills`).
    pub home_relative: &'static str,
    pub standing: DirStanding,
    pub source: Source,
}

/// What is known about an agent and skills. `dirs` is in order of preference: the first
/// documented one is where the skill is installed for this agent when nothing else it reads
/// has it. Empty when no directory is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SkillSupport {
    pub dirs: &'static [SkillsDir],
    /// When the agent notices a newly installed skill, in one sentence. `None` when its
    /// documentation does not say.
    pub pickup: Option<&'static str>,
    /// Anything a person should know that the directories do not say, in one sentence.
    pub note: Option<&'static str>,
}

/// The command line that adds a local stdio MCP server to an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct McpCommand {
    /// The command, with `{writer}` where the quoted path of the binary goes.
    pub add: &'static str,
    /// The command that removes the server again, when the agent has one.
    pub remove: Option<&'static str>,
    /// A command that shows whether the server is registered, when the agent has one.
    pub check: Option<&'static str>,
    pub sources: &'static [Source],
}

/// The syntax of a configuration file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConfigFormat {
    Json,
    Toml,
}

/// The configuration file an MCP server entry belongs in, and the entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct McpConfig {
    /// The file, relative to the home directory. `None` when the documentation names no
    /// path and the file is reached through `open_with`.
    pub home_relative: Option<&'static str>,
    /// How the agent itself opens the file (a menu path or a palette command), when it
    /// documents one.
    pub open_with: Option<&'static str>,
    pub format: ConfigFormat,
    /// The entry as a complete, valid file of that format, with `{writer}` where the path
    /// of the binary goes as a string literal.
    pub snippet: &'static str,
    /// Where the entry goes in a file that already has content, in one sentence.
    pub placement: &'static str,
    pub sources: &'static [Source],
}

/// How a local stdio MCP server is added to an agent. Both fields are `None` when it could
/// not be established.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct McpSupport {
    pub command: Option<McpCommand>,
    pub config: Option<McpConfig>,
    /// One line a person needs beside the command: a restart, a scope, a caveat.
    pub note: Option<&'static str>,
}

impl McpSupport {
    /// Whether anything is known.
    pub fn is_known(&self) -> bool {
        self.command.is_some() || self.config.is_some()
    }
}

/// One product.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Agent {
    pub id: AgentId,
    /// The name to show ("Claude Code").
    pub name: &'static str,
    /// Who makes it ("Anthropic").
    pub vendor: &'static str,
    /// What the entry covers, in one sentence.
    pub about: &'static str,
    /// The product's documentation, for a "Documentation" link.
    pub docs_url: &'static str,
    /// Any one of these is enough to call the agent installed.
    pub detection: &'static [Detection],
    pub skills: SkillSupport,
    pub mcp: McpSupport,
}

impl Agent {
    /// The directory the skill is installed into for this agent when none of the
    /// directories it reads has it yet, relative to the home directory.
    pub fn preferred_skills_dir(&self) -> Option<&'static SkillsDir> {
        self.skills
            .dirs
            .iter()
            .find(|dir| dir.standing == DirStanding::Documented)
    }

    /// Every source behind this entry, each address once, in the order of the entry.
    pub fn sources(&self) -> Vec<Source> {
        let mut sources: Vec<Source> = Vec::new();
        let mut add = |source: &Source| {
            if !sources
                .iter()
                .any(|known| known.url == source.url && known.title == source.title)
            {
                sources.push(*source);
            }
        };
        for dir in self.skills.dirs {
            add(&dir.source);
        }
        if let Some(command) = &self.mcp.command {
            command.sources.iter().for_each(&mut add);
        }
        if let Some(config) = &self.mcp.config {
            config.sources.iter().for_each(&mut add);
        }
        for detection in self.detection {
            add(&detection.source);
        }
        sources
    }
}

/// The agent with that id.
pub fn agent(id: AgentId) -> &'static Agent {
    id.agent()
}

// ---------------------------------------------------------------------------------------
// Sources
// ---------------------------------------------------------------------------------------

const AGENT_SKILLS_SPEC: Source = doc(
    "Agent Skills: Specification",
    "https://agentskills.io/specification",
);

const CLAUDE_CODE_SKILLS: Source = doc(
    "Claude Code: Extend Claude with skills",
    "https://code.claude.com/docs/en/skills",
);
const CLAUDE_CODE_MCP: Source = doc(
    "Claude Code: Connect Claude Code to tools via MCP",
    "https://code.claude.com/docs/en/mcp",
);
const CLAUDE_CODE_TRIED: Source = tried(
    "claude 2.1.322: \"claude mcp add --scope user inner-word -- '<path with a space>' mcp\" stored the path as the command and \"mcp\" as its argument",
);
const CLAUDE_CODE_CASK: Source = cask(
    "Homebrew cask claude-code (command claude)",
    "https://formulae.brew.sh/cask/claude-code",
);
const CLAUDE_DESKTOP_CODE_TAB: Source = doc(
    "Claude Code: Use Claude Code Desktop",
    "https://code.claude.com/docs/en/desktop",
);
const CLAUDE_DESKTOP_MCP: Source = doc(
    "Model Context Protocol: Connect to local MCP servers",
    "https://modelcontextprotocol.io/docs/develop/connect-local-servers",
);
const CLAUDE_DESKTOP_CASK: Source = cask(
    "Homebrew cask claude (Claude.app)",
    "https://formulae.brew.sh/cask/claude",
);

const CODEX_SKILLS: Source = doc(
    "Codex: Build skills",
    "https://learn.chatgpt.com/docs/build-skills",
);
const CODEX_SKILLS_SOURCE: Source = code(
    "openai/codex: codex-rs/ext/skills/src/host_roots.rs (\"Deprecated user skills location ($CODEX_HOME/skills), kept for backward compatibility\")",
    "https://github.com/openai/codex/blob/main/codex-rs/ext/skills/src/host_roots.rs",
);
const CODEX_MCP: Source = doc(
    "Codex: Model Context Protocol",
    "https://learn.chatgpt.com/docs/extend/mcp?surface=cli",
);
const CODEX_TRIED: Source = tried(
    "codex-cli 0.167.0: \"codex mcp add inner-word -- '<path with a space>' mcp\" wrote [mcp_servers.inner-word] with that command and args = [\"mcp\"] to config.toml; \"codex mcp --help\" lists remove",
);
const CODEX_CASK: Source = cask(
    "Homebrew cask codex (command codex)",
    "https://formulae.brew.sh/cask/codex",
);
const CHATGPT_CASK: Source = cask(
    "Homebrew cask chatgpt (ChatGPT.app)",
    "https://formulae.brew.sh/cask/chatgpt",
);

const CURSOR_SKILLS: Source = doc(
    "Cursor: Agent Skills",
    "https://cursor.com/docs/context/skills",
);
const CURSOR_MCP: Source = doc(
    "Cursor: Model Context Protocol (MCP)",
    "https://cursor.com/docs/context/mcp",
);
const CURSOR_CLI_MCP: Source = doc("Cursor CLI: MCP", "https://cursor.com/docs/cli/mcp");
const CURSOR_CASK: Source = cask(
    "Homebrew cask cursor (Cursor.app)",
    "https://formulae.brew.sh/cask/cursor",
);
const CURSOR_CLI_CASK: Source = cask(
    "Homebrew cask cursor-cli (command cursor-agent)",
    "https://formulae.brew.sh/cask/cursor-cli",
);

const VS_CODE_SKILLS: Source = doc(
    "Visual Studio Code: Use Agent Skills",
    "https://code.visualstudio.com/docs/copilot/customization/agent-skills",
);
const VS_CODE_MCP: Source = doc(
    "Visual Studio Code: MCP configuration reference",
    "https://code.visualstudio.com/docs/agents/reference/mcp-configuration",
);
const VS_CODE_CASK: Source = cask(
    "Homebrew cask visual-studio-code (Visual Studio Code.app, command code)",
    "https://formulae.brew.sh/cask/visual-studio-code",
);

const COPILOT_SKILLS: Source = doc(
    "GitHub Docs: About agent skills",
    "https://docs.github.com/en/copilot/concepts/agents/about-agent-skills",
);
const COPILOT_CLI_MCP: Source = doc(
    "GitHub Docs: Adding MCP servers for GitHub Copilot CLI",
    "https://docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/add-mcp-servers",
);
const COPILOT_CLI_CASK: Source = cask(
    "Homebrew cask copilot-cli (command copilot)",
    "https://formulae.brew.sh/cask/copilot-cli",
);

const GEMINI_SKILLS: Source = doc(
    "Gemini CLI: Agent Skills",
    "https://geminicli.com/docs/cli/skills/",
);
const GEMINI_MCP: Source = doc(
    "Gemini CLI: MCP servers",
    "https://geminicli.com/docs/tools/mcp-server/",
);

const ANTIGRAVITY_SKILLS: Source = doc(
    "Google Antigravity: Skills",
    "https://antigravity.google/docs/skills",
);
const ANTIGRAVITY_MCP: Source = doc(
    "Google Antigravity: MCP",
    "https://antigravity.google/docs/mcp",
);
const ANTIGRAVITY_CASK: Source = cask(
    "Homebrew cask antigravity (Antigravity.app)",
    "https://formulae.brew.sh/cask/antigravity",
);
const ANTIGRAVITY_CLI_CASK: Source = cask(
    "Homebrew cask antigravity-cli (command agy)",
    "https://formulae.brew.sh/cask/antigravity-cli",
);

const OPENCODE_SKILLS: Source = doc("OpenCode: Agent Skills", "https://opencode.ai/docs/skills/");
const OPENCODE_MCP: Source = doc(
    "OpenCode: MCP servers",
    "https://opencode.ai/docs/mcp-servers/",
);
const OPENCODE_CONFIG: Source = doc("OpenCode: Config", "https://opencode.ai/docs/config/");
const OPENCODE_TRIED: Source = tried(
    "opencode 1.19.5: \"opencode mcp add inner-word -- '<path with a space>' mcp\" wrote the server to ~/.config/opencode/opencode.jsonc, inside a Git project too; it rejects --global; \"opencode mcp list\" lists a server written under mcp.servers; the command itself writes the server directly under \"mcp\" (\"mcp\": {\"inner-word\": {\"type\": \"local\", \"command\": [...]}}), and \"opencode mcp list\" then shows it as connected",
);
const OPENCODE_CASK: Source = cask(
    "Homebrew cask opencode-desktop (OpenCode.app)",
    "https://formulae.brew.sh/cask/opencode-desktop",
);

const DEVIN_DESKTOP_EXTEND: Source = doc(
    "Devin Desktop: rules, skills, plugins, MCP servers and hooks of the Devin Local agent",
    "https://docs.devin.ai/desktop/cascade/skills",
);
const DEVIN_MCP: Source = doc(
    "Devin CLI: MCP Configuration",
    "https://docs.devin.ai/cli/extensibility/mcp/configuration",
);
const DEVIN_RENAME: Source = doc(
    "Cognition: Windsurf is now Devin Desktop",
    "https://devin.ai/blog/windsurf-is-now-devin-desktop",
);
const DEVIN_CASK: Source = cask(
    "Homebrew cask devin-desktop (Devin.app)",
    "https://formulae.brew.sh/cask/devin-desktop",
);

const ZED_SKILLS: Source = doc("Zed: Skills", "https://zed.dev/docs/ai/skills");
const ZED_MCP: Source = doc("Zed: Model Context Protocol", "https://zed.dev/docs/ai/mcp");
const ZED_CASK: Source = cask(
    "Homebrew cask zed (Zed.app, command zed)",
    "https://formulae.brew.sh/cask/zed",
);

const PI_SKILLS: Source = doc(
    "Pi coding agent: Skills",
    "https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/docs/skills.md",
);
const PI_TRIED: Source =
    tried("pi 0.85.1 is installed on the development Mac as the command pi, with ~/.pi/agent");

/// The specification of the skill format every directory below holds.
pub const AGENT_SKILLS_SPECIFICATION: Source = AGENT_SKILLS_SPEC;

// ---------------------------------------------------------------------------------------
// Skills directories
// ---------------------------------------------------------------------------------------

const fn documented(home_relative: &'static str, source: Source) -> SkillsDir {
    SkillsDir {
        home_relative,
        standing: DirStanding::Documented,
        source,
    }
}

const fn deprecated(home_relative: &'static str, source: Source) -> SkillsDir {
    SkillsDir {
        home_relative,
        standing: DirStanding::Deprecated,
        source,
    }
}

/// The directory several agents share, relative to the home directory. It is real: Codex,
/// Cursor, GitHub Copilot, Gemini CLI, OpenCode, Devin Desktop, Zed and Pi each document it
/// as a place they read user-level skills from. Claude Code does not read it.
pub const SHARED_SKILLS_DIR: &str = ".agents/skills";

const CLAUDE_SKILLS_DIR: &str = ".claude/skills";

// ---------------------------------------------------------------------------------------
// Configuration snippets
// ---------------------------------------------------------------------------------------

const MCP_SERVERS_PLACEMENT: &str = "Add the \"inner-word\" entry to the \"mcpServers\" object of the file; keep what is already there.";

const SNIPPET_MCP_SERVERS: &str = r#"{
  "mcpServers": {
    "inner-word": {
      "command": {writer},
      "args": ["mcp"]
    }
  }
}
"#;

const SNIPPET_CURSOR: &str = r#"{
  "mcpServers": {
    "inner-word": {
      "type": "stdio",
      "command": {writer},
      "args": ["mcp"]
    }
  }
}
"#;

const SNIPPET_COPILOT_VS_CODE: &str = r#"{
  "mcpServers": {
    "inner-word": {
      "type": "stdio",
      "command": {writer},
      "args": ["mcp"],
      "tools": ["*"]
    }
  }
}
"#;

const SNIPPET_COPILOT_CLI: &str = r#"{
  "mcpServers": {
    "inner-word": {
      "type": "local",
      "command": {writer},
      "args": ["mcp"],
      "env": {},
      "tools": ["*"]
    }
  }
}
"#;

const SNIPPET_CODEX: &str = r#"[mcp_servers.inner-word]
command = {writer}
args = ["mcp"]
"#;

const SNIPPET_OPENCODE: &str = r#"{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "servers": {
      "inner-word": {
        "type": "local",
        "command": [{writer}, "mcp"]
      }
    }
  }
}
"#;

const SNIPPET_DEVIN: &str = r#"{
  "mcpServers": {
    "inner-word": {
      "command": {writer},
      "args": ["mcp"],
      "env": {}
    }
  }
}
"#;

const SNIPPET_ZED: &str = r#"{
  "context_servers": {
    "inner-word": {
      "command": {writer},
      "args": ["mcp"],
      "env": {}
    }
  }
}
"#;

// ---------------------------------------------------------------------------------------
// The table
// ---------------------------------------------------------------------------------------

/// Every agent, in the order the interface lists them.
pub static AGENTS: &[Agent] = &[
    Agent {
        id: AgentId::ClaudeCode,
        name: "Claude Code",
        vendor: "Anthropic",
        about: "Claude Code in the terminal.",
        docs_url: "https://code.claude.com/docs/en/overview",
        detection: &[
            Detection {
                rule: DetectRule::Command("claude"),
                source: CLAUDE_CODE_CASK,
            },
            Detection {
                rule: DetectRule::HomePath(".claude"),
                source: CLAUDE_CODE_SKILLS,
            },
        ],
        skills: SkillSupport {
            dirs: &[documented(CLAUDE_SKILLS_DIR, CLAUDE_CODE_SKILLS)],
            pickup: Some(
                "A running session picks the skill up without a restart; if ~/.claude/skills did not exist when the session started, run /reload-skills.",
            ),
            note: None,
        },
        mcp: McpSupport {
            command: Some(McpCommand {
                add: "claude mcp add --scope user inner-word -- {writer} mcp",
                remove: Some("claude mcp remove inner-word"),
                check: Some("claude mcp get inner-word"),
                sources: &[CLAUDE_CODE_MCP, CLAUDE_CODE_TRIED],
            }),
            config: None,
            note: Some(
                "--scope user makes the server available in every project; without it, it is added for the current project only.",
            ),
        },
    },
    Agent {
        id: AgentId::ClaudeDesktop,
        name: "Claude desktop app",
        vendor: "Anthropic",
        about: "The Claude app for macOS: chats, Cowork and the Code tab.",
        docs_url: "https://code.claude.com/docs/en/desktop",
        detection: &[
            Detection {
                rule: DetectRule::App("Claude.app"),
                source: CLAUDE_DESKTOP_CASK,
            },
            Detection {
                rule: DetectRule::HomePath("Library/Application Support/Claude"),
                source: CLAUDE_DESKTOP_MCP,
            },
        ],
        skills: SkillSupport {
            dirs: &[documented(CLAUDE_SKILLS_DIR, CLAUDE_DESKTOP_CODE_TAB)],
            pickup: None,
            note: Some(
                "Only local sessions of the Code tab read this folder. Chats and Cowork take their skills from the Customize settings of the Claude account, not from the disk: there, use the MCP server.",
            ),
        },
        mcp: McpSupport {
            command: None,
            config: Some(McpConfig {
                home_relative: Some(
                    "Library/Application Support/Claude/claude_desktop_config.json",
                ),
                open_with: Some("Claude > Settings > Developer > Edit Config"),
                format: ConfigFormat::Json,
                snippet: SNIPPET_MCP_SERVERS,
                placement: MCP_SERVERS_PLACEMENT,
                sources: &[CLAUDE_DESKTOP_MCP, CLAUDE_DESKTOP_CODE_TAB],
            }),
            note: Some(
                "Quit Claude completely and open it again. The server is then available in chats and in local sessions of the Code tab.",
            ),
        },
    },
    Agent {
        id: AgentId::Codex,
        name: "Codex",
        vendor: "OpenAI",
        about: "Codex CLI, the ChatGPT desktop app and the Codex IDE extension, which share one configuration.",
        docs_url: "https://learn.chatgpt.com/docs/build-skills",
        detection: &[
            Detection {
                rule: DetectRule::Command("codex"),
                source: CODEX_CASK,
            },
            Detection {
                rule: DetectRule::HomePath(".codex"),
                source: CODEX_MCP,
            },
            Detection {
                rule: DetectRule::App("ChatGPT.app"),
                source: CHATGPT_CASK,
            },
        ],
        skills: SkillSupport {
            dirs: &[
                documented(SHARED_SKILLS_DIR, CODEX_SKILLS),
                deprecated(".codex/skills", CODEX_SKILLS_SOURCE),
            ],
            pickup: Some(
                "Codex detects skill changes automatically; if the skill does not appear, restart Codex.",
            ),
            note: None,
        },
        mcp: McpSupport {
            command: Some(McpCommand {
                add: "codex mcp add inner-word -- {writer} mcp",
                remove: Some("codex mcp remove inner-word"),
                check: Some("codex mcp list"),
                sources: &[CODEX_MCP, CODEX_TRIED],
            }),
            config: Some(McpConfig {
                home_relative: Some(".codex/config.toml"),
                open_with: None,
                format: ConfigFormat::Toml,
                snippet: SNIPPET_CODEX,
                placement: "Add the table to the file; keep what is already there.",
                sources: &[CODEX_MCP],
            }),
            note: Some(
                "The Codex CLI, the ChatGPT desktop app and the Codex IDE extension share this configuration.",
            ),
        },
    },
    Agent {
        id: AgentId::Cursor,
        name: "Cursor",
        vendor: "Anysphere",
        about: "The Cursor editor and the Cursor CLI, which use the same configuration.",
        docs_url: "https://cursor.com/docs",
        detection: &[
            Detection {
                rule: DetectRule::App("Cursor.app"),
                source: CURSOR_CASK,
            },
            Detection {
                rule: DetectRule::Command("cursor-agent"),
                source: CURSOR_CLI_CASK,
            },
            Detection {
                rule: DetectRule::HomePath(".cursor"),
                source: CURSOR_MCP,
            },
        ],
        skills: SkillSupport {
            dirs: &[
                documented(SHARED_SKILLS_DIR, CURSOR_SKILLS),
                documented(".cursor/skills", CURSOR_SKILLS),
                documented(CLAUDE_SKILLS_DIR, CURSOR_SKILLS),
                documented(".codex/skills", CURSOR_SKILLS),
            ],
            pickup: Some("Cursor discovers skills when it starts."),
            note: None,
        },
        mcp: McpSupport {
            command: None,
            config: Some(McpConfig {
                home_relative: Some(".cursor/mcp.json"),
                open_with: None,
                format: ConfigFormat::Json,
                snippet: SNIPPET_CURSOR,
                placement: MCP_SERVERS_PLACEMENT,
                sources: &[CURSOR_MCP, CURSOR_CLI_MCP],
            }),
            note: Some(
                "Cursor has no command for adding a server; the Cursor CLI reads the same file.",
            ),
        },
    },
    Agent {
        id: AgentId::VsCode,
        name: "VS Code with GitHub Copilot",
        vendor: "Microsoft",
        about: "Copilot's agent in Visual Studio Code.",
        docs_url: "https://code.visualstudio.com/docs/copilot/customization/agent-skills",
        detection: &[
            Detection {
                rule: DetectRule::App("Visual Studio Code.app"),
                source: VS_CODE_CASK,
            },
            Detection {
                rule: DetectRule::Command("code"),
                source: VS_CODE_CASK,
            },
        ],
        skills: SkillSupport {
            dirs: &[documented(".copilot/skills", VS_CODE_SKILLS)],
            pickup: None,
            note: Some(
                "Claude and Codex sessions inside VS Code read ~/.claude/skills and ~/.agents/skills instead, as the deprecated Local sessions also do.",
            ),
        },
        mcp: McpSupport {
            command: None,
            config: Some(McpConfig {
                home_relative: Some(".copilot/mcp-config.json"),
                open_with: Some("MCP: Add Server, with the destination Copilot Global"),
                format: ConfigFormat::Json,
                snippet: SNIPPET_COPILOT_VS_CODE,
                placement: MCP_SERVERS_PLACEMENT,
                sources: &[VS_CODE_MCP],
            }),
            note: Some(
                "\"code --add-mcp\" writes the editor's older configuration, which Copilot's agent does not read directly: use this file.",
            ),
        },
    },
    Agent {
        id: AgentId::CopilotCli,
        name: "GitHub Copilot CLI",
        vendor: "GitHub",
        about: "Copilot in the terminal.",
        docs_url: "https://docs.github.com/en/copilot/concepts/agents/about-copilot-cli",
        detection: &[Detection {
            rule: DetectRule::Command("copilot"),
            source: COPILOT_CLI_CASK,
        }],
        skills: SkillSupport {
            dirs: &[
                documented(SHARED_SKILLS_DIR, COPILOT_SKILLS),
                documented(".copilot/skills", COPILOT_SKILLS),
            ],
            pickup: None,
            note: None,
        },
        mcp: McpSupport {
            command: Some(McpCommand {
                add: "copilot mcp add inner-word -- {writer} mcp",
                remove: Some("copilot mcp remove inner-word"),
                check: Some("copilot mcp list"),
                sources: &[COPILOT_CLI_MCP],
            }),
            config: Some(McpConfig {
                home_relative: Some(".copilot/mcp-config.json"),
                open_with: None,
                format: ConfigFormat::Json,
                snippet: SNIPPET_COPILOT_CLI,
                placement: MCP_SERVERS_PLACEMENT,
                sources: &[COPILOT_CLI_MCP],
            }),
            note: Some("Copilot's agent in VS Code reads the same file."),
        },
    },
    Agent {
        id: AgentId::GeminiCli,
        name: "Gemini CLI",
        vendor: "Google",
        about: "Gemini in the terminal. Its documentation says Antigravity CLI replaced it on 18 June 2026.",
        docs_url: "https://geminicli.com/docs/",
        detection: &[
            Detection {
                rule: DetectRule::Command("gemini"),
                source: GEMINI_MCP,
            },
            Detection {
                rule: DetectRule::HomePath(".gemini/settings.json"),
                source: GEMINI_MCP,
            },
        ],
        skills: SkillSupport {
            dirs: &[
                documented(SHARED_SKILLS_DIR, GEMINI_SKILLS),
                documented(".gemini/skills", GEMINI_SKILLS),
            ],
            pickup: Some("In a running session, /skills reload refreshes the list."),
            note: None,
        },
        mcp: McpSupport {
            command: Some(McpCommand {
                add: "gemini mcp add --scope user inner-word {writer} mcp",
                remove: Some("gemini mcp remove --scope user inner-word"),
                check: Some("gemini mcp list"),
                sources: &[GEMINI_MCP],
            }),
            config: Some(McpConfig {
                home_relative: Some(".gemini/settings.json"),
                open_with: None,
                format: ConfigFormat::Json,
                snippet: SNIPPET_MCP_SERVERS,
                placement: MCP_SERVERS_PLACEMENT,
                sources: &[GEMINI_MCP],
            }),
            note: Some(
                "--scope user adds the server for every project; the default is the current project.",
            ),
        },
    },
    Agent {
        id: AgentId::Antigravity,
        name: "Antigravity",
        vendor: "Google",
        about: "The Antigravity app and Antigravity CLI (agy), which share one configuration.",
        docs_url: "https://antigravity.google/docs/skills",
        detection: &[
            Detection {
                rule: DetectRule::App("Antigravity.app"),
                source: ANTIGRAVITY_CASK,
            },
            Detection {
                rule: DetectRule::Command("agy"),
                source: ANTIGRAVITY_CLI_CASK,
            },
            Detection {
                rule: DetectRule::HomePath(".gemini/config"),
                source: ANTIGRAVITY_SKILLS,
            },
            Detection {
                rule: DetectRule::HomePath(".gemini/antigravity-cli"),
                source: ANTIGRAVITY_SKILLS,
            },
        ],
        skills: SkillSupport {
            dirs: &[documented(".gemini/config/skills", ANTIGRAVITY_SKILLS)],
            pickup: None,
            note: Some(
                "Antigravity CLI alone also reads ~/.gemini/antigravity-cli/skills and ~/.gemini/skills.",
            ),
        },
        mcp: McpSupport {
            command: None,
            config: Some(McpConfig {
                home_relative: Some(".gemini/config/mcp_config.json"),
                open_with: Some(
                    "Customizations > MCP Servers, in the Antigravity app (Copy Config File Path shows where the file is)",
                ),
                format: ConfigFormat::Json,
                snippet: SNIPPET_MCP_SERVERS,
                placement: MCP_SERVERS_PLACEMENT,
                sources: &[ANTIGRAVITY_MCP],
            }),
            note: Some(
                "In Antigravity CLI, /mcp opens the MCP manager, where the configuration can be reloaded.",
            ),
        },
    },
    Agent {
        id: AgentId::OpenCode,
        name: "OpenCode",
        vendor: "Anomaly",
        about: "OpenCode in the terminal and the OpenCode desktop app.",
        docs_url: "https://opencode.ai/docs/",
        detection: &[
            Detection {
                rule: DetectRule::Command("opencode"),
                source: OPENCODE_MCP,
            },
            Detection {
                rule: DetectRule::App("OpenCode.app"),
                source: OPENCODE_CASK,
            },
            Detection {
                rule: DetectRule::HomePath(".config/opencode"),
                source: OPENCODE_CONFIG,
            },
        ],
        skills: SkillSupport {
            dirs: &[
                documented(SHARED_SKILLS_DIR, OPENCODE_SKILLS),
                documented(".config/opencode/skills", OPENCODE_SKILLS),
                documented(CLAUDE_SKILLS_DIR, OPENCODE_SKILLS),
            ],
            pickup: None,
            note: None,
        },
        mcp: McpSupport {
            command: Some(McpCommand {
                add: "opencode mcp add inner-word -- {writer} mcp",
                remove: None,
                check: Some("opencode mcp list"),
                sources: &[OPENCODE_MCP, OPENCODE_TRIED],
            }),
            config: Some(McpConfig {
                home_relative: Some(".config/opencode/opencode.json"),
                open_with: None,
                format: ConfigFormat::Json,
                snippet: SNIPPET_OPENCODE,
                placement: "Add the \"inner-word\" entry to the \"servers\" object under \"mcp\"; keep what is already there. That is the form of OpenCode 2. OpenCode 1 keeps its servers directly under \"mcp\", without \"servers\": there, run the command, which writes the entry in the form its version reads.",
                sources: &[OPENCODE_MCP, OPENCODE_CONFIG, OPENCODE_TRIED],
            }),
            note: Some(
                "The command writes the entry in the form the installed version reads. OpenCode 1.19.5 writes it to the global configuration (opencode.jsonc when that file exists). If it turns up in one project only, repeat the command with --global after the name, as the documentation of newer versions describes.",
            ),
        },
    },
    Agent {
        id: AgentId::DevinDesktop,
        name: "Devin Desktop (formerly Windsurf)",
        vendor: "Cognition",
        about: "The Devin Local agent of Devin Desktop, the editor that was called Windsurf until June 2026, and Devin CLI.",
        docs_url: "https://docs.devin.ai/desktop/cascade/skills",
        detection: &[
            Detection {
                rule: DetectRule::App("Devin.app"),
                source: DEVIN_CASK,
            },
            Detection {
                rule: DetectRule::Command("devin"),
                source: DEVIN_MCP,
            },
            Detection {
                rule: DetectRule::HomePath(".config/devin"),
                source: DEVIN_DESKTOP_EXTEND,
            },
        ],
        skills: SkillSupport {
            dirs: &[
                documented(SHARED_SKILLS_DIR, DEVIN_DESKTOP_EXTEND),
                documented(".config/devin/skills", DEVIN_DESKTOP_EXTEND),
            ],
            pickup: None,
            note: None,
        },
        mcp: McpSupport {
            command: Some(McpCommand {
                add: "devin mcp add -s user inner-word -- {writer} mcp",
                remove: Some("devin mcp remove inner-word"),
                check: Some("devin mcp list"),
                sources: &[DEVIN_MCP],
            }),
            config: Some(McpConfig {
                home_relative: Some(".config/devin/mcp_config.json"),
                open_with: Some("Customize > MCPs > This computer, in Devin Desktop"),
                format: ConfigFormat::Json,
                snippet: SNIPPET_DEVIN,
                placement: MCP_SERVERS_PLACEMENT,
                sources: &[DEVIN_DESKTOP_EXTEND, DEVIN_MCP, DEVIN_RENAME],
            }),
            note: Some("The command belongs to Devin CLI. Without it, add the entry to the file."),
        },
    },
    Agent {
        id: AgentId::Zed,
        name: "Zed",
        vendor: "Zed Industries",
        about: "The Zed Agent in the Zed editor. Agents that Zed runs from outside (external agents) use their own setup.",
        docs_url: "https://zed.dev/docs/ai/skills",
        detection: &[
            Detection {
                rule: DetectRule::App("Zed.app"),
                source: ZED_CASK,
            },
            Detection {
                rule: DetectRule::Command("zed"),
                source: ZED_CASK,
            },
        ],
        skills: SkillSupport {
            dirs: &[documented(SHARED_SKILLS_DIR, ZED_SKILLS)],
            pickup: Some("The skill takes effect immediately, without restarting the session."),
            note: None,
        },
        mcp: McpSupport {
            command: None,
            config: Some(McpConfig {
                home_relative: None,
                open_with: Some("zed: open settings file, in the command palette"),
                format: ConfigFormat::Json,
                snippet: SNIPPET_ZED,
                placement: "Add the \"inner-word\" entry to the \"context_servers\" object of the settings; keep what is already there.",
                sources: &[ZED_MCP],
            }),
            note: Some(
                "Or without the file: Settings > AI > MCP Servers > Add Server > Add Local Server.",
            ),
        },
    },
    Agent {
        id: AgentId::Pi,
        name: "Pi",
        vendor: "Mario Zechner",
        about: "The Pi coding agent in the terminal.",
        docs_url: "https://github.com/badlogic/pi-mono/tree/main/packages/coding-agent",
        detection: &[
            Detection {
                rule: DetectRule::Command("pi"),
                source: PI_TRIED,
            },
            Detection {
                rule: DetectRule::HomePath(".pi/agent"),
                source: PI_TRIED,
            },
        ],
        skills: SkillSupport {
            dirs: &[documented(SHARED_SKILLS_DIR, PI_SKILLS)],
            pickup: Some("Pi scans its skill locations when it starts."),
            note: None,
        },
        mcp: McpSupport {
            command: None,
            config: None,
            note: None,
        },
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_id_has_exactly_one_entry_and_a_key_that_reads_back() {
        let mut seen = HashSet::new();
        for agent in AGENTS {
            assert!(
                seen.insert(agent.id),
                "{} is in the table twice",
                agent.name
            );
            assert_eq!(AgentId::from_key(agent.id.key()), Some(agent.id));
            assert_eq!(agent.id.agent().name, agent.name);
            assert_eq!(agent.id.to_string(), agent.name);
            let key = agent.id.key();
            assert!(
                key.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
                "{key}"
            );
        }
        assert_eq!(seen.len(), 12);
    }

    #[test]
    fn keys_are_forgiving() {
        assert_eq!(AgentId::from_key("Claude Code"), Some(AgentId::ClaudeCode));
        assert_eq!(AgentId::from_key("claude_code"), Some(AgentId::ClaudeCode));
        assert_eq!(AgentId::from_key("claude"), Some(AgentId::ClaudeCode));
        assert_eq!(AgentId::from_key("vs-code"), Some(AgentId::VsCode));
        assert_eq!(AgentId::from_key("code"), Some(AgentId::VsCode));
        assert_eq!(AgentId::from_key("open-code"), Some(AgentId::OpenCode));
        assert_eq!(
            serde_json::to_string(&AgentId::OpenCode).unwrap(),
            "\"opencode\""
        );
        assert_eq!(AgentId::from_key("windsurf"), Some(AgentId::DevinDesktop));
        assert_eq!(AgentId::from_key("emacs"), None);
        assert_eq!(AgentId::from_key(""), None);
    }

    /// Nothing in the table is shown to the user without a source and a date.
    #[test]
    fn every_fact_has_a_source_with_a_date_and_documentation_has_a_link() {
        let check = |agent: &Agent, source: &Source| {
            assert_eq!(source.read_on.len(), 10, "{}: {:?}", agent.name, source);
            assert!(!source.title.is_empty(), "{}", agent.name);
            match source.kind {
                SourceKind::TriedLocally => assert!(source.url.is_empty()),
                _ => assert!(
                    source.url.starts_with("https://"),
                    "{}: {:?}",
                    agent.name,
                    source
                ),
            }
        };
        for agent in AGENTS {
            assert!(agent.docs_url.starts_with("https://"));
            assert!(
                !agent.detection.is_empty(),
                "{} cannot be detected",
                agent.name
            );
            for detection in agent.detection {
                check(agent, &detection.source);
            }
            for dir in agent.skills.dirs {
                check(agent, &dir.source);
            }
            if let Some(command) = &agent.mcp.command {
                assert!(!command.sources.is_empty(), "{}", agent.name);
                command
                    .sources
                    .iter()
                    .for_each(|source| check(agent, source));
            }
            if let Some(config) = &agent.mcp.config {
                assert!(!config.sources.is_empty(), "{}", agent.name);
                config
                    .sources
                    .iter()
                    .for_each(|source| check(agent, source));
                assert!(
                    config.home_relative.is_some() || config.open_with.is_some(),
                    "{}: a configuration nobody can find",
                    agent.name
                );
            }
            assert!(!agent.sources().is_empty());
        }
    }

    #[test]
    fn paths_are_relative_to_the_home_directory() {
        for agent in AGENTS {
            for dir in agent.skills.dirs {
                assert!(!dir.home_relative.starts_with('/') && !dir.home_relative.starts_with('~'));
                assert!(
                    dir.home_relative.ends_with("skills"),
                    "{}",
                    dir.home_relative
                );
            }
            if let Some(path) = agent.mcp.config.and_then(|config| config.home_relative) {
                assert!(!path.starts_with('/') && !path.starts_with('~'));
            }
            for detection in agent.detection {
                match detection.rule {
                    DetectRule::App(name) => assert!(name.ends_with(".app")),
                    DetectRule::Command(name) => assert!(!name.contains('/')),
                    DetectRule::HomePath(path) => {
                        assert!(!path.starts_with('/') && !path.starts_with('~'))
                    }
                }
            }
        }
    }

    #[test]
    fn templates_name_the_server_and_have_the_placeholder_exactly_once() {
        for agent in AGENTS {
            if let Some(command) = &agent.mcp.command {
                assert_eq!(command.add.matches("{writer}").count(), 1, "{}", agent.name);
                assert!(command.add.contains(MCP_SERVER_NAME));
                assert!(command.add.ends_with("{writer} mcp"), "{}", command.add);
                for other in [command.remove, command.check].into_iter().flatten() {
                    assert!(!other.contains("{writer}"));
                }
            }
            if let Some(config) = &agent.mcp.config {
                assert_eq!(
                    config.snippet.matches("{writer}").count(),
                    1,
                    "{}",
                    agent.name
                );
                assert!(config.snippet.contains(MCP_SERVER_NAME));
                assert!(config.snippet.contains("\"mcp\""));
            }
        }
    }

    #[test]
    fn the_agents_the_brief_names_are_all_there() {
        for id in [
            AgentId::ClaudeCode,
            AgentId::ClaudeDesktop,
            AgentId::Codex,
            AgentId::Cursor,
            AgentId::VsCode,
            AgentId::GeminiCli,
            AgentId::OpenCode,
            AgentId::DevinDesktop,
            AgentId::Zed,
        ] {
            let agent = id.agent();
            assert!(agent.mcp.is_known(), "{}", agent.name);
            assert!(agent.preferred_skills_dir().is_some(), "{}", agent.name);
        }
        assert!(!AgentId::Pi.agent().mcp.is_known());
        assert_eq!(
            AgentId::Codex
                .agent()
                .preferred_skills_dir()
                .unwrap()
                .home_relative,
            SHARED_SKILLS_DIR
        );
        assert_eq!(
            AgentId::ClaudeCode
                .agent()
                .preferred_skills_dir()
                .unwrap()
                .home_relative,
            ".claude/skills"
        );
    }
}
