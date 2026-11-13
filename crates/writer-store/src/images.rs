//! The bytes of the images that documents hold.
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
//!   pragmas. A [`Store`] holds one and opens it at the first use.
//! - `scan.rs`: which images stored content names. The one thing the store reads in a
//!   document's content.
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

use sha2::{Digest, Sha256};

use crate::Store;
use crate::error::Result;

pub use collect::{IMAGE_RETENTION_MS, ImagesCollected, collect_unused_images, mark_unused_images};
pub use copy::{IMAGE_BACKUP_SLICE_BYTES, ImagesBackedUp, back_up_all_images, back_up_images};
pub use file::ImageFile;

/// The name of stored image bytes: the first 16 bytes of their SHA-256. Written as 32
/// lowercase hex digits wherever it is text (the column, a document's JSON, Markdown).
///
/// `writer-core` has a type of the same name and the same 16 bytes for the id inside a
/// document; this crate does not depend on it, and the two meet in the hex digits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ImageId(pub [u8; 16]);

impl ImageId {
    /// The id of these bytes.
    pub fn of(bytes: &[u8]) -> ImageId {
        let digest = Sha256::digest(bytes);
        let mut id = [0u8; 16];
        id.copy_from_slice(&digest[..16]);
        ImageId(id)
    }

    /// Reads exactly 32 hex digits, in either case. Anything else is `None`.
    pub fn parse(text: &str) -> Option<ImageId> {
        Self::from_hex(text.as_bytes())
    }

    /// The id as 32 lowercase hex digits. [`fmt::Display`] writes the same.
    pub fn to_hex(&self) -> String {
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
        let mut hex = String::with_capacity(32);
        for byte in self.0 {
            hex.push(DIGITS[usize::from(byte >> 4)] as char);
            hex.push(DIGITS[usize::from(byte & 0x0f)] as char);
        }
        hex
    }

    /// 32 hex digits as bytes of text. The scan of stored content reads ids with it.
    pub(crate) fn from_hex(digits: &[u8]) -> Option<ImageId> {
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
        Some(ImageId(id))
    }
}

impl fmt::Display for ImageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

/// What kind of file stored image bytes are. The library stores these three and nothing
/// else; `writer-image` turns every other kind into one of them on the way in.
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
    /// hex digits, `"`. That is the key `writer-core` writes an image block's id under,
    /// so the key must keep its name. A false hit (a writer's own text holding the token)
    /// only keeps an image alive.
    pub fn referenced_images(&self) -> Result<HashSet<ImageId>> {
        scan::referenced(&self.conn)
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
