//! What is done with a marked word: suggestions, replacing it, learning
//! and ignoring it, the rows for a menu.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use gpui::{App, Context, Window};
use writer_core::{BlockId, Position, TextRange};
use writer_editor::ContextMenuRequest;
use writer_ui::toast::{Toast, show_toast};
use writer_ui::{ContextMenu, ContextMenuItem, MenuAnchor};

use crate::checker::{
    IGNORE_SPELLING, LEARN_SPELLING, MAX_SUGGESTIONS, NO_GUESSES, SpellChecker, SpellEvent, Tag,
};
use crate::marks;
use crate::service::SpellLanguage;
use crate::text::{checkable_text, common_ends, window_around, word_at};
use crate::work::Checked;

/// How much text to either side of a word goes to the service with it
/// when it is asked for suggestions.
const GUESS_REACH: usize = 600;

impl SpellChecker {
    // --- Queries (the Vim keys use them) ---

    /// The marked word at `position`: the mark that contains it or ends
    /// or starts there.
    pub fn misspelling_at(&self, position: Position, cx: &App) -> Option<TextRange> {
        if !self.is_active(cx) {
            return None;
        }
        marks::at(self.editor.read(cx), position)
    }

    /// The suggestions for the marked word at `position`, best first, at
    /// most [`MAX_SUGGESTIONS`]. Empty when the word is not marked or the
    /// service has none.
    pub fn suggestions_at(&mut self, position: Position, cx: &mut Context<Self>) -> Vec<String> {
        if !self.was_active {
            return Vec::new();
        }
        self.settle_block(position.block, cx);
        match marks::at(self.editor.read(cx), position) {
            Some(range) => self.suggestions_for(range, cx),
            None => Vec::new(),
        }
    }

    /// The word at `position` (marked or not): its range and text.
    pub fn word_at(&self, position: Position, cx: &App) -> Option<(TextRange, String)> {
        let editor = self.editor.read(cx);
        let document = editor.document();
        let range = marks::at(editor, position).or_else(|| {
            let block = document.block(position.block)?;
            let word = word_at(block.text(), position.offset)?;
            Some(TextRange::in_block(position.block, word.start, word.end))
        })?;
        Some((range, document.text_in(range)))
    }

    /// Replaces the text at `range` (a marked word) by `replacement` as
    /// one undo step, keeping the formatting of the replaced text, and
    /// puts the caret after it. Returns whether the document changed.
    ///
    /// Only the letters that differ are replaced, so a word that is bold
    /// in part stays bold in that part; what is put in takes the
    /// formatting of what it replaces.
    pub fn replace(&mut self, range: TextRange, replacement: &str, cx: &mut Context<Self>) -> bool {
        let editor = self.editor.read(cx);
        if editor.is_read_only() || range.start.block != range.end.block {
            return false;
        }
        let block_index = range.start.block;
        let Some(block) = editor.document().block(block_index) else {
            return false;
        };
        let Some(old) = block.text().get(range.start.offset..range.end.offset) else {
            return false;
        };
        // The document's own apostrophe, where it sets them.
        let replacement = if editor.config().smart_punctuation {
            replacement.replace('\'', "\u{2019}")
        } else {
            replacement.to_string()
        };
        if old.is_empty() || replacement.is_empty() || old == replacement {
            return false;
        }
        let from = old.to_string();
        let (prefix, suffix) = common_ends(old, &replacement);
        let changed_part = TextRange::in_block(
            block_index,
            range.start.offset + prefix,
            range.end.offset - suffix,
        );
        let put_in = replacement[prefix..replacement.len() - suffix].to_string();
        let caret = Position::new(block_index, range.start.offset + replacement.len());
        let changed = self.editor.update(cx, |editor, cx| {
            editor.edit(
                |state| {
                    state.grouped(|state| {
                        let changed = state.replace_range(changed_part, &put_in);
                        state.set_caret(caret);
                        changed
                    })
                },
                cx,
            )
        });
        if changed {
            cx.emit(SpellEvent::Replaced {
                from,
                to: replacement,
            });
        }
        changed
    }

    /// Learns the marked word at `position` (permanent). Every occurrence
    /// in the document loses its mark at once. Returns the word.
    pub fn learn_at(&mut self, position: Position, cx: &mut Context<Self>) -> Option<String> {
        let word = self.marked_word_at(position, cx)?;
        self.learn_word(&word, cx);
        Some(word)
    }

    /// Takes the word at `position` out of the user's own dictionary, if
    /// it was learned; the document is checked again. Returns the word.
    pub fn unlearn_at(&mut self, position: Position, cx: &mut Context<Self>) -> Option<String> {
        if !self.was_active {
            return None;
        }
        let (_, word) = self.word_at(position, cx)?;
        if !self.service.has_learned(&word) {
            return None;
        }
        self.unlearn_word(&word, cx);
        Some(word)
    }

    /// Ignores the marked word at `position` in this document until it is
    /// closed. Every occurrence loses its mark at once. Returns the word.
    pub fn ignore_at(&mut self, position: Position, cx: &mut Context<Self>) -> Option<String> {
        let word = self.marked_word_at(position, cx)?;
        self.ignore_word(&word, cx);
        Some(word)
    }

    /// Learns `word` (permanent) and removes its marks.
    pub fn learn_word(&mut self, word: &str, cx: &mut Context<Self>) {
        let word = word.trim();
        if word.is_empty() {
            return;
        }
        self.service.learn(word);
        self.drop_marks_of(word, cx);
        cx.emit(SpellEvent::Learned(word.to_string()));
    }

    /// Unlearns `word` and checks the document again (the marks stay
    /// where they are until the new results replace them).
    pub fn unlearn_word(&mut self, word: &str, cx: &mut Context<Self>) {
        let word = word.trim();
        if word.is_empty() {
            return;
        }
        self.service.unlearn(word);
        if self.was_active {
            marks::invalidate(&self.editor, cx);
            self.scan = 0;
            self.held = None;
            self.settled = false;
            self.start_worker(cx);
        }
        cx.emit(SpellEvent::Unlearned(word.to_string()));
    }

    /// Ignores `word` in this document and removes its marks.
    pub fn ignore_word(&mut self, word: &str, cx: &mut Context<Self>) {
        let word = word.trim();
        if word.is_empty() {
            return;
        }
        self.service.ignore(word, self.document);
        self.drop_marks_of(word, cx);
        cx.emit(SpellEvent::Ignored(word.to_string()));
    }

    // --- Commands ---

    /// Learns the marked word at the caret (or the marked word that is
    /// selected) and says so, with a way back. Returns whether there was
    /// one.
    pub fn learn_at_caret(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.ready_for_command(cx) {
            return false;
        }
        let at = self.caret(cx);
        match self.learn_at(at, cx) {
            Some(word) => {
                self.say_learned(&word, cx);
                true
            }
            None => {
                self.notice("No misspelled word at the caret", cx);
                false
            }
        }
    }

    /// Unlearns the word at the caret, if it was learned.
    pub fn unlearn_at_caret(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.ready_for_command(cx) {
            return false;
        }
        let at = self.caret(cx);
        match self.unlearn_at(at, cx) {
            Some(word) => {
                self.notice(format!("Unlearned \u{201c}{word}\u{201d}"), cx);
                true
            }
            None => {
                self.notice("No learned word at the caret", cx);
                false
            }
        }
    }

    /// Ignores the marked word at the caret (or the marked word that is
    /// selected) in this document. Returns whether there was one.
    pub fn ignore_at_caret(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.ready_for_command(cx) {
            return false;
        }
        let at = self.caret(cx);
        if self.ignore_at(at, cx).is_some() {
            true
        } else {
            self.notice("No misspelled word at the caret", cx);
            false
        }
    }

    /// Opens the suggestions for the marked word at the caret as a menu
    /// under it, with the first row marked (the keyboard path; `z=` in Vim
    /// mode). Returns whether a menu was opened.
    pub fn show_suggestions(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.ready_for_command(cx) {
            return false;
        }
        let at = self.caret(cx);
        self.release_typed_word(cx);
        self.settle_block(at.block, cx);
        let Some(range) = marks::at(self.editor.read(cx), at) else {
            self.notice("No misspelled word at the caret", cx);
            return false;
        };
        let items = self.menu_items_for(range, cx);
        let editor = self.editor.read(cx);
        let anchor = editor
            .range_bounds(range)
            .and_then(|rows| rows.first().copied())
            .or_else(|| editor.caret_bounds());
        let Some(anchor) = anchor else {
            return false;
        };
        let menu = ContextMenu::open(MenuAnchor::Below(anchor), items, window, cx);
        menu.update(cx, |menu, cx| menu.select_first(cx));
        true
    }

    // --- The context menu ---

    /// The rows that lead the text context menu for `request`: when the
    /// request is on a marked word, up to [`MAX_SUGGESTIONS`] suggestions
    /// (or a disabled [`NO_GUESSES`] row), [`LEARN_SPELLING`],
    /// [`IGNORE_SPELLING`]; otherwise nothing. The caller adds the
    /// separator and the rest of the menu.
    pub fn context_menu_items(
        &mut self,
        request: &ContextMenuRequest,
        cx: &mut Context<Self>,
    ) -> Vec<ContextMenuItem> {
        if request.on_checkbox {
            return Vec::new();
        }
        self.menu_items_at(request.document_position, cx)
    }

    /// The same rows for the marked word at `position`.
    pub fn menu_items_at(
        &mut self,
        position: Position,
        cx: &mut Context<Self>,
    ) -> Vec<ContextMenuItem> {
        if !self.was_active {
            return Vec::new();
        }
        // A question about this word: it is not "still being typed".
        self.release_typed_word(cx);
        self.settle_block(position.block, cx);
        match marks::at(self.editor.read(cx), position) {
            Some(range) => self.menu_items_for(range, cx),
            None => Vec::new(),
        }
    }

    // --- Inside ---

    fn menu_items_for(&mut self, range: TextRange, cx: &mut Context<Self>) -> Vec<ContextMenuItem> {
        let word = self.editor.read(cx).document().text_in(range);
        let suggestions = self.suggestions_for(range, cx);
        let this = cx.weak_entity();
        let mut items = Vec::with_capacity(suggestions.len() + 3);
        if suggestions.is_empty() {
            items.push(ContextMenuItem::new(NO_GUESSES, |_, _| {}).disabled(true));
        }
        for suggestion in suggestions {
            let (this, word, title) = (this.clone(), word.clone(), suggestion.clone());
            items.push(ContextMenuItem::new(title, move |_, cx| {
                this.update(cx, |this, cx| {
                    // Only while the word is still what the menu was
                    // made for.
                    if this.editor.read(cx).document().text_in(range) == word {
                        this.replace(range, &suggestion, cx);
                    }
                })
                .ok();
            }));
        }
        items.push(ContextMenuItem::new(LEARN_SPELLING, {
            let (this, word) = (this.clone(), word.clone());
            move |_, cx| {
                this.update(cx, |this, cx| {
                    this.learn_word(&word, cx);
                    this.say_learned(&word, cx);
                })
                .ok();
            }
        }));
        items.push(ContextMenuItem::new(IGNORE_SPELLING, move |_, cx| {
            this.update(cx, |this, cx| this.ignore_word(&word, cx)).ok();
        }));
        items
    }

    /// Says that a word was learned, with a button that takes it back:
    /// learning is permanent and reaches every app on the Mac.
    fn say_learned(&mut self, word: &str, cx: &mut Context<Self>) {
        let message = format!("Learned \u{201c}{word}\u{201d}");
        self.last_notice = Some(message.clone().into());
        let (this, learned) = (cx.weak_entity(), word.to_string());
        show_toast(
            Toast::new(message.clone()).on_action("Undo", move |_, cx| {
                this.update(cx, |this, cx| this.unlearn_word(&learned, cx))
                    .ok();
            }),
            cx,
        );
        cx.emit(SpellEvent::Notice(message.into()));
    }

    /// The marked word at `position`, as text. The block is checked first
    /// when it has no result for its current text.
    fn marked_word_at(&mut self, position: Position, cx: &mut Context<Self>) -> Option<String> {
        if !self.was_active {
            return None;
        }
        self.release_typed_word(cx);
        self.settle_block(position.block, cx);
        let editor = self.editor.read(cx);
        let range = marks::at(editor, position)?;
        Some(editor.document().text_in(range))
    }

    /// Checks the block at `index` now when it has no result for its
    /// current text (it was just edited, or the background pass has not
    /// come to it).
    pub(crate) fn settle_block(&mut self, index: usize, cx: &mut Context<Self>) {
        let editor = self.editor.read(cx);
        if marks::is_checked(editor, index) {
            return;
        }
        let Some(block) = editor.document().block(index) else {
            return;
        };
        let id = block.id();
        self.waiting.remove(&id);
        if let Checked::Done { changed: true } = self.check_block(index, None, false, cx) {
            cx.emit(SpellEvent::MarksChanged);
        }
    }

    /// The suggestions for the marked word at `range`.
    fn suggestions_for(&mut self, range: TextRange, cx: &mut Context<Self>) -> Vec<String> {
        let editor = self.editor.read(cx);
        let Some(block) = editor.document().block(range.start.block) else {
            return Vec::new();
        };
        let Some(text) = checkable_text(block) else {
            return Vec::new();
        };
        let word = range.start.offset..range.end.offset;
        let Some(original) = text.get(word.clone()) else {
            return Vec::new();
        };
        // The service guesses in context, but a whole chapter in one
        // block is more context than it needs.
        let window = window_around(&text, word.clone(), GUESS_REACH);
        let piece = &text[window.clone()];
        let in_piece = word.start - window.start..word.end - window.start;
        let language = match &self.language {
            SpellLanguage::Fixed(_) => self.language.clone(),
            // The language the block was checked in.
            SpellLanguage::Automatic => match self.tags.get(&block.id()).cloned() {
                Some(Tag::Detected(language)) => self
                    .variant_of(&language)
                    .map_or(SpellLanguage::Automatic, SpellLanguage::Fixed),
                Some(Tag::Fallback(Some(language))) => SpellLanguage::Fixed(language.to_string()),
                _ => SpellLanguage::Automatic,
            },
        };
        let mut seen = HashSet::new();
        self.service
            .suggestions(piece, in_piece, &language, self.document)
            .into_iter()
            .filter(|suggestion| {
                !suggestion.trim().is_empty()
                    && suggestion != original
                    && seen.insert(suggestion.clone())
            })
            .take(MAX_SUGGESTIONS)
            .collect()
    }

    /// Takes the marks off every occurrence of `word` (which was just
    /// learned or ignored), at once and without checking the document
    /// again: the marks with exactly that text go, and those that only
    /// differ in case or contain the word (a plural, a possessive) go
    /// when the service accepts them now.
    fn drop_marks_of(&mut self, word: &str, cx: &mut Context<Self>) {
        let editor = self.editor.read(cx);
        if marks::count(editor) == 0 && self.held.is_none() {
            return;
        }
        let language = match &self.language {
            SpellLanguage::Fixed(_) => Some(self.language.clone()),
            SpellLanguage::Automatic => self
                .fallback
                .as_ref()
                .map(|language| SpellLanguage::Fixed(language.to_string())),
        };
        let lower = word.to_lowercase();
        let mut verdicts: HashMap<String, bool> = HashMap::new();
        let service = self.service.clone();
        let document_tag = self.document;
        let mut goes = |candidate: &str| -> bool {
            if candidate == word {
                return true;
            }
            if !candidate.to_lowercase().contains(&lower) {
                return false;
            }
            let Some(language) = &language else {
                return false;
            };
            *verdicts.entry(candidate.to_string()).or_insert_with(|| {
                service
                    .check(candidate, language, document_tag)
                    .misspelled
                    .is_empty()
            })
        };
        let mut updates: Vec<(BlockId, u64, Vec<Range<usize>>)> = Vec::new();
        for (index, block) in editor.document().blocks().iter().enumerate() {
            let ranges = marks::of_block(editor, index);
            // A block without a result for its text is checked again
            // anyway, and then with the word known.
            if ranges.is_empty() || !marks::is_checked(editor, index) {
                continue;
            }
            let text = block.text();
            let kept: Vec<Range<usize>> = ranges
                .iter()
                .filter(|range| !text.get((*range).clone()).is_some_and(&mut goes))
                .cloned()
                .collect();
            if kept.len() != ranges.len() {
                updates.push((block.id(), block.revision(), kept));
            }
        }
        // The result that is held back for the word being typed.
        let held_text = self.held.as_ref().and_then(|held| {
            let document = editor.document();
            let index = document.block_index(held.block)?;
            let block = document.block(index)?;
            (block.revision() == held.revision).then(|| block.text().to_string())
        });
        if let (Some(held), Some(text)) = (self.held.as_mut(), held_text) {
            held.all
                .retain(|range| !text.get(range.clone()).is_some_and(&mut goes));
            if !held.all.contains(&held.word) {
                self.held = None;
            }
        }
        let changed = !updates.is_empty();
        for (id, revision, kept) in updates {
            marks::set(&self.editor, id, revision, kept, cx);
        }
        if changed {
            cx.emit(SpellEvent::MarksChanged);
        }
    }
}
