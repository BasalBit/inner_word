//! The [`Editor`] entity: state, configuration, the mutation seam, events.
//!
//! The rest of the entity's behaviour is spread over sibling modules, each
//! an `impl Editor` block: `geometry`, `movement` (visual rows), `scroll`,
//! `input` (platform text input), `handlers` (actions), `mouse`, `popup`,
//! `view_state`, `compact`, `stats`.

use std::cell::{Cell, RefCell};

use gpui::{
    App, Context, EventEmitter, FocusHandle, Focusable, IntoElement, KeyContext, Keystroke,
    Pixels, Render, SharedString, Subscription, Window, div, prelude::*, px,
};
use writer_core::{
    CommentId, Document, EditorOptions, EditorState, Selection, Stats, TextRange,
};
use writer_ui::theme::GlobalTheme;

use crate::{
    actions::KEY_CONTEXT,
    config::{CaretStyle, EditorConfig},
    element::EditorElement,
    events::EditorEvent,
    handlers,
    highlights::{HighlightStyle, Highlights},
    input::MarkedText,
    intercept::{self, InterceptPriority, Interceptors, KeyDisposition},
    layout::DocumentLayout,
    popup::{Popup, PopupElement},
    scroll::{Autoscroll, PendingScroll},
};

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
    /// Block layouts and the geometry of the last frame. Behind a `RefCell`
    /// so geometry queries work through `&Editor` and can lay out on demand.
    pub(crate) layout: RefCell<DocumentLayout>,
    /// How far the content is scrolled, in pixels from its top.
    pub(crate) scroll_y: Pixels,
    /// A scroll request waiting for the next layout.
    pub(crate) pending_scroll: Option<PendingScroll>,
    /// Column-local x that vertical caret movement tries to keep.
    pub(crate) goal_x: Option<Pixels>,
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
    pub(crate) popup: Option<Popup>,
    pub(crate) stats: Stats,
    pub(crate) selection_stats: Cell<Option<(u64, Selection, Stats)>>,
    /// A mouse drag selection is in progress.
    pub(crate) selecting: bool,
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
        let focus_handle = cx.focus_handle();
        let interceptors = Interceptors::default();
        let mut state = EditorState::new(document);
        state.set_options(options_of(&config));
        let stats = state.stats();

        let subscriptions = vec![
            cx.on_focus(&focus_handle, window, |_, _, cx| {
                cx.emit(EditorEvent::Focused);
                cx.notify();
            }),
            cx.on_blur(&focus_handle, window, |this, _, cx| {
                this.end_composition();
                this.selecting = false;
                this.state.break_undo_group();
                cx.emit(EditorEvent::Blurred);
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

        Self {
            state,
            config,
            focus_handle,
            layout: RefCell::new(DocumentLayout::default()),
            scroll_y: px(0.),
            pending_scroll: None,
            goal_x: None,
            marked: None,
            in_ime_edit: false,
            interceptors,
            text_input_enabled: true,
            context_entries: Vec::new(),
            highlights: Highlights::default(),
            active_comment: None,
            popup: None,
            stats,
            selection_stats: Cell::new(None),
            selecting: false,
            _subscriptions: subscriptions,
        }
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

    /// Replaces the whole document: new undo history, caret at the start,
    /// scrolled to the top, highlight layers and the active comment cleared,
    /// the popup hidden. Emits [`EditorEvent::DocumentReplaced`] and nothing
    /// else. Restore a saved view afterwards with
    /// [`Editor::restore_view_state`].
    pub fn set_document(&mut self, document: Document, cx: &mut Context<Self>) {
        self.marked = None;
        self.state = EditorState::new(document);
        self.state.set_options(options_of(&self.config));
        self.layout.borrow_mut().clear();
        self.scroll_y = px(0.);
        self.pending_scroll = None;
        self.goal_x = None;
        self.selecting = false;
        self.highlights.clear_all();
        self.active_comment = None;
        self.popup = None;
        self.stats = self.state.stats();
        self.selection_stats.set(None);
        cx.emit(EditorEvent::DocumentReplaced);
        cx.notify();
    }

    /// Whether the document is shown as pages. This is the document's own
    /// setting (`DocumentSettings::page_view`); compact editors ignore it.
    pub fn page_view(&self) -> bool {
        self.state.document().settings().page_view
    }

    /// Turns page view on or off (a document settings change: emits
    /// [`EditorEvent::SettingsChanged`], not undoable).
    pub fn set_page_view(&mut self, on: bool, cx: &mut Context<Self>) {
        self.edit_with(Autoscroll::Fit, |state| state.set_page_view(on), cx);
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
    pub fn edit<R>(
        &mut self,
        f: impl FnOnce(&mut EditorState) -> R,
        cx: &mut Context<Self>,
    ) -> R {
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

    /// The second half of the mutation seam, shared by every path that
    /// touches `self.state` (outside edits, actions, text input, the mouse).
    pub(crate) fn finish_edit(
        &mut self,
        selection_before: Selection,
        autoscroll: Autoscroll,
        cx: &mut Context<Self>,
    ) {
        let change = self.state.take_changes();
        let selection_changed = self.state.selection() != selection_before;
        if change.is_none() && !selection_changed {
            return;
        }
        if let Some(change) = change {
            // Block layouts are keyed by revision stamps, so only the stack
            // needs a rebuild; it happens on the next layout query or frame.
            self.layout.borrow_mut().invalidate();
            if change.blocks.is_some() {
                if !self.in_ime_edit {
                    // An edit from elsewhere during a composition: the
                    // marked range no longer means anything.
                    self.end_composition();
                }
                self.stats = self.state.stats();
            }
            if change.blocks.is_some() || change.comments {
                cx.emit(EditorEvent::Edited { change });
            }
            if change.settings {
                cx.emit(EditorEvent::SettingsChanged);
            }
        }
        if selection_changed {
            self.goal_x = None;
            cx.emit(EditorEvent::SelectionChanged);
        }
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
