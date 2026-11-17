//! The Mac's own spell checker (`NSSpellChecker`) as a [`SpellService`].
//!
//! Nothing is bundled: the languages are the ones the Mac offers, and the
//! learned words are the user's own, shared with every other app.
//!
//! # Threads
//!
//! Every call is made on the thread that calls the service, and the app
//! calls it on the main thread only. Neither Apple's documentation of
//! `NSSpellChecker` nor `NSSpellChecker.h` says that the synchronous
//! calls used here (`checkSpellingOfString:…`, `checkString:range:types:…`,
//! `guessesForWordRange:…`, `learnWord:`, `ignoreWord:…`) may be made
//! from another thread. The one sanctioned background path is
//! `requestCheckingOfString:…completionHandler:`, whose handler runs "in
//! an arbitrary context"; the Threading Programming Guide's thread-safety
//! summary does not list the class at all. So the checker
//! ([`crate::SpellChecker`]) stays on the main thread and works in small
//! time-boxed slices instead (a paragraph costs a fraction of a
//! millisecond). [`SystemSpellService`] is not `Send`.
//!
//! # Offsets
//!
//! `NSString` ranges count UTF-16 code units; the service takes and
//! returns UTF-8 byte offsets. The conversion walks the text once per
//! call (`Utf16Map`) and never lands inside a character.
//!
//! # Memory
//!
//! Every call runs inside its own autorelease pool, and the strings it
//! creates are released when it returns: a long session does not
//! accumulate Objective-C objects.
//!
//! # Which call checks
//!
//! - A fixed language: `checkSpellingOfString:startingAt:language:wrap:
//!   inSpellDocumentWithTag:wordCount:`, once per misspelled word.
//!   A text in another language costs a call per word (about 0.1 ms
//!   each), so [`SpellService::check_up_to`] stops after a given number
//!   and says how far it came.
//! - Automatic: `checkString:range:types:options:inSpellDocumentWithTag:
//!   orthography:wordCount:` with the spelling and orthography types: the
//!   checker identifies the language of the text and reports it as the
//!   dominant language of the orthography (nothing, when it could not
//!   tell: a line of two words, a language without a dictionary). One
//!   call, about 3 µs per word.
//!
//! The unified call is not used for a fixed language: given an
//! orthography in its options it still matches other languages (German
//! words are not flagged in a text checked "as English", and plain `en`
//! becomes the user's own variant of English), measured with the tests in
//! `tests/suite/system.rs`.

use std::ops::Range;

use crate::service::{
    LanguageInfo, SpellCheck, SpellCheckPart, SpellDocument, SpellLanguage, SpellService,
};

/// Converts UTF-16 offsets of a text into UTF-8 byte offsets, for
/// offsets asked in rising order in one walk over the text (an offset
/// that goes back starts the walk again).
pub(crate) struct Utf16Map<'a> {
    text: &'a str,
    /// Byte offset the walk has reached.
    byte: usize,
    /// The same place in UTF-16 code units.
    unit: usize,
}

impl<'a> Utf16Map<'a> {
    pub(crate) fn new(text: &'a str) -> Self {
        Utf16Map {
            text,
            byte: 0,
            unit: 0,
        }
    }

    /// The byte offset of the UTF-16 offset `unit`. An offset inside a
    /// surrogate pair goes to the start of that character (`round_up`:
    /// to its end); one past the end goes to the end.
    pub(crate) fn byte_offset(&mut self, unit: usize, round_up: bool) -> usize {
        if unit < self.unit {
            self.byte = 0;
            self.unit = 0;
        }
        for c in self.text[self.byte..].chars() {
            if self.unit >= unit {
                break;
            }
            let units = c.len_utf16();
            if self.unit + units > unit {
                // `unit` is the second half of a surrogate pair.
                if round_up {
                    return self.byte + c.len_utf8();
                }
                return self.byte;
            }
            self.unit += units;
            self.byte += c.len_utf8();
        }
        self.byte
    }

    /// The byte range of the UTF-16 range `location .. location + length`,
    /// widened to whole characters.
    pub(crate) fn byte_range(&mut self, location: usize, length: usize) -> Range<usize> {
        let start = self.byte_offset(location, false);
        let end = self.byte_offset(location.saturating_add(length), true);
        start..end.max(start)
    }
}

/// The length of `text` in UTF-16 code units.
pub(crate) fn utf16_len(text: &str) -> usize {
    text.chars().map(char::len_utf16).sum()
}

/// The UTF-16 range of the byte range `range` of `text`.
pub(crate) fn utf16_range(text: &str, range: Range<usize>) -> Range<usize> {
    let start = utf16_len(&text[..range.start]);
    start..start + utf16_len(&text[range])
}

/// The name a language is listed under. The system's names are kept, with
/// one exception: the checker's plain `en` is its American dictionary
/// ("harbour" is misspelled under it), and the system calls it just
/// "English" in a list with nine entries like "English (United Kingdom)".
/// Nobody looking for American English would find it under that name.
pub fn display_name(id: &str, system_name: String) -> String {
    if id == "en" && system_name == "English" {
        "English (United States)".to_string()
    } else {
        system_name
    }
}

/// The system spell checker. Create it on the main thread and use it
/// there (see the [module docs](self)).
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

    /// Whether the user has "identify languages automatically" switched
    /// on for spelling (System Settings, Keyboard, Text Input). Only
    /// read, never changed. `false` where there is no system checker.
    pub fn identifies_languages_automatically(&self) -> bool {
        imp::identifies_languages()
    }
}

impl SpellService for SystemSpellService {
    fn check(&self, text: &str, language: &SpellLanguage, document: SpellDocument) -> SpellCheck {
        if text.trim().is_empty() {
            return SpellCheck::default();
        }
        imp::check(text, usize::MAX, language, document).0
    }

    fn check_up_to(
        &self,
        text: &str,
        max_misspelled: usize,
        language: &SpellLanguage,
        document: SpellDocument,
    ) -> SpellCheckPart {
        if text.trim().is_empty() {
            return SpellCheckPart {
                check: SpellCheck::default(),
                covered: text.len(),
            };
        }
        let (check, covered) = imp::check(text, max_misspelled, language, document);
        SpellCheckPart { check, covered }
    }

    fn suggestions(
        &self,
        text: &str,
        word: Range<usize>,
        language: &SpellLanguage,
        document: SpellDocument,
    ) -> Vec<String> {
        if word.start >= word.end
            || word.end > text.len()
            || !text.is_char_boundary(word.start)
            || !text.is_char_boundary(word.end)
        {
            return Vec::new();
        }
        imp::suggestions(text, word, language, document)
    }

    fn learn(&self, word: &str) {
        if !word.trim().is_empty() {
            imp::learn(word);
        }
    }

    fn unlearn(&self, word: &str) {
        if !word.trim().is_empty() {
            imp::unlearn(word);
        }
    }

    fn has_learned(&self, word: &str) -> bool {
        !word.trim().is_empty() && imp::has_learned(word)
    }

    fn open_document(&self) -> SpellDocument {
        imp::open_document()
    }

    fn close_document(&self, document: SpellDocument) {
        if document != SpellDocument::NONE {
            imp::close_document(document);
        }
    }

    fn ignore(&self, word: &str, document: SpellDocument) {
        if document != SpellDocument::NONE && !word.trim().is_empty() {
            imp::ignore(word, document);
        }
    }

    fn ignored_words(&self, document: SpellDocument) -> Vec<String> {
        if document == SpellDocument::NONE {
            return Vec::new();
        }
        imp::ignored_words(document)
    }

    fn languages(&self) -> Vec<LanguageInfo> {
        imp::languages()
    }

    fn preferred_languages(&self) -> Vec<String> {
        imp::preferred_languages()
    }

    fn warm_up(&self) {
        imp::warm_up();
    }
}

#[cfg(target_os = "macos")]
// objc 0.2's msg_send! expands to a `cfg(feature = "cargo-clippy")` check.
#[allow(unexpected_cfgs)]
mod imp {
    use std::ffi::CStr;
    use std::ops::Range;
    use std::os::raw::{c_char, c_void};
    use std::ptr;
    use std::sync::atomic::{AtomicBool, Ordering};

    use objc::rc::{StrongPtr, autoreleasepool};
    use objc::runtime::{BOOL, NO, Object};
    use objc::{Encode, Encoding, class, msg_send, sel, sel_impl};

    use super::{Utf16Map, utf16_len, utf16_range};
    use crate::service::{LanguageInfo, SpellCheck, SpellDocument, SpellLanguage};

    // The checker lives in AppKit; nothing else in this crate links it.
    #[link(name = "AppKit", kind = "framework")]
    unsafe extern "C" {}

    type Id = *mut Object;
    const NIL: Id = ptr::null_mut();

    /// `NSRange`.
    #[repr(C)]
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct NSRange {
        location: usize,
        length: usize,
    }

    // SAFETY: the layout and the type encoding of Foundation's NSRange on
    // 64-bit platforms (two NSUInteger).
    unsafe impl Encode for NSRange {
        fn encode() -> Encoding {
            unsafe { Encoding::from_str("{_NSRange=QQ}") }
        }
    }

    /// `NSNotFound` (`NSIntegerMax`).
    const NS_NOT_FOUND: usize = isize::MAX as usize;
    /// `NSUTF8StringEncoding`.
    const UTF8: usize = 4;
    /// `NSTextCheckingTypeOrthography`.
    const TYPE_ORTHOGRAPHY: u64 = 1 << 0;
    /// `NSTextCheckingTypeSpelling`.
    const TYPE_SPELLING: u64 = 1 << 1;
    /// The dominant language of an orthography when none was identified.
    const UNDETERMINED: &str = "und";
    /// More misspellings than this in one text are not looked for (a
    /// guard against a checker that never stops answering).
    const MAX_RESULTS: usize = 100_000;

    /// An `NSString` with the bytes of `text`, owned by the caller.
    ///
    /// SAFETY of the body: `initWithBytes:length:encoding:` copies the
    /// bytes; `text` is valid UTF-8, so the result is not nil unless the
    /// allocation failed (the callers check).
    fn ns_string(text: &str) -> Option<StrongPtr> {
        unsafe {
            let alloc: Id = msg_send![class!(NSString), alloc];
            let string: Id = msg_send![alloc,
                initWithBytes: text.as_ptr() as *const c_void
                length: text.len()
                encoding: UTF8];
            (!string.is_null()).then(|| StrongPtr::new(string))
        }
    }

    /// The text of an `NSString`.
    ///
    /// SAFETY: `string` is nil or a valid NSString for the duration of the
    /// call; `UTF8String` returns a NUL-terminated buffer that lives as
    /// long as the string (or its autorelease pool), and it is copied at
    /// once.
    unsafe fn rust_string(string: Id) -> Option<String> {
        if string.is_null() {
            return None;
        }
        let utf8: *const c_char = msg_send![string, UTF8String];
        if utf8.is_null() {
            return None;
        }
        Some(
            unsafe { CStr::from_ptr(utf8) }
                .to_string_lossy()
                .into_owned(),
        )
    }

    /// The strings of an `NSArray<NSString *>`.
    ///
    /// SAFETY: `array` is nil or a valid NSArray of NSStrings.
    unsafe fn rust_strings(array: Id) -> Vec<String> {
        if array.is_null() {
            return Vec::new();
        }
        let count: usize = msg_send![array, count];
        let mut strings = Vec::with_capacity(count);
        for index in 0..count {
            let item: Id = msg_send![array, objectAtIndex: index];
            if let Some(string) = unsafe { rust_string(item) } {
                strings.push(string);
            }
        }
        strings
    }

    /// `[NSSpellChecker sharedSpellChecker]`: one per process, never
    /// released.
    fn checker() -> Id {
        // SAFETY: a class method without preconditions.
        unsafe { msg_send![class!(NSSpellChecker), sharedSpellChecker] }
    }

    pub(super) fn identifies_languages() -> bool {
        autoreleasepool(|| {
            let checker = checker();
            if checker.is_null() {
                return false;
            }
            // SAFETY: a property read on the shared checker.
            let on: BOOL = unsafe { msg_send![checker, automaticallyIdentifiesLanguages] };
            on != NO
        })
    }

    /// The misspelled ranges of `string` in the fixed language `language`
    /// (nil: the checker's own current language), as UTF-16 ranges.
    ///
    /// SAFETY: `checker` and `string` are valid objects, `language` is nil
    /// or a valid NSString.
    unsafe fn misspelled_in(
        checker: Id,
        string: Id,
        length: usize,
        language: Id,
        tag: isize,
        max: usize,
    ) -> Vec<NSRange> {
        let mut found = Vec::new();
        let mut start: usize = 0;
        while start < length && found.len() < max {
            let range: NSRange = msg_send![checker,
                checkSpellingOfString: string
                startingAt: start as isize
                language: language
                wrap: NO
                inSpellDocumentWithTag: tag
                wordCount: ptr::null_mut::<isize>()];
            if range.location == NS_NOT_FOUND || range.length == 0 || range.location < start {
                break;
            }
            start = range.location.saturating_add(range.length);
            found.push(range);
        }
        found
    }

    /// The misspelled ranges of `string` with the language identified by
    /// the checker, and the dominant language it found.
    ///
    /// SAFETY: `checker` and `string` are valid objects.
    unsafe fn misspelled_detecting(
        checker: Id,
        string: Id,
        length: usize,
        tag: isize,
    ) -> (Vec<NSRange>, Option<String>) {
        let mut orthography: Id = NIL;
        let results: Id = msg_send![checker,
            checkString: string
            range: NSRange { location: 0, length }
            types: TYPE_SPELLING | TYPE_ORTHOGRAPHY
            options: NIL
            inSpellDocumentWithTag: tag
            orthography: &mut orthography as *mut Id
            wordCount: ptr::null_mut::<isize>()];
        let mut found = Vec::new();
        if !results.is_null() {
            let count: usize = msg_send![results, count];
            for index in 0..count.min(MAX_RESULTS) {
                let result: Id = msg_send![results, objectAtIndex: index];
                let kind: u64 = msg_send![result, resultType];
                if kind != TYPE_SPELLING {
                    continue;
                }
                let range: NSRange = msg_send![result, range];
                if range.location != NS_NOT_FOUND && range.length > 0 {
                    found.push(range);
                }
            }
        }
        let language = if orthography.is_null() {
            None
        } else {
            let dominant: Id = msg_send![orthography, dominantLanguage];
            unsafe { rust_string(dominant) }.filter(|l| !l.is_empty() && l != UNDETERMINED)
        };
        (found, language)
    }

    /// UTF-16 ranges as byte ranges of `text`: in order, on character
    /// boundaries, not overlapping.
    fn byte_ranges(text: &str, mut ranges: Vec<NSRange>) -> Vec<Range<usize>> {
        ranges.sort_by_key(|r| (r.location, r.length));
        let mut map = Utf16Map::new(text);
        let mut out: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
        for range in ranges {
            let bytes = map.byte_range(range.location, range.length);
            if bytes.is_empty() || out.last().is_some_and(|last| bytes.start < last.end) {
                continue;
            }
            out.push(bytes);
        }
        out
    }

    /// Checks `text`; with a fixed language at most `max` misspelled
    /// words are looked for. Returns the result and how many bytes of the
    /// text it covers.
    pub(super) fn check(
        text: &str,
        max: usize,
        language: &SpellLanguage,
        document: SpellDocument,
    ) -> (SpellCheck, usize) {
        autoreleasepool(|| {
            let nothing = (SpellCheck::default(), text.len());
            let checker = checker();
            let Some(string) = ns_string(text) else {
                return nothing;
            };
            if checker.is_null() {
                return nothing;
            }
            let length = utf16_len(text);
            let tag = document.0 as isize;
            match language {
                SpellLanguage::Fixed(id) => {
                    let Some(id) = ns_string(id) else {
                        return nothing;
                    };
                    let max = max.clamp(1, MAX_RESULTS);
                    // SAFETY: valid objects, kept alive by `string` and `id`.
                    let found = unsafe { misspelled_in(checker, *string, length, *id, tag, max) };
                    let stopped = found.len() >= max;
                    let misspelled = byte_ranges(text, found);
                    // Stopped at the limit: what lies behind the last word
                    // found has not been looked at.
                    let covered = match misspelled.last() {
                        Some(last) if stopped => last.end,
                        _ => text.len(),
                    };
                    (
                        SpellCheck {
                            misspelled,
                            language: None,
                        },
                        covered,
                    )
                }
                SpellLanguage::Automatic => {
                    // SAFETY: valid objects, kept alive by `string`.
                    let (found, language) =
                        unsafe { misspelled_detecting(checker, *string, length, tag) };
                    (
                        SpellCheck {
                            misspelled: byte_ranges(text, found),
                            language,
                        },
                        text.len(),
                    )
                }
            }
        })
    }

    /// The identifier the checker should guess in for a language it
    /// identified: the user's own variant of it when they prefer one
    /// (`en` becomes `en_GB` for someone who writes British English).
    ///
    /// SAFETY: `checker` is the shared checker.
    unsafe fn preferred_variant(checker: Id, detected: &str) -> String {
        let preferred: Id = msg_send![checker, userPreferredLanguages];
        let preferred = unsafe { rust_strings(preferred) };
        let base = detected.split(['_', '-']).next().unwrap_or(detected);
        preferred
            .iter()
            .find(|id| id.as_str() == detected)
            .or_else(|| {
                preferred
                    .iter()
                    .find(|id| id.split(['_', '-']).next() == Some(base))
            })
            .cloned()
            .unwrap_or_else(|| detected.to_string())
    }

    pub(super) fn suggestions(
        text: &str,
        word: Range<usize>,
        language: &SpellLanguage,
        document: SpellDocument,
    ) -> Vec<String> {
        autoreleasepool(|| {
            let checker = checker();
            let Some(string) = ns_string(text) else {
                return Vec::new();
            };
            if checker.is_null() {
                return Vec::new();
            }
            let tag = document.0 as isize;
            let language = match language {
                SpellLanguage::Fixed(id) => Some(id.clone()),
                SpellLanguage::Automatic => {
                    // SAFETY: valid objects, kept alive by `string`.
                    let (_, detected) =
                        unsafe { misspelled_detecting(checker, *string, utf16_len(text), tag) };
                    // SAFETY: the shared checker.
                    detected.map(|detected| unsafe { preferred_variant(checker, &detected) })
                }
            };
            let language = language.as_deref().and_then(ns_string);
            let language_id: Id = language.as_ref().map_or(NIL, |l| **l);
            let range = utf16_range(text, word);
            // SAFETY: valid objects; the range lies inside the string.
            unsafe {
                let guesses: Id = msg_send![checker,
                    guessesForWordRange: NSRange { location: range.start, length: range.len() }
                    inString: *string
                    language: language_id
                    inSpellDocumentWithTag: tag];
                rust_strings(guesses)
            }
        })
    }

    pub(super) fn learn(word: &str) {
        autoreleasepool(|| {
            let (checker, Some(word)) = (checker(), ns_string(word)) else {
                return;
            };
            if !checker.is_null() {
                // SAFETY: valid objects.
                let _: () = unsafe { msg_send![checker, learnWord: *word] };
            }
        })
    }

    pub(super) fn unlearn(word: &str) {
        autoreleasepool(|| {
            let (checker, Some(word)) = (checker(), ns_string(word)) else {
                return;
            };
            if !checker.is_null() {
                // SAFETY: valid objects.
                let _: () = unsafe { msg_send![checker, unlearnWord: *word] };
            }
        })
    }

    pub(super) fn has_learned(word: &str) -> bool {
        autoreleasepool(|| {
            let (checker, Some(word)) = (checker(), ns_string(word)) else {
                return false;
            };
            if checker.is_null() {
                return false;
            }
            // SAFETY: valid objects.
            let learned: BOOL = unsafe { msg_send![checker, hasLearnedWord: *word] };
            learned != NO
        })
    }

    pub(super) fn open_document() -> SpellDocument {
        // SAFETY: a class method without preconditions.
        let tag: isize = unsafe { msg_send![class!(NSSpellChecker), uniqueSpellDocumentTag] };
        SpellDocument(tag as i64)
    }

    pub(super) fn close_document(document: SpellDocument) {
        autoreleasepool(|| {
            let checker = checker();
            if !checker.is_null() {
                // SAFETY: the shared checker; any tag is accepted.
                let _: () =
                    unsafe { msg_send![checker, closeSpellDocumentWithTag: document.0 as isize] };
            }
        })
    }

    pub(super) fn ignore(word: &str, document: SpellDocument) {
        autoreleasepool(|| {
            let (checker, Some(word)) = (checker(), ns_string(word)) else {
                return;
            };
            if !checker.is_null() {
                // SAFETY: valid objects.
                let _: () = unsafe {
                    msg_send![checker, ignoreWord: *word inSpellDocumentWithTag: document.0 as isize]
                };
            }
        })
    }

    pub(super) fn ignored_words(document: SpellDocument) -> Vec<String> {
        autoreleasepool(|| {
            let checker = checker();
            if checker.is_null() {
                return Vec::new();
            }
            // SAFETY: the shared checker; the result is nil or an array of
            // strings.
            unsafe {
                let words: Id =
                    msg_send![checker, ignoredWordsInSpellDocumentWithTag: document.0 as isize];
                rust_strings(words)
            }
        })
    }

    pub(super) fn languages() -> Vec<LanguageInfo> {
        autoreleasepool(|| {
            let checker = checker();
            if checker.is_null() {
                return Vec::new();
            }
            // SAFETY: plain Foundation and AppKit calls; every object is
            // used inside this pool only.
            unsafe {
                let available: Id = msg_send![checker, availableLanguages];
                let ids = rust_strings(available);
                // The interface is in English, so are the names.
                let english = ns_string("en");
                let locale: Id = match &english {
                    Some(english) => {
                        msg_send![class!(NSLocale), localeWithLocaleIdentifier: **english]
                    }
                    None => NIL,
                };
                ids.into_iter()
                    .map(|id| {
                        let name = ns_string(&id)
                            .filter(|_| !locale.is_null())
                            .and_then(|ns_id| {
                                let name: Id =
                                    msg_send![locale, localizedStringForLocaleIdentifier: *ns_id];
                                rust_string(name)
                            })
                            .filter(|name| !name.trim().is_empty())
                            .unwrap_or_else(|| id.clone());
                        let name = super::display_name(&id, name);
                        LanguageInfo { id, name }
                    })
                    .collect()
            }
        })
    }

    pub(super) fn preferred_languages() -> Vec<String> {
        autoreleasepool(|| {
            let checker = checker();
            if checker.is_null() {
                return Vec::new();
            }
            // SAFETY: a property read; nil or an array of strings.
            unsafe {
                let preferred: Id = msg_send![checker, userPreferredLanguages];
                rust_strings(preferred)
            }
        })
    }

    pub(super) fn warm_up() {
        // Once per process: it is the process that is cold.
        static DONE: AtomicBool = AtomicBool::new(false);
        if DONE.swap(true, Ordering::Relaxed) {
            return;
        }
        // A check and a guess start the spell server and load the
        // dictionary of the user's first language (about 10 ms in a
        // running app).
        let text = "Teh";
        let language = preferred_languages()
            .into_iter()
            .next()
            .map(SpellLanguage::Fixed)
            .unwrap_or(SpellLanguage::Automatic);
        check(text, usize::MAX, &language, SpellDocument::NONE);
        suggestions(text, 0..text.len(), &language, SpellDocument::NONE);
        // A sentence in a language the Mac has no dictionary for: to place
        // it the checker consults the dictionaries of all the user's
        // languages, and loads them (30 to 70 ms the first time, which
        // would otherwise be paid under the first short line somebody
        // types).
        check(
            "Anija mb\u{eb}rriti n\u{eb} port k\u{eb}t\u{eb} m\u{eb}ngjes dhe askush nuk e priste.",
            usize::MAX,
            &SpellLanguage::Automatic,
            SpellDocument::NONE,
        );
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    //! No system checker: nothing is ever misspelled.

    use std::ops::Range;

    use crate::service::{LanguageInfo, SpellCheck, SpellDocument, SpellLanguage};

    pub(super) fn identifies_languages() -> bool {
        false
    }
    pub(super) fn check(
        text: &str,
        _: usize,
        _: &SpellLanguage,
        _: SpellDocument,
    ) -> (SpellCheck, usize) {
        (SpellCheck::default(), text.len())
    }
    pub(super) fn suggestions(
        _: &str,
        _: Range<usize>,
        _: &SpellLanguage,
        _: SpellDocument,
    ) -> Vec<String> {
        Vec::new()
    }
    pub(super) fn learn(_: &str) {}
    pub(super) fn unlearn(_: &str) {}
    pub(super) fn has_learned(_: &str) -> bool {
        false
    }
    pub(super) fn open_document() -> SpellDocument {
        SpellDocument::NONE
    }
    pub(super) fn close_document(_: SpellDocument) {}
    pub(super) fn ignore(_: &str, _: SpellDocument) {}
    pub(super) fn ignored_words(_: SpellDocument) -> Vec<String> {
        Vec::new()
    }
    pub(super) fn languages() -> Vec<LanguageInfo> {
        Vec::new()
    }
    pub(super) fn preferred_languages() -> Vec<String> {
        Vec::new()
    }
    pub(super) fn warm_up() {}
}
