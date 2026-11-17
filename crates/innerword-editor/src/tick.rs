//! The tick of a checked to-do box on screen: a small picture the editor
//! draws itself, pixel by pixel, at the size and place it is shown.
//!
//! The tick is not a GPUI path, and nothing the editor paints on screen
//! is. In gpui 0.2.2 the Metal renderer keeps, for every window, an
//! intermediate texture the size of the window and a copy of it with four
//! samples a pixel for paths; they are made when the first path is drawn
//! and kept until the window is resized again. One ticked to-do on screen
//! cost a window of 1400 by 900 points about 100 MB of graphics memory,
//! and the user's larger window about 400 MB. A sprite of a few hundred
//! pixels costs a tile of GPUI's atlas.
//!
//! The sprite is drawn in device pixels, so it is as sharp at 400 % as at
//! 50 %. It holds the box's offset from the device pixel grid, so the tick
//! sits where the box is to the fraction of a pixel. The pictures a frame
//! does not use any more are handed back to the window (`Window::drop_image`)
//! at the start of the next frame, as the pictures of image blocks are.

use std::{collections::HashMap, sync::Arc};

use gpui::{Bounds, Hsla, Pixels, Point, RenderImage, Rgba, point, px, size};

use crate::decorations::CheckShape;

/// Where a tick's sprite is painted and what it is.
pub(crate) struct PlannedTick {
    /// In window coordinates, a hair inside the device pixels it covers:
    /// `Window::paint_image` floors the origin and ceils the size in
    /// device pixels, and the sprite is exactly that many pixels.
    pub(crate) bounds: Bounds<Pixels>,
    pub(crate) picture: Arc<RenderImage>,
}

/// Everything that decides the pixels of a tick, in eighths of a device
/// pixel (the colour as 8-bit RGBA).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TickKey {
    /// The box's offset from the device pixel its sprite starts at.
    offset: (i32, i32),
    /// The side of the box.
    side: i32,
    /// The width of the stroke.
    width: i32,
    color: [u8; 4],
}

const EIGHTHS: f32 = 8.;

fn eighths(value: f32) -> i32 {
    (value * EIGHTHS).round() as i32
}

impl TickKey {
    /// The key of `check` drawn at `scale_factor` device pixels a point,
    /// and the device pixel its sprite starts at.
    fn of(check: &CheckShape, scale_factor: f32) -> (Self, Point<i32>) {
        let left = f32::from(check.bounds.left()) * scale_factor;
        let top = f32::from(check.bounds.top()) * scale_factor;
        let start = point(left.floor() as i32, top.floor() as i32);
        let key = Self {
            offset: (eighths(left - left.floor()), eighths(top - top.floor())),
            side: eighths(f32::from(check.bounds.size.width) * scale_factor).max(EIGHTHS as i32),
            width: eighths(f32::from(check.width()) * scale_factor).max(1),
            color: rgba8(check.color),
        };
        (key, start)
    }

    /// The sprite's size in device pixels.
    fn pixels(&self) -> (u32, u32) {
        let (x, y) = self.offset;
        let side = self.side as f32 / EIGHTHS;
        (
            (x as f32 / EIGHTHS + side).ceil().max(1.) as u32,
            (y as f32 / EIGHTHS + side).ceil().max(1.) as u32,
        )
    }
}

fn rgba8(color: Hsla) -> [u8; 4] {
    let rgba = Rgba::from(color);
    [rgba.r, rgba.g, rgba.b, rgba.a].map(|channel| (channel.clamp(0., 1.) * 255.).round() as u8)
}

/// The distance from `p` to the segment from `a` to `b`.
fn distance_to_segment(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length = dx * dx + dy * dy;
    let t = if length > 0. {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / length).clamp(0., 1.)
    } else {
        0.
    };
    let (x, y) = (a.0 + t * dx - p.0, a.1 + t * dy - p.1);
    (x * x + y * y).sqrt()
}

/// The tick's pixels: BGRA with straight alpha, what GPUI paints. Each
/// pixel is covered as far as its centre lies inside the stroke (round
/// joins and caps, as the printed tick has), smoothed over one pixel.
pub(crate) fn rasterize(key: &TickKey) -> RenderImage {
    let (width, height) = key.pixels();
    let side = key.side as f32 / EIGHTHS;
    let half = key.width as f32 / EIGHTHS / 2.;
    let origin = (key.offset.0 as f32 / EIGHTHS, key.offset.1 as f32 / EIGHTHS);
    let points = CheckShape::POINTS.map(|(x, y)| (origin.0 + side * x, origin.1 + side * y));
    let [r, g, b, a] = key.color;
    let mut pixels = image::RgbaImage::new(width, height);
    for (x, y, pixel) in pixels.enumerate_pixels_mut() {
        let centre = (x as f32 + 0.5, y as f32 + 0.5);
        let distance = points
            .windows(2)
            .map(|pair| distance_to_segment(centre, pair[0], pair[1]))
            .fold(f32::INFINITY, f32::min);
        let coverage = (half + 0.5 - distance).clamp(0., 1.);
        let alpha = (a as f32 * coverage).round() as u8;
        // BGRA.
        pixel.0 = [b, g, r, alpha];
    }
    let frames: smallvec::SmallVec<[image::Frame; 1]> =
        smallvec::smallvec![image::Frame::new(pixels)];
    RenderImage::new(frames)
}

/// The ticks' sprites of one editor, kept while frames use them.
#[derive(Default)]
pub(crate) struct TickSprites {
    entries: HashMap<TickKey, (Arc<RenderImage>, bool)>,
    /// Sprites the last frame did not use: the window lets go of them at
    /// the start of the next frame.
    evicted: Vec<Arc<RenderImage>>,
}

impl TickSprites {
    /// The sprites to hand to `Window::drop_image`, and a new frame begins.
    pub(crate) fn begin_frame(&mut self) -> Vec<Arc<RenderImage>> {
        for (_, used) in self.entries.values_mut() {
            *used = false;
        }
        std::mem::take(&mut self.evicted)
    }

    /// What a frame paints for `check` at `scale_factor`.
    pub(crate) fn tick(&mut self, check: &CheckShape, scale_factor: f32) -> PlannedTick {
        let scale_factor = scale_factor.max(0.01);
        let (key, start) = TickKey::of(check, scale_factor);
        let (picture, used) = self
            .entries
            .entry(key)
            .or_insert_with(|| (Arc::new(rasterize(&key)), false));
        *used = true;
        let (width, height) = key.pixels();
        // A hair inside the pixels: floored and ceiled back to exactly them.
        let hair = 0.05;
        PlannedTick {
            bounds: Bounds::new(
                point(
                    px((start.x as f32 + hair) / scale_factor),
                    px((start.y as f32 + hair) / scale_factor),
                ),
                size(
                    px((width as f32 - hair) / scale_factor),
                    px((height as f32 - hair) / scale_factor),
                ),
            ),
            picture: picture.clone(),
        }
    }

    /// The frame's plan is made: what it did not use goes.
    pub(crate) fn end_frame(&mut self) {
        let evicted = &mut self.evicted;
        self.entries.retain(|_, (picture, used)| {
            if !*used {
                evicted.push(picture.clone());
            }
            *used
        });
    }

    /// Every sprite there is, leaving none: for an editor that goes away.
    pub(crate) fn drain(&mut self) -> Vec<Arc<RenderImage>> {
        let mut pictures = std::mem::take(&mut self.evicted);
        pictures.extend(self.entries.drain().map(|(_, (picture, _))| picture));
        pictures
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::hsla;

    fn check(left: f32, top: f32, side: f32) -> CheckShape {
        CheckShape {
            bounds: Bounds::new(point(px(left), px(top)), size(px(side), px(side))),
            color: hsla(0., 0., 1., 1.),
        }
    }

    /// The alpha of the sprite's pixel at (`x`, `y`).
    fn alpha(picture: &RenderImage, x: u32, y: u32) -> u8 {
        let (width, _) = {
            let size = picture.size(0);
            (size.width.0 as u32, size.height.0 as u32)
        };
        picture.as_bytes(0).unwrap()[((y * width + x) * 4 + 3) as usize]
    }

    #[test]
    fn the_tick_covers_its_three_points_and_leaves_the_corners() {
        for (side, scale) in [(7., 1.), (15., 2.), (15., 1.), (60., 2.)] {
            let shape = check(10., 20., side);
            let mut sprites = TickSprites::default();
            let tick = sprites.tick(&shape, scale);
            let device = side * scale;
            let picture = &tick.picture;
            let size = picture.size(0);
            assert_eq!(
                size.width.0 as f32, device,
                "on whole pixels: {side} at {scale}"
            );
            assert_eq!(size.height.0 as f32, device);
            for (x, y) in CheckShape::POINTS {
                let (x, y) = ((x * device) as u32, (y * device) as u32);
                assert!(
                    alpha(picture, x, y) > 200,
                    "{side} at {scale}: ({x}, {y}) is in the stroke"
                );
            }
            let last = device as u32 - 1;
            for (x, y) in [(0, 0), (last, 0), (0, last), (last, last)] {
                assert_eq!(alpha(picture, x, y), 0, "{side} at {scale}: a corner");
            }
            // Painted on exactly the device pixels of the box.
            let device_of = |value: Pixels| f32::from(value) * scale;
            assert_eq!(device_of(tick.bounds.left()).floor(), 10. * scale);
            assert_eq!(device_of(tick.bounds.top()).floor(), 20. * scale);
            assert_eq!(device_of(tick.bounds.size.width).ceil(), device);
        }
    }

    #[test]
    fn the_stroke_is_as_wide_as_the_printed_one() {
        // At 15 points the stroke is 1.8 points: 3.6 device pixels at 2x.
        let shape = check(0., 0., 15.);
        assert_eq!(shape.width(), px(1.8));
        let picture = TickSprites::default().tick(&shape, 2.).picture;
        // Across the long arm, at its middle, perpendicular to it: the
        // pixels that are mostly covered are about as many as the stroke
        // is wide.
        let device = 30.;
        let (a, b) = (CheckShape::POINTS[1], CheckShape::POINTS[2]);
        let middle = ((a.0 + b.0) / 2. * device, (a.1 + b.1) / 2. * device);
        let x = middle.0 as u32;
        let covered: f32 = (0..30)
            .map(|y| alpha(&picture, x, y) as f32 / 255.)
            .sum::<f32>();
        // A vertical cut across a line at about 50 degrees is longer than
        // the stroke is wide by 1 / cos.
        let angle = ((b.1 - a.1) / (b.0 - a.0)).atan();
        let expected = 3.6 / angle.cos().abs();
        assert!(
            (covered - expected).abs() < 0.6,
            "covered {covered}, expected {expected}"
        );
        // The minimum holds for a small box.
        assert_eq!(check(0., 0., 7.).width(), px(1.4));
    }

    #[test]
    fn a_box_off_the_pixel_grid_gets_a_sprite_of_its_own_offset() {
        let mut sprites = TickSprites::default();
        let on = sprites.tick(&check(10., 20., 15.), 1.);
        let off = sprites.tick(&check(10.5, 20., 15.), 1.);
        assert_eq!(sprites.len(), 2);
        assert_eq!(on.picture.size(0).width.0, 15);
        assert_eq!(off.picture.size(0).width.0, 16, "half a pixel in: one more");
        // Both start on the pixel the box starts in.
        assert_eq!(f32::from(off.bounds.origin.x).floor(), 10.);
    }

    #[test]
    fn sprites_a_frame_does_not_use_are_handed_back_once() {
        let mut sprites = TickSprites::default();
        assert!(sprites.begin_frame().is_empty());
        let first = sprites.tick(&check(0., 0., 15.), 2.);
        let again = sprites.tick(&check(0., 40., 15.), 2.);
        assert!(
            Arc::ptr_eq(&first.picture, &again.picture),
            "one sprite for one look"
        );
        sprites.end_frame();
        assert!(sprites.begin_frame().is_empty());
        // A zoom: another size.
        sprites.tick(&check(0., 0., 30.), 2.);
        sprites.end_frame();
        let dropped = sprites.begin_frame();
        assert_eq!(dropped.len(), 1);
        assert!(Arc::ptr_eq(&dropped[0], &first.picture));
        assert_eq!(sprites.drain().len(), 1);
        assert_eq!(sprites.len(), 0);
    }

    #[test]
    fn the_colour_is_the_tick_colour_with_straight_alpha() {
        let shape = CheckShape {
            bounds: Bounds::new(point(px(0.), px(0.)), size(px(20.), px(20.))),
            color: gpui::Hsla::from(gpui::rgb(0x336699)),
        };
        let picture = TickSprites::default().tick(&shape, 1.).picture;
        let (x, y) = ((0.43 * 20.) as u32, (0.70 * 20.) as u32);
        let at = ((y * 20 + x) * 4) as usize;
        let bytes = picture.as_bytes(0).unwrap();
        let [b, g, r, a] = [bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]];
        assert_eq!((r, g, b), (0x33, 0x66, 0x99));
        assert_eq!(a, 255);
    }
}
