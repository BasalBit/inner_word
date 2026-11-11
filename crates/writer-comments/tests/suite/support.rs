//! Shared fixture: a window with an editor and the comments panel side by
//! side, mounted the way the crate docs tell the app to: the panel in a
//! container of a fixed width that is shown while the panel is open. No
//! comment action has a listener here; they reach the panel through the
//! global listeners that `writer_comments::init` installs.

use std::{cell::Cell, rc::Rc};

use gpui::{
    Context, Entity, IntoElement, Pixels, Render, TestAppContext, VisualTestContext, Window, div,
    prelude::*, px,
};
use writer_comments::{CommentsEvent, CommentsPanel};
use writer_core::{Block, CommentId, Document, Position, Selection, TextRange};
use writer_editor::{Editor, EditorConfig};
use writer_ui::theme::{self, ThemeMode};

/// The fixed "now" of the test clock (14 November 2023, in milliseconds).
pub const NOW: i64 = 1_700_000_000_000;
pub const AUTHOR: &str = "Ada";
/// Width of the container the workspace gives the panel.
pub const SLOT_WIDTH: Pixels = px(320.);

pub struct Workspace {
    pub editor: Entity<Editor>,
    pub panel: Entity<CommentsPanel>,
    pub events: Vec<CommentsEvent>,
    /// Whether the panel's container is shown (follows `OpenChanged`).
    pub panel_open: bool,
}

impl Render for Workspace {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context("Workspace")
            .flex()
            .size_full()
            .child(div().flex_1().min_w_0().h_full().child(self.editor.clone()))
            .when(self.panel_open, |root| {
                root.child(
                    div()
                        .debug_selector(|| "panel-slot".into())
                        .w(SLOT_WIDTH)
                        .h_full()
                        .flex_none()
                        .child(self.panel.clone()),
                )
            })
    }
}

pub struct Fixture {
    pub workspace: Entity<Workspace>,
    pub editor: Entity<Editor>,
    pub panel: Entity<CommentsPanel>,
    pub clock: Rc<Cell<i64>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Editor,
    List,
    Composer,
    Elsewhere,
}

pub fn setup(cx: &mut TestAppContext) {
    cx.update(|cx| {
        theme::init(ThemeMode::Light, cx);
        writer_ui::init(cx);
        writer_editor::init(cx);
        writer_comments::init(cx);
    });
}

/// Three paragraphs:
///
/// ```text
/// 0: The quick brown fox jumps over the lazy dog.
/// 1: A second paragraph with more words in it.
/// 2: The third and last paragraph.
/// ```
pub fn sample() -> Document {
    Document::from_blocks(vec![
        Block::paragraph("The quick brown fox jumps over the lazy dog."),
        Block::paragraph("A second paragraph with more words in it."),
        Block::paragraph("The third and last paragraph."),
    ])
}

/// Opens an active window with a focused editor on `document` and the
/// panel beside it.
pub fn open(document: Document, cx: &mut TestAppContext) -> (Fixture, &mut VisualTestContext) {
    setup(cx);
    open_window(document, EditorConfig::full(), cx)
}

/// [`open`] without the one-time setup and with a choice of editor
/// configuration (a second window, an editor that starts with comments
/// hidden).
pub fn open_window(
    document: Document,
    config: EditorConfig,
    cx: &mut TestAppContext,
) -> (Fixture, &mut VisualTestContext) {
    let clock = Rc::new(Cell::new(NOW));
    let (workspace, cx) = cx.add_window_view({
        let clock = clock.clone();
        move |window, cx| {
            let editor = cx.new(|cx| Editor::new(document, config, window, cx));
            let panel = cx.new(|cx| {
                let mut panel = CommentsPanel::new(editor.clone(), AUTHOR, window, cx);
                panel.set_clock(move || clock.get(), cx);
                panel
            });
            cx.subscribe(
                &panel,
                |this: &mut Workspace, _, event: &CommentsEvent, cx| {
                    if let CommentsEvent::OpenChanged(open) = event {
                        this.panel_open = *open;
                        cx.notify();
                    }
                    this.events.push(event.clone());
                },
            )
            .detach();
            editor.read(cx).focus(window);
            Workspace {
                // A document that already has comments.
                panel_open: panel.read(cx).is_open(),
                editor,
                panel,
                events: Vec::new(),
            }
        }
    });
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    let (editor, panel) = workspace.read_with(cx, |workspace, _| {
        (workspace.editor.clone(), workspace.panel.clone())
    });
    (
        Fixture {
            workspace,
            editor,
            panel,
            clock,
        },
        cx,
    )
}

impl Fixture {
    pub fn focus(&self, cx: &mut VisualTestContext) -> Focus {
        cx.update(|window, cx| {
            let panel = self.panel.read(cx);
            if self.editor.read(cx).is_focused(window) {
                Focus::Editor
            } else if panel.composer().read(cx).is_focused(window) {
                Focus::Composer
            } else if panel.is_list_focused(window) {
                Focus::List
            } else {
                Focus::Elsewhere
            }
        })
    }

    /// Selects `start..end` of block `block` in the editor.
    pub fn select(&self, block: usize, start: usize, end: usize, cx: &mut VisualTestContext) {
        self.editor.update(cx, |editor, cx| {
            editor.edit(
                |state| state.select_range(TextRange::in_block(block, start, end)),
                cx,
            );
        });
        cx.run_until_parked();
    }

    /// Puts a collapsed caret at `block`, `offset`.
    pub fn caret(&self, block: usize, offset: usize, cx: &mut VisualTestContext) {
        self.editor.update(cx, |editor, cx| {
            editor.edit(|state| state.set_caret(Position::new(block, offset)), cx);
        });
        cx.run_until_parked();
    }

    /// Adds a comment through the keyboard: selects the range, opens the
    /// composer, types the body, submits. Returns the new thread's id.
    pub fn comment(
        &self,
        block: usize,
        start: usize,
        end: usize,
        body: &str,
        cx: &mut VisualTestContext,
    ) -> CommentId {
        let before: Vec<CommentId> = self.all_ids(cx);
        self.select(block, start, end, cx);
        cx.simulate_keystrokes("cmd-alt-m");
        assert_eq!(self.focus(cx), Focus::Composer, "composer did not open");
        cx.simulate_input(body);
        cx.simulate_keystrokes("enter");
        let after = self.all_ids(cx);
        let new: Vec<CommentId> = after
            .into_iter()
            .filter(|id| !before.contains(id))
            .collect();
        assert_eq!(new.len(), 1, "expected exactly one new thread");
        new[0]
    }

    /// Ids of every thread the panel knows, in its order.
    pub fn all_ids(&self, cx: &mut VisualTestContext) -> Vec<CommentId> {
        self.panel.read_with(cx, |panel, _| {
            panel.threads().iter().map(|entry| entry.id).collect()
        })
    }

    /// Ids of the threads the panel lists, in display order.
    pub fn listed_ids(&self, cx: &mut VisualTestContext) -> Vec<CommentId> {
        self.panel.read_with(cx, |panel, _| {
            panel.listed_threads().map(|entry| entry.id).collect()
        })
    }

    pub fn active(&self, cx: &mut VisualTestContext) -> Option<CommentId> {
        let in_panel = self.panel.read_with(cx, |panel, _| panel.active_thread());
        let in_editor = self
            .editor
            .read_with(cx, |editor, _| editor.active_comment());
        assert_eq!(
            in_panel, in_editor,
            "panel and editor disagree about the active thread"
        );
        in_panel
    }

    pub fn selection(&self, cx: &mut VisualTestContext) -> Selection {
        self.editor.read_with(cx, |editor, _| editor.selection())
    }

    pub fn comment_count(&self, cx: &mut VisualTestContext) -> usize {
        self.editor
            .read_with(cx, |editor, _| editor.document().comment_count())
    }

    pub fn events(&self, cx: &mut VisualTestContext) -> Vec<CommentsEvent> {
        self.workspace
            .read_with(cx, |workspace, _| workspace.events.clone())
    }

    /// The values of every `VisibilityChanged` so far.
    pub fn visibility_events(&self, cx: &mut VisualTestContext) -> Vec<bool> {
        self.events(cx)
            .into_iter()
            .filter_map(|event| match event {
                CommentsEvent::VisibilityChanged(visible) => Some(visible),
                _ => None,
            })
            .collect()
    }

    /// The values of every `OpenChanged` so far.
    pub fn open_events(&self, cx: &mut VisualTestContext) -> Vec<bool> {
        self.events(cx)
            .into_iter()
            .filter_map(|event| match event {
                CommentsEvent::OpenChanged(open) => Some(open),
                _ => None,
            })
            .collect()
    }

    /// The text in the composer.
    pub fn composer_text(&self, cx: &mut VisualTestContext) -> String {
        self.panel
            .read_with(cx, |panel, cx| panel.composer().read(cx).plain_text())
    }
}
