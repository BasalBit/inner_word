//! [`TextField`]: a single-line text input.
//!
//! Real text input goes through GPUI's platform input handler, so IME
//! composition (marked text), dead keys (`alt-e e` gives "é"), the press-and-hold
//! accent menu and the emoji picker (`ctrl-cmd-space`) all work. The field
//! paints its own caret, selection and marked-text underline, scrolls
//! horizontally to keep the caret visible, and has its own undo history.
//!
//! # Usage
//!
//! ```ignore
//! use writer_ui::text_field::{TextField, TextFieldEvent, FieldAppearance};
//!
//! // in a view constructor (needs the window for focus tracking)
//! let name = cx.new(|cx| {
//!     TextField::new(window, cx)
//!         .with_placeholder("Document title")
//!         .with_text("Untitled")
//! });
//! cx.subscribe_in(&name, window, |this, field, event: &TextFieldEvent, window, cx| {
//!     match event {
//!         TextFieldEvent::Changed => this.title = field.read(cx).text().to_string(),
//!         TextFieldEvent::Submitted => this.commit(window, cx),
//!         TextFieldEvent::Cancelled => this.cancel(window, cx),
//!         TextFieldEvent::Focused | TextFieldEvent::Blurred => {}
//!     }
//! })
//! .detach();
//!
//! // in render: the entity is the element
//! div().child(self.name.clone())
//!
//! // programmatic control
//! self.name.update(cx, |field, cx| { field.set_text("Chapter 1", cx); field.select_all(cx); });
//! self.name.read(cx).focus_handle(cx).focus(window);
//! ```
//!
//! Inline rename in a tree: `TextField::new(window, cx).with_appearance(FieldAppearance::Inline)`,
//! commit on `Submitted` and on `Blurred`, revert on `Cancelled`.
//!
//! # Keys
//!
//! Bound in the `"TextField"` key context by [`crate::init`]:
//! arrows, `alt-left/right` (word), `cmd-left/right` and `ctrl-a/e` (line),
//! `shift` + any of those to extend the selection, `backspace`, `delete`,
//! `alt-backspace` / `alt-delete` (word), `cmd-backspace` / `cmd-delete` /
//! `ctrl-k` (to line start / end), `ctrl-b/f/h/d`, `cmd-a`, `cmd-c/x/v`,
//! `cmd-shift-v`, `cmd-z` / `cmd-shift-z`, `enter` ([`TextFieldEvent::Submitted`]),
//! `escape` ([`TextFieldEvent::Cancelled`]), `ctrl-cmd-space` (emoji and symbols).
//!
//! `escape` is not consumed by default: after emitting `Cancelled` the key
//! continues to the parent, so a surrounding modal or picker closes too. Use
//! [`TextField::consume_escape`] to stop it.
//!
//! # Mouse
//!
//! Click, shift-click, drag, double-click (word, and drag by words),
//! triple-click (all). A right-click, and `ctrl-enter` from the keyboard,
//! opens the field's [`crate::ContextMenu`]: Cut, Copy, Paste and Select All
//! with their keys, greyed when they cannot apply (nothing selected, nothing
//! to paste, a disabled field). A right-click outside the selection first
//! selects the word under the pointer, as macOS text fields do. The owner of
//! a field can add items of its own with [`TextField::with_menu_items`].
//!
//! `up` / `down` / `ctrl-p` / `ctrl-n` / `pageup` / `pagedown` / `home` / `end`
//! move the caret to the start or end of the line, unless
//! [`TextField::set_list_navigation`] is on: then they are passed to the
//! parent (the [`crate::picker::Picker`] uses this to move its selection).

pub mod boundaries;
mod element;
mod history;

use std::{ops::Range, rc::Rc, time::Duration};

use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, EntityInputHandler, EventEmitter,
    FocusHandle, Focusable, KeyBinding, MouseButton, MouseDownEvent, Pixels, Point, SharedString,
    Subscription, Task, UTF16Selection, Window, actions, div, point, prelude::*, px,
};

use crate::{
    context_menu::{ContextMenu, ContextMenuItem, MenuAnchor, OpenContextMenu},
    theme::{ActiveTheme, RADIUS_MD, RADIUS_SM, UI_LINE_HEIGHT, UI_TEXT_BASE, focus_ring},
};
use boundaries::*;
use element::TextFieldElement;
use history::{EditKind, History, Snapshot};

/// The key context of a focused [`TextField`].
pub const KEY_CONTEXT: &str = "TextField";

actions!(
    text_field,
    [
        /// Delete the grapheme before the caret, or the selection.
        Backspace,
        /// Delete the grapheme after the caret, or the selection.
        Delete,
        /// Delete to the start of the word before the caret.
        DeleteWordLeft,
        /// Delete to the end of the word after the caret.
        DeleteWordRight,
        /// Delete to the start of the line.
        DeleteToLineStart,
        /// Delete to the end of the line.
        DeleteToLineEnd,
        /// Move the caret one grapheme left.
        Left,
        /// Move the caret one grapheme right.
        Right,
        /// Move the caret to the previous word start.
        WordLeft,
        /// Move the caret to the next word end.
        WordRight,
        /// Move the caret to the start of the line.
        LineStart,
        /// Move the caret to the end of the line.
        LineEnd,
        /// Up arrow: start of line, or list navigation in a picker.
        Up,
        /// Down arrow: end of line, or list navigation in a picker.
        Down,
        /// Page up: start of line, or list navigation in a picker.
        PageUp,
        /// Page down: end of line, or list navigation in a picker.
        PageDown,
        /// Home: start of line, or list navigation in a picker.
        Home,
        /// End: end of line, or list navigation in a picker.
        End,
        /// Extend the selection one grapheme left.
        SelectLeft,
        /// Extend the selection one grapheme right.
        SelectRight,
        /// Extend the selection to the previous word start.
        SelectWordLeft,
        /// Extend the selection to the next word end.
        SelectWordRight,
        /// Extend the selection to the start of the line.
        SelectToLineStart,
        /// Extend the selection to the end of the line.
        SelectToLineEnd,
        /// Select all text.
        SelectAll,
        /// Copy the selection.
        Copy,
        /// Cut the selection.
        Cut,
        /// Paste (line breaks become spaces).
        Paste,
        /// Undo the last edit in this field.
        Undo,
        /// Redo the last undone edit in this field.
        Redo,
        /// Submit (enter).
        Submit,
        /// Cancel (escape).
        Cancel,
        /// Show the macOS emoji and symbols picker.
        ShowCharacterPalette,
    ]
);

/// Key bindings for [`TextField`], installed by [`crate::init`].
pub fn key_bindings() -> Vec<KeyBinding> {
    let c = Some(KEY_CONTEXT);
    vec![
        // Alternatives first, the canonical key last: the last binding is
        // the one KeyHint::for_action shows.
        KeyBinding::new("ctrl-h", Backspace, c),
        KeyBinding::new("shift-backspace", Backspace, c),
        KeyBinding::new("backspace", Backspace, c),
        KeyBinding::new("ctrl-d", Delete, c),
        KeyBinding::new("delete", Delete, c),
        KeyBinding::new("ctrl-w", DeleteWordLeft, c),
        KeyBinding::new("alt-backspace", DeleteWordLeft, c),
        KeyBinding::new("alt-delete", DeleteWordRight, c),
        KeyBinding::new("cmd-backspace", DeleteToLineStart, c),
        KeyBinding::new("ctrl-k", DeleteToLineEnd, c),
        KeyBinding::new("cmd-delete", DeleteToLineEnd, c),
        KeyBinding::new("ctrl-b", Left, c),
        KeyBinding::new("left", Left, c),
        KeyBinding::new("ctrl-f", Right, c),
        KeyBinding::new("right", Right, c),
        KeyBinding::new("alt-left", WordLeft, c),
        KeyBinding::new("alt-right", WordRight, c),
        KeyBinding::new("ctrl-a", LineStart, c),
        KeyBinding::new("cmd-up", LineStart, c),
        KeyBinding::new("cmd-left", LineStart, c),
        KeyBinding::new("ctrl-e", LineEnd, c),
        KeyBinding::new("cmd-down", LineEnd, c),
        KeyBinding::new("cmd-right", LineEnd, c),
        KeyBinding::new("ctrl-p", Up, c),
        KeyBinding::new("up", Up, c),
        KeyBinding::new("ctrl-n", Down, c),
        KeyBinding::new("down", Down, c),
        KeyBinding::new("pageup", PageUp, c),
        KeyBinding::new("pagedown", PageDown, c),
        KeyBinding::new("home", Home, c),
        KeyBinding::new("end", End, c),
        KeyBinding::new("shift-left", SelectLeft, c),
        KeyBinding::new("shift-right", SelectRight, c),
        KeyBinding::new("alt-shift-left", SelectWordLeft, c),
        KeyBinding::new("alt-shift-right", SelectWordRight, c),
        KeyBinding::new("shift-up", SelectToLineStart, c),
        KeyBinding::new("shift-home", SelectToLineStart, c),
        KeyBinding::new("cmd-shift-up", SelectToLineStart, c),
        KeyBinding::new("cmd-shift-left", SelectToLineStart, c),
        KeyBinding::new("shift-down", SelectToLineEnd, c),
        KeyBinding::new("shift-end", SelectToLineEnd, c),
        KeyBinding::new("cmd-shift-down", SelectToLineEnd, c),
        KeyBinding::new("cmd-shift-right", SelectToLineEnd, c),
        KeyBinding::new("cmd-a", SelectAll, c),
        KeyBinding::new("cmd-c", Copy, c),
        KeyBinding::new("cmd-x", Cut, c),
        KeyBinding::new("cmd-shift-v", Paste, c),
        KeyBinding::new("cmd-v", Paste, c),
        KeyBinding::new("cmd-z", Undo, c),
        KeyBinding::new("cmd-shift-z", Redo, c),
        KeyBinding::new("enter", Submit, c),
        KeyBinding::new("escape", Cancel, c),
        KeyBinding::new("ctrl-cmd-space", ShowCharacterPalette, c),
        // `shift-f10` first, so key hints show `ctrl-enter`.
        KeyBinding::new("shift-f10", OpenContextMenu, c),
        KeyBinding::new("ctrl-enter", OpenContextMenu, c),
    ]
}

/// Events emitted by a [`TextField`]. Subscribe with `cx.subscribe(..)` or
/// `cx.subscribe_in(..)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextFieldEvent {
    /// The text changed through user input (typing, IME, paste, cut, delete,
    /// undo / redo). Not emitted for [`TextField::set_text`].
    Changed,
    /// `enter` was pressed.
    Submitted,
    /// `escape` was pressed.
    Cancelled,
    /// The field gained focus (only fires while the window is active).
    Focused,
    /// The field lost focus.
    Blurred,
}

/// How a [`TextField`] is drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FieldAppearance {
    /// Outlined field with a background and a focus ring (forms, settings,
    /// link URL field). 28px tall at the default text size.
    #[default]
    Bordered,
    /// No border, no background, no padding: for query rows in pickers and
    /// find bars where the surrounding panel provides the frame.
    Borderless,
    /// Compact outlined field for editing in place (rename in a tree row).
    /// 22px tall, small radius, accent outline while focused.
    Inline,
}

#[derive(Clone, Debug)]
enum DragMode {
    Graphemes,
    /// Double-click drag extends by words; holds the word first clicked.
    Words(Range<usize>),
}

type MenuItems = Rc<dyn Fn(&mut Window, &mut App) -> Vec<ContextMenuItem>>;

const BLINK_INTERVAL: Duration = Duration::from_millis(530);

/// A single-line text input entity. See the [module docs](self).
pub struct TextField {
    focus_handle: FocusHandle,
    text: String,
    placeholder: SharedString,
    /// Byte range, always on grapheme boundaries.
    selection: Range<usize>,
    /// The caret is at `selection.start` when true, else at `selection.end`.
    reversed: bool,
    marked: Option<Range<usize>>,
    history: History,
    appearance: FieldAppearance,
    text_size: Pixels,
    list_navigation: bool,
    consume_escape: bool,
    disabled: bool,
    menu_items: Option<MenuItems>,

    // Layout from the last paint, for hit testing and IME bounds.
    pub(crate) scroll_x: Pixels,
    pub(crate) last_layout: Option<gpui::ShapedLine>,
    pub(crate) last_bounds: Option<Bounds<Pixels>>,

    // Mouse selection.
    pub(crate) selecting: bool,
    drag: Option<DragMode>,

    // Caret blink.
    has_focus: bool,
    caret_visible: bool,
    blink_task: Option<Task<()>>,

    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<TextFieldEvent> for TextField {}

impl Focusable for TextField {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl TextField {
    /// A new empty field. Create it with `cx.new(|cx| TextField::new(window, cx))`.
    /// The focus handle is a tab stop, so `tab` navigation (see
    /// [`crate::focus::tab_navigation`]) reaches it.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle().tab_stop(true);
        let subscriptions = vec![
            cx.on_focus(&focus_handle, window, |this, _, cx| {
                this.has_focus = true;
                this.reset_blink(cx);
                cx.emit(TextFieldEvent::Focused);
                cx.notify();
            }),
            cx.on_blur(&focus_handle, window, |this, _, cx| {
                this.has_focus = false;
                this.blink_task = None;
                this.caret_visible = true;
                this.selecting = false;
                this.history.break_group();
                cx.emit(TextFieldEvent::Blurred);
                cx.notify();
            }),
        ];
        Self {
            focus_handle,
            text: String::new(),
            placeholder: SharedString::default(),
            selection: 0..0,
            reversed: false,
            marked: None,
            history: History::default(),
            appearance: FieldAppearance::default(),
            text_size: UI_TEXT_BASE,
            list_navigation: false,
            consume_escape: false,
            disabled: false,
            menu_items: None,
            scroll_x: px(0.),
            last_layout: None,
            last_bounds: None,
            selecting: false,
            drag: None,
            has_focus: false,
            caret_visible: true,
            blink_task: None,
            _subscriptions: subscriptions,
        }
    }

    // ---------------------------------------------------------------------
    // Builders (use inside `cx.new`)
    // ---------------------------------------------------------------------

    /// Placeholder shown in the faint colour while the field is empty.
    pub fn with_placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    /// Initial text; the caret goes to the end.
    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.text = single_line(&text.into()).into_owned();
        self.selection = self.text.len()..self.text.len();
        self
    }

    pub fn with_appearance(mut self, appearance: FieldAppearance) -> Self {
        self.appearance = appearance;
        self
    }

    /// Font size (default 13px, the UI base size). Line height scales with it.
    pub fn with_text_size(mut self, size: Pixels) -> Self {
        self.text_size = size;
        self
    }

    /// Stop `escape` from reaching the parent after `Cancelled` is emitted.
    pub fn consume_escape(mut self) -> Self {
        self.consume_escape = true;
        self
    }

    /// Items of the field's owner for the field's context menu, asked for
    /// each time the menu opens. They follow Cut, Copy, Paste and Select All
    /// after a separator ("Rename", "Clear", "Open link").
    pub fn with_menu_items(
        mut self,
        items: impl Fn(&mut Window, &mut App) -> Vec<ContextMenuItem> + 'static,
    ) -> Self {
        self.menu_items = Some(Rc::new(items));
        self
    }

    // ---------------------------------------------------------------------
    // Reading and writing
    // ---------------------------------------------------------------------

    /// The current text (marked IME text included).
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Replaces the whole text, puts the caret at the end and clears the undo
    /// history. Does not emit [`TextFieldEvent::Changed`].
    pub fn set_text(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.text = single_line(&text.into()).into_owned();
        self.selection = self.text.len()..self.text.len();
        self.reversed = false;
        self.marked = None;
        self.scroll_x = px(0.);
        self.history.clear();
        cx.notify();
    }

    /// Inserts `text` at the caret (replacing the selection) as if typed:
    /// undoable, emits `Changed`.
    pub fn insert(&mut self, text: &str, cx: &mut Context<Self>) {
        let range = self.selection.clone();
        self.replace(range, text, EditKind::Other, cx);
    }

    pub fn set_placeholder(
        &mut self,
        placeholder: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.placeholder = placeholder.into();
        cx.notify();
    }

    pub fn placeholder_text(&self) -> &SharedString {
        &self.placeholder
    }

    pub fn set_appearance(&mut self, appearance: FieldAppearance, cx: &mut Context<Self>) {
        self.appearance = appearance;
        cx.notify();
    }

    pub fn set_text_size(&mut self, size: Pixels, cx: &mut Context<Self>) {
        self.text_size = size;
        cx.notify();
    }

    /// When on, `up`/`down`/`ctrl-p`/`ctrl-n`/`pageup`/`pagedown`/`home`/`end`
    /// are not handled by the field and go to the parent's bindings.
    pub fn set_list_navigation(&mut self, on: bool) {
        self.list_navigation = on;
    }

    pub fn set_consume_escape(&mut self, consume: bool) {
        self.consume_escape = consume;
    }

    /// A disabled field ignores input and draws faint text.
    pub fn set_disabled(&mut self, disabled: bool, cx: &mut Context<Self>) {
        self.disabled = disabled;
        cx.notify();
    }

    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    /// Selected byte range (empty = caret position).
    pub fn selection(&self) -> Range<usize> {
        self.selection.clone()
    }

    /// The caret offset (the moving end of the selection).
    pub fn cursor(&self) -> usize {
        if self.reversed {
            self.selection.start
        } else {
            self.selection.end
        }
    }

    /// Byte range of uncommitted IME text, if composing.
    pub fn marked_range(&self) -> Option<Range<usize>> {
        self.marked.clone()
    }

    /// The selected text.
    pub fn selected_text(&self) -> &str {
        &self.text[self.selection.clone()]
    }

    /// Selects `range` (clamped to grapheme boundaries). The caret ends at
    /// `range.end`.
    pub fn set_selection(&mut self, range: Range<usize>, cx: &mut Context<Self>) {
        let start = clamp_to_grapheme(&self.text, range.start);
        let end = clamp_to_grapheme(&self.text, range.end);
        self.selection = start.min(end)..start.max(end);
        self.reversed = end < start;
        self.after_move(cx);
    }

    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        self.selection = 0..self.text.len();
        self.reversed = false;
        self.after_move(cx);
    }

    /// Moves keyboard focus into the field.
    pub fn focus(&self, window: &mut Window) {
        window.focus(&self.focus_handle);
    }

    pub fn is_focused(&self, window: &Window) -> bool {
        self.focus_handle.is_focused(window)
    }

    /// Bounds of the text area in the last painted frame (window coordinates).
    pub fn last_painted_bounds(&self) -> Option<Bounds<Pixels>> {
        self.last_bounds
    }

    /// Horizontal scroll of the text inside the field (0 = start visible).
    pub fn scroll_offset(&self) -> Pixels {
        self.scroll_x
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    // ---------------------------------------------------------------------
    // Editing core
    // ---------------------------------------------------------------------

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            text: self.text.clone(),
            selection: self.selection.clone(),
            reversed: self.reversed,
        }
    }

    fn restore(&mut self, snapshot: Snapshot, cx: &mut Context<Self>) {
        self.text = snapshot.text;
        self.selection = snapshot.selection;
        self.reversed = snapshot.reversed;
        self.marked = None;
        self.reset_blink(cx);
        cx.emit(TextFieldEvent::Changed);
        cx.notify();
    }

    /// Replaces `range` with `new_text` (newlines become spaces), records undo,
    /// puts the caret after the insertion, emits `Changed`.
    fn replace(
        &mut self,
        range: Range<usize>,
        new_text: &str,
        kind: EditKind,
        cx: &mut Context<Self>,
    ) {
        if self.disabled {
            return;
        }
        let new_text = single_line(new_text);
        let range = range.start.min(self.text.len())..range.end.min(self.text.len());
        if range.is_empty() && new_text.is_empty() {
            return;
        }
        let before = self.snapshot();
        let caret_before = self.cursor();
        self.text.replace_range(range.clone(), &new_text);
        let caret = range.start + new_text.len();
        self.selection = caret..caret;
        self.reversed = false;
        self.marked = None;
        self.history
            .record(before, kind, caret_before, caret, &new_text);
        self.reset_blink(cx);
        cx.emit(TextFieldEvent::Changed);
        cx.notify();
    }

    /// Deletes the selection if there is one, else `start..end` around the caret.
    fn delete_to(&mut self, target: usize, kind: EditKind, cx: &mut Context<Self>) {
        let range = if self.selection.is_empty() {
            let caret = self.cursor();
            caret.min(target)..caret.max(target)
        } else {
            self.selection.clone()
        };
        let kind = if self.selection.is_empty() {
            kind
        } else {
            EditKind::Other
        };
        self.replace(range, "", kind, cx);
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selection = offset..offset;
        self.reversed = false;
        self.after_move(cx);
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let anchor = if self.reversed {
            self.selection.end
        } else {
            self.selection.start
        };
        self.selection = anchor.min(offset)..anchor.max(offset);
        self.reversed = offset < anchor;
        self.after_move(cx);
    }

    fn after_move(&mut self, cx: &mut Context<Self>) {
        self.history.break_group();
        self.reset_blink(cx);
        cx.notify();
    }

    fn reset_blink(&mut self, cx: &mut Context<Self>) {
        self.caret_visible = true;
        if self.has_focus {
            self.blink_task = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(BLINK_INTERVAL).await;
                    let keep_going = this.update(cx, |this, cx| {
                        if !this.has_focus {
                            this.caret_visible = true;
                            return false;
                        }
                        this.caret_visible = !this.caret_visible;
                        cx.notify();
                        true
                    });
                    if !matches!(keep_going, Ok(true)) {
                        break;
                    }
                }
            }));
        }
    }

    pub(crate) fn caret_visible(&self) -> bool {
        self.caret_visible
    }

    // ---------------------------------------------------------------------
    // Actions
    // ---------------------------------------------------------------------

    fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        let target = prev_grapheme(&self.text, self.cursor());
        self.delete_to(target, EditKind::Delete, cx);
    }

    fn delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        let target = next_grapheme(&self.text, self.cursor());
        self.delete_to(target, EditKind::Delete, cx);
    }

    fn delete_word_left(&mut self, _: &DeleteWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        let target = prev_word_start(&self.text, self.cursor());
        self.delete_to(target, EditKind::Other, cx);
    }

    fn delete_word_right(&mut self, _: &DeleteWordRight, _: &mut Window, cx: &mut Context<Self>) {
        let target = next_word_end(&self.text, self.cursor());
        self.delete_to(target, EditKind::Other, cx);
    }

    fn delete_to_line_start(
        &mut self,
        _: &DeleteToLineStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.delete_to(0, EditKind::Other, cx);
    }

    fn delete_to_line_end(&mut self, _: &DeleteToLineEnd, _: &mut Window, cx: &mut Context<Self>) {
        let end = self.text.len();
        self.delete_to(end, EditKind::Other, cx);
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.selection.is_empty() {
            self.move_to(prev_grapheme(&self.text, self.cursor()), cx);
        } else {
            self.move_to(self.selection.start, cx);
        }
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.selection.is_empty() {
            self.move_to(next_grapheme(&self.text, self.cursor()), cx);
        } else {
            self.move_to(self.selection.end, cx);
        }
    }

    fn word_left(&mut self, _: &WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        let from = if self.selection.is_empty() {
            self.cursor()
        } else {
            self.selection.start
        };
        self.move_to(prev_word_start(&self.text, from), cx);
    }

    fn word_right(&mut self, _: &WordRight, _: &mut Window, cx: &mut Context<Self>) {
        let from = if self.selection.is_empty() {
            self.cursor()
        } else {
            self.selection.end
        };
        self.move_to(next_word_end(&self.text, from), cx);
    }

    fn line_start(&mut self, _: &LineStart, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }

    fn line_end(&mut self, _: &LineEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.text.len(), cx);
    }

    fn nav_start(&mut self, cx: &mut Context<Self>) {
        if self.list_navigation {
            cx.propagate();
        } else {
            self.move_to(0, cx);
        }
    }

    fn nav_end(&mut self, cx: &mut Context<Self>) {
        if self.list_navigation {
            cx.propagate();
        } else {
            self.move_to(self.text.len(), cx);
        }
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        self.nav_start(cx);
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        self.nav_end(cx);
    }

    fn page_up(&mut self, _: &PageUp, _: &mut Window, cx: &mut Context<Self>) {
        self.nav_start(cx);
    }

    fn page_down(&mut self, _: &PageDown, _: &mut Window, cx: &mut Context<Self>) {
        self.nav_end(cx);
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.nav_start(cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.nav_end(cx);
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(prev_grapheme(&self.text, self.cursor()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(next_grapheme(&self.text, self.cursor()), cx);
    }

    fn select_word_left(&mut self, _: &SelectWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(prev_word_start(&self.text, self.cursor()), cx);
    }

    fn select_word_right(&mut self, _: &SelectWordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(next_word_end(&self.text, self.cursor()), cx);
    }

    fn select_to_line_start(
        &mut self,
        _: &SelectToLineStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_to(0, cx);
    }

    fn select_to_line_end(&mut self, _: &SelectToLineEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.text.len(), cx);
    }

    fn select_all_action(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.select_all(cx);
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selection.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.selected_text().to_string()));
        }
    }

    fn cut(&mut self, _: &Cut, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selection.is_empty() && !self.disabled {
            cx.write_to_clipboard(ClipboardItem::new_string(self.selected_text().to_string()));
            let range = self.selection.clone();
            self.replace(range, "", EditKind::Other, cx);
        }
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            let range = self.selection.clone();
            self.replace(range, &text, EditKind::Other, cx);
        }
    }

    fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        if let Some(previous) = self.history.undo(self.snapshot()) {
            self.restore(previous, cx);
        }
    }

    fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        if let Some(next) = self.history.redo(self.snapshot()) {
            self.restore(next, cx);
        }
    }

    fn submit(&mut self, _: &Submit, _: &mut Window, cx: &mut Context<Self>) {
        self.history.break_group();
        cx.emit(TextFieldEvent::Submitted);
    }

    fn cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        self.history.break_group();
        cx.emit(TextFieldEvent::Cancelled);
        if !self.consume_escape {
            cx.propagate();
        }
    }

    fn show_character_palette(
        &mut self,
        _: &ShowCharacterPalette,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        window.show_character_palette();
    }

    // ---------------------------------------------------------------------
    // Mouse
    // ---------------------------------------------------------------------

    /// Text offset for a window position, using the last painted layout.
    pub(crate) fn index_for_position(&self, position: Point<Pixels>) -> usize {
        let (Some(bounds), Some(line)) = (self.last_bounds.as_ref(), self.last_layout.as_ref())
        else {
            return self.text.len();
        };
        if self.text.is_empty() {
            return 0;
        }
        let x = position.x - bounds.left() + self.scroll_x;
        if x <= px(0.) {
            return 0;
        }
        clamp_to_grapheme(&self.text, line.closest_index_for_x(x))
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle);
        // Clicking while composing commits the composition as typed.
        self.marked = None;
        let ix = self.index_for_position(event.position);
        self.selecting = true;
        match event.click_count {
            2 => {
                let word = word_range_at(&self.text, ix);
                self.selection = word.clone();
                self.reversed = false;
                self.drag = Some(DragMode::Words(word));
                self.after_move(cx);
            }
            count if count >= 3 => {
                self.drag = None;
                self.select_all(cx);
            }
            _ => {
                self.drag = Some(DragMode::Graphemes);
                if event.modifiers.shift {
                    self.select_to(ix, cx);
                } else {
                    self.move_to(ix, cx);
                }
            }
        }
    }

    /// Right button: the menu of the field, on the selection when the
    /// pointer is on it, else on the word under the pointer.
    fn on_secondary_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle);
        self.marked = None;
        self.selecting = false;
        self.drag = None;
        let ix = self.index_for_position(event.position);
        let on_selection =
            !self.selection.is_empty() && self.selection.start <= ix && ix <= self.selection.end;
        if !on_selection {
            let word = word_range_at(&self.text, ix);
            if word.is_empty() {
                self.move_to(ix, cx);
            } else {
                self.selection = word;
                self.reversed = false;
                self.after_move(cx);
            }
        }
        self.open_context_menu(event.position, window, cx);
    }

    fn open_context_menu_action(
        &mut self,
        _: &OpenContextMenu,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let anchor = match (self.last_bounds, self.last_layout.as_ref()) {
            (Some(bounds), Some(line)) => {
                let x = bounds.left() - self.scroll_x + line.x_for_index(self.cursor());
                let x = x.max(bounds.left()).min(bounds.right());
                MenuAnchor::Below(Bounds::from_corners(
                    point(x, bounds.top()),
                    point(x, bounds.bottom()),
                ))
            }
            (Some(bounds), None) => MenuAnchor::Below(bounds),
            _ => MenuAnchor::Point(window.mouse_position()),
        };
        let menu = self.open_context_menu(anchor, window, cx);
        menu.update(cx, |menu, cx| menu.select_first(cx));
    }

    /// What the field's context menu holds right now: Cut, Copy, Paste and
    /// Select All (disabled when they cannot apply), then the owner's items
    /// ([`TextField::with_menu_items`]).
    pub fn context_menu_items(&self, window: &mut Window, cx: &mut App) -> Vec<ContextMenuItem> {
        let has_selection = !self.selection.is_empty();
        let can_paste = !self.disabled
            && cx
                .read_from_clipboard()
                .and_then(|item| item.text())
                .is_some_and(|text| !text.is_empty());
        let mut items = vec![
            ContextMenuItem::action("Cut", Cut).disabled(!has_selection || self.disabled),
            ContextMenuItem::action("Copy", Copy).disabled(!has_selection),
            ContextMenuItem::action("Paste", Paste).disabled(!can_paste),
            ContextMenuItem::separator(),
            ContextMenuItem::action("Select All", SelectAll).disabled(self.text.is_empty()),
        ];
        if let Some(extra) = self.menu_items.clone() {
            items.push(ContextMenuItem::separator());
            items.extend(extra(window, cx));
        }
        items
    }

    /// Opens the field's context menu at `anchor`. Its actions go to this
    /// field wherever the keyboard is.
    pub fn open_context_menu(
        &mut self,
        anchor: impl Into<MenuAnchor>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Entity<ContextMenu> {
        let items = self.context_menu_items(window, cx);
        ContextMenu::open_for(&self.focus_handle, anchor, items, window, cx)
    }

    /// Called by the element for window-wide mouse moves while selecting.
    pub(crate) fn drag_to(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if !self.selecting {
            return;
        }
        let ix = self.index_for_position(position);
        match self.drag.clone() {
            Some(DragMode::Words(origin)) => {
                let word = word_range_at(&self.text, ix);
                let start = origin.start.min(word.start);
                let end = origin.end.max(word.end);
                self.selection = start..end;
                self.reversed = word.start < origin.start;
                self.after_move(cx);
            }
            Some(DragMode::Graphemes) => self.select_to(ix, cx),
            None => {}
        }
    }

    pub(crate) fn end_drag(&mut self, cx: &mut Context<Self>) {
        if self.selecting {
            self.selecting = false;
            self.drag = None;
            cx.notify();
        }
    }

    // ---------------------------------------------------------------------
    // UTF-16 helpers for the input handler
    // ---------------------------------------------------------------------

    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        utf16_to_utf8(&self.text, range.start)..utf16_to_utf8(&self.text, range.end)
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        utf8_to_utf16(&self.text, range.start)..utf8_to_utf16(&self.text, range.end)
    }
}

impl EntityInputHandler for TextField {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        adjusted_range.replace(self.range_to_utf16(&range));
        Some(self.text[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selection),
            reversed: self.reversed,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_ref().map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.disabled {
            return;
        }
        let composing = self.marked.is_some();
        let range = range_utf16
            .as_ref()
            .map(|range| self.range_from_utf16(range))
            .or(self.marked.clone())
            .unwrap_or(self.selection.clone());
        if composing {
            // The composition already recorded an undo step when it started.
            let text = single_line(text);
            self.text.replace_range(range.clone(), &text);
            let caret = range.start + text.len();
            self.selection = caret..caret;
            self.reversed = false;
            self.marked = None;
            self.history.break_group();
            self.reset_blink(cx);
            cx.emit(TextFieldEvent::Changed);
            cx.notify();
        } else {
            self.replace(range, text, EditKind::Insert, cx);
        }
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.disabled {
            return;
        }
        let range = range_utf16
            .as_ref()
            .map(|range| self.range_from_utf16(range))
            .or(self.marked.clone())
            .unwrap_or(self.selection.clone());
        if self.marked.is_none() {
            let before = self.snapshot();
            let caret = self.cursor();
            self.history
                .record(before, EditKind::Other, caret, caret, new_text);
        }
        let new_text = single_line(new_text);
        self.text.replace_range(range.clone(), &new_text);
        self.marked = (!new_text.is_empty()).then(|| range.start..range.start + new_text.len());
        // `new_selected_range` is relative to the inserted text, in UTF-16.
        self.selection = match new_selected_range_utf16 {
            Some(relative) => {
                let start = range.start + utf16_to_utf8(&new_text, relative.start);
                let end = range.start + utf16_to_utf8(&new_text, relative.end);
                start..end
            }
            None => {
                let caret = range.start + new_text.len();
                caret..caret
            }
        };
        self.reversed = false;
        self.reset_blink(cx);
        cx.emit(TextFieldEvent::Changed);
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let line = self.last_layout.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);
        let left = element_bounds.left() - self.scroll_x;
        Some(Bounds::from_corners(
            point(left + line.x_for_index(range.start), element_bounds.top()),
            point(left + line.x_for_index(range.end), element_bounds.bottom()),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let bounds = self.last_bounds?;
        let line = self.last_layout.as_ref()?;
        let x = point.x - bounds.left() + self.scroll_x;
        let ix = line.index_for_x(x)?;
        Some(utf8_to_utf16(&self.text, ix))
    }
}

impl Render for TextField {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let colors = theme.colors;
        let focused = self.focus_handle.is_focused(window);
        let line_height = (self.text_size * (UI_LINE_HEIGHT / UI_TEXT_BASE)).round();
        let appearance = self.appearance;

        let base = div()
            .id("text-field")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::delete_word_left))
            .on_action(cx.listener(Self::delete_word_right))
            .on_action(cx.listener(Self::delete_to_line_start))
            .on_action(cx.listener(Self::delete_to_line_end))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::word_left))
            .on_action(cx.listener(Self::word_right))
            .on_action(cx.listener(Self::line_start))
            .on_action(cx.listener(Self::line_end))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::page_up))
            .on_action(cx.listener(Self::page_down))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::select_to_line_start))
            .on_action(cx.listener(Self::select_to_line_end))
            .on_action(cx.listener(Self::select_all_action))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::submit))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::show_character_palette))
            .on_action(cx.listener(Self::open_context_menu_action))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(Self::on_secondary_mouse_down),
            )
            .flex()
            .items_center()
            .w_full()
            .min_w(px(24.))
            .overflow_hidden()
            .font_family(theme.fonts.ui.clone())
            .text_size(self.text_size)
            .line_height(line_height)
            .text_color(if self.disabled {
                colors.faint
            } else {
                colors.ink
            });

        let styled = match appearance {
            FieldAppearance::Bordered => base
                .h(line_height + px(10.))
                .px(px(8.))
                .rounded(RADIUS_MD)
                .bg(colors.page)
                .border_1()
                .border_color(if focused {
                    colors.accent
                } else {
                    colors.border_strong
                })
                .when(focused, |el| el.shadow(focus_ring(&colors))),
            FieldAppearance::Borderless => base.h(line_height),
            FieldAppearance::Inline => base
                .h(line_height + px(4.))
                .px(px(4.))
                .rounded(RADIUS_SM)
                .bg(colors.page)
                .border_1()
                .border_color(if focused {
                    colors.accent
                } else {
                    colors.border_strong
                }),
        };

        styled.child(
            div()
                .flex_1()
                .h(line_height)
                .overflow_hidden()
                .child(TextFieldElement::new(cx.entity())),
        )
    }
}
