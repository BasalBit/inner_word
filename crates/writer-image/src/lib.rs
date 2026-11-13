//! Image files for The Writer: bytes as they come (a file the writer chooses, the
//! clipboard, a drop, a path an agent names) to bytes as the library stores them.
//!
//! No GPUI and no library in here: the crate reads bytes and gives bytes, with ImageIO
//! and CoreGraphics. The app and the command layer for agents (`writer-agent`) both go
//! through it, so the two make the same thing of the same file.
//!
//! What the library stores is a PNG, a JPEG or a GIF, upright, at most
//! [`MAX_LONG_EDGE`] pixels on its long edge and at most [`MAX_STORED_BYTES`] bytes,
//! and without a GPS position. [`prepare`] holds a file to that:
//!
//! | What comes in | What is stored |
//! |---|---|
//! | more than [`MAX_INPUT_BYTES`] | nothing: [`PrepareError::TooLarge`] |
//! | an SVG, a PDF | nothing: [`PrepareError::Unsupported`] |
//! | bytes ImageIO does not take for an image | nothing: [`PrepareError::NotAnImage`] |
//! | a PNG, JPEG or GIF that is upright, within both limits and carries no position | **the bytes as they came** ([`Prepared::untouched`]): nothing is lost, and the same file twice is stored once |
//! | a JPEG whose only fault is a position | the same file without the position; the picture itself is not encoded again (where ImageIO cannot do that, it is encoded again) |
//! | a PNG whose only fault is a position | the same, or the picture written again as a PNG, which loses nothing of it |
//! | an animated GIF over one of the limits | nothing: it cannot be made smaller without losing its animation |
//! | anything else ImageIO reads (HEIC and HEIF, TIFF, BMP, WebP, AVIF; a JPEG that lies on its side; a picture over one of the limits) | decoded upright and at most 4096 pixels on the long edge, written as a JPEG at quality 0.9, or as a PNG when it has transparency; nothing the file said about itself is copied |
//!
//! A file that is stored as it came keeps everything it says about itself except where
//! it was taken: camera and date, author, copyright, caption.
//!
//! The density ([`Prepared::density`]) is what makes a picture come in at its own size on
//! the page: a screenshot of a Retina display declares 144 pixels per inch and is half
//! its pixels wide in points.
//!
//! [`fixtures`] makes small picture files for tests.

#![warn(missing_docs)]

pub mod fixtures;
mod imageio;

use std::path::Path;

use writer_store::{ImageFormat, NewImage};

use crate::imageio::{Facts, Source, Written, uti};

/// The long edge of a stored image, in pixels, at most. A larger picture is made
/// smaller on the way in.
pub const MAX_LONG_EDGE: u32 = 4096;

/// The bytes of one stored image, at most.
pub const MAX_STORED_BYTES: usize = 16 * 1024 * 1024;

/// The largest file that is read at all. A larger one is refused before it is read.
pub const MAX_INPUT_BYTES: u64 = 100 * 1024 * 1024;

/// The density a picture has when its file says nothing, in pixels per inch.
const DEFAULT_DENSITY: u32 = 72;

/// A density a file declares is believed when it lies in this range.
const DENSITIES: std::ops::RangeInclusive<u32> = 36..=1200;

/// The quality a picture is written at when it becomes a JPEG.
const JPEG_QUALITY: f64 = 0.9;

/// The smallest long edge a picture is brought down to while its file is still over
/// [`MAX_STORED_BYTES`].
const SMALLEST_LONG_EDGE: u32 = 512;

/// An image as the library stores it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prepared {
    /// The file: a PNG, a JPEG or a GIF.
    pub bytes: Vec<u8>,
    /// Which of the three.
    pub format: ImageFormat,
    /// Width and height in pixels, upright.
    pub pixels: (u32, u32),
    /// Pixels per inch, 72 when the file says nothing.
    pub density: u32,
    /// Whether the bytes are the input's own: nothing was converted, scaled, turned or
    /// taken out.
    pub untouched: bool,
}

impl Prepared {
    /// What the library is handed (`Store::put_image`, `ImageFile::put`).
    pub fn as_new_image(&self) -> NewImage<'_> {
        NewImage {
            bytes: &self.bytes,
            format: self.format,
            width: self.pixels.0,
            height: self.pixels.1,
            density: self.density,
        }
    }
}

/// Why bytes did not become a stored image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrepareError {
    /// ImageIO does not take the bytes for an image.
    NotAnImage,
    /// A kind of file that is not stored and not converted: `"SVG"`, `"PDF"`.
    Unsupported(&'static str),
    /// Over [`MAX_INPUT_BYTES`]; or (`animated`) an animated GIF over
    /// [`MAX_STORED_BYTES`] or [`MAX_LONG_EDGE`], which cannot be made smaller without
    /// losing its animation.
    TooLarge {
        /// Whether it is the animated GIF that is too large, and not the file as such.
        animated: bool,
    },
    /// Reading, decoding or writing failed; the text says what, for a log.
    Failed(String),
}

impl PrepareError {
    /// One sentence for the user or an agent. `subject` is what the sentence is about,
    /// as it should read: `“harbour.jpg”`, or `The picture on the clipboard`.
    pub fn sentence(&self, subject: &str) -> String {
        const KINDS: &str = "Images can be PNG, JPEG, GIF, HEIC, TIFF, WebP or BMP.";
        match self {
            PrepareError::NotAnImage => format!("{subject} is not an image."),
            PrepareError::Unsupported("SVG") => format!("{subject} is an SVG file. {KINDS}"),
            PrepareError::Unsupported(kind) => format!("{subject} is a {kind} file. {KINDS}"),
            PrepareError::TooLarge { animated: false } => {
                format!("{subject} is larger than 100 MB.")
            }
            PrepareError::TooLarge { animated: true } => format!(
                "{subject} is an animated GIF over 16 MB or 4096 pixels, more than a document can hold."
            ),
            PrepareError::Failed(_) => format!("{subject} could not be read as an image."),
        }
    }
}

/// Bytes as they come to bytes as the library stores them (the table in the
/// [crate docs](crate)). Runs on any thread, and should not run on the main thread of an
/// app: a large photograph takes a moment to decode.
pub fn prepare(bytes: &[u8]) -> Result<Prepared, PrepareError> {
    if bytes.len() as u64 > MAX_INPUT_BYTES {
        return Err(PrepareError::TooLarge { animated: false });
    }
    if let Some(kind) = unsupported_kind(bytes) {
        return Err(PrepareError::Unsupported(kind));
    }
    let source = Source::new(bytes).ok_or(PrepareError::NotAnImage)?;
    let facts = source.facts().ok_or(PrepareError::NotAnImage)?;

    // One of the three kinds that are stored, by ImageIO's word and by the first bytes.
    let format = stored_format(&facts.kind).filter(|f| ImageFormat::sniff(bytes) == Some(*f));
    let (width, height) = facts.stored_pixels;
    let fits = width.max(height) <= MAX_LONG_EDGE && bytes.len() <= MAX_STORED_BYTES;
    let as_it_came = |bytes: Vec<u8>, untouched: bool| Prepared {
        bytes,
        format: format.unwrap_or(ImageFormat::Png),
        pixels: (width, height),
        density: density_of(&facts),
        untouched,
    };

    if format == Some(ImageFormat::Gif) && facts.count > 1 {
        // An animation is kept whole or not at all.
        return if fits {
            Ok(as_it_came(bytes.to_vec(), true))
        } else {
            Err(PrepareError::TooLarge { animated: true })
        };
    }
    if format.is_some() && fits && facts.orientation == 1 {
        if !facts.has_location {
            return Ok(as_it_came(bytes.to_vec(), true));
        }
        // Only the position is in the way: take it out and leave the picture as it is.
        if let Some(copied) = without_location(&source, &facts, format) {
            return Ok(as_it_came(copied, false));
        }
        // Where that cannot be done for a PNG, it is written again as a PNG: that
        // loses nothing of the picture either.
        if format == Some(ImageFormat::Png) {
            return encode_again(&source, &facts, Some(ImageFormat::Png));
        }
    }
    encode_again(&source, &facts, None)
}

/// Reads the file (refusing one over [`MAX_INPUT_BYTES`] before reading it), then
/// [`prepare`]. A file that cannot be read at all is [`PrepareError::Failed`]; a path
/// that names no plain file (a folder, a device such as `/dev/zero`, a named pipe) is
/// [`PrepareError::NotAnImage`], and is not opened.
///
/// Whatever the file system says about the length, no more than one byte over the
/// limit is ever read: the length of a device or a pipe is 0, and reading one to its
/// end does not end.
pub fn prepare_file(path: &Path) -> Result<Prepared, PrepareError> {
    use std::io::Read;

    let failed = |error: std::io::Error| PrepareError::Failed(error.to_string());
    // Asked before the file is opened: opening a named pipe waits for a writer.
    let metadata = std::fs::metadata(path).map_err(failed)?;
    if !metadata.is_file() {
        return Err(PrepareError::NotAnImage);
    }
    if metadata.len() > MAX_INPUT_BYTES {
        return Err(PrepareError::TooLarge { animated: false });
    }
    let file = std::fs::File::open(path).map_err(failed)?;
    // The file that was opened, in case another one took the name in between.
    if !file.metadata().map_err(failed)?.is_file() {
        return Err(PrepareError::NotAnImage);
    }
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
    file.take(MAX_INPUT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(failed)?;
    if bytes.len() as u64 > MAX_INPUT_BYTES {
        return Err(PrepareError::TooLarge { animated: false });
    }
    prepare(&bytes)
}

/// `"SVG"` or `"PDF"` when the bytes are one of the two kinds of file that are told
/// from the others before ImageIO is asked. An SVG is text that begins with a tag and
/// has `<svg` or `<?xml` in its first kilobyte; a PDF begins with `%PDF`.
fn unsupported_kind(bytes: &[u8]) -> Option<&'static str> {
    let start = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let white = start
        .iter()
        .take_while(|byte| byte.is_ascii_whitespace())
        .count();
    let start = &start[white..];
    if start.starts_with(b"%PDF") {
        return Some("PDF");
    }
    let head = &start[..start.len().min(1024)];
    let holds = |tag: &[u8]| head.windows(tag.len()).any(|window| window == tag);
    (head.starts_with(b"<") && (holds(b"<svg") || holds(b"<?xml"))).then_some("SVG")
}

/// The stored format a kind of file is, when it is one of the three.
fn stored_format(kind: &str) -> Option<ImageFormat> {
    match kind {
        uti::PNG => Some(ImageFormat::Png),
        uti::JPEG => Some(ImageFormat::Jpeg),
        uti::GIF => Some(ImageFormat::Gif),
        _ => None,
    }
}

/// The density a file declares, 72 when it says nothing or something that cannot be.
fn density_of(facts: &Facts) -> u32 {
    facts
        .dpi
        .map(f64::round)
        .filter(|dpi| DENSITIES.contains(&(*dpi as u32)))
        .map_or(DEFAULT_DENSITY, |dpi| dpi as u32)
}

/// The file without its position and with the picture as it is, when ImageIO can do
/// that and the result is what it should be: the same kind of file, the same picture
/// size, no position, within the limit.
fn without_location(
    source: &Source<'_>,
    facts: &Facts,
    format: Option<ImageFormat>,
) -> Option<Vec<u8>> {
    let copied = source.copy_without_location(facts)?;
    let after = Source::new(&copied)?.facts()?;
    let good = !after.has_location
        && after.orientation == 1
        && after.stored_pixels == facts.stored_pixels
        && stored_format(&after.kind) == format
        && ImageFormat::sniff(&copied) == format
        && copied.len() <= MAX_STORED_BYTES;
    good.then_some(copied)
}

/// The picture decoded upright and at most [`MAX_LONG_EDGE`] pixels on its long edge,
/// written as a JPEG, or as a PNG when it has transparency (or when `keep` asks for a
/// PNG, for as long as that fits). Nothing the file said about itself is copied; the
/// density it declared is written again, in proportion when the picture got smaller, so
/// that its size on the page stays what it was.
fn encode_again(
    source: &Source<'_>,
    facts: &Facts,
    keep: Option<ImageFormat>,
) -> Result<Prepared, PrepareError> {
    let (width, height) = facts.upright_pixels();
    let full_edge = width.max(height);
    let mut long_edge = full_edge.min(MAX_LONG_EDGE);
    loop {
        let picture = source
            .decode(long_edge)
            .ok_or_else(|| PrepareError::Failed("the picture could not be decoded".to_owned()))?;
        let pixels = picture.pixels();
        // The file's own word where it gives one; where it gives none, the pixels'.
        let transparent = facts
            .has_alpha
            .unwrap_or_else(|| picture.has_transparency());
        let at_full_size = pixels.0.max(pixels.1) == full_edge;
        let format = if transparent || (keep == Some(ImageFormat::Png) && at_full_size) {
            ImageFormat::Png
        } else {
            ImageFormat::Jpeg
        };
        let scale = f64::from(pixels.0.max(pixels.1)) / f64::from(full_edge);
        let density = ((f64::from(density_of(facts)) * scale).round() as u32)
            .clamp(*DENSITIES.start(), *DENSITIES.end());
        let with = Written {
            quality: (format == ImageFormat::Jpeg).then_some(JPEG_QUALITY),
            dpi: Some(density),
            ..Written::default()
        };
        let kind = match format {
            ImageFormat::Png => uti::PNG,
            _ => uti::JPEG,
        };
        let bytes = imageio::write(&[&picture], kind, &with)
            .ok_or_else(|| PrepareError::Failed("the picture could not be written".to_owned()))?;
        if bytes.len() <= MAX_STORED_BYTES {
            return Ok(Prepared {
                bytes,
                format,
                pixels,
                density,
                untouched: false,
            });
        }
        // Still more than the library stores (a large picture with transparency, as a
        // PNG): half the pixels, and again.
        if long_edge <= SMALLEST_LONG_EDGE {
            return Err(PrepareError::Failed(
                "the picture is over 16 MB at every size tried".to_owned(),
            ));
        }
        long_edge = (long_edge * 5 / 7).max(SMALLEST_LONG_EDGE);
    }
}

#[cfg(test)]
mod tests;
