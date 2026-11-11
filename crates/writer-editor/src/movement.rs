//! The visual-row oracle: vertical movement over wrapped rows.
//!
//! writer-core moves the caret without layout (characters, words, hard
//! lines, blocks). Everything that depends on how text wraps is answered
//! here: which visual row a position is on, where N rows up or down is,
//! what fits in the viewport. The editor's own `up` / `down` / page keys
//! use it, and so does the Vim glue (`j`, `k`, `gj`, `H`, `M`, `L`,
//! `ctrl-d`, …).
//!
//! Like all geometry, the oracle answers `None` before the first layout and
//! is otherwise valid for the current document, also right after an edit in
//! the same handler and for positions that are scrolled out of view (the
//! blocks a query walks over are shaped on demand).
//!
//! A visual row is one wrapped line of one block. A divider or page break is
//! one row with a single position. A position at a soft wrap belongs to the
//! row that starts there, so [`VisualRow::end`] of a soft-wrapped row is
//! also the start of the next row; the oracle's vertical movement and
//! [`Editor::position_at_point`] never land on such an end, they stop one
//! character before it.
//!
//! The editor's own caret is the exception: it has an affinity
//! (`Editor::caret_is_upstream`), so its `up` / `down`, line-end key and
//! clicks can put it at the end of a soft-wrapped row and keep it drawn
//! there. That is view state of the editor; positions and the public
//! oracle know nothing of it.

use std::cell::RefCell;

use gpui::Pixels;
use writer_core::{Document, EditorState, Position};

use crate::{
    editor::Editor,
    geometry::with_layout_of,
    layout::{DocumentLayout, Lay},
};

/// One visual row of the document.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VisualRow {
    /// The first position on the row.
    pub start: Position,
    /// The position after the row's last character (before the line break
    /// that ends a hard line). For a soft-wrapped row this equals the start
    /// of the next row.
    pub end: Position,
}

/// The result of [`Editor::move_by_visual_rows`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RowMove {
    /// Where the caret lands.
    pub position: Position,
    /// The goal x that was used: pass it to the next vertical move so a
    /// run of `up` / `down` keeps its column across short rows. Measured
    /// from the left edge of the text column, so it survives scrolling.
    pub goal_x: Pixels,
    /// How many rows the move actually covered (fewer than asked at the
    /// start or end of the document; negative for upward moves).
    pub rows_moved: isize,
}

/// The visual-row oracle for an [`EditorState`] that is mutably borrowed:
/// what [`Editor::edit_with_rows`] hands to its closure next to the state.
///
/// It answers the same questions as [`Editor::visual_row`],
/// [`Editor::column_x`], [`Editor::move_by_visual_rows`],
/// [`Editor::visible_rows`] and [`Editor::rows_per_page`], for the state as
/// it is at the moment of the call: also after the closure has already
/// changed the document (the layout is then brought in line with it first,
/// which costs a pass over the blocks; asking before editing costs
/// nothing extra).
///
/// Every method takes the state it is asked about. That must be the state
/// the closure was given; for any other `EditorState` (a clone, another
/// editor's) the answer is `None`, as it is before the first layout.
pub struct RowOracle<'a> {
    layout: &'a RefCell<DocumentLayout>,
    /// The editor's own state, by address only: never dereferenced, only
    /// compared with the state a question is asked about.
    owner: *const EditorState,
}

impl<'a> RowOracle<'a> {
    pub(crate) fn new(layout: &'a RefCell<DocumentLayout>, owner: *const EditorState) -> Self {
        Self { layout, owner }
    }

    /// Whether `state` is the state this oracle's layout describes.
    fn owns(&self, state: &EditorState) -> bool {
        std::ptr::eq(state, self.owner)
    }

    /// The visual row containing `position` (see [`Editor::visual_row`]).
    pub fn visual_row(&self, state: &EditorState, position: Position) -> Option<VisualRow> {
        if !self.owns(state) {
            return None;
        }
        let position = state.document().clamp_position(position);
        with_layout_of(self.layout, state, |lay| {
            let block = lay.placed(position.block)?.layout;
            let row = &block.rows[block.row_for_offset(position.offset)];
            Some(VisualRow {
                start: Position::new(position.block, row.range.start),
                end: Position::new(position.block, row.range.end),
            })
        })
        .flatten()
    }

    /// The x of `position` from the left edge of the text column (see
    /// [`Editor::column_x`]).
    pub fn column_x(&self, state: &EditorState, position: Position) -> Option<Pixels> {
        if !self.owns(state) {
            return None;
        }
        let position = state.document().clamp_position(position);
        with_layout_of(self.layout, state, |lay| {
            let block = lay.placed(position.block)?.layout;
            Some(block.caret_bounds(position.offset).1.origin.x)
        })
        .flatten()
    }

    /// Moves `position` by `rows` visual rows (see
    /// [`Editor::move_by_visual_rows`]). Does not touch the selection.
    pub fn move_by_visual_rows(
        &self,
        state: &EditorState,
        position: Position,
        rows: isize,
        goal_x: Option<Pixels>,
    ) -> Option<RowMove> {
        if !self.owns(state) {
            return None;
        }
        let document = state.document();
        with_layout_of(self.layout, state, |lay| {
            move_rows(lay, document, position, rows, goal_x, None)
        })
        .flatten()
        .map(|(step, _)| step)
    }

    /// How many rows of body text fit in the viewport (see
    /// [`Editor::rows_per_page`]).
    pub fn rows_per_page(&self) -> Option<usize> {
        let layout = self.layout.borrow();
        let env = layout.env()?;
        let rows = (env.viewport.size.height / env.style.base_line_height()).floor() as usize;
        Some(rows.max(1))
    }

    /// The rows that are completely inside the viewport, top to bottom
    /// (see [`Editor::visible_rows`]). The scroll position is the one from
    /// before the closure ran: scroll requests take effect after it.
    pub fn visible_rows(&self, state: &EditorState) -> Option<Vec<VisualRow>> {
        if !self.owns(state) {
            return None;
        }
        // Half a pixel of tolerance: a row scrolled exactly to an edge
        // counts as visible despite float rounding.
        let slack = gpui::px(0.5);
        with_layout_of(self.layout, state, |lay| {
            let top = lay.scroll_y();
            let bottom = top + lay.env().viewport.size.height;
            let count = lay.count();
            let first_block = lay.block_at(top);
            let mut rows = Vec::new();
            for block_ix in first_block..count {
                let placed = lay.placed(block_ix)?;
                if placed.top > bottom {
                    break;
                }
                let block = &placed.layout;
                let window = placed.row_at(top)..placed.row_at(bottom) + 1;
                for row_ix in window {
                    let row = &block.rows[row_ix];
                    let row_top = placed.row_top(row_ix);
                    if row_top + slack >= top && row_top + block.line_height <= bottom + slack {
                        rows.push(VisualRow {
                            start: Position::new(block_ix, row.range.start),
                            end: Position::new(block_ix, row.range.end),
                        });
                    }
                }
            }
            if rows.is_empty() {
                let placed = lay.placed(first_block)?;
                let row = &placed.layout.rows[placed.row_at(top)];
                rows.push(VisualRow {
                    start: Position::new(first_block, row.range.start),
                    end: Position::new(first_block, row.range.end),
                });
            }
            Some(rows)
        })
        .flatten()
    }
}

/// Moves `position` by `rows` visual rows towards `goal_x` (or the x of
/// the start).
///
/// `affinity` is `None` for a plain position: it is on the row that starts
/// at it, and the result is never the end of a soft-wrapped row. For a
/// caret with an affinity it is `Some(upstream)`: the move starts on the
/// row the caret is drawn on, and it may end at the end of a soft-wrapped
/// row, which the returned flag says (the caret is upstream there).
fn move_rows(
    lay: &mut Lay,
    document: &Document,
    position: Position,
    rows: isize,
    goal_x: Option<Pixels>,
    affinity: Option<bool>,
) -> Option<(RowMove, bool)> {
    let position = document.clamp_position(position);
    let upstream = affinity.unwrap_or(false);
    let count = lay.count();
    let start = lay.placed(position.block)?.layout;
    let mut block_ix = position.block;
    let mut block = start.clone();
    let mut row_ix = start.row_for_caret(position.offset, upstream);
    let goal_x =
        goal_x.unwrap_or_else(|| start.caret_bounds_on(position.offset, upstream).1.origin.x);
    let mut moved: isize = 0;
    while moved != rows {
        if rows > 0 {
            // Whole rows left in this block first, then on.
            let here = (block.rows.len() - 1 - row_ix) as isize;
            let step = here.min(rows - moved);
            if step > 0 {
                row_ix += step as usize;
                moved += step;
            } else if block_ix + 1 < count {
                block_ix += 1;
                block = lay.placed(block_ix)?.layout;
                row_ix = 0;
                moved += 1;
            } else {
                break;
            }
        } else {
            let step = (row_ix as isize).min(moved - rows);
            if step > 0 {
                row_ix -= step as usize;
                moved -= step;
            } else if block_ix > 0 {
                block_ix -= 1;
                block = lay.placed(block_ix)?.layout;
                row_ix = block.rows.len() - 1;
                moved -= 1;
            } else {
                break;
            }
        }
    }
    let x = goal_x - block.text_left;
    let (offset, upstream) = match affinity {
        Some(_) => block.caret_for_x(row_ix, x),
        None => (block.offset_for_x(row_ix, x), false),
    };
    Some((
        RowMove {
            position: document.clamp_position(Position::new(block_ix, offset)),
            goal_x,
            rows_moved: moved,
        },
        upstream,
    ))
}

/// The visual row a caret is drawn on (see [`Editor::caret_row`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CaretRow {
    pub(crate) row: VisualRow,
    /// The row ends at a soft wrap: its end is also the next row's start.
    pub(crate) soft_wrapped: bool,
}

impl Editor {
    /// The visual row the selection head is drawn on: the row its affinity
    /// puts it on when it sits at a soft wrap.
    pub(crate) fn caret_row(&self) -> Option<CaretRow> {
        let head = self.state.selection().head;
        let upstream = self.caret_is_upstream();
        self.with_layout(|lay| {
            let block = lay.placed(head.block)?.layout;
            let row = &block.rows[block.row_for_caret(head.offset, upstream)];
            Some(CaretRow {
                row: VisualRow {
                    start: Position::new(head.block, row.range.start),
                    end: Position::new(head.block, row.range.end),
                },
                soft_wrapped: row.soft_wrapped,
            })
        })
        .flatten()
    }

    /// [`Editor::move_by_visual_rows`] for the editor's own caret: starts
    /// on the row the caret is drawn on (`upstream` is its affinity at
    /// `position`) and, when the caret has an affinity at all, may end at
    /// the end of a soft-wrapped row. Returns the move and the affinity
    /// the caret has at its target.
    pub(crate) fn move_caret_by_rows(
        &self,
        position: Position,
        upstream: bool,
        rows: isize,
        goal_x: Option<Pixels>,
    ) -> Option<(RowMove, bool)> {
        let document = self.state.document();
        let affinity = self.caret_has_affinity().then_some(upstream);
        self.with_layout(|lay| move_rows(lay, document, position, rows, goal_x, affinity))
            .flatten()
    }

    /// The oracle over the editor's own state and layout.
    fn rows(&self) -> RowOracle<'_> {
        RowOracle::new(&self.layout, &self.state)
    }

    /// The visual row containing `position`.
    pub fn visual_row(&self, position: Position) -> Option<VisualRow> {
        self.rows().visual_row(&self.state, position)
    }

    /// The x of `position` from the left edge of the text column: the goal
    /// x a vertical move starting there would use.
    pub fn column_x(&self, position: Position) -> Option<Pixels> {
        self.rows().column_x(&self.state, position)
    }

    /// Moves `position` by `rows` visual rows (negative: up), landing on the
    /// character boundary closest to `goal_x` (or to the x of `position`
    /// when `goal_x` is `None`). Stops at the first or last row of the
    /// document; [`RowMove::rows_moved`] tells how far it got. Does not
    /// touch the selection: apply the result through [`Editor::edit`].
    ///
    /// ```ignore
    /// // Vim `3j`, keeping the column across calls:
    /// let head = editor.selection().head;
    /// if let Some(step) = editor.move_by_visual_rows(head, 3, self.goal_x) {
    ///     self.goal_x = Some(step.goal_x);
    ///     editor.edit(|state| state.set_caret(step.position), cx);
    /// }
    /// ```
    pub fn move_by_visual_rows(
        &self,
        position: Position,
        rows: isize,
        goal_x: Option<Pixels>,
    ) -> Option<RowMove> {
        self.rows()
            .move_by_visual_rows(&self.state, position, rows, goal_x)
    }

    /// How many rows of body text fit in the viewport (at least 1): the
    /// size of a page for page-wise movement and scrolling.
    pub fn rows_per_page(&self) -> Option<usize> {
        self.rows().rows_per_page()
    }

    /// The rows that are completely inside the viewport, top to bottom
    /// (Vim `H`, `M`, `L` and their counts index into this). Never empty:
    /// when no row fits completely, it holds the row at the top edge.
    pub fn visible_rows(&self) -> Option<Vec<VisualRow>> {
        self.rows().visible_rows(&self.state)
    }

    /// The first row that is completely inside the viewport (Vim `H`): the
    /// first of [`Editor::visible_rows`].
    pub fn first_visible_row(&self) -> Option<VisualRow> {
        self.visible_rows()?.first().copied()
    }

    /// The last row that is completely inside the viewport (Vim `L`): the
    /// last of [`Editor::visible_rows`].
    pub fn last_visible_row(&self) -> Option<VisualRow> {
        self.visible_rows()?.last().copied()
    }
}
