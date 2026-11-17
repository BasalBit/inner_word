//! What Inner Word knows about the AI agents on a Mac, and what it does for them.
//!
//! The `innerword` command line tool (`Inner Word.app/Contents/MacOS/innerword`) lets agents work
//! with the library. An agent has to be told that the tool exists. There are two ways, and
//! this crate is the knowledge and the mechanics behind both; the tool exposes it as
//! subcommands (`innerword agents ...`) and the app as its Agents panel.
//!
//! - **The skill** ([`skill`]): a folder `inner-word` with a `SKILL.md` in the Agent Skills
//!   format, which says what the tool is, where it is and how to use it well. It is
//!   installed into the directory an agent reads skills from ([`install`] for one
//!   directory, [`detect`] for an agent or for all detected ones).
//! - **The MCP server** ([`mcp`]): `innerword mcp` over standard input and output. For each
//!   agent this crate renders the command to run or the configuration entry to paste, with
//!   the real path of the binary. Nothing in this crate edits an agent's configuration.
//!
//! What is known about each agent is a table ([`agents::AGENTS`]): how it is detected, where
//! its skills live, how an MCP server is added to it. Every fact in it names the page it
//! was read on and the date ([`agents::Source`]); what could not be established is left out.
//!
//! # In short
//!
//! ```no_run
//! use innerword_agent_setup::{
//!     all_mcp_instructions, install_for_detected, survey, Machine, SkillSpec, DEFAULT_BINARY_PATH,
//! };
//!
//! # fn main() -> Result<(), innerword_agent_setup::SetupError> {
//! let machine = Machine::this_mac()?;
//! let spec = SkillSpec::new(DEFAULT_BINARY_PATH)?;
//!
//! // Which agents are here, and does each have the skill?
//! for status in survey(&machine, &spec) {
//!     if let (true, Some(skill)) = (status.detected, &status.skill) {
//!         println!("{}: {} in {}", status.name, skill.state.label(), skill.dir_display);
//!     }
//! }
//!
//! // Install it for all of them, and say exactly what was written.
//! for outcome in install_for_detected(&machine, &spec).outcomes {
//!     println!("{}", outcome.line());
//! }
//!
//! // The commands that add the MCP server.
//! for instructions in all_mcp_instructions(spec.binary(), Some(machine.home()))? {
//!     if let Some(command) = instructions.command {
//!         println!("{}: {}", instructions.name, command.add);
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! # Safety
//!
//! A skills directory belongs to the user and to other tools. This crate writes exactly one
//! file in it, `inner-word/SKILL.md`, never overwrites or removes a folder of that name it
//! did not write ([`SetupError::Foreign`]), and reports every path it created, wrote or
//! removed ([`Report`]). The one other folder it touches is its own old install,
//! `the-writer` with the old generator in its `SKILL.md` ([`legacy`]), which installing and
//! removing take away; a `the-writer` folder without that marker is left alone. Tests hand
//! it a temporary home ([`Machine::isolated`]).

pub mod agents;
pub mod detect;
pub mod error;
pub mod install;
pub mod legacy;
pub mod mcp;
pub mod shell;
pub mod skill;

pub use agents::{
    AGENT_SKILLS_SPECIFICATION, AGENTS, Agent, AgentId, ConfigFormat, DetectRule, Detection,
    DirStanding, MCP_SERVER_NAME, McpCommand, McpConfig, McpSupport, RESEARCHED_ON,
    SHARED_SKILLS_DIR, SkillSupport, SkillsDir, Source, SourceKind, agent,
};
pub use detect::{
    AgentSkill, AgentStatus, Batch, Evidence, Machine, Outcome, SkillCopy, SkillLocation,
    agent_status, all_dirs, detected_agents, evidence, install_for_agent, install_for_detected,
    is_detected, remove_everywhere, remove_for_agent, skill_locations, survey, update_for_agent,
    update_installed,
};
pub use error::SetupError;
pub use install::{
    Change, Report, SkillState, StaleReason, StaleSkill, install_skill, installed_marker,
    legacy_marker, legacy_skill_dir, remove_skill, skill_dir, skill_file, skill_state,
};
pub use legacy::{LEGACY_APP_NAME, LEGACY_GENERATOR, LEGACY_MCP_SERVER_NAME, LEGACY_SKILL_NAME};
pub use mcp::{
    McpCommandLines, McpConfigEntry, McpInstructions, McpOldEntry, all_mcp_instructions,
    mcp_instructions,
};
pub use skill::{
    APP_VERSION, DEFAULT_BINARY_PATH, DESCRIPTION, GENERATOR, SKILL_FILE, SKILL_NAME, SkillMarker,
    SkillSpec, binary_beside, example_lines, mentioned_commands, read_legacy_marker, read_marker,
};
