//! Statistics and pages, cached for status bars.
//!
//! Word counts are kept per block and summed incrementally: an edit
//! recounts the blocks it changed, nothing else. Counting a whole book (on
//! open, or after a change that touches every block) is done by the
//! background layout task in slices, so it never blocks typing; until it
//! is done [`Editor::stats`] returns the last complete result.

use innerword_core::Stats;

use crate::editor::Editor;

impl Editor {
    /// Word, character and paragraph counts and the reading time of the
    /// whole document. A plain read: the counts are maintained by the
    /// mutation seam.
    ///
    /// After a change that rewrites more than about 64 KB of text at once
    /// (opening a book, a document-wide formatting command) the counts are
    /// brought up to date in the background; until then this returns the
    /// previous result (zero for a document that was just opened), and
    /// [`crate::EditorEvent::StatsChanged`] is emitted when the new one is
    /// ready. [`Editor::stats_pending`] tells the two cases apart.
    pub fn stats(&self) -> Stats {
        self.layout.borrow().stats()
    }

    /// Whether [`Editor::stats`] is still being brought up to date in the
    /// background.
    pub fn stats_pending(&self) -> bool {
        !self.layout.borrow().stats_complete()
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
        let document = self.state.document();
        let whole = selection.start() == document.start() && selection.end() == document.end();
        let stats = if whole && !self.stats_pending() {
            // Everything is selected: the document's counts are the answer.
            self.stats()
        } else {
            self.state.selection_stats()
        };
        self.selection_stats.set(Some((revision, selection, stats)));
        Some(stats)
    }

    /// The number of pages in page view; `None` in flow view and in compact
    /// mode.
    ///
    /// Pages are laid out lazily: blocks that were never on screen count
    /// with an estimated height until the background layout has measured
    /// them (a moment after a long document opens), so the number can
    /// still change while [`Editor::layout_pending`] is true. The editor
    /// notifies its observers when it does and emits
    /// [`crate::EditorEvent::LayoutSettled`] when the number is final.
    pub fn page_count(&self) -> Option<usize> {
        if !self.paginated() {
            return None;
        }
        self.with_layout(|lay| lay.layout.page_count())
    }

    /// The 1-based page the caret is on in page view; `None` in flow view
    /// and in compact mode. Subject to the same estimates as
    /// [`Editor::page_count`].
    pub fn caret_page(&self) -> Option<usize> {
        if !self.paginated() {
            return None;
        }
        let head = self.state.selection().head;
        let upstream = self.caret_is_upstream();
        self.with_layout(|lay| {
            let placed = lay.placed(head.block)?;
            let row = placed.layout.row_for_caret(head.offset, upstream);
            Some(lay.page_at(placed.row_top(row)) + 1)
        })
        .flatten()
    }

    /// Whether the background layout is still replacing estimated block
    /// heights with measured ones. While it is, the scroll range, the page
    /// count and positions far outside the viewport are approximate.
    pub fn layout_pending(&self) -> bool {
        !self.layout.borrow().settled()
    }

    fn paginated(&self) -> bool {
        self.page_view() && !self.config.mode.is_compact()
    }
}
