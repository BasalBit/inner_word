//! Mouse input.
//!
//! - click places the caret, shift-click extends the selection;
//! - dragging selects, by characters, or by words after a double click and
//!   by blocks after a triple click, and scrolls when the pointer leaves
//!   the viewport;
//! - clicking a to-do checkbox toggles it without moving the caret;
//! - a click on a picture selects its image (the caret goes onto its
//!   block); dragging the picture moves the block to another place, and
//!   dragging a corner handle of the selected image makes it larger or
//!   smaller (see "Images" below);
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
//!
//! # Images
//!
//! **Resize.** A press on a handle of the selected image starts it. The
//! width follows the pointer sideways: pulling a handle away from the
//! picture makes it wider, by as much as the pointer moves, and by twice
//! as much for a centred image, which grows on both sides (so the handle
//! stays under the pointer wherever the picture has an edge that can
//! move). The height follows from the aspect ratio. The width is a share
//! of the text column and stays within what a width can be (a twentieth
//! of the column, 24 points, the whole column, and the width at which the
//! picture would be taller than a page); near a quarter, a half, three
//! quarters, the whole column and the picture's original size it snaps to
//! them. While the button is held nothing of the document changes: the
//! layout shows the width as a preview
//! (`DocumentLayout::set_image_preview`), so the text under the picture
//! moves, and no edit, no autosave and no undo step is made. The release
//! stores the width as one undo step.
//!
//! **Move.** A press on a picture selects it and arms a move, which starts
//! once the pointer is 4 pixels away. The picture is dimmed where it is
//! and a line shows the boundary between two blocks nearest to the
//! pointer; the view scrolls at its edges. The release puts the block
//! there as one undo step; at the two boundaries around the picture
//! itself there is no line and nothing changes.
//!
//! **`escape`** cancels either. It is no key binding: for the length of
//! the drag the editor registers one of its own key interceptors
//! (`crate::intercept`), ahead of everyone else's, so the key cannot be
//! given another meaning and does nothing else meanwhile.
//!
//! The image is held by the id of its block. A change from elsewhere that
//! is taken over in the middle of a drag (`Editor::adopt_document`) either
//! still has the block, and the drag goes on, or not, and it ends.

use std::{ops::Range, time::Duration};

use gpui::{Context, MouseDownEvent, MouseMoveEvent, Pixels, Point, Window, point, px};
use writer_core::{Alignment, BlockId, Image, ImageWidth, Position, Selection, TextRange, text};

use crate::{
    editor::Editor,
    element::{HANDLE_SLACK, SCROLLBAR_ZONE},
    events::{ContextMenuRequest, EditorEvent},
    intercept::{InterceptPriority, KeyDisposition},
    layout::style::MIN_IMAGE_WIDTH,
    scroll::{Autoscroll, PendingScroll},
};

/// How often a drag outside the viewport scrolls.
const DRAG_SCROLL_INTERVAL: Duration = Duration::from_millis(30);
/// How far the pointer has to go from where it pressed a picture before
/// the picture is being moved, in window pixels.
const MOVE_THRESHOLD: Pixels = px(4.);
/// A width that is being dragged snaps to a preset when it is this close
/// to it, in window pixels.
const SNAP_DISTANCE: f32 = 6.;
/// The widths a resize with the pointer snaps to, in thousandths of the
/// column (and the image's original size).
const SNAP_WIDTHS: [u16; 4] = [250, 500, 750, 1000];
/// The editor's own interceptor for `escape` during the drag of an image
/// comes before every other one (an open menu, Vim).
const DRAG_PRIORITY: InterceptPriority = InterceptPriority(i32::MAX);

/// The pointer holds a corner handle of an image.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ImageResize {
    pub(crate) block: BlockId,
    /// The handle: 0 to 3 for top left, top right, bottom left, bottom
    /// right.
    pub(crate) corner: usize,
    /// Window x of the press.
    pressed_x: Pixels,
    /// How wide the picture was drawn then, in layout units.
    start_width: Pixels,
    /// A centred image grows on both sides.
    centred: bool,
    /// The pointer has moved since the press.
    moved: bool,
    /// The width that is shown now.
    pub(crate) width: ImageWidth,
}

/// The pointer pressed a picture and may be dragging it elsewhere.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ImageMove {
    pub(crate) block: BlockId,
    pressed_at: Point<Pixels>,
    /// The pointer went far enough for this to be a move.
    pub(crate) started: bool,
    /// The boundary the block would go to: the index of the block after
    /// it. `None` at the two boundaries around the block itself.
    pub(crate) drop: Option<usize>,
}

/// What a drag extends the selection by.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Drag {
    /// From the anchor to the pointer.
    Characters,
    /// By words, keeping the word that was double-clicked.
    Words(TextRange),
    /// By blocks, keeping the blocks that were triple-clicked.
    Blocks(Range<usize>),
    /// Not a selection: a handle of an image is being dragged.
    ResizeImage(ImageResize),
    /// Not a selection: a picture was pressed, and is dragged once the
    /// pointer has moved far enough.
    MoveImage(ImageMove),
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

        // A handle of the selected image: the pointer resizes it.
        if let Some((block, handles)) = self.image_handles
            && let Some(corner) = handles
                .iter()
                .position(|handle| handle.dilate(HANDLE_SLACK).contains(&event.position))
            && self.start_image_resize(block, corner, event.position, cx)
        {
            return;
        }

        // A picture: its image is selected (the caret goes onto its
        // block; shift extends the selection to it as to any place), and
        // dragging it on moves the block. However often it is clicked.
        if let Some(index) = self.picture_at(event.position) {
            self.end_composition();
            let position = Position::new(index, 0);
            let anchor = self.state.selection().anchor;
            let extend = event.modifiers.shift;
            // The picture is under the pointer: nothing scrolls.
            self.edit_with(
                Autoscroll::None,
                |state| {
                    if extend {
                        state.set_selection(Selection::new(anchor, position))
                    } else {
                        state.set_caret(position)
                    }
                },
                cx,
            );
            self.state.break_undo_group();
            self.caret_upstream = false;
            let movable = !extend && !self.config.read_only && !self.config.mode.is_compact();
            self.drag = Some(
                match self.state.document().block(index).filter(|_| movable) {
                    Some(block) => Drag::MoveImage(ImageMove {
                        block: block.id(),
                        pressed_at: event.position,
                        started: false,
                        drop: None,
                    }),
                    None => Drag::Characters,
                },
            );
            self.last_mouse = event.position;
            cx.notify();
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
        // A handle is not followed out of the viewport: the width only
        // goes sideways.
        if !matches!(self.drag, Some(Drag::ResizeImage(_)) | None) {
            self.update_drag_scroll(cx);
        }
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
            // Where a picture that is being moved goes is where the
            // pointer is now, in the document as it is now.
            if matches!(self.drag, Some(Drag::MoveImage(_))) {
                self.move_image_to_pointer(cx);
            }
            let drag = self.drag.take();
            self.drag_escape = None;
            self.drag_scroll = None;
            self.drag_scrolling = false;
            match drag {
                Some(Drag::ResizeImage(resize)) => self.finish_image_resize(resize, cx),
                Some(Drag::MoveImage(moving)) => self.finish_image_move(moving, cx),
                _ => {}
            }
            if self.scrollbar.drag.take().is_some() {
                self.flash_scrollbar(cx);
            }
            cx.notify();
        }
    }

    /// The scroll wheel or trackpad moved over the editor: `delta` in
    /// window pixels. Sideways it moves the view only where a sheet is
    /// wider than its room (page view, zoomed in or in a narrow window);
    /// there `shift` turns a wheel that only goes up and down sideways.
    pub(crate) fn scroll_wheel(
        &mut self,
        delta: Point<Pixels>,
        shift: bool,
        cx: &mut Context<Self>,
    ) {
        // A positive wheel delta moves the content down (towards the top).
        // Whole half pixels keep rules and text on device pixels.
        let whole = |delta: Pixels| px((f32::from(delta) * 2.).round() / 2.);
        self.hide_link_tip(cx);
        let delta = sideways_intent(delta, shift, self.max_horizontal_scroll() > px(0.));
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

    /// Extends the drag selection to the last pointer position (or, when
    /// it is an image that is dragged, lets the image follow it).
    fn extend_drag(&mut self, cx: &mut Context<Self>) {
        match self.drag {
            Some(Drag::ResizeImage(_)) => return self.resize_image_to_pointer(cx),
            Some(Drag::MoveImage(_)) => return self.move_image_to_pointer(cx),
            _ => {}
        }
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
            // Handled above: they select nothing.
            Drag::ResizeImage(_) | Drag::MoveImage(_) => return,
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

// --- The pointer without a pointer ------------------------------------------------

impl Editor {
    /// A press of the left button at a window point, as the element's
    /// mouse listener hands it on. For scripted visual QA (the demo
    /// example, a scripted run of the app): GPUI cannot be handed pointer
    /// events from outside, so a run that wants to look at what the
    /// pointer does (an image being resized or moved) calls what the
    /// pointer calls. Not a seam for consumers.
    #[doc(hidden)]
    pub fn pointer_press(
        &mut self,
        at: Point<Pixels>,
        clicks: usize,
        shift: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let event = MouseDownEvent {
            button: gpui::MouseButton::Left,
            position: at,
            modifiers: gpui::Modifiers {
                shift,
                ..gpui::Modifiers::default()
            },
            click_count: clicks.max(1),
            first_mouse: false,
        };
        self.mouse_down(&event, window, cx);
    }

    /// The pointer moved to a window point with the left button held
    /// (see [`Editor::pointer_press`]).
    #[doc(hidden)]
    pub fn pointer_drag_to(&mut self, at: Point<Pixels>, cx: &mut Context<Self>) {
        let event = MouseMoveEvent {
            position: at,
            pressed_button: Some(gpui::MouseButton::Left),
            modifiers: gpui::Modifiers::default(),
        };
        self.mouse_dragged(&event, cx);
    }

    /// The left button was let go (see [`Editor::pointer_press`]).
    #[doc(hidden)]
    pub fn pointer_release(&mut self, cx: &mut Context<Self>) {
        self.mouse_up(cx);
    }

    /// Files are dragged over the window at a window point, or (`None`)
    /// left it again (see [`Editor::pointer_press`]). What is dropped is
    /// [`Editor::drop_files`].
    #[doc(hidden)]
    pub fn pointer_drags_files(&mut self, at: Option<Point<Pixels>>, cx: &mut Context<Self>) {
        match at {
            Some(at) => self.file_drag_moved(at, cx),
            None => self.set_file_drop(None, cx),
        }
    }
}

// --- Images: resize and move ---------------------------------------------------

impl Editor {
    /// The index of the block with this id, as the document is now.
    pub(crate) fn block_index(&self, id: BlockId) -> Option<usize> {
        self.state
            .document()
            .blocks()
            .iter()
            .position(|block| block.id() == id)
    }

    /// The image block whose picture is under a window point.
    pub(crate) fn picture_at(&self, at: Point<Pixels>) -> Option<usize> {
        let (position, _) = self.caret_at_point(at)?;
        self.image_bounds(position.block)
            .is_some_and(|picture| picture.contains(&at))
            .then_some(position.block)
    }

    /// `escape` ends the drag of an image for as long as one lasts (see
    /// the module documentation).
    fn watch_escape(&mut self, cx: &mut Context<Self>) {
        let editor = cx.weak_entity();
        self.drag_escape = Some(
            self.interceptors
                .register(DRAG_PRIORITY, move |key, _, cx| {
                    if key.key != "escape" || key.modifiers.modified() {
                        return KeyDisposition::Pass;
                    }
                    editor
                        .update(cx, |editor, cx| editor.cancel_image_drag(cx))
                        .ok();
                    KeyDisposition::Consumed
                }),
        );
    }

    /// Ends the drag of an image without changing anything: the picture
    /// is as wide as it was and where it was.
    pub(crate) fn cancel_image_drag(&mut self, cx: &mut Context<Self>) {
        if !matches!(
            self.drag,
            Some(Drag::ResizeImage(_)) | Some(Drag::MoveImage(_))
        ) {
            return;
        }
        self.drag = None;
        self.drag_escape = None;
        self.drag_scroll = None;
        self.drag_scrolling = false;
        self.layout
            .borrow_mut()
            .set_image_preview(None, self.state.document());
        cx.notify();
    }

    /// A press on handle `corner` of the image block `block`. Says whether
    /// a resize began.
    fn start_image_resize(
        &mut self,
        block: BlockId,
        corner: usize,
        at: Point<Pixels>,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.config.read_only || self.config.mode.is_compact() {
            return false;
        }
        let Some(index) = self.block_index(block) else {
            return false;
        };
        let (Some(picture), Some(block_ref)) =
            (self.image_bounds(index), self.state.document().block(index))
        else {
            return false;
        };
        let Some(image) = block_ref.as_image() else {
            return false;
        };
        let resize = ImageResize {
            block,
            corner,
            pressed_x: at.x,
            start_width: picture.size.width / self.view_zoom(),
            centred: block_ref.alignment() == Alignment::Center,
            moved: false,
            width: image.width,
        };
        self.end_composition();
        self.state.break_undo_group();
        self.drag = Some(Drag::ResizeImage(resize));
        self.last_mouse = at;
        self.watch_escape(cx);
        cx.notify();
        true
    }

    /// The width an image gets for `wanted` layout units under the
    /// pointer: a share of the column within what a width can be, snapped
    /// to the presets and to the original size where one is near.
    fn dragged_image_width(&self, image: &Image, wanted: Pixels) -> ImageWidth {
        let (column, zoom) = self.layout.borrow().env().map_or((1., 1.), |env| {
            (f32::from(env.column_width).max(1.), env.zoom)
        });
        let wanted = f32::from(wanted);
        let widest = self.state.max_image_width(image);
        let narrowest = ImageWidth::MIN
            .max(ImageWidth::from_fraction(MIN_IMAGE_WIDTH / column))
            .min(widest);
        let snapped = SNAP_WIDTHS
            .into_iter()
            .map(ImageWidth::new)
            .chain([self.state.original_image_width(image)])
            .filter(|preset| (narrowest..=widest).contains(preset))
            .map(|preset| (preset, (preset.fraction() * column - wanted).abs() * zoom))
            .filter(|(_, distance)| *distance <= SNAP_DISTANCE)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        match snapped {
            Some((preset, _)) => preset,
            None => ImageWidth::from_fraction(wanted / column).clamp(narrowest, widest),
        }
    }

    /// The handle that is held follows the pointer: the layout shows the
    /// image at the new width, and the document stays as it is.
    fn resize_image_to_pointer(&mut self, cx: &mut Context<Self>) {
        let Some(Drag::ResizeImage(mut resize)) = self.drag.clone() else {
            return;
        };
        let image = self
            .block_index(resize.block)
            .and_then(|index| self.state.document().block(index))
            .and_then(|block| block.as_image());
        let Some(image) = image else {
            // The block is gone (a change from elsewhere).
            return self.cancel_image_drag(cx);
        };
        let sideways = (self.last_mouse.x - resize.pressed_x) / self.view_zoom();
        // Away from the picture is wider: to the right for the handles on
        // the right, to the left for those on the left.
        let outward = if resize.corner % 2 == 1 {
            sideways
        } else {
            -sideways
        };
        let grown = if resize.centred {
            outward * 2.
        } else {
            outward
        };
        let width = self.dragged_image_width(image, resize.start_width + grown);
        if resize.moved && width == resize.width {
            return;
        }
        resize.moved = true;
        resize.width = width;
        let block = resize.block;
        self.drag = Some(Drag::ResizeImage(resize));
        self.layout
            .borrow_mut()
            .set_image_preview(Some((block, width)), self.state.document());
        cx.notify();
    }

    /// The handle was let go: the width that was shown is stored, as one
    /// undo step.
    fn finish_image_resize(&mut self, resize: ImageResize, cx: &mut Context<Self>) {
        if resize.moved
            && !self.config.read_only
            && let Some(index) = self.block_index(resize.block)
        {
            // Nothing scrolls: the picture is where the pointer is.
            self.edit_with(
                Autoscroll::None,
                |state| state.set_image_width(index, resize.width),
                cx,
            );
            self.state.break_undo_group();
        }
        self.layout
            .borrow_mut()
            .set_image_preview(None, self.state.document());
        cx.notify();
    }

    /// The picture that was pressed follows the pointer: once it is far
    /// enough from the press, the boundary between blocks nearest to it
    /// is where the block would go.
    fn move_image_to_pointer(&mut self, cx: &mut Context<Self>) {
        let Some(Drag::MoveImage(mut moving)) = self.drag.clone() else {
            return;
        };
        let Some(from) = self.block_index(moving.block) else {
            return self.cancel_image_drag(cx);
        };
        if !moving.started {
            let (dx, dy) = (
                f32::from(self.last_mouse.x - moving.pressed_at.x),
                f32::from(self.last_mouse.y - moving.pressed_at.y),
            );
            if dx.hypot(dy) < f32::from(MOVE_THRESHOLD) {
                return;
            }
            moving.started = true;
            self.state.break_undo_group();
            self.watch_escape(cx);
        }
        // Outside the viewport the place follows the edge; the autoscroll
        // brings more of the text under it.
        let pointer = match self.viewport_bounds() {
            Some(viewport) => point(
                self.last_mouse.x,
                self.last_mouse
                    .y
                    .clamp(viewport.top(), viewport.bottom() - px(1.)),
            ),
            None => self.last_mouse,
        };
        // Before or after the block itself is where it is already.
        moving.drop = self
            .boundary_at(pointer)
            .filter(|to| *to != from && *to != from + 1);
        if self.drag.as_ref() != Some(&Drag::MoveImage(moving.clone())) {
            self.drag = Some(Drag::MoveImage(moving));
            cx.notify();
        }
    }

    /// The picture was let go: its block goes to the boundary that was
    /// shown, as one undo step, and stays selected.
    fn finish_image_move(&mut self, moving: ImageMove, cx: &mut Context<Self>) {
        if moving.started
            && let Some(to) = moving.drop
            && let Some(from) = self.block_index(moving.block)
        {
            self.user_edit(|state| state.move_block_to(from, to), cx);
            self.state.break_undo_group();
        }
        cx.notify();
    }
}

/// What a wheel delta means where the view may also move sideways
/// (`can_go_sideways`). Two fingers on a trackpad never move straight: the
/// small part of a move that is mostly up and down (or mostly sideways) is
/// dropped, so a sheet does not wander while the text is scrolled. With
/// `shift` a delta that only goes up and down goes sideways (macOS turns
/// the wheel of most mice itself; this is for the ones it does not).
pub(crate) fn sideways_intent(
    delta: Point<Pixels>,
    shift: bool,
    can_go_sideways: bool,
) -> Point<Pixels> {
    if !can_go_sideways {
        return gpui::point(px(0.), delta.y);
    }
    if shift && delta.x == px(0.) {
        return gpui::point(delta.y, px(0.));
    }
    let (across, along) = (delta.x.abs(), delta.y.abs());
    if along >= across * 2. {
        gpui::point(px(0.), delta.y)
    } else if across >= along * 2. {
        gpui::point(delta.x, px(0.))
    } else {
        delta
    }
}
