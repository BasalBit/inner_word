//! Images in imported Markdown: who says which stored image a
//! `![description](destination)` is.
//!
//! The importer stores nothing and reads no file. For every image of the
//! Markdown it asks an [`ImageResolver`] what the destination is, and makes
//! an image block only of what the resolver says is stored. Whoever calls
//! the importer decides what a destination may be: the command line tool
//! reads the local files an agent named (before it imports:
//! [`markdown_image_sources`](super::markdown_image_sources) lists them)
//! and stores them; the app, applying an answer of the assistant, knows the
//! images of the library and never reads a path.
//!
//! [`KnownImages`] is the resolver for the commonest question: the
//! Markdown names, as `image:<id>`, an image the document already holds,
//! and it should come back as it was.

use std::collections::HashMap;

use writer_core::{Alignment, Block, Document, Image, ImageId, ImageWidth};

/// What a destination of an image in Markdown turned out to be.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageResolution {
    /// The image, stored: it becomes an image block.
    Stored {
        /// The stored image. Its description is used when the Markdown
        /// gives none; its width is not looked at (see `width`).
        image: Image,
        /// How wide it is drawn. `None`: at its original size on the page
        /// of the imported document
        /// ([`Image::original_width`] with
        /// [`MarkdownImportOptions::settings`](super::MarkdownImportOptions::settings)).
        width: Option<ImageWidth>,
        /// Where it stands on its line when the Markdown does not say (a
        /// `<div align="…">` around the image says). `None`: left.
        alignment: Option<Alignment>,
    },
    /// An address on the network, or one this resolver does not read. The
    /// image is kept as a link.
    Remote,
    /// A local file that is not there or cannot be read. The image is kept
    /// as a link.
    NotFound,
    /// There, but not an image the library can hold; the sentence says why
    /// (it is handed back in [`ImageProblem::sentence`]). The image is kept
    /// as a link.
    Unsupported(String),
}

/// Says what the images of a Markdown text are. See
/// [`from_markdown_with_images`](super::from_markdown_with_images).
pub trait ImageResolver {
    /// What `source` is: the destination as the Markdown has it, without
    /// the angle brackets and backslashes of its syntax. It can be
    /// `image:<id>`, a path, a `file:` or an `https:` address.
    ///
    /// The importer calls this once for every image of the Markdown, in the
    /// Markdown's order, also for an image in a place where no image can
    /// stand (a heading, a quote, a table), which stays a link whatever the
    /// answer. A resolver may count on that: [`KnownImages`] does.
    fn resolve(&mut self, source: &str) -> ImageResolution;
}

/// The resolver of [`from_markdown`](super::from_markdown): every image
/// stays a link.
pub(crate) struct NoImages;

impl ImageResolver for NoImages {
    fn resolve(&mut self, _source: &str) -> ImageResolution {
        ImageResolution::Remote
    }
}

/// An image of the Markdown that was kept as a link because its file could
/// not be used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageProblem {
    /// The destination as the Markdown has it: what the resolver was asked.
    pub source: String,
    /// The line of the imported text the image is on, counted from 1.
    pub line: usize,
    /// Why the file could not be used, in the resolver's words
    /// ([`ImageResolution::Unsupported`]). `None` for a file that was not
    /// found ([`ImageResolution::NotFound`]).
    pub sentence: Option<String>,
}

/// The id a destination of the form `image:<id>` names: what the exporter
/// writes for an image ([`MarkdownImages::References`](crate::MarkdownImages::References)).
/// `None` for every other destination.
///
/// ```
/// use writer_export::image_reference;
///
/// let id = image_reference("image:3f2a9c1b5d6e7f80a1b2c3d4e5f60718").unwrap();
/// assert_eq!(id.to_string(), "3f2a9c1b5d6e7f80a1b2c3d4e5f60718");
/// assert_eq!(image_reference("pictures/harbour.jpg"), None);
/// assert_eq!(image_reference("image:harbour"), None);
/// ```
pub fn image_reference(source: &str) -> Option<ImageId> {
    let source = source.trim();
    let scheme = source.get(..6)?;
    if !scheme.eq_ignore_ascii_case("image:") {
        return None;
    }
    ImageId::parse(&source[6..])
}

/// The uses of one image, in the order they were added, and how many of
/// them were handed out.
#[derive(Clone, Debug, Default)]
struct Uses {
    images: Vec<(Image, Alignment)>,
    next: usize,
}

/// Resolves `image:<id>` to the image a document already holds: its pixel
/// size, its width, its alignment, and its description when the Markdown
/// gives none. Nothing else is resolved.
///
/// It knows every use of an id, in the order they were added: the n-th
/// `image:<id>` it is asked for is the n-th image with that id (the last
/// one again when it is asked more often), so one picture that a document
/// shows twice, at a quarter of the column and at three quarters, comes
/// back as those two.
///
/// ```
/// use writer_core::{Alignment, Block, Document, Image, ImageId, ImageWidth};
/// use writer_export::{
///     ExportSource, KnownImages, MarkdownImportOptions, from_markdown_with_images, to_markdown,
/// };
///
/// let id = ImageId::parse("3f2a9c1b5d6e7f80a1b2c3d4e5f60718").unwrap();
/// let image = Image {
///     width: ImageWidth::from_percent(50),
///     description: "The harbour at dusk".into(),
///     ..Image::new(id, (1600, 1067), 144)
/// };
/// let document = Document::from_blocks(vec![
///     Block::paragraph("Arrival."),
///     Block::image(image.clone()).with_alignment(Alignment::Center),
/// ]);
///
/// // What an agent reads says nothing of the width or the alignment ...
/// let markdown = to_markdown(&ExportSource::from(&document), &Default::default());
/// assert_eq!(
///     markdown,
///     "Arrival.\n\n![The harbour at dusk](image:3f2a9c1b5d6e7f80a1b2c3d4e5f60718)\n"
/// );
///
/// // ... and what it writes back has them all the same.
/// let mut known = KnownImages::of(&document);
/// let imported =
///     from_markdown_with_images(&markdown, &MarkdownImportOptions::default(), &mut known);
/// assert!(imported.warnings.is_empty());
/// let block = &imported.document.blocks()[1];
/// assert_eq!(block.as_image(), Some(&image));
/// assert_eq!(block.alignment(), Alignment::Center);
/// ```
#[derive(Clone, Debug, Default)]
pub struct KnownImages {
    uses: HashMap<ImageId, Uses>,
}

impl KnownImages {
    /// A resolver that knows no image yet.
    pub fn new() -> KnownImages {
        KnownImages::default()
    }

    /// Every image of `document`, in its order.
    pub fn of(document: &Document) -> KnownImages {
        KnownImages::of_blocks(document.blocks())
    }

    /// Every image among `blocks`, in their order.
    pub fn of_blocks(blocks: &[Block]) -> KnownImages {
        let mut known = KnownImages::new();
        known.add_blocks(blocks);
        known
    }

    /// One more use of an image, after the uses of its id that are known
    /// already.
    pub fn add(&mut self, image: Image, alignment: Alignment) {
        self.uses
            .entry(image.id)
            .or_default()
            .images
            .push((image, alignment));
    }

    /// Every image among `blocks`, in their order, after what is known
    /// already.
    pub fn add_blocks(&mut self, blocks: &[Block]) {
        for block in blocks {
            if let Some(image) = block.as_image() {
                self.add(image.clone(), block.alignment());
            }
        }
    }

    /// Hands out the same uses from the first again: for a second import
    /// over the same document.
    pub fn rewind(&mut self) {
        for uses in self.uses.values_mut() {
            uses.next = 0;
        }
    }

    /// Whether no image is known.
    pub fn is_empty(&self) -> bool {
        self.uses.is_empty()
    }

    /// The answer for `image:<id>` of an id that is known:
    /// `Some(Stored { width: Some(..), alignment: Some(..), .. })` with the
    /// next use of that id. `None` for anything else, so that a resolver
    /// can ask here first and somewhere else next.
    pub fn known(&mut self, source: &str) -> Option<ImageResolution> {
        let uses = self.uses.get_mut(&image_reference(source)?)?;
        let last = uses.images.len().checked_sub(1)?;
        let (image, alignment) = &uses.images[uses.next.min(last)];
        uses.next = (uses.next + 1).min(uses.images.len());
        Some(ImageResolution::Stored {
            image: image.clone(),
            width: Some(image.width),
            alignment: Some(*alignment),
        })
    }
}

/// [`KnownImages::known`], and [`ImageResolution::Remote`] for everything
/// it does not know.
impl ImageResolver for KnownImages {
    fn resolve(&mut self, source: &str) -> ImageResolution {
        self.known(source).unwrap_or(ImageResolution::Remote)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(first: &str) -> ImageId {
        ImageId::parse(&format!("{first:0<32}")).expect("hex digits")
    }

    fn image(first: &str, percent: u16, description: &str) -> Image {
        Image {
            width: ImageWidth::from_percent(percent),
            description: description.to_string(),
            ..Image::new(id(first), (800, 600), 72)
        }
    }

    fn reference(first: &str) -> String {
        format!("image:{}", id(first))
    }

    #[test]
    fn references() {
        let harbour = id("3f2a");
        assert_eq!(image_reference(&reference("3f2a")), Some(harbour));
        assert_eq!(
            image_reference("  IMAGE:3F2A0000000000000000000000000000 "),
            Some(harbour)
        );
        for other in [
            "",
            "image:",
            "image:3f2a",
            "image:3f2a00000000000000000000000000000",
            "image:zz2a0000000000000000000000000000",
            "image: 3f2a0000000000000000000000000000",
            "images:3f2a0000000000000000000000000000",
            "3f2a0000000000000000000000000000",
            "file:3f2a0000000000000000000000000000",
            "https://example.com/image:3f2a0000000000000000000000000000",
            "ímage:3f2a0000000000000000000000000000",
        ] {
            assert_eq!(image_reference(other), None, "{other:?}");
        }
    }

    #[test]
    fn the_nth_reference_is_the_nth_use() {
        let mut known = KnownImages::of_blocks(&[
            Block::paragraph("text"),
            Block::image(image("3f2a", 25, "small")),
            Block::image(image("00ff", 100, "other")).with_alignment(Alignment::Right),
            Block::image(image("3f2a", 75, "large")).with_alignment(Alignment::Center),
        ]);
        assert!(!known.is_empty() && KnownImages::new().is_empty());
        let stored = |image: Image, alignment: Alignment| ImageResolution::Stored {
            width: Some(image.width),
            image,
            alignment: Some(alignment),
        };
        let small = stored(image("3f2a", 25, "small"), Alignment::Left);
        let large = stored(image("3f2a", 75, "large"), Alignment::Center);
        let other = stored(image("00ff", 100, "other"), Alignment::Right);
        assert_eq!(known.resolve(&reference("3f2a")), small);
        // Another id in between does not count.
        assert_eq!(known.resolve(&reference("00ff")), other);
        assert_eq!(known.resolve(&reference("3f2a")), large);
        // More references than uses: the last use again.
        assert_eq!(known.resolve(&reference("3f2a")), large);
        assert_eq!(known.resolve(&reference("3f2a")), large);
        assert_eq!(known.resolve(&reference("00ff")), other);
        // From the first again.
        known.rewind();
        assert_eq!(known.resolve(&reference("3f2a")), small);
        assert_eq!(known.resolve(&reference("3f2a")), large);
        // What is added later comes after what was there.
        known.add(image("3f2a", 10, "tiny"), Alignment::Right);
        assert_eq!(
            known.resolve(&reference("3f2a")),
            stored(image("3f2a", 10, "tiny"), Alignment::Right)
        );
        known.rewind();
        known.add_blocks(&[Block::image(image("abcd", 50, "new"))]);
        assert_eq!(known.resolve(&reference("3f2a")), small);
        assert_eq!(
            known.known(&reference("abcd")),
            Some(stored(image("abcd", 50, "new"), Alignment::Left))
        );
    }

    #[test]
    fn what_is_not_known_is_left_to_somebody_else() {
        let mut known = KnownImages::of_blocks(&[Block::image(image("3f2a", 25, "small"))]);
        for source in [
            reference("00ff"),
            "pictures/harbour.jpg".to_string(),
            "/Users/someone/harbour.jpg".to_string(),
            "file:///harbour.jpg".to_string(),
            "https://example.com/harbour.jpg".to_string(),
            String::new(),
        ] {
            assert_eq!(known.known(&source), None, "{source:?}");
            assert_eq!(known.resolve(&source), ImageResolution::Remote);
        }
        // Asking for what is not known uses nothing up.
        assert!(matches!(
            known.known(&reference("3f2a")),
            Some(ImageResolution::Stored { width: Some(w), .. }) if w == ImageWidth::from_percent(25)
        ));
        assert_eq!(
            NoImages.resolve(&reference("3f2a")),
            ImageResolution::Remote
        );
        assert_eq!(
            KnownImages::new().resolve(&reference("3f2a")),
            ImageResolution::Remote
        );
    }
}
