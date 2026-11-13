//! The decoded pictures of an editor: what a frame paints for an image
//! block, made in the background and kept within a budget.
//!
//! # Levels
//!
//! A picture is not kept at its natural size. A photograph of twelve
//! megapixels is 48 MB of pixels, a column on screen is a thousand or two
//! device pixels wide, and the GPU's atlas samples without mip maps, so a
//! picture that is drawn at less than half its size looks rough. A picture
//! is therefore decoded at a **level**: its natural size halved that many
//! times. A frame asks for the largest level that is still at least as
//! wide as the device pixels it draws the picture on, and never one with a
//! side over [`MAX_SIDE`] (a texture is 16384 pixels a side at most). So a
//! picture has a handful of variants, a step of the zoom usually finds the
//! one it needs, and the GPU never shrinks one to less than half.
//!
//! While the right level is being made, the nearest one that is there is
//! painted: nothing flickers during a pinch.
//!
//! # By image, not by block
//!
//! The cache is keyed by [`ImageId`], the name of the stored bytes. Two
//! blocks with one picture share it, a block that is moved, cut and pasted
//! or replaced by an agent finds it again, and replacing the document
//! (`Editor::set_document`) and coming back costs nothing.
//!
//! # Memory
//!
//! Decoded pixels are bounded at [`BUDGET_BYTES`]; the level that was
//! painted longest ago goes first, and never one the last frame painted. A
//! level that leaves the cache still has a texture in the window's atlas:
//! the frame hands it to `Window::drop_image` in its prepaint
//! ([`Pictures::take_evicted`]), or the texture would stay for the life of
//! the window.

use std::{collections::HashMap, rc::Rc, sync::Arc};

use gpui::{RenderImage, Task};
use writer_core::{Image, ImageId};

use super::{ImageProvider, ImageStatus};

/// How many bytes of decoded pixels an editor keeps.
pub(crate) const BUDGET_BYTES: usize = 128 * 1024 * 1024;
/// The longest side of a decoded picture, in pixels. Half of what a
/// texture can hold, so nothing near the limit is ever handed to the
/// atlas.
pub(crate) const MAX_SIDE: u32 = 8192;

#[cfg(test)]
thread_local! {
    /// How many textures editors that went away handed back to their
    /// window.
    pub(crate) static RELEASED_TEXTURES: std::cell::Cell<usize> =
        const { std::cell::Cell::new(0) };
}

/// Why there is no picture for an image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    /// The provider has no such image.
    Missing,
    /// Its bytes do not decode.
    Broken,
}

/// One decoded size of a picture.
struct Level {
    level: u8,
    picture: Arc<RenderImage>,
    /// Bytes of its pixels.
    bytes: usize,
    /// The frame that painted it last.
    used: u64,
}

#[derive(Default)]
struct Entry {
    levels: Vec<Level>,
    /// The level that is being made, and the task that makes it.
    loading: Option<(u8, Task<()>)>,
    /// Kept until `Editor::images_changed`.
    failure: Option<Failure>,
    /// The frame that asked for the image last.
    seen: u64,
}

/// The decoded pictures of one editor and whoever supplies their bytes.
pub(crate) struct Pictures {
    pub(crate) provider: Option<Rc<dyn ImageProvider>>,
    entries: HashMap<ImageId, Entry>,
    /// Bytes of all decoded levels.
    bytes: usize,
    budget: usize,
    /// Counts the frames that were planned.
    frame: u64,
    /// What the frame that is being planned wanted and did not find.
    wanted: Vec<(ImageId, u8)>,
    /// Pictures that left the cache and still have a texture in the
    /// window's atlas.
    evicted: Vec<Arc<RenderImage>>,
    /// How many pictures were handed back for their textures to be
    /// dropped.
    #[cfg(test)]
    pub(crate) textures_dropped: usize,
}

/// The level a frame wants for a picture of `pixels` that it draws
/// `device_width` device pixels wide: the natural size halved as often as
/// it can be while it stays at least that wide, and at least as often as
/// it takes to bring its longer side to [`MAX_SIDE`].
pub(crate) fn level_for(pixels: (u32, u32), device_width: f32) -> u8 {
    let (width, height) = (pixels.0.max(1), pixels.1.max(1));
    let needed = device_width.max(1.);
    let mut level = 0u8;
    while level < 30 && (width >> (level + 1)) >= 1 && (width >> (level + 1)) as f32 >= needed {
        level += 1;
    }
    while level < 30 && (width.max(height) >> level) > MAX_SIDE {
        level += 1;
    }
    level
}

/// Decodes stored bytes (PNG, JPEG or GIF; of a GIF the first frame) at
/// `level` into what GPUI paints: BGRA with straight alpha. `None` when
/// the bytes are not a picture. Meant for a background thread.
pub(crate) fn decode(bytes: &[u8], level: u8) -> Option<RenderImage> {
    let picture = image::load_from_memory(bytes).ok()?;
    let (width, height) = (picture.width(), picture.height());
    if width == 0 || height == 0 {
        return None;
    }
    // The side limit holds for the picture as it decodes, whatever the
    // document said about its size.
    let mut level = level.min(30);
    while level < 30 && (width.max(height) >> level) > MAX_SIDE {
        level += 1;
    }
    let target = ((width >> level).max(1), (height >> level).max(1));
    let mut pixels = if target == (width, height) {
        picture.into_rgba8()
    } else {
        // Each source pixel goes into exactly one target pixel: an
        // average over whole blocks when the size is halved.
        picture.thumbnail_exact(target.0, target.1).into_rgba8()
    };
    // RGBA to BGRA.
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let frames: smallvec::SmallVec<[image::Frame; 1]> =
        smallvec::smallvec![image::Frame::new(pixels)];
    Some(RenderImage::new(frames))
}

impl Pictures {
    pub(crate) fn new() -> Self {
        Self {
            provider: None,
            entries: HashMap::new(),
            bytes: 0,
            budget: BUDGET_BYTES,
            frame: 0,
            wanted: Vec::new(),
            evicted: Vec::new(),
            #[cfg(test)]
            textures_dropped: 0,
        }
    }

    /// Replaces the provider. What was missing or broken with the old one
    /// is asked for again; what is decoded stays (an image is named by
    /// its bytes, so a picture cannot be another provider's).
    pub(crate) fn set_provider(&mut self, provider: Option<Rc<dyn ImageProvider>>) {
        self.provider = provider;
        for entry in self.entries.values_mut() {
            entry.failure = None;
            entry.loading = None;
        }
    }

    /// Forgets which images were missing or broken: the next frame asks
    /// for them again.
    pub(crate) fn forget_failures(&mut self) {
        for entry in self.entries.values_mut() {
            entry.failure = None;
        }
    }

    /// A frame is about to be planned.
    pub(crate) fn begin_frame(&mut self) {
        self.frame += 1;
        self.wanted.clear();
    }

    /// The picture a frame paints for `image` on `device_width` device
    /// pixels, with the level it is, and what there is to say when there
    /// is none. The level the frame needs is noted as wanted when it is
    /// not there; meanwhile the nearest level that is there is the answer.
    pub(crate) fn picture(
        &mut self,
        image: &Image,
        device_width: f32,
    ) -> (Option<(Arc<RenderImage>, u8)>, ImageStatus) {
        let level = level_for(image.pixels, device_width);
        let frame = self.frame;
        let has_provider = self.provider.is_some();
        let entry = self.entries.entry(image.id).or_default();
        entry.seen = frame;
        let exact = entry.levels.iter().any(|have| have.level == level);
        if !exact
            && entry.failure.is_none()
            && has_provider
            && !self.wanted.contains(&(image.id, level))
        {
            self.wanted.push((image.id, level));
        }
        // The nearest level; of two equally near the finer one.
        let nearest = entry
            .levels
            .iter_mut()
            .min_by_key(|have| (have.level.abs_diff(level), have.level));
        match nearest {
            Some(have) => {
                have.used = frame;
                (Some((have.picture.clone(), have.level)), ImageStatus::Ready)
            }
            None => (None, status_of(Some(entry), has_provider)),
        }
    }

    /// What there is to say about an image.
    pub(crate) fn status(&self, image: ImageId) -> ImageStatus {
        status_of(self.entries.get(&image), self.provider.is_some())
    }

    /// What the last planned frame wanted and is not being made yet. The
    /// loads of images that frame did not ask for are given up: they left
    /// the viewport.
    pub(crate) fn take_wanted(&mut self) -> Vec<(ImageId, u8)> {
        let frame = self.frame;
        for entry in self.entries.values_mut() {
            if entry.seen != frame {
                entry.loading = None;
            }
        }
        let wanted = std::mem::take(&mut self.wanted);
        wanted
            .into_iter()
            .filter(|(image, level)| {
                !matches!(
                    self.entries.get(image).and_then(|entry| entry.loading.as_ref()),
                    Some((loading, _)) if loading == level
                )
            })
            .collect()
    }

    /// `task` is making `level` of `image`. A load of another level of the
    /// same image is given up for it.
    pub(crate) fn set_loading(&mut self, image: ImageId, level: u8, task: Task<()>) {
        self.entries.entry(image).or_default().loading = Some((level, task));
    }

    /// A load ended. Returns whether the image has something new to show.
    pub(crate) fn finish(
        &mut self,
        image: ImageId,
        level: u8,
        outcome: Result<RenderImage, Failure>,
    ) -> bool {
        let frame = self.frame;
        let Some(entry) = self.entries.get_mut(&image) else {
            return false;
        };
        if matches!(&entry.loading, Some((loading, _)) if *loading == level) {
            entry.loading = None;
        }
        match outcome {
            Ok(picture) => {
                entry.failure = None;
                let size = picture.size(0);
                // What was decoded can be coarser than what was asked
                // for (the side limit).
                let bytes = (size.width.0.max(0) as usize) * (size.height.0.max(0) as usize) * 4;
                if let Some(at) = entry.levels.iter().position(|have| have.level == level) {
                    let old = entry.levels.swap_remove(at);
                    self.bytes -= old.bytes;
                    self.evicted.push(old.picture);
                }
                entry.levels.push(Level {
                    level,
                    picture: Arc::new(picture),
                    bytes,
                    // Not the first to go: it was made for the frame that
                    // comes now.
                    used: frame,
                });
                self.bytes += bytes;
                self.evict();
            }
            Err(failure) => entry.failure = Some(failure),
        }
        true
    }

    /// Brings the decoded pixels back under the budget: the level painted
    /// longest ago goes first, and none that the last frame painted.
    fn evict(&mut self) {
        while self.bytes > self.budget {
            let frame = self.frame;
            let oldest = self
                .entries
                .iter()
                .flat_map(|(image, entry)| {
                    entry
                        .levels
                        .iter()
                        .map(move |have| (have.used, *image, have.level))
                })
                .filter(|(used, ..)| *used < frame)
                .min();
            let Some((_, image, level)) = oldest else {
                break;
            };
            let Some(entry) = self.entries.get_mut(&image) else {
                break;
            };
            let Some(at) = entry.levels.iter().position(|have| have.level == level) else {
                break;
            };
            let gone = entry.levels.swap_remove(at);
            self.bytes -= gone.bytes;
            self.evicted.push(gone.picture);
        }
    }

    /// The pictures that left the cache since this was last asked: their
    /// textures are still in the window's atlas, and whoever has the
    /// window drops them.
    pub(crate) fn take_evicted(&mut self) -> Vec<Arc<RenderImage>> {
        let evicted = std::mem::take(&mut self.evicted);
        #[cfg(test)]
        {
            self.textures_dropped += evicted.len();
        }
        evicted
    }

    /// Every picture there is, leaving none: for an editor that goes
    /// away.
    pub(crate) fn drain(&mut self) -> Vec<Arc<RenderImage>> {
        let mut pictures = std::mem::take(&mut self.evicted);
        for entry in self.entries.values_mut() {
            entry.loading = None;
            pictures.extend(entry.levels.drain(..).map(|have| have.picture));
        }
        self.bytes = 0;
        pictures
    }

    /// Bytes of decoded pixels in the cache.
    #[cfg(test)]
    pub(crate) fn decoded_bytes(&self) -> usize {
        self.bytes
    }

    /// The levels of an image that are decoded, finest first.
    #[cfg(test)]
    pub(crate) fn levels(&self, image: ImageId) -> Vec<u8> {
        let mut levels: Vec<u8> = self
            .entries
            .get(&image)
            .map(|entry| entry.levels.iter().map(|have| have.level).collect())
            .unwrap_or_default();
        levels.sort_unstable();
        levels
    }

    /// Sets how many bytes of decoded pixels are kept.
    #[cfg(test)]
    pub(crate) fn set_budget(&mut self, bytes: usize) {
        self.budget = bytes;
        self.evict();
    }
}

fn status_of(entry: Option<&Entry>, has_provider: bool) -> ImageStatus {
    match entry {
        Some(entry) if !entry.levels.is_empty() => ImageStatus::Ready,
        Some(Entry {
            failure: Some(Failure::Missing),
            ..
        }) => ImageStatus::Missing,
        Some(Entry {
            failure: Some(Failure::Broken),
            ..
        }) => ImageStatus::Broken,
        _ if !has_provider => ImageStatus::NoProvider,
        _ => ImageStatus::Loading,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_level_is_never_narrower_than_what_is_drawn_and_never_twice_as_wide() {
        // (pixels, device pixels drawn, level)
        for (pixels, drawn, level) in [
            ((4000, 3000), 1400., 1),
            ((4000, 3000), 2000., 1),
            ((4000, 3000), 2001., 0),
            ((4000, 3000), 999., 2),
            ((4000, 3000), 5000., 0),
            ((1, 1), 300., 0),
            ((800, 600), 1., 9),
            ((800, 600), 0., 9),
        ] {
            assert_eq!(level_for(pixels, drawn), level, "{pixels:?} at {drawn}");
            let width = (pixels.0 >> level).max(1) as f32;
            if drawn <= pixels.0 as f32 && drawn >= 1. {
                assert!(width >= drawn, "{pixels:?} at {drawn}: {width}");
                assert!(width < drawn * 2. + 1., "{pixels:?} at {drawn}: {width}");
            }
        }
        // The side limit wins over the width that is drawn.
        assert_eq!(level_for((20000, 100), 20000.), 2);
        assert_eq!(level_for((100, 20000), 100.), 2);
        assert_eq!(level_for((8192, 8192), 8192.), 0);
    }

    fn encoded(picture: &image::RgbaImage, format: image::ImageFormat) -> Vec<u8> {
        let mut bytes = Vec::new();
        let picture = image::DynamicImage::ImageRgba8(picture.clone());
        let picture = match format {
            // A JPEG has no alpha.
            image::ImageFormat::Jpeg => image::DynamicImage::ImageRgb8(picture.into_rgb8()),
            _ => picture,
        };
        picture
            .write_to(&mut std::io::Cursor::new(&mut bytes), format)
            .expect("encoded in memory");
        bytes
    }

    #[test]
    fn stored_bytes_decode_to_bgra_at_the_level_that_is_asked_for() {
        // Red at the left, blue at the right, half transparent below.
        let picture = image::RgbaImage::from_fn(64, 40, |x, y| {
            let alpha = if y < 20 { 255 } else { 128 };
            if x < 32 {
                image::Rgba([255, 0, 0, alpha])
            } else {
                image::Rgba([0, 0, 255, alpha])
            }
        });
        let png = encoded(&picture, image::ImageFormat::Png);

        let whole = decode(&png, 0).expect("a PNG");
        assert_eq!(whole.frame_count(), 1);
        assert_eq!((whole.size(0).width.0, whole.size(0).height.0), (64, 40));
        let pixels = whole.as_bytes(0).unwrap();
        // Blue, green, red, alpha; the alpha as it is (not multiplied in).
        assert_eq!(&pixels[..4], &[0, 0, 255, 255]);
        assert_eq!(&pixels[63 * 4..64 * 4], &[255, 0, 0, 255]);
        let below = 30 * 64 * 4;
        assert_eq!(&pixels[below..below + 4], &[0, 0, 255, 128]);

        // Halved twice: a quarter of each side, the same picture.
        let small = decode(&png, 2).expect("a PNG");
        assert_eq!((small.size(0).width.0, small.size(0).height.0), (16, 10));
        let pixels = small.as_bytes(0).unwrap();
        assert_eq!(&pixels[..4], &[0, 0, 255, 255]);
        assert_eq!(&pixels[15 * 4..16 * 4], &[255, 0, 0, 255]);
        // Never smaller than one pixel.
        let dot = decode(&png, 20).expect("a PNG");
        assert_eq!((dot.size(0).width.0, dot.size(0).height.0), (1, 1));

        // A JPEG, and of a GIF with several frames the first.
        let jpeg = decode(&encoded(&picture, image::ImageFormat::Jpeg), 1).expect("a JPEG");
        assert_eq!((jpeg.size(0).width.0, jpeg.size(0).height.0), (32, 20));
        let mut gif = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut gif);
            let green = image::RgbaImage::from_pixel(64, 40, image::Rgba([0, 255, 0, 255]));
            encoder
                .encode_frames([image::Frame::new(picture.clone()), image::Frame::new(green)])
                .expect("two frames");
        }
        let first = decode(&gif, 0).expect("a GIF");
        assert_eq!(first.frame_count(), 1);
        assert_eq!((first.size(0).width.0, first.size(0).height.0), (64, 40));
        let pixels = first.as_bytes(0).unwrap();
        assert!(
            pixels[2] > 200 && pixels[1] < 60,
            "red, not the second frame's green"
        );

        // What is no picture, or only the beginning of one.
        assert!(decode(b"not a picture", 0).is_none());
        assert!(decode(&png[..png.len() / 2], 0).is_none());
        assert!(decode(&[], 0).is_none());
    }
}
