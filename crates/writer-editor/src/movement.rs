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
//! the same handler and for positions that are scrolled out of view.
//!
//! A visual row is one wrapped line of one block. A divider or page break is
//! one row with a single position. A position at a soft wrap belongs to the
//! row that starts there (there is no caret affinity), so
//! [`VisualRow::end`] of a soft-wrapped row is also the start of the next
//! row; vertical movement and clicks never land on such an end, they stop
//! one character before it.

use gpui::Pixels;
use writer_core::Position;

use crate::editor::Editor;

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

impl Editor {
    /// The visual row containing `position`.
    pub fn visual_row(&self, position: Position) -> Option<VisualRow> {
        let position = self.state.document().clamp_position(position);
        self.with_layout(|layout, _| {
            let block = layout.block(position.block)?;
            let row = &block.rows[block.row_for_offset(position.offset)];
            Some(VisualRow {
                start: Position::new(position.block, row.range.start),
                end: Position::new(position.block, row.range.end),
            })
        })
        .flatten()
    }

    /// The x of `position` from the left edge of the text column: the goal
    /// x a vertical move starting there would use.
    pub fn column_x(&self, position: Position) -> Option<Pixels> {
        let position = self.state.document().clamp_position(position);
        self.with_layout(|layout, _| {
            let block = layout.block(position.block)?;
            Some(block.caret_bounds(position.offset).origin.x)
        })
        .flatten()
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
        let document = self.state.document();
        let position = document.clamp_position(position);
        self.with_layout(|layout, _| {
            let start = layout.block(position.block)?;
            let mut block_ix = position.block;
            let mut row_ix = start.row_for_offset(position.offset);
            let goal_x = goal_x.unwrap_or_else(|| start.caret_bounds(position.offset).origin.x);
            let mut moved = 0;
            while moved != rows {
                let block = layout.block(block_ix)?;
                if rows > 0 {
                    if row_ix + 1 < block.rows.len() {
                        row_ix += 1;
                    } else if block_ix + 1 < layout.block_count() {
                        block_ix += 1;
                        row_ix = 0;
                    } else {
                        break;
                    }
                    moved += 1;
                } else {
                    if row_ix > 0 {
                        row_ix -= 1;
                    } else if block_ix > 0 {
                        block_ix -= 1;
                        row_ix = layout.block(block_ix)?.rows.len() - 1;
                    } else {
                        break;
                    }
                    moved -= 1;
                }
            }
            let block = layout.block(block_ix)?;
            let offset = block.offset_for_x(row_ix, goal_x - block.text_left);
            Some(RowMove {
                position: document.clamp_position(Position::new(block_ix, offset)),
                goal_x,
                rows_moved: moved,
            })
        })
        .flatten()
    }

    /// How many rows of body text fit in the viewport (at least 1): the
    /// size of a page for page-wise movement and scrolling.
    pub fn rows_per_page(&self) -> Option<usize> {
        let layout = self.layout.borrow();
        let env = layout.env()?;
        let rows = (env.viewport.size.height / env.style.base_line_height()).floor() as usize;
        Some(rows.max(1))
    }

    /// The rows that are completely inside the viewport, top to bottom
    /// (Vim `H`, `M`, `L` and their counts index into this). Never empty:
    /// when no row fits completely, it holds the row at the top edge.
    pub fn visible_rows(&self) -> Option<Vec<VisualRow>> {
        // Half a pixel of tolerance: a row scrolled exactly to an edge
        // counts as visible despite float rounding.
        let slack = gpui::px(0.5);
        self.with_layout(|layout, env| {
            let top = self.scroll_y;
            let bottom = top + env.viewport.size.height;
            let first_block = layout.block_at_y(top);
            let mut rows = Vec::new();
            for block_ix in first_block..layout.block_count() {
                let block_top = layout.block_top(block_ix);
                if block_top > bottom {
                    break;
                }
                let block = layout.block(block_ix)?;
                for row in &block.rows {
                    let row_top = block_top + row.y;
                    if row_top + slack >= top && row_top + block.line_height <= bottom + slack {
                        rows.push(VisualRow {
                            start: Position::new(block_ix, row.range.start),
                            end: Position::new(block_ix, row.range.end),
                        });
                    }
                }
            }
            if rows.is_empty() {
                let block = layout.block(first_block)?;
                let row = &block.rows[block.row_at_y(top - layout.block_top(first_block))];
                rows.push(VisualRow {
                    start: Position::new(first_block, row.range.start),
                    end: Position::new(first_block, row.range.end),
                });
            }
            Some(rows)
        })
        .flatten()
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
