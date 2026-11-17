//! The names the app had before it was called Inner Word, in the one place they are kept.
//!
//! Until version 0.9.0 the app was "The Writer". What it installed for agents then is still
//! on people's Macs, under the old names:
//!
//! - **The skill** was a folder [`LEGACY_SKILL_NAME`] whose `SKILL.md` carries
//!   `metadata.generator: ` [`LEGACY_GENERATOR`]. Such a folder is Inner Word's own old
//!   install: its state is "needs update" ([`crate::StaleReason::OldName`]), installing or
//!   updating writes the skill under its new name and removes the old folder, and removing
//!   removes both. A folder of that name without that marker is the user's own and is
//!   never touched.
//! - **The MCP server** was registered as [`LEGACY_MCP_SERVER_NAME`]. The app never edits an
//!   agent's configuration, so the old entry is only named: beside the instructions that add
//!   the server, one line says how to remove the old one
//!   ([`crate::McpInstructions::old_entry`]).
//!
//! These names are read, never written.

/// The folder the skill was installed as when the app was called The Writer.
pub const LEGACY_SKILL_NAME: &str = "the-writer";

/// The `metadata.generator` of a `SKILL.md` that The Writer wrote.
pub const LEGACY_GENERATOR: &str = "the-writer-app";

/// The name the MCP server was registered under when the app was called The Writer.
pub const LEGACY_MCP_SERVER_NAME: &str = "the-writer";

/// The app's old name, as the interface says it ("installed under the old name The Writer").
pub const LEGACY_APP_NAME: &str = "The Writer";

/// The text of the `SKILL.md` The Writer installed for the tool at `binary`, as version
/// `version` of it wrote it: the front matter of this crate's skill with the old name and
/// the old generator. For tests (of this crate, of the tool and of the app) that stand for
/// a Mac on which The Writer installed its skill.
#[doc(hidden)]
pub fn old_skill_text(binary: &std::path::Path, version: &str) -> String {
    let spec = crate::SkillSpec::new(binary)
        .expect("an absolute path")
        .with_version(version);
    spec.render()
        .replacen(
            &format!("name: {}\n", crate::SKILL_NAME),
            &format!("name: {LEGACY_SKILL_NAME}\n"),
            1,
        )
        .replacen(
            &format!("generator: {}\n", crate::GENERATOR),
            &format!("generator: {LEGACY_GENERATOR}\n"),
            1,
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn an_old_skill_reads_as_the_old_generator_and_not_as_ours() {
        let binary = Path::new("/Applications/The Writer.app/Contents/MacOS/writer");
        let text = old_skill_text(binary, "0.8.0");
        assert!(text.starts_with("---\nname: the-writer\n"), "{text}");
        assert!(text.contains("\n  generator: the-writer-app\n"), "{text}");
        assert_eq!(crate::read_marker(&text), None);
        let marker = crate::read_legacy_marker(&text).expect("the old marker");
        assert_eq!(marker.version, "0.8.0");
        assert_eq!(marker.binary, binary);
        // And the other way round: today's skill is not an old one.
        let today = crate::SkillSpec::new(binary).unwrap().render();
        assert_eq!(crate::read_legacy_marker(&today), None);
    }
}
