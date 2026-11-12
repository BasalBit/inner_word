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

// ---------------------------------------------------------------------------
// The quick chat in a window, as the app mounts it
// ---------------------------------------------------------------------------

use std::sync::Arc;

use gpui::{
    AppContext, Bounds, Context, IntoElement, Modifiers, MouseButton, Pixels, Point, Render,
    Window, div, prelude::*, px,
};
use writer_assistant::{FakeBackend, Message, MessageId, Role};
use writer_assistant_quick::{QuickChat, QuickChatEvent};
use writer_editor_tools::EditorTools;

/// What is beside the editor in the test window: a strip at the left and a
/// band on top, so the editor's viewport is not the window.
pub const SIDE: Pixels = px(180.);
pub const TOP: Pixels = px(40.);

/// A root view like the app's: the editor with its tools in the middle,
/// the quick chat mounted after it, the layer that draws context menus.
pub struct Host {
    pub tools: Entity<EditorTools>,
    pub quick: Entity<QuickChat>,
    pub events: Vec<QuickChatEvent>,
    /// Somewhere else the keyboard can go.
    pub elsewhere: gpui::FocusHandle,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .child(
                div()
                    .debug_selector(|| "side-strip".into())
                    .track_focus(&self.elsewhere)
                    .w(SIDE)
                    .h_full()
                    .flex_none(),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .debug_selector(|| "top-band".into())
                            .h(TOP)
                            .flex_none(),
                    )
                    .child(div().flex_1().min_h_0().child(self.tools.clone())),
            )
            .child(self.quick.clone())
            .child(writer_ui::ContextMenuLayer)
    }
}

pub struct Fixture {
    pub host: Entity<Host>,
    pub tools: Entity<EditorTools>,
    pub editor: Entity<Editor>,
    pub quick: Entity<QuickChat>,
    pub fake: FakeBackend,
}

pub fn setup_all(cx: &mut TestAppContext) {
    cx.update(|cx| {
        theme::init(ThemeMode::Light, cx);
        writer_ui::init(cx);
        writer_editor::init(cx);
        writer_editor_tools::init(cx);
        writer_vim_mode::init(cx);
        writer_assistant_ui::init(cx);
        writer_assistant_quick::init(cx);
    });
}

/// Opens an active window with `document` in an editor that has the
/// keyboard, and its quick chat on a new fake backend.
pub fn open_quick(
    document: Document,
    cx: &mut TestAppContext,
) -> (Fixture, &mut VisualTestContext) {
    setup_all(cx);
    open_quick_on(FakeBackend::new(), document, cx)
}

pub fn open_quick_on(
    fake: FakeBackend,
    document: Document,
    cx: &mut TestAppContext,
) -> (Fixture, &mut VisualTestContext) {
    let backend: Arc<dyn writer_assistant::Backend> = Arc::new(fake.clone());
    let (host, cx) = cx.add_window_view(|window, cx| {
        let editor = cx.new(|cx| Editor::new(document, EditorConfig::full(), window, cx));
        editor.read(cx).focus(window);
        let tools = cx.new(|cx| EditorTools::new(editor.clone(), window, cx));
        let quick = cx.new(|cx| QuickChat::new(editor, backend, window, cx));
        cx.subscribe(&quick, |host: &mut Host, _, event: &QuickChatEvent, _| {
            host.events.push(event.clone())
        })
        .detach();
        Host {
            tools,
            quick,
            events: Vec::new(),
            elsewhere: cx.focus_handle(),
        }
    });
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    let (tools, quick) = host.read_with(cx, |host, _| (host.tools.clone(), host.quick.clone()));
    let editor = tools.read_with(cx, |tools, _| tools.editor().clone());
    (
        Fixture {
            host,
            tools,
            editor,
            quick,
            fake,
        },
        cx,
    )
}

impl Fixture {
    /// What the app's key does.
    pub fn toggle(&self, cx: &mut VisualTestContext) {
        self.quick
            .update_in(cx, |quick, window, cx| quick.toggle(window, cx));
        cx.run_until_parked();
    }

    pub fn is_open(&self, cx: &mut VisualTestContext) -> bool {
        self.quick.read_with(cx, |quick, _| quick.is_open())
    }

    /// Types a question into whatever has the keyboard and presses `enter`.
    pub fn send(&self, text: &str, cx: &mut VisualTestContext) {
        cx.simulate_input(text);
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
    }

    /// Lets the held events of the fake agent through, one at a time,
    /// until the first piece of the answer that is being written is
    /// there.
    pub fn first_piece(&self, cx: &mut VisualTestContext) {
        for _ in 0..8 {
            let arrived = self
                .messages(cx)
                .last()
                .is_some_and(|message| message.is_streaming() && !message.text.is_empty());
            if arrived {
                return;
            }
            assert_eq!(self.fake.advance(1), 1, "the fake agent holds nothing more");
            cx.run_until_parked();
        }
        panic!("no piece of an answer arrived");
    }

    pub fn messages(&self, cx: &mut VisualTestContext) -> Vec<Message> {
        self.quick.read_with(cx, |quick, _| {
            quick
                .transcript()
                .map(|transcript| transcript.messages.clone())
                .unwrap_or_default()
        })
    }

    /// The texts of the answers, in order.
    pub fn answers(&self, cx: &mut VisualTestContext) -> Vec<String> {
        self.messages(cx)
            .into_iter()
            .filter(|message| message.role == Role::Assistant)
            .map(|message| message.text)
            .collect()
    }

    pub fn last_answer_id(&self, cx: &mut VisualTestContext) -> MessageId {
        self.messages(cx)
            .into_iter()
            .rev()
            .find(|message| message.role == Role::Assistant)
            .expect("an answer")
            .id
    }

    /// What the editor of an answer shows.
    pub fn shown_answer(&self, id: MessageId, cx: &mut VisualTestContext) -> String {
        self.quick.read_with(cx, |quick, cx| {
            quick
                .messages()
                .and_then(|list| list.read(cx).answer_editor(id).cloned())
                .map(|editor| editor.read(cx).plain_text())
                .unwrap_or_default()
        })
    }

    pub fn draft(&self, cx: &mut VisualTestContext) -> String {
        self.quick.read_with(cx, |quick, cx| {
            quick
                .composer()
                .map(|composer| composer.read(cx).text(cx))
                .unwrap_or_default()
        })
    }

    pub fn events(&self, cx: &mut VisualTestContext) -> Vec<QuickChatEvent> {
        self.host.read_with(cx, |host, _| host.events.clone())
    }

    pub fn popover(&self, cx: &mut VisualTestContext) -> Bounds<Pixels> {
        self.quick
            .read_with(cx, |quick, _| quick.bounds())
            .expect("the popover is drawn")
    }

    pub fn viewport(&self, cx: &mut VisualTestContext) -> Bounds<Pixels> {
        self.editor
            .read_with(cx, |editor, _| editor.viewport_bounds())
            .expect("the editor has a layout")
    }

    pub fn editor_focused(&self, cx: &mut VisualTestContext) -> bool {
        let editor = self.editor.clone();
        cx.update(|window, cx| editor.read(cx).is_focused(window))
    }

    /// Whether the keyboard is in the composer of the popover.
    pub fn composer_focused(&self, cx: &mut VisualTestContext) -> bool {
        let quick = self.quick.clone();
        cx.update(|window, cx| {
            quick
                .read(cx)
                .composer()
                .is_some_and(|composer| composer.read(cx).is_focused(window, cx))
        })
    }

    pub fn has_keyboard(&self, cx: &mut VisualTestContext) -> bool {
        let quick = self.quick.clone();
        cx.update(|window, cx| quick.read(cx).has_keyboard(window, cx))
    }
}

pub fn try_bounds(
    selector: impl Into<String>,
    cx: &mut VisualTestContext,
) -> Option<Bounds<Pixels>> {
    // `debug_bounds` wants a `&'static str`; a test may leak a few names.
    let selector: &'static str = Box::leak(selector.into().into_boxed_str());
    cx.debug_bounds(selector)
}

pub fn bounds(selector: impl Into<String>, cx: &mut VisualTestContext) -> Bounds<Pixels> {
    let selector = selector.into();
    try_bounds(selector.clone(), cx).unwrap_or_else(|| panic!("nothing was painted for {selector}"))
}

/// Moves the pointer to `position` (no button held).
pub fn move_to(position: Point<Pixels>, cx: &mut VisualTestContext) {
    cx.simulate_mouse_move(position, None, Modifiers::none());
    cx.run_until_parked();
}

/// Moves the pointer onto the element and clicks it, as a hand does.
pub fn click(selector: impl Into<String>, cx: &mut VisualTestContext) {
    let center = bounds(selector, cx).center();
    click_at(center, cx);
}

pub fn click_at(position: Point<Pixels>, cx: &mut VisualTestContext) {
    move_to(position, cx);
    cx.simulate_click(position, Modifiers::none());
    cx.run_until_parked();
}

pub fn press_at(position: Point<Pixels>, cx: &mut VisualTestContext) {
    move_to(position, cx);
    cx.simulate_mouse_down(position, MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
    cx.simulate_mouse_up(position, MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
}

/// The labels of the open context menu ("-" for a separator).
pub fn menu_labels(cx: &mut VisualTestContext) -> Vec<String> {
    cx.update(|window, cx| {
        writer_ui::ContextMenu::current(window, cx)
            .map(|menu| menu.read(cx).labels())
            .unwrap_or_default()
    })
}

pub fn menu_is_open(cx: &mut VisualTestContext) -> bool {
    cx.update(|window, cx| writer_ui::ContextMenu::is_open(window, cx))
}

/// Chooses an item of the open context menu, as a click on it does.
pub fn choose(title: &str, cx: &mut VisualTestContext) {
    let chosen = cx.update(|window, cx| writer_ui::ContextMenu::choose(title, window, cx));
    assert!(chosen, "the menu has no item {title:?}");
    cx.run_until_parked();
}
