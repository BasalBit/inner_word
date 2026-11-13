//! The ways a picture comes into a document and goes out of it: chosen
//! files, a paste, a drop, copy and cut.
//!
//! # Where a picture lands
//!
//! One rule for every way in (it is writer-core's,
//! `EditorState::insert_image`): the caret's block is an empty paragraph,
//! then that paragraph becomes the image; otherwise the image goes after
//! the last block the selection touches. A drop goes to the boundary
//! between blocks nearest to the pointer. The caret ends on the new image:
//! it is selected. A new picture is centred at its original size on the
//! page (`EditorState::original_image_width`); one that was copied here
//! comes back as it was.
//!
//! # Pictures take a moment
//!
//! Storing a picture is the provider's and is asynchronous (a large file
//! is prepared in the background). The place is therefore remembered as a
//! block id and looked up again when the picture is there: the document
//! may have changed meanwhile. When that block is gone, the picture goes
//! where the caret is. When another document was opened meanwhile, it
//! goes nowhere.
//!
//! # The clipboard
//!
//! A clipboard item is either text or one picture, never both (GPUI on
//! macOS writes one or the other). A selection with text is copied as it
//! always was: its plain text for other apps, the fragment beside it for
//! pasting back with formatting. **An image by itself is copied as the
//! picture**, so it can be pasted into other apps; the app remembers which
//! picture it put there and the block that was copied
//! ([`CopiedPicture`]), and a paste of that same picture puts the block
//! back with its width, its alignment and its description.
//!
//! A paste looks, in this order, for: a fragment (what was copied here);
//! the picture this app copied last; a picture GPUI's clipboard shows; a
//! picture only the provider can see on the pasteboard (a file copied in
//! Finder, "Copy Image" in a browser: GPUI hands over their text only);
//! text. "Paste as plain text" and compact editors never take pictures.

use std::path::PathBuf;

use gpui::{
    App, Bounds, ClipboardEntry, ClipboardItem, Context, Global, ImageFormat, Pixels, Point,
    Window, point, px, size,
};
use writer_core::{Alignment, BlockId, EditorState, Fragment, Image, ImageId};

use super::NewImageSource;
use crate::{editor::Editor, layout::Lay, paint::snap};

/// Thickness of the line that shows where a dragged picture or a dropped
/// file would go, in window pixels.
const DROP_LINE: Pixels = px(2.);
/// Where the drop line stands when it has no two blocks to stand between
/// (before the first block, after the last one, at a sheet break): this
/// far from the block's edge, in layout units.
const DROP_LINE_GAP: f32 = 4.;

/// The last picture this app put on the clipboard: which one
/// (`gpui::Image::id`, a hash of its bytes) and the block it was copied
/// from, so that pasting that picture here brings the block back as it
/// was. An app global: a picture copied in one editor is pasted whole
/// into another.
pub(crate) struct CopiedPicture {
    id: u64,
    fragment: Fragment,
}

impl Global for CopiedPicture {}

/// Where pictures that are on their way go once they are there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Place {
    /// Where an insertion goes for a selection that ended in this block
    /// (`None`: for the selection as it is then).
    Selection(Option<BlockId>),
    /// Before this block; `None`: after the last block.
    Before(Option<BlockId>),
}

/// A picture to put into the document.
pub(crate) struct Incoming {
    pub(crate) image: Image,
    pub(crate) alignment: Alignment,
    /// It was in a document before (a paste of what was copied): its
    /// width stays. A new picture gets its original size.
    pub(crate) as_it_was: bool,
}

/// PNG, JPEG or GIF, by the first bytes.
fn format_of(bytes: &[u8]) -> Option<ImageFormat> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(ImageFormat::Png)
    } else if bytes.starts_with(b"\xFF\xD8\xFF") {
        Some(ImageFormat::Jpeg)
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some(ImageFormat::Gif)
    } else {
        None
    }
}

/// The one image of a fragment that is nothing but an image.
fn lone_image(fragment: &Fragment) -> Option<(Image, Alignment)> {
    match fragment.blocks() {
        [block] => block
            .as_image()
            .map(|image| (image.clone(), block.alignment())),
        _ => None,
    }
}

/// An image on the clipboard when its bytes cannot be had: the block
/// beside the words that describe it. (Never an empty text: what a real
/// pasteboard makes of one was not tried.)
fn write_described(image: &Image, fragment: &Fragment, cx: &mut App) {
    let text = if image.description.is_empty() {
        "Image".to_string()
    } else {
        image.description.clone()
    };
    cx.write_to_clipboard(ClipboardItem::new_string_with_metadata(
        text,
        fragment.to_json(),
    ));
}

/// The line that shows boundary `boundary` (the place before the block
/// with that index; the block count: after the last block), in window
/// coordinates: as wide as the column, between the two blocks. A boundary
/// that falls on a sheet break is shown under the upper block.
pub(crate) fn drop_line(lay: &mut Lay, boundary: usize) -> Option<Bounds<Pixels>> {
    let count = lay.count();
    let above = boundary
        .checked_sub(1)
        .and_then(|index| lay.placed(index))
        .map(|placed| placed.bottom());
    let below = (boundary < count)
        .then(|| lay.placed(boundary))
        .flatten()
        .map(|placed| placed.top);
    let y = match (above, below) {
        (Some(above), Some(below))
            if below >= above && lay.page_at(above) == lay.page_at(below) =>
        {
            (above + below) / 2.
        }
        (Some(above), _) => above + px(DROP_LINE_GAP),
        (None, Some(below)) => below - px(DROP_LINE_GAP),
        (None, None) => return None,
    };
    let view = lay.view();
    let width = view.len(lay.env().column_width);
    Some(Bounds::new(
        point(view.x(px(0.)), snap(view.y(y) - DROP_LINE / 2.)),
        size(width, DROP_LINE),
    ))
}

impl Editor {
    /// What "Insert > Image…" does once files are chosen: imports each
    /// through the provider and inserts them in order, one undo step, the
    /// last one selected. A file the provider refuses is left out (the
    /// provider has told the user). Does nothing in an editor that cannot
    /// be changed, holds plain text or has no provider.
    pub fn insert_image_files(
        &mut self,
        paths: Vec<PathBuf>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let place = Place::Selection(self.selection_block());
        let sources = paths.into_iter().map(NewImageSource::File).collect();
        self.import_pictures(sources, place, cx);
    }

    /// What a drop of files at a window point does: the same as
    /// [`Editor::insert_image_files`], at the boundary between blocks
    /// nearest to the point. The editor takes the keyboard.
    pub fn drop_files(
        &mut self,
        paths: &[PathBuf],
        at: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_file_drop(None, cx);
        if !self.takes_pictures() || paths.is_empty() {
            return;
        }
        window.focus(&self.focus_handle);
        let place = match self.boundary_at(at) {
            Some(boundary) => Place::Before(
                self.state
                    .document()
                    .block(boundary)
                    .map(|block| block.id()),
            ),
            None => Place::Selection(self.selection_block()),
        };
        let sources = paths.iter().cloned().map(NewImageSource::File).collect();
        self.import_pictures(sources, place, cx);
    }

    /// Whether Paste would put something in: GPUI's clipboard has an
    /// item, or the provider's `pasteboard()` has a picture GPUI does not
    /// show (a HEIC, a file copied in Finder) and this editor takes
    /// pictures. What "Paste" in a menu asks before it is enabled.
    pub fn can_paste(&self, cx: &mut App) -> bool {
        if cx.read_from_clipboard().is_some() {
            return true;
        }
        self.takes_pictures()
            && self
                .images
                .provider
                .clone()
                .is_some_and(|provider| provider.pasteboard(cx).is_some())
    }

    /// The last block the selection touches: where an insertion goes
    /// after.
    fn selection_block(&self) -> Option<BlockId> {
        let touched = self.state.touched_blocks();
        self.state
            .document()
            .block(touched.end.saturating_sub(1))
            .map(|block| block.id())
    }

    /// The boundary between two blocks nearest to a window point: the
    /// index of the block after it (the block count: after the last
    /// block). The upper half of a block's box is the boundary before it,
    /// the lower half and the space under it the one after it.
    pub(crate) fn boundary_at(&self, at: Point<Pixels>) -> Option<usize> {
        self.with_layout(|lay| {
            let index = {
                let y = lay.view().layout_y(at.y);
                lay.block_at(y)
            };
            let placed = lay.placed(index)?;
            // Shaping the block may have moved it: ask again.
            let y = lay.view().layout_y(at.y);
            let middle = (placed.top + placed.bottom()) / 2.;
            Some(if y < middle { index } else { index + 1 })
        })
        .flatten()
    }

    /// Files are dragged over the window: the drop line follows the
    /// pointer while it is over an editor that takes pictures.
    pub(crate) fn file_drag_moved(&mut self, at: Point<Pixels>, cx: &mut Context<Self>) {
        let inside = self
            .viewport_bounds()
            .is_some_and(|viewport| viewport.contains(&at));
        let boundary = (inside && self.takes_pictures())
            .then(|| self.boundary_at(at))
            .flatten();
        self.set_file_drop(boundary, cx);
    }

    /// Sets where the files that are dragged over the editor would go
    /// (`None`: nowhere, the line goes away).
    pub(crate) fn set_file_drop(&mut self, boundary: Option<usize>, cx: &mut Context<Self>) {
        if self.file_drop != boundary {
            self.file_drop = boundary;
            cx.notify();
        }
    }

    /// Stores `sources` through the provider, one after the other, and
    /// puts the pictures that came of them at `place`.
    pub(crate) fn import_pictures(
        &mut self,
        sources: Vec<NewImageSource>,
        place: Place,
        cx: &mut Context<Self>,
    ) {
        if sources.is_empty() || !self.takes_pictures() {
            return;
        }
        let Some(provider) = self.images.provider.clone() else {
            return;
        };
        let serial = self.document_serial;
        cx.spawn(async move |this, cx| {
            let mut images = Vec::new();
            for source in sources {
                // One at a time and in order: several large files are not
                // prepared side by side.
                let Ok(import) = cx.update(|cx| provider.import(source, cx)) else {
                    return;
                };
                if let Some(image) = import.await {
                    images.push(Incoming {
                        image,
                        alignment: Alignment::Center,
                        as_it_was: false,
                    });
                }
            }
            this.update(cx, |editor, cx| {
                // Another document is open now: these pictures were not
                // meant for it.
                if editor.document_serial == serial {
                    editor.place_pictures(place, images, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Puts pictures into the document at `place`, in order, as one undo
    /// step; the caret ends on the last one. Returns whether anything
    /// changed.
    pub(crate) fn place_pictures(
        &mut self,
        place: Place,
        incoming: Vec<Incoming>,
        cx: &mut Context<Self>,
    ) -> bool {
        if incoming.is_empty() || self.config.mode.is_compact() {
            return false;
        }
        enum Target {
            Selection(Option<usize>),
            Before(usize),
        }
        let document = self.state.document();
        let index_of = |id: BlockId| document.blocks().iter().position(|block| block.id() == id);
        let target = match place {
            Place::Selection(block) => Target::Selection(block.and_then(index_of)),
            Place::Before(None) => Target::Before(document.block_count()),
            Place::Before(Some(id)) => match index_of(id) {
                Some(index) => Target::Before(index),
                // The block is gone: where the caret is.
                None => Target::Selection(None),
            },
        };
        let sized = |state: &EditorState, incoming: Incoming| {
            let mut image = incoming.image;
            if !incoming.as_it_was {
                image.width = state.original_image_width(&image);
            }
            (image, incoming.alignment)
        };
        let placed = self.user_edit(
            |state| {
                state.grouped(|state| match target {
                    Target::Before(index) => {
                        let images = incoming
                            .into_iter()
                            .map(|incoming| sized(state, incoming))
                            .collect();
                        state.insert_images_at(index, images)
                    }
                    Target::Selection(index) => {
                        if let Some(index) = index
                            && state.touched_blocks().end != index + 1
                        {
                            // The selection moved on while the picture was
                            // on its way: it goes where it was asked for.
                            let end = state.document().block_range(index).end;
                            state.set_caret(end);
                        }
                        let mut any = false;
                        for incoming in incoming {
                            let (image, alignment) = sized(state, incoming);
                            any |= state.insert_image(image, alignment);
                        }
                        any
                    }
                })
            },
            cx,
        );
        self.state.break_undo_group();
        placed == Some(true)
    }

    /// `Copy`: the selection, or the image the caret is on.
    pub(crate) fn copy(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if !self.state.has_something_to_copy() {
            return;
        }
        let fragment = self.state.copy();
        self.write_clipboard(fragment, cx);
    }

    /// `Cut`: the selection, or the image the caret is on.
    pub(crate) fn cut(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if !self.state.has_something_to_copy() || self.config.read_only {
            return;
        }
        if let Some(fragment) = self.user_edit(|state| state.cut(), cx) {
            self.write_clipboard(fragment, cx);
        }
    }

    /// Writes what was copied to the clipboard: plain text for other apps
    /// with the fragment beside it for pasting back with formatting, or,
    /// for an image by itself, the picture (see the module documentation).
    fn write_clipboard(&mut self, fragment: Fragment, cx: &mut Context<Self>) {
        let Some((image, _)) = lone_image(&fragment).filter(|_| !self.config.mode.is_compact())
        else {
            cx.write_to_clipboard(ClipboardItem::new_string_with_metadata(
                fragment.to_plain_text(),
                fragment.to_json(),
            ));
            return;
        };
        let Some(provider) = self.images.provider.clone() else {
            write_described(&image, &fragment, cx);
            return;
        };
        // The bytes are read off the main thread: the picture is on the
        // clipboard a moment after the key.
        let bytes = provider.bytes(image.id, cx);
        cx.spawn(async move |_, cx| {
            let bytes = bytes.await;
            cx.update(|cx| {
                let picture = bytes.as_deref().and_then(|bytes| {
                    Some(gpui::Image::from_bytes(format_of(bytes)?, bytes.to_vec()))
                });
                match picture {
                    Some(picture) => {
                        cx.set_global(CopiedPicture {
                            id: picture.id(),
                            fragment,
                        });
                        cx.write_to_clipboard(ClipboardItem::new_image(&picture));
                    }
                    None => write_described(&image, &fragment, cx),
                }
            })
            .ok();
        })
        .detach();
    }

    /// `Paste` (see the module documentation for the order).
    pub(crate) fn paste(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let item = cx.read_from_clipboard();
        // Compact editors hold plain text only.
        let rich = !self.config.mode.is_compact();
        let place = Place::Selection(self.selection_block());

        // What was copied here, with its formatting.
        let fragment = item
            .as_ref()
            .and_then(|item| item.metadata())
            .filter(|_| rich)
            .and_then(|json| Fragment::from_json(json).ok());
        if let Some(fragment) = fragment {
            let images: Vec<ImageId> = fragment
                .blocks()
                .iter()
                .filter_map(|block| block.as_image().map(|image| image.id))
                .collect();
            let pasted = match lone_image(&fragment) {
                // An image by itself lands where every picture lands.
                Some((image, alignment)) => self.place_pictures(
                    place,
                    vec![Incoming {
                        image,
                        alignment,
                        as_it_was: true,
                    }],
                    cx,
                ),
                None => self.user_edit(|state| state.paste(&fragment), cx) == Some(true),
            };
            self.touch_images(pasted, &images, cx);
            return;
        }

        if self.takes_pictures() {
            let picture = item.as_ref().and_then(|item| {
                item.entries().iter().find_map(|entry| match entry {
                    ClipboardEntry::Image(picture) => Some(picture),
                    ClipboardEntry::String(_) => None,
                })
            });
            if let Some(picture) = picture {
                // The picture this app copied: its block comes back as it
                // was.
                let copied = cx
                    .try_global::<CopiedPicture>()
                    .filter(|copied| copied.id == picture.id())
                    .and_then(|copied| lone_image(&copied.fragment));
                if let Some((image, alignment)) = copied {
                    let id = image.id;
                    let pasted = self.place_pictures(
                        place,
                        vec![Incoming {
                            image,
                            alignment,
                            as_it_was: true,
                        }],
                        cx,
                    );
                    self.touch_images(pasted, &[id], cx);
                } else {
                    let source = NewImageSource::Bytes(picture.bytes.clone());
                    self.import_pictures(vec![source], place, cx);
                }
                return;
            }
            // A picture only the provider sees.
            let outside = self
                .images
                .provider
                .clone()
                .and_then(|provider| provider.pasteboard(cx));
            if let Some(source) = outside {
                self.import_pictures(vec![source], place, cx);
                return;
            }
        }

        if let Some(text) = item.and_then(|item| item.text()) {
            self.user_edit(|state| state.paste_plain_text(&text), cx);
        }
    }

    /// Tells the provider that images came into the document from
    /// outside.
    fn touch_images(&mut self, pasted: bool, images: &[ImageId], cx: &mut Context<Self>) {
        if pasted
            && !images.is_empty()
            && let Some(provider) = self.images.provider.clone()
        {
            provider.touch(images, cx);
        }
    }
}
