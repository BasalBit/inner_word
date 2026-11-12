//! Shared helpers: a focused editor in an active test window.

#![allow(dead_code)]

use gpui::{Entity, TestAppContext, VisualTestContext};
use writer_core::{Block, BlockKind, Document, InlineAttrs, Position, Selection, TextRange};
use writer_editor::{Editor, EditorConfig};
use writer_ui::theme::{self, ThemeMode};

pub fn setup(cx: &mut TestAppContext) {
    cx.update(|cx| {
        theme::init(ThemeMode::Light, cx);
        writer_ui::init(cx);
        writer_editor::init(cx);
    });
}

pub fn paragraphs(lines: &[&str]) -> Document {
    Document::from_blocks(lines.iter().map(|line| Block::paragraph(line)).collect())
}

/// An editor alone in an active window, with the keyboard.
pub fn open_editor(
    document: Document,
    cx: &mut TestAppContext,
) -> (Entity<Editor>, &mut VisualTestContext) {
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let editor = Editor::new(document, EditorConfig::full(), window, cx);
        editor.focus(window);
        editor
    });
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    (editor, cx)
}

pub fn select(editor: &Entity<Editor>, range: TextRange, cx: &mut VisualTestContext) {
    editor.update(cx, |editor, cx| {
        editor.edit(
            |state| state.set_selection(Selection::from_range(range)),
            cx,
        )
    });
    cx.run_until_parked();
}

pub fn caret(editor: &Entity<Editor>, block: usize, offset: usize, cx: &mut VisualTestContext) {
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.set_caret(Position::new(block, offset)), cx)
    });
    cx.run_until_parked();
}

pub fn text(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> String {
    editor.read_with(cx, |editor, _| editor.document().plain_text())
}

pub fn selection(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> TextRange {
    editor.read_with(cx, |editor, _| editor.selection().range())
}

pub fn selected_text(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> String {
    editor.read_with(cx, |editor, _| editor.state().selected_text())
}

pub fn kinds(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> Vec<BlockKind> {
    editor.read_with(cx, |editor, _| {
        editor
            .document()
            .blocks()
            .iter()
            .map(|block| block.kind().clone())
            .collect()
    })
}

/// The attributes of the character at `offset` of block `block`.
pub fn attrs(
    editor: &Entity<Editor>,
    block: usize,
    offset: usize,
    cx: &mut VisualTestContext,
) -> InlineAttrs {
    editor.read_with(cx, |editor, _| {
        editor
            .document()
            .block(block)
            .and_then(|block| block.attrs_at(offset))
            .cloned()
            .unwrap_or_default()
    })
}

pub fn undo(editor: &Entity<Editor>, cx: &mut VisualTestContext) {
    editor.update(cx, |editor, cx| editor.edit(|state| state.undo(), cx));
    cx.run_until_parked();
}

pub fn redo(editor: &Entity<Editor>, cx: &mut VisualTestContext) {
    editor.update(cx, |editor, cx| editor.edit(|state| state.redo(), cx));
    cx.run_until_parked();
}

pub fn undo_depth(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> usize {
    editor.read_with(cx, |editor, _| editor.state().undo_depth())
}
