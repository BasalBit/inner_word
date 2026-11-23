//! What the tool hands its PDF child (`innerword __board-pdf <dir> <out.pdf>`): the stored
//! scene as `scene.json` and the bytes of each picture it names as `_files/<FileId>`
//! (`docs/whiteboard-design.md`, section 5). The export page reads the same two paths
//! from the app's source.

use std::path::Path;

use crate::FileHex;

/// The stored scene, as the export page asks for it (`innerword_board_web::SCENE_PATH`).
pub const SCENE_FILE: &str = "scene.json";
/// The folder of the pictures' bytes, each named by its `FileId` (no extension).
pub const FILES_DIR: &str = "_files";

/// Writes `scene` to `<dir>/scene.json` and each picture's bytes to `<dir>/_files/<id>`.
/// `dir` must exist; `_files` is made when a picture is written. A file already there is
/// written over.
pub fn write(
    dir: &Path,
    scene: &str,
    files: &mut dyn Iterator<Item = (FileHex, Vec<u8>)>,
) -> std::io::Result<()> {
    std::fs::write(dir.join(SCENE_FILE), scene.as_bytes())?;
    let mut made = false;
    for (id, bytes) in files {
        let folder = dir.join(FILES_DIR);
        if !made {
            std::fs::create_dir_all(&folder)?;
            made = true;
        }
        std::fs::write(folder.join(id.as_str()), bytes)?;
    }
    Ok(())
}
