//! Images in the editor: the seam to whoever keeps their bytes, and what
//! the editor does with a picture.
//!
//! An image is a block of its own (`writer_core::BlockKind::Image`), a
//! void block like a divider: no text, one caret position. The block names
//! stored bytes by an id and carries what layout needs without reading
//! them (the pixel size, the width as a share of the text column). **The
//! image is selected when the caret is on its block**: a collapsed
//! selection there. Nothing else in the editor knows a second kind of
//! selection.
//!
//! # The seam
//!
//! The editor does not know the library. Whoever owns it gives the editor
//! an [`ImageProvider`] ([`Editor::set_image_provider`]): it hands out the
//! stored bytes of an image, stores what the user pastes, drops or
//! chooses, and asks for files. Without a provider every image is a
//! placeholder and nothing can be inserted (a compact editor, an answer
//! of the assistant). [`FakeImages`] is the provider for tests.
//!
//! # What is where
//!
//! | Part | Where |
//! |---|---|
//! | The box: size, alignment, the cap by the page | `layout::style` (`LayoutStyle::image_size`), `layout::block` (`ImageBox`) |
//! | Decoding and the cache of decoded pictures | `images::cache` |
//! | Painting: picture, placeholder, outline, handles, the chip, the drop line | `element` |
//! | Resize and move with the pointer, `escape` | `mouse` |
//! | Insert, paste, copy, cut, drop | `images::insert`, `handlers` |
//! | The picture on paper | `print` |
//!
//! # A picture arrives
//!
//! A frame asks the cache for the picture of every image block in the
//! viewport, at the size it draws it. What is not there is asked of the
//! provider after the frame, decoded on a background thread, and the
//! editor is drawn again when it is there. Until then the block shows a
//! placeholder of exactly the picture's size (the size is in the block),
//! so nothing moves when the picture arrives.

mod cache;
mod fake;
mod insert;

use std::{path::PathBuf, rc::Rc, sync::Arc};

use gpui::{App, AppContext, Bounds, Context, Pixels, Task, Window, point, px, size};
use writer_core::{Image, ImageId};

pub(crate) use cache::{Failure, Pictures, decode};
pub use fake::FakeImages;
pub(crate) use insert::drop_line;

use crate::editor::Editor;

/// Side of a corner handle of the selected image, in window pixels at
/// every zoom.
pub(crate) const HANDLE_SIZE: Pixels = px(8.);
/// Thickness of the outline of the selected image, in window pixels.
pub(crate) const OUTLINE_WIDTH: Pixels = px(1.5);

/// What an editor needs from whoever owns the library. Set per editor
/// ([`Editor::set_image_provider`]); without one, every image is a
/// placeholder and nothing can be inserted.
///
/// Every method is called on the main thread. What takes time (reading
/// bytes, preparing a file) is the provider's to do elsewhere: it answers
/// with a task.
pub trait ImageProvider: 'static {
    /// The stored bytes (PNG, JPEG or GIF, upright). `None`: no such image.
    fn bytes(&self, image: ImageId, cx: &mut App) -> Task<Option<Arc<[u8]>>>;

    /// Stores what the user pasted, dropped or chose. `None`: it could not
    /// be used, and the provider has told the user. The image comes back
    /// with id, pixels and density; width and description are the editor's
    /// to set.
    fn import(&self, source: NewImageSource, cx: &mut App) -> Task<Option<Image>>;

    /// "Insert > Image…": asks for files. `None`: cancelled.
    fn choose(&self, window: &mut Window, cx: &mut App) -> Task<Option<Vec<PathBuf>>>;

    /// A picture on the system pasteboard that GPUI's clipboard read does
    /// not show (a file copied in Finder, "Copy Image" in a browser).
    /// Default: none.
    fn pasteboard(&self, _cx: &mut App) -> Option<NewImageSource> {
        None
    }

    /// Image ids that entered the document from outside (a pasted
    /// fragment): they count as used from now. Default: nothing.
    fn touch(&self, _images: &[ImageId], _cx: &mut App) {}
}

/// A picture that is not in the library yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NewImageSource {
    /// A file on disk.
    File(PathBuf),
    /// The bytes of a picture file (from the clipboard).
    Bytes(Vec<u8>),
}

/// What there is to show for an image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageStatus {
    /// The picture is decoded.
    Ready,
    /// Its bytes are on their way, or were not asked for yet (the image
    /// was never on screen).
    Loading,
    /// The provider has no such image.
    Missing,
    /// Its bytes do not decode.
    Broken,
    /// The editor has no provider.
    NoProvider,
}

/// The four handles of a picture with the window bounds `picture`: top
/// left, top right, bottom left, bottom right, each centred on its corner.
pub(crate) fn handles_of(picture: Bounds<Pixels>) -> [Bounds<Pixels>; 4] {
    let handle = |x: Pixels, y: Pixels| {
        Bounds::new(
            point(x - HANDLE_SIZE / 2., y - HANDLE_SIZE / 2.),
            size(HANDLE_SIZE, HANDLE_SIZE),
        )
    };
    [
        handle(picture.left(), picture.top()),
        handle(picture.right(), picture.top()),
        handle(picture.left(), picture.bottom()),
        handle(picture.right(), picture.bottom()),
    ]
}

impl Editor {
    /// Sets who supplies the bytes of images and stores new ones; `None`
    /// takes the provider away. Images that were missing or broken are
    /// asked for again.
    pub fn set_image_provider(
        &mut self,
        provider: Option<Rc<dyn ImageProvider>>,
        cx: &mut Context<Self>,
    ) {
        self.images.set_provider(provider);
        cx.notify();
    }

    /// Whether the editor has an image provider.
    pub fn has_image_provider(&self) -> bool {
        self.images.provider.is_some()
    }

    /// Forget which images were missing or broken and ask again (after an
    /// agent's change, a restored version, an import).
    pub fn images_changed(&mut self, cx: &mut Context<Self>) {
        self.images.forget_failures();
        cx.notify();
    }

    /// The image the caret is on: the selection is collapsed on an image
    /// block. With the index of the block.
    pub fn selected_image(&self) -> Option<(usize, Image)> {
        self.state
            .selected_image()
            .map(|(index, image)| (index, image.clone()))
    }

    /// Window bounds of the picture of block `block`, at the zoom; `None`
    /// when it is not an image or not laid out.
    pub fn image_bounds(&self, block: usize) -> Option<Bounds<Pixels>> {
        self.with_layout(|lay| {
            let placed = lay.placed(block)?;
            let picture = placed.image_rect()?;
            Some(lay.view().rect(picture))
        })
        .flatten()
    }

    /// The four handles of the selected image (top left, top right, bottom
    /// left, bottom right), in window coordinates: where the pointer
    /// resizes it. `None` when no image is selected, and in an editor that
    /// cannot be changed or holds plain text (it draws no handles).
    pub fn image_handle_bounds(&self) -> Option<[Bounds<Pixels>; 4]> {
        if self.config.read_only || self.config.mode.is_compact() {
            return None;
        }
        let (block, _) = self.state.selected_image()?;
        self.image_bounds(block).map(handles_of)
    }

    /// What there is to show for `image`: its picture, a reason why there
    /// is none, or that it is on its way.
    pub fn image_status(&self, image: ImageId) -> ImageStatus {
        self.images.status(image)
    }

    /// Whether pictures can come into this editor: a document editor that
    /// can be changed and has a provider.
    pub(crate) fn takes_pictures(&self) -> bool {
        !self.config.read_only && !self.config.mode.is_compact() && self.images.provider.is_some()
    }

    /// After a frame was planned: starts the loads of the pictures the
    /// frame wanted and did not find. Each asks the provider for the
    /// bytes, decodes them on a background thread at the level the frame
    /// needs and draws the editor again.
    pub(crate) fn load_wanted_images(&mut self, cx: &mut Context<Self>) {
        let wanted = self.images.take_wanted();
        let Some(provider) = self.images.provider.clone() else {
            return;
        };
        for (image, level) in wanted {
            let bytes = provider.bytes(image, cx);
            let task = cx.spawn(async move |this, cx| {
                let outcome = match bytes.await {
                    None => Err(Failure::Missing),
                    Some(bytes) => {
                        cx.background_spawn(
                            async move { decode(&bytes, level).ok_or(Failure::Broken) },
                        )
                        .await
                    }
                };
                this.update(cx, |editor, cx| {
                    if editor.images.finish(image, level, outcome) {
                        cx.notify();
                    }
                })
                .ok();
            });
            self.images.set_loading(image, level, task);
        }
    }
}
