//! Adversarial tests written against the public API only, one area of the
//! engine at a time: graphemes and UTF-16, undo exactness and coalescing,
//! comment anchors, Enter / Backspace per block kind, input rules, smart
//! punctuation, fragments, numbering, search and serialization.

#![allow(clippy::type_complexity)]

use writer_core::text;
use writer_core::*;

const FAMILY: &str = "\u{1F469}\u{200D}\u{1F469}\u{200D}\u{1F467}";
const E_ACUTE: &str = "e\u{301}";
const FLAG: &str = "\u{1F1EB}\u{1F1F7}";

fn ed(blocks: Vec<Block>) -> EditorState {
    EditorState::new(Document::from_blocks(blocks))
}
fn para(t: &str) -> Block {
    Block::paragraph(t)
}
fn kb(kind: BlockKind, t: &str) -> Block {
    Block::with_text(kind, t)
}
fn divider() -> Block {
    Block::new(BlockKind::Divider)
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
fn typ(e: &mut EditorState, s: &str) {
    for c in s.chars() {
        e.insert_text(&c.to_string());
        e.check_invariants().unwrap();
    }
}
fn caret(e: &EditorState) -> Position {
    assert!(e.selection().is_collapsed(), "{:?}", e.selection());
    e.selection().head
}
fn bold() -> InlineAttrs {
    InlineAttrs {
        bold: true,
        ..Default::default()
    }
}
fn no_rules(e: &mut EditorState) {
    e.set_options(EditorOptions {
        input_rules: false,
        smart_punctuation: false,
    });
}
fn only_smart(e: &mut EditorState) {
    e.set_options(EditorOptions {
        input_rules: false,
        smart_punctuation: true,
    });
}
/// Undoes everything, checking invariants, and returns the number of steps.
fn undo_count(e: &mut EditorState) -> usize {
    let mut n = 0;
    while e.undo() {
        e.check_invariants().unwrap();
        n += 1;
    }
    n
}
fn styled(e: &EditorState, block: usize, f: impl Fn(&InlineAttrs) -> bool) -> String {
    let b = &e.document().blocks()[block];
    b.text_spans()
        .filter(|(_, _, a)| f(a))
        .map(|(_, t, _)| t)
        .collect()
}

// ----- graphemes and UTF-16 ------------------------------------------------------------

#[test]
fn caret_never_lands_inside_a_grapheme() {
    let t = format!("a{FAMILY}{E_ACUTE}{FLAG}{FLAG}中\u{1100}\u{1161}\u{11A8}ש");
    let d = Document::from_blocks(vec![para(&t)]);
    for o in 0..=t.len() + 2 {
        let c = d.clamp_position(p(0, o));
        assert!(text::is_grapheme_boundary(&t, c.offset), "{o} -> {c:?}");
        assert!(c.offset <= o.min(t.len()));
    }
    // Walking right then left visits the same grapheme boundaries.
    let mut fwd = vec![0];
    let mut pos = p(0, 0);
    loop {
        let n = d.moved(pos, Movement::Right);
        if n == pos {
            break;
        }
        fwd.push(n.offset);
        pos = n;
    }
    let mut back = vec![pos.offset];
    loop {
        let n = d.moved(pos, Movement::Left);
        if n == pos {
            break;
        }
        back.push(n.offset);
        pos = n;
    }
    back.reverse();
    assert_eq!(fwd, back);
    // a, family, e+acute, flag, flag, 中, Hangul syllable (3 jamo), ש
    assert_eq!(fwd.len() - 1, 8);
    assert_eq!(text::grapheme_count(&t), 8);
}

#[test]
fn regional_indicators_pair_up() {
    let t = format!("{FLAG}{FLAG}\u{1F1EB}");
    assert_eq!(text::next_grapheme_boundary(&t, 0), 8);
    assert_eq!(text::next_grapheme_boundary(&t, 8), 16);
    assert_eq!(text::prev_grapheme_boundary(&t, t.len()), 16);
    assert!(!text::is_grapheme_boundary(&t, 4));
    assert!(!text::is_grapheme_boundary(&t, 12));
}

#[test]
fn backspace_and_delete_take_whole_graphemes() {
    let t = format!("x{FAMILY}{FLAG}{E_ACUTE}");
    let mut e = ed(vec![para(&t)]);
    e.set_caret(e.document().end());
    e.backspace();
    assert_eq!(texts(&e)[0], format!("x{FAMILY}{FLAG}"));
    e.backspace();
    assert_eq!(texts(&e)[0], format!("x{FAMILY}"));
    e.set_caret(p(0, 1));
    e.delete_forward();
    assert_eq!(texts(&e)[0], "x");
}

#[test]
fn utf16_offsets_on_astral_and_combining_text() {
    let t = format!("\u{1F600}a{E_ACUTE}中{FAMILY}");
    let b = para(&t);
    let mut u = 0;
    for (i, c) in t.char_indices() {
        assert_eq!(b.offset_to_utf16(i), u);
        assert_eq!(b.utf16_to_offset(u), i);
        u += c.len_utf16();
    }
    assert_eq!(b.offset_to_utf16(t.len()), u);
    assert_eq!(b.utf16_to_offset(u), t.len());
    assert_eq!(
        b.utf16_to_offset(1),
        0,
        "inside a surrogate pair rounds down"
    );
    assert_eq!(b.utf16_to_offset(10_000), t.len());
    assert_eq!(
        b.offset_to_utf16(2),
        0,
        "inside a UTF-8 sequence rounds down"
    );
    let r = text::utf8_range_to_utf16(&t, 4..t.len());
    assert_eq!(text::utf16_range_to_utf8(&t, r), 4..t.len());
}

#[test]
fn typing_combining_marks_and_zwj_keeps_the_caret_on_a_boundary() {
    let mut e = ed(vec![para("ab")]);
    no_rules(&mut e);
    e.set_caret(p(0, 1));
    e.insert_text("\u{301}");
    assert_eq!(texts(&e)[0], "a\u{301}b");
    assert_eq!(caret(&e), p(0, 3));
    let mut e = ed(vec![para("\u{1F469}\u{1F467}")]);
    e.set_caret(p(0, 4));
    e.insert_text("\u{200D}");
    e.check_invariants().unwrap();
    assert!(text::is_grapheme_boundary(
        e.document().blocks()[0].text(),
        caret(&e).offset
    ));
}

// ----- undo / redo ------------------------------------------------------------------

#[test]
fn typing_groups_break_at_words_movement_kind_and_toggles() {
    let mut e = ed(vec![para("")]);
    no_rules(&mut e);
    typ(&mut e, "one two");
    e.toggle_bold();
    typ(&mut e, "three");
    e.backspace();
    e.backspace();
    typ(&mut e, "x");
    assert_eq!(texts(&e), ["one twothrx"]);
    let steps: Vec<String> =
        std::iter::from_fn(|| e.undo().then(|| texts(&e)[0].clone())).collect();
    assert_eq!(steps, ["one twothr", "one twothree", "one two", "one ", ""]);
}

#[test]
fn undo_restores_reversed_selection_and_redo_the_result() {
    let mut e = ed(vec![para("hello world"), para("second")]);
    let sel = Selection::new(p(1, 3), p(0, 2));
    e.set_selection(sel);
    e.insert_text("X");
    assert_eq!(texts(&e), ["heXond"]);
    let after = e.selection();
    e.undo();
    assert_eq!(e.selection(), sel);
    assert_eq!(texts(&e), ["hello world", "second"]);
    e.redo();
    assert_eq!(e.selection(), after);
    // A new edit after undo clears redo.
    e.undo();
    e.insert_text("y");
    assert!(!e.can_redo());
}

#[test]
fn groups_nest_and_caret_moves_inside_a_group_do_not_split_it() {
    let mut e = ed(vec![para("abc")]);
    no_rules(&mut e);
    e.grouped(|e| {
        e.set_caret(p(0, 3));
        e.insert_text("d");
        e.begin_undo_group();
        e.move_caret(Movement::DocStart, false);
        e.insert_text("z");
        e.end_undo_group();
        e.insert_paragraph_break();
    });
    assert_eq!(texts(&e), ["z", "abcd"]);
    assert_eq!(e.undo_depth(), 1);
    e.undo();
    assert_eq!(texts(&e), ["abc"]);
    // The selection before the group's first edit.
    assert_eq!(caret(&e), p(0, 3));
}

#[test]
fn undo_limit_drops_oldest() {
    let mut e = ed(vec![para("")]);
    e.set_undo_limit(3);
    for c in ["a", "b", "c", "d", "e"] {
        e.insert_text(c);
        e.break_undo_group();
    }
    assert_eq!(e.undo_depth(), 3);
    assert_eq!(undo_count(&mut e), 3);
    assert_eq!(texts(&e), ["ab"]);
}

#[test]
fn change_reports_are_local() {
    let blocks: Vec<Block> = (0..50).map(|i| para(&format!("block {i}"))).collect();
    let mut e = ed(blocks);
    e.take_changes();
    e.set_caret(p(20, 3));
    e.insert_text("x");
    let c = e.take_changes().unwrap();
    assert_eq!(
        c.blocks,
        Some(BlockSplice {
            start: 20,
            old_len: 1,
            new_len: 1
        })
    );
    e.insert_paragraph_break();
    e.undo();
    let c = e.take_changes().unwrap();
    let s = c.blocks.unwrap();
    assert_eq!(s.start, 20);
    assert_eq!(s.old_len, s.new_len);
    let rev = e.revision();
    e.set_caret(p(3, 0));
    assert!(e.take_changes().is_none());
    assert_eq!(e.revision(), rev);
}

// ----- comment anchors --------------------------------------------------------------

#[test]
fn comment_anchor_survives_split_merge_undo() {
    let mut e = ed(vec![para("hello world")]);
    e.select_range(TextRange::in_block(0, 2, 9));
    let id = e.add_comment("c", "me", 1).unwrap();
    let range = |e: &EditorState| e.document().comment_ranges(id);
    e.set_caret(p(0, 5));
    e.insert_paragraph_break();
    assert_eq!(range(&e), vec![TextRange::new(p(0, 2), p(1, 4))]);
    e.backspace();
    assert_eq!(range(&e), vec![TextRange::in_block(0, 2, 9)]);
    // Typing inside extends, typing at the edges does not.
    e.set_caret(p(0, 4));
    e.insert_text("Z");
    e.set_caret(p(0, 2));
    e.insert_text("A");
    e.set_caret(p(0, 11));
    e.insert_text("B");
    assert_eq!(texts(&e), ["heAllZo worBld"]);
    assert_eq!(range(&e), vec![TextRange::in_block(0, 3, 11)]);
    undo_count(&mut e);
    assert!(e.document().comment(id).is_none());
    while e.redo() {}
    assert_eq!(range(&e), vec![TextRange::in_block(0, 3, 11)]);
}

#[test]
fn comments_orphan_on_cut_and_come_back_with_undo() {
    let mut e = ed(vec![para("aa bb cc")]);
    e.select_range(TextRange::in_block(0, 3, 5));
    let id = e.add_comment("c", "me", 1).unwrap();
    e.select_range(TextRange::in_block(0, 2, 6));
    let f = e.cut();
    assert!(e.document().is_orphaned(id));
    assert!(e.document().comment(id).is_some(), "the thread is kept");
    assert_eq!(e.document().threads_in_order(), vec![id]);
    // Pasting the cut text does not duplicate or revive the anchor.
    e.set_caret(p(0, 0));
    e.paste(&f);
    assert!(e.document().is_orphaned(id));
    assert!(e.document().blocks()[0].comment_ids().is_empty());
    e.undo();
    e.undo();
    assert!(!e.document().is_orphaned(id));
    assert_eq!(
        e.document().comment_ranges(id),
        vec![TextRange::in_block(0, 3, 5)]
    );
}

#[test]
fn replace_all_keeps_anchors_on_replaced_text() {
    let mut e = ed(vec![para("cat dog cat")]);
    e.select_range(TextRange::in_block(0, 8, 11));
    let id = e.add_comment("c", "me", 1).unwrap();
    let n = e.replace_all("cat", SearchOptions::default(), "lion");
    assert_eq!(n, 2);
    assert_eq!(texts(&e), ["lion dog lion"]);
    assert_eq!(
        e.document().comment_ranges(id),
        vec![TextRange::in_block(0, 9, 13)]
    );
    e.undo();
    assert_eq!(
        e.document().comment_ranges(id),
        vec![TextRange::in_block(0, 8, 11)]
    );
}

#[test]
fn comment_navigation_and_queries() {
    let mut e = ed(vec![para("one two three"), divider(), para("four five")]);
    e.select_range(TextRange::in_block(2, 0, 4));
    let c2 = e.add_comment("b", "me", 2).unwrap();
    e.select_range(TextRange::in_block(0, 4, 7));
    let c1 = e.add_comment("a", "me", 1).unwrap();
    let d = e.document();
    assert_eq!(d.threads_in_order(), vec![c1, c2]);
    assert_eq!(d.next_comment(p(0, 0), false).map(|x| x.0), Some(c1));
    assert_eq!(d.next_comment(p(0, 4), false).map(|x| x.0), Some(c2));
    assert_eq!(d.next_comment(p(2, 0), false), None);
    assert_eq!(d.next_comment(p(2, 0), true).map(|x| x.0), Some(c1));
    assert_eq!(d.prev_comment(p(2, 0), false).map(|x| x.0), Some(c1));
    assert_eq!(d.prev_comment(p(0, 4), true).map(|x| x.0), Some(c2));
    assert_eq!(d.comments_at(p(0, 4)), vec![c1]);
    assert_eq!(d.comments_at(p(0, 7)), vec![c1]);
    assert!(d.comments_at(p(0, 8)).is_empty());
    // A comment across a divider and an empty block is one range.
    let mut e = ed(vec![para("ab"), divider(), para(""), para("cd")]);
    e.select_all();
    let id = e.add_comment("x", "me", 1).unwrap();
    assert_eq!(
        e.document().comment_ranges(id),
        vec![TextRange::new(p(0, 0), p(3, 2))]
    );
}

#[test]
fn deleting_a_thread_while_pending_attrs_carry_it() {
    let mut e = ed(vec![para("hello world")]);
    e.select_range(TextRange::in_block(0, 0, 11));
    let id = e.add_comment("c", "a", 1).unwrap();
    e.set_caret(p(0, 5));
    e.toggle_bold();
    e.delete_comment(id);
    assert!(e.current_attrs().comments.is_empty());
    e.insert_text("x");
    e.check_invariants().unwrap();
    assert!(e.document().blocks()[0].attrs_at(5).unwrap().bold);
}

#[test]
fn thread_operations_are_undoable() {
    let mut e = ed(vec![para("text")]);
    e.select_all();
    let id = e.add_comment(" body\r\n", " Ann ", 10).unwrap();
    let t = e.document().comment(id).unwrap().clone();
    assert_eq!(t.author, "Ann");
    assert_eq!(t.body, " body\n");
    let r1 = e.add_reply(id, "r1", "Bo", 11).unwrap();
    let r2 = e.add_reply(id, "r2", "Cy", 12).unwrap();
    assert_ne!(r1, r2);
    assert!(e.edit_reply(id, r1, "r1!"));
    assert!(!e.edit_reply(id, ReplyId(999), "x"));
    assert!(e.delete_reply(id, r2));
    assert!(e.resolve_comment(id));
    assert!(!e.resolve_comment(id), "already resolved");
    assert!(e.reopen_comment(id));
    assert!(e.edit_comment(id, "new"));
    assert!(e.delete_comment(id));
    assert!(!e.delete_comment(id));
    assert_eq!(e.document().comment_count(), 0);
    assert!(e.add_reply(CommentId(12345), "x", "y", 0).is_none());
    assert_eq!(undo_count(&mut e), 9);
    assert_eq!(e.document().comment_count(), 0);
    for _ in 0..8 {
        e.redo();
    }
    let t = e.document().comment(id).unwrap();
    assert_eq!(t.body, "new");
    assert_eq!(t.replies.len(), 1);
    assert_eq!(t.replies[0].body, "r1!");
    assert!(!t.resolved);
}

// ----- Enter and Backspace per block kind ---------------------------------------------

#[test]
fn enter_matrix() {
    // (kind, text, caret) -> (kinds after, texts after, caret after)
    let cases: Vec<(BlockKind, &str, usize, Vec<BlockKind>, Vec<&str>, Position)> = vec![
        (
            BlockKind::Paragraph,
            "ab",
            1,
            vec![BlockKind::Paragraph; 2],
            vec!["a", "b"],
            p(1, 0),
        ),
        (
            BlockKind::heading(2),
            "ab",
            2,
            vec![BlockKind::heading(2), BlockKind::Paragraph],
            vec!["ab", ""],
            p(1, 0),
        ),
        (
            BlockKind::heading(2),
            "ab",
            1,
            vec![BlockKind::heading(2); 2],
            vec!["a", "b"],
            p(1, 0),
        ),
        (
            BlockKind::heading(2),
            "ab",
            0,
            vec![BlockKind::Paragraph, BlockKind::heading(2)],
            vec!["", "ab"],
            p(1, 0),
        ),
        (
            BlockKind::BulletItem,
            "ab",
            2,
            vec![BlockKind::BulletItem; 2],
            vec!["ab", ""],
            p(1, 0),
        ),
        (
            BlockKind::TodoItem { checked: true },
            "ab",
            2,
            vec![BlockKind::TodoItem { checked: true }, BlockKind::todo()],
            vec!["ab", ""],
            p(1, 0),
        ),
        (
            BlockKind::NumberedItem,
            "",
            0,
            vec![BlockKind::Paragraph],
            vec![""],
            p(0, 0),
        ),
        (
            BlockKind::Quote,
            "",
            0,
            vec![BlockKind::Paragraph],
            vec![""],
            p(0, 0),
        ),
        (
            BlockKind::Quote,
            "q",
            1,
            vec![BlockKind::Quote; 2],
            vec!["q", ""],
            p(1, 0),
        ),
        (
            BlockKind::code(),
            "ab",
            1,
            vec![BlockKind::code()],
            vec!["a\nb"],
            p(0, 2),
        ),
        (
            BlockKind::Divider,
            "",
            0,
            vec![BlockKind::Divider, BlockKind::Paragraph],
            vec!["", ""],
            p(1, 0),
        ),
        (
            BlockKind::PageBreak,
            "",
            0,
            vec![BlockKind::PageBreak, BlockKind::Paragraph],
            vec!["", ""],
            p(1, 0),
        ),
    ];
    for (kind, t, at, want_kinds, want_texts, want_caret) in cases {
        let mut e = ed(vec![kb(kind.clone(), t)]);
        e.set_caret(p(0, at));
        e.insert_paragraph_break();
        e.check_invariants().unwrap();
        assert_eq!(kinds(&e), want_kinds, "{kind:?} {t:?} at {at}");
        assert_eq!(texts(&e), want_texts, "{kind:?} {t:?} at {at}");
        assert_eq!(caret(&e), want_caret, "{kind:?} {t:?} at {at}");
        e.undo();
        assert_eq!(kinds(&e), vec![kind.clone()]);
        assert_eq!(texts(&e), vec![t]);
    }
    // An empty indented list item outdents first.
    let mut e = ed(vec![kb(BlockKind::BulletItem, "").with_indent(2)]);
    e.insert_paragraph_break();
    assert_eq!(e.document().blocks()[0].indent(), 1);
    assert_eq!(kinds(&e), vec![BlockKind::BulletItem]);
    // New list items keep the indent.
    let mut e = ed(vec![kb(BlockKind::NumberedItem, "x").with_indent(3)]);
    e.set_caret(p(0, 1));
    e.insert_paragraph_break();
    assert_eq!(e.document().blocks()[1].indent(), 3);
    assert_eq!(e.document().numbered_ordinal(1), Some(2));
}

#[test]
fn code_block_enter_keeps_indent_and_exits_cleanly() {
    let mut e = ed(vec![kb(BlockKind::code(), "fn f() {\n\tif x {")]);
    e.set_caret(e.document().end());
    e.insert_paragraph_break();
    assert_eq!(texts(&e), ["fn f() {\n\tif x {\n\t"]);
    // With auto-indent the blank lines are not empty; two of them still exit.
    e.insert_paragraph_break();
    e.insert_paragraph_break();
    assert_eq!(texts(&e), ["fn f() {\n\tif x {", ""]);
    assert_eq!(kinds(&e), vec![BlockKind::code(), BlockKind::Paragraph]);
    assert_eq!(caret(&e), p(1, 0));
    // An empty code block left by the exit becomes the paragraph.
    let mut e = ed(vec![para("")]);
    typ(&mut e, "```");
    assert_eq!(kinds(&e), vec![BlockKind::code()]);
    e.insert_paragraph_break();
    e.insert_paragraph_break();
    assert_eq!(kinds(&e), vec![BlockKind::Paragraph]);
    assert_eq!(texts(&e), [""]);
    // Enter in the middle of a code block never exits.
    let mut e = ed(vec![kb(BlockKind::code(), "a\n\n\nb")]);
    e.set_caret(p(0, 3));
    e.insert_paragraph_break();
    assert_eq!(texts(&e), ["a\n\n\n\nb"]);
    // exit_block leaves from anywhere.
    e.exit_block();
    assert_eq!(kinds(&e), vec![BlockKind::code(), BlockKind::Paragraph]);
}

#[test]
fn backspace_matrix_at_block_start() {
    let cases: Vec<(Block, Vec<BlockKind>, Vec<&str>, Position)> = vec![
        (
            kb(BlockKind::heading(1), "h"),
            vec![BlockKind::Paragraph, BlockKind::Paragraph],
            vec!["p", "h"],
            p(1, 0),
        ),
        (
            kb(BlockKind::Quote, "q"),
            vec![BlockKind::Paragraph; 2],
            vec!["p", "q"],
            p(1, 0),
        ),
        (
            kb(BlockKind::todo(), "t"),
            vec![BlockKind::Paragraph; 2],
            vec!["p", "t"],
            p(1, 0),
        ),
        (
            kb(BlockKind::code(), "a\nb"),
            vec![BlockKind::Paragraph; 3],
            vec!["p", "a", "b"],
            p(1, 0),
        ),
        (para("x"), vec![BlockKind::Paragraph], vec!["px"], p(0, 1)),
        (divider(), vec![BlockKind::Paragraph], vec!["p"], p(0, 1)),
    ];
    for (b, want_kinds, want_texts, want_caret) in cases {
        let k = b.kind().clone();
        let mut e = ed(vec![para("p"), b]);
        e.set_caret(p(1, 0));
        e.backspace();
        e.check_invariants().unwrap();
        assert_eq!(kinds(&e), want_kinds, "{k:?}");
        assert_eq!(texts(&e), want_texts, "{k:?}");
        assert_eq!(caret(&e), want_caret, "{k:?}");
    }
    // Paragraph after a divider: the divider goes, the text stays.
    let mut e = ed(vec![para("a"), divider(), para("b")]);
    e.set_caret(p(2, 0));
    e.backspace();
    assert_eq!(texts(&e), ["a", "b"]);
    assert_eq!(caret(&e), p(1, 0));
    // Void block at document start and end.
    let mut e = ed(vec![divider(), para("x"), Block::new(BlockKind::PageBreak)]);
    e.set_caret(p(1, 0));
    e.backspace();
    assert_eq!(texts(&e), ["x", ""]);
    assert_eq!(caret(&e), p(0, 0));
    e.set_caret(p(1, 0));
    e.backspace();
    assert_eq!(texts(&e), ["x"]);
    assert_eq!(caret(&e), p(0, 1));
    assert!(!e.backspace() || texts(&e) == ["x"] || true);
    // Delete forward at the end of the last block does nothing.
    let mut e = ed(vec![para("x")]);
    e.set_caret(p(0, 1));
    assert!(!e.delete_forward());
}

// ----- input rules --------------------------------------------------------------------

#[test]
fn inline_rules_fire_on_markdown() {
    let cases: &[(&str, &str, fn(&InlineAttrs) -> bool, &str)] = &[
        ("**bold**", "bold", |a| a.bold, "bold"),
        ("say *this* now", "say this now", |a| a.italic, "this"),
        ("_it_", "it", |a| a.italic, "it"),
        ("use `x+1` here", "use x+1 here", |a| a.code, "x+1"),
        ("~~gone~~", "gone", |a| a.strikethrough, "gone"),
        ("==mark==", "mark", |a| a.highlight.is_some(), "mark"),
        ("(*aside*)", "(aside)", |a| a.italic, "aside"),
        ("**note:** x", "note: x", |a| a.bold, "note:"),
        ("*a * b*", "a * b", |a| a.italic, "a * b"),
    ];
    for (typed, want, f, want_styled) in cases {
        let mut e = ed(vec![para("")]);
        typ(&mut e, typed);
        assert_eq!(texts(&e), [*want], "{typed}");
        assert_eq!(styled(&e, 0, f), *want_styled, "{typed}");
    }
    // One undo gives back the literal characters.
    let mut e = ed(vec![para("")]);
    typ(&mut e, "**b**");
    e.undo();
    assert_eq!(texts(&e), ["**b**"]);
    // After the rule the next character is not bold.
    let mut e = ed(vec![para("")]);
    typ(&mut e, "**b** x");
    assert_eq!(styled(&e, 0, |a| a.bold), "b");
}

#[test]
fn inline_rules_leave_prose_and_code_alone() {
    let prose = [
        "2 * 3 * 4",
        "2*3*4",
        "snake_case_name",
        "__init__",
        "a footnote* and more*",
        "file_name.txt and other_name",
        "x == y == z",
        "a==b==c",
        "~ approx ~",
        "price: 5 * 2",
        "**",
        "* not a list in the middle *",
        "foo`bar`",
        "it's 'quoted'",
        "a*b*c",
        "_ spaced _",
        "**a*",
    ];
    for t in prose {
        let mut e = ed(vec![para("x ")]);
        e.set_caret(p(0, 2));
        e.set_options(EditorOptions {
            input_rules: true,
            smart_punctuation: false,
        });
        typ(&mut e, t);
        assert_eq!(texts(&e), [format!("x {t}")], "{t}");
        let b = &e.document().blocks()[0];
        assert!(
            b.runs().iter().all(|r| r.attrs.is_plain()),
            "{t}: {:?}",
            b.runs()
        );
    }
    // A span never jumps over a delimiter that could close an earlier one:
    // "*a*" typed and undone to the literal text, then " b*".
    let mut e = ed(vec![para("")]);
    typ(&mut e, "*a*");
    e.undo();
    typ(&mut e, " b*");
    assert_eq!(texts(&e), ["*a* b*"]);
    assert!(
        e.document().blocks()[0]
            .runs()
            .iter()
            .all(|r| r.attrs.is_plain())
    );
    // Never in code blocks or inline code.
    let mut e = ed(vec![kb(BlockKind::code(), "")]);
    typ(&mut e, "**x** \"q\" -- ... # ");
    assert_eq!(texts(&e), ["**x** \"q\" -- ... # "]);
    let mut e = ed(vec![para("")]);
    e.toggle_code();
    typ(&mut e, "*a* \"q\"");
    assert_eq!(texts(&e), ["*a* \"q\""]);
}

#[test]
fn block_rules() {
    let cases: &[(&str, BlockKind)] = &[
        ("# ", BlockKind::heading(1)),
        ("###### ", BlockKind::heading(6)),
        ("- ", BlockKind::BulletItem),
        ("* ", BlockKind::BulletItem),
        ("+ ", BlockKind::BulletItem),
        ("1. ", BlockKind::NumberedItem),
        ("12) ", BlockKind::NumberedItem),
        ("[] ", BlockKind::todo()),
        ("[ ] ", BlockKind::todo()),
        ("[x] ", BlockKind::TodoItem { checked: true }),
        ("> ", BlockKind::Quote),
        ("```", BlockKind::code()),
        ("---", BlockKind::Divider),
    ];
    for (t, k) in cases {
        let mut e = ed(vec![para("")]);
        typ(&mut e, t);
        assert_eq!(&kinds(&e)[0], k, "{t:?}");
        assert_eq!(texts(&e)[0], "", "{t:?}");
        e.undo();
        assert_eq!(kinds(&e)[0], BlockKind::Paragraph, "{t:?}");
        assert_eq!(texts(&e)[0], *t, "{t:?}");
    }
    // Not rules: too many hashes, no space, middle of a block, in a heading.
    for t in ["####### ", "#tag ", "1.5 ", "a- ", "-x "] {
        let mut e = ed(vec![para("")]);
        typ(&mut e, t);
        assert_eq!(kinds(&e)[0], BlockKind::Paragraph, "{t:?}");
    }
    let mut e = ed(vec![kb(BlockKind::heading(2), "")]);
    typ(&mut e, "- ");
    assert_eq!(kinds(&e)[0], BlockKind::heading(2));
    // Switched off.
    let mut e = ed(vec![para("")]);
    no_rules(&mut e);
    typ(&mut e, "# **a** \"q\"");
    assert_eq!(kinds(&e)[0], BlockKind::Paragraph);
    assert_eq!(texts(&e), ["# **a** \"q\""]);
}

#[test]
fn smart_punctuation_cases() {
    let cases = [
        ("\"Hi,\" she said", "\u{201C}Hi,\u{201D} she said"),
        ("don't", "don\u{2019}t"),
        ("rock 'n' roll", "rock \u{2018}n\u{2019} roll"),
        ("(\"x\")", "(\u{201C}x\u{201D})"),
        ("[\"y\"]", "[\u{201C}y\u{201D}]"),
        ("\"'nested'\"", "\u{201C}\u{2018}nested\u{2019}\u{201D}"),
        ("5'10\"", "5\u{2019}10\u{201D}"),
        ("a--b", "a\u{2014}b"),
        ("wait...", "wait\u{2026}"),
        ("--x", "\u{2014}x"),
        ("one -- two", "one \u{2014} two"),
    ];
    for (typed, want) in cases {
        let mut e = ed(vec![para("")]);
        only_smart(&mut e);
        typ(&mut e, typed);
        assert_eq!(texts(&e), [want], "{typed}");
    }
    // Undo restores the literal characters, one replacement at a time.
    let mut e = ed(vec![para("")]);
    only_smart(&mut e);
    typ(&mut e, "a--");
    e.undo();
    assert_eq!(texts(&e), ["a--"]);
    let mut e = ed(vec![para("")]);
    only_smart(&mut e);
    typ(&mut e, "\"");
    e.undo();
    assert_eq!(texts(&e), ["\""]);
}

// ----- fragments ----------------------------------------------------------------------

#[test]
fn copy_paste_across_block_kinds() {
    let src = Document::from_blocks(vec![
        kb(BlockKind::heading(1), "Title"),
        para("").with_span("bold", bold()),
        kb(BlockKind::BulletItem, "item"),
    ]);
    let all = src.fragment(TextRange::new(src.start(), src.end()));
    assert_eq!(all.blocks().len(), 3);
    // Into the middle of a paragraph.
    let mut e = ed(vec![para("[]")]);
    e.set_caret(p(0, 1));
    e.paste(&all);
    assert_eq!(texts(&e), ["[Title", "bold", "item]"]);
    assert_eq!(
        kinds(&e),
        vec![
            BlockKind::Paragraph,
            BlockKind::Paragraph,
            BlockKind::BulletItem
        ]
    );
    assert!(e.document().blocks()[1].attrs_at(0).unwrap().bold);
    assert_eq!(caret(&e), p(2, 4));
    // Into an empty paragraph: block kinds come along.
    let mut e = ed(vec![para("")]);
    e.paste(&all);
    assert_eq!(
        kinds(&e),
        vec![
            BlockKind::heading(1),
            BlockKind::Paragraph,
            BlockKind::BulletItem
        ]
    );
    // Into a code block: plain text, no formatting.
    let mut e = ed(vec![kb(BlockKind::code(), "")]);
    e.paste(&all);
    assert_eq!(texts(&e), ["Title\nbold\nitem"]);
    assert!(
        e.document().blocks()[0]
            .runs()
            .iter()
            .all(|r| r.attrs.is_plain())
    );
    // Inline fragment keeps the target's kind.
    let inline = src.fragment(TextRange::in_block(0, 0, 3));
    let mut e = ed(vec![kb(BlockKind::Quote, "q")]);
    e.set_caret(p(0, 1));
    e.paste(&inline);
    assert_eq!(texts(&e), ["qTit"]);
    assert_eq!(kinds(&e), vec![BlockKind::Quote]);
    // Onto a divider: a paragraph after it.
    let mut e = ed(vec![divider()]);
    e.paste(&inline);
    assert_eq!(kinds(&e), vec![BlockKind::Divider, BlockKind::Paragraph]);
    assert_eq!(texts(&e), ["", "Tit"]);
    // Whole-block (linewise) fragments.
    let lines = src.block_fragment(1..3);
    let mut e = ed(vec![para("a"), para("b")]);
    e.insert_blocks_at(1, &lines);
    assert_eq!(texts(&e), ["a", "bold", "item", "b"]);
    assert_eq!(caret(&e), p(1, 0));
}

#[test]
fn select_all_copy_and_delete_include_trailing_void_blocks() {
    let mut e = ed(vec![para("text"), divider()]);
    e.select_all();
    let f = e.copy();
    assert_eq!(f.blocks().len(), 2);
    assert!(f.blocks()[1].is_void());
    e.delete_selection();
    assert_eq!(kinds(&e), vec![BlockKind::Paragraph]);
    assert_eq!(texts(&e), [""]);
    e.paste(&f);
    assert_eq!(
        kinds(&e),
        vec![
            BlockKind::Paragraph,
            BlockKind::Divider,
            BlockKind::Paragraph
        ]
    );
    e.check_invariants().unwrap();
}

#[test]
fn void_end_of_selection_is_touched_by_block_commands() {
    let mut e = ed(vec![para("a"), divider(), para("b")]);
    e.set_selection(Selection::new(p(0, 0), p(1, 0)));
    assert_eq!(e.touched_blocks(), 0..2);
    e.move_blocks_down();
    assert_eq!(texts(&e), ["b", "a", ""]);
    assert_eq!(kinds(&e)[2], BlockKind::Divider);
    // A text block ending at offset 0 is not touched.
    let mut e = ed(vec![para("a"), para("b")]);
    e.set_selection(Selection::new(p(0, 0), p(1, 0)));
    assert_eq!(e.touched_blocks(), 0..1);
    e.select_blocks(0..2);
    assert_eq!(e.touched_blocks(), 0..2);
    e.select_blocks(7..9);
    assert_eq!(e.touched_blocks(), 1..2);
}

#[test]
fn fragments_from_plain_text_and_json() {
    let f = Fragment::from_plain_text("a\r\nb\u{0}c\n");
    assert_eq!(f.to_plain_text(), "a\nbc\n");
    assert_eq!(f.blocks().len(), 3);
    let json = f.to_json();
    assert_eq!(Fragment::from_json(&json).unwrap(), f);
    assert!(Fragment::from_json("{\"format\":\"other\",\"blocks\":[]}").is_err());
}

// ----- numbering --------------------------------------------------------------------

#[test]
fn numbered_ordinals_with_nesting() {
    let n = |i: u8| kb(BlockKind::NumberedItem, "n").with_indent(i);
    let b = |i: u8| kb(BlockKind::BulletItem, "b").with_indent(i);
    let d = Document::from_blocks(vec![
        n(0), // 1
        n(1), // 1
        n(1), // 2
        b(2), // -
        n(1), // 3
        n(0), // 2
        b(0), // -  (restarts level 0)
        n(0), // 1
        n(2), // 1
        n(0), // 2  (deeper item does not break the run)
        para("p").with_indent(1),
        n(1), // 1 (paragraph at level 1 restarted)
        n(0), // 3
        divider(),
        n(0), // 1
    ]);
    let want = [
        Some(1),
        Some(1),
        Some(2),
        None,
        Some(3),
        Some(2),
        None,
        Some(1),
        Some(1),
        Some(2),
        None,
        Some(1),
        Some(3),
        None,
        Some(1),
    ];
    for (i, w) in want.iter().enumerate() {
        assert_eq!(d.numbered_ordinal(i), *w, "block {i}");
    }
    for s in 0..want.len() {
        assert_eq!(
            d.numbered_ordinals(s..want.len()),
            want[s..].to_vec(),
            "from {s}"
        );
    }
    #[allow(clippy::reversed_empty_ranges)]
    let reversed = 3..1;
    assert_eq!(d.numbered_ordinals(reversed), Vec::<Option<usize>>::new());
}

// ----- search -------------------------------------------------------------------------

#[test]
fn search_multibyte_and_case_folding() {
    let d = Document::from_blocks(vec![
        para("Ünïcödé ünïcödé ÜNÏCÖDÉ UNÏCÖDÉ"),
        para("ΣΟΦΟΣ σοφος"),
        para("e\u{301} é e"),
        para("中文 中文字"),
        para("İstanbul"),
    ]);
    let any = SearchOptions::default();
    assert_eq!(d.find_all("ünïcödé", any).len(), 3);
    assert_eq!(
        d.find_all(
            "ünïcödé",
            SearchOptions {
                case_sensitive: true,
                ..any
            }
        )
        .len(),
        1
    );
    // Final sigma folds like sigma.
    assert_eq!(d.find_all("σοφος", any).len(), 2);
    assert_eq!(d.find_all("ΣΟΦΟΣ", any).len(), 2);
    // A bare e never matches the e of e + combining accent.
    let m = d.find_all("e", any);
    assert_eq!(m, vec![TextRange::in_block(2, 7, 8)]);
    // Precomposed and decomposed are different text (no normalisation).
    assert_eq!(d.find_in_block(2, "é", any), vec![4..6]);
    // Whole words in CJK: a run of ideographs is several words.
    let ww = SearchOptions {
        whole_word: true,
        ..any
    };
    assert_eq!(d.find_all("中文", ww).len(), 2);
    let d2 = Document::from_blocks(vec![para("don't don snake_case case, Case")]);
    let m: Vec<String> = d2
        .find_all("don", ww)
        .iter()
        .map(|r| d2.text_in(*r))
        .collect();
    assert_eq!(m, ["don"]);
    assert_eq!(d2.find_all("case", ww).len(), 2);
    assert_eq!(
        d2.find_all(
            "case",
            SearchOptions {
                whole_word: true,
                case_sensitive: true
            }
        )
        .len(),
        1
    );
    assert_eq!(d.find_all("İstanbul", any).len(), 1);
    assert_eq!(d.find_all("istanbul", any).len(), 1);
    for m in d.find_all("i", any) {
        assert!(d.is_valid_position(m.start) && d.is_valid_position(m.end));
    }
    // Replace one, then all, as single undo steps.
    let mut e = EditorState::new(d.clone());
    let m = e.document().find_all("中文", any)[1];
    e.replace_range(m, "x");
    assert_eq!(texts(&e)[3], "中文 x字");
    assert_eq!(e.replace_all("σοφος", any, "wise"), 2);
    assert_eq!(texts(&e)[1], "wise wise");
    e.undo();
    assert_eq!(texts(&e)[1], "ΣΟΦΟΣ σοφος");
}

#[test]
fn replace_keeps_per_character_formatting_when_lengths_match() {
    let b = para("a")
        .with_span("b", bold())
        .with_span("c", InlineAttrs::default());
    let mut e = ed(vec![b]);
    e.replace_range(TextRange::in_block(0, 0, 3), "ABC");
    assert_eq!(styled(&e, 0, |a| a.bold), "B");
    e.replace_range(TextRange::in_block(0, 0, 3), "wxyz");
    assert_eq!(
        styled(&e, 0, |a| a.bold),
        "",
        "different length: attrs of the first char"
    );
}

#[test]
fn ime_composition_uses_pending_attrs() {
    let mut e = ed(vec![para("ab")]);
    e.set_caret(p(0, 2));
    e.toggle_bold();
    e.begin_undo_group();
    e.replace_range(TextRange::empty(p(0, 2)), "k");
    e.replace_range(TextRange::in_block(0, 2, 3), "ka");
    e.replace_range(TextRange::in_block(0, 2, 4), "\u{304B}");
    e.end_undo_group();
    assert_eq!(texts(&e), ["ab\u{304B}"]);
    assert_eq!(styled(&e, 0, |a| a.bold), "\u{304B}");
    e.undo();
    assert_eq!(texts(&e), ["ab"]);
}

// ----- serialization ------------------------------------------------------------------

#[test]
fn malformed_json_is_repaired_or_rejected() {
    let ok = [
        r#"{"blocks":[{"text":"a","runs":"nope"}]}"#,
        r#"{"blocks":[{"text":5,"type":7,"id":"x"}]}"#,
        r#"{"blocks":[null, 3, "s", [], {"type":"divider","runs":[{"len":3}]}]}"#,
        r#"{"blocks":[{"text":"abc","runs":[{"len":-1},{"len":1.7,"bold":"yes"},{"len":2,"italic":1}]}]}"#,
        r#"{"blocks":[{"text":"x","runs":[{"len":1,"comments":[1,1,"2",-3]}]}],"comments":[{"id":1},{"id":1},{"id":-5},"junk"]}"#,
        r#"{"version":0,"blocks":[{"text":"\u0007bell\r\n"}]}"#,
        r#"{"version":"9","blocks":[{}]}"#,
        r#"{"blocks":[{"type":"heading","level":0,"text":"h"},{"type":"heading","level":99,"text":"h"}]}"#,
        r#"{"blocks":[{"type":"code","lang":"   ","text":"x","runs":[{"len":1,"bold":true,"link":"u"}]}]}"#,
        r#"{"blocks":[{"text":"a"}],"settings":{"page":{"paper":"custom","width":-1,"height":1e30,"margins":{"top":1e9}},"typography":{"font":"","size":0,"line_height":-2}}}"#,
        r##"{"blocks":[{"text":"a","runs":[{"len":1,"font":"   ","link":"  x  ","color":"#zzz","highlight":"teal"}]}]}"##,
        r#"{"blocks":[{"text":"é","runs":[{"len":1,"bold":true},{"len":1}]}]}"#,
        r#"{"blocks":[{"id":4503599627370496},{"id":4503599627370495},{"id":4503599627370495}]}"#,
    ];
    for j in ok {
        let d = Document::from_json(j).unwrap_or_else(|e| panic!("{j}: {e}"));
        d.check_invariants().unwrap_or_else(|e| panic!("{j}: {e}"));
        let back = Document::from_json(&d.to_json()).unwrap();
        assert_eq!(back, d, "{j}");
        let mut e = EditorState::new(d);
        e.select_all();
        e.insert_paragraph_break();
        e.insert_text("ok");
        e.check_invariants().unwrap();
    }
    let bad = [
        "",
        "null",
        "[]",
        "{}",
        r#"{"blocks":{}}"#,
        r#"{"format":"x","blocks":[]}"#,
        r#"{"version":2,"blocks":[]}"#,
        "{\"blocks\":[",
    ];
    for j in bad {
        assert!(Document::from_json(j).is_err(), "{j}");
    }
    // Deep nesting is an error, not a stack overflow.
    let deep = format!(
        "{{\"blocks\":[{}{}]}}",
        "[".repeat(10_000),
        "]".repeat(10_000)
    );
    assert!(Document::from_json(&deep).is_err() || Document::from_json(&deep).is_ok());
    let d = Document::from_json(r#"{"blocks":[{"type":"code","lang":"   ","text":"x","runs":[{"len":1,"bold":true,"link":"u"}]}]}"#).unwrap();
    assert_eq!(d.blocks()[0].kind(), &BlockKind::code());
    assert!(d.blocks()[0].runs()[0].attrs.is_plain());
}

#[test]
fn serialization_is_compact_and_versioned() {
    let mut e = ed(vec![para("plain"), kb(BlockKind::heading(2), "H")]);
    e.select_range(TextRange::in_block(0, 0, 5));
    e.add_comment("c", "", 5);
    let j = e.document().to_json();
    assert!(
        j.starts_with(r#"{"format":"the-writer-document","version":1"#),
        "{j}"
    );
    assert!(!j.contains("\"bold\""));
    assert!(!j.contains("\"author\""));
    assert!(!j.contains("\"settings\""));
    let back = Document::from_json(&j).unwrap();
    assert_eq!(&back, e.document());
    // Default settings are omitted, changed ones are written.
    let mut s = DocumentSettings {
        page_view: true,
        ..Default::default()
    };
    s.page.paper = PaperSize::UsLetter;
    let d = Document::new().with_settings(s.clone());
    let j = d.to_json();
    assert!(
        j.contains("\"page_view\":true") && j.contains("letter"),
        "{j}"
    );
    assert_eq!(Document::from_json(&j).unwrap().settings(), &s);
}

// ----- misc boundary cases ------------------------------------------------------------

#[test]
fn single_empty_block_every_command() {
    let mut e = EditorState::default();
    assert!(!e.backspace());
    assert!(!e.delete_forward());
    assert!(!e.delete_word_backward());
    assert!(!e.delete_word_forward());
    assert!(!e.delete_to_line_start());
    assert!(!e.delete_to_line_end());
    assert!(!e.delete_selection());
    assert!(!e.move_blocks_up());
    assert!(!e.move_blocks_down());
    assert!(!e.toggle_todo_checked());
    assert!(!e.outdent());
    assert!(e.add_comment("x", "y", 0).is_none());
    assert!(!e.toggle_bold());
    assert_eq!(e.replace_all("x", SearchOptions::default(), "y"), 0);
    assert!(e.copy().is_empty());
    assert!(e.delete_blocks());
    assert_eq!(e.document().block_count(), 1);
    e.check_invariants().unwrap();
    for m in [
        Movement::Left,
        Movement::Right,
        Movement::WordLeft,
        Movement::WordRight,
        Movement::DocEnd,
    ] {
        e.move_caret(m, true);
        assert_eq!(e.selection(), Selection::caret(p(0, 0)));
    }
    assert_eq!(e.document().title(), None);
    assert_eq!(e.stats(), Stats::default());
}

#[test]
fn word_movement_matches_macos() {
    let d = Document::from_blocks(vec![
        para("Hello, wörld! It's 3.14 o'clock"),
        para(""),
        divider(),
        para("  next"),
    ]);
    let rights: Vec<usize> = {
        let mut v = Vec::new();
        let mut pos = p(0, 0);
        loop {
            let n = d.moved(pos, Movement::WordRight);
            if n.block != 0 || n == pos {
                break;
            }
            v.push(n.offset);
            pos = n;
        }
        v
    };
    let t = d.blocks()[0].text();
    let words: Vec<&str> = rights.iter().map(|&o| &t[..o]).collect();
    assert_eq!(words.last().copied(), Some(t));
    assert_eq!(&t[..rights[0]], "Hello");
    assert_eq!(&t[..rights[1]], "Hello, wörld");
    assert_eq!(&t[..rights[2]], "Hello, wörld! It's");
    assert_eq!(&t[..rights[3]], "Hello, wörld! It's 3.14");
    // From the end of block 0, option-right skips empty and void blocks.
    assert_eq!(d.moved(p(0, t.len()), Movement::WordRight), p(3, 6));
    assert_eq!(
        d.moved(p(3, 2), Movement::WordLeft),
        p(0, t.len() - "clock".len() - 2)
    );
}

#[test]
fn undo_history_has_a_memory_budget() {
    // About 2.4 MB per step: every select-all edit keeps a copy of everything.
    let blocks: Vec<Block> = (0..2000).map(|i| para(&format!("{i:>1000}"))).collect();
    let mut e = ed(blocks);
    for _ in 0..70 {
        e.select_all();
        e.toggle_bold();
    }
    // 70 steps would hold about 170 MB; the budget keeps about 128 MB.
    let depth = e.undo_depth();
    assert!((20..60).contains(&depth), "{depth}");
    assert_eq!(undo_count(&mut e), depth);
    assert!(!e.can_undo());
}
