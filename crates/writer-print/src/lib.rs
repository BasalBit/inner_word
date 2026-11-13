//! Printing for The Writer.
//!
//! No GPUI in here. The editor's layout decides what stands where on a sheet, exactly
//! as page view shows it, and says so as data: [`Page`]s of [`Item`]s ([`mod@page`]).
//! This crate is what becomes of those pages afterwards: a PDF with real text in it,
//! and the macOS print operation that takes the PDF.
//!
//! * [`mod@page`]: what a printed page is. Plain data, `Send`, no framework types:
//!   rectangles, strokes, rows of text with the place of every glyph, single-line
//!   labels, images. All lengths are points (1/72 inch); a page's origin is its top
//!   left corner, y down; items are painted in order.
//!
//! The crate builds and tests in the default `target/` directory, not in `target-pure`
//! (see its manifest).

#![warn(missing_docs)]

pub mod page;

pub use page::{
    Anchor, Color, FontSpec, GlyphStart, ImageItem, Item, Label, Page, RectItem, RowRun,
    StrokeItem, TextRow,
};
