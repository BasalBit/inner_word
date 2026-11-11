//! The engine's host: wrapped rows, the viewport, scrolling and the system
//! clipboard, answered from an [`Editor`].
//!
//! # Why there are two ways to run the engine
//!
//! The engine works on `&mut EditorState` and asks its layout questions
//! through [`VimHost`] during the same call. The editor hands out
//! `&mut EditorState` only inside [`Editor::edit_with`], where the editor
//! itself is mutably borrowed, and its row oracle (`visual_row`, `column_x`,
//! `move_by_visual_rows`, `visible_rows`) needs `&Editor`. Both at once is
//! not possible with the editor's API as it is (a request for a seam is
//! filed, see the crate docs). Until then:
//!
//! - **Direct**: the engine runs inside `Editor::edit_with` on the editor's
//!   own state, and the host has no layout. Used for everything that asks
//!   no layout question: Insert, Replace and command-line keys, and most
//!   Normal-mode commands.
//! - **With layout**: the engine runs on a *copy* of the editor state while
//!   the host answers from `&Editor`, and the copy is then moved into the
//!   editor through `Editor::edit_with`. Used for the keys that move by
//!   visual rows or look at the viewport ([`may_ask_layout`]).
//!
//! A layout question that arrives without a layout (a key that was not
//! expected to ask, or an editor that has not been drawn yet) is answered
//! from hard lines, one row per line, and counted in
//! [`EditorHost::fallbacks`].
//!
//! Scroll requests and clipboard writes are collected and carried out by
//! the caller after the engine returns, as the host trait allows.

use std::ops::Range;

use gpui::{App, ClipboardItem, px};
use writer_core::{Document, EditorState, Fragment, Position, text};
use writer_editor::Editor;
use writer_vim::{GoalColumn, Key, KeyCode, Mode, RegisterContent, Scroll, Viewport, Vim, VimHost};

/// A goal x far to the right of any row: the end of the row.
const END_OF_ROW_X: f32 = 1.0e7;
/// Width of one character in the hard-line fallback, in the same unit as
/// the editor's column x (pixels), so a goal column survives a switch
/// between the two.
const FALLBACK_CELL: f32 = 8.0;

/// The host for one call into the engine.
pub(crate) struct EditorHost<'a> {
    /// The editor whose layout answers, and the document revision that
    /// layout is for.
    layout: Option<(&'a Editor, u64)>,
    clipboard: Option<RegisterContent>,
    /// The last clipboard write of the call.
    pub(crate) clipboard_write: Option<RegisterContent>,
    /// The scroll requests of the call, in order.
    pub(crate) scrolls: Vec<Scroll>,
    /// Layout questions answered without the editor's layout.
    pub(crate) fallbacks: usize,
}

impl<'a> EditorHost<'a> {
    /// A host without layout. `clipboard` is what the system clipboard
    /// holds, read before the call.
    pub(crate) fn new(clipboard: Option<RegisterContent>) -> Self {
        Self {
            layout: None,
            clipboard,
            clipboard_write: None,
            scrolls: Vec::new(),
            fallbacks: 0,
        }
    }

    /// Lets the host answer layout questions from `editor`, as long as the
    /// state it is asked about still has the editor's document.
    pub(crate) fn with_layout(mut self, editor: &'a Editor) -> Self {
        self.layout = Some((editor, editor.state().revision()));
        self
    }

    /// The editor, when its layout describes `state`.
    fn editor_for(&self, state: &EditorState) -> Option<&'a Editor> {
        self.layout
            .filter(|(_, revision)| *revision == state.revision())
            .map(|(editor, _)| editor)
    }
}

impl VimHost for EditorHost<'_> {
    fn visual_row(&mut self, state: &EditorState, pos: Position) -> Range<usize> {
        if let Some(editor) = self.editor_for(state)
            && let Some(row) = editor.visual_row(pos)
        {
            return row.start.offset..row.end.offset;
        }
        self.fallbacks += 1;
        hard_line(state.document(), pos)
    }

    fn column_of(&mut self, state: &EditorState, pos: Position) -> f32 {
        if let Some(editor) = self.editor_for(state)
            && let Some(x) = editor.column_x(pos)
        {
            return f32::from(x);
        }
        self.fallbacks += 1;
        let document = state.document();
        let line = hard_line(document, pos);
        let text = document.blocks()[pos.block.min(document.block_count() - 1)].text();
        let upto = pos.offset.clamp(line.start, line.end);
        let before = text.get(line.start..upto).unwrap_or_default();
        text::grapheme_count(before) as f32 * FALLBACK_CELL
    }

    fn move_by_rows(
        &mut self,
        state: &EditorState,
        from: Position,
        rows: isize,
        goal: GoalColumn,
    ) -> Position {
        if let Some(editor) = self.editor_for(state) {
            let goal_x = match goal {
                GoalColumn::X(x) => x.max(0.),
                GoalColumn::EndOfRow => END_OF_ROW_X,
            };
            if let Some(step) = editor.move_by_visual_rows(from, rows, Some(px(goal_x))) {
                return character_on_row(editor, state.document(), step.position);
            }
        }
        self.fallbacks += 1;
        move_by_hard_lines(state.document(), from, rows, goal)
    }

    fn viewport(&mut self, state: &EditorState) -> Option<Viewport> {
        let editor = self.editor_for(state)?;
        let rows = editor.visible_rows()?;
        let document = state.document();
        // The editor keeps a row of context around the caret whenever it
        // scrolls for a movement. The engine gets the viewport without its
        // first and last row (Vim's `scrolloff=1`), so `H`, `L` and the
        // caret the scroll commands drag along stay inside that margin and
        // the next movement does not shift the view. The ends of the
        // document have no context to keep, but the page size stays the
        // same there: `ctrl-f` then `ctrl-b` must come back to where it
        // started.
        let mut inner = rows.as_slice();
        let mut page = rows.len();
        if rows.len() >= 3 {
            page = rows.len() - 2;
            if inner[0].start != Position::ZERO {
                inner = &inner[1..];
            }
            let last = inner[inner.len() - 1].end;
            let document_end = last.block + 1 == document.block_count()
                && last.offset >= document.blocks()[last.block].len();
            if !document_end {
                inner = &inner[..inner.len() - 1];
            }
        }
        Some(Viewport {
            top: inner.first()?.start,
            bottom: inner.last()?.start,
            rows: page.max(1),
        })
    }

    fn scroll(&mut self, _: &EditorState, scroll: Scroll) {
        self.scrolls.push(scroll);
    }

    fn clipboard_read(&mut self) -> Option<RegisterContent> {
        self.clipboard.clone()
    }

    fn clipboard_write(&mut self, content: &RegisterContent) {
        // Later reads in the same call see what was just written.
        self.clipboard = Some(content.clone());
        self.clipboard_write = Some(content.clone());
    }
}

/// The editor's vertical movement lands on a caret position, which at the
/// end of a hard line is the offset after its last character. The engine
/// wants the character itself.
fn character_on_row(editor: &Editor, document: &Document, position: Position) -> Position {
    let Some(row) = editor.visual_row(position) else {
        return position;
    };
    if row.start == row.end || position.offset < row.end.offset {
        return position;
    }
    let text = document.blocks()[position.block].text();
    let last = text::prev_grapheme_boundary(text, row.end.offset).max(row.start.offset);
    Position::new(position.block, last)
}

// ---------------------------------------------------------------------------
// Without a layout: one row per hard line
// ---------------------------------------------------------------------------

/// The hard line holding `pos`, as a byte range of its block.
fn hard_line(document: &Document, pos: Position) -> Range<usize> {
    let pos = document.clamp_position(pos);
    document.blocks()[pos.block].line_range(pos.offset)
}

fn move_by_hard_lines(
    document: &Document,
    from: Position,
    rows: isize,
    goal: GoalColumn,
) -> Position {
    let from = document.clamp_position(from);
    let blocks = document.blocks();
    let mut block = from.block;
    let mut line = blocks[block].line_range(from.offset);
    for _ in 0..rows.unsigned_abs() {
        if rows > 0 {
            if line.end < blocks[block].len() {
                line = blocks[block].line_range(line.end + 1);
            } else if block + 1 < blocks.len() {
                block += 1;
                line = blocks[block].line_range(0);
            } else {
                break;
            }
        } else if line.start > 0 {
            line = blocks[block].line_range(line.start - 1);
        } else if block > 0 {
            block -= 1;
            line = blocks[block].line_range(blocks[block].len());
        } else {
            break;
        }
    }
    let text = blocks[block].text();
    if line.is_empty() {
        return Position::new(block, line.start);
    }
    let last = text::prev_grapheme_boundary(text, line.end).max(line.start);
    let cells = match goal {
        GoalColumn::X(x) => (x.max(0.) / FALLBACK_CELL).round() as usize,
        GoalColumn::EndOfRow => usize::MAX,
    };
    let mut offset = line.start;
    for _ in 0..cells {
        if offset >= last {
            break;
        }
        offset = text::next_grapheme_boundary(text, offset);
    }
    Position::new(block, offset.min(last))
}

// ---------------------------------------------------------------------------
// Which keys ask layout questions
// ---------------------------------------------------------------------------

/// Whether `key` can make the engine ask about visual rows or the viewport
/// in its current state, so the call has to run with a layout.
///
/// The engine asks for `j` / `k` and the arrow keys (as movements; after an
/// operator they count blocks, which is harmless to over-approximate), the
/// `g` commands (`gj`, `gk`, `g0`, `g^`, `g$`), `H` / `M` / `L`, the scroll
/// commands and `.` (which may repeat any of them with an operator). It
/// never asks in Insert, Replace or command-line mode.
pub(crate) fn may_ask_layout(vim: &Vim, key: &Key) -> bool {
    if matches!(vim.mode(), Mode::Insert | Mode::Replace | Mode::CommandLine) {
        return false;
    }
    if key.alt {
        return false;
    }
    // The key after `g` decides; `z` only scrolls.
    if vim.pending_keys().ends_with('g') {
        return true;
    }
    match key.code {
        KeyCode::Up | KeyCode::Down | KeyCode::PageUp | KeyCode::PageDown => true,
        KeyCode::Char(c) if key.ctrl => {
            matches!(c, 'd' | 'u' | 'f' | 'b' | 'e' | 'y' | 'n' | 'p' | 'j')
        }
        KeyCode::Char(c) => matches!(c, 'j' | 'k' | 'H' | 'M' | 'L' | '.'),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// The system clipboard
// ---------------------------------------------------------------------------

/// What the system clipboard holds, as a register.
///
/// The editor's own copy puts the fragment's JSON next to the plain text
/// (GPUI clipboard metadata), and so does [`write_clipboard`]: formatting
/// survives between `cmd-c` / `cmd-v` and the `+` register. Text from other
/// applications follows the Vim convention: a final line break makes it
/// linewise.
pub(crate) fn read_clipboard(cx: &App) -> Option<RegisterContent> {
    let item = cx.read_from_clipboard()?;
    let text = item.text()?;
    if text.is_empty() {
        return None;
    }
    let rich = item
        .metadata()
        .and_then(|json| Fragment::from_json(json).ok())
        .filter(|fragment| !fragment.blocks().is_empty());
    let Some(fragment) = rich else {
        return Some(RegisterContent::from_plain_text(&text));
    };
    let linewise = text.ends_with('\n');
    let mut blocks = fragment.blocks().to_vec();
    // The editor's copy of whole lines ends with an empty block that stands
    // for the final line break; a linewise register written by the engine
    // has the break in the plain text only.
    let break_in_fragment = linewise && fragment.to_plain_text() == text;
    if break_in_fragment
        && blocks.len() > 1
        && blocks
            .last()
            .is_some_and(|block| block.is_empty() && !block.is_void())
    {
        blocks.pop();
    }
    Some(RegisterContent {
        fragment: Fragment::new(blocks),
        linewise,
    })
}

/// Writes a register to the system clipboard: plain text for other
/// applications, the fragment for pasting back with formatting.
pub(crate) fn write_clipboard(content: &RegisterContent, cx: &App) {
    cx.write_to_clipboard(ClipboardItem::new_string_with_metadata(
        content.to_plain_text(),
        content.fragment.to_json(),
    ));
}
