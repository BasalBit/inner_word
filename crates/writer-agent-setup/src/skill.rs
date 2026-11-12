//! The skill: a folder named [`SKILL_NAME`] with one file, `SKILL.md`, in the Agent Skills
//! format. It tells an agent that The Writer's command line tool exists, where it is, and
//! how to use it well.
//!
//! The text is the same for every agent. Two things are filled in when it is installed: the
//! absolute path of the `writer` binary and the version of the app it came with. Both are
//! also recorded in the front matter, under `metadata`, which the Agent Skills specification
//! reserves for the client's own keys:
//!
//! ```yaml
//! metadata:
//!   generator: the-writer-app
//!   version: "0.4.0"
//!   binary: "/Applications/The Writer.app/Contents/MacOS/writer"
//! ```
//!
//! That block is the version marker, and it is how an installed copy is recognised as
//! written by this crate ([`read_marker`]).

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::SetupError;
use crate::shell;

/// The name of the skill and of its folder. Lowercase letters and hyphens, as the
/// specification asks, and the same as the `name` in the front matter.
pub const SKILL_NAME: &str = "the-writer";

/// The one file in the skill's folder.
pub const SKILL_FILE: &str = "SKILL.md";

/// The value of `metadata.generator` in a `SKILL.md` this crate wrote.
pub const GENERATOR: &str = "the-writer-app";

/// The version of the app this crate was built with, which an installed skill records.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Where the `writer` binary is when the app is installed in `/Applications`.
pub const DEFAULT_BINARY_PATH: &str = "/Applications/The Writer.app/Contents/MacOS/writer";

/// The `description` of the skill: what an agent reads to decide whether to load it. It
/// names the app and the words people use for what is in it, and says when to use it.
pub const DESCRIPTION: &str = "Work with the user's writing in The Writer, the macOS writing app, and with the documents, drafts, chapters, books, blog posts, notes and folders in its library. Use it whenever the user mentions The Writer or their library, or asks to find, read, list, search, create, edit, rename, move, organise, comment on, review, export or restore a document, a draft, a chapter or a folder that lives in that app. It provides the writer command line tool, which reads and changes the library directly, also while the app is closed.";

/// The text below the front matter. In its code blocks a line that starts with `writer `
/// is an example; the installed text has the quoted path of the binary in place of that
/// word.
const BODY: &str = include_str!("skill_body.md");

/// How an example line of [`BODY`] starts.
const EXAMPLE_PREFIX: &str = "writer ";

/// The path of the `writer` binary that lies beside an executable: for the app's own
/// executable (`The Writer.app/Contents/MacOS/the-writer`) that is the tool in the bundle,
/// and for the tool itself it is the tool.
pub fn binary_beside(executable: &Path) -> PathBuf {
    match executable.parent() {
        Some(folder) => folder.join("writer"),
        None => PathBuf::from("writer"),
    }
}

/// What an installed skill should be: which binary it names and which version it carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkillSpec {
    binary: PathBuf,
    version: String,
}

impl SkillSpec {
    /// A skill for the tool at `binary`, with the version of this build of the app.
    ///
    /// The path must be absolute; whether a file is there is not checked (the skill may be
    /// written before the app is moved into place).
    pub fn new(binary: impl Into<PathBuf>) -> Result<SkillSpec, SetupError> {
        let binary = binary.into();
        if !binary.is_absolute() {
            return Err(SetupError::BinaryNotAbsolute(binary));
        }
        Ok(SkillSpec {
            binary,
            version: APP_VERSION.to_string(),
        })
    }

    /// The same skill carrying another version (for tests and for a caller that knows the
    /// app's version better than this crate does).
    pub fn with_version(mut self, version: impl Into<String>) -> SkillSpec {
        self.version = version.into();
        self
    }

    /// The absolute path of the `writer` binary.
    pub fn binary(&self) -> &Path {
        &self.binary
    }

    /// The version the skill carries.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// The complete text of `SKILL.md`.
    pub fn render(&self) -> String {
        let binary = self.binary.to_string_lossy();
        let mut text = String::with_capacity(BODY.len() + 1024);
        text.push_str("---\n");
        text.push_str(&format!("name: {SKILL_NAME}\n"));
        text.push_str(&format!("description: {DESCRIPTION}\n"));
        text.push_str("metadata:\n");
        text.push_str(&format!("  generator: {GENERATOR}\n"));
        text.push_str(&format!(
            "  version: {}\n",
            shell::string_literal(&self.version)
        ));
        text.push_str(&format!("  binary: {}\n", shell::string_literal(&binary)));
        text.push_str("---\n\n");
        // In the examples the tool is called by its full path, quoted for the shell: an
        // agent copies them as they stand.
        let quoted = shell::quote(&binary);
        let mut in_code = false;
        for line in BODY.lines() {
            if line.trim_start().starts_with("```") {
                in_code = !in_code;
            }
            match line.strip_prefix(EXAMPLE_PREFIX).filter(|_| in_code) {
                Some(rest) => {
                    text.push_str(&quoted);
                    text.push(' ');
                    text.push_str(rest);
                }
                None => text.push_str(&line.replace("{{VERSION}}", &self.version)),
            }
            text.push('\n');
        }
        text
    }
}

/// What a `SKILL.md` written by this crate says about itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkillMarker {
    /// The version of the app that installed it.
    pub version: String,
    /// The binary its text names.
    pub binary: PathBuf,
}

/// The marker of a `SKILL.md`, or `None` when the text was not written by this crate (no
/// front matter, or no `metadata.generator` with this crate's value).
pub fn read_marker(skill_md: &str) -> Option<SkillMarker> {
    let front_matter = front_matter(skill_md)?;
    let mut in_metadata = false;
    let (mut generator, mut version, mut binary) = (None, None, None);
    for line in front_matter.lines() {
        if !line.starts_with(' ') {
            in_metadata = line.trim_end() == "metadata:";
            continue;
        }
        if !in_metadata {
            continue;
        }
        let Some((key, value)) = line.trim().split_once(':') else {
            continue;
        };
        let value = scalar(value.trim());
        match key.trim() {
            "generator" => generator = Some(value),
            "version" => version = Some(value),
            "binary" => binary = Some(value),
            _ => {}
        }
    }
    if generator.as_deref() != Some(GENERATOR) {
        return None;
    }
    Some(SkillMarker {
        version: version.unwrap_or_default(),
        binary: PathBuf::from(binary.unwrap_or_default()),
    })
}

/// The lines between the two `---` lines at the start of the text.
fn front_matter(text: &str) -> Option<&str> {
    let rest = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))?;
    let end = rest.find("\n---")?;
    Some(&rest[..end])
}

/// A YAML scalar as this crate writes it: a JSON string literal, or plain text.
fn scalar(value: &str) -> String {
    if value.starts_with('"')
        && let Ok(serde_json::Value::String(text)) = serde_json::from_str(value)
    {
        return text;
    }
    value.trim_matches(|c| c == '"' || c == '\'').to_string()
}

/// The `writer` commands the skill's text shows, each as its words up to the first argument
/// that is not a command word (`["comment", "add"]`, `["tree"]`), in the order they appear
/// and each once. A test runs every one of them against the built binary.
pub fn mentioned_commands() -> Vec<Vec<String>> {
    let mut commands: Vec<Vec<String>> = Vec::new();
    let mut in_code = false;
    for line in BODY.lines() {
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if !in_code {
            continue;
        }
        let Some(rest) = line.strip_prefix(EXAMPLE_PREFIX) else {
            continue;
        };
        let words: Vec<String> = rest
            .split_whitespace()
            .take_while(|word| {
                word.chars().all(|c| c.is_ascii_lowercase() || c == '-') && !word.starts_with('-')
            })
            .map(str::to_string)
            .collect();
        if !words.is_empty() && !commands.contains(&words) {
            commands.push(words);
        }
    }
    commands
}

/// Every example line of the skill's text that calls the tool (the lines of its code blocks
/// that start with `writer`), with the bare word `writer` where the installed text has the
/// quoted path of the binary.
pub fn example_lines() -> Vec<&'static str> {
    let mut lines = Vec::new();
    let mut in_code = false;
    for line in BODY.lines() {
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
        } else if in_code && line.starts_with(EXAMPLE_PREFIX) {
            lines.push(line);
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> SkillSpec {
        SkillSpec::new(DEFAULT_BINARY_PATH).unwrap()
    }

    #[test]
    fn a_relative_binary_path_is_refused() {
        assert!(matches!(
            SkillSpec::new("writer"),
            Err(SetupError::BinaryNotAbsolute(_))
        ));
        assert!(matches!(
            SkillSpec::new("./bin/writer"),
            Err(SetupError::BinaryNotAbsolute(_))
        ));
    }

    /// What the Agent Skills specification and the stricter agents ask of the front matter.
    #[test]
    fn the_front_matter_follows_the_specification() {
        let text = spec().render();
        let front = front_matter(&text).expect("front matter");
        let mut lines = front.lines();
        assert_eq!(lines.next(), Some("name: the-writer"));
        let description = lines
            .next()
            .unwrap()
            .strip_prefix("description: ")
            .expect("description second");
        assert_eq!(description, DESCRIPTION);

        assert!(SKILL_NAME.len() <= 64);
        assert!(
            SKILL_NAME
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        );
        assert!(
            !SKILL_NAME.starts_with('-')
                && !SKILL_NAME.ends_with('-')
                && !SKILL_NAME.contains("--")
        );

        assert!(
            DESCRIPTION.len() <= 1024,
            "{} characters",
            DESCRIPTION.len()
        );
        // A plain YAML scalar on one line: nothing in it that YAML would read as syntax.
        assert!(!DESCRIPTION.contains('\n'));
        assert!(
            !DESCRIPTION.contains(": ") && !DESCRIPTION.contains(" #"),
            "would not be a plain scalar"
        );
        assert!(!DESCRIPTION.contains('<') && !DESCRIPTION.contains('>'));
        assert!(DESCRIPTION.starts_with(|c: char| c.is_ascii_alphabetic()));
        for word in ["The Writer", "documents", "drafts", "chapters", "library"] {
            assert!(
                DESCRIPTION.contains(word),
                "the description does not mention {word}"
            );
        }
    }

    #[test]
    fn the_body_is_short_enough_and_has_no_placeholder_left() {
        let text = spec().render();
        assert!(!text.contains("{{"), "a placeholder was not filled in");
        assert!(
            text.lines().count() < 500,
            "the specification recommends under 500 lines"
        );
        assert!(
            text.is_ascii()
                || !text
                    .chars()
                    .any(|c| ('\u{1F300}'..='\u{1FAFF}').contains(&c)),
            "no emoji"
        );
        assert!(text.ends_with('\n'));
    }

    #[test]
    fn the_binary_path_is_in_the_text_quoted_for_the_shell() {
        let text = spec().render();
        assert!(text.contains("'/Applications/The Writer.app/Contents/MacOS/writer'"));
        let odd = SkillSpec::new("/Users/o'brien/The Writer.app/Contents/MacOS/writer")
            .unwrap()
            .render();
        assert!(odd.contains(r"'/Users/o'\''brien/The Writer.app/Contents/MacOS/writer'"));
    }

    #[test]
    fn the_marker_reads_back_version_and_binary() {
        let marker = read_marker(&spec().render()).expect("marker");
        assert_eq!(marker.version, APP_VERSION);
        assert_eq!(marker.binary, PathBuf::from(DEFAULT_BINARY_PATH));

        let odd = SkillSpec::new("/tmp/a \"quoted\" \\ folder/writer")
            .unwrap()
            .with_version("9.9.9-beta");
        let marker = read_marker(&odd.render()).expect("marker");
        assert_eq!(marker.version, "9.9.9-beta");
        assert_eq!(
            marker.binary,
            PathBuf::from("/tmp/a \"quoted\" \\ folder/writer")
        );
    }

    #[test]
    fn a_skill_somebody_else_wrote_has_no_marker() {
        assert_eq!(read_marker(""), None);
        assert_eq!(read_marker("# The Writer\n\nNotes of my own.\n"), None);
        assert_eq!(
            read_marker("---\nname: the-writer\ndescription: Mine.\n---\n\nBody\n"),
            None
        );
        assert_eq!(
            read_marker(
                "---\nname: the-writer\ndescription: Mine.\nmetadata:\n  generator: somebody-else\n---\n"
            ),
            None
        );
        // The generator key outside `metadata` does not count.
        assert_eq!(
            read_marker("---\nname: the-writer\ngenerator: the-writer-app\n---\n"),
            None
        );
    }

    #[test]
    fn binary_beside_an_executable() {
        assert_eq!(
            binary_beside(Path::new(
                "/Applications/The Writer.app/Contents/MacOS/the-writer"
            )),
            PathBuf::from(DEFAULT_BINARY_PATH)
        );
        assert_eq!(
            binary_beside(Path::new(DEFAULT_BINARY_PATH)),
            PathBuf::from(DEFAULT_BINARY_PATH)
        );
    }

    #[test]
    fn the_text_shows_commands() {
        let commands = mentioned_commands();
        assert!(commands.len() >= 25, "{commands:?}");
        assert!(example_lines().len() >= commands.len());
        for expected in [
            vec!["list"],
            vec!["search"],
            vec!["get"],
            vec!["edit"],
            vec!["replace-section"],
            vec!["comment", "add"],
            vec!["history", "restore"],
            vec!["export"],
        ] {
            let expected: Vec<String> = expected.into_iter().map(str::to_string).collect();
            assert!(commands.contains(&expected), "{expected:?} is not shown");
        }
    }

    /// What the text has to teach, whatever its wording.
    #[test]
    fn the_text_teaches_the_rules() {
        let text = spec().render();
        for needed in [
            "exit code 5",
            "Quit The Writer",
            "do not quit the app yourself",
            "--dry-run",
            "--json",
            "Read before you change",
            "`comment add`",
            "Use the ids",
            "--permanently",
            "is not on the PATH",
        ] {
            assert!(text.contains(needed), "the skill does not say: {needed}");
        }
    }

    /// Every example of the installed text is a command line that runs the tool at its
    /// path with spaces: each one is handed to the real shell, with a stand-in for the
    /// tool that prints its arguments.
    #[test]
    fn every_example_of_the_installed_text_runs_the_tool_at_its_path() {
        use std::os::unix::fs::PermissionsExt;
        let home = tempfile::Builder::new()
            .prefix("skill examples ")
            .tempdir()
            .unwrap();
        let binary = home
            .path()
            .join("My Apps/The Writer.app/Contents/MacOS/writer");
        std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
        std::fs::write(&binary, "#!/bin/sh\nprintf '%s\\n' \"$@\"\n").unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();

        let text = SkillSpec::new(&binary).unwrap().render();
        let quoted = shell::quote(&binary.to_string_lossy());
        let mut in_code = false;
        let mut ran = 0;
        for line in text.lines() {
            if line.starts_with("```") {
                in_code = !in_code;
                continue;
            }
            if !in_code {
                continue;
            }
            assert!(
                line.starts_with(&quoted),
                "an example that does not call the tool by its path: {line}"
            );
            let output = std::process::Command::new("/bin/sh")
                .arg("-c")
                .arg(line)
                .current_dir(home.path())
                .env("HOME", home.path())
                .output()
                .expect("run /bin/sh");
            assert!(
                output.status.success(),
                "{line}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let printed = String::from_utf8_lossy(&output.stdout);
            let first = printed.lines().next().unwrap_or_default();
            let expected = line[quoted.len()..].split_whitespace().next().unwrap();
            assert_eq!(first, expected, "{line}");
            ran += 1;
        }
        assert_eq!(ran, example_lines().len(), "every example was run");
    }
}
