//! The [`CommentsPanel`] entity: state and commands. Rendering is in
//! `render`.
//!
//! The panel keeps no comment data of its own. The threads live in the
//! document; [`CommentsPanel::threads`] is a snapshot that is rebuilt from
//! the editor's document after every change. Every change the panel makes
//! goes through `Editor::edit_with`, so it is one undo step in the
//! document's history.

use std::{rc::Rc, time::Duration};

use gpui::{
    App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, Pixels, ScrollHandle,
    SharedString, Subscription, Task, Window, px,
};
use writer_core::{CommentId, Document, EditorState, ReplyId, Selection, TextRange, Timestamp};
use writer_editor::{
    Autoscroll, CompactOptions, Editor, EditorConfig, EditorEvent, HighlightStyle,
    InterceptPriority, KeyDisposition,
};
use writer_ui::{Toast, theme::ActiveTheme, toast::show_toast};

use crate::{
    threads::{ThreadEntry, collect_threads, quote, word_range_at},
    time,
};

/// Default width of the panel.
pub const PANEL_WIDTH: Pixels = px(300.);

/// The editor highlight layer that marks the text a new comment will be
/// anchored to while its composer is open.
pub const DRAFT_HIGHLIGHT_LAYER: &str = "comment-draft";

/// How often the relative times ("5 min ago") are refreshed.
const TIME_REFRESH: Duration = Duration::from_secs(30);

/// What a [`CommentsPanel`] tells its subscribers.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum CommentsEvent {
    /// Comments were shown or hidden (the panel and the tints in the text).
    /// The app saves this flag and passes it back with
    /// [`CommentsPanel::set_visible`] on the next launch.
    VisibilityChanged(bool),
    /// Resolved threads were shown or hidden in the panel.
    ShowResolvedChanged(bool),
    /// Another thread became the active one, or none is active any more.
    ActiveThreadChanged(Option<CommentId>),
}

/// What the composer is writing.
#[derive(Clone, Debug, PartialEq)]
pub enum DraftKind {
    /// A new thread on `range` of the document.
    New {
        /// The text the thread will be anchored to.
        range: TextRange,
    },
    /// A reply to a thread.
    Reply {
        /// The thread replied to.
        thread: CommentId,
    },
    /// A new body for a thread's opening comment.
    EditComment {
        /// The thread edited.
        thread: CommentId,
    },
    /// A new body for a reply.
    EditReply {
        /// The thread of the reply.
        thread: CommentId,
        /// The reply edited.
        reply: ReplyId,
    },
}

impl DraftKind {
    /// The thread a reply or an edit belongs to; `None` for a new comment.
    pub fn thread(&self) -> Option<CommentId> {
        match *self {
            DraftKind::New { .. } => None,
            DraftKind::Reply { thread }
            | DraftKind::EditComment { thread }
            | DraftKind::EditReply { thread, .. } => Some(thread),
        }
    }

    fn placeholder(&self) -> &'static str {
        match self {
            DraftKind::New { .. } => "Add a comment",
            DraftKind::Reply { .. } => "Reply",
            DraftKind::EditComment { .. } | DraftKind::EditReply { .. } => "Edit comment",
        }
    }
}

pub(crate) struct Draft {
    pub(crate) kind: DraftKind,
    /// The quoted target of a new comment, for display.
    pub(crate) quote: String,
    /// Where focus goes when the composer closes: the thread list when the
    /// draft was started from it, the editor otherwise.
    return_to_list: bool,
    /// Whether comments were visible before the draft made them visible.
    was_visible: bool,
}

/// What a delete confirmation is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeleteTarget {
    /// A whole thread, with its replies.
    Thread(CommentId),
    /// One reply.
    Reply(CommentId, ReplyId),
}

impl DeleteTarget {
    /// The thread the target is, or belongs to.
    pub fn thread(&self) -> CommentId {
        match *self {
            DeleteTarget::Thread(thread) | DeleteTarget::Reply(thread, _) => thread,
        }
    }
}

#[derive(Clone, Copy)]
struct PendingDelete {
    target: DeleteTarget,
    return_to_editor: bool,
}

/// The comments panel: the list of a document's comment threads beside the
/// editor, with a composer for new comments, replies and edits.
///
/// See the [crate documentation](crate) for how the app mounts it.
pub struct CommentsPanel {
    pub(crate) editor: Entity<Editor>,
    author: SharedString,
    visible: bool,
    pub(crate) show_resolved: bool,
    pub(crate) width: Pixels,
    /// Every thread of the document: anchored ones in document order, then
    /// detached ones.
    pub(crate) threads: Vec<ThreadEntry>,
    pub(crate) active: Option<CommentId>,
    pub(crate) selected_reply: Option<ReplyId>,
    pub(crate) draft: Option<Draft>,
    confirming: Option<PendingDelete>,
    /// The one composer, a compact editor, shown wherever the draft is.
    pub(crate) composer: Entity<Editor>,
    /// Focus of the thread list.
    pub(crate) focus_handle: FocusHandle,
    pub(crate) scroll: ScrollHandle,
    clock: Rc<dyn Fn() -> Timestamp>,
    _editor_subscription: Subscription,
    _subscriptions: Vec<Subscription>,
    _refresh_times: Task<()>,
}

impl EventEmitter<CommentsEvent> for CommentsPanel {}

impl Focusable for CommentsPanel {
    /// The composer while one is open, the thread list otherwise. Prefer
    /// [`CommentsPanel::focus_panel`], which also shows the panel and picks
    /// an active thread.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        if self.draft.is_some() {
            self.composer.focus_handle(cx)
        } else {
            self.focus_handle.clone()
        }
    }
}

// ---------------------------------------------------------------------------
// Construction and configuration
// ---------------------------------------------------------------------------

impl CommentsPanel {
    /// A panel for the document in `editor`. New comments and replies are
    /// signed with `author`. Comments start visible; restore a saved choice
    /// with [`CommentsPanel::set_visible`].
    pub fn new(
        editor: Entity<Editor>,
        author: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let composer = cx.new(|cx| {
            Editor::new(
                Document::new(),
                EditorConfig::compact(CompactOptions::default().max_height(px(160.)))
                    .placeholder("Add a comment"),
                window,
                cx,
            )
        });
        let panel = cx.weak_entity();
        let subscriptions = vec![
            // `enter` in the composer (submit-on-enter).
            cx.subscribe_in(
                &composer,
                window,
                |this, _, event: &EditorEvent, window, cx| {
                    if matches!(event, EditorEvent::Submitted) {
                        this.submit(window, cx);
                    }
                },
            ),
            // `cmd-enter` and `escape` in the composer. The editor binds
            // `cmd-enter` itself (page break, a no-op in compact mode), and
            // an interceptor runs before its bindings whatever the order in
            // which the crates installed their keys.
            composer
                .read(cx)
                .intercept_keys(InterceptPriority::DEFAULT, move |key, window, cx| {
                    let modifiers = key.modifiers;
                    let only_cmd = modifiers.platform
                        && !modifiers.alt
                        && !modifiers.control
                        && !modifiers.shift;
                    match key.key.as_str() {
                        "enter" if only_cmd => {
                            panel.update(cx, |panel, cx| panel.submit(window, cx)).ok();
                            KeyDisposition::Consumed
                        }
                        "escape" if !modifiers.modified() => {
                            panel.update(cx, |panel, cx| panel.cancel(window, cx)).ok();
                            KeyDisposition::Consumed
                        }
                        // A comment is plain text: nothing to indent.
                        "tab" if !modifiers.platform && !modifiers.control && !modifiers.alt => {
                            KeyDisposition::Consumed
                        }
                        _ => KeyDisposition::Pass,
                    }
                }),
        ];
        let refresh_times = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TIME_REFRESH).await;
                let alive = this.update(cx, |this, cx| {
                    if this.is_open() {
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        });

        let mut this = Self {
            _editor_subscription: Self::subscribe_to_editor(&editor, window, cx),
            editor,
            author: author.into(),
            visible: true,
            show_resolved: false,
            width: PANEL_WIDTH,
            threads: Vec::new(),
            active: None,
            selected_reply: None,
            draft: None,
            confirming: None,
            composer,
            focus_handle: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            clock: Rc::new(time::now),
            _subscriptions: subscriptions,
            _refresh_times: refresh_times,
        };
        this.attach_editor(cx);
        this
    }

    fn subscribe_to_editor(
        editor: &Entity<Editor>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Subscription {
        cx.subscribe_in(editor, window, Self::on_editor_event)
    }

    /// Brings a (new) editor in line with the panel and reads its threads.
    fn attach_editor(&mut self, cx: &mut Context<Self>) {
        let visible = self.visible;
        self.editor.update(cx, |editor, cx| {
            editor.set_comments_visible(visible, cx);
            editor.set_active_comment(None, cx);
        });
        self.refresh_threads(cx);
    }

    /// The editor whose document the panel shows.
    pub fn editor(&self) -> &Entity<Editor> {
        &self.editor
    }

    /// Switches the panel to another editor (the app opened another
    /// document in a new editor entity). An open composer is discarded, no
    /// thread is active, and the new editor's comment tints follow
    /// [`CommentsPanel::is_visible`]. If the panel had keyboard focus, the
    /// new editor gets it.
    ///
    /// Not needed when the app keeps one editor and calls
    /// `Editor::set_document`: the panel follows `DocumentReplaced` by
    /// itself.
    pub fn set_editor(
        &mut self,
        editor: Entity<Editor>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editor == editor {
            return;
        }
        let had_focus = self.has_focus(window, cx);
        self.reset_for_new_document(cx);
        self.editor = editor;
        self._editor_subscription = Self::subscribe_to_editor(&self.editor, window, cx);
        self.attach_editor(cx);
        if had_focus {
            self.editor.read(cx).focus(window);
        }
        cx.notify();
    }

    /// The name new comments and replies are signed with.
    pub fn author(&self) -> &SharedString {
        &self.author
    }

    /// Changes the author name (the user renamed themselves in settings).
    /// Existing comments keep the name they were written under.
    pub fn set_author(&mut self, author: impl Into<SharedString>) {
        self.author = author.into();
    }

    /// The panel's width when it is open.
    pub fn width(&self) -> Pixels {
        self.width
    }

    /// Sets the panel's width.
    pub fn set_width(&mut self, width: Pixels, cx: &mut Context<Self>) {
        if self.width != width {
            self.width = width;
            cx.notify();
        }
    }

    /// Replaces the clock that stamps new comments and that relative times
    /// are measured against (milliseconds since the Unix epoch). The
    /// default is the system clock; tests install a fixed one.
    pub fn set_clock(&mut self, clock: impl Fn() -> Timestamp + 'static, cx: &mut Context<Self>) {
        self.clock = Rc::new(clock);
        cx.notify();
    }

    pub(crate) fn now(&self) -> Timestamp {
        (self.clock)()
    }
}

// ---------------------------------------------------------------------------
// Reading the state
// ---------------------------------------------------------------------------

impl CommentsPanel {
    /// Whether comments are shown: the panel, and the tints in the text
    /// (`EditorConfig::comments_visible`). The app persists this flag; see
    /// [`CommentsEvent::VisibilityChanged`].
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Whether the panel takes up space right now: comments are visible and
    /// there is something to show (at least one thread, or an open
    /// composer). A document without comments has no panel beside it.
    pub fn is_open(&self) -> bool {
        self.visible && (!self.threads.is_empty() || self.draft.is_some())
    }

    /// Whether resolved threads are listed.
    pub fn show_resolved(&self) -> bool {
        self.show_resolved
    }

    /// Every thread of the document, listed or not: anchored threads in
    /// document order, then detached ones (see
    /// [`ThreadEntry::is_detached`]).
    pub fn threads(&self) -> &[ThreadEntry] {
        &self.threads
    }

    /// The threads the panel lists, in display order: resolved threads are
    /// left out unless [`CommentsPanel::show_resolved`] is on. Detached
    /// threads come last, under their own heading.
    pub fn listed_threads(&self) -> impl Iterator<Item = &ThreadEntry> {
        let show_resolved = self.show_resolved;
        self.threads
            .iter()
            .filter(move |entry| show_resolved || !entry.is_resolved())
    }

    /// How many threads are resolved.
    pub fn resolved_count(&self) -> usize {
        self.threads
            .iter()
            .filter(|entry| entry.is_resolved())
            .count()
    }

    /// The thread with `id`.
    pub fn thread(&self, id: CommentId) -> Option<&ThreadEntry> {
        self.threads.iter().find(|entry| entry.id == id)
    }

    /// The active thread: the one marked in the panel and drawn with the
    /// stronger tint in the text.
    pub fn active_thread(&self) -> Option<CommentId> {
        self.active
    }

    /// The reply selected inside the active thread, if the user stepped
    /// into its replies.
    pub fn selected_reply(&self) -> Option<ReplyId> {
        self.selected_reply
    }

    /// What the composer is writing, while it is open.
    pub fn draft(&self) -> Option<&DraftKind> {
        self.draft.as_ref().map(|draft| &draft.kind)
    }

    /// Whether a composer is open.
    pub fn is_composing(&self) -> bool {
        self.draft.is_some()
    }

    /// The composer: a compact `Editor` that holds the text being written.
    /// It is empty while no draft is open.
    pub fn composer(&self) -> &Entity<Editor> {
        &self.composer
    }

    /// The delete confirmation that is showing, if any.
    pub fn pending_delete(&self) -> Option<DeleteTarget> {
        self.confirming.map(|pending| pending.target)
    }

    /// Whether keyboard focus is in the panel (the thread list or the
    /// composer).
    pub fn has_focus(&self, window: &Window, cx: &App) -> bool {
        self.focus_handle.is_focused(window) || self.composer.focus_handle(cx).is_focused(window)
    }

    /// Whether the thread list itself has keyboard focus.
    pub fn is_list_focused(&self, window: &Window) -> bool {
        self.focus_handle.is_focused(window)
    }

    fn is_listed(&self, id: CommentId) -> bool {
        self.listed_threads().any(|entry| entry.id == id)
    }

    fn listed_ids(&self) -> Vec<CommentId> {
        self.listed_threads().map(|entry| entry.id).collect()
    }

    fn active_entry(&self) -> Option<&ThreadEntry> {
        self.active.and_then(|id| self.thread(id))
    }

    fn read_only(&self, cx: &App) -> bool {
        self.editor.read(cx).is_read_only()
    }
}

// ---------------------------------------------------------------------------
// Visibility
// ---------------------------------------------------------------------------

impl CommentsPanel {
    /// Shows or hides comments: the panel and the tints in the text.
    /// Emits [`CommentsEvent::VisibilityChanged`] when the flag changes.
    /// Hiding discards an open composer, clears the active thread and gives
    /// focus back to the editor if the panel had it.
    pub fn set_visible(&mut self, visible: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.visible == visible {
            return;
        }
        let had_focus = self.has_focus(window, cx);
        self.visible = visible;
        self.editor
            .update(cx, |editor, cx| editor.set_comments_visible(visible, cx));
        if !visible {
            self.discard_draft(cx);
            self.confirming = None;
            self.set_active(None, false, cx);
            if had_focus {
                self.editor.read(cx).focus(window);
            }
        }
        cx.emit(CommentsEvent::VisibilityChanged(visible));
        cx.notify();
    }

    /// Flips [`CommentsPanel::is_visible`].
    pub fn toggle_visible(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_visible(!self.visible, window, cx);
    }

    /// Lists or hides resolved threads. Emits
    /// [`CommentsEvent::ShowResolvedChanged`] when the flag changes.
    pub fn set_show_resolved(&mut self, show: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.show_resolved == show {
            return;
        }
        self.show_resolved = show;
        self.validate(cx);
        self.repair_focus(window, cx);
        cx.emit(CommentsEvent::ShowResolvedChanged(show));
        cx.notify();
    }

    /// Flips [`CommentsPanel::show_resolved`].
    pub fn toggle_show_resolved(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_show_resolved(!self.show_resolved, window, cx);
    }
}

// ---------------------------------------------------------------------------
// Following the editor
// ---------------------------------------------------------------------------

impl CommentsPanel {
    fn on_editor_event(
        &mut self,
        _: &Entity<Editor>,
        event: &EditorEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            EditorEvent::Edited { change } => {
                if change.comments || change.blocks.is_some() {
                    self.refresh_threads(cx);
                    self.refresh_draft_highlight(cx);
                    self.repair_focus(window, cx);
                    // A thread may have appeared under the caret (undo of a
                    // delete, redo of an add) without the caret moving.
                    self.follow_caret(false, window, cx);
                }
            }
            EditorEvent::SelectionChanged => self.follow_caret(true, window, cx),
            EditorEvent::DocumentReplaced => {
                let had_focus = self.has_focus(window, cx);
                self.reset_for_new_document(cx);
                self.attach_editor(cx);
                if had_focus {
                    self.editor.read(cx).focus(window);
                }
            }
            _ => {}
        }
    }

    /// Drops everything that was derived from the current document.
    fn reset_for_new_document(&mut self, cx: &mut Context<Self>) {
        self.discard_draft(cx);
        self.confirming = None;
        self.set_active(None, false, cx);
        self.threads.clear();
        cx.notify();
    }

    /// Rebuilds the thread list from the document and drops whatever
    /// referred to a thread or reply that is gone.
    fn refresh_threads(&mut self, cx: &mut Context<Self>) {
        let threads = collect_threads(self.editor.read(cx).document());
        if threads != self.threads {
            self.threads = threads;
            cx.notify();
        }
        self.validate(cx);
    }

    fn validate(&mut self, cx: &mut Context<Self>) {
        if self.active.is_some_and(|id| !self.is_listed(id)) {
            self.set_active(None, false, cx);
        }
        if let Some(reply) = self.selected_reply
            && !self
                .active_entry()
                .is_some_and(|entry| entry.thread.reply(reply).is_some())
        {
            self.selected_reply = None;
            cx.notify();
        }
        if let Some(pending) = self.confirming {
            let exists = match pending.target {
                DeleteTarget::Thread(thread) => self.is_listed(thread),
                DeleteTarget::Reply(thread, reply) => self
                    .thread(thread)
                    .is_some_and(|entry| entry.thread.reply(reply).is_some()),
            };
            if !exists {
                self.confirming = None;
                cx.notify();
            }
        }
        let draft_target_gone = self.draft.as_ref().is_some_and(|draft| match draft.kind {
            DraftKind::New { .. } => false,
            DraftKind::Reply { thread } | DraftKind::EditComment { thread } => {
                !self.is_listed(thread)
            }
            DraftKind::EditReply { thread, reply } => !self
                .thread(thread)
                .is_some_and(|entry| self.is_listed(thread) && entry.thread.reply(reply).is_some()),
        });
        if draft_target_gone {
            self.discard_draft(cx);
        }
    }

    /// Marks the thread the caret is in. Only while focus is outside the
    /// panel; inside, the panel leads and the editor follows. With
    /// `may_clear` (the caret moved), a caret outside every listed thread
    /// leaves no thread active; without it, an active thread is kept.
    fn follow_caret(&mut self, may_clear: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.visible || self.has_focus(window, cx) {
            return;
        }
        if !may_clear && self.active.is_some() {
            return;
        }
        let at_caret = {
            let editor = self.editor.read(cx);
            editor.document().comments_at(editor.selection().head)
        };
        let next = match self.active {
            Some(active) if at_caret.contains(&active) => Some(active),
            _ => self
                .listed_threads()
                .find(|entry| at_caret.contains(&entry.id))
                .map(|entry| entry.id),
        };
        if next != self.active {
            self.set_active(next, false, cx);
        }
    }

    /// Makes `id` the active thread in the panel and in the editor. With
    /// `reveal`, the editor scrolls to the thread's anchor.
    fn set_active(&mut self, id: Option<CommentId>, reveal: bool, cx: &mut Context<Self>) {
        let changed = self.active != id;
        self.active = id;
        if changed {
            self.selected_reply = None;
            self.confirming = None;
        }
        let anchor = id
            .and_then(|id| self.thread(id))
            .and_then(ThreadEntry::anchor);
        self.editor.update(cx, |editor, cx| {
            editor.set_active_comment(id, cx);
            if reveal && let Some(anchor) = anchor {
                editor.scroll_to_position(anchor.start, Autoscroll::Center, cx);
            }
        });
        self.scroll_list_to_active();
        if changed {
            cx.emit(CommentsEvent::ActiveThreadChanged(id));
            cx.notify();
        }
    }

    /// Makes a thread active (`None` clears it) and scrolls the editor to
    /// its anchor. Ignored for a thread the panel does not list. This is
    /// what clicking a thread does; it does not move focus or the caret.
    pub fn select_thread(&mut self, id: Option<CommentId>, cx: &mut Context<Self>) {
        if id.is_none_or(|id| self.is_listed(id)) {
            self.set_active(id, true, cx);
        }
    }

    /// The index of the active thread among the children of the list
    /// element (the "Detached" heading is a child too).
    fn scroll_list_to_active(&self) {
        let Some(active) = self.active else {
            return;
        };
        let mut child = 0;
        let mut seen_detached = false;
        for entry in self.listed_threads() {
            if entry.is_detached() && !seen_detached {
                seen_detached = true;
                child += 1;
            }
            if entry.id == active {
                self.scroll.scroll_to_item(child);
                return;
            }
            child += 1;
        }
    }

    /// Runs a comment command on the document through the editor's mutation
    /// seam (one undo step), without scrolling the text, then reads the
    /// threads again. `None` when the editor is read-only.
    fn mutate<R>(
        &mut self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut EditorState) -> R,
    ) -> Option<R> {
        if self.read_only(cx) {
            return None;
        }
        let result = self
            .editor
            .update(cx, |editor, cx| editor.edit_with(Autoscroll::None, f, cx));
        self.refresh_threads(cx);
        Some(result)
    }

    /// Keeps keyboard focus on something that is on screen: when the
    /// composer or the list had focus and is gone, focus moves to the list
    /// or back to the editor.
    fn repair_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let composer_focused = self.composer.focus_handle(cx).is_focused(window);
        let list_focused = self.focus_handle.is_focused(window);
        if composer_focused && self.draft.is_none() {
            if self.is_open() {
                window.focus(&self.focus_handle);
            } else {
                self.editor.read(cx).focus(window);
            }
        } else if list_focused && !self.is_open() {
            self.editor.read(cx).focus(window);
        }
    }

    fn notice(&self, message: &'static str, cx: &mut Context<Self>) {
        // Shown through the app's toast layer when it installed one.
        show_toast(Toast::new(message), cx);
    }
}

// ---------------------------------------------------------------------------
// Focus
// ---------------------------------------------------------------------------

impl CommentsPanel {
    /// Moves keyboard focus into the panel, showing comments first if they
    /// were hidden. With an open composer, focus goes to it. Otherwise the
    /// thread list gets focus and, when no thread is active, the thread at
    /// the caret (else the next one after it, else the first) becomes
    /// active. Does nothing in a document without comments.
    pub fn focus_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_visible(true, window, cx);
        if self.draft.is_some() {
            window.focus(&self.composer.focus_handle(cx));
            return;
        }
        if !self.is_open() {
            self.notice("This document has no comments.", cx);
            return;
        }
        if self.active.is_none() {
            let head = self.editor.read(cx).selection().head;
            let pick = self
                .listed_threads()
                .find(|entry| entry.contains(head))
                .or_else(|| {
                    self.listed_threads()
                        .find(|entry| entry.anchor().is_some_and(|anchor| anchor.start > head))
                })
                .or_else(|| self.listed_threads().next())
                .map(|entry| entry.id);
            if pick.is_some() {
                self.set_active(pick, true, cx);
            }
        }
        window.focus(&self.focus_handle);
        cx.notify();
    }

    /// Gives keyboard focus back to the editor. The active thread stays
    /// marked until the caret moves somewhere else.
    pub fn focus_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.confirming.take().is_some() {
            cx.notify();
        }
        self.editor.read(cx).focus(window);
    }
}

// ---------------------------------------------------------------------------
// Navigation
// ---------------------------------------------------------------------------

impl CommentsPanel {
    /// Makes the next listed thread active (no wrap) and scrolls the editor
    /// to its anchor.
    pub fn select_next_thread(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.step_thread(1, cx);
    }

    /// Makes the previous listed thread active.
    pub fn select_previous_thread(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.step_thread(-1, cx);
    }

    /// Makes the first listed thread active.
    pub fn select_first_thread(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(first) = self.listed_ids().first() {
            self.set_active(Some(*first), true, cx);
        }
    }

    /// Makes the last listed thread active.
    pub fn select_last_thread(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(last) = self.listed_ids().last() {
            self.set_active(Some(*last), true, cx);
        }
    }

    fn step_thread(&mut self, delta: isize, cx: &mut Context<Self>) {
        let ids = self.listed_ids();
        if ids.is_empty() {
            return;
        }
        let current = self
            .active
            .and_then(|active| ids.iter().position(|id| *id == active));
        let ix = match current {
            Some(ix) => (ix as isize + delta).clamp(0, ids.len() as isize - 1) as usize,
            None if delta > 0 => 0,
            None => ids.len() - 1,
        };
        self.set_active(Some(ids[ix]), true, cx);
    }

    /// Steps into the replies of the active thread: selects its first
    /// reply, then the next one. [`CommentsPanel::edit`] and
    /// [`CommentsPanel::request_delete`] act on the selected reply.
    pub fn select_next_reply(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.active_entry() else {
            return;
        };
        let replies = &entry.thread.replies;
        let current = self
            .selected_reply
            .and_then(|selected| replies.iter().position(|reply| reply.id == selected));
        let next = match current {
            Some(ix) => replies.get(ix + 1).or(replies.last()),
            None => replies.first(),
        }
        .map(|reply| reply.id);
        if next != self.selected_reply {
            self.selected_reply = next;
            cx.notify();
        }
    }

    /// Selects the previous reply of the active thread; from the first
    /// reply, back to the opening comment.
    pub fn select_previous_reply(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.active_entry() else {
            return;
        };
        let replies = &entry.thread.replies;
        let current = self
            .selected_reply
            .and_then(|selected| replies.iter().position(|reply| reply.id == selected));
        let previous = match current {
            Some(ix) if ix > 0 => Some(replies[ix - 1].id),
            _ => None,
        };
        if previous != self.selected_reply {
            self.selected_reply = previous;
            cx.notify();
        }
    }

    /// Moves the caret to the start of the next comment in the text
    /// (wrapping to the first) and makes its thread active in the panel and
    /// in the editor. Focus stays where it is. Resolved threads are skipped
    /// unless they are listed; hidden comments are shown first.
    pub fn next_comment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.step_comment(true, window, cx);
    }

    /// Like [`CommentsPanel::next_comment`], backwards.
    pub fn previous_comment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.step_comment(false, window, cx);
    }

    fn step_comment(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.set_visible(true, window, cx);
        let stops: Vec<(CommentId, TextRange)> = self
            .listed_threads()
            .filter_map(|entry| entry.anchor().map(|anchor| (entry.id, anchor)))
            .collect();
        if stops.is_empty() {
            self.notice("This document has no comments to go to.", cx);
            return;
        }
        let head = self.editor.read(cx).selection().head;
        // Standing in the active thread: step from it, so threads that
        // share a start position are all reachable.
        let current = self.active.and_then(|active| {
            stops.iter().position(|(id, anchor)| {
                *id == active && anchor.start <= head && head <= anchor.end
            })
        });
        let count = stops.len();
        let ix = match (current, forward) {
            (Some(ix), true) => (ix + 1) % count,
            (Some(ix), false) => (ix + count - 1) % count,
            (None, true) => stops
                .iter()
                .position(|(_, anchor)| anchor.start > head)
                .unwrap_or(0),
            (None, false) => stops
                .iter()
                .rposition(|(_, anchor)| anchor.start < head)
                .unwrap_or(count - 1),
        };
        let (id, anchor) = stops[ix];
        self.editor.update(cx, |editor, cx| {
            editor.edit_with(Autoscroll::None, |state| state.set_caret(anchor.start), cx);
            editor.scroll_to_position(anchor.start, Autoscroll::Center, cx);
        });
        self.set_active(Some(id), false, cx);
    }
}

// ---------------------------------------------------------------------------
// The composer: new comments, replies, edits
// ---------------------------------------------------------------------------

impl CommentsPanel {
    /// Starts a comment: opens the composer for the selected text, or for
    /// the word at a collapsed caret, showing the panel if it was hidden.
    /// The selection in the editor is left as it is; the target text is
    /// marked with the [`DRAFT_HIGHLIGHT_LAYER`] highlight until the
    /// composer closes. With a new-comment composer already open, the
    /// composer is pointed at the current selection and keeps its text.
    ///
    /// Does nothing in a read-only editor, or when there is no text to
    /// anchor to (a caret in white space, an empty selection).
    pub fn add_comment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only(cx) {
            return;
        }
        let target = {
            let editor = self.editor.read(cx);
            let document = editor.document();
            let selection = editor.selection();
            let range = if selection.is_collapsed() {
                word_range_at(document, selection.head)
            } else {
                Some(selection.range())
            };
            range
                .filter(|range| !document.text_in(*range).trim().is_empty())
                .map(|range| (range, quote(document, &[range])))
        };
        let retargeting = matches!(self.draft(), Some(DraftKind::New { .. }));
        let Some((range, quote)) = target else {
            if retargeting {
                window.focus(&self.composer.focus_handle(cx));
            } else {
                self.notice("Select the text to comment on first.", cx);
            }
            return;
        };
        if retargeting {
            if let Some(draft) = &mut self.draft {
                draft.kind = DraftKind::New { range };
                draft.quote = quote;
            }
            self.refresh_draft_highlight(cx);
            window.focus(&self.composer.focus_handle(cx));
            cx.notify();
            return;
        }
        self.open_draft(DraftKind::New { range }, quote, "", window, cx);
    }

    /// Opens the composer for a reply to the active thread.
    pub fn reply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(thread) = self.active else {
            return;
        };
        if self.draft() == Some(&DraftKind::Reply { thread }) {
            window.focus(&self.composer.focus_handle(cx));
            return;
        }
        self.open_draft(DraftKind::Reply { thread }, String::new(), "", window, cx);
    }

    /// Opens the composer with the body of the active comment, or of the
    /// selected reply, to change it.
    pub fn edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.active_entry() else {
            return;
        };
        let thread = entry.id;
        let (kind, body) = match self
            .selected_reply
            .and_then(|reply| entry.thread.reply(reply))
        {
            Some(reply) => (
                DraftKind::EditReply {
                    thread,
                    reply: reply.id,
                },
                reply.body.clone(),
            ),
            None => (DraftKind::EditComment { thread }, entry.thread.body.clone()),
        };
        if self.draft() == Some(&kind) {
            window.focus(&self.composer.focus_handle(cx));
            return;
        }
        self.open_draft(kind, String::new(), &body, window, cx);
    }

    fn open_draft(
        &mut self,
        kind: DraftKind,
        quote: String,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.read_only(cx) {
            return;
        }
        // Where to go back to: the list when the draft starts there (or
        // replaces a draft that started there), the editor otherwise.
        let return_to_list = self.focus_handle.is_focused(window)
            || self.draft.as_ref().is_some_and(|draft| {
                draft.return_to_list && self.composer.focus_handle(cx).is_focused(window)
            });
        let was_visible = self.visible;
        self.confirming = None;
        self.set_visible(true, window, cx);
        let placeholder = kind.placeholder();
        self.composer.update(cx, |composer, cx| {
            composer.set_plain_text(text, cx);
            composer.update_config(|config| config.placeholder = placeholder.into(), cx);
        });
        self.draft = Some(Draft {
            kind,
            quote,
            return_to_list,
            was_visible,
        });
        self.refresh_draft_highlight(cx);
        window.focus(&self.composer.focus_handle(cx));
        cx.notify();
    }

    /// Marks the target of a new-comment draft in the text, or removes the
    /// mark when there is no such draft.
    fn refresh_draft_highlight(&mut self, cx: &mut Context<Self>) {
        let range = match self.draft() {
            Some(DraftKind::New { range }) => Some(*range),
            _ => None,
        };
        let color = cx.theme().colors.comment_active;
        self.editor.update(cx, |editor, cx| match range {
            Some(range) => {
                let document = editor.document();
                let range = TextRange::new(
                    document.clamp_position(range.start),
                    document.clamp_position(range.end),
                );
                editor.set_highlights(
                    DRAFT_HIGHLIGHT_LAYER,
                    vec![range],
                    HighlightStyle::background(color),
                    cx,
                );
            }
            None => editor.clear_highlights(DRAFT_HIGHLIGHT_LAYER, cx),
        });
    }

    /// Saves what the composer holds: creates the thread, adds the reply or
    /// changes the body, as one undo step of the document. An empty text
    /// saves nothing and leaves the composer open. Afterwards focus returns
    /// to where the draft was started (the editor for a new comment, the
    /// thread list for a reply or an edit made there), and a new thread is
    /// the active one.
    pub fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = &self.draft else {
            return;
        };
        let kind = draft.kind.clone();
        let body = self.composer.read(cx).plain_text();
        let body = body.trim();
        if body.is_empty() {
            return;
        }
        let author = self.author.clone();
        let now = self.now();
        match kind {
            DraftKind::New { range } => {
                let created = self.mutate(cx, |state| {
                    state.grouped(|state| {
                        // The thread is anchored to the selection; put the
                        // user's own selection back afterwards.
                        let selection: Selection = state.selection();
                        state.select_range(range);
                        let id = state.add_comment(body, &author, now);
                        state.set_selection(selection);
                        id
                    })
                });
                if let Some(Some(id)) = created {
                    self.set_active(Some(id), false, cx);
                }
            }
            DraftKind::Reply { thread } => {
                self.mutate(cx, |state| state.add_reply(thread, body, &author, now));
            }
            DraftKind::EditComment { thread } => {
                self.mutate(cx, |state| state.edit_comment(thread, body));
            }
            DraftKind::EditReply { thread, reply } => {
                self.mutate(cx, |state| state.edit_reply(thread, reply, body));
            }
        }
        self.close_draft(window, cx);
    }

    /// Discards what the composer holds and returns focus to where the
    /// draft was started. If starting the draft is what made comments
    /// visible, they are hidden again.
    pub fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let hide_again = self
            .draft
            .as_ref()
            .is_some_and(|draft| !draft.was_visible && matches!(draft.kind, DraftKind::New { .. }));
        self.close_draft(window, cx);
        if hide_again {
            self.set_visible(false, window, cx);
        }
    }

    fn close_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = self.draft.take() else {
            return;
        };
        self.composer.update(cx, |composer, cx| composer.clear(cx));
        self.refresh_draft_highlight(cx);
        if draft.return_to_list && self.is_open() {
            window.focus(&self.focus_handle);
        } else {
            self.editor.read(cx).focus(window);
        }
        cx.notify();
    }

    /// Drops the draft without touching focus (the caller repairs it).
    fn discard_draft(&mut self, cx: &mut Context<Self>) {
        if self.draft.take().is_some() {
            self.composer.update(cx, |composer, cx| composer.clear(cx));
            self.refresh_draft_highlight(cx);
            cx.notify();
        }
    }
}

// ---------------------------------------------------------------------------
// Resolve, delete, undo
// ---------------------------------------------------------------------------

impl CommentsPanel {
    /// Resolves the active thread, or reopens it when it is resolved. A
    /// resolved thread leaves the list unless resolved threads are shown;
    /// with focus in the list, its neighbour becomes active.
    pub fn toggle_resolved(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.active_entry() else {
            return;
        };
        let id = entry.id;
        let resolved = entry.is_resolved();
        let position = self.listed_ids().iter().position(|listed| *listed == id);
        self.mutate(cx, |state| {
            if resolved {
                state.reopen_comment(id)
            } else {
                state.resolve_comment(id)
            }
        });
        self.activate_neighbour(position, window, cx);
        self.repair_focus(window, cx);
    }

    /// After the active thread left the list: with focus in the list, the
    /// thread that took its place (or the last one) becomes active.
    fn activate_neighbour(
        &mut self,
        position: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active.is_some() || !self.focus_handle.is_focused(window) {
            return;
        }
        let ids = self.listed_ids();
        let neighbour = position
            .and_then(|position| ids.get(position))
            .or(ids.last())
            .copied();
        if neighbour.is_some() {
            self.set_active(neighbour, true, cx);
        }
    }

    /// Asks before deleting the active thread, or the selected reply: the
    /// thread shows a confirmation, the list takes focus, `enter` deletes
    /// and `escape` keeps. See [`CommentsPanel::confirm_delete`].
    pub fn request_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only(cx) {
            return;
        }
        let Some(thread) = self.active else {
            return;
        };
        let target = match self.selected_reply {
            Some(reply) => DeleteTarget::Reply(thread, reply),
            None => DeleteTarget::Thread(thread),
        };
        let return_to_editor = match self.confirming {
            Some(pending) => pending.return_to_editor,
            None => !self.has_focus(window, cx),
        };
        self.discard_draft(cx);
        self.confirming = Some(PendingDelete {
            target,
            return_to_editor,
        });
        window.focus(&self.focus_handle);
        self.scroll_list_to_active();
        cx.notify();
    }

    /// Deletes what [`CommentsPanel::request_delete`] asked about: the
    /// thread with its replies and its anchor in the text, or the one
    /// reply. One undo step.
    pub fn confirm_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pending) = self.confirming.take() else {
            return;
        };
        match pending.target {
            DeleteTarget::Thread(thread) => {
                let position = self.listed_ids().iter().position(|id| *id == thread);
                self.mutate(cx, |state| state.delete_comment(thread));
                if !pending.return_to_editor {
                    self.activate_neighbour(position, window, cx);
                }
            }
            DeleteTarget::Reply(thread, reply) => {
                self.mutate(cx, |state| state.delete_reply(thread, reply));
            }
        }
        if pending.return_to_editor {
            self.editor.read(cx).focus(window);
        } else {
            self.repair_focus(window, cx);
        }
        cx.notify();
    }

    /// Dismisses the delete confirmation and keeps the comment.
    pub fn cancel_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pending) = self.confirming.take() else {
            return;
        };
        if pending.return_to_editor {
            self.editor.read(cx).focus(window);
        }
        cx.notify();
    }

    /// Undoes the last change of the document (comment changes are part of
    /// its history). Bound to `cmd-z` in the thread list, so a resolve or a
    /// delete can be taken back without leaving the panel.
    pub fn undo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.mutate(cx, |state| state.undo());
        self.repair_focus(window, cx);
    }

    /// Redoes the last undone change of the document.
    pub fn redo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.mutate(cx, |state| state.redo());
        self.repair_focus(window, cx);
    }
}
