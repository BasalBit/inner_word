//! The checker with a real editor and the fake service: what is marked,
//! and when.

use std::rc::Rc;

use gpui::TestAppContext;
use innerword_core::{Block, BlockKind, Document, InlineAttrs, Position};
use innerword_editor::EditorConfig;
use innerword_spell::{
    EDIT_PAUSE, FakeSpellService, IDLE_PAUSE, SpellEvent, SpellLanguage, SpellService, TYPING_PAUSE,
};

use crate::suite::support::*;

#[gpui::test]
fn marks_appear_for_misspellings(cx: &mut TestAppContext) {
    setup(cx);
    let service = Rc::new(english());
    let document = paragraphs(&[
        "Ths is a sentense with teh words.",
        "Nobody expected it.",
        "",
        "A wrod here and a wrod there, https://exampel.com and x9.",
    ]);
    let (checker, editor, cx) = open(document, &service, cx);
    // One turn of the worker is enough for a short document.
    assert_eq!(
        marked(&checker, cx),
        ["Ths", "sentense", "teh", "wrod", "wrod"]
    );
    assert!(checker.read_with(cx, |checker, cx| checker.is_settled(cx)));
    assert_eq!(
        checker.read_with(cx, |checker, cx| checker.pending_blocks(cx)),
        0
    );
    assert_eq!(
        checker.read_with(cx, |checker, cx| checker.misspelling_count(cx)),
        5
    );
    // The marks are the editor's: drawn by it, and answered by it.
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.spelling_marks(0), [0..3, 9..17, 23..26]);
        assert_eq!(editor.spelling_mark_count(), 5);
        assert!(editor.spelling_checked(2));
    });
    // Every block was asked about once (the empty one not at all). The
    // service is warmed up a moment later, once.
    assert_eq!(service.check_count(), 3);
    assert_eq!(service.warm_up_count(), 0);
    wait(innerword_spell::WARM_UP_DELAY, cx);
    wait(innerword_spell::WARM_UP_DELAY, cx);
    assert_eq!(service.warm_up_count(), 1);
    assert_eq!(service.check_count(), 3);
    checker.read_with(cx, |checker, cx| {
        assert_eq!(
            checker
                .misspelling_at(Position::new(0, 10), cx)
                .map(|r| (r.start.offset, r.end.offset)),
            Some((9, 17))
        );
        assert_eq!(checker.misspelling_at(Position::new(1, 3), cx), None);
    });
}

#[gpui::test]
fn the_word_being_typed_is_not_marked_until_the_caret_leaves_or_a_pause(cx: &mut TestAppContext) {
    setup(cx);
    let service = Rc::new(english());
    let (checker, editor, cx) = open(paragraphs(&["The ship came in "]), &service, cx);
    assert!(marked(&checker, cx).is_empty());
    caret(&editor, 0, 17, cx);

    // Typed, and the paragraph is checked again after the short pause:
    // the word under the caret stays unmarked.
    cx.simulate_input("teh");
    assert!(marked(&checker, cx).is_empty());
    wait(EDIT_PAUSE, cx);
    assert!(marked(&checker, cx).is_empty(), "still being typed");
    // About a second after the last key it is marked, without asking the
    // service again.
    let checks = service.check_count();
    wait(TYPING_PAUSE, cx);
    assert_eq!(marked(&checker, cx), ["teh"]);
    assert_eq!(service.check_count(), checks);

    // A space ends the word: marked with the next check.
    cx.simulate_input(" wrod");
    wait(EDIT_PAUSE, cx);
    assert_eq!(marked(&checker, cx), ["teh"], "wrod is being typed");
    cx.simulate_input(" ");
    wait(EDIT_PAUSE, cx);
    assert_eq!(marked(&checker, cx), ["teh", "wrod"]);

    // The caret leaves the word: marked at once.
    cx.simulate_input("sentense");
    wait(EDIT_PAUSE, cx);
    assert_eq!(marked(&checker, cx), ["teh", "wrod"]);
    let checks = service.check_count();
    cx.simulate_keystrokes("left");
    assert_eq!(
        marked(&checker, cx),
        ["teh", "wrod"],
        "one step back is still inside the word"
    );
    cx.simulate_keystrokes("cmd-left");
    assert_eq!(marked(&checker, cx), ["teh", "wrod", "sentense"]);
    assert_eq!(
        service.check_count(),
        checks,
        "from the result that was held back"
    );

    // Leaving before the paragraph was checked again: nothing is held
    // back by the check that follows.
    cx.simulate_keystrokes("cmd-right");
    cx.simulate_input(" mornin");
    cx.simulate_keystrokes("cmd-left");
    wait(EDIT_PAUSE, cx);
    assert_eq!(marked(&checker, cx), ["teh", "wrod", "sentense", "mornin"]);

    // Typing inside a marked word takes its mark away while it is typed
    // (the editor drops it with the edit, the check holds it back).
    caret(&editor, 0, 19, cx);
    cx.simulate_input("x");
    assert_eq!(
        text(&editor, cx),
        "The ship came in texh wrod sentense mornin"
    );
    assert_eq!(marked(&checker, cx), ["wrod", "sentense", "mornin"]);
    wait(EDIT_PAUSE, cx);
    assert_eq!(marked(&checker, cx), ["wrod", "sentense", "mornin"]);
    wait(TYPING_PAUSE, cx);
    assert_eq!(marked(&checker, cx), ["texh", "wrod", "sentense", "mornin"]);
}

#[gpui::test]
fn code_is_not_checked(cx: &mut TestAppContext) {
    setup(cx);
    let service = Rc::new(english());
    let code = InlineAttrs {
        code: true,
        ..InlineAttrs::default()
    };
    let document = Document::from_blocks(vec![
        Block::with_text(BlockKind::code(), "let teh = wrod;\nmore codee"),
        Block::paragraph("This is ")
            .with_span("teh_codee", code.clone())
            .with_span(" and a wrod with ", InlineAttrs::default())
            .with_span("wrod()", code)
            .with_span(" here.", InlineAttrs::default()),
        Block::new(BlockKind::Divider),
        Block::with_text(BlockKind::heading(2), "Teh chapter"),
    ]);
    let (checker, editor, cx) = open(document, &service, cx);
    assert_eq!(marked(&checker, cx), ["wrod", "Teh"]);
    // The code block was never sent; the paragraph went with its inline
    // code blanked out, at the same length.
    let texts = service.checked_texts();
    assert_eq!(texts.len(), 2);
    assert_eq!(texts[0], "This is           and a wrod with        here.");
    assert_eq!(
        texts[0].len(),
        editor.read_with(cx, |editor, _| editor.document().blocks()[1].len())
    );
    assert_eq!(texts[1], "Teh chapter");
    // Skipped blocks count as checked: nothing is left to do.
    assert!(checker.read_with(cx, |checker, cx| checker.is_settled(cx)));
    editor.read_with(cx, |editor, _| {
        assert!(editor.spelling_checked(0) && editor.spelling_checked(2));
        assert_eq!(editor.spelling_marks(1), [24..28]);
    });

    // Turning the code block into a paragraph makes it text.
    caret(&editor, 0, 0, cx);
    cx.simulate_keystrokes("cmd-alt-0");
    wait(EDIT_PAUSE, cx);
    wait(TYPING_PAUSE, cx);
    assert_eq!(
        marked(&checker, cx),
        ["teh", "wrod", "codee", "wrod", "Teh"]
    );
}

#[gpui::test]
fn an_edit_rechecks_the_paragraph_it_touched(cx: &mut TestAppContext) {
    setup(cx);
    let service = Rc::new(english());
    let lines: Vec<String> = (0..40)
        .map(|i| format!("Paragraph {i} with a wrod here."))
        .collect();
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    let (checker, editor, cx) = open(paragraphs(&lines), &service, cx);
    settle(&checker, cx);
    assert_eq!(marked(&checker, cx).len(), 40);
    assert_eq!(service.check_count(), 40);

    service.reset_calls();
    caret(&editor, 7, 0, cx);
    cx.simulate_input("A ");
    assert_eq!(service.check_count(), 0, "not under the keystroke");
    wait(EDIT_PAUSE, cx);
    assert_eq!(service.checked_texts(), ["A Paragraph 7 with a wrod here."]);
    settle(&checker, cx);
    assert_eq!(service.check_count(), 1, "nothing else was asked again");
    assert_eq!(marked(&checker, cx).len(), 40);

    // Several keys in a row: the paragraph is checked once per pause,
    // not once per key.
    service.reset_calls();
    for _ in 0..5 {
        cx.simulate_input("x");
    }
    wait(EDIT_PAUSE, cx);
    assert_eq!(service.check_count(), 1);

    // A new paragraph: the two halves of the split, nothing else.
    settle(&checker, cx);
    service.reset_calls();
    caret(&editor, 3, 11, cx);
    cx.simulate_keystrokes("enter");
    settle(&checker, cx);
    assert_eq!(service.checked_texts().len(), 2);
    assert_eq!(marked(&checker, cx).len(), 41, "xxxxx is one more");

    // Undo brings the old text back: that block again, and no other.
    service.reset_calls();
    cx.simulate_keystrokes("cmd-z");
    settle(&checker, cx);
    assert_eq!(service.check_count(), 1);

    // Moving the caret and scrolling over checked text ask nothing.
    service.reset_calls();
    cx.simulate_keystrokes("cmd-down");
    cx.simulate_keystrokes("cmd-up");
    editor.update(cx, |editor, cx| editor.scroll_by_pages(1.0, cx));
    wait(IDLE_PAUSE, cx);
    settle(&checker, cx);
    assert_eq!(service.check_count(), 0);
}

#[gpui::test]
fn the_language_decides_and_a_change_checks_again(cx: &mut TestAppContext) {
    setup(cx);
    let service = Rc::new(
        FakeSpellService::with_words(["the", "harbor", "was", "quiet"])
            .language(
                "en_GB",
                "British English",
                ["the", "harbour", "was", "quiet"],
            )
            .language("de", "German", ["der", "hafen", "war", "still"]),
    );
    let document = paragraphs(&[
        "The harbour was quiet",
        "The harbor was quiet",
        "Der Hafen war still",
    ]);
    let (checker, _editor, cx) = open(document, &service, cx);
    assert_eq!(
        marked(&checker, cx),
        ["harbour", "Der", "Hafen", "war", "still"]
    );

    let events = Rc::new(std::cell::RefCell::new(Vec::new()));
    let _subscription = cx.update(|_, cx| {
        let events = events.clone();
        cx.subscribe(&checker, move |_, event: &SpellEvent, _| {
            events.borrow_mut().push(event.clone())
        })
    });
    service.reset_calls();
    checker.update(cx, |checker, cx| {
        checker.set_language(SpellLanguage::fixed("en_GB"), cx)
    });
    cx.run_until_parked();
    assert_eq!(
        marked(&checker, cx),
        ["harbor", "Der", "Hafen", "war", "still"]
    );
    assert_eq!(service.check_count(), 3, "every block again");
    assert!(
        events
            .borrow()
            .contains(&SpellEvent::LanguageChanged(SpellLanguage::fixed("en_GB")))
    );
    assert!(events.borrow().contains(&SpellEvent::Settled));

    // The same language again: nothing happens.
    service.reset_calls();
    checker.update(cx, |checker, cx| {
        checker.set_language(SpellLanguage::fixed("en_GB"), cx)
    });
    cx.run_until_parked();
    assert_eq!(service.check_count(), 0);

    checker.update(cx, |checker, cx| {
        checker.set_language(SpellLanguage::fixed("de"), cx)
    });
    cx.run_until_parked();
    assert_eq!(
        marked(&checker, cx),
        [
            "The", "harbour", "was", "quiet", "The", "harbor", "was", "quiet"
        ]
    );

    // Automatic: each paragraph in its own language.
    checker.update(cx, |checker, cx| {
        checker.set_language(SpellLanguage::Automatic, cx)
    });
    cx.run_until_parked();
    assert!(marked(&checker, cx).is_empty());
    checker.read_with(cx, |checker, cx| {
        assert_eq!(checker.detected_language(0, cx).as_deref(), Some("en_GB"));
        assert_eq!(checker.detected_language(1, cx).as_deref(), Some("en"));
        assert_eq!(checker.detected_language(2, cx).as_deref(), Some("de"));
        assert_eq!(checker.language(), &SpellLanguage::Automatic);
    });
}

#[gpui::test]
fn automatic_checks_a_short_line_in_the_language_of_the_text_around_it(cx: &mut TestAppContext) {
    setup(cx);
    let german = [
        "das", "schiff", "kam", "heute", "an", "und", "niemand", "hat", "es", "erwartet",
        "kapitel", "eins",
    ];
    let service = Rc::new(
        FakeSpellService::with_words(WORDS)
            .language("en_GB", "British English", WORDS)
            .language("de", "German", german)
            .prefer(["en_GB", "de", "en"]),
    );
    // The heading has no word either list knows: the service cannot tell
    // its language. It comes first, so it is looked at before any other
    // block has been identified.
    let document = Document::from_blocks(vec![
        Block::with_text(BlockKind::heading(1), "Kapitl Ains"),
        Block::paragraph("Das Schiff kam heute an und niemand hat es erwartet."),
        Block::paragraph("Das Schif kam heute an."),
        Block::paragraph("Zwei"),
    ]);
    let (checker, editor, cx) = open_in(
        document,
        &service,
        SpellLanguage::Automatic,
        EditorConfig::full(),
        cx,
    );
    settle(&checker, cx);
    // In German, the language of the document, not in the user's first
    // language: "Zwei" and the heading are marked as German words that
    // are not in the list, and nothing of the German text is marked as
    // English.
    assert_eq!(marked(&checker, cx), ["Kapitl", "Ains", "Schif", "Zwei"]);
    checker.read_with(cx, |checker, cx| {
        assert_eq!(checker.detected_language(0, cx).as_deref(), Some("de"));
        assert_eq!(checker.detected_language(1, cx).as_deref(), Some("de"));
    });

    // An English document: the user's own variant of English.
    editor.update(cx, |editor, cx| {
        editor.set_document(
            paragraphs(&[
                "Ths sentense",
                "The ship came in this morning and nobody expected it.",
            ]),
            cx,
        )
    });
    cx.run_until_parked();
    settle(&checker, cx);
    assert_eq!(marked(&checker, cx), ["Ths", "sentense"]);
    checker.read_with(cx, |checker, cx| {
        assert_eq!(checker.detected_language(0, cx).as_deref(), Some("en_GB"));
    });

    // Nothing identified anywhere: the user's first language.
    editor.update(cx, |editor, cx| {
        editor.set_document(paragraphs(&["Ths sentense"]), cx)
    });
    cx.run_until_parked();
    settle(&checker, cx);
    assert_eq!(marked(&checker, cx), ["Ths", "sentense"]);
}

#[gpui::test]
fn turned_off_removes_the_marks_and_stops(cx: &mut TestAppContext) {
    setup(cx);
    let service = Rc::new(english());
    let (checker, editor, cx) = open(paragraphs(&["A wrod here.", "Teh ship."]), &service, cx);
    assert_eq!(marked(&checker, cx), ["wrod", "Teh"]);

    checker.update(cx, |checker, cx| checker.set_enabled(false, cx));
    cx.run_until_parked();
    assert!(marked(&checker, cx).is_empty());
    checker.read_with(cx, |checker, cx| {
        assert!(!checker.is_enabled() && !checker.is_active(cx) && !checker.is_settled(cx));
        assert_eq!(checker.misspelling_at(Position::new(0, 3), cx), None);
    });
    // Nothing is asked while it is off, whatever happens in the text.
    service.reset_calls();
    caret(&editor, 0, 12, cx);
    cx.simulate_input(" Mor wrods");
    wait(TYPING_PAUSE, cx);
    wait(TYPING_PAUSE, cx);
    assert_eq!(service.check_count(), 0);
    assert!(marked(&checker, cx).is_empty());
    // The keys say so instead of doing nothing.
    cx.simulate_keystrokes("cmd-;");
    assert_eq!(
        notice(&checker, cx).as_deref(),
        Some("Spell checking is off")
    );

    // On again: everything is checked, as the text is now.
    checker.update(cx, |checker, cx| checker.toggle_enabled(cx));
    cx.run_until_parked();
    wait(TYPING_PAUSE, cx);
    assert_eq!(marked(&checker, cx), ["wrod", "Mor", "wrods", "Teh"]);

    // The toggle action does the same and tells the app.
    let events = Rc::new(std::cell::RefCell::new(Vec::new()));
    let _subscription = cx.update(|_, cx| {
        let events = events.clone();
        cx.subscribe(&checker, move |_, event: &SpellEvent, _| {
            events.borrow_mut().push(event.clone())
        })
    });
    cx.dispatch_action(innerword_spell::actions::ToggleSpellChecking);
    cx.run_until_parked();
    assert!(marked(&checker, cx).is_empty());
    assert!(events.borrow().contains(&SpellEvent::EnabledChanged(false)));
    assert!(events.borrow().contains(&SpellEvent::MarksChanged));
}

#[gpui::test]
fn read_only_and_compact_editors_are_left_alone(cx: &mut TestAppContext) {
    setup(cx);
    let service = Rc::new(english());
    let (checker, editor, cx) = open_in(
        paragraphs(&["A wrod here."]),
        &service,
        SpellLanguage::fixed(innerword_spell::FAKE_LANGUAGE),
        EditorConfig::full().read_only(true),
        cx,
    );
    assert!(marked(&checker, cx).is_empty());
    assert_eq!(service.check_count(), 0);
    assert!(!checker.read_with(cx, |checker, cx| checker.is_active(cx)));

    // Nothing was warmed up for an editor that is left alone.
    wait(innerword_spell::WARM_UP_DELAY, cx);
    assert_eq!(service.warm_up_count(), 0);

    // Writable: checked, and the service is warmed up a moment later.
    editor.update(cx, |editor, cx| editor.set_read_only(false, cx));
    cx.run_until_parked();
    assert_eq!(marked(&checker, cx), ["wrod"]);
    wait(innerword_spell::WARM_UP_DELAY, cx);
    assert_eq!(service.warm_up_count(), 1);
    // Read-only again: the marks go.
    editor.update(cx, |editor, cx| editor.set_read_only(true, cx));
    cx.run_until_parked();
    assert!(marked(&checker, cx).is_empty());

    let (checker, _editor, cx) = open_in(
        paragraphs(&["A wrod here."]),
        &service,
        SpellLanguage::fixed(innerword_spell::FAKE_LANGUAGE),
        EditorConfig::compact(Default::default()),
        cx,
    );
    wait(TYPING_PAUSE, cx);
    assert!(marked(&checker, cx).is_empty());
    assert!(!checker.read_with(cx, |checker, cx| checker.is_active(cx)));
}

#[gpui::test]
fn another_document_starts_clean(cx: &mut TestAppContext) {
    setup(cx);
    let service = Rc::new(english());
    let (checker, editor, cx) = open(paragraphs(&["A wrod here and a zorblat."]), &service, cx);
    assert_eq!(marked(&checker, cx), ["wrod", "zorblat"]);
    let first = checker.read_with(cx, |checker, _| checker.document());
    assert_eq!(service.open_documents(), [first]);
    checker.update(cx, |checker, cx| checker.ignore_word("zorblat", cx));
    assert_eq!(marked(&checker, cx), ["wrod"]);
    assert_eq!(service.ignored_words(first), ["zorblat"]);

    // The app keeps one editor and replaces its document.
    editor.update(cx, |editor, cx| {
        editor.set_document(
            paragraphs(&["Teh zorblat came in.", "Nobody expected it."]),
            cx,
        )
    });
    cx.run_until_parked();
    let second = checker.read_with(cx, |checker, _| checker.document());
    assert_ne!(first, second);
    assert_eq!(
        service.open_documents(),
        [second],
        "the old document was closed"
    );
    assert!(service.ignored_words(first).is_empty());
    assert_eq!(
        marked(&checker, cx),
        ["Teh", "zorblat"],
        "ignored in the other document only"
    );
    assert!(checker.read_with(cx, |checker, cx| checker.is_settled(cx)));

    // A checker that goes takes its document along.
    let dyn_service: Rc<dyn SpellService> = service.clone();
    let extra = cx.update(|_, cx| {
        use gpui::AppContext;
        cx.new(|cx| innerword_spell::SpellChecker::new(editor.clone(), dyn_service, cx))
    });
    let third = extra.read_with(cx, |checker, _| checker.document());
    assert_eq!(service.open_documents(), [second, third]);
    drop(extra);
    cx.update(|_, _| {});
    cx.run_until_parked();
    assert_eq!(service.open_documents(), [second]);
}

#[gpui::test]
fn automatic_does_not_let_a_short_line_pass_as_some_other_language(cx: &mut TestAppContext) {
    setup(cx);
    // As on a Mac with many languages: "teh" is a word in one of them, and
    // a line of three words is too short to be placed.
    let service = Rc::new(
        english()
            .language("id", "Indonesian", ["teh"])
            .place_from(3),
    );
    let document = paragraphs(&[
        "The ship came in this morning and nobody expected it.",
        "A teh wrod",
        "Ths sentense",
    ]);
    let (checker, editor, cx) = open_in(
        document,
        &service,
        SpellLanguage::Automatic,
        EditorConfig::full(),
        cx,
    );
    settle(&checker, cx);
    // Left to itself the service would mark "wrod" alone in the second
    // line and nothing in the third.
    assert_eq!(marked(&checker, cx), ["teh", "wrod", "Ths", "sentense"]);
    checker.read_with(cx, |checker, cx| {
        assert_eq!(checker.detected_language(0, cx).as_deref(), Some("en"));
        assert_eq!(checker.detected_language(1, cx).as_deref(), Some("en"));
    });
    // The suggestions are asked for in that language too.
    assert_eq!(
        checker.update(cx, |checker, cx| checker
            .suggestions_at(Position::new(1, 3), cx)),
        ["the", "ten", "tea"]
    );

    // A document in a language without a list: its paragraphs come back
    // without a language but with marks. Nothing is forced on it: the
    // service's own answer stands, and a line it says nothing about is
    // left alone.
    editor.update(cx, |editor, cx| {
        editor.set_document(
            paragraphs(&[
                "Anija mberriti ne port kete mengjes the.",
                "Askush nuk e priste ate in.",
                "Dhe porti ishte bosh a.",
                "Kapitulli pare",
                "Nje dite me pare of.",
            ]),
            cx,
        )
    });
    cx.run_until_parked();
    settle(&checker, cx);
    let words = marked(&checker, cx);
    assert!(!words.contains(&"Kapitulli".to_string()), "{words:?}");
    assert!(words.contains(&"Anija".to_string()) && words.contains(&"Askush".to_string()));
    // Not checked as English: the English words in it are words.
    assert!(
        !words
            .iter()
            .any(|word| ["the", "in", "a", "of"].contains(&word.as_str())),
        "{words:?}"
    );
    checker.read_with(cx, |checker, cx| {
        assert_eq!(checker.detected_language(0, cx), None);
        assert_eq!(checker.detected_language(3, cx), None);
    });
}
