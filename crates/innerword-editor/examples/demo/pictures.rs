//! The pictures of the demo document. They are painted here, so the demo
//! needs no files: a landscape (a photograph's proportions, fine lines to
//! judge how it is scaled), a tall "phone screenshot" (taller than a page
//! lets a picture be at its full width), a small stamp, and one image the
//! provider has no bytes for (its placeholder says so).
//!
//! The provider is `innerword_editor::FakeImages`: pictures in memory. The
//! bytes are kept here as well, for the print pass.

use std::{collections::HashMap, io::Cursor, rc::Rc, sync::Arc};

use image::{Rgba, RgbaImage};
use innerword_core::{Alignment, Block, Image, ImageId, ImageWidth, PageSetup};
use innerword_editor::FakeImages;

pub struct Pictures {
    pub provider: Rc<FakeImages>,
    pub landscape: Image,
    pub phone: Image,
    pub stamp: Image,
    /// An image nobody has bytes for.
    pub missing: Image,
    bytes: HashMap<ImageId, Arc<[u8]>>,
}

fn png(picture: &RgbaImage) -> Vec<u8> {
    let mut bytes = Vec::new();
    picture
        .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
        .expect("a PNG can be written to memory");
    bytes
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let t = t.clamp(0., 1.);
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

fn rgba(colour: [f32; 3]) -> Rgba<u8> {
    Rgba([colour[0] as u8, colour[1] as u8, colour[2] as u8, 255])
}

/// A harbour at dusk: sky, sun, two ranges of hills, water, and the masts
/// of a few boats (lines two pixels wide).
fn landscape(width: u32, height: u32) -> RgbaImage {
    let (w, h) = (width as f32, height as f32);
    RgbaImage::from_fn(width, height, |x, y| {
        let (fx, fy) = (x as f32 / w, y as f32 / h);
        let mut colour = mix([92., 132., 190.], [250., 214., 170.], fy / 0.62);
        // The sun and its glow.
        let (dx, dy) = ((fx - 0.72) * w / h, fy - 0.36);
        let from_sun = (dx * dx + dy * dy).sqrt();
        colour = mix(colour, [255., 236., 196.], 1. - from_sun / 0.35);
        if from_sun < 0.075 {
            colour = [255., 244., 214.];
        }
        let far = 0.56 + 0.05 * (fx * 9.4).sin() + 0.02 * (fx * 23.).sin();
        if fy > far {
            colour = mix([104., 128., 136.], [78., 104., 108.], (fy - far) / 0.2);
        }
        let near = 0.68 + 0.04 * (fx * 6.3 + 1.2).sin() + 0.015 * (fx * 31.).sin();
        if fy > near {
            colour = mix([52., 84., 72.], [36., 60., 56.], (fy - near) / 0.2);
        }
        if fy > 0.8 {
            // Water, with the light on it.
            let ripple = ((fy * h / 3.).floor() as i32 % 4 == 0) as u8 as f32;
            colour = mix([58., 88., 128.], [30., 50., 84.], (fy - 0.8) / 0.2);
            let glint = (1. - ((fx - 0.72).abs() / 0.12)).max(0.) * ripple;
            colour = mix(colour, [250., 220., 180.], glint * 0.7);
        }
        // Masts.
        for (at, top) in [(0.18, 0.5), (0.24, 0.56), (0.31, 0.46), (0.47, 0.6)] {
            let mast = (fx - at).abs() * w < 1.01 && fy > top && fy < 0.84;
            let boom = (fy - (top + 0.2)).abs() * h < 1.01 && fx > at && fx < at + 0.035;
            if mast || boom {
                colour = [34., 34., 40.];
            }
        }
        rgba(colour)
    })
}

/// Something like the screenshot of a phone: a bar on top and a list of
/// cards.
fn phone(width: u32, height: u32) -> RgbaImage {
    RgbaImage::from_fn(width, height, |x, y| {
        let (w, card) = (width as f32, 150);
        if y < 110 {
            return rgba([36., 38., 46.]);
        }
        let row = (y - 110) / card;
        let in_row = (y - 110) % card;
        let inside = in_row > 14 && in_row < card - 6 && x > 24 && (x as f32) < w - 24.;
        if !inside {
            return rgba([238., 238., 242.]);
        }
        // A dot and two lines of "text".
        let (dx, dy) = (x as f32 - 84., in_row as f32 - 80.);
        if dx * dx + dy * dy < 34. * 34. {
            let hues = [
                [204., 96., 72.],
                [84., 142., 196.],
                [96., 164., 120.],
                [212., 164., 72.],
            ];
            return rgba(hues[row as usize % hues.len()]);
        }
        let line = |top: u32, length: f32| {
            in_row > top && in_row < top + 14 && x > 150 && (x as f32) < 150. + length
        };
        if line(48, w * 0.45) {
            return rgba([60., 62., 72.]);
        }
        if line(84, w * 0.3 + (row % 3) as f32 * 40.) {
            return rgba([170., 172., 182.]);
        }
        rgba([255., 255., 255.])
    })
}

/// A small mark: a ring on a square.
fn stamp(side: u32) -> RgbaImage {
    RgbaImage::from_fn(side, side, |x, y| {
        let half = side as f32 / 2.;
        let (dx, dy) = (x as f32 - half, y as f32 - half);
        let from_centre = (dx * dx + dy * dy).sqrt();
        if from_centre > half * 0.55 && from_centre < half * 0.8 {
            rgba([250., 244., 236.])
        } else {
            rgba([184., 84., 60.])
        }
    })
}

impl Pictures {
    pub fn paint() -> Pictures {
        let provider = FakeImages::new();
        let mut bytes = HashMap::new();
        let mut add = |picture: RgbaImage, density: u16, description: &str| {
            let encoded = png(&picture);
            let mut image = provider
                .add_bytes(encoded.clone())
                .expect("a PNG is a picture");
            // What the file would declare: a display of two pixels to the
            // point for the two large ones.
            image.density = density;
            image.description = description.to_string();
            bytes.insert(image.id, Arc::from(encoded));
            image
        };
        let landscape = add(landscape(1600, 1000), 144, "The harbour at dusk");
        let phone = add(phone(780, 1688), 144, "");
        let stamp = add(stamp(96), 72, "");
        let mut missing = Image::new(ImageId::from_bytes([7; 16]), (1200, 800), 144);
        missing.description = "A picture the library does not have".to_string();
        Pictures {
            provider,
            landscape,
            phone,
            stamp,
            missing,
            bytes,
        }
    }

    /// The stored bytes of an image, for the print pass.
    pub fn bytes(&self) -> Rc<dyn Fn(ImageId) -> Option<Arc<[u8]>>> {
        let bytes = self.bytes.clone();
        Rc::new(move |image| bytes.get(&image).cloned())
    }

    /// The image blocks of the sample document: the landscape centred at
    /// three fifths of the column, the stamp at its own size at the left,
    /// the phone screenshot as wide as a page lets it be at the right,
    /// and the one without bytes.
    pub fn blocks(&self) -> Vec<Block> {
        let page = PageSetup::default();
        let at = |image: &Image, width: ImageWidth, alignment: Alignment| {
            let mut image = image.clone();
            image.width = width;
            Block::image(image).with_alignment(alignment)
        };
        vec![
            at(
                &self.landscape,
                ImageWidth::from_percent(60),
                Alignment::Center,
            ),
            Block::paragraph(
                "A paragraph between two pictures. An image is a block of its own; the text \
                 goes on under it.",
            ),
            at(
                &self.stamp,
                self.stamp.original_width(&page),
                Alignment::Left,
            ),
            at(&self.phone, self.phone.max_width(&page), Alignment::Right),
            at(
                &self.missing,
                ImageWidth::from_percent(40),
                Alignment::Center,
            ),
        ]
    }

    /// One of the two large pictures, for a generated book: full width.
    pub fn for_book(&self, index: usize) -> Block {
        let image = if index.is_multiple_of(2) {
            &self.landscape
        } else {
            &self.phone
        };
        let mut image = image.clone();
        image.width = image.max_width(&PageSetup::default());
        Block::image(image).with_alignment(Alignment::Center)
    }
}
