//! Mouse input.
//!
//! - click places the caret, shift-click extends the selection;
//! - dragging selects, by characters, or by words after a double click and
//!   by blocks after a triple click, and scrolls when the pointer leaves
//!   the viewport;
//! - clicking a to-do checkbox toggles it without moving the caret;
//! - `cmd`-click opens a link;
//! - a right-click (the secondary button, or `ctrl`-click) adjusts the
//!   selection as a macOS text view does and asks for the context menu
//!   ([`EditorEvent::ContextMenuRequested`]); `ctrl-enter` asks for the
//!   same menu at the caret;
//! - the wheel and the trackpad scroll, and the overlay scroll bar can be
//!   dragged.
//!
//! The element registers window-level listeners in paint (they live for one
//! frame) and forwards the events here. The mouse is never required: every
//! one of these has a keyboard equivalent. Nor is the keyboard: what has no
//! pointer gesture of its own is in the context menu.

use std::{ops::Range, time::Duration};

use gpui::{Context, MouseDownEvent, MouseMoveEvent, Pixels, Point, Window, point, px};
use writer_core::{Position, Selection, TextRange, text};

use crate::{
    editor::Editor,
    element::SCROLLBAR_ZONE,
    events::{ContextMenuRequest, EditorEvent},
    scroll::{Autoscroll, PendingScroll},
};

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

    /// Something depends on where the pointer is: the scroll bar strip, a
    /// link cursor or a link tooltip.
    pub(crate) fn has_hover_state(&self) -> bool {
        self.scrollbar.hovered || self.link_hover || self.link_tip.is_some()
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
        self.hide_link_tip(cx);

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

        let Some((position, upstream)) = self.caret_at_point(event.position) else {
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
        // A click past the end of a wrapped row leaves the caret there.
        self.caret_upstream =
            upstream && !matches!(clicks, 2 | 3) && self.state.selection().head == position;
        self.drag = Some(match clicks {
            2 => Drag::Words(self.state.selection().range()),
            3 => Drag::Blocks(position.block..position.block + 1),
            _ => Drag::Characters,
        });
        self.last_mouse = event.position;
        cx.notify();
    }

    /// A press of the secondary button inside the editor (GPUI turns a
    /// `ctrl`-click into one on macOS; a left press that still carries
    /// `ctrl` is taken the same way). Adjusts the selection as macOS text
    /// views do and asks for the context menu.
    pub(crate) fn context_click(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Not while the left button is selecting or dragging the thumb,
        // and not on the scroll bar strip.
        if self.mouse_busy() {
            return;
        }
        if let Some(geometry) = self.scrollbar.geometry
            && !self.config.mode.is_compact()
            && event.position.x >= geometry.track.right() + px(3.) - SCROLLBAR_ZONE
        {
            return;
        }
        let Some((position, upstream)) = self.caret_at_point(event.position) else {
            return;
        };
        window.focus(&self.focus_handle);
        self.restart_blink(cx);
        self.hide_link_tip(cx);
        self.end_composition();

        let checkbox = self
            .checkbox_hits
            .iter()
            .find(|hit| hit.bounds.contains(&event.position))
            .copied();
        // The character the pointer is on, if it is on one.
        let under = if checkbox.is_some() {
            None
        } else {
            self.position_under_point(event.position)
        };
        let position = match checkbox {
            // The gutter belongs to the item: its start, wherever the
            // nearest row boundary is.
            Some(hit) => Position::new(hit.block, 0),
            None => position,
        };

        let selection = self.state.selection();
        let range = selection.range();
        let inside = !selection.is_collapsed()
            && match under {
                Some(under) => range.start <= under && under < range.end,
                // Not on a character (a margin, the end of a row, between
                // two blocks): inside when the selection runs past it on
                // both sides.
                None => range.start < position && position < range.end,
            };
        if !inside {
            // The word under the pointer, or just the caret.
            let word = under.and_then(|under| {
                let block = self.state.document().block(under.block)?;
                let segment = text::segment_at(block.text(), under.offset);
                block.text()[segment.clone()]
                    .chars()
                    .any(|c| c.is_alphanumeric())
                    .then(|| TextRange::in_block(under.block, segment.start, segment.end))
            });
            self.edit_with(
                Autoscroll::None,
                |state| match word {
                    Some(word) => state.set_selection(Selection::from_range(word)),
                    None => state.set_caret(position),
                },
                cx,
            );
            self.state.break_undo_group();
            self.caret_upstream =
                upstream && word.is_none() && checkbox.is_none() && self.caret_has_affinity();
        }

        let link = if checkbox.is_some() {
            None
        } else {
            self.link_under_point(event.position)
        };
        let document = self.state.document();
        let attrs = under.and_then(|under| document.block(under.block)?.attrs_at(under.offset));
        let comments = attrs
            .map(|attrs| {
                attrs
                    .comments
                    .iter()
                    .copied()
                    .filter(|id| !self.hidden_comments.contains(id))
                    .collect()
            })
            .unwrap_or_default();
        let request = ContextMenuRequest {
            position: event.position,
            document_position: position,
            block: position.block,
            has_selection: !self.state.selection().is_collapsed(),
            link: link.as_ref().map(|info| info.url.clone()),
            link_range: link.map(|info| info.range),
            comments,
            on_checkbox: checkbox.is_some(),
            from_keyboard: false,
            // The pointer has to be on a character of the word.
            spelling_mark: under.filter(|_| checkbox.is_none()).and_then(|under| {
                self.drawn_spelling_mark_at(under)
                    .filter(|mark| under < mark.end)
            }),
        };
        cx.emit(EditorEvent::ContextMenuRequested(request));
        cx.notify();
    }

    /// `ctrl-enter`: asks for the context menu at the caret, which is
    /// brought into view first. The selection stays as it is.
    pub(crate) fn open_context_menu(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.end_composition();
        self.request_scroll(PendingScroll::Position(None, Autoscroll::Fit));
        let position = self.caret_position();
        let Some(caret) = self.caret_bounds() else {
            return;
        };
        // Under the caret's row, and never outside the editor.
        let mut at = point(caret.left(), caret.bottom());
        if let Some(viewport) = self.viewport_bounds() {
            at.x = at.x.clamp(viewport.left(), viewport.right());
            at.y = at.y.clamp(viewport.top(), viewport.bottom());
        }
        let document = self.state.document();
        let link = document.link_at(position);
        let comments = document
            .comments_at(position)
            .into_iter()
            .filter(|id| !self.hidden_comments.contains(id))
            .collect();
        let request = ContextMenuRequest {
            position: at,
            document_position: position,
            block: position.block,
            has_selection: !self.state.selection().is_collapsed(),
            link: link.as_ref().map(|info| info.url.clone()),
            link_range: link.map(|info| info.range),
            comments,
            on_checkbox: false,
            from_keyboard: true,
            spelling_mark: self.drawn_spelling_mark_at(position),
        };
        cx.emit(EditorEvent::ContextMenuRequested(request));
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
        // A popup over the text (a menu, the link field) has the user's
        // attention: no tooltips under it.
        let link = if hovered && !over_scrollbar && self.popup.is_none() {
            self.link_under_point(event.position)
        } else {
            None
        };
        let over_link = link.is_some() && event.modifiers.platform;
        if over_scrollbar != self.scrollbar.hovered || over_link != self.link_hover {
            self.scrollbar.hovered = over_scrollbar;
            self.link_hover = over_link;
            cx.notify();
        }
        self.update_link_tip(link, event.position, cx);
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

    /// The scroll wheel or trackpad moved over the editor: `delta` in
    /// window pixels. Sideways it moves the view only where a sheet is
    /// wider than its room (page view, zoomed in or in a narrow window).
    pub(crate) fn scroll_wheel(&mut self, delta: Point<Pixels>, cx: &mut Context<Self>) {
        // A positive wheel delta moves the content down (towards the top).
        // Whole half pixels keep rules and text on device pixels.
        let whole = |delta: Pixels| px((f32::from(delta) * 2.).round() / 2.);
        self.hide_link_tip(cx);
        if delta.y != px(0.) {
            self.scroll_by_pixels(-whole(delta.y), cx);
        }
        if delta.x != px(0.) {
            self.scroll_sideways_by_pixels(-whole(delta.x), cx);
        }
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
        self.scroll_to_y(geometry.max_scroll * ratio, cx);
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
        let Some((position, upstream)) = self.caret_at_point(pointer) else {
            return;
        };
        let by_characters = drag == Drag::Characters;
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
        let upstream = upstream && by_characters && self.state.selection().head == position;
        if upstream != self.caret_upstream {
            self.caret_upstream = upstream;
            cx.notify();
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
