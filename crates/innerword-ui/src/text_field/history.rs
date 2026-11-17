//! Undo / redo for a single-line field.
//!
//! Snapshots of the whole text (fields are short). Consecutive typing and
//! consecutive deleting coalesce into one undo step; a caret jump, a change
//! of edit kind, or typing whitespace after a word starts a new step, so
//! `cmd-z` removes roughly one word at a time.

use std::ops::Range;

const MAX_STEPS: usize = 200;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Snapshot {
    pub text: String,
    pub selection: Range<usize>,
    pub reversed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EditKind {
    /// Typing (including IME commits).
    Insert,
    /// Backspace / delete, one grapheme at a time.
    Delete,
    /// Anything else (paste, cut, word deletion, replace all): always its own step.
    Other,
}

#[derive(Default)]
pub(crate) struct History {
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// The open coalescing group: kind and caret position after the last edit.
    group: Option<(EditKind, usize)>,
}

impl History {
    /// Records `before` (the state prior to an edit) unless the edit continues
    /// the open group. `caret_before` / `caret_after` are the caret offsets
    /// around the edit; `inserted` is the inserted text.
    pub fn record(
        &mut self,
        before: Snapshot,
        kind: EditKind,
        caret_before: usize,
        caret_after: usize,
        inserted: &str,
    ) {
        let word_break = kind == EditKind::Insert
            && inserted.chars().all(char::is_whitespace)
            && !before.text[..caret_before.min(before.text.len())]
                .chars()
                .next_back()
                .is_none_or(char::is_whitespace);
        let continues = matches!(self.group, Some((open, at)) if open == kind && at == caret_before)
            && kind != EditKind::Other
            && !word_break;
        if !continues {
            self.undo.push(before);
            if self.undo.len() > MAX_STEPS {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.group = (kind != EditKind::Other).then_some((kind, caret_after));
    }

    /// Closes the open group (caret moved, focus changed, ...).
    pub fn break_group(&mut self) {
        self.group = None;
    }

    pub fn undo(&mut self, current: Snapshot) -> Option<Snapshot> {
        let previous = self.undo.pop()?;
        self.redo.push(current);
        self.group = None;
        Some(previous)
    }

    pub fn redo(&mut self, current: Snapshot) -> Option<Snapshot> {
        let next = self.redo.pop()?;
        self.undo.push(current);
        self.group = None;
        Some(next)
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.group = None;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}
