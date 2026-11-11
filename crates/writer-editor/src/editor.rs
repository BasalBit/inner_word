//! The [`Editor`] entity: state, configuration, the mutation seam, events.
//!
//! The rest of the entity's behaviour is spread over sibling modules, each
//! an `impl Editor` block: `geometry`, `movement` (visual rows), `scroll`,
//! `input` (platform text input), `handlers` (actions), `mouse`, `popup`,
//! `view_state`, `compact`, `stats`.

use std::{
    cell::{Cell, RefCell},
    collections::BTreeSet,
    time::{Duration, Instant},
};

use gpui::{
    App, Context, EventEmitter, FocusHandle, Focusable, IntoElement, KeyContext, Keystroke,
    ModifiersChangedEvent, Pixels, Point, Render, SharedString, Subscription, Task, Window, div,
    prelude::*,
};
use writer_core::{
    CommentId, Document, EditorOptions, EditorState, Position, Selection, Stats, TextRange,
};
use writer_ui::theme::GlobalTheme;

use crate::{
    actions::KEY_CONTEXT,
    config::{CaretStyle, EditorConfig},
    decorations::CheckboxHit,
    element::EditorElement,
    events::EditorEvent,
    handlers,
    highlights::{HighlightStyle, Highlights},
    input::MarkedText,
    intercept::{self, InterceptPriority, Interceptors, KeyDisposition},
    layout::{DocumentLayout, ScrollPosition},
    mouse::Drag,
    movement::RowOracle,
    popup::{Popup, PopupElement},
    scroll::{Autoscroll, PendingScroll, Scrollbar},
};

/// Half a caret blink: how long the caret is shown, then hidden.
const BLINK_INTERVAL: Duration = Duration::from_millis(530);
/// The background layout works for this long, then lets the app breathe.
const REFINE_SLICE: Duration = Duration::from_millis(4);
const REFINE_PAUSE: Duration = Duration::from_millis(1);
/// Most blocks one slice handles (what bounds a slice in tests, where
/// time does not).
const REFINE_BLOCKS: usize = 256;

/// Timings of the editor's last frame, for performance tests and the demo.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FramePerf {
    /// Layout and planning (the element's prepaint).
    pub prepaint: Duration,
    /// Painting (the element's paint).
    pub paint: Duration,
    /// Frames drawn so far.
    pub frames: u64,
}

impl FramePerf {
    pub(crate) fn with_prepaint(mut self, prepaint: Duration) -> Self {
        self.prepaint = prepaint;
        self.frames += 1;
        self
    }

    pub(crate) fn with_paint(mut self, paint: Duration) -> Self {
        self.paint = paint;
        self
    }
}

/// The rich text editor: a view over a [`writer_core::EditorState`].
///
/// Create it with [`Editor::new`] inside `cx.new(..)`, put the entity in a
/// container (`div().size_full().child(editor.clone())`), and call
/// [`crate::init`] once at startup so its key bindings exist.
/// `writer_ui::theme::init` must have run before an editor is created.
///
/// Everything other crates do with an editor goes through the methods of
/// this type; the crate documentation groups them by seam.
pub struct Editor {
    pub(crate) state: EditorState,
    pub(crate) config: EditorConfig,
    pub(crate) focus_handle: FocusHandle,
    /// Block measures, shaped layouts, placement, the scroll position and
    /// the word counts. Behind a `RefCell` so geometry queries work through
    /// `&Editor` and can lay out on demand.
    pub(crate) layout: RefCell<DocumentLayout>,
    /// A scroll request waiting for the next layout.
    pub(crate) pending_scroll: Option<PendingScroll>,
    /// Column-local x that vertical caret movement tries to keep.
    pub(crate) goal_x: Option<Pixels>,
    /// Where the caret is drawn instead of the selection head.
    pub(crate) caret_override: Option<Position>,
    /// The caret's affinity: a selection head that sits at a soft wrap is
    /// drawn at the end of the row that ends there, not at the start of
    /// the next one (after `cmd-right`, a click past the end of a wrapped
    /// row, `up` / `down` into a shorter row). View state: positions in
    /// writer-core have no affinity. Reset whenever the head moves or the
    /// text changes; see `Editor::caret_is_upstream`.
    pub(crate) caret_upstream: bool,
    /// IME composition in progress.
    pub(crate) marked: Option<MarkedText>,
    /// Set while the platform input handler mutates the state, so the
    /// mutation seam does not end the composition it is part of.
    pub(crate) in_ime_edit: bool,
    pub(crate) interceptors: Interceptors,
    pub(crate) text_input_enabled: bool,
    /// Extra key context entries from consumers: `(key, Some(value))` is
    /// `key=value`, `(name, None)` is a bare identifier.
    pub(crate) context_entries: Vec<(SharedString, Option<SharedString>)>,
    pub(crate) highlights: Highlights,
    pub(crate) active_comment: Option<CommentId>,
    /// Threads whose anchors are not tinted.
    pub(crate) hidden_comments: BTreeSet<CommentId>,
    pub(crate) popup: Option<Popup>,
    /// Stands for the popup in the focus tree: whatever is focused inside
    /// the popup's view is inside this handle. Never focused itself.
    pub(crate) popup_scope: FocusHandle,
    /// The keyboard is in the popup and the editor counts as focused: no
    /// `Blurred` was emitted when it went there.
    focus_in_popup: bool,
    pub(crate) selection_stats: Cell<Option<(u64, Selection, Stats)>>,
    /// A mouse drag selection is in progress.
    pub(crate) drag: Option<Drag>,
    /// Scrolls while a drag selection is outside the viewport.
    pub(crate) drag_scroll: Option<Task<()>>,
    pub(crate) drag_scrolling: bool,
    /// The last pointer position of a drag, in window coordinates.
    pub(crate) last_mouse: Point<Pixels>,
    pub(crate) scrollbar: Scrollbar,
    /// The to-do checkboxes of the last frame.
    pub(crate) checkbox_hits: Vec<CheckboxHit>,
    /// The pointer is over a link with `cmd` held.
    pub(crate) link_hover: bool,
    /// The editor's focus handle is focused (as of the last focus event).
    focused: bool,
    /// Paint the caret and the selection as in an active window even while
    /// the window is in the background.
    pub(crate) draw_as_active: bool,
    /// The caret is in the visible half of its blink.
    pub(crate) blink_on: bool,
    /// The last frame painted a caret.
    pub(crate) caret_painted: bool,
    blink_task: Option<Task<()>>,
    /// Ends the typing undo step after a pause.
    pub(crate) typing_pause: Option<Task<()>>,
    /// The background layout task, while it has work.
    refine_task: Option<Task<()>>,
    refining: bool,
    refine_slices: usize,
    pub(crate) perf: Cell<FramePerf>,
    /// The selection rectangles of the last frame, in window coordinates.
    #[cfg(test)]
    pub(crate) painted_selection: Vec<gpui::Bounds<Pixels>>,
    /// The labels of the last frame (list numbers, page numbers, ...).
    #[cfg(test)]
    pub(crate) painted_markers: Vec<gpui::Bounds<Pixels>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<EditorEvent> for Editor {}

impl Focusable for Editor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

// ---------------------------------------------------------------------------
// Construction and configuration
// ---------------------------------------------------------------------------

impl Editor {
    /// An editor for `document`.
    ///
    /// ```ignore
    /// let editor = cx.new(|cx| Editor::new(document, EditorConfig::full(), window, cx));
    /// let composer = cx.new(|cx| {
    ///     Editor::new(
    ///         Document::new(),
    ///         EditorConfig::compact(CompactOptions::default()).placeholder("Add a comment"),
    ///         window,
    ///         cx,
    ///     )
    /// });
    /// ```
    pub fn new(
        document: Document,
        config: EditorConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // A compact editor is a field among others: `tab` reaches it.
        let focus_handle = cx.focus_handle().tab_stop(config.mode.is_compact());
        let interceptors = Interceptors::default();
        let mut state = EditorState::new(document);
        state.set_options(options_of(&config));
        let layout = DocumentLayout::new(state.document(), state.revision());

        let popup_scope = cx.focus_handle();
        // `Focused` and `Blurred` are about the editor as a whole: its
        // text and its popup. The keyboard moving between those two is
        // neither. The four listeners below can run in any order for one
        // focus change; `focused` and `focus_in_popup` make each of them
        // give the same answer whichever comes first.
        let subscriptions = vec![
            cx.on_focus(&focus_handle, window, |this, _, cx| {
                this.focused = true;
                this.restart_blink(cx);
                // Back from the popup: the editor never lost the keyboard.
                if !std::mem::take(&mut this.focus_in_popup) {
                    cx.emit(EditorEvent::Focused);
                }
                cx.notify();
            }),
            cx.on_blur(&focus_handle, window, |this, window, cx| {
                this.focused = false;
                this.blink_task = None;
                this.blink_on = true;
                this.end_composition();
                this.drag = None;
                this.drag_scroll = None;
                this.drag_scrolling = false;
                this.link_hover = false;
                this.state.break_undo_group();
                if this.popup_scope.contains_focused(window, cx) {
                    // Into the editor's own popup (a link field, the
                    // format menu's filter): that is not leaving.
                    this.focus_in_popup = true;
                } else {
                    cx.emit(EditorEvent::Blurred);
                }
                cx.notify();
            }),
            // The keyboard left the popup for somewhere that is not the
            // text (or the window was deactivated with it there).
            cx.on_focus_out(&popup_scope, window, |this, _, window, cx| {
                if this.focus_in_popup && !this.focus_handle.is_focused(window) {
                    this.focus_in_popup = false;
                    cx.emit(EditorEvent::Blurred);
                }
            }),
            // The keyboard came into the popup: from the text (nothing to
            // report), or from outside the editor.
            cx.on_focus_in(&popup_scope, window, |this, _, cx| {
                if !this.focus_in_popup {
                    this.focus_in_popup = true;
                    if !this.focused {
                        cx.emit(EditorEvent::Focused);
                    }
                }
            }),
            // The caret and the selection colour follow the window's
            // active state.
            cx.observe_window_activation(window, |this, window, cx| {
                if window.is_window_active() {
                    this.restart_blink(cx);
                } else {
                    this.blink_task = None;
                }
                cx.notify();
            }),
            // The one interception point, see `intercept`. It holds the
            // focus handle and the shared registry, not the entity.
            cx.intercept_keystrokes({
                let focus_handle = focus_handle.clone();
                let interceptors = interceptors.clone();
                move |event, window, cx| {
                    if interceptors.is_empty()
                        || !focus_handle.is_focused(window)
                        || intercept::is_synthetic_keystroke(&event.keystroke)
                    {
                        return;
                    }
                    if interceptors.dispatch(&event.keystroke, window, cx)
                        == KeyDisposition::Consumed
                    {
                        cx.stop_propagation();
                    }
                }
            }),
            // Colours and fonts are part of the shaped runs; the next frame
            // compares styles and drops the layout cache if they differ.
            cx.observe_global::<GlobalTheme>(|_, cx| cx.notify()),
        ];

        let mut editor = Self {
            state,
            config,
            focus_handle,
            layout: RefCell::new(layout),
            pending_scroll: None,
            goal_x: None,
            caret_override: None,
            caret_upstream: false,
            marked: None,
            in_ime_edit: false,
            interceptors,
            text_input_enabled: true,
            context_entries: Vec::new(),
            highlights: Highlights::default(),
            active_comment: None,
            hidden_comments: BTreeSet::new(),
            popup: None,
            popup_scope,
            focus_in_popup: false,
            selection_stats: Cell::new(None),
            drag: None,
            drag_scroll: None,
            drag_scrolling: false,
            last_mouse: Point::default(),
            scrollbar: Scrollbar::default(),
            checkbox_hits: Vec::new(),
            link_hover: false,
            focused: false,
            // A scripted run stays in the background (so it does not take
            // the keyboard from whoever is at the machine); its screenshots
            // should still show the caret.
            draw_as_active: std::env::var_os("THE_WRITER_AUTOMATION").is_some(),
            blink_on: true,
            caret_painted: false,
            blink_task: None,
            typing_pause: None,
            refine_task: None,
            refining: false,
            refine_slices: 0,
            perf: Cell::new(FramePerf::default()),
            #[cfg(test)]
            painted_selection: Vec::new(),
            #[cfg(test)]
            painted_markers: Vec::new(),
            _subscriptions: subscriptions,
        };
        // A long document: its words are counted in the background.
        if editor.layout.borrow().pending() {
            editor.start_refining(cx);
        }
        editor
    }

    /// The current configuration.
    pub fn config(&self) -> &EditorConfig {
        &self.config
    }

    /// Replaces the configuration.
    pub fn set_config(&mut self, config: EditorConfig, cx: &mut Context<Self>) {
        if self.config == config {
            return;
        }
        self.config = config;
        self.state.set_options(options_of(&self.config));
        if self.config.read_only {
            self.end_composition();
        }
        cx.notify();
    }

    /// Changes the configuration in place:
    /// `editor.update_config(|config| config.read_only = true, cx)`.
    pub fn update_config(&mut self, f: impl FnOnce(&mut EditorConfig), cx: &mut Context<Self>) {
        let mut config = self.config.clone();
        f(&mut config);
        self.set_config(config, cx);
    }

    /// Sets how the caret is drawn (Vim switches this with its mode).
    pub fn set_caret_style(&mut self, style: CaretStyle, cx: &mut Context<Self>) {
        self.update_config(|config| config.caret_style = style, cx);
    }

    /// Draws the caret at `position` instead of at the selection head, in
    /// the current [`CaretStyle`] and with the usual blink, also while the
    /// selection is not collapsed (a bar caret is otherwise hidden then).
    /// `None` puts it back on the head. Vim's Visual mode uses this: the
    /// editor selection ends one character past the Vim cursor, and the
    /// cursor cell belongs on the last selected character.
    ///
    /// The caret is painted over the selection and the text, so the
    /// selection tint does not cover it. Everything that means "the caret"
    /// follows the override: [`Editor::caret_bounds`],
    /// [`Editor::caret_position`], popups anchored to the caret,
    /// [`Editor::scroll_to_caret`] and the autoscroll of
    /// [`Editor::edit`]. The selection, typing and the platform input
    /// handler do not: they keep working on the selection.
    ///
    /// The override stays until it is cleared or the document is replaced
    /// ([`Editor::set_document`]). It does not follow edits or selection
    /// changes (set it again after each of your own changes and on
    /// [`EditorEvent::SelectionChanged`]); a position that no longer
    /// exists is clamped to the document.
    ///
    /// ```ignore
    /// // Visual mode, cursor on the last selected character:
    /// editor.edit_with(Autoscroll::None, |state| state.set_selection(selection), cx);
    /// editor.set_caret_style(CaretStyle::Block, cx);
    /// editor.set_caret_override(Some(cursor), cx);
    /// editor.scroll_to_caret(Autoscroll::Fit, cx);
    /// // Leaving Visual mode:
    /// editor.set_caret_override(None, cx);
    /// ```
    pub fn set_caret_override(&mut self, position: Option<Position>, cx: &mut Context<Self>) {
        if self.caret_override == position {
            return;
        }
        self.caret_override = position;
        // A caret that moved is shown at once.
        self.restart_blink(cx);
        cx.notify();
    }

    /// The position set with [`Editor::set_caret_override`], as it was
    /// given.
    pub fn caret_override(&self) -> Option<Position> {
        self.caret_override
    }

    /// Where the caret is drawn: the caret override (clamped to the
    /// document) when one is set, the selection head otherwise.
    pub fn caret_position(&self) -> Position {
        match self.caret_override {
            Some(position) => self.state.document().clamp_position(position),
            None => self.state.selection().head,
        }
    }

    /// Shows or hides the comment tints.
    pub fn set_comments_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.update_config(|config| config.comments_visible = visible, cx);
    }

    /// Makes the editor read-only or editable.
    pub fn set_read_only(&mut self, read_only: bool, cx: &mut Context<Self>) {
        self.update_config(|config| config.read_only = read_only, cx);
    }

    /// Whether the user can change the document.
    pub fn is_read_only(&self) -> bool {
        self.config.read_only
    }

    /// Paints the caret and the active selection colour even while the
    /// window is in the background. A background window normally shows no
    /// caret and a grey selection, as macOS text views do. Scripted runs
    /// (the `THE_WRITER_AUTOMATION` environment variable is set) stay in
    /// the background on purpose and take screenshots, so there this is on
    /// from the start; the caret then does not blink either, because focus
    /// events only reach active windows.
    pub fn set_draw_as_active(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.draw_as_active != on {
            self.draw_as_active = on;
            cx.notify();
        }
    }

    /// Replaces the whole document: new undo history, caret at the start,
    /// scrolled to the top, highlight layers, the caret override and the
    /// active comment cleared, the popup hidden. Emits [`EditorEvent::DocumentReplaced`] and nothing
    /// else. Restore a saved view afterwards with
    /// [`Editor::restore_view_state`].
    pub fn set_document(&mut self, document: Document, cx: &mut Context<Self>) {
        self.marked = None;
        self.typing_pause = None;
        self.state = EditorState::new(document);
        self.state.set_options(options_of(&self.config));
        self.layout
            .borrow_mut()
            .reset(self.state.document(), self.state.revision());
        self.pending_scroll = None;
        self.goal_x = None;
        self.caret_override = None;
        self.caret_upstream = false;
        self.drag = None;
        self.drag_scroll = None;
        self.drag_scrolling = false;
        self.checkbox_hits.clear();
        self.highlights.clear_all();
        self.active_comment = None;
        self.hidden_comments.clear();
        self.popup = None;
        self.selection_stats.set(None);
        if self.layout.borrow().pending() {
            self.start_refining(cx);
        }
        cx.emit(EditorEvent::DocumentReplaced);
        cx.notify();
    }

    /// Whether the document is shown as pages. This is the document's own
    /// setting (`DocumentSettings::page_view`); compact editors ignore it.
    pub fn page_view(&self) -> bool {
        self.state.document().settings().page_view
    }

    /// Turns page view on or off (a document settings change: emits
    /// [`EditorEvent::SettingsChanged`], not undoable). A caret that is on
    /// screen stays where it is on screen; otherwise the text at the top
    /// of the viewport stays there.
    pub fn set_page_view(&mut self, on: bool, cx: &mut Context<Self>) {
        if on == self.page_view() {
            return;
        }
        // At the very top the top stays (the first sheet shows its margin).
        let at_top = self.layout.borrow().scroll_position() == ScrollPosition::Top;
        let caret_y = self.caret_viewport_y().filter(|_| !at_top);
        self.edit_with(Autoscroll::None, |state| state.set_page_view(on), cx);
        if let Some(viewport_y) = caret_y {
            // The layout changes shape with the next frame; the request is
            // applied then.
            self.defer_scroll(PendingScroll::CaretAt { viewport_y });
        }
        cx.notify();
    }

    /// Switches between flow view and page view.
    pub fn toggle_page_view(&mut self, cx: &mut Context<Self>) {
        self.set_page_view(!self.page_view(), cx);
    }
}

fn options_of(config: &EditorConfig) -> EditorOptions {
    EditorOptions {
        input_rules: config.input_rules,
        smart_punctuation: config.smart_punctuation,
    }
}

// ---------------------------------------------------------------------------
// Engine access and the mutation seam
// ---------------------------------------------------------------------------

impl Editor {
    /// The editing engine, read-only. Mutate it through [`Editor::edit`].
    pub fn state(&self) -> &EditorState {
        &self.state
    }

    /// The document.
    pub fn document(&self) -> &Document {
        self.state.document()
    }

    /// The selection (shorthand for `state().selection()`).
    pub fn selection(&self) -> Selection {
        self.state.selection()
    }

    /// The one way to change the document or the selection from outside.
    ///
    /// Runs `f` on the [`EditorState`], then does everything a change needs:
    /// consumes writer-core's change tracking, brings the block layouts up
    /// to date, refreshes the statistics, emits [`EditorEvent::Edited`] /
    /// [`EditorEvent::SettingsChanged`] / [`EditorEvent::SelectionChanged`]
    /// as applicable, scrolls the caret into view and notifies. When `f`
    /// changes nothing, nothing is emitted. Returns what `f` returns.
    ///
    /// ```ignore
    /// editor.update(cx, |editor, cx| {
    ///     editor.edit(|state| state.toggle_bold(), cx);
    ///     let id = editor.edit(|state| state.add_comment("Tighten this", "Me", now), cx);
    /// });
    /// ```
    ///
    /// One call is one batch of events, however many commands `f` runs; wrap
    /// them in `state.grouped(|state| ..)` to make them one undo step too.
    /// `edit` is not blocked by read-only mode: check
    /// [`Editor::is_read_only`] before running a user command through it.
    pub fn edit<R>(&mut self, f: impl FnOnce(&mut EditorState) -> R, cx: &mut Context<Self>) -> R {
        self.edit_with(Autoscroll::Fit, f, cx)
    }

    /// [`Editor::edit`] with a choice of scrolling: [`Autoscroll::None`]
    /// leaves the view where it is (use it for changes that are not at the
    /// caret, such as resolving a comment), the others say where the caret
    /// should end up.
    pub fn edit_with<R>(
        &mut self,
        autoscroll: Autoscroll,
        f: impl FnOnce(&mut EditorState) -> R,
        cx: &mut Context<Self>,
    ) -> R {
        let selection_before = self.state.selection();
        let result = f(&mut self.state);
        self.finish_edit(selection_before, autoscroll, cx);
        result
    }

    /// [`Editor::edit_with`] for callers that need layout answers while
    /// they hold the state: `f` also gets a [`RowOracle`], which answers
    /// the visual-row questions ([`Editor::visual_row`],
    /// [`Editor::column_x`], [`Editor::move_by_visual_rows`],
    /// [`Editor::visible_rows`], [`Editor::rows_per_page`]) for the state
    /// it is handed. The Vim engine runs inside this: it mutates the state
    /// and asks about rows during the same call.
    ///
    /// Everything [`Editor::edit`] guarantees holds: one batch of events
    /// for the whole call, layout and statistics brought up to date, the
    /// caret scrolled as `autoscroll` says. The oracle sees the scroll
    /// position from before the call (scrolling is applied after `f`
    /// returns) and answers for the document as it is when asked, also
    /// after `f` has changed it; asking first and editing afterwards is
    /// the cheap order (a question after a change costs a pass over the
    /// blocks). Before the first layout the oracle answers `None`.
    ///
    /// ```ignore
    /// let consumed = editor.update(cx, |editor, cx| {
    ///     editor.edit_with_rows(
    ///         Autoscroll::Fit,
    ///         |state, rows| {
    ///             // rows.visual_row(state, pos), rows.column_x(state, pos),
    ///             // rows.move_by_visual_rows(state, pos, 3, goal_x),
    ///             // rows.visible_rows(state), rows.rows_per_page()
    ///             let mut host = MyHost { rows };
    ///             vim.handle_key(key, state, &mut host)
    ///         },
    ///         cx,
    ///     )
    /// });
    /// ```
    pub fn edit_with_rows<R>(
        &mut self,
        autoscroll: Autoscroll,
        f: impl FnOnce(&mut EditorState, &RowOracle<'_>) -> R,
        cx: &mut Context<Self>,
    ) -> R {
        let selection_before = self.state.selection();
        // The oracle borrows the layout and knows the state by address
        // only, so the state itself can be lent out mutably next to it.
        let rows = RowOracle::new(&self.layout, std::ptr::addr_of!(self.state));
        let result = f(&mut self.state, &rows);
        self.finish_edit(selection_before, autoscroll, cx);
        result
    }

    /// The second half of the mutation seam, shared by every path that
    /// touches `self.state` (outside edits, actions, text input, the mouse).
    pub(crate) fn finish_edit(
        &mut self,
        selection_before: Selection,
        autoscroll: Autoscroll,
        cx: &mut Context<Self>,
    ) {
        self.finish_edit_announced(selection_before, autoscroll, None, cx);
    }

    /// [`Editor::finish_edit`] for the platform input handler: `input` (an
    /// [`EditorEvent::TextInput`]) is emitted ahead of `Edited` when the
    /// document changed, and dropped when it did not.
    pub(crate) fn finish_edit_announced(
        &mut self,
        selection_before: Selection,
        autoscroll: Autoscroll,
        input: Option<EditorEvent>,
        cx: &mut Context<Self>,
    ) {
        let change = self.state.take_changes();
        let selection_changed = self.state.selection() != selection_before;
        if change.is_none() && !selection_changed {
            return;
        }
        // The caret's affinity belongs to where the head was put: it goes
        // when the head moves or the text under it changes. (Whoever moved
        // the head to the end of a wrapped row sets it again afterwards.)
        if self.state.selection().head != selection_before.head
            || change.is_some_and(|change| change.blocks.is_some() || change.settings)
        {
            self.caret_upstream = false;
        }
        if let Some(change) = change {
            if change.blocks.is_some()
                && let Some(input) = input
            {
                cx.emit(input);
            }
            // The layout replaces the slots of the changed blocks (their
            // measures, word counts and placement); everything else stays.
            self.layout.borrow_mut().apply_change(
                self.state.document(),
                self.state.revision(),
                change.blocks,
            );
            if change.blocks.is_some() && !self.in_ime_edit {
                // An edit from elsewhere during a composition: the marked
                // range no longer means anything.
                self.end_composition();
            }
            if change.blocks.is_some() || change.comments {
                cx.emit(EditorEvent::Edited { change });
            }
            if change.settings {
                cx.emit(EditorEvent::SettingsChanged);
            }
            if self.layout.borrow().pending() {
                self.start_refining(cx);
            }
        }
        if selection_changed {
            self.goal_x = None;
            cx.emit(EditorEvent::SelectionChanged);
        }
        // The caret is solid while the user types or moves.
        self.restart_blink(cx);
        if autoscroll != Autoscroll::None {
            self.request_scroll(PendingScroll::Position(None, autoscroll));
        }
        cx.notify();
    }

    /// Runs a user command: like [`Editor::edit`], but does nothing (and
    /// returns `None`) in read-only mode.
    pub(crate) fn user_edit<R>(
        &mut self,
        f: impl FnOnce(&mut EditorState) -> R,
        cx: &mut Context<Self>,
    ) -> Option<R> {
        if self.config.read_only {
            return None;
        }
        Some(self.edit(f, cx))
    }
}

// ---------------------------------------------------------------------------
// Timers: caret blink, background layout
// ---------------------------------------------------------------------------

impl Editor {
    /// Shows the caret and starts its blink over. Does nothing but show it
    /// while the editor is not focused.
    pub(crate) fn restart_blink(&mut self, cx: &mut Context<Self>) {
        self.blink_on = true;
        if !self.focused {
            self.blink_task = None;
            return;
        }
        self.blink_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(BLINK_INTERVAL).await;
                let alive = this.update(cx, |editor, cx| {
                    editor.blink_on = !editor.blink_on;
                    cx.notify();
                });
                if alive.is_err() {
                    break;
                }
            }
        }));
    }

    /// Starts the background layout task unless it is running: it measures
    /// the blocks that only have an estimated height and counts words, a
    /// few milliseconds at a time, until nothing is left.
    pub(crate) fn start_refining(&mut self, cx: &mut Context<Self>) {
        if self.refining {
            return;
        }
        self.refining = true;
        self.refine_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(REFINE_PAUSE).await;
                let more = this.update(cx, |editor, cx| editor.refine_slice(cx));
                if !matches!(more, Ok(true)) {
                    break;
                }
            }
        }));
    }

    /// One slice of background layout. Returns whether work remains.
    fn refine_slice(&mut self, cx: &mut Context<Self>) -> bool {
        // Tests have no clock: there a slice is bounded by its block count.
        let deadline = (!cfg!(test)).then(|| Instant::now() + REFINE_SLICE);
        let (more, counted) = {
            let mut layout = self.layout.borrow_mut();
            let counting = !layout.stats_complete();
            let more = layout.refine(self.state.document(), deadline, REFINE_BLOCKS);
            (more, counting && layout.stats_complete())
        };
        if counted {
            cx.emit(EditorEvent::StatsChanged);
        }
        self.refine_slices += 1;
        if !more {
            self.refining = false;
        }
        // Estimates turning into measures move the scroll bar and the page
        // count; a few repaints while that happens are enough.
        if !more || counted || self.refine_slices.is_multiple_of(8) {
            cx.notify();
        }
        more
    }

    /// Timings of the last frame.
    #[doc(hidden)]
    pub fn frame_perf(&self) -> FramePerf {
        self.perf.get()
    }

    /// Finishes the background layout now: every block measured, every
    /// word counted. For tests and benchmarks; the app never needs it.
    #[doc(hidden)]
    pub fn finish_layout(&mut self, cx: &mut Context<Self>) {
        let counted = {
            let mut layout = self.layout.borrow_mut();
            let counting = !layout.stats_complete();
            layout.measure_all(self.state.document());
            counting && layout.stats_complete()
        };
        if counted {
            cx.emit(EditorEvent::StatsChanged);
        }
        cx.notify();
    }

    /// `cmd` went down or up: the cursor over a link changes.
    fn modifiers_changed(
        &mut self,
        event: &ModifiersChangedEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let over_link = event.modifiers.platform
            && self
                .viewport_bounds()
                .is_some_and(|viewport| viewport.contains(&window.mouse_position()))
            && self.link_at_point(window.mouse_position()).is_some();
        if over_link != self.link_hover {
            self.link_hover = over_link;
            cx.notify();
        }
    }
}

// ---------------------------------------------------------------------------
// Focus, key interception, key context
// ---------------------------------------------------------------------------

impl Editor {
    /// Whether the editor's own focus handle is focused.
    pub fn is_focused(&self, window: &Window) -> bool {
        self.focus_handle.is_focused(window)
    }

    /// Moves focus to the editor.
    pub fn focus(&self, window: &mut Window) {
        window.focus(&self.focus_handle);
    }

    /// Registers a key interceptor: `handler` sees every key-down while this
    /// editor is focused, before the editor's key bindings and before text
    /// input, and returns [`KeyDisposition::Consumed`] to swallow the key or
    /// [`KeyDisposition::Pass`] to let it continue. Interceptors run in
    /// [`InterceptPriority`] order (higher first, newest first among
    /// equals). Dropping the returned subscription removes the interceptor;
    /// keep it for as long as the menu is open or the mode is on.
    ///
    /// The handler runs outside any entity update, so it may `update` this
    /// editor and any other entity. See the crate docs for the full
    /// contract and for what is not offered.
    ///
    /// ```ignore
    /// // A menu: navigation keys go to the menu, everything else to the document.
    /// let menu = menu.downgrade();
    /// self.keys = Some(editor.read(cx).intercept_keys(InterceptPriority::OVERLAY, move |key, window, cx| {
    ///     if key.modifiers.modified() { return KeyDisposition::Pass; }
    ///     match key.key.as_str() {
    ///         "up" | "down" | "enter" | "tab" | "escape" => {
    ///             menu.update(cx, |menu, cx| menu.handle_key(key, window, cx)).ok();
    ///             KeyDisposition::Consumed
    ///         }
    ///         _ => KeyDisposition::Pass,
    ///     }
    /// }));
    /// ```
    pub fn intercept_keys(
        &self,
        priority: InterceptPriority,
        handler: impl FnMut(&Keystroke, &mut Window, &mut App) -> KeyDisposition + 'static,
    ) -> Subscription {
        self.interceptors.register(priority, handler)
    }

    /// Turns platform text input on or off. While off, the editor does not
    /// register its input handler, so nothing typed, composed by an IME,
    /// dictated or picked from the emoji palette reaches the document (Vim
    /// normal and visual mode). Key bindings and interceptors are not
    /// affected. On by default.
    pub fn set_text_input_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.text_input_enabled != enabled {
            self.text_input_enabled = enabled;
            if !enabled {
                self.end_composition();
            }
            cx.notify();
        }
    }

    /// Whether platform text input reaches the document.
    pub fn text_input_enabled(&self) -> bool {
        self.text_input_enabled
    }

    /// Adds `key=value` to the editor's key context (replacing an earlier
    /// value of `key`), so bindings registered with a predicate such as
    /// `"Editor && vim_mode == normal"` take part in normal dispatch. The
    /// new context applies from the next key on (GPUI redraws a dirty
    /// window before it dispatches a key).
    pub fn set_key_context_value(
        &mut self,
        key: impl Into<SharedString>,
        value: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.set_context_entry(key.into(), Some(value.into()), cx);
    }

    /// Adds a bare identifier to the editor's key context
    /// (`"Editor && vim"`).
    pub fn add_key_context_flag(&mut self, name: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.set_context_entry(name.into(), None, cx);
    }

    /// Removes an entry added with [`Editor::set_key_context_value`] or
    /// [`Editor::add_key_context_flag`].
    pub fn remove_key_context_entry(&mut self, key: &str, cx: &mut Context<Self>) {
        let before = self.context_entries.len();
        self.context_entries
            .retain(|(entry, _)| entry.as_ref() != key);
        if self.context_entries.len() != before {
            cx.notify();
        }
    }

    fn set_context_entry(
        &mut self,
        key: SharedString,
        value: Option<SharedString>,
        cx: &mut Context<Self>,
    ) {
        match self
            .context_entries
            .iter_mut()
            .find(|(entry, _)| *entry == key)
        {
            Some((_, existing)) if *existing == value => return,
            Some((_, existing)) => *existing = value,
            None => self.context_entries.push((key, value)),
        }
        cx.notify();
    }

    /// The key context the editor renders with: `Editor`, `mode=full` or
    /// `mode=compact`, `read_only` when read-only, plus the consumer
    /// entries.
    pub fn key_context(&self) -> KeyContext {
        let mut context = KeyContext::default();
        context.add(KEY_CONTEXT);
        context.set(
            "mode",
            if self.config.mode.is_compact() {
                "compact"
            } else {
                "full"
            },
        );
        if self.config.read_only {
            context.add("read_only");
        }
        for (key, value) in &self.context_entries {
            match value {
                Some(value) => context.set(key.clone(), value.clone()),
                None => context.add(key.clone()),
            }
        }
        context
    }
}

// ---------------------------------------------------------------------------
// Highlight layers and comments
// ---------------------------------------------------------------------------

impl Editor {
    /// Sets the ranges and the style of the highlight layer `layer`,
    /// creating it if needed. Layers paint behind the text in the order
    /// they were first set, so set "find" before "find-current". Ranges do
    /// not follow edits: set them again on [`EditorEvent::Edited`].
    ///
    /// ```ignore
    /// let colors = cx.theme().colors;
    /// editor.set_highlights("find", matches, HighlightStyle::background(colors.find_match), cx);
    /// editor.set_highlights("find-current", vec![current], HighlightStyle::background(colors.find_match_current), cx);
    /// editor.clear_highlights("find", cx);
    /// ```
    pub fn set_highlights(
        &mut self,
        layer: impl Into<SharedString>,
        ranges: Vec<TextRange>,
        style: HighlightStyle,
        cx: &mut Context<Self>,
    ) {
        self.highlights.set(layer.into(), ranges, style);
        cx.notify();
    }

    /// Removes a highlight layer.
    pub fn clear_highlights(&mut self, layer: &str, cx: &mut Context<Self>) {
        if self.highlights.clear(layer) {
            cx.notify();
        }
    }

    /// The ranges of a highlight layer, sorted by start (empty when the
    /// layer does not exist).
    pub fn highlights(&self, layer: &str) -> &[TextRange] {
        self.highlights
            .get(layer)
            .map_or(&[], |layer| layer.ranges.as_slice())
    }

    /// Marks one comment thread as the active one: its anchored text gets
    /// the stronger tint. `None` clears it. Tints are drawn only while
    /// [`EditorConfig::comments_visible`] is on.
    pub fn set_active_comment(&mut self, comment: Option<CommentId>, cx: &mut Context<Self>) {
        if self.active_comment != comment {
            self.active_comment = comment;
            cx.notify();
        }
    }

    /// The active comment thread.
    pub fn active_comment(&self) -> Option<CommentId> {
        self.active_comment
    }

    /// Sets the comment threads whose anchored text is *not* tinted (for
    /// example resolved threads the comments panel is hiding). Text that
    /// also carries a thread outside the set keeps its tint. The set is
    /// kept until it is replaced or the document is replaced
    /// ([`Editor::set_document`] clears it); it does not follow edits, so
    /// set it again when the list of threads changes.
    ///
    /// ```ignore
    /// let resolved = editor.document().comments().filter(|thread| thread.resolved).map(|thread| thread.id);
    /// editor.set_hidden_comments(resolved.collect::<Vec<_>>(), cx);
    /// ```
    pub fn set_hidden_comments(
        &mut self,
        comments: impl IntoIterator<Item = CommentId>,
        cx: &mut Context<Self>,
    ) {
        let hidden: BTreeSet<CommentId> = comments.into_iter().collect();
        if hidden != self.hidden_comments {
            self.hidden_comments = hidden;
            cx.notify();
        }
    }

    /// The comment threads whose anchors are not tinted.
    pub fn hidden_comments(&self) -> impl Iterator<Item = CommentId> + '_ {
        self.hidden_comments.iter().copied()
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

impl Render for Editor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let compact = self.config.mode.is_compact();
        // The surface carries the key context, the focus and the action
        // handlers. The popup is a sibling, outside the "Editor" context, so
        // a focused popup does not trigger editor bindings.
        let surface = div()
            .key_context(self.key_context())
            .track_focus(&self.focus_handle)
            .on_modifiers_changed(cx.listener(Self::modifiers_changed))
            .w_full()
            .when(!compact, |surface| surface.h_full());
        let surface = handlers::register(surface, cx).child(EditorElement::new(cx.entity()));
        div()
            .relative()
            .w_full()
            .when(!compact, |root| root.h_full())
            .child(surface)
            .child(PopupElement::new(cx.entity()))
    }
}
