//! The engine's host: wrapped rows, the viewport, scrolling and the system
//! clipboard, answered from an [`Editor`](writer_editor::Editor).
//!
//! The engine works on `&mut EditorState` and asks its layout questions
//! through [`VimHost`] during the same call. Every call into the engine
//! runs inside `Editor::edit_with_rows`, which lends the editor's state
//! together with a [`RowOracle`]: the oracle answers for that state as it
//! is when asked, also after the engine has edited it. Nothing is copied.
//!
//! A layout question that cannot be answered from the layout (the editor
//! has not been drawn yet) is answered from hard lines, one row per line,
//! and counted in [`EditorHost::fallbacks`].
//!
//! Scroll requests, clipboard writes and spelling commands are collected
//! and carried out by the caller after the engine returns, as the host
//! trait allows. The spell checker cannot be asked during the call either
//! (it works on the editor, which is lent out): its answers are fetched
//! before the call, see the `spell` module.

use std::ops::Range;

use gpui::{App, ClipboardItem, px};
use writer_core::{Document, EditorState, Fragment, Position, TextRange, text};
use writer_editor::RowOracle;
use writer_vim::{GoalColumn, RegisterContent, Scroll, SpellCommand, SpellOff, Viewport, VimHost};

use crate::spell::{SpellAnswers, SpellHost};

/// A goal x far to the right of any row: the end of the row.
const END_OF_ROW_X: f32 = 1.0e7;
/// Width of one character in the hard-line fallback, in the same unit as
/// the editor's column x (pixels), so a goal column survives a switch
/// between the two.
const FALLBACK_CELL: f32 = 8.0;

/// The host for one call into the engine.
pub(crate) struct EditorHost<'a> {
    /// The editor's row oracle for the state the engine is working on.
    rows: Option<&'a RowOracle<'a>>,
    clipboard: Option<RegisterContent>,
    /// The last clipboard write of the call.
    pub(crate) clipboard_write: Option<RegisterContent>,
    /// The scroll requests of the call, in order.
    pub(crate) scrolls: Vec<Scroll>,
    /// Layout questions answered without the editor's layout.
    pub(crate) fallbacks: usize,
    /// The spell checker's prepared answers and the spelling commands of
    /// the call (see the `spell` module).
    pub(crate) spell: SpellHost,
}

impl<'a> EditorHost<'a> {
    /// A host without layout (for calls that never reach the editor
    /// state). `clipboard` is what the system clipboard holds, read before
    /// the call.
    pub(crate) fn new(clipboard: Option<RegisterContent>) -> Self {
        Self {
            rows: None,
            clipboard,
            clipboard_write: None,
            scrolls: Vec::new(),
            fallbacks: 0,
            spell: SpellHost::default(),
        }
    }

    /// What the spell checker answers during this call; `None` while spell
    /// checking is off.
    pub(crate) fn with_spelling(mut self, answers: Option<SpellAnswers>) -> Self {
        self.spell.answers = answers;
        self
    }

    /// Lets the host answer layout questions from the editor's layout.
    pub(crate) fn with_rows(mut self, rows: &'a RowOracle<'a>) -> Self {
        self.rows = Some(rows);
        self
    }
}

impl VimHost for EditorHost<'_> {
    fn visual_row(&mut self, state: &EditorState, pos: Position) -> Range<usize> {
        if let Some(row) = self.rows.and_then(|rows| rows.visual_row(state, pos)) {
            return row.start.offset..row.end.offset;
        }
        self.fallbacks += 1;
        hard_line(state.document(), pos)
    }

    fn column_of(&mut self, state: &EditorState, pos: Position) -> f32 {
        if let Some(x) = self.rows.and_then(|rows| rows.column_x(state, pos)) {
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
        if let Some(oracle) = self.rows {
            let goal_x = match goal {
                GoalColumn::X(x) => x.max(0.),
                GoalColumn::EndOfRow => END_OF_ROW_X,
            };
            if let Some(step) = oracle.move_by_visual_rows(state, from, rows, Some(px(goal_x))) {
                return character_on_row(oracle, state, step.position);
            }
        }
        self.fallbacks += 1;
        move_by_hard_lines(state.document(), from, rows, goal)
    }

    fn viewport(&mut self, state: &EditorState) -> Option<Viewport> {
        // The rows that are completely on screen, as they are. The row of
        // context the editor keeps around the caret when it scrolls is the
        // engine's `scroll_off` option (see `VimMode::sync_options`), so
        // `H`, `L` and the caret that `ctrl-e` drags along stay inside
        // that margin while a page is counted in real rows.
        let rows = self.rows?.visible_rows(state)?;
        Some(Viewport {
            top: rows.first()?.start,
            bottom: rows.last()?.start,
            rows: rows.len().max(1),
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

    fn misspelling_from(
        &mut self,
        _: &EditorState,
        from: Position,
        forward: bool,
    ) -> Result<Option<TextRange>, SpellOff> {
        self.spell.misspelling_from(from, forward)
    }

    fn spell_command(&mut self, _: &EditorState, command: SpellCommand) -> Result<(), SpellOff> {
        self.spell.command(command)
    }
}

/// The editor's vertical movement lands on a caret position, which at the
/// end of a hard line is the offset after its last character. The engine
/// wants the character itself.
fn character_on_row(rows: &RowOracle<'_>, state: &EditorState, position: Position) -> Position {
    let Some(row) = rows.visual_row(state, position) else {
        return position;
    };
    if row.start == row.end || position.offset < row.end.offset {
        return position;
    }
    let text = state.document().blocks()[position.block].text();
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
