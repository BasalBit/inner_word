//! Action handlers: what each action in [`crate::actions`] does.
//!
//! [`register`] attaches one listener per action to the editor's surface
//! div. Every handler is a method `fn(&mut Editor, &mut Window, &mut
//! Context<Editor>)`. Editing handlers go through `Editor::user_edit`
//! (blocked in read-only mode); movement and selection go through
//! `Editor::edit`. Both end in the mutation seam, so events, layout and
//! scrolling are handled in one place.
//!
//! Status: the handlers are real but simple. Left for the implementer:
//! `tab` inside a code block (inserts nothing, indents the block),
//! page-wise movement keeping the caret's viewport row, rich-text (HTML)
//! clipboard formats.

use gpui::{ClipboardItem, Context, Div, Window, prelude::*};
use writer_core::{
    Alignment, BlockKind, Choice, Fragment, Highlight, InlineStyle, Movement, Selection,
};

use crate::{actions::*, config::EditorMode, editor::Editor, events::EditorEvent};

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
    })
}

type Cx<'a, 'b> = &'a mut Context<'b, Editor>;

// --- Movement and selection -------------------------------------------------

impl Editor {
    fn movement(&mut self, movement: Movement, extend: bool, cx: Cx) {
        self.edit(|state| state.move_caret(movement, extend), cx);
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
        let Some(step) = self.move_by_visual_rows(from, rows, self.goal_x) else {
            // No layout yet: fall back to hard lines.
            let movement = if rows < 0 {
                Movement::BlockStart
            } else {
                Movement::BlockEnd
            };
            return self.movement(movement, extend, cx);
        };
        let target = if step.rows_moved == 0 {
            if rows < 0 {
                self.state.document().start()
            } else {
                self.state.document().end()
            }
        } else {
            step.position
        };
        let anchor = if extend { selection.anchor } else { target };
        self.edit(
            |state| state.set_selection(Selection::new(anchor, target)),
            cx,
        );
        // `edit` clears the goal when the selection changes; a vertical
        // move keeps it.
        self.goal_x = Some(step.goal_x);
    }

    fn page(&mut self, direction: isize, extend: bool, cx: Cx) {
        let rows = self.rows_per_page().unwrap_or(1) as isize;
        self.vertical(rows * direction, extend, cx);
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
        self.movement(Movement::LineStart, false, cx)
    }
    fn move_to_line_end(&mut self, _: &mut Window, cx: Cx) {
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
        self.movement(Movement::LineStart, true, cx)
    }
    fn select_to_line_end(&mut self, _: &mut Window, cx: Cx) {
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
    fn delete_to_line_start(&mut self, _: &mut Window, cx: Cx) {
        self.user_edit(|state| state.delete_to_line_start(), cx);
    }
    fn delete_to_line_end(&mut self, _: &mut Window, cx: Cx) {
        self.user_edit(|state| state.delete_to_line_end(), cx);
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
        if self.config.mode.is_compact() {
            return;
        }
        self.user_edit(|state| state.insert_page_break(), cx);
    }
    fn insert_divider(&mut self, _: &mut Window, cx: Cx) {
        if self.config.mode.is_compact() {
            return;
        }
        self.user_edit(|state| state.insert_divider(), cx);
    }
}

// --- Formatting ---------------------------------------------------------------

impl Editor {
    fn toggle_bold(&mut self, _: &mut Window, cx: Cx) {
        self.user_edit(|state| state.toggle_style(InlineStyle::Bold), cx);
    }
    fn toggle_italic(&mut self, _: &mut Window, cx: Cx) {
        self.user_edit(|state| state.toggle_style(InlineStyle::Italic), cx);
    }
    fn toggle_underline(&mut self, _: &mut Window, cx: Cx) {
        self.user_edit(|state| state.toggle_style(InlineStyle::Underline), cx);
    }
    fn toggle_strikethrough(&mut self, _: &mut Window, cx: Cx) {
        self.user_edit(|state| state.toggle_style(InlineStyle::Strikethrough), cx);
    }
    fn toggle_inline_code(&mut self, _: &mut Window, cx: Cx) {
        self.user_edit(|state| state.toggle_style(InlineStyle::Code), cx);
    }
    fn toggle_highlight(&mut self, _: &mut Window, cx: Cx) {
        self.user_edit(|state| state.toggle_highlight(Highlight::Yellow), cx);
    }
    fn clear_formatting(&mut self, _: &mut Window, cx: Cx) {
        self.user_edit(|state| state.clear_formatting(), cx);
    }

    fn set_kind(&mut self, kind: BlockKind, cx: Cx) {
        self.user_edit(|state| state.set_block_kind(kind), cx);
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
        self.user_edit(|state| state.toggle_todo_checked(), cx);
    }

    fn align_left(&mut self, _: &mut Window, cx: Cx) {
        self.user_edit(|state| state.set_alignment(Alignment::Left), cx);
    }
    fn align_center(&mut self, _: &mut Window, cx: Cx) {
        self.user_edit(|state| state.set_alignment(Alignment::Center), cx);
    }
    fn align_right(&mut self, _: &mut Window, cx: Cx) {
        self.user_edit(|state| state.set_alignment(Alignment::Right), cx);
    }
    fn align_justify(&mut self, _: &mut Window, cx: Cx) {
        self.user_edit(|state| state.set_alignment(Alignment::Justify), cx);
    }
    fn increase_text_size(&mut self, _: &mut Window, cx: Cx) {
        self.user_edit(|state| state.step_text_size(1), cx);
    }
    fn decrease_text_size(&mut self, _: &mut Window, cx: Cx) {
        self.user_edit(|state| state.step_text_size(-1), cx);
    }
    fn reset_text_size(&mut self, _: &mut Window, cx: Cx) {
        self.user_edit(|state| state.reset_text_size(), cx);
    }
    fn indent(&mut self, _: &mut Window, cx: Cx) {
        self.user_edit(|state| state.indent(), cx);
    }
    fn outdent(&mut self, _: &mut Window, cx: Cx) {
        self.user_edit(|state| state.outdent(), cx);
    }
    fn move_block_up(&mut self, _: &mut Window, cx: Cx) {
        self.user_edit(|state| state.move_blocks_up(), cx);
    }
    fn move_block_down(&mut self, _: &mut Window, cx: Cx) {
        self.user_edit(|state| state.move_blocks_down(), cx);
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
