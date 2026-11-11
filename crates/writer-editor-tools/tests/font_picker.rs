use gpui::{TestAppContext, VisualTestContext};
use writer_core::{Block, BlockKind, Document, InlineAttrs, TextRange};
use writer_editor_tools::{
    Mark, RECENT_FONT_LIMIT, ToolKind, recent_fonts, set_font_families, set_recent_fonts,
};

use crate::support::*;

/// A few named families among several hundred generated ones.
fn install_fonts(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let mut families: Vec<String> = (0..600).map(|n| format!("Family {n:03}")).collect();
        families.extend(
            [
                "Avenir Next",
                "Baskerville",
                "Charter",
                "Georgia",
                "Helvetica Neue",
                "Menlo",
            ]
            .map(String::from),
        );
        families.sort();
        set_font_families(families, cx);
    });
}

fn font_at(
    editor: &gpui::Entity<writer_editor::Editor>,
    offset: usize,
    cx: &mut VisualTestContext,
) -> Option<String> {
    editor.read_with(cx, |editor, _| {
        editor.document().blocks()[0]
            .attrs_at(offset)
            .and_then(|attrs| attrs.font.clone())
    })
}

#[gpui::test]
fn lists_filters_and_applies_to_the_selection(cx: &mut TestAppContext) {
    setup(cx);
    install_fonts(cx);
    let (tools, editor, cx) = open(paragraphs(&["Set this in another face"]), cx);
    select(&editor, TextRange::in_block(0, 4, 8), cx);

    cx.simulate_keystrokes("cmd-t");
    assert_eq!(active(&tools, cx), Some(ToolKind::Font));
    assert!(!editor_focused(&editor, cx));
    let all = rows(&tools, cx);
    assert_eq!(all[0].label, "Document default");
    assert_eq!(all[0].detail.as_deref(), Some("Charter"));
    assert_eq!(
        all[0].mark,
        Mark::Check,
        "the selection has no font of its own"
    );
    assert!(all[0].selected);
    assert!(
        all[1].header && all[1].label == "All fonts",
        "no recent fonts yet"
    );
    assert_eq!(all.len(), 2 + 606);

    // Fuzzy filter over hundreds of families.
    let started = std::time::Instant::now();
    cx.simulate_input("bask");
    eprintln!(
        "font filter keystrokes over 606 families: {:?}",
        started.elapsed()
    );
    assert_eq!(selected(&tools, cx).as_deref(), Some("Baskerville"));
    assert!(rows(&tools, cx).iter().all(|row| !row.header));
    cx.simulate_keystrokes("cmd-a backspace");
    cx.simulate_input("hlvnue");
    assert_eq!(selected(&tools, cx).as_deref(), Some("Helvetica Neue"));

    cx.simulate_keystrokes("enter");
    assert_eq!(active(&tools, cx), None);
    assert!(editor_focused(&editor, cx));
    assert_eq!(font_at(&editor, 4, cx).as_deref(), Some("Helvetica Neue"));
    assert_eq!(font_at(&editor, 7, cx).as_deref(), Some("Helvetica Neue"));
    assert_eq!(font_at(&editor, 3, cx), None);
    assert_eq!(font_at(&editor, 8, cx), None);

    // Next time: recent comes right after the default, the current family
    // is marked and preselected.
    cx.simulate_keystrokes("cmd-t");
    let all = rows(&tools, cx);
    assert_eq!(all[0].label, "Document default");
    assert_eq!(all[0].mark, Mark::None);
    assert!(all[1].header && all[1].label == "Recent");
    assert_eq!(all[2].label, "Helvetica Neue");
    assert_eq!(all[2].mark, Mark::Check);
    assert!(all[2].selected);
    assert!(all[3].header && all[3].label == "All fonts");

    // "Document default" takes the family off again.
    cx.simulate_keystrokes("up enter");
    assert_eq!(font_at(&editor, 4, cx), None);
    // Two undo steps: clearing, then applying.
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(font_at(&editor, 4, cx).as_deref(), Some("Helvetica Neue"));
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(font_at(&editor, 4, cx), None);
}

#[gpui::test]
fn a_collapsed_caret_sets_the_font_of_the_next_text(cx: &mut TestAppContext) {
    setup(cx);
    install_fonts(cx);
    let (tools, editor, cx) = open(paragraphs(&["Plain "]), cx);
    caret(&editor, 0, 6, cx);

    cx.simulate_keystrokes("cmd-t");
    cx.simulate_input("menlo");
    cx.simulate_keystrokes("enter");
    assert_eq!(active(&tools, cx), None);
    assert_eq!(text(&editor, cx), "Plain ", "nothing is inserted");
    cx.simulate_input("mono");
    assert_eq!(text(&editor, cx), "Plain mono");
    assert_eq!(font_at(&editor, 6, cx).as_deref(), Some("Menlo"));
    assert_eq!(font_at(&editor, 9, cx).as_deref(), Some("Menlo"));
    assert_eq!(font_at(&editor, 0, cx), None);
}

#[gpui::test]
fn cmd_enter_sets_the_document_font(cx: &mut TestAppContext) {
    setup(cx);
    install_fonts(cx);
    let (tools, editor, cx) = open(paragraphs(&["Body text"]), cx);
    select(&editor, TextRange::in_block(0, 0, 4), cx);
    let body_font = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |editor, _| {
            editor.document().settings().typography.body_font.clone()
        })
    };
    assert_eq!(body_font(cx), "Charter");

    cx.simulate_keystrokes("cmd-t");
    cx.simulate_input("georgia");
    cx.simulate_keystrokes("cmd-enter");
    assert_eq!(active(&tools, cx), None);
    assert!(editor_focused(&editor, cx));
    assert_eq!(body_font(cx), "Georgia");
    assert_eq!(
        font_at(&editor, 0, cx),
        None,
        "the selection keeps no font of its own"
    );

    // The default row shows the document font and resets it with cmd-enter.
    cx.simulate_keystrokes("cmd-t");
    assert_eq!(rows(&tools, cx)[0].label, "Document default");
    cx.simulate_keystrokes("cmd-a backspace");
    cx.simulate_input("document def");
    assert_eq!(selected(&tools, cx).as_deref(), Some("Document default"));
    cx.simulate_keystrokes("cmd-enter");
    assert_eq!(body_font(cx), "Charter");
}

#[gpui::test]
fn recent_fonts_are_kept_short_and_escape_changes_nothing(cx: &mut TestAppContext) {
    setup(cx);
    install_fonts(cx);
    cx.update(|cx| set_recent_fonts(["Georgia", "Not Installed", "Menlo", "Georgia"], cx));
    let document = Document::from_blocks(vec![
        Block::new(BlockKind::Paragraph)
            .with_span(
                "serif",
                InlineAttrs {
                    font: Some("Georgia".into()),
                    ..InlineAttrs::default()
                },
            )
            .with_span(" plain", InlineAttrs::default()),
    ]);
    let (tools, editor, cx) = open(document, cx);
    select(&editor, TextRange::in_block(0, 0, 11), cx);

    // Mixed fonts: nothing is marked. Recent families that are not
    // installed are left out.
    cx.simulate_keystrokes("cmd-t");
    let all = rows(&tools, cx);
    assert_eq!(all[0].mark, Mark::None);
    assert_eq!(all[2].label, "Georgia");
    assert_eq!(all[3].label, "Menlo");
    assert!(all[4].header);
    assert!(all.iter().all(|row| row.mark == Mark::None));

    cx.simulate_keystrokes("down down escape");
    assert_eq!(active(&tools, cx), None);
    assert!(editor_focused(&editor, cx));
    assert_eq!(font_at(&editor, 0, cx).as_deref(), Some("Georgia"));
    assert_eq!(font_at(&editor, 6, cx), None);

    // Applying families keeps the recent list at its limit, newest first.
    for n in 0..RECENT_FONT_LIMIT + 2 {
        cx.simulate_keystrokes("cmd-t");
        cx.simulate_input(&format!("family 0{n:02}"));
        cx.simulate_keystrokes("enter");
    }
    let recent = cx.update(|_, cx| recent_fonts(cx));
    assert_eq!(recent.len(), RECENT_FONT_LIMIT);
    assert_eq!(
        recent[0].as_ref(),
        format!("Family 0{:02}", RECENT_FONT_LIMIT + 1)
    );
}
