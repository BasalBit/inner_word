use gpui::{TestAppContext, VisualTestContext};
use innerword_core::{Block, BlockKind, Document, InlineAttrs, TextRange};
use innerword_editor_tools::{
    Mark, RECENT_FONT_LIMIT, ToolKind, WRITING_FONTS, recent_fonts, set_font_families,
    set_recent_fonts,
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
    editor: &gpui::Entity<innerword_editor::Editor>,
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
    // The installed writing faces lead, in the picker's own order; then
    // everything. No recent fonts yet.
    assert!(all[1].header && all[1].label == "Writing fonts");
    let writing: Vec<&str> = all[2..8].iter().map(|row| row.label.as_str()).collect();
    assert_eq!(
        writing,
        [
            "Charter",
            "Georgia",
            "Baskerville",
            "Avenir Next",
            "Helvetica Neue",
            "Menlo"
        ]
    );
    assert!(writing.iter().all(|name| WRITING_FONTS.contains(name)));
    assert!(all[8].header && all[8].label == "All fonts");
    assert_eq!(all.len(), 3 + 6 + 606);

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

    // Next time the current family is marked and preselected where it
    // comes first. A writing font is not repeated under "Recent".
    cx.simulate_keystrokes("cmd-t");
    let all = rows(&tools, cx);
    assert_eq!(all[0].label, "Document default");
    assert_eq!(all[0].mark, Mark::None);
    assert!(all[1].header && all[1].label == "Writing fonts");
    assert_eq!(all[6].label, "Helvetica Neue");
    assert_eq!(all[6].mark, Mark::Check);
    assert!(all[6].selected);
    assert!(all[8].header && all[8].label == "All fonts");
    assert_eq!(
        cx.update(|_, cx| recent_fonts(cx)),
        ["Helvetica Neue"],
        "it is remembered all the same"
    );

    // "Document default" takes the family off again.
    cx.simulate_keystrokes("up up up up up enter");
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
    cx.update(|cx| {
        set_recent_fonts(
            [
                "Family 300",
                "Not Installed",
                "Georgia",
                "Family 200",
                "Family 300",
            ],
            cx,
        )
    });
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
    // installed are left out, and so is one that is already in view among
    // the writing fonts (Georgia).
    cx.simulate_keystrokes("cmd-t");
    let all = rows(&tools, cx);
    assert_eq!(all[0].mark, Mark::None);
    assert!(all[1].header && all[1].label == "Writing fonts");
    assert!(all[8].header && all[8].label == "Recent");
    assert_eq!(all[9].label, "Family 300");
    assert_eq!(all[10].label, "Family 200");
    assert!(all[11].header && all[11].label == "All fonts");
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

#[gpui::test]
fn typing_after_a_font_was_applied_to_a_selection(cx: &mut TestAppContext) {
    setup(cx);
    install_fonts(cx);
    let (_tools, editor, cx) = open(paragraphs(&["Set this in another face"]), cx);
    select(&editor, TextRange::in_block(0, 4, 8), cx);

    cx.simulate_keystrokes("cmd-t");
    cx.simulate_input("menlo");
    cx.simulate_keystrokes("enter");
    // The selection is still on the word, now in Menlo.
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.selection().range(), TextRange::in_block(0, 4, 8));
    });
    let fonts = |cx: &mut VisualTestContext| -> String {
        // One letter per character: M for Menlo, a dot for the default.
        let len = editor.read_with(cx, |editor, _| editor.document().blocks()[0].len());
        (0..len)
            .map(|offset| match font_at(&editor, offset, cx).as_deref() {
                Some("Menlo") => 'M',
                Some(_) => '?',
                None => '.',
            })
            .collect()
    };
    assert_eq!(fonts(cx), "....MMMM................");

    // Typing over the selection keeps the font that was just chosen.
    cx.simulate_input("that");
    assert_eq!(text(&editor, cx), "Set that in another face");
    assert_eq!(fonts(cx), "....MMMM................");
    // Typing on at its end stays in the font.
    cx.simulate_input("s");
    assert_eq!(text(&editor, cx), "Set thats in another face");
    assert_eq!(fonts(cx), "....MMMMM................");
    // Coming back to the end of the run from the right, too (text takes
    // the font of the character before the caret)...
    cx.simulate_keystrokes("right left");
    cx.simulate_input("x");
    assert_eq!(fonts(cx), "....MMMMMM................");
    // ...and text typed elsewhere does not.
    cx.simulate_keystrokes("cmd-left");
    cx.simulate_input("y");
    assert_eq!(text(&editor, cx), "ySet thatsx in another face");
    assert_eq!(fonts(cx), ".....MMMMMM................");
    cx.simulate_keystrokes("cmd-right");
    cx.simulate_input("!");
    assert_eq!(fonts(cx), ".....MMMMMM.................");

    // Applying the font and typing over it are two undo steps.
    let (_, editor, cx) = open(paragraphs(&["Set this in another face"]), cx);
    select(&editor, TextRange::in_block(0, 4, 8), cx);
    cx.simulate_keystrokes("cmd-t");
    cx.simulate_input("menlo");
    cx.simulate_keystrokes("enter");
    cx.simulate_input("that");
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(text(&editor, cx), "Set this in another face");
    assert_eq!(font_at(&editor, 4, cx).as_deref(), Some("Menlo"));
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(font_at(&editor, 4, cx), None);
}

#[gpui::test]
fn the_list_of_fonts_is_what_the_picker_shows(cx: &mut TestAppContext) {
    use innerword_editor_tools::{FontList, FontRow};

    setup(cx);
    cx.update(|cx| {
        set_font_families(["Charter", "Georgia", "Menlo", "Papyrus", "Zapfino"], cx);
        set_recent_fonts(["Zapfino", "Georgia"], cx);
    });
    let list = cx.update(FontList::current);
    assert_eq!(list.position("Menlo"), Some(2));
    assert_eq!(list.position("Sabon Next"), None);
    let name = |row: &FontRow, leading: &[&str]| match row {
        FontRow::Header(title) => title.to_string(),
        FontRow::Leading { index, .. } => leading[*index].to_string(),
        FontRow::Family { index, .. } => list.families()[*index].to_string(),
    };
    let names = |leading: &[&str], query: &str| -> Vec<String> {
        list.rows(leading, query)
            .iter()
            .map(|row| name(row, leading))
            .collect()
    };

    // Without a query: the groups under their headings. A recent family
    // that is a writing font is not said twice.
    let groups = [
        "Writing fonts",
        "Charter",
        "Georgia",
        "Menlo",
        "Recent",
        "Zapfino",
        "All fonts",
        "Charter",
        "Georgia",
        "Menlo",
        "Papyrus",
        "Zapfino",
    ];
    assert_eq!(names(&[], ""), groups);
    // Rows that are not installed families lead.
    let with_leading = names(&["Sabon Next"], "");
    assert_eq!(with_leading[0], "Sabon Next");
    assert_eq!(with_leading[1..], groups);
    // With a query: the matches, the leading rows among them, no headings.
    assert_eq!(names(&["Sabon Next"], "sab"), ["Sabon Next"]);
    assert_eq!(names(&["Sabon Next"], "zap"), ["Zapfino"]);
    assert!(matches!(
        list.rows(&[], "zap")[0],
        FontRow::Family { index: 4, ref positions } if positions == &[0, 1, 2]
    ));

    // The font picker of the text shows exactly these rows after its own
    // "Document default".
    let (tools, _editor, cx) = open(paragraphs(&["Text"]), cx);
    cx.simulate_keystrokes("cmd-t");
    let shown: Vec<String> = rows(&tools, cx).into_iter().map(|row| row.label).collect();
    assert_eq!(shown[0], "Document default");
    assert_eq!(shown[1..], groups);
    cx.simulate_input("zap");
    let shown: Vec<String> = rows(&tools, cx).into_iter().map(|row| row.label).collect();
    assert_eq!(shown, names(&["Document default"], "zap"));
}
