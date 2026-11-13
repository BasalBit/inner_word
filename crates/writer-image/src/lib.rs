//! Image files for The Writer: bytes as they come (a file the writer chooses, the
//! clipboard, a drop, a path an agent names) to bytes as the library stores them.
//!
//! No GPUI and no library in here: the crate reads bytes and gives bytes, with ImageIO
//! and CoreGraphics. The app and the command layer for agents (`writer-agent`) both go
//! through it, so the two make the same thing of the same file.
//!
//! What the library stores is a PNG, a JPEG or a GIF, upright, at most
//! [`MAX_LONG_EDGE`] pixels on its long edge and at most [`MAX_STORED_BYTES`] bytes. A
//! file that already is one is stored as it came, byte for byte; anything else ImageIO
//! reads is decoded and written again as one of the three. The three limits are here;
//! the function that holds a file to them is not written yet.

#![warn(missing_docs)]

/// The long edge of a stored image, in pixels, at most. A larger picture is made
/// smaller on the way in.
pub const MAX_LONG_EDGE: u32 = 4096;

/// The bytes of one stored image, at most.
pub const MAX_STORED_BYTES: usize = 16 * 1024 * 1024;

/// The largest file that is read at all. A larger one is refused before it is read.
pub const MAX_INPUT_BYTES: u64 = 100 * 1024 * 1024;

#[cfg(test)]
mod tests {
    use super::*;

    /// The limits as the design has them, and in an order that makes sense: a file that
    /// may be read can be larger than what is stored of it, and a picture of the largest
    /// size, decoded, is what the limit on the long edge is there to bound.
    #[test]
    fn the_limits() {
        assert_eq!(MAX_LONG_EDGE, 4096);
        assert_eq!(MAX_STORED_BYTES, 16_777_216);
        assert_eq!(MAX_INPUT_BYTES, 104_857_600);
        assert!(MAX_INPUT_BYTES > MAX_STORED_BYTES as u64);
        let decoded = u64::from(MAX_LONG_EDGE) * u64::from(MAX_LONG_EDGE) * 4;
        assert_eq!(decoded, 64 * 1024 * 1024);
    }
}
