//! Whiteboards for Inner Word: the real Excalidraw built into one offline folder, and the Rust
//! half of its contract (`docs/whiteboard-design.md`, section 3 and 9.2 B3).
//!
//! The folder (`dist/`, committed) is built by `node build.mjs` in `web/` from Excalidraw
//! 0.18.1, React 19.2.9 and the glue around them (`web/src/glue/`): `index.html` is the
//! whiteboard's page in the window's view, `export.html` the page a PDF is made from, both
//! loading `assets/main.js`; `fonts/` holds Excalidraw's fonts without Xiaolai; `MANIFEST.json`
//! names every file with its size, SHA-256 and media type, `BUILD-INFO.json` what it was built
//! from, `NOTICES.txt` the licenses, `font-metrics.json` the advance widths `innerword-board`
//! measures text with. No cargo build and no test needs node or the network.
//!
//! What is here:
//!
//! - [`Assets`]: where the folder is and its files ([`Assets::find`]: `Contents/Resources/Whiteboard`
//!   in an app bundle, else this crate's `dist/`; never a folder a variable names).
//! - [`bridge`]: the messages the page posts through the one script message handler of a
//!   whiteboard's view ([`BRIDGE_NAME`]), read by [`bridge::parse`].
//! - [`calls`]: the bodies of the functions the app runs in the page
//!   (`callAsyncJavaScript` with one argument `arg`, the JSON text of the call's argument),
//!   their arguments and their answers.
//! - [`keys`]: the keys Excalidraw acts on that the window must leave to it on a whiteboard.
//!
//! The paths the view's source answers besides the bundle's files are the page's contract
//! too: [`BOARD_PATH`] and [`LIBRARY_PATH`] (the editor page), [`SCENE_PATH`] (the export
//! page), and [`FILES_PREFIX`] followed by a `FileId` (the stored bytes of a picture).

pub mod bridge;
pub mod calls;
pub mod keys;

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The version of Excalidraw in the bundle (`web/package.json`; a test holds the two equal,
/// and `innerword_board::EXCALIDRAW_VERSION` equal to this).
pub const EXCALIDRAW_VERSION: &str = "0.18.1";
/// The whiteboard's page in the window's view.
pub const ENTRY: &str = "index.html";
/// The page a whiteboard's PDF is made from, offscreen.
pub const EXPORT_ENTRY: &str = "export.html";
/// The export page's scene: the stored scene alone (`innerword_board::export_dir::SCENE_FILE`).
pub const SCENE_PATH: &str = "scene.json";
/// The view's scene: `{"stamp": "<ContentStamp>", "scene": <the stored scene>, "view":
/// <the row's view_state, or {}>}` (section 4.3), as the library holds it at that moment.
pub const BOARD_PATH: &str = "board.json";
/// The view's shape library: the setting `board.shape_library` as it is stored (section 4.15),
/// or `{}` when there is none.
pub const LIBRARY_PATH: &str = "library.json";
/// The advance widths of the bundled fonts, in the bundle (a copy is in `innerword-board`).
pub const FONT_METRICS: &str = "font-metrics.json";
/// `_files/<FileId>`: the stored bytes of a picture the scene names, served with the type of
/// the bytes. Only the ids the source was told about are answered.
pub const FILES_PREFIX: &str = "_files/";
/// The name of the one script message handler of a whiteboard's view.
pub const BRIDGE_NAME: &str = "innerwordBoard";
/// The bundle's list of its files.
pub const MANIFEST: &str = "MANIFEST.json";
/// The bundle's licenses (shipped beside the app's own third-party licenses).
pub const NOTICES: &str = "NOTICES.txt";

/// Where an app bundle keeps the folder: `Contents/Resources/Whiteboard`.
pub const BUNDLE_FOLDER: &str = "Whiteboard";

/// This crate's `dist/` by the path cargo gave at compile time, except in a build made with
/// `INNER_WORD_BUNDLED` set (`scripts/bundle.sh`), which compiles no path of the build
/// machine in (build.rs sets the `innerword_bundled` cfg).
#[cfg(not(innerword_bundled))]
const DEVELOPMENT_DIST: Option<&str> = Some(concat!(env!("CARGO_MANIFEST_DIR"), "/dist"));
#[cfg(innerword_bundled)]
const DEVELOPMENT_DIST: Option<&str> = None;

/// One file of the bundle as `MANIFEST.json` lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetInfo {
    /// What it is served as: `text/javascript; charset=utf-8`, `font/woff2`, ...
    pub media_type: String,
    /// Its size.
    pub bytes: u64,
    /// The SHA-256 of its bytes, 64 lowercase hex digits.
    pub sha256: String,
}

/// The bundle: its folder and the files its manifest lists. Only those files are served,
/// with the media types the manifest gives.
#[derive(Clone, Debug)]
pub struct Assets {
    root: PathBuf,
    files: BTreeMap<String, AssetInfo>,
}

#[derive(Deserialize)]
struct ManifestFile {
    path: String,
    bytes: u64,
    sha256: String,
    #[serde(rename = "type")]
    media_type: String,
}

#[derive(Deserialize)]
struct Manifest {
    files: Vec<ManifestFile>,
}

impl Assets {
    /// The bundle of this executable: `Contents/Resources/Whiteboard` when it runs from
    /// `<something>.app/Contents/MacOS/` (the bundled app and the bundled tool find the same
    /// folder, and look nowhere else), otherwise this crate's `dist/` (a `cargo build`, debug
    /// or release, and the tests). There is no variable that points it elsewhere: what runs
    /// in a whiteboard's view is what was signed into the app.
    pub fn find() -> Result<Assets, String> {
        let executable = std::env::current_exe().map_err(|error| missing(&format!("where this program is: {error}")))?;
        Assets::find_for(&executable)
    }

    /// [`Assets::find`] for an executable at `executable` (what `find` asks of the running
    /// program; tests name a made-up one).
    pub fn find_for(executable: &Path) -> Result<Assets, String> {
        if let Some(contents) = app_bundle_contents(executable) {
            return Assets::at(&contents.join("Resources").join(BUNDLE_FOLDER));
        }
        match DEVELOPMENT_DIST {
            Some(dist) => Assets::at(Path::new(dist)),
            None => Err(missing(&format!(
                "{} is not inside an app bundle, and this build looks nowhere else",
                executable.display()
            ))),
        }
    }

    /// The bundle at `root`: its `MANIFEST.json` read once. Refuses a folder without one.
    pub fn at(root: &Path) -> Result<Assets, String> {
        let manifest_path = root.join(MANIFEST);
        let text = std::fs::read_to_string(&manifest_path)
            .map_err(|error| missing(&format!("{}: {error}", manifest_path.display())))?;
        let manifest: Manifest = serde_json::from_str(&text)
            .map_err(|error| missing(&format!("{} cannot be read: {error}", manifest_path.display())))?;
        let mut files = BTreeMap::new();
        for file in manifest.files {
            if !is_plain_relative(&file.path) {
                return Err(missing(&format!("{} names a path outside its folder: {}", manifest_path.display(), file.path)));
            }
            files.insert(
                file.path,
                AssetInfo {
                    media_type: file.media_type,
                    bytes: file.bytes,
                    sha256: file.sha256,
                },
            );
        }
        Ok(Assets {
            root: root.to_path_buf(),
            files,
        })
    }

    /// The folder.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// A file of the bundle as the manifest lists it (`assets/main.js`), or `None`.
    pub fn get(&self, path: &str) -> Option<&AssetInfo> {
        self.files.get(path)
    }

    /// The bytes of a file of the bundle, on whatever thread asks (the scheme handler asks on
    /// a background queue). `None`: not a file of the bundle (`board.json`, `_files/…` and
    /// anything else is the caller's to answer). A file whose size is not the manifest's is
    /// an error, never served.
    pub fn read(&self, path: &str) -> Option<io::Result<Vec<u8>>> {
        let info = self.files.get(path)?;
        Some(std::fs::read(self.root.join(path)).and_then(|bytes| {
            if bytes.len() as u64 == info.bytes {
                Ok(bytes)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{path} is {} bytes, and the whiteboard's manifest says {}", bytes.len(), info.bytes),
                ))
            }
        }))
    }

    /// Every file of the bundle, in the order of their paths.
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.files.keys().map(String::as_str)
    }
}

/// The sentence of a bundle that cannot be found or read.
fn missing(detail: &str) -> String {
    format!("The whiteboard's files are missing from Inner Word ({detail}).")
}

/// `X.app/Contents` when `executable` is `X.app/Contents/MacOS/<name>`.
fn app_bundle_contents(executable: &Path) -> Option<PathBuf> {
    let macos = executable.parent()?;
    let contents = macos.parent()?;
    let app = contents.parent()?;
    let is_app = app.extension().is_some_and(|extension| extension == "app");
    (macos.file_name()? == "MacOS" && contents.file_name()? == "Contents" && is_app).then(|| contents.to_path_buf())
}

fn is_plain_relative(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && path.split('/').all(|part| !part.is_empty() && part != "." && part != "..")
}
