//! `innerword agents ...` against a temporary home: detection, the skill, the MCP entries.
//!
//! Detection also looks at `/Applications` and the system's command folders, which a test
//! cannot replace, so these tests never count the agents that are found: they plant what
//! shows one agent in the temporary home and look at that one. Everything that is written
//! must be inside the temporary home.

use serde_json::Value;

use super::harness::{Sandbox, WRITER, inside};

fn agent<'a>(listing: &'a Value, id: &str) -> &'a Value {
    listing["agents"]
        .as_array()
        .expect("the listing has agents")
        .iter()
        .find(|agent| agent["id"] == id)
        .unwrap_or_else(|| panic!("{id} is not in the listing: {listing}"))
}

/// Every path a JSON value names, anywhere in it.
fn paths_in(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(text) if text.starts_with('/') => out.push(text.clone()),
        Value::Array(items) => items.iter().for_each(|item| paths_in(item, out)),
        Value::Object(map) => map.values().for_each(|item| paths_in(item, out)),
        _ => {}
    }
}

#[test]
fn the_listing_names_every_agent_and_finds_the_one_in_the_home() {
    let sandbox = Sandbox::new();
    std::fs::create_dir_all(sandbox.home().join(".claude")).unwrap();
    let listing = sandbox.json(&["agents"]);
    assert_eq!(sandbox.json(&["agents", "list"]), listing);
    assert_eq!(listing["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(
        listing["binary"], WRITER,
        "the skill names the binary that ran"
    );
    assert_eq!(listing["home"], sandbox.home().to_str().unwrap());
    let agents = listing["agents"].as_array().unwrap();
    assert!(
        agents.len() >= 8,
        "the prominent agents are all there: {}",
        agents.len()
    );
    for id in [
        "claude-code",
        "claude-desktop",
        "codex",
        "cursor",
        "gemini-cli",
        "opencode",
    ] {
        agent(&listing, id);
    }
    let claude = agent(&listing, "claude-code");
    assert_eq!(claude["detected"], true, "{claude}");
    assert_eq!(claude["name"], "Claude Code");
    assert_eq!(claude["skill"]["state"]["state"], "not-installed");
    let dir = claude["skill"]["dir"].as_str().unwrap();
    assert_eq!(dir, sandbox.home().join(".claude/skills").to_str().unwrap());

    let text = sandbox.ok(&["agents"]);
    assert!(text.contains("claude-code"), "{text}");
    assert!(
        text.contains("Claude Code: skill not installed in ~/.claude/skills"),
        "{text}"
    );
    assert!(text.contains(WRITER), "{text}");
    assert!(text.is_ascii(), "{text}");
}

#[test]
fn the_skill_is_installed_updated_and_removed_for_one_agent() {
    let sandbox = Sandbox::new();
    std::fs::create_dir_all(sandbox.home().join(".claude")).unwrap();
    let skill_file = sandbox.home().join(".claude/skills/inner-word/SKILL.md");

    // A dry run says what would happen and writes nothing.
    let dry = sandbox.ok(&["agents", "install", "claude-code", "--dry-run"]);
    assert!(dry.starts_with("Dry run, nothing was changed."), "{dry}");
    assert!(dry.contains("~/.claude/skills: install"), "{dry}");
    assert!(!skill_file.exists());

    let installed = sandbox.ok(&["agents", "install", "claude-code"]);
    assert!(
        installed.contains("~/.claude/skills: installed"),
        "{installed}"
    );
    assert!(
        installed.contains(&format!("wrote {}", skill_file.display())),
        "{installed}"
    );
    let skill = std::fs::read_to_string(&skill_file).expect("the skill was written");
    assert!(skill.starts_with("---\nname: inner-word\n"), "{skill}");
    assert!(skill.contains(WRITER), "the skill names this binary");
    assert_eq!(
        sandbox.ok(&["agents", "skill"]),
        skill,
        "what is printed is what is installed"
    );

    let listing = sandbox.json(&["agents"]);
    assert_eq!(
        agent(&listing, "claude-code")["skill"]["state"]["state"],
        "current"
    );
    let again = sandbox.json(&["agents", "install", "claude-code"]);
    assert_eq!(again["outcomes"][0]["report"]["change"], "unchanged");

    // Somebody edits the file: the listing says so and an update puts it right.
    std::fs::write(&skill_file, skill.replace("Inner Word", "The Reader")).unwrap();
    let listing = sandbox.json(&["agents"]);
    assert_eq!(
        agent(&listing, "claude-code")["skill"]["state"]["state"],
        "stale"
    );
    let dry = sandbox.ok(&["agents", "update", "claude-code", "--dry-run"]);
    assert!(dry.contains("~/.claude/skills: update"), "{dry}");
    let updated = sandbox.json(&["agents", "update", "claude-code"]);
    assert_eq!(updated["outcomes"][0]["report"]["change"], "updated");
    assert_eq!(std::fs::read_to_string(&skill_file).unwrap(), skill);

    let dry = sandbox.ok(&["agents", "remove", "claude-code", "--dry-run"]);
    assert!(dry.contains("~/.claude/skills: remove"), "{dry}");
    assert!(skill_file.exists());
    let removed = sandbox.ok(&["agents", "remove", "claude-code"]);
    assert!(removed.contains("~/.claude/skills: removed"), "{removed}");
    assert!(!skill_file.exists());
    assert!(
        !skill_file.parent().unwrap().exists(),
        "the folder of the skill goes with it"
    );
    assert!(
        sandbox.home().join(".claude/skills").is_dir(),
        "the skills folder itself stays"
    );

    // Nothing left to do is said, not failed.
    let nothing = sandbox.ok(&["agents", "remove", "claude-code"]);
    assert!(nothing.contains("nothing to remove"), "{nothing}");
    let nothing = sandbox.ok(&["agents", "update", "claude-code"]);
    assert!(nothing.contains("nothing to update"), "{nothing}");
}

#[test]
fn a_folder_somebody_else_made_is_left_alone() {
    let sandbox = Sandbox::new();
    let foreign = sandbox.home().join(".claude/skills/inner-word");
    std::fs::create_dir_all(&foreign).unwrap();
    std::fs::write(
        foreign.join("SKILL.md"),
        "---\nname: inner-word\n---\nMine.\n",
    )
    .unwrap();
    let run = sandbox.run(&["agents", "install", "claude-code"]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert_eq!(run.stdout, "");
    assert!(
        run.stderr.contains("was not installed by Inner Word"),
        "{}",
        run.stderr
    );
    assert_eq!(
        std::fs::read_to_string(foreign.join("SKILL.md")).unwrap(),
        "---\nname: inner-word\n---\nMine.\n"
    );
    let listing = sandbox.json(&["agents"]);
    assert_eq!(
        agent(&listing, "claude-code")["skill"]["state"]["state"],
        "foreign"
    );
}

/// The skill as The Writer installed it, in a skills folder of the sandbox's home.
fn plant_old_skill(sandbox: &Sandbox, skills: &str) -> std::path::PathBuf {
    let folder = sandbox.home().join(skills).join("the-writer");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("SKILL.md"),
        innerword_agent_setup::legacy::old_skill_text(
            std::path::Path::new("/Applications/The Writer.app/Contents/MacOS/writer"),
            "0.8.0",
        ),
    )
    .unwrap();
    folder
}

#[test]
fn a_skill_the_writer_installed_is_offered_for_update_replaced_and_removed() {
    let sandbox = Sandbox::new();
    std::fs::create_dir_all(sandbox.home().join(".claude")).unwrap();
    let old = plant_old_skill(&sandbox, ".claude/skills");
    let new = sandbox.home().join(".claude/skills/inner-word");

    let listing = sandbox.json(&["agents"]);
    let state = &agent(&listing, "claude-code")["skill"]["state"];
    assert_eq!(state["state"], "stale", "{state}");
    assert_eq!(state["reason"], "old-name", "{state}");
    assert_eq!(state["version"], "0.8.0", "{state}");
    let text = sandbox.ok(&["agents"]);
    assert!(
        text.contains(
            "Claude Code: skill needs update in ~/.claude/skills (It is installed under the old \
             name The Writer."
        ),
        "{text}"
    );

    let dry = sandbox.ok(&["agents", "install", "claude-code", "--dry-run"]);
    assert!(dry.contains("~/.claude/skills: update"), "{dry}");
    assert!(old.join("SKILL.md").is_file() && !new.exists());

    let installed = sandbox.ok(&["agents", "install", "claude-code"]);
    assert!(
        installed.contains("~/.claude/skills: updated"),
        "{installed}"
    );
    for line in [
        format!("  wrote {}", new.join("SKILL.md").display()),
        format!("  removed {}", old.join("SKILL.md").display()),
        format!("  removed {}", old.display()),
    ] {
        assert!(installed.contains(&line), "{line} in {installed}");
    }
    assert!(!old.exists(), "the old folder is gone");
    assert!(new.join("SKILL.md").is_file());
    let listing = sandbox.json(&["agents"]);
    assert_eq!(
        agent(&listing, "claude-code")["skill"]["state"]["state"],
        "current"
    );

    // Removing takes both names away; a copy only under the old name is found too.
    plant_old_skill(&sandbox, ".claude/skills");
    let removed = sandbox.json(&["agents", "remove", "claude-code"]);
    let paths: Vec<String> = removed["outcomes"][0]["report"]["removed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|path| path.as_str().unwrap().to_string())
        .collect();
    assert!(paths.contains(&old.display().to_string()), "{paths:?}");
    assert!(paths.contains(&new.display().to_string()), "{paths:?}");
    assert!(!old.exists() && !new.exists());

    plant_old_skill(&sandbox, ".claude/skills");
    let removed = sandbox.ok(&["agents", "remove", "--all"]);
    assert!(
        removed.contains(&format!("  removed {}", old.display())),
        "{removed}"
    );
    assert!(!old.exists());
}

#[test]
fn a_the_writer_folder_somebody_else_made_is_left_alone() {
    let sandbox = Sandbox::new();
    std::fs::create_dir_all(sandbox.home().join(".claude")).unwrap();
    let theirs = sandbox.home().join(".claude/skills/the-writer");
    std::fs::create_dir_all(&theirs).unwrap();
    let mine = "---\nname: the-writer\ndescription: My notes.\n---\nMine.\n";
    std::fs::write(theirs.join("SKILL.md"), mine).unwrap();

    let listing = sandbox.json(&["agents"]);
    assert_eq!(
        agent(&listing, "claude-code")["skill"]["state"]["state"],
        "not-installed"
    );
    let installed = sandbox.ok(&["agents", "install", "claude-code"]);
    assert!(
        installed.contains("~/.claude/skills: installed"),
        "{installed}"
    );
    assert!(!installed.contains("the-writer"), "{installed}");
    let removed = sandbox.ok(&["agents", "remove", "--all"]);
    assert!(!removed.contains("the-writer"), "{removed}");
    assert_eq!(
        std::fs::read_to_string(theirs.join("SKILL.md")).unwrap(),
        mine
    );
    assert_eq!(std::fs::read_dir(&theirs).unwrap().count(), 1);
}

#[test]
fn installing_for_all_agents_writes_only_inside_the_home() {
    let sandbox = Sandbox::new();
    std::fs::create_dir_all(sandbox.home().join(".claude")).unwrap();
    let root = sandbox.root();
    let installed = sandbox.json(&["agents", "install", "--all"]);
    let outcomes = installed["outcomes"].as_array().unwrap();
    assert!(!outcomes.is_empty());
    let mut paths = Vec::new();
    for outcome in outcomes {
        assert!(outcome.get("error").is_none(), "{outcome}");
        paths_in(&outcome["report"], &mut paths);
        paths_in(&outcome["skills_dir"], &mut paths);
    }
    assert!(!paths.is_empty());
    for path in &paths {
        assert!(
            inside(std::path::Path::new(path), &root),
            "{path} is outside the temporary home"
        );
    }
    assert!(
        sandbox
            .home()
            .join(".claude/skills/inner-word/SKILL.md")
            .is_file()
    );

    let updated = sandbox.json(&["agents", "update", "--all"]);
    for outcome in updated["outcomes"].as_array().unwrap() {
        assert_eq!(outcome["report"]["change"], "unchanged", "{outcome}");
    }
    let removed = sandbox.json(&["agents", "remove", "--all"]);
    assert_eq!(
        removed["outcomes"].as_array().unwrap().len(),
        outcomes.len()
    );
    assert!(!sandbox.home().join(".claude/skills/inner-word").exists());
    let listing = sandbox.json(&["agents"]);
    for agent in listing["agents"].as_array().unwrap() {
        if let Some(state) = agent["skill"]["state"]["state"].as_str() {
            assert_eq!(state, "not-installed", "{agent}");
        }
    }
}

#[test]
fn the_mcp_instructions_name_this_binary_for_every_agent() {
    let sandbox = Sandbox::new();
    let all = sandbox.json(&["agents", "mcp"]);
    assert_eq!(all["server_name"], "inner-word");
    assert_eq!(all["binary"], WRITER);
    let agents = all["agents"].as_array().unwrap();
    assert!(agents.len() >= 6, "{}", agents.len());
    for one in agents {
        let command = one["command"]["add"].as_str();
        let snippet = one["config"]["snippet"].as_str();
        assert!(command.is_some() || snippet.is_some(), "{one}");
        for text in command.iter().chain(snippet.iter()) {
            assert!(text.contains(WRITER), "the entry names this binary: {text}");
            assert!(text.contains("mcp"), "{text}");
        }
    }
    let claude = sandbox.json(&["agents", "mcp", "claude-code"]);
    assert_eq!(claude["agents"].as_array().unwrap().len(), 1);
    assert_eq!(claude["agents"][0]["agent"], "claude-code");
    let add = claude["agents"][0]["command"]["add"].as_str().unwrap();
    assert!(add.starts_with("claude mcp add "), "{add}");
    assert!(add.ends_with(" mcp"), "{add}");

    let text = sandbox.ok(&["agents", "mcp"]);
    assert!(
        text.contains("Claude Code (claude-code)\n  Run:     claude mcp add "),
        "{text}"
    );
    assert!(
        text.contains("Add to ") || text.contains("Or add to "),
        "{text}"
    );
    // The entry of the old name: the agent's own command, or the entry and its file.
    assert_eq!(all["old_server_name"], "the-writer");
    assert!(
        text.contains(
            "  Old name: If you added the server when the app was called The Writer, remove \
             that entry: claude mcp remove the-writer\n"
        ),
        "{text}"
    );
    assert!(
        text.contains("delete its \"the-writer\" entry from ~/.cursor/mcp.json."),
        "{text}"
    );
    assert_eq!(
        claude["agents"][0]["old_entry"]["command"],
        "claude mcp remove the-writer"
    );
    for one in agents {
        assert!(
            one["old_entry"]["line"]
                .as_str()
                .is_some_and(|line| line.contains("the-writer")),
            "{one}"
        );
    }
    // Printing instructions changes nothing in the home.
    assert_eq!(std::fs::read_dir(sandbox.home()).unwrap().count(), 0);
}

#[test]
fn an_agent_nobody_knows_is_not_found() {
    let sandbox = Sandbox::new();
    for args in [
        &["agents", "install", "hal-9000"][..],
        &["agents", "mcp", "hal-9000"],
        &["agents", "remove", "hal-9000"],
    ] {
        let message = sandbox.fail(args, 3);
        assert!(
            message.contains("there is no agent \"hal-9000\""),
            "{message}"
        );
        assert!(
            message.contains("claude-code"),
            "the message lists the agents: {message}"
        );
    }
}
