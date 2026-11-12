//! Agents: what the app shows about the `writer` command line tool, the
//! skill that tells an agent about it, and the MCP server.
//!
//! The tool is a second binary beside the app's own (in the bundle:
//! `The Writer.app/Contents/MacOS/writer`). It works on the library without
//! the app; while the app is open it only reads (the app holds the lock of
//! the data directory, see [`crate::single_instance`]). Everything that is
//! known about the agents themselves (how one is detected on a Mac, where
//! its skills live, how an MCP server is added to it, the text of the
//! skill) is in the crate `writer-agent-setup`, which the tool uses too
//! (`writer agents`, `writer skill install`). This module turns it into
//! what the Agents panel ([`crate::agents_view`]) lists: a [`Model`].
//!
//! # Which Mac
//!
//! The panel reads agent directories under a home directory and writes the
//! skill there when the user presses a button. Which machine that is (a
//! home directory and the folders applications and commands are looked for
//! in, a `writer_agent_setup::Machine`), where the tool is and whether
//! "copy" may write the system clipboard is the [`Environment`], an app
//! global:
//!
//! - `main` calls [`use_this_mac`]: this Mac and the user's home, the tool
//!   beside the running executable, the real clipboard.
//! - With [`AGENT_HOME_ENV`] set, the machine is nothing but that home
//!   directory (`Machine::isolated`): what is installed on the Mac is not
//!   looked at, and nothing outside that directory is written.
//! - A scripted run (`THE_WRITER_AUTOMATION`) gets an environment only when
//!   [`AGENT_HOME_ENV`] names a home directory to use, and never writes the
//!   clipboard: a script cannot reach the user's agent directories or their
//!   clipboard by pressing a button.
//! - A test calls [`set_environment`] with a temporary home
//!   ([`Environment::isolated`]). Without an environment the panel lists
//!   nothing and touches nothing.
//!
//! # The hint
//!
//! Once per library, after a launch, a toast says that agents can work with
//! the library and leads to the panel ([`Workspace::offer_agents_hint`]).
//! It is switched on by `main` ([`enable_hint`]); a scripted run shows it
//! only when [`HINT_ENV`] is set, so screenshots are not covered by it.

use std::path::{Path, PathBuf};

use gpui::{App, Global};
use writer_agent_setup::{self as setup, AgentId, Batch, Machine, SkillSpec, SkillState};

use crate::workspace::Workspace;

/// The file name of the command line tool, beside the app's executable.
pub const TOOL_NAME: &str = "writer";

/// Environment variable: the home directory the Agents panel looks for
/// agents in and installs the skill under, instead of the user's. Nothing
/// else of the Mac is looked at then. A scripted run has no agents
/// environment without it.
pub const AGENT_HOME_ENV: &str = "THE_WRITER_AGENT_HOME";

/// Environment variable: when set to a non-empty value, a scripted run
/// shows the one-time hint about agents too.
pub const HINT_ENV: &str = "THE_WRITER_AGENTS_HINT";

/// What the one-time hint says. Its button opens the panel.
pub const HINT: &str = "New: the AI agents on this Mac can work with your library.";

/// The label of the hint's button.
pub const HINT_BUTTON: &str = "Agents";

/// Where the Agents panel looks and writes. See the [module docs](self).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Environment {
    /// The home directory and the folders agents are looked for in.
    pub machine: Machine,
    /// The absolute path of the `writer` tool.
    pub tool: PathBuf,
    /// Whether a copy button writes the system clipboard.
    pub clipboard: bool,
}

impl Environment {
    /// An environment that is nothing but `home`: agents are looked for
    /// below it only, and the skill is installed below it. For tests (the
    /// test platform's clipboard is written) and scripted runs.
    pub fn isolated(home: impl Into<PathBuf>, tool: impl Into<PathBuf>) -> Self {
        Self {
            machine: Machine::isolated(home),
            tool: tool.into(),
            clipboard: true,
        }
    }

    /// The home directory.
    pub fn home(&self) -> &Path {
        self.machine.home()
    }
}

#[derive(Default)]
struct State {
    environment: Option<Environment>,
    hint: bool,
}

impl Global for State {}

/// The path of the tool that belongs to this copy of the app: the file
/// [`TOOL_NAME`] beside the running executable, with symbolic links in the
/// executable's path resolved. In the bundle that is
/// `The Writer.app/Contents/MacOS/writer`; in a development build
/// `target/debug/writer` (built by `cargo build -p writer-cli`).
pub fn bundled_tool_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let exe = exe.canonicalize().unwrap_or(exe);
    Some(exe.parent()?.join(TOOL_NAME))
}

/// Why the tool's path will not last, when it will not: the app runs from
/// a disk image or from the place macOS moves an app to that was opened
/// straight from a download. An agent told this path would lose the tool.
pub fn temporary_place(tool: &Path) -> Option<&'static str> {
    let text = tool.to_string_lossy();
    if text.contains("/AppTranslocation/") {
        Some(
            "The Writer is running from a temporary place. Move it to the Applications folder \
             and open it from there; the path will then be a lasting one.",
        )
    } else if text.starts_with("/Volumes/") {
        Some(
            "The Writer is running from a disk image. Drag it to the Applications folder and \
             open it from there; the path will then be a lasting one.",
        )
    } else {
        None
    }
}

/// The agents environment of the real app: this Mac. `scripted` is whether
/// the app runs a `THE_WRITER_AUTOMATION` script (see the
/// [module docs](self) for what that changes). Called by `main`.
pub fn use_this_mac(scripted: bool, cx: &mut App) {
    let set = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty());
    let machine = match set(AGENT_HOME_ENV) {
        Some(home) => Some(Machine::isolated(PathBuf::from(home))),
        None if scripted => None,
        None => Machine::this_mac().ok(),
    };
    let environment = machine
        .zip(bundled_tool_path())
        .map(|(machine, tool)| Environment {
            machine,
            tool,
            clipboard: !scripted,
        });
    let state = cx.default_global::<State>();
    state.environment = environment;
    state.hint = !scripted || set(HINT_ENV).is_some();
}

/// Sets the agents environment (tests: a temporary home).
pub fn set_environment(environment: Environment, cx: &mut App) {
    cx.default_global::<State>().environment = Some(environment);
}

/// The agents environment, when the app has one.
pub fn environment(cx: &App) -> Option<Environment> {
    cx.try_global::<State>()?.environment.clone()
}

/// From now on a workspace shows the one-time hint about agents.
pub fn enable_hint(cx: &mut App) {
    cx.default_global::<State>().hint = true;
}

/// Whether workspaces show the one-time hint.
pub fn hint_enabled(cx: &App) -> bool {
    cx.try_global::<State>().is_some_and(|state| state.hint)
}

impl Workspace {
    /// Shows the hint about agents if this library has not shown it yet,
    /// and remembers that it did: a toast with a button that opens the
    /// Agents panel, never a dialog. Shown once, whether or not the button
    /// is pressed.
    pub(crate) fn offer_agents_hint(&mut self, cx: &mut gpui::Context<Self>) {
        use crate::settings::keys;
        if !hint_enabled(cx)
            || self
                .library
                .read(cx)
                .setting_or(keys::AGENTS_HINT_SHOWN, false)
        {
            return;
        }
        let stored = self.library.update(cx, |library, cx| {
            library.set_setting(keys::AGENTS_HINT_SHOWN, &true, cx)
        });
        // A hint that cannot be remembered would come back at every
        // launch: better not to show it at all.
        if stored.is_err() {
            return;
        }
        self.show_toast(
            writer_ui::Toast::new(HINT)
                .action(HINT_BUTTON, crate::actions::ShowAgents)
                .duration(std::time::Duration::from_secs(20)),
            cx,
        );
    }
}

// ----- what the panel lists ------------------------------------------------------

/// How the panel says the state of the skill.
pub fn skill_state_label(state: &SkillState) -> &'static str {
    match state {
        SkillState::NotInstalled => "Not installed",
        SkillState::Current => "Installed",
        SkillState::Stale(_) => "Needs updating",
        SkillState::Foreign => "Another folder is in the way",
    }
}

/// The skill for an agent found on this Mac, and where it is or goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillTarget {
    /// The agent. `None` for a copy of the skill in a folder that no agent
    /// found on this Mac reads any more (it can only be removed).
    pub agent: Option<AgentId>,
    /// The agent's name.
    pub name: String,
    /// The skills directory the skill's folder is in or goes to.
    pub skills_dir: PathBuf,
    /// The folder of the skill for this agent.
    pub directory: PathBuf,
    /// The same as the panel shows it (`~/.claude/skills/the-writer`).
    pub directory_display: String,
    /// Whether the skill is there.
    pub state: SkillState,
    /// What else there is to know: why it needs updating, which other
    /// agents read the same folder.
    pub detail: Option<String>,
}

/// The two ways an agent is told about an MCP server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum McpForm {
    /// A command to run in a terminal.
    Command,
    /// An entry in the agent's configuration file.
    Config,
}

/// The entry that adds the MCP server to an agent's configuration file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpSnippet {
    /// The file the entry belongs in, when it is one known file.
    pub file: Option<PathBuf>,
    /// One line that says which file that is, or how the agent opens it.
    pub place: String,
    /// Where in the file the entry goes.
    pub placement: String,
    /// The entry, with the path of the tool in it.
    pub text: String,
}

/// How The Writer's MCP server is added to one agent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpAgent {
    /// The agent.
    pub id: AgentId,
    /// The agent's name.
    pub name: String,
    /// Whether the agent was found on this Mac.
    pub detected: bool,
    /// The agent's own documentation about MCP servers.
    pub docs_url: String,
    /// The command that adds the server, with the path of the tool.
    pub command: Option<String>,
    /// The configuration entry that adds the server.
    pub config: Option<McpSnippet>,
    /// Something to know about this agent and MCP.
    pub note: Option<String>,
}

/// Everything the Agents panel lists.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Model {
    /// The absolute path of the `writer` tool.
    pub tool: PathBuf,
    /// Whether there is a file at that path.
    pub tool_found: bool,
    /// The agents found on this Mac that read skills (and, last, copies of
    /// the skill no agent found reads any more).
    pub skills: Vec<SkillTarget>,
    /// The names of the agents that read skills and were not found.
    pub not_found: Vec<String>,
    /// How the MCP server is added to each agent, found ones first.
    pub mcp: Vec<McpAgent>,
    /// Why nothing is listed, when the tool's path cannot be used.
    pub problem: Option<String>,
}

/// What an install, an update or a removal did: one line that says it, and
/// every path that was written or removed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    /// Whether it worked.
    pub ok: bool,
    /// What happened, in a sentence.
    pub headline: String,
    /// What was done to each path, one line each ("Wrote ~/...").
    pub lines: Vec<String>,
    /// What to know next: when the agent notices the skill.
    pub note: Option<String>,
    /// Every file and folder that was created, written or removed.
    pub paths: Vec<PathBuf>,
}

impl Report {
    fn failed(headline: impl Into<String>) -> Self {
        Self {
            ok: false,
            headline: headline.into(),
            lines: Vec::new(),
            note: None,
            paths: Vec::new(),
        }
    }
}

// ----- from writer-agent-setup -----------------------------------------------------

/// "Codex", "Codex and Cursor", "Codex, Cursor and Zed".
fn name_list(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [one] => (*one).to_string(),
        [first @ .., last] => format!("{} and {last}", first.join(", ")),
    }
}

fn agent_names(ids: &[AgentId]) -> Vec<&'static str> {
    ids.iter().map(|id| id.agent().name).collect()
}

/// Reads what the panel lists from the environment's machine. Reads only.
pub fn load(environment: &Environment) -> Model {
    let machine = &environment.machine;
    let mut model = Model {
        tool: environment.tool.clone(),
        tool_found: environment.tool.is_file(),
        ..Model::default()
    };
    let spec = match SkillSpec::new(&environment.tool) {
        Ok(spec) => spec,
        Err(error) => {
            model.problem = Some(error.to_string());
            return model;
        }
    };

    let survey = setup::survey(machine, &spec);
    for status in &survey {
        let Some(skill) = &status.skill else {
            continue;
        };
        if !status.detected {
            model.not_found.push(status.name.to_string());
            continue;
        }
        let mut detail = Vec::new();
        match &skill.state {
            SkillState::Stale(stale) => detail.push(stale.explanation()),
            SkillState::Foreign => detail.push(
                "A folder of this name is there that The Writer did not make. It is left as it \
                 is: move or rename it, then install."
                    .to_string(),
            ),
            SkillState::NotInstalled | SkillState::Current => {}
        }
        if !skill.shared_with.is_empty() {
            detail.push(format!(
                "{} {} this folder too.",
                name_list(&agent_names(&skill.shared_with)),
                if skill.shared_with.len() == 1 {
                    "reads"
                } else {
                    "read"
                }
            ));
        }
        model.skills.push(SkillTarget {
            agent: Some(status.id),
            name: status.name.to_string(),
            skills_dir: skill.dir.clone(),
            directory_display: machine.display(&skill.skill_dir),
            directory: skill.skill_dir.clone(),
            state: skill.state.clone(),
            detail: (!detail.is_empty()).then(|| detail.join(" ")),
        });
    }
    // A copy of the skill in a folder that no agent found here reads: it
    // can still be removed from the panel.
    for location in setup::skill_locations(machine, &spec) {
        if location.state.is_installed() && location.agents.is_empty() {
            let directory = setup::skill_dir(&location.dir);
            model.skills.push(SkillTarget {
                agent: None,
                name: "Left over".to_string(),
                directory_display: machine.display(&directory),
                skills_dir: location.dir,
                directory,
                state: location.state,
                detail: Some("No agent found on this Mac reads this folder any more.".to_string()),
            });
        }
    }

    let detected = |id: AgentId| {
        survey
            .iter()
            .any(|status| status.id == id && status.detected)
    };
    let instructions =
        setup::all_mcp_instructions(&environment.tool, Some(machine.home())).unwrap_or_default();
    let mut mcp: Vec<McpAgent> = instructions
        .into_iter()
        .map(|instructions| {
            let docs_url = instructions
                .docs_url()
                .unwrap_or(instructions.agent.agent().docs_url)
                .to_string();
            let config = instructions.config.map(|config| {
                let file = config
                    .file
                    .as_deref()
                    .map(|file| machine.display(file))
                    .or(config.file_display.clone());
                let place = match (file, config.open_with) {
                    (Some(file), Some(open_with)) => format!("In {file} ({open_with})"),
                    (Some(file), None) => format!("In {file}"),
                    (None, Some(open_with)) => format!("In the file opened by: {open_with}"),
                    (None, None) => "In its configuration file".to_string(),
                };
                McpSnippet {
                    file: config.file,
                    place,
                    placement: config.placement.to_string(),
                    text: config.snippet.trim_end().to_string(),
                }
            });
            McpAgent {
                id: instructions.agent,
                name: instructions.name.to_string(),
                detected: detected(instructions.agent),
                docs_url,
                command: instructions.command.map(|command| command.add),
                config,
                note: instructions.note.map(str::to_string),
            }
        })
        .collect();
    // The agents found on this Mac first, each group in the table's order.
    mcp.sort_by_key(|agent| !agent.detected);
    model.mcp = mcp;
    model
}

/// What was done in one skills directory, path by path.
fn outcome_lines(
    machine: &Machine,
    result: &Result<setup::Report, setup::SetupError>,
    lines: &mut Vec<String>,
) {
    let done = match result {
        Ok(done) => done,
        Err(error) => {
            lines.push(error.to_string());
            return;
        }
    };
    let show = |path: &PathBuf| machine.display(path);
    lines.extend(
        done.created_dirs
            .iter()
            .map(|path| format!("Created the folder {}", show(path))),
    );
    lines.extend(
        done.written
            .iter()
            .map(|path| format!("Wrote {}", show(path))),
    );
    lines.extend(
        done.removed
            .iter()
            .map(|path| format!("Removed {}", show(path))),
    );
    lines.extend(
        done.left_in_place
            .iter()
            .map(|path| format!("Left in place (not The Writer's): {}", show(path))),
    );
    if !done.touched_the_disk() {
        lines.push(format!(
            "{}: {}, nothing was written",
            show(&done.skill_dir),
            done.change.label()
        ));
    }
}

/// What the batch did, path by path.
fn report(environment: &Environment, batch: &Batch, headline: String) -> Report {
    let mut lines = Vec::new();
    for outcome in &batch.outcomes {
        outcome_lines(&environment.machine, &outcome.result, &mut lines);
    }
    Report {
        ok: batch.is_ok(),
        headline,
        lines,
        note: None,
        paths: batch.changed_paths(),
    }
}

/// The agents of the batch's folders other than `agent`.
fn others_in(batch: &Batch, agent: AgentId) -> Vec<AgentId> {
    let mut others = Vec::new();
    for outcome in &batch.outcomes {
        for id in &outcome.agents {
            if *id != agent && !others.contains(id) {
                others.push(*id);
            }
        }
    }
    others
}

/// Installs the skill for one agent, or brings it up to date.
pub fn install_skill(environment: &Environment, skill: &SkillTarget) -> Report {
    let Some(agent) = skill.agent else {
        return Report::failed("This copy of the skill belongs to no agent; it can be removed.");
    };
    let name = agent.agent().name;
    let spec = match SkillSpec::new(&environment.tool) {
        Ok(spec) => spec,
        Err(error) => return Report::failed(error.to_string()),
    };
    let batch = match setup::install_for_agent(&environment.machine, agent, &spec) {
        Ok(batch) => batch,
        Err(error) => return Report::failed(error.to_string()),
    };
    let mut headline = if !batch.is_ok() {
        format!("The skill for {name} could not be installed.")
    } else if !batch.touched_the_disk() {
        format!("The skill for {name} is already up to date.")
    } else if batch
        .reports()
        .all(|done| done.change == setup::Change::Updated)
    {
        format!("Updated the skill for {name}.")
    } else {
        format!("Installed the skill for {name}.")
    };
    if batch.is_ok() && batch.touched_the_disk() {
        let others = others_in(&batch, agent);
        if !others.is_empty() {
            headline.push_str(&format!(
                " {} {} the same folder and {} it too.",
                name_list(&agent_names(&others)),
                if others.len() == 1 { "reads" } else { "read" },
                if others.len() == 1 { "has" } else { "have" },
            ));
        }
    }
    let mut report = report(environment, &batch, headline);
    if report.ok && batch.touched_the_disk() {
        report.note = agent.agent().skills.pickup.map(str::to_string);
    }
    report
}

/// Removes the skill of one agent (from every folder that agent reads; the
/// other agents reading those folders lose it too, and the report says so).
pub fn remove_skill(environment: &Environment, skill: &SkillTarget) -> Report {
    let machine = &environment.machine;
    let Some(agent) = skill.agent else {
        // A copy no agent reads: that one folder.
        let result = setup::remove_skill(&skill.skills_dir);
        let mut lines = Vec::new();
        outcome_lines(machine, &result, &mut lines);
        return match result {
            Ok(done) => Report {
                ok: true,
                headline: "Removed the left-over copy of the skill.".to_string(),
                lines,
                note: None,
                paths: done.removed,
            },
            Err(_) => Report {
                ok: false,
                headline: "The left-over copy of the skill could not be removed.".to_string(),
                lines,
                note: None,
                paths: Vec::new(),
            },
        };
    };
    let name = agent.agent().name;
    let batch = setup::remove_for_agent(machine, agent);
    let mut headline = if !batch.is_ok() {
        format!("The skill for {name} could not be removed.")
    } else if batch.is_empty() {
        format!("There was no skill of The Writer's to remove for {name}.")
    } else {
        format!("Removed the skill for {name}.")
    };
    if batch.is_ok() && batch.touched_the_disk() {
        // The others that read a folder the skill was in, and do not find
        // it in another folder they read.
        let spec = SkillSpec::new(&environment.tool).ok();
        let still_has = |id: AgentId| {
            spec.as_ref().is_some_and(|spec| {
                setup::agent_status(machine, id, spec)
                    .skill
                    .is_some_and(|skill| skill.state.is_installed())
            })
        };
        let others: Vec<AgentId> = others_in(&batch, agent)
            .into_iter()
            .filter(|id| !still_has(*id))
            .collect();
        if !others.is_empty() {
            headline.push_str(&format!(
                " {} read the same folder and no longer {} it either.",
                name_list(&agent_names(&others)),
                if others.len() == 1 { "has" } else { "have" },
            ));
        }
    }
    report(environment, &batch, headline)
}

/// Installs the skill for every agent found, or brings it up to date,
/// writing as few copies as cover them all.
pub fn install_skill_for_all(environment: &Environment) -> Report {
    let spec = match SkillSpec::new(&environment.tool) {
        Ok(spec) => spec,
        Err(error) => return Report::failed(error.to_string()),
    };
    let batch = setup::install_for_detected(&environment.machine, &spec);
    let mut covered: Vec<AgentId> = Vec::new();
    for outcome in batch
        .outcomes
        .iter()
        .filter(|outcome| outcome.result.is_ok())
    {
        for id in &outcome.agents {
            if !covered.contains(id) {
                covered.push(*id);
            }
        }
    }
    let names = name_list(&agent_names(&covered));
    let headline = if batch.is_empty() {
        "No agent that reads skills was found on this Mac.".to_string()
    } else if !batch.is_ok() {
        "The skill could not be installed everywhere.".to_string()
    } else if !batch.touched_the_disk() {
        format!("The skill is already up to date for {names}.")
    } else {
        format!("Installed the skill for {names}.")
    };
    report(environment, &batch, headline)
}
