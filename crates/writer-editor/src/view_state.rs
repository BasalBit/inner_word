//! A serialisable snapshot of where the user was in a document: selection
//! and scroll position. The app stores one per document and restores it
//! when the document is opened again.

use gpui::{Context, px};
use serde::{Deserialize, Serialize};
use writer_core::{Position, Selection};

use crate::{
    editor::Editor,
    scroll::{Autoscroll, PendingScroll},
};

/// Selection and scroll position of an editor. Plain data: store it as JSON
/// next to the document. Every field has a default, so older or partial
/// snapshots still load.
///
/// The scroll position is stored relative to a block (the block at the top
/// of the viewport and the distance scrolled into it), so it survives a
/// different window width.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewState {
    /// Block index of the selection anchor.
    pub anchor_block: usize,
    /// Byte offset of the selection anchor in its block.
    pub anchor_offset: usize,
    /// Block index of the selection head (the caret).
    pub head_block: usize,
    /// Byte offset of the selection head in its block.
    pub head_offset: usize,
    /// Index of the block at the top of the viewport.
    pub scroll_block: usize,
    /// How far the top of the viewport is below that block's top, in
    /// pixels.
    pub scroll_offset: f32,
}

impl ViewState {
    /// The selection in the snapshot.
    pub fn selection(&self) -> Selection {
        Selection::new(
            Position::new(self.anchor_block, self.anchor_offset),
            Position::new(self.head_block, self.head_offset),
        )
    }
}

impl Editor {
    /// The current selection and scroll position.
    pub fn view_state(&self) -> ViewState {
        let selection = self.state.selection();
        // A restore that has not been laid out yet is still the truth.
        let scroll = match self.pending_scroll {
            Some(PendingScroll::BlockOffset { block, offset }) => Some((block, offset)),
            _ => self.with_layout(|layout, _| {
                let block = layout.block_at_y(self.scroll_y);
                (block, self.scroll_y - layout.block_top(block))
            }),
        };
        let (scroll_block, scroll_offset) = scroll.unwrap_or((0, px(0.)));
        ViewState {
            anchor_block: selection.anchor.block,
            anchor_offset: selection.anchor.offset,
            head_block: selection.head.block,
            head_offset: selection.head.offset,
            scroll_block,
            scroll_offset: f32::from(scroll_offset),
        }
    }

    /// Restores a snapshot: sets the selection (clamped to the document)
    /// and the scroll position. Works before the first layout; the scroll
    /// position is then applied on the first frame. The caret is not
    /// scrolled into view, the saved scroll position wins.
    pub fn restore_view_state(&mut self, view: &ViewState, cx: &mut Context<Self>) {
        let selection = view.selection();
        self.edit_with(Autoscroll::None, |state| state.set_selection(selection), cx);
        let offset = if view.scroll_offset.is_finite() {
            view.scroll_offset
        } else {
            0.
        };
        self.request_scroll(PendingScroll::BlockOffset {
            block: view.scroll_block,
            offset: px(offset),
        });
        cx.notify();
    }
}
