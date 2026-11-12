//! The fake spell service: the rules its documentation states.

use writer_spell::{
    FAKE_LANGUAGE, FakeSpellService, SpellDocument, SpellLanguage, SpellService, language_choices,
};

fn words(text: &str, ranges: &[std::ops::Range<usize>]) -> Vec<String> {
    ranges.iter().map(|r| text[r.clone()].to_string()).collect()
}

#[test]
fn flags_what_is_not_in_the_list() {
    let service =
        FakeSpellService::with_words(["this", "is", "a", "with", "words", "and", "don't"])
            .suggest("teh", ["the", "ten"]);
    let text = "Ths is a sentense with teh words, https://exampel.com and don't, R2D2 x9.";
    let fixed = SpellLanguage::fixed(FAKE_LANGUAGE);
    let check = service.check(text, &fixed, SpellDocument::NONE);
    assert_eq!(words(text, &check.misspelled), ["Ths", "sentense", "teh"]);
    assert_eq!(check.language, None, "a fixed language is not a detection");
    assert_eq!(service.check_count(), 1);
    assert_eq!(service.checked_texts(), [text]);

    let teh = check.misspelled[2].clone();
    assert_eq!(
        service.suggestions(text, teh, &fixed, SpellDocument::NONE),
        ["the", "ten"]
    );
    let ths = check.misspelled[0].clone();
    assert!(
        service
            .suggestions(text, ths, &fixed, SpellDocument::NONE)
            .is_empty()
    );
    assert_eq!(service.suggestion_count(), 2);
    service.reset_calls();
    assert_eq!((service.check_count(), service.suggestion_count()), (0, 0));
}

#[test]
fn learn_and_ignore_behave_like_the_real_ones() {
    let service = FakeSpellService::with_words(["the", "is", "here"]);
    let fixed = SpellLanguage::fixed(FAKE_LANGUAGE);
    let text = "Zorblat is here, zorblat the Quux";
    let a = service.open_document();
    let b = service.open_document();
    assert_ne!(a, b);
    assert_eq!(service.check(text, &fixed, a).misspelled.len(), 3);

    service.ignore("Quux", a);
    assert_eq!(
        words(text, &service.check(text, &fixed, a).misspelled),
        ["Zorblat", "zorblat"]
    );
    assert_eq!(
        service.check(text, &fixed, b).misspelled.len(),
        3,
        "ignored in one document only"
    );
    assert_eq!(service.ignored_words(a), ["quux"]);

    service.learn("zorblat");
    assert!(service.has_learned("Zorblat"));
    assert!(service.check(text, &fixed, a).misspelled.is_empty());
    assert_eq!(
        words(text, &service.check(text, &fixed, b).misspelled),
        ["Quux"]
    );

    service.unlearn("zorblat");
    assert!(!service.has_learned("zorblat"));
    assert_eq!(service.check(text, &fixed, a).misspelled.len(), 2);

    service.close_document(a);
    assert!(service.ignored_words(a).is_empty());
    assert_eq!(service.open_documents(), [b]);
}

#[test]
fn automatic_picks_the_language_of_most_words() {
    let service = FakeSpellService::with_words(["the", "ship", "came", "in"])
        .language("de", "German", ["das", "schiff", "kam", "an"])
        .language(
            "en_GB",
            "British English",
            ["the", "ship", "came", "in", "harbour"],
        )
        .prefer(["en_GB", "en"]);
    let auto = SpellLanguage::Automatic;
    let german = "Das Schif kam an";
    let check = service.check(german, &auto, SpellDocument::NONE);
    assert_eq!(check.language.as_deref(), Some("de"));
    assert_eq!(words(german, &check.misspelled), ["Schif"]);

    let english = "The ship came in the harbour";
    let check = service.check(english, &auto, SpellDocument::NONE);
    assert_eq!(check.language.as_deref(), Some("en_GB"));
    assert!(check.misspelled.is_empty());
    let generic = service.check(english, &SpellLanguage::fixed("en"), SpellDocument::NONE);
    assert_eq!(words(english, &generic.misspelled), ["harbour"]);
    // A language the fake does not have flags nothing.
    assert!(
        service
            .check(english, &SpellLanguage::fixed("xx"), SpellDocument::NONE)
            .misspelled
            .is_empty()
    );

    let choices: Vec<String> = language_choices(&service)
        .into_iter()
        .map(|l| l.name)
        .collect();
    assert_eq!(choices, ["British English", "English", "German"]);
    assert_eq!(service.preferred_languages(), ["en_GB", "en"]);
}

#[test]
fn ranges_are_on_character_boundaries() {
    let service = FakeSpellService::with_words(["café", "naïve", "a"]);
    let fixed = SpellLanguage::fixed(FAKE_LANGUAGE);
    let text = "a 😀 cafe\u{301} naïve cafè 👨‍👩‍👧 e\u{301}te\u{301}";
    let check = service.check(text, &fixed, SpellDocument::NONE);
    for range in &check.misspelled {
        assert!(text.is_char_boundary(range.start) && text.is_char_boundary(range.end));
    }
    assert_eq!(
        words(text, &check.misspelled),
        ["cafe\u{301}", "cafè", "e\u{301}te\u{301}"]
    );
}

#[test]
fn setting_strings() {
    assert_eq!(
        SpellLanguage::from_setting("auto"),
        SpellLanguage::Automatic
    );
    assert_eq!(
        SpellLanguage::from_setting(" Automatic "),
        SpellLanguage::Automatic
    );
    assert_eq!(SpellLanguage::from_setting(""), SpellLanguage::Automatic);
    assert_eq!(
        SpellLanguage::from_setting("en_GB"),
        SpellLanguage::fixed("en_GB")
    );
    assert_eq!(SpellLanguage::fixed("de").as_setting(), "de");
    assert_eq!(SpellLanguage::Automatic.as_setting(), "auto");
    assert_eq!(SpellLanguage::fixed("de").identifier(), Some("de"));
    assert!(SpellLanguage::default().is_automatic());
}
