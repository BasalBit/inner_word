//! The composer: starting, saving and discarding a new comment, a reply or
//! an edit.

use gpui::{App, Context, Focusable, Window};
use innerword_core::{Selection, TextRange};
use innerword_editor::HighlightStyle;
use innerword_ui::theme::ActiveTheme;

use super::{CommentsPanel, DRAFT_HIGHLIGHT_LAYER, Draft, DraftKind, fingerprint_of};
use crate::{
    threads::{quote, word_range_at},
    track::RangeTracker,
};

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
        let target = self.comment_target(cx);
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
            let tracker = RangeTracker::new(self.editor.read(cx).document(), range);
            if let Some(draft) = &mut self.draft {
                draft.kind = DraftKind::New {
                    range: tracker.range(),
                };
                draft.quote = quote;
                draft.fingerprint = tracker.text().to_string();
                draft.tracker = Some(tracker);
            }
            self.refresh_draft_highlight(cx);
            window.focus(&self.composer.focus_handle(cx));
            cx.notify();
            return;
        }
        self.open_draft(DraftKind::New { range }, quote, "", window, cx);
    }

    /// The text [`CommentsPanel::add_comment`] would anchor a comment to
    /// right now, and its quote: the selection, or the word at a collapsed
    /// caret. `None` when there is no text there.
    fn comment_target(&self, cx: &App) -> Option<(TextRange, String)> {
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
    }

    /// Whether [`CommentsPanel::add_comment`] would open (or re-aim) a
    /// composer right now: the document can be changed and there is text
    /// to anchor to, a selection or the word at the caret. For an "Add
    /// Comment" item that is shown disabled when it cannot apply.
    pub fn can_add_comment(&self, cx: &App) -> bool {
        !self.read_only(cx) && self.comment_target(cx).is_some()
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
        // Another composer with text in it is set aside, not overwritten:
        // it reopens when this one closes.
        self.keep_draft(true, cx);
        // Showing comments would bring a kept draft back into the composer
        // this one is about to use; it waits its turn instead.
        let kept = std::mem::take(&mut self.kept);
        self.set_visible(true, window, cx);
        self.kept = kept;
        let placeholder = kind.placeholder();
        self.composer.update(cx, |composer, cx| {
            composer.set_plain_text(text, cx);
            composer.update_config(|config| config.placeholder = placeholder.into(), cx);
        });
        let document = self.editor.read(cx).document();
        let tracker = match kind {
            DraftKind::New { range } => Some(RangeTracker::new(document, range)),
            _ => None,
        };
        let fingerprint = match &tracker {
            Some(tracker) => tracker.text().to_string(),
            None => fingerprint_of(&kind, document).unwrap_or_default(),
        };
        self.draft = Some(Draft {
            kind,
            quote,
            tracker,
            fingerprint,
            original: text.to_string(),
            return_to_list,
            was_visible,
        });
        self.refresh_draft_highlight(cx);
        // A reply or an edit grows its card: keep the card in view.
        self.scroll_list_to_active(cx);
        window.focus(&self.composer.focus_handle(cx));
        cx.notify();
    }

    /// Marks the target of a new-comment draft in the text, or removes the
    /// mark when there is no such draft.
    pub(super) fn refresh_draft_highlight(&mut self, cx: &mut Context<Self>) {
        let range = match self.draft() {
            Some(DraftKind::New { range }) => Some(*range),
            _ => None,
        };
        let range = range.filter(|range| !range.is_empty() && self.visible);
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
    /// saves nothing and leaves the composer open; so does a new comment
    /// whose text was deleted meanwhile
    /// ([`CommentsPanel::draft_target_lost`]). Afterwards focus returns
    /// to where the draft was started (the editor for a new comment, the
    /// thread list for a reply or an edit made there), and a new thread is
    /// the active one.
    pub fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.draft.is_none() || self.read_only(cx) {
            return;
        }
        // The target as it is in the document now.
        self.track_edit(None, cx);
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
                if range.is_empty() {
                    self.notice(
                        "The text this comment was for is gone. Select text and add the comment again.",
                        cx,
                    );
                    return;
                }
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
                let Some(Some(id)) = created else {
                    // Nothing to anchor to after all: keep what was written.
                    return;
                };
                self.set_active(Some(id), false, cx);
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
        // A draft that was set aside for this one takes the composer back.
        self.restore_kept_draft(cx);
        cx.notify();
    }

    /// Drops the draft without touching focus (the caller repairs it).
    pub(super) fn discard_draft(&mut self, cx: &mut Context<Self>) {
        if self.draft.take().is_some() {
            self.composer.update(cx, |composer, cx| composer.clear(cx));
            self.refresh_draft_highlight(cx);
            cx.notify();
        }
    }
}
