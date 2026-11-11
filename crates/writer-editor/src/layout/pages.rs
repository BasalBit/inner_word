//! Pagination for page view.
//!
//! Status: skeleton. Page view is not laid out as sheets yet: the editor
//! paints the flow column in both views. Until rows are distributed over
//! sheets, pages are counted from explicit page breaks only, which is exact
//! for a document whose every page ends in a page break and a lower bound
//! otherwise.
//!
//! The implementer replaces this with real pagination: the page content
//! size comes from `DocumentSettings::page` (`PageSetup::content_size`, in
//! points; one point is one logical pixel), rows are assigned to pages from
//! the block layouts (a row never splits; a `PageBreak` block starts a new
//! page), and the result feeds both the element (sheet backgrounds, page
//! numbers, gaps between sheets in the block tops) and these two queries.

use writer_core::{BlockKind, Document};

/// Number of pages of `document`.
pub(crate) fn page_count(document: &Document) -> usize {
    1 + document
        .blocks()
        .iter()
        .filter(|block| matches!(block.kind(), BlockKind::PageBreak))
        .count()
}

/// 1-based page that block `block` is on.
pub(crate) fn page_of_block(document: &Document, block: usize) -> usize {
    1 + document
        .blocks()
        .iter()
        .take(block)
        .filter(|block| matches!(block.kind(), BlockKind::PageBreak))
        .count()
}
