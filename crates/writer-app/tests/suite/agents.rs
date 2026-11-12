//! The Agents panel and the one-time hint that leads to it.
//!
//! Every test gives the app an agents environment that is nothing but a
//! temporary directory (`Environment::isolated`): agents are "found" by
//! what the test puts there, the skill is installed there, and the copy
//! buttons write the test platform's clipboard. Nothing here reads or
//! writes the real agent directories of whoever runs the tests; each test
//! checks that every path the panel names lies in its temporary home.

use std::path::{Path, PathBuf};

use gpui::{Modifiers, TestAppContext};
use writer_agent_setup::{AgentId, SkillState};
use writer_app::{
    actions::ShowAgents,
    agents::{self, Environment, McpForm},
    agents_view::{AgentsView, INTRODUCTION, Row},
    controls::CLOSE_CONTROL,
    menus,
    settings::keys,
    shortcuts::{Place, collect_shortcuts},
};

use super::*;

/// A temporary home directory with a `writer` tool in an app bundle whose
/// path has a space in it, as the real one has.
struct Home {
    dir: tempfile::TempDir,
}

impl Home {
    /// A home in which nothing is installed.
    fn empty() -> Self {
        let home = Self {
            dir: tempfile::tempdir().expect("a temporary directory"),
        };
        let tool = home.tool();
        std::fs::create_dir_all(tool.parent().unwrap()).unwrap();
        std::fs::write(&tool, "#!/bin/sh\n").unwrap();
        home
    }

    /// A home with the traces of Claude Code, Codex and Gemini CLI (the
    /// last two read the same skills folder).
    fn with_agents() -> Self {
        let home = Self::empty();
        std::fs::create_dir_all(home.path().join(".claude")).unwrap();
        std::fs::create_dir_all(home.path().join(".codex")).unwrap();
        std::fs::create_dir_all(home.path().join(".gemini")).unwrap();
        std::fs::write(home.path().join(".gemini/settings.json"), "{}\n").unwrap();
        home
    }

    fn path(&self) -> PathBuf {
        // The directory as the system names it (/private/var, not /var).
        self.dir.path().canonicalize().unwrap()
    }

    fn tool(&self) -> PathBuf {
        self.path()
            .join("Applications/The Writer.app/Contents/MacOS/writer")
    }

    fn environment(&self) -> Environment {
        Environment::isolated(self.path(), self.tool())
    }

    /// The skill's file in a skills folder of this home.
    fn skill_file(&self, skills_dir: &str) -> PathBuf {
        self.path().join(skills_dir).join("the-writer/SKILL.md")
    }

    /// Every file below the home, relative to it, sorted.
    fn files(&self) -> Vec<String> {
        fn walk(dir: &Path, root: &Path, out: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, root, out);
                } else {
                    out.push(path.strip_prefix(root).unwrap().display().to_string());
                }
            }
        }
        let root = self.path();
        let mut out = Vec::new();
        walk(&root, &root, &mut out);
        out.sort();
        out
    }
}

/// A workspace over a library with one document, with `home` as the
/// agents environment.
fn workspace_with<'a>(
    home: &Home,
    cx: &'a mut TestAppContext,
) -> (Entity<Workspace>, &'a mut VisualTestContext) {
    init(cx);
    let environment = home.environment();
    cx.update(|cx| agents::set_environment(environment, cx));
    let library = library(cx);
    add_document(&library, None, &simple_document("Notes", "Text."), cx);
    open_workspace(&library, cx)
}

fn panel(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Option<Entity<AgentsView>> {
    workspace.read_with(cx, |workspace, cx| {
        workspace.modal_layer().read(cx).active::<AgentsView>()
    })
}

fn open_panel(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Entity<AgentsView> {
    cx.simulate_keystrokes("cmd-alt-a");
    cx.run_until_parked();
    panel(workspace, cx).expect("the Agents panel is open")
}

fn toasts(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Vec<String> {
    workspace.read_with(cx, |w, cx| {
        w.toast_layer()
            .read(cx)
            .toasts()
            .map(|toast| toast.message.to_string())
            .collect()
    })
}

fn clipboard(cx: &mut VisualTestContext) -> Option<String> {
    cx.read_from_clipboard().and_then(|item| item.text())
}

fn click(selector: &str, cx: &mut VisualTestContext) {
    let bounds = cx
        .debug_bounds(Box::leak(selector.to_string().into_boxed_str()))
        .unwrap_or_else(|| panic!("{selector} is not drawn"));
    cx.simulate_click(bounds.center(), Modifiers::default());
    cx.run_until_parked();
}

/// The index of the row of `agent`'s skill.
fn skill_row(view: &Entity<AgentsView>, agent: AgentId, cx: &mut VisualTestContext) -> usize {
    view.read_with(cx, |view, _| {
        view.rows()
            .iter()
            .position(|row| {
                matches!(row, Row::Skill(ix) if view.model().skills[*ix].agent == Some(agent))
            })
            .unwrap_or_else(|| panic!("no skill row for {agent:?}"))
    })
}

/// The index of the row of one way to add the MCP server to `agent`.
fn mcp_row(
    view: &Entity<AgentsView>,
    agent: AgentId,
    form: McpForm,
    cx: &mut VisualTestContext,
) -> usize {
    view.read_with(cx, |view, _| {
        view.rows()
            .iter()
            .position(|row| {
                matches!(row, Row::Mcp(ix, row_form)
                    if view.model().mcp[*ix].id == agent && *row_form == form)
            })
            .unwrap_or_else(|| panic!("no {form:?} row for {agent:?}"))
    })
}

fn select(view: &Entity<AgentsView>, row: usize, cx: &mut VisualTestContext) {
    view.update(cx, |view, cx| view.select(row, cx));
    cx.run_until_parked();
}

fn skill_state(
    view: &Entity<AgentsView>,
    agent: AgentId,
    cx: &mut VisualTestContext,
) -> SkillState {
    view.read_with(cx, |view, _| {
        view.model()
            .skills
            .iter()
            .find(|skill| skill.agent == Some(agent))
            .unwrap_or_else(|| panic!("{agent:?} is not listed"))
            .state
            .clone()
    })
}

/// Every path the panel names lies in the temporary home.
fn assert_stays_in(home: &Home, view: &Entity<AgentsView>, cx: &mut VisualTestContext) {
    let root = home.path();
    view.read_with(cx, |view, _| {
        let model = view.model();
        assert!(model.tool.starts_with(&root), "{:?}", model.tool);
        for skill in &model.skills {
            assert!(skill.directory.starts_with(&root), "{:?}", skill.directory);
            assert!(
                skill.skills_dir.starts_with(&root),
                "{:?}",
                skill.skills_dir
            );
        }
        for agent in &model.mcp {
            if let Some(file) = agent
                .config
                .as_ref()
                .and_then(|config| config.file.as_ref())
            {
                assert!(file.starts_with(&root), "{file:?}");
            }
        }
        if let Some(report) = view.report() {
            for path in &report.paths {
                assert!(path.starts_with(&root), "{path:?}");
            }
        }
    });
}

#[gpui::test]
fn the_panel_opens_from_the_key_the_palette_and_the_menu_bar(cx: &mut TestAppContext) {
    let home = Home::with_agents();
    let (workspace, vcx) = workspace_with(&home, cx);
    assert!(editor_focused(&workspace, vcx));

    // Its key. Escape closes it and the keyboard is back in the text.
    let view = open_panel(&workspace, vcx);
    assert!(!editor_focused(&workspace, vcx));
    assert_stays_in(&home, &view, vcx);
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    assert!(panel(&workspace, vcx).is_none());
    assert!(editor_focused(&workspace, vcx));

    // The same key closes it too.
    open_panel(&workspace, vcx);
    vcx.simulate_keystrokes("cmd-alt-a");
    vcx.run_until_parked();
    assert!(panel(&workspace, vcx).is_none());
    assert!(editor_focused(&workspace, vcx));

    // The command palette: "Agents", and by the name of an agent.
    for query in ["agents", "claude", "codex"] {
        vcx.simulate_keystrokes("cmd-shift-p");
        vcx.simulate_input(query);
        vcx.simulate_keystrokes("enter");
        vcx.run_until_parked();
        assert!(panel(&workspace, vcx).is_some(), "the palette, {query:?}");
        vcx.simulate_keystrokes("escape");
        vcx.run_until_parked();
        assert!(panel(&workspace, vcx).is_none());
    }

    // The menu bar: The Writer > Agents…, which is the same action.
    let all = menus::app_menus();
    let app_menu = menus::menu_actions(&all[..1]);
    let item = app_menu
        .iter()
        .find(|(name, _)| name == "Agents…")
        .expect("The Writer > Agents…");
    assert!(item.1.partial_eq(&ShowAgents));
    vcx.dispatch_action(ShowAgents);
    vcx.run_until_parked();
    assert!(panel(&workspace, vcx).is_some());
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();

    // The shortcuts overlay lists the key that opens it and the keys in it.
    vcx.simulate_keystrokes("cmd-/");
    vcx.run_until_parked();
    assert!(
        workspace
            .read_with(vcx, |workspace, cx| workspace.shortcuts_list(cx))
            .is_some(),
        "the list is open"
    );
    let listed: Vec<(String, Place, String)> = vcx.update(|_, cx| {
        collect_shortcuts(cx)
            .iter()
            .map(|shortcut| {
                (
                    shortcut.title.to_string(),
                    shortcut.place,
                    shortcut.keys.clone(),
                )
            })
            .collect()
    });
    assert!(
        listed.contains(&("Agents".to_string(), Place::Anywhere, "⌥⌘A".to_string())),
        "{listed:?}"
    );
    let in_panel: Vec<_> = listed
        .iter()
        .filter(|(_, place, _)| *place == Place::Agents)
        .map(|(title, _, keys)| (title.as_str(), keys.as_str()))
        .collect();
    for expected in [
        ("Activate", "↩"),
        ("Remove skill", "⌫"),
        ("Copy row", "⌘C"),
        ("Open documentation", "O"),
    ] {
        assert!(in_panel.contains(&expected), "{expected:?} in {in_panel:?}");
    }
}

#[gpui::test]
fn the_panel_says_what_this_is_and_shows_the_tool(cx: &mut TestAppContext) {
    let home = Home::with_agents();
    let (workspace, vcx) = workspace_with(&home, cx);
    let view = open_panel(&workspace, vcx);

    // Two sentences, with the rule in them.
    assert_eq!(INTRODUCTION.matches(". ").count() + 1, 2, "{INTRODUCTION}");
    assert!(INTRODUCTION.contains("read at any time"));
    assert!(INTRODUCTION.contains("only change things while The Writer is closed"));

    // The first row is the tool, with its real path; it is there.
    let (rows, model) = view.read_with(vcx, |view, _| (view.rows().to_vec(), view.model().clone()));
    assert_eq!(rows[0], Row::Tool);
    assert_eq!(model.tool, home.tool());
    assert!(model.tool_found);
    assert_eq!(model.problem, None);
    assert_eq!(
        view.read_with(vcx, |view, _| view.selected_row()),
        Some(Row::Tool)
    );
    assert_eq!(
        view.read_with(vcx, |view, _| view.button_labels(Row::Tool)),
        ["Copy Path"]
    );

    // Enter copies the path, to the clipboard of the test platform.
    assert_eq!(clipboard(vcx), None);
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert_eq!(clipboard(vcx), Some(home.tool().display().to_string()));
    assert!(
        toasts(&workspace, vcx).contains(&"Copied the path of the tool.".to_string()),
        "{:?}",
        toasts(&workspace, vcx)
    );
    // The panel stays open and keeps the keyboard.
    assert!(panel(&workspace, vcx).is_some());

    // The button does the same with the mouse.
    vcx.write_to_clipboard(gpui::ClipboardItem::new_string("something else".into()));
    click("agents-copy-0", vcx);
    assert_eq!(clipboard(vcx), Some(home.tool().display().to_string()));
    assert!(panel(&workspace, vcx).is_some());
}

#[gpui::test]
fn a_missing_tool_is_said(cx: &mut TestAppContext) {
    let home = Home::with_agents();
    std::fs::remove_file(home.tool()).unwrap();
    let (workspace, vcx) = workspace_with(&home, cx);
    let view = open_panel(&workspace, vcx);
    let model = view.read_with(vcx, |view, _| view.model().clone());
    assert_eq!(model.tool, home.tool());
    assert!(!model.tool_found);
    assert!(vcx.debug_bounds("agents-tool-missing").is_some());
}

#[gpui::test]
fn the_agents_found_are_listed_and_the_others_named(cx: &mut TestAppContext) {
    let home = Home::with_agents();
    let (workspace, vcx) = workspace_with(&home, cx);
    let view = open_panel(&workspace, vcx);
    assert_stays_in(&home, &view, vcx);
    let model = view.read_with(vcx, |view, _| view.model().clone());

    // Found: the three whose traces are in the home, each with the folder
    // the skill goes to and its state.
    let found: Vec<_> = model
        .skills
        .iter()
        .map(|skill| {
            (
                skill.name.as_str(),
                skill.directory_display.as_str(),
                agents::skill_state_label(&skill.state),
            )
        })
        .collect();
    assert_eq!(
        found,
        [
            (
                "Claude Code",
                "~/.claude/skills/the-writer",
                "Not installed"
            ),
            ("Codex", "~/.agents/skills/the-writer", "Not installed"),
            ("Gemini CLI", "~/.agents/skills/the-writer", "Not installed"),
        ]
    );
    assert_eq!(
        model.skills[0].directory,
        home.path().join(".claude/skills/the-writer")
    );
    // Two of them read one folder, and the row says so.
    assert_eq!(model.skills[0].detail, None);
    assert_eq!(
        model.skills[1].detail.as_deref(),
        Some("Gemini CLI reads this folder too.")
    );

    // Not found: named quietly, so that one sees what is supported.
    for name in ["Cursor", "OpenCode", "Zed"] {
        assert!(
            model.not_found.iter().any(|listed| listed == name),
            "{name} in {:?}",
            model.not_found
        );
    }
    assert!(!model.not_found.iter().any(|name| name == "Claude Code"));
    assert!(vcx.debug_bounds("agents-not-found").is_some());

    // The rows: the tool, install for all, one per agent, then MCP.
    let rows = view.read_with(vcx, |view, _| view.rows().to_vec());
    assert_eq!(
        rows[..5],
        [
            Row::Tool,
            Row::InstallAll,
            Row::Skill(0),
            Row::Skill(1),
            Row::Skill(2)
        ]
    );
    assert!(matches!(rows[5], Row::Mcp(0, _)));
    assert_eq!(
        view.read_with(vcx, |view, _| view.button_labels(Row::Skill(0))),
        ["Install"]
    );

    // Nothing was written by looking.
    assert_eq!(
        home.files(),
        [
            ".gemini/settings.json",
            "Applications/The Writer.app/Contents/MacOS/writer"
        ]
    );
}

#[gpui::test]
fn a_home_without_agents_says_so(cx: &mut TestAppContext) {
    let home = Home::empty();
    let (workspace, vcx) = workspace_with(&home, cx);
    let view = open_panel(&workspace, vcx);
    let (model, rows) = view.read_with(vcx, |view, _| (view.model().clone(), view.rows().to_vec()));
    assert!(model.skills.is_empty());
    assert!(model.not_found.iter().any(|name| name == "Claude Code"));
    assert!(!rows.contains(&Row::InstallAll));
    assert!(vcx.debug_bounds("agents-none-found").is_some());
    // The MCP commands are shown all the same: an agent may be installed
    // where the app does not look.
    assert!(rows.iter().any(|row| matches!(row, Row::Mcp(..))));
    assert!(model.mcp.iter().all(|agent| !agent.detected));
}

#[gpui::test]
fn the_skill_is_installed_and_removed_from_the_keyboard(cx: &mut TestAppContext) {
    let home = Home::with_agents();
    let (workspace, vcx) = workspace_with(&home, cx);
    let view = open_panel(&workspace, vcx);
    let file = home.skill_file(".claude/skills");
    assert!(!file.exists());

    // Down to Claude Code's row (the tool, install for all, then it).
    vcx.simulate_keystrokes("down down");
    let row = skill_row(&view, AgentId::ClaudeCode, vcx);
    assert_eq!(
        view.read_with(vcx, |view, _| view.selected_row()),
        Some(Row::Skill(0))
    );
    assert_eq!(
        view.read_with(vcx, |view, _| view.rows()[row]),
        Row::Skill(0)
    );

    // Backspace has nothing to remove yet.
    vcx.simulate_keystrokes("backspace");
    vcx.run_until_parked();
    assert!(view.read_with(vcx, |view, _| view.report().is_none()));

    // Enter installs. The report names exactly what was written.
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert!(file.is_file());
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.starts_with("---\nname: the-writer\n"), "{text}");
    assert!(
        text.contains(&home.tool().display().to_string()),
        "the skill names the real path of the tool"
    );
    let report = view.read_with(vcx, |view, _| view.report().cloned().unwrap());
    assert!(report.ok);
    assert!(
        report
            .headline
            .starts_with("Installed the skill for Claude Code."),
        "{}",
        report.headline
    );
    assert_eq!(
        report.lines,
        [
            "Created the folder ~/.claude/skills",
            "Created the folder ~/.claude/skills/the-writer",
            "Wrote ~/.claude/skills/the-writer/SKILL.md",
        ]
    );
    assert_eq!(
        report.paths,
        [
            home.path().join(".claude/skills"),
            home.path().join(".claude/skills/the-writer"),
            file.clone(),
        ]
    );
    // And when the agent will notice it, as its documentation says.
    assert_eq!(
        report.note.as_deref(),
        AgentId::ClaudeCode.agent().skills.pickup
    );
    assert!(report.note.is_some());
    assert_stays_in(&home, &view, vcx);
    assert!(vcx.debug_bounds("agents-report").is_some());
    assert_eq!(
        skill_state(&view, AgentId::ClaudeCode, vcx),
        SkillState::Current
    );
    // The others were not touched.
    assert_eq!(
        skill_state(&view, AgentId::Codex, vcx),
        SkillState::NotInstalled
    );
    assert!(!home.skill_file(".agents/skills").exists());
    // The selection stayed on the row, which now offers to remove.
    assert_eq!(
        view.read_with(vcx, |view, _| view.selected_row()),
        Some(Row::Skill(0))
    );
    assert_eq!(
        view.read_with(vcx, |view, _| view.button_labels(Row::Skill(0))),
        ["Remove"]
    );

    // Enter never removes: it does nothing on an installed skill.
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert!(file.is_file());

    // Backspace removes the file and the folder that was made for it, and
    // leaves the agent's skills folder.
    vcx.simulate_keystrokes("backspace");
    vcx.run_until_parked();
    assert!(!file.exists());
    assert!(!file.parent().unwrap().exists());
    assert!(home.path().join(".claude/skills").is_dir());
    let report = view.read_with(vcx, |view, _| view.report().cloned().unwrap());
    assert!(report.ok);
    assert_eq!(report.headline, "Removed the skill for Claude Code.");
    assert_eq!(
        report.lines,
        [
            "Removed ~/.claude/skills/the-writer/SKILL.md",
            "Removed ~/.claude/skills/the-writer",
        ]
    );
    assert_eq!(
        skill_state(&view, AgentId::ClaudeCode, vcx),
        SkillState::NotInstalled
    );
    assert_stays_in(&home, &view, vcx);
}

#[gpui::test]
fn install_for_all_and_the_buttons_work_with_the_mouse(cx: &mut TestAppContext) {
    let home = Home::with_agents();
    let (workspace, vcx) = workspace_with(&home, cx);
    let view = open_panel(&workspace, vcx);

    // A click on a row selects it; nothing else happens.
    let codex = skill_row(&view, AgentId::Codex, vcx);
    click(&format!("agents-row-{codex}"), vcx);
    assert_eq!(
        view.read_with(vcx, |view, _| view.selected_row()),
        Some(Row::Skill(1))
    );
    assert!(view.read_with(vcx, |view, _| view.report().is_none()));
    assert!(panel(&workspace, vcx).is_some());

    // "Install for All": one copy per folder covers the three agents.
    click("agents-install-all-1", vcx);
    let claude = home.skill_file(".claude/skills");
    let shared = home.skill_file(".agents/skills");
    assert!(claude.is_file());
    assert!(shared.is_file());
    let report = view.read_with(vcx, |view, _| view.report().cloned().unwrap());
    assert!(report.ok);
    assert_eq!(
        report.headline,
        "Installed the skill for Claude Code, Codex and Gemini CLI."
    );
    assert!(report.paths.contains(&claude) && report.paths.contains(&shared));
    assert!(
        report
            .lines
            .contains(&"Wrote ~/.agents/skills/the-writer/SKILL.md".to_string()),
        "{:?}",
        report.lines
    );
    assert_stays_in(&home, &view, vcx);
    for agent in [AgentId::ClaudeCode, AgentId::Codex, AgentId::GeminiCli] {
        assert_eq!(
            skill_state(&view, agent, vcx),
            SkillState::Current,
            "{agent:?}"
        );
    }
    assert_eq!(
        home.files(),
        [
            ".agents/skills/the-writer/SKILL.md",
            ".claude/skills/the-writer/SKILL.md",
            ".gemini/settings.json",
            "Applications/The Writer.app/Contents/MacOS/writer"
        ]
    );

    // A second time changes nothing and says so.
    let before = std::fs::read_to_string(&shared).unwrap();
    click("agents-install-all-1", vcx);
    let report = view.read_with(vcx, |view, _| view.report().cloned().unwrap());
    assert!(report.ok);
    assert!(
        report.headline.contains("already up to date"),
        "{}",
        report.headline
    );
    assert!(report.paths.is_empty());
    assert_eq!(std::fs::read_to_string(&shared).unwrap(), before);

    // "Remove" on Codex's row takes the shared copy away, and the report
    // says who else read it.
    click(&format!("agents-remove-{codex}"), vcx);
    assert!(!shared.exists());
    assert!(claude.is_file());
    let report = view.read_with(vcx, |view, _| view.report().cloned().unwrap());
    assert!(report.ok);
    assert_eq!(
        report.headline,
        "Removed the skill for Codex. Gemini CLI read the same folder and no longer has it either."
    );
    assert_eq!(
        skill_state(&view, AgentId::GeminiCli, vcx),
        SkillState::NotInstalled
    );
    assert_eq!(
        skill_state(&view, AgentId::ClaudeCode, vcx),
        SkillState::Current
    );

    // "Install" on one row.
    click(&format!("agents-install-{codex}"), vcx);
    assert!(shared.is_file());
    let report = view.read_with(vcx, |view, _| view.report().cloned().unwrap());
    assert!(
        report.headline.starts_with(
            "Installed the skill for Codex. Gemini CLI reads the same folder and has it too."
        ),
        "{}",
        report.headline
    );
    assert_stays_in(&home, &view, vcx);

    // The close control closes the panel; a click outside does too.
    click(CLOSE_CONTROL, vcx);
    assert!(panel(&workspace, vcx).is_none());
    assert!(editor_focused(&workspace, vcx));
    open_panel(&workspace, vcx);
    vcx.simulate_click(
        gpui::point(gpui::px(6.), gpui::px(300.)),
        Modifiers::default(),
    );
    vcx.run_until_parked();
    assert!(panel(&workspace, vcx).is_none());
}

#[gpui::test]
fn removing_names_only_the_agents_that_lose_the_skill(cx: &mut TestAppContext) {
    // Cursor reads the shared folder and Claude's folder.
    let home = Home::with_agents();
    std::fs::create_dir_all(home.path().join(".cursor")).unwrap();
    assert!(agents::install_skill_for_all(&home.environment()).ok);
    let (workspace, vcx) = workspace_with(&home, cx);
    let view = open_panel(&workspace, vcx);
    let row = skill_row(&view, AgentId::Codex, vcx);
    select(&view, row, vcx);
    vcx.simulate_keystrokes("backspace");
    vcx.run_until_parked();
    assert!(!home.skill_file(".agents/skills").exists());
    let report = view.read_with(vcx, |view, _| view.report().cloned().unwrap());
    // Cursor still finds the skill in ~/.claude/skills.
    assert_eq!(
        report.headline,
        "Removed the skill for Codex. Gemini CLI read the same folder and no longer has it either."
    );
    assert_eq!(
        skill_state(&view, AgentId::Cursor, vcx),
        SkillState::Current
    );
    assert_eq!(
        skill_state(&view, AgentId::GeminiCli, vcx),
        SkillState::NotInstalled
    );
}

#[gpui::test]
fn a_skill_from_another_copy_of_the_app_needs_updating(cx: &mut TestAppContext) {
    let home = Home::with_agents();
    // Installed by a copy of the app that lives somewhere else.
    let other_tool = home.path().join("Old/The Writer.app/Contents/MacOS/writer");
    std::fs::create_dir_all(other_tool.parent().unwrap()).unwrap();
    std::fs::write(&other_tool, "#!/bin/sh\n").unwrap();
    let report = agents::install_skill_for_all(&Environment::isolated(home.path(), &other_tool));
    assert!(report.ok, "{report:?}");
    let file = home.skill_file(".claude/skills");
    assert!(
        std::fs::read_to_string(&file)
            .unwrap()
            .contains("/Old/The Writer.app/")
    );

    let (workspace, vcx) = workspace_with(&home, cx);
    let view = open_panel(&workspace, vcx);
    assert!(matches!(
        skill_state(&view, AgentId::ClaudeCode, vcx),
        SkillState::Stale(_)
    ));
    let skill = view.read_with(vcx, |view, _| view.model().skills[0].clone());
    assert_eq!(agents::skill_state_label(&skill.state), "Needs updating");
    assert!(
        skill
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("another copy of the tool")),
        "{:?}",
        skill.detail
    );
    assert_eq!(
        view.read_with(vcx, |view, _| view.button_labels(Row::Skill(0))),
        ["Update", "Remove"]
    );

    // Enter updates: the file now names this copy's tool.
    let row = skill_row(&view, AgentId::ClaudeCode, vcx);
    select(&view, row, vcx);
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains(&home.tool().display().to_string()));
    assert!(!text.contains("/Old/The Writer.app/"));
    let report = view.read_with(vcx, |view, _| view.report().cloned().unwrap());
    assert!(report.ok);
    assert!(
        report
            .headline
            .starts_with("Updated the skill for Claude Code."),
        "{}",
        report.headline
    );
    assert_eq!(report.lines, ["Wrote ~/.claude/skills/the-writer/SKILL.md"]);
    assert_eq!(
        skill_state(&view, AgentId::ClaudeCode, vcx),
        SkillState::Current
    );
    assert_stays_in(&home, &view, vcx);
}

#[gpui::test]
fn a_folder_somebody_else_made_is_left_alone(cx: &mut TestAppContext) {
    let home = Home::with_agents();
    let theirs = home.path().join(".claude/skills/the-writer/SKILL.md");
    std::fs::create_dir_all(theirs.parent().unwrap()).unwrap();
    std::fs::write(&theirs, "---\nname: the-writer\n---\nMy own notes.\n").unwrap();

    let (workspace, vcx) = workspace_with(&home, cx);
    let view = open_panel(&workspace, vcx);
    assert_eq!(
        skill_state(&view, AgentId::ClaudeCode, vcx),
        SkillState::Foreign
    );
    let skill = view.read_with(vcx, |view, _| view.model().skills[0].clone());
    assert_eq!(
        agents::skill_state_label(&skill.state),
        "Another folder is in the way"
    );
    // No button on the row; enter and backspace do nothing.
    assert!(
        view.read_with(vcx, |view, _| view.button_labels(Row::Skill(0)))
            .is_empty()
    );
    let row = skill_row(&view, AgentId::ClaudeCode, vcx);
    select(&view, row, vcx);
    vcx.simulate_keystrokes("enter backspace");
    vcx.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(&theirs).unwrap(),
        "---\nname: the-writer\n---\nMy own notes.\n"
    );

    // Install for all installs for the others and says what was in the way.
    click("agents-install-all-1", vcx);
    let report = view.read_with(vcx, |view, _| view.report().cloned().unwrap());
    assert!(!report.ok);
    assert!(
        report
            .lines
            .iter()
            .any(|line| line.contains("was not installed by The Writer")),
        "{:?}",
        report.lines
    );
    assert_eq!(
        std::fs::read_to_string(&theirs).unwrap(),
        "---\nname: the-writer\n---\nMy own notes.\n"
    );
    assert!(home.skill_file(".agents/skills").is_file());
    assert_stays_in(&home, &view, vcx);
}

#[gpui::test]
fn a_copy_no_agent_reads_any_more_can_be_removed(cx: &mut TestAppContext) {
    let home = Home::with_agents();
    assert!(agents::install_skill_for_all(&home.environment()).ok);
    // Codex and Gemini CLI are gone; their shared folder still has the skill.
    std::fs::remove_dir_all(home.path().join(".codex")).unwrap();
    std::fs::remove_dir_all(home.path().join(".gemini")).unwrap();
    let shared = home.skill_file(".agents/skills");
    assert!(shared.is_file());

    let (workspace, vcx) = workspace_with(&home, cx);
    let view = open_panel(&workspace, vcx);
    let skills = view.read_with(vcx, |view, _| view.model().skills.clone());
    assert_eq!(skills.len(), 2);
    assert_eq!(skills[0].agent, Some(AgentId::ClaudeCode));
    assert_eq!(skills[1].agent, None);
    assert_eq!(skills[1].directory_display, "~/.agents/skills/the-writer");
    assert_eq!(
        view.read_with(vcx, |view, _| view.button_labels(Row::Skill(1))),
        ["Remove"]
    );
    let row = view.read_with(vcx, |view, _| {
        view.rows()
            .iter()
            .position(|row| *row == Row::Skill(1))
            .unwrap()
    });
    select(&view, row, vcx);
    vcx.simulate_keystrokes("backspace");
    vcx.run_until_parked();
    assert!(!shared.exists());
    assert!(home.skill_file(".claude/skills").is_file());
    let report = view.read_with(vcx, |view, _| view.report().cloned().unwrap());
    assert!(report.ok, "{report:?}");
    assert_eq!(view.read_with(vcx, |view, _| view.model().skills.len()), 1);
    assert_stays_in(&home, &view, vcx);
}

#[gpui::test]
fn the_mcp_commands_carry_the_real_path_and_are_copied(cx: &mut TestAppContext) {
    let home = Home::with_agents();
    let (workspace, vcx) = workspace_with(&home, cx);
    let view = open_panel(&workspace, vcx);
    let model = view.read_with(vcx, |view, _| view.model().clone());
    let tool = home.tool().display().to_string();
    assert!(
        tool.contains(' '),
        "the path has a space, as the real one has"
    );

    // The agents found come first.
    let names: Vec<_> = model.mcp.iter().map(|agent| agent.name.as_str()).collect();
    assert_eq!(names[..3], ["Claude Code", "Codex", "Gemini CLI"]);
    assert!(model.mcp[..3].iter().all(|agent| agent.detected));
    assert!(model.mcp[3..].iter().all(|agent| !agent.detected));
    for name in ["Claude desktop app", "Cursor", "OpenCode", "Zed"] {
        assert!(names.contains(&name), "{name} in {names:?}");
    }

    // Every one has a command or an entry with the path in it, in the form
    // the shell or the file needs, and a page to read.
    let quoted = format!("'{tool}'");
    let literal = format!("\"{tool}\"");
    for agent in &model.mcp {
        assert!(
            agent.command.is_some() || agent.config.is_some(),
            "{}",
            agent.name
        );
        if let Some(command) = &agent.command {
            assert!(command.contains(&quoted), "{}: {command}", agent.name);
            assert!(command.ends_with(" mcp"), "{}: {command}", agent.name);
        }
        if let Some(config) = &agent.config {
            assert!(
                config.text.contains(&literal),
                "{}: {}",
                agent.name,
                config.text
            );
            assert!(config.text.contains("the-writer"), "{}", agent.name);
            assert!(!config.place.is_empty() && !config.placement.is_empty());
        }
        assert!(agent.docs_url.starts_with("https://"), "{}", agent.name);
    }
    let claude = &model.mcp[0];
    assert_eq!(
        claude.command.as_deref(),
        Some(format!("claude mcp add --scope user the-writer -- {quoted} mcp").as_str())
    );
    // The file an entry belongs in is named, below the home that was given.
    let cursor = model
        .mcp
        .iter()
        .find(|agent| agent.id == AgentId::Cursor)
        .unwrap();
    let config = cursor.config.as_ref().unwrap();
    assert_eq!(config.file, Some(home.path().join(".cursor/mcp.json")));
    assert_eq!(config.place, "In ~/.cursor/mcp.json");

    // Keyboard: enter on a command row copies the command.
    let row = mcp_row(&view, AgentId::ClaudeCode, McpForm::Command, vcx);
    select(&view, row, vcx);
    assert_eq!(
        view.read_with(vcx, |view, _| view.button_labels(view.rows()[row])),
        ["Copy", "Documentation"]
    );
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert_eq!(clipboard(vcx), claude.command.clone());
    assert!(
        toasts(&workspace, vcx).contains(&"Copied the command for Claude Code.".to_string()),
        "{:?}",
        toasts(&workspace, vcx)
    );

    // cmd-c on a configuration row copies the entry, which is valid JSON
    // that runs this tool.
    let row = mcp_row(&view, AgentId::Cursor, McpForm::Config, vcx);
    select(&view, row, vcx);
    vcx.simulate_keystrokes("cmd-c");
    vcx.run_until_parked();
    let copied = clipboard(vcx).unwrap();
    assert_eq!(copied, config.text);
    let json: serde_json::Value = serde_json::from_str(&copied).expect("valid JSON");
    assert_eq!(json["mcpServers"]["the-writer"]["command"], tool.as_str());
    assert_eq!(json["mcpServers"]["the-writer"]["args"][0], "mcp");

    // Mouse: the copy button of a row.
    let row = mcp_row(&view, AgentId::Codex, McpForm::Command, vcx);
    select(&view, row, vcx);
    vcx.run_until_parked();
    click(&format!("agents-copy-{row}"), vcx);
    assert_eq!(
        clipboard(vcx),
        Some(format!("codex mcp add the-writer -- {quoted} mcp"))
    );

    // The documentation: "o" on the row, and the button.
    assert_eq!(vcx.opened_url(), None);
    vcx.simulate_keystrokes("o");
    vcx.run_until_parked();
    let codex = model
        .mcp
        .iter()
        .find(|agent| agent.id == AgentId::Codex)
        .unwrap();
    assert_eq!(vcx.opened_url(), Some(codex.docs_url.clone()));
    let row = mcp_row(&view, AgentId::ClaudeCode, McpForm::Command, vcx);
    select(&view, row, vcx);
    vcx.run_until_parked();
    click(&format!("agents-documentation-{row}"), vcx);
    assert_eq!(vcx.opened_url(), Some(claude.docs_url.clone()));

    // None of this wrote anything: the app does not edit an agent's
    // configuration.
    assert_eq!(
        home.files(),
        [
            ".gemini/settings.json",
            "Applications/The Writer.app/Contents/MacOS/writer"
        ]
    );
}

#[gpui::test]
fn the_keyboard_walks_every_row(cx: &mut TestAppContext) {
    let home = Home::with_agents();
    let (workspace, vcx) = workspace_with(&home, cx);
    let view = open_panel(&workspace, vcx);
    let rows = view.read_with(vcx, |view, _| view.rows().to_vec());
    assert!(rows.len() > 12, "{rows:?}");
    let selected = |cx: &mut VisualTestContext| view.read_with(cx, |view, _| view.selected_row());

    assert_eq!(selected(vcx), Some(rows[0]));
    vcx.simulate_keystrokes("up");
    assert_eq!(selected(vcx), Some(rows[0]), "the list starts there");
    for row in &rows[1..] {
        vcx.simulate_keystrokes("down");
        assert_eq!(selected(vcx), Some(*row));
    }
    vcx.simulate_keystrokes("down");
    assert_eq!(selected(vcx), rows.last().copied(), "the list ends there");
    // The last row was scrolled into view: its copy button can be clicked.
    vcx.run_until_parked();
    let last = rows.len() - 1;
    let list = vcx.debug_bounds("agents-list").expect("the list is drawn");
    let button = vcx
        .debug_bounds(Box::leak(format!("agents-copy-{last}").into_boxed_str()))
        .expect("the last row is drawn");
    assert!(
        button.bottom() <= list.bottom() && button.top() >= list.top(),
        "{button:?} in {list:?}"
    );
    vcx.simulate_keystrokes("home");
    assert_eq!(selected(vcx), Some(rows[0]));
    vcx.simulate_keystrokes("end ctrl-p");
    assert_eq!(selected(vcx), Some(rows[last - 1]));
    vcx.simulate_keystrokes("cmd-up ctrl-n");
    assert_eq!(selected(vcx), Some(rows[1]));
    // Typing does not reach the text behind the panel.
    vcx.simulate_input("x");
    vcx.run_until_parked();
    assert_eq!(editor_text(&workspace, vcx), "Notes\nText.");
    assert!(panel(&workspace, vcx).is_some());
}

#[gpui::test]
fn a_scripted_run_does_not_write_the_clipboard(cx: &mut TestAppContext) {
    let home = Home::with_agents();
    init(cx);
    let environment = Environment {
        clipboard: false,
        ..home.environment()
    };
    cx.update(|cx| agents::set_environment(environment, cx));
    let library = library(cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    open_panel(&workspace, vcx);
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert_eq!(clipboard(vcx), None);
    assert!(
        toasts(&workspace, vcx)
            .iter()
            .any(|toast| toast.starts_with("Not copied in a scripted run")),
        "{:?}",
        toasts(&workspace, vcx)
    );
}

#[gpui::test]
fn without_an_environment_the_panel_lists_and_touches_nothing(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    let view = open_panel(&workspace, vcx);
    let (model, rows) = view.read_with(vcx, |view, _| (view.model().clone(), view.rows().to_vec()));
    assert!(rows.is_empty());
    assert!(model.skills.is_empty() && model.mcp.is_empty());
    // The keys do nothing and nothing breaks.
    vcx.simulate_keystrokes("down enter backspace cmd-c o");
    vcx.run_until_parked();
    assert_eq!(clipboard(vcx), None);
    assert_eq!(vcx.opened_url(), None);
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    assert!(panel(&workspace, vcx).is_none());
}

// ----- the hint ------------------------------------------------------------------

fn hint_toasts(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> usize {
    toasts(workspace, cx)
        .iter()
        .filter(|toast| toast.as_str() == agents::HINT)
        .count()
}

#[gpui::test]
fn the_hint_is_shown_once_and_leads_to_the_panel(cx: &mut TestAppContext) {
    let home = Home::with_agents();
    init(cx);
    let environment = home.environment();
    cx.update(|cx| {
        agents::set_environment(environment, cx);
        agents::enable_hint(cx);
    });
    let library = library(cx);
    add_document(&library, None, &simple_document("Notes", "Text."), cx);
    assert!(!library.read_with(cx, |library, _| {
        library.setting_or(keys::AGENTS_HINT_SHOWN, false)
    }));

    // The first launch: a toast, not a dialog. The keyboard stays in the
    // text and nothing is in the way.
    let (workspace, vcx) = open_workspace(&library, cx);
    assert_eq!(hint_toasts(&workspace, vcx), 1);
    assert!(editor_focused(&workspace, vcx));
    assert!(!workspace.read_with(vcx, |w, cx| w.modal_layer().read(cx).has_modal()));
    assert!(library.read_with(vcx, |library, _| {
        library.setting_or(keys::AGENTS_HINT_SHOWN, false)
    }));
    let (id, label) = workspace.read_with(vcx, |w, cx| {
        let layer = w.toast_layer().read(cx);
        let ids = layer.ids();
        let toast = layer
            .toasts()
            .position(|toast| toast.message.as_ref() == agents::HINT)
            .unwrap();
        let label = layer
            .toasts()
            .nth(toast)
            .unwrap()
            .action_label()
            .map(|label| label.to_string());
        (ids[toast], label)
    });
    assert_eq!(label.as_deref(), Some(agents::HINT_BUTTON));

    // Its button opens the panel.
    let layer = workspace.read_with(vcx, |w, _| w.toast_layer().clone());
    let ran =
        vcx.update(|window, cx| layer.update(cx, |layer, cx| layer.run_action(id, window, cx)));
    assert!(ran);
    vcx.run_until_parked();
    assert!(panel(&workspace, vcx).is_some());
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    vcx.update(|window, _| window.remove_window());
    vcx.run_until_parked();

    // The next launch on this library: never again.
    let (workspace, vcx) = open_workspace(&library, cx);
    assert_eq!(hint_toasts(&workspace, vcx), 0);
}

#[gpui::test]
fn the_hint_that_was_only_seen_does_not_come_back(cx: &mut TestAppContext) {
    init(cx);
    cx.update(agents::enable_hint);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.db");
    let open = |cx: &mut TestAppContext| {
        let library = cx.new(|_| Library::open(&path).expect("the library opens"));
        library.update(cx, |library, cx| {
            library
                .set_setting(keys::WELCOME_CREATED, &true, cx)
                .unwrap();
        });
        library
    };

    // Seen, never pressed, and the app is closed.
    let library = open(cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    assert_eq!(hint_toasts(&workspace, vcx), 1);
    // It goes away by itself after a while.
    vcx.executor()
        .advance_clock(std::time::Duration::from_secs(30));
    vcx.run_until_parked();
    assert_eq!(hint_toasts(&workspace, vcx), 0);
    vcx.update(|window, _| window.remove_window());
    vcx.run_until_parked();
    drop(library);

    // A later launch from the file on disk does not show it.
    let library = open(cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    assert_eq!(hint_toasts(&workspace, vcx), 0);
}

#[gpui::test]
fn no_hint_where_it_was_not_switched_on(cx: &mut TestAppContext) {
    // Tests and scripted runs: `main` switches it on, nothing else does.
    init(cx);
    let library = library(cx);
    let (workspace, vcx) = open_workspace(&library, cx);
    assert_eq!(hint_toasts(&workspace, vcx), 0);
    assert!(!library.read_with(vcx, |library, _| {
        library.setting_or(keys::AGENTS_HINT_SHOWN, false)
    }));
}

#[test]
fn the_tool_is_looked_for_beside_the_executable() {
    let tool = agents::bundled_tool_path().expect("the executable has a folder");
    assert!(tool.is_absolute());
    assert_eq!(tool.file_name().unwrap(), agents::TOOL_NAME);
    let exe = std::env::current_exe().unwrap().canonicalize().unwrap();
    assert_eq!(tool.parent(), exe.parent());

    // A path that will not last is said to be one.
    assert!(
        agents::temporary_place(Path::new(
            "/private/var/folders/x/AppTranslocation/ABC/d/The Writer.app/Contents/MacOS/writer"
        ))
        .is_some()
    );
    assert!(
        agents::temporary_place(Path::new(
            "/Volumes/The Writer/The Writer.app/Contents/MacOS/writer"
        ))
        .is_some()
    );
    assert!(
        agents::temporary_place(Path::new(
            "/Applications/The Writer.app/Contents/MacOS/writer"
        ))
        .is_none()
    );
}

/// Wrapped text in a row once made the row thousands of pixels tall (every
/// configuration entry, which has several lines): no row is taller than
/// its text needs.
#[gpui::test]
fn no_row_is_taller_than_its_text(cx: &mut TestAppContext) {
    let home = Home::with_agents();
    // A skill row with a long explanation under it, too.
    let theirs = home.path().join(".claude/skills/the-writer/SKILL.md");
    std::fs::create_dir_all(theirs.parent().unwrap()).unwrap();
    std::fs::write(&theirs, "Mine.\n").unwrap();
    let (workspace, vcx) = workspace_with(&home, cx);
    let view = open_panel(&workspace, vcx);
    let rows = view.read_with(vcx, |view, _| view.rows().to_vec());
    for (ix, row) in rows.iter().enumerate() {
        select(&view, ix, vcx);
        let bounds = vcx
            .debug_bounds(Box::leak(format!("agents-row-{ix}").into_boxed_str()))
            .unwrap_or_else(|| panic!("row {ix} is not drawn"));
        let lines = view.read_with(vcx, |view, _| {
            view.copy_text(*row).map_or(1, |text| text.lines().count())
        });
        // Generous: three lines of explanation, the text itself with every
        // line wrapped once, and the padding.
        let limit = 80. + 2. * 17. * lines as f32;
        assert!(
            f32::from(bounds.size.height) < limit,
            "row {ix} ({row:?}) is {:?} tall",
            bounds.size.height
        );
    }
}
