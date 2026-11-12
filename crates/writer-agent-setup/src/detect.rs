//! Which agents are on a Mac, the state of the skill for each, and installing, updating and
//! removing the skill for one agent or for all of them.
//!
//! Everything here is asked of a [`Machine`]: a home directory and the folders applications
//! and commands are looked for in. Tests build one over a temporary home
//! ([`Machine::isolated`]), so nothing in this module reaches the user's real directories
//! unless it is handed them ([`Machine::this_mac`]).
//!
//! # Agents share directories
//!
//! The skill is installed into a directory, and several agents may read the same one
//! (`~/.agents/skills` is read by Codex, Cursor, Copilot, Gemini CLI, OpenCode, Devin, Zed
//! and Pi; Cursor and OpenCode also read `~/.claude/skills`). So:
//!
//! - The state of the skill for an agent is the best state among the directories it reads:
//!   a current copy anywhere counts.
//! - Installing for an agent brings the copies it reads up to date, and writes a new copy
//!   into its preferred directory only when it reads none.
//! - Installing for all detected agents writes as few copies as cover them all.
//! - Removing for an agent removes every copy it reads, which takes the skill away from the
//!   other agents reading those directories too. [`AgentSkill::shared_with`] and
//!   [`Outcome::agents`] name them, so the interface can say so.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::ser::SerializeStruct;

use crate::agents::{AGENTS, Agent, AgentId, DetectRule, DirStanding};
use crate::error::SetupError;
use crate::install::{
    Report, SkillState, install_skill, installed_marker, remove_skill, skill_dir, skill_state,
};
use crate::skill::SkillSpec;

/// Folders below the home directory that commands are installed to by the installers the
/// agents use (their own scripts, npm, bun, Homebrew is system-wide and listed apart).
const HOME_COMMAND_DIRS: &[&str] = &[
    ".local/bin",
    "bin",
    ".bun/bin",
    ".npm-global/bin",
    ".npm/bin",
    ".yarn/bin",
    ".volta/bin",
    ".cargo/bin",
    ".deno/bin",
    ".opencode/bin",
    ".claude/local",
    "Library/pnpm",
    ".local/share/pnpm",
    ".local/share/mise/shims",
    ".asdf/shims",
];

/// System folders commands are installed to on a Mac.
const SYSTEM_COMMAND_DIRS: &[&str] = &["/opt/homebrew/bin", "/usr/local/bin"];

/// Where to look: a home directory, and the folders applications and commands live in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Machine {
    home: PathBuf,
    app_dirs: Vec<PathBuf>,
    command_dirs: Vec<PathBuf>,
}

impl Machine {
    /// The Mac this process runs on, for the user it runs as: `HOME`, `/Applications` and
    /// `~/Applications`, and for commands the `PATH` of this process plus the folders
    /// installers use (an app started from the Finder has a `PATH` without them).
    pub fn this_mac() -> Result<Machine, SetupError> {
        let home = std::env::var_os("HOME")
            .filter(|home| !home.is_empty())
            .ok_or(SetupError::NoHome)?;
        Ok(Machine::for_home(home))
    }

    /// This Mac with another home directory: the system's folders (`/Applications`,
    /// `/opt/homebrew/bin`, the `PATH`) are looked at as well as the ones below `home`.
    pub fn for_home(home: impl Into<PathBuf>) -> Machine {
        let mut machine = Machine::isolated(home);
        machine.app_dirs.insert(0, PathBuf::from("/Applications"));
        let mut command_dirs: Vec<PathBuf> = std::env::var_os("PATH")
            .map(|path| {
                std::env::split_paths(&path)
                    .filter(|dir| dir.is_absolute())
                    .collect()
            })
            .unwrap_or_default();
        command_dirs.append(&mut machine.command_dirs);
        command_dirs.extend(SYSTEM_COMMAND_DIRS.iter().map(PathBuf::from));
        let mut seen = BTreeSet::new();
        command_dirs.retain(|dir| seen.insert(dir.clone()));
        machine.command_dirs = command_dirs;
        machine
    }

    /// A machine that is nothing but `home`: applications are looked for in
    /// `home/Applications` and commands in the folders below `home` only. For tests and for
    /// scripted runs, which must not depend on what is installed on the Mac they run on.
    pub fn isolated(home: impl Into<PathBuf>) -> Machine {
        let home = home.into();
        let app_dirs = vec![home.join("Applications")];
        let mut command_dirs: Vec<PathBuf> =
            HOME_COMMAND_DIRS.iter().map(|dir| home.join(dir)).collect();
        // Node versions managed by nvm each have their own folder of commands.
        if let Ok(versions) = std::fs::read_dir(home.join(".nvm/versions/node")) {
            let mut bins: Vec<PathBuf> = versions
                .filter_map(|entry| Some(entry.ok()?.path().join("bin")))
                .collect();
            bins.sort();
            command_dirs.extend(bins);
        }
        Machine {
            home,
            app_dirs,
            command_dirs,
        }
    }

    /// The same machine with these folders of applications instead.
    pub fn with_app_dirs(mut self, dirs: Vec<PathBuf>) -> Machine {
        self.app_dirs = dirs;
        self
    }

    /// The same machine with these folders of commands instead.
    pub fn with_command_dirs(mut self, dirs: Vec<PathBuf>) -> Machine {
        self.command_dirs = dirs;
        self
    }

    /// The home directory.
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// The folders applications are looked for in.
    pub fn app_dirs(&self) -> &[PathBuf] {
        &self.app_dirs
    }

    /// The folders commands are looked for in.
    pub fn command_dirs(&self) -> &[PathBuf] {
        &self.command_dirs
    }

    /// A path below the home directory.
    pub fn in_home(&self, home_relative: &str) -> PathBuf {
        self.home.join(home_relative)
    }

    /// `path` as a person reads it: `~/...` when it is below the home directory.
    pub fn display(&self, path: &Path) -> String {
        match path.strip_prefix(&self.home) {
            Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
            Ok(rest) => format!("~/{}", rest.display()),
            Err(_) => path.display().to_string(),
        }
    }

    /// Where a rule is satisfied on this machine, if it is.
    fn find(&self, rule: DetectRule) -> Option<PathBuf> {
        match rule {
            DetectRule::App(name) => self
                .app_dirs
                .iter()
                .map(|dir| dir.join(name))
                .find(|path| path.is_dir()),
            DetectRule::Command(name) => self
                .command_dirs
                .iter()
                .map(|dir| dir.join(name))
                .find(|path| is_executable_file(path)),
            DetectRule::HomePath(path) => Some(self.home.join(path)).filter(|path| path.exists()),
        }
    }
}

fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// One thing that shows an agent is installed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Evidence {
    /// The rule of the table that matched.
    pub rule: DetectRule,
    /// What was found.
    pub path: PathBuf,
}

/// The skill in one of the directories an agent reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkillCopy {
    /// The skills directory (`~/.claude/skills`), absolute.
    pub dir: PathBuf,
    /// The same as a person reads it.
    pub dir_display: String,
    pub standing: DirStanding,
    pub state: SkillState,
}

/// The skill for one agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentSkill {
    /// The skills directory the state is about: where the agent finds the skill now, or
    /// where it would be installed for it.
    pub dir: PathBuf,
    /// The same as a person reads it (`~/.claude/skills`).
    pub dir_display: String,
    /// The skill's folder in it.
    pub skill_dir: PathBuf,
    /// The best state among the directories the agent reads.
    pub state: SkillState,
    /// The other detected agents that read `dir`: installing or removing here does it for
    /// them too.
    pub shared_with: Vec<AgentId>,
    /// Every directory the agent reads, in its order of preference, with the state there.
    pub copies: Vec<SkillCopy>,
    /// When the agent notices a newly installed skill, if its documentation says.
    pub pickup: Option<&'static str>,
    /// Anything else a person should know about this agent and skills.
    pub note: Option<&'static str>,
}

/// One agent of the table on one machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentStatus {
    pub id: AgentId,
    /// The agent's name ("Claude Code").
    pub name: &'static str,
    /// Whether anything shows that it is installed.
    pub detected: bool,
    /// What shows it, one entry per rule that matched.
    pub evidence: Vec<Evidence>,
    /// The skill for it; `None` when no skills directory is known for the agent.
    pub skill: Option<AgentSkill>,
}

/// What shows that an agent is installed on the machine; empty when nothing does.
pub fn evidence(machine: &Machine, agent: AgentId) -> Vec<Evidence> {
    agent
        .agent()
        .detection
        .iter()
        .filter_map(|detection| {
            machine.find(detection.rule).map(|path| Evidence {
                rule: detection.rule,
                path,
            })
        })
        .collect()
}

/// Whether an agent is installed on the machine.
pub fn is_detected(machine: &Machine, agent: AgentId) -> bool {
    agent
        .agent()
        .detection
        .iter()
        .any(|detection| machine.find(detection.rule).is_some())
}

/// The agents installed on the machine, in the order of the table.
pub fn detected_agents(machine: &Machine) -> Vec<AgentId> {
    AGENTS
        .iter()
        .filter(|agent| is_detected(machine, agent.id))
        .map(|agent| agent.id)
        .collect()
}

/// Every agent of the table with whether it is installed and the state of the skill for
/// it, measured against `spec` (what would be installed now). Reads only.
pub fn survey(machine: &Machine, spec: &SkillSpec) -> Vec<AgentStatus> {
    let detected = detected_agents(machine);
    AGENTS
        .iter()
        .map(|agent| status_of(machine, agent, spec, &detected))
        .collect()
}

/// One agent of the [`survey`].
pub fn agent_status(machine: &Machine, agent: AgentId, spec: &SkillSpec) -> AgentStatus {
    status_of(machine, agent.agent(), spec, &detected_agents(machine))
}

fn status_of(
    machine: &Machine,
    agent: &'static Agent,
    spec: &SkillSpec,
    detected: &[AgentId],
) -> AgentStatus {
    let evidence = evidence(machine, agent.id);
    let copies: Vec<SkillCopy> = agent
        .skills
        .dirs
        .iter()
        .map(|dir| {
            let path = machine.in_home(dir.home_relative);
            SkillCopy {
                dir_display: machine.display(&path),
                state: skill_state(&path, spec),
                standing: dir.standing,
                dir: path,
            }
        })
        .collect();
    let chosen = copies
        .iter()
        .find(|copy| copy.state == SkillState::Current)
        .or_else(|| copies.iter().find(|copy| copy.state.is_installed()))
        .or_else(|| {
            copies
                .iter()
                .find(|copy| copy.standing == DirStanding::Documented)
        })
        .or(copies.first())
        .cloned();
    let skill = chosen.map(|copy| AgentSkill {
        shared_with: readers(machine, &copy.dir, detected)
            .into_iter()
            .filter(|other| *other != agent.id)
            .collect(),
        skill_dir: skill_dir(&copy.dir),
        dir: copy.dir,
        dir_display: copy.dir_display,
        state: copy.state,
        copies,
        pickup: agent.skills.pickup,
        note: agent.skills.note,
    });
    AgentStatus {
        id: agent.id,
        name: agent.name,
        detected: !evidence.is_empty(),
        evidence,
        skill,
    }
}

/// The agents among `among` that read skills from `dir`.
fn readers(machine: &Machine, dir: &Path, among: &[AgentId]) -> Vec<AgentId> {
    among
        .iter()
        .copied()
        .filter(|id| {
            id.agent()
                .skills
                .dirs
                .iter()
                .any(|known| machine.in_home(known.home_relative) == dir)
        })
        .collect()
}

/// The directories an agent reads, absolute, in its order.
fn dirs_of(machine: &Machine, agent: AgentId) -> Vec<PathBuf> {
    agent
        .agent()
        .skills
        .dirs
        .iter()
        .map(|dir| machine.in_home(dir.home_relative))
        .collect()
}

/// Whether a skill The Writer wrote is in `dir`, current or not.
fn has_ours(dir: &Path) -> bool {
    installed_marker(dir).is_some()
}

/// What was done in one skills directory.
#[derive(Debug)]
pub struct Outcome {
    /// The skills directory.
    pub skills_dir: PathBuf,
    /// The same as a person reads it (`~/.agents/skills`).
    pub dir_display: String,
    /// The detected agents that read this directory, plus the one that was asked for.
    pub agents: Vec<AgentId>,
    /// What was written or removed, or why nothing was.
    pub result: Result<Report, SetupError>,
}

impl Outcome {
    /// One line for a person: `~/.agents/skills: installed (Codex, Cursor)`.
    pub fn line(&self) -> String {
        let what = match &self.result {
            Ok(report) => report.change.label().to_string(),
            Err(error) => format!("failed: {error}"),
        };
        if self.agents.is_empty() {
            return format!("{}: {what}", self.dir_display);
        }
        let names: Vec<&str> = self.agents.iter().map(|id| id.agent().name).collect();
        format!("{}: {what} ({})", self.dir_display, names.join(", "))
    }
}

/// `{"skills_dir", "dir_display", "agents", "report"}` when it worked, with `"error"` (the
/// message) and `"foreign"` (whether a folder somebody else made was in the way) in place
/// of the report when it did not.
impl Serialize for Outcome {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut out = serializer.serialize_struct("Outcome", 5)?;
        out.serialize_field("skills_dir", &self.skills_dir)?;
        out.serialize_field("dir_display", &self.dir_display)?;
        out.serialize_field("agents", &self.agents)?;
        match &self.result {
            Ok(report) => {
                out.serialize_field("report", report)?;
                out.skip_field("error")?;
                out.skip_field("foreign")?;
            }
            Err(error) => {
                out.skip_field("report")?;
                out.serialize_field("error", &error.to_string())?;
                out.serialize_field("foreign", &matches!(error, SetupError::Foreign(_)))?;
            }
        }
        out.end()
    }
}

/// What a call did, one [`Outcome`] per skills directory it was about. Empty when there
/// was nothing to do (updating or removing where the skill is not installed).
#[derive(Debug, Default, Serialize)]
pub struct Batch {
    pub outcomes: Vec<Outcome>,
}

impl Batch {
    /// Whether nothing failed.
    pub fn is_ok(&self) -> bool {
        self.outcomes.iter().all(|outcome| outcome.result.is_ok())
    }

    /// Whether there was nothing to do.
    pub fn is_empty(&self) -> bool {
        self.outcomes.is_empty()
    }

    /// Whether anything on disk was changed.
    pub fn touched_the_disk(&self) -> bool {
        self.reports().any(Report::touched_the_disk)
    }

    /// The reports of the directories where it worked.
    pub fn reports(&self) -> impl Iterator<Item = &Report> {
        self.outcomes
            .iter()
            .filter_map(|outcome| outcome.result.as_ref().ok())
    }

    /// The failures, each with its directory.
    pub fn errors(&self) -> impl Iterator<Item = (&Path, &SetupError)> {
        self.outcomes.iter().filter_map(|outcome| {
            outcome
                .result
                .as_ref()
                .err()
                .map(|error| (outcome.skills_dir.as_path(), error))
        })
    }

    /// Every path that was written, created or removed, in order.
    pub fn changed_paths(&self) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        for report in self.reports() {
            paths.extend(report.created_dirs.iter().cloned());
            paths.extend(report.written.iter().cloned());
            paths.extend(report.removed.iter().cloned());
        }
        paths
    }
}

fn outcome(
    machine: &Machine,
    dir: PathBuf,
    detected: &[AgentId],
    asked_for: Option<AgentId>,
    result: Result<Report, SetupError>,
) -> Outcome {
    let mut agents = readers(machine, &dir, detected);
    if let Some(asked_for) = asked_for
        && !agents.contains(&asked_for)
    {
        agents.insert(0, asked_for);
    }
    Outcome {
        dir_display: machine.display(&dir),
        skills_dir: dir,
        agents,
        result,
    }
}

/// The preferred directory of an agent, or the error that none is known.
fn preferred(machine: &Machine, agent: AgentId) -> Result<PathBuf, SetupError> {
    let dir = agent
        .agent()
        .preferred_skills_dir()
        .ok_or(SetupError::NoSkillsDir(agent))?;
    Ok(machine.in_home(dir.home_relative))
}

/// Installs the skill for one agent, detected or not, or brings it up to date.
///
/// Every copy of The Writer's in the directories the agent reads is brought up to date.
/// When it reads none, the skill is written into its preferred directory (created if
/// missing). Fails as a whole with [`SetupError::NoSkillsDir`] when no directory is known
/// for the agent; what goes wrong in a directory is in its [`Outcome`].
pub fn install_for_agent(
    machine: &Machine,
    agent: AgentId,
    spec: &SkillSpec,
) -> Result<Batch, SetupError> {
    let target = preferred(machine, agent)?;
    let detected = detected_agents(machine);
    let mut dirs: Vec<PathBuf> = dirs_of(machine, agent)
        .into_iter()
        .filter(|dir| has_ours(dir))
        .collect();
    if dirs.is_empty() {
        dirs.push(target);
    }
    Ok(Batch {
        outcomes: dirs
            .into_iter()
            .map(|dir| {
                let result = install_skill(&dir, spec);
                outcome(machine, dir, &detected, Some(agent), result)
            })
            .collect(),
    })
}

/// Installs the skill for every detected agent that reads skills, or brings it up to date,
/// writing as few copies as cover them all: an agent that already reads a directory with
/// the skill in it (or one that is about to get it) needs no copy of its own.
pub fn install_for_detected(machine: &Machine, spec: &SkillSpec) -> Batch {
    let detected = detected_agents(machine);
    let mut planned: Vec<PathBuf> = Vec::new();
    for agent in &detected {
        let dirs = dirs_of(machine, *agent);
        let mut covered = false;
        for dir in &dirs {
            if planned.contains(dir) {
                covered = true;
            } else if has_ours(dir) {
                planned.push(dir.clone());
                covered = true;
            }
        }
        if !covered && let Ok(target) = preferred(machine, *agent) {
            planned.push(target);
        }
    }
    Batch {
        outcomes: planned
            .into_iter()
            .map(|dir| {
                let result = install_skill(&dir, spec);
                outcome(machine, dir, &detected, None, result)
            })
            .collect(),
    }
}

/// Brings the skill up to date where one agent reads it. An agent that does not have the
/// skill is left alone: the batch is then empty.
pub fn update_for_agent(machine: &Machine, agent: AgentId, spec: &SkillSpec) -> Batch {
    let detected = detected_agents(machine);
    let dirs = dirs_of(machine, agent)
        .into_iter()
        .filter(|dir| has_ours(dir));
    Batch {
        outcomes: dirs
            .map(|dir| {
                let result = install_skill(&dir, spec);
                outcome(machine, dir, &detected, Some(agent), result)
            })
            .collect(),
    }
}

/// Brings the skill up to date in every directory of the table where The Writer installed
/// it, whether the agents reading it are still detected or not. Installs nothing new.
pub fn update_installed(machine: &Machine, spec: &SkillSpec) -> Batch {
    let detected = detected_agents(machine);
    Batch {
        outcomes: all_dirs(machine)
            .into_iter()
            .filter(|dir| has_ours(dir))
            .map(|dir| {
                let result = install_skill(&dir, spec);
                outcome(machine, dir, &detected, None, result)
            })
            .collect(),
    }
}

/// Removes the skill from every directory one agent reads where The Writer installed it.
/// Other agents reading those directories lose it too ([`Outcome::agents`] names them). The
/// batch is empty when the agent does not have the skill.
pub fn remove_for_agent(machine: &Machine, agent: AgentId) -> Batch {
    let detected = detected_agents(machine);
    remove_from(machine, dirs_of(machine, agent), &detected, Some(agent))
}

/// Removes the skill from every directory of the table where The Writer installed it.
pub fn remove_everywhere(machine: &Machine) -> Batch {
    let detected = detected_agents(machine);
    remove_from(machine, all_dirs(machine), &detected, None)
}

fn remove_from(
    machine: &Machine,
    dirs: Vec<PathBuf>,
    detected: &[AgentId],
    asked_for: Option<AgentId>,
) -> Batch {
    let mut outcomes = Vec::new();
    for dir in dirs {
        if !has_ours(&dir) {
            continue;
        }
        let result = remove_skill(&dir);
        outcomes.push(outcome(machine, dir, detected, asked_for, result));
    }
    Batch { outcomes }
}

/// Every skills directory of the table on this machine, each once, in the order of the
/// table.
pub fn all_dirs(machine: &Machine) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    for agent in AGENTS {
        for dir in dirs_of(machine, agent.id) {
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
    }
    dirs
}

/// One skills directory of the table with the state of the skill in it and who reads it:
/// the view by directory, for an interface that lists places and not agents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkillLocation {
    pub dir: PathBuf,
    /// The same as a person reads it (`~/.agents/skills`).
    pub dir_display: String,
    pub state: SkillState,
    /// The detected agents that read it.
    pub agents: Vec<AgentId>,
}

/// The skills directories that matter on this machine: those a detected agent reads from
/// by preference, and those The Writer's skill is in. Reads only.
pub fn skill_locations(machine: &Machine, spec: &SkillSpec) -> Vec<SkillLocation> {
    let detected = detected_agents(machine);
    let preferred: Vec<PathBuf> = detected
        .iter()
        .filter_map(|agent| preferred(machine, *agent).ok())
        .collect();
    all_dirs(machine)
        .into_iter()
        .filter_map(|dir| {
            let state = skill_state(&dir, spec);
            (state.is_installed() || preferred.contains(&dir)).then(|| SkillLocation {
                dir_display: machine.display(&dir),
                agents: readers(machine, &dir, &detected),
                state,
                dir,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::install::{Change, StaleReason};
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::TempDir;

    /// A temporary home whose path has spaces, with a fake app that carries the tool.
    struct Home {
        dir: TempDir,
    }

    impl Home {
        fn new() -> Home {
            let dir = tempfile::Builder::new()
                .prefix("a home with spaces ")
                .tempdir()
                .unwrap();
            let home = Home { dir };
            fs::create_dir_all(home.binary().parent().unwrap()).unwrap();
            fs::write(home.binary(), "#!/bin/sh\n").unwrap();
            home
        }

        fn path(&self, relative: &str) -> PathBuf {
            self.dir.path().join(relative)
        }

        fn binary(&self) -> PathBuf {
            self.path("Applications/The Writer.app/Contents/MacOS/writer")
        }

        fn spec(&self) -> SkillSpec {
            SkillSpec::new(self.binary()).unwrap()
        }

        fn machine(&self) -> Machine {
            Machine::isolated(self.dir.path())
        }

        fn mkdir(&self, relative: &str) {
            fs::create_dir_all(self.path(relative)).unwrap();
        }

        fn command(&self, relative: &str) {
            let path = self.path(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, "#!/bin/sh\n").unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }

        fn state(&self, relative: &str) -> SkillState {
            skill_state(&self.path(relative), &self.spec())
        }

        /// Every path below the home, sorted.
        fn everything(&self) -> Vec<PathBuf> {
            fn walk(folder: &Path, out: &mut Vec<PathBuf>) {
                for entry in fs::read_dir(folder).unwrap() {
                    let path = entry.unwrap().path();
                    out.push(path.clone());
                    if path.is_dir() && !path.is_symlink() {
                        walk(&path, out);
                    }
                }
            }
            let mut out = Vec::new();
            walk(self.dir.path(), &mut out);
            out.sort();
            out
        }
    }

    #[test]
    fn an_empty_home_has_no_agents() {
        let home = Home::new();
        let machine = home.machine();
        assert_eq!(detected_agents(&machine), Vec::<AgentId>::new());
        let survey = survey(&machine, &home.spec());
        assert_eq!(survey.len(), AGENTS.len());
        for status in &survey {
            assert!(
                !status.detected && status.evidence.is_empty(),
                "{}",
                status.name
            );
            let skill = status
                .skill
                .as_ref()
                .expect("every agent of the table reads skills");
            assert_eq!(skill.state, SkillState::NotInstalled);
            assert!(skill.shared_with.is_empty());
        }
        assert!(install_for_detected(&machine, &home.spec()).is_empty());
        assert!(skill_locations(&machine, &home.spec()).is_empty());
    }

    #[test]
    fn agents_are_detected_by_application_command_and_configuration_directory() {
        let home = Home::new();
        home.mkdir("Applications/Claude.app/Contents");
        home.command(".local/bin/codex");
        home.mkdir(".cursor");
        home.command(".opencode/bin/opencode");
        // A file that is not executable is not a command, and a file is not an application.
        fs::write(home.path(".local/bin/gemini"), "").unwrap();
        fs::write(home.path("Applications/Zed.app"), "").unwrap();
        let machine = home.machine();

        assert_eq!(
            detected_agents(&machine),
            vec![
                AgentId::ClaudeDesktop,
                AgentId::Codex,
                AgentId::Cursor,
                AgentId::OpenCode
            ]
        );
        let desktop = evidence(&machine, AgentId::ClaudeDesktop);
        assert_eq!(desktop.len(), 1);
        assert_eq!(desktop[0].rule, DetectRule::App("Claude.app"));
        assert_eq!(desktop[0].path, home.path("Applications/Claude.app"));
        assert_eq!(
            evidence(&machine, AgentId::Codex)[0].path,
            home.path(".local/bin/codex")
        );
        assert_eq!(
            evidence(&machine, AgentId::Cursor)[0].rule,
            DetectRule::HomePath(".cursor")
        );
        assert!(!is_detected(&machine, AgentId::GeminiCli));
        assert!(!is_detected(&machine, AgentId::Zed));

        // Several rules can match; each is listed.
        home.command("bin/claude");
        home.mkdir(".claude");
        let code = evidence(&home.machine(), AgentId::ClaudeCode);
        assert_eq!(code.len(), 2);
    }

    #[test]
    fn commands_are_found_in_the_folders_given_and_under_nvm() {
        let home = Home::new();
        home.command("elsewhere/bin/copilot");
        assert!(!is_detected(&home.machine(), AgentId::CopilotCli));
        let machine = home
            .machine()
            .with_command_dirs(vec![home.path("elsewhere/bin")]);
        assert_eq!(detected_agents(&machine), vec![AgentId::CopilotCli]);

        home.command(".nvm/versions/node/v24.1.0/bin/gemini");
        assert!(is_detected(&home.machine(), AgentId::GeminiCli));

        let apps = home.machine().with_app_dirs(vec![home.path("Other Apps")]);
        home.mkdir("Other Apps/Zed.app");
        assert!(is_detected(&apps, AgentId::Zed));
        assert_eq!(apps.app_dirs(), &[home.path("Other Apps")]);
    }

    #[test]
    fn the_real_layout_adds_the_system_folders() {
        let home = Home::new();
        let machine = Machine::for_home(home.dir.path());
        assert_eq!(machine.home(), home.dir.path());
        assert_eq!(machine.app_dirs()[0], PathBuf::from("/Applications"));
        assert!(machine.app_dirs().contains(&home.path("Applications")));
        assert!(
            machine
                .command_dirs()
                .contains(&PathBuf::from("/opt/homebrew/bin"))
        );
        assert!(machine.command_dirs().contains(&home.path(".local/bin")));
        let mut sorted = machine.command_dirs().to_vec();
        sorted.sort();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            machine.command_dirs().len(),
            "no folder twice"
        );
    }

    #[test]
    fn paths_are_shown_with_a_tilde() {
        let home = Home::new();
        let machine = home.machine();
        assert_eq!(
            machine.display(&home.path(".claude/skills")),
            "~/.claude/skills"
        );
        assert_eq!(machine.display(home.dir.path()), "~");
        assert_eq!(
            machine.display(Path::new("/Applications/Zed.app")),
            "/Applications/Zed.app"
        );
    }

    #[test]
    fn install_for_one_agent_writes_its_preferred_directory_and_reports_it() {
        let home = Home::new();
        home.mkdir(".claude");
        let machine = home.machine();
        let before = home.everything();

        let batch = install_for_agent(&machine, AgentId::ClaudeCode, &home.spec()).unwrap();
        assert!(batch.is_ok() && batch.touched_the_disk());
        assert_eq!(batch.outcomes.len(), 1);
        let outcome = &batch.outcomes[0];
        assert_eq!(outcome.skills_dir, home.path(".claude/skills"));
        assert_eq!(outcome.dir_display, "~/.claude/skills");
        assert_eq!(outcome.agents, vec![AgentId::ClaudeCode]);
        assert_eq!(outcome.line(), "~/.claude/skills: installed (Claude Code)");
        let report = outcome.result.as_ref().unwrap();
        assert_eq!(report.change, Change::Installed);
        assert_eq!(
            batch.changed_paths(),
            vec![
                home.path(".claude/skills"),
                home.path(".claude/skills/the-writer"),
                home.path(".claude/skills/the-writer/SKILL.md"),
            ]
        );
        let mut expected = before;
        expected.extend(batch.changed_paths());
        expected.sort();
        assert_eq!(home.everything(), expected, "nothing else was written");

        let status = agent_status(&machine, AgentId::ClaudeCode, &home.spec());
        let skill = status.skill.unwrap();
        assert_eq!(skill.state, SkillState::Current);
        assert_eq!(skill.dir_display, "~/.claude/skills");
        assert_eq!(skill.skill_dir, home.path(".claude/skills/the-writer"));
        // The text names the tool by its path with spaces, quoted.
        let text = fs::read_to_string(home.path(".claude/skills/the-writer/SKILL.md")).unwrap();
        assert!(text.contains(&format!("'{}'", home.binary().display())));
    }

    #[test]
    fn an_agent_that_is_not_detected_can_still_be_installed_for() {
        let home = Home::new();
        let machine = home.machine();
        let batch = install_for_agent(&machine, AgentId::Antigravity, &home.spec()).unwrap();
        assert_eq!(
            batch.outcomes[0].skills_dir,
            home.path(".gemini/config/skills")
        );
        assert_eq!(batch.outcomes[0].agents, vec![AgentId::Antigravity]);
        assert_eq!(home.state(".gemini/config/skills"), SkillState::Current);
    }

    #[test]
    fn install_for_all_detected_writes_as_few_copies_as_cover_them() {
        let home = Home::new();
        home.mkdir(".claude"); // Claude Code: ~/.claude/skills only
        home.command(".local/bin/codex"); // Codex: ~/.agents/skills
        home.mkdir(".cursor"); // Cursor reads both
        home.command(".opencode/bin/opencode"); // OpenCode reads both
        home.mkdir("Applications/Zed.app"); // Zed: ~/.agents/skills
        // Somebody else's skills, which must stay exactly as they are.
        home.mkdir(".agents/skills/another-skill");
        fs::write(
            home.path(".agents/skills/another-skill/SKILL.md"),
            "---\nname: another-skill\n---\n",
        )
        .unwrap();
        let machine = home.machine();
        let before = home.everything();

        let batch = install_for_detected(&machine, &home.spec());
        assert!(batch.is_ok());
        let dirs: Vec<&str> = batch
            .outcomes
            .iter()
            .map(|outcome| outcome.dir_display.as_str())
            .collect();
        assert_eq!(dirs, vec!["~/.claude/skills", "~/.agents/skills"]);
        assert_eq!(
            batch.outcomes[0].agents,
            vec![AgentId::ClaudeCode, AgentId::Cursor, AgentId::OpenCode]
        );
        assert_eq!(
            batch.outcomes[1].agents,
            vec![
                AgentId::Codex,
                AgentId::Cursor,
                AgentId::OpenCode,
                AgentId::Zed
            ]
        );
        assert!(
            !home.path(".cursor/skills").exists(),
            "Cursor is covered and gets no copy of its own"
        );
        assert!(!home.path(".config/opencode").exists());

        let mut expected = before;
        expected.extend(batch.changed_paths());
        expected.sort();
        assert_eq!(home.everything(), expected, "nothing else was written");

        for status in survey(&machine, &home.spec())
            .into_iter()
            .filter(|status| status.detected)
        {
            assert_eq!(
                status.skill.as_ref().unwrap().state,
                SkillState::Current,
                "{}",
                status.name
            );
        }
        let cursor = agent_status(&machine, AgentId::Cursor, &home.spec())
            .skill
            .unwrap();
        assert_eq!(cursor.dir_display, "~/.agents/skills");
        assert_eq!(
            cursor.shared_with,
            vec![AgentId::Codex, AgentId::OpenCode, AgentId::Zed]
        );
        assert_eq!(cursor.copies.len(), 4);

        // Again: nothing to do, and it says so.
        let again = install_for_detected(&machine, &home.spec());
        assert!(!again.touched_the_disk());
        assert!(
            again
                .reports()
                .all(|report| report.change == Change::Unchanged)
        );

        let locations = skill_locations(&machine, &home.spec());
        assert_eq!(locations.len(), 2);
        assert!(
            locations
                .iter()
                .all(|location| location.state == SkillState::Current)
        );
    }

    #[test]
    fn a_stale_copy_is_reported_and_updated_where_it_is() {
        let home = Home::new();
        home.mkdir(".cursor");
        home.mkdir(".claude");
        let machine = home.machine();
        let old = home.spec().with_version("0.1.0");
        install_for_agent(&machine, AgentId::ClaudeCode, &old).unwrap();

        // Cursor reads ~/.claude/skills too: it has the old copy, and no new one is made.
        let cursor = agent_status(&machine, AgentId::Cursor, &home.spec())
            .skill
            .unwrap();
        assert_eq!(cursor.dir_display, "~/.claude/skills");
        let SkillState::Stale(stale) = &cursor.state else {
            panic!("not stale: {:?}", cursor.state)
        };
        assert_eq!(stale.reason, StaleReason::OtherVersion);

        let batch = update_for_agent(&machine, AgentId::Cursor, &home.spec());
        assert_eq!(batch.outcomes.len(), 1);
        assert_eq!(
            batch.outcomes[0].result.as_ref().unwrap().change,
            Change::Updated
        );
        assert_eq!(
            batch.outcomes[0].agents,
            vec![AgentId::ClaudeCode, AgentId::Cursor]
        );
        assert_eq!(home.state(".claude/skills"), SkillState::Current);
        assert!(!home.path(".agents").exists());

        // An agent without the skill is left alone by an update.
        assert!(update_for_agent(&machine, AgentId::Codex, &home.spec()).is_empty());
        assert!(!home.path(".agents").exists());
    }

    #[test]
    fn update_installed_finds_every_copy_and_installs_nothing_new() {
        let home = Home::new();
        home.mkdir(".claude");
        home.command(".local/bin/codex");
        let machine = home.machine();
        let old = home.spec().with_version("0.1.0");
        install_for_detected(&machine, &old);
        // A copy for an agent that is not detected any more.
        install_for_agent(&machine, AgentId::Antigravity, &old).unwrap();
        // A deprecated directory that still has a copy.
        install_skill(&home.path(".codex/skills"), &old).unwrap();

        let batch = update_installed(&machine, &home.spec());
        assert!(batch.is_ok());
        let dirs: Vec<&str> = batch
            .outcomes
            .iter()
            .map(|outcome| outcome.dir_display.as_str())
            .collect();
        assert_eq!(
            dirs,
            vec![
                "~/.claude/skills",
                "~/.agents/skills",
                "~/.codex/skills",
                "~/.gemini/config/skills"
            ]
        );
        assert!(
            batch
                .reports()
                .all(|report| report.change == Change::Updated)
        );
        assert!(
            update_installed(&machine, &home.spec())
                .reports()
                .all(|report| report.change == Change::Unchanged)
        );
        assert!(!home.path(".cursor").exists() && !home.path(".copilot").exists());
    }

    #[test]
    fn codex_keeps_its_copy_in_the_deprecated_directory_up_to_date_instead_of_getting_a_second() {
        let home = Home::new();
        home.command(".local/bin/codex");
        let machine = home.machine();
        install_skill(
            &home.path(".codex/skills"),
            &home.spec().with_version("0.1.0"),
        )
        .unwrap();

        let status = agent_status(&machine, AgentId::Codex, &home.spec())
            .skill
            .unwrap();
        assert_eq!(status.dir_display, "~/.codex/skills");
        assert!(matches!(status.state, SkillState::Stale(_)));

        let batch = install_for_agent(&machine, AgentId::Codex, &home.spec()).unwrap();
        assert_eq!(batch.outcomes.len(), 1);
        assert_eq!(batch.outcomes[0].dir_display, "~/.codex/skills");
        assert!(!home.path(".agents").exists());

        // Without a copy, the new one goes to the documented directory.
        remove_for_agent(&machine, AgentId::Codex);
        let batch = install_for_agent(&machine, AgentId::Codex, &home.spec()).unwrap();
        assert_eq!(batch.outcomes[0].dir_display, "~/.agents/skills");
    }

    #[test]
    fn a_foreign_folder_stops_the_install_for_that_directory_only() {
        let home = Home::new();
        home.mkdir(".claude");
        home.command(".local/bin/codex");
        home.mkdir(".agents/skills/the-writer");
        fs::write(
            home.path(".agents/skills/the-writer/SKILL.md"),
            "---\nname: the-writer\n---\nMine.\n",
        )
        .unwrap();
        let machine = home.machine();

        let codex = agent_status(&machine, AgentId::Codex, &home.spec())
            .skill
            .unwrap();
        assert_eq!(codex.state, SkillState::Foreign);

        let batch = install_for_detected(&machine, &home.spec());
        assert!(!batch.is_ok());
        assert_eq!(batch.outcomes.len(), 2);
        assert!(batch.outcomes[0].result.is_ok());
        assert!(matches!(
            batch.outcomes[1].result,
            Err(SetupError::Foreign(_))
        ));
        let errors: Vec<_> = batch.errors().collect();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].0, home.path(".agents/skills"));
        assert!(
            batch.outcomes[1]
                .line()
                .starts_with("~/.agents/skills: failed: ")
        );
        assert_eq!(
            fs::read_to_string(home.path(".agents/skills/the-writer/SKILL.md")).unwrap(),
            "---\nname: the-writer\n---\nMine.\n"
        );

        let json = serde_json::to_value(&batch).unwrap();
        assert_eq!(json["outcomes"][0]["report"]["change"], "installed");
        assert_eq!(json["outcomes"][1]["foreign"], true);
        assert!(
            json["outcomes"][1]["error"]
                .as_str()
                .unwrap()
                .contains("was not installed by The Writer")
        );
        assert!(json["outcomes"][1].get("report").is_none());

        // Removing does not touch it either.
        assert!(remove_for_agent(&machine, AgentId::Codex).is_empty());
        assert!(
            remove_everywhere(&machine)
                .outcomes
                .iter()
                .all(|outcome| outcome.dir_display == "~/.claude/skills")
        );
        assert!(home.path(".agents/skills/the-writer/SKILL.md").exists());
    }

    #[test]
    fn removing_for_an_agent_removes_every_copy_it_reads_and_names_who_else_loses_it() {
        let home = Home::new();
        home.mkdir(".claude");
        home.command(".local/bin/codex");
        home.mkdir(".cursor");
        home.mkdir(".agents/skills/another-skill");
        let machine = home.machine();
        install_for_detected(&machine, &home.spec());

        let batch = remove_for_agent(&machine, AgentId::Cursor);
        assert!(batch.is_ok());
        let dirs: Vec<&str> = batch
            .outcomes
            .iter()
            .map(|outcome| outcome.dir_display.as_str())
            .collect();
        assert_eq!(dirs, vec!["~/.agents/skills", "~/.claude/skills"]);
        assert_eq!(
            batch.outcomes[0].agents,
            vec![AgentId::Codex, AgentId::Cursor]
        );
        assert_eq!(
            batch.outcomes[1].agents,
            vec![AgentId::ClaudeCode, AgentId::Cursor]
        );
        assert_eq!(
            batch.outcomes[0].result.as_ref().unwrap().removed,
            vec![
                home.path(".agents/skills/the-writer/SKILL.md"),
                home.path(".agents/skills/the-writer")
            ]
        );
        assert!(
            home.path(".agents/skills/another-skill").is_dir(),
            "the neighbours stay"
        );
        assert!(
            home.path(".claude/skills").is_dir(),
            "the skills directory itself stays"
        );
        for status in survey(&machine, &home.spec()) {
            assert_eq!(
                status.skill.unwrap().state,
                SkillState::NotInstalled,
                "{}",
                status.name
            );
        }
        assert!(
            remove_for_agent(&machine, AgentId::Cursor).is_empty(),
            "nothing left to remove"
        );
    }

    #[test]
    fn remove_everywhere_removes_copies_of_any_version() {
        let home = Home::new();
        home.mkdir(".claude");
        let machine = home.machine();
        install_for_agent(
            &machine,
            AgentId::ClaudeCode,
            &home.spec().with_version("0.0.1"),
        )
        .unwrap();
        install_for_agent(&machine, AgentId::VsCode, &home.spec()).unwrap();
        let batch = remove_everywhere(&machine);
        assert_eq!(batch.outcomes.len(), 2);
        assert!(
            batch
                .reports()
                .all(|report| report.change == Change::Removed)
        );
        assert!(!home.path(".claude/skills/the-writer").exists());
        assert!(!home.path(".copilot/skills/the-writer").exists());
        assert!(remove_everywhere(&machine).is_empty());
    }

    #[test]
    fn the_survey_is_json_with_keys_for_ids() {
        let home = Home::new();
        home.mkdir(".claude");
        let machine = home.machine();
        install_for_agent(&machine, AgentId::ClaudeCode, &home.spec()).unwrap();
        let json = serde_json::to_value(survey(&machine, &home.spec())).unwrap();
        assert_eq!(json[0]["id"], "claude-code");
        assert_eq!(json[0]["detected"], true);
        assert_eq!(json[0]["skill"]["state"]["state"], "current");
        assert_eq!(json[0]["skill"]["dir_display"], "~/.claude/skills");
        assert_eq!(json[0]["evidence"][0]["rule"]["home-path"], ".claude");
    }
}
