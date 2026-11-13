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
//! * [`mod@pdf`]: [`PdfWriter`] draws pages into a PDF in memory (CoreGraphics for
//!   shapes and pictures, CoreText for glyphs, in the fonts the layout named) and
//!   hands out a [`Pdf`] with a [`Report`] of what it did.
//! * [`mod@read`]: [`PdfFile`] reads a PDF back: page count, page sizes, the text of
//!   a page, where a word stands, a page as a picture. For tests and for looking.
//! * [`mod@system`]: the print operation. [`run_sheet`] shows the print panel on a
//!   window, [`run_save_job`] runs the same operation into a file without a panel,
//!   [`check_selectors`] asks the Objective-C runtime for everything the two send.
//!
//! ```ignore
//! let mut writer = PdfWriter::new(&JobInfo { title, creator });
//! for page in &pages {
//!     writer.add_page(page);
//! }
//! let pdf = writer.finish();
//! // From a foreground task, outside any GPUI update:
//! unsafe { run_sheet(&PrintRequest { pdf: &pdf, paper, title: &title }, ns_window, done)? };
//! ```
//!
//! # What decides where a glyph stands
//!
//! The layout, never this crate. A [`TextRow`] carries the x of every glyph the
//! layout shaped; the writer asks CoreText for the glyphs of the same text in the
//! same fonts and puts each where its cluster starts in the layout. So justified
//! rows, inline code at its smaller size and right-aligned text are on paper where
//! they are on screen without any of that being computed twice. A row that CoreText
//! measures at another width than the layout did was set in another face; it is
//! still whole and in place, and [`Report::rows_off`] counts it.
//!
//! The crate builds and tests in the default `target/` directory, not in `target-pure`
//! (see its manifest).

#![warn(missing_docs)]

mod fonts;
pub mod page;
pub mod pdf;
pub mod read;
pub mod system;
mod text;

#[cfg(test)]
mod tests;

pub use page::{
    Anchor, Color, FontSpec, GlyphStart, ImageItem, Item, Label, Page, RectItem, RowRun,
    StrokeItem, TextRow,
};
pub use pdf::{JobInfo, Pdf, PdfWriter, Report};
pub use read::PdfFile;
pub use system::{PrintError, PrintRequest, check_selectors, run_save_job, run_sheet};
