//! The one place where Markdown is read: everything in this crate that takes Markdown
//! goes through a [`Reader`], which is `innerword_export::from_markdown_with_images` with
//! the options an agent's text wants and with somebody to say what the images of the
//! text are ([`Images`]).

use std::collections::HashMap;

use innerword_core::{Block, Document, DocumentSettings, Image, ImageId};
use innerword_export::{
    ImageResolution, ImageResolver, ImportWarningKind, KnownImages, MarkdownImportOptions,
    from_markdown_with_images, summarize_warnings,
};
use innerword_store::{ImageFormat, ImageInfo, NewImage, Store};

use crate::attach::{self, Destination};
use crate::error::{Error, Result};
use crate::types::AttachedImage;

/// Markdown, read.
pub(crate) struct Imported {
    /// The blocks as a document with the given settings. One empty paragraph when the
    /// text held nothing.
    pub(crate) document: Document,
    /// Whether the text held anything.
    pub(crate) has_content: bool,
    /// What could not be kept as written, one sentence per kind of thing.
    pub(crate) warnings: Vec<String>,
}

impl Imported {
    /// The blocks the text gave; none when it held nothing.
    pub(crate) fn blocks(&self) -> &[Block] {
        if self.has_content {
            self.document.blocks()
        } else {
            &[]
        }
    }
}

/// The options an agent's text is read with. A level-1 heading that opens the text stays
/// in the text (the title of a document follows its first heading); front matter is taken
/// out and reported.
fn options(settings: &DocumentSettings) -> MarkdownImportOptions {
    MarkdownImportOptions {
        settings: settings.clone(),
        ..MarkdownImportOptions::default()
    }
}

/// Reads `markdown` for a comparison with `document`, never for something that is
/// stored: an image line that names an image of the document (`image:<id>`) is that
/// image, and every other image is the link it would have stayed without anybody to ask.
pub(crate) fn read_against(markdown: &str, document: &Document) -> Imported {
    let mut known = KnownImages::of(document);
    let imported = from_markdown_with_images(markdown, &options(document.settings()), &mut known);
    Imported {
        has_content: !imported.is_empty(),
        warnings: Vec::new(),
        document: imported.document,
    }
}

/// What the images of the text an operation is given are, in the order the design of
/// the operations names:
///
/// 1. `image:<id>` of an image the document held before the change is that image, as it
///    was: its width, its alignment, its description. The n-th line that names an id is
///    the n-th image with that id, counted from the blocks the operation replaces
///    ([`Images::know`]). This is what keeps an image that an agent moves, or names
///    again, from coming back at another size.
/// 2. `image:<id>` of any other image the library has is that image at its original
///    size, at the left.
/// 3. A file the request brought ([`AttachedImage`]) is stored and is that image, the
///    same way. One that could not be used is reported with its sentence; a path the
///    request brought nothing for was not found. An address on the network stays a link.
///
/// No path is opened here, by design: this runs in the app when the app carries the
/// operation out. In a dry run nothing is stored and the answers are the same.
pub(crate) struct Images<'a> {
    known: KnownImages,
    attached: &'a [AttachedImage],
    dry_run: bool,
    /// What the files that were asked for became, by their source: a file named twice
    /// is stored once.
    answered: HashMap<String, ImageResolution>,
    /// The first failure of the library while an image was stored. The importer cannot
    /// be told; the reader says it afterwards.
    failure: Option<Error>,
}

impl<'a> Images<'a> {
    /// For an operation whose request brought `attached`. Fails when they weigh more
    /// together than one request may bring.
    pub(crate) fn new(attached: &'a [AttachedImage], dry_run: bool) -> Result<Images<'a>> {
        attach::weigh(attached)?;
        Ok(Images {
            known: KnownImages::new(),
            attached,
            dry_run,
            answered: HashMap::new(),
            failure: None,
        })
    }

    /// Says which images the document held before the change: those among `replaced`
    /// (the blocks the text takes the place of) first, then those of `before` and of
    /// `after` (the rest of the document on either side). Every use is counted from the
    /// first again.
    pub(crate) fn know(&mut self, replaced: &[Block], before: &[Block], after: &[Block]) {
        let mut known = KnownImages::of_blocks(replaced);
        known.add_blocks(before);
        known.add_blocks(after);
        self.known = known;
    }

    fn resolve(&mut self, store: &mut Store, source: &str) -> ImageResolution {
        if let Some(known) = self.known.known(source) {
            return known;
        }
        let stored = |image: Image| ImageResolution::Stored {
            image,
            width: None,
            alignment: None,
        };
        match attach::destination(source) {
            Destination::Reference(id) => {
                let name = innerword_store::ImageId(*id.as_bytes());
                // Naming an image is using it: the collector's stamp on it goes.
                let found = if self.dry_run {
                    store.image_info(&name)
                } else {
                    store.touch_image(&name)
                };
                match found {
                    Ok(Some(info)) => stored(image_of(&info)),
                    Ok(None) => ImageResolution::NotFound,
                    Err(error) => {
                        self.failure.get_or_insert(error.into());
                        ImageResolution::NotFound
                    }
                }
            }
            Destination::BadReference => ImageResolution::NotFound,
            Destination::Network => ImageResolution::Remote,
            Destination::Local => {
                if let Some(answer) = self.answered.get(source) {
                    return answer.clone();
                }
                let attached = self.attached;
                let answer = match attached.iter().find(|image| image.source == source) {
                    None => ImageResolution::NotFound,
                    Some(AttachedImage {
                        problem: Some(problem),
                        ..
                    }) => ImageResolution::Unsupported(problem.clone()),
                    Some(image) => match self.store(store, image) {
                        Ok(Some(image)) => stored(image),
                        Ok(None) => ImageResolution::Unsupported(format!(
                            "{} did not arrive as an image the library can hold.",
                            attach::subject(source)
                        )),
                        Err(error) => {
                            self.failure.get_or_insert(error);
                            ImageResolution::NotFound
                        }
                    },
                };
                self.answered.insert(source.to_string(), answer.clone());
                answer
            }
        }
    }

    /// Stores what a request brought and says which image it is now. `None` for bytes
    /// that are not what they are said to be. In a dry run nothing is stored.
    pub(crate) fn store(
        &mut self,
        store: &mut Store,
        attached: &AttachedImage,
    ) -> Result<Option<Image>> {
        let format = ImageFormat::from_extension(&attached.format)
            .filter(|format| ImageFormat::sniff(&attached.data) == Some(*format));
        let Some(format) = format else {
            return Ok(None);
        };
        let (width, height) = attached.pixels;
        if width == 0 || height == 0 || attached.data.len() > innerword_image::MAX_STORED_BYTES {
            return Ok(None);
        }
        let new = NewImage {
            bytes: &attached.data,
            format,
            width,
            height,
            density: attached.density,
        };
        let info = if self.dry_run {
            let id = innerword_store::ImageId::of(&attached.data);
            // Bytes the library has already are the image it has.
            store.image_info(&id)?.unwrap_or(ImageInfo {
                id,
                format,
                width,
                height,
                density: attached.density,
                byte_len: attached.data.len() as u64,
                created_at: 0,
            })
        } else {
            // Committed before a block that names the bytes enters a document.
            store.put_image(&new)?
        };
        Ok(Some(image_of(&info)))
    }
}

/// A stored image as a document names it: as wide as the column and without a
/// description, until whoever places it says otherwise.
pub(crate) fn image_of(info: &ImageInfo) -> Image {
    Image::new(
        ImageId::from_bytes(info.id.0),
        (info.width, info.height),
        u16::try_from(info.density).unwrap_or(Image::MAX_DENSITY),
    )
}

/// Reads Markdown for one operation: with the library the images go to, and the answers
/// about them.
pub(crate) struct Reader<'r, 'a> {
    /// The library of the operation: the tool's own connection, the app's, or one that
    /// only reads in a dry run.
    pub(crate) store: &'r mut Store,
    /// What the images of the text are.
    pub(crate) images: &'r mut Images<'a>,
}

impl ImageResolver for Reader<'_, '_> {
    fn resolve(&mut self, source: &str) -> ImageResolution {
        self.images.resolve(self.store, source)
    }
}

impl Reader<'_, '_> {
    /// Reads `markdown` into blocks of a document with `settings`. Fails only when the
    /// library could not store an image the text brought.
    pub(crate) fn read(&mut self, markdown: &str, settings: &DocumentSettings) -> Result<Imported> {
        let imported = from_markdown_with_images(markdown, &options(settings), self);
        if let Some(error) = self.images.failure.take() {
            return Err(error);
        }
        // A file that could not be used is named, with the reason; everything else is
        // said once per kind of thing.
        let (files, general): (Vec<_>, Vec<_>) =
            imported.warnings.iter().cloned().partition(|warning| {
                matches!(
                    warning.kind,
                    ImportWarningKind::ImageNotFound | ImportWarningKind::ImageUnsupported
                )
            });
        let mut warnings = summarize_warnings(&general);
        if files.len() != imported.image_problems.len() {
            // Should the importer ever warn of a file without naming it.
            warnings.extend(summarize_warnings(&files));
        } else {
            for problem in &imported.image_problems {
                let sentence = match &problem.sentence {
                    Some(sentence) => format!("{sentence} It was kept as a link."),
                    None => not_found(&problem.source),
                };
                if !warnings.contains(&sentence) {
                    warnings.push(sentence);
                }
            }
        }
        Ok(Imported {
            has_content: !imported.is_empty(),
            warnings,
            document: imported.document,
        })
    }
}

/// What is said of an image the text names and nobody has.
fn not_found(source: &str) -> String {
    let subject = attach::subject(source);
    match attach::destination(source) {
        Destination::Reference(_) | Destination::BadReference => {
            format!("{subject} names an image the library does not have. It was kept as a link.")
        }
        _ => format!("{subject} was not found. It was kept as a link."),
    }
}
