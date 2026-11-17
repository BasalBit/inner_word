//! What is done with a marked word: the menu rows, replacing, learning,
//! ignoring, and the walk from one marked word to the next.

use std::rc::Rc;

use gpui::TestAppContext;
use innerword_core::{Block, Document, InlineAttrs, Position, TextRange};
use innerword_editor::EditorEvent;
use innerword_spell::{
    EDIT_PAUSE, IGNORE_SPELLING, LEARN_SPELLING, NO_GUESSES, SpellEvent, SpellService, TYPING_PAUSE,
};

use crate::suite::support::*;

fn bold() -> InlineAttrs {
    InlineAttrs {
        bold: true,
        ..InlineAttrs::default()
    }
}

#[gpui::test]
fn a_suggestion_replaces_the_word_as_one_undo_step_and_keeps_its_formatting(
    cx: &mut TestAppContext,
) {
    setup(cx);
    let service = Rc::new(english());
    // "sentense" with its first four letters bold, "teh" bold and italic,
    // and "wrod" plain.
    let both = InlineAttrs {
        bold: true,
        italic: true,
        ..InlineAttrs::default()
    };
    let document = Document::from_blocks(vec![
        Block::paragraph("This ")
            .with_span("sent", bold())
            .with_span("ense with ", InlineAttrs::default())
            .with_span("teh", both.clone())
            .with_span(" wrod here.", InlineAttrs::default()),
    ]);
    let (checker, editor, cx) = open(document, &service, cx);
    assert_eq!(marked(&checker, cx), ["sentense", "teh", "wrod"]);

    // The rows for the word at a position: at most five suggestions,
    // then learn and ignore.
    let titles = |position: Position, cx: &mut gpui::VisualTestContext| -> Vec<String> {
        checker.update(cx, |checker, cx| {
            checker
                .menu_items_at(position, cx)
                .iter()
                .map(|item| item.title().to_string())
                .collect()
        })
    };
    assert_eq!(
        titles(Position::new(0, 7), cx),
        ["sentence", "sentences", LEARN_SPELLING, IGNORE_SPELLING]
    );
    assert_eq!(
        titles(Position::new(0, 27), cx),
        [
            "word",
            "wood",
            "rod",
            "trod",
            "prod",
            LEARN_SPELLING,
            IGNORE_SPELLING
        ],
        "five of the seven"
    );
    assert!(
        titles(Position::new(0, 2), cx).is_empty(),
        "not a marked word"
    );
    assert_eq!(
        checker.update(cx, |checker, cx| checker
            .suggestions_at(Position::new(0, 20), cx)),
        ["the", "ten", "tea"]
    );

    // Through the keyboard: the caret in the word, the suggestions as a
    // menu under it, the first row marked.
    caret(&editor, 0, 8, cx);
    let before = editor.read_with(cx, |editor, _| editor.state().undo_depth());
    cx.dispatch_action(innerword_spell::actions::ShowSuggestions);
    cx.run_until_parked();
    assert_eq!(
        menu_labels(cx).unwrap(),
        ["sentence", "sentences", LEARN_SPELLING, IGNORE_SPELLING]
    );
    let selected = cx.update(|window, cx| {
        innerword_ui::ContextMenu::current(window, cx).and_then(|menu| {
            menu.read(cx)
                .selected_item()
                .map(|item| item.title().to_string())
        })
    });
    assert_eq!(selected.as_deref(), Some("sentence"));
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "This sentence with teh wrod here.");
    editor.read_with(cx, |editor, _| {
        let block = &editor.document().blocks()[0];
        // "sent" is still bold, "ence" still is not.
        assert!(block.attrs_at(5).unwrap().bold && block.attrs_at(8).unwrap().bold);
        assert!(!block.attrs_at(9).unwrap().bold && !block.attrs_at(12).unwrap().bold);
        assert_eq!(
            editor.selection().head,
            Position::new(0, 13),
            "the caret is after the word"
        );
        assert!(editor.selection().is_collapsed());
        assert_eq!(editor.state().undo_depth(), before + 1, "one undo step");
    });
    wait(EDIT_PAUSE, cx);
    assert_eq!(marked(&checker, cx), ["teh", "wrod"]);

    // A replacement of another length takes the formatting of the word.
    let teh = checker
        .read_with(cx, |checker, cx| {
            checker.misspelling_at(Position::new(0, 20), cx)
        })
        .unwrap();
    assert!(checker.update(cx, |checker, cx| checker.replace(teh, "the", cx)));
    assert_eq!(text(&editor, cx), "This sentence with the wrod here.");
    editor.read_with(cx, |editor, _| {
        let block = &editor.document().blocks()[0];
        for offset in 19..22 {
            assert_eq!(block.attrs_at(offset), Some(&both), "{offset}");
        }
        assert!(block.attrs_at(22).unwrap().is_plain());
    });

    // Undo takes one replacement back at a time, with its formatting.
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(text(&editor, cx), "This sentence with teh wrod here.");
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(text(&editor, cx), "This sentense with teh wrod here.");
    editor.read_with(cx, |editor, _| {
        let block = &editor.document().blocks()[0];
        assert!(block.attrs_at(8).unwrap().bold && !block.attrs_at(9).unwrap().bold);
    });
    wait(TYPING_PAUSE, cx);
    assert_eq!(marked(&checker, cx), ["sentense", "teh", "wrod"]);
}

#[gpui::test]
fn replacements_longer_and_shorter_and_with_an_apostrophe(cx: &mut TestAppContext) {
    setup(cx);
    let service = Rc::new(english());
    let document = Document::from_blocks(vec![
        Block::paragraph("I ")
            .with_span("dont", bold())
            .with_span(" and ", InlineAttrs::default())
            .with_span("mornin", bold())
            .with_span(" x", InlineAttrs::default()),
    ]);
    let (checker, editor, cx) = open(document, &service, cx);
    let events = Rc::new(std::cell::RefCell::new(Vec::new()));
    let _subscription = cx.update(|_, cx| {
        let events = events.clone();
        cx.subscribe(&checker, move |_, event: &SpellEvent, _| {
            events.borrow_mut().push(event.clone())
        })
    });
    let range = |start, end| TextRange::in_block(0, start, end);
    // Only letters added at the end: they are bold like the word.
    assert!(checker.update(cx, |checker, cx| checker.replace(
        range(11, 17),
        "morning",
        cx
    )));
    assert_eq!(text(&editor, cx), "I dont and morning x");
    editor.read_with(cx, |editor, _| {
        let block = &editor.document().blocks()[0];
        assert!((11..18).all(|offset| block.attrs_at(offset).unwrap().bold));
        assert!(!block.attrs_at(18).unwrap().bold);
        assert_eq!(editor.selection().head, Position::new(0, 18));
    });
    // The apostrophe is the document's own (smart punctuation is on).
    assert!(checker.update(cx, |checker, cx| checker.replace(range(2, 6), "don't", cx)));
    assert_eq!(text(&editor, cx), "I don\u{2019}t and morning x");
    editor.read_with(cx, |editor, _| {
        let block = &editor.document().blocks()[0];
        assert!((2..9).all(|offset| block.attrs_at(offset).unwrap().bold));
        assert!(!block.attrs_at(9).unwrap().bold);
    });
    assert!(events.borrow().contains(&SpellEvent::Replaced {
        from: "dont".into(),
        to: "don\u{2019}t".into()
    }));
    // Nothing to do, or nothing to do it to.
    assert!(!checker.update(cx, |checker, cx| checker.replace(range(0, 1), "I", cx)));
    assert!(!checker.update(cx, |checker, cx| checker.replace(range(0, 99), "x", cx)));
    assert!(!checker.update(cx, |checker, cx| checker.replace(
        TextRange::new(Position::new(0, 0), Position::new(1, 0)),
        "x",
        cx
    )));
    // A read-only editor is not written in.
    editor.update(cx, |editor, cx| editor.set_read_only(true, cx));
    assert!(!checker.update(cx, |checker, cx| checker.replace(range(0, 1), "We", cx)));
}

#[gpui::test]
fn no_guesses_found(cx: &mut TestAppContext) {
    setup(cx);
    let service = Rc::new(english());
    let (checker, editor, cx) = open(paragraphs(&["A zorblat here."]), &service, cx);
    caret(&editor, 0, 4, cx);
    cx.dispatch_action(innerword_spell::actions::ShowSuggestions);
    cx.run_until_parked();
    assert_eq!(
        menu_labels(cx).unwrap(),
        [NO_GUESSES, LEARN_SPELLING, IGNORE_SPELLING]
    );
    let (disabled, selected) = cx.update(|window, cx| {
        let menu = innerword_ui::ContextMenu::current(window, cx).unwrap();
        let menu = menu.read(cx);
        (
            menu.items()[0].is_disabled(),
            menu.selected_item().map(|item| item.title().to_string()),
        )
    });
    assert!(disabled);
    assert_eq!(
        selected.as_deref(),
        Some(LEARN_SPELLING),
        "the first row that can be chosen"
    );
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert_eq!(menu_labels(cx), None);
    assert_eq!(text(&editor, cx), "A zorblat here.");

    // No marked word at the caret: the command says so.
    caret(&editor, 0, 12, cx);
    cx.dispatch_action(innerword_spell::actions::ShowSuggestions);
    cx.run_until_parked();
    assert_eq!(menu_labels(cx), None);
    assert_eq!(
        notice(&checker, cx).as_deref(),
        Some("No misspelled word at the caret")
    );
}

#[gpui::test]
fn the_rows_for_a_context_menu_request(cx: &mut TestAppContext) {
    setup(cx);
    let service = Rc::new(english());
    let (checker, editor, cx) = open(paragraphs(&["The teh ship came in."]), &service, cx);
    let requests = Rc::new(std::cell::RefCell::new(Vec::new()));
    let _subscription = cx.update(|_, cx| {
        let requests = requests.clone();
        cx.subscribe(&editor, move |_, event: &EditorEvent, _| {
            if let EditorEvent::ContextMenuRequested(request) = event {
                requests.borrow_mut().push(request.clone());
            }
        })
    });
    // ctrl-enter with the caret in the word.
    caret(&editor, 0, 5, cx);
    cx.simulate_keystrokes("ctrl-enter");
    cx.run_until_parked();
    let request = requests.borrow().last().cloned().expect("a request");
    let titles: Vec<String> = checker.update(cx, |checker, cx| {
        checker
            .context_menu_items(&request, cx)
            .iter()
            .map(|item| item.title().to_string())
            .collect()
    });
    assert_eq!(
        titles,
        ["the", "ten", "tea", LEARN_SPELLING, IGNORE_SPELLING]
    );
    // In a word that is not marked: nothing to add.
    caret(&editor, 0, 10, cx);
    cx.simulate_keystrokes("ctrl-enter");
    cx.run_until_parked();
    let request = requests.borrow().last().cloned().expect("a request");
    assert!(
        checker
            .update(cx, |checker, cx| checker.context_menu_items(&request, cx))
            .is_empty()
    );

    // A word typed a moment ago is not marked yet, but a menu on it
    // knows that it is misspelled.
    cx.simulate_keystrokes("cmd-right");
    cx.simulate_input(" wrod");
    assert_eq!(marked(&checker, cx), ["teh"]);
    cx.simulate_keystrokes("ctrl-enter");
    cx.run_until_parked();
    let request = requests.borrow().last().cloned().expect("a request");
    let titles: Vec<String> = checker.update(cx, |checker, cx| {
        checker
            .context_menu_items(&request, cx)
            .iter()
            .map(|item| item.title().to_string())
            .collect()
    });
    assert_eq!(titles[0], "word");
    assert_eq!(marked(&checker, cx), ["teh", "wrod"]);
}

#[gpui::test]
fn learn_and_ignore_clear_every_occurrence(cx: &mut TestAppContext) {
    setup(cx);
    let service = Rc::new(english());
    let document = paragraphs(&[
        "A zorblat here and a Zorblat there.",
        "Teh zorblat and the quux.",
        "Zorblat, quux and zorblat's wrod.",
    ]);
    let (checker, editor, cx) = open(document, &service, cx);
    assert_eq!(
        marked(&checker, cx),
        [
            "zorblat",
            "Zorblat",
            "Teh",
            "zorblat",
            "quux",
            "Zorblat",
            "quux",
            "zorblat's",
            "wrod"
        ]
    );
    let events = Rc::new(std::cell::RefCell::new(Vec::new()));
    let _subscription = cx.update(|_, cx| {
        let events = events.clone();
        cx.subscribe(&checker, move |_, event: &SpellEvent, _| {
            events.borrow_mut().push(event.clone())
        })
    });

    // Ignore, from the menu of one occurrence: all of them at once, and
    // without asking about any paragraph again.
    service.reset_calls();
    caret(&editor, 1, 22, cx);
    cx.dispatch_action(innerword_spell::actions::ShowSuggestions);
    cx.run_until_parked();
    assert!(choose(IGNORE_SPELLING, cx));
    assert_eq!(
        marked(&checker, cx),
        [
            "zorblat",
            "Zorblat",
            "Teh",
            "zorblat",
            "Zorblat",
            "zorblat's",
            "wrod"
        ]
    );
    assert!(
        events
            .borrow()
            .contains(&SpellEvent::Ignored("quux".into()))
    );
    let document_tag = checker.read_with(cx, |checker, _| checker.document());
    assert_eq!(service.ignored_words(document_tag), ["quux"]);
    assert!(
        service
            .checked_texts()
            .iter()
            .all(|text| !text.contains(' ')),
        "single words at most"
    );

    // Learn, with the key-less command at the caret: every occurrence,
    // the capitalised one too. "zorblat's" is another word to the fake.
    caret(&editor, 0, 3, cx);
    cx.dispatch_action(innerword_spell::actions::LearnSpelling);
    cx.run_until_parked();
    assert_eq!(marked(&checker, cx), ["Teh", "zorblat's", "wrod"]);
    assert!(service.has_learned("zorblat"));
    assert!(
        events
            .borrow()
            .contains(&SpellEvent::Learned("zorblat".into()))
    );
    assert_eq!(
        notice(&checker, cx).as_deref(),
        Some("Learned \u{201c}zorblat\u{201d}")
    );

    // A new occurrence of a learned or ignored word is not marked.
    cx.simulate_keystrokes("cmd-right");
    cx.simulate_input(" Quux zorblat ");
    wait(EDIT_PAUSE, cx);
    assert_eq!(marked(&checker, cx), ["Teh", "zorblat's", "wrod"]);

    // Unlearn at the caret (the word is not marked any more): it is
    // misspelled again, everywhere.
    caret(&editor, 0, 4, cx);
    cx.dispatch_action(innerword_spell::actions::UnlearnSpelling);
    cx.run_until_parked();
    wait(TYPING_PAUSE, cx);
    assert!(!service.has_learned("zorblat"));
    assert_eq!(
        marked(&checker, cx),
        [
            "zorblat",
            "Zorblat",
            "zorblat",
            "Teh",
            "zorblat",
            "Zorblat",
            "zorblat's",
            "wrod"
        ]
    );
    assert!(
        events
            .borrow()
            .contains(&SpellEvent::Unlearned("zorblat".into()))
    );
    // Not learned, so nothing to unlearn; and no marked word to learn.
    caret(&editor, 0, 1, cx);
    cx.dispatch_action(innerword_spell::actions::UnlearnSpelling);
    assert_eq!(
        notice(&checker, cx).as_deref(),
        Some("No learned word at the caret")
    );
    cx.dispatch_action(innerword_spell::actions::IgnoreSpelling);
    assert_eq!(
        notice(&checker, cx).as_deref(),
        Some("No misspelled word at the caret")
    );

    // By position, as the Vim keys ask.
    assert_eq!(
        checker
            .update(cx, |checker, cx| checker.learn_at(Position::new(2, 30), cx))
            .as_deref(),
        Some("wrod")
    );
    assert_eq!(
        checker.update(cx, |checker, cx| checker.learn_at(Position::new(2, 30), cx)),
        None
    );
    assert_eq!(
        checker
            .update(cx, |checker, cx| checker
                .unlearn_at(Position::new(2, 31), cx))
            .as_deref(),
        Some("wrod")
    );
    assert_eq!(
        checker.update(cx, |checker, cx| checker
            .unlearn_at(Position::new(2, 31), cx)),
        None
    );
    assert_eq!(
        checker
            .update(cx, |checker, cx| checker.ignore_at(Position::new(1, 1), cx))
            .as_deref(),
        Some("Teh")
    );
    assert_eq!(
        checker
            .read_with(cx, |checker, cx| checker.word_at(Position::new(1, 6), cx))
            .map(|(_, word)| word)
            .as_deref(),
        Some("zorblat")
    );
    service.unlearn("wrod");
}

#[gpui::test]
fn next_and_previous_wrap_around_the_document(cx: &mut TestAppContext) {
    setup(cx);
    let service = Rc::new(english());
    let document = paragraphs(&[
        "The teh ship and a wrod.",
        "Nobody expected it.",
        "A sentense here.",
    ]);
    let (checker, editor, cx) = open(document, &service, cx);
    caret(&editor, 0, 0, cx);

    cx.simulate_keystrokes("cmd-;");
    assert_eq!(selected_text(&editor, cx), "teh");
    cx.simulate_keystrokes("cmd-;");
    assert_eq!(selected_text(&editor, cx), "wrod");
    cx.simulate_keystrokes("cmd-;");
    assert_eq!(selected_text(&editor, cx), "sentense");
    assert_eq!(
        editor.read_with(cx, |editor, _| editor.selection().start()),
        Position::new(2, 2)
    );
    cx.simulate_keystrokes("cmd-;");
    assert_eq!(selected_text(&editor, cx), "teh", "around the end");

    cx.simulate_keystrokes("cmd-shift-;");
    assert_eq!(selected_text(&editor, cx), "sentense", "around the start");
    cx.simulate_keystrokes("cmd-shift-;");
    assert_eq!(selected_text(&editor, cx), "wrod");
    // The key as a real keyboard sends it.
    cx.simulate_keystrokes("cmd-:");
    assert_eq!(selected_text(&editor, cx), "teh");

    // From a caret inside a marked word: that word first, then the next.
    caret(&editor, 0, 5, cx);
    cx.simulate_keystrokes("cmd-;");
    assert_eq!(selected_text(&editor, cx), "teh");
    // From a caret right behind one: the next; backwards: that one.
    caret(&editor, 0, 7, cx);
    cx.simulate_keystrokes("cmd-;");
    assert_eq!(selected_text(&editor, cx), "wrod");
    caret(&editor, 0, 7, cx);
    cx.simulate_keystrokes("cmd-shift-;");
    assert_eq!(selected_text(&editor, cx), "teh");

    // The queries the Vim keys use: strictly after, strictly before.
    let (next, previous) = checker.update(cx, |checker, cx| {
        (
            checker.next_misspelling(Position::new(0, 4), cx),
            checker.previous_misspelling(Position::new(0, 4), cx),
        )
    });
    assert_eq!(
        next,
        Some(TextRange::in_block(0, 19, 23)),
        "not the word that starts here"
    );
    assert_eq!(
        previous,
        Some(TextRange::in_block(2, 2, 10)),
        "around the start"
    );
    assert_eq!(
        checker.update(cx, |checker, cx| checker
            .previous_misspelling(Position::new(0, 6), cx)),
        Some(TextRange::in_block(0, 4, 7)),
        "the word the position is in"
    );

    // One misspelling: it is the next one from anywhere.
    editor.update(cx, |editor, cx| {
        editor.set_document(paragraphs(&["Good.", "A wrod here."]), cx)
    });
    cx.run_until_parked();
    for from in [
        Position::new(0, 0),
        Position::new(1, 2),
        Position::new(1, 8),
    ] {
        let (next, previous) = checker.update(cx, |checker, cx| {
            (
                checker.next_misspelling(from, cx),
                checker.previous_misspelling(from, cx),
            )
        });
        assert_eq!(next, Some(TextRange::in_block(1, 2, 6)), "{from:?}");
        assert_eq!(previous, Some(TextRange::in_block(1, 2, 6)), "{from:?}");
    }

    // None: the command says so and leaves the selection alone.
    editor.update(cx, |editor, cx| {
        editor.set_document(paragraphs(&["All good here."]), cx)
    });
    cx.run_until_parked();
    caret(&editor, 0, 4, cx);
    cx.simulate_keystrokes("cmd-;");
    assert_eq!(
        notice(&checker, cx).as_deref(),
        Some("No misspellings found")
    );
    assert!(editor.read_with(cx, |editor, _| editor.selection().is_collapsed()));
    assert_eq!(
        checker.update(cx, |checker, cx| checker
            .next_misspelling(Position::new(0, 0), cx)),
        None
    );
}

#[gpui::test]
fn the_commands_are_listed_and_bound(cx: &mut TestAppContext) {
    setup(cx);
    let commands = innerword_spell::commands();
    let titles: Vec<&str> = commands
        .iter()
        .map(|command| command.title.as_ref())
        .collect();
    assert_eq!(
        titles,
        [
            "Next Misspelling",
            "Previous Misspelling",
            "Show Spelling Suggestions",
            "Learn Spelling",
            "Unlearn Spelling",
            "Ignore Spelling",
            "Check Spelling While Typing",
        ]
    );
    assert!(
        commands
            .iter()
            .all(|command| command.category.as_ref() == "Spelling")
    );
    let service = Rc::new(english());
    let (_checker, _editor, cx) = open(paragraphs(&["Text"]), &service, cx);
    let hints: Vec<Option<String>> = cx.update(|_, cx| {
        commands
            .iter()
            .map(|command| command.key_hint(cx).map(|hint| format!("{hint:?}")))
            .collect()
    });
    assert!(
        hints[0].is_some() && hints[1].is_some(),
        "next and previous have keys"
    );
    assert!(hints[2..].iter().all(Option::is_none));
}
