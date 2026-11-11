//! Unit tests for the editing engine.

use super::*;
use crate::attrs::{Highlight, InlineStyle, Rgb};
use crate::block::{Alignment, BlockKind};
use crate::fragment::Fragment;
use crate::search::SearchOptions;

const FAMILY: &str = "\u{1F469}\u{200D}\u{1F469}\u{200D}\u{1F467}";
const E_ACUTE: &str = "e\u{301}";

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
    e.document().blocks().iter().map(|b| b.text().to_string()).collect()
}

fn kinds(e: &EditorState) -> Vec<BlockKind> {
    e.document().blocks().iter().map(|b| b.kind().clone()).collect()
}

fn typ(e: &mut EditorState, s: &str) {
    for c in s.chars() {
        e.insert_text(&c.to_string());
        e.check_invariants().unwrap();
    }
}

fn check(e: &EditorState) {
    e.check_invariants().unwrap();
}

fn caret(e: &EditorState) -> Position {
    assert!(e.selection().is_collapsed(), "expected a caret, got {:?}", e.selection());
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

fn undo_all(e: &mut EditorState) {
    while e.undo() {
        check(e);
    }
}

// ----- typing and undo grouping ---------------------------------------------------------

#[test]
fn typing_coalesces_by_word() {
    let mut e = EditorState::default();
    typ(&mut e, "hello big world");
    assert_eq!(texts(&e), ["hello big world"]);
    assert_eq!(e.undo_depth(), 3);
    e.undo();
    assert_eq!(texts(&e), ["hello big "]);
    assert_eq!(caret(&e), p(0, 10));
    e.undo();
    assert_eq!(texts(&e), ["hello "]);
    e.undo();
    assert_eq!(texts(&e), [""]);
    assert!(!e.undo());
    e.redo();
    e.redo();
    e.redo();
    assert_eq!(texts(&e), ["hello big world"]);
    assert_eq!(caret(&e), p(0, 15));
    assert!(!e.redo());
}

#[test]
fn group_breaks_on_movement_and_explicit_break() {
    let mut e = EditorState::default();
    typ(&mut e, "ab");
    e.break_undo_group();
    typ(&mut e, "cd");
    e.move_caret(Movement::Left, false);
    e.move_caret(Movement::Right, false);
    typ(&mut e, "ef");
    assert_eq!(e.undo_depth(), 3);
    e.undo();
    assert_eq!(texts(&e), ["abcd"]);
    // A new edit after undo clears redo.
    typ(&mut e, "x");
    assert!(!e.can_redo());
    assert_eq!(texts(&e), ["abcdx"]);
}

#[test]
fn backspaces_coalesce_by_word() {
    let mut e = ed(vec![para("one two")]);
    e.set_caret(p(0, 7));
    for _ in 0..7 {
        e.backspace();
        check(&e);
    }
    assert_eq!(texts(&e), [""]);
    assert_eq!(e.undo_depth(), 2);
    e.undo();
    assert_eq!(texts(&e), ["one "]);
    e.undo();
    assert_eq!(texts(&e), ["one two"]);
    assert_eq!(caret(&e), p(0, 7));
}

#[test]
fn typing_over_selection_restores_selection_on_undo() {
    let mut e = ed(vec![para("hello world")]);
    let sel = Selection::new(p(0, 6), p(0, 11));
    e.set_selection(sel);
    typ(&mut e, "you");
    assert_eq!(texts(&e), ["hello you"]);
    e.undo();
    assert_eq!(texts(&e), ["hello world"]);
    assert_eq!(e.selection(), sel);
}

#[test]
fn undo_groups_nest_and_undo_limit() {
    let mut e = EditorState::default();
    e.begin_undo_group();
    typ(&mut e, "abc def");
    e.begin_undo_group();
    e.insert_paragraph_break();
    e.end_undo_group();
    typ(&mut e, "x");
    e.end_undo_group();
    assert_eq!(e.undo_depth(), 1);
    e.undo();
    assert_eq!(texts(&e), [""]);
    e.redo();
    assert_eq!(texts(&e), ["abc def", "x"]);
    let r = e.grouped(|e| {
        e.select_all();
        e.delete_selection()
    });
    assert!(r);
    assert_eq!(e.undo_depth(), 2);

    let mut e = EditorState::default();
    e.set_undo_limit(3);
    for i in 0..10 {
        e.insert_text(&format!("w{i} "));
        e.break_undo_group();
    }
    assert_eq!(e.undo_depth(), 3);
    undo_all(&mut e);
    assert_eq!(texts(&e), ["w0 w1 w2 w3 w4 w5 w6 "]);
}

#[test]
fn inherits_attributes_but_not_links_or_comments_at_edges() {
    let link = InlineAttrs {
        link: Some("https://a.b".into()),
        ..Default::default()
    };
    let mut e = ed(vec![para("x").with_span("link", link).with_span("y", InlineAttrs::default())]);
    // After the link: no link.
    e.set_caret(p(0, 5));
    e.insert_text("Z");
    assert_eq!(e.document().blocks()[0].attrs_at(5).unwrap().link, None);
    // Strictly inside: link continues.
    e.set_caret(p(0, 3));
    e.insert_text("Q");
    assert_eq!(e.document().blocks()[0].attrs_at(3).unwrap().link.as_deref(), Some("https://a.b"));
    // Bold is inherited from the preceding char.
    let mut e = ed(vec![para("").with_span("bold", bold()).with_span(" plain", InlineAttrs::default())]);
    e.set_caret(p(0, 4));
    e.insert_text("!");
    assert!(e.document().blocks()[0].attrs_at(4).unwrap().bold);
    // At block start: takes the first char's style.
    e.set_caret(p(0, 0));
    e.insert_text("<");
    assert!(e.document().blocks()[0].attrs_at(0).unwrap().bold);
}

#[test]
fn pending_attributes() {
    let mut e = EditorState::default();
    no_rules(&mut e);
    typ(&mut e, "a");
    assert!(!e.toggle_bold());
    assert!(e.current_attrs().bold);
    typ(&mut e, "bc");
    let b = &e.document().blocks()[0];
    assert!(!b.attrs_at(0).unwrap().bold);
    assert!(b.attrs_at(1).unwrap().bold && b.attrs_at(2).unwrap().bold);
    e.toggle_bold();
    e.move_caret(Movement::Left, false);
    assert!(e.pending_attrs().is_none(), "moving drops pending attributes");
    e.set_highlight(Some(Highlight::Green));
    e.set_text_color(Some(Rgb::new(9, 9, 9)));
    e.set_font_family(Some("Menlo"));
    typ(&mut e, "x");
    let a = e.document().blocks()[0].attrs_at(2).unwrap().clone();
    assert_eq!(a.highlight, Some(Highlight::Green));
    assert_eq!(a.color, Some(Rgb::new(9, 9, 9)));
    assert_eq!(a.font.as_deref(), Some("Menlo"));
    e.clear_formatting();
    typ(&mut e, "y");
    assert!(e.document().blocks()[0].attrs_at(3).unwrap().is_plain());
    // An edit that moves the caret drops pending attributes too.
    e.toggle_italic();
    e.backspace();
    assert!(e.pending_attrs().is_none());
    // A block command that keeps the caret keeps them.
    e.toggle_italic();
    e.set_block_kind(BlockKind::Quote);
    assert!(e.pending_attrs().is_some_and(|a| a.italic));
}

#[test]
fn multi_line_insert_splits_blocks() {
    let mut e = ed(vec![kb(BlockKind::BulletItem, "ab")]);
    e.set_caret(p(0, 1));
    e.insert_text("1\r\n2\n3");
    assert_eq!(texts(&e), ["a1", "2", "3b"]);
    assert_eq!(kinds(&e), vec![BlockKind::BulletItem; 3]);
    assert_eq!(caret(&e), p(2, 1));
    e.undo();
    assert_eq!(texts(&e), ["ab"]);
    // In a code block a newline stays inside the block.
    let mut e = ed(vec![kb(BlockKind::code(), "x")]);
    e.set_caret(p(0, 1));
    e.insert_text("\ny");
    assert_eq!(texts(&e), ["x\ny"]);
    e.paste_plain_text("\nz\n");
    assert_eq!(texts(&e), ["x\ny\nz\n"]);
}

#[test]
fn typing_on_void_blocks() {
    let mut e = ed(vec![Block::new(BlockKind::Divider)]);
    typ(&mut e, "a");
    assert_eq!(kinds(&e), vec![BlockKind::Divider, BlockKind::Paragraph]);
    assert_eq!(texts(&e), ["", "a"]);
    assert_eq!(caret(&e), p(1, 1));
    // Soft break on a void block behaves like Enter.
    let mut e = ed(vec![para("x"), Block::new(BlockKind::PageBreak)]);
    e.set_caret(p(1, 5));
    assert_eq!(caret(&e), p(1, 0));
    e.insert_soft_break();
    assert_eq!(kinds(&e)[2], BlockKind::Paragraph);
    assert_eq!(caret(&e), p(2, 0));
}

#[test]
fn soft_break() {
    let mut e = ed(vec![kb(BlockKind::heading(2), "ab")]);
    e.set_caret(p(0, 1));
    e.insert_soft_break();
    assert_eq!(texts(&e), ["a\nb"]);
    assert_eq!(caret(&e), p(0, 2));
    assert_eq!(e.document().block_count(), 1);
}

// ----- enter ----------------------------------------------------------------------------

#[test]
fn enter_splits_and_keeps_ids() {
    let mut e = ed(vec![para("hello world")]);
    let id = e.document().blocks()[0].id();
    e.set_caret(p(0, 5));
    e.insert_paragraph_break();
    check(&e);
    assert_eq!(texts(&e), ["hello", " world"]);
    assert_eq!(e.document().blocks()[0].id(), id);
    assert_ne!(e.document().blocks()[1].id(), id);
    assert_eq!(caret(&e), p(1, 0));
    e.undo();
    assert_eq!(texts(&e), ["hello world"]);
    assert_eq!(e.document().blocks()[0].id(), id);
    assert_eq!(caret(&e), p(0, 5));
}

#[test]
fn enter_heading_rules() {
    let h = BlockKind::heading(1);
    // End of heading: paragraph.
    let mut e = ed(vec![kb(h.clone(), "Title")]);
    e.set_caret(p(0, 5));
    e.insert_paragraph_break();
    assert_eq!(kinds(&e), vec![h.clone(), BlockKind::Paragraph]);
    // Middle: splits the heading.
    let mut e = ed(vec![kb(h.clone(), "Title")]);
    e.set_caret(p(0, 2));
    e.insert_paragraph_break();
    assert_eq!(kinds(&e), vec![h.clone(), h.clone()]);
    assert_eq!(texts(&e), ["Ti", "tle"]);
    // Start: empty paragraph before, heading keeps its id.
    let mut e = ed(vec![kb(h.clone(), "Title")]);
    let id = e.document().blocks()[0].id();
    e.insert_paragraph_break();
    assert_eq!(kinds(&e), vec![BlockKind::Paragraph, h.clone()]);
    assert_eq!(e.document().blocks()[1].id(), id);
    assert_eq!(caret(&e), p(1, 0));
}

#[test]
fn enter_list_rules() {
    let mut e = ed(vec![kb(BlockKind::TodoItem { checked: true }, "task").with_indent(2)]);
    e.set_caret(p(0, 4));
    e.insert_paragraph_break();
    assert_eq!(kinds(&e)[1], BlockKind::TodoItem { checked: false });
    assert_eq!(e.document().blocks()[1].indent(), 2);
    // Empty item: outdent twice, then paragraph.
    e.insert_paragraph_break();
    assert_eq!(e.document().blocks()[1].indent(), 1);
    e.insert_paragraph_break();
    assert_eq!(e.document().blocks()[1].indent(), 0);
    assert_eq!(kinds(&e)[1], BlockKind::todo());
    e.insert_paragraph_break();
    assert_eq!(kinds(&e)[1], BlockKind::Paragraph);
    assert_eq!(e.document().block_count(), 2);
    // Numbered and bullet continue.
    let mut e = ed(vec![kb(BlockKind::NumberedItem, "one")]);
    e.set_caret(p(0, 3));
    e.insert_paragraph_break();
    typ(&mut e, "two");
    assert_eq!(kinds(&e), vec![BlockKind::NumberedItem; 2]);
    assert_eq!(e.document().numbered_ordinal(1), Some(2));
    // Quote: continues, empty quote leaves.
    let mut e = ed(vec![kb(BlockKind::Quote, "q")]);
    e.set_caret(p(0, 1));
    e.insert_paragraph_break();
    assert_eq!(kinds(&e)[1], BlockKind::Quote);
    e.insert_paragraph_break();
    assert_eq!(kinds(&e)[1], BlockKind::Paragraph);
}

#[test]
fn enter_in_code_blocks() {
    let mut e = ed(vec![kb(BlockKind::code(), "fn x() {\n    let a;")]);
    no_rules(&mut e);
    e.set_caret(e.document().end());
    e.insert_paragraph_break();
    assert_eq!(texts(&e), ["fn x() {\n    let a;\n    "]);
    // Clean exit: two empty lines then Enter.
    let mut e = ed(vec![kb(BlockKind::code(), "x")]);
    e.set_caret(p(0, 1));
    e.insert_paragraph_break();
    e.insert_paragraph_break();
    assert_eq!(texts(&e), ["x\n\n"]);
    e.insert_paragraph_break();
    assert_eq!(texts(&e), ["x", ""]);
    assert_eq!(kinds(&e), vec![BlockKind::code(), BlockKind::Paragraph]);
    assert_eq!(caret(&e), p(1, 0));
    // exit_block from the middle.
    let mut e = ed(vec![kb(BlockKind::code(), "a\nb"), para("after")]);
    e.set_caret(p(0, 1));
    e.exit_block();
    assert_eq!(texts(&e), ["a\nb", "", "after"]);
    assert_eq!(caret(&e), p(1, 0));
}

#[test]
fn enter_carries_attributes_and_handles_voids() {
    let mut e = ed(vec![para("").with_span("bold", bold())]);
    e.set_caret(p(0, 4));
    e.insert_paragraph_break();
    assert!(e.pending_attrs().unwrap().bold);
    typ(&mut e, "x");
    assert!(e.document().blocks()[1].attrs_at(0).unwrap().bold);
    let mut e = ed(vec![Block::new(BlockKind::Divider)]);
    e.insert_paragraph_break();
    assert_eq!(kinds(&e), vec![BlockKind::Divider, BlockKind::Paragraph]);
    assert_eq!(caret(&e), p(1, 0));
}

// ----- backspace and delete -------------------------------------------------------------

#[test]
fn backspace_graphemes() {
    let t = format!("a{FAMILY}{E_ACUTE}中ש");
    let mut e = ed(vec![para(&t)]);
    e.set_caret(e.document().end());
    let mut seen = Vec::new();
    while caret(&e).offset > 0 {
        e.backspace();
        check(&e);
        seen.push(texts(&e)[0].clone());
    }
    assert_eq!(seen.len(), 5);
    assert_eq!(seen[1], format!("a{FAMILY}{E_ACUTE}"));
    assert_eq!(seen[2], format!("a{FAMILY}"));
    assert_eq!(seen[3], "a");
    // delete forward
    let mut e = ed(vec![para(&t)]);
    e.set_caret(p(0, 1));
    e.delete_forward();
    assert_eq!(texts(&e)[0], format!("a{E_ACUTE}中ש"));
}

#[test]
fn backspace_at_block_start_rules() {
    // Indented list item outdents, then becomes a paragraph.
    let mut e = ed(vec![para("p"), kb(BlockKind::BulletItem, "item").with_indent(1)]);
    e.set_caret(p(1, 0));
    e.backspace();
    assert_eq!(e.document().blocks()[1].indent(), 0);
    e.backspace();
    assert_eq!(kinds(&e)[1], BlockKind::Paragraph);
    e.backspace();
    assert_eq!(texts(&e), ["pitem"]);
    assert_eq!(caret(&e), p(0, 1));
    // Heading and quote become paragraphs.
    for k in [BlockKind::heading(3), BlockKind::Quote, BlockKind::NumberedItem] {
        let mut e = ed(vec![kb(k, "x")]);
        e.backspace();
        assert_eq!(kinds(&e), vec![BlockKind::Paragraph]);
    }
    // Code block becomes one paragraph per line.
    let mut e = ed(vec![kb(BlockKind::code(), "a\nb\n")]);
    e.backspace();
    assert_eq!(texts(&e), ["a", "b", ""]);
    assert_eq!(kinds(&e), vec![BlockKind::Paragraph; 3]);
    e.undo();
    assert_eq!(texts(&e), ["a\nb\n"]);
    // A void block before the caret is deleted.
    let mut e = ed(vec![para("a"), Block::new(BlockKind::Divider), para("b")]);
    e.set_caret(p(2, 0));
    e.backspace();
    assert_eq!(texts(&e), ["a", "b"]);
    assert_eq!(caret(&e), p(1, 0));
    // First block: nothing.
    let mut e = ed(vec![para("a")]);
    assert!(!e.backspace());
    // Indented paragraph outdents.
    let mut e = ed(vec![para("a"), para("b").with_indent(2)]);
    e.set_caret(p(1, 0));
    e.backspace();
    assert_eq!(e.document().blocks()[1].indent(), 1);
}

#[test]
fn caret_on_void_block() {
    let mut e = ed(vec![para("a"), Block::new(BlockKind::Divider), para("b")]);
    e.set_caret(p(1, 0));
    e.backspace();
    assert_eq!(texts(&e), ["a", "b"]);
    assert_eq!(caret(&e), p(0, 1));
    e.undo();
    e.set_caret(p(1, 0));
    e.delete_forward();
    assert_eq!(texts(&e), ["a", "b"]);
    assert_eq!(caret(&e), p(1, 0));
    // Only block: replaced by an empty paragraph.
    let mut e = ed(vec![Block::new(BlockKind::PageBreak)]);
    e.backspace();
    assert_eq!(kinds(&e), vec![BlockKind::Paragraph]);
    check(&e);
    // Void at the end, delete forward goes to the end of the previous block.
    let mut e = ed(vec![para("ab"), Block::new(BlockKind::Divider)]);
    e.set_caret(p(1, 0));
    e.delete_forward();
    assert_eq!(caret(&e), p(0, 2));
}

#[test]
fn delete_forward_merges() {
    let mut e = ed(vec![kb(BlockKind::heading(1), "ab"), para("cd"), Block::new(BlockKind::Divider), para("e")]);
    e.set_caret(p(0, 2));
    e.delete_forward();
    assert_eq!(texts(&e), ["abcd", "", "e"]);
    assert_eq!(kinds(&e)[0], BlockKind::heading(1));
    assert_eq!(caret(&e), p(0, 2));
    e.set_caret(p(0, 4));
    e.delete_forward();
    assert_eq!(texts(&e), ["abcd", "e"]);
    e.set_caret(e.document().end());
    assert!(!e.delete_forward());
}

#[test]
fn word_and_line_deletion() {
    let mut e = ed(vec![para("one two, three"), para("next")]);
    no_rules(&mut e);
    e.set_caret(p(0, 14));
    e.delete_word_backward();
    assert_eq!(texts(&e)[0], "one two, ");
    e.delete_word_backward();
    assert_eq!(texts(&e)[0], "one ");
    e.set_caret(p(0, 0));
    e.delete_word_forward();
    assert_eq!(texts(&e)[0], " ");
    e.set_caret(p(1, 0));
    e.delete_word_backward();
    assert_eq!(texts(&e), [" next"]);
    let mut e = ed(vec![para("ab\ncd ef")]);
    e.set_caret(p(0, 5));
    e.delete_to_line_start();
    assert_eq!(texts(&e), ["ab\n ef"]);
    e.delete_to_line_start();
    assert_eq!(texts(&e), ["ab ef"]);
    e.set_caret(p(0, 1));
    e.delete_to_line_end();
    assert_eq!(texts(&e), ["a"]);
}

#[test]
fn delete_selection_across_blocks() {
    let blocks = || {
        vec![
            kb(BlockKind::heading(1), "Head"),
            para("middle"),
            kb(BlockKind::Quote, "quote"),
        ]
    };
    let mut e = ed(blocks());
    let id0 = e.document().blocks()[0].id();
    e.set_selection(Selection::new(p(2, 2), p(0, 2)));
    e.delete_selection();
    assert_eq!(texts(&e), ["Heote"]);
    assert_eq!(kinds(&e), vec![BlockKind::heading(1)]);
    assert_eq!(e.document().blocks()[0].id(), id0);
    assert_eq!(caret(&e), p(0, 2));
    e.undo();
    assert_eq!(texts(&e), ["Head", "middle", "quote"]);
    assert_eq!(e.selection(), Selection::new(p(2, 2), p(0, 2)));
    // Ending at offset 0 of a block merges with it.
    let mut e = ed(blocks());
    e.set_selection(Selection::new(p(0, 0), p(2, 0)));
    e.delete_selection();
    assert_eq!(texts(&e), ["quote"]);
    assert_eq!(kinds(&e), vec![BlockKind::heading(1)]);
    // Void blocks at the start are deleted, at the end kept.
    let mut e = ed(vec![Block::new(BlockKind::Divider), para("ab"), Block::new(BlockKind::PageBreak)]);
    e.set_selection(Selection::new(p(0, 0), p(1, 1)));
    e.delete_selection();
    assert_eq!(texts(&e), ["b", ""]);
    assert_eq!(kinds(&e)[0], BlockKind::Paragraph);
    let mut e = ed(vec![para("ab"), para("cd"), Block::new(BlockKind::PageBreak)]);
    e.set_selection(Selection::new(p(0, 1), p(2, 0)));
    e.delete_selection();
    assert_eq!(texts(&e), ["a", ""]);
    assert_eq!(kinds(&e)[1], BlockKind::PageBreak);
    let mut e = ed(vec![para("ab"), Block::new(BlockKind::PageBreak)]);
    e.set_selection(Selection::new(p(0, 0), p(1, 0)));
    e.delete_selection();
    assert_eq!(kinds(&e), vec![BlockKind::PageBreak]);
    check(&e);
    // Select all and delete leaves one empty block.
    let mut e = ed(blocks());
    e.select_all();
    e.backspace();
    assert_eq!(texts(&e), [""]);
    check(&e);
}

// ----- movement and selection -------------------------------------------------------------

#[test]
fn movement_and_extension() {
    let mut e = ed(vec![para("one two"), para("three")]);
    e.move_caret(Movement::WordRight, true);
    assert_eq!(e.selection(), Selection::new(p(0, 0), p(0, 3)));
    e.move_caret(Movement::Right, false);
    assert_eq!(caret(&e), p(0, 3));
    e.move_caret(Movement::DocEnd, true);
    assert_eq!(e.selection().head, p(1, 5));
    e.move_caret(Movement::Left, false);
    assert_eq!(caret(&e), p(0, 3));
    e.select_word_at(p(0, 5));
    assert_eq!(e.selection(), Selection::new(p(0, 4), p(0, 7)));
    e.select_block(1);
    assert_eq!(e.selection(), Selection::new(p(1, 0), p(1, 5)));
    e.select_all();
    assert_eq!(e.selection(), Selection::new(p(0, 0), p(1, 5)));
    // Clamping.
    e.set_selection(Selection::new(p(7, 7), p(0, 99)));
    assert_eq!(e.selection(), Selection::new(p(1, 5), p(0, 7)));
    let mut e = ed(vec![para(E_ACUTE)]);
    e.set_caret(p(0, 1));
    assert_eq!(caret(&e), p(0, 0), "carets snap to grapheme boundaries");
}

// ----- inline formatting -------------------------------------------------------------------

#[test]
fn toggle_styles_over_selection() {
    let mut e = ed(vec![para("ab").with_span("cd", bold()), kb(BlockKind::code(), "code"), para("ef")]);
    e.set_selection(Selection::new(p(0, 0), p(0, 4)));
    assert_eq!(e.format_state().bold, Toggle::Mixed);
    assert!(e.toggle_bold());
    assert_eq!(e.format_state().bold, Toggle::On);
    assert!(e.toggle_bold());
    assert_eq!(e.format_state().bold, Toggle::Off);
    assert_eq!(e.document().blocks()[0].runs().len(), 1);
    // Across blocks, code blocks are skipped.
    e.set_selection(Selection::new(p(0, 1), p(2, 1)));
    e.toggle_style(InlineStyle::Italic);
    assert!(e.document().blocks()[2].attrs_at(0).unwrap().italic);
    assert!(!e.document().blocks()[2].attrs_at(1).unwrap().italic);
    assert!(e.document().blocks()[1].attrs_at(0).unwrap().is_plain());
    check(&e);
    for s in InlineStyle::ALL {
        e.select_all();
        e.toggle_style(s);
        assert_eq!(e.format_state().style(s), Toggle::On, "{s:?}");
    }
    let st = e.format_state();
    assert_eq!(st.kind, Choice::Mixed);
    e.clear_formatting();
    assert_eq!(e.format_state().bold, Toggle::Off);
    check(&e);
}

#[test]
fn clear_formatting_keeps_links_and_comments() {
    let a = InlineAttrs {
        bold: true,
        link: Some("https://x".into()),
        color: Some(Rgb::new(1, 1, 1)),
        ..Default::default()
    };
    let mut e = ed(vec![para("").with_span("text", a)]);
    e.select_all();
    let id = e.add_comment("c", "me", 1).unwrap();
    e.clear_formatting();
    let attrs = e.document().blocks()[0].attrs_at(0).unwrap();
    assert!(!attrs.bold);
    assert_eq!(attrs.color, None);
    assert_eq!(attrs.link.as_deref(), Some("https://x"));
    assert!(attrs.comments.contains(&id));
}

#[test]
fn highlight_color_font() {
    let mut e = ed(vec![para("hello")]);
    e.set_selection(Selection::new(p(0, 1), p(0, 3)));
    e.toggle_highlight(Highlight::Yellow);
    assert_eq!(e.format_state().highlight, Choice::Same(Some(Highlight::Yellow)));
    e.toggle_highlight(Highlight::Yellow);
    assert_eq!(e.format_state().highlight, Choice::Same(None));
    e.set_text_color(Some(Rgb::new(255, 0, 0)));
    e.set_font_family(Some("  Menlo "));
    let st = e.format_state();
    assert_eq!(st.color, Choice::Same(Some(Rgb::new(255, 0, 0))));
    assert_eq!(st.font, Choice::Same(Some("Menlo".to_string())));
    e.set_selection(Selection::new(p(0, 0), p(0, 3)));
    assert_eq!(e.format_state().font, Choice::Mixed);
    e.set_font_family(Some(""));
    assert_eq!(e.format_state().font, Choice::Same(None));
}

#[test]
fn links() {
    let mut e = ed(vec![para("see the docs here")]);
    e.set_selection(Selection::new(p(0, 8), p(0, 12)));
    assert!(e.set_link(" https://docs "));
    let info = e.document().link_at(p(0, 10)).unwrap();
    assert_eq!(info.url, "https://docs");
    assert_eq!(info.range, TextRange::in_block(0, 8, 12));
    // Bold inside the link does not split the link range.
    e.set_selection(Selection::new(p(0, 9), p(0, 10)));
    e.toggle_bold();
    assert_eq!(e.document().link_at(p(0, 12)).unwrap().range, TextRange::in_block(0, 8, 12));
    assert_eq!(e.document().link_at(p(0, 8)).unwrap().range, TextRange::in_block(0, 8, 12));
    assert!(e.document().link_at(p(0, 7)).is_none());
    // Collapsed caret inside: edit the whole link.
    e.set_caret(p(0, 10));
    assert!(e.set_link("https://new"));
    assert_eq!(e.document().link_at(p(0, 9)).unwrap().url, "https://new");
    e.set_caret(p(0, 2));
    assert!(!e.set_link("https://nowhere"));
    e.set_caret(p(0, 11));
    assert!(e.remove_link());
    assert!(e.document().link_at(p(0, 10)).is_none());
    // insert_link
    let mut e = ed(vec![para("a  b")]);
    e.set_caret(p(0, 2));
    assert!(e.insert_link("site", "https://s"));
    assert_eq!(texts(&e), ["a site b"]);
    assert_eq!(e.document().link_at(p(0, 4)).unwrap().range, TextRange::in_block(0, 2, 6));
    typ(&mut e, "x");
    assert_eq!(e.document().blocks()[0].attrs_at(6).unwrap().link, None);
}

#[test]
fn format_state_of_blocks() {
    let mut e = ed(vec![
        kb(BlockKind::heading(2), "a").with_alignment(Alignment::Center),
        kb(BlockKind::heading(2), "b").with_alignment(Alignment::Center),
        para("c"),
    ]);
    e.set_selection(Selection::new(p(0, 0), p(1, 1)));
    let st = e.format_state();
    assert_eq!(st.kind, Choice::Same(BlockKind::heading(2)));
    assert_eq!(st.alignment, Choice::Same(Alignment::Center));
    // Ending at offset 0 of block 2 does not touch it.
    e.set_selection(Selection::new(p(0, 0), p(2, 0)));
    assert_eq!(e.format_state().kind, Choice::Same(BlockKind::heading(2)));
    e.set_selection(Selection::new(p(0, 0), p(2, 1)));
    assert_eq!(e.format_state().kind, Choice::Mixed);
}

// ----- block commands -------------------------------------------------------------------

#[test]
fn set_block_kind_toggles() {
    let mut e = ed(vec![para("a"), para("b"), Block::new(BlockKind::Divider)]);
    e.set_selection(Selection::new(p(0, 0), p(2, 0)));
    assert!(e.set_block_kind(BlockKind::BulletItem));
    assert_eq!(kinds(&e)[..2], [BlockKind::BulletItem, BlockKind::BulletItem]);
    assert_eq!(kinds(&e)[2], BlockKind::Divider);
    assert!(e.set_block_kind(BlockKind::BulletItem));
    assert_eq!(kinds(&e)[..2], [BlockKind::Paragraph, BlockKind::Paragraph]);
    assert!(e.set_block_kind(BlockKind::heading(1)));
    assert!(e.set_block_kind(BlockKind::heading(2)));
    assert_eq!(kinds(&e)[0], BlockKind::heading(2));
    assert!(!e.set_block_kind(BlockKind::Divider));
    e.set_caret(p(2, 0));
    assert!(!e.set_block_kind(BlockKind::Quote));
}

#[test]
fn code_block_conversion_merges_and_splits() {
    let mut e = ed(vec![para("let a;"), para("").with_span("let b;", bold()), para("tail")]);
    e.set_selection(Selection::new(p(0, 4), p(1, 3)));
    e.set_block_kind(BlockKind::code());
    assert_eq!(texts(&e), ["let a;\nlet b;", "tail"]);
    assert_eq!(e.selection(), Selection::new(p(0, 4), p(0, 10)));
    assert!(e.document().blocks()[0].attrs_at(8).unwrap().is_plain());
    check(&e);
    e.set_block_kind(BlockKind::code());
    assert_eq!(texts(&e), ["let a;", "let b;", "tail"]);
    assert_eq!(e.selection(), Selection::new(p(0, 4), p(1, 3)));
    e.undo();
    e.undo();
    assert_eq!(texts(&e), ["let a;", "let b;", "tail"]);
    assert!(e.document().blocks()[1].attrs_at(0).unwrap().bold);
    e.set_caret(p(0, 0));
    e.set_block_kind(BlockKind::code());
    assert!(e.set_code_language(Some("rust")));
    assert_eq!(
        kinds(&e)[0],
        BlockKind::CodeBlock {
            language: Some("rust".into())
        }
    );
}

#[test]
fn todo_alignment_size_indent() {
    let mut e = ed(vec![kb(BlockKind::todo(), "a"), kb(BlockKind::TodoItem { checked: true }, "b"), para("c")]);
    e.select_all();
    assert!(e.toggle_todo_checked());
    assert_eq!(kinds(&e)[..2], [BlockKind::TodoItem { checked: true }, BlockKind::TodoItem { checked: true }]);
    e.toggle_todo_checked();
    assert_eq!(kinds(&e)[1], BlockKind::todo());
    e.set_caret(p(2, 0));
    assert!(!e.toggle_todo_checked());
    e.select_all();
    e.set_alignment(Alignment::Right);
    assert_eq!(e.format_state().alignment, Choice::Same(Alignment::Right));
    // Size steps from the kind default (17) along the scale.
    e.set_caret(p(2, 0));
    e.step_text_size(1);
    assert_eq!(e.document().blocks()[2].text_size(), Some(18.0));
    e.step_text_size(-1);
    assert_eq!(e.document().blocks()[2].text_size(), None);
    e.step_text_size(-2);
    assert_eq!(e.document().blocks()[2].text_size(), Some(15.0));
    e.set_text_size(Some(1000.0));
    assert_eq!(e.document().blocks()[2].text_size(), Some(crate::MAX_TEXT_SIZE));
    e.reset_text_size();
    assert_eq!(e.document().blocks()[2].text_size(), None);
    for _ in 0..12 {
        e.indent();
    }
    assert_eq!(e.document().blocks()[2].indent(), MAX_INDENT_TEST);
    e.outdent();
    assert_eq!(e.document().blocks()[2].indent(), MAX_INDENT_TEST - 1);
    for _ in 0..12 {
        e.outdent();
    }
    assert!(!e.outdent());
}

const MAX_INDENT_TEST: u8 = crate::MAX_INDENT;

#[test]
fn move_duplicate_delete_blocks() {
    let mut e = ed(vec![para("a"), para("b"), para("c"), para("d")]);
    let ids: Vec<_> = e.document().blocks().iter().map(|b| b.id()).collect();
    e.set_selection(Selection::new(p(1, 0), p(2, 1)));
    assert!(e.move_blocks_up());
    assert_eq!(texts(&e), ["b", "c", "a", "d"]);
    assert_eq!(e.selection(), Selection::new(p(0, 0), p(1, 1)));
    assert!(!e.move_blocks_up());
    e.move_blocks_down();
    e.move_blocks_down();
    assert_eq!(texts(&e), ["a", "d", "b", "c"]);
    assert!(!e.move_blocks_down());
    let ids_after: Vec<_> = e.document().blocks().iter().map(|b| b.id()).collect();
    assert_eq!(ids_after, vec![ids[0], ids[3], ids[1], ids[2]]);
    // Selection ending at offset 0 of the next block.
    let mut e = ed(vec![para("a"), para("b"), para("c")]);
    e.set_selection(Selection::new(p(0, 0), p(1, 0)));
    e.move_blocks_down();
    assert_eq!(texts(&e), ["b", "a", "c"]);
    assert_eq!(e.touched_blocks(), 1..2);
    e.move_blocks_up();
    assert_eq!(texts(&e), ["a", "b", "c"]);
    assert_eq!(e.touched_blocks(), 0..1);
    // Duplicate.
    e.set_caret(p(1, 1));
    e.duplicate_blocks();
    assert_eq!(texts(&e), ["a", "b", "b", "c"]);
    assert_eq!(caret(&e), p(2, 1));
    assert_ne!(e.document().blocks()[1].id(), e.document().blocks()[2].id());
    // Delete.
    e.delete_blocks();
    assert_eq!(texts(&e), ["a", "b", "c"]);
    assert_eq!(caret(&e), p(2, 0));
    e.set_caret(p(2, 0));
    e.delete_blocks();
    assert_eq!(caret(&e), p(1, 1));
    e.select_all();
    e.delete_blocks();
    assert_eq!(texts(&e), [""]);
    check(&e);
    undo_all(&mut e);
    assert_eq!(texts(&e), ["a", "b", "c"]);
}

#[test]
fn insert_void_and_empty_blocks() {
    let mut e = ed(vec![para("text"), para("")]);
    e.set_caret(p(0, 2));
    e.insert_divider();
    assert_eq!(kinds(&e), vec![BlockKind::Paragraph, BlockKind::Divider, BlockKind::Paragraph]);
    assert_eq!(caret(&e), p(2, 0), "reuses the empty paragraph");
    e.insert_page_break();
    assert_eq!(kinds(&e)[2], BlockKind::PageBreak, "empty paragraph becomes the break");
    assert_eq!(kinds(&e)[3], BlockKind::Paragraph);
    assert_eq!(caret(&e), p(3, 0));
    let mut e = ed(vec![kb(BlockKind::BulletItem, "x").with_indent(1)]);
    e.insert_block_after();
    assert_eq!(kinds(&e)[1], BlockKind::BulletItem);
    assert_eq!(e.document().blocks()[1].indent(), 1);
    assert_eq!(caret(&e), p(1, 0));
    e.insert_block_before();
    assert_eq!(e.document().block_count(), 3);
    assert_eq!(caret(&e), p(1, 0));
    let mut e = ed(vec![kb(BlockKind::heading(1), "h")]);
    e.insert_block_before();
    assert_eq!(kinds(&e), vec![BlockKind::Paragraph, BlockKind::heading(1)]);
}

// ----- input rules -------------------------------------------------------------------------

#[test]
fn block_input_rules() {
    let cases: &[(&str, BlockKind)] = &[
        ("# ", BlockKind::heading(1)),
        ("### ", BlockKind::heading(3)),
        ("###### ", BlockKind::heading(6)),
        ("- ", BlockKind::BulletItem),
        ("* ", BlockKind::BulletItem),
        ("+ ", BlockKind::BulletItem),
        ("1. ", BlockKind::NumberedItem),
        ("42) ", BlockKind::NumberedItem),
        ("[] ", BlockKind::todo()),
        ("[ ] ", BlockKind::todo()),
        ("[x] ", BlockKind::TodoItem { checked: true }),
        ("> ", BlockKind::Quote),
        ("```", BlockKind::code()),
    ];
    for (input, kind) in cases {
        let mut e = EditorState::default();
        typ(&mut e, input);
        assert_eq!(kinds(&e), vec![kind.clone()], "input {input:?}");
        assert_eq!(texts(&e), [""], "input {input:?}");
        e.undo();
        assert_eq!(texts(&e), [*input], "one undo restores the typed text for {input:?}");
        assert_eq!(kinds(&e), vec![BlockKind::Paragraph]);
    }
    for input in ["####### ", "a# ", "1.5 ", "[y] ", "-- "] {
        let mut e = EditorState::default();
        e.set_options(EditorOptions {
            input_rules: true,
            smart_punctuation: false,
        });
        typ(&mut e, input);
        assert_eq!(kinds(&e), vec![BlockKind::Paragraph], "input {input:?}");
    }
    // Existing text after the caret is kept.
    let mut e = ed(vec![para("Title")]);
    typ(&mut e, "## ");
    assert_eq!(texts(&e), ["Title"]);
    assert_eq!(kinds(&e), vec![BlockKind::heading(2)]);
    // Divider.
    let mut e = EditorState::default();
    typ(&mut e, "---");
    assert_eq!(kinds(&e), vec![BlockKind::Divider, BlockKind::Paragraph]);
    assert_eq!(caret(&e), p(1, 0));
    e.undo();
    assert_eq!(texts(&e), ["---"]);
    // Not in headings or with rules off.
    let mut e = ed(vec![kb(BlockKind::heading(1), "")]);
    typ(&mut e, "- ");
    assert_eq!(kinds(&e), vec![BlockKind::heading(1)]);
    let mut e = EditorState::default();
    no_rules(&mut e);
    typ(&mut e, "# ");
    assert_eq!(kinds(&e), vec![BlockKind::Paragraph]);
}

fn styled(e: &EditorState, block: usize, f: impl Fn(&InlineAttrs) -> bool) -> String {
    let b = &e.document().blocks()[block];
    b.text_spans().filter(|(_, _, a)| f(a)).map(|(_, t, _)| t).collect()
}

#[test]
fn inline_input_rules() {
    let mut e = EditorState::default();
    typ(&mut e, "a **bold** b");
    assert_eq!(texts(&e), ["a bold b"]);
    assert_eq!(styled(&e, 0, |a| a.bold), "bold");
    let mut e = EditorState::default();
    typ(&mut e, "*it* _em_ `co` ~~st~~ ==hi==");
    assert_eq!(texts(&e), ["it em co st hi"]);
    assert_eq!(styled(&e, 0, |a| a.italic), "item");
    assert_eq!(styled(&e, 0, |a| a.code), "co");
    assert_eq!(styled(&e, 0, |a| a.strikethrough), "st");
    assert_eq!(styled(&e, 0, |a| a.highlight.is_some()), "hi");
    // Undo gives back the literal delimiters.
    let mut e = EditorState::default();
    typ(&mut e, "**b**");
    e.undo();
    assert_eq!(texts(&e), ["**b**"]);
    assert_eq!(styled(&e, 0, |a| a.bold), "");
    // Typing after the closing delimiter is not styled.
    let mut e = EditorState::default();
    typ(&mut e, "**b** x");
    assert_eq!(styled(&e, 0, |a| a.bold), "b");
}

#[test]
fn inline_rules_leave_prose_alone() {
    for input in [
        "2 * 3 * 4",
        "snake_case_name",
        "__init__",
        "a footnote* here",
        "x == y == z",
        "a ** b **",
        "price_1 and_2",
        "2*3*4",
        "``",
        "* not a list item *",
    ] {
        let mut e = ed(vec![para("p")]);
        e.set_caret(p(0, 1));
        e.set_options(EditorOptions {
            input_rules: true,
            smart_punctuation: false,
        });
        typ(&mut e, input);
        assert_eq!(texts(&e), [format!("p{input}")], "input {input:?}");
        assert!(e.document().blocks()[0].runs().len() <= 1, "no styling for {input:?}");
    }
    // Never inside code blocks or inline code.
    let mut e = ed(vec![kb(BlockKind::code(), "")]);
    typ(&mut e, "**x** \"q\" -- ...");
    assert_eq!(texts(&e), ["**x** \"q\" -- ..."]);
    let code = InlineAttrs {
        code: true,
        ..Default::default()
    };
    let mut e = ed(vec![para("").with_span("ab", code)]);
    e.set_caret(p(0, 1));
    typ(&mut e, "*x* \"");
    assert_eq!(texts(&e), ["a*x* \"b"]);
}

#[test]
fn smart_punctuation() {
    let mut e = EditorState::default();
    typ(&mut e, "\"Hi,\" she said -- it's 'fine'...");
    assert_eq!(texts(&e), ["\u{201C}Hi,\u{201D} she said \u{2014} it\u{2019}s \u{2018}fine\u{2019}\u{2026}"]);
    // Undo restores the literal character.
    let mut e = EditorState::default();
    typ(&mut e, "\"");
    assert_eq!(texts(&e), ["\u{201C}"]);
    e.undo();
    assert_eq!(texts(&e), ["\""]);
    // Deferred em dash at block start.
    let mut e = EditorState::default();
    typ(&mut e, "--");
    assert_eq!(texts(&e), ["--"]);
    typ(&mut e, " Hola");
    assert_eq!(texts(&e), ["\u{2014} Hola"]);
    // Switchable.
    let mut e = EditorState::default();
    e.set_options(EditorOptions {
        input_rules: true,
        smart_punctuation: false,
    });
    typ(&mut e, "\"a\" --");
    assert_eq!(texts(&e), ["\"a\" --"]);
}

// ----- clipboard ----------------------------------------------------------------------------

#[test]
fn copy_cut_paste_inline() {
    let mut e = ed(vec![kb(BlockKind::heading(1), "Hello"), para("world")]);
    e.set_selection(Selection::new(p(0, 1), p(0, 4)));
    let f = e.copy();
    assert!(f.is_inline());
    e.set_caret(p(1, 5));
    assert!(e.paste(&f));
    assert_eq!(texts(&e), ["Hello", "worldell"]);
    assert_eq!(kinds(&e)[1], BlockKind::Paragraph);
    assert_eq!(caret(&e), p(1, 8));
    let cut = e.cut();
    assert!(cut.is_empty());
    e.set_selection(Selection::new(p(1, 0), p(1, 5)));
    let cut = e.cut();
    assert_eq!(cut.to_plain_text(), "world");
    assert_eq!(texts(&e), ["Hello", "ell"]);
    e.undo();
    assert_eq!(texts(&e), ["Hello", "worldell"]);
}

#[test]
fn paste_multi_block() {
    let src = ed(vec![para("alpha"), kb(BlockKind::BulletItem, "beta"), kb(BlockKind::heading(2), "gamma")]);
    let mut src = src;
    src.set_selection(Selection::new(p(0, 2), p(2, 3)));
    let f = src.copy();
    assert_eq!(f.blocks().len(), 3);
    // Into the middle of a paragraph.
    let mut e = ed(vec![para("XY")]);
    let id = e.document().blocks()[0].id();
    e.set_caret(p(0, 1));
    e.paste(&f);
    check(&e);
    assert_eq!(texts(&e), ["Xpha", "beta", "gamY"]);
    assert_eq!(kinds(&e), vec![BlockKind::Paragraph, BlockKind::BulletItem, BlockKind::heading(2)]);
    assert_eq!(e.document().blocks()[0].id(), id);
    assert_eq!(caret(&e), p(2, 3));
    e.undo();
    assert_eq!(texts(&e), ["XY"]);
    // Into an empty paragraph: the first block's kind is taken.
    let f = Fragment::new(vec![kb(BlockKind::Quote, "q"), para("")]);
    let mut e = ed(vec![para(""), para("z")]);
    e.paste(&f);
    assert_eq!(texts(&e), ["q", "", "z"]);
    assert_eq!(kinds(&e)[0], BlockKind::Quote);
    assert_eq!(caret(&e), p(1, 0));
    // Into a code block: plain text.
    let mut e = ed(vec![kb(BlockKind::code(), "c")]);
    e.set_caret(p(0, 1));
    let f = Fragment::new(vec![kb(BlockKind::heading(1), "a"), para("").with_span("b", bold())]);
    e.paste(&f);
    assert_eq!(texts(&e), ["ca\nb"]);
    check(&e);
    // Void blocks in fragments never merge.
    let f = Fragment::new(vec![para("x"), Block::new(BlockKind::Divider)]);
    let mut e = ed(vec![para("ab")]);
    e.set_caret(p(0, 1));
    e.paste(&f);
    assert_eq!(kinds(&e), vec![BlockKind::Paragraph, BlockKind::Divider, BlockKind::Paragraph]);
    assert_eq!(texts(&e), ["ax", "", "b"]);
    assert_eq!(caret(&e), p(2, 0));
    // A lone void fragment at the end of a block.
    let f = Fragment::new(vec![Block::new(BlockKind::PageBreak)]);
    let mut e = ed(vec![para("ab")]);
    e.set_caret(p(0, 2));
    e.paste(&f);
    assert_eq!(kinds(&e), vec![BlockKind::Paragraph, BlockKind::PageBreak, BlockKind::Paragraph]);
    check(&e);
}

#[test]
fn paste_plain_text_and_blocks_at() {
    let mut e = ed(vec![kb(BlockKind::NumberedItem, "ab")]);
    e.set_caret(p(0, 1));
    e.paste_plain_text("x\ny");
    assert_eq!(texts(&e), ["ax", "yb"]);
    assert_eq!(kinds(&e), vec![BlockKind::NumberedItem; 2]);
    assert_eq!(e.undo_depth(), 1);
    // Rules do not run on paste.
    let mut e = EditorState::default();
    e.paste_plain_text("# ");
    assert_eq!(kinds(&e), vec![BlockKind::Paragraph]);
    let f = Fragment::from_plain_text("l1\nl2");
    let mut e = ed(vec![para("a"), para("b")]);
    e.insert_blocks_at(1, &f);
    assert_eq!(texts(&e), ["a", "l1", "l2", "b"]);
    assert_eq!(caret(&e), p(1, 0));
    e.insert_blocks_at(99, &f);
    assert_eq!(texts(&e).len(), 6);
    check(&e);
}

#[test]
fn fragments_drop_comment_anchors() {
    let mut e = ed(vec![para("commented text")]);
    e.select_all();
    let id = e.add_comment("note", "me", 5).unwrap();
    let f = e.copy();
    assert!(f.blocks()[0].comment_ids().is_empty());
    e.set_caret(e.document().end());
    e.paste(&f);
    assert_eq!(e.document().comment_ranges(id), vec![TextRange::in_block(0, 0, 14)]);
    e.duplicate_blocks();
    assert_eq!(e.document().comment_ranges(id).len(), 1);
    check(&e);
}

// ----- comments ----------------------------------------------------------------------------

#[test]
fn comment_anchors_follow_edits() {
    let mut e = ed(vec![para("one two three")]);
    e.set_selection(Selection::new(p(0, 4), p(0, 7)));
    let id = e.add_comment("why two?", "ann", 100).unwrap();
    assert_eq!(e.document().comment_ranges(id), vec![TextRange::in_block(0, 4, 7)]);
    assert_eq!(e.document().comment(id).unwrap().author, "ann");
    // Typing before shifts the anchor; typing inside extends it.
    e.set_caret(p(0, 0));
    typ(&mut e, "X");
    assert_eq!(e.document().comment_ranges(id), vec![TextRange::in_block(0, 5, 8)]);
    e.set_caret(p(0, 6));
    typ(&mut e, "w");
    assert_eq!(e.document().comment_ranges(id), vec![TextRange::in_block(0, 5, 9)]);
    // Typing at the edge does not extend it.
    e.set_caret(p(0, 9));
    typ(&mut e, "!");
    assert_eq!(e.document().comment_ranges(id), vec![TextRange::in_block(0, 5, 9)]);
    // Split in the middle: one range across the block boundary.
    e.set_caret(p(0, 7));
    e.insert_paragraph_break();
    assert_eq!(
        e.document().comment_ranges(id),
        vec![TextRange::new(p(0, 5), p(1, 2))]
    );
    assert_eq!(e.document().comments_at(p(1, 1)), vec![id]);
    assert_eq!(e.document().comments_at(p(1, 2)), vec![id]);
    assert!(e.document().comments_at(p(1, 4)).is_empty());
    // Merge back.
    e.backspace();
    assert_eq!(e.document().comment_ranges(id), vec![TextRange::in_block(0, 5, 9)]);
    // Delete all anchored text: orphaned, undo restores.
    e.set_selection(Selection::new(p(0, 4), p(0, 10)));
    e.delete_selection();
    assert!(e.document().is_orphaned(id));
    assert!(e.document().comment(id).is_some());
    assert_eq!(e.document().threads_in_order(), vec![id]);
    e.undo();
    assert!(!e.document().is_orphaned(id));
    e.redo();
    assert!(e.document().is_orphaned(id));
    check(&e);
}

#[test]
fn comment_thread_operations() {
    let mut e = ed(vec![para("aaa bbb ccc")]);
    e.set_selection(Selection::new(p(0, 8), p(0, 11)));
    let c2 = e.add_comment("second", "x", 2).unwrap();
    e.set_selection(Selection::new(p(0, 0), p(0, 3)));
    let c1 = e.add_comment("first", "x", 1).unwrap();
    assert_eq!(e.document().threads_in_order(), vec![c1, c2]);
    assert!(e.edit_comment(c1, "first!"));
    assert!(!e.edit_comment(c1, "first!"));
    let r1 = e.add_reply(c1, "reply", "y", 3).unwrap();
    let r2 = e.add_reply(c1, "reply 2", "z", 4).unwrap();
    assert_ne!(r1, r2);
    assert!(e.edit_reply(c1, r1, "edited"));
    assert!(e.delete_reply(c1, r2));
    assert!(e.resolve_comment(c1));
    assert!(e.document().comment(c1).unwrap().resolved);
    assert!(e.reopen_comment(c1));
    let t = e.document().comment(c1).unwrap();
    assert_eq!(t.body, "first!");
    assert_eq!(t.replies.len(), 1);
    assert_eq!(t.replies[0].body, "edited");
    assert!(!t.resolved);
    // Navigation.
    let d = e.document();
    assert_eq!(d.next_comment(p(0, 0), false).map(|c| c.0), Some(c2));
    assert_eq!(d.next_comment(p(0, 8), false), None);
    assert_eq!(d.next_comment(p(0, 8), true).map(|c| c.0), Some(c1));
    assert_eq!(d.prev_comment(p(0, 8), false).map(|c| c.0), Some(c1));
    assert_eq!(d.prev_comment(p(0, 0), true).map(|c| c.0), Some(c2));
    // Delete removes anchors; undo restores thread and anchors.
    let before = e.document().clone();
    assert!(e.delete_comment(c1));
    assert!(e.document().comment(c1).is_none());
    assert!(e.document().blocks()[0].attrs_at(0).unwrap().comments.is_empty());
    check(&e);
    e.undo();
    assert_eq!(e.document(), &before);
    assert!(!e.delete_comment(CommentId(999)));
    // No text selected: no comment.
    e.set_caret(p(0, 1));
    assert!(e.add_comment("x", "y", 0).is_none());
    // Comments in code blocks are allowed.
    let mut e = ed(vec![kb(BlockKind::code(), "fn main")]);
    e.select_all();
    let c = e.add_comment("code", "me", 0).unwrap();
    assert_eq!(e.document().comment_ranges(c).len(), 1);
    check(&e);
}

// ----- change tracking ---------------------------------------------------------------------

#[test]
fn change_tracking_reports_local_ranges() {
    let mut e = ed(vec![para("a"), para("b"), para("c"), para("d")]);
    assert_eq!(e.revision(), 0);
    let stamps = |e: &EditorState| -> Vec<(BlockId, u64)> {
        e.document().blocks().iter().map(|b| (b.id(), b.revision())).collect()
    };
    let s0 = stamps(&e);
    e.set_caret(p(2, 1));
    typ(&mut e, "x");
    assert_eq!(e.revision(), 1);
    let c = e.take_changes().unwrap();
    assert_eq!(c.blocks, Some(BlockSplice { start: 2, old_len: 1, new_len: 1 }));
    assert!(e.take_changes().is_none());
    let s1 = stamps(&e);
    assert_eq!(s0[0], s1[0]);
    assert_eq!(s0[3], s1[3]);
    assert_eq!(s0[2].0, s1[2].0);
    assert_ne!(s0[2].1, s1[2].1);
    e.insert_paragraph_break();
    e.set_caret(p(0, 0));
    typ(&mut e, "y");
    let c = e.take_changes().unwrap();
    assert_eq!(c.revision, 3);
    // Split of 2 into 2..4 then edit of 0: union 0..3 old -> 0..4 new.
    assert_eq!(c.blocks, Some(BlockSplice { start: 0, old_len: 3, new_len: 4 }));
    e.undo();
    let c = e.last_change().unwrap();
    assert_eq!(c.blocks, Some(BlockSplice { start: 0, old_len: 1, new_len: 1 }));
    assert_eq!(e.revision(), 4);
    // Undo restores the exact stamp of the restored version.
    e.undo();
    e.undo();
    assert_eq!(stamps(&e), s0);
    // Selection-only changes do not bump the revision.
    let r = e.revision();
    e.select_all();
    assert_eq!(e.revision(), r);
    assert!(e.set_page_view(true));
    assert!(e.take_changes().unwrap().settings);
    assert!(!e.set_page_view(true));
}

#[test]
fn splice_composition() {
    let a = BlockSplice { start: 5, old_len: 1, new_len: 3 };
    let b = BlockSplice { start: 2, old_len: 2, new_len: 0 };
    let c = BlockSplice::then(a, b);
    assert_eq!(c, BlockSplice { start: 2, old_len: 4, new_len: 4 });
    let d = BlockSplice::then(a, BlockSplice { start: 20, old_len: 1, new_len: 1 });
    assert_eq!(d, BlockSplice { start: 5, old_len: 14, new_len: 16 });
}

// ----- search and replace ------------------------------------------------------------------

#[test]
fn replace_one_and_all() {
    let mut e = ed(vec![para("cat Cat cat"), para("concat cat")]);
    let opts = SearchOptions {
        whole_word: true,
        ..Default::default()
    };
    let m = e.document().find_all("cat", opts);
    assert_eq!(m.len(), 4);
    assert!(e.replace_range(m[1], "dog"));
    assert_eq!(texts(&e), ["cat dog cat", "concat cat"]);
    assert_eq!(caret(&e), p(0, 7));
    e.set_caret(p(1, 9));
    assert_eq!(e.replace_all("cat", opts, "lion"), 3);
    assert_eq!(texts(&e), ["lion dog lion", "concat lion"]);
    assert_eq!(caret(&e), p(1, 7), "caret inside a match moves to its start");
    e.undo();
    assert_eq!(texts(&e), ["cat dog cat", "concat cat"]);
    assert_eq!(e.replace_all("zzz", opts, "x"), 0);
    // Replacement keeps the attributes of the replaced text.
    let mut e = ed(vec![para("a ").with_span("bold", bold())]);
    e.replace_all("bold", SearchOptions::default(), "strong");
    assert_eq!(styled(&e, 0, |a| a.bold), "strong");
    check(&e);
}

// ----- stats --------------------------------------------------------------------------------

#[test]
fn editor_stats_cache() {
    let mut e = ed(vec![para("one two"), para("three")]);
    assert_eq!(e.stats(), e.document().stats());
    assert_eq!(e.stats().words, 3);
    e.set_caret(p(1, 5));
    typ(&mut e, " four");
    assert_eq!(e.stats().words, 4);
    assert_eq!(e.stats(), e.document().stats());
    e.set_selection(Selection::new(p(0, 4), p(1, 5)));
    let s = e.selection_stats();
    assert_eq!(s.words, 2);
    assert_eq!(s.paragraphs, 2);
}

// ----- boundary cases ------------------------------------------------------------------------

#[test]
fn single_empty_block_never_panics() {
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
    assert!(!e.toggle_bold());
    assert!(!e.indent() || e.document().blocks()[0].indent() == 1);
    assert!(e.delete_blocks());
    assert!(!e.paste(&Fragment::default()));
    assert!(!e.insert_text(""));
    assert!(e.copy().is_empty());
    for m in [
        Movement::Left,
        Movement::Right,
        Movement::WordLeft,
        Movement::WordRight,
        Movement::LineStart,
        Movement::LineEnd,
        Movement::BlockStart,
        Movement::BlockEnd,
        Movement::DocStart,
        Movement::DocEnd,
    ] {
        e.move_caret(m, true);
        e.move_caret(m, false);
    }
    assert_eq!(e.format_state().kind, Choice::Same(BlockKind::Paragraph));
    check(&e);
}

#[test]
fn utf16_round_trip_on_blocks() {
    let t = format!("a{FAMILY}b中ש{E_ACUTE}");
    let b = Block::paragraph(&t);
    for (i, _) in t.char_indices().chain(std::iter::once((t.len(), ' '))) {
        let u = b.offset_to_utf16(i);
        assert_eq!(b.utf16_to_offset(u), i);
    }
}

#[test]
fn void_blocks_at_document_edges() {
    let mut e = ed(vec![Block::new(BlockKind::Divider), para("mid"), Block::new(BlockKind::PageBreak)]);
    e.move_caret(Movement::Left, false);
    assert_eq!(caret(&e), p(0, 0));
    e.move_caret(Movement::DocEnd, false);
    assert_eq!(caret(&e), p(2, 0));
    e.move_caret(Movement::Right, false);
    assert_eq!(caret(&e), p(2, 0));
    e.select_all();
    let f = e.copy();
    assert_eq!(f.blocks().len(), 3);
    e.paste(&f);
    check(&e);
    e.select_all();
    e.toggle_bold();
    e.set_block_kind(BlockKind::Quote);
    e.indent();
    e.add_comment("c", "a", 0);
    check(&e);
    undo_all(&mut e);
    assert_eq!(texts(&e), ["", "mid", ""]);
}

#[test]
fn large_document_edits_are_local() {
    // 10,000 blocks, about a megabyte of text.
    let line = "The quick brown fox jumps over the lazy dog again and again, ".repeat(2);
    let blocks: Vec<Block> = (0..10_000).map(|_| para(&line)).collect();
    let mut e = ed(blocks);
    let t0 = std::time::Instant::now();
    e.set_caret(p(5_000, 10));
    for i in 0..500 {
        e.insert_text(if i % 6 == 5 { " " } else { "x" });
    }
    for _ in 0..100 {
        e.backspace();
    }
    e.insert_paragraph_break();
    for _ in 0..50 {
        e.undo();
    }
    let elapsed = t0.elapsed();
    let c = e.take_changes().unwrap();
    let s = c.blocks.unwrap();
    assert!(s.start >= 5_000 && s.old_len <= 2 && s.new_len <= 2, "{s:?}");
    // Generous bound that still catches whole-document work per keystroke.
    assert!(elapsed.as_secs_f64() < 5.0, "took {elapsed:?}");
    let t1 = std::time::Instant::now();
    let stats = e.stats();
    assert!(stats.words > 100_000);
    let _ = e.stats();
    assert!(t1.elapsed().as_secs_f64() < 10.0);
}

#[test]
fn extreme_inputs_do_not_panic() {
    let mut e = ed(vec![para("ab"), Block::new(BlockKind::Divider), para("中")]);
    let far = Position::new(usize::MAX, usize::MAX);
    e.set_selection(Selection::new(far, Position::ZERO));
    e.set_caret(far);
    e.select_block(usize::MAX);
    e.select_word_at(far);
    assert!(e.step_text_size(i32::MIN));
    e.step_text_size(i32::MAX);
    e.set_text_size(Some(f32::NAN));
    e.set_text_size(Some(f32::INFINITY));
    e.set_text_size(Some(-5.0));
    e.set_undo_limit(0);
    e.insert_blocks_at(usize::MAX, &Fragment::from_plain_text("z"));
    e.delete_range(TextRange::new(far, Position::new(1, 5)));
    e.replace_range(TextRange::new(far, far), "tail");
    let d = e.document();
    assert!(d.numbered_ordinals(usize::MAX..0).is_empty());
    let _ = d.text_in(TextRange { start: far, end: Position::ZERO });
    let _ = d.stats_in(TextRange { start: far, end: Position::ZERO });
    let _ = d.fragment(TextRange { start: far, end: Position::ZERO });
    let _ = d.position_from_flat(usize::MAX);
    let _ = d.flat_offset(far);
    let _ = d.link_at(far);
    let _ = d.comments_at(far);
    let _ = d.next_comment(far, true);
    let _ = d.moved(far, Movement::WordRight);
    let _ = d.find_in_block(usize::MAX, "a", SearchOptions::default());
    let b = &d.blocks()[0];
    let _ = b.utf16_to_offset(usize::MAX);
    let _ = b.offset_to_utf16(usize::MAX);
    let _ = b.attrs_before(usize::MAX);
    let _ = b.line_range(usize::MAX);
    let _ = crate::text::utf16_range_to_utf8("abc", 9..2);
    e.set_settings(crate::DocumentSettings {
        typography: crate::Typography {
            base_size: f32::NAN,
            line_height: -1.0,
            body_font: "   ".into(),
            paragraph_spacing: 1e9,
        },
        ..Default::default()
    });
    assert_eq!(e.document().settings().typography.body_font, "Charter");
    check(&e);
    assert_eq!(e.undo_depth(), 1);
}
