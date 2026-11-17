//! Installing, updating and removing the skill in one skills directory.
//!
//! A skills directory (`~/.claude/skills`, `~/.agents/skills`, ...) belongs to the user and
//! to other tools. This module touches exactly one thing in it: the folder named
//! [`SKILL_NAME`], and in that folder exactly one file, `SKILL.md`.
//!
//! - Installing creates the directory if it is missing, the folder, and the file. Updating
//!   replaces the file (written beside it first and moved into place, so a reader never
//!   sees half of it).
//! - A folder of that name that this crate did not write (no `SKILL.md` with this crate's
//!   marker in it) is never written to and never removed: [`SetupError::Foreign`]. An empty
//!   folder of that name is taken over, since nothing in it can be lost.
//! - Removing deletes the `SKILL.md` this crate wrote and then the folder if nothing else is
//!   in it. Anything a person added to the folder stays, with the folder, and is reported.
//!   The skills directory itself is never removed.
//!
//! Every function reports exactly what it wrote, created and removed ([`Report`]).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::SetupError;
use crate::skill::{SKILL_FILE, SKILL_NAME, SkillMarker, SkillSpec, read_marker};

/// The state of the skill in one skills directory, measured against what would be installed
/// now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum SkillState {
    /// There is no folder of the skill's name (or the skills directory does not exist).
    NotInstalled,
    /// Installed, and the file is exactly what would be written now.
    Current,
    /// Installed by Inner Word, but not what would be written now.
    Stale(StaleSkill),
    /// A folder or file of the skill's name is there that Inner Word did not write.
    Foreign,
}

impl SkillState {
    /// Installed by Inner Word, current or not.
    pub fn is_installed(&self) -> bool {
        matches!(self, SkillState::Current | SkillState::Stale(_))
    }

    /// A word or two for a status column: "not installed", "installed", "needs update",
    /// "another folder is in the way".
    pub fn label(&self) -> &'static str {
        match self {
            SkillState::NotInstalled => "not installed",
            SkillState::Current => "installed",
            SkillState::Stale(_) => "needs update",
            SkillState::Foreign => "another folder is in the way",
        }
    }
}

/// An installed skill that is not what would be written now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StaleSkill {
    /// The version of the app that installed it.
    pub version: String,
    /// The binary its text names.
    pub binary: PathBuf,
    /// The most important way in which it differs.
    pub reason: StaleReason,
}

impl StaleSkill {
    /// One sentence for the interface.
    pub fn explanation(&self) -> String {
        match self.reason {
            StaleReason::BinaryMissing => {
                format!(
                    "It points at {}, which is no longer there.",
                    self.binary.display()
                )
            }
            StaleReason::OtherBinary => format!(
                "It points at another copy of the tool: {}.",
                self.binary.display()
            ),
            StaleReason::OtherVersion => format!("It was installed by version {}.", self.version),
            StaleReason::Edited => "Its text differs from what this version installs.".to_string(),
        }
    }
}

/// Why an installed skill is stale. When several hold, the first of this list is given.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum StaleReason {
    /// The binary it names does not exist any more (the app was moved or deleted). An agent
    /// following the skill would fail.
    BinaryMissing,
    /// The binary it names exists but is not the one of this copy of the app.
    OtherBinary,
    /// It was installed by another version of the app.
    OtherVersion,
    /// Version and binary are right, but the text is not what this build writes (somebody
    /// edited the file, or the text changed between two builds of one version).
    Edited,
}

/// What a call did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Change {
    /// The skill was not there and was written.
    Installed,
    /// The skill was there and its file was replaced.
    Updated,
    /// The skill was there and already what would be written: nothing was touched.
    Unchanged,
    /// The skill was removed.
    Removed,
    /// There was nothing of Inner Word's to remove: nothing was touched.
    NothingToRemove,
}

impl Change {
    /// A word for a report line: "installed", "updated", "already up to date", "removed",
    /// "not installed".
    pub fn label(self) -> &'static str {
        match self {
            Change::Installed => "installed",
            Change::Updated => "updated",
            Change::Unchanged => "already up to date",
            Change::Removed => "removed",
            Change::NothingToRemove => "not installed",
        }
    }
}

/// Exactly what a call did to the disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Report {
    /// The skills directory the call was about.
    pub skills_dir: PathBuf,
    /// The skill's folder in it.
    pub skill_dir: PathBuf,
    pub change: Change,
    /// What was installed there before, when it was Inner Word's and not current.
    pub replaced: Option<StaleSkill>,
    /// Files written, in order.
    pub written: Vec<PathBuf>,
    /// Directories created, outermost first.
    pub created_dirs: Vec<PathBuf>,
    /// Files and directories removed, in order.
    pub removed: Vec<PathBuf>,
    /// What was found in the skill's folder that Inner Word did not write and so left in
    /// place when removing (the folder then stays too).
    pub left_in_place: Vec<PathBuf>,
}

impl Report {
    fn new(skills_dir: &Path, change: Change) -> Report {
        Report {
            skills_dir: skills_dir.to_path_buf(),
            skill_dir: skill_dir(skills_dir),
            change,
            replaced: None,
            written: Vec::new(),
            created_dirs: Vec::new(),
            removed: Vec::new(),
            left_in_place: Vec::new(),
        }
    }

    /// Whether the call changed anything on disk.
    pub fn touched_the_disk(&self) -> bool {
        !(self.written.is_empty() && self.created_dirs.is_empty() && self.removed.is_empty())
    }

    /// Exactly what was done, one line per path, in the order it was done: "created
    /// folder ...", "wrote ...", "removed ...", "left in place ...". Empty when nothing on
    /// disk was touched.
    pub fn lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        lines.extend(
            self.created_dirs
                .iter()
                .map(|path| format!("created folder {}", path.display())),
        );
        lines.extend(
            self.written
                .iter()
                .map(|path| format!("wrote {}", path.display())),
        );
        lines.extend(
            self.removed
                .iter()
                .map(|path| format!("removed {}", path.display())),
        );
        lines.extend(
            self.left_in_place
                .iter()
                .map(|path| format!("left in place {}", path.display())),
        );
        lines
    }
}

/// The skill's folder in a skills directory.
pub fn skill_dir(skills_dir: &Path) -> PathBuf {
    skills_dir.join(SKILL_NAME)
}

/// The skill's file in a skills directory.
pub fn skill_file(skills_dir: &Path) -> PathBuf {
    skill_dir(skills_dir).join(SKILL_FILE)
}

/// What is in the skill's folder, as far as this crate cares.
enum Found {
    Nothing,
    /// A folder with nothing in it.
    EmptyFolder,
    /// A `SKILL.md` with this crate's marker, and its text.
    Ours(SkillMarker, String),
    Foreign,
}

fn look(skills_dir: &Path) -> Found {
    let folder = skill_dir(skills_dir);
    // `metadata` follows a link: a folder that is linked in from elsewhere counts as what it
    // points at. A link that points nowhere is something this crate did not make.
    let Ok(metadata) = fs::metadata(&folder) else {
        return if fs::symlink_metadata(&folder).is_ok() {
            Found::Foreign
        } else {
            Found::Nothing
        };
    };
    if !metadata.is_dir() {
        return Found::Foreign;
    }
    match fs::read_to_string(folder.join(SKILL_FILE)) {
        Ok(text) => match read_marker(&text) {
            Some(marker) => Found::Ours(marker, text),
            None => Found::Foreign,
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let empty = fs::read_dir(&folder)
                .map(|mut entries| entries.next().is_none())
                .unwrap_or(false);
            if empty {
                Found::EmptyFolder
            } else {
                Found::Foreign
            }
        }
        // Unreadable, or not text: not something to overwrite.
        Err(_) => Found::Foreign,
    }
}

/// How an installed skill differs from what `spec` would write; `None` when it does not.
///
/// A skill whose binary is gone is stale even when its text is what would be written: an
/// agent following it would fail.
fn staleness(marker: SkillMarker, text: &str, spec: &SkillSpec) -> Option<StaleSkill> {
    let reason = if !marker.binary.exists() {
        StaleReason::BinaryMissing
    } else if text == spec.render() {
        return None;
    } else if marker.binary != spec.binary() {
        StaleReason::OtherBinary
    } else if marker.version != spec.version() {
        StaleReason::OtherVersion
    } else {
        StaleReason::Edited
    };
    Some(StaleSkill {
        version: marker.version,
        binary: marker.binary,
        reason,
    })
}

/// What the skill in `skills_dir` says about itself, when a skill Inner Word wrote is there
/// (of whatever version, pointing at whatever binary). Reads only.
pub fn installed_marker(skills_dir: &Path) -> Option<SkillMarker> {
    match look(skills_dir) {
        Found::Ours(marker, _) => Some(marker),
        _ => None,
    }
}

/// The state of the skill in `skills_dir`. Reads only.
pub fn skill_state(skills_dir: &Path, spec: &SkillSpec) -> SkillState {
    match look(skills_dir) {
        Found::Nothing | Found::EmptyFolder => SkillState::NotInstalled,
        Found::Foreign => SkillState::Foreign,
        Found::Ours(marker, text) => match staleness(marker, &text, spec) {
            None => SkillState::Current,
            Some(stale) => SkillState::Stale(stale),
        },
    }
}

/// Installs the skill into `skills_dir`, or brings the installed one up to date.
///
/// Creates `skills_dir` when it does not exist. Refuses with [`SetupError::Foreign`] when a
/// folder of the skill's name is there that Inner Word did not write.
pub fn install_skill(skills_dir: &Path, spec: &SkillSpec) -> Result<Report, SetupError> {
    let text = spec.render();
    let folder = skill_dir(skills_dir);
    let file = folder.join(SKILL_FILE);
    let mut report = Report::new(skills_dir, Change::Installed);
    match look(skills_dir) {
        Found::Foreign => return Err(SetupError::Foreign(folder)),
        Found::Ours(_, installed) if installed == text => {
            report.change = Change::Unchanged;
            return Ok(report);
        }
        Found::Ours(marker, installed) => {
            report.change = Change::Updated;
            report.replaced = staleness(marker, &installed, spec);
        }
        Found::EmptyFolder => {}
        Found::Nothing => create_dirs(&folder, &mut report.created_dirs)?,
    }
    write_in_place(&file, &text)?;
    report.written.push(file);
    Ok(report)
}

/// Removes the skill from `skills_dir`.
///
/// Deletes the `SKILL.md` Inner Word wrote and the skill's folder when nothing else is in
/// it. Refuses with [`SetupError::Foreign`] when the folder of that name is not Inner
/// Word's. When nothing is installed, nothing happens ([`Change::NothingToRemove`]).
pub fn remove_skill(skills_dir: &Path) -> Result<Report, SetupError> {
    let folder = skill_dir(skills_dir);
    let file = folder.join(SKILL_FILE);
    let mut report = Report::new(skills_dir, Change::Removed);
    match look(skills_dir) {
        Found::Nothing | Found::EmptyFolder => {
            report.change = Change::NothingToRemove;
            return Ok(report);
        }
        Found::Foreign => return Err(SetupError::Foreign(folder)),
        Found::Ours(..) => {}
    }
    fs::remove_file(&file).map_err(|error| SetupError::io("could not remove", &file, error))?;
    report.removed.push(file);

    let mut rest: Vec<PathBuf> = fs::read_dir(&folder)
        .map_err(|error| SetupError::io("could not read", &folder, error))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .collect();
    rest.sort();
    if !rest.is_empty() {
        report.left_in_place = rest;
        return Ok(report);
    }
    let is_link = fs::symlink_metadata(&folder)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false);
    if is_link {
        // The folder is linked in from elsewhere: the link is somebody's arrangement and
        // stays; the now empty folder it points at stays with it.
        report.left_in_place.push(folder);
        return Ok(report);
    }
    fs::remove_dir(&folder).map_err(|error| SetupError::io("could not remove", &folder, error))?;
    report.removed.push(folder);
    Ok(report)
}

/// Creates `folder` and the folders above it that are missing, recording each one made.
fn create_dirs(folder: &Path, created: &mut Vec<PathBuf>) -> Result<(), SetupError> {
    let mut missing = Vec::new();
    let mut current = Some(folder);
    while let Some(path) = current {
        if path.as_os_str().is_empty() || path.exists() {
            break;
        }
        missing.push(path.to_path_buf());
        current = path.parent();
    }
    for path in missing.into_iter().rev() {
        match fs::create_dir(&path) {
            Ok(()) => created.push(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(SetupError::io("could not create", path, error)),
        }
    }
    Ok(())
}

/// Writes `text` to `file` by way of a file beside it, so `file` is either the old text or
/// the new one, never a part.
fn write_in_place(file: &Path, text: &str) -> Result<(), SetupError> {
    let folder = file.parent().unwrap_or(Path::new("."));
    let temporary = folder.join(format!(".{SKILL_FILE}.{}.tmp", std::process::id()));
    let result = fs::write(&temporary, text).and_then(|()| fs::rename(&temporary, file));
    if let Err(error) = result {
        let _ = fs::remove_file(&temporary);
        return Err(SetupError::io("could not write", file, error));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skill::APP_VERSION;
    use tempfile::TempDir;

    /// A home with a fake app in it, whose path has spaces, and a skills directory.
    struct Fixture {
        home: TempDir,
    }

    impl Fixture {
        fn new() -> Fixture {
            let home = tempfile::Builder::new()
                .prefix("the innerword home ")
                .tempdir()
                .unwrap();
            let fixture = Fixture { home };
            fs::create_dir_all(fixture.binary().parent().unwrap()).unwrap();
            fs::write(fixture.binary(), "#!/bin/sh\n").unwrap();
            fixture
        }

        fn binary(&self) -> PathBuf {
            self.home
                .path()
                .join("Applications/Inner Word.app/Contents/MacOS/innerword")
        }

        fn spec(&self) -> SkillSpec {
            SkillSpec::new(self.binary()).unwrap()
        }

        fn skills(&self) -> PathBuf {
            self.home.path().join(".agents/skills")
        }

        /// Every path below the home, sorted, for "nothing else was touched".
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
            walk(self.home.path(), &mut out);
            out.sort();
            out
        }
    }

    #[test]
    fn install_into_a_directory_that_does_not_exist_creates_it_and_says_so() {
        let fixture = Fixture::new();
        let skills = fixture.skills();
        assert_eq!(
            skill_state(&skills, &fixture.spec()),
            SkillState::NotInstalled
        );

        let report = install_skill(&skills, &fixture.spec()).unwrap();
        assert_eq!(report.change, Change::Installed);
        assert_eq!(report.skills_dir, skills);
        assert_eq!(report.skill_dir, skills.join("inner-word"));
        assert_eq!(report.written, vec![skills.join("inner-word/SKILL.md")]);
        assert_eq!(
            report.created_dirs,
            vec![
                fixture.home.path().join(".agents"),
                skills.clone(),
                skills.join("inner-word")
            ]
        );
        assert!(
            report.removed.is_empty()
                && report.left_in_place.is_empty()
                && report.replaced.is_none()
        );
        assert!(report.touched_the_disk());

        let text = fs::read_to_string(skills.join("inner-word/SKILL.md")).unwrap();
        assert_eq!(text, fixture.spec().render());
        assert!(
            text.contains(&format!("'{}'", fixture.binary().display())),
            "the path with spaces, quoted"
        );
        assert_eq!(skill_state(&skills, &fixture.spec()), SkillState::Current);
        // Only the file is in the folder: no temporary file is left.
        assert_eq!(fs::read_dir(skills.join("inner-word")).unwrap().count(), 1);
    }

    #[test]
    fn install_touches_nothing_else_in_the_skills_directory() {
        let fixture = Fixture::new();
        let skills = fixture.skills();
        fs::create_dir_all(skills.join("somebody-elses-skill")).unwrap();
        fs::write(
            skills.join("somebody-elses-skill/SKILL.md"),
            "---\nname: somebody-elses-skill\n---\n",
        )
        .unwrap();
        fs::write(skills.join("notes.txt"), "mine").unwrap();
        let before = fixture.everything();

        let report = install_skill(&skills, &fixture.spec()).unwrap();
        assert_eq!(report.created_dirs, vec![skills.join("inner-word")]);
        let mut expected = before.clone();
        expected.extend([
            skills.join("inner-word"),
            skills.join("inner-word/SKILL.md"),
        ]);
        expected.sort();
        assert_eq!(fixture.everything(), expected);
        assert_eq!(
            fs::read_to_string(skills.join("notes.txt")).unwrap(),
            "mine"
        );

        let report = remove_skill(&skills).unwrap();
        assert_eq!(report.change, Change::Removed);
        assert_eq!(
            report.removed,
            vec![
                skills.join("inner-word/SKILL.md"),
                skills.join("inner-word")
            ]
        );
        assert_eq!(
            fixture.everything(),
            before,
            "removing leaves the directory as it was found"
        );
    }

    #[test]
    fn installing_twice_changes_nothing_the_second_time() {
        let fixture = Fixture::new();
        install_skill(&fixture.skills(), &fixture.spec()).unwrap();
        let file = skill_file(&fixture.skills());
        let modified = fs::metadata(&file).unwrap().modified().unwrap();

        let report = install_skill(&fixture.skills(), &fixture.spec()).unwrap();
        assert_eq!(report.change, Change::Unchanged);
        assert!(!report.touched_the_disk());
        assert_eq!(fs::metadata(&file).unwrap().modified().unwrap(), modified);
    }

    #[test]
    fn a_skill_from_another_version_is_stale_and_updated() {
        let fixture = Fixture::new();
        let skills = fixture.skills();
        install_skill(&skills, &fixture.spec().with_version("0.1.0")).unwrap();

        let SkillState::Stale(stale) = skill_state(&skills, &fixture.spec()) else {
            panic!("not stale")
        };
        assert_eq!(stale.reason, StaleReason::OtherVersion);
        assert_eq!(stale.version, "0.1.0");
        assert_eq!(stale.binary, fixture.binary());
        assert!(stale.explanation().contains("0.1.0"));

        let report = install_skill(&skills, &fixture.spec()).unwrap();
        assert_eq!(report.change, Change::Updated);
        assert_eq!(report.replaced, Some(stale));
        assert_eq!(report.written, vec![skill_file(&skills)]);
        assert!(report.created_dirs.is_empty());
        assert_eq!(skill_state(&skills, &fixture.spec()), SkillState::Current);
        assert_eq!(
            read_marker(&fs::read_to_string(skill_file(&skills)).unwrap())
                .unwrap()
                .version,
            APP_VERSION
        );
    }

    #[test]
    fn a_skill_that_points_at_a_binary_that_is_gone_is_stale() {
        let fixture = Fixture::new();
        let skills = fixture.skills();
        let old_binary = fixture
            .home
            .path()
            .join("Downloads/Inner Word.app/Contents/MacOS/innerword");
        fs::create_dir_all(old_binary.parent().unwrap()).unwrap();
        fs::write(&old_binary, "").unwrap();
        install_skill(&skills, &SkillSpec::new(&old_binary).unwrap()).unwrap();

        // The other copy still exists: the skill works, but is not this app's.
        let SkillState::Stale(stale) = skill_state(&skills, &fixture.spec()) else {
            panic!("not stale")
        };
        assert_eq!(stale.reason, StaleReason::OtherBinary);

        // The app was moved to /Applications and the old copy deleted.
        fs::remove_dir_all(fixture.home.path().join("Downloads")).unwrap();
        let SkillState::Stale(stale) = skill_state(&skills, &fixture.spec()) else {
            panic!("not stale")
        };
        assert_eq!(stale.reason, StaleReason::BinaryMissing);
        assert_eq!(stale.binary, old_binary);
        assert!(stale.explanation().contains("no longer there"));

        // Also when the installed copy is otherwise exactly what would be written: an agent
        // following it would fail. Installing again has nothing to change.
        install_skill(&skills, &fixture.spec()).unwrap();
        fs::remove_file(fixture.binary()).unwrap();
        let SkillState::Stale(stale) = skill_state(&skills, &fixture.spec()) else {
            panic!("not stale")
        };
        assert_eq!(stale.reason, StaleReason::BinaryMissing);
        assert_eq!(
            install_skill(&skills, &fixture.spec()).unwrap().change,
            Change::Unchanged
        );
    }

    #[test]
    fn an_edited_skill_is_stale_and_update_restores_it() {
        let fixture = Fixture::new();
        let skills = fixture.skills();
        install_skill(&skills, &fixture.spec()).unwrap();
        let file = skill_file(&skills);
        let edited = fs::read_to_string(&file).unwrap() + "\nA line somebody added.\n";
        fs::write(&file, edited).unwrap();

        let SkillState::Stale(stale) = skill_state(&skills, &fixture.spec()) else {
            panic!("not stale")
        };
        assert_eq!(stale.reason, StaleReason::Edited);
        assert_eq!(
            install_skill(&skills, &fixture.spec()).unwrap().change,
            Change::Updated
        );
        assert_eq!(fs::read_to_string(&file).unwrap(), fixture.spec().render());
    }

    #[test]
    fn a_folder_somebody_else_made_is_never_overwritten_or_removed() {
        let fixture = Fixture::new();
        let skills = fixture.skills();
        let folder = skills.join("inner-word");

        // A skill of the same name written by a person or another tool.
        fs::create_dir_all(&folder).unwrap();
        fs::write(
            folder.join("SKILL.md"),
            "---\nname: inner-word\ndescription: My own notes.\n---\nMine.\n",
        )
        .unwrap();
        let before = fixture.everything();
        assert_eq!(skill_state(&skills, &fixture.spec()), SkillState::Foreign);
        let error = install_skill(&skills, &fixture.spec()).unwrap_err();
        assert!(
            matches!(&error, SetupError::Foreign(path) if path == &folder),
            "{error}"
        );
        assert!(
            error
                .to_string()
                .contains("was not installed by Inner Word"),
            "{error}"
        );
        assert!(matches!(remove_skill(&skills), Err(SetupError::Foreign(_))));
        assert_eq!(fixture.everything(), before);
        assert!(
            fs::read_to_string(folder.join("SKILL.md"))
                .unwrap()
                .ends_with("Mine.\n")
        );

        // A folder with files but no SKILL.md.
        fs::remove_file(folder.join("SKILL.md")).unwrap();
        fs::write(folder.join("draft.md"), "notes").unwrap();
        assert_eq!(skill_state(&skills, &fixture.spec()), SkillState::Foreign);
        assert!(matches!(
            install_skill(&skills, &fixture.spec()),
            Err(SetupError::Foreign(_))
        ));
        assert!(matches!(remove_skill(&skills), Err(SetupError::Foreign(_))));
        assert_eq!(
            fs::read_to_string(folder.join("draft.md")).unwrap(),
            "notes"
        );

        // A file of that name.
        fs::remove_dir_all(&folder).unwrap();
        fs::write(&folder, "a file").unwrap();
        assert_eq!(skill_state(&skills, &fixture.spec()), SkillState::Foreign);
        assert!(matches!(
            install_skill(&skills, &fixture.spec()),
            Err(SetupError::Foreign(_))
        ));
        assert!(matches!(remove_skill(&skills), Err(SetupError::Foreign(_))));
        assert_eq!(fs::read_to_string(&folder).unwrap(), "a file");

        // A link that points nowhere.
        fs::remove_file(&folder).unwrap();
        std::os::unix::fs::symlink(fixture.home.path().join("gone"), &folder).unwrap();
        assert_eq!(skill_state(&skills, &fixture.spec()), SkillState::Foreign);
        assert!(matches!(
            install_skill(&skills, &fixture.spec()),
            Err(SetupError::Foreign(_))
        ));
        assert!(fs::symlink_metadata(&folder).is_ok());
    }

    #[test]
    fn an_empty_folder_of_that_name_is_taken_over() {
        let fixture = Fixture::new();
        let skills = fixture.skills();
        fs::create_dir_all(skills.join("inner-word")).unwrap();
        assert_eq!(
            skill_state(&skills, &fixture.spec()),
            SkillState::NotInstalled
        );
        assert_eq!(
            remove_skill(&skills).unwrap().change,
            Change::NothingToRemove
        );
        assert!(
            skills.join("inner-word").is_dir(),
            "an empty folder that was there stays"
        );

        let report = install_skill(&skills, &fixture.spec()).unwrap();
        assert_eq!(report.change, Change::Installed);
        assert!(report.created_dirs.is_empty());
        assert_eq!(skill_state(&skills, &fixture.spec()), SkillState::Current);
    }

    #[test]
    fn removing_what_is_not_there_does_nothing() {
        let fixture = Fixture::new();
        let before = fixture.everything();
        let report = remove_skill(&fixture.skills()).unwrap();
        assert_eq!(report.change, Change::NothingToRemove);
        assert!(!report.touched_the_disk());
        assert_eq!(
            fixture.everything(),
            before,
            "not even the skills directory is created"
        );
    }

    #[test]
    fn removing_keeps_what_a_person_added_to_the_folder() {
        let fixture = Fixture::new();
        let skills = fixture.skills();
        install_skill(&skills, &fixture.spec()).unwrap();
        let extra = skills.join("inner-word/my-notes.md");
        fs::write(&extra, "keep me").unwrap();

        let report = remove_skill(&skills).unwrap();
        assert_eq!(report.change, Change::Removed);
        assert_eq!(report.removed, vec![skill_file(&skills)]);
        assert_eq!(report.left_in_place, vec![extra.clone()]);
        assert_eq!(fs::read_to_string(&extra).unwrap(), "keep me");
        // What is left is no longer Inner Word's.
        assert_eq!(skill_state(&skills, &fixture.spec()), SkillState::Foreign);
    }

    #[test]
    fn a_skills_directory_that_is_a_link_is_written_through() {
        let fixture = Fixture::new();
        let real = fixture.home.path().join(".agents/skills");
        fs::create_dir_all(&real).unwrap();
        fs::create_dir_all(fixture.home.path().join(".claude")).unwrap();
        let linked = fixture.home.path().join(".claude/skills");
        std::os::unix::fs::symlink(&real, &linked).unwrap();

        install_skill(&linked, &fixture.spec()).unwrap();
        assert_eq!(skill_state(&real, &fixture.spec()), SkillState::Current);
        assert_eq!(skill_state(&linked, &fixture.spec()), SkillState::Current);
        assert!(
            fs::symlink_metadata(&linked)
                .unwrap()
                .file_type()
                .is_symlink(),
            "the link is still a link"
        );

        remove_skill(&real).unwrap();
        assert_eq!(
            skill_state(&linked, &fixture.spec()),
            SkillState::NotInstalled
        );
    }

    #[test]
    fn a_skill_folder_that_is_a_link_to_our_own_folder_is_updated_and_the_link_stays() {
        let fixture = Fixture::new();
        let real = fixture.home.path().join(".agents/skills");
        install_skill(&real, &fixture.spec().with_version("0.1.0")).unwrap();
        let claude = fixture.home.path().join(".claude/skills");
        fs::create_dir_all(&claude).unwrap();
        std::os::unix::fs::symlink(real.join("inner-word"), claude.join("inner-word")).unwrap();

        assert!(matches!(
            skill_state(&claude, &fixture.spec()),
            SkillState::Stale(_)
        ));
        assert_eq!(
            install_skill(&claude, &fixture.spec()).unwrap().change,
            Change::Updated
        );
        assert_eq!(skill_state(&real, &fixture.spec()), SkillState::Current);

        let report = remove_skill(&claude).unwrap();
        assert_eq!(report.removed, vec![claude.join("inner-word/SKILL.md")]);
        assert_eq!(report.left_in_place, vec![claude.join("inner-word")]);
        assert!(
            fs::symlink_metadata(claude.join("inner-word"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(!real.join("inner-word/SKILL.md").exists());
    }

    #[test]
    fn a_directory_that_cannot_be_written_is_an_error_that_names_the_path() {
        let fixture = Fixture::new();
        let blocked = fixture.home.path().join("blocked");
        fs::write(&blocked, "a file where a folder should be").unwrap();
        let error = install_skill(&blocked.join("skills"), &fixture.spec()).unwrap_err();
        assert!(matches!(error, SetupError::Io { .. }), "{error}");
        assert!(error.to_string().contains("blocked"), "{error}");
    }

    #[test]
    fn states_and_changes_have_words() {
        assert_eq!(SkillState::NotInstalled.label(), "not installed");
        assert!(SkillState::Current.is_installed());
        assert!(!SkillState::Foreign.is_installed());
        assert_eq!(Change::Unchanged.label(), "already up to date");
        let json = serde_json::to_value(SkillState::Current).unwrap();
        assert_eq!(json["state"], "current");
    }
}
