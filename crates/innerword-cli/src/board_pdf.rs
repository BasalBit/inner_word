//! The child that makes the PDF of a whiteboard: `innerword __board-pdf <dir> <out.pdf>`
//! (docs/whiteboard-design.md, section 5, "For agents").
//!
//! `innerword export board:12 x.pdf` reads the stored scene, writes it and the bytes of its
//! pictures into a directory of its own (`innerword_board::export_dir::write`: `scene.json`
//! and `_files/<FileId>`) and runs this program again as its child
//! (`innerword_agent::ProcessChecker::print_board`), standard input closed, killed after
//! 60 s. The child loads the whiteboard's export page (`export.html` of the bundle,
//! `innerword_board_web::Assets::find`) offscreen in WebKit, served from the bundle and that
//! directory under the export page's policy, has it lay the drawing out as an SVG of
//! Excalidraw's (`calls::EXPORT_LAYOUT`) and writes WebKit's one-page PDF of exactly that
//! size into `out`. What went wrong is the last line of its standard error, which the
//! parent puts into its sentence ("the PDF of the whiteboard could not be made: …").
//!
//! Like the children of a page ([`crate::page_check`]), it is not in the table of commands
//! or in the help, and WebKit never runs in the tool or the MCP server themselves.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use innerword_agent::BOARD_PDF_ARGUMENT;
use innerword_board::export_dir::{FILES_DIR, SCENE_FILE};
use innerword_board_web::{Assets, EXPORT_ENTRY, FILES_PREFIX, SCENE_PATH, calls};
use innerword_web::{PageSource, ScriptInput, ServedFile};

use crate::page_check::{Failed, finish, rules_dir};

/// How long the load, the layout and the PDF may take together: under the 60 s after which
/// the parent kills the child, so that the child says why itself.
const LIMIT: Duration = Duration::from_secs(50);

/// Runs the child when the command line is one: `Some` with its exit code, `None` for any
/// other command line.
pub fn run(arguments: &[String]) -> Option<ExitCode> {
    let (first, rest) = arguments.split_first()?;
    if first != BOARD_PDF_ARGUMENT {
        return None;
    }
    Some(finish(match rest {
        [dir, out] => pdf(Path::new(dir), Path::new(out)),
        _ => Err(Failed::usage(format!(
            "usage: innerword {BOARD_PDF_ARGUMENT} <dir> <out.pdf>"
        ))),
    }))
}

/// `innerword __board-pdf <dir> <out.pdf>`: the PDF of the whiteboard whose stored scene and
/// pictures `dir` holds, written into exactly `out`.
fn pdf(dir: &Path, out: &Path) -> Result<(), Failed> {
    innerword_web::available().map_err(Failed::new)?;
    let assets = Assets::find().map_err(Failed::new)?;
    let source = BoardDirSource::new(assets, dir)?;
    let output = innerword_web::run_script_blocking(ScriptInput {
        source: Arc::new(source),
        entry: EXPORT_ENTRY.to_string(),
        script: calls::EXPORT_LAYOUT.to_string(),
        argument_json: calls::export_argument(),
        pdf: true,
        csp: innerword_web::board_export_policy(),
        limit: LIMIT,
        rules_dir: rules_dir()?,
    })
    .map_err(|why| Failed::new(format!("the export page failed ({why})")))?;
    let printed = output
        .pdf
        .ok_or_else(|| Failed::new("the export page made no PDF"))?;
    std::fs::write(out, &printed.bytes)
        .map_err(|error| Failed::new(format!("{} could not be written ({error})", out.display())))
}

/// What the export page is served from: the bundle's files, the stored scene as
/// `scene.json`, and the bytes of each picture the directory holds as `_files/<FileId>`,
/// served with the type of the bytes.
struct BoardDirSource {
    assets: Assets,
    scene: Arc<[u8]>,
    /// The pictures of the directory by their `FileId` (32 hex digits): only those are
    /// answered under `_files/`.
    pictures: HashMap<String, PathBuf>,
    /// The bundle's files read so far.
    cache: Mutex<HashMap<String, Arc<[u8]>>>,
}

impl BoardDirSource {
    fn new(assets: Assets, dir: &Path) -> Result<BoardDirSource, Failed> {
        let scene_path = dir.join(SCENE_FILE);
        let scene = std::fs::read(&scene_path).map_err(|error| {
            Failed::new(format!(
                "its scene could not be read from {} ({error})",
                scene_path.display()
            ))
        })?;
        let mut pictures = HashMap::new();
        let files = dir.join(FILES_DIR);
        if files.is_dir() {
            let entries = std::fs::read_dir(&files).map_err(|error| {
                Failed::new(format!(
                    "its pictures could not be read from {} ({error})",
                    files.display()
                ))
            })?;
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if is_file_id(&name) && entry.path().is_file() {
                    pictures.insert(name, entry.path());
                }
            }
        }
        Ok(BoardDirSource {
            assets,
            scene: Arc::from(scene),
            pictures,
            cache: Mutex::default(),
        })
    }

    fn picture(&self, hex: &str) -> Option<ServedFile> {
        let path = self.pictures.get(hex)?;
        let bytes: Arc<[u8]> = Arc::from(std::fs::read(path).ok()?);
        Some(ServedFile {
            media_type: picture_type(&bytes).to_string(),
            bytes,
        })
    }

    fn bundle_file(&self, path: &str) -> Option<ServedFile> {
        let info = self.assets.get(path)?;
        let media_type = info.media_type.clone();
        if let Some(bytes) = self.cache.lock().ok()?.get(path) {
            return Some(ServedFile {
                media_type,
                bytes: bytes.clone(),
            });
        }
        let bytes: Arc<[u8]> = Arc::from(self.assets.read(path)?.ok()?);
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(path.to_string(), bytes.clone());
        }
        Some(ServedFile { media_type, bytes })
    }
}

impl PageSource for BoardDirSource {
    fn file(&self, path: &str) -> Option<ServedFile> {
        let path = path.trim_start_matches('/');
        if path == SCENE_PATH {
            return Some(ServedFile {
                media_type: "application/json".to_string(),
                bytes: self.scene.clone(),
            });
        }
        if let Some(hex) = path.strip_prefix(FILES_PREFIX) {
            return self.picture(hex);
        }
        self.bundle_file(path)
    }
}

/// Whether `name` is a `FileId` as hex: 32 lowercase hex digits (and so no path).
fn is_file_id(name: &str) -> bool {
    name.len() == 32
        && name
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// The type a stored picture is served with: the type of its bytes (`image/svg+xml` for
/// what `sniff_svg` takes, else PNG, JPEG or GIF by their signature), never `text/html`.
fn picture_type(bytes: &[u8]) -> &'static str {
    if innerword_board::sniff_svg(bytes) {
        return "image/svg+xml";
    }
    match innerword_page::sniffed_type(bytes) {
        Some(kind) if kind.starts_with("image/") => kind,
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn other_command_lines_are_not_this_child() {
        assert!(run(&["list".to_string()]).is_none());
        assert!(run(&[]).is_none());
    }

    #[test]
    fn only_the_pictures_of_the_directory_are_served_with_the_type_of_their_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let scene = r#"{"type":"excalidraw","elements":[]}"#;
        let png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x02\0\0\0";
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"></svg>"#;
        let (a, b) = ("0".repeat(32), "a".repeat(32));
        innerword_board::export_dir::write(
            dir.path(),
            scene,
            &mut [
                (innerword_board::FileHex::parse(&a).unwrap(), png.to_vec()),
                (innerword_board::FileHex::parse(&b).unwrap(), svg.to_vec()),
            ]
            .into_iter(),
        )
        .unwrap();
        // Something else in the folder of the pictures is not a picture of the scene.
        std::fs::write(dir.path().join(FILES_DIR).join("notes.txt"), "x").unwrap();
        let assets = Assets::find().expect("the bundle of this checkout");
        let source = BoardDirSource::new(assets, dir.path()).unwrap();
        let served = source.file(SCENE_PATH).unwrap();
        assert_eq!(served.media_type, "application/json");
        assert_eq!(&*served.bytes, scene.as_bytes());
        assert_eq!(
            source
                .file(&format!("{FILES_PREFIX}{a}"))
                .unwrap()
                .media_type,
            "image/png"
        );
        assert_eq!(
            source
                .file(&format!("/{FILES_PREFIX}{b}"))
                .unwrap()
                .media_type,
            "image/svg+xml"
        );
        assert!(source.file(&format!("{FILES_PREFIX}notes.txt")).is_none());
        assert!(
            source
                .file(&format!("{FILES_PREFIX}{}", "c".repeat(32)))
                .is_none()
        );
        assert!(
            source
                .file(&format!("{FILES_PREFIX}../scene.json"))
                .is_none()
        );
        // The bundle's own files are answered with the types its manifest gives.
        let entry = source.file(EXPORT_ENTRY).expect("the export page");
        assert!(
            entry.media_type.starts_with("text/html"),
            "{}",
            entry.media_type
        );
        assert!(source.file("no/such/file.js").is_none());
    }

    #[test]
    fn a_directory_without_a_scene_is_said_to_be_so() {
        let dir = tempfile::tempdir().unwrap();
        let assets = Assets::find().expect("the bundle of this checkout");
        let failed = BoardDirSource::new(assets, dir.path()).err().unwrap();
        assert!(failed.reason().contains("its scene could not be read"));
    }
}
