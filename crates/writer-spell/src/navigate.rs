//! From one marked word to the next: the walk behind "next misspelling",
//! "previous misspelling" and the Vim keys.
//!
//! The walk goes block by block from a position, around the end of the
//! document and back to where it started. A block that has no result for
//! its current text is checked on the way, so the answer does not depend
//! on how far the background pass has come; the checking is bounded by
//! [`SEEK_LIMIT`], and a walk that runs out of time says so instead of
//! claiming that nothing is misspelled.

use std::time::Instant;

use gpui::Context;
use writer_core::{Position, Selection, TextRange};
use writer_editor::Autoscroll;

use crate::checker::{SEEK_LIMIT, SpellChecker, SpellEvent};
use crate::marks;
use crate::work::Checked;

/// Which marks of the block the walk starts in count.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Rule {
    /// Forward: the first mark that starts after the position.
    After,
    /// Forward: the first mark that ends after the position (the word the
    /// caret is in counts).
    Through,
    /// Forward: the first mark that starts at or after the position.
    AtOrAfter,
    /// Backward: the last mark that starts before the position.
    Before,
}

/// How a walk ended.
enum Sought {
    Found(TextRange),
    /// Every block was looked at: nothing is marked.
    Nothing,
    /// Not every block could be checked in time.
    OutOfTime,
}

impl SpellChecker {
    /// The first marked word that starts after `from`, wrapping around
    /// the end of the document. Blocks that were not checked yet are
    /// checked on the way, within a time limit; `None` when nothing is
    /// marked (or nothing was found in time).
    pub fn next_misspelling(
        &mut self,
        from: Position,
        cx: &mut Context<Self>,
    ) -> Option<TextRange> {
        match self.seek(from, Rule::After, cx) {
            Sought::Found(range) => Some(range),
            Sought::Nothing | Sought::OutOfTime => None,
        }
    }

    /// The last marked word that starts before `from`, wrapping around
    /// the start of the document. See [`SpellChecker::next_misspelling`].
    pub fn previous_misspelling(
        &mut self,
        from: Position,
        cx: &mut Context<Self>,
    ) -> Option<TextRange> {
        match self.seek(from, Rule::Before, cx) {
            Sought::Found(range) => Some(range),
            Sought::Nothing | Sought::OutOfTime => None,
        }
    }

    /// Selects the next misspelled word and scrolls to it, wrapping: from
    /// a caret the word it is in or the first one after it, from a
    /// selection the first one after it. Says so ([`SpellEvent::Notice`])
    /// when there is none. Returns whether a word was selected.
    pub fn go_to_next_misspelling(&mut self, cx: &mut Context<Self>) -> bool {
        self.go_to(true, cx)
    }

    /// Selects the previous misspelled word and scrolls to it, wrapping:
    /// the last one that starts before the caret or the selection. See
    /// [`SpellChecker::go_to_next_misspelling`].
    pub fn go_to_previous_misspelling(&mut self, cx: &mut Context<Self>) -> bool {
        self.go_to(false, cx)
    }

    fn go_to(&mut self, forward: bool, cx: &mut Context<Self>) -> bool {
        if !self.ready_for_command(cx) {
            return false;
        }
        // Asking for the misspellings ends "still typing this word".
        self.release_typed_word(cx);
        let selection = self.editor.read(cx).selection();
        let (from, rule) = match (forward, selection.is_collapsed()) {
            (true, true) => (selection.head, Rule::Through),
            (true, false) => (selection.end(), Rule::AtOrAfter),
            (false, _) => (selection.start(), Rule::Before),
        };
        match self.seek(from, rule, cx) {
            Sought::Found(range) => {
                self.editor.update(cx, |editor, cx| {
                    // No scrolling for a word that is on screen already.
                    let on_screen = editor.visible_range().is_some_and(|visible| {
                        visible.contains(range.start) && visible.contains(range.end)
                    });
                    let autoscroll = if on_screen {
                        Autoscroll::Fit
                    } else {
                        Autoscroll::Center
                    };
                    editor.edit_with(
                        autoscroll,
                        |state| state.set_selection(Selection::from_range(range)),
                        cx,
                    );
                });
                true
            }
            Sought::Nothing => {
                self.notice("No misspellings found", cx);
                false
            }
            Sought::OutOfTime => {
                self.notice(
                    "No misspelling found yet: the document is still being checked",
                    cx,
                );
                // Go on where the walk stopped.
                self.start_worker(cx);
                false
            }
        }
    }

    /// Walks from `from` to the first mark the rule accepts.
    fn seek(&mut self, from: Position, rule: Rule, cx: &mut Context<Self>) -> Sought {
        if !self.was_active {
            return Sought::Nothing;
        }
        let editor = self.editor.read(cx);
        let document = editor.document();
        let count = document.block_count();
        if count == 0 || (self.settled && marks::count(editor) == 0) {
            return Sought::Nothing;
        }
        let from = document.clamp_position(from);
        let forward = rule != Rule::Before;
        let deadline = Instant::now() + SEEK_LIMIT;
        let mut changed = false;
        let mut sought = Sought::Nothing;
        // The block of `from` is looked at twice: first what lies in the
        // direction of the walk, at the very end the rest of it.
        for step in 0..=count {
            let index = if forward {
                (from.block + step) % count
            } else {
                (from.block + count - step % count) % count
            };
            if !marks::is_checked(self.editor.read(cx), index) {
                if Instant::now() >= deadline {
                    sought = Sought::OutOfTime;
                    break;
                }
                if let Some(block) = self.editor.read(cx).document().block(index) {
                    self.waiting.remove(&block.id());
                }
                match self.check_block(index, Some(deadline), false, cx) {
                    Checked::Done {
                        changed: block_changed,
                    } => changed |= block_changed,
                    Checked::Interrupted => {
                        sought = Sought::OutOfTime;
                        break;
                    }
                }
            }
            let ranges = marks::of_block(self.editor.read(cx), index);
            let hit = if step == 0 {
                match rule {
                    Rule::After => ranges.iter().find(|range| range.start > from.offset),
                    Rule::Through => ranges.iter().find(|range| range.end > from.offset),
                    Rule::AtOrAfter => ranges.iter().find(|range| range.start >= from.offset),
                    Rule::Before => ranges.iter().rev().find(|range| range.start < from.offset),
                }
            } else if forward {
                ranges.first()
            } else {
                ranges.last()
            };
            if let Some(range) = hit {
                sought = Sought::Found(TextRange::in_block(index, range.start, range.end));
                break;
            }
        }
        if changed {
            cx.emit(SpellEvent::MarksChanged);
        }
        sought
    }
}
