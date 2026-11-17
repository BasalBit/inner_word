//! The system spell service against the Mac's real checker.
//!
//! Every test here is ignored by default: the answers depend on the macOS
//! version and on the user's own dictionary and languages. Run them with
//!
//! ```text
//! cargo test -p innerword-spell -- --ignored --nocapture --test-threads=1 system_
//! ```
//!
//! They print what the checker said, so a run is also a probe. They never
//! learn a real word (one invented word is learned and unlearned again in
//! the same test).

use std::sync::Mutex;
use std::time::Instant;

use innerword_spell::{
    SpellDocument, SpellLanguage, SpellService, SystemSpellService, language_choices,
};

/// One test at a time talks to the shared checker.
static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn flagged(service: &SystemSpellService, text: &str, language: &SpellLanguage) -> Vec<String> {
    let check = service.check(text, language, SpellDocument::NONE);
    for range in &check.misspelled {
        assert!(
            text.is_char_boundary(range.start) && text.is_char_boundary(range.end),
            "{range:?} in {text:?}"
        );
    }
    check
        .misspelled
        .iter()
        .map(|r| text[r.clone()].to_string())
        .collect()
}

#[test]
#[ignore = "calls the real system spell checker"]
fn system_languages() {
    let _guard = serial();
    let service = SystemSpellService::new();
    let languages = service.languages();
    let preferred = service.preferred_languages();
    println!("available ({}):", languages.len());
    for language in &languages {
        println!("  {} = {}", language.id, language.name);
    }
    println!("preferred: {preferred:?}");
    println!(
        "identifies languages automatically: {}",
        service.identifies_languages_automatically()
    );
    let choices = language_choices(&service);
    println!(
        "choices start with: {:?}",
        choices
            .iter()
            .take(6)
            .map(|l| l.name.as_str())
            .collect::<Vec<_>>()
    );
    assert!(!languages.is_empty());
    assert!(languages.iter().any(|l| l.id.starts_with("en")));
    assert!(languages.iter().all(|l| !l.name.is_empty()));
    assert_eq!(choices.len(), languages.len());
    // The user's first language leads; the whole list only when it is
    // short enough to be a choice (see `language_choices`).
    if let Some(first) = preferred
        .iter()
        .find(|id| languages.iter().any(|l| &l.id == *id))
    {
        assert_eq!(&choices[0].id, first);
    }
    assert!(
        languages
            .iter()
            .any(|l| l.id == "en" && l.name == "English (United States)")
    );
}

#[test]
#[ignore = "calls the real system spell checker"]
fn system_english() {
    let _guard = serial();
    let service = SystemSpellService::new();
    let text = "Ths is a sentense with teh mispelled words, a URL https://example.com, and a name like Velmorin.";
    let english = SpellLanguage::fixed("en");
    let started = Instant::now();
    let check = service.check(text, &english, SpellDocument::NONE);
    let first = started.elapsed();
    let words: Vec<&str> = check.misspelled.iter().map(|r| &text[r.clone()]).collect();
    println!("en: {words:?} (first call {first:?})");
    assert!(words.contains(&"Ths") && words.contains(&"sentense") && words.contains(&"teh"));
    assert!(words.contains(&"mispelled"));
    assert!(
        !words
            .iter()
            .any(|w| w.contains("example") || w.contains("https"))
    );
    for range in &check.misspelled {
        let started = Instant::now();
        let guesses = service.suggestions(text, range.clone(), &english, SpellDocument::NONE);
        println!(
            "  {} -> {:?} ({:?})",
            &text[range.clone()],
            guesses.iter().take(5).collect::<Vec<_>>(),
            started.elapsed()
        );
    }
    let teh = check
        .misspelled
        .iter()
        .find(|r| &text[(*r).clone()] == "teh")
        .unwrap();
    let guesses = service.suggestions(text, teh.clone(), &english, SpellDocument::NONE);
    assert!(guesses.iter().any(|g| g == "the"), "{guesses:?}");
}

#[test]
#[ignore = "calls the real system spell checker"]
fn system_british_and_generic_english() {
    let _guard = serial();
    let service = SystemSpellService::new();
    let text = "The harbour was grey and the colour of the neighbour's centre.";
    let generic = flagged(&service, text, &SpellLanguage::fixed("en"));
    let british = flagged(&service, text, &SpellLanguage::fixed("en_GB"));
    let american = "The harbor was gray and the color of the neighbor's center.";
    let british_on_american = flagged(&service, american, &SpellLanguage::fixed("en_GB"));
    let automatic = service.check(text, &SpellLanguage::Automatic, SpellDocument::NONE);
    println!("generic en on British text:   {generic:?}");
    println!("en_GB on British text:        {british:?}");
    println!("en_GB on American text:       {british_on_american:?}");
    println!(
        "automatic on British text:    {:?} (detected {:?})",
        automatic
            .misspelled
            .iter()
            .map(|r| &text[r.clone()])
            .collect::<Vec<_>>(),
        automatic.language
    );
    assert!(british.is_empty(), "{british:?}");
    assert!(generic.contains(&"harbour".to_string()), "{generic:?}");
}

#[test]
#[ignore = "calls the real system spell checker"]
fn system_other_languages_and_detection() {
    let _guard = serial();
    let service = SystemSpellService::new();
    let samples = [
        (
            "fr",
            "Le batau est arrivé au port ce matin et persone ne l'attendait.",
            vec!["batau", "persone"],
        ),
        (
            "de",
            "Das Schif kam heute Morgen im Hafen an und niemand hat es erwartet.",
            vec!["Schif"],
        ),
        (
            "es",
            "El barco llegó al puerto esta manana y nadie lo esperava.",
            vec!["esperava"],
        ),
        (
            "it",
            "La nave è arivata al porto stamattina e nessuno la aspettava.",
            vec!["arivata"],
        ),
    ];
    for (language, text, expected) in samples {
        let fixed = flagged(&service, text, &SpellLanguage::fixed(language));
        let automatic = service.check(text, &SpellLanguage::Automatic, SpellDocument::NONE);
        let auto_words: Vec<&str> = automatic
            .misspelled
            .iter()
            .map(|r| &text[r.clone()])
            .collect();
        println!(
            "{language}: fixed {fixed:?} | automatic detected {:?} -> {auto_words:?}",
            automatic.language
        );
        for word in &expected {
            assert!(
                fixed.iter().any(|w| w == word),
                "{language}: {word} not in {fixed:?}"
            );
            assert!(
                auto_words.contains(word),
                "{language}: {word} not in {auto_words:?}"
            );
        }
        assert_eq!(automatic.language.as_deref(), Some(language));
        // Guessing in the detected language.
        let range = automatic.misspelled[0].clone();
        let guesses = service.suggestions(
            text,
            range.clone(),
            &SpellLanguage::Automatic,
            SpellDocument::NONE,
        );
        println!(
            "   {} -> {:?}",
            &text[range],
            guesses.iter().take(5).collect::<Vec<_>>()
        );
    }
}

#[test]
#[ignore = "calls the real system spell checker"]
fn system_mixed_paragraph() {
    let _guard = serial();
    let service = SystemSpellService::new();
    let text = "The ship came in this morning. Das Schiff kam heute Morgen im Hafen an. Le bateau est arrivé ce matin. Nobdy expected it, und niemand hat es erwatet.";
    let automatic = service.check(text, &SpellLanguage::Automatic, SpellDocument::NONE);
    println!(
        "mixed, automatic: detected {:?}, flagged {:?}",
        automatic.language,
        automatic
            .misspelled
            .iter()
            .map(|r| &text[r.clone()])
            .collect::<Vec<_>>()
    );
    for language in ["en", "de", "fr"] {
        println!(
            "mixed, {language}: {:?}",
            flagged(&service, text, &SpellLanguage::fixed(language))
        );
    }
    // A language the Mac has no dictionary for (Albanian).
    let albanian = "Anija mbërriti në port këtë mëngjes dhe askush nuk e priste atë.";
    let check = service.check(albanian, &SpellLanguage::Automatic, SpellDocument::NONE);
    println!(
        "albanian, automatic: detected {:?}, flagged {:?}",
        check.language,
        check
            .misspelled
            .iter()
            .map(|r| &albanian[r.clone()])
            .collect::<Vec<_>>()
    );
    println!(
        "albanian, fixed sq: {:?}",
        flagged(&service, albanian, &SpellLanguage::fixed("sq"))
    );
    println!(
        "albanian, fixed en_GB: {} flagged",
        flagged(&service, albanian, &SpellLanguage::fixed("en_GB")).len()
    );
    // A short text: what does detection do with two words?
    for short in ["Ths sentense", "Chaptr One", "Kapitel Eins", "Bonjour", "x"] {
        let check = service.check(short, &SpellLanguage::Automatic, SpellDocument::NONE);
        println!(
            "short {short:?}: detected {:?}, flagged {:?}",
            check.language,
            check
                .misspelled
                .iter()
                .map(|r| &short[r.clone()])
                .collect::<Vec<_>>()
        );
    }
}

#[test]
#[ignore = "calls the real system spell checker"]
fn system_offsets_around_emoji_and_combining_characters() {
    let _guard = serial();
    let service = SystemSpellService::new();
    let english = SpellLanguage::fixed("en");
    let text = "😀 A mispeled 👨‍👩‍👧 wrod 🇦🇱 after the cafe\u{301} and a nai\u{308}ve 𝒳 sentense é.";
    for language in [english.clone(), SpellLanguage::Automatic] {
        let words = flagged(&service, text, &language);
        println!("{language:?}: {words:?}");
        for word in ["mispeled", "wrod", "sentense"] {
            assert!(words.iter().any(|w| w == word), "{word} not in {words:?}");
        }
        // Suggestions take a byte range back to UTF-16.
        let check = service.check(text, &language, SpellDocument::NONE);
        let wrod = check
            .misspelled
            .iter()
            .find(|r| &text[(*r).clone()] == "wrod")
            .unwrap();
        let guesses = service.suggestions(text, wrod.clone(), &language, SpellDocument::NONE);
        println!("  wrod -> {:?}", guesses.iter().take(5).collect::<Vec<_>>());
        assert!(guesses.iter().any(|g| g == "word"), "{guesses:?}");
    }
    // Text that is nothing but symbols, and empty text.
    assert!(flagged(&service, "😀 👨‍👩‍👧 🇦🇱", &english).is_empty());
    assert!(flagged(&service, "", &english).is_empty());
    assert!(flagged(&service, "   \n ", &SpellLanguage::Automatic).is_empty());
}

#[test]
#[ignore = "calls the real system spell checker"]
fn system_ignore_and_learn_an_invented_word() {
    let _guard = serial();
    let service = SystemSpellService::new();
    let english = SpellLanguage::fixed("en");
    // Invented, so nothing real is taught to the user's dictionary; it is
    // unlearned again below whatever happens in between.
    let word = "zxqvorblatt";
    let text = format!("A {word} here and a {word} there, with a qqzzblorf.");
    let a = service.open_document();
    let b = service.open_document();
    println!("document tags: {a:?} {b:?}");
    assert_ne!(a, b);
    assert_ne!(a, SpellDocument::NONE);
    let count = |document| {
        service
            .check(&text, &english, document)
            .misspelled
            .iter()
            .map(|r| text[r.clone()].to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(count(a), [word, word, "qqzzblorf"]);

    service.ignore(word, a);
    println!("ignored in a: {:?}", service.ignored_words(a));
    assert_eq!(count(a), ["qqzzblorf"], "ignored in the document");
    assert_eq!(
        count(b),
        [word, word, "qqzzblorf"],
        "not in another document"
    );
    assert_eq!(count(SpellDocument::NONE).len(), 3);
    // The same through automatic detection.
    let automatic = service.check(&text, &SpellLanguage::Automatic, a);
    println!(
        "automatic with the ignored word: {:?}",
        automatic
            .misspelled
            .iter()
            .map(|r| &text[r.clone()])
            .collect::<Vec<_>>()
    );
    service.close_document(a);
    println!("after close, ignored in a: {:?}", service.ignored_words(a));
    assert_eq!(count(a).len(), 3, "closing forgets the ignored words");

    let was_learned = service.has_learned(word);
    assert!(
        !was_learned,
        "{word} is in the user's dictionary already; not touching it"
    );
    service.learn(word);
    let learned = service.has_learned(word);
    let after_learning = count(b);
    service.unlearn(word);
    let unlearned = !service.has_learned(word);
    let after_unlearning = count(b);
    service.close_document(b);
    println!(
        "learned: {learned}, flagged then: {after_learning:?}; unlearned: {unlearned}, flagged then: {after_unlearning:?}"
    );
    assert!(learned);
    assert_eq!(after_learning, ["qqzzblorf"]);
    assert!(unlearned);
    assert_eq!(after_unlearning.len(), 3);
}

#[test]
#[ignore = "calls the real system spell checker"]
fn system_timing_of_ten_thousand_words() {
    let _guard = serial();
    let service = SystemSpellService::new();
    service.warm_up();
    // 15 words per sentence, one of them misspelled in every second one.
    // No word that British and American English spell differently: with
    // automatic detection the user's own variant of English is the one
    // that checks.
    let good = "The bridge was a plain stone fact and nobody owed the town anything at all. ";
    let bad = "The bridge was a plain stone fact and nobdy owed the town anything at all. ";
    let paragraph: String = (0..14)
        .map(|i| if i % 2 == 0 { good } else { bad })
        .collect();
    let words = paragraph.split_whitespace().count();
    let paragraphs = 10_000 / words + 1;
    for language in [SpellLanguage::fixed("en"), SpellLanguage::Automatic] {
        let started = Instant::now();
        let mut flagged = 0;
        let mut slowest = std::time::Duration::ZERO;
        for _ in 0..paragraphs {
            let one = Instant::now();
            flagged += service
                .check(&paragraph, &language, SpellDocument::NONE)
                .misspelled
                .len();
            slowest = slowest.max(one.elapsed());
        }
        let total = started.elapsed();
        println!(
            "{language:?}: {} words in {paragraphs} paragraphs of {words}: {total:?} total, {:?} per paragraph, slowest {slowest:?}, {flagged} flagged",
            paragraphs * words,
            total / paragraphs as u32,
        );
        assert_eq!(flagged, paragraphs * 7);
    }
    // One text of 10,000 words in one call.
    let book: String = std::iter::repeat_n(paragraph.as_str(), paragraphs).collect();
    for language in [SpellLanguage::fixed("en"), SpellLanguage::Automatic] {
        let started = Instant::now();
        let flagged = service
            .check(&book, &language, SpellDocument::NONE)
            .misspelled
            .len();
        println!(
            "{language:?}: one text of {} words in one call: {:?}, {flagged} flagged",
            paragraphs * words,
            started.elapsed()
        );
    }
    // A clean paragraph (nothing flagged): the common case.
    let clean: String = std::iter::repeat_n(good, 14).collect();
    for language in [SpellLanguage::fixed("en"), SpellLanguage::Automatic] {
        let started = Instant::now();
        for _ in 0..200 {
            assert!(
                service
                    .check(&clean, &language, SpellDocument::NONE)
                    .misspelled
                    .is_empty()
            );
        }
        println!(
            "{language:?}: a clean paragraph of {words} words: {:?}",
            started.elapsed() / 200
        );
    }
}

#[test]
#[ignore = "calls the real system spell checker"]
fn system_bounded_check_of_a_text_in_another_language() {
    let _guard = serial();
    let service = SystemSpellService::new();
    let english = SpellLanguage::fixed("en");
    let german = "Das Schiff kam heute Morgen im Hafen an und niemand hat es erwartet, weil der Hafen leer war. ".repeat(12);
    let words = german.split_whitespace().count();
    let started = Instant::now();
    let whole = service.check(&german, &english, SpellDocument::NONE);
    let whole_time = started.elapsed();
    println!(
        "{words} German words checked as English in one go: {} flagged in {whole_time:?}",
        whole.misspelled.len()
    );

    // The same in parts of at most 16 misspelled words.
    let mut at = 0;
    let mut found = Vec::new();
    let mut slowest = std::time::Duration::ZERO;
    let mut calls = 0;
    while at < german.len() {
        let one = Instant::now();
        let part = service.check_up_to(&german[at..], 16, &english, SpellDocument::NONE);
        slowest = slowest.max(one.elapsed());
        calls += 1;
        assert!(part.check.misspelled.len() <= 16);
        assert!(part.covered > 0 && german.is_char_boundary(at + part.covered));
        found.extend(
            part.check
                .misspelled
                .iter()
                .map(|r| at + r.start..at + r.end),
        );
        at += part.covered;
    }
    println!(
        "in {calls} parts of at most 16: {} flagged, slowest part {slowest:?}",
        found.len()
    );
    assert_eq!(found, whole.misspelled);

    // Automatic is one call whatever the text.
    let started = Instant::now();
    let part = service.check_up_to(&german, 16, &SpellLanguage::Automatic, SpellDocument::NONE);
    println!(
        "automatic: detected {:?}, {} flagged, covered {} of {} in {:?}",
        part.check.language,
        part.check.misspelled.len(),
        part.covered,
        german.len(),
        started.elapsed()
    );
    assert_eq!(part.covered, german.len());
}

/// Run alone (`-- --ignored --nocapture system_first_call`): the first
/// call of a process is the one that starts things.
#[test]
#[ignore = "calls the real system spell checker"]
fn system_first_call() {
    let _guard = serial();
    let service = SystemSpellService::new();
    let started = Instant::now();
    let tag = service.open_document();
    println!(
        "the first document tag (a class method): {:?}",
        started.elapsed()
    );
    service.close_document(tag);
    println!(
        "... and closing it (the shared checker comes to be): {:?}",
        started.elapsed()
    );
    let preferred = service.preferred_languages();
    println!("... preferred languages: {:?}", started.elapsed());
    let first = SpellLanguage::fixed(preferred.first().cloned().unwrap_or_else(|| "en".into()));
    service.check("Teh", &first, SpellDocument::NONE);
    println!("... the first check: {:?}", started.elapsed());
    service.suggestions("Teh", 0..3, &first, SpellDocument::NONE);
    println!("... the first suggestions: {:?}", started.elapsed());
    let again = Instant::now();
    service.warm_up();
    println!("warm_up after that: {:?}", again.elapsed());
    for language in ["en_GB", "en", "de", "fr", "en"] {
        let language = SpellLanguage::fixed(language);
        let text = "The ship came in this morning and nobdy expected it.";
        let started = Instant::now();
        let check = service.check(text, &language, SpellDocument::NONE);
        let first = started.elapsed();
        let started = Instant::now();
        service.check(text, &language, SpellDocument::NONE);
        println!(
            "{language:?}: first check {first:?} ({} flagged), second {:?}",
            check.misspelled.len(),
            started.elapsed()
        );
    }
    let text = "The ship came in this morning and nobdy expected it.";
    let started = Instant::now();
    service.check(text, &SpellLanguage::Automatic, SpellDocument::NONE);
    println!("automatic: first check {:?}", started.elapsed());
    let started = Instant::now();
    service.languages();
    service.preferred_languages();
    println!("languages: {:?}", started.elapsed());
}

/// What automatic detection costs for texts it can and cannot place.
#[test]
#[ignore = "calls the real system spell checker"]
fn system_cost_of_texts_the_checker_cannot_place() {
    let _guard = serial();
    let service = SystemSpellService::new();
    service.warm_up();
    let texts = [
        "The ship came in this morning and nobody expected it at all.",
        " A teh",
        " A teh wrod ",
        "Ths sentense",
        "Teh",
        "T",
        "Chaptr One",
        "Kapitel Eins",
        "Anija mbërriti në port këtë mëngjes dhe askush nuk e priste atë.",
        "Buy mlik",
        "Yes, she said.",
    ];
    for round in 0..3 {
        for text in texts {
            let started = Instant::now();
            let check = service.check(text, &SpellLanguage::Automatic, SpellDocument::NONE);
            let automatic = started.elapsed();
            let started = Instant::now();
            let fixed = service.check(text, &SpellLanguage::fixed("en_GB"), SpellDocument::NONE);
            let fixed_time = started.elapsed();
            if round != 1 {
                println!(
                    "round {round} {text:?}: automatic {automatic:?} detected {:?} flagged {:?} | en_GB {fixed_time:?} flagged {}",
                    check.language,
                    check
                        .misspelled
                        .iter()
                        .map(|r| &text[r.clone()])
                        .collect::<Vec<_>>(),
                    fixed.misspelled.len()
                );
            }
        }
    }
}

/// The resident memory of this process in kilobytes.
fn resident_kb() -> u64 {
    let output = std::process::Command::new("/bin/ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .expect("ps");
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .unwrap_or(0)
}

/// A long session: many calls must not pile up Objective-C objects.
#[test]
#[ignore = "calls the real system spell checker"]
fn system_memory_stays_flat_over_many_calls() {
    let _guard = serial();
    let service = SystemSpellService::new();
    service.warm_up();
    let paragraph = "The bridge was a plain stone fact and nobdy owed the town anything at all,                      😀 with a cafe\u{301} and a wrod in it. "
        .repeat(4);
    let languages = [SpellLanguage::fixed("en_GB"), SpellLanguage::Automatic];
    let round = |checks: usize, guesses: usize| {
        let document = service.open_document();
        for index in 0..checks {
            let check = service.check(&paragraph, &languages[index % 2], document);
            assert!(!check.misspelled.is_empty());
            if index < guesses {
                let range = check.misspelled[0].clone();
                service.suggestions(&paragraph, range, &languages[0], document);
            }
        }
        service.ignore("zxqvorblatt", document);
        service.ignored_words(document);
        service.languages();
        service.preferred_languages();
        service.close_document(document);
    };
    // Let everything that is allocated once be allocated.
    round(2_000, 50);
    let before = resident_kb();
    let started = Instant::now();
    round(20_000, 200);
    let after = resident_kb();
    println!(
        "20,000 checks and 200 guesses in {:?}: resident memory {before} KB before, {after} KB after ({:+} KB)",
        started.elapsed(),
        after as i64 - before as i64
    );
    // Without the pools and releases every call leaves a string of 500
    // bytes and an array of results behind: tens of megabytes here.
    assert!(after < before + 8 * 1024, "{before} KB -> {after} KB");
}
