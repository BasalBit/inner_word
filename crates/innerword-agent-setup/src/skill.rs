//! The skill: a folder named [`SKILL_NAME`] with one file, `SKILL.md`, in the Agent Skills
//! format. It tells an agent that Inner Word's command line tool exists, where it is, and
//! how to use it well.
//!
//! The text is the same for every agent. Two things are filled in when it is installed: the
//! absolute path of the `innerword` binary and the version of the app it came with. Both are
//! also recorded in the front matter, under `metadata`, which the Agent Skills specification
//! reserves for the client's own keys:
//!
//! ```yaml
//! metadata:
//!   generator: inner-word-app
//!   version: "0.4.0"
//!   binary: "/Applications/Inner Word.app/Contents/MacOS/innerword"
//! ```
//!
//! That block is the version marker, and it is how an installed copy is recognised as
//! written by this crate ([`read_marker`]).

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::SetupError;
use crate::legacy::LEGACY_GENERATOR;
use crate::shell;

/// The name of the skill and of its folder. Lowercase letters and hyphens, as the
/// specification asks, and the same as the `name` in the front matter.
pub const SKILL_NAME: &str = "inner-word";

/// The one file in the skill's folder.
pub const SKILL_FILE: &str = "SKILL.md";

/// The value of `metadata.generator` in a `SKILL.md` this crate wrote.
pub const GENERATOR: &str = "inner-word-app";

/// The version of the app this crate was built with, which an installed skill records.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Where the `innerword` binary is when the app is installed in `/Applications`.
pub const DEFAULT_BINARY_PATH: &str = "/Applications/Inner Word.app/Contents/MacOS/innerword";

/// The `description` of the skill: what an agent reads to decide whether to load it. It
/// names the app and the words people use for what is in it, and says when to use it.
pub const DESCRIPTION: &str = "Work with the user's writing in Inner Word, the macOS writing app, and with the documents, drafts, chapters, books, blog posts, notes, whiteboards and folders in its library. Use it whenever the user mentions Inner Word or their library, or asks to find, read, list, search, create, edit, rename, move, organise, comment on, review, draw on, export or restore a document, a draft, a chapter, a whiteboard or a folder that lives in that app. It provides the innerword command line tool, which reads and changes the library whether the app is open or closed.";

/// The text below the front matter. In its code blocks a line that starts with `innerword `
/// is an example; the installed text has the quoted path of the binary in place of that
/// word.
const BODY: &str = include_str!("skill_body.md");

/// How an example line of [`BODY`] starts.
const EXAMPLE_PREFIX: &str = "innerword ";

/// The path of the `innerword` binary that lies beside an executable: for the app's own
/// executable (`Inner Word.app/Contents/MacOS/inner-word`) that is the tool in the bundle,
/// and for the tool itself it is the tool.
pub fn binary_beside(executable: &Path) -> PathBuf {
    match executable.parent() {
        Some(folder) => folder.join("innerword"),
        None => PathBuf::from("innerword"),
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

    /// The absolute path of the `innerword` binary.
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
    marker_of(skill_md, GENERATOR)
}

/// The marker of a `SKILL.md` that the app wrote when it was called The Writer (generator
/// [`LEGACY_GENERATOR`]), or `None` when the text is anybody else's.
pub fn read_legacy_marker(skill_md: &str) -> Option<SkillMarker> {
    marker_of(skill_md, LEGACY_GENERATOR)
}

/// The marker of a `SKILL.md` whose `metadata.generator` is `generator`.
fn marker_of(skill_md: &str, generator_wanted: &str) -> Option<SkillMarker> {
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
    if generator.as_deref() != Some(generator_wanted) {
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

/// The `innerword` commands the skill's text shows, each as its words up to the first argument
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
/// that start with `innerword`), with the bare word `innerword` where the installed text has the
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
            SkillSpec::new("innerword"),
            Err(SetupError::BinaryNotAbsolute(_))
        ));
        assert!(matches!(
            SkillSpec::new("./bin/innerword"),
            Err(SetupError::BinaryNotAbsolute(_))
        ));
    }

    /// What the Agent Skills specification and the stricter agents ask of the front matter.
    #[test]
    fn the_front_matter_follows_the_specification() {
        let text = spec().render();
        let front = front_matter(&text).expect("front matter");
        let mut lines = front.lines();
        assert_eq!(lines.next(), Some("name: inner-word"));
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
        for word in ["Inner Word", "documents", "drafts", "chapters", "library"] {
            assert!(
                DESCRIPTION.contains(word),
                "the description does not mention {word}"
            );
        }
    }

    /// The rule since 0.6.0: an agent changes things while the app is open too, the
    /// writer sees the change and can undo it, and the agent says what it changed.
    #[test]
    fn the_skill_says_what_happens_while_the_app_is_open() {
        let text = spec().render();
        let part = text
            .split("## While Inner Word is open")
            .nth(1)
            .and_then(|rest| rest.split("\n## ").next())
            .expect("the part about the open app");
        for words in [
            "You can read and change the library whether the app is open or closed.",
            "the app makes it, and the user sees it in the window at once",
            "take it back there with Undo",
            "Say what you changed and in which document, every time.",
            "Pass `--author` with your name.",
            "Exit code 5 means that the app is open and did not make the change.",
            // The switch by the name it has in the app's Agents panel and in the tool's
            // message.
            "\"Agents can make changes while Inner Word is open\"",
            "do not quit the app yourself",
            "it is not known whether the change was made",
        ] {
            assert!(part.contains(words), "the skill does not say: {words}");
        }
        // Nothing of the rule of before is left anywhere in it.
        for words in [
            "every change is refused",
            "I can read your library but not change it",
            "also while the app is closed",
        ] {
            assert!(!text.contains(words), "the skill still says: {words}");
        }
        assert!(DESCRIPTION.contains("whether the app is open or closed"));
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
        assert!(text.contains("'/Applications/Inner Word.app/Contents/MacOS/innerword'"));
        let odd = SkillSpec::new("/Users/o'brien/Inner Word.app/Contents/MacOS/innerword")
            .unwrap()
            .render();
        assert!(odd.contains(r"'/Users/o'\''brien/Inner Word.app/Contents/MacOS/innerword'"));
    }

    #[test]
    fn the_marker_reads_back_version_and_binary() {
        let marker = read_marker(&spec().render()).expect("marker");
        assert_eq!(marker.version, APP_VERSION);
        assert_eq!(marker.binary, PathBuf::from(DEFAULT_BINARY_PATH));

        let odd = SkillSpec::new("/tmp/a \"quoted\" \\ folder/innerword")
            .unwrap()
            .with_version("9.9.9-beta");
        let marker = read_marker(&odd.render()).expect("marker");
        assert_eq!(marker.version, "9.9.9-beta");
        assert_eq!(
            marker.binary,
            PathBuf::from("/tmp/a \"quoted\" \\ folder/innerword")
        );
    }

    #[test]
    fn a_skill_somebody_else_wrote_has_no_marker() {
        assert_eq!(read_marker(""), None);
        assert_eq!(read_marker("# Inner Word\n\nNotes of my own.\n"), None);
        assert_eq!(
            read_marker("---\nname: inner-word\ndescription: Mine.\n---\n\nBody\n"),
            None
        );
        assert_eq!(
            read_marker(
                "---\nname: inner-word\ndescription: Mine.\nmetadata:\n  generator: somebody-else\n---\n"
            ),
            None
        );
        // The generator key outside `metadata` does not count.
        assert_eq!(
            read_marker("---\nname: inner-word\ngenerator: inner-word-app\n---\n"),
            None
        );
    }

    #[test]
    fn binary_beside_an_executable() {
        assert_eq!(
            binary_beside(Path::new(
                "/Applications/Inner Word.app/Contents/MacOS/inner-word"
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
            vec!["image", "add"],
            vec!["image", "list"],
            vec!["board", "new"],
            vec!["board", "add"],
            vec!["board", "update"],
            vec!["board", "remove"],
            vec!["board", "list"],
        ] {
            let expected: Vec<String> = expected.into_iter().map(str::to_string).collect();
            assert!(commands.contains(&expected), "{expected:?} is not shown");
        }
    }

    /// What an agent has to know about images, whatever the wording: how one reads, that
    /// the line keeps it and its absence removes it, how one is added, and how to look at
    /// one.
    #[test]
    fn the_text_says_how_images_read_and_how_one_is_added() {
        let text = spec().render();
        for needed in [
            "`![description](image:<id>)`",
            "Keep that line as it is and the image stays, with its size",
            "the image is removed (the version kept before your change still has it)",
            "image add doc:12 ~/Pictures/harbour.jpg --alt \"The harbour at dusk\"",
            "`![what it shows](/absolute/path.png)`",
            "PNG, JPEG, GIF, HEIC, TIFF, WebP and BMP",
            "an address on the web stays a link",
            "export the document as Markdown to a file",
            "runs across an image",
        ] {
            assert!(text.contains(needed), "the skill does not say: {needed}");
        }
    }

    /// What the text says about whiteboards, as the design has it (section 7.8).
    #[test]
    fn the_text_says_what_a_whiteboard_is_and_how_one_is_drawn_on() {
        let text = spec().render();
        for needed in [
            "Whiteboards. A whiteboard is a drawing in the library, drawn in the app with \
             Excalidraw: shapes, arrows, text, frames and pictures on a canvas.",
            "`innerword board new \"Architecture\" --in Sketches` makes one (with `--file \
             shapes.json` it starts with elements).",
            "`innerword get board:12` lists its elements, one per line with id, type, position, \
             size and text; `--format json` gives the scene as Excalidraw stores it.",
            "Add elements as Excalidraw's element skeletons:",
            "Give shapes room for their labels and leave out the width and height of text.",
            "A picture from a file of this Mac is an element too, \
             `{\"type\":\"image\",\"id\":\"photo\",\"x\":640,\"y\":80,\"path\":\"/Users/ada/Pictures/harbour.jpg\"}`",
            "the tool reads the file and the library keeps a copy, as it does for `innerword \
             image add`.",
            "moves, resizes, restyles or rewrites by id;",
            "`innerword board remove board:12 db` removes elements with their labels.",
            "Every change keeps the drawing as it was as a version first.",
            "A whiteboard is exported as a PDF only: `innerword export board:12 \
             ~/Desktop/architecture.pdf`.",
            "Its text is not edited with `edit` or `append` (exit code 15).",
            "- 15: a page or a whiteboard where a document is needed, or the reverse. A page \
             cannot be edited;",
            "A whiteboard is changed with `board add`, `board update` and `board remove`",
        ] {
            assert!(text.contains(needed), "the skill does not say: {needed}");
        }
        assert!(DESCRIPTION.contains("whiteboard"));
    }

    /// What the text says about pages, as the design has it: the paragraph and the recipe
    /// for copying the user's claude.ai artifacts.
    #[test]
    fn the_text_says_what_a_page_is_and_how_artifacts_are_copied() {
        let text = spec().render();
        for needed in [
            "Pages. A page is a web page kept in the library as it was made, with its styles, \
             scripts, fonts, pictures and sounds, so that it shows the same offline and for good.",
            "`innerword page add ~/Downloads/explainer --source-url https://claude.ai/artifact/\u{2026} \
             --folder \"Claude Pages\" --create-folders`",
            "A folder is taken whole and opens with its `index.html` (`--entry` names another file).",
            "a page that needs something that cannot be stored (data from a service, an embedded \
             site) is not added, exit code 14 says why, and nothing is saved.",
            "With `--source-url`, adding the same address again changes nothing when the page is \
             the same and updates it (keeping the old one as a version) when it changed.",
            "`innerword pages` lists the pages with their addresses.",
            "`innerword get page:12` gives the text the page shows.",
            "`innerword export page:12 ~/Desktop/page.pdf` makes a PDF of it.",
            "A page cannot be edited (exit code 15).",
            "`--include-trashed` adds it anyway",
            "are for the user: pass them on.",
            "To copy the user's claude.ai artifacts into Inner Word (Claude Code), use the \
             Artifact tool and change nothing on claude.ai: never republish, edit or delete an \
             artifact while you copy it, whatever an error suggests.",
            "List the artifacts (`action: \"list\"`, `scope: \"mine\"`, `limit: 200`)",
            "without `out_dir` (the default folder needs no approval)",
            "\"externals\": {\"<asset url>\": \"<saved file>\"}",
            "run `innerword page add --list <file> --folder \"Claude Pages\" --create-folders --author \
             Claude`",
            "Report what was added, updated, unchanged, trashed and not added, with the reasons \
             and the warnings.",
            "read its rows with the ArtifactData tool and keep them as a document beside the page, \
             titled \"<page title> (saved data)\", with `innerword create`.",
        ] {
            assert!(text.contains(needed), "the skill does not say: {needed}");
        }
        let commands = mentioned_commands();
        for expected in [vec!["page", "add"], vec!["page", "list"]] {
            let expected: Vec<String> = expected.into_iter().map(str::to_string).collect();
            assert!(commands.contains(&expected), "{expected:?} is not shown");
        }
    }

    /// The recipe for copying artifacts calls the Artifact tool, which this crate cannot run:
    /// its calls are held against what the tool does, as it was read and tried (the
    /// tool's description, and its answers on the user's own artifacts, 2026-11-14).
    ///
    /// - `list` takes `scope` (`mine`, `shared`, `all`, `types`, `files` with `url`,
    ///   `assets` with `url`) and `limit`; `read` takes `url` with `path`, `paths`,
    ///   `out_dir`, `prompt` or `page`.
    /// - A `read` with only the `url` answers the page inline and saves no file; with
    ///   `paths` (the page of a single-page artifact is `index.html`) it saves the files
    ///   and names the folder ("Files saved under …"). So every read of the recipe saves.
    /// - Listing the assets of an artifact that does not declare the `assets` capability
    ///   fails with `capability_disabled`, and the error says to republish the artifact.
    /// - The list of artifacts gives the day of the last update, without a time.
    /// - The listing of files of an artifact made from a type says "an Artifact of type
    ///   \"Docs\"" and lists its program (135 files, 25 MB, for one of the user's).
    #[test]
    fn the_recipe_calls_the_artifact_tool_as_it_answers() {
        let text = spec().render();
        let start = text
            .find("To copy the user's claude.ai artifacts")
            .expect("the recipe");
        let end = text[start..]
            .find("(saved data)")
            .map(|at| start + at)
            .expect("the end of the recipe");
        let recipe = &text[start..end];

        // Every call, as "(`action: "…"`, …)": the action and the arguments it names.
        let mut calls: Vec<(String, Vec<String>)> = Vec::new();
        let mut rest = recipe;
        while let Some(at) = rest.find("(`action: \"") {
            let after = &rest[at + "(`action: \"".len()..];
            let action = after[..after.find('"').unwrap()].to_string();
            let group = &after[..after.find(')').expect("a call ends")];
            let mut arguments = Vec::new();
            for piece in group.split('`').skip(2).step_by(2) {
                if let Some((name, _)) = piece.split_once(": ") {
                    arguments.push(name.to_string());
                } else {
                    arguments.push(piece.to_string());
                }
            }
            calls.push((action, arguments));
            rest = &after[group.len()..];
        }
        assert!(calls.len() >= 6, "{calls:?}");
        let mut read_with_paths = false;
        for (action, arguments) in &calls {
            let known: &[&str] = match action.as_str() {
                "list" => &["scope", "limit", "url"],
                "read" => &["url", "path", "paths", "out_dir", "prompt", "page"],
                other => panic!("the Artifact tool has no action {other} in the recipe"),
            };
            for argument in arguments {
                assert!(
                    known.contains(&argument.as_str()),
                    "{action} takes no {argument}: {arguments:?}"
                );
            }
            if action == "read" {
                assert!(
                    arguments.iter().any(|a| a == "path" || a == "paths"),
                    "a read without path or paths saves no file: {arguments:?}"
                );
                read_with_paths |= arguments.iter().any(|a| a == "paths");
            }
        }
        assert!(read_with_paths);
        for scope in ["mine", "files", "assets"] {
            assert!(
                recipe.contains(&format!("`scope: \"{scope}\"`")),
                "the recipe lists {scope}"
            );
        }
        // A single page is read as a file, and the claim that a plain read saves one is
        // gone.
        assert!(recipe.contains("`paths: [\"index.html\"]`"), "{recipe}");
        assert!(!recipe.contains("names the saved file"), "{recipe}");
        assert!(recipe.contains("A read without `paths` gives the page inline and saves nothing"));
        // Assets only where the capability is declared, and the error is not obeyed.
        let assets = recipe.find("`scope: \"assets\"`").unwrap();
        let sentence_start = recipe[..assets].rfind(". ").unwrap();
        assert!(
            recipe[sentence_start..assets].contains("names the `assets` capability"),
            "the assets are listed only with the capability: {}",
            &recipe[sentence_start..assets]
        );
        assert!(recipe.contains("`capability_disabled`"));
        assert!(recipe.contains("never republish, edit or delete an artifact"));
        // The list gives a day: a copy of the same day or the next is not trusted.
        assert!(recipe.contains("two days or more after the day the list says it was updated"));
        // An artifact of a type is not read; a Docs document is exported.
        assert!(recipe.contains("The listing says \"an Artifact of type\""));
        assert!(recipe.contains("do not read its files"));
        assert!(recipe.contains("Claude Docs connector's `export` tool (`format: \"html\"`"));
    }

    /// What the text has to teach, whatever its wording.
    #[test]
    fn the_text_teaches_the_rules() {
        let text = spec().render();
        for needed in [
            "Exit code 5",
            "Say what you changed",
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
            .join("My Apps/Inner Word.app/Contents/MacOS/innerword");
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
