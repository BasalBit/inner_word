//! Replace Selection and Insert Below on a real editor.

use gpui::{TestAppContext, VisualTestContext};
use writer_assistant_quick::{
    Capture,
    apply::{answer_fragment, can_insert, can_replace, insert_below, replace_selection},
};
use writer_core::{Block, BlockKind, Document, InlineAttrs, InlineStyle, Position, Rgb, TextRange};
use writer_editor::Editor;

use crate::support::*;

/// Selects `range` and captures it, as opening the chat does.
fn capture(editor: &gpui::Entity<Editor>, range: TextRange, cx: &mut VisualTestContext) -> Capture {
    select(editor, range, cx);
    editor.read_with(cx, |editor, _| {
        Capture::new(editor.document(), editor.selection().range())
    })
}

fn replace(
    editor: &gpui::Entity<Editor>,
    capture: &Capture,
    markdown: &str,
    cx: &mut VisualTestContext,
) -> Option<TextRange> {
    let done = editor.update(cx, |editor, cx| {
        replace_selection(editor, capture, markdown, cx)
    });
    cx.run_until_parked();
    done
}

fn below(
    editor: &gpui::Entity<Editor>,
    capture: &Capture,
    markdown: &str,
    cx: &mut VisualTestContext,
) -> Option<TextRange> {
    let done = editor.update(cx, |editor, cx| insert_below(editor, capture, markdown, cx));
    cx.run_until_parked();
    done
}

#[gpui::test]
fn replacing_a_word_inserts_inline_text(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open_editor(paragraphs(&["The quick brown fox", "Next."]), cx);
    let capture = capture(&editor, TextRange::in_block(0, 4, 9), cx);
    let depth = undo_depth(&editor, cx);

    let changed = replace(&editor, &capture, "swift", cx);

    assert_eq!(text(&editor, cx), "The swift brown fox\nNext.");
    assert_eq!(changed, Some(TextRange::in_block(0, 4, 9)));
    assert_eq!(selection(&editor, cx), TextRange::in_block(0, 4, 9));
    assert_eq!(selected_text(&editor, cx), "swift");
    assert_eq!(undo_depth(&editor, cx), depth + 1, "one undo step");

    undo(&editor, cx);
    assert_eq!(text(&editor, cx), "The quick brown fox\nNext.");
    redo(&editor, cx);
    assert_eq!(text(&editor, cx), "The swift brown fox\nNext.");
}

#[gpui::test]
fn replacing_works_after_the_caret_has_moved(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open_editor(paragraphs(&["The quick brown fox", "Next."]), cx);
    let capture = capture(&editor, TextRange::in_block(0, 4, 9), cx);
    caret(&editor, 1, 2, cx);

    replace(&editor, &capture, "swift", cx);

    assert_eq!(text(&editor, cx), "The swift brown fox\nNext.");
    assert_eq!(selected_text(&editor, cx), "swift");
}

#[gpui::test]
fn an_answer_with_emphasis_is_formatted_text(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open_editor(paragraphs(&["The quick brown fox"]), cx);
    let capture = capture(&editor, TextRange::in_block(0, 4, 9), cx);

    replace(&editor, &capture, "very **swift** and *sly*", cx);

    assert_eq!(text(&editor, cx), "The very swift and sly brown fox");
    assert!(!attrs(&editor, 0, 4, cx).bold);
    assert!(attrs(&editor, 0, 9, cx).bold, "swift is bold");
    assert!(attrs(&editor, 0, 19, cx).italic, "sly is italic");
    assert!(
        attrs(&editor, 0, 23, cx).is_plain(),
        "the rest is as it was"
    );
    assert_eq!(selected_text(&editor, cx), "very swift and sly");
    assert_eq!(kinds(&editor, cx), vec![BlockKind::Paragraph]);
}

#[gpui::test]
fn inline_text_takes_the_formatting_of_what_it_replaces(cx: &mut TestAppContext) {
    setup(cx);
    let styled = InlineAttrs {
        italic: true,
        color: Some(Rgb::new(0x20, 0x40, 0x80)),
        font: Some("Georgia".into()),
        ..InlineAttrs::default()
    };
    let document = Document::from_blocks(vec![
        Block::new(BlockKind::heading(2))
            .with_span("A ", InlineAttrs::default())
            .with_span("quick fox", styled.clone())
            .with_span(" runs", InlineAttrs::default()),
    ]);
    let (editor, cx) = open_editor(document, cx);
    let capture = capture(&editor, TextRange::in_block(0, 2, 7), cx);

    replace(&editor, &capture, "**swift**", cx);

    assert_eq!(text(&editor, cx), "A swift fox runs");
    let put = attrs(&editor, 0, 3, cx);
    assert!(put.bold, "the answer's own emphasis");
    assert!(put.italic, "the italic of the text it replaced");
    assert_eq!(put.color, styled.color);
    assert_eq!(put.font, styled.font);
    // The block is still the heading it was.
    assert_eq!(kinds(&editor, cx), vec![BlockKind::heading(2)]);
}

#[gpui::test]
fn a_word_inside_a_link_stays_in_the_link(cx: &mut TestAppContext) {
    setup(cx);
    let link = InlineAttrs {
        link: Some("https://example.com".into()),
        ..InlineAttrs::default()
    };
    let document = Document::from_blocks(vec![
        Block::new(BlockKind::Paragraph)
            .with_span("See ", InlineAttrs::default())
            .with_span("the quick guide", link.clone())
            .with_span(" now", InlineAttrs::default()),
    ]);
    let (editor, cx) = open_editor(document.clone(), cx);
    let inside = capture(&editor, TextRange::in_block(0, 8, 13), cx);
    replace(&editor, &inside, "short", cx);
    assert_eq!(text(&editor, cx), "See the short guide now");
    assert_eq!(attrs(&editor, 0, 9, cx).link, link.link);

    // A selection that only starts in the link does not become one.
    editor.update(cx, |editor, cx| editor.set_document(document, cx));
    cx.run_until_parked();
    let across = capture(&editor, TextRange::in_block(0, 14, 23), cx);
    assert_eq!(across.text(), "guide now");
    replace(&editor, &across, "manual today", cx);
    assert_eq!(text(&editor, cx), "See the quick manual today");
    assert_eq!(attrs(&editor, 0, 16, cx).link, None);
}

#[gpui::test]
fn an_answer_of_several_paragraphs_replaces_as_blocks(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open_editor(paragraphs(&["Before.", "Old text.", "After."]), cx);
    let capture = capture(&editor, TextRange::in_block(1, 0, 9), cx);
    let depth = undo_depth(&editor, cx);

    let changed = replace(&editor, &capture, "## Heading\n\nOne.\n\n- a\n- b", cx);

    assert_eq!(text(&editor, cx), "Before.\nHeading\nOne.\na\nb\nAfter.");
    assert_eq!(
        kinds(&editor, cx),
        vec![
            BlockKind::Paragraph,
            BlockKind::heading(2),
            BlockKind::Paragraph,
            BlockKind::BulletItem,
            BlockKind::BulletItem,
            BlockKind::Paragraph,
        ]
    );
    let changed = changed.expect("replaced");
    assert_eq!(changed.start, Position::new(1, 0));
    assert_eq!(changed.end, Position::new(4, 1));
    assert_eq!(selected_text(&editor, cx), "Heading\nOne.\na\nb");
    assert_eq!(undo_depth(&editor, cx), depth + 1);

    undo(&editor, cx);
    assert_eq!(text(&editor, cx), "Before.\nOld text.\nAfter.");
    assert_eq!(kinds(&editor, cx), vec![BlockKind::Paragraph; 3]);
}

#[gpui::test]
fn a_selection_over_several_paragraphs_is_replaced(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open_editor(paragraphs(&["One two", "three four", "five six"]), cx);
    let capture = capture(
        &editor,
        TextRange::new(Position::new(0, 4), Position::new(2, 4)),
        cx,
    );
    assert_eq!(capture.text(), "two\nthree four\nfive");

    replace(&editor, &capture, "2 to 5,", cx);

    assert_eq!(text(&editor, cx), "One 2 to 5, six");
    assert_eq!(selected_text(&editor, cx), "2 to 5,");
    undo(&editor, cx);
    assert_eq!(text(&editor, cx), "One two\nthree four\nfive six");
}

#[gpui::test]
fn replace_is_refused_after_the_text_changed(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open_editor(paragraphs(&["The quick brown fox"]), cx);
    let capture = capture(&editor, TextRange::in_block(0, 4, 9), cx);
    assert!(editor.read_with(cx, |editor, _| can_replace(editor, &capture)));

    editor.update(cx, |editor, cx| {
        editor.edit(
            |state| {
                state.set_caret(Position::new(0, 6));
                state.insert_text("x")
            },
            cx,
        )
    });
    assert!(!editor.read_with(cx, |editor, _| can_replace(editor, &capture)));
    let depth = undo_depth(&editor, cx);

    assert_eq!(replace(&editor, &capture, "swift", cx), None);
    assert_eq!(text(&editor, cx), "The quxick brown fox");
    assert_eq!(undo_depth(&editor, cx), depth);

    // Taking the change back makes the text the captured one again.
    undo(&editor, cx);
    assert!(editor.read_with(cx, |editor, _| can_replace(editor, &capture)));
}

#[gpui::test]
fn replace_needs_a_selection(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open_editor(paragraphs(&["The quick brown fox"]), cx);
    let capture = capture(&editor, TextRange::empty(Position::new(0, 4)), cx);
    assert!(!editor.read_with(cx, |editor, _| can_replace(editor, &capture)));
    assert_eq!(replace(&editor, &capture, "swift", cx), None);
    assert_eq!(text(&editor, cx), "The quick brown fox");
}

#[gpui::test]
fn insert_below_adds_a_paragraph_after_the_block(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open_editor(paragraphs(&["The quick brown fox", "Next."]), cx);
    let capture = capture(&editor, TextRange::in_block(0, 4, 9), cx);
    let depth = undo_depth(&editor, cx);

    let inserted = below(&editor, &capture, "A *swift* one.", cx);

    assert_eq!(
        text(&editor, cx),
        "The quick brown fox\nA swift one.\nNext."
    );
    assert_eq!(inserted, Some(TextRange::in_block(1, 0, 12)));
    assert_eq!(selected_text(&editor, cx), "A swift one.");
    assert!(attrs(&editor, 1, 3, cx).has_style(InlineStyle::Italic));
    assert_eq!(kinds(&editor, cx), vec![BlockKind::Paragraph; 3]);
    assert_eq!(undo_depth(&editor, cx), depth + 1, "one undo step");

    undo(&editor, cx);
    assert_eq!(text(&editor, cx), "The quick brown fox\nNext.");
    redo(&editor, cx);
    assert_eq!(
        text(&editor, cx),
        "The quick brown fox\nA swift one.\nNext."
    );
}

#[gpui::test]
fn insert_below_from_a_caret_and_at_the_end(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open_editor(paragraphs(&["First.", "Last."]), cx);
    let capture = capture(&editor, TextRange::empty(Position::new(1, 2)), cx);
    assert!(editor.read_with(cx, |editor, _| can_insert(editor)));

    let inserted = below(&editor, &capture, "One.\n\nTwo.", cx);

    assert_eq!(text(&editor, cx), "First.\nLast.\nOne.\nTwo.");
    assert_eq!(
        inserted,
        Some(TextRange::new(Position::new(2, 0), Position::new(3, 4)))
    );
    assert_eq!(selected_text(&editor, cx), "One.\nTwo.");
    undo(&editor, cx);
    assert_eq!(text(&editor, cx), "First.\nLast.");
}

#[gpui::test]
fn insert_below_goes_after_the_last_block_of_a_selection(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open_editor(paragraphs(&["One", "Two", "Three"]), cx);
    let capture = capture(
        &editor,
        TextRange::new(Position::new(0, 1), Position::new(1, 2)),
        cx,
    );
    // The caret has moved since, and a paragraph was added on top.
    editor.update(cx, |editor, cx| {
        editor.edit(
            |state| {
                state.insert_blocks_at(
                    0,
                    &writer_core::Fragment::new(vec![Block::paragraph("Zero")]),
                )
            },
            cx,
        )
    });
    caret(&editor, 3, 0, cx);

    below(&editor, &capture, "After two.", cx);

    assert_eq!(text(&editor, cx), "Zero\nOne\nTwo\nAfter two.\nThree");
}

#[gpui::test]
fn insert_below_keeps_the_kinds_of_the_answer(cx: &mut TestAppContext) {
    setup(cx);
    let document = Document::from_blocks(vec![
        Block::with_text(BlockKind::BulletItem, "an item"),
        Block::paragraph("Next."),
    ]);
    let (editor, cx) = open_editor(document, cx);
    let capture = capture(&editor, TextRange::in_block(0, 0, 7), cx);

    below(&editor, &capture, "> A quote.\n\n```\ncode\n```", cx);

    assert_eq!(text(&editor, cx), "an item\nA quote.\ncode\nNext.");
    assert_eq!(
        kinds(&editor, cx),
        vec![
            BlockKind::BulletItem,
            BlockKind::Quote,
            BlockKind::CodeBlock { language: None },
            BlockKind::Paragraph,
        ]
    );
}

#[gpui::test]
fn an_empty_answer_changes_nothing(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open_editor(paragraphs(&["The quick brown fox"]), cx);
    let capture = capture(&editor, TextRange::in_block(0, 4, 9), cx);
    let depth = undo_depth(&editor, cx);
    assert!(answer_fragment("  \n\n", &Document::new()).is_empty());

    assert_eq!(replace(&editor, &capture, "  \n\n", cx), None);
    assert_eq!(below(&editor, &capture, "", cx), None);

    assert_eq!(text(&editor, cx), "The quick brown fox");
    assert_eq!(undo_depth(&editor, cx), depth);
}

#[gpui::test]
fn a_read_only_editor_is_left_alone(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open_editor(paragraphs(&["The quick brown fox"]), cx);
    let capture = capture(&editor, TextRange::in_block(0, 4, 9), cx);
    editor.update(cx, |editor, cx| editor.set_read_only(true, cx));

    assert!(!editor.read_with(cx, |editor, _| can_replace(editor, &capture)));
    assert!(!editor.read_with(cx, |editor, _| can_insert(editor)));
    assert_eq!(replace(&editor, &capture, "swift", cx), None);
    assert_eq!(below(&editor, &capture, "swift", cx), None);
    assert_eq!(text(&editor, cx), "The quick brown fox");
}

#[gpui::test]
fn replacing_inside_a_code_block_inserts_plain_text(cx: &mut TestAppContext) {
    setup(cx);
    let document = Document::from_blocks(vec![Block::with_text(
        BlockKind::CodeBlock { language: None },
        "let x = 1;",
    )]);
    let (editor, cx) = open_editor(document, cx);
    let capture = capture(&editor, TextRange::in_block(0, 4, 5), cx);

    replace(&editor, &capture, "**count**", cx);

    assert_eq!(text(&editor, cx), "let count = 1;");
    assert!(attrs(&editor, 0, 5, cx).is_plain());
}

#[gpui::test]
fn several_blocks_for_a_part_of_a_paragraph_split_it_around_them(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open_editor(paragraphs(&["Before. Old words. After.", "Next."]), cx);
    let capture = capture(&editor, TextRange::in_block(0, 8, 18), cx);
    assert_eq!(capture.text(), "Old words.");
    let depth = undo_depth(&editor, cx);

    let changed = replace(
        &editor,
        &capture,
        "First new paragraph.\n\n- one\n- two",
        cx,
    );

    // Whole blocks between the two halves; nothing is merged into them.
    assert_eq!(
        text(&editor, cx),
        "Before. \nFirst new paragraph.\none\ntwo\n After.\nNext."
    );
    assert_eq!(
        kinds(&editor, cx),
        vec![
            BlockKind::Paragraph,
            BlockKind::Paragraph,
            BlockKind::BulletItem,
            BlockKind::BulletItem,
            BlockKind::Paragraph,
            BlockKind::Paragraph,
        ]
    );
    assert_eq!(
        changed,
        Some(TextRange::new(Position::new(1, 0), Position::new(3, 3)))
    );
    assert_eq!(selected_text(&editor, cx), "First new paragraph.\none\ntwo");
    assert_eq!(undo_depth(&editor, cx), depth + 1, "one undo step");
    undo(&editor, cx);
    assert_eq!(text(&editor, cx), "Before. Old words. After.\nNext.");
    assert_eq!(kinds(&editor, cx), vec![BlockKind::Paragraph; 2]);
}

#[gpui::test]
fn a_heading_for_a_whole_paragraph_makes_it_a_heading(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open_editor(paragraphs(&["Before.", "about the harbour", "After."]), cx);
    let capture = capture(&editor, TextRange::in_block(1, 0, 17), cx);

    let changed = replace(&editor, &capture, "## The Harbour", cx);

    assert_eq!(text(&editor, cx), "Before.\nThe Harbour\nAfter.");
    assert_eq!(
        kinds(&editor, cx),
        vec![
            BlockKind::Paragraph,
            BlockKind::heading(2),
            BlockKind::Paragraph
        ]
    );
    assert_eq!(changed, Some(TextRange::in_block(1, 0, 11)));
    assert_eq!(selected_text(&editor, cx), "The Harbour");
    undo(&editor, cx);
    assert_eq!(text(&editor, cx), "Before.\nabout the harbour\nAfter.");
    assert_eq!(kinds(&editor, cx), vec![BlockKind::Paragraph; 3]);
}

#[gpui::test]
fn a_paragraph_for_a_whole_heading_leaves_it_a_heading(cx: &mut TestAppContext) {
    setup(cx);
    let document = Document::from_blocks(vec![
        Block::with_text(BlockKind::heading(1), "A very long title of the chapter"),
        Block::paragraph("Text."),
    ]);
    let (editor, cx) = open_editor(document, cx);
    let capture = capture(&editor, TextRange::in_block(0, 0, 32), cx);

    replace(&editor, &capture, "A short title", cx);

    assert_eq!(text(&editor, cx), "A short title\nText.");
    assert_eq!(
        kinds(&editor, cx),
        vec![BlockKind::heading(1), BlockKind::Paragraph]
    );
    assert_eq!(selected_text(&editor, cx), "A short title");
}

#[gpui::test]
fn one_block_of_another_kind_for_a_few_words_stays_in_the_paragraph(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open_editor(paragraphs(&["The quick brown fox"]), cx);
    let capture = capture(&editor, TextRange::in_block(0, 4, 9), cx);

    // The assistant answered with a list of one.
    replace(&editor, &capture, "- swift", cx);

    assert_eq!(text(&editor, cx), "The swift brown fox");
    assert_eq!(kinds(&editor, cx), vec![BlockKind::Paragraph]);
    assert_eq!(selected_text(&editor, cx), "swift");
}

#[gpui::test]
fn whole_paragraphs_are_replaced_by_the_blocks_of_the_answer(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open_editor(paragraphs(&["Keep.", "one, two", "three", "Keep too."]), cx);
    let capture = capture(
        &editor,
        TextRange::new(Position::new(1, 0), Position::new(2, 5)),
        cx,
    );

    let changed = replace(&editor, &capture, "1. one\n2. two\n3. three", cx);

    assert_eq!(text(&editor, cx), "Keep.\none\ntwo\nthree\nKeep too.");
    assert_eq!(
        kinds(&editor, cx),
        vec![
            BlockKind::Paragraph,
            BlockKind::NumberedItem,
            BlockKind::NumberedItem,
            BlockKind::NumberedItem,
            BlockKind::Paragraph,
        ]
    );
    assert_eq!(
        changed,
        Some(TextRange::new(Position::new(1, 0), Position::new(3, 5)))
    );
    undo(&editor, cx);
    assert_eq!(text(&editor, cx), "Keep.\none, two\nthree\nKeep too.");
}

#[gpui::test]
fn the_last_paragraph_of_a_document_can_be_replaced_by_blocks(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open_editor(paragraphs(&["Only this."]), cx);
    let capture = capture(&editor, TextRange::in_block(0, 0, 10), cx);

    replace(&editor, &capture, "One.\n\nTwo.", cx);

    assert_eq!(text(&editor, cx), "One.\nTwo.");
    assert_eq!(selected_text(&editor, cx), "One.\nTwo.");
    undo(&editor, cx);
    assert_eq!(text(&editor, cx), "Only this.");
}

#[gpui::test]
fn an_answer_that_opens_with_a_rule_keeps_it(cx: &mut TestAppContext) {
    setup(cx);
    let (editor, cx) = open_editor(paragraphs(&["First.", "Last."]), cx);
    let capture = capture(&editor, TextRange::empty(Position::new(0, 2)), cx);

    // Text that a Markdown reader could take for data about the text.
    below(
        &editor,
        &capture,
        "---\ntitle: A letter\n---\n\nDear Anna,",
        cx,
    );

    let all = text(&editor, cx);
    assert!(
        all.contains("title: A letter"),
        "nothing was left out: {all:?}"
    );
    assert!(all.contains("Dear Anna,"));
    assert!(all.starts_with("First.\n") && all.ends_with("\nLast."));
}

// ----- images ------------------------------------------------------------------------------

/// A document with an image between two paragraphs: half the column wide,
/// at the right, with a description.
fn with_an_image() -> (Document, writer_core::Image) {
    use writer_core::{Alignment, Image, ImageId, ImageWidth};
    let mut image = Image::new(ImageId::from_bytes([7; 16]), (1600, 1067), 144);
    image.width = ImageWidth::from_percent(50);
    image.description = "The harbour at dusk".to_string();
    let document = Document::from_blocks(vec![
        Block::paragraph("Before the picture."),
        Block::image(image.clone()).with_alignment(Alignment::Right),
        Block::paragraph("After the picture."),
    ]);
    (document, image)
}

#[gpui::test]
fn an_answer_that_names_an_image_of_the_document_brings_it_back_as_it_was(
    cx: &mut TestAppContext,
) {
    setup(cx);
    let (document, image) = with_an_image();
    let (editor, cx) = open_editor(document, cx);
    // Everything is selected, the image among it.
    let end = Position::new(2, "After the picture.".len());
    let capture = capture(&editor, TextRange::new(Position::new(0, 0), end), cx);

    // The answer puts the image first and rewrites the two paragraphs.
    let answer = format!(
        "![The harbour at dusk](image:{})\n\nThe boats came in.\n\nThen it was dark.",
        image.id
    );
    assert!(replace(&editor, &capture, &answer, cx).is_some());

    editor.read_with(cx, |editor, _| {
        let document = editor.document();
        let block = document.block(0).expect("the first block");
        assert_eq!(
            block.as_image(),
            Some(&image),
            "the image is the document's own again: its id, its width, its description"
        );
        assert_eq!(block.alignment(), writer_core::Alignment::Right);
        assert_eq!(document.block(1).map(|block| block.text()), Some("The boats came in."));
        assert_eq!(document.block(2).map(|block| block.text()), Some("Then it was dark."));
        assert_eq!(document.block_count(), 3);
    });
}

#[gpui::test]
fn an_answer_never_makes_an_image_of_a_file_or_of_an_id_the_document_does_not_have(
    cx: &mut TestAppContext,
) {
    setup(cx);
    let (document, _) = with_an_image();
    let (editor, cx) = open_editor(document, cx);
    let capture = capture(&editor, TextRange::in_block(0, 0, 6), cx);

    // A path, an address, and the id of an image this document does not
    // hold: none of them becomes an image block.
    for destination in [
        "/etc/hosts",
        "file:///etc/hosts",
        "https://example.com/a.png",
        "image:00112233445566778899aabbccddeeff",
    ] {
        let fragment = editor.read_with(cx, |editor, _| {
            answer_fragment(&format!("![x]({destination})"), editor.document())
        });
        assert!(
            fragment.blocks().iter().all(|block| block.as_image().is_none()),
            "{destination} became an image"
        );
        assert!(!fragment.is_empty(), "{destination} left nothing");
    }

    // Inserted below, the path is a link in the text and nothing else.
    assert!(below(&editor, &capture, "![x](/etc/hosts)", cx).is_some());
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().images().count(), 1, "only the image that was there");
        let inserted = editor.document().block(1).expect("the inserted block");
        assert!(inserted.as_image().is_none());
        let linked = inserted
            .text_spans()
            .any(|(_, _, attrs)| attrs.link.as_deref() == Some("/etc/hosts"));
        assert!(linked, "the path stays a link: {:?}", inserted.text());
    });
}
