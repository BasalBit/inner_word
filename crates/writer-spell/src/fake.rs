//! A deterministic spell service for tests, built from word lists.
//!
//! Tests must not depend on the Mac's dictionary (its answers differ
//! between macOS versions), so every test in every crate uses this one:
//!
//! ```ignore
//! let service = Rc::new(
//!     FakeSpellService::with_words(["the", "harbor", "was", "grey"])
//!         .language("en_GB", "British English", ["the", "harbour", "was", "grey"])
//!         .suggest("teh", ["the", "ten"]),
//! );
//! let checker = cx.new(|cx| SpellChecker::new(editor.clone(), service.clone(), cx));
//! // ...
//! assert_eq!(service.check_count(), 1);
//! ```
//!
//! # The rules
//!
//! - A word is a run of letters; an apostrophe between two letters
//!   belongs to it (`don't`). A run with a digit in it is never
//!   misspelled, and neither is anything in a stretch without white space
//!   that contains `://` or `@` (addresses).
//! - A word is right when its lower-case form is in the list of the
//!   language, or was learned, or is ignored in the document.
//! - A fixed language that the fake does not have flags nothing.
//! - Automatic: the language in whose list most words of the text are
//!   found; when several tie, the first of them in the order preferred
//!   languages, then all languages. That language is reported as detected.
//!   When no word of the text is in any list the language is not
//!   identified and nothing is flagged (the system checker does the same
//!   with a line of two misspelled words).
//! - [`SpellService::check_up_to`] stops after the number of misspelled
//!   words it was given and reports how far it came; every call counts as
//!   a check.
//! - Suggestions are scripted ([`FakeSpellService::suggest`]); a word
//!   without a script has none.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::ops::Range;
use std::time::{Duration, Instant};

use crate::service::{
    LanguageInfo, SpellCheck, SpellCheckPart, SpellDocument, SpellLanguage, SpellService,
};

/// The identifier of the language [`FakeSpellService::with_words`] makes.
pub const FAKE_LANGUAGE: &str = "en";

struct FakeLanguage {
    info: LanguageInfo,
    words: HashSet<String>,
}

#[derive(Default)]
struct Inner {
    languages: Vec<FakeLanguage>,
    preferred: Vec<String>,
    suggestions: HashMap<String, Vec<String>>,
    learned: BTreeSet<String>,
    ignored: HashMap<SpellDocument, BTreeSet<String>>,
    open: BTreeSet<SpellDocument>,
    next_tag: i64,
    check_cost: Duration,
    checks: Vec<String>,
    suggestion_calls: usize,
    warm_ups: usize,
}

/// The fake spell service. See the [module docs](self).
pub struct FakeSpellService {
    inner: RefCell<Inner>,
}

impl Default for FakeSpellService {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeSpellService {
    /// A service without any language.
    pub fn new() -> Self {
        FakeSpellService {
            inner: RefCell::new(Inner {
                next_tag: 1,
                ..Inner::default()
            }),
        }
    }

    /// A service with one language, [`FAKE_LANGUAGE`] ("English"), that
    /// knows `words` (compared in lower case).
    pub fn with_words<I, S>(words: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Self::new().language(FAKE_LANGUAGE, "English", words)
    }

    /// Adds a language (or replaces the one with this identifier). The
    /// first language added is the first preferred one unless
    /// [`FakeSpellService::prefer`] says otherwise.
    pub fn language<I, S>(self, id: &str, name: &str, words: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        {
            let mut inner = self.inner.borrow_mut();
            let language = FakeLanguage {
                info: LanguageInfo::new(id, name),
                words: words
                    .into_iter()
                    .map(|w| w.as_ref().to_lowercase())
                    .collect(),
            };
            match inner.languages.iter_mut().find(|l| l.info.id == id) {
                Some(existing) => *existing = language,
                None => inner.languages.push(language),
            }
        }
        self
    }

    /// Sets the user's preferred languages, best first.
    pub fn prefer<I, S>(self, ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.inner.borrow_mut().preferred =
            ids.into_iter().map(|id| id.as_ref().to_string()).collect();
        self
    }

    /// Scripts the suggestions for a misspelled word (compared in lower
    /// case).
    pub fn suggest<I, S>(self, word: &str, suggestions: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.inner.borrow_mut().suggestions.insert(
            word.to_lowercase(),
            suggestions
                .into_iter()
                .map(|s| s.as_ref().to_string())
                .collect(),
        );
        self
    }

    /// Makes every [`SpellService::check`] take at least `cost` (the call
    /// spins that long), to stand in for the real checker in tests of the
    /// time budget.
    pub fn check_cost(self, cost: Duration) -> Self {
        self.inner.borrow_mut().check_cost = cost;
        self
    }

    /// How many times [`SpellService::check`] was called since the last
    /// [`FakeSpellService::reset_calls`].
    pub fn check_count(&self) -> usize {
        self.inner.borrow().checks.len()
    }

    /// The texts that were checked since the last
    /// [`FakeSpellService::reset_calls`], in order, as they were passed
    /// in.
    pub fn checked_texts(&self) -> Vec<String> {
        self.inner.borrow().checks.clone()
    }

    /// How many times [`SpellService::suggestions`] was called since the
    /// last [`FakeSpellService::reset_calls`].
    pub fn suggestion_count(&self) -> usize {
        self.inner.borrow().suggestion_calls
    }

    /// How many times [`SpellService::warm_up`] was called.
    pub fn warm_up_count(&self) -> usize {
        self.inner.borrow().warm_ups
    }

    /// Forgets the calls counted so far.
    pub fn reset_calls(&self) {
        let mut inner = self.inner.borrow_mut();
        inner.checks.clear();
        inner.suggestion_calls = 0;
    }

    /// The learned words, sorted.
    pub fn learned_words(&self) -> Vec<String> {
        self.inner.borrow().learned.iter().cloned().collect()
    }

    /// The documents that were opened and not closed.
    pub fn open_documents(&self) -> Vec<SpellDocument> {
        self.inner.borrow().open.iter().copied().collect()
    }
}

/// The byte ranges of the words of `text` that can be misspelled: runs of
/// letters (with apostrophes inside), without runs that contain a digit
/// and without anything in an address.
fn candidate_words(text: &str) -> Vec<Range<usize>> {
    let mut words = Vec::new();
    let mut chunk_start = 0;
    let push_chunk = |chunk: Range<usize>, words: &mut Vec<Range<usize>>| {
        let piece = &text[chunk.clone()];
        if piece.contains("://") || piece.contains('@') {
            return;
        }
        let chars: Vec<(usize, char)> = piece.char_indices().collect();
        let mut i = 0;
        while i < chars.len() {
            let (start, c) = chars[i];
            if !c.is_alphanumeric() {
                i += 1;
                continue;
            }
            let mut end = start + c.len_utf8();
            let mut digit = c.is_numeric();
            let mut j = i + 1;
            while j < chars.len() {
                let (at, c) = chars[j];
                let inside_apostrophe = (c == '\'' || c == '\u{2019}')
                    && chars
                        .get(j + 1)
                        .is_some_and(|(_, next)| next.is_alphanumeric());
                // Combining marks stay with the letter before them.
                let combining = ('\u{0300}'..='\u{036F}').contains(&c);
                if c.is_alphanumeric() || inside_apostrophe || combining {
                    digit |= c.is_numeric();
                    end = at + c.len_utf8();
                    j += 1;
                } else {
                    break;
                }
            }
            if !digit {
                words.push(chunk.start + start..chunk.start + end);
            }
            i = j;
        }
    };
    for (at, c) in text.char_indices() {
        if c.is_whitespace() {
            if chunk_start < at {
                push_chunk(chunk_start..at, &mut words);
            }
            chunk_start = at + c.len_utf8();
        }
    }
    if chunk_start < text.len() {
        push_chunk(chunk_start..text.len(), &mut words);
    }
    words
}

fn normalized(word: &str) -> String {
    word.replace('\u{2019}', "'").to_lowercase()
}

impl Inner {
    fn language(&self, id: &str) -> Option<&FakeLanguage> {
        self.languages.iter().find(|l| l.info.id == id)
    }

    fn preferred_order(&self) -> Vec<String> {
        let mut order: Vec<String> = self
            .preferred
            .iter()
            .filter(|id| self.language(id).is_some())
            .cloned()
            .collect();
        for language in &self.languages {
            if !order.contains(&language.info.id) {
                order.push(language.info.id.clone());
            }
        }
        order
    }

    /// The language most words of `text` belong to; none when no word of
    /// the text is in any list.
    fn detect(&self, text: &str) -> Option<String> {
        let words: Vec<String> = candidate_words(text)
            .into_iter()
            .map(|r| normalized(&text[r]))
            .collect();
        let mut best: Option<(usize, String)> = None;
        for id in self.preferred_order() {
            let language = self.language(&id)?;
            let known = words.iter().filter(|w| language.words.contains(*w)).count();
            if known > 0 && best.as_ref().is_none_or(|(most, _)| known > *most) {
                best = Some((known, id));
            }
        }
        best.map(|(_, id)| id)
    }

    fn resolve(&self, text: &str, language: &SpellLanguage) -> (Option<String>, bool) {
        match language {
            SpellLanguage::Automatic => (self.detect(text), true),
            SpellLanguage::Fixed(id) => (Some(id.clone()), false),
        }
    }

    fn is_accepted(&self, word: &str, language: &FakeLanguage, document: SpellDocument) -> bool {
        let word = normalized(word);
        language.words.contains(&word)
            || self.learned.contains(&word)
            || self
                .ignored
                .get(&document)
                .is_some_and(|words| words.contains(&word))
    }
}

impl SpellService for FakeSpellService {
    fn check(&self, text: &str, language: &SpellLanguage, document: SpellDocument) -> SpellCheck {
        self.check_up_to(text, usize::MAX, language, document).check
    }

    fn check_up_to(
        &self,
        text: &str,
        max_misspelled: usize,
        language: &SpellLanguage,
        document: SpellDocument,
    ) -> SpellCheckPart {
        let started = Instant::now();
        let mut inner = self.inner.borrow_mut();
        inner.checks.push(text.to_string());
        let (id, detected) = inner.resolve(text, language);
        let max = max_misspelled.max(1);
        let mut misspelled = Vec::new();
        let mut covered = text.len();
        if let Some(language) = id.as_deref().and_then(|id| inner.language(id)) {
            for word in candidate_words(text) {
                if !inner.is_accepted(&text[word.clone()], language, document) {
                    covered = word.end;
                    misspelled.push(word);
                    if misspelled.len() >= max {
                        break;
                    }
                }
            }
            if misspelled.len() < max {
                covered = text.len();
            }
        }
        let cost = inner.check_cost;
        drop(inner);
        while started.elapsed() < cost {
            std::hint::spin_loop();
        }
        SpellCheckPart {
            check: SpellCheck {
                misspelled,
                language: if detected { id } else { None },
            },
            covered,
        }
    }

    fn suggestions(
        &self,
        text: &str,
        word: Range<usize>,
        _language: &SpellLanguage,
        _document: SpellDocument,
    ) -> Vec<String> {
        let mut inner = self.inner.borrow_mut();
        inner.suggestion_calls += 1;
        text.get(word)
            .and_then(|word| inner.suggestions.get(&normalized(word)))
            .cloned()
            .unwrap_or_default()
    }

    fn learn(&self, word: &str) {
        self.inner.borrow_mut().learned.insert(normalized(word));
    }

    fn unlearn(&self, word: &str) {
        self.inner.borrow_mut().learned.remove(&normalized(word));
    }

    fn has_learned(&self, word: &str) -> bool {
        self.inner.borrow().learned.contains(&normalized(word))
    }

    fn open_document(&self) -> SpellDocument {
        let mut inner = self.inner.borrow_mut();
        let document = SpellDocument(inner.next_tag);
        inner.next_tag += 1;
        inner.open.insert(document);
        document
    }

    fn close_document(&self, document: SpellDocument) {
        let mut inner = self.inner.borrow_mut();
        inner.open.remove(&document);
        inner.ignored.remove(&document);
    }

    fn ignore(&self, word: &str, document: SpellDocument) {
        if document == SpellDocument::NONE {
            return;
        }
        self.inner
            .borrow_mut()
            .ignored
            .entry(document)
            .or_default()
            .insert(normalized(word));
    }

    fn ignored_words(&self, document: SpellDocument) -> Vec<String> {
        self.inner
            .borrow()
            .ignored
            .get(&document)
            .map(|words| words.iter().cloned().collect())
            .unwrap_or_default()
    }

    fn languages(&self) -> Vec<LanguageInfo> {
        self.inner
            .borrow()
            .languages
            .iter()
            .map(|l| l.info.clone())
            .collect()
    }

    fn preferred_languages(&self) -> Vec<String> {
        let inner = self.inner.borrow();
        if inner.preferred.is_empty() {
            inner
                .languages
                .first()
                .map(|l| l.info.id.clone())
                .into_iter()
                .collect()
        } else {
            inner
                .preferred
                .iter()
                .filter(|id| inner.language(id).is_some())
                .cloned()
                .collect()
        }
    }

    fn warm_up(&self) {
        self.inner.borrow_mut().warm_ups += 1;
    }
}
