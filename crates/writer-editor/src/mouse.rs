//! Mouse input: click to place the caret, shift-click and drag to select,
//! double-click a word, triple-click a block, wheel to scroll.
//!
//! The element registers window-level listeners in paint (they live for one
//! frame) and forwards the events here. The mouse is never required: every
//! one of these has a keyboard equivalent.
//!
//! Status: skeleton for the rest. Not done yet: clicking a to-do checkbox
//! (`EditorState::set_todo_checked`), cmd-click on a link, autoscroll while
//! dragging past the viewport edge, word- and block-wise drag after a
//! double or triple click.

use gpui::{Context, MouseDownEvent, MouseMoveEvent, Pixels, Window};
use writer_core::Selection;

use crate::{editor::Editor, scroll::Autoscroll};

impl Editor {
    /// A left mouse button press inside the editor.
    pub(crate) fn mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle);
        let Some(position) = self.position_at_point(event.position) else {
            return;
        };
        self.end_composition();
        let anchor = self.state.selection().anchor;
        let extend = event.modifiers.shift;
        let clicks = event.click_count;
        self.edit_with(
            Autoscroll::Fit,
            |state| match clicks {
                2 => state.select_word_at(position),
                3 => state.select_block(position.block),
                _ if extend => state.set_selection(Selection::new(anchor, position)),
                _ => state.set_caret(position),
            },
            cx,
        );
        self.state.break_undo_group();
        self.selecting = clicks <= 1;
        cx.notify();
    }

    /// The mouse moved with the left button held after a press in the
    /// editor: extend the selection to the pointer.
    pub(crate) fn mouse_dragged(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if !self.selecting {
            return;
        }
        let Some(position) = self.position_at_point(event.position) else {
            return;
        };
        let anchor = self.state.selection().anchor;
        self.edit_with(
            Autoscroll::Fit,
            |state| state.set_selection(Selection::new(anchor, position)),
            cx,
        );
    }

    /// The left button was released (anywhere).
    pub(crate) fn mouse_up(&mut self, cx: &mut Context<Self>) {
        if self.selecting {
            self.selecting = false;
            cx.notify();
        }
    }

    /// The scroll wheel or trackpad moved over the editor.
    pub(crate) fn scroll_wheel(&mut self, delta_y: Pixels, cx: &mut Context<Self>) {
        // A positive wheel delta moves the content down (towards the top).
        self.scroll_by_pixels(-delta_y, cx);
    }
}
