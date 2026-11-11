//! Platform text input: typed characters, IME composition, dead keys, the
//! emoji palette. Implements GPUI's `EntityInputHandler` for [`Editor`]; the
//! element registers it every frame while the editor is focused, editable
//! and [`Editor::text_input_enabled`].
//!
//! The platform talks in UTF-16 offsets into "the document". For the editor
//! that document is one block: the block holding the composition, or else
//! the block of the selection's start. Conversions use writer-core's
//! block-local helpers.
//!
//! A composition is kept as a marked byte range in the UI ([`MarkedText`])
//! and written into the document with `replace_range` inside one undo group,
//! as writer-core's guide prescribes, so a whole composition is one undo
//! step. The element draws the marked range underlined; the part of it the
//! input method reports as selected is the editor's selection.

use std::{ops::Range, time::Duration};

use gpui::{Bounds, Context, EntityInputHandler, Pixels, Point, UTF16Selection, Window};
use writer_core::{Position, Selection, TextRange};

use crate::{editor::Editor, events::EditorEvent, scroll::Autoscroll};

/// A pause in typing this long ends the undo step: `cmd-z` after it takes
/// back what was typed since, not the whole paragraph.
pub(crate) const TYPING_PAUSE: Duration = Duration::from_millis(800);

/// An IME composition in progress: bytes `range` of block `block`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MarkedText {
    pub(crate) block: usize,
    pub(crate) range: Range<usize>,
}

impl Editor {
    /// The block the platform input handler currently talks about.
    fn input_block(&self) -> usize {
        match &self.marked {
            Some(marked) => marked.block,
            None => self.state.selection().start().block,
        }
        .min(self.state.document().block_count() - 1)
    }

    /// Ends a composition: forgets the marked range and closes its undo
    /// group. The composed text stays in the document.
    pub(crate) fn end_composition(&mut self) {
        if self.marked.take().is_some() {
            self.state.end_undo_group();
        }
    }

    /// The marked text range in document positions, while an IME
    /// composition is in progress.
    pub fn marked_range(&self) -> Option<TextRange> {
        self.marked
            .as_ref()
            .map(|marked| TextRange::in_block(marked.block, marked.range.start, marked.range.end))
    }

    /// Block-local byte range for a platform UTF-16 range.
    fn bytes_for_utf16(&self, block: usize, range: &Range<usize>) -> Range<usize> {
        match self.state.document().block(block) {
            Some(block) => block.utf16_to_offset(range.start)..block.utf16_to_offset(range.end),
            None => 0..0,
        }
    }

    /// The selection clipped to `block`, as block-local bytes.
    fn selection_in_block(&self, block: usize) -> Range<usize> {
        let selection = self.state.selection();
        self.state
            .document()
            .range_in_block(selection.range(), block)
            .unwrap_or_else(|| {
                let offset = if selection.head.block == block {
                    selection.head.offset
                } else {
                    0
                };
                offset..offset
            })
    }

    /// Ends the typing undo step after a pause (see [`TYPING_PAUSE`]).
    /// Called after every typed insertion; each call restarts the wait.
    fn break_undo_after_pause(&mut self, cx: &mut Context<Self>) {
        self.typing_pause = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(TYPING_PAUSE).await;
            this.update(cx, |editor, _| {
                if editor.marked.is_none() {
                    editor.state.break_undo_group();
                }
            })
            .ok();
        }));
    }
}

impl EntityInputHandler for Editor {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let index = self.input_block();
        let block = self.state.document().block(index)?;
        let range = self.bytes_for_utf16(index, &range_utf16);
        adjusted_range
            .replace(block.offset_to_utf16(range.start)..block.offset_to_utf16(range.end));
        block.text().get(range).map(str::to_string)
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let index = self.input_block();
        let block = self.state.document().block(index)?;
        let range = self.selection_in_block(index);
        Some(UTF16Selection {
            range: block.offset_to_utf16(range.start)..block.offset_to_utf16(range.end),
            reversed: self.state.selection().is_reversed(),
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        let marked = self.marked.as_ref()?;
        let block = self.state.document().block(marked.block)?;
        Some(block.offset_to_utf16(marked.range.start)..block.offset_to_utf16(marked.range.end))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.end_composition();
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.config.read_only || !self.text_input_enabled {
            return;
        }
        let index = self.input_block();
        let explicit = range_utf16
            .as_ref()
            .map(|range| self.bytes_for_utf16(index, range))
            .or_else(|| self.marked.as_ref().map(|marked| marked.range.clone()));
        let selection_before = self.state.selection();
        self.in_ime_edit = true;
        let composing = explicit.is_some();
        // A range that is not the composition's own marked text: the
        // platform replaces text that was committed earlier.
        let replaced = explicit
            .as_ref()
            .filter(|range| {
                self.marked
                    .as_ref()
                    .is_none_or(|marked| marked.block != index || marked.range != **range)
            })
            .map(|range| TextRange::in_block(index, range.start, range.end));
        match explicit {
            // Plain typing: the engine replaces the selection and runs the
            // input rules and smart punctuation.
            None => {
                self.state.insert_text(text);
            }
            // The end of a composition, or the platform replacing a range
            // (accent menu, text replacement): no input rules.
            Some(range) => {
                let range = TextRange::in_block(index, range.start, range.end);
                if self.marked.is_none() {
                    self.state.begin_undo_group();
                }
                self.state.set_caret(range.start);
                self.state.replace_range(range, text);
                let inserted = writer_core::text::normalize_input(text).len();
                self.state
                    .set_caret(Position::new(index, range.start.offset + inserted));
                self.state.end_undo_group();
            }
        }
        // The composition, if any, is over; its undo group was closed above
        // or is closed here.
        if !composing {
            self.end_composition();
        } else {
            self.marked = None;
        }
        // Reported when the document changed, ahead of `Edited`. An empty
        // text that only removed a composition typed nothing.
        let input = (!text.is_empty() || replaced.is_some()).then(|| EditorEvent::TextInput {
            text: text.to_string(),
            replaced,
        });
        self.finish_edit_announced(selection_before, Autoscroll::Fit, input, cx);
        self.in_ime_edit = false;
        self.break_undo_after_pause(cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.config.read_only || !self.text_input_enabled {
            return;
        }
        let index = self.input_block();
        let range = range_utf16
            .as_ref()
            .map(|range| self.bytes_for_utf16(index, range))
            .or_else(|| self.marked.as_ref().map(|marked| marked.range.clone()))
            .unwrap_or_else(|| self.selection_in_block(index));
        let selection_before = self.state.selection();
        self.in_ime_edit = true;
        if self.marked.is_none() {
            // The whole composition is one undo step.
            self.state.begin_undo_group();
        }
        self.state.set_caret(Position::new(index, range.start));
        self.state
            .replace_range(TextRange::in_block(index, range.start, range.end), new_text);
        let inserted = writer_core::text::normalize_input(new_text);
        // The part of the composition the input method has selected (the
        // clause being converted), relative to the inserted text; without
        // one the caret goes to its end.
        let selected = new_selected_range_utf16
            .map(|selected| {
                writer_core::text::utf16_to_utf8(&inserted, selected.start)
                    ..writer_core::text::utf16_to_utf8(&inserted, selected.end)
            })
            .unwrap_or(inserted.len()..inserted.len());
        self.state.set_selection(Selection::new(
            Position::new(index, range.start + selected.start),
            Position::new(index, range.start + selected.end.max(selected.start)),
        ));
        if inserted.is_empty() {
            // The composition was cancelled.
            self.marked = None;
            self.state.end_undo_group();
        } else {
            self.marked = Some(MarkedText {
                block: index,
                range: range.start..range.start + inserted.len(),
            });
        }
        self.finish_edit(selection_before, Autoscroll::Fit, cx);
        self.in_ime_edit = false;
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        _element_bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let index = self.input_block();
        let range = self.bytes_for_utf16(index, &range_utf16);
        if range.is_empty() {
            let position = Position::new(index, range.start);
            // The input method's window goes where the caret is drawn.
            if position == self.state.selection().head && self.caret_is_upstream() {
                return self.caret_line_bounds();
            }
            return self.position_bounds(position);
        }
        self.range_bounds(TextRange::in_block(index, range.start, range.end))?
            .into_iter()
            .next()
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let position = self.position_at_point(point)?;
        let block = self.state.document().block(position.block)?;
        Some(block.offset_to_utf16(position.offset))
    }
}
