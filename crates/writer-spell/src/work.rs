//! The checking itself: what is checked, when, and in which language.
//!
//! # Order
//!
//! One worker task runs slices on the main thread (the system checker is
//! used from there only, see `system`). A slice has a time budget
//! ([`SLICE_BUDGET`]) and takes, in this order:
//!
//! 1. the blocks to check before anything else (edited ones whose pause is
//!    over, lines waiting for the language of the text around them);
//! 2. what is on screen;
//! 3. the rest of the document from where the last slice stopped, but not
//!    while somebody types or scrolls ([`IDLE_PAUSE`]).
//!
//! Which blocks have a result for their current text is the editor's
//! knowledge (`marks`): a block is checked once and then left alone until
//! it changes, the language changes or a word is unlearned. Scrolling over
//! checked text costs a lookup per visible block.
//!
//! A long block is checked in pieces of about [`PIECE_BYTES`], and with a
//! fixed language a piece in at most [`MISSPELLED_PER_CALL`] misspelled
//! words per call (the system checker costs a call per misspelled word:
//! a paragraph in another language would otherwise take 10 ms and more).
//! A block that is not finished when the slice ends is taken up again by
//! the next one.
//!
//! # The word being typed
//!
//! After an edit the block of the caret counts as "being typed in". A
//! result for it is stored without the misspelled word the caret is in or
//! right behind (`Held` keeps the whole result). The word gets its mark
//! from that kept result, without another call, when the caret leaves it
//! or [`TYPING_PAUSE`] after the last edit.
//!
//! # Automatic language
//!
//! The service identifies the language of each piece. When it cannot (a
//! heading of two words), the piece is checked in the language most
//! blocks of the document were identified as, in the user's own variant
//! of it; before any block was identified, in the user's first language.
//! (Left to itself the system checker accepts, in a text it cannot place,
//! every word that any of the user's languages knows: "teh" passes as
//! Indonesian.) Lines checked that way are checked again when the
//! document's language turns out to be another.
//!
//! A document in a language the Mac has no dictionary for shows as blocks
//! in which the service marked words without naming a language. When
//! there are at least three of them and more than blocks of the most
//! frequent identified language, nothing is forced on such text: the
//! service's own answer stands.

use std::rc::Rc;
use std::time::Instant;

use gpui::Context;
use writer_core::{BlockId, Change};
use writer_editor::{Editor, EditorEvent};

use crate::checker::{
    EDIT_PAUSE, Held, IDLE_PAUSE, Partial, SLICE_BUDGET, SLICE_PAUSE, SpellChecker, SpellEvent,
    TYPING_PAUSE, Tag,
};
use crate::marks;
use crate::service::SpellLanguage;
use crate::text::{checkable_text, has_letters, piece_end};

/// About how many bytes of a long block go to the service in one call.
pub(crate) const PIECE_BYTES: usize = 1500;
/// With a fixed language: how many misspelled words one call looks for.
pub(crate) const MISSPELLED_PER_CALL: usize = 16;
/// A change of more blocks than this is not remembered block by block:
/// the background pass goes back to where it starts.
const SMALL_SPLICE: usize = 64;
/// With automatic detection: from this many blocks in which the service
/// marked words without naming a language, the document may count as
/// written in a language it has no dictionary for.
const UNPLACED_BLOCKS: usize = 3;
/// A slice checks at most this many blocks, whatever the clock says.
const SLICE_BLOCKS: usize = 2000;

/// How the work of a slice ended.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Work {
    /// Out of budget: more right away.
    More,
    /// Nothing to do now, but not everything is checked: somebody is
    /// typing or scrolling, or a block waits for its pause. A timer
    /// starts the worker again.
    Waiting,
    /// Every block is checked.
    Done,
}

/// How checking one block ended.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Checked {
    /// The block has its result; whether its marks are different now.
    Done { changed: bool },
    /// Out of time in the middle of the block.
    Interrupted,
}

/// The blocks on screen.
pub(crate) fn visible_blocks(editor: &Editor) -> Option<std::ops::Range<usize>> {
    editor
        .visible_range()
        .map(|range| range.start.block..range.end.block + 1)
}

fn base_of(language: &str) -> &str {
    language.split(['_', '-']).next().unwrap_or(language)
}

impl SpellChecker {
    // --- What the editor tells ---

    pub(crate) fn on_editor_event(&mut self, event: &EditorEvent, cx: &mut Context<Self>) {
        match event {
            EditorEvent::Edited { change } => self.on_edited(change, cx),
            EditorEvent::SelectionChanged => self.on_selection_changed(cx),
            EditorEvent::DocumentReplaced => self.on_document_replaced(cx),
            _ => {}
        }
    }

    /// The editor notified: its view may have moved, or it may have
    /// become read-only.
    pub(crate) fn editor_notified(&mut self, cx: &mut Context<Self>) {
        self.follow_activity(cx);
        if !self.was_active {
            return;
        }
        let editor = self.editor.read(cx);
        let view = (
            editor.scroll_offset(),
            editor.viewport_bounds().map(|bounds| bounds.size),
        );
        if self.last_view == Some(view) {
            return;
        }
        let scrolled = self
            .last_view
            .is_some_and(|(offset, size)| size.is_some() && offset != view.0);
        self.last_view = Some(view);
        // What came into view goes first.
        let unchecked = visible_blocks(editor)
            .is_some_and(|visible| marks::first_unchecked(editor, visible).is_some());
        if scrolled {
            self.hold_background(cx);
        }
        if unchecked {
            self.start_worker(cx);
        }
    }

    /// Starts or stops when the checker becomes active or inactive
    /// (enabled, the editor read-only or compact).
    pub(crate) fn follow_activity(&mut self, cx: &mut Context<Self>) {
        let active = self.is_active(cx);
        if active == self.was_active {
            return;
        }
        self.was_active = active;
        if active {
            self.forget_results();
            self.start_worker(cx);
        } else {
            self.stop_work();
            let had_marks = marks::count(self.editor.read(cx)) > 0;
            marks::clear(&self.editor, cx);
            self.forget_results();
            if had_marks {
                cx.emit(SpellEvent::MarksChanged);
            }
        }
    }

    fn on_document_replaced(&mut self, cx: &mut Context<Self>) {
        // The words ignored in the old document go with it.
        self.service.close_document(self.document);
        self.document = self.service.open_document();
        self.stop_work();
        self.forget_results();
        self.last_view = None;
        // The editor dropped the marks of the old document itself.
        self.start_worker(cx);
    }

    fn on_edited(&mut self, change: &Change, cx: &mut Context<Self>) {
        if !self.was_active {
            return;
        }
        let editor = self.editor.read(cx);
        let document = editor.document();
        let selection = editor.selection();
        self.selection_at_edit = Some(selection);
        let Some(splice) = change.blocks else {
            return;
        };
        let new_range = splice.new_range();
        if splice.new_len.max(splice.old_len) <= SMALL_SPLICE {
            for index in new_range.clone() {
                if let Some(block) = document.block(index)
                    && !marks::is_checked(editor, index)
                {
                    self.waiting.insert(block.id());
                }
            }
            // The blocks behind the change have moved: so has the place
            // where the background pass goes on.
            let old_end = splice.start + splice.old_len;
            if self.scan >= old_end {
                self.scan = self.scan - splice.old_len + splice.new_len;
            } else if self.scan > splice.start {
                self.scan = new_range.end;
            }
        } else {
            self.scan = self.scan.min(splice.start);
        }
        self.settled = false;

        // The word the caret is in is being typed.
        let caret_block = selection.head.block;
        self.typing = (selection.is_collapsed() && new_range.contains(&caret_block))
            .then(|| document.block(caret_block).map(|block| block.id()))
            .flatten();
        if let Some(held) = &self.held
            && document
                .block_index(held.block)
                .and_then(|index| document.block(index))
                .is_none_or(|block| block.revision() != held.revision)
        {
            self.held = None;
        }

        self.hold_background(cx);
        if self.edit_timer.is_none() {
            self.edit_timer = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(EDIT_PAUSE).await;
                this.update(cx, |this, cx| this.edit_pause_over(cx)).ok();
            }));
        }
        self.typing_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(TYPING_PAUSE).await;
            this.update(cx, |this, cx| this.release_typed_word(cx)).ok();
        }));
    }

    /// The pause after an edit is over: the edited blocks are checked.
    fn edit_pause_over(&mut self, cx: &mut Context<Self>) {
        self.edit_timer = None;
        // The latest first is as good as any order; there are few.
        for id in self.waiting.drain() {
            self.recheck.push_back(id);
        }
        self.start_worker(cx);
    }

    fn on_selection_changed(&mut self, cx: &mut Context<Self>) {
        let selection = self.editor.read(cx).selection();
        if self.selection_at_edit == Some(selection) {
            // The selection the last edit left: typing goes on.
            return;
        }
        self.selection_at_edit = None;
        if self.typing.is_none() && self.held.is_none() {
            return;
        }
        // The caret moved by itself. Still in the word that is held back?
        let still_in_word = self.held.as_ref().is_some_and(|held| {
            let document = self.editor.read(cx).document();
            selection.is_collapsed()
                && document
                    .block(selection.head.block)
                    .is_some_and(|block| block.id() == held.block)
                && held.word.start < selection.head.offset
                && selection.head.offset <= held.word.end
        });
        if !still_in_word {
            self.release_typed_word(cx);
        }
    }

    /// The word the caret was typing gets its mark: the caret left it, or
    /// the pause has passed.
    pub(crate) fn release_typed_word(&mut self, cx: &mut Context<Self>) {
        self.typing = None;
        self.typing_timer = None;
        if let Some(held) = self.held.take()
            && marks::set(&self.editor, held.block, held.revision, held.all, cx)
        {
            cx.emit(SpellEvent::MarksChanged);
        }
        // A result that no longer fits is no loss: the block was edited
        // since and is checked again, now without holding anything back.
    }

    /// Somebody types or scrolls: the background pass waits until
    /// [`IDLE_PAUSE`] after the last sign of it.
    pub(crate) fn hold_background(&mut self, cx: &mut Context<Self>) {
        self.busy = true;
        self.busy_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(IDLE_PAUSE).await;
            this.update(cx, |this, cx| {
                this.busy = false;
                this.busy_timer = None;
                this.start_worker(cx);
            })
            .ok();
        }));
    }

    // --- The worker ---

    /// Starts the worker task unless it is running.
    pub(crate) fn start_worker(&mut self, cx: &mut Context<Self>) {
        if self.working || !self.was_active {
            return;
        }
        self.working = true;
        self.settled = false;
        self.worker = Some(cx.spawn(async move |this, cx| {
            loop {
                let more = this.update(cx, |this, cx| this.run_slice(cx));
                if !matches!(more, Ok(true)) {
                    break;
                }
                cx.background_executor().timer(SLICE_PAUSE).await;
            }
        }));
    }

    /// Stops the worker and every timer.
    pub(crate) fn stop_work(&mut self) {
        self.worker = None;
        self.working = false;
        self.busy = false;
        self.busy_timer = None;
        self.edit_timer = None;
        self.typing_timer = None;
    }

    /// Forgets everything derived from the document's text. The marks in
    /// the editor are the caller's business.
    pub(crate) fn forget_results(&mut self) {
        self.scan = 0;
        self.waiting.clear();
        self.recheck.clear();
        self.partial = None;
        self.settled = false;
        self.typing = None;
        self.held = None;
        self.selection_at_edit = None;
        self.tags.clear();
        self.detected.clear();
        self.unidentified = 0;
        self.fallback = None;
    }

    /// One slice, as the worker runs it. Returns whether there is more to
    /// do right away.
    fn run_slice(&mut self, cx: &mut Context<Self>) -> bool {
        let started = Instant::now();
        let more = self.slice(Some(started + SLICE_BUDGET), false, cx);
        let took = started.elapsed();
        self.slices += 1;
        self.longest_slice = self.longest_slice.max(took);
        if took > SLICE_BUDGET * 2 {
            self.slow_slices += 1;
        }
        if !more {
            self.working = false;
        }
        more
    }

    /// Checks everything that is waiting, now, without regard to time
    /// (the word being typed stays unmarked). For tests and scripts; the
    /// app never needs it.
    pub fn check_all_now(&mut self, cx: &mut Context<Self>) {
        if !self.was_active {
            return;
        }
        for id in self.waiting.drain() {
            self.recheck.push_back(id);
        }
        self.edit_timer = None;
        while self.slice(None, true, cx) {}
    }

    /// Does the work of one slice until `deadline`. `everything`: the
    /// background pass runs although somebody is busy. Returns whether
    /// there is more to do right away.
    fn slice(
        &mut self,
        deadline: Option<Instant>,
        everything: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.is_active(cx) {
            return false;
        }
        let mut budget = Budget {
            deadline,
            blocks: SLICE_BLOCKS,
            changed: false,
        };
        let mut state = self.work(&mut budget, everything, cx);
        // Lines that waited for the language of the text around them:
        // within the same slice when it is known now, so that no mark is
        // drawn and taken back.
        if self.refresh_fallback(cx) && !self.recheck.is_empty() {
            let mut extra = Budget {
                deadline: deadline.map(|_| Instant::now() + SLICE_BUDGET / 2),
                blocks: SLICE_BLOCKS,
                changed: false,
            };
            if !self.work_through_rechecks(&mut extra, cx) {
                state = Work::More;
            }
            budget.changed |= extra.changed;
        }
        if budget.changed {
            cx.emit(SpellEvent::MarksChanged);
        }
        match state {
            Work::More => true,
            Work::Waiting => false,
            Work::Done => {
                if !self.settled {
                    self.settled = true;
                    self.forget_gone_blocks(cx);
                    cx.emit(SpellEvent::Settled);
                    cx.notify();
                }
                false
            }
        }
    }

    fn work(&mut self, budget: &mut Budget, everything: bool, cx: &mut Context<Self>) -> Work {
        // 1. What has to be checked before anything else.
        if !self.work_through_rechecks(budget, cx) {
            return Work::More;
        }

        // 2. What is on screen.
        if let Some(visible) = visible_blocks(self.editor.read(cx)) {
            let mut at = visible.start;
            while let Some(index) = self.next_unchecked(at, visible.end, cx) {
                if !self.check_within(index, budget, cx) {
                    return Work::More;
                }
                at = index + 1;
            }
        }

        // 3. The rest of the document, while nobody is at work in it.
        if self.busy && !everything {
            // The idle timer starts the worker again.
            return Work::Waiting;
        }
        loop {
            let count = self.editor.read(cx).document().block_count();
            let Some(index) = self.next_unchecked(self.scan, count, cx) else {
                self.scan = count;
                break;
            };
            if !self.check_within(index, budget, cx) {
                return Work::More;
            }
            self.scan = index + 1;
        }

        // Every block has its result: a check that was interrupted belongs
        // to a block that has changed or gone since.
        self.partial = None;
        if self.waiting.is_empty() {
            Work::Done
        } else {
            // What was just edited has a timer of its own.
            Work::Waiting
        }
    }

    /// Checks the blocks in `recheck`. Returns whether it got through
    /// them all.
    fn work_through_rechecks(&mut self, budget: &mut Budget, cx: &mut Context<Self>) -> bool {
        while let Some(id) = self.recheck.front().copied() {
            let index = self.editor.read(cx).document().block_index(id);
            let Some(index) = index else {
                self.recheck.pop_front();
                continue;
            };
            if self.waiting.contains(&id) {
                // Edited again meanwhile: its pause decides.
                self.recheck.pop_front();
                continue;
            }
            let go_on = self.check_within(index, budget, cx);
            // Done unless it was interrupted in the middle (then the
            // unfinished check is kept and the block stays first).
            if self.partial.is_none() {
                self.recheck.pop_front();
            }
            if !go_on {
                return self.recheck.is_empty();
            }
        }
        true
    }

    /// The first block in `from..to` that has no result and is not
    /// waiting for its pause.
    fn next_unchecked(&self, from: usize, to: usize, cx: &Context<Self>) -> Option<usize> {
        let editor = self.editor.read(cx);
        let mut at = from;
        loop {
            let index = marks::first_unchecked(editor, at..to)?;
            let waits = !self.waiting.is_empty()
                && editor
                    .document()
                    .block(index)
                    .is_some_and(|block| self.waiting.contains(&block.id()));
            if !waits {
                return Some(index);
            }
            at = index + 1;
        }
    }

    /// Checks one block within the budget. Returns whether the slice may
    /// go on (`false`: out of time, possibly in the middle of this block).
    fn check_within(&mut self, index: usize, budget: &mut Budget, cx: &mut Context<Self>) -> bool {
        match self.check_block(index, budget.deadline, true, cx) {
            Checked::Interrupted => false,
            Checked::Done { changed } => {
                budget.changed |= changed;
                budget.blocks = budget.blocks.saturating_sub(1);
                budget.blocks > 0
                    && budget
                        .deadline
                        .is_none_or(|deadline| Instant::now() < deadline)
            }
        }
    }

    // --- One block ---

    /// Checks the block at `index` and stores its result. With a
    /// `deadline` a long block is left unfinished when the time is up and
    /// taken up again by the next call. `hold_typed_word`: the word the
    /// caret is typing is not marked yet.
    pub(crate) fn check_block(
        &mut self,
        index: usize,
        deadline: Option<Instant>,
        hold_typed_word: bool,
        cx: &mut Context<Self>,
    ) -> Checked {
        let editor = self.editor.read(cx);
        let Some(block) = editor.document().block(index) else {
            return Checked::Done { changed: false };
        };
        let (id, revision) = (block.id(), block.revision());
        let mut partial = match self.partial.take() {
            Some(partial) if partial.block == id && partial.revision == revision => partial,
            _ => Partial {
                block: id,
                revision,
                at: 0,
                found: Vec::new(),
                tag: None,
            },
        };
        if let Some(text) = checkable_text(block) {
            let text: &str = &text;
            while partial.at < text.len() {
                let end = piece_end(text, partial.at, PIECE_BYTES);
                let piece = &text[partial.at..end];
                let (misspelled, covered, tag) = self.check_piece(piece);
                partial.found.extend(
                    misspelled
                        .into_iter()
                        .map(|range| partial.at + range.start..partial.at + range.end),
                );
                // The block's language: what the first piece with an
                // identified language says, else what the first piece
                // says.
                let better = match (&partial.tag, &tag) {
                    (None, _) => true,
                    (Some(Tag::Detected(_)), _) => false,
                    (Some(_), Some(Tag::Detected(_))) => true,
                    _ => false,
                };
                if better {
                    partial.tag = tag;
                }
                // A piece is always left behind, whatever the service
                // says about how far it came.
                partial.at += if covered == 0 || covered > piece.len() {
                    piece.len()
                } else {
                    covered
                };
                while !text.is_char_boundary(partial.at) {
                    partial.at += 1;
                }
                if partial.at < text.len()
                    && deadline.is_some_and(|deadline| Instant::now() >= deadline)
                {
                    self.partial = Some(partial);
                    return Checked::Interrupted;
                }
            }
        }

        // The word that is being typed is not marked yet.
        let selection = editor.selection();
        let mut shown = partial.found;
        if self.held.as_ref().is_some_and(|held| held.block == id) {
            self.held = None;
        }
        if hold_typed_word
            && self.typing == Some(id)
            && selection.is_collapsed()
            && selection.head.block == index
        {
            let caret = selection.head.offset;
            if let Some(at) = shown
                .iter()
                .position(|range| range.start < caret && caret <= range.end)
            {
                self.held = Some(Held {
                    block: id,
                    revision,
                    word: shown[at].clone(),
                    all: shown.clone(),
                });
                shown.remove(at);
            }
        }
        let changed = marks::of_block(editor, index) != shown.as_slice();
        self.set_tag(id, partial.tag);
        marks::set(&self.editor, id, revision, shown, cx);
        Checked::Done { changed }
    }

    /// Asks the service about one piece of text. Returns the misspelled
    /// ranges, how much of the piece they cover, and what was learned
    /// about its language.
    fn check_piece(&mut self, piece: &str) -> (Vec<std::ops::Range<usize>>, usize, Option<Tag>) {
        if !has_letters(piece) {
            return (Vec::new(), piece.len(), None);
        }
        let part =
            self.service
                .check_up_to(piece, MISSPELLED_PER_CALL, &self.language, self.document);
        if !self.language.is_automatic() {
            return (part.check.misspelled, part.covered, None);
        }
        match part.check.language {
            Some(language) => (
                part.check.misspelled,
                part.covered,
                Some(Tag::Detected(self.interned(&language))),
            ),
            // The service could not place the text. In the language of
            // the text around it, when there is one; else its own answer
            // stands.
            None => {
                let flagged = !part.check.misspelled.is_empty();
                match self.fallback.clone() {
                    Some(fallback) => {
                        let fixed = self.service.check_up_to(
                            piece,
                            MISSPELLED_PER_CALL,
                            &SpellLanguage::Fixed(fallback.to_string()),
                            self.document,
                        );
                        (
                            fixed.check.misspelled,
                            fixed.covered,
                            Some(Tag::Unplaced {
                                flagged,
                                checked_in: Some(fallback),
                            }),
                        )
                    }
                    None => (
                        part.check.misspelled,
                        part.covered,
                        Some(Tag::Unplaced {
                            flagged,
                            checked_in: None,
                        }),
                    ),
                }
            }
        }
    }

    // --- Languages ---

    /// One shared string per language name.
    fn interned(&self, language: &str) -> Rc<str> {
        self.detected
            .keys()
            .find(|known| known.as_ref() == language)
            .cloned()
            .unwrap_or_else(|| Rc::from(language))
    }

    /// Records what is known about the language of a block.
    fn set_tag(&mut self, block: BlockId, tag: Option<Tag>) {
        let old = match tag.clone() {
            Some(tag) => self.tags.insert(block, tag),
            None => self.tags.remove(&block),
        };
        if old == tag {
            return;
        }
        match old {
            Some(Tag::Detected(language)) => {
                if let Some(count) = self.detected.get_mut(&language) {
                    *count = count.saturating_sub(1);
                }
            }
            Some(Tag::Unplaced { flagged: true, .. }) => {
                self.unidentified = self.unidentified.saturating_sub(1)
            }
            _ => {}
        }
        match tag {
            Some(Tag::Detected(language)) => *self.detected.entry(language).or_insert(0) += 1,
            Some(Tag::Unplaced { flagged: true, .. }) => self.unidentified += 1,
            _ => {}
        }
    }

    /// The user's preferred languages and every language of the service.
    pub(crate) fn known_languages(&mut self) -> &(Vec<String>, Vec<String>) {
        let service = &self.service;
        self.known_languages.get_or_insert_with(|| {
            (
                service.preferred_languages(),
                service.languages().into_iter().map(|l| l.id).collect(),
            )
        })
    }

    /// The identifier to check and guess in for a language the service
    /// named: the user's own variant of it when they prefer one (`en`
    /// becomes `en_GB`), else the language itself, else any variant the
    /// service has. `None` when the service has no such language.
    pub(crate) fn variant_of(&mut self, language: &str) -> Option<String> {
        let (preferred, available) = self.known_languages();
        let base = base_of(language);
        preferred
            .iter()
            .find(|id| id.as_str() == language)
            .or_else(|| preferred.iter().find(|id| base_of(id) == base))
            .or_else(|| available.iter().find(|id| id.as_str() == language))
            .or_else(|| available.iter().find(|id| base_of(id) == base))
            .cloned()
    }

    /// The language for a text the service cannot name a language for.
    fn fallback_now(&mut self) -> Option<String> {
        let most = self
            .detected
            .iter()
            .filter(|(_, count)| **count > 0)
            .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
            .map(|(language, count)| (language.to_string(), *count));
        // Blocks the service marked words in without naming a language
        // speak for a document in a language it has no dictionary for;
        // one or two can be short lines with a slip of the pen.
        let foreign = self.unidentified >= UNPLACED_BLOCKS;
        match most {
            Some((_, count)) if foreign && self.unidentified > count => None,
            Some((language, _)) => self.variant_of(&language),
            None if foreign => None,
            None => self.known_languages().0.first().cloned(),
        }
    }

    /// Brings the fallback language up to date. When it changed, the
    /// blocks that were checked in the old one are queued to be checked
    /// again; returns whether it changed.
    fn refresh_fallback(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.language.is_automatic() {
            return false;
        }
        let now: Option<Rc<str>> = self
            .fallback_now()
            .map(|language| Rc::from(language.as_str()));
        if now == self.fallback {
            return false;
        }
        self.fallback = now.clone();
        let stale: Vec<BlockId> = self
            .tags
            .iter()
            .filter(
                |(_, tag)| matches!(tag, Tag::Unplaced { checked_in, .. } if *checked_in != now),
            )
            .map(|(id, _)| *id)
            .collect();
        if stale.is_empty() {
            return true;
        }
        // In document order, so that what is on screen does not flicker
        // from the bottom up.
        let document = self.editor.read(cx).document();
        let wanted: std::collections::HashSet<BlockId> = stale.into_iter().collect();
        for block in document.blocks() {
            if wanted.contains(&block.id()) && !self.recheck.contains(&block.id()) {
                self.recheck.push_back(block.id());
            }
        }
        self.settled = false;
        true
    }

    /// Drops what is remembered about blocks that are not in the document
    /// any more.
    fn forget_gone_blocks(&mut self, cx: &mut Context<Self>) {
        let document = self.editor.read(cx).document();
        if self.tags.len() <= document.block_count() + 64 {
            return;
        }
        let ids: std::collections::HashSet<BlockId> =
            document.blocks().iter().map(|block| block.id()).collect();
        let gone: Vec<BlockId> = self
            .tags
            .keys()
            .filter(|id| !ids.contains(id))
            .copied()
            .collect();
        for id in gone {
            self.set_tag(id, None);
        }
    }
}

/// What a slice has left.
struct Budget {
    deadline: Option<Instant>,
    blocks: usize,
    /// Whether the marks of a block changed.
    changed: bool,
}
