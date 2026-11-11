//! Action handlers: what each action in [`crate::actions`] does.
//!
//! [`register`] attaches one listener per action to the editor's surface
//! div. Every handler is a method `fn(&mut Editor, &mut Window, &mut
//! Context<Editor>)`. Editing handlers go through `Editor::user_edit`
//! (blocked in read-only mode); movement and selection go through
//! `Editor::edit`. Both end in the mutation seam, so events, layout and
//! scrolling are handled in one place.
//!
//! A compact editor holds plain text: formatting and block commands do
//! nothing there and let the key travel on to the next binding (see
//! `Editor::rich_text`).
//!
//! The clipboard carries plain text for other apps and the copied fragment
//! (as JSON in GPUI's clipboard metadata) for pasting back with formatting.
//! GPUI 0.2.2 cannot write HTML or RTF, so formatting survives only inside
//! The Writer.

use gpui::{ClipboardItem, Context, Div, Window, prelude::*};
use writer_core::{
    Alignment, BlockKind, Choice, Fragment, Highlight, InlineStyle, Movement, Position, Selection,
    TextRange, text,
};

use crate::{
    actions::*,
    config::EditorMode,
    editor::Editor,
    events::EditorEvent,
    movement::CaretRow,
    scroll::{Autoscroll, PendingScroll},
};

/// Width of one indentation step in a code block, in spaces.
const CODE_INDENT: usize = 4;

/// Attaches `$editor.$method(window, cx)` as the listener of each action.
macro_rules! on_actions {
    ($div:expr, $cx:expr, { $($action:ty => $method:ident),* $(,)? }) => {{
        let div = $div;
        $(
            let div = div.on_action($cx.listener(
                |editor: &mut Editor, _: &$action, window: &mut Window, cx: &mut Context<Editor>| {
                    editor.$method(window, cx)
                },
            ));
        )*
        div
    }};
}

/// Registers every editor action on the surface div.
pub(crate) fn register(surface: Div, cx: &mut Context<Editor>) -> Div {
    on_actions!(surface, cx, {
        MoveLeft => move_left,
        MoveRight => move_right,
        MoveUp => move_up,
        MoveDown => move_down,
        MoveWordLeft => move_word_left,
        MoveWordRight => move_word_right,
        MoveToLineStart => move_to_line_start,
        MoveToLineEnd => move_to_line_end,
        MoveToParagraphStart => move_to_paragraph_start,
        MoveToParagraphEnd => move_to_paragraph_end,
        MoveToDocumentStart => move_to_document_start,
        MoveToDocumentEnd => move_to_document_end,
        MovePageUp => move_page_up,
        MovePageDown => move_page_down,
        SelectLeft => select_left,
        SelectRight => select_right,
        SelectUp => select_up,
        SelectDown => select_down,
        SelectWordLeft => select_word_left,
        SelectWordRight => select_word_right,
        SelectToLineStart => select_to_line_start,
        SelectToLineEnd => select_to_line_end,
        SelectToParagraphStart => select_to_paragraph_start,
        SelectToParagraphEnd => select_to_paragraph_end,
        SelectToDocumentStart => select_to_document_start,
        SelectToDocumentEnd => select_to_document_end,
        SelectPageUp => select_page_up,
        SelectPageDown => select_page_down,
        SelectAll => select_all,
        Backspace => backspace,
        Delete => delete,
        DeleteWordBackward => delete_word_backward,
        DeleteWordForward => delete_word_forward,
        DeleteToLineStart => delete_to_line_start,
        DeleteToLineEnd => delete_to_line_end,
        DeleteToParagraphEnd => delete_to_paragraph_end,
        Newline => newline,
        InsertSoftBreak => insert_soft_break,
        InsertPageBreak => insert_page_break,
        InsertDivider => insert_divider,
        ToggleBold => toggle_bold,
        ToggleItalic => toggle_italic,
        ToggleUnderline => toggle_underline,
        ToggleStrikethrough => toggle_strikethrough,
        ToggleInlineCode => toggle_inline_code,
        ToggleHighlight => toggle_highlight,
        ClearFormatting => clear_formatting,
        SetParagraph => set_paragraph,
        SetHeading1 => set_heading_1,
        SetHeading2 => set_heading_2,
        SetHeading3 => set_heading_3,
        SetHeading4 => set_heading_4,
        SetHeading5 => set_heading_5,
        SetHeading6 => set_heading_6,
        ToggleNumberedList => toggle_numbered_list,
        ToggleBulletList => toggle_bullet_list,
        ToggleTodoList => toggle_todo_list,
        ToggleQuote => toggle_quote,
        ToggleCodeBlock => toggle_code_block,
        ToggleTodoChecked => toggle_todo_checked,
        AlignLeft => align_left,
        AlignCenter => align_center,
        AlignRight => align_right,
        AlignJustify => align_justify,
        IncreaseTextSize => increase_text_size,
        DecreaseTextSize => decrease_text_size,
        ResetTextSize => reset_text_size,
        Indent => indent,
        Outdent => outdent,
        MoveBlockUp => move_block_up,
        MoveBlockDown => move_block_down,
        Undo => undo,
        Redo => redo,
        Copy => copy,
        Cut => cut,
        Paste => paste,
        PasteAsPlainText => paste_as_plain_text,
        OpenContextMenu => open_context_menu,
    })
}

type Cx<'a, 'b> = &'a mut Context<'b, Editor>;

// --- Movement and selection -------------------------------------------------

impl Editor {
    fn movement(&mut self, movement: Movement, extend: bool, cx: Cx) {
        self.edit(|state| state.move_caret(movement, extend), cx);
        self.reveal_caret(cx);
    }

    /// The start or the end of the visual row the caret is on, and the
    /// affinity the caret has there: what `cmd-left` / `cmd-right` go to
    /// in a wrapped paragraph. `None` before the first layout.
    fn row_edge(&self, end: bool) -> Option<(Position, bool)> {
        let CaretRow { row, soft_wrapped } = self.caret_row()?;
        if !end {
            return Some((row.start, false));
        }
        if !soft_wrapped || row.end.offset == row.start.offset {
            return Some((row.end, false));
        }
        // A row that ends at a soft wrap ends where the next one starts.
        // The caret's affinity keeps it on this row.
        if self.caret_has_affinity() {
            return Some((row.end, true));
        }
        // A caret without one (a block caret sits on a character) stops
        // on the row's last character instead.
        let block = self.state.document().block(row.end.block)?;
        let before = text::prev_grapheme_boundary(block.text(), row.end.offset);
        Some((
            Position::new(row.end.block, before.max(row.start.offset)),
            false,
        ))
    }

    /// Moves or extends to the start or end of the visual row (of the hard
    /// line when there is no layout yet).
    fn line_edge(&mut self, end: bool, extend: bool, cx: Cx) {
        let Some((target, upstream)) = self.row_edge(end) else {
            let movement = if end {
                Movement::LineEnd
            } else {
                Movement::LineStart
            };
            return self.movement(movement, extend, cx);
        };
        let anchor = if extend {
            self.state.selection().anchor
        } else {
            target
        };
        self.edit(
            |state| state.set_selection(Selection::new(anchor, target)),
            cx,
        );
        self.caret_upstream = upstream;
        self.reveal_caret(cx);
    }

    /// A movement key brings the caret into view even when the caret has
    /// nowhere to go (`cmd-up` at the start of a document that is scrolled
    /// down).
    fn reveal_caret(&mut self, cx: Cx) {
        self.request_scroll(PendingScroll::Position(None, Autoscroll::Fit));
        cx.notify();
    }

    /// Moves the selection head by visual rows, keeping the goal x across
    /// consecutive vertical moves. At the first row `up` goes to the start
    /// of the document and at the last row `down` goes to its end, as macOS
    /// text views do.
    fn vertical(&mut self, rows: isize, extend: bool, cx: Cx) {
        let selection = self.state.selection();
        // A plain up / down on a range collapses to its start / end first.
        let from = if extend || selection.is_collapsed() {
            selection.head
        } else if rows < 0 {
            selection.start()
        } else {
            selection.end()
        };
        // The move starts on the row the caret is drawn on.
        let upstream = from == selection.head && self.caret_is_upstream();
        let Some((step, upstream)) = self.move_caret_by_rows(from, upstream, rows, self.goal_x)
        else {
            // No layout yet: fall back to hard lines.
            let movement = if rows < 0 {
                Movement::BlockStart
            } else {
                Movement::BlockEnd
            };
            return self.movement(movement, extend, cx);
        };
        let (target, upstream) = if step.rows_moved == 0 {
            if rows < 0 {
                (self.state.document().start(), false)
            } else {
                (self.state.document().end(), false)
            }
        } else {
            (step.position, upstream)
        };
        let anchor = if extend { selection.anchor } else { target };
        self.edit(
            |state| state.set_selection(Selection::new(anchor, target)),
            cx,
        );
        self.caret_upstream = upstream;
        self.reveal_caret(cx);
        // `edit` clears the goal when the selection changes; a vertical
        // move keeps it.
        self.goal_x = Some(step.goal_x);
    }

    /// Moves the caret a viewport page up or down and scrolls by the same
    /// amount, so the caret keeps its place on screen. One row of the old
    /// page stays visible for context.
    fn page(&mut self, direction: isize, extend: bool, cx: Cx) {
        let rows = (self.rows_per_page().unwrap_or(2) as isize - 1).max(1);
        let caret_y = self.caret_viewport_y();
        self.vertical(rows * direction, extend, cx);
        if let Some(viewport_y) = caret_y {
            self.request_scroll(PendingScroll::CaretAt { viewport_y });
        }
    }

    fn move_left(&mut self, _: &mut Window, cx: Cx) {
        self.movement(Movement::Left, false, cx)
    }
    fn move_right(&mut self, _: &mut Window, cx: Cx) {
        self.movement(Movement::Right, false, cx)
    }
    fn move_up(&mut self, _: &mut Window, cx: Cx) {
        self.vertical(-1, false, cx)
    }
    fn move_down(&mut self, _: &mut Window, cx: Cx) {
        self.vertical(1, false, cx)
    }
    fn move_word_left(&mut self, _: &mut Window, cx: Cx) {
        self.movement(Movement::WordLeft, false, cx)
    }
    fn move_word_right(&mut self, _: &mut Window, cx: Cx) {
        self.movement(Movement::WordRight, false, cx)
    }
    fn move_to_line_start(&mut self, _: &mut Window, cx: Cx) {
        self.line_edge(false, false, cx)
    }
    fn move_to_line_end(&mut self, _: &mut Window, cx: Cx) {
        self.line_edge(true, false, cx)
    }
    // The paragraph keys (`ctrl-a`, `ctrl-e`) go by hard lines: the whole
    // paragraph however often it wraps, or one line of a block that has
    // line breaks in it (a code block, a paragraph with soft breaks).
    fn move_to_paragraph_start(&mut self, _: &mut Window, cx: Cx) {
        self.movement(Movement::LineStart, false, cx)
    }
    fn move_to_paragraph_end(&mut self, _: &mut Window, cx: Cx) {
        self.movement(Movement::LineEnd, false, cx)
    }
    fn move_to_document_start(&mut self, _: &mut Window, cx: Cx) {
        self.movement(Movement::DocStart, false, cx)
    }
    fn move_to_document_end(&mut self, _: &mut Window, cx: Cx) {
        self.movement(Movement::DocEnd, false, cx)
    }
    fn move_page_up(&mut self, _: &mut Window, cx: Cx) {
        self.page(-1, false, cx)
    }
    fn move_page_down(&mut self, _: &mut Window, cx: Cx) {
        self.page(1, false, cx)
    }
    fn select_left(&mut self, _: &mut Window, cx: Cx) {
        self.movement(Movement::Left, true, cx)
    }
    fn select_right(&mut self, _: &mut Window, cx: Cx) {
        self.movement(Movement::Right, true, cx)
    }
    fn select_up(&mut self, _: &mut Window, cx: Cx) {
        self.vertical(-1, true, cx)
    }
    fn select_down(&mut self, _: &mut Window, cx: Cx) {
        self.vertical(1, true, cx)
    }
    fn select_word_left(&mut self, _: &mut Window, cx: Cx) {
        self.movement(Movement::WordLeft, true, cx)
    }
    fn select_word_right(&mut self, _: &mut Window, cx: Cx) {
        self.movement(Movement::WordRight, true, cx)
    }
    fn select_to_line_start(&mut self, _: &mut Window, cx: Cx) {
        self.line_edge(false, true, cx)
    }
    fn select_to_line_end(&mut self, _: &mut Window, cx: Cx) {
        self.line_edge(true, true, cx)
    }
    fn select_to_paragraph_start(&mut self, _: &mut Window, cx: Cx) {
        self.movement(Movement::LineStart, true, cx)
    }
    fn select_to_paragraph_end(&mut self, _: &mut Window, cx: Cx) {
        self.movement(Movement::LineEnd, true, cx)
    }
    fn select_to_document_start(&mut self, _: &mut Window, cx: Cx) {
        self.movement(Movement::DocStart, true, cx)
    }
    fn select_to_document_end(&mut self, _: &mut Window, cx: Cx) {
        self.movement(Movement::DocEnd, true, cx)
    }
    fn select_page_up(&mut self, _: &mut Window, cx: Cx) {
        self.page(-1, true, cx)
    }
    fn select_page_down(&mut self, _: &mut Window, cx: Cx) {
        self.page(1, true, cx)
    }
    fn select_all(&mut self, _: &mut Window, cx: Cx) {
        self.edit(|state| state.select_all(), cx);
    }
}

// --- Deletion and insertion -------------------------------------------------

impl Editor {
    fn backspace(&mut self, _: &mut Window, cx: Cx) {
        self.user_edit(|state| state.backspace(), cx);
    }
    fn delete(&mut self, _: &mut Window, cx: Cx) {
        self.user_edit(|state| state.delete_forward(), cx);
    }
    fn delete_word_backward(&mut self, _: &mut Window, cx: Cx) {
        self.user_edit(|state| state.delete_word_backward(), cx);
    }
    fn delete_word_forward(&mut self, _: &mut Window, cx: Cx) {
        self.user_edit(|state| state.delete_word_forward(), cx);
    }
    /// Deletes back to the start of the visual row the caret is on (with a
    /// selection, or at the start of a row, what the engine's line
    /// deletion does).
    fn delete_to_line_start(&mut self, _: &mut Window, cx: Cx) {
        let selection = self.state.selection();
        let range = selection
            .is_collapsed()
            .then(|| self.caret_row())
            .flatten()
            .map(|caret| TextRange::new(caret.row.start, selection.head))
            .filter(|range| !range.is_empty());
        match range {
            Some(range) => self.delete_text(range, cx),
            None => {
                self.user_edit(|state| state.delete_to_line_start(), cx);
            }
        }
    }
    /// Deletes forward to the end of the visual row the caret is on.
    fn delete_to_line_end(&mut self, _: &mut Window, cx: Cx) {
        let selection = self.state.selection();
        let caret = selection.is_collapsed().then(|| self.caret_row()).flatten();
        if let Some(caret) = caret
            && caret.soft_wrapped
            && caret.row.end == selection.head
        {
            // The caret is at the end of a wrapped row (upstream): there
            // is nothing of this row left to delete, and the rest of the
            // paragraph is other rows.
            return;
        }
        let range = caret
            .map(|caret| TextRange::new(selection.head, caret.row.end))
            .filter(|range| !range.is_empty());
        match range {
            Some(range) => self.delete_text(range, cx),
            None => {
                self.user_edit(|state| state.delete_to_line_end(), cx);
            }
        }
    }

    /// `ctrl-k`: deletes to the end of the paragraph (the hard line); at
    /// its end, the line break that follows.
    fn delete_to_paragraph_end(&mut self, _: &mut Window, cx: Cx) {
        self.user_edit(|state| state.delete_to_line_end(), cx);
    }

    /// Deletes a range as one user edit, leaving the caret at its start.
    fn delete_text(&mut self, range: TextRange, cx: Cx) {
        self.user_edit(
            |state| {
                state.grouped(|state| {
                    state.select_range(range);
                    state.delete_selection()
                })
            },
            cx,
        );
    }

    fn newline(&mut self, _: &mut Window, cx: Cx) {
        if let EditorMode::Compact(options) = self.config.mode
            && options.submit_on_enter
        {
            cx.emit(EditorEvent::Submitted);
            return;
        }
        self.user_edit(|state| state.insert_paragraph_break(), cx);
    }
    fn insert_soft_break(&mut self, _: &mut Window, cx: Cx) {
        self.user_edit(|state| state.insert_soft_break(), cx);
    }
    fn insert_page_break(&mut self, _: &mut Window, cx: Cx) {
        if self.rich_text(cx) {
            self.user_edit(|state| state.insert_page_break(), cx);
        }
    }
    fn insert_divider(&mut self, _: &mut Window, cx: Cx) {
        if self.rich_text(cx) {
            self.user_edit(|state| state.insert_divider(), cx);
        }
    }
}

// --- Formatting ---------------------------------------------------------------

impl Editor {
    /// Whether rich-text commands apply. A compact editor holds plain
    /// text: there the command does nothing and the key goes on to the
    /// next binding that wants it.
    fn rich_text(&self, cx: Cx) -> bool {
        if self.config.mode.is_compact() {
            cx.propagate();
            false
        } else {
            true
        }
    }

    fn toggle_bold(&mut self, _: &mut Window, cx: Cx) {
        if self.rich_text(cx) {
            self.user_edit(|state| state.toggle_style(InlineStyle::Bold), cx);
        }
    }
    fn toggle_italic(&mut self, _: &mut Window, cx: Cx) {
        if self.rich_text(cx) {
            self.user_edit(|state| state.toggle_style(InlineStyle::Italic), cx);
        }
    }
    fn toggle_underline(&mut self, _: &mut Window, cx: Cx) {
        if self.rich_text(cx) {
            self.user_edit(|state| state.toggle_style(InlineStyle::Underline), cx);
        }
    }
    fn toggle_strikethrough(&mut self, _: &mut Window, cx: Cx) {
        if self.rich_text(cx) {
            self.user_edit(|state| state.toggle_style(InlineStyle::Strikethrough), cx);
        }
    }
    fn toggle_inline_code(&mut self, _: &mut Window, cx: Cx) {
        if self.rich_text(cx) {
            self.user_edit(|state| state.toggle_style(InlineStyle::Code), cx);
        }
    }
    fn toggle_highlight(&mut self, _: &mut Window, cx: Cx) {
        if self.rich_text(cx) {
            self.user_edit(|state| state.toggle_highlight(Highlight::Yellow), cx);
        }
    }
    fn clear_formatting(&mut self, _: &mut Window, cx: Cx) {
        if self.rich_text(cx) {
            self.user_edit(|state| state.clear_formatting(), cx);
        }
    }

    fn set_kind(&mut self, kind: BlockKind, cx: Cx) {
        if self.rich_text(cx) {
            self.user_edit(|state| state.set_block_kind(kind), cx);
        }
    }

    /// Sets `kind` on the touched blocks, or turns them back into
    /// paragraphs when they all have that kind already.
    fn toggle_kind(&mut self, kind: BlockKind, cx: Cx) {
        let already = matches!(
            &self.state.format_state().kind,
            Choice::Same(current) if current.same_type(&kind)
        );
        self.set_kind(if already { BlockKind::Paragraph } else { kind }, cx);
    }

    fn set_paragraph(&mut self, _: &mut Window, cx: Cx) {
        self.set_kind(BlockKind::Paragraph, cx)
    }
    fn set_heading_1(&mut self, _: &mut Window, cx: Cx) {
        self.set_kind(BlockKind::heading(1), cx)
    }
    fn set_heading_2(&mut self, _: &mut Window, cx: Cx) {
        self.set_kind(BlockKind::heading(2), cx)
    }
    fn set_heading_3(&mut self, _: &mut Window, cx: Cx) {
        self.set_kind(BlockKind::heading(3), cx)
    }
    fn set_heading_4(&mut self, _: &mut Window, cx: Cx) {
        self.set_kind(BlockKind::heading(4), cx)
    }
    fn set_heading_5(&mut self, _: &mut Window, cx: Cx) {
        self.set_kind(BlockKind::heading(5), cx)
    }
    fn set_heading_6(&mut self, _: &mut Window, cx: Cx) {
        self.set_kind(BlockKind::heading(6), cx)
    }
    fn toggle_numbered_list(&mut self, _: &mut Window, cx: Cx) {
        self.toggle_kind(BlockKind::NumberedItem, cx)
    }
    fn toggle_bullet_list(&mut self, _: &mut Window, cx: Cx) {
        self.toggle_kind(BlockKind::BulletItem, cx)
    }
    fn toggle_todo_list(&mut self, _: &mut Window, cx: Cx) {
        self.toggle_kind(BlockKind::todo(), cx)
    }
    fn toggle_quote(&mut self, _: &mut Window, cx: Cx) {
        self.toggle_kind(BlockKind::Quote, cx)
    }
    fn toggle_code_block(&mut self, _: &mut Window, cx: Cx) {
        self.toggle_kind(BlockKind::code(), cx)
    }
    fn toggle_todo_checked(&mut self, _: &mut Window, cx: Cx) {
        if self.rich_text(cx) {
            self.user_edit(|state| state.toggle_todo_checked(), cx);
        }
    }

    fn align_left(&mut self, _: &mut Window, cx: Cx) {
        if self.rich_text(cx) {
            self.user_edit(|state| state.set_alignment(Alignment::Left), cx);
        }
    }
    fn align_center(&mut self, _: &mut Window, cx: Cx) {
        if self.rich_text(cx) {
            self.user_edit(|state| state.set_alignment(Alignment::Center), cx);
        }
    }
    fn align_right(&mut self, _: &mut Window, cx: Cx) {
        if self.rich_text(cx) {
            self.user_edit(|state| state.set_alignment(Alignment::Right), cx);
        }
    }
    fn align_justify(&mut self, _: &mut Window, cx: Cx) {
        if self.rich_text(cx) {
            self.user_edit(|state| state.set_alignment(Alignment::Justify), cx);
        }
    }
    fn increase_text_size(&mut self, _: &mut Window, cx: Cx) {
        if self.rich_text(cx) {
            self.user_edit(|state| state.step_text_size(1), cx);
        }
    }
    fn decrease_text_size(&mut self, _: &mut Window, cx: Cx) {
        if self.rich_text(cx) {
            self.user_edit(|state| state.step_text_size(-1), cx);
        }
    }
    fn reset_text_size(&mut self, _: &mut Window, cx: Cx) {
        if self.rich_text(cx) {
            self.user_edit(|state| state.reset_text_size(), cx);
        }
    }
    /// `tab`: indents the selected blocks; inside a code block it indents
    /// the text instead; in a compact editor it moves focus on.
    fn indent(&mut self, window: &mut Window, cx: Cx) {
        if self.config.mode.is_compact() {
            window.focus_next();
        } else if let Some(block) = self.code_block_of_selection() {
            self.indent_code(block, cx);
        } else {
            self.user_edit(|state| state.indent(), cx);
        }
    }
    fn outdent(&mut self, window: &mut Window, cx: Cx) {
        if self.config.mode.is_compact() {
            window.focus_prev();
        } else if let Some(block) = self.code_block_of_selection() {
            self.outdent_code(block, cx);
        } else {
            self.user_edit(|state| state.outdent(), cx);
        }
    }

    /// The code block that holds the whole selection, if there is one.
    fn code_block_of_selection(&self) -> Option<usize> {
        let selection = self.state.selection();
        (selection.anchor.block == selection.head.block)
            .then_some(selection.head.block)
            .filter(|index| {
                self.state
                    .document()
                    .block(*index)
                    .is_some_and(|block| block.kind().is_code())
            })
    }

    /// The starts of the hard lines the selection touches in block `index`
    /// (a selection that ends at the start of a line does not touch it).
    fn selected_line_starts(&self, index: usize) -> Vec<usize> {
        let selection = self.state.selection();
        let (from, to) = (selection.start().offset, selection.end().offset);
        let Some(block) = self.state.document().block(index) else {
            return Vec::new();
        };
        let code = block.text();
        let first = text::line_start(code, from);
        let mut starts = vec![first];
        let mut at = first;
        while let Some(newline) = code[at..].find('\n') {
            at += newline + 1;
            if at >= to {
                break;
            }
            starts.push(at);
        }
        starts
    }

    /// `tab` in a code block: a caret moves to the next indentation stop,
    /// a selection indents every line it touches.
    fn indent_code(&mut self, index: usize, cx: Cx) {
        let selection = self.state.selection();
        if selection.is_collapsed() {
            let column = self.state.document().block(index).map_or(0, |block| {
                let code = block.text();
                let offset = selection.head.offset.min(code.len());
                code[text::line_start(code, offset)..offset].chars().count()
            });
            let spaces = " ".repeat(CODE_INDENT - column % CODE_INDENT);
            self.user_edit(|state| state.insert_text(&spaces), cx);
            return;
        }
        let starts = self.selected_line_starts(index);
        let shift = |offset: usize| {
            offset + CODE_INDENT * starts.iter().filter(|start| **start < offset).count()
        };
        let moved = Selection::new(
            Position::new(index, shift(selection.anchor.offset)),
            Position::new(index, shift(selection.head.offset)),
        );
        let indent = " ".repeat(CODE_INDENT);
        self.user_edit(
            |state| {
                state.grouped(|state| {
                    for start in starts.iter().rev() {
                        state.replace_range(TextRange::in_block(index, *start, *start), &indent);
                    }
                    state.set_selection(moved);
                })
            },
            cx,
        );
    }

    /// `shift-tab` in a code block: removes one indentation step from
    /// every line the selection touches.
    fn outdent_code(&mut self, index: usize, cx: Cx) {
        let selection = self.state.selection();
        let starts = self.selected_line_starts(index);
        // (line start, bytes of indentation to remove)
        let cuts: Vec<(usize, usize)> = {
            let Some(block) = self.state.document().block(index) else {
                return;
            };
            let code = block.text().as_bytes();
            starts
                .iter()
                .map(|start| {
                    let spaces = code[*start..]
                        .iter()
                        .take(CODE_INDENT)
                        .take_while(|byte| **byte == b' ')
                        .count();
                    let cut = if spaces == 0 && code.get(*start) == Some(&b'\t') {
                        1
                    } else {
                        spaces
                    };
                    (*start, cut)
                })
                .filter(|(_, cut)| *cut > 0)
                .collect()
        };
        if cuts.is_empty() {
            return;
        }
        let shift = |offset: usize| {
            offset
                - cuts
                    .iter()
                    .filter(|(start, _)| *start < offset)
                    .map(|(start, cut)| (*cut).min(offset - start))
                    .sum::<usize>()
        };
        let moved = Selection::new(
            Position::new(index, shift(selection.anchor.offset)),
            Position::new(index, shift(selection.head.offset)),
        );
        self.user_edit(
            |state| {
                state.grouped(|state| {
                    for (start, cut) in cuts.iter().rev() {
                        state.delete_range(TextRange::in_block(index, *start, start + cut));
                    }
                    state.set_selection(moved);
                })
            },
            cx,
        );
    }
    fn move_block_up(&mut self, _: &mut Window, cx: Cx) {
        if self.rich_text(cx) {
            self.user_edit(|state| state.move_blocks_up(), cx);
        }
    }
    fn move_block_down(&mut self, _: &mut Window, cx: Cx) {
        if self.rich_text(cx) {
            self.user_edit(|state| state.move_blocks_down(), cx);
        }
    }
}

// --- History and clipboard ------------------------------------------------------

impl Editor {
    fn undo(&mut self, _: &mut Window, cx: Cx) {
        self.user_edit(|state| state.undo(), cx);
    }
    fn redo(&mut self, _: &mut Window, cx: Cx) {
        self.user_edit(|state| state.redo(), cx);
    }

    /// Writes a fragment to the clipboard: plain text for other apps, the
    /// fragment's JSON as metadata for pasting back with formatting.
    fn write_clipboard(fragment: &Fragment, cx: Cx) {
        cx.write_to_clipboard(ClipboardItem::new_string_with_metadata(
            fragment.to_plain_text(),
            fragment.to_json(),
        ));
    }

    fn copy(&mut self, _: &mut Window, cx: Cx) {
        if self.state.selection().is_collapsed() {
            return;
        }
        let fragment = self.state.copy();
        Self::write_clipboard(&fragment, cx);
    }

    fn cut(&mut self, _: &mut Window, cx: Cx) {
        if self.state.selection().is_collapsed() || self.config.read_only {
            return;
        }
        if let Some(fragment) = self.user_edit(|state| state.cut(), cx) {
            Self::write_clipboard(&fragment, cx);
        }
    }

    fn paste(&mut self, _: &mut Window, cx: Cx) {
        let Some(item) = cx.read_from_clipboard() else {
            return;
        };
        // Compact editors hold plain text only.
        let fragment = item
            .metadata()
            .filter(|_| !self.config.mode.is_compact())
            .and_then(|json| Fragment::from_json(json).ok());
        match (fragment, item.text()) {
            (Some(fragment), _) => {
                self.user_edit(|state| state.paste(&fragment), cx);
            }
            (None, Some(text)) => {
                self.user_edit(|state| state.paste_plain_text(&text), cx);
            }
            (None, None) => {}
        }
    }

    fn paste_as_plain_text(&mut self, _: &mut Window, cx: Cx) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.user_edit(|state| state.paste_plain_text(&text), cx);
        }
    }
}
