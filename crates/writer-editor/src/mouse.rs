//! Mouse input.
//!
//! - click places the caret, shift-click extends the selection;
//! - dragging selects, by characters, or by words after a double click and
//!   by blocks after a triple click, and scrolls when the pointer leaves
//!   the viewport;
//! - clicking a to-do checkbox toggles it without moving the caret;
//! - `cmd`-click opens a link;
//! - the wheel and the trackpad scroll, and the overlay scroll bar can be
//!   dragged.
//!
//! The element registers window-level listeners in paint (they live for one
//! frame) and forwards the events here. The mouse is never required: every
//! one of these has a keyboard equivalent.

use std::{ops::Range, time::Duration};

use gpui::{Context, MouseDownEvent, MouseMoveEvent, Pixels, Point, Window, point, px};
use writer_core::{Position, Selection, TextRange, text};

use crate::{editor::Editor, element::SCROLLBAR_ZONE, scroll::Autoscroll};

/// How often a drag outside the viewport scrolls.
const DRAG_SCROLL_INTERVAL: Duration = Duration::from_millis(30);

/// What a drag extends the selection by.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Drag {
    /// From the anchor to the pointer.
    Characters,
    /// By words, keeping the word that was double-clicked.
    Words(TextRange),
    /// By blocks, keeping the blocks that were triple-clicked.
    Blocks(Range<usize>),
}

impl Editor {
    /// A drag selection or a scroll bar drag is in progress.
    pub(crate) fn mouse_busy(&self) -> bool {
        self.drag.is_some() || self.scrollbar.drag.is_some()
    }

    /// Something depends on where the pointer is: the scroll bar strip or a
    /// link cursor.
    pub(crate) fn has_hover_state(&self) -> bool {
        self.scrollbar.hovered || self.link_hover
    }

    /// A left mouse button press inside the editor.
    pub(crate) fn mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle);
        self.restart_blink(cx);

        // The scroll bar strip: grab the thumb, or jump to the click.
        if let Some(geometry) = self.scrollbar.geometry
            && !self.config.mode.is_compact()
            && event.position.x >= geometry.track.right() + px(3.) - SCROLLBAR_ZONE
        {
            let grab = if geometry.thumb.contains(&event.position) {
                event.position.y - geometry.thumb.top()
            } else {
                geometry.thumb.size.height / 2.
            };
            self.scrollbar.drag = Some(grab);
            self.drag_scrollbar(event.position, cx);
            return;
        }

        // A to-do checkbox: toggle it, leave the caret alone.
        if let Some(hit) = self
            .checkbox_hits
            .iter()
            .find(|hit| hit.bounds.contains(&event.position))
            .copied()
        {
            if !self.config.read_only {
                self.end_composition();
                self.edit_with(
                    Autoscroll::None,
                    |state| state.set_todo_checked(hit.block, !hit.checked),
                    cx,
                );
                self.state.break_undo_group();
            }
            return;
        }

        // cmd-click on a link opens it.
        if event.modifiers.platform
            && let Some(url) = self.link_at_point(event.position)
        {
            cx.open_url(&url);
            return;
        }

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
        self.drag = Some(match clicks {
            2 => Drag::Words(self.state.selection().range()),
            3 => Drag::Blocks(position.block..position.block + 1),
            _ => Drag::Characters,
        });
        self.last_mouse = event.position;
        cx.notify();
    }

    /// The mouse moved with the left button held after a press in the
    /// editor.
    pub(crate) fn mouse_dragged(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if self.scrollbar.drag.is_some() {
            self.drag_scrollbar(event.position, cx);
            return;
        }
        if self.drag.is_none() {
            return;
        }
        self.last_mouse = event.position;
        self.extend_drag(cx);
        self.update_drag_scroll(cx);
    }

    /// The mouse moved without a button held.
    pub(crate) fn mouse_moved(
        &mut self,
        event: &MouseMoveEvent,
        hovered: bool,
        cx: &mut Context<Self>,
    ) {
        let over_scrollbar = hovered
            && !self.config.mode.is_compact()
            && self.scrollbar.geometry.is_some_and(|geometry| {
                event.position.x >= geometry.track.right() + px(3.) - SCROLLBAR_ZONE
            });
        let over_link = hovered
            && !over_scrollbar
            && event.modifiers.platform
            && self.link_at_point(event.position).is_some();
        if over_scrollbar != self.scrollbar.hovered || over_link != self.link_hover {
            self.scrollbar.hovered = over_scrollbar;
            self.link_hover = over_link;
            cx.notify();
        }
    }

    /// The left button was released (anywhere).
    pub(crate) fn mouse_up(&mut self, cx: &mut Context<Self>) {
        if self.mouse_busy() {
            self.drag = None;
            self.drag_scroll = None;
            self.drag_scrolling = false;
            if self.scrollbar.drag.take().is_some() {
                self.flash_scrollbar(cx);
            }
            cx.notify();
        }
    }

    /// The scroll wheel or trackpad moved over the editor.
    pub(crate) fn scroll_wheel(&mut self, delta_y: Pixels, cx: &mut Context<Self>) {
        // A positive wheel delta moves the content down (towards the top).
        // Whole half pixels keep rules and text on device pixels.
        let delta = px((f32::from(delta_y) * 2.).round() / 2.);
        self.scroll_by_pixels(-delta, cx);
    }

    /// The URL of the link under a window point.
    pub(crate) fn link_at_point(&self, point: Point<Pixels>) -> Option<String> {
        let position = self.position_under_point(point)?;
        self.state
            .document()
            .block(position.block)?
            .attrs_at(position.offset)?
            .link
            .clone()
    }

    /// Moves the scroll bar thumb under the pointer.
    fn drag_scrollbar(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let (Some(geometry), Some(grab)) = (self.scrollbar.geometry, self.scrollbar.drag) else {
            return;
        };
        let travel = geometry.track.size.height - geometry.thumb.size.height;
        if travel <= px(0.) {
            return;
        }
        let ratio = ((position.y - grab - geometry.track.top()) / travel).clamp(0., 1.);
        self.scroll_to_pixels(geometry.max_scroll * ratio, cx);
    }

    /// Extends the drag selection to the last pointer position.
    fn extend_drag(&mut self, cx: &mut Context<Self>) {
        let (Some(drag), Some(viewport)) = (self.drag.clone(), self.viewport_bounds()) else {
            return;
        };
        // Outside the viewport the selection follows the edge; the
        // autoscroll brings more text under it.
        let pointer = point(
            self.last_mouse.x,
            self.last_mouse
                .y
                .clamp(viewport.top(), viewport.bottom() - px(1.)),
        );
        let Some(position) = self.position_at_point(pointer) else {
            return;
        };
        let document = self.state.document();
        let selection = match drag {
            Drag::Characters => Selection::new(self.state.selection().anchor, position),
            Drag::Words(origin) => {
                let word =
                    document
                        .block(position.block)
                        .map_or(TextRange::empty(position), |block| {
                            let range = text::segment_at(block.text(), position.offset);
                            TextRange::in_block(position.block, range.start, range.end)
                        });
                if position < origin.start {
                    Selection::new(origin.end, word.start)
                } else {
                    Selection::new(origin.start, word.end.max(origin.end))
                }
            }
            Drag::Blocks(origin) => {
                let block_start = |index: usize| Position::new(index, 0);
                let block_end = |index: usize| document.block_range(index).end;
                if position.block < origin.start {
                    Selection::new(block_end(origin.end - 1), block_start(position.block))
                } else {
                    Selection::new(
                        block_start(origin.start),
                        block_end(position.block.max(origin.end - 1)),
                    )
                }
            }
        };
        if selection != self.state.selection() {
            self.edit_with(Autoscroll::None, |state| state.set_selection(selection), cx);
        }
    }

    /// Starts or stops scrolling while the pointer is dragged outside the
    /// viewport.
    fn update_drag_scroll(&mut self, cx: &mut Context<Self>) {
        if self.drag_scroll_step().is_none() {
            self.drag_scroll = None;
            self.drag_scrolling = false;
            return;
        }
        if self.drag_scrolling {
            return;
        }
        self.drag_scrolling = true;
        self.drag_scroll = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(DRAG_SCROLL_INTERVAL).await;
                let scrolling = this.update(cx, |editor, cx| {
                    let Some(step) = editor.drag_scroll_step().filter(|_| editor.drag.is_some())
                    else {
                        editor.drag_scrolling = false;
                        return false;
                    };
                    editor.scroll_by_pixels(step, cx);
                    editor.extend_drag(cx);
                    true
                });
                if !matches!(scrolling, Ok(true)) {
                    break;
                }
            }
        }));
    }

    /// How far one tick of drag scrolling moves: more the further the
    /// pointer is outside the viewport. `None` inside it.
    fn drag_scroll_step(&self) -> Option<Pixels> {
        let viewport = self.viewport_bounds()?;
        let y = self.last_mouse.y;
        let distance = if y < viewport.top() {
            y - viewport.top()
        } else if y > viewport.bottom() {
            y - viewport.bottom()
        } else {
            return None;
        };
        let speed = (f32::from(distance.abs()) * 0.4).clamp(4., 80.);
        Some(px(speed.round() * f32::from(distance).signum()))
    }
}
