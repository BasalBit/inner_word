//! What the engine needs from the outside world.

use std::ops::Range;

use writer_core::{EditorState, Position};

use crate::register::RegisterContent;

/// The horizontal target of a vertical movement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GoalColumn {
    /// A horizontal position in the host's own unit, as returned by
    /// [`VimHost::column_of`]. The engine never interprets the number; it
    /// only stores it between `j` / `k` presses and hands it back.
    X(f32),
    /// The end of the row: set by `$` (and `g$`), so that the following
    /// vertical movements land on the last character of each row.
    EndOfRow,
}

/// The visual rows currently on screen. Every position is the start of a
/// row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Viewport {
    /// Start of the first fully visible row.
    pub top: Position,
    /// Start of the last fully visible row.
    pub bottom: Position,
    /// Number of fully visible rows (at least 1).
    pub rows: usize,
}

/// Where a scroll command puts the row of the caret.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ScrollAlign {
    /// At the top of the viewport (`zt`).
    Top,
    /// In the middle of the viewport (`zz`).
    Center,
    /// At the bottom of the viewport (`zb`).
    Bottom,
}

/// A scroll request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Scroll {
    /// Scroll the view by this many visual rows: positive moves the view
    /// towards the end of the document (the text moves up). The engine
    /// moves the caret itself.
    Rows(isize),
    /// Scroll so that the visual row of `caret` sits at `align`.
    Align {
        /// The caret whose row is aligned.
        caret: Position,
        /// Where the row goes.
        align: ScrollAlign,
    },
}

/// Everything only the user interface knows: wrapped rows, the viewport and
/// the system clipboard.
///
/// Every layout method receives the editor state as it is at the time of
/// the call, so an implementation does not need to hold a reference to it:
/// it can bring its layout cache up to date from
/// [`EditorState::document`] and [`EditorState::revision`] and answer. The
/// engine asks its layout questions before it edits anything in the same
/// call, but an implementation should not rely on that.
///
/// **Positions name characters.** In Normal and Visual mode the caret sits
/// *on* a character, so a [`Position`] passed to or returned from these
/// methods means "the character that starts at this offset". An offset at a
/// soft wrap therefore belongs to the row that starts there, never to the
/// end of the row before. The end of a hard line (the offset of a `\n` or
/// the end of the block) belongs to the last row of that line; an empty line
/// and a void block (divider, page break) have one row holding one position.
pub trait VimHost {
    /// The visual row holding the character at `pos`, as a byte range of
    /// the block's text: from the first character on the row to the end of
    /// its last character (excluding a trailing `\n`). Empty for an empty
    /// line and for a void block.
    fn visual_row(&mut self, state: &EditorState, pos: Position) -> Range<usize>;

    /// The horizontal position of the character at `pos`, in any unit the
    /// host likes (pixels, cells). Used as the goal column of `j` and `k`.
    fn column_of(&mut self, state: &EditorState, pos: Position) -> f32;

    /// The character `rows` visual rows below (negative: above) the row
    /// holding `from`, at horizontal position `goal`.
    ///
    /// Rows continue across blocks. The result is clamped to the first and
    /// last row of the document; `rows` may be 0 (the character at `goal`
    /// on the row of `from`). It must be a character on the target row:
    /// on a row shorter than the goal return its last character (not the
    /// offset after it, which on a wrapped row is the next row); for
    /// [`GoalColumn::EndOfRow`] return the row's last character. On an
    /// empty row return its only position.
    fn move_by_rows(
        &mut self,
        state: &EditorState,
        from: Position,
        rows: isize,
        goal: GoalColumn,
    ) -> Position;

    /// The rows on screen, or `None` when nothing is laid out yet. Used by
    /// `H`, `M`, `L` and the scroll commands.
    fn viewport(&mut self, state: &EditorState) -> Option<Viewport>;

    /// Scrolls the view. Called for `ctrl-e`, `ctrl-y`, `ctrl-d`, `ctrl-u`,
    /// `ctrl-f`, `ctrl-b`, `zz`, `zt` and `zb`. The engine sets the caret
    /// through the editor state as usual; the host should not let its own
    /// "keep the caret in view" logic undo an explicit scroll. The scroll
    /// may be applied after the engine returns: the engine does not ask for
    /// the viewport again in the same call.
    fn scroll(&mut self, state: &EditorState, scroll: Scroll);

    /// Reads the system clipboard for the `+` and `*` registers. Return
    /// `None` when it is empty or holds no text. A host that keeps rich
    /// fragments on the pasteboard returns them here; plain text goes
    /// through [`RegisterContent::from_plain_text`].
    fn clipboard_read(&mut self) -> Option<RegisterContent>;

    /// Writes the `+` / `*` register to the system clipboard. Use
    /// [`RegisterContent::to_plain_text`] for the plain-text flavour.
    fn clipboard_write(&mut self, content: &RegisterContent);
}
