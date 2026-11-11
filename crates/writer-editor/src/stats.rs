//! Statistics and pages, cached for status bars.

use writer_core::Stats;

use crate::{editor::Editor, layout::pages};

impl Editor {
    /// Word, character and paragraph counts and the reading time of the
    /// whole document. Kept up to date by the mutation seam (writer-core
    /// caches per block), so this is a plain read.
    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// The same counts for the selected text, or `None` while the selection
    /// is collapsed. Computed on first use after the selection or the
    /// document changed, then cached.
    pub fn selection_stats(&self) -> Option<Stats> {
        let selection = self.state.selection();
        if selection.is_collapsed() {
            return None;
        }
        let revision = self.state.revision();
        if let Some((cached_revision, cached_selection, stats)) = self.selection_stats.get()
            && cached_revision == revision
            && cached_selection == selection
        {
            return Some(stats);
        }
        let stats = self.state.selection_stats();
        self.selection_stats.set(Some((revision, selection, stats)));
        Some(stats)
    }

    /// The number of pages in page view; `None` in flow view and in compact
    /// mode.
    ///
    /// Status: skeleton. Counts explicit page breaks only (see
    /// `layout::pages`); real pagination replaces it.
    pub fn page_count(&self) -> Option<usize> {
        self.paginated()
            .then(|| pages::page_count(self.state.document()))
    }

    /// The 1-based page the caret is on in page view; `None` in flow view
    /// and in compact mode.
    ///
    /// Status: skeleton, as [`Editor::page_count`].
    pub fn caret_page(&self) -> Option<usize> {
        self.paginated().then(|| {
            pages::page_of_block(self.state.document(), self.state.selection().head.block)
        })
    }

    fn paginated(&self) -> bool {
        self.page_view() && !self.config.mode.is_compact()
    }
}
