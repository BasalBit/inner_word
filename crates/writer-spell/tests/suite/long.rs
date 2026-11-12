//! Book-length documents and long blocks: what is on screen first, the
//! rest in slices, typing within budget. Prints its measurements
//! (`cargo test -p writer-spell --test main long -- --nocapture`).

use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::TestAppContext;
use writer_core::{Block, Document, Position};
use writer_spell::{
    EDIT_PAUSE, FAKE_LANGUAGE, IDLE_PAUSE, SLICE_BUDGET, SLICE_PAUSE, SpellEvent, SpellLanguage,
    TYPING_PAUSE,
};

use crate::suite::support::*;

/// `blocks` paragraphs of about fifteen words; every tenth has a
/// misspelled word.
fn book(blocks: usize) -> Document {
    Document::from_blocks(
        (0..blocks)
            .map(|index| {
                let word = if index % 10 == 0 { "wrod" } else { "word" };
                Block::paragraph(&format!(
                    "Paragraph {index}: the boats came in before the weather turned, and the harbor went quiet with a {word}."
                ))
            })
            .collect(),
    )
}

#[gpui::test]
fn a_book_is_checked_from_the_screen_outwards_in_slices(cx: &mut TestAppContext) {
    setup(cx);
    // Each call costs what the system checker costs for such a paragraph.
    let cost = Duration::from_micros(150);
    let service = Rc::new(english().check_cost(cost));
    let started = Instant::now();
    let (checker, editor, cx) = open(book(10_000), &service, cx);
    let opened = started.elapsed();

    // The first turn of the worker has run: what is on screen is marked,
    // the rest is not checked yet.
    let visible = editor
        .read_with(cx, |editor, _| editor.visible_range())
        .map(|range| range.start.block..range.end.block + 1)
        .expect("a layout");
    editor.read_with(cx, |editor, _| {
        assert!(visible.len() > 3 && visible.len() < 200, "{visible:?}");
        assert_eq!(editor.first_unchecked_spelling_block(visible.clone()), None);
        let text = editor.document().blocks()[0].text();
        let wrod = text.find("wrod").unwrap();
        assert_eq!(editor.spelling_marks(0), [wrod..wrod + 4]);
    });
    let first = service.check_count();
    assert!(
        first >= visible.len() && first < 2_000,
        "{first} checks in the first slice"
    );
    assert!(!checker.read_with(cx, |checker, cx| checker.is_settled(cx)));
    let (_, first_slice) = checker.read_with(cx, |checker, _| checker.slice_stats());
    println!(
        "long: 10,000 blocks opened and the first slice run in {opened:?}; the first slice took {first_slice:?} and checked {first} blocks ({} on screen)",
        visible.len()
    );

    // Far away, before the background pass gets there: scrolling brings
    // those blocks to the front.
    editor.update(cx, |editor, cx| {
        editor.scroll_to_position(Position::new(7_000, 0), writer_editor::Autoscroll::Top, cx)
    });
    cx.run_until_parked();
    wait(SLICE_PAUSE, cx);
    wait(SLICE_PAUSE, cx);
    let there = editor
        .read_with(cx, |editor, _| editor.visible_range())
        .map(|range| range.start.block..range.end.block + 1)
        .unwrap();
    assert!(there.contains(&7_000), "{there:?}");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.first_unchecked_spelling_block(there.clone()), None);
        assert_eq!(editor.spelling_marks(7_000).len(), 1);
        assert!(!editor.spelling_checked(5_000), "the pass is not there yet");
    });

    // Typing while the pass runs: the keystroke itself asks nothing, the
    // paragraph is checked after the pause, and the pass waits until the
    // typing stops.
    caret(&editor, 7_000, 0, cx);
    checker.update(cx, |checker, _| checker.reset_slice_stats());
    let mut slowest_key = Duration::ZERO;
    let mut keys = Duration::ZERO;
    let before = service.check_count();
    for round in 0..30 {
        let started = Instant::now();
        cx.simulate_input("x");
        let key = started.elapsed();
        slowest_key = slowest_key.max(key);
        keys += key;
        // 60 ms between keys: a fast typist.
        wait(Duration::from_millis(60), cx);
        if round == 0 {
            assert_eq!(
                service.check_count(),
                before,
                "nothing under the key or right after it"
            );
        }
    }
    let while_typing = service.check_count() - before;
    let (slices, longest) = checker.read_with(cx, |checker, _| checker.slice_stats());
    println!(
        "long: 30 keys while the pass was running: {:?} per key (slowest {slowest_key:?}); {while_typing} checks in {slices} slices, the longest slice {longest:?}",
        keys / 30
    );
    assert!(
        while_typing <= 30,
        "only the paragraph being typed in: {while_typing}"
    );
    assert!(
        !editor.read_with(cx, |editor, _| editor.spelling_checked(5_000)),
        "the pass waited"
    );

    // The typing stops: the pass goes on and comes to an end.
    wait(IDLE_PAUSE, cx);
    checker.update(cx, |checker, _| checker.reset_slice_stats());
    let events = Rc::new(std::cell::RefCell::new(Vec::new()));
    let _subscription = cx.update(|_, cx| {
        let events = events.clone();
        cx.subscribe(&checker, move |_, event: &SpellEvent, _| {
            events.borrow_mut().push(event.clone())
        })
    });
    let started = Instant::now();
    let rounds = settle(&checker, cx);
    let took = started.elapsed();
    let (slices, longest) = checker.read_with(cx, |checker, _| checker.slice_stats());
    println!(
        "long: the rest of the pass: {slices} slices in {rounds} rounds, {took:?} of work (the pauses between slices are not real time here), the longest slice {longest:?} against a budget of {SLICE_BUDGET:?}"
    );
    assert_eq!(
        checker.read_with(cx, |checker, cx| checker.pending_blocks(cx)),
        0
    );
    assert_eq!(
        checker.read_with(cx, |checker, cx| checker.misspelling_count(cx)),
        1_001
    );
    // Whoever counts the marks is told a few times a second while the
    // pass runs, not once a slice, and a last time before it is over.
    {
        let events = events.borrow();
        let told = events
            .iter()
            .filter(|event| **event == SpellEvent::MarksChanged)
            .count();
        println!("long: {told} MarksChanged events for {slices} slices");
        assert!(
            told >= 1 && (told as u64) < slices / 4,
            "{told} events, {slices} slices"
        );
        let last_marks = events
            .iter()
            .rposition(|event| *event == SpellEvent::MarksChanged);
        let settled = events
            .iter()
            .rposition(|event| *event == SpellEvent::Settled);
        assert!(last_marks.is_some() && last_marks < settled, "{events:?}");
    }
    wait(TYPING_PAUSE, cx);
    assert_eq!(
        checker.read_with(cx, |checker, cx| checker.misspelling_count(cx)),
        1_001
    );
    // Every block once, the typed paragraph a few times more.
    let total = service.check_count();
    assert!((10_000..10_000 + 40).contains(&total), "{total}");
    // A slice ends with the call that passes the budget; one call of the
    // fake costs 150 µs, the machine may be busy, so the bound is loose.
    assert!(longest < SLICE_BUDGET * 5, "{longest:?}");

    // Everything is checked: going around, scrolling and waiting ask
    // nothing more.
    service.reset_calls();
    cx.simulate_keystrokes("cmd-down");
    cx.simulate_keystrokes("cmd-up");
    wait(IDLE_PAUSE, cx);
    wait(TYPING_PAUSE, cx);
    assert_eq!(service.check_count(), 0);

    // "Next misspelling" from the end wraps to the first one.
    cx.simulate_keystrokes("cmd-down cmd-;");
    assert_eq!(selected_text(&editor, cx), "wrod");
    assert_eq!(
        editor.read_with(cx, |editor, _| editor.selection().start().block),
        0
    );
}

#[gpui::test]
fn zooming_out_checks_what_comes_into_view_first(cx: &mut TestAppContext) {
    setup(cx);
    let service = Rc::new(english().check_cost(Duration::from_micros(150)));
    let (_checker, editor, cx) = open(book(10_000), &service, cx);
    let on_screen = |cx: &mut gpui::VisualTestContext| {
        editor
            .read_with(cx, |editor, _| editor.visible_range())
            .map(|range| range.start.block..range.end.block + 1)
            .expect("a layout")
    };
    // Far into the book, where the background pass has not been, at a
    // large zoom: a few paragraphs are on screen, and checked.
    editor.update(cx, |editor, cx| {
        editor.set_zoom(writer_editor::Zoom::Percent(300), cx);
        editor.scroll_to_position(Position::new(6_000, 0), writer_editor::Autoscroll::Top, cx)
    });
    cx.run_until_parked();
    wait(SLICE_PAUSE, cx);
    wait(SLICE_PAUSE, cx);
    let near = on_screen(cx);
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.first_unchecked_spelling_block(near.clone()), None);
    });
    // Zoomed out, the same place shows several times as much text. The
    // view did not scroll and the window did not change: the zoom alone
    // brought those paragraphs into view, and they are checked now, not
    // when the background pass arrives.
    editor.update(cx, |editor, cx| {
        editor.set_zoom(writer_editor::Zoom::Percent(50), cx)
    });
    cx.run_until_parked();
    wait(SLICE_PAUSE, cx);
    wait(SLICE_PAUSE, cx);
    let far = on_screen(cx);
    assert!(far.len() > near.len() * 3, "{near:?} then {far:?}");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.first_unchecked_spelling_block(far.clone()), None);
        assert!(
            !editor.spelling_checked(far.end + 2_000),
            "the pass is not there yet"
        );
    });
}

#[gpui::test]
fn next_misspelling_checks_ahead_in_a_book_that_is_not_checked_yet(cx: &mut TestAppContext) {
    setup(cx);
    let service = Rc::new(english());
    // One misspelled word, far from the top.
    let mut blocks: Vec<Block> = (0..3_000)
        .map(|_| Block::paragraph("The ship came in."))
        .collect();
    blocks[2_500] = Block::paragraph("The wrod came in.");
    let (checker, editor, cx) = open(Document::from_blocks(blocks), &service, cx);
    assert!(!checker.read_with(cx, |checker, cx| checker.is_settled(cx)));
    assert!(!editor.read_with(cx, |editor, _| editor.spelling_checked(2_500)));
    caret(&editor, 0, 0, cx);
    cx.simulate_keystrokes("cmd-;");
    // Found although the background pass was nowhere near (the fake
    // answers at once, so the time limit is not reached).
    assert_eq!(selected_text(&editor, cx), "wrod");
    assert_eq!(
        editor.read_with(cx, |editor, _| editor.selection().start()),
        Position::new(2_500, 4)
    );
    // And backwards from the top, around the start of the document.
    caret(&editor, 0, 0, cx);
    cx.simulate_keystrokes("cmd-shift-;");
    assert_eq!(selected_text(&editor, cx), "wrod");
}

#[gpui::test]
fn next_misspelling_says_when_it_ran_out_of_time(cx: &mut TestAppContext) {
    setup(cx);
    // A slow service: 2 ms a paragraph, 3,000 paragraphs, nothing
    // misspelled. The walk gives up after its time limit.
    let service = Rc::new(english().check_cost(Duration::from_millis(2)));
    let blocks: Vec<Block> = (0..3_000)
        .map(|_| Block::paragraph("The ship came in."))
        .collect();
    let (checker, editor, cx) = open(Document::from_blocks(blocks), &service, cx);
    caret(&editor, 0, 0, cx);
    let started = Instant::now();
    cx.simulate_keystrokes("cmd-;");
    let took = started.elapsed();
    assert_eq!(
        notice(&checker, cx).as_deref(),
        Some("No misspelling found yet: the document is still being checked")
    );
    assert!(editor.read_with(cx, |editor, _| editor.selection().is_collapsed()));
    println!(
        "long: next misspelling gave up after {took:?} (limit {:?})",
        writer_spell::SEEK_LIMIT
    );
    assert!(took < writer_spell::SEEK_LIMIT * 4, "{took:?}");
}

#[gpui::test]
fn a_long_block_is_checked_in_pieces(cx: &mut TestAppContext) {
    setup(cx);
    // 2 ms a call: more than half a slice, so a block of many pieces
    // cannot be done in one.
    let service = Rc::new(english().check_cost(Duration::from_millis(2)));
    // A misspelled word in every fourth sentence.
    let good =
        "The boats came in before the weather turned and the harbor went quiet with a word. ";
    let bad = "The boats came in before the weather turned and the harbor went quiet with a wrod. ";
    let long: String = (0..200)
        .map(|i| if i % 4 == 0 { bad } else { good })
        .collect();
    let document =
        Document::from_blocks(vec![Block::paragraph(&long), Block::paragraph("A wrod.")]);
    let (checker, editor, cx) = open(document, &service, cx);
    // The first slice did not get through the first block.
    assert!(!editor.read_with(cx, |editor, _| editor.spelling_checked(0)));
    assert!(marked(&checker, cx).is_empty(), "no half results");
    settle(&checker, cx);
    let (slices, _) = checker.read_with(cx, |checker, _| checker.slice_stats());
    assert!(slices > 5, "{slices} slices");
    assert_eq!(marked(&checker, cx).len(), 51);
    // The pieces are stretches of the text cut at white space, each sent
    // once, and together they are the block.
    let texts = service.checked_texts();
    let pieces = &texts[..texts.len() - 1];
    assert!(pieces.len() > 5, "{}", pieces.len());
    assert_eq!(pieces.concat(), long);
    assert!(pieces.iter().all(|piece| piece.len() < 3_000));
    let (_, longest) = checker.read_with(cx, |checker, _| checker.slice_stats());
    println!(
        "long: a block of {} bytes in {} pieces, the longest slice {longest:?}",
        long.len(),
        pieces.len()
    );
    // Every mark is on a "wrod".
    editor.read_with(cx, |editor, _| {
        let text = editor.document().blocks()[0].text();
        assert!(
            editor
                .spelling_marks(0)
                .iter()
                .all(|range| &text[range.clone()] == "wrod")
        );
    });

    // An edit while the block is being checked again: the unfinished
    // check is dropped and the block is checked as it is now.
    caret(&editor, 0, 0, cx);
    cx.simulate_input("Teh ");
    wait(EDIT_PAUSE, cx);
    cx.simulate_input("x");
    settle(&checker, cx);
    wait(TYPING_PAUSE, cx);
    let words = marked(&checker, cx);
    assert_eq!(words.len(), 53);
    assert_eq!(words[..2], ["Teh", "xThe"]);
}

#[gpui::test]
fn a_paragraph_in_another_language_is_checked_a_few_words_at_a_time(cx: &mut TestAppContext) {
    setup(cx);
    // With a fixed language every unknown word costs a call into the
    // system checker; the checker asks for sixteen at a time.
    let service = Rc::new(english());
    let german = "Das Schiff kam heute Morgen im Hafen an und niemand hat es erwartet weil der Hafen leer war";
    let (checker, _editor, cx) = open_in(
        paragraphs(&[german]),
        &service,
        SpellLanguage::fixed(FAKE_LANGUAGE),
        Default::default(),
        cx,
    );
    settle(&checker, cx);
    let words = marked(&checker, cx);
    // "an" is an English word too.
    assert_eq!(words.len(), 17, "{words:?}");
    assert_eq!(words.join(" "), german.replace(" an ", " "));
    let texts = service.checked_texts();
    assert_eq!(texts.len(), 2, "sixteen words, then the rest");
    assert_eq!(texts[0], german);
    assert_eq!(texts[1], " war");
}
