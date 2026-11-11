//! Adversarial tests written against the public API only.

use writer_core::*;

const FAMILY: &str = "\u{1F469}\u{200D}\u{1F469}\u{200D}\u{1F467}";

fn ed(blocks: Vec<Block>) -> EditorState {
    EditorState::new(Document::from_blocks(blocks))
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
fn caret(e: &EditorState) -> Position {
    assert!(e.selection().is_collapsed(), "{:?}", e.selection());
    e.selection().head
}

#[test]
fn probe_stale_pending_comment_anchor() {
    let mut e = ed(vec![Block::paragraph("hello world")]);
    e.select_range(TextRange::in_block(0, 0, 11));
    let id = e.add_comment("c", "a", 1).unwrap();
    e.set_caret(p(0, 5));
    e.toggle_bold();
    assert!(e.pending_attrs().unwrap().comments.contains(&id));
    e.delete_comment(id);
    e.insert_text("x");
    e.check_invariants().unwrap();
}

#[test]
fn probe_code_exit_with_indent() {
    let mut e = ed(vec![Block::with_text(BlockKind::code(), "  foo")]);
    e.set_caret(p(0, 5));
    e.insert_paragraph_break();
    e.insert_paragraph_break();
    e.insert_paragraph_break();
    assert_eq!(e.document().block_count(), 2, "{:?}", texts(&e));
}

#[test]
fn probe_ime_pending() {
    let mut e = ed(vec![Block::paragraph("ab")]);
    e.set_caret(p(0, 2));
    e.toggle_bold();
    e.begin_undo_group();
    e.replace_range(TextRange::empty(p(0, 2)), "k");
    e.end_undo_group();
    assert!(e.document().blocks()[0].attrs_at(2).unwrap().bold);
}

#[test]
fn probe_duplicate_selection_end_at_zero() {
    let mut e = ed(vec![Block::paragraph("a"), Block::paragraph("b"), Block::paragraph("c")]);
    e.set_selection(Selection::new(p(0, 0), p(1, 0)));
    e.duplicate_blocks();
    assert_eq!(texts(&e), ["a", "a", "b", "c"]);
    assert_eq!(e.selection(), Selection::new(p(1, 0), p(2, 0)));
}

#[test]
fn probe_yank_void_line() {
    let d = Document::from_blocks(vec![Block::paragraph("a"), Block::new(BlockKind::Divider)]);
    let f = d.fragment(TextRange::new(p(1, 0), p(1, 0)));
    assert!(f.blocks().is_empty());
}

#[test]
fn probe_comment_range_across_empty_block() {
    let mut e = ed(vec![Block::paragraph("aa"), Block::paragraph(""), Block::paragraph("bb")]);
    e.select_all();
    let id = e.add_comment("c", "a", 1).unwrap();
    assert_eq!(e.document().comment_ranges(id).len(), 1, "{:?}", e.document().comment_ranges(id));
}

#[test]
fn probe_replace_keeps_per_char_attrs() {
    let b = Block::paragraph("a").with_span("b", InlineAttrs { bold: true, ..Default::default() }).with_span("c", InlineAttrs::default());
    let mut e = ed(vec![b]);
    e.replace_range(TextRange::in_block(0, 0, 3), "ABC");
    assert!(e.document().blocks()[0].attrs_at(1).unwrap().bold);
}

#[test]
fn probe_undo_selection_exact() {
    let mut e = ed(vec![Block::paragraph("hello world")]);
    e.set_selection(Selection::new(p(0, 8), p(0, 2)));
    e.toggle_bold();
    e.set_caret(p(0, 0));
    e.undo();
    assert_eq!(e.selection(), Selection::new(p(0, 8), p(0, 2)));
    e.redo();
    assert_eq!(e.selection(), Selection::new(p(0, 8), p(0, 2)));
}

#[test]
fn probe_smart_quotes() {
    let mut e = ed(vec![Block::paragraph("")]);
    typ(&mut e, "\"don't\" ('x') [\"y\"]");
    assert_eq!(texts(&e)[0], "\u{201C}don\u{2019}t\u{201D} (\u{2018}x\u{2019}) [\u{201C}y\u{201D}]");
}

#[test]
fn probe_paste_void_keeps_id() {
    let mut e = ed(vec![Block::paragraph("")]);
    let id = e.document().blocks()[0].id();
    let f = Fragment::new(vec![Block::new(BlockKind::Divider)]);
    e.paste(&f);
    e.check_invariants().unwrap();
    assert!(e.document().blocks().iter().any(|b| b.id() == id), "{:?}", kinds(&e));
}

#[test]
fn probe_utf16_flat() {
    let e = ed(vec![Block::paragraph(FAMILY), Block::paragraph("x")]);
    let _ = caret(&e);
}
