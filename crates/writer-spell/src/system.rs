//! The Mac's own spell checker (`NSSpellChecker`) as a [`SpellService`].
//!
//! Status: skeleton (the calls into AppKit follow in the next commit).

use std::ops::Range;

use crate::service::{LanguageInfo, SpellCheck, SpellDocument, SpellLanguage, SpellService};

/// The system spell checker. Create it on the main thread
/// ([`SystemSpellService::new`]) and use it there.
pub struct SystemSpellService {
    _not_send: std::marker::PhantomData<*const ()>,
}

impl Default for SystemSpellService {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemSpellService {
    /// The shared system checker.
    pub fn new() -> Self {
        SystemSpellService {
            _not_send: std::marker::PhantomData,
        }
    }
}

impl SpellService for SystemSpellService {
    fn check(
        &self,
        _text: &str,
        _language: &SpellLanguage,
        _document: SpellDocument,
    ) -> SpellCheck {
        SpellCheck::default()
    }

    fn suggestions(
        &self,
        _text: &str,
        _word: Range<usize>,
        _language: &SpellLanguage,
        _document: SpellDocument,
    ) -> Vec<String> {
        Vec::new()
    }

    fn learn(&self, _word: &str) {}

    fn unlearn(&self, _word: &str) {}

    fn has_learned(&self, _word: &str) -> bool {
        false
    }

    fn open_document(&self) -> SpellDocument {
        SpellDocument::NONE
    }

    fn close_document(&self, _document: SpellDocument) {}

    fn ignore(&self, _word: &str, _document: SpellDocument) {}

    fn ignored_words(&self, _document: SpellDocument) -> Vec<String> {
        Vec::new()
    }

    fn languages(&self) -> Vec<LanguageInfo> {
        Vec::new()
    }

    fn preferred_languages(&self) -> Vec<String> {
        Vec::new()
    }
}
