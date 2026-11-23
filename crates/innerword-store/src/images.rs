//! The bytes of the images that documents hold, and of the files of pages.
//!
//! A document names an image by the hash of its bytes ([`ImageId`]); the bytes are stored
//! once, in a second SQLite file beside the library database (`images.db`,
//! [`crate::images_path`]). The file is there only once the library holds an image.
//!
//! Why a second file: the daily backup is a copy of the whole library database, made
//! before the window can open, and seven are kept. Pictures in that file would be copied
//! at the first launch of every day and stored eight times; and the library database
//! changes with every autosave, so a backup of files (Time Machine) would copy the
//! pictures again each time. Pictures are written once and never change.
//!
//! The parts:
//!
//! - [`ImageFile`] (`file.rs`): `images.db` by itself, one connection, its schema and
//!   pragmas. A [`Store`] holds one and opens it at the first use. Since version 2 of the
//!   file it has a second table, `files`: the bytes of the files of pages ([`FileId`],
//!   [`FileInfo`]), stored once each under their hash, as the pictures are.
//! - `scan.rs`: which images and which files stored content names. The one thing the
//!   store reads in a document's content or a page's manifest.
//! - [`collect_unused_images`], [`mark_unused_images`] (`collect.rs`): the collector, the
//!   only thing that ever deletes the bytes of an image.
//! - [`back_up_images`] (`copy.rs`): the one copy of the pictures in the backups
//!   directory, and reading from it when `images.db` does not have an image.

mod collect;
mod copy;
mod file;
mod scan;

use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::Store;
use crate::error::Result;
use crate::paths::{images_backup_path, images_path};

pub use collect::{IMAGE_RETENTION_MS, ImagesCollected, collect_unused_images, mark_unused_images};
pub use copy::{IMAGE_BACKUP_SLICE_BYTES, ImagesBackedUp, back_up_all_images, back_up_images};
pub use file::ImageFile;

/// The images databases of the library at `database` that were found damaged and moved
/// aside (`images.db.damaged-<unix ms>` beside the library, the same in the backups
/// directory for the copy of the pictures), oldest first. Their pictures came back from
/// the other file; what neither had is only there, so they are kept for the writer (or
/// whoever helps them) to look at, and the app says that they are there.
pub fn images_moved_aside(database: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for dir in [images_path(database), images_backup_path(database)]
        .iter()
        .filter_map(|path| path.parent().map(Path::to_path_buf))
    {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let prefix = format!("{}{}", crate::paths::IMAGES_FILE_NAME, file::DAMAGED_MARK);
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let Some(rest) = name.strip_prefix(&prefix) else {
                continue;
            };
            if rest.ends_with("-wal") || rest.ends_with("-shm") {
                continue;
            }
            let stamp = rest.split('-').next().and_then(|ms| ms.parse::<i64>().ok());
            found.push((stamp.unwrap_or(0), entry.path()));
        }
    }
    found.sort();
    found.into_iter().map(|(_, path)| path).collect()
}

/// The name of stored image bytes: the first 16 bytes of their SHA-256. Written as 32
/// lowercase hex digits wherever it is text (the column, a document's JSON, Markdown).
///
/// `innerword-core` has a type of the same name and the same 16 bytes for the id inside a
/// document; this crate does not depend on it, and the two meet in the hex digits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ImageId(pub [u8; 16]);

impl ImageId {
    /// The id of these bytes.
    pub fn of(bytes: &[u8]) -> ImageId {
        ImageId(hash16(bytes))
    }

    /// Reads exactly 32 hex digits, in either case. Anything else is `None`.
    pub fn parse(text: &str) -> Option<ImageId> {
        Self::from_hex(text.as_bytes())
    }

    /// The id as 32 lowercase hex digits. [`fmt::Display`] writes the same.
    pub fn to_hex(&self) -> String {
        hex16(&self.0)
    }

    /// 32 hex digits as bytes of text. The scan of stored content reads ids with it.
    pub(crate) fn from_hex(digits: &[u8]) -> Option<ImageId> {
        parse16(digits).map(ImageId)
    }
}

impl fmt::Display for ImageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

/// The name of the stored bytes of a file of a page (a style sheet, a font, a picture, a
/// sound): the first 16 bytes of their SHA-256, the rule of [`ImageId`], in a type of its
/// own so that the two are never mixed. Written as 32 lowercase hex digits wherever it is
/// text: the column of `files`, and the `"file"` key of a page's manifest, the one token
/// the store reads in it ([`Store::referenced_files`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FileId(pub [u8; 16]);

impl FileId {
    /// The id of these bytes.
    pub fn of(bytes: &[u8]) -> FileId {
        FileId(hash16(bytes))
    }

    /// Reads exactly 32 hex digits, in either case. Anything else is `None`.
    pub fn parse(text: &str) -> Option<FileId> {
        Self::from_hex(text.as_bytes())
    }

    /// The id as 32 lowercase hex digits. [`fmt::Display`] writes the same.
    pub fn to_hex(&self) -> String {
        hex16(&self.0)
    }

    /// 32 hex digits as bytes of text. The scan of stored content reads ids with it.
    pub(crate) fn from_hex(digits: &[u8]) -> Option<FileId> {
        parse16(digits).map(FileId)
    }
}

impl fmt::Display for FileId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

/// What the library knows about the stored bytes of a file of a page without reading them.
/// No media type: the same bytes can be served under two paths with two types, and the
/// manifest of the page says the type of each path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileInfo {
    /// The name of the bytes.
    pub id: FileId,
    /// How many bytes there are.
    pub byte_len: u64,
    /// When the bytes were first stored, unix ms.
    pub created_at: i64,
}

/// The first 16 bytes of the SHA-256 of `bytes`: the name of stored bytes, an image's or a
/// file's.
pub(crate) fn hash16(bytes: &[u8]) -> [u8; 16] {
    let digest = Sha256::digest(bytes);
    let mut id = [0u8; 16];
    id.copy_from_slice(&digest[..16]);
    id
}

/// 16 bytes as 32 lowercase hex digits.
pub(crate) fn hex16(id: &[u8; 16]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut hex = String::with_capacity(32);
    for byte in id {
        hex.push(DIGITS[usize::from(byte >> 4)] as char);
        hex.push(DIGITS[usize::from(byte & 0x0f)] as char);
    }
    hex
}

/// Exactly 32 hex digits, in either case, as 16 bytes.
pub(crate) fn parse16(digits: &[u8]) -> Option<[u8; 16]> {
    fn value(digit: u8) -> Option<u8> {
        match digit {
            b'0'..=b'9' => Some(digit - b'0'),
            b'a'..=b'f' => Some(digit - b'a' + 10),
            b'A'..=b'F' => Some(digit - b'A' + 10),
            _ => None,
        }
    }
    if digits.len() != 32 {
        return None;
    }
    let mut id = [0u8; 16];
    for (byte, pair) in id.iter_mut().zip(digits.chunks_exact(2)) {
        *byte = (value(pair[0])? << 4) | value(pair[1])?;
    }
    Some(id)
}

/// What kind of file stored image bytes are. The library stores these three and nothing
/// else; `innerword-image` turns every other kind into one of them on the way in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ImageFormat {
    /// PNG.
    Png,
    /// JPEG.
    Jpeg,
    /// GIF, animated or not.
    Gif,
}

impl ImageFormat {
    /// The name in the `format` column: `"png"`, `"jpeg"` or `"gif"`.
    pub fn as_str(self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpeg",
            ImageFormat::Gif => "gif",
        }
    }

    /// The extension a file of this kind gets: `"png"`, `"jpg"` or `"gif"`.
    pub fn extension(self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpg",
            ImageFormat::Gif => "gif",
        }
    }

    /// The media type: `"image/png"`, `"image/jpeg"` or `"image/gif"`.
    pub fn mime_type(self) -> &'static str {
        match self {
            ImageFormat::Png => "image/png",
            ImageFormat::Jpeg => "image/jpeg",
            ImageFormat::Gif => "image/gif",
        }
    }

    /// The format a file with this extension has, in either case and without the dot:
    /// `"png"`, `"jpg"` and `"jpeg"`, `"gif"`.
    pub fn from_extension(extension: &str) -> Option<ImageFormat> {
        match extension.to_ascii_lowercase().as_str() {
            "png" => Some(ImageFormat::Png),
            "jpg" | "jpeg" => Some(ImageFormat::Jpeg),
            "gif" => Some(ImageFormat::Gif),
            _ => None,
        }
    }

    /// The format by the first bytes of a file: `89 50 4E 47` is a PNG, `FF D8 FF` a
    /// JPEG, `GIF8` a GIF.
    pub fn sniff(bytes: &[u8]) -> Option<ImageFormat> {
        if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
            Some(ImageFormat::Png)
        } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            Some(ImageFormat::Jpeg)
        } else if bytes.starts_with(b"GIF8") {
            Some(ImageFormat::Gif)
        } else {
            None
        }
    }

    /// The format a value of the `format` column names.
    pub(crate) fn from_column(name: &str) -> Option<ImageFormat> {
        match name {
            "png" => Some(ImageFormat::Png),
            "jpeg" => Some(ImageFormat::Jpeg),
            "gif" => Some(ImageFormat::Gif),
            _ => None,
        }
    }
}

/// An image to store: its bytes and what whoever read them found out about them. The
/// store hashes the bytes; it does not decode them and takes the rest as it is given.
#[derive(Clone, Copy, Debug)]
pub struct NewImage<'a> {
    /// The file, whole.
    pub bytes: &'a [u8],
    /// What kind of file it is.
    pub format: ImageFormat,
    /// Width in pixels, upright.
    pub width: u32,
    /// Height in pixels, upright.
    pub height: u32,
    /// Pixels per inch the file declares, 72 when it says nothing.
    pub density: u32,
}

/// What the library knows about a stored image without reading its bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageInfo {
    /// The name of the bytes.
    pub id: ImageId,
    /// What kind of file they are.
    pub format: ImageFormat,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Pixels per inch, 72 when the file said nothing.
    pub density: u32,
    /// How many bytes the file has.
    pub byte_len: u64,
    /// When the bytes were first stored, unix ms.
    pub created_at: i64,
}

impl Store {
    /// Stores the bytes once: the id is their hash, and bytes that are there already are
    /// not written again. Either way the image counts as used from now (the collector's
    /// stamp on it is cleared). Committed before it returns. Creates `images.db` when it
    /// is not there.
    ///
    /// Store the bytes before a block that names them enters a document: then a save
    /// never names bytes that are not there.
    pub fn put_image(&mut self, image: &NewImage<'_>) -> Result<ImageInfo> {
        let now = self.now();
        self.images.put_at(image, now)
    }

    /// For an id that should be there (a pasted fragment, `image:<id>` in Markdown):
    /// the image counts as used from now (the collector's stamp on it is cleared).
    /// `None` when the library has no such image. Writes nothing when there is no stamp
    /// to clear, which is the usual case: no transaction, no fsync.
    ///
    /// An image that only the copy in the backups directory has is brought back from
    /// there, as [`Store::image_bytes`] does.
    pub fn touch_image(&mut self, id: &ImageId) -> Result<Option<ImageInfo>> {
        self.images.touch_image(id)
    }

    /// What is known about a stored image; `None` when the library does not have it.
    pub fn image_info(&self, id: &ImageId) -> Result<Option<ImageInfo>> {
        self.images.info(id)
    }

    /// The bytes of a stored image; `None` when the library does not have it. A document
    /// can name an image the library does not have: whoever shows it keeps the block and
    /// says that the picture is missing.
    ///
    /// When `images.db` does not have the image and the copy of the pictures in the
    /// backups directory does ([`back_up_images`]), the bytes are read from the copy and
    /// written back into `images.db` (not by a store that is read-only or in a dry run:
    /// that one only reads them).
    pub fn image_bytes(&self, id: &ImageId) -> Result<Option<Vec<u8>>> {
        self.images.bytes(id)
    }

    /// How many images `images.db` holds; 0 when there is no such file.
    pub fn image_count(&self) -> Result<u64> {
        self.images.count()
    }

    /// Every image id that stored content names: the `content` of every row of
    /// `documents` (live and trashed) and of `snapshots`.
    ///
    /// This is the one thing the store reads in content, and it does not parse it: it
    /// collects every `"image"`, optional white space, `:`, optional white space, `"`, 32
    /// hex digits, `"`. That is the key `innerword-core` writes an image block's id under,
    /// so the key must keep its name. A false hit (a writer's own text holding the token)
    /// only keeps an image alive.
    pub fn referenced_images(&self) -> Result<HashSet<ImageId>> {
        scan::referenced(&self.conn)
    }

    /// Stores the bytes of a file of a page once: the id is their hash ([`FileId::of`]),
    /// and bytes that are there already are not written again. Either way the file counts
    /// as used from now (the collector's stamp on it is cleared). Committed before it
    /// returns. Creates `images.db` when it is not there.
    ///
    /// Store every file of a page before the page that names them is created or saved
    /// ([`Store::create_page`], [`Store::save_page`] refuse a manifest that names a file
    /// the library does not have).
    pub fn put_file(&mut self, bytes: &[u8]) -> Result<FileInfo> {
        let now = self.now();
        self.images.put_file_at(bytes, now)
    }

    /// For a file id that should be there: the file counts as used from now (the
    /// collector's stamp on it is cleared). `None` when the library does not have it.
    /// Writes nothing when there is no stamp to clear. A file that only the copy in the
    /// backups directory has is brought back from there.
    pub fn touch_file(&mut self, id: &FileId) -> Result<Option<FileInfo>> {
        self.images.touch_file(id)
    }

    /// What is known about the stored bytes of a file of a page; `None` when the library
    /// does not have them. Reads no bytes (unless only the copy in the backups directory
    /// has the file, which is then brought back).
    pub fn file_info(&self, id: &FileId) -> Result<Option<FileInfo>> {
        self.images.file_info(id)
    }

    /// The bytes of a file of a page; `None` when the library does not have them. As
    /// [`Store::image_bytes`]: from `images.db`, else from the copy in the backups
    /// directory (and written back, unless this store is read-only or in a dry run); bytes
    /// that are not the ones the id names are not believed.
    pub fn file_bytes(&self, id: &FileId) -> Result<Option<Vec<u8>>> {
        self.images.file_bytes(id)
    }

    /// How many files of pages `images.db` holds; 0 when there is no such file.
    pub fn file_count(&self) -> Result<u64> {
        self.images.file_count()
    }

    /// Every file id that stored content names: the `content` of every row of
    /// `documents` (live and trashed) and of `snapshots`, read as the images are
    /// ([`Store::referenced_images`]): every `"file"`, optional white space, `:`, optional
    /// white space, `"`, 32 hex digits, `"`. That is the key a page's manifest writes the
    /// id of each of its files under, so the key must keep its name.
    pub fn referenced_files(&self) -> Result<HashSet<FileId>> {
        scan::referenced_files(&self.conn)
    }

    /// The files a manifest names that the library does not have, in order; each one it
    /// has counts as used from now ([`Store::touch_file`]), so that no run of the
    /// collector takes it between this look and the save that names it.
    pub(crate) fn files_missing(&mut self, manifest: &str) -> Result<Vec<FileId>> {
        let mut missing = Vec::new();
        for id in scan::files_in(manifest) {
            if self.images.touch_file(&id)?.is_none() {
                missing.push(id);
            }
        }
        Ok(missing)
    }

    /// The files a whiteboard's save names (`files`, as the caller read them from its
    /// scene: the store does not scan a scene for what it checks) that the library does
    /// not have and that are new to the whiteboard, in order, each once. An id the library
    /// has counts as used from now ([`Store::touch_file`]). A missing id that the row of
    /// `board` (`None` for one not made yet) or a version of it names already is not new:
    /// it is kept, so that a whiteboard whose pictures were lost (a `library.db` brought
    /// back without the copy of its pictures) goes on saving.
    pub(crate) fn board_files_missing(
        &mut self,
        board: Option<crate::DocumentId>,
        files: &[FileId],
    ) -> Result<Vec<FileId>> {
        let mut seen = HashSet::new();
        let mut missing = Vec::new();
        for id in files {
            if seen.insert(*id) && self.images.touch_file(id)?.is_none() {
                missing.push(*id);
            }
        }
        let Some(board) = board else {
            return Ok(missing);
        };
        let mut new = Vec::new();
        for id in missing {
            if !self.named_by_item_or_its_versions(board, &id)? {
                new.push(id);
            }
        }
        Ok(new)
    }

    /// Whether the content of the row `item`, or of one of its versions, names the file
    /// `id` (read as the collector reads it, `scan.rs`). Only the contents that hold its
    /// digits are read whole.
    fn named_by_item_or_its_versions(&self, item: crate::DocumentId, id: &FileId) -> Result<bool> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT content FROM documents WHERE id = ?1 AND instr(lower(content), ?2) > 0
             UNION ALL
             SELECT content FROM snapshots WHERE document_id = ?1
                 AND instr(lower(content), ?2) > 0",
        )?;
        let mut rows = stmt.query(rusqlite::params![item, id.to_hex()])?;
        while let Some(row) = rows.next()? {
            let content: String = row.get(0)?;
            if scan::files_in(&content).contains(id) {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The id is the first half of the SHA-256, in lowercase hex. The vectors are the
    /// published digests of the empty input and of "abc".
    #[test]
    fn the_id_is_the_first_16_bytes_of_the_sha_256() {
        assert_eq!(
            ImageId::of(b"").to_hex(),
            "e3b0c44298fc1c149afbf4c8996fb924"
        );
        assert_eq!(
            ImageId::of(b"abc").to_hex(),
            "ba7816bf8f01cfea414140de5dae2223"
        );
        assert_eq!(
            ImageId::of(b"abc").to_string(),
            "ba7816bf8f01cfea414140de5dae2223"
        );
    }

    #[test]
    fn an_id_is_read_from_32_hex_digits_and_nothing_else() {
        let id = ImageId::of(b"abc");
        assert_eq!(ImageId::parse(&id.to_hex()), Some(id));
        assert_eq!(ImageId::parse("BA7816BF8F01CFEA414140DE5DAE2223"), Some(id));
        assert_eq!(ImageId::parse("ba7816bf8f01cfea414140de5dae222"), None);
        assert_eq!(ImageId::parse("ba7816bf8f01cfea414140de5dae22233"), None);
        assert_eq!(ImageId::parse("ga7816bf8f01cfea414140de5dae2223"), None);
        assert_eq!(ImageId::parse(""), None);
        assert_eq!(ImageId::parse("ba7816bf8f01cfea414140de5dae222é"), None);
        assert_eq!(
            ImageId([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 255]).to_hex(),
            "000102030405060708090a0b0c0d0eff"
        );
    }

    #[test]
    fn a_format_has_its_names_and_is_told_by_the_first_bytes() {
        for format in [ImageFormat::Png, ImageFormat::Jpeg, ImageFormat::Gif] {
            assert_eq!(ImageFormat::from_column(format.as_str()), Some(format));
            assert_eq!(
                ImageFormat::from_extension(format.extension()),
                Some(format)
            );
            assert!(format.mime_type().starts_with("image/"));
        }
        assert_eq!(ImageFormat::Jpeg.as_str(), "jpeg");
        assert_eq!(ImageFormat::Jpeg.extension(), "jpg");
        assert_eq!(ImageFormat::Jpeg.mime_type(), "image/jpeg");
        assert_eq!(ImageFormat::from_extension("JPEG"), Some(ImageFormat::Jpeg));
        assert_eq!(ImageFormat::from_extension("Jpg"), Some(ImageFormat::Jpeg));
        assert_eq!(ImageFormat::from_extension(".png"), None);
        assert_eq!(ImageFormat::from_extension("heic"), None);
        assert_eq!(ImageFormat::from_column("jpg"), None);

        assert_eq!(
            ImageFormat::sniff(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]),
            Some(ImageFormat::Png)
        );
        assert_eq!(
            ImageFormat::sniff(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 16]),
            Some(ImageFormat::Jpeg)
        );
        assert_eq!(ImageFormat::sniff(b"GIF89a..."), Some(ImageFormat::Gif));
        assert_eq!(ImageFormat::sniff(b"GIF87a..."), Some(ImageFormat::Gif));
        assert_eq!(ImageFormat::sniff(b"%PDF-1.7"), None);
        assert_eq!(ImageFormat::sniff(b"<svg"), None);
        assert_eq!(ImageFormat::sniff(&[0x89, b'P']), None);
        assert_eq!(ImageFormat::sniff(b""), None);
    }
}
