//! Second review pass: behaviours fixed or added after the first adversarial
//! suite (`adversarial.rs`). Each test states the rule it pins down.

use writer_core::*;

fn ed(blocks: Vec<Block>) -> EditorState {
    EditorState::new(Document::from_blocks(blocks))
}
fn para(t: &str) -> Block {
    Block::paragraph(t)
}
fn kb(kind: BlockKind, t: &str) -> Block {
    Block::with_text(kind, t)
}
fn p(b: usize, o: usize) -> Position {
    Position::new(b, o)
}
fn texts(e: &EditorState) -> Vec<String> {
    e.document()
        .blocks()
        .iter()
        .map(|b| b.text().to_string())
        .collect()
}
fn kinds(e: &EditorState) -> Vec<BlockKind> {
    e.document()
        .blocks()
        .iter()
        .map(|b| b.kind().clone())
        .collect()
}
fn ids(e: &EditorState) -> Vec<(BlockId, u64)> {
    e.document()
        .blocks()
        .iter()
        .map(|b| (b.id(), b.revision()))
        .collect()
}
/// Types `s` one character at a time, as the keyboard does.
fn typ(e: &mut EditorState, s: &str) {
    for c in s.chars() {
        e.insert_text(&c.to_string());
    }
}
fn typed(s: &str) -> EditorState {
    let mut e = ed(vec![para("")]);
    typ(&mut e, s);
    e.check_invariants().unwrap();
    e
}
/// Undoes everything, checks the original comes back, redoes everything.
fn undo_redo_round_trip(e: &mut EditorState, original: &Document) {
    let fin = e.document().clone();
    let sel = e.selection();
    let mut n = 0;
    while e.undo() {
        n += 1;
        e.check_invariants().unwrap();
    }
    assert_eq!(e.document(), original);
    for _ in 0..n {
        assert!(e.redo());
    }
    assert_eq!(e.document(), &fin);
    assert_eq!(e.selection(), sel);
}

// ----- deleting whole blocks ----------------------------------------------------------

#[test]
fn whole_block_selection_leaves_the_following_block_untouched() {
    // shift-down from the start of a line selects (i, 0)..(i + 1, 0).
    let blocks = || {
        vec![
            para("delete me"),
            kb(BlockKind::BulletItem, "and me"),
            kb(BlockKind::heading(2), "Keep"),
            para("tail"),
        ]
    };
    let mut e = ed(blocks());
    let original = e.document().clone();
    let keep = ids(&e)[2];
    e.set_selection(Selection::new(p(0, 0), p(2, 0)));
    assert_eq!(e.touched_blocks(), 0..2);
    assert!(e.delete_selection());
    assert_eq!(texts(&e), ["Keep", "tail"]);
    assert_eq!(kinds(&e)[0], BlockKind::heading(2));
    assert_eq!(
        ids(&e)[0],
        keep,
        "same id and same revision stamp: nothing to re-lay-out"
    );
    assert_eq!(e.selection(), Selection::caret(p(0, 0)));
    assert_eq!(
        e.last_change().unwrap().blocks,
        Some(BlockSplice {
            start: 0,
            old_len: 2,
            new_len: 0
        })
    );
    assert!(e.undo());
    assert_eq!(e.document(), &original);
    assert_eq!(e.selection(), Selection::new(p(0, 0), p(2, 0)));
    assert_eq!(ids(&e), ids(&EditorState::new(original.clone())));

    // Reversed selection, and Backspace / typing over it.
    let mut e = ed(blocks());
    e.set_selection(Selection::new(p(1, 0), p(0, 0)));
    assert!(e.backspace());
    assert_eq!(texts(&e), ["and me", "Keep", "tail"]);
    assert_eq!(kinds(&e)[0], BlockKind::BulletItem);
    let mut e = ed(blocks());
    e.set_selection(Selection::new(p(0, 0), p(2, 0)));
    e.insert_text("x");
    assert_eq!(texts(&e), ["xKeep", "tail"]);
    assert_eq!(
        kinds(&e)[0],
        BlockKind::heading(2),
        "the surviving line keeps its kind"
    );

    // A range that starts inside the first block still merges into it.
    let mut e = ed(blocks());
    e.set_selection(Selection::new(p(0, 6), p(2, 0)));
    e.delete_selection();
    assert_eq!(texts(&e), ["deleteKeep", "tail"]);
    assert_eq!(kinds(&e)[0], BlockKind::Paragraph);
    // ... and so does one that ends inside the last block.
    let mut e = ed(blocks());
    e.set_selection(Selection::new(p(0, 0), p(2, 2)));
    e.delete_selection();
    assert_eq!(texts(&e), ["ep", "tail"]);
    assert_eq!(kinds(&e)[0], BlockKind::Paragraph);
    // Ending on a void block takes the void block along, as before.
    let mut e = ed(vec![para("a"), Block::new(BlockKind::Divider), para("b")]);
    e.set_selection(Selection::new(p(0, 0), p(1, 0)));
    e.delete_selection();
    assert_eq!(texts(&e), ["", "b"]);
    e.check_invariants().unwrap();
}

#[test]
fn cut_whole_lines_and_paste_them_at_a_line_start() {
    let mut e = ed(vec![
        kb(BlockKind::heading(1), "Title"),
        kb(BlockKind::NumberedItem, "one"),
        kb(BlockKind::NumberedItem, "two"),
        kb(BlockKind::Quote, "quote"),
        para("end"),
    ]);
    let original = e.document().clone();
    // Cut the two list items as whole lines.
    e.set_selection(Selection::new(p(1, 0), p(3, 0)));
    let f = e.cut();
    assert_eq!(
        f.blocks().len(),
        3,
        "two items and the empty head of the next line"
    );
    assert_eq!(texts(&e), ["Title", "quote", "end"]);
    assert_eq!(kinds(&e)[1], BlockKind::Quote);
    // Paste them in front of "end".
    e.set_caret(p(2, 0));
    assert!(e.paste(&f));
    assert_eq!(texts(&e), ["Title", "quote", "one", "two", "end"]);
    assert_eq!(
        kinds(&e),
        vec![
            BlockKind::heading(1),
            BlockKind::Quote,
            BlockKind::NumberedItem,
            BlockKind::NumberedItem,
            BlockKind::Paragraph
        ]
    );
    assert_eq!(e.selection(), Selection::caret(p(4, 0)));
    assert_eq!(
        e.document().numbered_ordinals(0..5),
        [None, None, Some(1), Some(2), None]
    );
    e.check_invariants().unwrap();
    // Replace a whole line by pasting over a whole-line selection.
    e.set_selection(Selection::new(p(1, 0), p(2, 0)));
    assert!(e.paste(&f));
    assert_eq!(texts(&e), ["Title", "one", "two", "one", "two", "end"]);
    assert!(
        kinds(&e)[1..5]
            .iter()
            .all(|k| *k == BlockKind::NumberedItem)
    );
    undo_redo_round_trip(&mut e, &original);
}

// ----- merging with empty blocks ------------------------------------------------------

#[test]
fn deleting_an_empty_line_never_restyles_the_text_next_to_it() {
    // Forward delete in an empty paragraph above a heading.
    let mut e = ed(vec![
        para(""),
        kb(BlockKind::heading(1), "Title"),
        para("x"),
    ]);
    let title = ids(&e)[1];
    assert!(e.delete_forward());
    assert_eq!(texts(&e), ["Title", "x"]);
    assert_eq!(kinds(&e)[0], BlockKind::heading(1));
    assert_eq!(ids(&e)[0], title);
    assert_eq!(e.selection(), Selection::caret(p(0, 0)));
    assert!(e.undo());
    assert_eq!(texts(&e), ["", "Title", "x"]);
    assert_eq!(e.selection(), Selection::caret(p(0, 0)));

    // Backspace at the start of a paragraph below an empty heading / item.
    for kind in [
        BlockKind::heading(3),
        BlockKind::BulletItem,
        BlockKind::Quote,
        BlockKind::code(),
    ] {
        let mut e = ed(vec![kb(kind.clone(), ""), para("text")]);
        let text = ids(&e)[1];
        e.set_caret(p(1, 0));
        assert!(e.backspace());
        assert_eq!(texts(&e), ["text"], "{kind:?}");
        assert_eq!(kinds(&e), vec![BlockKind::Paragraph], "{kind:?}");
        assert_eq!(ids(&e)[0], text);
        assert_eq!(e.selection(), Selection::caret(p(0, 0)));
        assert!(e.undo());
        assert_eq!(kinds(&e)[0], kind);
        assert_eq!(e.selection(), Selection::caret(p(1, 0)));
    }

    // With text on both sides the first block still wins (id and kind).
    let mut e = ed(vec![kb(BlockKind::heading(1), "Head"), para("tail")]);
    let head = e.document().blocks()[0].id();
    e.set_caret(p(1, 0));
    e.backspace();
    assert_eq!(texts(&e), ["Headtail"]);
    assert_eq!(kinds(&e), vec![BlockKind::heading(1)]);
    assert_eq!(e.document().blocks()[0].id(), head);
    assert_eq!(e.selection(), Selection::caret(p(0, 4)));
    let mut e = ed(vec![para("tail"), kb(BlockKind::heading(1), "Head")]);
    e.set_caret(p(0, 4));
    e.delete_forward();
    assert_eq!(texts(&e), ["tailHead"]);
    assert_eq!(kinds(&e), vec![BlockKind::Paragraph]);

    // Two empty blocks: Backspace removes the one the caret is in, forward
    // delete the one after it.
    let mut e = ed(vec![kb(BlockKind::heading(1), ""), para("")]);
    e.set_caret(p(1, 0));
    e.backspace();
    assert_eq!(kinds(&e), vec![BlockKind::heading(1)]);
    let mut e = ed(vec![para(""), kb(BlockKind::heading(1), "")]);
    e.delete_forward();
    assert_eq!(kinds(&e), vec![BlockKind::Paragraph]);
    // An empty block at the end of the text above keeps the usual merge.
    let mut e = ed(vec![para("abc"), kb(BlockKind::BulletItem, "")]);
    e.set_caret(p(0, 3));
    e.delete_forward();
    assert_eq!(texts(&e), ["abc"]);
    assert_eq!(kinds(&e), vec![BlockKind::Paragraph]);
}

// ----- input rules --------------------------------------------------------------------

#[test]
fn markdown_task_syntax_makes_a_todo() {
    for (typed_text, checked) in [
        ("- [ ] ", false),
        ("- [] ", false),
        ("* [x] ", true),
        ("+ [X] ", true),
    ] {
        let mut e = typed(typed_text);
        assert_eq!(
            kinds(&e),
            vec![BlockKind::TodoItem { checked }],
            "{typed_text:?}"
        );
        assert_eq!(texts(&e), [""]);
        typ(&mut e, "task");
        assert_eq!(texts(&e), ["task"]);
        // One undo per word / rule: the to-do rule gives back the literal
        // characters inside the bullet, the next undos lead back to "- ".
        for _ in 0..2 {
            e.undo();
        }
        assert_eq!(kinds(&e), vec![BlockKind::BulletItem]);
        assert!(texts(&e)[0].starts_with('['), "{:?}", texts(&e));
    }
    // Only at the very start of the item, and only in bullets.
    let e = typed("- a [ ] ");
    assert_eq!(kinds(&e), vec![BlockKind::BulletItem]);
    assert_eq!(texts(&e), ["a [ ] "]);
    let e = typed("1. [ ] ");
    assert_eq!(kinds(&e), vec![BlockKind::NumberedItem]);
    assert_eq!(texts(&e), ["[ ] "]);
    // The indent of the bullet is kept.
    let mut e = ed(vec![kb(BlockKind::BulletItem, "").with_indent(2)]);
    typ(&mut e, "[ ] ");
    assert_eq!(kinds(&e), vec![BlockKind::todo()]);
    assert_eq!(e.document().blocks()[0].indent(), 2);
    // Switched off with the other input rules.
    let mut e = ed(vec![kb(BlockKind::BulletItem, "")]);
    e.set_options(EditorOptions {
        input_rules: false,
        smart_punctuation: true,
    });
    typ(&mut e, "[ ] ");
    assert_eq!(kinds(&e), vec![BlockKind::BulletItem]);
}

#[test]
fn triple_asterisk_is_bold_italic() {
    let mut e = typed("a ***both*** b");
    let b = &e.document().blocks()[0];
    assert_eq!(b.text(), "a both b");
    let spans: Vec<(String, bool, bool)> = b
        .text_spans()
        .map(|(_, t, a)| (t.to_string(), a.bold, a.italic))
        .collect();
    assert_eq!(
        spans,
        [
            ("a ".to_string(), false, false),
            ("both".to_string(), true, true),
            (" b".to_string(), false, false)
        ]
    );
    // Undo steps: "b", the space, then the rule (literal characters come back).
    e.undo();
    e.undo();
    assert_eq!(texts(&e), ["a both"]);
    e.undo();
    assert_eq!(texts(&e), ["a ***both***"]);
    // Still plain: unbalanced or spaced delimiters.
    for s in ["***", "*** x ***", "a***b***", "2 *** 3 *** 4"] {
        let e = typed(s);
        assert_eq!(texts(&e), [s], "{s:?}");
        assert!(
            e.document().blocks()[0]
                .runs()
                .iter()
                .all(|r| r.attrs.is_plain()),
            "{s:?}"
        );
    }
    // `**bold**` and `*italic*` are unaffected.
    let e = typed("**b** *i*");
    let b = &e.document().blocks()[0];
    assert_eq!(b.text(), "b i");
    assert!(b.attrs_at(0).unwrap().bold && !b.attrs_at(0).unwrap().italic);
    assert!(b.attrs_at(2).unwrap().italic && !b.attrs_at(2).unwrap().bold);
}

#[test]
fn smart_quotes_pair_up() {
    let cases = [
        ("\"\"", "\u{201C}\u{201D}"),
        ("''", "\u{2018}\u{2019}"),
        ("say \"\" now", "say \u{201C}\u{201D} now"),
        (
            "\"'nested' quotes\"",
            "\u{201C}\u{2018}nested\u{2019} quotes\u{201D}",
        ),
        (
            "'\"nested\" quotes'",
            "\u{2018}\u{201C}nested\u{201D} quotes\u{2019}",
        ),
        ("\"'", "\u{201C}\u{2018}"),
        ("'\"", "\u{2018}\u{201C}"),
        (
            "(\"a\") [\"b\"] {\"c\"}",
            "(\u{201C}a\u{201D}) [\u{201C}b\u{201D}] {\u{201C}c\u{201D}}",
        ),
        ("don't won't it's", "don\u{2019}t won\u{2019}t it\u{2019}s"),
        ("dogs' bowls", "dogs\u{2019} bowls"),
        ("word\u{2014}\"quote\"", "word\u{2014}\u{201C}quote\u{201D}"),
        ("a\n\"b\"", "a\n\u{201C}b\u{201D}"),
    ];
    for (input, want) in cases {
        let mut e = ed(vec![para("")]);
        for c in input.chars() {
            if c == '\n' {
                e.insert_soft_break();
            } else {
                e.insert_text(&c.to_string());
            }
        }
        assert_eq!(texts(&e), [want], "{input:?}");
    }
    // Quotes inside Markdown emphasis open and close correctly, and the
    // inline rule still fires around them.
    for (input, want) in [
        ("**\"bold\"** x", "\u{201C}bold\u{201D} x"),
        ("a *'it'* x", "a \u{2018}it\u{2019} x"),
        ("_\"u\"_", "\u{201C}u\u{201D}"),
        ("word*\"", "word*\u{201D}"),
    ] {
        let e = typed(input);
        assert_eq!(texts(&e), [want], "{input:?}");
    }
    let e = typed("**\"bold\"**");
    assert!(e.document().blocks()[0].runs().iter().all(|r| r.attrs.bold));
    // One undo gives the straight quote back.
    let mut e = typed("\"\"");
    e.undo();
    assert_eq!(texts(&e), ["\u{201C}\""]);
}

// ----- settings -----------------------------------------------------------------------

#[test]
fn sanitising_settings_is_a_fixed_point() {
    let mut k: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut rnd = |max: f32| {
        k ^= k << 13;
        k ^= k >> 7;
        k ^= k << 17;
        (k % 1_000_000) as f32 / 1_000_000.0 * max
    };
    for i in 0..20_000 {
        let s = DocumentSettings {
            page: PageSetup {
                paper: PaperSize::Custom {
                    width: rnd(2000.0),
                    height: rnd(2000.0),
                },
                orientation: if i % 2 == 0 {
                    Orientation::Portrait
                } else {
                    Orientation::Landscape
                },
                margins: Margins {
                    top: rnd(900.0),
                    right: rnd(900.0),
                    bottom: rnd(900.0),
                    left: rnd(900.0),
                },
            },
            typography: Typography {
                base_size: rnd(100.0),
                line_height: rnd(6.0),
                paragraph_spacing: rnd(200.0),
                ..Typography::default()
            },
            page_view: i % 3 == 0,
            spelling_language: match i % 4 {
                0 => None,
                1 => Some(writer_core::SpellingLanguage::Automatic),
                2 => Some(writer_core::SpellingLanguage::Language(format!(
                    " l{} ",
                    i % 97
                ))),
                _ => Some(writer_core::SpellingLanguage::Language(format!(
                    "{} x",
                    i % 97
                ))),
            },
        };
        let once = s.clone().sanitized();
        assert_eq!(once.clone().sanitized(), once, "{s:?}");
        let (w, h) = once.page.content_size();
        assert!(w >= 71.5 && h >= 71.5, "an inch of text area: {once:?}");
        if i % 50 == 0 {
            // The document keeps its invariants and round-trips with them.
            let mut e = EditorState::new(Document::new());
            e.set_settings(s);
            e.check_invariants().unwrap();
            assert_eq!(e.document().settings(), &once);
            let back = Document::from_json(&e.document().to_json()).unwrap();
            assert_eq!(&back, e.document());
            back.check_invariants().unwrap();
        }
    }
}

// ----- word scans ---------------------------------------------------------------------

#[test]
fn word_scans_agree_with_whole_text_segmentation() {
    let pieces = [
        "can't",
        "don\u{2019}t",
        "3,14",
        "3.14.15",
        "a.b",
        "a'",
        "'a",
        "''",
        "x_y",
        "_",
        "__a__",
        "שלום",
        "ש\"ח",
        "\"",
        "中文字",
        "カタカナ",
        "e\u{301}",
        "\u{301}",
        "\u{1F469}\u{200D}\u{1F469}\u{200D}\u{1F467}",
        "\u{1F1EB}\u{1F1F7}",
        "\u{1F1EB}",
        " ",
        "  ",
        "\n",
        "\n\n",
        "\t",
        ".",
        ",",
        ":",
        "a:b",
        "1:2",
        "é",
        "-",
        "\u{2014}",
        "\u{2026}",
        "word",
        "A",
        "1",
        "a1",
        "\u{663}",
        "\u{B7}",
        "l\u{B7}l",
        "\u{200D}",
        "\u{FE0F}",
        "#",
        "x@y.z",
        "\u{202F}",
        "\u{A0}",
    ];
    let mut k: u64 = 99;
    let mut next = |n: usize| {
        k ^= k << 13;
        k ^= k >> 7;
        k ^= k << 17;
        (k % n as u64) as usize
    };
    for _ in 0..4000 {
        let mut t = String::new();
        for _ in 0..1 + next(9) {
            t.push_str(pieces[next(pieces.len())]);
        }
        let words: Vec<std::ops::Range<usize>> = text::word_ranges(&t).collect();
        for o in (0..=t.len()).filter(|o| t.is_char_boundary(*o)) {
            let start = words
                .iter()
                .take_while(|r| r.start < o)
                .last()
                .map(|r| r.start);
            let end = words.iter().find(|r| r.end > o).map(|r| r.end);
            assert_eq!(
                text::word_start_before(&t, o),
                start,
                "start in {t:?} at {o}"
            );
            assert_eq!(text::word_end_after(&t, o), end, "end in {t:?} at {o}");
        }
    }
    // Out of range and mid-character offsets are clamped.
    assert_eq!(text::word_start_before("héllo", 99), Some(0));
    assert_eq!(text::word_end_after("héllo", 2), Some(6));
    assert_eq!(text::word_end_after("", 5), None);
}

// ----- multi-line insertion -----------------------------------------------------------

#[test]
fn multi_line_plain_text_follows_the_enter_rules_in_one_step() {
    // In the middle of a heading: every piece stays a heading.
    let mut e = ed(vec![
        para("before"),
        kb(BlockKind::heading(2), "HeadTail"),
        para("after"),
    ]);
    let original = e.document().clone();
    let head = e.document().blocks()[1].id();
    e.set_caret(p(1, 4));
    e.take_changes();
    assert!(e.paste_plain_text("a\nb\r\nc"));
    assert_eq!(texts(&e), ["before", "Heada", "b", "cTail", "after"]);
    assert!(kinds(&e)[1..4].iter().all(|k| *k == BlockKind::heading(2)));
    assert_eq!(e.document().blocks()[1].id(), head);
    assert_eq!(e.selection(), Selection::caret(p(3, 1)));
    assert_eq!(
        e.take_changes().unwrap().blocks,
        Some(BlockSplice {
            start: 1,
            old_len: 1,
            new_len: 3
        }),
        "one local splice, neighbours untouched"
    );
    assert_eq!(e.undo_depth(), 1);
    undo_redo_round_trip(&mut e, &original);

    // At the end of a heading the new lines are paragraphs; at the end of
    // a checked to-do they are unchecked to-dos.
    let mut e = ed(vec![kb(BlockKind::heading(1), "Head")]);
    e.set_caret(p(0, 4));
    e.paste_plain_text("\n\nx");
    assert_eq!(texts(&e), ["Head", "", "x"]);
    assert_eq!(
        kinds(&e),
        vec![
            BlockKind::heading(1),
            BlockKind::Paragraph,
            BlockKind::Paragraph
        ]
    );
    let mut e = ed(vec![
        kb(BlockKind::TodoItem { checked: true }, "done").with_indent(1),
    ]);
    e.set_caret(p(0, 4));
    e.paste_plain_text("\nnext\nlast");
    assert_eq!(texts(&e), ["done", "next", "last"]);
    assert_eq!(kinds(&e)[0], BlockKind::TodoItem { checked: true });
    assert_eq!(kinds(&e)[1..], [BlockKind::todo(), BlockKind::todo()]);
    assert!(e.document().blocks().iter().all(|b| b.indent() == 1));

    // At offset 0 of a block with text, a leading line break puts an empty
    // block before it and the text block keeps its id.
    let mut e = ed(vec![kb(BlockKind::Quote, "Text")]);
    let id = e.document().blocks()[0].id();
    e.paste_plain_text("\nx");
    assert_eq!(texts(&e), ["", "xText"]);
    assert_eq!(e.document().blocks()[1].id(), id);
    assert_eq!(kinds(&e), vec![BlockKind::Quote, BlockKind::Quote]);

    // Over a selection, on a void block, and into a code block.
    let mut e = ed(vec![
        para("abcd"),
        Block::new(BlockKind::Divider),
        kb(BlockKind::code(), "fn x"),
    ]);
    e.set_selection(Selection::new(p(0, 1), p(0, 3)));
    e.insert_text("1\n2");
    assert_eq!(texts(&e), ["a1", "2d", "", "fn x"]);
    e.set_caret(p(2, 0));
    e.paste_plain_text("p\nq");
    assert_eq!(texts(&e), ["a1", "2d", "", "p", "q", "fn x"]);
    assert!(e.document().blocks()[2].is_void());
    e.set_caret(p(5, 4));
    e.paste_plain_text("()\n{}");
    assert_eq!(texts(&e).last().unwrap(), "fn x()\n{}");
    e.check_invariants().unwrap();

    // Typed attributes apply to every line; comment anchors are not spread.
    let mut e = ed(vec![para("ab")]);
    e.set_caret(p(0, 1));
    e.toggle_bold();
    e.insert_text("x\ny");
    assert_eq!(texts(&e), ["ax", "yb"]);
    assert!(e.document().blocks()[0].attrs_at(1).unwrap().bold);
    assert!(e.document().blocks()[1].attrs_at(0).unwrap().bold);
    assert!(!e.document().blocks()[1].attrs_at(1).unwrap().bold);
}

#[test]
fn pasting_many_lines_is_one_splice_and_one_undo_step() {
    let blocks: Vec<Block> = (0..2000).map(|i| para(&format!("block {i}"))).collect();
    let mut e = ed(blocks);
    let original = e.document().clone();
    let plain: String = (0..3000).map(|i| format!("line {i}\n")).collect();
    e.set_caret(p(1000, 3));
    let before = ids(&e);
    e.take_changes();
    assert!(e.paste_plain_text(&plain));
    assert_eq!(e.document().block_count(), 5000);
    let c = e.take_changes().unwrap();
    assert_eq!(
        c.blocks,
        Some(BlockSplice {
            start: 1000,
            old_len: 1,
            new_len: 3001
        })
    );
    let after = ids(&e);
    assert_eq!(before[..1000], after[..1000]);
    assert_eq!(before[1001..], after[4001..]);
    assert_eq!(e.document().blocks()[1000].text(), "bloline 0");
    assert_eq!(e.document().blocks()[4000].text(), "ck 1000");
    assert_eq!(e.selection(), Selection::caret(p(4000, 0)));
    assert!(e.undo());
    assert_eq!(e.document(), &original);
    assert_eq!(ids(&e), before);
    assert!(!e.undo());
}

// ----- small API additions --------------------------------------------------------------

#[test]
fn todo_checkbox_click_keeps_the_selection() {
    let mut e = ed(vec![
        para("text"),
        kb(BlockKind::todo(), "task"),
        kb(BlockKind::BulletItem, "b"),
    ]);
    e.set_selection(Selection::new(p(0, 1), p(0, 3)));
    let sel = e.selection();
    e.toggle_bold();
    let pending_before = e.pending_attrs().cloned();
    assert!(e.set_todo_checked(1, true));
    assert_eq!(kinds(&e)[1], BlockKind::TodoItem { checked: true });
    assert_eq!(e.selection(), sel);
    assert_eq!(e.pending_attrs().cloned(), pending_before);
    assert!(!e.set_todo_checked(1, true), "already checked");
    assert!(!e.set_todo_checked(2, true), "not a to-do");
    assert!(!e.set_todo_checked(99, true));
    assert!(e.undo());
    assert_eq!(kinds(&e)[1], BlockKind::todo());
    assert_eq!(e.selection(), sel);
}

#[test]
fn block_range_and_selected_text() {
    let mut e = ed(vec![
        para("héllo"),
        Block::new(BlockKind::Divider),
        para("wörld"),
    ]);
    assert_eq!(e.document().block_range(0), TextRange::in_block(0, 0, 6));
    assert_eq!(e.document().block_range(1), TextRange::in_block(1, 0, 0));
    assert_eq!(e.document().block_range(99), TextRange::in_block(2, 0, 6));
    assert_eq!(e.selected_text(), "");
    e.select_range(TextRange::new(p(0, 1), p(2, 3)));
    assert_eq!(e.selected_text(), "éllo\n\nwö");
    e.select_range(e.document().block_range(2));
    assert_eq!(e.selected_text(), "wörld");
}

// ----- Vim-style use of the API ---------------------------------------------------------

#[test]
fn linewise_and_grouped_operations_undo_as_one_step() {
    let mut e = ed(vec![
        kb(BlockKind::heading(1), "Title"),
        kb(BlockKind::NumberedItem, "one"),
        kb(BlockKind::NumberedItem, "two"),
        para("three"),
    ]);
    let original = e.document().clone();
    // "dd" on block 1, then "p" below block 2 (now index 1), as one step each.
    e.set_caret(p(1, 2));
    let reg = e.grouped(|e| {
        let reg = e.document().block_fragment(1..2);
        e.select_blocks(1..2);
        e.delete_blocks();
        reg
    });
    assert_eq!(texts(&e), ["Title", "two", "three"]);
    assert_eq!(e.selection(), Selection::caret(p(1, 0)));
    assert_eq!(e.undo_depth(), 1);
    assert!(e.insert_blocks_at(2, &reg));
    assert_eq!(texts(&e), ["Title", "two", "one", "three"]);
    assert_eq!(e.selection(), Selection::caret(p(2, 0)));
    assert_eq!(
        e.document().numbered_ordinals(0..4),
        [None, Some(1), Some(2), None]
    );
    // "cw"-like change: delete a range and type, with caret moves in between.
    e.grouped(|e| {
        e.delete_range(TextRange::in_block(3, 0, 5));
        e.insert_text("3");
        e.move_caret(Movement::BlockStart, false);
        e.insert_text("#");
    });
    assert_eq!(texts(&e)[3], "#3");
    assert_eq!(e.undo_depth(), 3);
    // "J": join with a space.
    let end1 = e.document().block_range(1).end;
    assert!(e.replace_range(TextRange::new(end1, p(2, 0)), " "));
    assert_eq!(texts(&e), ["Title", "two one", "#3"]);
    assert_eq!(e.selection(), Selection::caret(p(1, 4)));
    // "~" keeps per-character formatting.
    e.select_range(TextRange::in_block(1, 0, 3));
    e.toggle_italic();
    assert!(e.replace_range(TextRange::in_block(1, 2, 5), "O O"));
    let b = &e.document().blocks()[1];
    assert_eq!(b.text(), "twO One");
    assert!(b.attrs_at(2).unwrap().italic && !b.attrs_at(3).unwrap().italic);
    undo_redo_round_trip(&mut e, &original);
}

#[test]
fn ime_composition_is_one_undo_step_and_keeps_utf16_in_sync() {
    let mut e = ed(vec![para("a\u{1F600}b")]);
    let original = e.document().clone();
    // Caret after the emoji, given by the IME as a UTF-16 offset.
    let b = &e.document().blocks()[0];
    let caret = b.utf16_to_offset(3);
    assert_eq!(caret, 5);
    assert_eq!(
        b.utf16_to_offset(2),
        1,
        "inside the surrogate pair rounds down"
    );
    e.set_caret(p(0, caret));
    e.toggle_bold();
    // Marked text grows and changes, then is committed.
    e.begin_undo_group();
    let mut marked = TextRange::empty(p(0, caret));
    for text in ["k", "か", "かn", "かな", "仮名"] {
        assert!(e.replace_range(marked, text));
        marked = TextRange::new(marked.start, e.selection().head);
        assert_eq!(e.document().text_in(marked), text);
        let b = &e.document().blocks()[0];
        assert_eq!(
            b.offset_to_utf16(marked.end.offset) - b.offset_to_utf16(marked.start.offset),
            text.encode_utf16().count()
        );
    }
    e.end_undo_group();
    let b = &e.document().blocks()[0];
    assert_eq!(b.text(), "a\u{1F600}仮名b");
    assert!(b.attrs_at(caret).unwrap().bold && b.attrs_at(caret + 3).unwrap().bold);
    assert!(!b.attrs_at(caret + 6).unwrap().bold);
    assert_eq!(e.undo_depth(), 1);
    undo_redo_round_trip(&mut e, &original);
}

// ----- typing over a selection ----------------------------------------------------------

#[test]
fn typing_over_a_selection_continues_the_replaced_formatting() {
    let bold = InlineAttrs {
        bold: true,
        ..Default::default()
    };
    let link = InlineAttrs {
        link: Some("https://example.com".into()),
        italic: true,
        ..Default::default()
    };
    let build = || {
        ed(vec![
            para("plain ")
                .with_span("bold", bold.clone())
                .with_span(" mid ", InlineAttrs::default())
                .with_span("linked", link.clone())
                .with_span(" end", InlineAttrs::default()),
        ])
    };
    let spans = |e: &EditorState| -> Vec<(String, bool, bool, bool)> {
        e.document().blocks()[0]
            .text_spans()
            .map(|(_, t, a)| (t.to_string(), a.bold, a.italic, a.link.is_some()))
            .collect()
    };
    // Retyping the bold word keeps it bold.
    let mut e = build();
    e.select_range(TextRange::in_block(0, 6, 10));
    typ(&mut e, "new");
    assert_eq!(spans(&e)[1], ("new".to_string(), true, false, false));
    assert_eq!(e.document().blocks()[0].text(), "plain new mid linked end");
    // A selection that starts in plain text gives plain text.
    let mut e = build();
    e.select_range(TextRange::in_block(0, 3, 8));
    typ(&mut e, "x");
    assert!(!e.document().blocks()[0].attrs_at(3).unwrap().bold);
    // Pending attributes win over the replaced formatting.
    let mut e = build();
    e.select_range(TextRange::in_block(0, 6, 10));
    e.delete_selection();
    e.clear_formatting();
    typ(&mut e, "new");
    assert!(!e.document().blocks()[0].attrs_at(6).unwrap().bold);
    // Replacing the whole link keeps its italic but not the link ...
    let mut e = build();
    e.select_range(TextRange::in_block(0, 15, 21));
    typ(&mut e, "text");
    assert!(
        spans(&e).contains(&("text".to_string(), false, true, false)),
        "{:?}",
        spans(&e)
    );
    // ... replacing part of it stays inside the link.
    let mut e = build();
    e.select_range(TextRange::in_block(0, 16, 20));
    typ(&mut e, "oo");
    assert!(
        spans(&e).contains(&("lood".to_string(), false, true, true)),
        "{:?}",
        spans(&e)
    );
    // The same for comment anchors: only strictly inside.
    let mut e = build();
    e.select_range(TextRange::in_block(0, 6, 10));
    let id = e.add_comment("note", "me", 1).unwrap();
    e.select_range(TextRange::in_block(0, 7, 9));
    typ(&mut e, "x");
    assert_eq!(
        e.document().comment_ranges(id),
        [TextRange::in_block(0, 6, 9)]
    );
    e.select_range(TextRange::in_block(0, 6, 9));
    typ(&mut e, "y");
    assert!(e.document().is_orphaned(id));
    assert!(
        e.document().blocks()[0].attrs_at(6).unwrap().bold,
        "formatting still continues"
    );
    e.undo();
    assert!(!e.document().is_orphaned(id));
    // Whole block selected: the first character's formatting carries on.
    let mut e = ed(vec![para("").with_span("Title", bold.clone())]);
    e.select_all();
    typ(&mut e, "New");
    assert!(e.document().blocks()[0].runs().iter().all(|r| r.attrs.bold));
    // Plain-text paste over a selection follows the same rule.
    let mut e = build();
    e.select_range(TextRange::in_block(0, 6, 10));
    e.paste_plain_text("pasted");
    assert!(e.document().blocks()[0].attrs_at(8).unwrap().bold);
    e.check_invariants().unwrap();
}

#[test]
fn word_movement_stops_at_emoji() {
    let family = "\u{1F469}\u{200D}\u{1F469}\u{200D}\u{1F467}";
    let text = format!("hello {family} \u{1F1EB}\u{1F1F7}\u{2764}\u{FE0F} wor-ld!! \u{2014} end");
    let d = Document::from_blocks(vec![para(&text)]);
    let mut stops = Vec::new();
    let mut pos = d.start();
    loop {
        let next = d.moved(pos, Movement::WordRight);
        if next == pos {
            break;
        }
        stops.push(text[pos.offset..next.offset].to_string());
        pos = next;
    }
    assert_eq!(
        stops,
        [
            "hello".to_string(),
            format!(" {family}"),
            " \u{1F1EB}\u{1F1F7}".to_string(),
            "\u{2764}\u{FE0F}".to_string(),
            " wor".to_string(),
            "-ld".to_string(),
            "!! \u{2014} end".to_string(),
        ]
    );
    // And back again, landing on the same word starts.
    let mut back = Vec::new();
    loop {
        let prev = d.moved(pos, Movement::WordLeft);
        if prev == pos {
            break;
        }
        back.push(prev.offset);
        pos = prev;
    }
    let starts: Vec<usize> = text::word_ranges(&text).map(|r| r.start).collect();
    back.reverse();
    assert_eq!(back, starts);
    // Option-backspace takes one emoji, not the word before it too.
    let mut e = ed(vec![para(&format!("hello {family} "))]);
    e.move_caret(Movement::DocEnd, false);
    e.delete_word_backward();
    assert_eq!(texts(&e), ["hello "]);
    // Emoji are not counted as words.
    assert_eq!(Stats::of_text(&text).words, 4);
}
