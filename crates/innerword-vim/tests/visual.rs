//! Visual and Visual Line mode.

use crate::harness::*;
use innerword_core::{Position, Selection};
use innerword_vim::{Mode, VimOptions};

#[test]
fn selecting() {
    run(&[
        c("|hello", "vl", "«h|ello").visual(),
        c("|hello", "v", "«|hello").visual(),
        c("he|llo", "vh", "h|e«llo").visual(),
        c("|hello world", "ve", "«hell|o world").visual(),
        c("|hello world", "vw", "«hello |world").visual(),
        c("a|bc\ndef", "vj", "a«bc\nd|ef").visual(),
        // $ goes past the last character and takes the line break.
        c("a|bc\ndef", "v$", "a«bc|\ndef").visual(),
        c("a|bc\ndef", "v$h", "a«b|c\ndef").visual(),
        c("|hello", "vllo", "|he«llo").visual(),
        c("|hello", "vlloo", "«he|llo").visual(),
        c("a|b\ncd", "vV", "«a|b\ncd").vline(),
        c("a|b\ncd", "Vv", "a«|b\ncd").visual(),
        c("|abcdef", "3v", "«ab|cdef").visual(),
        c("|a\nb\nc\nd", "2V", "«a\n|b\nc\nd").vline(),
        // Leaving.
        c("a|bc", "vl<Esc>", "ab|c"),
        c("a|bc", "vlv", "ab|c"),
        c("a|bc", "VV", "a|bc"),
        c("a|bc", "v<C-c>", "a|bc"),
        c("a|bc\ndef", "v$<Esc>", "ab|c\ndef"),
    ]);
}

#[test]
fn the_editor_selection_follows() {
    let p = Position::new;
    let s = session("a|bcd", "vl");
    assert_eq!(s.selection(), Selection::new(p(0, 1), p(0, 3)));
    let s = session("ab|cd", "vh");
    assert_eq!(s.selection(), Selection::new(p(0, 3), p(0, 1)));
    let s = session("a|b\ncd\nef", "Vj");
    assert_eq!(s.selection(), Selection::new(p(0, 0), p(1, 2)));
    assert_eq!(s.vim.visual_line_blocks(&s.ed), Some(0..2));
    let s = session("ab\nc|d", "Vk");
    assert_eq!(s.selection(), Selection::new(p(1, 2), p(0, 0)));
    assert_eq!(s.vim.caret(&s.ed), p(0, 1));
    // The selection on an empty line or a divider reaches the next block.
    let s = session("ab\n|\ncd", "v");
    assert_eq!(s.selection(), Selection::new(p(1, 0), p(2, 0)));
    let s = session("ab\n|---\ncd", "v");
    assert_eq!(s.selection(), Selection::new(p(1, 0), p(2, 0)));
    let s = session("|ab", "v");
    assert_eq!(s.vim.visual_line_blocks(&s.ed), None);
}

#[test]
fn operators() {
    run(&[
        c("|hello world", "vey", "|hello world").regs(&[('"', "hello"), ('0', "hello")]),
        c("|hello world", "ved", "| world").regs(&[('"', "hello")]),
        c("|hello world", "vex", "| world"),
        c("hello |world", "vbd", "|orld"),
        c("|hello world", "vecX<Esc>", "|X world"),
        c("|hello world", "vesX<Esc>", "|X world"),
        c("ab|c", "vd", "a|b"),
        c("a|bc\ndef", "v$d", "a|def"),
        c("a|bc\ndef", "v$y", "a|bc\ndef").regs(&[('"', "bc\n")]),
        c("a|bc\ndef", "vjd", "a|f"),
        c("a|b\ncd", "vjhd", "a|d"),
        c("|abcdef", "v3ld", "|ef"),
        c("|a b c d", "v2wd", "| d"),
        c("a|bcd", "vl<Del>", "a|d"),
        c("|hello", "vllU", "|HELlo"),
        c("|HELLO", "vllu", "|helLO"),
        c("|hello", "vl~", "|HEllo"),
        c("|hello", "vlgU", "|HEllo"),
        c("|hello", "vllrx", "|xxxlo"),
        c("a|b\ncd", "vjrx", "a|x\nxx"),
        c("|hello world", "ve>", "{1}|hello world"),
        c("|a\nb", "vJ", "a| b"),
        // The upper-case forms take whole lines.
        c("a|b\ncd", "vX", "|cd").regs(&[('"', "ab\n")]),
        c("a|b\ncd", "vD", "|cd"),
        c("a|b\ncd", "vY", "|ab\ncd").regs(&[('"', "ab\n")]),
        c("a|b\ncd", "vCX<Esc>", "|X\ncd"),
        c("a|b\ncd", "vSX<Esc>", "|X\ncd"),
        c("a|b\ncd", "vRX<Esc>", "|X\ncd"),
    ]);
}

#[test]
fn line_operators() {
    run(&[
        c("a\n|b\nc", "Vd", "a\n|c").regs(&[('"', "b\n")]),
        c("|a\nb\nc", "Vjd", "|c"),
        c("a\nb\n|c", "Vkd", "|a"),
        c("a\n|b\nc", "Vy", "a\n|b\nc").regs(&[('"', "b\n")]),
        c("a\nb|c", "Vky", "|a\nbc").regs(&[('"', "a\nbc\n")]),
        c("a\n|b\nc", "VcX<Esc>", "a\n|X\nc"),
        c("|a\nb", "Vj>", "{1}|a\n{1}b"),
        c("|a", "V2>", "{2}|a"),
        c("{2}|a\n{2}b", "Vj<", "{1}|a\n{1}b"),
        c("|a\nb\nc", "VjjJ", "a b| c"),
        c("|a\nb\nc", "VjgJ", "a|b\nc"),
        c("|ab\ncd", "VjU", "|AB\nCD"),
        c("|ab\ncd", "Vrx", "|xx\ncd"),
        c("a\n|\nb", "Vd", "a\n|b"),
        c("a\n|---\nb", "Vd", "a\n|b"),
        c("a\n|\n\nb", "Vjd", "a\n|b"),
        c("|a\nb\nc\nd", "VGd", "|"),
        c("a\nb\nc\n|d", "Vggy", "|a\nb\nc\nd").message("4 lines yanked"),
        c("# |H\nx", "Vd", "|x").regs(&[('"', "# H\n")]),
    ]);
}

#[test]
fn reselecting() {
    run(&[
        c("|hello world", "ve<Esc>0gvd", "| world"),
        c("|ab cd", "vly$gvd", "| cd"),
        c("|a\nb\nc", "Vj<Esc>Ggvd", "|c"),
        c("|x", "gv", "|x").bell(true),
        c("|ab cd", "vl<Esc>wgv", "«a|b cd").visual(),
        // In Visual mode gv swaps with the selection before.
        c("|ab cd", "vl<Esc>wvlgv", "«a|b cd").visual(),
        c("|ab cd ef", "vl<Esc>wvlgvgv", "ab «c|d ef").visual(),
        // The marks of the last selection.
        c("a|bc\ndef", "vj<Esc>gg`>", "abc\nd|ef"),
        c("a|bc\ndef", "vj<Esc>G`<", "a|bc\ndef"),
        c("a\n|b\nc", "V<Esc>gg'>", "a\n|b\nc"),
    ]);
}

#[test]
fn text_objects() {
    run(&[
        c("foo b|ar baz", "viwd", "foo | baz"),
        c("foo b|ar baz", "vawd", "foo |baz"),
        c("foo b|ar baz", "viw", "foo «ba|r baz").visual(),
        c("foo b|ar baz", "viwiw", "foo «bar| baz").visual(),
        c("f(a, |b)", "vi(d", "f(|)"),
        c("f(a, |b)", "va(d", "|f"),
        c("((a|b))", "vi(i(d", "(|)"),
        c("((a|b))", "va(a(d", "|"),
        c("a\n|b\n\nc", "vipd", "|\nc"),
        c("a\n|b\n\nc", "vip", "«a\n|b\n\nc").vline(),
        c("a\n|b\n\nc", "vapd", "|c"),
        c("say \"h|i\" x", "vi\"d", "say \"|\" x"),
        c("say \"h|i\" x", "va\"d", "say |x"),
        c("One. T|wo. Three.", "visd", "One. | Three."),
        c("f(a, |b)", "vi[", "f(a, «|b)").visual().bell(true),
        c("a f.o|o b", "viWy", "a |f.oo b").regs(&[('"', "f.oo")]),
    ]);
}

#[test]
fn put_over_selection() {
    run(&[
        c("|foo bar", "yiwwviwp", "foo fo|o").regs(&[('"', "bar"), ('0', "foo")]),
        // P leaves the registers alone.
        c("|foo bar", "yiwwviwP", "foo fo|o").regs(&[('"', "foo")]),
        c("|foo bar", "yiwwviw2p", "foo foofo|o"),
        // Lines over characters: they go on lines of their own.
        // Lines over characters: the line is split where the selection was
        // and both halves stay, also when one is empty (as in Vim).
        c("|ab\ncd", "yyjvp", "ab\n\n|ab\nd"),
        c("|ab\ncxd", "yyjlvp", "ab\nc\n|ab\nd"),
        c("|ab\ncd", "yyj$vp", "ab\nc\n|ab\n"),
        c("|ab\nc", "yyjvp", "ab\n\n|ab\n"),
        // Characters over lines: they become a line.
        c("|x\nfoo bar", "yljVp", "x\n|x"),
        c("|a\nb\nc", "yyjVp", "a\n|a\nc").regs(&[('"', "b\n")]),
        c("|a\nb\nc", "yyjVjp", "a\n|a"),
        c("# |H\nb\nc", "yyjVp", "# H\n# |H\nc"),
        // An empty register puts nothing: the selection is deleted.
        c("|x", "vp", "|"),
        c("|ab", "yl$v\"_p", "|a"),
    ]);
}

#[test]
fn selections_made_by_the_host() {
    run(&[
        // Shift-arrows are the editor's; the engine picks the selection up.
        c("a|bcd", "<S-Right><S-Right>", "a«b|cd").visual(),
        c("a|bcd", "<S-Right><S-Right>d", "a|d"),
        c("abc|d", "<S-Left><S-Left>", "a|b«cd").visual(),
        c("abc|d", "<S-Left><S-Left>y", "a|bcd").regs(&[('"', "bc")]),
        c("a|bcd", "<S-Right><S-Left>", "a|bcd"),
    ]);
    // A mouse click in Visual mode ends it.
    let mut s = session("|abcd", "vl");
    assert_eq!(s.mode(), Mode::Visual);
    s.ed.set_caret(Position::new(0, 4));
    let o = s.vim.sync(&mut s.ed, &mut s.host);
    assert!(o.consumed);
    assert_eq!(s.mode(), Mode::Normal);
    assert_eq!(s.buffer(), "abc|d");
    // A drag in Normal mode starts it.
    let mut s = Session::with("|abcd", 1000, 10, VimOptions::default());
    s.ed.set_selection(Selection::new(Position::new(0, 1), Position::new(0, 3)));
    s.vim.sync(&mut s.ed, &mut s.host);
    assert_eq!(s.mode(), Mode::Visual);
    assert_eq!(s.buffer(), "a«b|cd");
    s.keys("d");
    assert_eq!(s.buffer(), "a|d");
}

#[test]
fn registers_counts_and_the_command_line() {
    run(&[
        c("|foo", "ve\"ay", "|foo").regs(&[('a', "foo"), ('0', "<unset>")]),
        c("|foo bar", "ve\"ad\"aP", "fo|o bar"),
        c("|foo", "ve\"_d", "|").regs(&[('"', "<unset>")]),
        c("|foo", "ve\".y", "|foo").bell(true),
        // The command line takes the selection as its range.
        c("|a\nb\nc", "Vj:<Esc>", "a\n|b\nc"),
        c("|a\nb\nc", "vj:d<CR>", "|c"),
        c("|a\nb\nc", "Vj:><CR>", "{1}|a\n{1}b\nc"),
        c("|a\nb\nc", "Vj:y<CR>", "a\n|b\nc").regs(&[('"', "a\nb\n")]),
        c("|a\nb\nc", "Vj<Esc>:'<,'>d<CR>", "|c"),
        // Marks and searches move the cursor end.
        c("a|bc\ndef", "mxjv`xd", "a|f"),
        c("|foo bar baz", "v/baz<CR>y", "|foo bar baz").regs(&[('"', "foo bar b")]),
        c("|foo bar foo", "v*d", "|oo"),
        c("|a b c", "v3ed", "|"),
        c("|abc def", "vfdd", "|ef"),
        c("|(abc) x", "v%d", "| x"),
    ]);
}

#[test]
fn an_empty_last_line_of_a_linewise_selection_takes_its_line_break() {
    let p = Position::new;
    // So the host has something to show as selected on that line.
    let s = session("ab\n|\ncd", "V");
    assert_eq!(s.selection(), Selection::new(p(1, 0), p(2, 0)));
    assert_eq!(s.vim.visual_line_blocks(&s.ed), Some(1..2));
    assert_eq!(s.vim.caret(&s.ed), p(1, 0));
    let s = session("a|b\n\ncd", "Vj");
    assert_eq!(s.selection(), Selection::new(p(0, 0), p(2, 0)));
    assert_eq!(s.vim.caret(&s.ed), p(1, 0));
    // Backwards from the empty line: the anchor end takes the break.
    let s = session("ab\n|\ncd", "Vk");
    assert_eq!(s.selection(), Selection::new(p(2, 0), p(0, 0)));
    assert_eq!(s.vim.visual_line_blocks(&s.ed), Some(0..2));
    // Not where the break would select something else: before a divider,
    // and at the end of the document there is none.
    let s = session("ab\n|\n---\ncd", "V");
    assert_eq!(s.selection(), Selection::caret(p(1, 0)));
    let s = session("ab\n|", "V");
    assert_eq!(s.selection(), Selection::caret(p(1, 0)));
    // A line with text is selected as before.
    let s = session("a|b\ncd", "V");
    assert_eq!(s.selection(), Selection::new(p(0, 0), p(0, 2)));

    // The commands see the lines, not the extra break.
    run(&[
        c("ab\n|\ncd", "Vd", "ab\n|cd"),
        c("ab\n|\ncd", "Vy", "ab\n|\ncd").regs(&[('"', "\n")]),
        c("a|b\n\ncd", "Vjd", "|cd"),
        c("ab\n|\ncd", "Vkd", "|cd"),
        c("ab\n|\ncd", "Vj", "ab\n«\n|cd").vline(),
        c("ab\n|\ncd", "V<Esc>", "ab\n|\ncd"),
        c("ab\n|\ncd", "Vcx<Esc>", "ab\n|x\ncd"),
    ]);

    // The host reporting that same selection back changes nothing.
    let mut s = session("a|b\n\ncd", "Vj");
    let before = s.selection();
    let outcome = s.vim.sync(&mut s.ed, &mut s.host);
    assert!(outcome.effects.is_empty(), "{:?}", outcome.effects);
    assert_eq!(s.selection(), before);
    assert_eq!(s.mode(), Mode::VisualLine);
    assert_eq!(s.vim.visual_line_blocks(&s.ed), Some(0..2));
}

#[test]
fn an_empty_last_line_of_code_takes_its_line_break() {
    let p = Position::new;
    let mut s = session("x", "");
    s.ed.set_block_kind(innerword_core::BlockKind::code());
    s.ed.set_selection(Selection::caret(p(0, 1)));
    s.ed.insert_text("\n\nz");
    s.ed.set_caret(p(0, 2));
    s.vim.sync(&mut s.ed, &mut s.host);
    assert_eq!(s.ed.document().blocks()[0].text(), "x\n\nz");
    s.keys("V");
    assert_eq!(s.selection(), Selection::new(p(0, 2), p(0, 3)));
    s.keys("d");
    assert_eq!(s.ed.document().blocks()[0].text(), "x\nz");
}
