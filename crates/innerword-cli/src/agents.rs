//! `innerword agents ...`: which agents are on this Mac, the skill that tells them about this
//! tool, and how the MCP server is added to each.
//!
//! Everything here is `innerword-agent-setup`; this module chooses the function for a command
//! line and prints what it returns. The skill and the MCP entries name the binary that is
//! running (in the app bundle: `Inner Word.app/Contents/MacOS/innerword`), so what is
//! installed always points at the tool that installed it.

use std::path::PathBuf;

use serde_json::{Value, json};
use innerword_agent_setup::{
    AgentId, AgentStatus, Batch, Machine, McpInstructions, SkillSpec, SkillState, agent_status,
    all_mcp_instructions, install_for_agent, install_for_detected, mcp_instructions,
    remove_everywhere, remove_for_agent, skill_locations, survey, update_for_agent,
    update_installed,
};
use innerword_mcp::{ErrorKind, Failure, Reply};

use crate::parse::{Globals, Parsed};
use crate::spec::Spec;

fn failed(error: impl std::fmt::Display) -> Failure {
    Failure::new(ErrorKind::Failed, error.to_string())
}

/// The binary the skill and the MCP entries name: this one.
fn binary() -> Result<PathBuf, Failure> {
    std::env::current_exe()
        .map_err(|error| failed(format!("the path of this program is not known: {error}")))
}

fn agent_id(key: &str) -> Result<AgentId, Failure> {
    AgentId::from_key(key).ok_or_else(|| {
        let known: Vec<&str> = AgentId::all().map(AgentId::key).collect();
        Failure::new(
            ErrorKind::NotFound,
            format!(
                "there is no agent \"{key}\". The agents this tool knows: {}",
                known.join(", ")
            ),
        )
    })
}

/// The agent a command names, or `None` for `--all`.
fn target(spec: &Spec, parsed: &Parsed) -> Result<Option<AgentId>, Failure> {
    match (parsed.arg(0), parsed.flag("all")) {
        (Some(key), false) => agent_id(key).map(Some),
        (None, true) => Ok(None),
        (given, _) => Err(Failure::invalid(format!(
            "name one agent or give --all, not {}\nusage: {}\n\"innerword agents\" lists the agents.",
            if given.is_some() { "both" } else { "neither" },
            crate::spec::usage(spec)
        ))),
    }
}

/// Runs an `agents` command.
pub fn run(spec: &Spec, parsed: &Parsed, globals: &Globals) -> Result<Reply, Failure> {
    let machine = Machine::this_mac().map_err(failed)?;
    let skill = SkillSpec::new(binary()?).map_err(failed)?;
    match spec.name {
        "agents list" => Ok(list(&machine, &skill)),
        "agents skill" => {
            let content = skill.render();
            let data = json!({
                "name": innerword_agent_setup::SKILL_NAME,
                "file": innerword_agent_setup::SKILL_FILE,
                "version": skill.version(),
                "binary": skill.binary(),
                "content": content,
            });
            Ok(Reply {
                text: content,
                data,
            })
        }
        "agents mcp" => {
            let instructions = match parsed.arg(0) {
                Some(key) => {
                    let id = agent_id(key)?;
                    match mcp_instructions(id, skill.binary(), Some(machine.home()))
                        .map_err(failed)?
                    {
                        Some(instructions) => vec![instructions],
                        None => {
                            return Err(Failure::new(
                                ErrorKind::NotFound,
                                format!(
                                    "it is not known how an MCP server is added to {}",
                                    id.agent().name
                                ),
                            ));
                        }
                    }
                }
                None => {
                    all_mcp_instructions(skill.binary(), Some(machine.home())).map_err(failed)?
                }
            };
            Ok(mcp(&skill, &instructions))
        }
        "agents install" | "agents update" | "agents remove" => {
            let agent = target(spec, parsed)?;
            if globals.dry_run {
                return Ok(dry_run(spec.name, agent, &machine, &skill));
            }
            let batch = match (spec.name, agent) {
                ("agents install", Some(agent)) => {
                    install_for_agent(&machine, agent, &skill).map_err(failed)?
                }
                ("agents install", None) => install_for_detected(&machine, &skill),
                ("agents update", Some(agent)) => update_for_agent(&machine, agent, &skill),
                ("agents update", None) => update_installed(&machine, &skill),
                (_, Some(agent)) => remove_for_agent(&machine, agent),
                (_, None) => remove_everywhere(&machine),
            };
            finish(spec.name, agent, batch)
        }
        other => Err(failed(format!(
            "\"innerword {other}\" is not an agents command"
        ))),
    }
}

fn skill_line(status: &AgentStatus) -> String {
    match &status.skill {
        None => "no skills folder is known for it".to_string(),
        Some(skill) => {
            let mut line = format!("skill {} in {}", skill.state.label(), skill.dir_display);
            if let SkillState::Stale(stale) = &skill.state {
                line.push_str(&format!(" ({})", stale.explanation()));
            }
            if !skill.shared_with.is_empty() {
                let names: Vec<&str> = skill.shared_with.iter().map(|id| id.agent().name).collect();
                line.push_str(&format!("; shared with {}", names.join(", ")));
            }
            line
        }
    }
}

fn list(machine: &Machine, skill: &SkillSpec) -> Reply {
    let statuses = survey(machine, skill);
    let width = statuses
        .iter()
        .map(|status| status.id.key().len())
        .max()
        .unwrap_or(0);
    let mut text = String::new();
    let (found, missing): (Vec<&AgentStatus>, Vec<&AgentStatus>) =
        statuses.iter().partition(|status| status.detected);
    if found.is_empty() {
        text.push_str("No agent this tool knows was found on this Mac.\n");
    } else {
        text.push_str("Found on this Mac:\n");
        for status in &found {
            text.push_str(&format!(
                "  {:width$}  {}: {}\n",
                status.id.key(),
                status.name,
                skill_line(status),
                width = width
            ));
        }
    }
    if !missing.is_empty() {
        text.push_str("Not found:\n");
        for status in &missing {
            let installed = status
                .skill
                .as_ref()
                .is_some_and(|skill| skill.state.is_installed());
            let note = if installed {
                format!(": {}", skill_line(status))
            } else {
                String::new()
            };
            text.push_str(&format!(
                "  {:width$}  {}{note}\n",
                status.id.key(),
                status.name,
                width = width
            ));
        }
    }
    text.push_str(&format!(
        "The skill names this tool: {} (version {}).\n\
\"innerword agents install <agent>\" or \"--all\" installs it; \"innerword agents mcp\" shows how to add the MCP server.\n",
        skill.binary().display(),
        skill.version()
    ));
    let data = json!({
        "binary": skill.binary(),
        "version": skill.version(),
        "home": machine.home(),
        "agents": statuses,
    });
    Reply { text, data }
}

fn mcp(skill: &SkillSpec, instructions: &[McpInstructions]) -> Reply {
    let mut text = String::new();
    for one in instructions {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&format!("{} ({})\n", one.name, one.agent.key()));
        if let Some(command) = &one.command {
            text.push_str(&format!("  Run:     {}\n", command.add));
            if let Some(check) = &command.check {
                text.push_str(&format!("  Check:   {check}\n"));
            }
            if let Some(remove) = &command.remove {
                text.push_str(&format!("  Remove:  {remove}\n"));
            }
        }
        if let Some(config) = &one.config {
            let file = config
                .file_display
                .as_deref()
                .unwrap_or("its configuration file");
            let lead = if one.command.is_some() {
                "Or add to"
            } else {
                "Add to"
            };
            text.push_str(&format!("  {lead} {file}"));
            if let Some(open_with) = config.open_with {
                text.push_str(&format!(" (open it with: {open_with})"));
            }
            text.push_str(":\n");
            for line in config.snippet.lines() {
                text.push_str(&format!("    {line}\n"));
            }
            text.push_str(&format!("  Where:   {}\n", config.placement));
        }
        if let Some(note) = one.note {
            text.push_str(&format!("  Note:    {note}\n"));
        }
        if let Some(url) = one.docs_url() {
            text.push_str(&format!("  Docs:    {url}\n"));
        }
    }
    if instructions.is_empty() {
        text.push_str("It is not known how an MCP server is added to any agent.\n");
    }
    let data = json!({
        "binary": skill.binary(),
        "server_name": innerword_agent_setup::MCP_SERVER_NAME,
        "agents": instructions,
    });
    Reply { text, data }
}

fn what(command: &str) -> (&'static str, &'static str) {
    match command {
        "agents install" => ("install", "installed"),
        "agents update" => ("update", "updated"),
        _ => ("remove", "removed"),
    }
}

/// What a command would do, from the state of the skill in each directory. Nothing is
/// written.
fn dry_run(command: &str, agent: Option<AgentId>, machine: &Machine, skill: &SkillSpec) -> Reply {
    let (verb, _) = what(command);
    // (directory, state, the agents that read it)
    let mut rows: Vec<(String, SkillState, Vec<&'static str>)> = Vec::new();
    match agent {
        Some(agent) => {
            let status = agent_status(machine, agent, skill);
            if let Some(agent_skill) = &status.skill {
                let installed: Vec<_> = agent_skill
                    .copies
                    .iter()
                    .filter(|copy| copy.state.is_installed())
                    .collect();
                if installed.is_empty() {
                    if verb == "install" {
                        rows.push((
                            agent_skill.dir_display.clone(),
                            agent_skill.state.clone(),
                            vec![status.name],
                        ));
                    }
                } else {
                    for copy in installed {
                        rows.push((
                            copy.dir_display.clone(),
                            copy.state.clone(),
                            vec![status.name],
                        ));
                    }
                }
            }
        }
        None => {
            if verb == "install" {
                for status in survey(machine, skill)
                    .iter()
                    .filter(|status| status.detected)
                {
                    let Some(agent_skill) = &status.skill else {
                        continue;
                    };
                    match rows.iter_mut().find(|row| row.0 == agent_skill.dir_display) {
                        Some(row) => row.2.push(status.name),
                        None => rows.push((
                            agent_skill.dir_display.clone(),
                            agent_skill.state.clone(),
                            vec![status.name],
                        )),
                    }
                }
            } else {
                for location in skill_locations(machine, skill) {
                    if location.state.is_installed() {
                        let names = location.agents.iter().map(|id| id.agent().name).collect();
                        rows.push((location.dir_display, location.state, names));
                    }
                }
            }
        }
    }
    let mut text = String::from("Dry run, nothing was changed. This would happen:\n");
    let mut planned: Vec<Value> = Vec::new();
    for (dir, state, agents) in &rows {
        let action = match (verb, state) {
            ("install", SkillState::NotInstalled) => "install",
            ("install" | "update", SkillState::Stale(_)) => "update",
            ("install" | "update", SkillState::Current) => "nothing (already up to date)",
            ("install", SkillState::Foreign) => {
                "nothing (another folder of that name is in the way)"
            }
            ("remove", SkillState::Current | SkillState::Stale(_)) => "remove",
            _ => "nothing",
        };
        text.push_str(&format!("  {dir}: {action} ({})\n", agents.join(", ")));
        planned.push(
            json!({ "dir_display": dir, "action": action, "state": state, "agents": agents }),
        );
    }
    if rows.is_empty() {
        text.push_str(match verb {
            "install" => "  nothing: no agent with a skills folder was found\n",
            _ => "  nothing: the skill is not installed there\n",
        });
    }
    Reply {
        text,
        data: json!({ "dry_run": true, "planned": planned }),
    }
}

fn finish(command: &str, agent: Option<AgentId>, batch: Batch) -> Result<Reply, Failure> {
    let (verb, _) = what(command);
    let mut text = String::new();
    if batch.is_empty() {
        text.push_str(&match (verb, agent) {
            ("install", _) => {
                "No agent with a skills folder was found on this Mac; nothing was installed.\n"
                    .to_string()
            }
            (_, Some(agent)) => {
                format!(
                    "The skill is not installed for {}; there was nothing to {verb}.\n",
                    agent.agent().name
                )
            }
            (_, None) => {
                format!("The skill is not installed anywhere; there was nothing to {verb}.\n")
            }
        });
    }
    for outcome in &batch.outcomes {
        text.push_str(&outcome.line());
        text.push('\n');
        if let Ok(report) = &outcome.result {
            for path in report.created_dirs.iter().chain(&report.written) {
                text.push_str(&format!("  wrote {}\n", path.display()));
            }
            for path in &report.removed {
                text.push_str(&format!("  removed {}\n", path.display()));
            }
            for path in &report.left_in_place {
                text.push_str(&format!("  left in place {}\n", path.display()));
            }
        }
    }
    let data = serde_json::to_value(&batch).map_err(failed)?;
    if batch.is_ok() {
        return Ok(Reply { text, data });
    }
    let mut failure = Failure::new(ErrorKind::Failed, text.trim_end().to_string());
    failure.data["outcomes"] = data["outcomes"].clone();
    Err(failure)
}
