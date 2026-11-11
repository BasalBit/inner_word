//! Document layout: per-block layouts cached on writer-core's revision
//! stamps, stacked into one scrollable column.
//!
//! - [`style`]: attributes to text runs, block metrics.
//! - [`block`]: one block's shaped lines, visual rows and hit testing.
//! - [`pages`]: pagination for page view (Status: skeleton).
//! - this module: [`DocumentLayout`], the cache and the vertical stack.
//!
//! # How it stays valid
//!
//! The element's prepaint calls [`DocumentLayout::set_env`] (viewport,
//! column, style, the window's text system) and then
//! [`DocumentLayout::sync`]. Between frames, every geometry or visual-row
//! query syncs again first, reusing the environment of the last frame, so a
//! consumer that edits and then asks for a caret position in the same key
//! handler (Vim does) gets an answer for the edited document, not for the
//! last painted one.
//!
//! A block's layout is cached under `(BlockId, Block::revision)`. Typing in
//! one block reshapes one block. The cache is dropped when the column width
//! or the style changes, and when the document is replaced (stamps restart
//! per `EditorState`).
//!
//! Status: skeleton in two respects. Every block of the document is laid
//! out (no virtualisation with estimated heights), and `sync` rebuilds the
//! whole stack of block tops after any change instead of using the
//! `BlockSplice`. Both are linear in the number of blocks.

pub(crate) mod block;
pub(crate) mod pages;
pub(crate) mod style;

use std::{collections::HashMap, rc::Rc, sync::Arc};

use gpui::{Bounds, Pixels, WindowTextSystem, px};
use writer_core::{BlockId, Document};

pub(crate) use block::BlockLayout;
pub(crate) use style::LayoutStyle;

/// What a layout pass needs from the outside. Captured at prepaint and kept
/// for on-demand layout between frames.
pub(crate) struct LayoutEnv {
    pub(crate) text_system: Arc<WindowTextSystem>,
    pub(crate) style: LayoutStyle,
    /// The editor element's bounds, in window coordinates.
    pub(crate) viewport: Bounds<Pixels>,
    /// Window x of the text column's left edge.
    pub(crate) column_left: Pixels,
    pub(crate) column_width: Pixels,
    /// Space above the first block and below the last one.
    pub(crate) pad_top: Pixels,
    pub(crate) pad_bottom: Pixels,
}

#[derive(Default)]
pub(crate) struct DocumentLayout {
    env: Option<LayoutEnv>,
    cache: HashMap<BlockId, (u64, Rc<BlockLayout>)>,
    /// Layout of every block of the document at `revision`.
    blocks: Vec<Rc<BlockLayout>>,
    /// Content-space y of each block's top, plus one entry for the end of
    /// the last block. Content space starts at the top of the scrollable
    /// content (`pad_top` is included).
    tops: Vec<Pixels>,
    /// The `EditorState::revision` that `blocks` and `tops` describe.
    revision: Option<u64>,
}

impl DocumentLayout {
    /// Forgets every cached block layout. For a replaced document.
    pub(crate) fn clear(&mut self) {
        self.cache.clear();
        self.blocks.clear();
        self.tops.clear();
        self.revision = None;
    }

    /// Marks the stack stale without dropping block layouts (the block
    /// stamps decide what is reshaped).
    pub(crate) fn invalidate(&mut self) {
        self.revision = None;
    }

    /// Installs the environment of a new frame. Drops the cache when the
    /// column width or the style changed.
    pub(crate) fn set_env(&mut self, env: LayoutEnv) {
        if let Some(old) = &self.env {
            if old.column_width != env.column_width || old.style != env.style {
                self.clear();
            } else if old.pad_top != env.pad_top {
                self.invalidate();
            }
        }
        self.env = Some(env);
    }

    pub(crate) fn env(&self) -> Option<&LayoutEnv> {
        self.env.as_ref()
    }

    /// Brings `blocks` and `tops` up to date with `document` at `revision`.
    /// Returns false when there is no environment yet (nothing was ever
    /// painted or measured).
    pub(crate) fn sync(&mut self, document: &Document, revision: u64) -> bool {
        let Some(env) = &self.env else {
            return false;
        };
        if self.revision == Some(revision) && self.blocks.len() == document.block_count() {
            return true;
        }
        self.blocks.clear();
        self.tops.clear();
        let mut y = env.pad_top;
        for block in document.blocks() {
            let layout = match self.cache.get(&block.id()) {
                Some((stamp, layout)) if *stamp == block.revision() => layout.clone(),
                _ => {
                    let layout = Rc::new(BlockLayout::shape(
                        block,
                        &env.style,
                        env.column_width,
                        &env.text_system,
                    ));
                    self.cache
                        .insert(block.id(), (block.revision(), layout.clone()));
                    layout
                }
            };
            self.tops.push(y);
            y += layout.height;
            self.blocks.push(layout);
        }
        self.tops.push(y);
        self.revision = Some(revision);
        // Forget blocks that left the document once they outnumber the live
        // ones (undo may still bring a few back, so not eagerly).
        if self.cache.len() > self.blocks.len() * 2 + 64 {
            let live: std::collections::HashSet<BlockId> =
                document.blocks().iter().map(|block| block.id()).collect();
            self.cache.retain(|id, _| live.contains(id));
        }
        true
    }

    pub(crate) fn block_count(&self) -> usize {
        self.blocks.len()
    }

    pub(crate) fn block(&self, index: usize) -> Option<&Rc<BlockLayout>> {
        self.blocks.get(index)
    }

    /// Content-space y of the top of block `index` (clamped).
    pub(crate) fn block_top(&self, index: usize) -> Pixels {
        self.tops
            .get(index)
            .or(self.tops.last())
            .copied()
            .unwrap_or(px(0.))
    }

    /// Height of the scrollable content, padding included.
    pub(crate) fn content_height(&self) -> Pixels {
        let pad_bottom = self.env.as_ref().map_or(px(0.), |env| env.pad_bottom);
        self.tops.last().copied().unwrap_or(px(0.)) + pad_bottom
    }

    /// The block whose vertical extent contains content-space `y`, clamped
    /// to the first and last block.
    pub(crate) fn block_at_y(&self, y: Pixels) -> usize {
        let count = self.blocks.len();
        if count == 0 {
            return 0;
        }
        self.tops[..count]
            .partition_point(|top| *top <= y)
            .saturating_sub(1)
    }
}
