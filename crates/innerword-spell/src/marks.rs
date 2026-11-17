//! The editor's side of spelling: every call into the seam the editor
//! offers for spelling marks (`innerword_editor::spelling`) is made here, so
//! a change of that seam is a change of this file.
//!
//! The editor keeps the marks per block and block revision, draws them,
//! moves them with edits and knows which blocks have a result for their
//! current text. That store is also the checker's cache: a block that is
//! "checked" there is never asked about again until it changes or
//! everything is invalidated (another language, a word unlearned).

use std::ops::Range;

use gpui::{App, Entity};
use innerword_core::{BlockId, Position, TextRange};
use innerword_editor::Editor;

/// Stores the result for one block at one revision. Returns whether the
/// editor took it (not when the block has changed or gone meanwhile).
pub(crate) fn set(
    editor: &Entity<Editor>,
    block: BlockId,
    revision: u64,
    ranges: Vec<Range<usize>>,
    cx: &mut App,
) -> bool {
    editor.update(cx, |editor, cx| {
        editor
            .set_spelling_marks(block, revision, ranges, cx)
            .is_applied()
    })
}

/// Removes every mark and every result.
pub(crate) fn clear(editor: &Entity<Editor>, cx: &mut App) {
    editor.update(cx, |editor, cx| editor.clear_spelling_marks(cx));
}

/// Makes every block unchecked and leaves the marks where they are until
/// the new results replace them.
pub(crate) fn invalidate(editor: &Entity<Editor>, cx: &mut App) {
    editor.update(cx, |editor, _| editor.invalidate_spelling_marks());
}

/// Whether the block at `index` has a result for its current text.
pub(crate) fn is_checked(editor: &Editor, index: usize) -> bool {
    editor.spelling_checked(index)
}

/// The first block in `blocks` without a result for its current text.
pub(crate) fn first_unchecked(editor: &Editor, blocks: Range<usize>) -> Option<usize> {
    editor.first_unchecked_spelling_block(blocks)
}

/// The marks of the block at `index`, in order.
pub(crate) fn of_block(editor: &Editor, index: usize) -> &[Range<usize>] {
    editor.spelling_marks(index)
}

/// The mark `position` is in or at an end of.
pub(crate) fn at(editor: &Editor, position: Position) -> Option<TextRange> {
    editor.spelling_mark_at(position)
}

/// How many words are marked in the document.
pub(crate) fn count(editor: &Editor) -> usize {
    editor.spelling_mark_count()
}
