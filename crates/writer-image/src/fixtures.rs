//! Small picture files for the tests of every crate that needs real ones
//! (`writer-agent`, `writer-cli`, `writer-app`, and this crate's own): made with ImageIO
//! when called, so no binary file is in the repository. Not for the app.
//!
//! Each picture is one flat colour. Each file is one [`crate::prepare`] passes as it is,
//! with the size and the density asked for. A function panics when this Mac's ImageIO
//! does not write the file, which for these three kinds it always does.

use crate::imageio::{Picture, Written, uti, write};

/// An opaque PNG of one colour, `width` by `height` pixels, that declares no density
/// (so it counts as 72 pixels per inch).
pub fn png(width: u32, height: u32, rgb: [u8; 3]) -> Vec<u8> {
    flat(width, height, rgb, uti::PNG, &Written::default())
}

/// An opaque PNG of one colour that declares `dpi` pixels per inch. A screenshot of a
/// Retina display is one with 144.
pub fn png_at(width: u32, height: u32, rgb: [u8; 3], dpi: u32) -> Vec<u8> {
    let with = Written {
        dpi: Some(dpi),
        ..Written::default()
    };
    flat(width, height, rgb, uti::PNG, &with)
}

/// A JPEG of one colour, upright and without a position, at 72 pixels per inch.
pub fn jpeg(width: u32, height: u32, rgb: [u8; 3]) -> Vec<u8> {
    let with = Written {
        quality: Some(0.9),
        ..Written::default()
    };
    flat(width, height, rgb, uti::JPEG, &with)
}

/// A GIF of one colour with a single frame.
pub fn gif(width: u32, height: u32, rgb: [u8; 3]) -> Vec<u8> {
    flat(width, height, rgb, uti::GIF, &Written::default())
}

fn flat(width: u32, height: u32, rgb: [u8; 3], kind: &str, with: &Written) -> Vec<u8> {
    let picture = Picture::flat(width, height, rgb, None);
    write(&[&picture], kind, with)
        .unwrap_or_else(|| panic!("ImageIO did not write a {width} x {height} file of kind {kind}"))
}
