//! What the engine needs from the outside world.

use std::ops::Range;

use writer_core::{EditorState, Position, TextRange};

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

/// The host has no spell checker, or spell checking is switched off. The
/// engine reports `E756: Spell checking is off` and the command fails.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SpellOff;

/// A spelling command that the host carries out (`z=`, `zg`, `zug`). The
/// engine only knows where the Vim cursor is; the word list, the
/// suggestions and the menu are the host's.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum SpellCommand {
    /// `z=`: offer the suggestions for the misspelled word at `at` (a menu,
    /// a list). With a count (`1z=`, `3z=`) `pick` is that count: replace
    /// the word with the suggestion of that number (1 is the first)
    /// without asking. The replacement is the host's edit: one undo step,
    /// the caret at the start of the word.
    Suggest {
        /// The Vim cursor (in Visual mode the cursor end of the selection;
        /// Visual mode has ended by the time the host is asked).
        at: Position,
        /// The number of the suggestion to take, from 1.
        pick: Option<usize>,
    },
    /// `zg`: the word at `at` is spelled correctly; remember it.
    MarkGood {
        /// The Vim cursor.
        at: Position,
        /// The word under the cursor as the engine sees it (letters,
        /// digits and, with
        /// [`VimOptions::apostrophe_in_words`](crate::VimOptions::apostrophe_in_words),
        /// apostrophes inside it); the first one after the cursor when it
        /// is on a blank. A host with its own idea of the misspelled word
        /// at `at` may prefer that.
        word: String,
    },
    /// `zug`: undo `zg` for the word at `at`.
    UndoMarkGood {
        /// The Vim cursor.
        at: Position,
        /// The word under the cursor, as for [`SpellCommand::MarkGood`].
        word: String,
    },
}

/// Where `]s` or `[s` with a count ends: see [`walk_misspellings`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SpellJump {
    /// The misspelled word reached.
    pub range: TextRange,
    /// Whether the way there passed the end (or, backwards, the start) of
    /// the document.
    pub wrapped: bool,
}

/// Walks `count` misspelled words from `from`, the way the engine does for
/// `]s` and `[s`: `next(p)` is asked for the misspelled word after (before)
/// `p`, first with `from` and then with the start of each answer. A walk
/// that comes back to the first word it found has seen every misspelled
/// word, so a count larger than their number costs one round and no more.
/// `None` when `next` has no answer.
///
/// The questions depend only on `from`, `count` and the answers. A host
/// that cannot answer [`VimHost::misspelling_from`] while the engine runs
/// (its spell checker lives behind a lock the engine call holds) can
/// therefore run the same walk before the call, with
/// [`Vim::spell_lookahead`](crate::Vim::spell_lookahead) telling it when,
/// record the questions and answers, and replay them during the call.
pub fn walk_misspellings(
    from: Position,
    forward: bool,
    count: usize,
    mut next: impl FnMut(Position) -> Option<TextRange>,
) -> Option<SpellJump> {
    let count = count.max(1);
    let mut found: Vec<TextRange> = Vec::new();
    let mut wrapped = false;
    let mut p = from;
    for step in 0..count {
        let range = next(p)?;
        wrapped |= if forward {
            range.start <= p
        } else {
            range.start >= p
        };
        if step > 0 && range.start == found[0].start {
            // Once around: the rest of the count is a position in the round.
            return Some(SpellJump {
                range: found[(count - 1) % found.len()],
                wrapped: true,
            });
        }
        found.push(range);
        p = range.start;
    }
    found.last().map(|&range| SpellJump { range, wrapped })
}

/// Everything only the user interface knows: wrapped rows, the viewport,
/// the system clipboard and the spell checker.
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

    /// The misspelled word that `]s` (`forward`) or `[s` goes to from
    /// `from`: forward, the first one that starts after `from`; backward,
    /// the last one that starts before `from` (which is the word the cursor
    /// is in, when it is past its first character). The search wraps around
    /// the document, so with a single misspelled word the answer is that
    /// word from anywhere. `Ok(None)` when nothing is marked as misspelled.
    ///
    /// The engine asks once per step of a count (see [`walk_misspellings`]).
    /// The default has no spell checker.
    fn misspelling_from(
        &mut self,
        state: &EditorState,
        from: Position,
        forward: bool,
    ) -> Result<Option<TextRange>, SpellOff> {
        let _ = (state, from, forward);
        Err(SpellOff)
    }

    /// Carries out `z=`, `zg` or `zug`. Like a scroll, the command may be
    /// carried out after the engine returns (it usually has to be: it edits
    /// the document or opens a menu), and the host reports what came of it
    /// itself ("no suggestions", the word that was learned). `Err` only
    /// says that there is no spell checker to ask. The default has none.
    fn spell_command(
        &mut self,
        state: &EditorState,
        command: SpellCommand,
    ) -> Result<(), SpellOff> {
        let _ = (state, command);
        Err(SpellOff)
    }
}
