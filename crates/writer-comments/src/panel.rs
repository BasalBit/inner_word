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
    AnyWindowHandle, App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable,
    Global, ScrollHandle, SharedString, Subscription, Task, WeakEntity, Window, px,
};
use writer_core::{CommentId, Document, EditorState, ReplyId, TextRange, Timestamp};
use writer_editor::{
    Autoscroll, CompactOptions, Editor, EditorConfig, EditorEvent, InterceptPriority,
    KeyDisposition,
};
use writer_ui::{Toast, toast::show_toast};

use crate::{
    threads::{ThreadEntry, collect_threads},
    time,
};

mod compose;
mod navigation;
mod resolve;

/// The editor highlight layer that marks the text a new comment will be
/// anchored to while its composer is open.
pub const DRAFT_HIGHLIGHT_LAYER: &str = "comment-draft";

/// How often the relative times ("5 min ago") are refreshed.
const TIME_REFRESH: Duration = Duration::from_secs(30);

/// What a [`CommentsPanel`] tells its subscribers.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum CommentsEvent {
    /// Comments were shown or hidden (the panel and the tints in the text),
    /// by the panel or by a change of `EditorConfig::comments_visible`. The
    /// app saves this flag.
    VisibilityChanged(bool),
    /// [`CommentsPanel::is_open`] changed: the panel now has something to
    /// show, or nothing any more. The app shows and hides the panel's
    /// container on it.
    OpenChanged(bool),
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
    /// [`CommentsPanel::is_open`] as last reported with
    /// [`CommentsEvent::OpenChanged`].
    reported_open: bool,
    pub(crate) show_resolved: bool,
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
    _editor_subscriptions: [Subscription; 2],
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
    /// signed with `author`. Whether comments start visible is read from
    /// the editor (`EditorConfig::comments_visible`).
    ///
    /// The panel registers itself as the comments panel of `window`: the
    /// comment actions that work from anywhere in a window reach the panel
    /// that was created last for it.
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
        let mut subscriptions = vec![
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

        subscriptions.push(cx.observe_self(Self::report_open));
        register_panel(window.window_handle(), cx.weak_entity(), cx);

        let mut this = Self {
            _editor_subscriptions: Self::subscribe_to_editor(&editor, window, cx),
            visible: editor.read(cx).config().comments_visible,
            reported_open: false,
            editor,
            author: author.into(),
            show_resolved: false,
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
    ) -> [Subscription; 2] {
        [
            cx.subscribe_in(editor, window, Self::on_editor_event),
            // `EditorConfig::comments_visible` is the flag the app's own
            // settings code may flip; the panel goes along with it.
            cx.observe_in(editor, window, |this, editor, window, cx| {
                let visible = editor.read(cx).config().comments_visible;
                this.set_visible(visible, window, cx);
            }),
        ]
    }

    /// Emits [`CommentsEvent::OpenChanged`] when [`CommentsPanel::is_open`]
    /// differs from what was last reported. Runs after every notify.
    fn report_open(&mut self, cx: &mut Context<Self>) {
        let open = self.is_open();
        if open != self.reported_open {
            self.reported_open = open;
            cx.emit(CommentsEvent::OpenChanged(open));
        }
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
        self._editor_subscriptions = Self::subscribe_to_editor(&self.editor, window, cx);
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

    /// Whether the panel has something to show: comments are visible and
    /// the document has at least one thread, or a composer is open. The app
    /// shows the panel's container only while this is true, so that a
    /// document without comments has no panel beside it; see
    /// [`CommentsEvent::OpenChanged`].
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

    /// A thread the panel does not list has no tint in the text either: a
    /// resolved thread is tinted only while "show resolved" is on. (The
    /// editor forgets the set when its document is replaced, and does not
    /// follow edits, so this runs with every refresh.)
    fn sync_hidden_tints(&mut self, cx: &mut Context<Self>) {
        let mut hidden: Vec<CommentId> = if self.show_resolved {
            Vec::new()
        } else {
            self.threads
                .iter()
                .filter(|entry| entry.is_resolved())
                .map(|entry| entry.id)
                .collect()
        };
        hidden.sort_unstable();
        let mut current: Vec<CommentId> = self.editor.read(cx).hidden_comments().collect();
        current.sort_unstable();
        if hidden != current {
            self.editor
                .update(cx, |editor, cx| editor.set_hidden_comments(hidden, cx));
        }
    }

    fn validate(&mut self, cx: &mut Context<Self>) {
        self.sync_hidden_tints(cx);
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
// The panel of a window
// ---------------------------------------------------------------------------

/// Which panel serves which window, newest last. The actions that work
/// from anywhere in a window are handled by global listeners (see
/// [`crate::init`]); they look the panel up here.
#[derive(Default)]
struct WindowPanels(Vec<(AnyWindowHandle, WeakEntity<CommentsPanel>)>);

impl Global for WindowPanels {}

fn register_panel(window: AnyWindowHandle, panel: WeakEntity<CommentsPanel>, cx: &mut App) {
    let panels = &mut cx.default_global::<WindowPanels>().0;
    panels.retain(|(_, panel)| panel.upgrade().is_some());
    panels.push((window, panel));
}

/// The panel that window-wide comment actions go to: the one of the active
/// window, or, when no window is active (the app is in the background),
/// the only one there is.
pub(crate) fn panel_for_actions(cx: &App) -> Option<(AnyWindowHandle, Entity<CommentsPanel>)> {
    let panels = cx.try_global::<WindowPanels>()?;
    let live = || {
        panels
            .0
            .iter()
            .rev()
            .filter_map(|(window, panel)| Some((*window, panel.upgrade()?)))
    };
    if let Some(active) = cx.active_window()
        && let Some(found) = live().find(|(window, _)| *window == active)
    {
        return Some(found);
    }
    let (window, panel) = live().next()?;
    live()
        .all(|(other, _)| other == window)
        .then_some((window, panel))
}
