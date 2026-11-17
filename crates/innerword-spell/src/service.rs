//! The spell service: what a spell checker can be asked, independent of
//! who answers ([`crate::SystemSpellService`] over the Mac's own checker,
//! [`crate::FakeSpellService`] in tests).

use std::ops::Range;

/// The language a text is checked in.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum SpellLanguage {
    /// The checker detects the language of each paragraph by itself.
    #[default]
    Automatic,
    /// One language, by the identifier the service knows it under
    /// (`"en_GB"`, `"de"`, `"pt_BR"`; see [`SpellService::languages`]).
    Fixed(String),
}

impl SpellLanguage {
    /// How [`SpellLanguage::Automatic`] is written as a setting string.
    pub const AUTOMATIC: &'static str = "auto";

    /// A fixed language.
    pub fn fixed(identifier: impl Into<String>) -> SpellLanguage {
        SpellLanguage::Fixed(identifier.into())
    }

    /// Reads the one-string form settings are stored in: `"auto"`,
    /// `"automatic"` or nothing is [`SpellLanguage::Automatic`], anything
    /// else is the identifier of a fixed language.
    pub fn from_setting(value: &str) -> SpellLanguage {
        let value = value.trim();
        if value.is_empty()
            || value.eq_ignore_ascii_case(Self::AUTOMATIC)
            || value.eq_ignore_ascii_case("automatic")
        {
            SpellLanguage::Automatic
        } else {
            SpellLanguage::Fixed(value.to_string())
        }
    }

    /// The one-string form: `"auto"` or the language identifier.
    pub fn as_setting(&self) -> &str {
        match self {
            SpellLanguage::Automatic => Self::AUTOMATIC,
            SpellLanguage::Fixed(id) => id,
        }
    }

    /// The identifier of the fixed language; `None` for
    /// [`SpellLanguage::Automatic`].
    pub fn identifier(&self) -> Option<&str> {
        match self {
            SpellLanguage::Automatic => None,
            SpellLanguage::Fixed(id) => Some(id),
        }
    }

    /// Whether this is [`SpellLanguage::Automatic`].
    pub fn is_automatic(&self) -> bool {
        matches!(self, SpellLanguage::Automatic)
    }
}

impl From<&innerword_core::SpellingLanguage> for SpellLanguage {
    /// The language a document (or the app's setting) names, as the
    /// service takes it.
    fn from(language: &innerword_core::SpellingLanguage) -> SpellLanguage {
        match language {
            innerword_core::SpellingLanguage::Automatic => SpellLanguage::Automatic,
            innerword_core::SpellingLanguage::Language(id) => SpellLanguage::Fixed(id.clone()),
        }
    }
}

impl From<innerword_core::SpellingLanguage> for SpellLanguage {
    fn from(language: innerword_core::SpellingLanguage) -> SpellLanguage {
        SpellLanguage::from(&language)
    }
}

impl SpellLanguage {
    /// The language in force for a document: its own
    /// (`DocumentSettings::spelling_language`) when it has one, else the
    /// app's setting.
    pub fn effective(
        document: Option<&innerword_core::SpellingLanguage>,
        app: &SpellLanguage,
    ) -> SpellLanguage {
        document.map_or_else(|| app.clone(), SpellLanguage::from)
    }
}

/// A language the service can check: its identifier and the name a person
/// reads.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LanguageInfo {
    /// What the service calls the language (`"en_GB"`).
    pub id: String,
    /// The name for a settings row ("British English"), in the language
    /// of the interface.
    pub name: String,
}

impl LanguageInfo {
    /// A language with its display name.
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> LanguageInfo {
        LanguageInfo {
            id: id.into(),
            name: name.into(),
        }
    }
}

/// One open document, as the service knows it: the words ignored "for
/// this document" belong to it. Get one from
/// [`SpellService::open_document`] and give it back with
/// [`SpellService::close_document`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SpellDocument(pub i64);

impl SpellDocument {
    /// No document: nothing is ignored. What a caller passes when it only
    /// wants to try a word.
    pub const NONE: SpellDocument = SpellDocument(0);
}

/// The answer to [`SpellService::check`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpellCheck {
    /// The misspelled words: byte ranges of the checked text, on
    /// character boundaries, in order, not overlapping.
    pub misspelled: Vec<Range<usize>>,
    /// The language the service took the text to be in, when it was asked
    /// to detect it ([`SpellLanguage::Automatic`]) and could tell.
    pub language: Option<String>,
}

/// The answer to [`SpellService::check_up_to`]: the result for the start
/// of a text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpellCheckPart {
    /// What was found in the first [`SpellCheckPart::covered`] bytes.
    pub check: SpellCheck,
    /// How many bytes of the text the result covers, from its start: the
    /// length of the text when all of it was checked, else the end of the
    /// last misspelled word that was looked for (a character boundary).
    /// Check the rest by passing the text from there on.
    pub covered: usize,
}

/// A spell checker.
///
/// All methods take `&self`: a service is shared (`Rc<dyn SpellService>`)
/// between the checker of the open document and whoever lists the
/// languages. A service is not `Send`: it is used from the main thread
/// only (the system checker is an AppKit object).
///
/// Offsets are UTF-8 byte offsets of the `text` that was passed in, on
/// character boundaries.
pub trait SpellService: 'static {
    /// Checks `text` (one paragraph is the intended unit) and returns its
    /// misspelled words. With [`SpellLanguage::Automatic`] the service
    /// detects the language and reports it. Words that were learned, and
    /// words ignored in `document`, are not reported.
    fn check(&self, text: &str, language: &SpellLanguage, document: SpellDocument) -> SpellCheck;

    /// Like [`SpellService::check`], but allowed to stop after
    /// `max_misspelled` misspelled words (at least one): a service that
    /// pays per misspelled word (the system checker with a fixed
    /// language does) stops there and reports how far it came, so that a
    /// caller with a time budget can go on later. The default checks
    /// everything.
    fn check_up_to(
        &self,
        text: &str,
        max_misspelled: usize,
        language: &SpellLanguage,
        document: SpellDocument,
    ) -> SpellCheckPart {
        let _ = max_misspelled;
        SpellCheckPart {
            check: self.check(text, language, document),
            covered: text.len(),
        }
    }

    /// Replacements for the misspelled word at `word` (a range a
    /// [`SpellService::check`] of the same `text` returned), best first.
    /// Empty when the service has none.
    fn suggestions(
        &self,
        text: &str,
        word: Range<usize>,
        language: &SpellLanguage,
        document: SpellDocument,
    ) -> Vec<String>;

    /// Adds `word` to the user's own dictionary: permanent, and for the
    /// system service shared with every other app on the Mac.
    fn learn(&self, word: &str);

    /// Takes `word` out of the user's own dictionary again.
    fn unlearn(&self, word: &str);

    /// Whether `word` is in the user's own dictionary.
    fn has_learned(&self, word: &str) -> bool;

    /// A new document tag.
    fn open_document(&self) -> SpellDocument;

    /// Forgets `document` and the words ignored in it.
    fn close_document(&self, document: SpellDocument);

    /// Stops reporting `word` in `document` until it is closed.
    fn ignore(&self, word: &str, document: SpellDocument);

    /// The words ignored in `document`.
    fn ignored_words(&self, document: SpellDocument) -> Vec<String>;

    /// Every language the service can check, in the service's own order.
    fn languages(&self) -> Vec<LanguageInfo>;

    /// The identifiers of the languages the user prefers, best first
    /// (each is one of [`SpellService::languages`]).
    fn preferred_languages(&self) -> Vec<String>;

    /// Does whatever makes the first real calls slow, so that it does not
    /// happen under a keystroke or a right-click (the system checker
    /// starts its server and loads dictionaries: 40 to 80 ms in a running
    /// app, once per process). The checker calls it a moment after it was
    /// attached. The default does nothing.
    fn warm_up(&self) {}
}

/// The languages for a settings row or a picker: the user's preferred
/// languages first, in their order, then every other language sorted by
/// its display name.
///
/// A preferred list that names most of what the service offers is not a
/// choice anybody made: a Mac left on "Automatic by Language" reports
/// every language it can tell apart (34 of 44 on the Mac this was written
/// on), the user's own first and the rest in the order of their
/// identifiers. Then only the first entry says something about the user,
/// and the others are sorted by name with the rest ([`OWN_CHOICE_LIMIT`]).
pub fn language_choices(service: &dyn SpellService) -> Vec<LanguageInfo> {
    let all = service.languages();
    let mut preferred = service.preferred_languages();
    if preferred.len() > OWN_CHOICE_LIMIT && preferred.len() * 2 > all.len() {
        preferred.truncate(1);
    }
    let mut first: Vec<LanguageInfo> = Vec::new();
    for id in &preferred {
        if first.iter().any(|l| &l.id == id) {
            continue;
        }
        if let Some(info) = all.iter().find(|l| &l.id == id) {
            first.push(info.clone());
        }
    }
    let mut rest: Vec<LanguageInfo> = all
        .into_iter()
        .filter(|l| !first.iter().any(|f| f.id == l.id))
        .collect();
    rest.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.id.cmp(&b.id))
    });
    first.extend(rest);
    first
}

/// Up to this many preferred languages are always taken as the user's own
/// choice and listed first by [`language_choices`]; a longer list only
/// when it names no more than half of the languages there are.
pub const OWN_CHOICE_LIMIT: usize = 4;

/// The display name of the language `id` ("British English"), or the
/// identifier itself when the service does not know it.
pub fn language_name(service: &dyn SpellService, id: &str) -> String {
    service
        .languages()
        .into_iter()
        .find(|l| l.id == id)
        .map(|l| l.name)
        .unwrap_or_else(|| id.to_string())
}
