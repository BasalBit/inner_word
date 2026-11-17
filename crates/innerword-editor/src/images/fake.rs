//! [`FakeImages`]: an [`ImageProvider`] that keeps its pictures in memory,
//! for the tests of every crate that shows an editor with images (as
//! `innerword_spell::FakeSpellService` is for spelling). No library, no
//! panel, no pasteboard: a test says beforehand what each of them answers.

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, hash_map::DefaultHasher},
    future::Future,
    hash::{Hash, Hasher},
    io::Cursor,
    path::PathBuf,
    pin::Pin,
    rc::Rc,
    sync::Arc,
    task::{Context as TaskContext, Poll, Waker},
};

use gpui::{App, Task, Window};
use innerword_core::{Image, ImageId};

use super::{ImageProvider, NewImageSource};

/// A provider for tests: pictures in memory, and answers a test sets up
/// beforehand.
///
/// ```ignore
/// let images = FakeImages::new();
/// let photo = images.add_png(1600, 1200);            // a flat colour, 72 pixels to the inch
/// editor.update(cx, |editor, cx| editor.set_image_provider(Some(images.clone()), cx));
/// editor.update(cx, |editor, cx| editor.edit(|state| state.insert_image(photo, Alignment::Center), cx));
/// ```
pub struct FakeImages {
    images: RefCell<HashMap<ImageId, Arc<[u8]>>>,
    /// Closed while `bytes` is held back.
    gate: Rc<Gate>,
    /// What the "panel" answers; `None` is a cancelled panel.
    chosen: RefCell<Option<Vec<PathBuf>>>,
    refused: RefCell<Vec<PathBuf>>,
    pasteboard: RefCell<Option<NewImageSource>>,
    asked: RefCell<Vec<ImageId>>,
    touched: RefCell<Vec<ImageId>>,
}

#[derive(Default)]
struct Gate {
    closed: Cell<bool>,
    waiting: RefCell<Vec<Waker>>,
}

/// Resolves when the gate is open.
struct Open(Rc<Gate>);

impl Future for Open {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<()> {
        if self.0.closed.get() {
            self.0.waiting.borrow_mut().push(cx.waker().clone());
            Poll::Pending
        } else {
            Poll::Ready(())
        }
    }
}

/// An id for bytes: the same bytes, the same id. (Not the library's id,
/// which is a SHA-256; a test never compares the two.)
fn id_of(bytes: &[u8]) -> ImageId {
    let half = |salt: u8| {
        let mut hasher = DefaultHasher::new();
        salt.hash(&mut hasher);
        bytes.hash(&mut hasher);
        hasher.finish().to_be_bytes()
    };
    let mut id = [0u8; 16];
    id[..8].copy_from_slice(&half(1));
    id[8..].copy_from_slice(&half(2));
    ImageId::from_bytes(id)
}

/// The pixel size of PNG, JPEG or GIF bytes, read from their header.
fn pixel_size(bytes: &[u8]) -> Option<(u32, u32)> {
    let known = bytes.starts_with(b"\x89PNG")
        || bytes.starts_with(b"\xFF\xD8\xFF")
        || bytes.starts_with(b"GIF8");
    if !known {
        return None;
    }
    image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()
}

impl FakeImages {
    /// A provider without pictures.
    #[allow(clippy::new_ret_no_self)]
    pub fn new() -> Rc<FakeImages> {
        Rc::new(FakeImages {
            images: RefCell::new(HashMap::new()),
            gate: Rc::new(Gate::default()),
            chosen: RefCell::new(None),
            refused: RefCell::new(Vec::new()),
            pasteboard: RefCell::new(None),
            asked: RefCell::new(Vec::new()),
            touched: RefCell::new(Vec::new()),
        })
    }

    /// The bytes of a PNG of that size in one flat colour (the colour
    /// follows from the size, so two sizes are two pictures).
    pub fn png(width: u32, height: u32) -> Vec<u8> {
        let (width, height) = (width.max(1), height.max(1));
        let colour = image::Rgba([
            (width % 199) as u8 + 40,
            (height % 173) as u8 + 60,
            ((width + height) % 151) as u8 + 80,
            255,
        ]);
        let picture = image::RgbaImage::from_pixel(width, height, colour);
        let mut bytes = Vec::new();
        picture
            .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
            .expect("a PNG can be written to memory");
        bytes
    }

    /// Adds a flat-colour PNG of that size and returns the image: its id
    /// (a hash of the bytes), its pixel size, 72 pixels to the inch, the
    /// whole column wide, no description.
    pub fn add_png(&self, width: u32, height: u32) -> Image {
        self.add_bytes(Self::png(width, height))
            .expect("a PNG that was just written is a picture")
    }

    /// Adds stored bytes as they are (PNG, JPEG or GIF) and returns the
    /// image. `None` for anything else. Only the header is read: bytes
    /// that begin like a picture and do not decode are added too (the
    /// editor then says that the image cannot be shown).
    pub fn add_bytes(&self, bytes: Vec<u8>) -> Option<Image> {
        let pixels = pixel_size(&bytes)?;
        let id = id_of(&bytes);
        self.images.borrow_mut().insert(id, bytes.into());
        Some(Image::new(id, pixels, Image::DEFAULT_DENSITY))
    }

    /// Takes a picture away again: its image is then missing.
    pub fn remove(&self, image: ImageId) {
        self.images.borrow_mut().remove(&image);
    }

    /// From now on `bytes` does not answer until [`FakeImages::release`]:
    /// a picture that is still on its way.
    pub fn hold(&self) {
        self.gate.closed.set(true);
    }

    /// Lets every held answer of `bytes` through (they arrive when the
    /// executor runs next).
    pub fn release(&self) {
        self.gate.closed.set(false);
        for waker in self.gate.waiting.borrow_mut().drain(..) {
            waker.wake();
        }
    }

    /// What the "panel" of [`ImageProvider::choose`] answers next (once;
    /// without it the panel was cancelled).
    pub fn choose_beforehand(&self, paths: Vec<PathBuf>) {
        *self.chosen.borrow_mut() = Some(paths);
    }

    /// `import` of this file answers `None`, as for a file that is not a
    /// picture.
    pub fn refuse(&self, path: impl Into<PathBuf>) {
        self.refused.borrow_mut().push(path.into());
    }

    /// What [`ImageProvider::pasteboard`] answers: a picture on the system
    /// pasteboard that GPUI's clipboard does not show.
    pub fn set_pasteboard(&self, source: Option<NewImageSource>) {
        *self.pasteboard.borrow_mut() = source;
    }

    /// Every call of `bytes`, in order.
    pub fn asked(&self) -> Vec<ImageId> {
        self.asked.borrow().clone()
    }

    /// Every image `touch` was called with, in order.
    pub fn touched(&self) -> Vec<ImageId> {
        self.touched.borrow().clone()
    }
}

impl ImageProvider for FakeImages {
    fn bytes(&self, image: ImageId, cx: &mut App) -> Task<Option<Arc<[u8]>>> {
        self.asked.borrow_mut().push(image);
        let bytes = self.images.borrow().get(&image).cloned();
        let open = Open(self.gate.clone());
        cx.spawn(async move |_| {
            open.await;
            bytes
        })
    }

    /// `File(path)`: the file's bytes as they are (no conversion); `None`
    /// for a file that was refused, cannot be read or is no PNG, JPEG or
    /// GIF.
    fn import(&self, source: NewImageSource, _: &mut App) -> Task<Option<Image>> {
        let bytes = match source {
            NewImageSource::File(path) => {
                if self.refused.borrow().contains(&path) {
                    return Task::ready(None);
                }
                std::fs::read(&path).ok()
            }
            NewImageSource::Bytes(bytes) => Some(bytes),
        };
        Task::ready(bytes.and_then(|bytes| self.add_bytes(bytes)))
    }

    fn choose(&self, _: &mut Window, _: &mut App) -> Task<Option<Vec<PathBuf>>> {
        Task::ready(self.chosen.borrow_mut().take())
    }

    fn pasteboard(&self, _: &mut App) -> Option<NewImageSource> {
        self.pasteboard.borrow().clone()
    }

    fn touch(&self, images: &[ImageId], _: &mut App) {
        self.touched.borrow_mut().extend_from_slice(images);
    }
}
