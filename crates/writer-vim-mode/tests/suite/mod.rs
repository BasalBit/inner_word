//! Shared helpers: a window with an editor, its Vim mode and the two views.

mod integration;
mod measure;
mod modes;
mod mouse;
mod platform_input;
mod rows;
mod seams;
mod search;
mod typing;

use std::{cell::RefCell, rc::Rc};

use gpui::{
    Context, Entity, IntoElement, Render, TestAppContext, VisualTestContext, Window, div,
    prelude::*,
};
use writer_core::{Block, Document, Position, Selection};
use writer_editor::{CaretStyle, Editor, EditorConfig};
use writer_ui::theme::{self, ThemeMode};
use writer_vim_mode::{Mode, VimCommandLine, VimEvent, VimMode, VimStatus, VimStatusItem};

pub fn setup(cx: &mut TestAppContext) {
    cx.update(|cx| {
        theme::init(ThemeMode::Light, cx);
        writer_ui::init(cx);
        writer_editor::init(cx);
        writer_vim_mode::init(cx);
    });
}

/// The root view: the editor with the command line under it and the status
/// item below, as the app shell mounts them.
pub struct Shell {
    pub editor: Entity<Editor>,
    pub vim: Entity<VimMode>,
    pub status: Entity<VimStatusItem>,
    pub command_line: Entity<VimCommandLine>,
}

impl Render for Shell {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(div().flex_1().min_h_0().child(self.editor.clone()))
            .child(self.command_line.clone())
            .child(div().flex_none().child(self.status.clone()))
            // Draws an open context menu (in the app the toast layer does).
            .child(writer_ui::ContextMenuLayer)
    }
}

/// Handles to what a test works with.
#[derive(Clone)]
pub struct Rig {
    pub editor: Entity<Editor>,
    pub vim: Entity<VimMode>,
    pub command_line: Entity<VimCommandLine>,
    pub status_item: Entity<VimStatusItem>,
    pub events: Rc<RefCell<Vec<VimEvent>>>,
}

/// Opens a focused editor with Vim mode created but detached.
pub fn open_detached(document: Document, cx: &mut TestAppContext) -> (Rig, &mut VisualTestContext) {
    let (shell, cx) = cx.add_window_view(|window, cx| {
        let editor = cx.new(|cx| Editor::new(document, EditorConfig::full(), window, cx));
        let vim = cx.new(|cx| VimMode::new(editor.clone(), cx));
        let status = cx.new(|cx| VimStatusItem::new(vim.clone(), cx));
        let command_line = cx.new(|cx| VimCommandLine::new(vim.clone(), cx));
        editor.read(cx).focus(window);
        Shell {
            editor,
            vim,
            status,
            command_line,
        }
    });
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    let events = Rc::new(RefCell::new(Vec::new()));
    let rig = shell.read_with(cx, |shell, _| Rig {
        editor: shell.editor.clone(),
        vim: shell.vim.clone(),
        command_line: shell.command_line.clone(),
        status_item: shell.status.clone(),
        events: events.clone(),
    });
    cx.update(|_, cx| {
        cx.subscribe(&rig.vim, {
            let events = events.clone();
            move |_, event: &VimEvent, _| events.borrow_mut().push(event.clone())
        })
        .detach();
    });
    (rig, cx)
}

/// Opens a focused editor with Vim mode on.
pub fn open(document: Document, cx: &mut TestAppContext) -> (Rig, &mut VisualTestContext) {
    let (rig, cx) = open_detached(document, cx);
    rig.vim.update(cx, |vim, cx| vim.attach(cx));
    cx.run_until_parked();
    (rig, cx)
}

/// A document of paragraphs.
pub fn doc(lines: &[&str]) -> Document {
    Document::from_blocks(lines.iter().map(|line| Block::paragraph(line)).collect())
}

impl Rig {
    pub fn text(&self, cx: &mut VisualTestContext) -> String {
        self.editor
            .read_with(cx, |editor, _| editor.document().plain_text())
    }

    pub fn selection(&self, cx: &mut VisualTestContext) -> Selection {
        self.editor.read_with(cx, |editor, _| editor.selection())
    }

    /// The head of the editor's selection.
    pub fn head(&self, cx: &mut VisualTestContext) -> Position {
        self.selection(cx).head
    }

    /// The Vim cursor (differs from the head in Visual mode).
    pub fn cursor(&self, cx: &mut VisualTestContext) -> Position {
        let editor = self.editor.clone();
        self.vim
            .read_with(cx, |vim, cx| vim.engine().caret(editor.read(cx).state()))
    }

    /// Where the editor was told to draw its caret instead of at the head
    /// of the selection.
    pub fn caret_override(&self, cx: &mut VisualTestContext) -> Option<Position> {
        self.editor
            .read_with(cx, |editor, _| editor.caret_override())
    }

    /// Where the editor draws its caret.
    pub fn drawn_caret(&self, cx: &mut VisualTestContext) -> Position {
        self.editor
            .read_with(cx, |editor, _| editor.caret_position())
    }

    pub fn selected_text(&self, cx: &mut VisualTestContext) -> String {
        self.editor
            .read_with(cx, |editor, _| editor.state().selected_text())
    }

    pub fn status(&self, cx: &mut VisualTestContext) -> VimStatus {
        self.vim.read_with(cx, |vim, _| vim.status().clone())
    }

    pub fn mode(&self, cx: &mut VisualTestContext) -> Mode {
        self.vim.read_with(cx, |vim, _| vim.mode())
    }

    pub fn caret_style(&self, cx: &mut VisualTestContext) -> CaretStyle {
        self.editor
            .read_with(cx, |editor, _| editor.config().caret_style)
    }

    pub fn text_input(&self, cx: &mut VisualTestContext) -> bool {
        self.editor
            .read_with(cx, |editor, _| editor.text_input_enabled())
    }

    /// The `vim_mode` entry of the editor's key context.
    pub fn context_mode(&self, cx: &mut VisualTestContext) -> Option<String> {
        self.editor.read_with(cx, |editor, _| {
            editor
                .key_context()
                .get(writer_vim_mode::KEY_CONTEXT_MODE)
                .map(|value| value.to_string())
        })
    }

    /// The ranges of an editor highlight layer as `(block, start, end)`.
    pub fn layer(&self, name: &str, cx: &mut VisualTestContext) -> Vec<(usize, usize, usize)> {
        self.editor.read_with(cx, |editor, _| {
            editor
                .highlights(name)
                .iter()
                .map(|range| (range.start.block, range.start.offset, range.end.offset))
                .collect()
        })
    }

    pub fn fallbacks(&self, cx: &mut VisualTestContext) -> usize {
        self.vim.read_with(cx, |vim, _| vim.layout_fallbacks())
    }

    /// The events since the last call.
    pub fn take_events(&self) -> Vec<VimEvent> {
        std::mem::take(&mut self.events.borrow_mut())
    }

    /// Puts the caret somewhere from outside the engine, as the app would.
    pub fn set_caret(&self, position: Position, cx: &mut VisualTestContext) {
        self.editor.update(cx, |editor, cx| {
            editor.edit(|state| state.set_caret(position), cx);
        });
        cx.run_until_parked();
    }
}

pub fn at(block: usize, offset: usize) -> Position {
    Position::new(block, offset)
}
