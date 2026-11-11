//! Shared helpers: a focused editor wrapped in `EditorTools`, in an active
//! test window.

use gpui::{AppContext, Entity, TestAppContext, VisualTestContext};
use writer_core::{Block, Document, Position, Selection, TextRange};
use writer_editor::{Editor, EditorConfig};
use writer_editor_tools::{EditorTools, MenuRow, ToolKind};
use writer_ui::theme::{self, ThemeMode};

pub fn setup(cx: &mut TestAppContext) {
    cx.update(|cx| {
        theme::init(ThemeMode::Light, cx);
        writer_ui::init(cx);
        writer_editor::init(cx);
        writer_editor_tools::init(cx);
    });
}

pub fn open(
    document: Document,
    cx: &mut TestAppContext,
) -> (Entity<EditorTools>, Entity<Editor>, &mut VisualTestContext) {
    let (tools, cx) = cx.add_window_view(|window, cx| {
        let editor = cx.new(|cx| Editor::new(document, EditorConfig::full(), window, cx));
        editor.read(cx).focus(window);
        EditorTools::new(editor, window, cx)
    });
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    let editor = tools.read_with(cx, |tools, _| tools.editor().clone());
    (tools, editor, cx)
}

pub fn paragraphs(lines: &[&str]) -> Document {
    Document::from_blocks(lines.iter().map(|line| Block::paragraph(line)).collect())
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

pub fn active(tools: &Entity<EditorTools>, cx: &mut VisualTestContext) -> Option<ToolKind> {
    tools.read_with(cx, |tools, _| tools.active_tool())
}

pub fn rows(tools: &Entity<EditorTools>, cx: &mut VisualTestContext) -> Vec<MenuRow> {
    tools.read_with(cx, |tools, cx| tools.menu_rows(cx))
}

/// Labels of the selectable rows of the open menu.
pub fn labels(tools: &Entity<EditorTools>, cx: &mut VisualTestContext) -> Vec<String> {
    rows(tools, cx)
        .into_iter()
        .filter(|row| !row.header)
        .map(|row| row.label)
        .collect()
}

pub fn selected(tools: &Entity<EditorTools>, cx: &mut VisualTestContext) -> Option<String> {
    tools.read_with(cx, |tools, cx| tools.selected_menu_label(cx))
}

pub fn row(tools: &Entity<EditorTools>, label: &str, cx: &mut VisualTestContext) -> MenuRow {
    rows(tools, cx)
        .into_iter()
        .find(|row| row.label == label)
        .unwrap_or_else(|| panic!("no row {label:?}"))
}

pub fn editor_focused(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> bool {
    let editor = editor.clone();
    cx.update(|window, cx| editor.read(cx).is_focused(window))
}
