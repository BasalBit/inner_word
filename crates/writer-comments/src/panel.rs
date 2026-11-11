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
    Global, ListAlignment, ListOffset, ListState, SharedString, Subscription, Task, WeakEntity,
    Window, px,
};
use writer_core::{
    BlockSplice, Change, CommentId, Document, EditorState, Position, ReplyId, TextRange, Timestamp,
};
use writer_editor::{
    Autoscroll, CompactOptions, Editor, EditorConfig, EditorEvent, InterceptPriority,
    KeyDisposition,
};
use writer_ui::{Toast, toast::show_toast};

use crate::{
    threads::{ThreadEntry, collect_threads, quote},
    time,
    track::RangeTracker,
};

mod compose;
mod navigation;
mod resolve;

/// The editor highlight layer that marks the text a new comment will be
/// anchored to while its composer is open.
pub const DRAFT_HIGHLIGHT_LAYER: &str = "comment-draft";

/// How often the relative times ("5 min ago") are refreshed.
const TIME_REFRESH: Duration = Duration::from_secs(30);

/// How many unfinished comments are kept for documents that are not open.
const KEPT_DRAFTS_MAX: usize = 16;

/// How far beyond the visible part of the thread list cards are measured.
const LIST_OVERDRAW: gpui::Pixels = px(240.);

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
    /// Follows the target of a new comment through edits of the document.
    tracker: Option<RangeTracker>,
    /// What the draft is about, as text: the target text of a new comment,
    /// or author, time and body of the comment replied to or edited. A
    /// draft that outlives its document is only reopened in a document
    /// where this still fits.
    fingerprint: String,
    /// The text the composer started with (the body being edited).
    original: String,
    /// Where focus goes when the composer closes: the thread list when the
    /// draft was started from it, the editor otherwise.
    return_to_list: bool,
    /// Whether comments were visible before the draft made them visible.
    was_visible: bool,
}

/// An unfinished comment that was set aside instead of thrown away: the
/// document was switched or comments were hidden while it was being
/// written, or another composer was opened over it. It reopens when its
/// place is back.
struct KeptDraft {
    kind: DraftKind,
    quote: String,
    /// What was written so far.
    text: String,
    original: String,
    fingerprint: String,
    /// `Some` while the draft belongs to the document that is open and its
    /// target is still followed through edits.
    tracker: Option<RangeTracker>,
    /// Whether the draft belongs to the document that is open now. Once
    /// that document was replaced, the fingerprint decides where the draft
    /// fits.
    live: bool,
}

/// One row of the thread list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Row {
    /// The "Detached" heading above the threads without an anchor.
    DetachedHeading,
    /// A thread card.
    Thread(CommentId),
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
    /// Unfinished comments that were set aside, oldest first.
    kept: Vec<KeptDraft>,
    /// Block changes reported by the editor that the draft targets were
    /// not moved through yet.
    pending_splice: Option<BlockSplice>,
    confirming: Option<PendingDelete>,
    /// The one composer, a compact editor, shown wherever the draft is.
    pub(crate) composer: Entity<Editor>,
    /// Focus of the thread list.
    pub(crate) focus_handle: FocusHandle,
    /// The rows of the thread list, in display order. Only the rows near
    /// the visible part of the list are built and laid out.
    pub(crate) rows: Vec<Row>,
    pub(crate) list: ListState,
    /// The row that still has to be brought fully into view, and how often
    /// that was tried. A card is measured when it is drawn, so the first
    /// scroll can fall short.
    reveal: Option<(Row, u8)>,
    /// How many rows of the list were built so far (see
    /// [`CommentsPanel::rows_built`]).
    pub(crate) rows_built: u64,
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
        if self.draft.is_some() && self.visible {
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
            // which the crates installed their keys. (`tab` needs no
            // interceptor: a compact editor does not indent, and the
            // composer binds the key at the editor's level, see
            // `actions::key_bindings`.)
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
        let focus_handle = cx.focus_handle();
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
            kept: Vec::new(),
            pending_splice: None,
            confirming: None,
            composer,
            focus_handle,
            rows: Vec::new(),
            list: ListState::new(0, ListAlignment::Top, LIST_OVERDRAW),
            reveal: None,
            rows_built: 0,
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
        self.restore_kept_draft(cx);
    }

    /// The editor whose document the panel shows.
    pub fn editor(&self) -> &Entity<Editor> {
        &self.editor
    }

    /// Switches the panel to another editor (the app opened another
    /// document in a new editor entity). An open composer with text in it
    /// is set aside (see [`CommentsPanel::kept_drafts`]), no
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

    /// How many unfinished comments are set aside. A composer with text in
    /// it is never thrown away by something other than its own cancel key:
    /// when the document is switched, comments are hidden, or another
    /// composer opens over it, the text is kept and the composer reopens
    /// with it when its place is back (the document is opened again,
    /// comments are shown, the other composer closes). Kept drafts live as
    /// long as the panel; they are not stored with the document.
    pub fn kept_drafts(&self) -> usize {
        self.kept.len()
    }

    /// Whether the text a new comment was started on has been deleted
    /// while its composer is open. The comment cannot be saved until the
    /// text is back (undo) or the composer is pointed at other text
    /// ([`CommentsPanel::add_comment`]).
    pub fn draft_target_lost(&self) -> bool {
        matches!(self.draft(), Some(DraftKind::New { range }) if range.is_empty())
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
    /// Hiding closes an open composer (what was written in it comes back
    /// when comments are shown again), clears the active thread and gives
    /// focus back to the editor if the panel had it.
    pub fn set_visible(&mut self, visible: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.visible == visible {
            return;
        }
        let had_focus = self.has_focus(window, cx);
        self.visible = visible;
        self.editor
            .update(cx, |editor, cx| editor.set_comments_visible(visible, cx));
        if visible {
            self.restore_kept_draft(cx);
        } else {
            self.keep_draft(true, cx);
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
        self.rebuild_rows();
        self.validate(cx);
        self.restore_kept_draft(cx);
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
                    self.track_edit(Some(*change), cx);
                    self.repair_focus(window, cx);
                    // A thread may have appeared under the caret (undo of a
                    // delete, redo of an add) without the caret moving.
                    self.follow_caret(false, window, cx);
                }
            }
            EditorEvent::SelectionChanged => self.follow_caret(true, window, cx),
            // A settings change moves the revision on without an `Edited`.
            EditorEvent::SettingsChanged => self.track_edit(None, cx),
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

    /// Drops everything that was derived from the current document. An
    /// unfinished comment is set aside, not applied to the next document
    /// and not thrown away: it reopens when its document is back.
    fn reset_for_new_document(&mut self, cx: &mut Context<Self>) {
        // Whatever was waiting for this document to show comments again
        // now waits for the document itself.
        self.pending_splice = None;
        for kept in &mut self.kept {
            kept.live = false;
            kept.tracker = None;
        }
        if self.keep_draft(false, cx) {
            self.notice(
                "Your unfinished comment was kept. It reopens when you return to its document.",
                cx,
            );
        }
        self.confirming = None;
        self.set_active(None, false, cx);
        self.threads.clear();
        self.rebuild_rows();
        cx.notify();
    }

    /// Rebuilds the thread list from the document and drops whatever
    /// referred to a thread or reply that is gone.
    fn refresh_threads(&mut self, cx: &mut Context<Self>) {
        let threads = collect_threads(self.editor.read(cx).document());
        if threads != self.threads {
            self.threads = threads;
            self.rebuild_rows();
            cx.notify();
        }
        self.validate(cx);
    }

    /// Rebuilds the rows of the list from the threads. The list keeps its
    /// scroll position and the measured heights of the rows that stayed.
    fn rebuild_rows(&mut self) {
        let mut rows = Vec::with_capacity(self.threads.len() + 1);
        let mut heading = false;
        for entry in self.listed_threads() {
            if entry.is_detached() && !heading {
                heading = true;
                rows.push(Row::DetachedHeading);
            }
            rows.push(Row::Thread(entry.id));
        }
        let prefix = rows
            .iter()
            .zip(&self.rows)
            .take_while(|(new, old)| new == old)
            .count();
        let suffix = rows[prefix..]
            .iter()
            .rev()
            .zip(self.rows[prefix..].iter().rev())
            .take_while(|(new, old)| new == old)
            .count();
        let (old_len, new_len) = (self.rows.len(), rows.len());
        if old_len != new_len || prefix + suffix < new_len {
            self.list
                .splice(prefix..old_len - suffix, new_len - prefix - suffix);
        }
        self.rows = rows;
    }

    /// Moves the targets of unfinished new comments through a change of
    /// the document, so that a draft still points at its text after the
    /// text around it was edited. `change` is what the editor reported;
    /// `None` applies what is still pending.
    ///
    /// Several changes can be reported in a row while the document is
    /// already past all of them; the targets are moved once, when the
    /// report that matches the document's revision arrives.
    pub(super) fn track_edit(&mut self, change: Option<Change>, cx: &mut Context<Self>) {
        if let Some(splice) = change.and_then(|change| change.blocks) {
            self.pending_splice = Some(match self.pending_splice {
                Some(earlier) => BlockSplice::then(earlier, splice),
                None => splice,
            });
        }
        let editor = self.editor.read(cx);
        if change.is_some_and(|change| change.revision < editor.state().revision()) {
            return;
        }
        let splice = self.pending_splice.take();
        let document = editor.document();
        let follow = |kind: &mut DraftKind,
                      text: &mut String,
                      fingerprint: &mut String,
                      tracker: &mut Option<RangeTracker>| {
            if let Some(tracker) = tracker {
                tracker.update(document, splice);
                *kind = DraftKind::New {
                    range: tracker.range(),
                };
                *text = quote(document, &[tracker.range()]);
                *fingerprint = tracker.text().to_string();
            } else if let Some(current) = fingerprint_of(kind, document) {
                *fingerprint = current;
            }
        };
        if let Some(draft) = &mut self.draft {
            follow(
                &mut draft.kind,
                &mut draft.quote,
                &mut draft.fingerprint,
                &mut draft.tracker,
            );
        }
        for kept in self.kept.iter_mut().filter(|kept| kept.live) {
            follow(
                &mut kept.kind,
                &mut kept.quote,
                &mut kept.fingerprint,
                &mut kept.tracker,
            );
        }
        self.refresh_draft_highlight(cx);
        cx.notify();
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
        // A reply or an edit that was set aside goes with its thread.
        let threads = &self.threads;
        self.kept.retain(|kept| {
            !kept.live
                || kept
                    .kind
                    .thread()
                    .is_none_or(|thread| threads.iter().any(|entry| entry.id == thread))
        });
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
        self.scroll_list_to_active(cx);
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

    /// Scrolls the thread list so that the active thread is in view.
    pub(super) fn scroll_list_to_active(&mut self, cx: &mut Context<Self>) {
        let Some(active) = self.active else {
            return;
        };
        let Some(ix) = self.row_index(Row::Thread(active)) else {
            return;
        };
        let top = self.list.logical_scroll_top();
        if ix <= top.item_ix {
            self.list.scroll_to(ListOffset {
                item_ix: ix,
                offset_in_item: px(0.),
            });
        } else if ix - top.item_ix <= 32 && self.list.bounds_for_item(ix).is_some() {
            // Near, and measured: the least scrolling that shows it.
            self.list.scroll_to_reveal_item(ix);
        } else {
            // Far away: the rows in between were never measured.
            self.list.scroll_to(ListOffset {
                item_ix: ix,
                offset_in_item: px(0.),
            });
        }
        // The card may grow once it is active (its key hints, a composer),
        // and a card that was never drawn has no height yet: look again
        // after the next frame.
        self.reveal = Some((Row::Thread(active), 0));
        cx.notify();
    }

    fn row_index(&self, row: Row) -> Option<usize> {
        self.rows.iter().position(|candidate| *candidate == row)
    }

    /// Runs after a frame while a row waits to be brought into view (see
    /// `reveal`): scrolls the rest of the way if the frame showed that the
    /// row is still cut off.
    pub(crate) fn settle_reveal(&mut self, cx: &mut Context<Self>) {
        let Some((row, attempts)) = self.reveal.take() else {
            return;
        };
        let Some(ix) = self.row_index(row) else {
            return;
        };
        let viewport = self.list.viewport_bounds();
        let Some(bounds) = self.list.bounds_for_item(ix) else {
            // Above the first drawn row, or not measured: put it at the top.
            self.list.scroll_to(ListOffset {
                item_ix: ix,
                offset_in_item: px(0.),
            });
            self.retry_reveal(row, attempts, cx);
            return;
        };
        let slack = px(0.5);
        if bounds.top() < viewport.top() - slack || bounds.size.height > viewport.size.height {
            self.list.scroll_to(ListOffset {
                item_ix: ix,
                offset_in_item: px(0.),
            });
            if bounds.top() < viewport.top() - slack {
                self.retry_reveal(row, attempts, cx);
            }
        } else if bounds.bottom() > viewport.bottom() + slack {
            self.list.scroll_by(bounds.bottom() - viewport.bottom());
            self.retry_reveal(row, attempts, cx);
        }
    }

    fn retry_reveal(&mut self, row: Row, attempts: u8, cx: &mut Context<Self>) {
        if attempts < 3 {
            self.reveal = Some((row, attempts + 1));
        }
        cx.notify();
    }

    /// How many rows of the thread list were built since the panel was
    /// created. The list builds only the rows near its visible part; tests
    /// measure that with this counter.
    #[doc(hidden)]
    pub fn rows_built(&self) -> u64 {
        self.rows_built
    }

    /// Whether a row waits to be brought into view.
    pub(crate) fn reveal_pending(&self) -> bool {
        self.reveal.is_some()
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
            let pick = self.thread_near_caret(cx);
            if pick.is_some() {
                self.set_active(pick, true, cx);
            }
        }
        window.focus(&self.focus_handle);
        cx.notify();
    }

    /// The thread to start from when the list takes focus without an
    /// active thread: the one at the caret, else the next one after it,
    /// else the first.
    pub(super) fn thread_near_caret(&self, cx: &App) -> Option<CommentId> {
        let head: Position = self.editor.read(cx).selection().head;
        self.listed_threads()
            .find(|entry| entry.contains(head))
            .or_else(|| {
                self.listed_threads()
                    .find(|entry| entry.anchor().is_some_and(|anchor| anchor.start > head))
            })
            .or_else(|| self.listed_threads().next())
            .map(|entry| entry.id)
    }

    /// Moves the keyboard between an open composer and the thread list:
    /// from the composer to the list (the draft stays open, the list keys
    /// work beside it), from the list back into the composer. Does nothing
    /// when there is no other place to go: no composer is open, or the
    /// document has no thread to list.
    pub fn cycle_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_open() {
            return;
        }
        if self.composer.focus_handle(cx).is_focused(window) {
            if self.listed_threads().next().is_none() {
                return;
            }
            if self.active.is_none() {
                let pick = self.thread_near_caret(cx);
                self.set_active(pick, true, cx);
            }
            window.focus(&self.focus_handle);
        } else if self.draft.is_some() {
            window.focus(&self.composer.focus_handle(cx));
            self.reveal_draft(cx);
        }
        cx.notify();
    }

    /// Scrolls the list to the card that holds the open composer.
    fn reveal_draft(&mut self, cx: &mut Context<Self>) {
        let Some(thread) = self.draft().and_then(DraftKind::thread) else {
            return;
        };
        if let Some(ix) = self.row_index(Row::Thread(thread)) {
            self.list.scroll_to_reveal_item(ix);
            self.reveal = Some((Row::Thread(thread), 0));
            cx.notify();
        }
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
// Unfinished comments that are set aside
// ---------------------------------------------------------------------------

/// What a draft is about, as text, in `document`: the target text of a new
/// comment, or author, time and body of the comment replied to or edited.
/// `None` when the document has no such text or comment.
fn fingerprint_of(kind: &DraftKind, document: &Document) -> Option<String> {
    let of_thread = |thread: CommentId| {
        document.comment(thread).map(|thread| {
            format!(
                "{}\u{1f}{}\u{1f}{}",
                thread.author, thread.created_at, thread.body
            )
        })
    };
    match *kind {
        DraftKind::New { range } => {
            let valid = !range.is_empty()
                && document.is_valid_position(range.start)
                && document.is_valid_position(range.end);
            valid.then(|| document.text_in(range))
        }
        DraftKind::Reply { thread } | DraftKind::EditComment { thread } => of_thread(thread),
        DraftKind::EditReply { thread, reply } => {
            let reply = document.comment(thread)?.reply(reply)?;
            Some(format!(
                "{}\u{1f}{}\u{1f}{}\u{1f}{}",
                of_thread(thread)?,
                reply.author,
                reply.created_at,
                reply.body
            ))
        }
    }
}

impl CommentsPanel {
    /// Closes the composer without touching focus and sets its draft
    /// aside, unless nothing was written in it. `live`: the draft's
    /// document stays open. Returns whether a draft was kept.
    pub(super) fn keep_draft(&mut self, live: bool, cx: &mut Context<Self>) -> bool {
        let Some(draft) = self.draft.take() else {
            return false;
        };
        let text = self.composer.read(cx).plain_text();
        self.composer.update(cx, |composer, cx| composer.clear(cx));
        self.refresh_draft_highlight(cx);
        cx.notify();
        if text.trim().is_empty() || text == draft.original {
            return false;
        }
        self.kept.push(KeptDraft {
            kind: draft.kind,
            quote: draft.quote,
            text,
            original: draft.original,
            fingerprint: draft.fingerprint,
            tracker: draft.tracker.filter(|_| live),
            live,
        });
        if self.kept.len() > KEPT_DRAFTS_MAX {
            self.kept.remove(0);
        }
        true
    }

    /// Reopens the newest draft that was set aside and fits: one of the
    /// open document, or one whose text or comment is found in it
    /// unchanged. Only while comments are visible and no composer is open.
    /// Focus stays where it is.
    pub(super) fn restore_kept_draft(&mut self, cx: &mut Context<Self>) {
        if self.draft.is_some() || !self.visible || self.kept.is_empty() || self.read_only(cx) {
            return;
        }
        let document = self.editor.read(cx).document();
        let fits = |kept: &KeptDraft| {
            let listed = kept
                .kind
                .thread()
                .is_none_or(|thread| self.is_listed(thread));
            listed
                && (kept.live
                    || fingerprint_of(&kept.kind, document).as_ref() == Some(&kept.fingerprint))
        };
        let Some(ix) = self.kept.iter().rposition(fits) else {
            return;
        };
        let tracker = match (self.kept[ix].tracker.take(), &self.kept[ix].kind) {
            (Some(tracker), _) => Some(tracker),
            (None, DraftKind::New { range }) => Some(RangeTracker::new(document, *range)),
            _ => None,
        };
        let kept = self.kept.remove(ix);
        let placeholder = kept.kind.placeholder();
        self.composer.update(cx, |composer, cx| {
            composer.set_plain_text(&kept.text, cx);
            composer.update_config(|config| config.placeholder = placeholder.into(), cx);
        });
        self.draft = Some(Draft {
            kind: kept.kind,
            quote: kept.quote,
            tracker,
            fingerprint: kept.fingerprint,
            original: kept.original,
            return_to_list: false,
            was_visible: true,
        });
        self.refresh_draft_highlight(cx);
        cx.notify();
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
