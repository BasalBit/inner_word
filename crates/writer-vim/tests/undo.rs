//! Undo, redo and where they leave the caret.

use crate::harness::*;
use writer_core::Position;

#[test]
fn undo_and_redo() {
    run(&[
        c("|abc", "xu", "|abc"),
        c("|abc", "xxu", "|bc"),
        c("|abc", "xxuu", "|abc"),
        c("|abc", "xu<C-r>", "|bc"),
        c("|abcd", "xxx2u", "|bcd"),
        c("|abcd", "xxx3u2<C-r>", "|cd"),
        c("|abcd", "xxx9u", "|abcd"),
        c("|a", "u", "|a").message("Already at oldest change").bell(true),
        c("|a", "<C-r>", "|a").message("Already at newest change").bell(true),
        // A new change ends redo.
        c("|abc", "xuix<Esc><C-r>", "|xabc").message("Already at newest change"),
        // Yanking and moving are not changes.
        c("|abc", "xyylu", "|abc"),
        c("|abc", "xu.", "|bc"),
    ]);
}

#[test]
fn one_command_is_one_step() {
    run(&[
        c("|abcdef", "3xu", "|abcdef"),
        c("|a b c d", "d3wu", "|a b c d"),
        c("foo |bar", "ciwxyz<Esc>u", "foo |bar"),
        c("|x", "iab<CR>cd<BS><Esc>u", "|x"),
        c("|x", "3iab<Esc>u", "|x"),
        c("|x", "3oy<Esc>u", "|x"),
        c("|a\nb\nc\nd", "4Ju", "|a\nb\nc\nd"),
        c("|a b c d", "dw..u", "|c d"),
        c("|a b c d", "dw..uu", "|b c d"),
        c("|a\nb\nc", "3>>u", "|a\nb\nc"),
        c("|ab", "yl3pu", "|ab"),
        c("|a a\na a", ":%s/a/b/g<CR>u", "|a a\na a"),
        c("|abc", "3~u", "|abc"),
        c("|abc", "3rxu", "|abc"),
        c("|a\nb\nc", "Vjdu", "|a\nb\nc"),
        c("|a\nb\nc", "VjJu", "|a\nb\nc"),
        c("|foo bar", "yiwwviwpu", "foo |bar"),
        c("|a\nb", "ccx<CR>y<Esc>u", "|a\nb"),
        c("|abcdef", "RXYZ<Esc>u", "|abcdef"),
        c("|a\nb\nc\nd", ":2,3d<CR>u", "a\n|b\nc\nd"),
        c("|foo", "gUUu", "|foo"),
        c("|straße x", "gUiwu", "|straße x"),
        // Typing after ctrl-o is a step of its own.
        c("|x", "ia<C-o>0b<Esc>u", "|ax"),
    ]);
}

#[test]
fn the_caret_returns_to_the_change() {
    run(&[
        c("a\nb|c\nd", "ddu", "a\nb|c\nd"),
        c("a|bc", "ylpu", "a|bc"),
        c("foo |bar baz", "dwu", "foo |bar baz"),
        c("foo |bar", "dbu", "|foo bar"),
        c("|abc", "AX<Esc>u", "ab|c"),
        c("ab|c", "IX<Esc>u", "|abc"),
        c("|a\nb", "oX<Esc>u", "|a\nb"),
        c("a\n|b", "OX<Esc>u", "a\n|b"),
        c("a|b", ">>u", "a|b"),
        c("a|b\ncd", "Ju", "a|b\ncd"),
        c("a|bcd", "vldu", "a|bcd"),
        c("a\n|b\nc", "Vdu", "a\n|b\nc"),
        c("ab|c", "xu", "ab|c"),
        c("a\nb\n|c", "ggdGu", "|a\nb\nc"),
        c("a b |c", "yiw0Pu", "|a b c"),
        c("|a\nb\nc", "yyjpu", "a\n|b\nc"),
        c("a\nb|c\nd", "ccX<Esc>u", "a\nb|c\nd"),
        c("a\nb|c\nd", "dku", "|a\nbc\nd"),
        c("a|b\ncd\ne", "dju", "a|b\ncd\ne"),
        // Redo comes back to the same place.
        c("|x", "iab<Esc>u<C-r>", "|abx"),
        c("a|bc", "xu<C-r>", "a|c"),
        c("a\n|b\nc", "ddu<C-r>", "a\n|c"),
        c("a\nb|c\nd", "ddu<C-r>u", "a\nb|c\nd"),
    ]);
}

#[test]
fn edits_made_by_the_host_are_steps_too() {
    let mut s = Session::new("|abc");
    s.keys("x");
    // Something outside the engine edits the document.
    s.ed.set_caret(Position::new(0, 2));
    s.ed.insert_text("Z");
    s.vim.sync(&mut s.ed, &mut s.host);
    assert_eq!(s.buffer(), "bc|Z");
    s.keys("u");
    assert_eq!(s.text(), "bc");
    s.keys("u");
    assert_eq!(s.buffer(), "|abc");
    s.keys("<C-r><C-r>");
    assert_eq!(s.text(), "bcZ");
    assert!(s.check_invariants().is_ok());
}

#[test]
fn an_insert_session_is_one_step_even_with_many_edits() {
    let mut s = Session::new("|x");
    let depth = s.ed.undo_depth();
    s.keys("ihello world<CR>second line<BS><BS><C-w>word");
    assert_eq!(s.ed.undo_depth(), depth + 1);
    s.keys("<Esc>");
    assert_eq!(s.ed.undo_depth(), depth + 1);
    s.keys("u");
    assert_eq!(s.buffer(), "|x");
    assert_eq!(s.ed.undo_depth(), depth);
    // An empty session leaves no step behind.
    s.keys("i<Esc>a<Esc>");
    assert_eq!(s.ed.undo_depth(), depth);
}
