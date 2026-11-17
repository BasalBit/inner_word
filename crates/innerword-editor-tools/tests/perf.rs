//! Timing of the find bar on a book-length document. Prints its
//! measurements (`cargo test -p innerword-editor-tools --test main perf -- --nocapture`).

use std::time::{Duration, Instant};

use gpui::TestAppContext;
use innerword_core::{Block, Document, SearchOptions};
use innerword_editor_tools::FIND_LAYER;

use crate::support::*;

/// 10,000 paragraphs of about a dozen words; "the" is in every one.
pub fn book(blocks: usize) -> Document {
    Document::from_blocks(
        (0..blocks)
            .map(|index| {
                Block::paragraph(&format!(
                    "Paragraph {index}: the boats came in before the weather turned, and the harbour went quiet."
                ))
            })
            .collect(),
    )
}

fn type_in_the_middle(
    editor: &gpui::Entity<innerword_editor::Editor>,
    cx: &mut gpui::VisualTestContext,
) -> Duration {
    caret(editor, 5_000, 10, cx);
    let started = Instant::now();
    for _ in 0..20 {
        cx.simulate_input("x");
    }
    started.elapsed() / 20
}

#[gpui::test]
fn typing_with_the_find_bar_open_on_a_long_document(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(book(10_000), cx);

    let closed = type_in_the_middle(&editor, cx);

    cx.simulate_keystrokes("cmd-f");
    let started = Instant::now();
    cx.simulate_input("the");
    let query = started.elapsed() / 3;
    let count = tools.read_with(cx, |tools, _| tools.find_match_count());
    assert_eq!(count, 40_000);
    cx.update(|window, cx| editor.read(cx).focus(window));
    cx.run_until_parked();

    // Typing in the document with the bar open does not search the 10,000
    // blocks again: only the block that changed.
    let searches = tools.read_with(cx, |tools, _| tools.find_full_search_count());
    let open = type_in_the_middle(&editor, cx);
    tools.read_with(cx, |tools, _| {
        assert_eq!(tools.find_full_search_count(), searches);
        assert_eq!(tools.find_match_count(), 40_000);
    });
    // In a paragraph that has no match, before and after, the editor is
    // not even handed the highlights again.
    cx.simulate_keystrokes("cmd-f cmd-a");
    cx.simulate_input("Paragraph 7");
    assert_eq!(
        tools.read_with(cx, |tools, _| tools.find_match_count()),
        1111
    );
    cx.update(|window, cx| editor.read(cx).focus(window));
    cx.run_until_parked();
    let rare = type_in_the_middle(&editor, cx);
    println!(
        "perf: per typed character with a rarer query (1,111 matches, none in the paragraph): {rare:?}"
    );
    cx.simulate_keystrokes("cmd-f cmd-a");
    cx.simulate_input("the");
    cx.update(|window, cx| editor.read(cx).focus(window));
    cx.run_until_parked();
    // And the result is the same as a search of everything.
    editor.read_with(cx, |editor, _| {
        assert_eq!(
            editor.highlights(FIND_LAYER),
            editor.document().find_all("the", SearchOptions::default())
        );
    });

    // cmd-g with the bar closed: the first press searches, the next ones
    // step through what is already known.
    cx.simulate_keystrokes("escape");
    let started = Instant::now();
    cx.simulate_keystrokes("cmd-g");
    let first_step = started.elapsed();
    let searches = tools.read_with(cx, |tools, _| tools.find_full_search_count());
    let started = Instant::now();
    for _ in 0..20 {
        cx.simulate_keystrokes("cmd-g");
    }
    let step = started.elapsed() / 20;
    assert_eq!(
        tools.read_with(cx, |tools, _| tools.find_full_search_count()),
        searches
    );

    // The format menu over the whole book (select all, then cmd-.).
    cx.simulate_keystrokes("escape cmd-a");
    let started = Instant::now();
    cx.simulate_keystrokes("cmd-.");
    let format_menu = started.elapsed();
    assert_eq!(
        active(&tools, cx),
        Some(innerword_editor_tools::ToolKind::Format)
    );
    let started = Instant::now();
    cx.simulate_keystrokes("cmd-t");
    let font_picker = started.elapsed();
    cx.simulate_keystrokes("escape");
    println!(
        "perf: with all 10,000 blocks selected, the format menu opens in {format_menu:?}, the font picker in {font_picker:?}"
    );

    println!(
        "perf: 10,000 blocks, {count} matches; per typed character: bar closed {closed:?}, bar open {open:?}; per query character {query:?}; cmd-g with the bar closed: first {first_step:?}, then {step:?}"
    );
}
