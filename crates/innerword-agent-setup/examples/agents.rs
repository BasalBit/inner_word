//! Prints what this crate sees on a Mac and what it would show: the agents it detects, the
//! state of the skill for each, and the MCP instructions. It reads only; nothing is
//! installed, and no agent's configuration is opened.
//!
//! ```text
//! cargo run -p innerword-agent-setup --example agents -- --home /tmp/some-home
//! cargo run -p innerword-agent-setup --example agents -- --this-mac [--binary <path>] [--skill]
//! ```
//!
//! `--home` looks only below that directory ([`Machine::isolated`]); `--this-mac` looks at
//! the real home and the system's folders. `--skill` prints the skill text instead.

use std::path::PathBuf;

use innerword_agent_setup::{
    DEFAULT_BINARY_PATH, Machine, SkillSpec, all_mcp_instructions, survey,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let mut machine = None;
    let mut binary = PathBuf::from(DEFAULT_BINARY_PATH);
    let mut print_skill = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--home" => {
                machine = Some(Machine::isolated(
                    args.next().ok_or("--home needs a directory")?,
                ))
            }
            "--this-mac" => machine = Some(Machine::this_mac()?),
            "--binary" => binary = PathBuf::from(args.next().ok_or("--binary needs a path")?),
            "--skill" => print_skill = true,
            other => return Err(format!("unknown argument {other}").into()),
        }
    }
    let spec = SkillSpec::new(binary)?;
    if print_skill {
        print!("{}", spec.render());
        return Ok(());
    }
    let machine = machine.ok_or("say where to look: --home <dir> or --this-mac")?;

    println!("Agents ({})", machine.display(machine.home()));
    for status in survey(&machine, &spec) {
        let found = match status.evidence.first() {
            Some(evidence) => format!("found: {}", machine.display(&evidence.path)),
            None => "not found".to_string(),
        };
        println!("  {:<34} {found}", status.name);
        if let Some(skill) = &status.skill {
            let shared: Vec<&str> = skill.shared_with.iter().map(|id| id.key()).collect();
            let shared = if shared.is_empty() {
                String::new()
            } else {
                format!(" (shared with {})", shared.join(", "))
            };
            println!(
                "  {:<34} skill: {} in {}{shared}",
                "",
                skill.state.label(),
                skill.dir_display
            );
        }
    }

    println!("\nMCP server");
    for instructions in all_mcp_instructions(spec.binary(), Some(machine.home()))? {
        println!("  {}", instructions.name);
        if let Some(command) = &instructions.command {
            println!("    run:  {}", command.add);
        }
        if let Some(config) = &instructions.config {
            let place = config
                .file_display
                .as_deref()
                .or(config.open_with)
                .unwrap_or("its configuration");
            println!("    file: {place} ({})", config.placement);
            for line in config.snippet.lines() {
                println!("      {line}");
            }
        }
        if let Some(note) = instructions.note {
            println!("    note: {note}");
        }
        if let Some(url) = instructions.docs_url() {
            println!("    docs: {url}");
        }
    }
    Ok(())
}
