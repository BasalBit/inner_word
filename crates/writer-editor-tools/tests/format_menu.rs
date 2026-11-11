use gpui::{Hsla, Rgba, TestAppContext};
use writer_core::{
    Alignment, Block, BlockKind, Document, Highlight, InlineAttrs, Position, TextRange,
};
use writer_editor_tools::{Mark, TEXT_COLORS, ToolKind};
use writer_ui::theme::{self, ActiveTheme, ThemeMode};

use crate::support::*;

#[gpui::test]
fn opens_with_inline_formatting_first_on_a_selection(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(paragraphs(&["Hello brave world"]), cx);
    select(&editor, TextRange::in_block(0, 6, 11), cx);

    cx.simulate_keystrokes("cmd-.");
    assert_eq!(active(&tools, cx), Some(ToolKind::Format));
    assert!(
        !editor_focused(&editor, cx),
        "focus moves into the menu's filter"
    );

    let rows = rows(&tools, cx);
    assert!(rows[0].header && rows[0].label == "Text");
    assert_eq!(rows[1].label, "Bold");
    assert!(rows[1].selected);
    // The real binding from the keymap.
    assert_eq!(rows[1].key_hint.as_deref(), Some("⌘B"));
    let labels = labels(&tools, cx);
    for expected in [
        "Bold",
        "Italic",
        "Underline",
        "Strikethrough",
        "Inline code",
        "Link",
        "Font",
        "Clear formatting",
        "Yellow highlight",
        "Purple highlight",
        "Default text colour",
        "Red text",
        "Paragraph",
        "Heading 1",
        "Heading 6",
        "Bullet list",
        "Numbered list",
        "To-do list",
        "Quote",
        "Code block",
        "Align left",
        "Align centre",
        "Align right",
        "Justify",
        "Larger text",
        "Smaller text",
        "Reset text size",
    ] {
        assert!(
            labels.iter().any(|label| label == expected),
            "missing {expected}"
        );
    }
    let position = |label: &str| labels.iter().position(|other| other == label).unwrap();
    assert!(position("Bold") < position("Heading 1"));
    assert_eq!(
        row(&tools, "Heading 1", cx).key_hint.as_deref(),
        Some("⌥⌘1")
    );
    assert_eq!(row(&tools, "Link", cx).key_hint.as_deref(), Some("⌘K"));
    assert_eq!(row(&tools, "Font", cx).key_hint.as_deref(), Some("⌘T"));

    // Enter applies the selected row and closes; focus is back in the editor.
    cx.simulate_keystrokes("enter");
    assert_eq!(active(&tools, cx), None);
    assert!(editor_focused(&editor, cx));
    editor.read_with(cx, |editor, _| {
        let block = &editor.document().blocks()[0];
        assert!(block.attrs_at(6).unwrap().bold);
        assert!(!block.attrs_at(0).unwrap().bold);
        assert!(!block.attrs_at(11).unwrap().bold);
    });

    // One undo step.
    cx.simulate_keystrokes("cmd-z");
    editor.read_with(cx, |editor, _| {
        assert!(!editor.document().blocks()[0].attrs_at(6).unwrap().bold);
    });
}

#[gpui::test]
fn leads_with_block_options_on_a_collapsed_caret(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(paragraphs(&["Title", "Body"]), cx);

    cx.simulate_keystrokes("cmd-.");
    let rows = rows(&tools, cx);
    assert!(rows[0].header && rows[0].label == "Turn into");
    assert_eq!(rows[1].label, "Paragraph");
    assert_eq!(
        rows[1].mark,
        Mark::Check,
        "the caret's block is a paragraph"
    );
    let labels = labels(&tools, cx);
    let position = |label: &str| labels.iter().position(|other| other == label).unwrap();
    assert!(position("Heading 1") < position("Align left"));
    assert!(position("Reset text size") < position("Bold"));
    assert_eq!(row(&tools, "Align left", cx).mark, Mark::Check);

    // Arrows move, enter applies: down twice from Paragraph is Heading 2.
    cx.simulate_keystrokes("down down");
    assert_eq!(selected(&tools, cx).as_deref(), Some("Heading 2"));
    cx.simulate_keystrokes("up");
    assert_eq!(selected(&tools, cx).as_deref(), Some("Heading 1"));
    cx.simulate_keystrokes("enter");
    editor.read_with(cx, |editor, _| {
        assert_eq!(
            editor.document().blocks()[0].kind().heading_level(),
            Some(1)
        );
        assert_eq!(editor.document().blocks()[1].kind(), &BlockKind::Paragraph);
    });
    assert!(editor_focused(&editor, cx));

    // The state is read again on the next open.
    cx.simulate_keystrokes("cmd-.");
    assert_eq!(row(&tools, "Heading 1", cx).mark, Mark::Check);
    assert_eq!(row(&tools, "Paragraph", cx).mark, Mark::None);
    cx.simulate_keystrokes("escape");
    assert_eq!(active(&tools, cx), None);
    assert!(editor_focused(&editor, cx));
}

#[gpui::test]
fn typing_filters_and_escape_closes(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(paragraphs(&["One", "Two"]), cx);
    caret(&editor, 1, 0, cx);

    cx.simulate_keystrokes("cmd-.");
    cx.simulate_input("quo");
    let filtered = labels(&tools, cx);
    assert_eq!(filtered.first().map(String::as_str), Some("Quote"));
    assert!(
        rows(&tools, cx).iter().all(|row| !row.header),
        "no headers while filtering"
    );
    // The text went to the filter, not to the document.
    assert_eq!(text(&editor, cx), "One\nTwo");

    // Keywords match too: "h3" finds Heading 3.
    cx.simulate_keystrokes("cmd-a backspace");
    cx.simulate_input("h3");
    assert_eq!(selected(&tools, cx).as_deref(), Some("Heading 3"));

    cx.simulate_keystrokes("cmd-a backspace");
    cx.simulate_input("zzzz");
    assert!(labels(&tools, cx).is_empty());
    // Enter with nothing selected does nothing.
    cx.simulate_keystrokes("enter");
    assert_eq!(active(&tools, cx), Some(ToolKind::Format));

    cx.simulate_keystrokes("escape");
    assert_eq!(active(&tools, cx), None);
    assert!(editor_focused(&editor, cx));
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().blocks()[1].kind(), &BlockKind::Paragraph);
        assert!(!editor.has_popup());
    });

    // Filter then apply.
    cx.simulate_keystrokes("cmd-.");
    cx.simulate_input("quote");
    cx.simulate_keystrokes("enter");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().blocks()[1].kind(), &BlockKind::Quote);
        assert_eq!(editor.document().blocks()[0].kind(), &BlockKind::Paragraph);
    });
}

#[gpui::test]
fn shows_checks_and_mixed_state(cx: &mut TestAppContext) {
    setup(cx);
    let bold = InlineAttrs {
        bold: true,
        ..InlineAttrs::default()
    };
    let yellow = InlineAttrs {
        highlight: Some(Highlight::Yellow),
        ..InlineAttrs::default()
    };
    let document = Document::from_blocks(vec![
        Block::new(BlockKind::heading(2))
            .with_span("bold", bold.clone())
            .with_span(" plain", InlineAttrs::default()),
        Block::new(BlockKind::Paragraph)
            .with_span("marked", yellow)
            .with_alignment(Alignment::Center),
    ]);
    let (tools, editor, cx) = open(document, cx);

    // All bold.
    select(&editor, TextRange::in_block(0, 0, 4), cx);
    cx.simulate_keystrokes("cmd-.");
    assert_eq!(row(&tools, "Bold", cx).mark, Mark::Check);
    assert_eq!(row(&tools, "Italic", cx).mark, Mark::None);
    assert_eq!(row(&tools, "Heading 2", cx).mark, Mark::Check);
    cx.simulate_keystrokes("escape");

    // Bold in a part of the selection, two block kinds, two alignments,
    // highlight in a part.
    select(
        &editor,
        TextRange::new(
            writer_core::Position::new(0, 0),
            writer_core::Position::new(1, 6),
        ),
        cx,
    );
    cx.simulate_keystrokes("cmd-.");
    assert_eq!(row(&tools, "Bold", cx).mark, Mark::Mixed);
    assert_eq!(row(&tools, "Heading 2", cx).mark, Mark::Mixed);
    assert_eq!(row(&tools, "Paragraph", cx).mark, Mark::Mixed);
    assert_eq!(row(&tools, "Heading 1", cx).mark, Mark::None);
    assert_eq!(row(&tools, "Align left", cx).mark, Mark::Mixed);
    assert_eq!(row(&tools, "Align centre", cx).mark, Mark::Mixed);
    assert_eq!(row(&tools, "Align right", cx).mark, Mark::None);
    assert_eq!(row(&tools, "Yellow highlight", cx).mark, Mark::Mixed);
    assert_eq!(row(&tools, "Green highlight", cx).mark, Mark::None);
    cx.simulate_keystrokes("escape");

    select(&editor, TextRange::in_block(1, 0, 6), cx);
    cx.simulate_keystrokes("cmd-.");
    assert_eq!(row(&tools, "Yellow highlight", cx).mark, Mark::Check);
    assert_eq!(row(&tools, "Default text colour", cx).mark, Mark::Check);
    assert_eq!(row(&tools, "Align centre", cx).mark, Mark::Check);
}

#[gpui::test]
fn applies_highlight_and_text_colours(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(paragraphs(&["Colour me in"]), cx);
    select(&editor, TextRange::in_block(0, 0, 6), cx);

    cx.simulate_keystrokes("cmd-.");
    cx.simulate_input("green hi");
    assert_eq!(selected(&tools, cx).as_deref(), Some("Green highlight"));
    cx.simulate_keystrokes("enter");
    editor.read_with(cx, |editor, _| {
        let block = &editor.document().blocks()[0];
        assert_eq!(block.attrs_at(0).unwrap().highlight, Some(Highlight::Green));
        assert_eq!(block.attrs_at(7).unwrap().highlight, None);
    });

    cx.simulate_keystrokes("cmd-.");
    assert_eq!(row(&tools, "Green highlight", cx).mark, Mark::Check);
    cx.simulate_input("blue text");
    assert_eq!(selected(&tools, cx).as_deref(), Some("Blue text"));
    cx.simulate_keystrokes("enter");
    let blue = TEXT_COLORS
        .iter()
        .find(|(name, _)| *name == "Blue")
        .unwrap()
        .1;
    editor.read_with(cx, |editor, _| {
        assert_eq!(
            editor.document().blocks()[0].attrs_at(0).unwrap().color,
            Some(blue)
        );
    });

    // Choosing the checked highlight again removes it (a toggle).
    cx.simulate_keystrokes("cmd-.");
    assert_eq!(row(&tools, "Blue text", cx).mark, Mark::Check);
    cx.simulate_input("green hi");
    cx.simulate_keystrokes("enter");
    editor.read_with(cx, |editor, _| {
        assert_eq!(
            editor.document().blocks()[0].attrs_at(0).unwrap().highlight,
            None
        );
    });

    // Back to the default colour.
    cx.simulate_keystrokes("cmd-.");
    cx.simulate_input("default");
    cx.simulate_keystrokes("enter");
    editor.read_with(cx, |editor, _| {
        assert_eq!(
            editor.document().blocks()[0].attrs_at(0).unwrap().color,
            None
        );
    });
    assert!(editor_focused(&editor, cx));
}

#[gpui::test]
fn shortcut_toggles_and_read_only_blocks(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(paragraphs(&["Text"]), cx);

    cx.simulate_keystrokes("cmd-.");
    assert_eq!(active(&tools, cx), Some(ToolKind::Format));
    editor.read_with(cx, |editor, _| assert!(editor.has_popup()));
    // The same shortcut closes it again.
    cx.simulate_keystrokes("cmd-.");
    assert_eq!(active(&tools, cx), None);
    assert!(editor_focused(&editor, cx));

    editor.update(cx, |editor, cx| editor.set_read_only(true, cx));
    cx.simulate_keystrokes("cmd-.");
    assert_eq!(active(&tools, cx), None);
}

#[gpui::test]
fn rows_show_current_values(cx: &mut TestAppContext) {
    setup(cx);
    let serif = InlineAttrs {
        font: Some("Georgia".into()),
        ..InlineAttrs::default()
    };
    let document = Document::from_blocks(vec![
        Block::new(BlockKind::Paragraph)
            .with_span("serif", serif)
            .with_span(" plain", InlineAttrs::default()),
    ]);
    let (tools, editor, cx) = open(document, cx);

    select(&editor, TextRange::in_block(0, 0, 5), cx);
    cx.simulate_keystrokes("cmd-.");
    assert_eq!(row(&tools, "Font", cx).detail.as_deref(), Some("Georgia"));
    assert_eq!(row(&tools, "Reset text size", cx).detail, None);
    cx.simulate_keystrokes("escape");

    select(&editor, TextRange::in_block(0, 0, 11), cx);
    cx.simulate_keystrokes("cmd-.");
    assert_eq!(row(&tools, "Font", cx).detail.as_deref(), Some("Mixed"));
    // Larger text, through the menu.
    cx.simulate_input("larger");
    cx.simulate_keystrokes("enter");
    let size = editor.read_with(cx, |editor, _| editor.document().blocks()[0].text_size());
    let size = size.expect("the block has its own text size now");
    cx.simulate_keystrokes("cmd-.");
    assert_eq!(
        row(&tools, "Reset text size", cx).detail,
        Some(format!("Now {size} pt"))
    );
    cx.simulate_input("reset");
    cx.simulate_keystrokes("enter");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.document().blocks()[0].text_size(), None);
    });
}

#[gpui::test]
fn a_selection_across_blocks_of_several_kinds(cx: &mut TestAppContext) {
    setup(cx);
    let plain = InlineAttrs::default;
    let document = Document::from_blocks(vec![
        Block::new(BlockKind::heading(1)).with_span("Title", plain()),
        Block::paragraph("First paragraph"),
        Block::new(BlockKind::BulletItem).with_span("a bullet", plain()),
        Block::new(BlockKind::todo()).with_span("a task", plain()),
        Block::new(BlockKind::Quote).with_span("a quote", plain()),
        Block::paragraph("Last paragraph"),
    ]);
    let (tools, editor, cx) = open(document, cx);
    // From inside the title to inside the quote; the last paragraph is not
    // touched.
    let range = TextRange::new(Position::new(0, 2), Position::new(4, 3));
    select(&editor, range, cx);
    let kinds = |cx: &mut gpui::VisualTestContext| -> Vec<BlockKind> {
        editor.read_with(cx, |editor, _| {
            editor
                .document()
                .blocks()
                .iter()
                .map(|block| block.kind().clone())
                .collect()
        })
    };
    let bold = |block: usize, offset: usize, cx: &mut gpui::VisualTestContext| {
        editor.read_with(cx, |editor, _| {
            editor.document().blocks()[block]
                .attrs_at(offset)
                .unwrap()
                .bold
        })
    };
    let before = kinds(cx);

    // The menu says what the selection is made of: every kind in it is
    // there in part, the others not at all.
    cx.simulate_keystrokes("cmd-.");
    for label in [
        "Heading 1",
        "Paragraph",
        "Bullet list",
        "To-do list",
        "Quote",
    ] {
        assert_eq!(row(&tools, label, cx).mark, Mark::Mixed, "{label}");
    }
    for label in ["Heading 2", "Numbered list", "Code block", "Bold"] {
        assert_eq!(row(&tools, label, cx).mark, Mark::None, "{label}");
    }
    assert_eq!(row(&tools, "Align left", cx).mark, Mark::Check);

    // Inline formatting goes on exactly the selected text, in every block.
    cx.simulate_input("bold");
    cx.simulate_keystrokes("enter");
    assert!(!bold(0, 1, cx) && bold(0, 2, cx) && bold(0, 4, cx));
    assert!(bold(1, 0, cx) && bold(1, 14, cx));
    assert!(bold(2, 0, cx) && bold(3, 5, cx));
    assert!(bold(4, 2, cx) && !bold(4, 3, cx));
    assert!(!bold(5, 0, cx));
    assert_eq!(kinds(cx), before, "bold changes no block");
    editor.read_with(cx, |editor, _| {
        assert_eq!(editor.selection().range(), range)
    });
    cx.simulate_keystrokes("cmd-.");
    assert_eq!(row(&tools, "Bold", cx).mark, Mark::Check);

    // "Turn into" changes every block the selection touches, as one step.
    cx.simulate_input("heading 2");
    assert_eq!(selected(&tools, cx).as_deref(), Some("Heading 2"));
    cx.simulate_keystrokes("enter");
    let mut expected = vec![BlockKind::heading(2); 5];
    expected.push(BlockKind::Paragraph);
    assert_eq!(kinds(cx), expected);
    assert_eq!(text(&editor, cx).lines().count(), 6, "no text was lost");
    cx.simulate_keystrokes("cmd-.");
    assert_eq!(row(&tools, "Heading 2", cx).mark, Mark::Check);
    assert_eq!(row(&tools, "Heading 1", cx).mark, Mark::None);
    assert_eq!(row(&tools, "Quote", cx).mark, Mark::None);
    cx.simulate_keystrokes("escape cmd-z");
    assert_eq!(kinds(cx), before, "one undo step brings every kind back");
    assert!(bold(1, 0, cx), "and leaves the bold");

    // A list from all of them, and alignment on all of them.
    cx.simulate_keystrokes("cmd-.");
    cx.simulate_input("bullet");
    cx.simulate_keystrokes("enter");
    let mut expected = vec![BlockKind::BulletItem; 5];
    expected.push(BlockKind::Paragraph);
    assert_eq!(kinds(cx), expected);
    cx.simulate_keystrokes("cmd-.");
    cx.simulate_input("centre");
    cx.simulate_keystrokes("enter");
    editor.read_with(cx, |editor, _| {
        let alignments: Vec<Alignment> = editor
            .document()
            .blocks()
            .iter()
            .map(|block| block.alignment())
            .collect();
        assert_eq!(alignments[..5], [Alignment::Center; 5]);
        assert_eq!(alignments[5], Alignment::Left);
        assert_eq!(editor.selection().range(), range, "the selection is kept");
    });
    // A colour and a highlight across the blocks, then "Clear formatting"
    // takes the inline formatting off again and keeps the kinds.
    cx.simulate_keystrokes("cmd-.");
    cx.simulate_input("blue text");
    cx.simulate_keystrokes("enter");
    cx.simulate_keystrokes("cmd-.");
    cx.simulate_input("pink");
    cx.simulate_keystrokes("enter");
    editor.read_with(cx, |editor, _| {
        for (block, offset) in [(0, 2), (1, 3), (2, 0), (3, 1), (4, 2)] {
            let attrs = editor.document().blocks()[block].attrs_at(offset).unwrap();
            assert_eq!(attrs.color, Some(TEXT_COLORS[3].1), "block {block}");
            assert_eq!(attrs.highlight, Some(Highlight::Pink), "block {block}");
        }
    });
    cx.simulate_keystrokes("cmd-.");
    assert_eq!(row(&tools, "Blue text", cx).mark, Mark::Check);
    assert_eq!(row(&tools, "Pink highlight", cx).mark, Mark::Check);
    cx.simulate_input("clear");
    cx.simulate_keystrokes("enter");
    editor.read_with(cx, |editor, _| {
        for (block, offset) in [(0, 2), (1, 3), (2, 0), (3, 1), (4, 2)] {
            let attrs = editor.document().blocks()[block].attrs_at(offset).unwrap();
            assert_eq!(attrs, &InlineAttrs::default(), "block {block}");
        }
    });
    assert_eq!(kinds(cx), expected);
}

/// WCAG contrast ratio of two opaque colours.
fn contrast(a: Hsla, b: Hsla) -> f32 {
    fn luminance(color: Hsla) -> f32 {
        let rgba = Rgba::from(color);
        let channel = |value: f32| {
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(rgba.r) + 0.7152 * channel(rgba.g) + 0.0722 * channel(rgba.b)
    }
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

#[gpui::test]
fn text_colours_read_on_paper_and_on_night(cx: &mut TestAppContext) {
    setup(cx);
    // The document stores one RGB value per colour and the editor paints
    // it as stored, so each colour must work on every surface text is
    // drawn on: the canvas and the page sheet of both themes.
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        cx.update(|cx| theme::set_theme_mode(mode, cx));
        let surfaces = cx.update(|cx| {
            let colors = cx.theme().colors;
            [("canvas", colors.canvas), ("page", colors.page)]
        });
        for (name, rgb) in TEXT_COLORS {
            let color: Hsla = Rgba {
                r: f32::from(rgb.r) / 255.,
                g: f32::from(rgb.g) / 255.,
                b: f32::from(rgb.b) / 255.,
                a: 1.,
            }
            .into();
            for (surface, background) in surfaces {
                let ratio = contrast(color, background);
                assert!(
                    ratio >= 3.8,
                    "{name} on the {mode:?} {surface}: contrast {ratio:.2}"
                );
            }
        }
    }
    // The names are distinct colours.
    for (index, (_, a)) in TEXT_COLORS.iter().enumerate() {
        for (_, b) in &TEXT_COLORS[index + 1..] {
            assert_ne!(a, b);
        }
    }
}

#[gpui::test]
fn an_open_menu_follows_a_theme_change(cx: &mut TestAppContext) {
    setup(cx);
    let (tools, editor, cx) = open(paragraphs(&["Hello brave world"]), cx);
    select(&editor, TextRange::in_block(0, 6, 11), cx);
    cx.simulate_keystrokes("cmd-.");
    cx.simulate_input("highl");
    cx.simulate_keystrokes("down down");
    let before = rows(&tools, cx);
    assert_eq!(selected(&tools, cx).as_deref(), Some("Blue highlight"));

    cx.update(|_, cx| theme::set_theme_mode(ThemeMode::Dark, cx));
    cx.run_until_parked();
    // Still open, same rows, same row selected, the filter kept; and the
    // row applies as before.
    assert_eq!(active(&tools, cx), Some(ToolKind::Format));
    assert_eq!(rows(&tools, cx), before);
    cx.simulate_keystrokes("enter");
    editor.read_with(cx, |editor, _| {
        assert_eq!(
            editor.document().blocks()[0].attrs_at(7).unwrap().highlight,
            Some(Highlight::Blue)
        );
    });
}
