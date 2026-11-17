//! Where the library lives on disk.
//!
//! By default in `~/Library/Application Support/Inner Word`. Before the app was called
//! Inner Word it was "The Writer", and so was the folder of its library: the first use of
//! [`data_dir`] after the new name moves that folder to the new name and leaves a link
//! under the old one (see [`data_dir`]).

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::error::{Result, StoreError};
use crate::instance_lock::{self, Acquired, HolderKind, RunningCopy};

/// Environment variable that overrides the data directory (used by tests and manual QA).
pub const DATA_DIR_ENV: &str = "INNER_WORD_DATA_DIR";

/// The name of the data directory in `~/Library/Application Support`.
pub const DATA_DIR_NAME: &str = "Inner Word";

/// The name the data directory had while the app was called The Writer (0.9.0 and
/// before). A folder of that name is moved to [`DATA_DIR_NAME`] by [`data_dir`], and a
/// link of that name to the new folder is left in its place.
pub const OLD_DATA_DIR_NAME: &str = "The Writer";

/// File name of the library database inside the data directory.
pub const DATABASE_FILE_NAME: &str = "library.db";

/// Name of the directory, next to the database, that holds the daily backups.
pub const BACKUPS_DIR_NAME: &str = "backups";

/// File name of the database that holds the bytes of the images, inside the data
/// directory beside the library database. The copy of the pictures in the backups
/// directory has the same name ([`images_backup_path`]).
pub const IMAGES_FILE_NAME: &str = "images.db";

/// How long the move of the old folder waits for a tool that holds the old library's lock
/// for one write (a tool of the old name working for an agent, or a second process of
/// this version that is moving the folder right now). The app waits as long for a tool
/// at launch (`single_instance::TOOL_PATIENCE`).
const MOVE_PATIENCE: Duration = Duration::from_secs(3);

/// How long a lock that claims to be the app's is looked at again before the old folder
/// is left where it is: the instant in which a new holder has the lock but has not yet
/// written its line, as in [`instance_lock::acquire_waiting`].
const APP_GRACE: Duration = Duration::from_millis(250);

/// How long the move sleeps between two looks at a held lock.
const RETRY_INTERVAL: Duration = Duration::from_millis(20);

/// The data directory: `$INNER_WORD_DATA_DIR` when set and non-empty, otherwise
/// `~/Library/Application Support/Inner Word`. The directory is not created.
///
/// Without the variable the library of the old name is brought over first, once:
///
/// - When `Inner Word` exists (whatever it is), it is the answer and nothing else is done.
/// - Else, when `The Writer` is a real folder (not a link), its lock
///   ([`instance_lock`], the lock the app and the tool take) is tried; a tool that holds it
///   for a write is waited for, three seconds at most. Held by an app (the version of the
///   old name is still running), or by a tool past that: the old folder is the answer for
///   this run and nothing is moved, so this launch finds that copy and a tool talks to
///   it. Free: the folder is renamed to `Inner Word` (the same volume,
///   one step), a relative link `The Writer` -> `Inner Word` is made in its place, so that
///   an older app and the `writer` tool of the old name that agents may still be set up
///   with find the same library and the same lock file, and the lock is let go.
/// - A link `The Writer` that leads to a folder is one the user made (the one left by the
///   move leads nowhere until `Inner Word` exists): it is the answer, and nothing is moved.
/// - Else `Inner Word` is the answer (and is not created here).
///
/// A rename that fails leaves the folder where it is, which is then the answer, and says
/// why on stderr; a link that cannot be made is said on stderr, the new folder being the
/// answer. Two processes of this version that look at the same moment both end with the
/// moved folder: the lock lets one move, and the other finds `Inner Word` afterwards.
pub fn data_dir() -> Result<PathBuf> {
    locate_data_dir(
        std::env::var_os(DATA_DIR_ENV),
        std::env::var_os("HOME"),
        MOVE_PATIENCE,
    )
}

/// The library database path: `data_dir()/library.db`.
pub fn database_path() -> Result<PathBuf> {
    Ok(data_dir()?.join(DATABASE_FILE_NAME))
}

/// The backups directory used for a database at `database_path`: a `backups` directory next
/// to it.
pub fn backups_dir(database_path: &Path) -> PathBuf {
    database_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default()
        .join(BACKUPS_DIR_NAME)
}

/// Where the bytes of the images of the library at `database_path` are stored:
/// `images.db` in the same directory. The file is there only once the library holds an
/// image.
pub fn images_path(database_path: &Path) -> PathBuf {
    database_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default()
        .join(IMAGES_FILE_NAME)
}

/// Where the one copy of the pictures is kept for a database at `database_path`:
/// `images.db` in its backups directory ([`backups_dir`]).
pub fn images_backup_path(database_path: &Path) -> PathBuf {
    backups_dir(database_path).join(IMAGES_FILE_NAME)
}

/// Where the data directory is by name, without looking at the disk: the variable when it
/// is set and non-empty, else `Inner Word` in the home directory's Application Support.
pub(crate) fn resolve_data_dir(env: Option<OsString>, home: Option<OsString>) -> Result<PathBuf> {
    if let Some(dir) = env.filter(|d| !d.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    match home.filter(|h| !h.is_empty()) {
        Some(home) => Ok(application_support(Path::new(&home)).join(DATA_DIR_NAME)),
        None => Err(StoreError::NoDataDirectory),
    }
}

/// [`data_dir`] with the variable, the home directory and the patience given: what the
/// tests call, with a home directory of their own.
pub(crate) fn locate_data_dir(
    env: Option<OsString>,
    home: Option<OsString>,
    patience: Duration,
) -> Result<PathBuf> {
    if env.as_ref().is_some_and(|dir| !dir.is_empty()) {
        return resolve_data_dir(env, None);
    }
    let home = home
        .filter(|h| !h.is_empty())
        .ok_or(StoreError::NoDataDirectory)?;
    Ok(bring_over_the_old_folder(
        &application_support(Path::new(&home)),
        patience,
    ))
}

fn application_support(home: &Path) -> PathBuf {
    home.join("Library").join("Application Support")
}

/// The data directory in `support` (the home directory's Application Support), the folder
/// of the old name moved to the new one first when that is how things stand. See
/// [`data_dir`].
fn bring_over_the_old_folder(support: &Path, patience: Duration) -> PathBuf {
    let new = support.join(DATA_DIR_NAME);
    let old = support.join(OLD_DATA_DIR_NAME);
    let started = Instant::now();
    loop {
        let waited = started.elapsed();
        match one_look(&old, &new) {
            Look::Use(dir) => return dir,
            Look::Held(copy) => {
                let limit = match copy.kind {
                    HolderKind::Tool => patience,
                    HolderKind::App => patience.min(APP_GRACE),
                };
                if waited >= limit {
                    // A copy of the app has the library open where it is. This run uses
                    // it there too (and finds that copy); a later one moves it.
                    return old;
                }
                std::thread::sleep(RETRY_INTERVAL.min(limit - waited));
            }
            // The old folder went away between two looks: another process moved it.
            Look::Again if waited < patience => std::thread::sleep(RETRY_INTERVAL),
            Look::Again => return if exists(&new) { new } else { old },
        }
    }
}

/// What one look at the two folders decided.
enum Look {
    /// The data directory is this one.
    Use(PathBuf),
    /// The lock of the old folder is held by another open file: a copy of the app, or a
    /// tool for one write.
    Held(RunningCopy),
    /// The old folder changed while it was looked at: look again.
    Again,
}

fn one_look(old: &Path, new: &Path) -> Look {
    if !surely_absent(new) {
        return Look::Use(new.to_path_buf());
    }
    match std::fs::symlink_metadata(old) {
        Ok(meta) if meta.is_dir() => {}
        // A link the user made to a library elsewhere (the one the move leaves leads
        // nowhere while the new folder is missing): keep reading through it.
        Ok(meta) if meta.file_type().is_symlink() && old.is_dir() => {
            return Look::Use(old.to_path_buf());
        }
        _ => return Look::Use(new.to_path_buf()),
    }
    // The lock line says "tool": a short holder, which a copy of the old app that starts
    // meanwhile waits for (and then finds the library through the link).
    let lock = match instance_lock::acquire_in_place(old, HolderKind::Tool) {
        Ok(Acquired::Lock(lock)) => lock,
        Ok(Acquired::Taken(copy)) => {
            // This process holds the old library already (the app took its lock after
            // an earlier answer of `old`): no waiting, no moving.
            if copy.kind == HolderKind::App && copy.pid == Some(std::process::id()) {
                return Look::Use(old.to_path_buf());
            }
            return Look::Held(copy);
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Look::Again,
        Err(error) => {
            eprintln!(
                "Inner Word: the library in {} is not moved to {}: whether another copy \
                 has it open cannot be checked ({error}). It is used where it is.",
                old.display(),
                new.display()
            );
            return Look::Use(old.to_path_buf());
        }
    };
    // Whoever moves holds this lock, so what is seen now stays so until it is let go.
    if !surely_absent(new) {
        return Look::Use(new.to_path_buf());
    }
    if !std::fs::symlink_metadata(old).is_ok_and(|meta| meta.is_dir()) {
        return Look::Again;
    }
    if let Err(error) = std::fs::rename(old, new) {
        if exists(new) {
            return Look::Use(new.to_path_buf());
        }
        eprintln!(
            "Inner Word: the library in {} could not be moved to {} ({error}). It is used \
             where it is.",
            old.display(),
            new.display()
        );
        return Look::Use(old.to_path_buf());
    }
    match std::os::unix::fs::symlink(DATA_DIR_NAME, old) {
        Ok(()) => eprintln!(
            "Inner Word: moved the library from {} to {}, and left a link to it under the \
             old name.",
            old.display(),
            new.display()
        ),
        Err(error) => eprintln!(
            "Inner Word: moved the library from {} to {}, but no link to it could be left \
             under the old name ({error}): a version of the app or of the `writer` tool \
             from before the new name will not find it.",
            old.display(),
            new.display()
        ),
    }
    drop(lock);
    Look::Use(new.to_path_buf())
}

/// Whether nothing at all is at `path`, not even a link that leads nowhere. An error other
/// than "not found" is not taken for absence.
fn surely_absent(path: &Path) -> bool {
    matches!(
        std::fs::symlink_metadata(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound
    )
}

/// Whether something (a folder, a file, a link) is at `path`.
fn exists(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_override_wins() {
        let dir = resolve_data_dir(Some("/tmp/x".into()), Some("/Users/me".into())).unwrap();
        assert_eq!(dir, PathBuf::from("/tmp/x"));
    }

    #[test]
    fn falls_back_to_application_support() {
        let dir = resolve_data_dir(Some("".into()), Some("/Users/me".into())).unwrap();
        assert_eq!(
            dir,
            PathBuf::from("/Users/me/Library/Application Support/Inner Word")
        );
    }

    #[test]
    fn no_home_is_an_error() {
        assert!(matches!(
            resolve_data_dir(None, None),
            Err(StoreError::NoDataDirectory)
        ));
    }

    #[test]
    fn the_images_are_beside_the_database_and_their_copy_among_the_backups() {
        assert_eq!(
            images_path(Path::new("/a/b/library.db")),
            PathBuf::from("/a/b/images.db")
        );
        assert_eq!(
            images_backup_path(Path::new("/a/b/library.db")),
            PathBuf::from("/a/b/backups/images.db")
        );
        assert_eq!(
            images_path(Path::new("library.db")),
            PathBuf::from("images.db")
        );
    }

    #[test]
    fn backups_dir_is_next_to_database() {
        assert_eq!(
            backups_dir(Path::new("/a/b/library.db")),
            PathBuf::from("/a/b/backups")
        );
    }

    // The move of the library of the old name. Every test has a home directory of its own;
    // none looks at the real one.

    use std::os::unix::fs::PermissionsExt;
    use std::sync::{Arc, Barrier};

    use tempfile::TempDir;

    /// A home directory with an Application Support folder in it.
    fn a_home() -> (TempDir, PathBuf) {
        let home = TempDir::new().unwrap();
        let support = application_support(home.path());
        std::fs::create_dir_all(&support).unwrap();
        (home, support)
    }

    fn locate(home: &TempDir, patience: Duration) -> PathBuf {
        locate_data_dir(None, Some(home.path().into()), patience).unwrap()
    }

    /// A library of the old name, as 0.9.0 left it: the database, its pictures, the
    /// backups with the copy of the pictures, and the lock file of a copy that has quit.
    fn old_library(support: &Path) -> PathBuf {
        let old = support.join(OLD_DATA_DIR_NAME);
        std::fs::create_dir_all(old.join(BACKUPS_DIR_NAME)).unwrap();
        std::fs::write(old.join(DATABASE_FILE_NAME), "the library").unwrap();
        std::fs::write(old.join(IMAGES_FILE_NAME), "the pictures").unwrap();
        std::fs::write(old.join(BACKUPS_DIR_NAME).join("library-1.db"), "a backup").unwrap();
        std::fs::write(
            old.join(BACKUPS_DIR_NAME).join(IMAGES_FILE_NAME),
            "their copy",
        )
        .unwrap();
        std::fs::write(old.join(instance_lock::LOCK_FILE_NAME), "4711\n").unwrap();
        old
    }

    fn is_real_dir(path: &Path) -> bool {
        std::fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir())
    }

    fn names_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn the_old_folder_is_moved_and_a_link_to_it_left_in_its_place() {
        let (home, support) = a_home();
        let old = old_library(&support);
        let new = support.join(DATA_DIR_NAME);

        assert_eq!(locate(&home, Duration::from_secs(2)), new);

        assert!(
            is_real_dir(&new),
            "the library is in the folder of the new name"
        );
        let link = std::fs::symlink_metadata(&old).unwrap();
        assert!(link.file_type().is_symlink(), "the old name is a link");
        assert_eq!(
            std::fs::read_link(&old).unwrap(),
            PathBuf::from(DATA_DIR_NAME),
            "the link is relative, so it holds wherever the home directory is"
        );
        // Everything came along, and reads the same by both names.
        for name in [DATABASE_FILE_NAME, IMAGES_FILE_NAME] {
            assert_eq!(
                std::fs::read(new.join(name)).unwrap(),
                std::fs::read(old.join(name)).unwrap()
            );
        }
        assert_eq!(
            std::fs::read_to_string(new.join(DATABASE_FILE_NAME)).unwrap(),
            "the library"
        );
        assert_eq!(
            std::fs::read_to_string(images_backup_path(&new.join(DATABASE_FILE_NAME))).unwrap(),
            "their copy"
        );
        assert_eq!(
            std::fs::read_to_string(new.join(BACKUPS_DIR_NAME).join("library-1.db")).unwrap(),
            "a backup"
        );
        // A write by the old name lands in the one library.
        std::fs::write(old.join(DATABASE_FILE_NAME), "changed by an old tool").unwrap();
        assert_eq!(
            std::fs::read_to_string(new.join(DATABASE_FILE_NAME)).unwrap(),
            "changed by an old tool"
        );
        // The lock was let go, and it is one lock file by both names.
        assert_eq!(instance_lock::holder(&new).unwrap(), None);
        let lock = instance_lock::acquire(&new).unwrap().lock().expect("free");
        assert!(matches!(
            instance_lock::acquire(&old).unwrap(),
            Acquired::Taken(_)
        ));
        drop(lock);

        // Once moved, it stays: the next look changes nothing.
        assert_eq!(locate(&home, Duration::from_secs(2)), new);
        assert_eq!(
            std::fs::read_link(&old).unwrap(),
            PathBuf::from(DATA_DIR_NAME)
        );
        assert_eq!(
            names_in(&support),
            vec![DATA_DIR_NAME.to_string(), OLD_DATA_DIR_NAME.to_string()]
        );
    }

    #[test]
    fn a_folder_of_the_new_name_is_used_and_nothing_is_moved() {
        let (home, support) = a_home();
        let old = old_library(&support);
        let new = support.join(DATA_DIR_NAME);
        std::fs::create_dir(&new).unwrap();
        std::fs::write(new.join(DATABASE_FILE_NAME), "the new library").unwrap();

        assert_eq!(locate(&home, Duration::from_secs(2)), new);

        assert!(is_real_dir(&old), "the old folder stays a folder");
        assert_eq!(
            std::fs::read_to_string(old.join(DATABASE_FILE_NAME)).unwrap(),
            "the library"
        );
        assert_eq!(
            std::fs::read_to_string(new.join(DATABASE_FILE_NAME)).unwrap(),
            "the new library"
        );
        assert_eq!(names_in(&new), vec![DATABASE_FILE_NAME.to_string()]);

        // Anything of the new name counts, a file or a link that leads nowhere too.
        let makers: [fn(&Path); 2] = [
            |p| std::fs::write(p, "a file").unwrap(),
            |p| std::os::unix::fs::symlink("nowhere", p).unwrap(),
        ];
        for make in makers {
            std::fs::remove_dir_all(&new).ok();
            std::fs::remove_file(&new).ok();
            make(&new);
            assert_eq!(locate(&home, Duration::from_secs(2)), new);
            assert!(is_real_dir(&old));
        }
    }

    #[test]
    fn a_copy_of_the_app_that_has_the_old_library_keeps_it_where_it_is() {
        let (home, support) = a_home();
        let old = old_library(&support);
        let new = support.join(DATA_DIR_NAME);
        // Another open file holds the lock, as the old app does (a second open file in
        // the same process is refused the same way), and its line names another process.
        let lock = instance_lock::acquire(&old).unwrap().lock().expect("free");
        std::fs::write(old.join(instance_lock::LOCK_FILE_NAME), "99999\n").unwrap();

        let started = Instant::now();
        assert_eq!(locate(&home, Duration::from_secs(10)), old);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "an app is not waited for beyond a moment"
        );
        assert!(is_real_dir(&old), "nothing was moved");
        assert!(!new.exists(), "nothing was made");

        // The app of this version holds it already (it was told `old` before, and took
        // the lock): the answer stays the same, at once.
        std::fs::write(
            old.join(instance_lock::LOCK_FILE_NAME),
            format!("{}\n", std::process::id()),
        )
        .unwrap();
        let started = Instant::now();
        assert_eq!(locate(&home, Duration::from_secs(10)), old);
        assert!(started.elapsed() < APP_GRACE);
        assert!(is_real_dir(&old));

        // Once the copy has quit, the next look moves it.
        drop(lock);
        assert_eq!(locate(&home, Duration::from_secs(2)), new);
        assert!(is_real_dir(&new));
    }

    #[test]
    fn a_tool_that_holds_the_lock_for_a_write_is_waited_for() {
        let (home, support) = a_home();
        let old = old_library(&support);
        let new = support.join(DATA_DIR_NAME);
        let lock = instance_lock::acquire_as(&old, HolderKind::Tool)
            .unwrap()
            .lock()
            .expect("free");
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            drop(lock);
        });

        assert_eq!(locate(&home, Duration::from_secs(10)), new);
        release.join().unwrap();
        assert!(is_real_dir(&new));
        assert!(
            std::fs::symlink_metadata(&old)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[test]
    fn a_tool_that_holds_on_past_the_patience_leaves_the_library_where_it_is() {
        let (home, support) = a_home();
        let old = old_library(&support);
        let new = support.join(DATA_DIR_NAME);
        let _lock = instance_lock::acquire_as(&old, HolderKind::Tool)
            .unwrap()
            .lock()
            .expect("free");

        assert_eq!(locate(&home, Duration::from_millis(100)), old);
        assert!(is_real_dir(&old));
        assert!(!new.exists());
    }

    #[test]
    fn a_link_of_the_old_name_moves_nothing() {
        // A link the user made to a library elsewhere is read through.
        let (home, support) = a_home();
        let elsewhere = TempDir::new().unwrap();
        std::fs::write(elsewhere.path().join(DATABASE_FILE_NAME), "far away").unwrap();
        let old = support.join(OLD_DATA_DIR_NAME);
        std::os::unix::fs::symlink(elsewhere.path(), &old).unwrap();
        let new = support.join(DATA_DIR_NAME);

        assert_eq!(locate(&home, Duration::from_secs(2)), old);
        assert_eq!(std::fs::read_link(&old).unwrap(), elsewhere.path());
        assert!(!new.exists());
        assert_eq!(
            names_in(elsewhere.path()),
            vec![DATABASE_FILE_NAME.to_string()]
        );

        // The link a move left, after the new folder went: it leads nowhere, and the
        // new name is the answer. Nothing is made.
        let (home, support) = a_home();
        let old = support.join(OLD_DATA_DIR_NAME);
        std::os::unix::fs::symlink(DATA_DIR_NAME, &old).unwrap();
        let new = support.join(DATA_DIR_NAME);

        assert_eq!(locate(&home, Duration::from_secs(2)), new);
        assert!(std::fs::symlink_metadata(&new).is_err());
        assert_eq!(
            std::fs::read_link(&old).unwrap(),
            PathBuf::from(DATA_DIR_NAME)
        );

        // A file of the old name is no library.
        let (home, support) = a_home();
        std::fs::write(support.join(OLD_DATA_DIR_NAME), "a file").unwrap();
        assert_eq!(
            locate(&home, Duration::from_secs(2)),
            support.join(DATA_DIR_NAME)
        );
        assert_eq!(names_in(&support), vec![OLD_DATA_DIR_NAME.to_string()]);
    }

    #[test]
    fn with_neither_folder_nothing_is_made() {
        let (home, support) = a_home();
        assert_eq!(
            locate(&home, Duration::from_secs(2)),
            support.join(DATA_DIR_NAME)
        );
        assert!(names_in(&support).is_empty());

        // Not even Application Support.
        let bare = TempDir::new().unwrap();
        assert_eq!(
            locate(&bare, Duration::from_secs(2)),
            application_support(bare.path()).join(DATA_DIR_NAME)
        );
        assert!(names_in(bare.path()).is_empty());
    }

    #[test]
    fn the_variable_moves_nothing() {
        let (home, support) = a_home();
        let old = old_library(&support);
        let elsewhere = TempDir::new().unwrap();

        let dir = locate_data_dir(
            Some(elsewhere.path().into()),
            Some(home.path().into()),
            Duration::from_secs(2),
        )
        .unwrap();
        assert_eq!(dir, elsewhere.path());
        assert!(is_real_dir(&old));
        assert!(!support.join(DATA_DIR_NAME).exists());
        assert!(names_in(elsewhere.path()).is_empty());

        // An empty variable is no variable.
        assert_eq!(
            locate_data_dir(
                Some("".into()),
                Some(home.path().into()),
                Duration::from_secs(2)
            )
            .unwrap(),
            support.join(DATA_DIR_NAME)
        );
        assert!(matches!(
            locate_data_dir(None, Some("".into()), Duration::from_secs(2)),
            Err(StoreError::NoDataDirectory)
        ));
    }

    #[test]
    fn a_rename_that_fails_leaves_the_library_where_it_is() {
        let (home, support) = a_home();
        let old = old_library(&support);
        let new = support.join(DATA_DIR_NAME);
        // Application Support cannot be written: the folder cannot be renamed in it.
        let writable = std::fs::metadata(&support).unwrap().permissions();
        std::fs::set_permissions(&support, std::fs::Permissions::from_mode(0o555)).unwrap();

        let found = locate(&home, Duration::from_secs(2));
        std::fs::set_permissions(&support, writable).unwrap();

        assert_eq!(found, old);
        assert!(is_real_dir(&old));
        assert!(!new.exists());
        // Its lock was let go.
        assert_eq!(instance_lock::holder(&old).unwrap(), None);
    }

    #[test]
    fn processes_that_look_at_the_same_moment_end_with_the_one_moved_library() {
        for _ in 0..20 {
            let (home, support) = a_home();
            let old = old_library(&support);
            let new = support.join(DATA_DIR_NAME);
            let home = Arc::new(home);
            let start = Arc::new(Barrier::new(8));
            let lookers: Vec<_> = (0..8)
                .map(|_| {
                    let (home, start) = (home.clone(), start.clone());
                    std::thread::spawn(move || {
                        start.wait();
                        locate(&home, Duration::from_secs(10))
                    })
                })
                .collect();
            for looker in lookers {
                assert_eq!(looker.join().unwrap(), new);
            }
            assert!(is_real_dir(&new));
            assert_eq!(
                std::fs::read_link(&old).unwrap(),
                PathBuf::from(DATA_DIR_NAME)
            );
            assert_eq!(
                std::fs::read_to_string(new.join(DATABASE_FILE_NAME)).unwrap(),
                "the library"
            );
            assert_eq!(
                names_in(&support),
                vec![DATA_DIR_NAME.to_string(), OLD_DATA_DIR_NAME.to_string()]
            );
            assert_eq!(instance_lock::holder(&new).unwrap(), None);
        }
    }
}
